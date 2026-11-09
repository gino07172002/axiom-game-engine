//! Asset thumbnails for the Assets browser: a bounded, cancellable background job (one at a
//! time, `jobs::Job`) hashes each source, reuses `<db stem>.thumbs/<hash>.png` when present and
//! otherwise produces the image: PNGs are decoded and fitted on the CPU, models (bind pose,
//! material colours) and materials (a sphere in the material tint) are rendered off-screen on
//! the editor's GPU device from the worker thread and read back. The main thread only uploads
//! finished pixels as egui textures, a few per frame, and keeps at most [`TEXTURE_BUDGET`]
//! (more only while more cells are visible; the least recently drawn are evicted and come back
//! from the disk cache). Nothing here is ever written to a world revision.
use crate::jobs::{Job, Outcome, Progress};
use axiom_core::assets::{AssetEntry, AssetKind};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

/// Thumbnail edge in pixels (RGBA8, 64 KiB each).
pub(crate) const SIZE: u32 = 128;
const BYTES: usize = (SIZE * SIZE * 4) as usize;
/// GPU textures kept for drawn thumbnails (64 x 64 KiB = 4 MiB).
pub(crate) const TEXTURE_BUDGET: usize = 64;
/// egui texture uploads per frame.
pub(crate) const UPLOADS_PER_FRAME: usize = 4;
/// Job results taken from the worker per frame.
const RESULTS_PER_FRAME: usize = 16;
/// Visible thumbnails one lazy job takes; an explicit job takes every thumbnail asset.
pub(crate) const LAZY_BATCH: usize = 24;
pub(crate) const EXPLICIT_LIMIT: usize = 256;
const JOB_TIMEOUT: Duration = Duration::from_secs(120);
/// Triangles drawn for one model thumbnail (larger meshes are thinned evenly).
const MAX_TRIANGLES: usize = 200_000;
/// Source files read for hashing (models / images).
const MAX_SOURCE_BYTES: u64 = 134_217_728;
/// Bump when the produced image changes so stale cache files are not reused.
const VERSION: &str = "axiom-thumb-1";
const NO_GPU: &str = "no GPU: model and material thumbnails render on the editor window's device";

fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
    }
    hash
}
const FNV_START: u64 = 0xcbf29ce484222325;

/// Source hash cache shared by the thumbnail workers.
type Hashes =
    std::sync::Arc<std::sync::Mutex<HashMap<(PathBuf, u64, std::time::SystemTime, String), u64>>>;

/// One thumbnail as the browser knows it: the asset plus a signature of what it points at
/// (file path + import options, or the material value), so an edit makes a new key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ThumbKey {
    pub kind: AssetKind,
    pub id: String,
    pub source: u64,
}

#[derive(Clone, Debug)]
enum Source {
    File { path: PathBuf, options: String },
    Material { tint: [f32; 4], value: String },
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    key: ThumbKey,
    source: Source,
}

/// What the worker needs to make the thumbnail of `entry`, or `None` for kinds without one.
pub(crate) fn request(world: &axiom_core::World, entry: &AssetEntry) -> Option<Request> {
    let (source, signature) = match entry.kind {
        AssetKind::Model => {
            let value = axiom_core::assets::map(world, AssetKind::Model)?.get(&entry.id)?;
            let path = value.get("path")?.as_str()?;
            let options = value
                .get("options")
                .cloned()
                .unwrap_or(json!({}))
                .to_string();
            let signature = format!("{path}\n{options}");
            (
                Source::File {
                    path: PathBuf::from(path),
                    options,
                },
                signature,
            )
        }
        AssetKind::Image => {
            let path = entry.path.as_deref()?;
            (
                Source::File {
                    path: PathBuf::from(path),
                    options: String::new(),
                },
                path.to_owned(),
            )
        }
        AssetKind::Material => {
            let value = axiom_core::assets::map(world, AssetKind::Material)?.get(&entry.id)?;
            let tint = value
                .get("tint")
                .and_then(|t| serde_json::from_value::<[f32; 4]>(t.clone()).ok())
                .unwrap_or([1.0; 4]);
            let value = value.to_string();
            (
                Source::Material {
                    tint,
                    value: value.clone(),
                },
                value,
            )
        }
        _ => return None,
    };
    Some(Request {
        key: ThumbKey {
            kind: entry.kind,
            id: entry.id.clone(),
            source: fnv(FNV_START, signature.as_bytes()),
        },
        source,
    })
}

