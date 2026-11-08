//! Paths and path following (`path.spline` + `path.follow` + `path.move`): end modes,
//! closed paths, parents, orientation, lateral offsets, runtime commands and bindings,
//! budgets, authoring errors, hot reload and checkpoints.
use axiom_core::{gameplay::RuntimeCommand, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use glam::{Quat, Vec3};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Paths and followers on an empty plane. `arrived` (payload follower + path) counts into
/// variables; `stop_car` is bound to `path.pause` on `car`.
fn world(entities: Value) -> World {
    serde_json::from_value(json!({
        "schema_version":2,"id":"path-test","title":"Path test",
        "capabilities":{"core":1,"path":1,"behavior":1,"motion":1},
        "systems":["path.move","behavior.execute"],
        "entities":entities,
        "resources":{
            "behavior.program":{
                "variables":{"arrivals":0,"last":"","last_path":""},
                "events":{"arrived":{"fields":{"follower":"text","path":"text"}},"stop_car":{"fields":{}},"bare":{"fields":{}}},
                "rules":[
                    {"id":"count","event":"arrived","actions":[
                        {"kind":"add","variable":"arrivals","value":{"kind":"literal","value":1}},
                        {"kind":"set","variable":"last","value":{"kind":"event","field":"follower"}},
                        {"kind":"set","variable":"last_path","value":{"kind":"event","field":"path"}}]}
                ]
            },
            "runtime.bindings":{"events":[{"event":"stop_car","calls":[{"kind":"call","capability":"path","action":"pause","args":{"id":"car"}}]}]}
        }
    }))
    .unwrap()
}

fn path(id: &str, at: [f32; 3], points: Value, closed: bool) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at},"path.spline":{"points":points,"closed":closed}}})
}

fn line(id: &str, length: f32) -> Value {
    path(id, [0.0; 3], json!([[0, 0, 0], [length, 0, 0]]), false)
}

fn follower(id: &str, follow: Value) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":[0,0,0]},"path.follow":follow}})
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, installed_registry()).unwrap()
}

fn steps(r: &mut WorldRuntime, n: usize) {
    for _ in 0..n {
        r.step(&ActionMap::default());
    }
}

fn position(r: &WorldRuntime, id: &str) -> Vec3 {
    Vec3::from_array(
        r.state.transforms[r.state.compiled.indices[id]]
            .as_ref()
            .unwrap()
            .position,
    )
}

/// Local +Z of the entity in its parent's space (the facing direction).
fn facing(r: &WorldRuntime, id: &str) -> Vec3 {
    let t = r.state.transforms[r.state.compiled.indices[id]]
        .as_ref()
        .unwrap();
    Quat::from_array(t.rotation) * Vec3::Z
}

fn summary(r: &WorldRuntime) -> Value {
    r.observation().metrics["path"].clone()
}

fn var(r: &WorldRuntime, name: &str) -> Value {
    r.observation().metrics["gameplay"]["variables"][name].clone()
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "path".into(),
        action: action.into(),
        args,
    })
    .map(|v| v["result"].clone())
}

fn compile_error(w: &World) -> String {
    format!(
        "{:#}",
        w.compile(&installed_registry().authoring).unwrap_err()
    )
}

fn near(a: Vec3, b: Vec3) -> bool {
    a.distance(b) < 2e-3
}

