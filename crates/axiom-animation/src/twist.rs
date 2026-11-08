use crate::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwistBone {
    pub bone: String,
    pub fraction: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Twist {
    pub enabled: bool,
    pub driver: String,
    pub axis: Vec3,
    pub bones: Vec<TwistBone>,
    pub driver_fraction: f32,
    pub min_angle: f32,
    pub max_angle: f32,
    pub preserve: Vec<String>,
}
impl Default for Twist {
    fn default() -> Self {
        Self {
            enabled: true,
            driver: String::new(),
            axis: Vec3::X,
            bones: vec![],
            driver_fraction: 1.,
            min_angle: -std::f32::consts::PI,
            max_angle: std::f32::consts::PI,
            preserve: vec![],
        }
    }
}
pub struct CompiledTwist {
    source: Twist,
    driver: usize,
    helpers: Vec<(usize, f32)>,
    preserve: Vec<usize>,
}
pub fn swing_twist(q: Quat, axis: Vec3) -> (Quat, Quat) {
    let p = axis * q.xyz().dot(axis);
    let raw = Quat::from_xyzw(p.x, p.y, p.z, q.w);
    let twist = if raw.length_squared() > 1e-10 {
        raw.normalize()
    } else {
        Quat::IDENTITY
    };
    ((q * twist.inverse()).normalize(), twist)
}
impl Twist {
    pub fn compile(&self, s: &CompiledSkeleton) -> Result<CompiledTwist> {
        ensure!(
            self.axis.is_finite()
                && (self.axis.length_squared() - 1.).abs() < 0.001
                && self.driver_fraction.is_finite()
                && (0.0..=1.0).contains(&self.driver_fraction)
                && self.min_angle.is_finite()
                && self.max_angle.is_finite()
                && self.min_angle <= self.max_angle
                && self.min_angle >= -std::f32::consts::PI
                && self.max_angle <= std::f32::consts::PI
                && self.bones.len() <= 32,
            "invalid twist settings"
        );
        let driver = s.bone(&self.driver)?;
        let mut helpers = vec![];
        let mut seen = std::collections::BTreeSet::new();
        for b in &self.bones {
            let i = s.bone(&b.bone)?;
            ensure!(
                i != driver
                    && seen.insert(i)
                    && b.fraction.is_finite()
                    && (0.0..=1.0).contains(&b.fraction),
                "invalid twist helper/fraction"
            );
            helpers.push((i, b.fraction));
        }
        helpers.sort_by_key(|(i, _)| s.order.iter().position(|x| x == i).unwrap());
        let preserve = self
            .preserve
            .iter()
            .map(|b| s.bone(b))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            preserve.iter().all(|i| !seen.contains(i) && *i != driver),
            "twist cannot preserve a driven bone"
        );
        Ok(CompiledTwist {
            source: self.clone(),
            driver,
            helpers,
            preserve,
        })
    }
}
impl CompiledTwist {
    pub fn apply(&self, s: &CompiledSkeleton, pose: &mut Pose) -> Result<f32> {
        if !self.source.enabled {
            return Ok(0.);
        }
        let before = s.globals(pose)?;
        let rest = s.source.bones[self.driver].rest.rotation;
        let delta = (rest.inverse() * pose.local[self.driver].rotation).normalize();
        let (swing, twist) = swing_twist(delta, self.source.axis);
        let mut angle = 2. * twist.xyz().dot(self.source.axis).atan2(twist.w);
        if angle > std::f32::consts::PI {
            angle -= std::f32::consts::TAU
        }
        if angle < -std::f32::consts::PI {
            angle += std::f32::consts::TAU
        }
        angle = angle.clamp(self.source.min_angle, self.source.max_angle);
        let parent = s.source.bones[self.driver]
            .parent
            .map_or(Quat::IDENTITY, |p| {
                before[p].to_scale_rotation_translation().1
            });
        let base = (parent * rest * swing).normalize();
        let driver_rest = s.rest_global[self.driver].to_scale_rotation_translation().1;
        pose.local[self.driver].rotation = (rest
            * swing
            * Quat::from_axis_angle(self.source.axis, angle * self.source.driver_fraction))
        .normalize();
        for &(i, fraction) in &self.helpers {
            let globals = s.globals(pose)?;
            let p = s.source.bones[i].parent.map_or(Quat::IDENTITY, |p| {
                globals[p].to_scale_rotation_translation().1
            });
            let relative =
                driver_rest.inverse() * s.rest_global[i].to_scale_rotation_translation().1;
            let desired =
                base * Quat::from_axis_angle(self.source.axis, angle * fraction) * relative;
            pose.local[i].rotation = (p.inverse() * desired).normalize();
        }
        for &i in &s.order {
            if self.preserve.contains(&i) {
                let g = s.globals(pose)?;
                let p = s.source.bones[i]
                    .parent
                    .map_or(Quat::IDENTITY, |p| g[p].to_scale_rotation_translation().1);
                pose.local[i].rotation =
                    (p.inverse() * before[i].to_scale_rotation_translation().1).normalize();
            }
        }
        Ok(angle)
    }
}
