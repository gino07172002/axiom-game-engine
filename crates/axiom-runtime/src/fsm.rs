//! Generic data-defined entity behavior: per-entity finite state machines.
//!
//! Content declares machines once in the `fsm.machines` resource and attaches them with an
//! `fsm.agent` component. The runtime compiles state/transition names to indices, keeps agent
//! state in dense arrays, and evaluates at most one transition per agent per fixed tick.
//! `fsm.inspect` reports the current state, the last transition and every outgoing condition
//! with its measured value, so "why is this enemy not chasing?" is one bounded query.
use crate::world::*;
use anyhow::{ensure, Context, Result};
use axiom_core::*;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{any::Any, collections::BTreeMap};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActionDef {
    #[default]
    Stop,
    MoveToward {
        tag: String,
        speed: f32,
        #[serde(default)]
        stop_within: f32,
    },
    MoveAway {
        tag: String,
        speed: f32,
    },
    /// Damage the nearest `tag` target within `range` meters every `interval_ticks`.
    Attack {
        tag: String,
        range: f32,
        damage: f32,
        interval_ticks: u32,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Cond {
    /// Nearest enabled entity carrying `tag` is at most `within` meters away (XZ plane).
    Near {
        tag: String,
        within: f32,
    },
    /// No entity carrying `tag` is within `beyond` meters (or none exists).
    Far {
        tag: String,
        beyond: f32,
    },
    HealthBelow {
        value: f32,
    },
    /// The agent has stayed in its current state for at least `ticks` fixed steps.
    InStateTicks {
        ticks: u32,
    },
    /// Passes with probability `per_tick` each step, drawn from the seeded world stream.
    Chance {
        per_tick: f32,
    },
    Always,
}
impl Cond {
    fn describe(&self) -> String {
        match self {
            Self::Near { tag, within } => format!("near({tag})<={within}"),
            Self::Far { tag, beyond } => format!("far({tag})>{beyond}"),
            Self::HealthBelow { value } => format!("health<{value}"),
            Self::InStateTicks { ticks } => format!("ticks_in_state>={ticks}"),
            Self::Chance { per_tick } => format!("chance({per_tick}/tick)"),
            Self::Always => "always".into(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StateDef {
    #[serde(default)]
    pub action: ActionDef,
    /// `behavior.program` event emitted when an agent enters this state. If the event
    /// declares a text field `agent`, it receives the agent's entity ID.
    #[serde(default)]
    pub emit: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransitionDef {
    /// Source state name, or `*` for any state.
    pub from: String,
    pub to: String,
    pub when: Cond,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub initial: String,
    pub states: BTreeMap<String, StateDef>,
    #[serde(default)]
    pub transitions: Vec<TransitionDef>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Machines {
    pub machines: BTreeMap<String, Machine>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub machine: String,
}
fn finite(x: f32, lo: f32, hi: f32) -> bool {
    x.is_finite() && (lo..=hi).contains(&x)
}
impl Machines {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.machines.len() <= 64, "fsm machine budget is 64");
        for (id, m) in &self.machines {
            valid_id(id)?;
            ensure!(
                (1..=32).contains(&m.states.len()) && m.transitions.len() <= 64,
                "machine {id}: 1..32 states and up to 64 transitions"
            );
            ensure!(
                m.states.contains_key(&m.initial),
                "machine {id}: unknown initial state {}",
                m.initial
            );
            for (name, s) in &m.states {
                valid_id(name)?;
                ensure!(name != "*", "machine {id}: '*' is reserved");
                match &s.action {
                    ActionDef::Stop => {}
                    ActionDef::MoveToward {
                        tag,
                        speed,
                        stop_within,
                    } => {
                        valid_id(tag)?;
                        ensure!(
                            finite(*speed, 0.0, 100.0) && finite(*stop_within, 0.0, 1000.0),
                            "machine {id}.{name}: speed 0..100, stop_within 0..1000"
                        );
                    }
                    ActionDef::MoveAway { tag, speed } => {
                        valid_id(tag)?;
                        ensure!(
                            finite(*speed, 0.0, 100.0),
                            "machine {id}.{name}: speed 0..100"
                        );
                    }
                    ActionDef::Attack {
                        tag,
                        range,
                        damage,
                        interval_ticks,
                    } => {
                        valid_id(tag)?;
                        ensure!(
                            finite(*range, 0.0, 1000.0)
                                && finite(*damage, 0.0, 100000.0)
                                && (1..=1_000_000).contains(interval_ticks),
                            "machine {id}.{name}: range 0..1000, damage 0..100000, interval_ticks 1..1000000"
                        );
                    }
                }
                if let Some(event) = &s.emit {
                    valid_id(event)?;
                }
            }
            for t in &m.transitions {
                ensure!(
                    t.from == "*" || m.states.contains_key(&t.from),
                    "machine {id}: unknown state {}",
                    t.from
                );
                ensure!(
                    m.states.contains_key(&t.to),
                    "machine {id}: unknown state {}",
                    t.to
                );
                match &t.when {
                    Cond::Near { tag, within: d } | Cond::Far { tag, beyond: d } => {
                        valid_id(tag)?;
                        ensure!(finite(*d, 0.0, 10000.0), "machine {id}: distance 0..10000");
                    }
                    Cond::HealthBelow { value } => {
                        ensure!(value.is_finite(), "machine {id}: finite health")
                    }
                    Cond::InStateTicks { ticks } => {
                        ensure!(*ticks <= 1_000_000, "machine {id}: ticks <= 1000000")
                    }
                    Cond::Chance { per_tick } => ensure!(
                        finite(*per_tick, 0.0, 1.0),
                        "machine {id}: chance per_tick 0..1"
                    ),
                    Cond::Always => {}
                }
            }
        }
        Ok(())
    }
}

struct Compiled {
    id: String,
    states: Vec<(String, ActionDef)>,
    /// Event emitted on entering each state (parallel to `states`).
    emits: Vec<Option<String>>,
    initial: usize,
    /// Per source state: ordered (condition, target) list; `*` transitions are appended in order.
    outgoing: Vec<Vec<(Cond, usize)>>,
}
impl Compiled {
    fn new(id: &str, m: &Machine) -> Self {
        let states: Vec<(String, ActionDef)> = m
            .states
            .iter()
            .map(|(k, v)| (k.clone(), v.action.clone()))
            .collect();
        let index = |name: &str| states.iter().position(|(n, _)| n == name).unwrap();
        let mut outgoing = vec![vec![]; states.len()];
        for t in &m.transitions {
            let to = index(&t.to);
            if t.from == "*" {
                for (from, list) in outgoing.iter_mut().enumerate() {
                    if from != to {
                        list.push((t.when.clone(), to));
                    }
                }
            } else {
                outgoing[index(&t.from)].push((t.when.clone(), to));
            }
        }
        Self {
            id: id.into(),
            initial: index(&m.initial),
            emits: m.states.values().map(|s| s.emit.clone()).collect(),
            states,
            outgoing,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
struct Last {
    from: usize,
    to: usize,
    tick: u64,
}
/// Dense per-agent arrays.
#[derive(Default)]
struct Agents {
    ids: Vec<String>,
    slots: Vec<usize>,
    machine: Vec<usize>,
    state: Vec<usize>,
    ticks: Vec<u32>,
    last: Vec<Option<Last>>,
    cooldown: Vec<u32>,
}

pub struct FsmSystem {
    machines: Vec<Compiled>,
    agents: Agents,
    /// Emitted event -> whether its payload declares a text `agent` field.
    events: BTreeMap<String, bool>,
    hits: u64,
    emitted: u64,
    fault: Option<String>,
}

/// Nearest enabled, living entity carrying `tag`: (XZ distance, XZ direction, slot).
/// Entities whose health reached 0 are no longer targets; entities without health are.
fn nearest_slot(state: &RuntimeState, me: usize, tag: &str) -> Option<(f32, Vec3, usize)> {
    let here = Vec3::from_array(state.transforms[me].as_ref()?.position);
    state
        .compiled
        .entities
        .iter()
        .enumerate()
        .filter(|(i, e)| {
            *i != me
                && state.compiled.enabled[*i]
                && e.tags.iter().any(|t| t == tag)
                && state.health[*i].as_ref().is_none_or(|h| h.value > 0.0)
        })
        .filter_map(|(i, _)| {
            state.transforms[i]
                .as_ref()
                .map(|t| (Vec3::from_array(t.position), i))
        })
        .map(|(p, i)| {
            let d = Vec3::new(p.x - here.x, 0.0, p.z - here.z);
            (d.length(), d, i)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

fn nearest(state: &RuntimeState, me: usize, tag: &str) -> Option<(f32, Vec3)> {
    nearest_slot(state, me, tag).map(|(d, dir, _)| (d, dir))
}

impl FsmSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let defs: Machines = if state.compiled.source.resources.contains_key("fsm.machines") {
            state.compiled.source.resource("fsm.machines")?
        } else {
            Machines::default()
        };
        defs.validate()?;
        let machines: Vec<Compiled> = defs
            .machines
            .iter()
            .map(|(id, m)| Compiled::new(id, m))
            .collect();
        let mut agents = Agents::default();
        for (slot, e) in state.compiled.entities.iter().enumerate() {
            let Some(raw) = e.components.get("fsm.agent") else {
                continue;
            };
            let agent: Agent = serde_json::from_value(raw.clone())?;
            let m = machines
                .iter()
                .position(|c| c.id == agent.machine)
                .with_context(|| {
                    format!("entity {} uses unknown fsm machine {}", e.id, agent.machine)
                })?;
            ensure!(
                state.transforms[slot].is_some(),
                "fsm agent {} needs core.transform",
                e.id
            );
            agents.ids.push(e.id.clone());
            agents.slots.push(slot);
            agents.machine.push(m);
            agents.state.push(machines[m].initial);
            agents.ticks.push(0);
            agents.last.push(None);
            agents.cooldown.push(0);
        }
        // Every emitted event must exist in behavior.program and need at most an `agent` field.
        let mut events = BTreeMap::new();
        let program = state.compiled.source.resources.get("behavior.program");
        for m in defs.machines.values() {
            for event in m.states.values().filter_map(|s| s.emit.as_ref()) {
                let fields = program
                    .and_then(|p| {
                        p.pointer(&format!(
                            "/events/{}/fields",
                            axiom_core::local::escape(event)
                        ))
                    })
                    .and_then(Value::as_object)
                    .with_context(|| {
                        format!("fsm emits {event}, which behavior.program does not declare")
                    })?;
                ensure!(
                    fields.keys().all(|k| k == "agent")
                        && fields.get("agent").is_none_or(|t| t == "text"),
                    "fsm event {event} may only declare a text `agent` field"
                );
                ensure!(
                    state
                        .compiled
                        .source
                        .systems
                        .iter()
                        .any(|s| s == "behavior.execute"),
                    "fsm emits {event}; enable the behavior.execute system"
                );
                events.insert(event.clone(), fields.contains_key("agent"));
            }
        }
        Ok(Self {
            machines,
            agents,
            events,
            hits: 0,
            emitted: 0,
            fault: None,
        })
    }

    fn emit(&mut self, state: &mut RuntimeState, event: &str, agent: &str) {
        let mut fields = BTreeMap::new();
        if self.events.get(event) == Some(&true) {
            fields.insert(
                "agent".to_owned(),
                axiom_core::gameplay::Scalar::Text(agent.to_owned()),
            );
        }
        let tick = state.tick;
        let result = match state.gameplay.as_mut() {
            Some(g) => g.command(
                &axiom_core::gameplay::RuntimeCommand::Emit {
                    event: event.to_owned(),
                    fields,
                },
                tick,
            ),
            None => Err(anyhow::anyhow!("behavior.execute is not running")),
        };
        match result {
            Ok(_) => {
                self.emitted += 1;
                state.sync_scenes();
            }
            // Keep simulating; surface the first failure through summary/inspect/observe.
            Err(e) => {
                self.fault
                    .get_or_insert_with(|| format!("emit {event}: {e:#}"));
            }
        }
    }

    /// Measured value and verdict for one condition, without side effects.
    fn evaluate(&self, state: &RuntimeState, agent: usize, cond: &Cond) -> (Option<f32>, bool) {
        let slot = self.agents.slots[agent];
        match cond {
            Cond::Near { tag, within } => {
                let d = nearest(state, slot, tag).map(|(d, _)| d);
                (d, d.is_some_and(|d| d <= *within))
            }
            Cond::Far { tag, beyond } => {
                let d = nearest(state, slot, tag).map(|(d, _)| d);
                (d, d.is_none_or(|d| d > *beyond))
            }
            Cond::HealthBelow { value } => {
                let h = state.health[slot].as_ref().map(|h| h.value);
                (h, h.is_some_and(|h| h < *value))
            }
            Cond::InStateTicks { ticks } => {
                let t = self.agents.ticks[agent];
                (Some(t as f32), t >= *ticks)
            }
            // Inspection must not consume randomness; report the probability instead.
            Cond::Chance { per_tick } => (Some(*per_tick), false),
            Cond::Always => (None, true),
        }
    }

    fn inspect(&self, state: &RuntimeState, id: &str) -> Result<Value> {
        let a = self
            .agents
            .ids
            .iter()
            .position(|x| x == id)
            .ok_or_else(|| {
                Diagnostic::error("not_found", "/args/id", format!("{id} is not an fsm agent"))
            })?;
        let m = &self.machines[self.agents.machine[a]];
        let s = self.agents.state[a];
        let slot = self.agents.slots[a];
        let transitions: Vec<Value> = m.outgoing[s]
            .iter()
            .map(|(cond, to)| {
                let (measured, pass) = self.evaluate(state, a, cond);
                json!({"to":m.states[*to].0,"when":cond.describe(),"measured":measured,"pass":pass})
            })
            .collect();
        Ok(json!({
            "id":id,"machine":m.id,"state":m.states[s].0,"action":m.states[s].1,
            "ticks_in_state":self.agents.ticks[a],
            "cooldown":self.agents.cooldown[a],
            "last":self.agents.last[a].map(|l|json!({"from":m.states[l.from].0,"to":m.states[l.to].0,"tick":l.tick})),
            "position":state.transforms[slot].as_ref().map(|t|t.position),
            "health":state.health[slot].as_ref().map(|h|h.value),
            "transitions":transitions,
            "enabled":state.compiled.enabled[slot]
        }))
    }

    /// Overlay labels for debug views: (world position slightly above the agent, text).
    pub fn labels(&self, state: &RuntimeState) -> Vec<([f32; 3], String)> {
        (0..self.agents.ids.len())
            .filter(|&a| state.compiled.enabled[self.agents.slots[a]])
            .filter_map(|a| {
                let slot = self.agents.slots[a];
                let p = state.transforms[slot].as_ref()?.position;
                let m = &self.machines[self.agents.machine[a]];
                Some((
                    [p[0], p[1] + 1.2, p[2]],
                    m.states[self.agents.state[a]].0.clone(),
                ))
            })
            .collect()
    }

    fn summary(&self) -> Value {
        let mut counts = BTreeMap::<String, usize>::new();
        for a in 0..self.agents.ids.len() {
            let m = &self.machines[self.agents.machine[a]];
            *counts
                .entry(m.states[self.agents.state[a]].0.clone())
                .or_default() += 1;
        }
        json!({"agents":self.agents.ids.len(),"states":counts,"hits":self.hits,"emitted":self.emitted,"fault":self.fault})
    }
}

impl RuntimeSystem for FsmSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, _input: &ActionMap) {
        for a in 0..self.agents.ids.len() {
            let slot = self.agents.slots[a];
            if !state.compiled.enabled[slot] {
                continue;
            }
            let m = &self.machines[self.agents.machine[a]];
            let current = self.agents.state[a];
            let mut next = None;
            for (cond, to) in &m.outgoing[current] {
                let pass = match cond {
                    Cond::Chance { per_tick } => state.random_f32() < *per_tick,
                    other => self.evaluate(state, a, other).1,
                };
                if pass {
                    next = Some(*to);
                    break;
                }
            }
            let mi = self.agents.machine[a];
            let mut entered = None;
            if let Some(to) = next.filter(|to| *to != current) {
                self.agents.last[a] = Some(Last {
                    from: current,
                    to,
                    tick: state.tick,
                });
                self.agents.state[a] = to;
                self.agents.ticks[a] = 0;
                entered = self.machines[mi].emits[to].clone();
            } else {
                self.agents.ticks[a] = self.agents.ticks[a].saturating_add(1);
            }
            let s = self.agents.state[a];
            if let ActionDef::Attack {
                tag,
                range,
                damage,
                interval_ticks,
            } = &self.machines[mi].states[s].1
            {
                self.agents.cooldown[a] = self.agents.cooldown[a].saturating_sub(1);
                if self.agents.cooldown[a] == 0 {
                    let target = nearest_slot(state, slot, tag).filter(|(d, _, _)| *d <= *range);
                    if let Some(h) = target.and_then(|(_, _, t)| state.health[t].as_mut()) {
                        h.value = (h.value - damage).max(0.0);
                        self.hits += 1;
                        self.agents.cooldown[a] = *interval_ticks;
                    }
                }
            }
            if let Some(event) = entered {
                let id = self.agents.ids[a].clone();
                self.emit(state, &event, &id);
            }
            let action = &self.machines[mi].states[s].1;
            let step = match action {
                ActionDef::Stop => None,
                ActionDef::MoveToward {
                    tag,
                    speed,
                    stop_within,
                } => nearest(state, slot, tag)
                    .filter(|(d, _)| *d > *stop_within && *d > 1e-4)
                    .map(|(d, dir)| dir / d * speed.min((d - stop_within) / FIXED_DT) * FIXED_DT),
                ActionDef::MoveAway { tag, speed } => nearest(state, slot, tag)
                    .filter(|(d, _)| *d > 1e-4)
                    .map(|(d, dir)| -dir / d * *speed * FIXED_DT),
                ActionDef::Attack { .. } => None,
            };
            if let (Some(delta), Some(t)) = (step, state.transforms[slot].as_mut()) {
                t.position = (Vec3::from_array(t.position) + delta).to_array();
            }
        }
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        _next: &mut RuntimeState,
    ) -> Result<()> {
        // Hot reload keeps an agent's state when its ID, machine ID and state name survive.
        if let Some(old) = old.as_any().downcast_ref::<Self>() {
            for a in 0..self.agents.ids.len() {
                let Some(o) = old
                    .agents
                    .ids
                    .iter()
                    .position(|id| *id == self.agents.ids[a])
                else {
                    continue;
                };
                let (om, nm) = (
                    &old.machines[old.agents.machine[o]],
                    &self.machines[self.agents.machine[a]],
                );
                if om.id != nm.id {
                    continue;
                }
                let name = &om.states[old.agents.state[o]].0;
                if let Some(s) = nm.states.iter().position(|(n, _)| n == name) {
                    self.agents.state[a] = s;
                    self.agents.ticks[a] = old.agents.ticks[o];
                }
            }
        }
        Ok(())
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "fsm" {
            return None;
        }
        Some(match action {
            "inspect" => args
                .get("id")
                .and_then(Value::as_str)
                .context("fsm.inspect needs {\"id\":\"entity\"}")
                .and_then(|id| self.inspect(state, id)),
            "summary" => Ok(self.summary()),
            other => Err(anyhow::anyhow!("unknown fsm action {other}")),
        })
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("fsm".into(), self.summary())])
    }
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.capabilities.insert("fsm".into(), 1);
    r.authoring.resource(
        "fsm.machines",
        "fsm",
        json!({"machines":"id -> {initial, states:{name:{action:{kind:stop|move_toward{tag,speed,stop_within?}|move_away{tag,speed}|attack{tag,range,damage,interval_ticks}}, emit?: behavior.program event on enter (text field `agent` receives the agent ID)}}, transitions:[{from:state|*, to, when:{kind:near{tag,within}|far{tag,beyond}|health_below{value}|in_state_ticks{ticks}|chance{per_tick}|always}}]}; first passing transition per tick wins"}),
        |v| serde_json::from_value::<Machines>(v.clone())?.validate(),
    );
    r.authoring.component(
        "fsm.agent",
        "fsm",
        json!({"machine":"fsm.machines ID"}),
        |v| {
            let a: Agent = serde_json::from_value(v.clone())?;
            valid_id(&a.machine)
        },
    );
    r.authoring.action("fsm.inspect", "fsm", json!({"id":"agent entity ID; returns state, last transition and each outgoing condition with measured value"}), |v| {
        ensure!(v.get("id").and_then(Value::as_str).is_some(), "fsm.inspect needs id");
        Ok(())
    });
    r.authoring
        .action("fsm.summary", "fsm", json!({}), |_| Ok(()));
    r.authoring.systems.insert(
        "fsm.step".into(),
        SystemDescriptor {
            id: "fsm.step".into(),
            capability: "fsm".into(),
            phase: 8,
            reads: vec![
                "fsm.machines".into(),
                "fsm.agent".into(),
                "core.health".into(),
            ],
            writes: vec!["core.transform".into()],
            after: vec![],
        },
    );
    r.factories.insert("fsm.step".into(), |state| {
        Ok(Box::new(FsmSystem::new(state)?))
    });
}
