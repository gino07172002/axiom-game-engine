//! Untrusted-input limits of the importers (review R4 F1/F2/F5): malformed glTF image views,
//! oversize and too many PMX textures, and the clip index the authoring validator reads. All
//! inputs are generated here; they must fail recoverably, never panic.
use axiom_animation::import;
use std::path::{Path, PathBuf};

/// A fresh, empty scratch directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("axiom-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn load_err(path: &Path) -> String {
    let result = std::panic::catch_unwind(|| import::load(path, &Default::default()));
    match result {
        Ok(Ok(_)) => panic!("malformed asset imported"),
        Ok(Err(e)) => format!("{e:#}"),
        Err(_) => panic!("importer panicked on malformed input"),
    }
}

#[test]
fn a_gltf_image_view_whose_end_overflows_is_an_error() {
    let dir = scratch("gltf-view");
    let path = dir.join("bad.gltf");
    std::fs::write(
        &path,
        r#"{"asset":{"version":"2.0"},
        "buffers":[{"byteLength":4,"uri":"data:application/octet-stream;base64,AAAAAA=="}],
        "bufferViews":[{"buffer":0,"byteOffset":18446744073709551615,"byteLength":2}],
        "images":[{"bufferView":0,"mimeType":"image/png"}],
        "nodes":[{}]}"#,
    )
    .unwrap();
    let e = load_err(&path);
    assert!(e.contains("image 0: invalid image buffer view 0"), "{e}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// A PMX 2.0 file (UTF-8, 1-byte indices) with no vertices, `textures` texture paths and one
/// material per texture, ending after the materials (the bone block is never reached when a
/// texture fails).
fn pmx(textures: &[&str]) -> Vec<u8> {
    let mut b = b"PMX ".to_vec();
    b.extend(2.0f32.to_le_bytes());
    b.extend([8, 1, 0, 1, 1, 1, 1, 1, 1]);
    let text = |b: &mut Vec<u8>, s: &str| {
        b.extend((s.len() as i32).to_le_bytes());
        b.extend(s.as_bytes());
    };
    for _ in 0..4 {
        text(&mut b, "");
    }
    b.extend(0i32.to_le_bytes()); // vertices
    b.extend(0i32.to_le_bytes()); // indices
    b.extend((textures.len() as i32).to_le_bytes());
    for t in textures {
        text(&mut b, t);
    }
    b.extend((textures.len() as i32).to_le_bytes());
    for i in 0..textures.len() {
        text(&mut b, &format!("m{i}"));
        text(&mut b, "");
        for _ in 0..(4 + 3 + 1 + 3) {
            b.extend(1.0f32.to_le_bytes()); // color, specular, power, ambient
        }
        b.push(0); // flags
        for _ in 0..(4 + 1) {
            b.extend(1.0f32.to_le_bytes()); // edge color, edge size
        }
        b.push(i as u8); // texture index
        b.push(0xff); // sphere index -1
        b.push(0); // sphere mode
        b.push(1); // shared toon
        b.push(0);
        text(&mut b, "");
        b.extend(0i32.to_le_bytes()); // index count
    }
    b
}

#[test]
fn an_oversize_pmx_texture_is_refused_with_its_path() {
    let dir = scratch("pmx-big");
    // 32 MiB + 1 byte: refused after reading at most that much.
    std::fs::File::create(dir.join("huge.png"))
        .unwrap()
        .set_len(import::MAX_ENCODED_TEXTURE as u64 + 1)
        .unwrap();
    let path = dir.join("model.pmx");
    std::fs::write(&path, pmx(&["huge.png"])).unwrap();
    let e = load_err(&path);
    assert!(
        e.contains("PMX texture huge.png") && e.contains("exceeds 32 MiB"),
        "{e}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pmx_textures_share_the_decoded_texture_budget() {
    let dir = scratch("pmx-many");
    // Five distinct 4096 x 4000 images decode to 62.5 MiB each: the fifth passes 256 MiB.
    let image = image::RgbaImage::new(4096, 4000);
    image.save(dir.join("t0.png")).unwrap();
    let names = ["t0.png", "t1.png", "t2.png", "t3.png", "t4.png"];
    for n in &names[1..] {
        std::fs::copy(dir.join("t0.png"), dir.join(n)).unwrap();
    }
    let path = dir.join("model.pmx");
    std::fs::write(&path, pmx(&names)).unwrap();
    let e = load_err(&path);
    assert!(
        e.contains("PMX texture t4.png") && e.contains("decoded textures exceed 256 MiB"),
        "{e}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn clip_ids_list_gltf_animations_without_importing() {
    let ids = import::clip_ids(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/animator-lab/axiom-short.glb"),
    )
    .unwrap()
    .unwrap();
    for clip in ["idle", "walk", "run", "wave"] {
        assert!(ids.iter().any(|c| c == clip), "{clip} missing from {ids:?}");
    }
    // The same IDs the importer gives.
    let asset = import::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/animator-lab/axiom-short.glb"),
        &Default::default(),
    )
    .unwrap();
    let imported: Vec<&str> = asset.clips.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.iter().map(String::as_str).collect::<Vec<_>>(), imported);
    // A truncated GLB is an error, not a panic.
    let dir = scratch("glb-short");
    let bad = dir.join("bad.glb");
    std::fs::write(&bad, b"glTF\x02\0\0\0").unwrap();
    assert!(import::clip_ids(&bad).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
