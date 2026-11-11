//! Graphs tab: the editable state machine graph (`fsm.machines`), the `behavior.program`
//! rule table and the read-only overview (narrative included).
//!
//! Every change is an `axiom_core::ops` call, the same code as ACL `fsm …`, `behavior …`,
//! `append|remove|reorder resources[id=behavior.program] rules …` and `set rules[id=…] …`,
//! committed as one revision per interaction (a node drag writes once, on release). The
//! canvas is self-made (rectangles, straight arrows with condition labels, drag to move or
//! to link): FSM transitions are ordered, may repeat between two states and may start from
//! any state (`*`), which a pin-to-pin wire set cannot represent (M6 ADR in
//! `docs/plan-editor-human-tuning.md`). The drawn model is rebuilt only when the world
//! version or the open machine changes; drawing is bounded by `MAX_DRAW_*`.
use super::{form, Action, Dock, Editor};
use crate::world::WorldRuntime;
use axiom_core::ops::{self, FsmEdit, GraphProblem};
use axiom_core::World;
use egui::{pos2, vec2, Color32, PointerButton, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::{json, Map, Value};

/// Node size on the canvas (canvas units = points at the default zoom).
pub(super) const NODE: Vec2 = vec2(156.0, 46.0);
/// Drawing budgets per frame (the runtime validator allows 32 states / 64 transitions).
pub(super) const MAX_DRAW_NODES: usize = 64;
pub(super) const MAX_DRAW_EDGES: usize = 128;
/// Width of the element / rule inspector next to the canvas.
const SIDE: f32 = 300.0;
const PORT: f32 = 6.0;
/// Canvas zoom limits (`graph.zoom`, wheel); Frame never magnifies past 100 %.
pub(super) const ZOOM_MIN: f32 = 0.25;
pub(super) const ZOOM_MAX: f32 = 2.0;
/// Fit margin around the framed states (screen points).
const FIT_MARGIN: f32 = 16.0;
/// Where the "Any state" pseudo node (source of `*` transitions) sits.
const ANY_AT: Pos2 = pos2(24.0, 20.0);

const MUTED: Color32 = Color32::from_rgb(155, 174, 194);
const ACCENT: Color32 = Color32::from_rgb(105, 222, 195);
const WARNING: Color32 = Color32::from_rgb(255, 178, 111);
const LIVE: Color32 = Color32::from_rgb(120, 220, 140);
const CANVAS: Color32 = Color32::from_rgb(17, 21, 28);
const NODE_FILL: Color32 = Color32::from_rgb(38, 47, 61);
const EDGE: Color32 = Color32::from_rgb(130, 150, 172);

/// Which view the Graphs tab shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum View {
    #[default]
    Machine,
    Rules,
    Overview,
}

/// The selected graph element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Sel {
    State(String),
    /// Index into the machine's `transitions`.
    Transition(usize),
}

/// Graphs tab state (view only; content lives in the world).
#[derive(Default)]
pub(crate) struct GraphUi {
    pub machine: Option<String>,
    pub view: View,
    model: Option<Model>,
    pub selected: Option<Sel>,
    drag: Option<NodeDrag>,
    /// A transition being dragged out of a node's port (`None` = the Any state).
    link: Option<Option<usize>>,
    panning: bool,
    pan: Vec2,
    /// Canvas scale (screen points per canvas unit); 0 before the first use = 100 %.
    zoom: f32,
    /// Size of the canvas when last drawn (`graph.zoom` zooms around its center).
    canvas_size: Vec2,
    /// Graph ▸ Frame: zoom out (never past 100 %) and pan so every state is in view on the
    /// next drawn frame.
    fit: bool,
    /// State name being typed in the inspector (rename).
    name: String,
    element: Option<Draft>,
    rule: Option<RuleDraft>,
    /// Selected rule (index into `rules`).
    pub rule_index: Option<usize>,
    problems: Option<((u64, u64), Vec<GraphProblem>)>,
    kinds: Option<Kinds>,
    /// Agents in each node's state this frame (Play), parallel to the model's nodes.
    live: Vec<u16>,
    context: Option<Hit>,
    /// The editor's world plus the graph writes queued since it last changed: several edits
    /// in one frame (or before the write is shown) build on each other instead of the
    /// later whole-resource write erasing the earlier one.
    scratch: Option<(u64, std::sync::Arc<World>)>,
    /// Graph writes queued so far (with the world version, the model / checks cache key).
    edits: u64,
    /// Writes made outside a tab pass (a draft committed because the selection changed).
    queued: Vec<Action>,
    /// Model rebuilds so far (tests: not per frame).
    pub builds: usize,
}

struct Model {
    version: u64,
    /// `GraphUi::edits` when built (queued writes show at once).
    edits: u64,
    machine: String,
    exists: bool,
    initial: String,
    nodes: Vec<Node>,
    edges: Vec<EdgeView>,
    /// The machine has `*` transitions (the Any state node is drawn).
    any: bool,
    transitions: usize,
    machines: Vec<String>,
    events: Vec<String>,
}

struct Node {
    name: String,
    at: Pos2,
    detail: String,
    problem: Option<String>,
}

struct EdgeView {
    index: usize,
    /// `None` = from any state.
    from: Option<usize>,
    to: usize,
    label: String,
    /// Sideways offset so parallel / opposite transitions do not overlap.
    lane: f32,
}

struct NodeDrag {
    node: usize,
    grab: Vec2,
    at: Pos2,
}

/// The selected state's or transition's value being edited (committed when released).
struct Draft {
    sel: Sel,
    base: Value,
    value: Value,
    dirty: bool,
}

struct RuleDraft {
    index: usize,
    /// The rule's ID when loaded (an index alone may point at another rule later).
    id: String,
    base: Value,
    value: Value,
    dirty: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Node(usize),
    Any,
    Edge(usize),
    Empty(Pos2),
}

/// A `kind`-tagged value: the template a newly picked kind starts with and the typed form
/// spec of its other fields.
struct Kind {
    name: &'static str,
    template: Value,
    spec: form::Spec,
}

/// Compiled once per editor (never per frame).
struct Kinds {
    actions: Vec<Kind>,
    conditions: Vec<Kind>,
    rule_conditions: Vec<Kind>,
    rule_actions: Vec<Kind>,
    operands: Vec<Kind>,
}

