use axiom_core::*;
use axiom_runtime::{
    scenario,
    world::{ActionMap, RuntimeRegistry, WorldRuntime},
};
use serde_json::json;

#[test]
fn fps_and_motion_are_scheduled_through_the_same_registry() {
    let mut w = axiom_fps::demo_world().unwrap();
    w.capabilities.insert("motion".into(), 1);
    w.systems.push("motion.integrate".into());
    let mut marker = Entity::new("moving-marker");
    marker.set("core.transform", Transform::default()).unwrap();
    marker
        .set(
            "core.motion",
            Motion {
                velocity: [1.0, 0.0, 0.0],
                ..Default::default()
            },
        )
        .unwrap();
    w.entities.push(marker);
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    for _ in 0..120 {
        r.step(&ActionMap::default());
    }
    let slot = r.state.compiled.indices["moving-marker"];
    assert!((r.state.transforms[slot].as_ref().unwrap().position[0] - 1.0).abs() < 0.001);
    assert_eq!(r.observation().metrics["enemies_total"], 8);
    assert_eq!(
        r.observation().systems,
        vec!["motion.integrate", "fps.simulate"]
    );
}

#[test]
fn custom_columns_survive_reload_and_entity_refs_are_relinked() {
    let mut w = World::empty("flags");
    w.component_schemas.insert(
        "game.flags".into(),
        serde_json::from_value(
            json!({"fields":{"enabled":{"ty":"bool"},"target":{"ty":"entity"}}}),
        )
        .unwrap(),
    );
    let mut a = Entity::new("a");
    a.set("game.flags", json!({"enabled":false,"target":"b"}))
        .unwrap();
    w.entities = vec![a, Entity::new("b")];
    let mut r = WorldRuntime::new(w.clone(), 1, RuntimeRegistry::core()).unwrap();
    if let Column::Bool(v) = r
        .state
        .columns
        .get_mut("game.flags")
        .unwrap()
        .fields
        .get_mut("enabled")
        .unwrap()
    {
        v[0] = Some(true);
    }
    w.entities.swap(0, 1);
    r.reconcile(w, 2).unwrap();
    assert!(
        matches!(&r.state.columns["game.flags"].fields["enabled"],Column::Bool(v) if v[0]==Some(true))
    );
    assert!(
        matches!(&r.state.columns["game.flags"].fields["target"],Column::Entity(v) if v[0]==Some(0))
    );
}

