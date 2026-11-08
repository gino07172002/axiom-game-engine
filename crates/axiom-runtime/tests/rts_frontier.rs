//! The playable Frontier skirmish: opponent director, towers, combat feedback data and the
//! depot-based outcome. Headless; uses only typed commands and observations.
use axiom_core::gameplay::RuntimeCommand;
use axiom_rts::{simulation::Order, Role};
use axiom_runtime::{rts::RtsSystem, world::WorldRuntime, ActionMap};
use serde_json::{json, Value};

fn frontier() -> WorldRuntime {
    WorldRuntime::new(
        axiom_rts::content::frontier_world().unwrap(),
        1,
        axiom_runtime::installed_registry(),
    )
    .unwrap()
}
fn call(r: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "rts".into(),
        action: action.into(),
        args,
    })
}
fn run(r: &mut WorldRuntime, ticks: u32) {
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
}
fn summary(r: &WorldRuntime) -> Value {
    json!(r.observation())["rts"]["summary"].clone()
}
fn sim(r: &WorldRuntime) -> &axiom_rts::simulation::Simulation {
    &r.system::<RtsSystem>().unwrap().sim
}
fn var(r: &WorldRuntime, id: &str) -> Value {
    json!(r.observation())["gameplay"]["variables"][id].clone()
}

/// Through text, like the exporter (f32 fields serialize in their shortest f32 form).
fn as_written(world: &axiom_core::World) -> Value {
    serde_json::from_slice(&serde_json::to_vec(world).unwrap()).unwrap()
}

#[test]
fn the_shipped_pack_is_the_frontier_generator_output() {
    let generated = as_written(&axiom_rts::content::frontier_world().unwrap());
    let shipped: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/rts/world.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(generated == shipped, "re-run `export_rts_pack frontier`");
    let bench = as_written(&axiom_rts::content::demo_world(64).unwrap());
    let shipped_bench: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/rts-bench/world.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(bench == shipped_bench, "re-run `export_rts_pack 64`");
}

#[test]
fn the_opponent_mines_trains_and_attacks_an_idle_player() {
    let mut r = frontier();
    run(&mut r, 120 * 60);
    let early = summary(&r);
    let enemy_workers = sim(&r)
        .units
        .iter()
        .filter(|u| u.def.faction == 1 && u.def.role == Role::Worker && u.health > 0.0)
        .count();
    assert!(
        enemy_workers > 6,
        "opponent should add workers: {enemy_workers}"
    );
    assert!(
        early["stats"]["delivered"].as_u64().unwrap() > 0,
        "opponent should mine: {early}"
    );
    assert_eq!(
        early["opponent"]["wave"], 0,
        "no wave before first_wave_tick"
    );
    // first_wave_tick is 3 minutes; give the wave time to arrive.
    run(&mut r, 120 * 180);
    let later = summary(&r);
    assert!(later["opponent"]["wave"].as_u64().unwrap() >= 1, "{later}");
    let losses = sim(&r)
        .units
        .iter()
        .filter(|u| u.def.faction == 0 && u.health < u.max_health)
        .count();
    assert!(losses > 0, "the wave must reach the idle player: {later}");
    // Dead units keep their death tick for presentation; shots are recorded.
    assert!(sim(&r).units.iter().any(|u| u.died.is_some()));
    assert!(!sim(&r).shots.is_empty());
    // UI metrics follow the simulation.
    assert_eq!(var(&r, "rts.wave"), later["opponent"]["wave"]);
    assert!(var(&r, "rts.enemy_units").as_i64().unwrap() > 0);
}

#[test]
fn towers_fire_on_intruders_and_buildings_report_health() {
    let mut r = frontier();
    // Walk the four guards next to the enemy's northern tower.
    call(&mut r, "select_role", json!({"role":"soldier"})).unwrap();
    call(&mut r, "move", json!({"destination":[25.5, -9.5]})).unwrap();
    run(&mut r, 120 * 25);
    let hurt = sim(&r)
        .units
        .iter()
        .filter(|u| u.def.faction == 0 && u.def.role == Role::Soldier)
        .any(|u| u.health < u.max_health);
    assert!(hurt, "tower/1/0 should shoot guards inside its 7.5 m range");
    assert!(sim(&r).shots.iter().any(|s| s.faction == 1 && s.ranged));
    assert_eq!(var(&r, "rts.alert"), json!(false));
}

