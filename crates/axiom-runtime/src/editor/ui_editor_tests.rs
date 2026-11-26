//! UI Editor driven by real egui input on Axiom Town: canvas drag between regions, the
//! element panel's dropdowns and effects, locale preview and effect preview. Every write is
//! compared with the ACL `ui ...` line an agent would send.
use super::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::{Edit, Request, Store, World};
use egui::{epaint::Shape, vec2, Event, Modifiers, PointerButton, Pos2, Rect};

fn town() -> World {
    authored(raw_town())
}

/// The town package as shipped (templates not expanded).
fn raw_town() -> World {
    axiom_core::ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &crate::installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// Axiom Town as its UI compiles: the template uses (plan M5, the emote buttons) expanded into
/// authored widgets, so these tests can edit those buttons as ordinary widgets.
fn authored(mut world: World) -> World {
    let doc: axiom_core::gameplay::UiDocument =
        serde_json::from_value(world.resources["ui.document"].clone()).unwrap();
    let doc = axiom_core::ui_template::expand(&doc).unwrap();
    world
        .resources
        .insert("ui.document".into(), serde_json::to_value(doc).unwrap());
    world
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

/// Every text painted in a frame (to check what the canvas shows).
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Text(t) => out.push(t.galley.text().to_owned()),
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    for s in shapes {
        walk(&s.shape, &mut out);
    }
    out
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
    store: Store,
    time: f64,
    shown: Vec<String>,
}

impl Harness {
    fn new(world: World) -> Self {
        let registry = crate::installed_registry();
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_component_hints(&registry.authoring);
        editor.run_command("window.ui_editor").unwrap();
        let mut h = Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
            store: Store::memory_with(registry.authoring, world).unwrap(),
            time: 0.0,
            shown: vec![],
        };
        // Areas lay out invisibly on their first frame.
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }
    fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1700.0, 1100.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        self.shown = texts(&out.shapes);
        self.time += 1.0 / 60.0;
        actions
    }
    fn rect(&self, key: &str) -> Rect {
        *self.editor.rects.get(key).unwrap_or_else(|| {
            let mut keys: Vec<_> = self
                .editor
                .rects
                .keys()
                .filter(|k| k.starts_with("ui_"))
                .collect();
            keys.sort();
            panic!("{key} not drawn; have {keys:?}")
        })
    }
    fn click(&mut self, key: &str) -> Vec<Action> {
        let at = self.rect(key).center();
        let mut actions = self.frame(vec![Event::PointerMoved(at), press(at, true)]);
        actions.extend(self.frame(vec![press(at, false)]));
        actions
    }
    /// Opens a dropdown and picks an item (both drawn by name in `rects`).
    fn pick(&mut self, combo: &str, item: &str) -> Vec<Action> {
        assert!(self.click(combo).is_empty());
        self.frame(vec![]);
        self.click(item)
    }
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
        self.frame(vec![]);
        self.frame(vec![]);
        edits
    }
    fn world(&self) -> World {
        self.store.current().unwrap().1
    }
    fn shows(&self, text: &str) -> bool {
        self.shown.iter().any(|t| t.contains(text))
    }
}

