//! M-UI chrome driven by real egui input: the top bar, the Hierarchy toolbar, card menus and
//! the status bar run the same registry commands (and produce the same writes) as the menus,
//! the palette and `editor_view {command}`.
use super::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::{Request, Store, World};
use egui::{epaint::Shape, vec2, Event, Modifiers, PointerButton, Pos2, Rect};

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
}

impl Harness {
    fn new() -> Self {
        let runtime = WorldRuntime::new(lab(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(lab(), 1, None);
        editor.set_component_hints(&crate::installed_registry().authoring);
        Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
        }
    }
    fn run(&mut self, events: Vec<Event>) -> (Vec<Action>, egui::FullOutput) {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1366.0, 768.0))),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        (actions, out)
    }
    fn frame(&mut self) -> egui::FullOutput {
        self.run(vec![]).1
    }
    fn click(&mut self, key: &str) -> Vec<Action> {
        let at = self
            .editor
            .rects
            .get(key)
            .unwrap_or_else(|| panic!("{key} not drawn"))
            .center();
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut actions = self.run(vec![Event::PointerMoved(at), press(true)]).0;
        actions.extend(self.run(vec![press(false)]).0);
        actions
    }
    /// Opens a drop-down, lets its popup settle on screen, then clicks `item`.
    fn pick(&mut self, menu: &str, item: &str) -> Vec<Action> {
        let mut actions = self.click(menu);
        self.frame();
        actions.extend(self.click(item));
        actions
    }
}

fn drawn(out: &egui::FullOutput, needle: &str) -> bool {
    fn find(shape: &Shape, needle: &str) -> bool {
        match shape {
            Shape::Text(t) => t.galley.text().contains(needle),
            Shape::Vec(list) => list.iter().any(|s| find(s, needle)),
            _ => false,
        }
    }
    out.shapes.iter().any(|c| find(&c.shape, needle))
}

/// The edits of the single write in `actions`.
fn edits(actions: &[Action]) -> String {
    match actions {
        [Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }] => serde_json::to_string(edits).unwrap(),
        other => panic!("expected one write, got {}", other.len()),
    }
}

#[test]
fn top_bar_runs_the_play_and_undo_commands() {
    let mut h = Harness::new();
    let out = h.frame();
    // Edit mode: no stray "Maximized" toggle; Run is the registry's play.start.
    assert!(!drawn(&out, "Maximized"));
    assert!(drawn(&out, "Run") && drawn(&out, "REV 1") && drawn(&out, "Agent API"));
    assert!(matches!(h.click("play:start").as_slice(), [Action::Play]));
    // The target drop-down flips the Play Maximized preference (play.maximize) and then shows
    // it next to Run.
    assert!(h.pick("play:target", "play:maximized:true").is_empty());
    assert!(h.editor.prefs.play_maximized);
    assert!(drawn(&h.frame(), "Maximized"));
    // Pause / Step / Stop are disabled outside Play (their commands say why).
    assert!(h.editor.command_blocked("play.stop").is_some());
    assert!(h.click("play:play.stop").is_empty());
    // Undo after a write: the icon sends the same restore as Edit > Undo.
    h.editor.undo.push(1);
    h.frame();
    let undo = h.click("top:edit.undo");
    assert!(matches!(
        undo.as_slice(),
        [Action::Restore {
            rev: 1,
            redo: false
        }]
    ));
    // Gear = Preferences; Agent API = the AI / Protocol tab.
    h.click("top:edit.preferences");
    assert!(h.editor.show_prefs);
    h.click("top:agent");
    assert!(h.editor.dock_shows(super::Dock::Agent));
}

#[test]
fn hierarchy_toolbar_runs_the_same_commands_as_the_menus() {
    let mut h = Harness::new();
    h.editor.select(Some("stage/crate".into()));
    h.frame();
    // Duplicate / Create Group icons = edit.duplicate / object.group.
    let via_icon = edits(&h.click("hierarchy:edit.duplicate"));
    let mut reference = Editor::new(lab(), 1, Some("stage/crate".into()));
    assert_eq!(
        via_icon,
        edits(&reference.run_command("edit.duplicate").unwrap())
    );
    let via_icon = edits(&h.click("hierarchy:object.group"));
    assert_eq!(
        via_icon,
        edits(&reference.run_command("object.group").unwrap())
    );
    // Add ▾ Cube = object.cube.
    let via_add = edits(&h.pick("hierarchy:add", "hierarchy:add:cube"));
    assert_eq!(
        via_add,
        edits(&reference.run_command("object.cube").unwrap())
    );
    // Rename icon starts the inline rename (edit.rename).
    h.click("hierarchy:edit.rename");
    assert!(h.editor.renaming.is_some());
    // Without a selection the selection commands are disabled.
    let mut h = Harness::new();
    h.frame();
    assert!(h.click("hierarchy:edit.delete").is_empty());
    assert!(h.click("hierarchy:object.group").is_empty());
}

