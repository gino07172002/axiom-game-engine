//! Game tab, Play controls and the Inspector camera preview driven by real egui input: views
//! composed into the tab (or skipped when hidden), aspect letterboxing, the view-only camera
//! choice, input capture / release, the read-only Inspector during Play and the preview hole.
use super::game::{compose, fit, Aspect};
use super::{layout, Action, Dock, Editor};
use crate::world::WorldRuntime;
use axiom_core::{Request, Store, World};
use egui::{epaint::Shape, pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use serde_json::json;

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

fn two_cameras() -> World {
    serde_json::from_str(include_str!("../../../../content/two-cameras/world.json")).unwrap()
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
    screen: Rect,
    time: f64,
}

impl Harness {
    fn new(world: World) -> Self {
        let registry = crate::installed_registry();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_component_hints(&registry.authoring);
        Self {
            ctx: egui::Context::default(),
            runtime: WorldRuntime::new(world, 1, registry).unwrap(),
            editor,
            screen: Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0)),
            time: 0.0,
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> (Vec<Action>, egui::FullOutput) {
        self.time += 1.0 / 60.0;
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        // Shortcuts read the frame's modifier state, not the key event's.
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(self.screen),
                time: Some(self.time),
                modifiers,
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        (actions, out)
    }
    /// Two frames (egui lays new windows / tabs out invisibly on their first frame).
    fn settle(&mut self) -> egui::FullOutput {
        self.frame(vec![]);
        self.frame(vec![]).1
    }
    fn render(&mut self) -> crate::presentation::RenderFrame {
        let screen = self.screen;
        self.editor.render_frame(&mut self.runtime, screen)
    }
    fn click(&mut self, at: Pos2) -> Vec<Action> {
        let mut actions = self.frame(vec![Event::PointerMoved(at), press(at, true)]).0;
        actions.extend(self.frame(vec![press(at, false)]).0);
        actions
    }
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
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

fn drawn(out: &egui::FullOutput, needle: &str) -> bool {
    out.shapes.iter().any(|c| match &c.shape {
        Shape::Text(t) => t.galley.text().contains(needle),
        _ => false,
    })
}

/// `viewport` (fractions of `screen`) as a rectangle in points.
fn rect_of(viewport: [f32; 4], screen: Rect) -> Rect {
    Rect::from_min_size(
        pos2(
            screen.min.x + viewport[0] * screen.width(),
            screen.min.y + viewport[1] * screen.height(),
        ),
        vec2(viewport[2] * screen.width(), viewport[3] * screen.height()),
    )
}

fn close(a: Rect, b: Rect) -> bool {
    (a.min - b.min).length() < 0.01 && (a.max - b.max).length() < 0.01
}

#[test]
fn aspect_presets_letterbox_and_viewports_compose_into_the_game_area() {
    let body = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 300.0));
    assert_eq!(fit(body, Aspect::Free), body);
    for (aspect, ratio) in [
        (Aspect::Wide, 16.0 / 9.0),
        (Aspect::Standard, 4.0 / 3.0),
        (Aspect::Square, 1.0),
    ] {
        let r = fit(body, aspect);
        assert!((r.width() / r.height() - ratio).abs() < 1e-4, "{aspect:?}");
        // Wide body: full height, centred horizontally (pillarbox).
        assert_eq!(r.height(), 300.0);
        assert!((r.center() - body.center()).length() < 1e-3);
        assert!(body.contains_rect(r));
    }
    // Tall body: full width, centred vertically (letterbox).
    let tall = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 900.0));
    let r = fit(tall, Aspect::Wide);
    assert_eq!(r.width(), 400.0);
    assert!((r.height() - 225.0).abs() < 1e-3 && (r.center().y - 450.0).abs() < 1e-3);
    // Degenerate bodies are returned as they are (never NaN).
    let empty = Rect::from_min_size(pos2(5.0, 5.0), vec2(0.0, 0.0));
    assert_eq!(fit(empty, Aspect::Square), empty);
    for name in ["free", "16:9", "4:3", "1:1"] {
        assert_eq!(Aspect::from_name(name).unwrap().name(), name);
    }
    assert!(Aspect::from_name("5:4").is_none());
    // A right-half split-screen camera lands in the right half of the game area.
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0));
    let game = Rect::from_min_size(pos2(200.0, 100.0), vec2(400.0, 200.0));
    let v = compose([0.5, 0.0, 0.5, 1.0], game, screen);
    assert!(close(
        rect_of(v, screen),
        Rect::from_min_size(pos2(400.0, 100.0), vec2(200.0, 200.0))
    ));
    assert!(close(
        rect_of(compose([0.0, 0.0, 1.0, 1.0], game, screen), screen),
        game
    ));
}

