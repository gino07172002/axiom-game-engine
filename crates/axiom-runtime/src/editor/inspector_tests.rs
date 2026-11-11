//! Inspector 2.0 driven by real egui input: typed fields commit once per interaction, Esc
//! cancels, arrays, mixed multi-object values, the entity picker and Hierarchy drops, the
//! component header and the create-command options dialogs. Every write is compared with
//! the ACL line an agent would send.
use super::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::{Edit, Request, Store, World};
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use serde_json::{json, Value};

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
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
}

impl Harness {
    fn new(world: World) -> Self {
        let registry = crate::installed_registry();
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_component_hints(&registry.authoring);
        // Inspector at full height: the fields under test are on screen without scrolling.
        editor
            .run_command_with("layout.load", Some("Tall Inspector"))
            .unwrap();
        Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
            store: Store::memory_with(registry.authoring, world).unwrap(),
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 1300.0))),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        actions
    }
    fn rect(&self, key: &str) -> Rect {
        *self
            .editor
            .rects
            .get(key)
            .unwrap_or_else(|| panic!("{key} not drawn; have {:?}", self.keys("form:")))
    }
    fn keys(&self, prefix: &str) -> Vec<&String> {
        let mut keys: Vec<&String> = self
            .editor
            .rects
            .keys()
            .filter(|k| k.starts_with(prefix))
            .collect();
        keys.sort();
        keys
    }
    fn click(&mut self, key: &str) -> Vec<Action> {
        let at = self.rect(key).center();
        let mut actions = self.frame(vec![Event::PointerMoved(at), press(at, true)]);
        actions.extend(self.frame(vec![press(at, false)]));
        actions
    }
    /// Opens the open component card's "…" menu, lets the popup settle (its first frame is
    /// laid out before it is kept on screen), then clicks `item` in it.
    fn menu(&mut self, item: &str) -> Vec<Action> {
        self.click("component:menu");
        self.frame(vec![]);
        self.click(item)
    }
    fn key(&mut self, key: Key) -> Vec<Action> {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }])
    }
    /// Presses on `key`, moves `dx` px over `steps` frames (no write may appear while held)
    /// and returns the actions of the release frame.
    fn drag(&mut self, key: &str, dx: f32, steps: usize) -> Vec<Action> {
        let from = self.rect(key).center();
        assert!(self
            .frame(vec![Event::PointerMoved(from), press(from, true)])
            .is_empty());
        let mut at = from;
        for step in 1..=steps {
            at = from + vec2(dx * step as f32 / steps as f32, 0.0);
            let held = self.frame(vec![Event::PointerMoved(at)]);
            assert!(
                held.is_empty(),
                "a write while the drag is held (step {step})"
            );
        }
        self.frame(vec![press(at, false)])
    }
    /// Commits the single write in `actions` like the host and shows the new revision.
    fn commit(&mut self, actions: Vec<Action>) -> Vec<Edit> {
        let [Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }] = actions.as_slice()
        else {
            panic!("expected exactly one write, got {}", actions.len());
        };
        let edits = edits.clone();
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
        edits
    }
    fn world(&self) -> World {
        self.store.current().unwrap().1
    }
    fn value(&self, id: &str, component: &str, pointer: &str) -> Value {
        let world = self.world();
        let e = world.entity(id).unwrap();
        world.resolved_components(e)[component]
            .pointer(pointer)
            .cloned()
            .unwrap_or(Value::Null)
    }
    fn open(&mut self, id: &str, component: &str) {
        self.editor.select(Some(id.into()));
        self.frame(vec![]);
        self.click(&format!("component:{component}"));
        self.frame(vec![]);
        assert_eq!(self.editor.draft.as_ref().unwrap().component, component);
    }
}

