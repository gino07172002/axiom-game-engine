//! Secondary motion (`animation.spring`, `animation.look_at`, IK sockets) on the procedural
//! `content/rig-lab` puppets: visible spring motion, deterministic replay, one-step socket
//! lag, look-at limits, checkpoints, hot reload, typed build errors and allocation-free
//! stepping.
use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::world::{ActionMap, WorldRuntime};
use glam::{Mat4, Quat, Vec3};
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
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/rig-lab"),
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
    .map_err(|e| format!("{e:#}"))
}

fn component<'a>(w: &'a mut World, entity: &str, id: &str) -> &'a mut Value {
    w.entities
        .iter_mut()
        .find(|e| e.id == entity)
        .unwrap()
        .components
        .get_mut(id)
        .unwrap()
}

fn rig(r: &WorldRuntime, entity: &str) -> Value {
    r.observation().metrics["animation"]["players"][entity]["rig"].clone()
}

fn vec3(v: &Value) -> Vec3 {
    Vec3::new(
        v[0].as_f64().unwrap() as f32,
        v[1].as_f64().unwrap() as f32,
        v[2].as_f64().unwrap() as f32,
    )
}

/// Model-space position and rotation of `bone` (`animation.inspect`).
fn bone(r: &mut WorldRuntime, entity: &str, bone: &str) -> (Vec3, Quat) {
    let v = call(r, "inspect", json!({"entity":entity,"bones":[bone]})).unwrap();
    let b = &v["result"]["bones"][bone];
    let q = &b["rotation"];
    (
        vec3(&b["position"]),
        Quat::from_xyzw(
            q[0].as_f64().unwrap() as f32,
            q[1].as_f64().unwrap() as f32,
            q[2].as_f64().unwrap() as f32,
            q[3].as_f64().unwrap() as f32,
        ),
    )
}

/// Entity world matrices of the lab (static transforms).
fn model(w: &World, entity: &str) -> Mat4 {
    let t: axiom_core::Transform =
        serde_json::from_value(w.entity(entity).unwrap().components["core.transform"].clone())
            .unwrap();
    t.matrix()
}

fn hash(r: &WorldRuntime) -> String {
    axiom_runtime::scenario::state_hash(r).unwrap()
}

#[test]
fn the_ponytail_swings_with_the_sway_and_replays_identically() {
    let mut springy = runtime(lab());
    let mut stiff = {
        let mut w = lab();
        component(&mut w, "puppet/a", "animation.spring")["chains"]["ponytail"]["enabled"] =
            json!(false);
        runtime(w)
    };
    let mut lag = 0.0f32;
    for _ in 0..8 {
        run(&mut springy, 30);
        run(&mut stiff, 30);
        let a = bone(&mut springy, "puppet/a", "hair.5").0;
        let b = bone(&mut stiff, "puppet/a", "hair.5").0;
        lag = lag.max(a.distance(b));
    }
    // The tip trails the animated ponytail by centimeters while the body sways.
    assert!(lag > 0.03, "spring tip only {lag} m from the animated tip");
    let tips = rig(&springy, "puppet/a");
    assert!(tips["spring"]["ponytail"].is_array(), "{tips}");
    // Same content, same steps: same state (spring points and look-at angles are hashed).
    let mut again = runtime(lab());
    run(&mut again, 240);
    assert_eq!(hash(&again), hash(&springy));
    // A different stiffness gives a different simulation.
    let mut w = lab();
    component(&mut w, "puppet/a", "animation.spring")["chains"]["ponytail"]["stiffness"] =
        json!(3.0);
    let mut other = runtime(w);
    run(&mut other, 240);
    assert_ne!(hash(&other), hash(&springy));
}

#[test]
fn scenario_replay_is_deterministic() {
    let scenario = || -> axiom_runtime::scenario::Scenario {
        serde_json::from_value(json!({
            "ticks": 360,
            "commands": [
                {"tick":120,"operation":{"op":"command","command":{"kind":"call","capability":"animation","action":"speed","args":{"entity":"puppet/a","speed":1.6}}}}
            ],
            "assertions": []
        }))
        .unwrap()
    };
    let run = || {
        axiom_runtime::scenario::run(lab(), 1, axiom_runtime::installed_registry(), scenario())
            .unwrap()
    };
    let (a, b) = (run(), run());
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["state_hash"], b["state_hash"]);
}

