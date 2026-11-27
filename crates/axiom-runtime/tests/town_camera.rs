//! Axiom Town hero: faces where it runs, and the mouse-wheel `zoom` axis moves the follow
//! camera in to a first-person view at the hero's eyes and back out.
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use glam::{Quat, Vec3};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn pack(name: &str) -> World {
    ContentPackage::load(
        format!("{}/../../content/{name}", env!("CARGO_MANIFEST_DIR")),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}
fn input(pairs: &[(&str, f32)]) -> ActionMap {
    ActionMap(
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), *v))
            .collect::<BTreeMap<_, _>>(),
    )
}
fn transform(r: &WorldRuntime, id: &str) -> axiom_core::Transform {
    r.state.transforms[r.state.compiled.indices[id]]
        .clone()
        .unwrap()
}
fn camera(r: &WorldRuntime) -> Value {
    r.observation().metrics["action"]["camera"].clone()
}
fn run(r: &mut WorldRuntime, map: &ActionMap, ticks: u32) {
    for _ in 0..ticks {
        r.step(map);
    }
}
fn hero_hidden_in_frame(r: &WorldRuntime) -> bool {
    let frame = r.render_frame();
    let mesh = frame
        .meshes
        .iter()
        .find(|m| m.id == "hero")
        .expect("hero mesh");
    mesh.hidden == [u64::MAX; 4]
}

#[test]
fn the_town_hero_faces_the_way_it_runs() {
    // axiom-short.glb faces +Z (its eye meshes sit at z = +0.10..0.14, like glTF models in
    // general); the action simulation's yaw 0 faces -Z, so the actor needs model_yaw = pi.
    // With model_yaw 0 the hero ran backwards (dot = -1).
    for (axis, sign) in [
        ("move_z", 1.0),
        ("move_z", -1.0),
        ("move_x", 1.0),
        ("move_x", -1.0),
    ] {
        let mut r = WorldRuntime::new(pack("axiom-town"), 1, installed_registry()).unwrap();
        let start = Vec3::from_array(transform(&r, "hero").position);
        run(&mut r, &input(&[(axis, sign)]), 60);
        let t = transform(&r, "hero");
        let moved = (Vec3::from_array(t.position) - start).normalize();
        let model_front = Quat::from_array(t.rotation) * Vec3::Z;
        assert!(
            model_front.dot(moved) > 0.99,
            "{axis} {sign}: the model looks along {model_front} while running along {moved}"
        );
    }
}

#[test]
fn wheel_zoom_ends_in_first_person_at_the_eyes_and_returns() {
    let mut r = WorldRuntime::new(pack("axiom-town"), 1, installed_registry()).unwrap();
    run(&mut r, &ActionMap::default(), 2);
    let third = transform(&r, "camera");
    assert_eq!(camera(&r)["distance"], 6.5, "{}", camera(&r));
    assert!(!hero_hidden_in_frame(&r));

    // One notch per fixed step (the host feeds WheelUp as zoom = -1 for one step).
    let zoom_in = input(&[("zoom", -1.0)]);
    run(&mut r, &zoom_in, 3);
    let after_three = camera(&r);
    assert!(
        after_three["target_distance"].as_f64().unwrap() < 3.4,
        "{after_three}"
    );
    // Partway: still third person, closer than authored, the hero is drawn.
    run(&mut r, &ActionMap::default(), 240);
    let close = camera(&r);
    assert_eq!(close["first_person"], 0.0, "{close}");
    assert!(!hero_hidden_in_frame(&r));
    let eye = Vec3::from_array(transform(&r, "camera").position);
    let hero = Vec3::from_array(transform(&r, "hero").position);
    assert!(eye.distance(hero) < Vec3::from_array(third.position).distance(hero));

    // Past the closest third-person distance: first person at the authored eye height,
    // just in front of the face, looking along the camera yaw, hero mesh not drawn.
    run(&mut r, &zoom_in, 6);
    run(&mut r, &ActionMap::default(), 400);
    let fp = camera(&r);
    assert_eq!(fp["distance"], 0.0, "{fp}");
    assert_eq!(fp["first_person"], 1.0, "{fp}");
    assert_eq!(fp["hero_hidden"], true, "{fp}");
    assert!(hero_hidden_in_frame(&r));
    let t = transform(&r, "camera");
    let hero = Vec3::from_array(transform(&r, "hero").position);
    let eye = Vec3::from_array(t.position);
    assert!((eye.y - 1.15).abs() < 1e-4, "eye {eye}");
    // camera_yaw 0: the view looks along -Z, the eye 0.25 m ahead of the hero's axis.
    assert!(
        (eye - Vec3::new(hero.x, 1.15, hero.z - 0.25)).length() < 1e-4,
        "eye {eye}"
    );
    let look = Quat::from_array(t.rotation) * -Vec3::Z;
    assert!(look.dot(-Vec3::Z) > 0.9999, "look {look}");

    // Moving forward in first person runs along the view.
    let before = hero;
    run(&mut r, &input(&[("move_z", 1.0)]), 30);
    let walked = Vec3::from_array(transform(&r, "hero").position) - before;
    assert!(walked.normalize().dot(-Vec3::Z) > 0.99, "{walked}");

    // One notch out returns to the closest third-person view, the hero drawn again.
    run(&mut r, &input(&[("zoom", 1.0)]), 1);
    run(&mut r, &ActionMap::default(), 400);
    let back = camera(&r);
    assert_eq!(back["first_person"], 0.0, "{back}");
    assert_eq!(back["hero_hidden"], false, "{back}");
    assert!(!hero_hidden_in_frame(&r));
    // All the way out: the authored follow camera again.
    run(&mut r, &input(&[("zoom", 1.0)]), 20);
    run(&mut r, &ActionMap::default(), 400);
    let out = camera(&r);
    assert_eq!(out["distance"], 6.5, "{out}");
    assert_eq!(out["first_person"], 0.0, "{out}");
}