#[test]
fn loop_ping_pong_and_stop_follow_the_expected_distance() {
    let mut r = runtime(world(json!([
        line("road", 10.0),
        follower("looper", json!({"path":"road","speed":4.0})),
        follower(
            "bouncer",
            json!({"path":"road","speed":4.0,"end":"ping_pong"})
        ),
        follower(
            "stopper",
            json!({"path":"road","speed":4.0,"end":"stop","on_arrive":"arrived"})
        ),
    ])));
    // 1 s: 4 m for all three.
    steps(&mut r, 120);
    for id in ["looper", "bouncer", "stopper"] {
        assert!(
            near(position(&r, id), Vec3::new(4.0, 0.0, 0.0)),
            "{id} {}",
            position(&r, id)
        );
    }
    // 3 s = 12 m: loop wraps to 2, ping-pong folds to 8 coming back, stop holds at 10.
    steps(&mut r, 240);
    assert!(near(position(&r, "looper"), Vec3::new(2.0, 0.0, 0.0)));
    assert!(near(position(&r, "bouncer"), Vec3::new(8.0, 0.0, 0.0)));
    assert!(near(position(&r, "stopper"), Vec3::new(10.0, 0.0, 0.0)));
    // Facing: +X forward, the bouncer looks back along -X.
    assert!(near(facing(&r, "looper"), Vec3::X));
    assert!(near(facing(&r, "bouncer"), -Vec3::X));
    assert!(near(facing(&r, "stopper"), Vec3::X));
    // on_arrive fired exactly once with the payload, at 2.5 s.
    assert_eq!(var(&r, "arrivals"), 1);
    assert_eq!(var(&r, "last"), "stopper");
    assert_eq!(var(&r, "last_path"), "road");
    let s = summary(&r);
    assert_eq!(s["arrived"], json!(["stopper"]), "{s}");
    assert_eq!(s["at"]["stopper"], json!([10.0, 1.0]));
    assert_eq!(s["at"]["bouncer"], json!([8.0, 0.8]));
    assert_eq!(s["moving"], 2);
    assert_eq!(s["emitted"], 1);
    steps(&mut r, 240);
    assert_eq!(var(&r, "arrivals"), 1);
    // restart puts it back at the start and it arrives (and emits) again.
    let restarted = call(&mut r, "restart", json!({"id":"stopper"})).unwrap();
    assert_eq!(restarted["distance"], 0.0);
    assert_eq!(restarted["arrived"], false);
    steps(&mut r, 301);
    assert_eq!(var(&r, "arrivals"), 2);
    assert!(summary(&r)["fault"].is_null());
}

#[test]
fn closed_paths_wrap_continuously_and_corners_blend_the_offset() {
    // 8 x 6 rectangle (perimeter 28), followed at 4 m/s with and without a lateral offset.
    let rect = json!([[-4, 0, -3], [4, 0, -3], [4, 0, 3], [-4, 0, 3]]);
    let mut r = runtime(world(json!([
        path("loop", [0.0; 3], rect, true),
        follower("car", json!({"path":"loop","speed":4.0})),
        follower("walker", json!({"path":"loop","speed":4.0,"offset":0.5})),
    ])));
    let dt_travel = 4.0 / 120.0;
    let mut previous = (position(&r, "car"), position(&r, "walker"));
    let mut largest = (0.0f32, 0.0f32);
    for tick in 0..900 {
        steps(&mut r, 1);
        let now = (position(&r, "car"), position(&r, "walker"));
        if tick > 0 {
            largest.0 = largest.0.max(now.0.distance(previous.0));
            largest.1 = largest.1.max(now.1.distance(previous.1));
        }
        previous = now;
        if tick == 179 {
            // 1.5 s = 6 m along the first edge; the walker is 0.5 m to the right (inside).
            assert!(near(now.0, Vec3::new(2.0, 0.0, -3.0)), "{}", now.0);
            assert!(near(now.1, Vec3::new(2.0, 0.0, -2.5)), "{}", now.1);
        }
    }
    // The closing piece joins the last point to the first: no jump anywhere, and the offset
    // follower moves continuously round the corners (it cuts them, so it is never faster).
    assert!(largest.0 <= dt_travel * 1.001, "{largest:?}");
    assert!(largest.1 <= dt_travel * 1.001, "{largest:?}");
    // 7 s = 28 m = one lap, plus 0.5 s.
    assert!(near(position(&r, "car"), Vec3::new(-2.0, 0.0, -3.0)));
    let progress = summary(&r)["at"]["car"][1].as_f64().unwrap();
    assert!((progress - 2.0 / 28.0).abs() < 1e-3, "{progress}");
}

