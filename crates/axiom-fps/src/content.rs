use crate::legacy::*;
use anyhow::{ensure, Context, Result};
use axiom_core::{
    AxisBinding, Camera, Collider, ColliderShape, Entity, Health, InputBindings, MeshPart,
    RenderSettings, Transform, World,
};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
#[path = "geometry.rs"]
mod geometry;
pub(crate) use geometry::{arena_parts, cover_parts};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub player: String,
    pub camera: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controller {
    pub speed: f32,
    pub sprint_multiplier: f32,
    pub jump_speed: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sentinel {
    pub speed: f32,
    pub damage: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPart {
    pub name: String,
    pub part: MeshPart,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visuals {
    pub sentinel: Vec<ModelPart>,
    pub rifle: Vec<ModelPart>,
    pub corpse: Vec<ModelPart>,
}
pub fn demo_world() -> Result<World> {
    convert(&LegacyWorld::demo())
}
pub fn convert(old: &LegacyWorld) -> Result<World> {
    old.validate()?;
    let mut w = World::empty(&old.id);
    w.title = old.title.clone();
    for id in ["render", "physics", "fps"] {
        w.capabilities.insert(id.into(), 1);
    }
    w.systems.push("fps.simulate".into());
    w.resources.insert("fps.arena".into(), json!(old.arena));
    w.resources
        .insert("fps.defaults".into(), json!(old.enemies));
    w.resources
        .insert("fps.behavior".into(), json!(old.behavior));
    w.resources.insert(
        "fps.session".into(),
        json!(Session {
            player: "player".into(),
            camera: "camera/fps".into()
        }),
    );
    w.resources.insert(
        "fps.visuals".into(),
        serde_json::from_str(include_str!("../../../content/fps/models.json"))?,
    );
    w.resources.insert(
        "render.settings".into(),
        json!(RenderSettings {
            accent: old.arena.accent,
            shadow_extent: old.arena.half_size * 1.5
        }),
    );
    w.resources.insert(
        "input.bindings".into(),
        json!(InputBindings {
            axes: BTreeMap::from([
                (
                    "move.forward".into(),
                    AxisBinding {
                        positive: "KeyW".into(),
                        negative: "KeyS".into()
                    }
                ),
                (
                    "move.right".into(),
                    AxisBinding {
                        positive: "KeyD".into(),
                        negative: "KeyA".into()
                    }
                )
            ]),
            buttons: BTreeMap::from([
                ("sprint".into(), "ShiftLeft".into()),
                ("jump".into(), "Space".into()),
                ("fire".into(), "MouseLeft".into()),
                ("reload".into(), "KeyR".into())
            ]),
            capture_pointer: true
        }),
    );
    let mut player = Entity::new("player");
    player.tags.push("fps.owned".into());
    player.set(
        "core.transform",
        Transform {
            position: old.player.spawn,
            ..Default::default()
        },
    )?;
    player.set(
        "core.health",
        Health {
            value: old.player.health,
            max: old.player.health,
        },
    )?;
    player.set(
        "physics.collider",
        Collider::new(ColliderShape::Capsule {
            half_height: 0.55,
            radius: 0.35,
        }),
    )?;
    player.set(
        "fps.controller",
        Controller {
            speed: old.player.speed,
            sprint_multiplier: old.player.sprint_multiplier,
            jump_speed: old.player.jump_speed,
        },
    )?;
    player.set("fps.weapon", &old.weapon)?;
    w.entities.push(player);
    for i in 0..old.enemies.count {
        w.entities.push(sentinel(old, i)?);
    }
    for obstacle in &old.obstacles {
        w.entities.push(cover_entity(obstacle, old.arena.accent)?);
    }
    let mut arena = Entity::new("scene/arena");
    arena.tags.push("fps.owned".into());
    arena.set("core.transform", Transform::default())?;
    arena.set(
        "render.mesh",
        axiom_core::RenderMesh {
            parts: arena_parts(old),
        },
    )?;
    w.entities.push(arena);
    let mut camera = Entity::new("camera/fps");
    camera.tags.push("fps.owned".into());
    camera.set(
        "core.transform",
        Transform {
            position: (Vec3::from_array(old.player.spawn) + Vec3::Y * 0.65).to_array(),
            rotation: Quat::from_rotation_x(-0.035).to_array(),
            ..Default::default()
        },
    )?;
    camera.set("render.camera", Camera::perspective())?;
    w.entities.push(camera);
    Ok(w)
}
pub(crate) fn sentinel(old: &LegacyWorld, i: usize) -> Result<Entity> {
    let mut e = Entity::new(format!("enemy/{i}"));
    e.tags.push("fps.owned".into());
    e.set(
        "core.transform",
        Transform {
            position: old.enemy_spawn(i),
            ..Default::default()
        },
    )?;
    e.set(
        "core.health",
        Health {
            value: old.enemies.health,
            max: old.enemies.health,
        },
    )?;
    e.set(
        "physics.collider",
        Collider::new(ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.4,
        }),
    )?;
    e.set(
        "fps.sentinel",
        Sentinel {
            speed: old.enemies.speed,
            damage: old.enemies.damage,
        },
    )?;
    Ok(e)
}
pub(crate) fn cover_entity(o: &Obstacle, accent: [f32; 3]) -> Result<Entity> {
    let mut e = Entity::new(format!("obstacle/{}", o.id));
    e.tags.push("fps.owned".into());
    e.set(
        "core.transform",
        Transform {
            position: o.position,
            ..Default::default()
        },
    )?;
    e.set(
        "physics.collider",
        Collider::new(ColliderShape::Box {
            half_extents: o.size.map(|v| v * 0.5),
        }),
    )?;
    e.set("fps.cover", json!({"id":o.id}))?;
    e.set(
        "render.mesh",
        axiom_core::RenderMesh {
            parts: cover_parts(o, accent),
        },
    )?;
    Ok(e)
}
pub fn from_world(w: &World) -> Result<LegacyWorld> {
    let resolved = resolve_prefabs(w)?;
    let w = &resolved;
    let session: Session = w.resource("fps.session")?;
    let p = w.entity(&session.player).context("missing FPS player")?;
    let t: Transform = serde_json::from_value(
        p.components
            .get("core.transform")
            .context("FPS player needs transform")?
            .clone(),
    )?;
    let h: Health = serde_json::from_value(
        p.components
            .get("core.health")
            .context("FPS player needs health")?
            .clone(),
    )?;
    let c: Controller = serde_json::from_value(
        p.components
            .get("fps.controller")
            .context("FPS player needs controller")?
            .clone(),
    )?;
    let mut enemies: EnemyGroup = w.resource("fps.defaults")?;
    enemies.count = w
        .entities
        .iter()
        .filter(|e| e.components.contains_key("fps.sentinel"))
        .count();
    let mut obstacles = vec![];
    for e in &w.entities {
        // Rigid bodies move (physics.step); the FPS arena only collides with static shapes.
        if e.components.contains_key("fps.sentinel")
            || e.id == session.player
            || e.components.contains_key(axiom_core::physics::BODY)
        {
            continue;
        }
        if let Some(v) = e.components.get("physics.collider") {
            let collider: Collider = serde_json::from_value(v.clone())?;
            let t: Transform = serde_json::from_value(
                e.components
                    .get("core.transform")
                    .context("collider needs transform")?
                    .clone(),
            )?;
            ensure!(
                t.rotation == [0.0, 0.0, 0.0, 1.0],
                "FPS box collisions currently require axis aligned transforms"
            );
            if let ColliderShape::Box { half_extents } = collider.shape {
                obstacles.push(Obstacle {
                    id: e
                        .components
                        .get("fps.cover")
                        .and_then(|v| v.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(&e.id)
                        .to_owned(),
                    position: t.position,
                    size: [
                        half_extents[0] * 2.0 * t.scale[0],
                        half_extents[1] * 2.0 * t.scale[1],
                        half_extents[2] * 2.0 * t.scale[2],
                    ],
                });
            }
        }
    }
    Ok(LegacyWorld {
        schema_version: 1,
        id: w.id.clone(),
        title: w.title.clone(),
        arena: w.resource("fps.arena")?,
        player: Player {
            spawn: t.position,
            health: h.max,
            speed: c.speed,
            sprint_multiplier: c.sprint_multiplier,
            jump_speed: c.jump_speed,
        },
        weapon: serde_json::from_value(
            p.components
                .get("fps.weapon")
                .context("missing weapon")?
                .clone(),
        )?,
        enemies,
        behavior: w.resource("fps.behavior")?,
        obstacles,
    })
}

pub(crate) fn resolve_prefabs(world: &World) -> Result<World> {
    let mut w = world.clone();
    for e in &mut w.entities {
        if let Some(id) = &e.prefab {
            let mut components = world
                .prefabs
                .get(id)
                .with_context(|| format!("missing prefab {id}"))?
                .components
                .clone();
            for (k, v) in &e.components {
                if let Some(base) = components.get_mut(k) {
                    axiom_core::merge_json(base, v);
                } else {
                    components.insert(k.clone(), v.clone());
                }
            }
            e.components = components;
        }
    }
    Ok(w)
}
pub fn actor_definitions(w: &axiom_core::CompiledWorld) -> Result<(Vec<EntityDef>, Vec<Sentinel>)> {
    let session: Session = w.source.resource("fps.session")?;
    let p = &w.entities[*w.indices.get(&session.player).context("player missing")?];
    let mut defs = vec![EntityDef {
        id: p.id.clone(),
        prefab: "fps.player".into(),
        position: p
            .transform
            .as_ref()
            .context("missing player transform")?
            .position,
        health: Some(p.health.as_ref().context("missing player health")?.max),
    }];
    let mut settings = vec![];
    for e in &w.entities {
        if let Some(v) = e.components.get("fps.sentinel") {
            defs.push(EntityDef {
                id: e.id.clone(),
                prefab: "fps.sentinel".into(),
                position: e
                    .transform
                    .as_ref()
                    .context("missing sentinel transform")?
                    .position,
                health: Some(e.health.as_ref().context("missing sentinel health")?.max),
            });
            settings.push(serde_json::from_value(v.clone())?);
        }
    }
    Ok((defs, settings))
}