#[test]
fn opponent_waves_engage_defenders_but_player_orders_stay_literal() {
    let targets_unit = |order: &Order| {
        matches!(order, Order::Attack { target }
            if target.starts_with("unit/") || target.starts_with("produced/"))
    };
    // An explicit player attack on a structure keeps its target.
    let mut r = frontier();
    let guards: Vec<String> = sim(&r)
        .units
        .iter()
        .filter(|u| u.def.faction == 0 && u.def.role == Role::Soldier)
        .map(|u| u.id.clone())
        .collect();
    call(
        &mut r,
        "attack",
        json!({"units":guards,"target":"tower/1/1"}),
    )
    .unwrap();
    for _ in 0..(120 * 25 / 4) {
        run(&mut r, 4);
        assert!(!sim(&r)
            .units
            .iter()
            .any(|u| u.def.faction == 0 && u.health > 0.0 && targets_unit(&u.order)));
    }
    // An opponent wave marching on our structures turns on defenders it meets in the field.
    let mut r = frontier();
    run(&mut r, 120 * 150);
    assert_eq!(summary(&r)["opponent"]["wave"], 1);
    let mut intercepted = false;
    for _ in 0..(120 * 60 / 4) {
        run(&mut r, 4);
        intercepted |= sim(&r).units.iter().any(|u| {
            u.def.faction == 1 && u.health > 0.0 && u.position[0] < 0.0 && targets_unit(&u.order)
        });
        if intercepted {
            break;
        }
    }
    assert!(intercepted, "{}", summary(&r));
}

/// A plain human-like build: two mining groups, defend the base when attacked, then push
/// towers and the HQ with a large army. Commands go through the same typed calls as the UI.
#[test]
fn an_active_player_defends_and_wins() {
    let mut r = frontier();
    let mut attacking = false;
    let mut alternate = 0;
    for _ in 0..(15 * 60 / 2) {
        run(&mut r, 240);
        let s = sim(&r);
        if s.victory || s.defeated {
            break;
        }
        let mine = |role: Role| -> Vec<(String, Order, [f32; 2])> {
            s.units
                .iter()
                .filter(|u| u.def.faction == 0 && u.health > 0.0 && u.def.role == role)
                .map(|u| (u.id.clone(), u.order.clone(), u.position))
                .collect()
        };
        let workers = mine(Role::Worker);
        let army = mine(Role::Soldier);
        if std::env::var_os("FRONTIER_TRACE").is_some() && s.tick.is_multiple_of(3600) {
            let theirs = |role: Role| {
                s.units
                    .iter()
                    .filter(|u| u.def.faction == 1 && u.health > 0.0 && u.def.role == role)
                    .count()
            };
            eprintln!(
                "t={}s us {}w/{}a {}m | them {}w/{}a {}m | hq {:?} towers {:?}",
                s.tick / 120,
                workers.len(),
                army.len(),
                s.balance[0],
                theirs(Role::Worker),
                theirs(Role::Soldier),
                s.balance[1],
                s.buildings
                    .iter()
                    .filter(|b| b.def.depot)
                    .map(|b| b.health)
                    .collect::<Vec<_>>(),
                s.buildings
                    .iter()
                    .filter(|b| !b.def.depot)
                    .map(|b| b.health)
                    .collect::<Vec<_>>(),
            );
        }
        let home = [-36.5f32, 0.5];
        let near_home = |p: [f32; 2]| (p[0] - home[0]).hypot(p[1] - home[1]) < 20.0;
        let intruder = s
            .units
            .iter()
            .filter(|u| u.def.faction == 1 && u.health > 0.0 && near_home(u.position))
            .min_by(|a, b| {
                let d = |p: [f32; 2]| (p[0] - home[0]).hypot(p[1] - home[1]);
                d(a.position).total_cmp(&d(b.position))
            })
            .map(|u| u.id.clone());
        let mut orders: Vec<(&str, Value)> = vec![];
        // Mine the two closest non-empty nodes (home first, then the contested center).
        let mut nodes: Vec<&axiom_rts::simulation::Resource> =
            s.resources.iter().filter(|n| n.remaining > 0).collect();
        nodes.sort_by(|a, b| {
            let d = |p: [f32; 2]| (p[0] - home[0]).hypot(p[1] - home[1]);
            d(a.position).total_cmp(&d(b.position))
        });
        for (k, (id, order, _)) in workers.iter().enumerate() {
            if let (Order::Idle, Some(node)) = (order, nodes.get(k % nodes.len().clamp(1, 2))) {
                orders.push(("gather", json!({"units":[id],"target":node.id})));
            }
        }
        if s.buildings[0].queue.len() < 2 {
            let next = if workers.len() < 12 {
                "worker"
            } else {
                alternate += 1;
                if alternate % 2 == 0 {
                    "guard"
                } else {
                    "ranger"
                }
            };
            orders.push((
                "train",
                json!({"building":"base/0","recipe":next,"count":1}),
            ));
        }
        if let Some(enemy) = intruder {
            let defenders: Vec<&String> = army
                .iter()
                .filter(|(_, o, p)| near_home(*p) && !matches!(o, Order::Attack { .. }))
                .map(|(id, ..)| id)
                .collect();
            if !defenders.is_empty() {
                orders.push(("attack", json!({"units":defenders,"target":enemy})));
            }
        } else {
            attacking |= army.len() >= 18;
            let idle: Vec<&String> = army
                .iter()
                .filter(|(_, o, _)| *o == Order::Idle)
                .map(|(id, ..)| id)
                .collect();
            let target = ["tower/1/0", "base/1"]
                .into_iter()
                .find(|id| s.buildings.iter().any(|b| &b.id == id && b.health > 0.0));
            if let (true, false, Some(target)) = (attacking, idle.is_empty(), target) {
                orders.push((
                    "attack",
                    json!({"units":idle,"target":target,"engage":true}),
                ));
            }
        }
        for (action, args) in orders {
            let _ = call(&mut r, action, args);
        }
    }
    let s = summary(&r);
    eprintln!(
        "frontier scripted player: victory={} at tick {} (waves faced {})",
        s["victory"], s["tick"], s["opponent"]["wave"]
    );
    assert_eq!(s["victory"], true, "{s}");
    assert_eq!(s["defeated"], false);
    assert_eq!(var(&r, "rts.victory"), json!(true));
    assert_eq!(var(&r, "rts.enemy_base_health"), json!(0));
    // Winning stops the director.
    let wave = s["opponent"]["wave"].clone();
    run(&mut r, 120 * 30);
    assert_eq!(summary(&r)["opponent"]["wave"], wave);
}