fn press(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

/// The world the editor's writes produced equals the ACL script's.
fn assert_same(h: &Harness, start: &World, script: &str) {
    assert_eq!(
        h.world(),
        acl(start, script),
        "editor and `{script}` diverge"
    );
}

#[test]
fn dragging_a_field_commits_once_on_release_and_undo_restores_it() {
    let start = acl(&lab(), "create light point id=lamp");
    let mut h = Harness::new(start.clone());
    h.open("lamp", "render.light");
    let before = h.value("lamp", "render.light", "/intensity");
    let rev = h.editor.rev;
    // 12 frames of dragging: nothing is written while held, one revision on release.
    let released = h.drag("form:/intensity", 60.0, 12);
    let edits = h.commit(released);
    assert_eq!(h.editor.rev, rev + 1);
    assert_eq!(edits.len(), 1);
    let after = h.value("lamp", "render.light", "/intensity");
    assert!(
        after.as_f64().unwrap() > before.as_f64().unwrap(),
        "{before} -> {after}"
    );
    assert_same(
        &h,
        &start,
        &format!("set lamp render.light/intensity {after}"),
    );
    // The component stays open after the commit and the next frames write nothing.
    for _ in 0..3 {
        assert!(h.frame(vec![]).is_empty());
    }
    assert_eq!(h.editor.draft.as_ref().unwrap().component, "render.light");
    // Undo appends a revision restoring the value.
    let [Action::Restore {
        rev: target,
        redo: false,
    }] = h.editor.run_command("edit.undo").unwrap()[..]
    else {
        panic!("expected an undo");
    };
    let current = h.editor.rev;
    h.store
        .execute(Request::Undo {
            rev: target,
            expected_rev: Some(current),
        })
        .unwrap();
    assert_eq!(h.value("lamp", "render.light", "/intensity"), before);
}

#[test]
fn an_edit_made_while_an_earlier_one_is_saving_waits_and_is_kept() {
    let start = acl(&lab(), "create light point id=lamp");
    let mut h = Harness::new(start.clone());
    h.open("lamp", "render.light");
    // The host is still saving an earlier write: the released drag waits, nothing is sent.
    h.editor.saving = 1;
    let released = h.drag("form:/intensity", 40.0, 4);
    assert!(released.is_empty());
    assert!(h.editor.dirty());
    let value = h.editor.draft.as_ref().unwrap().form.as_ref().unwrap()["intensity"].clone();
    // The earlier write's staged world arrives: the edit in progress is not reset.
    let staged = std::sync::Arc::new(h.editor.world.as_ref().clone());
    h.editor.apply_staged(staged, None);
    assert!(h.editor.dirty());
    // Saved: the waiting edit commits once.
    h.editor.saving = 0;
    let actions = h.frame(vec![]);
    h.commit(actions);
    assert_same(
        &h,
        &start,
        &format!("set lamp render.light/intensity {value}"),
    );
}

#[test]
fn escape_without_live_mode_keeps_the_draft() {
    let start = acl(&lab(), "create light point id=lamp");
    let mut h = Harness::new(start);
    h.editor.prefs.live_inspector = false;
    h.open("lamp", "render.light");
    assert!(
        h.drag("form:/intensity", 40.0, 4).is_empty(),
        "batch mode never commits"
    );
    assert!(h.key(Key::Escape).is_empty());
    assert!(
        h.editor.dirty(),
        "Esc must not discard an unapplied batch draft"
    );
}

#[test]
fn escape_cancels_a_drag_or_a_typed_value_without_a_revision() {
    let start = acl(&lab(), "create light point id=lamp");
    let mut h = Harness::new(start.clone());
    h.open("lamp", "render.light");
    let field = h.rect("form:/intensity").center();
    // Drag, Esc while still holding, keep moving, release: nothing written.
    h.frame(vec![Event::PointerMoved(field), press(field, true)]);
    h.frame(vec![Event::PointerMoved(field + vec2(30.0, 0.0))]);
    assert!(h.editor.dirty());
    let mut actions = h.key(Key::Escape);
    actions.extend(h.frame(vec![Event::PointerMoved(field + vec2(60.0, 0.0))]));
    actions.extend(h.frame(vec![press(field + vec2(60.0, 0.0), false)]));
    actions.extend(h.frame(vec![]));
    assert!(actions.is_empty(), "Esc must cancel the drag");
    assert!(!h.editor.dirty());
    assert_eq!(h.world(), start);
    // Click to type a value, then Esc: nothing written either.
    h.click("form:/intensity");
    let mut actions = h.frame(vec![Event::Text("7".into())]);
    actions.extend(h.key(Key::Escape));
    actions.extend(h.frame(vec![]));
    assert!(actions.is_empty(), "Esc must cancel the typed value");
    assert_eq!(h.world(), start);
    // The light preview showed the draft only while it was being dragged.
    assert!(h.editor.draft.as_ref().is_some_and(|d| !d.form_dirty));
}

#[test]
fn array_items_add_insert_reorder_and_remove_like_acl() {
    let start = lab();
    let mut h = Harness::new(start.clone());
    h.open("stage/rack", "render.mesh");
    let parts = h.value("stage/rack", "render.mesh", "/parts");
    let n = parts.as_array().unwrap().len();
    // + Add item: a copy of the last item at the end.
    let clicked = h.click("form:/parts:add");
    h.commit(clicked);
    let last = parts[n - 1].to_string();
    assert_same(
        &h,
        &start,
        &format!("append stage/rack render.mesh/parts {last}"),
    );
    // + before item 0: a copy of item 0 inserted there.
    h.frame(vec![]);
    let clicked = h.click("form:/parts:ins:0");
    h.commit(clicked);
    let script = format!(
        "append stage/rack render.mesh/parts {last}\ninsert stage/rack render.mesh/parts 0 {}",
        parts[0]
    );
    assert_same(&h, &start, &script);
    // ▼ on item 0, then ✕ on item 1.
    h.frame(vec![]);
    let clicked = h.click("form:/parts:down:0");
    h.commit(clicked);
    h.frame(vec![]);
    let clicked = h.click("form:/parts:del:1");
    h.commit(clicked);
    assert_same(
        &h,
        &start,
        &format!("{script}\nreorder stage/rack render.mesh/parts 0 1\nremove stage/rack render.mesh/parts 1"),
    );
}

#[test]
fn long_arrays_page_instead_of_drawing_every_item() {
    let parts: Vec<Value> = (0..300)
        .map(|i| json!({"transform":{"position":[i as f32 * 0.01,0.0,0.0]},"color":[0.5,0.5,0.5]}))
        .collect();
    let start = acl(
        &lab(),
        &format!(
            "create cube id=big\nset big render.mesh/parts {}",
            Value::Array(parts)
        ),
    );
    let mut h = Harness::new(start);
    h.open("big", "render.mesh");
    let drawn = h.keys("form:/parts:del:").len();
    assert_eq!(drawn, super::form::PAGE, "one page of items, not 300");
}

#[test]
fn mixed_values_show_a_dash_and_one_edit_sets_every_selected_object() {
    let start = acl(
        &lab(),
        "create light point id=lamp at=0,1,0\ncreate light point id=lamp2 at=2,1,0\nset lamp2 render.light/intensity 3",
    );
    let mut h = Harness::new(start.clone());
    h.editor.select(Some("lamp".into()));
    h.editor.extra.insert("lamp2".into());
    h.frame(vec![]);
    h.click("component:render.light");
    h.frame(vec![]);
    assert!(h.editor.mixed.fields.contains("/intensity"));
    assert!(!h.editor.mixed.fields.contains("/kind"));
    assert_eq!(h.editor.mixed.transform[0], [true, false, false]);
    // Cached per world / selection / component, not rebuilt per frame.
    let builds = h.editor.mixed.builds;
    for _ in 0..4 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.mixed.builds, builds);
    let released = h.drag("form:/intensity", 40.0, 6);
    h.commit(released);
    let value = h.value("lamp", "render.light", "/intensity");
    assert_eq!(h.value("lamp2", "render.light", "/intensity"), value);
    assert_same(
        &h,
        &start,
        &format!("set entities[id=lamp,id=lamp2] render.light/intensity {value}"),
    );
    // Same value everywhere now: no longer mixed.
    h.frame(vec![]);
    assert!(!h.editor.mixed.fields.contains("/intensity"));
}

