//! Positional audio (`audio.source` / `audio.listener` + `audio.spatial`): attenuation and
//! constant-power panning through the runtime, listener selection, the voice budget and
//! deterministic stealing, one-shots and loops, playback from rules and bindings, authoring
//! errors and budgets, hot reload, spawns and checkpoints, and allocation-free steady steps and
//! mixing. Everything runs headless: no audio device is opened.
use axiom_core::{audio, gameplay::RuntimeCommand, Diagnostic, World};
use axiom_runtime::{
    audio_mix::{ClipTable, Mixer, VoiceParams, MAX_VOICES},
    installed_registry, ActionMap, WorldRuntime,
};
use serde_json::{json, Value};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

/// Counts allocations of the current thread only (tests run in parallel).
struct Counting;
thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;
fn allocations() -> u64 {
    ALLOCS.with(Cell::get)
}

/// A listener `ear` at the origin (facing -Z, right = +X) plus `entities`.
fn world(entities: Value) -> World {
    let mut all = vec![json!({"id":"ear","components":{"core.transform":{},"audio.listener":{}}})];
    all.extend(entities.as_array().cloned().unwrap_or_default());
    serde_json::from_value(json!({
        "schema_version":2,"id":"audio-test","title":"Audio test",
        "capabilities":{"core":1,"render":1,"audio":1,"motion":1},
        "systems":["motion.integrate","audio.spatial"],
        "entities":all,
        "resources":{"audio.library":{"clips":{
            "hum":{"frequency":110,"seconds":1,"attack":0,"release":0},
            "click":{"wave":"noise","seconds":0.25,"lowpass_hz":900},
            "other":{"frequency":220,"seconds":1}
        }}}
    }))
    .unwrap()
}

fn source(id: &str, at: [f32; 3], extra: Value) -> Value {
    let mut s = json!({"clip":"hum","loop":true,"autoplay":true});
    s.as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap_or_default());
    json!({"id":id,"components":{"core.transform":{"position":at},"audio.source":s}})
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, installed_registry()).unwrap()
}

fn steps(r: &mut WorldRuntime, n: usize) {
    for _ in 0..n {
        r.step(&ActionMap::default());
    }
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "audio".into(),
        action: action.into(),
        args,
    })
    .map(|v| v["result"].clone())
}

fn inspect(r: &mut WorldRuntime, id: &str) -> Value {
    call(r, "inspect", json!({ "id": id })).unwrap()
}

fn summary(r: &WorldRuntime) -> Value {
    r.observation().metrics["audio"].clone()
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 2e-4
}

fn code(e: &anyhow::Error) -> String {
    e.chain()
        .find_map(|c| c.downcast_ref::<Diagnostic>())
        .map(|d| d.code.clone())
        .unwrap_or_else(|| format!("{e:#}"))
}

