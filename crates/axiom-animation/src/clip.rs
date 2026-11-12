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
/// Weight keys of one morph target (cubic splines store in-tangent, value, out-tangent per key).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorphTrack {
    pub target: String,
    #[serde(default)]
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    pub duration: f32,
    pub tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub morphs: Vec<MorphTrack>,
}
#[derive(Clone, Debug)]
pub struct CompiledClip {
    pub source: Clip,
    pub bones: Vec<usize>,
    /// Morph target index of each `source.morphs` track.
    pub morphs: Vec<usize>,
}
fn factor(i: Interpolation) -> usize {
    if i == Interpolation::CubicSpline {
        3
    } else {
        1
    }
}
/// Key times are non-empty, finite, strictly increasing and inside the clip.
fn check_times(times: &[f32], duration: f32) -> Result<()> {
    ensure!(
        !times.is_empty()
            && times
                .iter()
                .all(|v| v.is_finite() && *v >= 0. && *v <= duration + 1e-4)
            && times.windows(2).all(|w| w[1] > w[0]),
        "key times must be strictly increasing and inside duration"
    );
    Ok(())
}
/// Where `time` falls among `times`: on key `i`, or between keys `a` and `a + 1` at
/// fraction `t` of a span `dt` seconds long.
enum Span {
    Key(usize),
    Between { a: usize, t: f32, dt: f32 },
}
fn span(times: &[f32], time: f32) -> Span {
    let n = times.len();
    if time <= times[0] {
        return Span::Key(0);
    }
    if time >= times[n - 1] {
        return Span::Key(n - 1);
    }
    let b = times.partition_point(|v| *v <= time);
    let a = b - 1;
    let dt = times[b] - times[a];
    Span::Between {
        a,
        t: (time - times[a]) / dt,
        dt,
    }
}
/// Cubic Hermite between `p0` and `p1` with out / in tangents `m0`, `m1` (glTF cubic spline).
fn hermite<T>(p0: T, m0: T, p1: T, m1: T, t: f32, dt: f32) -> T
where
    T: Copy + std::ops::Mul<f32, Output = T> + std::ops::Add<Output = T>,
{
    let t2 = t * t;
    let t3 = t2 * t;
    p0 * (2. * t3 - 3. * t2 + 1.)
        + m0 * dt * (t3 - 2. * t2 + t)
        + p1 * (-2. * t3 + 3. * t2)
        + m1 * dt * (t3 - t2)
}
impl Clip {
    /// [`Self::compile_for`] for an asset without morph targets (a morph track is an error).
    pub fn compile(&self, skeleton: &CompiledSkeleton) -> Result<CompiledClip> {
        self.compile_for(skeleton, &morph::Morphs::default())
    }
    /// Resolves bone tracks against `skeleton` and morph tracks against `morphs`.
    pub fn compile_for(
        &self,
        skeleton: &CompiledSkeleton,
        morphs: &morph::Morphs,
    ) -> Result<CompiledClip> {
        ensure!(
            !self.id.is_empty()
                && self.duration.is_finite()
                && self.duration >= 0.
                && self.duration <= 86400.
                && self.tracks.len() <= MAX_BONES * 3
                && self.morphs.len() <= morph::MAX_TARGETS,
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
            check_times(&t.times, self.duration)?;
            let factor = factor(t.interpolation);
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
        let mut targets = vec![];
        for t in &self.morphs {
            let i = morphs.index(&t.target).with_context(|| {
                format!(
                    "clip {} animates morph target {} which the asset does not have",
                    self.id, t.target
                )
            })?;
            ensure!(!targets.contains(&i), "duplicate morph track {}", t.target);
            targets.push(i);
            check_times(&t.times, self.duration)?;
            total += t.values.len();
            ensure!(
                t.values.len() == t.times.len() * factor(t.interpolation)
                    && total <= 2_000_000
                    && t.values.iter().all(|v| v.is_finite()),
                "clip key/value budget mismatch"
            );
        }
        Ok(CompiledClip {
            source: self.clone(),
            bones,
            morphs: targets,
        })
    }
}
impl Track {
    pub fn sample(&self, time: f32) -> Vec4 {
        let factor = factor(self.interpolation);
        let value = |i: usize| self.values[i * factor + usize::from(factor == 3)];
        let (a, t, dt) = match span(&self.times, time) {
            Span::Key(i) => return value(i),
            Span::Between { a, t, dt } => (a, t, dt),
        };
        let b = a + 1;
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
                let v = hermite(
                    value(a),
                    self.values[a * 3 + 2],
                    value(b),
                    self.values[b * 3],
                    t,
                    dt,
                );
                if self.property == Property::Rotation {
                    v.normalize()
                } else {
                    v
                }
            }
        }
    }
}
impl MorphTrack {
    pub fn sample(&self, time: f32) -> f32 {
        let factor = factor(self.interpolation);
        let value = |i: usize| self.values[i * factor + usize::from(factor == 3)];
        let (a, t, dt) = match span(&self.times, time) {
            Span::Key(i) => return value(i),
            Span::Between { a, t, dt } => (a, t, dt),
        };
        let b = a + 1;
        match self.interpolation {
            Interpolation::Step => value(a),
            Interpolation::Linear => value(a) + (value(b) - value(a)) * t,
            Interpolation::CubicSpline => hermite(
                value(a),
                self.values[a * 3 + 2],
                value(b),
                self.values[b * 3],
                t,
                dt,
            ),
        }
    }
}
impl CompiledClip {
    /// The clip-local time `time` maps to (wrapped when looping, else clamped).
    fn local(&self, time: f32, looping: bool) -> f32 {
        if looping && self.source.duration > 0. {
            time.rem_euclid(self.source.duration)
        } else {
            time.clamp(0., self.source.duration)
        }
    }
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
        let t = self.local(time, looping);
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
    /// Writes the weights of the targets this clip animates into `weights` (one per target of
    /// the asset; the others are left as they are). No allocation.
    pub fn morph_weights_into(&self, time: f32, looping: bool, weights: &mut [f32]) {
        if self.morphs.is_empty() || !time.is_finite() {
            return;
        }
        let t = self.local(time, looping);
        for (track, &i) in self.source.morphs.iter().zip(&self.morphs) {
            if let Some(w) = weights.get_mut(i) {
                *w = track.sample(t);
            }
        }
    }
    pub fn sample(&self, skeleton: &CompiledSkeleton, time: f32, looping: bool) -> Result<Pose> {
        ensure!(time.is_finite(), "non-finite sample time");
        let t = self.local(time, looping);
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
