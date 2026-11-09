//! `tween.animate`: runtime side of `tween.clips` (see `axiom_core::tween`).
//!
//! Every track is resolved once, when the runtime is built, to a typed write (entity slot +
//! field, or gameplay variable index) and its keys are copied into flat arrays shared by all
//! components (times, values with 1 or 3 floats per key, eases). Each fixed step advances the
//! playing components and samples their current clip's tracks with a cached segment hint per
//! track (binary search only after a jump). A steady step does not allocate; finished `once`
//! clips queue their `on_finish` event (at most `MAX_TWEEN_EVENTS_PER_STEP` per step, the rest
//! on the next steps) through the same gameplay command path as `path.follow` arrivals.
use crate::world::*;
use anyhow::{bail, ensure, Context, Result};
use axiom_core::{
    gameplay::{RuntimeCommand, Scalar},
    tween::*,
};
use glam::Quat;
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
};

/// Components listed with clip/time/state in `observe` (counts are always complete).
const OBSERVED: usize = 32;

/// Where a sampled value goes. Entity slots index `RuntimeState` columns.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Write {
    Position(usize),
    Rotation(usize),
    Yaw(usize),
    Scale(usize),
    LightColor(usize),
    LightIntensity(usize),
    MeshColor(usize),
    MeshEmission(usize),
    /// Gameplay variable index; `true` = integer variable (rounded).
    Variable(usize, bool),
}

struct Track {
    /// First key in the shared `times` / `eases` arrays, and the key count.
    first: usize,
    count: usize,
    /// First float in the shared `values` array (`dims` per key).
    values: usize,
    dims: usize,
    write: Write,
    /// Segment of the last sample (see `axiom_core::tween::sample`).
    hint: usize,
}

struct Clip {
    name: String,
    first: usize,
    count: usize,
    length: f32,
    mode: TweenMode,
    speed: f32,
    on_finish: Option<usize>,
    /// Authored clip, compared on hot reload.
    def: TweenClip,
}

struct Player {
    slot: usize,
    first: usize,
    count: usize,
    enabled: bool,
    /// Global clip index of the current clip.
    current: Option<usize>,
    time: f32,
    forward: bool,
    playing: bool,
    /// The current clip's values were written at least once (re-applied after hot reload).
    applied: bool,
    /// Waiting in the `on_finish` queue.
    queued: bool,
}

struct EventInfo {
    name: String,
    entity_field: bool,
    clip_field: bool,
}

pub struct TweenSystem {
    players: Vec<Player>,
    ids: Vec<String>,
    clips: Vec<Clip>,
    tracks: Vec<Track>,
    times: Vec<f32>,
    values: Vec<f32>,
    eases: Vec<Ease>,
    events: Vec<EventInfo>,
    /// (player, clip) of finished clips that emit `on_finish` (capacity = players: a player
    /// waits at most once).
    queue: VecDeque<(usize, usize)>,
    emitted: u64,
    scanned: usize,
    /// Set when an unsupported runtime spawn appears; the system stops and observe says why.
    halted: Option<String>,
    fault: Option<String>,
}

/// A value the editor's scrub preview writes for one frame.
#[derive(Clone, Copy, Debug)]
pub struct PreviewWrite(Write, [f32; 3]);

/// What a preview replaced; `restore` puts it back after the frame is extracted.
#[derive(Default)]
pub struct PreviewRestore {
    transforms: Vec<(usize, Option<axiom_core::Transform>)>,
    lights: Vec<(usize, Option<axiom_core::light::RenderLight>)>,
    meshes: Vec<(usize, Option<axiom_core::RenderMesh>)>,
    variables: Vec<(usize, Scalar)>,
}

impl PreviewRestore {
    pub fn restore(self, state: &mut RuntimeState) {
        for (slot, t) in self.transforms.into_iter().rev() {
            state.transforms[slot] = t;
        }
        for (slot, l) in self.lights.into_iter().rev() {
            state.compiled.entities[slot].light = l;
        }
        for (slot, m) in self.meshes.into_iter().rev() {
            state.compiled.entities[slot].mesh = m;
        }
        if let Some(g) = state.gameplay.as_mut() {
            for (i, v) in self.variables.into_iter().rev() {
                g.values[i] = v;
            }
        }
    }
}

