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

pub use crate::ops_assets::*;
pub use crate::ops_graph::*;
pub use crate::ops_ui::*;
pub use crate::ops_world::*;

/// Presets offered by the editor's "+ Object" menu. ACL: `create light point` is
/// shorthand for `create light.point` (and `create trigger box` for `create trigger.box`).
pub const PRESETS: [&str; 11] = [
    "empty",
    "cube",
    "camera",
    "light.directional",
    "light.point",
    "light.spot",
    "light.area",
    "trigger.box",
    "trigger.sphere",
    "path",
    "obstacle",
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
    /// Local rotation as Euler degrees (XYZ, the Inspector's Rotation° fields).
    pub rotation: Option<[f32; 3]>,
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
    if let Some(r) = place.rotation {
        if !r.iter().all(|v| v.is_finite() && v.abs() <= 36_000.0) {
            return Err(Diagnostic::error(
                "invalid_value",
                "/rotation",
                "rotation degrees must be finite and within ±36000",
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

/// The `trigger.zone` a trigger preset creates (`None` for other presets): a 1 m box or a
/// 1.5 m sphere that players (tag `player`) trigger, with no events until some are named.
pub fn preset_trigger(preset: &str) -> Option<crate::trigger::TriggerZone> {
    let mut zone = match preset {
        "trigger.box" => crate::trigger::TriggerZone::boxed([1.0; 3]),
        "trigger.sphere" => crate::trigger::TriggerZone::sphere(1.5),
        _ => return None,
    };
    zone.filter.tags = vec!["player".into()];
    Some(zone)
}

/// Edits that add `capability` and `system` to the world when it lacks them.
fn enable_system(world: &World, capability: Option<&str>, system: Option<&str>) -> Vec<Edit> {
    let mut edits = vec![];
    if let Some(capability) = capability.filter(|c| !world.capabilities.contains_key(*c)) {
        let mut capabilities = world.capabilities.clone();
        capabilities.insert(capability.into(), 1);
        edits.push(Edit::set("/capabilities", json!(capabilities)));
    }
    if let Some(system) = system.filter(|s| !world.systems.iter().any(|x| x == s)) {
        let mut systems = world.systems.clone();
        systems.push(system.into());
        edits.push(Edit::set("/systems", json!(systems)));
    }
    edits
}

/// Creates an object from an editor preset: `empty`, `cube` (one default mesh part),
/// `camera` (perspective; active only when no other camera is active) or a light
/// (`light.directional|point|spot|area`: a `render.light` aimed along local -Z; the
/// directional one matches the engine's default sun, the others point straight down).
/// Trigger presets (`trigger.box|sphere`) add a `trigger.zone` and, when missing, the
/// `trigger` capability and the `trigger.detect` system; `path` adds an 8 x 6 m closed
/// rectangle `path.spline` and, when missing, the `path` capability and the `path.move`
/// system; `obstacle` is a 1 m cube with a matching `nav.blocker` (navigation agents, FSM
/// agents and the action controller walk around it). Every other preset but `empty` adds the
/// `render` capability when the world lacks it.
pub fn create_object(world: &World, preset: &str, place: &Placement) -> Result<Created> {
    if !PRESETS.contains(&preset) {
        let hint = match preset {
            "light" => "; lights need a kind: light point|spot|area|directional",
            "trigger" => "; triggers need a shape: trigger box|sphere",
            _ => "",
        };
        return Err(Diagnostic::error(
            "unknown_preset",
            "/preset",
            format!(
                "unknown preset '{preset}' (use empty, cube, camera, light.directional|point|spot|area, trigger.box|sphere, path, obstacle){hint}"
            ),
        ));
    }
    let id = placement_id(world, preset, place)?;
    let mut entity = Entity::new(&id);
    let mut transform = Transform::default();
    match preset {
        "cube" | "obstacle" => transform.position[1] = 0.5,
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
    if let Some(r) = place.rotation {
        transform.rotation = rotation_from_degrees(r);
    }
    entity.set("core.transform", transform)?;
    if preset == "cube" || preset == "obstacle" {
        entity.set(
            "render.mesh",
            RenderMesh {
                parts: vec![MeshPart {
                    transform: Transform::default(),
                    color: if preset == "obstacle" {
                        [0.55, 0.52, 0.48]
                    } else {
                        [0.20, 0.65, 0.73]
                    },
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
    let trigger = preset_trigger(preset);
    let (capability, system) = match (preset, &trigger) {
        ("empty", _) => (None, None),
        (_, Some(_)) => (Some("trigger"), Some("trigger.detect")),
        ("path", _) => (Some("path"), Some("path.move")),
        _ => (Some("render"), None),
    };
    if let Some(zone) = trigger {
        entity.set("trigger.zone", zone)?;
    }
    if preset == "path" {
        entity.set("path.spline", crate::path::PathSpline::preset())?;
    }
    if preset == "obstacle" {
        entity.set(
            "nav.blocker",
            crate::blocker::NavBlocker {
                half: [0.5, 0.5],
                offset: [0.0, 0.0],
            },
        )?;
    }
    let mut edits = enable_system(world, capability, system);
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
    let mut transform = serde_json::Map::new();
    if let Some(p) = place.position {
        transform.insert("position".into(), json!(p));
    }
    if let Some(r) = place.rotation {
        transform.insert("rotation".into(), json!(rotation_from_degrees(r)));
    }
    if !transform.is_empty() {
        entity
            .components
            .insert("core.transform".into(), Value::Object(transform));
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

/// Creates an empty group object and moves `ids` under it, keeping their world poses, in one
/// batch (`ACL: group SEL [id=ID]`). An object whose ancestor is also listed stays under that
/// ancestor. When every grouped object has the same parent the group goes under it, at the
/// mean of their local positions, just before the first of them in the authored order;
/// otherwise it is a root object at the origin. The group joins every scene that all grouped
/// objects belong to. The default ID is `group` (`group/2`, ...).
pub fn create_group(world: &World, ids: &[String], id: Option<&str>) -> Result<Created> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "nothing to group",
        ));
    }
    for id in ids {
        require_entity(world, id)?;
    }
    let graph = crate::scene_edit::Hierarchy::new(world);
    let listed: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let covered = |id: &str| {
        let mut cursor = graph.parents.get(id).copied();
        while let Some(ancestor) = cursor {
            if listed.contains(ancestor) {
                return true;
            }
            cursor = graph.parents.get(ancestor).copied();
        }
        false
    };
    // Authored order, top-most objects only.
    let members: Vec<&str> = world
        .entities
        .iter()
        .map(|e| e.id.as_str())
        .filter(|id| listed.contains(id) && !covered(id))
        .collect();
    let parents: BTreeSet<Option<&str>> = members
        .iter()
        .map(|m| graph.parents.get(m).copied())
        .collect();
    let shared = parents.len() == 1;
    let parent = if shared {
        parents.into_iter().next().flatten()
    } else {
        None
    };
    let mut position = [0.0_f32; 3];
    if shared {
        for m in &members {
            let local = world
                .entity(m)
                .and_then(|e| world.resolved_components(e).remove("core.transform"))
                .and_then(|v| serde_json::from_value::<Transform>(v).ok())
                .unwrap_or_default();
            for (sum, v) in position.iter_mut().zip(local.position) {
                *sum += v / members.len() as f32;
            }
        }
    }
    let place = Placement {
        id: id.map(str::to_owned),
        position: Some(position),
        rotation: None,
        parent: parent.map(str::to_owned),
        scene: None,
    };
    let group = placement_id(world, "group", &place)?;
    let mut entity = Entity::new(&group);
    entity.set(
        "core.transform",
        Transform {
            position,
            ..Default::default()
        },
    )?;
    let mut edits = place_entity(entity, &place);
    if shared {
        edits.push(Edit::Core(CoreEdit::MoveEntity {
            id: group.clone(),
            before: Some(members[0].to_owned()),
        }));
    }
    for scene in world
        .scenes
        .iter()
        .filter(|s| members.iter().all(|m| s.entities.iter().any(|e| e == m)))
    {
        edits.push(Edit::Core(CoreEdit::SetSceneMembership {
            scene: scene.id.clone(),
            entity: group.clone(),
            member: true,
        }));
    }
    for m in &members {
        edits.push(Edit::Core(CoreEdit::Reparent {
            entity: (*m).to_owned(),
            parent: Some(group.clone()),
            keep_world: true,
        }));
    }
    Ok(Created {
        edits,
        ids: vec![group],
    })
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

/// Options of Object ▸ Camera Follow Selected / ACL `camera SEL follow TARGET`; `None` keeps
/// the `camera.follow` default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraFollowOptions {
    pub distance: Option<f32>,
    pub min_distance: Option<f32>,
    pub max_distance: Option<f32>,
    pub yaw: Option<f32>,
    pub pitch: Option<f32>,
    pub look_height: Option<f32>,
    /// `input.bindings` axis that orbits (`input.yaw_axis`).
    pub yaw_axis: Option<String>,
    /// `input.bindings` axis that zooms (`input.zoom_axis`).
    pub zoom_axis: Option<String>,
    pub radius: Option<f32>,
    pub collision: Option<bool>,
}

/// Object ▸ Camera Follow Selected: puts a `camera.follow` of `target` (defaults plus
/// `options`) on each camera, replacing an existing one, and adds the `camera` capability
/// and the `camera.rig` system when missing. Every camera needs `render.camera`; the target
/// must exist and must not be one of the cameras. Input axes must already be bound and the
/// camera must not have another mover (both checked when the batch commits:
/// `invalid_camera` / `camera_conflict`).
pub fn add_camera_follow(
    world: &World,
    cameras: &[String],
    target: &str,
    options: &CameraFollowOptions,
) -> Result<Vec<Edit>> {
    if cameras.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the camera that should follow the target",
        ));
    }
    require_entity(world, target)?;
    let mut follow = crate::camera::CameraFollow::new(target);
    follow.min_distance = options.min_distance.unwrap_or(follow.min_distance);
    follow.max_distance = options.max_distance.unwrap_or(follow.max_distance);
    // Without an explicit distance the default one is kept inside the requested range.
    follow.distance = options.distance.unwrap_or_else(|| {
        follow.distance.clamp(
            follow.min_distance,
            follow.max_distance.max(follow.min_distance),
        )
    });
    follow.yaw = options.yaw.unwrap_or(follow.yaw);
    follow.pitch = options.pitch.unwrap_or(follow.pitch);
    follow.look_height = options.look_height.unwrap_or(follow.look_height);
    if let Some(axis) = &options.yaw_axis {
        follow.input.yaw_axis = axis.clone();
    }
    if let Some(axis) = &options.zoom_axis {
        follow.input.zoom_axis = axis.clone();
    }
    follow.collision.radius = options.radius.unwrap_or(follow.collision.radius);
    follow.collision.enabled = options.collision.unwrap_or(follow.collision.enabled);
    follow.validate().map_err(|e| {
        Diagnostic::error(
            "invalid_value",
            "/components/camera.follow",
            format!("{e:#}"),
        )
    })?;
    let mut edits = enable_system(world, Some("camera"), Some("camera.rig"));
    let mut seen = BTreeSet::new();
    for id in cameras {
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
        if id == target {
            return Err(Diagnostic::error(
                "invalid_value",
                format!("{}/components/camera.follow/target", entity_path(id)),
                format!("camera '{id}' cannot follow itself"),
            ));
        }
        if seen.insert(id) {
            edits.push(Edit::Core(CoreEdit::SetComponent {
                entity: id.clone(),
                component: "camera.follow".into(),
                value: json!(follow),
            }));
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

/// Removes a component authored on each entity (Inspector ▸ Remove component). A component
/// the entity only inherits from its prefab cannot be removed here (edit the prefab).
pub fn remove_component(world: &World, ids: &[String], component: &str) -> Result<Vec<Edit>> {
    let mut edits = vec![];
    for id in ids {
        require_entity(world, id)?;
        let entity = world.entity(id).unwrap();
        if !entity.components.contains_key(component) {
            let inherited = entity
                .prefab
                .as_ref()
                .and_then(|p| world.prefabs.get(p))
                .is_some_and(|p| p.components.contains_key(component));
            return Err(not_found(
                format!("{}/components/{}", entity_path(id), escape(component)),
                if inherited {
                    format!("'{id}' inherits {component} from its prefab; edit or override the prefab instead")
                } else {
                    format!("'{id}' has no component {component}")
                },
            ));
        }
        edits.push(Edit::Core(CoreEdit::RemoveComponent {
            entity: id.clone(),
            component: component.to_owned(),
        }));
    }
    Ok(edits)
}

/// Options of `follow_path` (`None` = the `path.follow` default).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FollowOptions {
    pub speed: Option<f32>,
    pub end: Option<crate::path::PathEnd>,
    pub offset: Option<f32>,
}

/// The effective `path.spline` of an entity (prefab-inherited + authored).
fn path_of(world: &World, id: &str) -> Result<crate::path::PathSpline> {
    let entity = world
        .entity(id)
        .ok_or_else(|| not_found(entity_path(id), format!("entity '{id}' not found")))?;
    let value = world
        .resolved_components(entity)
        .remove("path.spline")
        .ok_or_else(|| {
            Diagnostic::error(
                "not_a_path",
                entity_path(id),
                format!("entity '{id}' has no path.spline component"),
            )
        })?;
    serde_json::from_value(value).map_err(|e| {
        Diagnostic::error(
            "invalid_path",
            format!("{}/components/path.spline", entity_path(id)),
            format!("{e}"),
        )
    })
}

/// Object ▸ Follow Path: puts a `path.follow` on each entity (replacing an existing one) that
/// follows `path`. Several followers start evenly spaced (`start_fraction` k/n in the given
/// order) so they do not overlap. Adds the `path` capability and the `path.move` system when
/// missing.
pub fn follow_path(
    world: &World,
    ids: &[String],
    path: &str,
    options: &FollowOptions,
) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the objects that should follow the path",
        ));
    }
    path_of(world, path)?;
    let mut edits = enable_system(world, Some("path"), Some("path.move"));
    let mut followers: Vec<&String> = vec![];
    for id in ids {
        require_entity(world, id)?;
        if id == path {
            return Err(Diagnostic::error(
                "invalid_value",
                entity_path(id),
                format!("'{id}' is the path itself; select the objects that should follow it"),
            ));
        }
        if !followers.contains(&id) {
            followers.push(id);
        }
    }
    let n = followers.len();
    for (k, id) in followers.into_iter().enumerate() {
        let mut follow = crate::path::PathFollow::new(path);
        follow.speed = options.speed.unwrap_or(follow.speed);
        follow.end = options.end.unwrap_or(follow.end);
        follow.offset = options.offset.unwrap_or(follow.offset);
        if n > 1 {
            follow.start_fraction = Some(k as f32 / n as f32);
        }
        follow.validate().map_err(|e| {
            Diagnostic::error(
                "invalid_value",
                format!("{}/components/path.follow", entity_path(id)),
                format!("{e:#}"),
            )
        })?;
        edits.push(Edit::Core(CoreEdit::SetComponent {
            entity: id.clone(),
            component: "path.follow".into(),
            value: json!(follow),
        }));
    }
    Ok(edits)
}

/// One change to a path's points (local coordinates of the path entity).
#[derive(Clone, Debug, PartialEq)]
pub enum PointEdit {
    /// Append; `None` extends the last piece by its own length.
    Add(Option<[f32; 3]>),
    /// Insert before index I (I = count appends); `None` splits the piece that ends at I
    /// (the closing piece for I = 0 or count on a closed path; an open path extends instead).
    Insert(usize, Option<[f32; 3]>),
    Move(usize, [f32; 3]),
    Remove(usize),
}

/// Where a new point goes when no coordinates are given (see `PointEdit::Insert`).
fn new_point(spline: &crate::path::PathSpline, index: usize) -> [f32; 3] {
    let p = |i: usize| glam::Vec3::from_array(spline.points[i]);
    let n = spline.points.len();
    let at = if spline.closed && (index == 0 || index == n) {
        (p(n - 1) + p(0)) * 0.5
    } else if index == 0 {
        p(0) * 2.0 - p(1)
    } else if index >= n {
        p(n - 1) * 2.0 - p(n - 2)
    } else {
        (p(index - 1) + p(index)) * 0.5
    };
    at.to_array()
}

/// Edits one point of the path on entity `id` (editor point handles and Inspector ▸ Path
/// points; ACL `path ID add|insert|move|remove`). Writes the whole `points` array; the result
/// must still be a valid path (2+ points, 3+ when closed, no coincident neighbours).
pub fn edit_path_point(world: &World, id: &str, edit: &PointEdit) -> Result<Vec<Edit>> {
    let mut spline = path_of(world, id)?;
    let n = spline.points.len();
    let points_path = format!("{}/components/path.spline/points", entity_path(id));
    let index_error = |i: usize, max: usize| {
        Diagnostic::error(
            "invalid_value",
            format!("{points_path}/{i}"),
            format!("point index {i} is out of range 0..{max}"),
        )
    };
    let checked = |at: [f32; 3]| -> Result<[f32; 3]> {
        if !at.iter().all(|v| v.is_finite()) {
            return Err(Diagnostic::error(
                "invalid_value",
                points_path.clone(),
                "point coordinates must be finite",
            ));
        }
        Ok(at)
    };
    match *edit {
        PointEdit::Add(at) => {
            let at = checked(at.unwrap_or_else(|| new_point(&spline, n)))?;
            spline.points.push(at);
        }
        PointEdit::Insert(i, at) => {
            if i > n {
                return Err(index_error(i, n));
            }
            let at = checked(at.unwrap_or_else(|| new_point(&spline, i)))?;
            spline.points.insert(i, at);
        }
        PointEdit::Move(i, at) => {
            if i >= n {
                return Err(index_error(i, n - 1));
            }
            spline.points[i] = checked(at)?;
        }
        PointEdit::Remove(i) => {
            if i >= n {
                return Err(index_error(i, n - 1));
            }
            spline.points.remove(i);
        }
    }
    spline
        .validate()
        .map_err(|e| Diagnostic::error("invalid_path", points_path.clone(), format!("{e:#}")))?;
    Ok(vec![Edit::Core(CoreEdit::SetField {
        target: Target::Entity { id: id.to_owned() },
        field: "/components/path.spline/points".into(),
        value: json!(spline.points),
    })])
}

/// Options of `add_nav_agent` (`None` = the `nav.agent` default).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavOptions {
    pub speed: Option<f32>,
    pub radius: Option<f32>,
}

/// The effective `nav.agent` of an entity (prefab-inherited + authored).
fn nav_agent_of(world: &World, id: &str) -> Result<crate::navigation::NavAgent> {
    let entity = world
        .entity(id)
        .ok_or_else(|| not_found(entity_path(id), format!("entity '{id}' not found")))?;
    let value = world
        .resolved_components(entity)
        .remove("nav.agent")
        .ok_or_else(|| {
            Diagnostic::error(
                "not_a_nav_agent",
                entity_path(id),
                format!("entity '{id}' has no nav.agent component"),
            )
        })?;
    serde_json::from_value(value).map_err(|e| {
        Diagnostic::error(
            "invalid_nav",
            format!("{}/components/nav.agent", entity_path(id)),
            format!("{e}"),
        )
    })
}

/// Object ▸ Add Nav Agent: puts a `nav.agent` (defaults plus `options`, no target) on each
/// entity, replacing an existing one. Adds the `nav` capability and the `nav.steer` system
/// when missing. Entities that already have another mover fail when the batch commits
/// (`nav_conflict`).
pub fn add_nav_agent(world: &World, ids: &[String], options: &NavOptions) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the objects that should become navigation agents",
        ));
    }
    let mut agent = crate::navigation::NavAgent::new();
    agent.speed = options.speed.unwrap_or(agent.speed);
    agent.radius = options.radius.unwrap_or(agent.radius);
    agent.validate().map_err(|e| {
        Diagnostic::error("invalid_value", "/components/nav.agent", format!("{e:#}"))
    })?;
    let mut edits = enable_system(world, Some("nav"), Some("nav.steer"));
    let mut seen = BTreeSet::new();
    for id in ids {
        require_entity(world, id)?;
        if seen.insert(id) {
            edits.push(Edit::Core(CoreEdit::SetComponent {
                entity: id.clone(),
                component: "nav.agent".into(),
                value: json!(agent),
            }));
        }
    }
    Ok(edits)
}

