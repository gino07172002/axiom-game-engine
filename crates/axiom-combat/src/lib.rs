//! Deterministic XZ collision and damage shared by independent gameplay controllers.
use anyhow::{ensure, Result};
use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub min: [f32; 2],
    pub max: [f32; 2],
}
impl Rect {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.min.iter().chain(&self.max).all(|v| v.is_finite())
                && (0..2).all(|i| self.min[i] < self.max[i]),
            "invalid combat rectangle"
        );
        Ok(())
    }
    pub fn clear(&self, p: Vec2, radius: f32) -> bool {
        let q = p.clamp(Vec2::from_array(self.min), Vec2::from_array(self.max));
        p.distance_squared(q) + 1e-6 >= radius * radius
    }
    pub fn intersects_segment(&self, a: Vec2, b: Vec2) -> bool {
        let d = b - a;
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for axis in 0..2 {
            if d[axis].abs() < 1e-8 {
                if a[axis] < self.min[axis] || a[axis] > self.max[axis] {
                    return false;
                }
            } else {
                let p = (self.min[axis] - a[axis]) / d[axis];
                let q = (self.max[axis] - a[axis]) / d[axis];
                lo = lo.max(p.min(q));
                hi = hi.min(p.max(q));
            }
        }
        lo <= hi
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub bounds: Rect,
    #[serde(default)]
    pub obstacles: Vec<Rect>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        self.bounds.validate()?;
        ensure!(
            self.obstacles.len() <= 2048,
            "combat obstacle budget exceeded"
        );
        for o in &self.obstacles {
            o.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Handle {
    pub id: String,
    pub generation: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Body {
    pub handle: Handle,
    pub owner: String,
    pub position: Vec2,
    pub radius: f32,
    pub faction: u8,
    pub health: f32,
    pub max_health: f32,
    pub invulnerable: bool,
}
impl Body {
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub source: Handle,
    pub target: Handle,
    pub owner: String,
    pub damage: f32,
    pub range: f32,
    /// Full circle when None; otherwise a normalized direction and cosine of half arc.
    pub arc: Option<(Vec2, f32)>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Hit,
    Evaded,
    Rejected,
}
#[derive(Clone, Debug, Serialize)]
pub struct Receipt {
    pub source: String,
    pub target: String,
    pub owner: String,
    pub amount: f32,
    pub killed: bool,
    pub outcome: Outcome,
}
#[derive(Clone, Debug)]
pub struct Scene {
    pub config: Config,
    pub bodies: BTreeMap<String, Body>,
}
impl Scene {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            bodies: BTreeMap::new(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        ensure!(self.bodies.len() <= 4096, "combat body budget exceeded");
        for b in self.bodies.values() {
            ensure!(
                b.position.is_finite()
                    && b.radius.is_finite()
                    && b.radius > 0.0
                    && b.health.is_finite()
                    && b.max_health.is_finite()
                    && b.max_health > 0.0
                    && (0.0..=b.max_health).contains(&b.health),
                "invalid combat body: {}",
                b.handle.id
            );
            ensure!(
                !b.alive() || self.clear(&b.handle.id, b.position, b.radius),
                "combat body overlaps blocker or body: {}",
                b.handle.id
            );
        }
        Ok(())
    }
    pub fn clear(&self, id: &str, p: Vec2, radius: f32) -> bool {
        let bounds = &self.config.bounds;
        p.is_finite()
            && radius.is_finite()
            && radius > 0.0
            && (0..2).all(|i| p[i] >= bounds.min[i] + radius && p[i] <= bounds.max[i] - radius)
            && self.config.obstacles.iter().all(|o| o.clear(p, radius))
            && self.bodies.values().all(|b| {
                b.handle.id == id
                    || !b.alive()
                    || p.distance_squared(b.position) + 1e-6 >= (radius + b.radius).powi(2)
            })
    }
    pub fn line_clear(&self, a: Vec2, b: Vec2) -> bool {
        self.config
            .obstacles
            .iter()
            .all(|o| !o.intersects_segment(a, b))
    }
    pub fn set_position(&mut self, id: &str, p: Vec2) {
        if let Some(b) = self.bodies.get_mut(id) {
            b.position = p;
        }
    }
    pub fn attempt(
        &self,
        source: &str,
        target: &str,
        damage: f32,
        range: f32,
        arc: Option<(Vec2, f32)>,
    ) -> Option<Hit> {
        let a = self.bodies.get(source)?;
        let b = self.bodies.get(target)?;
        Some(Hit {
            source: a.handle.clone(),
            target: b.handle.clone(),
            owner: a.owner.clone(),
            damage,
            range,
            arc,
        })
    }
    pub fn in_range(&self, hit: &Hit) -> bool {
        let Some(a) = self.bodies.get(&hit.source.id) else {
            return false;
        };
        let Some(b) = self.bodies.get(&hit.target.id) else {
            return false;
        };
        let d = b.position - a.position;
        a.alive()
            && b.alive()
            && a.faction != b.faction
            && a.handle == hit.source
            && b.handle == hit.target
            && a.owner == hit.owner
            && hit.damage.is_finite()
            && (0.0..=1000.0).contains(&hit.damage)
            && hit.range.is_finite()
            && (0.0..=100.0).contains(&hit.range)
            && d.length() <= hit.range + b.radius
            && hit.arc.is_none_or(|(f, c)| {
                f.is_finite()
                    && c.is_finite()
                    && (-1.0..=1.0).contains(&c)
                    && (f.length_squared() - 1.0).abs() < 0.001
                    && (d.length_squared() < 1e-6 || f.dot(d.normalize()) + 1e-6 >= c)
            })
            && self.line_clear(a.position, b.position)
    }
    /// All attempts see the same alive/geometry snapshot, allowing simultaneous trades.
    /// Health transitions are applied once, in deterministic submission order.
    pub fn resolve(&mut self, hits: Vec<Hit>) -> Vec<Receipt> {
        let eligible = hits.iter().map(|h| self.in_range(h)).collect::<Vec<_>>();
        hits.into_iter()
            .zip(eligible)
            .map(|(hit, valid)| {
                let mut receipt = Receipt {
                    source: hit.source.id,
                    target: hit.target.id,
                    owner: hit.owner,
                    amount: 0.0,
                    killed: false,
                    outcome: Outcome::Rejected,
                };
                if valid {
                    let target = self.bodies.get_mut(&receipt.target).unwrap();
                    if target.invulnerable {
                        receipt.outcome = Outcome::Evaded;
                    } else {
                        let before = target.health;
                        target.health = (before - hit.damage).max(0.0);
                        receipt.amount = before - target.health;
                        receipt.killed = before > 0.0 && target.health == 0.0;
                        receipt.outcome = Outcome::Hit;
                    }
                }
                receipt
            })
            .collect()
    }
}
