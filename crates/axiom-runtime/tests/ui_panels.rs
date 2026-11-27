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
        // The emote buttons draw their own (small) frames since M2, the tracker (top left)
        // has its own frame since M8 and the dialogue choices use the emote button class
        // since U1 (wide but short): the dialog is the tall framed panel in the lower half.
        .filter(|r| r.width() > 400.0 && r.height() > 120.0 && r.center().y > 540.0)
        .collect();
    assert_eq!(frames.len(), 1, "one framed panel: {frames:?}");
    let frame = frames[0];
    // Authored in the 1280x720 reference (plan M3): 1080 / 720 = 1.5x in this window.
    assert!(
        (frame.width() - 900.0 * 1.5).abs() < 1.0,
        "authored width 900 x 1.5: {frame:?}"
    );
    assert!(
        (frame.center().x - 960.0).abs() < 1.0,
        "centred at the bottom: {frame:?}"
    );
    assert!(
        (1080.0 - frame.bottom() - 14.0 * 1.5).abs() < 1.0,
        "14 reference px above the bottom: {frame:?}"
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
    // The town overrides the engine strings with axiom.save / axiom.load (plan U1).
    for label in ["存檔", "讀檔"] {
        assert!(
            texts.iter().any(|t| t == label),
            "{label} missing from {texts:?}"
        );
    }
}

/// Draws the town HUD `frames` times with the given pointer input and returns the shapes.
fn frame_with(
    r: &WorldRuntime,
    ctx: &egui::Context,
    ui: &mut UiState,
    events: Vec<egui::Event>,
) -> Vec<egui::epaint::ClippedShape> {
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0))),
        events,
        ..Default::default()
    };
    ctx.run(raw, |ctx| {
        let _ = r.draw_ui(ctx, ui);
    })
    .shapes
}

/// Whether a (premultiplied) drawn colour is `rgb` at some opacity: a disabled UI is drawn
/// faded, so each channel is the authored value times the alpha (within rounding).
fn is_rgb(c: egui::Color32, rgb: [u8; 3]) -> bool {
    let a = c.a() as f32 / 255.0;
    c.a() > 0
        && [c.r(), c.g(), c.b()]
            .iter()
            .zip(rgb)
            .all(|(&got, want)| (got as f32 - want as f32 * a).abs() <= 2.0)
}

/// Texture of the 36-vertex nine-slice drawn inside `area`.
fn frame_texture_in(shapes: &[egui::epaint::ClippedShape], area: Rect) -> Option<egui::TextureId> {
    let mut found = vec![];
    meshes(shapes, &mut found);
    found
        .iter()
        .filter(|m| m.vertices.len() == 36)
        .find(|m| m.vertices.iter().all(|v| area.expand(1.0).contains(v.pos)))
        .map(|m| m.texture_id)
}

#[test]
fn emote_buttons_switch_frames_for_hover_and_press() {
    let r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    let ctx = egui::Context::default();
    let mut ui = UiState {
        playing: true,
        ..Default::default()
    };
    for _ in 0..6 {
        frame_with(&r, &ctx, &mut ui, vec![]);
    }
    let button = ui.targets["button/emote_wave"];
    let normal =
        frame_texture_in(&frame_with(&r, &ctx, &mut ui, vec![]), button).expect("normal frame");
    let at = button.center();
    let hover = frame_texture_in(
        &frame_with(&r, &ctx, &mut ui, vec![egui::Event::PointerMoved(at)]),
        button,
    )
    .expect("hover frame");
    let press = egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    };
    frame_with(&r, &ctx, &mut ui, vec![press]);
    let pressed =
        frame_texture_in(&frame_with(&r, &ctx, &mut ui, vec![]), button).expect("pressed frame");
    assert_ne!(normal, hover, "hover shows its own frame");
    assert_ne!(hover, pressed, "pressed shows its own frame");
    assert_ne!(normal, pressed);
}

#[test]
fn a_stopped_game_shows_buttons_in_the_disabled_text_colour() {
    let r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    let ctx = egui::Context::default();
    let mut ui = UiState::default(); // not playing: buttons cannot be used
    let mut shapes = vec![];
    for _ in 0..3 {
        shapes = frame_with(&r, &ctx, &mut ui, vec![]);
    }
    let colours: Vec<egui::Color32> = shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Text(t) if t.galley.text().contains("揮手") => Some(t.fallback_color),
            _ => None,
        })
        .collect();
    assert!(
        colours.iter().any(|c| is_rgb(*c, [150, 140, 130])),
        "{colours:?}"
    );
}

