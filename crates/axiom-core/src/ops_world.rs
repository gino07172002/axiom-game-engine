//! World-level and generic editor operations (re-exported from [`crate::ops`]): entity tags,
//! capabilities, systems, resources, world id/title, adding components with what they need,
//! array element edits, event declarations, Euler rotations and multi-object field edits.
//! Pure like the rest of `ops`: `fn(&World, ..) -> Result<Vec<Edit>>`, committed by the caller.
use crate::gameplay::ScalarType;
use crate::local::{escape, Selection, Target};
use crate::*;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Tags per entity and bytes per tag (the compile-time budget, `World::compile`).
pub const MAX_TAGS: usize = 32;
pub const MAX_TAG_BYTES: usize = 64;

fn entity_path(id: &str) -> String {
    format!("/entities/@{}", escape(id))
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}

fn require_entity<'a>(world: &'a World, id: &str) -> Result<&'a Entity> {
    world.entity(id).ok_or_else(|| {
        Diagnostic::error(
            "not_found",
            entity_path(id),
            format!("entity '{id}' not found"),
        )
    })
}

/// Selection order without repeats.
fn unique(ids: &[String]) -> Vec<&String> {
    let mut seen = BTreeSet::new();
    ids.iter().filter(|id| seen.insert(*id)).collect()
}

fn check_tag(tag: &str) -> Result<()> {
    if tag.is_empty()
        || tag.len() > MAX_TAG_BYTES
        || tag
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ',' | '[' | ']' | '"'))
    {
        return Err(Diagnostic::error(
            "invalid_tag",
            "/tags",
            format!(
                "invalid tag '{tag}' (1..{MAX_TAG_BYTES} bytes, no spaces, commas, brackets or quotes)"
            ),
        ));
    }
    Ok(())
}

/// Inspector tag chips / ACL `tag SEL +a -b`: removes `remove` and appends the missing `add`
/// tags (in the given order) on every entity, keeping the other tags and their order.
/// Entities whose tags do not change get no edit, so the batch may be empty.
pub fn edit_tags(
    world: &World,
    ids: &[String],
    add: &[String],
    remove: &[String],
) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the objects to tag",
        ));
    }
    if add.is_empty() && remove.is_empty() {
        return Err(invalid("/tags", "name tags to add (+tag) or remove (-tag)"));
    }
    for tag in add.iter().chain(remove) {
        check_tag(tag)?;
    }
    if let Some(both) = add.iter().find(|t| remove.contains(t)) {
        return Err(invalid(
            "/tags",
            format!("tag '{both}' is both added and removed"),
        ));
    }
    let mut edits = vec![];
    for id in unique(ids) {
        let entity = require_entity(world, id)?;
        let mut tags: Vec<String> = entity
            .tags
            .iter()
            .filter(|t| !remove.contains(t))
            .cloned()
            .collect();
        for tag in add {
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
        if tags.len() > MAX_TAGS {
            return Err(invalid(
                format!("{}/tags", entity_path(id)),
                format!(
                    "'{id}' would have {} tags; the limit is {MAX_TAGS}",
                    tags.len()
                ),
            ));
        }
        if tags != entity.tags {
            edits.push(Edit::Core(CoreEdit::SetField {
                target: Target::Entity { id: id.clone() },
                field: "/tags".into(),
                value: json!(tags),
            }));
        }
    }
    Ok(edits)
}

/// Tags the world already uses, for autocomplete: entity tags plus the tags named by
/// `filter.tags` in authored components (trigger zones, interactables), sorted.
pub fn known_tags(world: &World) -> BTreeSet<String> {
    let mut tags: BTreeSet<String> = world
        .entities
        .iter()
        .flat_map(|e| e.tags.iter().cloned())
        .collect();
    let components = world
        .entities
        .iter()
        .flat_map(|e| e.components.values())
        .chain(world.prefabs.values().flat_map(|p| p.components.values()));
    for value in components {
        if let Some(list) = value.pointer("/filter/tags").and_then(Value::as_array) {
            tags.extend(list.iter().filter_map(Value::as_str).map(str::to_owned));
        }
    }
    tags
}

/// Capability and systems a world needs for one component or resource type: the type's
/// capability, plus every registered system of that capability that reads the type (and,
/// transitively, the systems those must run after, with their capabilities).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Requirements {
    pub capabilities: Vec<String>,
    pub systems: Vec<String>,
}

