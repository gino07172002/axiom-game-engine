//! State machine graph, behavior template and rule table operations (re-exported from
//! [`crate::ops`]): `fsm.machines` states and transitions, graph node positions in the
//! `editor.layout` resource (editor metadata the runtime never reads), `fsm.agent` with
//! its machine, the Inspector ▸ Behavior templates, `behavior.program` rules, and the
//! static checks the graph editor and Problems show. Pure like the rest of `ops`.
use crate::gameplay::ScalarType;
use crate::local::{escape, Collection, Selection, Target};
use crate::ops_world::{add_component, add_system, declare_event, requirements};
use crate::*;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The resource holding every state machine (registered by the runtime's `fsm` capability).
pub const FSM_RESOURCE: &str = "fsm.machines";
/// Editor-only metadata: graph node positions (`graphs.KEY.NODE = [x, y]`).
pub const LAYOUT_RESOURCE: &str = "editor.layout";
/// Budgets of the runtime's `fsm.machines` validator, checked here for clearer messages.
pub const MAX_MACHINES: usize = 64;
pub const MAX_STATES: usize = 32;
pub const MAX_TRANSITIONS: usize = 64;
/// `editor.layout` budgets.
pub const MAX_LAYOUT_GRAPHS: usize = 256;
pub const MAX_LAYOUT_NODES: usize = 64;
pub const MAX_LAYOUT_COORD: f32 = 100_000.0;
/// What a new transition waits for: one second in its source state (120 fixed ticks).
pub const DEFAULT_TRANSITION_TICKS: u32 = 120;

/// `editor.layout`: positions of graph nodes per graph (`fsm/MACHINE`), in canvas units.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorLayout {
    #[serde(default)]
    pub graphs: BTreeMap<String, BTreeMap<String, [f32; 2]>>,
}

impl EditorLayout {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.graphs.len() <= MAX_LAYOUT_GRAPHS,
            "editor.layout: at most {MAX_LAYOUT_GRAPHS} graphs"
        );
        for (key, nodes) in &self.graphs {
            anyhow::ensure!(
                !key.is_empty() && key.len() <= 192,
                "editor.layout: graph key must be 1..192 bytes"
            );
            anyhow::ensure!(
                nodes.len() <= MAX_LAYOUT_NODES,
                "editor.layout: graph {key} has more than {MAX_LAYOUT_NODES} nodes"
            );
            for (node, [x, y]) in nodes {
                anyhow::ensure!(
                    !node.is_empty() && node.len() <= 128,
                    "editor.layout: node names are 1..128 bytes"
                );
                anyhow::ensure!(
                    [x, y]
                        .iter()
                        .all(|c| c.is_finite() && c.abs() <= MAX_LAYOUT_COORD),
                    "editor.layout: {key}/{node} position must be finite within ±{MAX_LAYOUT_COORD}"
                );
            }
        }
        Ok(())
    }
}

/// The `editor.layout` graph key of a state machine.
pub fn fsm_graph_key(machine: &str) -> String {
    format!("fsm/{machine}")
}

/// The resource holding every animation controller (registered by the `animation` capability).
pub const ANIM_RESOURCE: &str = "animation.controllers";

/// Which state graph an edit targets: an `fsm.machines` machine or an
/// `animation.controllers` controller. Both are `{initial, states:{NAME:{..}},
/// transitions:[{from, to, ..}]}`, so the same state / transition / layout operations (and
/// the same Graphs canvas) serve both; only the state and transition bodies differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphDoc {
    #[default]
    Fsm,
    Animator,
}

impl GraphDoc {
    pub fn resource(self) -> &'static str {
        match self {
            Self::Fsm => FSM_RESOURCE,
            Self::Animator => ANIM_RESOURCE,
        }
    }
    /// The object key of the graphs inside the resource.
    pub fn root(self) -> &'static str {
        match self {
            Self::Fsm => "machines",
            Self::Animator => "controllers",
        }
    }
    fn short(self) -> &'static str {
        match self {
            Self::Fsm => "machine",
            Self::Animator => "controller",
        }
    }
    fn noun(self) -> &'static str {
        match self {
            Self::Fsm => "state machine",
            Self::Animator => "animation controller",
        }
    }
    /// The ACL verb (error hints).
    fn verb(self) -> &'static str {
        match self {
            Self::Fsm => "fsm",
            Self::Animator => "animctl",
        }
    }
    /// The `editor.layout` graph key of graph `id`.
    pub fn layout_key(self, id: &str) -> String {
        match self {
            Self::Fsm => fsm_graph_key(id),
            Self::Animator => format!("anim/{id}"),
        }
    }
    /// The graph `id` in `world`, if any.
    pub fn graph<'a>(self, world: &'a World, id: &str) -> Option<&'a Value> {
        world
            .resources
            .get(self.resource())?
            .get(self.root())?
            .get(id)
    }
    /// IDs of every graph of this kind.
    pub fn ids(self, world: &World) -> Vec<String> {
        world
            .resources
            .get(self.resource())
            .and_then(|r| r.get(self.root()))
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
    /// The body of a state added without one.
    fn default_state(self) -> Value {
        match self {
            Self::Fsm => json!({"action":{"kind":"stop"}}),
            Self::Animator => json!({}),
        }
    }
}

fn machine_path(g: GraphDoc, machine: &str) -> String {
    format!(
        "/resources/{}/{}/{}",
        g.resource(),
        g.root(),
        escape(machine)
    )
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}

fn check_name(path: &str, what: &str, name: &str) -> Result<()> {
    if valid_id(name).is_err() || name == "*" {
        return Err(Diagnostic::error(
            "invalid_id",
            path,
            format!("invalid {what} '{name}' (1..128 of letters, digits, . _ / -)"),
        ));
    }
    Ok(())
}

/// The resource value to edit (an empty one when the world has none).
fn machines_doc(world: &World, g: GraphDoc) -> Value {
    world
        .resources
        .get(g.resource())
        .cloned()
        .unwrap_or_else(|| json!({ g.root(): {} }))
}

fn machines_map(g: GraphDoc, doc: &mut Value) -> Result<&mut Map<String, Value>> {
    doc.get_mut(g.root())
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            invalid(
                format!("/resources/{}/{}", g.resource(), g.root()),
                format!("{} {} must be an object", g.resource(), g.root()),
            )
        })
}

fn machine_mut<'a>(
    g: GraphDoc,
    doc: &'a mut Value,
    machine: &str,
) -> Result<&'a mut Map<String, Value>> {
    let machines = machines_map(g, doc)?;
    if !machines.contains_key(machine) {
        let names: Vec<&str> = machines.keys().take(8).map(String::as_str).collect();
        let message = format!(
            "no {} '{machine}' ({}: {})",
            g.noun(),
            g.root(),
            names.join(", ")
        );
        return Err(Diagnostic::error(
            "not_found",
            machine_path(g, machine),
            message,
        ));
    }
    machines
        .get_mut(machine)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            invalid(
                machine_path(g, machine),
                format!("a {} must be an object", g.short()),
            )
        })
}

