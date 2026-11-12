//! The assets a world names: what the editor's Assets browser lists and what agents read with
//! ACL `assets [KIND]` / `usages KIND:ID`.
//!
//! Assets are not a separate store. Each kind is a keyed map inside a resource (models in
//! `animation.library/assets`, materials in `render.shaders/materials`, ...), the world's prefabs,
//! or a singleton resource (`ui.document`, `input.bindings`). References are the fields of the
//! table below; [`usages`] and the rename op ([`crate::ops::rename_asset`]) walk the same table,
//! so a rename fixes exactly what Find Usages lists. References written by extension types
//! outside the table are not found.
use crate::local::escape;
use crate::*;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Asset kinds, in browser order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    /// Skinned model file (glb / gltf / pmx) in `animation.library/assets`.
    Model,
    /// PNG in `ui.document/images`.
    Image,
    /// `render.shaders/materials`.
    Material,
    /// Custom WGSL program in `render.shaders/shaders` (built-ins are not listed).
    Shader,
    /// `prefabs`.
    Prefab,
    /// FSM machine in `fsm.machines/machines`.
    Machine,
    /// Procedural clip in `audio.library/clips`.
    Audio,
    /// The `ui.document` resource (one per world).
    Ui,
    /// The `input.bindings` resource (one per world).
    Input,
}

/// Shaders the runtime provides without a `render.shaders/shaders` entry.
pub const BUILTIN_SHADERS: [&str; 3] = ["lit", "unlit", "toon"];

/// `(component, field pointer, kind)`: component fields that name an asset.
const COMPONENT_REFS: [(&str, &str, AssetKind); 6] = [
    ("animation.player", "/asset", AssetKind::Model),
    (
        "animation.player",
        "/retarget/source_asset",
        AssetKind::Model,
    ),
    ("render.material", "/id", AssetKind::Material),
    ("fsm.agent", "/machine", AssetKind::Machine),
    ("audio.source", "/clip", AssetKind::Audio),
    ("render.decal", "/image", AssetKind::Image),
];

impl AssetKind {
    pub const ALL: [AssetKind; 9] = [
        Self::Model,
        Self::Image,
        Self::Material,
        Self::Shader,
        Self::Prefab,
        Self::Machine,
        Self::Audio,
        Self::Ui,
        Self::Input,
    ];
    /// ACL / protocol name (`model:axiom-short`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Image => "image",
            Self::Material => "material",
            Self::Shader => "shader",
            Self::Prefab => "prefab",
            Self::Machine => "machine",
            Self::Audio => "audio",
            Self::Ui => "ui",
            Self::Input => "input",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
    /// Plural title for the browser's type tree.
    pub fn title(self) -> &'static str {
        match self {
            Self::Model => "Models",
            Self::Image => "Images",
            Self::Material => "Materials",
            Self::Shader => "Shaders",
            Self::Prefab => "Prefabs",
            Self::Machine => "FSM machines",
            Self::Audio => "Audio clips",
            Self::Ui => "UI document",
            Self::Input => "Input map",
        }
    }
    /// `(resource, pointer to the keyed map)` of a keyed kind; `None` for prefabs and the
    /// singleton kinds.
    pub fn location(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Self::Model => ("animation.library", "/assets"),
            Self::Image => ("ui.document", "/images"),
            Self::Material => ("render.shaders", "/materials"),
            Self::Shader => ("render.shaders", "/shaders"),
            Self::Machine => ("fsm.machines", "/machines"),
            Self::Audio => ("audio.library", "/clips"),
            Self::Prefab | Self::Ui | Self::Input => return None,
        })
    }
    /// The resource a singleton kind is.
    pub fn singleton(self) -> Option<&'static str> {
        match self {
            Self::Ui => Some("ui.document"),
            Self::Input => Some("input.bindings"),
            _ => None,
        }
    }
    /// Dropping it on the scene creates an object ([`crate::ops::place_asset`]).
    pub fn placeable(self) -> bool {
        matches!(self, Self::Model | Self::Prefab | Self::Material)
    }
    /// The browser can show a rendered / decoded thumbnail.
    pub fn has_thumbnail(self) -> bool {
        matches!(self, Self::Model | Self::Image | Self::Material)
    }
}