#[test]
fn attenuation_and_panning_follow_the_listener() {
    let mut r = runtime(world(json!([
        source("right", [5.0, 0.0, 0.0], json!({"max_distance":31})),
        source("left", [-5.0, 0.0, 0.0], json!({"max_distance":31})),
        source(
            "front",
            [0.0, 0.0, -2.0],
            json!({"rolloff":"linear","max_distance":5})
        ),
        source("far", [0.0, 0.0, 40.0], json!({"max_distance":31})),
        source(
            "flat",
            [30.0, 0.0, 0.0],
            json!({"spatial":false,"volume":0.5})
        ),
    ])));
    steps(&mut r, 1);
    let right = inspect(&mut r, "right");
    assert!(close(f(&right["gain"]), 0.2), "{right}");
    assert!(close(f(&right["pan"]), 1.0) && close(f(&right["left"]), 0.0));
    assert!(close(f(&right["right"]), 0.2));
    let left = inspect(&mut r, "left");
    assert!(close(f(&left["pan"]), -1.0) && close(f(&left["left"]), 0.2));
    assert!(close(f(&left["right"]), 0.0));
    let front = inspect(&mut r, "front");
    let want = audio::attenuation(audio::Rolloff::Linear, 1.0, 5.0, 2.0);
    assert!(close(want, 0.75) && close(f(&front["gain"]), want));
    let c = std::f32::consts::FRAC_1_SQRT_2 * 0.75;
    assert!(close(f(&front["left"]), c) && close(f(&front["right"]), c));
    let far = inspect(&mut r, "far");
    assert_eq!(far["gain"], 0.0);
    assert_eq!(
        far["voice"],
        Value::Null,
        "beyond max_distance takes no voice"
    );
    let flat = inspect(&mut r, "flat");
    assert!(close(f(&flat["gain"]), 0.5) && close(f(&flat["pan"]), 0.0));
    let s = summary(&r);
    assert_eq!(
        (s["audible"].clone(), s["culled"].clone()),
        (json!(4), json!(1))
    );
    assert_eq!(s["listener"]["id"], "ear");
    assert_eq!(s["listener"]["from"], "audio.listener");
    // The voice block the host publishes carries the same gains.
    let sys = r
        .system::<axiom_runtime::spatial_audio::AudioSpatialSystem>()
        .unwrap();
    let v = right["voice"].as_u64().unwrap() as usize;
    assert!(close(sys.voices()[v].right, 0.2) && sys.voices()[v].looping);
    // Turning the listener around swaps the ears.
    let ear = r.state.compiled.indices["ear"];
    r.state.transforms[ear].as_mut().unwrap().rotation =
        glam::Quat::from_rotation_y(std::f32::consts::PI).to_array();
    steps(&mut r, 1);
    assert!(close(f(&inspect(&mut r, "right")["pan"]), -1.0));
}

#[test]
fn moving_past_the_listener_rises_falls_and_sweeps_the_pan() {
    // A car drives along x at 10 m/s, 3 m in front of the listener.
    let car = json!({"id":"car","components":{"core.transform":{"position":[-20,0,-3]},
        "core.motion":{"velocity":[10,0,0]},
        "audio.source":{"clip":"hum","loop":true,"autoplay":true,"max_distance":25}}});
    let mut r = runtime(world(json!([car])));
    let mut samples = vec![];
    for _ in 0..40 {
        steps(&mut r, 12);
        let s = inspect(&mut r, "car");
        samples.push((f(&s["gain"]), f(&s["pan"])));
    }
    let loudest = samples
        .iter()
        .enumerate()
        .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
        .unwrap()
        .0;
    assert!(samples[..loudest].windows(2).all(|w| w[1].0 >= w[0].0));
    assert!(samples[loudest..].windows(2).all(|w| w[1].0 <= w[0].0));
    assert!(samples[0].1 < -0.9 && samples[39].1 > 0.9, "{samples:?}");
    assert!(
        samples.windows(2).all(|w| w[1].1 >= w[0].1 - 1e-6),
        "pan sweeps left to right"
    );
}

