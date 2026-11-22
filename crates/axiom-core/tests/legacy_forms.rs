//! Databases written before validation refused the array form of object types (e.g.
//! `audio.library: []`) still open, edit and undo: reading a revision rewrites the legacy
//! form, while new writes of it stay refused.
use axiom_core::{ops, CoreEdit, Edit, Registry, Request, Store, World};
use serde_json::{json, Value};

/// A world as an older build stored it: an array-form resource, component and prefab component.
fn legacy_world() -> Value {
    let mut w = json!(World::empty("legacy"));
    w["capabilities"]["audio"] = json!(1);
    w["resources"]["audio.library"] = json!([]);
    w["entities"] =
        json!([{"id":"hero","components":{"core.transform":{},"core.health":[50.0,100.0]}}]);
    w["prefabs"] = json!({"crate":{"components":{"core.health":[10.0,20.0]}}});
    w
}

/// A database in a fresh temp directory whose newest revision is `legacy_world()`, written the
/// way an older build did (straight JSON, no validation by this build).
fn legacy_database(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("axiom-legacy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("world.sqlite");
    drop(Store::open(&path).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO revisions(world,reason) VALUES (?1,'legacy')",
            [legacy_world().to_string()],
        )
        .unwrap();
    path
}

#[test]
fn a_database_holding_legacy_array_forms_opens_edits_and_undoes() {
    let path = legacy_database("open");
    let mut store = Store::open(&path).expect("a legacy revision must not block opening");
    assert_eq!(store.revision().unwrap(), 2, "opening writes no revision");
    // Reading normalizes to the object form the ops edit in place.
    let (_, world) = store.current().unwrap();
    assert_eq!(
        world.resources["audio.library"],
        json!({"cues":{},"clips":{}})
    );
    assert_eq!(
        world.entity("hero").unwrap().components["core.health"],
        json!({"value":50.0,"max":100.0})
    );
    assert_eq!(
        world.prefabs["crate"].components["core.health"],
        json!({"value":10.0,"max":20.0})
    );
    assert_eq!(store.execute(Request::Verify {}).unwrap()["ok"], true);
    // A preview and an edit through an op that edits audio.library in place.
    let edits = ops::add_audio_clip(&world, "creak", "creak").unwrap();
    let preview = store
        .execute(Request::Preview {
            expected_rev: Some(2),
            edits: edits.clone(),
        })
        .unwrap();
    assert_eq!(preview["changed"], true, "{preview}");
    let applied = store
        .execute(Request::Apply {
            expected_rev: Some(2),
            edits,
        })
        .unwrap();
    assert_eq!(applied["rev"], 3, "{applied}");
    // The legacy form is not reported as a change of that edit.
    let diff = store
        .execute(Request::Diff {
            from_rev: 2,
            to_rev: 3,
            offset: 0,
            limit: Some(128),
        })
        .unwrap();
    assert!(
        !diff.to_string().contains("core.health"),
        "normalization is not an edit: {diff}"
    );
    // Undo back to the legacy revision, then keep editing on top of it.
    let undone = store
        .execute(Request::Undo {
            rev: 2,
            expected_rev: Some(3),
        })
        .unwrap();
    assert_eq!(undone["rev"], 4, "{undone}");
    let (_, restored) = store.current().unwrap();
    assert!(!restored.resources["audio.library"]["clips"]
        .as_object()
        .unwrap()
        .contains_key("creak"));
    let edits = ops::add_audio_clip(&restored, "wind", "creak").unwrap();
    assert_eq!(
        store
            .execute(Request::Apply {
                expected_rev: Some(4),
                edits,
            })
            .unwrap()["rev"],
        5
    );
    // A fresh connection to the same file (e.g. the next CLI call) opens it too.
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.revision().unwrap(), 5);
    // The stored legacy revision itself is history and stays as written.
    assert!(reopened
        .revision_json(2)
        .unwrap()
        .contains("\"audio.library\":[]"));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn new_writes_of_the_array_form_are_still_refused() {
    let path = legacy_database("write");
    let mut store = Store::open(&path).unwrap();
    for (edit, what) in [
        (
            CoreEdit::SetResource {
                id: "audio.library".into(),
                value: json!([]),
            },
            "resource",
        ),
        (
            CoreEdit::SetComponent {
                entity: "hero".into(),
                component: "core.health".into(),
                value: json!([1.0, 2.0]),
            },
            "component",
        ),
    ] {
        let error = store
            .execute(Request::Apply {
                expected_rev: None,
                edits: vec![Edit::Core(edit)],
            })
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("not an array"),
            "{what}: {error:#}"
        );
    }
    assert_eq!(store.revision().unwrap(), 2);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn decoding_rewrites_only_registered_object_types() {
    let registry = Registry::core();
    let mut raw = legacy_world();
    // A world-defined resource keeps whatever its own schema says; an unregistered value is
    // left for validation to report.
    raw["resources"]["custom.list"] = json!([1, 2]);
    let world = registry.decode(raw).unwrap();
    assert_eq!(world.resources["custom.list"], json!([1, 2]));
    assert!(world.resources["audio.library"].is_object());
}