fn states_mut<'a>(
    g: GraphDoc,
    m: &'a mut Map<String, Value>,
    machine: &str,
) -> Result<&'a mut Map<String, Value>> {
    m.entry("states")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            invalid(
                format!("{}/states", machine_path(g, machine)),
                "states must be an object",
            )
        })
}

fn transitions_mut<'a>(
    g: GraphDoc,
    m: &'a mut Map<String, Value>,
    machine: &str,
) -> Result<&'a mut Vec<Value>> {
    m.entry("transitions")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| {
            invalid(
                format!("{}/transitions", machine_path(g, machine)),
                "transitions must be an array",
            )
        })
}

fn require_state(
    g: GraphDoc,
    states: &Map<String, Value>,
    machine: &str,
    name: &str,
) -> Result<()> {
    if !states.contains_key(name) {
        return Err(Diagnostic::error(
            "not_found",
            format!("{}/states/{}", machine_path(g, machine), escape(name)),
            format!("{} '{machine}' has no state '{name}'", g.short()),
        ));
    }
    Ok(())
}

fn set_machines(g: GraphDoc, doc: Value) -> Edit {
    Edit::Core(CoreEdit::SetResource {
        id: g.resource().into(),
        value: doc,
    })
}

/// Capability / system edits a world needs before it can hold the graph resource (for
/// `fsm.machines` the `fsm` capability and `fsm.step`); nothing when the registry does not
/// know the resource.
fn graph_requirements(world: &World, registry: &Registry, g: GraphDoc) -> Vec<Edit> {
    if world.resources.contains_key(g.resource()) {
        return vec![];
    }
    crate::ops_world::requirement_edits(world, registry, &requirements(registry, g.resource()))
}

/// A machine with one state `initial` that does nothing.
fn new_machine(initial: &str) -> Value {
    json!({
        "initial": initial,
        "states": {initial: {"action": {"kind": "stop"}}},
        "transitions": []
    })
}

/// Graphs ▸ New machine / ACL `fsm M create [INITIAL]`: a machine with one `stop` state
/// (default `idle`), creating `fsm.machines` (+ the `fsm` capability and `fsm.step`) when
/// the world has none. An existing machine ID is refused.
pub fn fsm_create_machine(
    world: &World,
    registry: &Registry,
    machine: &str,
    initial: Option<&str>,
) -> Result<Vec<Edit>> {
    insert_machine(
        world,
        registry,
        GraphDoc::Fsm,
        machine,
        new_machine(initial.unwrap_or("idle")),
    )
}

pub(crate) fn insert_graph(
    world: &World,
    registry: &Registry,
    g: GraphDoc,
    machine: &str,
    value: Value,
) -> Result<Vec<Edit>> {
    insert_machine(world, registry, g, machine, value)
}

fn insert_machine(
    world: &World,
    registry: &Registry,
    g: GraphDoc,
    machine: &str,
    value: Value,
) -> Result<Vec<Edit>> {
    check_name(
        &machine_path(g, machine),
        &format!("{} id", g.short()),
        machine,
    )?;
    if let Some(initial) = value["initial"].as_str() {
        check_name(
            &format!("{}/initial", machine_path(g, machine)),
            "state name",
            initial,
        )?;
    }
    let mut doc = machines_doc(world, g);
    let machines = machines_map(g, &mut doc)?;
    if machines.contains_key(machine) {
        return Err(Diagnostic::error(
            "id_taken",
            machine_path(g, machine),
            format!("{} '{machine}' already exists", g.noun()),
        ));
    }
    if machines.len() >= MAX_MACHINES {
        return Err(invalid(
            machine_path(g, machine),
            format!("at most {MAX_MACHINES} {}s", g.noun()),
        ));
    }
    machines.insert(machine.to_owned(), value);
    let mut edits = graph_requirements(world, registry, g);
    edits.push(set_machines(g, doc));
    Ok(edits)
}

/// Entities and prefabs whose `fsm.agent` runs `machine` (or whose `animation.controller`
/// names controller `machine`).
fn machine_users(world: &World, g: GraphDoc, machine: &str) -> Vec<String> {
    let (component, field) = match g {
        GraphDoc::Fsm => ("fsm.agent", "machine"),
        GraphDoc::Animator => ("animation.controller", "controller"),
    };
    let uses = |c: &BTreeMap<String, Value>| {
        c.get(component)
            .is_some_and(|a| a[field].as_str() == Some(machine))
    };
    let mut users: Vec<String> = world
        .entities
        .iter()
        .filter(|e| uses(&e.components))
        .map(|e| e.id.clone())
        .collect();
    users.extend(
        world
            .prefabs
            .iter()
            .filter(|(_, p)| uses(&p.components))
            .map(|(id, _)| format!("prefab {id}")),
    );
    users
}

/// Graphs ▸ Delete machine / ACL `fsm M remove`. Refused while an `fsm.agent` runs it; its
/// graph positions go with it.
pub fn fsm_remove_machine(world: &World, machine: &str) -> Result<Vec<Edit>> {
    graph_remove(world, GraphDoc::Fsm, machine)
}

/// [`fsm_remove_machine`] for either graph kind (ACL `animctl C remove`).
pub fn graph_remove(world: &World, g: GraphDoc, machine: &str) -> Result<Vec<Edit>> {
    let mut doc = machines_doc(world, g);
    machine_mut(g, &mut doc, machine)?;
    let users = machine_users(world, g, machine);
    if !users.is_empty() {
        let component = match g {
            GraphDoc::Fsm => "fsm.agent",
            GraphDoc::Animator => "animation.controller",
        };
        return Err(Diagnostic::error(
            "machine_in_use",
            machine_path(g, machine),
            format!(
                "{} '{machine}' is used by {}; give them another {} or remove their {component} first",
                g.noun(),
                users.join(", "),
                g.short()
            ),
        ));
    }
    machines_map(g, &mut doc)?.remove(machine);
    let mut edits = vec![set_machines(g, doc)];
    edits.extend(layout_edit(world, &g.layout_key(machine), |nodes| {
        nodes.clear();
    })?);
    Ok(edits)
}