/// Where a key stands on the main thread.
enum State {
    /// Handed to the running job.
    Queued,
    /// Pixels arrived, not uploaded yet (dropped again if the cell is no longer drawn).
    Pixels(Vec<u8>),
    Texture {
        texture: egui::TextureHandle,
        drawn: u64,
    },
    /// Made before (cache file exists) but not in memory: evicted or never drawn.
    OnDisk,
    Failed(String),
}

/// The running job and its result stream.
struct Running {
    job: Job<Value>,
    results: mpsc::Receiver<(ThumbKey, Result<Vec<u8>, String>)>,
    explicit: bool,
    /// The job's keys in order (progress `done` = index of the item in flight when it stops).
    keys: Vec<ThumbKey>,
}

/// What the browser draws for one cell.
pub(crate) enum Shown<'a> {
    Texture(&'a egui::TextureHandle),
    Pending,
    Failed,
    None,
}

#[derive(Default)]
pub(crate) struct Thumbnails {
    pub dir: Option<PathBuf>,
    pub gpu: Option<(wgpu::Device, wgpu::Queue)>,
    states: HashMap<ThumbKey, State>,
    /// Frame each thumbnail cell was last drawn (one entry per asset ever shown).
    last_drawn: HashMap<ThumbKey, u64>,
    running: Option<Running>,
    /// Cells drawn this frame that need a thumbnail (lazy batch), filled while drawing.
    wanted: Vec<Request>,
    frame: u64,
    uploads: usize,
    textures: usize,
    /// No lazy jobs after a cancel until the world changes or a job is asked for explicitly
    /// (else the visible cells would restart the job at once).
    pub lazy_paused: bool,
    /// Source hashes by (path, length, modified, options), shared with the workers: an
    /// evicted thumbnail comes back from disk without re-reading a large model.
    hashes: Hashes,
    /// Counters for `observe` and tests.
    pub evicted: u64,
    pub uploaded: u64,
    pub last: Option<Value>,
    /// Per-item worker delay (tests drive progress and cancel with it).
    #[cfg(test)]
    pub delay: Duration,
}

impl Thumbnails {
    /// Start of a drawn browser frame (results are taken in `poll`, every frame).
    pub fn begin(&mut self) {
        self.frame += 1;
        self.uploads = 0;
        self.wanted.clear();
    }

    /// Drains results (bounded per frame) and the job outcome; called every frame.
    pub fn poll(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        let mut taken = vec![];
        for _ in 0..RESULTS_PER_FRAME {
            match running.results.try_recv() {
                Ok(result) => taken.push(result),
                Err(_) => break,
            }
        }
        for (key, result) in taken {
            self.accept(key, result);
        }
        let Some(running) = &mut self.running else {
            return;
        };
        if let Some(outcome) = running.job.poll() {
            let (done, total) = running.job.progress();
            let elapsed_ms = running.job.elapsed().as_millis() as u64;
            let state = match &outcome {
                Outcome::Done { stop: None, .. } => "finished",
                Outcome::Done { stop: Some(s), .. } | Outcome::Abandoned { stop: s } => s.name(),
            };
            let mut summary = match outcome {
                Outcome::Done { value, .. } => value,
                Outcome::Abandoned { .. } => json!({}),
            };
            summary["kind"] = json!("thumbnails");
            summary["state"] = json!(state);
            summary["done"] = json!(done);
            summary["total"] = json!(total);
            summary["elapsed_ms"] = json!(elapsed_ms);
            // Results sent before the worker ended are still in the channel.
            let rest: Vec<_> = running.results.try_iter().collect();
            let stuck = (state == "timed_out" || state == "abandoned")
                .then(|| running.keys.get(done as usize).cloned())
                .flatten();
            self.running = None;
            for (key, result) in rest {
                self.accept(key, result);
            }
            // The item a timed-out / stuck worker was on is not retried by the lazy batch.
            if let Some(key) = stuck {
                if matches!(self.states.get(&key), Some(State::Queued)) {
                    self.states.insert(
                        key,
                        State::Failed(format!("thumbnail worker {state} on this item")),
                    );
                }
            }
            if state == "cancelled" {
                self.lazy_paused = true;
            }
            // Items the stopped job never reached are asked for again when drawn.
            self.states.retain(|_, s| !matches!(s, State::Queued));
            self.last = Some(summary);
        }
    }

