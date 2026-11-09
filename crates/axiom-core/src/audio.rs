//! Audio authoring types: the `audio.library` resource (short tone cues for gameplay events and
//! procedural clips), positional sources (`audio.source`) and the listener (`audio.listener`).
//!
//! Clips are synthesis definitions, not files: the runtime renders each one once to 22,050 Hz
//! mono 16-bit PCM (cached by the definition's content hash) when a runtime is built, so playing
//! a clip never decodes anything. Spatialisation is a pure function of the listener and source
//! poses ([`attenuation`], [`pan`], [`pan_gains`]); the runtime system `audio.spatial` applies it
//! every fixed step and reports the results in `observe`, so headless runs can verify what a
//! listener would hear without an audio device.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// PCM rate of synthesised clips and of the mixer output.
pub const SAMPLE_RATE: u32 = 22_050;
/// Tone cues per library (existing `audio.events` budget).
pub const MAX_CUES: usize = 64;
/// Procedural clips per library.
pub const MAX_LIBRARY_CLIPS: usize = 64;
/// Longest clip (seconds).
pub const MAX_CLIP_SECONDS: f32 = 20.0;
/// PCM samples of all clips of one library together (16-bit mono: 16 MiB).
pub const MAX_LIBRARY_SAMPLES: usize = 8 * 1024 * 1024;
/// `audio.source` components per world.
pub const MAX_AUDIO_SOURCES: usize = 1024;
/// Clip names a preset can create (`ops::add_audio_clip`, ACL `audio clip ID PRESET`).
pub const CLIP_PRESETS: [&str; 5] = ["tone", "engine", "fountain", "creak", "footstep"];

/// A short tone cue played by `audio.events` (gameplay sounds such as hit feedback).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tone {
    pub frequency: f32,
    pub duration_ms: u32,
    pub gain: f32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Wave {
    #[default]
    Sine,
    Square,
    Saw,
    Triangle,
    /// White noise (frequency ignored); usually low-passed.
    Noise,
}

fn one() -> f32 {
    1.0
}
fn half() -> f32 {
    0.5
}
fn default_frequency() -> f32 {
    220.0
}
fn short_ramp() -> f32 {
    0.005
}
fn default_seed() -> u64 {
    1
}

/// A procedural clip: an oscillator (optionally swept, roughened, tremoloed and low-passed)
/// with a linear attack / release, peak-normalised to `gain`. Pure function of its fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioClip {
    #[serde(default)]
    pub wave: Wave,
    #[serde(default = "default_frequency")]
    pub frequency: f32,
    /// Linear sweep target reached at the end of the clip (`None` = constant frequency).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_end: Option<f32>,
    #[serde(default = "one")]
    pub seconds: f32,
    /// Peak amplitude after normalisation.
    #[serde(default = "half")]
    pub gain: f32,
    /// Sine overtones 2..=harmonics+1 added at 1/k amplitude.
    #[serde(default)]
    pub harmonics: u8,
    /// Two-pole low-pass cutoff (`None` = no filter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowpass_hz: Option<f32>,
    #[serde(default)]
    pub tremolo_hz: f32,
    #[serde(default)]
    pub tremolo_depth: f32,
    /// Random pitch wobble (0..1, fraction of +-50 %), changes every 10 ms (creaks).
    #[serde(default)]
    pub jitter: f32,
    #[serde(default = "short_ramp")]
    pub attack: f32,
    #[serde(default = "short_ramp")]
    pub release: f32,
    #[serde(default = "default_seed")]
    pub seed: u64,
}

impl Default for AudioClip {
    fn default() -> Self {
        Self {
            wave: Wave::Sine,
            frequency: default_frequency(),
            frequency_end: None,
            seconds: 1.0,
            gain: 0.5,
            harmonics: 0,
            lowpass_hz: None,
            tremolo_hz: 0.0,
            tremolo_depth: 0.0,
            jitter: 0.0,
            attack: short_ramp(),
            release: short_ramp(),
            seed: 1,
        }
    }
}