fn type_capability<'a>(registry: &'a Registry, id: &str) -> Option<&'a str> {
    registry
        .components
        .get(id)
        .or_else(|| registry.resources.get(id))
        .map(|d| d.capability.as_str())
}

/// Adds `system`, its capability and (first) everything it must run after.
fn need_system(registry: &Registry, system: &str, out: &mut Requirements, depth: usize) {
    let Some(d) = registry.systems.get(system) else {
        return;
    };
    if out.systems.iter().any(|s| s == system) || depth > 32 {
        return;
    }
    if !out.capabilities.contains(&d.capability) {
        out.capabilities.push(d.capability.clone());
    }
    for after in &d.after {
        need_system(registry, after, out, depth + 1);
    }
    if !out.systems.iter().any(|s| s == system) {
        out.systems.push(system.to_owned());
    }
}

/// What adding a component or resource `id` needs (see [`Requirements`]). Types defined by
/// the world's own schemas need nothing.
pub fn requirements(registry: &Registry, id: &str) -> Requirements {
    let mut out = Requirements::default();
    let Some(capability) = type_capability(registry, id) else {
        return out;
    };
    out.capabilities.push(capability.to_owned());
    let mut readers: Vec<&SystemDescriptor> = registry
        .systems
        .values()
        .filter(|s| s.capability == capability && s.reads.iter().any(|r| r == id))
        .collect();
    readers.sort_by(|a, b| (a.phase, &a.id).cmp(&(b.phase, &b.id)));
    for system in readers {
        need_system(registry, &system.id, &mut out, 0);
    }
    out
}

/// At most one `/capabilities` and one `/systems` edit that add what `needs` lists and the
/// world lacks (systems are appended in the listed order).
pub(crate) fn requirement_edits(
    world: &World,
    registry: &Registry,
    needs: &Requirements,
) -> Vec<Edit> {
    let mut capabilities = world.capabilities.clone();
    for c in &needs.capabilities {
        let version = registry.capabilities.get(c).copied().unwrap_or(1);
        capabilities.entry(c.clone()).or_insert(version);
    }
    let mut systems = world.systems.clone();
    for s in &needs.systems {
        if !systems.contains(s) {
            systems.push(s.clone());
        }
    }
    let mut edits = vec![];
    if capabilities != world.capabilities {
        edits.push(Edit::set("/capabilities", json!(capabilities)));
    }
    if systems != world.systems {
        edits.push(Edit::set("/systems", json!(systems)));
    }
    edits
}

fn list(names: impl Iterator<Item = impl AsRef<str>>) -> String {
    let names: Vec<String> = names.map(|n| n.as_ref().to_owned()).collect();
    if names.len() > 8 {
        format!("{} … (+{})", names[..8].join(", "), names.len() - 8)
    } else {
        names.join(", ")
    }
}

/// World ▸ Capabilities ▸ Add / ACL `capability add ID`. Present already = empty batch.
pub fn add_capability(world: &World, registry: &Registry, id: &str) -> Result<Vec<Edit>> {
    let Some(&version) = registry.capabilities.get(id) else {
        return Err(Diagnostic::error(
            "unknown_capability",
            format!("/capabilities/{}", escape(id)),
            format!(
                "capability '{id}' is not installed (installed: {})",
                list(registry.capabilities.keys())
            ),
        ));
    };
    if world.capabilities.contains_key(id) {
        return Ok(vec![]);
    }
    let mut capabilities = world.capabilities.clone();
    capabilities.insert(id.to_owned(), version);
    Ok(vec![Edit::set("/capabilities", json!(capabilities))])
}

/// What in `world` uses capability `id`: components, resources and systems of that capability.
pub fn capability_users(world: &World, registry: &Registry, id: &str) -> Vec<String> {
    let of = |ty: &str| type_capability(registry, ty) == Some(id);
    let mut users = vec![];
    for e in &world.entities {
        for c in e.components.keys().filter(|c| of(c)) {
            users.push(format!("{} {c}", e.id));
        }
    }
    for (p, prefab) in &world.prefabs {
        for c in prefab.components.keys().filter(|c| of(c)) {
            users.push(format!("prefab {p} {c}"));
        }
    }
    for r in world.resources.keys().filter(|r| of(r)) {
        users.push(format!("resource {r}"));
    }
    for s in &world.systems {
        if registry.systems.get(s).is_some_and(|d| d.capability == id) {
            users.push(format!("system {s}"));
        }
    }
    users
}

