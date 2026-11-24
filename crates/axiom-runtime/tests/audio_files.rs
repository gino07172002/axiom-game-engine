//! File-backed audio clips (`audio.library.clips.ID.file`): pack resolution and confinement,
//! decode-once caching shared across runtimes, playback through the same voices as procedural
//! clips, `observe` file metadata, hot reload when the file content changes, and
//! recoverable errors (missing, corrupt, over budget, outside the pack) that name the path.
use axiom_core::{ContentPackage, Diagnostic, World};
use axiom_runtime::{audio_file::encode_wav, installed_registry, ActionMap, WorldRuntime};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn sine(frequency: f32, rate: u32, seconds: f32, channels: u16) -> Vec<f32> {
    let n = (rate as f32 * seconds) as usize;
    (0..n)
        .flat_map(|i| {
            let s = 0.4 * (std::f32::consts::TAU * frequency * i as f32 / rate as f32).sin();
            std::iter::repeat_n(s, usize::from(channels))
        })
        .collect()
}

fn fixture_ogg() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/audio-lab/sounds/chime.ogg")
}

/// A fresh pack directory `name` with `clips` and one looping autoplay source per clip.
fn pack(name: &str, clips: Value) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("audio-files-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sounds")).unwrap();
    let mut entities =
        vec![json!({"id":"ear","components":{"core.transform":{},"audio.listener":{}}})];
    for (k, id) in clips.as_object().unwrap().keys().enumerate() {
        entities.push(json!({"id":format!("src/{id}"),"components":{
            "core.transform":{"position":[k as f32 * 2.0 - 1.0, 0.0, -2.0]},
            "audio.source":{"clip":id,"loop":true,"autoplay":true}}}));
    }
    let world = json!({"schema_version":2,"id":name,"title":name,
        "capabilities":{"core":1,"audio":1},"systems":["audio.spatial"],
        "entities":entities,"resources":{"audio.library":{"clips":clips}}});
    std::fs::write(root.join("world.json"), world.to_string()).unwrap();
    std::fs::write(
        root.join("manifest.json"),
        json!({"id":"test.audio","version":1,"world":"world.json","capabilities":{"core":1,"audio":1}}).to_string(),
    )
    .unwrap();
    root
}

fn load(root: &Path) -> anyhow::Result<World> {
    ContentPackage::load(root, &installed_registry().authoring).map(|p| p.world)
}

fn message(e: &anyhow::Error) -> String {
    let path = e
        .chain()
        .find_map(|c| c.downcast_ref::<Diagnostic>())
        .map(|d| d.path.clone())
        .unwrap_or_default();
    format!("{path} {e:#}")
}

