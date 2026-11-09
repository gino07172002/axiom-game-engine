//! Versioned, genre-independent authoring data and typed compiled storage.
use crate::{Diagnostic, Registry};
use anyhow::{ensure, Context, Result};
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_ENTITIES: usize = 4096;
pub const MAX_INSTANCES: usize = 8192;
pub const MAX_CAMERAS: usize = 8;
pub type Components = BTreeMap<String, Value>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct World {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub capabilities: BTreeMap<String, u32>,
    #[serde(default)]
    pub component_schemas: BTreeMap<String, RecordSchema>,
    #[serde(default)]
    pub resource_schemas: BTreeMap<String, RecordSchema>,
    #[serde(default)]
    pub entities: Vec<Entity>,
    #[serde(default)]
    pub prefabs: BTreeMap<String, Prefab>,
    #[serde(default)]
    pub resources: BTreeMap<String, Value>,
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default)]
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub systems: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packages: Option<crate::package::PackageLock>,
}
impl World {
    pub fn empty(id: impl Into<String>) -> Self {
        Self {
            schema_version: 2,
            id: id.into(),
            title: "Empty world".into(),
            capabilities: BTreeMap::from([("core".into(), 1)]),
            component_schemas: BTreeMap::new(),
            resource_schemas: BTreeMap::new(),
            entities: vec![],
            prefabs: BTreeMap::new(),
            resources: BTreeMap::new(),
            relations: vec![],
            scenes: vec![],
            systems: vec![],
            packages: None,
        }
    }
    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| e.id == id)
    }
    pub fn entity_mut(&mut self, id: &str) -> Option<&mut Entity> {
        self.entities.iter_mut().find(|e| e.id == id)
    }
    pub fn resolved_components(&self, entity: &Entity) -> Components {
        let mut values = entity
            .prefab
            .as_ref()
            .and_then(|p| self.prefabs.get(p))
            .map(|p| p.components.clone())
            .unwrap_or_default();
        for (id, value) in &entity.components {
            if let Some(old) = values.get_mut(id) {
                merge_json(old, value);
            } else {
                values.insert(id.clone(), value.clone());
            }
        }
        values
    }
    pub fn resource<T: serde::de::DeserializeOwned>(&self, id: &str) -> Result<T> {
        serde_json::from_value(
            self.resources
                .get(id)
                .with_context(|| format!("missing resource {id}"))?
                .clone(),
        )
        .with_context(|| format!("resource {id}"))
    }
    pub fn compile(&self, r: &Registry) -> Result<CompiledWorld> {
        if let Some(packages) = &self.packages {
            packages.validate(self, r)?;
        }
        ensure!(
            self.schema_version == 2,
            "unsupported schema_version {}; expected 2",
            self.schema_version
        );
        valid_id(&self.id)?;
        ensure!(self.title.len() <= 128, "title exceeds 128 bytes");
        ensure!(
            self.entities.len() <= MAX_ENTITIES
                && self.prefabs.len() <= 256
                && self.resources.len() <= 256
                && self.component_schemas.len() <= 64
                && self.resource_schemas.len() <= 64,
            "definition budget exceeded"
        );
        ensure!(
            self.relations.len() <= 8192 && self.scenes.len() <= 128 && self.systems.len() <= 64,
            "graph budget exceeded"
        );
        r.validate_capabilities(self)?;
        for (id, s) in &self.component_schemas {
            valid_id(id)?;
            ensure!(
                !r.components.contains_key(id),
                "cannot shadow native component {id}"
            );
            s.validate_definition()?;
        }
        for (id, s) in &self.resource_schemas {
            valid_id(id)?;
            ensure!(
                !r.resources.contains_key(id),
                "cannot shadow native resource {id}"
            );
            s.validate_definition()?;
        }
        let ids: BTreeMap<String, usize> = self
            .entities
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        ensure!(ids.len() == self.entities.len(), "duplicate entity id");
        for (id, p) in &self.prefabs {
            valid_id(id)?;
            r.validate_components(self, &p.components, &ids)
                .with_context(|| format!("prefabs/{id}"))?;
        }
        let mut entities = vec![];
        let mut columns = BTreeMap::new();
        let mut instances = 0;
        let mut cameras = 0;
        for e in &self.entities {
            valid_id(&e.id)?;
            if let Some(label) = &e.label {
                crate::local::at(
                    &format!("/entities/@{}/label", crate::local::escape(&e.id)),
                    (|| {
                        ensure!(
                            !label.trim().is_empty()
                                && label.len() <= 128
                                && !label.chars().any(char::is_control),
                            "label must contain 1..128 bytes of display text"
                        );
                        Ok(())
                    })(),
                )?;
            }
            ensure!(
                e.tags.len() <= 32 && e.tags.iter().all(|t| t.len() <= 64),
                "tag budget exceeded"
            );
            let mut c = match &e.prefab {
                Some(id) => self
                    .prefabs
                    .get(id)
                    .with_context(|| format!("missing prefab {id}"))?
                    .components
                    .clone(),
                None => BTreeMap::new(),
            };
            for (k, v) in &e.components {
                if let Some(old) = c.get_mut(k) {
                    merge_json(old, v);
                } else {
                    c.insert(k.clone(), v.clone());
                }
            }
            r.validate_components(self, &c, &ids).map_err(|err| {
                Diagnostic::error(
                    "invalid_component",
                    format!("entities/{}/components", e.id),
                    format!("{err:#}"),
                )
            })?;
            let transform = decode::<Transform>(&c, "core.transform")?;
            let mesh = decode::<RenderMesh>(&c, "render.mesh")?;
            let material = decode::<MaterialBinding>(&c, "render.material")?;
            let camera = decode::<Camera>(&c, "render.camera")?;
            let light = decode::<crate::light::RenderLight>(&c, "render.light")?;
            let trigger = decode::<crate::trigger::TriggerZone>(&c, "trigger.zone")?;
            let path = decode::<crate::path::PathSpline>(&c, "path.spline")?;
            let follow = decode::<crate::path::PathFollow>(&c, "path.follow")?;
            let nav_agent = decode::<crate::navigation::NavAgent>(&c, "nav.agent")?;
            let interact = decode::<crate::interact::InteractTarget>(&c, "interact.target")?;
            let tween = decode::<crate::tween::TweenClips>(&c, "tween.clips")?;
            let audio_source = decode::<crate::audio::AudioSource>(&c, "audio.source")?;
            let audio_listener = decode::<crate::audio::AudioListener>(&c, "audio.listener")?;
            let camera_follow = decode::<crate::camera::CameraFollow>(&c, "camera.follow")?;
            let collider = decode::<Collider>(&c, "physics.collider")?;
            let health = decode::<Health>(&c, "core.health")?;
            let motion = decode::<Motion>(&c, "core.motion")?;
            ensure!(
                (mesh.is_none()
                    && camera.is_none()
                    && light.is_none()
                    && trigger.is_none()
                    && path.is_none()
                    && follow.is_none()
                    && nav_agent.is_none()
                    && interact.is_none()
                    && camera_follow.is_none()
                    && collider.is_none()
                    && motion.is_none())
                    || transform.is_some(),
                "spatial component requires core.transform on {}",
                e.id
            );
            instances += mesh.as_ref().map_or(0, |m| m.parts.len());
            cameras += usize::from(camera.as_ref().is_some_and(|c| c.active));
            for (name, s) in &self.component_schemas {
                if let Some(v) = c.get(name) {
                    columns
                        .entry(name.clone())
                        .or_insert_with(|| ComponentTable::new(s))
                        .push(entities.len(), s, v, &ids)?;
                }
            }
            entities.push(CompiledEntity {
                id: e.id.clone(),
                tags: e.tags.clone(),
                components: c,
                transform,
                mesh,
                material,
                camera,
                light,
                trigger,
                path,
                follow,
                nav_agent,
                interact,
                tween,
                audio_source,
                audio_listener,
                camera_follow,
                collider,
                health,
                motion,
            });
        }
        crate::light::validate_budget(
            entities
                .iter()
                .filter_map(|e: &CompiledEntity| e.light.as_ref().map(|l| l.kind)),
        )
        .map_err(|err| Diagnostic::error("light_budget", "/entities", format!("{err:#}")))?;
        let zones: Vec<(&str, &crate::trigger::TriggerZone)> = entities
            .iter()
            .filter_map(|e| Some((e.id.as_str(), e.trigger.as_ref()?)))
            .collect();
        crate::trigger::validate_world(self, &zones)?;
        let interactables: Vec<(&str, &crate::interact::InteractTarget)> = entities
            .iter()
            .filter_map(|e| Some((e.id.as_str(), e.interact.as_ref()?)))
            .collect();
        crate::interact::validate_world(self, &interactables)?;
        let sources: Vec<_> = entities
            .iter()
            .filter_map(|e| {
                Some((
                    e.id.as_str(),
                    e.audio_source.as_ref()?,
                    e.transform.is_some(),
                ))
            })
            .collect();
        let listeners: Vec<_> = entities
            .iter()
            .filter_map(|e| {
                Some((
                    e.id.as_str(),
                    e.audio_listener.as_ref()?,
                    e.transform.is_some(),
                ))
            })
            .collect();
        crate::audio::validate_world(self, &sources, &listeners)?;
        ensure!(
            instances <= MAX_INSTANCES,
            "render instance budget exceeded"
        );
        ensure!(cameras <= MAX_CAMERAS, "active camera budget exceeded");
        for (id, v) in &self.resources {
            r.validate_resource(self, id, v, &ids)?;
        }
        let mut parents = vec![None; entities.len()];
        let mut edges = BTreeSet::new();
        for rel in &self.relations {
            valid_id(&rel.kind)?;
            let &a = ids.get(&rel.source).context("missing relation source")?;
            let &b = ids.get(&rel.target).context("missing relation target")?;
            ensure!(edges.insert((a, b, rel.kind.clone())), "duplicate relation");
            if rel.kind == "parent" {
                ensure!(parents[a].replace(b).is_none(), "multiple parents");
            }
        }
        fn visit(
            i: usize,
            p: &[Option<usize>],
            marks: &mut [u8],
            out: &mut Vec<usize>,
        ) -> Result<()> {
            let mut cursor = Some(i);
            let mut trail = vec![];
            while let Some(node) = cursor {
                ensure!(marks[node] != 1, "parent cycle");
                if marks[node] == 2 {
                    break;
                }
                marks[node] = 1;
                trail.push(node);
                cursor = p[node];
            }
            while let Some(node) = trail.pop() {
                marks[node] = 2;
                out.push(node);
            }
            Ok(())
        }
        let mut transform_order = vec![];
        let mut marks = vec![0; entities.len()];
        for i in 0..entities.len() {
            visit(i, &parents, &mut marks, &mut transform_order)?;
        }
        let mut scene_ids = BTreeSet::new();
        let mut mentioned = BTreeSet::new();
        let mut active = BTreeSet::new();
        for s in &self.scenes {
            valid_id(&s.id)?;
            ensure!(scene_ids.insert(&s.id), "duplicate scene");
            let mut local = BTreeSet::new();
            for id in &s.entities {
                let &i = ids.get(id).context("scene references missing entity")?;
                ensure!(local.insert(i), "duplicate scene member");
                mentioned.insert(i);
                if s.active {
                    active.insert(i);
                }
            }
        }
        let enabled = (0..entities.len())
            .map(|i| !mentioned.contains(&i) || active.contains(&i))
            .collect();
        crate::path::validate_world(self, r, &entities, &ids, &parents)?;
        crate::navigation::validate_world(self, r, &entities, &ids, &parents)?;
        crate::tween::validate_world(self, r, &entities, &ids, &parents)?;
        crate::camera::validate_world(self, r, &entities, &ids, &parents)?;
        let schedule = r.schedule(self)?;
        r.validate_world(self)?;
        Ok(CompiledWorld {
            source: self.clone(),
            entities,
            indices: ids,
            parents,
            transform_order,
            columns,
            enabled,
            schedule,
        })
    }
}
fn decode<T: serde::de::DeserializeOwned>(c: &Components, k: &str) -> Result<Option<T>> {
    c.get(k)
        .map(|v| serde_json::from_value(v.clone()).map_err(Into::into))
        .transpose()
}
pub fn valid_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .chars()
                .all(|c| c.is_alphanumeric() || "._/-".contains(c)),
        "invalid id: {id}"
    );
    Ok(())
}
pub fn merge_json(a: &mut Value, b: &Value) {
    if let (Some(a), Some(b)) = (a.as_object_mut(), b.as_object()) {
        for (k, v) in b {
            if let Some(old) = a.get_mut(k) {
                merge_json(old, v);
            } else {
                a.insert(k.clone(), v.clone());
            }
        }
    } else {
        *a = b.clone();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub prefab: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub components: Components,
}
impl Entity {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: None,
            prefab: None,
            tags: vec![],
            components: BTreeMap::new(),
        }
    }
    pub fn set<T: Serialize>(&mut self, k: &str, v: T) -> Result<()> {
        self.components.insert(k.into(), serde_json::to_value(v)?);
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Prefab {
    pub components: Components,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub source: String,
    pub kind: String,
    pub target: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub id: String,
    pub entities: Vec<String>,
    pub active: bool,
}

fn rotation() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}
fn scale() -> [f32; 3] {
    [1.0; 3]
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default = "rotation")]
    pub rotation: [f32; 4],
    #[serde(default = "scale")]
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: rotation(),
            scale: scale(),
        }
    }
}
impl Transform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::from_array(self.scale),
            Quat::from_array(self.rotation),
            Vec3::from_array(self.position),
        )
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.position
                .iter()
                .chain(self.rotation.iter())
                .chain(self.scale.iter())
                .all(|v| v.is_finite()),
            "non-finite transform"
        );
        ensure!(
            self.position.iter().all(|v| v.abs() <= 1_000_000.0)
                && self.scale.iter().all(|&v| (0.0001..=10000.0).contains(&v)),
            "transform out of range"
        );
        ensure!(
            (Quat::from_array(self.rotation).length_squared() - 1.0).abs() < 0.001,
            "rotation must be a unit quaternion"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MeshPart {
    #[serde(default)]
    pub transform: Transform,
    pub color: [f32; 3],
    #[serde(default)]
    pub emission: f32,
    #[serde(default)]
    pub grid: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenderMesh {
    pub parts: Vec<MeshPart>,
}
impl RenderMesh {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.parts.is_empty() && self.parts.len() <= 1024,
            "mesh part budget is 1..1024"
        );
        for p in &self.parts {
            p.transform.validate()?;
            ensure!(
                p.color
                    .iter()
                    .all(|&v| v.is_finite() && (0.0..=1.0).contains(&v))
                    && p.emission.is_finite()
                    && (0.0..=16.0).contains(&p.emission),
                "invalid material"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Projection {
    Perspective { fov_degrees: f32 },
    Orthographic { height: f32 },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub projection: Projection,
    pub near: f32,
    pub far: f32,
    pub viewport: [f32; 4],
    pub active: bool,
}
impl Camera {
    pub fn perspective() -> Self {
        Self {
            projection: Projection::Perspective { fov_degrees: 75.0 },
            near: 0.04,
            far: 180.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            active: true,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.near.is_finite()
                && self.far.is_finite()
                && self.near > 0.0
                && self.far > self.near
                && self.far <= 100000.0,
            "invalid camera clipping planes"
        );
        match self.projection {
            Projection::Perspective { fov_degrees } => ensure!(
                fov_degrees.is_finite() && (5.0..=160.0).contains(&fov_degrees),
                "invalid fov"
            ),
            Projection::Orthographic { height } => ensure!(
                height.is_finite() && (0.01..=100000.0).contains(&height),
                "invalid orthographic height"
            ),
        };
        let [x, y, w, h] = self.viewport;
        ensure!(
            self.viewport.iter().all(|v| v.is_finite())
                && x >= 0.0
                && y >= 0.0
                && w >= 0.001
                && h >= 0.001
                && x + w <= 1.00001
                && y + h <= 1.00001,
            "invalid viewport"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum ColliderShape {
    Box { half_extents: [f32; 3] },
    Capsule { half_height: f32, radius: f32 },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Collider {
    pub shape: ColliderShape,
}
impl Collider {
    pub fn validate(&self) -> Result<()> {
        let v = match self.shape {
            ColliderShape::Box { half_extents } => half_extents.to_vec(),
            ColliderShape::Capsule {
                half_height,
                radius,
            } => vec![half_height, radius],
        };
        ensure!(
            v.iter()
                .all(|&n| n.is_finite() && (0.001..=10000.0).contains(&n)),
            "invalid collider dimensions"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub value: f32,
    pub max: f32,
}
impl Health {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.max.is_finite()
                && self.value.is_finite()
                && self.max > 0.0
                && self.value >= 0.0
                && self.value <= self.max,
            "health requires 0 <= value <= max, max > 0"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Motion {
    #[serde(default)]
    pub velocity: [f32; 3],
    #[serde(default)]
    pub angular_velocity: [f32; 3],
}
impl Motion {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.velocity
                .iter()
                .chain(self.angular_velocity.iter())
                .all(|v| v.is_finite() && v.abs() <= 10000.0),
            "invalid motion"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenderSettings {
    pub accent: [f32; 3],
    pub shadow_extent: f32,
}
impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            accent: [0.18, 0.93, 0.78],
            shadow_extent: 33.0,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InputBindings {
    #[serde(default)]
    pub axes: BTreeMap<String, AxisBinding>,
    #[serde(default)]
    pub buttons: BTreeMap<String, String>,
    #[serde(default)]
    pub capture_pointer: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AxisBinding {
    pub positive: String,
    pub negative: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Bool,
    Int,
    Float,
    String,
    Vec3,
    Entity,
    Asset,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FieldSchema {
    pub ty: FieldType,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecordSchema {
    pub fields: BTreeMap<String, FieldSchema>,
}
impl RecordSchema {
    pub fn validate_definition(&self) -> Result<()> {
        ensure!(
            !self.fields.is_empty() && self.fields.len() <= 64,
            "field budget is 1..64"
        );
        for (id, f) in &self.fields {
            valid_id(id)?;
            ensure!(
                f.min.is_none_or(f64::is_finite) && f.max.is_none_or(f64::is_finite),
                "non-finite bound"
            );
            if let (Some(a), Some(b)) = (f.min, f.max) {
                ensure!(a <= b, "min exceeds max");
            }
            if let Some(v) = &f.default {
                f.validate(v, &BTreeMap::new(), false)?;
            }
        }
        Ok(())
    }
    pub fn validate(&self, v: &Value, ids: &BTreeMap<String, usize>) -> Result<()> {
        let obj = v.as_object().context("expected record object")?;
        ensure!(
            obj.keys().all(|k| self.fields.contains_key(k)),
            "unknown field"
        );
        for (id, f) in &self.fields {
            match obj.get(id).or(f.default.as_ref()) {
                Some(v) => f
                    .validate(v, ids, true)
                    .with_context(|| format!("field {id}"))?,
                None => ensure!(f.optional, "missing field {id}"),
            }
        }
        Ok(())
    }
}
impl FieldSchema {
    fn validate(&self, v: &Value, ids: &BTreeMap<String, usize>, refs: bool) -> Result<()> {
        let valid = match self.ty {
            FieldType::Bool => v.is_boolean(),
            FieldType::Int => v.as_i64().is_some(),
            FieldType::Float => v
                .as_f64()
                .is_some_and(|n| n.is_finite() && (n as f32).is_finite()),
            FieldType::String | FieldType::Asset => v.as_str().is_some_and(|s| s.len() <= 4096),
            FieldType::Entity => v.as_str().is_some_and(|s| !refs || ids.contains_key(s)),
            FieldType::Vec3 => v.as_array().is_some_and(|a| {
                a.len() == 3
                    && a.iter().all(|v| {
                        v.as_f64()
                            .is_some_and(|n| n.is_finite() && (n as f32).is_finite())
                    })
            }),
        };
        ensure!(valid, "incorrect field type or missing entity reference");
        if let Some(n) = v.as_f64() {
            ensure!(
                self.min.is_none_or(|m| n >= m) && self.max.is_none_or(|m| n <= m),
                "value outside range"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub enum Column {
    Bool(Vec<Option<bool>>),
    Int(Vec<Option<i64>>),
    Float(Vec<Option<f32>>),
    String(Vec<Option<String>>),
    Vec3(Vec<Option<[f32; 3]>>),
    Entity(Vec<Option<usize>>),
}
impl Column {
    pub fn preserve_scalar(&mut self, row: usize, old: &Self, old_row: usize) -> bool {
        fn copy<T: Clone>(dst: &mut [T], i: usize, src: &[T], j: usize) {
            dst[i] = src[j].clone();
        }
        match (self, old) {
            (Self::Bool(a), Self::Bool(b)) => copy(a, row, b, old_row),
            (Self::Int(a), Self::Int(b)) => copy(a, row, b, old_row),
            (Self::Float(a), Self::Float(b)) => copy(a, row, b, old_row),
            (Self::String(a), Self::String(b)) => copy(a, row, b, old_row),
            (Self::Vec3(a), Self::Vec3(b)) => copy(a, row, b, old_row),
            _ => return false,
        }
        true
    }
}
#[derive(Clone, Debug)]
pub struct ComponentTable {
    pub entities: Vec<usize>,
    pub fields: BTreeMap<String, Column>,
}
impl ComponentTable {
    pub fn new(s: &RecordSchema) -> Self {
        Self {
            entities: vec![],
            fields: s
                .fields
                .iter()
                .map(|(id, f)| {
                    (
                        id.clone(),
                        match f.ty {
                            FieldType::Bool => Column::Bool(vec![]),
                            FieldType::Int => Column::Int(vec![]),
                            FieldType::Float => Column::Float(vec![]),
                            FieldType::String | FieldType::Asset => Column::String(vec![]),
                            FieldType::Vec3 => Column::Vec3(vec![]),
                            FieldType::Entity => Column::Entity(vec![]),
                        },
                    )
                })
                .collect(),
        }
    }
    pub fn push(
        &mut self,
        e: usize,
        s: &RecordSchema,
        v: &Value,
        ids: &BTreeMap<String, usize>,
    ) -> Result<()> {
        self.entities.push(e);
        for (id, f) in &s.fields {
            let v = v.get(id).or(f.default.as_ref());
            match self.fields.get_mut(id).unwrap() {
                Column::Bool(c) => c.push(v.and_then(Value::as_bool)),
                Column::Int(c) => c.push(v.and_then(Value::as_i64)),
                Column::Float(c) => c.push(v.and_then(Value::as_f64).map(|n| n as f32)),
                Column::String(c) => c.push(v.and_then(Value::as_str).map(str::to_owned)),
                Column::Vec3(c) => {
                    c.push(v.map(|v| serde_json::from_value(v.clone())).transpose()?)
                }
                Column::Entity(c) => {
                    c.push(v.and_then(Value::as_str).and_then(|s| ids.get(s)).copied())
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct CompiledEntity {
    pub id: String,
    pub tags: Vec<String>,
    pub components: Components,
    pub transform: Option<Transform>,
    pub mesh: Option<RenderMesh>,
    pub material: Option<MaterialBinding>,
    pub camera: Option<Camera>,
    pub light: Option<crate::light::RenderLight>,
    pub trigger: Option<crate::trigger::TriggerZone>,
    pub path: Option<crate::path::PathSpline>,
    pub follow: Option<crate::path::PathFollow>,
    pub nav_agent: Option<crate::navigation::NavAgent>,
    pub interact: Option<crate::interact::InteractTarget>,
    pub tween: Option<crate::tween::TweenClips>,
    pub audio_source: Option<crate::audio::AudioSource>,
    pub audio_listener: Option<crate::audio::AudioListener>,
    pub camera_follow: Option<crate::camera::CameraFollow>,
    pub collider: Option<Collider>,
    pub health: Option<Health>,
    pub motion: Option<Motion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MaterialBinding {
    pub id: String,
}
#[derive(Clone, Debug)]
pub struct CompiledWorld {
    pub source: World,
    pub entities: Vec<CompiledEntity>,
    pub indices: BTreeMap<String, usize>,
    pub parents: Vec<Option<usize>>,
    pub transform_order: Vec<usize>,
    pub columns: BTreeMap<String, ComponentTable>,
    pub enabled: Vec<bool>,
    pub schedule: Vec<crate::SystemDescriptor>,
}
impl CompiledWorld {
    pub fn len(&self) -> usize {
        self.entities.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}
