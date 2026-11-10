//! Follow cameras (`camera.follow` + `camera.rig`): orbit and look point, smoothing, wall
//! pull-in and smooth recovery, an interior room, minimum distance, input axes,
//! `camera.activate` / `camera.follow.set` / `camera.inspect`, authoring errors and mover
//! conflicts, budgets, hot reload, checkpoints and spawns.
use axiom_core::{gameplay::RuntimeCommand, raycast::Shape, World};
use axiom_runtime::{camera_rig::CameraRigSystem, installed_registry, ActionMap, WorldRuntime};
use glam::{Quat, Vec3};
use serde_json::{json, Value};

fn world(entities: Value) -> World {
    serde_json::from_value(json!({
        "schema_version":2,"id":"camera-test","title":"Camera test",
        "capabilities":{"core":1,"render":1,"camera":1,"physics":1},
        "systems":["camera.rig"],
        "entities":entities,
        "resources":{"input.bindings":{"axes":{"orbit":{"positive":"KeyC","negative":"KeyZ"},"zoom":{"positive":"KeyE","negative":"KeyQ"},"tilt":{"positive":"KeyR","negative":"KeyF"}},"buttons":{}}}
    }))
    .unwrap()
}

fn camera(id: &str, follow: Value) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":[0,10,0]},
        "render.camera":{"projection":{"kind":"perspective","fov_degrees":60},"near":0.05,"far":200,"viewport":[0,0,1,1],"active":true},
        "camera.follow":follow}})
}

fn hero(at: [f32; 3]) -> Value {
    json!({"id":"hero","tags":["player"],"components":{"core.transform":{"position":at}}})
}

/// A wall box as a static `physics.collider` (center, half extents).
fn wall(id: &str, center: [f32; 3], half: [f32; 3]) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":center},
        "physics.collider":{"shape":{"shape":"box","half_extents":half}}}})
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, installed_registry()).unwrap()
}

fn steps(r: &mut WorldRuntime, n: usize) {
    for _ in 0..n {
        r.step(&ActionMap::default());
    }
}

fn slot(r: &WorldRuntime, id: &str) -> usize {
    r.state.compiled.indices[id]
}

fn eye(r: &WorldRuntime, id: &str) -> Vec3 {
    Vec3::from_array(r.state.transforms[slot(r, id)].as_ref().unwrap().position)
}

fn forward(r: &WorldRuntime, id: &str) -> Vec3 {
    Quat::from_array(r.state.transforms[slot(r, id)].as_ref().unwrap().rotation) * -Vec3::Z
}

fn place(r: &mut WorldRuntime, id: &str, p: Vec3) {
    r.state.set_position(id, p.to_array());
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "camera".into(),
        action: action.into(),
        args,
    })
    .map(|v| v["result"].clone())
}

fn inspect(r: &mut WorldRuntime, id: &str) -> Value {
    call(r, "inspect", json!({ "id": id })).unwrap()
}

fn compile_error(w: &World) -> String {
    format!(
        "{:#}",
        w.compile(&installed_registry().authoring).unwrap_err()
    )
}

fn close(a: Vec3, b: Vec3, tolerance: f32) -> bool {
    a.distance(b) < tolerance
}

fn orbit(yaw: f32, pitch: f32) -> Vec3 {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    Vec3::new(y.sin() * p.cos(), p.sin(), y.cos() * p.cos())
}

#[test]
fn clear_view_orbits_at_the_distance_and_looks_at_the_pivot() {
    let mut r = runtime(world(json!([
        hero([2.0, 0.0, 3.0]),
        camera(
            "cam",
            json!({"target":"hero","distance":6,"yaw":30,"pitch":20,"look_height":1.5,
            "position_smoothing":0,"rotation_smoothing":0})
        ),
    ])));
    // The authored pose stays until the first step.
    assert!(close(eye(&r, "cam"), Vec3::new(0.0, 10.0, 0.0), 1e-6));
    steps(&mut r, 1);
    let pivot = Vec3::new(2.0, 1.5, 3.0);
    let expected = pivot + orbit(30.0, 20.0) * 6.0;
    assert!(close(eye(&r, "cam"), expected, 1e-4), "{}", eye(&r, "cam"));
    let look = (pivot - eye(&r, "cam")).normalize();
    assert!(close(forward(&r, "cam"), look, 1e-4));
    let i = inspect(&mut r, "cam");
    assert_eq!(i["desired"], 6.0);
    assert_eq!(i["actual"], 6.0);
    assert!(i["hit"].is_null() && i["focus_hit"].is_null(), "{i}");
    // The observe section lists desired vs actual distance and the hit entity.
    let o = &r.observation().metrics["camera"];
    assert_eq!(o["follow"][0]["id"], "cam");
    assert_eq!(o["follow"][0]["desired"], 6.0);
    assert_eq!(o["follow"][0]["actual"], 6.0);
    assert!(o["follow"][0]["hit"].is_null());
    assert_eq!(o["active"], json!(["cam"]));
    // A rigid follow keeps the same offset while the target moves.
    place(&mut r, "hero", Vec3::new(-5.0, 0.0, 1.0));
    steps(&mut r, 1);
    let expected = Vec3::new(-5.0, 1.5, 1.0) + orbit(30.0, 20.0) * 6.0;
    assert!(close(eye(&r, "cam"), expected, 1e-4));
}

