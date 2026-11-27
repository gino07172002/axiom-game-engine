//! `audio.spatial`: runtime side of `audio.source` / `audio.listener` (see `axiom_core::audio`).
//!
//! Built once per runtime: every clip a source uses is rendered to PCM through a process-wide
//! cache keyed by the clip definition's content hash (bounded memory, explicit error when the
//! budget is exceeded), and sources become typed rows. Each fixed step advances play time
//! (one-shots stop at the clip end), computes distance gain and constant-power pan against the
//! listener (the authored `audio.listener`, else the first enabled active camera), and gives the
//! [`MAX_VOICES`] voices to the audible sources with the highest priority, then nearest, then
//! authored order. A voiced source keeps its voice slot while it stays selected. The result is a
//! fixed `[VoiceParams; MAX_VOICES]` block the host publishes to the audio thread; a steady step
//! does not allocate. Everything is also in `observe` (`audio`), so headless runs can check what
//! a listener would hear.
use crate::audio_mix::{ClipTable, VoiceParams, MAX_VOICES};
use crate::world::*;
use anyhow::{bail, ensure, Context, Result};
use axiom_core::audio::{
    attenuation, pan, pan_gains, AudioClip, AudioLibrary, AudioListener, AudioSource, ClipFile,
    ListenerForward, MAX_FILE_BYTES, MAX_LIBRARY_SAMPLES, SAMPLE_RATE,
};
use glam::Vec3;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    any::Any,
    collections::BTreeMap,
    io::Read,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
};

/// Rendered clip PCM kept across runtime rebuilds (all runtimes of the process together).
pub const CLIP_CACHE_BYTES: usize = 32 << 20;
/// Gain below which a playing source is inaudible (-80 dB) and takes no voice.
const MIN_AUDIBLE: f32 = 1e-4;
/// One-shots at most this long (clip seconds / pitch) that end while voiced are also handed to
/// the output whole ([`AudioSpatialSystem::shots`]): they can start and end between two frames'
/// voice blocks (or two mixer reads), so the voice block alone might never sound them.
pub const SHOT_SECONDS: f32 = 0.25;
/// Ended one-shots kept for the output (oldest dropped first, counted).
const SHOTS: usize = 32;

/// What a decoded file clip held (`observe`, the editor's Inspector).
#[derive(Clone, Debug)]
pub struct FileInfo {
    /// As authored after pack resolution (absolute inside the pack).
    pub path: String,
    pub format: &'static str,
    pub channels: u16,
    pub rate: u32,
    /// Source duration.
    pub seconds: f64,
    /// Encoded size.
    pub bytes: u64,
    /// Wall time of the one decode (on the thread that built the runtime or loaded the pack).
    pub decode_ms: f64,
}

struct CacheEntry {
    text: String,
    pcm: Arc<[i16]>,
    file: Option<FileInfo>,
}

static CLIP_CACHE: Mutex<BTreeMap<u64, CacheEntry>> = Mutex::new(BTreeMap::new());

fn fnv1a(text: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in text.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h.max(1)
}

/// The encoded bytes of a file clip (bounded by `MAX_FILE_BYTES`).
fn file_bytes(file: &ClipFile) -> Result<Vec<u8>> {
    let path = std::path::Path::new(&file.path);
    let size = std::fs::metadata(path)
        .with_context(|| format!("audio file {} not found", file.path))?
        .len();
    let too_big = |size: u64| {
        anyhow::anyhow!(
            "audio file {} is {size} bytes (budget {MAX_FILE_BYTES})",
            file.path
        )
    };
    if size > MAX_FILE_BYTES {
        return Err(too_big(size));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .with_context(|| format!("audio file {} could not be read", file.path))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(too_big(bytes.len() as u64));
    }
    Ok(bytes)
}

/// The cache text of a file clip: content hash (SHA-256) and gain, so the same file under two
/// paths is decoded once and a changed file is decoded again.
fn file_text(file: &ClipFile, bytes: &[u8]) -> String {
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("file:{hex}:gain:{:08x}", file.gain.to_bits())
}

