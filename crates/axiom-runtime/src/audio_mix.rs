//! Real-time side of positional audio: the fixed voice-parameter block the simulation publishes
//! (a sequence lock over atomics, so neither side ever waits), the clip table, and the mixer the
//! audio thread runs per device buffer.
//!
//! The mixer owns only fixed-size state allocated when it is created. `render` never allocates,
//! frees or locks: voices read PCM through `Arc`s it already holds; replaced clip tables and
//! finished cue buffers are handed back to the caller (`set_clips`, `play_cue`,
//! `take_finished_cue`) so the last reference is dropped on the main thread, not here.
use std::sync::{
    atomic::{fence, AtomicU32, Ordering},
    Arc,
};

/// Positional voices mixed at once (sources beyond it are virtual; see `audio.spatial`).
pub const MAX_VOICES: usize = 32;
/// Simultaneous `audio.events` tone cues; a new cue replaces the oldest when all are busy.
pub const CUE_VOICES: usize = 8;
/// Simultaneous short one-shots started whole (`play_shot`); a new one replaces the oldest.
pub const SHOT_VOICES: usize = 8;
/// Largest `render` request in frames (stereo pairs).
pub const MAX_FRAMES: usize = 1024;
/// Recently started tokens remembered per path, so a one-shot sounds once whether its start
/// arrives through the voice block, `play_shot`, or both.
const TOKEN_RING: usize = 64;

/// Fixed ring of recent tokens (no allocation).
#[derive(Clone, Copy)]
struct Tokens {
    ring: [u32; TOKEN_RING],
    at: usize,
}
impl Default for Tokens {
    fn default() -> Self {
        Self {
            ring: [0; TOKEN_RING],
            at: 0,
        }
    }
}
impl Tokens {
    fn push(&mut self, token: u32) {
        self.ring[self.at] = token;
        self.at = (self.at + 1) % TOKEN_RING;
    }
    fn contains(&self, token: u32) -> bool {
        token != 0 && self.ring.contains(&token)
    }
}
const RATE: f64 = axiom_core::audio::SAMPLE_RATE as f64;

/// What one voice should play, computed by the simulation each fixed step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VoiceParams {
    /// Content hash of the clip (`ClipTable` key); 0 = silent voice.
    pub clip: u64,
    /// Changes whenever the source (re)starts; a new token restarts the voice at `offset`.
    pub token: u32,
    /// Seconds into the clip where a (re)started voice begins (the simulation's play time).
    pub offset: f32,
    pub pitch: f32,
    pub left: f32,
    pub right: f32,
    pub looping: bool,
    /// Follows the music volume (else the sound-effect volume).
    pub music: bool,
}

impl VoiceParams {
    pub fn active(&self) -> bool {
        self.clip != 0
    }
}

const WORDS: usize = 8;

/// Single-writer, single-reader sequence lock over the voice block. The writer never waits;
/// the reader retries a bounded number of times and otherwise keeps its previous copy.
pub struct SharedVoices {
    seq: AtomicU32,
    words: [[AtomicU32; WORDS]; MAX_VOICES],
}

impl Default for SharedVoices {
    fn default() -> Self {
        Self {
            seq: AtomicU32::new(0),
            words: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))),
        }
    }
}

impl SharedVoices {
    /// Publishes a whole block (one writer only: the main thread).
    pub fn publish(&self, voices: &[VoiceParams; MAX_VOICES]) {
        self.seq.fetch_add(1, Ordering::Relaxed);
        fence(Ordering::Release);
        for (w, v) in self.words.iter().zip(voices) {
            let values = [
                v.clip as u32,
                (v.clip >> 32) as u32,
                v.token,
                v.offset.to_bits(),
                v.pitch.to_bits(),
                v.left.to_bits(),
                v.right.to_bits(),
                u32::from(v.looping) | (u32::from(v.music) << 1),
            ];
            for (a, x) in w.iter().zip(values) {
                a.store(x, Ordering::Relaxed);
            }
        }
        self.seq.fetch_add(1, Ordering::Release);
    }

