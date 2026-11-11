//! On-demand screenshots of the running window (game or editor) without restarting the
//! process: the loopback op `{"op":"screenshot","path":"…png","region":"window|view"}` and the
//! editor's `view.screenshot`. One capture is pending at a time; the renderer copies the next
//! presented frame into a read-back buffer and maps it asynchronously (`Renderer::grab` /
//! `take_grab`: the main thread never waits for the GPU), and a bounded background job copies
//! the crop out of the mapped buffer, converts BGRA and writes the PNG. The loopback reply is sent
//! when the file is written: `{ok, path, width, height, region, ms}`.
use crate::jobs::{Job, Outcome, Progress};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::SyncSender,
    time::{Duration, Instant},
};

pub(crate) const JOB_TIMEOUT: Duration = Duration::from_secs(20);
const JOIN_LIMIT: Duration = Duration::from_millis(500);
/// A request no rendered frame picked up within this time fails (window minimized).
pub(crate) const PENDING_LIMIT: Duration = Duration::from_secs(10);

/// `Ok((path, width, height))` of the written PNG.
pub(crate) type Written = Result<(PathBuf, u32, u32), String>;

/// A `.png` path for a capture: relative paths resolve against the working directory; no `..`
/// components, at most 1024 bytes. The folder is created when the file is written.
pub(crate) fn validate_path(text: &str) -> anyhow::Result<PathBuf> {
    let text = text.trim();
    anyhow::ensure!(
        !text.is_empty() && text.len() <= 1024,
        "screenshot path must be 1..1024 bytes"
    );
    let path = Path::new(text);
    anyhow::ensure!(
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png")),
        "screenshot path must end in .png"
    );
    anyhow::ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "screenshot path must not contain .."
    );
    anyhow::ensure!(
        path.file_stem().is_some_and(|s| !s.is_empty()),
        "screenshot path needs a file name"
    );
    // Either relative (resolved against the working directory) or a plain absolute path on a
    // local drive: no drive-relative `C:a.png`, root-relative `\a.png`, UNC shares, verbatim
    // or device paths, and no reserved device names.
    use std::path::{Component, Prefix};
    let mut components = path.components().peekable();
    let plain = match components.peek() {
        Some(Component::Prefix(p)) => {
            matches!(p.kind(), Prefix::Disk(_))
                && path.has_root()
                && components.nth(1) == Some(Component::RootDir)
        }
        Some(Component::RootDir) => !cfg!(windows),
        _ => true,
    };
    anyhow::ensure!(
        plain,
        "screenshot path must be relative or an absolute path on a local drive (C:\\…)"
    );
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    anyhow::ensure!(!reserved, "screenshot path uses a reserved device name");
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    })
}

/// `rect` (points) → `[x, y, w, h]` physical pixels inside a `width` × `height` frame (at
/// least one pixel).
pub(crate) fn physical(rect: egui::Rect, scale: f32, width: u32, height: u32) -> [u32; 4] {
    let clamp = |v: f32, max: u32| (v * scale).round().clamp(0.0, max as f32) as u32;
    let (x0, y0) = (clamp(rect.min.x, width), clamp(rect.min.y, height));
    let (x1, y1) = (clamp(rect.max.x, width), clamp(rect.max.y, height));
    let x = x0.min(width.saturating_sub(1));
    let y = y0.min(height.saturating_sub(1));
    [
        x,
        y,
        x1.saturating_sub(x).max(1),
        y1.saturating_sub(y).max(1),
    ]
}

/// Copies the crop out of the rows (BGRA → RGBA when `bgra`) and writes it as PNG (worker).
pub(crate) fn write_crop(
    path: &Path,
    stride: usize,
    pixels: &[u8],
    bgra: bool,
    [x, y, w, h]: [u32; 4],
    progress: &Progress,
) -> Result<(u32, u32), String> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        if progress.cancelled() {
            return Err("cancelled".into());
        }
        let start = row as usize * stride + x as usize * 4;
        let line = pixels
            .get(start..start + w as usize * 4)
            .ok_or("crop outside the frame")?;
        out.extend_from_slice(line);
    }
    if bgra {
        for pixel in out.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    }
    let text = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| text(&e))?;
    }
    let file = std::fs::File::create(path).map_err(|e| text(&e))?;
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .and_then(|mut writer| writer.write_image_data(&out))
        .map_err(|e| text(&e))?;
    Ok((w, h))
}

/// A grabbed frame: RGBA / BGRA rows `stride` bytes apart, in memory or in a mapped
/// read-back buffer (the job copies the crop out of it and unmaps it).
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: bool,
    pub stride: u32,
    pub data: FrameData,
}

