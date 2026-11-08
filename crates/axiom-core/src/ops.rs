//! Editor operations shared by the editor UI and headless entry points (ACL, CLI).
//!
//! Every content-changing editor feature compiles here, once, to an ordinary edit batch:
//! `fn(&World, args) -> Result<Vec<Edit>>` (plus the new IDs where objects are created).
//! The functions are pure: they read the world, never mutate it, and the caller commits
//! the batch through the usual transactional protocol (CAS, receipts, diff, undo).
use crate::local::{escape, Target};
use crate::*;
use anyhow::Result;
use glam::{EulerRot, Quat};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Presets offered by the editor's "+ Object" menu. ACL: `create light point` is
/// shorthand for `create light.point`.
pub const PRESETS: [&str; 7] = [
    "empty",
    "cube",
    "camera",
    "light.directional",
    "light.point",
    "light.spot",
    "light.area",
];

/// An edit batch that creates objects, and the created IDs (first = the one to select).
#[derive(Clone, Debug)]
pub struct Created {
    pub edits: Vec<Edit>,
    pub ids: Vec<String>,
}

/// Optional placement of a newly created object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    /// Explicit ID; when omitted a unique ID is derived from the preset or prefab name.
    pub id: Option<String>,
    /// Local position (relative to `parent` when one is given).
    pub position: Option<[f32; 3]>,
    pub parent: Option<String>,
    /// Scene that receives membership of the new object.
    pub scene: Option<String>,
}

fn entity_path(id: &str) -> String {
    format!("/entities/@{}", escape(id))
}

fn scene_path(id: &str) -> String {
    format!("/scenes/@{}", escape(id))
}

fn not_found(path: String, message: String) -> anyhow::Error {
    Diagnostic::error("not_found", path, message)
}

fn require_entity(world: &World, id: &str) -> Result<()> {
    if world.entity(id).is_none() {
        return Err(not_found(
            entity_path(id),
            format!("entity '{id}' not found"),
        ));
    }
    Ok(())
}

fn require_scene<'a>(world: &'a World, id: &str) -> Result<&'a Scene> {
    world
        .scenes
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| not_found(scene_path(id), format!("scene '{id}' not found")))
}

fn check_new_id(path: String, id: &str) -> Result<()> {
    if valid_id(id).is_err() {
        return Err(Diagnostic::error(
            "invalid_id",
            path,
            format!("invalid id '{id}' (1..128 of letters, digits, . _ / -)"),
        ));
    }
    Ok(())
}

/// `base` if free, else `base/2`, `base/3`, ... skipping entity IDs in the world and `taken`.
pub fn unique_id(world: &World, base: &str, taken: &BTreeSet<String>) -> String {
    let free = |id: &str| world.entity(id).is_none() && !taken.contains(id);
    if free(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}/{n}"))
        .find(|id| free(id))
        .unwrap()
}

/// Euler degrees (XYZ order) to a unit quaternion, the editor's rotation convention.
pub fn rotation_from_degrees([x, y, z]: [f32; 3]) -> [f32; 4] {
    Quat::from_euler(
        EulerRot::XYZ,
        x.to_radians(),
        y.to_radians(),
        z.to_radians(),
    )
    .normalize()
    .to_array()
}

/// The effective (prefab-inherited + authored) `render.camera` of an entity, if any.
fn camera_of(world: &World, entity: &Entity) -> Option<Value> {
    world
        .resolved_components(entity)
        .remove("render.camera")
        .filter(Value::is_object)
}

fn any_active_camera(world: &World) -> bool {
    world.entities.iter().any(|e| {
        camera_of(world, e)
            .and_then(|v| serde_json::from_value::<Camera>(v).ok())
            .is_some_and(|c| c.active)
    })
}

/// Validates `place` and returns the ID the new object will get.
fn placement_id(world: &World, base: &str, place: &Placement) -> Result<String> {
    if let Some(parent) = &place.parent {
        require_entity(world, parent)?;
    }
    if let Some(scene) = &place.scene {
        require_scene(world, scene)?;
    }
    if let Some(p) = place.position {
        if !p.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0) {
            return Err(Diagnostic::error(
                "invalid_value",
                "/position",
                "position must be finite and within ±1e6",
            ));
        }
    }
    match &place.id {
        Some(id) => {
            check_new_id(entity_path(id), id)?;
            if world.entity(id).is_some() {
                return Err(Diagnostic::error(
                    "id_taken",
                    entity_path(id),
                    format!("entity '{id}' already exists"),
                ));
            }
            Ok(id.clone())
        }
        None => {
            let id = unique_id(world, base, &BTreeSet::new());
            check_new_id(entity_path(&id), &id)?;
            Ok(id)
        }
    }
}

