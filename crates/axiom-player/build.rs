//! Embeds the Axiom icon into `axiom-player.exe` on Windows (GNU toolchain).
//!
//! The .ico is assembled by hand from the branding PNGs (ICO entries may carry PNG payloads
//! directly), compiled with `windres` from the workspace's portable MinGW, and linked as a
//! plain COFF object so the linker cannot drop it. Without `windres` the build only warns.
use std::path::{Path, PathBuf};
use std::process::Command;

const SIZES: [u32; 3] = [256, 64, 32];

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let branding = manifest.join("../../assets/branding");
    println!("cargo:rerun-if-changed=build.rs");
    for size in SIZES {
        println!(
            "cargo:rerun-if-changed={}",
            branding.join(format!("axiom-icon-{size}.png")).display()
        );
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        println!("cargo:warning=axiom-player icon is only embedded with the GNU toolchain");
        return;
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let ico = out.join("axiom.ico");
    if let Err(error) = write_ico(&branding, &ico) {
        println!("cargo:warning=axiom-player icon skipped: {error}");
        return;
    }
    let rc = out.join("axiom-player.rc");
    let resource = ico.to_string_lossy().replace('\\', "/");
    std::fs::write(&rc, format!("1 ICON \"{resource}\"\n")).unwrap();
    let object = out.join("axiom-player-icon.o");
    let bundled = manifest.join("../../.tools/mingw64/bin/windres.exe");
    let windres = if bundled.is_file() {
        bundled
    } else {
        PathBuf::from("windres")
    };
    let status = Command::new(&windres)
        .arg("-O")
        .arg("coff")
        .arg("-i")
        .arg(&rc)
        .arg("-o")
        .arg(&object)
        .status();
    match status {
        Ok(status) if status.success() => {
            println!("cargo:rustc-link-arg-bins={}", object.display());
        }
        Ok(status) => println!("cargo:warning=windres failed ({status}); no embedded icon"),
        Err(error) => println!(
            "cargo:warning=windres not found ({error}); run scripts/env.ps1 for an embedded icon"
        ),
    }
}

/// ICONDIR + one ICONDIRENTRY per image, followed by the PNG files verbatim.
fn write_ico(branding: &Path, target: &Path) -> Result<(), String> {
    let mut images = vec![];
    for size in SIZES {
        let path = branding.join(format!("axiom-icon-{size}.png"));
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
            return Err(format!("{} is not a PNG", path.display()));
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        if width != size || height != size {
            return Err(format!(
                "{} is {width}x{height}, expected {size}x{size}",
                path.display()
            ));
        }
        images.push((size, bytes));
    }
    let mut ico = vec![];
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, bytes) in &images {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]); // width, height, palette, reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += bytes.len() as u32;
    }
    for (_, bytes) in &images {
        ico.extend_from_slice(bytes);
    }
    std::fs::write(target, ico).map_err(|e| format!("{}: {e}", target.display()))
}