fn kinds(list: Vec<(&'static str, Value, Value)>) -> Vec<Kind> {
    list.into_iter()
        .map(|(name, template, schema)| Kind {
            name,
            template,
            spec: form::Spec::compile(&schema),
        })
        .collect()
}

impl Kinds {
    fn new() -> Self {
        let object = |p: Value| json!({"type":"object","properties":p});
        let event = json!({"type":"string","x-ref":"event"});
        let variable = json!({"type":"string","x-ref":"key:behavior.program/variables"});
        let text = |d: &str| json!({"type":"string","description":d});
        let operand = || json!({"kind":"literal","value":true});
        Kinds {
            actions: kinds(crate::fsm::editor_kinds(false)),
            conditions: kinds(crate::fsm::editor_kinds(true)),
            rule_conditions: kinds(vec![
                ("always", json!({"kind":"always"}), object(json!({}))),
                (
                    "eq",
                    json!({"kind":"eq","left":{"kind":"event","field":"agent"},"right":{"kind":"literal","value":""}}),
                    object(json!({})),
                ),
                (
                    "ge",
                    json!({"kind":"ge","left":{"kind":"variable","id":""},"right":{"kind":"literal","value":0}}),
                    object(json!({})),
                ),
                (
                    "all",
                    json!({"kind":"all","conditions":[]}),
                    object(
                        json!({"conditions":{"type":"array","description":"every condition must pass (edit as JSON)"}}),
                    ),
                ),
                (
                    "not",
                    json!({"kind":"not","condition":{"kind":"always"}}),
                    object(json!({})),
                ),
            ]),
            rule_actions: kinds(vec![
                (
                    "set",
                    json!({"kind":"set","variable":"","value":operand()}),
                    object(json!({"variable":variable})),
                ),
                (
                    "add",
                    json!({"kind":"add","variable":"","value":{"kind":"literal","value":1}}),
                    object(json!({"variable":variable})),
                ),
                (
                    "emit",
                    json!({"kind":"emit","event":"","fields":{}}),
                    object(
                        json!({"event":event,"fields":{"type":"object","description":"event field -> operand"}}),
                    ),
                ),
                (
                    "delay",
                    json!({"kind":"delay","ticks":120,"event":"","fields":{}}),
                    object(
                        json!({"ticks":{"type":"integer","minimum":1,"maximum":1000000,"x-unit":"ticks"},"event":event,"fields":{"type":"object"}}),
                    ),
                ),
                (
                    "go_to",
                    json!({"kind":"go_to","node":""}),
                    object(json!({"node":text("narrative.graph node")})),
                ),
                (
                    "scene",
                    json!({"kind":"scene","scene":""}),
                    object(json!({"scene":text("scene ID")})),
                ),
                (
                    "end",
                    json!({"kind":"end","ending":""}),
                    object(json!({"ending":text("ending ID")})),
                ),
            ]),
            operands: kinds(vec![
                (
                    "literal",
                    json!({"kind":"literal","value":0}),
                    object(json!({"value":{"description":"bool, integer, number or text"}})),
                ),
                (
                    "variable",
                    json!({"kind":"variable","id":""}),
                    object(json!({"id":variable})),
                ),
                (
                    "event",
                    json!({"kind":"event","field":""}),
                    object(json!({"field":text("field of the triggering event")})),
                ),
            ]),
        }
    }
}

/// Short text of a transition condition (edge labels).
fn condition_label(when: &Value) -> String {
    let tag = when["tag"].as_str().unwrap_or("?");
    match when["kind"].as_str().unwrap_or("?") {
        "near" => format!("near {tag} <= {}", when["within"]),
        "far" => format!("far {tag} > {}", when["beyond"]),
        "health_below" => format!("health < {}", when["value"]),
        "in_state_ticks" => format!("after {} ticks", when["ticks"]),
        "chance" => format!("chance {}", when["per_tick"]),
        other => other.to_owned(),
    }
}

/// Short text of a state's action, entry event and clip (node subtitle).
fn state_label(state: &Value) -> String {
    let a = &state["action"];
    let tag = a["tag"].as_str().unwrap_or("?");
    let mut s = match a["kind"].as_str().unwrap_or("stop") {
        "wander" => format!("wander {} m", a["radius"]),
        "move_toward" => format!("move to {tag}"),
        "move_away" => format!("flee {tag}"),
        "attack" => format!("attack {tag}"),
        "face" => format!("face {tag}"),
        other => other.to_owned(),
    };
    if let Some(e) = state["emit"].as_str() {
        s.push_str(&format!(" | emit {e}"));
    }
    if let Some(c) = state["clip"].as_str() {
        s.push_str(&format!(" | clip {c}"));
    }
    s
}

fn names(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

impl Model {
    fn build(world: &World, version: u64, machine: &str, problems: &[GraphProblem]) -> Self {
        let machines = names(
            world
                .resources
                .get(ops::FSM_RESOURCE)
                .and_then(|r| r.get("machines")),
        );
        let events = names(
            world
                .resources
                .get("behavior.program")
                .and_then(|p| p.get("events")),
        );
        let m = world
            .resources
            .get(ops::FSM_RESOURCE)
            .and_then(|r| r["machines"].get(machine));
        let empty = Map::new();
        let states = m.and_then(|m| m["states"].as_object()).unwrap_or(&empty);
        let transitions: &[Value] = m
            .and_then(|m| m["transitions"].as_array())
            .map_or(&[], Vec::as_slice);
        let any = transitions.iter().any(|t| t["from"] == "*");
        let saved = ops::fsm_node_positions(world, machine);
        let top = if any { 110.0 } else { 30.0 };
        // Unsaved states are laid out in columns by their distance from the initial state
        // (states it cannot reach in the last column); odd columns sit half a row lower so
        // edges that skip a column pass between nodes. Bounded by the node budget.
        let mut depth: std::collections::BTreeMap<&str, usize> = Default::default();
        if let Some(initial) = m.and_then(|m| m["initial"].as_str()) {
            depth.insert(initial, 0);
            let mut frontier = vec![initial];
            for d in 1..=MAX_DRAW_NODES {
                let mut next = vec![];
                for t in transitions.iter().take(MAX_DRAW_EDGES) {
                    let (Some(from), Some(to)) = (t["from"].as_str(), t["to"].as_str()) else {
                        continue;
                    };
                    if (from == "*" || frontier.contains(&from))
                        && states.contains_key(to)
                        && !depth.contains_key(to)
                    {
                        depth.insert(to, d);
                        next.push(to);
                    }
                }
                if next.is_empty() {
                    break;
                }
                frontier = next;
            }
        }
        let last = depth.values().max().map_or(0, |d| d + 1);
        let mut rows = vec![0usize; last + 1];
        let nodes: Vec<Node> = states
            .iter()
            .take(MAX_DRAW_NODES)
            .map(|(name, def)| {
                let at = match saved.get(name) {
                    Some([x, y]) => pos2(*x, *y),
                    None => {
                        let col = depth.get(name.as_str()).copied().unwrap_or(last);
                        loop {
                            let row = rows[col];
                            rows[col] += 1;
                            let p = pos2(
                                30.0 + col as f32 * (NODE.x + 90.0),
                                top + row as f32 * (NODE.y + 56.0)
                                    + if col % 2 == 1 { 50.0 } else { 0.0 },
                            );
                            let free = saved.values().all(|[x, y]| {
                                (x - p.x).abs() >= NODE.x + 8.0 || (y - p.y).abs() >= NODE.y + 8.0
                            });
                            if free || row > MAX_DRAW_NODES {
                                break p;
                            }
                        }
                    }
                };
                let problem = problems
                    .iter()
                    .find(|p| p.machine == machine && p.state.as_deref() == Some(name))
                    .map(|p| p.message.clone());
                Node {
                    name: name.clone(),
                    at,
                    detail: state_label(def),
                    problem,
                }
            })
            .collect();
        let index = |name: &str| nodes.iter().position(|n| n.name == name);
        let mut edges: Vec<EdgeView> = transitions
            .iter()
            .enumerate()
            .take(MAX_DRAW_EDGES)
            .filter_map(|(i, t)| {
                let to = index(t["to"].as_str()?)?;
                let from = match t["from"].as_str()? {
                    "*" => None,
                    name => Some(index(name)?),
                };
                Some(EdgeView {
                    index: i,
                    from,
                    to,
                    label: condition_label(&t["when"]),
                    lane: 0.0,
                })
            })
            .collect();
        // Parallel and opposite transitions between the same two nodes get lanes in the
        // frame of the lower end, so A->B and B->A land on different sides.
        let key = |e: &EdgeView| {
            let a = e.from.unwrap_or(usize::MAX);
            (a.min(e.to), a.max(e.to))
        };
        for i in 0..edges.len() {
            let k = key(&edges[i]);
            let group: Vec<usize> = (0..edges.len()).filter(|&j| key(&edges[j]) == k).collect();
            if group.len() < 2 {
                continue;
            }
            let rank = group.iter().position(|&j| j == i).unwrap() as f32;
            let lane = (rank - (group.len() - 1) as f32 / 2.0) * 26.0;
            let forward = edges[i].from.unwrap_or(usize::MAX) <= edges[i].to;
            edges[i].lane = if forward { lane } else { -lane };
        }
        Model {
            version,
            edits: 0,
            machine: machine.to_owned(),
            exists: m.is_some(),
            initial: m
                .and_then(|m| m["initial"].as_str())
                .unwrap_or("")
                .to_owned(),
            nodes,
            edges,
            any,
            transitions: transitions.len(),
            machines,
            events,
        }
    }

    fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }
}

/// Point where the segment from `center` toward `other` leaves a node rectangle.
fn boundary(center: Pos2, other: Pos2, half: Vec2) -> Pos2 {
    let d = other - center;
    if d.length_sq() < 1e-6 {
        return center;
    }
    let tx = if d.x.abs() > 1e-6 {
        half.x / d.x.abs()
    } else {
        f32::INFINITY
    };
    let ty = if d.y.abs() > 1e-6 {
        half.y / d.y.abs()
    } else {
        f32::INFINITY
    };
    center + d * tx.min(ty).min(1.0)
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// What the canvas asked for this frame.
#[derive(Default)]
struct CanvasOut {
    select: Option<Option<Sel>>,
    moved: Option<(String, Pos2)>,
    link: Option<(String, String)>,
    add_at: Option<Pos2>,
    menu: Option<Menu>,
}

enum Menu {
    Initial(String),
    Rename(String),
    Delete(Sel),
    Raise(usize),
    AddState(Pos2),
}

/// A zoom argument (`1.5` or `150%`) within `min..=max` (`graph.zoom`, `ui.zoom`).
pub(super) fn parse_zoom(number: &str, min: f32, max: f32) -> Option<f32> {
    let number = number.strip_suffix('%').map_or_else(
        || number.parse::<f32>(),
        |p| p.trim().parse::<f32>().map(|p| p / 100.0),
    );
    number
        .ok()
        .filter(|z| z.is_finite() && (min..=max).contains(z))
}

/// Size of the "Any state" pseudo node in canvas units.
const ANY_SIZE: Vec2 = vec2(110.0, 30.0);

impl GraphUi {
    /// Screen points per canvas unit.
    pub(super) fn zoom(&self) -> f32 {
        if self.zoom > 0.0 {
            self.zoom
        } else {
            1.0
        }
    }

    /// Sets the zoom (clamped) keeping the canvas point under `anchor` (relative to the
    /// canvas' top-left corner, screen points) where it is.
    fn zoom_around(&mut self, anchor: Vec2, zoom: f32) {
        let old = self.zoom();
        let new = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let canvas = (anchor - self.pan) / old;
        self.pan = anchor - canvas * new;
        self.zoom = new;
    }

    /// Canvas position -> screen, with `origin` = canvas top-left + pan.
    fn to_screen(&self, origin: Pos2, at: Pos2) -> Pos2 {
        origin + at.to_vec2() * self.zoom()
    }

    /// Screen -> canvas position.
    fn to_canvas(&self, origin: Pos2, p: Pos2) -> Pos2 {
        ((p - origin) / self.zoom()).to_pos2()
    }

    fn node_at(&self, model: &Model, origin: Pos2, i: usize) -> Rect {
        let at = match &self.drag {
            Some(d) if d.node == i => d.at,
            _ => model.nodes[i].at,
        };
        Rect::from_min_size(self.to_screen(origin, at), NODE * self.zoom())
    }

    fn any_rect(&self, origin: Pos2) -> Rect {
        Rect::from_min_size(self.to_screen(origin, ANY_AT), ANY_SIZE * self.zoom())
    }

    /// Bounds of every state (and the Any state) in canvas units.
    fn bounds(&self, model: &Model) -> Rect {
        let mut bounds = (0..model.nodes.len())
            .map(|i| {
                let at = match &self.drag {
                    Some(d) if d.node == i => d.at,
                    _ => model.nodes[i].at,
                };
                Rect::from_min_size(at, NODE)
            })
            .fold(Rect::NOTHING, |a, b| a.union(b));
        if model.any {
            bounds = bounds.union(Rect::from_min_size(ANY_AT, ANY_SIZE));
        }
        bounds
    }

    /// Graph ▸ Frame: the largest zoom up to 100 % that shows `bounds` inside `size` with a
    /// margin, centered; when even the smallest zoom does not fit, its top-left corner just
    /// inside the canvas.
    fn frame_bounds(&mut self, bounds: Rect, size: Vec2) {
        if !bounds.is_positive() {
            return;
        }
        let room = size - Vec2::splat(2.0 * FIT_MARGIN);
        let fit = (room.x / bounds.width()).min(room.y / bounds.height());
        let zoom = fit.clamp(ZOOM_MIN, 1.0);
        self.zoom = zoom;
        self.pan = if fit >= ZOOM_MIN {
            size / 2.0 - bounds.center().to_vec2() * zoom
        } else {
            Vec2::splat(FIT_MARGIN) - bounds.min.to_vec2() * zoom
        };
    }

    fn hit(&self, model: &Model, origin: Pos2, p: Pos2) -> Hit {
        if let Some(i) = (0..model.nodes.len())
            .rev()
            .find(|&i| self.node_at(model, origin, i).contains(p))
        {
            return Hit::Node(i);
        }
        if model.any && self.any_rect(origin).contains(p) {
            return Hit::Any;
        }
        let mut best = (7.0, None);
        for (k, e) in model.edges.iter().enumerate() {
            let (a, b) = self.edge_ends(model, origin, e);
            let d = segment_distance(p, a, b);
            if d < best.0 {
                best = (d, Some(k));
            }
        }
        match best.1 {
            Some(k) => Hit::Edge(k),
            None => Hit::Empty(self.to_canvas(origin, p)),
        }
    }

    /// Output port of a node (right edge, middle), or of the Any state.
    fn port(rect: Rect) -> Pos2 {
        pos2(rect.right(), rect.center().y)
    }

    fn edge_ends(&self, model: &Model, origin: Pos2, e: &EdgeView) -> (Pos2, Pos2) {
        let to = self.node_at(model, origin, e.to);
        let from = match e.from {
            Some(i) => self.node_at(model, origin, i),
            None => self.any_rect(origin),
        };
        let (fc, tc) = (from.center(), to.center());
        let dir = (tc - fc).normalized();
        let offset = vec2(-dir.y, dir.x) * e.lane * self.zoom();
        let a = boundary(fc + offset, tc + offset, from.size() / 2.0);
        let b = boundary(tc + offset, fc + offset, to.size() / 2.0);
        (a, b)
    }

    /// Draws the machine graph; with `interactive` it also handles selection, node drags,
    /// linking, panning and the context menu.
    fn canvas(&mut self, ui: &mut Ui, model: &Model, size: Vec2, interactive: bool) -> CanvasOut {
        let mut out = CanvasOut::default();
        let sense = if interactive {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(size, sense);
        if interactive {
            self.canvas_size = rect.size();
        }
        if interactive && std::mem::take(&mut self.fit) {
            let bounds = self.bounds(model);
            self.frame_bounds(bounds, rect.size());
        }
        // Wheel (or pinch / Ctrl+wheel) over the canvas zooms around the pointer, before
        // anything is placed this frame; the scroll is used up so no parent scrolls too. Not
        // while a node drag, link or pan is held (their offsets are in this zoom's points).
        let held = self.drag.is_some() || self.link.is_some() || self.panning;
        if interactive && !held {
            if let Some(p) = response.hover_pos() {
                // egui reports Ctrl+wheel both as zoom and as scroll: one of them only.
                let (wheel, pinch) = ui.input(|i| (i.raw_scroll_delta.y, i.zoom_delta()));
                let factor = if (pinch - 1.0).abs() > 1e-4 {
                    pinch
                } else {
                    (wheel * 0.003).exp()
                };
                if (factor - 1.0).abs() > 1e-4 {
                    let zoom = self.zoom() * factor;
                    self.zoom_around(p - rect.min, zoom);
                    ui.input_mut(|i| {
                        i.raw_scroll_delta = Vec2::ZERO;
                        i.smooth_scroll_delta = Vec2::ZERO;
                    });
                }
            }
        }
        let zoom = self.zoom();
        let origin = rect.min + self.pan;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, CANVAS);
        // Background grid (bounded by the canvas size; none when it would be denser than
        // one line per 10 points).
        let step = 40.0 * zoom;
        let grid = Stroke::new(1.0_f32, Color32::from_rgb(26, 31, 40));
        if step >= 10.0 {
            let mut x = rect.left() + self.pan.x.rem_euclid(step);
            while x < rect.right() {
                painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], grid);
                x += step;
            }
            let mut y = rect.top() + self.pan.y.rem_euclid(step);
            while y < rect.bottom() {
                painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], grid);
                y += step;
            }
        }
        if interactive {
            self.interact(ui, model, origin, &response, &mut out);
        }
        // A release this canvas did not see (another tab, a non-interactive frame) must not
        // leave a drag, link or pan behind for the next one to finish.
        if !ui.input(|i| i.pointer.any_down()) {
            self.drag = None;
            self.link = None;
            self.panning = false;
        }
        // Transitions.
        let selected_edge = match &self.selected {
            Some(Sel::Transition(i)) => Some(*i),
            _ => None,
        };
        for e in &model.edges {
            let (a, b) = self.edge_ends(model, origin, e);
            if !rect.intersects(Rect::from_two_pos(a, b).expand(4.0)) {
                continue;
            }
            let selected = selected_edge == Some(e.index);
            let color = if selected { ACCENT } else { EDGE };
            let stroke = Stroke::new(if selected { 2.5_f32 } else { 1.5 }, color);
            painter.line_segment([a, b], stroke);
            let dir = (b - a).normalized();
            let side = vec2(-dir.y, dir.x);
            for s in [-1.0, 1.0] {
                painter.line_segment([b, b - dir * 10.0 * zoom + side * 5.0 * zoom * s], stroke);
            }
            let mid = a + (b - a) * 0.5;
            let galley = painter.layout_no_wrap(
                e.label.clone(),
                egui::FontId::proportional(11.0 * zoom),
                if selected { ACCENT } else { MUTED },
            );
            let label = Rect::from_center_size(mid, galley.size() + vec2(8.0, 2.0) * zoom);
            painter.rect_filled(label, 3.0, Color32::from_black_alpha(190));
            painter.galley(label.min + vec2(4.0, 1.0) * zoom, galley, MUTED);
            #[cfg(test)]
            GRAPH_RECTS.with(|r| {
                r.borrow_mut()
                    .push((format!("graph:edge:{}", e.index), label))
            });
        }
        // A transition being dragged out.
        if let (Some(from), Some(p)) = (self.link, ui.input(|i| i.pointer.latest_pos())) {
            let start = match from {
                Some(i) => Self::port(self.node_at(model, origin, i)),
                None => Self::port(self.any_rect(origin)),
            };
            painter.line_segment([start, p], Stroke::new(2.0_f32, ACCENT));
        }
        // Any state.
        if model.any {
            let r = self.any_rect(origin);
            painter.rect(
                r,
                6.0,
                Color32::from_rgb(44, 40, 66),
                Stroke::new(1.0_f32, MUTED),
                egui::StrokeKind::Inside,
            );
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "Any state",
                egui::FontId::proportional(12.0 * zoom),
                Color32::WHITE,
            );
            painter.circle_filled(Self::port(r), PORT * 0.7, MUTED);
        }
        // States.
        let selected_state = match &self.selected {
            Some(Sel::State(s)) => Some(s.as_str()),
            _ => None,
        };
        for (i, node) in model.nodes.iter().enumerate() {
            let r = self.node_at(model, origin, i);
            if !rect.intersects(r.expand(PORT)) {
                continue;
            }
            let live = self.live.get(i).copied().unwrap_or(0);
            let fill = if live > 0 {
                Color32::from_rgb(34, 82, 58)
            } else {
                NODE_FILL
            };
            let stroke = if selected_state == Some(node.name.as_str()) {
                Stroke::new(2.5_f32, ACCENT)
            } else if node.problem.is_some() {
                Stroke::new(2.0_f32, WARNING)
            } else if live > 0 {
                Stroke::new(2.0_f32, LIVE)
            } else {
                Stroke::new(1.0_f32, Color32::from_rgb(70, 84, 104))
            };
            painter.rect(r, 6.0 * zoom, fill, stroke, egui::StrokeKind::Inside);
            if node.name == model.initial {
                // Initial state: green bar on the left edge.
                painter.rect_filled(
                    Rect::from_min_size(r.min, vec2(5.0 * zoom, r.height())),
                    3.0 * zoom,
                    LIVE,
                );
            }
            // Names and subtitles longer than the node are cut at its edge.
            let text = painter.with_clip_rect(r.shrink(3.0 * zoom).intersect(rect));
            text.text(
                r.min + vec2(12.0, 7.0) * zoom,
                egui::Align2::LEFT_TOP,
                &node.name,
                egui::FontId::proportional(14.0 * zoom),
                Color32::WHITE,
            );
            text.text(
                r.min + vec2(12.0, 27.0) * zoom,
                egui::Align2::LEFT_TOP,
                &node.detail,
                egui::FontId::proportional(10.5 * zoom),
                MUTED,
            );
            if live > 0 {
                painter.text(
                    pos2(r.right() - 8.0 * zoom, r.top() + 7.0 * zoom),
                    egui::Align2::RIGHT_TOP,
                    format!("live {live}"),
                    egui::FontId::proportional(11.0 * zoom),
                    LIVE,
                );
            } else if node.problem.is_some() {
                painter.text(
                    pos2(r.right() - 9.0 * zoom, r.top() + 6.0 * zoom),
                    egui::Align2::RIGHT_TOP,
                    "!",
                    egui::FontId::proportional(15.0 * zoom),
                    WARNING,
                );
            }
            if interactive {
                painter.circle_filled(Self::port(r), PORT * 0.7, ACCENT);
            }
            #[cfg(test)]
            GRAPH_RECTS.with(|g| {
                let mut g = g.borrow_mut();
                g.push((format!("graph:node:{}", node.name), r));
                g.push((
                    format!("graph:port:{}", node.name),
                    Rect::from_center_size(Self::port(r), Vec2::splat(PORT * 2.0)),
                ));
            });
        }
        // Problem / live tooltips.
        if let Some(p) = response.hover_pos() {
            if let Hit::Node(i) = self.hit(model, origin, p) {
                if let Some(problem) = &model.nodes[i].problem {
                    response.clone().on_hover_text_at_pointer(problem);
                }
            }
        }
        #[cfg(test)]
        GRAPH_RECTS.with(|g| g.borrow_mut().push(("graph:canvas".into(), rect)));
        if !model.exists {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Pick or create a state machine above",
                egui::FontId::proportional(14.0),
                MUTED,
            );
        }
        out
    }

    fn interact(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        origin: Pos2,
        response: &egui::Response,
        out: &mut CanvasOut,
    ) {
        let latest = ui.input(|i| i.pointer.latest_pos());
        if response.drag_started_by(PointerButton::Primary) {
            if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                // Smaller nodes when zoomed out: the port's grab radius shrinks with them.
                let reach = (PORT + 2.0) * self.zoom().clamp(0.5, 1.0);
                let port = (0..model.nodes.len())
                    .find(|&i| Self::port(self.node_at(model, origin, i)).distance(p) <= reach)
                    .map(Some)
                    .or_else(|| {
                        (model.any && Self::port(self.any_rect(origin)).distance(p) <= reach)
                            .then_some(None)
                    });
                match (port, self.hit(model, origin, p)) {
                    (Some(from), _) => self.link = Some(from),
                    (None, Hit::Node(i)) => {
                        let r = self.node_at(model, origin, i);
                        self.drag = Some(NodeDrag {
                            node: i,
                            grab: p - r.min,
                            at: self.to_canvas(origin, r.min),
                        });
                        out.select = Some(Some(Sel::State(model.nodes[i].name.clone())));
                    }
                    _ => self.panning = true,
                }
            }
        }
        if response.dragged_by(PointerButton::Primary) {
            let zoom = self.zoom();
            if let (Some(d), Some(p)) = (self.drag.as_mut(), latest) {
                // The grab offset is in screen points; positions are canvas units.
                d.at = ((p - d.grab - origin) / zoom).to_pos2();
            } else if self.panning {
                self.pan += response.drag_delta();
            }
        }
        if response.dragged_by(PointerButton::Middle) {
            self.pan += response.drag_delta();
        }
        if response.drag_stopped() {
            if let Some(d) = self.drag.take() {
                let node = &model.nodes[d.node];
                if (d.at - node.at).length() > 0.5 {
                    out.moved = Some((node.name.clone(), d.at));
                }
            }
            if let (Some(from), Some(p)) = (self.link.take(), latest) {
                if let Hit::Node(to) = self.hit(model, origin, p) {
                    let from = from.map_or("*", |i| model.nodes[i].name.as_str());
                    if from != model.nodes[to].name {
                        out.link = Some((from.to_owned(), model.nodes[to].name.clone()));
                    }
                }
            }
            self.panning = false;
        }
        if response.clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                out.select = Some(match self.hit(model, origin, p) {
                    Hit::Node(i) => Some(Sel::State(model.nodes[i].name.clone())),
                    Hit::Edge(k) => Some(Sel::Transition(model.edges[k].index)),
                    Hit::Any | Hit::Empty(_) => None,
                });
            }
        }
        if response.double_clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                if let Hit::Empty(at) = self.hit(model, origin, p) {
                    out.add_at = Some(at);
                }
            }
        }
        if response.secondary_clicked() {
            self.context = response
                .interact_pointer_pos()
                .map(|p| self.hit(model, origin, p));
        }
        let context = self.context.clone();
        response.context_menu(|ui| {
            let mut item = |ui: &mut Ui, title: &str, menu: Menu| {
                let r = ui.button(title);
                #[cfg(test)]
                GRAPH_RECTS.with(|g| g.borrow_mut().push((format!("graph:menu:{title}"), r.rect)));
                if r.clicked() {
                    out.menu = Some(menu);
                    ui.close_menu();
                }
            };
            match &context {
                Some(Hit::Node(i)) => {
                    let name = model.nodes[*i].name.clone();
                    item(ui, "Set as initial state", Menu::Initial(name.clone()));
                    item(ui, "Rename…", Menu::Rename(name.clone()));
                    item(ui, "Delete state", Menu::Delete(Sel::State(name)));
                }
                Some(Hit::Edge(k)) => {
                    let index = model.edges[*k].index;
                    item(ui, "Raise priority", Menu::Raise(index));
                    item(
                        ui,
                        "Delete transition",
                        Menu::Delete(Sel::Transition(index)),
                    );
                }
                Some(Hit::Empty(at)) => item(ui, "Add state here", Menu::AddState(*at)),
                _ => {
                    ui.label(RichText::new("Drag from a port to add a transition").small());
                }
            }
        });
    }
}