/// Fast 64-bit content fingerprint (foldhash, fixed seed) that confirms a file still holds
/// what its remembered SHA-256 text was computed from. Not cryptographic: it only has to
/// tell a replaced file from the one hashed before under the same stamp.
fn fingerprint(bytes: &[u8]) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = foldhash::quality::FixedState::with_seed(0).build_hasher();
    hasher.write(bytes);
    hasher.finish()
}

/// What a file looked like when its cache text was computed: path, size, modification time
/// and gain bits.
type FileStamp = (String, u64, std::time::SystemTime, u32);
/// Cache texts of files by stamp, with the content fingerprint they were computed from
/// (bounded). A build reads every file (the content is always checked, so a replaced file
/// with the same size and modification time is decoded again) but skips the SHA-256 when
/// the stamp and the fingerprint both match.
static FILE_TEXTS: Mutex<BTreeMap<FileStamp, (u64, String)>> = Mutex::new(BTreeMap::new());
const FILE_TEXTS_MAX: usize = 256;

fn file_stamp(file: &ClipFile) -> Option<FileStamp> {
    let meta = std::fs::metadata(&file.path).ok()?;
    Some((
        file.path.clone(),
        meta.len(),
        meta.modified().ok()?,
        file.gain.to_bits(),
    ))
}

/// PCM of `clip` by content hash: synthesised or decoded on a miss (outside the lock), shared
/// afterwards. Entries no runtime references are evicted when the cache would exceed its
/// budget; a clip that still does not fit is an error, not a silent drop. File clips also
/// return what the file held.
pub(crate) fn cached_clip(clip: &AudioClip) -> Result<(u64, Arc<[i16]>, Option<FileInfo>)> {
    clip.validate()?;
    fn poisoned<T>(_: T) -> anyhow::Error {
        anyhow::anyhow!("audio clip cache lock poisoned")
    }
    let (text, bytes) = match &clip.file {
        Some(file) => {
            // Stamped before reading, so a file replaced during the read is hashed again.
            let stamp = file_stamp(file);
            let bytes = file_bytes(file)?;
            let print = fingerprint(&bytes);
            let known = match &stamp {
                Some(stamp) => FILE_TEXTS
                    .lock()
                    .map_err(poisoned)?
                    .get(stamp)
                    .filter(|(p, _)| *p == print)
                    .map(|(_, text)| text.clone()),
                None => None,
            };
            let text = match known {
                Some(text) => text,
                None => {
                    let text = file_text(file, &bytes);
                    if let Some(stamp) = stamp {
                        let mut texts = FILE_TEXTS.lock().map_err(poisoned)?;
                        if texts.len() >= FILE_TEXTS_MAX {
                            texts.clear();
                        }
                        texts.insert(stamp, (print, text.clone()));
                    }
                    text
                }
            };
            (text, Some(bytes))
        }
        None => (serde_json::to_string(clip)?, None),
    };
    let key = fnv1a(&text);
    let found = |e: &CacheEntry| {
        let file = e.file.clone().map(|mut info| {
            // Same content under another path: report the path this clip names.
            if let Some(f) = &clip.file {
                info.path.clone_from(&f.path);
            }
            info
        });
        (key, e.pcm.clone(), file)
    };
    if let Some(e) = CLIP_CACHE.lock().map_err(poisoned)?.get(&key) {
        ensure!(
            e.text == text,
            "audio clip content hash collision ({key:016x})"
        );
        return Ok(found(e));
    }
    let (pcm, file): (Arc<[i16]>, _) = match (&clip.file, bytes) {
        (Some(f), Some(bytes)) => {
            let started = std::time::Instant::now();
            let decoded = crate::audio_file::decode(&bytes, f.gain)
                .with_context(|| format!("audio file {}", f.path))?;
            let info = FileInfo {
                path: f.path.clone(),
                format: decoded.format,
                channels: decoded.channels,
                rate: decoded.rate,
                seconds: decoded.seconds,
                bytes: bytes.len() as u64,
                decode_ms: started.elapsed().as_secs_f64() * 1000.0,
            };
            (decoded.pcm.into(), Some(info))
        }
        _ => (clip.synthesize().into(), None),
    };
    let mut cache = CLIP_CACHE.lock().map_err(poisoned)?;
    if let Some(e) = cache.get(&key) {
        return Ok(found(e));
    }
    let bytes = |c: &BTreeMap<u64, CacheEntry>| c.values().map(|e| e.pcm.len() * 2).sum::<usize>();
    if bytes(&cache) + pcm.len() * 2 > CLIP_CACHE_BYTES {
        cache.retain(|_, e| Arc::strong_count(&e.pcm) > 1);
    }
    let used = bytes(&cache);
    ensure!(
        used + pcm.len() * 2 <= CLIP_CACHE_BYTES,
        "audio clip cache budget exceeded ({} + {} > {CLIP_CACHE_BYTES} bytes of PCM in use)",
        used,
        pcm.len() * 2
    );
    cache.insert(
        key,
        CacheEntry {
            text,
            pcm: pcm.clone(),
            file: file.clone(),
        },
    );
    Ok((key, pcm, file))
}