/// World ▸ Capabilities ▸ Remove / ACL `capability remove ID`. Refused for `core` and while
/// any component, resource or system of the capability remains (`capability_in_use`).
pub fn remove_capability(world: &World, registry: &Registry, id: &str) -> Result<Vec<Edit>> {
    let path = format!("/capabilities/{}", escape(id));
    if id == "core" {
        return Err(invalid(path, "the core capability is required"));
    }
    if !world.capabilities.contains_key(id) {
        return Err(Diagnostic::error(
            "not_found",
            path,
            format!("the world has no capability '{id}'"),
        ));
    }
    let users = capability_users(world, registry, id);
    if !users.is_empty() {
        return Err(Diagnostic::error(
            "capability_in_use",
            path,
            format!(
                "capability '{id}' is still used by {}: {}; remove those first",
                users.len(),
                list(users.iter())
            ),
        ));
    }
    let mut capabilities = world.capabilities.clone();
    capabilities.remove(id);
    Ok(vec![Edit::set("/capabilities", json!(capabilities))])
}

/// World ▸ Systems ▸ Add / ACL `system add ID`: appends the system, adding its capability and
/// the systems it must run after when missing. Present already = empty batch.
pub fn add_system(world: &World, registry: &Registry, id: &str) -> Result<Vec<Edit>> {
    if !registry.systems.contains_key(id) {
        return Err(Diagnostic::error(
            "unknown_system",
            "/systems",
            format!(
                "system '{id}' is not registered (registered: {})",
                list(registry.systems.keys())
            ),
        ));
    }
    let mut needs = Requirements::default();
    need_system(registry, id, &mut needs, 0);
    Ok(requirement_edits(world, registry, &needs))
}

/// World ▸ Systems ▸ Remove / ACL `system remove ID`. Refused while another scheduled system
/// must run after it (`system_in_use`). Components that need the system are reported by the
/// commit's validation (e.g. "trigger.zone needs the trigger.detect system").
pub fn remove_system(world: &World, registry: &Registry, id: &str) -> Result<Vec<Edit>> {
    if !world.systems.iter().any(|s| s == id) {
        return Err(Diagnostic::error(
            "not_found",
            "/systems",
            format!("the world does not schedule system '{id}'"),
        ));
    }
    let dependents: Vec<&String> = world
        .systems
        .iter()
        .filter(|s| {
            registry
                .systems
                .get(s.as_str())
                .is_some_and(|d| d.after.iter().any(|a| a == id))
        })
        .collect();
    if !dependents.is_empty() {
        return Err(Diagnostic::error(
            "system_in_use",
            "/systems",
            format!(
                "{} must run after '{id}'; remove {} first",
                list(dependents.iter()),
                if dependents.len() == 1 { "it" } else { "them" }
            ),
        ));
    }
    let systems: Vec<&String> = world.systems.iter().filter(|s| *s != id).collect();
    Ok(vec![Edit::set("/systems", json!(systems))])
}

/// `T` deserialized from `{}` and serialized back: every field at its serde default.
fn typed<T: serde::de::DeserializeOwned + serde::Serialize>() -> Option<Value> {
    serde_json::from_value::<T>(json!({}))
        .ok()
        .map(|t| json!(t))
}

