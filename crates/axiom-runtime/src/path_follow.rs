//! `path.move`: runtime side of `path.spline` / `path.follow` (see `axiom_core::path`).
//!
//! Every path is compiled once into a flat node table: local-space polyline points with the
//! cumulative distance (the arc-length table), the XZ direction of the piece starting at the
//! node and a corner blend half-width. Each fixed step computes each path's rigid world frame
//! once, then advances every follower by speed x dt, finds its piece by stepping from the
//! previous one (binary search after a jump) and writes the transform. Followers run in
//! parent-before-child order. A steady step does not allocate; `on_arrive` events go through
//! the same gameplay command path as `trigger.zone` events.
use crate::trigger::world_matrix;
use crate::world::*;
use anyhow::{ensure, Context, Result};
use axiom_core::{
    gameplay::{RuntimeCommand, Scalar},
    path::*,
};
use glam::{Quat, Vec2, Vec3};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::BTreeMap,
    f32::consts::{PI, TAU},
};

/// Half-width (meters) over which the direction (and so the lateral offset and the facing)
/// turns from one straight piece to the next at a corner; at most half of either piece.
const CORNER_BLEND: f32 = 1.0;
/// Followers listed with distance/progress in `observe` (counts are always complete).
const OBSERVED: usize = 32;
const OBSERVED_STATES: usize = 16;

#[derive(Clone, Copy)]
struct Node {
    /// Local-space point.
    p: Vec3,
    /// Distance from the first node along the polyline.
    s: f32,
    /// Unit XZ direction (x, z) of the piece starting here; the closing node of a closed
    /// path repeats the first piece's direction.
    dir: Vec2,
    blend: f32,
}

struct Path {
    slot: usize,
    first: usize,
    count: usize,
    length: f32,
    spline: PathSpline,
}

struct Follower {
    slot: usize,
    path: usize,
    /// Piece index within the path's nodes (search hint).
    piece: usize,
    distance: f32,
    forward: bool,
    speed: f32,
    /// `speed` was set by `path.set_speed` (kept across hot reload).
    speed_set: bool,
    /// Start distance after wrapping / folding / clamping, and the travel direction there.
    start: (f32, bool),
    /// Authored start, end mode and paused flag: hot reload keeps progress only when they and
    /// the path are unchanged.
    authored: (f32, Option<f32>, PathEnd, bool),
    end: PathEnd,
    orient: bool,
    /// Radians per fixed step (0 = face the tangent at once).
    turn: f32,
    yaw_offset: f32,
    offset: f32,
    yaw: Option<f32>,
    enabled: bool,
    paused: bool,
    arrived: bool,
    on_arrive: Option<usize>,
}

struct EventInfo {
    name: String,
    follower_field: bool,
    path_field: bool,
}

pub struct PathSystem {
    paths: Vec<Path>,
    path_ids: Vec<String>,
    nodes: Vec<Node>,
    followers: Vec<Follower>,
    follower_ids: Vec<String>,
    /// Rigid world frame (rotation, translation) per path, recomputed each step.
    frames: Vec<(Quat, Vec3)>,
    events: Vec<EventInfo>,
    /// Followers that arrived this step and emit an event (capacity = event budget).
    arrivals: Vec<usize>,
    emitted: u64,
    scanned: usize,
    /// Set when a budget is exceeded or an unsupported runtime spawn appears; the system
    /// stops and observe reports why.
    halted: Option<String>,
    fault: Option<String>,
}

/// Rotation and translation of a world matrix; scale is ignored (distances stay meters).
fn rigid(state: &RuntimeState, slot: usize) -> (Quat, Vec3) {
    let (_, rotation, translation) = world_matrix(state, slot).to_scale_rotation_translation();
    (rotation, translation)
}

/// The piece of `nodes` containing distance `d`, starting the search at `hint`.
fn piece(nodes: &[Node], hint: usize, d: f32) -> usize {
    let last = nodes.len() - 2;
    let inside = |k: usize| nodes[k].s <= d && d <= nodes[k + 1].s;
    let hint = hint.min(last);
    if inside(hint) {
        return hint;
    }
    if hint < last && inside(hint + 1) {
        return hint + 1;
    }
    // First piece whose end reaches `d`.
    nodes[1..].partition_point(|n| n.s < d).min(last)
}

