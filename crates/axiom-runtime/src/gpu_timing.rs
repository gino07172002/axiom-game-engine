//! GPU time of the renderer's pass groups for the editor Profiler, from encoder timestamp
//! queries (`TIMESTAMP_QUERY` + `TIMESTAMP_QUERY_INSIDE_ENCODERS`).
//!
//! Only frames rendered while the host enables timing write timestamps. Each such frame takes
//! one slot of a small ring (query set, resolve buffer, readback buffer); the readback maps
//! asynchronously and is collected on a later frame with a non-blocking poll. When every slot
//! is still waiting for its readback the frame is simply not measured: the main thread never
//! waits for the GPU here.
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Pass groups, in encoding order. The time between two timestamps belongs to the group of
/// the later one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    VfxCompute,
    Shadows,
    World,
    VfxDraw,
    Post,
    GameUi,
    Ui,
}

pub const SECTIONS: usize = 7;

impl Section {
    pub const ALL: [Section; SECTIONS] = [
        Section::VfxCompute,
        Section::Shadows,
        Section::World,
        Section::VfxDraw,
        Section::Post,
        Section::GameUi,
        Section::Ui,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Section::VfxCompute => "vfx_compute",
            Section::Shadows => "shadows",
            Section::World => "world",
            Section::VfxDraw => "vfx_draw",
            Section::Post => "post",
            Section::GameUi => "game_ui",
            Section::Ui => "ui",
        }
    }
}

/// One measured frame: milliseconds per section and from the first to the last timestamp.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuFrame {
    pub ms: [f32; SECTIONS],
    pub total: f32,
}

/// Timestamps one frame may write: the start, VFX compute, shadows, three per camera view
/// (`MAX_CAMERAS` = 8) and two interface passes is 29.
const QUERIES: u32 = 32;
/// Frames in flight at most (readbacks usually complete 1–3 frames later).
const SLOTS: usize = 4;

const IDLE: u8 = 0;
const RECORDING: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;
const FAILED: u8 = 4;

struct Slot {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// Written by the map callback (any thread), read on the main thread.
    state: Arc<AtomicU8>,
    sections: [Option<Section>; QUERIES as usize],
    count: u32,
    overflow: bool,
}

pub struct GpuTimer {
    unavailable: Option<&'static str>,
    /// Nanoseconds per timestamp tick (`Queue::get_timestamp_period`).
    period: f32,
    enabled: bool,
    slots: Vec<Slot>,
    active: Option<usize>,
    results: Vec<GpuFrame>,
    /// Frames rendered with timing on but not measured (every slot awaited its readback) or
    /// thrown away (timestamps out of order, too many passes, map failure) since `take`.
    dropped: u64,
}

impl GpuTimer {
    /// The device features timing needs, when the adapter has all of them.
    /// With `AXIOM_GPU_TIMING=0` none: the device is created exactly as without timing
    /// (overhead A/B measurements).
    pub fn features(adapter: wgpu::Features) -> wgpu::Features {
        let needed =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        if adapter.contains(needed) && !off_by_env() {
            needed
        } else {
            wgpu::Features::empty()
        }
    }

    /// `device` = the features the device was created with.
    pub fn new(adapter: wgpu::Features, device: wgpu::Features, queue: &wgpu::Queue) -> Self {
        let unavailable = if off_by_env() {
            Some("turned off by AXIOM_GPU_TIMING=0")
        } else {
            unavailable(adapter, device)
        };
        Self {
            unavailable,
            period: if unavailable.is_none() {
                queue.get_timestamp_period()
            } else {
                0.0
            },
            enabled: false,
            slots: vec![],
            active: None,
            results: Vec::with_capacity(SLOTS),
            dropped: 0,
        }
    }