#[test]
fn the_speaker_name_uses_its_text_class_with_a_shadow() {
    let r = talking();
    let ctx = egui::Context::default();
    let mut ui = UiState::default();
    let mut shapes = vec![];
    for _ in 0..3 {
        shapes = frame_with(&r, &ctx, &mut ui, vec![]);
    }
    let speaker = r.observation().metrics["gameplay"]["bindings"]["speaker"]
        .as_str()
        .map(str::to_owned);
    let texts: Vec<(Pos2, egui::Color32, String)> = shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Text(t) => Some((t.pos, t.fallback_color, t.galley.text().to_owned())),
            _ => None,
        })
        .collect();
    let main = texts
        .iter()
        .find(|(_, c, _)| is_rgb(*c, [233, 196, 120]))
        .unwrap_or_else(|| panic!("no gold speaker text (speaker {speaker:?}): {texts:?}"));
    assert!(
        texts.iter().any(|(p, c, t)| is_rgb(*c, [0, 0, 0])
            && *t == main.2
            // 2 reference px, drawn in the panel layer scaled 1080 / 720 = 1.5x.
            && (*p - (main.0 + Vec2::splat(3.0))).length() < 0.01),
        "shadow 2 x 1.5 px down-right of {main:?}"
    );
}

/// Draws the HUD `n` times in a `w` x `h` window and returns the last frame's shapes.
fn frames_at(
    r: &WorldRuntime,
    ctx: &egui::Context,
    ui: &mut UiState,
    [w, h]: [f32; 2],
    n: usize,
    events: Vec<egui::Event>,
) -> Vec<egui::epaint::ClippedShape> {
    let mut shapes = vec![];
    for k in 0..n {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h))),
            events: if k + 1 == n { events.clone() } else { vec![] },
            ..Default::default()
        };
        shapes = ctx
            .run(raw, |ctx| {
                let _ = r.draw_ui(ctx, ui);
            })
            .shapes;
    }
    shapes
}

fn dialog_frame(shapes: &[egui::epaint::ClippedShape]) -> Rect {
    let mut found = vec![];
    meshes(shapes, &mut found);
    let frames: Vec<Rect> = found
        .iter()
        .filter(|m| m.vertices.len() == 36 && m.texture_id != egui::TextureId::default())
        .map(|m| Rect::from_points(&m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>()))
        // The dialog is the wide, tall framed panel in the lower half (the tracker is top
        // left; the choice buttons are framed too since U1 but short).
        .filter(|r| r.width() > 200.0 && r.height() > 120.0 && r.center().y > r.height())
        .collect();
    assert_eq!(frames.len(), 1, "{frames:?}");
    frames[0]
}

#[test]
fn the_dialog_keeps_its_proportions_at_720p_and_1080p() {
    // theme.reference = 1280x720, fit_height: the panel layer scales by window height / 720.
    let r = talking();
    let mut seen = vec![];
    for size in [[1920.0, 1080.0], [1280.0, 720.0]] {
        let ctx = egui::Context::default();
        let mut ui = UiState::default();
        let frame = dialog_frame(&frames_at(&r, &ctx, &mut ui, size, 3, vec![]));
        let s = size[1] / 720.0;
        assert!(
            (frame.width() - 900.0 * s).abs() < 1.0,
            "{size:?}: {frame:?}"
        );
        assert!(
            (frame.center().x - size[0] / 2.0).abs() < 1.0,
            "{size:?}: {frame:?}"
        );
        assert!(
            (size[1] - frame.bottom() - 14.0 * s).abs() < 1.0,
            "{size:?}: {frame:?}"
        );
        seen.push((frame.width() / size[0], frame.height() / size[1]));
        // The choice buttons are clickable where they are drawn: their targets sit inside.
        for (k, c) in ui.targets.iter().filter(|(k, _)| k.starts_with("choice")) {
            assert!(
                frame.contains_rect(*c),
                "{size:?} {k} {c:?} outside {frame:?}"
            );
        }
    }
    assert!(
        (seen[0].0 - seen[1].0).abs() < 0.002 && (seen[0].1 - seen[1].1).abs() < 0.002,
        "same proportions of the window: {seen:?}"
    );
}