impl AudioClip {
    /// The definitions behind [`CLIP_PRESETS`]. Looping presets (`tone`, `engine`,
    /// `fountain`) hold whole oscillator / tremolo cycles and no ramps, so loops are seamless.
    pub fn preset(name: &str) -> Option<Self> {
        let base = Self::default();
        Some(match name {
            "tone" => Self {
                frequency: 440.0,
                gain: 0.4,
                attack: 0.0,
                release: 0.0,
                ..base
            },
            "engine" => Self {
                wave: Wave::Saw,
                frequency: 55.0,
                seconds: 2.0,
                gain: 0.6,
                harmonics: 0,
                lowpass_hz: Some(420.0),
                tremolo_hz: 6.0,
                tremolo_depth: 0.35,
                attack: 0.0,
                release: 0.0,
                ..base
            },
            "fountain" => Self {
                wave: Wave::Noise,
                seconds: 2.0,
                gain: 0.45,
                lowpass_hz: Some(1800.0),
                tremolo_hz: 1.5,
                tremolo_depth: 0.25,
                attack: 0.0,
                release: 0.0,
                seed: 7,
                ..base
            },
            "creak" => Self {
                wave: Wave::Saw,
                frequency: 170.0,
                frequency_end: Some(95.0),
                seconds: 0.9,
                gain: 0.5,
                lowpass_hz: Some(1400.0),
                tremolo_hz: 23.0,
                tremolo_depth: 0.6,
                jitter: 0.35,
                attack: 0.03,
                release: 0.25,
                seed: 3,
                ..base
            },
            "footstep" => Self {
                wave: Wave::Noise,
                seconds: 0.14,
                gain: 0.5,
                lowpass_hz: Some(900.0),
                attack: 0.002,
                release: 0.11,
                seed: 11,
                ..base
            },
            _ => return None,
        })
    }

    pub fn samples(&self) -> usize {
        (self.seconds * SAMPLE_RATE as f32).round() as usize
    }

    pub fn validate(&self) -> Result<()> {
        let within = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        ensure!(
            within(self.frequency, 20.0, 8000.0),
            "frequency must be 20..8000 Hz"
        );
        if let Some(end) = self.frequency_end {
            ensure!(
                within(end, 20.0, 8000.0),
                "frequency_end must be 20..8000 Hz"
            );
        }
        ensure!(
            within(self.seconds, 0.01, MAX_CLIP_SECONDS),
            "seconds must be 0.01..{MAX_CLIP_SECONDS}"
        );
        ensure!(within(self.gain, 0.0, 1.0), "gain must be 0..1");
        ensure!(self.harmonics <= 8, "harmonics must be 0..8");
        if let Some(cut) = self.lowpass_hz {
            ensure!(
                within(cut, 20.0, SAMPLE_RATE as f32 / 2.0),
                "lowpass_hz must be 20..11025 Hz"
            );
        }
        ensure!(
            within(self.tremolo_hz, 0.0, 40.0),
            "tremolo_hz must be 0..40 Hz"
        );
        ensure!(
            within(self.tremolo_depth, 0.0, 1.0),
            "tremolo_depth must be 0..1"
        );
        ensure!(within(self.jitter, 0.0, 1.0), "jitter must be 0..1");
        ensure!(
            within(self.attack, 0.0, self.seconds) && within(self.release, 0.0, self.seconds),
            "attack and release must be 0..seconds"
        );
        Ok(())
    }

