//! Asset operations (re-exported from [`crate::ops`]): drop an asset into the scene, create
//! material / prefab / FSM machine / UI document / input map assets, and rename an asset with
//! every reference fixed in the same batch. Pure like the rest of `ops`; the Assets browser and
//! ACL `asset place|create|rename` call these same functions.
use crate::assets::{self, AssetKind, Owner};
use crate::local::{escape, Target};
use crate::ops::{create_object, create_prefab, create_resource, instantiate_prefab, unique_id};
use crate::ops::{Created, Placement};
use crate::*;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn asset_path(kind: AssetKind, id: &str) -> String {
    match (kind.location(), kind.singleton()) {
        (Some((resource, pointer)), _) => format!("/resources/{resource}{pointer}/{}", escape(id)),
        (None, Some(resource)) => format!("/resources/{resource}"),
        _ => format!("/prefabs/{}", escape(id)),
    }
}

fn missing(kind: AssetKind, id: &str) -> anyhow::Error {
    Diagnostic::error(
        "not_found",
        asset_path(kind, id),
        format!(
            "no {} asset '{id}' (list them: assets {})",
            kind.name(),
            kind.name()
        ),
    )
}

fn check_id(path: &str, id: &str) -> Result<()> {
    if valid_id(id).is_err() {
        return Err(Diagnostic::error(
            "invalid_id",
            path,
            format!("invalid id '{id}' (1..128 of letters, digits, . _ / -)"),
        ));
    }
    Ok(())
}

/// `world` with `edits` applied (later steps of one op see earlier ones, like ACL lines).
fn applied(world: &World, registry: &Registry, edits: &[Edit]) -> Result<World> {
    let mut scratch = world.clone();
    for edit in edits {
        edit.clone().apply(&mut scratch, registry)?;
    }
    Ok(scratch)
}

/// Adds `component = value` to the entity the batch puts, then the capability / systems the
/// component needs (computed after the batch, so two `/capabilities` writes never race).
fn with_component(
    world: &World,
    registry: &Registry,
    mut created: Created,
    component: &str,
    value: Value,
) -> Result<Created> {
    let id = created.ids.first().cloned().unwrap_or_default();
    for edit in &mut created.edits {
        if let Edit::Core(CoreEdit::PutEntity { entity }) = edit {
            if entity.id == id {
                entity
                    .components
                    .insert(component.to_owned(), value.clone());
            }
        }
    }
    let scratch = applied(world, registry, &created.edits)?;
    created
        .edits
        .extend(crate::ops::component_requirement_edits(
            &scratch, registry, component,
        ));
    Ok(created)
}

/// Assets ▸ drag onto the Scene / ACL `asset place KIND:ID P`: a model becomes an object with
/// `core.transform` + `animation.player {asset}` (rest pose; pick a clip in the Inspector), a
/// prefab an instance (as `instantiate`), a material a 1 m cube with `render.material {id}`.
/// The default object ID is the asset ID (then `ID/2`, ...). Other kinds are refused.
pub fn place_asset(
    world: &World,
    registry: &Registry,
    kind: AssetKind,
    id: &str,
    place: &Placement,
) -> Result<Created> {
    if !kind.placeable() {
        return Err(Diagnostic::error(
            "not_placeable",
            asset_path(kind, id),
            format!(
                "{} assets do not create scene objects (drop a model, prefab or material; reference a {} from a component field instead)",
                kind.name(),
                kind.name()
            ),
        ));
    }
    if !assets::exists(world, kind, id) {
        return Err(missing(kind, id));
    }
    if kind == AssetKind::Prefab {
        return instantiate_prefab(world, id, place);
    }
    let mut place = place.clone();
    if place.id.is_none() {
        place.id = Some(unique_id(world, id, &BTreeSet::new()));
    }
    match kind {
        AssetKind::Model => {
            let created = create_object(world, "empty", &place)?;
            with_component(
                world,
                registry,
                created,
                "animation.player",
                json!({"asset": id}),
            )
        }
        _ => {
            let created = create_object(world, "cube", &place)?;
            with_component(
                world,
                registry,
                created,
                "render.material",
                json!({"id": id}),
            )
        }
    }
}

