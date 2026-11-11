//! Rigid bodies (`physics.body` + `physics.collider` + `physics.step`): gravity onto static
//! scenery, shapes and contact materials, sleeping, kinematic bodies and followers pushing
//! dynamic bodies, determinism, checkpoints, hot reload and authoring errors.
use axiom_core::{gameplay::RuntimeCommand, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use glam::Vec3;
use serde_json::{json, Value};

fn world(entities: Value) -> World {
    serde_json::from_value(json!({
        "schema_version":2,"id":"physics-test","title":"Physics test",
        "capabilities":{"core":1,"physics":1},
        "systems":["physics.step"],
        "entities":entities,
    }))
    .unwrap()
}

/// A 40 x 1 x 40 m static floor whose top is at y = 0.
fn floor() -> Value {
    json!({"id":"floor","components":{"core.transform":{"position":[0,-0.5,0]},
        "physics.collider":{"shape":{"shape":"box","half_extents":[20,0.5,20]}}}})
}

fn crate_at(id: &str, at: [f32; 3]) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at},
        "physics.collider":{"shape":{"shape":"box","half_extents":[0.5,0.5,0.5]}},
        "physics.body":{}}})
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, installed_registry()).unwrap()
}

fn steps(r: &mut WorldRuntime, n: usize) {
    for _ in 0..n {
        r.step(&ActionMap::default());
    }
    r.assert_valid().unwrap();
}

fn position(r: &WorldRuntime, id: &str) -> Vec3 {
    Vec3::from_array(
        r.state.transforms[r.state.compiled.indices[id]]
            .as_ref()
            .unwrap()
            .position,
    )
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "physics".into(),
        action: action.into(),
        args,
    })
    .map(|v| v["result"].clone())
}

#[test]
fn a_dynamic_box_falls_onto_a_static_floor_and_sleeps() {
    let mut r = runtime(world(json!([floor(), crate_at("box", [0.0, 5.0, 0.0])])));
    let summary = call(&mut r, "summary", json!({})).unwrap();
    assert_eq!(summary["dynamic"], 1);
    assert_eq!(summary["static"], 1);
    steps(&mut r, 60);
    let falling = position(&r, "box");
    assert!(falling.y < 4.0 && falling.y > 1.0, "{falling}");
    steps(&mut r, 300);
    let rest = position(&r, "box");
    assert!((rest.y - 0.5).abs() < 0.02, "{rest}");
    assert!(rest.x.abs() < 0.01 && rest.z.abs() < 0.01, "{rest}");
    steps(&mut r, 360);
    let summary = call(&mut r, "summary", json!({})).unwrap();
    assert_eq!(summary["sleeping"], 1, "{summary}");
    assert_eq!(summary["awake"], 0, "{summary}");
    let inspect = call(&mut r, "inspect", json!({"id":"box"})).unwrap();
    assert_eq!(inspect["kind"], "dynamic");
    assert_eq!(inspect["sleeping"], true);
    assert_eq!(r.observation().metrics["physics"]["sleeping"], 1);
}

fn ball(id: &str, at: [f32; 3], restitution: f32) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at},
        "physics.collider":{"shape":{"shape":"sphere","radius":0.25},"restitution":restitution},
        "physics.body":{}}})
}

/// Highest center height after the first contact (a bounce).
fn bounce_height(restitution: f32) -> f32 {
    let mut r = runtime(world(json!([
        floor(),
        ball("ball", [0.0, 3.0, 0.0], restitution)
    ])));
    let mut touched = false;
    let mut best = 0.0f32;
    for _ in 0..480 {
        r.step(&ActionMap::default());
        let y = position(&r, "ball").y;
        if y < 0.3 {
            touched = true;
        } else if touched {
            best = best.max(y);
        }
    }
    best
}