    /// Why GPU timing cannot run on this device (`None`: it can).
    pub fn unavailable(&self) -> Option<&'static str> {
        self.unavailable
    }

    /// Whether the following frames write timestamps (the editor: Profiler visible).
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on && self.unavailable.is_none();
    }

    /// Start of a frame's encoding: collects finished readbacks and, when enabled and a slot
    /// is free, writes the frame's first timestamp.
    pub fn begin(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if let Some(i) = self.active.take() {
            // The previous frame was encoded but never submitted (an error path).
            self.slots[i].state.store(IDLE, Ordering::Release);
        }
        self.collect(device);
        if !self.enabled {
            return;
        }
        if self.slots.is_empty() {
            self.slots = (0..SLOTS).map(|i| Slot::new(device, i)).collect();
        }
        let Some(i) = self
            .slots
            .iter()
            .position(|s| s.state.load(Ordering::Acquire) == IDLE)
        else {
            self.dropped += 1;
            return;
        };
        let slot = &mut self.slots[i];
        slot.state.store(RECORDING, Ordering::Release);
        slot.count = 1;
        slot.overflow = false;
        slot.sections[0] = None;
        encoder.write_timestamp(&slot.queries, 0);
        self.active = Some(i);
    }

    /// Ends the time of `section` here (between passes, never inside one).
    pub fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, section: Section) {
        let Some(i) = self.active else {
            return;
        };
        let slot = &mut self.slots[i];
        if slot.count == QUERIES {
            slot.overflow = true;
            return;
        }
        encoder.write_timestamp(&slot.queries, slot.count);
        slot.sections[slot.count as usize] = Some(section);
        slot.count += 1;
    }

    /// Last commands of the frame's encoder: resolve and copy into the readback buffer.
    pub fn end(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(i) = self.active else {
            return;
        };
        let slot = &self.slots[i];
        encoder.resolve_query_set(&slot.queries, 0..slot.count, &slot.resolve, 0);
        encoder.copy_buffer_to_buffer(
            &slot.resolve,
            0,
            &slot.readback,
            0,
            u64::from(slot.count) * wgpu::QUERY_SIZE as u64,
        );
    }

    /// After the encoder was submitted: starts the readback (completes on a later poll).
    pub fn submitted(&mut self) {
        let Some(i) = self.active.take() else {
            return;
        };
        let slot = &self.slots[i];
        slot.state.store(MAPPING, Ordering::Release);
        let state = slot.state.clone();
        slot.readback
            .slice(..u64::from(slot.count) * wgpu::QUERY_SIZE as u64)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { FAILED },
                    Ordering::Release,
                );
            });
    }

    /// Readbacks that completed since the last call; never waits for one.
    fn collect(&mut self, device: &wgpu::Device) {
        let pending = |state: u8| {
            self.slots
                .iter()
                .any(|s| s.state.load(Ordering::Acquire) == state)
        };
        if pending(MAPPING) {
            device.poll(wgpu::Maintain::Poll);
        } else if !pending(MAPPED) && !pending(FAILED) {
            // Map callbacks also run in other polls and submits (e.g. a screenshot's
            // blocking poll), so finished slots are collected even when none is mapping.
            return;
        }
        for slot in &mut self.slots {
            match slot.state.load(Ordering::Acquire) {
                MAPPED => {
                    let bytes = u64::from(slot.count) * wgpu::QUERY_SIZE as u64;
                    let frame = {
                        let view = slot.readback.slice(..bytes).get_mapped_range();
                        let mut ticks = [0_u64; QUERIES as usize];
                        for (tick, chunk) in ticks.iter_mut().zip(view.as_chunks::<8>().0) {
                            *tick = u64::from_le_bytes(*chunk);
                        }
                        frame(&ticks[..slot.count as usize], &slot.sections, self.period)
                    };
                    slot.readback.unmap();
                    slot.state.store(IDLE, Ordering::Release);
                    match frame.filter(|_| !slot.overflow) {
                        Some(frame) if self.results.len() < SLOTS => self.results.push(frame),
                        _ => self.dropped += 1,
                    }
                }
                FAILED => {
                    slot.state.store(IDLE, Ordering::Release);
                    self.dropped += 1;
                }
                _ => {}
            }
        }
    }

    /// Hands over the frames measured since the last call (oldest first); returns how many
    /// frames went unmeasured meanwhile.
    pub fn take(&mut self, mut each: impl FnMut(&GpuFrame)) -> u64 {
        for frame in &self.results {
            each(frame);
        }
        self.results.clear();
        std::mem::take(&mut self.dropped)
    }
}

impl Slot {
    fn new(device: &wgpu::Device, index: usize) -> Self {
        let bytes = u64::from(QUERIES) * wgpu::QUERY_SIZE as u64;
        let label = format!("gpu timing {index}");
        Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some(&label),
                ty: wgpu::QueryType::Timestamp,
                count: QUERIES,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&label),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&label),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            state: Arc::new(AtomicU8::new(IDLE)),
            sections: [None; QUERIES as usize],
            count: 0,
            overflow: false,
        }
    }
}

fn off_by_env() -> bool {
    std::env::var_os("AXIOM_GPU_TIMING").is_some_and(|v| v == "0")
}

/// Why timing cannot run with these adapter / device features.
fn unavailable(adapter: wgpu::Features, device: wgpu::Features) -> Option<&'static str> {
    if !adapter.contains(wgpu::Features::TIMESTAMP_QUERY) {
        Some("the adapter has no TIMESTAMP_QUERY")
    } else if !adapter.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
        Some("the adapter has TIMESTAMP_QUERY but not TIMESTAMP_QUERY_INSIDE_ENCODERS (passes are timed from the encoder)")
    } else if !device.contains(GpuTimer::features(adapter)) {
        Some("the device was created without timestamp queries")
    } else {
        None
    }
}

