//! Animation controller (`animation.controllers` + `animation.controller`): parameter-driven
//! state changes, 1D blend by measured speed, triggers and exit time, protocol / gameplay
//! parameter commands, deterministic replay, explicit checkpoint refusal, hot reload, and
//! allocation-free steady stepping. Uses `content/animator-lab` (the mascot's own clips).
use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::world::{ActionMap, WorldRuntime};
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Counts allocations made by the current thread while `COUNTING` is set (the test harness
/// runs other tests on other threads).
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
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/animator-lab"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn run(r: &mut WorldRuntime, ticks: u32) {
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
}

fn controller(r: &WorldRuntime) -> Value {
    r.observation().metrics["animation"]["players"]["walker"]["controller"].clone()
}

fn call(
    r: &mut WorldRuntime,
    capability: &str,
    action: &str,
    args: Value,
) -> Result<Value, String> {
    r.command(&RuntimeCommand::Call {
        capability: capability.into(),
        action: action.into(),
        args,
    })
    .map_err(|e| format!("{e:#}"))
}

fn set_speed(r: &mut WorldRuntime, speed: f32) {
    call(r, "path", "set_speed", json!({"id":"walker","speed":speed})).unwrap();
}

#[test]
fn measured_speed_drives_idle_walk_run_and_a_trigger_waves() {
    let mut r = WorldRuntime::new(lab(), 1, axiom_runtime::installed_registry()).unwrap();
    // The pack starts walking (1.2 m/s); stand still first.
    set_speed(&mut r, 0.0);
    run(&mut r, 10);
    let c = controller(&r);
    assert_eq!(c["controller"], "locomotion", "{c}");
    assert_eq!(c["state"], "idle", "{c}");
    // Walking along the path: the speed parameter follows the measured planar speed.
    set_speed(&mut r, 1.0);
    run(&mut r, 120);
    let c = controller(&r);
    assert_eq!(c["state"], "move", "{c}");
    let speed = c["params"]["speed"].as_f64().unwrap();
    assert!((speed - 1.0).abs() < 0.1, "{c}");
    assert_eq!(c["previous"], Value::Null, "the 0.25 s fade is over: {c}");
    // Running: the blend tree moves toward the run clip (no state change).
    set_speed(&mut r, 3.0);
    run(&mut r, 60);
    let c = controller(&r);
    assert_eq!(c["state"], "move", "{c}");
    assert!(c["params"]["speed"].as_f64().unwrap() > 2.8, "{c}");
    // Stopping fades back to idle.
    set_speed(&mut r, 0.0);
    run(&mut r, 2);
    let c = controller(&r);
    assert_eq!(c["state"], "idle", "{c}");
    assert_eq!(c["previous"], "move", "cross-fade in progress: {c}");
    run(&mut r, 60);
    // The wave trigger (a gameplay / protocol command) plays the one-shot, which returns
    // to idle at exit time 1.
    let reply = call(
        &mut r,
        "animation",
        "param",
        json!({"entity":"walker","name":"wave","value":true}),
    )
    .unwrap();
    assert_eq!(
        reply["result"]["controller"]["params"]["wave"], true,
        "{reply}"
    );
    run(&mut r, 1);
    let c = controller(&r);
    assert_eq!(c["state"], "wave", "{c}");
    assert_eq!(c["params"]["wave"], false, "a trigger is consumed: {c}");
    run(&mut r, 260);
    assert_eq!(controller(&r)["state"], "idle");
    assert_eq!(controller(&r)["transitions"], 4);
    r.assert_valid().unwrap();
}

#[test]
fn commands_on_the_wrong_kind_of_player_fail_with_reasons() {
    let mut r = WorldRuntime::new(lab(), 1, axiom_runtime::installed_registry()).unwrap();
    let play = call(
        &mut r,
        "animation",
        "play",
        json!({"entity":"walker","clip":"wave"}),
    )
    .unwrap_err();
    assert!(
        play.contains("driven by animation controller locomotion"),
        "{play}"
    );
    let unknown = call(
        &mut r,
        "animation",
        "param",
        json!({"entity":"walker","name":"jump","value":true}),
    )
    .unwrap_err();
    assert!(unknown.contains("unknown parameter jump"), "{unknown}");
    let kind = call(
        &mut r,
        "animation",
        "param",
        json!({"entity":"walker","name":"speed","value":true}),
    )
    .unwrap_err();
    assert!(kind.contains("is a float"), "{kind}");
    // An FSM / action clip cue on a controlled entity is counted, not silently lost.
    let mut w = lab();
    w.entities
        .iter_mut()
        .find(|e| e.id == "walker")
        .unwrap()
        .components
        .remove("animation.controller");
    let mut plain = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    let e = call(
        &mut plain,
        "animation",
        "param",
        json!({"entity":"walker","name":"speed","value":1.0}),
    )
    .unwrap_err();
    assert!(e.contains("has no animation.controller"), "{e}");
}

#[test]
fn a_missing_clip_or_controller_is_an_explicit_error() {
    let mut w = lab();
    w.resources.get_mut("animation.controllers").unwrap()["controllers"]["locomotion"]["states"]
        ["idle"]["motion"]["clip"] = json!("nope");
    let e = WorldRuntime::new(w, 1, axiom_runtime::installed_registry())
        .err()
        .map(|e| format!("{e:#}"))
        .unwrap();
    assert!(e.contains("clip nope not found"), "{e}");
    let mut w = lab();
    w.entities
        .iter_mut()
        .find(|e| e.id == "walker")
        .unwrap()
        .components
        .insert("animation.controller".into(), json!({"controller":"ghost"}));
    let e = axiom_runtime::installed_registry()
        .authoring
        .validate_world(&w)
        .err()
        .map(|e| format!("{e:#}"))
        .unwrap();
    assert!(e.contains("animation controller ghost missing"), "{e}");
}