#[test]
fn spheres_bounce_by_restitution_and_capsules_rest_on_their_caps() {
    let lively = bounce_height(0.8);
    let dead = bounce_height(0.0);
    assert!(lively > 1.2, "restitution 0.8 bounced to {lively}");
    assert!(dead < 0.4, "restitution 0 bounced to {dead}");
    let mut r = runtime(world(json!([floor(),
        {"id":"pill","components":{"core.transform":{"position":[0,2,0]},
            "physics.collider":{"shape":{"shape":"capsule","half_height":0.5,"radius":0.3}},
            "physics.body":{"mass":2.0}}}])));
    steps(&mut r, 600);
    // Upright capsules tip over; at rest the center is one radius above the floor (lying)
    // or half_height + radius (standing).
    let y = position(&r, "pill").y;
    assert!((y - 0.3).abs() < 0.03 || (y - 0.8).abs() < 0.03, "{y}");
    assert_eq!(
        call(&mut r, "inspect", json!({"id":"pill"})).unwrap()["mass"],
        2.0
    );
}

/// A crate released on a ramp tilted 30° about Z (descending toward +X); returns how far it got
/// along X in 2 s.
fn slide(friction: f32) -> f32 {
    let (s, c) = (15f32.to_radians().sin(), 15f32.to_radians().cos());
    let mut r = runtime(world(json!([floor(),
        {"id":"ramp","components":{"core.transform":{"position":[0,2,0],"rotation":[0,0,-s,c]},
            "physics.collider":{"shape":{"shape":"box","half_extents":[4,0.1,1]},"friction":friction}}},
        {"id":"crate","components":{"core.transform":{"position":[-1.532,3.36,0],"rotation":[0,0,-s,c]},
            "physics.collider":{"shape":{"shape":"box","half_extents":[0.3,0.3,0.3]},"friction":friction},
            "physics.body":{}}}])));
    let start = position(&r, "crate");
    steps(&mut r, 240);
    position(&r, "crate").x - start.x
}

#[test]
fn friction_holds_a_crate_on_a_ramp_that_a_slippery_one_slides_down() {
    let sticky = slide(1.0);
    let slippery = slide(0.0);
    assert!(sticky.abs() < 0.1, "friction 1 slid {sticky} m");
    assert!(slippery > 2.0, "friction 0 slid only {slippery} m");
}

#[test]
fn boxes_stack_and_settle() {
    let mut entities = vec![floor()];
    for i in 0..5 {
        entities.push(crate_at(
            &format!("stack/{i}"),
            [0.0, 0.5 + i as f32 * 1.05, 0.0],
        ));
    }
    let mut r = runtime(world(json!(entities)));
    steps(&mut r, 900);
    for i in 0..5 {
        let p = position(&r, &format!("stack/{i}"));
        assert!((p.y - (0.5 + i as f32)).abs() < 0.05, "stack/{i} at {p}");
        assert!(
            p.x.abs() < 0.05 && p.z.abs() < 0.05,
            "stack/{i} drifted to {p}"
        );
    }
    let summary = call(&mut r, "summary", json!({})).unwrap();
    assert_eq!(summary["sleeping"], 5, "{summary}");
}

#[test]
fn kinematic_bodies_and_moved_colliders_push_dynamic_bodies() {
    // A kinematic paddle sliding along +X and a capsule carried by core.motion (a follower)
    // each push a crate in their way; neither is pushed back.
    let mut r = runtime(
        serde_json::from_value(json!({
            "schema_version":2,"id":"push","title":"Push",
            "capabilities":{"core":1,"physics":1,"motion":1},
            "systems":["motion.integrate","physics.step"],
            "entities":[floor(),
                {"id":"paddle","components":{"core.transform":{"position":[-3,0.5,0]},
                    "physics.collider":{"shape":{"shape":"box","half_extents":[0.2,0.5,1]}},
                    "physics.body":{"kind":"kinematic","velocity":[1.5,0,0]}}},
                crate_at("crate/a", [0.0, 0.5, 0.0]),
                {"id":"walker","components":{"core.transform":{"position":[-3,0.8,6]},
                    "core.motion":{"velocity":[1.5,0,0]},
                    "physics.collider":{"shape":{"shape":"capsule","half_height":0.5,"radius":0.3}}}},
                crate_at("crate/b", [0.0, 0.5, 6.0])]
        }))
        .unwrap(),
    );
    let summary = call(&mut r, "summary", json!({})).unwrap();
    assert_eq!(summary["kinematic"], 1);
    assert_eq!(summary["followers"], 1);
    steps(&mut r, 480);
    let paddle = position(&r, "paddle");
    assert!(
        (paddle.x - 3.0).abs() < 0.05,
        "kinematic paddle at {paddle}"
    );
    assert!(
        position(&r, "crate/a").x > 2.0,
        "{}",
        position(&r, "crate/a")
    );
    let walker = position(&r, "walker");
    assert!(
        (walker.x - 3.0).abs() < 0.05,
        "core.motion keeps the walker's pose: {walker}"
    );
    assert!(
        position(&r, "crate/b").x > 2.0,
        "{}",
        position(&r, "crate/b")
    );
}

