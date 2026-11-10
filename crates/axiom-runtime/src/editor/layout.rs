//! Dockable editor panels: the `egui_dock` state, its persisted form and the built-in layouts.
//!
//! The layout is saved in the editor preferences as our own small JSON tree of tab names, not
//! as `egui_dock`'s serde form: unknown or repeated names are dropped one by one (a newer build
//! may add tabs), and the crate can be upgraded without touching the file format.
//!
//! Spec v1: `{"v":1,"main":NODE|null,"windows":[{"rect":[x,y,w,h],"node":NODE}]}` where
//! `NODE = {"tabs":["scene",…],"active":0} | {"split":"h"|"v","fraction":F,"a":NODE,"b":NODE}`
//! and `F` is the share of `a` (left / top).
use super::Dock;
use egui_dock::{DockState, Node, NodeIndex, Split, Surface, SurfaceIndex, TabIndex, Tree};
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};

/// Built-in named layouts (not stored in the preferences file).
pub(crate) const BUILTIN: [&str; 3] = ["Default", "Tall Inspector", "Classic"];
/// Version of the built-in Default. A saved current layout from an older Default is replaced
/// by the new one once (named layouts are kept; Classic is the previous arrangement).
pub(crate) const LAYOUT_VERSION: u32 = 3;
/// The tabs that shared the old bottom dock; a reopened one joins the others.
const BOTTOM: [Dock; 11] = [
    Dock::Changes,
    Dock::Proposals,
    Dock::History,
    Dock::Problems,
    Dock::Scenarios,
    Dock::Graphs,
    Dock::Assets,
    Dock::Agent,
    Dock::Console,
    Dock::Profiler,
    Dock::Trace,
];
const MAX_DEPTH: usize = 12;
const MAX_WINDOWS: usize = 8;
pub(crate) const MAX_NAMED: usize = 32;
/// Where the World tab opens when it is not in the layout (the old floating window).
const WORLD_WINDOW: [f32; 4] = [300.0, 50.0, 400.0, 560.0];

impl Dock {
    pub(crate) const ALL: [Dock; 21] = [
        Dock::Hierarchy,
        Dock::Scene,
        Dock::Game,
        Dock::Inspector,
        Dock::Changes,
        Dock::Proposals,
        Dock::History,
        Dock::Problems,
        Dock::Scenarios,
        Dock::Graphs,
        Dock::Assets,
        Dock::Agent,
        Dock::World,
        Dock::UiEditor,
        Dock::Console,
        Dock::Profiler,
        Dock::Trace,
        Dock::InputMap,
        Dock::Localization,
        Dock::Animation,
        Dock::Vfx,
    ];
    /// Stable name used by the preferences file, `window.*` commands and `editor_view {dock}`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Dock::Hierarchy => "hierarchy",
            Dock::Scene => "scene",
            Dock::Game => "game",
            Dock::Inspector => "inspector",
            Dock::Changes => "changes",
            Dock::Proposals => "proposals",
            Dock::History => "history",
            Dock::Problems => "problems",
            Dock::Scenarios => "scenarios",
            Dock::Graphs => "graphs",
            Dock::Assets => "assets",
            Dock::Agent => "agent",
            Dock::World => "world",
            Dock::UiEditor => "ui_editor",
            Dock::Console => "console",
            Dock::Profiler => "profiler",
            Dock::Trace => "trace",
            Dock::InputMap => "input_map",
            Dock::Localization => "localization",
            Dock::Animation => "animation",
            Dock::Vfx => "vfx",
        }
    }
    pub(crate) fn from_name(name: &str) -> Option<Dock> {
        Dock::ALL.into_iter().find(|d| d.name() == name)
    }
    pub(crate) fn title(self) -> &'static str {
        match self {
            Dock::Hierarchy => "Hierarchy",
            Dock::Scene => "Scene",
            Dock::Game => "Game",
            Dock::Inspector => "Inspector",
            Dock::Changes => "Changes",
            Dock::Proposals => "Proposals",
            Dock::History => "History",
            Dock::Problems => "Problems",
            Dock::Scenarios => "Scenarios",
            Dock::Graphs => "Graphs",
            Dock::Assets => "Assets",
            Dock::Agent => "AI / Protocol",
            Dock::World => "World",
            Dock::UiEditor => "UI Editor",
            Dock::Console => "Console",
            Dock::Profiler => "Profiler",
            Dock::Trace => "Event Trace",
            Dock::InputMap => "Input Map",
            Dock::Localization => "Localization",
            Dock::Animation => "Animation",
            Dock::Vfx => "VFX",
        }
    }
    fn bottom(self) -> bool {
        BOTTOM.contains(&self)
    }
    /// Tuning editors: closed in every built-in layout, opened into the bottom strip.
    fn tuning(self) -> bool {
        matches!(
            self,
            Dock::InputMap | Dock::Localization | Dock::Animation | Dock::Vfx
        )
    }
}