/// The value a new component or resource `id` starts with: the editor preset for core types
/// (the same values the "+ Object" presets use), else the type's serde defaults written out,
/// else `{}`, whichever the registered validator accepts first. `None` when nothing valid
/// exists without more information (e.g. `path.follow` needs a path).
pub fn default_value(world: &World, registry: &Registry, id: &str) -> Option<Value> {
    let preset = match id {
        "core.transform" => typed::<Transform>(),
        "core.health" => Some(json!({"value":100.0,"max":100.0})),
        "core.motion" => typed::<Motion>(),
        "render.mesh" => Some(json!(RenderMesh {
            parts: vec![MeshPart {
                transform: Transform::default(),
                color: [0.20, 0.65, 0.73],
                emission: 0.0,
                grid: false,
            }],
        })),
        "render.camera" => {
            let mut camera = Camera::perspective();
            camera.active = !world.entities.iter().any(|e| {
                world
                    .resolved_components(e)
                    .get("render.camera")
                    .is_some_and(|c| c["active"] == true)
            });
            Some(json!(camera))
        }
        "render.light" => crate::ops::preset_light("light.point").map(|l| json!(l)),
        "render.surface" => typed::<crate::light::RenderSurface>(),
        "trigger.zone" => crate::ops::preset_trigger("trigger.box").map(|z| json!(z)),
        "path.spline" => Some(json!(crate::path::PathSpline::preset())),
        "nav.blocker" => Some(json!({"half":[0.5,0.5],"offset":[0.0,0.0]})),
        "nav.agent" => Some(json!(crate::navigation::NavAgent::default())),
        "interact.target" => Some(json!(crate::interact::InteractTarget::default())),
        "audio.listener" => Some(json!(crate::audio::AudioListener::default())),
        "physics.collider" => Some(json!(Collider::new(ColliderShape::Box {
            half_extents: [0.5; 3]
        }))),
        crate::physics::BODY => typed::<crate::physics::RigidBody>(),
        crate::physics::SETTINGS => typed::<crate::physics::PhysicsSettings>(),
        "render.environment" => typed::<crate::light::RenderEnvironment>(),
        "nav.grid" => Some(json!(crate::navigation::NavSettings::default())),
        "audio.library" => typed::<crate::audio::AudioLibrary>(),
        "input.bindings" => typed::<InputBindings>(),
        "behavior.program" => typed::<crate::gameplay::Program>(),
        _ => None,
    };
    let validate = registry
        .components
        .get(id)
        .or_else(|| registry.resources.get(id))
        .map(|d| d.validate);
    let valid = |v: &Value| validate.is_none_or(|f| f(v).is_ok());
    preset
        .into_iter()
        .chain(std::iter::once(json!({})))
        .find(|v| valid(v))
}

fn type_schema(registry: &Registry, world: &World, id: &str, component: bool) -> Option<Value> {
    if component {
        registry
            .components
            .get(id)
            .map(|d| d.schema.clone())
            .or_else(|| world.component_schemas.get(id).map(|s| json!(s)))
    } else {
        registry
            .resources
            .get(id)
            .map(|d| d.schema.clone())
            .or_else(|| world.resource_schemas.get(id).map(|s| json!(s)))
    }
}

/// The value to write for a new component/resource: `value` or the default, checked by the
/// registered validator.
fn initial_value(
    world: &World,
    registry: &Registry,
    id: &str,
    value: Option<Value>,
    path: &str,
    component: bool,
) -> Result<Value> {
    let verb = if component {
        "add SEL"
    } else {
        "resource create"
    };
    let value = match value {
        Some(v) => v,
        None => default_value(world, registry, id).ok_or_else(|| {
            Diagnostic::error(
                "no_default",
                path,
                format!(
                    "{id} has no valid default; give a value: {verb} {id} {{…}} (schema {})",
                    type_schema(registry, world, id, component).unwrap_or(Value::Null)
                ),
            )
        })?,
    };
    let validate = if component {
        registry.components.get(id)
    } else {
        registry.resources.get(id)
    }
    .map(|d| d.validate);
    if let Some(validate) = validate {
        validate(&value).map_err(|e| invalid(path, format!("{id}: {e:#}")))?;
    }
    Ok(value)
}

/// World ▸ Resources ▸ Create / ACL `resource create ID [VALUE]`: a registered (or world
/// schema) resource with `value` or its default, plus its capability and the systems of that
/// capability that read it (see [`requirements`]). An existing resource is refused.
pub fn create_resource(
    world: &World,
    registry: &Registry,
    id: &str,
    value: Option<Value>,
) -> Result<Vec<Edit>> {
    let path = format!("/resources/{}", escape(id));
    if !registry.resources.contains_key(id) && !world.resource_schemas.contains_key(id) {
        return Err(Diagnostic::error(
            "unknown_resource",
            path,
            format!(
                "resource type '{id}' is not registered (registered: {})",
                list(
                    registry
                        .resources
                        .keys()
                        .chain(world.resource_schemas.keys())
                )
            ),
        ));
    }
    if world.resources.contains_key(id) {
        return Err(Diagnostic::error(
            "id_taken",
            path,
            format!(
                "resource '{id}' already exists; edit it with set resources[id={id}] FIELD VALUE"
            ),
        ));
    }
    let value = initial_value(world, registry, id, value, &path, false)?;
    let mut edits = requirement_edits(world, registry, &requirements(registry, id));
    edits.push(Edit::Core(CoreEdit::SetResource {
        id: id.to_owned(),
        value,
    }));
    Ok(edits)
}

