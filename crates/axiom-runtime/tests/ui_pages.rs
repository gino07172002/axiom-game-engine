//! ui.document pages (plan M6): the Axiom Town pause menu opens with a typed command (the
//! host maps Esc / gamepad Start to it), stops the fixed step, blocks the HUD, takes arrow-key
//! focus, and is saved in checkpoints.
use axiom_core::gameplay::RuntimeCommand;
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, ui::Action, ui::UiState, ActionMap, WorldRuntime};
use egui::{Pos2, Rect, Vec2};

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn open() -> RuntimeCommand {
    RuntimeCommand::Open {
        page: "pause".into(),
    }
}

fn pages(r: &WorldRuntime) -> serde_json::Value {
    r.observation().metrics["gameplay"]["pages"].clone()
}

/// One HUD frame with `keys` pressed; returns the actions.
fn frame(
    r: &WorldRuntime,
    ctx: &egui::Context,
    ui: &mut UiState,
    keys: &[egui::Key],
) -> Vec<Action> {
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
        events: keys
            .iter()
            .map(|&key| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect(),
        ..Default::default()
    };
    let mut actions = vec![];
    let _ = ctx.run(raw, |ctx| actions = r.draw_ui(ctx, ui));
    actions
}

fn focused(r: &WorldRuntime, ui: &UiState) -> String {
    let i = ui.focus.expect("a focused button");
    let _ = r;
    // Compiled order = authored widgets, then template instances; the pause buttons are authored.
    let doc = &town().resources["ui.document"];
    let ids: Vec<String> = doc["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap().to_owned())
        .collect();
    ids[i].clone()
}

#[test]
fn the_pause_page_stops_the_fixed_step_until_it_closes() {
    let mut r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    assert_eq!(r.escape_page(), Some(("pause".into(), false)));
    assert_eq!(pages(&r), serde_json::json!([]));
    r.command(&open()).unwrap();
    assert_eq!(pages(&r), serde_json::json!(["pause"]));
    assert!(r.ui_paused());
    let tick = r.state.tick;
    for _ in 0..30 {
        r.step(&ActionMap::default());
    }
    assert_eq!(r.state.tick, tick, "paused: no fixed steps");
    // Closing (the page's own button, or Esc again) resumes.
    r.command(&RuntimeCommand::Close { page: None }).unwrap();
    assert!(!r.ui_paused());
    r.step(&ActionMap::default());
    assert_eq!(r.state.tick, tick + 1);
    assert!(
        r.command(&RuntimeCommand::Close { page: None }).is_err(),
        "nothing open"
    );
    assert!(r
        .command(&RuntimeCommand::Open {
            page: "nope".into()
        })
        .is_err());
}

#[test]
fn the_open_modal_page_blocks_the_hud_and_takes_arrow_key_focus() {
    let mut r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    r.command(&open()).unwrap();
    let ctx = egui::Context::default();
    let mut ui = UiState {
        playing: true,
        ..Default::default()
    };
    for _ in 0..4 {
        frame(&r, &ctx, &mut ui, &[]);
    }
    assert!(ui.blocked);
    assert_eq!(
        focused(&r, &ui),
        "pause_resume",
        "focus starts on the first button"
    );
    // An emote button under the backdrop does nothing when clicked.
    let emote = ui.targets["button/emote_wave"];
    let click = |pressed| egui::Event::PointerButton {
        pos: emote.center(),
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut clicked = vec![];
    for events in [
        vec![egui::Event::PointerMoved(emote.center()), click(true)],
        vec![click(false)],
    ] {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| clicked.extend(r.draw_ui(ctx, &mut ui)));
    }
    assert!(clicked.is_empty(), "HUD blocked: {} actions", clicked.len());
    // Down moves to English; Enter fires it (locale en).
    frame(&r, &ctx, &mut ui, &[egui::Key::ArrowDown]);
    assert_eq!(focused(&r, &ui), "pause_en");
    let fired = frame(&r, &ctx, &mut ui, &[egui::Key::Enter]);
    assert!(
        matches!(fired.as_slice(), [Action::Gameplay(RuntimeCommand::Locale { locale })] if locale == "en"),
        "{} actions",
        fired.len()
    );
    // Up wraps back to Resume; Enter closes the page.
    frame(&r, &ctx, &mut ui, &[egui::Key::ArrowUp]);
    assert_eq!(focused(&r, &ui), "pause_resume");
    let fired = frame(&r, &ctx, &mut ui, &[egui::Key::Enter]);
    let [Action::Gameplay(close)] = fired.as_slice() else {
        panic!("{} actions", fired.len())
    };
    r.command(close).unwrap();
    assert_eq!(pages(&r), serde_json::json!([]));
    frame(&r, &ctx, &mut ui, &[]);
    assert!(!ui.blocked && ui.focus.is_none());
}

/// The ADV demo (it enables saves) with a pause page holding one new text widget.
fn adv_with_a_page() -> World {
    let mut world = ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/adv"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world;
    let doc = world.resources.get_mut("ui.document").unwrap();
    doc["widgets"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(
            {"kind": "text", "id": "menu_title", "binding": {"kind": "key", "id": "title"}}
        ));
    doc["panels"] =
        serde_json::json!([{"id": "menu", "anchor": "center", "children": ["menu_title"]}]);
    doc["pages"] = serde_json::json!({"pause": {"panels": ["menu"], "pause": true}});
    world
}

#[test]
fn checkpoints_keep_the_open_page() {
    let mut r = WorldRuntime::new(adv_with_a_page(), 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    r.command(&open()).unwrap();
    let saved = r.checkpoint().unwrap();
    r.command(&RuntimeCommand::Close { page: None }).unwrap();
    assert_eq!(pages(&r), serde_json::json!([]));
    r.restore_checkpoint(&saved).unwrap();
    assert_eq!(pages(&r), serde_json::json!(["pause"]));
    assert!(r.ui_paused());
    // A save that names a page the document does not have is refused, not silently dropped.
    let mut bad = saved.clone();
    bad.gameplay.as_mut().unwrap().pages = vec!["ghost".into()];
    let error = r.restore_checkpoint(&bad).expect_err("refused");
    assert!(
        format!("{error:#}").contains("save page missing"),
        "{error:#}"
    );
}

#[test]
fn an_open_modal_page_dims_the_screen_behind_it() {
    let mut r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    r.command(&open()).unwrap();
    let ctx = egui::Context::default();
    let mut ui = UiState {
        playing: true,
        ..Default::default()
    };
    let raw = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
        ..Default::default()
    };
    let mut out = None;
    for _ in 0..4 {
        out = Some(ctx.run(raw(), |ctx| {
            let _ = r.draw_ui(ctx, &mut ui);
        }));
    }
    let out = out.unwrap();
    let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
    // A dark translucent quad over the whole window.
    let dim = primitives.iter().any(|p| match &p.primitive {
        egui::epaint::Primitive::Mesh(m) => {
            let r = Rect::from_points(&m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>());
            // (anti-aliasing adds a transparent feather ring around the filled quad)
            r.width() >= 1279.0
                && r.height() >= 719.0
                && m.vertices
                    .iter()
                    .any(|v| v.color.r() == 0 && (100..=140).contains(&v.color.a()))
        }
        _ => false,
    });
    assert!(dim, "no full-window backdrop");
}