#[test]
fn parented_and_rotated_paths_and_parented_followers() {
    // The path hangs under a rig at x=10 turned +90 degrees about Y (local +X -> world -Z).
    // The follower itself is a child of a holder at x=1, so its local position is relative.
    let yaw = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array();
    let mut w = world(json!([
        {"id":"rig","components":{"core.transform":{"position":[10,0,0],"rotation":yaw}}},
        line("road", 8.0),
        {"id":"holder","components":{"core.transform":{"position":[1,0,0]}}},
        follower("car", json!({"path":"road","speed":2.0})),
    ]));
    for (child, parent) in [("road", "rig"), ("car", "holder")] {
        w.relations.push(
            serde_json::from_value(json!({"kind":"parent","source":child,"target":parent}))
                .unwrap(),
        );
    }
    let mut r = runtime(w);
    steps(&mut r, 120);
    // 2 m along the path: world (10, 0, -2), local to the holder (9, 0, -2).
    assert!(
        near(position(&r, "car"), Vec3::new(9.0, 0.0, -2.0)),
        "{}",
        position(&r, "car")
    );
    assert!(near(facing(&r, "car"), -Vec3::Z));
    // Moving the rig at runtime moves the path (and the follower) on the next step.
    r.state.set_position("rig", [20.0, 0.0, 0.0]);
    steps(&mut r, 120);
    assert!(
        near(position(&r, "car"), Vec3::new(19.0, 0.0, -4.0)),
        "{}",
        position(&r, "car")
    );
}

#[test]
fn orientation_turn_rate_yaw_offset_and_lateral_offsets() {
    // An L: 4 m along +X, then 4 m along +Z.
    let l = json!([[0, 0, 0], [4, 0, 0], [4, 0, 4]]);
    let mut r = runtime(world(json!([
        path("l", [0.0; 3], l, false),
        follower(
            "snap",
            json!({"path":"l","speed":1.0,"start":2.0,"end":"stop"})
        ),
        follower(
            "smooth",
            json!({"path":"l","speed":1.0,"start":2.0,"end":"stop","turn_rate":30.0})
        ),
        follower("model", json!({"path":"l","speed":1.0,"yaw_offset":90.0})),
        follower(
            "left",
            json!({"path":"l","speed":1.0,"offset":-1.0,"orient":false})
        ),
        follower(
            "right",
            json!({"path":"l","speed":1.0,"offset":1.0,"orient":false})
        ),
    ])));
    steps(&mut r, 1);
    assert!(near(facing(&r, "snap"), Vec3::X));
    // yaw_offset 90: +Z of the model turns from +X a further 90 degrees about Y (to -Z).
    assert!(
        near(facing(&r, "model"), -Vec3::Z),
        "{}",
        facing(&r, "model")
    );
    // Offsets: right of +X travel is +Z (models look along +Z, right = forward x up).
    assert!((position(&r, "right").z - 1.0).abs() < 1e-3);
    assert!((position(&r, "left").z + 1.0).abs() < 1e-3);
    // orient false leaves the rotation alone.
    assert!(near(facing(&r, "left"), Vec3::Z));
    // The facing turns over the 1 m corner blend on each side (d = 3..5). Past it the
    // snapping one faces +Z; the 30 deg/s one has turned only about 60 degrees.
    steps(&mut r, 360);
    assert!(near(facing(&r, "snap"), Vec3::Z));
    let smooth = facing(&r, "smooth");
    assert!(smooth.x > 0.3 && smooth.z > 0.3, "{smooth}");
    // Stopped at the end, it keeps turning toward the tangent.
    steps(&mut r, 360);
    assert!(
        near(facing(&r, "smooth"), Vec3::Z),
        "{}",
        facing(&r, "smooth")
    );
}

#[test]
fn catmull_rom_paths_move_at_constant_speed_through_the_points() {
    let mut w = world(json!([
        path(
            "curve",
            [0.0; 3],
            json!([[0, 0, 0], [5, 0, 3], [10, 0, 0], [15, 0, 3]]),
            false
        ),
        follower("car", json!({"path":"curve","speed":3.0,"end":"stop"})),
    ]));
    w.entities[0].components.get_mut("path.spline").unwrap()["interpolation"] =
        json!("catmull_rom");
    let mut r = runtime(w);
    let inspect = call(&mut r, "inspect", json!({"id":"curve"})).unwrap();
    assert_eq!(inspect["nodes"], 3 * 8 + 1);
    let length = inspect["length"].as_f64().unwrap() as f32;
    assert!(length > 3.0 * 34f32.sqrt(), "{inspect}");
    let mut previous = position(&r, "car");
    let mut steps_taken = 0;
    while summary(&r)["arrived"].as_array().unwrap().is_empty() {
        steps(&mut r, 1);
        let now = position(&r, "car");
        if steps_taken > 0 {
            let d = now.distance(previous);
            assert!(d <= 3.0 / 120.0 * 1.001, "step {steps_taken}: {d}");
        }
        previous = now;
        steps_taken += 1;
        assert!(steps_taken < 10_000);
    }
    // Arrival after length / speed seconds, at the last control point.
    assert!((steps_taken as f32 - length / 3.0 * 120.0).abs() <= 1.0);
    assert!(near(position(&r, "car"), Vec3::new(15.0, 0.0, 3.0)));
}

