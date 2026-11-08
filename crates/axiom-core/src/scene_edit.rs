//! Shared scene graph projection and transactional structural edits.
use crate::{
    local::{escape, fingerprint},
    Diagnostic, Entity, Relation, Transform, World,
};
use anyhow::{ensure, Result};
use glam::{Mat4, Quat};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub struct Hierarchy<'a> {
    pub objects: BTreeMap<&'a str, &'a Entity>,
    pub parents: BTreeMap<&'a str, &'a str>,
    pub children: BTreeMap<Option<&'a str>, Vec<&'a str>>,
    pub memberships: BTreeMap<&'a str, Vec<&'a str>>,
    active_scenes: BTreeSet<&'a str>,
}

impl<'a> Hierarchy<'a> {
    pub fn new(world: &'a World) -> Self {
        let objects: BTreeMap<_, _> = world.entities.iter().map(|e| (e.id.as_str(), e)).collect();
        let parents: BTreeMap<_, _> = world
            .relations
            .iter()
            .filter(|r| r.kind == "parent")
            .map(|r| (r.source.as_str(), r.target.as_str()))
            .collect();
        // Siblings keep the authored entity order (what `move_entity` edits).
        let mut children: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for id in world.entities.iter().map(|e| e.id.as_str()) {
            children
                .entry(parents.get(id).copied())
                .or_default()
                .push(id);
        }
        let mut memberships: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for scene in &world.scenes {
            for id in &scene.entities {
                memberships
                    .entry(id.as_str())
                    .or_default()
                    .push(scene.id.as_str());
            }
        }
        Self {
            objects,
            parents,
            children,
            memberships,
            active_scenes: world
                .scenes
                .iter()
                .filter(|s| s.active)
                .map(|s| s.id.as_str())
                .collect(),
        }
    }
    pub fn active(&self, id: &str) -> bool {
        self.memberships
            .get(id)
            .is_none_or(|scenes| scenes.iter().any(|s| self.active_scenes.contains(s)))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TreeQuery {
    pub parent: Option<String>,
    pub roots_only: bool,
    pub scene: Option<String>,
    pub text: Option<String>,
    pub limit: Option<usize>,
    pub max_bytes: Option<usize>,
    pub cursor: Option<String>,
}

pub fn tree(world: &World, rev: i64, query: &TreeQuery) -> Result<Value> {
    let limit = query.limit.unwrap_or(32);
    let max_bytes = query.max_bytes.unwrap_or(8192);
    ensure!((1..=128).contains(&limit), "limit must be 1..128");
    ensure!(
        (256..=65536).contains(&max_bytes),
        "max_bytes must be 256..65536"
    );
    ensure!(
        !query.roots_only || query.parent.is_none(),
        "choose parent or roots_only"
    );
    ensure!(
        query.text.as_ref().is_none_or(|s| s.len() <= 128),
        "text exceeds 128 bytes"
    );
    let graph = Hierarchy::new(world);
    if let Some(parent) = &query.parent {
        require_object(&graph, parent)?;
    }
    if let Some(scene) = &query.scene {
        ensure!(
            world.scenes.iter().any(|s| &s.id == scene),
            "scene not found {scene}"
        );
    }
    let signature = fingerprint(&(&query.parent, query.roots_only, &query.scene, &query.text))?;
    let prefix = format!("s1.{rev}.{signature}.");
    let offset = match &query.cursor {
        None => 0,
        Some(cursor) => cursor
            .strip_prefix(&prefix)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                Diagnostic::error(
                    "stale_cursor",
                    "/query/cursor",
                    "revision or scene tree filter changed",
                )
            })?,
    };
    let term = query.text.as_deref().unwrap_or_default().to_lowercase();
    let rows: Vec<_> = graph
        .objects
        .iter()
        .filter(|&(&id, e)| {
            let parent = graph.parents.get(id).copied();
            (!query.roots_only || parent.is_none())
                && query.parent.as_deref().is_none_or(|p| parent == Some(p))
                && query
                    .scene
                    .as_deref()
                    .is_none_or(|s| graph.memberships.get(id).is_some_and(|v| v.contains(&s)))
                && (id.to_lowercase().contains(&term)
                    || e.label
                        .as_ref()
                        .is_some_and(|l| l.to_lowercase().contains(&term)))
        })
        .collect();
    ensure!(offset <= rows.len(), "scene tree cursor out of range");
    let mut page = Vec::new();
    let response = |page: &[Value], end: usize| {
        let next = (end < rows.len()).then(|| format!("{prefix}{end}"));
        json!({"ok":true,"rev":rev,"total":rows.len(),"objects":page,"next_cursor":next})
    };
    let mut result = response(&page, offset);
    for &(&id, entity) in rows.iter().skip(offset).take(limit) {
        let mut row = json!({"id":id,"parent":graph.parents.get(id),"active":graph.active(id),
            "children":graph.children.get(&Some(id)).map_or(0, Vec::len)});
        if let Some(label) = &entity.label {
            row["label"] = json!(label);
        }
        if let Some(prefab) = &entity.prefab {
            row["prefab"] = json!(prefab);
        }
        page.push(row);
        let candidate = response(&page, offset + page.len());
        if serde_json::to_vec(&candidate)?.len() > max_bytes {
            ensure!(
                page.len() > 1,
                "scene tree row exceeds max_bytes; increase the budget"
            );
            break;
        }
        result = candidate;
    }
    Ok(result)
}

