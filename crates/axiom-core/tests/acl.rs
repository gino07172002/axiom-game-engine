use axiom_core::{error_response, Entity, Prefab, Registry, Request, Store, World};
use serde_json::{json, Value};

fn fixture() -> Store {
    let mut world = World::empty("acl");
    world.prefabs.insert(
        "enemy.basic".into(),
        Prefab {
            components: [
                ("core.health".into(), json!({"value":100,"max":100})),
                ("core.transform".into(), json!({"position":[0,0,0]})),
            ]
            .into(),
        },
    );
    let mut player = Entity::new("player");
    player
        .set("core.health", json!({"value":80,"max":100}))
        .unwrap();
    world.entities.push(player);
    Store::memory_with(Registry::core(), world).unwrap()
}

fn acl(store: &mut Store, text: &str) -> Result<String, Value> {
    acl_with(store, text, false)
}

fn acl_with(store: &mut Store, text: &str, preview: bool) -> Result<String, Value> {
    store
        .execute(Request::Acl {
            text: text.into(),
            preview,
            expected_rev: None,
        })
        .map(|v| v["text"].as_str().unwrap().to_owned())
        .map_err(|e| error_response(&e))
}

#[test]
fn spawn_set_and_select_round_trip_in_few_bytes() {
    let mut store = fixture();
    let out = acl(
        &mut store,
        "spawn enemy.basic count=10 id=enemy/{i} at=0,0,0 step=2,0,0 tag=enemy",
    )
    .unwrap();
    assert!(out.starts_with("ok rev=2 changes="), "{out}");
    let out = acl(
        &mut store,
        "set enemy/* core.health/value 40 where core.transform/position/0>=10",
    )
    .unwrap();
    // x = 0,2,..,18 → enemy/5..enemy/9 match.
    assert!(out.starts_with("ok rev=3 changes=5"), "{out}");
    let out = acl(
        &mut store,
        "select entities[tag=enemy] where core.health/value<50 fields core.health/value limit 3",
    )
    .unwrap();
    assert!(
        out.starts_with(
            "enemy/5 core.health/value=40\n\
             enemy/6 core.health/value=40\n\
             enemy/7 core.health/value=40\n\
             matched=5 rev=3 next="
        ),
        "{out}"
    );
}

#[test]
fn edit_scripts_are_one_atomic_revision() {
    let mut store = fixture();
    let err = acl(
        &mut store,
        "spawn enemy.basic count=3 id=e/{i}\nset e/* core.health/value 999",
    )
    .unwrap_err();
    // max is 100, so the whole script fails and nothing is written.
    assert_eq!(err["ok"], false);
    assert_eq!(store.current().unwrap().0, 1);
    let ok = acl(
        &mut store,
        "spawn enemy.basic count=3 id=e/{i}; set e/* core.health/value 7; set player label \"Hero; one\"",
    )
    .unwrap();
    assert!(ok.starts_with("ok rev=2"), "{ok}");
    let inspect = acl(&mut store, "inspect player").unwrap();
    assert!(
        inspect.starts_with("player label=\"Hero; one\""),
        "{inspect}"
    );
    assert!(
        inspect.contains("core.health {\"max\":100,\"value\":80}"),
        "{inspect}"
    );
}

#[test]
fn preview_delete_undo_history_and_diff() {
    let mut store = fixture();
    acl(&mut store, "spawn enemy.basic count=4 id=e/{i}").unwrap();
    let preview = acl_with(&mut store, "delete e/*", true).unwrap();
    assert!(preview.starts_with("preview rev=2"), "{preview}");
    assert_eq!(store.current().unwrap().0, 2);
    let deleted = acl(&mut store, "delete e/*").unwrap();
    assert!(deleted.starts_with("ok rev=3"), "{deleted}");
    assert!(acl(&mut store, "select e/*")
        .unwrap()
        .ends_with("matched=0 rev=3"));
    let undo = acl(&mut store, "undo 2").unwrap();
    assert_eq!(undo, "ok rev=4 restored_from=2");
    assert!(acl(&mut store, "select e/*")
        .unwrap()
        .contains("matched=4 rev=4"));
    let history = acl(&mut store, "history 2").unwrap();
    assert!(history.starts_with("4 restore:2 "), "{history}");
    let diff = acl(&mut store, "diff 3 4").unwrap();
    assert!(diff.starts_with("diff 3..4 changes="), "{diff}");
}

#[test]
fn errors_name_the_line_and_keep_structured_codes() {
    let mut store = fixture();
    let err = acl(&mut store, "# comment\nsel enemy/*").unwrap_err();
    assert_eq!(err["diagnostics"][0]["code"], "acl_syntax");
    assert_eq!(err["diagnostics"][0]["path"], "/line/2");
    let err = acl(&mut store, "set ghost/* label x").unwrap_err();
    assert_eq!(err["diagnostics"][0]["code"], "no_match", "{err}");
    acl(&mut store, "spawn enemy.basic id=boss").unwrap();
    let err = acl(&mut store, "spawn enemy.basic id=boss").unwrap_err();
    assert_eq!(err["diagnostics"][0]["code"], "id_taken", "{err}");
    let err = acl(&mut store, "select player\nselect player").unwrap_err();
    assert!(err["error"].as_str().unwrap().contains("must be alone"));
    let err = store
        .execute(Request::Acl {
            text: "set player label x".into(),
            preview: false,
            expected_rev: Some(1),
        })
        .unwrap_err();
    assert_eq!(
        error_response(&err)["diagnostics"][0]["code"],
        "revision_conflict"
    );
}
