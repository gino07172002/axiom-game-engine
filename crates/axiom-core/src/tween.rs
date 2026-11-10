//! Property tweens (`tween.clips`): named clips of keyframe tracks that animate typed
//! properties of an entity or one of its descendants (local transform, `render.light`
//! colour/intensity, `render.mesh` colour/emission) and `behavior.program` number variables.
//! The runtime system `tween.animate` resolves every track to a typed slot when the runtime is
//! built and samples flat key arrays each fixed step; actions `tween.play` / `stop` / `seek`
//! drive playback and a finished `once` clip can emit an `on_finish` event.
//!
//! Values are absolute: a `core.transform/yaw` track sets the local rotation to a rotation
//! about +Y, `core.transform/rotation` to Euler XYZ degrees (the editor's convention). Keys
//! hold before the first and after the last key. The `ease` of a key shapes the segment that
//! arrives at it; every easing stays between the two key values, so a clip whose keys are
//! valid only ever writes valid values.
use anyhow::{bail, ensure, Context, Result};
use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// `tween.clips` components per world.
pub const MAX_TWEENS: usize = 2048;
/// Clips per component.
pub const MAX_CLIPS: usize = 16;
/// Tracks per clip.
pub const MAX_TRACKS: usize = 16;
/// Keys per track.
pub const MAX_KEYS: usize = 256;
/// Tracks across all components of a world (runtime track table).
pub const MAX_WORLD_TRACKS: usize = 16 * 1024;
/// Keys across all components of a world (runtime key arrays, up to 3 floats per key).
pub const MAX_WORLD_KEYS: usize = 128 * 1024;
/// `on_finish` events one fixed step emits; further finished clips wait for the next steps.
pub const MAX_TWEEN_EVENTS_PER_STEP: usize = 64;
/// Longest clip and latest key time (seconds).
pub const MAX_DURATION: f32 = 3600.0;
/// Clips without an explicit duration that end at 0 get this one (editor presets).
pub const DEFAULT_DURATION: f32 = 1.0;
const MAX_SPEED: f32 = 100.0;
const MIN_SPEED: f32 = 0.01;

/// Curve of the segment arriving at a key.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    #[default]
    Linear,
    /// Hold the previous key's value until this key's time.
    Step,
    /// Quadratic: slow start.
    EaseIn,
    /// Quadratic: slow end.
    EaseOut,
    /// Quadratic: slow start and end.
    EaseInOut,
}

impl Ease {
    /// Weight of the arriving key at fraction `u` (0..1) of the segment; stays in 0..1.
    pub fn apply(self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Self::Linear => u,
            Self::Step => {
                if u >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::EaseIn => u * u,
            Self::EaseOut => u * (2.0 - u),
            Self::EaseInOut => {
                if u < 0.5 {
                    2.0 * u * u
                } else {
                    let v = 2.0 - 2.0 * u;
                    1.0 - v * v * 0.5
                }
            }
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "linear" => Self::Linear,
            "step" => Self::Step,
            "ease_in" => Self::EaseIn,
            "ease_out" => Self::EaseOut,
            "ease_in_out" => Self::EaseInOut,
            other => {
                bail!("unknown ease '{other}' (use linear, step, ease_in, ease_out, ease_in_out)")
            }
        })
    }
}

fn is_linear(e: &Ease) -> bool {
    *e == Ease::Linear
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TweenMode {
    /// Play to the end (or back to 0 when reversed) and stop; `on_finish` fires.
    #[default]
    Once,
    /// Wrap around forever.
    Loop,
    /// Bounce between the ends forever.
    PingPong,
}

impl TweenMode {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "once" => Self::Once,
            "loop" => Self::Loop,
            "ping_pong" | "pingpong" => Self::PingPong,
            other => bail!("unknown mode '{other}' (use once, loop, ping_pong)"),
        })
    }
}

/// A key value: a number for 1-D properties, `[x, y, z]` for 3-D ones.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum KeyValue {
    Scalar(f32),
    Vector(Vec<f32>),
}