/// Local point and unit XZ direction at distance `d` on piece `k`. Near a node with a blend
/// window the direction mixes the two adjacent pieces (continuous offsets and facing).
fn sample(nodes: &[Node], k: usize, d: f32) -> (Vec3, Vec2) {
    let (a, b) = (nodes[k], nodes[k + 1]);
    let length = b.s - a.s;
    let t = if length > 0.0 {
        ((d - a.s) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut dir = a.dir;
    let (from_start, to_end) = (d - a.s, b.s - d);
    if a.blend > 0.0 && from_start < a.blend {
        // A blend at node 0 exists only on closed paths: the previous piece is the last one.
        let previous = nodes[if k > 0 { k - 1 } else { nodes.len() - 2 }].dir;
        let w = 0.5 + 0.5 * from_start / a.blend;
        dir = previous * (1.0 - w) + dir * w;
    } else if b.blend > 0.0 && to_end < b.blend {
        let w = 0.5 + 0.5 * to_end / b.blend;
        dir = dir * w + b.dir * (1.0 - w);
    }
    (a.p.lerp(b.p, t), dir.normalize_or(a.dir))
}

/// Moves `from` toward `to` (radians) by at most `max`, the short way round; result in -PI..PI.
fn approach(from: f32, to: f32, max: f32) -> f32 {
    let delta = (to - from + PI).rem_euclid(TAU) - PI;
    (from + delta.clamp(-max, max) + PI).rem_euclid(TAU) - PI
}

/// Start distance on a path of `length` normalized for the end mode, and the direction.
fn start_state(distance: f32, length: f32, end: PathEnd) -> (f32, bool) {
    match end {
        PathEnd::Loop => (distance.rem_euclid(length), true),
        PathEnd::PingPong => {
            let u = distance.rem_euclid(2.0 * length);
            if u <= length {
                (u, true)
            } else {
                (2.0 * length - u, false)
            }
        }
        PathEnd::Stop => (distance.min(length), true),
    }
}

impl Follower {
    /// Advances by `travel` meters; true when a stop follower reaches the end in this call.
    fn advance(&mut self, travel: f32, length: f32) -> bool {
        match self.end {
            PathEnd::Loop => self.distance = (self.distance + travel).rem_euclid(length),
            PathEnd::PingPong => {
                let u = if self.forward {
                    self.distance
                } else {
                    2.0 * length - self.distance
                };
                (self.distance, self.forward) = start_state(u + travel, length, PathEnd::PingPong);
            }
            PathEnd::Stop => {
                self.distance = (self.distance + travel).min(length);
                if self.distance >= length {
                    self.arrived = true;
                    return true;
                }
            }
        }
        false
    }

    /// Back to the start. A stop follower that starts at the end has arrived (no event: it
    /// never moved there).
    fn restart(&mut self, length: f32) {
        (self.distance, self.forward) = self.start;
        self.arrived = self.end == PathEnd::Stop && self.distance >= length;
        self.piece = 0;
    }
}

impl PathSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let compiled = &state.compiled;
        let program = compiled.source.resources.get("behavior.program");
        let mut system = Self {
            paths: vec![],
            path_ids: vec![],
            nodes: vec![],
            followers: vec![],
            follower_ids: vec![],
            frames: vec![],
            events: vec![],
            arrivals: Vec::with_capacity(MAX_PATH_EVENTS_PER_STEP),
            emitted: 0,
            scanned: compiled.len(),
            halted: None,
            fault: None,
        };
        let mut points = vec![];
        let mut path_index = BTreeMap::new();
        for (slot, e) in compiled.entities.iter().enumerate() {
            let Some(spline) = &e.path else { continue };
            spline
                .validate()
                .with_context(|| format!("path {}", e.id))?;
            ensure!(
                system.paths.len() < MAX_PATHS,
                "path budget exceeded (> {MAX_PATHS})"
            );
            spline.polyline(&mut points);
            ensure!(
                system.nodes.len() + points.len() <= MAX_PATH_NODES,
                "path node budget exceeded (> {MAX_PATH_NODES})"
            );
            let first = system.nodes.len();
            let mut s = 0.0;
            for (i, p) in points.iter().enumerate() {
                if i > 0 {
                    s += p.distance(points[i - 1]);
                }
                system.nodes.push(Node {
                    p: *p,
                    s,
                    dir: Vec2::Y,
                    blend: 0.0,
                });
            }
            let count = points.len();
            let nodes = &mut system.nodes[first..];
            let pieces = count - 1;
            let xz = |k: usize, nodes: &[Node]| {
                let d = nodes[k + 1].p - nodes[k].p;
                Some(Vec2::new(d.x, d.z)).filter(|v| v.length_squared() > 1e-10)
            };
            // Vertical pieces keep the previous direction (the first one: the first non-vertical).
            let mut dir = (0..pieces)
                .find_map(|k| xz(k, nodes))
                .map_or(Vec2::Y, Vec2::normalize);
            for k in 0..pieces {
                if let Some(v) = xz(k, nodes) {
                    dir = v.normalize();
                }
                nodes[k].dir = dir;
            }
            nodes[pieces].dir = if spline.closed { nodes[0].dir } else { dir };
            let length = |k: usize, nodes: &[Node]| nodes[k + 1].s - nodes[k].s;
            for k in 1..pieces {
                nodes[k].blend = CORNER_BLEND
                    .min(0.5 * length(k - 1, nodes))
                    .min(0.5 * length(k, nodes));
            }
            if spline.closed {
                let blend = CORNER_BLEND
                    .min(0.5 * length(pieces - 1, nodes))
                    .min(0.5 * length(0, nodes));
                nodes[0].blend = blend;
                nodes[pieces].blend = blend;
            }
            path_index.insert(e.id.as_str(), system.paths.len());
            system.paths.push(Path {
                slot,
                first,
                count,
                length: s,
                spline: spline.clone(),
            });
            system.path_ids.push(e.id.clone());
        }
        system.frames = vec![(Quat::IDENTITY, Vec3::ZERO); system.paths.len()];
        // Parents before children, so a follower under another follower sees its parent's
        // position of this step.
        let mut order = vec![0; compiled.len()];
        for (rank, &slot) in compiled.transform_order.iter().enumerate() {
            order[slot] = rank;
        }
        let mut slots: Vec<usize> = (0..compiled.len())
            .filter(|&i| compiled.entities[i].follow.is_some())
            .collect();
        slots.sort_by_key(|&i| order[i]);
        ensure!(
            slots.len() <= MAX_FOLLOWERS,
            "path follower budget exceeded ({} > {MAX_FOLLOWERS})",
            slots.len()
        );
        for slot in slots {
            let e = &compiled.entities[slot];
            let def = e.follow.as_ref().unwrap();
            ensure!(
                state.transforms[slot].is_some(),
                "path follower {} needs core.transform",
                e.id
            );
            let path = *path_index.get(def.path.as_str()).with_context(|| {
                format!("{} follows {}, which has no path.spline", e.id, def.path)
            })?;
            let on_arrive = if def.on_arrive.is_empty() {
                None
            } else if let Some(i) = system.events.iter().position(|v| v.name == def.on_arrive) {
                Some(i)
            } else {
                let fields = program
                    .and_then(|p| {
                        p.pointer(&format!(
                            "/events/{}/fields",
                            axiom_core::local::escape(&def.on_arrive)
                        ))
                    })
                    .and_then(Value::as_object)
                    .with_context(|| {
                        format!(
                            "path follower emits {}, which behavior.program does not declare",
                            def.on_arrive
                        )
                    })?;
                system.events.push(EventInfo {
                    name: def.on_arrive.clone(),
                    follower_field: fields.contains_key("follower"),
                    path_field: fields.contains_key("path"),
                });
                Some(system.events.len() - 1)
            };
            let length = system.paths[path].length;
            let start = start_state(def.start_distance(length), length, def.end);
            let mut follower = Follower {
                slot,
                path,
                piece: 0,
                distance: 0.0,
                forward: true,
                speed: def.speed,
                speed_set: false,
                start,
                authored: (def.start, def.start_fraction, def.end, def.paused),
                end: def.end,
                orient: def.orient,
                turn: def.turn_rate.to_radians() * FIXED_DT,
                yaw_offset: def.yaw_offset.to_radians(),
                offset: def.offset,
                yaw: None,
                enabled: def.enabled,
                paused: def.paused,
                arrived: false,
                on_arrive,
            };
            follower.restart(length);
            system.followers.push(follower);
            system.follower_ids.push(e.id.clone());
        }
        if !system.events.is_empty() {
            ensure!(
                compiled
                    .source
                    .systems
                    .iter()
                    .any(|s| s == "behavior.execute"),
                "path on_arrive events need the behavior.execute system"
            );
        }
        Ok(system)
    }

    fn place(&mut self, state: &mut RuntimeState, i: usize) {
        let f = &mut self.followers[i];
        let path = &self.paths[f.path];
        let nodes = &self.nodes[path.first..path.first + path.count];
        let (rotation, translation) = self.frames[f.path];
        f.piece = piece(nodes, f.piece, f.distance);
        let (local, dir) = sample(nodes, f.piece, f.distance);
        // Right of the path direction (models look along +Z, so right is forward x up).
        let right = Vec3::new(-dir.y, 0.0, dir.x);
        let world = rotation * (local + right * f.offset) + translation;
        if f.orient {
            let sign = if f.forward { 1.0 } else { -1.0 };
            let tangent = rotation * Vec3::new(dir.x, 0.0, dir.y) * sign;
            if tangent.x * tangent.x + tangent.z * tangent.z > 1e-12 {
                let target = tangent.x.atan2(tangent.z) + f.yaw_offset;
                f.yaw = Some(match f.yaw {
                    Some(yaw) if f.turn > 0.0 => approach(yaw, target, f.turn),
                    _ => approach(target, target, 0.0),
                });
            }
        }
        let facing = f.yaw.filter(|_| f.orient).map(Quat::from_rotation_y);
        let slot = f.slot;
        let (position, facing) = match state.compiled.parents[slot] {
            None => (world, facing),
            Some(parent) => {
                let m = world_matrix(state, parent);
                let (_, parent_rotation, _) = m.to_scale_rotation_translation();
                (
                    m.inverse().transform_point3(world),
                    facing.map(|q| parent_rotation.inverse() * q),
                )
            }
        };
        if let Some(t) = &mut state.transforms[slot] {
            t.position = position.to_array();
            if let Some(q) = facing {
                t.rotation = q.normalize().to_array();
            }
        }
    }

    fn emit(&mut self, state: &mut RuntimeState, i: usize) {
        let f = &self.followers[i];
        let Some(event) = f.on_arrive else { return };
        let info = &self.events[event];
        let mut fields = BTreeMap::new();
        if info.follower_field {
            fields.insert(
                "follower".to_owned(),
                Scalar::Text(self.follower_ids[i].clone()),
            );
        }
        if info.path_field {
            fields.insert(
                "path".to_owned(),
                Scalar::Text(self.path_ids[f.path].clone()),
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
            Ok(()) => self.emitted += 1,
            // Keep moving; surface the first failure through observe.
            Err(e) => {
                let name = &self.events[event].name;
                self.fault
                    .get_or_insert_with(|| format!("emit {name}: {e:#}"));
            }
        }
    }

    /// Local-space polyline nodes of the path on entity slot `slot` (editor gizmos).
    pub fn polyline(&self, slot: usize) -> Option<impl ExactSizeIterator<Item = Vec3> + '_> {
        let path = self.paths.iter().find(|p| p.slot == slot)?;
        Some(
            self.nodes[path.first..path.first + path.count]
                .iter()
                .map(|n| n.p),
        )
    }

    /// World position where the follower on entity slot `slot` starts (editor gizmos),
    /// computed from the current transforms.
    pub fn start_point(&self, state: &RuntimeState, slot: usize) -> Option<Vec3> {
        let f = self.followers.iter().find(|f| f.slot == slot)?;
        let path = &self.paths[f.path];
        let nodes = &self.nodes[path.first..path.first + path.count];
        let (rotation, translation) = rigid(state, path.slot);
        let k = piece(nodes, 0, f.start.0);
        let (local, dir) = sample(nodes, k, f.start.0);
        Some(rotation * (local + Vec3::new(-dir.y, 0.0, dir.x) * f.offset) + translation)
    }

    fn find_follower(&self, args: &Value) -> Result<usize> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .context("path actions need {\"id\":\"follower entity\"}")?;
        self.follower_ids
            .iter()
            .position(|x| x == id)
            .ok_or_else(|| {
                axiom_core::Diagnostic::error(
                    "not_found",
                    "/args/id",
                    format!("{id} is not a path follower"),
                )
            })
    }

    fn progress(&self, f: &Follower) -> f32 {
        let length = self.paths[f.path].length;
        f.distance / length
    }

    fn follower_json(&self, i: usize) -> Value {
        let f = &self.followers[i];
        json!({
            "id":self.follower_ids[i],
            "path":self.path_ids[f.path],
            "distance":round(f.distance, 1000.0),
            "length":round(self.paths[f.path].length, 1000.0),
            "progress":round(self.progress(f), 10000.0),
            "forward":f.forward,
            "speed":f.speed,
            "end":f.end,
            "enabled":f.enabled,
            "paused":f.paused,
            "arrived":f.arrived,
        })
    }

    fn summary(&self) -> Value {
        let at: serde_json::Map<String, Value> = self
            .followers
            .iter()
            .zip(&self.follower_ids)
            .take(OBSERVED)
            .map(|(f, id)| {
                (
                    id.clone(),
                    json!([round(f.distance, 100.0), round(self.progress(f), 1000.0)]),
                )
            })
            .collect();
        let listed = |keep: fn(&Follower) -> bool| -> Vec<&str> {
            self.followers
                .iter()
                .zip(&self.follower_ids)
                .filter(|(f, _)| keep(f))
                .take(OBSERVED_STATES)
                .map(|(_, id)| id.as_str())
                .collect()
        };
        json!({
            "paths":self.paths.len(),
            "followers":self.followers.len(),
            "moving":self.followers.iter().filter(|f| f.enabled && !f.paused && !f.arrived && f.speed > 0.0).count(),
            "paused":listed(|f| f.enabled && f.paused),
            "arrived":listed(|f| f.arrived),
            "at":at,
            "more":self.followers.len().saturating_sub(OBSERVED),
            "emitted":self.emitted,
            "halted":self.halted,
            "fault":self.fault,
        })
    }
}

