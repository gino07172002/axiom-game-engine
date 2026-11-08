//! `interact.focus`: runtime side of `interact.target` (see `axiom_core::interact`).
//!
//! Targets, interactor filters and candidate lists are compiled once into dense arrays. Each
//! fixed step first handles input: a rising edge of a target's `action` button uses the target
//! the *local interactor* focused in the previous step (what its prompt showed). Then focus is
//! recomputed for every interactor by testing, per target, only the entities its filter
//! matches: O(Σ targets × filter candidates), bounded by `MAX_INTERACT_PAIRS` (brute force is
//! measured at that bound in docs/interaction.zh-TW.md). Scratch arrays are reused, so a steady
//! step does not allocate. Uses emit `behavior.program` events through the same gameplay
//! command path as trigger zones and FSM state events, so rules and `runtime.bindings` run.
//!
//! The system runs in phase 7 (after behavior.execute, before fsm.step and the movers): focus
//! is computed from the positions the previous frame showed, exactly the timing the
//! FSM `near` conditions use.
use crate::{
    gameplay::Prompt,
    trigger::{pivot, world_matrix},
    world::*,
};
use anyhow::{ensure, Context, Result};
use axiom_core::{
    gameplay::{RuntimeCommand, Scalar},
    interact::*,
    trigger::TriggerFilter,
    InputBindings,
};
use glam::{Vec2, Vec3};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

/// Recent uses kept for `observe`.
const RECENT: usize = 32;
/// Interactors listed in `observe` / targets listed by `interact.inspect` (counts are complete).
const OBSERVED: usize = 32;
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Use {
    tick: u64,
    /// Interactor entity slot and target index.
    interactor: u32,
    target: u16,
    by_key: bool,
}

struct Target {
    slot: usize,
    range: f32,
    margin: f32,
    /// cos(view_angle / 2); `None` = any facing.
    min_cos: Option<f32>,
    prompt: Arc<str>,
    action: Option<usize>,
    event: Option<usize>,
    cooldown_ticks: u64,
    once: bool,
    enabled: bool,
    priority: i32,
    filter: usize,
    uses: u32,
    /// First tick at which the target can be focused / used again.
    ready_at: u64,
}

struct Filter {
    def: TriggerFilter,
    /// Matching entity slots with a transform, ascending.
    slots: Vec<u32>,
}

struct EventInfo {
    name: String,
    interactor_field: bool,
    target_field: bool,
}

struct ActionInfo {
    name: String,
    /// Key label from `input.bindings` ("KeyE" -> "E").
    key: Arc<str>,
    held: bool,
}

/// Why a target is not eligible for an interactor right now.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Reason {
    Disabled,
    Inactive,
    Spent,
    Cooling(u64),
    Filter,
    InteractorUnavailable,
    OutOfRange(f32),
    Facing,
}

impl Reason {
    fn text(self, range: f32) -> String {
        match self {
            Reason::Disabled => "target is disabled".into(),
            Reason::Inactive => "target is in an inactive scene".into(),
            Reason::Spent => "target is once and already used".into(),
            Reason::Cooling(t) => format!("target is cooling down ({t} ticks left)"),
            Reason::Filter => "entity does not match the target's interactor filter".into(),
            Reason::InteractorUnavailable => "interactor is disabled or dead".into(),
            Reason::OutOfRange(d) => format!("out of range ({d:.2} m > {range} m)"),
            Reason::Facing => "outside the target's view_angle".into(),
        }
    }
}

/// Candidate ordering key: higher priority, nearer, more in front, earlier authored.
#[derive(Clone, Copy)]
struct Candidate {
    target: u16,
    priority: i32,
    distance: f32,
    cos: f32,
}

impl Candidate {
    fn beats(&self, other: &Candidate) -> bool {
        (self.priority, other.distance, self.cos, other.target)
            > (other.priority, self.distance, other.cos, self.target)
    }
}

