//! Databases written before validation refused the array form of object types (e.g.
//! `audio.library: []`) still open, edit and undo: reading a revision rewrites the legacy
//! form, while new writes of it stay refused.
use axiom_core::{ops, CoreEdit, Edit, Registry, RenderLight, Request, Store, World};
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
    legacy_database_with(name, legacy_world())
}

/// `legacy_database` whose newest revision is `world`.
fn legacy_database_with(name: &str, world: Value) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("axiom-legacy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("world.sqlite");
    drop(Store::open(&path).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO revisions(world,reason) VALUES (?1,'legacy')",
            [world.to_string()],
        )
        .unwrap();
    path
}

/// Stored before validation refused arrays: a prefab point light with a range, overridden by
/// an entity's array-form `render.light` (which replaced the prefab value whole), and the
/// other way round (an array prefab value replaced whole by an object override).
fn legacy_light_world() -> Value {
    let mut w = json!(World::empty("lights"));
    w["capabilities"]["render"] = json!(1);
    w["prefabs"] = json!({
        "lamp":{"components":{"render.light":{"kind":"point","range":10.0,"shadows":true}}},
        "old_lamp":{"components":{"render.light":["point",[1.0,0.5,0.5],2.0,6.0]}}
    });
    w["entities"] = json!([
        {"id":"sun","prefab":"lamp","components":{"core.transform":{},"render.light":["directional"]}},
        {"id":"moon","prefab":"old_lamp","components":{"core.transform":{},"render.light":{"kind":"directional"}}}
    ]);
    w
}

/// The `render.light` an entity of `world` compiles to.
fn compiled_light(world: &World, id: &str) -> RenderLight {
    let c = world.compile(&Registry::core()).unwrap();
    let e = &c.entities[c.indices[id]];
    serde_json::from_value(e.components["render.light"].clone()).unwrap()
}

#[test]
fn a_legacy_array_override_still_replaces_its_prefab_value_whole() {
    let path = legacy_database_with("override", legacy_light_world());
    let mut store = Store::open(&path).expect("a legacy override must not block opening");
    let (_, world) = store.current().unwrap();
    let sun = RenderLight::directional([1.0; 3], 1.0, false);
    assert_eq!(
        compiled_light(&world, "sun"),
        sun,
        "no range or shadows inherited"
    );
    assert_eq!(
        compiled_light(&world, "moon"),
        sun,
        "no point fields inherited"
    );
    assert_eq!(store.execute(Request::Verify {}).unwrap()["ok"], true);
    // A repair edit to an unrelated component commits, and the committed (normalized)
    // revision reads the same light.
    let applied = store
        .execute(Request::Apply {
            expected_rev: Some(2),
            edits: vec![Edit::Core(CoreEdit::SetComponent {
                entity: "sun".into(),
                component: "core.transform".into(),
                value: json!({"position":[1.0,2.0,3.0]}),
            })],
        })
        .unwrap();
    assert_eq!(applied["rev"], 3, "{applied}");
    let (_, edited) = store.current().unwrap();
    assert_eq!(compiled_light(&edited, "sun"), sun);
    assert_eq!(compiled_light(&edited, "moon"), sun);
    // Undo restores the legacy revision; a fresh connection opens it.
    let undone = store
        .execute(Request::Undo {
            rev: 2,
            expected_rev: Some(3),
        })
        .unwrap();
    assert_eq!(undone["rev"], 4, "{undone}");
    drop(store);
    let reopened = Store::open(&path).unwrap();
    let (rev, restored) = reopened.current().unwrap();
    assert_eq!(rev, 4);
    assert_eq!(compiled_light(&restored, "sun"), sun);
    assert_eq!(compiled_light(&restored, "moon"), sun);
    drop(reopened);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn replace_world_refuses_array_forms_like_set_component() {
    let mut store = Store::memory_with(Registry::core(), World::empty("clean")).unwrap();
    for world in [legacy_light_world(), legacy_world()] {
        let error = store
            .execute(Request::Apply {
                expected_rev: Some(1),
                edits: vec![Edit::Core(CoreEdit::ReplaceWorld { world })],
            })
            .unwrap_err();
        assert!(format!("{error:#}").contains("not an array"), "{error:#}");
    }
    assert_eq!(store.revision().unwrap(), 1);
    // The object form of the same world is accepted.
    let path = legacy_database_with("replace", legacy_light_world());
    let current = Store::open(&path).unwrap().current().unwrap().1;
    let applied = store
        .execute(Request::Apply {
            expected_rev: Some(1),
            edits: vec![Edit::replace(current)],
        })
        .unwrap();
    assert_eq!(applied["rev"], 2, "{applied}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
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

/// Final round C0 R1-6.2-F3: an override of a legacy pair that spelled a value equal to a
/// default serialization skips (`camera.follow` `input.drag_yaw` 360) keeps it instead of
/// inheriting the prefab's value.
#[test]
fn a_legacy_override_keeps_an_explicit_value_serialization_skips() {
    // CameraFollow in field order, with input {drag, drag_yaw}.
    let array = |drag_yaw: f64| {
        json!(["hero", null, 5.0, 1.0, 12.0, 0.0, 20.0, -30.0, 75.0, 1.5, 0.1, 0.1,
            {"drag": true, "drag_yaw": drag_yaw}, {}, true])
    };
    let mut raw = json!(World::empty("cams"));
    raw["capabilities"]["camera"] = json!(1);
    raw["prefabs"] = json!({
        "old_cam": {"components": {"camera.follow": array(720.0)}},
        "cam": {"components": {"camera.follow": {"target": "hero", "input": {"drag": true, "drag_yaw": 720.0}}}}
    });
    raw["entities"] = json!([
        // Object override of an array prefab value.
        {"id": "a", "prefab": "old_cam", "components": {"camera.follow":
            {"target": "hero", "input": {"drag": true, "drag_yaw": 360.0}}}},
        // Array override of an object prefab value.
        {"id": "b", "prefab": "cam", "components": {"camera.follow": array(360.0)}}
    ]);
    let world = Registry::core().decode(raw).unwrap();
    for id in ["a", "b"] {
        let resolved = world.resolved_components(world.entity(id).unwrap());
        let follow: axiom_core::camera::CameraFollow =
            serde_json::from_value(resolved["camera.follow"].clone()).unwrap();
        assert_eq!(follow.input.drag_yaw, 360.0, "{id}: {follow:?}");
        assert!(follow.input.drag, "{id}");
    }
}