/// Puts `entity` plus its parent relation and scene membership (in that order).
fn place_entity(entity: Entity, place: &Placement) -> Vec<Edit> {
    let id = entity.id.clone();
    let mut edits = vec![Edit::Core(CoreEdit::PutEntity { entity })];
    if let Some(parent) = &place.parent {
        edits.push(Edit::Core(CoreEdit::Reparent {
            entity: id.clone(),
            parent: Some(parent.clone()),
            keep_world: false,
        }));
    }
    if let Some(scene) = &place.scene {
        edits.push(Edit::Core(CoreEdit::SetSceneMembership {
            scene: scene.clone(),
            entity: id,
            member: true,
        }));
    }
    edits
}

/// The `render.light` a light preset creates (`None` for other presets).
pub fn preset_light(preset: &str) -> Option<RenderLight> {
    let mut light = RenderLight::directional([1.0, 0.95, 0.85], 1.0, true);
    match preset {
        "light.directional" => {}
        "light.point" => {
            light.kind = LightKind::Point;
            light.color = [1.0, 0.85, 0.6];
            light.intensity = 2.0;
            light.range = Some(6.0);
            light.shadows = false;
        }
        "light.spot" => {
            light.kind = LightKind::Spot;
            light.color = [1.0, 0.95, 0.85];
            light.intensity = 3.0;
            light.range = Some(10.0);
            light.spot = Some(SpotCone {
                inner_degrees: 30.0,
                outer_degrees: 45.0,
            });
            light.shadows = false;
        }
        "light.area" => {
            light.kind = LightKind::Area;
            light.color = [0.9, 0.95, 1.0];
            light.intensity = 2.0;
            light.range = Some(8.0);
            light.area = Some(AreaLight {
                shape: AreaShape::Rect,
                size: [2.0, 1.0],
            });
            light.shadows = false;
        }
        _ => return None,
    }
    Some(light)
}

/// Creates an object from an editor preset: `empty`, `cube` (one default mesh part),
/// `camera` (perspective; active only when no other camera is active) or a light
/// (`light.directional|point|spot|area`: a `render.light` aimed along local -Z; the
/// directional one matches the engine's default sun, the others point straight down).
/// Every preset but `empty` adds the `render` capability when the world lacks it.
pub fn create_object(world: &World, preset: &str, place: &Placement) -> Result<Created> {
    if !PRESETS.contains(&preset) {
        let hint = if preset == "light" {
            "; lights need a kind: light point|spot|area|directional"
        } else {
            ""
        };
        return Err(Diagnostic::error(
            "unknown_preset",
            "/preset",
            format!(
                "unknown preset '{preset}' (use empty, cube, camera, light.directional|point|spot|area){hint}"
            ),
        ));
    }
    let id = placement_id(world, preset, place)?;
    let mut entity = Entity::new(&id);
    let mut transform = Transform::default();
    match preset {
        "cube" => transform.position[1] = 0.5,
        "camera" => {
            transform.position = [0.0, 3.0, 6.0];
            transform.rotation = rotation_from_degrees([-16.0, 0.0, 0.0]);
        }
        "light.directional" => {
            // Aim -Z along the default sun (it shines away from DEFAULT_SUN_TOWARD).
            transform.position = [0.0, 6.0, 0.0];
            let toward = glam::Vec3::from_array(DEFAULT_SUN_TOWARD).normalize();
            transform.rotation = Quat::from_rotation_arc(glam::Vec3::NEG_Z, -toward)
                .normalize()
                .to_array();
        }
        "light.point" => transform.position = [0.0, 2.0, 0.0],
        "light.spot" | "light.area" => {
            transform.position = [0.0, 3.0, 0.0];
            transform.rotation = rotation_from_degrees([-90.0, 0.0, 0.0]);
        }
        _ => {}
    }
    if let Some(p) = place.position {
        transform.position = p;
    }
    entity.set("core.transform", transform)?;
    if preset == "cube" {
        entity.set(
            "render.mesh",
            RenderMesh {
                parts: vec![MeshPart {
                    transform: Transform::default(),
                    color: [0.20, 0.65, 0.73],
                    emission: 0.0,
                    grid: false,
                }],
            },
        )?;
    }
    if preset == "camera" {
        let mut camera = Camera::perspective();
        camera.active = !any_active_camera(world);
        entity.set("render.camera", camera)?;
    }
    if let Some(light) = preset_light(preset) {
        entity.set("render.light", light)?;
    }
    let mut edits = vec![];
    if preset != "empty" && !world.capabilities.contains_key("render") {
        let mut capabilities = world.capabilities.clone();
        capabilities.insert("render".into(), 1);
        edits.push(Edit::set("/capabilities", json!(capabilities)));
    }
    edits.extend(place_entity(entity, place));
    Ok(Created {
        edits,
        ids: vec![id],
    })
}