impl KeyValue {
    pub fn from_slice(v: &[f32]) -> Self {
        if v.len() == 1 {
            Self::Scalar(v[0])
        } else {
            Self::Vector(v.to_vec())
        }
    }
    pub fn as_slice(&self) -> &[f32] {
        match self {
            Self::Scalar(v) => std::slice::from_ref(v),
            Self::Vector(v) => v,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TweenKey {
    /// Seconds from the clip start.
    pub t: f32,
    pub v: KeyValue,
    /// Curve of the segment arriving at this key (ignored on the first key).
    #[serde(default, skip_serializing_if = "is_linear")]
    pub ease: Ease,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TweenTrack {
    /// Entity ID of the animated object: this entity or a descendant ("" = this entity).
    /// Ignored (must be "") for `behavior.program/variables/NAME`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// See [`Property::parse`].
    pub property: String,
    /// Strictly increasing times.
    pub keys: Vec<TweenKey>,
}

fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TweenClip {
    pub tracks: Vec<TweenTrack>,
    /// Seconds; omitted = the last key time across the tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f32>,
    #[serde(default)]
    pub mode: TweenMode,
    /// Playback rate (1 = authored times).
    #[serde(default = "one")]
    pub speed: f32,
    /// Starts playing when the runtime starts (at most one clip per component).
    #[serde(default)]
    pub autoplay: bool,
    /// `behavior.program` event emitted when a `once` clip reaches its end ("" = none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_finish: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TweenClips {
    pub clips: BTreeMap<String, TweenClip>,
    /// A disabled component never advances (actions are refused).
    #[serde(default = "yes")]
    pub enabled: bool,
}

/// A typed animatable property. Transform values are local to the parent.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Property {
    Position,
    /// Euler XYZ degrees.
    Rotation,
    /// Degrees about +Y (sets the whole rotation).
    Yaw,
    Scale,
    LightColor,
    LightIntensity,
    /// Every `render.mesh` part's colour.
    MeshColor,
    /// Every `render.mesh` part's emission.
    MeshEmission,
    /// A `behavior.program` number (or integer, rounded) variable.
    Variable(String),
}

/// Property paths accepted by tracks (variables: `behavior.program/variables/NAME`).
pub const PROPERTIES: [&str; 8] = [
    "core.transform/position",
    "core.transform/rotation",
    "core.transform/yaw",
    "core.transform/scale",
    "render.light/color",
    "render.light/intensity",
    "render.mesh/color",
    "render.mesh/emission",
];
const VARIABLE_PREFIX: &str = "behavior.program/variables/";

impl Property {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "core.transform/position" => Self::Position,
            "core.transform/rotation" => Self::Rotation,
            "core.transform/yaw" => Self::Yaw,
            "core.transform/scale" => Self::Scale,
            "render.light/color" => Self::LightColor,
            "render.light/intensity" => Self::LightIntensity,
            "render.mesh/color" => Self::MeshColor,
            "render.mesh/emission" => Self::MeshEmission,
            other => match other.strip_prefix(VARIABLE_PREFIX) {
                Some(name) => {
                    crate::valid_id(name).context("variable")?;
                    Self::Variable(name.to_owned())
                }
                None => bail!(
                    "unsupported tween property '{other}' (use {}, or {VARIABLE_PREFIX}NAME)",
                    PROPERTIES.join(", ")
                ),
            },
        })
    }

    /// Floats per key.
    pub fn dims(&self) -> usize {
        match self {
            Self::Position | Self::Rotation | Self::Scale | Self::LightColor | Self::MeshColor => 3,
            _ => 1,
        }
    }

    pub fn is_transform(&self) -> bool {
        matches!(
            self,
            Self::Position | Self::Rotation | Self::Yaw | Self::Scale
        )
    }

    /// The state a track writes: rotation and yaw share one (they would fight).
    pub fn channel(&self) -> String {
        match self {
            Self::Position => "position".into(),
            Self::Rotation | Self::Yaw => "rotation".into(),
            Self::Scale => "scale".into(),
            Self::LightColor => "light color".into(),
            Self::LightIntensity => "light intensity".into(),
            Self::MeshColor => "mesh color".into(),
            Self::MeshEmission => "mesh emission".into(),
            Self::Variable(v) => format!("variable {v}"),
        }
    }

    /// Valid key values (inclusive range per component).
    fn range(&self) -> (f32, f32) {
        match self {
            Self::Position => (-1_000_000.0, 1_000_000.0),
            Self::Rotation | Self::Yaw => (-36_000.0, 36_000.0),
            Self::Scale => (0.0001, 10_000.0),
            Self::LightColor | Self::MeshColor => (0.0, 1.0),
            Self::LightIntensity => (0.0, 100.0),
            Self::MeshEmission => (0.0, 16.0),
            Self::Variable(_) => (-1.0e9, 1.0e9),
        }
    }
}

/// Unit quaternion of Euler XYZ degrees (the editor's rotation convention).
pub fn euler_rotation(v: [f32; 3]) -> Quat {
    Quat::from_euler(
        EulerRot::XYZ,
        v[0].to_radians(),
        v[1].to_radians(),
        v[2].to_radians(),
    )
    .normalize()
}

/// Degrees about +Y of a rotation: the heading of its +Z axis on the XZ plane.
pub fn yaw_degrees(q: Quat) -> f32 {
    let f = q * Vec3::Z;
    if f.x * f.x + f.z * f.z < 1e-12 {
        0.0
    } else {
        f.x.atan2(f.z).to_degrees()
    }
}

impl TweenClip {
    /// The playback length: the authored duration or the last key time.
    pub fn length(&self) -> f32 {
        self.duration.unwrap_or_else(|| self.last_key())
    }

    pub fn last_key(&self) -> f32 {
        self.tracks
            .iter()
            .filter_map(|t| t.keys.last())
            .map(|k| k.t)
            .fold(0.0, f32::max)
    }

