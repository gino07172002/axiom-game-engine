//! Embedded Axiom brand marks used by native windows and the editor chrome.
use anyhow::{ensure, Result};

/// 64 px square mark (character head crop) for window icons and toolbar badges.
pub const ICON_64: &[u8] = include_bytes!("../../../assets/branding/axiom-icon-64.png");

/// Decodes an 8-bit RGBA/RGB PNG into tightly packed RGBA pixels.
pub fn decode_rgba(raw: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(raw));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let (w, h) = (reader.info().width, reader.info().height);
    ensure!(w > 0 && h > 0 && w <= 1024 && h <= 1024, "brand mark size");
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes)?;
    let bytes = &bytes[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => bytes
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        _ => anyhow::bail!("unsupported brand mark color format"),
    };
    ensure!(rgba.len() == (w * h * 4) as usize, "brand mark buffer size");
    Ok((rgba, w, h))
}

pub fn window_icon() -> Option<winit::window::Icon> {
    let (rgba, w, h) = decode_rgba(ICON_64).ok()?;
    winit::window::Icon::from_rgba(rgba, w, h).ok()
}

pub fn egui_icon() -> Option<egui::ColorImage> {
    let (rgba, w, h) = decode_rgba(ICON_64).ok()?;
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        &rgba,
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_icon_decodes_to_square_rgba() {
        let (rgba, w, h) = super::decode_rgba(super::ICON_64).unwrap();
        assert_eq!((w, h), (64, 64));
        assert_eq!(rgba.len(), 64 * 64 * 4);
        assert!(super::window_icon().is_some());
        // Transparent corners, opaque artwork in the middle.
        assert_eq!(rgba[3], 0);
        assert!(rgba[((32 * 64 + 32) * 4 + 3) as usize] > 200);
    }
}