#[cfg(test)]
thread_local! {
    /// Screen rects of graph widgets drawn this frame (tests copy them into `Editor::rects`).
    pub(super) static GRAPH_RECTS: std::cell::RefCell<Vec<(String, Rect)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The value of `kind`'s fields other than `kind` and the `nested` ones: drawn with the typed
/// form, then put back.
fn kind_fields(
    ui: &mut Ui,
    ctx: &mut form::Ctx,
    spec: &form::Spec,
    value: &mut Value,
    nested: &[&str],
    id: &str,
) -> bool {
    let Some(map) = value.as_object_mut() else {
        return false;
    };
    let keys: Vec<String> = map
        .keys()
        .filter(|k| *k != "kind" && !nested.contains(&k.as_str()))
        .cloned()
        .collect();
    if keys.is_empty() {
        return false;
    }
    let mut rest = Map::new();
    for k in &keys {
        if let Some(v) = map.remove(k) {
            rest.insert(k.clone(), v);
        }
    }
    let mut rest = Value::Object(rest);
    let changed = form::show(ui, ctx, spec, &mut rest, id).changed;
    if let Value::Object(rest) = rest {
        map.extend(rest);
    }
    changed
}

/// A `kind`-tagged value: kind dropdown (a new kind starts from its template), then the
/// typed form of its fields; operands and nested conditions recurse. Returns whether it
/// changed.
#[allow(clippy::too_many_arguments)]
fn variant(
    ui: &mut Ui,
    ctx: &mut form::Ctx,
    all: &Kinds,
    list: &[Kind],
    value: &mut Value,
    id: &str,
    defaults: (&str, &str),
    depth: usize,
) -> bool {
    let current = value["kind"].as_str().unwrap_or("").to_owned();
    let mut changed = false;
    let mut picked = None;
    let combo = egui::ComboBox::from_id_salt(("variant", id))
        .selected_text(if current.is_empty() {
            "?"
        } else {
            current.as_str()
        })
        .width(130.0)
        .show_ui(ui, |ui| {
            for k in list {
                let r = ui.selectable_label(k.name == current, k.name);
                #[cfg(test)]
                GRAPH_RECTS.with(|g| {
                    g.borrow_mut()
                        .push((format!("graph:kind:{id}:{}", k.name), r.rect))
                });
                if r.clicked() && k.name != current {
                    picked = Some(k);
                }
            }
        });
    if let Some(rects) = ctx.rects.as_deref_mut() {
        rects.insert(format!("graph:kind:{id}"), combo.response.rect);
    }
    if let Some(k) = picked {
        let mut v = k.template.clone();
        // Point new values at what the program declares (the form then offers the rest).
        if v.get("variable").is_some_and(|x| x == "") && !defaults.0.is_empty() {
            v["variable"] = json!(defaults.0);
        }
        if v.get("event").is_some_and(|x| x == "") && !defaults.1.is_empty() {
            v["event"] = json!(defaults.1);
        }
        *value = v;
        return true;
    }
    let Some(kind) = list.iter().find(|k| k.name == current) else {
        ui.label(
            RichText::new("unknown kind: pick one")
                .small()
                .color(WARNING),
        );
        return false;
    };
    let nested: &[&str] = match current.as_str() {
        "eq" | "ge" => &["left", "right"],
        "set" | "add" => &["value"],
        "not" => &["condition"],
        _ => &[],
    };
    if depth > 6 {
        return false;
    }
    for field in nested {
        let Some(child) = value.get_mut(*field) else {
            continue;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(*field).small().color(MUTED));
            let child_list = if *field == "condition" {
                &all.rule_conditions
            } else {
                &all.operands
            };
            ui.vertical(|ui| {
                changed |= variant(
                    ui,
                    ctx,
                    all,
                    child_list,
                    child,
                    &format!("{id}/{field}"),
                    defaults,
                    depth + 1,
                );
            });
        });
    }
    changed |= kind_fields(ui, ctx, &kind.spec, value, nested, id);
    changed
}

