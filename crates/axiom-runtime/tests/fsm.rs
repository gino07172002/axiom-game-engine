use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::world::{ActionMap, WorldRuntime};
use serde_json::{json, Value};

fn lab() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/behavior-lab"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> Value {
    r.command(&RuntimeCommand::Call {
        capability: "fsm".into(),
        action: action.into(),
        args,
    })
    .unwrap()["result"]
        .clone()
}

fn run(r: &mut WorldRuntime, ticks: u32) {
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
}

#[test]
fn idle_enemy_explains_why_it_is_not_chasing_then_chases_when_in_range() {
    let mut r = WorldRuntime::new(lab(), 1, axiom_runtime::installed_registry()).unwrap();
    let before = call(&mut r, "inspect", json!({"id":"enemy/0"}));
    assert_eq!(before["state"], "idle");
    let chase = before["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["to"] == "chase")
        .unwrap();
    assert_eq!(chase["when"], "near(player)<=15");
    assert_eq!(chase["pass"], false);
    assert!(chase["measured"].as_f64().unwrap() > 29.0);
    // Player walks +x at 2 m/s from x=-20; enemy/0 sits at x=10 → in range after ~7.5 s.
    run(&mut r, 1200);
    let after = call(&mut r, "inspect", json!({"id":"enemy/0"}));
    assert_eq!(after["state"], "chase");
    assert_eq!(after["last"]["from"], "idle");
    let x = after["position"][0].as_f64().unwrap();
    assert!(
        x < 9.0,
        "a chasing enemy must have moved toward the player: x={x}"
    );
    run(&mut r, 1200);
    let summary = call(&mut r, "summary", json!({}));
    assert_eq!(summary["agents"], 6);
    assert_eq!(r.observation().metrics["fsm"]["agents"], 6);
}

#[test]
fn identical_runs_are_deterministic() {
    let snapshot = || {
        let mut r = WorldRuntime::new(lab(), 1, axiom_runtime::installed_registry()).unwrap();
        run(&mut r, 2000);
        (0..6)
            .map(|i| call(&mut r, "inspect", json!({"id":format!("enemy/{i}")})))
            .collect::<Vec<_>>()
    };
    assert_eq!(snapshot(), snapshot());
}

#[test]
fn hot_reload_keeps_agent_state_when_the_machine_survives() {
    let world = lab();
    let mut r = WorldRuntime::new(world.clone(), 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 1200);
    assert_eq!(
        call(&mut r, "inspect", json!({"id":"enemy/0"}))["state"],
        "chase"
    );
    let mut edited = world;
    edited.resources.get_mut("fsm.machines").unwrap()["machines"]["enemy.basic"]["states"]
        ["chase"]["action"]["speed"] = json!(4.0);
    r.reconcile(edited, 2).unwrap();
    let after = call(&mut r, "inspect", json!({"id":"enemy/0"}));
    assert_eq!(after["state"], "chase");
    assert_eq!(after["action"]["speed"], 4.0);
}

#[test]
fn invalid_machines_and_unknown_agents_fail_validation() {
    let registry = axiom_runtime::installed_registry();
    let mut bad = lab();
    bad.resources.get_mut("fsm.machines").unwrap()["machines"]["enemy.basic"]["transitions"][0]
        ["to"] = json!("ghost");
    let error = format!("{:#}", bad.compile(&registry.authoring).unwrap_err());
    assert!(error.contains("unknown state ghost"), "{error}");
    let mut orphan = lab();
    orphan
        .prefabs
        .get_mut("enemy.basic")
        .unwrap()
        .components
        .insert("fsm.agent".into(), json!({"machine":"missing"}));
    let error = WorldRuntime::new(orphan, 1, registry).err().unwrap();
    assert!(format!("{error:#}").contains("unknown fsm machine missing"));
}

#[test]
fn checkpoints_fail_explicitly_for_fsm_worlds() {
    let mut w = lab();
    w.capabilities.insert("save".into(), 1);
    let r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    let error = format!("{:#}", r.checkpoint().err().unwrap());
    assert!(
        error.contains("fsm.step does not implement checkpoints"),
        "{error}"
    );
}

#[test]
fn fps_inspect_reports_sentinel_state_and_transition_verdicts() {
    let w = axiom_fps::demo_world().unwrap();
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    let id = r.state.compiled.entities[1].id.clone();
    let out = r
        .command(&RuntimeCommand::Call {
            capability: "fps".into(),
            action: "inspect".into(),
            args: json!({ "id": id }),
        })
        .unwrap();
    let result = &out["result"];
    assert_eq!(result["id"], json!(id));
    assert!(result["distance_to_player"].as_f64().unwrap() > 0.0);
    assert!(result["transitions"].as_array().is_some());
    let missing = r.command(&RuntimeCommand::Call {
        capability: "fps".into(),
        action: "inspect".into(),
        args: json!({"id":"nobody"}),
    });
    assert!(format!("{:#}", missing.unwrap_err()).contains("not a sentinel"));
}
