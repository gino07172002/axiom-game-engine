//! Trigger zones (`trigger.zone` + `trigger.detect`): enter/exit order, filters, once,
//! hysteresis, parent transforms, budgets, hot reload, checkpoints and the axiom-town port.
use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use serde_json::{json, Value};

/// Zones and movers on an empty plane. Events `in` (payload trigger + entity) and `out`
/// (payload entity) count into variables; `in` is also bound to a native call.
fn world(entities: Value) -> World {
    serde_json::from_value(json!({
        "schema_version":2,"id":"trigger-test","title":"Trigger test",
        "capabilities":{"core":1,"trigger":1,"behavior":1},
        "systems":["trigger.detect","behavior.execute"],
        "entities":entities,
        "resources":{
            "behavior.program":{
                "variables":{"ins":0,"outs":0,"last":""},
                "events":{"in":{"fields":{"trigger":"text","entity":"text"}},"out":{"fields":{"entity":"text"}},"bare":{"fields":{}}},
                "rules":[
                    {"id":"count-in","event":"in","actions":[
                        {"kind":"add","variable":"ins","value":{"kind":"literal","value":1}},
                        {"kind":"set","variable":"last","value":{"kind":"event","field":"entity"}}]},
                    {"id":"count-out","event":"out","actions":[
                        {"kind":"add","variable":"outs","value":{"kind":"literal","value":1}}]}
                ]
            },
            "runtime.bindings":{"events":[{"event":"in","calls":[{"kind":"call","capability":"trigger","action":"summary","args":{}}]}]}
        }
    }))
    .unwrap()
}

fn zone(id: &str, at: [f32; 3], zone: Value) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at},"trigger.zone":zone}})
}

fn mover(id: &str, at: [f32; 3], tags: &[&str]) -> Value {
    json!({"id":id,"tags":tags,"components":{"core.transform":{"position":at}}})
}

fn sphere(r: f32) -> Value {
    json!({"shape":"sphere","radius":r,"filter":{"tags":["player"]},"on_enter":"in","on_exit":"out"})
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, installed_registry()).unwrap()
}

fn step(r: &mut WorldRuntime) {
    r.step(&ActionMap::default());
}

fn summary(r: &WorldRuntime) -> Value {
    r.observation().metrics["trigger"].clone()
}

fn var(r: &WorldRuntime, name: &str) -> Value {
    r.observation().metrics["gameplay"]["variables"][name].clone()
}

