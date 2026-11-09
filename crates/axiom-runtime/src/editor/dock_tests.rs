//! Dock tabs driven by real egui input: drag a tab to split, close / reopen through commands
//! and bridge names, saved layouts and bad preferences, Scene visibility and render frames.
use super::{layout, Dock, Editor};
use crate::world::WorldRuntime;
use axiom_core::World;
use egui::{epaint::Shape, vec2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_dock::{Node, NodeIndex};
use serde_json::json;

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
    time: f64,
    screen: Rect,
}

impl Harness {
    fn new() -> Self {
        let world = lab();
        Self {
            ctx: egui::Context::default(),
            runtime: WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap(),
            editor: Editor::new(world, 1, None),
            time: 0.0,
            screen: Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0)),
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(self.screen),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| {
                editor.draw(ctx, runtime);
            },
        )
    }
    /// Centre of the drawn text that equals `needle` (a tab title).
    fn text(out: &egui::FullOutput, needle: &str) -> Pos2 {
        out.shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.text() == needle => {
                    Some(t.pos + t.galley.rect.center().to_vec2())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{needle} not drawn"))
    }
    fn button(&mut self, at: Pos2, pressed: bool) -> egui::FullOutput {
        self.frame(vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ])
    }
    /// Press at `from`, move there in steps, keep `hold` callback running while held.
    fn drag(&mut self, from: Pos2, to: Pos2, mut held: impl FnMut(&mut Self)) {
        self.frame(vec![Event::PointerMoved(from)]);
        self.button(from, true);
        for k in 1..=12 {
            let p = from + (to - from) * (k as f32 / 12.0);
            self.frame(vec![Event::PointerMoved(p)]);
        }
        for _ in 0..3 {
            self.frame(vec![Event::PointerMoved(to)]);
        }
        held(self);
        self.button(to, false);
        for _ in 0..2 {
            self.frame(vec![]);
        }
    }
    fn state(&self) -> &egui_dock::DockState<Dock> {
        self.editor.layout.dock.as_ref().unwrap()
    }
    /// (node index, full node rect) of the leaf holding `tab` on the main surface.
    fn leaf(&self, tab: Dock) -> (NodeIndex, Rect) {
        let (surface, node, _) = self.state().find_tab(&tab).expect("tab open");
        assert!(surface.is_main());
        (node, self.state()[surface][node].rect().unwrap())
    }
}