#[test]
fn identical_runs_and_checkpoints_are_deterministic() {
    let scene = || {
        let mut entities = vec![floor()];
        for i in 0..12 {
            let f = i as f32;
            entities.push(crate_at(
                &format!("c/{i}"),
                [
                    (f * 0.37).sin() * 1.5,
                    1.0 + f * 1.2,
                    (f * 0.53).cos() * 1.5,
                ],
            ));
        }
        let mut w = world(json!(entities));
        w.capabilities.insert("save".into(), 1);
        w
    };
    let mut a = runtime(scene());
    let mut b = runtime(scene());
    steps(&mut a, 200);
    steps(&mut b, 200);
    assert_eq!(a.state.transforms, b.state.transforms);
    let hash = |r: &WorldRuntime| axiom_runtime::scenario::state_hash(r).unwrap();
    assert_eq!(hash(&a), hash(&b));
    // Save mid-flight; loading continues with the saved velocities (two loads of the same
    // checkpoint stay identical, and the bodies keep moving instead of restarting at rest).
    let saved = a.checkpoint().unwrap();
    assert!(saved.systems.contains_key("physics.step"));
    let moving = call(&mut a, "summary", json!({})).unwrap()["awake"]
        .as_u64()
        .unwrap();
    assert!(moving > 0);
    let velocity = |r: &mut WorldRuntime| {
        call(r, "inspect", json!({"id":"c/11"})).unwrap()["velocity"].clone()
    };
    let before = velocity(&mut a);
    let mut c = runtime(scene());
    let mut d = runtime(scene());
    c.restore_checkpoint(&saved).unwrap();
    d.restore_checkpoint(&saved).unwrap();
    assert_eq!(velocity(&mut c), before);
    assert_eq!(c.state.transforms, a.state.transforms);
    steps(&mut c, 300);
    steps(&mut d, 300);
    assert_eq!(hash(&c), hash(&d));
    assert_eq!(c.state.tick, 500);
    // A checkpoint without the physics state is refused, not silently restarted at rest.
    let mut stripped = saved.clone();
    stripped.systems.clear();
    let error = format!("{:#}", d.restore_checkpoint(&stripped).unwrap_err());
    assert!(error.contains("physics.step"), "{error}");
}

#[test]
fn hot_reload_keeps_moving_bodies_and_restarts_edited_ones() {
    let start = world(json!([
        floor(),
        crate_at("a", [0.0, 6.0, 0.0]),
        crate_at("b", [3.0, 6.0, 0.0])
    ]));
    let mut r = runtime(start.clone());
    steps(&mut r, 40);
    let (a, b) = (position(&r, "a"), position(&r, "b"));
    assert!(a.y < 5.9);
    let mut next = start.clone();
    next.entity_mut("b")
        .unwrap()
        .components
        .insert("core.transform".into(), json!({"position":[3.0,8.0,0.0]}));
    r.reconcile(next, 2).unwrap();
    assert_eq!(
        position(&r, "a"),
        a,
        "untouched body keeps its simulated pose"
    );
    assert_eq!(
        position(&r, "b"),
        Vec3::new(3.0, 8.0, 0.0),
        "edited body restarts at its new pose"
    );
    let fall = call(&mut r, "inspect", json!({"id":"a"})).unwrap()["velocity"][1]
        .as_f64()
        .unwrap();
    assert!(fall < -2.0, "velocity carried across the reload: {fall}");
    assert_eq!(
        call(&mut r, "inspect", json!({"id":"b"})).unwrap()["velocity"][1],
        0.0
    );
    assert!(b.y < 5.9);
    steps(&mut r, 600);
    assert!((position(&r, "a").y - 0.5).abs() < 0.02);
}