#[test]
fn emotes_sit_in_a_three_column_grid_and_observe_reports_the_layout() {
    let r = talking();
    let ctx = egui::Context::default();
    let mut ui = UiState::default();
    frames_at(&r, &ctx, &mut ui, [1920.0, 1080.0], 3, vec![]);
    let layout = r.ui_layout_observation(&ui).expect("ui_layout");
    let widgets = layout["widgets"].as_array().unwrap();
    let rect = |id: &str| -> [f64; 4] {
        let w = widgets
            .iter()
            .find(|w| w["id"] == id)
            .unwrap_or_else(|| panic!("{id} in {layout}"));
        assert_eq!(
            w["panel"],
            if id.starts_with("emote") {
                "emotes"
            } else {
                "dialog"
            }
        );
        serde_json::from_value(w["rect"].clone()).unwrap()
    };
    let emotes = [
        "emote_wave",
        "emote_bow",
        "emote_dance",
        "emote_cheer",
        "emote_pose",
        "emote_think",
    ];
    let rects: Vec<[f64; 4]> = emotes.iter().map(|id| rect(id)).collect();
    let rows: std::collections::BTreeSet<i64> = rects.iter().map(|r| r[1].round() as i64).collect();
    let cols: std::collections::BTreeSet<i64> = rects.iter().map(|r| r[0].round() as i64).collect();
    assert_eq!(
        (rows.len(), cols.len()),
        (2, 3),
        "2 rows x 3 columns: {rects:?}"
    );
    // Row-major: wave, bow, dance on the first row.
    assert!(rects[0][1] < rects[3][1] && rects[0][0] < rects[1][0] && rects[1][0] < rects[2][0]);
    rect("line");
    let panels: Vec<&str> = layout["panels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    // Child panels are recorded before their parent (coins_row sits in tracker).
    assert_eq!(
        panels,
        ["coins_row", "tracker", "prompt_bar", "dialog", "emotes"],
        "{layout}"
    );
    assert_eq!(layout["scale"], 1.5, "1080 / the 720 reference");
    assert_eq!(
        layout["diagnostics"],
        serde_json::json!([]),
        "no overlap or off-screen HUD: {layout}"
    );
}

#[test]
fn observe_reports_overlapping_and_offscreen_panels() {
    // Put the emote grid onto the dialog and make it wider than the window (egui keeps an
    // area that fits inside the window, so off-screen means "larger than the window").
    let mut world = town();
    let doc = world.resources.get_mut("ui.document").unwrap();
    let emotes = doc["panels"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["id"] == "emotes")
        .unwrap();
    emotes["anchor"] = serde_json::json!("bottom");
    emotes["offset"] = serde_json::json!([0, -20]);
    emotes["width"] = serde_json::json!(4000);
    let mut r = WorldRuntime::new(world, 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    let ctx = egui::Context::default();
    let mut ui = UiState::default();
    frames_at(&r, &ctx, &mut ui, [1920.0, 1080.0], 3, vec![]);
    let layout = r.ui_layout_observation(&ui).expect("ui_layout");
    let codes: Vec<(String, String)> = layout["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["code"].as_str().unwrap().to_owned(), d.to_string()))
        .collect();
    assert!(
        codes
            .iter()
            .any(|(c, d)| c == "ui_offscreen" && d.contains("panel:emotes")),
        "{codes:?}"
    );
    assert!(
        codes.iter().any(|(c, d)| c == "ui_overlap"
            && d.contains("panel:emotes")
            && d.contains("panel:dialog")),
        "{codes:?}"
    );
}

#[test]
fn scaled_emote_buttons_still_hover_and_press_where_they_are_drawn() {
    // At 1280x720 the panel layer is scaled by 2/3; targets and hit tests use screen space.
    let r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    let ctx = egui::Context::default();
    let mut ui = UiState {
        playing: true,
        ..Default::default()
    };
    let size = [1280.0, 720.0];
    // The content font arrives on frame 2 and the grid settles its columns after it.
    frames_at(&r, &ctx, &mut ui, size, 6, vec![]);
    let button = ui.targets["button/emote_wave"];
    assert!(
        Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0)).contains_rect(button),
        "{button:?}"
    );
    let normal = frame_texture_in(&frames_at(&r, &ctx, &mut ui, size, 1, vec![]), button)
        .expect("normal frame");
    let at = button.center();
    let hover = frame_texture_in(
        &frames_at(
            &r,
            &ctx,
            &mut ui,
            size,
            1,
            vec![egui::Event::PointerMoved(at)],
        ),
        button,
    )
    .expect("hover frame");
    assert_ne!(normal, hover, "hovering the drawn button highlights it");
    let press = egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    };
    let release = egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    };
    frames_at(&r, &ctx, &mut ui, size, 1, vec![press]);
    let ctx2 = ctx.clone();
    let mut clicked = vec![];
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
        events: vec![release],
        ..Default::default()
    };
    let _ = ctx2.run(raw, |ctx| clicked = r.draw_ui(ctx, &mut ui));
    assert!(
        clicked
            .iter()
            .any(|a| matches!(a, axiom_runtime::ui::Action::Gameplay(_))),
        "the click reaches the scaled button"
    );
}