/// `base` if no `kind` asset has that ID, else `base/2`, `base/3`, ... (New asset defaults).
pub fn free_asset_id(world: &World, kind: AssetKind, base: &str) -> String {
    if !assets::exists(world, kind, base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}/{n}"))
        .find(|id| !assets::exists(world, kind, id))
        .unwrap()
}

/// Options of a new asset (ACL `asset create ...`; the Assets ▸ New dialogs).
#[derive(Clone, Debug, PartialEq)]
pub enum NewAsset {
    /// `render.shaders/materials/ID {shader, tint}`; shader default `lit`.
    Material {
        id: String,
        shader: Option<String>,
        tint: Option<[f32; 4]>,
    },
    /// An empty prefab (`core.transform` only), or `from` an entity (as `prefab create`).
    Prefab { id: String, from: Option<String> },
    /// `fsm.machines/machines/ID` with one `stop` state (default `idle`).
    Machine { id: String, initial: Option<String> },
    /// The world's `ui.document` with one empty catalog (default locale `en`), plus an empty
    /// `behavior.program` and `behavior.execute` when the world has none (UI runs on them).
    Ui { locale: Option<String> },
    /// The world's `input.bindings` (no bindings yet).
    Input,
}

impl NewAsset {
    /// The equivalent ACL statement.
    pub fn acl(&self) -> String {
        match self {
            NewAsset::Material { id, shader, tint } => {
                let mut s = format!("asset create material {id}");
                if let Some(shader) = shader {
                    s.push_str(&format!(" shader={shader}"));
                }
                if let Some(t) = tint {
                    s.push_str(&format!(" tint={},{},{},{}", t[0], t[1], t[2], t[3]));
                }
                s
            }
            NewAsset::Prefab { id, from } => match from {
                Some(from) => format!("asset create prefab {id} from={from}"),
                None => format!("asset create prefab {id}"),
            },
            NewAsset::Machine { id, initial } => match initial {
                Some(state) => format!("asset create machine {id} initial={state}"),
                None => format!("asset create machine {id}"),
            },
            NewAsset::Ui { locale } => match locale {
                Some(l) => format!("asset create ui locale={l}"),
                None => "asset create ui".to_owned(),
            },
            NewAsset::Input => "asset create input".to_owned(),
        }
    }

    /// The kind and ID the asset gets.
    pub fn target(&self) -> (AssetKind, String) {
        match self {
            NewAsset::Material { id, .. } => (AssetKind::Material, id.clone()),
            NewAsset::Prefab { id, .. } => (AssetKind::Prefab, id.clone()),
            NewAsset::Machine { id, .. } => (AssetKind::Machine, id.clone()),
            NewAsset::Ui { .. } => (AssetKind::Ui, "ui.document".into()),
            NewAsset::Input => (AssetKind::Input, "input.bindings".into()),
        }
    }
}

/// Puts `value` at `pointer/key` of keyed resource `resource`, creating the resource (with its
/// capability and systems) when the world lacks it.
fn put_keyed(
    world: &World,
    registry: &Registry,
    kind: AssetKind,
    key: &str,
    value: Value,
) -> Result<Vec<Edit>> {
    let (resource, pointer) = kind.location().expect("keyed kind");
    let path = asset_path(kind, key);
    check_id(&path, key)?;
    if assets::exists(world, kind, key) {
        return Err(Diagnostic::error(
            "id_taken",
            path,
            format!("{} '{key}' already exists", kind.name()),
        ));
    }
    let map_name = &pointer[1..];
    let Some(current) = world.resources.get(resource) else {
        return create_resource(
            world,
            registry,
            resource,
            Some(json!({ map_name: {key: value} })),
        );
    };
    let target = Target::Resource {
        id: resource.to_owned(),
    };
    Ok(vec![Edit::Core(if current.get(map_name).is_some() {
        CoreEdit::SetField {
            target,
            field: format!("{pointer}/{}", escape(key)),
            value,
        }
    } else {
        CoreEdit::SetField {
            target,
            field: pointer.to_owned(),
            value: json!({ key: value }),
        }
    })])
}

