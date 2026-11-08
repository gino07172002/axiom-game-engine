use axiom_core::*;
use axiom_rts::{simulation::Simulation, *};
use serde_json::json;
use std::collections::BTreeMap;
fn setup() -> (Simulation, BTreeMap<String, (UnitDef, f32)>) {
    let w = content::demo_world(64).unwrap();
    let mut registry = Registry::core();
    register(&mut registry);
    let mut templates = BTreeMap::new();
    for (id, p) in &w.prefabs {
        templates.insert(
            id.clone(),
            (
                serde_json::from_value(p.components["rts.unit"].clone()).unwrap(),
                serde_json::from_value::<Health>(p.components["core.health"].clone())
                    .unwrap()
                    .value,
            ),
        );
    }
    (
        Simulation::new(&w.compile(&registry).unwrap()).unwrap(),
        templates,
    )
}
#[test]
fn grid_search_handles_detours_no_path_and_expansion_budget() {
    let mut config = nav::GridConfig {
        width: 8,
        height: 8,
        cell_size: 1.0,
        origin: [0.0, 0.0],
        blocked: (0..7).map(|z| [3, z]).collect(),
    };
    let grid = nav::Grid::new(config.clone()).unwrap();
    let path = grid.search([1, 1], [6, 1], 128);
    assert!(path
        .path
        .as_ref()
        .unwrap()
        .iter()
        .all(|c| grid.walkable(*c)));
    assert!(path.path.unwrap().iter().any(|c| c[1] == 7));
    assert!(grid.search([1, 1], [6, 1], 1).exhausted);
    config.blocked.push([3, 7]);
    assert!(nav::Grid::new(config)
        .unwrap()
        .search([1, 1], [6, 1], 128)
        .path
        .is_none());
}
#[test]
fn rectangle_selection_filters_ownership_and_supports_reverse_drag() {
    let (mut s, _) = setup();
    s.select_rect(RectArgs {
        min: [40.0, 30.0],
        max: [-40.0, -30.0],
        additive: false,
    })
    .unwrap();
    assert_eq!(s.selected.len(), 32);
    assert!(s
        .select(SelectArgs {
            ids: vec!["unit/1/0".into()],
            additive: false
        })
        .is_err());
    assert_eq!(s.selected.len(), 32);
}
#[test]
fn group_move_queues_paths_and_command_replacement_discards_stale_jobs() {
    let (mut s, templates) = setup();
    let ids = (0..8).map(|i| format!("unit/0/{i}")).collect::<Vec<_>>();
    s.move_group(MoveArgs {
        units: Some(ids.clone()),
        destination: [10.5, 0.5],
    })
    .unwrap();
    s.move_group(MoveArgs {
        units: Some(ids),
        destination: [-12.5, -15.5],
    })
    .unwrap();
    for _ in 0..1200 {
        s.step(&templates);
        s.validate().unwrap();
    }
    assert_eq!(s.stats.completed_moves, 8);
    assert!(s.units[..8].iter().all(|u| u.position[0] < 0.0));
    assert_eq!(s.overlaps(), 0);
    assert!(s.stats.max_nav_queue <= 8);
}
#[test]
fn sixty_four_units_group_moves_around_obstacles_without_overlap() {
    let (mut s, templates) = setup();
    let ids = (0..32).map(|i| format!("unit/0/{i}")).collect();
    s.move_group(MoveArgs {
        units: Some(ids),
        destination: [9.5, 0.5],
    })
    .unwrap();
    for tick in 0..3600 {
        s.step(&templates);
        if tick % 120 == 0 {
            s.validate().unwrap();
        }
    }
    assert_eq!(s.units.len(), 64);
    assert_eq!(s.overlaps(), 0);
    assert_eq!(s.stats.no_path, 0);
    assert!(
        s.stats.completed_moves == 32,
        "only {} completed: {:?}",
        s.stats.completed_moves,
        s.summary()
    );
}
#[test]
fn gathering_delivery_and_production_conserve_resources() {
    let (mut s, templates) = setup();
    let initial = s.balance[0];
    s.gather(TargetArgs {
        units: Some(vec!["unit/0/0".into(), "unit/0/1".into()]),
        target: "mineral/0".into(),
    })
    .unwrap();
    s.train(TrainArgs {
        building: "base/0".into(),
        recipe: "worker".into(),
        count: 2,
    })
    .unwrap();
    for _ in 0..3600 {
        s.step(&templates);
        s.validate().unwrap();
    }
    assert_eq!(s.stats.produced, 2);
    assert_eq!(s.units.len(), 66);
    assert!(s.stats.delivered >= 8, "{}", s.summary());
    assert_eq!(s.balance[0] as u64 + 80, initial as u64 + s.stats.delivered);
    assert_eq!(s.stats.spent, 80);
    assert_eq!(s.overlaps(), 0);
}
#[test]
fn insufficient_funds_enemy_orders_and_invalid_targets_are_atomic() {
    let (mut s, _) = setup();
    let before = s.summary();
    assert!(s
        .train(TrainArgs {
            building: "base/0".into(),
            recipe: "soldier".into(),
            count: 8
        })
        .is_err());
    assert!(s
        .train(TrainArgs {
            building: "base/1".into(),
            recipe: "worker".into(),
            count: 1
        })
        .is_err());
    assert!(s
        .move_group(MoveArgs {
            units: Some(vec!["unit/1/0".into()]),
            destination: [-10.0, 0.0]
        })
        .is_err());
    assert!(s
        .gather(TargetArgs {
            units: Some(vec!["unit/0/20".into()]),
            target: "mineral/0".into()
        })
        .is_err());
    assert_eq!(s.summary(), before);
}
#[test]
fn production_cap_and_queue_limits_are_enforced() {
    let (mut s, _) = setup();
    s.session.max_units = 64;
    assert!(s
        .train(TrainArgs {
            building: "base/0".into(),
            recipe: "worker".into(),
            count: 1
        })
        .is_err());
    assert_eq!(s.balance[0], 300);
    assert_eq!(s.stats.produced, 0);
}
#[test]
fn no_path_is_observable_without_invalidating_simulation() {
    let (mut s, templates) = setup();
    let mut config = s.session.grid.clone();
    for z in 0..config.height {
        config.blocked.push([48, z as i32]);
    }
    s.grid = nav::Grid::new(config).unwrap();
    s.move_group(MoveArgs {
        units: Some(vec!["unit/0/0".into()]),
        destination: [9.5, 0.5],
    })
    .unwrap();
    for _ in 0..8 {
        s.step(&templates);
    }
    assert_eq!(s.stats.no_path, 1);
    assert!(s.last_error.as_ref().unwrap().starts_with("no_path:"));
    s.validate().unwrap();
}
#[test]
fn depleted_node_finishes_after_delivery_and_never_goes_negative() {
    let (mut s, templates) = setup();
    s.initial_economy -= s.resources[0].remaining as u64 - 3;
    s.resources[0].remaining = 3;
    s.gather(TargetArgs {
        units: Some(vec!["unit/0/0".into()]),
        target: "mineral/0".into(),
    })
    .unwrap();
    for _ in 0..3000 {
        s.step(&templates);
    }
    s.validate().unwrap();
    assert_eq!(s.resources[0].remaining, 0);
    assert_eq!(s.stats.delivered, 3);
}
#[test]
fn unit_inspection_is_paged_and_projected() {
    let (s, _) = setup();
    let first = s
        .inspect(serde_json::from_value(json!({"fields":["id"],"limit":2})).unwrap())
        .unwrap();
    assert_eq!(first["total"], 64);
    assert_eq!(first["next_offset"], 2);
    assert_eq!(first["units"][0].as_object().unwrap().len(), 1);
}