    /// A finished worker result. Pixels are kept only for a cell drawn in the last frames
    /// (memory stays bounded by what is on screen); others wait on disk until drawn.
    fn accept(&mut self, key: ThumbKey, result: Result<Vec<u8>, String>) {
        let recent = self
            .last_drawn
            .get(&key)
            .is_some_and(|f| *f + 2 >= self.frame);
        let state = match result {
            Ok(pixels) if recent => State::Pixels(pixels),
            Ok(_) => State::OnDisk,
            Err(e) => State::Failed(e),
        };
        if let Some(State::Texture { .. }) = self.states.insert(key, state) {
            self.textures -= 1;
        }
    }

    /// The cell of a thumbnail asset is drawn this frame: its texture (uploading pending
    /// pixels within the per-frame budget) or what to show instead. Unknown thumbnails join
    /// the lazy batch while no job runs. No allocation unless something changes state.
    pub fn shown(&mut self, ctx: &egui::Context, request: Option<&Request>) -> Shown<'_> {
        let Some(request) = request else {
            return Shown::None;
        };
        let frame = self.frame;
        let key = &request.key;
        match self.last_drawn.get_mut(key) {
            Some(f) => *f = frame,
            None => {
                self.last_drawn.insert(key.clone(), frame);
            }
        }
        let upload = matches!(self.states.get(key), Some(State::Pixels(_)))
            && self.uploads < UPLOADS_PER_FRAME;
        if upload {
            let Some(State::Pixels(pixels)) = self.states.remove(key) else {
                unreachable!()
            };
            let image =
                egui::ColorImage::from_rgba_unmultiplied([SIZE as usize, SIZE as usize], &pixels);
            let texture = ctx.load_texture(
                format!("thumb:{}:{}", key.kind.name(), key.id),
                image,
                egui::TextureOptions::LINEAR,
            );
            self.uploads += 1;
            self.uploaded += 1;
            self.textures += 1;
            self.states.insert(
                key.clone(),
                State::Texture {
                    texture,
                    drawn: frame,
                },
            );
        }
        let wanted = matches!(self.states.get(key), None | Some(State::OnDisk));
        let lazy = self.running.is_none() && !self.lazy_paused;
        if wanted && lazy && self.wanted.len() < LAZY_BATCH {
            self.wanted.push(request.clone());
        }
        match self.states.get_mut(key) {
            Some(State::Texture { texture, drawn }) => {
                *drawn = frame;
                Shown::Texture(texture)
            }
            Some(State::Failed(_)) => Shown::Failed,
            Some(State::Queued | State::Pixels(_) | State::OnDisk) | None => Shown::Pending,
        }
    }

    /// Why the thumbnail of `request` failed, if it did (cell tooltip).
    pub fn failure(&self, request: Option<&Request>) -> Option<&str> {
        match self.states.get(&request?.key) {
            Some(State::Failed(e)) => Some(e),
            _ => None,
        }
    }

    /// End of a drawn browser frame: drops pixels of cells no longer drawn, evicts textures
    /// over the budget (least recently drawn first; never one drawn this frame, so more than
    /// the budget can be held only while that many cells are visible) and starts a lazy job
    /// for the visible missing cells.
    pub fn end(&mut self) {
        let frame = self.frame;
        let last_drawn = &self.last_drawn;
        for (key, state) in self.states.iter_mut() {
            if matches!(state, State::Pixels(_))
                && last_drawn.get(key).is_none_or(|f| *f + 2 < frame)
            {
                *state = State::OnDisk;
            }
        }
        if self.textures > TEXTURE_BUDGET {
            let mut drawn: Vec<(u64, ThumbKey)> = self
                .states
                .iter()
                .filter_map(|(k, s)| match s {
                    State::Texture { drawn, .. } if *drawn < self.frame => {
                        Some((*drawn, k.clone()))
                    }
                    _ => None,
                })
                .collect();
            drawn.sort_by_key(|(d, _)| *d);
            for (_, key) in drawn.into_iter().take(self.textures - TEXTURE_BUDGET) {
                self.states.insert(key, State::OnDisk);
                self.textures -= 1;
                self.evicted += 1;
            }
        }
        if self.running.is_none() && !self.wanted.is_empty() {
            let wanted = std::mem::take(&mut self.wanted);
            if let Err(e) = self.start(wanted, false) {
                self.last =
                    Some(json!({"kind":"thumbnails","state":"failed","error":format!("{e:#}")}));
            }
        }
    }

    /// The world changed: lazy jobs may run again.
    pub fn resume_lazy(&mut self) {
        self.lazy_paused = false;
    }

    /// Starts a job for `requests` (an explicit one replaces a running lazy job).
    pub fn start(&mut self, requests: Vec<Request>, explicit: bool) -> anyhow::Result<()> {
        if let Some(running) = self.running.take() {
            anyhow::ensure!(
                !running.explicit,
                "a thumbnail job is running; cancel it first"
            );
            running.job.detach();
            self.states.retain(|_, s| !matches!(s, State::Queued));
        }
        if explicit {
            self.lazy_paused = false;
        }
        if requests.is_empty() {
            return Ok(());
        }
        let keys: Vec<ThumbKey> = requests.iter().map(|r| r.key.clone()).collect();
        let (tx, results) = mpsc::channel();
        let hashes = self.hashes.clone();
        let dir = self.dir.clone();
        let gpu = self.gpu.clone();
        #[cfg(test)]
        let delay = self.delay;
        #[cfg(not(test))]
        let delay = Duration::ZERO;
        let label = format!(
            "{} {} thumbnail(s)",
            if explicit { "generate" } else { "visible" },
            requests.len()
        );
        let job = Job::spawn("thumbnails", label, JOB_TIMEOUT, move |progress| {
            generate(
                &requests,
                dir.as_deref(),
                gpu,
                &hashes,
                delay,
                progress,
                &tx,
            )
        })?;
        // Queued only once the worker exists (a failed spawn leaves the cells retryable).
        for key in &keys {
            if let Some(State::Texture { .. }) = self.states.insert(key.clone(), State::Queued) {
                self.textures -= 1;
            }
        }
        self.running = Some(Running {
            job,
            results,
            explicit,
            keys,
        });
        Ok(())
    }

    pub fn cancel(&mut self) -> bool {
        match &mut self.running {
            Some(running) => {
                running.job.cancel();
                true
            }
            None => false,
        }
    }

    pub fn running(&self) -> Option<&Job<Value>> {
        self.running.as_ref().map(|r| &r.job)
    }

    pub fn explicit_running(&self) -> bool {
        self.running.as_ref().is_some_and(|r| r.explicit)
    }

    /// GPU textures held now.
    #[cfg(test)]
    pub fn textures(&self) -> usize {
        self.textures
    }

    pub fn observation(&self) -> Value {
        let failed = self
            .states
            .values()
            .filter(|s| matches!(s, State::Failed(_)))
            .count();
        json!({
            "textures": self.textures,
            "texture_budget": TEXTURE_BUDGET,
            "texture_bytes": self.textures * BYTES,
            "known": self.states.len(),
            "failed": failed,
            "uploaded": self.uploaded,
            "evicted": self.evicted,
            "cache_dir": self.dir,
            "gpu": self.gpu.is_some(),
            "last_job": self.last,
        })
    }
}

