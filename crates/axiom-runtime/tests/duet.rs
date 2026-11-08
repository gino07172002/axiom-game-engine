use axiom_core::{gameplay::RuntimeCommand, ContentPackage, CoreEdit, Edit, Request, Store, World};
use axiom_runtime::{installed_registry, scenario, ActionMap, WorldRuntime};
use serde_json::{json, Value};

fn world() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/love-no-war"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn call(runtime: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    runtime.command(&RuntimeCommand::Call {
        capability: "rhythm".into(),
        action: action.into(),
        args,
    })
}

#[test]
fn playable_package_scores_boundary_taps_and_plays_both_imported_finales() {
    let scenario =
        serde_json::from_str(include_str!("../../../examples/love-no-war.scenario.json")).unwrap();
    let result = scenario::run(world(), 1, installed_registry(), scenario).unwrap();
    assert_eq!(result["ok"], true);
}

#[test]
fn refusal_and_stop_prevent_dance_and_do_not_score() {
    let mut source = world();
    source.resources.get_mut("rhythm.sequence").unwrap()["accepts_invite"] = json!(false);
    let mut runtime = WorldRuntime::new(source, 1, installed_registry()).unwrap();
    assert!(call(&mut runtime, "begin", json!({})).is_err());
    call(&mut runtime, "invite", json!({})).unwrap();
    assert_eq!(runtime.observation().metrics["rhythm"]["phase"], "declined");
    assert!(call(&mut runtime, "begin", json!({})).is_err());
    assert_eq!(
        call(&mut runtime, "press", json!({"input":"sway"})).unwrap()["result"]["hit"],
        false
    );

    runtime.reconcile(world(), 2).unwrap();
    call(&mut runtime, "invite", json!({})).unwrap();
    call(&mut runtime, "begin", json!({})).unwrap();
    call(&mut runtime, "stop", json!({})).unwrap();
    for _ in 0..320 {
        runtime.step(&ActionMap::default());
    }
    let metrics = runtime.observation().metrics;
    assert_eq!(metrics["rhythm"]["phase"], "stopped");
    assert_eq!(metrics["rhythm"]["hits"], 0);
    assert_eq!(
        metrics["animation"]["players"]["cast/valentina"]["clip"],
        "idle"
    );
}

#[test]
fn held_button_does_not_score_multiple_beats_and_misses_allow_replay() {
    let mut runtime = WorldRuntime::new(world(), 1, installed_registry()).unwrap();
    call(&mut runtime, "invite", json!({})).unwrap();
    call(&mut runtime, "begin", json!({})).unwrap();
    for _ in 0..251 {
        runtime.step(&ActionMap::default());
    }
    let hold = ActionMap([("sway".into(), 1.0)].into());
    for _ in 0..800 {
        runtime.step(&hold);
    }
    let observed = &runtime.observation().metrics["rhythm"];
    assert_eq!(observed["phase"], "retry");
    assert_eq!(observed["hits"], 1);
    assert_eq!(observed["misses"], 7);
    call(&mut runtime, "invite", json!({})).unwrap();
    assert_eq!(runtime.observation().metrics["rhythm"]["phase"], "ready");
}

#[test]
fn ai_edits_are_revision_checked_and_missing_clips_retain_last_good_revision() {
    let registry = installed_registry();
    let source = world();
    let mut store = Store::memory_with(registry.authoring.clone(), source.clone()).unwrap();
    let mut runtime = WorldRuntime::new(source, 1, registry).unwrap();
    let field = |path: &str, value| {
        Edit::Core(CoreEdit::SetField {
            target: axiom_core::local::Target::Resource {
                id: "rhythm.sequence".into(),
            },
            field: path.into(),
            value,
        })
    };
    let error = store
        .execute(Request::Preview {
            expected_rev: Some(1),
            edits: vec![field("/window_ticks", json!(45))],
        })
        .unwrap_err();
    assert_eq!(
        axiom_core::error_response(&error)["diagnostics"][0]["path"],
        "/resources/rhythm.sequence/window_ticks"
    );
    assert_eq!(store.revision().unwrap(), 1);
    store
        .apply(Some(1), vec![field("/window_ticks", json!(20))])
        .unwrap();
    let (rev, world) = store.current().unwrap();
    runtime.reconcile(world, rev).unwrap();
    assert_eq!(runtime.observation().metrics["rhythm"]["window_ticks"], 20);
    store
        .apply(
            Some(2),
            vec![field("/moves/velvet/clips/0", json!("unknown"))],
        )
        .unwrap();
    let (rev, world) = store.current().unwrap();
    assert!(runtime.reconcile(world, rev).is_err());
    assert_eq!(runtime.rev, 2);
    assert_eq!(runtime.requested_rev, 3);
    assert_eq!(
        runtime.activation_error.unwrap()["diagnostics"][0]["code"],
        "rhythm_clip"
    );
}
