//! UI leftovers (docs/plan-player-settings.zh-TW.md U1) in the town: dialogue choices take a
//! button class (not another kind), and engine strings come from the catalogs.
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, WorldRuntime};

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

#[test]
fn choices_take_a_button_class_only() {
    let w = town();
    let doc = &w.resources["ui.document"];
    assert_eq!(doc["classes"]["choices"], "emote");
    WorldRuntime::new(w.clone(), 1, installed_registry()).unwrap();
    let mut wrong = w;
    wrong.resources.get_mut("ui.document").unwrap()["classes"]["choices"] = "affection".into();
    let msg = format!(
        "{:#}",
        WorldRuntime::new(wrong, 1, installed_registry())
            .err()
            .unwrap()
    );
    assert!(msg.contains("/classes/choices"), "{msg}");
}

fn package(name: &str) -> World {
    ContentPackage::load(
        format!("{}/../../content/{name}", env!("CARGO_MANIFEST_DIR")),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// Every shape of a frame, nested ones included.
fn flat(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
    match shape {
        egui::Shape::Vec(list) => list.iter().for_each(|s| flat(s, out)),
        s => out.push(s.clone()),
    }
}

#[test]
fn the_default_prompt_takes_the_theme_prompt_look() {
    // The street without its own prompt widget: the engine draws the default prompt.
    let mut w = package("street");
    let ming = w.entities.iter_mut().find(|e| e.id == "npc/ming").unwrap();
    ming.components.get_mut("interact.target").unwrap()["range"] = serde_json::json!(100.0);
    let doc = w.resources.get_mut("ui.document").unwrap();
    doc["widgets"]
        .as_array_mut()
        .unwrap()
        .retain(|x| x["id"] != "prompt");
    doc["regions"].as_object_mut().unwrap().remove("prompt");
    doc["theme"] = serde_json::json!({
        "text": {"hint": {"size": 22, "color": [255, 224, 160, 255]}},
        "panel": {"hint_box": {"fill": [16, 24, 32, 224], "corner": 10, "padding": 14}},
        "prompt": {"class": "hint", "style": "hint_box"}
    });
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    for _ in 0..4 {
        r.step(&axiom_runtime::ActionMap::default());
    }
    assert!(
        r.state.gameplay.as_ref().unwrap().prompt.is_some(),
        "the player is in range of Ming"
    );
    let ctx = egui::Context::default();
    // Areas fade in over time; this test has no clock, so no animation.
    ctx.style_mut(|s| s.animation_time = 0.0);
    let mut ui = axiom_runtime::ui::UiState {
        playing: true,
        ..Default::default()
    };
    let mut shapes = vec![];
    for _ in 0..3 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(raw, |ctx| {
            r.draw_ui(ctx, &mut ui);
        });
        shapes.clear();
        out.shapes.iter().for_each(|c| flat(&c.shape, &mut shapes));
    }
    let fill = egui::Color32::from_rgba_unmultiplied(16, 24, 32, 224);
    assert!(
        shapes
            .iter()
            .any(|s| matches!(s, egui::Shape::Rect(rect) if rect.fill == fill)),
        "the prompt box has the panel style's fill: {:?}",
        shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) => Some(format!("rect {:?}", r.fill)),
                egui::Shape::Text(t) =>
                    Some(format!("text {} {:?}", t.galley.text(), t.fallback_color)),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
    let gold = egui::Color32::from_rgba_unmultiplied(255, 224, 160, 255);
    assert!(
        shapes.iter().any(|s| matches!(s,
            egui::Shape::Text(t) if t.galley.text().contains("[E]") && t.fallback_color == gold)),
        "the prompt text has the text class colour"
    );
}