#[test]
fn pause_resume_set_speed_and_bindings() {
    let mut r = runtime(world(json!([
        line("road", 100.0),
        follower("car", json!({"path":"road","speed":2.0})),
        follower("bus", json!({"path":"road","speed":1.0,"paused":true})),
    ])));
    steps(&mut r, 120);
    assert!(near(position(&r, "car"), Vec3::new(2.0, 0.0, 0.0)));
    // An authored-paused follower is placed at its start but does not move.
    assert!(near(position(&r, "bus"), Vec3::ZERO));
    assert_eq!(summary(&r)["paused"], json!(["bus"]));
    let paused = call(&mut r, "pause", json!({"id":"car"})).unwrap();
    assert_eq!(paused["paused"], true);
    steps(&mut r, 120);
    assert!(near(position(&r, "car"), Vec3::new(2.0, 0.0, 0.0)));
    call(&mut r, "resume", json!({"id":"car"})).unwrap();
    call(&mut r, "resume", json!({"id":"bus"})).unwrap();
    let fast = call(&mut r, "set_speed", json!({"id":"car","speed":6.0})).unwrap();
    assert_eq!(fast["speed"], 6.0);
    steps(&mut r, 120);
    assert!(near(position(&r, "car"), Vec3::new(8.0, 0.0, 0.0)));
    assert!(near(position(&r, "bus"), Vec3::new(1.0, 0.0, 0.0)));
    // A behavior event bound through runtime.bindings pauses the car (a crossing).
    r.command(&RuntimeCommand::Emit {
        event: "stop_car".into(),
        fields: BTreeMap::new(),
    })
    .unwrap();
    steps(&mut r, 60);
    assert!(near(position(&r, "car"), Vec3::new(8.0, 0.0, 0.0)));
    assert_eq!(summary(&r)["paused"], json!(["car"]));
    let inspect = call(&mut r, "inspect", json!({"id":"car"})).unwrap();
    assert_eq!(inspect["paused"], true);
    assert_eq!(inspect["progress"], 0.08);
    // Recoverable request errors.
    let missing = call(&mut r, "pause", json!({"id":"nobody"})).unwrap_err();
    assert!(format!("{missing:#}").contains("not a path follower"));
    let fast = call(&mut r, "set_speed", json!({"id":"car","speed":500})).unwrap_err();
    assert!(format!("{fast:#}").contains("speed"), "{fast:#}");
    assert!(call(&mut r, "pause", json!({})).is_err());
}

