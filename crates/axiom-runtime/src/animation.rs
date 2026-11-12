//! Generic skeletal capability: prepare assets once, evaluate typed poses at fixed steps.
use crate::{
    presentation::{Instance, MeshFrame, RenderFrame},
    world::*,
};
use anyhow::{ensure, Context, Result};
use axiom_animation::{
    authoring::*,
    clip::CompiledClip,
    humanoid::Retargeter,
    ik::{CompiledIk, IkResult},
    twist::CompiledTwist,
    *,
};
use axiom_core::gameplay::{RuntimeCommand, Scalar};
use axiom_core::local::at;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{any::Any, collections::BTreeMap, sync::Arc};

#[path = "animation_choreo.rs"]
mod choreo;
#[path = "animation_parts.rs"]
mod parts;
mod rig;

#[derive(Clone, Default)]
pub(crate) struct AnimationCues {
    pub pending: Vec<PlayArgs>,
    pub catalog: BTreeMap<String, BTreeMap<String, f32>>,
    pub bone_names: BTreeMap<String, std::collections::BTreeSet<String>>,
    pub socket_requests: BTreeMap<String, std::collections::BTreeSet<String>>,
    pub sockets: BTreeMap<(String, String), Mat4>,
    /// Entities whose one-shot clip (or controller one-shot state) has reached its end
    /// (FSM `clip_finished`). Changed only when an entity enters or leaves the set.
    pub finished: std::collections::BTreeSet<String>,
}
#[derive(Clone)]
struct Fade {
    from: Pose,
    /// Morph weights faded out of (empty without morph targets).
    from_morph: Vec<f32>,
    elapsed: f32,
    duration: f32,
}
/// An `animation.morph` override of one target: its weight and a ramp in progress
/// (from, to, elapsed, duration seconds).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ManualMorph {
    value: Option<f32>,
    ramp: Option<(f32, f32, f32, f32)>,
}
#[derive(Clone)]
struct Controller {
    config: Player,
    asset: Arc<Asset>,
    source: Arc<Asset>,
    clips: Arc<BTreeMap<String, CompiledClip>>,
    retarget: Option<Arc<Retargeter>>,
    active: Option<String>,
    time: f32,
    playing: bool,
    speed: f32,
    /// `animation.speed` ramp in progress: (from, to, elapsed, duration) seconds.
    ramp: Option<(f32, f32, f32, f32)>,
    looping: bool,
    pose: Pose,
    sampled: Pose,
    fade: Option<Fade>,
    twists: BTreeMap<String, Arc<CompiledTwist>>,
    ik: BTreeMap<String, Arc<CompiledIk>>,
    overrides: BTreeMap<usize, Quat>,
    fault: Option<String>,
    ik_results: BTreeMap<String, IkResult>,
    /// The next step starts a clip (markers at 0 fire), set by `play` and on build.
    marker_fresh: bool,
    /// This step's playback interval of the primary clip (prev, end; unwrapped): seconds,
    /// or normalized time of the controller state when the flag is set.
    crossing: Option<(f32, f32, bool)>,
    /// Controller only: the last step of the state a transition just left (state, prev, end),
    /// in that state's normalized time.
    exit_crossing: Option<(u16, f32, f32)>,
    /// `animation.player.on_end` (index into the system's events).
    on_end: Option<usize>,
    /// `animation.controller`: when present it chooses the clips (and `play` is refused).
    animator: Option<Animator>,
    /// Reused pose buffers (sized once): the pose being built, a blend scratch / the next
    /// `sampled`, and bone globals for validation. Evaluation itself does not allocate unless
    /// retargeting, twists or IK are configured.
    accum: Pose,
    scratch: Pose,
    globals: Vec<Mat4>,
    /// Morph weights the skinning applies (one per target of `asset`; all morph buffers are
    /// empty without targets and sized once otherwise).
    morph_weights: Vec<f32>,
    /// Clip / controller weights before `animation.morph` overrides (a fade starts from it).
    morph_sampled: Vec<f32>,
    /// Weights being built and one sample's weights (blend scratch).
    morph_accum: Vec<f32>,
    morph_scratch: Vec<f32>,
    morph_manual: Vec<ManualMorph>,
    /// Hidden primitives (`animation.player.hidden`, `animation.visibility`).
    hidden: PartMask,
    /// Controller layers: a layer's sampled pose and its first frame (reused buffers).
    layer_pose: Pose,
    layer_ref: Pose,
    /// Morph tracks of retarget-source clips naming targets the drawn asset lacks.
    unbound_morphs: usize,
    /// `animation.spring` / `animation.look_at` / IK sockets of this entity, if any.
    rig: Option<Box<rig::Rig>>,
}
/// Per-entity state of an `animation.controller` (cheap to clone: `Arc`s and `Copy` state).
#[derive(Clone)]
pub(crate) struct Animator {
    pub id: Arc<str>,
    pub compiled: Arc<controller::Compiled>,
    pub state: controller::State,
    /// Local position at the previous step (parameters with `source: speed`).
    last_position: Option<[f32; 3]>,
    /// `animation.play` cues (FSM entry clips, actions) dropped because the controller owns
    /// playback; reported in metrics instead of being silently lost.
    ignored_cues: u32,
    /// Per layer: the source-skeleton bones its mask covers (empty = all).
    masks: Arc<Vec<Vec<u16>>>,
}
impl Animator {
    pub(crate) fn describe(&self) -> Value {
        let mut v = self.compiled.describe(&self.state);
        v["controller"] = json!(&*self.id);
        if self.ignored_cues > 0 {
            v["ignored_cues"] = json!(self.ignored_cues);
        }
        v
    }
}
impl Controller {
    fn new(config: Player, lib: &Library, assets: &BTreeMap<String, Arc<Asset>>) -> Result<Self> {
        let asset = assets
            .get(&config.asset)
            .context("animation asset not prepared")?
            .clone();
        let (source, retarget) = if let Some(rt) = &config.retarget {
            let source = assets
                .get(&rt.source_asset)
                .context("retarget source missing")?
                .clone();
            let compiled = Retargeter::new(
                &source.skeleton,
                &lib.profiles[&rt.source_profile],
                &asset.skeleton,
                &lib.profiles[&rt.target_profile],
                rt.require_complete,
            )?;
            (source, Some(Arc::new(compiled)))
        } else {
            (asset.clone(), None)
        };
        // Bone tracks bind to the clip's (source) skeleton, morph tracks to the drawn asset.
        // A retarget source's morph tracks animate the source mesh: they bind to the drawn
        // asset by target name, and the ones it lacks are counted (`inspect`), not applied.
        let mut clips = BTreeMap::new();
        let mut unbound_morphs = 0;
        for c in &source.clips {
            let compiled = if retarget.is_some() && !c.morphs.is_empty() {
                let mut bound = c.clone();
                bound
                    .morphs
                    .retain(|t| asset.morphs.index(&t.target).is_some());
                unbound_morphs += c.morphs.len() - bound.morphs.len();
                bound.compile_for(&source.skeleton, &asset.morphs)?
            } else {
                c.compile_for(&source.skeleton, &asset.morphs)?
            };
            ensure!(
                clips.insert(c.id.clone(), compiled).is_none(),
                "duplicate asset clip ID"
            );
        }
        for (id, c) in &lib.clips {
            if c.tracks
                .iter()
                .all(|t| source.skeleton.indices.contains_key(&t.bone))
                && c.morphs
                    .iter()
                    .all(|t| asset.morphs.index(&t.target).is_some())
            {
                ensure!(
                    clips
                        .insert(id.clone(), c.compile_for(&source.skeleton, &asset.morphs)?)
                        .is_none(),
                    "library clip shadows imported clip: {id}"
                );
            }
        }
        let mut hidden = [0; 4];
        for (i, group) in config.hidden.iter().enumerate() {
            let parts = asset.parts(group);
            ensure!(
                parts != [0; 4],
                "hidden/{i}: no primitive named {group} (parts: {})",
                part_names(&asset)
            );
            for (h, p) in hidden.iter_mut().zip(parts) {
                *h |= p;
            }
        }
        let mut defaults = vec![];
        asset.morphs.defaults_into(&mut defaults);
        let twists = config
            .twists
            .iter()
            .map(|(id, t)| Ok((id.clone(), Arc::new(t.compile(&asset.skeleton)?))))
            .collect::<Result<_>>()?;
        let ik = config
            .ik
            .iter()
            .map(|(id, t)| Ok((id.clone(), Arc::new(t.compile(&asset.skeleton)?))))
            .collect::<Result<_>>()?;
        let mut out = Self {
            active: config.clip.clone(),
            time: 0.,
            playing: config.playing,
            speed: config.speed,
            ramp: None,
            looping: config.looping,
            pose: asset.skeleton.rest_pose(),
            sampled: asset.skeleton.rest_pose(),
            asset,
            source,
            clips: Arc::new(clips),
            retarget,
            twists,
            ik,
            overrides: BTreeMap::new(),
            fade: None,
            fault: None,
            ik_results: BTreeMap::new(),
            animator: None,
            marker_fresh: true,
            crossing: None,
            exit_crossing: None,
            on_end: None,
            accum: Pose { local: vec![] },
            scratch: Pose { local: vec![] },
            globals: vec![],
            morph_manual: vec![ManualMorph::default(); defaults.len()],
            morph_sampled: defaults.clone(),
            morph_accum: defaults.clone(),
            morph_scratch: defaults.clone(),
            morph_weights: defaults,
            hidden,
            layer_pose: Pose { local: vec![] },
            layer_ref: Pose { local: vec![] },
            unbound_morphs,
            rig: None,
            config,
        };
        out.evaluate(0.)?;
        Ok(out)
    }
    /// One `animation.morph` override.
    fn morph(&mut self, a: &MorphArgs) -> Result<()> {
        let targets = &self.asset.morphs.targets;
        let i = match (&a.target, a.index) {
            (Some(name), _) => self.asset.morphs.index(name).with_context(|| {
                format!(
                    "no morph target {name} (targets: {})",
                    targets
                        .iter()
                        .map(|t| t.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?,
            (None, Some(i)) => {
                ensure!(
                    i < targets.len(),
                    "morph target index {i} out of range ({} targets)",
                    targets.len()
                );
                i
            }
            (None, None) => anyhow::bail!("pass target or index"),
        };
        let m = &mut self.morph_manual[i];
        match a.weight {
            None => *m = ManualMorph::default(),
            Some(w) if a.ramp_seconds > 0.0 => {
                let from = self.morph_weights[i];
                *m = ManualMorph {
                    value: Some(from),
                    ramp: Some((from, w, 0.0, a.ramp_seconds)),
                };
            }
            Some(w) => {
                *m = ManualMorph {
                    value: Some(w),
                    ramp: None,
                }
            }
        }
        Ok(())
    }
    /// Advances `animation.morph` ramps by `dt` seconds; whether any ran.
    fn ramp_morphs(&mut self, dt: f32) -> bool {
        let mut any = false;
        for m in &mut self.morph_manual {
            if let Some((from, to, elapsed, duration)) = &mut m.ramp {
                *elapsed += dt;
                let k = (*elapsed / *duration).min(1.0);
                m.value = Some(*from + (*to - *from) * k);
                if k >= 1.0 {
                    m.ramp = None;
                }
                any = true;
            }
        }
        any
    }
    /// The applied weights: overrides, else the sampled weights.
    fn compose_morphs(&mut self) {
        for ((w, m), s) in self
            .morph_weights
            .iter_mut()
            .zip(&self.morph_manual)
            .zip(&self.morph_sampled)
        {
            *w = m.value.unwrap_or(*s);
        }
    }
    /// Attaches controller `id` (clip lengths come from this player's clips).
    fn attach(&mut self, id: &str, def: &controller::ControllerDef) -> Result<()> {
        let clips = self.clips.clone();
        let compiled =
            controller::Compiled::compile(def, &|c| clips.get(c).map(|m| m.source.duration))?;
        let masks = (0..compiled.layer_count())
            .map(|i| compiled.layer_mask(i, &self.source.skeleton))
            .collect::<Result<Vec<_>>>()?;
        self.animator = Some(Animator {
            id: id.into(),
            state: compiled.start(),
            compiled: Arc::new(compiled),
            last_position: None,
            ignored_cues: 0,
            masks: Arc::new(masks),
        });
        self.evaluate(0.)
    }
    /// The source-skeleton pose of the controller's weighted samples, into `self.accum`, then
    /// its layers (morph weights likewise into `self.morph_accum`).
    fn blend_samples(&mut self) -> Result<()> {
        let a = self.animator.as_ref().expect("animator");
        let samples = a.compiled.samples(&a.state);
        let morphing = !self.morph_accum.is_empty();
        let mut total = 0.0;
        for (n, s) in samples.as_slice().iter().enumerate() {
            let (target, weights) = if n == 0 {
                (&mut self.accum, &mut self.morph_accum)
            } else {
                (&mut self.scratch, &mut self.morph_scratch)
            };
            let clip = match a.compiled.clip(s) {
                Some(id) => Some(
                    self.clips
                        .get(id)
                        .with_context(|| format!("clip not found: {id}"))?,
                ),
                None => None,
            };
            match clip {
                Some(c) => c.sample_into(&self.source.skeleton, s.time, false, target)?,
                None => self.source.skeleton.rest_into(target),
            }
            if morphing {
                self.asset.morphs.defaults_into(weights);
                if let Some(c) = clip {
                    c.morph_weights_into(s.time, false, weights);
                }
            }
            total += s.weight;
            if n > 0 {
                let k = s.weight / total;
                self.accum.blend_toward(&self.scratch, k)?;
                for (w, x) in self.morph_accum.iter_mut().zip(&self.morph_scratch) {
                    *w += (x - *w) * k;
                }
            }
        }
        if samples.len == 0 {
            self.source.skeleton.rest_into(&mut self.accum);
            self.asset.morphs.defaults_into(&mut self.morph_accum);
        }
        for i in 0..a.compiled.layer_count() {
            let l = a.compiled.layer_sample(&a.state, i);
            if l.weight <= 0.0 {
                continue;
            }
            let clip = self
                .clips
                .get(l.clip)
                .with_context(|| format!("clip not found: {}", l.clip))?;
            let skeleton = &self.source.skeleton;
            clip.sample_into(skeleton, l.time, false, &mut self.layer_pose)?;
            if l.additive {
                clip.sample_into(skeleton, 0.0, false, &mut self.layer_ref)?;
            }
            controller::apply_layer(
                &mut self.accum,
                &self.layer_pose,
                &self.layer_ref,
                &a.masks[i],
                l.weight,
                l.additive,
            );
            if morphing {
                for (track, &t) in clip.source.morphs.iter().zip(&clip.morphs) {
                    let w = &mut self.morph_accum[t];
                    let v = track.sample(l.time);
                    if l.additive {
                        *w += (v - track.sample(0.0)) * l.weight;
                    } else {
                        *w += (v - *w) * l.weight;
                    }
                }
            }
        }
        Ok(())
    }
    fn evaluate(&mut self, dt: f32) -> Result<()> {
        self.asset.morphs.defaults_into(&mut self.morph_accum);
        if self.animator.is_some() {
            self.blend_samples()?;
        } else if let Some(id) = &self.active {
            let clip = self
                .clips
                .get(id)
                .with_context(|| format!("clip not found: {id}"))?;
            clip.sample_into(
                &self.source.skeleton,
                self.time,
                self.looping,
                &mut self.accum,
            )?;
            clip.morph_weights_into(self.time, self.looping, &mut self.morph_accum);
        } else {
            self.source.skeleton.rest_into(&mut self.accum);
        }
        if let Some(rt) = &self.retarget {
            self.accum = rt.apply(
                &self.accum,
                self.config.retarget.as_ref().unwrap().root_motion,
            )?;
        }
        if let Some(fade) = &mut self.fade {
            fade.elapsed += dt;
            let weight = (fade.elapsed / fade.duration).clamp(0., 1.);
            self.accum.blend_from(&fade.from, weight)?;
            for (w, from) in self.morph_accum.iter_mut().zip(&fade.from_morph) {
                *w = from + (*w - from) * weight;
            }
            if weight >= 1. {
                self.fade = None;
            }
        }
        // The next `sampled` (before overrides / constraints), committed with the pose.
        self.scratch.local.clone_from(&self.accum.local);
        for (&i, &rotation) in &self.overrides {
            self.accum.local[i].rotation = rotation;
        }
        for (id, twist) in &self.twists {
            if let Err(e) = twist.apply(&self.asset.skeleton, &mut self.accum) {
                return at(&format!("/twists/{id}"), Err(e));
            }
        }
        // IK results are committed with the pose (IK solving allocates anyway; without IK
        // this list stays empty and allocation-free).
        let mut results = Vec::new();
        for (id, ik) in &self.ik {
            let result = match self.rig.as_ref().and_then(|r| r.ik_target(id)) {
                Some(target) => ik.apply_to(&self.asset.skeleton, &mut self.accum, target),
                None => ik.apply(&self.asset.skeleton, &mut self.accum),
            };
            match result {
                Ok(r) => results.push(r),
                Err(e) => return at(&format!("/ik/{id}"), Err(e)),
            }
        }
        if let Some(rig) = &mut self.rig {
            rig.apply(&self.asset.skeleton, &mut self.accum)?;
        }
        self.asset
            .skeleton
            .globals_into(&self.accum, &mut self.globals)?;
        if let Some(rig) = &mut self.rig {
            rig.commit();
        }
        for ((id, _), result) in self.ik.iter().zip(results) {
            match self.ik_results.get_mut(id) {
                Some(slot) => *slot = result,
                None => {
                    self.ik_results.insert(id.clone(), result);
                }
            }
        }
        std::mem::swap(&mut self.pose, &mut self.accum);
        std::mem::swap(&mut self.sampled, &mut self.scratch);
        std::mem::swap(&mut self.morph_sampled, &mut self.morph_accum);
        self.compose_morphs();
        self.fault = None;
        Ok(())
    }
    fn inspect(&self, bones: &[String], parts: bool) -> Result<Value> {
        let globals = self.asset.skeleton.globals(&self.pose)?;
        let mut values = BTreeMap::new();
        for id in bones {
            let i = self.asset.skeleton.bone(id)?;
            values.insert(id.clone(),json!({"local":self.pose.local[i],"position":globals[i].w_axis.truncate(),"rotation":globals[i].to_scale_rotation_translation().1}));
        }
        let mut out = json!({"asset":self.config.asset,"clip":self.active,"time":self.time,"playing":self.playing,"speed":self.speed,"ramp":self.ramp.map(|(_, to, elapsed, duration)| json!({"to":to,"remaining":duration - elapsed})),"looping":self.looping,"bone_count":self.pose.local.len(),"vertices":self.asset.vertices.len(),"clips":self.clips.keys().collect::<Vec<_>>(),"retargeted":self.retarget.is_some(),"ik":self.ik_results,"fault":self.fault,"bones":values});
        if let (Some(a), Some(map)) = (&self.animator, out.as_object_mut()) {
            map.insert("controller".into(), a.describe());
        }
        self.describe_extras(&mut out);
        if self.unbound_morphs > 0 {
            out["unbound_morph_tracks"] = json!(self.unbound_morphs);
        }
        if parts {
            out["parts"] = Value::Array(
                self.asset
                    .materials
                    .iter()
                    .enumerate()
                    .map(|(i, m)| json!({"id":m.id,"groups":m.groups,"visible":!part_set(&self.hidden, i)}))
                    .collect(),
            );
        }
        Ok(out)
    }
    /// Morph weights and hidden parts, only when the asset has targets / something is hidden
    /// (observations of other players, and so `state_hash`, are unchanged).
    fn describe_extras(&self, out: &mut Value) {
        if !self.morph_weights.is_empty() {
            out["morphs"] = Value::Object(
                self.asset
                    .morphs
                    .targets
                    .iter()
                    .zip(&self.morph_weights)
                    .map(|(t, w)| (t.id.clone(), json!((w * 1000.0).round() / 1000.0)))
                    .collect(),
            );
        }
        if self.hidden != [0; 4] {
            out["hidden"] = json!(self.hidden_ids());
        }
    }
    fn hidden_ids(&self) -> Vec<&str> {
        self.asset
            .materials
            .iter()
            .enumerate()
            .filter(|(i, _)| part_set(&self.hidden, *i))
            .map(|(_, m)| m.id.as_str())
            .collect()
    }
    fn play(&mut self, a: PlayArgs) -> Result<()> {
        if let Some(c) = &self.animator {
            anyhow::bail!(
                "this entity is driven by animation controller {}; set its parameters (animation.param) instead of playing clips",
                c.id
            );
        }
        ensure!(
            self.clips.contains_key(&a.clip),
            "clip not found: {}",
            a.clip
        );
        self.fade = (a.fade_seconds > 0.).then(|| Fade {
            from: self.sampled.clone(),
            from_morph: self.morph_sampled.clone(),
            elapsed: 0.,
            duration: a.fade_seconds,
        });
        self.active = Some(a.clip);
        self.marker_fresh = true;
        self.time = 0.;
        self.playing = true;
        self.speed = a.speed;
        self.looping = a.looping;
        Ok(())
    }
}
pub struct AnimationSystem {
    players: BTreeMap<String, Controller>,
    /// `animation.library.markers` (clip -> markers), shared read-only.
    markers: BTreeMap<String, Vec<CMarker>>,
    /// `behavior.program` events the markers and `on_end` emit.
    events: Vec<EventInfo>,
    /// Marker / end events and cues fired since start.
    fired: u64,
    /// First failure to deliver an event (reported in metrics; simulation continues).
    emit_fault: Option<String>,
    /// The editor's last scrubbed pose: (entity, clip, seconds, pose). A playhead that did not
    /// move reuses it, so an idle preview neither clones nor evaluates per frame.
    scrubbed: Option<(String, String, f32, Pose)>,
    /// Reused list of this step's resolved rig targets.
    rig_points: Vec<Option<Vec3>>,
    /// `animation.choreography` stages and the running one (`animation.stage`).
    choreo: choreo::Choreo,
    /// `behavior.program` animation reads this system publishes each step.
    reads: Vec<choreo::ReadSlot>,
    /// `animation.parts` click capsules.
    parts: parts::Parts,
}

/// An entity posed by the editor's Animation timeline for one extraction: only the drawn
/// pose is swapped; [`AnimationSystem::end_scrub`] puts the simulation's pose back.
pub(crate) struct Scrub {
    entity: String,
    clip: String,
    time: f32,
    pose: Pose,
}

impl AnimationSystem {
    /// Entities driven by an animation controller, with its live state (editor overlay).
    pub(crate) fn animators(&self) -> impl Iterator<Item = (&str, &Animator)> {
        self.players
            .iter()
            .filter_map(|(id, p)| Some((id.as_str(), p.animator.as_ref()?)))
    }
    /// Poses `entity` at `seconds` of `clip` (clamped, not looping) until `end_scrub`. The
    /// simulation's controller state is untouched apart from the swapped pose; `None` when
    /// there is no such player or clip, or the pose cannot be evaluated.
    pub(crate) fn begin_scrub(&mut self, entity: &str, clip: &str, seconds: f32) -> Option<Scrub> {
        let p = self.players.get_mut(entity)?;
        let duration = p.clips.get(clip)?.source.duration;
        let time = if seconds.is_finite() {
            seconds.clamp(0.0, duration.max(0.0))
        } else {
            0.0
        };
        let (entity, clip, posed) = match self.scrubbed.take() {
            Some((e, c, t, pose)) if e == entity && c == clip && t == time => (e, c, pose),
            _ => {
                // A new playhead: evaluate on a copy (twists / IK / overrides apply as in play).
                let mut probe = p.clone();
                probe.active = Some(clip.to_owned());
                probe.time = time;
                probe.playing = false;
                probe.looping = false;
                probe.fade = None;
                // The timeline previews the clip itself, not the controller's choice.
                probe.animator = None;
                probe.evaluate(0.0).ok()?;
                (entity.to_owned(), clip.to_owned(), probe.pose)
            }
        };
        let pose = std::mem::replace(&mut p.pose, posed);
        Some(Scrub {
            entity,
            clip,
            time,
            pose,
        })
    }

    pub(crate) fn end_scrub(&mut self, scrub: Scrub) {
        if let Some(p) = self.players.get_mut(&scrub.entity) {
            let posed = std::mem::replace(&mut p.pose, scrub.pose);
            self.scrubbed = Some((scrub.entity, scrub.clip, scrub.time, posed));
        }
    }
}

/// Imported assets shared across runtime rebuilds (derived data cached outside revisions).
/// The key hashes the asset path, import options, the asset file bytes and the size and
/// modification time of every file under the asset's directory (the only files an importer
/// may read), so any input change misses. Bounded; unused entries are evicted first.
const ASSET_CACHE_ENTRIES: usize = 16;
const ASSET_CACHE_DIRECTORY_FILES: usize = 512;
static ASSET_CACHE: std::sync::Mutex<BTreeMap<u64, Arc<Asset>>> =
    std::sync::Mutex::new(BTreeMap::new());

fn asset_signature(source: &axiom_animation::authoring::Source) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let path = &source.path;
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 134_217_728 {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    serde_json::to_string(&source.options)
        .ok()?
        .hash(&mut hasher);
    std::fs::read(path).ok()?.hash(&mut hasher);
    let root = path.parent().unwrap_or(std::path::Path::new("."));
    let mut pending = vec![root.to_path_buf()];
    let mut files = vec![];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            let meta = entry.metadata().ok()?;
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                files.push((entry.path(), meta.len(), meta.modified().ok()));
            }
            if files.len() + pending.len() > ASSET_CACHE_DIRECTORY_FILES {
                return None;
            }
        }
    }
    files.sort();
    files.hash(&mut hasher);
    Some(hasher.finish())
}

fn load_asset(source: &axiom_animation::authoring::Source) -> Result<Arc<Asset>> {
    let Some(key) = asset_signature(source) else {
        return Ok(Arc::new(axiom_animation::import::load(
            &source.path,
            &source.options,
        )?));
    };
    if let Some(asset) = ASSET_CACHE.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Ok(asset);
    }
    let asset = Arc::new(axiom_animation::import::load(
        &source.path,
        &source.options,
    )?);
    if let Ok(mut cache) = ASSET_CACHE.lock() {
        if cache.len() >= ASSET_CACHE_ENTRIES {
            cache.retain(|_, a| Arc::strong_count(a) > 1);
        }
        if cache.len() < ASSET_CACHE_ENTRIES {
            cache.insert(key, asset.clone());
        }
    }
    Ok(asset)
}
pub fn register(registry: &mut RuntimeRegistry) {
    axiom_animation::authoring::register(&mut registry.authoring);
    choreo::register(&mut registry.authoring);
    parts::register(&mut registry.authoring);
    registry
        .factories
        .insert("animation.evaluate".into(), |state| {
            let lib: Library = state.compiled.source.resource("animation.library")?;
            let mut assets = BTreeMap::new();
            for (id, source) in &lib.assets {
                let path = format!(
                    "/resources/animation.library/assets/{}",
                    axiom_core::local::escape(id)
                );
                assets.insert(id.clone(), at(&path, load_asset(source))?);
            }
            ensure!(
                assets.values().map(|a| a.vertices.len()).sum::<usize>() <= 1_000_000,
                "total animation vertex budget exceeded"
            );
            let controllers: controller::Controllers = state
                .compiled
                .source
                .resources
                .get("animation.controllers")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?
                .unwrap_or_default();
            let mut players = BTreeMap::new();
            for e in &state.compiled.entities {
                if let Some(value) = e.components.get("animation.player") {
                    let config: Player = serde_json::from_value(value.clone())?;
                    let mut player = at(
                        &component_path(&e.id),
                        Controller::new(config, &lib, &assets),
                    )?;
                    if let Some(attach) = e.components.get("animation.controller") {
                        let path = controller_path(&e.id);
                        let result = serde_json::from_value::<controller::Attach>(attach.clone())
                            .map_err(anyhow::Error::from)
                            .and_then(|a| {
                                let def = controllers.controllers.get(&a.controller).with_context(
                                    || format!("animation controller {} missing", a.controller),
                                )?;
                                player.attach(&a.controller, def)
                            });
                        at(&path, result)?;
                    }
                    let model = rig::model_of(state, state.compiled.indices[&e.id]);
                    player.attach_rig(
                        &e.components,
                        &format!("/entities/@{}", axiom_core::local::escape(&e.id)),
                        model,
                    )?;
                    if player.rig.is_some() {
                        at(&component_path(&e.id), player.evaluate(0.))?;
                    }
                    players.insert(e.id.clone(), player);
                }
            }
            state.animation_cues.catalog = players
                .iter()
                .map(|(id, player)| {
                    (
                        id.clone(),
                        player
                            .clips
                            .iter()
                            .map(|(clip, motion)| (clip.clone(), motion.source.duration))
                            .collect(),
                    )
                })
                .collect();
            state.animation_cues.bone_names = players
                .iter()
                .map(|(id, p)| {
                    (
                        id.clone(),
                        p.asset.skeleton.indices.keys().cloned().collect(),
                    )
                })
                .collect();
            let (markers, events) = compile_markers(state, &lib, &mut players)?;
            let choreo = choreo::Choreo::build(state, &players)?;
            let reads = choreo::resolve_reads(state, &players)?;
            let parts = parts::Parts::build(state, &players)?;
            let system = AnimationSystem {
                players,
                markers,
                events,
                fired: 0,
                emit_fault: None,
                scrubbed: None,
                rig_points: vec![],
                choreo,
                reads,
                parts,
            };
            system.check_rig_targets(state)?;
            system.publish(state);
            Ok(Box::new(system))
        });
}
impl RuntimeSystem for AnimationSystem {
    fn finish_step(&mut self, state: &mut RuntimeState) {
        for cue in std::mem::take(&mut state.animation_cues.pending) {
            if let Some(player) = self.players.get_mut(&cue.entity) {
                if let Some(a) = &mut player.animator {
                    a.ignored_cues = a.ignored_cues.saturating_add(1);
                    continue;
                }
                state.animation_cues.finished.remove(&cue.entity);
                let result = player.play(cue).and_then(|_| player.evaluate(0.0));
                if let Err(error) = result {
                    player.fault = Some(format!("{error:#}"));
                }
            }
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn step(&mut self, state: &mut RuntimeState, _: &ActionMap) {
        for cue in std::mem::take(&mut state.animation_cues.pending) {
            if let Some(player) = self.players.get_mut(&cue.entity) {
                if let Some(a) = &mut player.animator {
                    a.ignored_cues = a.ignored_cues.saturating_add(1);
                    continue;
                }
                state.animation_cues.finished.remove(&cue.entity);
                if let Err(error) = player.play(cue) {
                    player.fault = Some(format!("{error:#}"));
                }
            }
        }
        self.resolve_rig_targets(state);
        for (id, p) in &mut self.players {
            let slot = state.compiled.indices[id];
            if !state.compiled.enabled[slot] {
                // Re-enabled entities start a fresh speed sample (no jump across the gap).
                if let Some(a) = &mut p.animator {
                    a.last_position = None;
                }
                continue;
            }
            // Advanced in place; a failed evaluation rolls back the (`Copy`) playback state
            // and keeps the last good pose, without cloning the player every step.
            let saved = (
                p.time,
                p.fade.as_ref().map(|f| f.elapsed),
                p.animator.as_ref().map(|a| a.state),
            );
            let saved_speed = (p.speed, p.ramp);
            if let Some((from, to, elapsed, duration)) = &mut p.ramp {
                *elapsed += FIXED_DT;
                let k = (*elapsed / *duration).min(1.0);
                p.speed = *from + (*to - *from) * k;
                if k >= 1.0 {
                    p.ramp = None;
                }
            }
            let position = state.transforms[slot].as_ref().map(|t| t.position);
            if let Some(a) = &mut p.animator {
                // Bound parameters read this step's movement (also while paused).
                let speed = match (a.last_position, position) {
                    (Some(a), Some(b)) => {
                        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
                        (dx * dx + dz * dz).sqrt() / FIXED_DT
                    }
                    _ => 0.0,
                };
                a.last_position = position;
                for i in 0..a.compiled.params.len() {
                    if a.compiled.param_source(i) == Some(controller::ParamSource::Speed) {
                        a.state.params[i] = speed;
                    }
                }
                if p.playing {
                    let dt = FIXED_DT * p.speed.max(0.0);
                    let snapshot = a.state;
                    let before = snapshot.current;
                    a.compiled.tick(&mut a.state, dt);
                    // Marker intervals in normalized time of the state's clip; a transition
                    // first finishes the left state's last step, then starts the new one at 0.
                    let after = a.state.current;
                    let prev = if p.marker_fresh { -1e-4 } else { before.frac };
                    p.crossing = Some(if a.state.fired != snapshot.fired {
                        let left = a.compiled.advanced(&snapshot, before, dt);
                        let laps = left.loops.saturating_sub(before.loops) as f32;
                        p.exit_crossing = Some((before.state, prev, left.frac + laps));
                        (-1e-4, after.normalized(), true)
                    } else {
                        let laps = after.loops.saturating_sub(before.loops) as f32;
                        (prev, after.frac + laps, true)
                    });
                }
            } else if p.playing {
                if p.speed > 0.0 && p.active.is_some() {
                    let prev = if p.marker_fresh { -1e-4 } else { p.time };
                    p.crossing = Some((prev, p.time.max(0.0) + FIXED_DT * p.speed, false));
                } else if p.speed < 0.0 {
                    // Markers fire forward only; a reverse run is no fresh start.
                    p.marker_fresh = false;
                }
                p.time += FIXED_DT * p.speed;
                if let Some(clip) = p.active.as_ref().and_then(|id| p.clips.get(id)) {
                    let duration = clip.source.duration;
                    if p.looping && duration > 0.0 {
                        p.time = p.time.rem_euclid(duration);
                    } else {
                        p.time = p.time.clamp(0.0, duration);
                    }
                }
            }
            if let Some(rig) = &mut p.rig {
                // Springs and look-at smoothing advance every step, also while paused.
                rig.dt = FIXED_DT;
            }
            let evaluated = p.evaluate(if p.playing { FIXED_DT } else { 0. });
            if let Some(rig) = &mut p.rig {
                rig.dt = 0.0;
            }
            if let Err(e) = evaluated {
                p.time = saved.0;
                (p.speed, p.ramp) = saved_speed;
                if let (Some(f), Some(elapsed)) = (&mut p.fade, saved.1) {
                    f.elapsed = elapsed;
                }
                p.crossing = None;
                p.exit_crossing = None;
                // The position read this step stays the reference for the next speed sample.
                if let (Some(a), Some(s)) = (&mut p.animator, saved.2) {
                    a.state = s;
                }
                p.fault = Some(format!("{e:#}"));
            } else if p.ramp_morphs(FIXED_DT) {
                p.compose_morphs();
            }
        }
        let stage_done = self.stage_cycles();
        self.fire_markers(state);
        if stage_done {
            self.advance_stage(state);
        }
        self.publish(state);
        if !state.animation_cues.socket_requests.is_empty() {
            let models = state.matrices();
            state.animation_cues.sockets.clear();
            for (entity, bones) in &state.animation_cues.socket_requests {
                if let Some(p) = self.players.get(entity) {
                    let slot = state.compiled.indices[entity];
                    if !state.compiled.enabled[slot] {
                        continue;
                    }
                    if let Ok(globals) = p.asset.skeleton.globals(&p.pose) {
                        for bone in bones {
                            if let Some(&i) = p.asset.skeleton.indices.get(bone) {
                                state.animation_cues.sockets.insert(
                                    (entity.clone(), bone.clone()),
                                    models[slot] * globals[i],
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    fn checkpoint_supported(&self) -> bool {
        true
    }
    fn uses_screen_pointer(&self) -> bool {
        self.parts.any()
    }
    fn screen_pointer(&mut self, state: &mut RuntimeState, pointer: &ScreenPointer) -> Result<()> {
        self.parts_pointer(state, pointer);
        Ok(())
    }
    fn save_state(&self, state: &RuntimeState) -> Option<Result<Value>> {
        let saved = SavedAnimation {
            players: self
                .players
                .iter()
                .map(|(id, p)| (id.clone(), p.save()))
                .collect(),
            fired: self.fired,
            emit_fault: self.emit_fault.clone(),
            finished: state.animation_cues.finished.iter().cloned().collect(),
            stage: self.choreo.save(),
            picks: self.parts.save(),
        };
        Some(serde_json::to_value(saved).map_err(Into::into))
    }
    fn load_state(&mut self, state: &mut RuntimeState, value: &Value) -> Result<()> {
        let saved: SavedAnimation = serde_json::from_value(value.clone())?;
        ensure!(
            saved.players.keys().eq(self.players.keys()),
            "checkpoint animation players do not match the content"
        );
        for (id, s) in &saved.players {
            let p = self.players.get_mut(id).expect("same keys");
            at(&component_path(id), p.load(s))?;
        }
        self.fired = saved.fired;
        self.emit_fault = saved.emit_fault;
        state.animation_cues.finished = saved
            .finished
            .into_iter()
            .filter(|id| self.players.contains_key(id))
            .collect();
        self.choreo.load(saved.stage.as_ref())?;
        self.parts.load(saved.picks.as_ref())?;
        self.publish(state);
        Ok(())
    }
    fn validate(&self, _: &RuntimeState) -> Result<()> {
        for p in self.players.values() {
            ensure!(p.fault.is_none(), "animation fault: {:?}", p.fault);
            p.asset.skeleton.globals(&p.pose)?;
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
        if capability != "animation" {
            return None;
        }
        Some((|| {
            if action == "stage" {
                return self.start_stage(state, choreo::stage_arg(args)?);
            }
            if action == "pick" {
                return self.pick_command(state, args);
            }
            if action == "inspect" {
                let entity = args
                    .get("entity")
                    .and_then(Value::as_str)
                    .context("entity required")?;
                let old = self
                    .players
                    .get(entity)
                    .with_context(|| format!("animation entity missing: {entity}"))?;
                let args: InspectArgs = serde_json::from_value(args.clone())?;
                return old.inspect(&args.bones, true);
            }
            let (entity, p) = self.prepare(state, action, args)?;
            if action == "play" {
                state.animation_cues.finished.remove(&entity);
            }
            let response = p.inspect(&[], false)?;
            self.players.insert(entity, p);
            self.publish(state);
            Ok(response)
        })())
    }
    fn check_command(
        &self,
        state: &RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<()>> {
        if capability != "animation" {
            return None;
        }
        Some(match action {
            "stage" => choreo::stage_arg(args).and_then(|s| self.stage_plan(s).map(drop)),
            "pick" => Ok(()),
            "inspect" => args
                .get("entity")
                .and_then(Value::as_str)
                .filter(|e| self.players.contains_key(*e))
                .map(drop)
                .context("animation entity missing"),
            _ => self.prepare(state, action, args).map(drop),
        })
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        if previous.compiled.source.id != next.compiled.source.id {
            return Ok(());
        }
        let old = old
            .as_any()
            .downcast_ref::<Self>()
            .context("animation transfer type")?;
        self.fired = old.fired;
        self.emit_fault.clone_from(&old.emit_fault);
        for (id, p) in &mut self.players {
            if let Some(previous) = old.players.get(id) {
                if p.config.asset == previous.config.asset
                    && p.config.retarget == previous.config.retarget
                {
                    p.time = previous.time;
                    // The same playback continues: markers already passed do not fire again.
                    p.marker_fresh = previous.marker_fresh;
                    if p.config.clip == previous.config.clip {
                        p.active = previous.active.clone();
                        p.playing = previous.playing;
                        if p.config.speed == previous.config.speed {
                            p.speed = previous.speed;
                            p.ramp = previous.ramp;
                        }
                        if p.config.looping == previous.config.looping {
                            p.looping = previous.looping;
                        }
                    }
                    // Runtime part visibility and morph overrides survive edits of the same
                    // asset; an edited `hidden` list is applied.
                    if p.asset.key == previous.asset.key {
                        p.morph_manual.clone_from(&previous.morph_manual);
                        if p.config.hidden == previous.config.hidden {
                            p.hidden = previous.hidden;
                        }
                    }
                    p.evaluate(0.)?;
                }
                if p.config.asset == previous.config.asset {
                    if let (Some(a), Some(b)) = (&mut p.rig, &previous.rig) {
                        a.carry(b);
                        p.evaluate(0.)?;
                    }
                }
                // A controller keeps its state and parameters by name across edits (live
                // graph editing in Play), as long as the entity keeps the same controller.
                if let (Some(a), Some(b)) = (&mut p.animator, &previous.animator) {
                    if a.id == b.id {
                        a.state = a.compiled.carry(&b.compiled, &b.state);
                        a.last_position = b.last_position;
                        a.ignored_cues = b.ignored_cues;
                        p.marker_fresh = previous.marker_fresh;
                        p.evaluate(0.)?;
                    }
                }
            }
        }
        self.choreo.transfer(&old.choreo);
        self.parts.transfer(&old.parts);
        self.publish(next);
        Ok(())
    }
    fn extract(&self, state: &RuntimeState, frame: &mut RenderFrame) {
        let models = state.matrices();
        for (id, p) in &self.players {
            let slot = state.compiled.indices[id];
            if !state.compiled.enabled[slot] {
                continue;
            }
            let model = models[slot];
            if let Ok(vertices) =
                skin::deform_with(&p.asset, &p.pose, p.config.skinning, &p.morph_weights)
            {
                frame.meshes.push(MeshFrame {
                    id: id.clone(),
                    asset: p.asset.clone(),
                    vertices: Arc::new(vertices),
                    model,
                    material: state.compiled.entities[slot]
                        .material
                        .as_ref()
                        .map(|m| m.id.clone()),
                    surface: state.compiled.entities[slot].surface,
                    hidden: p.hidden,
                });
            }
            if p.config.debug_skeleton {
                if let Ok(g) = p.asset.skeleton.globals(&p.pose) {
                    for (i, b) in p.asset.skeleton.source.bones.iter().enumerate() {
                        if let Some(parent) = b.parent {
                            let a = model.transform_point3(g[parent].w_axis.truncate());
                            let b = model.transform_point3(g[i].w_axis.truncate());
                            let d = b - a;
                            if d.length() > 0.001 {
                                let m = Mat4::from_scale_rotation_translation(
                                    Vec3::new(0.006, d.length(), 0.006),
                                    Quat::from_rotation_arc(Vec3::Y, d.normalize()),
                                    (a + b) * 0.5,
                                );
                                frame
                                    .instances
                                    .push(Instance::new(m, [1., 0.4, 0.1], 1., false));
                            }
                        }
                    }
                }
            }
            if let Some(rig) = &p.rig {
                rig.debug_draw(&p.asset.skeleton, &p.pose, model, &mut frame.instances);
            }
        }
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        let mut out = json!({"players":self.players.iter().map(|(id,p)|{let mut v=json!({"clip":p.active,"time":p.time,"playing":p.playing,"bones":p.pose.local.len(),"vertices":p.asset.vertices.len(),"retargeted":p.retarget.is_some(),"ik":p.ik_results,"fault":p.fault});if let Some(a)=&p.animator{v["controller"]=a.describe();}p.describe_extras(&mut v);
            if let Some(r)=&p.rig{v["rig"]=r.metrics();}(id.clone(),v)}).collect::<BTreeMap<_,_>>()});
        // Only when used, so worlds without markers keep their observation (and state_hash).
        if self.fired > 0 {
            out["markers_fired"] = json!(self.fired);
        }
        if let Some(f) = &self.emit_fault {
            out["emit_fault"] = json!(f);
        }
        if let Some(stage) = self.choreo.metrics() {
            out["stage"] = stage;
        }
        if let Some(pick) = self.parts.metrics() {
            out["pick"] = pick;
        }
        BTreeMap::from([("animation".into(), out)])
    }
}

impl AnimationSystem {
    /// `animation.ACTION args` applied to a copy of the entity's player (nothing changes
    /// here): the command commits it, `check_command` (batches) only checks it.
    fn prepare(
        &self,
        state: &RuntimeState,
        action: &str,
        args: &Value,
    ) -> Result<(String, Controller)> {
        let entity = args
            .get("entity")
            .and_then(Value::as_str)
            .context("entity required")?;
        let old = self
            .players
            .get(entity)
            .with_context(|| format!("animation entity missing: {entity}"))?;
        let mut p = old.clone();
        match action {
            "play" => p.play(serde_json::from_value(args.clone())?)?,
            "seek" => {
                let a: SeekArgs = serde_json::from_value(args.clone())?;
                p.time = a.seconds;
                p.playing = !a.paused;
                p.fade = None;
                // A jump fires nothing on the way; only a seek to 0 restarts the markers.
                p.marker_fresh = a.seconds == 0.0;
            }
            "ik" => {
                let a: IkArgs = serde_json::from_value(args.clone())?;
                let mut bound = None;
                let mut c =
                    p.ik.get(&a.id)
                        .context("IK constraint missing")?
                        .source
                        .clone();
                match (a.target, a.socket) {
                    (Some(target), None) => {
                        c.target = target;
                        c.socket = None;
                    }
                    (None, Some(socket)) => {
                        self.check_socket(state, &socket)?;
                        bound = self.socket_now(state, entity, &socket);
                        c.socket = Some(socket);
                    }
                    _ => anyhow::bail!("give either target or socket"),
                }
                if let Some(pole) = a.pole {
                    c.pole = pole
                }
                if let Some(weight) = a.weight {
                    c.weight = weight
                }
                p.ik.insert(a.id.clone(), Arc::new(c.compile(&p.asset.skeleton)?));
                p.rebind_ik();
                if let Some(rig) = &mut p.rig {
                    rig.set_ik_point(&a.id, bound);
                }
            }
            "morph" => {
                let a: MorphArgs = serde_json::from_value(args.clone())?;
                a.validate()?;
                p.morph(&a)?;
            }
            "visibility" => {
                let a: VisibilityArgs = serde_json::from_value(args.clone())?;
                let parts = p.asset.parts(&a.group);
                ensure!(
                    parts != [0; 4],
                    "no primitive named {} (parts: {})",
                    a.group,
                    part_names(&p.asset)
                );
                for (h, x) in p.hidden.iter_mut().zip(parts) {
                    if a.visible {
                        *h &= !x;
                    } else {
                        *h |= x;
                    }
                }
            }
            "twist" => {
                let a: TwistArgs = serde_json::from_value(args.clone())?;
                let mut c = p
                    .config
                    .twists
                    .get(&a.id)
                    .context("twist constraint missing")?
                    .clone();
                c.enabled = a.enabled;
                p.twists
                    .insert(a.id, Arc::new(c.compile(&p.asset.skeleton)?));
            }
            "bone" => {
                let a: BoneArgs = serde_json::from_value(args.clone())?;
                let i = p.asset.skeleton.bone(&a.bone)?;
                p.overrides.insert(i, a.rotation);
            }
            "speed" => {
                let a: SpeedArgs = serde_json::from_value(args.clone())?;
                if a.ramp_seconds > 0.0 {
                    p.ramp = Some((p.speed, a.speed, 0.0, a.ramp_seconds));
                } else {
                    p.speed = a.speed;
                    p.ramp = None;
                }
            }
            "param" => {
                let a: ParamArgs = serde_json::from_value(args.clone())?;
                let c = p.animator.as_mut().with_context(|| {
                    format!(
                        "{entity} has no animation.controller (parameters belong to controllers)"
                    )
                })?;
                c.compiled.set_param(&mut c.state, &a.name, a.value)?;
            }
            _ => anyhow::bail!("unknown animation action {action}"),
        }
        p.evaluate(0.)?;
        Ok((entity.to_owned(), p))
    }
}

/// A compiled `animation.library.markers` entry.
#[derive(Clone)]
struct CMarker {
    at: f32,
    event: Option<usize>,
    cue: Option<String>,
}

/// A `behavior.program` event the animation system emits, with the optional text fields it
/// fills (`entity`, `clip`) when the program declares them.
struct EventInfo {
    name: String,
    entity: bool,
    clip: bool,
}

fn event_index(
    events: &mut Vec<EventInfo>,
    program: Option<&Value>,
    name: &str,
    path: &str,
) -> Result<usize> {
    if let Some(i) = events.iter().position(|e| e.name == name) {
        return Ok(i);
    }
    let fields = program
        .and_then(|p| {
            p.pointer(&format!(
                "/events/{}/fields",
                axiom_core::local::escape(name)
            ))
        })
        .and_then(Value::as_object);
    let Some(fields) = fields else {
        return Err(axiom_core::Diagnostic::error(
            "undeclared_event",
            path,
            format!("animation emits {name}, which behavior.program does not declare"),
        ));
    };
    events.push(EventInfo {
        name: name.to_owned(),
        entity: fields.contains_key("entity"),
        clip: fields.contains_key("clip"),
    });
    Ok(events.len() - 1)
}

/// Markers by clip and the events they and `animation.player.on_end` emit.
type Markers = (BTreeMap<String, Vec<CMarker>>, Vec<EventInfo>);

/// Compiles the markers; unknown events, unknown audio cues and a missing
/// `behavior.execute` / `audio.events` system are errors.
fn compile_markers(
    state: &RuntimeState,
    lib: &Library,
    players: &mut BTreeMap<String, Controller>,
) -> Result<Markers> {
    let source = &state.compiled.source;
    let program = source.resources.get("behavior.program");
    let audio: Option<axiom_core::audio::AudioLibrary> = source
        .resources
        .get("audio.library")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?;
    let mut events = vec![];
    let mut markers = BTreeMap::new();
    let mut cues = false;
    for (clip, list) in &lib.markers {
        let path = format!(
            "/resources/animation.library/markers/{}",
            axiom_core::local::escape(clip)
        );
        let mut out = vec![];
        for m in list {
            let event = m
                .event
                .as_deref()
                .map(|e| event_index(&mut events, program, e, &path))
                .transpose()?;
            if let Some(cue) = &m.cue {
                cues = true;
                let known = audio
                    .as_ref()
                    .is_some_and(|a| a.cues.contains_key(cue) || a.clips.contains_key(cue));
                if !known {
                    return Err(axiom_core::Diagnostic::error(
                        "unknown_cue",
                        path,
                        format!("marker cue {cue} is not in audio.library"),
                    ));
                }
            }
            out.push(CMarker {
                at: m.at,
                event,
                cue: m.cue.clone(),
            });
        }
        markers.insert(clip.clone(), out);
    }
    // A marker past the end of a clip a player has would never fire.
    for (clip, list) in &markers {
        for p in players.values() {
            if let Some(c) = p.clips.get(clip) {
                let d = c.source.duration;
                if let Some(m) = list.iter().find(|m: &&CMarker| m.at > d + 1e-4) {
                    return Err(axiom_core::Diagnostic::error(
                        "marker_out_of_range",
                        format!(
                            "/resources/animation.library/markers/{}",
                            axiom_core::local::escape(clip)
                        ),
                        format!("marker at {} s is past the end of {clip} ({d} s)", m.at),
                    ));
                }
            }
        }
    }
    for (id, p) in players.iter_mut() {
        if let Some(e) = p.config.on_end.clone() {
            let path = format!("{}/on_end", component_path(id));
            p.on_end = Some(event_index(&mut events, program, &e, &path)?);
        }
    }
    ensure!(
        events.is_empty() || source.systems.iter().any(|s| s == "behavior.execute"),
        "animation marker / on_end events need the behavior.execute system"
    );
    ensure!(
        !cues || source.systems.iter().any(|s| s == "audio.events"),
        "animation marker cues need the audio.events system"
    );
    Ok((markers, events))
}

/// Calls `hit(Some(marker))` for every marker of a clip `duration` seconds long crossed
/// going from `prev` to `end` (seconds; unwrapped for loops; `prev < 0` includes time 0),
/// and `hit(None)` for every completed cycle. A loop fires each marker once per iteration
/// (at most 8 iterations per step); a one-shot stops at its end.
fn crossings(
    list: &[CMarker],
    duration: f32,
    looping: bool,
    prev: f32,
    end: f32,
    mut hit: impl FnMut(Option<&CMarker>),
) {
    if end <= prev || duration <= 0.0 || !end.is_finite() {
        return;
    }
    if !looping {
        let end = end.min(duration);
        for m in list.iter().filter(|m| prev < m.at && m.at <= end) {
            hit(Some(m));
        }
        if prev < duration && duration <= end {
            hit(None);
        }
        return;
    }
    let first = (prev.max(0.0) / duration).floor() as u32;
    let last = ((end / duration).floor() as u32).min(first + 8);
    for k in first..=last {
        let base = k as f32 * duration;
        if k >= 1 && prev < base && base <= end {
            hit(None);
        }
        for m in list.iter().filter(|m| m.at <= duration) {
            let t = base + m.at;
            if prev < t && t <= end {
                hit(Some(m));
            }
        }
    }
}

impl AnimationSystem {
    /// Delivers the markers and `on_end` events this step crossed and refreshes the
    /// one-shot `finished` set (FSM `clip_finished`). No allocation unless something fires.
    fn fire_markers(&mut self, state: &mut RuntimeState) {
        let tick = state.tick;
        let Self {
            players,
            markers,
            events,
            fired,
            emit_fault,
            ..
        } = self;
        let mut out = Delivery {
            state,
            events,
            fired,
            fault: emit_fault,
            tick,
        };
        for (id, p) in players.iter_mut() {
            let crossing = p.crossing.take();
            let exit = p.exit_crossing.take();
            if crossing.is_some() {
                p.marker_fresh = false;
            }
            let primary = match &p.animator {
                Some(a) => a.compiled.primary(&a.state),
                None => p.active.as_deref().and_then(|c| {
                    let d = p.clips.get(c)?.source.duration;
                    Some((c, d, p.looping))
                }),
            };
            let finished = match &p.animator {
                Some(a) => a.compiled.finished(&a.state),
                None => primary.is_some_and(|(_, d, looping)| !looping && p.time >= d),
            };
            let finished_set = &mut out.state.animation_cues.finished;
            let listed = finished_set.contains(id);
            if finished && !listed {
                finished_set.insert(id.clone());
            } else if !finished && listed {
                finished_set.remove(id);
            }
            // The state a transition left finishes its last step first.
            if let (Some((left, prev, end)), Some(a)) = (exit, &p.animator) {
                if let Some((clip, duration, looping)) = a.compiled.primary_of(&a.state, left) {
                    let list = markers.get(clip).map_or(&[][..], Vec::as_slice);
                    out.cross(
                        id,
                        clip,
                        list,
                        (duration, looping),
                        (prev, end, true),
                        p.on_end,
                    );
                }
            }
            let (Some(crossing), Some((clip, duration, looping))) = (crossing, primary) else {
                continue;
            };
            let list = markers.get(clip).map_or(&[][..], Vec::as_slice);
            out.cross(id, clip, list, (duration, looping), crossing, p.on_end);
        }
    }
}

/// Where marker / end events of one step go.
struct Delivery<'a> {
    state: &'a mut RuntimeState,
    events: &'a [EventInfo],
    fired: &'a mut u64,
    fault: &'a mut Option<String>,
    tick: u64,
}

impl Delivery<'_> {
    /// Fires what `crossing` (prev, end, normalized?) passes in `clip`.
    fn cross(
        &mut self,
        entity: &str,
        clip: &str,
        list: &[CMarker],
        (duration, looping): (f32, bool),
        (prev, end, normalized): (f32, f32, bool),
        on_end: Option<usize>,
    ) {
        if list.is_empty() && on_end.is_none() {
            return;
        }
        let scale = if normalized { duration } else { 1.0 };
        crossings(list, duration, looping, prev * scale, end * scale, |m| {
            let (event, cue) = match m {
                Some(m) => (m.event, m.cue.as_deref()),
                None => (on_end, None),
            };
            if let Some(cue) = cue {
                self.state.audio.emit(self.tick, cue);
                *self.fired += 1;
            }
            if let Some(e) = event {
                self.emit(e, entity, clip);
            }
        });
    }

    fn emit(&mut self, event: usize, entity: &str, clip: &str) {
        let info = &self.events[event];
        let mut fields = BTreeMap::new();
        if info.entity {
            fields.insert("entity".to_owned(), Scalar::Text(entity.to_owned()));
        }
        if info.clip {
            fields.insert("clip".to_owned(), Scalar::Text(clip.to_owned()));
        }
        let result = match self.state.gameplay.as_mut() {
            Some(g) => g.command(
                &RuntimeCommand::Emit {
                    event: info.name.clone(),
                    fields,
                },
                self.tick,
            ),
            None => Err(anyhow::anyhow!("behavior.execute is not running")),
        };
        match result {
            Ok(()) => *self.fired += 1,
            Err(e) => {
                self.fault
                    .get_or_insert_with(|| format!("emit {}: {e:#}", info.name));
            }
        }
    }
}

/// Checkpoint form of one player: everything `step` / commands change at runtime. Bones and
/// controller states are saved by name, so a mismatch fails loudly instead of misbinding.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedPlayer {
    clip: Option<String>,
    time: f32,
    playing: bool,
    speed: f32,
    looping: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ramp: Option<(f32, f32, f32, f32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade: Option<SavedFade>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    overrides: BTreeMap<String, Quat>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    ik: BTreeMap<String, axiom_animation::ik::TwoBoneIk>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    twists: BTreeMap<String, bool>,
    marker_fresh: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    controller: Option<SavedController>,
    /// `animation.morph` overrides by target name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    morphs: BTreeMap<String, SavedMorph>,
    /// Hidden primitives (material IDs) when they differ from the authored `hidden`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hidden: Option<Vec<String>>,
    /// Spring / look-at / IK socket state (players with a rig only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rig: Option<rig::SavedRig>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedMorph {
    weight: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ramp: Option<(f32, f32, f32, f32)>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedPhase {
    name: String,
    loops: u32,
    frac: f32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedFade {
    from: Vec<Trs>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    from_morph: Vec<f32>,
    elapsed: f32,
    duration: f32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedLayer {
    state: String,
    loops: u32,
    frac: f32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedController {
    id: String,
    current: SavedLayer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous: Option<SavedLayer>,
    fade: f32,
    fade_duration: f32,
    params: BTreeMap<String, f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_transition: Option<u16>,
    fired: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_position: Option<[f32; 3]>,
    ignored_cues: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    layers: Vec<SavedPhase>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedAnimation {
    players: BTreeMap<String, SavedPlayer>,
    #[serde(default)]
    fired: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    emit_fault: Option<String>,
    /// Entities whose one-shot clip had finished (FSM `clip_finished` reads it first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    finished: Vec<String>,
    /// `animation.choreography` state (worlds with stages; absent in older checkpoints).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stage: Option<choreo::SavedStage>,
    /// `animation.parts` click counters (worlds with parts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    picks: Option<parts::SavedPicks>,
}

fn save_layer(c: &controller::Compiled, l: &controller::Layer) -> SavedLayer {
    SavedLayer {
        state: c.states[l.state as usize].clone(),
        loops: l.loops,
        frac: l.frac,
    }
}

fn load_layer(c: &controller::Compiled, l: &SavedLayer) -> Result<controller::Layer> {
    let state = c
        .states
        .iter()
        .position(|s| *s == l.state)
        .with_context(|| format!("checkpoint names unknown controller state {}", l.state))?;
    ensure!(
        l.frac.is_finite() && (0.0..=1.0).contains(&l.frac),
        "invalid checkpoint controller time"
    );
    Ok(controller::Layer {
        state: state as u16,
        loops: l.loops,
        frac: l.frac,
    })
}

impl Controller {
    fn save(&self) -> SavedPlayer {
        let bones = &self.asset.skeleton.source.bones;
        SavedPlayer {
            clip: self.active.clone(),
            time: self.time,
            playing: self.playing,
            speed: self.speed,
            looping: self.looping,
            ramp: self.ramp,
            fade: self.fade.as_ref().map(|f| SavedFade {
                from: f.from.local.clone(),
                from_morph: f.from_morph.clone(),
                elapsed: f.elapsed,
                duration: f.duration,
            }),
            overrides: self
                .overrides
                .iter()
                .map(|(&i, &q)| (bones[i].id.clone(), q))
                .collect(),
            ik: self
                .ik
                .iter()
                .map(|(id, c)| (id.clone(), c.source.clone()))
                .collect(),
            twists: self
                .twists
                .iter()
                .map(|(id, t)| (id.clone(), t.source.enabled))
                .collect(),
            marker_fresh: self.marker_fresh,
            rig: self.rig.as_ref().map(|r| r.save()),
            controller: self.animator.as_ref().map(|a| {
                let c = &a.compiled;
                SavedController {
                    id: a.id.to_string(),
                    current: save_layer(c, &a.state.current),
                    previous: a.state.previous.as_ref().map(|l| save_layer(c, l)),
                    fade: a.state.fade,
                    fade_duration: a.state.fade_duration,
                    params: c
                        .params
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (n.clone(), a.state.params[i]))
                        .collect(),
                    last_transition: a.state.last_transition,
                    fired: a.state.fired,
                    last_position: a.last_position,
                    ignored_cues: a.ignored_cues,
                    layers: (0..c.layer_count())
                        .map(|i| SavedPhase {
                            name: c.layer_name(i).to_owned(),
                            loops: a.state.layers[i].loops,
                            frac: a.state.layers[i].frac,
                        })
                        .collect(),
                }
            }),
            morphs: self
                .morph_manual
                .iter()
                .zip(&self.asset.morphs.targets)
                .filter_map(|(m, t)| {
                    Some((
                        t.id.clone(),
                        SavedMorph {
                            weight: m.value?,
                            ramp: m.ramp,
                        },
                    ))
                })
                .collect(),
            hidden: (self.hidden != self.authored_hidden())
                .then(|| self.hidden_ids().into_iter().map(str::to_owned).collect()),
        }
    }

    fn load(&mut self, s: &SavedPlayer) -> Result<()> {
        let finite = |x: f32| x.is_finite();
        ensure!(
            finite(s.time) && finite(s.speed) && s.speed.abs() <= 8.0,
            "invalid checkpoint playback"
        );
        if let Some(clip) = &s.clip {
            ensure!(
                self.clips.contains_key(clip),
                "checkpoint clip {clip} not found"
            );
        }
        self.active = s.clip.clone();
        self.time = s.time;
        self.playing = s.playing;
        self.speed = s.speed;
        self.looping = s.looping;
        self.ramp = s.ramp;
        self.marker_fresh = s.marker_fresh;
        let bones = self.asset.skeleton.source.bones.len();
        self.fade = match &s.fade {
            Some(f) => {
                ensure!(
                    f.from.len() == bones
                        && finite(f.elapsed)
                        && finite(f.duration)
                        && f.duration > 0.0,
                    "invalid checkpoint fade"
                );
                for t in &f.from {
                    t.validate()?;
                }
                ensure!(
                    f.from_morph.is_empty() || f.from_morph.len() == self.morph_sampled.len(),
                    "checkpoint fade morph weights do not match the content"
                );
                ensure!(
                    f.from_morph.iter().all(|w| w.is_finite()),
                    "invalid checkpoint fade morph weight"
                );
                Some(Fade {
                    from: Pose {
                        local: f.from.clone(),
                    },
                    from_morph: if f.from_morph.is_empty() {
                        self.morph_sampled.clone()
                    } else {
                        f.from_morph.clone()
                    },
                    elapsed: f.elapsed,
                    duration: f.duration,
                })
            }
            None => None,
        };
        self.overrides.clear();
        for (bone, q) in &s.overrides {
            ensure!(
                q.is_finite() && (q.length_squared() - 1.0).abs() < 0.001,
                "invalid checkpoint bone override"
            );
            self.overrides.insert(self.asset.skeleton.bone(bone)?, *q);
        }
        ensure!(
            s.ik.keys().eq(self.ik.keys()) && s.twists.keys().eq(self.twists.keys()),
            "checkpoint IK / twist constraints do not match the content"
        );
        for (id, c) in &s.ik {
            self.ik
                .insert(id.clone(), Arc::new(c.compile(&self.asset.skeleton)?));
        }
        for (id, enabled) in &s.twists {
            let mut t = self.twists[id].source.clone();
            t.enabled = *enabled;
            self.twists
                .insert(id.clone(), Arc::new(t.compile(&self.asset.skeleton)?));
        }
        self.rebind_ik();
        rig::load_rig(self, s.rig.as_ref())?;
        match (&mut self.animator, &s.controller) {
            (None, None) => {}
            (Some(a), Some(saved)) => {
                ensure!(
                    *a.id == *saved.id,
                    "checkpoint controller {} does not match {}",
                    saved.id,
                    a.id
                );
                let c = a.compiled.clone();
                let mut state = c.start();
                state.current = load_layer(&c, &saved.current)?;
                state.previous = saved
                    .previous
                    .as_ref()
                    .map(|l| load_layer(&c, l))
                    .transpose()?;
                ensure!(
                    finite(saved.fade) && finite(saved.fade_duration),
                    "invalid checkpoint cross-fade"
                );
                state.fade = saved.fade;
                state.fade_duration = saved.fade_duration;
                ensure!(
                    saved.params.len() == c.params.len(),
                    "checkpoint controller parameters do not match the content"
                );
                for (name, v) in &saved.params {
                    let i = c
                        .param(name)
                        .with_context(|| format!("checkpoint names unknown parameter {name}"))?;
                    ensure!(v.is_finite(), "invalid checkpoint parameter {name}");
                    state.params[i] = *v;
                }
                state.last_transition = saved.last_transition;
                state.fired = saved.fired;
                ensure!(
                    saved.layers.len() == c.layer_count()
                        && saved
                            .layers
                            .iter()
                            .enumerate()
                            .all(|(i, l)| l.name == c.layer_name(i)),
                    "checkpoint controller layers do not match the content"
                );
                for (i, l) in saved.layers.iter().enumerate() {
                    ensure!(
                        l.frac.is_finite() && (0.0..=1.0).contains(&l.frac),
                        "invalid checkpoint layer time"
                    );
                    state.layers[i] = controller::Phase {
                        loops: l.loops,
                        frac: l.frac,
                    };
                }
                a.state = state;
                a.last_position = saved.last_position;
                a.ignored_cues = saved.ignored_cues;
            }
            _ => anyhow::bail!("checkpoint controller layout does not match the content"),
        }
        self.morph_manual.fill(ManualMorph::default());
        for (name, m) in &s.morphs {
            let i = self
                .asset
                .morphs
                .index(name)
                .with_context(|| format!("checkpoint names unknown morph target {name}"))?;
            ensure!(
                m.weight.is_finite()
                    && m.ramp.is_none_or(|(a, b, e, d)| {
                        [a, b, e, d].iter().all(|x| x.is_finite()) && d > 0.0
                    }),
                "invalid checkpoint morph weight {name}"
            );
            self.morph_manual[i] = ManualMorph {
                value: Some(m.weight),
                ramp: m.ramp,
            };
        }
        self.hidden = match &s.hidden {
            None => self.authored_hidden(),
            Some(ids) => {
                let mut mask = [0; 4];
                for id in ids {
                    let i = self
                        .asset
                        .materials
                        .iter()
                        .position(|m| m.id == *id)
                        .with_context(|| format!("checkpoint names unknown primitive {id}"))?;
                    mask[i / 64] |= 1 << (i % 64);
                }
                mask
            }
        };
        self.evaluate(0.0)
    }
    /// The parts `animation.player.hidden` hides (validated when the player was built).
    fn authored_hidden(&self) -> PartMask {
        let mut mask = [0; 4];
        for g in &self.config.hidden {
            for (h, p) in mask.iter_mut().zip(self.asset.parts(g)) {
                *h |= p;
            }
        }
        mask
    }
}

/// The primitives of `asset` for error messages: material IDs with their group names.
fn part_names(asset: &Asset) -> String {
    asset
        .materials
        .iter()
        .map(|m| {
            if m.groups.is_empty() {
                m.id.clone()
            } else {
                format!("{} ({})", m.id, m.groups.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}