/// World ▸ Resources ▸ Remove / ACL `resource remove ID`. What still needs the resource is
/// reported by the commit's validation.
pub fn remove_resource(world: &World, id: &str) -> Result<Vec<Edit>> {
    if !world.resources.contains_key(id) {
        return Err(Diagnostic::error(
            "not_found",
            format!("/resources/{}", escape(id)),
            format!("resource '{id}' not found"),
        ));
    }
    Ok(vec![Edit::Core(CoreEdit::RemoveResource {
        id: id.to_owned(),
    })])
}

/// World ▸ Sky / ACL `sky gradient|procedural|none`. `gradient` / `procedural` put that preset
/// sky in `render.environment` (created with its capability when missing) and, when no ambient
/// is authored, the sky light `ambient: {mode: sky}`. A sky already in that mode = empty batch
/// (tuned values and the ambient stay as authored). `none` removes the sky and a sky-mode
/// ambient (other ambient and fog stay). Only `sky` / `ambient` change: the other authored
/// values are copied as they are, never re-serialised. Unchanged = empty batch.
pub fn set_sky(world: &World, registry: &Registry, mode: &str) -> Result<Vec<Edit>> {
    const PATH: &str = "/resources/render.environment/sky";
    let preset = match mode {
        "none" => None,
        other => Some(crate::light::Sky::preset(other).ok_or_else(|| {
            invalid(
                PATH,
                format!("sky takes gradient|procedural|none, not '{other}'"),
            )
        })?),
    };
    let current = world.resources.get("render.environment");
    let mut value = current.cloned().unwrap_or_else(|| json!({}));
    let Some(env) = value.as_object_mut() else {
        return Err(invalid(
            "/resources/render.environment",
            "render.environment must be an object",
        ));
    };
    fn mode_of(v: Option<&Value>) -> Option<&str> {
        v.and_then(|v| v.get("mode")).and_then(Value::as_str)
    }
    let current_mode = mode_of(env.get("sky")).map(str::to_owned);
    match preset {
        None => {
            env.remove("sky");
            if mode_of(env.get("ambient")) == Some("sky") {
                env.remove("ambient");
            }
        }
        Some(_) if current_mode.as_deref() == Some(mode) => return Ok(vec![]),
        Some(sky) => {
            env.insert("sky".into(), json!(sky));
            env.entry("ambient")
                .or_insert_with(|| json!(crate::light::Ambient::Sky { intensity: 1.0 }));
        }
    }
    if current == Some(&value) || (current.is_none() && mode == "none") {
        return Ok(vec![]);
    }
    if current.is_none() {
        return create_resource(world, registry, "render.environment", Some(value));
    }
    if let Some(d) = registry.resources.get("render.environment") {
        (d.validate)(&value).map_err(|e| invalid(PATH, format!("{e:#}")))?;
    }
    Ok(vec![Edit::Core(CoreEdit::SetResource {
        id: "render.environment".into(),
        value,
    })])
}

/// World ▸ ID / ACL `world id ID`. Unchanged = empty batch.
pub fn set_world_id(world: &World, id: &str) -> Result<Vec<Edit>> {
    if valid_id(id).is_err() {
        return Err(Diagnostic::error(
            "invalid_id",
            "/id",
            format!("invalid world id '{id}' (1..128 of letters, digits, . _ / -)"),
        ));
    }
    if world.id == id {
        return Ok(vec![]);
    }
    Ok(vec![Edit::set("id", json!(id))])
}

/// World ▸ Title / ACL `world title TEXT`: trimmed, 1..128 bytes of display text.
pub fn set_world_title(world: &World, title: &str) -> Result<Vec<Edit>> {
    let title = title.trim();
    if title.is_empty() || title.len() > 128 || title.chars().any(char::is_control) {
        return Err(invalid(
            "/title",
            "title must be 1..128 bytes of display text",
        ));
    }
    if world.title == title {
        return Ok(vec![]);
    }
    Ok(vec![Edit::set("title", json!(title))])
}

