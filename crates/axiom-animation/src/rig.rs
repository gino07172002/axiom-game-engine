//! Secondary motion applied after the animated pose (clip / controller, overrides, twist, IK)
//! and before skinning: spring-bone chains with bone-attached colliders (`animation.spring`)
//! and look-at chains (`animation.look_at`). Pure functions of the pose, a model matrix, a
//! fixed step and an explicit state; once the state and scratch buffers are sized a step
//! does not allocate.
use crate::*;

pub const MAX_SPRING_CHAINS: usize = 8;
/// Bones of one chain, the tip included.
pub const MAX_SPRING_JOINTS: usize = 32;
/// Simulated segments of one entity (all chains).
pub const MAX_SPRING_SEGMENTS: usize = 64;
pub const MAX_SPRING_COLLIDERS: usize = 16;
pub const MAX_LOOK_CHAINS: usize = 4;
pub const MAX_LOOK_BONES: usize = 4;

/// One spring chain: `root` and its descendants down to `tip` (or, without a tip, along the
/// only child until a leaf or a branching bone). Each bone of the chain except the tip is
/// rotated so that its child follows a simulated point.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SpringChain {
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tip: Option<String>,
    /// Pull back toward the animated direction, per second (0..64).
    pub stiffness: f32,
    /// Fraction of the velocity lost each step (0..1).
    pub drag: f32,
    /// External pull in world space, meters per second added each step (VRM-style, not an
    /// acceleration).
    pub gravity: Vec3,
    /// Collision radius of the chain's joints (meters).
    pub radius: f32,
    /// Collider IDs this chain collides with; empty = all colliders of the entity.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub colliders: Vec<String>,
    pub enabled: bool,
}
impl Default for SpringChain {
    fn default() -> Self {
        Self {
            root: String::new(),
            tip: None,
            stiffness: 1.0,
            drag: 0.4,
            gravity: Vec3::new(0.0, -0.5, 0.0),
            radius: 0.02,
            colliders: vec![],
            enabled: true,
        }
    }
}

/// A sphere (no `tail`) or capsule (`offset`..`tail`) in the bone's local space.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SpringCollider {
    pub bone: String,
    pub offset: Vec3,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail: Option<Vec3>,
    pub radius: f32,
}
impl Default for SpringCollider {
    fn default() -> Self {
        Self {
            bone: String::new(),
            offset: Vec3::ZERO,
            tail: None,
            radius: 0.05,
        }
    }
}

/// `animation.spring` component.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SpringSet {
    pub chains: BTreeMap<String, SpringChain>,
    pub colliders: BTreeMap<String, SpringCollider>,
    /// A simulated point farther than this (meters) from where the animation puts it is
    /// snapped back with no velocity (teleports, scene cuts).
    pub reset_distance: f32,
    /// Scene-view debug draw of joints and colliders.
    pub debug: bool,
}
impl Default for SpringSet {
    fn default() -> Self {
        Self {
            chains: BTreeMap::new(),
            colliders: BTreeMap::new(),
            reset_distance: 1.0,
            debug: false,
        }
    }
}

/// What a look-at chain aims at.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LookTarget {
    /// The first enabled active camera.
    Camera,
    /// An entity's origin, or one of its bones when it has an `animation.player`; `offset`
    /// is in that bone's (or entity's) local space.
    Entity {
        entity: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bone: Option<String>,
        #[serde(default)]
        offset: Vec3,
    },
    /// A fixed world-space point.
    Point { position: Vec3 },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LookBone {
    pub bone: String,
    /// Share of the turn this bone takes (the shares of a chain add up to at most 1).
    #[serde(default = "one")]
    pub weight: f32,
}
fn one() -> f32 {
    1.0
}

/// One look-at chain: `bones` from parent to child (e.g. neck, head); the last bone's
/// `forward` (model space at rest) is turned toward the target.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LookChain {
    pub bones: Vec<LookBone>,
    pub target: LookTarget,
    pub forward: Vec3,
    pub up: Vec3,
    /// Degrees, 0..180.
    pub max_yaw: f32,
    /// Degrees, 0..89.
    pub max_pitch: f32,
    pub weight: f32,
    /// Seconds to cover ~63% of a change (0 = immediate).
    pub smoothing: f32,
    pub enabled: bool,
}
impl Default for LookChain {
    fn default() -> Self {
        Self {
            bones: vec![],
            target: LookTarget::Camera,
            forward: Vec3::Z,
            up: Vec3::Y,
            max_yaw: 60.0,
            max_pitch: 30.0,
            weight: 1.0,
            smoothing: 0.15,
            enabled: true,
        }
    }
}

/// `animation.look_at` component.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LookAtSet {
    pub chains: BTreeMap<String, LookChain>,
    pub debug: bool,
}

fn finite_in(x: f32, lo: f32, hi: f32) -> bool {
    x.is_finite() && (lo..=hi).contains(&x)
}

