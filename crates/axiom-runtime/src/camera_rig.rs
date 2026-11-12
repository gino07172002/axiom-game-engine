//! `camera.rig`: runtime side of `camera.follow` (see `axiom_core::camera`) and the runtime
//! camera actions (`camera.follow.set`, `camera.activate`, `camera.inspect`,
//! `camera.summary`).
//!
//! The rig runs at the fixed step, after every mover: the engine renders the latest
//! simulated state without interpolation, so the target only moves at fixed steps and a
//! per-render-frame camera would see the same target pose between steps; updating in the
//! same step as the target keeps the two from drifting apart (no relative jitter) and keeps
//! runs deterministic and testable headless. Smoothing is exponential with half-lives, so it
//! does not depend on the step size.
//!
//! Each step per camera: input moves the yaw / pitch / distance goals; the current values
//! approach them (`rotation_smoothing`); the look point trails the target pivot
//! (`position_smoothing`) but is cast from the pivot so it never trails through a blocker;
//! a sphere cast from the look point toward the desired eye gives the allowed distance. The
//! camera moves in to a nearer allowed distance at once and back out smoothly (`recover`),
//! never closer than `min_distance`. Static blockers are collected once per runtime build
//! and bucketed in a grid; a steady step does not allocate.
use crate::trigger::pivot;
use crate::world::*;
use anyhow::{ensure, Context, Result};
use axiom_core::camera::*;
use axiom_core::raycast::{BlockerGrid, CastScratch};
use glam::Vec3;
use serde_json::{json, Value};
use std::{any::Any, f32::consts::TAU};

struct Rig {
    slot: usize,
    id: String,
    def: CameraFollow,
    target: usize,
    target_id: String,
    /// The target came from `camera.follow.set` (kept across a reload that leaves the
    /// authored component unchanged).
    target_set: bool,
    /// Sorted slots that never block this camera: the target and its descendants.
    excluded: Vec<usize>,
    /// `(target ID, bone)` socket the look point follows (`target_bone`), the key of
    /// `animation_cues.sockets` kept here so a step looks it up without allocating.
    socket: Option<(String, String)>,
    /// Pointer drag in progress: (start screen position, yaw goal, pitch goal at the start).
    drag: Option<([f32; 2], f32, f32)>,
    /// Goals (radians / meters) that input and `camera.follow.set` move.
    yaw_goal: f32,
    pitch_goal: f32,
    distance_goal: f32,
    goals_set: bool,
    /// Smoothed state.
    yaw: f32,
    pitch: f32,
    distance: f32,
    focus: Vec3,
    /// Collision-limited distance from the look point.
    actual: f32,
    /// `actual` ended the last step nearer than the desired distance (blocked, or still
    /// moving back out): it recovers smoothly instead of following zoom changes at once.
    pulled: bool,
    started: bool,
    /// Last step, for observe.
    allowed: f32,
    hit: Option<usize>,
    focus_hit: Option<usize>,
    tests: usize,
    over_budget: u64,
}

impl Rig {
    fn new(state: &RuntimeState, slot: usize, def: &CameraFollow) -> Result<Self> {
        let compiled = &state.compiled;
        let target = *compiled
            .indices
            .get(&def.target)
            .with_context(|| format!("follow target {} does not exist", def.target))?;
        let mut rig = Self {
            slot,
            id: compiled.entities[slot].id.clone(),
            def: def.clone(),
            target,
            target_id: def.target.clone(),
            target_set: false,
            excluded: vec![],
            socket: def
                .target_bone
                .as_ref()
                .map(|b| (def.target.clone(), b.clone())),
            drag: None,
            yaw_goal: 0.0,
            pitch_goal: 0.0,
            distance_goal: 0.0,
            goals_set: false,
            yaw: 0.0,
            pitch: 0.0,
            distance: 0.0,
            focus: Vec3::ZERO,
            actual: 0.0,
            pulled: false,
            started: false,
            allowed: 0.0,
            hit: None,
            focus_hit: None,
            tests: 0,
            over_budget: 0,
        };
        rig.reset_goals();
        rig.exclude(state);
        Ok(rig)
    }

