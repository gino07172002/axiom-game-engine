//! Navigation agents (`nav.agent` + `nav.steer`): straight lines, obstacles, doorways,
//! unreachable targets, moving entity targets, separation, the per-step search budget,
//! authoring errors, hot reload, checkpoints, runtime commands, bindings and the pointer.
use axiom_core::{gameplay::RuntimeCommand, World};
use axiom_runtime::{installed_registry, world::PointerKind, ActionMap, WorldRuntime};
use glam::{Vec2, Vec3};
use serde_json::{json, Value};

/// Agents and walls on an empty plane. `arrived` / `failed` count into variables and keep the
/// payload; `send_home` is bound to `nav.go` (bob -> home).
fn world(entities: Value) -> World {
    world_with(entities, json!({}))
}

fn world_with(entities: Value, grid: Value) -> World {
    serde_json::from_value(json!({
        "schema_version":2,"id":"nav-test","title":"Nav test",
        "capabilities":{"core":1,"nav":1,"behavior":1,"motion":1,"path":1},
        "systems":["nav.steer","behavior.execute","motion.integrate","path.move"],
        "entities":entities,
        "resources":{
            "nav.grid":grid,
            "behavior.program":{
                "variables":{"arrivals":0,"fails":0,"last":"","target":"","reason":""},
                "events":{
                    "arrived":{"fields":{"agent":"text","target":"text","reason":"text"}},
                    "failed":{"fields":{"agent":"text","reason":"text"}},
                    "send_home":{"fields":{}}
                },
                "rules":[
                    {"id":"count","event":"arrived","actions":[
                        {"kind":"add","variable":"arrivals","value":{"kind":"literal","value":1}},
                        {"kind":"set","variable":"last","value":{"kind":"event","field":"agent"}},
                        {"kind":"set","variable":"target","value":{"kind":"event","field":"target"}}]},
                    {"id":"fail","event":"failed","actions":[
                        {"kind":"add","variable":"fails","value":{"kind":"literal","value":1}},
                        {"kind":"set","variable":"reason","value":{"kind":"event","field":"reason"}}]}
                ]
            },
            "runtime.bindings":{"events":[{"event":"send_home","calls":[{"kind":"call","capability":"nav","action":"go","args":{"id":"bob","entity":"home"}}]}]}
        }
    }))
    .unwrap()
}

fn agent(id: &str, at: [f32; 3], nav: Value) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at},"nav.agent":nav}})
}

/// A wall: `nav.blocker` rectangle centered at (x, z) with half extents.
fn wall(id: &str, x: f32, z: f32, hx: f32, hz: f32) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":[x,0,z]},"nav.blocker":{"half":[hx,hz]}}})
}

fn marker(id: &str, at: [f32; 3]) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at}}})
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

fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

fn summary(r: &WorldRuntime) -> Value {
    r.observation().metrics["nav"].clone()
}

fn state_of(r: &WorldRuntime, id: &str) -> String {
    summary(r)["at"][id][0].as_str().unwrap().to_owned()
}