#[test]
fn without_first_person_the_zoom_stops_at_the_closest_third_person_view() {
    let mut w = pack("action");
    w.resources.get_mut("input.bindings").unwrap()["axes"]["zoom"] =
        json!({"positive":"WheelDown","negative":"WheelUp"});
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    run(&mut r, &input(&[("zoom", -1.0)]), 30);
    run(&mut r, &ActionMap::default(), 400);
    let c = camera(&r);
    assert!((c["distance"].as_f64().unwrap() - 1.6).abs() < 1e-3, "{c}");
    assert_eq!(c["first_person"], 0.0, "{c}");
    assert_eq!(c["hero_hidden"], false, "{c}");
}

#[test]
fn a_world_that_never_zooms_keeps_its_observation() {
    // No `first_person`, no zoom input: no `camera` metrics (recorded state hashes stay).
    let mut r = WorldRuntime::new(pack("action"), 1, installed_registry()).unwrap();
    run(&mut r, &ActionMap::default(), 10);
    assert!(r.observation().metrics["action"].get("camera").is_none());
}

#[test]
fn first_person_values_out_of_range_are_refused() {
    let registry = installed_registry();
    for bad in [
        json!({"eye_height":10.0}),
        json!({"eye_height":1.0,"eye_forward":-0.5}),
        json!({"eye_height":1.0,"extra":1}),
    ] {
        let mut w = pack("axiom-town");
        w.resources.get_mut("action.session").unwrap()["first_person"] = bad.clone();
        let e = format!("{:#}", w.compile(&registry.authoring).unwrap_err());
        assert!(
            e.contains("first_person") || e.contains("unknown field"),
            "{bad}: {e}"
        );
    }
}

#[test]
fn the_zoom_command_reaches_the_same_first_person_view_as_the_wheel() {
    use axiom_core::gameplay::RuntimeCommand;
    let zoom = |r: &mut WorldRuntime, args: Value| {
        r.command(&RuntimeCommand::Call {
            capability: "action".into(),
            action: "zoom".into(),
            args,
        })
    };
    let mut wheel = WorldRuntime::new(pack("axiom-town"), 1, installed_registry()).unwrap();
    run(&mut wheel, &input(&[("zoom", -1.0)]), 12);
    run(&mut wheel, &ActionMap::default(), 400);
    let mut agent = WorldRuntime::new(pack("axiom-town"), 1, installed_registry()).unwrap();
    let reply = zoom(&mut agent, json!({"distance":0.0})).unwrap();
    assert_eq!(reply["result"]["target_distance"], 0.0, "{reply}");
    run(&mut agent, &ActionMap::default(), 412);
    assert_eq!(camera(&agent)["first_person"], 1.0, "{}", camera(&agent));
    assert_eq!(transform(&agent, "camera"), transform(&wheel, "camera"));
    // Bad arguments are refused before anything changes.
    for bad in [
        json!({}),
        json!({"distance":-1.0}),
        json!({"distance":1.0,"x":1}),
    ] {
        assert!(zoom(&mut agent, bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(camera(&agent)["target_distance"], 0.0);
}

#[test]
fn camera_tuning_changes_the_wheel_step_and_the_closest_distance() {
    // One wheel notch: the default step is 0.8 of the distance, camera_tuning sets it.
    let notch = |tuning: Option<Value>| {
        let mut w = pack("axiom-town");
        let session = w.resources.get_mut("action.session").unwrap();
        if let Some(t) = tuning {
            session["camera_tuning"] = t;
        }
        let full = session["camera_distance"].as_f64().unwrap();
        let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
        run(&mut r, &input(&[("zoom", -1.0)]), 1);
        camera(&r)["target_distance"].as_f64().unwrap() / full
    };
    assert!((notch(None) - 0.8).abs() < 1e-5);
    assert!((notch(Some(json!({"zoom_step":0.5}))) - 0.5).abs() < 1e-5);
    // Without first person, zoom stops at first_person_from.
    let mut w = pack("action");
    w.resources.get_mut("input.bindings").unwrap()["axes"]["zoom"] =
        json!({"positive":"WheelDown","negative":"WheelUp"});
    w.resources.get_mut("action.session").unwrap()["camera_tuning"] =
        json!({"first_person_from":3.0,"zoom_ease":0.5});
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    run(&mut r, &input(&[("zoom", -1.0)]), 30);
    run(&mut r, &ActionMap::default(), 60);
    let c = camera(&r);
    assert!((c["distance"].as_f64().unwrap() - 3.0).abs() < 1e-3, "{c}");
    // Out-of-range values are refused with the field named.
    for (bad, needle) in [
        (json!({"zoom_step":0.99}), "zoom_step"),
        (
            json!({"first_person_from":0.4,"first_person_at":0.5}),
            "first_person_at",
        ),
        (json!({"orbit_speed":0.0}), "orbit_speed"),
        (json!({"spin":1}), "spin"),
    ] {
        let mut w = pack("axiom-town");
        w.resources.get_mut("action.session").unwrap()["camera_tuning"] = bad.clone();
        let msg = format!(
            "{:#}",
            WorldRuntime::new(w, 1, installed_registry())
                .err()
                .expect("refused")
        );
        assert!(msg.contains(needle), "{bad}: {msg}");
    }
}