    fn reset_goals(&mut self) {
        self.yaw_goal = wrap(self.def.yaw.to_radians());
        self.pitch_goal = self.def.pitch.to_radians();
        self.distance_goal = self.def.distance;
        self.goals_set = false;
    }

    /// The target and every entity under it.
    fn exclude(&mut self, state: &RuntimeState) {
        let parents = &state.compiled.parents;
        self.excluded = (0..parents.len())
            .filter(|&i| {
                let mut cursor = Some(i);
                let mut depth = 0;
                while let Some(c) = cursor {
                    if c == self.target {
                        return true;
                    }
                    cursor = parents[c];
                    depth += 1;
                    if depth > parents.len() {
                        break;
                    }
                }
                false
            })
            .collect();
    }

    fn json(&self, state: &RuntimeState, names: &[String], grid: &BlockerGrid) -> Value {
        let name = |slot: Option<usize>| slot.and_then(|s| names.get(s)).map(String::as_str);
        let position = state.transforms[self.slot]
            .as_ref()
            .map(|t| t.position.map(|v| round(v, 1000.0)));
        json!({
            "id": self.id,
            "target": self.target_id,
            "bone": self.socket.as_ref().map(|(_, b)| b),
            "dragging": self.drag.is_some(),
            "enabled": self.def.enabled,
            "started": self.started,
            "desired": round(self.distance, 1000.0),
            "goal": round(self.distance_goal, 1000.0),
            "allowed": round(self.allowed, 1000.0),
            "actual": round(self.actual, 1000.0),
            "min_distance": self.def.min_distance,
            "hit": name(self.hit),
            "focus_hit": name(self.focus_hit),
            "yaw": round(self.yaw.to_degrees(), 100.0),
            "pitch": round(self.pitch.to_degrees(), 100.0),
            "focus": self.focus.to_array().map(|v| round(v, 1000.0)),
            "position": position,
            "collision": self.def.collision.enabled,
            "tests": self.tests,
            "over_budget": self.over_budget,
            "blockers": grid.len(),
        })
    }
}

/// Meters a clamped look point stays in front of the blocker it was stopped by.
const FOCUS_BACKOFF: f32 = 1e-3;
/// Meters: a camera this close to its desired distance is no longer recovering.
const PULLED_EPSILON: f32 = 1e-3;

fn wrap(angle: f32) -> f32 {
    use std::f32::consts::PI;
    (angle + PI).rem_euclid(TAU) - PI
}

/// Display rounding for observe (f64 so the JSON shows the short decimal).
fn round(v: f32, scale: f64) -> f64 {
    (f64::from(v) * scale).round() / scale
}

pub struct CameraRigSystem {
    rigs: Vec<Rig>,
    /// Entity IDs by slot at build (blocker owners are always built entities).
    names: Vec<String>,
    /// Cameras whose `render.camera.active` is set, kept for observe.
    active: Vec<String>,
    grid: BlockerGrid,
    scratch: CastScratch,
    /// Camera made the active one by `camera.activate` (re-applied after a reload that leaves
    /// the authored camera flags unchanged).
    activated: Option<String>,
    /// Authored (id, active) of every camera of this runtime's world.
    authored_active: Vec<(String, bool)>,
    scanned: usize,
    /// Set when a runtime spawn brings an unsupported follow camera; the rig stops and
    /// observe says why.
    halted: Option<String>,
}