/// Worker: one thumbnail per request, in order, checking for cancellation between items.
fn generate(
    requests: &[Request],
    dir: Option<&Path>,
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    hashes: &Hashes,
    delay: Duration,
    progress: &Progress,
    results: &mpsc::Sender<(ThumbKey, Result<Vec<u8>, String>)>,
) -> Value {
    progress.set_total(requests.len() as u64);
    let mut renderer: Option<Result<ThumbGpu, String>> = None;
    let (mut cached, mut made, mut failed) = (0, 0, 0);
    let mut timings: Vec<u64> = vec![];
    for (n, request) in requests.iter().enumerate() {
        if progress.cancelled() {
            break;
        }
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        let started = std::time::Instant::now();
        let result = one(request, dir, &gpu, hashes, &mut renderer);
        timings.push(started.elapsed().as_micros() as u64);
        match &result {
            Ok((_, true)) => cached += 1,
            Ok((_, false)) => made += 1,
            Err(_) => failed += 1,
        }
        // The browser may be gone (editor closed): stop quietly.
        if results
            .send((request.key.clone(), result.map(|(p, _)| p)))
            .is_err()
        {
            break;
        }
        progress.set_done(n as u64 + 1);
    }
    timings.sort_unstable();
    json!({
        "cached": cached,
        "made": made,
        "failed": failed,
        "item_us_p50": timings.get(timings.len() / 2),
        "item_us_max": timings.last(),
    })
}