#[test]
fn actions_wake_and_push_bodies_and_refuse_static_colliders() {
    let mut r = runtime(world(json!([floor(), crate_at("box", [0.0, 0.5, 0.0])])));
    steps(&mut r, 400);
    assert_eq!(
        call(&mut r, "inspect", json!({"id":"box"})).unwrap()["sleeping"],
        true
    );
    let pushed = call(&mut r, "impulse", json!({"id":"box","impulse":[0,6,0]})).unwrap();
    assert_eq!(pushed["sleeping"], false);
    steps(&mut r, 20);
    assert!(position(&r, "box").y > 0.8);
    call(
        &mut r,
        "set_velocity",
        json!({"id":"box","velocity":[2,0,0]}),
    )
    .unwrap();
    for (action, args) in [
        ("impulse", json!({"id":"floor","impulse":[0,1,0]})),
        ("impulse", json!({"id":"nobody","impulse":[0,1,0]})),
        ("impulse", json!({"id":"box","impulse":[0,1e9,0]})),
        ("set_velocity", json!({"id":"box"})),
        ("inspect", json!({"id":"nobody"})),
    ] {
        assert!(
            call(&mut r, action, args.clone()).is_err(),
            "{action} {args}"
        );
    }
    assert_eq!(
        call(&mut r, "inspect", json!({"id":"floor"})).unwrap()["kind"],
        "static"
    );
}

#[test]
fn bodies_that_leave_the_world_are_disabled_and_reported() {
    let mut r = runtime(world(json!([
        {"id":"rocket","components":{"core.transform":{"position":[0,0,0]},
            "physics.collider":{"shape":{"shape":"sphere","radius":0.2}},
            "physics.body":{"velocity":[0,1000,0],"gravity_scale":0.0}}}])));
    steps(&mut r, 1300);
    let summary = call(&mut r, "summary", json!({})).unwrap();
    assert_eq!(summary["out_of_bounds"], json!(["rocket"]), "{summary}");
    assert!(position(&r, "rocket").y <= 10_000.0);
    r.assert_valid().unwrap();
}

#[test]
fn authoring_errors_are_explicit() {
    let compile = |w: World| {
        w.compile(&installed_registry().authoring)
            .map(drop)
            .map_err(|e| format!("{e:#}"))
    };
    // A body without a collider, under a parent, with a second mover, or without the system.
    let lonely = world(json!([{"id":"x","components":{"core.transform":{},"physics.body":{}}}]));
    assert!(compile(lonely).unwrap_err().contains("no physics.collider"));
    let mut child = world(json!([
        crate_at("p", [0.0, 1.0, 0.0]),
        crate_at("c", [0.0, 2.0, 0.0])
    ]));
    child.relations.push(axiom_core::Relation {
        source: "c".into(),
        kind: "parent".into(),
        target: "p".into(),
    });
    let error = compile(child).unwrap_err();
    assert!(
        error.contains("child of p") || error.contains("under rigid body p"),
        "{error}"
    );
    let mut moved = world(json!([crate_at("m", [0.0, 1.0, 0.0])]));
    moved.capabilities.insert("motion".into(), 1);
    moved.systems.push("motion.integrate".into());
    moved
        .entity_mut("m")
        .unwrap()
        .components
        .insert("core.motion".into(), json!({}));
    assert!(compile(moved).unwrap_err().contains("physics_conflict"));
    let mut unscheduled = world(json!([crate_at("u", [0.0, 1.0, 0.0])]));
    unscheduled.systems.clear();
    assert!(compile(unscheduled)
        .unwrap_err()
        .contains("physics.step system"));
    let mut many = vec![];
    for i in 0..=axiom_core::physics::MAX_BODIES {
        many.push(crate_at(&format!("b/{i}"), [i as f32 * 2.0, 1.0, 0.0]));
    }
    assert!(compile(world(json!(many))).unwrap_err().contains("budget"));
    // Old content without the new fields is unchanged; a sphere collider is accepted
    // everywhere colliders are read (navigation, camera blockers).
    let c: axiom_core::Collider =
        serde_json::from_value(json!({"shape":{"shape":"box","half_extents":[1,1,1]}})).unwrap();
    assert_eq!(
        serde_json::to_value(&c).unwrap(),
        json!({"shape":{"shape":"box","half_extents":[1.0,1.0,1.0]}})
    );
    assert!(serde_json::from_value::<axiom_core::Collider>(
        json!({"shape":{"shape":"sphere","radius":1},"friction":11})
    )
    .map_err(anyhow::Error::from)
    .and_then(|c| c.validate())
    .is_err());
}