/// Nav ▸ Set Target (editor: selected object, or a click on the ground; ACL `nav SEL target`):
/// writes `nav.agent/target` on each agent; `None` clears it. The target entity must exist
/// and must not be the agent itself; the resulting agent must stay valid (a point target
/// with `follow_target` on is refused).
pub fn set_nav_target(
    world: &World,
    ids: &[String],
    target: Option<&crate::navigation::NavTarget>,
) -> Result<Vec<Edit>> {
    use crate::navigation::NavTarget;
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the navigation agents to send",
        ));
    }
    if let Some(t) = target {
        t.validate()
            .map_err(|e| Diagnostic::error("invalid_value", "/target", format!("{e:#}")))?;
        if let NavTarget::Entity(goal) = t {
            require_entity(world, goal)?;
        }
    }
    let mut edits = vec![];
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let mut agent = nav_agent_of(world, id)?;
        let at = format!("{}/components/nav.agent/target", entity_path(id));
        if matches!(target, Some(NavTarget::Entity(goal)) if goal == id) {
            return Err(Diagnostic::error(
                "invalid_value",
                at,
                format!("'{id}' cannot walk to itself"),
            ));
        }
        agent.target = target.cloned();
        agent
            .validate()
            .map_err(|e| Diagnostic::error("invalid_value", at, format!("{e:#}")))?;
        edits.push(Edit::Core(CoreEdit::SetField {
            target: Target::Entity { id: id.clone() },
            field: "/components/nav.agent/target".into(),
            value: target.map_or(Value::Null, |t| json!(t)),
        }));
    }
    Ok(edits)
}