impl CameraRigSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let compiled = &state.compiled;
        let mut rigs = vec![];
        for (slot, e) in compiled.entities.iter().enumerate() {
            let Some(def) = &e.camera_follow else {
                continue;
            };
            ensure!(
                rigs.len() < MAX_FOLLOW_CAMERAS,
                "follow camera budget exceeded (> {MAX_FOLLOW_CAMERAS})"
            );
            ensure!(
                state.transforms[slot].is_some() && e.camera.is_some(),
                "follow camera {} needs core.transform and render.camera",
                e.id
            );
            rigs.push(Rig::new(state, slot, def)?);
        }
        let grid = if rigs.iter().any(|r| r.def.collision.enabled) {
            let blockers = collect_blockers(
                &compiled.entities,
                &compiled.parents,
                &state.registry.movers,
            )?;
            let radius = rigs
                .iter()
                .map(|r| r.def.collision.radius)
                .fold(0.0, f32::max);
            BlockerGrid::new(
                blockers
                    .into_iter()
                    .map(|b| (b.shape, b.owner as u32))
                    .collect(),
                radius,
            )
        } else {
            BlockerGrid::default()
        };
        let mut scratch = CastScratch::default();
        // Size the dedupe marks now so the first step does not allocate.
        if !grid.is_empty() {
            grid.cast(Vec3::ZERO, Vec3::ZERO, 0.0, 0, &mut scratch, |_| true);
        }
        Ok(Self {
            rigs,
            names: compiled.entities.iter().map(|e| e.id.clone()).collect(),
            active: active_cameras(state),
            grid,
            scratch,
            activated: None,
            authored_active: authored_active(state),
            scanned: compiled.len(),
            halted: None,
        })
    }

    /// Camera pose and look point of the follow camera on entity `slot` as of the last
    /// step (`None` before its first step or for other entities).
    pub fn pose(&self, slot: usize) -> Option<(Vec3, Vec3)> {
        let r = self.rigs.iter().find(|r| r.slot == slot && r.started)?;
        let dir = orbit_direction(r.yaw, r.pitch);
        Some((r.focus + dir * r.actual, r.focus))
    }

    fn rig(&self, args: &Value) -> Result<usize> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .context("camera actions need {\"id\":\"camera entity\"}")?;
        self.rigs.iter().position(|r| r.id == id).ok_or_else(|| {
            axiom_core::Diagnostic::error(
                "not_found",
                "/args/id",
                format!("{id} is not a follow camera (no camera.follow)"),
            )
        })
    }

    fn summary(&self) -> Value {
        let (cells, big, cell) = self.grid.layout();
        json!({
            "follow": self.rigs.iter().map(|r| json!({
                "id": r.id,
                "target": r.target_id,
                "desired": round(r.distance, 1000.0),
                "actual": round(r.actual, 1000.0),
                "hit": r.hit.and_then(|s| self.names.get(s)),
                "tests": r.tests,
            })).collect::<Vec<_>>(),
            "active": self.active,
            "activated": self.activated,
            "blockers": self.grid.len(),
            "grid": {"cells": cells, "always_tested": big, "cell": cell},
            "over_budget": self.rigs.iter().map(|r| r.over_budget).sum::<u64>(),
            "halted": self.halted,
        })
    }

    fn activate(&mut self, state: &mut RuntimeState, id: &str) -> Result<Value> {
        let slot = *state.compiled.indices.get(id).ok_or_else(|| {
            axiom_core::Diagnostic::error("not_found", "/args/id", format!("{id} does not exist"))
        })?;
        if state.compiled.entities[slot].camera.is_none() {
            return Err(axiom_core::Diagnostic::error(
                "not_a_camera",
                "/args/id",
                format!("{id} has no render.camera"),
            ));
        }
        // Input contexts decide which cameras show (switch with input.activate); turning
        // their cameras off here could leave a context with nothing on screen.
        ensure!(
            state.contexts.is_none(),
            "this world has input contexts, which own the active cameras; use input.activate"
        );
        ensure!(
            state.compiled.enabled[slot],
            "camera {id} is in an inactive scene; activate its scene first"
        );
        let deactivated = set_active(state, id);
        self.activated = Some(id.to_owned());
        self.active = active_cameras(state);
        Ok(json!({
            "active": id,
            "deactivated": deactivated,
            "enabled": state.compiled.enabled[slot],
        }))
    }

    fn set(&mut self, state: &mut RuntimeState, args: &Value) -> Result<Value> {
        let SetPlan {
            rig: i,
            target,
            distance,
            pitch,
            yaw,
            bone,
            target_id,
        } = self.plan_set(state, args)?;
        if let Some(Some(b)) = &bone {
            request_socket(state, &target_id, b)?;
        }
        let r = &mut self.rigs[i];
        if let Some(t) = target {
            r.target = t;
            r.target_id = target_id.clone();
            r.target_set = true;
            r.exclude(state);
            if bone.is_none() {
                r.socket = None;
            }
        }
        if let Some(b) = bone {
            r.socket = b.map(|b| (target_id, b));
        }
        if let Some(d) = distance {
            r.distance_goal = d;
            r.goals_set = true;
        }
        if let Some(p) = pitch {
            r.pitch_goal = p.to_radians();
            r.goals_set = true;
        }
        if let Some(y) = yaw {
            r.yaw_goal = wrap(y.to_radians());
            r.goals_set = true;
        }
        Ok(r.json(state, &self.names, &self.grid))
    }

    /// Everything `camera.follow.set` checks, without changing anything (also the batch
    /// dry run).
    fn plan_set(&self, state: &RuntimeState, args: &Value) -> Result<SetPlan> {
        let i = self.rig(args)?;
        let r = &self.rigs[i];
        let target = match args.get("target").and_then(Value::as_str) {
            Some(id) => {
                let &t = state.compiled.indices.get(id).ok_or_else(|| {
                    axiom_core::Diagnostic::error(
                        "not_found",
                        "/args/target",
                        format!("target {id} does not exist"),
                    )
                })?;
                let mut cursor = Some(t);
                while let Some(c) = cursor {
                    ensure!(
                        c != r.slot,
                        "target {id} is the camera itself or one of its descendants"
                    );
                    cursor = state.compiled.parents[c];
                }
                ensure!(
                    state.transforms[t].is_some(),
                    "target {id} has no core.transform"
                );
                ensure!(
                    state.compiled.entities[t].camera_follow.is_none(),
                    "target {id} is itself a follow camera; follow its target instead"
                );
                Some(t)
            }
            None => None,
        };
        let number = |k: &str| args.get(k).and_then(Value::as_f64).map(|v| v as f32);
        let distance = number("distance");
        if let Some(d) = distance {
            ensure!(
                (r.def.min_distance..=r.def.max_distance).contains(&d),
                "distance must lie within min_distance..max_distance ({}..{})",
                r.def.min_distance,
                r.def.max_distance
            );
        }
        let pitch = number("pitch");
        if let Some(p) = pitch {
            ensure!(
                (r.def.min_pitch..=r.def.max_pitch).contains(&p),
                "pitch must lie within min_pitch..max_pitch ({}..{} degrees)",
                r.def.min_pitch,
                r.def.max_pitch
            );
        }
        let yaw = number("yaw");
        if let Some(y) = yaw {
            ensure!(
                y.is_finite() && (-3600.0..=3600.0).contains(&y),
                "yaw must be -3600..3600 degrees"
            );
        }
        // `bone`: a bone of the (new) target to look at, or null for its pivot.
        let bone = match args.get("bone") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(b)) => Some(Some(b.clone())),
            Some(_) => anyhow::bail!("bone must be a bone name or null"),
        };
        let target_id = target.map_or_else(
            || r.target_id.clone(),
            |t| state.compiled.entities[t].id.clone(),
        );
        if let Some(Some(b)) = &bone {
            check_bone(state, &target_id, b)?;
        }
        Ok(SetPlan {
            rig: i,
            target,
            distance,
            pitch,
            yaw,
            bone,
            target_id,
        })
    }
}

