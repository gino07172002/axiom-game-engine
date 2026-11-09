//! Bounded audio events and prepared PCM cues, and the native output: one worker thread mixes
//! cues and the `audio.spatial` voices (`audio_mix::Mixer`) into a WinMM wave-out stream. Device
//! I/O only runs in the native host; headless runs never open a device.
use crate::audio_mix::{ClipTable, Mixer, SharedVoices, VoiceParams, MAX_VOICES};
use crate::world::{ActionMap, RuntimeRegistry, RuntimeState, RuntimeSystem};
use anyhow::Result;
use axiom_core::{
    audio::{AudioLibrary, Tone, SAMPLE_RATE},
    SystemDescriptor,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc, Arc,
    },
};

#[derive(Clone, Serialize)]
pub(crate) struct Event {
    pub seq: u64,
    tick: u64,
    cue: String,
}
#[derive(Clone, Default)]
pub(crate) struct Events {
    /// Mono PCM per cue at `SAMPLE_RATE`.
    pub waves: BTreeMap<String, Arc<[i16]>>,
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
    // The `audio` capability, `audio.library`, the source/listener components and the
    // `audio.spatial` descriptor are core types (axiom_core::audio); this adds the cue system.
    registry.authoring.systems.insert(
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
        let lib: AudioLibrary = state.compiled.source.resource("audio.library")?;
        state.audio.waves = lib
            .cues
            .iter()
            .map(|(id, t)| (id.clone(), wave(t).into()))
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
fn wave(tone: &Tone) -> Vec<i16> {
    let samples = tone.duration_ms * SAMPLE_RATE / 1000;
    (0..samples)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let envelope = (i as f32 / 60.0).min(1.0) * (1.0 - i as f32 / samples as f32).powi(2);
            let sample = (std::f32::consts::TAU * tone.frequency * t * (1.0 - t * 1.8)).sin()
                * tone.gain
                * envelope;
            (sample * 32767.0) as i16
        })
        .collect()
}

/// Main thread -> audio thread (and back, for buffers whose last reference must not be
/// dropped on the audio thread).
enum Message {
    Clips(Arc<ClipTable>),
    Cue(Arc<[i16]>),
}

/// Device buffers in flight and their size: 4 x 512 frames = 93 ms at 22,050 Hz.
const BUFFERS: usize = 4;
const BUFFER_FRAMES: usize = 512;

const DEVICE_STARTING: u32 = 0;
const DEVICE_OPEN: u32 = 1;
const DEVICE_UNAVAILABLE: u32 = 2;
const DEVICE_DISABLED: u32 = 3;

/// Counters shared with the audio thread (atomics only).
#[derive(Default)]
struct Shared {
    voices: SharedVoices,
    stop: AtomicBool,
    device: AtomicU32,
    /// Device error code when unavailable.
    error: AtomicU32,
    buffers: AtomicU64,
    /// Wake-ups that found every buffer already played (the stream ran dry).
    underruns: AtomicU64,
    mix_us_max: AtomicU64,
    clipped: AtomicU64,
    voices_mixed: AtomicU32,
    cues: AtomicU64,
    late_frees: AtomicU64,
    write_errors: AtomicU64,
}

