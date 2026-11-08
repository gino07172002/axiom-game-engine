//! Paths (`path.spline`) and path following (`path.follow`), executed by the runtime system
//! `path.move`.
//!
//! A path is an ordered list of points in the local space of its entity: moving or rotating
//! the entity (parents included) moves the path. Transform scale is ignored (like
//! `trigger.zone`), so distances along the path are always meters. Catmull-Rom paths are
//! sampled into a polyline once (`PathSpline::polyline`); followers move along that polyline at
//! constant speed, so the arc-length table is built when the runtime is built, never per step.
use anyhow::{bail, ensure, Context, Result};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Entities with `path.spline` per world.
pub const MAX_PATHS: usize = 256;
/// Control points per path.
pub const MAX_PATH_POINTS: usize = 256;
/// Entities with `path.follow` per world.
pub const MAX_FOLLOWERS: usize = 2048;
/// Polyline nodes across all paths after Catmull-Rom sampling: bounds the runtime tables
/// (28 bytes per node).
pub const MAX_PATH_NODES: usize = 65536;
/// `on_arrive` events one fixed step may emit across all followers.
pub const MAX_PATH_EVENTS_PER_STEP: usize = 64;
/// Polyline pieces per Catmull-Rom segment.
pub const CATMULL_ROM_SAMPLES: usize = 8;
/// Minimum distance between consecutive points (meters).
pub const MIN_POINT_SPACING: f32 = 0.01;
const MAX_COORDINATE: f32 = 100_000.0;
const MAX_SPEED: f32 = 100.0;
const MAX_OFFSET: f32 = 50.0;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathInterpolation {
    /// Straight segments between the points.
    #[default]
    Linear,
    /// Centripetal Catmull-Rom through every point, sampled into a polyline.
    CatmullRom,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PathSpline {
    /// Points in the entity's local space, in travel order.
    pub points: Vec<[f32; 3]>,
    /// Loop back from the last point to the first (do not repeat the first point).
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub interpolation: PathInterpolation,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathEnd {
    /// Open path: jump back to the start. Closed path: keep going around.
    #[default]
    Loop,
    /// Reverse at each end.
    PingPong,
    /// Stay at the end and emit `on_arrive`.
    Stop,
}

fn yes() -> bool {
    true
}

fn default_speed() -> f32 {
    1.5
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PathFollow {
    /// Entity ID of the path (an entity with `path.spline`).
    pub path: String,
    /// Meters per second along the path.
    #[serde(default = "default_speed")]
    pub speed: f32,
    /// Start distance along the path in meters (wraps / folds / clamps like the end mode).
    #[serde(default)]
    pub start: f32,
    /// Start as a fraction 0..1 of the path length (instead of `start`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_fraction: Option<f32>,
    #[serde(default)]
    pub end: PathEnd,
    /// Turn the entity (yaw only) to face along the path; models look along +Z.
    #[serde(default = "yes")]
    pub orient: bool,
    /// Maximum turn speed in degrees per second while orienting (0 = face the tangent at once).
    #[serde(default)]
    pub turn_rate: f32,
    /// Added to the facing yaw, degrees (for models that do not look along +Z).
    #[serde(default)]
    pub yaw_offset: f32,
    /// Lateral offset in meters to the right of the path direction (negative = left), in the
    /// path's local XZ plane. Lets several followers share one path side by side.
    #[serde(default)]
    pub offset: f32,
    /// `behavior.program` event emitted when a `stop` follower reaches the end ("" = none).
    #[serde(default)]
    pub on_arrive: String,
    /// Start paused (resume with the `path.resume` action).
    #[serde(default)]
    pub paused: bool,
    /// A disabled follower never moves or writes the transform.
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl PathSpline {
    /// The editor preset: an 8 x 6 m closed rectangle on the local XZ plane.
    pub fn preset() -> Self {
        Self {
            points: vec![
                [-4.0, 0.0, -3.0],
                [4.0, 0.0, -3.0],
                [4.0, 0.0, 3.0],
                [-4.0, 0.0, 3.0],
            ],
            closed: true,
            interpolation: PathInterpolation::Linear,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let n = self.points.len();
        ensure!(
            (2..=MAX_PATH_POINTS).contains(&n),
            "a path needs 2..{MAX_PATH_POINTS} points (has {n})"
        );
        ensure!(
            !self.closed || n >= 3,
            "a closed path needs at least 3 points"
        );
        for (i, p) in self.points.iter().enumerate() {
            ensure!(
                p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE),
                "point {i} must be finite and within ±{MAX_COORDINATE} m"
            );
        }
        for i in 0..self.segments() {
            let j = (i + 1) % n;
            let d = Vec3::from_array(self.points[i]).distance(Vec3::from_array(self.points[j]));
            if d < MIN_POINT_SPACING {
                if j == 0 {
                    bail!("the last point repeats the first one; a closed path joins them itself (remove the last point)");
                }
                bail!("points {i} and {j} are closer than {MIN_POINT_SPACING} m; remove one");
            }
        }
        Ok(())
    }

    /// Segments between control points (closed paths add the closing one).
    pub fn segments(&self) -> usize {
        self.points.len() - 1 + usize::from(self.closed)
    }

    /// Nodes of `polyline`: what the runtime stores for this path.
    pub fn node_count(&self) -> usize {
        match self.interpolation {
            PathInterpolation::Linear => self.segments() + 1,
            PathInterpolation::CatmullRom => self.segments() * CATMULL_ROM_SAMPLES + 1,
        }
    }

    /// The local-space polyline followers travel along: the points themselves (linear) or
    /// the sampled centripetal Catmull-Rom curve. A closed path repeats its first node at
    /// the end, so node `k` to `k + 1` is always one piece. Needs a valid spline.
    pub fn polyline(&self, out: &mut Vec<Vec3>) {
        out.clear();
        let n = self.points.len();
        let p = |i: usize| Vec3::from_array(self.points[i % n]);
        match self.interpolation {
            PathInterpolation::Linear => {
                out.extend((0..n).map(p));
                if self.closed {
                    out.push(p(0));
                }
            }
            PathInterpolation::CatmullRom => {
                for i in 0..self.segments() {
                    let (p1, p2) = (p(i), p(i + 1));
                    // Neighbors: wrap on closed paths, mirror the end segments on open ones.
                    let p0 = if self.closed || i > 0 {
                        p(i + n - 1)
                    } else {
                        2.0 * p1 - p2
                    };
                    let p3 = if self.closed || i + 2 < n {
                        p(i + 2)
                    } else {
                        2.0 * p2 - p1
                    };
                    for k in 0..CATMULL_ROM_SAMPLES {
                        out.push(centripetal(
                            [p0, p1, p2, p3],
                            k as f32 / CATMULL_ROM_SAMPLES as f32,
                        ));
                    }
                }
                out.push(if self.closed { p(0) } else { p(n - 1) });
            }
        }
    }
}

/// Centripetal Catmull-Rom (alpha 0.5, Barry-Goldman) between `p[1]` and `p[2]`, `u` in 0..1.
/// Consecutive control points are at least `MIN_POINT_SPACING` apart, so no knot interval is 0.
fn centripetal(p: [Vec3; 4], u: f32) -> Vec3 {
    let knot = |a: Vec3, b: Vec3| a.distance(b).sqrt();
    let t0 = 0.0;
    let t1 = t0 + knot(p[0], p[1]);
    let t2 = t1 + knot(p[1], p[2]);
    let t3 = t2 + knot(p[2], p[3]);
    let t = t1 + (t2 - t1) * u;
    let mix = |a: Vec3, b: Vec3, ta: f32, tb: f32| {
        a * ((tb - t) / (tb - ta)) + b * ((t - ta) / (tb - ta))
    };
    let a1 = mix(p[0], p[1], t0, t1);
    let a2 = mix(p[1], p[2], t1, t2);
    let a3 = mix(p[2], p[3], t2, t3);
    let b1 = mix(a1, a2, t0, t2);
    let b2 = mix(a2, a3, t1, t3);
    mix(b1, b2, t1, t2)
}

fn event_ok(id: &str) -> Result<()> {
    if !id.is_empty() {
        crate::valid_id(id)?;
    }
    Ok(())
}

impl PathFollow {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            speed: default_speed(),
            start: 0.0,
            start_fraction: None,
            end: PathEnd::Loop,
            orient: true,
            turn_rate: 0.0,
            yaw_offset: 0.0,
            offset: 0.0,
            on_arrive: String::new(),
            paused: false,
            enabled: true,
        }
    }

    pub fn validate(&self) -> Result<()> {
        crate::valid_id(&self.path).context("path")?;
        ensure!(
            self.speed.is_finite() && (0.0..=MAX_SPEED).contains(&self.speed),
            "speed must be 0..{MAX_SPEED} m/s"
        );
        ensure!(
            self.start.is_finite() && (0.0..=1_000_000.0).contains(&self.start),
            "start must be 0..1000000 meters"
        );
        if let Some(f) = self.start_fraction {
            ensure!(
                f.is_finite() && (0.0..=1.0).contains(&f),
                "start_fraction must be 0..1"
            );
            ensure!(
                self.start == 0.0,
                "use either start (meters) or start_fraction, not both"
            );
        }
        ensure!(
            self.turn_rate.is_finite() && (0.0..=3600.0).contains(&self.turn_rate),
            "turn_rate must be 0..3600 degrees per second (0 = instant)"
        );
        ensure!(
            self.yaw_offset.is_finite() && (-360.0..=360.0).contains(&self.yaw_offset),
            "yaw_offset must be -360..360 degrees"
        );
        ensure!(
            self.offset.is_finite() && self.offset.abs() <= MAX_OFFSET,
            "offset must be -{MAX_OFFSET}..{MAX_OFFSET} meters"
        );
        event_ok(&self.on_arrive).context("on_arrive")?;
        ensure!(
            self.on_arrive.is_empty() || self.end == PathEnd::Stop,
            "on_arrive only fires with end stop (loop and ping_pong never arrive)"
        );
        Ok(())
    }

    /// Start distance on a path of `length` meters, before wrapping.
    pub fn start_distance(&self, length: f32) -> f32 {
        self.start_fraction.map_or(self.start, |f| f * length)
    }
}

pub fn spline_schema() -> Value {
    json!({
        "points": format!("2..{MAX_PATH_POINTS} [x,y,z] in the entity's local space, travel order; consecutive points at least {MIN_POINT_SPACING} m apart. Placed by the entity's world position and rotation (parents included); scale is ignored"),
        "closed": "bool, default false: join the last point back to the first (do not repeat it); needs 3+ points",
        "interpolation": format!("linear (default) | catmull_rom (centripetal, through every point, {CATMULL_ROM_SAMPLES} pieces per segment)"),
        "budget": format!("{MAX_PATHS} paths per world, {MAX_PATH_NODES} polyline nodes across all paths (linear: points (+1 closed); catmull_rom: segments x {CATMULL_ROM_SAMPLES} + 1)"),
        "edit": "ACL: path ID add [x,y,z] | path ID insert I [x,y,z] | path ID move I x,y,z | path ID remove I (local coordinates; without coordinates add/insert extend the path or split a segment)"
    })
}

pub fn follow_schema() -> Value {
    json!({
        "path": "entity ID with path.spline (not this entity or one of its descendants)",
        "speed": format!("m/s 0..{MAX_SPEED}, default 1.5"),
        "start": "meters >= 0 along the path, default 0 (wrapped for loop, folded for ping_pong, clamped for stop)",
        "start_fraction": "optional 0..1 of the path length instead of start",
        "end": "loop (default; open paths jump back to the start) | ping_pong | stop",
        "orient": "bool, default true: yaw to face along the path (models look along +Z)",
        "turn_rate": "degrees/second 0..3600, default 0 = face the tangent at once",
        "yaw_offset": "degrees -360..360 added to the facing yaw",
        "offset": format!("meters -{MAX_OFFSET}..{MAX_OFFSET}: lateral offset to the right of the path direction in the path's local XZ plane (corners are blended over up to 1 m)"),
        "on_arrive": "behavior.program event emitted once when a stop follower reaches the end (\"\" = none); may declare text fields `follower` and/or `path`; needs end stop",
        "paused": "bool, default false: start paused (path.resume continues)",
        "enabled": "bool, default true: a disabled follower never moves the entity",
        "conflict": "an entity with path.follow must not have another mover component (core.motion, fsm.agent, rts.unit, action.actor, fps.*): authoring error path_conflict",
        "requires": "system path.move (capability path); on_arrive also needs behavior.execute",
        "actions": "path.pause|resume {id}, path.set_speed {id,speed}, path.restart {id}, path.inspect {id}, path.summary",
        "budget": format!("{MAX_FOLLOWERS} followers per world, {MAX_PATH_EVENTS_PER_STEP} on_arrive events per step"),
        "checkpoint": "not supported: worlds scheduling path.move refuse checkpoints explicitly"
    })
}

/// World-level checks run by `World::compile` on resolved components: budgets, the required
/// systems, references, mover conflicts and `on_arrive` payload contracts.
pub fn validate_world(
    world: &crate::World,
    registry: &crate::Registry,
    entities: &[crate::CompiledEntity],
    indices: &std::collections::BTreeMap<String, usize>,
    parents: &[Option<usize>],
) -> Result<()> {
    let paths: Vec<&PathSpline> = entities.iter().filter_map(|e| e.path.as_ref()).collect();
    let followers = entities.iter().filter(|e| e.follow.is_some()).count();
    if paths.is_empty() && followers == 0 {
        return Ok(());
    }
    let budget = |message: String| crate::Diagnostic::error("path_budget", "/entities", message);
    if paths.len() > MAX_PATHS {
        return Err(budget(format!(
            "path budget exceeded ({} > {MAX_PATHS})",
            paths.len()
        )));
    }
    if followers > MAX_FOLLOWERS {
        return Err(budget(format!(
            "path follower budget exceeded ({followers} > {MAX_FOLLOWERS})"
        )));
    }
    let nodes: usize = paths.iter().map(|p| p.node_count()).sum();
    if nodes > MAX_PATH_NODES {
        return Err(budget(format!(
            "path node budget exceeded ({nodes} > {MAX_PATH_NODES} polyline nodes; catmull_rom uses {CATMULL_ROM_SAMPLES} per segment)"
        )));
    }
    let at = |id: &str, field: &str| {
        format!(
            "/entities/@{}/components/path.follow{field}",
            crate::local::escape(id)
        )
    };
    let has_system = |s: &str| world.systems.iter().any(|x| x == s);
    let program = world.resources.get("behavior.program");
    for (slot, e) in entities.iter().enumerate() {
        let Some(follow) = &e.follow else { continue };
        let invalid = |field: &str, message: String| {
            crate::Diagnostic::error("invalid_path", at(&e.id, field), message)
        };
        if !has_system("path.move") {
            return Err(invalid(
                "",
                "path.follow needs the path.move system (add it to systems)".into(),
            ));
        }
        let target = indices
            .get(&follow.path)
            .copied()
            .filter(|&t| entities[t].path.is_some())
            .ok_or_else(|| {
                invalid(
                    "/path",
                    format!("{} is not an entity with path.spline", follow.path),
                )
            })?;
        let mut cursor = Some(target);
        while let Some(i) = cursor {
            if i == slot {
                return Err(invalid(
                    "/path",
                    format!(
                        "path {} is {} itself or one of its descendants; the follower would move its own path",
                        follow.path, e.id
                    ),
                ));
            }
            cursor = parents[i];
        }
        if let Some((component, system)) = e
            .components
            .keys()
            .find_map(|c| registry.movers.get_key_value(c))
        {
            return Err(crate::Diagnostic::error(
                "path_conflict",
                at(&e.id, ""),
                format!(
                    "{} has path.follow and {component} (moved by {system}); an entity takes one mover. Put the follower on a parent or child entity instead",
                    e.id
                ),
            ));
        }
        if !follow.on_arrive.is_empty() {
            if !has_system("behavior.execute") {
                return Err(invalid(
                    "/on_arrive",
                    "on_arrive events need the behavior.execute system".into(),
                ));
            }
            let event = &follow.on_arrive;
            let fields = program
                .and_then(|p| p.pointer(&format!("/events/{}/fields", crate::local::escape(event))))
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    invalid(
                        "/on_arrive",
                        format!("follower emits {event}, which behavior.program does not declare"),
                    )
                })?;
            if !fields
                .iter()
                .all(|(k, t)| (k == "follower" || k == "path") && t == "text")
            {
                return Err(invalid(
                    "/on_arrive",
                    format!(
                        "path event {event} may only declare text fields `follower` and `path`"
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spline(v: Value) -> Result<PathSpline> {
        let s: PathSpline = serde_json::from_value(v)?;
        s.validate()?;
        Ok(s)
    }
    fn follow(v: Value) -> Result<PathFollow> {
        let f: PathFollow = serde_json::from_value(v)?;
        f.validate()?;
        Ok(f)
    }
    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        format!("{:#}", r.unwrap_err())
    }

    #[test]
    fn defaults_and_round_trip() {
        let s = spline(json!({"points":[[0,0,0],[1,0,0]]})).unwrap();
        assert!(!s.closed && s.interpolation == PathInterpolation::Linear);
        assert_eq!(s.node_count(), 2);
        let f = follow(json!({"path":"road"})).unwrap();
        assert_eq!(f, PathFollow::new("road"));
        assert_eq!(
            serde_json::to_value(&f).unwrap(),
            json!({"path":"road","speed":1.5,"start":0.0,"end":"loop","orient":true,"turn_rate":0.0,"yaw_offset":0.0,"offset":0.0,"on_arrive":"","paused":false,"enabled":true})
        );
        PathSpline::preset().validate().unwrap();
    }

    #[test]
    fn invalid_values_are_rejected_with_reasons() {
        assert!(err(spline(json!({"points":[[0,0,0]]}))).contains("2..256 points"));
        assert!(
            err(spline(json!({"points":[[0,0,0],[1,0,0]],"closed":true}))).contains("3 points")
        );
        assert!(
            err(spline(json!({"points":[[0,0,0],[0,0,0.001],[1,0,0]]}))).contains("points 0 and 1")
        );
        assert!(err(spline(
            json!({"points":[[0,0,0],[1,0,0],[1,0,1],[0,0,0]],"closed":true})
        ))
        .contains("repeats the first"));
        assert!(err(spline(json!({"points":[[0,0,0],[1e6,0,0]]}))).contains("point 1"));
        assert!(err(spline(
            json!({"points":[[0,0,0],[1,0,0]],"interpolation":"bezier"})
        ))
        .contains("unknown variant"));
        assert!(
            err(spline(json!({"points":[[0,0,0],[1,0,0]],"loop":true}))).contains("unknown field")
        );
        let many: Vec<[f32; 3]> = (0..257).map(|i| [i as f32, 0.0, 0.0]).collect();
        assert!(err(spline(json!({ "points": many }))).contains("2..256"));
        assert!(err(follow(json!({"path":"r","speed":-1}))).contains("speed"));
        assert!(err(follow(json!({"path":"r","speed":101}))).contains("speed"));
        assert!(err(follow(json!({"path":"r","start":2,"start_fraction":0.5}))).contains("either"));
        assert!(err(follow(json!({"path":"r","start_fraction":1.5}))).contains("start_fraction"));
        assert!(err(follow(json!({"path":"r","offset":51}))).contains("offset"));
        assert!(err(follow(json!({"path":"r","on_arrive":"done"}))).contains("end stop"));
        assert!(
            err(follow(json!({"path":"r","end":"stop","on_arrive":"a b"}))).contains("on_arrive")
        );
        assert!(err(follow(json!({"path":"bad id"}))).contains("path"));
        assert!(err(follow(json!({"path":"r","end":"reverse"}))).contains("unknown variant"));
        assert!(err(follow(json!({"path":"r","turn_rate":-5}))).contains("turn_rate"));
    }

    #[test]
    fn polyline_shapes() {
        let mut out = vec![];
        let open = spline(json!({"points":[[0,0,0],[2,0,0],[2,0,2]]})).unwrap();
        open.polyline(&mut out);
        assert_eq!(out.len(), open.node_count());
        assert_eq!(
            out,
            [
                Vec3::ZERO,
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 2.0)
            ]
        );
        let closed = PathSpline::preset();
        closed.polyline(&mut out);
        assert_eq!(out.len(), 5);
        assert_eq!(out[0], out[4]);
        // Catmull-Rom passes through every control point and repeats the first when closed.
        let mut curve = PathSpline::preset();
        curve.interpolation = PathInterpolation::CatmullRom;
        curve.polyline(&mut out);
        assert_eq!(out.len(), curve.node_count());
        assert_eq!(out.len(), 4 * CATMULL_ROM_SAMPLES + 1);
        for (i, p) in curve.points.iter().enumerate() {
            assert!(out[i * CATMULL_ROM_SAMPLES].distance(Vec3::from_array(*p)) < 1e-4);
        }
        assert!(out[0].distance(*out.last().unwrap()) < 1e-6);
        // Between control points the curve bulges outward (not the straight chord).
        let mid = out[CATMULL_ROM_SAMPLES / 2];
        assert!(mid.z < -3.01, "{mid}");
        let mut open_curve = open.clone();
        open_curve.interpolation = PathInterpolation::CatmullRom;
        open_curve.polyline(&mut out);
        assert_eq!(out.len(), 2 * CATMULL_ROM_SAMPLES + 1);
        assert!(out.last().unwrap().distance(Vec3::new(2.0, 0.0, 2.0)) < 1e-5);
        assert!(out.iter().all(|p| p.is_finite()));
    }
}