#[test]
fn listener_falls_back_to_the_camera_and_conflicts_are_errors() {
    let mut w = world(json!([
        {"id":"cam/off","components":{"core.transform":{"position":[50,0,0]},
            "render.camera":{"projection":{"kind":"perspective","fov_degrees":60},"near":0.1,"far":100,"viewport":[0,0,1,1],"active":false}}},
        {"id":"cam","components":{"core.transform":{"position":[2,0,0]},
            "render.camera":{"projection":{"kind":"perspective","fov_degrees":60},"near":0.1,"far":100,"viewport":[0,0,1,1],"active":true}}},
        source("s", [4.0, 0.0, 0.0], json!({})),
        source("flat", [0.0, 0.0, 0.0], json!({"spatial":false})),
    ]));
    w.entities[0].components.remove("audio.listener");
    let mut r = runtime(w.clone());
    steps(&mut r, 1);
    let s = summary(&r);
    assert_eq!(s["listener"]["id"], "cam");
    assert_eq!(s["listener"]["from"], "camera");
    assert!(
        close(f(&inspect(&mut r, "s")["gain"]), 0.5),
        "2 m from the camera"
    );
    // No listener and no active camera: spatial sources are silent and observe says why.
    let mut silent = w.clone();
    silent.entities.retain(|e| !e.id.starts_with("cam"));
    let mut r = runtime(silent);
    steps(&mut r, 1);
    let s = summary(&r);
    assert_eq!(s["listener"], Value::Null);
    assert!(s["listener_note"].as_str().unwrap().contains("silent"));
    assert_eq!(inspect(&mut r, "s")["gain"], 0.0);
    assert!(
        close(f(&inspect(&mut r, "flat")["gain"]), 1.0),
        "2D still plays"
    );
    // Two active listeners are an authoring error; an inactive second one is fine.
    let mut two =
        world(json!([{"id":"ear2","components":{"core.transform":{},"audio.listener":{}}}]));
    let e = WorldRuntime::new(two.clone(), 1, installed_registry())
        .err()
        .unwrap();
    assert_eq!(code(&e), "audio_listener_conflict");
    two.entities[1]
        .components
        .insert("audio.listener".into(), json!({"active":false}));
    runtime(two);
}

#[test]
fn voice_budget_steals_by_priority_then_distance_deterministically() {
    let n = MAX_VOICES + 8;
    let mut entities: Vec<Value> = (0..n)
        .map(|k| {
            source(
                &format!("s/{k:02}"),
                [k as f32 + 1.0, 0.0, 0.0],
                json!({"max_distance":200}),
            )
        })
        .collect();
    // The farthest source outranks everything.
    entities[n - 1]["components"]["audio.source"]["priority"] = json!(5);
    let w = world(Value::Array(entities));
    let mut a = runtime(w.clone());
    steps(&mut a, 1);
    let s = summary(&a);
    assert_eq!(s["voiced"], MAX_VOICES);
    assert_eq!(s["virtual"], 8);
    let voiced = |r: &mut WorldRuntime, k: usize| {
        call(r, "inspect", json!({"id":format!("s/{k:02}")})).unwrap()["voice"].clone()
    };
    assert_ne!(voiced(&mut a, n - 1), Value::Null, "priority first");
    assert_ne!(voiced(&mut a, MAX_VOICES - 2), Value::Null);
    assert_eq!(voiced(&mut a, MAX_VOICES - 1), Value::Null, "then distance");
    assert_ne!(voiced(&mut a, 3), Value::Null);
    let slot = voiced(&mut a, 20);
    // The listener moves to the far end: near sources lose their voices (steals) and voices
    // still held keep their slots.
    let ear = a.state.compiled.indices["ear"];
    a.state.transforms[ear].as_mut().unwrap().position = [n as f32, 0.0, 0.0];
    steps(&mut a, 1);
    let s = summary(&a);
    assert_eq!(s["steals"], 8);
    assert_eq!(voiced(&mut a, 3), Value::Null);
    assert_ne!(voiced(&mut a, MAX_VOICES - 1), Value::Null);
    assert_eq!(voiced(&mut a, 20), slot, "a kept voice keeps its slot");
    // Same input, same result (the clip cache size is process-wide; leave it out).
    let mut b = runtime(w);
    steps(&mut b, 1);
    let ear = b.state.compiled.indices["ear"];
    b.state.transforms[ear].as_mut().unwrap().position = [n as f32, 0.0, 0.0];
    steps(&mut b, 1);
    let strip = |mut v: Value| {
        v.as_object_mut().unwrap().remove("clips");
        v
    };
    assert_eq!(strip(summary(&b)), strip(summary(&a)));
}