/// A checked `camera.follow.set`.
struct SetPlan {
    rig: usize,
    target: Option<usize>,
    distance: Option<f32>,
    pitch: Option<f32>,
    yaw: Option<f32>,
    /// `Some(None)` = back to the pivot.
    bone: Option<Option<String>>,
    target_id: String,
}

fn check_bone(state: &RuntimeState, target: &str, bone: &str) -> Result<()> {
    let bones = state.animation_cues.bone_names.get(target).with_context(|| {
        format!("target_bone needs an animation.player on {target} (and the animation.evaluate system)")
    })?;
    ensure!(
        bones.contains(bone),
        "{target} has no bone {bone} (e.g. {})",
        bones.iter().take(8).cloned().collect::<Vec<_>>().join(", ")
    );
    Ok(())
}

/// Asks `animation.evaluate` (which steps before the rig) to publish the world matrix of
/// `bone` of `target` every step.
fn request_socket(state: &mut RuntimeState, target: &str, bone: &str) -> Result<()> {
    check_bone(state, target, bone)?;
    state
        .animation_cues
        .socket_requests
        .entry(target.to_owned())
        .or_default()
        .insert(bone.to_owned());
    Ok(())
}

/// IDs of the cameras whose `render.camera.active` is set.
fn active_cameras(state: &RuntimeState) -> Vec<String> {
    state
        .compiled
        .entities
        .iter()
        .filter(|e| e.camera.as_ref().is_some_and(|c| c.active))
        .map(|e| e.id.clone())
        .collect()
}

