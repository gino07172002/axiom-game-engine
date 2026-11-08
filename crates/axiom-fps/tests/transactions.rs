use axiom_core::*;
use axiom_fps::{LegacyWorld as World, State};
use serde_json::json;

fn set(path: &str, value: serde_json::Value) -> Edit {
    Edit::set(path, value)
}
fn memory() -> Store {
    Store::memory_with(axiom_fps::registry(), axiom_fps::demo_world().unwrap()).unwrap()
}
#[test]
fn arena_factory_scales_cover_and_spawn() {
    let mut store = memory();
    store
        .apply(
            None,
            vec![Edit::Extension(
                json!({"kind":"make_arena","id":"small","half_size":10.0}),
            )],
        )
        .unwrap();
    let (_, world) = store.current().unwrap();
    assert_eq!(world.id, "small");
    assert_eq!(axiom_fps::from_world(&world).unwrap().arena.half_size, 10.0);
    world.compile(&axiom_fps::registry()).unwrap();
}

#[test]
fn schema_details_are_discoverable() {
    let mut store = memory();
    for topic in [None, Some("world"), Some("edits"), Some("behavior")] {
        store
            .execute(Request::Schema {
                topic: topic.map(str::to_string),
            })
            .unwrap();
    }
    assert!(store
        .execute(Request::Schema {
            topic: Some("typo".into())
        })
        .is_err());
}

#[test]
fn bundled_ir_compiles_to_stable_logical_entities() {
    let world = World::demo();
    world.validate().unwrap();
    let entities = world.compile().unwrap();
    assert_eq!(entities.len(), 14);
    assert_eq!(entities[0].id, "player");
    assert_eq!(entities[1].id, "enemy/0");
}
#[test]
fn batch_is_atomic_on_validation_failure() {
    let mut store = memory();
    let before = store.current().unwrap();
    assert!(store
        .apply(
            Some(before.0),
            vec![
                set("weapon.damage", json!(90)),
                set("enemies.count", json!(129))
            ]
        )
        .is_err());
    assert_eq!(store.current().unwrap(), before);
}
#[test]
fn batch_is_atomic_on_unknown_field_and_type_error() {
    let mut store = memory();
    let before = store.current().unwrap();
    for edit in [
        set("renderer.memory", json!(1)),
        set("enemies.count", json!("twelve")),
    ] {
        assert!(store
            .apply(None, vec![set("player.speed", json!(9)), edit])
            .is_err());
        assert_eq!(store.current().unwrap(), before);
    }
}
#[test]
fn optimistic_revision_rejects_stale_agent_edits() {
    let mut store = memory();
    store
        .apply(Some(1), vec![set("enemies.count", json!(12))])
        .unwrap();
    let error = store
        .apply(Some(1), vec![set("weapon.damage", json!(100))])
        .unwrap_err();
    assert!(error.to_string().contains("revision conflict"));
    assert_eq!(store.current().unwrap().0, 2);
}
#[test]
fn restoring_revision_appends_history() {
    let mut store = memory();
    let original = store.current().unwrap().1;
    store
        .apply(None, vec![set("enemies.count", json!(20))])
        .unwrap();
    let result = store
        .execute(Request::Undo {
            rev: 1,
            expected_rev: Some(2),
        })
        .unwrap();
    assert_eq!(result["rev"], 3);
    assert_eq!(store.current().unwrap().1, original);
    assert!(store
        .execute(Request::Undo {
            rev: 99,
            expected_rev: None
        })
        .is_err());
    assert_eq!(store.revision().unwrap(), 3);
}
#[test]
fn unchanged_edits_do_not_create_revisions() {
    let mut store = memory();
    let result = store
        .apply(Some(1), vec![set("enemies.count", json!(8))])
        .unwrap();
    assert_eq!(result["changed"], false);
    assert_eq!(store.revision().unwrap(), 1);
}
#[test]
fn invalid_spawn_and_duplicate_ids_are_rejected() {
    let mut world = World::demo();
    world.player.spawn = [-6.0, 0.9, 3.0];
    assert!(world.validate().is_err());
    let mut world = World::demo();
    world.obstacles.push(world.obstacles[0].clone());
    assert!(world.validate().is_err());
    let mut world = World::demo();
    world.weapon.damage = f32::NAN;
    assert!(world.validate().is_err());
}
#[test]
fn bounded_behavior_vm_and_terminal_death() {
    let b = World::demo().behavior;
    assert_eq!(b.step(State::Idle, 10.0, 100.0), State::Chase);
    assert_eq!(b.step(State::Chase, 1.0, 100.0), State::Attack);
    assert_eq!(b.step(State::Attack, 5.0, 100.0), State::Chase);
    assert_eq!(b.step(State::Chase, 5.0, 0.0), State::Dead);
    assert_eq!(b.step(State::Dead, 0.0, 100.0), State::Dead);
}
#[test]
fn query_projection_and_budgets() {
    let mut store = memory();
    let value = store
        .execute(Request::Query {
            scope: Scope::Entities,
            limit: Some(2),
        })
        .unwrap();
    assert_eq!(value["entities"].as_array().unwrap().len(), 2);
    assert_eq!(value["total"], 16);
    assert!(store.apply(None, vec![]).is_err());
    assert!(store
        .apply(None, vec![set("title", json!("x")); 257])
        .is_err());
    assert!(parse_request(&" ".repeat(1_048_577)).is_err());
    assert!(parse_request(r#"{"op":"verify","typo":true}"#).is_err());
}
#[test]
fn edits_validate_final_world_instead_of_intermediate_state() {
    let mut store = memory();
    let result = store
        .apply(
            None,
            vec![
                set("player.spawn", json!([-6.0, 0.9, 3.0])),
                Edit::Extension(json!({"kind":"remove_obstacle","id":"west-01"})),
            ],
        )
        .unwrap();
    assert_eq!(result["entities_delta"], -1);
}
#[test]
fn sqlite_persistence_and_two_writer_conflict() {
    let path = std::env::temp_dir().join(format!(
        "axiom-test-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let mut a = axiom_fps::open_store(&path).unwrap();
        let mut b = axiom_fps::open_store(&path).unwrap();
        a.apply(Some(1), vec![set("enemies.count", json!(16))])
            .unwrap();
        assert!(b
            .apply(Some(1), vec![set("enemies.count", json!(18))])
            .is_err());
        assert_eq!(
            axiom_fps::from_world(&b.current().unwrap().1)
                .unwrap()
                .enemies
                .count,
            16
        );
    }
    {
        let store = axiom_fps::open_store(&path).unwrap();
        assert_eq!(store.revision().unwrap(), 2);
        assert_eq!(
            axiom_fps::from_world(&store.current().unwrap().1)
                .unwrap()
                .enemies
                .count,
            16
        );
    }
    std::fs::remove_file(path).unwrap();
}