pub struct InteractSystem {
    targets: Vec<Target>,
    /// Target entity IDs (parallel to `targets`).
    target_ids: Vec<String>,
    filters: Vec<Filter>,
    events: Vec<EventInfo>,
    actions: Vec<ActionInfo>,
    /// Interactor candidates: union of every filter's slots (ascending) with IDs and focus.
    union: Vec<u32>,
    names: Vec<String>,
    focus: Vec<Option<u16>>,
    /// Entity slot -> index in `union` (`NONE` otherwise).
    union_index: Vec<u32>,
    /// Per union member, refreshed each step: world pivot, XZ facing, usable (enabled + alive).
    poses: Vec<(Vec3, Vec2, bool)>,
    best: Vec<Option<Candidate>>,
    target_pivots: Vec<Vec3>,
    /// Entity slots already matched against the filters (runtime spawns append).
    scanned: usize,
    pairs: usize,
    /// Target index whose prompt the gameplay UI currently shows (local interactor's focus).
    published: Option<u16>,
    /// Local interactor (union index) as of the last step, for `observe`.
    local_cached: Option<usize>,
    /// Set by `input_lost` for the next step.
    lost: bool,
    recent: VecDeque<Use>,
    used: u64,
    halted: Option<String>,
    fault: Option<String>,
}

/// "KeyE" -> "E", "Digit1" -> "1", anything else as written.
fn key_label(code: &str) -> String {
    for prefix in ["Key", "Digit"] {
        if let Some(rest) = code.strip_prefix(prefix) {
            if !rest.is_empty() {
                return rest.to_owned();
            }
        }
    }
    code.to_owned()
}

fn alive(state: &RuntimeState, slot: usize) -> bool {
    state.health[slot].as_ref().is_none_or(|h| h.value > 0.0)
}

