use axiom_core::{
    gameplay::RuntimeCommand, local::Target, ContentPackage, CoreEdit, Edit, Request, Store, World,
};
use axiom_runtime::{action::ActionSystem, installed_registry, scenario, ActionMap, WorldRuntime};
use serde_json::json;
use std::collections::BTreeMap;

fn source() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/action"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}
fn idle() -> World {
    let mut w = source();
    w.prefabs
        .get_mut("sentinel")
        .unwrap()
        .components
        .get_mut("action.actor")
        .unwrap()["ai"] = json!(false);
    w
}
fn command(r: &mut WorldRuntime, name: &str) {
    r.command(&RuntimeCommand::Call {
        capability: "action".into(),
        action: name.into(),
        args: json!({}),
    })
    .unwrap();
}
fn steps(r: &mut WorldRuntime, ticks: u32) {
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
}
#[test]
fn normal_package_runs_combo_dodge_enemy_and_actual_imported_clips() {
    let scenario = serde_json::from_str(include_str!(
        "../../../examples/action-functional.scenario.json"
    ))
    .unwrap();
    let result = scenario::run(source(), 1, installed_registry(), scenario).unwrap();
    assert_eq!(result["ok"], true);
}
#[test]
fn native_input_edges_move_follow_camera_and_drive_synced_animation_and_audio() {
    let mut r = WorldRuntime::new(idle(), 1, installed_registry()).unwrap();
    let before = r.state.transforms[r.state.compiled.indices["camera"]].clone();
    let input = ActionMap(BTreeMap::from([("move_x".into(), 1.0)]));
    for _ in 0..20 {
        r.step(&input);
    }
    let hero = r.state.compiled.indices["hero"];
    assert!(r.state.transforms[hero].as_ref().unwrap().position[0] > 0.5);
    assert_ne!(
        r.state.transforms[r.state.compiled.indices["camera"]],
        before
    );
    let attack = ActionMap([("attack".into(), 1.0)].into());
    r.step(&attack);
    assert_eq!(
        r.observation().metrics["animation"]["players"]["hero"]["clip"],
        "slash1"
    );
    let mut source = r.state.compiled.source.clone();
    source.capabilities.insert("save".into(), 1);
    r.reconcile(source, 3).unwrap();
    assert!(r.checkpoint().is_err());
    for _ in 0..75 {
        r.step(&attack);
    }
    assert_eq!(r.observation().metrics["action"]["stats"]["attacks"], 1);
    assert!(
        r.observation().metrics["audio"]["emitted"]
            .as_u64()
            .unwrap()
            > 0
    );
    let frame = r.render_frame();
    assert_eq!(frame.meshes.len(), 4);
    r.assert_valid().unwrap();
}
#[test]
fn unrelated_reload_preserves_timeline_and_sequence_change_restarts_it() {
    let mut r = WorldRuntime::new(idle(), 1, installed_registry()).unwrap();
    command(&mut r, "attack");
    steps(&mut r, 20);
    let mut w = r.state.compiled.source.clone();
    w.title = "Edited label".into();
    r.reconcile(w, 2).unwrap();
    steps(&mut r, 1);
    assert_eq!(r.system::<ActionSystem>().unwrap().sim.tick, r.state.tick);
    assert_eq!(r.observation().metrics["action"]["hero"]["elapsed"], 21);
    assert_eq!(
        r.observation().metrics["animation"]["players"]["hero"]["clip"],
        "slash1"
    );
    let mut w = r.state.compiled.source.clone();
    w.resources.get_mut("action.abilities").unwrap()["abilities"]["slash2"]["cost"] = json!(14);
    r.reconcile(w, 3).unwrap();
    assert_eq!(
        r.observation().metrics["action"]["hero"]["ability"],
        serde_json::Value::Null
    );
    assert_eq!(r.observation().metrics["action"]["stats"]["attacks"], 0);
    steps(&mut r, 1);
    assert_eq!(
        r.observation().metrics["animation"]["players"]["hero"]["clip"],
        "idle"
    );
}
#[test]
fn timeline_preview_is_atomic_and_invalid_clip_activation_retains_the_previous_world() {
    let registry = installed_registry();
    let w = idle();
    let mut store = Store::memory_with(registry.authoring.clone(), w.clone()).unwrap();
    let mut r = WorldRuntime::new(w, 1, registry).unwrap();
    let field = |field: &str, value| {
        Edit::Core(CoreEdit::SetField {
            target: Target::Resource {
                id: "action.abilities".into(),
            },
            field: field.into(),
            value,
        })
    };
    let err = store
        .execute(Request::Preview {
            expected_rev: Some(1),
            edits: vec![field(
                "/abilities/slash2/dodge_window",
                json!({"start":50,"end":90}),
            )],
        })
        .unwrap_err();
    assert_eq!(
        axiom_core::error_response(&err)["diagnostics"][0]["path"],
        "/resources/action.abilities/abilities/slash2/dodge_window"
    );
    assert_eq!(store.revision().unwrap(), 1);
    store
        .apply(
            Some(1),
            vec![field("/abilities/slash2/clip", json!("missing"))],
        )
        .unwrap();
    let (rev, w) = store.current().unwrap();
    assert!(r.reconcile(w, rev).is_err());
    assert_eq!(r.rev, 1);
    assert_eq!(r.requested_rev, 2);
    r.assert_valid().unwrap();
}
#[test]
fn replacing_asset_and_fixed_frame_batches_preserve_simulation_results() {
    let w = idle();
    let mut a = WorldRuntime::new(w.clone(), 1, installed_registry()).unwrap();
    let mut b = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    for frame in 0..120 {
        if frame == 0 {
            command(&mut a, "attack");
            command(&mut b, "attack");
        }
        steps(&mut a, 2);
        // A slower presentation can batch four fixed steps.
        if frame % 2 == 1 {
            steps(&mut b, 4);
        }
    }
    assert_eq!(
        a.observation().metrics["action"],
        b.observation().metrics["action"]
    );
    let mut w = a.state.compiled.source.clone();
    let lib = w.resources.get_mut("animation.library").unwrap();
    lib["assets"]["alternate"] = lib["assets"]["sentinel"].clone();
    w.entity_mut("hero")
        .unwrap()
        .set(
            "animation.player",
            json!({"asset":"alternate","clip":"idle"}),
        )
        .unwrap();
    a.reconcile(w, 2).unwrap();
    command(&mut a, "dodge");
    steps(&mut a, 2);
    assert_eq!(
        a.observation().metrics["animation"]["players"]["hero"]["clip"],
        "dodge"
    );
}