/// The editor's dock and what was last written to the preferences.
pub(crate) struct Layout {
    /// `None` only while `Editor::draw` lends it to the dock area.
    pub dock: Option<DockState<Dock>>,
    /// `show_dock` requests made while the dock area was being drawn.
    pub pending: Vec<Dock>,
    /// Fingerprint of the state last copied into `Prefs::layout` (or loaded from it).
    pub synced: u64,
    /// Open tabs for `observe`, rebuilt when the fingerprint changes.
    observed: (u64, Value),
    /// The dock look, derived from the egui style once.
    pub style: Option<egui_dock::Style>,
    /// Window ▸ Save Layout As… prompt text while it is open.
    pub save_as: Option<String>,
    /// Times the persisted spec was rebuilt (tests: not per frame in steady state).
    #[cfg(test)]
    pub syncs: usize,
}

impl Default for Layout {
    fn default() -> Self {
        let dock = default_state();
        Self {
            synced: fingerprint(&dock),
            dock: Some(dock),
            pending: vec![],
            observed: (0, Value::Null),
            style: None,
            save_as: None,
            #[cfg(test)]
            syncs: 0,
        }
    }
}

impl Layout {
    /// Applies the requests queued while the dock was borrowed.
    pub(crate) fn apply_pending(&mut self) {
        if let Some(state) = &mut self.dock {
            for tab in self.pending.drain(..) {
                show(state, tab);
            }
        }
    }

    /// Open tabs (cached per layout and focus; current even before the next frame syncs).
    pub(crate) fn observed(&self) -> Value {
        match &self.dock {
            Some(state) if observed_key(state) == self.observed.0 => self.observed.1.clone(),
            Some(state) => Value::Array(open_tabs(state)),
            None => Value::Null,
        }
    }

    /// Refreshes the `observe` cache; returns the persisted spec when the layout changed since
    /// it was last synced (`None` = unchanged, nothing to write).
    pub(crate) fn sync(&mut self) -> Option<Value> {
        let state = self.dock.as_ref()?;
        let key = observed_key(state);
        if self.observed.0 != key || self.observed.1.is_null() {
            self.observed = (key, Value::Array(open_tabs(state)));
        }
        let print = fingerprint(state);
        if print == self.synced {
            return None;
        }
        self.synced = print;
        #[cfg(test)]
        {
            self.syncs += 1;
        }
        Some(to_spec(state))
    }

    /// Replaces the dock; it counts as already persisted when `persisted` (just loaded).
    pub(crate) fn replace(&mut self, state: DockState<Dock>, persisted: bool) {
        let print = fingerprint(&state);
        self.synced = if persisted { print } else { print ^ 1 };
        self.dock = Some(state);
        self.pending.clear();
    }
}

