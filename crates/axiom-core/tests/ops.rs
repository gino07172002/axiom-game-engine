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
            "create light spot at=1,4,0",
            ops::create_object(
                &start,
                "light.spot",
                &ops::Placement {
                    position: Some([1.0, 4.0, 0.0]),
                    ..Default::default()
                },
            )
            .unwrap()
            .edits,
        ),
        (
            "create light.area id=panel",
            ops::create_object(
                &start,
                "light.area",
                &ops::Placement {
                    id: Some("panel".into()),
                    ..Default::default()
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
        (
            "rename stage/crate \"信號 Crate\"",
            ops::rename(&start, "stage/crate", "信號 Crate").unwrap(),
        ),
        (
            "move stage/camera before stage/floor",
            ops::move_entity(
                &start,
                "stage/camera",
                &ops::Order::Before("stage/floor".into()),
            )
            .unwrap(),
        ),
        (
            "move stage/floor after stage/beacon",
            ops::move_entity(
                &start,
                "stage/floor",
                &ops::Order::After("stage/beacon".into()),
            )
            .unwrap(),
        ),
        (
            "prefab create beacon-kit from stage/beacon",
            ops::create_prefab(&start, Some("beacon-kit"), "stage/beacon")
                .unwrap()
                .edits,
        ),
        (
            "ground entities[id=stage/crate,id=stage/rack]",
            ops::drop_to_ground(&start, &["stage/crate".into(), "stage/rack".into()]).unwrap(),
        ),
        (
            "duplicate entities[id=stage/crate,id=stage/floor]",
            ops::duplicate(&start, &["stage/crate".into(), "stage/floor".into()])
                .unwrap()
                .edits,
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
        ("create light", "acl_syntax"),
        ("create light laser", "unknown_preset"),
        ("create light point id=bad!id", "invalid_id"),
        ("create trigger", "acl_syntax"),
        ("create trigger cone", "unknown_preset"),
        ("remove stage/crate", "acl_syntax"),
        ("remove nope/* render.mesh", "no_match"),
        ("remove stage/crate render.mesh", "not_found"),
        ("remove stage/floor render.camera", "not_found"),
        ("create cube id=stage/rack", "id_taken"),
        ("create cube scene=nope", "not_found"),
        ("create cube parent=nope", "not_found"),
        ("instantiate nope", "not_found"),
        ("duplicate nope/*", "no_match"),
        ("ground nope/*", "no_match"),
        ("ground a b", "acl_syntax"),
        ("create cube colour=red", "acl_syntax"),
        ("scene rename a b", "acl_syntax"),
        ("camera follow a", "acl_syntax"),
        ("rename nope x", "not_found"),
        ("rename stage/crate", "acl_syntax"),
        ("move stage/crate before stage/crate", "invalid_value"),
        ("move stage/crate sideways", "acl_syntax"),
        ("prefab create cargo-crate from stage/crate", "id_taken"),
        ("prefab create from nope", "not_found"),
        ("prefab make x", "acl_syntax"),
    ] {
        assert_eq!(code(acl(&mut s, script).unwrap_err()), expected, "{script}");
    }
    assert_eq!(s.current().unwrap().0, 1, "failed scripts never commit");
}

#[test]
fn rename_move_and_prefab_create_round_trip() {
    let mut s = store();
    let out = acl(
        &mut s,
        "rename stage/crate \"信號 箱\"\nmove stage/crate end\nprefab create from stage/beacon",
    )
    .unwrap();
    assert!(out.starts_with("ok rev=2"), "{out}");
    let w = s.current().unwrap().1;
    assert_eq!(
        w.entity("stage/crate").unwrap().label.as_deref(),
        Some("信號 箱")
    );
    assert_eq!(w.entities.last().unwrap().id, "stage/crate");
    let beacon = w.entity("stage/beacon").unwrap();
    assert_eq!(beacon.prefab.as_deref(), Some("stage/beacon"));
    assert!(w.prefabs["stage/beacon"]
        .components
        .contains_key("render.mesh"));
    // The resolved look is unchanged, so the world still compiles to the same meshes.
    assert_eq!(
        w.resolved_components(beacon),
        lab().resolved_components(lab().entity("stage/beacon").unwrap())
    );
    // An empty label clears; a second prefab from the same entity gets /2.
    acl(
        &mut s,
        "rename stage/crate \"\"\nprefab create from stage/beacon",
    )
    .unwrap();
    let w = s.current().unwrap().1;
    assert_eq!(w.entity("stage/crate").unwrap().label, None);
    assert_eq!(
        w.entity("stage/beacon").unwrap().prefab.as_deref(),
        Some("stage/beacon/2")
    );
    // Moving to where it already is commits nothing new.
    let out = acl(&mut s, "move stage/crate end").unwrap();
    assert!(out.starts_with("unchanged"), "{out}");
}

#[test]
fn light_presets_create_valid_lights_aimed_along_minus_z() {
    use axiom_core::{LightKind, RenderLight};
    let mut s = store();
    acl(
        &mut s,
        "create light directional\ncreate light point at=2,3,4\ncreate light spot\ncreate light area\ncreate light point",
    )
    .unwrap();
    let world = s.current().unwrap().1;
    let light = |id: &str| -> RenderLight {
        serde_json::from_value(world.entity(id).unwrap().components["render.light"].clone())
            .unwrap()
    };
    assert_eq!(light("light.directional").kind, LightKind::Directional);
    assert!(light("light.directional").shadows);
    assert_eq!(light("light.point").range, Some(6.0));
    assert_eq!(light("light.point/2").kind, LightKind::Point);
    assert!(light("light.spot").spot.is_some());
    assert!(light("light.area").area.is_some());
    assert_eq!(
        world.entity("light.point").unwrap().components["core.transform"]["position"],
        serde_json::json!([2.0, 3.0, 4.0])
    );
    // The directional preset matches the default sun: -Z maps onto -DEFAULT_SUN_TOWARD.
    let t: axiom_core::Transform = serde_json::from_value(
        world.entity("light.directional").unwrap().components["core.transform"].clone(),
    )
    .unwrap();
    let forward = glam::Quat::from_array(t.rotation) * -glam::Vec3::Z;
    let toward = glam::Vec3::from_array(axiom_core::DEFAULT_SUN_TOWARD).normalize();
    assert!((forward + toward).length() < 1e-5, "{forward}");
    // Spot/area presets point straight down.
    let t: axiom_core::Transform = serde_json::from_value(
        world.entity("light.spot").unwrap().components["core.transform"].clone(),
    )
    .unwrap();
    assert!((glam::Quat::from_array(t.rotation) * -glam::Vec3::Z + glam::Vec3::Y).length() < 1e-5);
    // Bad component values are rejected by the transaction, with the reason.
    let error = acl(&mut s, "set light.point render.light/shadows true").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("point light shadows not supported yet"),
        "{error}"
    );
}

#[test]
fn light_budget_is_enforced_at_commit() {
    let mut s = store();
    let script = (0..5)
        .map(|_| "create light directional")
        .collect::<Vec<_>>()
        .join("\n");
    let error = acl(&mut s, &script).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("directional light budget exceeded"),
        "{error}"
    );
    acl(&mut s, "create light directional\ncreate light directional").unwrap();
}