/// Live commit point: nothing held, nothing being typed, no write in flight.
fn settled(ui: &Ui, saving: usize) -> bool {
    let held = ui.input(|i| i.pointer.any_down());
    let typing = ui
        .ctx()
        .memory(|m| m.focused())
        .is_some_and(|f| egui::TextEdit::load_state(ui.ctx(), f).is_some());
    !held && !typing && saving == 0 && !ui.ctx().memory(|m| m.any_popup_open())
}

/// `state`, `state_2`, ... not used by the machine yet.
fn free_state(model: &Model) -> String {
    if model.node("state").is_none() {
        return "state".into();
    }
    (2..)
        .map(|n| format!("state_{n}"))
        .find(|n| model.node(n).is_none())
        .unwrap()
}

impl Editor {
    /// Refreshes the cached checks and model when the world or the open machine changed.
    fn graph_refresh(&mut self) {
        let version = self.world_version;
        let key = (version, self.graph.edits);
        if self.graph.problems.as_ref().is_none_or(|(k, _)| *k != key) {
            let problems = ops::fsm_problems(self.graph_world());
            self.graph.problems = Some((key, problems));
        }
        if self.graph.machine.is_none() {
            self.graph.machine = self
                .world
                .resources
                .get(ops::FSM_RESOURCE)
                .and_then(|r| r["machines"].as_object())
                .and_then(|m| m.keys().next().cloned());
        }
        let machine = self.graph.machine.clone().unwrap_or_default();
        let stale = self
            .graph
            .model
            .as_ref()
            .is_none_or(|m| (m.version, m.edits) != key || m.machine != machine);
        if stale {
            // A drag / link in progress follows its node by name into the new model.
            let old = self.graph.model.as_ref().filter(|m| m.machine == machine);
            let name = |i: usize| old.and_then(|m| m.nodes.get(i)).map(|n| n.name.clone());
            let drag = self
                .graph
                .drag
                .take()
                .and_then(|d| Some((name(d.node)?, d)));
            // `Some(None)` = from the Any state.
            let link: Option<Option<String>> = match self.graph.link.take() {
                Some(Some(i)) => name(i).map(Some),
                Some(None) => Some(None),
                None => None,
            };
            let problems = &self.graph.problems.as_ref().expect("set above").1;
            let mut model = Model::build(self.graph_world(), version, &machine, problems);
            if let Some((n, mut d)) = drag {
                if let Some(i) = model.node(&n) {
                    d.node = i;
                    self.graph.drag = Some(d);
                }
            }
            self.graph.link = match link {
                Some(Some(n)) => model.node(&n).map(Some),
                Some(None) if model.any => Some(None),
                _ => None,
            };
            model.edits = key.1;
            self.graph.model = Some(model);
            self.graph.builds += 1;
            if self.graph.kinds.is_none() {
                self.graph.kinds = Some(Kinds::new());
            }
            // A selection the new revision no longer has is dropped.
            let model = self.graph.model.as_ref().unwrap();
            let keep = match &self.graph.selected {
                Some(Sel::State(s)) => model.node(s).is_some(),
                Some(Sel::Transition(i)) => *i < model.transitions,
                None => true,
            };
            if !keep {
                self.graph.selected = None;
            }
            // A context menu points at a node / edge of the old model.
            self.graph.context = None;
        }
    }

    /// The world graph edits build on (see `GraphUi::scratch`).
    fn graph_world_arc(&self) -> std::sync::Arc<World> {
        match &self.graph.scratch {
            Some((version, world)) if *version == self.world_version => world.clone(),
            _ => self.world.clone(),
        }
    }

    fn graph_world(&self) -> &World {
        match &self.graph.scratch {
            Some((version, world)) if *version == self.world_version => world,
            _ => &self.world,
        }
    }

    /// Queues the edits of a graph / rule op and records them in the scratch world.
    fn graph_write(&mut self, result: anyhow::Result<Vec<axiom_core::Edit>>) -> Option<Action> {
        let edits = match result {
            Ok(edits) if edits.is_empty() => return None,
            Ok(edits) => edits,
            Err(error) => {
                self.problem(&error);
                return None;
            }
        };
        let mut next = self.graph_world().clone();
        for e in &edits {
            if let Err(error) = e.clone().apply(&mut next, &self.registry) {
                self.problem(&error);
                return None;
            }
        }
        self.graph.scratch = Some((self.world_version, std::sync::Arc::new(next)));
        self.graph.edits += 1;
        // Writes still queued (a draft flushed on a selection change) go first, in one
        // batch: these are whole-resource writes, so their order decides the result. All
        // writes of a pass share `expected_rev`, so `coalesce` leaves exactly one action.
        let write = self.write(edits, None);
        self.graph.queued.push(write);
        let mut merged = super::coalesce(std::mem::take(&mut self.graph.queued)).into_iter();
        let first = merged.next();
        self.graph.queued.extend(merged);
        first
    }

    /// The write a dirty element draft stands for, if the element is still the one it was
    /// loaded from (else the edit is dropped with a notice).
    fn element_edit(&mut self, draft: &Draft) -> Option<FsmEdit> {
        if self.graph_element(&draft.sel).as_ref() != Some(&draft.base) {
            self.set_error(
                "The graph element changed in another revision; your unsaved edit was dropped",
            );
            return None;
        }
        Some(match &draft.sel {
            Sel::State(s) => FsmEdit::SetState {
                name: s.clone(),
                state: draft.value.clone(),
            },
            Sel::Transition(i) => FsmEdit::SetTransition(*i, draft.value.clone()),
        })
    }

    /// Commits a dirty element / rule draft before the selection moves away from it.
    fn graph_flush(&mut self) {
        if let Some(draft) = self.graph.element.take().filter(|d| d.dirty) {
            if let Some(edit) = self.element_edit(&draft) {
                let action = self.graph_edit(edit);
                self.graph.queued.extend(action);
            }
        }
        if let Some(draft) = self.graph.rule.take().filter(|d| d.dirty) {
            let action = self.rule_write(&draft);
            self.graph.queued.extend(action);
        }
    }