#[test]
fn a_target_jump_is_followed_smoothly_and_monotonically() {
    let mut r = runtime(world(json!([
        hero([0.0, 0.0, 0.0]),
        camera("cam", json!({"target":"hero","position_smoothing":0.2})),
    ])));
    steps(&mut r, 1);
    let start = eye(&r, "cam");
    place(&mut r, "hero", Vec3::new(10.0, 0.0, 0.0));
    let goal = start + Vec3::X * 10.0;
    let mut last = start.distance(goal);
    let mut first_step = None;
    for _ in 0..600 {
        steps(&mut r, 1);
        let gap = eye(&r, "cam").distance(goal);
        assert!(
            gap <= last + 1e-5,
            "the camera never moves away from its goal"
        );
        first_step.get_or_insert(last - gap);
        last = gap;
    }
    assert!(last < 1e-3, "settles: {last}");
    // No pop: the first step covers only dt / half-life of the jump (2^(-1/24) left).
    let moved = first_step.unwrap();
    assert!(moved > 0.2 && moved < 0.4, "{moved}");
}

#[test]
fn walls_pull_the_camera_in_at_once_and_it_recovers_smoothly() {
    // Wall 3 m behind the target (its near face at z = 3), the camera wants 6 m at yaw 0.
    let mut r = runtime(world(json!([
        hero([0.0, 0.0, 0.0]),
        wall("wall", [0.0, 1.5, 3.15], [1.0, 1.5, 0.15]),
        camera(
            "cam",
            json!({"target":"hero","distance":6,"pitch":0,"look_height":1,
            "position_smoothing":0,"rotation_smoothing":0,"collision":{"radius":0.25,"recover":0.25}})
        ),
    ])));
    steps(&mut r, 1);
    let i = inspect(&mut r, "cam");
    assert_eq!(i["hit"], "wall");
    assert_eq!(i["actual"], 2.75, "{i}");
    assert!(close(eye(&r, "cam"), Vec3::new(0.0, 1.0, 2.75), 1e-4));
    // Step aside: nothing blocks any more; the camera moves back out without jumping.
    place(&mut r, "hero", Vec3::new(3.0, 0.0, 0.0));
    let mut last = 2.75;
    let mut largest = 0.0f32;
    for _ in 0..360 {
        steps(&mut r, 1);
        let d = inspect(&mut r, "cam")["actual"].as_f64().unwrap() as f32;
        assert!(d >= last - 1e-3, "recovery is monotone");
        largest = largest.max(d - last);
        last = d;
    }
    assert!(largest < 0.12, "no pop: largest step {largest} m");
    assert!((last - 6.0).abs() < 0.01, "{last}");
    // Back behind the wall: pulled in within the same step (never through the wall).
    place(&mut r, "hero", Vec3::new(0.0, 0.0, 0.0));
    steps(&mut r, 1);
    assert_eq!(inspect(&mut r, "cam")["actual"], 2.75);
    // With collision off the wall is ignored.
    let mut w = world(json!([
        hero([0.0, 0.0, 0.0]),
        wall("wall", [0.0, 1.5, 3.15], [1.0, 1.5, 0.15]),
        camera(
            "cam",
            json!({"target":"hero","distance":6,"collision":{"enabled":false}})
        ),
    ]));
    w.id = "camera-off".into();
    let mut r = runtime(w);
    steps(&mut r, 1);
    assert_eq!(inspect(&mut r, "cam")["actual"], 6.0);
}

#[test]
fn the_minimum_distance_wins_over_a_nearer_wall() {
    let mut r = runtime(world(json!([
        hero([0.0, 0.0, 0.0]),
        wall("wall", [0.0, 1.5, 0.75], [1.0, 1.5, 0.15]),
        camera(
            "cam",
            json!({"target":"hero","distance":6,"min_distance":1.5,"pitch":0,"look_height":1,
            "position_smoothing":0,"rotation_smoothing":0})
        ),
    ])));
    steps(&mut r, 1);
    let i = inspect(&mut r, "cam");
    assert_eq!(i["hit"], "wall");
    assert_eq!(i["allowed"], 0.4, "{i}");
    assert_eq!(i["actual"], 1.5);
}