#[test]
fn budgets_and_authoring_errors_are_explicit() {
    let code = |w: &World| compile_error(w);
    // Path shape errors are component validation errors.
    let e = code(&world(json!([path(
        "p",
        [0.0; 3],
        json!([[0, 0, 0]]),
        false
    )])));
    assert!(e.contains("2..256 points"), "{e}");
    let e = code(&world(json!([path(
        "p",
        [0.0; 3],
        json!([[0, 0, 0], [0, 0, 0]]),
        false
    )])));
    assert!(e.contains("closer than"), "{e}");
    // References.
    let e = code(&world(json!([follower("car", json!({"path":"nowhere"}))])));
    assert!(
        e.contains("invalid_path") && e.contains("nowhere is not an entity with path.spline"),
        "{e}"
    );
    let e = code(&world(json!([
        {"id":"plain","components":{"core.transform":{}}},
        follower("car", json!({"path":"plain"})),
    ])));
    assert!(e.contains("plain is not an entity with path.spline"), "{e}");
    // A follower that is (an ancestor of) its own path.
    let mut own = follower("car", json!({"path":"car"}));
    own["components"]["path.spline"] = json!({"points":[[0,0,0],[1,0,0]]});
    let e = code(&world(json!([own])));
    assert!(e.contains("would move its own path"), "{e}");
    let mut w = world(json!([
        line("road", 5.0),
        follower("car", json!({"path":"road"}))
    ]));
    w.relations.push(
        serde_json::from_value(json!({"kind":"parent","source":"road","target":"car"})).unwrap(),
    );
    assert!(code(&w).contains("would move its own path"));
    // Missing systems / undeclared events.
    let mut w = world(json!([
        line("road", 5.0),
        follower("car", json!({"path":"road"}))
    ]));
    w.systems.retain(|s| s != "path.move");
    assert!(code(&w).contains("needs the path.move system"));
    let w = world(json!([
        line("road", 5.0),
        follower(
            "car",
            json!({"path":"road","end":"stop","on_arrive":"nope"})
        )
    ]));
    assert!(code(&w).contains("does not declare"));
    let mut w = world(json!([
        line("road", 5.0),
        follower("car", json!({"path":"road","end":"stop","on_arrive":"bad"}))
    ]));
    w.resources.get_mut("behavior.program").unwrap()["events"]["bad"] =
        json!({"fields":{"speed":"number"}});
    assert!(
        code(&w).contains("may only declare text fields"),
        "{}",
        code(&w)
    );
    let w = world(json!([
        line("road", 5.0),
        follower("car", json!({"path":"road","on_arrive":"arrived"}))
    ]));
    assert!(code(&w).contains("on_arrive only fires with end stop"));
    // Mover conflict: one entity, one mover.
    let mut car = follower("car", json!({"path":"road"}));
    car["components"]["core.motion"] = json!({"velocity":[1,0,0]});
    let e = code(&world(json!([line("road", 5.0), car])));
    assert!(
        e.contains("path_conflict") && e.contains("core.motion (moved by motion.integrate)"),
        "{e}"
    );
    // Budgets: paths, nodes, followers.
    let paths: Vec<Value> = (0..257).map(|i| line(&format!("p/{i}"), 1.0)).collect();
    let e = code(&world(Value::Array(paths)));
    assert!(
        e.contains("path_budget") && e.contains("path budget exceeded (257 > 256)"),
        "{e}"
    );
    let curve = |i: usize| {
        let points: Vec<[f32; 3]> = (0..256).map(|k| [k as f32, 0.0, (k % 2) as f32]).collect();
        json!({"id":format!("c/{i}"),"components":{"core.transform":{},"path.spline":{"points":points,"interpolation":"catmull_rom"}}})
    };
    // 32 x (255 x 8 + 1) = 65,312 nodes fit; one more curve does not.
    assert!(world(Value::Array((0..32).map(curve).collect()))
        .compile(&installed_registry().authoring)
        .is_ok());
    let e = code(&world(Value::Array((0..33).map(curve).collect())));
    assert!(e.contains("path node budget exceeded"), "{e}");
    let mut many = vec![line("road", 5.0)];
    many.extend((0..2049).map(|i| follower(&format!("f/{i}"), json!({"path":"road"}))));
    let e = code(&world(Value::Array(many)));
    assert!(e.contains("follower budget exceeded (2049 > 2048)"), "{e}");
}

#[test]
fn arrival_event_budget_halts_explicitly() {
    let followers = |n: usize| {
        let mut e = vec![line("road", 1.0)];
        e.extend((0..n).map(|i| {
            follower(
                &format!("f/{i}"),
                json!({"path":"road","speed":10.0,"end":"stop","on_arrive":"arrived"}),
            )
        }));
        Value::Array(e)
    };
    let mut r = runtime(world(followers(64)));
    steps(&mut r, 13);
    assert_eq!(var(&r, "arrivals"), 64);
    assert!(summary(&r)["halted"].is_null());
    let mut r = runtime(world(followers(65)));
    steps(&mut r, 13);
    let s = summary(&r);
    assert!(
        s["halted"]
            .as_str()
            .unwrap()
            .contains("path event budget exceeded (65 > 64"),
        "{s}"
    );
    assert_eq!(var(&r, "arrivals"), 0);
}

