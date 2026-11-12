//! Character expression on `content/character-lab`: glTF morph targets driven by clip weight
//! tracks and `animation.morph` (ramped, playback kept), primitive visibility
//! (`animation.player.hidden`, `animation.visibility`) down to the render frame, controller
//! layers (additive, bone mask, weight parameter), checkpoints, determinism and
//! allocation-free steady steps.
use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::world::{ActionMap, WorldRuntime};
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;
thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

fn lab() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/character-lab"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap()
}

fn run(r: &mut WorldRuntime, ticks: u32) {
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> Result<Value, String> {
    r.command(&RuntimeCommand::Call {
        capability: "animation".into(),
        action: action.into(),
        args,
    })
    .map(|v| v["result"].clone())
    .map_err(|e| format!("{e:#}"))
}

fn player(r: &WorldRuntime, id: &str) -> Value {
    r.observation().metrics["animation"]["players"][id].clone()
}

fn weight(r: &WorldRuntime, id: &str, target: &str) -> f64 {
    player(r, id)["morphs"][target].as_f64().unwrap()
}

/// The drawn vertices and hidden-part mask of `id` in the render frame.
fn drawn(r: &WorldRuntime, id: &str) -> (Vec<glam::Vec3>, [u64; 4]) {
    let frame = r.render_frame();
    let m = frame.meshes.iter().find(|m| m.id == id).unwrap();
    (m.vertices.iter().map(|v| v.position).collect(), m.hidden)
}

#[test]
fn clip_weight_tracks_animate_morph_targets() {
    let mut r = runtime(lab());
    // `pulse`: stretch 0 -> 1 -> 0 and puff 0 -> 0.5 -> 0 over 2 s.
    run(&mut r, 120);
    assert!(
        (weight(&r, "face", "stretch") - 1.0).abs() < 0.02,
        "{}",
        player(&r, "face")
    );
    assert!((weight(&r, "face", "puff") - 0.5).abs() < 0.02);
    let (moved, _) = drawn(&r, "face");
    let (rest, _) = drawn(&r, "face2");
    // face2 has no clip: its equator vertex sits on the sphere (model space), face's is
    // pushed out by 20 % (puff 0.5).
    let i = 4 * 17;
    assert!((rest[i].x - 0.5).abs() < 1e-3, "{:?}", rest[i]);
    assert!((moved[i].x - 0.6).abs() < 0.02, "{:?}", moved[i]);
    // Players without targets report no morphs (their observation is unchanged).
    assert!(player(&r, "axi").get("morphs").is_none());
}

#[test]
fn morph_command_overrides_with_a_ramp_and_keeps_playback() {
    let mut r = runtime(lab());
    run(&mut r, 30);
    let before = player(&r, "face")["time"].as_f64().unwrap();
    let reply = call(
        &mut r,
        "morph",
        json!({"entity":"face","target":"puff","weight":1.0,"ramp_seconds":0.5}),
    )
    .unwrap();
    // Time is untouched by the command.
    assert_eq!(reply["time"].as_f64().unwrap(), before);
    run(&mut r, 30);
    let mid = weight(&r, "face", "puff");
    assert!(mid > 0.3 && mid < 0.9, "half way through the ramp: {mid}");
    run(&mut r, 40);
    assert_eq!(weight(&r, "face", "puff"), 1.0);
    // stretch is still driven by the clip.
    assert!(weight(&r, "face", "stretch") > 0.0);
    assert!(player(&r, "face")["time"].as_f64().unwrap() > before);
    // By index, then clear: back to the clip.
    call(
        &mut r,
        "morph",
        json!({"entity":"face","index":1,"weight":0.0}),
    )
    .unwrap();
    assert_eq!(weight(&r, "face", "puff"), 0.0);
    call(
        &mut r,
        "morph",
        json!({"entity":"face","target":"puff","clear":true}),
    )
    .unwrap();
    run(&mut r, 1);
    assert!(weight(&r, "face", "puff") > 0.0);
    // Errors are recoverable and name the problem.
    let e = call(
        &mut r,
        "morph",
        json!({"entity":"face","target":"grin","weight":1.0}),
    )
    .unwrap_err();
    assert!(
        e.contains("no morph target grin") && e.contains("stretch"),
        "{e}"
    );
    let e = call(
        &mut r,
        "morph",
        json!({"entity":"face","index":9,"weight":1.0}),
    )
    .unwrap_err();
    assert!(e.contains("out of range"), "{e}");
    assert!(call(&mut r, "morph", json!({"entity":"face","target":"puff"})).is_err());
    assert!(call(
        &mut r,
        "morph",
        json!({"entity":"face","target":"puff","weight":5.0})
    )
    .is_err());
    let e = call(
        &mut r,
        "morph",
        json!({"entity":"face","target":"puff","clear":true,"ramp_seconds":1.0}),
    )
    .unwrap_err();
    assert!(e.contains("clear"), "{e}");
}

#[test]
fn hot_reload_keeps_morph_overrides_and_runtime_parts_of_the_same_asset() {
    let mut r = runtime(lab());
    run(&mut r, 5);
    call(
        &mut r,
        "morph",
        json!({"entity":"face2","target":"puff","weight":0.8}),
    )
    .unwrap();
    call(
        &mut r,
        "visibility",
        json!({"entity":"axi","group":"hair","visible":false}),
    )
    .unwrap();
    // An unrelated edit (the floor moves) keeps both; an edited hidden list applies.
    let mut w = lab();
    w.entities
        .iter_mut()
        .find(|e| e.id == "floor")
        .unwrap()
        .components
        .get_mut("core.transform")
        .unwrap()["position"] = json!([0, -0.1, 0]);
    r.reconcile(w.clone(), 2).unwrap();
    run(&mut r, 1);
    assert!((weight(&r, "face2", "puff") - 0.8).abs() < 1e-6);
    assert_eq!(drawn(&r, "axi").1[0].count_ones(), 1);
    w.entities
        .iter_mut()
        .find(|e| e.id == "face2")
        .unwrap()
        .components
        .get_mut("animation.player")
        .unwrap()["hidden"] = json!([]);
    r.reconcile(w, 3).unwrap();
    assert_eq!(drawn(&r, "face2").1, [0; 4]);
}

#[test]
fn authored_and_runtime_visibility_reach_the_render_frame() {
    let mut r = runtime(lab());
    run(&mut r, 1);
    // face2 authors hidden: [hat] (material 2).
    assert_eq!(drawn(&r, "face2").1, [0b100, 0, 0, 0]);
    assert_eq!(player(&r, "face2")["hidden"], json!(["1/1/0"]));
    assert_eq!(drawn(&r, "face").1, [0; 4]);
    let parts = call(&mut r, "inspect", json!({"entity":"face2"})).unwrap()["parts"].clone();
    assert_eq!(parts[2]["visible"], false, "{parts}");
    assert_eq!(parts[2]["groups"], json!(["hat", "felt"]));
    call(
        &mut r,
        "visibility",
        json!({"entity":"face2","group":"hat","visible":true}),
    )
    .unwrap();
    assert_eq!(drawn(&r, "face2").1, [0; 4]);
    // A material name hides every primitive using it; the mascot hides its hair.
    call(
        &mut r,
        "visibility",
        json!({"entity":"axi","group":"hair","visible":false}),
    )
    .unwrap();
    let hidden = drawn(&r, "axi").1;
    assert_eq!(hidden[0].count_ones(), 1, "{hidden:?}");
    let e = call(
        &mut r,
        "visibility",
        json!({"entity":"axi","group":"cape","visible":false}),
    )
    .unwrap_err();
    assert!(
        e.contains("no primitive named cape") && e.contains("hair"),
        "{e}"
    );
    // An unknown authored part fails the build with the component path.
    let mut w = lab();
    w.entities
        .iter_mut()
        .find(|e| e.id == "face2")
        .unwrap()
        .components
        .get_mut("animation.player")
        .unwrap()["hidden"] = json!(["cape"]);
    let e = format!(
        "{:#}",
        WorldRuntime::new(w, 1, axiom_runtime::installed_registry())
            .err()
            .unwrap()
    );
    assert!(e.contains("no primitive named cape"), "{e}");
}

fn bone(r: &mut WorldRuntime, bone: &str) -> Value {
    call(r, "inspect", json!({"entity":"axi","bones":[bone]})).unwrap()["bones"][bone]["local"]
        .clone()
}

#[test]
fn an_additive_layer_moves_only_its_masked_bones_by_its_weight() {
    let mut on = runtime(lab());
    let mut off = runtime(lab());
    call(
        &mut off,
        "param",
        json!({"entity":"axi","name":"wave_weight","value":0.0}),
    )
    .unwrap();
    run(&mut on, 50);
    run(&mut off, 50);
    let c = player(&on, "axi")["controller"].clone();
    assert_eq!(c["layers"][0]["name"], "wave_arm", "{c}");
    assert_eq!(c["layers"][0]["weight"], 1.0);
    assert_eq!(
        player(&off, "axi")["controller"]["layers"][0]["weight"],
        0.0
    );
    // Inside the mask (upper.R subtree) the pose differs; outside it is the same idle.
    assert_ne!(bone(&mut on, "fore.R"), bone(&mut off, "fore.R"));
    assert_eq!(bone(&mut on, "upper.L"), bone(&mut off, "upper.L"));
    assert_eq!(bone(&mut on, "head"), bone(&mut off, "head"));
}

fn hash(r: &WorldRuntime) -> String {
    axiom_runtime::scenario::state_hash(r).unwrap()
}

#[test]
fn replay_is_deterministic_and_checkpoints_round_trip_everything() {
    let script = |r: &mut WorldRuntime| {
        run(r, 25);
        call(
            r,
            "morph",
            json!({"entity":"face2","target":"puff","weight":1.0,"ramp_seconds":1.0}),
        )
        .unwrap();
        call(
            r,
            "visibility",
            json!({"entity":"face2","group":"hat","visible":true}),
        )
        .unwrap();
        call(
            r,
            "visibility",
            json!({"entity":"axi","group":"hair","visible":false}),
        )
        .unwrap();
        call(
            r,
            "param",
            json!({"entity":"axi","name":"wave_weight","value":0.4}),
        )
        .unwrap();
        call(
            r,
            "play",
            json!({"entity":"face","clip":"pulse_cubic","fade_seconds":0.5}),
        )
        .unwrap();
        run(r, 20);
    };
    let (mut a, mut b) = (runtime(lab()), runtime(lab()));
    script(&mut a);
    script(&mut b);
    assert_eq!(hash(&a), hash(&b));
    let mut w = lab();
    w.capabilities.insert("save".into(), 1);
    let mut r = runtime(w.clone());
    script(&mut r);
    let saved = r.checkpoint().unwrap();
    let text = serde_json::to_string(&saved).unwrap();
    let saved: axiom_runtime::checkpoint::Checkpoint = serde_json::from_str(&text).unwrap();
    let players = &saved.systems["animation.evaluate"]["players"];
    assert!(
        players["face2"]["morphs"]["puff"]["ramp"].is_array(),
        "{players}"
    );
    assert_eq!(players["face2"]["hidden"], json!([]), "{players}");
    assert!(players["axi"]["hidden"]
        .as_array()
        .is_some_and(|h| h.len() == 1));
    assert!(players["face"]["fade"]["from_morph"].is_array());
    assert!(players["axi"]["controller"]["layers"][0]["frac"].is_number());
    let mut restored = runtime(w.clone());
    restored.restore_checkpoint(&saved).unwrap();
    assert_eq!(hash(&restored), hash(&r));
    assert_eq!(drawn(&restored, "face2"), drawn(&r, "face2"));
    run(&mut r, 90);
    run(&mut restored, 90);
    assert_eq!(
        hash(&restored),
        hash(&r),
        "replay after the checkpoint differs"
    );
    assert_eq!(drawn(&restored, "face2"), drawn(&r, "face2"));
    // A save naming a layer the content no longer has is refused.
    let mut wrong = saved.clone();
    wrong.systems.get_mut("animation.evaluate").unwrap()["players"]["axi"]["controller"]
        ["layers"][0]["name"] = json!("other");
    let e = format!("{:#}", runtime(w).restore_checkpoint(&wrong).unwrap_err());
    assert!(e.contains("layers do not match"), "{e}");
}

#[test]
fn steady_steps_with_morphs_ramps_and_layers_do_not_allocate() {
    let count = |w: World, poke: bool| {
        let mut r = runtime(w);
        if poke {
            call(
                &mut r,
                "morph",
                json!({"entity":"face2","target":"puff","weight":1.0,"ramp_seconds":4.0}),
            )
            .unwrap();
        }
        run(&mut r, 30);
        ALLOCATIONS.with(|a| a.set(0));
        COUNTING.with(|c| c.set(true));
        run(&mut r, 240);
        COUNTING.with(|c| c.set(false));
        ALLOCATIONS.with(Cell::get)
    };
    let mut none = lab();
    for e in &mut none.entities {
        e.components.remove("animation.player");
        e.components.remove("animation.controller");
    }
    let baseline = count(none, false);
    let full = count(lab(), true);
    println!("allocations in 240 steps: no players {baseline}, morphs + ramp + layer {full}");
    assert_eq!(full, baseline, "morph / layer stepping allocated");
}
