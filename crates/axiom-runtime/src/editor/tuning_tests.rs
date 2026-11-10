//! Tuning editors driven by real egui input: Input Map key capture and conflicts,
//! Localization grid editing / warnings / filter, Animation scrub preview and ability event
//! markers, VFX size curve and gradient, and the material shader source check. Every write is
//! compared with the ACL line an agent would send, and drags write once on release.
use super::{Action, Dock, Editor, Tool};
use crate::world::{RuntimeSystem, WorldRuntime};
use axiom_core::{Request, Store, World};
use egui::{vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};

fn package(name: &str) -> World {
    axiom_core::ContentPackage::load(
        format!("{}/../../content/{name}", env!("CARGO_MANIFEST_DIR")),
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

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn key(k: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key: k,
        physical_key: Some(k),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
    store: Store,
    time: f64,
    screen: egui::Vec2,
}

impl Harness {
    fn new(world: World, tab: &str) -> Self {
        Self::sized(world, tab, vec2(1700.0, 1100.0))
    }
    /// A taller screen gives the bottom tab strip room for long tables.
    fn sized(world: World, tab: &str, screen: egui::Vec2) -> Self {
        let registry = crate::installed_registry();
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_component_hints(&registry.authoring);
        editor.run_command(tab).unwrap();
        let mut h = Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
            store: Store::memory_with(registry.authoring, world).unwrap(),
            time: 0.0,
            screen,
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }
    fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        self.editor.poll_jobs();
        self.time += 1.0 / 60.0;
        actions
    }
    #[track_caller]
    fn rect(&self, key: &str) -> Rect {
        *self.editor.rects.get(key).unwrap_or_else(|| {
            let mut keys: Vec<_> = self.editor.rects.keys().collect();
            keys.sort();
            panic!("{key} not drawn; have {keys:?}")
        })
    }
    /// Hover first (a person moves there before pressing), then press and release.
    fn click_at(&mut self, at: Pos2) -> Vec<Action> {
        let mut actions = self.frame(vec![Event::PointerMoved(at)]);
        actions.extend(self.frame(vec![press(at, true)]));
        actions.extend(self.frame(vec![press(at, false)]));
        actions
    }
    #[track_caller]
    fn click(&mut self, key: &str) -> Vec<Action> {
        let at = self.rect(key).center();
        self.click_at(at)
    }
    /// Drags in `steps` frames; nothing may be written before the release.
    #[track_caller]
    fn drag(&mut self, from: Pos2, to: Pos2, steps: usize) -> Vec<Action> {
        assert!(self
            .frame(vec![Event::PointerMoved(from), press(from, true)])
            .is_empty());
        for step in 1..=steps {
            let at = from + (to - from) * (step as f32 / steps as f32);
            assert!(
                self.frame(vec![Event::PointerMoved(at)]).is_empty(),
                "step {step}"
            );
        }
        self.frame(vec![press(to, false)])
    }
    /// Commits exactly one write and syncs the editor; returns the new world.
    #[track_caller]
    fn commit(&mut self, actions: Vec<Action>) -> World {
        let actions = super::coalesce(actions);
        let [Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }] = actions.as_slice()
        else {
            panic!(
                "expected exactly one write, got {}; status {:?}, problems {:?}",
                actions.len(),
                self.editor.status,
                self.editor.problems.first()
            );
        };
        let previous = self.editor.rev;
        self.store
            .execute(Request::Apply {
                expected_rev: None,
                edits: edits.clone(),
            })
            .unwrap();
        let (rev, world) = self.store.current().unwrap();
        self.editor.update(rev, world, true);
        self.editor.record_apply(previous);
        self.frame(vec![]);
        self.frame(vec![]);
        self.world()
    }
    fn world(&self) -> World {
        self.store.current().unwrap().1
    }
}