#[test]
fn entity_picker_search_and_hierarchy_drop_set_a_reference() {
    let start = acl(
        &lab(),
        "create path id=route\ncreate path id=loop at=0,0,12\ncreate cube id=walker\nfollow walker route",
    );
    let mut h = Harness::new(start.clone());
    h.open("walker", "path.follow");
    // Click the field: the picker lists objects with path.spline; type to filter, pick.
    h.click("form:/path");
    h.frame(vec![Event::Text("loo".into())]);
    h.frame(vec![]);
    assert!(h.editor.rects.contains_key("form:pick:/path:loop"));
    assert!(!h.editor.rects.contains_key("form:pick:/path:route"));
    assert!(!h.editor.rects.contains_key("form:pick:/path:walker"));
    let picked = h.click("form:pick:/path:loop");
    h.commit(picked);
    assert_same(&h, &start, "set walker path.follow/path \"loop\"");
    // Drag the Hierarchy row of `route` onto the field.
    h.frame(vec![]);
    let row = h.rect("row:route");
    let grab = pos2(row.left() + 80.0, row.center().y);
    let drop = h.rect("form:/path").center();
    let mut actions = h.frame(vec![Event::PointerMoved(grab), press(grab, true)]);
    for step in 1..=8 {
        let p = grab + (drop - grab) * (step as f32 / 8.0);
        actions.extend(h.frame(vec![Event::PointerMoved(p)]));
    }
    actions.extend(h.frame(vec![press(drop, false)]));
    actions.extend(h.frame(vec![]));
    h.commit(actions);
    assert_same(
        &h,
        &start,
        "set walker path.follow/path \"loop\"\nset walker path.follow/path \"route\"",
    );
}