pub enum FrameData {
    Pixels(Vec<u8>),
    Mapped(wgpu::Buffer),
}

/// Starts the crop / encode / write job for a grabbed frame.
pub(crate) fn spawn(
    path: PathBuf,
    frame: Frame,
    crop: Option<[u32; 4]>,
) -> anyhow::Result<Job<Written>> {
    let crop = crop.unwrap_or([0, 0, frame.width, frame.height]);
    let label = path.display().to_string();
    Job::spawn("screenshot", label, JOB_TIMEOUT, move |progress| {
        progress.set_total(1);
        let started = Instant::now();
        let (stride, bgra) = (frame.stride as usize, frame.bgra);
        let written = match &frame.data {
            FrameData::Pixels(pixels) => write_crop(&path, stride, pixels, bgra, crop, progress),
            FrameData::Mapped(buffer) => {
                let mapped = buffer.slice(..).get_mapped_range();
                let written = write_crop(&path, stride, &mapped, bgra, crop, progress);
                drop(mapped);
                buffer.unmap();
                written
            }
        };
        progress.set_done(1);
        written.map(|(w, h)| {
            eprintln!(
                "screenshot: {} ({w}x{h}) written in {:.0} ms",
                path.display(),
                started.elapsed().as_secs_f64() * 1000.0
            );
            (path, w, h)
        })
    })
}

/// The job's result once it ended (`None` while running).
pub(crate) fn poll(job: &mut Job<Written>) -> Option<Written> {
    match job.poll()? {
        Outcome::Done { value, stop: None } => Some(value),
        Outcome::Done {
            stop: Some(stop), ..
        }
        | Outcome::Abandoned { stop } => Some(Err(format!("screenshot job {}", stop.name()))),
    }
}

/// The loopback capture of the host (`App`): one at a time.
#[derive(Default)]
pub(crate) struct Loopback {
    pending: Option<Request>,
    job: Option<(Job<Written>, Request)>,
    /// `{ok, path, width, height, …}` or `{ok:false, error}` of the last capture.
    pub last: Option<Value>,
}

/// A loopback request waiting for its frame / file.
pub(crate) struct Request {
    pub path: PathBuf,
    /// Crop to the editor's Scene / Game view (`region:"view"`), else the whole window.
    pub view: bool,
    pub asked: Instant,
    pub reply: Option<SyncSender<Value>>,
}