impl SpringSet {
    /// Checks budgets and ranges without a skeleton (the component validator).
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.chains.len() <= MAX_SPRING_CHAINS,
            "at most {MAX_SPRING_CHAINS} spring chains"
        );
        ensure!(
            self.colliders.len() <= MAX_SPRING_COLLIDERS,
            "at most {MAX_SPRING_COLLIDERS} spring colliders"
        );
        ensure!(
            finite_in(self.reset_distance, 0.05, 100.0),
            "reset_distance must be 0.05..100 m"
        );
        for (id, c) in &self.chains {
            axiom_core::valid_id(id)?;
            ensure!(!c.root.is_empty(), "spring chain {id}: root bone required");
            ensure!(
                finite_in(c.stiffness, 0.0, 64.0)
                    && finite_in(c.drag, 0.0, 1.0)
                    && c.gravity.is_finite()
                    && c.gravity.length() <= 100.0
                    && finite_in(c.radius, 0.0, 1.0),
                "spring chain {id}: stiffness 0..64, drag 0..1, |gravity| <= 100, radius 0..1"
            );
            ensure!(
                c.colliders.len() <= MAX_SPRING_COLLIDERS,
                "spring chain {id}: too many colliders"
            );
            for k in &c.colliders {
                ensure!(
                    self.colliders.contains_key(k),
                    "spring chain {id}: unknown collider {k}"
                );
            }
        }
        for (id, c) in &self.colliders {
            axiom_core::valid_id(id)?;
            ensure!(!c.bone.is_empty(), "spring collider {id}: bone required");
            ensure!(
                c.offset.is_finite()
                    && c.tail.is_none_or(|t| t.is_finite())
                    && finite_in(c.radius, 0.0, 10.0),
                "spring collider {id}: finite offset / tail, radius 0..10"
            );
        }
        Ok(())
    }

    pub fn compile(&self, s: &CompiledSkeleton) -> Result<CompiledSpring> {
        self.validate()?;
        let mut colliders = vec![];
        let mut collider_index = BTreeMap::new();
        for (id, c) in &self.colliders {
            collider_index.insert(id.as_str(), colliders.len());
            colliders.push(CCollider {
                bone: s
                    .bone(&c.bone)
                    .with_context(|| format!("spring collider {id}"))?,
                offset: c.offset,
                tail: c.tail,
                radius: c.radius,
            });
        }
        let mut chains = vec![];
        let mut segments = 0;
        for (id, c) in &self.chains {
            if !c.enabled {
                continue;
            }
            let bones = chain_bones(s, c).with_context(|| format!("spring chain {id}"))?;
            let n = bones.len() - 1;
            ensure!(
                segments + n <= MAX_SPRING_SEGMENTS,
                "spring chains: at most {MAX_SPRING_SEGMENTS} simulated segments per entity"
            );
            let list = if c.colliders.is_empty() {
                (0..colliders.len()).collect()
            } else {
                c.colliders
                    .iter()
                    .map(|k| collider_index[k.as_str()])
                    .collect()
            };
            chains.push(CChain {
                id: id.clone(),
                bones,
                slot: segments,
                stiffness: c.stiffness,
                drag: c.drag,
                gravity: c.gravity,
                radius: c.radius,
                colliders: list,
            });
            segments += n;
        }
        // Chains read their parents' posed globals; a chain below another chain's bone would
        // read a stale parent, and shared bones would be driven twice.
        for a in &chains {
            for b in &chains {
                if a.id != b.id {
                    ensure!(
                        !a.bones
                            .iter()
                            .any(|&x| humanoid::ancestor(s, x, b.bones[0])),
                        "spring chains {} and {} overlap or nest",
                        a.id,
                        b.id
                    );
                }
            }
        }
        Ok(CompiledSpring {
            chains,
            colliders,
            reset_distance: self.reset_distance,
            segments,
        })
    }
}

/// Root..tip bone indices of a chain (at least two bones).
fn chain_bones(s: &CompiledSkeleton, c: &SpringChain) -> Result<Vec<usize>> {
    let root = s.bone(&c.root)?;
    let mut bones = vec![];
    match &c.tip {
        Some(tip) => {
            let tip = s.bone(tip)?;
            ensure!(
                tip != root && humanoid::ancestor(s, root, tip),
                "tip must be a descendant of root"
            );
            let mut b = tip;
            while b != root {
                bones.push(b);
                ensure!(
                    bones.len() < MAX_SPRING_JOINTS,
                    "at most {MAX_SPRING_JOINTS} bones"
                );
                b = s.source.bones[b].parent.expect("descendant of root");
            }
            bones.push(root);
            bones.reverse();
        }
        None => {
            let mut b = root;
            loop {
                bones.push(b);
                let mut children = s
                    .source
                    .bones
                    .iter()
                    .enumerate()
                    .filter(|(_, x)| x.parent == Some(b))
                    .map(|(i, _)| i);
                let (Some(only), None) = (children.next(), children.next()) else {
                    break;
                };
                ensure!(
                    bones.len() < MAX_SPRING_JOINTS,
                    "at most {MAX_SPRING_JOINTS} bones (set tip)"
                );
                b = only;
            }
        }
    }
    ensure!(
        bones.len() >= 2,
        "a chain needs root and at least one descendant bone"
    );
    Ok(bones)
}

struct CChain {
    id: String,
    bones: Vec<usize>,
    /// First state slot (one per segment).
    slot: usize,
    stiffness: f32,
    drag: f32,
    gravity: Vec3,
    radius: f32,
    colliders: Vec<usize>,
}
struct CCollider {
    bone: usize,
    offset: Vec3,
    tail: Option<Vec3>,
    radius: f32,
}

pub struct CompiledSpring {
    chains: Vec<CChain>,
    colliders: Vec<CCollider>,
    reset_distance: f32,
    /// State slots (simulated segments of all enabled chains).
    pub segments: usize,
}

