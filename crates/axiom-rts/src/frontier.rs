//! "Frontier": the playable RTS skirmish shipped in `content/rts`.
//! Three lanes separated by rock ridges, home and contested crystal fields, defensive towers and
//! a data-driven opponent director. `content::demo_world` stays the fixed benchmark fixture.
use crate::*;

const TEAM: [f32; 3] = [0.16, 0.5, 0.95];
const TEAM_DARK: [f32; 3] = [0.09, 0.24, 0.5];
const TEAM_LIGHT: [f32; 3] = [0.55, 0.85, 1.0];
const ENEMY: [f32; 3] = [0.9, 0.22, 0.17];
const ENEMY_DARK: [f32; 3] = [0.45, 0.1, 0.08];
const ENEMY_LIGHT: [f32; 3] = [1.0, 0.62, 0.4];
const SKIN: [f32; 3] = [0.96, 0.8, 0.66];
const STEEL: [f32; 3] = [0.7, 0.74, 0.78];
const STONE: [f32; 3] = [0.52, 0.5, 0.47];
const WOOD: [f32; 3] = [0.45, 0.3, 0.17];

fn yaw(angle: f32) -> [f32; 4] {
    [0.0, (angle * 0.5).sin(), 0.0, (angle * 0.5).cos()]
}
fn part(position: [f32; 3], scale: [f32; 3], color: [f32; 3], emission: f32) -> MeshPart {
    part_r(position, scale, 0.0, color, emission)
}
fn part_r(
    position: [f32; 3],
    scale: [f32; 3],
    angle: f32,
    color: [f32; 3],
    emission: f32,
) -> MeshPart {
    MeshPart {
        transform: Transform {
            position,
            rotation: yaw(angle),
            scale,
        },
        color,
        emission,
        grid: false,
    }
}
fn team_pairs() -> Vec<[[f32; 3]; 2]> {
    vec![
        [TEAM, ENEMY],
        [TEAM_DARK, ENEMY_DARK],
        [TEAM_LIGHT, ENEMY_LIGHT],
    ]
}
fn recolor(parts: &mut [MeshPart]) {
    for p in parts {
        if let Some(pair) = team_pairs().into_iter().find(|pair| pair[0] == p.color) {
            p.color = pair[1];
        }
    }
}
/// Deterministic decoration noise in 0..1.
fn noise(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x % 10_000) as f32 / 10_000.0
}
fn prop(id: String, position: [f32; 3], parts: Vec<MeshPart>) -> Result<Entity> {
    let mut e = Entity::new(id);
    e.set(
        "core.transform",
        Transform {
            position,
            ..Default::default()
        },
    )?;
    e.set("render.mesh", RenderMesh { parts })?;
    Ok(e)
}
/// World position -> grid cell for the 96x64 map with origin (-48,-32).
fn cell(x: f32, z: f32) -> [i32; 2] {
    [(x + 48.0).floor() as i32, (z + 32.0).floor() as i32]
}

