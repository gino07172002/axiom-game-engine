//! `physics.step`: fixed-step 3D rigid bodies on rapier3d (see `axiom_core::physics` and
//! docs/physics.zh-TW.md; the choice of rapier3d is the ADR in docs/plan-engine-next.zh-TW.md).
//!
//! Built once per runtime from the compiled world, in entity-slot order (insertion order is
//! what makes rapier deterministic for one binary): every `physics.collider` becomes a static
//! collider (authored world pose, inactive scenes disabled), a kinematic follower (the entity
//! is moved by another system: the body is driven to the entity's world pose each step and
//! pushes dynamic bodies) or the shape of a `physics.body` (dynamic / kinematic). Each fixed
//! step drives the followers, steps the pipeline once with `FIXED_DT` and writes the awake
//! bodies' poses into `core.transform`. A steady step does not allocate in this module; the
//! tables are typed and bounded by `MAX_BODIES`.
use crate::trigger::world_matrix;
use crate::world::*;
use anyhow::{ensure, Context, Result};
use axiom_core::gameplay::{RuntimeCommand, Scalar};
use axiom_core::physics::*;
use axiom_core::{Collider, ColliderShape, Diagnostic};
use glam::{Quat, Vec3};
use rapier3d::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{any::Any, collections::BTreeMap};

/// Bodies farther than this from the origin (any axis) are disabled and reported, so a body
/// falling forever cannot push `core.transform` out of its valid range.
const WORLD_LIMIT: f32 = 10_000.0;
/// A follower moving farther (meters) or turning more (radians) than this in one step is
/// placed at its new pose instead of being driven there (60 m/s, about 1400°/s).
const TELEPORT_DISTANCE: f32 = 0.5;
const TELEPORT_ANGLE: f32 = 0.2;
/// Bodies listed by entity ID in `physics.summary` (counts are always complete).
const LISTED: usize = 16;
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Static,
    /// Collider of an entity moved by another system; position-based kinematic body.
    Follower,
    Dynamic,
    Kinematic,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Role::Static => "static",
            Role::Follower => "follower",
            Role::Dynamic => "dynamic",
            Role::Kinematic => "kinematic",
        }
    }
    /// Has a body whose pose this system writes into `core.transform`.
    fn simulated(self) -> bool {
        matches!(self, Role::Dynamic | Role::Kinematic)
    }
}

struct Entry {
    slot: usize,
    /// Entity ID (observe lists; the table is built once, so this is not a per-step cost).
    id: String,
    role: Role,
    body: Option<RigidBodyHandle>,
    collider: ColliderHandle,
    /// Scene activation last applied to rapier.
    enabled: bool,
    /// Left `WORLD_LIMIT`: disabled for the rest of the run.
    out: bool,
}

