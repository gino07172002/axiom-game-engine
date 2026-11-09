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
/// Largest `render` request in frames (stereo pairs).
pub const MAX_FRAMES: usize = 1024;
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
                u32::from(v.looping),
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
                    looping: x[7] != 0,
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
}

/// Counters of the mixer (read by the audio thread's owner).
#[derive(Clone, Copy, Debug, Default)]
pub struct MixStats {
    pub renders: u64,
    /// Voices that produced sound in the last render.
    pub voices: u32,
    pub clipped_samples: u64,
    pub cues_started: u64,
}

pub struct Mixer {
    table: Option<Arc<ClipTable>>,
    params: [VoiceParams; MAX_VOICES],
    voices: [MixVoice; MAX_VOICES],
    cues: [(Option<Arc<[i16]>>, usize); CUE_VOICES],
    next_cue: usize,
    scratch: Box<[f32; 2 * MAX_FRAMES]>,
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
            scratch: Box::new([0.0; 2 * MAX_FRAMES]),
            stats: MixStats::default(),
        }
    }
}

impl Mixer {
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
                    *v = MixVoice {
                        token: p.token,
                        clip: p.clip,
                        pitch: p.pitch,
                        looping: p.looping,
                        pos,
                        left: 0.0,
                        right: 0.0,
                        live: pos < len,
                    };
                    target = (p.left, p.right);
                }
            } else if wants {
                v.pitch = p.pitch;
                v.looping = p.looping;
                target = (p.left, p.right);
            }
            if !v.live {
                continue;
            }
            let Some(pcm) = table.and_then(|t| t.get(v.clip)) else {
                v.live = false;
                continue;
            };
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
                let s = (f32::from(pcm[i]) + (f32::from(next) - f32::from(pcm[i])) * frac)
                    * (1.0 / 32768.0);
                frame[0] += s * gl;
                frame[1] += s * gr;
                v.pos += step;
                if v.pos >= len as f64 && v.looping {
                    v.pos -= len as f64;
                }
            }
            (v.left, v.right) = target;
            voiced += 1;
            if !wants {
                // Faded out across this buffer.
                v.live = false;
                v.token = 0;
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
        for (o, m) in out.iter_mut().zip(mix.iter()) {
            let x = (m * 32768.0).round();
            if !(-32768.0..=32767.0).contains(&x) {
                self.stats.clipped_samples += 1;
            }
            *o = x.clamp(-32768.0, 32767.0) as i16;
        }
        out[frames * 2..].fill(0);
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
        }
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
