//! Generates the original nine-slice UI frame images used by the demo content (no third-party
//! art). Deterministic: running it again writes byte-identical PNGs.
//!
//! cargo run -p axiom-runtime --example build_ui_frames -- content/axiom-town/ui
//!
//! `dialog-frame.png` (96 x 96, slice 32 on every side): a translucent night-blue panel with
//! a double gold border, rounded corners and a small diamond in each corner.
//! `button-normal.png`, `button-hover.png`, `button-pressed.png` (48 x 48, slice 14): a
//! rounded wine-red plate with a gold edge; hover is lighter, pressed darker with a thin edge.
use std::path::PathBuf;

const SIZE: usize = 96;

fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t.clamp(0.0, 1.0))
}

/// Signed distance to a rounded rectangle centred in a `size` image (negative inside).
fn rounded_in(size: usize, x: f32, y: f32, half: f32, radius: f32) -> f32 {
    let c = size as f32 / 2.0;
    let qx = (x - c).abs() - (half - radius);
    let qy = (y - c).abs() - (half - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// A 48 x 48 button plate: `body` with a vertical sheen, a `rim`-coloured edge `edge` px wide.
fn button(body: [f32; 4], rim: [f32; 4], edge: f32, sheen: f32) -> Vec<u8> {
    const B: usize = 48;
    let mut rgba = Vec::with_capacity(B * B * 4);
    for y in 0..B {
        for x in 0..B {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = rounded_in(B, fx, fy, 23.0, 9.0);
            let t = fy / B as f32;
            let lit = mix(
                body,
                [body[0] + 40.0, body[1] + 30.0, body[2] + 30.0, body[3]],
                sheen * (1.0 - t),
            );
            let mut c = mix([0.0; 4], lit, 0.5 - d);
            let rim_cover = (edge - (d + edge).abs()).clamp(0.0, 1.0);
            c = mix(c, rim, rim_cover);
            rgba.extend(c.map(|v| v.round().clamp(0.0, 255.0) as u8));
        }
    }
    rgba
}

/// Signed distance to a rounded rectangle centred in the image (negative inside).
fn rounded(x: f32, y: f32, half: f32, radius: f32) -> f32 {
    let c = SIZE as f32 / 2.0;
    let qx = (x - c).abs() - (half - radius);
    let qy = (y - c).abs() - (half - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

fn dialog_frame() -> Vec<u8> {
    let fill = [16.0, 22.0, 38.0, 222.0];
    let gold = [233.0, 196.0, 120.0, 255.0];
    let deep = [150.0, 110.0, 52.0, 255.0];
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Sample at the pixel centre; one-pixel coverage gives antialiased edges.
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let outer = rounded(fx, fy, 46.0, 14.0);
            let mut c = [0.0; 4];
            // Body, with a slight vertical gradient.
            let body = mix(fill, [24.0, 32.0, 54.0, 230.0], fy / SIZE as f32);
            c = mix(c, body, 0.5 - outer);
            // Outer gold rim (2 px) and inner thin line 5 px inside it.
            let rim = 1.0 - (outer + 1.0).abs();
            c = mix(c, gold, rim.clamp(0.0, 1.0));
            let inner = rounded(fx, fy, 40.0, 9.0);
            let line = 1.0 - (inner.abs() * 1.6);
            c = mix(c, deep, line.clamp(0.0, 1.0) * 0.9);
            // Corner diamonds centred 11 px in from each corner.
            for (cx, cy) in [(11.0, 11.0), (85.0, 11.0), (11.0, 85.0), (85.0, 85.0)] {
                let d = (fx - cx).abs() + (fy - cy).abs();
                c = mix(c, gold, (4.5 - d).clamp(0.0, 1.0));
            }
            rgba.extend(c.map(|v| v.round().clamp(0.0, 255.0) as u8));
        }
    }
    rgba
}

fn write_png(path: &PathBuf, size: usize, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create PNG");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size as u32, size as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .expect("write PNG");
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "content/axiom-town/ui".into()),
    );
    std::fs::create_dir_all(&out).expect("output directory");
    let gold = [233.0, 196.0, 120.0, 255.0];
    let images = [
        ("dialog-frame.png", SIZE, dialog_frame()),
        (
            "button-normal.png",
            48,
            button([92.0, 30.0, 44.0, 235.0], gold, 1.6, 0.6),
        ),
        (
            "button-hover.png",
            48,
            button([128.0, 44.0, 62.0, 240.0], gold, 2.0, 1.0),
        ),
        (
            "button-pressed.png",
            48,
            button(
                [60.0, 18.0, 30.0, 245.0],
                [170.0, 132.0, 72.0, 255.0],
                1.0,
                0.0,
            ),
        ),
    ];
    for (name, size, rgba) in images {
        let path = out.join(name);
        write_png(&path, size, &rgba);
        println!("{}", path.display());
    }
}