/// A validated layout tree (tab names resolved, empty leaves pruned, fractions clamped).
#[derive(Debug, PartialEq)]
enum Spec {
    Leaf(Vec<Dock>, usize),
    Split {
        vertical: bool,
        fraction: f32,
        a: Box<Spec>,
        b: Box<Spec>,
    },
}

impl Spec {
    /// Parses one node. Unknown and already used names go to `dropped`; a leaf left without
    /// tabs disappears (its split collapses into the other side). `Err` = not a layout node.
    fn parse(
        value: &Value,
        depth: usize,
        used: &mut Vec<Dock>,
        dropped: &mut Vec<String>,
    ) -> Result<Option<Spec>, String> {
        if depth > MAX_DEPTH {
            return Err(format!("nested deeper than {MAX_DEPTH}"));
        }
        if let Some(tabs) = value.get("tabs") {
            let tabs = tabs.as_array().ok_or("\"tabs\" is not a list")?;
            let active_name = tabs
                .get(value.get("active").and_then(Value::as_u64).unwrap_or(0) as usize)
                .and_then(Value::as_str);
            let mut leaf = vec![];
            let mut active = 0;
            for name in tabs {
                let name = name.as_str().ok_or("a tab name is not a string")?;
                match Dock::from_name(name).filter(|d| !used.contains(d)) {
                    Some(dock) => {
                        if Some(name) == active_name {
                            active = leaf.len();
                        }
                        used.push(dock);
                        leaf.push(dock);
                    }
                    // Reported in the status line: a few names are enough.
                    None if dropped.len() < 8 => dropped.push(name.chars().take(32).collect()),
                    None => {}
                }
            }
            return Ok((!leaf.is_empty()).then_some(Spec::Leaf(leaf, active)));
        }
        let vertical = match value.get("split").and_then(Value::as_str) {
            Some("v") => true,
            Some("h") => false,
            _ => return Err("a node needs \"tabs\" or \"split\":\"h\"|\"v\"".into()),
        };
        let fraction = value
            .get("fraction")
            .and_then(Value::as_f64)
            .map(|f| f as f32)
            .filter(|f| f.is_finite())
            .unwrap_or(0.5)
            .clamp(0.05, 0.95);
        let a = Self::parse(
            value.get("a").ok_or("a split needs \"a\"")?,
            depth + 1,
            used,
            dropped,
        )?;
        let b = Self::parse(
            value.get("b").ok_or("a split needs \"b\"")?,
            depth + 1,
            used,
            dropped,
        )?;
        Ok(match (a, b) {
            (Some(a), Some(b)) => Some(Spec::Split {
                vertical,
                fraction,
                a: Box::new(a),
                b: Box::new(b),
            }),
            (one, None) | (None, one) => one,
        })
    }

    /// The tabs of the first (left / top-most) leaf.
    fn first_tabs(&self) -> Vec<Dock> {
        match self {
            Spec::Leaf(tabs, _) => tabs.clone(),
            Spec::Split { a, .. } => a.first_tabs(),
        }
    }

    /// Builds this node at `at`, which holds a leaf with `self.first_tabs()`.
    fn place(&self, tree: &mut Tree<Dock>, at: NodeIndex) {
        match self {
            Spec::Leaf(_, active) => tree.set_active_tab(at, TabIndex(*active)),
            Spec::Split {
                vertical,
                fraction,
                a,
                b,
            } => {
                // Right / Below keep the old leaf first, so `fraction` is the share of `a`.
                let split = if *vertical {
                    Split::Below
                } else {
                    Split::Right
                };
                let [first, second] =
                    tree.split(at, split, *fraction, Node::leaf_with(b.first_tabs()));
                a.place(tree, first);
                b.place(tree, second);
            }
        }
    }
}