/// One edit of a machine's states or transitions (ACL `fsm M state|initial|transition ...`).
#[derive(Clone, Debug, PartialEq)]
pub enum FsmEdit {
    /// New state; `None` = `{"action":{"kind":"stop"}}`.
    AddState {
        name: String,
        state: Option<Value>,
    },
    /// Removes the state and every transition from or to it (not the initial state).
    RemoveState(String),
    /// Renames a state; the initial state, transitions and graph position follow.
    RenameState {
        from: String,
        to: String,
    },
    /// Replaces a whole state (`{action, emit?, clip?, clip_loop?}`).
    SetState {
        name: String,
        state: Value,
    },
    SetInitial(String),
    /// New transition (last = lowest priority); `None` = wait [`DEFAULT_TRANSITION_TICKS`].
    AddTransition {
        from: String,
        to: String,
        when: Option<Value>,
    },
    RemoveTransition(usize),
    /// Moves the transition at the first index to the second (priority order).
    MoveTransition(usize, usize),
    /// Replaces a whole transition (`{from, to, when}`).
    SetTransition(usize, Value),
}

/// A change of one graph's node positions in `editor.layout`.
type LayoutChange = Box<dyn FnOnce(&mut BTreeMap<String, [f32; 2]>)>;

/// Graph editor / ACL `fsm M state ...`, `fsm M initial S`, `fsm M transition ...`: one
/// edit of machine `machine`, written as the whole `fsm.machines` value (plus the
/// `editor.layout` position change of a renamed or removed state). The runtime validator
/// checks actions and conditions at commit; this checks names, references and budgets.
pub fn fsm_edit(world: &World, machine: &str, edit: &FsmEdit) -> Result<Vec<Edit>> {
    graph_edit(world, GraphDoc::Fsm, machine, edit)
}

/// [`fsm_edit`] for either graph kind (ACL `animctl C state|initial|transition ...`). An
/// animation transition added without a body waits one cycle of its source state
/// (`exit_time` 1) and cross-fades 0.2 s; a body given is merged into `{from, to}`.
pub fn graph_edit(world: &World, g: GraphDoc, machine: &str, edit: &FsmEdit) -> Result<Vec<Edit>> {
    let mut doc = machines_doc(world, g);
    let m = machine_mut(g, &mut doc, machine)?;
    let base = machine_path(g, machine);
    let state_path = |name: &str| format!("{base}/states/{}", escape(name));
    let mut layout: Option<LayoutChange> = None;
    match edit {
        FsmEdit::AddState { name, state } => {
            check_name(&state_path(name), "state name", name)?;
            let states = states_mut(g, m, machine)?;
            if states.contains_key(name) {
                return Err(Diagnostic::error(
                    "id_taken",
                    state_path(name),
                    format!("{} '{machine}' already has a state '{name}'", g.short()),
                ));
            }
            if states.len() >= MAX_STATES {
                return Err(invalid(
                    state_path(name),
                    format!("a {} has at most {MAX_STATES} states", g.short()),
                ));
            }
            let state = state.clone().unwrap_or_else(|| g.default_state());
            if !state.is_object() {
                return Err(invalid(state_path(name), "a state must be a JSON object"));
            }
            states.insert(name.clone(), state);
        }
        FsmEdit::RemoveState(name) => {
            require_state(g, states_mut(g, m, machine)?, machine, name)?;
            if m.get("initial").and_then(Value::as_str) == Some(name) {
                return Err(Diagnostic::error(
                    "fsm_initial",
                    state_path(name),
                    format!(
                        "'{name}' is the initial state of '{machine}'; make another state initial first ({} {machine} initial OTHER)",
                        g.verb()
                    ),
                ));
            }
            states_mut(g, m, machine)?.remove(name);
            transitions_mut(g, m, machine)?
                .retain(|t| t["from"].as_str() != Some(name) && t["to"].as_str() != Some(name));
            let name = name.clone();
            layout = Some(Box::new(move |nodes: &mut BTreeMap<String, [f32; 2]>| {
                nodes.remove(&name);
            }));
        }
        FsmEdit::RenameState { from, to } => {
            check_name(&state_path(to), "state name", to)?;
            let states = states_mut(g, m, machine)?;
            require_state(g, states, machine, from)?;
            if from == to {
                return Ok(vec![]);
            }
            if states.contains_key(to) {
                return Err(Diagnostic::error(
                    "id_taken",
                    state_path(to),
                    format!("machine '{machine}' already has a state '{to}'"),
                ));
            }
            let def = states.remove(from).expect("checked above");
            states.insert(to.clone(), def);
            if m.get("initial").and_then(Value::as_str) == Some(from) {
                m.insert("initial".into(), json!(to));
            }
            for t in transitions_mut(g, m, machine)?.iter_mut() {
                for end in ["from", "to"] {
                    if t[end].as_str() == Some(from) {
                        t[end] = json!(to);
                    }
                }
            }
            let (from, to) = (from.clone(), to.clone());
            layout = Some(Box::new(move |nodes: &mut BTreeMap<String, [f32; 2]>| {
                if let Some(at) = nodes.remove(&from) {
                    nodes.insert(to, at);
                }
            }));
        }
        FsmEdit::SetState { name, state } => {
            let states = states_mut(g, m, machine)?;
            require_state(g, states, machine, name)?;
            if !state.is_object() {
                return Err(invalid(state_path(name), "a state must be a JSON object"));
            }
            if states.get(name) == Some(state) {
                return Ok(vec![]);
            }
            states.insert(name.clone(), state.clone());
        }
        FsmEdit::SetInitial(name) => {
            require_state(g, states_mut(g, m, machine)?, machine, name)?;
            if m.get("initial").and_then(Value::as_str) == Some(name) {
                return Ok(vec![]);
            }
            m.insert("initial".into(), json!(name));
        }
        FsmEdit::AddTransition { from, to, when } => {
            let transition = match g {
                GraphDoc::Fsm => json!({
                    "from": from,
                    "to": to,
                    "when": when.clone().unwrap_or_else(|| json!({"kind":"in_state_ticks","ticks":DEFAULT_TRANSITION_TICKS})),
                }),
                GraphDoc::Animator => {
                    let mut t = json!({"from": from, "to": to, "duration": 0.2});
                    match when {
                        None => t["exit_time"] = json!(1.0),
                        Some(Value::Object(body)) => {
                            for (k, v) in body {
                                if k != "from" && k != "to" {
                                    t[k] = v.clone();
                                }
                            }
                        }
                        Some(_) => {
                            return Err(invalid(
                                format!("{base}/transitions"),
                                "a transition body is an object {\"conditions\":[..],\"duration\":S,\"exit_time\":N}",
                            ))
                        }
                    }
                    t
                }
            };
            check_transition(g, m, machine, &transition, None)?;
            let list = transitions_mut(g, m, machine)?;
            if list.len() >= MAX_TRANSITIONS {
                return Err(invalid(
                    format!("{base}/transitions"),
                    format!("a machine has at most {MAX_TRANSITIONS} transitions"),
                ));
            }
            list.push(transition);
        }
        FsmEdit::RemoveTransition(i) => {
            let list = transitions_mut(g, m, machine)?;
            check_index(&base, *i, list.len())?;
            list.remove(*i);
        }
        FsmEdit::MoveTransition(from, to) => {
            let list = transitions_mut(g, m, machine)?;
            check_index(&base, *from.max(to), list.len())?;
            if from == to {
                return Ok(vec![]);
            }
            let t = list.remove(*from);
            list.insert(*to, t);
        }
        FsmEdit::SetTransition(i, transition) => {
            check_index(&base, *i, transitions_mut(g, m, machine)?.len())?;
            check_transition(g, m, machine, transition, Some(*i))?;
            let list = transitions_mut(g, m, machine)?;
            if &list[*i] == transition {
                return Ok(vec![]);
            }
            list[*i] = transition.clone();
        }
    }
    let mut edits = vec![set_machines(g, doc)];
    if let Some(change) = layout {
        edits.extend(layout_edit(world, &g.layout_key(machine), change)?);
    }
    Ok(edits)
}