#[test]
fn the_fps_player_pushes_crates_without_turning_them_into_arena_walls() {
    let registry = installed_registry();
    let mut w = axiom_core::ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/fps"),
        &registry.authoring,
    )
    .unwrap()
    .world;
    w.systems.push("physics.step".into());
    // The FPS arena floor lives inside the FPS simulation (an authored floor collider would
    // also be an FPS cover box, which may not extend below ground): a floating crate.
    let mut crate_entity = crate_at("crate", [0.0, 0.9, 9.5]);
    crate_entity["components"]["physics.body"] = json!({"gravity_scale":0.0,"linear_damping":2.0});
    w.entities
        .push(serde_json::from_value(crate_entity).unwrap());
    let mut r = WorldRuntime::new(w, 1, registry).unwrap();
    let summary = call(&mut r, "summary", json!({})).unwrap();
    assert_eq!(summary["dynamic"], 1);
    assert!(
        summary["followers"].as_u64().unwrap() >= 2,
        "player + sentinels: {summary}"
    );
    let mut forward = ActionMap::default();
    forward.0.insert("move.forward".into(), 1.0);
    for _ in 0..120 {
        r.step(&forward);
    }
    r.assert_valid().unwrap();
    let pushed = position(&r, "crate");
    assert!(pushed.z < 9.0, "the player walked into the crate: {pushed}");
    assert!(position(&r, "player").z < 9.0, "a body is not an FPS wall");
}

#[test]
fn nav_agents_walk_through_dynamic_crates_pushing_them_and_around_static_ones() {
    // The crate is a mover (physics.body), so the navigation grid ignores it and the agent's
    // capsule (a follower) shoves it aside; the static wall stays a nav obstacle.
    let mut r = runtime(
        serde_json::from_value(json!({
            "schema_version":2,"id":"nav-push","title":"Nav push",
            "capabilities":{"core":1,"physics":1,"nav":1},
            "systems":["nav.steer","physics.step"],
            "entities":[floor(),
                {"id":"agent","components":{"core.transform":{"position":[-4,0.85,0]},
                    "nav.agent":{"speed":2.0,"target":{"point":[4,0,0]}},
                    "physics.collider":{"shape":{"shape":"capsule","half_height":0.5,"radius":0.35}}}},
                crate_at("crate", [2.6, 0.5, 0.0]),
                {"id":"wall","components":{"core.transform":{"position":[0,1,0]},
                    "physics.collider":{"shape":{"shape":"box","half_extents":[0.2,1,0.6]}}}}]
        }))
        .unwrap(),
    );
    steps(&mut r, 960);
    let state = r
        .command(&RuntimeCommand::Call {
            capability: "nav".into(),
            action: "inspect".into(),
            args: json!({"id":"agent"}),
        })
        .unwrap()["result"]["state"]
        .clone();
    assert_eq!(state, "arrived");
    let agent = position(&r, "agent");
    assert!(agent.x > 3.5, "{agent}");
    assert!(position(&r, "crate").distance(Vec3::new(2.6, 0.5, 0.0)) > 0.3);
}