#[test]
fn one_shots_stop_at_the_clip_end_and_loops_wrap() {
    let mut r = runtime(world(json!([
        source(
            "shot",
            [1.0, 0.0, 0.0],
            json!({"clip":"click","loop":false,"autoplay":false})
        ),
        source("loop", [1.0, 0.0, 0.0], json!({"pitch":2.0})),
    ])));
    steps(&mut r, 1);
    assert_eq!(inspect(&mut r, "shot")["playing"], false);
    assert_eq!(
        call(&mut r, "play", json!({"id":"shot"})).unwrap()["playing"],
        true
    );
    // The first step after play publishes the clip start (offset 0), then time advances.
    steps(&mut r, 1);
    let v = inspect(&mut r, "shot")["voice"].as_u64().unwrap() as usize;
    let sys = r
        .system::<axiom_runtime::spatial_audio::AudioSpatialSystem>()
        .unwrap();
    assert_eq!(sys.voices()[v].offset, 0.0);
    steps(&mut r, 28); // 0.2333 s of 0.25
    let s = inspect(&mut r, "shot");
    assert_eq!(s["playing"], true);
    assert_ne!(s["voice"], Value::Null);
    steps(&mut r, 3);
    let s = inspect(&mut r, "shot");
    assert_eq!(
        (s["playing"].clone(), s["voice"].clone()),
        (json!(false), Value::Null)
    );
    assert!(close(f(&s["time"]), 0.25));
    assert_eq!(summary(&r)["finished"], 1);
    // play again restarts a finished one-shot; stop rewinds.
    call(&mut r, "play", json!({"id":"shot"})).unwrap();
    steps(&mut r, 7);
    assert!(close(f(&inspect(&mut r, "shot")["time"]), 0.05));
    let stopped = call(&mut r, "stop", json!({"id":"shot"})).unwrap();
    assert_eq!(
        (stopped["playing"].clone(), stopped["time"].clone()),
        (json!(false), json!(0.0))
    );
    // The loop at pitch 2 covers its 1 s clip every 0.5 s: 151 steps = 150 advances (1.25 s): 0.5.
    steps(&mut r, 151 - 40);
    let l = inspect(&mut r, "loop");
    assert_eq!(l["playing"], true);
    assert!(close(f(&l["time"]), 0.5), "{l}");
    // Errors are explicit.
    assert_eq!(
        code(&call(&mut r, "play", json!({"id":"ear"})).unwrap_err()),
        "not_found"
    );
    assert!(call(&mut r, "play", json!({})).is_err());
}

#[test]
fn rules_bindings_and_tween_finish_play_sounds() {
    let mut w = world(json!([
        {"id":"door","components":{"core.transform":{},
            "interact.target":{"range":2.0,"event":"use_door"},
            "tween.clips":{"clips":{"swing":{"on_finish":"door_moved","tracks":[
                {"property":"core.transform/yaw","keys":[{"t":0,"v":0},{"t":1,"v":90}]}]}}},
            "audio.source":{"clip":"click"}}},
        {"id":"latch","components":{"core.transform":{},"audio.source":{"clip":"click"}}},
        {"id":"hero","tags":["player"],"components":{"core.transform":{"position":[1,0,0]}}},
    ]));
    for c in ["tween", "interact", "behavior"] {
        w.capabilities.insert(c.into(), 1);
    }
    w.systems = vec![
        "behavior.execute".into(),
        "interact.focus".into(),
        "tween.animate".into(),
        "audio.spatial".into(),
    ];
    w.resources.insert(
        "input.bindings".into(),
        json!({"buttons":{"interact":"KeyE"}}),
    );
    w.resources.insert(
        "behavior.program".into(),
        json!({"events":{"use_door":{"fields":{}},"open_door":{"fields":{}},"door_moved":{"fields":{}}},
            "rules":[{"id":"open","event":"use_door","actions":[{"kind":"emit","event":"open_door"}]}]}),
    );
    w.resources.insert(
        "runtime.bindings".into(),
        json!({"events":[
            {"event":"open_door","calls":[
                {"kind":"call","capability":"tween","action":"play","args":{"id":"door","clip":"swing"}},
                {"kind":"call","capability":"audio","action":"play","args":{"id":"door"}}]},
            {"event":"door_moved","calls":[{"kind":"call","capability":"audio","action":"play","args":{"id":"latch"}}]}
        ]}),
    );
    let mut r = runtime(w);
    steps(&mut r, 1);
    let mut input = ActionMap::default();
    input.0.insert("interact".into(), 1.0);
    r.step(&input);
    assert_eq!(
        inspect(&mut r, "door")["playing"],
        true,
        "interact -> rule -> binding"
    );
    assert_eq!(inspect(&mut r, "latch")["playing"], false);
    steps(&mut r, 125);
    assert_eq!(
        inspect(&mut r, "latch")["playing"],
        true,
        "tween on_finish -> binding"
    );
    assert_eq!(r.observation().metrics["bindings"]["fault"], Value::Null);
}