/// One thumbnail: `(pixels, from the cache)`.
fn one(
    request: &Request,
    dir: Option<&Path>,
    gpu: &Option<(wgpu::Device, wgpu::Queue)>,
    hashes: &Hashes,
    renderer: &mut Option<Result<ThumbGpu, String>>,
) -> Result<(Vec<u8>, bool), String> {
    let text = |e: anyhow::Error| format!("{e:#}");
    let read = |path: &Path| std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()));
    let (hash, mut bytes) = match &request.source {
        Source::File { path, options } => {
            let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
            if meta.len() > MAX_SOURCE_BYTES {
                return Err(format!("{} exceeds 128 MiB", path.display()));
            }
            // Same file (length + modified time) as hashed before: no re-read.
            let id = (
                path.clone(),
                meta.len(),
                meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                options.clone(),
            );
            let known = hashes.lock().ok().and_then(|h| h.get(&id).copied());
            match known {
                Some(hash) => (hash, None),
                None => {
                    let bytes = read(path)?;
                    let hash = fnv(
                        fnv(fnv(FNV_START, VERSION.as_bytes()), options.as_bytes()),
                        &bytes,
                    );
                    if let Ok(mut h) = hashes.lock() {
                        if h.len() >= 1024 {
                            h.clear();
                        }
                        h.insert(id, hash);
                    }
                    (hash, Some(bytes))
                }
            }
        }
        Source::Material { value, .. } => (
            fnv(fnv(FNV_START, VERSION.as_bytes()), value.as_bytes()),
            None,
        ),
    };
    let file = dir.map(|d| d.join(format!("{hash:016x}.png")));
    if let Some(pixels) = file.as_deref().and_then(read_png) {
        return Ok((pixels, true));
    }
    if request.key.kind != AssetKind::Image && gpu.is_none() {
        // Before parsing the model: without a device there is nothing to render it with.
        return Err(NO_GPU.into());
    }
    let pixels = match (&request.source, request.key.kind) {
        (Source::File { path, .. }, AssetKind::Image) => {
            if bytes.is_none() {
                bytes = Some(read(path)?);
            }
            let texture = axiom_animation::import::texture(bytes.as_deref().unwrap_or_default())
                .map_err(text)?;
            fit(texture.width, texture.height, &texture.rgba)
        }
        (Source::File { path, options }, _) => {
            let options: axiom_animation::import::ImportOptions =
                serde_json::from_str(options).map_err(|e| e.to_string())?;
            let asset = axiom_animation::import::load(path, &options).map_err(text)?;
            let mesh = Mesh::model(&asset);
            gpu_render(gpu, renderer, &mesh)?
        }
        (Source::Material { tint, .. }, _) => gpu_render(gpu, renderer, &Mesh::sphere(*tint))?,
    };
    if let Some(file) = &file {
        write_png(file, &pixels).map_err(text)?;
    }
    Ok((pixels, false))
}

fn gpu_render(
    gpu: &Option<(wgpu::Device, wgpu::Queue)>,
    renderer: &mut Option<Result<ThumbGpu, String>>,
    mesh: &Mesh,
) -> Result<Vec<u8>, String> {
    let Some((device, queue)) = gpu else {
        return Err(NO_GPU.into());
    };
    let gpu = renderer
        .get_or_insert_with(|| Ok(ThumbGpu::new(device.clone(), queue.clone())))
        .as_mut()
        .map_err(|e| e.clone())?;
    let result = gpu.render(mesh).map_err(|e| format!("{e:#}"));
    if result.is_err() {
        // A failed readback may leave the buffer mapped: start the next item afresh.
        *renderer = None;
    }
    result
}