/// An 8 x 8 m room (walls 0.3 m thick, 3 m high) with a 1.6 m doorway in the south wall
/// (+Z). The three blocker sources each build some walls.
fn room() -> (Vec<Value>, Vec<Shape>) {
    let mut entities = vec![
        wall("room/north", [0.0, 1.5, -4.15], [4.3, 1.5, 0.15]),
        wall("room/south_left", [-2.55, 1.5, 4.15], [1.75, 1.5, 0.15]),
        wall("room/south_right", [2.55, 1.5, 4.15], [1.75, 1.5, 0.15]),
    ];
    // West: nav.blocker whose height comes from its own mesh.
    entities.push(json!({"id":"room/west","components":{"core.transform":{"position":[-4.15,0,0]},
        "nav.blocker":{"half":[0.15,4.3]},
        "render.mesh":{"parts":[{"transform":{"position":[0,1.5,0],"scale":[0.3,3,8.6]},"color":[0.5,0.5,0.5]}]}}}));
    // East: a mesh that only blocks the camera (camera.blocker).
    entities.push(json!({"id":"room/east","components":{"core.transform":{"position":[4.15,1.5,0]},
        "render.mesh":{"parts":[{"transform":{"scale":[0.3,3,8.6]},"color":[0.5,0.5,0.5]}]},"camera.blocker":{}}}));
    let boxes = [
        ([0.0, 1.5, -4.15], [4.3, 1.5, 0.15]),
        ([-2.55, 1.5, 4.15], [1.75, 1.5, 0.15]),
        ([2.55, 1.5, 4.15], [1.75, 1.5, 0.15]),
        ([-4.15, 1.5, 0.0], [0.15, 1.5, 4.3]),
        ([4.15, 1.5, 0.0], [0.15, 1.5, 4.3]),
    ]
    .map(|(c, h)| {
        let (c, h) = (Vec3::from_array(c), Vec3::from_array(h));
        Shape::aligned(c - h, c + h)
    });
    (entities, boxes.to_vec())
}

#[test]
fn walking_into_a_room_keeps_the_camera_out_of_the_walls_and_ends_inside() {
    let (mut entities, walls) = room();
    entities.push(hero([0.0, 0.0, 10.0]));
    entities.push(camera(
        "cam",
        json!({"target":"hero","distance":6,"pitch":20,"look_height":1.5,"collision":{"radius":0.2}}),
    ));
    let mut r = runtime(world(Value::Array(entities)));
    let cam = slot(&r, "cam");
    // Through the doorway to the middle (2 m/s), then aside so the south wall (not the
    // doorway) is behind the hero.
    let mut route: Vec<Vec3> = (0..=600)
        .map(|k| Vec3::new(0.0, 0.0, 10.0 - k as f32 / 60.0))
        .collect();
    route.extend((1..=150).map(|k| Vec3::new(2.5, 0.0, 1.0) * (k as f32 / 150.0)));
    route.extend(std::iter::repeat_n(Vec3::new(2.5, 0.0, 1.0), 240));
    for at in route {
        place(&mut r, "hero", at);
        steps(&mut r, 1);
        let e = eye(&r, "cam");
        for w in &walls {
            assert!(
                !w.contains(e, 0.0),
                "camera {e} inside a wall with the hero at {at}"
            );
        }
        // Line of sight: target pivot -> look point -> camera never crosses a wall.
        let (eye_at, focus) = r.system::<CameraRigSystem>().unwrap().pose(cam).unwrap();
        assert!(close(eye_at, e, 1e-4));
        let pivot = at + Vec3::Y * 1.5;
        for w in &walls {
            assert!(
                w.cast(pivot, focus - pivot, 0.0).is_none(),
                "focus behind a wall at {at}"
            );
            assert!(
                w.cast(focus, e - focus, 0.0).is_none(),
                "camera behind a wall at {at}"
            );
        }
    }
    let e = eye(&r, "cam");
    assert!(
        e.x.abs() < 4.0 && e.z.abs() < 4.0,
        "the camera ends inside the room: {e}"
    );
    let i = inspect(&mut r, "cam");
    assert!(i["actual"].as_f64().unwrap() < 6.0, "{i}");
    assert!(i["hit"].as_str().unwrap().starts_with("room/"), "{i}");
}