#[test]
fn fps_hud_and_presets_use_the_applied_revision() {
    let w = axiom_fps::demo_world().unwrap();
    let mut r = WorldRuntime::new(w.clone(), 7, axiom_runtime::installed_registry()).unwrap();
    assert_eq!(r.observation().metrics["fps_revision"], 7);
    r.reconcile(w, 8).unwrap();
    assert_eq!(r.observation().metrics["fps_revision"], 8);
    assert_eq!(r.state.revision, r.rev);
}
fn motion_world() -> World {
    let mut w = World::empty("motion");
    w.capabilities.insert("motion".into(), 1);
    w.systems.push("motion.integrate".into());
    let mut e = Entity::new("box");
    e.set("core.transform", Transform::default()).unwrap();
    e.set(
        "core.motion",
        Motion {
            velocity: [1.0, 0.0, 0.0],
            ..Default::default()
        },
    )
    .unwrap();
    w.entities.push(e);
    w
}
#[test]
fn empty_runtime_runs_without_fps_system_or_physics() {
    let mut r = WorldRuntime::new(World::empty("empty"), 1, RuntimeRegistry::core()).unwrap();
    for _ in 0..120 {
        r.step(&ActionMap::default());
    }
    assert_eq!(r.observation().tick, 120);
    assert_eq!(r.observation().entities, 0);
    assert!(r.observation().metrics.is_empty());
    assert!(r.render_frame().instances.is_empty());
    assert!(r.render_frame().views.is_empty());
}
#[test]
fn typed_motion_and_hot_reload_preserve_runtime_state() {
    let mut w = motion_world();
    let mut r = WorldRuntime::new(w.clone(), 1, RuntimeRegistry::core()).unwrap();
    for _ in 0..120 {
        r.step(&ActionMap::default());
    }
    assert!((r.state.transforms[0].as_ref().unwrap().position[0] - 1.0).abs() < 0.001);
    let id = r.state.ids[0];
    w.title = "Renamed".into();
    r.reconcile(w.clone(), 2).unwrap();
    assert_eq!(r.state.ids[0], id);
    assert!((r.state.transforms[0].as_ref().unwrap().position[0] - 1.0).abs() < 0.001);
    w.entities.clear();
    r.reconcile(w.clone(), 3).unwrap();
    assert!(r.state.resolve(id).is_none());
    w = motion_world();
    r.reconcile(w, 4).unwrap();
    assert_ne!(r.state.ids[0], id);
    assert!(r.state.resolve(id).is_none());
}
#[test]
fn failed_activation_preserves_last_good_world_and_reports_requested_revision() {
    let mut r = WorldRuntime::new(motion_world(), 1, RuntimeRegistry::core()).unwrap();
    let mut invalid = motion_world();
    invalid.capabilities.insert("missing".into(), 1);
    assert!(r.reconcile(invalid, 2).is_err());
    let obs = r.observation();
    assert_eq!(obs.rev, 1);
    assert_eq!(obs.requested_rev, 2);
    assert_eq!(obs.activation, "activation_failed");
    assert!(obs.activation_error.is_some());
    r.step(&ActionMap::default());
    r.assert_valid().unwrap();
}
#[test]
fn renderer_extracts_two_views_and_non_fps_meshes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/two-cameras");
    let package = ContentPackage::load(root, &Registry::core()).unwrap();
    let mut r = WorldRuntime::new(package.world, 1, RuntimeRegistry::core()).unwrap();
    r.step(&ActionMap::default());
    let frame = r.render_frame();
    assert_eq!(frame.views.len(), 2);
    assert_eq!(frame.instances.len(), 3);
    assert_eq!(frame.views[0].camera.viewport, [0.0, 0.0, 0.5, 1.0]);
    assert!(r.observation().metrics.is_empty());
}
#[test]
fn parent_transform_and_disabled_scenes_affect_extraction() {
    let mut w = motion_world();
    w.capabilities.insert("render".into(), 1);
    let mut child = Entity::new("child");
    child
        .set(
            "core.transform",
            Transform {
                position: [0.0, 2.0, 0.0],
                ..Default::default()
            },
        )
        .unwrap();
    child
        .set(
            "render.mesh",
            RenderMesh {
                parts: vec![MeshPart {
                    transform: Transform::default(),
                    color: [1.0; 3],
                    emission: 0.0,
                    grid: false,
                }],
            },
        )
        .unwrap();
    w.entities.push(child);
    w.relations.push(Relation {
        source: "child".into(),
        target: "box".into(),
        kind: "parent".into(),
    });
    let mut r = WorldRuntime::new(w.clone(), 1, RuntimeRegistry::core()).unwrap();
    for _ in 0..120 {
        r.step(&ActionMap::default());
    }
    let frame = r.render_frame();
    assert!((frame.instances[0].model[3][0] - 1.0).abs() < 0.001);
    assert_eq!(frame.instances[0].model[3][1], 2.0);
    w.scenes.push(Scene {
        id: "hidden".into(),
        entities: vec!["child".into()],
        active: false,
    });
    r.reconcile(w, 2).unwrap();
    assert!(r.render_frame().instances.is_empty());
}
#[test]
fn fps_content_runs_through_generic_actions_and_scenario() {
    let w = axiom_fps::demo_world().unwrap();
    let scenario: scenario::Scenario = serde_json::from_str(include_str!(
        "../../../examples/fps-regression.scenario.json"
    ))
    .unwrap();
    let report = scenario::run(w, 1, axiom_runtime::installed_registry(), scenario).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["observation"]["tick"], 240);
    assert!(report["observation"]["shots"].as_u64().unwrap() >= 5);
}
#[test]
fn scenario_rejects_unknown_actions_and_failed_assertions() {
    let scenario: scenario::Scenario = serde_json::from_value(
        json!({"ticks":1,"inputs":[{"tick":0,"actions":{"typo":1}}],"assertions":[]}),
    )
    .unwrap();
    assert!(scenario::run(World::empty("empty"), 1, RuntimeRegistry::core(), scenario).is_err());
    let scenario:scenario::Scenario=serde_json::from_value(json!({"ticks":1,"assertions":[{"kind":"position","entity":"box","axis":0,"min":9,"max":10}]})).unwrap();
    assert!(scenario::run(motion_world(), 1, RuntimeRegistry::core(), scenario).is_err());
}
