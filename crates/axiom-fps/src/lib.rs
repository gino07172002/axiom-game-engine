//! FPS capability schemas, v1 migration, and content recipes. The generic core has no FPS dependency.
mod content;
mod editing;
pub mod legacy;
use anyhow::{ensure, Result};
use axiom_core::*;
pub use content::convert as from_legacy;
pub use content::{
    actor_definitions, demo_world, from_world, Controller, ModelPart, Sentinel, Session, Visuals,
};
pub use legacy::{
    Arena, Behavior, Condition, EnemyGroup, EntityDef, LegacyWorld, Obstacle, Player, State,
    Transition, Weapon,
};
use serde_json::{json, Value};

pub fn registry() -> Registry {
    let mut r = Registry::core();
    register(&mut r);
    r
}
pub fn register(r: &mut Registry) {
    r.capabilities.insert("fps".into(), 1);
    r.component(
        "fps.controller",
        "fps",
        json!({"speed":"0.1..20","sprint_multiplier":"1..4","jump_speed":"1..15"}),
        |v| {
            let c: Controller = serde_json::from_value(v.clone())?;
            ensure!(
                (0.1..=20.0).contains(&c.speed)
                    && (1.0..=4.0).contains(&c.sprint_multiplier)
                    && (1.0..=15.0).contains(&c.jump_speed),
                "invalid FPS controller"
            );
            Ok(())
        },
    );
    r.component("fps.weapon","fps",json!({"damage":"0.1..10000","fire_interval":"0.04..10","magazine":"integer 1..500","reload_seconds":"0.1..20","range":"1..1000"}),|v|{let weapon:Weapon=serde_json::from_value(v.clone())?;let mut fixture=LegacyWorld::demo();fixture.weapon=weapon;fixture.validate()});
    r.component(
        "fps.sentinel",
        "fps",
        json!({"speed":"0..12","damage":"0..1000"}),
        |v| {
            let c: Sentinel = serde_json::from_value(v.clone())?;
            ensure!(
                (0.0..=12.0).contains(&c.speed) && (0.0..=1000.0).contains(&c.damage),
                "invalid sentinel"
            );
            Ok(())
        },
    );
    r.component("fps.cover", "fps", json!({"id":"string"}), |v| {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Cover {
            id: String,
        }
        let c: Cover = serde_json::from_value(v.clone())?;
        valid_id(&c.id)
    });
    r.resource(
        "fps.session",
        "fps",
        json!({"player":"entity reference","camera":"entity reference"}),
        |v| {
            let s: Session = serde_json::from_value(v.clone())?;
            valid_id(&s.player)?;
            valid_id(&s.camera)
        },
    );
    r.resource(
        "fps.arena",
        "fps",
        json!({"half_size":"10..100","wall_height":"2..20","accent":"rgb[3]"}),
        |v| {
            let a: Arena = serde_json::from_value(v.clone())?;
            ensure!(
                (10.0..=100.0).contains(&a.half_size)
                    && (2.0..=20.0).contains(&a.wall_height)
                    && a.accent.iter().all(|v| (0.0..=1.0).contains(v)),
                "invalid arena settings"
            );
            Ok(())
        },
    );
    r.resource(
        "fps.defaults",
        "fps",
        json!({"count":"0..128","health":"1..10000","speed":"0..12","damage":"0..1000"}),
        |v| {
            let e: EnemyGroup = serde_json::from_value(v.clone())?;
            ensure!(
                e.count <= 128
                    && (1.0..=10000.0).contains(&e.health)
                    && (0.0..=12.0).contains(&e.speed)
                    && (0.0..=1000.0).contains(&e.damage),
                "invalid enemy defaults"
            );
            Ok(())
        },
    );
    r.resource(
        "fps.behavior",
        "fps",
        json!(LegacyWorld::demo().behavior),
        |v| {
            let behavior: Behavior = serde_json::from_value(v.clone())?;
            let mut fixture = LegacyWorld::demo();
            fixture.behavior = behavior;
            fixture.validate()
        },
    );
    r.resource(
        "fps.visuals",
        "fps",
        json!({"sentinel":"model parts","rifle":"model parts","corpse":"model parts"}),
        |v| {
            let models: Visuals = serde_json::from_value(v.clone())?;
            for parts in [&models.sentinel, &models.rifle, &models.corpse] {
                ensure!(parts.len() <= 64, "model part budget exceeded");
                for p in parts {
                    RenderMesh {
                        parts: vec![p.part.clone()],
                    }
                    .validate()?;
                    ensure!(p.name.len() <= 64, "model part name too long");
                }
            }
            Ok(())
        },
    );
    r.systems.insert(
        "fps.simulate".into(),
        SystemDescriptor {
            id: "fps.simulate".into(),
            capability: "fps".into(),
            phase: 20,
            reads: vec![
                "physics.collider".into(),
                "fps.controller".into(),
                "fps.weapon".into(),
                "fps.sentinel".into(),
            ],
            writes: vec!["core.transform".into(), "core.health".into()],
            after: vec![],
        },
    );
    r.migrations.insert(1, |v| {
        let old: LegacyWorld = serde_json::from_value(v)?;
        old.validate()?;
        content::convert(&old)
    });
    r.world_validators.push(|w| {
        if !w.capabilities.contains_key("fps") {
            return Ok(());
        }
        let resolved = content::resolve_prefabs(w)?;
        let w = &resolved;
        ensure!(
            w.systems.iter().any(|s| s == "fps.simulate"),
            "FPS capability requires fps.simulate"
        );
        let old = from_world(w)?;
        let mut structural = old.clone();
        structural.enemies.count = 0;
        structural.validate()?;
        ensure!(old.enemies.count <= 128, "enemy count exceeds 128");
        let session: Session = w.resource("fps.session")?;
        let camera = w
            .entity(&session.camera)
            .ok_or_else(|| anyhow::anyhow!("missing FPS camera"))?;
        ensure!(
            camera.components.contains_key("render.camera"),
            "FPS camera requires render.camera"
        );
        let view: Camera = serde_json::from_value(camera.components["render.camera"].clone())?;
        ensure!(
            view.active
                && view.viewport == [0.0, 0.0, 1.0, 1.0]
                && matches!(view.projection, Projection::Perspective { .. }),
            "FPS HUD requires an active fullscreen perspective primary camera"
        );
        let models: Visuals = w.resource("fps.visuals")?;
        ensure!(
            !models.rifle.is_empty() && !models.sentinel.is_empty(),
            "FPS visual models required"
        );
        let players = w
            .entities
            .iter()
            .filter(|e| e.components.contains_key("fps.controller"))
            .count();
        ensure!(players == 1, "FPS pack requires exactly one controller");
        let instances = w
            .entities
            .iter()
            .filter_map(|e| e.components.get("render.mesh"))
            .map(|v| serde_json::from_value::<RenderMesh>(v.clone()).map(|m| m.parts.len()))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .iter()
            .sum::<usize>();
        ensure!(
            instances + old.enemies.count * models.sentinel.len() + models.rifle.len() + 64
                <= MAX_INSTANCES,
            "FPS presentation exceeds instance budget"
        );
        for e in &w.entities {
            let controlled = e.components.contains_key("fps.controller")
                || e.components.contains_key("fps.sentinel");
            if controlled {
                let t: Transform = serde_json::from_value(
                    e.components
                        .get("core.transform")
                        .ok_or_else(|| anyhow::anyhow!("controlled actor needs transform"))?
                        .clone(),
                )?;
                ensure!(
                    t.scale == [1.0; 3] && !e.components.contains_key("core.motion"),
                    "FPS actors use unit scale and FPS locomotion, not core.motion"
                );
                let c: Collider = serde_json::from_value(
                    e.components
                        .get("physics.collider")
                        .ok_or_else(|| anyhow::anyhow!("FPS actor requires physics.collider"))?
                        .clone(),
                )?;
                let expected = if e.components.contains_key("fps.controller") {
                    ColliderShape::Capsule {
                        half_height: 0.55,
                        radius: 0.35,
                    }
                } else {
                    ColliderShape::Capsule {
                        half_height: 0.5,
                        radius: 0.4,
                    }
                };
                ensure!(
                    c.shape == expected,
                    "FPS v1 controller capsule dimensions are fixed"
                );
                let memberships = w
                    .scenes
                    .iter()
                    .filter(|s| s.entities.contains(&e.id))
                    .collect::<Vec<_>>();
                ensure!(
                    memberships.is_empty() || memberships.iter().any(|s| s.active),
                    "FPS controlled actors must belong to an active scene"
                );
            }
            if e.components.contains_key("fps.sentinel") {
                let t: Transform = serde_json::from_value(
                    e.components
                        .get("core.transform")
                        .ok_or_else(|| anyhow::anyhow!("sentinel needs transform"))?
                        .clone(),
                )?;
                ensure!(
                    t.position[0].abs() < old.arena.half_size - 0.5
                        && t.position[2].abs() < old.arena.half_size - 0.5
                        && !old.overlaps_obstacle(t.position, 0.45),
                    "sentinel spawn blocked or outside arena: {}",
                    e.id
                );
                let h: Health = serde_json::from_value(
                    e.components
                        .get("core.health")
                        .ok_or_else(|| anyhow::anyhow!("sentinel needs health"))?
                        .clone(),
                )?;
                h.validate()?;
            }
        }
        Ok(())
    });
    r.edit_handlers.push(editing::handle);
    r.summaries.push(|w|if w.capabilities.contains_key("fps"){let w=content::resolve_prefabs(w).unwrap();json!({"enemies":w.entities.iter().filter(|e|e.components.contains_key("fps.sentinel")).count(),"obstacles":w.entities.iter().filter(|e|e.components.contains_key("fps.cover")).count()})}else{json!({})});
    r.topics.insert("fps".into(),json!({"capability":"fps","version":1,"legacy_paths":editing::PATHS,"content":"content/fps","systems":["fps.simulate"],"note":"legacy set paths remain supported through FPS adapter; generic edits use v2 components"}));
    r.topics.insert("behavior".into(),json!({"states":["idle","chase","attack","dead"],"conditions":["distance_below","distance_above","health_below"],"example":LegacyWorld::demo().behavior}));
}
pub fn open_store(path: impl AsRef<std::path::Path>) -> Result<Store> {
    Store::open_with(path, registry(), demo_world()?)
}
pub fn summary(world: &World) -> Value {
    json!({"id":world.id,"schema_version":world.schema_version})
}
