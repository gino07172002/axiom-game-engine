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

/// A one-triangle glTF whose POSITION accessor is `accessor` (JSON object members), over a
/// 36-byte buffer holding three `f32` vec3 positions (view 0) and a second 36-byte view 1.
fn triangle(dir: &Path, accessor: &str, extra: &str) -> PathBuf {
    use base64::Engine;
    let mut bytes = vec![];
    for f in [0.0f32, 0., 0., 1., 0., 0., 0., 1., 0.].repeat(2) {
        bytes.extend(f.to_le_bytes());
    }
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let path = dir.join("tri.gltf");
    std::fs::write(
        &path,
        format!(
            r#"{{"asset":{{"version":"2.0"}},
            "buffers":[{{"byteLength":72,"uri":"data:application/octet-stream;base64,{data}"}}],
            "bufferViews":[{{"buffer":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":36}}],
            "accessors":[{{"min":[0,0,0],"max":[1,1,0],{accessor}}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}{extra}}}]}}],
            "nodes":[{{"mesh":0}}]}}"#
        ),
    )
    .unwrap();
    path
}

#[test]
fn malformed_gltf_accessors_are_errors_not_panics() {
    let dir = scratch("gltf-accessor");
    let ok = r#""bufferView":0,"componentType":5126,"count":3,"type":"VEC3""#;
    // The well-formed triangle imports.
    import::load(&triangle(&dir, ok, ""), &Default::default()).unwrap();
    for (accessor, extra, expected) in [
        // count = 0: the crate computes `count - 1` before any range check.
        (
            r#""bufferView":0,"componentType":5126,"count":0,"type":"VEC3""#,
            "",
            "accessor 0: count must be at least 1",
        ),
        // A count whose byte size overflows `usize` arithmetic.
        (
            r#""bufferView":0,"componentType":5126,"count":4611686018427387904,"type":"VEC3""#,
            "",
            "accessor 0: range exceeds buffer view 0",
        ),
        // A read sparse accessor that expands past 256 MiB (23,000,000 x 12 bytes).
        (
            r#""componentType":5126,"count":23000000,"type":"VEC3","sparse":{"count":1,"indices":{"bufferView":1,"componentType":5125},"values":{"bufferView":1}}"#,
            "",
            "accessor 0: exceeds 256 MiB",
        ),
        // Four elements in a 36-byte view.
        (
            r#""bufferView":0,"componentType":5126,"count":4,"type":"VEC3""#,
            "",
            "accessor 0: range exceeds buffer view 0",
        ),
        // An offset near usize::MAX.
        (
            r#""bufferView":0,"byteOffset":18446744073709551600,"componentType":5126,"count":3,"type":"VEC3""#,
            "",
            "accessor 0: range exceeds buffer view 0",
        ),
        // POSITION declared as VEC2 (the reader assumes VEC3 f32).
        (
            r#""bufferView":0,"componentType":5126,"count":3,"type":"VEC2""#,
            "",
            "accessor 0 (POSITION/NORMAL) must be Vec3",
        ),
        // Float indices (the crate's index reader is `unreachable!` for them).
        (ok, r#","indices":0"#, "accessor 0 (indices) must be Scalar"),
        // A sparse accessor with zero sparse elements.
        (
            r#""componentType":5126,"count":3,"type":"VEC3","sparse":{"count":0,"indices":{"bufferView":1,"componentType":5125},"values":{"bufferView":1}}"#,
            "",
            "accessor 0: sparse count must be in 1..=3",
        ),
        // Sparse values that run past their view.
        (
            r#""componentType":5126,"count":3,"type":"VEC3","sparse":{"count":3,"indices":{"bufferView":1,"componentType":5125},"values":{"bufferView":1,"byteOffset":12}}"#,
            "",
            "accessor 0: sparse values range exceeds buffer view 1",
        ),
    ] {
        let e = load_err(&triangle(&dir, accessor, extra));
        assert!(e.contains(expected), "expected {expected:?}, got {e}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unused_sparse_accessor_is_not_expanded() {
    // Review R4 N1 (round 3): the 256 MiB expansion budget applies to accessors the importer
    // reads. A 23,000,000-element sparse VEC3 accessor nothing uses does not stop the import.
    let dir = scratch("gltf-unused-sparse");
    let ok = r#""bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
        {"componentType":5126,"count":23000000,"type":"VEC3","sparse":{"count":1,"indices":{"bufferView":1,"componentType":5125},"values":{"bufferView":1}}"#;
    let asset = import::load(&triangle(&dir, ok, ""), &Default::default()).unwrap();
    assert_eq!(asset.vertices.len(), 3);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A glTF of one triangle (`triangle`) whose node 0 has the animations `animations` (JSON
/// array members), over extra accessors `accessors` (JSON array members, indices from 1).
fn animated(dir: &Path, accessors: &str, animations: &str) -> PathBuf {
    let path = triangle(
        dir,
        &format!(r#""bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},{accessors}"#),
        "",
    );
    let text = std::fs::read_to_string(&path).unwrap().replace(
        r#""nodes":"#,
        &format!(r#""animations":[{animations}],"nodes":"#),
    );
    std::fs::write(&path, text).unwrap();
    path
}

/// Accessor 1: one time key 0; accessor 2: one zero translation.
const ONE_KEY: &str = r#"{"bufferView":0,"componentType":5126,"count":1,"type":"SCALAR","min":[0],"max":[0]},
    {"bufferView":0,"componentType":5126,"count":1,"type":"VEC3""#;

fn one_key_clip(name: &str) -> String {
    format!(
        r#"{{"name":"{name}","samplers":[{{"input":1,"output":2}}],"channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}]}}"#
    )
}

#[test]
fn a_renamed_clip_is_listed_although_the_file_kept_its_length_and_time() {
    // Review R4 A-F5 (round 3): the clip index is keyed by content, not by length and
    // modification time. An import fills it; a file replaced with the same length and time
    // whose clip was renamed lists the new name (the validator accepts it), while unchanged
    // content is still served from the index.
    let dir = scratch("clip-index");
    let path = animated(&dir, ONE_KEY, &one_key_clip("aaaa"));
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let asset = import::load(&path, &Default::default()).unwrap();
    assert_eq!(asset.clips[0].id, "aaaa");
    let first = import::clip_ids(&path).unwrap().unwrap();
    assert_eq!(*first, ["aaaa"]);
    // Unchanged content: the same index entry.
    let again = import::clip_ids(&path).unwrap().unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(r#""name":"aaaa""#, r#""name":"bbbb""#);
    std::fs::write(&path, text).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    assert_eq!(meta.modified().unwrap(), modified);
    let ids = import::clip_ids(&path).unwrap().unwrap();
    assert_eq!(*ids, ["bbbb"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn shared_animation_inputs_are_budgeted_before_they_are_expanded() {
    // Review R4 N2 (round 3): channels share accessors, so one 1,000,000-key input (a sparse
    // accessor: a few bytes of file) read by three channels must be refused from the counts,
    // before 3,000,000 times are collected.
    let dir = scratch("gltf-anim-budget");
    let big_input = r#"{"componentType":5126,"count":1000000,"type":"SCALAR","min":[0],"max":[0],"sparse":{"count":1,"indices":{"bufferView":1,"componentType":5125},"values":{"bufferView":1}}},
        {"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"},
        {"bufferView":0,"componentType":5126,"count":1,"type":"VEC4""#;
    let clip = r#"{"samplers":[{"input":1,"output":2},{"input":1,"output":3}],"channels":[
        {"sampler":0,"target":{"node":0,"path":"translation"}},
        {"sampler":0,"target":{"node":0,"path":"scale"}},
        {"sampler":1,"target":{"node":0,"path":"rotation"}}]}"#;
    let e = load_err(&animated(&dir, big_input, clip));
    assert!(
        e.contains("glTF animation 0: key budget exceeded (3 values, 3000000 times"),
        "{e}"
    );
    // Within each clip but over the asset: 17 clips of one 1,000,000-key channel.
    let clip = r#"{"samplers":[{"input":1,"output":2}],"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]}"#;
    let clips = vec![clip; 17].join(",");
    let e = load_err(&animated(&dir, big_input, &clips));
    assert!(e.contains("asset key budget of 16000000"), "{e}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_glb_header_length_that_disagrees_with_the_file_is_an_error() {
    // Review R4 N1 (round 3): the gltf crate subtracts the 12-byte header from the declared
    // length unchecked and allocates the result before reading. Short or overlong declared
    // lengths are refused first.
    let dir = scratch("glb-header");
    let path = dir.join("bad.glb");
    let json = br#"{"asset":{"version":"2.0"},"nodes":[{}]}  "#;
    for declared in [4u32, 11, 0xffff_fff0] {
        let mut b = b"glTF".to_vec();
        b.extend(2u32.to_le_bytes());
        b.extend(declared.to_le_bytes());
        b.extend((json.len() as u32).to_le_bytes());
        b.extend(b"JSON");
        b.extend(json);
        std::fs::write(&path, &b).unwrap();
        let e = load_err(&path);
        assert!(e.contains("GLB header length"), "{declared}: {e}");
    }
    // A header alone, shorter than its length field.
    std::fs::write(&path, b"glTF  ").unwrap();
    assert!(load_err(&path).contains("GLB header length (missing)"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_gltf_view_stride_shorter_than_its_element_is_an_error() {
    let dir = scratch("gltf-stride");
    let path = triangle(
        &dir,
        r#""bufferView":2,"componentType":5126,"count":3,"type":"VEC3""#,
        "",
    );
    // Add view 2 with byteStride 4 over the same bytes.
    let text = std::fs::read_to_string(&path).unwrap().replace(
        r#"{"buffer":0,"byteOffset":36,"byteLength":36}]"#,
        r#"{"buffer":0,"byteOffset":36,"byteLength":36},{"buffer":0,"byteLength":36,"byteStride":4}]"#,
    );
    std::fs::write(&path, text).unwrap();
    let e = load_err(&path);
    assert!(
        e.contains("accessor 0: byteStride 4 is smaller than its 12-byte element"),
        "{e}"
    );
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
fn an_oversize_texture_stream_is_read_only_to_the_limit() {
    /// 64 MiB of zeros that counts what was read.
    struct Counting<'a>(&'a std::cell::Cell<usize>);
    impl std::io::Read for Counting<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min((64 << 20) - self.0.get());
            buf[..n].fill(0);
            self.0.set(self.0.get() + n);
            Ok(n)
        }
    }
    let read = std::cell::Cell::new(0);
    let e = import::read_texture(Counting(&read))
        .unwrap_err()
        .to_string();
    assert!(e.contains("exceeds 32 MiB"), "{e}");
    assert_eq!(read.get(), import::MAX_ENCODED_TEXTURE + 1);
}

#[test]
fn pmx_textures_share_the_decoded_texture_budget() {
    let dir = scratch("pmx-many");
    // Five distinct 4096 x 4000 images decode to 62.5 MiB each: the fifth passes 256 MiB.
    let image = image::RgbaImage::new(4096, 4000);
    image.save(dir.join("t0.png")).unwrap();
    let names = ["t0.png", "t1.png", "t2.png", "t3.png", "t4.png"];
    for n in &names[1..4] {
        std::fs::copy(dir.join("t0.png"), dir.join(n)).unwrap();
    }
    // The fifth is cut 64 bytes into its first IDAT chunk: its header declares 4096 x 4000 but
    // it cannot be decoded, so the budget error proves the size was checked before decoding.
    let png = std::fs::read(dir.join("t0.png")).unwrap();
    let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
    std::fs::write(dir.join("t4.png"), &png[..idat + 68]).unwrap();
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