/// The persisted form of one surface node (`Value::Null` for an empty tree).
fn node_spec(tree: &Tree<Dock>, at: NodeIndex) -> Value {
    if at.0 >= tree.len() {
        return Value::Null;
    }
    let round = |f: f32| (f as f64 * 1000.0).round() / 1000.0;
    let split = |kind: &str, fraction: f32| {
        let a = node_spec(tree, at.left());
        let b = node_spec(tree, at.right());
        match (a.is_null(), b.is_null()) {
            (false, false) => json!({"split":kind,"fraction":round(fraction),"a":a,"b":b}),
            (true, _) => b,
            (_, true) => a,
        }
    };
    match &tree[at] {
        Node::Empty => Value::Null,
        Node::Leaf { tabs, active, .. } => {
            json!({"tabs": tabs.iter().map(|t| t.name()).collect::<Vec<_>>(), "active": active.0})
        }
        Node::Horizontal { fraction, .. } => split("h", *fraction),
        Node::Vertical { fraction, .. } => split("v", *fraction),
    }
}

/// The persisted form of a dock state.
pub(crate) fn to_spec(state: &DockState<Dock>) -> Value {
    let mut windows = vec![];
    let mut main = Value::Null;
    for surface in state.iter_surfaces() {
        match surface {
            Surface::Main(tree) => main = node_spec(tree, NodeIndex::root()),
            Surface::Window(tree, window) => {
                let node = node_spec(tree, NodeIndex::root());
                if node.is_null() {
                    continue;
                }
                let r = window.rect();
                let rect = if r.is_finite() && r.is_positive() {
                    [r.min.x, r.min.y, r.width(), r.height()].map(|v| v.round())
                } else {
                    WORLD_WINDOW
                };
                windows.push(json!({"rect":rect,"node":node}));
            }
            Surface::Empty => {}
        }
    }
    json!({"v":1,"main":main,"windows":windows})
}

/// Builds a dock state from a persisted spec. `Ok` carries the names that were dropped
/// (unknown or repeated); `Err` means the spec is unusable and the caller falls back.
pub(crate) fn from_spec(spec: &Value) -> Result<(DockState<Dock>, Vec<String>), String> {
    if spec.get("v").and_then(Value::as_u64) != Some(1) {
        return Err("unsupported layout version (expected \"v\":1)".into());
    }
    let (mut used, mut dropped) = (vec![], vec![]);
    let main = match spec.get("main") {
        None | Some(Value::Null) => None,
        Some(node) => Spec::parse(node, 0, &mut used, &mut dropped)?,
    };
    let mut windows = vec![];
    let listed = match spec.get("windows") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(list)) => list.as_slice(),
        Some(_) => return Err("\"windows\" is not a list".into()),
    };
    for window in listed.iter().take(MAX_WINDOWS) {
        let rect: Option<[f32; 4]> = window
            .get("rect")
            .and_then(|r| serde_json::from_value::<[f32; 4]>(r.clone()).ok())
            .filter(|r| r.iter().all(|v| v.is_finite()));
        let node = window.get("node").ok_or("a window needs \"node\"")?;
        if let Some(node) = Spec::parse(node, 0, &mut used, &mut dropped)? {
            windows.push((rect.unwrap_or(WORLD_WINDOW), node));
        }
    }
    let mut state = DockState::new(main.as_ref().map(Spec::first_tabs).unwrap_or_default());
    match &main {
        Some(main) => main.place(state.main_surface_mut(), NodeIndex::root()),
        // `DockState::new(vec![])` leaves an empty leaf; an empty main surface has no nodes.
        None => state.main_surface_mut().remove_leaf(NodeIndex::root()),
    }
    for ([x, y, w, h], node) in windows {
        let surface = state.add_window(node.first_tabs());
        node.place(&mut state[surface], NodeIndex::root());
        if let Some(window) = state.get_window_state_mut(surface) {
            window
                .set_position(egui::pos2(
                    x.clamp(-4096.0, 16384.0),
                    y.clamp(-4096.0, 16384.0),
                ))
                .set_size(egui::vec2(w.clamp(120.0, 8192.0), h.clamp(80.0, 8192.0)));
        }
    }
    Ok((state, dropped))
}