/// Display rounding for observe (f64 so the JSON shows the short decimal).
fn round(v: f32, scale: f64) -> f64 {
    (f64::from(v) * scale).round() / scale
}

impl RuntimeSystem for PathSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, _input: &ActionMap) {
        if self.halted.is_some() {
            return;
        }
        let len = state.compiled.len();
        if len > self.scanned {
            if let Some(e) = state.compiled.entities[self.scanned..]
                .iter()
                .find(|e| e.path.is_some() || e.follow.is_some())
            {
                self.halted = Some(format!(
                    "runtime-spawned entity {} has path.spline or path.follow; spawned paths and followers are not supported (author them instead)",
                    e.id
                ));
                return;
            }
            self.scanned = len;
        }
        if self.followers.is_empty() {
            return;
        }
        for (frame, path) in self.frames.iter_mut().zip(&self.paths) {
            *frame = rigid(state, path.slot);
        }
        self.arrivals.clear();
        let mut events = 0;
        for i in 0..self.followers.len() {
            let f = &mut self.followers[i];
            if !f.enabled || !state.compiled.enabled[f.slot] {
                continue;
            }
            if !f.paused && !f.arrived && f.speed > 0.0 {
                let length = self.paths[f.path].length;
                if f.advance(f.speed * FIXED_DT, length) && f.on_arrive.is_some() {
                    events += 1;
                    if self.arrivals.len() < self.arrivals.capacity() {
                        self.arrivals.push(i);
                    }
                }
            }
            self.place(state, i);
        }
        if events == 0 {
            return;
        }
        if events > MAX_PATH_EVENTS_PER_STEP {
            self.halted = Some(format!(
                "path event budget exceeded ({events} > {MAX_PATH_EVENTS_PER_STEP} on_arrive events in one step at tick {})",
                state.tick
            ));
            return;
        }
        for k in 0..self.arrivals.len() {
            self.emit(state, self.arrivals[k]);
        }
        state.sync_scenes();
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        _next: &mut RuntimeState,
    ) -> Result<()> {
        // Hot reload keeps progress per follower entity ID when the follower still follows
        // the same path ID with the same spline and the same start / end mode / paused flag;
        // otherwise the follower restarts. Runtime speed overrides are kept with the progress.
        let Some(old) = old.as_any().downcast_ref::<Self>() else {
            return Ok(());
        };
        let previous: BTreeMap<&str, usize> = old
            .follower_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        for (f, id) in self.followers.iter_mut().zip(&self.follower_ids) {
            let Some(&o) = previous.get(id.as_str()) else {
                continue;
            };
            let of = &old.followers[o];
            if old.path_ids[of.path] != self.path_ids[f.path]
                || old.paths[of.path].spline != self.paths[f.path].spline
                || of.authored != f.authored
            {
                continue;
            }
            f.distance = of.distance;
            f.forward = of.forward;
            f.piece = of.piece;
            f.paused = of.paused;
            f.arrived = of.arrived;
            f.yaw = of.yaw;
            if of.speed_set {
                f.speed = of.speed;
                f.speed_set = true;
            }
        }
        self.emitted = old.emitted;
        self.fault = old.fault.clone();
        Ok(())
    }
    fn command(
        &mut self,
        _state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "path" {
            return None;
        }
        Some(match action {
            "summary" => Ok(self.summary()),
            "inspect" => (|| {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .context("path.inspect needs {\"id\":\"follower or path entity\"}")?;
                if let Some(p) = self.path_ids.iter().position(|x| x == id) {
                    let path = &self.paths[p];
                    return Ok(json!({
                        "id":id,
                        "length":round(path.length, 1000.0),
                        "closed":path.spline.closed,
                        "points":path.spline.points.len(),
                        "nodes":path.count,
                        "followers":self.followers.iter().filter(|f| f.path == p).count(),
                    }));
                }
                let i = self.find_follower(args)?;
                Ok(self.follower_json(i))
            })(),
            "pause" | "resume" | "set_speed" | "restart" => (|| {
                let i = self.find_follower(args)?;
                let speed = if action == "set_speed" {
                    let speed = args
                        .get("speed")
                        .and_then(Value::as_f64)
                        .context("path.set_speed needs {\"id\":..,\"speed\":m/s}")?
                        as f32;
                    ensure!(
                        speed.is_finite() && (0.0..=100.0).contains(&speed),
                        "speed must be 0..100 m/s"
                    );
                    Some(speed)
                } else {
                    None
                };
                let length = self.paths[self.followers[i].path].length;
                let f = &mut self.followers[i];
                ensure!(
                    f.enabled,
                    "{} is a disabled follower (enabled false)",
                    self.follower_ids[i]
                );
                match action {
                    "pause" => f.paused = true,
                    "resume" => f.paused = false,
                    "restart" => f.restart(length),
                    _ => {
                        f.speed = speed.unwrap();
                        f.speed_set = true;
                    }
                }
                Ok(self.follower_json(i))
            })(),
            other => Err(anyhow::anyhow!("unknown path action {other}")),
        })
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("path".into(), self.summary())])
    }
}

