//! Headless editor operations: `axiom_core::ops` and their ACL verbs, with no editor running.
use axiom_core::{error_response, ops, Camera, Edit, Registry, Request, Store, World};
use serde_json::Value;

fn lab() -> World {
    serde_json::from_str(include_str!("../../../content/editor-lab/world.json")).unwrap()
}

fn store() -> Store {
    Store::memory_with(Registry::core(), lab()).unwrap()
}

fn acl_mode(store: &mut Store, text: &str, preview: bool, propose: bool) -> Result<Value, Value> {
    store
        .execute(Request::Acl {
            text: text.into(),
            preview,
            expected_rev: None,
            propose,
        })
        .map_err(|e| error_response(&e))
}

fn acl(store: &mut Store, text: &str) -> Result<String, Value> {
    acl_mode(store, text, false, false).map(|v| v["text"].as_str().unwrap().to_owned())
}

fn code(error: Value) -> String {
    error["diagnostics"][0]["code"].as_str().unwrap().to_owned()
}

fn apply(edits: Vec<Edit>) -> World {
    let mut store = store();
    store
        .execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
    store.current().unwrap().1
}

fn camera(world: &World, id: &str) -> Camera {
    serde_json::from_value(world.entity(id).unwrap().components["render.camera"].clone()).unwrap()
}

#[test]
fn acl_verbs_compile_to_the_shared_ops() {
    let start = lab();
    let cases: Vec<(&str, Vec<Edit>)> = vec![
        (
            "create cube id=box at=1,2,3 parent=stage/rack scene=staging",
            ops::create_object(
                &start,
                "cube",
                &ops::Placement {
                    id: Some("box".into()),
                    position: Some([1.0, 2.0, 3.0]),
                    parent: Some("stage/rack".into()),
                    scene: Some("staging".into()),
                },
            )
            .unwrap()
            .edits,
        ),
        (
            "instantiate cargo-crate at=0,0,2",
            ops::instantiate_prefab(
                &start,
                "cargo-crate",
                &ops::Placement {
                    position: Some([0.0, 0.0, 2.0]),
                    ..Default::default()
                },
            )
            .unwrap()
            .edits,
        ),
        (
            "camera main stage/camera",
            ops::make_main_camera(&start, "stage/camera").unwrap(),
        ),
        (
            "reparent stage/crate stage/rack keep=local",
            ops::reparent(&start, "stage/crate", Some("stage/rack"), false).unwrap(),
        ),
        (
            "scene member staging stage/crate on",
            ops::set_scene_membership(&start, "staging", "stage/crate", true).unwrap(),
        ),
    ];
    for (script, edits) in cases {
        let mut s = store();
        acl(&mut s, script).unwrap_or_else(|e| panic!("{script}: {e}"));
        assert_eq!(s.current().unwrap().1, apply(edits), "{script}");
    }
}

#[test]
fn create_with_placement_parents_and_joins_scene() {
    let mut s = store();
    let out = acl(
        &mut s,
        "create cube id=box at=1,2,3 parent=stage/rack scene=staging",
    )
    .unwrap();
    assert!(out.starts_with("ok rev=2"), "{out}");
    let w = s.current().unwrap().1;
    let e = w.entity("box").unwrap();
    assert_eq!(
        e.components["core.transform"]["position"],
        serde_json::json!([1.0, 2.0, 3.0])
    );
    assert!(w
        .relations
        .iter()
        .any(|r| r.kind == "parent" && r.source == "box" && r.target == "stage/rack"));
    assert!(w.scenes[1].entities.contains(&"box".to_owned()));
}