/// One listed asset.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetEntry {
    pub kind: AssetKind,
    pub id: String,
    /// Source file of models and images (as stored; absolute after a pack import).
    pub path: Option<String>,
}

impl AssetEntry {
    /// `kind:id`, the form ACL statements take.
    pub fn reference(&self) -> String {
        format!("{}:{}", self.kind.name(), self.id)
    }
}

/// Parses `kind:id`.
pub fn parse_reference(text: &str) -> Result<(AssetKind, String)> {
    let (kind, id) = text.split_once(':').unwrap_or(("", text));
    let kind = AssetKind::parse(kind).ok_or_else(|| {
        Diagnostic::error(
            "invalid_asset",
            "/asset",
            format!(
                "'{text}' is not KIND:ID (kinds: {})",
                AssetKind::ALL.map(AssetKind::name).join(", ")
            ),
        )
    })?;
    if id.is_empty() {
        return Err(Diagnostic::error(
            "invalid_asset",
            "/asset",
            format!("'{text}' names no asset ID"),
        ));
    }
    Ok((kind, id.to_owned()))
}

/// The keyed map of `kind`, if the world has it.
pub fn map(world: &World, kind: AssetKind) -> Option<&serde_json::Map<String, Value>> {
    let (resource, pointer) = kind.location()?;
    world.resources.get(resource)?.pointer(pointer)?.as_object()
}

/// Whether `kind:id` exists.
pub fn exists(world: &World, kind: AssetKind, id: &str) -> bool {
    match kind {
        AssetKind::Prefab => world.prefabs.contains_key(id),
        AssetKind::Ui | AssetKind::Input => {
            kind.singleton() == Some(id) && world.resources.contains_key(id)
        }
        _ => map(world, kind).is_some_and(|m| m.contains_key(id)),
    }
}

/// Every asset of the world (kind order, then ID order).
pub fn list(world: &World) -> Vec<AssetEntry> {
    let mut out = vec![];
    for kind in AssetKind::ALL {
        match kind {
            AssetKind::Prefab => out.extend(world.prefabs.keys().map(|id| AssetEntry {
                kind,
                id: id.clone(),
                path: None,
            })),
            AssetKind::Ui | AssetKind::Input => {
                let id = kind.singleton().expect("singleton kind");
                if world.resources.contains_key(id) {
                    out.push(AssetEntry {
                        kind,
                        id: id.to_owned(),
                        path: None,
                    });
                }
            }
            _ => {
                for (id, value) in map(world, kind).into_iter().flatten() {
                    let path = match kind {
                        AssetKind::Model => value.get("path").and_then(Value::as_str),
                        AssetKind::Image => value.as_str(),
                        _ => None,
                    };
                    out.push(AssetEntry {
                        kind,
                        id: id.clone(),
                        path: path.map(str::to_owned),
                    });
                }
            }
        }
    }
    out
}

/// Who holds a reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// An entity's own (authored) component or its `prefab` field.
    Entity(String),
    Prefab(String),
    Resource(String),
    /// An entity that inherits the reference from its prefab without overriding it.
    Instance {
        entity: String,
        prefab: String,
    },
}

/// One field that names an asset. `field` is a JSON pointer inside the owner (the form
/// `set_field` takes); instances carry the prefab's field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub owner: Owner,
    pub field: String,
}

impl Usage {
    /// One compact line (`entity lumi /components/animation.player/asset`).
    pub fn line(&self) -> String {
        match &self.owner {
            Owner::Entity(id) => format!("entity {id} {}", self.field),
            Owner::Prefab(id) => format!("prefab {id} {}", self.field),
            Owner::Resource(id) => format!("resource {id} {}", self.field),
            Owner::Instance { entity, prefab } => {
                format!("instance {entity} (prefab {prefab}) {}", self.field)
            }
        }
    }
}