/// PCM bytes currently held by the clip cache.
pub fn clip_cache_bytes() -> usize {
    CLIP_CACHE
        .lock()
        .map(|c| c.values().map(|e| e.pcm.len() * 2).sum())
        .unwrap_or(0)
}

/// Package resolver: every file clip's pack-relative path becomes the absolute path inside
/// the pack, and the file is decoded now (through the cache, so the runtime build that
/// follows reuses it): a missing, corrupt, over-budget or escaping file fails the import with
/// its clip's path. Only the `path` strings change; the rest keeps its authored text.
pub fn resolve_files(root: &std::path::Path, world: &mut axiom_core::World) -> Result<()> {
    let Some(clips) = world
        .resources
        .get_mut("audio.library")
        .and_then(|l| l.get_mut("clips"))
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    // The clip budget before any file is opened (the composed world is checked again).
    if clips.len() > axiom_core::audio::MAX_LIBRARY_CLIPS {
        return Err(axiom_core::Diagnostic::error(
            "audio_budget",
            "/resources/audio.library/clips",
            format!(
                "audio clip budget exceeded ({} > {} clips)",
                clips.len(),
                axiom_core::audio::MAX_LIBRARY_CLIPS
            ),
        ));
    }
    for (id, value) in clips.iter_mut() {
        let Some(raw) = value
            .pointer("/file/path")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let at = format!(
            "/resources/audio.library/clips/{}/file",
            axiom_core::local::escape(id)
        );
        let resolved = axiom_core::local::at(
            &at,
            (|| -> Result<String> {
                ensure!(
                    !raw.contains(':') && !raw.starts_with('/') && !raw.starts_with('\\'),
                    "audio file {raw} must be a path relative to the content pack"
                );
                let path = root
                    .join(raw.replace('\\', "/"))
                    .canonicalize()
                    .with_context(|| format!("audio file {raw} not found in the content pack"))?;
                ensure!(
                    path.starts_with(root),
                    "audio file {raw} escapes the content pack"
                );
                let resolved = path.to_string_lossy().into_owned();
                let mut clip: AudioClip = serde_json::from_value(value.clone())?;
                if let Some(file) = clip.file.as_mut() {
                    file.path.clone_from(&resolved);
                }
                cached_clip(&clip).with_context(|| format!("audio file {raw}"))?;
                Ok(resolved)
            })(),
        )?;
        value["file"]["path"] = Value::String(resolved);
    }
    Ok(())
}

static NEXT_TOKEN: AtomicU32 = AtomicU32::new(1);
fn next_token() -> u32 {
    NEXT_TOKEN.fetch_add(1, Ordering::Relaxed).max(1)
}

struct Clip {
    name: String,
    hash: u64,
    seconds: f32,
    file: Option<FileInfo>,
}

struct Source {
    slot: usize,
    clip: usize,
    def: AudioSource,
    playing: bool,
    /// Seconds into the clip (the simulation's play position).
    time: f32,
    token: u32,
    /// Play position when `token` was assigned (where its voice starts).
    from: f32,
    /// Started since the last step: that step plays from `time` without advancing it, so the
    /// first published offset is the clip start.
    fresh: bool,
    voice: Option<u8>,
    selected: bool,
    gain: f32,
    pan: f32,
    left: f32,
    right: f32,
    distance: f32,
}

