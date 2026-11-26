//! Plan M4 cost check: main-thread time of the HUD frames around a content font install
//! (`ensure_fonts` calls `set_fonts` on frame 0; egui rebuilds its fonts and atlas at the
//! start of frame 1). The context starts with the game's fonts (`ui::configure`: egui
//! defaults + the Windows CJK fallback), as in the player. Also times the off-main-thread part
//! (world build: read + parse of the font file) for reference.
//!
//! `cargo run -p axiom-runtime --example ui_font_bench [--release]`; prints one JSON line.
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, ui::UiState, ActionMap, WorldRuntime};
use egui::{Pos2, Rect, Vec2};
use serde_json::json;
use std::time::Instant;

fn town(fonts: bool) -> World {
    let mut world = ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world;
    if !fonts {
        // The same document without content fonts: the classes draw the default font.
        let doc = world.resources.get_mut("ui.document").unwrap();
        doc["theme"].as_object_mut().unwrap().remove("fonts");
        for class in ["speaker", "dialog_line"] {
            doc["theme"]["text"][class]
                .as_object_mut()
                .unwrap()
                .remove("font");
        }
        doc["theme"]["button"]["emote"]
            .as_object_mut()
            .unwrap()
            .remove("font");
    }
    world
}

/// (world build ms, per-frame ms for 8 frames, fonts ready after).
fn run(fonts: bool) -> (f64, Vec<f64>, bool) {
    let world = town(fonts);
    let started = Instant::now();
    let mut r = WorldRuntime::new(world, 1, installed_registry()).unwrap();
    let build = started.elapsed().as_secs_f64() * 1000.0;
    r.step(&ActionMap::default());
    let ctx = egui::Context::default();
    axiom_runtime::ui::configure(&ctx);
    // Settle the base fonts first so the font install is the only change measured.
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
        ..Default::default()
    };
    let mut warm = UiState::default();
    for _ in 0..3 {
        let _ = ctx.run(input(), |_| {});
    }
    let mut times = vec![];
    for _ in 0..8 {
        let started = Instant::now();
        let out = ctx.run(input(), |ctx| {
            let _ = r.draw_ui(ctx, &mut warm);
        });
        let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
        times.push((started.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0);
    }
    (build, times, warm.fonts_ready)
}

fn main() {
    let (build_without, without, _) = run(false);
    let (build_with, with, ready) = run(true);
    println!(
        "{}",
        json!({
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "window": [1280, 720],
            "without_font": {"world_build_ms": build_without, "frame_ms": without},
            "with_font": {"world_build_ms": build_with, "frame_ms": with, "ready": ready},
            "budget_ms": 16.0,
        })
    );
}
