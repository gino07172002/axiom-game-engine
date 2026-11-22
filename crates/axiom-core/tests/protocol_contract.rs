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

/// The JSON Schema subset discovery uses (local `$ref`, `oneOf`, `type`, `const`, `enum`,
/// object `properties`/`required`/`additionalProperties`, array `items`/`minItems`/`maxItems`,
/// number `minimum`/`maximum`): whether `value` is valid against `schema`.
fn conforms(root: &Value, schema: &Value, value: &Value) -> bool {
    if let Some(r) = schema["$ref"].as_str() {
        let target = root.pointer(r.trim_start_matches('#')).expect(r);
        return conforms(root, target, value);
    }
    if let Some(options) = schema["oneOf"].as_array() {
        return options.iter().filter(|o| conforms(root, o, value)).count() == 1;
    }
    if let Some(c) = schema.get("const") {
        return c == value;
    }
    if let Some(options) = schema["enum"].as_array() {
        return options.contains(value);
    }
    let types: Vec<&str> = match &schema["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    let type_ok = |t: &str| match t {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "null" => value.is_null(),
        other => panic!("unsupported type {other}"),
    };
    if !types.is_empty() && !types.into_iter().any(type_ok) {
        return false;
    }
    if let Some(object) = value.as_object() {
        let properties = schema["properties"].as_object();
        for name in schema["required"].as_array().into_iter().flatten() {
            if !object.contains_key(name.as_str().unwrap()) {
                return false;
            }
        }
        for (k, v) in object {
            match properties.and_then(|p| p.get(k)) {
                Some(s) if !conforms(root, s, v) => return false,
                Some(_) => {}
                None => match &schema["additionalProperties"] {
                    Value::Bool(false) => return false,
                    s @ Value::Object(_) if !conforms(root, s, v) => return false,
                    _ => {}
                },
            }
        }
    }
    if let Some(items) = value.as_array() {
        let n = items.len() as u64;
        if schema["minItems"].as_u64().is_some_and(|m| n < m)
            || schema["maxItems"].as_u64().is_some_and(|m| n > m)
        {
            return false;
        }
        if let Some(item) = schema.get("items") {
            if !items.iter().all(|v| conforms(root, item, v)) {
                return false;
            }
        }
    }
    if let Some(n) = value.as_f64() {
        if schema["minimum"].as_f64().is_some_and(|m| n < m)
            || schema["maximum"].as_f64().is_some_and(|m| n > m)
        {
            return false;
        }
    }
    true
}

#[test]
fn gameplay_schema_accepts_and_refuses_the_same_samples_as_the_runtime() {
    use axiom_core::gameplay::{validate_command, Operand, Program};
    let schema = Registry::core().schema(Some("narrative.graph")).unwrap();
    let operand = &schema["$defs"]["operand"];
    for (sample, valid) in [
        (
            json!({"kind":"animation","entity":"hero","field":"param","param":"speed"}),
            true,
        ),
        (
            json!({"kind":"animation","entity":"hero","field":"finished"}),
            true,
        ),
        (
            json!({"kind":"animation","entity":"hero","field":"bogus"}),
            false,
        ),
        (json!({"kind":"animation","field":"clip"}), false),
        (json!({"kind":"variable","id":"score"}), true),
    ] {
        assert_eq!(
            conforms(&schema, operand, &sample),
            valid,
            "schema on {sample}"
        );
        assert_eq!(
            serde_json::from_value::<Operand>(sample.clone()).is_ok(),
            valid,
            "serde on {sample}"
        );
    }
    let command = &schema["$defs"]["command"];
    let call = json!({"kind":"call","capability":"fsm","action":"inspect","args":{}});
    for (sample, valid) in [
        (json!({"kind":"batch","commands":[call]}), true),
        (call.clone(), true),
        (
            json!({"kind":"batch","commands":[{"kind":"batch","commands":[call]}]}),
            false,
        ),
        (
            json!({"kind":"batch","commands":[{"kind":"slide","widget":"w","value":1.0}]}),
            false,
        ),
        (json!({"kind":"batch","commands":[]}), false),
        (
            json!({"kind":"batch","commands":vec![call.clone(); 17]}),
            false,
        ),
    ] {
        assert_eq!(
            conforms(&schema, command, &sample),
            valid,
            "schema on {sample}"
        );
        let runtime = serde_json::from_value::<RuntimeCommand>(sample.clone())
            .map_err(anyhow::Error::from)
            .and_then(|c| validate_command(&c, &Program::default(), None, None));
        assert_eq!(runtime.is_ok(), valid, "runtime on {sample}: {runtime:?}");
    }
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