/// Typed per-body state a checkpoint carries (transforms are saved by the checkpoint itself).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SavedBody {
    velocity: [f32; 3],
    angular_velocity: [f32; 3],
    sleeping: bool,
    /// Seconds spent below the sleep thresholds (rapier activation clock).
    still: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    out: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SavedPhysics {
    steps: u64,
    bodies: BTreeMap<String, SavedBody>,
    /// Motor targets of the motorized joints by joint entity ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    motors: BTreeMap<String, SavedMotor>,
    /// Per contact-event collider: the colliders it touches, by entity ID (so loading does not
    /// re-fire `on_enter` for contacts that already existed).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    touching: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "is_zero")]
    events: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SavedMotor {
    target_velocity: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_position: Option<f32>,
}

/// A behavior event a collider emits and which of the payload fields it declares.
struct EventInfo {
    name: String,
    self_field: bool,
    other_field: bool,
}

/// A collider with `on_enter` / `on_exit`.
struct Watch {
    entry: u32,
    on_enter: Option<u16>,
    on_exit: Option<u16>,
    /// Entries touching it (contact or sensor overlap), sorted by entity ID rank.
    touching: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Change {
    watch: u32,
    other: u32,
    enter: bool,
}

struct Joint {
    slot: usize,
    handle: ImpulseJointHandle,
    def: PhysicsJoint,
    /// Current motor targets in authored units (degrees / meters); `physics.set_motor` changes them.
    motor: Option<SavedMotor>,
}

pub struct PhysicsSystem {
    gravity: Vector<Real>,
    params: IntegrationParameters,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    /// Every physics collider in slot order.
    entries: Vec<Entry>,
    /// Entity slot -> index in `entries` (`NONE` without a collider).
    by_slot: Vec<u32>,
    steps: u64,
    /// Entity slots scanned for runtime spawns.
    scanned: usize,
    /// Runtime-spawned entities with a collider (not simulated; reported).
    ignored_spawns: u32,
    halted: Option<String>,
    /// Fixed body the world-attached joints connect to (created only when one exists).
    world_body: Option<RigidBodyHandle>,
    joints: Vec<Joint>,
    events: Vec<EventInfo>,
    /// Contact-event colliders in entity ID order.
    watches: Vec<Watch>,
    /// Entry index -> rank of its entity ID among all entries (deterministic event order).
    rank: Vec<u32>,
    scratch: Vec<u32>,
    changes: Vec<Change>,
    emitted: u64,
    /// Contact events stopped (budget) or the first emit failure.
    events_halted: Option<String>,
    event_fault: Option<String>,
}

fn isometry(rotation: Quat, translation: Vec3) -> Isometry<Real> {
    Isometry::from_parts(
        Translation::new(translation.x, translation.y, translation.z),
        Rotation::from_quaternion(nalgebra::Quaternion::new(
            rotation.w, rotation.x, rotation.y, rotation.z,
        )),
    )
}

fn shape(collider: &Collider, scale: Vec3) -> ColliderBuilder {
    let scale = scale.abs();
    let mut builder = match collider.shape {
        ColliderShape::Box { half_extents } => {
            let h = Vec3::from_array(half_extents) * scale;
            ColliderBuilder::cuboid(h.x, h.y, h.z)
        }
        ColliderShape::Capsule {
            half_height,
            radius,
        } => ColliderBuilder::capsule_y(half_height * scale.y, radius * scale.max_element()),
        ColliderShape::Sphere { radius } => ColliderBuilder::ball(radius * scale.max_element()),
    };
    if let Some(friction) = collider.friction {
        builder = builder.friction(friction);
    }
    if let Some(restitution) = collider.restitution {
        builder = builder.restitution(restitution);
    }
    if collider.sensor {
        // Kinematic movers count too (rapier's default only pairs colliders when one side is
        // dynamic); static-static pairs never change, so they are left out (a sensor zone
        // resting on the floor does not report the floor). Sensors produce no contact forces.
        builder = builder.sensor(true).active_collision_types(
            ActiveCollisionTypes::default()
                | ActiveCollisionTypes::KINEMATIC_FIXED
                | ActiveCollisionTypes::KINEMATIC_KINEMATIC,
        );
    }
    // The bouncier surface wins (a lively ball bounces on a default floor); friction keeps
    // the average of the two.
    builder.restitution_combine_rule(CoefficientCombineRule::Max)
}

fn vec3(v: &Vector<Real>) -> [f32; 3] {
    [v.x, v.y, v.z]
}

/// Authored joint units (degrees / meters) to rapier's (radians / meters).
fn joint_unit(kind: JointKind) -> f32 {
    if kind == JointKind::Revolute {
        std::f32::consts::PI / 180.0
    } else {
        1.0
    }
}

fn free_axis(kind: JointKind) -> Option<JointAxis> {
    match kind {
        JointKind::Revolute => Some(JointAxis::AngX),
        JointKind::Prismatic => Some(JointAxis::LinX),
        JointKind::Fixed | JointKind::Spherical => None,
    }
}

/// Display rounding for observe (f64 so the JSON shows the short decimal).
fn round(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

impl PhysicsSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let compiled = &state.compiled;
        let settings: PhysicsSettings = if compiled.source.resources.contains_key(SETTINGS) {
            compiled.source.resource(SETTINGS)?
        } else {
            PhysicsSettings::default()
        };
        settings.validate()?;
        let params = IntegrationParameters {
            dt: FIXED_DT,
            num_solver_iterations: std::num::NonZeroUsize::new(settings.solver_iterations as usize)
                .context("solver_iterations")?,
            ..Default::default()
        };
        let g = settings.gravity;
        let mut system = Self {
            gravity: vector![g[0], g[1], g[2]],
            params,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            entries: vec![],
            by_slot: vec![NONE; compiled.len()],
            steps: 0,
            scanned: compiled.len(),
            ignored_spawns: 0,
            halted: None,
            world_body: None,
            joints: vec![],
            events: vec![],
            watches: vec![],
            rank: vec![],
            scratch: vec![],
            changes: Vec::with_capacity(MAX_CONTACT_EVENTS_PER_STEP),
            emitted: 0,
            events_halted: None,
            event_fault: None,
        };
        let matrices = state.matrices();
        let mut moving = 0;
        for (slot, e) in compiled.entities.iter().enumerate() {
            let Some(collider) = &e.collider else {
                continue;
            };
            let Some(transform) = &state.transforms[slot] else {
                continue;
            };
            let enabled = compiled.enabled[slot];
            let (role, body) = if let Some(def) = &e.body {
                def.validate()
                    .with_context(|| format!("{BODY} on {}", e.id))?;
                ensure!(
                    compiled.parents[slot].is_none(),
                    "{BODY} on {} must be a root entity",
                    e.id
                );
                let position = isometry(
                    Quat::from_array(transform.rotation),
                    Vec3::from_array(transform.position),
                );
                let [vx, vy, vz] = def.velocity;
                let [wx, wy, wz] = def.angular_velocity;
                let builder = match def.kind {
                    BodyKind::Dynamic => RigidBodyBuilder::dynamic()
                        .linear_damping(def.linear_damping)
                        .angular_damping(def.angular_damping)
                        .gravity_scale(def.gravity_scale)
                        .can_sleep(def.can_sleep)
                        .ccd_enabled(def.ccd),
                    BodyKind::Kinematic => RigidBodyBuilder::kinematic_velocity_based(),
                };
                let body = builder
                    .position(position)
                    .linvel(vector![vx, vy, vz])
                    .angvel(vector![wx, wy, wz])
                    .enabled(enabled)
                    .build();
                let role = match def.kind {
                    BodyKind::Dynamic => Role::Dynamic,
                    BodyKind::Kinematic => Role::Kinematic,
                };
                (
                    role,
                    Some((body, Vec3::from_array(transform.scale), def.mass)),
                )
            } else if axiom_core::navigation::moved_by_system(
                &compiled.entities,
                &compiled.parents,
                &state.registry.movers,
                slot,
            ) {
                let (scale, rotation, translation) = matrices[slot].to_scale_rotation_translation();
                let body = RigidBodyBuilder::kinematic_position_based()
                    .position(isometry(rotation, translation))
                    .enabled(enabled)
                    .build();
                (Role::Follower, Some((body, scale, 1.0)))
            } else {
                (Role::Static, None)
            };
            // Collider -> entry index (contact events map rapier pairs back to entities).
            let index = u128::from(system.entries.len() as u32);
            let (body, collider) = match body {
                Some((body, scale, mass)) => {
                    moving += 1;
                    ensure!(
                        moving <= MAX_BODIES,
                        "physics body budget exceeded (> {MAX_BODIES} dynamic, kinematic and moved colliders)"
                    );
                    let handle = system.bodies.insert(body);
                    // The body's enabled flag switches its collider (rapier never re-enables a
                    // collider that was itself built disabled when its body is enabled).
                    let mut builder = shape(collider, scale).user_data(index);
                    if role == Role::Dynamic {
                        builder = builder.mass(mass);
                    }
                    let collider =
                        system
                            .colliders
                            .insert_with_parent(builder, handle, &mut system.bodies);
                    (Some(handle), collider)
                }
                None => {
                    let (scale, rotation, translation) =
                        matrices[slot].to_scale_rotation_translation();
                    let builder = shape(collider, scale)
                        .user_data(index)
                        .position(isometry(rotation, translation))
                        .enabled(enabled);
                    (None, system.colliders.insert(builder))
                }
            };
            system.by_slot[slot] = system.entries.len() as u32;
            system.entries.push(Entry {
                slot,
                id: e.id.clone(),
                role,
                body,
                collider,
                enabled,
                out: false,
            });
        }
        system.build_events(state)?;
        system.build_joints(state)?;
        Ok(system)
    }

    /// Ranks entries by entity ID and lists the colliders with contact events in that order.
    fn build_events(&mut self, state: &RuntimeState) -> Result<()> {
        let program = state.compiled.source.resources.get("behavior.program");
        let mut order: Vec<u32> = (0..self.entries.len() as u32).collect();
        order.sort_by(|&a, &b| {
            self.entries[a as usize]
                .id
                .cmp(&self.entries[b as usize].id)
        });
        self.rank = vec![0; self.entries.len()];
        for (rank, &i) in order.iter().enumerate() {
            self.rank[i as usize] = rank as u32;
        }
        for &i in &order {
            let entry = &self.entries[i as usize];
            let collider = state.compiled.entities[entry.slot]
                .collider
                .as_ref()
                .expect("entries have a collider");
            if !collider.has_events() {
                continue;
            }
            let mut event = |name: &str| -> Result<Option<u16>> {
                if name.is_empty() {
                    return Ok(None);
                }
                if let Some(i) = self.events.iter().position(|e| e.name == name) {
                    return Ok(Some(i as u16));
                }
                let fields = program
                    .and_then(|p| {
                        p.pointer(&format!(
                            "/events/{}/fields",
                            axiom_core::local::escape(name)
                        ))
                    })
                    .and_then(Value::as_object)
                    .with_context(|| {
                        format!("collider emits {name}, which behavior.program does not declare")
                    })?;
                self.events.push(EventInfo {
                    name: name.to_owned(),
                    self_field: fields.contains_key("self"),
                    other_field: fields.contains_key("other"),
                });
                Ok(Some(self.events.len() as u16 - 1))
            };
            let on_enter = event(&collider.on_enter)?;
            let on_exit = event(&collider.on_exit)?;
            self.watches.push(Watch {
                entry: i,
                on_enter,
                on_exit,
                touching: vec![],
            });
        }
        ensure!(
            self.watches.len() <= MAX_EVENT_COLLIDERS,
            "contact event collider budget exceeded ({} > {MAX_EVENT_COLLIDERS})",
            self.watches.len()
        );
        if !self.watches.is_empty() {
            ensure!(
                state.gameplay.is_some()
                    || state
                        .compiled
                        .source
                        .systems
                        .iter()
                        .any(|s| s == "behavior.execute"),
                "contact events need the behavior.execute system"
            );
        }
        Ok(())
    }

    /// One rapier impulse joint per `physics.joint`, in entity-slot order. Frames come from the
    /// authored poses, so the joint holds the authored relative placement (angle / offset 0).
    fn build_joints(&mut self, state: &RuntimeState) -> Result<()> {
        let compiled = &state.compiled;
        for (slot, e) in compiled.entities.iter().enumerate() {
            let Some(def) = joint_of(e) else { continue };
            def.validate()
                .with_context(|| format!("{JOINT} on {}", e.id))?;
            let pose = |slot: usize| -> Result<(Isometry<Real>, Vec3, RigidBodyHandle)> {
                let entry = self
                    .by_slot
                    .get(slot)
                    .filter(|&&i| i != NONE)
                    .map(|&i| &self.entries[i as usize])
                    .filter(|entry| entry.role.simulated())
                    .with_context(|| {
                        format!(
                            "{JOINT} on {}: {} is not a simulated rigid body",
                            e.id, compiled.entities[slot].id
                        )
                    })?;
                let t = compiled.entities[slot]
                    .transform
                    .as_ref()
                    .context("rigid bodies have core.transform")?;
                Ok((
                    isometry(Quat::from_array(t.rotation), Vec3::from_array(t.position)),
                    Vec3::from_array(t.scale),
                    entry.body.expect("simulated entries have a body"),
                ))
            };
            let (pose1, scale1, body1) = pose(slot)?;
            let (pose2, body2) = match &def.to {
                Some(to) => {
                    let &other = compiled
                        .indices
                        .get(to)
                        .with_context(|| format!("{JOINT} on {}: missing body {to}", e.id))?;
                    let (pose2, _, body2) = pose(other)?;
                    (pose2, body2)
                }
                None => {
                    let world = *self
                        .world_body
                        .get_or_insert_with(|| self.bodies.insert(RigidBodyBuilder::fixed()));
                    (Isometry::identity(), world)
                }
            };
            let anchor = Vec3::from_array(def.anchor) * scale1;
            let axis = Vec3::from_array(def.unit_axis());
            // rapier measures body 2 relative to body 1: the jointed entity is body 2, so angles,
            // offsets, limits and motor targets describe its own motion along `axis`.
            let own = isometry(Quat::from_rotation_arc(Vec3::X, axis), anchor);
            let other = pose2.inverse() * pose1 * own;
            let locked = match def.kind {
                JointKind::Fixed => JointAxesMask::LOCKED_FIXED_AXES,
                JointKind::Revolute => JointAxesMask::LOCKED_REVOLUTE_AXES,
                JointKind::Prismatic => JointAxesMask::LOCKED_PRISMATIC_AXES,
                JointKind::Spherical => JointAxesMask::LOCKED_SPHERICAL_AXES,
            };
            let (free, unit) = (free_axis(def.kind), joint_unit(def.kind));
            let mut builder = GenericJointBuilder::new(locked)
                .local_frame1(other)
                .local_frame2(own)
                .contacts_enabled(def.collide_connected);
            if let Some(axis) = free {
                if let Some([min, max]) = def.limits {
                    builder = builder.limits(axis, [min * unit, max * unit]);
                }
                if let Some(m) = &def.motor {
                    builder = builder
                        .set_motor(
                            axis,
                            m.target_position.unwrap_or(0.0) * unit,
                            m.target_velocity * unit,
                            m.stiffness,
                            m.damping,
                        )
                        .motor_max_force(axis, m.max_force);
                }
            }
            let handle = self
                .impulse_joints
                .insert(body2, body1, builder.build(), true);
            let motor = def.motor.as_ref().map(|m| SavedMotor {
                target_velocity: m.target_velocity,
                target_position: m.target_position,
            });
            self.joints.push(Joint {
                slot,
                handle,
                def,
                motor,
            });
        }
        ensure!(
            self.joints.len() <= MAX_JOINTS,
            "joint budget exceeded ({} > {MAX_JOINTS})",
            self.joints.len()
        );
        Ok(())
    }

    /// Applies `target` to the motor of joint `j` (authored units) and wakes its bodies.
    fn set_motor(&mut self, j: usize, target: SavedMotor) {
        let joint = &mut self.joints[j];
        let (Some(m), Some(axis)) = (&joint.def.motor, free_axis(joint.def.kind)) else {
            return;
        };
        let unit = joint_unit(joint.def.kind);
        if let Some(data) = self.impulse_joints.get_mut(joint.handle, true) {
            data.data.set_motor(
                axis,
                target.target_position.unwrap_or(0.0) * unit,
                target.target_velocity * unit,
                m.stiffness,
                m.damping,
            );
        }
        joint.motor = Some(target);
    }

    /// Angle (degrees, revolute) or offset along the axis (meters, prismatic) of joint `j`.
    fn joint_position(&self, j: usize) -> Option<f32> {
        let joint = &self.joints[j];
        let data = self.impulse_joints.get(joint.handle)?;
        let (b1, b2) = (&self.bodies[data.body1], &self.bodies[data.body2]);
        let f1 = b1.position() * data.data.local_frame1;
        let f2 = b2.position() * data.data.local_frame2;
        match joint.def.kind {
            JointKind::Revolute => {
                let rel = f1.rotation.inverse() * f2.rotation;
                // Signed angle about the frame's X axis (the hinge axis).
                let angle = 2.0 * rel.i.atan2(rel.w);
                let angle = if angle > std::f32::consts::PI {
                    angle - std::f32::consts::TAU
                } else if angle < -std::f32::consts::PI {
                    angle + std::f32::consts::TAU
                } else {
                    angle
                };
                Some(angle.to_degrees())
            }
            JointKind::Prismatic => Some((f1.inverse() * f2).translation.vector.x),
            _ => None,
        }
    }

    /// Recomputes which colliders touch each contact-event collider and emits the enter / exit
    /// events: watches in entity ID order, per watch exits then enters, others in ID order.
    fn detect(&mut self, state: &mut RuntimeState) {
        if self.watches.is_empty() || self.events_halted.is_some() {
            return;
        }
        self.changes.clear();
        let mut events = 0;
        for (w, watch) in self.watches.iter_mut().enumerate() {
            let me = self.entries[watch.entry as usize].collider;
            let scratch = &mut self.scratch;
            scratch.clear();
            let colliders = &self.colliders;
            let mut other = |a: ColliderHandle, b: ColliderHandle| {
                let o = if a == me { b } else { a };
                if let Some(c) = colliders.get(o) {
                    scratch.push(c.user_data as u32);
                }
            };
            for pair in self.narrow_phase.contact_pairs_with(me) {
                if pair.has_any_active_contact {
                    other(pair.collider1, pair.collider2);
                }
            }
            for (a, b, hit) in self.narrow_phase.intersection_pairs_with(me) {
                if hit {
                    other(a, b);
                }
            }
            let rank = &self.rank;
            scratch.sort_unstable_by_key(|&i| rank[i as usize]);
            scratch.dedup();
            for enter in [false, true] {
                let (from, to) = if enter {
                    (&*scratch, &watch.touching)
                } else {
                    (&watch.touching, &*scratch)
                };
                for &o in from {
                    if to
                        .binary_search_by_key(&rank[o as usize], |&i| rank[i as usize])
                        .is_err()
                        && (if enter { watch.on_enter } else { watch.on_exit }).is_some()
                    {
                        events += 1;
                        // Bounded (no growth past the preallocated capacity); an overflow
                        // stops the contact events below.
                        if self.changes.len() < MAX_CONTACT_EVENTS_PER_STEP {
                            self.changes.push(Change {
                                watch: w as u32,
                                other: o,
                                enter,
                            });
                        }
                    }
                }
            }
            std::mem::swap(&mut watch.touching, scratch);
        }
        if events > MAX_CONTACT_EVENTS_PER_STEP {
            self.events_halted = Some(format!(
                "contact event budget exceeded ({events} > {MAX_CONTACT_EVENTS_PER_STEP} events in one step at tick {}); contact events stopped",
                state.tick
            ));
            return;
        }
        let mut emitted_any = false;
        for i in 0..self.changes.len() {
            let c = self.changes[i];
            let watch = &self.watches[c.watch as usize];
            let Some(event) = (if c.enter {
                watch.on_enter
            } else {
                watch.on_exit
            }) else {
                continue;
            };
            let info = &self.events[event as usize];
            let mut fields = BTreeMap::new();
            if info.self_field {
                fields.insert(
                    "self".to_owned(),
                    Scalar::Text(self.entries[watch.entry as usize].id.clone()),
                );
            }
            if info.other_field {
                fields.insert(
                    "other".to_owned(),
                    Scalar::Text(self.entries[c.other as usize].id.clone()),
                );
            }
            let tick = state.tick;
            let result = match state.gameplay.as_mut() {
                Some(g) => g.command(
                    &RuntimeCommand::Emit {
                        event: info.name.clone(),
                        fields,
                    },
                    tick,
                ),
                None => Err(anyhow::anyhow!("behavior.execute is not running")),
            };
            match result {
                Ok(()) => {
                    self.emitted += 1;
                    emitted_any = true;
                }
                // Keep detecting; surface the first failure through observe.
                Err(e) => {
                    let name = &self.events[event as usize].name;
                    self.event_fault
                        .get_or_insert_with(|| format!("emit {name}: {e:#}"));
                }
            }
        }
        if emitted_any {
            state.sync_scenes();
        }
    }

    fn entry_of(&self, state: &RuntimeState, id: &str) -> Result<usize> {
        state
            .compiled
            .indices
            .get(id)
            .and_then(|&slot| self.by_slot.get(slot))
            .filter(|&&i| i != NONE)
            .map(|&i| i as usize)
            .ok_or_else(|| {
                Diagnostic::error(
                    "not_found",
                    "/args/id",
                    format!("{id} has no physics.collider in this world"),
                )
            })
    }

    /// The body a velocity / impulse action may change: dynamic only.
    fn dynamic_of(&self, state: &RuntimeState, args: &Value) -> Result<RigidBodyHandle> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .context("physics actions need {\"id\":\"body entity\"}")?;
        let entry = &self.entries[self.entry_of(state, id)?];
        ensure!(
            entry.role == Role::Dynamic,
            "{id} is a {} collider; only dynamic bodies take impulses and velocities",
            entry.role.name()
        );
        ensure!(!entry.out, "{id} left the world bounds and is disabled");
        Ok(entry.body.expect("dynamic entries have a body"))
    }

    /// Sleeping state of the body on entity `slot` (editor gizmos); `None` without a body.
    pub fn sleeping(&self, slot: usize) -> Option<bool> {
        let i = *self.by_slot.get(slot)?;
        let entry = self.entries.get(i as usize)?;
        entry
            .body
            .filter(|_| entry.role.simulated())
            .map(|h| self.bodies[h].is_sleeping())
    }

    /// Role name of the collider on entity `slot` (editor gizmos).
    pub fn role(&self, slot: usize) -> Option<&'static str> {
        let i = *self.by_slot.get(slot)?;
        self.entries.get(i as usize).map(|e| e.role.name())
    }

    /// Moves every simulated body (and follower) to its entity's current pose (after a
    /// checkpoint load or a hot reload replaced `core.transform`).
    fn sync_poses(&mut self, state: &RuntimeState) {
        for entry in &self.entries {
            let Some(handle) = entry.body else {
                // A static collider sits where its entity is (a hot reload keeps the simulated
                // pose of an entity whose body was just removed).
                let (_, rotation, translation) =
                    world_matrix(state, entry.slot).to_scale_rotation_translation();
                self.colliders[entry.collider].set_position(isometry(rotation, translation));
                continue;
            };
            let pose = match entry.role {
                Role::Follower => {
                    let (_, rotation, translation) =
                        world_matrix(state, entry.slot).to_scale_rotation_translation();
                    isometry(rotation, translation)
                }
                _ => match &state.transforms[entry.slot] {
                    Some(t) => isometry(Quat::from_array(t.rotation), Vec3::from_array(t.position)),
                    None => continue,
                },
            };
            let body = &mut self.bodies[handle];
            body.set_position(pose, false);
            if entry.role == Role::Follower {
                body.set_next_kinematic_position(pose);
            }
        }
    }

    fn summary(&self) -> Value {
        let count = |role: Role| self.entries.iter().filter(|e| e.role == role).count();
        let (mut awake, mut sleeping) = (0, 0);
        let mut listed_awake = vec![];
        for entry in self.entries.iter().filter(|e| e.role.simulated() && !e.out) {
            let body = &self.bodies[entry.body.expect("simulated entries have a body")];
            if !body.is_enabled() {
                continue;
            }
            if body.is_sleeping() {
                sleeping += 1;
            } else {
                awake += 1;
                if listed_awake.len() < LISTED {
                    listed_awake.push(entry.id.as_str());
                }
            }
        }
        let contacts = self
            .narrow_phase
            .contact_pairs()
            .filter(|p| p.has_any_active_contact)
            .count();
        let out: Vec<&str> = self
            .entries
            .iter()
            .filter(|e| e.out)
            .take(LISTED)
            .map(|e| e.id.as_str())
            .collect();
        let mut summary = json!({
            "dynamic": count(Role::Dynamic),
            "kinematic": count(Role::Kinematic),
            "followers": count(Role::Follower),
            "static": count(Role::Static),
            "awake": awake,
            "sleeping": sleeping,
            "contacts": contacts,
            "steps": self.steps,
            "awake_ids": listed_awake,
            "out_of_bounds": out,
            "ignored_spawns": self.ignored_spawns,
            "halted": self.halted,
        });
        // Only when present, so worlds without joints / events keep their observation.
        if !self.joints.is_empty() {
            summary["joints"] = json!(self.joints.len());
        }
        if !self.watches.is_empty() {
            let touching: serde_json::Map<String, Value> = self
                .watches
                .iter()
                .filter(|w| !w.touching.is_empty())
                .take(LISTED)
                .map(|w| {
                    let ids: Vec<&str> = w
                        .touching
                        .iter()
                        .take(LISTED)
                        .map(|&o| self.entries[o as usize].id.as_str())
                        .collect();
                    (self.entries[w.entry as usize].id.clone(), json!(ids))
                })
                .collect();
            summary["contact_events"] = json!({
                "colliders": self.watches.len(),
                "emitted": self.emitted,
                "touching": touching,
                "halted": self.events_halted,
                "fault": self.event_fault,
            });
        }
        summary
    }

    fn inspect(&self, state: &RuntimeState, id: &str) -> Result<Value> {
        let entry = &self.entries[self.entry_of(state, id)?];
        let mut out = json!({"id": id, "kind": entry.role.name(), "enabled": entry.enabled});
        if let Some(handle) = entry.body {
            let body = &self.bodies[handle];
            let p = body.translation();
            out["position"] = json!([round(p.x), round(p.y), round(p.z)]);
            let v = body.linvel();
            out["velocity"] = json!([round(v.x), round(v.y), round(v.z)]);
            let w = body.angvel();
            out["angular_velocity"] = json!([round(w.x), round(w.y), round(w.z)]);
            out["speed"] = json!(round(v.norm()));
            if entry.role == Role::Dynamic {
                out["mass"] = json!(round(body.mass()));
            }
            if entry.role.simulated() {
                out["sleeping"] = json!(body.is_sleeping());
                out["out_of_bounds"] = json!(entry.out);
            }
        }
        if let Some(j) = self.joints.iter().position(|j| j.slot == entry.slot) {
            let joint = &self.joints[j];
            let mut info = json!({"kind": joint.def.kind, "to": joint.def.to});
            if let Some(p) = self.joint_position(j) {
                let key = if joint.def.kind == JointKind::Revolute {
                    "angle"
                } else {
                    "offset"
                };
                info[key] = json!(round(p));
            }
            if let Some(m) = &joint.motor {
                info["motor"] = json!(m);
            }
            out["joint"] = info;
        }
        if let Some(w) = self
            .watches
            .iter()
            .find(|w| self.entries[w.entry as usize].slot == entry.slot)
        {
            out["touching"] = json!(w
                .touching
                .iter()
                .map(|&o| self.entries[o as usize].id.as_str())
                .collect::<Vec<_>>());
        }
        Ok(out)
    }
}