/// Options of Object ▸ Make Interactable / ACL `interact SEL`; `None` keeps the defaults
/// (event `interacted`, action `interact`, range 2 m, default prompt).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InteractOptions {
    pub event: Option<String>,
    pub prompt: Option<String>,
    pub range: Option<f32>,
    pub action: Option<String>,
}

/// Object ▸ Make Interactable: puts an `interact.target` on each entity (replacing an existing
/// one) and makes it work in one revision: adds the `interact` capability and the
/// `interact.focus` system, declares the event in `behavior.program` (text fields
/// `interactor` and `target`; creating the program and `behavior.execute` when missing) and,
/// when `input.bindings` has no such button, binds the action to `KeyE` (refused when KeyE
/// is already taken: bind a key yourself first).
pub fn make_interactable(
    world: &World,
    ids: &[String],
    options: &InteractOptions,
) -> Result<Vec<Edit>> {
    use crate::interact::{InteractTarget, DEFAULT_EVENT};
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the objects that should become interactable",
        ));
    }
    let mut target = InteractTarget {
        event: DEFAULT_EVENT.into(),
        ..InteractTarget::default()
    };
    if let Some(event) = &options.event {
        target.event = event.clone();
    }
    if let Some(prompt) = &options.prompt {
        target.prompt = prompt.clone();
    }
    if let Some(range) = options.range {
        target.range = range;
    }
    if let Some(action) = &options.action {
        target.action = action.clone();
    }
    target.validate().map_err(|e| {
        Diagnostic::error(
            "invalid_value",
            "/components/interact.target",
            format!("{e:#}"),
        )
    })?;
    // One /capabilities and one /systems edit (two enable_system calls would each start from
    // the original lists and the second would drop the first's addition).
    let (mut capabilities, mut systems) = (world.capabilities.clone(), world.systems.clone());
    let mut needs = |capability: &str, system: &str| {
        capabilities.entry(capability.into()).or_insert(1);
        if !systems.iter().any(|s| s == system) {
            systems.push(system.into());
        }
    };
    needs("interact", "interact.focus");
    if !target.event.is_empty() {
        needs("behavior", "behavior.execute");
    }
    let mut edits = vec![];
    if capabilities != world.capabilities {
        edits.push(Edit::set("/capabilities", json!(capabilities)));
    }
    if systems != world.systems {
        edits.push(Edit::set("/systems", json!(systems)));
    }
    if !target.event.is_empty() {
        let fields = json!({"fields":{"interactor":"text","target":"text"}});
        match world.resources.get("behavior.program") {
            Some(program) => {
                let declared = program
                    .pointer(&format!("/events/{}", escape(&target.event)))
                    .is_some();
                if !declared {
                    let mut program = program.clone();
                    let events = program
                        .as_object_mut()
                        .map(|p| p.entry("events").or_insert_with(|| json!({})))
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| {
                            Diagnostic::error(
                                "invalid_value",
                                "/resources/behavior.program/events",
                                "behavior.program events must be an object",
                            )
                        })?;
                    events.insert(target.event.clone(), fields);
                    edits.push(Edit::Core(CoreEdit::SetResource {
                        id: "behavior.program".into(),
                        value: program,
                    }));
                }
            }
            None => edits.push(Edit::Core(CoreEdit::SetResource {
                id: "behavior.program".into(),
                value: json!({"events":{target.event.clone(): fields}}),
            })),
        }
    }
    if !target.action.is_empty() {
        let raw = world.resources.get("input.bindings");
        let bindings: InputBindings = raw
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| {
                Diagnostic::error("invalid_value", "/resources/input.bindings", format!("{e}"))
            })?
            .unwrap_or_default();
        if !bindings.buttons.contains_key(&target.action) {
            let taken = bindings.buttons.values().any(|k| k == "KeyE")
                || bindings
                    .axes
                    .values()
                    .any(|a| a.positive == "KeyE" || a.negative == "KeyE");
            if taken {
                return Err(Diagnostic::error(
                    "missing_binding",
                    format!("/resources/input.bindings/buttons/{}", escape(&target.action)),
                    format!(
                        "input.bindings has no button {0} and KeyE is already bound; bind one first (set resources[id=input.bindings] buttons/{0} \"KeyF\") or pick an existing button with action=",
                        target.action
                    ),
                ));
            }
            let mut value = raw.cloned().unwrap_or_else(|| json!({}));
            let buttons = value
                .as_object_mut()
                .map(|b| b.entry("buttons").or_insert_with(|| json!({})))
                .and_then(Value::as_object_mut)
                .expect("input.bindings parsed as an object with a buttons map");
            buttons.insert(target.action.clone(), json!("KeyE"));
            edits.push(Edit::Core(CoreEdit::SetResource {
                id: "input.bindings".into(),
                value,
            }));
        }
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        require_entity(world, id)?;
        if seen.insert(id) {
            edits.push(Edit::Core(CoreEdit::SetComponent {
                entity: id.clone(),
                component: "interact.target".into(),
                value: json!(target),
            }));
        }
    }
    Ok(edits)
}