#[test]
fn pack_file_clips_resolve_decode_and_play_like_procedural_ones() {
    let root = pack(
        "play",
        json!({"hum":{"file":{"path":"sounds/hum.wav"}},
               "chime":{"file":{"path":"sounds/chime.ogg","gain":0.5}},
               "tone":{"frequency":440,"seconds":1}}),
    );
    std::fs::write(
        root.join("sounds/hum.wav"),
        encode_wav(&sine(110.0, 48_000, 1.5, 2), 2, 48_000, 24, false),
    )
    .unwrap();
    std::fs::copy(fixture_ogg(), root.join("sounds/chime.ogg")).unwrap();
    let world = load(&root).unwrap();
    // Resolved to absolute paths inside the pack (what packaging and the runtime read).
    let resolved = world.resources["audio.library"]["clips"]["hum"]["file"]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    let canonical = root.canonicalize().unwrap();
    assert!(Path::new(&resolved).is_absolute() && Path::new(&resolved).starts_with(&canonical));
    // The procedural clip keeps its authored text.
    assert_eq!(
        world.resources["audio.library"]["clips"]["tone"],
        json!({"frequency":440,"seconds":1})
    );
    let mut r = WorldRuntime::new(world.clone(), 1, installed_registry()).unwrap();
    for _ in 0..30 {
        r.step(&ActionMap::default());
    }
    let audio = r.observation().metrics["audio"].clone();
    assert_eq!(audio["voiced"], 3, "{audio}");
    let files = audio["clips"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{audio}");
    let by = |id: &str| files.iter().find(|f| f["clip"] == id).unwrap().clone();
    let hum = by("hum");
    assert_eq!(
        (
            hum["format"].clone(),
            hum["channels"].clone(),
            hum["rate"].clone()
        ),
        (json!("wav"), json!(2), json!(48_000))
    );
    assert_eq!(hum["seconds"], 1.5);
    assert_eq!(hum["path"], resolved);
    assert!(hum["decode_ms"].as_f64().unwrap() >= 0.0);
    let chime = by("chime");
    assert_eq!(
        (chime["format"].clone(), chime["channels"].clone()),
        (json!("ogg"), json!(2))
    );
    assert!((chime["seconds"].as_f64().unwrap() - 2.0).abs() < 0.05);
    let inspect = r
        .command(&axiom_core::gameplay::RuntimeCommand::Call {
            capability: "audio".into(),
            action: "inspect".into(),
            args: json!({"id":"src/hum"}),
        })
        .unwrap()["result"]
        .clone();
    assert_eq!(inspect["length"], 1.5);
    assert_eq!(inspect["playing"], true);
    // A second runtime (hot reload, another session) shares the decoded PCM.
    let bytes = audio["clips"]["cache_bytes"].clone();
    let again = WorldRuntime::new(world, 2, installed_registry()).unwrap();
    assert_eq!(
        again.observation().metrics["audio"]["clips"]["cache_bytes"],
        bytes
    );
}

#[test]
fn changed_file_content_restarts_and_old_paths_still_work_after_reload() {
    let root = pack("reload", json!({"hum":{"file":{"path":"sounds/hum.wav"}}}));
    let wav = root.join("sounds/hum.wav");
    std::fs::write(
        &wav,
        encode_wav(&sine(110.0, 22_050, 1.0, 1), 1, 22_050, 16, false),
    )
    .unwrap();
    let world = load(&root).unwrap();
    let mut r = WorldRuntime::new(world.clone(), 1, installed_registry()).unwrap();
    for _ in 0..60 {
        r.step(&ActionMap::default());
    }
    // The Inspector reads file info from the active runtime, not the process-wide cache
    // (which then holds both versions under the same path).
    let seconds = |r: &WorldRuntime| {
        r.system::<axiom_runtime::spatial_audio::AudioSpatialSystem>()
            .and_then(|s| s.clip_file("hum"))
            .map(|f| f.seconds)
    };
    assert_eq!(seconds(&r), Some(1.0));
    std::fs::write(
        &wav,
        encode_wav(&sine(220.0, 22_050, 2.0, 1), 1, 22_050, 16, false),
    )
    .unwrap();
    r.reconcile(world, 2).unwrap();
    r.step(&ActionMap::default());
    assert_eq!(seconds(&r), Some(2.0));
    let audio = r.observation().metrics["audio"].clone();
    assert_eq!(audio["clips"]["files"][0]["seconds"], 2.0, "{audio}");
    let inspect = r
        .command(&axiom_core::gameplay::RuntimeCommand::Call {
            capability: "audio".into(),
            action: "inspect".into(),
            args: json!({"id":"src/hum"}),
        })
        .unwrap()["result"]
        .clone();
    assert!(
        inspect["time"].as_f64().unwrap() < 0.05,
        "restarted: {inspect}"
    );
}

/// R6 F6 (round 3): every build checks the file content. A file replaced by another of the
/// same size whose modification time is put back (a copy that preserves times, a restored
/// backup) is decoded again; round 1 keyed the cache on (path, size, mtime) alone and
/// returned the old PCM.
#[test]
fn a_replaced_file_with_the_same_size_and_time_is_decoded_again() {
    let root = pack(
        "replaced",
        json!({"hum":{"file":{"path":"sounds/hum.wav"}}}),
    );
    let file = root.join("sounds/hum.wav");
    let a = encode_wav(&sine(130.0, 22_050, 0.7, 1), 1, 22_050, 16, false);
    // Same sample count and header at twice the rate: same size, other content.
    let b = encode_wav(&sine(130.0, 22_050, 0.7, 1), 1, 44_100, 16, false);
    assert_eq!(a.len(), b.len());
    assert_ne!(a, b);
    std::fs::write(&file, &a).unwrap();
    let world = load(&root).unwrap();
    let observed = |rev| {
        WorldRuntime::new(world.clone(), rev, installed_registry())
            .unwrap()
            .observation()
            .metrics["audio"]["clips"]["files"][0]
            .clone()
    };
    let first = observed(1);
    assert_eq!(first["rate"], 22_050, "{first}");
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::fs::write(&file, &b).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        modified
    );
    let second = observed(2);
    assert_eq!(second["rate"], 44_100, "{second}");
    assert_eq!(second["seconds"], 0.35, "{second}");
    // Unchanged again: the same (cached) decode.
    assert_eq!(observed(3)["rate"], 44_100);
}