    /// Copies a consistent block into `out`; false (and `out` untouched) when the writer was
    /// busy for both attempts.
    pub fn read(&self, out: &mut [VoiceParams; MAX_VOICES]) -> bool {
        for _ in 0..2 {
            let before = self.seq.load(Ordering::Acquire);
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let mut copy = [VoiceParams::default(); MAX_VOICES];
            for (w, v) in self.words.iter().zip(copy.iter_mut()) {
                let x: [u32; WORDS] = std::array::from_fn(|k| w[k].load(Ordering::Relaxed));
                *v = VoiceParams {
                    clip: u64::from(x[0]) | (u64::from(x[1]) << 32),
                    token: x[2],
                    offset: f32::from_bits(x[3]),
                    pitch: f32::from_bits(x[4]),
                    left: f32::from_bits(x[5]),
                    right: f32::from_bits(x[6]),
                    looping: x[7] & 1 != 0,
                    music: x[7] & 2 != 0,
                };
            }
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == before {
                *out = copy;
                return true;
            }
        }
        false
    }
}

/// Clip PCM by content hash (sorted for binary search).
pub struct ClipTable {
    entries: Vec<(u64, Arc<[i16]>)>,
}

impl ClipTable {
    pub fn new(mut entries: Vec<(u64, Arc<[i16]>)>) -> Self {
        entries.sort_by_key(|e| e.0);
        entries.dedup_by_key(|e| e.0);
        Self { entries }
    }
    pub fn get(&self, clip: u64) -> Option<&[i16]> {
        self.entries
            .binary_search_by_key(&clip, |e| e.0)
            .ok()
            .map(|i| &*self.entries[i].1)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// PCM bytes referenced (shared with the clip cache).
    pub fn bytes(&self) -> usize {
        self.entries.iter().map(|e| e.1.len() * 2).sum()
    }
}

#[derive(Clone, Copy, Default)]
struct MixVoice {
    token: u32,
    clip: u64,
    pitch: f32,
    looping: bool,
    pos: f64,
    left: f32,
    right: f32,
    live: bool,
    music: bool,
}

/// Counters of the mixer (read by the audio thread's owner).
#[derive(Clone, Copy, Debug, Default)]
pub struct MixStats {
    pub renders: u64,
    /// Voices that produced sound in the last render.
    pub voices: u32,
    pub clipped_samples: u64,
    pub cues_started: u64,
    /// One-shots started whole by `play_shot`.
    pub shots_started: u64,
    /// `play_shot` calls for a token the voice block had already started (played once).
    pub shots_skipped: u64,
}

pub struct Mixer {
    table: Option<Arc<ClipTable>>,
    params: [VoiceParams; MAX_VOICES],
    voices: [MixVoice; MAX_VOICES],
    cues: [(Option<Arc<[i16]>>, usize); CUE_VOICES],
    next_cue: usize,
    shots: [MixVoice; SHOT_VOICES],
    next_shot: usize,
    voiced_tokens: Tokens,
    shot_tokens: Tokens,
    scratch: Box<[f32; 2 * MAX_FRAMES]>,
    /// Music voices mix here (their own volume), sound effects and cues in `scratch`.
    music_scratch: Box<[f32; 2 * MAX_FRAMES]>,
    /// Master / music / sound-effect gains now and the ones they ramp to across the next
    /// buffer (player settings).
    master: f32,
    master_target: f32,
    music: f32,
    music_target: f32,
    sfx: f32,
    sfx_target: f32,
    pub stats: MixStats,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            table: None,
            params: [VoiceParams::default(); MAX_VOICES],
            voices: [MixVoice::default(); MAX_VOICES],
            cues: std::array::from_fn(|_| (None, 0)),
            next_cue: 0,
            shots: [MixVoice::default(); SHOT_VOICES],
            next_shot: 0,
            voiced_tokens: Tokens::default(),
            shot_tokens: Tokens::default(),
            scratch: Box::new([0.0; 2 * MAX_FRAMES]),
            music_scratch: Box::new([0.0; 2 * MAX_FRAMES]),
            master: 1.0,
            master_target: 1.0,
            music: 1.0,
            music_target: 1.0,
            sfx: 1.0,
            sfx_target: 1.0,
            stats: MixStats::default(),
        }
    }
}

