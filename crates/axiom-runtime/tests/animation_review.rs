//! Regression tests for the 2026-11 animation review (R4): hot reload of runtime playback,
//! authoring validation of `animation.player.clip`, batch pre-checks, `play` vs speed ramps,
//! short-clip seeks, compact `inspect` output, the generated `discover animation` topic and
//! reuse of skinned vertices. Uses `content/animator-lab` and `content/character-lab`.
use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::world::{ActionMap, WorldRuntime};
use serde_json::{json, Value};
use std::sync::Arc;

fn load(name: &str) -> World {
    ContentPackage::load(
        format!("{}/../../content/{name}", env!("CARGO_MANIFEST_DIR")),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// `animator-lab` with a plain (controller-free, standing) walker and a library clip `lean`.
fn plain() -> World {
    let mut w = load("animator-lab");
    let walker = w.entities.iter_mut().find(|e| e.id == "walker").unwrap();
    walker.components.remove("path.follow");
    walker.components.remove("animation.controller");
    w.resources.get_mut("animation.library").unwrap()["clips"] =
        json!({"lean":{"id":"lean","duration":2.0,"tracks":[]}});
    w
}

fn player_mut<'a>(w: &'a mut World, id: &str) -> &'a mut Value {
    w.entities
        .iter_mut()
        .find(|e| e.id == id)
        .unwrap()
        .components
        .get_mut("animation.player")
        .unwrap()
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
    .map(|v| v["result"].clone())
    .map_err(|e| format!("{e:#}"))
}

fn player(r: &WorldRuntime, id: &str) -> Value {
    r.observation().metrics["animation"]["players"][id].clone()
}

#[test]
fn a_revision_removing_the_clip_playing_at_runtime_still_activates() {
    let mut r = runtime(plain());
    call(&mut r, "play", json!({"entity":"walker","clip":"lean"})).unwrap();
    run(&mut r, 30);
    assert_eq!(player(&r, "walker")["clip"], "lean");
    // The next revision drops the library clip the AI started: it activates and the player
    // starts again as authored (idle from 0), instead of refusing with "clip not found".
    let mut w = plain();
    w.resources.get_mut("animation.library").unwrap()["clips"] = json!({});
    r.reconcile(w, 2)
        .expect("revision without the runtime clip activates");
    let p = player(&r, "walker");
    assert_eq!(p["clip"], "idle", "{p}");
    assert_eq!(p["time"], 0.0, "{p}");
    run(&mut r, 2);
    r.assert_valid().unwrap();
}

#[test]
fn an_authored_clip_switch_starts_the_new_clip_from_its_head() {
    // Review R4 N2: switching the authored clip while the old one still exists must not carry
    // the old clip's time into the new one.
    let authored = |clip: &str| {
        let mut w = plain();
        w.resources.get_mut("animation.library").unwrap()["clips"] = json!({
            "long":{"id":"long","duration":10.0,"tracks":[]},
            "short":{"id":"short","duration":1.0,"tracks":[]}
        });
        let p = player_mut(&mut w, "walker");
        p["clip"] = json!(clip);
        p["looping"] = json!(false);
        w
    };
    let mut r = runtime(authored("long"));
    run(&mut r, 360);
    let before = player(&r, "walker");
    assert_eq!(before["clip"], "long", "{before}");
    assert!(before["time"].as_f64().unwrap() > 2.5, "{before}");
    r.reconcile(authored("short"), 2).unwrap();
    let after = player(&r, "walker");
    assert_eq!(after["clip"], "short", "{after}");
    assert_eq!(after["time"], 0.0, "{after}");
    run(&mut r, 60);
    let t = player(&r, "walker")["time"].as_f64().unwrap();
    assert!(t > 0.1 && t < 0.9, "plays from the head: {t}");
    // An unrelated edit still continues the running playback.
    let mut moved = authored("short");
    moved
        .entities
        .iter_mut()
        .find(|e| e.id == "floor")
        .unwrap()
        .components
        .get_mut("core.transform")
        .unwrap()["position"] = json!([0, -0.1, 0]);
    r.reconcile(moved, 3).unwrap();
    assert_eq!(player(&r, "walker")["time"].as_f64().unwrap(), t);
}

#[test]
fn authoring_the_clip_already_playing_at_runtime_keeps_its_playback() {
    // Review R4 N2 (round 3): the reset follows the clip that actually plays. Authoring the
    // clip `animation.play` already started keeps its time (and passed markers).
    let authored = |clip: &str| {
        let mut w = plain();
        w.resources.get_mut("animation.library").unwrap()["clips"] = json!({
            "long":{"id":"long","duration":10.0,"tracks":[]},
            "short":{"id":"short","duration":1.0,"tracks":[]}
        });
        let p = player_mut(&mut w, "walker");
        p["clip"] = json!(clip);
        p["looping"] = json!(false);
        w
    };
    let mut r = runtime(authored("short"));
    call(
        &mut r,
        "play",
        json!({"entity":"walker","clip":"long","looping":false}),
    )
    .unwrap();
    run(&mut r, 180);
    let before = player(&r, "walker");
    assert_eq!(before["clip"], "long", "{before}");
    let time = before["time"].as_f64().unwrap();
    assert!(time > 1.0, "{before}");
    r.reconcile(authored("long"), 2).unwrap();
    let after = player(&r, "walker");
    assert_eq!(after["clip"], "long", "{after}");
    assert_eq!(after["time"].as_f64().unwrap(), time, "{after}");
}

