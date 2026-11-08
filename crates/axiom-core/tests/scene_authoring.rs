use axiom_core::{
    local::Target, parse_request, scene_edit::TreeQuery, CoreEdit, Edit, Entity, Prefab, Registry,
    Relation, Request, Scene, Store, Transform, World,
};
use serde_json::{json, Value};

fn fixture() -> Store {
    let mut world = World::empty("graph");
    let mut parent = Entity::new("stage/root");
    parent
        .set(
            "core.transform",
            Transform {
                position: [4.0, 0.0, 0.0],
                ..Default::default()
            },
        )
        .unwrap();
    let mut child = Entity::new("stage/child");
    child.label = Some("Cargo".into());
    child.prefab = Some("props".into());
    world.prefabs.insert(
        "props".into(),
        Prefab {
            components: [("core.transform".into(), json!({"position":[1,2,3]}))].into(),
        },
    );
    world.entities = vec![parent, child, Entity::new("loose")];
    world.relations = vec![Relation {
        kind: "parent".into(),
        source: "stage/child".into(),
        target: "stage/root".into(),
    }];
    world.scenes = vec![Scene {
        id: "stage".into(),
        entities: vec!["stage/root".into(), "stage/child".into()],
        active: false,
    }];
    Store::memory_with(Registry::core(), world).unwrap()
}

#[test]
fn scene_tree_is_compact_filterable_and_revision_bound() {
    let mut store = fixture();
    let page = store
        .execute(Request::SceneTree {
            query: TreeQuery {
                limit: Some(1),
                max_bytes: Some(256),
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(page["objects"][0]["id"], "loose");
    assert_eq!(page["objects"][0]["active"], true);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 256);
    let cursor = page["next_cursor"].as_str().unwrap().to_owned();
    let q = Request::SceneTree {
        query: TreeQuery {
            limit: Some(1),
            cursor: Some(cursor),
            ..Default::default()
        },
    };
    let page = store.execute(q.clone()).unwrap();
    assert_eq!(page["objects"][0]["parent"], "stage/root");
    assert_eq!(page["objects"][0]["label"], "Cargo");
    assert_eq!(page["objects"][0]["active"], false);
    assert!(page["objects"][0].get("components").is_none());

    let filtered = store.execute(parse_request(r#"{"op":"scene_tree","query":{"scene":"stage","parent":"stage/root","text":"cargo"}}"#).unwrap()).unwrap();
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["objects"][0]["id"], "stage/child");
    store
        .apply(
            Some(1),
            vec![Edit::Core(CoreEdit::SetField {
                target: Target::Entity {
                    id: "stage/child".into(),
                },
                field: "/label".into(),
                value: json!("Renamed Cargo"),
            })],
        )
        .unwrap();
    let error = axiom_core::error_response(&store.execute(q).unwrap_err());
    assert_eq!(error["diagnostics"][0]["code"], "stale_cursor");
    let selected = store.execute(parse_request(r#"{"op":"select","query":{"from":"entities","ids":["stage/child"],"fields":["label"]}}"#).unwrap()).unwrap();
    assert!(selected.to_string().contains("Renamed Cargo"));
}

#[test]
fn scene_reparent_preserves_pose_and_reports_stable_paths() {
    let mut store = fixture();
    let edit = Edit::Core(CoreEdit::Reparent {
        entity: "stage/child".into(),
        parent: None,
        keep_world: true,
    });
    let preview = store
        .execute(Request::Preview {
            expected_rev: Some(1),
            edits: vec![edit.clone()],
        })
        .unwrap();
    assert_eq!(store.revision().unwrap(), 1);
    let changes = preview["diff"]["changes"].as_array().unwrap();
    assert!(changes
        .iter()
        .any(|c| c["path"] == "/relations/parent/@stage~1child"));
    let apply = store.apply(Some(1), vec![edit]).unwrap();
    assert_eq!(preview["diff"], apply["diff"]);
    let (_, world) = store.current().unwrap();
    assert_eq!(
        world.entity("stage/child").unwrap().components["core.transform"],
        json!({"position":[5.0,2.0,3.0]})
    );
    let compiled = world.compile(&Registry::core()).unwrap();
    let slot = compiled.indices["stage/child"];
    assert_eq!(
        compiled.entities[slot].transform.as_ref().unwrap().position,
        [5.0, 2.0, 3.0]
    );
    assert_eq!(compiled.parents[slot], None);

    let edit = |entity: &str, parent: &str| {
        Edit::Core(CoreEdit::Reparent {
            entity: entity.into(),
            parent: Some(parent.into()),
            keep_world: false,
        })
    };
    let error = store
        .apply(
            Some(2),
            vec![
                edit("stage/child", "stage/root"),
                edit("stage/root", "stage/child"),
            ],
        )
        .unwrap_err();
    let diagnostic = &axiom_core::error_response(&error)["diagnostics"][0];
    assert_eq!(diagnostic["code"], "parent_cycle");
    assert_eq!(diagnostic["path"], "/relations/parent/@stage~1root");
    assert_eq!(store.revision().unwrap(), 2);
}

#[test]
fn duplicate_and_delete_preserve_graph_consistency_and_retry_receipts() {
    let mut store = fixture();
    let request = Request::Execute {
        request_id: "copy-props".into(),
        runtime_session: None,
        request: Box::new(Request::Apply {
            expected_rev: Some(1),
            edits: vec![Edit::Core(CoreEdit::DuplicateEntity {
                source: "stage/child".into(),
                id: "stage/copy".into(),
            })],
        }),
    };
    assert_eq!(
        store.execute(request.clone()).unwrap(),
        store.execute(request).unwrap()
    );
    let (_, world) = store.current().unwrap();
    assert_eq!(
        world.entity("stage/copy").unwrap().prefab.as_deref(),
        Some("props")
    );
    assert!(world.scenes[0].entities.contains(&"stage/copy".into()));
    assert!(world
        .relations
        .iter()
        .any(|r| r.source == "stage/copy" && r.target == "stage/root"));
    let result = store
        .apply(
            Some(2),
            vec![Edit::Core(CoreEdit::DeleteEntity {
                id: "stage/root".into(),
            })],
        )
        .unwrap();
    let paths = result["diff"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"/scenes/@stage/entities/@stage~1root"));
    let (_, world) = store.current().unwrap();
    assert!(world.relations.is_empty());
    assert_eq!(world.scenes[0].entities, ["stage/child", "stage/copy"]);
    let result = store
        .apply(
            Some(3),
            vec![Edit::Core(CoreEdit::SetSceneMembership {
                scene: "stage".into(),
                entity: "stage/copy".into(),
                member: true,
            })],
        )
        .unwrap();
    assert_eq!(result["changed"], Value::Bool(false));
    assert_eq!(result["rev"], 3);
}
