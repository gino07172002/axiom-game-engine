use crate::*;
use glam::Vec4;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Step,
    #[default]
    Linear,
    CubicSpline,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub bone: String,
    pub property: Property,
    #[serde(default)]
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<Vec4>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    pub duration: f32,
    pub tracks: Vec<Track>,
}
#[derive(Clone, Debug)]
pub struct CompiledClip {
    pub source: Clip,
    pub bones: Vec<usize>,
}
impl Clip {
    pub fn compile(&self, skeleton: &CompiledSkeleton) -> Result<CompiledClip> {
        ensure!(
            !self.id.is_empty()
                && self.duration.is_finite()
                && self.duration >= 0.
                && self.duration <= 86400.
                && self.tracks.len() <= MAX_BONES * 3,
            "invalid clip/budget"
        );
        let mut bones = vec![];
        let mut seen = std::collections::BTreeSet::new();
        let mut total = 0;
        for t in &self.tracks {
            bones.push(skeleton.bone(&t.bone)?);
            ensure!(
                seen.insert((t.bone.clone(), t.property as u8)),
                "duplicate clip channel"
            );
            ensure!(
                !t.times.is_empty()
                    && t.times
                        .iter()
                        .all(|v| v.is_finite() && *v >= 0. && *v <= self.duration + 1e-4)
                    && t.times.windows(2).all(|w| w[1] > w[0]),
                "key times must be strictly increasing and inside duration"
            );
            let factor = if t.interpolation == Interpolation::CubicSpline {
                3
            } else {
                1
            };
            total += t.values.len();
            ensure!(
                t.values.len() == t.times.len() * factor
                    && total <= 2_000_000
                    && t.values.iter().all(|v| v.is_finite()),
                "clip key/value budget mismatch"
            );
            if t.property == Property::Rotation {
                for value in t
                    .values
                    .iter()
                    .skip(if factor == 3 { 1 } else { 0 })
                    .step_by(factor)
                {
                    ensure!(
                        (value.length_squared() - 1.).abs() < 0.002,
                        "rotation key must be normalized"
                    )
                }
            }
        }
        Ok(CompiledClip {
            source: self.clone(),
            bones,
        })
    }
}
impl Track {
    pub fn sample(&self, time: f32) -> Vec4 {
        let n = self.times.len();
        let factor = if self.interpolation == Interpolation::CubicSpline {
            3
        } else {
            1
        };
        let value = |i: usize| self.values[i * factor + usize::from(factor == 3)];
        if time <= self.times[0] {
            return value(0);
        }
        if time >= self.times[n - 1] {
            return value(n - 1);
        }
        let b = self.times.partition_point(|v| *v <= time);
        let a = b - 1;
        let dt = self.times[b] - self.times[a];
        let t = (time - self.times[a]) / dt;
        match self.interpolation {
            Interpolation::Step => value(a),
            Interpolation::Linear if self.property == Property::Rotation => Vec4::from_array(
                Quat::from_array(value(a).to_array())
                    .slerp(Quat::from_array(value(b).to_array()), t)
                    .normalize()
                    .to_array(),
            ),
            Interpolation::Linear => value(a).lerp(value(b), t),
            Interpolation::CubicSpline => {
                let t2 = t * t;
                let t3 = t2 * t;
                let v = value(a) * (2. * t3 - 3. * t2 + 1.)
                    + self.values[a * 3 + 2] * dt * (t3 - 2. * t2 + t)
                    + value(b) * (-2. * t3 + 3. * t2)
                    + self.values[b * 3] * dt * (t3 - t2);
                if self.property == Property::Rotation {
                    v.normalize()
                } else {
                    v
                }
            }
        }
    }
}
impl CompiledClip {
    /// [`Self::sample`] into `out` (resized once, then reused): no allocation per call and
    /// no globals validation (the caller validates the final pose).
    pub fn sample_into(
        &self,
        skeleton: &CompiledSkeleton,
        time: f32,
        looping: bool,
        out: &mut Pose,
    ) -> Result<()> {
        ensure!(time.is_finite(), "non-finite sample time");
        let t = if looping && self.source.duration > 0. {
            time.rem_euclid(self.source.duration)
        } else {
            time.clamp(0., self.source.duration)
        };
        skeleton.rest_into(out);
        for (track, &bone) in self.source.tracks.iter().zip(&self.bones) {
            let v = track.sample(t);
            match track.property {
                Property::Translation => out.local[bone].translation = v.truncate(),
                Property::Scale => out.local[bone].scale = v.truncate(),
                Property::Rotation => {
                    out.local[bone].rotation = Quat::from_array(v.to_array()).normalize()
                }
            }
        }
        Ok(())
    }
    pub fn sample(&self, skeleton: &CompiledSkeleton, time: f32, looping: bool) -> Result<Pose> {
        ensure!(time.is_finite(), "non-finite sample time");
        let t = if looping && self.source.duration > 0. {
            time.rem_euclid(self.source.duration)
        } else {
            time.clamp(0., self.source.duration)
        };
        let mut pose = skeleton.rest_pose();
        for (track, &bone) in self.source.tracks.iter().zip(&self.bones) {
            let v = track.sample(t);
            match track.property {
                Property::Translation => pose.local[bone].translation = v.truncate(),
                Property::Scale => pose.local[bone].scale = v.truncate(),
                Property::Rotation => {
                    pose.local[bone].rotation = Quat::from_array(v.to_array()).normalize()
                }
            }
        }
        skeleton.globals(&pose)?;
        Ok(pose)
    }
}