/// Simulated tail points (world space) of every segment, current and previous step.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpringState {
    pub current: Vec<Vec3>,
    pub previous: Vec<Vec3>,
    /// False until the first application (the points then start at the animated pose).
    pub ready: bool,
}

/// Reused buffers of [`CompiledSpring::apply`] / [`CompiledLook::apply`] (not state).
#[derive(Clone, Debug, Default)]
pub struct RigScratch {
    globals: Vec<Mat4>,
    shapes: Vec<(Vec3, Vec3, f32)>,
}

/// Closest point to `p` on segment `a`..`b`.
fn closest(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= 1e-12 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

impl CompiledSpring {
    /// Chain IDs with their first state slot and segment count (metrics / checkpoints).
    pub fn chains(&self) -> impl Iterator<Item = (&str, usize, usize)> {
        self.chains
            .iter()
            .map(|c| (c.id.as_str(), c.slot, c.bones.len() - 1))
    }

    /// Advances (`dt > 0`) or re-applies (`dt == 0`) the springs on `pose`, reading `input`
    /// and writing the next state to `out` (committed by the caller once the whole pose
    /// succeeded). `model` maps the entity's model space to world space.
    pub fn apply(
        &self,
        s: &CompiledSkeleton,
        pose: &mut Pose,
        model: Mat4,
        dt: f32,
        input: &SpringState,
        out: &mut SpringState,
        scratch: &mut RigScratch,
    ) -> Result<()> {
        ensure!(model.is_finite(), "non-finite entity transform");
        s.globals_into(pose, &mut scratch.globals)?;
        let globals = &mut scratch.globals;
        scratch.shapes.clear();
        for c in &self.colliders {
            let g = model * globals[c.bone];
            let a = g.transform_point3(c.offset);
            let b = c.tail.map_or(a, |t| g.transform_point3(t));
            // World radius follows the bone's largest scale axis.
            let scale = g.to_scale_rotation_translation().0.abs().max_element();
            scratch.shapes.push((a, b, c.radius * scale));
        }
        let fresh = !input.ready || input.current.len() != self.segments;
        out.current.clone_from(&input.current);
        out.previous.clone_from(&input.previous);
        out.current.resize(self.segments, Vec3::ZERO);
        out.previous.resize(self.segments, Vec3::ZERO);
        out.ready = true;
        let inverse = model.inverse();
        for chain in &self.chains {
            for k in 0..chain.bones.len() - 1 {
                let (b, child) = (chain.bones[k], chain.bones[k + 1]);
                let parent = s.source.bones[b]
                    .parent
                    .map_or(Mat4::IDENTITY, |p| globals[p]);
                let g = parent * pose.local[b].matrix();
                let head_m = g.w_axis.truncate();
                let child_m = (g * pose.local[child].matrix()).w_axis.truncate();
                let head = model.transform_point3(head_m);
                let animated = model.transform_point3(child_m);
                let len = head.distance(animated);
                let slot = chain.slot + k;
                if len < 1e-6 {
                    globals[b] = g;
                    out.current[slot] = animated;
                    out.previous[slot] = animated;
                    continue;
                }
                let rest = (animated - head) / len;
                let current = out.current[slot];
                if fresh || !current.is_finite() || current.distance(animated) > self.reset_distance
                {
                    out.current[slot] = animated;
                    out.previous[slot] = animated;
                } else if dt > 0.0 {
                    let velocity = (current - out.previous[slot]) * (1.0 - chain.drag);
                    let mut next =
                        current + velocity + rest * (chain.stiffness * dt) + chain.gravity * dt;
                    next = head + (next - head).try_normalize().unwrap_or(rest) * len;
                    for &i in &chain.colliders {
                        let (a, b, r) = scratch.shapes[i];
                        let p = closest(a, b, next);
                        let d = next - p;
                        let min = r + chain.radius;
                        let dist = d.length();
                        if dist < min {
                            next = p + if dist > 1e-6 { d / dist } else { rest } * min;
                            next = head + (next - head).try_normalize().unwrap_or(rest) * len;
                        }
                    }
                    out.previous[slot] = current;
                    out.current[slot] = next;
                }
                let tail = head + (out.current[slot] - head).try_normalize().unwrap_or(rest) * len;
                let to = inverse.transform_point3(tail) - head_m;
                let from = child_m - head_m;
                let (Some(to), Some(from)) = (to.try_normalize(), from.try_normalize()) else {
                    globals[b] = g;
                    continue;
                };
                let delta = Quat::from_rotation_arc(from, to);
                let parent_rotation = parent.to_scale_rotation_translation().1;
                let rotation = g.to_scale_rotation_translation().1;
                pose.local[b].rotation = (parent_rotation.inverse() * delta * rotation).normalize();
                globals[b] = parent * pose.local[b].matrix();
            }
        }
        Ok(())
    }

    /// World-space joints (head of every chain bone) and collider shapes of the last
    /// application, for debug drawing: `joint(position)`, `collider(a, b, radius)`.
    pub fn debug_shapes(
        &self,
        s: &CompiledSkeleton,
        pose: &Pose,
        model: Mat4,
        mut joint: impl FnMut(Vec3),
        mut collider: impl FnMut(Vec3, Vec3, f32),
    ) {
        let Ok(globals) = s.globals(pose) else {
            return;
        };
        for chain in &self.chains {
            for &b in &chain.bones {
                joint(model.transform_point3(globals[b].w_axis.truncate()));
            }
        }
        for c in &self.colliders {
            let g = model * globals[c.bone];
            let a = g.transform_point3(c.offset);
            collider(a, c.tail.map_or(a, |t| g.transform_point3(t)), c.radius);
        }
    }
}

impl LookAtSet {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.chains.len() <= MAX_LOOK_CHAINS,
            "at most {MAX_LOOK_CHAINS} look-at chains"
        );
        for (id, c) in &self.chains {
            axiom_core::valid_id(id)?;
            ensure!(
                (1..=MAX_LOOK_BONES).contains(&c.bones.len()),
                "look-at chain {id}: 1..{MAX_LOOK_BONES} bones"
            );
            let total: f32 = c.bones.iter().map(|b| b.weight).sum();
            ensure!(
                c.bones.iter().all(|b| finite_in(b.weight, 0.0, 1.0)) && total <= 1.001,
                "look-at chain {id}: bone weights 0..1 adding up to at most 1"
            );
            ensure!(
                c.forward.is_finite()
                    && c.up.is_finite()
                    && c.forward.length() > 1e-3
                    && c.up.length() > 1e-3
                    && c.forward.normalize().cross(c.up.normalize()).length() > 1e-2,
                "look-at chain {id}: forward and up must be non-zero and not parallel"
            );
            ensure!(
                finite_in(c.max_yaw, 0.0, 180.0)
                    && finite_in(c.max_pitch, 0.0, 89.0)
                    && finite_in(c.weight, 0.0, 1.0)
                    && finite_in(c.smoothing, 0.0, 10.0),
                "look-at chain {id}: max_yaw 0..180, max_pitch 0..89 degrees, weight 0..1, smoothing 0..10 s"
            );
            match &c.target {
                LookTarget::Camera => {}
                LookTarget::Entity {
                    entity,
                    bone,
                    offset,
                } => {
                    axiom_core::valid_id(entity)?;
                    ensure!(
                        bone.as_ref().is_none_or(|b| !b.is_empty()) && offset.is_finite(),
                        "look-at chain {id}: invalid entity target"
                    );
                }
                LookTarget::Point { position } => ensure!(
                    position.is_finite(),
                    "look-at chain {id}: non-finite target point"
                ),
            }
        }
        Ok(())
    }

    pub fn compile(&self, s: &CompiledSkeleton) -> Result<CompiledLook> {
        self.validate()?;
        let mut chains = vec![];
        for (id, c) in &self.chains {
            let mut bones = vec![];
            for b in &c.bones {
                let i = s
                    .bone(&b.bone)
                    .with_context(|| format!("look-at chain {id}"))?;
                if let Some(&(prev, _)) = bones.last() {
                    ensure!(
                        i != prev && humanoid::ancestor(s, prev, i),
                        "look-at chain {id}: each bone must be a descendant of the previous one"
                    );
                }
                bones.push((i, b.weight));
            }
            let reference = bones.last().expect("validated").0;
            let rest = s.rest_global[reference].to_scale_rotation_translation().1;
            chains.push(CLook {
                id: id.clone(),
                bones,
                forward: rest.inverse() * c.forward.normalize(),
                up: rest.inverse() * c.up.normalize(),
                max_yaw: c.max_yaw.to_radians(),
                max_pitch: c.max_pitch.to_radians(),
                weight: c.weight,
                smoothing: c.smoothing,
                enabled: c.enabled,
            });
        }
        Ok(CompiledLook { chains })
    }
}