/// Demo step 6: drag 好感度 (the `affection` progress) from top-left to top-right, switch it to
/// the `value` style, add enter fade-in + number roll, preview zh-TW / en.
#[test]
fn demo_step_six_drag_style_effects_and_locales_match_acl() {
    let start = town();
    let mut h = Harness::new(start.clone());
    // The canvas draws the document with the game's code: zh-TW by default.
    assert!(h.shows("中央廣場"), "{:?}", h.shown);
    let from = h.rect("ui_canvas:affection").center();
    let to = h.rect("ui_cell:top_right").center();
    assert!(h.rect("ui_cell:top_left").contains(from));
    // Drag: nothing is written while the button is held; one write on release.
    assert!(h
        .frame(vec![Event::PointerMoved(from), press(from, true)])
        .is_empty());
    for step in 1..=10 {
        let at = from + (to - from) * (step as f32 / 10.0);
        assert!(
            h.frame(vec![Event::PointerMoved(at)]).is_empty(),
            "step {step}"
        );
    }
    let release = h.frame(vec![press(to, false)]);
    h.commit(release);
    let moved = acl(&start, "ui place affection top_right");
    assert_eq!(h.world(), moved);
    // The canvas follows: the bar is now in the top-right cell.
    assert!(h
        .rect("ui_cell:top_right")
        .contains(h.rect("ui_canvas:affection").center()));
    // Select it, then the Style dropdown.
    assert!(h.click("ui_canvas:affection").is_empty());
    let styled = h.pick("ui_editor:style", "ui_style:value");
    h.commit(styled);
    let valued = acl(&moved, "ui style affection value");
    assert_eq!(h.world(), valued);
    // The value style shows the number next to the bar.
    assert!(h.shows("0 / 100"), "{:?}", h.shown);
    // Add effect: fade_in (the default kind), then number_roll.
    let added = h.click("ui_editor:add_effect");
    h.commit(added);
    let faded = acl(&valued, "ui effect add affection.fade_in affection fade_in");
    assert_eq!(h.world(), faded);
    assert!(faded.systems.iter().any(|s| s == "ui.effects"));
    assert!(h
        .pick("ui_editor:new_kind", "ui_new_kind:number_roll")
        .is_empty());
    let added = h.click("ui_editor:add_effect");
    h.commit(added);
    let rolled = acl(
        &faded,
        "ui effect add affection.number_roll affection number_roll",
    );
    assert_eq!(h.world(), rolled);
    // Preview plays both: half way through the 60-step roll (ease_out) it shows 75 / 100
    // (the authored value is 0, so the preview rolls to the bar's end).
    h.click("ui_editor:preview");
    let started = h.time;
    while h.time < started + 0.25 {
        h.frame(vec![]);
    }
    assert!(
        h.shown
            .iter()
            .any(|t| t.ends_with(" / 100") && t != "0 / 100"),
        "{:?}",
        h.shown
    );
    while h.time < started + 1.0 {
        h.frame(vec![]);
    }
    assert!(h.shows("0 / 100"));
    // Locale switch is a view change only (no write); the canvas shows English.
    assert!(h.click("ui_locale:en").is_empty());
    h.frame(vec![]);
    assert!(
        h.shows("Central Plaza") && !h.shows("中央廣場"),
        "{:?}",
        h.shown
    );
    assert!(h.click("ui_locale:zh-TW").is_empty());
    h.frame(vec![]);
    assert!(h.shows("中央廣場"));
    // Nameplate preview.
    assert!(h.shows("Lumi"), "{:?}", h.shown);
    // The preview was built once per revision, not per frame.
    let builds = h.editor.ui_editor.builds;
    for _ in 0..20 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.ui_editor.builds, builds);
}