pub struct AudioSpatialSystem {
    sources: Vec<Source>,
    ids: Vec<String>,
    clips: Vec<Clip>,
    table: Arc<ClipTable>,
    /// Authored listener slot (validated: at most one active).
    listener: Option<usize>,
    listener_def: AudioListener,
    /// Entities with `render.camera` (fallback listener: first enabled active one) and IDs
    /// of every possible listener entity (observe never needs the runtime state).
    cameras: Vec<usize>,
    names: BTreeMap<usize, String>,
    /// Who listened in the last step: (slot, from a camera), and where.
    heard: Option<(usize, bool, Vec3)>,
    /// Audible sources of this step, sorted for voice assignment (capacity = sources).
    order: Vec<u32>,
    voices: [Option<u32>; MAX_VOICES],
    params: [VoiceParams; MAX_VOICES],
    audible: usize,
    culled: usize,
    steals: u64,
    finished: u64,
    starts: u64,
    scanned: usize,
    halted: Option<String>,
    /// Short one-shots that ended while voiced, by serial (see [`SHOT_SECONDS`]); capacity
    /// [`SHOTS`], reserved once.
    shots: std::collections::VecDeque<(u64, VoiceParams)>,
    shot_serial: u64,
    shots_dropped: u64,
    /// Identity of the shot serials (new system = new stream, kept by hot reload).
    stream: u64,
}

fn round(v: f32) -> f64 {
    (f64::from(v) * 10000.0).round() / 10000.0
}