    pub fn validate(&self) -> Result<()> {
        let n = self.tracks.len();
        ensure!(
            (1..=MAX_TRACKS).contains(&n),
            "a clip needs 1..{MAX_TRACKS} tracks (has {n})"
        );
        let mut channels = std::collections::BTreeSet::new();
        for (i, track) in self.tracks.iter().enumerate() {
            track.validate().with_context(|| format!("tracks/{i}"))?;
            let property = Property::parse(&track.property)?;
            ensure!(
                channels.insert((track.target.clone(), property.channel())),
                "tracks/{i}: another track of this clip already animates the {} of {}",
                property.channel(),
                if track.target.is_empty() {
                    "this entity"
                } else {
                    &track.target
                }
            );
        }
        if let Some(d) = self.duration {
            ensure!(
                d.is_finite() && (0.01..=MAX_DURATION).contains(&d),
                "duration must be 0.01..{MAX_DURATION} seconds"
            );
            ensure!(
                self.last_key() <= d,
                "a key at {} s lies after the duration {d} s (move the key or lengthen the clip)",
                self.last_key()
            );
        }
        ensure!(
            self.length() > 0.0,
            "the clip is 0 seconds long: add a key after 0 or set duration"
        );
        ensure!(
            self.speed.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&self.speed),
            "speed must be {MIN_SPEED}..{MAX_SPEED}"
        );
        if !self.on_finish.is_empty() {
            crate::valid_id(&self.on_finish).context("on_finish")?;
            ensure!(
                self.mode == TweenMode::Once,
                "on_finish only fires in mode once (loop and ping_pong never finish)"
            );
        }
        Ok(())
    }
}

impl TweenTrack {
    pub fn validate(&self) -> Result<()> {
        let property = Property::parse(&self.property)?;
        if matches!(property, Property::Variable(_)) {
            ensure!(
                self.target.is_empty(),
                "variable tracks have no target (variables are world state)"
            );
        } else if !self.target.is_empty() {
            crate::valid_id(&self.target).context("target")?;
        }
        let n = self.keys.len();
        ensure!(
            (1..=MAX_KEYS).contains(&n),
            "a track needs 1..{MAX_KEYS} keys (has {n})"
        );
        let dims = property.dims();
        let (lo, hi) = property.range();
        for (i, key) in self.keys.iter().enumerate() {
            ensure!(
                key.t.is_finite() && (0.0..=MAX_DURATION).contains(&key.t),
                "keys/{i}: time must be 0..{MAX_DURATION} seconds"
            );
            if i > 0 {
                ensure!(
                    key.t > self.keys[i - 1].t,
                    "keys/{i}: times must be strictly increasing ({} after {})",
                    key.t,
                    self.keys[i - 1].t
                );
            }
            let v = key.v.as_slice();
            ensure!(
                v.len() == dims,
                "keys/{i}: {} takes {} (got {} numbers)",
                self.property,
                if dims == 1 {
                    "a number".to_owned()
                } else {
                    format!("[{dims} numbers]")
                },
                v.len()
            );
            ensure!(
                v.iter().all(|x| x.is_finite() && (lo..=hi).contains(x)),
                "keys/{i}: {} values must be {lo}..{hi}",
                self.property
            );
        }
        Ok(())
    }
}

impl TweenClips {
    pub fn validate(&self) -> Result<()> {
        let n = self.clips.len();
        ensure!(
            (1..=MAX_CLIPS).contains(&n),
            "tween.clips needs 1..{MAX_CLIPS} clips (has {n})"
        );
        for (name, clip) in &self.clips {
            crate::valid_id(name).context("clip name")?;
            clip.validate().with_context(|| format!("clips/{name}"))?;
        }
        ensure!(
            self.clips.values().filter(|c| c.autoplay).count() <= 1,
            "at most one clip per component may autoplay"
        );
        Ok(())
    }
}

/// Samples one track at `time` into `out` (`dims` floats). `times`, `values` (`dims` per key)
/// and `eases` are the track's flat key arrays. `hint` caches the segment of the previous
/// call: steady playback finds its segment in O(1), a jump falls back to binary search.
pub fn sample(
    times: &[f32],
    values: &[f32],
    eases: &[Ease],
    dims: usize,
    time: f32,
    hint: &mut usize,
    out: &mut [f32],
) {
    let n = times.len();
    let value = |k: usize| &values[k * dims..k * dims + dims];
    if n == 1 || time <= times[0] {
        out[..dims].copy_from_slice(value(0));
        *hint = 0;
        return;
    }
    if time >= times[n - 1] {
        out[..dims].copy_from_slice(value(n - 1));
        *hint = n - 2;
        return;
    }
    let inside = |a: usize| times[a] <= time && time < times[a + 1];
    let a = if *hint < n - 1 && inside(*hint) {
        *hint
    } else if *hint + 1 < n - 1 && inside(*hint + 1) {
        *hint + 1
    } else {
        times.partition_point(|&t| t <= time) - 1
    };
    *hint = a;
    let u = (time - times[a]) / (times[a + 1] - times[a]);
    let w = eases[a + 1].apply(u);
    let (va, vb) = (value(a), value(a + 1));
    for i in 0..dims {
        out[i] = va[i] + (vb[i] - va[i]) * w;
    }
}