#[test]
fn game_tab_composes_the_runtime_cameras_only_while_shown() {
    let mut h = Harness::new(two_cameras());
    h.settle();
    // Default layout: Scene and Game share a leaf, Scene in front -> only the editor view.
    assert!(layout::is_open(
        h.editor.layout.dock.as_ref().unwrap(),
        Dock::Game
    ));
    assert!(h.editor.scene_visible && !h.editor.game.visible);
    let frame = h.render();
    assert_eq!(frame.views.len(), 1);
    assert_eq!(frame.views[0].id, "editor.orbit");
    // Window ▸ Game (= editor_view dock:game): Game in front, the Scene is not rendered.
    h.editor.remote_view(&[], false, Some("game")).unwrap();
    h.settle();
    assert!(h.editor.game.visible && !h.editor.scene_visible);
    let game = h.editor.game.rect;
    assert!(game.width() > 300.0 && game.height() > 200.0, "{game:?}");
    let frame = h.render();
    assert!(frame.draw_world);
    let ids: Vec<&str> = frame.views.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["camera/perspective", "camera/orthographic"]);
    // Split screen: each camera keeps its half, inside the game area.
    let left = rect_of(frame.views[0].camera.viewport, h.screen);
    let right = rect_of(frame.views[1].camera.viewport, h.screen);
    assert!(close(
        left,
        Rect::from_min_max(game.min, pos2(game.center().x, game.max.y))
    ));
    assert!(close(
        right,
        Rect::from_min_max(pos2(game.center().x, game.min.y), game.max)
    ));
    let observed = h.editor.observation();
    assert_eq!(observed["game"]["visible"], true);
    assert_eq!(observed["game"]["views"], 2);
    assert_eq!(observed["play"]["state"], "stopped");
    // Hidden again (Scene in front): no game views, nothing composed.
    h.editor.remote_view(&[], false, Some("scene")).unwrap();
    h.settle();
    assert!(!h.editor.game.visible && h.editor.game.rect == Rect::NOTHING);
    assert_eq!(h.render().views.len(), 1);
    // Scene and Game both closed: no world pass at all.
    h.editor
        .run_command_with("window.close", Some("scene"))
        .unwrap();
    h.editor
        .run_command_with("window.close", Some("game"))
        .unwrap();
    h.settle();
    let frame = h.render();
    assert!(!frame.draw_world && frame.views.is_empty() && frame.instances.is_empty());
    // Reopened through its window command, next to nothing else: it renders again.
    h.editor.run_command("window.game").unwrap();
    h.settle();
    assert!(h.editor.game.visible);
    assert_eq!(h.render().views.len(), 2);
}

#[test]
fn during_play_the_views_show_the_games_own_visibility() {
    let mut h = Harness::new(two_cameras());
    h.editor.set_playing(true);
    h.settle();
    assert!(h.editor.game.visible);
    let all = h.render().instances.len();
    // Gameplay hides an object (e.g. a scene switch): the Game tab follows the game.
    let slot = h.runtime.state.compiled.indices["box"];
    h.runtime.state.compiled.enabled[slot] = false;
    let hidden = h.render().instances.len();
    assert!(hidden < all, "{hidden} < {all}");
    // Editing shows the authored visibility, as before.
    h.editor.set_playing(false);
    h.settle();
    assert_eq!(h.render().instances.len(), all);
}