#[test]
fn input_map_captures_keys_flags_conflicts_and_matches_acl() {
    let start = package("axiom-town");
    let mut h = Harness::sized(start.clone(), "window.input_map", vec2(1700.0, 2400.0));
    assert!(h.editor.dock_shows(Dock::InputMap));
    assert!(!h.editor.rects.contains_key("input:conflicts"));
    // Click attack's key, then press W: the key is not the Move tool's shortcut now.
    assert!(h.click("input:key:attack:button").is_empty());
    assert!(h.editor.tuning.input.capture.is_some());
    let tool = h.editor.tool;
    let actions = h.frame(vec![key(Key::W, Modifiers::NONE)]);
    assert!(
        h.editor.tool == tool,
        "W switched the tool during a capture"
    );
    let after = h.commit(actions);
    assert_eq!(after, acl(&start, "input button attack KeyW"));
    // Bind it to E (interact's key): one conflict, shown in the header.
    h.click("input:key:attack:button");
    let actions = h.frame(vec![key(Key::E, Modifiers::NONE)]);
    let conflicted = h.commit(actions);
    assert_eq!(conflicted, acl(&after, "input button attack KeyE"));
    assert!(h.editor.rects.contains_key("input:conflicts"));
    assert_eq!(h.editor.tuning_observation()["input"]["conflicts"], 1);
    // Esc cancels without writing; shortcuts work again.
    h.click("input:key:attack:button");
    assert!(h.frame(vec![key(Key::Escape, Modifiers::NONE)]).is_empty());
    assert!(h.editor.tuning.input.capture.is_none());
    h.frame(vec![key(Key::E, Modifiers::NONE)]);
    assert!(
        h.editor.tool == Tool::Rotate,
        "E is Rotate again after the capture"
    );
    // An axis half: orbit's negative key -> Shift (a modifier-only press).
    h.click("input:key:orbit:negative");
    let shift = Modifiers {
        shift: true,
        ..Modifiers::NONE
    };
    let actions = h.ctx_frame_with_modifiers(shift);
    let shifted = h.commit(actions);
    assert_eq!(shifted, acl(&conflicted, "input axis orbit KeyC ShiftLeft"));
    // New button: type the name, + Button, press T.
    h.click("input:new_action");
    h.frame(vec![Event::Text("taunt".into())]);
    // egui hit-tests against last frame's widgets: the Add button was still disabled.
    h.frame(vec![]);
    assert!(h.click("input:add_button").is_empty());
    let actions = h.frame(vec![key(Key::T, Modifiers::NONE)]);
    let taunt = h.commit(actions);
    assert_eq!(taunt, acl(&shifted, "input button taunt KeyT"));
    // New axis: + Axis, then the positive and the negative key (one revision).
    h.click("input:new_action");
    h.frame(vec![Event::Text("lean".into())]);
    // egui hit-tests against last frame's widgets: the Add button was still disabled.
    h.frame(vec![]);
    assert!(h.click("input:add_axis").is_empty());
    assert!(h.frame(vec![key(Key::Q, Modifiers::NONE)]).is_empty());
    let actions = h.frame(vec![key(Key::Num3, Modifiers::NONE)]);
    let lean = h.commit(actions);
    assert_eq!(lean, acl(&taunt, "input axis lean KeyQ Digit3"));
    // Mouse Left for a button; remove an action.
    h.click("input:key:taunt:button");
    h.frame(vec![]);
    let actions = h.click("input:mouse_left");
    let mouse = h.commit(actions);
    assert_eq!(mouse, acl(&lean, "input button taunt MouseLeft"));
    let actions = h.click("input:remove:taunt");
    assert_eq!(h.commit(actions), acl(&mouse, "input remove taunt"));
    // A capture ends when the person starts typing into a text field elsewhere (here the
    // Hierarchy search box, left of its filter icon); the typed text is not a binding.
    h.click("input:key:attack:button");
    assert!(h.editor.tuning.input.capture.is_some());
    let search = h.rect("hierarchy:filter").left_center() - vec2(80.0, 0.0);
    h.click_at(search);
    assert!(h.frame(vec![Event::Text("x".into())]).is_empty());
    assert!(h.editor.tuning.input.capture.is_none());
    // The table model is rebuilt per revision, not per frame.
    let builds = h.editor.tuning.input.builds;
    for _ in 0..10 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.tuning.input.builds, builds);
}

impl Harness {
    /// One frame where only the modifiers change (how egui reports Shift / Ctrl / Alt).
    fn ctx_frame_with_modifiers(&mut self, modifiers: Modifiers) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
                time: Some(self.time),
                modifiers,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        self.time += 1.0 / 60.0;
        actions
    }
}