/// The families of every text shape drawn with `text`.
fn families_of(shapes: &[egui::epaint::ClippedShape], text: &str) -> Vec<egui::FontFamily> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Text(t) if t.galley.text() == text => {
                Some(t.galley.job.sections[0].format.font_id.family.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_dialog_line_switches_to_the_content_font_once_installed() {
    // Plan M4: the pack's own font (content/axiom-town/ui/axiom-pixel.ttf). egui applies new
    // fonts at the next pass: on the first frame they are not usable yet (classes would draw
    // the default font; areas are only measured on their first frame) and nothing panics.
    let mut r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    let line = r.observation().metrics["gameplay"]["bindings"]["line"]
        .as_str()
        .map(str::to_owned)
        .expect("the opening line");
    let ctx = egui::Context::default();
    axiom_runtime::ui::configure(&ctx);
    let mut ui = UiState::default();
    frames_at(&r, &ctx, &mut ui, [1280.0, 720.0], 1, vec![]);
    assert!(!ui.fonts_ready, "not installed on the first frame");
    let later = frames_at(&r, &ctx, &mut ui, [1280.0, 720.0], 2, vec![]);
    let families = families_of(&later, &line);
    assert_eq!(families.len(), 1, "{families:?}");
    assert!(
        matches!(&families[0], egui::FontFamily::Name(n) if n.starts_with("ui:pixel:")),
        "{families:?}"
    );
    assert!(ui.fonts_ready);
    // A context that loses the fonts (another set_fonts) gets them back.
    axiom_runtime::ui::configure(&ctx);
    frames_at(&r, &ctx, &mut ui, [1280.0, 720.0], 4, vec![]);
    assert!(ui.fonts_ready, "reinstalled after being replaced");
}

#[test]
fn a_font_file_that_is_not_a_font_fails_activation_with_its_id() {
    let dir = std::env::temp_dir().join(format!("axiom-ui-font-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.ttf");
    std::fs::write(&bad, b"not a font at all").unwrap();
    let mut world = town();
    world.resources.get_mut("ui.document").unwrap()["theme"]["fonts"]["pixel"]["path"] =
        serde_json::json!(bad.to_string_lossy());
    let error = WorldRuntime::new(world, 1, installed_registry())
        .err()
        .expect("refused");
    let text = format!("{error:#}");
    assert!(
        text.contains("ui.document font pixel") && text.contains("TrueType"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_counted_template_use_shows_one_instance_per_unit_of_its_variable() {
    // Plan M5: 25 instances with count=coins; coins starts at 20, so 20 are drawn.
    let mut world = town();
    let doc = world.resources.get_mut("ui.document").unwrap();
    doc["templates"]["pip"] = serde_json::json!({
        "widgets": [{"kind": "text", "id": "pip_{i}", "binding": {"kind": "key", "id": "e_wave"}}]
    });
    doc["panels"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "pips", "anchor": "top_left", "offset": [0, 200], "layout": "grid", "columns": 5,
            "children": []
        }));
    doc["uses"].as_array_mut().unwrap().push(serde_json::json!({
        "id": "pip_row", "template": "pip", "into": "pips", "count": "coins",
        "instances": vec![serde_json::json!({}); 25]
    }));
    let mut r = WorldRuntime::new(world, 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    assert_eq!(
        r.observation().metrics["gameplay"]["variables"]["coins"],
        20
    );
    let ctx = egui::Context::default();
    let mut ui = UiState::default();
    frames_at(&r, &ctx, &mut ui, [1280.0, 720.0], 4, vec![]);
    let layout = r.ui_layout_observation(&ui).expect("ui_layout");
    let pips: Vec<&str> = layout["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["panel"] == "pips")
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(pips.len(), 20, "{pips:?}");
    assert_eq!(pips.first(), Some(&"pip_0"));
    assert_eq!(pips.last(), Some(&"pip_19"));
}
