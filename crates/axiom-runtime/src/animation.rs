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
use axiom_core::local::at;
use glam::{Mat4, Quat, Vec3};
use serde_json::{json, Value};
use std::{any::Any, collections::BTreeMap, sync::Arc};

#[derive(Clone, Default)]
pub(crate) struct AnimationCues {
    pub pending: Vec<PlayArgs>,
    pub catalog: BTreeMap<String, BTreeMap<String, f32>>,
    pub bone_names: BTreeMap<String, std::collections::BTreeSet<String>>,
    pub socket_requests: BTreeMap<String, std::collections::BTreeSet<String>>,
    pub sockets: BTreeMap<(String, String), Mat4>,
}
#[derive(Clone)]
struct Fade {
    from: Pose,
    elapsed: f32,
    duration: f32,
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
    looping: bool,
    pose: Pose,
    sampled: Pose,
    fade: Option<Fade>,
    twists: BTreeMap<String, Arc<CompiledTwist>>,
    ik: BTreeMap<String, Arc<CompiledIk>>,
    overrides: BTreeMap<usize, Quat>,
    fault: Option<String>,
    ik_results: BTreeMap<String, IkResult>,
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
        let mut clips = BTreeMap::new();
        for c in &source.clips {
            ensure!(
                clips
                    .insert(c.id.clone(), c.compile(&source.skeleton)?)
                    .is_none(),
                "duplicate asset clip ID"
            );
        }
        for (id, c) in &lib.clips {
            if c.tracks
                .iter()
                .all(|t| source.skeleton.indices.contains_key(&t.bone))
            {
                ensure!(
                    clips
                        .insert(id.clone(), c.compile(&source.skeleton)?)
                        .is_none(),
                    "library clip shadows imported clip: {id}"
                );
            }
        }
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
            config,
        };
        out.evaluate(0.)?;
        Ok(out)
    }
    fn evaluate(&mut self, dt: f32) -> Result<()> {
        let source = if let Some(id) = &self.active {
            self.clips
                .get(id)
                .with_context(|| format!("clip not found: {id}"))?
                .sample(&self.source.skeleton, self.time, self.looping)?
        } else {
            self.source.skeleton.rest_pose()
        };
        let mut pose = if let Some(rt) = &self.retarget {
            rt.apply(&source, self.config.retarget.as_ref().unwrap().root_motion)?
        } else {
            source
        };
        if let Some(fade) = &mut self.fade {
            fade.elapsed += dt;
            let weight = (fade.elapsed / fade.duration).clamp(0., 1.);
            pose = Pose::blend(&fade.from, &pose, weight)?;
            if weight >= 1. {
                self.fade = None;
            }
        }
        let sampled = pose.clone();
        for (&i, &rotation) in &self.overrides {
            pose.local[i].rotation = rotation;
        }
        for (id, twist) in &self.twists {
            at(
                &format!("/twists/{id}"),
                twist.apply(&self.asset.skeleton, &mut pose),
            )?;
        }
        let mut results = BTreeMap::new();
        for (id, ik) in &self.ik {
            results.insert(
                id.clone(),
                at(
                    &format!("/ik/{id}"),
                    ik.apply(&self.asset.skeleton, &mut pose),
                )?,
            );
        }
        self.asset.skeleton.globals(&pose)?;
        self.pose = pose;
        self.sampled = sampled;
        self.ik_results = results;
        self.fault = None;
        Ok(())
    }
    fn inspect(&self, bones: &[String]) -> Result<Value> {
        let globals = self.asset.skeleton.globals(&self.pose)?;
        let mut values = BTreeMap::new();
        for id in bones {
            let i = self.asset.skeleton.bone(id)?;
            values.insert(id.clone(),json!({"local":self.pose.local[i],"position":globals[i].w_axis.truncate(),"rotation":globals[i].to_scale_rotation_translation().1}));
        }
        Ok(
            json!({"asset":self.config.asset,"clip":self.active,"time":self.time,"playing":self.playing,"speed":self.speed,"looping":self.looping,"bone_count":self.pose.local.len(),"vertices":self.asset.vertices.len(),"clips":self.clips.keys().collect::<Vec<_>>(),"retargeted":self.retarget.is_some(),"ik":self.ik_results,"fault":self.fault,"bones":values}),
        )
    }
    fn play(&mut self, a: PlayArgs) -> Result<()> {
        ensure!(
            self.clips.contains_key(&a.clip),
            "clip not found: {}",
            a.clip
        );
        self.fade = (a.fade_seconds > 0.).then(|| Fade {
            from: self.sampled.clone(),
            elapsed: 0.,
            duration: a.fade_seconds,
        });
        self.active = Some(a.clip);
        self.time = 0.;
        self.playing = true;
        self.speed = a.speed;
        self.looping = a.looping;
        Ok(())
    }
}
pub struct AnimationSystem {
    players: BTreeMap<String, Controller>,
    /// The editor's last scrubbed pose: (entity, clip, seconds, pose). A playhead that did not
    /// move reuses it, so an idle preview neither clones nor evaluates per frame.
    scrubbed: Option<(String, String, f32, Pose)>,
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
            let mut players = BTreeMap::new();
            for e in &state.compiled.entities {
                if let Some(value) = e.components.get("animation.player") {
                    let config: Player = serde_json::from_value(value.clone())?;
                    players.insert(
                        e.id.clone(),
                        at(
                            &component_path(&e.id),
                            Controller::new(config, &lib, &assets),
                        )?,
                    );
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
            Ok(Box::new(AnimationSystem {
                players,
                scrubbed: None,
            }))
        });
}
impl RuntimeSystem for AnimationSystem {
    fn finish_step(&mut self, state: &mut RuntimeState) {
        for cue in std::mem::take(&mut state.animation_cues.pending) {
            if let Some(player) = self.players.get_mut(&cue.entity) {
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
                if let Err(error) = player.play(cue) {
                    player.fault = Some(format!("{error:#}"));
                }
            }
        }
        for (id, p) in &mut self.players {
            if !state.compiled.enabled[state.compiled.indices[id]] {
                continue;
            }
            let mut next = p.clone();
            if next.playing {
                next.time += FIXED_DT * next.speed;
                if let Some(clip) = next.active.as_ref().and_then(|id| next.clips.get(id)) {
                    let duration = clip.source.duration;
                    if next.looping && duration > 0.0 {
                        next.time = next.time.rem_euclid(duration);
                    } else {
                        next.time = next.time.clamp(0.0, duration);
                    }
                }
            }
            match next.evaluate(if next.playing { FIXED_DT } else { 0. }) {
                Ok(()) => *p = next,
                Err(e) => p.fault = Some(format!("{e:#}")),
            }
        }
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
    fn validate(&self, _: &RuntimeState) -> Result<()> {
        for p in self.players.values() {
            ensure!(p.fault.is_none(), "animation fault: {:?}", p.fault);
            p.asset.skeleton.globals(&p.pose)?;
        }
        Ok(())
    }
    fn command(
        &mut self,
        _: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "animation" {
            return None;
        }
        Some((|| {
            let entity = args
                .get("entity")
                .and_then(Value::as_str)
                .context("entity required")?;
            let old = self
                .players
                .get(entity)
                .with_context(|| format!("animation entity missing: {entity}"))?;
            if action == "inspect" {
                let args: InspectArgs = serde_json::from_value(args.clone())?;
                return old.inspect(&args.bones);
            }
            let mut p = old.clone();
            match action {
                "play" => {
                    p.play(serde_json::from_value(args.clone())?)?;
                }
                "seek" => {
                    let a: SeekArgs = serde_json::from_value(args.clone())?;
                    p.time = a.seconds;
                    p.playing = !a.paused;
                    p.fade = None;
                }
                "ik" => {
                    let a: IkArgs = serde_json::from_value(args.clone())?;
                    let mut c =
                        p.ik.get(&a.id)
                            .context("IK constraint missing")?
                            .source
                            .clone();
                    c.target = a.target;
                    if let Some(pole) = a.pole {
                        c.pole = pole
                    }
                    if let Some(weight) = a.weight {
                        c.weight = weight
                    }
                    p.ik.insert(a.id, Arc::new(c.compile(&p.asset.skeleton)?));
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
                _ => anyhow::bail!("unknown animation action {action}"),
            }
            p.evaluate(0.)?;
            let response = p.inspect(&[])?;
            self.players.insert(entity.to_owned(), p);
            Ok(response)
        })())
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
        for (id, p) in &mut self.players {
            if let Some(previous) = old.players.get(id) {
                if p.config.asset == previous.config.asset
                    && p.config.retarget == previous.config.retarget
                {
                    p.time = previous.time;
                    if p.config.clip == previous.config.clip {
                        p.active = previous.active.clone();
                        p.playing = previous.playing;
                        if p.config.speed == previous.config.speed {
                            p.speed = previous.speed;
                        }
                        if p.config.looping == previous.config.looping {
                            p.looping = previous.looping;
                        }
                    }
                    p.evaluate(0.)?;
                }
            }
        }
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
            if let Ok(vertices) = skin::deform(&p.asset, &p.pose, p.config.skinning) {
                frame.meshes.push(MeshFrame {
                    id: id.clone(),
                    asset: p.asset.clone(),
                    vertices: Arc::new(vertices),
                    model,
                    material: state.compiled.entities[slot]
                        .material
                        .as_ref()
                        .map(|m| m.id.clone()),
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
        }
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([(
            "animation".into(),
            json!({"players":self.players.iter().map(|(id,p)|(id.clone(),json!({"clip":p.active,"time":p.time,"playing":p.playing,"bones":p.pose.local.len(),"vertices":p.asset.vertices.len(),"retargeted":p.retarget.is_some(),"ik":p.ik_results,"fault":p.fault}))).collect::<BTreeMap<_,_>>()}),
        )])
    }
}