#[test]
fn input_map_create_writes_like_acl() {
    let start = package("shader-lab");
    assert!(!start.resources.contains_key("input.bindings"));
    let mut h = Harness::new(start.clone(), "window.input_map");
    let actions = h.click("input:create");
    assert_eq!(h.commit(actions), acl(&start, "asset create input"));
}

#[test]
fn localization_grid_edits_once_and_flags_missing_text() {
    let start = package("axiom-town");
    let mut h = Harness::new(start.clone(), "window.localization");
    let keys = start.resources["ui.document"]["catalogs"]["en"]
        .as_object()
        .unwrap()
        .len();
    assert_eq!(h.editor.tuning_observation()["localization"]["rows"], keys);
    // Edit en/town (filtered to it: the grid is a virtual list): Ctrl+A, type, Enter = one
    // revision (nothing while typing).
    h.editor
        .run_command_with("loc.filter", Some("town"))
        .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.click("loc:cell:en:town").is_empty());
    h.frame(vec![]);
    assert!(h.frame(vec![key(Key::A, Modifiers::COMMAND)]).is_empty());
    assert!(h
        .frame(vec![Event::Text("Central Plaza".into())])
        .is_empty());
    let actions = h.frame(vec![key(Key::Enter, Modifiers::NONE)]);
    let plaza = h.commit(actions);
    assert_eq!(plaza, acl(&start, r#"loc set en town "Central Plaza""#));
    // Esc cancels an edit.
    h.click("loc:cell:en:town");
    h.frame(vec![]);
    h.frame(vec![Event::Text("zzz".into())]);
    assert!(h.frame(vec![key(Key::Escape, Modifiers::NONE)]).is_empty());
    assert!(h.editor.tuning.loc.editing.is_none());
    h.editor.run_command("loc.filter").unwrap();
    // + Key: empty everywhere -> missing in every locale (chip with the count).
    h.click("loc:new_key");
    h.frame(vec![Event::Text("greeting".into())]);
    // egui hit-tests against last frame's widgets: the Add button was still disabled.
    h.frame(vec![]);
    let actions = h.click("loc:add_key");
    let added = h.commit(actions);
    assert_eq!(added, acl(&plaza, "loc key add greeting"));
    let counts = &h.editor.tuning_observation()["localization"]["locales"];
    assert_eq!(counts["en"]["missing"], 1, "{counts}");
    assert_eq!(counts["zh-TW"]["missing"], 1, "{counts}");
    assert!(h.editor.rects.contains_key("loc:missing:en"));
    // Missing only: just the new key; the filter narrows by key or text.
    h.click("loc:missing_only");
    h.frame(vec![]);
    assert_eq!(h.editor.tuning_observation()["localization"]["rows"], 1);
    h.click("loc:missing_only");
    h.click("loc:filter");
    h.frame(vec![Event::Text("lumi".into())]);
    h.frame(vec![]);
    let rows = h.editor.tuning_observation()["localization"]["rows"]
        .as_u64()
        .unwrap();
    assert!(rows > 0 && rows < keys as u64, "{rows} of {keys}");
    // Filtered rows are cached: typing nothing more rebuilds nothing.
    let (builds, filters) = (h.editor.tuning.loc.builds, h.editor.tuning.loc.filters);
    for _ in 0..10 {
        h.frame(vec![]);
    }
    assert_eq!(
        (h.editor.tuning.loc.builds, h.editor.tuning.loc.filters),
        (builds, filters)
    );
    // + Locale (empty) and its removal.
    h.click("loc:new_locale");
    h.frame(vec![Event::Text("ja".into())]);
    // egui hit-tests against last frame's widgets: the Add button was still disabled.
    h.frame(vec![]);
    let actions = h.click("loc:add_locale");
    let ja = h.commit(actions);
    assert_eq!(ja, acl(&added, "loc locale add ja"));
    let actions = h.click("loc:remove_locale:ja");
    assert_eq!(h.commit(actions), acl(&ja, "loc locale remove ja"));
}

#[test]
fn animation_scrub_previews_without_writing_and_event_markers_match_acl() {
    let start = package("axiom-town");
    let mut h = Harness::new(start.clone(), "window.animation");
    h.editor
        .run_command_with("anim.target", Some("hero wave"))
        .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.editor.tuning.anim.clip.as_deref(), Some("wave"));
    // Scrub: a click half way on the clip row moves the playhead and turns preview on.
    let strip = h.rect("anim:strip");
    let at = Pos2::new(
        strip.left() + 112.0 + (strip.width() - 112.0) * 0.5,
        strip.top() + 18.0 + 13.0,
    );
    assert!(h.click_at(at).is_empty());
    let length = h.runtime.state.animation_cues.catalog["hero"]["wave"];
    let t = h.editor.tuning.anim.time;
    assert!((t / length - 0.5).abs() < 0.03, "{t} of {length}");
    assert!(h.editor.anim_scrub_target().is_some());
    // The scene frame shows the scrubbed pose; the simulation keeps its own.
    let before = h
        .runtime
        .system::<crate::animation::AnimationSystem>()
        .unwrap()
        .metrics();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1700.0, 1100.0));
    let posed = h.editor.render_frame(&mut h.runtime, screen);
    let after = h
        .runtime
        .system::<crate::animation::AnimationSystem>()
        .unwrap()
        .metrics();
    assert_eq!(before, after, "the scrub leaked into the simulation");
    h.editor.tuning.anim.preview = false;
    let rest = h.editor.render_frame(&mut h.runtime, screen);
    let vertices = |f: &crate::presentation::RenderFrame| {
        f.meshes
            .iter()
            .find(|m| m.id == "hero")
            .map(|m| m.vertices.iter().map(|v| v.position).collect::<Vec<_>>())
            .unwrap()
    };
    assert_ne!(
        vertices(&posed),
        vertices(&rest),
        "the scrub did not pose the model"
    );
    h.editor.tuning.anim.preview = true;
    assert_eq!(h.world(), start, "scrubbing wrote");
    // Drag the existing event marker (wave: tick 10) to the right: one write on release.
    let marker = h.rect("anim:event:wave:0").center();
    let actions = h.drag(marker, marker + vec2(120.0, 0.0), 8);
    let moved = h.commit(actions);
    let tick = moved.resources["action.abilities"]["abilities"]["wave"]["events"][0]["tick"]
        .as_u64()
        .unwrap();
    assert!(tick > 10, "{tick}");
    assert_eq!(
        moved,
        acl(&start, &format!("ability wave event move 0 {tick}"))
    );
    // + Event at the playhead (half way through the ability's 190 ticks).
    let actions = h.click("anim:add_event");
    let added = h.commit(actions);
    let sound = h.editor.tuning.anim.sound.clone();
    assert_eq!(
        added,
        acl(&moved, &format!("ability wave event add 95 {sound}"))
    );
    assert_eq!(h.editor.tuning.anim.selected, Some(("wave".into(), 1)));
    // The selected event's cue, then delete it.
    let other = if sound == "pop" { "chime" } else { "pop" };
    assert!(h.click("anim:event_sound").is_empty());
    h.frame(vec![]);
    let actions = h.click(&format!("anim:event_sound:{other}"));
    let cued = h.commit(actions);
    assert_eq!(
        cued,
        acl(&added, &format!("ability wave event sound 1 {other}"))
    );
    let actions = h.click("anim:delete_event");
    assert_eq!(h.commit(actions), moved);
    // The ability model is rebuilt per revision only.
    let builds = h.editor.tuning.anim.builds;
    for _ in 0..10 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.tuning.anim.builds, builds);
}

