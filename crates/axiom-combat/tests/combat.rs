use axiom_combat::*;
use glam::Vec2;
fn scene() -> Scene {
    let mut s = Scene::new(Config {
        bounds: Rect {
            min: [-10.0; 2],
            max: [10.0; 2],
        },
        obstacles: vec![],
    });
    for (id, x, faction) in [("a", 0.0, 0), ("b", 1.0, 1), ("c", -1.0, 0)] {
        s.bodies.insert(
            id.into(),
            Body {
                handle: Handle {
                    id: id.into(),
                    generation: 1,
                },
                owner: id.into(),
                position: Vec2::new(x, 0.0),
                radius: 0.3,
                faction,
                health: 10.0,
                max_health: 10.0,
                invulnerable: false,
            },
        );
    }
    s
}
#[test]
fn simultaneous_hits_trade_and_death_is_emitted_once() {
    let mut s = scene();
    let hits = vec![
        s.attempt("a", "b", 10.0, 2.0, None).unwrap(),
        s.attempt("c", "b", 10.0, 2.0, None).unwrap(),
        s.attempt("b", "a", 10.0, 2.0, None).unwrap(),
    ];
    let receipts = s.resolve(hits);
    assert_eq!(receipts.iter().filter(|r| r.killed).count(), 2);
    assert!(!receipts[1].killed);
    assert_eq!(receipts[1].amount, 0.0);
    assert_eq!(s.bodies["a"].health, 0.0);
    assert_eq!(s.bodies["b"].health, 0.0);
}
#[test]
fn masks_generations_owner_los_arc_and_invulnerability_are_checked() {
    let mut s = scene();
    let friendly = s.attempt("a", "c", 9.0, 2.0, None).unwrap();
    assert!(!s.in_range(&friendly));
    let mut stale = s.attempt("a", "b", 9.0, 2.0, None).unwrap();
    stale.target.generation = 2;
    assert!(!s.in_range(&stale));
    let mut forged = s.attempt("a", "b", 9.0, 2.0, None).unwrap();
    forged.owner = "b".into();
    assert!(!s.in_range(&forged));
    let backwards = s
        .attempt("a", "b", 9.0, 2.0, Some((-Vec2::X, 0.0)))
        .unwrap();
    assert!(!s.in_range(&backwards));
    let hit = s.attempt("a", "b", 9.0, 2.0, None).unwrap();
    s.bodies.get_mut("b").unwrap().invulnerable = true;
    assert_eq!(s.resolve(vec![hit.clone()])[0].outcome, Outcome::Evaded);
    assert_eq!(s.bodies["b"].health, 10.0);
    s.config.obstacles.push(Rect {
        min: [0.4, -0.1],
        max: [0.6, 0.1],
    });
    assert!(!s.in_range(&hit));
}
#[test]
fn shared_discs_and_rectangles_block_all_factions() {
    let mut s = scene();
    assert!(!s.clear("a", Vec2::new(-0.7, 0.0), 0.3));
    assert!(!s.clear("a", Vec2::new(0.7, 0.0), 0.3));
    s.config.obstacles.push(Rect {
        min: [-0.2, 1.0],
        max: [0.2, 2.0],
    });
    assert!(!s.clear("a", Vec2::new(0.0, 0.9), 0.3));
    assert!(s.clear("a", Vec2::new(0.0, 0.5), 0.3));
    s.bodies.get_mut("b").unwrap().health = 0.0;
    assert!(s.clear("a", Vec2::new(1.0, 0.0), 0.3));
}