fn check_index(base: &str, i: usize, len: usize) -> Result<()> {
    if i >= len {
        return Err(Diagnostic::error(
            "index_out_of_range",
            format!("{base}/transitions"),
            format!("transition {i} is out of range: the machine has {len} transition(s)"),
        ));
    }
    Ok(())
}

/// `{from, to, when}` with known states (`from` may be `*`), different ends and a `when`
/// object; `at` = the index being replaced (error paths).
fn check_transition(
    g: GraphDoc,
    m: &mut Map<String, Value>,
    machine: &str,
    t: &Value,
    at: Option<usize>,
) -> Result<()> {
    let base = machine_path(g, machine);
    let path = match at {
        Some(i) => format!("{base}/transitions/{i}"),
        None => format!("{base}/transitions"),
    };
    let (Some(from), Some(to), Some(_)) = (
        t.get("from").and_then(Value::as_str),
        t.get("to").and_then(Value::as_str),
        match g {
            GraphDoc::Fsm => t.get("when").filter(|w| w.is_object()),
            GraphDoc::Animator => t.as_object().map(|_| t),
        },
    ) else {
        return Err(invalid(
            path,
            match g {
                GraphDoc::Fsm => "a transition is {\"from\":STATE|\"*\",\"to\":STATE,\"when\":{\"kind\":..}}",
                GraphDoc::Animator => "a transition is {\"from\":STATE|\"*\",\"to\":STATE,\"conditions\":[..],\"duration\":S,\"exit_time\":N}",
            },
        ));
    };
    let states = states_mut(g, m, machine)?;
    if from != "*" {
        require_state(g, states, machine, from)?;
    }
    require_state(g, states, machine, to)?;
    if from == to {
        return Err(invalid(
            path,
            format!("a transition from '{from}' to itself never fires; pick another target"),
        ));
    }
    Ok(())
}

/// Edits `editor.layout` graph `key` with `change`; no edit when nothing changes (or there
/// is no layout and the change adds nothing).
fn layout_edit(
    world: &World,
    key: &str,
    change: impl FnOnce(&mut BTreeMap<String, [f32; 2]>),
) -> Result<Vec<Edit>> {
    let mut layout: EditorLayout = match world.resources.get(LAYOUT_RESOURCE) {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| {
            invalid(
                format!("/resources/{LAYOUT_RESOURCE}"),
                format!("editor.layout is malformed: {e}"),
            )
        })?,
        None => EditorLayout::default(),
    };
    let before = layout.clone();
    let nodes = layout.graphs.entry(key.to_owned()).or_default();
    change(nodes);
    if nodes.is_empty() {
        layout.graphs.remove(key);
    }
    if layout == before
        || (!world.resources.contains_key(LAYOUT_RESOURCE) && layout.graphs.is_empty())
    {
        return Ok(vec![]);
    }
    layout
        .validate()
        .map_err(|e| invalid(format!("/resources/{LAYOUT_RESOURCE}"), format!("{e:#}")))?;
    Ok(vec![Edit::Core(CoreEdit::SetResource {
        id: LAYOUT_RESOURCE.into(),
        value: json!(layout),
    })])
}

/// Graph editor node drag / ACL `fsm M node STATE x,y`: the state's graph position in
/// `editor.layout` (whole canvas units; the runtime never reads it). Unchanged = no edit.
pub fn fsm_set_node(world: &World, machine: &str, state: &str, at: [f32; 2]) -> Result<Vec<Edit>> {
    graph_set_node(world, GraphDoc::Fsm, machine, state, at)
}

/// [`fsm_set_node`] for either graph kind (ACL `animctl C node STATE x,y`).
pub fn graph_set_node(
    world: &World,
    g: GraphDoc,
    machine: &str,
    state: &str,
    at: [f32; 2],
) -> Result<Vec<Edit>> {
    let mut doc = machines_doc(world, g);
    let m = machine_mut(g, &mut doc, machine)?;
    require_state(g, states_mut(g, m, machine)?, machine, state)?;
    let at = at.map(f32::round);
    if !at
        .iter()
        .all(|c| c.is_finite() && c.abs() <= MAX_LAYOUT_COORD)
    {
        return Err(invalid(
            format!("/resources/{LAYOUT_RESOURCE}"),
            format!("node position must be finite within ±{MAX_LAYOUT_COORD}"),
        ));
    }
    // Positions of states removed or renamed through plain `set` are dropped on the way, so
    // they never count against the node budget or attach to a later state of that name.
    let states: BTreeSet<String> = states_mut(g, m, machine)?.keys().cloned().collect();
    let state = state.to_owned();
    layout_edit(world, &g.layout_key(machine), move |nodes| {
        nodes.retain(|name, _| states.contains(name));
        nodes.insert(state, at);
    })
}

/// Saved graph positions of a machine's states (empty when none are stored).
pub fn fsm_node_positions(world: &World, machine: &str) -> BTreeMap<String, [f32; 2]> {
    graph_node_positions(world, GraphDoc::Fsm, machine)
}

/// [`fsm_node_positions`] for either graph kind.
pub fn graph_node_positions(
    world: &World,
    g: GraphDoc,
    machine: &str,
) -> BTreeMap<String, [f32; 2]> {
    let key = g.layout_key(machine);
    world
        .resources
        .get(LAYOUT_RESOURCE)
        .and_then(|l| l.get("graphs"))
        .and_then(|graphs| graphs.get(&key))
        .and_then(|n| serde_json::from_value(n.clone()).ok())
        .unwrap_or_default()
}