/// A built-in layout by name.
pub(crate) fn builtin(name: &str) -> Option<Value> {
    let bottom: Vec<&str> = BOTTOM.iter().map(|d| d.name()).collect();
    match name {
        // Hierarchy | Scene, Game, Graphs over Assets, Console, Problems, History |
        // full-height Inspector. Changes, Proposals, Scenarios, AI / Protocol, Profiler and
        // Event Trace open from the Window menu (into the bottom leaf).
        "Default" => Some(json!({"v":1,"main":{
            "split":"h","fraction":0.17,
            "a":{"tabs":["hierarchy"]},
            "b":{"split":"h","fraction":0.77,
                "a":{"split":"v","fraction":0.7,
                    "a":{"tabs":["scene","game","graphs"],"active":0},
                    "b":{"tabs":["assets","console","problems","history"],"active":0}},
                "b":{"tabs":["inspector"]}}},"windows":[]})),
        // The inspector gets the full window height (long component forms on 1080p screens).
        "Tall Inspector" => Some(json!({"v":1,"main":{
            "split":"h","fraction":0.72,
            "a":{"split":"v","fraction":0.7,
                "a":{"split":"h","fraction":0.24,"a":{"tabs":["hierarchy"]},"b":{"tabs":["scene","game","graphs"],"active":0}},
                "b":{"tabs":["assets","console","problems","history"],"active":0}},
            "b":{"tabs":["inspector"]}},"windows":[]})),
        // The arrangement before M-UI: every tool tab in one full-width bottom strip.
        "Classic" => Some(json!({"v":1,"main":{
            "split":"v","fraction":0.72,
            "a":{"split":"h","fraction":0.18,
                "a":{"tabs":["hierarchy"]},
                "b":{"split":"h","fraction":0.71,"a":{"tabs":["scene","game"]},"b":{"tabs":["inspector"]}}},
            "b":{"tabs":bottom,"active":0}},"windows":[]})),
        _ => None,
    }
}

pub(crate) fn default_state() -> DockState<Dock> {
    from_spec(&builtin("Default").unwrap())
        .expect("the built-in layout is valid")
        .0
}

/// The dock state is lent to the dock area while the tabs are drawn (commands that rearrange
/// it are not run from inside a tab).
pub(crate) fn busy() -> anyhow::Error {
    anyhow::anyhow!("the dock is being drawn; run layout commands from the menu or the bridge")
}

