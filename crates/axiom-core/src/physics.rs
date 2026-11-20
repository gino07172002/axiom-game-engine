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
use anyhow::{ensure, Context, Result};
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
pub const JOINT: &str = "physics.joint";
/// `physics.joint` components per world.
pub const MAX_JOINTS: usize = 256;
/// Colliders with `on_enter` / `on_exit` per world.
pub const MAX_EVENT_COLLIDERS: usize = 256;
/// Contact enter + exit events one fixed step may emit (more halts the system with a reason).
pub const MAX_CONTACT_EVENTS_PER_STEP: usize = 64;
pub const MAX_JOINT_EXTENT: f32 = 1000.0;
pub const MAX_JOINT_FORCE: f32 = 1.0e9;
pub const MAX_MOTOR_GAIN: f32 = 1.0e6;

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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JointKind {
    /// Welds the two bodies at their authored relative pose.
    Fixed,
    /// Hinge: rotation about `axis` through `anchor` only.
    Revolute,
    /// Slider: translation along `axis` only.
    Prismatic,
    /// Ball and socket: free rotation about `anchor`.
    Spherical,
}

impl JointKind {
    /// Has a single free axis (`axis`, `limits`, `motor` apply).
    pub fn has_axis(self) -> bool {
        matches!(self, JointKind::Revolute | JointKind::Prismatic)
    }
}

fn default_max_force() -> f32 {
    MAX_JOINT_FORCE
}

/// Drive of a revolute (degrees) or prismatic (meters) joint along its axis: rapier's
/// acceleration-based motor, `stiffness · (target_position − position) + damping ·
/// (target_velocity − velocity)`, capped by `max_force`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JointMotor {
    /// Degrees per second (revolute) or meters per second (prismatic).
    #[serde(default)]
    pub target_velocity: f32,
    /// Degrees (revolute) or meters (prismatic) from the authored pose; needs `stiffness`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_position: Option<f32>,
    #[serde(default)]
    pub stiffness: f32,
    #[serde(default = "one")]
    pub damping: f32,
    #[serde(default = "default_max_force")]
    pub max_force: f32,
}

/// `physics.joint`: connects this entity's `physics.body` to the body of `to` (or to the world).
/// Frames come from the authored poses: the joint holds the bodies' relative placement at
/// `anchor` as authored (angle / offset 0 there).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PhysicsJoint {
    pub kind: JointKind,
    /// Entity ID of the other body; absent = attached to the world.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Joint point in this entity's local space (scaled by its transform scale).
    #[serde(default)]
    pub anchor: [f32; 3],
    /// Revolute / prismatic only: free axis in this entity's local space (default +Y).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<[f32; 3]>,
    /// Revolute / prismatic only: `[min,max]` degrees / meters from the authored pose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motor: Option<JointMotor>,
    /// Let the two connected bodies collide with each other (default: they do not).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collide_connected: bool,
}

impl Default for PhysicsJoint {
    fn default() -> Self {
        Self {
            kind: JointKind::Revolute,
            to: None,
            anchor: [0.0; 3],
            axis: Some([0.0, 1.0, 0.0]),
            limits: None,
            motor: None,
            collide_connected: false,
        }
    }
}

impl PhysicsJoint {
    /// The free axis, unit length (revolute / prismatic; +Y when not authored).
    pub fn unit_axis(&self) -> [f32; 3] {
        let [x, y, z] = self.axis.unwrap_or([0.0, 1.0, 0.0]);
        let n = (x * x + y * y + z * z).sqrt();
        [x / n, y / n, z / n]
    }