/// Inspector ▸ Add component / ACL `add SEL COMPONENT [VALUE]`: puts `component` (`value`, or
/// its default) on each entity and adds the capability and systems it needs (see
/// [`requirements`]) in the same batch. Refused when an entity already has the component
/// (own or from its prefab; edit it instead).
pub fn add_component(
    world: &World,
    registry: &Registry,
    ids: &[String],
    component: &str,
    value: Option<Value>,
) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the objects that should get the component",
        ));
    }
    let path = format!("/components/{}", escape(component));
    if !registry.components.contains_key(component)
        && !world.component_schemas.contains_key(component)
    {
        return Err(Diagnostic::error(
            "unknown_component",
            path,
            format!(
                "component '{component}' is not registered (registered: {})",
                list(
                    registry
                        .components
                        .keys()
                        .chain(world.component_schemas.keys())
                )
            ),
        ));
    }
    let value = initial_value(world, registry, component, value, &path, true)?;
    let ids = unique(ids);
    for id in &ids {
        let entity = require_entity(world, id)?;
        if world.resolved_components(entity).contains_key(component) {
            return Err(Diagnostic::error(
                "component_exists",
                format!("{}{path}", entity_path(id)),
                format!(
                    "'{id}' already has {component}; edit it with set {id} {component}/FIELD VALUE"
                ),
            ));
        }
    }
    let mut edits = requirement_edits(world, registry, &requirements(registry, component));
    for id in ids {
        edits.push(Edit::Core(CoreEdit::SetComponent {
            entity: id.clone(),
            component: component.to_owned(),
            value: value.clone(),
        }));
    }
    Ok(edits)
}

/// The capability/system edits a hand-written new component needs (Inspector ▸ Stage
/// component, whose value the person typed): [`add_component`] without the component itself.
pub fn component_requirement_edits(
    world: &World,
    registry: &Registry,
    component: &str,
) -> Vec<Edit> {
    requirement_edits(world, registry, &requirements(registry, component))
}

/// An edit of one array element (ACL `append` / `insert` / `remove SEL FIELD INDEX` /
/// `reorder SEL FIELD FROM TO`).
#[derive(Clone, Debug, PartialEq)]
pub enum ArrayEdit {
    Append(Value),
    Insert(usize, Value),
    Remove(usize),
    /// Moves the element at the first index so it ends up at the second (both < length).
    Move(usize, usize),
}

/// Appends, inserts, removes or moves one element of the array at `field` (a JSON pointer
/// into each matched object, as for `set`) on every object `select` matches, writing each
/// whole array back as one field edit. Missing fields, non-arrays and indexes past the end
/// are errors.
pub fn edit_array(
    world: &World,
    registry: &Registry,
    select: &Selection,
    field: &str,
    edit: &ArrayEdit,
) -> Result<Vec<Edit>> {
    let field = crate::local::pointer(field);
    let targets = crate::local::matches(world, registry, select)?;
    if targets.is_empty() {
        return Err(Diagnostic::error(
            "no_match",
            "/select",
            "selector matched no objects",
        ));
    }
    let mut edits = vec![];
    for (target, data) in targets {
        let at = format!("{}{field}", target.path());
        let current = data.pointer(&field).ok_or_else(|| {
            Diagnostic::error(
                "invalid_field",
                &at,
                format!("{field} does not exist on {}", target.id()),
            )
        })?;
        let mut items = current
            .as_array()
            .ok_or_else(|| {
                Diagnostic::error(
                    "invalid_field",
                    &at,
                    format!("{field} is not an array (use set to replace it)"),
                )
            })?
            .clone();
        let len = items.len();
        match edit {
            ArrayEdit::Append(v) => items.push(v.clone()),
            ArrayEdit::Insert(i, v) if *i <= len => items.insert(*i, v.clone()),
            ArrayEdit::Remove(i) if *i < len => {
                items.remove(*i);
            }
            ArrayEdit::Move(from, to) if *from < len && *to < len => {
                let item = items.remove(*from);
                items.insert(*to, item);
            }
            ArrayEdit::Move(from, to) => {
                return Err(Diagnostic::error(
                    "index_out_of_range",
                    &at,
                    format!(
                        "index {} is out of range: {field} has {len} item(s)",
                        from.max(to)
                    ),
                ))
            }
            ArrayEdit::Insert(i, _) | ArrayEdit::Remove(i) => {
                return Err(Diagnostic::error(
                    "index_out_of_range",
                    &at,
                    format!("index {i} is out of range: {field} has {len} item(s)"),
                ))
            }
        }
        edits.push(Edit::Core(CoreEdit::SetField {
            target,
            field: field.clone(),
            value: Value::Array(items),
        }));
    }
    Ok(edits)
}