/// Options of Object ▸ Add Audio Source / ACL `audio SEL source`; `None` = the `audio.source`
/// default (clip: the library's first clip).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioSourceOptions {
    pub clip: Option<String>,
    pub volume: Option<f32>,
    pub looping: Option<bool>,
    pub autoplay: Option<bool>,
    pub spatial: Option<bool>,
    pub min_distance: Option<f32>,
    pub max_distance: Option<f32>,
    pub rolloff: Option<crate::audio::Rolloff>,
}

/// The authored `audio.library` (empty when the world has none).
fn audio_library(world: &World) -> Result<crate::audio::AudioLibrary> {
    world
        .resources
        .get("audio.library")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|e| Diagnostic::error("invalid_value", "/resources/audio.library", format!("{e}")))
        .map(Option::unwrap_or_default)
}

fn missing_clip(library: &crate::audio::AudioLibrary, clip: &str) -> anyhow::Error {
    let known: Vec<&str> = library.clips.keys().map(String::as_str).collect();
    Diagnostic::error(
        "missing_clip",
        format!("/resources/audio.library/clips/{}", escape(clip)),
        format!(
            "audio.library has no clip {clip} (clips: {}); add one with `audio clip {clip} PRESET` ({})",
            if known.is_empty() {
                "none".to_owned()
            } else {
                known.join(", ")
            },
            crate::audio::CLIP_PRESETS.join("|")
        ),
    )
}