/// Instantiates one prefab. The instance inherits every prefab component; only an explicit
/// position is authored on it. The default ID derives from the prefab ID.
pub fn instantiate_prefab(world: &World, prefab: &str, place: &Placement) -> Result<Created> {
    if !world.prefabs.contains_key(prefab) {
        return Err(not_found(
            format!("/prefabs/{}", escape(prefab)),
            format!("prefab '{prefab}' not found"),
        ));
    }
    let id = placement_id(world, prefab, place)?;
    let mut entity = Entity::new(&id);
    entity.prefab = Some(prefab.to_owned());
    if let Some(p) = place.position {
        entity
            .components
            .insert("core.transform".into(), json!({ "position": p }));
    }
    Ok(Created {
        edits: place_entity(entity, place),
        ids: vec![id],
    })
}

/// Duplicates entities (same parent and scenes) with IDs `<source>/copy`, `/copy/2`, ...
/// (the source part is cut to 24 characters). The batch keeps the given order. Each copy is
/// placed right after its source in the authored order (= right below it among its siblings),
/// via a `move_entity` edit when the copy would otherwise be appended at the end.
pub fn duplicate(world: &World, ids: &[String]) -> Result<Created> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "nothing to duplicate",
        ));
    }
    let mut taken = BTreeSet::new();
    let mut created = Created {
        edits: vec![],
        ids: vec![],
    };
    // Authored order as the edits so far leave it (copies are appended, then moved).
    let mut order: Vec<String> = world.entities.iter().map(|e| e.id.clone()).collect();
    for source in ids {
        require_entity(world, source)?;
        let base = format!("{}/copy", source.chars().take(24).collect::<String>());
        let id = unique_id(world, &base, &taken);
        taken.insert(id.clone());
        created.ids.push(id.clone());
        created.edits.push(Edit::Core(CoreEdit::DuplicateEntity {
            source: source.clone(),
            id: id.clone(),
        }));
        let at = order.iter().position(|e| e == source).unwrap();
        if let Some(next) = order.get(at + 1).cloned() {
            created.edits.push(Edit::Core(CoreEdit::MoveEntity {
                id: id.clone(),
                before: Some(next),
            }));
            order.insert(at + 1, id);
        } else {
            order.push(id);
        }
    }
    Ok(created)
}