impl AudioSpatialSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let compiled = &state.compiled;
        let library: AudioLibrary = compiled
            .source
            .resources
            .get("audio.library")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .context("audio.library")?
            .unwrap_or_default();
        let mut system = Self {
            sources: vec![],
            ids: vec![],
            clips: vec![],
            table: Arc::new(ClipTable::new(vec![])),
            listener: None,
            listener_def: AudioListener::default(),
            cameras: vec![],
            names: BTreeMap::new(),
            heard: None,
            order: vec![],
            voices: [None; MAX_VOICES],
            params: [VoiceParams::default(); MAX_VOICES],
            audible: 0,
            culled: 0,
            steals: 0,
            finished: 0,
            starts: 0,
            scanned: compiled.len(),
            halted: None,
            shots: std::collections::VecDeque::with_capacity(SHOTS),
            shot_serial: 0,
            shots_dropped: 0,
            stream: crate::audio::next_stream(),
        };
        let mut pcm = vec![];
        for (slot, e) in compiled.entities.iter().enumerate() {
            if e.camera.is_some() {
                system.cameras.push(slot);
                system.names.insert(slot, e.id.clone());
            }
            if let Some(l) = e.audio_listener.as_ref().filter(|l| l.active) {
                ensure!(
                    system.listener.is_none(),
                    "more than one active audio.listener"
                );
                system.listener = Some(slot);
                system.listener_def = l.clone();
                system.names.insert(slot, e.id.clone());
            }
            let Some(def) = &e.audio_source else { continue };
            def.validate()
                .with_context(|| format!("audio.source {}", e.id))?;
            let clip = match system.clips.iter().position(|c| c.name == def.clip) {
                Some(c) => c,
                None => {
                    let synth = library
                        .clips
                        .get(&def.clip)
                        .with_context(|| format!("audio.library has no clip {}", def.clip))?;
                    let (hash, samples, file) =
                        cached_clip(synth).with_context(|| format!("clip {}", def.clip))?;
                    system.clips.push(Clip {
                        name: def.clip.clone(),
                        hash,
                        seconds: samples.len() as f32 / SAMPLE_RATE as f32,
                        file,
                    });
                    pcm.push((hash, samples));
                    system.clips.len() - 1
                }
            };
            let playing = def.autoplay && def.enabled;
            system.sources.push(Source {
                slot,
                clip,
                def: def.clone(),
                playing,
                time: 0.0,
                token: if playing { next_token() } else { 0 },
                from: 0.0,
                fresh: playing,
                voice: None,
                selected: false,
                gain: 0.0,
                pan: 0.0,
                left: 0.0,
                right: 0.0,
                distance: 0.0,
            });
            system.ids.push(e.id.clone());
        }
        ensure!(
            system.sources.len() <= axiom_core::audio::MAX_AUDIO_SOURCES,
            "audio source budget exceeded"
        );
        // Procedural clips are budgeted when authored; decoded files only now.
        let samples: usize = pcm.iter().map(|(_, p)| p.len()).sum();
        ensure!(
            samples <= MAX_LIBRARY_SAMPLES,
            "audio clip memory budget exceeded ({samples} > {MAX_LIBRARY_SAMPLES} samples of 16-bit PCM at {SAMPLE_RATE} Hz in the clips sources use)"
        );
        system.table = Arc::new(ClipTable::new(pcm));
        system.order.reserve(system.sources.len());
        Ok(system)
    }

    /// Voice block of the last step (the host publishes it to the audio thread).
    pub fn voices(&self) -> &[VoiceParams; MAX_VOICES] {
        &self.params
    }

    /// Short one-shots that ended while voiced, with serial > `after` (oldest first): the
    /// output starts each once even when no voice block it published held it.
    pub fn shots(&self, after: u64) -> impl Iterator<Item = (u64, &VoiceParams)> + '_ {
        self.shots
            .iter()
            .filter(move |(serial, _)| *serial > after)
            .map(|(serial, p)| (*serial, p))
    }

    /// What this runtime decoded for file clip `clip` (`None`: procedural, or no source uses
    /// it). Read from this runtime, not the process-wide cache, so it matches `observe`.
    pub fn clip_file(&self, clip: &str) -> Option<&FileInfo> {
        self.clips.iter().find(|c| c.name == clip)?.file.as_ref()
    }

    /// Identity of the [`Self::shots`] serials (a new system restarts them).
    pub fn stream(&self) -> u64 {
        self.stream
    }

    /// PCM of every clip the sources use (shared with the clip cache).
    pub fn clip_table(&self) -> &Arc<ClipTable> {
        &self.table
    }

    fn find(&self, args: &Value) -> Result<usize> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .context("audio actions need {\"id\":\"entity with audio.source\"}")?;
        self.ids.iter().position(|x| x == id).ok_or_else(|| {
            axiom_core::Diagnostic::error(
                "not_found",
                "/args/id",
                format!("{id} has no audio.source"),
            )
        })
    }

    fn source_json(&self, i: usize) -> Value {
        let s = &self.sources[i];
        json!({
            "id": self.ids[i],
            "clip": self.clips[s.clip].name,
            "playing": s.playing,
            "time": round(s.time),
            "length": round(self.clips[s.clip].seconds),
            "loop": s.def.looping,
            "enabled": s.def.enabled,
            "spatial": s.def.spatial,
            "gain": round(s.gain),
            "left": round(s.left),
            "right": round(s.right),
            "pan": round(s.pan),
            "distance": round(s.distance),
            "voice": s.voice,
            "priority": s.def.priority,
        })
    }

    fn listener_json(&self) -> Value {
        match self.heard {
            Some((slot, camera, at)) => json!({
                "id": self.names.get(&slot),
                "from": if camera { "camera" } else { "audio.listener" },
                "position": [round(at.x), round(at.y), round(at.z)],
            }),
            None => Value::Null,
        }
    }

    fn summary(&self) -> Value {
        let mut voices: Vec<(u8, usize)> = self
            .sources
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((s.voice?, i)))
            .collect();
        voices.sort_unstable();
        let voiced = voices.len();
        let mut out = json!({
            "sources": self.sources.len(),
            "playing": self.sources.iter().filter(|s| s.playing).count(),
            "audible": self.audible,
            "voiced": voiced,
            "virtual": self.audible.saturating_sub(voiced),
            "culled": self.culled,
            "max_voices": MAX_VOICES,
            "steals": self.steals,
            "finished": self.finished,
            "starts": self.starts,
            "shots": self.shot_serial,
            "shots_dropped": self.shots_dropped,
            "listener": self.listener_json(),
            "voices": voices.iter().map(|&(v, i)| {
                let s = &self.sources[i];
                json!({"voice": v, "id": self.ids[i], "clip": self.clips[s.clip].name,
                    "gain": round(s.gain), "left": round(s.left), "right": round(s.right),
                    "pan": round(s.pan), "distance": round(s.distance)})
            }).collect::<Vec<_>>(),
            "clips": {"count": self.table.len(), "bytes": self.table.bytes(), "cache_bytes": clip_cache_bytes(),
                "files": self.clips.iter().filter_map(|c| {
                    let f = c.file.as_ref()?;
                    Some(json!({"clip": c.name, "path": f.path, "format": f.format, "channels": f.channels,
                        "rate": f.rate, "seconds": round(f.seconds as f32), "bytes": f.bytes,
                        "decode_ms": (f.decode_ms * 1000.0).round() / 1000.0}))
                }).collect::<Vec<_>>()},
            "halted": self.halted,
        });
        if self.heard.is_none() && self.sources.iter().any(|s| s.def.spatial) {
            out["listener_note"] =
                json!("no active audio.listener or camera: spatial sources are silent");
        }
        out
    }

    fn start(&mut self, i: usize) {
        let s = &mut self.sources[i];
        s.playing = true;
        s.time = 0.0;
        s.token = next_token();
        s.from = 0.0;
        s.fresh = true;
        self.starts += 1;
    }
}