/// Calls `found(kind, id, owner, field)` for every authored reference, in a fixed order:
/// entities (authored order), prefabs, then resources. Instances are not reported here.
fn walk(world: &World, mut found: impl FnMut(AssetKind, &str, Owner, String)) {
    let component_refs =
        |owner: &dyn Fn() -> Owner,
         components: &Components,
         found: &mut dyn FnMut(AssetKind, &str, Owner, String)| {
            for (component, pointer, kind) in COMPONENT_REFS {
                if let Some(id) = components
                    .get(component)
                    .and_then(|c| c.pointer(pointer))
                    .and_then(Value::as_str)
                {
                    found(
                        kind,
                        id,
                        owner(),
                        format!("/components/{}{pointer}", escape(component)),
                    );
                }
            }
        };
    for e in &world.entities {
        if let Some(prefab) = &e.prefab {
            found(
                AssetKind::Prefab,
                prefab,
                Owner::Entity(e.id.clone()),
                "/prefab".into(),
            );
        }
        component_refs(&|| Owner::Entity(e.id.clone()), &e.components, &mut found);
    }
    for (id, prefab) in &world.prefabs {
        component_refs(
            &|| Owner::Prefab(id.clone()),
            &prefab.components,
            &mut found,
        );
    }
    if let Some(materials) = map(world, AssetKind::Material) {
        for (material, value) in materials {
            if let Some(shader) = value.get("shader").and_then(Value::as_str) {
                found(
                    AssetKind::Shader,
                    shader,
                    Owner::Resource("render.shaders".into()),
                    format!("/materials/{}/shader", escape(material)),
                );
            }
        }
    }
    if let Some(widgets) = world
        .resources
        .get("ui.document")
        .and_then(|d| d.get("widgets"))
        .and_then(Value::as_array)
    {
        for (i, w) in widgets.iter().enumerate() {
            if w.get("kind").and_then(Value::as_str) == Some("image") {
                if let Some(image) = w.get("image").and_then(Value::as_str) {
                    found(
                        AssetKind::Image,
                        image,
                        Owner::Resource("ui.document".into()),
                        format!("/widgets/{i}/image"),
                    );
                }
            }
        }
    }
}

/// Every field that names `kind:id` (Find Usages): authored references, then the instances that
/// inherit a prefab's reference without overriding that field.
pub fn usages(world: &World, kind: AssetKind, id: &str) -> Vec<Usage> {
    let mut out = vec![];
    walk(world, |k, target, owner, field| {
        if k == kind && target == id {
            out.push(Usage { owner, field });
        }
    });
    let inherited: Vec<(String, String)> = out
        .iter()
        .filter_map(|u| match &u.owner {
            Owner::Prefab(p) => Some((p.clone(), u.field.clone())),
            _ => None,
        })
        .collect();
    for (prefab, field) in inherited {
        for e in world
            .entities
            .iter()
            .filter(|e| e.prefab.as_deref() == Some(prefab.as_str()))
        {
            // `/components/C/F`: overridden when the entity authors that field itself.
            let overridden = field
                .strip_prefix("/components/")
                .and_then(|rest| rest.split_once('/'))
                .and_then(|(component, pointer)| {
                    let component = component.replace("~1", "/").replace("~0", "~");
                    e.components
                        .get(&component)?
                        .pointer(&format!("/{pointer}"))
                })
                .is_some();
            if !overridden {
                out.push(Usage {
                    owner: Owner::Instance {
                        entity: e.id.clone(),
                        prefab: prefab.clone(),
                    },
                    field: field.clone(),
                });
            }
        }
    }
    out
}

/// Authored reference counts per asset (one pass over the world), for listings.
pub fn reference_counts(world: &World) -> HashMap<(AssetKind, String), usize> {
    let mut counts = HashMap::new();
    walk(world, |kind, id, _, _| {
        *counts.entry((kind, id.to_owned())).or_insert(0) += 1;
    });
    counts
}