impl RuntimeSystem for PhysicsSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn checkpoint_supported(&self) -> bool {
        true
    }
    fn step(&mut self, state: &mut RuntimeState, _input: &ActionMap) {
        if self.halted.is_some() {
            return;
        }
        let len = state.compiled.len();
        if len > self.scanned {
            for e in &state.compiled.entities[self.scanned..] {
                if e.body.is_some() {
                    self.halted = Some(format!(
                        "runtime-spawned entity {} has {BODY}; spawned rigid bodies are not supported (author them instead)",
                        e.id
                    ));
                    return;
                }
                if e.collider.is_some() {
                    self.ignored_spawns += 1;
                }
            }
            self.scanned = len;
        }
        for entry in &mut self.entries {
            let enabled = state.compiled.enabled[entry.slot] && !entry.out;
            if enabled != entry.enabled {
                entry.enabled = enabled;
                match entry.body {
                    Some(h) => self.bodies[h].set_enabled(enabled),
                    None => self.colliders[entry.collider].set_enabled(enabled),
                }
            }
            if entry.role == Role::Follower && enabled {
                let (_, rotation, translation) =
                    world_matrix(state, entry.slot).to_scale_rotation_translation();
                let target = isometry(rotation, translation);
                let body = &mut self.bodies[entry.body.expect("followers have a body")];
                let current = *body.position();
                // A jump (teleport, a ping-pong path flipping the facing) would become a huge
                // push velocity: place the follower instead of driving it there.
                if (target.translation.vector - current.translation.vector).norm()
                    > TELEPORT_DISTANCE
                {
                    body.set_position(target, true);
                } else {
                    if current.rotation.angle_to(&target.rotation) > TELEPORT_ANGLE {
                        body.set_rotation(target.rotation, true);
                    }
                    body.set_next_kinematic_position(target);
                }
            }
        }
        self.pipeline.step(
            &self.gravity,
            &self.params,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            None,
            &(),
            &(),
        );
        self.steps += 1;
        for entry in &mut self.entries {
            if !entry.role.simulated() || !entry.enabled {
                continue;
            }
            let body = &mut self.bodies[entry.body.expect("simulated entries have a body")];
            if body.is_sleeping() {
                continue;
            }
            let p = *body.translation();
            if !(p.x.abs() <= WORLD_LIMIT && p.y.abs() <= WORLD_LIMIT && p.z.abs() <= WORLD_LIMIT) {
                entry.out = true;
                entry.enabled = false;
                body.set_enabled(false);
                continue;
            }
            let q = body.rotation();
            if let Some(t) = &mut state.transforms[entry.slot] {
                t.position = [p.x, p.y, p.z];
                t.rotation = [q.i, q.j, q.k, q.w];
            }
        }
        self.detect(state);
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        // `WorldRuntime::adopt` already carried each entity's simulated transform when its
        // authored transform is unchanged; bodies follow those poses. Velocities and sleep
        // state carry only for such bodies of the same kind (an edited pose restarts the body
        // from rest at its new authored pose).
        let Some(old) = old.as_any().downcast_ref::<Self>() else {
            return Ok(());
        };
        self.sync_poses(next);
        for entry in &self.entries {
            if !entry.role.simulated() {
                continue;
            }
            let e = &next.compiled.entities[entry.slot];
            let Some(&old_slot) = previous.compiled.indices.get(&e.id) else {
                continue;
            };
            let Some(old_entry) = old
                .by_slot
                .get(old_slot)
                .filter(|&&i| i != NONE)
                .map(|&i| &old.entries[i as usize])
            else {
                continue;
            };
            // An edited velocity (a kinematic body's whole behaviour) restarts from the new value.
            let velocities = |b: Option<&axiom_core::physics::RigidBody>| {
                b.map(|b| (b.velocity, b.angular_velocity))
            };
            if old_entry.role != entry.role
                || old_entry.out
                || previous.compiled.entities[old_slot].transform != e.transform
                || velocities(previous.compiled.entities[old_slot].body.as_ref())
                    != velocities(e.body.as_ref())
            {
                continue;
            }
            let from = &old.bodies[old_entry.body.expect("simulated entries have a body")];
            let to = &mut self.bodies[entry.body.expect("simulated entries have a body")];
            to.set_linvel(*from.linvel(), false);
            to.set_angvel(*from.angvel(), false);
            to.activation_mut().time_since_can_sleep = from.activation().time_since_can_sleep;
            if from.is_sleeping() {
                to.sleep();
            }
        }
        // Motor targets set at runtime carry while the authored joint is unchanged.
        for j in 0..self.joints.len() {
            let id = &next.compiled.entities[self.joints[j].slot].id;
            let carried = old.joints.iter().find(|o| {
                previous.compiled.entities[o.slot].id == *id && o.def == self.joints[j].def
            });
            if let Some(motor) = carried.and_then(|o| o.motor) {
                if Some(motor) != self.joints[j].motor {
                    self.set_motor(j, motor);
                }
            }
        }
        // Contacts carry by entity ID (a deleted collider drops out without an exit event),
        // so a reload does not re-fire on_enter for contacts that already existed.
        for w in 0..self.watches.len() {
            let id = &self.entries[self.watches[w].entry as usize].id;
            let Some(o) = old
                .watches
                .iter()
                .find(|o| old.entries[o.entry as usize].id == *id)
            else {
                continue;
            };
            let mut touching: Vec<u32> = o
                .touching
                .iter()
                .filter_map(|&i| self.entry_of(next, &old.entries[i as usize].id).ok())
                .map(|i| i as u32)
                .collect();
            touching.sort_unstable_by_key(|&i| self.rank[i as usize]);
            self.watches[w].touching = touching;
        }
        self.emitted = old.emitted;
        self.events_halted = old.events_halted.clone();
        self.event_fault = old.event_fault.clone();
        self.steps = old.steps;
        Ok(())
    }
    fn save_state(&self, state: &RuntimeState) -> Option<Result<Value>> {
        let bodies = self
            .entries
            .iter()
            .filter(|e| e.role.simulated())
            .map(|e| {
                let body = &self.bodies[e.body.expect("simulated entries have a body")];
                (
                    state.compiled.entities[e.slot].id.clone(),
                    SavedBody {
                        velocity: vec3(body.linvel()),
                        angular_velocity: vec3(body.angvel()),
                        sleeping: body.is_sleeping(),
                        still: body.activation().time_since_can_sleep,
                        out: e.out,
                    },
                )
            })
            .collect();
        let motors = self
            .joints
            .iter()
            .filter_map(|j| Some((state.compiled.entities[j.slot].id.clone(), j.motor?)))
            .collect();
        let touching = self
            .watches
            .iter()
            .map(|w| {
                (
                    self.entries[w.entry as usize].id.clone(),
                    w.touching
                        .iter()
                        .map(|&o| self.entries[o as usize].id.clone())
                        .collect(),
                )
            })
            .collect();
        Some(
            serde_json::to_value(SavedPhysics {
                steps: self.steps,
                bodies,
                motors,
                touching,
                events: self.emitted,
            })
            .map_err(Into::into),
        )
    }
    fn load_state(&mut self, state: &mut RuntimeState, saved: &Value) -> Result<()> {
        let state = &*state;
        let saved: SavedPhysics =
            serde_json::from_value(saved.clone()).context("physics.step checkpoint state")?;
        let simulated = self.entries.iter().filter(|e| e.role.simulated()).count();
        ensure!(
            saved.bodies.len() == simulated,
            "checkpoint physics layout mismatch ({} saved bodies, {simulated} in this world)",
            saved.bodies.len()
        );
        let motorized = self.joints.iter().filter(|j| j.motor.is_some()).count();
        ensure!(
            saved.motors.len() == motorized && saved.touching.len() == self.watches.len(),
            "checkpoint physics layout mismatch ({} saved joint motors and {} contact-event colliders, {motorized} and {} in this world)",
            saved.motors.len(),
            saved.touching.len(),
            self.watches.len()
        );
        // Joint and contact state are checked completely before any of it is applied.
        let mut motors = Vec::with_capacity(motorized);
        for (j, joint) in self.joints.iter().enumerate() {
            if joint.motor.is_none() {
                continue;
            }
            let id = &state.compiled.entities[joint.slot].id;
            let m = *saved
                .motors
                .get(id)
                .with_context(|| format!("checkpoint physics state lacks joint motor {id}"))?;
            let mut def = joint.def.clone();
            if let Some(motor) = &mut def.motor {
                motor.target_velocity = m.target_velocity;
                motor.target_position = m.target_position;
            }
            def.validate()
                .with_context(|| format!("invalid checkpoint joint motor for {id}"))?;
            motors.push((j, m));
        }
        let mut touching = Vec::with_capacity(self.watches.len());
        for watch in &self.watches {
            let id = &self.entries[watch.entry as usize].id;
            let ids = saved
                .touching
                .get(id)
                .with_context(|| format!("checkpoint physics state lacks contacts of {id}"))?;
            let mut list = ids
                .iter()
                .map(|o| {
                    self.entry_of(state, o)
                        .map(|i| i as u32)
                        .with_context(|| format!("checkpoint contact of {id}: {o}"))
                })
                .collect::<Result<Vec<u32>>>()?;
            list.sort_unstable_by_key(|&i| self.rank[i as usize]);
            list.dedup();
            touching.push(list);
        }
        for (j, m) in motors {
            self.set_motor(j, m);
        }
        for (watch, list) in self.watches.iter_mut().zip(touching) {
            watch.touching = list;
        }
        self.emitted = saved.events;
        self.sync_poses(state);
        for entry in &mut self.entries {
            if !entry.role.simulated() {
                continue;
            }
            let id = &state.compiled.entities[entry.slot].id;
            let s = saved
                .bodies
                .get(id)
                .with_context(|| format!("checkpoint physics state lacks body {id}"))?;
            ensure!(
                s.velocity
                    .iter()
                    .chain(&s.angular_velocity)
                    .all(|v| v.is_finite() && v.abs() <= MAX_SPEED)
                    && s.still.is_finite()
                    && s.still >= 0.0,
                "invalid checkpoint physics state for {id}"
            );
            let body = &mut self.bodies[entry.body.expect("simulated entries have a body")];
            let [vx, vy, vz] = s.velocity;
            let [wx, wy, wz] = s.angular_velocity;
            body.set_linvel(vector![vx, vy, vz], false);
            body.set_angvel(vector![wx, wy, wz], false);
            body.activation_mut().time_since_can_sleep = s.still;
            if s.sleeping {
                body.sleep();
            }
            if s.out {
                entry.out = true;
                entry.enabled = false;
                body.set_enabled(false);
            }
        }
        self.steps = saved.steps;
        Ok(())
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "physics" {
            return None;
        }
        let triple = |key: &str| -> Result<Option<Vector<Real>>> {
            args.get(key)
                .map(|v| {
                    let [x, y, z]: [f32; 3] = serde_json::from_value(v.clone())
                        .with_context(|| format!("{key} must be [x,y,z]"))?;
                    ensure!(
                        [x, y, z]
                            .iter()
                            .all(|c| c.is_finite() && c.abs() <= MAX_SPEED),
                        "{key} components must be finite and within ±{MAX_SPEED}"
                    );
                    Ok(vector![x, y, z])
                })
                .transpose()
        };
        Some(match action {
            "summary" => Ok(self.summary()),
            "inspect" => args
                .get("id")
                .and_then(Value::as_str)
                .context("physics.inspect needs {\"id\":\"entity with physics.collider\"}")
                .and_then(|id| self.inspect(state, id)),
            "impulse" => (|| {
                let handle = self.dynamic_of(state, args)?;
                let impulse =
                    triple("impulse")?.context("physics.impulse needs impulse [x,y,z]")?;
                self.bodies[handle].apply_impulse(impulse, true);
                self.inspect(state, args["id"].as_str().unwrap_or_default())
            })(),
            "set_velocity" => (|| {
                let handle = self.dynamic_of(state, args)?;
                let (v, w) = (triple("velocity")?, triple("angular_velocity")?);
                ensure!(
                    v.is_some() || w.is_some(),
                    "physics.set_velocity needs velocity and/or angular_velocity"
                );
                let body = &mut self.bodies[handle];
                if let Some(v) = v {
                    body.set_linvel(v, true);
                }
                if let Some(w) = w {
                    body.set_angvel(w, true);
                }
                self.inspect(state, args["id"].as_str().unwrap_or_default())
            })(),
            "set_motor" => (|| {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .context("physics.set_motor needs {\"id\":\"joint entity\"}")?;
                let slot = self.entries[self.entry_of(state, id)?].slot;
                let j = self
                    .joints
                    .iter()
                    .position(|j| j.slot == slot)
                    .with_context(|| format!("{id} has no {JOINT}"))?;
                let current = self.joints[j]
                    .motor
                    .with_context(|| format!("{id}'s joint has no motor (author one first)"))?;
                let number = |key: &str| -> Result<Option<f32>> {
                    args.get(key)
                        .map(|v| {
                            v.as_f64()
                                .map(|n| n as f32)
                                .with_context(|| format!("{key} must be a number"))
                        })
                        .transpose()
                };
                let (velocity, position) = (number("target_velocity")?, number("target_position")?);
                ensure!(
                    velocity.is_some() || position.is_some(),
                    "physics.set_motor needs target_velocity and/or target_position"
                );
                let target = SavedMotor {
                    target_velocity: velocity.unwrap_or(current.target_velocity),
                    target_position: position.or(current.target_position),
                };
                // Same ranges (and the stiffness rule for target_position) as the authored motor.
                let mut def = self.joints[j].def.clone();
                if let Some(m) = &mut def.motor {
                    m.target_velocity = target.target_velocity;
                    m.target_position = target.target_position;
                }
                def.validate()?;
                self.set_motor(j, target);
                self.inspect(state, id)
            })(),
            other => Err(anyhow::anyhow!("unknown physics action {other}")),
        })
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("physics".into(), self.summary())])
    }
    fn validate(&self, state: &RuntimeState) -> Result<()> {
        ensure!(
            self.entries.iter().all(|e| e.body.is_none_or(|h| {
                let b = &self.bodies[h];
                b.translation().iter().all(|v| v.is_finite())
                    && b.linvel().iter().all(|v| v.is_finite())
            })),
            "non-finite rigid body state"
        );
        state.validate()
    }
}

