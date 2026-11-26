//! ui.document v2 panels (plan M1b), drawn headless: the Axiom Town dialog sits in a framed
//! bottom panel (nine-slice `ui/dialog-frame.png`) instead of the old bottom region.
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, ui::UiState, ActionMap, WorldRuntime};
use egui::{epaint::Mesh, Pos2, Rect, Shape, Vec2};
use std::collections::BTreeMap;

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// Walks the hero to Lumi and talks (the town-dance scenario's opening).
fn talking() -> WorldRuntime {
    let mut r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    let walk = ActionMap(BTreeMap::from([
        ("move_z".into(), 1.0),
        ("move_x".into(), 0.45),
    ]));
    for _ in 0..250 {
        r.step(&walk);
    }
    for _ in 0..50 {
        r.step(&ActionMap::default());
    }
    let talk = ActionMap(BTreeMap::from([("interact".into(), 1.0)]));
    for _ in 0..2 {
        r.step(&talk);
    }
    for _ in 0..20 {
        r.step(&ActionMap::default());
    }
    assert_eq!(
        r.observation().metrics["gameplay"]["node"],
        "lumi",
        "the dialog is open"
    );
    r
}

fn meshes(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<Mesh>) {
    fn walk(s: &Shape, out: &mut Vec<Mesh>) {
        match s {
            Shape::Mesh(m) => out.push((**m).clone()),
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    shapes.iter().for_each(|c| walk(&c.shape, out));
}

#[test]
fn the_town_dialog_draws_in_a_framed_bottom_panel() {
    let r = talking();
    let ctx = egui::Context::default();
    let mut ui = UiState::default();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0))),
        ..Default::default()
    };
    let mut out = None;
    // New areas are sized on their first frame and drawn from the next one.
    for _ in 0..3 {
        out = Some(ctx.run(input(), |ctx| {
            let _ = r.draw_ui(ctx, &mut ui);
        }));
    }
    let out = out.unwrap();
    let mut found = vec![];
    meshes(&out.shapes, &mut found);
    // The nine-slice background: 9 quads = 36 vertices on a managed (non-font) texture.
    let frames: Vec<Rect> = found
        .iter()
        .filter(|m| m.vertices.len() == 36 && m.texture_id != egui::TextureId::default())
        .map(|m| Rect::from_points(&m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>()))
        .collect();
    assert_eq!(frames.len(), 1, "one framed panel: {frames:?}");
    let frame = frames[0];
    assert!(
        (frame.width() - 900.0).abs() < 1.0,
        "authored width 900: {frame:?}"
    );
    assert!(
        (frame.center().x - 960.0).abs() < 1.0,
        "centred at the bottom: {frame:?}"
    );
    assert!(
        (1080.0 - frame.bottom() - 14.0).abs() < 1.0,
        "14 px above the bottom: {frame:?}"
    );
    // The choice buttons are inside the frame (they are the panel's last child).
    let choices: Vec<&Rect> = ui
        .targets
        .iter()
        .filter(|(k, _)| k.starts_with("choice"))
        .map(|(_, r)| r)
        .collect();
    assert!(
        !choices.is_empty(),
        "choice targets: {:?}",
        ui.targets.keys().collect::<Vec<_>>()
    );
    for c in choices {
        assert!(
            frame.contains_rect(*c),
            "choice {c:?} outside the frame {frame:?}"
        );
    }
}

#[test]
fn session_controls_stay_on_screen_when_every_widget_is_in_a_region_or_panel() {
    // Regression (M1b): with the dialog widgets moved into a panel, no bottom document panel
    // is drawn; the locale / save / load controls must then sit in the top-right region.
    let r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    let ctx = egui::Context::default();
    let mut ui = UiState::default();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0))),
        ..Default::default()
    };
    let mut out = None;
    for _ in 0..3 {
        out = Some(ctx.run(input(), |ctx| {
            let _ = r.draw_ui(ctx, &mut ui);
        }));
    }
    let texts: Vec<String> = out
        .unwrap()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    for label in ["SAVE / 存檔", "LOAD / 讀檔"] {
        assert!(
            texts.iter().any(|t| t == label),
            "{label} missing from {texts:?}"
        );
    }
}