impl InteractSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let source = &state.compiled.source;
        let program = source.resources.get("behavior.program");
        let bindings: InputBindings = match source.resources.get("input.bindings") {
            Some(v) => serde_json::from_value(v.clone())?,
            None => InputBindings::default(),
        };
        let mut system = Self {
            targets: vec![],
            target_ids: vec![],
            filters: vec![],
            events: vec![],
            actions: vec![],
            union: vec![],
            names: vec![],
            focus: vec![],
            union_index: vec![],
            poses: vec![],
            best: vec![],
            target_pivots: vec![],
            scanned: 0,
            pairs: 0,
            published: None,
            local_cached: None,
            lost: false,
            recent: VecDeque::with_capacity(RECENT),
            used: 0,
            halted: None,
            fault: None,
        };
        for (slot, e) in state.compiled.entities.iter().enumerate() {
            let Some(t) = &e.interact else { continue };
            ensure!(
                state.transforms[slot].is_some(),
                "interactable {} needs core.transform",
                e.id
            );
            let filter = match system.filters.iter().position(|f| f.def == t.interactor) {
                Some(i) => i,
                None => {
                    system.filters.push(Filter {
                        def: t.interactor.clone(),
                        slots: vec![],
                    });
                    system.filters.len() - 1
                }
            };
            let event = if t.event.is_empty() {
                None
            } else if let Some(i) = system.events.iter().position(|x| x.name == t.event) {
                Some(i)
            } else {
                let fields = program
                    .and_then(|p| {
                        p.pointer(&format!(
                            "/events/{}/fields",
                            axiom_core::local::escape(&t.event)
                        ))
                    })
                    .and_then(Value::as_object)
                    .with_context(|| {
                        format!(
                            "interactable emits {}, which behavior.program does not declare",
                            t.event
                        )
                    })?;
                system.events.push(EventInfo {
                    name: t.event.clone(),
                    interactor_field: fields.contains_key("interactor"),
                    target_field: fields.contains_key("target"),
                });
                Some(system.events.len() - 1)
            };
            let action = if t.action.is_empty() {
                None
            } else if let Some(i) = system.actions.iter().position(|a| a.name == t.action) {
                Some(i)
            } else {
                let code = bindings.buttons.get(&t.action).with_context(|| {
                    format!("input.bindings has no button {} ({})", t.action, e.id)
                })?;
                system.actions.push(ActionInfo {
                    name: t.action.clone(),
                    key: key_label(code).into(),
                    held: false,
                });
                Some(system.actions.len() - 1)
            };
            system.targets.push(Target {
                slot,
                range: t.range,
                margin: t.exit_margin,
                min_cos: (t.view_angle < 360.0).then(|| (t.view_angle.to_radians() * 0.5).cos()),
                prompt: t.prompt_text().into(),
                action,
                event,
                cooldown_ticks: (t.cooldown / FIXED_DT).round() as u64,
                once: t.once,
                enabled: t.enabled,
                priority: t.priority,
                filter,
                uses: 0,
                ready_at: 0,
            });
            system.target_ids.push(e.id.clone());
        }
        ensure!(
            system.targets.len() <= MAX_INTERACTABLES,
            "interactable budget exceeded ({} > {MAX_INTERACTABLES})",
            system.targets.len()
        );
        if !system.events.is_empty() {
            ensure!(
                source.systems.iter().any(|s| s == "behavior.execute"),
                "interaction events need the behavior.execute system"
            );
        }
        system.target_pivots = vec![Vec3::ZERO; system.targets.len()];
        system.scan(state, true);
        if let Some(reason) = system.halted.take() {
            anyhow::bail!(reason);
        }
        Ok(system)
    }

    /// Matches entity slots added since the last scan (all of them on construction).
    fn scan(&mut self, state: &RuntimeState, initial: bool) {
        let len = state.compiled.len();
        for slot in self.scanned..len {
            let e = &state.compiled.entities[slot];
            if !initial && e.interact.is_some() {
                self.halted = Some(format!(
                    "runtime-spawned entity {} has interact.target; spawned interactables are not supported (author the target instead)",
                    e.id
                ));
                return;
            }
            if state.transforms[slot].is_none() {
                continue;
            }
            let mut any = false;
            for f in &mut self.filters {
                if f.def.matches(&e.tags, |c| e.components.contains_key(c)) {
                    f.slots.push(slot as u32);
                    any = true;
                }
            }
            if any {
                self.union.push(slot as u32);
                self.names.push(e.id.clone());
                self.focus.push(None);
            }
        }
        self.scanned = len;
        self.union_index.clear();
        self.union_index.resize(len, NONE);
        for (i, &s) in self.union.iter().enumerate() {
            self.union_index[s as usize] = i as u32;
        }
        self.poses
            .resize(self.union.len(), (Vec3::ZERO, Vec2::ZERO, false));
        self.best.resize(self.union.len(), None);
        self.pairs = self
            .targets
            .iter()
            .map(|t| self.filters[t.filter].slots.len())
            .sum();
        if self.pairs > MAX_INTERACT_PAIRS {
            self.halted = Some(format!(
                "interaction candidate budget exceeded ({} interactor x target tests > {MAX_INTERACT_PAIRS} per step)",
                self.pairs
            ));
        }
    }

    /// The local interactor (keyboard and prompt): the first enabled, living candidate.
    fn local(&self, state: &RuntimeState) -> Option<usize> {
        self.union
            .iter()
            .position(|&s| state.compiled.enabled[s as usize] && alive(state, s as usize))
    }

    /// World pivot and XZ facing (world +Z) of an entity.
    fn pose(state: &RuntimeState, slot: usize) -> (Vec3, Vec2) {
        let m = world_matrix(state, slot);
        let forward = m.transform_vector3(Vec3::Z);
        (
            m.w_axis.truncate(),
            Vec2::new(forward.x, forward.z).normalize_or_zero(),
        )
    }

    /// Target-level eligibility (enabled, active scene, once, cooldown), independent of who
    /// asks and where they stand.
    fn ready(&self, state: &RuntimeState, t: usize) -> Result<(), Reason> {
        let target = &self.targets[t];
        if !target.enabled {
            return Err(Reason::Disabled);
        }
        if !state.compiled.enabled[target.slot] {
            return Err(Reason::Inactive);
        }
        if target.once && target.uses > 0 {
            return Err(Reason::Spent);
        }
        if state.tick < target.ready_at {
            return Err(Reason::Cooling(target.ready_at - state.tick));
        }
        Ok(())
    }

    /// Eligibility of target `t` for the interactor at `pose`, ignoring the filter and the
    /// interactor's own state. `focused` widens the range by the exit margin.
    fn check(
        &self,
        state: &RuntimeState,
        t: usize,
        here: Vec3,
        facing: Vec2,
        at: Vec3,
        focused: bool,
    ) -> Result<Candidate, Reason> {
        self.ready(state, t)?;
        let target = &self.targets[t];
        let reach = if focused {
            target.range + target.margin
        } else {
            target.range
        };
        // Compares the length itself (not squared) so the boundary matches FSM near/far
        // conditions bit for bit.
        let distance = (at - here).length();
        if distance > reach {
            return Err(Reason::OutOfRange(distance));
        }
        let flat = Vec2::new(at.x - here.x, at.z - here.z);
        let cos = if flat.length_squared() < 1e-8 || facing == Vec2::ZERO {
            1.0
        } else {
            facing.dot(flat.normalize())
        };
        if target.min_cos.is_some_and(|min| cos < min) {
            return Err(Reason::Facing);
        }
        Ok(Candidate {
            target: t as u16,
            priority: target.priority,
            distance,
            cos,
        })
    }

    /// Recomputes every interactor's focus (Σ targets × filter candidates tests).
    fn refocus(&mut self, state: &RuntimeState) {
        for (i, &s) in self.union.iter().enumerate() {
            let slot = s as usize;
            let usable = state.compiled.enabled[slot] && alive(state, slot);
            self.poses[i] = if usable {
                let (p, f) = Self::pose(state, slot);
                (p, f, true)
            } else {
                (Vec3::ZERO, Vec2::ZERO, false)
            };
        }
        self.best.fill(None);
        for t in 0..self.targets.len() {
            self.target_pivots[t] = pivot(state, self.targets[t].slot);
        }
        for t in 0..self.targets.len() {
            let filter = self.targets[t].filter;
            let at = self.target_pivots[t];
            for k in 0..self.filters[filter].slots.len() {
                let s = self.filters[filter].slots[k] as usize;
                if s == self.targets[t].slot {
                    continue;
                }
                let u = self.union_index[s] as usize;
                let (here, facing, usable) = self.poses[u];
                if !usable {
                    continue;
                }
                let focused = self.focus[u] == Some(t as u16);
                if let Ok(c) = self.check(state, t, here, facing, at, focused) {
                    if self.best[u].is_none_or(|b| c.beats(&b)) {
                        self.best[u] = Some(c);
                    }
                }
            }
        }
        for (f, b) in self.focus.iter_mut().zip(&self.best) {
            *f = b.map(|c| c.target);
        }
    }

    /// Shows the local interactor's focus through the gameplay `prompt` binding.
    fn publish(&mut self, state: &mut RuntimeState) {
        self.local_cached = self.local(state);
        let focus = self.local_cached.and_then(|u| self.focus[u]);
        let Some(g) = state.gameplay.as_mut() else {
            return;
        };
        if self.published == focus && (focus.is_some() == g.prompt.is_some()) {
            return;
        }
        self.published = focus;
        g.prompt = focus.map(|t| {
            let target = &self.targets[t as usize];
            Prompt {
                key: target
                    .action
                    .map_or_else(|| Arc::from(""), |a| self.actions[a].key.clone()),
                text: target.prompt.clone(),
            }
        });
    }

    /// Uses target `t` on behalf of union member `u`: counters, cooldown, event.
    fn apply(&mut self, state: &mut RuntimeState, u: usize, t: usize, by_key: bool) {
        let tick = state.tick;
        let target = &mut self.targets[t];
        target.uses = target.uses.saturating_add(1);
        target.ready_at = tick.saturating_add(target.cooldown_ticks);
        self.used += 1;
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(Use {
            tick,
            interactor: self.union[u],
            target: t as u16,
            by_key,
        });
        let Some(event) = self.targets[t].event else {
            return;
        };
        let info = &self.events[event];
        let mut fields = BTreeMap::new();
        if info.interactor_field {
            fields.insert("interactor".to_owned(), Scalar::Text(self.names[u].clone()));
        }
        if info.target_field {
            fields.insert(
                "target".to_owned(),
                Scalar::Text(self.target_ids[t].clone()),
            );
        }
        let result = match state.gameplay.as_mut() {
            Some(g) => g.command(
                &RuntimeCommand::Emit {
                    event: info.name.clone(),
                    fields,
                },
                tick,
            ),
            None => Err(anyhow::anyhow!("behavior.execute is not running")),
        };
        match result {
            Ok(()) => state.sync_scenes(),
            // Keep running; surface the first failure through observe.
            Err(e) => {
                let name = &self.events[event].name;
                self.fault
                    .get_or_insert_with(|| format!("emit {name}: {e:#}"));
            }
        }
    }

    fn target_index(&self, id: &str) -> Option<usize> {
        self.target_ids.iter().position(|x| x == id)
    }

    fn interactor_index(&self, id: &str) -> Option<usize> {
        self.names.iter().position(|x| x == id)
    }

    /// `interact.use`: the interactor (default: local) uses `target` if it is eligible now,
    /// or its current focus.
    fn use_command(&mut self, state: &mut RuntimeState, args: &Value) -> Result<Value> {
        let not_found =
            |path: &str, message: String| axiom_core::Diagnostic::error("not_found", path, message);
        let unavailable =
            |message: String| axiom_core::Diagnostic::error("not_interactable", "/args", message);
        if let Some(reason) = &self.halted {
            return Err(unavailable(format!("interact.focus halted: {reason}")));
        }
        let u = match args.get("interactor").and_then(Value::as_str) {
            Some(id) => self.interactor_index(id).ok_or_else(|| {
                not_found(
                    "/args/interactor",
                    format!("{id} matches no interact.target interactor filter"),
                )
            })?,
            None => self.local(state).ok_or_else(|| {
                unavailable("no enabled, living interactor (pass args.interactor)".into())
            })?,
        };
        let slot = self.union[u] as usize;
        if !(state.compiled.enabled[slot] && alive(state, slot)) {
            return Err(unavailable(format!(
                "{}: {}",
                self.names[u],
                Reason::InteractorUnavailable.text(0.0)
            )));
        }
        let t = match args.get("target").and_then(Value::as_str) {
            Some(id) => {
                let t = self.target_index(id).ok_or_else(|| {
                    not_found("/args/target", format!("{id} is not an interact.target"))
                })?;
                let filter = &self.filters[self.targets[t].filter];
                if filter.slots.binary_search(&(slot as u32)).is_err()
                    || self.targets[t].slot == slot
                {
                    return Err(unavailable(format!(
                        "{} cannot use {id}: {}",
                        self.names[u],
                        Reason::Filter.text(0.0)
                    )));
                }
                let (here, facing) = Self::pose(state, slot);
                let at = pivot(state, self.targets[t].slot);
                let focused = self.focus[u] == Some(t as u16);
                if let Err(reason) = self.check(state, t, here, facing, at, focused) {
                    return Err(unavailable(format!(
                        "{} cannot use {id}: {}",
                        self.names[u],
                        reason.text(self.targets[t].range)
                    )));
                }
                t
            }
            None => {
                let t = self.focus[u].ok_or_else(|| {
                    unavailable(format!("{} has no target in range", self.names[u]))
                })? as usize;
                // Focus is from the last step; the target may have been used, disabled or
                // deactivated since (range / facing stay as the prompt showed them).
                if let Err(reason) = self.ready(state, t) {
                    return Err(unavailable(reason.text(0.0)));
                }
                t
            }
        };
        self.apply(state, u, t, false);
        Ok(json!({
            "interactor":self.names[u],
            "target":self.target_ids[t],
            "event":self.targets[t].event.map(|e| self.events[e].name.as_str()),
            "uses":self.targets[t].uses
        }))
    }

    fn inspect(&self, state: &RuntimeState, id: &str) -> Result<Value> {
        if let Some(t) = self.target_index(id) {
            let target = &self.targets[t];
            let focused_by: Vec<&str> = self
                .focus
                .iter()
                .enumerate()
                .filter(|(_, f)| **f == Some(t as u16))
                .map(|(u, _)| self.names[u].as_str())
                .collect();
            return Ok(json!({
                "id":id,"kind":"target","enabled":target.enabled,
                "active":state.compiled.enabled[target.slot],
                "range":target.range,"exit_margin":target.margin,"priority":target.priority,
                "prompt":&*target.prompt,
                "action":target.action.map(|a| self.actions[a].name.as_str()),
                "key":target.action.map(|a| &*self.actions[a].key),
                "event":target.event.map(|e| self.events[e].name.as_str()),
                "once":target.once,"uses":target.uses,
                "cooldown_ticks_left":target.ready_at.saturating_sub(state.tick),
                "focused_by":focused_by,
                "candidates":self.filters[target.filter].slots.len()
            }));
        }
        let u = self.interactor_index(id).ok_or_else(|| {
            axiom_core::Diagnostic::error(
                "not_found",
                "/args/id",
                format!("{id} is neither an interact.target nor an interactor"),
            )
        })?;
        let slot = self.union[u] as usize;
        let usable = state.compiled.enabled[slot] && alive(state, slot);
        let (here, facing) = Self::pose(state, slot);
        let targets: Vec<Value> = (0..self.targets.len())
            .filter(|&t| {
                self.targets[t].slot != slot
                    && self.filters[self.targets[t].filter]
                        .slots
                        .binary_search(&(slot as u32))
                        .is_ok()
            })
            .take(OBSERVED)
            .map(|t| {
                let at = pivot(state, self.targets[t].slot);
                let focused = self.focus[u] == Some(t as u16);
                let distance = (at - here).length();
                match self.check(state, t, here, facing, at, focused) {
                    Ok(_) if usable => {
                        json!({"id":self.target_ids[t],"eligible":true,"distance":distance})
                    }
                    Ok(_) => {
                        json!({"id":self.target_ids[t],"eligible":false,"distance":distance,"reason":Reason::InteractorUnavailable.text(0.0)})
                    }
                    Err(r) => {
                        json!({"id":self.target_ids[t],"eligible":false,"distance":distance,"reason":r.text(self.targets[t].range)})
                    }
                }
            })
            .collect();
        Ok(json!({
            "id":id,"kind":"interactor","usable":usable,
            "local":self.local(state) == Some(u),
            "focus":self.focus[u].map(|t| self.target_ids[t as usize].as_str()),
            "targets":targets
        }))
    }

    fn summary(&self) -> Value {
        let focus: serde_json::Map<String, Value> = self
            .focus
            .iter()
            .enumerate()
            .filter_map(|(u, f)| {
                Some((
                    self.names[u].clone(),
                    json!(self.target_ids[(*f)? as usize]),
                ))
            })
            .take(OBSERVED)
            .collect();
        let name = |slot: u32| {
            self.union
                .binary_search(&slot)
                .map_or("?", |i| self.names[i].as_str())
        };
        let recent: Vec<Value> = self
            .recent
            .iter()
            .map(|r| {
                json!([
                    r.tick,
                    name(r.interactor),
                    self.target_ids[r.target as usize],
                    if r.by_key { "key" } else { "call" }
                ])
            })
            .collect();
        json!({
            "targets":self.targets.len(),
            "interactors":self.union.len(),
            "local":self.local_cached.map(|u| self.names[u].as_str()),
            "focus":focus,
            "recent":recent,
            "used":self.used,
            "pairs":self.pairs,
            "halted":self.halted,
            "fault":self.fault,
        })
    }

    /// Editor / Play overlay helper: (interactor pivot, focused target pivot) per focus.
    pub fn focus_lines(&self, state: &RuntimeState) -> Vec<(Vec3, Vec3)> {
        self.focus
            .iter()
            .enumerate()
            .filter_map(|(u, f)| {
                let t = (*f)? as usize;
                Some((
                    pivot(state, self.union[u] as usize),
                    pivot(state, self.targets[t].slot),
                ))
            })
            .collect()
    }
}