#[test]
fn authoring_errors_and_budgets_are_explicit() {
    let compile = |w: World| WorldRuntime::new(w, 1, installed_registry()).err().unwrap();
    let e = compile(world(json!([source(
        "s",
        [0.0; 3],
        json!({"clip":"nope"})
    )])));
    assert_eq!(code(&e), "missing_clip");
    assert!(format!("{e:#}").contains("click, hum, other"), "{e:#}");
    let mut w = world(json!([source("s", [0.0; 3], json!({}))]));
    w.systems.retain(|s| s != "audio.spatial");
    assert_eq!(code(&compile(w)), "invalid_audio");
    let mut w = world(json!([{"id":"s","components":{"audio.source":{"clip":"hum"}}}]));
    assert_eq!(code(&compile(w.clone())), "invalid_audio");
    w.entities[1]
        .components
        .insert("audio.source".into(), json!({"clip":"hum","spatial":false}));
    runtime(w);
    let many: Vec<Value> = (0..=audio::MAX_AUDIO_SOURCES)
        .map(|k| source(&format!("s/{k}"), [0.0; 3], json!({})))
        .collect();
    assert_eq!(code(&compile(world(Value::Array(many)))), "audio_budget");
    let mut w = world(json!([]));
    let clips: serde_json::Map<String, Value> = (0..30)
        .map(|k| (format!("c{k}"), json!({"seconds":audio::MAX_CLIP_SECONDS})))
        .collect();
    w.resources
        .insert("audio.library".into(), json!({ "clips": clips }));
    assert!(format!("{:#}", compile(w)).contains("memory budget"));
}

#[test]
fn hot_reload_keeps_playback_unless_the_clip_changes() {
    let w = world(json!([
        source("s", [1.0, 0.0, 0.0], json!({})),
        source("quiet", [1.0, 0.0, 0.0], json!({"autoplay":false})),
        source("near", [0.5, 0.0, 0.0], json!({"autoplay":false})),
    ]));
    let mut r = runtime(w.clone());
    steps(&mut r, 60);
    // `near` starts later and ranks first, but `s` keeps voice 0 and `near` takes voice 1;
    // a reload must keep both slots (not re-deal them in rank order).
    call(&mut r, "play", json!({"id":"near"})).unwrap();
    steps(&mut r, 1);
    let near = inspect(&mut r, "near")["voice"].clone();
    assert_eq!(near, 1);
    let before = inspect(&mut r, "s");
    let voice = before["voice"].clone();
    let token = r
        .system::<axiom_runtime::spatial_audio::AudioSpatialSystem>()
        .unwrap()
        .voices()[voice.as_u64().unwrap() as usize]
        .token;
    let mut louder = w.clone();
    louder.entities[1]
        .components
        .get_mut("audio.source")
        .unwrap()["volume"] = json!(2.0);
    r.reconcile(louder.clone(), 2).unwrap();
    steps(&mut r, 1);
    let after = inspect(&mut r, "s");
    assert!(close(f(&after["time"]), f(&before["time"]) + 1.0 / 120.0));
    assert!(close(f(&after["gain"]), 2.0));
    assert_eq!(after["voice"], voice, "same voice slot");
    assert_eq!(inspect(&mut r, "near")["voice"], near);
    let sys = r
        .system::<axiom_runtime::spatial_audio::AudioSpatialSystem>()
        .unwrap();
    let v = after["voice"].as_u64().unwrap() as usize;
    assert_eq!(
        sys.voices()[v].token,
        token,
        "the audio thread continues the same voice"
    );
    assert_eq!(inspect(&mut r, "quiet")["playing"], false);
    // A different clip restarts as authored (autoplay from 0).
    let mut swapped = louder;
    swapped.entities[1]
        .components
        .get_mut("audio.source")
        .unwrap()["clip"] = json!("other");
    r.reconcile(swapped, 3).unwrap();
    steps(&mut r, 1);
    let s = inspect(&mut r, "s");
    assert_eq!(s["clip"], "other");
    assert_eq!(
        f(&s["time"]),
        0.0,
        "restarted: the first step plays the clip start"
    );
}