    /// The selected element's committed value (with queued graph writes).
    fn graph_element(&self, sel: &Sel) -> Option<Value> {
        let m = self
            .graph_world()
            .resources
            .get(ops::FSM_RESOURCE)?
            .get("machines")?
            .get(self.graph.machine.as_deref()?)?;
        match sel {
            Sel::State(s) => m["states"].get(s).cloned(),
            Sel::Transition(i) => m["transitions"].get(*i).cloned(),
        }
    }

    /// Opens the Graphs tab on `machine` (Inspector ▸ Edit Graph, `graph.open MACHINE`,
    /// `editor_view {open:"graph:MACHINE"}`).
    pub(super) fn graph_open(&mut self, machine: &str) -> anyhow::Result<()> {
        let exists = self
            .graph_world()
            .resources
            .get(ops::FSM_RESOURCE)
            .and_then(|r| r["machines"].get(machine))
            .is_some();
        if !exists {
            return Err(axiom_core::Diagnostic::error(
                "not_found",
                format!("/resources/fsm.machines/machines/{machine}"),
                format!("no state machine '{machine}'"),
            ));
        }
        if self.graph.machine.as_deref() != Some(machine) {
            self.graph_flush();
            self.graph.selected = None;
            self.graph.element = None;
            self.graph.pan = Vec2::ZERO;
            self.graph.zoom = 1.0;
        }
        self.graph.machine = Some(machine.to_owned());
        self.graph.view = View::Machine;
        self.show_dock(Dock::Graphs);
        Ok(())
    }

    /// The rule table, with rule `id` selected unless empty (`editor_view {open:"rules:ID"}`).
    pub(super) fn graph_open_rules(&mut self, id: &str) -> anyhow::Result<()> {
        if !id.is_empty() {
            let index = self
                .graph_world()
                .resources
                .get("behavior.program")
                .and_then(|p| p["rules"].as_array())
                .and_then(|r| r.iter().position(|x| x["id"] == id))
                .ok_or_else(|| {
                    axiom_core::Diagnostic::error(
                        "not_found",
                        "/resources/behavior.program/rules",
                        format!("no rule '{id}'"),
                    )
                })?;
            if self.graph.rule_index != Some(index) {
                self.graph_flush();
                self.graph.rule_index = Some(index);
            }
        }
        self.graph.view = View::Rules;
        self.show_dock(Dock::Graphs);
        Ok(())
    }

    fn graph_select(&mut self, sel: Option<Sel>) {
        if self.graph.selected != sel {
            // An unsaved edit of the previous element is written, not dropped.
            self.graph_flush();
            self.graph.element = None;
            if let Some(Sel::State(s)) = &sel {
                self.graph.name.clone_from(s);
            }
        }
        self.graph.selected = sel;
    }

    /// Runs one `ops::fsm_edit` on the open machine.
    fn graph_edit(&mut self, edit: FsmEdit) -> Option<Action> {
        let machine = self.graph.machine.clone()?;
        let result = ops::fsm_edit(self.graph_world(), &machine, &edit);
        match result {
            Ok(edits) if edits.is_empty() => None,
            result => self.graph_write(result),
        }
    }

    /// `graph.add_state` / double-click on the canvas: a new `stop` state (at `at`: plus its
    /// position, like `fsm M state add S; fsm M node S x,y` in one script).
    fn graph_add_state(&mut self, at: Option<Pos2>) -> Option<Action> {
        let machine = self.graph.machine.clone()?;
        let name = free_state(self.graph.model.as_ref()?);
        let add = FsmEdit::AddState {
            name: name.clone(),
            state: None,
        };
        let world = self.graph_world();
        let result = ops::fsm_edit(world, &machine, &add).and_then(|mut edits| {
            if let Some(at) = at {
                let mut next = world.clone();
                for e in &edits {
                    e.clone().apply(&mut next, &self.registry)?;
                }
                edits.extend(ops::fsm_set_node(&next, &machine, &name, [at.x, at.y])?);
            }
            Ok(edits)
        });
        let action = self.graph_write(result);
        if action.is_some() {
            self.graph_select(Some(Sel::State(name)));
        }
        action
    }

    fn graph_delete(&mut self, sel: Sel) -> Option<Action> {
        let edit = match &sel {
            Sel::State(s) => FsmEdit::RemoveState(s.clone()),
            Sel::Transition(i) => FsmEdit::RemoveTransition(*i),
        };
        let action = self.graph_edit(edit);
        if action.is_some() {
            // The deleted element's draft goes with it.
            self.graph.element = None;
            self.graph_select(None);
        }
        action
    }

    fn graph_rename(&mut self, from: &str, to: &str) -> Option<Action> {
        // A pending edit of the state is written under its old name first.
        self.graph_flush();
        let action = self.graph_edit(FsmEdit::RenameState {
            from: from.to_owned(),
            to: to.trim().to_owned(),
        });
        if action.is_some() {
            self.graph.selected = Some(Sel::State(to.trim().to_owned()));
            self.graph.element = None;
        }
        action
    }

    /// Inspector ▸ Behavior ▸ FSM Agent (`ACL: fsm SEL agent MACHINE [radius=M]`).
    pub(super) fn add_fsm_agent(&mut self, machine: &str, radius: Option<f32>) -> Option<Action> {
        let ids = self.selection();
        let result = ops::fsm_add_agent(self.graph_world(), &self.registry, &ids, machine, radius);
        let action = self.graph_write(result);
        if action.is_some() {
            self.open_component_after = Some("fsm.agent".into());
            self.graph.machine = Some(machine.to_owned());
        }
        action
    }

    /// Inspector ▸ Behavior ▸ Patrol / Guard / NPC Dialogue (`ACL: behavior SEL TEMPLATE ..`).
    pub(super) fn add_behavior(
        &mut self,
        template: ops::BehaviorTemplate,
        options: &ops::BehaviorOptions,
    ) -> Option<Action> {
        let ids = self.selection();
        let machine = options
            .machine
            .clone()
            .unwrap_or_else(|| ops::free_machine_id(self.graph_world(), template.name()));
        let result = ops::add_behavior(self.graph_world(), &self.registry, &ids, template, options);
        let action = self.graph_write(result);
        if action.is_some() {
            self.open_component_after = Some("fsm.agent".into());
            self.graph.machine = Some(machine);
            self.graph.view = View::Machine;
        }
        action
    }