fn unit_meshes() -> [(&'static str, UnitDef, f32, Vec<MeshPart>); 3] {
    let worker = UnitDef {
        faction: 0,
        role: Role::Worker,
        speed: 4.0,
        radius: 0.26,
        carry_capacity: 8,
        gather_ticks: 20,
        damage: 4.0,
        attack_range: 1.5,
        attack_ticks: 120,
    };
    let guard = UnitDef {
        role: Role::Soldier,
        speed: 3.8,
        carry_capacity: 0,
        damage: 12.0,
        attack_range: 1.6,
        attack_ticks: 100,
        radius: 0.3,
        ..worker.clone()
    };
    let ranger = UnitDef {
        role: Role::Soldier,
        speed: 4.4,
        carry_capacity: 0,
        damage: 8.0,
        attack_range: 6.0,
        attack_ticks: 100,
        ..worker.clone()
    };
    [
        (
            "worker",
            worker,
            60.0,
            vec![
                part([0.0, 0.32, 0.0], [0.4, 0.46, 0.34], TEAM, 0.08),
                part([0.0, 0.1, 0.0], [0.36, 0.2, 0.3], TEAM_DARK, 0.0),
                part([0.0, 0.72, 0.0], [0.26, 0.26, 0.26], SKIN, 0.0),
                part(
                    [0.0, 0.88, 0.0],
                    [0.32, 0.09, 0.32],
                    [0.98, 0.78, 0.12],
                    0.15,
                ),
                part([0.0, 0.4, -0.22], [0.3, 0.32, 0.14], WOOD, 0.0),
                part_r([0.26, 0.42, 0.08], [0.06, 0.5, 0.06], 0.4, STEEL, 0.0),
            ],
        ),
        (
            "guard",
            guard,
            160.0,
            vec![
                part([0.0, 0.36, 0.0], [0.5, 0.52, 0.4], TEAM, 0.08),
                part([0.0, 0.1, 0.0], [0.44, 0.2, 0.36], TEAM_DARK, 0.0),
                part([0.0, 0.8, 0.0], [0.28, 0.28, 0.28], SKIN, 0.0),
                part([0.0, 0.96, 0.0], [0.34, 0.14, 0.34], STEEL, 0.1),
                part([0.32, 0.42, 0.06], [0.1, 0.5, 0.42], TEAM_LIGHT, 0.2),
                part([-0.3, 0.55, 0.2], [0.06, 0.08, 0.62], STEEL, 0.35),
            ],
        ),
        (
            "ranger",
            ranger,
            80.0,
            vec![
                part([0.0, 0.34, 0.0], [0.4, 0.5, 0.34], TEAM, 0.08),
                part([0.0, 0.1, 0.0], [0.36, 0.2, 0.3], [0.2, 0.32, 0.18], 0.0),
                part([0.0, 0.76, 0.0], [0.26, 0.26, 0.26], SKIN, 0.0),
                part(
                    [0.0, 0.9, -0.03],
                    [0.32, 0.16, 0.34],
                    [0.18, 0.36, 0.2],
                    0.0,
                ),
                part([0.27, 0.55, 0.12], [0.05, 0.8, 0.08], WOOD, 0.0),
                part([0.0, 0.5, -0.22], [0.16, 0.38, 0.12], TEAM_LIGHT, 0.25),
            ],
        ),
    ]
}

fn hq_parts() -> Vec<MeshPart> {
    let mut parts = vec![
        part([0.0, 0.1, 0.0], [4.4, 0.3, 4.4], STONE, 0.0),
        part([0.0, 0.95, 0.0], [3.0, 1.5, 3.0], TEAM_DARK, 0.05),
        part([0.0, 1.85, 0.0], [3.3, 0.3, 3.3], TEAM, 0.15),
        part([0.0, 2.3, 0.0], [1.6, 0.6, 1.6], STONE, 0.0),
        part_r([0.0, 3.05, 0.0], [0.8, 0.9, 0.8], 0.785, TEAM_LIGHT, 1.6),
        part([0.0, 0.7, 1.52], [1.0, 1.1, 0.1], [0.12, 0.1, 0.09], 0.0),
    ];
    for (x, z) in [(-1.6, -1.6), (1.6, -1.6), (-1.6, 1.6), (1.6, 1.6)] {
        parts.push(part([x, 1.2, z], [0.5, 2.4, 0.5], STEEL, 0.0));
        parts.push(part([x, 2.5, z], [0.3, 0.3, 0.3], TEAM_LIGHT, 1.2));
    }
    parts
}
fn tower_parts() -> Vec<MeshPart> {
    vec![
        part([0.0, 0.25, 0.0], [2.0, 0.5, 2.0], STONE, 0.0),
        part([0.0, 1.6, 0.0], [1.1, 2.4, 1.1], [0.62, 0.6, 0.56], 0.0),
        part([0.0, 2.95, 0.0], [1.6, 0.35, 1.6], TEAM, 0.15),
        part_r([0.0, 3.5, 0.0], [0.5, 0.7, 0.5], 0.785, TEAM_LIGHT, 2.0),
    ]
}
fn crystal_parts(seed: u32) -> Vec<MeshPart> {
    let mut parts = vec![part(
        [0.0, 0.05, 0.0],
        [2.2, 0.15, 2.0],
        [0.3, 0.33, 0.35],
        0.0,
    )];
    for k in 0..6 {
        let a = noise(seed * 11 + k) * std::f32::consts::TAU;
        let r = if k == 0 {
            0.0
        } else {
            0.45 + noise(seed * 13 + k) * 0.4
        };
        let h = 0.7 + noise(seed * 17 + k) * 1.1;
        parts.push(part_r(
            [a.cos() * r, h * 0.5, a.sin() * r],
            [0.32, h, 0.32],
            a,
            [0.25 + noise(k) * 0.1, 0.85 + noise(k + 3) * 0.1, 0.95],
            1.1,
        ));
    }
    parts
}
fn rock_parts(seed: u32, height: f32) -> Vec<MeshPart> {
    let shade = 0.38 + noise(seed) * 0.12;
    let mut parts = vec![part_r(
        [0.0, height * 0.5, 0.0],
        [1.05, height, 1.05],
        noise(seed + 1) * 1.5,
        [shade, shade * 0.95, shade * 0.88],
        0.0,
    )];
    if noise(seed + 2) > 0.4 {
        parts.push(part_r(
            [0.2, height + 0.15, -0.1],
            [0.6, 0.4, 0.55],
            noise(seed + 3) * 2.0,
            [shade + 0.08, shade + 0.06, shade],
            0.0,
        ));
    }
    parts
}
fn tree_parts(seed: u32) -> Vec<MeshPart> {
    let h = 1.0 + noise(seed) * 0.8;
    let green = [
        0.16 + noise(seed + 1) * 0.08,
        0.42 + noise(seed + 2) * 0.12,
        0.2,
    ];
    vec![
        part([0.0, h * 0.5, 0.0], [0.28, h, 0.28], WOOD, 0.0),
        part_r(
            [0.0, h + 0.4, 0.0],
            [1.5, 0.9, 1.5],
            noise(seed + 3),
            green,
            0.0,
        ),
        part_r(
            [0.0, h + 1.1, 0.0],
            [1.0, 0.7, 1.0],
            noise(seed + 4) + 0.6,
            [green[0] + 0.05, green[1] + 0.08, green[2] + 0.04],
            0.0,
        ),
        part_r(
            [0.0, h + 1.6, 0.0],
            [0.5, 0.45, 0.5],
            noise(seed + 5),
            [green[0] + 0.1, green[1] + 0.14, green[2] + 0.06],
            0.0,
        ),
    ]
}

pub fn frontier_world() -> Result<World> {
    let mut w = World::empty("frontier");
    w.title = "Frontier / RTS skirmish".into();
    w.capabilities.extend([
        ("render".into(), 1),
        ("rts".into(), 1),
        ("behavior".into(), 1),
        ("ui".into(), 1),
    ]);
    w.systems.push("behavior.execute".into());
    w.systems.push("rts.simulate".into());
    let units = unit_meshes();
    for (name, def, hp, parts) in &units {
        // Visual scale only; the navigation radius stays in the unit definition.
        let parts: Vec<MeshPart> = parts
            .iter()
            .map(|p| {
                let mut p = p.clone();
                p.transform.position = p.transform.position.map(|v| v * 1.3);
                p.transform.scale = p.transform.scale.map(|v| v * 1.3);
                p
            })
            .collect();
        let mut e = Entity::new(*name);
        e.set("core.transform", Transform::default())?;
        e.set(
            "core.health",
            Health {
                value: *hp,
                max: *hp,
            },
        )?;
        e.set("rts.unit", def)?;
        e.set("render.mesh", RenderMesh { parts })?;
        w.prefabs.insert(
            (*name).into(),
            Prefab {
                components: e.components,
            },
        );
    }
    let mut blocked: Vec<[i32; 2]> = vec![];
    let mut block = |c: [i32; 2]| {
        if !blocked.contains(&c) {
            blocked.push(c);
        }
    };
    // Starting forces: 6 workers + 4 guards per side, mirrored.
    for faction in 0..2u8 {
        let s = if faction == 0 { 1.0 } else { -1.0 };
        let mut n = 0;
        let mut spawn = |w: &mut World, recipe: &str, x: f32, z: f32| -> Result<()> {
            let mut e = Entity::new(format!("unit/{faction}/{n}"));
            n += 1;
            e.prefab = Some(recipe.into());
            e.set(
                "core.transform",
                Transform {
                    position: [x * s, 0.0, z],
                    ..Default::default()
                },
            )?;
            e.set("rts.unit", json!({"faction":faction}))?;
            if faction == 1 {
                let mut mesh: RenderMesh =
                    serde_json::from_value(w.prefabs[recipe].components["render.mesh"].clone())?;
                recolor(&mut mesh.parts);
                e.set("render.mesh", mesh)?;
            }
            e.tags = vec![format!("faction/{faction}"), recipe.into()];
            w.entities.push(e);
            Ok(())
        };
        for i in 0..6 {
            spawn(
                &mut w,
                "worker",
                -33.5 + (i % 3) as f32 * 1.2,
                -3.5 + (i / 3) as f32 * 1.2,
            )?;
        }
        for i in 0..4 {
            spawn(
                &mut w,
                "guard",
                -32.5 + (i % 2) as f32 * 1.2,
                3.5 + (i / 2) as f32 * 1.2,
            )?;
        }
    }
    // Headquarters and towers.
    for faction in 0..2u8 {
        let s = if faction == 0 { -1.0 } else { 1.0 };
        let mut hq_mesh = hq_parts();
        let mut tower_mesh = tower_parts();
        if faction == 1 {
            recolor(&mut hq_mesh);
            recolor(&mut tower_mesh);
        }
        let mut hq = prop(format!("base/{faction}"), [36.5 * s, 0.0, 0.5], hq_mesh)?;
        hq.set(
            "core.health",
            Health {
                value: 2500.0,
                max: 2500.0,
            },
        )?;
        hq.set(
            "rts.building",
            BuildingDef {
                faction,
                depot: true,
                produces: vec!["worker".into(), "guard".into(), "ranger".into()],
                attack: None,
            },
        )?;
        hq.tags = vec![format!("faction/{faction}"), "hq".into()];
        w.entities.push(hq);
        let c = cell(36.5 * s, 0.5);
        for dz in -1..=1 {
            for dx in -1..=1 {
                block([c[0] + dx, c[1] + dz]);
            }
        }
        for (k, z) in [(0, -6.5f32), (1, 7.5)] {
            let mut tower = prop(
                format!("tower/{faction}/{k}"),
                [28.5 * s, 0.0, z],
                tower_mesh.clone(),
            )?;
            tower.set(
                "core.health",
                Health {
                    value: 800.0,
                    max: 800.0,
                },
            )?;
            tower.set(
                "rts.building",
                BuildingDef {
                    faction,
                    depot: false,
                    produces: vec![],
                    attack: Some(BuildingAttack {
                        damage: 12.0,
                        range: 7.5,
                        interval_ticks: 90,
                    }),
                },
            )?;
            tower.tags = vec![format!("faction/{faction}"), "tower".into()];
            w.entities.push(tower);
            let c = cell(28.5 * s, z);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    block([c[0] + dx, c[1] + dz]);
                }
            }
        }
    }
    // Crystal fields: two home nodes per side and two contested nodes on the outer lanes.
    let nodes = [
        ("mineral/0/a", [-40.5, -8.5], 1500),
        ("mineral/0/b", [-40.5, 9.5], 1500),
        ("mineral/1/a", [40.5, -8.5], 1500),
        ("mineral/1/b", [40.5, 9.5], 1500),
        ("mineral/north", [0.5, -23.5], 2500),
        ("mineral/south", [0.5, 24.5], 2500),
    ];
    for (k, (id, p, amount)) in nodes.iter().enumerate() {
        let mut node = prop((*id).into(), [p[0], 0.0, p[1]], crystal_parts(k as u32 + 1))?;
        node.set("rts.resource", ResourceDef { amount: *amount })?;
        w.entities.push(node);
        block(cell(p[0], p[1]));
    }
    // Rock ridges split the map into north / middle / south lanes; boulders guard the flanks.
    let mut rocks = vec![];
    for z in (-16..=-5).chain(5..=16) {
        for x in [-1, 0] {
            rocks.push((x as f32 + 0.5, z as f32 + 0.5));
        }
    }
    for (cx, cz) in [(-17.0, -12.0), (17.0, -12.0), (-17.0, 13.0), (17.0, 13.0)] {
        for dx in 0..3 {
            for dz in 0..2 {
                rocks.push((cx + dx as f32 - 0.5, cz + dz as f32 - 0.5));
            }
        }
    }
    for (k, (x, z)) in rocks.iter().enumerate() {
        let h = 0.9 + noise(k as u32 * 7) * 1.3;
        w.entities.push(prop(
            format!("rock/{k}"),
            [*x, 0.0, *z],
            rock_parts(k as u32 * 5, h),
        )?);
        block(cell(*x, *z));
    }
    // Groves along the map edges (blocking their trunk cells).
    let mut groves = vec![];
    for k in 0..34u32 {
        let x = -46.5 + (k as f32 * 2.83) % 93.0;
        let z = if k % 2 == 0 { -29.5 } else { 29.5 } + (noise(k) - 0.5) * 2.0;
        groves.push((x.floor() + 0.5, z.floor() + 0.5));
    }
    for k in 0..16u32 {
        let x = if k % 2 == 0 { -46.5 } else { 46.5 };
        groves.push((x, -20.5 + (k / 2) as f32 * 5.5));
    }
    for (k, (x, z)) in groves.iter().enumerate() {
        w.entities.push(prop(
            format!("tree/{k}"),
            [*x, 0.0, *z],
            tree_parts(k as u32 * 3 + 1),
        )?);
        block(cell(*x, *z));
    }
    // Ground: outer apron, play field, grass patches and dirt roads (flat, non-blocking).
    w.entities.push(prop(
        "ground/apron".into(),
        [0.0, -0.45, 0.0],
        vec![part(
            [0.0, 0.0, 0.0],
            [160.0, 0.5, 110.0],
            [0.24, 0.32, 0.21],
            0.0,
        )],
    )?);
    w.entities.push(prop(
        "ground".into(),
        [0.0, -0.35, 0.0],
        vec![part(
            [0.0, 0.0, 0.0],
            [96.0, 0.5, 64.0],
            [0.3, 0.42, 0.25],
            0.0,
        )],
    )?);
    let mut patches = vec![];
    for k in 0..46u32 {
        let x = (noise(k * 3) - 0.5) * 90.0;
        let z = (noise(k * 3 + 1) - 0.5) * 58.0;
        let size = 3.0 + noise(k * 3 + 2) * 6.0;
        let tone = noise(k + 99);
        patches.push(part_r(
            [x, 0.0, z],
            [size, 0.02, size * (0.5 + tone * 0.6)],
            tone * 3.0,
            [0.29 + tone * 0.04, 0.4 + tone * 0.05, 0.24],
            0.0,
        ));
    }
    w.entities
        .push(prop("ground/grass".into(), [0.0, -0.095, 0.0], patches)?);
    let road = [0.5, 0.42, 0.3];
    let roads = vec![
        part([0.0, 0.0, 0.5], [70.0, 0.02, 3.6], road, 0.0),
        part_r([-18.0, 0.0, -12.0], [27.0, 0.02, 2.6], 0.62, road, 0.0),
        part_r([18.0, 0.0, -12.0], [27.0, 0.02, 2.6], -0.62, road, 0.0),
        part_r([-18.0, 0.0, 13.0], [27.0, 0.02, 2.6], -0.62, road, 0.0),
        part_r([18.0, 0.0, 13.0], [27.0, 0.02, 2.6], 0.62, road, 0.0),
    ];
    w.entities
        .push(prop("ground/roads".into(), [0.0, -0.08, 0.0], roads)?);
    let mut camera = Entity::new("camera");
    camera.set(
        "core.transform",
        Transform {
            position: [-26.0, 40.0, 0.5 + 40.0 / 3f32.sqrt()],
            rotation: [-0.5, 0.0, 0.0, 0.8660254],
            ..Default::default()
        },
    )?;
    camera.set(
        "render.camera",
        Camera {
            projection: Projection::Orthographic { height: 30.0 },
            near: 0.1,
            far: 160.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            active: true,
        },
    )?;
    w.entities.push(camera);
    let metrics = [
        ("minerals", "rts.minerals"),
        ("units", "rts.units"),
        ("workers", "rts.workers"),
        ("soldiers", "rts.soldiers"),
        ("enemy_units", "rts.enemy_units"),
        ("selected", "rts.selected"),
        ("queued", "rts.queued"),
        ("delivered", "rts.delivered"),
        ("produced", "rts.produced"),
        ("wave", "rts.wave"),
        ("base_health", "rts.base_health"),
        ("enemy_base_health", "rts.enemy_base_health"),
        ("victory", "rts.victory"),
        ("defeated", "rts.defeated"),
        ("alert", "rts.alert"),
    ];
    let recipe = |prefab: &str, cost, build_ticks| {
        (
            prefab.to_string(),
            Recipe {
                prefab: prefab.into(),
                cost,
                build_ticks,
            },
        )
    };
    w.resources.insert(
        "rts.session".into(),
        json!(Session {
            ui_bindings: metrics
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            auto_combat: true,
            grid: nav::GridConfig {
                width: 96,
                height: 64,
                cell_size: 1.0,
                origin: [-48.0, -32.0],
                blocked,
            },
            player_faction: 0,
            initial_balance: [150, 150],
            max_units: 200,
            nav_stride: 4,
            path_budget: 6,
            search_budget: 8192,
            camera: "camera".into(),
            recipes: BTreeMap::from([
                recipe("worker", 50, 600),
                recipe("guard", 75, 840),
                recipe("ranger", 90, 960),
            ]),
            opponent: Some(Opponent {
                faction: 1,
                think_ticks: 60,
                worker_recipe: "worker".into(),
                workers: 10,
                army: vec!["guard".into(), "ranger".into(), "guard".into()],
                first_wave_tick: 120 * 150,
                wave_size: 6,
                wave_growth: 2,
                defend_radius: 16.0,
            }),
            team_colors: team_pairs(),
            minimap: true,
        }),
    );
    w.resources.insert(
        "render.settings".into(),
        json!(RenderSettings {
            accent: [0.98, 0.78, 0.35],
            shadow_extent: 72.0
        }),
    );
    w.resources.insert("input.bindings".into(),json!({"axes":{"camera.x":{"positive":"KeyD","negative":"KeyA"},"camera.z":{"positive":"KeyS","negative":"KeyW"},"camera.zoom":{"positive":"KeyE","negative":"KeyQ"}},"buttons":{},"capture_pointer":false}));
    let variables: serde_json::Map<String, Value> = metrics
        .iter()
        .map(|(metric, var)| {
            let v = match *metric {
                "victory" | "defeated" | "alert" => json!(false),
                "minerals" => json!(150),
                "units" | "enemy_units" => json!(10),
                "workers" => json!(6),
                "soldiers" => json!(4),
                "base_health" | "enemy_base_health" => json!(100),
                _ => json!(0),
            };
            (var.to_string(), v)
        })
        .collect();
    w.resources
        .insert("behavior.program".into(), json!({"variables": variables}));
    w.resources.insert("ui.document".into(), ui_document());
    let mut registry = Registry::core();
    register(&mut registry);
    w.compile(&registry)?;
    Ok(w)
}

