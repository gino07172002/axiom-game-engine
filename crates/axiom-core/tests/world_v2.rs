use axiom_core::*;
use serde_json::json;
fn parse(v: serde_json::Value) -> World {
    serde_json::from_value(v).unwrap()
}
fn with_entities(entities: serde_json::Value) -> World {
    parse(
        json!({"schema_version":2,"id":"test","title":"test","capabilities":{"core":1,"render":1,"motion":1},"entities":entities}),
    )
}
#[test]
fn empty_world_has_no_fps_requirements() {
    let w = World::empty("empty");
    let c = w.compile(&Registry::core()).unwrap();
    assert!(c.is_empty());
    assert!(c.schedule.is_empty());
    assert!(!json!(w).as_object().unwrap().contains_key("player"));
}
#[test]
fn native_optional_components_and_prefab_overrides() {
    let mut w = with_entities(
        json!([{"id":"box","prefab":"crate","components":{"core.transform":{"position":[2,0,0]}}}]),
    );
    w.prefabs.insert("crate".into(),Prefab{components:serde_json::from_value(json!({"core.transform":{"position":[0,0,0]},"core.health":{"value":100,"max":100}})).unwrap()});
    let c = w.compile(&Registry::core()).unwrap();
    assert_eq!(
        c.entities[0].transform.as_ref().unwrap().position,
        [2.0, 0.0, 0.0]
    );
    assert_eq!(c.entities[0].health.as_ref().unwrap().max, 100.0);
}
#[test]
fn custom_schema_compiles_typed_columns_and_entity_refs() {
    let mut w = with_entities(
        json!([{"id":"a","components":{"game.flags":{"enabled":true,"count":3,"target":"b"}}},{"id":"b"}]),
    );
    w.component_schemas.insert("game.flags".into(),serde_json::from_value(json!({"fields":{"enabled":{"ty":"bool"},"count":{"ty":"int","min":0},"target":{"ty":"entity"}}})).unwrap());
    let c = w.compile(&Registry::core()).unwrap();
    let table = &c.columns["game.flags"];
    assert!(matches!(&table.fields["count"],Column::Int(v) if v==&vec![Some(3)]));
    assert!(matches!(&table.fields["target"],Column::Entity(v) if v==&vec![Some(1)]));
    w.entities[0].components.get_mut("game.flags").unwrap()["count"] = json!(3.5);
    assert!(w.compile(&Registry::core()).is_err());
}
#[test]
fn missing_capability_and_unknown_component_are_diagnostic() {
    let mut w = World::empty("test");
    w.capabilities.insert("not.installed".into(), 1);
    let error = w.compile(&Registry::core()).unwrap_err();
    assert_eq!(
        error_response(&error)["diagnostics"][0]["code"],
        "missing_capability"
    );
    w.capabilities.remove("not.installed");
    let mut e = Entity::new("a");
    e.components.insert("typo.component".into(), json!({}));
    w.entities.push(e);
    let error = w.compile(&Registry::core()).unwrap_err();
    assert_eq!(
        error_response(&error)["diagnostics"][0]["path"],
        "entities/a/components"
    );
}
#[test]
fn native_fields_are_strict_and_finite() {
    for component in [
        json!({"core.health":{"value":2,"max":1}}),
        json!({"core.transform":{"rotation":[0,0,0,0]}}),
        json!({"core.transform":{"position":[1e100,0,0]}}),
        json!({"core.health":{"value":1,"max":1,"typo":1}}),
        json!({"render.mesh":{"parts":[{"color":[1,1,1]}]}}),
    ] {
        let w = with_entities(json!([{"id":"a","components":component}]));
        assert!(w.compile(&Registry::core()).is_err());
    }
}
#[test]
fn duplicate_ids_dangling_references_and_parent_cycles_fail() {
    let mut w = with_entities(json!([{"id":"a"},{"id":"a"}]));
    assert!(w.compile(&Registry::core()).is_err());
    w.entities[1].id = "b".into();
    w.relations = vec![
        Relation {
            source: "a".into(),
            kind: "parent".into(),
            target: "b".into(),
        },
        Relation {
            source: "b".into(),
            kind: "parent".into(),
            target: "a".into(),
        },
    ];
    assert!(w.compile(&Registry::core()).is_err());
    w.relations.clear();
    w.scenes.push(Scene {
        id: "room".into(),
        entities: vec!["missing".into()],
        active: true,
    });
    assert!(w.compile(&Registry::core()).is_err());
}
#[test]
fn inactive_scene_members_are_disabled_and_globals_remain_active() {
    let mut w = with_entities(json!([{"id":"global"},{"id":"room"}]));
    w.scenes.push(Scene {
        id: "sleeping".into(),
        entities: vec!["room".into()],
        active: false,
    });
    assert_eq!(
        w.compile(&Registry::core()).unwrap().enabled,
        vec![true, false]
    );
}
#[test]
fn system_schedule_rejects_missing_dependencies_and_cycles() {
    let mut w = World::empty("test");
    w.systems = vec!["a".into(), "b".into()];
    let mut r = Registry::core();
    for (id, after) in [("a", "b"), ("b", "a")] {
        r.systems.insert(
            id.into(),
            SystemDescriptor {
                id: id.into(),
                capability: "core".into(),
                phase: 1,
                reads: vec![],
                writes: vec![],
                after: vec![after.into()],
            },
        );
    }
    assert!(w.compile(&r).is_err());
    r.systems.get_mut("a").unwrap().after.clear();
    let c = w.compile(&r).unwrap();
    assert_eq!(
        c.schedule.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}
#[test]
fn preview_and_failed_batches_do_not_write_revisions() {
    let mut s = Store::memory().unwrap();
    let before = s.current().unwrap();
    let edit:Edit=serde_json::from_value(json!({"kind":"put_entity","entity":{"id":"box","components":{"core.transform":{"position":[1,0,0]}}}})).unwrap();
    let p = s
        .execute(Request::Preview {
            expected_rev: Some(1),
            edits: vec![edit.clone()],
        })
        .unwrap();
    assert_eq!(p["diff"]["added"], 1);
    assert_eq!(s.current().unwrap(), before);
    let bad = Edit::set("/entities/0/id", json!(""));
    assert!(s.apply(None, vec![edit.clone(), bad]).is_err());
    assert_eq!(s.current().unwrap(), before);
    s.apply(Some(1), vec![edit]).unwrap();
    assert_eq!(s.revision().unwrap(), 2);
}
#[test]
fn generic_edits_resource_schemas_and_cas() {
    let mut s = Store::memory().unwrap();
    let request=parse_request(r#"{"op":"apply","expected_rev":1,"edits":[{"kind":"define_resource","id":"story.flags","schema":{"fields":{"opened":{"ty":"bool"}}}},{"kind":"set_resource","id":"story.flags","value":{"opened":false}}]}"#).unwrap();
    s.execute(request).unwrap();
    assert_eq!(
        s.current().unwrap().1.resources["story.flags"]["opened"],
        false
    );
    assert!(s
        .apply(Some(1), vec![Edit::set("title", json!("stale"))])
        .is_err());
    assert!(s
        .apply(
            None,
            vec![Edit::set("/resources/story.flags/opened", json!("yes"))]
        )
        .is_err());
    assert_eq!(s.revision().unwrap(), 2);
    s.execute(Request::Undo {
        rev: 1,
        expected_rev: Some(2),
    })
    .unwrap();
    assert!(s.current().unwrap().1.resources.is_empty());
}
#[test]
fn unknown_edit_fields_cannot_fall_through_to_extension() {
    let mut s = Store::memory().unwrap();
    let r = parse_request(
        r#"{"op":"apply","edits":[{"kind":"set","path":"title","value":"x","typo":1}]}"#,
    )
    .unwrap();
    assert!(s.execute(r).is_err());
    assert!(parse_request(r#"{"op":"observe","typo":1}"#).is_err());
    assert_eq!(s.revision().unwrap(), 1);
}
#[test]
fn schema_cannot_shadow_native_types() {
    let mut w = World::empty("test");
    let schema = serde_json::from_value(json!({"fields":{"enabled":{"ty":"bool"}}})).unwrap();
    w.component_schemas.insert("core.health".into(), schema);
    assert!(w.compile(&Registry::core()).is_err());
}
#[test]
fn multiple_cameras_require_valid_viewports() {
    let mut w = with_entities(
        json!([{"id":"left","components":{"core.transform":{},"render.camera":{"projection":{"kind":"perspective","fov_degrees":75},"near":0.1,"far":100,"active":true,"viewport":[0,0,0.5,1]}}},{"id":"right","components":{"core.transform":{},"render.camera":{"projection":{"kind":"orthographic","height":10},"near":0.1,"far":100,"active":true,"viewport":[0.5,0,0.5,1]}}}]),
    );
    assert_eq!(
        w.compile(&Registry::core())
            .unwrap()
            .entities
            .iter()
            .filter(|e| e.camera.is_some())
            .count(),
        2
    );
    w.entities[1].components.get_mut("render.camera").unwrap()["viewport"] = json!([0.5, 0, 1, 1]);
    assert!(w.compile(&Registry::core()).is_err());
}