fn read_png(path: &Path) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if info.width != SIZE
        || info.height != SIZE
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
    {
        return None;
    }
    let mut pixels = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).ok()?;
    (pixels.len() == BYTES).then_some(pixels)
}

/// Writes through a temporary file so a reader never sees half a PNG.
fn write_png(path: &Path, pixels: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Unique per write: a replaced (detached) worker may write the same content-addressed file.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = path.with_extension(format!("{}-{n}.part", std::process::id()));
    {
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(&partial)?),
            SIZE,
            SIZE,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.write_header()?.write_image_data(pixels)?;
    }
    if let Err(e) = std::fs::rename(&partial, path) {
        let _ = std::fs::remove_file(&partial);
        // Another editor wrote the same content-addressed file first.
        if !path.exists() {
            return Err(e.into());
        }
    }
    Ok(())
}

/// Fits an image into the square (aspect kept, centred, transparent margin) with a box filter.
fn fit(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; BYTES];
    if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
        return out;
    }
    let scale = (SIZE as f32 / width as f32).min(SIZE as f32 / height as f32);
    let (w, h) = (
        ((width as f32 * scale).round() as u32).clamp(1, SIZE),
        ((height as f32 * scale).round() as u32).clamp(1, SIZE),
    );
    let (ox, oy) = ((SIZE - w) / 2, (SIZE - h) / 2);
    for y in 0..h {
        let (sy0, sy1) = (
            (y as f32 / scale) as u32,
            (((y + 1) as f32 / scale) as u32).clamp((y as f32 / scale) as u32 + 1, height),
        );
        for x in 0..w {
            let (sx0, sx1) = (
                (x as f32 / scale) as u32,
                (((x + 1) as f32 / scale) as u32).clamp((x as f32 / scale) as u32 + 1, width),
            );
            let mut sum = [0u32; 4];
            let mut n = 0;
            for sy in sy0.min(height - 1)..sy1 {
                for sx in sx0.min(width - 1)..sx1 {
                    let i = ((sy * width + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += u32::from(rgba[i + c]);
                    }
                    n += 1;
                }
            }
            let o = (((oy + y) * SIZE + ox + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (sum[c] / n.max(1)) as u8;
            }
        }
    }
    out
}

/// Interleaved position, normal, colour.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
}

struct Mesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl Mesh {
    /// Bind pose; each vertex takes the colour of the material drawing it (texture averaged).
    fn model(asset: &axiom_animation::Asset) -> Self {
        let mut vertices: Vec<Vertex> = asset
            .vertices
            .iter()
            .map(|v| Vertex {
                position: v.position.to_array(),
                normal: v.normal.to_array(),
                color: [0.8, 0.8, 0.8, 1.0],
            })
            .collect();
        for m in &asset.materials {
            let mut color = m.color;
            if let Some(t) = m.texture.and_then(|i| asset.textures.get(i)) {
                let average = average(&t.rgba);
                for c in 0..3 {
                    color[c] *= average[c];
                }
            }
            let range = m.first_index as usize..(m.first_index + m.index_count) as usize;
            for &i in asset.indices.get(range).unwrap_or_default() {
                if let Some(v) = vertices.get_mut(i as usize) {
                    v.color = color;
                }
            }
        }
        let triangles = asset.indices.len() / 3;
        let step = triangles.div_ceil(MAX_TRIANGLES).max(1);
        let indices = asset
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .step_by(step)
            .flatten()
            .copied()
            .collect();
        Self { vertices, indices }
    }