#[test]
fn component_header_reset_copy_paste_enable_and_main_camera_match_acl() {
    let start = acl(
        &lab(),
        "create light point id=lamp\ncreate light point id=lamp2 at=3,1,0\nset lamp render.light/intensity 9",
    );
    let mut h = Harness::new(start.clone());
    h.open("lamp", "render.light");
    // Copy lamp's values, paste them on lamp2.
    h.menu("component:copy");
    assert!(h.editor.component_clipboard.is_some());
    let copied = h.value("lamp", "render.light", "");
    h.open("lamp2", "render.light");
    let pasted = h.menu("component:paste");
    h.commit(pasted);
    assert_same(
        &h,
        &start,
        &format!("set lamp2 /components/render.light {copied}"),
    );
    // Enabled toggle: a live form edit.
    h.frame(vec![]);
    let toggled = h.click("component:enabled");
    h.commit(toggled);
    let script = format!(
        "set lamp2 /components/render.light {copied}\nset lamp2 render.light/enabled false"
    );
    assert_same(&h, &start, &script);
    // Reset lamp back to the default light.
    h.open("lamp", "render.light");
    let reset = h.click("component:reset");
    h.commit(reset);
    assert_same(&h, &start, &format!("{script}\nreset lamp render.light"));
    // Make Main Camera on a second camera.
    let cams = acl(&lab(), "create camera id=cam2");
    let mut h = Harness::new(cams.clone());
    h.open("cam2", "render.camera");
    let main = h.menu("component:main_camera");
    h.commit(main);
    assert_same(&h, &cams, "camera main cam2");
}

#[test]
fn render_mesh_blocks_follow_cameras_checkbox_matches_acl() {
    let start = acl(&lab(), "create cube id=wall");
    let mut h = Harness::new(start.clone());
    h.open("wall", "render.mesh");
    let on = h.click("form:camera_blocker");
    h.commit(on);
    assert_same(&h, &start, "camera wall blocker on");
    h.open("wall", "render.mesh");
    let off = h.click("form:camera_blocker");
    h.commit(off);
    assert_same(
        &h,
        &start,
        "camera wall blocker on\ncamera wall blocker off",
    );
}

#[test]
fn play_on_field_binds_and_unbinds_a_clip_like_acl() {
    let start = acl(&lab(), "create cube id=door\ntween door clip open");
    let mut h = Harness::new(start.clone());
    h.editor.select(Some("door".into()));
    h.frame(vec![]);
    h.click("tween:play_on");
    h.frame(vec![Event::Text("door_open".into())]);
    let bound = h.click("tween:bind");
    h.commit(bound);
    assert_same(&h, &start, "tween door clip open set play_on=door_open");
    h.frame(vec![]);
    let unbound = h.click("tween:unbind:door_open");
    h.commit(unbound);
    assert_same(
        &h,
        &start,
        "tween door clip open set play_on=door_open\nremove resources[id=runtime.bindings] events 0",
    );
}