#[test]
fn nav_blockers_without_a_mesh_are_unbounded_and_inactive_scenes_do_not_block() {
    let mut w = world(json!([
        hero([0.0, 0.0, 0.0]),
        {"id":"fence","components":{"core.transform":{"position":[0,0,3]},"nav.blocker":{"half":[2,0.2]}}},
        {"id":"later","components":{"core.transform":{"position":[0,1,-2]},"physics.collider":{"shape":{"shape":"capsule","half_height":1,"radius":0.3}}}},
        camera("cam", json!({"target":"hero","distance":6,"pitch":60,"position_smoothing":0,"rotation_smoothing":0})),
        camera("back", json!({"target":"hero","distance":6,"yaw":180,"pitch":0,"look_height":1,"position_smoothing":0,"rotation_smoothing":0})),
    ]));
    w.entities[4].components.get_mut("render.camera").unwrap()["active"] = json!(false);
    w.scenes.push(
        serde_json::from_value(json!({"id":"s","entities":["later"],"active":false})).unwrap(),
    );
    let mut r = runtime(w);
    steps(&mut r, 1);
    // Steep pitch: the camera at 6 m would be 5.2 m up, over any wall of finite height,
    // but a mesh-less nav.blocker has no height.
    let i = inspect(&mut r, "cam");
    assert_eq!(i["hit"], "fence", "{i}");
    // The capsule behind the hero is in an inactive scene: no block.
    let i = inspect(&mut r, "back");
    assert!(i["hit"].is_null(), "{i}");
    assert_eq!(i["actual"], 6.0);
}

#[test]
fn input_axes_orbit_tilt_and_zoom() {
    let mut r = runtime(world(json!([
        hero([0.0, 0.0, 0.0]),
        camera(
            "cam",
            json!({"target":"hero","distance":5,"pitch":10,"position_smoothing":0,"rotation_smoothing":0,
            "input":{"yaw_axis":"orbit","pitch_axis":"tilt","zoom_axis":"zoom","yaw_speed":120,"pitch_speed":60,"zoom_speed":4}})
        ),
    ])));
    steps(&mut r, 1);
    let mut input = ActionMap::default();
    input.0.insert("orbit".into(), 1.0);
    input.0.insert("tilt".into(), 1.0);
    input.0.insert("zoom".into(), 1.0);
    // 0.5 s at full axis: +60 deg yaw, +30 deg pitch, +2 m.
    for _ in 0..60 {
        r.step(&input);
    }
    let i = inspect(&mut r, "cam");
    assert!((i["yaw"].as_f64().unwrap() - 60.0).abs() < 0.01, "{i}");
    assert!((i["pitch"].as_f64().unwrap() - 40.0).abs() < 0.01, "{i}");
    assert!((i["desired"].as_f64().unwrap() - 7.0).abs() < 0.01, "{i}");
    let expected = Vec3::new(0.0, 1.5, 0.0) + orbit(60.0, 40.0) * 7.0;
    assert!(close(eye(&r, "cam"), expected, 1e-3));
    // Limits clamp: 10 s more reaches max_pitch 75 and max_distance 12.
    for _ in 0..1200 {
        r.step(&input);
    }
    let i = inspect(&mut r, "cam");
    assert_eq!(i["pitch"], 75.0);
    assert_eq!(i["desired"], 12.0);
    // Releasing the keys holds the pose.
    let before = eye(&r, "cam");
    steps(&mut r, 30);
    assert!(close(eye(&r, "cam"), before, 1e-5));
    // A hot reload keeps what the player turned and zoomed to.
    let mut edited = r.state.compiled.source.clone();
    edited.entities.push(
        serde_json::from_value(json!({"id":"lamp","components":{"core.transform":{}}})).unwrap(),
    );
    r.reconcile(edited, 2).unwrap();
    steps(&mut r, 30);
    assert!(
        close(eye(&r, "cam"), before, 1e-4),
        "{} vs {before}",
        eye(&r, "cam")
    );
}

#[test]
fn hugging_a_wall_around_a_corner_never_puts_the_camera_behind_it() {
    // A building whose south-east corner is at the origin; the hero walks 0.15 m from its
    // east face, around the corner and along its south face, with the camera on the
    // building's side (yaw 180) and a trailing look point.
    let mut r = runtime(world(json!([
        hero([0.15, 0.0, -3.0]),
        wall("building", [-2.0, 1.5, -2.0], [2.0, 1.5, 2.0]),
        camera(
            "cam",
            json!({"target":"hero","distance":6,"min_distance":0.1,"yaw":135,"pitch":5,
            "look_height":1,"position_smoothing":0.4,"rotation_smoothing":0})
        ),
    ])));
    let cam = slot(&r, "cam");
    // 1 cm inside the real faces, so a point on the surface counts as outside.
    let solid = Shape::aligned(Vec3::new(-3.99, 0.01, -3.99), Vec3::new(-0.01, 2.99, -0.01));
    let mut route: Vec<Vec3> = (0..=190)
        .map(|k| Vec3::new(0.15, 0.0, -3.0 + k as f32 / 60.0))
        .collect();
    route.extend((1..=200).map(|k| Vec3::new(0.15 - k as f32 / 60.0, 0.0, 0.15)));
    for at in route {
        place(&mut r, "hero", at);
        steps(&mut r, 1);
        let (e, focus) = r.system::<CameraRigSystem>().unwrap().pose(cam).unwrap();
        // At min_distance (0.1 m) the camera may sit in or past a nearer wall (documented);
        // otherwise it is outside and the line from the look point to it is clear.
        let at_min = e.distance(focus) < 0.1 + 1e-4;
        assert!(
            at_min || !solid.contains(e, 0.0),
            "camera {e} inside with the hero at {at}"
        );
        assert!(
            at_min || solid.cast(focus, e - focus, 0.0).is_none(),
            "camera {e} behind the building (look point {focus}) with the hero at {at}"
        );
        let pivot = at + Vec3::Y;
        assert!(
            solid.cast(pivot, focus - pivot, 0.0).is_none(),
            "look point {focus} behind it"
        );
    }
}