#[test]
fn the_hand_follows_the_other_puppets_hand_one_step_late() {
    let w = lab();
    let (a_model, b_model) = (model(&w, "puppet/a"), model(&w, "puppet/b"));
    let mut r = runtime(w);
    run(&mut r, 60);
    // A's left hand (plus the socket offset) at the end of this step, in world space...
    let (p, q) = bone(&mut r, "puppet/a", "hand.L");
    let socket = a_model.transform_point3(p + q * Vec3::new(0.05, 0.14, 0.06));
    run(&mut r, 1);
    // ...is exactly B's IK target during the next step (in B's model space).
    let resolved = vec3(&rig(&r, "puppet/b")["sockets"]["hand"]);
    let expected = b_model.inverse().transform_point3(socket);
    assert!(
        resolved.distance(expected) < 1e-5,
        "{resolved} vs {expected}"
    );
    // B's hand reached it (the sway keeps it inside the arm's reach).
    let ik = &r.observation().metrics["animation"]["players"]["puppet/b"]["ik"]["hand"];
    assert_eq!(ik["reachable"], true, "{ik}");
    assert!(ik["residual"].as_f64().unwrap() < 1e-3, "{ik}");
    let hand = bone(&mut r, "puppet/b", "hand.R").0;
    assert!(hand.distance(expected) < 1e-3);
    // ..through a whole 2 s sway cycle.
    for _ in 0..24 {
        run(&mut r, 10);
        let ik = &r.observation().metrics["animation"]["players"]["puppet/b"]["ik"]["hand"];
        assert_eq!(ik["reachable"], true, "{ik}");
    }
    // Rebinding at runtime: a fixed target unbinds, a socket binds again.
    call(
        &mut r,
        "ik",
        json!({"entity":"puppet/b","id":"hand","target":[-0.21,0.45,0.1]}),
    )
    .unwrap();
    assert!(rig(&r, "puppet/b").get("sockets").is_none());
    let e = call(
        &mut r,
        "ik",
        json!({"entity":"puppet/b","id":"hand","socket":{"entity":"puppet/a","bone":"tail"}}),
    )
    .unwrap_err();
    assert!(e.contains("bone not found: tail"), "{e}");
    call(
        &mut r,
        "ik",
        json!({"entity":"puppet/b","id":"hand","socket":{"entity":"puppet/a","bone":"hand.L"}}),
    )
    .unwrap();
    run(&mut r, 1);
    assert!(rig(&r, "puppet/b")["sockets"]["hand"].is_array());
}

#[test]
fn the_head_turns_toward_its_target_within_the_limits() {
    let mut w = lab();
    // A stands still and looks at a point; B keeps looking at A's head.
    component(&mut w, "puppet/a", "animation.player")["clip"] = Value::Null;
    let look = component(&mut w, "puppet/a", "animation.look_at");
    look["chains"]["gaze"]["target"] = json!({"kind":"point","position":[1.8,1.4,1.0]});
    look["chains"]["gaze"]["smoothing"] = json!(0.1);
    let a_model = model(&w, "puppet/a");
    let mut r = runtime(w.clone());
    run(&mut r, 240);
    let angles = &rig(&r, "puppet/a")["look_at"]["gaze"];
    // Yaw toward +X is positive and below the 70 degree limit; pitch is small and upward.
    let yaw = angles[0].as_f64().unwrap();
    let pitch = angles[1].as_f64().unwrap();
    let head = a_model.transform_point3(bone(&mut r, "puppet/a", "head").0);
    let to = Vec3::new(1.8, 1.4, 1.0) - head;
    let expected = to.x.atan2(to.z).to_degrees() as f64;
    assert!(
        (yaw - expected).abs() < 1.5,
        "yaw {yaw} expected {expected}"
    );
    assert!(pitch.abs() < 10.0, "pitch {pitch}");
    let facing = bone(&mut r, "puppet/a", "head").1 * Vec3::Z;
    assert!(
        facing.angle_between(to.normalize()) < 0.15,
        "{facing} vs {to}"
    );
    // Behind the puppet: clamped at max_yaw.
    let look = component(&mut w, "puppet/a", "animation.look_at");
    look["chains"]["gaze"]["target"] = json!({"kind":"point","position":[-1.0,1.3,-2.0]});
    let mut r = runtime(w.clone());
    run(&mut r, 240);
    let yaw = rig(&r, "puppet/a")["look_at"]["gaze"][0].as_f64().unwrap();
    assert!((yaw.abs() - 70.0).abs() < 0.01, "{yaw}");
    // Weight 0: no turn.
    component(&mut w, "puppet/a", "animation.look_at")["chains"]["gaze"]["weight"] = json!(0.0);
    let mut r = runtime(w);
    run(&mut r, 60);
    assert_eq!(rig(&r, "puppet/a")["look_at"]["gaze"], json!([0.0, 0.0]));
    // B (entity bone target) looks toward A, i.e. to its -X: negative yaw.
    let yaw = rig(&r, "puppet/b")["look_at"]["gaze"][0].as_f64().unwrap();
    assert!(yaw < -45.0, "{yaw}");
}