fn temp_prefs(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("axiom-dock-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("w.editor-prefs.json")
}

#[test]
fn dragging_a_tab_onto_the_scene_splits_it_and_saves_the_layout_on_release() {
    let path = temp_prefs("split");
    let mut h = Harness::new();
    h.editor.set_prefs_path(path.clone());
    h.frame(vec![]);
    let out = h.frame(vec![]);
    assert!(!path.exists(), "an untouched default layout writes nothing");
    let before = h.editor.viewport;
    assert!(
        before.width() > 600.0 && before.height() > 350.0,
        "{before:?}"
    );
    // Drop History on the right split button of the Scene leaf (egui_dock's icon overlay:
    // the centre button with four around it, `shortest_side + spacing` apart).
    let history = Harness::text(&out, "History");
    let (_, scene) = h.leaf(Dock::Scene);
    let area = scene.shrink(10.0);
    let side = ((area.width() - 20.0) / 3.0)
        .min((area.height() - 20.0) / 3.0)
        .min(100.0);
    let target = area.center() + vec2(side + 10.0, 0.0);
    h.drag(history, target, |_| {
        assert!(
            !path.exists(),
            "nothing is written while the button is held"
        );
    });
    let (scene_node, _) = h.leaf(Dock::Scene);
    let (history_node, history_rect) = h.leaf(Dock::History);
    let (changes_node, _) = h.leaf(Dock::Changes);
    assert_ne!(history_node, changes_node, "History left the bottom tabs");
    let parent = scene_node.parent().unwrap();
    assert_eq!(history_node.parent(), Some(parent));
    assert!(history_node.is_right() && scene_node.is_left());
    assert!(matches!(
        h.state().main_surface()[parent],
        Node::Horizontal { .. }
    ));
    // The scene view now covers the left part only, and History sits to its right.
    let after = h.editor.viewport;
    assert!(after.width() < before.width() * 0.7, "{after:?}");
    assert!(history_rect.min.x >= after.max.x - 1.0);
    assert!(h.editor.scene_visible);
    // Saved on release: a new editor with the same preferences shows the same layout.
    let saved: super::commands::Prefs =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.layout, layout::to_spec(h.state()));
    let mut again = Harness::new();
    again.editor.set_prefs_path(path.clone());
    assert_eq!(layout::to_spec(again.state()), saved.layout);
    // Dragging the separator: written once, after release.
    let fraction = |h: &Harness| match h.state().main_surface()[parent] {
        Node::Horizontal { fraction, .. } => fraction,
        _ => unreachable!(),
    };
    let rect = h.state().main_surface()[parent].rect().unwrap();
    let x = rect.min.x + rect.width() * fraction(&h);
    let grab = egui::pos2(x, rect.center().y);
    let text = std::fs::read_to_string(&path).unwrap();
    let old = fraction(&h);
    h.drag(grab, grab - vec2(120.0, 0.0), |h| {
        assert!(fraction(h) < old, "the split follows the pointer");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "no write while held"
        );
    });
    assert_ne!(
        std::fs::read_to_string(&path).unwrap(),
        text,
        "written after release"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn tabs_close_and_reopen_through_commands_and_bridge_names_and_the_scene_stops_rendering() {
    let mut h = Harness::new();
    h.frame(vec![]);
    let screen = h.screen;
    let frame = h.editor.render_frame(&mut h.runtime, screen);
    assert!(frame.draw_world && frame.views[0].id == "editor.orbit");
    // Close by name, reopen through its window command: back among the bottom tabs.
    h.editor
        .run_command_with("window.close", Some("history"))
        .unwrap();
    assert!(!layout::is_open(h.state(), Dock::History));
    assert!(h
        .editor
        .run_command_with("window.close", Some("history"))
        .is_err_and(|e| e.to_string().contains("not open")));
    h.editor.run_command("window.history").unwrap();
    assert!(h.editor.dock_shows(Dock::History));
    assert_eq!(h.leaf(Dock::History).0, h.leaf(Dock::Changes).0);
    // Without an argument the focused tab closes.
    h.editor.run_command("window.problems").unwrap();
    h.editor.run_command("window.close").unwrap();
    assert!(!layout::is_open(h.state(), Dock::Problems));
    // Closing the Scene: no editor view, nothing acts on its old rectangle. The Game tab that
    // shared its leaf comes to the front and shows only the game camera.
    h.editor
        .run_command_with("window.close", Some("scene"))
        .unwrap();
    h.frame(vec![]);
    assert!(!h.editor.scene_visible);
    assert_eq!(h.editor.viewport, Rect::NOTHING);
    let frame = h.editor.render_frame(&mut h.runtime, screen);
    assert!(frame.views.iter().all(|v| v.id != "editor.orbit"));
    // Game closed too: no view and no world pass.
    h.editor
        .run_command_with("window.close", Some("game"))
        .unwrap();
    h.frame(vec![]);
    let frame = h.editor.render_frame(&mut h.runtime, screen);
    assert!(!frame.draw_world && frame.views.is_empty() && frame.instances.is_empty());
    let observed = h.editor.observation();
    assert_eq!(observed["scene_visible"], false);
    assert!(!observed["tabs"].to_string().contains("\"scene\""));
    // Old and new dock names over the bridge: open/focus, unknown names list the choices.
    h.editor.remote_view(&[], false, Some("assets")).unwrap();
    assert!(h.editor.dock_shows(Dock::Assets));
    let error = h
        .editor
        .remote_view(&[], false, Some("console"))
        .unwrap_err();
    assert!(
        error.to_string().contains("hierarchy|scene|game|inspector"),
        "{error}"
    );
    h.editor.remote_view(&[], false, Some("scene")).unwrap();
    h.frame(vec![]);
    assert!(h.editor.scene_visible && h.editor.viewport.is_positive());
    let observed = h.editor.observation();
    assert!(observed["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["tab"] == "scene" && t["focused"] == true));
    let frame = h.editor.render_frame(&mut h.runtime, screen);
    assert!(frame.draw_world && frame.views.len() == 1);
    // A smaller window: the scene view follows its tab.
    let big = h.editor.viewport;
    h.screen = Rect::from_min_size(Pos2::ZERO, vec2(960.0, 600.0));
    h.frame(vec![]);
    h.frame(vec![]);
    let small = h.editor.viewport;
    assert!(small.width() < big.width() && small.height() < big.height());
    assert!(h.screen.contains_rect(small));
    // The Scene behind another tab in the same leaf is not rendered either.
    h.editor.prefs.layouts.insert(
        "Stack".into(),
        json!({"v":1,"main":{"split":"h","fraction":0.3,
            "a":{"tabs":["hierarchy"]},"b":{"tabs":["scene","inspector"],"active":1}}}),
    );
    h.editor
        .run_command_with("layout.load", Some("Stack"))
        .unwrap();
    h.frame(vec![]);
    assert!(!h.editor.scene_visible);
    assert!(!h.editor.render_frame(&mut h.runtime, h.screen).draw_world);
    // World keeps its toggle and opens floating.
    h.editor.run_command("window.world").unwrap();
    let (surface, _, _) = h.state().find_tab(&Dock::World).unwrap();
    assert!(!surface.is_main());
    h.frame(vec![]);
    h.editor.run_command("window.world").unwrap();
    assert!(!layout::is_open(h.state(), Dock::World));
}

#[test]
fn layouts_save_load_and_bad_or_old_preferences_fall_back_with_a_notice() {
    let path = temp_prefs("layouts");
    let mut h = Harness::new();
    h.editor.set_prefs_path(path.clone());
    h.editor
        .run_command_with("layout.load", Some("Tall Inspector"))
        .unwrap();
    h.editor
        .run_command_with("window.close", Some("graphs"))
        .unwrap();
    h.editor
        .run_command_with("layout.save_as", Some("Mine 中文"))
        .unwrap();
    h.frame(vec![]);
    assert!(path.exists());
    let tall_without_graphs = layout::to_spec(h.state());
    assert!(h
        .editor
        .run_command_with("layout.save_as", Some("Default"))
        .is_err());
    assert!(h
        .editor
        .run_command_with("layout.save_as", Some("  "))
        .is_err());
    assert!(h
        .editor
        .run_command_with("layout.load", Some("nope"))
        .is_err_and(|e| e.to_string().contains("no layout named nope")));
    assert!(h
        .editor
        .run_command_with("layout.reset", Some("x"))
        .is_err());
    // A second editor with the same file: same layout, same saved names.
    let mut b = Harness::new();
    b.editor.set_prefs_path(path.clone());
    assert_eq!(layout::to_spec(b.state()), tall_without_graphs);
    b.editor.run_command("layout.reset").unwrap();
    assert!(layout::is_open(b.state(), Dock::Graphs));
    assert_eq!(
        layout::to_spec(b.state()),
        layout::to_spec(&layout::default_state())
    );
    b.editor
        .run_command_with("layout.load", Some("Mine 中文"))
        .unwrap();
    assert_eq!(layout::to_spec(b.state()), tall_without_graphs);
    b.editor
        .run_command_with("layout.delete", Some("Mine 中文"))
        .unwrap();
    assert!(b
        .editor
        .run_command_with("layout.delete", Some("Default"))
        .is_err());
    b.frame(vec![]);
    let saved: super::commands::Prefs =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(saved.layouts.is_empty());
    // Bad layouts: Default plus a visible notice; the other preferences survive.
    for (layout_json, notice) in [
        (json!({"v":1,"main":{"split":"q"}}), "Saved layout ignored"),
        (json!("garbage"), "Saved layout ignored"),
        (json!({"v":7}), "Saved layout ignored"),
        (
            json!({"v":1,"main":{"tabs":["console","scene"]}}),
            "unknown panel(s) ignored: console",
        ),
    ] {
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"snap":true,"snap_move":0.5,"layout":layout_json})).unwrap(),
        )
        .unwrap();
        let mut c = Harness::new();
        c.editor.set_prefs_path(path.clone());
        assert!(
            c.editor.error && c.editor.status.contains(notice),
            "{}",
            c.editor.status
        );
        assert!(c.editor.prefs.snap && c.editor.prefs.snap_move == 0.5);
        c.frame(vec![]);
        assert!(layout::is_open(c.state(), Dock::Scene));
    }
    // Preferences from before the dock: only the active bottom tab. Default + that tab, and
    // the file is not rewritten just for loading it.
    std::fs::write(&path, r#"{"dock":"history"}"#).unwrap();
    let mut d = Harness::new();
    d.editor.set_prefs_path(path.clone());
    assert!(!d.editor.error);
    assert!(d.editor.dock_shows(Dock::History) && d.editor.dock_shows(Dock::Scene));
    d.frame(vec![]);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        r#"{"dock":"history"}"#
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn steady_state_frames_do_not_grow_or_resync_the_dock() {
    let mut h = Harness::new();
    for _ in 0..5 {
        h.frame(vec![]);
    }
    let syncs = h.editor.layout.syncs;
    let nodes = h.state().main_surface().len();
    let pending = h.editor.layout.pending.capacity();
    let print = layout::fingerprint(h.state());
    for _ in 0..200 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.layout.syncs, syncs, "no spec rebuild per frame");
    assert_eq!(h.state().main_surface().len(), nodes);
    assert_eq!(h.editor.layout.pending.capacity(), pending);
    assert_eq!(layout::fingerprint(h.state()), print);
    // The open-tab list for observe is cached, not rebuilt per call.
    let a = h.editor.observation()["tabs"].clone();
    assert_eq!(a.as_array().unwrap().len(), 12, "every tab but World");
    assert_eq!(h.editor.observation()["tabs"], a);
}