/// Authored `render.camera.active` of every camera.
fn authored_active(state: &RuntimeState) -> Vec<(String, bool)> {
    state
        .compiled
        .entities
        .iter()
        .filter_map(|e| Some((e.id.clone(), e.camera.as_ref()?.active)))
        .collect()
}

/// Makes `id` the only active camera; returns the cameras that were switched off.
fn set_active(state: &mut RuntimeState, id: &str) -> Vec<String> {
    let mut off = vec![];
    for e in &mut state.compiled.entities {
        if let Some(camera) = &mut e.camera {
            let on = e.id == id;
            if camera.active && !on {
                off.push(e.id.clone());
            }
            camera.active = on;
        }
    }
    off
}

impl RuntimeSystem for CameraRigSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, input: &ActionMap) {
        if self.halted.is_some() {
            return;
        }
        let len = state.compiled.len();
        if len > self.scanned {
            if let Some(e) = state.compiled.entities[self.scanned..]
                .iter()
                .find(|e| e.camera_follow.is_some())
            {
                self.halted = Some(format!(
                    "runtime-spawned entity {} has camera.follow; spawned follow cameras are not supported (author them instead)",
                    e.id
                ));
                return;
            }
            self.scanned = len;
        }
        let Self {
            rigs,
            grid,
            scratch,
            ..
        } = self;
        for r in rigs.iter_mut() {
            if !r.def.enabled || !state.compiled.enabled[r.slot] {
                continue;
            }
            let def = &r.def;
            let axis = |name: &str| {
                if name.is_empty() {
                    0.0
                } else {
                    input.axis(name)
                }
            };
            let (yaw_in, pitch_in, zoom_in) = (
                axis(&def.input.yaw_axis),
                axis(&def.input.pitch_axis),
                axis(&def.input.zoom_axis),
            );
            if yaw_in != 0.0 {
                r.yaw_goal =
                    wrap(r.yaw_goal + yaw_in * def.input.yaw_speed.to_radians() * FIXED_DT);
                r.goals_set = true;
            }
            if pitch_in != 0.0 {
                r.pitch_goal = (r.pitch_goal
                    + pitch_in * def.input.pitch_speed.to_radians() * FIXED_DT)
                    .clamp(def.min_pitch.to_radians(), def.max_pitch.to_radians());
                r.goals_set = true;
            }
            if zoom_in != 0.0 {
                r.distance_goal = (r.distance_goal + zoom_in * def.input.zoom_speed * FIXED_DT)
                    .clamp(def.min_distance, def.max_distance);
                r.goals_set = true;
            }
            let base = match r
                .socket
                .as_ref()
                .and_then(|key| state.animation_cues.sockets.get(key))
            {
                Some(m) => m.w_axis.truncate(),
                None => pivot(state, r.target),
            };
            let pivot = base + Vec3::Y * def.look_height;
            let enabled = &state.compiled.enabled;
            let excluded = &r.excluded;
            let skip = |owner: u32| {
                let owner = owner as usize;
                !enabled[owner] || excluded.binary_search(&owner).is_ok()
            };
            let collide = def.collision.enabled;
            let radius = def.collision.radius;
            if r.started {
                let hl = def.rotation_smoothing;
                r.yaw = damp_angle(r.yaw, r.yaw_goal, hl, FIXED_DT);
                r.pitch = damp(r.pitch, r.pitch_goal, hl, FIXED_DT);
                r.distance = damp(r.distance, r.distance_goal, hl, FIXED_DT);
                let trailing = damp_vec(r.focus, pivot, def.position_smoothing, FIXED_DT);
                r.focus = trailing;
                r.focus_hit = None;
                if collide && trailing != pivot {
                    let cast = grid.cast(
                        pivot,
                        trailing - pivot,
                        radius,
                        MAX_BLOCKER_TESTS,
                        scratch,
                        skip,
                    );
                    if cast.over_budget {
                        r.focus = pivot;
                        r.over_budget += 1;
                    } else if let Some((t, shape)) = cast.hit {
                        // Stop just short of the contact: a look point on a blocker's
                        // surface would count as inside it and the next cast would ignore it.
                        let length = (trailing - pivot).length();
                        let back = ((t * length - FOCUS_BACKOFF).max(0.0)) / length;
                        r.focus = pivot + (trailing - pivot) * back;
                        r.focus_hit = Some(grid.owner(shape) as usize);
                    }
                }
            } else {
                r.yaw = r.yaw_goal;
                r.pitch = r.pitch_goal;
                r.distance = r.distance_goal;
                r.focus = pivot;
                r.focus_hit = None;
            }
            let dir = orbit_direction(r.yaw, r.pitch);
            r.allowed = r.distance;
            r.hit = None;
            r.tests = 0;
            if collide {
                let cast = grid.cast(
                    r.focus,
                    dir * r.distance,
                    radius,
                    MAX_BLOCKER_TESTS,
                    scratch,
                    skip,
                );
                r.tests = cast.tests;
                if cast.over_budget {
                    r.allowed = def.min_distance;
                    r.over_budget += 1;
                } else if let Some((t, shape)) = cast.hit {
                    r.allowed = r.distance * t;
                    r.hit = Some(grid.owner(shape) as usize);
                }
            }
            r.actual = if !r.started || r.allowed < r.actual || !r.pulled {
                r.allowed
            } else {
                damp(r.actual, r.allowed, def.collision.recover, FIXED_DT)
            };
            r.actual = r.actual.max(def.min_distance);
            r.pulled = r.actual < r.distance - PULLED_EPSILON;
            r.started = true;
            let eye = r.focus + dir * r.actual;
            let rotation = look_rotation(eye, r.focus);
            if let Some(t) = &mut state.transforms[r.slot] {
                t.position = eye.to_array();
                t.rotation = rotation.to_array();
            }
        }
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        // Hot reload keeps every follow camera's smoothed pose by camera ID (no pop); goals
        // and a runtime target from camera.follow.set are kept while the authored component
        // is unchanged, otherwise the camera eases to the new authored values.
        let Some(old) = old.as_any().downcast_ref::<Self>() else {
            return Ok(());
        };
        for r in &mut self.rigs {
            let Some(o) = old.rigs.iter().find(|o| o.id == r.id) else {
                continue;
            };
            r.yaw = o.yaw;
            r.pitch = o.pitch;
            r.distance = o.distance;
            r.focus = o.focus;
            r.actual = o.actual;
            r.pulled = o.pulled;
            r.allowed = o.allowed;
            r.started = o.started;
            r.over_budget = o.over_budget;
            if o.def != r.def {
                continue;
            }
            if o.goals_set {
                r.yaw_goal = o.yaw_goal;
                r.pitch_goal = o.pitch_goal;
                r.distance_goal = o.distance_goal;
                r.goals_set = true;
            }
            if o.target_set {
                let id = &previous.compiled.entities[o.target].id;
                if let Some(&t) = next.compiled.indices.get(id) {
                    r.target = t;
                    r.target_id = id.clone();
                    r.target_set = true;
                    r.exclude(next);
                }
            }
            // A bone chosen by camera.follow.set stays while its target and bone exist.
            match &o.socket {
                Some((t, b)) => {
                    if *t == r.target_id && request_socket(next, t, b).is_ok() {
                        r.socket = o.socket.clone();
                    }
                }
                None => r.socket = None,
            }
        }
        if let Some(id) = &old.activated {
            if old.authored_active == self.authored_active
                && next
                    .compiled
                    .indices
                    .get(id)
                    .is_some_and(|&s| next.compiled.entities[s].camera.is_some())
            {
                set_active(next, id);
                self.activated = Some(id.clone());
                self.active = active_cameras(next);
            }
        }
        Ok(())
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "camera" {
            return None;
        }
        Some(match action {
            "summary" => Ok(self.summary()),
            "inspect" => (|| {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .context("camera.inspect needs {\"id\":\"camera entity\"}")?;
                if let Some(r) = self.rigs.iter().find(|r| r.id == id) {
                    return Ok(r.json(state, &self.names, &self.grid));
                }
                let slot = *state.compiled.indices.get(id).ok_or_else(|| {
                    axiom_core::Diagnostic::error(
                        "not_found",
                        "/args/id",
                        format!("{id} does not exist"),
                    )
                })?;
                let camera = state.compiled.entities[slot]
                    .camera
                    .as_ref()
                    .ok_or_else(|| {
                        axiom_core::Diagnostic::error(
                            "not_a_camera",
                            "/args/id",
                            format!("{id} has no render.camera"),
                        )
                    })?;
                Ok(json!({
                    "id": id,
                    "follow": false,
                    "active": camera.active,
                    "enabled": state.compiled.enabled[slot],
                    "position": state.transforms[slot].as_ref().map(|t| t.position),
                }))
            })(),
            "activate" => (|| {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .context("camera.activate needs {\"id\":\"camera entity\"}")?
                    .to_owned();
                self.activate(state, &id)
            })(),
            "follow.set" => self.set(state, args),
            other => Err(anyhow::anyhow!("unknown camera action {other}")),
        })
    }
    fn metrics(&self) -> std::collections::BTreeMap<String, Value> {
        std::collections::BTreeMap::from([("camera".into(), self.summary())])
    }
    fn uses_screen_pointer(&self) -> bool {
        self.rigs.iter().any(|r| r.def.input.drag)
    }
    fn check_command(
        &self,
        state: &RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<()>> {
        if capability != "camera" {
            return None;
        }
        Some(match action {
            "follow.set" => self.plan_set(state, args).map(drop),
            "activate" => (|| {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .context("camera.activate needs {\"id\":\"camera entity\"}")?;
                let &slot = state
                    .compiled
                    .indices
                    .get(id)
                    .context("camera does not exist")?;
                ensure!(
                    state.compiled.entities[slot].camera.is_some(),
                    "{id} has no render.camera"
                );
                ensure!(
                    state.contexts.is_none(),
                    "this world has input contexts, which own the active cameras"
                );
                ensure!(
                    state.compiled.enabled[slot],
                    "camera {id} is in an inactive scene"
                );
                Ok(())
            })(),
            _ => Ok(()),
        })
    }
    /// `input.drag`: a primary-button drag orbits active drag-enabled follow cameras. Each
    /// move sets the goals from the drag start (absolute), so a run of moves between two
    /// steps acts as its last one, as scenario recordings require.
    fn screen_pointer(&mut self, state: &mut RuntimeState, pointer: &ScreenPointer) -> Result<()> {
        for r in &mut self.rigs {
            let def = &r.def;
            let live = def.input.drag
                && def.enabled
                && state.compiled.enabled[r.slot]
                && state.compiled.entities[r.slot]
                    .camera
                    .as_ref()
                    .is_some_and(|c| c.active);
            match pointer.kind {
                PointerKind::PrimaryDown if live => {
                    r.drag = Some((pointer.screen, r.yaw_goal, r.pitch_goal));
                }
                PointerKind::Move => {
                    if let Some((start, yaw, pitch)) = r.drag.filter(|_| live) {
                        let dx = pointer.screen[0] - start[0];
                        let dy = pointer.screen[1] - start[1];
                        r.yaw_goal = wrap(yaw - dx * def.input.drag_yaw.to_radians());
                        r.pitch_goal = (pitch + dy * def.input.drag_pitch.to_radians())
                            .clamp(def.min_pitch.to_radians(), def.max_pitch.to_radians());
                        r.goals_set = true;
                    }
                }
                PointerKind::PrimaryUp => r.drag = None,
                _ => {}
            }
        }
        Ok(())
    }
}