#[test]
fn trigger_presets_create_valid_zones_with_capability_and_system() {
    use axiom_core::trigger::{TriggerShape, TriggerZone};
    let mut s = store();
    acl(
        &mut s,
        "create trigger box at=1,0,2\ncreate trigger sphere\ncreate trigger.box",
    )
    .unwrap();
    let world = s.current().unwrap().1;
    let zone = |id: &str| -> TriggerZone {
        serde_json::from_value(world.entity(id).unwrap().components["trigger.zone"].clone())
            .unwrap()
    };
    assert_eq!(zone("trigger.box").shape, TriggerShape::Box);
    assert_eq!(zone("trigger.box").half_extents, Some([1.0; 3]));
    assert_eq!(zone("trigger.sphere").radius, Some(1.5));
    assert_eq!(zone("trigger.box/2").filter.tags, ["player"]);
    assert_eq!(
        world.entity("trigger.box").unwrap().components["core.transform"]["position"],
        serde_json::json!([1.0, 0.0, 2.0])
    );
    assert_eq!(world.capabilities.get("trigger"), Some(&1));
    assert_eq!(
        world
            .systems
            .iter()
            .filter(|s| *s == "trigger.detect")
            .count(),
        1,
        "the system is added once"
    );
    // ACL statements compile to the shared op.
    let start = lab();
    let expected = ops::create_object(&start, "trigger.sphere", &ops::Placement::default())
        .unwrap()
        .edits;
    let mut fresh = store();
    assert_eq!(
        acl_mode(&mut fresh, "create trigger sphere", true, false).unwrap()["text"],
        acl_mode(&mut fresh, "create trigger.sphere", true, false).unwrap()["text"]
    );
    assert_eq!(apply(expected), {
        acl(&mut fresh, "create trigger sphere").unwrap();
        fresh.current().unwrap().1
    });
    // Invalid field values are rejected by the transaction with the reason; nothing changes.
    let rev = s.current().unwrap().0;
    let error = acl(&mut s, "set trigger.sphere trigger.zone/radius -2").unwrap_err();
    assert!(
        error["error"].as_str().unwrap().contains("sphere radius"),
        "{error}"
    );
    let error = acl(
        &mut s,
        "set trigger.box trigger.zone/on_enter missing_event",
    )
    .unwrap_err();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("does not declare"),
        "{error}"
    );
    assert_eq!(code(error), "invalid_trigger");
    assert_eq!(s.current().unwrap().0, rev);
    // Valid edits apply.
    acl(&mut s, "set trigger.box trigger.zone/exit_margin 0.25").unwrap();
}

#[test]
fn remove_verb_matches_the_op_and_reports_inherited_components() {
    let start = lab();
    let ids = vec!["stage/floor".to_owned(), "stage/beacon".to_owned()];
    let edits = ops::remove_component(&start, &ids, "render.mesh").unwrap();
    let mut s = store();
    acl(
        &mut s,
        "remove entities[id=stage/floor,id=stage/beacon] render.mesh",
    )
    .unwrap();
    let world = s.current().unwrap().1;
    assert_eq!(world, apply(edits));
    assert!(!world
        .entity("stage/floor")
        .unwrap()
        .components
        .contains_key("render.mesh"));
    // A prefab-inherited component is explained, not silently skipped.
    let error = acl(&mut s, "remove stage/crate render.mesh").unwrap_err();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("inherits render.mesh"),
        "{error}"
    );
}
