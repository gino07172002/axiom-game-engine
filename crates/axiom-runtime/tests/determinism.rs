use axiom_core::{ContentPackage, World};
use axiom_runtime::{
    scenario::{self, Scenario},
    world::{ActionMap, WorldRuntime},
};
use serde_json::json;

fn lab() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/behavior-lab"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// behavior-lab plus a random idle → chase trigger, so the seed changes the outcome.
fn random_lab(seed: u64) -> World {
    let mut w = lab();
    w.resources
        .insert("core.rng".into(), json!({ "seed": seed }));
    let machine = &mut w.resources.get_mut("fsm.machines").unwrap()["machines"]["enemy.basic"];
    machine["transitions"].as_array_mut().unwrap().insert(
        1,
        json!({"from":"idle","to":"chase","when":{"kind":"chance","per_tick":0.002}}),
    );
    w
}

fn hash_after(world: World, ticks: u32) -> String {
    let mut r = WorldRuntime::new(world, 1, axiom_runtime::installed_registry()).unwrap();
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
    scenario::state_hash(&r).unwrap()
}

#[test]
fn seeded_stream_is_reproducible_and_seed_sensitive() {
    let draw = |seed: Option<u64>| {
        let mut w = World::empty("rng");
        if let Some(seed) = seed {
            w.resources
                .insert("core.rng".into(), json!({ "seed": seed }));
        }
        let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
        (0..8).map(|_| r.state.random_u64()).collect::<Vec<_>>()
    };
    assert_eq!(draw(Some(7)), draw(Some(7)));
    assert_ne!(draw(Some(7)), draw(Some(8)));
    assert_eq!(
        draw(None),
        draw(None),
        "the default seed is fixed, not time-based"
    );
    let mut r =
        WorldRuntime::new(World::empty("f"), 1, axiom_runtime::installed_registry()).unwrap();
    for _ in 0..1000 {
        let x = r.state.random_f32();
        assert!((0.0..1.0).contains(&x));
    }
}

#[test]
fn checkpoint_restores_the_random_stream_position() {
    let mut w = World::empty("rng-save");
    w.capabilities.insert("save".into(), 1);
    w.resources.insert("core.rng".into(), json!({"seed": 42}));
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    r.state.random_u64();
    let saved = r.checkpoint().unwrap();
    assert!(saved.rng.is_some());
    let expected: Vec<u64> = (0..4).map(|_| r.state.random_u64()).collect();
    r.restore_checkpoint(&saved).unwrap();
    let replay: Vec<u64> = (0..4).map(|_| r.state.random_u64()).collect();
    assert_eq!(expected, replay);
}

#[test]
fn chance_transitions_follow_the_seed() {
    assert_eq!(
        hash_after(random_lab(1), 600),
        hash_after(random_lab(1), 600)
    );
    assert_ne!(
        hash_after(random_lab(1), 600),
        hash_after(random_lab(2), 600)
    );
}

#[test]
fn field_and_call_assertions_check_live_entities_and_behavior_state() {
    let run = |assertions: serde_json::Value| -> anyhow::Result<serde_json::Value> {
        let s: Scenario = serde_json::from_value(json!({"ticks":1200,"assertions":assertions}))?;
        scenario::run(lab(), 1, axiom_runtime::installed_registry(), s)
    };
    let out = run(json!([
        {"kind":"call","action":"fsm.inspect","args":{"id":"enemy/0"},"pointer":"/state","op":"eq","value":"chase"},
        {"kind":"call","action":"fsm.summary","pointer":"/agents","op":"eq","value":6},
        {"kind":"field","entity":"enemy/0","field":"/position/0","op":"lt","value":9.0},
        {"kind":"field","entity":"enemy/0","field":"health","op":"eq","value":100.0}
    ]))
    .unwrap();
    assert_eq!(out["assertions"].as_array().unwrap().len(), 4);
    assert_eq!(out["state_hash"].as_str().unwrap().len(), 16);
    let failed = run(json!([
        {"kind":"call","action":"fsm.inspect","args":{"id":"enemy/5"},"pointer":"/state","op":"eq","value":"attack"}
    ]))
    .unwrap_err();
    let message = format!("{failed:#}");
    assert!(
        message.contains("fsm.inspect/state") && message.contains("expected == \"attack\""),
        "{message}"
    );
    let impure = run(json!([
        {"kind":"call","action":"rts.move","pointer":"/x","op":"eq","value":1}
    ]))
    .unwrap_err();
    assert!(format!("{impure:#}").contains("side-effect-free"));
}