#[test]
fn the_camera_target_follows_the_active_camera() {
    let mut w = lab();
    component(&mut w, "puppet/a", "animation.player")["clip"] = Value::Null;
    let mut r = runtime(w.clone());
    run(&mut r, 240);
    let right = rig(&r, "puppet/a")["look_at"]["gaze"][0].as_f64().unwrap();
    // The camera is to A's front right: positive yaw. Move it to the left: negative yaw.
    component(&mut w, "camera/main", "core.transform")["position"] = json!([-1.6, 1.35, 2.6]);
    let mut r = runtime(w);
    run(&mut r, 240);
    let left = rig(&r, "puppet/a")["look_at"]["gaze"][0].as_f64().unwrap();
    assert!(right > 10.0 && left < -10.0, "right {right} left {left}");
}

fn savable() -> World {
    let mut w = lab();
    w.capabilities.insert("save".into(), 1);
    w
}

#[test]
fn checkpoints_carry_springs_look_at_and_sockets() {
    let mut r = runtime(savable());
    run(&mut r, 75);
    let saved = r.checkpoint().unwrap();
    let text = serde_json::to_string(&saved).unwrap();
    let saved: axiom_runtime::checkpoint::Checkpoint = serde_json::from_str(&text).unwrap();
    let player = &saved.systems["animation.evaluate"]["players"]["puppet/b"]["rig"];
    assert!(
        player["springs"]["ponytail"].is_array() && player["sockets"]["hand"].is_array(),
        "{player}"
    );
    let mut restored = runtime(savable());
    restored.restore_checkpoint(&saved).unwrap();
    assert_eq!(hash(&restored), hash(&r), "restored state differs");
    run(&mut r, 200);
    run(&mut restored, 200);
    assert_eq!(
        hash(&restored),
        hash(&r),
        "replay after the checkpoint differs"
    );
    // A save without the rig state (as an older build wrote it) is refused, not reset.
    let mut old = saved.clone();
    old.systems.get_mut("animation.evaluate").unwrap()["players"]["puppet/a"]
        .as_object_mut()
        .unwrap()
        .remove("rig");
    let e = format!(
        "{:#}",
        runtime(savable()).restore_checkpoint(&old).unwrap_err()
    );
    assert!(
        e.contains("lacks spring / look-at / IK socket state"),
        "{e}"
    );
    // ..and so is one whose chain state does not match the content's chain.
    let mut short = saved.clone();
    short.systems.get_mut("animation.evaluate").unwrap()["players"]["puppet/a"]["rig"]["springs"]
        ["ponytail"]
        .as_array_mut()
        .unwrap()
        .pop();
    let e = format!(
        "{:#}",
        runtime(savable()).restore_checkpoint(&short).unwrap_err()
    );
    assert!(e.contains("spring chain ponytail does not match"), "{e}");
}

#[test]
fn hot_reload_keeps_an_unchanged_simulation_and_restarts_an_edited_one() {
    let mut a = runtime(lab());
    let mut b = runtime(lab());
    run(&mut a, 90);
    run(&mut b, 90);
    // An unrelated edit (the floor colour): the springs carry on as if nothing happened.
    let mut w = lab();
    component(&mut w, "stage/floor", "render.mesh")["parts"][0]["color"] = json!([0.2, 0.2, 0.2]);
    a.reconcile(w, 2).unwrap();
    run(&mut a, 30);
    run(&mut b, 30);
    assert_eq!(rig(&a, "puppet/a"), rig(&b, "puppet/a"));
    // Editing the chain restarts it from the animated pose (documented).
    let mut w = lab();
    component(&mut w, "puppet/a", "animation.spring")["chains"]["ponytail"]["drag"] = json!(0.5);
    a.reconcile(w, 3).unwrap();
    assert_ne!(rig(&a, "puppet/a")["spring"], rig(&b, "puppet/a")["spring"]);
}