fn var(r: &WorldRuntime, name: &str) -> Value {
    r.observation().metrics["gameplay"]["variables"][name].clone()
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "nav".into(),
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

/// Steps until `id` reaches `state` (or `limit` steps); returns the steps taken.
fn until(r: &mut WorldRuntime, id: &str, state: &str, limit: usize) -> usize {
    for n in 0..limit {
        if state_of(r, id) == state {
            return n;
        }
        r.step(&ActionMap::default());
    }
    panic!("{id} never became {state}: {}", summary(r));
}

/// Distance from `p` to the nearest point of an axis-aligned XZ rectangle.
fn rect_distance(p: Vec2, min: Vec2, max: Vec2) -> f32 {
    p.distance(p.clamp(min, max))
}

#[test]
fn walks_a_straight_line_brakes_and_reports_arrival() {
    let mut r = runtime(world(json!([agent(
        "bob",
        [0.0, 0.0, 0.0],
        json!({"speed":2.0,"target":{"point":[6,0,0]},"on_arrive":"arrived"})
    )])));
    let s = summary(&r);
    assert_eq!(s["agents"], 1);
    assert_eq!(s["grid"]["cell"], 0.25);
    let taken = until(&mut r, "bob", "arrived", 1200);
    // 6 m at 2 m/s with acceleration 8 m/s²: a little over 3 s.
    assert!((360..480).contains(&taken), "{taken}");
    let p = position(&r, "bob");
    assert!(xz(p).distance(Vec2::new(6.0, 0.0)) <= 0.2 + 1e-3, "{p}");
    assert_eq!(var(&r, "arrivals"), 1);
    assert_eq!(var(&r, "last"), "bob");
    assert_eq!(var(&r, "target"), "6,0");
    // It brakes to a stop rather than sliding on.
    steps(&mut r, 120);
    assert!(xz(position(&r, "bob")).distance(Vec2::new(6.0, 0.0)) <= 0.2);
    let inspect = call(&mut r, "inspect", json!({"id":"bob"})).unwrap();
    assert_eq!(inspect["state"], "arrived");
    assert_eq!(inspect["speed"], 0.0);
    // Facing: models look along +Z; walking +X turns the yaw to +90 degrees.
    let t = r.state.transforms[r.state.compiled.indices["bob"]]
        .as_ref()
        .unwrap();
    let forward = glam::Quat::from_array(t.rotation) * Vec3::Z;
    assert!(forward.distance(Vec3::X) < 1e-3, "{forward}");
    // A straight line needs no search (line of sight).
    assert_eq!(summary(&r)["budget"]["searches"], 1);
    assert!(summary(&r)["fault"].is_null());
}

#[test]
fn walks_around_an_obstacle_without_touching_it() {
    // A 1 x 6 m wall between start and goal.
    let mut r = runtime(world(json!([
        wall("wall", 3.0, 0.0, 0.5, 3.0),
        agent(
            "bob",
            [0.0, 0.0, 0.0],
            json!({"radius":0.3,"target":{"point":[6,0,0]},"on_arrive":"arrived"})
        ),
    ])));
    let (min, max) = (Vec2::new(2.5, -3.0), Vec2::new(3.5, 3.0));
    let mut closest = f32::MAX;
    for _ in 0..2400 {
        r.step(&ActionMap::default());
        closest = closest.min(rect_distance(xz(position(&r, "bob")), min, max));
        if state_of(&r, "bob") == "arrived" {
            break;
        }
    }
    assert_eq!(state_of(&r, "bob"), "arrived", "{}", summary(&r));
    assert!(closest >= 0.3 - 1e-3, "touched the wall: {closest}");
    assert_eq!(var(&r, "arrivals"), 1);
}

/// A 6 x 6 m room centered at (0, 8) with 0.2 m thick walls and a doorway in the south wall.
fn room(door: f32) -> Vec<Value> {
    let (cx, cz, half, t) = (0.0, 8.0, 3.0, 0.1);
    let side = (2.0 * half - door) / 2.0;
    vec![
        wall("north", cx, cz + half, half + t, t),
        wall("east", cx + half, cz, t, half + t),
        wall("west", cx - half, cz, t, half + t),
        // South wall in two pieces around the doorway at x = cx.
        wall("south/a", cx - half + side / 2.0, cz - half, side / 2.0, t),
        wall("south/b", cx + half - side / 2.0, cz - half, side / 2.0, t),
    ]
}

#[test]
fn enters_a_room_through_a_doorway_wider_than_the_agent() {
    // 0.3 m radius (0.6 m diameter) through a 0.7 m doorway; the grid cell is 0.25 m.
    let mut entities = room(0.7);
    entities.push(agent(
        "bob",
        [4.0, 0.0, 0.0],
        json!({"radius":0.3,"target":{"point":[0.5,0,9]},"on_arrive":"arrived"}),
    ));
    let mut r = runtime(world(Value::Array(entities)));
    let mut inside_wall = false;
    for _ in 0..3600 {
        r.step(&ActionMap::default());
        let p = xz(position(&r, "bob"));
        // Never inside the south wall pieces.
        for (lo, hi) in [
            (Vec2::new(-3.0, 4.9), Vec2::new(-0.35, 5.1)),
            (Vec2::new(0.35, 4.9), Vec2::new(3.0, 5.1)),
        ] {
            inside_wall |= rect_distance(p, lo, hi) < 0.3 - 1e-3;
        }
        if state_of(&r, "bob") == "arrived" {
            break;
        }
    }
    assert_eq!(state_of(&r, "bob"), "arrived", "{}", summary(&r));
    assert!(!inside_wall);
    let p = xz(position(&r, "bob"));
    assert!(p.y > 5.5 && p.x.abs() < 3.0, "inside the room: {p}");
    // A doorway narrower than the diameter (0.5 m) keeps the room closed: unreachable.
    let mut entities = room(0.5);
    entities.push(agent(
        "bob",
        [4.0, 0.0, 0.0],
        json!({"radius":0.3,"target":{"point":[0.5,0,9]},"on_fail":"failed"}),
    ));
    let mut r = runtime(world(Value::Array(entities)));
    until(&mut r, "bob", "failed", 600);
    assert_eq!(summary(&r)["failures"]["bob"], "unreachable");
    assert_eq!(var(&r, "fails"), 1);
    assert_eq!(var(&r, "reason"), "unreachable");
}

#[test]
fn unreachable_and_outside_targets_fail_with_reasons() {
    let mut entities = room(0.0);
    entities.push(agent(
        "bob",
        [4.0, 0.0, 0.0],
        json!({"target":{"point":[0,0,8]},"on_fail":"failed"}),
    ));
    entities.push(agent("ann", [-4.0, 0.0, 0.0], json!({"on_fail":"failed"})));
    let mut r = runtime(world_with(
        Value::Array(entities),
        json!({"bounds":{"min":[-10,-5],"max":[10,15]}}),
    ));
    until(&mut r, "bob", "failed", 600);
    assert_eq!(var(&r, "reason"), "unreachable");
    // Outside the grid bounds.
    call(&mut r, "go", json!({"id":"ann","point":[50,0,0]})).unwrap();
    until(&mut r, "ann", "failed", 10);
    assert_eq!(summary(&r)["failures"]["ann"], "outside_grid");
    assert_eq!(var(&r, "fails"), 2);
    // Inside a wall: the goal snaps to the nearest walkable cell and is reached.
    call(&mut r, "go", json!({"id":"ann","point":[-3,0,1]})).unwrap();
    until(&mut r, "ann", "arrived", 1200);
    // Bad requests are recoverable errors.
    assert!(call(&mut r, "go", json!({"id":"nobody","point":[0,0,0]})).is_err());
    assert!(call(&mut r, "go", json!({"id":"ann","point":[0,0]})).is_err());
    assert!(call(&mut r, "go", json!({"id":"ann","entity":"ghost"})).is_err());
    assert!(call(&mut r, "go", json!({"id":"ann","entity":"ann"})).is_err());
    assert!(call(&mut r, "go", json!({"id":"ann"})).is_err());
}

#[test]
fn follows_a_moving_entity_target_and_rearrives() {
    // The target walks +Z at 0.5 m/s; the agent (1.5 m/s) catches it and keeps following.
    let mut r = runtime(world(json!([
        {"id":"dog","components":{"core.transform":{"position":[6,0,0]},"core.motion":{"velocity":[0,0,0.5]}}},
        agent(
            "bob",
            [0.0, 0.0, 0.0],
            json!({"speed":1.5,"arrive_distance":0.8,"target":{"entity":"dog"},"follow_target":true,"on_arrive":"arrived"})
        ),
    ])));
    let taken = until(&mut r, "bob", "arrived", 2400);
    let gap = xz(position(&r, "bob")).distance(xz(position(&r, "dog")));
    assert!(gap <= 0.8 + 0.05, "{gap}");
    assert!(
        position(&r, "dog").z > 1.0,
        "the target moved meanwhile ({taken} steps)"
    );
    assert_eq!(var(&r, "target"), "dog");
    // It keeps following: later it is still close to the moving target and arrived again.
    steps(&mut r, 600);
    let gap = xz(position(&r, "bob")).distance(xz(position(&r, "dog")));
    assert!(gap < 2.0, "{gap}");
    assert!(var(&r, "arrivals").as_u64().unwrap() >= 2);
    assert!(summary(&r)["budget"]["searches"].as_u64().unwrap() >= 2);
}

#[test]
fn agents_keep_apart_and_pass_each_other() {
    // Head-on along the same line, plus two that converge on one spot.
    let mut r = runtime(world(json!([
        agent(
            "a",
            [-5.0, 0.0, 0.0],
            json!({"radius":0.3,"target":{"point":[5,0,0]}})
        ),
        agent(
            "b",
            [5.0, 0.0, 0.0],
            json!({"radius":0.3,"target":{"point":[-5,0,0]}})
        ),
        agent(
            "c",
            [0.0, 0.0, 6.0],
            json!({"radius":0.3,"target":{"point":[0,0,9]}})
        ),
        agent(
            "d",
            [0.5, 0.0, 12.0],
            json!({"radius":0.3,"target":{"point":[0,0,9]},"arrive_distance":0.7})
        ),
    ])));
    let mut closest = f32::MAX;
    for _ in 0..1800 {
        r.step(&ActionMap::default());
        let (a, b) = (xz(position(&r, "a")), xz(position(&r, "b")));
        closest = closest.min(a.distance(b));
    }
    let s = summary(&r);
    assert_eq!(s["at"]["a"][0], "arrived", "{s}");
    assert_eq!(s["at"]["b"][0], "arrived", "{s}");
    // Separation keeps bodies (nearly) apart: at most a little overlap while pushing.
    assert!(closest > 0.45, "{closest}");
    let (c, d) = (xz(position(&r, "c")), xz(position(&r, "d")));
    assert!(c.distance(d) > 0.55, "{c} {d}");
    assert_eq!(s["at"]["c"][0], "arrived", "{s}");
    assert_eq!(s["at"]["d"][0], "arrived", "{s}");
}

#[test]
fn searches_are_budgeted_per_step_and_queued() {
    // A long wall with a gap at the far end forces long searches; the budget is tiny.
    let mut entities = vec![wall("wall", 0.0, 0.0, 14.0, 0.3)];
    for k in 0..12 {
        entities.push(agent(
            &format!("n{k}"),
            [-12.0 + 2.0 * k as f32, 0.0, -4.0],
            json!({"target":{"point":[-12.0 + 2.0 * k as f32,0,4]},"on_arrive":"arrived","avoidance":false}),
        ));
    }
    let mut r = runtime(world_with(
        Value::Array(entities),
        json!({"expansions_per_step":256}),
    ));
    let mut most = 0;
    let mut deferred = 0;
    for _ in 0..6000 {
        r.step(&ActionMap::default());
        let s = summary(&r);
        let last = s["budget"]["last"].as_u64().unwrap();
        most = most.max(last);
        deferred = s["budget"]["deferred_steps"].as_u64().unwrap();
        if s["arrived"] == 12 {
            break;
        }
    }
    let s = summary(&r);
    assert_eq!(s["arrived"], 12, "{s}");
    assert_eq!(var(&r, "arrivals"), 12);
    // Requests waited for later steps; one step never spent more than the budget plus one
    // path's straightening allowance (1024 line-of-sight cells).
    assert!(deferred > 20, "{deferred}");
    assert!(most <= 256 + 1024, "{most}");
    assert_eq!(s["budget"]["max"].as_u64().unwrap(), most);
}

#[test]
fn authoring_errors_are_explicit() {
    // Two movers on one entity.
    let w = world(json!([
        {"id":"bob","components":{"core.transform":{},"nav.agent":{},"core.motion":{"velocity":[1,0,0]}}}
    ]));
    let e = compile_error(&w);
    assert!(
        e.contains("nav_conflict") && e.contains("core.motion"),
        "{e}"
    );
    let w = world(json!([
        {"id":"road","components":{"core.transform":{},"path.spline":{"points":[[0,0,0],[1,0,0]]}}},
        {"id":"bob","components":{"core.transform":{},"nav.agent":{},"path.follow":{"path":"road"}}}
    ]));
    let e = compile_error(&w);
    assert!(e.contains("conflict") && e.contains("nav.agent"), "{e}");
    // Missing system, unknown target, undeclared events, bad payloads, oversize grids.
    let mut w = world(json!([agent("bob", [0.0; 3], json!({}))]));
    w.systems.retain(|s| s != "nav.steer");
    assert!(compile_error(&w).contains("nav.steer"));
    let w = world(json!([agent(
        "bob",
        [0.0; 3],
        json!({"target":{"entity":"ghost"}})
    )]));
    assert!(compile_error(&w).contains("ghost"));
    let w = world(json!([agent("bob", [0.0; 3], json!({"on_arrive":"nope"}))]));
    assert!(compile_error(&w).contains("does not declare"));
    let w = world(json!([agent(
        "bob",
        [0.0; 3],
        json!({"on_arrive":"send_home"})
    )]));
    let w = {
        let mut w = w;
        w.resources.get_mut("behavior.program").unwrap()["events"]["send_home"]["fields"] =
            json!({"count":"number"});
        w
    };
    assert!(compile_error(&w).contains("may only declare"));
    let w = world(json!([
        agent("bob", [0.0; 3], json!({})),
        agent("far", [5000.0, 0.0, 0.0], json!({}))
    ]));
    let e = compile_error(&w);
    assert!(e.contains("nav_budget") && e.contains("bounds"), "{e}");
    let w = world_with(
        json!([agent("bob", [0.0; 3], json!({}))]),
        json!({"pointer_agent":"ghost"}),
    );
    assert!(compile_error(&w).contains("pointer_agent"));
    let w = world(json!([agent("bob", [0.0; 3], json!({"radius":9}))]));
    assert!(compile_error(&w).contains("radius"));
}

#[test]
fn hot_reload_keeps_progress_and_replans_when_obstacles_move() {
    let w = world(json!([
        wall("wall", 3.0, 0.0, 0.5, 3.0),
        agent(
            "bob",
            [0.0, 0.0, 0.0],
            json!({"target":{"point":[6,0,0]},"on_arrive":"arrived"})
        ),
    ]));
    let mut r = runtime(w.clone());
    steps(&mut r, 240);
    assert_eq!(state_of(&r, "bob"), "moving");
    let before = position(&r, "bob");
    let searches = summary(&r)["budget"]["searches"].clone();
    // Unrelated edit: same grid; the path and position carry over, no new search.
    let mut edited = w.clone();
    edited.entities.push(
        serde_json::from_value(
            json!({"id":"lamp","components":{"core.transform":{"position":[1,0,1]}}}),
        )
        .unwrap(),
    );
    r.reconcile(edited.clone(), 2).unwrap();
    assert_eq!(position(&r, "bob"), before);
    assert_eq!(state_of(&r, "bob"), "moving");
    assert_eq!(summary(&r)["budget"]["searches"], searches);
    assert_eq!(summary(&r)["grid"]["cached"], true);
    // The wall moves: the agent re-plans to the same goal from where it is.
    let mut moved = edited.clone();
    moved.entities[0]
        .components
        .get_mut("core.transform")
        .unwrap()["position"] = json!([3, 0, 2]);
    r.reconcile(moved.clone(), 3).unwrap();
    assert_eq!(position(&r, "bob"), before);
    assert_eq!(state_of(&r, "bob"), "pending");
    until(&mut r, "bob", "arrived", 2400);
    assert_eq!(var(&r, "arrivals"), 1);
    // A changed authored target restarts the agent toward it.
    let mut retarget = moved;
    retarget.entities[1]
        .components
        .get_mut("nav.agent")
        .unwrap()["target"] = json!({"point":[0,0,-5]});
    r.reconcile(retarget, 4).unwrap();
    until(&mut r, "bob", "arrived", 2400);
    assert!(xz(position(&r, "bob")).distance(Vec2::new(0.0, -5.0)) < 0.25);
}

#[test]
fn commands_bindings_pointer_and_checkpoints() {
    // Looking straight down: -90 degrees about X.
    let down = glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2).to_array();
    let mut w = world(json!([
        marker("home", [0.0, 0.0, -6.0]),
        agent("bob", [0.0, 0.0, 0.0], json!({"on_arrive":"arrived"})),
        {"id":"cam","components":{"core.transform":{"position":[0,20,0],"rotation":down},
            "render.camera":{"projection":{"kind":"orthographic","height":20},"near":0.1,"far":100,"viewport":[0,0,1,1],"active":true}}}
    ]));
    w.capabilities.insert("render".into(), 1);
    w.resources.insert(
        "nav.grid".into(),
        json!({"pointer_agent":"bob","bounds":{"min":[-10,-10],"max":[10,10]}}),
    );
    let mut r = runtime(w.clone());
    // Idle until told; nav.go to a point, stop, then a binding sends it to an entity.
    steps(&mut r, 10);
    assert_eq!(state_of(&r, "bob"), "idle");
    let go = call(&mut r, "go", json!({"id":"bob","point":[4,0,0]})).unwrap();
    assert_eq!(go["state"], "pending");
    steps(&mut r, 60);
    assert_eq!(state_of(&r, "bob"), "moving");
    let stopped = call(&mut r, "stop", json!({"id":"bob"})).unwrap();
    assert_eq!(stopped["state"], "idle");
    steps(&mut r, 120);
    let resting = position(&r, "bob");
    steps(&mut r, 60);
    assert_eq!(position(&r, "bob"), resting, "braked to a stop");
    r.command(&RuntimeCommand::Emit {
        event: "send_home".into(),
        fields: Default::default(),
    })
    .unwrap();
    until(&mut r, "bob", "arrived", 1200);
    assert!(xz(position(&r, "bob")).distance(Vec2::new(0.0, -6.0)) <= 0.21);
    assert_eq!(var(&r, "target"), "home");
    // A primary click on the ground sends the pointer agent there (orthographic top view:
    // screen center = world origin, right = +X).
    r.screen_pointer(PointerKind::PrimaryDown, [0.75, 0.5], 1.0, false)
        .unwrap();
    let inspect = call(&mut r, "inspect", json!({"id":"bob"})).unwrap();
    assert_eq!(inspect["state"], "pending");
    let target = inspect["target"]["point"].as_array().unwrap().clone();
    assert!(
        (target[0].as_f64().unwrap() - 5.0).abs() < 0.01,
        "{inspect}"
    );
    assert!(target[1].as_f64().unwrap().abs() < 0.01, "{inspect}");
    until(&mut r, "bob", "arrived", 1200);
    // Unknown actions and checkpoints fail explicitly.
    assert!(call(&mut r, "jump", json!({})).is_err());
    let mut saved = w;
    saved.capabilities.insert("save".into(), 1);
    saved.systems.retain(|s| s != "path.move");
    let error = format!("{:#}", runtime(saved).checkpoint().unwrap_err());
    assert!(
        error.contains("nav.steer does not implement checkpoints"),
        "{error}"
    );
}