impl Loopback {
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.job.is_some()
    }
    pub fn request(&mut self, request: Request) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.busy(),
            "a screenshot is already being captured; one at a time"
        );
        self.pending = Some(request);
        Ok(())
    }
    /// The bridge reply of the request just queued.
    pub fn set_reply(&mut self, reply: SyncSender<Value>) {
        if let Some(pending) = &mut self.pending {
            pending.reply = Some(reply);
        }
    }
    pub fn wants_grab(&self) -> bool {
        self.pending.is_some()
    }
    /// Whether the pending capture crops to the editor view.
    pub fn view(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| p.view)
    }
    /// After a rendered frame: hands the grabbed frame to the job (`crop` already physical).
    pub fn grabbed(&mut self, frame: Result<Frame, String>, crop: Option<[u32; 4]>) {
        let Some(request) = self.pending.take() else {
            return;
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => {
                self.finish(request, Err(error), 0);
                return;
            }
        };
        match spawn(request.path.clone(), frame, crop) {
            Ok(job) => self.job = Some((job, request)),
            Err(e) => self.finish(request, Err(format!("could not start the job: {e}")), 0),
        }
    }
    /// Every frame: the job's outcome or a pending request that no frame picked up.
    pub fn poll(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.asked.elapsed() > PENDING_LIMIT)
        {
            let request = self.pending.take().expect("checked above");
            self.finish(
                request,
                Err("no frame was rendered within 10 s (window minimized?)".into()),
                0,
            );
        }
        let Some((job, _)) = &mut self.job else {
            return;
        };
        let Some(written) = poll(job) else {
            return;
        };
        let (_, request) = self.job.take().expect("checked above");
        let ms = request.asked.elapsed().as_millis() as u64;
        self.finish(request, written, ms);
    }
    fn finish(&mut self, mut request: Request, written: Written, ms: u64) {
        let value = match written {
            Ok((path, width, height)) => json!({
                "ok":true,"path":path,"width":width,"height":height,
                "region":if request.view {"view"} else {"window"},"ms":ms
            }),
            Err(error) => {
                // Same shape as a refused request (`error`, `diagnostics`), plus the path.
                let mut value = axiom_core::error_response(&anyhow::anyhow!(error));
                value["path"] = json!(request.path);
                value
            }
        };
        if let Some(reply) = request.reply.take() {
            let _ = reply.try_send(value.clone());
        }
        self.last = Some(value);
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        if let Some((job, _)) = &mut self.job {
            job.shutdown(JOIN_LIMIT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_must_be_png_without_parent_components() {
        assert!(validate_path("shots/a.png").unwrap().is_absolute());
        assert!(validate_path("C:/tmp/A.PNG").is_ok() || cfg!(not(windows)));
        for bad in [
            "",
            "a.jpg",
            "../a.png",
            "x/../a.png",
            ".png",
            "noext",
            "NUL.png",
            "com1.png",
        ] {
            assert!(validate_path(bad).is_err(), "{bad}");
        }
        if cfg!(windows) {
            for bad in [
                "C:a.png",
                "\\a.png",
                "\\\\host\\share\\x.png",
                "\\\\.\\pipe\\x.png",
                "\\\\?\\C:\\x.png",
            ] {
                assert!(validate_path(bad).is_err(), "{bad}");
            }
        }
    }

    #[test]
    fn crops_scale_and_clamp_to_the_frame() {
        let r = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));
        assert_eq!(physical(r, 1.5, 1000, 1000), [15, 30, 150, 75]);
        let r = egui::Rect::from_min_max(egui::pos2(-5.0, 90.0), egui::pos2(500.0, 500.0));
        assert_eq!(physical(r, 1.0, 100, 100), [0, 90, 100, 10]);
        let r = egui::Rect::from_min_max(egui::pos2(200.0, 200.0), egui::pos2(300.0, 300.0));
        assert_eq!(physical(r, 1.0, 100, 100), [99, 99, 1, 1]);
    }

    #[test]
    fn writes_the_cropped_rows_and_converts_bgra() {
        let dir = std::env::temp_dir().join(format!("axiom-shot-{}", std::process::id()));
        let path = dir.join("crop.png");
        // 4×2 frame: pixel (x, y) = [x, y, 7, 255].
        let pixels: Vec<u8> = (0..2u8)
            .flat_map(|y| (0..4u8).flat_map(move |x| [x, y, 7, 255]))
            .collect();
        let progress = Progress::default();
        let read = || {
            let decoder = png::Decoder::new(std::fs::File::open(&path).unwrap());
            let mut reader = decoder.read_info().unwrap();
            let mut buf = vec![0; reader.output_buffer_size()];
            reader.next_frame(&mut buf).unwrap();
            buf
        };
        assert_eq!(
            write_crop(&path, 16, &pixels, false, [1, 1, 2, 1], &progress),
            Ok((2, 1))
        );
        assert_eq!(read(), [1, 1, 7, 255, 2, 1, 7, 255]);
        write_crop(&path, 16, &pixels, true, [1, 1, 2, 1], &progress).unwrap();
        assert_eq!(read(), [7, 1, 1, 255, 7, 1, 2, 255]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn one_capture_at_a_time_and_the_reply_carries_the_size() {
        let dir = std::env::temp_dir().join(format!("axiom-loopback-{}", std::process::id()));
        let path = dir.join("shot.png");
        let mut shots = Loopback::default();
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        let request = |reply| Request {
            path: path.clone(),
            view: false,
            asked: Instant::now(),
            reply,
        };
        shots.request(request(Some(reply))).unwrap();
        assert!(shots.request(request(None)).is_err(), "one at a time");
        assert!(shots.wants_grab());
        let frame = Frame {
            width: 3,
            height: 2,
            bgra: true,
            stride: 12,
            data: FrameData::Pixels(vec![9; 3 * 2 * 4]),
        };
        shots.grabbed(Ok(frame), None);
        let started = Instant::now();
        while shots.busy() {
            assert!(started.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
            shots.poll();
        }
        let value = answer.recv().unwrap();
        assert_eq!(value["ok"], true, "{value}");
        assert_eq!(
            (value["width"].as_u64(), value["height"].as_u64()),
            (Some(3), Some(2))
        );
        assert!(path.exists());
        // A request no frame picks up fails after the limit.
        shots.request(request(None)).unwrap();
        shots.pending.as_mut().unwrap().asked =
            Instant::now() - PENDING_LIMIT - Duration::from_secs(1);
        shots.poll();
        assert!(!shots.busy());
        assert_eq!(shots.last.as_ref().unwrap()["ok"], false);
        let _ = std::fs::remove_dir_all(dir);
    }
}