#[test]
fn an_over_budget_clip_count_fails_before_any_file_is_read() {
    // Every clip names a missing file: reading any of them would fail with "not found", so
    // a budget error proves the count was checked first.
    let clips: serde_json::Map<String, Value> = (0..=axiom_core::audio::MAX_LIBRARY_CLIPS)
        .map(|k| (format!("c{k}"), json!({"file":{"path":"sounds/none.wav"}})))
        .collect();
    let root = pack("too-many", Value::Object(clips));
    let e = message(&load(&root).unwrap_err());
    assert!(
        e.starts_with("/resources/audio.library/clips ") && e.contains("clip budget"),
        "{e}"
    );
}

#[test]
fn bad_files_are_recoverable_errors_that_name_the_path() {
    let wav = encode_wav(&sine(110.0, 22_050, 0.2, 1), 1, 22_050, 16, false);
    let case = |name: &str, path: &str, bytes: Option<&[u8]>| -> String {
        let root = pack(name, json!({"hum":{"file":{"path":path}}}));
        if let Some(bytes) = bytes {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        message(&load(&root).unwrap_err())
    };
    let missing = case("missing", "sounds/none.wav", None);
    assert!(
        missing.contains("/resources/audio.library/clips/hum/file")
            && missing.contains("sounds/none.wav"),
        "{missing}"
    );
    let corrupt = case(
        "corrupt",
        "sounds/bad.wav",
        Some(b"RIFF\x10\0\0\0WAVEjunkjunk"),
    );
    assert!(
        corrupt.contains("sounds/bad.wav") && corrupt.contains("WAV"),
        "{corrupt}"
    );
    let mut long = encode_wav(&[0.0; 16], 1, 8_000, 8, false);
    // Header claims 125 s of data; the file really holds it (8 kHz 8-bit mono = 1 MB).
    long.truncate(44);
    long.resize(44 + 8_000 * 125, 128);
    let n = (long.len() - 44) as u32;
    long[40..44].copy_from_slice(&n.to_le_bytes());
    long[4..8].copy_from_slice(&(n + 36).to_le_bytes());
    let over = case("long", "sounds/long.wav", Some(&long));
    assert!(
        over.contains("budget 120 s") && over.contains("sounds/long.wav"),
        "{over}"
    );
    let mp3 = case("mp3", "sounds/song.mp3", Some(&wav));
    assert!(mp3.contains(".wav or .ogg"), "{mp3}");
    // Outside the pack: parent directories and absolute paths.
    let up = case("up", "../outside.wav", Some(&wav));
    assert!(up.contains("escapes") || up.contains("relative"), "{up}");
    let abs = fixture_ogg().canonicalize().unwrap();
    let absolute = case("abs", abs.to_str().unwrap(), None);
    assert!(absolute.contains("relative"), "{absolute}");
    // A world authored without a pack (apply) that names a missing file fails at runtime
    // build, with the path, instead of playing silence.
    let mut world = load(&pack("apply", json!({"tone":{"frequency":440}}))).unwrap();
    world.resources.insert(
        "audio.library".into(),
        json!({"clips":{"tone":{"file":{"path":"Z:/no/such/dir/tone.wav"}}}}),
    );
    let e = WorldRuntime::new(world, 1, installed_registry())
        .err()
        .unwrap();
    assert!(
        format!("{e:#}").contains("Z:/no/such/dir/tone.wav"),
        "{e:#}"
    );
}

#[test]
fn file_clips_take_no_synthesis_fields_and_schema_reports_files() {
    let registry = installed_registry().authoring;
    let mut world: World = serde_json::from_value(json!({"schema_version":2,"id":"w","title":"w",
        "capabilities":{"core":1,"audio":1},"systems":["audio.spatial"],"entities":[],
        "resources":{"audio.library":{"clips":{"a":{"file":{"path":"a.wav"},"seconds":3}}}}}))
    .unwrap();
    let e = world.compile(&registry).unwrap_err();
    assert!(format!("{e:#}").contains("synthesis"), "{e:#}");
    world.resources.insert(
        "audio.library".into(),
        json!({"clips":{"a":{"file":{"path":"a.wav","gain":2}}}}),
    );
    assert!(format!("{:#}", world.compile(&registry).unwrap_err()).contains("gain"));
    world.resources.insert(
        "audio.library".into(),
        json!({"clips":{"a":{"file":{"path":"a.wav"}}}}),
    );
    world.compile(&registry).unwrap();
    let schema = axiom_core::audio::library_schema().to_string();
    assert!(
        schema.contains("file") && schema.contains(".ogg"),
        "{schema}"
    );
}