/// `base` if no library clip uses it, else `base2`, `base3`, ...
pub fn free_audio_clip_id(world: &World, base: &str) -> String {
    let library = audio_library(world).unwrap_or_default();
    if !library.clips.contains_key(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}{n}"))
        .find(|id| !library.clips.contains_key(id))
        .unwrap()
}

/// Adds clip `id` made from preset `preset` (`audio::CLIP_PRESETS`) to `audio.library`
/// (created when missing, with the `audio` capability). An existing clip ID is refused.
pub fn add_audio_clip(world: &World, id: &str, preset: &str) -> Result<Vec<Edit>> {
    let path = format!("/resources/audio.library/clips/{}", escape(id));
    check_new_id(path.clone(), id)?;
    let clip = crate::audio::AudioClip::preset(preset).ok_or_else(|| {
        Diagnostic::error(
            "unknown_preset",
            "/preset",
            format!(
                "unknown clip preset '{preset}' (use {})",
                crate::audio::CLIP_PRESETS.join(", ")
            ),
        )
    })?;
    let mut library = audio_library(world)?;
    if library.clips.contains_key(id) {
        return Err(Diagnostic::error(
            "id_taken",
            path,
            format!("audio.library already has a clip {id}"),
        ));
    }
    library.clips.insert(id.to_owned(), clip.clone());
    library.validate().map_err(|e| {
        Diagnostic::error("audio_budget", "/resources/audio.library", format!("{e:#}"))
    })?;
    // Insert into the authored JSON so the other cues and clips keep their exact text.
    let mut value = world
        .resources
        .get("audio.library")
        .cloned()
        .unwrap_or_else(|| json!({}));
    value
        .as_object_mut()
        .map(|l| l.entry("clips").or_insert_with(|| json!({})))
        .and_then(Value::as_object_mut)
        .expect("audio.library parsed as an object with a clips map")
        .insert(id.to_owned(), json!(clip));
    let mut edits = enable_system(world, Some("audio"), None);
    edits.push(Edit::Core(CoreEdit::SetResource {
        id: "audio.library".into(),
        value,
    }));
    Ok(edits)
}

/// Object ▸ Add Audio Source: puts an `audio.source` (defaults plus `options`) on each entity,
/// replacing an existing one. The clip must exist in `audio.library` (`missing_clip`); without
/// a clip option the library's first clip is used, and a world without clips gets the `tone`
/// preset as clip `tone`. Adds the `audio` capability and the `audio.spatial` system when
/// missing.
pub fn add_audio_source(
    world: &World,
    ids: &[String],
    options: &AudioSourceOptions,
) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the objects that should play a sound",
        ));
    }
    let library = audio_library(world)?;
    let mut clip_edits = vec![];
    let clip = match &options.clip {
        Some(clip) if library.clips.contains_key(clip) => clip.clone(),
        Some(clip) => return Err(missing_clip(&library, clip)),
        None => match library.clips.keys().next() {
            Some(first) => first.clone(),
            None => {
                clip_edits = add_audio_clip(world, "tone", "tone")?;
                "tone".to_owned()
            }
        },
    };
    let mut source = crate::audio::AudioSource::new(&clip);
    source.volume = options.volume.unwrap_or(source.volume);
    source.looping = options.looping.unwrap_or(source.looping);
    source.autoplay = options.autoplay.unwrap_or(source.autoplay);
    source.spatial = options.spatial.unwrap_or(source.spatial);
    source.min_distance = options.min_distance.unwrap_or(source.min_distance);
    source.max_distance = options.max_distance.unwrap_or(source.max_distance);
    source.rolloff = options.rolloff.unwrap_or(source.rolloff);
    source.validate().map_err(|e| {
        Diagnostic::error(
            "invalid_value",
            "/components/audio.source",
            format!("{e:#}"),
        )
    })?;
    // One /capabilities edit: add_audio_clip may already add the capability.
    let mut edits = enable_system(
        world,
        clip_edits.is_empty().then_some("audio"),
        Some("audio.spatial"),
    );
    edits.splice(0..0, clip_edits);
    let mut seen = BTreeSet::new();
    for id in ids {
        require_entity(world, id)?;
        if seen.insert(id) {
            edits.push(Edit::Core(CoreEdit::SetComponent {
                entity: id.clone(),
                component: "audio.source".into(),
                value: json!(source),
            }));
        }
    }
    Ok(edits)
}

/// Inspector ▸ Audio source ▸ Clip: sets `audio.source/clip` on each entity (the same field
/// edit as ACL `set ID audio.source/clip "CLIP"`). The clip must exist in `audio.library`.
pub fn set_audio_clip(world: &World, ids: &[String], clip: &str) -> Result<Vec<Edit>> {
    let library = audio_library(world)?;
    if !library.clips.contains_key(clip) {
        return Err(missing_clip(&library, clip));
    }
    let mut edits = vec![];
    let mut seen = BTreeSet::new();
    for id in ids {
        require_entity(world, id)?;
        let entity = world.entity(id).unwrap();
        if !world
            .resolved_components(entity)
            .contains_key("audio.source")
        {
            return Err(Diagnostic::error(
                "not_an_audio_source",
                entity_path(id),
                format!("entity '{id}' has no audio.source component"),
            ));
        }
        if seen.insert(id) {
            edits.push(Edit::Core(CoreEdit::SetField {
                target: Target::Entity { id: id.clone() },
                field: "/components/audio.source/clip".into(),
                value: json!(clip),
            }));
        }
    }
    Ok(edits)
}

/// Object ▸ Make Audio Listener: puts a default `audio.listener` on `id` and sets `active`
/// false on every other active listener in the same batch (a world has one active listener).
/// Adds the `audio` capability and the `audio.spatial` system when missing.
pub fn add_audio_listener(world: &World, id: &str) -> Result<Vec<Edit>> {
    require_entity(world, id)?;
    let mut edits = enable_system(world, Some("audio"), Some("audio.spatial"));
    edits.push(Edit::Core(CoreEdit::SetComponent {
        entity: id.to_owned(),
        component: "audio.listener".into(),
        value: json!(crate::audio::AudioListener::default()),
    }));
    for other in world.entities.iter().filter(|e| e.id != id) {
        let active = world
            .resolved_components(other)
            .remove("audio.listener")
            .and_then(|v| serde_json::from_value::<crate::audio::AudioListener>(v).ok())
            .is_some_and(|l| l.active);
        if active {
            edits.push(Edit::Core(CoreEdit::SetField {
                target: Target::Entity {
                    id: other.id.clone(),
                },
                field: "/components/audio.listener/active".into(),
                value: json!(false),
            }));
        }
    }
    Ok(edits)
}