/// Adds `v` (ramping its gains to `target` across the buffer) into `mix`; ends it at the clip
/// end unless it loops.
fn mix_voice(v: &mut MixVoice, pcm: &[i16], mix: &mut [f32], target: (f32, f32)) {
    let frames = mix.len() / 2;
    let len = pcm.len();
    let step = f64::from(v.pitch.max(0.0));
    let (dl, dr) = (
        (target.0 - v.left) / frames.max(1) as f32,
        (target.1 - v.right) / frames.max(1) as f32,
    );
    let (mut gl, mut gr) = (v.left, v.right);
    for frame in mix.as_chunks_mut::<2>().0.iter_mut() {
        gl += dl;
        gr += dr;
        let i = v.pos as usize;
        if i >= len {
            v.live = false;
            break;
        }
        let frac = (v.pos - i as f64) as f32;
        let next = if i + 1 < len {
            pcm[i + 1]
        } else if v.looping {
            pcm[0]
        } else {
            0
        };
        let s =
            (f32::from(pcm[i]) + (f32::from(next) - f32::from(pcm[i])) * frac) * (1.0 / 32768.0);
        frame[0] += s * gl;
        frame[1] += s * gr;
        v.pos += step;
        if v.pos >= len as f64 && v.looping {
            // Modulo, not one subtraction: a clip shorter than the step (a one-frame WAV at
            // pitch 4) would otherwise stay past the end and fall silent.
            v.pos %= len as f64;
        }
    }
    (v.left, v.right) = target;
}

impl Mixer {
    /// The master gain (0..1) the next buffer ramps to.
    pub fn set_master(&mut self, gain: f32) {
        self.master_target = gain.clamp(0.0, 1.0);
    }
    /// The music and sound-effect gains (0..1) the next buffer ramps to.
    pub fn set_buses(&mut self, music: f32, sfx: f32) {
        self.music_target = music.clamp(0.0, 1.0);
        self.sfx_target = sfx.clamp(0.0, 1.0);
    }
    /// Replaces the voice block (from `SharedVoices::read` or a test).
    pub fn set_params(&mut self, params: &[VoiceParams; MAX_VOICES]) {
        self.params = *params;
    }
    /// Installs a clip table, returning the previous one for the caller to drop elsewhere.
    pub fn set_clips(&mut self, table: Arc<ClipTable>) -> Option<Arc<ClipTable>> {
        self.table.replace(table)
    }
    /// Starts a cue (mono, played on both channels), returning the buffer it displaced.
    pub fn play_cue(&mut self, pcm: Arc<[i16]>) -> Option<Arc<[i16]>> {
        let slot = (0..CUE_VOICES)
            .map(|k| (self.next_cue + k) % CUE_VOICES)
            .find(|&k| {
                self.cues[k]
                    .0
                    .as_ref()
                    .is_none_or(|p| self.cues[k].1 >= p.len())
            })
            .unwrap_or(self.next_cue);
        self.next_cue = (slot + 1) % CUE_VOICES;
        self.stats.cues_started += 1;
        let old = self.cues[slot].0.replace(pcm);
        self.cues[slot].1 = 0;
        old
    }
    /// Starts a short one-shot whole (at its offset, fixed gains, to the clip end) unless the
    /// voice block already started that token; a later voice block holding the token does not
    /// start it again. With every shot voice busy the oldest is replaced.
    pub fn play_shot(&mut self, p: &VoiceParams) {
        if !p.active()
            || p.looping
            || self.voiced_tokens.contains(p.token)
            || self.shot_tokens.contains(p.token)
        {
            self.stats.shots_skipped += 1;
            return;
        }
        let slot = (0..SHOT_VOICES)
            .map(|k| (self.next_shot + k) % SHOT_VOICES)
            .find(|&k| !self.shots[k].live)
            .unwrap_or(self.next_shot);
        self.next_shot = (slot + 1) % SHOT_VOICES;
        self.shot_tokens.push(p.token);
        self.stats.shots_started += 1;
        self.shots[slot] = MixVoice {
            token: p.token,
            clip: p.clip,
            pitch: p.pitch,
            looping: false,
            pos: f64::from(p.offset.max(0.0)) * RATE,
            left: p.left,
            right: p.right,
            live: true,
            music: p.music,
        };
    }
    /// A finished cue buffer to drop off the audio thread (call until `None`).
    pub fn take_finished_cue(&mut self) -> Option<Arc<[i16]>> {
        self.cues
            .iter_mut()
            .find(|(p, at)| p.as_ref().is_some_and(|p| *at >= p.len()))
            .and_then(|(p, _)| p.take())
    }