/// Assets ▸ New / ACL `asset create ...`.
pub fn create_asset(world: &World, registry: &Registry, new: &NewAsset) -> Result<Vec<Edit>> {
    match new {
        NewAsset::Material { id, shader, tint } => {
            let shader = shader.as_deref().unwrap_or("lit");
            let path = asset_path(AssetKind::Material, id);
            if !assets::BUILTIN_SHADERS.contains(&shader)
                && !assets::exists(world, AssetKind::Shader, shader)
            {
                return Err(Diagnostic::error(
                    "unknown_shader",
                    format!("{path}/shader"),
                    format!(
                        "no shader '{shader}' (built-in: {}; custom ones: assets shader)",
                        assets::BUILTIN_SHADERS.join(", ")
                    ),
                ));
            }
            let mut material = json!({"shader": shader});
            if let Some(tint) = tint {
                if !tint
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                {
                    return Err(Diagnostic::error(
                        "invalid_value",
                        format!("{path}/tint"),
                        "tint is r,g,b,a with each 0..1",
                    ));
                }
                material["tint"] = json!(tint);
            }
            let mut edits = vec![];
            // Materials need the shader capability, which needs render.
            for capability in ["render", "shader"] {
                if registry.capabilities.contains_key(capability) {
                    let scratch = applied(world, registry, &edits)?;
                    edits.extend(crate::ops::add_capability(&scratch, registry, capability)?);
                }
            }
            let scratch = applied(world, registry, &edits)?;
            edits.extend(put_keyed(
                &scratch,
                registry,
                AssetKind::Material,
                id,
                material,
            )?);
            Ok(edits)
        }
        NewAsset::Prefab {
            id,
            from: Some(from),
        } => create_prefab(world, Some(id), from).map(|c| c.edits),
        NewAsset::Prefab { id, from: None } => {
            let path = asset_path(AssetKind::Prefab, id);
            check_id(&path, id)?;
            if world.prefabs.contains_key(id) {
                return Err(Diagnostic::error(
                    "id_taken",
                    path,
                    format!("prefab '{id}' already exists"),
                ));
            }
            Ok(vec![Edit::Core(CoreEdit::PutPrefab {
                id: id.clone(),
                prefab: Prefab {
                    components: [("core.transform".to_owned(), json!(Transform::default()))].into(),
                },
            })])
        }
        NewAsset::Machine { id, initial } => {
            let state = initial.as_deref().unwrap_or("idle");
            check_id(
                &format!("{}/initial", asset_path(AssetKind::Machine, id)),
                state,
            )?;
            put_keyed(
                world,
                registry,
                AssetKind::Machine,
                id,
                json!({"initial": state, "states": {state: {}}, "transitions": []}),
            )
        }
        NewAsset::Ui { locale } => {
            let locale = locale.as_deref().unwrap_or("en");
            check_id("/resources/ui.document/locale", locale)?;
            // Gameplay resources (UI documents among them) run on behavior.program and the
            // behavior.execute system: an empty program comes along when the world has none.
            let mut edits = vec![];
            if !world.resources.contains_key("behavior.program") {
                edits.extend(create_resource(world, registry, "behavior.program", None)?);
            }
            if registry.systems.contains_key("behavior.execute") {
                let scratch = applied(world, registry, &edits)?;
                edits.extend(crate::ops::add_system(
                    &scratch,
                    registry,
                    "behavior.execute",
                )?);
            }
            let scratch = applied(world, registry, &edits)?;
            edits.extend(create_resource(
                &scratch,
                registry,
                "ui.document",
                Some(json!({"locale": locale, "catalogs": {locale: {}}, "widgets": []})),
            )?);
            Ok(edits)
        }
        NewAsset::Input => create_resource(world, registry, "input.bindings", None),
    }
}

