//! Graphs tab driven by real egui input on Axiom Town: states, transitions, node drags,
//! typed condition / action forms, the rule table, validation, Play highlight and the
//! Inspector ▸ Behavior dialogs. Every write is compared with the ACL an agent would send.
use super::graph::{Sel, View};
use super::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::{ContentPackage, Edit, Request, Store, World};
use egui::{vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use serde_json::json;

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &crate::installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn acl(world: &World, script: &str) -> World {
    let mut store =
        Store::memory_with(crate::installed_registry().authoring, world.clone()).unwrap();
    store
        .execute(Request::Acl {
            text: script.into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .unwrap_or_else(|e| panic!("acl {script:?}: {e:#}"));
    store.current().unwrap().1
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
    store: Store,
    /// The world the same steps produce through ACL alone.
    expected: World,
}

impl Harness {
    fn new(world: World) -> Self {
        Self::with_runtime(world.clone(), world)
    }
    /// `world` in the editor and store; the runtime is built from `running` (a world the
    /// runtime refuses still opens in the editor).
    fn with_runtime(world: World, running: World) -> Self {
        let registry = crate::installed_registry();
        let runtime = WorldRuntime::new(running, 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_component_hints(&registry.authoring);
        // Graphs and Inspector side by side, Problems and Hierarchy below: no tiny panes.
        editor.prefs.layouts.insert(
            "graph-test".into(),
            json!({"v":1,"main":{"split":"v","fraction":0.72,
                "a":{"split":"h","fraction":0.7,"a":{"tabs":["graphs"]},"b":{"tabs":["inspector"]}},
                "b":{"tabs":["problems","hierarchy"]}},"windows":[]}),
        );
        editor
            .run_command_with("layout.load", Some("graph-test"))
            .unwrap();
        Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
            store: Store::memory_with(registry.authoring, world.clone()).unwrap(),
            expected: world,
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(2000.0, 1300.0))),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        actions
    }
    fn rect(&self, key: &str) -> Rect {
        *self.editor.rects.get(key).unwrap_or_else(|| {
            let mut keys: Vec<&String> = self
                .editor
                .rects
                .keys()
                .filter(|k| !k.starts_with("graph:edge") && !k.starts_with("row:"))
                .collect();
            keys.sort();
            panic!("{key} not drawn; have {keys:?}")
        })
    }
    fn press(at: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }
    fn click_at(&mut self, at: Pos2, button: PointerButton) -> Vec<Action> {
        let mut actions = self.frame(vec![Event::PointerMoved(at), Self::press(at, button, true)]);
        actions.extend(self.frame(vec![Self::press(at, button, false)]));
        // Popups (menus, dropdowns) close and live edits settle on the next frame.
        actions.extend(self.frame(vec![]));
        actions
    }
    fn click(&mut self, key: &str) -> Vec<Action> {
        let at = self.rect(key).center();
        self.click_at(at, PointerButton::Primary)
    }
    /// Presses at `from`, moves by `delta` over `steps` frames (asserting nothing is written
    /// while held) and returns the release frame's actions.
    fn drag(&mut self, from: Pos2, to: Pos2, steps: usize) -> Vec<Action> {
        assert!(self
            .frame(vec![
                Event::PointerMoved(from),
                Self::press(from, PointerButton::Primary, true)
            ])
            .is_empty());
        for step in 1..=steps {
            let at = from + (to - from) * (step as f32 / steps as f32);
            let held = self.frame(vec![Event::PointerMoved(at)]);
            assert!(
                held.is_empty(),
                "a write while the drag is held (step {step})"
            );
        }
        self.frame(vec![Self::press(to, PointerButton::Primary, false)])
    }
    /// Commits the single write in `actions` like the host and checks the world equals the
    /// expected one after `script` runs through ACL on it.
    fn commit(&mut self, actions: Vec<Action>, script: &str) -> Vec<Edit> {
        let writes: Vec<&Action> = actions
            .iter()
            .filter(|a| matches!(a, Action::Write { .. }))
            .collect();
        let [Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }] = writes.as_slice()
        else {
            panic!(
                "expected exactly one write for `{script}`, got {} (status: {})",
                writes.len(),
                self.editor.status
            );
        };
        let edits = edits.clone();
        self.store
            .execute(Request::Apply {
                expected_rev: None,
                edits: edits.clone(),
            })
            .unwrap_or_else(|e| panic!("editor write for `{script}` failed: {e:#}"));
        let (rev, world) = self.store.current().unwrap();
        self.editor.update(rev, world, true);
        self.expected = acl(&self.expected, script);
        assert_eq!(
            self.store.current().unwrap().1,
            self.expected,
            "editor and `{script}` diverge"
        );
        self.frame(vec![]);
        edits
    }
    fn lumi(&self) -> serde_json::Value {
        self.store.current().unwrap().1.resources["fsm.machines"]["machines"]["lumi"].clone()
    }
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn graph_edits_through_egui_match_the_fsm_acl_verbs() {
    let mut h = Harness::new(town());
    h.editor
        .run_command_with("graph.open", Some("lumi"))
        .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.editor.graph.machine.as_deref(), Some("lumi"));
    // + State: a stop state named `state`.
    let actions = h.click("graph:graph.add_state");
    h.commit(actions, "fsm lumi state add state");
    // Dragging the node writes nothing while held and one position on release.
    let node = h.rect("graph:node:state");
    let from = node.center();
    let rev = h.editor.rev;
    let released = h.drag(from, from + vec2(60.0, 40.0), 8);
    let edits = h.commit(released, "fsm lumi node state 828,120");
    assert_eq!(edits.len(), 1, "one editor.layout write");
    assert_eq!(h.editor.rev, rev + 1);
    assert!(h.frame(vec![]).is_empty(), "no write after the drag");
    // Drag from stroll's port to the new state: a transition (waits 120 ticks).
    let port = h.rect("graph:port:stroll").center();
    let target = h.rect("graph:node:state").center();
    let linked = h.drag(port, target, 6);
    h.commit(linked, "fsm lumi transition add stroll state");
    assert_eq!(h.editor.graph.selected, Some(Sel::Transition(6)));
    // Condition kind via the typed form: in_state_ticks -> near (template values).
    h.frame(vec![]);
    h.click("graph:kind:when");
    let actions = h.click("graph:kind:when:near");
    h.commit(
        actions,
        r#"fsm lumi transition set 6 {"from":"stroll","to":"state","when":{"kind":"near","tag":"player","within":2.0}}"#,
    );
    // Drag the `within` field: one revision on release.
    let field = h.rect("form:/within").center();
    let released = h.drag(field, field + vec2(30.0, 0.0), 6);
    let within = {
        let mut store = Store::memory_with(
            crate::installed_registry().authoring,
            h.store.current().unwrap().1,
        )
        .unwrap();
        let Action::Write {
            request: Request::Apply { edits, .. },
            ..
        } = &released[0]
        else {
            panic!("expected the field write");
        };
        store
            .execute(Request::Apply {
                expected_rev: None,
                edits: edits.clone(),
            })
            .unwrap();
        store.current().unwrap().1.resources["fsm.machines"]["machines"]["lumi"]["transitions"][6]
            ["when"]["within"]
            .clone()
    };
    assert_ne!(within, json!(2.0), "the drag changed the distance");
    h.commit(
        released,
        &format!(
            r#"fsm lumi transition set 6 {{"from":"stroll","to":"state","when":{{"kind":"near","tag":"player","within":{within}}}}}"#
        ),
    );
    // Rename the state from the side panel (initial, transitions and position follow).
    h.click("graph:node:state");
    h.click("graph:name");
    h.frame(vec![key(Key::A, Modifiers::COMMAND)]);
    h.frame(vec![Event::Text("rest".into())]);
    let actions = h.frame(vec![key(Key::Enter, Modifiers::NONE)]);
    h.commit(actions, "fsm lumi state rename state rest");
    assert_eq!(h.lumi()["transitions"][6]["to"], "rest");
    // Right-click a node: Set as initial state.
    let stroll = h.rect("graph:node:stroll").center();
    h.click_at(stroll, PointerButton::Secondary);
    let actions = h.click("graph:menu:Set as initial state");
    h.commit(actions, "fsm lumi initial stroll");
    // Select the transition by its label, Delete.
    h.click("graph:edge:6");
    assert_eq!(h.editor.graph.selected, Some(Sel::Transition(6)));
    let actions = h.click("graph:graph.delete");
    h.commit(actions, "fsm lumi transition remove 6");
    // Delete the state.
    h.click("graph:node:rest");
    let actions = h.click("graph:graph.delete");
    h.commit(actions, "fsm lumi state remove rest");
    // The model was rebuilt per revision, not per frame.
    let builds = h.editor.graph.builds;
    for _ in 0..30 {
        assert!(h.frame(vec![]).is_empty());
    }
    assert_eq!(h.editor.graph.builds, builds);
}

#[test]
fn state_action_form_and_double_click_add_match_acl() {
    let mut h = Harness::new(town());
    h.editor
        .run_command_with("graph.open", Some("lumi"))
        .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    // Double-click on empty canvas: state + its position in one revision.
    let canvas = h.rect("graph:canvas");
    let at = canvas.min + vec2(500.0, 300.0);
    h.frame(vec![
        Event::PointerMoved(at),
        Harness::press(at, PointerButton::Primary, true),
    ]);
    h.frame(vec![Harness::press(at, PointerButton::Primary, false)]);
    h.frame(vec![Harness::press(at, PointerButton::Primary, true)]);
    let actions = h.frame(vec![Harness::press(at, PointerButton::Primary, false)]);
    let edits = h.commit(
        actions,
        "fsm lumi state add state; fsm lumi node state 500,300",
    );
    assert_eq!(edits.len(), 2);
    // Its action kind: stop -> wander (template), via the typed form.
    h.click("graph:node:state");
    h.click("graph:kind:action");
    let actions = h.click("graph:kind:action:wander");
    h.commit(
        actions,
        r#"fsm lumi state set state {"action":{"kind":"wander","radius":3.0,"speed":1.0}}"#,
    );
}

#[test]
fn checks_show_on_nodes_and_in_problems_and_open_the_graph() {
    let mut world = town();
    {
        let m = &mut world.resources.get_mut("fsm.machines").unwrap()["machines"]["lumi"];
        m["states"]["lonely"] = json!({"action":{"kind":"stop"},"emit":"ghost"});
    }
    let mut h = Harness::with_runtime(world, town());
    h.frame(vec![]);
    h.frame(vec![]);
    let codes: Vec<&str> = h
        .editor
        .graph_problems_for_test()
        .iter()
        .map(|p| p.code)
        .collect();
    assert_eq!(codes, ["fsm_unreachable", "fsm_undeclared_event"]);
    // Problems lists them with Open graph; it opens lumi with the state selected.
    h.click("problem:graph:0");
    h.frame(vec![]);
    assert_eq!(h.editor.graph.machine.as_deref(), Some("lumi"));
    assert_eq!(h.editor.graph.selected, Some(Sel::State("lonely".into())));
    assert!(h.editor.rects.contains_key("graph:node:lonely"));
    assert_eq!(
        h.editor.observation()["graph"]["problems"],
        json!(2),
        "observe reports the check count"
    );
}

#[test]
fn play_highlights_each_agents_current_state() {
    let mut h = Harness::new(town());
    h.editor
        .run_command_with("graph.open", Some("lumi"))
        .unwrap();
    // Walk the hero to Lumi like town-dance does: she greets.
    let mut walk = crate::world::ActionMap::default();
    walk.0.insert("move_z".into(), 1.0);
    walk.0.insert("move_x".into(), 0.45);
    for _ in 0..250 {
        h.runtime.step(&walk);
    }
    let state = h
        .runtime
        .system::<crate::fsm::FsmSystem>()
        .unwrap()
        .agent_states()
        .find(|(id, _, _)| *id == "npc/lumi")
        .map(|(_, _, s)| s.to_owned())
        .unwrap();
    assert_eq!(state, "greet");
    // Edit mode: no live highlight.
    h.frame(vec![]);
    assert!(h.editor.graph_live_for_test().iter().all(|n| *n == 0));
    h.editor.previewing = true;
    h.frame(vec![]);
    let live = h.editor.graph_live_for_test();
    let greet = h.editor.graph_node_index_for_test("greet").unwrap();
    assert_eq!(live[greet], 1, "{live:?}");
    assert_eq!(live.iter().map(|n| *n as usize).sum::<usize>(), 1);
    // The Play preview overlay draws the same model read-only (no writes).
    let (editor, runtime) = (&mut h.editor, &h.runtime);
    let _ = h.ctx.run(egui::RawInput::default(), |ctx| {
        editor.play_graph_overlay(ctx, runtime)
    });
}

#[test]
fn rule_table_edits_match_array_and_set_acl() {
    let mut h = Harness::new(town());
    h.editor.remote_open("rules:").unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.editor.graph.view, View::Rules);
    let actions = h.click("graph:rule:add");
    h.commit(
        actions,
        r#"append resources[id=behavior.program] rules {"id":"rule","event":"dance_together","actions":[]}"#,
    );
    assert_eq!(h.editor.graph.rule_index, Some(8));
    // Event dropdown.
    h.click("graph:rule:event");
    let actions = h.click("graph:rule:event:eat");
    h.commit(actions, r#"set rules[id=rule] event "eat""#);
    // + Action ▸ add (variable defaults to the first declared one).
    h.click("graph:rule:add_action");
    let actions = h.click("graph:rule:action:add");
    h.commit(
        actions,
        r#"set rules[id=rule] actions [{"kind":"add","variable":"affection","value":{"kind":"literal","value":1}}]"#,
    );
    // Condition kind: always -> not (nested condition edited by its own dropdown).
    h.click("graph:kind:rule/when");
    let actions = h.click("graph:kind:rule/when:not");
    h.commit(
        actions,
        r#"set rules[id=rule] when {"kind":"not","condition":{"kind":"always"}}"#,
    );
    // Reorder and remove rows.
    let actions = h.click("graph:rule:up:8");
    h.commit(actions, "reorder resources[id=behavior.program] rules 8 7");
    let actions = h.click("graph:rule:remove:7");
    h.commit(actions, "remove resources[id=behavior.program] rules 7");
}

#[test]
fn behavior_menu_dialogs_write_what_their_acl_line_says() {
    // Lumi: remove her fsm.agent, then Inspector ▸ Behavior ▸ FSM Agent… ▸ lumi puts it back.
    let mut h = Harness::new(town());
    // The Behavior picker sits under the component list: give the Inspector full height.
    h.editor
        .run_command_with("layout.load", Some("Tall Inspector"))
        .unwrap();
    h.editor.select(Some("npc/lumi".into()));
    h.frame(vec![]);
    let actions = h
        .editor
        .remove_component("npc/lumi", "fsm.agent")
        .into_iter()
        .collect();
    h.commit(actions, "remove npc/lumi fsm.agent");
    h.frame(vec![]);
    h.click("behavior:menu");
    h.click("behavior:behavior.fsm_agent");
    h.frame(vec![]);
    let dialog = h.editor.options.as_ref().expect("FSM Agent dialog");
    assert_eq!(dialog.acl(&h.editor), "fsm npc/lumi agent lumi");
    let actions = h.click("options:create");
    h.commit(actions, "fsm npc/lumi agent lumi");
    assert_eq!(
        h.store.current().unwrap().1,
        town(),
        "back to the shipped town"
    );
    // Edit Graph on the re-added component opens lumi in the Graphs tab.
    h.frame(vec![]);
    h.editor.graph.machine = None;
    let actions = h.click("component:edit_graph");
    assert!(actions.is_empty());
    assert_eq!(h.editor.graph.machine.as_deref(), Some("lumi"));
    // Templates on Pix: each dialog's Create equals its ACL line.
    for (command, line) in [
        ("behavior.patrol", "behavior pc/pix patrol"),
        ("behavior.guard", "behavior pc/pix guard range=4"),
        ("behavior.dialogue", "behavior pc/pix dialogue"),
    ] {
        let mut h = Harness::new(town());
        h.editor.select(Some("pc/pix".into()));
        h.frame(vec![]);
        let mut actions = vec![];
        h.editor
            .invoke(super::commands::command(command).unwrap().id, &mut actions);
        assert!(actions.is_empty(), "the menu opens the dialog");
        if command == "behavior.guard" {
            h.editor
                .options
                .as_mut()
                .unwrap()
                .set("Chase range (m)", "4");
        }
        // A new window lays itself out on its first frame; click where it settled.
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.editor.options.as_ref().unwrap().acl(&h.editor), line);
        let actions = h.click("options:create");
        h.commit(actions, line);
        // The agent runs and its machine opened in the Graphs tab.
        assert!(h.editor.graph.machine.is_some());
    }
}