struct CLook {
    id: String,
    bones: Vec<(usize, f32)>,
    /// Axes in the reference (last) bone's local frame.
    forward: Vec3,
    up: Vec3,
    max_yaw: f32,
    max_pitch: f32,
    weight: f32,
    smoothing: f32,
    enabled: bool,
}

pub struct CompiledLook {
    chains: Vec<CLook>,
}

/// Smoothed (yaw, pitch) radians per look-at chain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LookState {
    pub angles: Vec<[f32; 2]>,
}

impl CompiledLook {
    pub fn len(&self) -> usize {
        self.chains.len()
    }
    pub fn is_empty(&self) -> bool {
        self.chains.is_empty()
    }
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.chains.iter().map(|c| c.id.as_str())
    }

    /// Turns each chain toward its model-space target (`targets[i]`, `None` = return to the
    /// animated pose), advancing the smoothing by `dt` (0 = re-apply the stored angles).
    /// Yaw is about the reference bone's up axis, pitch about its right axis, both measured
    /// from its animated forward and clamped; the turn is shared by the chain's bones.
    pub fn apply(
        &self,
        s: &CompiledSkeleton,
        pose: &mut Pose,
        dt: f32,
        targets: &[Option<Vec3>],
        input: &LookState,
        out: &mut LookState,
        scratch: &mut RigScratch,
    ) -> Result<()> {
        out.angles.clone_from(&input.angles);
        out.angles.resize(self.chains.len(), [0.0; 2]);
        for (i, c) in self.chains.iter().enumerate() {
            s.globals_into(pose, &mut scratch.globals)?;
            let globals = &scratch.globals;
            let reference = c.bones.last().expect("compiled").0;
            let (_, rotation, position) = globals[reference].to_scale_rotation_translation();
            let forward = (rotation * c.forward).normalize();
            let right = (rotation * c.up).cross(forward).normalize();
            let up = forward.cross(right);
            let goal = match targets.get(i).copied().flatten() {
                Some(t) if c.enabled => match (t - position).try_normalize() {
                    Some(d) => [
                        d.dot(right)
                            .atan2(d.dot(forward))
                            .clamp(-c.max_yaw, c.max_yaw)
                            * c.weight,
                        d.dot(up)
                            .clamp(-1.0, 1.0)
                            .asin()
                            .clamp(-c.max_pitch, c.max_pitch)
                            * c.weight,
                    ],
                    None => out.angles[i],
                },
                _ => [0.0; 2],
            };
            if dt > 0.0 {
                let k = if c.smoothing > 0.0 {
                    1.0 - (-dt / c.smoothing).exp()
                } else {
                    1.0
                };
                let a = &mut out.angles[i];
                a[0] += (goal[0] - a[0]) * k;
                a[1] += (goal[1] - a[1]) * k;
            }
            let [yaw, pitch] = out.angles[i];
            if yaw == 0.0 && pitch == 0.0 {
                continue;
            }
            // Each bone's model-space turn also turns its descendants: with D the turns of
            // the earlier (ancestor) bones, the new local = P^-1 D^-1 delta D R.
            let mut turned = Quat::IDENTITY;
            for &(b, w) in &c.bones {
                let delta =
                    Quat::from_axis_angle(up, yaw * w) * Quat::from_axis_angle(right, -pitch * w);
                let parent = s.source.bones[b].parent.map_or(Quat::IDENTITY, |p| {
                    globals[p].to_scale_rotation_translation().1
                });
                let own = globals[b].to_scale_rotation_translation().1;
                pose.local[b].rotation =
                    (parent.inverse() * turned.inverse() * delta * turned * own).normalize();
                turned = (delta * turned).normalize();
            }
        }
        Ok(())
    }

    /// World position of each chain's reference bone and the direction it faces.
    pub fn debug_rays(
        &self,
        s: &CompiledSkeleton,
        pose: &Pose,
        model: Mat4,
        mut ray: impl FnMut(Vec3, Vec3),
    ) {
        let Ok(globals) = s.globals(pose) else {
            return;
        };
        for c in &self.chains {
            let g = model * globals[c.bones.last().expect("compiled").0];
            let (_, rotation, position) = g.to_scale_rotation_translation();
            ray(position, (rotation * c.forward).normalize());
        }
    }
}