/// Section times from resolved timestamps; `None` when they run backwards (a driver that
/// reorders across passes or a counter wrap) or there is no interval.
fn frame(ticks: &[u64], sections: &[Option<Section>], period: f32) -> Option<GpuFrame> {
    if ticks.len() < 2 || ticks.windows(2).any(|w| w[1] < w[0]) {
        return None;
    }
    let ms = |t: u64| (t as f64 * f64::from(period) / 1e6) as f32;
    let mut frame = GpuFrame {
        total: ms(ticks[ticks.len() - 1] - ticks[0]),
        ..Default::default()
    };
    for (k, w) in ticks.windows(2).enumerate() {
        if let Some(section) = sections[k + 1] {
            frame.ms[section as usize] += ms(w[1] - w[0]);
        }
    }
    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_belong_to_the_later_mark_and_backwards_time_is_rejected() {
        let mut sections = [None; QUERIES as usize];
        sections[1] = Some(Section::Shadows);
        sections[2] = Some(Section::World);
        sections[3] = Some(Section::World);
        sections[4] = Some(Section::Ui);
        // Period 2 ns per tick: 500_000 ticks = 1 ms.
        let f = frame(
            &[1000, 501_000, 1_501_000, 1_751_000, 2_001_000],
            &sections,
            2.0,
        )
        .unwrap();
        assert_eq!(f.ms[Section::Shadows as usize], 1.0);
        assert_eq!(f.ms[Section::World as usize], 2.5);
        assert_eq!(f.ms[Section::Ui as usize], 0.5);
        assert_eq!(f.total, 4.0);
        assert!(frame(&[10, 5], &sections, 1.0).is_none());
        assert!(frame(&[10], &sections, 1.0).is_none());
        assert_eq!(
            Section::ALL.map(|s| s as usize),
            [0, 1, 2, 3, 4, 5, 6],
            "ALL indexes GpuFrame::ms"
        );
    }

    /// Release measurement (`cargo test --release -p axiom-runtime --lib measure_gpu_timer --
    /// --ignored --nocapture`): CPU cost of the timer per frame on a headless device with the
    /// renderer's pass count (8 render passes), timing on vs off, interleaved rounds.
    #[test]
    #[ignore]
    fn measure_gpu_timer_overhead() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: GpuTimer::features(adapter.features()),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .expect("device");
        let mut timer = GpuTimer::new(adapter.features(), device.features(), &queue);
        assert!(timer.unavailable().is_none(), "{:?}", timer.unavailable());
        let target = device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1920,
                    height: 1080,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let frame = |timer: &mut GpuTimer| {
            let started = std::time::Instant::now();
            let mut encoder = device.create_command_encoder(&Default::default());
            timer.begin(&device, &mut encoder);
            for section in Section::ALL.iter().chain([Section::World].iter()) {
                drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                }));
                timer.mark(&mut encoder, *section);
            }
            timer.end(&mut encoder);
            queue.submit(std::iter::once(encoder.finish()));
            timer.submitted();
            let cpu = started.elapsed().as_secs_f64() * 1e6;
            // Pace like a presenting frame loop (the GPU finishes between frames).
            device.poll(wgpu::Maintain::Wait);
            cpu
        };
        let (mut on, mut off) = (vec![], vec![]);
        let mut measured = 0;
        for round in 0..10 {
            for enabled in [round % 2 == 0, round % 2 != 0] {
                timer.set_enabled(enabled);
                let mut samples: Vec<f64> = (0..400).map(|_| frame(&mut timer)).collect();
                samples.sort_by(f64::total_cmp);
                let median = samples[samples.len() / 2];
                if enabled {
                    on.push(median)
                } else {
                    off.push(median)
                }
                timer.take(|f| {
                    measured += 1;
                    assert!(f.total > 0.0);
                });
            }
        }
        on.sort_by(f64::total_cmp);
        off.sort_by(f64::total_cmp);
        println!(
            "gpu timer CPU per frame (median of round medians, us): on {:.1} off {:.1}; on rounds {on:.1?} off rounds {off:.1?}; measured frames {measured}",
            on[on.len() / 2],
            off[off.len() / 2]
        );
        assert!(measured > 0);
    }

    #[test]
    fn unavailability_is_explained_per_missing_feature() {
        use wgpu::Features as F;
        let both = F::TIMESTAMP_QUERY | F::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        assert_eq!(GpuTimer::features(F::empty()), F::empty());
        assert_eq!(GpuTimer::features(F::TIMESTAMP_QUERY), F::empty());
        assert_eq!(GpuTimer::features(both | F::DEPTH_CLIP_CONTROL), both);
        assert!(unavailable(F::empty(), F::empty())
            .unwrap()
            .contains("no TIMESTAMP_QUERY"));
        assert!(unavailable(F::TIMESTAMP_QUERY, F::TIMESTAMP_QUERY)
            .unwrap()
            .contains("INSIDE_ENCODERS"));
        assert!(unavailable(both, F::empty()).unwrap().contains("device"));
        assert_eq!(unavailable(both, both), None);
    }
}