#[test]
fn hot_reload_keeps_progress_unless_the_path_changes() {
    let w = world(json!([
        line("road", 100.0),
        follower("car", json!({"path":"road","speed":2.0})),
        follower("bus", json!({"path":"road","speed":1.0})),
    ]));
    let mut r = runtime(w.clone());
    steps(&mut r, 120);
    call(&mut r, "set_speed", json!({"id":"bus","speed":3.0})).unwrap();
    // An unrelated edit (new entity): progress and the speed override are kept.
    let mut edited = w.clone();
    edited.entities.push(
        serde_json::from_value(json!({"id":"lamp","components":{"core.transform":{}}})).unwrap(),
    );
    r.reconcile(edited.clone(), 2).unwrap();
    steps(&mut r, 120);
    assert!(
        near(position(&r, "car"), Vec3::new(4.0, 0.0, 0.0)),
        "{}",
        position(&r, "car")
    );
    assert!(
        near(position(&r, "bus"), Vec3::new(4.0, 0.0, 0.0)),
        "{}",
        position(&r, "bus")
    );
    // Editing the follower's speed keeps its distance (only the path, start, end mode or
    // paused flag reset it).
    let mut faster = edited.clone();
    faster
        .entity_mut("car")
        .unwrap()
        .components
        .get_mut("path.follow")
        .unwrap()["speed"] = json!(4.0);
    r.reconcile(faster.clone(), 3).unwrap();
    steps(&mut r, 120);
    assert!(
        near(position(&r, "car"), Vec3::new(8.0, 0.0, 0.0)),
        "{}",
        position(&r, "car")
    );
    // Changing the path restarts its followers.
    let mut moved = faster.clone();
    moved
        .entity_mut("road")
        .unwrap()
        .components
        .get_mut("path.spline")
        .unwrap()["points"] = json!([[0, 0, 0], [0, 0, 100]]);
    r.reconcile(moved, 4).unwrap();
    steps(&mut r, 120);
    assert!(
        near(position(&r, "car"), Vec3::new(0.0, 0.0, 4.0)),
        "{}",
        position(&r, "car")
    );
    assert!(
        near(position(&r, "bus"), Vec3::new(0.0, 0.0, 1.0)),
        "{}",
        position(&r, "bus")
    );
}

#[test]
fn spawned_followers_halt_and_checkpoints_refuse_explicitly() {
    let mut r = runtime(world(json!([
        line("road", 10.0),
        follower("car", json!({"path":"road"})),
    ])));
    steps(&mut r, 1);
    let template = r.state.compiled.entities[r.state.compiled.indices["car"]].clone();
    r.state.spawn(&template, "car/2".into(), [0.0; 3]).unwrap();
    steps(&mut r, 1);
    assert!(summary(&r)["halted"]
        .as_str()
        .unwrap()
        .contains("spawned paths and followers are not supported"));
    let mut w = world(json!([
        line("road", 10.0),
        follower("car", json!({"path":"road"}))
    ]));
    w.capabilities.insert("save".into(), 1);
    let error = format!("{:#}", runtime(w).checkpoint().unwrap_err());
    assert!(
        error.contains("path.move does not implement checkpoints"),
        "{error}"
    );
}

#[test]
fn disabled_and_inactive_scene_followers_do_not_move() {
    let mut w = world(json!([
        line("road", 10.0),
        follower("off", json!({"path":"road","enabled":false})),
        follower("hidden", json!({"path":"road"})),
    ]));
    w.entities[1].components.get_mut("core.transform").unwrap()["position"] = json!([0, 5, 0]);
    w.scenes.push(
        serde_json::from_value(json!({"id":"later","entities":["hidden"],"active":false})).unwrap(),
    );
    let mut r = runtime(w);
    steps(&mut r, 120);
    assert!(near(position(&r, "off"), Vec3::new(0.0, 5.0, 0.0)));
    assert!(near(position(&r, "hidden"), Vec3::ZERO));
    let error = call(&mut r, "pause", json!({"id":"off"})).unwrap_err();
    assert!(format!("{error:#}").contains("disabled follower"));
}