/// Clip options of `add_tween_clip` / `set_tween_clip`; `None` keeps the current value (or
/// the default when adding). `property` / `target` only apply when adding.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TweenClipOptions {
    /// First track's property (default `core.transform/position`).
    pub property: Option<String>,
    /// First track's target entity (default: the entity itself).
    pub target: Option<String>,
    /// `Some(None)` = automatic (the last key time).
    pub duration: Option<Option<f32>>,
    pub mode: Option<crate::tween::TweenMode>,
    pub speed: Option<f32>,
    pub autoplay: Option<bool>,
    /// `""` clears it.
    pub on_finish: Option<String>,
    /// A `behavior.program` event that plays this clip: appends a `tween.play` call to the
    /// event's `runtime.bindings` entry (declaring the event when missing). `""` = none.
    pub play_on: Option<String>,
}

/// One change to the keys of a tween track (Inspector timeline; ACL `tween ID key ...`).
#[derive(Clone, Debug, PartialEq)]
pub enum KeyEdit {
    /// A key at `time`; `value: None` records the property's current authored value. A key
    /// already at that time (within 0.1 ms) gets the new value instead.
    Add {
        time: f32,
        value: Option<Vec<f32>>,
        ease: Option<crate::tween::Ease>,
    },
    Move {
        index: usize,
        time: f32,
    },
    Remove(usize),
}

/// Keys closer than this are the same key (seconds).
const KEY_EPSILON: f32 = 1e-4;
/// Default property of the first track of a new clip.
pub const DEFAULT_TWEEN_PROPERTY: &str = "core.transform/position";

fn tween_path(id: &str, rest: &str) -> String {
    format!("{}/components/tween.clips{rest}", entity_path(id))
}

/// The `tween.clips` authored on entity `id` (`None` when it has none). Tweens inherited from
/// a prefab are edited on the prefab: an override cannot remove inherited clips or keys.
fn tween_of(world: &World, id: &str) -> Result<Option<crate::tween::TweenClips>> {
    require_entity(world, id)?;
    let entity = world.entity(id).unwrap();
    if entity
        .prefab
        .as_ref()
        .and_then(|p| world.prefabs.get(p))
        .is_some_and(|p| p.components.contains_key("tween.clips"))
    {
        return Err(Diagnostic::error(
            "inherited",
            tween_path(id, ""),
            format!("'{id}' inherits tween.clips from its prefab; edit the prefab (set prefabs[id=P] tween.clips/...)"),
        ));
    }
    entity
        .components
        .get("tween.clips")
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| Diagnostic::error("invalid_tween", tween_path(id, ""), format!("{e}")))
        })
        .transpose()
}

fn tween_clip<'a>(
    clips: &'a mut crate::tween::TweenClips,
    id: &str,
    name: &str,
) -> Result<&'a mut crate::tween::TweenClip> {
    let names = clips.clips.keys().cloned().collect::<Vec<_>>().join(", ");
    clips.clips.get_mut(name).ok_or_else(|| {
        not_found(
            tween_path(id, &format!("/clips/{}", escape(name))),
            format!("'{id}' has no tween clip {name} (clips: {names})"),
        )
    })
}

/// The recorded value of `property` on `target` ("" = `id`) for a new key.
fn recorded(world: &World, id: &str, target: &str, property: &str) -> Result<Vec<f32>> {
    let p = crate::tween::Property::parse(property)
        .map_err(|e| Diagnostic::error("invalid_value", "/property", format!("{e:#}")))?;
    let on = if target.is_empty() { id } else { target };
    crate::tween::authored_value(world, on, &p).map_err(|e| {
        Diagnostic::error(
            "invalid_tween",
            format!("{}/components", entity_path(on)),
            format!("cannot record {property}: {e:#}"),
        )
    })
}

/// Validates `clips` and writes them as the whole component (replacing the override).
fn write_tween(id: &str, clips: &crate::tween::TweenClips) -> Result<Edit> {
    clips
        .validate()
        .map_err(|e| Diagnostic::error("invalid_tween", tween_path(id, ""), format!("{e:#}")))?;
    Ok(Edit::Core(CoreEdit::SetComponent {
        entity: id.to_owned(),
        component: "tween.clips".into(),
        value: json!(clips),
    }))
}

/// Edits that make tweens run: the `tween` capability and `tween.animate`; with `behavior`
/// also `behavior.execute`, and `events` (name, declared fields) declared in
/// `behavior.program` when missing (the program is created when absent). One edit per list
/// and one program edit (see make_interactable).
fn enable_tweens(world: &World, behavior: bool, events: &[(&str, Value)]) -> Result<Vec<Edit>> {
    let (mut capabilities, mut systems) = (world.capabilities.clone(), world.systems.clone());
    let mut needs = |capability: &str, system: &str| {
        capabilities.entry(capability.into()).or_insert(1);
        if !systems.iter().any(|s| s == system) {
            systems.push(system.into());
        }
    };
    needs("tween", "tween.animate");
    if behavior || !events.is_empty() {
        needs("behavior", "behavior.execute");
    }
    let mut edits = vec![];
    if capabilities != world.capabilities {
        edits.push(Edit::set("/capabilities", json!(capabilities)));
    }
    if systems != world.systems {
        edits.push(Edit::set("/systems", json!(systems)));
    }
    let mut program = world
        .resources
        .get("behavior.program")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut declared = false;
    for (event, fields) in events {
        let declarations = program
            .as_object_mut()
            .map(|p| p.entry("events").or_insert_with(|| json!({})))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                Diagnostic::error(
                    "invalid_value",
                    "/resources/behavior.program/events",
                    "behavior.program events must be an object",
                )
            })?;
        if !declarations.contains_key(*event) {
            declarations.insert((*event).to_owned(), json!({ "fields": fields }));
            declared = true;
        }
    }
    if declared {
        edits.push(Edit::Core(CoreEdit::SetResource {
            id: "behavior.program".into(),
            value: program,
        }));
    }
    Ok(edits)
}

/// The behavior events a clip option set needs declared: `on_finish` (text fields `entity`
/// and `clip`) and `play_on` (no fields: any event may trigger a play).
fn tween_events(options: &TweenClipOptions) -> Vec<(&str, Value)> {
    let mut events = vec![];
    if let Some(e) = options.on_finish.as_deref().filter(|e| !e.is_empty()) {
        events.push((e, json!({"entity":"text","clip":"text"})));
    }
    if let Some(e) = options.play_on.as_deref().filter(|e| !e.is_empty()) {
        if !events.iter().any(|(n, _)| *n == e) {
            events.push((e, json!({})));
        }
    }
    events
}

/// `runtime.bindings` calls of `tween.play` for `(entity, clip)` pairs.
fn play_call(id: &str, clip: &str) -> Value {
    json!({"kind":"call","capability":"tween","action":"play","args":{"id":id,"clip":clip}})
}