    /// Renders the clip (mono, [`SAMPLE_RATE`]). Deterministic; the caller caches by content.
    pub fn synthesize(&self) -> Vec<i16> {
        let n = self.samples();
        let rate = SAMPLE_RATE as f32;
        let mut rng = self.seed.max(1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut random = move || {
            // xorshift64*: uniform in -1..1.
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            let v = rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
            v as f32 / (1u64 << 23) as f32 - 1.0
        };
        let alpha = self
            .lowpass_hz
            .map(|cut| 1.0 - (-std::f32::consts::TAU * cut / rate).exp());
        let (mut lp1, mut lp2, mut phase) = (0.0f32, 0.0f32, 0.0f32);
        let (mut wobble_from, mut wobble_to) = (0.0f32, random());
        let wobble_step = (rate * 0.01) as usize;
        let overtone_sum: f32 = (2..=u32::from(self.harmonics) + 1)
            .map(|h| 1.0 / h as f32)
            .sum();
        let end = self.frequency_end.unwrap_or(self.frequency);
        // Filter warm-up before the first recorded sample keeps loop starts free of a ramp.
        let warm = if alpha.is_some() { 2048 } else { 0 };
        let mut out = Vec::with_capacity(n);
        for i in 0..n + warm {
            let k = i.saturating_sub(warm);
            let t = k as f32 / rate;
            if i % wobble_step.max(1) == 0 {
                wobble_from = wobble_to;
                wobble_to = random();
            }
            let w = (i % wobble_step.max(1)) as f32 / wobble_step.max(1) as f32;
            let wobble = wobble_from + (wobble_to - wobble_from) * w;
            let sweep = self.frequency + (end - self.frequency) * (t / self.seconds);
            let frequency = sweep * (1.0 + self.jitter * 0.5 * wobble);
            phase = (phase + frequency / rate).fract();
            let x = std::f32::consts::TAU * phase;
            let mut s = match self.wave {
                Wave::Sine => x.sin(),
                Wave::Square => {
                    if phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                Wave::Saw => 2.0 * phase - 1.0,
                Wave::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
                Wave::Noise => random(),
            };
            if self.wave != Wave::Noise && self.harmonics > 0 {
                for h in 2..=u32::from(self.harmonics) + 1 {
                    s += (x * h as f32).sin() / h as f32;
                }
                s /= 1.0 + overtone_sum;
            }
            if let Some(a) = alpha {
                lp1 += a * (s - lp1);
                lp2 += a * (lp1 - lp2);
                s = lp2;
            }
            if i < warm {
                continue;
            }
            if self.tremolo_depth > 0.0 {
                let c = (std::f32::consts::TAU * self.tremolo_hz * t).cos();
                s *= 1.0 - self.tremolo_depth * 0.5 * (1.0 - c);
            }
            if self.attack > 0.0 && t < self.attack {
                s *= t / self.attack;
            }
            let left = self.seconds - t;
            if self.release > 0.0 && left < self.release {
                s *= (left / self.release).max(0.0);
            }
            out.push(s);
        }
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let scale = if peak > 0.0 { self.gain / peak } else { 0.0 };
        out.iter()
            .map(|v| (v * scale * 32767.0).round().clamp(-32768.0, 32767.0) as i16)
            .collect()
    }
}

/// `audio.library`: tone cues (gameplay events) and procedural clips (sources).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioLibrary {
    #[serde(default)]
    pub cues: BTreeMap<String, Tone>,
    #[serde(default)]
    pub clips: BTreeMap<String, AudioClip>,
}

impl AudioLibrary {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.cues.len() <= MAX_CUES, "audio cue budget exceeded");
        for (id, tone) in &self.cues {
            crate::valid_id(id)?;
            ensure!(
                tone.frequency.is_finite()
                    && (40.0..=4000.0).contains(&tone.frequency)
                    && (10..=150).contains(&tone.duration_ms)
                    && tone.gain.is_finite()
                    && (0.0..=0.5).contains(&tone.gain),
                "invalid tone"
            );
        }
        ensure!(
            self.clips.len() <= MAX_LIBRARY_CLIPS,
            "audio clip budget exceeded ({} > {MAX_LIBRARY_CLIPS} clips)",
            self.clips.len()
        );
        let mut samples = 0;
        for (id, clip) in &self.clips {
            crate::valid_id(id).with_context(|| format!("clip {id}"))?;
            clip.validate().with_context(|| format!("clip {id}"))?;
            samples += clip.samples();
        }
        ensure!(
            samples <= MAX_LIBRARY_SAMPLES,
            "audio clip memory budget exceeded ({samples} > {MAX_LIBRARY_SAMPLES} samples of 16-bit PCM at {SAMPLE_RATE} Hz)"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rolloff {
    Linear,
    #[default]
    Inverse,
}

fn default_max_distance() -> f32 {
    30.0
}
fn yes() -> bool {
    true
}

/// A sound attached to an entity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioSource {
    /// Clip ID in `audio.library.clips`.
    pub clip: String,
    #[serde(default = "one")]
    pub volume: f32,
    /// Playback rate (also shifts pitch).
    #[serde(default = "one")]
    pub pitch: f32,
    #[serde(default, rename = "loop")]
    pub looping: bool,
    /// Starts playing on the first step.
    #[serde(default)]
    pub autoplay: bool,
    /// false = 2D (no attenuation or panning).
    #[serde(default = "yes")]
    pub spatial: bool,
    #[serde(default = "one")]
    pub min_distance: f32,
    #[serde(default = "default_max_distance")]
    pub max_distance: f32,
    #[serde(default)]
    pub rolloff: Rolloff,
    /// Higher keeps its voice when more sources are audible than voices exist.
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl AudioSource {
    pub fn new(clip: &str) -> Self {
        Self {
            clip: clip.into(),
            volume: 1.0,
            pitch: 1.0,
            looping: false,
            autoplay: false,
            spatial: true,
            min_distance: 1.0,
            max_distance: default_max_distance(),
            rolloff: Rolloff::Inverse,
            priority: 0,
            enabled: true,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let within = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        crate::valid_id(&self.clip).context("clip")?;
        ensure!(within(self.volume, 0.0, 4.0), "volume must be 0..4");
        ensure!(within(self.pitch, 0.25, 4.0), "pitch must be 0.25..4");
        ensure!(
            within(self.min_distance, 0.01, 1000.0),
            "min_distance must be 0.01..1000 meters"
        );
        ensure!(
            within(self.max_distance, 0.02, 10000.0) && self.max_distance > self.min_distance,
            "max_distance must be greater than min_distance and at most 10000 meters"
        );
        ensure!(
            (-1000..=1000).contains(&self.priority),
            "priority must be -1000..1000"
        );
        Ok(())
    }
}

/// Which local axis the listener faces (its right ear is then on the right of that axis).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ListenerForward {
    /// Cameras look along local -Z.
    #[default]
    #[serde(rename = "-z")]
    MinusZ,
    /// Characters face local +Z (the engine's model convention).
    #[serde(rename = "+z")]
    PlusZ,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioListener {
    /// At most one authored listener may be active.
    #[serde(default = "yes")]
    pub active: bool,
    /// Master volume of everything this listener hears.
    #[serde(default = "one")]
    pub volume: f32,
    #[serde(default)]
    pub forward: ListenerForward,
}

impl Default for AudioListener {
    fn default() -> Self {
        Self {
            active: true,
            volume: 1.0,
            forward: ListenerForward::MinusZ,
        }
    }
}

impl AudioListener {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.volume.is_finite() && (0.0..=4.0).contains(&self.volume),
            "volume must be 0..4"
        );
        Ok(())
    }
}

/// Distance gain (0..1). `linear`: 1 up to `min`, straight down to 0 at `max`. `inverse`:
/// `min / d` (1 inside `min`), faded linearly to 0 over the last 10 % of `min..max` so the
/// sound ends at `max` without a step. Both are 0 at and beyond `max`.
pub fn attenuation(rolloff: Rolloff, min: f32, max: f32, distance: f32) -> f32 {
    // NaN distances are silent too.
    if distance.is_nan() || distance >= max {
        return 0.0;
    }
    if distance <= min {
        return 1.0;
    }
    match rolloff {
        Rolloff::Linear => 1.0 - (distance - min) / (max - min),
        Rolloff::Inverse => {
            let fade_start = max - 0.1 * (max - min);
            let g = min / distance;
            if distance > fade_start {
                g * (max - distance) / (max - fade_start)
            } else {
                g
            }
        }
    }
}

/// Stereo position -1 (left) .. 1 (right) of a source at `right` meters to the listener's right
/// and `forward` meters ahead (vertical offset ignored): the sine of the horizontal azimuth,
/// pulled toward the centre inside `min_distance` so a source passing through the listener
/// does not jump between ears.
pub fn pan(right: f32, forward: f32, min_distance: f32) -> f32 {
    let horizontal = (right * right + forward * forward).sqrt();
    if horizontal <= 1e-4 {
        return 0.0;
    }
    (right / horizontal * (horizontal / min_distance).min(1.0)).clamp(-1.0, 1.0)
}

/// Constant-power gains `(left, right)` for `pan` (-1..1): `cos` / `sin` of `(pan + 1) pi / 4`,
/// so `left^2 + right^2 = 1` everywhere and the centre is `0.7071` on both sides.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

pub fn library_schema() -> Value {
    json!({
        "cues": format!("stable ID -> {{frequency:40..4000 Hz,duration_ms:10..150,gain:0..0.5}} (<= {MAX_CUES}); played by audio.events for gameplay events"),
        "clips": format!("stable ID -> procedural clip (<= {MAX_LIBRARY_CLIPS}): {{wave:sine|square|saw|triangle|noise, frequency:20..8000 Hz (220), frequency_end:optional sweep target, seconds:0.01..{MAX_CLIP_SECONDS} (1), gain:peak 0..1 (0.5), harmonics:0..8, lowpass_hz:optional 20..11025, tremolo_hz:0..40, tremolo_depth:0..1, jitter:0..1 pitch wobble, attack/release:seconds (0.005), seed:u64}}; presets: {}", CLIP_PRESETS.join(", ")),
        "output": format!("mono {SAMPLE_RATE} Hz 16-bit PCM rendered once per definition (cached by content hash, never per play); mixer output stereo {SAMPLE_RATE} Hz through the native Windows host"),
        "budget": format!("{MAX_LIBRARY_SAMPLES} PCM samples across a library's clips; the runtime clip cache holds at most 32 MiB"),
        "events": "cues: simulation tick + monotonic sequence"
    })
}

pub fn source_schema() -> Value {
    json!({
        "clip": "clip ID in audio.library.clips (required)",
        "volume": "0..4, default 1",
        "pitch": "playback rate 0.25..4, default 1",
        "loop": "bool, default false; false = one-shot that stops at the clip end",
        "autoplay": "bool, default false: playing from the first step",
        "spatial": "bool, default true; false = 2D (gain = volume, centred)",
        "min_distance": "meters 0.01..1000, default 1: full gain inside",
        "max_distance": "meters > min_distance, <= 10000, default 30: silent at and beyond",
        "rolloff": "inverse (default; min/d, faded to 0 over the last 10% before max) | linear (1 at min to 0 at max)",
        "priority": "integer -1000..1000, default 0: voices go to higher priority, then nearer, then authored order",
        "enabled": "bool, default true",
        "pan": "constant power from the listener-relative horizontal direction (sin azimuth), pulled to the centre inside min_distance",
        "requires": "system audio.spatial (capability audio); spatial sources need core.transform",
        "actions": "audio.play {id, restart?} | audio.stop {id} | audio.inspect {id} | audio.summary; usable from runtime.bindings",
        "observe": "metrics `audio`: listener, voices with gain/left/right/pan per voiced source, stolen and culled counts"
    })
}

pub fn listener_schema() -> Value {
    json!({
        "active": "bool, default true; at most one active audio.listener per world (audio_listener_conflict otherwise)",
        "volume": "0..4, default 1 (master gain)",
        "forward": "\"-z\" (default, cameras) | \"+z\" (characters): the facing axis; the right ear is on its right",
        "default": "without an active audio.listener the first enabled active render.camera listens; without either spatial sources are silent (observe says why)",
        "requires": "system audio.spatial and core.transform"
    })
}

/// World-level checks run by `World::compile` on resolved components.
pub fn validate_world(
    world: &crate::World,
    sources: &[(&str, &AudioSource, bool)],
    listeners: &[(&str, &AudioListener, bool)],
) -> Result<()> {
    if sources.is_empty() && listeners.is_empty() {
        return Ok(());
    }
    let at = |id: &str, component: &str, field: &str| {
        format!(
            "/entities/@{}/components/{component}{field}",
            crate::local::escape(id)
        )
    };
    if sources.len() > MAX_AUDIO_SOURCES {
        return Err(crate::Diagnostic::error(
            "audio_budget",
            "/entities",
            format!(
                "audio source budget exceeded ({} > {MAX_AUDIO_SOURCES})",
                sources.len()
            ),
        ));
    }
    if !world.systems.iter().any(|s| s == "audio.spatial") {
        let (id, component) = sources
            .first()
            .map(|s| (s.0, "audio.source"))
            .unwrap_or((listeners[0].0, "audio.listener"));
        return Err(crate::Diagnostic::error(
            "invalid_audio",
            at(id, component, ""),
            format!("{component} needs the audio.spatial system (add it to systems)"),
        ));
    }
    let active: Vec<&str> = listeners
        .iter()
        .filter(|l| l.1.active)
        .map(|l| l.0)
        .collect();
    if active.len() > 1 {
        return Err(crate::Diagnostic::error(
            "audio_listener_conflict",
            at(active[1], "audio.listener", "/active"),
            format!(
                "more than one active audio.listener ({}); keep one active (set the others' active false)",
                active.join(", ")
            ),
        ));
    }
    for (id, _, transform) in listeners {
        if !transform {
            return Err(crate::Diagnostic::error(
                "invalid_audio",
                at(id, "audio.listener", ""),
                "audio.listener needs core.transform",
            ));
        }
    }
    let library: AudioLibrary = world
        .resources
        .get("audio.library")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?
        .unwrap_or_default();
    for (id, source, transform) in sources {
        if source.spatial && !transform {
            return Err(crate::Diagnostic::error(
                "invalid_audio",
                at(id, "audio.source", "/spatial"),
                "a spatial audio.source needs core.transform (or set spatial false)",
            ));
        }
        if !library.clips.contains_key(&source.clip) {
            let known: Vec<&str> = library.clips.keys().map(String::as_str).collect();
            return Err(crate::Diagnostic::error(
                "missing_clip",
                at(id, "audio.source", "/clip"),
                format!(
                    "audio.library has no clip {} (clips: {}); add one with `audio clip {} PRESET` ({})",
                    source.clip,
                    if known.is_empty() {
                        "none".to_owned()
                    } else {
                        known.join(", ")
                    },
                    source.clip,
                    CLIP_PRESETS.join("|")
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_defaults_round_trip_and_reject_bad_values() {
        let s: AudioSource = serde_json::from_value(json!({"clip":"engine"})).unwrap();
        assert_eq!(s, AudioSource::new("engine"));
        s.validate().unwrap();
        let full = json!({"clip":"door","volume":0.5,"pitch":1.5,"loop":true,"autoplay":true,"spatial":false,"min_distance":2.0,"max_distance":12.0,"rolloff":"linear","priority":3,"enabled":false});
        let s: AudioSource = serde_json::from_value(full.clone()).unwrap();
        assert_eq!(serde_json::to_value(&s).unwrap(), full);
        let bad = |v: Value| {
            let s: AudioSource = serde_json::from_value(v).unwrap();
            format!("{:#}", s.validate().unwrap_err())
        };
        assert!(bad(json!({"clip":"a","volume":5})).contains("volume"));
        assert!(bad(json!({"clip":"a","pitch":0.1})).contains("pitch"));
        assert!(bad(json!({"clip":"a","min_distance":5,"max_distance":5})).contains("max_distance"));
        assert!(bad(json!({"clip":"bad id"})).contains("clip"));
        assert!(serde_json::from_value::<AudioSource>(json!({"clip":"a","doppler":1})).is_err());
        let l: AudioListener = serde_json::from_value(json!({"forward":"+z"})).unwrap();
        assert_eq!(l.forward, ListenerForward::PlusZ);
        assert!(serde_json::from_value::<AudioListener>(json!({"forward":"z"})).is_err());
    }

    #[test]
    fn attenuation_curves_have_the_documented_values() {
        let lin = |d| attenuation(Rolloff::Linear, 2.0, 12.0, d);
        assert_eq!(lin(0.0), 1.0);
        assert_eq!(lin(2.0), 1.0);
        assert_eq!(lin(7.0), 0.5);
        assert_eq!(lin(12.0), 0.0);
        assert_eq!(lin(50.0), 0.0);
        let inv = |d| attenuation(Rolloff::Inverse, 1.0, 31.0, d);
        assert_eq!(inv(0.5), 1.0);
        assert_eq!(inv(2.0), 0.5);
        assert_eq!(inv(10.0), 0.1);
        // Fade over the last 10 % (28..31): halfway the gain is half of min/d.
        assert!((inv(29.5) - 0.5 / 29.5).abs() < 1e-6);
        assert_eq!(inv(31.0), 0.0);
        assert_eq!(inv(f32::NAN), 0.0);
        // Monotonic non-increasing.
        let mut last = 1.0;
        for k in 0..400 {
            let g = inv(k as f32 * 0.1);
            assert!(g <= last + 1e-7, "{k}");
            last = g;
        }
    }

    #[test]
    fn constant_power_panning() {
        let (l, r) = pan_gains(0.0);
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && (l - r).abs() < 1e-6);
        assert!(pan_gains(-1.0).1.abs() < 1e-6);
        assert!((pan_gains(1.0).1 - 1.0).abs() < 1e-6 && pan_gains(1.0).0.abs() < 1e-6);
        for k in -10..=10 {
            let (l, r) = pan_gains(k as f32 / 10.0);
            assert!((l * l + r * r - 1.0).abs() < 1e-5);
        }
        assert_eq!(pan(5.0, 0.0, 1.0), 1.0);
        assert_eq!(pan(-5.0, 0.0, 1.0), -1.0);
        assert!((pan(3.0, 3.0, 1.0) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(pan(0.0, -4.0, 1.0), 0.0, "straight behind is centred");
        assert!(
            (pan(0.5, 0.0, 1.0) - 0.5).abs() < 1e-6,
            "pulled in inside min"
        );
        assert_eq!(pan(0.0, 0.0, 1.0), 0.0);
    }

    #[test]
    fn presets_are_valid_deterministic_and_peak_normalised() {
        for name in CLIP_PRESETS {
            let clip = AudioClip::preset(name).unwrap();
            clip.validate().unwrap();
            let a = clip.synthesize();
            assert_eq!(a.len(), clip.samples(), "{name}");
            assert_eq!(a, clip.synthesize(), "{name} deterministic");
            let peak = a.iter().map(|v| v.unsigned_abs()).max().unwrap();
            let want = (clip.gain * 32767.0).round() as u16;
            assert!(peak.abs_diff(want) <= 1, "{name} peak {peak} want {want}");
        }
        assert!(AudioClip::preset("music").is_none());
    }

    #[test]
    fn library_budgets_and_bad_clips_are_reported() {
        let lib = |v: Value| {
            serde_json::from_value::<AudioLibrary>(v)
                .map_err(anyhow::Error::from)
                .and_then(|l| l.validate())
        };
        lib(json!({"clips":{"a":{"seconds":2}}})).unwrap();
        lib(json!({"cues":{"pop":{"frequency":520,"duration_ms":60,"gain":0.08}}})).unwrap();
        let clips: serde_json::Map<String, Value> = (0..30)
            .map(|i| (format!("c{i}"), json!({"seconds":MAX_CLIP_SECONDS})))
            .collect();
        let e = format!("{:#}", lib(json!({ "clips": clips })).unwrap_err());
        assert!(e.contains("memory budget"), "{e}");
        assert!(format!(
            "{:#}",
            lib(json!({"clips":{"a":{"seconds":30}}})).unwrap_err()
        )
        .contains("seconds"));
        assert!(format!(
            "{:#}",
            lib(json!({"clips":{"a":{"lowpass_hz":20000}}})).unwrap_err()
        )
        .contains("lowpass"));
    }
}