fn write(state: &mut RuntimeState, target: Write, v: &[f32; 3]) {
    fn transform(state: &mut RuntimeState, slot: usize) -> Option<&mut axiom_core::Transform> {
        state.transforms[slot].as_mut()
    }
    match target {
        Write::Position(slot) => {
            if let Some(t) = transform(state, slot) {
                t.position = *v;
            }
        }
        Write::Rotation(slot) => {
            if let Some(t) = transform(state, slot) {
                t.rotation = euler_rotation(*v).to_array();
            }
        }
        Write::Yaw(slot) => {
            if let Some(t) = transform(state, slot) {
                t.rotation = Quat::from_rotation_y(v[0].to_radians())
                    .normalize()
                    .to_array();
            }
        }
        Write::Scale(slot) => {
            if let Some(t) = transform(state, slot) {
                t.scale = *v;
            }
        }
        Write::LightColor(slot) | Write::LightIntensity(slot) => {
            if let Some(l) = state.compiled.entities[slot].light.as_mut() {
                if matches!(target, Write::LightColor(_)) {
                    l.color = *v;
                } else {
                    l.intensity = v[0];
                }
            }
        }
        Write::MeshColor(slot) | Write::MeshEmission(slot) => {
            if let Some(m) = state.compiled.entities[slot].mesh.as_mut() {
                for p in &mut m.parts {
                    if matches!(target, Write::MeshColor(_)) {
                        p.color = *v;
                    } else {
                        p.emission = v[0];
                    }
                }
            }
        }
        Write::Variable(i, int) => {
            if let Some(g) = state.gameplay.as_mut() {
                g.values[i] = if int {
                    Scalar::Int(v[0].round() as i64)
                } else {
                    Scalar::Number(f64::from(v[0]))
                };
            }
        }
    }
}

/// Display rounding for observe (f64 so the JSON shows the short decimal).
fn round(v: f32) -> f64 {
    (f64::from(v) * 10000.0).round() / 10000.0
}

