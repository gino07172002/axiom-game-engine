//! `physics.body` / `physics.settings`: authoring side of the 3D rigid-body system
//! `physics.step` (runtime: `axiom_runtime::physics`).
//!
//! Shapes and contact materials stay on `physics.collider` (box / capsule / sphere,
//! `friction`, `restitution`). In a world that schedules `physics.step`:
//! - an entity with `physics.collider` and `physics.body` is a simulated body (dynamic: gravity,
//!   contacts, sleeping; kinematic: moves by its own velocity, pushes dynamic bodies);
//! - a collider on an entity some other system moves (any `movers` component, `path.follow`,
//!   `nav.agent`, also through an ancestor) follows that entity as a kinematic pusher, so
//!   character controllers and moving platforms push dynamic bodies (one way: the controller
//!   keeps its own collision);
//! - every other collider is static scenery at its authored pose.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const BODY: &str = "physics.body";
pub const SETTINGS: &str = "physics.settings";
pub const SYSTEM: &str = "physics.step";
/// Simulated bodies per world: dynamic + kinematic bodies + kinematic followers.
pub const MAX_BODIES: usize = 1024;
pub const MAX_FRICTION: f32 = 10.0;
pub const MAX_SOLVER_ITERATIONS: u32 = 16;
pub const MAX_MASS: f32 = 100_000.0;
pub const MAX_SPEED: f32 = 1000.0;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    /// Gravity, contacts, friction, restitution and sleeping.
    #[default]
    Dynamic,
    /// Moves by `velocity` / `angular_velocity` only; pushes dynamic bodies, never pushed.
    Kinematic,
}

fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}
fn default_angular_damping() -> f32 {
    0.05
}

/// `physics.body`. Mass is the body's total mass (the collider shape gives the inertia).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RigidBody {
    #[serde(default)]
    pub kind: BodyKind,
    #[serde(default = "one")]
    pub mass: f32,
    /// Initial linear velocity (dynamic) or constant velocity (kinematic), m/s.
    #[serde(default)]
    pub velocity: [f32; 3],
    /// Initial / constant angular velocity, radians per second.
    #[serde(default)]
    pub angular_velocity: [f32; 3],
    #[serde(default)]
    pub linear_damping: f32,
    #[serde(default = "default_angular_damping")]
    pub angular_damping: f32,
    #[serde(default = "one")]
    pub gravity_scale: f32,
    #[serde(default = "yes")]
    pub can_sleep: bool,
    /// Continuous collision detection for small fast bodies (costs more per step).
    #[serde(default)]
    pub ccd: bool,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            kind: BodyKind::Dynamic,
            mass: 1.0,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            linear_damping: 0.0,
            angular_damping: default_angular_damping(),
            gravity_scale: 1.0,
            can_sleep: true,
            ccd: false,
        }
    }
}

impl RigidBody {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.mass.is_finite() && (0.001..=MAX_MASS).contains(&self.mass),
            "mass must be 0.001..{MAX_MASS} kg"
        );
        ensure!(
            self.velocity
                .iter()
                .chain(&self.angular_velocity)
                .all(|v| v.is_finite() && v.abs() <= MAX_SPEED),
            "velocity and angular_velocity components must be finite and within ±{MAX_SPEED}"
        );
        ensure!(
            [self.linear_damping, self.angular_damping]
                .iter()
                .all(|d| d.is_finite() && (0.0..=100.0).contains(d)),
            "linear_damping and angular_damping must be 0..100"
        );
        ensure!(
            self.gravity_scale.is_finite() && (-10.0..=10.0).contains(&self.gravity_scale),
            "gravity_scale must be -10..10"
        );
        Ok(())
    }
}

/// `physics.settings` (optional resource).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PhysicsSettings {
    #[serde(default = "default_gravity")]
    pub gravity: [f32; 3],
    #[serde(default = "default_iterations")]
    pub solver_iterations: u32,
}
fn default_gravity() -> [f32; 3] {
    [0.0, -9.81, 0.0]
}
fn default_iterations() -> u32 {
    4
}
impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            gravity: default_gravity(),
            solver_iterations: default_iterations(),
        }
    }
}
impl PhysicsSettings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.gravity
                .iter()
                .all(|g| g.is_finite() && g.abs() <= 100.0),
            "gravity components must be within ±100 m/s²"
        );
        ensure!(
            (1..=MAX_SOLVER_ITERATIONS).contains(&self.solver_iterations),
            "solver_iterations must be 1..{MAX_SOLVER_ITERATIONS}"
        );
        Ok(())
    }
}

