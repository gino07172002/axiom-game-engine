use axiom_action::{simulation::Simulation, *};
use axiom_core::{Entity, Health, Registry, Transform, World};
use glam::Vec2;
use serde_json::json;

fn sim() -> Simulation {
    let mut world: World =
        serde_json::from_str(include_str!("../../../content/action/world.json")).unwrap();
    // Pure simulation tests keep authoring definitions and omit visual systems.
    world.capabilities = [("core".into(), 1), ("action".into(), 1)].into();
    world.entities.clear();
    world.prefabs.clear();
    world.scenes.clear();
    world.systems = vec!["action.simulate".into()];
    world
        .resources
        .retain(|id, _| id == SESSION || id == ABILITIES);
    let actor = |id: &str, faction, position| {
        let mut e = Entity::new(id);
        e.set(
            "core.transform",
            Transform {
                position,
                ..Default::default()
            },
        )
        .unwrap();
        e.set(
            "core.health",
            Health {
                value: 100.0,
                max: 100.0,
            },
        )
        .unwrap();
        e.set(
            "action.actor",
            ActorDef {
                faction,
                speed: 3.8,
                radius: 0.27,
                stamina: 100.0,
                regen: 0.0,
                idle_clip: "idle".into(),
                run_clip: "run".into(),
                death_clip: "death".into(),
                attack: if faction == 0 {
                    "slash1".into()
                } else {
                    "cleave".into()
                },
                dodge: "roll".into(),
                ai: false,
                model_yaw: 0.0,
            },
        )
        .unwrap();
        e.set("animation.player", json!({})).unwrap();
        e
    };
    world.entities = vec![
        actor("hero", 0, [0.0, 0.0, 1.1]),
        actor("enemy", 1, [0.0, 0.0, 0.0]),
    ];
    let mut camera = Entity::new("camera");
    camera.set("core.transform", Transform::default()).unwrap();
    camera
        .set("render.camera", axiom_core::Camera::perspective())
        .unwrap();
    world.entities.push(camera);
    world.capabilities.insert("render".into(), 1);
    let mut registry = Registry::core();
    register(&mut registry);
    registry.component("animation.player", "action", json!({}), |_| Ok(()));
    Simulation::new(&world.compile(&registry).unwrap()).unwrap()
}
fn advance(sim: &mut Simulation, ticks: u32) {
    for _ in 0..ticks {
        sim.step(Vec2::ZERO);
        sim.drain_events();
    }
}
#[test]
fn active_window_is_half_open_and_each_target_is_attempted_once() {
    let mut s = sim();
    s.request(Kind::Attack, Vec2::ZERO).unwrap();
    advance(&mut s, 17);
    assert_eq!(s.actors[1].health, 100.0);
    advance(&mut s, 1);
    assert_eq!(s.actors[1].health, 82.0);
    advance(&mut s, 48);
    assert_eq!(s.stats.hits, 1);
    assert!(s.actors[0].active.is_none());
}
#[test]
fn second_attack_buffers_then_cancels_to_roll_only_inside_the_authored_window() {
    let mut s = sim();
    s.request(Kind::Attack, Vec2::ZERO).unwrap();
    advance(&mut s, 24);
    assert_eq!(s.request(Kind::Attack, Vec2::ZERO).unwrap(), "buffered");
    advance(&mut s, 9);
    assert_eq!(s.actors[0].active.as_ref().unwrap().ability, "slash1");
    advance(&mut s, 1);
    assert_eq!(s.actors[0].active.as_ref().unwrap().ability, "slash2");
    assert_eq!(s.stats.chains, 1);
    assert_eq!(s.actors[0].stamina, 71.0);
    advance(&mut s, 20);
    assert_eq!(s.request(Kind::Dodge, Vec2::ZERO).unwrap(), "buffered");
    advance(&mut s, 9);
    assert_eq!(s.actors[0].active.as_ref().unwrap().ability, "slash2");
    advance(&mut s, 1);
    assert_eq!(s.actors[0].active.as_ref().unwrap().ability, "roll");
    assert_eq!(s.actors[0].stamina, 45.0);
    assert_eq!(s.stats.rolls, 1);
}
#[test]
fn expired_input_and_insufficient_stamina_do_not_start_or_charge_an_ability() {
    let mut s = sim();
    s.request(Kind::Attack, Vec2::ZERO).unwrap();
    advance(&mut s, 1);
    s.request(Kind::Attack, Vec2::ZERO).unwrap();
    advance(&mut s, 40);
    assert_eq!(s.stats.attacks, 1);
    s.actors[0].stamina = 10.0;
    let active = s.actors[0].active.as_ref().unwrap().ability.clone();
    assert!(s.request(Kind::Dodge, Vec2::ZERO).is_err());
    assert_eq!(s.actors[0].stamina, 10.0);
    assert_eq!(s.actors[0].active.as_ref().unwrap().ability, active);
}
#[test]
fn roll_invulnerability_ends_at_the_exact_authored_tick() {
    let mut s = sim();
    // Prevent physical separation from obscuring the temporal boundary.
    s.library.abilities.get_mut("roll").unwrap().travel_speed = 0.0;
    let mut enemy = s.library.abilities["cleave"].clone();
    enemy.active = Some(Window { start: 23, end: 25 });
    enemy.arc_degrees = 360.0;
    s.library.abilities.insert("cleave".into(), enemy);
    s.actors[1].def.ai = true;
    s.request(Kind::Dodge, Vec2::ZERO).unwrap();
    advance(&mut s, 24);
    // The enemy started at tick 1. Its first attempt is tick 24, just outside [0,24).
    assert_eq!(s.actors[0].health, 82.0);
    let mut s = sim();
    s.library.abilities.get_mut("roll").unwrap().travel_speed = 0.0;
    let a = s.library.abilities.get_mut("cleave").unwrap();
    a.active = Some(Window { start: 22, end: 26 });
    a.arc_degrees = 360.0;
    s.actors[1].def.ai = true;
    s.request(Kind::Dodge, Vec2::ZERO).unwrap();
    advance(&mut s, 27);
    assert_eq!(s.actors[0].health, 100.0);
    assert_eq!(s.stats.dodged, 1);
}
#[test]
fn fast_roll_respects_walls_and_attacks_cannot_cross_an_obstacle() {
    let mut s = sim();
    s.actors[0].position = Vec2::new(0.0, 2.0);
    s.session.obstacles.push(Obstacle {
        min: [-1.0, 0.8],
        max: [1.0, 1.0],
    });
    s.library.abilities.get_mut("roll").unwrap().travel_speed = 18.0;
    s.request(Kind::Dodge, Vec2::ZERO).unwrap();
    advance(&mut s, 48);
    assert!(s.actors[0].position.y >= 1.26);
    s.request(Kind::Attack, Vec2::ZERO).unwrap();
    advance(&mut s, 45);
    assert_eq!(s.actors[1].health, 100.0);
}
#[test]
fn three_authored_attacks_can_chain_and_kill() {
    let mut s = sim();
    s.actors[1].health = 70.0;
    s.lock();
    s.request(Kind::Attack, Vec2::ZERO).unwrap();
    advance(&mut s, 34);
    s.request(Kind::Attack, Vec2::ZERO).unwrap();
    advance(&mut s, 36);
    s.request(Kind::Attack, Vec2::ZERO).unwrap();
    advance(&mut s, 35);
    assert_eq!(s.stats.attacks, 3);
    assert_eq!(s.stats.chains, 2);
    assert_eq!(s.stats.hits, 3);
    assert!(s.victory());
    assert!(s.request(Kind::Attack, Vec2::ZERO).is_err());
}