fn require_object<'a>(graph: &Hierarchy<'a>, id: &str) -> Result<&'a Entity> {
    graph.objects.get(id).copied().ok_or_else(|| {
        Diagnostic::error(
            "not_found",
            format!("/entities/@{}", escape(id)),
            "entity ID not found",
        )
    })
}

pub(crate) fn local_transform(world: &World, entity: &Entity) -> Result<Option<Transform>> {
    let mut value = entity
        .prefab
        .as_ref()
        .and_then(|p| world.prefabs.get(p))
        .and_then(|p| p.components.get("core.transform"))
        .cloned();
    if let Some(overrides) = entity.components.get("core.transform") {
        crate::merge_json(value.get_or_insert(Value::Null), overrides);
    }
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}

pub(crate) fn global(world: &World, graph: &Hierarchy<'_>, id: Option<&str>) -> Result<Mat4> {
    let mut matrix = Mat4::IDENTITY;
    let mut current = id;
    let mut visited = BTreeSet::new();
    while let Some(id) = current {
        ensure!(visited.insert(id), "parent cycle at {id}");
        if let Some(transform) = local_transform(world, require_object(graph, id)?)? {
            transform.validate()?;
            matrix = transform.matrix() * matrix;
        }
        current = graph.parents.get(id).copied();
    }
    Ok(matrix)
}

pub(crate) fn reparent(
    world: &mut World,
    id: &str,
    parent: Option<&str>,
    keep_world: bool,
) -> Result<()> {
    let graph = Hierarchy::new(world);
    let entity = require_object(&graph, id)?;
    if graph.parents.get(id).copied() == parent {
        return Ok(());
    }
    let path = format!("/relations/parent/@{}", escape(id));
    let mut current = parent;
    let mut visited = BTreeSet::new();
    while let Some(ancestor) = current {
        require_object(&graph, ancestor)?;
        if ancestor == id || !visited.insert(ancestor) {
            return Err(Diagnostic::error(
                "parent_cycle",
                &path,
                "parent cannot be the object or its descendant",
            ));
        }
        current = graph.parents.get(ancestor).copied();
    }
    if keep_world {
        let invalid_pose = || {
            Diagnostic::error(
                "unrepresentable_transform",
                &path,
                "keeping world pose requires a valid local TRS; try keeping local pose",
            )
        };
        let old = global(world, &graph, Some(id))?;
        let new_parent = global(world, &graph, parent)?;
        let det = new_parent.determinant();
        if !det.is_finite() || det <= 0.0 {
            return Err(invalid_pose());
        }
        let matrix = new_parent.inverse() * old;
        let det = matrix.determinant();
        if !matrix.is_finite() || !det.is_finite() || det <= 0.0 {
            return Err(invalid_pose());
        }
        let (scale, rotation, position) = matrix.to_scale_rotation_translation();
        let previous = local_transform(world, entity)?.unwrap_or_default();
        let mut rotation = rotation.normalize();
        let old_rotation = Quat::from_array(previous.rotation);
        if rotation.dot(old_rotation) < 0.0 {
            rotation = -rotation;
        }
        let mut transform = Transform {
            scale: scale.to_array(),
            rotation: rotation.to_array(),
            position: position.to_array(),
        };
        // Retain prefab inheritance and authored values when decomposition only adds roundoff.
        if rotation.abs_diff_eq(old_rotation, 1e-6) {
            transform.rotation = previous.rotation;
        }
        if transform
            .scale
            .iter()
            .zip(previous.scale)
            .all(|(a, b)| (*a - b).abs() <= 1e-6 * b)
        {
            transform.scale = previous.scale;
        }
        if transform.validate().is_err() {
            return Err(invalid_pose());
        }
        if !transform
            .matrix()
            .to_cols_array()
            .iter()
            .zip(matrix.to_cols_array())
            .all(|(a, b)| (*a - b).abs() <= 0.0001 * (1.0 + b.abs()))
        {
            return Err(invalid_pose());
        }
        let fields = [
            (
                "position",
                transform.position != previous.position,
                json!(transform.position),
            ),
            (
                "rotation",
                transform.rotation != previous.rotation,
                json!(transform.rotation),
            ),
            (
                "scale",
                transform.scale != previous.scale,
                json!(transform.scale),
            ),
        ];
        let patch: serde_json::Map<_, _> = fields
            .into_iter()
            .filter(|(_, changed, _)| *changed)
            .map(|(key, _, value)| (key.into(), value))
            .collect();
        if !patch.is_empty() {
            let own = world
                .entity_mut(id)
                .unwrap()
                .components
                .entry("core.transform".into())
                .or_insert(Value::Null);
            crate::merge_json(own, &Value::Object(patch));
        }
    }
    world
        .relations
        .retain(|r| r.kind != "parent" || r.source != id);
    if let Some(parent) = parent {
        world.relations.push(Relation {
            source: id.into(),
            kind: "parent".into(),
            target: parent.into(),
        });
    }
    Ok(())
}