#[test]
fn activate_follow_set_and_inspect_are_recoverable_actions() {
    let mut w = world(json!([
        hero([0.0, 0.0, 0.0]),
        {"id":"dog","components":{"core.transform":{"position":[10,0,0]}}},
        camera("cam", json!({"target":"hero","position_smoothing":0,"rotation_smoothing":0})),
        {"id":"overview","components":{"core.transform":{"position":[0,30,0]},
            "render.camera":{"projection":{"kind":"perspective","fov_degrees":60},"near":0.05,"far":200,"viewport":[0,0,1,1],"active":false}}},
    ]));
    w.id = "camera-actions".into();
    let mut r = runtime(w.clone());
    steps(&mut r, 1);
    let views = |r: &WorldRuntime| -> Vec<String> {
        r.render_frame().views.into_iter().map(|v| v.id).collect()
    };
    assert_eq!(views(&r), ["cam"]);
    let out = call(&mut r, "activate", json!({"id":"overview"})).unwrap();
    assert_eq!(out["active"], "overview");
    assert_eq!(out["deactivated"], json!(["cam"]));
    assert_eq!(views(&r), ["overview"]);
    assert_eq!(
        r.observation().metrics["camera"]["active"],
        json!(["overview"])
    );
    // Hot reload with an unrelated edit keeps the runtime activation.
    let mut edited = w.clone();
    edited.entities.push(
        serde_json::from_value(json!({"id":"lamp","components":{"core.transform":{}}})).unwrap(),
    );
    r.reconcile(edited.clone(), 2).unwrap();
    assert_eq!(views(&r), ["overview"]);
    // An authored change to the active flags wins over the runtime override.
    let mut authored = edited.clone();
    authored
        .entity_mut("cam")
        .unwrap()
        .components
        .get_mut("render.camera")
        .unwrap()["active"] = json!(false);
    authored
        .entity_mut("overview")
        .unwrap()
        .components
        .get_mut("render.camera")
        .unwrap()["active"] = json!(true);
    r.reconcile(authored, 3).unwrap();
    assert!(r.observation().metrics["camera"]["activated"].is_null());
    call(&mut r, "activate", json!({"id":"cam"})).unwrap();
    assert_eq!(views(&r), ["cam"]);
    // Inspect a plain camera.
    let plain = inspect(&mut r, "overview");
    assert_eq!(plain["follow"], false);
    assert_eq!(plain["active"], false);
    // follow.set: new target, distance and yaw.
    let out = call(
        &mut r,
        "follow.set",
        json!({"id":"cam","target":"dog","distance":8,"yaw":90}),
    )
    .unwrap();
    assert_eq!(out["target"], "dog");
    steps(&mut r, 1);
    let expected = Vec3::new(10.0, 1.5, 0.0) + orbit(90.0, 20.0) * 8.0;
    assert!(close(eye(&r, "cam"), expected, 1e-3), "{}", eye(&r, "cam"));
    // Recoverable errors.
    let e = |r: &mut WorldRuntime, action: &str, args: Value| {
        format!("{:#}", call(r, action, args).unwrap_err())
    };
    assert!(e(&mut r, "activate", json!({"id":"hero"})).contains("no render.camera"));
    assert!(e(&mut r, "activate", json!({"id":"nobody"})).contains("does not exist"));
    assert!(e(&mut r, "follow.set", json!({"id":"overview"})).contains("not a follow camera"));
    assert!(
        e(&mut r, "follow.set", json!({"id":"cam","target":"ghost"})).contains("does not exist")
    );
    assert!(e(&mut r, "follow.set", json!({"id":"cam","target":"cam"})).contains("camera itself"));
    assert!(e(&mut r, "follow.set", json!({"id":"cam","distance":50})).contains("min_distance"));
    assert!(e(&mut r, "follow.set", json!({"id":"cam","pitch":85})).contains("min_pitch"));
    assert!(e(&mut r, "follow.set", json!({"id":"cam","yaw":"left"})).contains("number"));
    assert!(e(&mut r, "follow.set", json!({"id":"cam","yaw":1e39})).contains("-3600..3600"));
    assert!(e(&mut r, "inspect", json!({})).contains("id"));
    assert!(e(&mut r, "inspect", json!({"id":"hero"})).contains("no render.camera"));
}