/// The event an "emits NAME, which behavior.program does not declare" error is about.
pub fn undeclared_event(message: &str) -> Option<&str> {
    let end = message.find(", which behavior.program does not declare")?;
    let start = message[..end].rfind(" emits ")? + " emits ".len();
    let name = &message[start..end];
    (!name.is_empty() && !name.contains(' ')).then_some(name)
}

/// Problems ▸ Declare event / ACL `event declare NAME [FIELD=TYPE ..]`: declares `name` in
/// `behavior.program` (creating the program, the `behavior` capability and `behavior.execute`
/// when missing). Emitters accept declarations with a subset of their text fields, so no
/// fields always works. Declared already with the same fields = empty batch.
pub fn declare_event(
    world: &World,
    name: &str,
    fields: &[(String, ScalarType)],
) -> Result<Vec<Edit>> {
    let path = format!("/resources/behavior.program/events/{}", escape(name));
    if valid_id(name).is_err() {
        return Err(invalid(path, format!("invalid event name '{name}'")));
    }
    let declaration = json!({
        "fields": fields.iter().cloned().collect::<BTreeMap<String, ScalarType>>()
    });
    let mut program = world
        .resources
        .get("behavior.program")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let events = program
        .as_object_mut()
        .map(|p| p.entry("events").or_insert_with(|| json!({})))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            invalid(
                "/resources/behavior.program/events",
                "behavior.program events must be an object",
            )
        })?;
    if let Some(existing) = events.get(name) {
        let same = existing.get("fields").unwrap_or(&json!({})) == &declaration["fields"];
        if same {
            return Ok(vec![]);
        }
        return Err(Diagnostic::error(
            "id_taken",
            path,
            format!("event '{name}' is already declared as {existing}"),
        ));
    }
    events.insert(name.to_owned(), declaration);
    let mut edits = vec![];
    let mut capabilities = world.capabilities.clone();
    capabilities.entry("behavior".into()).or_insert(1);
    if capabilities != world.capabilities {
        edits.push(Edit::set("/capabilities", json!(capabilities)));
    }
    if !world.systems.iter().any(|s| s == "behavior.execute") {
        let mut systems = world.systems.clone();
        systems.push("behavior.execute".into());
        edits.push(Edit::set("/systems", json!(systems)));
    }
    edits.push(Edit::Core(CoreEdit::SetResource {
        id: "behavior.program".into(),
        value: program,
    }));
    Ok(edits)
}

/// ACL `rotate SEL x,y,z` = the Inspector's Rotation° fields: sets `core.transform/rotation`
/// from Euler degrees (XYZ, [`crate::ops::rotation_from_degrees`]) on each entity.
pub fn set_rotation(world: &World, ids: &[String], degrees: [f32; 3]) -> Result<Vec<Edit>> {
    if !degrees.iter().all(|d| d.is_finite() && d.abs() <= 36_000.0) {
        return Err(invalid(
            "/rotation",
            "rotation degrees must be finite and within ±36000",
        ));
    }
    let rotation = crate::ops::rotation_from_degrees(degrees);
    set_fields(
        world,
        ids,
        &[(
            "/components/core.transform/rotation".into(),
            json!(rotation),
        )],
    )
}

/// Multi-object Inspector edits: the same field values on every entity (ACL `set
/// entities[id=a,id=b] FIELD VALUE`). Fields are JSON pointers into the entity record.
pub fn set_fields(world: &World, ids: &[String], fields: &[(String, Value)]) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "nothing selected",
        ));
    }
    let mut edits = vec![];
    for id in unique(ids) {
        require_entity(world, id)?;
        for (field, value) in fields {
            edits.push(Edit::Core(CoreEdit::SetField {
                target: Target::Entity { id: id.clone() },
                field: field.clone(),
                value: value.clone(),
            }));
        }
    }
    Ok(edits)
}