impl RuntimeSystem for InteractSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, input: &ActionMap) {
        if self.halted.is_some() {
            return;
        }
        if state.compiled.len() > self.scanned {
            self.scan(state, false);
            if self.halted.is_some() {
                // Nothing is focused any more: do not leave a prompt that cannot be used.
                self.focus.fill(None);
                self.publish(state);
                return;
            }
        }
        // Routed away by input contexts this step: buttons count as held, so a key still
        // down when input comes back is not a new press.
        if std::mem::take(&mut self.lost) {
            for a in &mut self.actions {
                a.held = true;
            }
            self.refocus(state);
            self.publish(state);
            return;
        }
        // Input first: a fresh press uses the focus the prompt showed after the last step.
        let local = self.local(state);
        for a in 0..self.actions.len() {
            let pressed = input.pressed(&self.actions[a].name);
            let edge = pressed && !self.actions[a].held;
            self.actions[a].held = pressed;
            if !edge {
                continue;
            }
            let Some(u) = local else { continue };
            let Some(t) = self.focus[u].map(usize::from) else {
                continue;
            };
            if self.targets[t].action == Some(a) && self.ready(state, t).is_ok() {
                self.apply(state, u, t, true);
            }
        }
        self.refocus(state);
        self.publish(state);
    }
    fn input_lost(&mut self) {
        // Called right before a step with empty input; that step keeps the buttons held.
        self.lost = true;
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        // Hot reload keeps uses / cooldowns per target entity ID, focus per interactor ID and
        // held buttons per action name (a held key does not fire again).
        let Some(old) = old.as_any().downcast_ref::<Self>() else {
            return Ok(());
        };
        for (t, id) in self.target_ids.iter().enumerate() {
            if let Some(o) = old.target_index(id) {
                self.targets[t].uses = old.targets[o].uses;
                self.targets[t].ready_at = old.targets[o].ready_at;
            }
        }
        for (u, id) in self.names.iter().enumerate() {
            let Some(o) = old.interactor_index(id) else {
                continue;
            };
            self.focus[u] = old.focus[o]
                .and_then(|t| self.target_index(&old.target_ids[t as usize]))
                .map(|t| t as u16);
        }
        for a in &mut self.actions {
            a.held = old.actions.iter().any(|o| o.name == a.name && o.held);
        }
        for r in &old.recent {
            let interactor = old
                .union
                .binary_search(&r.interactor)
                .ok()
                .and_then(|i| self.interactor_index(&old.names[i]));
            let target = self.target_index(&old.target_ids[r.target as usize]);
            if let (Some(u), Some(t)) = (interactor, target) {
                self.recent.push_back(Use {
                    interactor: self.union[u],
                    target: t as u16,
                    ..*r
                });
            }
        }
        self.used = old.used;
        self.fault = old.fault.clone();
        // The edit may have changed range, filters or enabled: re-validate the carried focus
        // (hysteresis keeps it while still in reach) before showing it.
        self.refocus(next);
        self.publish(next);
        Ok(())
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "interact" {
            return None;
        }
        Some(match action {
            "summary" => Ok(self.summary()),
            "use" => self.use_command(state, args),
            "inspect" => args
                .get("id")
                .and_then(Value::as_str)
                .context("interact.inspect needs {\"id\":\"target or interactor entity\"}")
                .and_then(|id| self.inspect(state, id)),
            other => Err(anyhow::anyhow!("unknown interact action {other}")),
        })
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("interact".into(), self.summary())])
    }
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.action(
        "interact.use",
        "interact",
        json!({"interactor":"optional entity ID (default: the local interactor)","target":"optional interact.target entity ID (default: the interactor's focus); must be eligible now (range, view_angle, enabled, cooldown, once)"}),
        |v| {
            let object = v.as_object().context("interact.use arguments must be an object")?;
            for (k, value) in object {
                ensure!(
                    k == "interactor" || k == "target",
                    "interact.use takes interactor and target, not {k}"
                );
                ensure!(value.is_string(), "{k} must be an entity ID string");
            }
            Ok(())
        },
    );
    r.authoring.action(
        "interact.inspect",
        "interact",
        json!({"id":"interact.target entity ID (state, uses, cooldown, who focuses it) or interactor ID (focus and every target with eligibility and reason)"}),
        |v| {
            ensure!(
                v.get("id").and_then(Value::as_str).is_some(),
                "interact.inspect needs id"
            );
            Ok(())
        },
    );
    r.authoring
        .action("interact.summary", "interact", json!({}), |_| Ok(()));
    r.factories.insert("interact.focus".into(), |state| {
        Ok(Box::new(InteractSystem::new(state)?))
    });
}