/// The file name of a stored path (either separator).
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// `discover assets`.
pub fn register(r: &mut Registry) {
    r.topics.insert(
        "assets".into(),
        json!({
            "kinds": AssetKind::ALL.map(|k| {
                let at = match (k.location(), k.singleton()) {
                    (Some((res, ptr)), _) => format!("{res}{ptr}"),
                    (None, Some(res)) => res.to_owned(),
                    _ => "prefabs".to_owned(),
                };
                json!([k.name(), at])
            }),
            "list": "acl: assets [KIND]  (kind id [file=] refs=N)",
            "usages": "acl: usages KIND:ID  (entity|prefab|resource|instance OWNER FIELD; instance = inherits from its prefab)",
            "place": "acl: asset place model|prefab|material:ID [id=] [at=x,y,z] [rot=x,y,z] [parent=] [scene=]  (editor: drag onto the Scene)",
            "create": "acl: asset create material ID [shader=lit|unlit|toon|ID] [tint=r,g,b,a] | asset create prefab ID [from=ENTITY] | asset create machine ID [initial=STATE] | asset create ui [locale=L] | asset create input",
            "rename": "acl: asset rename KIND:ID NEW  (one revision; fixes every reference usages lists)",
            "references": COMPONENT_REFS.map(|(c, p, k)| format!("{c}{p} -> {}", k.name())),
            "also": ["entity /prefab -> prefab", "render.shaders /materials/*/shader -> shader", "ui.document widgets[kind=image]/image -> image"],
            "thumbnails": "editor-only cache files <db>.thumbs/<content hash>.png, never in revisions; editor_view {command:\"asset.thumbnails\"}"
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut b = Entity::new("npc/2");
        b.prefab = Some("npc".into());
        b.components
            .insert("animation.player".into(), json!({"asset":"axiom-long"}));
        w.entities.push(a);
        w.entities.push(b);
        w
    }

    #[test]
    fn lists_kinds_and_parses_references() {
        let w = town();
        let all = list(&w);
        assert!(all
            .iter()
            .any(|a| a.kind == AssetKind::Model && a.id == "axiom-short"));
        assert_eq!(
            all.iter()
                .find(|a| a.id == "axiom-short")
                .unwrap()
                .path
                .as_deref(),
            Some("axiom-short.glb")
        );
        assert!(all.iter().any(|a| a.kind == AssetKind::Ui));
        assert_eq!(
            parse_reference("model:axiom-short").unwrap(),
            (AssetKind::Model, "axiom-short".into())
        );
        assert!(parse_reference("axiom-short").is_err());
        assert!(parse_reference("model:").is_err());
        assert!(exists(&w, AssetKind::Model, "axiom-short"));
        assert!(!exists(&w, AssetKind::Model, "nope"));
        assert!(exists(&w, AssetKind::Ui, "ui.document"));
        assert!(!exists(&w, AssetKind::Ui, "input.bindings"));
        assert_eq!(file_name("C:\\a\\b.glb"), "b.glb");
    }

    #[test]
    fn usages_cover_entities_prefabs_and_inheriting_instances() {
        let w = town();
        let found = usages(&w, AssetKind::Model, "axiom-short");
        let entities: Vec<&Owner> = found.iter().map(|u| &u.owner).collect();
        // Three town characters author it, the prefab does, npc/1 inherits it; npc/2
        // overrides the field with another model.
        let authored = found
            .iter()
            .filter(|u| matches!(u.owner, Owner::Entity(_)))
            .count();
        assert_eq!(authored, 3, "{entities:?}");
        assert!(found.contains(&Usage {
            owner: Owner::Prefab("npc".into()),
            field: "/components/animation.player/asset".into()
        }));
        assert!(found.contains(&Usage {
            owner: Owner::Instance {
                entity: "npc/1".into(),
                prefab: "npc".into()
            },
            field: "/components/animation.player/asset".into()
        }));
        assert!(!found
            .iter()
            .any(|u| matches!(&u.owner, Owner::Instance { entity, .. } if entity == "npc/2")));
        let prefab = usages(&w, AssetKind::Prefab, "npc");
        assert_eq!(prefab.len(), 2);
        assert_eq!(
            reference_counts(&w)[&(AssetKind::Model, "axiom-short".to_owned())],
            4
        );
        assert!(usages(&w, AssetKind::Model, "nope").is_empty());
    }
}