/// Opens `tab` if it is closed, then makes it the active, focused tab of its leaf.
/// A reopened tab returns near where the default layout puts it.
pub(crate) fn show(state: &mut DockState<Dock>, tab: Dock) {
    if let Some((surface, node, index)) = state.find_tab(&tab) {
        state.set_active_tab((surface, node, index));
        state.set_focused_node_and_surface((surface, node));
        return;
    }
    // Canvas tabs (UI Editor, Graphs) open beside Scene.
    if matches!(tab, Dock::UiEditor | Dock::Graphs) {
        if let Some((surface, node, _)) = state.find_tab(&Dock::Scene) {
            if let Node::Leaf { tabs, active, .. } = &mut state[surface][node] {
                tabs.push(tab);
                *active = TabIndex(tabs.len() - 1);
            }
            state.set_focused_node_and_surface((surface, node));
            return;
        }
    }
    if tab.bottom() || tab.tuning() {
        if let Some((surface, node, _)) = BOTTOM.iter().find_map(|b| state.find_tab(b)) {
            if let Node::Leaf { tabs, active, .. } = &mut state[surface][node] {
                tabs.push(tab);
                *active = TabIndex(tabs.len() - 1);
            }
            state.set_focused_node_and_surface((surface, node));
            return;
        }
    }
    let main = SurfaceIndex::main();
    // Game and Scene share a leaf (as in the built-in layouts) when the other one is open.
    let partner = match tab {
        Dock::Game => Some(Dock::Scene),
        Dock::Scene => Some(Dock::Game),
        _ => None,
    };
    if let Some((surface, node, _)) = partner.and_then(|p| state.find_tab(&p)) {
        if let Node::Leaf { tabs, active, .. } = &mut state[surface][node] {
            tabs.push(tab);
            *active = TabIndex(tabs.len() - 1);
        }
        state.set_focused_node_and_surface((surface, node));
        return;
    }
    if tab == Dock::World {
        let surface = state.add_window(vec![tab]);
        let [x, y, w, h] = WORLD_WINDOW;
        if let Some(window) = state.get_window_state_mut(surface) {
            window
                .set_position(egui::pos2(x, y))
                .set_size(egui::vec2(w, h));
        }
        state.set_focused_node_and_surface((surface, NodeIndex::root()));
        return;
    }
    if state.main_surface().is_empty() {
        state.main_surface_mut().push_to_first_leaf(tab);
        state.set_focused_node_and_surface((main, NodeIndex::root()));
        return;
    }
    // A node's fraction is the share of its left / top child.
    let root = NodeIndex::root();
    let tree = state.main_surface_mut();
    let [_, new] = match tab {
        Dock::Hierarchy => tree.split_left(root, 0.18, vec![tab]),
        Dock::Inspector => tree.split_right(root, 0.76, vec![tab]),
        Dock::Scene | Dock::Game => tree.split_above(root, 0.7, vec![tab]),
        _ => tree.split_below(root, 0.72, vec![tab]),
    };
    state.set_focused_node_and_surface((main, new));
}

/// Closes `tab`; false when it was not open.
pub(crate) fn close(state: &mut DockState<Dock>, tab: Dock) -> bool {
    match state.find_tab(&tab) {
        Some(found) => state.remove_tab(found).is_some(),
        None => false,
    }
}

/// Whether `tab` is open (anywhere) / the active tab of its leaf.
pub(crate) fn is_open(state: &DockState<Dock>, tab: Dock) -> bool {
    state.find_tab(&tab).is_some()
}
pub(crate) fn is_active(state: &DockState<Dock>, tab: Dock) -> bool {
    state
        .find_tab(&tab)
        .is_some_and(|(surface, node, index)| match &state[surface][node] {
            Node::Leaf { active, .. } => *active == index,
            _ => false,
        })
}

/// The focused tab (active tab of the focused leaf), if any.
pub(crate) fn focused(state: &DockState<Dock>) -> Option<Dock> {
    let (surface, node) = state.focused_leaf()?;
    match &state[surface][node] {
        Node::Leaf { tabs, active, .. } => tabs.get(active.0).copied(),
        _ => None,
    }
}

/// The active tab of the leaf holding the old bottom-dock tabs (kept in prefs as `dock`).
pub(crate) fn active_bottom(state: &DockState<Dock>) -> Option<Dock> {
    BOTTOM
        .into_iter()
        .find(|d| is_active(state, *d))
        .or_else(|| BOTTOM.into_iter().find(|d| is_open(state, *d)))
}

/// Open tabs in surface/node order with their state, for `observe` and `discover`.
pub(crate) fn open_tabs(state: &DockState<Dock>) -> Vec<Value> {
    let focused = focused(state);
    state
        .iter_all_tabs()
        .map(|((surface, _), tab)| {
            let mut row = json!({"tab": tab.name(), "active": is_active(state, *tab)});
            if !surface.is_main() {
                row["floating"] = json!(true);
            }
            if Some(*tab) == focused {
                row["focused"] = json!(true);
            }
            row
        })
        .collect()
}

