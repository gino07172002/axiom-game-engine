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

/// Presets offered by the editor's "+ Object" menu.
pub const PRESETS: [&str; 3] = ["empty", "cube", "camera"];

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

/// Creates an object from an editor preset: `empty`, `cube` (one default mesh part) or
/// `camera` (perspective; active only when no other camera is active). Cube and camera add
/// the `render` capability when the world lacks it.
pub fn create_object(world: &World, preset: &str, place: &Placement) -> Result<Created> {
    if !PRESETS.contains(&preset) {
        let hint = if preset == "light" {
            "; this engine has no light component (use a cube with emission, or a vfx light)"
        } else {
            ""
        };
        return Err(Diagnostic::error(
            "unknown_preset",
            "/preset",
            format!("unknown preset '{preset}' (use empty, cube, camera){hint}"),
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
/// (the source part is cut to 24 characters). The batch keeps the given order.
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
    for source in ids {
        require_entity(world, source)?;
        let base = format!("{}/copy", source.chars().take(24).collect::<String>());
        let id = unique_id(world, &base, &taken);
        taken.insert(id.clone());
        created.ids.push(id.clone());
        created.edits.push(Edit::Core(CoreEdit::DuplicateEntity {
            source: source.clone(),
            id,
        }));
    }
    Ok(created)
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
    }
}