/// The `runtime.bindings` edit that makes `event` play `clip` on each of `ids` (appending to
/// the event's binding, creating the resource when missing; existing identical calls are
/// kept once). `None` when nothing changes.
fn bind_plays(world: &World, ids: &[&String], clip: &str, event: &str) -> Result<Option<Edit>> {
    valid_id(event).map_err(|_| {
        Diagnostic::error(
            "invalid_value",
            "/play_on",
            format!("invalid event name '{event}'"),
        )
    })?;
    let mut bindings = world
        .resources
        .get("runtime.bindings")
        .cloned()
        .unwrap_or_else(|| json!({"events":[]}));
    let path = "/resources/runtime.bindings/events";
    let list = bindings
        .as_object_mut()
        .map(|b| b.entry("events").or_insert_with(|| json!([])))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            Diagnostic::error(
                "invalid_value",
                path,
                "runtime.bindings events must be an array",
            )
        })?;
    let at = match list.iter().position(|b| b["event"] == event) {
        Some(at) => at,
        None => {
            list.push(json!({"event":event,"calls":[]}));
            list.len() - 1
        }
    };
    let calls = list[at]["calls"].as_array_mut().ok_or_else(|| {
        Diagnostic::error("invalid_value", path, "binding calls must be an array")
    })?;
    let before = calls.len();
    for id in ids {
        let call = play_call(id, clip);
        if !calls.contains(&call) {
            calls.push(call);
        }
    }
    if calls.len() > 8 || list.len() > 128 {
        return Err(Diagnostic::error(
            "invalid_value",
            path,
            format!(
                "runtime.bindings allows 8 calls per event and 128 events; {event} would exceed it"
            ),
        ));
    }
    let changed = list[at]["calls"].as_array().map_or(0, Vec::len) != before;
    Ok(changed.then(|| {
        Edit::Core(CoreEdit::SetResource {
            id: "runtime.bindings".into(),
            value: bindings,
        })
    }))
}

/// Drops `runtime.bindings` calls that play `clip` on `id` (and bindings left without calls).
fn unbind_plays(world: &World, id: &str, clip: &str) -> Option<Edit> {
    let mut bindings = world.resources.get("runtime.bindings")?.clone();
    let call = play_call(id, clip);
    let list = bindings.get_mut("events")?.as_array_mut()?;
    let before = list.clone();
    for binding in list.iter_mut() {
        if let Some(calls) = binding.get_mut("calls").and_then(Value::as_array_mut) {
            calls.retain(|c| *c != call);
        }
    }
    list.retain(|b| b["calls"].as_array().is_some_and(|c| !c.is_empty()));
    (*list != before).then(|| {
        Edit::Core(CoreEdit::SetResource {
            id: "runtime.bindings".into(),
            value: bindings,
        })
    })
}
/// Applies the clip options of `options` (not property / target) to `clip`; autoplay on one
/// clip turns it off on the others (at most one autoplay clip per component).
fn apply_clip_options(
    clips: &mut crate::tween::TweenClips,
    name: &str,
    options: &TweenClipOptions,
) {
    if options.autoplay == Some(true) {
        for (other, clip) in clips.clips.iter_mut() {
            if other != name {
                clip.autoplay = false;
            }
        }
    }
    let clip = clips.clips.get_mut(name).expect("clip exists");
    if let Some(duration) = options.duration {
        clip.duration = duration;
    }
    if let Some(mode) = options.mode {
        clip.mode = mode;
    }
    if let Some(speed) = options.speed {
        clip.speed = speed;
    }
    if let Some(autoplay) = options.autoplay {
        clip.autoplay = autoplay;
    }
    if let Some(event) = &options.on_finish {
        clip.on_finish = event.clone();
    }
}

fn variable_property(property: &str) -> bool {
    property.starts_with("behavior.program/variables/")
}

/// Object ▸ Add Tween Clip / ACL `tween SEL clip NAME`: adds clip `name` to each entity's
/// `tween.clips` (creating the component). Its first track animates `options.property` of
/// `options.target` with two keys, at 0 and at the clip length (default 1 s), both recording
/// the current authored value, so the clip changes nothing until a key is moved or recorded.
/// Adds the `tween` capability and `tween.animate` (and `behavior.execute` plus the declared
/// `on_finish` event when needed) in the same batch.
pub fn add_tween_clip(
    world: &World,
    ids: &[String],
    name: &str,
    options: &TweenClipOptions,
) -> Result<Vec<Edit>> {
    use crate::tween::*;
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the objects that get the tween clip",
        ));
    }
    if valid_id(name).is_err() {
        return Err(Diagnostic::error(
            "invalid_value",
            "/clip",
            format!("invalid clip name '{name}' (letters, digits, . _ / -)"),
        ));
    }
    let property = options
        .property
        .clone()
        .unwrap_or_else(|| DEFAULT_TWEEN_PROPERTY.to_owned());
    let target = options.target.clone().unwrap_or_default();
    let length = options.duration.flatten().unwrap_or(DEFAULT_DURATION);
    let mut edits = enable_tweens(world, variable_property(&property), &tween_events(options))?;
    let mut seen = BTreeSet::new();
    let mut owners = vec![];
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        owners.push(id);
        let mut clips = tween_of(world, id)?.unwrap_or(TweenClips {
            clips: Default::default(),
            enabled: true,
        });
        if clips.clips.contains_key(name) {
            return Err(Diagnostic::error(
                "clip_exists",
                tween_path(id, &format!("/clips/{}", escape(name))),
                format!(
                    "'{id}' already has a tween clip {name}; use tween {id} clip {name} set ..."
                ),
            ));
        }
        let value = recorded(world, id, &target, &property)?;
        let key = |t: f32| TweenKey {
            t,
            v: KeyValue::from_slice(&value),
            ease: Ease::Linear,
        };
        clips.clips.insert(
            name.to_owned(),
            TweenClip {
                tracks: vec![TweenTrack {
                    target: target.clone(),
                    property: property.clone(),
                    keys: vec![key(0.0), key(length)],
                }],
                duration: None,
                mode: TweenMode::Once,
                speed: 1.0,
                autoplay: false,
                on_finish: String::new(),
            },
        );
        apply_clip_options(&mut clips, name, options);
        edits.push(write_tween(id, &clips)?);
    }
    if let Some(event) = options.play_on.as_deref().filter(|e| !e.is_empty()) {
        edits.extend(bind_plays(world, &owners, name, event)?);
    }
    Ok(edits)
}

/// Inspector timeline options / ACL `tween ID clip NAME set ...`: changes duration, mode,
/// speed, autoplay (turning it off on the other clips) and on_finish of an existing clip.
pub fn set_tween_clip(
    world: &World,
    id: &str,
    name: &str,
    options: &TweenClipOptions,
) -> Result<Vec<Edit>> {
    let mut clips = tween_of(world, id)?
        .ok_or_else(|| not_found(tween_path(id, ""), format!("'{id}' has no tween.clips")))?;
    tween_clip(&mut clips, id, name)?;
    apply_clip_options(&mut clips, name, options);
    let mut edits = enable_tweens(world, false, &tween_events(options))?;
    edits.push(write_tween(id, &clips)?);
    if let Some(event) = options.play_on.as_deref().filter(|e| !e.is_empty()) {
        edits.extend(bind_plays(world, &[&id.to_owned()], name, event)?);
    }
    Ok(edits)
}

