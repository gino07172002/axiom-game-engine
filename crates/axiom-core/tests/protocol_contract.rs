//! Discovery schemas match the request types the store accepts, and store entry points
//! that only read do not take the write lock or load unneeded content.
use axiom_core::{
    gameplay::{GameAction, RuntimeCommand},
    local::{MAX_SELECT_LIMIT, MAX_SET_WHERE_MATCHES},
    parse_request, CoreEdit, Entity, Registry, Request, Store, World,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// The tags serde accepts for an internally tagged enum, read from its own "unknown variant"
/// error, so a new variant cannot be left out of a hand-written schema unnoticed.
fn variants<T: serde::de::DeserializeOwned>(tag: &str) -> BTreeSet<String> {
    let error = serde_json::from_value::<T>(json!({ tag: "__unknown__" }))
        .err()
        .unwrap()
        .to_string();
    let list = error.split("expected one of").nth(1).expect(&error);
    list.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The `kind` constants of a schema `oneOf`.
fn kinds(one_of: &Value) -> BTreeSet<String> {
    one_of["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["properties"]["kind"]["const"].as_str())
        .map(str::to_owned)
        .collect()
}

fn discover(store: &mut Store, topic: &str) -> Value {
    store
        .execute(Request::Discover {
            topic: Some(topic.into()),
            if_hash: None,
        })
        .unwrap()["schema"]
        .clone()
}

#[test]
fn discovery_lists_every_edit_action_and_command_variant() {
    let registry = Registry::core();
    let edits = registry.schema(Some("edits")).unwrap();
    let listed: BTreeSet<String> = edits["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(listed, variants::<CoreEdit>("kind"));
    let gameplay = registry.schema(Some("narrative.graph")).unwrap();
    assert_eq!(
        kinds(&gameplay["$defs"]["action"]),
        variants::<GameAction>("kind")
    );
    assert_eq!(
        kinds(&gameplay["$defs"]["command"]),
        variants::<RuntimeCommand>("kind")
    );
}

#[test]
fn read_and_edit_selections_declare_the_limits_the_store_enforces() {
    let mut store = Store::memory_with(Registry::core(), {
        let mut w = World::empty("contract");
        for i in 0..300 {
            w.entities.push(Entity::new(format!("e/{i:03}")));
        }
        w
    })
    .unwrap();
    let local = discover(&mut store, "local");
    let read = &local["$defs"]["selection"]["properties"];
    let edit = &local["$defs"]["edit_selection"]["properties"];
    assert_eq!(read["limit"]["maximum"], json!(MAX_SELECT_LIMIT));
    assert_eq!(edit["limit"]["maximum"], json!(MAX_SET_WHERE_MATCHES));
    assert!(edit.get("cursor").is_none() && edit.get("fields").is_none());
    let set_where = local["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["properties"]["kind"]["const"] == "set_where")
        .unwrap();
    assert_eq!(
        set_where["properties"]["select"]["$ref"],
        "#/$defs/edit_selection"
    );
    // What the schema allows is accepted, and its limits are the store's.
    let select = |limit: usize| {
        parse_request(
            &json!({"op":"select","query":{"from":"entities","id_glob":"e/*","limit":limit}})
                .to_string(),
        )
        .unwrap()
    };
    assert!(store.execute(select(MAX_SELECT_LIMIT)).is_ok());
    assert!(store.execute(select(MAX_SELECT_LIMIT + 1)).is_err());
    let set = parse_request(
        &json!({"op":"preview","edits":[{"kind":"set_where","select":{"from":"entities","id_glob":"e/*","limit":300},"field":"/label","value":"x"}]})
            .to_string(),
    )
    .unwrap();
    assert_eq!(store.execute(set).unwrap()["changed"], true);
}

#[test]
fn typed_discovery_finds_the_current_worlds_own_schemas() {
    let mut world = World::empty("typed");
    world.component_schemas.insert(
        "game.score".into(),
        serde_json::from_value(json!({"fields":{"score":{"ty":"int","min":0}}})).unwrap(),
    );
    let mut store = Store::memory_with(Registry::core(), world).unwrap();
    let schema = discover(&mut store, "typed:game.score");
    assert!(schema.to_string().contains("score"), "{schema}");
    let direct = store
        .execute(Request::Schema {
            topic: Some("typed:game.score".into()),
        })
        .unwrap();
    assert_eq!(direct, schema);
    assert!(store
        .execute(Request::Schema {
            topic: Some("typed:game.unknown".into()),
        })
        .is_err());
}

fn temp_db(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "axiom-{name}-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    remove_db(&path);
    path
}

fn remove_db(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn preview_reads_a_snapshot_while_another_connection_holds_the_write_lock() {
    let path = temp_db("preview-lock");
    let mut store = Store::open_with(&path, Registry::core(), {
        let mut w = World::empty("lock");
        w.entities.push(Entity::new("crate"));
        w
    })
    .unwrap();
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    let preview = store
        .execute(
            parse_request(
                &json!({"op":"preview","expected_rev":1,"edits":[{"kind":"set_field","target":{"kind":"entity","id":"crate"},"field":"/label","value":"Box"}]})
                    .to_string(),
            )
            .unwrap(),
        )
        .unwrap();
    // A preview that took the write lock would wait the 3 s busy timeout and then fail.
    assert_eq!(preview["changed"], true);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    writer.execute_batch("ROLLBACK").unwrap();
    drop(writer);
    drop(store);
    remove_db(&path);
}

#[test]
fn an_existing_database_never_builds_the_initial_world() {
    let path = temp_db("lazy");
    let mut built = 0;
    drop(
        Store::open_lazy(&path, Registry::core(), || {
            built += 1;
            Ok(World::empty("first"))
        })
        .unwrap(),
    );
    let again = Store::open_lazy(&path, Registry::core(), || -> anyhow::Result<World> {
        anyhow::bail!("the initial content must not load for an existing database")
    })
    .unwrap();
    assert_eq!(built, 1);
    assert_eq!(again.current().unwrap().1.id, "first");
    drop(again);
    remove_db(&path);
}