pub const SPRING: &str = "animation.spring";
pub const LOOK_AT: &str = "animation.look_at";

/// Registers `animation.spring` and `animation.look_at` (both need an `animation.player`
/// on the same entity; bones are resolved when the runtime builds the player).
pub fn register(r: &mut axiom_core::Registry) {
    r.component(
        SPRING,
        "animation",
        serde_json::json!({
            "chains": format!("stable ID -> {{root:bone, tip?:bone (default: follow the only child), stiffness:0..64 /s (1), drag:0..1 (0.4), gravity:[x,y,z] world m/s per step (VRM-style, [0,-0.5,0]), radius:m (0.02), colliders?:[collider ID] (default all), enabled}}; at most {MAX_SPRING_CHAINS} chains, {MAX_SPRING_JOINTS} bones per chain, {MAX_SPRING_SEGMENTS} segments per entity"),
            "colliders": format!("stable ID -> {{bone, offset:[x,y,z] bone-local m, tail?:[x,y,z] (capsule end), radius:m}}; at most {MAX_SPRING_COLLIDERS}"),
            "reset_distance": "meters 0.05..100, default 1: a point farther than this from its animated place snaps back (teleports)",
            "debug": "bool: draw joints and colliders in the scene view",
            "order": "after clip / controller, overrides, twist, IK and look-at; before skinning; simulated at the 120 Hz fixed step",
        }),
        |v| serde_json::from_value::<SpringSet>(v.clone())?.validate(),
    );
    r.component(
        LOOK_AT,
        "animation",
        serde_json::json!({
            "chains": format!("stable ID -> {{bones:[{{bone, weight:0..1}}] parent to child (weights add up to <= 1), target:{{kind:camera}}|{{kind:entity, entity, bone?, offset?}}|{{kind:point, position:[x,y,z] world}}, forward:[x,y,z] model space at rest ([0,0,1]), up ([0,1,0]), max_yaw:degrees 0..180 (60), max_pitch:degrees 0..89 (30), weight:0..1, smoothing:seconds 0..10 (0.15), enabled}}; at most {MAX_LOOK_CHAINS} chains of {MAX_LOOK_BONES} bones"),
            "debug": "bool: draw the reference bone's facing ray",
            "order": "after IK, before springs; entity / bone targets read the end of the previous fixed step (one step lag)",
        }),
        |v| serde_json::from_value::<LookAtSet>(v.clone())?.validate(),
    );
    r.world_validators.push(|w| {
        for e in &w.entities {
            let components = w.resolved_components(e);
            for id in [SPRING, LOOK_AT] {
                if components.contains_key(id) {
                    ensure!(
                        components.contains_key("animation.player"),
                        "{id} on {} requires an animation.player",
                        e.id
                    );
                }
            }
            if let Some(look) = components.get(LOOK_AT) {
                let look: LookAtSet = serde_json::from_value(look.clone())?;
                for (chain, c) in &look.chains {
                    if let LookTarget::Entity { entity, .. } = &c.target {
                        ensure!(
                            w.entities.iter().any(|x| &x.id == entity),
                            "{LOOK_AT} chain {chain} on {}: target entity {entity} does not exist",
                            e.id
                        );
                    }
                }
            }
            if let Some(player) = components.get("animation.player") {
                if let Some(ik) = player.get("ik").and_then(serde_json::Value::as_object) {
                    for (id, c) in ik {
                        if let Some(entity) = c.pointer("/socket/entity").and_then(serde_json::Value::as_str) {
                            ensure!(
                                w.entities.iter().any(|x| x.id == entity),
                                "animation.player IK {id} on {}: socket entity {entity} does not exist",
                                e.id
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vertical chain: root at the origin, bones 0.1 m apart going down -Y (a hanging tail),
    /// plus a body bone at the side.
    fn tail(n: usize) -> CompiledSkeleton {
        let mut bones = vec![Bone {
            id: "root".into(),
            aliases: vec![],
            parent: None,
            rest: Trs::default(),
            inverse_bind: Mat4::IDENTITY,
        }];
        for i in 0..n {
            bones.push(Bone {
                id: format!("t{i}"),
                aliases: vec![],
                parent: Some(i),
                rest: Trs {
                    translation: if i == 0 {
                        Vec3::ZERO
                    } else {
                        Vec3::new(0.0, -0.1, 0.0)
                    },
                    ..Default::default()
                },
                inverse_bind: Mat4::IDENTITY,
            });
        }
        bones.push(Bone {
            id: "body".into(),
            aliases: vec![],
            parent: Some(0),
            rest: Trs {
                translation: Vec3::new(0.0, -0.2, 0.1),
                ..Default::default()
            },
            inverse_bind: Mat4::IDENTITY,
        });
        let mut s = Skeleton { bones }.compile().unwrap();
        s.rebind();
        s
    }

    fn set(chain: SpringChain) -> SpringSet {
        SpringSet {
            chains: BTreeMap::from([("tail".to_owned(), chain)]),
            ..Default::default()
        }
    }

    fn run(
        s: &CompiledSkeleton,
        spring: &CompiledSpring,
        models: impl Iterator<Item = Mat4>,
    ) -> (Pose, SpringState) {
        let mut state = SpringState::default();
        let mut next = SpringState::default();
        let mut scratch = RigScratch::default();
        let mut pose = s.rest_pose();
        for model in models {
            pose = s.rest_pose();
            spring
                .apply(
                    s,
                    &mut pose,
                    model,
                    1.0 / 120.0,
                    &state,
                    &mut next,
                    &mut scratch,
                )
                .unwrap();
            std::mem::swap(&mut state, &mut next);
        }
        (pose, state)
    }

    #[test]
    fn chain_walks_single_children_and_respects_tip() {
        let s = tail(5);
        let c = set(SpringChain {
            root: "t0".into(),
            ..Default::default()
        })
        .compile(&s)
        .unwrap();
        assert_eq!(c.segments, 4);
        let c = set(SpringChain {
            root: "t0".into(),
            tip: Some("t2".into()),
            ..Default::default()
        })
        .compile(&s)
        .unwrap();
        assert_eq!(c.segments, 2);
        // root has two children (t0, body): a chain from root stops there.
        let err = set(SpringChain {
            root: "root".into(),
            ..Default::default()
        })
        .compile(&s)
        .err()
        .unwrap();
        assert!(
            format!("{err:#}").contains("at least one descendant"),
            "{err:#}"
        );
        let err = set(SpringChain {
            root: "t3".into(),
            tip: Some("t1".into()),
            ..Default::default()
        })
        .compile(&s)
        .err()
        .unwrap();
        assert!(format!("{err:#}").contains("descendant of root"));
    }

    #[test]
    fn budgets_and_overlaps_are_refused() {
        let s = tail(40);
        let err = set(SpringChain {
            root: "t0".into(),
            ..Default::default()
        })
        .compile(&s)
        .err()
        .unwrap();
        assert!(format!("{err:#}").contains("at most 32 bones"), "{err:#}");
        let mut two = set(SpringChain {
            root: "t0".into(),
            tip: Some("t5".into()),
            ..Default::default()
        });
        two.chains.insert(
            "inner".into(),
            SpringChain {
                root: "t7".into(),
                tip: Some("t9".into()),
                ..Default::default()
            },
        );
        // Below the first chain (a stale parent) and sharing its bones are both refused.
        for root in ["t7", "t3"] {
            two.chains.get_mut("inner").unwrap().root = root.into();
            let err = two.compile(&s).err().unwrap();
            assert!(format!("{err:#}").contains("overlap or nest"), "{err:#}");
        }
        two.chains.remove("inner");
        assert!(two.compile(&s).is_ok());
        let mut bad = set(SpringChain {
            root: "t0".into(),
            stiffness: f32::NAN,
            ..Default::default()
        });
        assert!(bad.validate().is_err());
        bad.chains.get_mut("tail").unwrap().stiffness = 1.0;
        bad.chains.get_mut("tail").unwrap().colliders = vec!["nope".into()];
        assert!(format!("{:#}", bad.validate().unwrap_err()).contains("unknown collider"));
    }

    #[test]
    fn moving_root_drags_the_tail_behind_and_it_settles() {
        let s = tail(4);
        let spring = set(SpringChain {
            root: "t0".into(),
            gravity: Vec3::ZERO,
            ..Default::default()
        })
        .compile(&s)
        .unwrap();
        // Move +X at 1 m/s for 0.25 s: the tail lags behind (-X relative to the root).
        let moving = (0..30).map(|i| Mat4::from_translation(Vec3::X * i as f32 / 120.0));
        let (pose, _) = run(&s, &spring, moving);
        let g = s.globals(&pose).unwrap();
        let tip = g[s.bone("t3").unwrap()].w_axis.truncate();
        assert!(tip.x < -0.01, "tail lags: {tip}");
        // Then rest for 10 s: back near the animated pose.
        let end = Mat4::from_translation(Vec3::X * 29.0 / 120.0);
        let settle = (0..30)
            .map(|i| Mat4::from_translation(Vec3::X * i as f32 / 120.0))
            .chain(std::iter::repeat_n(end, 1200));
        let (pose, _) = run(&s, &spring, settle);
        let g = s.globals(&pose).unwrap();
        let tip = g[s.bone("t3").unwrap()].w_axis.truncate();
        assert!(tip.x.abs() < 0.005 && (tip.y + 0.3).abs() < 0.005, "{tip}");
        // Lengths are preserved.
        let t1 = g[s.bone("t1").unwrap()].w_axis.truncate();
        let t2 = g[s.bone("t2").unwrap()].w_axis.truncate();
        assert!((t1.distance(t2) - 0.1).abs() < 1e-4);
    }

    #[test]
    fn same_inputs_give_bit_identical_states_and_teleports_reset() {
        let s = tail(4);
        let spring = set(SpringChain {
            root: "t0".into(),
            ..Default::default()
        })
        .compile(&s)
        .unwrap();
        let path = |i: usize| {
            Mat4::from_rotation_y(i as f32 * 0.05)
                * Mat4::from_translation(Vec3::X * (i as f32 * 0.07).sin())
        };
        let a = run(&s, &spring, (0..240).map(path)).1;
        let b = run(&s, &spring, (0..240).map(path)).1;
        assert_eq!(a, b);
        assert!(a.current.iter().zip(&a.previous).any(|(c, p)| c != p));
        // A 5 m jump snaps every point to the animated pose with no velocity.
        let jump = (0..240)
            .map(path)
            .chain(std::iter::once(Mat4::from_translation(Vec3::new(
                5.0, 0.0, 0.0,
            ))));
        let (pose, state) = run(&s, &spring, jump);
        assert_eq!(state.current, state.previous);
        let g = s.globals(&pose).unwrap();
        let tip = g[s.bone("t3").unwrap()].w_axis.truncate();
        assert!(tip.abs_diff_eq(Vec3::new(0.0, -0.3, 0.0), 1e-4), "{tip}");
    }

    #[test]
    fn colliders_push_joints_out() {
        let s = tail(4);
        let mut set = set(SpringChain {
            root: "t0".into(),
            gravity: Vec3::ZERO,
            stiffness: 0.0,
            radius: 0.0,
            ..Default::default()
        });
        // A sphere on the body bone (0, -0.2, 0.1) in the way of a tail swinging +Z.
        set.colliders.insert(
            "ball".into(),
            SpringCollider {
                bone: "body".into(),
                radius: 0.08,
                ..Default::default()
            },
        );
        let spring = set.compile(&s).unwrap();
        // Move -Z quickly: the tail swings toward +Z into the sphere.
        let moving = (0..40).map(|i| Mat4::from_translation(Vec3::NEG_Z * i as f32 / 60.0));
        let (pose, state) = run(&s, &spring, moving);
        let model = Mat4::from_translation(Vec3::NEG_Z * 39.0 / 60.0);
        let g = s.globals(&pose).unwrap();
        let ball = model.transform_point3(g[s.bone("body").unwrap()].w_axis.truncate());
        for p in &state.current {
            assert!(p.distance(ball) >= 0.08 - 1e-4, "{p} inside the collider");
        }
    }

    #[test]
    fn re_applying_with_zero_dt_reproduces_the_pose() {
        let s = tail(4);
        let spring = set(SpringChain {
            root: "t0".into(),
            ..Default::default()
        })
        .compile(&s)
        .unwrap();
        let (pose, state) = run(
            &s,
            &spring,
            (0..50).map(|i| Mat4::from_translation(Vec3::X * i as f32 * 0.01)),
        );
        let model = Mat4::from_translation(Vec3::X * 0.49);
        let mut again = s.rest_pose();
        let mut out = SpringState::default();
        spring
            .apply(
                &s,
                &mut again,
                model,
                0.0,
                &state,
                &mut out,
                &mut RigScratch::default(),
            )
            .unwrap();
        assert_eq!(out, state);
        for (a, b) in again.local.iter().zip(&pose.local) {
            assert!(a.rotation.abs_diff_eq(b.rotation, 1e-6));
        }
    }

    fn head() -> CompiledSkeleton {
        let bone = |id: &str, parent: Option<usize>, y: f32| Bone {
            id: id.into(),
            aliases: vec![],
            parent,
            rest: Trs {
                translation: Vec3::Y * y,
                ..Default::default()
            },
            inverse_bind: Mat4::IDENTITY,
        };
        let mut s = Skeleton {
            bones: vec![
                bone("root", None, 0.0),
                bone("neck", Some(0), 1.4),
                bone("head", Some(1), 0.1),
            ],
        }
        .compile()
        .unwrap();
        s.rebind();
        s
    }

    fn look(chain: LookChain) -> CompiledLook {
        LookAtSet {
            chains: BTreeMap::from([("gaze".to_owned(), chain)]),
            debug: false,
        }
        .compile(&head())
        .unwrap()
    }

    fn facing(s: &CompiledSkeleton, pose: &Pose) -> Vec3 {
        let g = s.globals(pose).unwrap();
        g[2].to_scale_rotation_translation().1 * Vec3::Z
    }

    #[test]
    fn look_at_turns_toward_the_target_within_limits() {
        let s = head();
        let one = |b: &str, w: f32| LookBone {
            bone: b.into(),
            weight: w,
        };
        let c = look(LookChain {
            bones: vec![one("neck", 0.4), one("head", 0.6)],
            smoothing: 0.0,
            ..Default::default()
        });
        let run = |c: &CompiledLook, target: Vec3| {
            let mut pose = s.rest_pose();
            let mut out = LookState::default();
            c.apply(
                &s,
                &mut pose,
                1.0 / 120.0,
                &[Some(target)],
                &LookState::default(),
                &mut out,
                &mut RigScratch::default(),
            )
            .unwrap();
            (pose, out)
        };
        // 30° to the right (+X) and 10° up: inside the limits, the head faces it.
        let eye = Vec3::Y * 1.5;
        let dir = Quat::from_rotation_y(30f32.to_radians())
            * Quat::from_rotation_x(-10f32.to_radians())
            * Vec3::Z;
        let (pose, state) = run(&c, eye + dir * 2.0);
        assert!(
            (state.angles[0][0].to_degrees() - 30.0).abs() < 0.01,
            "{:?}",
            state.angles
        );
        assert!(
            (state.angles[0][1].to_degrees() - 10.0).abs() < 0.01,
            "{:?}",
            state.angles
        );
        let f = facing(&s, &pose);
        assert!(f.angle_between(dir) < 0.6f32.to_radians(), "{f} vs {dir}");
        // 120° to the right: clamped to max_yaw 60°.
        let (pose, state) = run(
            &c,
            eye + Quat::from_rotation_y(120f32.to_radians()) * Vec3::Z,
        );
        assert!((state.angles[0][0].to_degrees() - 60.0).abs() < 0.01);
        let f = facing(&s, &pose);
        assert!((f.x.atan2(f.z).to_degrees() - 60.0).abs() < 0.5, "{f}");
        // Half weight: half the turn.
        let half = look(LookChain {
            bones: vec![one("head", 1.0)],
            smoothing: 0.0,
            weight: 0.5,
            ..Default::default()
        });
        let (_, state) = run(
            &half,
            eye + Quat::from_rotation_y(-40f32.to_radians()) * Vec3::Z,
        );
        assert!((state.angles[0][0].to_degrees() + 20.0).abs() < 0.01);
    }

    #[test]
    fn look_at_smoothing_approaches_and_releases() {
        let s = head();
        let c = look(LookChain {
            bones: vec![LookBone {
                bone: "head".into(),
                weight: 1.0,
            }],
            smoothing: 0.2,
            ..Default::default()
        });
        let target = Vec3::new(2.0, 1.5, 2.0);
        let mut state = LookState::default();
        let mut next = LookState::default();
        let mut scratch = RigScratch::default();
        let mut history = vec![];
        for _ in 0..120 {
            let mut pose = s.rest_pose();
            c.apply(
                &s,
                &mut pose,
                1.0 / 120.0,
                &[Some(target)],
                &state,
                &mut next,
                &mut scratch,
            )
            .unwrap();
            std::mem::swap(&mut state, &mut next);
            history.push(state.angles[0][0]);
        }
        assert!(history.windows(2).all(|w| w[1] >= w[0]));
        // After 1 s = 5 time constants: within 1% of 45°.
        assert!(
            (history[119].to_degrees() - 45.0).abs() < 0.5,
            "{}",
            history[119].to_degrees()
        );
        assert!(history[11] < history[119] * 0.5);
        // No target: back toward the animated pose.
        for _ in 0..240 {
            let mut pose = s.rest_pose();
            c.apply(
                &s,
                &mut pose,
                1.0 / 120.0,
                &[None],
                &state,
                &mut next,
                &mut scratch,
            )
            .unwrap();
            std::mem::swap(&mut state, &mut next);
        }
        assert!(state.angles[0][0].abs() < 0.01);
    }

    #[test]
    fn look_at_refuses_bad_chains() {
        let s = head();
        let mut set = LookAtSet::default();
        set.chains.insert(
            "gaze".into(),
            LookChain {
                bones: vec![
                    LookBone {
                        bone: "head".into(),
                        weight: 0.5,
                    },
                    LookBone {
                        bone: "neck".into(),
                        weight: 0.5,
                    },
                ],
                ..Default::default()
            },
        );
        assert!(format!("{:#}", set.compile(&s).err().unwrap()).contains("descendant"));
        set.chains.get_mut("gaze").unwrap().bones.reverse();
        set.chains.get_mut("gaze").unwrap().bones[0].weight = 0.9;
        assert!(format!("{:#}", set.validate().unwrap_err()).contains("at most 1"));
        set.chains.get_mut("gaze").unwrap().bones[0].weight = 0.5;
        set.chains.get_mut("gaze").unwrap().up = Vec3::Z;
        assert!(set.validate().is_err());
        let json = serde_json::json!({"chains":{"g":{"bones":[{"bone":"head"}],"target":{"kind":"entity","entity":"npc","bone":"head"}}}});
        let parsed: LookAtSet = serde_json::from_value(json).unwrap();
        assert!(parsed.validate().is_ok());
        let bad = serde_json::json!({"chains":{"g":{"bones":[{"bone":"head"}],"target":{"kind":"entity","entity":"npc","bogus":1}}}});
        assert!(serde_json::from_value::<LookAtSet>(bad).is_err());
    }
}