/// Inspector ▸ Behavior ▸ FSM Agent / ACL `fsm SEL agent MACHINE [radius=M]`: `fsm.agent`
/// running `machine` on each entity (plus the capability and system it needs). A machine
/// that does not exist yet is created with one `idle` state in the same revision.
pub fn fsm_add_agent(
    world: &World,
    registry: &Registry,
    ids: &[String],
    machine: &str,
    radius: Option<f32>,
) -> Result<Vec<Edit>> {
    let mut edits = vec![];
    let known = world
        .resources
        .get(FSM_RESOURCE)
        .and_then(|r| r["machines"].get(machine))
        .is_some();
    let mut scratch;
    let mut on = world;
    if !known {
        edits = fsm_create_machine(world, registry, machine, None)?;
        scratch = world.clone();
        for e in &edits {
            e.clone().apply(&mut scratch, registry)?;
        }
        on = &scratch;
    }
    let mut agent = json!({"machine": machine});
    if let Some(r) = radius {
        if !(r.is_finite() && (0.05..=5.0).contains(&r)) {
            return Err(invalid(
                "/components/fsm.agent/radius",
                "radius must be 0.05..5 m",
            ));
        }
        agent["radius"] = json!(r);
    }
    edits.extend(add_component(on, registry, ids, "fsm.agent", Some(agent))?);
    Ok(edits)
}

/// Inspector ▸ Behavior templates (ACL `behavior SEL TEMPLATE ...`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BehaviorTemplate {
    /// Wander around the spawn point, pause, repeat.
    Patrol,
    /// Stand watch; chase the nearest `tag` that comes within `range`, give up beyond it.
    Guard,
    /// Idle; face the nearest `tag` within `range` and emit `event` (dialogue hook).
    Dialogue,
}

impl BehaviorTemplate {
    pub const ALL: [BehaviorTemplate; 3] = [Self::Patrol, Self::Guard, Self::Dialogue];
    pub fn name(self) -> &'static str {
        match self {
            Self::Patrol => "patrol",
            Self::Guard => "guard",
            Self::Dialogue => "dialogue",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// Options of a behavior template; `None` = the template default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BehaviorOptions {
    /// Machine ID (default: the template name, `NAME_2`, ... when taken).
    pub machine: Option<String>,
    /// Tag of the entities to watch (guard, dialogue; default `player`).
    pub tag: Option<String>,
    /// Meters (patrol: wander radius 4; guard: 6; dialogue: 2.4).
    pub range: Option<f32>,
    /// Meters per second (patrol 1.2, guard 2.5).
    pub speed: Option<f32>,
    /// Dialogue: event emitted on greeting (default `near_MACHINE`, declared with an `agent`
    /// text field when missing).
    pub event: Option<String>,
}

/// A machine ID for `base` that `fsm.machines` does not use yet (`base`, `base_2`, ...).
pub fn free_machine_id(world: &World, base: &str) -> String {
    let taken = |id: &str| {
        world
            .resources
            .get(FSM_RESOURCE)
            .and_then(|r| r["machines"].get(id))
            .is_some()
    };
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|id| !taken(id))
        .unwrap()
}

/// The machine a template builds (states, transitions) and the event it emits, if any.
pub fn behavior_machine(
    template: BehaviorTemplate,
    machine: &str,
    o: &BehaviorOptions,
) -> Result<(Value, Option<String>)> {
    let tag = o.tag.clone().unwrap_or_else(|| "player".into());
    check_name("/tag", "tag", &tag)?;
    let number = |v: Option<f32>, default: f32, lo: f32, hi: f32, what: &str| -> Result<f32> {
        let v = v.unwrap_or(default);
        if !(v.is_finite() && (lo..=hi).contains(&v)) {
            return Err(invalid(
                format!("/{what}"),
                format!("{what} must be {lo}..{hi}"),
            ));
        }
        Ok(v)
    };
    Ok(match template {
        BehaviorTemplate::Patrol => {
            let radius = number(o.range, 4.0, 0.1, 1000.0, "range")?;
            let speed = number(o.speed, 1.2, 0.0, 100.0, "speed")?;
            (
                json!({
                    "initial": "walk",
                    "states": {
                        "walk": {"action": {"kind":"wander","radius":radius,"speed":speed}},
                        "pause": {"action": {"kind":"stop"}}
                    },
                    "transitions": [
                        {"from":"walk","to":"pause","when":{"kind":"in_state_ticks","ticks":1200}},
                        {"from":"pause","to":"walk","when":{"kind":"in_state_ticks","ticks":360}}
                    ]
                }),
                None,
            )
        }
        BehaviorTemplate::Guard => {
            let range = number(o.range, 6.0, 0.0, 5000.0, "range")?;
            let speed = number(o.speed, 2.5, 0.0, 100.0, "speed")?;
            (
                json!({
                    "initial": "watch",
                    "states": {
                        "watch": {"action": {"kind":"face","tag":tag}},
                        "chase": {"action": {"kind":"move_toward","tag":tag,"speed":speed,"stop_within":1.2}}
                    },
                    "transitions": [
                        {"from":"watch","to":"chase","when":{"kind":"near","tag":tag,"within":range}},
                        {"from":"chase","to":"watch","when":{"kind":"far","tag":tag,"beyond":range * 1.5}}
                    ]
                }),
                None,
            )
        }
        BehaviorTemplate::Dialogue => {
            let range = number(o.range, 2.4, 0.0, 5000.0, "range")?;
            let event = o.event.clone().unwrap_or_else(|| format!("near_{machine}"));
            check_name("/event", "event name", &event)?;
            (
                json!({
                    "initial": "idle",
                    "states": {
                        "idle": {"action": {"kind":"stop"}},
                        "talk": {"action": {"kind":"face","tag":tag},"emit":event}
                    },
                    "transitions": [
                        {"from":"idle","to":"talk","when":{"kind":"near","tag":tag,"within":range}},
                        {"from":"talk","to":"idle","when":{"kind":"far","tag":tag,"beyond":range + 0.8}}
                    ]
                }),
                Some(event),
            )
        }
    })
}