#[test]
fn hot_reload_keeps_the_camera_state() {
    let w = world(json!([
        hero([0.0, 0.0, 0.0]),
        {"id":"dog","components":{"core.transform":{"position":[10,0,0]}}},
        camera("cam", json!({"target":"hero","position_smoothing":0.3,"rotation_smoothing":0.3})),
    ]));
    // Two identical runs; one is hot reloaded mid-transition.
    let mut r = runtime(w.clone());
    let mut twin = runtime(w.clone());
    for r in [&mut r, &mut twin] {
        steps(r, 1);
        // The target moved and the camera is still easing when a new target is set.
        place(r, "hero", Vec3::new(4.0, 0.0, 0.0));
        steps(r, 20);
        call(r, "follow.set", json!({"id":"cam","target":"dog","yaw":45})).unwrap();
        steps(r, 5);
    }
    // Unrelated edit: the camera continues exactly as without the reload (no pop), with
    // the runtime target and goals.
    let mut edited = w.clone();
    edited.entities.push(
        serde_json::from_value(json!({"id":"lamp","components":{"core.transform":{}}})).unwrap(),
    );
    r.reconcile(edited.clone(), 2).unwrap();
    for _ in 0..30 {
        steps(&mut r, 1);
        steps(&mut twin, 1);
        assert!(
            close(eye(&r, "cam"), eye(&twin, "cam"), 1e-5),
            "{} vs {}",
            eye(&r, "cam"),
            eye(&twin, "cam")
        );
    }
    let after = inspect(&mut r, "cam");
    assert_eq!(after["target"], "dog");
    assert_eq!(after, inspect(&mut twin, "cam"));
    // Editing camera.follow keeps the pose but goes back to the authored target and goals.
    let before = eye(&r, "cam");
    let mut changed = edited.clone();
    changed
        .entity_mut("cam")
        .unwrap()
        .components
        .get_mut("camera.follow")
        .unwrap()["distance"] = json!(8);
    r.reconcile(changed, 3).unwrap();
    steps(&mut r, 1);
    assert!(eye(&r, "cam").distance(before) < 0.3);
    let after = inspect(&mut r, "cam");
    assert_eq!(after["target"], "hero");
    assert_eq!(after["goal"], 8.0);
}