pub(crate) fn duplicate(world: &mut World, source: &str, id: String) -> Result<()> {
    crate::valid_id(&id)?;
    let graph = Hierarchy::new(world);
    ensure!(
        !graph.objects.contains_key(id.as_str()),
        "entity ID already exists {id}"
    );
    let mut entity = require_object(&graph, source)?.clone();
    let parent = graph.parents.get(source).map(|p| p.to_string());
    entity.id.clone_from(&id);
    world.entities.push(entity);
    if let Some(parent) = parent {
        world.relations.push(Relation {
            source: id.clone(),
            kind: "parent".into(),
            target: parent,
        });
    }
    for scene in &mut world.scenes {
        if scene.entities.iter().any(|e| e == source) {
            scene.entities.push(id.clone());
        }
    }
    Ok(())
}

pub(crate) fn move_entity(world: &mut World, id: &str, before: Option<&str>) -> Result<()> {
    let path = format!("/entities/@{}", escape(id));
    let from = world
        .entities
        .iter()
        .position(|e| e.id == id)
        .ok_or_else(|| Diagnostic::error("not_found", &path, "entity ID not found"))?;
    if before == Some(id) {
        return Err(Diagnostic::error(
            "invalid_value",
            &path,
            "an entity cannot be moved before itself",
        ));
    }
    if let Some(other) = before.filter(|o| world.entity(o).is_none()) {
        return Err(Diagnostic::error(
            "not_found",
            format!("/entities/@{}", escape(other)),
            "entity ID not found",
        ));
    }
    let entity = world.entities.remove(from);
    let to = before
        .and_then(|other| world.entities.iter().position(|e| e.id == other))
        .unwrap_or(world.entities.len());
    world.entities.insert(to, entity);
    Ok(())
}

pub(crate) fn membership(world: &mut World, scene: &str, entity: &str, member: bool) -> Result<()> {
    ensure!(world.entity(entity).is_some(), "entity not found {entity}");
    let scene = world
        .scenes
        .iter_mut()
        .find(|s| s.id == scene)
        .ok_or_else(|| {
            Diagnostic::error(
                "not_found",
                format!("/scenes/@{}", escape(scene)),
                "scene ID not found",
            )
        })?;
    if member {
        if !scene.entities.iter().any(|e| e == entity) {
            scene.entities.push(entity.into());
        }
    } else {
        scene.entities.retain(|e| e != entity);
    }
    Ok(())
}

pub(crate) fn delete(world: &mut World, id: &str) -> Result<()> {
    ensure!(world.entity(id).is_some(), "entity not found {id}");
    // Keep child objects and their local transforms. The batch validator checks typed references.
    world.entities.retain(|e| e.id != id);
    world.relations.retain(|r| r.source != id && r.target != id);
    for scene in &mut world.scenes {
        scene.entities.retain(|e| e != id);
    }
    Ok(())
}