/// Drops entities onto the ground: each moves vertically so its lowest point rests on the
/// highest surface below it, or on the y = 0 plane when nothing is below.
///
/// - Lowest point: the world-space bounds of the `render.mesh` parts of the entity and its
///   descendants (they move with it); without mesh parts, the entity pivot.
/// - Surfaces: the tops of other entities' `render.mesh` part bounds whose XZ footprint
///   overlaps the dropped one and that lie at or below its lowest point (1 mm tolerance).
///   Selected objects and their descendants are never surfaces, so a batch does not stack.
///   Imported/skinned models and procedural geometry are not surfaces (no authored bounds).
/// - Only the local `core.transform/position` changes (the world delta is mapped through the
///   parent). An entity whose ancestor is also selected moves only with that ancestor.
/// - Objects already resting on their surface yield no edit; the batch may be empty.
pub fn drop_to_ground(world: &World, ids: &[String]) -> Result<Vec<Edit>> {
    use crate::scene_edit::{global, local_transform, Hierarchy};
    use glam::{Mat4, Vec3};
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "nothing to drop to the ground",
        ));
    }
    for id in ids {
        require_entity(world, id)?;
    }
    let graph = Hierarchy::new(world);
    let selected: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let has_selected_ancestor = |id: &str| {
        let mut cursor = graph.parents.get(id).copied();
        for _ in 0..256 {
            let Some(p) = cursor else { return false };
            if selected.contains(p) {
                return true;
            }
            cursor = graph.parents.get(p).copied();
        }
        false
    };
    let subtree = |root: &str| {
        let mut out = vec![root.to_owned()];
        let mut i = 0;
        while i < out.len() {
            let children: Vec<&str> = graph
                .children
                .get(&Some(out[i].as_str()))
                .cloned()
                .unwrap_or_default();
            for child in children {
                if !out.iter().any(|o| o == child) {
                    out.push(child.to_owned());
                }
            }
            i += 1;
        }
        out
    };
    // World AABBs of an entity's mesh parts (unit boxes placed by their part transforms).
    let part_bounds = |entity: &Entity| -> Result<Vec<(Vec3, Vec3)>> {
        let Some(mesh) = world.resolved_components(entity).remove("render.mesh") else {
            return Ok(vec![]);
        };
        let Ok(mesh) = serde_json::from_value::<RenderMesh>(mesh) else {
            return Ok(vec![]);
        };
        let model = global(world, &graph, Some(&entity.id))?;
        Ok(mesh
            .parts
            .iter()
            .map(|part| {
                let m: Mat4 = model * part.transform.matrix();
                (0..8)
                    .map(|j: usize| {
                        m.transform_point3(Vec3::new(
                            (j & 1) as f32 - 0.5,
                            ((j >> 1) & 1) as f32 - 0.5,
                            ((j >> 2) & 1) as f32 - 0.5,
                        ))
                    })
                    .fold(
                        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                        |(lo, hi), p| (lo.min(p), hi.max(p)),
                    )
            })
            .filter(|(lo, hi)| lo.is_finite() && hi.is_finite())
            .collect())
    };
    let roots: Vec<&String> = {
        let mut seen = BTreeSet::new();
        ids.iter()
            .filter(|id| seen.insert(id.as_str()) && !has_selected_ancestor(id))
            .collect()
    };
    let moving: BTreeSet<String> = roots.iter().flat_map(|r| subtree(r)).collect();
    let mut surfaces = vec![];
    for e in world.entities.iter().filter(|e| !moving.contains(&e.id)) {
        surfaces.extend(part_bounds(e)?);
    }
    let mut edits = vec![];
    for root in roots {
        let entity = world.entity(root).unwrap();
        let path = format!("{}/components/core.transform", entity_path(root));
        let Some(local) = local_transform(world, entity)? else {
            return Err(Diagnostic::error(
                "no_transform",
                path,
                format!("entity '{root}' has no core.transform to move"),
            ));
        };
        let mut boxes = vec![];
        for id in subtree(root) {
            boxes.extend(part_bounds(world.entity(&id).unwrap())?);
        }
        let (lo, hi) = if boxes.is_empty() {
            let pivot = global(world, &graph, Some(root))?.transform_point3(Vec3::ZERO);
            (pivot, pivot)
        } else {
            boxes.iter().fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(a, b), (l, h)| (a.min(*l), b.max(*h)),
            )
        };
        let bottom = lo.y;
        let overlaps = |(l, h): &(Vec3, Vec3)| {
            l.x <= hi.x && h.x >= lo.x && l.z <= hi.z && h.z >= lo.z && h.y <= bottom + 1e-3
        };
        let ground = surfaces
            .iter()
            .filter(|s| overlaps(s))
            .map(|(_, h)| h.y)
            .reduce(f32::max)
            .unwrap_or(0.0);
        let dy = ground - bottom;
        if !dy.is_finite() || dy.abs() < 1e-5 {
            continue;
        }
        let parent = global(world, &graph, graph.parents.get(root.as_str()).copied())?;
        let delta = parent.inverse().transform_vector3(Vec3::new(0.0, dy, 0.0));
        let position = Vec3::from_array(local.position) + delta;
        if !position.is_finite() {
            return Err(Diagnostic::error(
                "unrepresentable_transform",
                path,
                "the parent transform cannot express this drop",
            ));
        }
        // Usually only y changes (no parent rotation): write just that leaf, so x/z keep their
        // authored text and the diff shows one change.
        let own_position = entity
            .components
            .get("core.transform")
            .and_then(|t| t.get("position"))
            .and_then(Value::as_array)
            .is_some_and(|p| p.len() == 3);
        let (field, value) = if own_position && delta.x.abs() < 1e-6 && delta.z.abs() < 1e-6 {
            ("/components/core.transform/position/1", json!(position.y))
        } else {
            (
                "/components/core.transform/position",
                json!(position.to_array()),
            )
        };
        edits.push(Edit::Core(CoreEdit::SetField {
            target: Target::Entity { id: root.clone() },
            field: field.into(),
            value,
        }));
    }
    Ok(edits)
}

/// Deletes entities. Children are kept (they become roots); relations and scene
/// memberships of the deleted objects are removed.
pub fn delete(world: &World, ids: &[String]) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "nothing to delete",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut edits = vec![];
    for id in ids {
        require_entity(world, id)?;
        if seen.insert(id) {
            edits.push(Edit::Core(CoreEdit::DeleteEntity { id: id.clone() }));
        }
    }
    Ok(edits)
}