fn id_arg(v: &Value) -> Result<()> {
    ensure!(
        v.get("id").and_then(Value::as_str).is_some(),
        "needs {{\"id\":\"follower entity ID\"}}"
    );
    Ok(())
}

pub fn register(r: &mut RuntimeRegistry) {
    let follower = "follower entity ID";
    r.authoring.action(
        "path.pause",
        "path",
        json!({"id":follower,"effect":"holds the distance; the follower still follows a moving path"}),
        id_arg,
    );
    r.authoring
        .action("path.resume", "path", json!({ "id": follower }), id_arg);
    r.authoring.action(
        "path.restart",
        "path",
        json!({"id":follower,"effect":"back to the authored start; clears arrived (pause state kept)"}),
        id_arg,
    );
    r.authoring.action(
        "path.set_speed",
        "path",
        json!({"id":follower,"speed":"m/s 0..100 (kept across hot reload while the path is unchanged)"}),
        |v| {
            id_arg(v)?;
            ensure!(
                v.get("speed")
                    .and_then(Value::as_f64)
                    .is_some_and(|s| (0.0..=100.0).contains(&s)),
                "path.set_speed needs speed 0..100 m/s"
            );
            Ok(())
        },
    );
    r.authoring.action(
        "path.inspect",
        "path",
        json!({"id":"follower or path entity ID; follower: distance, length, progress, speed, state; path: length, nodes, followers"}),
        id_arg,
    );
    r.authoring
        .action("path.summary", "path", json!({}), |_| Ok(()));
    r.factories.insert("path.move".into(), |state| {
        Ok(Box::new(PathSystem::new(state)?))
    });
}