/// Advances playback by `delta` seconds (>= 0) in the current direction on a clip of
/// `length` seconds. Returns the new time, direction and whether a `once` clip reached its
/// end in this call.
pub fn advance(
    time: f32,
    forward: bool,
    delta: f32,
    length: f32,
    mode: TweenMode,
) -> (f32, bool, bool) {
    match mode {
        TweenMode::Once => {
            if forward {
                let t = time + delta;
                if t >= length {
                    (length, true, true)
                } else {
                    (t, true, false)
                }
            } else {
                let t = time - delta;
                if t <= 0.0 {
                    (0.0, false, true)
                } else {
                    (t, false, false)
                }
            }
        }
        TweenMode::Loop => {
            let t = if forward { time + delta } else { time - delta };
            (t.rem_euclid(length), forward, false)
        }
        TweenMode::PingPong => {
            // Phase 0..2L: forward on 0..L, backward on L..2L.
            let phase = if forward { time } else { 2.0 * length - time };
            let phase = (phase + delta).rem_euclid(2.0 * length);
            if phase <= length {
                (phase, true, false)
            } else {
                (2.0 * length - phase, false, false)
            }
        }
    }
}

/// The authored (committed) value of `property` on entity `id`: what "record key" stores.
/// Light and mesh fields fall back to their defaults; mesh values come from the first part.
pub fn authored_value(world: &crate::World, id: &str, property: &Property) -> Result<Vec<f32>> {
    if let Property::Variable(name) = property {
        let value = world
            .resources
            .get("behavior.program")
            .and_then(|p| p.pointer(&format!("/variables/{}", crate::local::escape(name))))
            .with_context(|| format!("behavior.program declares no variable {name}"))?;
        let v = value
            .as_f64()
            .with_context(|| format!("variable {name} is not a number"))?;
        return Ok(vec![v as f32]);
    }
    let entity = world
        .entity(id)
        .with_context(|| format!("entity '{id}' not found"))?;
    let mut components = world.resolved_components(entity);
    let mut component = |name: &str| {
        components
            .remove(name)
            .with_context(|| format!("{id} has no {name}"))
    };
    Ok(match property {
        Property::Position | Property::Rotation | Property::Yaw | Property::Scale => {
            let t: crate::Transform = serde_json::from_value(component("core.transform")?)?;
            let q = Quat::from_array(t.rotation).normalize();
            match property {
                Property::Position => t.position.to_vec(),
                Property::Scale => t.scale.to_vec(),
                Property::Yaw => vec![yaw_degrees(q)],
                _ => {
                    let (x, y, z) = q.to_euler(EulerRot::XYZ);
                    vec![x.to_degrees(), y.to_degrees(), z.to_degrees()]
                }
            }
        }
        Property::LightColor | Property::LightIntensity => {
            let l: crate::light::RenderLight = serde_json::from_value(component("render.light")?)?;
            match property {
                Property::LightColor => l.color.to_vec(),
                _ => vec![l.intensity],
            }
        }
        Property::MeshColor | Property::MeshEmission => {
            let m: crate::RenderMesh = serde_json::from_value(component("render.mesh")?)?;
            let part = m.parts.first().context("render.mesh has no parts")?;
            match property {
                Property::MeshColor => part.color.to_vec(),
                _ => vec![part.emission],
            }
        }
        Property::Variable(_) => unreachable!(),
    })
}

pub fn clips_schema() -> Value {
    json!({
        "clips": format!("name -> clip, 1..{MAX_CLIPS} per component; one clip plays at a time"),
        "clip": {
            "tracks": format!("1..{MAX_TRACKS} [{{target, property, keys}}]"),
            "duration": format!("seconds 0.01..{MAX_DURATION}; omitted = the last key time"),
            "mode": "once (default; stops at the end, on_finish fires) | loop | ping_pong",
            "speed": format!("{MIN_SPEED}..{MAX_SPEED}, default 1 (playback rate)"),
            "autoplay": "bool, default false: starts playing forward when the runtime starts (at most one clip per component)",
            "on_finish": "behavior.program event when a once clip reaches its end (forward or reversed); may declare text fields `entity` (component owner) and/or `clip`"
        },
        "track": {
            "target": "entity ID: this entity or a descendant (\"\" = this entity); none for variables",
            "property": format!("{} | {VARIABLE_PREFIX}NAME (number or integer variable; drives ui.document progress/text bindings and rule conditions)", PROPERTIES.join(" | ")),
            "keys": format!("1..{MAX_KEYS} {{t: seconds, strictly increasing, v: number or [x,y,z], ease?: linear|step|ease_in|ease_out|ease_in_out (curve arriving at this key)}}")
        },
        "values": "absolute: position/scale local [x,y,z]; rotation Euler XYZ degrees; yaw degrees about +Y (whole rotation); colors rgb 0..1; light intensity 0..100; mesh emission 0..16 (every part). Interpolated per number; keys hold outside their range",
        "enabled": "bool, default true: a disabled component never advances and refuses actions",
        "conflicts": "authoring error tween_conflict: transform tracks on an entity moved by another system (core.motion, path.follow, nav.agent, fsm.agent, rts.unit, action.actor, fps.*); transform tracks on (or above) a nav.blocker or physics.collider (static obstacles do not follow); two components animating the same property",
        "requires": "system tween.animate (capability tween); on_finish and variable tracks need behavior.execute",
        "actions": "tween.play {id, clip, reverse?, restart?} | tween.stop {id} | tween.seek {id, time, clip?} | tween.inspect {id} | tween.summary",
        "budget": format!("{MAX_TWEENS} components, {MAX_WORLD_TRACKS} tracks, {MAX_WORLD_KEYS} keys per world; {MAX_TWEEN_EVENTS_PER_STEP} on_finish events per step (more wait for the next step)"),
        "checkpoint": "not supported: worlds scheduling tween.animate refuse checkpoints explicitly",
        "edit": "ACL: tween SEL clip NAME [property=P] [target=ID] [duration=S] [mode=M] [speed=X] [autoplay=on|off] [on_finish=E] | tween ID clip NAME set|remove | tween ID track CLIP add P [target=ID] | tween ID track CLIP remove I | tween ID key CLIP TRACK add T [VALUE] [ease=E] | ... move I T | ... remove I"
    })
}