    /// Mixes `out.len() / 2` stereo frames (interleaved L R, 16-bit) at the clip rate. Gains
    /// ramp linearly across the buffer from the previous to the new parameters; a voice whose
    /// token changes first fades its old sound out for one buffer, then starts at its offset.
    pub fn render(&mut self, out: &mut [i16]) {
        let frames = (out.len() / 2).min(MAX_FRAMES);
        let mix = &mut self.scratch[..frames * 2];
        mix.fill(0.0);
        let music = &mut self.music_scratch[..frames * 2];
        music.fill(0.0);
        let mut voiced = 0;
        let table = self.table.as_deref();
        for (p, v) in self.params.iter().zip(self.voices.iter_mut()) {
            let wants = p.active() && table.is_some_and(|t| t.get(p.clip).is_some());
            let mut target = (0.0, 0.0);
            if !wants && !v.live {
                // Forget the token: the same source may get this voice back later and must
                // then start again from its offset.
                v.token = 0;
                continue;
            }
            if wants && (v.token != p.token || v.clip != p.clip) {
                if !(v.live && v.left.max(v.right) > 1e-3) {
                    let len = table.and_then(|t| t.get(p.clip)).map_or(1, <[i16]>::len) as f64;
                    let mut pos = f64::from(p.offset.max(0.0)) * RATE;
                    if p.looping {
                        pos %= len;
                    }
                    // A one-shot `play_shot` already started whole is not started twice.
                    let shot = !p.looping && self.shot_tokens.contains(p.token);
                    if !shot {
                        self.voiced_tokens.push(p.token);
                    }
                    *v = MixVoice {
                        token: p.token,
                        clip: p.clip,
                        pitch: p.pitch,
                        looping: p.looping,
                        pos,
                        left: 0.0,
                        right: 0.0,
                        live: pos < len && !shot,
                        music: p.music,
                    };
                    target = (p.left, p.right);
                }
            } else if wants {
                v.pitch = p.pitch;
                v.looping = p.looping;
                v.music = p.music;
                target = (p.left, p.right);
            }
            if !v.live {
                continue;
            }
            let Some(pcm) = table.and_then(|t| t.get(v.clip)) else {
                v.live = false;
                continue;
            };
            let into = if v.music { &mut *music } else { &mut *mix };
            mix_voice(v, pcm, into, target);
            voiced += 1;
            if !wants {
                // Faded out across this buffer.
                v.live = false;
                v.token = 0;
            }
        }
        for v in self.shots.iter_mut().filter(|v| v.live) {
            match table.and_then(|t| t.get(v.clip)) {
                Some(pcm) => {
                    let into = if v.music { &mut *music } else { &mut *mix };
                    mix_voice(v, pcm, into, (v.left, v.right))
                }
                None => v.live = false,
            }
        }
        for (pcm, at) in &mut self.cues {
            let Some(pcm) = pcm else { continue };
            for frame in mix.as_chunks_mut::<2>().0.iter_mut() {
                if *at >= pcm.len() {
                    break;
                }
                let s = f32::from(pcm[*at]) * (1.0 / 32768.0);
                frame[0] += s;
                frame[1] += s;
                *at += 1;
            }
        }
        // Player volumes, each ramped across the buffer so a change never clicks:
        // (sound effects + music) x master.
        let n = frames.max(1) as f32;
        let ramp = |from: f32, to: f32| (from, (to - from) / n);
        let (m0, dm) = ramp(self.master, self.master_target);
        let (u0, du) = ramp(self.music, self.music_target);
        let (s0, ds) = ramp(self.sfx, self.sfx_target);
        for (i, ((o, m), u)) in out.iter_mut().zip(mix.iter()).zip(music.iter()).enumerate() {
            let f = (i / 2) as f32;
            let x = ((m * (s0 + ds * f) + u * (u0 + du * f)) * (m0 + dm * f) * 32768.0).round();
            if !(-32768.0..=32767.0).contains(&x) {
                self.stats.clipped_samples += 1;
            }
            *o = x.clamp(-32768.0, 32767.0) as i16;
        }
        out[frames * 2..].fill(0);
        self.master = self.master_target;
        self.music = self.music_target;
        self.sfx = self.sfx_target;
        self.stats.voices = voiced;
        self.stats.renders += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(samples: Vec<i16>) -> Arc<ClipTable> {
        Arc::new(ClipTable::new(vec![(7, samples.into())]))
    }
    fn voice(left: f32, right: f32, token: u32) -> VoiceParams {
        VoiceParams {
            clip: 7,
            token,
            offset: 0.0,
            pitch: 1.0,
            left,
            right,
            looping: true,
            music: false,
        }
    }

    /// Plan S2: music voices follow the music volume, sound effects the sfx volume, both
    /// times the master volume; each change ramps across one buffer.
    #[test]
    fn music_and_sound_effect_voices_follow_their_own_volume() {
        let render = |music: f32, sfx: f32| -> (i16, i16) {
            let mut m = Mixer::default();
            m.set_clips(table(vec![10_000; 4096]));
            let mut block = [VoiceParams::default(); MAX_VOICES];
            block[0] = voice(0.5, 0.5, 1);
            block[1] = VoiceParams {
                music: true,
                ..voice(0.25, 0.25, 2)
            };
            m.set_params(&block);
            m.set_buses(music, sfx);
            let mut out = vec![0i16; 512];
            for _ in 0..3 {
                m.render(&mut out);
            }
            (out[0], out[511])
        };
        let both = render(1.0, 1.0);
        let sfx_only = render(0.0, 1.0);
        let music_only = render(1.0, 0.0);
        // 10000 x 0.5 (sfx voice) and 10000 x 0.25 (music voice).
        assert_eq!(sfx_only, (5000, 5000), "{sfx_only:?}");
        assert_eq!(music_only, (2500, 2500), "{music_only:?}");
        assert_eq!(both, (7500, 7500), "{both:?}");
        assert_eq!(render(0.0, 0.0), (0, 0));
    }

    #[test]
    fn the_music_flag_survives_the_voice_block() {
        let shared = SharedVoices::default();
        let mut block = [VoiceParams::default(); MAX_VOICES];
        block[1] = VoiceParams {
            music: true,
            looping: false,
            ..voice(0.3, 0.3, 4)
        };
        shared.publish(&block);
        let mut out = [VoiceParams::default(); MAX_VOICES];
        assert!(shared.read(&mut out));
        assert!(out[1].music && !out[1].looping);
    }

    #[test]
    fn seqlock_round_trips_a_block() {
        let shared = SharedVoices::default();
        let mut block = [VoiceParams::default(); MAX_VOICES];
        block[3] = voice(0.25, 0.75, 9);
        block[3].clip = u64::MAX - 5;
        block[3].offset = 1.5;
        shared.publish(&block);
        let mut out = [VoiceParams::default(); MAX_VOICES];
        assert!(shared.read(&mut out));
        assert_eq!(out, block);
    }

    #[test]
    fn gains_ramp_in_pan_and_reach_the_target() {
        let mut m = Mixer::default();
        assert!(m.set_clips(table(vec![10_000; 4096])).is_none());
        let mut p = [VoiceParams::default(); MAX_VOICES];
        p[0] = voice(1.0, 0.0, 1);
        m.set_params(&p);
        let mut out = vec![0i16; 2 * 256];
        m.render(&mut out);
        // Ramp from silence: the first frame is quiet, the last is at full gain; right is silent.
        assert!(out[0] < 100 && out[510] > 9_900, "{} {}", out[0], out[510]);
        assert!(out.iter().skip(1).step_by(2).all(|&r| r == 0));
        m.render(&mut out);
        assert!(out.chunks(2).all(|f| f[0] > 9_990 && f[1] == 0));
        assert_eq!(m.stats.voices, 1);
        // Pan to the right: left fades out across one buffer.
        p[0] = voice(0.0, 1.0, 1);
        m.set_params(&p);
        m.render(&mut out);
        assert!(out[0] > 9_000 && out[510] < 100 && out[511] > 9_900);
    }

    #[test]
    fn one_shots_end_and_tokens_restart_after_a_fade() {
        let mut m = Mixer::default();
        m.set_clips(table(vec![8_000; 300]));
        let mut p = [VoiceParams::default(); MAX_VOICES];
        p[0] = VoiceParams {
            looping: false,
            ..voice(1.0, 1.0, 1)
        };
        m.set_params(&p);
        let mut out = vec![0i16; 2 * 256];
        m.render(&mut out);
        m.render(&mut out);
        // 300 samples: the second buffer holds the last 44 frames, then silence.
        assert!(out[2 * 43] > 7_900 && out[2 * 44] == 0 && out[511] == 0);
        m.render(&mut out);
        assert!(
            out.iter().all(|&s| s == 0),
            "an ended one-shot stays silent"
        );
        assert_eq!(m.stats.voices, 0);
        // A new token restarts it.
        p[0].token = 2;
        m.set_params(&p);
        m.render(&mut out);
        assert!(out[510] > 7_900);
    }

    #[test]
    fn silent_voices_fade_out_then_stop_and_cues_mix_in() {
        let mut m = Mixer::default();
        m.set_clips(table(vec![10_000; 4096]));
        let mut p = [VoiceParams::default(); MAX_VOICES];
        p[5] = voice(0.5, 0.5, 3);
        m.set_params(&p);
        let mut out = vec![0i16; 2 * 128];
        m.render(&mut out);
        m.render(&mut out);
        p[5] = VoiceParams::default();
        m.set_params(&p);
        m.render(&mut out);
        assert!(out[0] > 4_000 && out[254] < 100, "faded out");
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 0));
        assert!(m.play_cue(vec![1_000i16; 10].into()).is_none());
        m.render(&mut out);
        assert_eq!(&out[..4], &[1_000, 1_000, 1_000, 1_000]);
        assert_eq!(out[20], 0);
        assert!(m.take_finished_cue().is_some());
        assert!(m.take_finished_cue().is_none());
    }

    #[test]
    fn a_voice_given_back_to_the_same_source_starts_again() {
        // The simulation frees a voice (stolen) and later gives the same slot back to the same
        // source with the same token: it must sound again, not stay silent.
        let mut m = Mixer::default();
        m.set_clips(table(vec![10_000; 4096]));
        let mut p = [VoiceParams::default(); MAX_VOICES];
        p[2] = voice(1.0, 1.0, 4);
        let mut out = vec![0i16; 2 * 128];
        for params in [p, p, [VoiceParams::default(); MAX_VOICES]] {
            m.set_params(&params);
            m.render(&mut out);
        }
        m.set_params(&[VoiceParams::default(); MAX_VOICES]);
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 0));
        m.set_params(&p);
        m.render(&mut out);
        assert!(out[254] > 9_900, "{}", out[254]);
    }

    #[test]
    fn a_one_shot_sounds_once_through_either_path() {
        let shot = VoiceParams {
            looping: false,
            ..voice(1.0, 1.0, 9)
        };
        let mut out = vec![0i16; 2 * 64];
        // Shot first: a later voice block with the same token does not start it again.
        let mut m = Mixer::default();
        m.set_clips(table(vec![10_000; 100]));
        m.play_shot(&shot);
        let mut p = [VoiceParams::default(); MAX_VOICES];
        p[0] = shot;
        m.set_params(&p);
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 10_000), "single, not doubled");
        assert_eq!((m.stats.shots_started, m.stats.voices), (1, 0));
        m.render(&mut out);
        assert!(out[..2 * 36].iter().all(|&s| s == 10_000) && out[2 * 36] == 0);
        // Voice block first: the shot is skipped.
        let mut m = Mixer::default();
        m.set_clips(table(vec![10_000; 100]));
        m.set_params(&p);
        m.render(&mut out);
        m.play_shot(&shot);
        assert_eq!((m.stats.shots_started, m.stats.shots_skipped), (0, 1));
    }

    #[test]
    fn a_one_sample_loop_at_high_pitch_keeps_playing() {
        let mut m = Mixer::default();
        m.set_clips(table(vec![10_000]));
        let mut p = [VoiceParams::default(); MAX_VOICES];
        p[0] = VoiceParams {
            pitch: 4.0,
            ..voice(1.0, 1.0, 1)
        };
        m.set_params(&p);
        let mut out = vec![0i16; 2 * 128];
        m.render(&mut out);
        m.render(&mut out);
        assert!(
            out.iter().all(|&s| s > 9_900),
            "the loop must keep sounding in the second buffer: {:?}",
            &out[..8]
        );
        assert_eq!(m.stats.voices, 1);
    }

    #[test]
    fn unknown_clips_are_silent_and_loud_mixes_clamp() {
        let mut m = Mixer::default();
        let mut p = [VoiceParams::default(); MAX_VOICES];
        p[0] = voice(1.0, 1.0, 1);
        m.set_params(&p);
        let mut out = vec![1i16; 64];
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 0), "no table yet");
        m.set_clips(table(vec![30_000; 4096]));
        for v in p.iter_mut() {
            *v = voice(1.0, 1.0, 1);
        }
        m.set_params(&p);
        m.render(&mut out);
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 32767));
        assert!(m.stats.clipped_samples > 0);
    }
}