/// Inspector ▸ Behavior ▸ Patrol / Guard / NPC Dialogue (ACL `behavior SEL TEMPLATE
/// [machine=ID] [tag=T] [range=M] [speed=M/S] [event=E]`): a new machine from the template,
/// its event declared (dialogue), and `fsm.agent` running it on each entity.
pub fn add_behavior(
    world: &World,
    registry: &Registry,
    ids: &[String],
    template: BehaviorTemplate,
    options: &BehaviorOptions,
) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the objects that should get the behavior",
        ));
    }
    let machine = match &options.machine {
        Some(m) => m.clone(),
        None => free_machine_id(world, template.name()),
    };
    let (value, event) = behavior_machine(template, &machine, options)?;
    let mut edits = vec![];
    let mut scratch = world.clone();
    let mut push = |batch: Vec<Edit>, scratch: &mut World| -> Result<()> {
        for e in &batch {
            e.clone().apply(scratch, registry)?;
        }
        edits.extend(batch);
        Ok(())
    };
    if let Some(event) = &event {
        // An event already declared with no fields or only a text `agent` field works as is
        // (the FSM fills `agent` when declared); anything else is refused by declare_event.
        let fits = scratch
            .resources
            .get("behavior.program")
            .and_then(|p| p["events"].get(event))
            .and_then(|e| e.get("fields"))
            .and_then(Value::as_object)
            .is_some_and(|f| f.iter().all(|(k, t)| k == "agent" && t == "text"));
        let batch = if fits {
            match registry.systems.contains_key("behavior.execute") {
                true => add_system(&scratch, registry, "behavior.execute")?,
                false => vec![],
            }
        } else {
            declare_event(&scratch, event, &[("agent".into(), ScalarType::Text)])?
        };
        push(batch, &mut scratch)?;
    }
    let batch = insert_machine(&scratch, registry, GraphDoc::Fsm, &machine, value)?;
    push(batch, &mut scratch)?;
    let batch = add_component(
        &scratch,
        registry,
        ids,
        "fsm.agent",
        Some(json!({"machine": machine})),
    )?;
    push(batch, &mut scratch)?;
    Ok(edits)
}

/// The `behavior.program` rules list (`resources[id=behavior.program] rules`).
pub fn program_selection() -> Selection {
    let mut s = Selection::all(Collection::Resources);
    s.ids = vec!["behavior.program".into()];
    s
}

/// A new rule for the rule table / ACL `append resources[id=behavior.program] rules JSON`:
/// a free ID (`rule`, `rule_2`, ...), `event` (default: the first declared event) and no
/// actions yet.
pub fn new_rule(world: &World, event: Option<&str>) -> Result<Value> {
    let program = world.resources.get("behavior.program");
    let rules = program.and_then(|p| p["rules"].as_array());
    let taken = |id: &str| rules.is_some_and(|r| r.iter().any(|x| x["id"] == id));
    let id = if taken("rule") {
        (2..)
            .map(|n| format!("rule_{n}"))
            .find(|id| !taken(id))
            .unwrap()
    } else {
        "rule".to_owned()
    };
    let event = match event {
        Some(e) => e.to_owned(),
        None => program
            .and_then(|p| p["events"].as_object())
            .and_then(|e| e.keys().next().cloned())
            .ok_or_else(|| {
                Diagnostic::error(
                    "no_event",
                    "/resources/behavior.program/events",
                    "a rule runs on an event; declare one first: event declare NAME",
                )
            })?,
    };
    Ok(json!({"id": id, "event": event, "actions": []}))
}

/// Rule table cell edit / ACL `set rules[id=ID] FIELD VALUE` (`event`, `when`, `actions`).
pub fn set_rule(world: &World, id: &str, field: &str, value: Value) -> Result<Vec<Edit>> {
    let target = Target::Rule { id: id.to_owned() };
    let exists = world
        .resources
        .get("behavior.program")
        .and_then(|p| p["rules"].as_array())
        .is_some_and(|r| r.iter().any(|x| x["id"] == id));
    if !exists {
        return Err(Diagnostic::error(
            "not_found",
            target.path(),
            format!("behavior.program has no rule '{id}'"),
        ));
    }
    if !matches!(field, "id" | "event" | "when" | "actions") {
        return Err(invalid(
            format!("{}/{field}", target.path()),
            "rule fields are id, event, when, actions",
        ));
    }
    Ok(vec![Edit::Core(CoreEdit::SetField {
        target,
        field: format!("/{field}"),
        value,
    })])
}

/// A static check of the authored state machines (graph editor and Problems).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphProblem {
    /// `fsm_unreachable`, `fsm_dangling`, `fsm_undeclared_event`, `fsm_event_fields`,
    /// `fsm_unknown_machine`.
    pub code: &'static str,
    pub machine: String,
    /// The state the problem is about (its node), if any.
    pub state: Option<String>,
    /// The transition index, if any.
    pub transition: Option<usize>,
    pub path: String,
    pub message: String,
}

/// Dangling transition ends and states the initial state cannot reach, for graph `id`
/// (`{initial, states, transitions}`), with codes `anim_dangling` / `anim_unreachable` (or
/// the `fsm_` ones). Bounded by the validator budgets.
pub(crate) fn graph_shape_problems(g: GraphDoc, id: &str, m: &Value, out: &mut Vec<GraphProblem>) {
    let (dangling, unreachable) = match g {
        GraphDoc::Fsm => ("fsm_dangling", "fsm_unreachable"),
        GraphDoc::Animator => ("anim_dangling", "anim_unreachable"),
    };
    let base = machine_path(g, id);
    let empty = Map::new();
    let states = m["states"].as_object().unwrap_or(&empty);
    let transitions: &[Value] = m["transitions"].as_array().map_or(&[], |t| t.as_slice());
    let initial = m["initial"].as_str().unwrap_or("");
    let mut problem = |code, state: Option<&str>, transition, path: String, message: String| {
        out.push(GraphProblem {
            code,
            machine: id.to_owned(),
            state: state.map(str::to_owned),
            transition,
            path,
            message,
        })
    };
    if !states.contains_key(initial) {
        problem(
            dangling,
            None,
            None,
            format!("{base}/initial"),
            format!("{id}: initial state '{initial}' does not exist"),
        );
    }
    for (i, t) in transitions.iter().enumerate().take(MAX_TRANSITIONS) {
        for end in ["from", "to"] {
            let name = t[end].as_str().unwrap_or("");
            if !states.contains_key(name) && !(end == "from" && name == "*") {
                problem(
                    dangling,
                    None,
                    Some(i),
                    format!("{base}/transitions/{i}/{end}"),
                    format!("{id}: transition {i} {end} '{name}' does not exist"),
                );
            }
        }
    }
    let mut reached = BTreeSet::new();
    let mut queue = VecDeque::new();
    if states.contains_key(initial) {
        reached.insert(initial);
        queue.push_back(initial);
    }
    while let Some(s) = queue.pop_front() {
        for t in transitions.iter().take(MAX_TRANSITIONS) {
            let from = t["from"].as_str().unwrap_or("");
            let Some(to) = t["to"].as_str().filter(|to| states.contains_key(*to)) else {
                continue;
            };
            if (from == s || from == "*") && reached.insert(to) {
                queue.push_back(to);
            }
        }
    }
    for name in states.keys().take(MAX_STATES) {
        if !reached.contains(name.as_str()) && states.contains_key(initial) {
            problem(
                unreachable,
                Some(name),
                None,
                format!("{base}/states/{}", escape(name)),
                format!("{id}: state '{name}' cannot be reached from '{initial}'"),
            );
        }
    }
}