#[test]
fn aspect_command_letterboxes_inside_the_tab_and_persists() {
    let dir = std::env::temp_dir().join(format!("axiom-game-aspect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("w.editor-prefs.json");
    let mut h = Harness::new(lab());
    h.editor.set_prefs_path(path.clone());
    h.editor.run_command("window.game").unwrap();
    h.settle();
    let free = h.editor.game.rect;
    for (name, ratio) in [("4:3", 4.0 / 3.0), ("16:9", 16.0 / 9.0), ("1:1", 1.0)] {
        assert!(h
            .editor
            .run_command_with("game.aspect", Some(name))
            .unwrap()
            .is_empty());
        h.settle();
        let r = h.editor.game.rect;
        assert!(
            (r.width() / r.height() - ratio).abs() < 0.01,
            "{name}: {r:?}"
        );
        assert!(free.expand(0.5).contains_rect(r), "{name}: inside the tab");
        // The composed camera fills exactly the letterboxed area.
        let frame = h.render();
        assert_eq!(frame.views.len(), 1);
        assert!(close(rect_of(frame.views[0].camera.viewport, h.screen), r));
    }
    assert!(h
        .editor
        .run_command_with("game.aspect", Some("5:4"))
        .is_err_and(|e| e.to_string().contains("free|16:9|4:3|1:1")));
    // Without an argument the menu entry points at the tab (no change).
    h.editor.run_command("game.aspect").unwrap();
    assert_eq!(h.editor.aspect(), Aspect::Square);
    h.frame(vec![]);
    let mut again = Harness::new(lab());
    again.editor.set_prefs_path(path.clone());
    assert_eq!(again.editor.aspect(), Aspect::Square, "persisted in prefs");
    // A bad stored value falls back to Free.
    std::fs::write(&path, r#"{"game_aspect":"wide"}"#).unwrap();
    let mut bad = Harness::new(lab());
    bad.editor.set_prefs_path(path.clone());
    assert_eq!(bad.editor.aspect(), Aspect::Free);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn camera_choice_is_view_only_and_the_budget_drops_what_does_not_fit() {
    let world = acl(&lab(), "create camera id=cam2 at=3,2,6");
    let mut h = Harness::new(world.clone());
    h.editor.run_command("window.game").unwrap();
    h.settle();
    assert!(h
        .editor
        .run_command_with("game.camera", Some("cam2"))
        .unwrap()
        .is_empty());
    assert_eq!(*h.editor.world, world, "view only");
    h.settle();
    let frame = h.render();
    let ids: Vec<&str> = frame.views.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["cam2"], "the inactive camera, alone");
    assert!(close(
        rect_of(frame.views[0].camera.viewport, h.screen),
        h.editor.game.rect
    ));
    assert_eq!(h.editor.observation()["game"]["camera"], "cam2");
    assert!(h
        .editor
        .run_command_with("game.camera", Some("stage/crate"))
        .is_err_and(|e| e.to_string().contains("render.camera")));
    h.editor
        .run_command_with("game.camera", Some("active"))
        .unwrap();
    h.settle();
    assert_eq!(h.render().views[0].id, "stage/camera");
    // Budget: the renderer draws at most MAX_CAMERAS views; extra Game views and the camera
    // preview are dropped and counted, never sent.
    let mut frame = h.render();
    let view = frame.views.pop().unwrap();
    frame
        .views
        .push(h.editor.orbit.view(h.editor.game.rect, h.screen));
    let many = vec![view; axiom_core::MAX_CAMERAS];
    h.editor.camera_preview.rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(160.0, 90.0));
    h.editor.camera_preview.camera = Some("cam2".into());
    let screen = h.screen;
    h.editor.compose_views(&mut frame, many, &h.runtime, screen);
    assert_eq!(frame.views.len(), axiom_core::MAX_CAMERAS);
    assert_eq!(
        h.editor.game.dropped.get(),
        2,
        "one game view and the preview"
    );
}

#[test]
fn play_keeps_the_editor_routes_input_on_capture_and_esc_releases() {
    let mut h = Harness::new(lab());
    h.settle();
    h.editor.set_playing(true);
    h.settle();
    // Play focuses the Game tab; the editor chrome (menus, Inspector, Hierarchy) stays.
    assert!(h.editor.game.visible);
    let out = h.frame(vec![]).1;
    assert!(drawn(&out, "Inspector") && drawn(&out, "Hierarchy") && drawn(&out, "Stop"));
    assert!(drawn(&out, "click game for input"));
    // Click in the game area: the host is asked to give the game the input.
    let center = h.editor.game.rect.center();
    h.frame(vec![Event::PointerMoved(center)]);
    assert!(h.editor.game.hovered);
    h.click(center);
    assert!(std::mem::take(&mut h.editor.game.capture_request));
    // While the game has the keyboard, editor shortcuts are ignored except Play's own.
    h.editor.game.input = true;
    let tool = h.editor.tool;
    let (actions, _) = h.frame(vec![key(Key::Q, Modifiers::NONE)]);
    assert!(
        actions.is_empty() && h.editor.tool == tool,
        "Q stays with the game"
    );
    let ctrl_alt = Modifiers::CTRL | Modifiers::COMMAND | Modifiers::ALT;
    let (actions, _) = h.frame(vec![key(Key::P, ctrl_alt)]);
    assert!(matches!(actions.as_slice(), [Action::PlayPause]));
    let (actions, _) = h.frame(vec![key(Key::Period, ctrl_alt)]);
    assert!(matches!(actions.as_slice(), [Action::PlayStep]));
    // Esc gives the keyboard back.
    h.frame(vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(std::mem::take(&mut h.editor.game.release_request));
    h.editor.game.input = false;
    let (actions, _) = h.frame(vec![key(Key::Q, Modifiers::NONE)]);
    assert!(actions.is_empty() && h.editor.tool == super::Tool::Select);
    // A click elsewhere in the editor (a Hierarchy row) takes the input back.
    h.editor.game.input = true;
    let row = h.editor.rects["row:stage/crate"].center();
    h.click(row);
    assert!(std::mem::take(&mut h.editor.game.release_request));
    // Hiding the Game tab while the game has input releases it too.
    h.editor.game.input = true;
    h.editor.remote_view(&[], false, Some("scene")).unwrap();
    h.settle();
    assert!(std::mem::take(&mut h.editor.game.release_request));
    // Stop brings back the Scene tab that Play put the Game tab in front of.
    let mut s = Harness::new(lab());
    s.settle();
    s.editor.set_playing(true);
    s.settle();
    assert!(s.editor.game.visible && !s.editor.scene_visible);
    s.editor.set_playing(false);
    s.settle();
    assert!(s.editor.scene_visible && !s.editor.game.visible);
    // Shift+F5 stops from the editor.
    h.editor.game.input = false;
    let (actions, _) = h.frame(vec![key(Key::F5, Modifiers::SHIFT)]);
    assert!(matches!(actions.as_slice(), [Action::PlayStop]));
}

#[test]
fn game_raw_input_routes_pointer_and_keyboard_separately() {
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(320.0, 180.0));
    let events = vec![
        Event::PointerMoved(pos2(150.0, 150.0)),
        press(pos2(150.0, 150.0), true),
        key(Key::W, Modifiers::NONE),
        Event::Text("w".into()),
        key(Key::Escape, Modifiers::NONE),
        Event::WindowFocused(true),
    ];
    let raw = egui::RawInput {
        events,
        modifiers: Modifiers::SHIFT,
        ..Default::default()
    };
    let kinds = |r: &egui::RawInput| -> Vec<&'static str> {
        r.events
            .iter()
            .map(|e| match e {
                Event::PointerMoved(_) => "move",
                Event::PointerButton { .. } => "button",
                Event::PointerGone => "gone",
                Event::Key { .. } => "key",
                Event::Text(_) => "text",
                Event::WindowFocused(_) => "focus",
                _ => "other",
            })
            .collect()
    };
    // Hovered, no keyboard: pointer only, modifiers cleared.
    let r = crate::app::game_raw_input(raw.clone(), rect, true, false, false);
    assert_eq!(r.screen_rect, Some(rect));
    assert_eq!(kinds(&r), ["move", "button", "focus"]);
    assert_eq!(r.modifiers, Modifiers::NONE);
    // Captured: everything but Esc.
    let r = crate::app::game_raw_input(raw.clone(), rect, true, true, false);
    assert_eq!(kinds(&r), ["move", "button", "key", "text", "focus"]);
    // Neither, just left: no input, the pointer is gone for the game's widgets.
    let r = crate::app::game_raw_input(raw, rect, false, false, true);
    assert_eq!(kinds(&r), ["focus", "gone"]);
}

#[test]
fn inspector_is_read_only_during_play_and_content_commands_say_why() {
    let mut h = Harness::new(lab());
    h.editor
        .run_command_with("layout.load", Some("Tall Inspector"))
        .unwrap();
    h.editor.select(Some("stage/crate".into()));
    h.editor.remote_open("component:render.mesh").unwrap();
    h.editor.set_playing(true);
    h.editor.remote_view(&[], false, Some("inspector")).unwrap();
    h.settle();
    let out = h.frame(vec![]).1;
    assert!(drawn(&out, "Live (read-only)") && drawn(&out, "Playing · read-only"));
    // Dragging an authored field or clicking a header button writes nothing.
    let key = h
        .editor
        .rects
        .keys()
        .find(|k| k.starts_with("form:/"))
        .cloned()
        .expect("render.mesh form drawn");
    let from = h.editor.rects[&key].center();
    let mut actions = h
        .frame(vec![Event::PointerMoved(from), press(from, true)])
        .0;
    for step in 1..=8 {
        let at = from + vec2(step as f32 * 6.0, 0.0);
        actions.extend(h.frame(vec![Event::PointerMoved(at)]).0);
    }
    actions.extend(h.frame(vec![press(from + vec2(48.0, 0.0), false)]).0);
    for _ in 0..2 {
        actions.extend(h.frame(vec![]).0);
    }
    assert!(actions.is_empty(), "no write during Play");
    assert!(!h.editor.dirty());
    for id in [
        "edit.duplicate",
        "object.cube",
        "edit.undo",
        "game.main_camera",
    ] {
        assert!(
            h.editor
                .command_blocked(id)
                .is_some_and(|r| r.contains("stop Play")),
            "{id}"
        );
        assert!(h
            .editor
            .run_command(id)
            .is_err_and(|e| e.to_string().contains("stop Play")));
    }
    // View commands keep working.
    assert!(h.editor.run_command("view.top").unwrap().is_empty());
    // The live panel shows what the host read from the runtime.
    h.editor.set_live(
        "stage/crate".into(),
        7,
        vec![("tween.inspect".into(), json!({"clip":"open","time":0.5}))],
    );
    // (Opening the component scrolled the Inspector; a newly shown object starts at the top.)
    h.editor.inspector_shown = None;
    let out = h.frame(vec![]).1;
    let texts: Vec<String> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    assert!(
        drawn(&out, "tween.inspect") && drawn(&out, "\"clip\":\"open\""),
        "{texts:?}"
    );
    // Stop: editable again.
    h.editor.set_playing(false);
    assert!(h.editor.command_blocked("object.cube").is_none());
}

#[test]
fn camera_preview_composes_one_view_only_while_shown() {
    let mut h = Harness::new(lab());
    h.editor
        .run_command_with("layout.load", Some("Tall Inspector"))
        .unwrap();
    h.editor.select(Some("stage/camera".into()));
    h.settle();
    let hole = h.editor.camera_preview.rect;
    assert!(hole.width() > 100.0 && hole.height() > 50.0, "{hole:?}");
    let frame = h.render();
    let ids: Vec<&str> = frame.views.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["editor.orbit", "editor.camera_preview"]);
    assert!(close(
        rect_of(frame.views[1].camera.viewport, h.screen),
        hole
    ));
    assert_eq!(
        h.editor.observation()["game"]["camera_preview"],
        "stage/camera"
    );
    // The docked Inspector paints its background around the hole, never over it.
    let out = h.frame(vec![]).1;
    let surface = super::ui::SURFACE;
    let mut fills = vec![];
    fn collect(shape: &Shape, out: &mut Vec<(Rect, egui::Color32)>) {
        match shape {
            Shape::Vec(list) => list.iter().for_each(|s| collect(s, out)),
            Shape::Rect(r) => out.push((r.rect, r.fill)),
            _ => {}
        }
    }
    for c in &out.shapes {
        collect(&c.shape, &mut fills);
    }
    assert!(fills
        .iter()
        .any(|(r, f)| *f == surface && r.max.y <= hole.min.y + 0.5));
    assert!(!fills
        .iter()
        .any(|(r, f)| *f == surface && r.contains(h.editor.camera_preview.rect.center())));
    // Partly scrolled / clipped out of the Inspector: no preview (it would be squashed).
    let tall = h.screen;
    h.screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 330.0));
    h.settle();
    assert_eq!(h.editor.camera_preview.rect, Rect::NOTHING);
    h.screen = tall;
    h.settle();
    assert!(h.editor.camera_preview.rect.is_positive());
    // Toggle off (command and the Inspector checkbox share view.camera_preview).
    let toggle = h.editor.rects["camera.preview.toggle"].center();
    h.click(toggle);
    assert!(!h.editor.prefs.camera_preview);
    h.settle();
    assert_eq!(h.editor.camera_preview.rect, Rect::NOTHING);
    assert_eq!(h.render().views.len(), 1);
    h.editor.run_command("view.camera_preview").unwrap();
    // Not a camera: no preview.
    h.editor.select(Some("stage/crate".into()));
    h.settle();
    assert_eq!(h.editor.camera_preview.rect, Rect::NOTHING);
    assert_eq!(h.render().views.len(), 1);
    // Scene hidden, preview visible: the preview alone is still rendered.
    h.editor.select(Some("stage/camera".into()));
    h.editor
        .run_command_with("window.close", Some("scene"))
        .unwrap();
    h.editor
        .run_command_with("window.close", Some("game"))
        .unwrap();
    h.settle();
    let frame = h.render();
    assert!(frame.draw_world);
    let ids: Vec<&str> = frame.views.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["editor.camera_preview"]);
}