#[test]
fn vfx_curve_and_gradient_drags_write_once_like_acl() {
    let start = package("effects-tour");
    let mut h = Harness::sized(start.clone(), "window.vfx", vec2(1700.0, 2000.0));
    h.editor
        .run_command_with("vfx.select", Some("fx/01-cpu"))
        .unwrap();
    h.frame(vec![]);
    let size = |w: &World| {
        let e = &w.resources["vfx.library"]["emitters"]["fx/01-cpu"];
        (e["size"].as_f64().unwrap(), e["end_size"].as_f64().unwrap())
    };
    let (s0, e0) = size(&start);
    // Drag the start handle up: nothing written while held, one revision on release.
    let handle = h.rect("vfx:size_handle:0").center();
    let actions = h.drag(handle, handle - vec2(0.0, 30.0), 6);
    let grown = h.commit(actions);
    let (s1, e1) = size(&grown);
    assert!(s1 > s0 && e1 == e0, "{s0} -> {s1}");
    assert_eq!(grown, acl(&start, &format!("vfx fx/01-cpu size {s1} {e1}")));
    // Gradient: select the end stop, drag its R slider to the left end.
    h.click("vfx:stop:1");
    assert_eq!(h.editor.tuning.vfx.stop, 1);
    let slider = h.rect("vfx:channel:0");
    let actions = h.drag(
        slider.center(),
        Pos2::new(slider.left() - 20.0, slider.center().y),
        5,
    );
    let red = h.commit(actions);
    let e = &red.resources["vfx.library"]["emitters"]["fx/01-cpu"];
    assert_eq!(e["end_color"][0], 0.0);
    let c = |v: &serde_json::Value| {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap().to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    assert_eq!(
        red,
        acl(
            &grown,
            &format!(
                "vfx fx/01-cpu color {} {}",
                c(&e["color"]),
                c(&e["end_color"])
            )
        )
    );
    let builds = h.editor.tuning.vfx.builds;
    for _ in 0..10 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.tuning.vfx.builds, builds);
}