/// `recent` entries without ticks: [kind, zone, entity].
fn changes(r: &WorldRuntime) -> Vec<[String; 3]> {
    summary(r)["recent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| [1, 2, 3].map(|i| c[i].as_str().unwrap().to_owned()))
        .collect()
}

fn compile_error(w: &World) -> String {
    format!(
        "{:#}",
        w.compile(&installed_registry().authoring).unwrap_err()
    )
}

#[test]
fn enter_and_exit_fire_in_order_and_reach_rules_and_bindings() {
    let mut r = runtime(world(json!([
        zone("gate/a", [0.0, 0.0, 0.0], sphere(1.0)),
        zone("gate/b", [0.5, 0.0, 0.0], sphere(1.0)),
        mover("hero", [5.0, 0.0, 0.0], &["player"]),
        mover("ally", [5.0, 0.0, 0.0], &["player"]),
    ])));
    step(&mut r);
    assert!(changes(&r).is_empty());
    assert_eq!(summary(&r)["occupied"], 0);
    // Both movers enter both zones in one step: zones in authored order, occupants in
    // authored order.
    r.state.set_position("hero", [0.2, 0.0, 0.0]);
    r.state.set_position("ally", [0.3, 0.0, 0.0]);
    step(&mut r);
    let s = summary(&r);
    assert_eq!(s["inside"]["gate/a"], json!(["hero", "ally"]), "{s}");
    assert_eq!(s["inside"]["gate/b"], json!(["hero", "ally"]), "{s}");
    assert_eq!(var(&r, "ins"), 4);
    assert_eq!(var(&r, "last"), "ally");
    // Only hero leaves gate/a while ally enters nothing new; then ally leaves both and
    // hero is still in gate/b: per zone, exits come before enters.
    r.state.set_position("hero", [1.2, 0.0, 0.0]);
    step(&mut r);
    r.state.set_position("ally", [9.0, 0.0, 0.0]);
    r.state.set_position("hero", [0.0, 0.0, 0.0]);
    step(&mut r);
    let s = |k: &str, z: &str, e: &str| [k.to_owned(), z.to_owned(), e.to_owned()];
    assert_eq!(
        changes(&r),
        [
            s("enter", "gate/a", "hero"),
            s("enter", "gate/a", "ally"),
            s("enter", "gate/b", "hero"),
            s("enter", "gate/b", "ally"),
            s("exit", "gate/a", "hero"),
            s("exit", "gate/a", "ally"),
            s("enter", "gate/a", "hero"),
            s("exit", "gate/b", "ally"),
        ]
    );
    assert_eq!(var(&r, "ins"), 5);
    assert_eq!(var(&r, "outs"), 3);
    let o = r.observation().metrics;
    assert_eq!(o["bindings"]["calls"], 5, "{}", o["bindings"]);
    assert!(o["trigger"]["fault"].is_null());
    // The ticks of the recorded changes are the fixed steps they happened in.
    let ticks: Vec<u64> = o["trigger"]["recent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c[0].as_u64().unwrap())
        .collect();
    assert_eq!(ticks, [2, 2, 2, 2, 3, 4, 4, 4]);
    let inspect = r
        .command(&RuntimeCommand::Call {
            capability: "trigger".into(),
            action: "inspect".into(),
            args: json!({"id":"gate/b"}),
        })
        .unwrap();
    assert_eq!(inspect["result"]["inside"], json!(["hero"]));
    assert_eq!(inspect["result"]["enters"], 2);
}

#[test]
fn filters_use_tags_and_components_and_never_the_zone_itself() {
    let mut z = sphere(2.0);
    z["filter"] = json!({"tags":["player"],"components":["core.health"]});
    let mut tagged_zone = zone("zone", [0.0; 3], z);
    tagged_zone["tags"] = json!(["player"]);
    tagged_zone["components"]["core.health"] = json!({"value":1,"max":1});
    let mut healthy = mover("healthy", [0.0; 3], &["player"]);
    healthy["components"]["core.health"] = json!({"value":5,"max":5});
    let mut untagged = mover("untagged", [0.0; 3], &["npc"]);
    untagged["components"]["core.health"] = json!({"value":5,"max":5});
    let mut r = runtime(world(json!([
        tagged_zone,
        healthy,
        untagged,
        mover("no-health", [0.0; 3], &["player"]),
    ])));
    step(&mut r);
    assert_eq!(summary(&r)["inside"]["zone"], json!(["healthy"]));
    // Empty filter: any entity with a transform (except the zone).
    let mut any = sphere(2.0);
    any["filter"] = json!({});
    let mut r = runtime(world(json!([
        zone("zone", [0.0; 3], any),
        mover("a", [0.0; 3], &[]),
        mover("b", [0.0; 3], &["npc"]),
    ])));
    step(&mut r);
    assert_eq!(summary(&r)["inside"]["zone"], json!(["a", "b"]));
}

#[test]
fn once_emits_one_enter_and_one_exit_but_keeps_tracking() {
    let mut once = sphere(1.0);
    once["once"] = json!(true);
    let mut r = runtime(world(json!([
        zone("once", [0.0; 3], once),
        zone("again", [0.0; 3], sphere(1.0)),
        mover("hero", [5.0, 0.0, 0.0], &["player"]),
    ])));
    for _ in 0..3 {
        r.state.set_position("hero", [0.0; 3]);
        step(&mut r);
        assert_eq!(summary(&r)["inside"]["once"], json!(["hero"]));
        r.state.set_position("hero", [5.0, 0.0, 0.0]);
        step(&mut r);
        assert!(summary(&r)["inside"].get("once").is_none());
    }
    // `again` emitted 3 + 3, `once` 1 + 1.
    assert_eq!(var(&r, "ins"), 4);
    assert_eq!(var(&r, "outs"), 4);
    assert_eq!(summary(&r)["emitted"], 8);
}

#[test]
fn exit_margin_adds_hysteresis_and_disabled_zones_freeze() {
    let mut z = sphere(1.0);
    z["exit_margin"] = json!(0.5);
    let mut off = sphere(1.0);
    off["enabled"] = json!(false);
    let mut r = runtime(world(json!([
        zone("zone", [0.0; 3], z),
        zone("off", [0.0; 3], off),
        mover("hero", [1.2, 0.0, 0.0], &["player"]),
    ])));
    step(&mut r);
    assert_eq!(summary(&r)["occupied"], 0, "1.2 > radius: not entered");
    r.state.set_position("hero", [0.9, 0.0, 0.0]);
    step(&mut r);
    r.state.set_position("hero", [1.4, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(
        summary(&r)["inside"]["zone"],
        json!(["hero"]),
        "inside the margin"
    );
    r.state.set_position("hero", [1.6, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(summary(&r)["occupied"], 0);
    assert_eq!((var(&r, "ins"), var(&r, "outs")), (json!(1), json!(1)));
    // The disabled zone never saw the hero.
    assert!(changes(&r).iter().all(|c| c[1] == "zone"));
}

#[test]
fn zones_and_occupants_follow_parent_transforms() {
    // The zone is a child of a rotated, moved parent: box 2 x 0.5 x 0.5 along the parent's
    // rotated X axis (= world -Z after a +90 degree yaw). The occupant is a child too.
    let yaw = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array();
    let mut w = world(json!([
        {"id":"rig","components":{"core.transform":{"position":[10,0,0],"rotation":yaw}}},
        zone("rig/zone", [0.0; 3], json!({"shape":"box","half_extents":[2,0.5,0.5],"filter":{"tags":["player"]},"on_enter":"in","on_exit":"out"})),
        {"id":"cart","components":{"core.transform":{"position":[0,0,0]}}},
        mover("cart/rider", [10.0, 0.0, -1.5], &["player"]),
    ]));
    w.relations.push(
        serde_json::from_value(json!({"kind":"parent","source":"rig/zone","target":"rig"}))
            .unwrap(),
    );
    w.relations.push(
        serde_json::from_value(json!({"kind":"parent","source":"cart/rider","target":"cart"}))
            .unwrap(),
    );
    let mut r = runtime(w);
    step(&mut r);
    // World pivot (10,0,-1.5) lies along the zone's long axis (world Z), 1.5 m from center.
    assert_eq!(summary(&r)["inside"]["rig/zone"], json!(["cart/rider"]));
    // Moving the parent of the rider moves it out (world X offset 1 > half 0.5).
    r.state.set_position("cart", [1.0, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(summary(&r)["occupied"], 0);
    // Rotating the zone's parent back to identity: the long axis becomes world X again.
    r.state.set_position("cart", [0.0, 0.0, 0.0]);
    let slot = r.state.compiled.indices["rig"];
    r.state.transforms[slot].as_mut().unwrap().rotation = [0.0, 0.0, 0.0, 1.0];
    step(&mut r);
    assert_eq!(
        summary(&r)["occupied"],
        0,
        "z offset 1.5 > half 0.5 once unrotated"
    );
}

#[test]
fn authoring_errors_are_recoverable_diagnostics() {
    let base = || world(json!([zone("z", [0.0; 3], sphere(1.0))]));
    let mut w = base();
    w.systems.retain(|s| s != "trigger.detect");
    assert!(compile_error(&w).contains("needs the trigger.detect system"));
    let mut w = base();
    w.entities[0].components.get_mut("trigger.zone").unwrap()["on_enter"] = json!("nope");
    assert!(compile_error(&w).contains("does not declare"));
    let mut w = base();
    w.entities[0].components.get_mut("trigger.zone").unwrap()["on_enter"] = json!("count");
    w.resources.get_mut("behavior.program").unwrap()["events"]["count"] =
        json!({"fields":{"n":"int"}});
    assert!(compile_error(&w).contains("may only declare text fields"));
    let mut w = base();
    w.entities[0].components.remove("core.transform");
    assert!(compile_error(&w).contains("requires core.transform"));
    let mut w = base();
    w.entities[0].components.get_mut("trigger.zone").unwrap()["radius"] = json!(-1);
    assert!(compile_error(&w).contains("radius"));
    // Payload with no fields is fine (`bare`).
    let mut w = base();
    w.entities[0].components.get_mut("trigger.zone").unwrap()["on_enter"] = json!("bare");
    w.compile(&installed_registry().authoring).unwrap();
}

#[test]
fn zone_pair_and_event_budgets_fail_explicitly() {
    // 257 zones: rejected at compile.
    let zones: Vec<Value> = (0..257)
        .map(|i| zone(&format!("z/{i}"), [i as f32 * 10.0, 0.0, 0.0], sphere(1.0)))
        .collect();
    assert!(compile_error(&world(Value::Array(zones))).contains("trigger zone budget exceeded"));
    // 256 zones x 1025 candidates > 262144 tests per step: the runtime refuses to build.
    let mut entities: Vec<Value> = (0..256)
        .map(|i| zone(&format!("z/{i}"), [i as f32 * 10.0, 0.0, 0.0], sphere(1.0)))
        .collect();
    entities
        .extend((0..1025).map(|i| mover(&format!("m/{i}"), [0.0, 50.0, i as f32], &["player"])));
    let error = WorldRuntime::new(world(Value::Array(entities)), 1, installed_registry())
        .err()
        .map(|e| format!("{e:#}"))
        .unwrap();
    assert!(
        error.contains("trigger candidate budget exceeded"),
        "{error}"
    );
    // 65 enters in one step: detection halts with the reason and emits nothing.
    let mut entities = vec![zone("z", [0.0; 3], sphere(1.0))];
    entities.extend((0..65).map(|i| mover(&format!("m/{i}"), [5.0, 0.0, 0.0], &["player"])));
    let mut r = runtime(world(Value::Array(entities)));
    step(&mut r);
    for i in 0..65 {
        r.state.set_position(&format!("m/{i}"), [0.0; 3]);
    }
    step(&mut r);
    let s = summary(&r);
    assert!(
        s["halted"]
            .as_str()
            .unwrap()
            .contains("trigger event budget exceeded (65 > 64"),
        "{s}"
    );
    assert_eq!(var(&r, "ins"), 0);
    assert_eq!(s["emitted"], 0);
    // 64 is allowed.
    let mut entities = vec![zone("z", [0.0; 3], sphere(1.0))];
    entities.extend((0..64).map(|i| mover(&format!("m/{i}"), [0.0; 3], &["player"])));
    let mut r = runtime(world(Value::Array(entities)));
    step(&mut r);
    assert_eq!(var(&r, "ins"), 64);
    assert!(summary(&r)["halted"].is_null());
}

#[test]
fn spawned_zones_halt_and_checkpoints_refuse_explicitly() {
    let mut r = runtime(world(json!([
        zone("z", [0.0; 3], sphere(1.0)),
        mover("hero", [0.0; 3], &["player"]),
    ])));
    step(&mut r);
    // A runtime spawn of a matching mover becomes a candidate; a spawned zone halts.
    let template = r.state.compiled.entities[r.state.compiled.indices["hero"]].clone();
    r.state.spawn(&template, "hero/2".into(), [0.0; 3]).unwrap();
    step(&mut r);
    assert_eq!(summary(&r)["inside"]["z"], json!(["hero", "hero/2"]));
    let template = r.state.compiled.entities[r.state.compiled.indices["z"]].clone();
    r.state.spawn(&template, "z/2".into(), [0.0; 3]).unwrap();
    step(&mut r);
    assert!(summary(&r)["halted"]
        .as_str()
        .unwrap()
        .contains("spawned trigger zones are not supported"));
    // Occupancy is simulation state the checkpoint format does not carry: refuse.
    let mut w = world(json!([zone("z", [0.0; 3], sphere(1.0))]));
    w.capabilities.insert("save".into(), 1);
    let r = runtime(w);
    let error = format!("{:#}", r.checkpoint().unwrap_err());
    assert!(
        error.contains("trigger.detect does not implement checkpoints"),
        "{error}"
    );
}

#[test]
fn hot_reload_keeps_occupancy_without_repeating_enter() {
    let w = world(json!([
        zone("z", [0.0; 3], sphere(1.0)),
        mover("hero", [0.0; 3], &["player"]),
    ]));
    let mut r = runtime(w.clone());
    step(&mut r);
    assert_eq!(var(&r, "ins"), 1);
    // Unrelated edit: the occupant stays inside and no second enter fires.
    let mut edited = w.clone();
    edited
        .entities
        .push(serde_json::from_value(mover("other", [9.0, 0.0, 0.0], &[])).unwrap());
    r.reconcile(edited.clone(), 2).unwrap();
    step(&mut r);
    assert_eq!(summary(&r)["inside"]["z"], json!(["hero"]));
    assert_eq!(var(&r, "ins"), 1);
    // Shrinking the zone below the occupant's distance makes it leave.
    r.state.set_position("hero", [0.8, 0.0, 0.0]);
    step(&mut r);
    let mut smaller = edited;
    smaller.entities[0]
        .components
        .get_mut("trigger.zone")
        .unwrap()["radius"] = json!(0.5);
    r.reconcile(smaller.clone(), 3).unwrap();
    step(&mut r);
    assert_eq!(summary(&r)["occupied"], 0);
    assert_eq!(var(&r, "outs"), 1);
    // A filter edit that no longer matches the occupant makes it leave, named correctly.
    r.state.set_position("hero", [0.0; 3]);
    step(&mut r);
    assert_eq!(var(&r, "ins"), 2);
    let mut retagged = smaller;
    retagged.entities[0]
        .components
        .get_mut("trigger.zone")
        .unwrap()["filter"] = json!({"tags":["npc"]});
    r.reconcile(retagged, 4).unwrap();
    step(&mut r);
    assert_eq!(var(&r, "outs"), 2);
    let recent = changes(&r);
    assert_eq!(
        recent.last().unwrap(),
        &["exit".to_owned(), "z".to_owned(), "hero".to_owned()]
    );
}

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// The pre-trigger town: bench / hair-clip spots were FSM agents with near 1.6 / far 2.2.
/// Their authored positions drifted on the first step (blocker push-out) to the positions
/// the trigger zones now author.
fn legacy_town() -> World {
    let mut w = town();
    w.systems.retain(|s| s != "trigger.detect");
    w.capabilities.remove("trigger");
    let machine = |near: &str, left: &str| {
        json!({"initial":"away","states":{
            "away":{"action":{"kind":"stop"},"emit":left},
            "near":{"action":{"kind":"stop"},"emit":near}},
            "transitions":[
                {"from":"away","to":"near","when":{"kind":"near","tag":"player","within":1.6}},
                {"from":"near","to":"away","when":{"kind":"far","tag":"player","beyond":2.2}}]})
    };
    let machines = &mut w.resources.get_mut("fsm.machines").unwrap()["machines"];
    machines["bench_spot"] = machine("near_bench", "left_bench");
    machines["clip_spot"] = machine("near_clip", "left_clip");
    for (id, m, p) in [
        ("spot/bench-east", "bench_spot", [6.0, 0.0, 4.0]),
        ("spot/hairclip", "clip_spot", [-2.3, 0.0, -1.9]),
    ] {
        let e = w.entity_mut(id).unwrap();
        e.components.remove("trigger.zone");
        e.components
            .insert("fsm.agent".into(), json!({"machine":m}));
        e.components
            .insert("core.transform".into(), json!({"position":p}));
    }
    w
}

/// Drives the hero near the bench, away, near the hair clip and away again; returns every
/// change of `at_bench` / `at_clip` as (tick, variable, value).
fn walk(world: World) -> Vec<(u64, &'static str, Value)> {
    let mut r = runtime(world);
    // (ticks, move_x, move_z); move_z = +1 walks toward -Z.
    let plan: [(u32, f32, f32); 7] = [
        (240, 1.0, 0.0),
        (150, 0.0, 1.0),
        (200, 0.0, -1.0),
        (375, -1.0, 0.0),
        (425, 0.0, 1.0),
        (240, 0.0, -1.0),
        (60, 0.0, 0.0),
    ];
    let mut out = vec![];
    let mut last = (var(&r, "at_bench"), var(&r, "at_clip"));
    for (ticks, x, z) in plan {
        let mut input = ActionMap::default();
        input.0.insert("move_x".into(), x);
        input.0.insert("move_z".into(), z);
        for _ in 0..ticks {
            r.step(&input);
            let now = (var(&r, "at_bench"), var(&r, "at_clip"));
            if now.0 != last.0 {
                out.push((r.state.tick, "at_bench", now.0.clone()));
            }
            if now.1 != last.1 {
                out.push((r.state.tick, "at_clip", now.1.clone()));
            }
            last = now;
        }
    }
    out
}

#[test]
fn axiom_town_trigger_zones_match_the_fsm_near_workaround() {
    let zones = walk(town());
    let fsm = walk(legacy_town());
    let values = |v: &[(u64, &str, Value)]| {
        v.iter()
            .map(|(_, k, v)| (k.to_string(), v.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        values(&zones),
        [
            ("at_bench".into(), json!(true)),
            ("at_bench".into(), json!(false)),
            ("at_clip".into(), json!(true)),
            ("at_clip".into(), json!(false)),
        ],
        "{zones:?}"
    );
    // Same variable changes on the same fixed ticks.
    assert_eq!(zones, fsm);
}

#[test]
fn grid_query_matches_a_brute_force_reference() {
    // Spheres and rotated boxes from 0.3 m to 30 m (the large ones exceed the grid query
    // range and use the full candidate list), movers crossing cell boundaries and the origin.
    let mut entities = vec![];
    let mut zones = vec![];
    for i in 0..40 {
        let p = [
            (i % 8) as f32 * 7.3 - 25.0,
            (i % 3) as f32 - 1.0,
            (i / 8) as f32 * 6.1 - 12.0,
        ];
        let size = [0.3, 1.0, 2.5, 4.1, 30.0][i % 5];
        let yaw = glam::Quat::from_rotation_y(i as f32 * 0.37).to_array();
        let zone_def = if i % 2 == 0 {
            json!({"shape":"sphere","radius":size,"filter":{"tags":["player"]}})
        } else {
            json!({"shape":"box","half_extents":[size,1.5,size*0.5],"filter":{"tags":["player"]}})
        };
        let id = format!("zone/{i}");
        entities.push(json!({"id":id,"components":{"core.transform":{"position":p,"rotation":yaw},"trigger.zone":zone_def}}));
        zones.push((
            id,
            glam::Vec3::from_array(p),
            glam::Quat::from_array(yaw),
            i % 2 == 0,
            size,
        ));
    }
    for i in 0..300 {
        let p = [
            (i % 20) as f32 * 3.1 - 31.0,
            ((i % 5) as f32 - 2.0) * 0.6,
            (i / 20) as f32 * 2.9 - 20.0,
        ];
        let v = [((i * 7) % 11) as f32 - 5.0, 0.0, ((i * 3) % 7) as f32 - 3.0];
        let tags = if i % 10 == 0 {
            json!(["npc"])
        } else {
            json!(["player"])
        };
        entities.push(json!({"id":format!("m/{i}"),"tags":tags,"components":{"core.transform":{"position":p},"core.motion":{"velocity":v}}}));
    }
    let mut w = world(Value::Array(entities));
    w.capabilities.insert("motion".into(), 1);
    w.systems.push("motion.integrate".into());
    let mut r = runtime(w);
    for _ in 0..240 {
        step(&mut r);
        if !r.state.tick.is_multiple_of(8) {
            continue;
        }
        let system = r.system::<axiom_runtime::trigger::TriggerSystem>().unwrap();
        for (id, center, rotation, sphere, size) in &zones {
            let mut expected = vec![];
            for (slot, e) in r.state.compiled.entities.iter().enumerate() {
                if !e.tags.iter().any(|t| t == "player") {
                    continue;
                }
                let d = glam::Vec3::from_array(r.state.transforms[slot].as_ref().unwrap().position)
                    - *center;
                let inside = if *sphere {
                    d.length_squared() <= size * size
                } else {
                    let l = rotation.inverse() * d;
                    l.x.abs() <= *size && l.y.abs() <= 1.5 && l.z.abs() <= *size * 0.5
                };
                if inside {
                    expected.push(e.id.as_str());
                }
            }
            assert_eq!(
                system.occupants(id).unwrap(),
                expected,
                "{id} at tick {}",
                r.state.tick
            );
        }
    }
    let s = summary(&r);
    assert!(
        s["halted"].is_null() && s["occupied"].as_u64().unwrap() > 5,
        "{s}"
    );
}

#[test]
fn play_overlay_wireframes_mark_occupied_zones() {
    let mut off = sphere(1.0);
    off["enabled"] = json!(false);
    let mut r = runtime(world(json!([
        zone("ball", [0.0; 3], sphere(1.0)),
        zone(
            "crate",
            [10.0, 0.0, 0.0],
            json!({"shape":"box","half_extents":[1,1,1],"filter":{"tags":["player"]}})
        ),
        zone("off", [20.0, 0.0, 0.0], off),
        mover("hero", [10.0, 0.0, 0.0], &["player"]),
    ])));
    step(&mut r);
    let system = r.system::<axiom_runtime::trigger::TriggerSystem>().unwrap();
    let lines = system.wireframes(&r.state);
    // Sphere: 3 circles x 24 segments; box: 12 edges; the disabled zone is not drawn.
    assert_eq!(lines.len(), 72 + 12);
    assert!(lines[..72].iter().all(|(a, b, occupied)| !occupied
        && (a.length() - 1.0).abs() < 1e-4
        && (b.length() - 1.0).abs() < 1e-4));
    assert!(lines[72..].iter().all(|(_, _, occupied)| *occupied));
}
