pub use crate::frontier::frontier_world;
use crate::*;
/// Fixed benchmark/regression fixture (no opponent AI). The playable map is `frontier_world`.
pub fn demo_world(population: usize) -> Result<World> {
    ensure!(
        [64, 128, 256].contains(&population),
        "supported content populations: 64,128,256"
    );
    let mut w = World::empty("frontier");
    w.title = format!("Frontier / RTS / {population} units");
    w.capabilities.extend([
        ("render".into(), 1),
        ("rts".into(), 1),
        ("behavior".into(), 1),
        ("ui".into(), 1),
    ]);
    w.systems.push("behavior.execute".into());
    w.systems.push("rts.simulate".into());
    let worker = UnitDef {
        faction: 0,
        role: Role::Worker,
        speed: 4.0,
        radius: 0.26,
        carry_capacity: 8,
        gather_ticks: 20,
        damage: 4.0,
        attack_range: 1.5,
        attack_ticks: 60,
    };
    let soldier = UnitDef {
        role: Role::Soldier,
        speed: 4.2,
        carry_capacity: 0,
        damage: 10.0,
        attack_range: 2.5,
        attack_ticks: 45,
        ..worker.clone()
    };
    for (name, def, hp, color) in [
        ("worker", worker, 60.0, [0.15, 0.8, 0.85]),
        ("soldier", soldier, 100.0, [0.1, 0.55, 0.9]),
    ] {
        let mut e = Entity::new(name);
        e.set("core.transform", Transform::default())?;
        e.set("core.health", Health { value: hp, max: hp })?;
        e.set("rts.unit", &def)?;
        e.set(
            "render.mesh",
            RenderMesh {
                parts: vec![
                    MeshPart {
                        transform: Transform {
                            position: [0.0, 0.4, 0.0],
                            scale: [0.5, 0.7, 0.5],
                            ..Default::default()
                        },
                        color,
                        emission: 0.05,
                        grid: false,
                    },
                    MeshPart {
                        transform: Transform {
                            position: [0.0, 0.85, 0.0],
                            scale: [0.28, 0.25, 0.28],
                            ..Default::default()
                        },
                        color: [0.8, 0.9, 0.8],
                        emission: 0.0,
                        grid: false,
                    },
                ],
            },
        )?;
        w.prefabs.insert(
            name.into(),
            Prefab {
                components: e.components,
            },
        );
    }
    let per_side = population / 2;
    let columns = if population == 256 { 16 } else { 8 };
    for faction in 0..2 {
        for i in 0..per_side {
            let mut e = Entity::new(format!("unit/{faction}/{i}"));
            let recipe = if i < per_side / 2 {
                "worker"
            } else {
                "soldier"
            };
            e.prefab = Some(recipe.into());
            let base = if faction == 0 {
                if population == 256 {
                    -35.5
                } else {
                    -23.5
                }
            } else {
                if population == 256 {
                    18.5
                } else {
                    20.5
                }
            };
            let z = if faction == 0 {
                if population == 256 {
                    -14.5
                } else {
                    -7.5
                }
            } else {
                if population == 256 {
                    6.5
                } else {
                    4.5
                }
            };
            e.set(
                "core.transform",
                Transform {
                    position: [
                        base + (i % columns) as f32 * 1.2,
                        0.0,
                        z + (i / columns) as f32 * 1.2,
                    ],
                    ..Default::default()
                },
            )?;
            e.set("rts.unit", json!({"faction":faction}))?;
            if faction == 1 {
                let mut mesh: RenderMesh =
                    serde_json::from_value(w.prefabs[recipe].components["render.mesh"].clone())?;
                mesh.parts[0].color = [0.9, 0.26, 0.18];
                e.set("render.mesh", mesh)?;
            }
            e.tags = vec![format!("faction/{faction}"), recipe.into()];
            w.entities.push(e);
        }
    }
    let mut blocked = vec![];
    for z in 20..=44 {
        if (29..=35).contains(&z) {
            continue;
        }
        blocked.push([48, z]);
        let mut wall = Entity::new(format!("wall/{z}"));
        wall.set(
            "core.transform",
            Transform {
                position: [0.5, 0.7, -32.0 + z as f32 + 0.5],
                scale: [0.9, 1.4, 0.9],
                ..Default::default()
            },
        )?;
        wall.set(
            "render.mesh",
            RenderMesh {
                parts: vec![MeshPart {
                    color: [0.25, 0.3, 0.34],
                    transform: Transform::default(),
                    emission: 0.0,
                    grid: false,
                }],
            },
        )?;
        w.entities.push(wall);
    }
    for faction in 0..2 {
        let x = if faction == 0 { -29.5 } else { 29.5 };
        let mut base = Entity::new(format!("base/{faction}"));
        base.set(
            "core.transform",
            Transform {
                position: [x, 0.0, 0.5],
                ..Default::default()
            },
        )?;
        base.set(
            "core.health",
            Health {
                value: 600.0,
                max: 600.0,
            },
        )?;
        base.set(
            "rts.building",
            BuildingDef {
                faction,
                depot: true,
                produces: vec!["worker".into(), "soldier".into()],
                attack: None,
            },
        )?;
        base.set(
            "render.mesh",
            RenderMesh {
                parts: vec![
                    MeshPart {
                        transform: Transform {
                            position: [0.0, 0.7, 0.0],
                            scale: [2.8, 1.4, 2.8],
                            ..Default::default()
                        },
                        color: if faction == 0 {
                            [0.08, 0.45, 0.6]
                        } else {
                            [0.65, 0.18, 0.12]
                        },
                        emission: 0.1,
                        grid: false,
                    },
                    MeshPart {
                        transform: Transform {
                            position: [0.0, 1.6, 0.0],
                            scale: [1.0, 0.5, 1.0],
                            ..Default::default()
                        },
                        color: [0.8, 0.75, 0.4],
                        emission: 0.3,
                        grid: false,
                    },
                ],
            },
        )?;
        w.entities.push(base);
        let cx = (x + 48.0) as i32;
        for dz in -1..=1 {
            for dx in -1..=1 {
                blocked.push([cx + dx, 32 + dz]);
            }
        }
        let mut resource = Entity::new(format!("mineral/{faction}"));
        resource.set(
            "core.transform",
            Transform {
                position: [
                    if faction == 0 { -26.5 } else { 26.5 },
                    0.0,
                    if faction == 0 { -10.5 } else { 10.5 },
                ],
                ..Default::default()
            },
        )?;
        resource.set("rts.resource", ResourceDef { amount: 5000 })?;
        resource.set(
            "render.mesh",
            RenderMesh {
                parts: vec![MeshPart {
                    transform: Transform {
                        position: [0.0, 0.7, 0.0],
                        scale: [1.1, 1.4, 1.1],
                        rotation: [0.0, 0.2, 0.0, 0.9797959],
                    },
                    color: [0.25, 0.95, 0.7],
                    emission: 0.4,
                    grid: false,
                }],
            },
        )?;
        w.entities.push(resource);
    }
    let mut ground = Entity::new("ground");
    ground.set(
        "core.transform",
        Transform {
            position: [0.0, -0.35, 0.0],
            scale: [96.0, 0.5, 64.0],
            ..Default::default()
        },
    )?;
    ground.set(
        "render.mesh",
        RenderMesh {
            parts: vec![MeshPart {
                color: [0.13, 0.22, 0.2],
                grid: true,
                transform: Transform::default(),
                emission: 0.0,
            }],
        },
    )?;
    w.entities.push(ground);
    let mut camera = Entity::new("camera");
    camera.set(
        "core.transform",
        Transform {
            position: [0.0, 40.0, 22.0],
            rotation: [-0.5, 0.0, 0.0, 0.8660254],
            ..Default::default()
        },
    )?;
    camera.set(
        "render.camera",
        Camera {
            projection: Projection::Orthographic { height: 48.0 },
            near: 0.1,
            far: 150.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            active: true,
        },
    )?;
    w.entities.push(camera);
    w.resources.insert(
        "rts.session".into(),
        json!(Session {
            ui_bindings: [
                ("minerals", "rts.minerals"),
                ("units", "rts.units"),
                ("selected", "rts.selected"),
                ("queued", "rts.queued"),
                ("delivered", "rts.delivered"),
                ("produced", "rts.produced"),
                ("victory", "rts.victory"),
                ("defeated", "rts.defeated")
            ]
            .into_iter()
            .map(|(a, b)| (a.into(), b.into()))
            .collect(),
            auto_combat: true,
            grid: nav::GridConfig {
                width: 96,
                height: 64,
                cell_size: 1.0,
                origin: [-48.0, -32.0],
                blocked
            },
            player_faction: 0,
            initial_balance: [300, 300],
            max_units: 512,
            nav_stride: 4,
            path_budget: 4,
            search_budget: 8192,
            camera: "camera".into(),
            recipes: BTreeMap::from([
                (
                    "worker".into(),
                    Recipe {
                        prefab: "worker".into(),
                        cost: 40,
                        build_ticks: 120
                    }
                ),
                (
                    "soldier".into(),
                    Recipe {
                        prefab: "soldier".into(),
                        cost: 60,
                        build_ticks: 180
                    }
                )
            ]),
            opponent: None,
            team_colors: vec![],
            minimap: false,
        }),
    );
    w.resources.insert(
        "render.settings".into(),
        json!(RenderSettings {
            accent: [0.2, 0.85, 0.7],
            shadow_extent: 64.0
        }),
    );
    w.resources.insert("input.bindings".into(),json!({"axes":{"camera.x":{"positive":"KeyD","negative":"KeyA"},"camera.z":{"positive":"KeyS","negative":"KeyW"},"camera.zoom":{"positive":"KeyE","negative":"KeyQ"}},"buttons":{},"capture_pointer":false}));
    w.resources.insert("behavior.program".into(),json!({"variables":{"rts.minerals":300,"rts.units":population/2,"rts.selected":0,"rts.queued":0,"rts.delivered":0,"rts.produced":0,"rts.victory":false,"rts.defeated":false}}));
    let mut widgets =
        vec![json!({"kind":"text","id":"brand","binding":{"kind":"key","id":"brand"}})];
    for (id, key) in [
        ("minerals", "minerals"),
        ("units", "units"),
        ("selected", "selected"),
        ("queued", "queued"),
        ("delivered", "delivered"),
        ("produced", "produced"),
    ] {
        widgets.push(
            json!({"kind":"text","id":format!("label/{id}"),"binding":{"kind":"key","id":key}}),
        );
        widgets.push(json!({"kind":"text","id":format!("value/{id}"),"binding":{"kind":"variable","id":format!("rts.{id}")}}));
    }
    for (id, key, action, args) in [
        (
            "workers",
            "workers",
            "select_role",
            json!({"role":"worker"}),
        ),
        (
            "soldiers",
            "soldiers",
            "select_role",
            json!({"role":"soldier"}),
        ),
        ("stop", "stop", "stop", json!({"ids":[]})),
        (
            "train-worker",
            "train_worker",
            "train",
            json!({"building":"base/0","recipe":"worker","count":1}),
        ),
        (
            "train-soldier",
            "train_soldier",
            "train",
            json!({"building":"base/0","recipe":"soldier","count":1}),
        ),
        ("gather", "gather", "gather", json!({"target":"mineral/0"})),
        ("attack", "attack", "attack", json!({"target":"base/1"})),
    ] {
        widgets.push(json!({"kind":"button","id":id,"text":key,"command":{"kind":"call","capability":"rts","action":action,"args":args}}));
    }
    for id in ["victory", "defeated"] {
        widgets.push(json!({"kind":"text","id":id,"binding":{"kind":"key","id":id},"when":{"kind":"eq","left":{"kind":"variable","id":format!("rts.{id}")},"right":{"kind":"literal","value":true}}}));
    }
    widgets.push(json!({"kind":"text","id":"hint","binding":{"kind":"key","id":"hint"}}));
    w.resources.insert("ui.document".into(),json!({"flow":true,"session_controls":false,"locale":"zh-TW","catalogs":{"zh-TW":{"brand":"FRONTIER","minerals":"礦物","units":"單位","selected":"已選","queued":"佇列","delivered":"已交付","produced":"已生產","workers":"選擇工人","soldiers":"選擇士兵","stop":"停止","train_worker":"生產工人 [40]","train_soldier":"生產士兵 [60]","gather":"採集礦物","attack":"攻擊敵方基地","victory":"VICTORY / 敵方基地已摧毀","defeated":"DEFEAT / 我方基地已摧毀","hint":"拖曳框選 / Shift 加選 / 右鍵移動、採集、攻擊 / WASD 平移 / Q E 縮放"}},"images":{},"widgets":widgets}));
    let mut registry = Registry::core();
    register(&mut registry);
    w.compile(&registry)?;
    Ok(w)
}