/// Problems the runtime would hit or that make a graph misleading: states the initial
/// state can never reach, transitions to states that do not exist, entry events that
/// `behavior.program` does not declare (or declares with fields other than a text `agent`),
/// and `fsm.agent`s naming a missing machine. Bounded by the validator budgets.
pub fn fsm_problems(world: &World) -> Vec<GraphProblem> {
    let mut out = vec![];
    let Some(machines) = world
        .resources
        .get(FSM_RESOURCE)
        .and_then(|r| r["machines"].as_object())
    else {
        for e in world.entities.iter().take(65_536) {
            if let Some(m) = agent_machine(world, e) {
                unknown_machine(&mut out, &e.id, m);
            }
        }
        return out;
    };
    let events = world
        .resources
        .get("behavior.program")
        .and_then(|p| p["events"].as_object());
    for (id, m) in machines.iter().take(MAX_MACHINES) {
        let base = machine_path(GraphDoc::Fsm, id);
        let empty = Map::new();
        let states = m["states"].as_object().unwrap_or(&empty);
        let transitions: &[Value] = m["transitions"].as_array().map_or(&[], |t| t.as_slice());
        let initial = m["initial"].as_str().unwrap_or("");
        let mut problem = |code, state: Option<&str>, transition, path: String, message: String| {
            out.push(GraphProblem {
                code,
                machine: id.clone(),
                state: state.map(str::to_owned),
                transition,
                path,
                message,
            })
        };
        if !states.contains_key(initial) {
            problem(
                "fsm_dangling",
                None,
                None,
                format!("{base}/initial"),
                format!("{id}: initial state '{initial}' does not exist"),
            );
        }
        for (i, t) in transitions.iter().enumerate().take(MAX_TRANSITIONS) {
            for end in ["from", "to"] {
                let name = t[end].as_str().unwrap_or("");
                if !states.contains_key(name) && !(end == "from" && name == "*") {
                    problem(
                        "fsm_dangling",
                        None,
                        Some(i),
                        format!("{base}/transitions/{i}/{end}"),
                        format!("{id}: transition {i} {end} '{name}' does not exist"),
                    );
                }
            }
        }
        // Reachability from the initial state; `*` transitions leave every reached state.
        let mut reached = BTreeSet::new();
        let mut queue = VecDeque::new();
        if states.contains_key(initial) {
            reached.insert(initial);
            queue.push_back(initial);
        }
        while let Some(s) = queue.pop_front() {
            for t in transitions.iter().take(MAX_TRANSITIONS) {
                let from = t["from"].as_str().unwrap_or("");
                let Some(to) = t["to"].as_str().filter(|to| states.contains_key(*to)) else {
                    continue;
                };
                if (from == s || from == "*") && reached.insert(to) {
                    queue.push_back(to);
                }
            }
        }
        for (name, def) in states.iter().take(MAX_STATES) {
            let path = format!("{base}/states/{}", escape(name));
            if !reached.contains(name.as_str()) && states.contains_key(initial) {
                problem(
                    "fsm_unreachable",
                    Some(name),
                    None,
                    path.clone(),
                    format!("{id}: state '{name}' cannot be reached from '{initial}'"),
                );
            }
            let Some(event) = def["emit"].as_str() else {
                continue;
            };
            if !world.systems.iter().any(|s| s == "behavior.execute") {
                problem(
                    "fsm_needs_behavior",
                    Some(name),
                    None,
                    format!("{path}/emit"),
                    format!("{id}.{name} emits {event}; enable the behavior.execute system (system add behavior.execute)"),
                );
            }
            match events.and_then(|e| e.get(event)) {
                None => problem(
                    "fsm_undeclared_event",
                    Some(name),
                    None,
                    format!("{path}/emit"),
                    format!("{id}.{name} emits {event}, which behavior.program does not declare"),
                ),
                Some(decl) => {
                    let fields = decl["fields"].as_object();
                    // The runtime reads `fields` (it must be present, even empty).
                    let ok =
                        fields.is_some_and(|f| f.iter().all(|(k, t)| k == "agent" && t == "text"));
                    if !ok {
                        problem(
                            "fsm_event_fields",
                            Some(name),
                            None,
                            format!("{path}/emit"),
                            format!(
                                "{id}.{name} emits {event}; an FSM event may only declare a text `agent` field"
                            ),
                        );
                    }
                }
            }
        }
    }
    for e in world.entities.iter().take(65_536) {
        if let Some(m) = agent_machine(world, e) {
            if m.is_none_or(|m| !machines.contains_key(m)) {
                unknown_machine(&mut out, &e.id, m);
            }
        }
    }
    out
}

/// The machine an entity's `fsm.agent` runs (own override first, then its prefab's), without
/// copying components: `None` = no agent, `Some(None)` = an agent without a machine name.
fn agent_machine<'a>(world: &'a World, e: &'a Entity) -> Option<Option<&'a str>> {
    let prefab = e
        .prefab
        .as_ref()
        .and_then(|p| world.prefabs.get(p))
        .and_then(|p| p.components.get("fsm.agent"));
    let own = e.components.get("fsm.agent");
    if own.is_none() && prefab.is_none() {
        return None;
    }
    Some(
        own.and_then(|a| a["machine"].as_str())
            .or_else(|| prefab.and_then(|a| a["machine"].as_str())),
    )
}