#[test]
fn authoring_errors_and_mover_conflicts_are_explicit() {
    let code = |w: &World| compile_error(w);
    let base = || {
        vec![
            hero([0.0, 0.0, 0.0]),
            camera("cam", json!({"target":"hero"})),
        ]
    };
    // Missing system.
    let mut w = world(Value::Array(base()));
    w.systems.clear();
    assert!(
        code(&w).contains("needs the camera.rig system"),
        "{}",
        code(&w)
    );
    // Target problems.
    let e = code(&world(json!([camera("cam", json!({"target":"ghost"}))])));
    assert!(
        e.contains("invalid_camera") && e.contains("ghost does not exist"),
        "{e}"
    );
    let e = code(&world(json!([camera("cam", json!({"target":"cam"}))])));
    assert!(e.contains("camera itself"), "{e}");
    // Two cameras following each other.
    let mut b = camera("b", json!({"target":"a"}));
    b["components"]["render.camera"]["active"] = json!(false);
    let e = code(&world(json!([camera("a", json!({"target":"b"})), b])));
    assert!(e.contains("is itself a follow camera"), "{e}");
    let e = code(&world(json!([
        {"id":"nowhere","components":{}},
        camera("cam", json!({"target":"nowhere"})),
    ])));
    assert!(e.contains("has no core.transform"), "{e}");
    // Not a camera / not a root.
    let mut plain = camera("cam", json!({"target":"hero"}));
    plain["components"]
        .as_object_mut()
        .unwrap()
        .remove("render.camera");
    let e = code(&world(json!([hero([0.0; 3]), plain])));
    assert!(e.contains("no render.camera"), "{e}");
    let mut w = world(Value::Array(base()));
    w.relations.push(
        serde_json::from_value(json!({"kind":"parent","source":"cam","target":"hero"})).unwrap(),
    );
    let e = code(&w);
    assert!(e.contains("must be a root entity"), "{e}");
    // Unbound input axis.
    let e = code(&world(json!([
        hero([0.0; 3]),
        camera("cam", json!({"target":"hero","input":{"yaw_axis":"spin"}})),
    ])));
    assert!(e.contains("input.bindings has no axis spin"), "{e}");
    // Field validation.
    let e = code(&world(json!([
        hero([0.0; 3]),
        camera("cam", json!({"target":"hero","distance":30})),
    ])));
    assert!(e.contains("distance must lie within"), "{e}");
    // Movers: one mover per camera.
    let mut moving = camera("cam", json!({"target":"hero"}));
    moving["components"]["core.motion"] = json!({"velocity":[1,0,0]});
    let mut w = world(json!([hero([0.0; 3]), moving]));
    w.capabilities.insert("motion".into(), 1);
    w.systems.push("motion.integrate".into());
    let e = code(&w);
    assert!(
        e.contains("camera_conflict") && e.contains("core.motion (moved by motion.integrate)"),
        "{e}"
    );
    // A tween animating the follow camera's transform.
    let mut tweened = camera("cam", json!({"target":"hero"}));
    tweened["components"]["tween.clips"] = json!({"clips":{"pan":{"tracks":[{"property":"core.transform/position","keys":[{"t":0,"v":[0,0,0]},{"t":1,"v":[1,0,0]}]}]}}});
    let mut w = world(json!([hero([0.0; 3]), tweened]));
    w.capabilities.insert("tween".into(), 1);
    w.systems.push("tween.animate".into());
    let e = code(&w);
    assert!(
        e.contains("tween_conflict") && e.contains("camera.follow (moved by camera.rig)"),
        "{e}"
    );
    // A camera_blocker mesh under an animated transform cannot stay static.
    let mut w = world(json!([
        {"id":"door","tags":["camera_blocker"],"components":{"core.transform":{},
            "render.mesh":{"parts":[{"color":[1,1,1]}]},
            "tween.clips":{"clips":{"open":{"tracks":[{"property":"core.transform/yaw","keys":[{"t":0,"v":0},{"t":1,"v":90}]}]}}}}}
    ]));
    w.capabilities.insert("tween".into(), 1);
    w.systems = vec!["tween.animate".into()];
    let e = code(&w);
    assert!(
        e.contains("tween_conflict") && e.contains("render.mesh tagged camera_blocker"),
        "{e}"
    );
    let door = w.entity_mut("door").unwrap();
    door.tags.clear();
    door.components.insert("camera.blocker".into(), json!({}));
    let e = code(&w);
    assert!(
        e.contains("tween_conflict") && e.contains("render.mesh with camera.blocker"),
        "{e}"
    );
    // A gameplay session that places its own camera (action.session in axiom-town).
    let mut town: World =
        serde_json::from_str(include_str!("../../../content/axiom-town/world.json")).unwrap();
    town.capabilities.insert("camera".into(), 1);
    town.systems.push("camera.rig".into());
    town.entity_mut("camera")
        .unwrap()
        .components
        .insert("camera.follow".into(), json!({"target":"hero"}));
    let e = code(&town);
    assert!(
        e.contains("camera_conflict") && e.contains("resource action.session drives camera camera"),
        "{e}"
    );
}

#[test]
fn budgets_are_explicit() {
    // Nine follow cameras (inactive, so the active camera budget is not the limit).
    let mut entities = vec![hero([0.0; 3])];
    for i in 0..9 {
        let mut c = camera(&format!("cam/{i}"), json!({"target":"hero"}));
        c["components"]["render.camera"]["active"] = json!(false);
        entities.push(c);
    }
    let e = compile_error(&world(Value::Array(entities)));
    assert!(
        e.contains("camera_budget") && e.contains("follow camera budget exceeded (9 > 8)"),
        "{e}"
    );
    // Blocker shapes: 4093 entities with a collider and a nav.blocker (8186) plus a tagged
    // mesh of 7 parts = 8193.
    let mut entities = vec![hero([0.0; 3]), camera("cam", json!({"target":"hero"}))];
    for i in 0..4093 {
        let x = (i % 64) as f32 * 3.0;
        let z = (i / 64) as f32 * 3.0 + 20.0;
        entities.push(json!({"id":format!("w/{i}"),"components":{"core.transform":{"position":[x,0,z]},
            "physics.collider":{"shape":{"shape":"box","half_extents":[0.5,0.5,0.5]}},"nav.blocker":{"half":[0.5,0.5]}}}));
    }
    let parts: Vec<Value> = (0..7).map(|_| json!({"color":[1,1,1]})).collect();
    entities.push(json!({"id":"tagged","tags":["camera_blocker"],"components":{"core.transform":{},"render.mesh":{"parts":parts}}}));
    let mut w = world(Value::Array(entities));
    let e = compile_error(&w);
    assert!(
        e.contains("camera_budget") && e.contains("camera blocker budget exceeded"),
        "{e}"
    );
    // 8192 exactly is fine.
    let last = w.entities.len() - 1;
    w.entities[last].components.get_mut("render.mesh").unwrap()["parts"] =
        json!((0..6).map(|_| json!({"color":[1,1,1]})).collect::<Vec<_>>());
    assert!(w.compile(&installed_registry().authoring).is_ok());
    // Per-cast test budget: 600 small boxes above the line of sight (never hit) crowd the
    // cells the cast visits; the camera stays at min_distance and observe counts it.
    let mut entities = vec![
        hero([0.0; 3]),
        camera(
            "cam",
            json!({"target":"hero","distance":6,"min_distance":1.2,"pitch":0,"position_smoothing":0,"rotation_smoothing":0}),
        ),
    ];
    for i in 0..600 {
        let (x, z) = ((i % 20) as f32 * 0.1 - 1.0, (i / 20) as f32 * 0.2);
        entities.push(
            json!({"id":format!("crowd/{i}"),"components":{"core.transform":{"position":[x,40,z]},
            "physics.collider":{"shape":{"shape":"box","half_extents":[0.04,0.04,0.04]}}}}),
        );
    }
    let mut r = runtime(world(Value::Array(entities)));
    steps(&mut r, 2);
    let i = inspect(&mut r, "cam");
    assert_eq!(i["actual"], 1.2, "{i}");
    assert_eq!(i["tests"], 512);
    assert_eq!(r.observation().metrics["camera"]["over_budget"], 2);
}