#[test]
fn a_graph_edit_keeps_town_dance_passing() {
    let scenario = || -> crate::scenario::Scenario {
        serde_json::from_str(include_str!(
            "../../../../examples/town-dance.scenario.json"
        ))
        .unwrap()
    };
    let before = crate::scenario::run(town(), 1, crate::installed_registry(), scenario()).unwrap();
    assert_eq!(before["ok"], true, "{before}");
    // Demo step 5: a new state and a transition into it, made in the graph.
    let mut h = Harness::new(town());
    h.editor
        .run_command_with("graph.open", Some("lumi"))
        .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    let actions = h.click("graph:graph.add_state");
    h.commit(actions, "fsm lumi state add state");
    let port = h.rect("graph:port:greet").center();
    let target = h.rect("graph:node:state").center();
    let linked = h.drag(port, target, 6);
    h.commit(linked, "fsm lumi transition add greet state");
    let world = h.store.current().unwrap().1;
    let after = crate::scenario::run(world, 1, crate::installed_registry(), scenario()).unwrap();
    assert_eq!(after["ok"], true, "{after}");
}

#[test]
fn a_revision_arriving_mid_drag_keeps_the_dragged_node() {
    let mut h = Harness::new(town());
    h.editor
        .run_command_with("graph.open", Some("lumi"))
        .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    let from = h.rect("graph:node:stroll").center();
    let at = h.editor.graph_node_at_for_test("stroll");
    h.frame(vec![
        Event::PointerMoved(from),
        Harness::press(from, PointerButton::Primary, true),
    ]);
    h.frame(vec![Event::PointerMoved(from + vec2(10.0, 0.0))]);
    // An agent adds a state that sorts before `stroll` (its node index shifts).
    let external = acl(&h.store.current().unwrap().1, "fsm lumi state add aaa");
    h.store = Store::memory_with(crate::installed_registry().authoring, external.clone()).unwrap();
    h.expected = external.clone();
    h.editor.update(2, external, false);
    let to = from + vec2(40.0, 20.0);
    h.frame(vec![Event::PointerMoved(to)]);
    let released = h.frame(vec![Harness::press(to, PointerButton::Primary, false)]);
    h.commit(
        released,
        &format!("fsm lumi node stroll {},{}", at.x + 40.0, at.y + 20.0),
    );
}