    pub fn validate(&self) -> Result<()> {
        let angular = self.kind == JointKind::Revolute;
        ensure!(
            self.anchor
                .iter()
                .all(|v| v.is_finite() && v.abs() <= MAX_JOINT_EXTENT),
            "anchor components must be finite and within ±{MAX_JOINT_EXTENT} m"
        );
        if let Some(to) = &self.to {
            crate::valid_id(to).context("to")?;
        }
        if !self.kind.has_axis() {
            ensure!(
                self.axis.is_none() && self.limits.is_none() && self.motor.is_none(),
                "axis, limits and motor apply to revolute and prismatic joints only"
            );
            return Ok(());
        }
        if let Some(a) = self.axis {
            ensure!(
                a.iter()
                    .all(|v| v.is_finite() && v.abs() <= MAX_JOINT_EXTENT)
                    && a.iter().map(|v| v * v).sum::<f32>() >= 1e-6,
                "axis must be a finite non-zero [x,y,z]"
            );
        }
        let range = if angular { 180.0 } else { MAX_JOINT_EXTENT };
        let unit = if angular { "degrees" } else { "meters" };
        if let Some([min, max]) = self.limits {
            ensure!(
                min.is_finite() && max.is_finite() && -range <= min && min < max && max <= range,
                "limits must be [min,max] {unit} with -{range} <= min < max <= {range}"
            );
        }
        if let Some(m) = &self.motor {
            let speed = if angular { 3600.0 } else { MAX_SPEED };
            ensure!(
                m.target_velocity.is_finite() && m.target_velocity.abs() <= speed,
                "motor target_velocity must be within ±{speed} {unit} per second"
            );
            if let Some(p) = m.target_position {
                ensure!(
                    p.is_finite() && p.abs() <= range,
                    "motor target_position must be within ±{range} {unit}"
                );
                ensure!(
                    m.stiffness > 0.0,
                    "motor target_position needs stiffness > 0"
                );
            }
            ensure!(
                [m.stiffness, m.damping]
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=MAX_MOTOR_GAIN).contains(v)),
                "motor stiffness and damping must be 0..{MAX_MOTOR_GAIN}"
            );
            ensure!(
                m.stiffness > 0.0 || m.damping > 0.0,
                "a motor needs stiffness or damping > 0"
            );
            ensure!(
                m.max_force.is_finite() && m.max_force > 0.0 && m.max_force <= MAX_JOINT_FORCE,
                "motor max_force must be > 0 and <= {MAX_JOINT_FORCE}"
            );
        }
        Ok(())
    }
}

