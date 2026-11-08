use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_ENEMIES: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LegacyWorld {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub arena: Arena,
    pub player: Player,
    pub weapon: Weapon,
    pub enemies: EnemyGroup,
    pub behavior: Behavior,
    pub obstacles: Vec<Obstacle>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Arena {
    pub half_size: f32,
    pub wall_height: f32,
    pub accent: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Player {
    pub spawn: [f32; 3],
    pub health: f32,
    pub speed: f32,
    pub sprint_multiplier: f32,
    pub jump_speed: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Weapon {
    pub damage: f32,
    pub fire_interval: f32,
    pub magazine: u32,
    pub reload_seconds: f32,
    pub range: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EnemyGroup {
    pub count: usize,
    pub health: f32,
    pub speed: f32,
    pub damage: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Idle,
    Chase,
    Attack,
    Dead,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Condition {
    DistanceBelow(f32),
    DistanceAbove(f32),
    HealthBelow(f32),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub from: State,
    pub to: State,
    pub when: Condition,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Behavior {
    pub initial: State,
    pub attack_interval: f32,
    pub transitions: Vec<Transition>,
}

impl Behavior {
    /// Bounded VM: at most one ordered transition per actor and tick. Death is invariant.
    pub fn step(&self, current: State, distance: f32, health: f32) -> State {
        if health <= 0.0 {
            return State::Dead;
        }
        for rule in &self.transitions {
            let matches = match rule.when {
                Condition::DistanceBelow(v) => distance < v,
                Condition::DistanceAbove(v) => distance > v,
                Condition::HealthBelow(v) => health < v,
            };
            if rule.from == current && matches {
                return rule.to;
            }
        }
        current
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Obstacle {
    pub id: String,
    pub position: [f32; 3],
    pub size: [f32; 3],
}

#[derive(Clone, Debug, Serialize)]
pub struct EntityDef {
    pub id: String,
    pub prefab: String,
    pub position: [f32; 3],
    pub health: Option<f32>,
}

impl LegacyWorld {
    pub fn demo() -> Self {
        serde_json::from_str(include_str!("../../../assets/worlds/proving-ground.json"))
            .expect("bundled world must be valid JSON")
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported schema_version");
        ensure!(
            !self.id.is_empty() && self.id.len() <= 64,
            "world id must be 1..64 bytes"
        );
        ensure!(self.title.len() <= 128, "title too long");
        finite_range("arena.half_size", self.arena.half_size, 10.0, 100.0)?;
        finite_range("arena.wall_height", self.arena.wall_height, 2.0, 20.0)?;
        for v in self.arena.accent {
            finite_range("accent", v, 0.0, 1.0)?;
        }
        ensure!(
            self.enemies.count <= MAX_ENEMIES,
            "enemy count exceeds {MAX_ENEMIES}"
        );
        finite_range("enemy.health", self.enemies.health, 1.0, 10000.0)?;
        finite_range("enemy.speed", self.enemies.speed, 0.0, 12.0)?;
        finite_range("enemy.damage", self.enemies.damage, 0.0, 1000.0)?;
        finite_range("player.health", self.player.health, 1.0, 10000.0)?;
        finite_range("player.speed", self.player.speed, 0.1, 20.0)?;
        finite_range(
            "player.sprint_multiplier",
            self.player.sprint_multiplier,
            1.0,
            4.0,
        )?;
        finite_range("player.jump_speed", self.player.jump_speed, 1.0, 15.0)?;
        finite_range("weapon.damage", self.weapon.damage, 0.1, 10000.0)?;
        finite_range(
            "weapon.fire_interval",
            self.weapon.fire_interval,
            0.04,
            10.0,
        )?;
        finite_range(
            "weapon.reload_seconds",
            self.weapon.reload_seconds,
            0.1,
            20.0,
        )?;
        finite_range("weapon.range", self.weapon.range, 1.0, 1000.0)?;
        ensure!(
            (1..=500).contains(&self.weapon.magazine),
            "magazine must be 1..500"
        );
        finite_range("attack_interval", self.behavior.attack_interval, 0.1, 30.0)?;
        ensure!(
            self.behavior.initial != State::Dead,
            "initial behavior cannot be dead"
        );
        ensure!(
            self.behavior.transitions.len() <= 64,
            "behavior instruction budget exceeded"
        );
        for rule in &self.behavior.transitions {
            let v = match rule.when {
                Condition::DistanceBelow(v)
                | Condition::DistanceAbove(v)
                | Condition::HealthBelow(v) => v,
            };
            finite_range("behavior threshold", v, 0.0, 10000.0)?;
            ensure!(rule.from != State::Dead, "dead state must be terminal");
        }
        ensure!(self.obstacles.len() <= 256, "obstacle budget exceeded");
        let mut ids = HashSet::new();
        for o in &self.obstacles {
            ensure!(
                !o.id.is_empty() && o.id.len() <= 64 && ids.insert(&o.id),
                "empty, excessive or duplicate obstacle id"
            );
            for v in o.position {
                ensure!(v.is_finite(), "non-finite obstacle position");
            }
            for v in o.size {
                finite_range("obstacle size", v, 0.1, self.arena.half_size)?;
            }
            ensure!(
                o.position[0].abs() + o.size[0] * 0.5 < self.arena.half_size
                    && o.position[2].abs() + o.size[2] * 0.5 < self.arena.half_size,
                "obstacle {} crosses arena wall",
                o.id
            );
            ensure!(
                o.position[1] >= o.size[1] * 0.5 - 0.01,
                "obstacle {} extends below ground",
                o.id
            );
        }
        let p = self.player.spawn;
        ensure!(
            p.iter().all(|v| v.is_finite()) && p[1] >= 0.9 && p[1] <= 20.0,
            "invalid player spawn"
        );
        ensure!(
            p[0].abs() < self.arena.half_size - 0.6 && p[2].abs() < self.arena.half_size - 0.6,
            "player spawn outside arena"
        );
        ensure!(
            !self.overlaps_obstacle(p, 0.4),
            "player spawn intersects obstacle"
        );
        for i in 0..self.enemies.count {
            let p = self.enemy_spawn(i);
            ensure!(
                !self.overlaps_obstacle(p, 0.45),
                "enemy {i} spawn blocked; enlarge arena or move obstacles"
            );
        }
        Ok(())
    }

    pub fn overlaps_obstacle(&self, p: [f32; 3], radius: f32) -> bool {
        self.obstacles.iter().any(|o| {
            (p[0] - o.position[0]).abs() < o.size[0] * 0.5 + radius
                && (p[2] - o.position[2]).abs() < o.size[2] * 0.5 + radius
                && p[1] - 0.85 < o.position[1] + o.size[1] * 0.5
                && p[1] + 0.85 > o.position[1] - o.size[1] * 0.5
        })
    }

    /// Stable slots on nested perimeter rings, independent of group count.
    pub fn enemy_spawn(&self, index: usize) -> [f32; 3] {
        let ring = index / 32;
        let angle = ((index % 32 * 13) % 32) as f32 * std::f32::consts::TAU / 32.0;
        let radius = self.arena.half_size - 2.5 - ring as f32 * 1.2;
        [angle.sin() * radius, 0.9, -angle.cos() * radius]
    }

    pub fn compile(&self) -> Result<Vec<EntityDef>> {
        self.validate()?;
        let mut entities = vec![EntityDef {
            id: "player".into(),
            prefab: "fps.player".into(),
            position: self.player.spawn,
            health: Some(self.player.health),
        }];
        for i in 0..self.enemies.count {
            entities.push(EntityDef {
                id: format!("enemy/{i}"),
                prefab: "fps.sentinel".into(),
                position: self.enemy_spawn(i),
                health: Some(self.enemies.health),
            });
        }
        entities.extend(self.obstacles.iter().map(|o| EntityDef {
            id: format!("obstacle/{}", o.id),
            prefab: "geometry.cover".into(),
            position: o.position,
            health: None,
        }));
        Ok(entities)
    }
}

fn finite_range(name: &str, v: f32, min: f32, max: f32) -> Result<()> {
    if !v.is_finite() || v < min || v > max {
        bail!("{name} must be finite and in {min}..={max}");
    }
    Ok(())
}
