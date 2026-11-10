//! UI Editor driven by real egui input on Axiom Town: canvas drag between regions, the
//! element panel's dropdowns and effects, locale preview and effect preview. Every write is
//! compared with the ACL `ui ...` line an agent would send.
use super::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::{Edit, Request, Store, World};
use egui::{epaint::Shape, vec2, Event, Modifiers, PointerButton, Pos2, Rect};

fn town() -> World {
    axiom_core::ContentPackage::load(
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