/// Sets (or clears, with `None`) the parent of an entity. `keep_world` preserves the world
/// pose by rewriting the local transform (the editor's drag-and-drop default).
pub fn reparent(
    world: &World,
    entity: &str,
    parent: Option<&str>,
    keep_world: bool,
) -> Result<Vec<Edit>> {
    require_entity(world, entity)?;
    if let Some(parent) = parent {
        require_entity(world, parent)?;
    }
    Ok(vec![Edit::Core(CoreEdit::Reparent {
        entity: entity.to_owned(),
        parent: parent.map(str::to_owned),
        keep_world,
    })])
}

/// Creates an empty scene.
pub fn create_scene(world: &World, id: &str, active: bool) -> Result<Vec<Edit>> {
    check_new_id(scene_path(id), id)?;
    if world.scenes.iter().any(|s| s.id == id) {
        return Err(Diagnostic::error(
            "id_taken",
            scene_path(id),
            format!("scene '{id}' already exists"),
        ));
    }
    Ok(vec![Edit::Core(CoreEdit::PutScene {
        scene: Scene {
            id: id.to_owned(),
            entities: vec![],
            active,
        },
    })])
}

/// Activates or deactivates a scene (objects only in inactive scenes are hidden/disabled).
pub fn set_scene_active(world: &World, id: &str, active: bool) -> Result<Vec<Edit>> {
    require_scene(world, id)?;
    Ok(vec![Edit::Core(CoreEdit::SetField {
        target: Target::Scene { id: id.to_owned() },
        field: "/active".into(),
        value: json!(active),
    })])
}

/// Deletes a scene. Only membership is removed; the objects are kept.
pub fn delete_scene(world: &World, id: &str) -> Result<Vec<Edit>> {
    require_scene(world, id)?;
    Ok(vec![Edit::Core(CoreEdit::RemoveObject {
        target: Target::Scene { id: id.to_owned() },
    })])
}

/// Adds or removes an entity's membership in a scene.
pub fn set_scene_membership(
    world: &World,
    scene: &str,
    entity: &str,
    member: bool,
) -> Result<Vec<Edit>> {
    require_scene(world, scene)?;
    require_entity(world, entity)?;
    Ok(vec![Edit::Core(CoreEdit::SetSceneMembership {
        scene: scene.to_owned(),
        entity: entity.to_owned(),
        member,
    })])
}

/// Makes `id` the main camera: `render.camera.active` becomes true on it and false on every
/// other active camera, in one batch.
pub fn make_main_camera(world: &World, id: &str) -> Result<Vec<Edit>> {
    let entity = world
        .entity(id)
        .ok_or_else(|| not_found(entity_path(id), format!("entity '{id}' not found")))?;
    if camera_of(world, entity).is_none() {
        return Err(Diagnostic::error(
            "not_a_camera",
            entity_path(id),
            format!("entity '{id}' has no render.camera component"),
        ));
    }
    let set = |id: &str, active: bool| {
        Edit::Core(CoreEdit::SetField {
            target: Target::Entity { id: id.to_owned() },
            field: "/components/render.camera/active".into(),
            value: json!(active),
        })
    };
    let mut edits = vec![set(id, true)];
    for other in world.entities.iter().filter(|e| e.id != id) {
        if camera_of(world, other).is_some_and(|c| c["active"] == true) {
            edits.push(set(&other.id, false));
        }
    }
    Ok(edits)
}

/// Sets an entity's display label. Surrounding whitespace is trimmed; an empty label clears
/// it (the hierarchy then shows the ID). Labels are 1..128 bytes without control characters.
pub fn rename(world: &World, id: &str, label: &str) -> Result<Vec<Edit>> {
    require_entity(world, id)?;
    let label = label.trim();
    if label.len() > 128 || label.chars().any(char::is_control) {
        return Err(Diagnostic::error(
            "invalid_value",
            format!("{}/label", entity_path(id)),
            "label must be at most 128 bytes of display text",
        ));
    }
    Ok(vec![Edit::Core(CoreEdit::SetField {
        target: Target::Entity { id: id.to_owned() },
        field: "/label".into(),
        value: if label.is_empty() {
            Value::Null
        } else {
            json!(label)
        },
    })])
}

/// Where `move_entity` puts an entity in the authored order (= sibling order).
#[derive(Clone, Debug, PartialEq)]
pub enum Order {
    Before(String),
    After(String),
    End,
}

