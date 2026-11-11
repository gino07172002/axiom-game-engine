//! Fixed-step generic executor. Installed system factories provide optional gameplay.
use crate::{
    presentation::{Instance, RenderFrame, RenderView},
    ui::{Action, UiState},
};
use anyhow::{ensure, Context, Result};
use axiom_core::*;
use glam::{Mat4, Quat, Vec3};
use serde::Serialize;
use serde_json::Value;
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
};
pub const FIXED_DT: f32 = 1.0 / 120.0;
#[derive(Default, Clone)]
pub struct ActionMap(pub BTreeMap<String, f32>);
impl ActionMap {
    pub fn axis(&self, name: &str) -> f32 {
        self.0.get(name).copied().unwrap_or(0.0)
    }
    pub fn pressed(&self, name: &str) -> bool {
        self.axis(name) > 0.0
    }
    pub fn from_bindings(bindings: &InputBindings, keys: &BTreeSet<String>) -> Self {
        let mut a = Self::default();
        for (name, b) in &bindings.axes {
            a.0.insert(
                name.clone(),
                f32::from(keys.contains(&b.positive)) - f32::from(keys.contains(&b.negative)),
            );
        }
        for (name, key) in &bindings.buttons {
            a.0.insert(name.clone(), f32::from(keys.contains(key)));
        }
        a
    }
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct RuntimeEntityId {
    pub slot: u32,
    pub generation: u64,
}
/// World matrices of `len` slots of `compiled`, parents before children, from each slot's
/// local matrix (slots outside the transform order stay identity).
pub(crate) fn compose_matrices(
    compiled: &CompiledWorld,
    len: usize,
    local: impl Fn(usize) -> Mat4,
) -> Vec<Mat4> {
    let mut out = vec![Mat4::IDENTITY; len];
    for &i in &compiled.transform_order {
        let local = local(i);
        out[i] = compiled.parents[i].map_or(local, |p| out[p] * local);
    }
    out
}
#[derive(Clone)]
pub struct RuntimeState {
    pub registry: Registry,
    pub gameplay: Option<crate::gameplay::Gameplay>,
    pub revision: i64,
    pub compiled: CompiledWorld,
    pub transforms: Vec<Option<Transform>>,
    pub health: Vec<Option<Health>>,
    pub columns: BTreeMap<String, ComponentTable>,
    pub ids: Vec<RuntimeEntityId>,
    pub(crate) next_generation: u64,
    pub(crate) animation_cues: crate::animation::AnimationCues,
    pub(crate) audio: crate::audio::Events,
    pub events: crate::events::Events,
    pub contexts: Option<crate::input::Contexts>,
    pub combat: Option<crate::combat::Combat>,
    pub tick: u64,
    /// SplitMix64 state. Seeded from the `core.rng` resource; checkpointed with the tick.
    pub rng: u64,
}
pub const DEFAULT_SEED: u64 = 0x5EED_A110_C0DE_0001;
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RngSettings {
    pub seed: u64,
}
impl RuntimeState {
    /// Deterministic per-world random stream. Systems must draw only inside fixed steps.
    pub fn random_u64(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn random_f32(&mut self) -> f32 {
        (self.random_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn spawn(
        &mut self,
        template: &CompiledEntity,
        id: String,
        position: [f32; 3],
    ) -> Result<RuntimeEntityId> {
        valid_id(&id)?;
        ensure!(
            !self.compiled.indices.contains_key(&id) && self.compiled.len() < MAX_ENTITIES,
            "runtime entity ID or population budget exceeded"
        );
        ensure!(
            position.iter().all(|p| p.is_finite()),
            "non-finite spawn position"
        );
        let instances = self
            .compiled
            .entities
            .iter()
            .filter_map(|e| e.mesh.as_ref())
            .map(|m| m.parts.len())
            .sum::<usize>()
            + template.mesh.as_ref().map_or(0, |m| m.parts.len());
        ensure!(
            instances <= MAX_INSTANCES,
            "runtime instance budget exceeded"
        );
        ensure!(
            self.compiled
                .entities
                .iter()
                .filter(|e| e.camera.as_ref().is_some_and(|c| c.active))
                .count()
                + usize::from(template.camera.as_ref().is_some_and(|c| c.active))
                <= MAX_CAMERAS,
            "runtime camera budget exceeded"
        );
        let slot = self.compiled.len();
        let mut indices = self.compiled.indices.clone();
        indices.insert(id.clone(), slot);
        let mut columns = self.columns.clone();
        for (name, schema) in &self.compiled.source.component_schemas {
            if let Some(value) = template.components.get(name) {
                columns
                    .entry(name.clone())
                    .or_insert_with(|| ComponentTable::new(schema))
                    .push(slot, schema, value, &indices)?;
            }
        }
        let mut entity = template.clone();
        entity.id = id.clone();
        if let Some(t) = &mut entity.transform {
            t.position = position;
            t.validate()?;
            entity
                .components
                .insert("core.transform".into(), serde_json::to_value(&*t)?);
        }
        let runtime_id = RuntimeEntityId {
            slot: slot as u32,
            generation: self.next_generation,
        };
        self.next_generation += 1;
        self.transforms.push(entity.transform.clone());
        self.health.push(entity.health.clone());
        self.ids.push(runtime_id);
        // By name: the template may come from another compilation with other tag ids.
        self.compiled.tag_index.push(&entity.tags);
        self.compiled.entities.push(entity);
        self.compiled.indices = indices;
        self.compiled.parents.push(None);
        self.compiled.transform_order.push(slot);
        self.compiled.enabled.push(true);
        self.columns = columns;
        Ok(runtime_id)
    }
    fn new(compiled: CompiledWorld, registry: Registry) -> Self {
        let events = crate::events::Events::new(&compiled.source);
        let rng = compiled
            .source
            .resource::<RngSettings>("core.rng")
            .map_or(DEFAULT_SEED, |r| r.seed);
        Self {
            rng,
            registry,
            gameplay: None,
            revision: 0,
            transforms: compiled
                .entities
                .iter()
                .map(|e| e.transform.clone())
                .collect(),
            health: compiled.entities.iter().map(|e| e.health.clone()).collect(),
            columns: compiled.columns.clone(),
            ids: (0..compiled.len())
                .map(|i| RuntimeEntityId {
                    slot: i as u32,
                    generation: 1,
                })
                .collect(),
            next_generation: 2,
            animation_cues: Default::default(),
            audio: Default::default(),
            events,
            contexts: None,
            combat: None,
            compiled,
            tick: 0,
        }
    }
    pub fn resolve(&self, id: RuntimeEntityId) -> Option<usize> {
        self.ids.iter().position(|&v| v == id)
    }
    pub fn matrices(&self) -> Vec<Mat4> {
        compose_matrices(&self.compiled, self.transforms.len(), |i| {
            self.transforms[i]
                .as_ref()
                .map_or(Mat4::IDENTITY, Transform::matrix)
        })
    }
    pub fn set_position(&mut self, id: &str, p: [f32; 3]) {
        if let Some(&i) = self.compiled.indices.get(id) {
            if let Some(t) = &mut self.transforms[i] {
                t.position = p;
            }
        }
    }
    pub fn set_health(&mut self, id: &str, hp: f32) {
        if let Some(&i) = self.compiled.indices.get(id) {
            if let Some(h) = &mut self.health[i] {
                h.value = hp.clamp(0.0, h.max);
            }
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(g) = &self.gameplay {
            ensure!(g.fault.is_none(), "gameplay fault: {:?}", g.fault);
        }
        for t in self.transforms.iter().flatten() {
            t.validate()?;
        }
        for h in self.health.iter().flatten() {
            h.validate()?;
        }
        ensure!(
            self.compiled.tag_index.slots() == self.compiled.len(),
            "tag index covers {} of {} entity slots",
            self.compiled.tag_index.slots(),
            self.compiled.len()
        );
        Ok(())
    }
    pub fn sync_scenes(&mut self) {
        if let Some(g) = &self.gameplay {
            self.compiled.enabled.fill(true);
            let mut membership = vec![false; self.compiled.len()];
            for scene in &self.compiled.source.scenes {
                for id in &scene.entities {
                    membership[self.compiled.indices[id]] = true;
                }
            }
            for (i, member) in membership.iter().enumerate() {
                if *member {
                    self.compiled.enabled[i] = false;
                }
            }
            for (scene, active) in self.compiled.source.scenes.iter().zip(&g.active) {
                if *active {
                    for id in &scene.entities {
                        self.compiled.enabled[self.compiled.indices[id]] = true;
                    }
                }
            }
        }
    }
}
/// `Send` so a replacement runtime can be built on a worker thread and swapped in.
pub trait RuntimeSystem: Any + Send {
    fn validate(&self, _state: &RuntimeState) -> Result<()> {
        Ok(())
    }
    fn command(
        &mut self,
        _state: &mut RuntimeState,
        _capability: &str,
        _action: &str,
        _args: &Value,
    ) -> Option<Result<Value>> {
        None
    }
    fn screen_pointer(
        &mut self,
        _state: &mut RuntimeState,
        _pointer: &ScreenPointer,
    ) -> Result<()> {
        Ok(())
    }
    /// Whether `screen_pointer` can change this system's simulation (only then are pointer
    /// events recorded into scenarios).
    fn uses_screen_pointer(&self) -> bool {
        false
    }
    fn checkpoint_supported(&self) -> bool {
        false
    }
    /// Whether the current state can be saved now (a supported system may still hold
    /// transient state that checkpoints do not carry).
    fn checkpoint_ready(&self) -> Result<()> {
        Ok(())
    }
    /// Simulation state this system stores in checkpoints (`Checkpoint.systems`), if any.
    fn save_state(&self, _state: &RuntimeState) -> Option<Result<Value>> {
        None
    }
    /// Restores what `save_state` stored, into this freshly built system (before `restored`).
    fn load_state(&mut self, _state: &mut RuntimeState, _value: &Value) -> Result<()> {
        anyhow::bail!("this system stores no checkpoint state")
    }
    /// A checkpoint was just loaded into this freshly built system's runtime.
    fn restored(&mut self, _state: &RuntimeState) {}
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    fn step(&mut self, state: &mut RuntimeState, input: &ActionMap);
    fn finish_step(&mut self, _state: &mut RuntimeState) {}
    fn combat_bodies(&self) -> Vec<axiom_combat::Body> {
        vec![]
    }
    fn combat_obstacles(&self) -> Vec<axiom_combat::Rect> {
        vec![]
    }
    fn combat_commit(&mut self, _state: &mut RuntimeState) {}
    fn input_lost(&mut self) {}
    fn receive_events(
        &mut self,
        _state: &mut RuntimeState,
        events: &[crate::events::Envelope],
    ) -> Result<()> {
        ensure!(
            events.is_empty(),
            "native system does not support event consumption"
        );
        Ok(())
    }
    fn transfer(
        &mut self,
        _old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        _next: &mut RuntimeState,
    ) -> Result<()> {
        Ok(())
    }
    fn extract(&self, _state: &RuntimeState, _frame: &mut RenderFrame) {}
    fn draw_ui(
        &self,
        _ctx: &egui::Context,
        _state: &RuntimeState,
        _ui: &mut UiState,
    ) -> Option<Vec<Action>> {
        None
    }
    fn pointer(&mut self, _dx: f32, _dy: f32) {}
    fn pointer_allowed(&self) -> bool {
        true
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::new()
    }
}
pub type SystemFactory = fn(&mut RuntimeState) -> Result<Box<dyn RuntimeSystem>>;
/// Scenario recordings keep `Move` events only while the primary button is held (a drag),
/// and only the last of consecutive moves between two fixed steps: a system must treat a run
/// of moves as "the cursor is now here" (last wins) and must not change simulated state on
/// a move without a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerKind {
    Move,
    PrimaryDown,
    PrimaryUp,
    SecondaryDown,
}
#[derive(Clone, Copy, Debug)]
pub struct ScreenPointer {
    pub kind: PointerKind,
    pub screen: [f32; 2],
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub additive: bool,
}
struct MotionSystem;
impl RuntimeSystem for MotionSystem {
    fn checkpoint_supported(&self) -> bool {
        true
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, _input: &ActionMap) {
        for (i, e) in state.compiled.entities.iter().enumerate() {
            if !state.compiled.enabled[i] {
                continue;
            }
            if let (Some(m), Some(t)) = (&e.motion, &mut state.transforms[i]) {
                t.position = (Vec3::from_array(t.position)
                    + Vec3::from_array(m.velocity) * FIXED_DT)
                    .to_array();
                t.rotation =
                    (Quat::from_scaled_axis(Vec3::from_array(m.angular_velocity) * FIXED_DT)
                        * Quat::from_array(t.rotation))
                    .normalize()
                    .to_array();
            }
        }
    }
}
#[derive(Clone)]
pub struct RuntimeRegistry {
    pub authoring: Registry,
    pub factories: BTreeMap<String, SystemFactory>,
}
/// A native extension: registers capabilities, schemas, actions, systems and factories.
/// Game crates compose plugins without editing `axiom-runtime`:
/// `axiom_runtime::installed_registry().with(MyPlugin)`.
pub trait Plugin {
    fn id(&self) -> &str;
    fn register(&self, registry: &mut RuntimeRegistry);
}
impl RuntimeRegistry {
    /// Adds a plugin. Fails if it would silently replace an installed system or action,
    /// so extensions cannot shadow built-in behavior by accident.
    pub fn try_with(mut self, plugin: impl Plugin) -> Result<Self> {
        let systems: Vec<String> = self.factories.keys().cloned().collect();
        let actions: Vec<String> = self.authoring.actions.keys().cloned().collect();
        let mut added = self.clone();
        plugin.register(&mut added);
        for id in &systems {
            ensure!(
                added.factories[id] as usize == self.factories[id] as usize,
                "plugin {} replaces installed system {id}",
                plugin.id()
            );
        }
        for id in &actions {
            ensure!(
                added.authoring.actions[id].schema == self.authoring.actions[id].schema,
                "plugin {} replaces installed action {id}",
                plugin.id()
            );
        }
        self = added;
        Ok(self)
    }
    /// `try_with` that panics on conflicts; convenient in `main`.
    pub fn with(self, plugin: impl Plugin) -> Self {
        self.try_with(plugin).unwrap_or_else(|e| panic!("{e:#}"))
    }
    pub fn core() -> Self {
        let mut authoring = Registry::core();
        authoring.resource(
            "runtime.bindings",
            "core",
            serde_json::json!({"events":"[{event: behavior.program event ID, calls: [up to 8 {kind:call, capability, action, args}]}]; calls run when the event fires (rules, choices, UI, input)"}),
            |v| serde_json::from_value::<EventBindings>(v.clone())?.validate(),
        );
        authoring.resource(
            "core.rng",
            "core",
            serde_json::json!({"seed":"u64; seeds the deterministic runtime random stream (SplitMix64)"}),
            |v| {
                serde_json::from_value::<RngSettings>(v.clone())?;
                Ok(())
            },
        );
        Self {
            authoring,
            factories: BTreeMap::from([
                (
                    "behavior.execute".into(),
                    (|state: &mut RuntimeState| {
                        state.gameplay =
                            Some(crate::gameplay::Gameplay::new(&state.compiled.source)?);
                        state.sync_scenes();
                        Ok(Box::new(crate::gameplay::BehaviorSystem) as Box<dyn RuntimeSystem>)
                    }) as SystemFactory,
                ),
                (
                    "motion.integrate".into(),
                    (|_: &mut RuntimeState| Ok(Box::new(MotionSystem) as Box<dyn RuntimeSystem>))
                        as SystemFactory,
                ),
            ]),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Observation {
    pub world_id: String,
    pub runtime_session: String,
    pub runtime_epoch: u64,
    pub rev: i64,
    pub requested_rev: i64,
    pub activation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<Value>,
    pub tick: u64,
    pub elapsed_seconds: f32,
    pub entities: usize,
    pub cameras: usize,
    pub systems: Vec<String>,
    pub frame_ms_cpu: f32,
    pub fps: f32,
    pub gpu: String,
    #[serde(flatten)]
    pub metrics: BTreeMap<String, Value>,
}
/// `runtime.bindings`: native actions run when a gameplay event fires. This lets narrative
/// choices, rules and input-mapped events drive registered capabilities (animation.play,
/// fsm/rts/vfx actions…) without code. Only `call` commands are allowed, so bindings cannot
/// re-emit gameplay events and loop.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventBindings {
    #[serde(default)]
    pub events: Vec<EventBinding>,
}
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventBinding {
    pub event: String,
    pub calls: Vec<axiom_core::gameplay::RuntimeCommand>,
}
impl EventBindings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.events.len() <= 128,
            "runtime.bindings: at most 128 event bindings"
        );
        for b in &self.events {
            valid_id(&b.event)?;
            ensure!(
                (1..=8).contains(&b.calls.len()),
                "runtime.bindings {}: 1..8 calls",
                b.event
            );
            for c in &b.calls {
                ensure!(
                    matches!(c, axiom_core::gameplay::RuntimeCommand::Call { .. }),
                    "runtime.bindings {}: only call commands are allowed (no emit/choose loops)",
                    b.event
                );
            }
        }
        Ok(())
    }
}
fn world_has_bindings(state: &RuntimeState) -> bool {
    state
        .compiled
        .source
        .resources
        .contains_key("runtime.bindings")
}
/// Wall-clock phases of building one runtime (`WorldRuntime::new`), in milliseconds.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct BuildTimings {
    /// IR compile (`World::compile`).
    pub compile_ms: f64,
    /// CPU shader library validation (`shader::Prepared::from_world`); no GPU work.
    pub shader_library_ms: f64,
    /// Native system factories, combat/binding setup.
    pub systems_ms: f64,
    pub total_ms: f64,
}
/// Wall-clock phases of the last revision activation (`reconcile_checked`), in milliseconds.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ReconcileTimings {
    pub build: BuildTimings,
    /// Carry-over of ids, transforms, columns and native system state from the old runtime.
    pub transfer_ms: f64,
    /// Caller-supplied preparation (GPU shader/pipeline preparation in the app).
    pub prepare_ms: f64,
    pub total_ms: f64,
}
fn ms(since: std::time::Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}
pub struct WorldRuntime {
    /// Phases of the `new` call that built this runtime.
    pub build_timings: BuildTimings,
    /// Phases of the activation that produced this runtime, if it came from `reconcile`.
    pub reconcile_timings: Option<ReconcileTimings>,
    /// Per-system factory time (ms) of the `new` call that built this runtime.
    pub system_build_ms: Vec<(String, f64)>,
    pub runtime_session: String,
    pub runtime_epoch: u64,
    pub state: RuntimeState,
    pub rev: i64,
    pub requested_rev: i64,
    pub activation_error: Option<Value>,
    pub(crate) authored_visibility: Vec<bool>,
    registry: RuntimeRegistry,
    systems: Vec<(String, Box<dyn RuntimeSystem>)>,
    pub bindings: InputBindings,
    settings: RenderSettings,
    environment: axiom_core::RenderEnvironment,
    shaders: std::sync::Arc<crate::shader::Prepared>,
    event_bindings: EventBindings,
    /// Last gameplay trace sequence already scanned for bindings.
    bound_seq: u64,
    binding_fault: Option<String>,
    binding_calls: u64,
    /// Opt-in wall time of each system's `step` (ms, summed since the last `take_system_profile`;
    /// same order as `system_ids`). `None` = not measured (no `Instant` reads per step).
    system_profile: Option<Vec<f64>>,
}
impl WorldRuntime {
    /// Run `runtime.bindings` calls for gameplay events that fired since the last scan.
    fn dispatch_bindings(&mut self) {
        if self.event_bindings.events.is_empty() {
            return;
        }
        let Some(g) = &self.state.gameplay else {
            return;
        };
        let fired: Vec<String> = g
            .trace
            .iter()
            .filter(|t| t.seq > self.bound_seq && t.kind == "event")
            .map(|t| t.id.clone())
            .collect();
        if let Some(last) = g.trace.back() {
            self.bound_seq = self.bound_seq.max(last.seq);
        }
        for event in fired {
            let calls: Vec<_> = self
                .event_bindings
                .events
                .iter()
                .filter(|b| b.event == event)
                .flat_map(|b| b.calls.clone())
                .collect();
            for call in calls {
                match self.command(&call) {
                    Ok(_) => self.binding_calls += 1,
                    Err(e) => {
                        self.binding_fault
                            .get_or_insert_with(|| format!("{event}: {e:#}"));
                    }
                }
            }
        }
    }
    pub fn system<T: Any>(&self) -> Option<&T> {
        self.systems
            .iter()
            .find_map(|(_, s)| s.as_any().downcast_ref::<T>())
    }
    pub fn system_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.systems
            .iter_mut()
            .find_map(|(_, s)| s.as_any_mut()?.downcast_mut::<T>())
    }
    pub fn command(&mut self, command: &axiom_core::gameplay::RuntimeCommand) -> Result<Value> {
        ensure!(
            self.state.events.fault.is_none(),
            "event delivery failed; repair content before issuing gameplay commands"
        );
        if let axiom_core::gameplay::RuntimeCommand::Emit { event, .. } = command {
            ensure!(
                !self.state.events.owns(event),
                "owned native events may only be published by their registered system"
            );
        }
        if let axiom_core::gameplay::RuntimeCommand::Call {
            capability,
            action,
            args,
        } = command
        {
            let id = format!("{capability}.{action}");
            let descriptor = self
                .registry
                .authoring
                .actions
                .get(&id)
                .ok_or_else(|| Diagnostic::error("unknown_action", "/command/action", &id))?;
            ensure!(
                descriptor.capability == *capability
                    && self
                        .state
                        .compiled
                        .source
                        .capabilities
                        .contains_key(capability),
                "capability not enabled in this world"
            );
            axiom_core::local::at("/command/args", (descriptor.validate)(args))?;
            if capability == "input" && action == "activate" {
                self.state
                    .contexts
                    .as_mut()
                    .context("world has no input contexts")?
                    .activate(args)?;
                for (id, system) in &mut self.systems {
                    if !self.state.contexts.as_ref().unwrap().routes(id) {
                        system.input_lost();
                    }
                }
                return Ok(
                    serde_json::json!({"ok":true,"rev":self.rev,"tick":self.state.tick,"input":self.state.contexts.as_ref().unwrap().observation()}),
                );
            }
            for (_, system) in &mut self.systems {
                if let Some(result) = system.command(&mut self.state, capability, action, args) {
                    return Ok(
                        serde_json::json!({"ok":true,"rev":self.rev,"tick":self.state.tick,"result":result?}),
                    );
                }
            }
            anyhow::bail!("runtime handler missing for {id}");
        }
        self.state
            .gameplay
            .as_mut()
            .context("world has no gameplay command capability")?
            .command(command, self.state.tick)?;
        self.state.sync_scenes();
        // Choices, UI emits and loopback events may fire bound native actions immediately.
        self.dispatch_bindings();
        Ok(
            serde_json::json!({"ok":true,"rev":self.rev,"tick":self.state.tick,"gameplay":self.state.gameplay.as_ref().unwrap().observation()}),
        )
    }
    /// Read-only runtime state of entity `id`: the result of every installed
    /// `CAPABILITY.inspect {"id": id}` action enabled in this world that answers for it
    /// (actions that do not know the entity or take other arguments are skipped).
    pub fn inspect_entity(&mut self, id: &str) -> Vec<(String, Value)> {
        let actions: Vec<(String, String)> = self
            .registry
            .authoring
            .actions
            .iter()
            .filter(|(name, d)| {
                name.ends_with(".inspect")
                    && self
                        .state
                        .compiled
                        .source
                        .capabilities
                        .contains_key(&d.capability)
            })
            .filter_map(|(name, d)| {
                let action = name
                    .strip_prefix(d.capability.as_str())?
                    .strip_prefix('.')?;
                Some((d.capability.clone(), action.to_owned()))
            })
            .collect();
        let args = serde_json::json!({ "id": id });
        let mut out = vec![];
        for (capability, action) in actions {
            let name = format!("{capability}.{action}");
            let command = axiom_core::gameplay::RuntimeCommand::Call {
                capability,
                action,
                args: args.clone(),
            };
            if let Ok(mut response) = self.command(&command) {
                out.push((name, response["result"].take()));
            }
        }
        out
    }
    pub(crate) fn checkpoint_supported(&self) -> Result<()> {
        ensure!(
            self.state.contexts.is_none(),
            "input contexts do not implement checkpoints"
        );
        ensure!(
            self.state.compiled.source.capabilities.contains_key("save"),
            "world does not enable save capability"
        );
        for (id, s) in &self.systems {
            ensure!(
                s.checkpoint_supported(),
                "system {id} does not implement checkpoints"
            );
        }
        Ok(())
    }
    /// Saving needs every system's transient state to be saveable now (`checkpoint_ready`).
    pub(crate) fn checkpoint_ready(&self) -> Result<()> {
        for (id, s) in &self.systems {
            s.checkpoint_ready()
                .map_err(|e| anyhow::anyhow!("system {id}: {e:#}"))?;
        }
        Ok(())
    }
    /// State of every system that stores some (`RuntimeSystem::save_state`).
    pub(crate) fn save_system_states(&self) -> Result<BTreeMap<String, Value>> {
        let mut out = BTreeMap::new();
        for (id, s) in &self.systems {
            if let Some(v) = s.save_state(&self.state) {
                out.insert(
                    id.clone(),
                    v.map_err(|e| anyhow::anyhow!("system {id}: {e:#}"))?,
                );
            }
        }
        Ok(out)
    }
    /// Loads saved system states; a system that stores state must find it and every saved
    /// entry must belong to a running system (nothing is dropped silently).
    pub(crate) fn load_system_states(&mut self, saved: &BTreeMap<String, Value>) -> Result<()> {
        for id in saved.keys() {
            ensure!(
                self.systems.iter().any(|(s, _)| s == id),
                "checkpoint holds state of system {id}, which this world does not run"
            );
        }
        for (id, s) in &mut self.systems {
            match (saved.get(id.as_str()), s.save_state(&self.state).is_some()) {
                (Some(v), _) => s
                    .load_state(&mut self.state, v)
                    .map_err(|e| anyhow::anyhow!("system {id}: {e:#}"))?,
                (None, true) => anyhow::bail!(
                    "checkpoint lacks the state of system {id} (saved before it supported checkpoints)"
                ),
                (None, false) => {}
            }
        }
        Ok(())
    }
    /// Tells every system that a checkpoint was loaded into this runtime.
    pub(crate) fn restored(&mut self) {
        for (_, s) in &mut self.systems {
            s.restored(&self.state);
        }
    }
    pub fn registry_clone(&self) -> RuntimeRegistry {
        self.registry.clone()
    }
    pub fn new(
        world: impl std::borrow::Borrow<World>,
        rev: i64,
        registry: RuntimeRegistry,
    ) -> Result<Self> {
        let world: &World = world.borrow();
        let started = std::time::Instant::now();
        let compiled = world.compile(&registry.authoring)?;
        let compile_ms = ms(started);
        let authored_visibility = compiled.enabled.clone();
        let phase = std::time::Instant::now();
        let shaders = std::sync::Arc::new(crate::shader::Prepared::from_world(world)?);
        let shader_library_ms = ms(phase);
        let phase = std::time::Instant::now();
        let settings = if world.resources.contains_key("render.settings") {
            world.resource("render.settings")?
        } else {
            RenderSettings::default()
        };
        let environment = if world.resources.contains_key("render.environment") {
            world.resource("render.environment")?
        } else {
            axiom_core::RenderEnvironment::default()
        };
        let bindings = if world.resources.contains_key("input.bindings") {
            world.resource("input.bindings")?
        } else {
            InputBindings::default()
        };
        let mut state = RuntimeState::new(compiled, registry.authoring.clone());
        state.contexts = crate::input::Contexts::new(world)?;
        state.combat = crate::combat::Combat::new(world)?;
        state.revision = rev;
        let mut system_build_ms = vec![];
        let mut systems = vec![];
        for d in state.compiled.schedule.clone() {
            let factory = registry
                .factories
                .get(&d.id)
                .with_context(|| format!("runtime implementation missing for {}", d.id))?;
            let started = std::time::Instant::now();
            let system = factory(&mut state)?;
            system_build_ms.push((d.id.clone(), ms(started)));
            systems.push((d.id, system));
        }
        crate::bound_ui::prepare_assets(state.gameplay.as_mut())?;
        if let Some(combat) = &mut state.combat {
            for (_, system) in &systems {
                combat
                    .scene
                    .config
                    .obstacles
                    .extend(system.combat_obstacles());
            }
            combat.scene.config.validate()?;
        }
        if state.combat.is_some() {
            for (_, system) in &systems {
                crate::combat::snapshot(&mut state, system.combat_bodies());
            }
        }
        if let Some(combat) = &state.combat {
            combat.scene.validate()?;
        }
        let event_bindings: EventBindings = if world_has_bindings(&state) {
            state.compiled.source.resource("runtime.bindings")?
        } else {
            EventBindings::default()
        };
        event_bindings.validate()?;
        let bound_seq = state
            .gameplay
            .as_ref()
            .and_then(|g| g.trace.back())
            .map_or(0, |t| t.seq);
        let build_timings = BuildTimings {
            compile_ms,
            shader_library_ms,
            systems_ms: ms(phase),
            total_ms: ms(started),
        };
        Ok(Self {
            build_timings,
            system_build_ms,
            reconcile_timings: None,
            runtime_session: format!(
                "{:x}-{:x}-{:x}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos(),
                {
                    static SERIAL: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(0);
                    SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                }
            ),
            runtime_epoch: 1,
            state,
            rev,
            requested_rev: rev,
            activation_error: None,
            authored_visibility,
            registry,
            bound_seq,
            systems,
            bindings,
            settings,
            environment,
            shaders,
            event_bindings,
            binding_fault: None,
            binding_calls: 0,
            system_profile: None,
        })
    }
    /// Prepare a complete replacement and migrate state before changing the active runtime.
    pub fn reconcile(&mut self, world: impl std::borrow::Borrow<World>, rev: i64) -> Result<()> {
        self.reconcile_checked(world, rev, |_| Ok(()))
    }
    pub(crate) fn shaders(&self) -> &std::sync::Arc<crate::shader::Prepared> {
        &self.shaders
    }
    pub(crate) fn reconcile_checked(
        &mut self,
        world: impl std::borrow::Borrow<World>,
        rev: i64,
        prepare: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        let started = std::time::Instant::now();
        self.requested_rev = rev;
        let next = Self::new(world, rev, self.registry.clone());
        self.adopt(next, prepare, started).map(drop)
    }
    /// Migrates state from this runtime into a replacement built for a new revision (here
    /// or on a worker thread), runs `prepare` and swaps it in, returning the replaced
    /// runtime. On error this runtime stays active and records the activation error.
    pub(crate) fn adopt(
        &mut self,
        next: Result<Self>,
        prepare: impl FnOnce(&Self) -> Result<()>,
        started: std::time::Instant,
    ) -> Result<Self> {
        let prepared = (|| -> Result<Self> {
            let mut next = next?;
            let transfer = std::time::Instant::now();
            next.runtime_session = self.runtime_session.clone();
            next.runtime_epoch = self.runtime_epoch
                + u64::from(next.state.compiled.source.id != self.state.compiled.source.id);
            next.state.tick = self.state.tick;
            // Content edits continue the random stream; editing the seed restarts it.
            if next.state.compiled.source.resources.get("core.rng")
                == self.state.compiled.source.resources.get("core.rng")
            {
                next.state.rng = self.state.rng;
            }
            if let Some(g) = &mut next.state.gameplay {
                for event in &mut g.pending {
                    event.due = event
                        .due
                        .checked_add(next.state.tick)
                        .context("event tick overflow")?;
                }
                for trace in &mut g.trace {
                    trace.tick = trace.tick.saturating_add(next.state.tick);
                }
            }
            next.state.next_generation = self.state.next_generation;
            for e in &next.state.compiled.entities {
                if let Some(&old) = self.state.compiled.indices.get(&e.id) {
                    let new = next.state.compiled.indices[&e.id];
                    next.state.ids[new] = self.state.ids[old];
                    if e.transform == self.state.compiled.entities[old].transform {
                        next.state.transforms[new] = self.state.transforms[old].clone();
                    }
                    if let (Some(h), Some(previous)) =
                        (&mut next.state.health[new], &self.state.health[old])
                    {
                        h.value = (previous.value + h.value
                            - self.state.compiled.entities[old]
                                .health
                                .as_ref()
                                .map_or(0.0, |v| v.value))
                        .clamp(0.0, h.max);
                    }
                } else {
                    let new = next.state.compiled.indices[&e.id];
                    next.state.ids[new].generation = next.state.next_generation;
                    next.state.next_generation += 1;
                }
            }
            for (name, table) in &mut next.state.columns {
                if next.state.compiled.source.component_schemas.get(name)
                    != self.state.compiled.source.component_schemas.get(name)
                {
                    continue;
                }
                if let Some(previous) = self.state.columns.get(name) {
                    let rows = previous
                        .entities
                        .iter()
                        .enumerate()
                        .map(|(row, slot)| (*slot, row))
                        .collect::<BTreeMap<_, _>>();
                    for (row, &slot) in table.entities.iter().enumerate() {
                        let entity = &next.state.compiled.entities[slot];
                        if let Some(&old_slot) = self.state.compiled.indices.get(&entity.id) {
                            if entity.components.get(name)
                                != self.state.compiled.entities[old_slot].components.get(name)
                            {
                                continue;
                            }
                            if let Some(&old_row) = rows.get(&old_slot) {
                                for (field, column) in &mut table.fields {
                                    let old_column = &previous.fields[field];
                                    match (column, old_column) {
                                        (Column::Entity(dst), Column::Entity(src)) => {
                                            let target = src[old_row]
                                                .and_then(|i| self.state.compiled.entities.get(i))
                                                .and_then(|e| {
                                                    next.state.compiled.indices.get(&e.id)
                                                })
                                                .copied();
                                            if target.is_some() || src[old_row].is_none() {
                                                dst[row] = target;
                                            }
                                        }
                                        (column, old) => {
                                            column.preserve_scalar(row, old, old_row);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for (id, system) in &mut next.systems {
                if let Some((_, old)) = self.systems.iter().find(|(name, _)| name == id) {
                    system.transfer(old.as_ref(), &self.state, &mut next.state)?;
                }
            }
            if self.state.compiled.source.id == next.state.compiled.source.id {
                if let (Some(old), Some(new)) = (&self.state.combat, &mut next.state.combat) {
                    new.inherit(old);
                }
                next.state.events.transfer(&self.state.events)?;
                if let (Some(previous), Some(contexts)) =
                    (&self.state.contexts, &mut next.state.contexts)
                {
                    contexts.transfer(previous);
                }
            }
            if let Some(combat) = &mut next.state.combat {
                combat.scene.bodies.clear();
            }
            if next.state.combat.is_some() {
                for (_, system) in &next.systems {
                    crate::combat::snapshot(&mut next.state, system.combat_bodies());
                }
            }
            if let Some(combat) = &next.state.combat {
                combat.scene.validate()?;
            }
            next.state.validate()?;
            let transfer_ms = ms(transfer);
            let phase = std::time::Instant::now();
            prepare(&next)?;
            next.reconcile_timings = Some(ReconcileTimings {
                build: next.build_timings,
                transfer_ms,
                prepare_ms: ms(phase),
                total_ms: ms(started),
            });
            Ok(next)
        })();
        match prepared {
            Ok(next) => Ok(std::mem::replace(self, next)),
            Err(e) => {
                self.activation_error = Some(error_response(&e));
                Err(e)
            }
        }
    }
    pub fn step(&mut self, input: &ActionMap) {
        if self.state.events.fault.is_some() {
            return;
        }
        self.state.tick += 1;
        if let Some(combat) = &mut self.state.combat {
            combat.scene.bodies.clear();
        }
        if self.state.combat.is_some() {
            for (_, system) in &self.systems {
                crate::combat::snapshot(&mut self.state, system.combat_bodies());
            }
        }
        if let Some(contexts) = &mut self.state.contexts {
            contexts.step(input.pressed("context.next"));
        }
        let empty = ActionMap::default();
        for (index, (id, system)) in self.systems.iter_mut().enumerate() {
            let input = if self.state.contexts.as_ref().is_none_or(|c| c.routes(id)) {
                input
            } else {
                &empty
            };
            if self.state.contexts.as_ref().is_some_and(|c| !c.routes(id)) {
                system.input_lost();
            }
            match &mut self.system_profile {
                Some(profile) => {
                    let started = std::time::Instant::now();
                    system.step(&mut self.state, input);
                    if let Some(slot) = profile.get_mut(index) {
                        *slot += started.elapsed().as_secs_f64() * 1000.0;
                    }
                }
                None => system.step(&mut self.state, input),
            }
        }
        crate::combat::resolve(&mut self.state);
        for (_, system) in &mut self.systems {
            system.combat_commit(&mut self.state);
        }
        let routed = |state: &RuntimeState, systems: &[(String, Box<dyn RuntimeSystem>)]| {
            systems
                .iter()
                .any(|(id, _)| !state.events.for_consumer(id).is_empty())
        };
        if self.state.events.fault.is_none()
            && !self.state.events.pending.is_empty()
            && !routed(&self.state, &self.systems)
        {
            // No system consumes any pending event (receive_events with no events is a
            // no-op), so delivery cannot fail and needs no transactional copy.
            self.state.events.acknowledge();
        }
        if self.state.events.fault.is_none() && !self.state.events.pending.is_empty() {
            let mut prepared = self.state.clone();
            let result = self.systems.iter_mut().try_for_each(|(id, system)| {
                let events = prepared.events.for_consumer(id);
                system.receive_events(&mut prepared, &events)
            });
            match result {
                Ok(()) => {
                    prepared.events.acknowledge();
                    self.state = prepared;
                }
                Err(error) => self.state.events.fault = Some(error_response(&error)),
            }
        }
        for (_, system) in &mut self.systems {
            system.finish_step(&mut self.state);
        }
        self.dispatch_bindings();
    }
    /// Whether a system acts on screen pointer events (see `RuntimeSystem::uses_screen_pointer`).
    pub fn uses_screen_pointer(&self) -> bool {
        self.systems.iter().any(|(_, s)| s.uses_screen_pointer())
    }
    /// IDs of the installed systems in step order.
    pub fn system_ids(&self) -> impl ExactSizeIterator<Item = &str> {
        self.systems.iter().map(|(id, _)| id.as_str())
    }
    /// Starts or stops measuring each system's `step` wall time (debug tooling; the
    /// simulation is unchanged). Turning it on allocates one slot per system, once.
    pub fn set_system_profiling(&mut self, on: bool) {
        match (on, self.system_profile.is_some()) {
            (true, false) => self.system_profile = Some(vec![0.0; self.systems.len()]),
            (false, true) => self.system_profile = None,
            _ => {}
        }
    }
    /// Milliseconds each system spent in `step` since the last call (step order), then
    /// zeroes them. Empty when profiling is off.
    pub fn take_system_profile(&mut self, mut each: impl FnMut(usize, f64)) {
        if let Some(profile) = &mut self.system_profile {
            for (index, ms) in profile.iter_mut().enumerate() {
                each(index, *ms);
                *ms = 0.0;
            }
        }
    }
    pub fn pointer(&mut self, dx: f32, dy: f32) {
        for (id, system) in &mut self.systems {
            if self.state.contexts.as_ref().is_none_or(|c| c.routes(id)) {
                system.pointer(dx, dy);
            }
        }
    }
    pub fn screen_pointer(
        &mut self,
        kind: PointerKind,
        screen: [f32; 2],
        aspect: f32,
        additive: bool,
    ) -> Result<()> {
        ensure!(
            aspect.is_finite() && aspect > 0.0 && screen.iter().all(|v| v.is_finite()),
            "invalid screen pointer"
        );
        let Some((slot, camera)) =
            self.state
                .compiled
                .entities
                .iter()
                .enumerate()
                .find_map(|(i, e)| {
                    let camera = e.camera.as_ref()?;
                    let [x, y, w, h] = camera.viewport;
                    (self.state.compiled.enabled[i]
                        && camera.active
                        && self.state.contexts.as_ref().is_none_or(|c| c.camera(&e.id))
                        && screen[0] >= x
                        && screen[0] <= x + w
                        && screen[1] >= y
                        && screen[1] <= y + h)
                        .then_some((i, camera))
                })
        else {
            return Ok(());
        };
        let mut model = self.state.transforms[slot]
            .as_ref()
            .map_or(Mat4::IDENTITY, Transform::matrix);
        let mut parent = self.state.compiled.parents[slot];
        while let Some(i) = parent {
            model = self.state.transforms[i]
                .as_ref()
                .map_or(Mat4::IDENTITY, Transform::matrix)
                * model;
            parent = self.state.compiled.parents[i];
        }
        let eye = model.transform_point3(Vec3::ZERO);
        let forward = model.transform_vector3(-Vec3::Z).normalize();
        let up = model.transform_vector3(Vec3::Y).normalize();
        let [x, y, w, h] = camera.viewport;
        let projection = match camera.projection {
            Projection::Perspective { fov_degrees } => Mat4::perspective_rh(
                fov_degrees.to_radians(),
                aspect * w / h,
                camera.near,
                camera.far,
            ),
            Projection::Orthographic { height } => Mat4::orthographic_rh(
                -height * aspect * w / h * 0.5,
                height * aspect * w / h * 0.5,
                -height * 0.5,
                height * 0.5,
                camera.near,
                camera.far,
            ),
        };
        let inverse = (projection * Mat4::look_at_rh(eye, eye + forward, up)).inverse();
        let ndc = Vec3::new(
            (screen[0] - x) / w * 2.0 - 1.0,
            1.0 - (screen[1] - y) / h * 2.0,
            0.0,
        );
        let near = inverse.project_point3(ndc);
        let far = inverse.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
        let pointer = ScreenPointer {
            kind,
            screen,
            origin: near.to_array(),
            direction: (far - near).normalize().to_array(),
            additive,
        };
        for (id, system) in &mut self.systems {
            if self.state.contexts.as_ref().is_none_or(|c| c.routes(id)) {
                system.screen_pointer(&mut self.state, &pointer)?;
            }
        }
        Ok(())
    }
    pub fn pointer_allowed(&self) -> bool {
        self.state.events.fault.is_none()
            && self
                .state
                .contexts
                .as_ref()
                .map_or(self.bindings.capture_pointer, |c| c.capture())
            && self.systems.iter().all(|(_, s)| s.pointer_allowed())
    }
    pub fn draw_ui(&self, ctx: &egui::Context, ui: &mut UiState) -> Vec<Action> {
        let mut actions = vec![];
        let mut drawn = false;
        for (id, system) in &self.systems {
            if self.state.contexts.as_ref().is_some_and(|c| !c.routes(id)) {
                continue;
            }
            if let Some(a) = system.draw_ui(ctx, &self.state, ui) {
                actions.extend(a);
                drawn = true;
            }
        }
        if !drawn {
            actions = crate::ui::draw_generic(ctx, &self.state, ui);
        }
        if let Some(contexts) = &self.state.contexts {
            contexts.draw(
                ctx,
                &mut actions,
                self.bindings
                    .buttons
                    .get("context.next")
                    .map(String::as_str),
            );
        }
        if let Some(g) = &self.state.gameplay {
            let fx = self
                .system::<crate::ui_effects::UiEffects>()
                .map(|s| s as &dyn crate::ui_effects::EffectVisuals);
            actions.extend(crate::bound_ui::draw(ctx, g, ui, fx));
        }
        if let Some(error) = &self.state.events.fault {
            egui::Window::new("Event delivery stopped")
                .collapsible(false)
                .show(ctx, |p| {
                    p.label(
                        "Simulation is stopped. Repair the event rules in the editor or console.",
                    );
                    p.monospace(error.to_string());
                });
        }
        actions
    }
    /// A frame that draws only the interface (no views, nothing extracted).
    pub fn interface_frame(&self) -> RenderFrame {
        RenderFrame {
            draw_world: false,
            tick: self.state.tick,
            settings: self.settings.clone(),
            instances: vec![],
            views: vec![],
            meshes: vec![],
            shaders: self.shaders.clone(),
            cube_materials: vec![],
            vfx: Default::default(),
            lights: vec![],
            scene_lights: vec![],
            environment: self.environment.clone(),
            postfx: vec![],
        }
    }
    pub fn render_frame(&self) -> RenderFrame {
        let matrices = self.state.matrices();
        let mut frame = RenderFrame {
            draw_world: true,
            tick: self.state.tick,
            settings: self.settings.clone(),
            instances: vec![],
            views: vec![],
            meshes: vec![],
            shaders: self.shaders.clone(),
            cube_materials: vec![],
            vfx: Default::default(),
            lights: vec![],
            scene_lights: vec![],
            environment: self.environment.clone(),
            postfx: vec![],
        };
        for (i, e) in self.state.compiled.entities.iter().enumerate() {
            if !self.state.compiled.enabled[i] {
                continue;
            }
            if let Some(mesh) = &e.mesh {
                let start = frame.instances.len() as u32;
                frame
                    .instances
                    .extend(mesh.parts.iter().map(|p| Instance::part(matrices[i], p)));
                if let Some(material) = &e.material {
                    let end = frame.instances.len() as u32;
                    if let Some((range, _)) = frame
                        .cube_materials
                        .last_mut()
                        .filter(|(r, id)| r.end == start && *id == material.id)
                    {
                        range.end = end;
                    } else {
                        frame.cube_materials.push((start..end, material.id.clone()));
                    }
                }
            }
            if let Some(light) = e.light.as_ref().filter(|l| l.enabled) {
                let m = matrices[i];
                frame.scene_lights.push(crate::presentation::SceneLight {
                    id: e.id.clone(),
                    light: light.clone(),
                    position: m.transform_point3(Vec3::ZERO),
                    direction: m.transform_vector3(-Vec3::Z).normalize_or(-Vec3::Y),
                    right: m.transform_vector3(Vec3::X).normalize_or(Vec3::X),
                });
            }
            if let Some(camera) = &e.camera {
                if camera.active {
                    frame.views.push(RenderView {
                        id: e.id.clone(),
                        eye: matrices[i].transform_point3(Vec3::ZERO),
                        forward: matrices[i].transform_vector3(-Vec3::Z).normalize(),
                        up: matrices[i].transform_vector3(Vec3::Y).normalize(),
                        camera: camera.clone(),
                        instances: vec![],
                    });
                }
            }
        }
        for (_, s) in &self.systems {
            s.extract(&self.state, &mut frame);
        }
        if let Some(contexts) = &self.state.contexts {
            frame.views.retain(|v| contexts.camera(&v.id));
        }
        frame
    }
    pub fn observation(&self) -> Observation {
        let mut metrics = BTreeMap::new();
        if let Some(combat) = &self.state.combat {
            metrics.insert("combat".into(), combat.observation());
        }
        if let Some(contexts) = &self.state.contexts {
            metrics.insert("input".into(), contexts.observation());
        }
        if let Some(packages) = &self.state.compiled.source.packages {
            metrics.insert("events".into(), self.state.events.observation());
            metrics.insert("packages".into(), packages.summary());
        }
        for (_, s) in &self.systems {
            metrics.extend(s.metrics());
        }
        if let Some(g) = &self.state.gameplay {
            metrics.insert("gameplay".into(), g.observation());
        }
        if !self.event_bindings.events.is_empty() {
            metrics.insert(
                "bindings".into(),
                serde_json::json!({"events":self.event_bindings.events.len(),"calls":self.binding_calls,"fault":self.binding_fault}),
            );
        }
        if !self.shaders.materials.is_empty() {
            metrics.insert("shader".into(), serde_json::json!({"programs": self.shaders.programs.len(), "materials": self.shaders.materials.len()}));
        }
        if !self.state.audio.waves.is_empty() {
            // Cue counters join the `audio.spatial` section when both systems run.
            let cues = self.state.audio.observation();
            match metrics.get_mut("audio").and_then(Value::as_object_mut) {
                Some(audio) => {
                    if let Value::Object(cues) = cues {
                        audio.extend(cues);
                    }
                }
                None => {
                    metrics.insert("audio".into(), cues);
                }
            }
        }
        Observation {
            world_id: self.state.compiled.source.id.clone(),
            runtime_session: self.runtime_session.clone(),
            runtime_epoch: self.runtime_epoch,
            rev: self.rev,
            requested_rev: self.requested_rev,
            activation: if self.activation_error.is_some() {
                "activation_failed"
            } else {
                "applied"
            }
            .into(),
            activation_error: self.activation_error.clone(),
            tick: self.state.tick,
            elapsed_seconds: self.state.tick as f32 * FIXED_DT,
            entities: self.state.compiled.len(),
            cameras: self
                .state
                .compiled
                .entities
                .iter()
                .enumerate()
                .filter(|(i, e)| {
                    self.state.compiled.enabled[*i]
                        && e.camera.as_ref().is_some_and(|c| c.active)
                        && self.state.contexts.as_ref().is_none_or(|c| c.camera(&e.id))
                })
                .count(),
            systems: self
                .state
                .compiled
                .schedule
                .iter()
                .map(|s| s.id.clone())
                .collect(),
            frame_ms_cpu: 0.0,
            fps: 0.0,
            gpu: String::new(),
            metrics,
        }
    }
    pub fn assert_valid(&self) -> Result<()> {
        ensure!(
            self.state.events.fault.is_none(),
            "cross-package event delivery failed: {:?}",
            self.state.events.fault
        );
        ensure!(self.state.tick < u64::MAX, "tick budget exhausted");
        self.state.validate()?;
        for (_, system) in &self.systems {
            system.validate(&self.state)?;
        }
        Ok(())
    }
}