/// Inspector ▸ Remove clip / ACL `tween ID clip NAME remove`. The last clip removes the
/// component; `runtime.bindings` calls that play the clip are removed with it.
pub fn remove_tween_clip(world: &World, id: &str, name: &str) -> Result<Vec<Edit>> {
    let mut clips = tween_of(world, id)?
        .ok_or_else(|| not_found(tween_path(id, ""), format!("'{id}' has no tween.clips")))?;
    tween_clip(&mut clips, id, name)?;
    clips.clips.remove(name);
    let mut edits = if clips.clips.is_empty() {
        remove_component(world, &[id.to_owned()], "tween.clips")?
    } else {
        vec![write_tween(id, &clips)?]
    };
    edits.extend(unbind_plays(world, id, name));
    Ok(edits)
}

/// Inspector timeline ▸ Add track / ACL `tween ID track CLIP add PROPERTY [target=ID]`: a track
/// with keys at 0 and at the clip length that record the current authored value.
pub fn add_tween_track(
    world: &World,
    id: &str,
    clip: &str,
    property: &str,
    target: &str,
) -> Result<Vec<Edit>> {
    use crate::tween::*;
    let mut clips = tween_of(world, id)?
        .ok_or_else(|| not_found(tween_path(id, ""), format!("'{id}' has no tween.clips")))?;
    let value = recorded(world, id, target, property)?;
    let c = tween_clip(&mut clips, id, clip)?;
    let length = c.length();
    let key = |t: f32| TweenKey {
        t,
        v: KeyValue::from_slice(&value),
        ease: Ease::Linear,
    };
    c.tracks.push(TweenTrack {
        target: target.to_owned(),
        property: property.to_owned(),
        keys: vec![key(0.0), key(length)],
    });
    let mut edits = enable_tweens(world, variable_property(property), &[])?;
    edits.push(write_tween(id, &clips)?);
    Ok(edits)
}

/// Inspector timeline ▸ Remove track / ACL `tween ID track CLIP remove I`. A clip keeps at
/// least one track (remove the clip instead).
pub fn remove_tween_track(world: &World, id: &str, clip: &str, index: usize) -> Result<Vec<Edit>> {
    let mut clips = tween_of(world, id)?
        .ok_or_else(|| not_found(tween_path(id, ""), format!("'{id}' has no tween.clips")))?;
    let c = tween_clip(&mut clips, id, clip)?;
    let path = tween_path(id, &format!("/clips/{}/tracks/{index}", escape(clip)));
    if index >= c.tracks.len() {
        return Err(Diagnostic::error(
            "invalid_value",
            path,
            format!(
                "track index {index} is out of range 0..{}",
                c.tracks.len() - 1
            ),
        ));
    }
    if c.tracks.len() == 1 {
        return Err(Diagnostic::error(
            "invalid_value",
            path,
            format!("{clip} needs at least one track; remove the clip instead (tween {id} clip {clip} remove)"),
        ));
    }
    c.tracks.remove(index);
    Ok(vec![write_tween(id, &clips)?])
}

/// Inspector timeline keys (record at the playhead, drag, delete) / ACL `tween ID key CLIP
/// TRACK add|move|remove`. Keys stay sorted by time; the result must be a valid clip.
pub fn edit_tween_key(
    world: &World,
    id: &str,
    clip: &str,
    track: usize,
    edit: &KeyEdit,
) -> Result<Vec<Edit>> {
    use crate::tween::*;
    let mut clips = tween_of(world, id)?
        .ok_or_else(|| not_found(tween_path(id, ""), format!("'{id}' has no tween.clips")))?;
    let c = tween_clip(&mut clips, id, clip)?;
    let path = tween_path(id, &format!("/clips/{}/tracks/{track}", escape(clip)));
    let Some(t) = c.tracks.get_mut(track) else {
        return Err(Diagnostic::error(
            "invalid_value",
            path,
            format!(
                "track index {track} is out of range (clip {clip} has {} tracks)",
                c.tracks.len()
            ),
        ));
    };
    let keys_path = format!("{path}/keys");
    let bad = |message: String| Diagnostic::error("invalid_value", keys_path.clone(), message);
    let checked_time = |time: f32| -> Result<f32> {
        if time.is_finite() && (0.0..=MAX_DURATION).contains(&time) {
            Ok(time)
        } else {
            Err(bad(format!("key time must be 0..{MAX_DURATION} seconds")))
        }
    };
    let index_error =
        |i: usize, n: usize| bad(format!("key index {i} is out of range 0..{}", n - 1));
    match edit {
        KeyEdit::Add { time, value, ease } => {
            let time = checked_time(*time)?;
            let value = match value {
                Some(v) => v.clone(),
                None => recorded(world, id, &t.target, &t.property)?,
            };
            let v = KeyValue::from_slice(&value);
            if let Some(k) = t.keys.iter_mut().find(|k| (k.t - time).abs() < KEY_EPSILON) {
                k.v = v;
                if let Some(e) = ease {
                    k.ease = *e;
                }
            } else {
                let at = t.keys.partition_point(|k| k.t < time);
                t.keys.insert(
                    at,
                    TweenKey {
                        t: time,
                        v,
                        ease: ease.unwrap_or_default(),
                    },
                );
            }
        }
        KeyEdit::Move { index, time } => {
            let time = checked_time(*time)?;
            if *index >= t.keys.len() {
                return Err(index_error(*index, t.keys.len()));
            }
            let mut key = t.keys.remove(*index);
            if let Some(other) = t.keys.iter().position(|k| (k.t - time).abs() < KEY_EPSILON) {
                return Err(bad(format!(
                    "another key is already at {time} s (remove key {} first)",
                    if other >= *index { other + 1 } else { other }
                )));
            }
            key.t = time;
            let at = t.keys.partition_point(|k| k.t < time);
            t.keys.insert(at, key);
        }
        KeyEdit::Remove(index) => {
            if *index >= t.keys.len() {
                return Err(index_error(*index, t.keys.len()));
            }
            if t.keys.len() == 1 {
                return Err(bad("a track keeps at least one key".into()));
            }
            t.keys.remove(*index);
        }
    }
    Ok(vec![write_tween(id, &clips)?])
}

/// The first `clip`, `clip2`, `clip3`... no entity in `ids` has yet (editor default name).
pub fn free_clip_name(world: &World, ids: &[String]) -> String {
    let taken = |name: &str| {
        ids.iter().any(|id| {
            world
                .entity(id)
                .and_then(|e| e.components.get("tween.clips"))
                .and_then(|v| v.pointer(&format!("/clips/{}", escape(name))))
                .is_some()
        })
    };
    (1..)
        .map(|n| {
            if n == 1 {
                "clip".to_owned()
            } else {
                format!("clip{n}")
            }
        })
        .find(|name| !taken(name))
        .unwrap()
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