/// Zoom around the pointer (wheel), drags at 2× and ½× commit one canvas position each (the
/// same as ACL `fsm M node`), `graph.zoom` limits and arguments, Frame fits with zoom.
#[test]
fn zoomed_canvas_hit_tests_and_drags_in_canvas_units() {
    let mut h = Harness::new(town());
    h.editor
        .run_command_with("graph.open", Some("lumi"))
        .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    let actions = h.click("graph:graph.add_state");
    h.commit(actions, "fsm lumi state add state");
    assert_eq!(h.editor.graph_observation()["zoom"], 1.0);
    // Wheel over the new node: 2× around the pointer (the node stays under it).
    let node = h.rect("graph:node:state");
    let at = node.center();
    h.frame(vec![Event::PointerMoved(at)]);
    let wheel = |dy: f32| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        modifiers: Modifiers::NONE,
    };
    assert!(h.frame(vec![wheel(2.0f32.ln() / 0.003)]).is_empty());
    h.frame(vec![]);
    assert!((h.editor.graph.zoom() - 2.0).abs() < 1e-3);
    let zoomed = h.rect("graph:node:state");
    assert!((zoomed.width() - 2.0 * node.width()).abs() < 0.5);
    assert!(
        zoomed.center().distance(at) < 0.5,
        "zoomed around the pointer"
    );
    // A drag of (60, 40) points at 2× moves the state (30, 20) canvas units, once.
    let released = h.drag(at, at + vec2(60.0, 40.0), 8);
    let edits = h.commit(released, "fsm lumi node state 798,100");
    assert_eq!(edits.len(), 1);
    // ½× (command, around the canvas center): the same drag moves (120, 80).
    h.editor
        .run_command_with("graph.zoom", Some("50%"))
        .unwrap();
    h.frame(vec![]);
    let from = h.rect("graph:node:state").center();
    assert_eq!(h.editor.graph_observation()["zoom"], 0.5);
    let released = h.drag(from, from + vec2(60.0, 40.0), 8);
    h.commit(released, "fsm lumi node state 918,180");
    // Click on the node at ½× selects it (hit test in canvas units).
    h.click("graph:node:stroll");
    assert_eq!(h.editor.graph.selected, Some(Sel::State("stroll".into())));
    // Limits and arguments.
    for _ in 0..12 {
        h.frame(vec![wheel(-400.0)]);
    }
    assert_eq!(h.editor.graph.zoom(), super::graph::ZOOM_MIN);
    for arg in ["in", "in", "in", "in", "in", "in", "in", "in", "in", "in"] {
        h.editor.run_command_with("graph.zoom", Some(arg)).unwrap();
    }
    assert_eq!(h.editor.graph.zoom(), super::graph::ZOOM_MAX);
    assert!(h.editor.run_command_with("graph.zoom", Some("3")).is_err());
    assert!(h
        .editor
        .run_command_with("graph.zoom", Some("big"))
        .is_err());
    h.click("graph:zoom");
    assert_eq!(
        h.editor.graph.zoom(),
        1.0,
        "the zoom button resets to 100 %"
    );
    // Frame: every state inside the canvas, never magnified past 100 %.
    h.editor.run_command("graph.frame").unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    let canvas = h.rect("graph:canvas");
    assert!(h.editor.graph.zoom() <= 1.0);
    let nodes: Vec<_> = h
        .editor
        .rects
        .iter()
        .filter(|(k, _)| k.starts_with("graph:node:"))
        .map(|(k, r)| (k.clone(), *r))
        .collect();
    assert!(nodes.len() >= 4);
    for (key, r) in nodes {
        assert!(canvas.contains_rect(r), "{key} {r:?} outside {canvas:?}");
    }
}