#[test]
fn options_dialogs_create_what_their_acl_line_creates() {
    // Make Interactable: menu opens the dialog, fields typed, Create clicked.
    let start = acl(&lab(), "create cube id=npc");
    let mut h = Harness::new(start.clone());
    h.editor.select(Some("npc".into()));
    h.frame(vec![]);
    let mut actions = vec![];
    h.editor.invoke("object.interactable", &mut actions);
    assert!(actions.is_empty(), "the menu opens the dialog first");
    h.frame(vec![]);
    // The window lays itself out in an invisible first pass.
    h.frame(vec![]);
    h.click("options:Event");
    h.frame(vec![Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    h.frame(vec![Event::Text("talk".into())]);
    h.click("options:Prompt");
    h.frame(vec![Event::Text("和阿明說話".into())]);
    let line = h.editor.options.as_ref().unwrap().acl(&h.editor);
    assert_eq!(line, "interact npc event=talk prompt=和阿明說話");
    let created = h.click("options:create");
    h.commit(created);
    assert!(h.editor.options.is_none());
    assert_same(&h, &start, &line);
    // The other dialogs, options set on the dialog, Create = dialog.run.
    let base = acl(
        &lab(),
        "create cube id=a\ncreate cube id=b\ncreate path id=route\ncreate camera id=cam2\naudio clip engine engine",
    );
    type Case<'a> = (&'a str, Vec<&'a str>, Vec<(&'a str, &'a str)>);
    let cases: Vec<Case> = vec![
        (
            "object.tween_clip",
            vec!["a"],
            vec![
                ("Clip name", "spin"),
                ("Duration (s)", "2"),
                ("Mode", "once"),
                ("On finish event", "spun"),
                ("Play on event", "go"),
            ],
        ),
        (
            "object.audio_source",
            vec!["a", "b"],
            vec![("Clip", "engine"), ("Volume", "0.5"), ("Loop", "on")],
        ),
        (
            "object.nav_agent",
            vec!["a"],
            vec![("Speed (m/s)", "2.5"), ("Radius (m)", "0.4")],
        ),
        (
            "object.rigid_body",
            vec!["a", "b"],
            vec![
                ("Kind", "kinematic"),
                ("Mass (kg)", "2.5"),
                ("Collider (fit to mesh when missing)", "sphere"),
            ],
        ),
        (
            "object.follow_path",
            vec!["a", "route"],
            vec![("Speed (m/s)", "3"), ("At the end", "ping_pong")],
        ),
        (
            "object.camera_follow",
            vec!["cam2", "a"],
            vec![("Distance (m)", "7"), ("Pitch (°)", "30")],
        ),
    ];
    for (command, selection, fields) in cases {
        let mut e = Editor::new(base.clone(), 1, None);
        e.set_component_hints(&crate::installed_registry().authoring);
        e.select(Some(selection[0].into()));
        for id in &selection[1..] {
            e.extra.insert((*id).into());
        }
        let mut dialog = super::options::OptionsDialog::for_command(&e, command).unwrap();
        for (label, value) in fields {
            dialog.set(label, value);
        }
        let line = dialog.acl(&e);
        let Some(Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }) = dialog.run(&mut e).unwrap()
        else {
            panic!("{command}: no write ({})", e.status);
        };
        let mut store =
            Store::memory_with(crate::installed_registry().authoring, base.clone()).unwrap();
        store
            .execute(Request::Apply {
                expected_rev: None,
                edits,
            })
            .unwrap();
        assert_eq!(
            store.current().unwrap().1,
            acl(&base, &line),
            "{command}: `{line}`"
        );
    }
}

/// Inspector cost on a 4096-object world: an open entity picker filters once per keystroke
/// and the form is built from the cached spec (prints timings; debug build under
/// `cargo test`, the bound is the 16 ms frame budget times a debug factor).
#[test]
fn inspector_on_a_4096_object_world_stays_within_budget() {
    let mut world = lab();
    world.capabilities.insert("path".into(), 1);
    // 6 lab objects + 4087 + route, walker = 4096 (the entity budget).
    for i in 0..4087 {
        world.entities.push(
            serde_json::from_value(json!({"id":format!("obj/{i}"),"components":{
                "core.transform":{"position":[i as f32 % 64.0,0.0,(i / 64) as f32]},
                "render.mesh":{"parts":[{"color":[0.5,0.5,0.5]}]}}}))
            .unwrap(),
        );
    }
    let start = acl(
        &world,
        "create path id=route\ncreate cube id=walker\nfollow walker route",
    );
    let mut h = Harness::new(start);
    h.open("walker", "path.follow");
    let builds = h.editor.specs.builds;
    let time = |h: &mut Harness, events: Vec<Event>| {
        let t = std::time::Instant::now();
        h.frame(events);
        t.elapsed().as_secs_f64() * 1000.0
    };
    let idle: Vec<f64> = (0..10).map(|_| time(&mut h, vec![])).collect();
    h.click("form:/path");
    let open = time(&mut h, vec![]);
    let typed = time(&mut h, vec![Event::Text("obj/40".into())]);
    let filtered: Vec<f64> = (0..10).map(|_| time(&mut h, vec![])).collect();
    let max = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
    println!(
        "4096 world inspector frame ms: idle max {:.2}, picker open {open:.2}, typed {typed:.2}, filtered max {:.2}",
        max(&idle),
        max(&filtered)
    );
    assert_eq!(
        h.editor.specs.builds, builds,
        "specs are cached, not rebuilt per frame"
    );
    assert!(max(&filtered) < 16.0 * 6.0 && typed < 16.0 * 6.0);
}
