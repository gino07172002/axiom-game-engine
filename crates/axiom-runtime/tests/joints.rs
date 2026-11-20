//! Physics joints (`physics.joint`: fixed / revolute / prismatic / spherical, limits, motors) and
//! contact events (`physics.collider` `sensor`, `on_enter` / `on_exit` into `behavior.program`):
//! behaviour, ordering, checkpoints, hot reload, replay and authoring errors.
use axiom_core::{gameplay::RuntimeCommand, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use glam::Vec3;
use serde_json::{json, Value};

/// Physics + behavior world: events `enter` {self, other} and `exit` {other} count into
/// variables (`last` = the latest entering collider).
fn world(entities: Value) -> World {
    serde_json::from_value(json!({
        "schema_version":2,"id":"joint-test","title":"Joint test",
        "capabilities":{"core":1,"physics":1,"motion":1,"behavior":1},
        "systems":["motion.integrate","physics.step","behavior.execute"],
        "entities":entities,
        "resources":{"behavior.program":{
            "variables":{"enters":0,"exits":0,"last":"","zone":""},
            "events":{"enter":{"fields":{"self":"text","other":"text"}},"exit":{"fields":{"other":"text"}}},
            "rules":[
                {"id":"in","event":"enter","actions":[
                    {"kind":"add","variable":"enters","value":{"kind":"literal","value":1}},
                    {"kind":"set","variable":"last","value":{"kind":"event","field":"other"}},
                    {"kind":"set","variable":"zone","value":{"kind":"event","field":"self"}}]},
                {"id":"out","event":"exit","actions":[
                    {"kind":"add","variable":"exits","value":{"kind":"literal","value":1}}]}
            ]}}
    }))
    .unwrap()
}

fn floor() -> Value {
    json!({"id":"floor","components":{"core.transform":{"position":[0,-0.5,0]},
        "physics.collider":{"shape":{"shape":"box","half_extents":[20,0.5,20]}}}})
}

fn body(id: &str, at: [f32; 3], half: [f32; 3], joint: Value) -> Value {
    let mut v = json!({"id":id,"components":{"core.transform":{"position":at},
        "physics.collider":{"shape":{"shape":"box","half_extents":half}},
        "physics.body":{}}});
    if !joint.is_null() {
        v["components"]["physics.joint"] = joint;
    }
    v
}

fn ball(id: &str, at: [f32; 3]) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at},
        "physics.collider":{"shape":{"shape":"sphere","radius":0.25}},"physics.body":{}}})
}

fn sensor(id: &str, at: [f32; 3], half: [f32; 3]) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at},
        "physics.collider":{"shape":{"shape":"box","half_extents":half},"sensor":true,
            "on_enter":"enter","on_exit":"exit"}}})
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