/// Reorders an entity among the authored entities, which is also its position among its
/// siblings in the hierarchy. `After(x)` compiles to "before the entity that follows x".
/// Moving to where it already is yields an empty batch.
pub fn move_entity(world: &World, id: &str, to: &Order) -> Result<Vec<Edit>> {
    require_entity(world, id)?;
    let other = match to {
        Order::Before(o) | Order::After(o) => Some(o.as_str()),
        Order::End => None,
    };
    if let Some(other) = other {
        require_entity(world, other)?;
        if other == id {
            return Err(Diagnostic::error(
                "invalid_value",
                entity_path(id),
                "an entity cannot be moved relative to itself",
            ));
        }
    }
    let rest: Vec<&str> = world
        .entities
        .iter()
        .map(|e| e.id.as_str())
        .filter(|e| *e != id)
        .collect();
    let before = match to {
        Order::Before(o) => Some(o.clone()),
        Order::After(o) => {
            let i = rest.iter().position(|e| e == o).unwrap();
            rest.get(i + 1).map(|e| (*e).to_owned())
        }
        Order::End => None,
    };
    let current = world.entities.iter().position(|e| e.id == id).unwrap();
    let next = world.entities.get(current + 1).map(|e| e.id.as_str());
    if next == before.as_deref() {
        return Ok(vec![]);
    }
    Ok(vec![Edit::Core(CoreEdit::MoveEntity {
        id: id.to_owned(),
        before,
    })])
}

/// Default prefab ID for "Create Prefab From Selection": the entity ID, then `ID/2`, ...
pub fn prefab_id_for(world: &World, entity: &str) -> String {
    let base: String = entity.chars().take(120).collect();
    if !world.prefabs.contains_key(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}/{n}"))
        .find(|id| !world.prefabs.contains_key(id))
        .unwrap()
}

