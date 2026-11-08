//! Bounded audio events and prepared PCM cues. Device I/O only runs in the native host.
use crate::world::{ActionMap, RuntimeRegistry, RuntimeState, RuntimeSystem};
use anyhow::{ensure, Result};
use axiom_core::{valid_id, SystemDescriptor};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tone {
    frequency: f32,
    duration_ms: u32,
    gain: f32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    cues: BTreeMap<String, Tone>,
}
#[derive(Clone, Serialize)]
pub(crate) struct Event {
    pub seq: u64,
    tick: u64,
    cue: String,
}
#[derive(Clone, Default)]
pub(crate) struct Events {
    pub waves: BTreeMap<String, Arc<Vec<u8>>>,
    pub recent: VecDeque<Event>,
    sequence: u64,
}
impl Events {
    pub fn emit(&mut self, tick: u64, cue: &str) {
        if !self.waves.contains_key(cue) {
            return;
        }
        self.sequence += 1;
        self.recent.push_back(Event {
            seq: self.sequence,
            tick,
            cue: cue.into(),
        });
        if self.recent.len() > 64 {
            self.recent.pop_front();
        }
    }
    pub fn observation(&self) -> Value {
        json!({"emitted":self.sequence,"recent":self.recent.iter().rev().take(8).collect::<Vec<_>>()})
    }
}
pub(crate) fn register(registry: &mut RuntimeRegistry) {
    let r = &mut registry.authoring;
    r.capabilities.insert("audio".into(), 1);
    r.resource("audio.library","audio",json!({"cues":"stable ID -> {frequency:40..4000 Hz,duration_ms:10..150,gain:0..0.5}","output":"mono 22050 Hz PCM, native Windows host","events":"simulation tick + monotonic sequence"}),|v|{
        let lib:Library=serde_json::from_value(v.clone())?;
        ensure!(lib.cues.len()<=64,"audio cue budget exceeded");
        for (id,tone) in lib.cues {
            valid_id(&id)?;
            ensure!(tone.frequency.is_finite() && (40.0..=4000.0).contains(&tone.frequency)
                && (10..=150).contains(&tone.duration_ms) && tone.gain.is_finite() && (0.0..=0.5).contains(&tone.gain),"invalid tone");
        }
        Ok(())
    });
    r.systems.insert(
        "audio.events".into(),
        SystemDescriptor {
            id: "audio.events".into(),
            capability: "audio".into(),
            phase: 40,
            after: vec![],
            reads: vec!["audio.library".into()],
            writes: vec!["audio.events".into()],
        },
    );
    registry.factories.insert("audio.events".into(), |state| {
        let lib: Library = state.compiled.source.resource("audio.library")?;
        state.audio.waves = lib
            .cues
            .iter()
            .map(|(id, t)| (id.clone(), Arc::new(wave(t))))
            .collect();
        Ok(Box::new(AudioSystem))
    });
}
struct AudioSystem;
impl RuntimeSystem for AudioSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, _: &mut RuntimeState, _: &ActionMap) {}
    fn transfer(
        &mut self,
        _: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        if previous.compiled.source.id == next.compiled.source.id {
            next.audio.recent = previous.audio.recent.clone();
            next.audio.sequence = previous.audio.sequence;
        }
        Ok(())
    }
}
fn wave(tone: &Tone) -> Vec<u8> {
    let samples = tone.duration_ms * 22050 / 1000;
    let data_len = samples * 2;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend(b"RIFF");
    bytes.extend((36 + data_len).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(22050_u32.to_le_bytes());
    bytes.extend(44100_u32.to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_len.to_le_bytes());
    for i in 0..samples {
        let t = i as f32 / 22050.0;
        let envelope = (i as f32 / 60.0).min(1.0) * (1.0 - i as f32 / samples as f32).powi(2);
        let sample = (std::f32::consts::TAU * tone.frequency * t * (1.0 - t * 1.8)).sin()
            * tone.gain
            * envelope;
        bytes.extend(((sample * 32767.0) as i16).to_le_bytes());
    }
    bytes
}

pub(crate) struct Output {
    sender: Option<mpsc::SyncSender<Arc<Vec<u8>>>>,
    worker: Option<std::thread::JoinHandle<()>>,
    session: String,
    last: u64,
    played: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    dropped: u64,
}
impl Output {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Arc<Vec<u8>>>(4);
        let played = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicU64::new(0));
        let ok = played.clone();
        let errors = failed.clone();
        let worker = std::thread::spawn(move || {
            while let Ok(wave) = receiver.recv() {
                let counter = if play(&wave) { &ok } else { &errors };
                counter.fetch_add(1, Ordering::Relaxed);
            }
        });
        Self {
            sender: Some(sender),
            worker: Some(worker),
            session: String::new(),
            last: 0,
            played,
            failed,
            dropped: 0,
        }
    }
    pub fn update(&mut self, events: &Events, session: &str, enabled: bool) {
        if self.session != session {
            self.session = session.into();
            self.last = 0;
        }
        for event in &events.recent {
            if event.seq <= self.last {
                continue;
            }
            self.last = event.seq;
            if enabled {
                if let Some(wave) = events.waves.get(&event.cue) {
                    if self
                        .sender
                        .as_ref()
                        .unwrap()
                        .try_send(wave.clone())
                        .is_err()
                    {
                        self.dropped += 1;
                    }
                }
            }
        }
    }
    pub fn observation(&self) -> Value {
        json!({"backend":if cfg!(windows){"windows_pcm"}else{"unavailable"},
            "played":self.played.load(Ordering::Relaxed),"failed":self.failed.load(Ordering::Relaxed),"dropped":self.dropped})
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(windows)]
fn play(wave: &[u8]) -> bool {
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(sound: *const u16, module: *mut std::ffi::c_void, flags: u32) -> i32;
    }
    // SND_MEMORY | SND_NODEFAULT with synchronous playback keeps this owned buffer
    // alive until WinMM returns. The bounded worker keeps device waits off simulation.
    unsafe { PlaySoundW(wave.as_ptr().cast(), std::ptr::null_mut(), 0x0004 | 0x0002) != 0 }
}
#[cfg(not(windows))]
fn play(_: &[u8]) -> bool {
    false
}