/// Assets ▸ Rename / ACL `asset rename KIND:ID NEW`: the asset gets the new ID and every
/// reference [`assets::usages`] finds (entity / prefab components, prefab instances' `prefab`,
/// material shaders, UI image widgets) is rewritten, in one batch. Instances that inherit a
/// reference follow their prefab. Same ID = empty batch; singleton kinds cannot be renamed.
pub fn rename_asset(world: &World, kind: AssetKind, id: &str, new: &str) -> Result<Vec<Edit>> {
    if let Some(resource) = kind.singleton() {
        return Err(Diagnostic::error(
            "not_renameable",
            format!("/resources/{resource}"),
            format!(
                "{resource} is the world's only {} asset; its ID is fixed",
                kind.name()
            ),
        ));
    }
    if !assets::exists(world, kind, id) {
        return Err(missing(kind, id));
    }
    if id == new {
        return Ok(vec![]);
    }
    let path = asset_path(kind, new);
    check_id(&path, new)?;
    if assets::exists(world, kind, new)
        || (kind == AssetKind::Shader && assets::BUILTIN_SHADERS.contains(&new))
    {
        return Err(Diagnostic::error(
            "id_taken",
            path,
            format!("{} '{new}' already exists", kind.name()),
        ));
    }
    let mut edits = vec![];
    if kind == AssetKind::Prefab {
        edits.push(Edit::Core(CoreEdit::PutPrefab {
            id: new.to_owned(),
            prefab: world.prefabs[id].clone(),
        }));
        edits.push(Edit::Core(CoreEdit::RemoveObject {
            target: Target::Prefab { id: id.to_owned() },
        }));
    } else {
        let (resource, pointer) = kind.location().expect("keyed kind");
        let mut renamed = assets::map(world, kind).expect("exists").clone();
        let value = renamed.remove(id).expect("exists");
        renamed.insert(new.to_owned(), value);
        edits.push(Edit::Core(CoreEdit::SetField {
            target: Target::Resource {
                id: resource.to_owned(),
            },
            field: pointer.to_owned(),
            value: Value::Object(renamed),
        }));
    }
    for usage in assets::usages(world, kind, id) {
        let target = match usage.owner {
            Owner::Entity(id) => Target::Entity { id },
            Owner::Prefab(id) => Target::Prefab { id },
            Owner::Resource(id) => Target::Resource { id },
            Owner::Instance { .. } => continue,
        };
        edits.push(Edit::Core(CoreEdit::SetField {
            target,
            field: usage.field,
            value: json!(new),
        }));
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::usages;

    fn town() -> World {
        let mut w: World =
            serde_json::from_str(include_str!("../../../content/axiom-town/world.json")).unwrap();
        w.prefabs.insert(
            "npc".into(),
            Prefab {
                components: [(
                    "animation.player".to_owned(),
                    json!({"asset":"axiom-short"}),
                )]
                .into(),
            },
        );
        let mut a = Entity::new("npc/1");
        a.prefab = Some("npc".into());
        w.entities.push(a);
        w
    }

    fn run(world: &World, edits: &[Edit]) -> World {
        applied(world, &Registry::core(), edits).unwrap()
    }

    #[test]
    fn rename_fixes_every_reference_and_keeps_the_value() {
        let w = town();
        let before = usages(&w, AssetKind::Model, "axiom-short").len();
        assert!(before >= 5);
        let edits = rename_asset(&w, AssetKind::Model, "axiom-short", "mascot/short").unwrap();
        let after = run(&w, &edits);
        assert!(usages(&after, AssetKind::Model, "axiom-short").is_empty());
        assert_eq!(
            usages(&after, AssetKind::Model, "mascot/short").len(),
            before
        );
        assert_eq!(
            after.resources["animation.library"]["assets"]["mascot/short"],
            w.resources["animation.library"]["assets"]["axiom-short"]
        );
        // The inheriting instance still inherits (nothing authored on it).
        assert!(after.entity("npc/1").unwrap().components.is_empty());
        // Prefab rename moves the prefab and its instances.
        let edits = rename_asset(&w, AssetKind::Prefab, "npc", "villager").unwrap();
        let after = run(&w, &edits);
        assert!(!after.prefabs.contains_key("npc") && after.prefabs.contains_key("villager"));
        assert_eq!(
            after.entity("npc/1").unwrap().prefab.as_deref(),
            Some("villager")
        );
        // Refusals and the no-op.
        assert!(
            rename_asset(&w, AssetKind::Model, "axiom-short", "axiom-short")
                .unwrap()
                .is_empty()
        );
        let code = |r: Result<Vec<Edit>>| {
            r.unwrap_err()
                .downcast_ref::<Diagnostic>()
                .map(|d| d.code.clone())
                .unwrap()
        };
        assert_eq!(
            code(rename_asset(
                &w,
                AssetKind::Model,
                "axiom-short",
                "axiom-long"
            )),
            "id_taken"
        );
        assert_eq!(
            code(rename_asset(&w, AssetKind::Model, "nope", "x")),
            "not_found"
        );
        assert_eq!(
            code(rename_asset(&w, AssetKind::Model, "axiom-short", "bad id")),
            "invalid_id"
        );
        assert_eq!(
            code(rename_asset(&w, AssetKind::Ui, "ui.document", "x")),
            "not_renameable"
        );
        // A custom shader cannot take a built-in shader's name.
        let mut shaded = w.clone();
        shaded.resources.insert(
            "render.shaders".into(),
            json!({"shaders":{"glow":{"wgsl":"x"}},"materials":{"m":{"shader":"glow"}}}),
        );
        assert_eq!(
            code(rename_asset(&shaded, AssetKind::Shader, "glow", "lit")),
            "id_taken"
        );
        let edits = rename_asset(&shaded, AssetKind::Shader, "glow", "bloom").unwrap();
        let after = run(&shaded, &edits);
        assert_eq!(
            after.resources["render.shaders"]["materials"]["m"]["shader"],
            "bloom"
        );
        assert!(after.resources["render.shaders"]["shaders"]["bloom"].is_object());
    }

    #[test]
    fn place_and_create_build_the_documented_objects() {
        let w = town();
        let r = Registry::core();
        let created = place_asset(
            &w,
            &r,
            AssetKind::Model,
            "axiom-short",
            &Placement {
                position: Some([1.0, 0.0, 2.0]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(created.ids, ["axiom-short"]);
        let after = run(&w, &created.edits);
        let e = after.entity("axiom-short").unwrap();
        assert_eq!(
            e.components["animation.player"],
            json!({"asset":"axiom-short"})
        );
        assert_eq!(
            e.components["core.transform"]["position"],
            json!([1.0, 0.0, 2.0])
        );
        // Second drop gets the next free ID.
        let again = place_asset(
            &after,
            &r,
            AssetKind::Model,
            "axiom-short",
            &Placement::default(),
        )
        .unwrap();
        assert_eq!(again.ids, ["axiom-short/2"]);
        let refused = place_asset(&w, &r, AssetKind::Image, "x", &Placement::default());
        assert!(refused.is_err());
        let machine = create_asset(
            &w,
            &r,
            &NewAsset::Machine {
                id: "guard".into(),
                initial: Some("patrol".into()),
            },
        )
        .unwrap();
        let after = run(&w, &machine);
        assert_eq!(
            after.resources["fsm.machines"]["machines"]["guard"],
            json!({"initial":"patrol","states":{"patrol":{}},"transitions":[]})
        );
        // A taken ID and a world without the resource type (core registry) are refused.
        let taken = create_asset(
            &after,
            &r,
            &NewAsset::Machine {
                id: "guard".into(),
                initial: None,
            },
        );
        assert!(taken.is_err());
        assert!(create_asset(
            &World::empty("e"),
            &r,
            &NewAsset::Machine {
                id: "guard".into(),
                initial: None
            }
        )
        .is_err());
        let prefab = create_asset(
            &w,
            &r,
            &NewAsset::Prefab {
                id: "empty_thing".into(),
                from: None,
            },
        )
        .unwrap();
        assert!(run(&w, &prefab).prefabs.contains_key("empty_thing"));
        assert_eq!(
            NewAsset::Material {
                id: "m".into(),
                shader: Some("toon".into()),
                tint: Some([1.0, 0.5, 0.0, 1.0])
            }
            .acl(),
            "asset create material m shader=toon tint=1,0.5,0,1"
        );
    }
}