/// A cheap, allocation-free fingerprint of everything the persisted spec contains.
pub(crate) fn fingerprint(state: &DockState<Dock>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for surface in state.iter_surfaces() {
        let tree = match surface {
            Surface::Empty => {
                0u8.hash(&mut h);
                continue;
            }
            Surface::Main(tree) => {
                1u8.hash(&mut h);
                tree
            }
            Surface::Window(tree, window) => {
                2u8.hash(&mut h);
                let r = window.rect();
                if r.is_finite() {
                    for v in [r.min.x, r.min.y, r.width(), r.height()] {
                        (v.round() as i32).hash(&mut h);
                    }
                }
                tree
            }
        };
        for node in tree.iter() {
            match node {
                Node::Empty => 0u8.hash(&mut h),
                Node::Leaf { tabs, active, .. } => {
                    1u8.hash(&mut h);
                    tabs.hash(&mut h);
                    active.0.hash(&mut h);
                }
                Node::Horizontal { fraction, .. } | Node::Vertical { fraction, .. } => {
                    (2u8 + u8::from(matches!(node, Node::Vertical { .. }))).hash(&mut h);
                    ((fraction * 1000.0).round() as i32).hash(&mut h);
                }
            }
        }
    }
    h.finish()
}

/// The persisted content plus the focused leaf (reported by `observe`, not saved).
fn observed_key(state: &DockState<Dock>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fingerprint(state).hash(&mut h);
    state
        .focused_leaf()
        .map(|(surface, node)| (surface.0, node.0))
        .hash(&mut h);
    h.finish()
}

/// Bounds the user's named layouts (names and count); invalid entries are dropped.
pub(crate) fn sanitize_named(named: &mut std::collections::BTreeMap<String, Value>) {
    named.retain(|name, spec| {
        valid_name(name).is_ok() && !BUILTIN.contains(&name.as_str()) && spec.is_object()
    });
    while named.len() > MAX_NAMED {
        named.pop_last();
    }
}