fn rotation(r: &WorldRuntime, id: &str) -> glam::Quat {
    glam::Quat::from_array(
        r.state.transforms[r.state.compiled.indices[id]]
            .as_ref()
            .unwrap()
            .rotation,
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

fn var(r: &WorldRuntime, name: &str) -> Value {
    r.observation().metrics["gameplay"]["variables"][name].clone()
}

fn hash(r: &WorldRuntime) -> String {
    axiom_runtime::scenario::state_hash(r).unwrap()
}

/// A 1.2 x 2 x 0.1 door hinged to the world on its left edge (x = 1.4) and a `core.motion`
/// pusher walking through its free half along -z.
fn door_world(joint: Value) -> World {
    world(json!([
        floor(),
        body("door", [2.0, 1.05, 0.0], [0.6, 1.0, 0.05], joint),
        {"id":"pusher","components":{"core.transform":{"position":[2.3,0.9,3.0]},
            "core.motion":{"velocity":[0,0,-1.2]},
            "physics.collider":{"shape":{"shape":"capsule","half_height":0.5,"radius":0.35}}}}
    ]))
}

#[test]
fn a_revolute_door_swings_about_its_hinge_within_its_limits() {
    let mut r = runtime(door_world(
        json!({"kind":"revolute","anchor":[-0.6,0.0,0.0],
        "axis":[0,1,0],"limits":[-80,80]}),
    ));
    assert_eq!(call(&mut r, "summary", json!({})).unwrap()["joints"], 1);
    let inspect = call(&mut r, "inspect", json!({"id":"door"})).unwrap();
    assert_eq!(inspect["joint"]["kind"], "revolute");
    assert_eq!(inspect["joint"]["angle"], 0.0);
    let hinge =
        |r: &WorldRuntime| position(r, "door") + rotation(r, "door") * Vec3::new(-0.6, 0.0, 0.0);
    let start = hinge(&r);
    let mut widest: f64 = 0.0;
    for _ in 0..40 {
        steps(&mut r, 12);
        let angle = call(&mut r, "inspect", json!({"id":"door"})).unwrap()["joint"]["angle"]
            .as_f64()
            .unwrap();
        widest = widest.max(angle.abs());
        assert!(angle.abs() <= 80.0 + 2.0, "limit exceeded: {angle}");
        // The hinge point stays where it was authored; the door does not fall.
        assert!(
            (hinge(&r) - start).length() < 0.03,
            "{} vs {start}",
            hinge(&r)
        );
    }
    assert!(widest > 45.0, "the pusher swung the door open: {widest}");
    // Without the joint the same push carries the door away (the joint is what holds it).
    let mut free = runtime(door_world(Value::Null));
    steps(&mut free, 480);
    assert!((position(&free, "door") - Vec3::new(2.0, 1.05, 0.0)).length() > 0.5);
}

#[test]
fn a_spherical_chain_stays_connected_and_swings() {
    let link = |i: usize, joint: Value| {
        body(
            &format!("l/{i}"),
            [0.0, 5.0 - 0.55 * i as f32, 0.0],
            [0.08, 0.25, 0.08],
            joint,
        )
    };
    let mut entities = vec![
        floor(),
        link(0, json!({"kind":"spherical","anchor":[0,0.25,0]})),
    ];
    for i in 1..5 {
        entities.push(link(
            i,
            json!({"kind":"spherical","to":format!("l/{}",i-1),"anchor":[0,0.25,0]}),
        ));
    }
    let mut w = world(json!(entities));
    w.entity_mut("l/4")
        .unwrap()
        .components
        .get_mut("physics.body")
        .unwrap()["velocity"] = json!([3, 0, 0]);
    let mut r = runtime(w);
    let top = |r: &WorldRuntime, i: usize| {
        position(r, &format!("l/{i}")) + rotation(r, &format!("l/{i}")) * Vec3::new(0.0, 0.25, 0.0)
    };
    let bottom = |r: &WorldRuntime, i: usize| {
        position(r, &format!("l/{i}")) + rotation(r, &format!("l/{i}")) * Vec3::new(0.0, -0.3, 0.0)
    };
    let mut reach: f32 = 0.0;
    for _ in 0..30 {
        steps(&mut r, 10);
        assert!((top(&r, 0) - Vec3::new(0.0, 5.25, 0.0)).length() < 0.03);
        for i in 1..5 {
            // Each link's top stays on the joint point below its parent (authored gap 0.05).
            let gap = (top(&r, i) - bottom(&r, i - 1)).length();
            assert!(gap < 0.04, "link {i} separated by {gap}");
        }
        reach = reach.max(position(&r, "l/4").x.abs());
    }
    assert!(reach > 0.5, "the chain swung: {reach}");
}

#[test]
fn fixed_and_prismatic_joints_lock_and_limit_their_axes() {
    let mut r = runtime(world(json!([
        body(
            "weld",
            [0.0, 3.0, 0.0],
            [0.5, 0.5, 0.5],
            json!({"kind":"fixed"})
        ),
        body(
            "slide",
            [5.0, 3.0, 0.0],
            [0.5, 0.5, 0.5],
            json!({"kind":"prismatic","axis":[1,0,0],"limits":[-1,1]})
        ),
    ])));
    call(&mut r, "impulse", json!({"id":"slide","impulse":[8,0,0]})).unwrap();
    steps(&mut r, 240);
    // The weld holds its pose against gravity; the slider moves only along x, up to +1 m.
    assert!((position(&r, "weld") - Vec3::new(0.0, 3.0, 0.0)).length() < 0.02);
    let s = position(&r, "slide");
    assert!((s.y - 3.0).abs() < 0.02 && s.z.abs() < 0.02, "{s}");
    assert!(s.x > 5.5 && s.x < 6.05, "{s}");
    let offset = call(&mut r, "inspect", json!({"id":"slide"})).unwrap()["joint"]["offset"]
        .as_f64()
        .unwrap();
    assert!(offset > 0.5 && offset < 1.05, "{offset}");
}

fn motor_world() -> World {
    let mut w = world(json!([body(
        "wheel",
        [0.0, 3.0, 0.0],
        [1.0, 0.1, 1.0],
        json!({"kind":"revolute","axis":[0,1,0],"motor":{"target_velocity":90,"damping":20,"max_force":1000}})
    )]));
    w.capabilities.insert("save".into(), 1);
    w
}

#[test]
fn motors_drive_joints_change_at_runtime_and_round_trip_through_checkpoints() {
    let mut r = runtime(motor_world());
    steps(&mut r, 120);
    let spin = |r: &mut WorldRuntime| {
        call(r, "inspect", json!({"id":"wheel"})).unwrap()["angular_velocity"][1]
            .as_f64()
            .unwrap()
    };
    // 90°/s about +Y.
    assert!(
        (spin(&mut r) - 90f64.to_radians()).abs() < 0.05,
        "{}",
        spin(&mut r)
    );
    let set = call(
        &mut r,
        "set_motor",
        json!({"id":"wheel","target_velocity":-45}),
    )
    .unwrap();
    assert_eq!(set["joint"]["motor"]["target_velocity"], -45.0);
    steps(&mut r, 120);
    assert!((spin(&mut r) + 45f64.to_radians()).abs() < 0.05);
    // Out of range, a position without stiffness, a joint without motor: refused.
    assert!(call(
        &mut r,
        "set_motor",
        json!({"id":"wheel","target_velocity":1e6})
    )
    .is_err());
    assert!(call(
        &mut r,
        "set_motor",
        json!({"id":"wheel","target_position":10})
    )
    .is_err());
    assert!(call(&mut r, "set_motor", json!({"id":"wheel"})).is_err());
    // A checkpoint keeps the runtime motor target; two loads stay identical.
    let saved = r.checkpoint().unwrap();
    let mut a = runtime(motor_world());
    let mut b = runtime(motor_world());
    a.restore_checkpoint(&saved).unwrap();
    b.restore_checkpoint(&saved).unwrap();
    assert_eq!(
        call(&mut a, "inspect", json!({"id":"wheel"})).unwrap()["joint"]["motor"]
            ["target_velocity"],
        -45.0
    );
    steps(&mut a, 120);
    steps(&mut b, 120);
    assert_eq!(hash(&a), hash(&b));
    assert!((spin(&mut a) + 45f64.to_radians()).abs() < 0.05);
    // A checkpoint without the motor state is refused, not silently reset to the authored 90°/s.
    let mut stripped = saved.clone();
    stripped.systems.get_mut("physics.step").unwrap()["motors"] = json!({});
    let error = format!("{:#}", b.restore_checkpoint(&stripped).unwrap_err());
    assert!(error.contains("joint motors"), "{error}");
    // Hot reload of the unchanged world keeps the runtime target.
    a.reconcile(motor_world(), 2).unwrap();
    assert_eq!(
        call(&mut a, "inspect", json!({"id":"wheel"})).unwrap()["joint"]["motor"]
            ["target_velocity"],
        -45.0
    );
}

#[test]
fn sensors_report_enter_and_exit_without_stopping_anything() {
    let mut r = runtime(world(json!([
        floor(),
        sensor("zone", [0.0, 3.0, 0.0], [1.0, 1.0, 1.0]),
        ball("ball", [0.0, 6.0, 0.0]),
    ])));
    let mut entered_at = None;
    for t in 0..240 {
        steps(&mut r, 1);
        if entered_at.is_none() && var(&r, "enters") == 1 {
            entered_at = Some(t);
            let inspect = call(&mut r, "inspect", json!({"id":"zone"})).unwrap();
            assert_eq!(inspect["touching"], json!(["ball"]));
        }
    }
    assert!(entered_at.is_some());
    assert_eq!(var(&r, "enters"), 1);
    assert_eq!(var(&r, "exits"), 1);
    assert_eq!(var(&r, "last"), "ball");
    assert_eq!(var(&r, "zone"), "zone");
    // The ball fell straight through the sensor onto the floor.
    let p = position(&r, "ball");
    assert!((p.y - 0.25).abs() < 0.03 && p.x.abs() < 0.01, "{p}");
    let summary = call(&mut r, "summary", json!({})).unwrap();
    assert_eq!(summary["contact_events"]["emitted"], 2, "{summary}");
    assert_eq!(summary["contact_events"]["colliders"], 1);
}

#[test]
fn static_sensors_see_moved_colliders_and_events_follow_entity_id_order() {
    // A kinematic mover (core.motion follower) walks into a static sensor: rapier's default
    // filter would never pair these.
    let mut w = world(json!([
        sensor("zone", [0.0, 1.0, 0.0], [1.0, 1.0, 1.0]),
        {"id":"walker","components":{"core.transform":{"position":[-4.0,1.0,0.0]},
            "core.motion":{"velocity":[2,0,0]},
            "physics.collider":{"shape":{"shape":"capsule","half_height":0.5,"radius":0.3}}}},
        // Authored b before a: both land in the zone on the same step.
        ball("b", [6.0, 4.0, 0.0]),
        ball("a", [6.0, 4.0, 3.0]),
        sensor("pad", [6.0, 2.0, 1.5], [1.0, 0.5, 3.0]),
    ]));
    w.entities.retain(|e| e.id != "floor");
    let mut r = runtime(w);
    let mut order = vec![];
    for _ in 0..400 {
        steps(&mut r, 1);
        let last = var(&r, "last");
        if order.last() != Some(&last) && last != "" {
            order.push(last);
        }
    }
    // The walker entered and left the zone; a and b entered the pad on one step, a first.
    assert!(order.contains(&json!("walker")), "{order:?}");
    let ab: Vec<&Value> = order.iter().filter(|v| *v != "walker").collect();
    assert_eq!(
        ab,
        [&json!("b")],
        "a then b on the same step leaves b last: {order:?}"
    );
    assert_eq!(var(&r, "enters"), 3);
    assert_eq!(var(&r, "exits"), 3);
}

fn resting() -> World {
    let mut w = world(json!([
        floor(),
        sensor("zone", [0.0, 0.5, 0.0], [1.0, 1.0, 1.0]),
        ball("ball", [0.0, 1.0, 0.0]),
    ]));
    w.capabilities.insert("save".into(), 1);
    w
}

#[test]
fn contacts_survive_checkpoints_and_reloads_without_refiring() {
    let mut r = runtime(resting());
    steps(&mut r, 60);
    assert_eq!(var(&r, "enters"), 1);
    let saved = r.checkpoint().unwrap();
    let mut a = runtime(resting());
    a.restore_checkpoint(&saved).unwrap();
    steps(&mut a, 60);
    assert_eq!(
        var(&a, "enters"),
        1,
        "a contact that existed at save time does not re-enter"
    );
    assert_eq!(var(&a, "exits"), 0);
    assert_eq!(
        call(&mut a, "summary", json!({})).unwrap()["contact_events"]["emitted"],
        1
    );
    // Missing contact state is refused.
    let mut stripped = saved.clone();
    stripped.systems.get_mut("physics.step").unwrap()["touching"] = json!({});
    let error = format!("{:#}", a.restore_checkpoint(&stripped).unwrap_err());
    assert!(error.contains("contact-event colliders"), "{error}");
    // Hot reload keeps the contact.
    r.reconcile(resting(), 2).unwrap();
    steps(&mut r, 30);
    assert_eq!(var(&r, "enters"), 1);
    assert_eq!(var(&r, "exits"), 0);
}

fn lab() -> World {
    axiom_core::ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/joint-lab"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

#[test]
fn the_joint_lab_scenario_passes_and_replays_identically() {
    let run = || {
        let scenario =
            serde_json::from_str(include_str!("../../../examples/joint-lab.scenario.json"))
                .unwrap();
        axiom_runtime::scenario::run(lab(), 1, installed_registry(), scenario).unwrap()
    };
    let (a, b) = (run(), run());
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["state_hash"], b["state_hash"]);
}

#[test]
fn joint_and_event_authoring_errors_are_explicit() {
    let compile = |w: World| {
        w.compile(&installed_registry().authoring)
            .map(drop)
            .map_err(|e| format!("{e:#}"))
    };
    let revolute = json!({"kind":"revolute"});
    let one = |joint: Value| {
        world(json!([
            body("a", [0.0, 1.0, 0.0], [0.5; 3], joint),
            body("b", [2.0, 1.0, 0.0], [0.5; 3], Value::Null)
        ]))
    };
    assert!(compile(one(revolute.clone())).is_ok());
    assert!(compile(one(json!({"kind":"revolute","to":"b","limits":[-30,30]}))).is_ok());
    let mut no_body = one(json!({"kind":"revolute","to":"b"}));
    no_body
        .entity_mut("a")
        .unwrap()
        .components
        .remove("physics.body");
    assert!(compile(no_body).unwrap_err().contains("no physics.body"));
    assert!(compile(one(json!({"kind":"revolute","to":"a"})))
        .unwrap_err()
        .contains("itself"));
    assert!(compile(one(json!({"kind":"revolute","to":"ghost"})))
        .unwrap_err()
        .contains("does not exist"));
    let mut target = one(json!({"kind":"fixed","to":"floor"}));
    target
        .entities
        .push(serde_json::from_value(floor()).unwrap());
    assert!(compile(target).unwrap_err().contains("has no physics.body"));
    let mut kinematic = one(revolute.clone());
    kinematic
        .entity_mut("a")
        .unwrap()
        .components
        .get_mut("physics.body")
        .unwrap()["kind"] = json!("kinematic");
    assert!(compile(kinematic).unwrap_err().contains("no dynamic body"));
    for (joint, needle) in [
        (
            json!({"kind":"spherical","axis":[1,0,0]}),
            "revolute and prismatic",
        ),
        (json!({"kind":"revolute","axis":[0,0,0]}), "non-zero"),
        (json!({"kind":"revolute","limits":[30,-30]}), "min < max"),
        (json!({"kind":"revolute","limits":[-200,0]}), "-180"),
        (
            json!({"kind":"revolute","motor":{"target_position":5}}),
            "stiffness",
        ),
        (
            json!({"kind":"revolute","motor":{"damping":0}}),
            "stiffness or damping",
        ),
        (json!({"kind":"hinge"}), "unknown variant"),
        (json!({"kind":"fixed","bounce":1}), "unknown field"),
    ] {
        let e = compile(one(joint.clone())).unwrap_err();
        assert!(e.contains(needle), "{joint}: {e}");
    }
    let mut many = vec![];
    for i in 0..=axiom_core::physics::MAX_JOINTS {
        many.push(body(
            &format!("j/{i}"),
            [i as f32 * 2.0, 1.0, 0.0],
            [0.5; 3],
            revolute.clone(),
        ));
    }
    assert!(compile(world(json!(many)))
        .unwrap_err()
        .contains("joint budget"));
    // Events: declared, payload fields, systems, no sensor on a dynamic body.
    let zone = || world(json!([sensor("z", [0.0, 1.0, 0.0], [1.0; 3])]));
    assert!(compile(zone()).is_ok());
    let mut undeclared = zone();
    undeclared
        .entity_mut("z")
        .unwrap()
        .components
        .get_mut("physics.collider")
        .unwrap()["on_enter"] = json!("ghost");
    assert!(compile(undeclared)
        .unwrap_err()
        .contains("does not declare"));
    let mut fields = zone();
    fields.resources.get_mut("behavior.program").unwrap()["events"]["enter"] =
        json!({"fields":{"who":"text"}});
    assert!(compile(fields).unwrap_err().contains("`self` and `other`"));
    let mut unscheduled = zone();
    unscheduled.systems.retain(|s| s != "behavior.execute");
    assert!(compile(unscheduled)
        .unwrap_err()
        .contains("behavior.execute"));
    let mut falling = zone();
    falling
        .entity_mut("z")
        .unwrap()
        .components
        .insert("physics.body".into(), json!({}));
    assert!(compile(falling).unwrap_err().contains("sensor"));
    // Collider JSON without the new fields serializes as before.
    let c: axiom_core::Collider =
        serde_json::from_value(json!({"shape":{"shape":"sphere","radius":1}})).unwrap();
    assert_eq!(
        serde_json::to_value(&c).unwrap(),
        json!({"shape":{"shape":"sphere","radius":1.0}})
    );
}

#[test]
fn sensors_are_not_camera_blockers_or_nav_obstacles() {
    let mut w = world(json!([sensor("z", [0.0, 1.0, 0.0], [1.0; 3])]));
    let compiled = w.clone().compile(&installed_registry().authoring).unwrap();
    let registry = installed_registry();
    let blockers = axiom_core::camera::collect_blockers(
        &compiled.entities,
        &compiled.parents,
        &registry.authoring.movers,
    )
    .unwrap();
    assert!(blockers.is_empty());
    w.entity_mut("z")
        .unwrap()
        .components
        .get_mut("physics.collider")
        .unwrap()["sensor"] = json!(false);
    let compiled = w.compile(&installed_registry().authoring).unwrap();
    let blockers = axiom_core::camera::collect_blockers(
        &compiled.entities,
        &compiled.parents,
        &registry.authoring.movers,
    )
    .unwrap();
    assert_eq!(blockers.len(), 1);
}
