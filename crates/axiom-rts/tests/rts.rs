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
        engage: false,
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
            target: "mineral/0".into(),
            engage: false,
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
        engage: false,
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
fn compiled(w: &World) -> CompiledWorld {
    let mut registry = Registry::core();
    register(&mut registry);
    w.compile(&registry).unwrap()
}
#[test]
fn shared_obstacles_merge_with_duplicate_authored_cells_without_exceeding_the_cell_budget() {
    let (mut s, templates) = setup();
    let grid = s.session.grid.clone();
    // Legal on its own: duplicates of one cell up to the list budget.
    s.session.grid.blocked = vec![[0, 0]; 16384];
    nav::Grid::new(s.session.grid.clone()).unwrap();
    let mut scene = axiom_combat::Scene::new(axiom_combat::Config {
        bounds: axiom_combat::Rect {
            min: [-1000.0, -1000.0],
            max: [1000.0, 1000.0],
        },
        obstacles: vec![axiom_combat::Rect {
            min: [-999.0, -999.0],
            max: [999.0, 999.0],
        }],
    });
    s.set_combat(Some(scene.clone()));
    assert!(s.last_error.is_none(), "{:?}", s.last_error);
    assert!(!s.grid.walkable([1, 1]) && !s.grid.walkable([0, 0]));
    // A real shared layout keeps the authored cells and adds the covered ones.
    s.session.grid = grid.clone();
    scene.config.obstacles = vec![axiom_combat::Rect {
        min: [grid.origin[0], grid.origin[1]],
        max: [
            grid.origin[0] + grid.cell_size,
            grid.origin[1] + grid.cell_size,
        ],
    }];
    s.set_combat(Some(scene));
    assert!(!s.grid.walkable([0, 0]));
    for c in &grid.blocked {
        assert!(!s.grid.walkable(*c));
    }
    s.step(&templates);
}
#[test]
fn starting_balance_plus_world_minerals_must_fit_and_delivery_never_wraps() {
    let mut w = content::demo_world(64).unwrap();
    let mut registry = Registry::core();
    register(&mut registry);
    w.resources.get_mut("rts.session").unwrap()["initial_balance"] = json!([u32::MAX, 0]);
    // Strict on write: checked for new content, not when a stored revision is read.
    w.compile(&registry).unwrap();
    let e = format!("{:#}", registry.validate_write(&w, None).unwrap_err());
    assert!(e.contains("initial_balance"), "{e}");
    // A balance raised to the limit (e.g. by a reload) stops delivery instead of wrapping.
    let (mut s, templates) = setup();
    let top = u32::MAX - 1;
    s.initial_economy += u64::from(top - s.balance[0]);
    s.balance[0] = top;
    s.gather(TargetArgs {
        units: Some(vec!["unit/0/0".into()]),
        target: "mineral/0".into(),
        engage: false,
    })
    .unwrap();
    for _ in 0..3600 {
        s.step(&templates);
        s.validate().unwrap();
    }
    assert_eq!(s.balance[0], top);
    assert_eq!(s.stats.delivered, 0);
    assert!(s.units[0].carried > 0);
}
#[test]
fn hot_reload_adjusts_the_conservation_baseline_for_added_and_removed_nodes() {
    let w = content::demo_world(64).unwrap();
    let (_, templates) = setup();
    let mut old = Simulation::new(&compiled(&w)).unwrap();
    old.gather(TargetArgs {
        units: Some(vec!["unit/0/0".into(), "unit/0/1".into()]),
        target: "mineral/0".into(),
        engage: false,
    })
    .unwrap();
    for _ in 0..900 {
        old.step(&templates);
    }
    old.validate().unwrap();
    assert!(old.stats.harvested > 0);
    let mut edited = w.clone();
    let mut extra = edited.entity("mineral/0").unwrap().clone();
    extra.id = "mineral/extra".into();
    extra.components.get_mut("core.transform").unwrap()["position"][0] = json!(3.5);
    extra.components.get_mut("rts.resource").unwrap()["amount"] = json!(77);
    edited.entities.retain(|e| e.id != "mineral/1");
    edited.entities.push(extra);
    let mut next = Simulation::new(&compiled(&edited)).unwrap();
    next.restore(&old).unwrap();
    next.validate().unwrap();
    let removed = old
        .resources
        .iter()
        .find(|r| r.id == "mineral/1")
        .unwrap()
        .remaining as u64;
    assert_eq!(next.initial_economy, old.initial_economy + 77 - removed);
    for _ in 0..600 {
        next.step(&templates);
        next.validate().unwrap();
    }
}
#[test]
fn a_stored_world_over_the_balance_limit_opens_repairs_and_undoes() {
    let mut registry = Registry::core();
    register(&mut registry);
    let world = content::demo_world(64).unwrap();
    let dir = std::env::temp_dir().join(format!("axiom-rts-balance-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("world.sqlite");
    let open = || Store::open_with(&path, registry.clone(), world.clone());
    drop(open().unwrap());
    // Stored before the balance limit existed.
    let mut legacy = json!(world);
    legacy["resources"]["rts.session"]["initial_balance"] = json!([u32::MAX, 0]);
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "INSERT INTO revisions(world,reason) VALUES (?1,'legacy')",
            [legacy.to_string()],
        )
        .unwrap();
    let mut s = open().expect("a stored world over the limit must not block opening");
    let verify = s.execute(Request::Verify {}).unwrap();
    assert_eq!(verify["ok"], true, "{verify}");
    assert_eq!(
        verify["diagnostics"][0]["path"], "/resources/rts.session/initial_balance/0",
        "{verify}"
    );
    // An edit that leaves the old violation alone commits; so does the repair.
    assert_eq!(
        s.apply(Some(2), vec![Edit::set("title", json!("Renamed"))])
            .unwrap()["rev"],
        3
    );
    let repair = Edit::set("/resources/rts.session/initial_balance/0", json!(150));
    assert_eq!(s.apply(Some(3), vec![repair]).unwrap()["rev"], 4);
    // A repaired world cannot be pushed over the limit again.
    let over = Edit::set("/resources/rts.session/initial_balance/1", json!(u32::MAX));
    let e = format!("{:#}", s.apply(Some(4), vec![over]).unwrap_err());
    assert!(e.contains("initial_balance/1"), "{e}");
    // Undo back to the stored revision, then reopen.
    let undone = s
        .execute(Request::Undo {
            rev: 2,
            expected_rev: Some(4),
        })
        .unwrap();
    assert_eq!(undone["rev"], 5, "{undone}");
    drop(s);
    let reopened = open().unwrap();
    assert_eq!(
        reopened.current().unwrap().1.resources["rts.session"]["initial_balance"],
        json!([u32::MAX, 0])
    );
    drop(reopened);
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn restore_refuses_a_transferred_balance_that_no_longer_fits_with_the_minerals() {
    // R7 F3 (round 3): the old run's balance carries over on reload; with every node still
    // minable it must fit a u32 together with the minerals and what the workers carry.
    let w = content::demo_world(64).unwrap();
    let mut old = Simulation::new(&compiled(&w)).unwrap();
    let minerals: u64 = old.resources.iter().map(|r| u64::from(r.remaining)).sum();
    let top = u32::MAX - minerals as u32;
    old.initial_economy += u64::from(top - old.balance[1]);
    old.balance[1] = top;
    let mut same = Simulation::new(&compiled(&w)).unwrap();
    same.restore(&old).unwrap();
    assert_eq!(same.balance[1], top);
    old.initial_economy += 1;
    old.balance[1] = top + 1;
    let mut over = Simulation::new(&compiled(&w)).unwrap();
    let error = format!("{:#}", over.restore(&old).unwrap_err());
    assert!(
        error.contains("faction 1") && error.contains("exceeds"),
        "{error}"
    );
}
#[test]
fn shared_obstacle_cells_match_the_full_scan_and_stay_bounded() {
    // R7 F2 (round 3): the per-obstacle raster gives the same mask as testing every cell
    // against every obstacle, for odd sizes, cell sizes, sub-cell and huge rectangles, and
    // rectangles partly or fully outside the grid.
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    for (width, height, cell_size) in [(128, 128, 1.0), (37, 91, 0.5), (4, 4, 4.0), (100, 64, 1.3)]
    {
        let config = nav::GridConfig {
            width,
            height,
            cell_size,
            origin: [-20.0, -11.5],
            blocked: vec![],
        };
        let extent = [width as f32 * cell_size, height as f32 * cell_size];
        let mut obstacles = vec![
            axiom_combat::Rect {
                min: [-1000.0, -1000.0],
                max: [-999.0, 1000.0],
            },
            axiom_combat::Rect {
                min: [-19.9, -11.4],
                max: [-19.8, -11.3],
            },
            axiom_combat::Rect {
                min: [-20.0, -11.5],
                max: [-20.0 + cell_size, -11.5 + cell_size],
            },
        ];
        for _ in 0..300 {
            let size = if next() < 0.1 { 30.0 } else { 3.0 };
            let x = -25.0 + next() * (extent[0] + 10.0);
            let z = -16.5 + next() * (extent[1] + 10.0);
            obstacles.push(axiom_combat::Rect {
                min: [x, z],
                max: [x + next() * size, z + next() * size],
            });
        }
        let mut fast = vec![false; (width * height) as usize];
        simulation::shared_obstacle_mask(&config, &obstacles, &mut fast);
        for z in 0..height as i32 {
            for x in 0..width as i32 {
                let center = glam::Vec2::new(
                    config.origin[0] + (x as f32 + 0.5) * cell_size,
                    config.origin[1] + (z as f32 + 0.5) * cell_size,
                );
                let full = obstacles.iter().any(|o| !o.clear(center, cell_size * 0.49));
                assert_eq!(
                    fast[(z * width as i32 + x) as usize],
                    full,
                    "{width}x{height} cell {x},{z}"
                );
            }
        }
    }
    // The budget case: 128x128 cells, 2048 obstacles of up to the whole grid. Timed for the
    // record (debug or release as built); the old scan ran 16384 x 2048 distance tests.
    let config = nav::GridConfig {
        width: 128,
        height: 128,
        cell_size: 1.0,
        origin: [-64.0, -64.0],
        blocked: vec![],
    };
    let obstacles: Vec<_> = (0..2048)
        .map(|i| {
            let (a, b) = (next() * 128.0 - 64.0, next() * 128.0 - 64.0);
            let size = if i % 8 == 0 { 128.0 } else { 4.0 };
            axiom_combat::Rect {
                min: [a, b],
                max: [a + next() * size, b + next() * size],
            }
        })
        .collect();
    let mut mask = vec![false; 128 * 128];
    let started = std::time::Instant::now();
    simulation::shared_obstacle_mask(&config, &obstacles, &mut mask);
    let fast = started.elapsed();
    let started = std::time::Instant::now();
    let mut full = vec![false; 128 * 128];
    for z in 0..128 {
        for x in 0..128 {
            let center = glam::Vec2::new(-64.0 + x as f32 + 0.5, -64.0 + z as f32 + 0.5);
            full[z * 128 + x] = obstacles.iter().any(|o| !o.clear(center, 0.49));
        }
    }
    let scan = started.elapsed();
    assert_eq!(mask, full);
    eprintln!(
        "shared obstacle mask 128x128 / 2048 obstacles ({}): raster {:.2} ms, full scan {:.2} ms",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        fast.as_secs_f64() * 1000.0,
        scan.as_secs_f64() * 1000.0
    );
}