#[test]
fn arrival_drops_stale_replans_and_bodies_under_agents_are_not_obstacles() {
    // bob chases a slow entity target; six agents bound for a closed room hog the tiny
    // search budget, so bob's re-plan (queued when the target moved) is still waiting when bob
    // reaches the target. The arrival must stay final: one event, state arrived, no failure.
    let mut entities = vec![
        agent(
            "bob",
            [0.0, 0.0, -8.0],
            json!({"speed":2.0,"arrive_distance":0.6,"target":{"entity":"dog"},"on_arrive":"arrived","on_fail":"failed"}),
        ),
        json!({"id":"dog","components":{"core.transform":{"position":[5,0,-8]},"core.motion":{"velocity":[0,0,-0.6]}}}),
    ];
    entities.extend(room(0.0));
    for k in 0..6 {
        entities.push(agent(
            &format!("busy/{k}"),
            [-6.0 + k as f32, 0.0, -2.0],
            json!({"target":{"point":[0,0,8]},"avoidance":false}),
        ));
    }
    let mut r = runtime(world_with(
        Value::Array(entities),
        json!({"expansions_per_step":64}),
    ));
    let mut queued_at_arrival = false;
    for _ in 0..4000 {
        r.step(&ActionMap::default());
        if state_of(&r, "bob") == "arrived" {
            queued_at_arrival =
                summary(&r)["pending"].as_u64().unwrap() > 0 || !summary(&r)["searching"].is_null();
            break;
        }
    }
    assert_eq!(state_of(&r, "bob"), "arrived", "{}", summary(&r));
    assert!(
        queued_at_arrival,
        "the scenario needs other requests still waiting"
    );
    // Let every queued request finish; the target walks on but bob does not follow.
    steps(&mut r, 3000);
    assert_eq!(state_of(&r, "bob"), "arrived", "{}", summary(&r));
    assert_eq!(var(&r, "arrivals"), 1);
    assert_eq!(var(&r, "fails"), 0); // A collider on a child of an agent moves with it: it is not a static obstacle.
    let mut w = world(json!([
        agent("npc", [0.0, 0.0, 0.0], json!({"target":{"point":[4,0,0]}})),
        {"id":"npc/body","components":{"core.transform":{"position":[0,1,0]},"physics.collider":{"shape":{"shape":"capsule","half_height":0.5,"radius":0.3}}}},
        {"id":"crate","components":{"core.transform":{"position":[2,1,3]},"physics.collider":{"shape":{"shape":"box","half_extents":[0.5,1,0.5]}}}}
    ]));
    w.capabilities.insert("physics".into(), 1);
    w.relations.push(
        serde_json::from_value(json!({"kind":"parent","source":"npc/body","target":"npc"}))
            .unwrap(),
    );
    let mut r = runtime(w);
    assert_eq!(summary(&r)["grid"]["obstacles"], 1, "only the static crate");
    until(&mut r, "npc", "arrived", 1200);
}
