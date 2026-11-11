//! Analytic two-bone IK in normalized model coordinates. Pole controls the bend plane.
use crate::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwoBoneIk {
    pub root: String,
    pub middle: String,
    pub end: String,
    pub target: Vec3,
    pub pole: Vec3,
    #[serde(default = "one")]
    pub weight: f32,
    #[serde(default)]
    pub end_rotation: Option<Quat>,
    /// When set, `target` follows this socket (resolved by the runtime each step from the
    /// end of the previous step's poses); `target` is the fallback until it resolves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<Socket>,
}
/// A point on another entity: its origin, or one of its bones when it has an
/// `animation.player`, plus `offset` in that bone's (or entity's) local space.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Socket {
    pub entity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
    #[serde(default)]
    pub offset: Vec3,
}
impl Socket {
    pub fn validate(&self) -> Result<()> {
        axiom_core::valid_id(&self.entity)?;
        ensure!(
            self.bone
                .as_ref()
                .is_none_or(|b| !b.is_empty() && b.len() <= 256)
                && self.offset.is_finite(),
            "invalid socket bone / offset"
        );
        Ok(())
    }
}
fn one() -> f32 {
    1.
}
#[derive(Clone, Debug, Serialize)]
pub struct IkResult {
    pub reachable: bool,
    pub residual: f32,
    pub applied_weight: f32,
}
pub struct CompiledIk {
    pub source: TwoBoneIk,
    root: usize,
    middle: usize,
    end: usize,
}
impl TwoBoneIk {
    pub fn compile(&self, s: &CompiledSkeleton) -> Result<CompiledIk> {
        ensure!(
            self.target.is_finite()
                && self.pole.is_finite()
                && self.weight.is_finite()
                && (0.0..=1.0).contains(&self.weight)
                && self
                    .end_rotation
                    .is_none_or(|q| q.is_finite() && (q.length_squared() - 1.).abs() < 0.001),
            "invalid IK settings"
        );
        if let Some(socket) = &self.socket {
            socket.validate()?;
        }
        let root = s.bone(&self.root)?;
        let middle = s.bone(&self.middle)?;
        let end = s.bone(&self.end)?;
        ensure!(
            root != middle
                && middle != end
                && humanoid::ancestor(s, root, middle)
                && humanoid::ancestor(s, middle, end),
            "invalid IK chain hierarchy"
        );
        let a = s.rest_global[root].w_axis.truncate();
        let b = s.rest_global[middle].w_axis.truncate();
        let c = s.rest_global[end].w_axis.truncate();
        ensure!(
            a.distance(b) > 1e-5 && b.distance(c) > 1e-5,
            "zero-length IK chain"
        );
        Ok(CompiledIk {
            source: self.clone(),
            root,
            middle,
            end,
        })
    }
}
impl CompiledIk {
    pub fn apply(&self, s: &CompiledSkeleton, pose: &mut Pose) -> Result<IkResult> {
        self.apply_to(s, pose, self.source.target)
    }
    /// [`Self::apply`] toward `goal` (model space) instead of the authored one.
    pub fn apply_to(&self, s: &CompiledSkeleton, pose: &mut Pose, goal: Vec3) -> Result<IkResult> {
        ensure!(goal.is_finite(), "non-finite IK target");
        let mut work = pose.clone();
        let g = s.globals(&work)?;
        let a = g[self.root].w_axis.truncate();
        let b = g[self.middle].w_axis.truncate();
        let c = g[self.end].w_axis.truncate();
        let l1 = a.distance(b);
        let l2 = b.distance(c);
        ensure!(l1 > 1e-6 && l2 > 1e-6, "collapsed IK chain");
        let offset = goal - a;
        let distance = offset.length();
        let direction = if distance > 1e-6 {
            offset / distance
        } else {
            (c - a).try_normalize().unwrap_or(Vec3::Y)
        };
        let min = (l1 - l2).abs() + 1e-5;
        let max = (l1 + l2 - 1e-5).max(min);
        let d = distance.clamp(min, max);
        let reachable = (min..=max + 1e-5).contains(&distance);
        if self.source.weight == 0.0 {
            return Ok(IkResult {
                reachable,
                residual: c.distance(goal),
                applied_weight: 0.0,
            });
        }
        for i in [self.root, self.middle, self.end] {
            let scale = g[i].to_scale_rotation_translation().0;
            ensure!(
                scale.min_element() > 0.0 && scale.max_element() - scale.min_element() < 1e-4,
                "IK requires positive uniform bone scale; bake rig scale before solving"
            );
        }
        let pole = self.source.pole - a;
        let bend = (pole - direction * pole.dot(direction))
            .try_normalize()
            .unwrap_or_else(|| {
                let axis = if direction.x.abs() < 0.7 {
                    Vec3::X
                } else {
                    Vec3::Y
                };
                (axis - direction * axis.dot(direction)).normalize()
            });
        let along = (l1 * l1 - l2 * l2 + d * d) / (2. * d);
        let height = (l1 * l1 - along * along).max(0.).sqrt();
        let middle = a + direction * along + bend * height;
        let target = a + direction * d;
        let rotate = |pose: &mut Pose, globals: &[Mat4], i: usize, from: Vec3, to: Vec3| {
            let delta = Quat::from_rotation_arc(from.normalize(), to.normalize());
            let p = s.source.bones[i].parent.map_or(Quat::IDENTITY, |p| {
                globals[p].to_scale_rotation_translation().1
            });
            pose.local[i].rotation =
                (p.inverse() * delta * globals[i].to_scale_rotation_translation().1).normalize();
        };
        rotate(&mut work, &g, self.root, b - a, middle - a);
        let g = s.globals(&work)?;
        let b = g[self.middle].w_axis.truncate();
        let c = g[self.end].w_axis.truncate();
        rotate(&mut work, &g, self.middle, c - b, target - b);
        if let Some(end) = self.source.end_rotation {
            let g = s.globals(&work)?;
            let p = s.source.bones[self.end]
                .parent
                .map_or(Quat::IDENTITY, |p| g[p].to_scale_rotation_translation().1);
            work.local[self.end].rotation = (p.inverse() * end).normalize();
        }
        let blended = Pose::blend(pose, &work, self.source.weight)?;
        let end = s.globals(&blended)?[self.end].w_axis.truncate();
        *pose = blended;
        Ok(IkResult {
            reachable,
            residual: end.distance(goal),
            applied_weight: self.source.weight,
        })
    }
}
