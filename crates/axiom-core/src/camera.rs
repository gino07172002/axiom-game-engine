//! Follow cameras (`camera.follow`): a `render.camera` that orbits a target entity at a
//! distance, looks at a point above it, smooths its motion and pulls in front of static
//! obstacles instead of going through them. The runtime system `camera.rig` writes the
//! camera's transform every fixed step; `camera.follow.set` / `camera.activate` /
//! `camera.inspect` / `camera.summary` drive and observe it.
//!
//! Conventions: yaw 0 puts the camera on the target's world +Z side looking toward -Z (the
//! action controller's camera convention); positive pitch raises the camera. Angles are
//! authored in degrees.
//!
//! What blocks the camera (static geometry, collected once when the runtime is built, at the
//! authored pose): `physics.collider` boxes and capsules, `nav.blocker` rectangles, and the
//! `render.mesh` parts of entities tagged [`BLOCKER_TAG`]. Entities moved by a system (any
//! mover component, `path.follow`, or under one) and the followed target's own hierarchy
//! never block. See docs/camera-follow.zh-TW.md.
use crate::raycast::Shape;
use crate::{CompiledEntity, Diagnostic};
use anyhow::{ensure, Result};
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// `camera.follow` components per world (each needs its own camera; MAX_CAMERAS active).
pub const MAX_FOLLOW_CAMERAS: usize = crate::MAX_CAMERAS;
/// Static blocker shapes collected for follow cameras per world.
pub const MAX_CAMERA_BLOCKERS: usize = 8192;
/// Shapes one cast may test (the camera's two casts per step each have this budget). Above
/// it the camera stops at its minimum distance and `observe camera` counts `over_budget`.
pub const MAX_BLOCKER_TESTS: usize = 512;
/// Tag that makes an entity's `render.mesh` parts block follow cameras.
pub const BLOCKER_TAG: &str = "camera_blocker";
/// The largest collision radius (the grid is bucketed for it).
pub const MAX_COLLISION_RADIUS: f32 = 2.0;
/// Vertical half extent used for `nav.blocker` rectangles on entities without a mesh.
pub const UNBOUNDED_HALF_HEIGHT: f32 = 100_000.0;

fn yes() -> bool {
    true
}
fn default_distance() -> f32 {
    5.0
}
fn default_min_distance() -> f32 {
    1.0
}
fn default_max_distance() -> f32 {
    12.0
}
fn default_pitch() -> f32 {
    20.0
}
fn default_min_pitch() -> f32 {
    -30.0
}
fn default_max_pitch() -> f32 {
    75.0
}
fn default_look_height() -> f32 {
    1.5
}
fn default_smoothing() -> f32 {
    0.1
}
fn default_yaw_speed() -> f32 {
    120.0
}
fn default_pitch_speed() -> f32 {
    60.0
}
fn default_zoom_speed() -> f32 {
    4.0
}
fn default_radius() -> f32 {
    0.2
}
fn default_recover() -> f32 {
    0.25
}

/// Optional orbit / zoom input: `input.bindings` axis names (never key names).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FollowInput {
    /// Axis that turns the camera around the target ("" = none). Positive turns the camera
    /// counter-clockwise seen from above (yaw grows).
    #[serde(default)]
    pub yaw_axis: String,
    /// Axis that raises (positive) or lowers the camera ("" = none).
    #[serde(default)]
    pub pitch_axis: String,
    /// Axis that moves the camera away (positive) or closer ("" = none).
    #[serde(default)]
    pub zoom_axis: String,
    /// Degrees per second at full axis.
    #[serde(default = "default_yaw_speed")]
    pub yaw_speed: f32,
    #[serde(default = "default_pitch_speed")]
    pub pitch_speed: f32,
    /// Meters per second at full axis.
    #[serde(default = "default_zoom_speed")]
    pub zoom_speed: f32,
}

impl Default for FollowInput {
    fn default() -> Self {
        Self {
            yaw_axis: String::new(),
            pitch_axis: String::new(),
            zoom_axis: String::new(),
            yaw_speed: default_yaw_speed(),
            pitch_speed: default_pitch_speed(),
            zoom_speed: default_zoom_speed(),
        }
    }
}