fn scenario() -> axiom_runtime::scenario::Scenario {
    let call = |tick: u32, capability: &str, action: &str, args: Value| json!({"tick":tick,"operation":{"op":"command","command":{"kind":"call","capability":capability,"action":action,"args":args}}});
    serde_json::from_value(json!({
        "ticks": 900,
        "commands": [
            call(30, "path", "set_speed", json!({"id":"walker","speed":1.5})),
            call(300, "path", "set_speed", json!({"id":"walker","speed":3})),
            call(500, "path", "set_speed", json!({"id":"walker","speed":0})),
            call(800, "animation", "param", json!({"entity":"walker","name":"wave","value":true}))
        ],
        "assertions": [
            {"kind":"equals","pointer":"/animation/players/walker/controller/state","value":"wave"}
        ]
    }))
    .unwrap()
}

#[test]
fn scenario_replay_has_the_same_state_hash() {
    let a = axiom_runtime::scenario::run(lab(), 1, axiom_runtime::installed_registry(), scenario())
        .unwrap();
    let b = axiom_runtime::scenario::run(lab(), 1, axiom_runtime::installed_registry(), scenario())
        .unwrap();
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["state_hash"], b["state_hash"]);
    // The controller state is part of the hash: a different trigger tick changes it.
    let mut later = scenario();
    later.commands[3].tick = 810;
    let c =
        axiom_runtime::scenario::run(lab(), 1, axiom_runtime::installed_registry(), later).unwrap();
    assert_ne!(a["state_hash"], c["state_hash"]);
}

#[test]
fn checkpoints_refuse_animation_state_explicitly() {
    let mut w = lab();
    w.capabilities.insert("save".into(), 1);
    // path.move has no checkpoint support either; leave only the animation system to refuse.
    w.systems.retain(|s| s != "path.move");
    w.entities
        .iter_mut()
        .find(|e| e.id == "walker")
        .unwrap()
        .components
        .remove("path.follow");
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 5);
    let e = format!("{:#}", r.checkpoint().unwrap_err());
    assert!(e.contains("animation.evaluate"), "{e}");
}

#[test]
fn hot_reload_keeps_the_current_state_and_parameters() {
    let mut r = WorldRuntime::new(lab(), 1, axiom_runtime::installed_registry()).unwrap();
    set_speed(&mut r, 2.0);
    run(&mut r, 90);
    assert_eq!(controller(&r)["state"], "move");
    // Edit the controller (a new state + slower fade) like a graph edit during Play.
    let mut w = lab();
    let c = &mut w.resources.get_mut("animation.controllers").unwrap()["controllers"]["locomotion"];
    c["states"]["sit"] = json!({"motion":{"kind":"clip","clip":"sit"}});
    c["transitions"][1]["duration"] = json!(0.5);
    r.reconcile(w, 2).unwrap();
    let c = controller(&r);
    assert_eq!(c["state"], "move", "{c}");
    assert!(c["params"]["speed"].as_f64().unwrap() > 1.5, "{c}");
}

/// Allocations made by 240 steady steps of `w` (after a 30-step warm-up; `poke` runs first).
fn step_allocations(w: World, poke: bool) -> u64 {
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    if poke {
        call(
            &mut r,
            "animation",
            "param",
            json!({"entity":"walker","name":"wave","value":true}),
        )
        .unwrap();
    }
    run(&mut r, 30);
    ALLOCATIONS.with(|a| a.set(0));
    COUNTING.with(|c| c.set(true));
    // 240 steps cover the wave one-shot, its cross-fade back and looping idle.
    run(&mut r, 240);
    COUNTING.with(|c| c.set(false));
    if poke {
        assert_eq!(controller(&r)["state"], "idle");
        assert_eq!(controller(&r)["transitions"], 2);
    }
    ALLOCATIONS.with(Cell::get)
}

#[test]
fn steady_controller_steps_do_not_allocate() {
    // The same world with the walker standing still, without any animation player (the
    // baseline: the world step's own allocations), with a plain clip player, and driven by
    // the controller (wave trigger, exit time, cross-fade, idle loop).
    let still = || {
        let mut w = lab();
        let walker = w.entities.iter_mut().find(|e| e.id == "walker").unwrap();
        walker.components.remove("path.follow");
        w
    };
    let mut none = still();
    let walker = none.entities.iter_mut().find(|e| e.id == "walker").unwrap();
    walker.components.remove("animation.controller");
    walker.components.remove("animation.player");
    let mut plain = still();
    plain
        .entities
        .iter_mut()
        .find(|e| e.id == "walker")
        .unwrap()
        .components
        .remove("animation.controller");
    let baseline = step_allocations(none, false);
    let player = step_allocations(plain, false);
    let controlled = step_allocations(still(), true);
    println!("allocations in 240 steps: no player {baseline}, clip player {player}, controller {controlled}");
    assert_eq!(player, baseline, "a clip player allocated while stepping");
    assert_eq!(
        controlled, baseline,
        "the controller allocated while stepping"
    );
}