#[test]
fn spawned_audio_halts_and_checkpoints_refuse_explicitly() {
    let mut r = runtime(world(json!([source("s", [1.0, 0.0, 0.0], json!({}))])));
    steps(&mut r, 1);
    let template = r.state.compiled.entities[r.state.compiled.indices["s"]].clone();
    r.state.spawn(&template, "s/2".into(), [0.0; 3]).unwrap();
    steps(&mut r, 1);
    let s = summary(&r);
    assert!(s["halted"].as_str().unwrap().contains("s/2"), "{s}");
    assert_eq!(s["voiced"], 0);
    assert!(call(&mut r, "play", json!({"id":"s"})).is_err());
    let mut w = world(json!([]));
    w.capabilities.insert("save".into(), 1);
    w.systems.retain(|s| s != "motion.integrate");
    let error = format!("{:#}", runtime(w).checkpoint().unwrap_err());
    assert!(
        error.contains("audio.spatial does not implement checkpoints"),
        "{error}"
    );
}

#[test]
fn steady_steps_and_mixing_do_not_allocate() {
    // 256 moving sources compete for 32 voices.
    let sources: Vec<Value> = (0..256)
        .map(|k| {
            let mut s = source(
                &format!("s/{k}"),
                [
                    (k % 16) as f32 * 3.0 - 24.0,
                    0.0,
                    (k / 16) as f32 * 3.0 - 24.0,
                ],
                json!({"max_distance":60,"priority":k % 3}),
            );
            s["components"]["core.motion"] = json!({"velocity":[0.5,0,0.25]});
            s
        })
        .collect();
    let mut r = runtime(world(Value::Array(sources)));
    steps(&mut r, 10);
    let before = allocations();
    steps(&mut r, 200);
    assert_eq!(allocations() - before, 0, "steady steps allocate");
    assert_eq!(summary(&r)["voiced"], MAX_VOICES);
    // The mixer with every voice busy.
    let sys = r
        .system::<axiom_runtime::spatial_audio::AudioSpatialSystem>()
        .unwrap();
    let table: std::sync::Arc<ClipTable> = sys.clip_table().clone();
    let voices: [VoiceParams; MAX_VOICES] = *sys.voices();
    assert!(voices.iter().all(VoiceParams::active));
    let mut mixer = Mixer::default();
    mixer.set_clips(table);
    mixer.set_params(&voices);
    let mut out = vec![0i16; 1024];
    mixer.render(&mut out);
    let before = allocations();
    for _ in 0..200 {
        mixer.set_params(&voices);
        mixer.render(&mut out);
    }
    assert_eq!(allocations() - before, 0, "the mixer allocates");
    assert_eq!(mixer.stats.voices as usize, MAX_VOICES);
    assert!(out.iter().any(|&s| s != 0));
}