/// Layout names: 1..=48 characters, printable, not only spaces.
pub(crate) fn valid_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() || name.chars().count() > 48 || name.chars().any(char::is_control) {
        return Err("layout names are 1..48 printable characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_round_trip_through_the_dock_state() {
        for name in BUILTIN {
            let spec = builtin(name).unwrap();
            let (state, dropped) = from_spec(&spec).unwrap();
            assert!(dropped.is_empty());
            // Saved form = the built-in plus explicit `active` indices; loading it is lossless.
            let saved = to_spec(&state);
            assert_eq!(
                saved["main"]["fraction"], spec["main"]["fraction"],
                "{name}"
            );
            assert_eq!(to_spec(&from_spec(&saved).unwrap().0), saved, "{name}");
            for tab in Dock::ALL {
                // World floats and the UI Editor opens beside Scene on demand; Default and Tall
                // Inspector keep only Assets, Console, Problems and History in the bottom
                // strip (the other tool tabs open from the Window menu); Classic has them all.
                let on_demand = matches!(tab, Dock::World | Dock::UiEditor)
                    || tab.tuning()
                    || (name != "Classic"
                        && matches!(
                            tab,
                            Dock::Changes
                                | Dock::Proposals
                                | Dock::Scenarios
                                | Dock::Agent
                                | Dock::Profiler
                                | Dock::Trace
                        ));
                assert_eq!(is_open(&state, tab), !on_demand, "{name} {tab:?}");
            }
        }
    }

    #[test]
    fn unknown_repeated_and_empty_entries_are_dropped_and_bad_specs_rejected() {
        let spec = json!({"v":1,"main":{"split":"h","fraction":7.0,
            "a":{"tabs":["terminal","scene","scene"],"active":1},
            "b":{"split":"v","fraction":null,"a":{"tabs":["nope"]},"b":{"tabs":["inspector"]}}},
            "windows":[{"rect":[1,2,"x",4],"node":{"tabs":["world"]}}]});
        let (state, dropped) = from_spec(&spec).unwrap();
        assert_eq!(dropped, ["terminal", "scene", "nope"]);
        // The empty leaf collapsed its split; the fraction was clamped.
        assert_eq!(
            to_spec(&state)["main"],
            json!({"split":"h","fraction":0.95,"a":{"tabs":["scene"],"active":0},"b":{"tabs":["inspector"],"active":0}})
        );
        assert!(
            is_open(&state, Dock::World),
            "a window with a bad rect uses the default one"
        );
        for bad in [
            json!(null),
            json!({"v":2,"main":null}),
            json!({"v":1,"main":{"split":"x","a":{"tabs":[]},"b":{"tabs":[]}}}),
            json!({"v":1,"main":{"tabs":"scene"}}),
            json!({"v":1,"main":{"tabs":[1]}}),
            json!({"v":1,"windows":{}}),
        ] {
            assert!(from_spec(&bad).is_err(), "{bad}");
        }
        let mut deep = json!({"tabs":["scene"]});
        for _ in 0..20 {
            deep = json!({"split":"h","a":deep,"b":{"tabs":[]}});
        }
        assert!(from_spec(&json!({"v":1,"main":deep})).is_err());
        // Everything closed is a valid layout.
        let (empty, _) = from_spec(&json!({"v":1,"main":null})).unwrap();
        assert!(empty.main_surface().is_empty());
        assert_eq!(to_spec(&empty), json!({"v":1,"main":null,"windows":[]}));
    }

    #[test]
    fn closed_tabs_reopen_near_their_default_place() {
        let mut state = default_state();
        for tab in [Dock::History, Dock::Hierarchy, Dock::Inspector, Dock::Scene] {
            assert!(close(&mut state, tab));
            assert!(!close(&mut state, tab));
            assert!(!is_open(&state, tab));
            show(&mut state, tab);
            assert!(
                is_active(&state, tab) && focused(&state) == Some(tab),
                "{tab:?}"
            );
        }
        // History rejoined the other bottom tabs; Changes (closed by default) opens there too,
        // and Graphs opens beside Scene.
        let (s, n, _) = state.find_tab(&Dock::History).unwrap();
        assert_eq!(
            state.find_tab(&Dock::Assets).map(|f| (f.0, f.1)),
            Some((s, n))
        );
        show(&mut state, Dock::Changes);
        assert_eq!(
            state.find_tab(&Dock::Changes).map(|f| (f.0, f.1)),
            Some((s, n))
        );
        assert!(close(&mut state, Dock::Graphs));
        show(&mut state, Dock::Graphs);
        let scene = state.find_tab(&Dock::Scene).map(|f| (f.0, f.1));
        assert_eq!(state.find_tab(&Dock::Graphs).map(|f| (f.0, f.1)), scene);
        // World opens as a floating window; closing it removes the window.
        show(&mut state, Dock::World);
        assert!(!state.find_tab(&Dock::World).unwrap().0.is_main());
        assert!(close(&mut state, Dock::World));
        assert_eq!(to_spec(&state)["windows"], json!([]));
        // The UI Editor joins Scene's leaf as the active tab.
        show(&mut state, Dock::UiEditor);
        let (s, n, _) = state.find_tab(&Dock::UiEditor).unwrap();
        assert_eq!(
            state.find_tab(&Dock::Scene).map(|f| (f.0, f.1)),
            Some((s, n))
        );
        assert!(is_active(&state, Dock::UiEditor) && !is_active(&state, Dock::Scene));
        assert!(close(&mut state, Dock::UiEditor));
        // From an empty main surface every tab comes back.
        let (mut empty, _) = from_spec(&json!({"v":1,"main":null})).unwrap();
        for tab in Dock::ALL {
            show(&mut empty, tab);
        }
        assert!(Dock::ALL.iter().all(|t| is_open(&empty, *t)));
        // Fingerprint follows the persisted content.
        let a = fingerprint(&state);
        assert_eq!(a, fingerprint(&state));
        show(&mut state, Dock::Problems);
        assert_ne!(a, fingerprint(&state));
    }
}