#[test]
fn disabled_follows_spawns_and_checkpoints_are_explicit() {
    let mut r = runtime(world(json!([
        hero([0.0; 3]),
        camera("cam", json!({"target":"hero","enabled":false})),
    ])));
    steps(&mut r, 10);
    assert!(
        close(eye(&r, "cam"), Vec3::new(0.0, 10.0, 0.0), 1e-6),
        "a disabled follow leaves the camera"
    );
    let template = r.state.compiled.entities[slot(&r, "cam")].clone();
    r.state.spawn(&template, "cam/2".into(), [0.0; 3]).unwrap();
    steps(&mut r, 1);
    let halted = r.observation().metrics["camera"]["halted"].clone();
    assert!(
        halted
            .as_str()
            .unwrap()
            .contains("spawned follow cameras are not supported"),
        "{halted}"
    );
    let mut w = world(json!([
        hero([0.0; 3]),
        camera("cam", json!({"target":"hero"}))
    ]));
    w.capabilities.insert("save".into(), 1);
    let error = format!("{:#}", runtime(w).checkpoint().unwrap_err());
    assert!(
        error.contains("camera.rig does not implement checkpoints"),
        "{error}"
    );
}

/// Static blocker shapes of a world (as the rig collects them).
fn blockers(w: &World) -> Vec<axiom_core::camera::Blocker> {
    let registry = installed_registry().authoring;
    let c = w.compile(&registry).unwrap();
    axiom_core::camera::collect_blockers(&c.entities, &c.parents, &registry.movers).unwrap()
}

#[test]
fn the_deprecated_tag_still_blocks_is_reported_and_migrates_to_the_component() {
    let (mut entities, _) = room();
    entities.push(hero([0.0; 3]));
    entities.push(camera("cam", json!({"target":"hero"})));
    let typed = world(Value::Array(entities));
    let mut legacy = typed.clone();
    let east = legacy.entity_mut("room/east").unwrap();
    east.components.remove("camera.blocker");
    east.tags = vec!["camera_blocker".into()];
    // Same shapes either way (5 walls; the east mesh is one part).
    let shapes = blockers(&typed);
    assert_eq!(shapes.len(), 5);
    assert_eq!(blockers(&legacy), shapes);
    // A mesh without either blocks nothing.
    let mut plain = typed.clone();
    plain
        .entity_mut("room/east")
        .unwrap()
        .components
        .remove("camera.blocker");
    assert_eq!(blockers(&plain).len(), 4);
    // verify lists the deprecated tag; the typed world has no diagnostics.
    let verify = |w: &World| {
        let mut store =
            axiom_core::Store::memory_with(installed_registry().authoring, w.clone()).unwrap();
        store.execute(axiom_core::Request::Verify {}).unwrap()["diagnostics"].clone()
    };
    let d = verify(&legacy);
    assert_eq!(d.as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d[0]["code"], "deprecated_tag");
    assert!(d[0]["path"].as_str().unwrap().contains("room~1east"), "{d}");
    assert_eq!(verify(&typed), json!([]));
    // `migrate camera_blocker` (= `axiom migrate`) gives the typed world back.
    let mut store =
        axiom_core::Store::memory_with(installed_registry().authoring, legacy.clone()).unwrap();
    store
        .execute(axiom_core::Request::Acl {
            text: "migrate camera_blocker".into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .unwrap();
    let migrated = store.current().unwrap().1;
    assert_eq!(migrated, typed);
    assert_eq!(verify(&migrated), json!([]));
}