pub fn joint_schema() -> Value {
    json!({
        "kind": "fixed (weld at the authored pose) | revolute (hinge about axis) | prismatic (slide along axis) | spherical (ball joint)",
        "to": "optional entity ID with physics.body (the other body); absent = attached to the world",
        "anchor": format!("vec3 meters in this entity's local space (scaled by its transform scale), default [0,0,0]; within ±{MAX_JOINT_EXTENT}"),
        "axis": "optional vec3: revolute / prismatic free axis in this entity's local space, default [0,1,0]",
        "limits": "optional [min,max] from the authored pose: degrees -180..180 (revolute) or meters (prismatic)",
        "motor": format!("optional {{target_velocity (deg/s | m/s, default 0), target_position? (deg | m, needs stiffness), stiffness 0..{MAX_MOTOR_GAIN} (default 0), damping 0..{MAX_MOTOR_GAIN} (default 1), max_force (default {MAX_JOINT_FORCE})}}: revolute / prismatic only"),
        "collide_connected": "bool, default false: whether the two connected bodies collide with each other",
        "requires": "physics.body on this entity (and on `to`); at least one of the two is dynamic; never the entity itself",
        "budget": format!("{MAX_JOINTS} joints per world"),
        "observe": "physics.inspect {id} reports the joint angle (degrees) or offset (meters); physics.set_motor {id,target_velocity?,target_position?} changes motor targets at runtime (saved in checkpoints)"
    })
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
    validate_events(world, entities)?;
    validate_joints(entities)?;
    let bodies = entities.iter().filter(|e| e.body.is_some()).count();
    if bodies == 0 {
        return Ok(());
    }
    // Colliders that another system moves are simulated too (kinematic followers).
    let simulated = bodies
        + (0..entities.len())
            .filter(|&i| {
                entities[i].body.is_none()
                    && entities[i].collider.is_some()
                    && entities[i].transform.is_some()
                    && crate::navigation::moved_by_system(entities, parents, &registry.movers, i)
            })
            .count();
    if simulated > MAX_BODIES {
        return Err(crate::Diagnostic::error(
            "physics_budget",
            "/entities",
            format!(
                "physics body budget exceeded ({simulated} > {MAX_BODIES} rigid bodies and colliders moved by other systems)"
            ),
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

/// Contact events: `physics.step` and `behavior.execute` run, every event is declared with only
/// the text fields `self` / `other`, the event collider budget, and no sensor on a dynamic body.
fn validate_events(world: &crate::World, entities: &[crate::CompiledEntity]) -> Result<()> {
    let at = |id: &str, field: &str| {
        format!(
            "/entities/@{}/components/physics.collider{field}",
            crate::local::escape(id)
        )
    };
    let has = |system: &str| world.systems.iter().any(|s| s == system);
    let program = world.resources.get("behavior.program");
    let mut eventful = 0;
    for e in entities {
        let Some(c) = &e.collider else { continue };
        if c.sensor && e.body.as_ref().is_some_and(|b| b.kind == BodyKind::Dynamic) {
            return Err(crate::Diagnostic::error(
                "invalid_physics",
                at(&e.id, "/sensor"),
                format!(
                    "{} is a dynamic body with a sensor collider; it would fall through everything (use a kinematic body, a static or a moved collider)",
                    e.id
                ),
            ));
        }
        if !c.has_events() {
            continue;
        }
        eventful += 1;
        for (field, event) in [("/on_enter", &c.on_enter), ("/on_exit", &c.on_exit)] {
            if event.is_empty() {
                continue;
            }
            let invalid = |message: String| {
                crate::Diagnostic::error("invalid_physics", at(&e.id, field), message)
            };
            if !has(SYSTEM) || !has("behavior.execute") {
                return Err(invalid(format!(
                    "contact events need the {SYSTEM} and behavior.execute systems"
                )));
            }
            let fields = program
                .and_then(|p| p.pointer(&format!("/events/{}/fields", crate::local::escape(event))))
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    invalid(format!(
                        "collider emits {event}, which behavior.program does not declare"
                    ))
                })?;
            if !fields
                .iter()
                .all(|(k, t)| (k == "self" || k == "other") && t == "text")
            {
                return Err(invalid(format!(
                    "contact event {event} may only declare text fields `self` and `other`"
                )));
            }
        }
    }
    if eventful > MAX_EVENT_COLLIDERS {
        return Err(crate::Diagnostic::error(
            "physics_budget",
            "/entities",
            format!("contact event collider budget exceeded ({eventful} > {MAX_EVENT_COLLIDERS})"),
        ));
    }
    Ok(())
}

/// The joint of entity `e` (already validated as a component).
pub fn joint_of(e: &crate::CompiledEntity) -> Option<PhysicsJoint> {
    e.components
        .get(JOINT)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Joints: on a body, to an existing other body (never itself), one dynamic side, budget.
fn validate_joints(entities: &[crate::CompiledEntity]) -> Result<()> {
    let joints: Vec<_> = entities
        .iter()
        .filter_map(|e| Some((e, joint_of(e)?)))
        .collect();
    if joints.is_empty() {
        return Ok(());
    }
    if joints.len() > MAX_JOINTS {
        return Err(crate::Diagnostic::error(
            "physics_budget",
            "/entities",
            format!("joint budget exceeded ({} > {MAX_JOINTS})", joints.len()),
        ));
    }
    let index: std::collections::BTreeMap<&str, &crate::CompiledEntity> =
        entities.iter().map(|e| (e.id.as_str(), e)).collect();
    for (e, joint) in joints {
        let invalid = |message: String| {
            crate::Diagnostic::error(
                "invalid_physics",
                format!(
                    "/entities/@{}/components/{JOINT}",
                    crate::local::escape(&e.id)
                ),
                message,
            )
        };
        let Some(body) = &e.body else {
            return Err(invalid(format!(
                "{} has {JOINT} but no {BODY} (a joint connects rigid bodies)",
                e.id
            )));
        };
        let other = match &joint.to {
            None => None,
            Some(to) if *to == e.id => {
                return Err(invalid(format!("{} cannot be jointed to itself", e.id)))
            }
            Some(to) => {
                let Some(target) = index.get(to.as_str()) else {
                    return Err(invalid(format!("joint target {to} does not exist")));
                };
                let Some(b) = &target.body else {
                    return Err(invalid(format!(
                        "joint target {to} has no {BODY} (attach to the world by omitting `to`)"
                    )));
                };
                Some(b.kind)
            }
        };
        if body.kind != BodyKind::Dynamic && other != Some(BodyKind::Dynamic) {
            return Err(invalid(format!(
                "joint on {} connects no dynamic body (at least one side must be dynamic)",
                e.id
            )));
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
                "linear_damping":0.0,"angular_damping":0.05f32,"gravity_scale":1.0,"can_sleep":true,"ccd":false})
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