impl RuntimeSystem for AudioSpatialSystem {
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
                .find(|e| e.audio_source.is_some() || e.audio_listener.is_some())
            {
                self.halted = Some(format!(
                    "runtime-spawned entity {} has an audio component; spawned audio is not supported (author it instead)",
                    e.id
                ));
                self.params = [VoiceParams::default(); MAX_VOICES];
                self.voices = [None; MAX_VOICES];
                for s in &mut self.sources {
                    s.voice = None;
                }
                return;
            }
            self.scanned = len;
        }
        let compiled = &state.compiled;
        let authored = self
            .listener
            .filter(|&s| compiled.enabled[s])
            .map(|s| (s, false));
        let heard = authored.or_else(|| {
            self.cameras
                .iter()
                .find(|&&s| {
                    compiled.enabled[s]
                        && compiled.entities[s]
                            .camera
                            .as_ref()
                            .is_some_and(|c| c.active)
                        && state
                            .contexts
                            .as_ref()
                            .is_none_or(|c| c.camera(&compiled.entities[s].id))
                })
                .map(|&s| (s, true))
        });
        let pose = heard.map(|(slot, camera)| {
            let m = crate::trigger::world_matrix(state, slot);
            let (_, rotation, position) = m.to_scale_rotation_translation();
            let plus_z = !camera && self.listener_def.forward == ListenerForward::PlusZ;
            let (right, forward) = if plus_z {
                (rotation * Vec3::NEG_X, rotation * Vec3::Z)
            } else {
                (rotation * Vec3::X, rotation * Vec3::NEG_Z)
            };
            (slot, camera, position, right, forward)
        });
        self.heard = pose.map(|(slot, camera, at, _, _)| (slot, camera, at));
        let master = match pose {
            Some((_, false, ..)) => self.listener_def.volume,
            _ => 1.0,
        };
        self.order.clear();
        self.culled = 0;
        for (i, s) in self.sources.iter_mut().enumerate() {
            s.selected = false;
            let here = s.def.enabled && compiled.enabled[s.slot];
            if s.playing && here && std::mem::take(&mut s.fresh) {
                // Started since the last step: publish the clip start first.
            } else if s.playing && here {
                let length = self.clips[s.clip].seconds;
                s.time += FIXED_DT * s.def.pitch;
                if s.time >= length {
                    if s.def.looping {
                        s.time %= length;
                    } else {
                        // A short one-shot that held a voice: hand its whole start to the
                        // output (this frame's voice block may never have held it).
                        if s.voice.is_some() && length <= SHOT_SECONDS * s.def.pitch {
                            if self.shots.len() == SHOTS {
                                self.shots.pop_front();
                                self.shots_dropped += 1;
                            }
                            self.shot_serial += 1;
                            self.shots.push_back((
                                self.shot_serial,
                                VoiceParams {
                                    clip: self.clips[s.clip].hash,
                                    token: s.token,
                                    offset: s.from,
                                    pitch: s.def.pitch,
                                    left: s.left,
                                    right: s.right,
                                    looping: false,
                                    music: s.def.bus == axiom_core::audio::AudioBus::Music,
                                },
                            ));
                        }
                        s.playing = false;
                        s.time = length;
                        self.finished += 1;
                    }
                }
            }
            (s.gain, s.pan, s.distance) = (0.0, 0.0, 0.0);
            if s.playing && here {
                if !s.def.spatial {
                    s.gain = s.def.volume * master;
                } else if let Some((_, _, at, right, forward)) = pose {
                    let rel = crate::trigger::pivot(state, s.slot) - at;
                    s.distance = rel.length();
                    s.gain = s.def.volume
                        * master
                        * attenuation(
                            s.def.rolloff,
                            s.def.min_distance,
                            s.def.max_distance,
                            s.distance,
                        );
                    s.pan = pan(rel.dot(right), rel.dot(forward), s.def.min_distance);
                }
                if s.gain > MIN_AUDIBLE {
                    self.order.push(i as u32);
                } else {
                    self.culled += 1;
                }
            }
            let (l, r) = pan_gains(s.pan);
            (s.left, s.right) = (s.gain * l, s.gain * r);
        }
        self.audible = self.order.len();
        let sources = &self.sources;
        self.order.sort_unstable_by(|&a, &b| {
            let (x, y) = (&sources[a as usize], &sources[b as usize]);
            y.def
                .priority
                .cmp(&x.def.priority)
                .then(x.distance.total_cmp(&y.distance))
                .then(a.cmp(&b))
        });
        for &i in self.order.iter().take(MAX_VOICES) {
            self.sources[i as usize].selected = true;
        }
        // Voices whose source lost its place are freed first (a steal when it is still audible),
        // then newly selected sources take the free voices in rank order.
        for (v, slot) in self.voices.iter_mut().enumerate() {
            if let Some(i) = *slot {
                let s = &mut self.sources[i as usize];
                if !s.selected {
                    if s.playing && s.gain > MIN_AUDIBLE {
                        self.steals += 1;
                    }
                    s.voice = None;
                    *slot = None;
                    self.params[v] = VoiceParams::default();
                }
            }
        }
        let mut free = 0;
        for &i in self.order.iter().take(MAX_VOICES) {
            let s = &mut self.sources[i as usize];
            if s.voice.is_none() {
                while self.voices[free].is_some() {
                    free += 1;
                }
                self.voices[free] = Some(i);
                s.voice = Some(free as u8);
            }
        }
        for (v, slot) in self.voices.iter().enumerate() {
            let Some(i) = *slot else { continue };
            let s = &self.sources[i as usize];
            self.params[v] = VoiceParams {
                clip: self.clips[s.clip].hash,
                token: s.token,
                offset: s.time,
                pitch: s.def.pitch,
                left: s.left,
                right: s.right,
                looping: s.def.looping,
                music: s.def.bus == axiom_core::audio::AudioBus::Music,
            };
        }
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        _next: &mut RuntimeState,
    ) -> Result<()> {
        // Hot reload keeps a source's playback (playing, time, voice token: the audio thread
        // continues without a restart) when its entity ID, clip content, loop and enabled flags
        // are unchanged; otherwise it starts over as authored (autoplay plays from 0).
        let Some(old) = old.as_any().downcast_ref::<Self>() else {
            return Ok(());
        };
        let previous: BTreeMap<&str, usize> = old
            .ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        for (i, s) in self.sources.iter_mut().enumerate() {
            let Some(&o) = previous.get(self.ids[i].as_str()) else {
                continue;
            };
            let was = &old.sources[o];
            if old.clips[was.clip].hash != self.clips[s.clip].hash
                || was.def.looping != s.def.looping
                || was.def.enabled != s.def.enabled
            {
                continue;
            }
            s.playing = was.playing;
            s.time = was.time;
            s.token = was.token;
            s.from = was.from;
            s.fresh = was.fresh;
            // Keep the voice slot too, so the audio thread continues the same voice.
            if let Some(v) = was.voice.filter(|&v| self.voices[usize::from(v)].is_none()) {
                s.voice = Some(v);
                self.voices[usize::from(v)] = Some(i as u32);
            }
        }
        self.steals = old.steals;
        self.finished = old.finished;
        self.starts = old.starts;
        // The output's shot cursor follows this stream across hot reloads.
        self.shots.clone_from(&old.shots);
        self.shot_serial = old.shot_serial;
        self.shots_dropped = old.shots_dropped;
        self.stream = old.stream;
        Ok(())
    }
    fn command(
        &mut self,
        _state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "audio" {
            return None;
        }
        Some((|| {
            if action == "summary" {
                return Ok(self.summary());
            }
            let i = self.find(args)?;
            match action {
                "inspect" => return Ok(self.source_json(i)),
                "play" => {
                    if let Some(reason) = &self.halted {
                        bail!("audio.spatial is halted: {reason}");
                    }
                    ensure!(
                        self.sources[i].def.enabled,
                        "{} has a disabled audio.source (enabled false)",
                        self.ids[i]
                    );
                    let restart = args.get("restart").and_then(Value::as_bool) == Some(true);
                    if restart || !self.sources[i].playing {
                        self.start(i);
                    }
                }
                "stop" => {
                    let s = &mut self.sources[i];
                    s.playing = false;
                    s.time = 0.0;
                }
                other => bail!("unknown audio action {other}"),
            }
            Ok(self.source_json(i))
        })())
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("audio".into(), self.summary())])
    }
}