/// Turns an entity into a prefab instance. The new prefab receives the entity's resolved
/// components (prefab-inherited values merged with its overrides, so an existing instance
/// loses nothing); the entity then points at the new prefab and keeps only its pose
/// (`core.transform`) as an override, with the prefab's own transform left at the default, so
/// further instances do not all appear at this spot. Label, tags, parent and scenes stay.
/// With `id = None` the prefab ID is [`prefab_id_for`].
pub fn create_prefab(world: &World, id: Option<&str>, entity: &str) -> Result<Created> {
    let source = world
        .entity(entity)
        .ok_or_else(|| not_found(entity_path(entity), format!("entity '{entity}' not found")))?;
    let id = match id {
        Some(id) => id.to_owned(),
        None => prefab_id_for(world, entity),
    };
    let path = format!("/prefabs/{}", escape(&id));
    check_new_id(path.clone(), &id)?;
    if world.prefabs.contains_key(&id) {
        return Err(Diagnostic::error(
            "id_taken",
            path,
            format!("prefab '{id}' already exists"),
        ));
    }
    let mut components = world.resolved_components(source);
    if components.is_empty() {
        return Err(Diagnostic::error(
            "empty_entity",
            entity_path(entity),
            format!("entity '{entity}' has no components to put in a prefab"),
        ));
    }
    let mut instance = source.clone();
    instance.prefab = Some(id.clone());
    instance.components.clear();
    if let Some(transform) = components.get_mut("core.transform") {
        instance.components.insert(
            "core.transform".into(),
            std::mem::replace(transform, json!({})),
        );
    }
    Ok(Created {
        edits: vec![
            Edit::Core(CoreEdit::PutPrefab {
                id: id.clone(),
                prefab: Prefab { components },
            }),
            Edit::Core(CoreEdit::PutEntity { entity: instance }),
        ],
        ids: vec![id],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut w = World::empty("ops");
        w.entities.push(Entity::new("a"));
        w.scenes.push(Scene {
            id: "s".into(),
            entities: vec![],
            active: true,
        });
        w
    }

    fn code(e: anyhow::Error) -> String {
        e.downcast_ref::<Diagnostic>().unwrap().code.clone()
    }

    #[test]
    fn unique_ids_skip_world_and_taken() {
        let w = world();
        assert_eq!(unique_id(&w, "b", &BTreeSet::new()), "b");
        assert_eq!(unique_id(&w, "a", &BTreeSet::new()), "a/2");
        assert_eq!(unique_id(&w, "a", &BTreeSet::from(["a/2".into()])), "a/3");
    }

    #[test]
    fn errors_are_structured() {
        let w = world();
        let place = |f: fn(&mut Placement)| {
            let mut p = Placement::default();
            f(&mut p);
            p
        };
        assert_eq!(
            code(create_object(&w, "light", &Placement::default()).unwrap_err()),
            "unknown_preset"
        );
        assert_eq!(
            code(create_object(&w, "cube", &place(|p| p.id = Some("a".into()))).unwrap_err()),
            "id_taken"
        );
        assert_eq!(
            code(create_object(&w, "cube", &place(|p| p.scene = Some("x".into()))).unwrap_err()),
            "not_found"
        );
        assert_eq!(
            code(create_object(&w, "cube", &place(|p| p.id = Some("bad id".into()))).unwrap_err()),
            "invalid_id"
        );
        assert_eq!(code(make_main_camera(&w, "a").unwrap_err()), "not_a_camera");
        assert_eq!(code(create_scene(&w, "s", true).unwrap_err()), "id_taken");
        assert_eq!(code(delete_scene(&w, "x").unwrap_err()), "not_found");
        assert_eq!(code(duplicate(&w, &[]).unwrap_err()), "empty_selection");
        assert_eq!(
            code(instantiate_prefab(&w, "nope", &Placement::default()).unwrap_err()),
            "not_found"
        );
        assert_eq!(code(rename(&w, "x", "A").unwrap_err()), "not_found");
        assert_eq!(
            code(rename(&w, "a", &"x".repeat(129)).unwrap_err()),
            "invalid_value"
        );
        assert_eq!(
            code(move_entity(&w, "a", &Order::Before("a".into())).unwrap_err()),
            "invalid_value"
        );
        assert_eq!(
            code(move_entity(&w, "a", &Order::After("x".into())).unwrap_err()),
            "not_found"
        );
        assert_eq!(
            code(create_prefab(&w, None, "a").unwrap_err()),
            "empty_entity"
        );
    }

    fn apply(mut w: World, edits: Vec<Edit>) -> World {
        let r = Registry::core();
        for e in edits {
            e.apply(&mut w, &r).unwrap();
        }
        w
    }

    fn order(w: &World) -> Vec<&str> {
        w.entities.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn rename_trims_and_empty_clears() {
        let w = world();
        let w = apply(w.clone(), rename(&w, "a", "  路燈 A ").unwrap());
        assert_eq!(w.entity("a").unwrap().label.as_deref(), Some("路燈 A"));
        let w = apply(w.clone(), rename(&w, "a", " ").unwrap());
        assert_eq!(w.entity("a").unwrap().label, None);
    }

    #[test]
    fn move_entity_reorders_before_after_and_end() {
        let mut w = world();
        for id in ["b", "c", "d"] {
            w.entities.push(Entity::new(id));
        }
        let step =
            |w: &World, id: &str, to: Order| apply(w.clone(), move_entity(w, id, &to).unwrap());
        let w = step(&w, "d", Order::Before("a".into()));
        assert_eq!(order(&w), ["d", "a", "b", "c"]);
        let w = step(&w, "d", Order::After("b".into()));
        assert_eq!(order(&w), ["a", "b", "d", "c"]);
        let w = step(&w, "a", Order::End);
        assert_eq!(order(&w), ["b", "d", "c", "a"]);
        let w = step(&w, "c", Order::After("a".into()));
        assert_eq!(order(&w), ["b", "d", "a", "c"]);
        // Already in place: no edit at all.
        assert!(move_entity(&w, "d", &Order::After("b".into()))
            .unwrap()
            .is_empty());
        assert!(move_entity(&w, "c", &Order::End).unwrap().is_empty());
    }

    #[test]
    fn duplicates_land_right_after_their_sources() {
        let mut w = world();
        for id in ["b", "c"] {
            w.entities.push(Entity::new(id));
        }
        let ids: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let w2 = apply(w.clone(), duplicate(&w, &ids).unwrap().edits);
        assert_eq!(order(&w2), ["a", "a/copy", "b", "b/copy", "c", "c/copy"]);
        // Selection order is kept for IDs; placement still follows each source.
        let ids: Vec<String> = ["c", "a"].map(String::from).to_vec();
        let w3 = apply(w.clone(), duplicate(&w, &ids).unwrap().edits);
        assert_eq!(order(&w3), ["a", "a/copy", "b", "c", "c/copy"]);
    }

    fn placed(id: &str, position: [f32; 3], mesh: Option<Value>) -> Entity {
        let mut e = Entity::new(id);
        e.components
            .insert("core.transform".into(), json!({ "position": position }));
        if let Some(mesh) = mesh {
            e.components.insert("render.mesh".into(), mesh);
        }
        e
    }

    fn y(w: &World, id: &str) -> f32 {
        w.entity(id).unwrap().components["core.transform"]["position"][1]
            .as_f64()
            .unwrap() as f32
    }

    #[test]
    fn drop_to_ground_rests_on_surfaces_or_the_zero_plane() {
        let unit = json!({"parts":[{"transform":{},"color":[1,1,1]}]});
        let mut w = World::empty("ground");
        // A 2 m tall table top under x=0 (top at y=1), a crate above it, a crate off to the side,
        // a pivot-only empty, and a child cube under an empty parent at y=5.
        w.entities.push(placed(
            "table",
            [0.0, 0.5, 0.0],
            Some(json!({"parts":[{"transform":{"scale":[2,1,2]},"color":[1,1,1]}]})),
        ));
        w.entities
            .push(placed("crate", [0.2, 4.0, 0.0], Some(unit.clone())));
        w.entities
            .push(placed("side", [10.0, 3.0, 0.0], Some(unit.clone())));
        w.entities.push(placed("marker", [10.0, 2.0, 5.0], None));
        w.entities.push(placed("group", [20.0, 5.0, 0.0], None));
        w.entities
            .push(placed("group/box", [0.0, 1.0, 0.0], Some(unit.clone())));
        w.relations.push(Relation {
            source: "group/box".into(),
            kind: "parent".into(),
            target: "group".into(),
        });
        let ids: Vec<String> = ["crate", "side", "marker", "group", "group/box"]
            .map(String::from)
            .to_vec();
        let edits = drop_to_ground(&w, &ids).unwrap();
        assert_eq!(edits.len(), 4, "group/box moves with its selected parent");
        let after = apply(w.clone(), edits);
        assert!(
            (y(&after, "crate") - 1.5).abs() < 1e-5,
            "rests on the table"
        );
        assert!((y(&after, "side") - 0.5).abs() < 1e-5, "rests on y=0");
        assert!(y(&after, "marker").abs() < 1e-5, "pivot on y=0");
        // The child box bottom was at 5+1-0.5 = 5.5; the parent drops by 5.5.
        assert!((y(&after, "group") + 0.5).abs() < 1e-5);
        assert_eq!(y(&after, "group/box"), 1.0);
        // Already grounded: nothing to do.
        assert!(drop_to_ground(&after, &ids).unwrap().is_empty());
        // A child alone maps the world delta through its parent's scale.
        let mut scaled = w.clone();
        scaled.entity_mut("group").unwrap().components.insert(
            "core.transform".into(),
            json!({"position":[20.0, 5.0, 0.0], "scale":[1.0, 2.0, 1.0]}),
        );
        let edits = drop_to_ground(&scaled, &["group/box".into()]).unwrap();
        let after = apply(scaled, edits);
        // World bottom = 5 + 2*(1 - 0.5) = 6 -> needs -6 world = -3 local.
        assert!((y(&after, "group/box") + 2.0).abs() < 1e-5);
        assert_eq!(
            code(drop_to_ground(&w, &[]).unwrap_err()),
            "empty_selection"
        );
        let mut bare = World::empty("bare");
        bare.entities.push(Entity::new("x"));
        assert_eq!(
            code(drop_to_ground(&bare, &["x".into()]).unwrap_err()),
            "no_transform"
        );
    }

    #[test]
    fn create_prefab_keeps_resolved_components_and_pose() {
        let mut w = world();
        let mut base = Prefab::default();
        base.components
            .insert("render.mesh".into(), json!({"parts":[{"color":[1,0,0]}]}));
        base.components
            .insert("core.transform".into(), json!({"scale":[2,2,2]}));
        w.prefabs.insert("a".into(), base);
        let e = w.entity_mut("a").unwrap();
        e.prefab = Some("a".into());
        e.label = Some("Lamp".into());
        e.components
            .insert("core.transform".into(), json!({"position":[1,2,3]}));
        e.components.insert("x.y".into(), json!({"v":1}));
        let before = w.resolved_components(w.entity("a").unwrap());
        let created = create_prefab(&w, None, "a").unwrap();
        assert_eq!(created.ids, ["a/2"], "prefab 'a' exists, so the ID gets /2");
        let after = apply(w.clone(), created.edits);
        let e = after.entity("a").unwrap();
        assert_eq!(e.prefab.as_deref(), Some("a/2"));
        assert_eq!(e.label.as_deref(), Some("Lamp"));
        assert_eq!(e.components.keys().collect::<Vec<_>>(), ["core.transform"]);
        assert_eq!(after.prefabs["a/2"].components["core.transform"], json!({}));
        assert_eq!(after.resolved_components(e), before);
        assert_eq!(
            code(create_prefab(&after, Some("a/2"), "a").unwrap_err()),
            "id_taken"
        );
    }
}