    /// A UV sphere in `tint` (material thumbnails show the tint, not the material's shader).
    fn sphere(tint: [f32; 4]) -> Self {
        let (rings, segments) = (16u32, 32u32);
        let mut vertices = vec![];
        for r in 0..=rings {
            let phi = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..=segments {
                let theta = std::f32::consts::TAU * s as f32 / segments as f32;
                let n = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
                vertices.push(Vertex {
                    position: n,
                    normal: n,
                    color: [tint[0], tint[1], tint[2], 1.0],
                });
            }
        }
        let mut indices = vec![];
        for r in 0..rings {
            for s in 0..segments {
                let a = r * (segments + 1) + s;
                let b = a + segments + 1;
                indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        Self { vertices, indices }
    }

    /// Centre and half extents of the drawn triangles.
    fn bounds(&self) -> (glam::Vec3, glam::Vec3) {
        let mut min = glam::Vec3::splat(f32::MAX);
        let mut max = glam::Vec3::splat(f32::MIN);
        for v in self
            .indices
            .iter()
            .filter_map(|&i| self.vertices.get(i as usize))
        {
            let p = glam::Vec3::from_array(v.position);
            min = min.min(p);
            max = max.max(p);
        }
        if min.x > max.x {
            return (glam::Vec3::ZERO, glam::Vec3::ONE);
        }
        (
            (min + max) * 0.5,
            ((max - min) * 0.5).max(glam::Vec3::splat(1e-3)),
        )
    }
}

fn average(rgba: &[u8]) -> [f32; 3] {
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    // Every 7th opaque texel is plenty for an average colour (transparent padding of a
    // texture atlas would darken it).
    for px in rgba
        .as_chunks::<4>()
        .0
        .iter()
        .step_by(7)
        .filter(|p| p[3] >= 128)
    {
        for c in 0..3 {
            sum[c] += u64::from(px[c]);
        }
        n += 1;
    }
    if n == 0 {
        return [1.0; 3];
    }
    sum.map(|s| (s as f32 / n as f32 / 255.0).powf(2.2))
}

const THUMB_WGSL: &str = r#"
struct Globals { view_proj: mat4x4<f32>, light: vec4<f32> };
@group(0) @binding(0) var<uniform> g: Globals;
struct In { @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32> };
struct Out { @builtin(position) clip: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) color: vec4<f32> };
@vertex fn vs(v: In) -> Out {
    var o: Out;
    o.clip = g.view_proj * vec4<f32>(v.position, 1.0);
    o.normal = v.normal;
    o.color = v.color;
    return o;
}
@fragment fn fs(i: Out) -> @location(0) vec4<f32> {
    let len = length(i.normal);
    let n = select(vec3<f32>(0.0, 1.0, 0.0), i.normal / max(len, 1e-6), len > 1e-5);
    let key = max(dot(n, normalize(g.light.xyz)), 0.0);
    let fill = max(dot(n, normalize(vec3<f32>(-0.5, 0.2, -0.6))), 0.0) * 0.3;
    return vec4<f32>(i.color.rgb * (0.28 + 0.8 * key + fill), 1.0);
}
"#;

/// Off-screen thumbnail renderer: one pipeline, a 128 x 128 colour + depth target and a
/// readback buffer, created once per job on the editor's device (used from the worker).
pub(crate) struct ThumbGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    color: wgpu::Texture,
    depth: wgpu::TextureView,
    readback: wgpu::Buffer,
}