fn unknown_machine(out: &mut Vec<GraphProblem>, entity: &str, machine: Option<&str>) {
    let machine = machine.unwrap_or("").to_owned();
    out.push(GraphProblem {
        code: "fsm_unknown_machine",
        message: format!(
            "{entity} runs fsm machine '{machine}', which fsm.machines does not define"
        ),
        path: format!("/entities/@{}/components/fsm.agent/machine", escape(entity)),
        machine,
        state: None,
        transition: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn town() -> World {
        let mut w = World::empty("t");
        w.resources.insert(
            FSM_RESOURCE.into(),
            json!({"machines":{"m":{"initial":"idle","states":{"idle":{"action":{"kind":"stop"}},"walk":{"action":{"kind":"stop"}}},
                "transitions":[{"from":"idle","to":"walk","when":{"kind":"always"}}]}}}),
        );
        w
    }

    fn apply(w: &World, edits: Vec<Edit>) -> World {
        let mut w = w.clone();
        for e in edits {
            e.apply(&mut w, &Registry::core()).unwrap();
        }
        w
    }

    fn code(e: anyhow::Error) -> String {
        e.downcast_ref::<Diagnostic>().unwrap().code.clone()
    }

    fn machine(w: &World) -> &Value {
        &w.resources[FSM_RESOURCE]["machines"]["m"]
    }

    #[test]
    fn states_and_transitions_edit_one_machine() {
        let w = town();
        let edit = |w: &World, e: FsmEdit| apply(w, fsm_edit(w, "m", &e).unwrap());
        let w = edit(
            &w,
            FsmEdit::AddState {
                name: "rest".into(),
                state: None,
            },
        );
        assert_eq!(
            machine(&w)["states"]["rest"],
            json!({"action":{"kind":"stop"}})
        );
        let w = edit(
            &w,
            FsmEdit::AddTransition {
                from: "walk".into(),
                to: "rest".into(),
                when: None,
            },
        );
        assert_eq!(
            machine(&w)["transitions"][1]["when"],
            json!({"kind":"in_state_ticks","ticks":120})
        );
        let w = apply(&w, fsm_set_node(&w, "m", "walk", [10.4, -3.6]).unwrap());
        assert_eq!(fsm_node_positions(&w, "m")["walk"], [10.0, -4.0]);
        // Rename: initial, transitions and the graph position follow.
        let w = edit(
            &w,
            FsmEdit::RenameState {
                from: "walk".into(),
                to: "stroll".into(),
            },
        );
        assert_eq!(machine(&w)["transitions"][0]["to"], "stroll");
        assert_eq!(machine(&w)["transitions"][1]["from"], "stroll");
        assert!(fsm_node_positions(&w, "m").contains_key("stroll"));
        let w = edit(&w, FsmEdit::MoveTransition(1, 0));
        assert_eq!(machine(&w)["transitions"][0]["to"], "rest");
        // Removing a state removes its transitions and its position.
        let w = edit(&w, FsmEdit::RemoveState("stroll".into()));
        assert_eq!(machine(&w)["transitions"], json!([]));
        assert!(fsm_node_positions(&w, "m").is_empty());
        assert_eq!(w.resources[LAYOUT_RESOURCE], json!({"graphs":{}}));
        // The initial state cannot be removed; self transitions are refused.
        assert_eq!(
            code(fsm_edit(&w, "m", &FsmEdit::RemoveState("idle".into())).unwrap_err()),
            "fsm_initial"
        );
        assert_eq!(
            code(
                fsm_edit(
                    &w,
                    "m",
                    &FsmEdit::AddTransition {
                        from: "rest".into(),
                        to: "rest".into(),
                        when: None
                    }
                )
                .unwrap_err()
            ),
            "invalid_value"
        );
        assert_eq!(
            code(fsm_edit(&w, "x", &FsmEdit::SetInitial("idle".into())).unwrap_err()),
            "not_found"
        );
        assert_eq!(
            code(fsm_edit(&w, "m", &FsmEdit::RemoveTransition(5)).unwrap_err()),
            "index_out_of_range"
        );
        assert!(fsm_edit(&w, "m", &FsmEdit::SetInitial("idle".into()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn problems_find_unreachable_dangling_and_undeclared() {
        let mut w = town();
        w.resources.get_mut(FSM_RESOURCE).unwrap()["machines"]["m"]["states"]["lost"] =
            json!({"action":{"kind":"stop"},"emit":"hello"});
        w.resources.get_mut(FSM_RESOURCE).unwrap()["machines"]["m"]["transitions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"from":"walk","to":"gone","when":{"kind":"always"}}));
        let codes: Vec<(&str, Option<String>)> = fsm_problems(&w)
            .iter()
            .map(|p| (p.code, p.state.clone()))
            .collect();
        assert!(
            codes.contains(&("fsm_unreachable", Some("lost".into()))),
            "{codes:?}"
        );
        assert!(codes.contains(&("fsm_dangling", None)));
        assert!(codes.contains(&("fsm_undeclared_event", Some("lost".into()))));
        assert!(!codes.iter().any(|(_, s)| s.as_deref() == Some("walk")));
    }

    #[test]
    fn templates_and_rules_build_valid_values() {
        let mut w = World::empty("t");
        w.entities.push(Entity::new("npc"));
        // fsm.agent is registered by the runtime; a stand-in keeps this test in core.
        let mut registry = Registry::core();
        registry.component("fsm.agent", "core", json!({}), |_| Ok(()));
        let edits = add_behavior(
            &w,
            &registry,
            &["npc".into()],
            BehaviorTemplate::Dialogue,
            &BehaviorOptions::default(),
        )
        .unwrap();
        let w = apply(&w, edits);
        assert_eq!(
            w.entity("npc").unwrap().components["fsm.agent"],
            json!({"machine":"dialogue"})
        );
        assert_eq!(
            w.resources["behavior.program"]["events"]["near_dialogue"],
            json!({"fields":{"agent":"text"}})
        );
        assert!(fsm_problems(&w).is_empty(), "{:?}", fsm_problems(&w));
        assert_eq!(free_machine_id(&w, "dialogue"), "dialogue_2");
        // An event already declared without fields is used as is (not refused as taken).
        let mut plain = World::empty("p");
        plain.entities.push(Entity::new("npc"));
        let plain = apply(&plain, declare_event(&plain, "hi", &[]).unwrap());
        let options = BehaviorOptions {
            event: Some("hi".into()),
            ..Default::default()
        };
        let edits = add_behavior(
            &plain,
            &registry,
            &["npc".into()],
            BehaviorTemplate::Dialogue,
            &options,
        )
        .unwrap();
        let plain = apply(&plain, edits);
        assert_eq!(
            plain.resources["behavior.program"]["events"]["hi"],
            json!({"fields":{}})
        );
        assert!(
            fsm_problems(&plain).is_empty(),
            "{:?}",
            fsm_problems(&plain)
        );
        let rule = new_rule(&w, None).unwrap();
        assert_eq!(
            rule,
            json!({"id":"rule","event":"near_dialogue","actions":[]})
        );
        assert_eq!(
            code(set_rule(&w, "nope", "when", json!({})).unwrap_err()),
            "not_found"
        );
        assert_eq!(
            code(new_rule(&World::empty("e"), None).unwrap_err()),
            "no_event"
        );
    }

    #[test]
    fn layout_validates_budgets() {
        let mut l = EditorLayout::default();
        l.graphs
            .entry("fsm/m".into())
            .or_default()
            .insert("a".into(), [f32::NAN, 0.0]);
        assert!(l.validate().is_err());
        let ok: EditorLayout =
            serde_json::from_value(json!({"graphs":{"fsm/m":{"a":[1,2]}}})).unwrap();
        assert!(ok.validate().is_ok());
        assert!(serde_json::from_value::<EditorLayout>(json!({"other":1})).is_err());
    }
}