#[test]
fn shader_check_runs_in_the_background_and_marks_the_error_line() {
    let start = package("shader-lab");
    let mut h = Harness::new(start.clone(), "window.inspector");
    h.editor.open_material("scanner");
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.editor.shader_under_edit().as_deref(), Some("scan"));
    assert!(h.editor.rects.contains_key("shader:source"));
    // Valid as authored.
    h.click("shader:check");
    for _ in 0..600 {
        if h.editor.tuning.shader.job.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.frame(vec![]);
    }
    let checked = h
        .editor
        .tuning
        .shader
        .checked
        .clone()
        .expect("check finished");
    assert_eq!(checked.state, "finished");
    assert!(checked.issues.is_empty(), "{:?}", checked.issues);
    assert_eq!(h.editor.last_job.as_ref().unwrap()["kind"], "shader_check");
    // Break line 3 (an undefined name), check again: line 3 is reported and marked.
    let broken = h
        .editor
        .tuning
        .shader
        .draft
        .replacen("smoothstep", "smoothstepp", 1);
    h.editor.tuning.shader.draft = broken.clone();
    h.frame(vec![]);
    h.editor.run_command("shader.check").unwrap();
    assert!(
        h.editor.run_command("shader.check").is_err(),
        "one check at a time"
    );
    for _ in 0..600 {
        if h.editor.tuning.shader.job.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.frame(vec![]);
    }
    h.frame(vec![]);
    let issue = h.editor.tuning.shader.checked.as_ref().unwrap().issues[0].clone();
    assert_eq!(issue.line, Some(3), "{issue:?}");
    assert!(
        issue.message.starts_with("line 3, column"),
        "{}",
        issue.message
    );
    assert!(h.editor.rects.contains_key("shader:error_line"));
    // Apply the broken source: the commit is refused with the same line.
    let actions = h.click("shader:apply");
    let actions = super::coalesce(actions);
    let [Action::Write {
        request: Request::Apply { edits, .. },
        ..
    }] = actions.as_slice()
    else {
        panic!("expected one write");
    };
    let refused = h
        .store
        .execute(Request::Apply {
            expected_rev: None,
            edits: edits.clone(),
        })
        .unwrap_err();
    assert!(format!("{refused:#}").contains("line 3"), "{refused:#}");
    // A valid change applies like the ACL verb.
    let fixed = broken
        .replacen("smoothstepp", "smoothstep", 1)
        .replacen("0.65", "0.6", 1);
    h.editor.tuning.shader.draft = fixed.clone();
    h.frame(vec![]);
    let actions = h.click("shader:apply");
    let applied = h.commit(actions);
    assert_eq!(
        applied,
        acl(
            &start,
            &format!(
                "shader scan source {}",
                serde_json::to_string(&fixed).unwrap()
            )
        )
    );
    // Cancel: a running check stops (or finishes) and is never left behind.
    h.editor.run_command("shader.check").unwrap();
    h.editor.run_command("shader.cancel_check").unwrap();
    for _ in 0..600 {
        if h.editor.tuning.shader.job.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.frame(vec![]);
    }
    assert!(h.editor.tuning.shader.job.is_none());
}
