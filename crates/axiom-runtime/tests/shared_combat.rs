use axiom_core::{gameplay::RuntimeCommand, ContentPackage, Entity, Health, Transform, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use serde_json::{json, Value};

fn world() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/m4"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}
fn position(w: &mut World, id: &str, p: [f32; 2]) {
    w.entities
        .iter_mut()
        .find(|e| e.id == id)
        .unwrap()
        .set(
            "core.transform",
            Transform {
                position: [p[0], 0.0, p[1]],
                ..Default::default()
            },
        )
        .unwrap();
}
fn actor(w: &mut World, id: &str, field: &str, value: Value) {
    let e = w.entity(id).unwrap();
    let mut def = w.resolved_components(e)["action.actor"].clone();
    def[field] = value;
    w.entities
        .iter_mut()
        .find(|e| e.id == id)
        .unwrap()
        .components
        .insert("action.actor".into(), def);
}
fn stage() -> World {
    let mut w = world();
    position(&mut w, "hero", [0.0, 0.0]);
    position(&mut w, "enemy/guard", [0.0, -1.5]);
    position(&mut w, "squad/soldier/4", [1.2, -1.5]);
    position(&mut w, "squad/soldier/5", [-1.2, -1.5]);
    actor(&mut w, "enemy/guard", "ai", json!(false));
    w
}
fn call(r: &mut WorldRuntime, cap: &str, action: &str, args: Value) -> Value {
    r.command(&RuntimeCommand::Call {
        capability: cap.into(),
        action: action.into(),
        args,
    })
    .unwrap()
}
fn step(r: &mut WorldRuntime, n: u32) {
    for _ in 0..n {
        r.step(&ActionMap::default());
        r.assert_valid().unwrap();
        assert!(r.state.events.fault.is_none(), "{:?}", r.state.events.fault);
    }
}
fn hp(r: &WorldRuntime, id: &str) -> f32 {
    r.state.health[r.state.compiled.indices[id]]
        .as_ref()
        .unwrap()
        .value
}
fn raider(w: &mut World, p: [f32; 2], health: f32) {
    let mut e = Entity::new("enemy/raider");
    e.prefab = Some("squad.soldier".into());
    let mut def = w.prefabs["squad.soldier"].components["rts.unit"].clone();
    def["faction"] = json!(1);
    e.components.insert("rts.unit".into(), def);
    e.set(
        "core.health",
        Health {
            value: health,
            max: health,
        },
    )
    .unwrap();
    e.set(
        "core.transform",
        Transform {
            position: [p[0], 0.0, p[1]],
            ..Default::default()
        },
    )
    .unwrap();
    w.entities.push(e);
}
#[test]
fn hero_and_squad_share_guard_health_and_death_without_duplicate_events() {
    let mut r = WorldRuntime::new(stage(), 1, installed_registry()).unwrap();
    call(
        &mut r,
        "rts",
        "attack",
        json!({"units":["squad/soldier/4"],"target":"enemy/guard"}),
    );
    call(&mut r, "action", "lock", json!({}));
    call(&mut r, "action", "attack", json!({}));
    step(&mut r, 40);
    let scene = &r.state.combat.as_ref().unwrap().scene;
    assert_eq!(scene.bodies["enemy/guard"].health, hp(&r, "enemy/guard"));
    assert!(hp(&r, "enemy/guard") < 52.0);
    let recent = r.observation().metrics["combat"]["recent"]
        .as_array()
        .unwrap()
        .clone();
    assert!(recent.iter().any(|v| v["receipt"]["source"] == "hero"));
    assert!(recent
        .iter()
        .any(|v| v["receipt"]["source"] == "squad/soldier/4"));
    let before = hp(&r, "enemy/guard");
    let mut edited = r.state.compiled.source.clone();
    edited.title = "Shared damage retained".into();
    r.reconcile(edited, 2).unwrap();
    assert_eq!(hp(&r, "enemy/guard"), before);
    step(&mut r, 1000);
    assert_eq!(hp(&r, "enemy/guard"), 0.0);
    assert_eq!(r.observation().metrics["combat"]["deaths"], 1);
    assert_eq!(r.observation().metrics["action"]["victory"], true);
    let seq = r.observation().metrics["combat"]["sequence"].clone();
    step(&mut r, 120);
    assert_eq!(r.observation().metrics["combat"]["sequence"], seq);
}
#[test]
fn checked_in_support_scenario_finishes_with_squad_kill_and_hero_damage() {
    let scenario =
        serde_json::from_str(include_str!("../../../examples/m4-support.scenario.json")).unwrap();
    let result = axiom_runtime::scenario::run(world(), 1, installed_registry(), scenario).unwrap();
    assert_eq!(result["ok"], true);
}
#[test]
fn moving_controllers_cannot_cross_each_others_live_bodies() {
    let mut w = stage();
    position(&mut w, "enemy/guard", [8.0, -8.0]);
    position(&mut w, "squad/soldier/4", [0.0, -1.0]);
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    call(&mut r, "input", "activate", json!({"context":"hero"}));
    for _ in 0..90 {
        r.step(&ActionMap([("move_z".into(), 1.0)].into()));
        r.assert_valid().unwrap();
    }
    let scene = &r.state.combat.as_ref().unwrap().scene;
    assert!(scene.bodies["hero"].position.y > -0.5);
    call(
        &mut r,
        "rts",
        "move",
        json!({"units":["squad/soldier/4"],"destination":[0.5,-0.5]}),
    );
    for _ in 0..240 {
        r.step(&ActionMap::default());
        r.assert_valid().unwrap();
        let scene = &r.state.combat.as_ref().unwrap().scene;
        let (a, b) = (&scene.bodies["hero"], &scene.bodies["squad/soldier/4"]);
        assert!(a.position.distance(b.position) + 0.001 >= a.radius + b.radius);
    }
}
#[test]
fn production_avoids_foreign_bodies_and_keeps_generated_handles_on_reload() {
    let mut w = stage();
    position(&mut w, "hero", [-11.5, -1.5]);
    position(&mut w, "enemy/guard", [8.0, -8.0]);
    let mut r = WorldRuntime::new(w.clone(), 1, installed_registry()).unwrap();
    call(
        &mut r,
        "rts",
        "train",
        json!({"building":"squad/base","recipe":"worker","count":2}),
    );
    step(&mut r, 500);
    assert_eq!(
        r.observation().metrics["rts"]["summary"]["stats"]["produced"],
        2
    );
    let scene = &r.state.combat.as_ref().unwrap().scene;
    scene.validate().unwrap();
    assert_ne!(
        scene.bodies["produced/0/1"].position,
        scene.bodies["hero"].position
    );
    let handle = scene.bodies["produced/0/1"].handle.clone();
    w.title = "Keep combat handles".into();
    r.reconcile(w, 2).unwrap();
    assert_eq!(
        r.state.combat.as_ref().unwrap().scene.bodies["produced/0/1"].handle,
        handle
    );
}
#[test]
fn hero_locks_and_kills_rts_enemy_and_rts_removes_dead_selection_paths() {
    let mut w = stage();
    position(&mut w, "enemy/guard", [8.0, -8.0]);
    raider(&mut w, [0.0, -1.2], 10.0);
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    call(&mut r, "action", "lock", json!({}));
    assert_eq!(
        r.observation().metrics["action"]["target"]["id"],
        "enemy/raider"
    );
    call(&mut r, "action", "attack", json!({}));
    step(&mut r, 45);
    assert_eq!(hp(&r, "enemy/raider"), 0.0);
    assert!(!r.state.compiled.enabled[r.state.compiled.indices["enemy/raider"]]);
    assert_eq!(r.observation().metrics["combat"]["deaths"], 1);
    assert_eq!(r.observation().metrics["action"]["victory"], false);
    assert_eq!(
        r.observation().metrics["rts"]["summary"]["stats"]["kills"],
        0
    );
}
#[test]
fn action_enemy_targets_nearby_squad_instead_of_distant_hero() {
    let mut w = stage();
    position(&mut w, "hero", [6.0, 4.0]);
    actor(&mut w, "enemy/guard", "ai", json!(true));
    w.resources.get_mut("action.session").unwrap()["enemy_notice_range"] = json!(6.0);
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    let before = hp(&r, "squad/soldier/4");
    step(&mut r, 240);
    assert!(hp(&r, "squad/soldier/4") < before || hp(&r, "squad/soldier/5") < before);
    assert_eq!(hp(&r, "hero"), 300.0);
}
#[test]
fn rts_attacks_obey_hero_roll_invulnerability() {
    let mut w = stage();
    position(&mut w, "enemy/guard", [8.0, -8.0]);
    raider(&mut w, [0.0, -1.2], 100.0);
    w.resources.get_mut("rts.session").unwrap()["auto_combat"] = json!(true);
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    step(&mut r, 24);
    call(&mut r, "action", "dodge", json!({}));
    step(&mut r, 12);
    assert_eq!(hp(&r, "hero"), 300.0);
    assert!(
        r.observation().metrics["combat"]["evaded"]
            .as_u64()
            .unwrap()
            > 0
    );
}
#[test]
fn shared_wall_blocks_rts_damage_hero_motion_and_survives_hot_reload() {
    let mut w = stage();
    position(&mut w, "hero", [1.0, 0.0]);
    position(&mut w, "enemy/guard", [8.0, -8.0]);
    position(&mut w, "squad/soldier/4", [4.0, 3.0]);
    position(&mut w, "squad/soldier/5", [5.0, 3.0]);
    raider(&mut w, [-1.0, 0.0], 100.0);
    w.entities
        .iter_mut()
        .find(|e| e.id == "enemy/raider")
        .unwrap()
        .components
        .get_mut("rts.unit")
        .unwrap()["attack_range"] = json!(5.0);
    w.resources.get_mut("rts.session").unwrap()["auto_combat"] = json!(true);
    w.resources.get_mut("combat.session").unwrap()["obstacles"] =
        json!([{"min":[-0.2,-2.0],"max":[0.2,2.0]}]);
    let mut r = WorldRuntime::new(w.clone(), 1, installed_registry()).unwrap();
    call(&mut r, "input", "activate", json!({"context":"hero"}));
    for _ in 0..90 {
        r.step(&ActionMap([("move_x".into(), -1.0)].into()));
        r.assert_valid().unwrap();
    }
    assert!(
        r.state.transforms[r.state.compiled.indices["hero"]]
            .as_ref()
            .unwrap()
            .position[0]
            > 0.4
    );
    assert_eq!(hp(&r, "hero"), 300.0);
    assert!(
        r.observation().metrics["combat"]["rejected"]
            .as_u64()
            .unwrap()
            > 0
    );
    let old = r.observation().metrics["combat"].clone();
    let ids = r.state.ids.clone();
    w.title = "Shared combat reload".into();
    r.reconcile(w, 2).unwrap();
    assert_eq!(
        r.observation().metrics["combat"]["sequence"],
        old["sequence"]
    );
    assert_eq!(r.state.ids, ids);
    step(&mut r, 20);
    assert_eq!(hp(&r, "hero"), 300.0);
}
#[test]
fn activation_rejects_cross_controller_overlap_and_inspection_is_bounded() {
    let mut w = stage();
    position(&mut w, "squad/soldier/4", [0.0, 0.0]);
    assert!(WorldRuntime::new(w, 1, installed_registry()).is_err());
    let mut r = WorldRuntime::new(stage(), 1, installed_registry()).unwrap();
    let inspected = call(
        &mut r,
        "combat",
        "inspect",
        json!({"ids":["hero","enemy/guard"]}),
    );
    assert_eq!(inspected["result"]["bodies"][0]["owner"], "action.simulate");
    assert!(
        inspected["result"]["bodies"][0]["handle"]["generation"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(r
        .command(&RuntimeCommand::Call {
            capability: "combat".into(),
            action: "inspect".into(),
            args: json!({"ids":vec!["hero";129]})
        })
        .is_err());
}