impl TweenSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let compiled = &state.compiled;
        let program = compiled.source.resources.get("behavior.program");
        let mut system = Self {
            players: vec![],
            ids: vec![],
            clips: vec![],
            tracks: vec![],
            times: vec![],
            values: vec![],
            eases: vec![],
            events: vec![],
            queue: VecDeque::new(),
            emitted: 0,
            scanned: compiled.len(),
            halted: None,
            fault: None,
        };
        for (slot, e) in compiled.entities.iter().enumerate() {
            let Some(def) = &e.tween else { continue };
            def.validate()
                .with_context(|| format!("tween.clips {}", e.id))?;
            ensure!(
                system.players.len() < MAX_TWEENS,
                "tween component budget exceeded (> {MAX_TWEENS})"
            );
            let first = system.clips.len();
            for (name, clip) in &def.clips {
                let first_track = system.tracks.len();
                for track in &clip.tracks {
                    let property = Property::parse(&track.property)?;
                    let target = if track.target.is_empty() {
                        slot
                    } else {
                        *compiled
                            .indices
                            .get(&track.target)
                            .with_context(|| format!("tween target {} not found", track.target))?
                    };
                    let target_write = match &property {
                        Property::Position => Write::Position(target),
                        Property::Rotation => Write::Rotation(target),
                        Property::Yaw => Write::Yaw(target),
                        Property::Scale => Write::Scale(target),
                        Property::LightColor => Write::LightColor(target),
                        Property::LightIntensity => Write::LightIntensity(target),
                        Property::MeshColor => Write::MeshColor(target),
                        Property::MeshEmission => Write::MeshEmission(target),
                        Property::Variable(name) => {
                            let g = state
                                .gameplay
                                .as_ref()
                                .context("variable tracks need the behavior.execute system")?;
                            let i = g
                                .compiled
                                .variables
                                .iter()
                                .position(|v| v == name)
                                .with_context(|| {
                                    format!("behavior.program declares no variable {name}")
                                })?;
                            let int = match &g.compiled.initial[i] {
                                Scalar::Int(_) => true,
                                Scalar::Number(_) => false,
                                _ => bail!("variable {name} is not a number"),
                            };
                            Write::Variable(i, int)
                        }
                    };
                    let dims = property.dims();
                    ensure!(
                        system.times.len() + track.keys.len() <= MAX_WORLD_KEYS,
                        "tween key budget exceeded (> {MAX_WORLD_KEYS})"
                    );
                    system.tracks.push(Track {
                        first: system.times.len(),
                        count: track.keys.len(),
                        values: system.values.len(),
                        dims,
                        write: target_write,
                        hint: 0,
                    });
                    for key in &track.keys {
                        system.times.push(key.t);
                        system.eases.push(key.ease);
                        system.values.extend_from_slice(key.v.as_slice());
                    }
                }
                ensure!(
                    system.tracks.len() <= MAX_WORLD_TRACKS,
                    "tween track budget exceeded (> {MAX_WORLD_TRACKS})"
                );
                let on_finish = if clip.on_finish.is_empty() {
                    None
                } else if let Some(i) = system.events.iter().position(|v| v.name == clip.on_finish)
                {
                    Some(i)
                } else {
                    let fields = program
                        .and_then(|p| {
                            p.pointer(&format!(
                                "/events/{}/fields",
                                axiom_core::local::escape(&clip.on_finish)
                            ))
                        })
                        .and_then(Value::as_object)
                        .with_context(|| {
                            format!(
                                "tween clip emits {}, which behavior.program does not declare",
                                clip.on_finish
                            )
                        })?;
                    system.events.push(EventInfo {
                        name: clip.on_finish.clone(),
                        entity_field: fields.contains_key("entity"),
                        clip_field: fields.contains_key("clip"),
                    });
                    Some(system.events.len() - 1)
                };
                system.clips.push(Clip {
                    name: name.clone(),
                    first: first_track,
                    count: clip.tracks.len(),
                    length: clip.length(),
                    mode: clip.mode,
                    speed: clip.speed,
                    on_finish,
                    def: clip.clone(),
                });
            }
            let autoplay = def
                .clips
                .values()
                .position(|c| c.autoplay)
                .map(|k| first + k);
            system.players.push(Player {
                slot,
                first,
                count: def.clips.len(),
                enabled: def.enabled,
                current: autoplay,
                time: 0.0,
                forward: true,
                playing: autoplay.is_some() && def.enabled,
                applied: false,
                queued: false,
            });
            system.ids.push(e.id.clone());
        }
        if !system.events.is_empty() {
            ensure!(
                state
                    .compiled
                    .source
                    .systems
                    .iter()
                    .any(|s| s == "behavior.execute"),
                "tween on_finish events need the behavior.execute system"
            );
        }
        system.queue.reserve(system.players.len());
        // Autoplay clips write nothing until the first step: an edit-mode runtime (never
        // stepped) keeps showing the authored values.
        Ok(system)
    }

    /// Samples the current clip of player `p` at its time and writes every track.
    fn apply(&mut self, state: &mut RuntimeState, p: usize) {
        let player = &mut self.players[p];
        let Some(c) = player.current else { return };
        player.applied = true;
        let time = player.time;
        let clip = &self.clips[c];
        let mut out = [0.0f32; 3];
        for track in &mut self.tracks[clip.first..clip.first + clip.count] {
            let keys = track.first..track.first + track.count;
            sample(
                &self.times[keys.clone()],
                &self.values[track.values..track.values + track.count * track.dims],
                &self.eases[keys],
                track.dims,
                time,
                &mut track.hint,
                &mut out,
            );
            write(state, track.write, &out);
        }
    }

    fn emit(&mut self, state: &mut RuntimeState, p: usize, c: usize) {
        let Some(event) = self.clips[c].on_finish else {
            return;
        };
        let info = &self.events[event];
        let mut fields = BTreeMap::new();
        if info.entity_field {
            fields.insert("entity".to_owned(), Scalar::Text(self.ids[p].clone()));
        }
        if info.clip_field {
            fields.insert("clip".to_owned(), Scalar::Text(self.clips[c].name.clone()));
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
            // Keep animating; surface the first failure through observe.
            Err(e) => {
                let name = &self.events[event].name;
                self.fault
                    .get_or_insert_with(|| format!("emit {name}: {e:#}"));
            }
        }
    }

    fn find(&self, args: &Value) -> Result<usize> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .context("tween actions need {\"id\":\"entity with tween.clips\"}")?;
        self.ids.iter().position(|x| x == id).ok_or_else(|| {
            axiom_core::Diagnostic::error(
                "not_found",
                "/args/id",
                format!("{id} has no tween.clips"),
            )
        })
    }

    fn clip_index(&self, p: usize, name: &str) -> Result<usize> {
        let player = &self.players[p];
        (player.first..player.first + player.count)
            .find(|&c| self.clips[c].name == name)
            .ok_or_else(|| {
                let names: Vec<&str> = (player.first..player.first + player.count)
                    .map(|c| self.clips[c].name.as_str())
                    .collect();
                axiom_core::Diagnostic::error(
                    "not_found",
                    "/args/clip",
                    format!(
                        "{} has no clip {name} (clips: {})",
                        self.ids[p],
                        names.join(", ")
                    ),
                )
            })
    }

    fn state_name(player: &Player) -> &'static str {
        match (player.current, player.playing, player.forward) {
            (None, _, _) => "idle",
            (Some(_), true, true) => "playing",
            (Some(_), true, false) => "reversing",
            (Some(_), false, _) => "stopped",
        }
    }

    fn player_json(&self, p: usize) -> Value {
        let player = &self.players[p];
        json!({
            "id":self.ids[p],
            "clip":player.current.map(|c| &self.clips[c].name),
            "time":round(player.time),
            "length":player.current.map(|c| round(self.clips[c].length)),
            "state":Self::state_name(player),
            "enabled":player.enabled,
        })
    }

    fn inspect(&self, p: usize) -> Value {
        let player = &self.players[p];
        let mut out = self.player_json(p);
        out["clips"] = (player.first..player.first + player.count)
            .map(|c| {
                let clip = &self.clips[c];
                json!({
                    "name":clip.name,
                    "length":round(clip.length),
                    "mode":clip.mode,
                    "speed":clip.speed,
                    "autoplay":clip.def.autoplay,
                    "on_finish":clip.def.on_finish,
                    "tracks":clip.def.tracks.iter().map(|t| json!([t.target, t.property, t.keys.len()])).collect::<Vec<_>>(),
                })
            })
            .collect();
        out
    }

    fn summary(&self) -> Value {
        let at: serde_json::Map<String, Value> = self
            .players
            .iter()
            .zip(&self.ids)
            .filter(|(p, _)| p.current.is_some())
            .take(OBSERVED)
            .map(|(p, id)| {
                let c = p.current.unwrap();
                (
                    id.clone(),
                    json!([self.clips[c].name, round(p.time), Self::state_name(p)]),
                )
            })
            .collect();
        let active = self.players.iter().filter(|p| p.current.is_some()).count();
        json!({
            "components":self.players.len(),
            "playing":self.players.iter().filter(|p| p.playing).count(),
            "at":at,
            "more":active.saturating_sub(OBSERVED),
            "emitted":self.emitted,
            "queued":self.queue.len(),
            "halted":self.halted,
            "fault":self.fault,
        })
    }

    /// Clip, time and playing flag of the component on entity `id` (editor timeline).
    pub fn playback(&self, id: &str) -> Option<(&str, f32, bool)> {
        let p = self.ids.iter().position(|x| x == id)?;
        let player = &self.players[p];
        let c = player.current?;
        Some((self.clips[c].name.as_str(), player.time, player.playing))
    }

    /// The values clip `clip` of entity `id` has at `time` (editor scrub preview; nothing
    /// is written). `None` when the runtime has no such component or clip.
    pub fn preview(&self, id: &str, clip: &str, time: f32) -> Option<Vec<PreviewWrite>> {
        let p = self.ids.iter().position(|x| x == id)?;
        let c = self.clip_index(p, clip).ok()?;
        let clip = &self.clips[c];
        let time = time.clamp(0.0, clip.length);
        let mut out = vec![];
        for track in &self.tracks[clip.first..clip.first + clip.count] {
            let keys = track.first..track.first + track.count;
            let mut v = [0.0f32; 3];
            let mut hint = 0;
            sample(
                &self.times[keys.clone()],
                &self.values[track.values..track.values + track.count * track.dims],
                &self.eases[keys],
                track.dims,
                time,
                &mut hint,
                &mut v,
            );
            out.push(PreviewWrite(track.write, v));
        }
        Some(out)
    }

    /// Writes preview values into `state`, returning what they replaced.
    pub fn apply_preview(state: &mut RuntimeState, writes: &[PreviewWrite]) -> PreviewRestore {
        let mut restore = PreviewRestore::default();
        for &PreviewWrite(target, v) in writes {
            match target {
                Write::Position(s) | Write::Rotation(s) | Write::Yaw(s) | Write::Scale(s) => {
                    restore.transforms.push((s, state.transforms[s].clone()));
                }
                Write::LightColor(s) | Write::LightIntensity(s) => {
                    restore
                        .lights
                        .push((s, state.compiled.entities[s].light.clone()));
                }
                Write::MeshColor(s) | Write::MeshEmission(s) => {
                    restore
                        .meshes
                        .push((s, state.compiled.entities[s].mesh.clone()));
                }
                Write::Variable(i, _) => {
                    if let Some(g) = &state.gameplay {
                        restore.variables.push((i, g.values[i].clone()));
                    }
                }
            }
            write(state, target, &v);
        }
        restore
    }
}