/// Inspector ▸ component ▸ Reset / ACL `reset SEL COMPONENT`: each entity's `component` back
/// to its starting value. An authored component becomes the type's default (as
/// [`add_component`] writes it); on a prefab instance that inherits the component the
/// override is dropped, so the prefab's value shows again. Entities without the component
/// are errors; an instance without an override is left as it is. A camera keeps its
/// `active` flag (resetting the main camera does not leave the world without one).
pub fn reset_component(
    world: &World,
    registry: &Registry,
    ids: &[String],
    component: &str,
) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "nothing selected",
        ));
    }
    let path = format!("/components/{}", escape(component));
    let mut edits = vec![];
    let mut default = None;
    for id in unique(ids) {
        let entity = require_entity(world, id)?;
        let inherited = entity
            .prefab
            .as_ref()
            .and_then(|p| world.prefabs.get(p))
            .is_some_and(|p| p.components.contains_key(component));
        let own = entity.components.contains_key(component);
        if inherited {
            if own {
                edits.push(Edit::Core(CoreEdit::RemoveComponent {
                    entity: id.clone(),
                    component: component.to_owned(),
                }));
            }
            continue;
        }
        if !own {
            return Err(Diagnostic::error(
                "not_found",
                format!("{}{path}", entity_path(id)),
                format!("'{id}' has no {component}"),
            ));
        }
        if default.is_none() {
            default = Some(initial_value(
                world, registry, component, None, &path, true,
            )?);
        }
        let mut value = default.clone().expect("set above");
        // Which camera is the main one is a world choice, not a field to reset.
        if let Some(active) = entity.components[component]
            .get("active")
            .filter(|_| component == "render.camera")
        {
            value["active"] = active.clone();
        }
        edits.push(Edit::Core(CoreEdit::SetComponent {
            entity: id.clone(),
            component: component.to_owned(),
            value,
        }));
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: anyhow::Error) -> String {
        e.downcast_ref::<Diagnostic>().unwrap().code.clone()
    }

    #[test]
    fn undeclared_event_names_are_parsed_from_every_emitter_message() {
        for message in [
            "invalid_trigger at /x: trigger emits door_bump, which behavior.program does not declare",
            "path follower emits door_bump, which behavior.program does not declare",
            "target emits door_bump, which behavior.program does not declare",
        ] {
            assert_eq!(undeclared_event(message), Some("door_bump"));
        }
        assert_eq!(undeclared_event("something else"), None);
    }

    #[test]
    fn requirements_follow_reads_capability_and_after() {
        let r = Registry::core();
        let needs = requirements(&r, "trigger.zone");
        assert_eq!(needs.capabilities, ["trigger"]);
        assert_eq!(needs.systems, ["trigger.detect"]);
        // nav.blocker is a core component read by nav.steer (nav): nothing scheduled.
        assert_eq!(
            requirements(&r, "nav.blocker").systems,
            Vec::<String>::new()
        );
        assert_eq!(requirements(&r, "unknown.x"), Requirements::default());
    }

    #[test]
    fn add_system_schedules_what_it_runs_after_first() {
        let mut r = Registry::core();
        r.capabilities.insert("x".into(), 1);
        r.systems.insert(
            "x.late".into(),
            SystemDescriptor {
                id: "x.late".into(),
                capability: "x".into(),
                phase: 50,
                reads: vec![],
                writes: vec![],
                after: vec!["trigger.detect".into()],
            },
        );
        let edits = add_system(&World::empty("w"), &r, "x.late").unwrap();
        let mut w = World::empty("w");
        for e in edits {
            e.apply(&mut w, &r).unwrap();
        }
        assert_eq!(w.systems, ["trigger.detect", "x.late"]);
        assert!(w.capabilities.contains_key("trigger") && w.capabilities.contains_key("x"));
        assert_eq!(r.schedule(&w).unwrap().len(), 2);
        // Removing the dependency first is refused.
        assert_eq!(
            code(remove_system(&w, &r, "trigger.detect").unwrap_err()),
            "system_in_use"
        );
        assert!(add_system(&w, &r, "x.late").unwrap().is_empty());
    }

    #[test]
    fn tags_keep_order_and_validate() {
        let mut w = World::empty("t");
        let mut a = Entity::new("a");
        a.tags = vec!["npc".into(), "x".into()];
        w.entities.push(a);
        let edits = edit_tags(&w, &["a".into()], &["player".into()], &["npc".into()]).unwrap();
        assert_eq!(edits.len(), 1);
        let Edit::Core(CoreEdit::SetField { value, .. }) = &edits[0] else {
            panic!()
        };
        assert_eq!(value, &json!(["x", "player"]));
        assert!(edit_tags(&w, &["a".into()], &["x".into()], &[])
            .unwrap()
            .is_empty());
        assert_eq!(
            code(edit_tags(&w, &["a".into()], &["a b".into()], &[]).unwrap_err()),
            "invalid_tag"
        );
        assert_eq!(
            code(edit_tags(&w, &["zz".into()], &["p".into()], &[]).unwrap_err()),
            "not_found"
        );
    }
}