#[test]
fn camera_main_switches_the_active_camera_in_one_revision() {
    let mut s = store();
    // A second camera is created inactive because stage/camera is active.
    acl(&mut s, "create camera id=cam2").unwrap();
    let w = s.current().unwrap().1;
    assert!(!camera(&w, "cam2").active);
    let out = acl(&mut s, "camera main cam2").unwrap();
    assert!(out.starts_with("ok rev=3 changes=2"), "{out}");
    let w = s.current().unwrap().1;
    assert!(camera(&w, "cam2").active);
    assert!(!camera(&w, "stage/camera").active);
    assert_eq!(
        code(acl(&mut s, "camera main stage/floor").unwrap_err()),
        "not_a_camera"
    );
    assert_eq!(
        code(acl(&mut s, "camera main nope").unwrap_err()),
        "not_found"
    );
}

#[test]
fn later_statements_see_earlier_ones() {
    let mut s = store();
    acl(
        &mut s,
        "create cube; create cube; duplicate cube; scene create arena off; scene member arena cube/2 on",
    )
    .unwrap();
    let w = s.current().unwrap().1;
    for id in ["cube", "cube/2", "cube/copy"] {
        assert!(w.entity(id).is_some(), "{id}");
    }
    let arena = w.scenes.iter().find(|s| s.id == "arena").unwrap();
    assert!(!arena.active);
    assert_eq!(arena.entities, ["cube/2"]);
    // Create then delete in the same script nets out to nothing new.
    acl(&mut s, "create empty id=tmp; delete tmp").unwrap();
    assert!(s.current().unwrap().1.entity("tmp").is_none());
}

#[test]
fn scene_lifecycle_and_reparent_round_trip() {
    let mut s = store();
    acl(&mut s, "scene create arena").unwrap();
    acl(&mut s, "scene activate arena off").unwrap();
    acl(&mut s, "reparent stage/crate stage/rack").unwrap();
    let w = s.current().unwrap().1;
    assert!(!w.scenes.iter().find(|s| s.id == "arena").unwrap().active);
    assert!(w
        .relations
        .iter()
        .any(|r| r.source == "stage/crate" && r.target == "stage/rack"));
    acl(&mut s, "reparent stage/crate root; scene delete arena").unwrap();
    let w = s.current().unwrap().1;
    assert!(!w.relations.iter().any(|r| r.source == "stage/crate"));
    assert!(w.scenes.iter().all(|s| s.id != "arena"));
    assert_eq!(
        code(acl(&mut s, "scene delete arena").unwrap_err()),
        "not_found"
    );
    assert_eq!(
        code(acl(&mut s, "scene create staging").unwrap_err()),
        "id_taken"
    );
    assert_eq!(
        code(acl(&mut s, "reparent stage/rack stage/rack/cargo").unwrap_err()),
        "parent_cycle"
    );
}

#[test]
fn preview_and_propose_leave_the_revision_alone() {
    let mut s = store();
    let preview = acl_mode(&mut s, "create cube; camera main stage/camera", true, false).unwrap();
    assert!(
        preview["text"]
            .as_str()
            .unwrap()
            .starts_with("preview rev=1"),
        "{preview}"
    );
    let proposal = acl_mode(&mut s, "duplicate stage/crate", false, true).unwrap();
    assert!(proposal["text"]
        .as_str()
        .unwrap()
        .starts_with("proposal #1 pending base_rev=1"));
    assert_eq!(s.current().unwrap().0, 1);
}

#[test]
fn bad_input_reports_structured_errors() {
    let mut s = store();
    for (script, expected) in [
        ("create light", "unknown_preset"),
        ("create cube id=stage/rack", "id_taken"),
        ("create cube scene=nope", "not_found"),
        ("create cube parent=nope", "not_found"),
        ("instantiate nope", "not_found"),
        ("duplicate nope/*", "no_match"),
        ("create cube colour=red", "acl_syntax"),
        ("scene rename a b", "acl_syntax"),
        ("camera follow a", "acl_syntax"),
    ] {
        assert_eq!(code(acl(&mut s, script).unwrap_err()), expected, "{script}");
    }
    assert_eq!(s.current().unwrap().0, 1, "failed scripts never commit");
}
