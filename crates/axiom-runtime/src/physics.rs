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
    // The bouncier surface wins (a lively ball bounces on a default floor); friction keeps
    // the average of the two.
    builder.restitution_combine_rule(CoefficientCombineRule::Max)
}

fn vec3(v: &Vector<Real>) -> [f32; 3] {
    [v.x, v.y, v.z]
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
                    let mut builder = shape(collider, scale);
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
        Ok(system)
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
        json!({
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
        })
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
        self.steps = old.steps;
        Ok(())
    }
    fn checkpoint_state(&self, state: &RuntimeState) -> Option<Value> {
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
        serde_json::to_value(SavedPhysics {
            steps: self.steps,
            bodies,
        })
        .ok()
    }
    fn restore_checkpoint_state(
        &mut self,
        state: &RuntimeState,
        saved: Option<&Value>,
    ) -> Result<()> {
        let saved: SavedPhysics = serde_json::from_value(
            saved
                .context("checkpoint has no physics.step state (saved by an engine without rigid bodies)")?
                .clone(),
        )
        .context("physics.step checkpoint state")?;
        let simulated = self.entries.iter().filter(|e| e.role.simulated()).count();
        ensure!(
            saved.bodies.len() == simulated,
            "checkpoint physics layout mismatch ({} saved bodies, {simulated} in this world)",
            saved.bodies.len()
        );
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
        json!({"id":"entity with physics.collider; kind, position, velocity, angular_velocity, speed, mass, sleeping"}),
        id_arg,
    );
    r.authoring
        .action("physics.summary", "physics", json!({}), |_| Ok(()));
    r.factories.insert(SYSTEM.into(), |state| {
        Ok(Box::new(PhysicsSystem::new(state)?))
    });
}