fn ui_document() -> Value {
    let text =
        |id: &str, key: &str| json!({"kind":"text","id":id,"binding":{"kind":"key","id":key}});
    let value =
        |id: &str, var: &str| json!({"kind":"text","id":id,"binding":{"kind":"variable","id":var}});
    let when_true = |var: &str| json!({"kind":"eq","left":{"kind":"variable","id":var},"right":{"kind":"literal","value":true}});
    let button = |id: &str, action: &str, args: Value| json!({"kind":"button","id":id,"text":id,"command":{"kind":"call","capability":"rts","action":action,"args":args}});
    let mut widgets = vec![];
    let mut regions = serde_json::Map::new();
    let mut styles = serde_json::Map::new();
    // Top resource bar.
    for (key, var) in [
        ("minerals", "rts.minerals"),
        ("workers", "rts.workers"),
        ("soldiers", "rts.soldiers"),
        ("queued", "rts.queued"),
        ("enemy_units", "rts.enemy_units"),
        ("wave", "rts.wave"),
    ] {
        let (l, v) = (format!("label/{key}"), format!("value/{key}"));
        widgets.push(text(&l, key));
        widgets.push(value(&v, var));
        regions.insert(l.clone(), json!("top"));
        regions.insert(v.clone(), json!("top"));
        styles.insert(l, json!("label"));
        styles.insert(v, json!("value"));
    }
    let mut alert = text("alert", "alert");
    alert["when"] = when_true("rts.alert");
    widgets.push(alert);
    regions.insert("alert".into(), json!("top"));
    styles.insert("alert".into(), json!("banner"));
    // Structure status.
    widgets.push(text("brand", "brand"));
    regions.insert("brand".into(), json!("top_left"));
    styles.insert("brand".into(), json!("heading"));
    for (key, var) in [
        ("base_health", "rts.base_health"),
        ("enemy_base_health", "rts.enemy_base_health"),
    ] {
        widgets.push(text(&format!("label/{key}"), key));
        widgets
            .push(json!({"kind":"progress","id":format!("bar/{key}"),"variable":var,"max":100.0}));
        regions.insert(format!("label/{key}"), json!("top_left"));
        regions.insert(format!("bar/{key}"), json!("top_left"));
        styles.insert(format!("label/{key}"), json!("label"));
    }
    // Command card.
    for (id, action, args) in [
        ("select_workers", "select_role", json!({"role":"worker"})),
        ("select_army", "select_role", json!({"role":"soldier"})),
        ("stop", "stop", json!({"ids":[]})),
        ("gather", "gather", json!({"target":"mineral/0/a"})),
        ("attack", "attack", json!({"target":"base/1","engage":true})),
        (
            "train_worker",
            "train",
            json!({"building":"base/0","recipe":"worker","count":1}),
        ),
        (
            "train_guard",
            "train",
            json!({"building":"base/0","recipe":"guard","count":1}),
        ),
        (
            "train_ranger",
            "train",
            json!({"building":"base/0","recipe":"ranger","count":1}),
        ),
    ] {
        widgets.push(button(id, action, args));
        regions.insert(id.into(), json!("bottom"));
    }
    widgets.push(text("hint", "hint"));
    regions.insert("hint".into(), json!("bottom"));
    styles.insert("hint".into(), json!("muted"));
    for id in ["victory", "defeated"] {
        let mut t = text(id, id);
        t["when"] = when_true(&format!("rts.{id}"));
        widgets.push(t);
        regions.insert(id.into(), json!("center"));
        styles.insert(id.into(), json!("title"));
    }
    let zh = json!({
        "brand":"FRONTIER 邊境之戰","minerals":"礦物","workers":"工人","soldiers":"戰鬥單位",
        "queued":"生產中","enemy_units":"敵軍","wave":"來襲波次","base_health":"我方主堡",
        "enemy_base_health":"敵方主堡","alert":"⚠ 敵軍來襲！守住主堡",
        "select_workers":"選取工人","select_army":"選取部隊","stop":"停止","gather":"採集",
        "attack":"進攻敵方主堡","train_worker":"工人 50","train_guard":"衛兵 75",
        "train_ranger":"弓手 90","victory":"勝利！敵方主堡已摧毀","defeated":"戰敗⋯我方主堡已被摧毀",
        "hint":"左鍵框選 · 右鍵移動/採集/攻擊 · WASD 移動鏡頭 · Q/E 縮放 · 點擊小地圖跳轉"
    });
    let en = json!({
        "brand":"FRONTIER","minerals":"Minerals","workers":"Workers","soldiers":"Army",
        "queued":"Training","enemy_units":"Enemies","wave":"Wave","base_health":"Our HQ",
        "enemy_base_health":"Enemy HQ","alert":"⚠ Enemy attack! Defend the HQ",
        "select_workers":"Workers","select_army":"Army","stop":"Stop","gather":"Gather",
        "attack":"Attack enemy HQ","train_worker":"Worker 50","train_guard":"Guard 75",
        "train_ranger":"Ranger 90","victory":"VICTORY! Enemy HQ destroyed","defeated":"DEFEAT... our HQ has fallen",
        "hint":"Drag to select · Right-click move/gather/attack · WASD pan · Q/E zoom · Click minimap to jump"
    });
    json!({
        "session_controls": false,
        "chrome": "compact",
        "locale": "zh-TW",
        "catalogs": {"zh-TW": zh, "en": en},
        "images": {},
        "widgets": widgets,
        "regions": regions,
        "styles": styles,
    })
}