impl RuntimeSystem for TweenSystem {
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
                .find(|e| e.tween.is_some())
            {
                self.halted = Some(format!(
                    "runtime-spawned entity {} has tween.clips; spawned tweens are not supported (author them instead)",
                    e.id
                ));
                return;
            }
            self.scanned = len;
        }
        for p in 0..self.players.len() {
            let player = &mut self.players[p];
            if !player.playing || !player.enabled || !state.compiled.enabled[player.slot] {
                continue;
            }
            let Some(c) = player.current else { continue };
            let clip = &self.clips[c];
            let (time, forward, finished) = advance(
                player.time,
                player.forward,
                clip.speed * FIXED_DT,
                clip.length,
                clip.mode,
            );
            (player.time, player.forward) = (time, forward);
            if finished {
                player.playing = false;
                if clip.on_finish.is_some() && !player.queued {
                    player.queued = true;
                    self.queue.push_back((p, c));
                }
            }
            self.apply(state, p);
        }
        if self.queue.is_empty() {
            return;
        }
        for _ in 0..MAX_TWEEN_EVENTS_PER_STEP {
            let Some((p, c)) = self.queue.pop_front() else {
                break;
            };
            self.players[p].queued = false;
            self.emit(state, p, c);
        }
        state.sync_scenes();
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        // Hot reload keeps playback per component entity ID when its current clip still exists
        // with the same definition (same name, tracks, keys and options); the values are then
        // written again because the new runtime starts from the authored ones.
        let Some(old) = old.as_any().downcast_ref::<Self>() else {
            return Ok(());
        };
        let previous: BTreeMap<&str, usize> = old
            .ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let mut kept = vec![None; old.players.len()];
        for p in 0..self.players.len() {
            let Some(&o) = previous.get(self.ids[p].as_str()) else {
                continue;
            };
            let op = &old.players[o];
            let Some(oc) = op.current else { continue };
            let old_clip = &old.clips[oc];
            let Ok(c) = self.clip_index(p, &old_clip.name) else {
                continue;
            };
            if self.clips[c].def != old_clip.def {
                continue;
            }
            let player = &mut self.players[p];
            player.current = Some(c);
            player.time = op.time;
            player.forward = op.forward;
            player.playing = op.playing && player.enabled;
            kept[o] = Some(p);
            if op.applied || player.playing {
                self.apply(next, p);
            }
        }
        // Pending on_finish events survive when the finished clip is unchanged.
        for &(o, oc) in &old.queue {
            let Some(p) = kept[o] else { continue };
            let Ok(c) = self.clip_index(p, &old.clips[oc].name) else {
                continue;
            };
            if self.clips[c].def == old.clips[oc].def && !self.players[p].queued {
                self.players[p].queued = true;
                self.queue.push_back((p, c));
            }
        }
        self.emitted = old.emitted;
        self.fault = old.fault.clone();
        Ok(())
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "tween" {
            return None;
        }
        Some((|| {
            if action == "summary" {
                return Ok(self.summary());
            }
            let p = self.find(args)?;
            if action == "inspect" {
                return Ok(self.inspect(p));
            }
            if let Some(reason) = &self.halted {
                bail!("tween.animate is halted: {reason}");
            }
            ensure!(
                self.players[p].enabled,
                "{} has a disabled tween.clips (enabled false)",
                self.ids[p]
            );
            match action {
                "play" => {
                    let name = args
                        .get("clip")
                        .and_then(Value::as_str)
                        .context("tween.play needs {\"id\":..,\"clip\":NAME}")?;
                    let c = self.clip_index(p, name)?;
                    let reverse = args.get("reverse").and_then(Value::as_bool) == Some(true);
                    let restart = args.get("restart").and_then(Value::as_bool) == Some(true);
                    let length = self.clips[c].length;
                    let once = self.clips[c].mode == TweenMode::Once;
                    let player = &mut self.players[p];
                    if player.current != Some(c) || restart {
                        player.current = Some(c);
                        player.time = if reverse { length } else { 0.0 };
                        player.forward = !reverse;
                        player.playing = true;
                        self.apply(state, p);
                    } else {
                        player.forward = !reverse;
                        // A once clip already at the requested end has nothing to play.
                        let at_end = if reverse {
                            player.time <= 0.0
                        } else {
                            player.time >= length
                        };
                        player.playing = !(once && at_end);
                    }
                }
                "stop" => self.players[p].playing = false,
                "seek" => {
                    let time = args
                        .get("time")
                        .and_then(Value::as_f64)
                        .context("tween.seek needs {\"id\":..,\"time\":seconds}")?
                        as f32;
                    let c = match args.get("clip").and_then(Value::as_str) {
                        Some(name) => self.clip_index(p, name)?,
                        None => self.players[p].current.with_context(|| {
                            format!("{} has no current clip; pass \"clip\"", self.ids[p])
                        })?,
                    };
                    let length = self.clips[c].length;
                    ensure!(
                        time.is_finite() && (0.0..=length).contains(&time),
                        "time must be 0..{length} seconds (the clip length)"
                    );
                    let player = &mut self.players[p];
                    player.current = Some(c);
                    player.time = time;
                    self.apply(state, p);
                }
                other => bail!("unknown tween action {other}"),
            }
            Ok(self.player_json(p))
        })())
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("tween".into(), self.summary())])
    }
}