fn id_arg(v: &Value) -> Result<()> {
    ensure!(
        v.get("id").and_then(Value::as_str).is_some(),
        "needs {{\"id\":\"entity ID\"}}"
    );
    Ok(())
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.action(
        "physics.impulse",
        "physics",
        json!({"id":"dynamic body entity ID","impulse":"[x,y,z] N·s applied at the center of mass; wakes the body"}),
        |v| {
            id_arg(v)?;
            ensure!(v.get("impulse").is_some(), "physics.impulse needs impulse [x,y,z]");
            Ok(())
        },
    );
    r.authoring.action(
        "physics.set_velocity",
        "physics",
        json!({"id":"dynamic body entity ID","velocity":"optional [x,y,z] m/s","angular_velocity":"optional [x,y,z] radians/second"}),
        id_arg,
    );
    r.authoring.action(
        "physics.inspect",
        "physics",
        json!({"id":"entity with physics.collider; kind, position, velocity, angular_velocity, speed, mass, sleeping; joint {kind,to,angle|offset,motor}; touching (contact-event colliders)"}),
        id_arg,
    );
    r.authoring.action(
        "physics.set_motor",
        "physics",
        json!({"id":"entity with a motorized physics.joint","target_velocity":"optional number: degrees/second (revolute) or m/s (prismatic)","target_position":"optional number: degrees or meters from the authored pose (the motor needs stiffness > 0)"}),
        id_arg,
    );
    r.authoring
        .action("physics.summary", "physics", json!({}), |_| Ok(()));
    r.factories.insert(SYSTEM.into(), |state| {
        Ok(Box::new(PhysicsSystem::new(state)?))
    });
}