#[test]
fn effect_form_button_command_and_removal_match_acl() {
    let start = acl(
        &town(),
        "ui effect add coins.pulse coins pulse\nui place affection top_right",
    );
    let mut h = Harness::new(start.clone());
    assert!(h.click("ui_canvas:coins").is_empty());
    assert!(h.click("ui_effect:coins.pulse").is_empty());
    h.frame(vec![]);
    // Trigger dropdown: on event (the first declared event is chosen).
    let on = h.pick("ui_editor:effect_on", "ui_effect_on:event");
    h.commit(on);
    let first = start.resources["behavior.program"]["events"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    let expected = acl(
        &start,
        &format!("ui effect set coins.pulse on=event event={first}"),
    );
    assert_eq!(h.world(), expected);
    // Remove it with the row's x.
    let removed = h.click("ui_effect_remove:coins.pulse");
    h.commit(removed);
    assert_eq!(h.world(), acl(&expected, "ui effect remove coins.pulse"));
    // Button command picker on an emote button.
    assert!(h.click("ui_canvas:emote_wave").is_empty());
    let picked = h.pick("ui_editor:command", "ui_command:emit emote_bow");
    let edits = h.commit(picked);
    assert_eq!(edits.len(), 1);
    assert_eq!(
        h.world(),
        acl(
            &acl(&expected, "ui effect remove coins.pulse"),
            "ui button emote_wave emit emote_bow"
        )
    );
}

/// Canvas zoom: the wheel zooms around the pointer, middle-drag pans, `ui.zoom` and the
/// readout button; none of it writes. Drags at 200 % and 50 % move the document point by the
/// screen delta / scale and commit exactly one `ui place` each.
#[test]
fn zoomed_canvas_drags_in_document_pixels_and_zoom_is_view_only() {
    let start = town();
    let mut h = Harness::new(start.clone());
    let view = h.rect("ui_editor:view");
    assert_eq!(h.rect("ui_editor:canvas"), view, "100 % = fit to the tab");
    let fit = view.width() / 1280.0;
    let wheel = |dy: f32| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        modifiers: Modifiers::NONE,
    };
    let to_screen = |canvas: Rect, scale: f32, x: f32, y: f32| canvas.min + vec2(x, y) * scale;
    // Wheel 2x over the affection bar: it stays under the pointer.
    let at = h.rect("ui_canvas:affection").center();
    assert!(h.frame(vec![Event::PointerMoved(at)]).is_empty());
    assert!(h.frame(vec![wheel(2.0f32.ln() / 0.003)]).is_empty());
    h.frame(vec![]);
    assert!((h.editor.ui_editor.zoom() - 2.0).abs() < 1e-3);
    let canvas = h.rect("ui_editor:canvas");
    assert!((canvas.width() - 2.0 * view.width()).abs() < 0.5);
    assert_eq!(h.rect("ui_editor:view"), view, "the view does not move");
    assert!(h.rect("ui_canvas:affection").center().distance(at) < 2.0);
    // Drag at 200 % into the top cell: document delta = screen delta / (fit * 2).
    let scale = fit * 2.0;
    let target = to_screen(canvas, scale, 470.0, 100.0);
    assert!(view.contains(target), "{target:?} outside {view:?}");
    assert!(h
        .frame(vec![Event::PointerMoved(at), press(at, true)])
        .is_empty());
    for step in 1..=10 {
        let p = at + (target - at) * (step as f32 / 10.0);
        assert!(h.frame(vec![Event::PointerMoved(p)]).is_empty());
        let delta = h.editor.ui_editor.drag_doc_delta().unwrap();
        let expected = (p - at) / scale;
        assert!(
            (delta - expected).length() < 1e-3,
            "{delta:?} != {expected:?}"
        );
    }
    let released = h.frame(vec![press(target, false)]);
    assert!(h.editor.ui_editor.drag_doc_delta().is_none());
    h.commit(released);
    let top = acl(&start, "ui place affection top");
    assert_eq!(h.world(), top);
    // Middle-drag pans the canvas (the view stays), without a write.
    let canvas = h.rect("ui_editor:canvas");
    let grab = view.center();
    let middle = |pos: Pos2, pressed: bool| Event::PointerButton {
        pos,
        button: PointerButton::Middle,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut actions = h.frame(vec![Event::PointerMoved(grab), middle(grab, true)]);
    for step in 1..=4 {
        actions.extend(h.frame(vec![Event::PointerMoved(
            grab + vec2(-10.0, 8.0) * step as f32,
        )]));
    }
    actions.extend(h.frame(vec![middle(grab + vec2(-40.0, 32.0), false)]));
    actions.extend(h.frame(vec![]));
    assert!(actions.is_empty());
    let panned = h.rect("ui_editor:canvas");
    assert!((panned.min - canvas.min - vec2(-40.0, 32.0)).length() < 0.5);
    assert_eq!(h.rect("ui_editor:view"), view);
    // 50 % (command, around the view's center): drag the bar to the top-right cell.
    assert!(h
        .editor
        .run_command_with("ui.zoom", Some("50%"))
        .unwrap()
        .is_empty());
    h.frame(vec![]);
    assert_eq!(h.editor.ui_editor.zoom(), 0.5);
    let canvas = h.rect("ui_editor:canvas");
    let scale = fit * 0.5;
    assert!((canvas.width() - 0.5 * view.width()).abs() < 0.5);
    let from = h.rect("ui_canvas:affection").center();
    let target = to_screen(canvas, scale, 1100.0, 100.0);
    assert!(view.contains(target) && canvas.contains(target));
    assert!(h
        .frame(vec![Event::PointerMoved(from), press(from, true)])
        .is_empty());
    for step in 1..=10 {
        let p = from + (target - from) * (step as f32 / 10.0);
        assert!(h.frame(vec![Event::PointerMoved(p)]).is_empty());
        let delta = h.editor.ui_editor.drag_doc_delta().unwrap();
        let expected = (p - from) / scale;
        assert!(
            (delta - expected).length() < 1e-3,
            "{delta:?} != {expected:?}"
        );
    }
    let released = h.frame(vec![press(target, false)]);
    let edits = h.commit(released);
    assert_eq!(edits.len(), 1);
    assert_eq!(h.world(), acl(&top, "ui place affection top_right"));
    // Limits, arguments and the readout (= fit); none of it writes.
    let center = view.center();
    let mut actions = h.frame(vec![Event::PointerMoved(center)]);
    for _ in 0..12 {
        actions.extend(h.frame(vec![wheel(-400.0)]));
    }
    assert!(actions.is_empty());
    assert_eq!(h.editor.ui_editor.zoom(), super::ui_editor::ZOOM_MIN);
    for _ in 0..10 {
        h.editor.run_command_with("ui.zoom", Some("in")).unwrap();
    }
    assert_eq!(h.editor.ui_editor.zoom(), super::ui_editor::ZOOM_MAX);
    for bad in ["3", "10%", "big", "nan"] {
        assert!(
            h.editor.run_command_with("ui.zoom", Some(bad)).is_err(),
            "{bad}"
        );
    }
    assert!(h.click("ui_editor:zoom").is_empty());
    assert_eq!(h.editor.ui_editor.zoom(), 1.0, "the readout fits");
    h.frame(vec![]);
    assert_eq!(h.rect("ui_editor:canvas"), view);
    h.editor.run_command_with("ui.zoom", Some("1.5")).unwrap();
    h.editor.run_command_with("ui.zoom", Some("fit")).unwrap();
    h.frame(vec![]);
    assert_eq!(h.rect("ui_editor:canvas"), view);
    assert_eq!(h.world(), acl(&top, "ui place affection top_right"));
    // Registered like graph.zoom; without an argument it only shows the tab.
    assert!(super::commands::command("ui.zoom").is_some());
    assert!(h
        .editor
        .run_command_with("ui.zoom", None)
        .unwrap()
        .is_empty());
}

#[test]
fn missing_strings_are_counted_per_locale() {
    let mut world = town();
    let doc = world.resources.get_mut("ui.document").unwrap();
    doc["catalogs"]["en"]["title"] = serde_json::json!("");
    doc["catalogs"]["en"]["town"] = serde_json::json!("town");
    let mut h = Harness::new(world);
    assert!(h.shows("no missing strings"), "{:?}", h.shown);
    h.click("ui_locale:en");
    h.frame(vec![]);
    assert!(h.shows("2 missing string(s)"), "{:?}", h.shown);
}

/// R8 F4: a revision from elsewhere (an agent's unrelated edit) keeps the form value being
/// edited; a revision that changed the same effect drops it with an error naming it.
#[test]
fn a_revision_from_elsewhere_keeps_an_unrelated_ui_editor_draft() {
    let start = acl(&town(), "ui effect add coins.pulse coins pulse");
    let mut h = Harness::new(start.clone());
    assert!(h.click("ui_canvas:coins").is_empty());
    assert!(h.click("ui_effect:coins.pulse").is_empty());
    h.frame(vec![]);
    let agent = |h: &mut Harness, edit: &str| {
        h.store
            .execute(Request::Acl {
                text: edit.into(),
                preview: false,
                expected_rev: None,
                propose: false,
            })
            .unwrap();
        let (rev, world) = h.store.current().unwrap();
        h.editor.update(rev, world, false);
    };
    // Typed, not committed yet (the form commits when nothing is held or focused).
    h.editor
        .ui_effect_draft_for_test()
        .as_mut()
        .expect("effect form open")
        .ticks = Some(90);
    h.store
        .apply(
            None,
            vec![Edit::set("title", serde_json::json!("AI title"))],
        )
        .unwrap();
    let (rev, world) = h.store.current().unwrap();
    h.editor.update(rev, world, false);
    let kept = h.frame(vec![]);
    h.commit(kept);
    assert_eq!(
        h.world().resources["ui.document"]["effects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "coins.pulse")
            .unwrap()["ticks"],
        90
    );
    assert_eq!(h.world().title, "AI title");
    // The same effect changed elsewhere: the draft is not written over it.
    h.frame(vec![]);
    h.editor
        .ui_effect_draft_for_test()
        .as_mut()
        .expect("effect form open")
        .ticks = Some(240);
    agent(&mut h, "ui effect set coins.pulse ticks=60");
    assert!(h.frame(vec![]).is_empty());
    assert!(
        h.editor.error && h.editor.status.contains("coins.pulse"),
        "{}",
        h.editor.status
    );
}

/// Plan M3: the canvas draws panels with the document's reference scale, so a panel widget
/// sits at the same fraction of the canvas at 1280x720 and 1920x1080 (16:9 both), and the
/// canvas rects it reports are where it is drawn.
#[test]
fn canvas_panels_scale_with_the_reference_screen() {
    let mut h = Harness::new(town());
    // A grid settles its column widths over a frame and an anchored area is placed from its
    // previous size: let both settle before measuring.
    h.frame(vec![]);
    h.frame(vec![]);
    let fraction = |h: &Harness, key: &str| {
        let canvas = h.rect("ui_editor:canvas");
        let r = h.rect(key);
        [
            (r.min.x - canvas.min.x) / canvas.width(),
            (r.min.y - canvas.min.y) / canvas.height(),
            r.width() / canvas.width(),
            r.height() / canvas.height(),
        ]
    };
    let small = fraction(&h, "ui_canvas:emote_wave");
    let line_small = fraction(&h, "ui_canvas:line");
    h.editor
        .run_command_with("ui.resolution", Some("1920x1080"))
        .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    let large = fraction(&h, "ui_canvas:emote_wave");
    let line_large = fraction(&h, "ui_canvas:line");
    for (a, b) in [(small, large), (line_small, line_large)] {
        for k in 0..4 {
            assert!((a[k] - b[k]).abs() < 0.002, "{a:?} vs {b:?}");
        }
    }
    // The emote grid is on the right half, the dialog line in the lower part.
    assert!(
        small[0] > 0.5 && line_small[1] > 0.6,
        "{small:?} {line_small:?}"
    );
}

/// Plan M5: template instances (the shipped town's emote buttons) draw on the canvas by the
/// game's code but are edited through their template / use, so they cannot be picked or
/// dragged; authored widgets still can.
#[test]
fn template_instances_draw_but_are_not_selectable_on_the_canvas() {
    let mut h = Harness::new(raw_town());
    h.frame(vec![]);
    assert!(
        h.shows("揮手"),
        "the emote buttons are drawn: {:?}",
        h.shown
    );
    assert!(
        !h.editor.rects.contains_key("ui_canvas:emote_wave"),
        "an instance is not a canvas target"
    );
    assert!(h.editor.rects.contains_key("ui_canvas:affection"));
}

/// Plan M7 (demo step 7): drag the dialog panel and resize it from its edge handle on the
/// canvas; each release writes one revision equal to the ACL `ui panel` line, positions snap
/// to 8 reference pixels, and nothing is written while the pointer is held.
#[test]
fn dragging_and_resizing_a_panel_on_the_canvas_matches_acl() {
    let start = town();
    let mut h = Harness::new(start.clone());
    h.frame(vec![]);
    h.frame(vec![]);
    let view = h.rect("ui_editor:view");
    let fit = view.width() / 1280.0; // the town reference is 1280x720 = the default canvas
    let drag = |h: &mut Harness, from: Pos2, by: egui::Vec2| -> Vec<Action> {
        assert!(h
            .frame(vec![Event::PointerMoved(from), press(from, true)])
            .is_empty());
        for step in 1..=8 {
            let p = from + by * (step as f32 / 8.0);
            assert!(
                h.frame(vec![Event::PointerMoved(p)]).is_empty(),
                "no write while held"
            );
        }
        h.frame(vec![press(from + by, false)])
    };
    // Move: (+40, -40) document pixels from the dialog's centre; at=0,-14 -> 40,-54 -> snapped.
    let dialog = h.rect("ui_panel:dialog");
    let moved = drag(&mut h, dialog.center(), vec2(40.0, -40.0) * fit);
    h.commit(moved);
    assert_eq!(h.editor.ui_editor.selected_panel.as_deref(), Some("dialog"));
    let step1 = acl(
        &start,
        "ui panel dialog bottom at=40,-56 width=900 style=dialog widgets=speaker,line,choices",
    );
    assert_eq!(h.world(), step1);
    // Resize from the right handle (+30 px): a centred panel grows on both sides, 900 -> 960.
    h.frame(vec![]);
    let handle = h.rect("ui_panel_handle:dialog");
    let resized = drag(&mut h, handle.center(), vec2(30.0, 0.0) * fit);
    h.commit(resized);
    assert_eq!(
        h.world(),
        acl(
            &step1,
            "ui panel dialog bottom at=40,-56 width=960 style=dialog widgets=speaker,line,choices",
        )
    );
}

/// Plan M7: the selected panel's style (padding / corner steps) and its frame's nine-slice
/// margins (guides dragged on the frame image) edit through the same ops as ACL, one
/// revision per click / release.
#[test]
fn panel_style_steps_and_nine_slice_guides_match_acl() {
    let start = town();
    let mut h = Harness::new(start.clone());
    h.frame(vec![]);
    h.frame(vec![]);
    // A click (no movement) selects the panel and writes nothing.
    let at = h.rect("ui_panel:dialog").center();
    assert!(h
        .frame(vec![Event::PointerMoved(at), press(at, true)])
        .is_empty());
    assert!(h.frame(vec![press(at, false)]).is_empty());
    h.frame(vec![]);
    assert_eq!(h.editor.ui_editor.selected_panel.as_deref(), Some("dialog"));
    // Left guide 32 -> 40 source pixels (the guides are only grabbed where the panel shows
    // them: a press outside the visible part of the scrolling panel is ignored).
    h.frame(vec![]);
    let zoom = h.rect("ui_slice:box").width() / 96.0;
    let from = h.rect("ui_slice:left").center();
    let to = from + vec2(8.0 * zoom, 0.0);
    assert!(h
        .frame(vec![Event::PointerMoved(from), press(from, true)])
        .is_empty());
    for step in 1..=6 {
        let p = from + (to - from) * (step as f32 / 6.0);
        assert!(
            h.frame(vec![Event::PointerMoved(p)]).is_empty(),
            "no write while held"
        );
    }
    assert!(
        h.shows("slice 40 32 32 32"),
        "the guide follows the pointer"
    );
    let released = h.frame(vec![press(to, false)]);
    h.commit(released);
    let step1 = acl(&start, "ui frame dialog dialog 40 32 32 32");
    assert_eq!(h.world(), step1);
    // Padding + (26 -> 28).
    let padded = h.click("ui_style:padding:+");
    h.commit(padded);
    assert_eq!(
        h.world(),
        acl(&step1, "ui panelstyle dialog padding=28 frame=dialog")
    );
}
