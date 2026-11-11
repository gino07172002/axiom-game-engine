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
                    ..Default::default()
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

#[test]
fn path_verbs_match_the_ops_and_report_structured_errors() {
    use axiom_core::path::{PathEnd, PathFollow, PathSpline};
    let mut s = store();
    acl(
        &mut s,
        "create path id=road\ncreate cube id=car\ncreate cube id=bus",
    )
    .unwrap();
    let start = s.current().unwrap().1;
    let after = |world: &World, text: &str| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        acl(&mut t, text).unwrap_or_else(|e| panic!("{text}: {e}"));
        t.current().unwrap().1
    };
    let applied = |world: &World, edits: Vec<Edit>| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        t.execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
        t.current().unwrap().1
    };
    // follow: options parse into the shared op.
    let options = ops::FollowOptions {
        speed: Some(3.0),
        end: Some(PathEnd::PingPong),
        offset: Some(-1.5),
    };
    let expected = ops::follow_path(&start, &["car".into()], "road", &options).unwrap();
    let followed = after(&start, "follow car road speed=3 end=ping_pong offset=-1.5");
    assert_eq!(followed, applied(&start, expected));
    let follow: PathFollow =
        serde_json::from_value(followed.entity("car").unwrap().components["path.follow"].clone())
            .unwrap();
    assert_eq!(
        (follow.speed, follow.end, follow.offset),
        (3.0, PathEnd::PingPong, -1.5)
    );
    // path ID verbs: the shared op, default positions for add / insert without coordinates.
    for (text, edit) in [
        (
            "path road add 0,1,9",
            ops::PointEdit::Add(Some([0.0, 1.0, 9.0])),
        ),
        ("path road add", ops::PointEdit::Add(None)),
        ("path road insert 1", ops::PointEdit::Insert(1, None)),
        (
            "path road insert 0 [1,2,3]",
            ops::PointEdit::Insert(0, Some([1.0, 2.0, 3.0])),
        ),
        (
            "path road move 3 -4,0,4",
            ops::PointEdit::Move(3, [-4.0, 0.0, 4.0]),
        ),
        ("path road remove 2", ops::PointEdit::Remove(2)),
    ] {
        let expected = ops::edit_path_point(&start, "road", &edit).unwrap();
        assert_eq!(after(&start, text), applied(&start, expected), "{text}");
    }
    let spline = |world: &World| -> PathSpline {
        serde_json::from_value(world.entity("road").unwrap().components["path.spline"].clone())
            .unwrap()
    };
    // Closed rectangle: add splits the closing piece; insert 1 splits piece 0 -> 1.
    assert_eq!(
        spline(&after(&start, "path road add")).points[4],
        [-4.0, 0.0, 0.0]
    );
    assert_eq!(
        spline(&after(&start, "path road insert 1")).points[1],
        [0.0, 0.0, -3.0]
    );
    // Open path: add extends the last piece by its own length.
    let open = after(&start, "set road path.spline/closed false");
    assert_eq!(
        spline(&after(&open, "path road add")).points[4],
        [-12.0, 0.0, 3.0]
    );
    // Structured errors.
    let mut e = Store::memory_with(Registry::core(), start.clone()).unwrap();
    assert_eq!(
        code(acl(&mut e, "follow car nowhere").unwrap_err()),
        "not_found"
    );
    assert_eq!(
        code(acl(&mut e, "follow car bus").unwrap_err()),
        "not_a_path"
    );
    assert_eq!(
        code(acl(&mut e, "follow road road").unwrap_err()),
        "invalid_value"
    );
    assert_eq!(
        code(acl(&mut e, "follow car road speed=500").unwrap_err()),
        "invalid_value"
    );
    assert_eq!(
        code(acl(&mut e, "follow car road speed=fast").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(
        code(acl(&mut e, "follow car road end=reverse").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(
        code(acl(&mut e, "path road move 9 1,0,0").unwrap_err()),
        "invalid_value"
    );
    assert_eq!(
        code(acl(&mut e, "path road move 0 4,0,-3").unwrap_err()),
        "invalid_path"
    );
    assert_eq!(code(acl(&mut e, "path car add").unwrap_err()), "not_a_path");
    assert_eq!(
        code(acl(&mut e, "path road twist 1").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(
        code(acl(&mut e, "path road add 1,2").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(
        code(acl(&mut e, "path road remove 0 1").unwrap_err()),
        "acl_syntax"
    );
    acl(&mut e, "path road remove 0").unwrap();
    assert_eq!(
        code(acl(&mut e, "path road remove 0").unwrap_err()),
        "invalid_path"
    );
    // A second mover on a follower is refused at commit with path_conflict.
    acl(&mut e, "follow car road").unwrap();
    let mut capabilities = start.capabilities.clone();
    capabilities.insert("motion".into(), 1);
    e.execute(Request::Apply {
        expected_rev: None,
        edits: vec![Edit::set("/capabilities", serde_json::json!(capabilities))],
    })
    .unwrap();
    let conflict = acl(&mut e, "set car core.motion {\"velocity\":[1,0,0]}").unwrap_err();
    assert_eq!(code(conflict.clone()), "path_conflict", "{conflict}");
}

#[test]
fn nav_verbs_match_the_ops_and_report_structured_errors() {
    use axiom_core::navigation::{NavAgent, NavTarget};
    let mut s = store();
    acl(
        &mut s,
        "create cube id=bob\ncreate cube id=ann\ncreate empty id=door at=4,0,2",
    )
    .unwrap();
    let start = s.current().unwrap().1;
    let after = |world: &World, text: &str| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        acl(&mut t, text).unwrap_or_else(|e| panic!("{text}: {e}"));
        t.current().unwrap().1
    };
    let applied = |world: &World, edits: Vec<Edit>| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        t.execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
        t.current().unwrap().1
    };
    // nav SEL agent: the shared op; capability and system are added once.
    let options = ops::NavOptions {
        speed: Some(2.5),
        radius: Some(0.4),
    };
    let ids = ["bob".to_owned(), "ann".to_owned()];
    let expected = ops::add_nav_agent(&start, &ids, &options).unwrap();
    let agents = after(
        &start,
        "nav entities[id=bob,id=ann] agent speed=2.5 radius=0.4",
    );
    assert_eq!(agents, applied(&start, expected));
    assert_eq!(agents.capabilities.get("nav"), Some(&1));
    assert_eq!(
        agents.systems.iter().filter(|s| *s == "nav.steer").count(),
        1
    );
    let agent = |world: &World, id: &str| -> NavAgent {
        serde_json::from_value(world.entity(id).unwrap().components["nav.agent"].clone()).unwrap()
    };
    assert_eq!(
        (agent(&agents, "bob").speed, agent(&agents, "bob").radius),
        (2.5, 0.4)
    );
    // nav SEL target: point, entity, none.
    for (text, target) in [
        (
            "nav bob target 3,0,-2",
            Some(NavTarget::Point([3.0, 0.0, -2.0])),
        ),
        (
            "nav bob target [3,0,-2]",
            Some(NavTarget::Point([3.0, 0.0, -2.0])),
        ),
        (
            "nav entities[id=bob,id=ann] target door",
            Some(NavTarget::Entity("door".into())),
        ),
        ("nav bob target none", None),
    ] {
        let ids: Vec<String> = if text.contains("ann") {
            vec!["bob".into(), "ann".into()]
        } else {
            vec!["bob".into()]
        };
        let expected = ops::set_nav_target(&agents, &ids, target.as_ref()).unwrap();
        let got = after(&agents, text);
        assert_eq!(got, applied(&agents, expected), "{text}");
        assert_eq!(agent(&got, "bob").target, target, "{text}");
    }
    // create obstacle: a cube with a matching nav.blocker.
    let made = after(&start, "create obstacle at=2,0.5,0");
    let blocker = &made.entity("obstacle").unwrap().components["nav.blocker"];
    assert_eq!(blocker["half"], serde_json::json!([0.5, 0.5]));
    assert!(made
        .entity("obstacle")
        .unwrap()
        .components
        .contains_key("render.mesh"));
    // Structured errors.
    let mut e = Store::memory_with(Registry::core(), agents.clone()).unwrap();
    assert_eq!(
        code(acl(&mut e, "nav door target bob").unwrap_err()),
        "not_a_nav_agent"
    );
    assert_eq!(
        code(acl(&mut e, "nav bob target ghost").unwrap_err()),
        "not_found"
    );
    assert_eq!(
        code(acl(&mut e, "nav bob target bob").unwrap_err()),
        "invalid_value"
    );
    assert_eq!(
        code(acl(&mut e, "nav bob target 1,2").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(
        code(acl(&mut e, "nav bob agent speed=99").unwrap_err()),
        "invalid_value"
    );
    assert_eq!(
        code(acl(&mut e, "nav bob agent speed=fast").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(
        code(acl(&mut e, "nav bob agent fly=1").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(code(acl(&mut e, "nav bob walk").unwrap_err()), "acl_syntax");
    assert_eq!(
        code(acl(&mut e, "nav nobody agent").unwrap_err()),
        "no_match"
    );
    // A point target with follow_target on is refused (follow_target is for entities).
    acl(
        &mut e,
        "nav bob target door\nset bob nav.agent/follow_target true",
    )
    .unwrap();
    assert_eq!(
        code(acl(&mut e, "nav bob target 1,0,1").unwrap_err()),
        "invalid_value"
    );
    // A second mover on an agent is refused at commit with nav_conflict.
    let mut capabilities = agents.capabilities.clone();
    capabilities.insert("motion".into(), 1);
    e.execute(Request::Apply {
        expected_rev: None,
        edits: vec![Edit::set("/capabilities", serde_json::json!(capabilities))],
    })
    .unwrap();
    let conflict = acl(&mut e, "set ann core.motion {\"velocity\":[1,0,0]}").unwrap_err();
    assert_eq!(code(conflict.clone()), "nav_conflict", "{conflict}");
}

#[test]
fn interact_verb_matches_the_op_and_wires_events_and_bindings() {
    use axiom_core::interact::InteractTarget;
    let mut s = store();
    acl(&mut s, "create cube id=npc\ncreate cube id=door at=4,0,2").unwrap();
    let start = s.current().unwrap().1;
    let after = |world: &World, text: &str| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        acl(&mut t, text).unwrap_or_else(|e| panic!("{text}: {e}"));
        t.current().unwrap().1
    };
    let applied = |world: &World, edits: Vec<Edit>| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        t.execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
        t.current().unwrap().1
    };
    let target = |world: &World, id: &str| -> InteractTarget {
        serde_json::from_value(world.entity(id).unwrap().components["interact.target"].clone())
            .unwrap()
    };
    // Defaults: event `interacted` declared (and behavior.program created), action interact
    // bound to KeyE, capabilities and systems added once.
    let ids = ["npc".to_owned(), "door".to_owned()];
    let expected = ops::make_interactable(&start, &ids, &ops::InteractOptions::default()).unwrap();
    let made = after(&start, "interact entities[id=npc,id=door]");
    assert_eq!(made, applied(&start, expected));
    assert_eq!(target(&made, "door").event, "interacted");
    assert_eq!(target(&made, "door").interactor.tags, ["player"]);
    assert_eq!(
        made.resources["behavior.program"]["events"]["interacted"],
        serde_json::json!({"fields":{"interactor":"text","target":"text"}})
    );
    assert_eq!(
        made.resources["input.bindings"]["buttons"]["interact"],
        "KeyE"
    );
    for (capability, system) in [
        ("interact", "interact.focus"),
        ("behavior", "behavior.execute"),
    ] {
        assert_eq!(made.capabilities.get(capability), Some(&1));
        assert_eq!(made.systems.iter().filter(|s| *s == system).count(), 1);
    }
    // Options (quoted CJK prompt); an existing program keeps its events and gains the new one.
    let options = ops::InteractOptions {
        event: Some("open_door".into()),
        prompt: Some("開門".into()),
        range: Some(1.5),
        action: Some("interact".into()),
    };
    let expected = ops::make_interactable(&made, &["door".to_owned()], &options).unwrap();
    let door = after(
        &made,
        "interact door event=open_door prompt=\"開門\" range=1.5 action=interact",
    );
    assert_eq!(door, applied(&made, expected));
    let t = target(&door, "door");
    assert_eq!(
        (t.event.as_str(), t.prompt.as_str(), t.range),
        ("open_door", "開門", 1.5)
    );
    let events = door.resources["behavior.program"]["events"]
        .as_object()
        .unwrap();
    assert!(events.contains_key("interacted") && events.contains_key("open_door"));
    // Field edits are plain `set`.
    let edited = after(&door, "set door interact.target/once true");
    assert!(target(&edited, "door").once);
    // Structured errors.
    let mut e = Store::memory_with(Registry::core(), door.clone()).unwrap();
    for (text, expect) in [
        ("interact door range=500", "invalid_value"),
        ("interact door range=far", "acl_syntax"),
        ("interact door color=red", "acl_syntax"),
        ("interact door event=\"bad event\"", "invalid_value"),
        ("interact nobody", "no_match"),
        // A new button while KeyE is taken by `interact`.
        ("interact door action=use", "missing_binding"),
    ] {
        assert_eq!(code(acl(&mut e, text).unwrap_err()), expect, "{text}");
    }
    // A target without a transform is refused when the batch compiles.
    acl(&mut e, "create empty id=ghost").unwrap();
    acl(&mut e, "remove ghost core.transform").unwrap();
    let error = acl(&mut e, "interact ghost").unwrap_err();
    assert!(error.to_string().contains("core.transform"), "{error}");
}

#[test]
fn tween_verbs_match_the_ops_and_record_keys() {
    use axiom_core::tween::{Ease, TweenClips, TweenMode};
    let mut s = store();
    acl(&mut s, "create cube id=door at=1,0,2\ncreate empty id=door/hinge parent=door\ncreate light point id=lamp").unwrap();
    let start = s.current().unwrap().1;
    let after = |world: &World, text: &str| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        acl(&mut t, text).unwrap_or_else(|e| panic!("{text}: {e}"));
        t.current().unwrap().1
    };
    let applied = |world: &World, edits: Vec<Edit>| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        t.execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
        t.current().unwrap().1
    };
    let clips = |world: &World, id: &str| -> TweenClips {
        serde_json::from_value(world.entity(id).unwrap().components["tween.clips"].clone()).unwrap()
    };
    // Add a clip: the first track records the current value at 0 and at the end; the
    // capability and system are added once.
    let ids = ["door".to_owned()];
    let options = ops::TweenClipOptions {
        property: Some("core.transform/yaw".into()),
        target: Some("door/hinge".into()),
        on_finish: Some("opened".into()),
        ..Default::default()
    };
    let expected = ops::add_tween_clip(&start, &ids, "open", &options).unwrap();
    let w = after(
        &start,
        "tween door clip open property=core.transform/yaw target=door/hinge on_finish=opened",
    );
    assert_eq!(w, applied(&start, expected));
    let c = &clips(&w, "door").clips["open"];
    assert_eq!(c.tracks[0].target, "door/hinge");
    assert_eq!(c.tracks[0].keys.len(), 2);
    assert_eq!(c.tracks[0].keys[1].t, 1.0);
    assert_eq!(c.on_finish, "opened");
    assert_eq!(w.capabilities.get("tween"), Some(&1));
    assert_eq!(
        w.systems.iter().filter(|s| *s == "tween.animate").count(),
        1
    );
    assert_eq!(
        w.resources["behavior.program"]["events"]["opened"],
        serde_json::json!({"fields":{"entity":"text","clip":"text"}})
    );
    // Keys: explicit value with ease, record (no value), move, remove; each equals its op.
    let steps: Vec<(&str, ops::KeyEdit)> = vec![
        (
            "tween door key open 0 add 1 90 ease=ease_out",
            ops::KeyEdit::Add {
                time: 1.0,
                value: Some(vec![90.0]),
                ease: Some(Ease::EaseOut),
            },
        ),
        (
            "tween door key open 0 add 0.5",
            ops::KeyEdit::Add {
                time: 0.5,
                value: None,
                ease: None,
            },
        ),
        (
            "tween door key open 0 move 1 0.25",
            ops::KeyEdit::Move {
                index: 1,
                time: 0.25,
            },
        ),
        ("tween door key open 0 remove 1", ops::KeyEdit::Remove(1)),
    ];
    let mut w = w;
    for (text, edit) in steps {
        let expected = ops::edit_tween_key(&w, "door", "open", 0, &edit).unwrap();
        let next = after(&w, text);
        assert_eq!(next, applied(&w, expected), "{text}");
        w = next;
    }
    let keys = &clips(&w, "door").clips["open"].tracks[0].keys;
    assert_eq!(keys.len(), 2);
    assert_eq!(
        (keys[1].t, keys[1].v.as_slice(), keys[1].ease),
        (1.0, &[90.0][..], Ease::EaseOut)
    );
    // Recording at an existing key time replaces its value (the hinge's authored yaw is 0).
    let w = after(&w, "tween door key open 0 add 1");
    assert_eq!(
        clips(&w, "door").clips["open"].tracks[0].keys[1]
            .v
            .as_slice(),
        &[0.0]
    );
    // Clip options and tracks.
    let set = ops::TweenClipOptions {
        mode: Some(TweenMode::PingPong),
        autoplay: Some(true),
        duration: Some(Some(2.0)),
        on_finish: Some(String::new()),
        ..Default::default()
    };
    let expected = ops::set_tween_clip(&w, "door", "open", &set).unwrap();
    let w2 = after(
        &w,
        "tween door clip open set mode=ping_pong autoplay=on duration=2 on_finish=\"\"",
    );
    assert_eq!(w2, applied(&w, expected));
    // Keeping on_finish with ping_pong is refused (it never finishes).
    let error = ops::set_tween_clip(
        &w,
        "door",
        "open",
        &ops::TweenClipOptions {
            mode: Some(TweenMode::Loop),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("on_finish only fires"),
        "{error:#}"
    );
    let c = &clips(&w2, "door").clips["open"];
    assert_eq!(
        (c.mode, c.autoplay, c.duration, c.on_finish.as_str()),
        (TweenMode::PingPong, true, Some(2.0), "")
    );
    let expected = ops::add_tween_track(&w2, "door", "open", "core.transform/scale", "").unwrap();
    let w3 = after(&w2, "tween door track open add core.transform/scale");
    assert_eq!(w3, applied(&w2, expected));
    let t = &clips(&w3, "door").clips["open"].tracks[1];
    assert_eq!(
        (t.keys[0].t, t.keys[1].t),
        (0.0, 2.0),
        "new tracks span the clip"
    );
    let w4 = after(&w3, "tween door track open remove 1");
    assert_eq!(clips(&w4, "door"), clips(&w2, "door"));
    // A second clip on another entity, autoplay moves; remove clips.
    let w5 = after(
        &w4,
        "tween lamp clip glow property=render.light/intensity duration=auto mode=loop",
    );
    assert_eq!(clips(&w5, "lamp").clips["glow"].duration, None);
    let w6 = after(
        &w5,
        "tween door clip close property=core.transform/yaw target=door/hinge autoplay=on",
    );
    let d = clips(&w6, "door");
    assert!(
        d.clips["close"].autoplay && !d.clips["open"].autoplay,
        "one autoplay clip"
    );
    let w7 = after(
        &w6,
        "tween door clip close remove; tween door clip open remove",
    );
    assert!(!w7
        .entity("door")
        .unwrap()
        .components
        .contains_key("tween.clips"));
    // Structured errors.
    let mut e = Store::memory_with(Registry::core(), w6.clone()).unwrap();
    for (text, expect) in [
        ("tween door clip open", "clip_exists"),
        ("tween door clip nope set mode=loop", "not_found"),
        (
            "tween door clip x property=render.fog/color",
            "invalid_value",
        ),
        (
            "tween door clip x property=render.light/color",
            "invalid_tween",
        ),
        ("tween door clip x target=nobody", "invalid_tween"),
        ("tween door clip \"bad name\"", "invalid_value"),
        ("tween door clip x mode=sometimes", "acl_syntax"),
        ("tween door clip x color=red", "acl_syntax"),
        ("tween door key open 0 add 9999", "invalid_value"),
        ("tween door key open 0 add 3", "invalid_tween"),
        ("tween door key open 5 add 1", "invalid_value"),
        ("tween door key open 0 move 0 1", "invalid_value"),
        ("tween door key open 0 move 7 1", "invalid_value"),
        ("tween door key open 0 add 1 1,2,3", "invalid_tween"),
        ("tween door key open 0 add x", "acl_syntax"),
        ("tween door track open remove 0", "invalid_value"),
        ("tween door bounce", "acl_syntax"),
        ("tween ghost clip a", "no_match"),
        ("tween lamp key glow 0 add 0.5 ease=wobbly", "acl_syntax"),
    ] {
        assert_eq!(code(acl(&mut e, text).unwrap_err()), expect, "{text}");
    }
}

#[test]
fn acl_scripts_may_start_with_a_byte_order_mark() {
    let mut s = store();
    let out = acl(
        &mut s,
        "\u{feff}create cube id=bom\nrename bom \"BOM 測試\"",
    )
    .unwrap();
    assert!(out.starts_with("ok rev="), "{out}");
    let world = s.current().unwrap().1;
    assert_eq!(
        world.entity("bom").unwrap().label.as_deref(),
        Some("BOM 測試")
    );
    // A BOM anywhere else is still an error (only the file's leading mark is stripped).
    let error = acl(&mut s, "create cube id=x\n\u{feff}create cube id=y").unwrap_err();
    assert_eq!(code(error), "acl_syntax");
}

#[test]
fn tween_play_on_wires_runtime_bindings_and_remove_unwires() {
    // runtime.bindings is a runtime resource; register a stand-in like installed_registry does.
    let mut registry = Registry::core();
    registry.resource(
        "runtime.bindings",
        "core",
        serde_json::json!({}),
        |_| Ok(()),
    );
    let mut s = Store::memory_with(registry.clone(), lab()).unwrap();
    acl(
        &mut s,
        "create cube id=door\ncreate cube id=gate at=3,0,0\ninteract door event=use_door",
    )
    .unwrap();
    let start = s.current().unwrap().1;
    let after = |world: &World, text: &str| {
        let mut t = Store::memory_with(registry.clone(), world.clone()).unwrap();
        acl(&mut t, text).unwrap_or_else(|e| panic!("{text}: {e}"));
        t.current().unwrap().1
    };
    let applied = |world: &World, edits: Vec<Edit>| {
        let mut t = Store::memory_with(registry.clone(), world.clone()).unwrap();
        t.execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
        t.current().unwrap().1
    };
    let options = ops::TweenClipOptions {
        property: Some("core.transform/yaw".into()),
        play_on: Some("use_door".into()),
        ..Default::default()
    };
    let ids = ["door".to_owned(), "gate".to_owned()];
    let expected = ops::add_tween_clip(&start, &ids, "open", &options).unwrap();
    let w = after(
        &start,
        "tween entities[id=door,id=gate] clip open property=core.transform/yaw play_on=use_door",
    );
    assert_eq!(w, applied(&start, expected));
    assert_eq!(
        w.resources["runtime.bindings"],
        serde_json::json!({"events":[{"event":"use_door","calls":[
            {"kind":"call","capability":"tween","action":"play","args":{"id":"door","clip":"open"}},
            {"kind":"call","capability":"tween","action":"play","args":{"id":"gate","clip":"open"}}]}]})
    );
    // The interact event keeps its declared fields; a new event is declared without fields.
    assert_eq!(
        w.resources["behavior.program"]["events"]["use_door"],
        serde_json::json!({"fields":{"interactor":"text","target":"text"}})
    );
    let w2 = after(&w, "tween door clip open set play_on=bell");
    assert_eq!(
        w2.resources["behavior.program"]["events"]["bell"],
        serde_json::json!({"fields":{}})
    );
    // Setting the same binding twice keeps one call.
    let w3 = after(&w2, "tween door clip open set play_on=bell");
    assert_eq!(
        w3.resources["runtime.bindings"],
        w2.resources["runtime.bindings"]
    );
    // Removing the clip removes every call that plays it (and emptied bindings).
    let w4 = after(&w3, "tween door clip open remove");
    assert_eq!(
        w4.resources["runtime.bindings"],
        serde_json::json!({"events":[{"event":"use_door","calls":[
            {"kind":"call","capability":"tween","action":"play","args":{"id":"gate","clip":"open"}}]}]})
    );
    let mut e = Store::memory_with(registry.clone(), w.clone()).unwrap();
    assert_eq!(
        code(acl(&mut e, "tween door clip open set play_on=\"bad event\"").unwrap_err()),
        "invalid_value"
    );
}

#[test]
fn audio_verbs_match_the_ops_and_report_structured_errors() {
    use axiom_core::audio::{AudioListener, AudioSource, Rolloff};
    let mut s = store();
    acl(
        &mut s,
        "create cube id=car\ncreate cube id=fountain at=4,0,2\ncreate camera id=cam2",
    )
    .unwrap();
    let start = s.current().unwrap().1;
    let after = |world: &World, text: &str| -> Result<World, Value> {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        acl(&mut t, text)?;
        Ok(t.current().unwrap().1)
    };
    let applied = |world: &World, edits: Vec<Edit>| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        t.execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
        t.current().unwrap().1
    };
    let source = |world: &World, id: &str| -> AudioSource {
        serde_json::from_value(world.entity(id).unwrap().components["audio.source"].clone())
            .unwrap()
    };
    // No library yet: a default source creates clip `tone`, the capability and the system once.
    let ids = ["car".to_owned(), "fountain".to_owned()];
    let expected =
        ops::add_audio_source(&start, &ids, &ops::AudioSourceOptions::default()).unwrap();
    let made = after(&start, "audio entities[id=car,id=fountain] source").unwrap();
    assert_eq!(made, applied(&start, expected));
    assert_eq!(source(&made, "fountain").clip, "tone");
    assert!(made.resources["audio.library"]["clips"]["tone"].is_object());
    assert_eq!(made.capabilities.get("audio"), Some(&1));
    assert_eq!(
        made.systems
            .iter()
            .filter(|s| *s == "audio.spatial")
            .count(),
        1
    );
    // Clips from presets, then a source with every option.
    let expected = ops::add_audio_clip(&made, "engine", "engine").unwrap();
    let clips = after(&made, "audio clip engine engine").unwrap();
    assert_eq!(clips, applied(&made, expected));
    let options = ops::AudioSourceOptions {
        clip: Some("engine".into()),
        volume: Some(0.8),
        looping: Some(true),
        autoplay: Some(true),
        spatial: Some(true),
        min_distance: Some(2.0),
        max_distance: Some(40.0),
        rolloff: Some(Rolloff::Linear),
    };
    let expected = ops::add_audio_source(&clips, &["car".to_owned()], &options).unwrap();
    let car = after(
        &clips,
        "audio car source clip=engine volume=0.8 loop=on autoplay=on spatial=on min=2 max=40 rolloff=linear",
    )
    .unwrap();
    assert_eq!(car, applied(&clips, expected));
    let s = source(&car, "car");
    assert!(s.looping && s.autoplay && s.rolloff == Rolloff::Linear && s.max_distance == 40.0);
    // Choosing a clip = the field edit.
    let expected = ops::set_audio_clip(&car, &["fountain".to_owned()], "engine").unwrap();
    assert_eq!(
        after(&car, "set fountain audio.source/clip \"engine\"").unwrap(),
        applied(&car, expected)
    );
    // One active listener: the second one switches the first off in the same revision.
    let first = after(&car, "audio cam2 listener").unwrap();
    let expected = ops::add_audio_listener(&first, "car").unwrap();
    let second = after(&first, "audio car listener").unwrap();
    assert_eq!(second, applied(&first, expected));
    let listener = |w: &World, id: &str| -> AudioListener {
        serde_json::from_value(w.entity(id).unwrap().components["audio.listener"].clone()).unwrap()
    };
    assert!(listener(&second, "car").active && !listener(&second, "cam2").active);
    // Structured errors.
    assert_eq!(
        code(after(&car, "audio car source clip=nope").unwrap_err()),
        "missing_clip"
    );
    assert_eq!(
        code(after(&car, "audio clip engine tone").unwrap_err()),
        "id_taken"
    );
    assert_eq!(
        code(after(&car, "audio clip x music").unwrap_err()),
        "unknown_preset"
    );
    assert_eq!(
        code(after(&car, "audio ghost source").unwrap_err()),
        "no_match"
    );
    assert_eq!(
        code(after(&car, "audio ghost listener").unwrap_err()),
        "not_found"
    );
    assert_eq!(
        code(after(&car, "audio car source volume=9").unwrap_err()),
        "invalid_value"
    );
    assert_eq!(
        code(after(&car, "audio car speaker").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(
        code(after(&car, "set car audio.source/clip \"nope\"").unwrap_err()),
        "missing_clip",
        "the compile check catches what a raw set writes"
    );
    // Two active listeners written directly are refused at commit.
    assert_eq!(
        code(after(&first, "set car audio.listener {}").unwrap_err()),
        "audio_listener_conflict"
    );
    assert!(ops::add_audio_source(&car, &[], &ops::AudioSourceOptions::default()).is_err());
    assert_eq!(ops::free_audio_clip_id(&car, "engine"), "engine2");
    // Clip IDs that read like the selection keywords still work.
    let keyword = after(&car, "audio clip source tone").unwrap();
    assert!(keyword.resources["audio.library"]["clips"]["source"].is_object());
    assert_eq!(code(after(&car, "audio clip").unwrap_err()), "acl_syntax");
    assert_eq!(ops::free_audio_clip_id(&car, "creak"), "creak");
}

#[test]
fn camera_follow_verb_matches_the_op_and_reports_structured_errors() {
    use axiom_core::camera::CameraFollow;
    let mut s = store();
    acl(
        &mut s,
        "create cube id=hero\ncreate camera id=rig\ncreate camera id=rig2",
    )
    .unwrap();
    let start = s.current().unwrap().1;
    let after = |world: &World, text: &str| -> Result<World, Value> {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        acl(&mut t, text)?;
        Ok(t.current().unwrap().1)
    };
    let applied = |world: &World, edits: Vec<Edit>| {
        let mut t = Store::memory_with(Registry::core(), world.clone()).unwrap();
        t.execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
        t.current().unwrap().1
    };
    let follow = |world: &World, id: &str| -> CameraFollow {
        serde_json::from_value(world.entity(id).unwrap().components["camera.follow"].clone())
            .unwrap()
    };
    // Defaults on two cameras; the capability and the system are added once.
    let ids = ["rig".to_owned(), "rig2".to_owned()];
    let expected =
        ops::add_camera_follow(&start, &ids, "hero", &ops::CameraFollowOptions::default()).unwrap();
    let made = after(&start, "camera entities[id=rig,id=rig2] follow hero").unwrap();
    assert_eq!(made, applied(&start, expected));
    assert_eq!(follow(&made, "rig2"), CameraFollow::new("hero"));
    assert_eq!(made.capabilities.get("camera"), Some(&1));
    assert_eq!(
        made.systems.iter().filter(|s| *s == "camera.rig").count(),
        1
    );
    // Every option.
    let options = ops::CameraFollowOptions {
        distance: Some(4.0),
        min_distance: Some(0.5),
        max_distance: Some(9.0),
        yaw: Some(180.0),
        pitch: Some(10.0),
        look_height: Some(1.2),
        yaw_axis: None,
        zoom_axis: None,
        radius: Some(0.3),
        collision: Some(false),
    };
    let expected = ops::add_camera_follow(&start, &["rig".to_owned()], "hero", &options).unwrap();
    let full = after(
        &start,
        "camera rig follow hero distance=4 min=0.5 max=9 yaw=180 pitch=10 height=1.2 radius=0.3 collide=off",
    )
    .unwrap();
    assert_eq!(full, applied(&start, expected));
    let f = follow(&full, "rig");
    assert!(
        f.distance == 4.0 && f.yaw == 180.0 && !f.collision.enabled && f.collision.radius == 0.3
    );
    // Input axes must be bound when the batch commits.
    assert_eq!(
        code(after(&start, "camera rig follow hero orbit=spin").unwrap_err()),
        "invalid_camera"
    );
    let mut bound = start.clone();
    bound.resources.insert(
        "input.bindings".into(),
        serde_json::json!({"axes":{"spin":{"positive":"KeyC","negative":"KeyZ"}}}),
    );
    let orbiting = after(&bound, "camera rig follow hero orbit=spin zoom=spin").unwrap();
    assert_eq!(follow(&orbiting, "rig").input.yaw_axis, "spin");
    assert_eq!(follow(&orbiting, "rig").input.zoom_axis, "spin");
    // Structured errors.
    assert_eq!(
        code(after(&start, "camera hero follow rig").unwrap_err()),
        "not_a_camera"
    );
    assert_eq!(
        code(after(&start, "camera rig follow ghost").unwrap_err()),
        "not_found"
    );
    assert_eq!(
        code(after(&start, "camera rig follow rig").unwrap_err()),
        "invalid_value"
    );
    assert_eq!(
        code(after(&start, "camera rig follow hero distance=50").unwrap_err()),
        "invalid_value"
    );
    assert_eq!(
        code(after(&start, "camera ghost follow hero").unwrap_err()),
        "no_match"
    );
    assert_eq!(
        code(after(&start, "camera rig chase hero").unwrap_err()),
        "acl_syntax"
    );
    assert_eq!(
        code(after(&start, "camera rig follow hero speed=2").unwrap_err()),
        "acl_syntax"
    );
    // A second mover on the camera is refused at commit.
    let mut motion = made.clone();
    motion.capabilities.insert("motion".into(), 1);
    assert_eq!(
        code(after(&motion, "set rig core.motion {\"velocity\":[1,0,0]}").unwrap_err()),
        "camera_conflict"
    );
    // `camera main` still works next to the new verb, also for a camera named main.
    let main = after(&made, "camera main rig2").unwrap();
    assert!(main.entity("rig2").unwrap().components["render.camera"]["active"] == true);
    let named = after(&start, "create camera id=main\ncamera main follow hero").unwrap();
    assert_eq!(follow(&named, "main").target, "hero");
    assert!(ops::add_camera_follow(&start, &[], "hero", &Default::default()).is_err());
}

#[test]
fn adding_an_audio_clip_keeps_the_other_library_entries_verbatim() {
    let mut world = lab();
    world.capabilities.insert("audio".into(), 1);
    world.resources.insert(
        "audio.library".into(),
        serde_json::json!({"cues":{"pop":{"frequency":520,"duration_ms":60,"gain":0.08}},"clips":{"mini":{"seconds":1}}}),
    );
    let mut store = Store::memory_with(Registry::core(), world.clone()).unwrap();
    store
        .execute(Request::Apply {
            expected_rev: None,
            edits: ops::add_audio_clip(&world, "creak", "creak").unwrap(),
        })
        .unwrap();
    let library = &store.current().unwrap().1.resources["audio.library"];
    assert_eq!(library["clips"]["mini"], serde_json::json!({"seconds":1}));
    assert_eq!(library["cues"], world.resources["audio.library"]["cues"]);
    assert_eq!(library["clips"]["creak"]["wave"], "saw");
}

#[test]
fn world_tag_component_and_rotation_verbs_compile_to_the_shared_ops() {
    let start = lab();
    let r = Registry::core();
    let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let cases: Vec<(&str, Vec<Edit>)> = vec![
        (
            "tag entities[id=stage/crate,id=stage/rack] +player +npc",
            ops::edit_tags(
                &start,
                &ids(&["stage/crate", "stage/rack"]),
                &ids(&["player", "npc"]),
                &[],
            )
            .unwrap(),
        ),
        ("world id yard", ops::set_world_id(&start, "yard").unwrap()),
        (
            "world title \"組裝場 Yard\"",
            ops::set_world_title(&start, "組裝場 Yard").unwrap(),
        ),
        (
            "capability add physics",
            ops::add_capability(&start, &r, "physics").unwrap(),
        ),
        (
            "system add trigger.detect",
            ops::add_system(&start, &r, "trigger.detect").unwrap(),
        ),
        (
            "resource create nav.grid",
            ops::create_resource(&start, &r, "nav.grid", None).unwrap(),
        ),
        (
            "add stage/beacon trigger.zone",
            ops::add_component(&start, &r, &ids(&["stage/beacon"]), "trigger.zone", None).unwrap(),
        ),
        (
            "add stage/beacon core.health {\"value\":5,\"max\":10}",
            ops::add_component(
                &start,
                &r,
                &ids(&["stage/beacon"]),
                "core.health",
                Some(serde_json::json!({"value":5,"max":10})),
            )
            .unwrap(),
        ),
        (
            "rotate stage/crate 0,90,0",
            ops::set_rotation(&start, &ids(&["stage/crate"]), [0.0, 90.0, 0.0]).unwrap(),
        ),
        (
            "create cube id=turned rot=0,45,0",
            ops::create_object(
                &start,
                "cube",
                &ops::Placement {
                    id: Some("turned".into()),
                    rotation: Some([0.0, 45.0, 0.0]),
                    ..Default::default()
                },
            )
            .unwrap()
            .edits,
        ),
        (
            "event declare door_bump who=text",
            ops::declare_event(
                &start,
                "door_bump",
                &[("who".into(), axiom_core::gameplay::ScalarType::Text)],
            )
            .unwrap(),
        ),
    ];
    for (script, edits) in cases {
        let mut s = store();
        acl(&mut s, script).unwrap_or_else(|e| panic!("{script}: {e}"));
        assert_eq!(s.current().unwrap().1, apply(edits), "{script}");
    }
    // Spot checks of the results.
    let mut s = store();
    acl(
        &mut s,
        "add stage/beacon trigger.zone\nresource create behavior.program",
    )
    .unwrap();
    let w = s.current().unwrap().1;
    assert_eq!(w.capabilities.get("trigger"), Some(&1));
    assert_eq!(w.systems, ["trigger.detect", "behavior.execute"]);
    assert!(
        w.entity("stage/beacon").unwrap().components["trigger.zone"]["filter"]["tags"]
            == serde_json::json!(["player"])
    );
    let mut s = store();
    acl(&mut s, "create cube id=c rot=0,90,0").unwrap();
    let q = s.current().unwrap().1.entity("c").unwrap().components["core.transform"]["rotation"]
        .clone();
    assert_eq!(
        q,
        serde_json::json!(ops::rotation_from_degrees([0.0, 90.0, 0.0]))
    );
}

#[test]
fn world_verbs_report_structured_errors() {
    let cases = [
        ("capability add nope", "unknown_capability"),
        ("capability remove core", "invalid_value"),
        ("capability remove physics", "not_found"),
        ("capability remove render", "capability_in_use"),
        ("system add nope", "unknown_system"),
        ("system remove trigger.detect", "not_found"),
        ("resource create nope.x", "unknown_resource"),
        ("resource create render.settings", "id_taken"),
        ("resource remove nav.grid", "not_found"),
        ("resource create narrative.graph", "no_default"),
        (
            "resource create nav.grid {\"cell_size\":-1}",
            "invalid_value",
        ),
        ("world id \"has space\"", "invalid_id"),
        ("add stage/beacon render.mesh", "component_exists"),
        ("add stage/beacon nope.x", "unknown_component"),
        ("add stage/beacon path.follow", "no_default"),
        ("tag stage/crate +\"a b\"", "invalid_tag"),
        ("event declare \"bad name\"", "invalid_value"),
    ];
    for (script, expected) in cases {
        let mut s = store();
        let error = acl(&mut s, script).unwrap_err();
        assert_eq!(code(error.clone()), expected, "{script}: {error}");
        assert_eq!(s.current().unwrap().0, 1, "{script} must not write");
    }
    // Syntax: a tag needs + or -.
    let mut s = store();
    assert_eq!(
        code(acl(&mut s, "tag stage/crate player").unwrap_err()),
        "acl_syntax"
    );
    // Unchanged (already there) is not an error and writes nothing.
    let out = acl(&mut s, "capability add render").unwrap();
    assert!(out.starts_with("unchanged"), "{out}");
}

#[test]
fn array_element_verbs_append_insert_and_remove() {
    let mut s = store();
    acl(
        &mut s,
        "resource create input.bindings {\"buttons\":{\"a\":\"KeyA\"}}\nset stage/rack render.mesh/parts/0/color [1,0,0]",
    )
    .unwrap();
    let rack = |s: &Store| {
        s.current()
            .unwrap()
            .1
            .entity("stage/rack")
            .unwrap()
            .components["render.mesh"]["parts"]
            .as_array()
            .unwrap()
            .clone()
    };
    let before = rack(&s);
    acl(
        &mut s,
        "append stage/rack render.mesh/parts {\"color\":[0,1,0]}",
    )
    .unwrap();
    let after = rack(&s);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after.last().unwrap(), &serde_json::json!({"color":[0,1,0]}));
    acl(
        &mut s,
        "insert stage/rack render.mesh/parts 0 {\"color\":[0,0,1]}",
    )
    .unwrap();
    assert_eq!(rack(&s)[0], serde_json::json!({"color":[0,0,1]}));
    acl(&mut s, "remove stage/rack render.mesh/parts 0").unwrap();
    assert_eq!(rack(&s), after);
    // Out of range / not an array / missing: structured errors, no write.
    let rev = s.current().unwrap().0;
    for (script, expected) in [
        (
            "remove stage/rack render.mesh/parts 99",
            "index_out_of_range",
        ),
        (
            "insert stage/rack render.mesh/parts 99 {}",
            "index_out_of_range",
        ),
        ("append stage/rack label \"x\"", "invalid_field"),
        ("append stage/rack render.mesh/nope 1", "invalid_field"),
    ] {
        assert_eq!(code(acl(&mut s, script).unwrap_err()), expected, "{script}");
    }
    assert_eq!(s.current().unwrap().0, rev);
    // `set` past the end explains how to append.
    let error = acl(&mut s, "set stage/rack render.mesh/parts/99 {}").unwrap_err();
    assert!(
        error["error"].as_str().unwrap().contains("append"),
        "{error}"
    );
    // reorder FROM TO: the item ends up at TO, the others keep their order.
    let parts = rack(&s);
    acl(&mut s, "reorder stage/rack render.mesh/parts 0 1").unwrap();
    let moved = rack(&s);
    assert_eq!((&moved[0], &moved[1]), (&parts[1], &parts[0]));
    assert_eq!(&moved[2..], &parts[2..]);
    let rev = s.current().unwrap().0;
    for script in [
        "reorder stage/rack render.mesh/parts 0 99",
        "reorder stage/rack render.mesh/parts 99 0",
    ] {
        assert_eq!(code(acl(&mut s, script).unwrap_err()), "index_out_of_range");
    }
    assert!(acl(&mut s, "reorder stage/rack render.mesh/parts 0").is_err());
    assert_eq!(s.current().unwrap().0, rev);
}

#[test]
fn reset_puts_the_default_back_or_drops_a_prefab_override() {
    let mut s = store();
    acl(
        &mut s,
        "create light point id=lamp\nset lamp render.light/intensity 7\nset lamp render.light/color [1,0,0]",
    )
    .unwrap();
    acl(&mut s, "reset lamp render.light").unwrap();
    let world = s.current().unwrap().1;
    let fresh = {
        let mut other = store();
        acl(&mut other, "create light point id=lamp").unwrap();
        other.current().unwrap().1
    };
    assert_eq!(
        world.entity("lamp").unwrap().components["render.light"],
        fresh.entity("lamp").unwrap().components["render.light"]
    );
    // The main camera stays the main camera.
    acl(
        &mut s,
        "set stage/camera render.camera/near 0.5\nreset stage/camera render.camera",
    )
    .unwrap();
    let world = s.current().unwrap().1;
    let camera = &world.entity("stage/camera").unwrap().components["render.camera"];
    assert_eq!(camera["active"], true);
    assert_ne!(camera["near"], serde_json::json!(0.5));
    // Without the component: an error and no revision.
    let rev = s.current().unwrap().0;
    assert_eq!(
        code(acl(&mut s, "reset lamp audio.source").unwrap_err()),
        "not_found"
    );
    assert_eq!(s.current().unwrap().0, rev);
    // A prefab instance with an override goes back to the prefab's value.
    acl(
        &mut s,
        "create cube id=box\nprefab create crate from box\ninstantiate crate id=c2",
    )
    .unwrap();
    acl(&mut s, "set c2 render.mesh/parts/0/color [0,0,0]").unwrap();
    assert!(s
        .current()
        .unwrap()
        .1
        .entity("c2")
        .unwrap()
        .components
        .contains_key("render.mesh"));
    acl(&mut s, "reset c2 render.mesh").unwrap();
    let world = s.current().unwrap().1;
    let c2 = world.entity("c2").unwrap();
    assert!(!c2.components.contains_key("render.mesh"));
    assert!(world.resolved_components(c2).contains_key("render.mesh"));
}

#[test]
fn set_replacing_a_resource_list_warns_about_dropped_items() {
    let mut s = store();
    acl(
        &mut s,
        "resource create input.bindings {\"buttons\":{}}\ncreate trigger box id=gate",
    )
    .unwrap();
    // runtime.bindings is a runtime resource; a custom list in a core resource shows the same.
    let mut store = store();
    acl(&mut store, "event declare a\nevent declare b").unwrap();
    acl(
        &mut store,
        "set resources[id=behavior.program] rules [{\"id\":\"r1\",\"event\":\"a\",\"actions\":[]},{\"id\":\"r2\",\"event\":\"b\",\"actions\":[]}]",
    )
    .unwrap();
    let out = acl_mode(
        &mut store,
        "set resources[id=behavior.program] rules [{\"id\":\"r2\",\"event\":\"b\",\"actions\":[]}]",
        false,
        false,
    )
    .unwrap();
    let text = out["text"].as_str().unwrap();
    assert!(
        text.contains("warn line 1") && text.contains("1 current item(s)"),
        "{text}"
    );
    assert_eq!(out["warnings"].as_array().unwrap().len(), 1);
    // Appending keeps everything: no warning.
    let out = acl_mode(
        &mut store,
        "append resources[id=behavior.program] rules {\"id\":\"r3\",\"event\":\"a\",\"actions\":[]}",
        false,
        false,
    )
    .unwrap();
    assert!(out.get("warnings").is_none(), "{out}");
    // An item kept by id but rewritten: a superset of its list is fine, a lost entry warns
    // (the street's door binding that lost the tween play call another verb had added).
    let rule = |actions: &str| {
        format!("set resources[id=behavior.program] rules [{{\"id\":\"r1\",\"event\":\"a\",\"actions\":{actions}}}]")
    };
    acl(&mut store, &rule("[{\"kind\":\"emit\",\"event\":\"b\"}]")).unwrap();
    let grown = rule("[{\"kind\":\"emit\",\"event\":\"b\"},{\"kind\":\"emit\",\"event\":\"a\"}]");
    let out = acl_mode(&mut store, &grown, false, false).unwrap();
    assert!(out.get("warnings").is_none(), "{out}");
    let out = acl_mode(&mut store, &rule("[]"), false, false).unwrap();
    assert!(
        out["text"].as_str().unwrap().contains("2 current item(s)"),
        "{out}"
    );
    // Entity vectors and mesh parts are replaced on purpose: no warning.
    let out = acl_mode(
        &mut s,
        "set gate core.transform/position [1,2,3]",
        false,
        false,
    )
    .unwrap();
    assert!(out.get("warnings").is_none());
}

#[test]
fn undeclared_events_carry_the_fix() {
    let mut s = store();
    acl(&mut s, "create trigger box id=gate").unwrap();
    let error = acl(&mut s, "set gate trigger.zone/on_enter \"door_bump\"").unwrap_err();
    let message = error["error"].as_str().unwrap();
    assert!(
        message.contains("declare it first: event declare door_bump"),
        "{message}"
    );
    acl(&mut s, "event declare door_bump").unwrap();
    acl(&mut s, "set gate trigger.zone/on_enter \"door_bump\"").unwrap();
    // Declaring the same again is a no-op; different fields are refused.
    assert!(acl(&mut s, "event declare door_bump")
        .unwrap()
        .starts_with("unchanged"));
    assert_eq!(
        code(acl(&mut s, "event declare door_bump who=text").unwrap_err()),
        "id_taken"
    );
}

#[test]
fn group_verb_matches_the_op_and_keeps_world_poses() {
    let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let parent = |world: &World, id: &str| {
        world
            .relations
            .iter()
            .find(|r| r.kind == "parent" && r.source == id)
            .map(|r| r.target.clone())
    };
    let position = |world: &World, id: &str| -> [f32; 3] {
        serde_json::from_value(
            world.entity(id).unwrap().components["core.transform"]["position"].clone(),
        )
        .unwrap()
    };
    // Two root objects: a root group at their mean position, before the first of them.
    let created = ops::create_group(&lab(), &ids(&["stage/beacon", "stage/crate"]), None).unwrap();
    assert_eq!(created.ids, ["group"]);
    let via_op = apply(created.edits);
    let mut headless = store();
    acl(
        &mut headless,
        "group entities[id=stage/crate,id=stage/beacon]",
    )
    .unwrap();
    assert_eq!(headless.current().unwrap().1, via_op);
    let center = [(1.42 + 2.85) / 2.0, 0.63 / 2.0, (0.82 - 1.38) / 2.0];
    let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
    assert!(close(position(&via_op, "group"), center));
    for (id, world) in [
        ("stage/crate", [1.42, 0.63, 0.82]),
        ("stage/beacon", [2.85, 0.0, -1.38]),
    ] {
        assert_eq!(parent(&via_op, id).as_deref(), Some("group"));
        let local = position(&via_op, id);
        assert!(close(
            [
                local[0] + center[0],
                local[1] + center[1],
                local[2] + center[2]
            ],
            world
        ));
    }
    let order: Vec<&str> = via_op.entities.iter().map(|e| e.id.as_str()).collect();
    let at = |id: &str| order.iter().position(|e| *e == id).unwrap();
    assert!(at("group") < at("stage/crate"));
    // The group joins the scene every grouped object is in.
    let scene = via_op
        .scenes
        .iter()
        .find(|s| s.id == "assembly-yard")
        .unwrap();
    assert!(scene.entities.iter().any(|e| e == "group"));
    // A listed descendant stays under its listed ancestor; mixed parents -> root at origin.
    let nested = apply(
        ops::create_group(
            &lab(),
            &ids(&["stage/rack", "stage/rack/cargo"]),
            Some("kit"),
        )
        .unwrap()
        .edits,
    );
    assert_eq!(parent(&nested, "stage/rack").as_deref(), Some("kit"));
    assert_eq!(
        parent(&nested, "stage/rack/cargo").as_deref(),
        Some("stage/rack")
    );
    let mixed =
        ops::create_group(&lab(), &ids(&["stage/crate", "stage/rack/cargo"]), None).unwrap();
    let mixed = apply(mixed.edits);
    assert_eq!(parent(&mixed, "group"), None);
    assert!(close(position(&mixed, "group"), [0.0; 3]));
    assert_eq!(parent(&mixed, "stage/rack/cargo").as_deref(), Some("group"));
    // A second group gets the next free ID; bad input is a structured error.
    let mut store = store();
    acl(&mut store, "group stage/crate").unwrap();
    acl(&mut store, "group stage/beacon").unwrap();
    assert!(store.current().unwrap().1.entity("group/2").is_some());
    assert_eq!(
        code(acl(&mut store, "group stage/floor id=group").unwrap_err()),
        "id_taken"
    );
    assert_eq!(
        code(acl(&mut store, "group missing").unwrap_err()),
        "no_match"
    );
    assert!(ops::create_group(&lab(), &ids(&["missing"]), None).is_err());
    assert!(ops::create_group(&lab(), &[], None).is_err());
}

#[test]
fn set_sky_presets_keep_authored_values() {
    let registry = Registry::core();
    let world = lab();
    assert!(!world.resources.contains_key("render.environment"));
    // `none` on a world without a sky writes nothing.
    assert!(ops::set_sky(&world, &registry, "none").unwrap().is_empty());
    let sky = apply(ops::set_sky(&world, &registry, "procedural").unwrap());
    let env = &sky.resources["render.environment"];
    assert_eq!(env["sky"]["mode"], "procedural");
    assert_eq!(env["sky"]["sun_size_degrees"], 1.5);
    assert_eq!(env["ambient"]["mode"], "sky");
    // Tuned values survive pressing the same preset again (empty batch).
    let mut tuned = sky.clone();
    tuned.resources.get_mut("render.environment").unwrap()["sky"]["exposure"] = 1.7.into();
    assert!(ops::set_sky(&tuned, &registry, "procedural").unwrap().is_empty());
    // Another mode replaces the sky but keeps fog and an authored ambient.
    let mut fogged = tuned.clone();
    let env = fogged.resources.get_mut("render.environment").unwrap();
    env["fog"] = serde_json::json!({"color":[0.1,0.1,0.1],"density":0.02});
    env["ambient"] = serde_json::json!({"mode":"color","color":[0.3,0.3,0.3],"intensity":1});
    let mut s = Store::memory_with(Registry::core(), fogged.clone()).unwrap();
    s.execute(Request::Apply {
        expected_rev: None,
        edits: ops::set_sky(&fogged, &registry, "gradient").unwrap(),
    })
    .unwrap();
    let env = s.current().unwrap().1.resources["render.environment"].clone();
    assert_eq!(env["sky"]["mode"], "gradient");
    assert_eq!(env["ambient"]["mode"], "color");
    assert_eq!(env["fog"]["density"].as_f64().map(|d| (d * 100.0).round()), Some(2.0));
    // `none` drops the sky and a sky-mode ambient only.
    let cleared = apply(ops::set_sky(&sky, &registry, "none").unwrap());
    assert_eq!(cleared.resources["render.environment"], serde_json::json!({}));
    let error = ops::set_sky(&world, &registry, "hdri").unwrap_err();
    assert_eq!(code(error_response(&error)), "invalid_value");
    // The ACL verb is the same op.
    let mut s = store();
    acl(&mut s, "sky gradient").unwrap();
    assert_eq!(
        s.current().unwrap().1.resources["render.environment"]["sky"]["mode"],
        "gradient"
    );
    assert_eq!(code(acl(&mut s, "sky").unwrap_err()), "acl_syntax");
}