#[test]
fn scene_toolbar_icons_run_the_tool_and_view_commands() {
    let mut h = Harness::new();
    h.frame();
    h.click("scene:tool.rotate");
    assert!(h.editor.tool == super::Tool::Rotate);
    h.click("scene:view.snap");
    assert!(h.editor.prefs.snap);
    let local = h.editor.prefs.local_space;
    h.click("scene:view.space");
    assert_eq!(h.editor.prefs.local_space, !local);
    // Fullscreen is the registry command (handled by the host window).
    assert!(matches!(
        h.click("scene:view.fullscreen").as_slice(),
        [Action::ToggleFullscreen]
    ));
}

#[test]
fn status_bar_shows_the_render_counters_and_switches_mode() {
    let mut h = Harness::new();
    h.editor.frame_stats = crate::render::FrameStats {
        draw_calls: 7,
        triangles: 1_234_567,
        ui_draw_calls: 3,
    };
    h.editor.gpu_alloc = Some(256 * 1024 * 1024);
    let out = h.frame();
    assert!(drawn(&out, "Draw 7 + 3 UI"));
    assert!(drawn(&out, "Tris 1,234,567"));
    assert!(drawn(&out, "GPU alloc 256 MiB"));
    assert!(drawn(&out, "Entities 6"));
    h.editor.gpu_alloc = None;
    assert!(drawn(&h.frame(), "VRAM –"));
    h.editor.vram = Some(crate::vram::VramReading::Bytes {
        local: 300 << 20,
        shared: 0,
        budget: 2 << 30,
    });
    assert!(drawn(&h.frame(), "VRAM 300 MiB"));
    // Play Mode = play.start; while editing, Edit Mode does nothing.
    assert!(h.click("status:play.stop").is_empty());
    assert!(matches!(
        h.click("status:play.start").as_slice(),
        [Action::Play]
    ));
}

#[test]
fn renderer_counters_add_up() {
    let mut s = crate::render::FrameStats::default();
    s.add(1, 1);
    s.add(2, 24);
    assert_eq!((s.draw_calls, s.triangles), (3, 25));
}

#[test]
fn group_command_commits_like_acl_through_the_toolbar() {
    let mut h = Harness::new();
    let mut store = Store::memory_with(crate::installed_registry().authoring, lab()).unwrap();
    h.editor.select(Some("stage/crate".into()));
    h.editor.extra.insert("stage/beacon".into());
    h.frame();
    let actions = h.click("hierarchy:object.group");
    let [Action::Write { request, .. }] = actions.as_slice() else {
        panic!("one write");
    };
    store.execute(request.clone()).unwrap();
    let mut acl = Store::memory_with(crate::installed_registry().authoring, lab()).unwrap();
    acl.execute(Request::Acl {
        text: "group entities[id=stage/crate,id=stage/beacon]".into(),
        preview: false,
        expected_rev: None,
        propose: false,
    })
    .unwrap();
    assert_eq!(store.current().unwrap().1, acl.current().unwrap().1);
}

#[test]
fn narrow_panels_keep_the_hierarchy_toolbar_and_transform_fields_on_screen() {
    // 1366×768 (the harness screen): the Hierarchy toolbar and the Inspector's X / Y / Z
    // fields stay inside their panels instead of running off the right edge.
    let mut h = Harness::new();
    h.editor.select(Some("stage/crate".into()));
    for _ in 0..3 {
        h.frame();
    }
    let rects = &h.editor.rects;
    let row = rects["row:stage/crate"];
    let more = rects["hierarchy:more"];
    assert!(more.max.x <= row.max.x + 1.0, "{more:?} vs row {row:?}");
    for cmd in [
        "edit.duplicate",
        "edit.delete",
        "edit.rename",
        "object.group",
        "edit.frame",
    ] {
        if let Some(r) = rects.get(&format!("hierarchy:{cmd}")) {
            assert!(r.max.x < more.min.x, "{cmd} {r:?} overlaps … {more:?}");
        }
    }
    for key in ["inspector:position.z", "inspector:scale.z"] {
        let z = rects[key];
        assert!(z.is_positive() && z.max.x <= 1366.0 - 4.0, "{key} {z:?}");
    }
    // A very narrow Hierarchy moves the selection commands into "…" (still reachable there).
    let mut narrow = Harness::new();
    narrow.editor.select(Some("stage/crate".into()));
    let (editor, runtime) = (&mut narrow.editor, &narrow.runtime);
    for _ in 0..2 {
        let _ = narrow.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 640.0))),
                ..Default::default()
            },
            |ctx| {
                editor.draw(ctx, runtime);
            },
        );
    }
    let rects = &narrow.editor.rects;
    let row = rects["row:stage/crate"];
    assert!(rects["hierarchy:more"].max.x <= row.max.x + 1.0);
    assert!(rects["hierarchy:add"].max.x <= row.max.x);
    let z = rects["inspector:position.z"];
    assert!(z.max.x <= 1024.0, "{z:?}");
}