impl ThumbGpu {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("thumbnail"),
            source: wgpu::ShaderSource::Wgsl(THUMB_WGSL.into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("thumbnail globals"),
            size: 80,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("thumbnail globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("thumbnail globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("thumbnail"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&layout],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &ATTRIBUTES,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            // Imported meshes may be single-sided or mirrored: draw both faces.
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let extent = wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("thumbnail colour"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("thumbnail depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("thumbnail readback"),
            size: BYTES as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            device,
            queue,
            pipeline,
            uniform,
            bind,
            color,
            depth,
            readback,
        }
    }

    /// Renders `mesh` framed from the front-right-above and reads the pixels back (RGBA8,
    /// sRGB). Blocks this (worker) thread until the GPU finished this submission.
    fn render(&mut self, mesh: &Mesh) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(!mesh.indices.is_empty(), "mesh has no triangles");
        use wgpu::util::DeviceExt;
        let (center, half) = mesh.bounds();
        let fov = 30f32.to_radians();
        let direction = glam::Vec3::new(0.55, 0.35, 1.0).normalize();
        // Fit the larger of the height and the footprint (tall characters fill the frame
        // instead of shrinking into their bounding sphere); the depth range covers the box.
        let fit = half.y.max(half.x.max(half.z));
        let radius = half.length();
        let distance = (fit / (fov * 0.5).tan() * 1.15).max(radius * 1.05);
        let eye = center + direction * distance;
        let projection = glam::Mat4::perspective_rh(
            fov,
            1.0,
            (distance - radius * 1.5).max(distance * 0.01),
            distance + radius * 1.5,
        );
        let view_proj = projection * glam::Mat4::look_at_rh(eye, center, glam::Vec3::Y);
        let mut globals = [0f32; 20];
        globals[..16].copy_from_slice(&view_proj.to_cols_array());
        globals[16..19].copy_from_slice(&[0.6, 0.8, 0.9]);
        self.queue
            .write_buffer(&self.uniform, 0, bytemuck::cast_slice(&globals));
        let vertices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("thumbnail vertices"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let indices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("thumbnail indices"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let view = self.color.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("thumbnail"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("thumbnail"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.035,
                            g: 0.045,
                            b: 0.06,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    // 128 px x 4 B = 512 B, already a multiple of the 256 B row alignment.
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = self.readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        // Non-blocking polls: a waiting poll holds the device fence while it waits, which
        // would stall the editor's own frame submissions on the main thread.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mapped = loop {
            self.device.poll(wgpu::Maintain::Poll);
            match rx.try_recv() {
                Ok(result) => break result,
                Err(mpsc::TryRecvError::Disconnected) => {
                    anyhow::bail!("thumbnail readback was dropped")
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "thumbnail readback timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        mapped?;
        let pixels = slice.get_mapped_range().to_vec();
        self.readback.unmap();
        Ok(pixels)
    }
}

/// GPU check (`cargo run -p axiom-runtime --example verify_thumbnails_gpu`): renders each model
/// file and a tinted material sphere off-screen on a headless device, reads them back, writes
/// `out/<name>.png` and reports timings and how much of each image the mesh covers.
pub fn validate_thumbnails_gpu(models: &[PathBuf], out: &Path) -> anyhow::Result<Value> {
    use anyhow::Context;
    let (device, queue) = pollster::block_on(async {
        let instance = wgpu::Instance::new(&Default::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .context("GPU adapter unavailable")?;
        let info = adapter.get_info();
        let pair = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("thumbnail validation"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                },
                None,
            )
            .await?;
        anyhow::Ok((pair, format!("{} / {:?}", info.name, info.backend)))
    })
    .map(|(pair, gpu)| {
        println!("{{\"gpu\":{gpu:?}}}");
        pair
    })?;
    let started = std::time::Instant::now();
    let mut gpu = ThumbGpu::new(device, queue);
    let setup_ms = started.elapsed().as_secs_f64() * 1000.0;
    // Pixels that differ from the clear colour.
    let coverage = |pixels: &[u8]| {
        let background = pixels[..4].to_vec();
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[..3] != background[..3])
            .count() as f64
            / (SIZE * SIZE) as f64
    };
    let mut items = vec![];
    let mut render = |name: String, mesh: &Mesh, load_ms: f64| -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        let pixels = gpu.render(mesh)?;
        let render_ms = started.elapsed().as_secs_f64() * 1000.0;
        let file = out.join(format!("{name}.png"));
        write_png(&file, &pixels)?;
        items.push(json!({
            "name": name, "triangles": mesh.indices.len() / 3, "load_ms": load_ms,
            "render_readback_ms": render_ms, "coverage": coverage(&pixels), "png": file,
        }));
        Ok(())
    };
    for path in models {
        let started = std::time::Instant::now();
        let asset = axiom_animation::import::load(path, &Default::default())?;
        let mesh = Mesh::model(&asset);
        let load_ms = started.elapsed().as_secs_f64() * 1000.0;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model")
            .to_owned();
        render(name, &mesh, load_ms)?;
    }
    render(
        "material-sphere".into(),
        &Mesh::sphere([0.9, 0.45, 0.2, 1.0]),
        0.0,
    )?;
    Ok(json!({"ok": true, "size": SIZE, "pipeline_setup_ms": setup_ms, "items": items}))
}