#[test]
fn bad_rigs_fail_at_build_with_the_component_path() {
    let build = |edit: &dyn Fn(&mut World)| {
        let mut w = lab();
        edit(&mut w);
        match WorldRuntime::new(w, 1, axiom_runtime::installed_registry()) {
            Ok(_) => String::new(),
            Err(e) => format!("{e:#}"),
        }
    };
    let e = build(&|w| {
        component(w, "puppet/a", "animation.spring")["chains"]["ponytail"]["root"] = json!("hair.9")
    });
    assert!(
        e.contains("animation.spring") && e.contains("bone not found: hair.9"),
        "{e}"
    );
    let e = build(&|w| {
        component(w, "puppet/b", "animation.player")["ik"]["hand"]["socket"]["bone"] = json!("nope")
    });
    assert!(
        e.contains("/ik/hand/socket") && e.contains("bone not found: nope"),
        "{e}"
    );
    let e = build(&|w| {
        component(w, "puppet/b", "animation.look_at")["chains"]["gaze"]["target"]["bone"] =
            json!("nope")
    });
    assert!(e.contains("animation.look_at/chains/gaze/target"), "{e}");
    let e = build(&|w| {
        component(w, "puppet/a", "animation.look_at")["chains"]["gaze"]["bones"] =
            json!([{"bone":"head","weight":0.5},{"bone":"neck","weight":0.5}])
    });
    assert!(e.contains("descendant of the previous"), "{e}");
}

/// Allocations of 240 steady steps of `w` after a warm-up.
fn step_allocations(w: World) -> u64 {
    let mut r = runtime(w);
    run(&mut r, 30);
    ALLOCATIONS.with(|a| a.set(0));
    COUNTING.with(|c| c.set(true));
    run(&mut r, 240);
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.with(Cell::get)
}

#[test]
fn springs_and_look_at_do_not_allocate_while_stepping() {
    // Puppet A alone (B's IK solver allocates, as before this feature).
    let only_a = || {
        let mut w = lab();
        w.entities.retain(|e| e.id != "puppet/b");
        w
    };
    let mut plain = only_a();
    let a = w_entity(&mut plain, "puppet/a");
    a.components.remove("animation.spring");
    a.components.remove("animation.look_at");
    let baseline = step_allocations(plain);
    let rigged = step_allocations(only_a());
    println!("allocations in 240 steps: clip player {baseline}, with spring + look-at {rigged}");
    assert_eq!(
        rigged, baseline,
        "springs / look-at allocated while stepping"
    );
}

fn w_entity<'a>(w: &'a mut World, id: &str) -> &'a mut axiom_core::Entity {
    w.entities.iter_mut().find(|e| e.id == id).unwrap()
}

#[test]
fn springs_start_at_rest_where_the_entity_stands() {
    // A still puppet away from the origin, no gravity, no look-at, no
    // colliders: the ponytail must not move
    // (the first state is taken in world space, not at the origin).
    let mut w = lab();
    component(&mut w, "puppet/a", "animation.player")["clip"] = Value::Null;
    component(&mut w, "puppet/a", "core.transform")["position"] = json!([-0.6, 0.0, 0.4]);
    component(&mut w, "puppet/a", "core.transform")["rotation"] = json!([0.0, 1.0, 0.0, 0.0]);
    component(&mut w, "puppet/a", "animation.spring")["chains"]["ponytail"]["gravity"] =
        json!([0, 0, 0]);
    component(&mut w, "puppet/a", "animation.look_at")["chains"]["gaze"]["weight"] = json!(0.0);
    // (The head collider overlaps the authored ponytail and would push it out.)
    component(&mut w, "puppet/a", "animation.spring")["colliders"] = json!({});
    let mut still = w.clone();
    component(&mut still, "puppet/a", "animation.spring")["chains"]["ponytail"]["enabled"] =
        json!(false);
    let (mut r, mut s) = (runtime(w), runtime(still));
    for _ in 0..6 {
        run(&mut r, 10);
        run(&mut s, 10);
        let a = bone(&mut r, "puppet/a", "hair.5").0;
        let b = bone(&mut s, "puppet/a", "hair.5").0;
        assert!(a.distance(b) < 1e-4, "{a} vs {b}");
    }
}