#[test]
fn inspect_and_observe_report_the_controller_clip_and_time() {
    // Review R4 N3: with a controller, the top-level clip / time are the controller state's
    // primary clip, as the gameplay bindings report, not the clip-playback fields.
    let mut r = runtime(load("animator-lab"));
    run(&mut r, 90);
    let inspected = call(&mut r, "inspect", json!({"entity":"walker"})).unwrap();
    assert_eq!(inspected["controller"]["state"], "move", "{inspected}");
    assert_eq!(inspected["clip"], "walk", "{inspected}");
    let time = inspected["time"].as_f64().unwrap();
    assert!(time > 0.0, "{inspected}");
    let observed = player(&r, "walker");
    assert_eq!(observed["clip"], "walk", "{observed}");
    assert_eq!(observed["time"].as_f64().unwrap(), time, "{observed}");
}

#[test]
fn a_misspelled_player_clip_fails_validation_not_activation() {
    let registry = axiom_runtime::installed_registry();
    let mut w = plain();
    player_mut(&mut w, "walker")["clip"] = json!("idel");
    // The same world compile preview and apply run.
    let e = w.compile(&registry.authoring).unwrap_err();
    let d = e
        .downcast_ref::<axiom_core::Diagnostic>()
        .unwrap_or_else(|| panic!("not a diagnostic: {e:#}"));
    assert_eq!(d.code, "unknown_clip", "{e:#}");
    assert_eq!(
        d.path, "/entities/@walker/components/animation.player/clip",
        "{e:#}"
    );
    assert!(
        d.message.contains("idle") && d.message.contains("lean"),
        "{e:#}"
    );
    // Asset clips, library clips and no clip all validate.
    for clip in [json!("walk"), json!("lean"), Value::Null] {
        let mut w = plain();
        player_mut(&mut w, "walker")["clip"] = clip;
        w.compile(&registry.authoring).unwrap();
    }
}

#[test]
fn an_edited_playing_flag_applies_on_hot_reload_and_undo() {
    let mut r = runtime(plain());
    run(&mut r, 10);
    assert_eq!(player(&r, "walker")["playing"], true);
    let mut paused = plain();
    player_mut(&mut paused, "walker")["playing"] = json!(false);
    r.reconcile(paused, 2).unwrap();
    assert_eq!(player(&r, "walker")["playing"], false);
    let held = player(&r, "walker")["time"].clone();
    run(&mut r, 10);
    assert_eq!(player(&r, "walker")["time"], held, "paused by the edit");
    // Undo restores the authored true.
    r.reconcile(plain(), 3).unwrap();
    assert_eq!(player(&r, "walker")["playing"], true);
    // An unrelated edit keeps a runtime pause (playing unchanged in the authored data).
    call(
        &mut r,
        "seek",
        json!({"entity":"walker","seconds":0.2,"paused":true}),
    )
    .unwrap();
    let mut moved = plain();
    moved
        .entities
        .iter_mut()
        .find(|e| e.id == "floor")
        .unwrap()
        .components
        .get_mut("core.transform")
        .unwrap()["position"] = json!([0, -0.1, 0]);
    r.reconcile(moved, 4).unwrap();
    assert_eq!(player(&r, "walker")["playing"], false);
}

#[test]
fn play_drops_the_speed_ramp_of_the_previous_playback() {
    let mut r = runtime(plain());
    call(
        &mut r,
        "speed",
        json!({"entity":"walker","speed":3.0,"ramp_seconds":2.0}),
    )
    .unwrap();
    run(&mut r, 10);
    let reply = call(
        &mut r,
        "play",
        json!({"entity":"walker","clip":"walk","speed":0.5}),
    )
    .unwrap();
    assert_eq!(reply["ramp"], Value::Null, "{reply}");
    run(&mut r, 10);
    let speed = r.command(&RuntimeCommand::Call {
        capability: "animation".into(),
        action: "inspect".into(),
        args: json!({"entity":"walker"}),
    });
    let speed = speed.unwrap()["result"]["speed"].as_f64().unwrap();
    assert!((speed - 0.5).abs() < 1e-6, "{speed}");
}