    /// Why a `graph.*` / `behavior.*` command cannot run (`None` = not one of them).
    pub(super) fn graph_blocked(&self, id: &str) -> Option<Option<&'static str>> {
        let busy = self.dirty() || self.drag.is_some();
        let open = self.graph.machine.as_deref().is_some_and(|m| {
            self.world
                .resources
                .get(ops::FSM_RESOURCE)
                .and_then(|r| r["machines"].get(m))
                .is_some()
        });
        Some(match id {
            "graph.open" => None,
            // View only: allowed while a draft is open.
            "graph.frame" | "graph.zoom" if !open => Some("open a state machine in the Graphs tab"),
            "graph.frame" | "graph.zoom" => None,
            "graph.add_state" | "graph.set_initial" | "graph.delete" if !open => {
                Some("open a state machine in the Graphs tab")
            }
            "graph.set_initial" if !matches!(self.graph.selected, Some(Sel::State(_))) => {
                Some("select a state in the graph")
            }
            "graph.delete" if self.graph.selected.is_none() => {
                Some("select a state or transition in the graph")
            }
            "behavior.fsm_agent" | "behavior.patrol" | "behavior.guard" | "behavior.dialogue"
                if self.selected.is_none() =>
            {
                Some("nothing selected")
            }
            id if id.starts_with("graph.") || id.starts_with("behavior.") => {
                busy.then_some("apply or reload the inspector draft first")
            }
            _ => return None,
        })
    }

    /// Runs a `graph.*` / `behavior.*` command (blocked ones were refused by the caller).
    pub(super) fn graph_command(
        &mut self,
        id: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Vec<Action>> {
        let takes_arg = matches!(id, "graph.open" | "graph.zoom" | "behavior.fsm_agent");
        anyhow::ensure!(takes_arg || arg.is_none(), "{id} takes no argument");
        let arg = arg.map(str::trim).filter(|a| !a.is_empty());
        let action = match (id, arg) {
            ("graph.open", Some(machine)) => {
                self.graph_open(machine)?;
                None
            }
            ("graph.open", None) => {
                self.show_dock(Dock::Graphs);
                self.notice("Pick a state machine in the Graphs tab");
                None
            }
            ("graph.frame", _) => {
                self.graph.fit = true;
                None
            }
            ("graph.zoom", None) => {
                self.show_dock(Dock::Graphs);
                self.notice(
                    "Wheel over the graph zooms around the pointer; Frame fits every state",
                );
                None
            }
            ("graph.zoom", Some(arg)) => {
                let current = self.graph.zoom();
                let zoom = match arg {
                    "in" => current * 1.25,
                    "out" => current / 1.25,
                    "reset" => 1.0,
                    number => match parse_zoom(number, ZOOM_MIN, ZOOM_MAX) {
                        Some(z) => z,
                        None => anyhow::bail!(
                            "graph.zoom takes in|out|reset or a zoom {ZOOM_MIN}..{ZOOM_MAX} (e.g. 1.5 or 150%)"
                        ),
                    },
                };
                // Around the canvas center (the last drawn size).
                let center = self.graph.canvas_size / 2.0;
                self.graph.zoom_around(center, zoom);
                None
            }
            ("graph.add_state", _) => {
                self.graph_refresh();
                self.graph_add_state(None)
            }
            ("graph.set_initial", _) => match self.graph.selected.clone() {
                Some(Sel::State(s)) => self.graph_edit(FsmEdit::SetInitial(s)),
                _ => None,
            },
            ("graph.delete", _) => match self.graph.selected.clone() {
                Some(sel) => self.graph_delete(sel),
                None => None,
            },
            ("behavior.fsm_agent", Some(machine)) => self.add_fsm_agent(machine, None),
            ("behavior.fsm_agent", None) => {
                self.options =
                    super::options::OptionsDialog::for_command(self, "behavior.fsm_agent");
                None
            }
            (other, _) => {
                let template = other
                    .strip_prefix("behavior.")
                    .and_then(ops::BehaviorTemplate::parse)
                    .ok_or_else(|| anyhow::anyhow!("unknown editor command {other}"))?;
                self.add_behavior(template, &Default::default())
            }
        };
        let mut out = std::mem::take(&mut self.graph.queued);
        out.extend(action);
        Ok(out)
    }

    /// `observe` / `editor_view` view of the Graphs tab.
    pub(super) fn graph_observation(&self) -> Value {
        json!({
            "machine": self.graph.machine,
            "view": match self.graph.view { View::Machine => "machine", View::Rules => "rules", View::Overview => "overview" },
            "zoom": (self.graph.zoom() * 1000.0).round() / 1000.0,
            "selected": match &self.graph.selected {
                Some(Sel::State(s)) => json!({"state": s}),
                Some(Sel::Transition(i)) => json!({"transition": i}),
                None => Value::Null,
            },
            // Fresh even when the Graphs / Problems tabs are not drawn (agents rely on it).
            "problems": match &self.graph.problems {
                Some((k, p)) if *k == (self.world_version, self.graph.edits) => p.len(),
                _ => ops::fsm_problems(self.graph_world()).len(),
            },
        })
    }

    /// Graphs tab body.
    pub(super) fn graph_tab(
        &mut self,
        ui: &mut Ui,
        runtime: &WorldRuntime,
        actions: &mut Vec<Action>,
    ) {
        // Graph writes are sent after the frame that made them; with nothing saving, a
        // scratch world of the current version holds writes that were refused: drop it.
        if self.saving == 0
            && self
                .graph
                .scratch
                .as_ref()
                .is_some_and(|(v, _)| *v == self.world_version)
        {
            self.graph.scratch = None;
            self.graph.edits += 1;
        }
        self.graph_refresh();
        #[cfg(test)]
        GRAPH_RECTS.with(|g| g.borrow_mut().clear());
        let mut pick = None;
        let mut create = None;
        let mut command = None;
        let problems = self.graph.problems.as_ref().map_or(0, |(_, p)| p.len());
        let buttons = [
            ("graph.add_state", "+ State"),
            ("graph.set_initial", "Set initial"),
            ("graph.delete", "Delete"),
            ("graph.frame", "Frame"),
        ]
        .map(|(cmd, title)| (cmd, title, self.command_blocked(cmd)));
        let frame_blocked = self.command_blocked("graph.zoom");
        let mut zoom_reset = false;
        let mut view = self.graph.view;
        ui.horizontal_wrapped(|ui| {
            for (v, title) in [
                (View::Machine, "State machine"),
                (View::Rules, "Rules"),
                (View::Overview, "Overview"),
            ] {
                let r = ui.selectable_label(view == v, title);
                #[cfg(test)]
                GRAPH_RECTS.with(|g| g.borrow_mut().push((format!("graph:view:{title}"), r.rect)));
                if r.clicked() {
                    view = v;
                }
            }
            ui.separator();
            if view == View::Machine {
                let model = self.graph.model.as_ref().expect("refreshed");
                let current = self.graph.machine.clone().unwrap_or_default();
                egui::ComboBox::from_id_salt("graph_machine")
                    .selected_text(if current.is_empty() {
                        "(no machine)"
                    } else {
                        current.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for m in &model.machines {
                            if ui.selectable_label(*m == current, m).clicked() {
                                pick = Some(m.clone());
                            }
                        }
                    });
                ui.menu_button("New machine…", |ui| {
                    let id = egui::Id::new("graph_new_machine");
                    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .hint_text("machine id")
                            .desired_width(140.0),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button("Create").clicked() || enter) && !text.trim().is_empty() {
                        create = Some(text.trim().to_owned());
                        text.clear();
                        ui.close_menu();
                    }
                    ui.data_mut(|d| d.insert_temp(id, text));
                    ui.label(RichText::new("ACL: fsm ID create").small().weak());
                });
                for (cmd, title, blocked) in buttons {
                    let r = ui
                        .add_enabled(blocked.is_none(), egui::Button::new(title))
                        .on_disabled_hover_text(blocked.unwrap_or_default());
                    #[cfg(test)]
                    GRAPH_RECTS.with(|g| g.borrow_mut().push((format!("graph:{cmd}"), r.rect)));
                    if r.clicked() {
                        command = Some(cmd);
                    }
                }
                let zoom = self.graph.zoom();
                let r = ui
                    .add_enabled(
                        frame_blocked.is_none(),
                        egui::Button::new(format!("{:.0}%", zoom * 100.0)),
                    )
                    .on_hover_text(
                        "Zoom: wheel over the graph (around the pointer); click for 100 %",
                    );
                #[cfg(test)]
                GRAPH_RECTS.with(|g| g.borrow_mut().push(("graph:zoom".into(), r.rect)));
                if r.clicked() {
                    zoom_reset = true;
                }
            }
            let text = if problems == 0 {
                RichText::new("no graph problems").small().color(MUTED)
            } else {
                RichText::new(format!("{problems} problem(s) · see Problems"))
                    .small()
                    .color(WARNING)
            };
            ui.label(text);
        });
        self.graph.view = view;
        if let Some(m) = pick {
            if let Err(e) = self.graph_open(&m) {
                self.problem(&e);
            }
        }
        if let Some(id) = create {
            let result = ops::fsm_create_machine(self.graph_world(), &self.registry, &id, None);
            if let Some(action) = self.graph_write(result) {
                actions.push(action);
                self.graph.machine = Some(id);
                self.graph_select(None);
            }
        }
        if let Some(cmd) = command {
            self.run_into(cmd, actions);
        }
        if zoom_reset {
            if let Err(e) = self.run_command_with("graph.zoom", Some("reset")) {
                self.set_error(format!("{e:#}"));
            }
        }
        match self.graph.view {
            View::Overview => {
                egui::ScrollArea::vertical()
                    .id_salt("graph_overview")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.graphs(ui));
            }
            View::Rules => self.rules_view(ui, actions),
            View::Machine => self.machine_view(ui, runtime, actions),
        }
        #[cfg(test)]
        GRAPH_RECTS.with(|g| {
            for (k, r) in g.borrow_mut().drain(..) {
                self.rects.insert(k, r);
            }
        });
        actions.append(&mut self.graph.queued);
    }

    /// Agents per state of the open machine during Play (allocation-free once sized).
    fn graph_live(&mut self, runtime: &WorldRuntime) {
        let n = self.graph.model.as_ref().map_or(0, |m| m.nodes.len());
        self.graph.live.clear();
        self.graph.live.resize(n, 0);
        if !self.previewing {
            return;
        }
        let (Some(fsm), Some(model)) = (
            runtime.system::<crate::fsm::FsmSystem>(),
            self.graph.model.as_ref(),
        ) else {
            return;
        };
        for (_, machine, state) in fsm.agent_states() {
            if machine == model.machine {
                if let Some(i) = model.node(state) {
                    self.graph.live[i] = self.graph.live[i].saturating_add(1);
                }
            }
        }
    }

    fn machine_view(&mut self, ui: &mut Ui, runtime: &WorldRuntime, actions: &mut Vec<Action>) {
        self.graph_live(runtime);
        let available = ui.available_size();
        let side = available.x > 2.0 * SIDE;
        let canvas = if side {
            vec2(available.x - SIDE - 8.0, available.y.max(120.0))
        } else {
            vec2(available.x, (available.y * 0.6).max(120.0))
        };
        let model = self.graph.model.take().expect("refreshed");
        let mut out = CanvasOut::default();
        let layout = if side {
            egui::Layout::left_to_right(egui::Align::Min)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        };
        let interactive = !self.dirty();
        ui.with_layout(layout, |ui| {
            out = self.graph.canvas(ui, &model, canvas, interactive);
            ui.vertical(|ui| {
                ui.set_width(if side { SIDE } else { available.x });
                egui::ScrollArea::vertical()
                    .id_salt("graph_side")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.graph.model = Some(model);
                        self.element_panel(ui, actions);
                    });
            });
        });
        if self.graph.model.is_none() {
            return;
        }
        // Canvas requests (after the panel, which may have selected something itself).
        if let Some(sel) = out.select {
            self.graph_select(sel);
        }
        if let Some((name, at)) = out.moved {
            let machine = self.graph.machine.clone().unwrap_or_default();
            let result = ops::fsm_set_node(self.graph_world(), &machine, &name, [at.x, at.y]);
            actions.extend(self.graph_write(result));
        }
        if let Some((from, to)) = out.link {
            let action = self.graph_edit(FsmEdit::AddTransition {
                from,
                to,
                when: None,
            });
            if let Some(action) = action {
                actions.push(action);
                let index = self.graph.model.as_ref().map_or(0, |m| m.transitions);
                self.graph_select(Some(Sel::Transition(index)));
            }
        }
        if let Some(at) = out.add_at {
            actions.extend(self.graph_add_state(Some(at)));
        }
        match out.menu {
            Some(Menu::Initial(s)) => actions.extend(self.graph_edit(FsmEdit::SetInitial(s))),
            Some(Menu::Rename(s)) => {
                self.graph_select(Some(Sel::State(s.clone())));
                self.graph.name = s;
                ui.ctx()
                    .memory_mut(|m| m.request_focus(egui::Id::new("graph_rename")));
            }
            Some(Menu::Delete(sel)) => actions.extend(self.graph_delete(sel)),
            Some(Menu::Raise(i)) if i > 0 => {
                let action = self.graph_edit(FsmEdit::MoveTransition(i, i - 1));
                if action.is_some() {
                    self.graph_select(Some(Sel::Transition(i - 1)));
                }
                actions.extend(action);
            }
            Some(Menu::AddState(at)) => actions.extend(self.graph_add_state(Some(at))),
            _ => {}
        }
    }

    /// Inspector of the selected state or transition (typed forms; live commit).
    fn element_panel(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let Some(sel) = self.graph.selected.clone() else {
            ui.label(RichText::new("Click a state or transition to edit it. Drag a state to move it, drag from its port (right edge) to another state to add a transition, double-click empty space to add a state, right-click for more.").small().color(MUTED));
            self.graph_problem_list(ui);
            return;
        };
        let Some(committed) = self.graph_element(&sel) else {
            return;
        };
        // Reload the draft when it is clean and differs from the world (new revision, a
        // refused write, another selection).
        let reload = match &self.graph.element {
            Some(d) => d.sel != sel || (!d.dirty && d.value != committed),
            None => true,
        };
        if reload {
            self.graph.element = Some(Draft {
                sel: sel.clone(),
                base: committed.clone(),
                value: committed,
                dirty: false,
            });
        }
        let model = self.graph.model.as_ref().expect("set by machine_view");
        let states: Vec<String> = model.nodes.iter().map(|n| n.name.clone()).collect();
        let events = model.events.clone();
        let initial = model.initial.clone();
        let transitions = model.transitions;
        let kinds = self.graph.kinds.take().expect("built with the model");
        let mut draft = self.graph.element.take().expect("set above");
        let mut request = None;
        let mut rename = None;
        let defaults = ("", events.first().map_or("", String::as_str));
        let mixed = Default::default();
        {
            let mut ctx = form::Ctx {
                world: &self.world,
                world_version: self.world_version,
                base: &draft.base,
                mixed: &mixed,
                filter: "",
                structural: false,
                picker: &mut self.picker,
                rects: Some(&mut self.rects),
                overrides: None,
            };
            match &sel {
                Sel::State(name) => {
                    ui.label(
                        RichText::new(format!("STATE  {name}"))
                            .strong()
                            .color(ACCENT),
                    );
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.graph.name)
                                .id(egui::Id::new("graph_rename"))
                                .desired_width(150.0),
                        );
                        if let Some(m) = ctx.rects.as_deref_mut() {
                            m.insert("graph:name".into(), r.rect);
                        }
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let changed = self.graph.name.trim() != name;
                        if (ui
                            .add_enabled(changed, egui::Button::new("Rename"))
                            .clicked()
                            || enter)
                            && changed
                        {
                            rename = Some((name.clone(), self.graph.name.clone()));
                        }
                    });
                    if *name == initial {
                        ui.label(RichText::new("Initial state").small().color(LIVE));
                    } else if ui.small_button("Make initial").clicked() {
                        request = Some(FsmEdit::SetInitial(name.clone()));
                    }
                    ui.label(RichText::new("Action").small().color(MUTED));
                    // A state without `action` stops; the field is written only once edited.
                    match draft.value.get_mut("action") {
                        Some(action) => {
                            draft.dirty |= variant(
                                ui,
                                &mut ctx,
                                &kinds,
                                &kinds.actions,
                                action,
                                "action",
                                defaults,
                                0,
                            );
                        }
                        None => {
                            let mut action = json!({"kind":"stop"});
                            if variant(
                                ui,
                                &mut ctx,
                                &kinds,
                                &kinds.actions,
                                &mut action,
                                "action",
                                defaults,
                                0,
                            ) {
                                draft.value["action"] = action;
                                draft.dirty = true;
                            }
                        }
                    }
                    let state = draft.value.as_object_mut().expect("states are objects");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Emit on enter").small().color(MUTED));
                        let current = state
                            .get("emit")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        egui::ComboBox::from_id_salt("graph_emit")
                            .selected_text(if current.is_empty() {
                                "(none)"
                            } else {
                                current.as_str()
                            })
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(current.is_empty(), "(none)").clicked()
                                    && !current.is_empty()
                                {
                                    state.remove("emit");
                                    draft.dirty = true;
                                }
                                for e in &events {
                                    if ui.selectable_label(*e == current, e).clicked()
                                        && *e != current
                                    {
                                        state.insert("emit".into(), json!(e));
                                        draft.dirty = true;
                                    }
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Clip").small().color(MUTED));
                        let mut clip = state
                            .get("clip")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut clip)
                                    .hint_text("(none)")
                                    .desired_width(100.0),
                            )
                            .changed()
                        {
                            if clip.trim().is_empty() {
                                state.remove("clip");
                            } else {
                                state.insert("clip".into(), json!(clip.trim()));
                            }
                            draft.dirty = true;
                        }
                        if state.contains_key("clip") {
                            let mut looping = state
                                .get("clip_loop")
                                .and_then(Value::as_bool)
                                .unwrap_or(true);
                            if ui.checkbox(&mut looping, "loop").changed() {
                                if looping {
                                    state.remove("clip_loop");
                                } else {
                                    state.insert("clip_loop".into(), json!(false));
                                }
                                draft.dirty = true;
                            }
                        }
                    });
                }
                Sel::Transition(i) => {
                    ui.label(
                        RichText::new(format!("TRANSITION  #{i}  (priority {})", i + 1))
                            .strong()
                            .color(ACCENT),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(*i > 0, egui::Button::new("Up").small())
                            .on_hover_text("Checked earlier: the first passing transition wins")
                            .clicked()
                        {
                            request = Some(FsmEdit::MoveTransition(*i, i - 1));
                        }
                        if ui
                            .add_enabled(i + 1 < transitions, egui::Button::new("Down").small())
                            .clicked()
                        {
                            request = Some(FsmEdit::MoveTransition(*i, i + 1));
                        }
                    });
                    for (end, any) in [("from", true), ("to", false)] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(end).small().color(MUTED));
                            let current = draft.value[end].as_str().unwrap_or("").to_owned();
                            egui::ComboBox::from_id_salt(("graph_end", end))
                                .selected_text(if current == "*" {
                                    "Any state"
                                } else {
                                    current.as_str()
                                })
                                .show_ui(ui, |ui| {
                                    let options = any
                                        .then_some("*")
                                        .into_iter()
                                        .chain(states.iter().map(String::as_str));
                                    for s in options {
                                        let title = if s == "*" { "Any state" } else { s };
                                        if ui.selectable_label(s == current, title).clicked()
                                            && s != current
                                        {
                                            draft.value[end] = json!(s);
                                            draft.dirty = true;
                                        }
                                    }
                                });
                        });
                    }
                    ui.label(RichText::new("When").small().color(MUTED));
                    if let Some(when) = draft.value.get_mut("when") {
                        draft.dirty |= variant(
                            ui,
                            &mut ctx,
                            &kinds,
                            &kinds.conditions,
                            when,
                            "when",
                            defaults,
                            0,
                        );
                    }
                }
            }
        }
        ui.add_space(6.0);
        let delete = ui
            .button(
                RichText::new(match &sel {
                    Sel::State(_) => "Delete state",
                    Sel::Transition(_) => "Delete transition",
                })
                .color(WARNING),
            )
            .clicked();
        // Live: Esc reverts, a settled edit commits as one revision.
        let mut write = None;
        if draft.dirty {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                draft.value = draft.base.clone();
                draft.dirty = false;
            } else if settled(ui, self.saving) {
                write = self.element_edit(&draft);
                draft.base = draft.value.clone();
                draft.dirty = false;
            }
        }
        self.graph.element = Some(draft);
        self.graph.kinds = Some(kinds);
        // The draft first, then a button pressed in the same frame (built on top of it).
        if let Some(edit) = write {
            actions.extend(self.graph_edit(edit));
        }
        if let Some(edit) = request {
            if let FsmEdit::MoveTransition(_, to) = &edit {
                let to = *to;
                let action = self.graph_edit(edit);
                if action.is_some() {
                    self.graph_select(Some(Sel::Transition(to)));
                }
                actions.extend(action);
            } else {
                actions.extend(self.graph_edit(edit));
            }
        }
        if let Some((from, to)) = rename {
            actions.extend(self.graph_rename(&from, &to));
        }
        if delete {
            actions.extend(self.graph_delete(sel));
        }
    }

    /// The machine's problems under the inspector when nothing is selected.
    fn graph_problem_list(&mut self, ui: &mut Ui) {
        let machine = self.graph.machine.clone().unwrap_or_default();
        let mut select = None;
        for p in self
            .graph
            .problems
            .iter()
            .flat_map(|(_, p)| p)
            .filter(|p| p.machine == machine)
            .take(16)
        {
            let text = RichText::new(&p.message).small().color(WARNING);
            if ui
                .add(egui::Label::new(text).sense(Sense::click()))
                .clicked()
            {
                select = p
                    .state
                    .clone()
                    .map(Sel::State)
                    .or(p.transition.map(Sel::Transition));
            }
        }
        if select.is_some() {
            self.graph_select(select);
        }
    }

    /// Problems tab: the static state machine checks (cached per revision), each with Open.
    pub(super) fn graph_problem_rows(&mut self, ui: &mut Ui) {
        if self
            .graph
            .problems
            .as_ref()
            .is_none_or(|(k, _)| *k != (self.world_version, self.graph.edits))
        {
            let problems = ops::fsm_problems(self.graph_world());
            self.graph.problems = Some(((self.world_version, self.graph.edits), problems));
        }
        let mut open = None;
        let problems = &self.graph.problems.as_ref().expect("set above").1;
        if problems.is_empty() {
            return;
        }
        ui.label(
            RichText::new(format!("STATE MACHINES · {} check(s)", problems.len()))
                .small()
                .color(MUTED),
        );
        for (n, p) in problems.iter().enumerate().take(64) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(p.code).monospace().color(WARNING));
                ui.label(RichText::new(&p.path).monospace().small());
                ui.label(&p.message);
                let r = ui.small_button("Open graph");
                #[cfg(test)]
                self.rects.insert(format!("problem:graph:{n}"), r.rect);
                let _ = n;
                if r.clicked() {
                    open = Some((p.machine.clone(), p.state.clone(), p.path.clone()));
                }
            });
        }
        if let Some((machine, state, path)) = open {
            // An agent naming a missing machine: select the agent instead.
            if let Some(entity) = super::entity_in_path(&path, &self.world) {
                self.try_select(Some(entity));
                self.frame_selection = true;
                return;
            }
            match self.graph_open(&machine) {
                Ok(()) => self.graph_select(state.map(Sel::State)),
                Err(e) => self.problem(&e),
            }
        }
    }

    /// Rule table of `behavior.program`: add / remove / reorder rows, edit the selected rule
    /// (event, condition, actions) with typed forms.
    fn rules_view(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        // An Arc of the world: rows are read in place, not copied each frame.
        let world = self.graph_world_arc();
        let Some(program) = world.resources.get("behavior.program") else {
            ui.label(
                RichText::new(
                    "No behavior.program. Declare an event (World › Declare Event…) to create one.",
                )
                .small()
                .color(MUTED),
            );
            return;
        };
        let rules: &[Value] = program["rules"].as_array().map_or(&[], Vec::as_slice);
        let events = names(program.get("events"));
        let variables = names(program.get("variables"));
        let mut array = None;
        let mut add = false;
        let mut row = None;
        ui.horizontal(|ui| {
            let r = ui
                .add_enabled(!events.is_empty() && self.saving == 0, egui::Button::new("+ Rule"))
                .on_hover_text("ACL: append resources[id=behavior.program] rules {\"id\":..,\"event\":..,\"actions\":[]}");
            self.rects.insert("graph:rule:add".into(), r.rect);
            add = r.clicked();
            ui.label(RichText::new(format!("{} rule(s) · evaluated in order per event", rules.len())).small().color(MUTED));
        });
        let len = rules.len();
        // Rows hold buttons: the row height show_rows virtualizes with must match them.
        let row_height = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical()
            .id_salt("graph_rules")
            .max_height((ui.available_height() * 0.4).max(100.0))
            .show_rows(ui, row_height, len, |ui, range| {
                for i in range {
                    let rule = &rules[i];
                    ui.horizontal(|ui| {
                        let selected = self.graph.rule_index == Some(i);
                        let text = format!(
                            "{:>3}  {:<16} on {:<14} when {:<8} {} action(s)",
                            i,
                            rule["id"].as_str().unwrap_or("?"),
                            rule["event"].as_str().unwrap_or("?"),
                            rule["when"]["kind"].as_str().unwrap_or("always"),
                            rule["actions"].as_array().map_or(0, Vec::len)
                        );
                        let r = ui
                            .selectable_label(selected, RichText::new(text).monospace().size(11.0));
                        self.rects.insert(format!("graph:rule:{i}"), r.rect);
                        if r.clicked() && !selected {
                            row = Some(i);
                        }
                        let clean = self.saving == 0;
                        let up = ui.add_enabled(clean && i > 0, egui::Button::new("Up").small());
                        self.rects.insert(format!("graph:rule:up:{i}"), up.rect);
                        if up.clicked() {
                            array = Some(axiom_core::ops::ArrayEdit::Move(i, i - 1));
                        }
                        if ui
                            .add_enabled(clean && i + 1 < len, egui::Button::new("Down").small())
                            .clicked()
                        {
                            array = Some(axiom_core::ops::ArrayEdit::Move(i, i + 1));
                        }
                        let x = ui.add_enabled(clean, egui::Button::new("x").small());
                        self.rects.insert(format!("graph:rule:remove:{i}"), x.rect);
                        if x.clicked() {
                            array = Some(axiom_core::ops::ArrayEdit::Remove(i));
                        }
                    });
                }
            });
        ui.separator();
        if let Some(i) = row {
            // The selected rule's unsaved edit is written before another rule opens.
            self.graph_flush();
            self.graph.rule_index = Some(i);
        }
        if let Some(i) = self.graph.rule_index.filter(|i| *i < len) {
            egui::ScrollArea::vertical()
                .id_salt("graph_rule_panel")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.rule_panel(ui, i, &rules[i], &events, &variables, actions)
                });
        }
        if add {
            match ops::new_rule(self.graph_world(), None) {
                Ok(rule) => {
                    let edit = axiom_core::ops::ArrayEdit::Append(rule);
                    let result = ops::edit_array(
                        self.graph_world(),
                        &self.registry,
                        &ops::program_selection(),
                        "/rules",
                        &edit,
                    );
                    if let Some(action) = self.graph_write(result) {
                        actions.push(action);
                        self.graph.rule_index = Some(len);
                        self.graph.rule = None;
                    }
                }
                Err(e) => self.problem(&e),
            }
        }
        if let Some(edit) = array {
            self.graph_flush();
            actions.append(&mut self.graph.queued);
            let result = ops::edit_array(
                self.graph_world(),
                &self.registry,
                &ops::program_selection(),
                "/rules",
                &edit,
            );
            if let Some(action) = self.graph_write(result) {
                actions.push(action);
                self.graph.rule_index = match edit {
                    axiom_core::ops::ArrayEdit::Move(_, to) => Some(to),
                    _ => None,
                };
                self.graph.rule = None;
            }
        }
    }

    fn rule_panel(
        &mut self,
        ui: &mut Ui,
        index: usize,
        committed: &Value,
        events: &[String],
        variables: &[String],
        actions: &mut Vec<Action>,
    ) {
        let reload = match &self.graph.rule {
            Some(d) => d.index != index || (!d.dirty && d.base != *committed),
            None => true,
        };
        if reload {
            self.graph.rule = Some(RuleDraft {
                index,
                id: committed["id"].as_str().unwrap_or("").to_owned(),
                base: committed.clone(),
                value: committed.clone(),
                dirty: false,
            });
        }
        if self.graph.kinds.is_none() {
            self.graph.kinds = Some(Kinds::new());
        }
        let kinds = self.graph.kinds.take().expect("set above");
        let mut draft = self.graph.rule.take().expect("set above");
        let defaults = (
            variables.first().map_or("", String::as_str),
            events.first().map_or("", String::as_str),
        );
        let mixed = Default::default();
        let id = committed["id"].as_str().unwrap_or("").to_owned();
        let mut remove_action = None;
        let mut move_action = None;
        let mut add_action = None;
        {
            let mut ctx = form::Ctx {
                world: &self.world,
                world_version: self.world_version,
                base: &draft.base,
                mixed: &mixed,
                filter: "",
                structural: false,
                picker: &mut self.picker,
                rects: Some(&mut self.rects),
                overrides: None,
            };
            ui.label(RichText::new(format!("RULE  {id}")).strong().color(ACCENT));
            ui.horizontal(|ui| {
                ui.label(RichText::new("on event").small().color(MUTED));
                let current = draft.value["event"].as_str().unwrap_or("").to_owned();
                let r = egui::ComboBox::from_id_salt("graph_rule_event")
                    .selected_text(current.as_str())
                    .show_ui(ui, |ui| {
                        for e in events {
                            let r = ui.selectable_label(*e == current, e);
                            #[cfg(test)]
                            GRAPH_RECTS.with(|g| {
                                g.borrow_mut()
                                    .push((format!("graph:rule:event:{e}"), r.rect))
                            });
                            if r.clicked() && *e != current {
                                draft.value["event"] = json!(e);
                                draft.dirty = true;
                            }
                        }
                    });
                if let Some(m) = ctx.rects.as_deref_mut() {
                    m.insert("graph:rule:event".into(), r.response.rect);
                }
            });
            ui.label(RichText::new("when").small().color(MUTED));
            // A rule without `when` runs always; the field is written only once edited.
            match draft.value.get_mut("when") {
                Some(when) => {
                    draft.dirty |= variant(
                        ui,
                        &mut ctx,
                        &kinds,
                        &kinds.rule_conditions,
                        when,
                        "rule/when",
                        defaults,
                        0,
                    );
                }
                None => {
                    let mut when = json!({"kind":"always"});
                    if variant(
                        ui,
                        &mut ctx,
                        &kinds,
                        &kinds.rule_conditions,
                        &mut when,
                        "rule/when",
                        defaults,
                        0,
                    ) {
                        draft.value["when"] = when;
                        draft.dirty = true;
                    }
                }
            }
            ui.label(RichText::new("actions").small().color(MUTED));
            let list = draft.value["actions"].as_array_mut();
            if let Some(list) = list {
                let n = list.len();
                for (k, a) in list.iter_mut().enumerate().take(64) {
                    ui.push_id(("rule_action", k), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{k}")).small().monospace());
                            if ui
                                .add_enabled(k > 0, egui::Button::new("Up").small())
                                .clicked()
                            {
                                move_action = Some((k, k - 1));
                            }
                            if ui
                                .add_enabled(k + 1 < n, egui::Button::new("Down").small())
                                .clicked()
                            {
                                move_action = Some((k, k + 1));
                            }
                            if ui.small_button("x").clicked() {
                                remove_action = Some(k);
                            }
                        });
                        ui.indent("action", |ui| {
                            draft.dirty |= variant(
                                ui,
                                &mut ctx,
                                &kinds,
                                &kinds.rule_actions,
                                a,
                                &format!("rule/actions/{k}"),
                                defaults,
                                0,
                            );
                        });
                    });
                }
            }
            let menu = ui.menu_button("+ Action", |ui| {
                for k in &kinds.rule_actions {
                    let r = ui.button(k.name);
                    #[cfg(test)]
                    GRAPH_RECTS.with(|g| {
                        g.borrow_mut()
                            .push((format!("graph:rule:action:{}", k.name), r.rect))
                    });
                    if r.clicked() {
                        add_action = Some(k.name);
                        ui.close_menu();
                    }
                }
            });
            if let Some(m) = ctx.rects.as_deref_mut() {
                m.insert("graph:rule:add_action".into(), menu.response.rect);
            }
        }
        if let Some(list) = draft.value["actions"].as_array_mut() {
            if let Some(k) = remove_action.filter(|k| *k < list.len()) {
                list.remove(k);
                draft.dirty = true;
            }
            if let Some((from, to)) =
                move_action.filter(|(f, t)| *f < list.len() && *t < list.len())
            {
                let a = list.remove(from);
                list.insert(to, a);
                draft.dirty = true;
            }
            if let Some(name) = add_action {
                if let Some(k) = kinds.rule_actions.iter().find(|k| k.name == name) {
                    let mut v = k.template.clone();
                    if v.get("variable").is_some_and(|x| x == "") && !defaults.0.is_empty() {
                        v["variable"] = json!(defaults.0);
                    }
                    if v.get("event").is_some_and(|x| x == "") && !defaults.1.is_empty() {
                        v["event"] = json!(defaults.1);
                    }
                    list.push(v);
                    draft.dirty = true;
                }
            }
        }
        let mut write = None;
        if draft.dirty {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                draft.value = draft.base.clone();
                draft.dirty = false;
            } else if settled(ui, self.saving) {
                write = self.rule_write(&draft);
                draft.base = draft.value.clone();
                draft.dirty = false;
            }
        }
        self.graph.rule = Some(draft);
        self.graph.kinds = Some(kinds);
        actions.extend(write);
    }

    /// One `set rules[id=ID] FIELD VALUE` per field the draft changed, in one revision, if
    /// the rule at the draft's index is still the one it was loaded from.
    fn rule_write(&mut self, draft: &RuleDraft) -> Option<Action> {
        let current = self
            .graph_world()
            .resources
            .get("behavior.program")
            .and_then(|p| p["rules"].get(draft.index));
        if current != Some(&draft.base) {
            self.set_error(format!(
                "Rule {} changed in another revision; your unsaved edit was dropped",
                draft.id
            ));
            return None;
        }
        let mut edits = vec![];
        for field in ["event", "when", "actions"] {
            let Some(value) = draft
                .value
                .get(field)
                .filter(|v| draft.base.get(field) != Some(*v))
            else {
                continue;
            };
            match ops::set_rule(self.graph_world(), &draft.id, field, value.clone()) {
                Ok(more) => edits.extend(more),
                Err(e) => {
                    self.problem(&e);
                    return None;
                }
            }
        }
        self.graph_write(Ok(edits))
    }

    #[cfg(test)]
    pub(super) fn graph_problems_for_test(&self) -> &[GraphProblem] {
        self.graph
            .problems
            .as_ref()
            .map_or(&[], |(_, p)| p.as_slice())
    }

    #[cfg(test)]
    pub(super) fn graph_live_for_test(&self) -> Vec<u16> {
        self.graph.live.clone()
    }

    #[cfg(test)]
    pub(super) fn graph_node_at_for_test(&self, state: &str) -> Pos2 {
        let model = self.graph.model.as_ref().unwrap();
        model.nodes[model.node(state).unwrap()].at
    }

    #[cfg(test)]
    pub(super) fn graph_node_index_for_test(&self, state: &str) -> Option<usize> {
        self.graph.model.as_ref()?.node(state)
    }

    /// Play preview (this build draws the game UI instead of the editor): the open machine,
    /// read-only, with each agent's current state highlighted. Drawn when the behavior
    /// overlay is on and the Graphs tab has a machine open.
    pub(crate) fn play_graph_overlay(&mut self, ctx: &egui::Context, runtime: &WorldRuntime) {
        if !self.overlay || self.graph.machine.is_none() {
            return;
        }
        self.graph_refresh();
        self.graph_live(runtime);
        let Some(model) = self.graph.model.take() else {
            return;
        };
        if model.exists {
            egui::Window::new(format!("FSM · {} (live)", model.machine))
                .id(egui::Id::new("play_graph_overlay"))
                .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
                .resizable(false)
                .collapsible(true)
                .show(ctx, |ui| {
                    let extent = model.nodes.iter().fold(vec2(260.0, 120.0), |e, n| {
                        e.max(n.at.to_vec2() + NODE + vec2(16.0, 16.0))
                    });
                    let size = extent.min(vec2(720.0, 360.0));
                    // The overlay is sized in canvas units: drawn at 100 %, whatever the
                    // Graphs tab's zoom.
                    let zoom = std::mem::replace(&mut self.graph.zoom, 1.0);
                    self.graph.canvas(ui, &model, size, false);
                    self.graph.zoom = zoom;
                });
        }
        self.graph.model = Some(model);
    }
}
