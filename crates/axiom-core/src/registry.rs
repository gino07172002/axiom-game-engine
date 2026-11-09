use crate::*;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub path: String,
    pub message: String,
}
impl Diagnostic {
    pub fn error(
        code: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> anyhow::Error {
        anyhow::Error::new(Self {
            code: code.into(),
            path: path.into(),
            message: message.into(),
        })
    }
}
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}: {}", self.code, self.path, self.message)
    }
}
impl std::error::Error for Diagnostic {}
pub fn error_response(e: &anyhow::Error) -> Value {
    let d = e
        .downcast_ref::<Diagnostic>()
        .cloned()
        .unwrap_or_else(|| Diagnostic {
            code: "invalid_request".into(),
            path: "/".into(),
            message: format!("{e:#}"),
        });
    json!({"ok":false,"error":format!("{e:#}"),"diagnostics":[d]})
}
pub type ValueValidator = fn(&Value) -> Result<()>;
pub type WorldValidator = fn(&World) -> Result<()>;
pub type Migrator = fn(Value) -> Result<World>;
pub type EditHandler = fn(&mut World, &str, &Value) -> Result<bool>;
pub type SummaryProvider = fn(&World) -> Value;
pub type ResourceMerger = fn(&Value, &Value) -> Result<Value>;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NativeEvent {
    pub version: u32,
    pub publisher: String,
    pub fields: BTreeMap<String, crate::gameplay::ScalarType>,
}
#[derive(Clone)]
pub struct TypeDescriptor {
    pub capability: String,
    pub schema: Value,
    pub validate: ValueValidator,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SystemDescriptor {
    pub id: String,
    pub capability: String,
    pub phase: u32,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub after: Vec<String>,
}
#[derive(Clone, Default)]
pub struct Registry {
    pub native_events: BTreeMap<String, NativeEvent>,
    pub event_consumers: BTreeSet<String>,
    pub resource_mergers: BTreeMap<String, ResourceMerger>,
    pub actions: BTreeMap<String, TypeDescriptor>,
    pub capabilities: BTreeMap<String, u32>,
    pub components: BTreeMap<String, TypeDescriptor>,
    pub resources: BTreeMap<String, TypeDescriptor>,
    pub systems: BTreeMap<String, SystemDescriptor>,
    pub topics: BTreeMap<String, Value>,
    pub migrations: BTreeMap<u32, Migrator>,
    pub world_validators: Vec<WorldValidator>,
    pub package_resolvers: Vec<fn(&std::path::Path, &mut World) -> Result<()>>,
    pub edit_handlers: Vec<EditHandler>,
    pub summaries: Vec<SummaryProvider>,
    /// Component -> the system that writes the transform of entities carrying it. An entity
    /// may have at most one mover; `path.follow` or `nav.agent` next to one of these is an
    /// authoring error.
    pub movers: BTreeMap<String, String>,
}
impl Registry {
    pub fn core() -> Self {
        let mut r = Self::default();
        for id in [
            "core", "render", "physics", "motion", "trigger", "path", "nav", "interact", "tween",
            "audio", "camera",
        ] {
            r.capabilities.insert(id.into(), 1);
        }
        fn typed<T: serde::de::DeserializeOwned>(v: &Value, f: fn(&T) -> Result<()>) -> Result<()> {
            f(&serde_json::from_value(v.clone())?)
        }
        r.component(
            "core.transform",
            "core",
            json!({"position":"vec3","rotation":"unit quaternion xyzw","scale":"positive vec3"}),
            |v| typed(v, Transform::validate),
        );
        r.component(
            "nav.blocker",
            "core",
            json!({"half":"[hx,hz] meters 0.01..500","offset":"optional [ox,oz] from the entity position"}),
            |v| typed(v, crate::blocker::NavBlocker::validate),
        );
        r.component(
            "core.health",
            "core",
            json!({"value":"number >= 0","max":"number > 0"}),
            |v| typed(v, Health::validate),
        );
        r.component(
            "core.motion",
            "motion",
            json!({"velocity":"vec3","angular_velocity":"radians/second vec3"}),
            |v| typed(v, Motion::validate),
        );
        r.component(
            "render.mesh",
            "render",
            json!({"parts":"1..1024 cube parts: transform, color[3], emission, grid"}),
            |v| typed(v, RenderMesh::validate),
        );
        r.component("render.camera","render",json!({"projection":"{kind:perspective,fov_degrees} | {kind:orthographic,height}","near":"positive number","far":"number > near","viewport":"normalized [x,y,width,height]","active":"bool"}),|v|typed(v,Camera::validate));
        r.component(
            "render.light",
            "render",
            crate::light::light_schema(),
            |v| typed(v, crate::light::RenderLight::validate),
        );
        r.resource(
            "render.environment",
            "render",
            crate::light::environment_schema(),
            |v| typed(v, crate::light::RenderEnvironment::validate),
        );
        r.component(
            "trigger.zone",
            "trigger",
            crate::trigger::trigger_schema(),
            |v| typed(v, crate::trigger::TriggerZone::validate),
        );
        r.component("path.spline", "path", crate::path::spline_schema(), |v| {
            typed(v, crate::path::PathSpline::validate)
        });
        r.component("path.follow", "path", crate::path::follow_schema(), |v| {
            typed(v, crate::path::PathFollow::validate)
        });
        r.component("nav.agent", "nav", crate::navigation::agent_schema(), |v| {
            typed(v, crate::navigation::NavAgent::validate)
        });
        r.resource(
            "nav.grid",
            "nav",
            crate::navigation::settings_schema(),
            |v| typed(v, crate::navigation::NavSettings::validate),
        );
        r.component(
            "interact.target",
            "interact",
            crate::interact::target_schema(),
            |v| typed(v, crate::interact::InteractTarget::validate),
        );
        r.component("tween.clips", "tween", crate::tween::clips_schema(), |v| {
            typed(v, crate::tween::TweenClips::validate)
        });
        r.resource(
            "audio.library",
            "audio",
            crate::audio::library_schema(),
            |v| typed(v, crate::audio::AudioLibrary::validate),
        );
        r.component(
            "audio.source",
            "audio",
            crate::audio::source_schema(),
            |v| typed(v, crate::audio::AudioSource::validate),
        );
        r.component(
            "audio.listener",
            "audio",
            crate::audio::listener_schema(),
            |v| typed(v, crate::audio::AudioListener::validate),
        );
        r.component(
            "camera.follow",
            "camera",
            crate::camera::follow_schema(),
            |v| typed(v, crate::camera::CameraFollow::validate),
        );
        r.movers
            .insert("core.motion".into(), "motion.integrate".into());
        r.movers.insert("nav.agent".into(), "nav.steer".into());
        r.movers.insert("camera.follow".into(), "camera.rig".into());
        r.component(
            "physics.collider",
            "physics",
            json!({"shape":"box + half_extents[3] | capsule + half_height, radius"}),
            |v| typed(v, Collider::validate),
        );
        r.resource(
            "render.settings",
            "render",
            json!({"accent":"rgb[3]","shadow_extent":"positive number"}),
            |v| {
                let s: RenderSettings = serde_json::from_value(v.clone())?;
                ensure!(
                    s.accent
                        .iter()
                        .all(|&v| v.is_finite() && (0.0..=1.0).contains(&v))
                        && s.shadow_extent.is_finite()
                        && (1.0..=1000.0).contains(&s.shadow_extent),
                    "invalid render settings"
                );
                Ok(())
            },
        );
        r.resource("input.bindings","core",json!({"axes":"action -> positive/negative key names","buttons":"action -> key name","capture_pointer":"bool"}),|v|{let b:InputBindings=serde_json::from_value(v.clone())?;ensure!(b.axes.len()+b.buttons.len()<=64,"input binding budget exceeded");ensure!(b.buttons.iter().all(|(k,v)|k.len()<=64&&v.len()<=64)&&b.axes.iter().all(|(k,v)|k.len()<=64&&v.positive.len()<=64&&v.negative.len()<=64),"invalid input binding");Ok(())});
        r.systems.insert(
            "motion.integrate".into(),
            SystemDescriptor {
                id: "motion.integrate".into(),
                capability: "motion".into(),
                phase: 10,
                reads: vec!["core.motion".into()],
                writes: vec!["core.transform".into()],
                after: vec![],
            },
        );
        r.systems.insert(
            "trigger.detect".into(),
            SystemDescriptor {
                id: "trigger.detect".into(),
                capability: "trigger".into(),
                // After the movers (fsm 8, motion 10, rts 15, action/fps 20), before animation.
                phase: 25,
                reads: vec!["trigger.zone".into(), "core.transform".into()],
                writes: vec!["runtime.events".into()],
                after: vec![],
            },
        );
        r.systems.insert(
            "interact.focus".into(),
            SystemDescriptor {
                id: "interact.focus".into(),
                capability: "interact".into(),
                // After behavior.execute (5), before fsm.step (8) and the movers: focus uses the
                // positions the last frame showed (the timing of FSM near/far conditions).
                phase: 7,
                reads: vec![
                    "interact.target".into(),
                    "core.transform".into(),
                    "input.bindings".into(),
                ],
                writes: vec!["runtime.events".into()],
                after: vec![],
            },
        );
        r.systems.insert(
            "path.move".into(),
            SystemDescriptor {
                id: "path.move".into(),
                capability: "path".into(),
                // After motion (10) so a moving path parent is current; before rts (15) and
                // trigger.detect (25) so zones see the followers' new positions.
                phase: 12,
                reads: vec![
                    "path.spline".into(),
                    "path.follow".into(),
                    "core.transform".into(),
                ],
                writes: vec!["core.transform".into(), "runtime.events".into()],
                after: vec![],
            },
        );
        r.systems.insert(
            "nav.steer".into(),
            SystemDescriptor {
                id: "nav.steer".into(),
                capability: "nav".into(),
                // After path.move (12) so entity targets that follow paths are current;
                // before rts (15) and trigger.detect (25) so zones see the agents' new positions.
                phase: 13,
                reads: vec![
                    "nav.agent".into(),
                    "nav.blocker".into(),
                    "core.transform".into(),
                ],
                writes: vec!["core.transform".into(), "runtime.events".into()],
                after: vec![],
            },
        );
        r.systems.insert(
            "tween.animate".into(),
            SystemDescriptor {
                id: "tween.animate".into(),
                capability: "tween".into(),
                // After the authored movers of other entities (fsm 8, motion 10, path 12,
                // nav 13) so a tweened child sees its parent of this step; before rts (15),
                // action/fps (20) and trigger.detect (25), which see the tweened poses.
                phase: 14,
                reads: vec!["tween.clips".into()],
                writes: vec![
                    "core.transform".into(),
                    "render.light".into(),
                    "render.mesh".into(),
                    "runtime.events".into(),
                ],
                after: vec![],
            },
        );
        r.systems.insert(
            "audio.spatial".into(),
            SystemDescriptor {
                id: "audio.spatial".into(),
                capability: "audio".into(),
                // Last: every mover, tween, camera rig (action/fps 20) and trigger (25) has
                // written this step's poses, and audio.events (40) its cues.
                phase: 45,
                reads: vec![
                    "audio.source".into(),
                    "audio.listener".into(),
                    "audio.library".into(),
                    "core.transform".into(),
                ],
                writes: vec!["audio.voices".into()],
                after: vec![],
            },
        );
        r.systems.insert(
            "camera.rig".into(),
            SystemDescriptor {
                id: "camera.rig".into(),
                capability: "camera".into(),
                // After every mover and camera rig of gameplay controllers (action.simulate
                // runs after animation 30 / audio.events 40), so the camera sees this step's
                // target pose; before audio.spatial (45), whose fallback listener is the
                // active camera.
                phase: 44,
                reads: vec![
                    "camera.follow".into(),
                    "render.camera".into(),
                    "core.transform".into(),
                    "physics.collider".into(),
                    "nav.blocker".into(),
                    "render.mesh".into(),
                ],
                writes: vec!["core.transform".into(), "render.camera".into()],
                after: vec![],
            },
        );
        crate::gameplay::register(&mut r);
        r.resource_mergers.insert("input.bindings".into(), |a, b| {
            crate::package::merge_resource(a, b, &["axes", "buttons"], &[])
        });
        r.topics.insert("packages".into(),json!({
            "format_version":1,"content_version":"positive integer; dependency pins require exact equality",
            "dependencies":"package ID -> {path,content_version,fingerprint?}; local paths stay inside the root package",
            "exports":{"events":"ID -> {version,publisher,fields}","resources":"ID -> {version,owner}"},
            "imports":{"events":"ID -> {package,version,consumer,fields}","resources":"ID -> {package,version,consumer}"},
            "composition":"stable IDs must be unique; installed resource types opt into disjoint map/list merges",
            "query":{"op":"query","scope":"packages"},"lock":"World.packages persists the resolved graph and source fingerprints",
            "fingerprint":"FNV source identity covers manifest and World JSON, including the declared asset digest inventory",
            "assets":"optional relative filename -> SHA-256; checked at import, 256 files, 32 MiB each, 128 MiB across the graph"
        }));
        r.topics
            .insert("local".into(), crate::discovery::local_schema());
        r.topics.insert(
            "acl".into(),
            json!({"op":"acl","request":{"op":"acl","text":"script","preview":"optional bool","expected_rev":"optional CAS revision"},"cli":"axiom acl -e \"select enemy/* fields core.health/value limit 5\" | axiom acl FILE | stdin","grammar":crate::acl::GRAMMAR}),
        );
        r.topics.insert(
            "narrative.graph".into(),
            crate::discovery::gameplay_schema(),
        );
        r.topics.insert("observe_delta".into(),json!({"op":"observe_delta","cursor":"optional opaque cursor from previous response","fields":"optional array of JSON pointers; default excludes timing noise and UI text","trace_limit":"0..128; optional recent trace records","max_bytes":"256..65536; default 8192","responses":"snapshot + reset reason, or changes + cursor; expired cursors reset explicitly"}));
        r.topics.insert("editor".into(),json!({
            "commands":"discover editor_commands; run with {\"op\":\"editor_view\",\"command\":ID}",
            "read":{"op":"scene_tree","query":{"parent":"optional immediate parent ID","roots_only":"optional; mutually exclusive with parent","scene":"optional scene ID","text":"optional label or ID substring","limit":"1..128, default 32","max_bytes":"256..65536, default 8192","cursor":"next_cursor from same revision and filters"}},
            "inspect":{"op":"select","query":{"from":"entities","ids":["object-id"],"fields":["label","components/core.transform"]}},
            "edits":[
                {"kind":"reparent","entity":"object-id","parent":null,"keep_world":true},
                {"kind":"duplicate_entity","source":"object-id","id":"new-id"},
                {"kind":"delete_entity","id":"object-id"},
                {"kind":"move_entity","id":"object-id","before":"sibling-id, or null for the end"},
                {"kind":"set_scene_membership","scene":"scene-id","entity":"object-id","member":true}
            ],
            "rules":["label is mutable; ID is stable","sibling order is the authored entity order; move_entity reorders it","duplicate copies one entity, its parent and scene memberships","delete removes incident relations and memberships; children become roots with local transforms","keep_world can reject shear or out-of-range local TRS"],
            "write":"Use preview or apply with expected_rev; use execute with a request ID for durable retries",
            "observe":{"op":"observe_delta","fields":["/editor","/activation"]},
            "view":{"op":"editor_view","select":["object-id"],"focus":true,"dock":"opens/focuses a tab: hierarchy|scene|game|inspector|world|changes|proposals|history|problems|scenarios|graphs|assets|agent (discover editor_layout, editor_game)","capture":"name -> artifacts/captures/<name>.png on the next frame; refused while a human draft is dirty"},
            "review":{"propose":{"op":"propose","edits":[],"note":"why"},"list":{"op":"proposals","status":"pending"},"resolve":{"op":"resolve_proposal","id":1,"approve":true,"rebase":false}}
        }));
        r.topics.insert("execute".into(),json!({"op":"execute","request_id":"1..128 ASCII letters/digits/-_.:","runtime_session":"required for command/save/load; from observe.runtime_session","request":"one apply/undo/command/save/load; nesting prohibited","guarantees":{"apply_undo":"receipt and revision commit atomically in SQLite, durable across process restarts","runtime":"successful results cached for the host session; foreign sessions rejected; cache never silently evicts"},"failures":"failed operations do not reserve request_id; conflicting successful payloads rejected"}));
        r
    }
    pub fn component(
        &mut self,
        id: &str,
        capability: &str,
        schema: Value,
        validate: ValueValidator,
    ) {
        self.components.insert(
            id.into(),
            TypeDescriptor {
                capability: capability.into(),
                schema,
                validate,
            },
        );
    }
    pub fn resource(
        &mut self,
        id: &str,
        capability: &str,
        schema: Value,
        validate: ValueValidator,
    ) {
        self.resources.insert(
            id.into(),
            TypeDescriptor {
                capability: capability.into(),
                schema,
                validate,
            },
        );
    }
    pub fn action(&mut self, id: &str, capability: &str, schema: Value, validate: ValueValidator) {
        self.actions.insert(
            id.into(),
            TypeDescriptor {
                capability: capability.into(),
                schema,
                validate,
            },
        );
    }
    pub fn decode(&self, v: Value) -> Result<World> {
        let version = v
            .get("schema_version")
            .and_then(Value::as_u64)
            .context("missing numeric schema_version")?;
        if version == 2 {
            Ok(serde_json::from_value(v)?)
        } else if let Some(m) = self.migrations.get(&(version as u32)) {
            m(v)
        } else {
            bail!("unsupported world schema version {version}; no migration installed")
        }
    }
    pub fn validate_capabilities(&self, w: &World) -> Result<()> {
        ensure!(
            w.capabilities.get("core") == Some(&1),
            "world requires core capability version 1"
        );
        for (id, v) in &w.capabilities {
            if self.capabilities.get(id) != Some(v) {
                return Err(Diagnostic::error(
                    "missing_capability",
                    format!("capabilities/{id}"),
                    format!("version {v} not installed"),
                ));
            }
        }
        Ok(())
    }
    fn require(w: &World, d: &TypeDescriptor) -> Result<()> {
        ensure!(
            w.capabilities.get(&d.capability) == Some(&1),
            "component/resource requires capability {}",
            d.capability
        );
        Ok(())
    }
    pub fn validate_components(
        &self,
        w: &World,
        values: &Components,
        ids: &BTreeMap<String, usize>,
    ) -> Result<()> {
        ensure!(values.len() <= 64, "component budget exceeded");
        for (id, v) in values {
            if let Some(d) = self.components.get(id) {
                Self::require(w, d)?;
                (d.validate)(v).with_context(|| id.clone())?;
            } else if let Some(s) = w.component_schemas.get(id) {
                s.validate(v, ids)?;
            } else {
                bail!("unregistered component {id}");
            }
        }
        Ok(())
    }
    pub fn validate_resource(
        &self,
        w: &World,
        id: &str,
        v: &Value,
        ids: &BTreeMap<String, usize>,
    ) -> Result<()> {
        if let Some(d) = self.resources.get(id) {
            Self::require(w, d)?;
            (d.validate)(v).with_context(|| format!("resources/{id}"))
        } else if let Some(s) = w.resource_schemas.get(id) {
            s.validate(v, ids)
        } else {
            bail!("unregistered resource {id}")
        }
    }
    pub fn validate_world(&self, w: &World) -> Result<()> {
        for f in &self.world_validators {
            f(w)?;
        }
        if let Some(value) = w.resources.get("ui.document") {
            let doc: crate::gameplay::UiDocument = serde_json::from_value(value.clone())?;
            for widget in &doc.widgets {
                if let crate::gameplay::Widget::Button {
                    id,
                    command:
                        crate::gameplay::RuntimeCommand::Call {
                            capability,
                            action,
                            args,
                        },
                    ..
                } = widget
                {
                    let path = format!(
                        "{}/command",
                        crate::local::Target::Widget { id: id.clone() }.path()
                    );
                    let descriptor = self
                        .actions
                        .get(&format!("{capability}.{action}"))
                        .ok_or_else(|| {
                            Diagnostic::error("unknown_action", format!("{path}/action"), action)
                        })?;
                    if !w.capabilities.contains_key(capability) {
                        return Err(Diagnostic::error(
                            "missing_capability",
                            format!("{path}/capability"),
                            capability,
                        ));
                    }
                    crate::local::at(&format!("{path}/args"), (descriptor.validate)(args))?;
                }
            }
        }
        Ok(())
    }
    pub fn schedule(&self, w: &World) -> Result<Vec<SystemDescriptor>> {
        let mut pending = BTreeMap::new();
        for id in &w.systems {
            let d = self
                .systems
                .get(id)
                .with_context(|| format!("unregistered system {id}"))?;
            ensure!(
                w.capabilities.contains_key(&d.capability),
                "system {id} requires {}",
                d.capability
            );
            ensure!(
                pending.insert(id.clone(), d.clone()).is_none(),
                "duplicate system {id}"
            );
        }
        for d in pending.values() {
            for after in &d.after {
                ensure!(
                    pending.contains_key(after),
                    "system {} requires scheduled dependency {after}",
                    d.id
                );
            }
        }
        let mut out = vec![];
        let mut done = BTreeSet::new();
        while !pending.is_empty() {
            let mut ready = pending
                .values()
                .filter(|d| d.after.iter().all(|id| done.contains(id)))
                .cloned()
                .collect::<Vec<_>>();
            ready.sort_by(|a, b| (a.phase, &a.id).cmp(&(b.phase, &b.id)));
            let d = ready.first().context("system dependency cycle")?.clone();
            pending.remove(&d.id);
            done.insert(d.id.clone());
            out.push(d);
        }
        Ok(out)
    }
    pub fn schema(&self, topic: Option<&str>) -> Result<Value> {
        match topic {
            None => Ok(
                json!({"engine":"axiom","protocol":2,"world_schema":2,"operations":["schema","discover","query","select","scene_tree","preview","apply","diff","verify","observe","observe_delta","execute","undo","history","command","save","load"],"runtime_operations":["command","save","load"],"capabilities":self.capabilities,"topics":["world","components","resources","systems","actions","events","edits","packages","typed:ID"],"components":self.components.keys().collect::<Vec<_>>(),"resources":self.resources.keys().collect::<Vec<_>>(),"actions":self.actions.keys().collect::<Vec<_>>(),"package_topics":self.topics.keys().collect::<Vec<_>>(),"limits":{"entities":MAX_ENTITIES,"instances":MAX_INSTANCES,"cameras":MAX_CAMERAS,"edits":256,"request_bytes":1048576}}),
            ),
            Some("world") => Ok(
                json!({"example":World::empty("example"),"format":"strict schema v2; unknown fields rejected","entity":{"id":"box","prefab":null,"tags":[],"components":{"core.transform":{"position":[0,0,0]},"core.health":{"value":100,"max":100}}},"custom_schema_example":{"fields":{"active":{"ty":"bool"},"score":{"ty":"int","min":0}}},"scenes":"entities absent from scenes remain active"}),
            ),
            Some("components") => Ok(Value::Object(
                self.components
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            json!({"capability":v.capability,"schema":v.schema}),
                        )
                    })
                    .collect(),
            )),
            Some("resources") => Ok(Value::Object(
                self.resources
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            json!({"capability":v.capability,"schema":v.schema}),
                        )
                    })
                    .collect(),
            )),
            Some("systems") => Ok(json!({"systems":self.systems.values().collect::<Vec<_>>() })),
            Some("events") => Ok(
                json!({"native_events":self.native_events,"consumers":self.event_consumers,"delivery":"bounded batch at the fixed-step boundary; contracts are declared by packages"}),
            ),
            Some("actions") => Ok(Value::Object(
                self.actions
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            json!({"capability":v.capability,"schema":v.schema}),
                        )
                    })
                    .collect(),
            )),
            Some("edits") => Ok(
                json!({"kinds":["set_field","put_object","remove_object","set","put_entity","remove_entity","reparent","duplicate_entity","delete_entity","move_entity","set_scene_membership","set_component","remove_component","put_prefab","set_resource","remove_resource","define_component","define_resource","put_relation","remove_relation","put_scene","set_systems","replace_world"],"set":"JSON pointer beginning with /, or id/title; legacy paths require adapter","example":{"op":"preview","expected_rev":1,"edits":[{"kind":"put_entity","entity":{"id":"box","components":{"core.transform":{"position":[1,0,0]}}}}]},"activation":"authoring commit is distinct from runtime applied revision"}),
            ),
            Some(name) if name.starts_with("typed:") => {
                let id = &name["typed:".len()..];
                crate::typed_schema::typed_schema(self, &World::empty("discover"), id)
                    .with_context(|| format!("unknown component or resource {id}"))
            }
            Some(name) => self
                .topics
                .get(name)
                .cloned()
                .or_else(|| self.components.get(name).map(|d| d.schema.clone()))
                .or_else(|| self.resources.get(name).map(|d| d.schema.clone()))
                .or_else(|| self.actions.get(name).map(|d| d.schema.clone()))
                .with_context(|| format!("unknown schema topic {name}")),
        }
    }
}