#[test]
fn a_batch_with_a_misspelled_inspect_bone_changes_nothing() {
    let mut r = runtime(plain());
    run(&mut r, 3);
    let before = player(&r, "walker");
    let call = |action: &str, args: Value| RuntimeCommand::Call {
        capability: "animation".into(),
        action: action.into(),
        args,
    };
    let e = r
        .command(&RuntimeCommand::Batch {
            commands: vec![
                call("play", json!({"entity":"walker","clip":"lean"})),
                call(
                    "inspect",
                    json!({"entity":"walker","bones":["no-such-bone"]}),
                ),
            ],
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("no-such-bone"), "{e:#}");
    assert_eq!(player(&r, "walker"), before, "the play was not applied");
}

#[test]
fn seeking_far_into_a_short_loop_keeps_stepping() {
    let mut w = plain();
    w.resources.get_mut("animation.library").unwrap()["clips"] =
        json!({"blink":{"id":"blink","duration":0.00001,"tracks":[]}});
    w.resources.insert(
        "behavior.program".into(),
        json!({"variables":{"loops":0},"events":{"looped":{"fields":{}}},
            "rules":[{"id":"looped","event":"looped","actions":[{"kind":"add","variable":"loops","value":{"kind":"literal","value":1}}]}]}),
    );
    player_mut(&mut w, "walker")["on_end"] = json!("looped");
    let mut r = runtime(w);
    call(&mut r, "play", json!({"entity":"walker","clip":"blink"})).unwrap();
    run(&mut r, 2);
    // 86400 s over a 10 us loop: 8.64e9 cycles, past u32::MAX.
    let reply = call(&mut r, "seek", json!({"entity":"walker","seconds":86400.0})).unwrap();
    let t = reply["time"].as_f64().unwrap();
    assert!((0.0..0.00001).contains(&t), "seek wraps into the loop: {t}");
    run(&mut r, 5);
    r.assert_valid().unwrap();
    let loops = r.observation().metrics["gameplay"]["variables"]["loops"]
        .as_f64()
        .unwrap();
    // At most 8 completed cycles are reported per step.
    assert!(loops > 0.0 && loops <= 7.0 * 9.0, "{loops}");
}

#[test]
fn inspect_returns_a_part_count_unless_parts_are_asked_for() {
    let mut r = runtime(load("character-lab"));
    run(&mut r, 1);
    let summary = call(&mut r, "inspect", json!({"entity":"face2"})).unwrap();
    assert!(summary.get("parts").is_none(), "{summary}");
    let full = call(&mut r, "inspect", json!({"entity":"face2","parts":true})).unwrap();
    let parts = full["parts"].as_array().unwrap().len();
    assert!(parts >= 3, "{full}");
    assert_eq!(summary["part_count"], parts, "{summary}");
}

#[test]
fn discover_animation_lists_every_registered_action_and_format() {
    let registry = axiom_runtime::installed_registry();
    let topic = &registry.authoring.topics["animation"];
    let listed: Vec<&str> = topic["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for action in registry.authoring.actions.keys() {
        if let Some(a) = action.strip_prefix("animation.") {
            assert!(listed.contains(&a), "{a} missing from {listed:?}");
        }
    }
    for a in ["stage", "pick", "play", "seek", "inspect"] {
        assert!(listed.contains(&a), "{a} missing from {listed:?}");
    }
    let formats = topic["formats"].to_string();
    for ext in [".pmx", ".gltf", ".glb", ".axrig"] {
        assert!(formats.contains(ext), "{formats}");
    }
    assert!(topic["controller"].as_str().unwrap().contains("blend2d"));
}

#[test]
fn an_unchanged_pose_reuses_the_skinned_vertices() {
    let mut r = runtime(load("character-lab"));
    run(&mut r, 2);
    call(
        &mut r,
        "seek",
        json!({"entity":"face","seconds":0.3,"paused":true}),
    )
    .unwrap();
    run(&mut r, 1);
    let vertices = |r: &WorldRuntime| {
        r.render_frame()
            .meshes
            .iter()
            .find(|m| m.id == "face")
            .unwrap()
            .vertices
            .clone()
    };
    let a = vertices(&r);
    run(&mut r, 3);
    let b = vertices(&r);
    assert!(Arc::ptr_eq(&a, &b), "paused: the same skinned snapshot");
    // Review R4 F8 (round 2): a committed command that leaves pose and weights unchanged
    // (a visibility round trip, a seek to the same time) keeps the snapshot.
    let parts = call(&mut r, "inspect", json!({"entity":"face","parts":true})).unwrap();
    let part = parts["parts"][0]["id"].as_str().unwrap().to_owned();
    for visible in [false, true] {
        call(
            &mut r,
            "visibility",
            json!({"entity":"face","group":part,"visible":visible}),
        )
        .unwrap();
    }
    assert!(
        Arc::ptr_eq(&b, &vertices(&r)),
        "visibility kept the snapshot"
    );
    call(
        &mut r,
        "seek",
        json!({"entity":"face","seconds":0.3,"paused":true}),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&b, &vertices(&r)), "an equal seek kept it");
    // A morph weight change re-skins.
    call(
        &mut r,
        "morph",
        json!({"entity":"face","target":"puff","weight":1.0}),
    )
    .unwrap();
    let c = vertices(&r);
    assert!(!Arc::ptr_eq(&b, &c));
    assert!(
        b.iter()
            .zip(c.iter())
            .any(|(x, y)| x.position != y.position),
        "the new weight is drawn"
    );
}