#[test]
fn a_follower_turning_around_does_not_fling_the_crate_it_pushed() {
    // A path.follow porter pushes a crate to the end of a ping_pong path, where its facing
    // flips 180° in one step: that flip must not become a spin velocity on the crate.
    let mut r = runtime(
        serde_json::from_value(json!({
            "schema_version":2,"id":"porter","title":"Porter",
            "capabilities":{"core":1,"physics":1,"path":1},
            "systems":["path.move","physics.step"],
            "entities":[floor(),
                {"id":"route","components":{"core.transform":{},
                    "path.spline":{"points":[[3,0.85,0],[-3,0.85,0]]}}},
                {"id":"porter","components":{"core.transform":{"position":[3,0.85,0]},
                    "path.follow":{"path":"route","speed":1.5,"end":"ping_pong"},
                    "physics.collider":{"shape":{"shape":"capsule","half_height":0.5,"radius":0.35}}}},
                crate_at("crate", [1.0, 0.5, 0.0])]
        }))
        .unwrap(),
    );
    let mut fastest = 0.0f64;
    for _ in 0..900 {
        r.step(&ActionMap::default());
        let speed = call(&mut r, "inspect", json!({"id":"crate"})).unwrap()["speed"]
            .as_f64()
            .unwrap();
        fastest = fastest.max(speed);
    }
    r.assert_valid().unwrap();
    let p = position(&r, "crate");
    assert!(
        p.x < -3.0 && p.x > -5.0,
        "pushed to the end and left there: {p}"
    );
    assert!(
        fastest < 4.0,
        "the crate never moves much faster than the porter: {fastest}"
    );
}

#[test]
fn the_physics_lab_scenario_passes_and_replays_identically() {
    let run = || {
        let registry = installed_registry();
        let world = axiom_core::ContentPackage::load(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/physics-lab"),
            &registry.authoring,
        )
        .unwrap()
        .world;
        let scenario =
            serde_json::from_str(include_str!("../../../examples/physics-lab.scenario.json"))
                .unwrap();
        axiom_runtime::scenario::run(world, 1, registry, scenario).unwrap()
    };
    let (a, b) = (run(), run());
    assert_eq!(a["ok"], true);
    assert_eq!(a["state_hash"], b["state_hash"]);
}

#[test]
fn bodies_built_in_an_inactive_scene_collide_once_it_is_active() {
    let mut w = world(json!([floor(), crate_at("late", [0.0, 3.0, 0.0])]));
    w.scenes.push(axiom_core::Scene {
        id: "later".into(),
        entities: vec!["late".into()],
        active: false,
    });
    let mut r = runtime(w);
    steps(&mut r, 60);
    assert_eq!(position(&r, "late").y, 3.0, "inactive: not simulated");
    // Scene activation (what sync_scenes does when gameplay switches scenes).
    let slot = r.state.compiled.indices["late"];
    r.state.compiled.enabled[slot] = true;
    steps(&mut r, 360);
    assert!(
        (position(&r, "late").y - 0.5).abs() < 0.02,
        "{}",
        position(&r, "late")
    );
}

#[test]
fn hot_reload_applies_edited_velocities_and_moves_unbodied_colliders_with_their_entity() {
    let start = world(json!([floor(),
        {"id":"belt","components":{"core.transform":{"position":[0,0.5,-5]},
            "physics.collider":{"shape":{"shape":"box","half_extents":[0.5,0.5,0.5]}},
            "physics.body":{"kind":"kinematic","velocity":[1,0,0]}}},
        crate_at("lid", [8.0, 4.0, 0.0])]));
    let mut r = runtime(start.clone());
    steps(&mut r, 120);
    // Edited kinematic velocity takes effect at once.
    let mut next = start.clone();
    next.entity_mut("belt").unwrap().components.insert(
        "physics.body".into(),
        json!({"kind":"kinematic","velocity":[0,0,2]}),
    );
    r.reconcile(next.clone(), 2).unwrap();
    let v = call(&mut r, "inspect", json!({"id":"belt"})).unwrap()["velocity"].clone();
    assert_eq!(v, json!([0.0, 0.0, 2.0]));
    // The lid lands; removing its body leaves a static collider where it landed, so a crate
    // dropped onto that spot rests on top of it.
    steps(&mut r, 300);
    let landed = position(&r, "lid");
    assert!((landed.y - 0.5).abs() < 0.02);
    next.entity_mut("lid")
        .unwrap()
        .components
        .remove("physics.body");
    next.entities
        .push(serde_json::from_value(crate_at("drop", [landed.x, 3.0, landed.z])).unwrap());
    r.reconcile(next, 3).unwrap();
    assert_eq!(position(&r, "lid"), landed);
    steps(&mut r, 360);
    assert!(
        (position(&r, "drop").y - 1.5).abs() < 0.05,
        "{}",
        position(&r, "drop")
    );
}