/// World-level checks run by `World::compile` on resolved components: budgets, the required
/// systems, targets (this entity or a descendant, with the animated component), variables,
/// mover / static-obstacle / cross-component conflicts and `on_finish` payload contracts.
pub fn validate_world(
    world: &crate::World,
    registry: &crate::Registry,
    entities: &[crate::CompiledEntity],
    indices: &BTreeMap<String, usize>,
    parents: &[Option<usize>],
) -> Result<()> {
    let count = entities.iter().filter(|e| e.tween.is_some()).count();
    if count == 0 {
        return Ok(());
    }
    let budget = |message: String| crate::Diagnostic::error("tween_budget", "/entities", message);
    if count > MAX_TWEENS {
        return Err(budget(format!(
            "tween component budget exceeded ({count} > {MAX_TWEENS})"
        )));
    }
    let (mut tracks, mut keys) = (0, 0);
    for e in entities {
        for clip in e.tween.iter().flat_map(|t| t.clips.values()) {
            tracks += clip.tracks.len();
            keys += clip.tracks.iter().map(|t| t.keys.len()).sum::<usize>();
        }
    }
    if tracks > MAX_WORLD_TRACKS {
        return Err(budget(format!(
            "tween track budget exceeded ({tracks} > {MAX_WORLD_TRACKS})"
        )));
    }
    if keys > MAX_WORLD_KEYS {
        return Err(budget(format!(
            "tween key budget exceeded ({keys} > {MAX_WORLD_KEYS})"
        )));
    }
    let has_system = |s: &str| world.systems.iter().any(|x| x == s);
    let program = world.resources.get("behavior.program");
    let mover = |e: &crate::CompiledEntity| {
        e.components.keys().find_map(|c| {
            if c == "path.follow" {
                Some((c.clone(), "path.move".to_owned()))
            } else {
                registry.movers.get(c).map(|s| (c.clone(), s.clone()))
            }
        })
    };
    // (target slot, channel) -> owner of the component animating it.
    let mut owners: BTreeMap<(usize, String), &str> = BTreeMap::new();
    let mut variables: BTreeMap<String, &str> = BTreeMap::new();
    // Entities whose transform some track animates (static obstacles must not sit below).
    let mut moved = vec![false; entities.len()];
    for (slot, e) in entities.iter().enumerate() {
        let Some(tween) = &e.tween else { continue };
        let at = |field: &str| {
            format!(
                "/entities/@{}/components/tween.clips{field}",
                crate::local::escape(&e.id)
            )
        };
        let invalid = |field: &str, message: String| {
            crate::Diagnostic::error("invalid_tween", at(field), message)
        };
        if !has_system("tween.animate") {
            return Err(invalid(
                "",
                "tween.clips needs the tween.animate system (add it to systems)".into(),
            ));
        }
        for (name, clip) in &tween.clips {
            let clip_at = format!("/clips/{}", crate::local::escape(name));
            for (i, track) in clip.tracks.iter().enumerate() {
                let field = format!("{clip_at}/tracks/{i}");
                let property = Property::parse(&track.property)
                    .map_err(|e| invalid(&field, format!("{e:#}")))?;
                if let Property::Variable(var) = &property {
                    if !has_system("behavior.execute") {
                        return Err(invalid(
                            &field,
                            "variable tracks need the behavior.execute system".into(),
                        ));
                    }
                    let declared = program.and_then(|p| {
                        p.pointer(&format!("/variables/{}", crate::local::escape(var)))
                    });
                    match declared {
                        Some(v) if v.is_number() => {}
                        Some(_) => {
                            return Err(invalid(
                                &field,
                                format!("variable {var} is not a number (declare it as 0.0)"),
                            ))
                        }
                        None => {
                            return Err(invalid(
                                &field,
                                format!("behavior.program declares no variable {var}"),
                            ))
                        }
                    }
                    if let Some(other) = variables.insert(var.clone(), &e.id) {
                        if other != e.id {
                            return Err(crate::Diagnostic::error(
                                "tween_conflict",
                                at(&field),
                                format!("variable {var} is already animated by the tween.clips of {other}; one component per property"),
                            ));
                        }
                    }
                    continue;
                }
                let target = if track.target.is_empty() {
                    slot
                } else {
                    let &t = indices.get(&track.target).ok_or_else(|| {
                        invalid(
                            &format!("{field}/target"),
                            format!("target entity {} does not exist", track.target),
                        )
                    })?;
                    let mut cursor = Some(t);
                    let mut depth = 0;
                    while let Some(c) = cursor.filter(|&c| c != slot) {
                        cursor = parents[c];
                        depth += 1;
                        if depth > entities.len() {
                            break;
                        }
                    }
                    if cursor != Some(slot) {
                        return Err(invalid(
                            &format!("{field}/target"),
                            format!(
                                "target {} is not {} or one of its descendants",
                                track.target, e.id
                            ),
                        ));
                    }
                    t
                };
                let te = &entities[target];
                let missing = |component: &str| {
                    invalid(
                        &field,
                        format!(
                            "{} animates {}, which {} lacks",
                            track.property, component, te.id
                        ),
                    )
                };
                if property.is_transform() {
                    if te.transform.is_none() {
                        return Err(missing("core.transform"));
                    }
                    if let Some((component, system)) = mover(te) {
                        return Err(crate::Diagnostic::error(
                            "tween_conflict",
                            at(&field),
                            format!(
                                "{} has {component} (moved by {system}); a tween must not animate its transform too. Animate a parent or child entity instead",
                                te.id
                            ),
                        ));
                    }
                    moved[target] = true;
                } else if matches!(property, Property::LightColor | Property::LightIntensity) {
                    if te.light.is_none() {
                        return Err(missing("render.light"));
                    }
                } else if te.mesh.is_none() {
                    return Err(missing("render.mesh"));
                }
                if let Some(other) = owners.insert((target, property.channel()), &e.id) {
                    if other != e.id {
                        return Err(crate::Diagnostic::error(
                            "tween_conflict",
                            at(&field),
                            format!(
                                "the {} of {} is already animated by the tween.clips of {other}; one component per property",
                                property.channel(),
                                te.id
                            ),
                        ));
                    }
                }
            }
            if !clip.on_finish.is_empty() {
                let field = format!("{clip_at}/on_finish");
                if !has_system("behavior.execute") {
                    return Err(invalid(
                        &field,
                        "on_finish events need the behavior.execute system".into(),
                    ));
                }
                let event = &clip.on_finish;
                let fields = program
                    .and_then(|p| {
                        p.pointer(&format!("/events/{}/fields", crate::local::escape(event)))
                    })
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        invalid(
                            &field,
                            format!("clip emits {event}, which behavior.program does not declare"),
                        )
                    })?;
                if !fields
                    .iter()
                    .all(|(k, t)| (k == "entity" || k == "clip") && t == "text")
                {
                    return Err(invalid(
                        &field,
                        format!(
                            "tween event {event} may only declare text fields `entity` and `clip`"
                        ),
                    ));
                }
            }
        }
    }
    // Static obstacles are rasterized / copied once at their authored place.
    for (slot, e) in entities.iter().enumerate() {
        // Camera blocker meshes are collected at their authored pose too (camera.rs).
        let obstacle = ["nav.blocker", "physics.collider"]
            .into_iter()
            .find(|c| e.components.contains_key(*c))
            .or_else(|| crate::camera::mesh_blocks(e).then_some("render.mesh"));
        let Some(obstacle) = obstacle else { continue };
        let what = if obstacle != "render.mesh" {
            obstacle.to_owned()
        } else if e.components.contains_key(crate::camera::BLOCKER_COMPONENT) {
            format!("render.mesh with {}", crate::camera::BLOCKER_COMPONENT)
        } else {
            format!("render.mesh tagged {}", crate::camera::BLOCKER_TAG)
        };
        let mut cursor = Some(slot);
        let mut depth = 0;
        while let Some(c) = cursor {
            if moved[c] {
                return Err(crate::Diagnostic::error(
                    "tween_conflict",
                    format!("/entities/@{}/components/{obstacle}", crate::local::escape(&e.id)),
                    format!(
                        "{} has {what}, a static obstacle, and a tween animates the transform of {}; obstacles do not follow runtime transforms. Keep the obstacle off the animated hierarchy (see docs/tweens.zh-TW.md)",
                        e.id, entities[c].id
                    ),
                ));
            }
            cursor = parents[c];
            depth += 1;
            if depth > entities.len() {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clips(v: Value) -> Result<TweenClips> {
        let c: TweenClips = serde_json::from_value(v)?;
        c.validate()?;
        Ok(c)
    }
    fn err(v: Value) -> String {
        format!("{:#}", clips(v).unwrap_err())
    }
    fn one_track(track: Value) -> Value {
        json!({"clips":{"a":{"tracks":[track]}}})
    }

    #[test]
    fn easing_curves() {
        for e in [Ease::Linear, Ease::EaseIn, Ease::EaseOut, Ease::EaseInOut] {
            assert_eq!(e.apply(0.0), 0.0, "{e:?}");
            assert_eq!(e.apply(1.0), 1.0, "{e:?}");
        }
        assert_eq!(Ease::Linear.apply(0.25), 0.25);
        assert_eq!(Ease::EaseIn.apply(0.5), 0.25);
        assert_eq!(Ease::EaseOut.apply(0.5), 0.75);
        assert_eq!(Ease::EaseInOut.apply(0.25), 0.125);
        assert_eq!(Ease::EaseInOut.apply(0.5), 0.5);
        assert_eq!(Ease::EaseInOut.apply(0.75), 0.875);
        assert_eq!(Ease::Step.apply(0.999), 0.0);
        assert_eq!(Ease::Step.apply(1.0), 1.0);
        // Every curve stays inside 0..1 and is monotone (values never overshoot keys).
        for e in [
            Ease::Linear,
            Ease::Step,
            Ease::EaseIn,
            Ease::EaseOut,
            Ease::EaseInOut,
        ] {
            let mut last = 0.0;
            for k in 0..=100 {
                let w = e.apply(k as f32 / 100.0);
                assert!((0.0..=1.0).contains(&w) && w >= last, "{e:?}");
                last = w;
            }
        }
    }

    #[test]
    fn sampling_holds_outside_and_uses_the_arriving_ease() {
        let times = [0.0, 1.0, 3.0];
        let values = [0.0, 10.0, 30.0];
        let eases = [Ease::Linear, Ease::EaseIn, Ease::Step];
        let mut hint = 0;
        let mut out = [0.0];
        let mut at = |t: f32, hint: &mut usize| {
            sample(&times, &values, &eases, 1, t, hint, &mut out);
            out[0]
        };
        assert_eq!(at(-1.0, &mut hint), 0.0);
        assert_eq!(at(0.5, &mut hint), 2.5, "ease_in arriving at key 1");
        assert_eq!(hint, 0);
        assert_eq!(at(1.0, &mut hint), 10.0);
        assert_eq!(at(2.9, &mut hint), 10.0, "step holds until the key");
        assert_eq!(hint, 1);
        assert_eq!(at(3.0, &mut hint), 30.0);
        assert_eq!(at(99.0, &mut hint), 30.0);
        // A stale hint far away still finds the right segment (binary search).
        let mut far = 1;
        assert_eq!(at(0.5, &mut far), 2.5);
        assert_eq!(far, 0);
        // Vectors interpolate per component; one key holds.
        let mut v3 = [0.0; 3];
        sample(
            &[0.0, 2.0],
            &[0.0, 0.0, 0.0, 2.0, 4.0, -2.0],
            &[Ease::Linear; 2],
            3,
            1.0,
            &mut hint,
            &mut v3,
        );
        assert_eq!(v3, [1.0, 2.0, -1.0]);
        sample(&[0.5], &[7.0], &[Ease::Linear], 1, 0.0, &mut hint, &mut out);
        assert_eq!(out[0], 7.0);
    }

    #[test]
    fn playback_modes_and_reverse() {
        // once: stops exactly at the ends and reports the finish once.
        assert_eq!(
            advance(0.9, true, 0.2, 1.0, TweenMode::Once),
            (1.0, true, true)
        );
        assert_eq!(
            advance(0.5, true, 0.2, 1.0, TweenMode::Once),
            (0.7, true, false)
        );
        assert_eq!(
            advance(0.1, false, 0.2, 1.0, TweenMode::Once),
            (0.0, false, true)
        );
        // loop wraps both ways.
        let (t, f, done) = advance(0.9, true, 0.2, 1.0, TweenMode::Loop);
        assert!((t - 0.1).abs() < 1e-6 && f && !done);
        let (t, f, _) = advance(0.1, false, 0.2, 1.0, TweenMode::Loop);
        assert!((t - 0.9).abs() < 1e-6 && !f);
        // ping_pong reflects at the ends and flips the direction.
        let (t, f, done) = advance(0.9, true, 0.2, 1.0, TweenMode::PingPong);
        assert!((t - 0.9).abs() < 1e-6 && !f && !done);
        let (t, f, _) = advance(0.1, false, 0.2, 1.0, TweenMode::PingPong);
        assert!((t - 0.1).abs() < 1e-6 && f);
        let (t, f, _) = advance(0.5, true, 2.0, 1.0, TweenMode::PingPong);
        assert!((t - 0.5).abs() < 1e-6 && f, "a full bounce returns");
    }

    #[test]
    fn defaults_round_trip_and_lengths() {
        let c = clips(one_track(
            json!({"property":"core.transform/yaw","keys":[{"t":0,"v":0},{"t":1.5,"v":90,"ease":"ease_out"}]}),
        ))
        .unwrap();
        let clip = &c.clips["a"];
        assert!(c.enabled && clip.mode == TweenMode::Once && clip.speed == 1.0 && !clip.autoplay);
        assert_eq!(clip.length(), 1.5);
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(
            back,
            json!({"clips":{"a":{"tracks":[{"property":"core.transform/yaw","keys":[{"t":0.0,"v":0.0},{"t":1.5,"v":90.0,"ease":"ease_out"}]}],"mode":"once","speed":1.0,"autoplay":false}},"enabled":true})
        );
        assert_eq!(serde_json::from_value::<TweenClips>(back).unwrap(), c);
        // Vector keys and an explicit duration past the last key.
        let c = clips(json!({"clips":{"b":{"duration":4,"mode":"ping_pong","tracks":[
            {"target":"door/leaf","property":"core.transform/position","keys":[{"t":0,"v":[0,0,0]},{"t":2,"v":[1,0,0]}]},
            {"property":"behavior.program/variables/glow","keys":[{"t":0,"v":0}]}]}}}))
        .unwrap();
        assert_eq!(c.clips["b"].length(), 4.0);
        assert_eq!(
            Property::parse("behavior.program/variables/glow").unwrap(),
            Property::Variable("glow".into())
        );
    }

    #[test]
    fn invalid_clips_are_rejected_with_reasons() {
        let key = |t: f32, v: Value| json!({"t":t,"v":v});
        let yaw =
            |keys: Vec<Value>| one_track(json!({"property":"core.transform/yaw","keys":keys}));
        assert!(err(json!({"clips":{}})).contains("1..16 clips"));
        assert!(err(yaw(vec![])).contains("1..256 keys"));
        assert!(err(yaw(vec![key(0.0, json!(0))])).contains("0 seconds long"));
        assert!(
            err(yaw(vec![key(1.0, json!(0)), key(1.0, json!(1))])).contains("strictly increasing")
        );
        assert!(err(yaw(vec![key(-1.0, json!(0)), key(1.0, json!(1))])).contains("time"));
        assert!(
            err(yaw(vec![key(0.0, json!([0, 0, 0])), key(1.0, json!(1))])).contains("a number")
        );
        assert!(err(one_track(
            json!({"property":"core.transform/position","keys":[{"t":1,"v":5}]})
        ))
        .contains("[3 numbers]"));
        assert!(err(one_track(
            json!({"property":"render.light/color","keys":[{"t":1,"v":[2,0,0]}]})
        ))
        .contains("0..1"));
        assert!(err(one_track(
            json!({"property":"core.transform/scale","keys":[{"t":1,"v":[0,1,1]}]})
        ))
        .contains("values"));
        assert!(err(one_track(
            json!({"property":"render.material/tint","keys":[{"t":1,"v":1}]})
        ))
        .contains("unsupported tween property"));
        assert!(err(one_track(
            json!({"target":"x","property":"behavior.program/variables/v","keys":[{"t":1,"v":1}]})
        ))
        .contains("no target"));
        assert!(err(one_track(
            json!({"property":"core.transform/yaw","keys":[{"t":1,"v":1,"ease":"bouncy"}]})
        ))
        .contains("unknown variant"));
        assert!(err(json!({"clips":{"a":{"tracks":[
            {"property":"core.transform/yaw","keys":[{"t":1,"v":1}]},
            {"property":"core.transform/rotation","keys":[{"t":1,"v":[0,1,0]}]}]}}}))
        .contains("already animates the rotation"));
        let ok_track = json!({"property":"core.transform/yaw","keys":[{"t":1,"v":1}]});
        assert!(
            err(json!({"clips":{"a":{"tracks":[ok_track.clone()],"duration":0.5}}}))
                .contains("after the duration")
        );
        assert!(
            err(json!({"clips":{"a":{"tracks":[ok_track.clone()],"speed":0}}})).contains("speed")
        );
        assert!(err(
            json!({"clips":{"a":{"tracks":[ok_track.clone()],"mode":"loop","on_finish":"done"}}})
        )
        .contains("on_finish only fires"));
        assert!(err(json!({"clips":{"a":{"tracks":[ok_track.clone()],"autoplay":true},"b":{"tracks":[ok_track.clone()],"autoplay":true}}})).contains("at most one clip"));
        assert!(
            err(json!({"clips":{"bad name":{"tracks":[ok_track.clone()]}}})).contains("clip name")
        );
        assert!(
            err(json!({"clips":{"a":{"tracks":[ok_track],"loop":true}}})).contains("unknown field")
        );
    }

    #[test]
    fn yaw_and_euler_helpers_agree() {
        let q = Quat::from_rotation_y(90f32.to_radians());
        assert!((yaw_degrees(q) - 90.0).abs() < 1e-3);
        let e = euler_rotation([0.0, 90.0, 0.0]);
        assert!(e.angle_between(q) < 1e-4);
        assert_eq!(yaw_degrees(Quat::IDENTITY), 0.0);
    }
}