fn id_arg(v: &Value) -> Result<()> {
    ensure!(
        v.get("id").and_then(Value::as_str).is_some(),
        "needs {{\"id\":\"entity with tween.clips\"}}"
    );
    Ok(())
}

pub fn register(r: &mut RuntimeRegistry) {
    let entity = "entity ID with tween.clips";
    r.authoring.action(
        "tween.play",
        "tween",
        json!({"id":entity,"clip":"clip name","reverse":"optional bool: play toward 0","restart":"optional bool: start from the end (0, or the length when reversed) even if this clip is current","effect":"another clip, or restart: jumps to its start pose and plays; the current clip: continues from its time in the requested direction (a once clip already at that end stays stopped)"}),
        |v| {
            id_arg(v)?;
            ensure!(
                v.get("clip").and_then(Value::as_str).is_some(),
                "tween.play needs clip"
            );
            for flag in ["reverse", "restart"] {
                ensure!(
                    v.get(flag).is_none_or(Value::is_boolean),
                    "{flag} must be a bool"
                );
            }
            Ok(())
        },
    );
    r.authoring.action(
        "tween.stop",
        "tween",
        json!({"id":entity,"effect":"stops at the current time; values stay"}),
        id_arg,
    );
    r.authoring.action(
        "tween.seek",
        "tween",
        json!({"id":entity,"time":"seconds 0..clip length","clip":"optional clip name (default: the current clip)","effect":"jumps and writes the values now; playing state unchanged"}),
        |v| {
            id_arg(v)?;
            ensure!(
                v.get("time").and_then(Value::as_f64).is_some(),
                "tween.seek needs time (seconds)"
            );
            Ok(())
        },
    );
    r.authoring.action(
        "tween.inspect",
        "tween",
        json!({"id":entity,"result":"clip, time, length, state (idle|playing|reversing|stopped), clips with tracks [target, property, keys]"}),
        id_arg,
    );
    r.authoring
        .action("tween.summary", "tween", json!({}), |_| Ok(()));
    r.factories.insert("tween.animate".into(), |state| {
        Ok(Box::new(TweenSystem::new(state)?))
    });
}