pub fn body_schema() -> Value {
    json!({
        "kind": "dynamic (default; gravity, contacts, sleeping) | kinematic (moves by its velocity, pushes dynamic bodies)",
        "mass": format!("kg 0.001..{MAX_MASS}, default 1 (dynamic only; inertia from the collider shape)"),
        "velocity": "vec3 m/s: initial velocity (dynamic) or constant velocity (kinematic)",
        "angular_velocity": "vec3 radians/second: initial (dynamic) or constant (kinematic)",
        "linear_damping": "number 0..100, default 0",
        "angular_damping": "number 0..100, default 0.05",
        "gravity_scale": "number -10..10, default 1",
        "can_sleep": "bool, default true: rests after ~2 s below the sleep thresholds; contacts and physics actions wake it",
        "ccd": "bool, default false: continuous collision detection for small fast bodies",
        "requires": "physics.collider on the same entity (the body's shape and contact material), core.transform, a root entity (no parent) and the physics.step system",
        "conflict": "an entity with physics.body must not have another mover (core.motion, path.follow, nav.agent, fsm.agent, rts.unit, action.actor, fps.*): authoring error physics_conflict",
        "budget": format!("{MAX_BODIES} simulated bodies per world (dynamic + kinematic bodies + colliders moved by other systems)"),
        "actions": "physics.impulse {id,impulse:[x,y,z]}, physics.set_velocity {id,velocity?,angular_velocity?}, physics.inspect {id}, physics.summary",
        "checkpoint": "supported: velocities and sleep state are saved with the transforms"
    })
}

pub fn settings_schema() -> Value {
    json!({
        "gravity": "vec3 m/s², default [0,-9.81,0]",
        "solver_iterations": format!("integer 1..{MAX_SOLVER_ITERATIONS}, default 4: contact solver iterations per 1/120 s step (more = stiffer stacks, slower)"),
    })
}

/// World-level checks run by `World::compile`: the system, the collider, a root entity, no
/// second mover, no colliders under a body, and the body budget.
pub fn validate_world(
    world: &crate::World,
    registry: &crate::Registry,
    entities: &[crate::CompiledEntity],
    parents: &[Option<usize>],
) -> Result<()> {
    let bodies = entities.iter().filter(|e| e.body.is_some()).count();
    if bodies == 0 {
        return Ok(());
    }
    if bodies > MAX_BODIES {
        return Err(crate::Diagnostic::error(
            "physics_budget",
            "/entities",
            format!("physics body budget exceeded ({bodies} > {MAX_BODIES})"),
        ));
    }
    let at = |id: &str| format!("/entities/@{}/components/{BODY}", crate::local::escape(id));
    let invalid =
        |id: &str, message: String| crate::Diagnostic::error("invalid_physics", at(id), message);
    let scheduled = world.systems.iter().any(|s| s == SYSTEM);
    for (slot, e) in entities.iter().enumerate() {
        if e.body.is_none() {
            continue;
        }
        if !scheduled {
            return Err(invalid(
                &e.id,
                format!("{BODY} needs the {SYSTEM} system (add it to systems)"),
            ));
        }
        if e.collider.is_none() {
            return Err(invalid(
                &e.id,
                format!(
                    "{} has {BODY} but no physics.collider (the body's shape)",
                    e.id
                ),
            ));
        }
        if let Some(parent) = parents[slot] {
            return Err(invalid(
                &e.id,
                format!(
                    "{} has {BODY} but is a child of {}; rigid bodies must be root entities",
                    e.id, entities[parent].id
                ),
            ));
        }
        let other = e
            .components
            .keys()
            .filter(|c| c.as_str() != BODY)
            .find_map(|c| registry.movers.get_key_value(c))
            .map(|(c, s)| (c.clone(), s.clone()))
            .or_else(|| {
                e.follow
                    .as_ref()
                    .map(|_| ("path.follow".to_owned(), "path.move".to_owned()))
            });
        if let Some((component, system)) = other {
            return Err(crate::Diagnostic::error(
                "physics_conflict",
                at(&e.id),
                format!(
                    "{} has {BODY} and {component} (moved by {system}); an entity takes one mover",
                    e.id
                ),
            ));
        }
    }
    for (slot, e) in entities.iter().enumerate() {
        if e.collider.is_none() {
            continue;
        }
        let mut cursor = parents[slot];
        while let Some(p) = cursor {
            if entities[p].body.is_some() {
                return Err(crate::Diagnostic::error(
                    "invalid_physics",
                    format!(
                        "/entities/@{}/components/physics.collider",
                        crate::local::escape(&e.id)
                    ),
                    format!(
                        "{} has physics.collider under rigid body {}; compound bodies are not supported (one shape per body, on the body entity)",
                        e.id, entities[p].id
                    ),
                ));
            }
            cursor = parents[p];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_and_ranges() {
        let b: RigidBody = serde_json::from_value(json!({})).unwrap();
        assert_eq!(b, RigidBody::default());
        b.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&b).unwrap(),
            json!({"kind":"dynamic","mass":1.0,"velocity":[0.0,0.0,0.0],"angular_velocity":[0.0,0.0,0.0],
                "linear_damping":0.0,"angular_damping":0.05,"gravity_scale":1.0,"can_sleep":true,"ccd":false})
        );
        let bad = |v: Value| {
            serde_json::from_value::<RigidBody>(v)
                .map_err(anyhow::Error::from)
                .and_then(|b| b.validate())
                .is_err()
        };
        assert!(bad(json!({"mass":0})));
        assert!(bad(json!({"mass":1e6})));
        assert!(bad(json!({"velocity":[0,2000,0]})));
        assert!(bad(json!({"gravity_scale":11})));
        assert!(bad(json!({"linear_damping":-1})));
        assert!(bad(json!({"kind":"static"})));
        assert!(bad(json!({"bounce":1})));
        let s: PhysicsSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(s, PhysicsSettings::default());
        assert!(PhysicsSettings {
            solver_iterations: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}