pub(crate) struct Output {
    sender: Option<mpsc::SyncSender<Message>>,
    garbage: Option<mpsc::Receiver<Message>>,
    worker: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Shared>,
    session: String,
    last: u64,
    table: Option<Arc<ClipTable>>,
    silent: [VoiceParams; MAX_VOICES],
    dropped: u64,
}
impl Output {
    pub fn new() -> Self {
        Self::with_device(cfg!(windows))
    }
    /// `device: false` never opens an audio device (headless hosts, tests); every update is
    /// still accepted and counted.
    pub fn with_device(device: bool) -> Self {
        let shared = Arc::new(Shared::default());
        let (sender, receiver) = mpsc::sync_channel::<Message>(16);
        let (bin, garbage) = mpsc::sync_channel::<Message>(64);
        let worker = if device {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("axiom-audio".into())
                .spawn(move || device_loop(&shared, &receiver, &bin))
                .ok()
        } else {
            shared.device.store(DEVICE_DISABLED, Ordering::Relaxed);
            None
        };
        Self {
            sender: Some(sender),
            garbage: Some(garbage),
            worker,
            shared,
            session: String::new(),
            last: 0,
            table: None,
            silent: [VoiceParams::default(); MAX_VOICES],
            dropped: 0,
        }
    }
    /// Queues a message without blocking; false when the audio thread is not running or its
    /// queue is full.
    fn send(&self, message: Message) -> bool {
        self.shared.device.load(Ordering::Relaxed) <= DEVICE_OPEN
            && self
                .sender
                .as_ref()
                .is_some_and(|s| s.try_send(message).is_ok())
    }
    /// Once per frame on the main thread: new cues, the clip table when it changed and the
    /// latest voice block (`spatial`; silence while `enabled` is false). Never blocks.
    pub fn update(
        &mut self,
        events: &Events,
        session: &str,
        enabled: bool,
        spatial: Option<&crate::spatial_audio::AudioSpatialSystem>,
    ) {
        if let Some(bin) = &self.garbage {
            while bin.try_recv().is_ok() {}
        }
        if self.session != session {
            self.session = session.into();
            self.last = 0;
        }
        let fresh: Vec<Arc<[i16]>> = events
            .recent
            .iter()
            .filter(|e| e.seq > self.last)
            .filter_map(|e| events.waves.get(&e.cue).cloned())
            .collect();
        if let Some(last) = events.recent.back() {
            self.last = self.last.max(last.seq);
        }
        if enabled {
            for wave in fresh {
                if !self.send(Message::Cue(wave)) {
                    self.dropped += 1;
                }
            }
        }
        // Remembered only once queued, so a full queue retries on the next frame. Without
        // audio.spatial an empty table replaces the old one, which releases its PCM for the
        // clip cache.
        let table = match spatial {
            Some(s) => Some(s.clip_table().clone()),
            None if self.table.as_ref().is_some_and(|t| !t.is_empty()) => {
                Some(Arc::new(ClipTable::new(vec![])))
            }
            None => None,
        };
        if let Some(table) = table {
            if !self.table.as_ref().is_some_and(|t| Arc::ptr_eq(t, &table))
                && self.send(Message::Clips(table.clone()))
            {
                self.table = Some(table);
            }
        }
        let voices = match spatial {
            Some(s) if enabled => s.voices(),
            _ => &self.silent,
        };
        self.shared.voices.publish(voices);
    }
    pub fn observation(&self) -> Value {
        let s = &self.shared;
        let device = match s.device.load(Ordering::Relaxed) {
            DEVICE_STARTING => "starting".to_owned(),
            DEVICE_OPEN => "open".to_owned(),
            DEVICE_DISABLED => "disabled".to_owned(),
            _ => format!("unavailable (error {})", s.error.load(Ordering::Relaxed)),
        };
        json!({"backend":if cfg!(windows){"winmm_waveout"}else{"unavailable"},
            "device":device,"rate":SAMPLE_RATE,"buffer_frames":BUFFER_FRAMES,"buffers":BUFFERS,
            "rendered":s.buffers.load(Ordering::Relaxed),"underruns":s.underruns.load(Ordering::Relaxed),
            "mix_us_max":s.mix_us_max.load(Ordering::Relaxed),"voices_mixed":s.voices_mixed.load(Ordering::Relaxed),
            "clipped_samples":s.clipped.load(Ordering::Relaxed),"played":s.cues.load(Ordering::Relaxed),
            "late_frees":s.late_frees.load(Ordering::Relaxed),"write_errors":s.write_errors.load(Ordering::Relaxed),"dropped":self.dropped})
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            // The loop wakes at least every 50 ms; a stuck driver must not hang shutdown.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

/// Applies pending messages to the mixer and returns displaced buffers to the main thread
/// (dropped here only when that queue is full, counted as late frees).
fn accept(
    shared: &Shared,
    mixer: &mut Mixer,
    inbox: &mpsc::Receiver<Message>,
    bin: &mpsc::SyncSender<Message>,
) {
    let give_back = |m: Message| {
        if bin.try_send(m).is_err() {
            shared.late_frees.fetch_add(1, Ordering::Relaxed);
        }
    };
    while let Ok(message) = inbox.try_recv() {
        match message {
            Message::Clips(table) => {
                if let Some(old) = mixer.set_clips(table) {
                    give_back(Message::Clips(old));
                }
            }
            Message::Cue(pcm) => {
                if let Some(old) = mixer.play_cue(pcm) {
                    give_back(Message::Cue(old));
                }
            }
        }
    }
    while let Some(done) = mixer.take_finished_cue() {
        give_back(Message::Cue(done));
    }
}

/// Mixes one buffer (no allocation, no lock) and records its cost.
fn mix(shared: &Shared, mixer: &mut Mixer, out: &mut [i16]) {
    let started = std::time::Instant::now();
    let mut voices = [VoiceParams::default(); MAX_VOICES];
    if shared.voices.read(&mut voices) {
        mixer.set_params(&voices);
    }
    mixer.render(out);
    shared
        .mix_us_max
        .fetch_max(started.elapsed().as_micros() as u64, Ordering::Relaxed);
    shared.buffers.fetch_add(1, Ordering::Relaxed);
    shared
        .voices_mixed
        .store(mixer.stats.voices, Ordering::Relaxed);
    shared
        .clipped
        .store(mixer.stats.clipped_samples, Ordering::Relaxed);
    shared
        .cues
        .store(mixer.stats.cues_started, Ordering::Relaxed);
}

#[cfg(windows)]
#[allow(non_snake_case, clippy::upper_case_acronyms)]
mod winmm {
    use std::ffi::c_void;
    #[repr(C, packed(1))]
    pub struct WAVEFORMATEX {
        pub wFormatTag: u16,
        pub nChannels: u16,
        pub nSamplesPerSec: u32,
        pub nAvgBytesPerSec: u32,
        pub nBlockAlign: u16,
        pub wBitsPerSample: u16,
        pub cbSize: u16,
    }
    #[repr(C)]
    pub struct WAVEHDR {
        pub lpData: *mut u8,
        pub dwBufferLength: u32,
        pub dwBytesRecorded: u32,
        pub dwUser: usize,
        pub dwFlags: u32,
        pub dwLoops: u32,
        pub lpNext: *mut WAVEHDR,
        pub reserved: usize,
    }
    pub const WAVE_MAPPER: u32 = u32::MAX;
    pub const CALLBACK_EVENT: u32 = 0x0005_0000;
    pub const WHDR_DONE: u32 = 1;
    #[link(name = "winmm")]
    unsafe extern "system" {
        pub fn waveOutOpen(
            out: *mut *mut c_void,
            device: u32,
            format: *const WAVEFORMATEX,
            callback: usize,
            instance: usize,
            flags: u32,
        ) -> u32;
        pub fn waveOutPrepareHeader(out: *mut c_void, header: *mut WAVEHDR, size: u32) -> u32;
        pub fn waveOutUnprepareHeader(out: *mut c_void, header: *mut WAVEHDR, size: u32) -> u32;
        pub fn waveOutWrite(out: *mut c_void, header: *mut WAVEHDR, size: u32) -> u32;
        pub fn waveOutReset(out: *mut c_void) -> u32;
        pub fn waveOutClose(out: *mut c_void) -> u32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn CreateEventW(
            attributes: *mut c_void,
            manual_reset: i32,
            initial: i32,
            name: *const u16,
        ) -> *mut c_void;
        pub fn WaitForSingleObject(handle: *mut c_void, millis: u32) -> u32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
    }
}

/// Opens the default wave-out device and keeps `BUFFERS` mixed buffers queued until `stop`.
/// The only wait is on the device's buffer-done event (at most 50 ms, so `stop` is noticed).
#[cfg(windows)]
fn device_loop(shared: &Shared, inbox: &mpsc::Receiver<Message>, bin: &mpsc::SyncSender<Message>) {
    use winmm::*;
    let size = std::mem::size_of::<WAVEHDR>() as u32;
    let unavailable = |code: u32| {
        shared.error.store(code, Ordering::Relaxed);
        shared.device.store(DEVICE_UNAVAILABLE, Ordering::Relaxed);
    };
    // SAFETY: plain Win32 calls; every handle and header is released below on all paths, and
    // the buffers outlive the device (reset + unprepare before they are dropped).
    unsafe {
        let event = CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null());
        if event.is_null() {
            unavailable(u32::MAX);
            return;
        }
        let format = WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: 2,
            nSamplesPerSec: SAMPLE_RATE,
            nAvgBytesPerSec: SAMPLE_RATE * 4,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut device = std::ptr::null_mut();
        let code = waveOutOpen(
            &mut device,
            WAVE_MAPPER,
            &format,
            event as usize,
            0,
            CALLBACK_EVENT,
        );
        if code != 0 {
            CloseHandle(event);
            unavailable(code);
            return;
        }
        shared.device.store(DEVICE_OPEN, Ordering::Relaxed);
        let mut mixer = Mixer::default();
        let mut buffers: Vec<Box<[i16; BUFFER_FRAMES * 2]>> = (0..BUFFERS)
            .map(|_| Box::new([0; BUFFER_FRAMES * 2]))
            .collect();
        let mut headers: Vec<WAVEHDR> = buffers
            .iter_mut()
            .map(|b| WAVEHDR {
                lpData: b.as_mut_ptr().cast(),
                dwBufferLength: (BUFFER_FRAMES * 4) as u32,
                dwBytesRecorded: 0,
                dwUser: 0,
                dwFlags: 0,
                dwLoops: 0,
                lpNext: std::ptr::null_mut(),
                reserved: 0,
            })
            .collect();
        // The driver writes `dwFlags` of queued headers on its own thread: they are read and
        // written only through raw pointers (volatile), never through references.
        let hp = headers.as_mut_ptr();
        let flags = |k: usize| std::ptr::addr_of_mut!((*hp.add(k)).dwFlags);
        let mut prepared = 0;
        for k in 0..BUFFERS {
            let code = waveOutPrepareHeader(device, hp.add(k), size);
            if code != 0 {
                unavailable(code);
                break;
            }
            prepared += 1;
            // Prepared but not yet written: mark done so the loop fills it first.
            flags(k).write_volatile(flags(k).read_volatile() | WHDR_DONE);
        }
        let mut started = false;
        while prepared == BUFFERS && !shared.stop.load(Ordering::Relaxed) {
            accept(shared, &mut mixer, inbox, bin);
            let done = (0..BUFFERS)
                .filter(|&k| flags(k).read_volatile() & WHDR_DONE != 0)
                .count();
            if started && done == BUFFERS {
                shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
            for (k, b) in buffers.iter_mut().enumerate() {
                if flags(k).read_volatile() & WHDR_DONE != 0 {
                    mix(shared, &mut mixer, &mut b[..]);
                    flags(k).write_volatile(flags(k).read_volatile() & !WHDR_DONE);
                    let code = waveOutWrite(device, hp.add(k), size);
                    if code != 0 {
                        // Not queued: refilled (re-mixed) on the next wake-up; observe shows it.
                        flags(k).write_volatile(flags(k).read_volatile() | WHDR_DONE);
                        shared.error.store(code, Ordering::Relaxed);
                        shared.write_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            started = true;
            WaitForSingleObject(event, 50);
        }
        waveOutReset(device);
        for k in 0..prepared {
            waveOutUnprepareHeader(device, hp.add(k), size);
        }
        waveOutClose(device);
        CloseHandle(event);
    }
}
#[cfg(not(windows))]
fn device_loop(shared: &Shared, _: &mpsc::Receiver<Message>, _: &mpsc::SyncSender<Message>) {
    shared.device.store(DEVICE_UNAVAILABLE, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_without_a_device_accepts_updates_and_reports_disabled() {
        let mut out = Output::with_device(false);
        let mut events = Events::default();
        events.waves.insert("pop".into(), vec![1i16; 8].into());
        events.emit(1, "pop");
        out.update(&events, "s1", true, None);
        let o = out.observation();
        assert_eq!(o["device"], "disabled");
        assert_eq!(o["rendered"], 0);
        assert_eq!(o["dropped"], 1, "the cue had nowhere to go");
        drop(out);
    }
}