#[test]
fn opponent_and_tower_contracts_are_validated() {
    let registry = axiom_runtime::installed_registry();
    let base = axiom_rts::content::frontier_world().unwrap();
    let mut player_ai = base.clone();
    player_ai.resources.get_mut("rts.session").unwrap()["opponent"]["faction"] = json!(0);
    assert!(player_ai.compile(&registry.authoring).is_err());
    let mut missing = base.clone();
    missing.resources.get_mut("rts.session").unwrap()["opponent"]["army"] = json!(["catapult"]);
    let error = format!("{:#}", missing.compile(&registry.authoring).unwrap_err());
    assert!(error.contains("catapult"), "{error}");
    let mut tower = base.clone();
    tower.entity_mut("tower/0/0").unwrap().components.insert(
        "rts.building".into(),
        json!({"faction":0,"depot":false,"produces":[],"attack":{"damage":10,"range":99,"interval_ticks":60}}),
    );
    assert!(tower.compile(&registry.authoring).is_err());
    let mut style = base;
    style.resources.get_mut("ui.document").unwrap()["styles"]["ghost"] = json!("title");
    assert!(style.compile(&registry.authoring).is_err());
}

#[test]
fn the_minimap_camera_command_recenters_the_view() {
    let mut r = frontier();
    let result = call(&mut r, "camera", json!({"center":[30.0, 0.5]})).unwrap();
    let camera = result["result"]["camera"].clone();
    // Looking down at 60 degrees from 40 m: the eye sits ~23.1 m behind the ground point.
    assert!(
        (camera[0].as_f64().unwrap() - 30.0).abs() < 1e-3,
        "{camera}"
    );
    assert!((camera[1].as_f64().unwrap() - (0.5 + 40.0 / 3f64.sqrt())).abs() < 1e-2);
    assert!(call(&mut r, "camera", json!({"center":[f64::NAN, 0.0]})).is_err());
}

#[test]
fn frontier_runs_are_deterministic() {
    let hash = || {
        let mut r = frontier();
        run(&mut r, 120 * 170);
        summary(&r).to_string()
    };
    assert_eq!(hash(), hash());
}