fn id_arg(v: &Value) -> Result<()> {
    ensure!(
        v.get("id").and_then(Value::as_str).is_some(),
        "needs {{\"id\":\"entity with audio.source\"}}"
    );
    Ok(())
}

pub fn register(r: &mut RuntimeRegistry) {
    let entity = "entity ID with audio.source";
    r.authoring.action(
        "audio.play",
        "audio",
        json!({"id":entity,"restart":"optional bool: start from 0 even if playing","effect":"starts the source (one-shot: plays to the clip end, then stops); already playing: unchanged unless restart"}),
        |v| {
            id_arg(v)?;
            ensure!(
                v.get("restart").is_none_or(Value::is_boolean),
                "restart must be a bool"
            );
            Ok(())
        },
    );
    r.authoring.action(
        "audio.stop",
        "audio",
        json!({"id":entity,"effect":"stops and rewinds to 0; its voice is released next step"}),
        id_arg,
    );
    r.authoring.action(
        "audio.inspect",
        "audio",
        json!({"id":entity,"result":"clip, playing, time, length, loop, gain, left, right, pan, distance, voice (null = no voice)"}),
        id_arg,
    );
    r.authoring.action(
        "audio.summary",
        "audio",
        json!({"result":"the observe `audio` section: listener, counts, voices"}),
        |_| Ok(()),
    );
    r.authoring.package_resolvers.push(resolve_files);
    r.factories.insert("audio.spatial".into(), |state| {
        Ok(Box::new(AudioSpatialSystem::new(state)?))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cost of the file check per cached build of one 5.8 MB file (`cargo test -p
    /// axiom-runtime --lib measure_file_check -- --ignored --nocapture`): metadata only (round 1,
    /// no content check), read + SipHash fingerprint (now), read + SHA-256 (before round 1).
    /// Medians of 9 runs; the file is in the OS cache after the first read.
    #[test]
    #[ignore]
    fn measure_file_check() {
        let dir = std::env::temp_dir().join(format!("axiom-audio-measure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.wav");
        let samples: Vec<f32> = (0..2_880_000)
            .map(|i| (i as f32 * 0.01).sin() * 0.5)
            .collect();
        std::fs::write(
            &path,
            crate::audio_file::encode_wav(&samples, 2, 44_100, 16, false),
        )
        .unwrap();
        let file = ClipFile {
            path: path.to_string_lossy().into_owned(),
            gain: 1.0,
        };
        let median = |f: &dyn Fn()| {
            let mut t: Vec<f64> = (0..9)
                .map(|_| {
                    let s = std::time::Instant::now();
                    f();
                    s.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            t.sort_by(f64::total_cmp);
            t[4]
        };
        let stamp = median(&|| {
            file_stamp(&file).unwrap();
        });
        let read = median(&|| {
            file_bytes(&file).unwrap();
        });
        let print = median(&|| {
            fingerprint(&file_bytes(&file).unwrap());
        });
        let sha = median(&|| {
            file_text(&file, &file_bytes(&file).unwrap());
        });
        eprintln!(
            "file check of {} bytes ({}): stamp {stamp:.3} ms, read {read:.2} ms, read+fingerprint {print:.2} ms, read+sha256 {sha:.2} ms",
            std::fs::metadata(&path).unwrap().len(),
            if cfg!(debug_assertions) { "debug" } else { "release" }
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
