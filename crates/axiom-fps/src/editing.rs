use crate::{content, from_world, legacy::*, Session};
use anyhow::{ensure, Context, Result};
use axiom_core::{Health, RenderMesh, RenderSettings, World};
use serde_json::{json, Value};
pub const PATHS: &[&str] = &[
    "title",
    "arena.half_size",
    "arena.wall_height",
    "arena.accent",
    "player.spawn",
    "player.health",
    "player.speed",
    "player.sprint_multiplier",
    "player.jump_speed",
    "weapon.damage",
    "weapon.fire_interval",
    "weapon.magazine",
    "weapon.reload_seconds",
    "weapon.range",
    "enemies.count",
    "enemies.health",
    "enemies.speed",
    "enemies.damage",
    "behavior.attack_interval",
];
pub fn handle(w: &mut World, kind: &str, args: &Value) -> Result<bool> {
    if !w.capabilities.contains_key("fps") && kind != "make_arena" {
        return Ok(false);
    }
    match kind {
        "set" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Set {
                path: String,
                value: Value,
            }
            let Set { path, value } = serde_json::from_value(args.clone())?;
            if !PATHS.contains(&path.as_str()) {
                return Ok(false);
            }
            let mut old = from_world(w)?;
            let mut doc = json!(&old);
            let ptr = format!("/{}", path.replace('.', "/"));
            *doc.pointer_mut(&ptr).context("unknown FPS path")? = value.clone();
            let updated: LegacyWorld = serde_json::from_value(doc)?;
            // Validate the final world in Store, allowing dependent edits in one transaction.
            let session: Session = w.resource("fps.session")?;
            let resolved = content::resolve_prefabs(w)?;
            match path.as_str() {
                "enemies.count" => {
                    ensure!(updated.enemies.count <= 128, "enemy count exceeds 128");
                    let current = resolved
                        .entities
                        .iter()
                        .filter(|e| e.components.contains_key("fps.sentinel"))
                        .map(|e| e.id.clone())
                        .collect::<Vec<_>>();
                    if updated.enemies.count < current.len() {
                        let removed = &current[updated.enemies.count..];
                        w.entities.retain(|e| !removed.contains(&e.id));
                    } else {
                        let mut next = 0;
                        for _ in current.len()..updated.enemies.count {
                            while w.entity(&format!("enemy/{next}")).is_some() {
                                next += 1;
                            }
                            w.entities.push(content::sentinel(&updated, next)?);
                            next += 1;
                        }
                    }
                    w.resources
                        .insert("fps.defaults".into(), json!(updated.enemies));
                }
                "enemies.health" | "enemies.speed" | "enemies.damage" => {
                    let field = path.strip_prefix("enemies.").unwrap();
                    for e in &mut w.entities {
                        if resolved
                            .entity(&e.id)
                            .is_some_and(|e| e.components.contains_key("fps.sentinel"))
                        {
                            if field == "health" {
                                e.set(
                                    "core.health",
                                    Health {
                                        value: updated.enemies.health,
                                        max: updated.enemies.health,
                                    },
                                )?;
                            } else {
                                e.components
                                    .entry("fps.sentinel".into())
                                    .or_insert_with(|| json!({}))[field] = value.clone();
                            }
                        }
                    }
                    w.resources
                        .insert("fps.defaults".into(), json!(updated.enemies));
                }
                "player.spawn" => {
                    w.entity_mut(&session.player)
                        .unwrap()
                        .components
                        .entry("core.transform".into())
                        .or_insert_with(|| json!({}))["position"] = value;
                }
                "player.health" => {
                    w.entity_mut(&session.player).unwrap().set(
                        "core.health",
                        Health {
                            value: updated.player.health,
                            max: updated.player.health,
                        },
                    )?;
                }
                p if p.starts_with("player.") => {
                    w.entity_mut(&session.player)
                        .unwrap()
                        .components
                        .entry("fps.controller".into())
                        .or_insert_with(|| json!({}))[p.strip_prefix("player.").unwrap()] = value;
                }
                p if p.starts_with("weapon.") => {
                    w.entity_mut(&session.player)
                        .unwrap()
                        .components
                        .entry("fps.weapon".into())
                        .or_insert_with(|| json!({}))[p.strip_prefix("weapon.").unwrap()] = value;
                }
                "behavior.attack_interval" => {
                    w.resources.get_mut("fps.behavior").unwrap()["attack_interval"] = value;
                }
                p if p.starts_with("arena.") => {
                    w.resources.insert("fps.arena".into(), json!(updated.arena));
                    w.resources.insert(
                        "render.settings".into(),
                        json!(RenderSettings {
                            accent: updated.arena.accent,
                            shadow_extent: updated.arena.half_size * 1.5
                        }),
                    );
                    if let Some(e) = w.entity_mut("scene/arena") {
                        e.set(
                            "render.mesh",
                            RenderMesh {
                                parts: content::arena_parts(&updated),
                            },
                        )?;
                    }
                    for o in &updated.obstacles {
                        if let Some(e) = w.entity_mut(&format!("obstacle/{}", o.id)) {
                            e.set(
                                "render.mesh",
                                RenderMesh {
                                    parts: content::cover_parts(o, updated.arena.accent),
                                },
                            )?;
                        }
                    }
                }
                _ => {
                    old.title = updated.title;
                    w.title = old.title;
                }
            }
            Ok(true)
        }
        "remove_obstacle" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Remove {
                id: String,
            }
            let a: Remove = serde_json::from_value(args.clone())?;
            let before = w.entities.len();
            w.entities.retain(|e| {
                e.components
                    .get("fps.cover")
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_str)
                    != Some(&a.id)
            });
            ensure!(before != w.entities.len(), "obstacle not found");
            Ok(true)
        }
        "put_obstacle" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Put {
                obstacle: Obstacle,
            }
            let a: Put = serde_json::from_value(args.clone())?;
            let arena: Arena = w.resource("fps.arena")?;
            let e = content::cover_entity(&a.obstacle, arena.accent)?;
            if let Some(old) = w.entity_mut(&e.id) {
                *old = e;
            } else {
                w.entities.push(e);
            }
            Ok(true)
        }
        "set_behavior" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Set {
                behavior: Behavior,
            }
            let a: Set = serde_json::from_value(args.clone())?;
            w.resources.insert("fps.behavior".into(), json!(a.behavior));
            Ok(true)
        }
        "make_arena" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Make {
                id: String,
                half_size: f32,
            }
            let a: Make = serde_json::from_value(args.clone())?;
            let mut old = LegacyWorld::demo();
            let scale = a.half_size / old.arena.half_size;
            old.id = a.id;
            old.arena.half_size = a.half_size;
            old.player.spawn[2] *= scale;
            for o in &mut old.obstacles {
                for axis in 0..3 {
                    o.position[axis] *= scale;
                    o.size[axis] *= scale;
                }
            }
            *w = content::convert(&old)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}
