//! Gamepad input through `input.bindings.gamepad`: merging with the keyboard, the deadzone,
//! headless pad injection driving the FPS and action packs exactly like the keyboard, and pad
//! look. No controller hardware is read (`Gamepads` stays disabled outside the native host).
use axiom_core::{ContentPackage, InputBindings, World};
use axiom_runtime::{
    gamepad::{PadInput, Pads},
    installed_registry, scenario, ActionMap, WorldRuntime,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn pack(name: &str) -> World {
    ContentPackage::load(
        format!("{}/../../content/{name}", env!("CARGO_MANIFEST_DIR")),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn pads(input: Value) -> Pads {
    let mut p = Pads::default();
    p.0[1] = Some(
        serde_json::from_value::<PadInput>(input)
            .unwrap()
            .state()
            .unwrap(),
    );
    p
}

fn run(world: World, scenario: &str) -> Value {
    scenario::run(
        world,
        1,
        installed_registry(),
        serde_json::from_str(scenario).unwrap(),
    )
    .unwrap()
}

#[test]
fn pad_values_merge_with_the_keyboard_and_vanish_without_a_pad() {
    let bindings: InputBindings = serde_json::from_value(json!({
        "axes":{"move":{"positive":"KeyD","negative":"KeyA"}},
        "buttons":{"jump":"Space","fire":"MouseLeft"},
        "gamepad":{"axes":{"move":{"positive":"PadLStickRight","negative":"PadLStickLeft"}},
                   "buttons":{"jump":"PadA","fire":"PadRT"},"deadzone":0.2}
    }))
    .unwrap();
    let none = BTreeSet::new();
    let keys: BTreeSet<String> = ["KeyA".to_owned()].into();
    // No pad: exactly the keyboard map.
    let plain = ActionMap::from_bindings(&bindings, &keys);
    assert_eq!(
        ActionMap::from_input(&bindings, &keys, &Pads::default()).0,
        plain.0
    );
    // Half stick right (after the 0.2 deadzone: (0.5-0.2)/0.8) plus the A key's -1.
    let half = pads(json!({"left_stick":[0.5,0.0],"buttons":["PadA"],"right_trigger":0.3}));
    let a = ActionMap::from_input(&bindings, &keys, &half);
    assert!(
        (a.axis("move") - (0.375 - 1.0)).abs() < 1e-3,
        "{}",
        a.axis("move")
    );
    assert!(a.pressed("jump"));
    assert!(!a.pressed("fire"), "trigger below 0.5 is not a press");
    let a = ActionMap::from_input(
        &bindings,
        &none,
        &pads(json!({"left_stick":[0.1,0.1],"right_trigger":1.0})),
    );
    assert_eq!(a.axis("move"), 0.0, "inside the deadzone");
    assert!(a.pressed("fire"));
    // Full left on the stick and the D key cancel; values stay within -1..1.
    let keys: BTreeSet<String> = ["KeyD".to_owned()].into();
    let a = ActionMap::from_input(&bindings, &keys, &pads(json!({"left_stick":[-1.0,0.0]})));
    assert_eq!(a.axis("move"), 0.0);
    let a = ActionMap::from_input(&bindings, &keys, &pads(json!({"left_stick":[1.0,0.0]})));
    assert_eq!(a.axis("move"), 1.0);
}

#[test]
fn an_injected_pad_drives_the_fps_pack_like_the_keyboard_scenario() {
    let keyboard = run(
        pack("fps"),
        include_str!("../../../examples/fps-regression.scenario.json"),
    );
    let pad = run(
        pack("fps"),
        include_str!("../../../examples/fps-gamepad.scenario.json"),
    );
    assert_eq!(pad["ok"], true, "{pad}");
    // Full stick right + RT = move.right 1 + fire, so the run is identical to the keys.
    assert_eq!(pad["state_hash"], keyboard["state_hash"]);
    // Without the gamepad bindings the same pad does nothing.
    let mut unbound = pack("fps");
    unbound
        .resources
        .get_mut("input.bindings")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("gamepad");
    let idle = scenario::run(
        unbound,
        1,
        installed_registry(),
        serde_json::from_str(include_str!("../../../examples/fps-gamepad.scenario.json")).unwrap(),
    );
    assert!(
        idle.is_err(),
        "the player stays put, so the position assertion fails"
    );
}

#[test]
fn the_right_stick_turns_the_fps_camera_at_the_look_rate() {
    let world = pack("fps");
    let bindings: InputBindings =
        serde_json::from_value(world.resources["input.bindings"].clone()).unwrap();
    let mut r = WorldRuntime::new(world, 1, installed_registry()).unwrap();
    let look = ActionMap::from_input(
        &bindings,
        &BTreeSet::new(),
        &pads(json!({"right_stick":[1.0,0.0]})),
    );
    assert_eq!(look.axis("look.x"), 1.0);
    let heading = |r: &WorldRuntime| {
        let i = r.state.compiled.indices["camera/fps"];
        let q = glam::Quat::from_array(r.state.transforms[i].as_ref().unwrap().rotation);
        let f = q * -glam::Vec3::Z;
        f.x.atan2(-f.z)
    };
    r.step(&ActionMap::default());
    let before = heading(&r);
    for _ in 0..60 {
        r.step(&look);
    }
    // Half a second at 3 rad/s: 1.5 rad to the right (heading measured from -Z toward +X).
    let turned = (heading(&r) - before).rem_euclid(std::f32::consts::TAU);
    assert!((turned - 1.5).abs() < 0.02, "{turned}");
    // Stick up looks up (pitch is clamped, so a short push only).
    let up = ActionMap::from_input(
        &bindings,
        &BTreeSet::new(),
        &pads(json!({"right_stick":[0.0,1.0]})),
    );
    let i = r.state.compiled.indices["camera/fps"];
    let pitch = |r: &WorldRuntime| {
        let q = glam::Quat::from_array(r.state.transforms[i].as_ref().unwrap().rotation);
        (q * -glam::Vec3::Z).y
    };
    let level = pitch(&r);
    for _ in 0..10 {
        r.step(&up);
    }
    assert!(pitch(&r) > level + 0.1, "{level} -> {}", pitch(&r));
}

#[test]
fn an_injected_pad_moves_the_action_hero() {
    let start = pack("action");
    let at = |w: &World| {
        w.entity("hero").unwrap().components["core.transform"]["position"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<_>>()
    };
    let origin = at(&start);
    let result = run(
        start,
        include_str!("../../../examples/action-gamepad.scenario.json"),
    );
    assert_eq!(result["ok"], true, "{result}");
    let mut r = WorldRuntime::new(pack("action"), 1, installed_registry()).unwrap();
    let bindings: InputBindings =
        serde_json::from_value(pack("action").resources["input.bindings"].clone()).unwrap();
    let forward = ActionMap::from_input(
        &bindings,
        &BTreeSet::new(),
        &pads(json!({"left_stick":[0.0,1.0]})),
    );
    assert_eq!(forward.axis("move_z"), 1.0);
    for _ in 0..30 {
        r.step(&forward);
    }
    let i = r.state.compiled.indices["hero"];
    let p = r.state.transforms[i].as_ref().unwrap().position;
    let moved = ((p[0] as f64 - origin[0]).powi(2) + (p[2] as f64 - origin[2]).powi(2)).sqrt();
    assert!(moved > 0.5, "hero moved {moved} m from {origin:?} to {p:?}");
}
