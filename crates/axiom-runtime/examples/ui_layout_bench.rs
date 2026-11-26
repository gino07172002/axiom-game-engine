//! Plan M3 cost check: CPU time of one HUD frame (egui pass + tessellation) for the Axiom
//! Town document as authored, and with 30 extra framed grid panels (one text widget each;
//! the document budget is 64 widgets) laid
//! out against the 1920x1080 reference and scaled into a 1280x720 window.
//!
//! `cargo run -p axiom-runtime --example ui_layout_bench [--release]`; prints one JSON line.
//! Label results debug / release: the two differ by an order of magnitude.
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, ui::UiState, ActionMap, WorldRuntime};
use egui::{Pos2, Rect, Vec2};
use serde_json::json;
use std::time::Instant;

const WARMUP: usize = 20;
const FRAMES: usize = 300;

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// 30 more panels: 1 text widget each, a 2-column grid, the framed `dialog` style, spread
/// over the 9 anchors.
fn with_panels(mut world: World) -> World {
    let doc = world.resources.get_mut("ui.document").unwrap();
    let anchors = [
        "top_left",
        "top",
        "top_right",
        "left",
        "center",
        "right",
        "bottom_left",
        "bottom",
        "bottom_right",
    ];
    for i in 0..30 {
        let mut children = vec![];
        for k in 0..1 {
            let id = format!("bench_{i}_{k}");
            doc["widgets"].as_array_mut().unwrap().push(json!({
                "kind": "text", "id": id, "binding": {"kind": "variable", "id": "coins"}
            }));
            children.push(id);
        }
        doc["panels"].as_array_mut().unwrap().push(json!({
            "id": format!("bench_{i}"),
            "anchor": anchors[i % anchors.len()],
            "offset": [((i / 9) as f32) * 40.0, ((i / 9) as f32) * 30.0],
            "width": 220, "style": "dialog", "layout": "grid", "columns": 2, "gap": 6,
            "children": children,
        }));
    }
    world
}

/// Mean and 95th percentile (ms) of one HUD frame.
fn measure(world: World) -> (f64, f64, usize) {
    let mut r = WorldRuntime::new(world, 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    let ctx = egui::Context::default();
    let mut ui = UiState {
        playing: true,
        ..Default::default()
    };
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
        ..Default::default()
    };
    let mut times = vec![];
    let mut panels = 0;
    for k in 0..WARMUP + FRAMES {
        let start = Instant::now();
        let out = ctx.run(input(), |ctx| {
            let _ = r.draw_ui(ctx, &mut ui);
        });
        let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
        if k >= WARMUP {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        panels = ui.layout.panels.len();
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    (mean, times[times.len() * 95 / 100], panels)
}

fn main() {
    let (base_mean, base_p95, base_panels) = measure(town());
    let (mean, p95, panels) = measure(with_panels(town()));
    println!(
        "{}",
        json!({
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "window": [1280, 720], "frames": FRAMES,
            "town": {"panels_drawn": base_panels, "mean_ms": base_mean, "p95_ms": base_p95},
            "town_plus_30": {"panels_drawn": panels, "mean_ms": mean, "p95_ms": p95},
        })
    );
}