fn id_arg(v: &Value) -> Result<()> {
    ensure!(
        v.get("id").and_then(Value::as_str).is_some(),
        "needs {{\"id\":\"camera entity ID\"}}"
    );
    Ok(())
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.action(
        "camera.follow.set",
        "camera",
        json!({"id":"follow camera entity ID","target":"optional entity ID to follow (kept across hot reload while camera.follow is unchanged)","bone":"optional bone of the target's skeleton to look at, or null for the pivot (a new target without bone looks at its pivot)","distance":"optional meters within min_distance..max_distance","yaw":"optional degrees","pitch":"optional degrees within min_pitch..max_pitch","effect":"moves the goals; the camera eases there (rotation_smoothing) and the look point trails to a new target (position_smoothing)"}),
        |v| {
            id_arg(v)?;
            for k in ["distance", "yaw", "pitch"] {
                if let Some(n) = v.get(k) {
                    ensure!(
                        n.as_f64().is_some_and(f64::is_finite),
                        "{k} must be a number"
                    );
                }
            }
            if let Some(t) = v.get("target") {
                ensure!(t.is_string(), "target must be an entity ID");
            }
            if let Some(b) = v.get("bone") {
                ensure!(
                    b.is_string() || b.is_null(),
                    "bone must be a bone name or null"
                );
            }
            Ok(())
        },
    );
    r.authoring.action(
        "camera.activate",
        "camera",
        json!({"id":"camera entity ID (render.camera)","effect":"makes it the only active camera at runtime (others inactive); authoring is unchanged (use `camera main ID` for that). Kept across hot reload while the authored active flags are unchanged"}),
        id_arg,
    );
    r.authoring.action(
        "camera.inspect",
        "camera",
        json!({"id":"camera entity ID; follow cameras: target, desired / goal / allowed / actual distance, hit and focus_hit entity, yaw, pitch, look point, position, tests"}),
        id_arg,
    );
    r.authoring
        .action("camera.summary", "camera", json!({}), |_| Ok(()));
    r.factories.insert("camera.rig".into(), |state| {
        let system = CameraRigSystem::new(state)?;
        for rig in &system.rigs {
            if let Some((target, bone)) = &rig.socket {
                request_socket(state, target, bone).map_err(|e| {
                    axiom_core::Diagnostic::error(
                        "invalid_camera",
                        format!(
                            "/entities/@{}/components/camera.follow/target_bone",
                            axiom_core::local::escape(&rig.id)
                        ),
                        format!("{e:#}"),
                    )
                })?;
            }
        }
        Ok(Box::new(system))
    });
}