impl FollowInput {
    pub fn axes(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            ("yaw_axis", self.yaw_axis.as_str()),
            ("pitch_axis", self.pitch_axis.as_str()),
            ("zoom_axis", self.zoom_axis.as_str()),
        ]
        .into_iter()
        .filter(|(_, a)| !a.is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FollowCollision {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Meters kept between the camera and blockers (the cast is a sphere of this radius).
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// Half-life (seconds) of moving back out after an obstacle no longer blocks. Pulling in
    /// is immediate (the camera never passes through a blocker to smooth).
    #[serde(default = "default_recover")]
    pub recover: f32,
}

impl Default for FollowCollision {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: default_radius(),
            recover: default_recover(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CameraFollow {
    /// Entity ID the camera follows (its world pivot).
    pub target: String,
    /// Desired meters from the look point (zoom input and `camera.follow.set` change it at
    /// runtime within min..max).
    #[serde(default = "default_distance")]
    pub distance: f32,
    /// The camera never comes closer, even when a blocker is nearer.
    #[serde(default = "default_min_distance")]
    pub min_distance: f32,
    #[serde(default = "default_max_distance")]
    pub max_distance: f32,
    /// Degrees about +Y; 0 = camera on the target's +Z side.
    #[serde(default)]
    pub yaw: f32,
    /// Degrees above the horizontal.
    #[serde(default = "default_pitch")]
    pub pitch: f32,
    #[serde(default = "default_min_pitch")]
    pub min_pitch: f32,
    #[serde(default = "default_max_pitch")]
    pub max_pitch: f32,
    /// Meters above the target's pivot the camera looks at (and orbits around).
    #[serde(default = "default_look_height")]
    pub look_height: f32,
    /// Half-life (seconds) of the look point following the target (0 = rigid).
    #[serde(default = "default_smoothing")]
    pub position_smoothing: f32,
    /// Half-life (seconds) of yaw, pitch and distance approaching their goals (0 = at once).
    #[serde(default = "default_smoothing")]
    pub rotation_smoothing: f32,
    #[serde(default)]
    pub input: FollowInput,
    #[serde(default)]
    pub collision: FollowCollision,
    /// A disabled follow leaves the camera where it is.
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl CameraFollow {
    pub fn new(target: &str) -> Self {
        Self {
            target: target.to_owned(),
            distance: default_distance(),
            min_distance: default_min_distance(),
            max_distance: default_max_distance(),
            yaw: 0.0,
            pitch: default_pitch(),
            min_pitch: default_min_pitch(),
            max_pitch: default_max_pitch(),
            look_height: default_look_height(),
            position_smoothing: default_smoothing(),
            rotation_smoothing: default_smoothing(),
            input: FollowInput::default(),
            collision: FollowCollision::default(),
            enabled: true,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let within = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        crate::valid_id(&self.target).map_err(|e| anyhow::anyhow!("target: {e:#}"))?;
        ensure!(
            within(self.min_distance, 0.1, 100.0) && within(self.max_distance, 0.1, 100.0),
            "min_distance and max_distance must be 0.1..100 meters"
        );
        ensure!(
            self.min_distance <= self.max_distance,
            "min_distance must not exceed max_distance"
        );
        ensure!(
            within(self.distance, self.min_distance, self.max_distance),
            "distance must lie within min_distance..max_distance"
        );
        ensure!(
            within(self.min_pitch, -89.0, 89.0) && within(self.max_pitch, -89.0, 89.0),
            "min_pitch and max_pitch must be -89..89 degrees"
        );
        ensure!(
            self.min_pitch <= self.max_pitch,
            "min_pitch must not exceed max_pitch"
        );
        ensure!(
            within(self.pitch, self.min_pitch, self.max_pitch),
            "pitch must lie within min_pitch..max_pitch"
        );
        ensure!(
            within(self.yaw, -3600.0, 3600.0),
            "yaw must be -3600..3600 degrees"
        );
        ensure!(
            within(self.look_height, -10.0, 10.0),
            "look_height must be -10..10 meters"
        );
        ensure!(
            within(self.position_smoothing, 0.0, 5.0) && within(self.rotation_smoothing, 0.0, 5.0),
            "position_smoothing and rotation_smoothing are half-lives of 0..5 seconds"
        );
        for (field, axis) in self.input.axes() {
            crate::valid_id(axis).map_err(|e| anyhow::anyhow!("input.{field}: {e:#}"))?;
        }
        ensure!(
            within(self.input.yaw_speed, 0.0, 720.0) && within(self.input.pitch_speed, 0.0, 720.0),
            "input yaw_speed and pitch_speed must be 0..720 degrees per second"
        );
        ensure!(
            within(self.input.zoom_speed, 0.0, 100.0),
            "input zoom_speed must be 0..100 meters per second"
        );
        ensure!(
            within(self.collision.radius, 0.0, MAX_COLLISION_RADIUS),
            "collision radius must be 0..{MAX_COLLISION_RADIUS} meters"
        );
        ensure!(
            within(self.collision.recover, 0.0, 5.0),
            "collision recover is a half-life of 0..5 seconds"
        );
        Ok(())
    }
}

pub fn follow_schema() -> Value {
    json!({
        "target": "entity ID (required): the camera orbits this entity's world pivot; not the camera itself or one of its descendants, and not another follow camera",
        "distance": "meters, default 5, within min_distance..max_distance: desired distance from the look point",
        "min_distance": "meters 0.1..100, default 1: the camera never comes closer, even when a blocker is nearer",
        "max_distance": "meters 0.1..100, default 12 (zoom limit)",
        "yaw": "degrees, default 0: 0 = camera on the target's world +Z side looking toward -Z; positive turns counter-clockwise seen from above",
        "pitch": "degrees, default 20, within min_pitch..max_pitch (-89..89): positive = above the look point",
        "min_pitch": "degrees, default -30",
        "max_pitch": "degrees, default 75",
        "look_height": "meters -10..10, default 1.5: look point above the target pivot (the orbit center)",
        "position_smoothing": "half-life seconds 0..5, default 0.1: how the look point trails the target (0 = rigid). The look point never trails through a blocker",
        "rotation_smoothing": "half-life seconds 0..5, default 0.1: yaw, pitch and distance approach their goals (input, camera.follow.set)",
        "input": "{yaw_axis, pitch_axis, zoom_axis: input.bindings axis names (\"\" = none; keys are never hard-coded), yaw_speed deg/s 0..720 default 120, pitch_speed default 60, zoom_speed m/s 0..100 default 4}",
        "collision": format!("{{enabled default true, radius meters 0..{MAX_COLLISION_RADIUS} default 0.2 (sphere cast), recover half-life seconds 0..5 default 0.25}}: the camera stops in front of blockers at once and moves back out smoothly; never closer than min_distance"),
        "blockers": format!("static, at the authored pose: physics.collider box/capsule, nav.blocker (height = the entity's own render.mesh bounds, else unbounded), render.mesh parts of entities tagged {BLOCKER_TAG}. Moving entities (any mover, path.follow, or under one), the target's hierarchy and entities in inactive scenes never block"),
        "enabled": "bool, default true: a disabled follow leaves the camera where it is",
        "requires": "render.camera and core.transform on the same entity; the camera must be a root entity (no parent); system camera.rig (capability camera)",
        "conflicts": "authoring error camera_conflict: another mover on the camera, or a resource that drives the camera (e.g. action.session / fps / rts camera field); tween transform tracks and path.follow report tween_conflict / path_conflict",
        "runtime": "fixed step (120 Hz) after every mover and camera rig, before audio.spatial; actions camera.follow.set {id, target?, distance?, yaw?, pitch?} | camera.activate {id} | camera.inspect {id} | camera.summary; observe metrics `camera` (desired vs actual distance, hit entity)",
        "budget": format!("{MAX_FOLLOW_CAMERAS} follow cameras, {MAX_CAMERA_BLOCKERS} blocker shapes per world, {MAX_BLOCKER_TESTS} shape tests per cast (over budget: the camera stays at min_distance and observe counts it)"),
        "checkpoint": "not supported: worlds scheduling camera.rig refuse checkpoints explicitly"
    })
}

/// What a static blocker shape came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerSource {
    Collider,
    NavBlocker,
    Mesh,
}

/// One static blocker shape and the entity slot it belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blocker {
    pub shape: Shape,
    pub owner: usize,
    pub source: BlockerSource,
}

/// World-space shape of a `physics.collider` placed by `m` (scale applied like the
/// navigation grid does: box extents through the matrix, capsule radius by the largest
/// scale).
pub fn collider_shape(collider: &crate::Collider, m: Mat4) -> Shape {
    match collider.shape {
        crate::ColliderShape::Box { half_extents } => {
            Shape::unit_box(m * Mat4::from_scale(Vec3::from_array(half_extents) * 2.0))
        }
        crate::ColliderShape::Capsule {
            half_height,
            radius,
        } => {
            let (scale, _, _) = m.to_scale_rotation_translation();
            Shape::Capsule {
                a: m.transform_point3(Vec3::Y * half_height),
                b: m.transform_point3(-Vec3::Y * half_height),
                radius: radius * scale.abs().max_element(),
            }
        }
    }
}

/// Static camera blockers of a compiled world. `movers` lists the components whose entities
/// a system moves (their shapes, and those of their descendants, are skipped).
pub fn collect_blockers(
    entities: &[CompiledEntity],
    parents: &[Option<usize>],
    movers: &BTreeMap<String, String>,
) -> Result<Vec<Blocker>> {
    let mut out = vec![];
    for (i, e) in entities.iter().enumerate() {
        let tagged = e.mesh.is_some() && e.tags.iter().any(|t| t == BLOCKER_TAG);
        let blocker = e.components.get("nav.blocker");
        if e.collider.is_none() && blocker.is_none() && !tagged {
            continue;
        }
        if crate::navigation::moved_by_system(entities, parents, movers, i) {
            continue;
        }
        let m = crate::navigation::authored_matrix(entities, parents, i);
        let parts = || {
            e.mesh
                .iter()
                .flat_map(|mesh| mesh.parts.iter())
                .map(move |p| Shape::unit_box(m * p.transform.matrix()))
        };
        if let Some(collider) = &e.collider {
            out.push(Blocker {
                shape: collider_shape(collider, m),
                owner: i,
                source: BlockerSource::Collider,
            });
        }
        if let Some(v) = blocker {
            let b: crate::blocker::NavBlocker = serde_json::from_value(v.clone())?;
            // Same rectangle as the walkers use (the authored position, see blocker.rs).
            let p = e.transform.as_ref().map_or([0.0; 3], |t| t.position);
            let (lo, hi) = b.rect(p);
            let (y0, y1) = parts()
                .map(|s| s.bounds(0.0))
                .fold(None, |acc: Option<(f32, f32)>, (a, b)| {
                    Some(acc.map_or((a.y, b.y), |(lo, hi)| (lo.min(a.y), hi.max(b.y))))
                })
                .unwrap_or((-UNBOUNDED_HALF_HEIGHT, UNBOUNDED_HALF_HEIGHT));
            out.push(Blocker {
                shape: Shape::aligned(Vec3::new(lo[0], y0, lo[1]), Vec3::new(hi[0], y1, hi[1])),
                owner: i,
                source: BlockerSource::NavBlocker,
            });
        }
        if tagged {
            for shape in parts() {
                out.push(Blocker {
                    shape,
                    owner: i,
                    source: BlockerSource::Mesh,
                });
            }
        }
        if out.len() > MAX_CAMERA_BLOCKERS {
            return Err(Diagnostic::error(
                "camera_budget",
                "/entities",
                format!(
                    "camera blocker budget exceeded (> {MAX_CAMERA_BLOCKERS} shapes from physics.collider, nav.blocker and {BLOCKER_TAG} mesh parts)"
                ),
            ));
        }
    }
    Ok(out)
}

/// Frame-rate independent exponential approach: after `half_life` seconds half of the gap
/// is left, whatever the step size (two steps of dt/2 equal one step of dt).
pub fn damp(current: f32, goal: f32, half_life: f32, dt: f32) -> f32 {
    if half_life <= 0.0 {
        return goal;
    }
    goal + (current - goal) * (-dt / half_life).exp2()
}

/// [`damp`] for a point.
pub fn damp_vec(current: Vec3, goal: Vec3, half_life: f32, dt: f32) -> Vec3 {
    if half_life <= 0.0 {
        return goal;
    }
    goal + (current - goal) * (-dt / half_life).exp2()
}

/// [`damp`] for an angle in radians, the short way round; result in -PI..PI.
pub fn damp_angle(current: f32, goal: f32, half_life: f32, dt: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let delta = (goal - current + PI).rem_euclid(TAU) - PI;
    let left = if half_life <= 0.0 {
        0.0
    } else {
        delta * (-dt / half_life).exp2()
    };
    (current + delta - left + PI).rem_euclid(TAU) - PI
}

/// Unit offset from the look point toward the camera for yaw / pitch in radians.
pub fn orbit_direction(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    )
}

/// Rotation of a camera at `eye` looking at `at` with world +Y up (cameras look along -Z).
pub fn look_rotation(eye: Vec3, at: Vec3) -> Quat {
    Quat::from_mat4(&Mat4::look_at_rh(eye, at, Vec3::Y).inverse()).normalize()
}

/// World-level checks run by `World::compile` on resolved components: budget, required
/// system, the camera itself (render.camera, root), the target, input axis names, mover and
/// driver conflicts, and the blocker budget.
pub fn validate_world(
    world: &crate::World,
    registry: &crate::Registry,
    entities: &[CompiledEntity],
    indices: &BTreeMap<String, usize>,
    parents: &[Option<usize>],
) -> Result<()> {
    let follows: Vec<usize> = (0..entities.len())
        .filter(|&i| entities[i].camera_follow.is_some())
        .collect();
    if follows.is_empty() {
        return Ok(());
    }
    if follows.len() > MAX_FOLLOW_CAMERAS {
        return Err(Diagnostic::error(
            "camera_budget",
            "/entities",
            format!(
                "follow camera budget exceeded ({} > {MAX_FOLLOW_CAMERAS})",
                follows.len()
            ),
        ));
    }
    let at = |id: &str, field: &str| {
        format!(
            "/entities/@{}/components/camera.follow{field}",
            crate::local::escape(id)
        )
    };
    let invalid = |id: &str, field: &str, message: String| {
        Diagnostic::error("invalid_camera", at(id, field), message)
    };
    let conflict =
        |id: &str, message: String| Diagnostic::error("camera_conflict", at(id, ""), message);
    let axes: crate::InputBindings = world
        .resources
        .get("input.bindings")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?
        .unwrap_or_default();
    for &slot in &follows {
        let e = &entities[slot];
        let follow = e.camera_follow.as_ref().unwrap();
        if !world.systems.iter().any(|s| s == "camera.rig") {
            return Err(invalid(
                &e.id,
                "",
                "camera.follow needs the camera.rig system (add it to systems)".into(),
            ));
        }
        if e.camera.is_none() {
            return Err(invalid(
                &e.id,
                "",
                format!("{} has camera.follow but no render.camera", e.id),
            ));
        }
        if let Some(p) = parents[slot] {
            return Err(invalid(
                &e.id,
                "",
                format!(
                    "{} has a parent ({}); a follow camera must be a root entity (it writes its world pose)",
                    e.id, entities[p].id
                ),
            ));
        }
        let &target = indices.get(&follow.target).ok_or_else(|| {
            invalid(
                &e.id,
                "/target",
                format!("target entity {} does not exist", follow.target),
            )
        })?;
        let mut cursor = Some(target);
        let mut depth = 0;
        while let Some(c) = cursor {
            if c == slot {
                return Err(invalid(
                    &e.id,
                    "/target",
                    format!(
                        "target {} is the camera itself or one of its descendants",
                        follow.target
                    ),
                ));
            }
            cursor = parents[c];
            depth += 1;
            if depth > entities.len() {
                break;
            }
        }
        if entities[target].camera_follow.is_some() {
            return Err(invalid(
                &e.id,
                "/target",
                format!(
                    "target {} is itself a follow camera (cameras would chase each other); follow its target instead",
                    follow.target
                ),
            ));
        }
        if entities[target].transform.is_none() {
            return Err(invalid(
                &e.id,
                "/target",
                format!("target {} has no core.transform", follow.target),
            ));
        }
        for (field, axis) in follow.input.axes() {
            if !axes.axes.contains_key(axis) {
                return Err(invalid(
                    &e.id,
                    &format!("/input/{field}"),
                    format!("input.bindings has no axis {axis}; bind keys to it or use an existing axis name"),
                ));
            }
        }
        if let Some((component, system)) = e
            .components
            .keys()
            .filter(|c| *c != "camera.follow")
            .find_map(|c| registry.movers.get_key_value(c))
        {
            return Err(conflict(
                &e.id,
                format!(
                    "{} has camera.follow and {component} (moved by {system}); a camera takes one mover",
                    e.id
                ),
            ));
        }
        // Gameplay sessions that place their own camera name it in a top-level `camera`
        // field (action.session, fps / rts sessions, showcase.tour).
        if let Some((resource, _)) = world
            .resources
            .iter()
            .find(|(_, v)| v.get("camera").and_then(Value::as_str) == Some(e.id.as_str()))
        {
            return Err(conflict(
                &e.id,
                format!(
                    "resource {resource} drives camera {} itself; remove camera.follow or point the resource at another camera",
                    e.id
                ),
            ));
        }
    }
    // The runtime collects blockers only for colliding cameras; so does the budget check.
    if follows.iter().any(|&i| {
        entities[i]
            .camera_follow
            .as_ref()
            .unwrap()
            .collision
            .enabled
    }) {
        collect_blockers(entities, parents, &registry.movers)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn follow(v: Value) -> Result<CameraFollow> {
        let f: CameraFollow = serde_json::from_value(v)?;
        f.validate()?;
        Ok(f)
    }
    fn err(v: Value) -> String {
        format!("{:#}", follow(v).unwrap_err())
    }

    #[test]
    fn defaults_round_trip_and_invalid_fields_are_rejected() {
        let f = follow(json!({"target":"hero"})).unwrap();
        assert_eq!(f, CameraFollow::new("hero"));
        let back = serde_json::to_value(&f).unwrap();
        assert_eq!(serde_json::from_value::<CameraFollow>(back).unwrap(), f);
        assert!(err(json!({})).contains("target"));
        assert!(err(json!({"target":"a b"})).contains("target"));
        assert!(err(json!({"target":"h","distance":20})).contains("distance"));
        assert!(
            err(json!({"target":"h","min_distance":3,"max_distance":2,"distance":2}))
                .contains("min_distance")
        );
        assert!(err(json!({"target":"h","pitch":80})).contains("pitch"));
        assert!(err(json!({"target":"h","min_pitch":-90})).contains("pitch"));
        assert!(err(json!({"target":"h","position_smoothing":-1})).contains("smoothing"));
        assert!(err(json!({"target":"h","collision":{"radius":3}})).contains("radius"));
        assert!(err(json!({"target":"h","input":{"yaw_axis":"bad axis"}})).contains("yaw_axis"));
        assert!(err(json!({"target":"h","input":{"key":"KeyQ"}})).contains("unknown field"));
        assert!(err(json!({"target":"h","offset":[0,1,0]})).contains("unknown field"));
        assert!(follow(
            json!({"target":"英雄","distance":0.1,"min_distance":0.1,"pitch":-89,"min_pitch":-89})
        )
        .is_ok());
    }

    #[test]
    fn damping_is_frame_rate_independent_monotone_and_exact_at_zero() {
        let one = damp(0.0, 10.0, 0.25, 1.0 / 30.0);
        let mut two = 0.0;
        for _ in 0..2 {
            two = damp(two, 10.0, 0.25, 1.0 / 60.0);
        }
        let mut four = 0.0;
        for _ in 0..4 {
            four = damp(four, 10.0, 0.25, 1.0 / 120.0);
        }
        assert!(
            (one - two).abs() < 1e-4 && (one - four).abs() < 1e-4,
            "{one} {two} {four}"
        );
        // Half the gap after one half-life.
        let mut x = 0.0;
        for _ in 0..30 {
            x = damp(x, 10.0, 0.25, 1.0 / 120.0);
        }
        assert!((x - 5.0).abs() < 0.2, "{x}");
        // Monotone, never overshooting.
        let mut last = 0.0;
        for _ in 0..1000 {
            let next = damp(last, 10.0, 0.25, 1.0 / 120.0);
            assert!(next >= last && next <= 10.0);
            last = next;
        }
        assert_eq!(damp(3.0, 7.0, 0.0, 0.1), 7.0);
        let v = damp_vec(Vec3::ZERO, Vec3::X * 4.0, 0.25, 1.0 / 30.0);
        assert!((v.x - one * 0.4).abs() < 1e-4);
        // Angles go the short way round.
        use std::f32::consts::PI;
        let a = damp_angle(PI - 0.1, -PI + 0.1, 0.0, 0.1);
        assert!((a - (-PI + 0.1)).abs() < 1e-5, "{a}");
        let half = damp_angle(PI - 0.1, -PI + 0.1, 1.0, 1.0);
        assert!(
            (half.abs() - PI).abs() < 1e-4,
            "half way crosses PI, not zero: {half}"
        );
    }

    #[test]
    fn orbit_direction_and_look_rotation_follow_the_conventions() {
        let behind = orbit_direction(0.0, 0.0);
        assert!((behind - Vec3::Z).length() < 1e-6, "yaw 0 = +Z side");
        let side = orbit_direction(std::f32::consts::FRAC_PI_2, 0.0);
        assert!((side - Vec3::X).length() < 1e-6);
        let up = orbit_direction(0.0, 30f32.to_radians());
        assert!((up.y - 0.5).abs() < 1e-6 && (up.length() - 1.0).abs() < 1e-6);
        let eye = Vec3::new(0.0, 3.0, 6.0);
        let q = look_rotation(eye, Vec3::new(0.0, 1.0, 0.0));
        let forward = q * -Vec3::Z;
        let expected = (Vec3::new(0.0, 1.0, 0.0) - eye).normalize();
        assert!((forward - expected).length() < 1e-5);
        assert!((q * Vec3::Y).y > 0.0, "upright");
    }
}
