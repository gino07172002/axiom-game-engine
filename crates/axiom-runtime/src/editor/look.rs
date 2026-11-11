//! Scene view display mode (Lit / Unlit) and the Scene view screenshot (workstream E).
//!
//! Both are editor view state, never content: `view.display_mode lit|unlit` only changes how
//! the renderer lights the editor's own view (`Renderer::unlit_view`), and `view.screenshot
//! [NAME]` reads the next presented frame back (`Renderer::grab`) and crops / encodes / writes
//! the PNG in a bounded background job, so the main thread only copies the frame once.
use super::Editor;
use crate::jobs::{Job, Outcome};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// The editor orbit camera's render view id (`viewport.rs`).
const SCENE_VIEW: &str = "editor.orbit";
const JOB_TIMEOUT: Duration = Duration::from_secs(20);
const JOIN_LIMIT: Duration = Duration::from_millis(500);
const PENDING_LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DisplayMode {
    #[default]
    Lit,
    Unlit,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 2] = [DisplayMode::Lit, DisplayMode::Unlit];
    pub fn name(self) -> &'static str {
        match self {
            DisplayMode::Lit => "lit",
            DisplayMode::Unlit => "unlit",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            DisplayMode::Lit => "Lit",
            DisplayMode::Unlit => "Unlit",
        }
    }
    pub fn tip(self) -> &'static str {
        match self {
            DisplayMode::Lit => "Authored lighting: suns, shadows, local lights, ambient, fog",
            DisplayMode::Unlit => {
                "Flat base colour: one uniform white ambient, no lights, shadows or fog (editor view only; Game tab and Play are lit)"
            }
        }
    }
    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == text)
    }
}

/// A screenshot waiting for its frame: file and the crop in points (None = whole window).
struct Pending {
    path: PathBuf,
    region: Option<egui::Rect>,
    asked: Instant,
}

type Written = Result<(PathBuf, u32, u32), String>;

#[derive(Default)]
pub(crate) struct Look {
    pub display: DisplayMode,
    pending: Option<Pending>,
    job: Option<Job<Written>>,
    /// Result of the last screenshot (`observe editor.look.screenshot`).
    last: Option<Value>,
    /// Folder for the PNGs (tests); default `artifacts/captures` under the working directory.
    pub dir: Option<PathBuf>,
}

impl Drop for Look {
    fn drop(&mut self) {
        if let Some(job) = &mut self.job {
            job.shutdown(JOIN_LIMIT);
        }
    }
}

impl Editor {
    /// `view.display_mode lit|unlit` (no argument: toggles) and `view.screenshot [NAME]`.
    pub(super) fn look_command(&mut self, id: &str, arg: Option<&str>) -> anyhow::Result<()> {
        let arg = arg.map(str::trim).filter(|a| !a.is_empty());
        match id {
            "view.display_mode" => {
                let mode = match arg {
                    None => match self.look.display {
                        DisplayMode::Lit => DisplayMode::Unlit,
                        DisplayMode::Unlit => DisplayMode::Lit,
                    },
                    Some(text) => DisplayMode::parse(&text.to_ascii_lowercase())
                        .ok_or_else(|| anyhow::anyhow!("view.display_mode takes lit|unlit"))?,
                };
                self.look.display = mode;
                self.notice(format!("Scene view: {}", mode.title()));
            }
            "view.screenshot" => {
                anyhow::ensure!(
                    self.look.pending.is_none() && self.look.job.is_none(),
                    "a screenshot is already being written"
                );
                let name = match arg {
                    Some(name) => {
                        anyhow::ensure!(
                            name.len() <= 64
                                && name
                                    .chars()
                                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                                && !name.contains(".."),
                            "screenshot name must be 1..64 of [A-Za-z0-9-_.]"
                        );
                        let stem = name.trim_end_matches(".png");
                        anyhow::ensure!(
                            !stem.is_empty() && stem != ".",
                            "screenshot name must be 1..64 of [A-Za-z0-9-_.]"
                        );
                        stem.to_owned()
                    }
                    None => format!(
                        "scene-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_millis())
                    ),
                };
                let dir = match &self.look.dir {
                    Some(dir) => dir.clone(),
                    None => std::env::current_dir()?.join("artifacts").join("captures"),
                };
                self.look.pending = Some(Pending {
                    path: dir.join(format!("{name}.png")),
                    // The Scene view when it is shown, else the whole window.
                    region: (self.scene_visible && self.viewport.is_positive())
                        .then_some(self.viewport),
                    asked: Instant::now(),
                });
                self.notice("Screenshot: capturing the next frame…");
            }
            _ => anyhow::bail!("unknown view command {id}"),
        }
        Ok(())
    }

    /// Host, before rendering: the view to draw unlit.
    pub(crate) fn unlit_view(&self) -> Option<&'static str> {
        (self.look.display == DisplayMode::Unlit && !self.previewing).then_some(SCENE_VIEW)
    }

    /// Host, before rendering: read this frame back for a screenshot.
    pub(crate) fn wants_grab(&self) -> bool {
        self.look.pending.is_some()
    }

    /// Host, after a rendered frame that was asked to grab: the frame (physical pixels) or
    /// `None` when the surface cannot be read back. Starts the encode job.
    pub(crate) fn grabbed(
        &mut self,
        frame: Option<(u32, u32, Vec<u8>, bool)>,
        pixels_per_point: f32,
    ) {
        let Some(pending) = self.look.pending.take() else {
            return;
        };
        let Some((width, height, pixels, bgra)) = frame else {
            self.screenshot_done(Err(
                "this window surface does not support read-back (COPY_SRC)".into(),
            ));
            return;
        };
        let crop = pending
            .region
            .map(|r| physical(r, pixels_per_point, width, height))
            .unwrap_or([0, 0, width, height]);
        let path = pending.path;
        let label = path.display().to_string();
        let job = Job::spawn("screenshot", label, JOB_TIMEOUT, move |progress| {
            progress.set_total(1);
            let started = Instant::now();
            let written = write_crop(&path, width, &pixels, bgra, crop, progress);
            progress.set_done(1);
            written.map(|(w, h)| {
                eprintln!(
                    "screenshot: {} ({w}x{h}) in {:.0} ms",
                    path.display(),
                    started.elapsed().as_secs_f64() * 1000.0
                );
                (path, w, h)
            })
        });
        match job {
            Ok(job) => self.look.job = Some(job),
            Err(e) => self.screenshot_done(Err(format!("could not start the job: {e}"))),
        }
    }

    /// Every drawn frame: the screenshot job's outcome; a request no rendered frame picked up
    /// within `PENDING_LIMIT` (minimized window) fails instead of blocking the command.
    pub(super) fn poll_look(&mut self) {
        if self
            .look
            .pending
            .as_ref()
            .is_some_and(|p| p.asked.elapsed() > PENDING_LIMIT)
        {
            self.look.pending = None;
            self.screenshot_done(Err(
                "no frame was rendered within 10 s (window minimized?)".into()
            ));
        }
        let Some(job) = &mut self.look.job else {
            return;
        };
        let ms = job.elapsed().as_millis() as u64;
        let outcome = match job.poll() {
            None => return,
            Some(Outcome::Done { value, stop: None }) => value,
            Some(Outcome::Done {
                stop: Some(stop), ..
            })
            | Some(Outcome::Abandoned { stop }) => Err(format!("screenshot job {}", stop.name())),
        };
        self.look.job = None;
        self.last_job = Some(json!({"kind":"screenshot","ms":ms,"ok":outcome.is_ok()}));
        self.screenshot_done(outcome);
    }

    fn screenshot_done(&mut self, outcome: Written) {
        match outcome {
            Ok((path, w, h)) => {
                self.notice(format!("Saved screenshot {} ({w}×{h})", path.display()));
                self.look.last = Some(json!({"ok":true,"path":path,"width":w,"height":h}));
            }
            Err(e) => {
                self.set_error(format!("Screenshot failed: {e}"));
                self.look.last = Some(json!({"ok":false,"error":e}));
            }
        }
    }

    /// Scene toolbar: screenshot button and the Lit / Unlit pill (both registry commands).
    pub(super) fn look_toolbar(&mut self, ui: &mut egui::Ui, run: &mut Option<&'static str>) {
        use super::{icons::Icon, theme};
        let icons = self.icons;
        let blocked = self.command_blocked("view.screenshot");
        let tip = blocked.map_or_else(
            || {
                "Screenshot of the Scene view → artifacts/captures/scene-….png (view.screenshot)"
                    .to_owned()
            },
            |why| format!("Screenshot · {why}"),
        );
        let r = theme::pill_group(ui, |ui| {
            ui.add_enabled_ui(blocked.is_none(), |ui| {
                theme::icon_button_sized(ui, icons, Icon::Screenshot, &tip, false, 28.0)
            })
            .inner
        })
        .inner;
        self.rects.insert("scene:view.screenshot".into(), r.rect);
        if r.clicked() {
            *run = Some("view.screenshot");
        }
        let current = self.look.display;
        let pill = theme::pill(ui, |ui| {
            ui.label(current.title());
            icons.show(ui, Icon::ChevronDown, 14.0, theme::MUTED);
        })
        .on_hover_text(format!("Display mode: {}", current.tip()));
        self.rects
            .insert("scene:view.display_mode".into(), pill.rect);
        let id = ui.make_persistent_id("scene_display_mode");
        let mut picked = None;
        theme::dropdown(ui, id, &pill, |ui| {
            for mode in DisplayMode::ALL {
                let r = ui
                    .add(egui::Button::new(mode.title()).selected(mode == current))
                    .on_hover_text(mode.tip());
                self.rects
                    .insert(format!("scene:display:{}", mode.name()), r.rect);
                if r.clicked() {
                    picked = Some(mode);
                    theme::close_dropdown(ui, id);
                }
            }
            ui.label(
                egui::RichText::new("No wireframe: see docs/scene-editor (外觀)")
                    .small()
                    .color(theme::MUTED),
            );
        });
        if let Some(mode) = picked.filter(|m| *m != current) {
            if let Err(e) = self.run_command_with("view.display_mode", Some(mode.name())) {
                self.set_error(format!("{e:#}"));
            }
        }
    }

    pub(super) fn look_observation(&self) -> Value {
        json!({
            "display_mode":self.look.display.name(),
            "assets":{
                "folder":self.asset_folder_path(),
                "selected":self.assets.selected.as_ref().map(|(k, id)| format!("{}:{id}", k.name())),
            },
            "screenshot":{
                "pending":self.look.pending.is_some() || self.look.job.is_some(),
                "last":self.look.last,
            },
        })
    }

    /// The running screenshot job for `observe editor.jobs`.
    pub(super) fn screenshot_job_observation(&self) -> Option<Value> {
        self.look.job.as_ref().map(|j| j.observation())
    }

    pub(super) fn screenshot_running(&self) -> bool {
        self.look.pending.is_some() || self.look.job.is_some()
    }
}

/// `rect` (points) → `[x, y, w, h]` physical pixels inside a `width` × `height` frame (at
/// least one pixel).
fn physical(rect: egui::Rect, scale: f32, width: u32, height: u32) -> [u32; 4] {
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

/// Copies the crop out of the RGBA rows and writes it as PNG (worker thread).
fn write_crop(
    path: &Path,
    width: u32,
    pixels: &[u8],
    bgra: bool,
    [x, y, w, h]: [u32; 4],
    progress: &crate::jobs::Progress,
) -> Result<(u32, u32), String> {
    let stride = width as usize * 4;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_scale_and_clamp_to_the_frame() {
        let r = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));
        assert_eq!(physical(r, 1.5, 1000, 1000), [15, 30, 150, 75]);
        // Past the frame edge: clamped, never empty.
        let r = egui::Rect::from_min_max(egui::pos2(-5.0, 90.0), egui::pos2(500.0, 500.0));
        assert_eq!(physical(r, 1.0, 100, 100), [0, 90, 100, 10]);
        let r = egui::Rect::from_min_max(egui::pos2(200.0, 200.0), egui::pos2(300.0, 300.0));
        assert_eq!(physical(r, 1.0, 100, 100), [99, 99, 1, 1]);
    }

    fn lab() -> axiom_core::World {
        serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
    }

    #[test]
    fn display_mode_is_view_state_and_lights_only_the_scene_view() {
        let mut e = Editor::new(lab(), 1, None);
        assert_eq!(e.unlit_view(), None);
        assert!(e
            .run_command_with("view.display_mode", Some("unlit"))
            .unwrap()
            .is_empty());
        assert_eq!(e.unlit_view(), Some(SCENE_VIEW));
        assert_eq!(e.observation()["look"]["display_mode"], "unlit");
        // No argument toggles; a bad one is an error, not a silent no-op.
        e.run_command("view.display_mode").unwrap();
        assert_eq!(e.look.display, DisplayMode::Lit);
        assert!(e
            .run_command_with("view.display_mode", Some("wireframe"))
            .is_err());
        // Play shows the game lit.
        e.run_command_with("view.display_mode", Some("Unlit"))
            .unwrap();
        e.previewing = true;
        assert_eq!(e.unlit_view(), None);
        assert_eq!(*e.world, lab(), "view state only");
    }

    #[test]
    fn a_screenshot_crops_the_scene_view_in_a_background_job() {
        let mut e = Editor::new(lab(), 1, None);
        e.scene_visible = true;
        let dir = std::env::temp_dir().join(format!("axiom-look-shot-{}", std::process::id()));
        e.look.dir = Some(dir.clone());
        e.viewport = egui::Rect::from_min_max(egui::pos2(2.0, 1.0), egui::pos2(4.0, 3.0));
        let name = format!("we-look-test-{}", std::process::id());
        assert!(e
            .run_command_with("view.screenshot", Some(&name))
            .unwrap()
            .is_empty());
        assert!(e.wants_grab());
        // A second request while one is pending is refused (and the command is greyed out).
        assert!(e.command_blocked("view.screenshot").is_some());
        assert!(e
            .run_command_with("view.screenshot", Some("x/../y"))
            .is_err());
        // The host hands over a 6×4 frame at 1.5 pixels per point: crop at (3, 2), 3×2.
        let pixels: Vec<u8> = (0..4u8)
            .flat_map(|y| (0..6u8).flat_map(move |x| [x, y, 0, 255]))
            .collect();
        e.grabbed(Some((6, 4, pixels, false)), 1.5);
        assert!(!e.wants_grab());
        let started = Instant::now();
        while e.look.job.is_some() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "job did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
            e.poll_look();
        }
        let last = &e.observation()["look"]["screenshot"]["last"];
        assert_eq!(last["ok"], true, "{last}");
        assert_eq!(
            (last["width"].as_u64(), last["height"].as_u64()),
            (Some(3), Some(2))
        );
        let path = PathBuf::from(last["path"].as_str().unwrap());
        assert!(path.ends_with(format!("{name}.png")));
        assert!(path.starts_with(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        // Empty names are refused.
        assert!(e.run_command_with("view.screenshot", Some(".png")).is_err());
        assert_eq!(e.last_job.as_ref().unwrap()["kind"], "screenshot");
        // No read-back support: an error, not a hang.
        e.run_command("view.screenshot").unwrap();
        e.grabbed(None, 1.0);
        assert_eq!(e.observation()["look"]["screenshot"]["last"]["ok"], false);
        assert!(!e.screenshot_running());
        // A request no frame picks up fails after the limit instead of blocking the command.
        e.run_command("view.screenshot").unwrap();
        e.look.pending.as_mut().unwrap().asked =
            Instant::now() - PENDING_LIMIT - Duration::from_secs(1);
        e.poll_look();
        assert!(!e.screenshot_running());
        assert_eq!(e.observation()["look"]["screenshot"]["last"]["ok"], false);
    }

    #[test]
    fn writes_the_cropped_rows() {
        let dir = std::env::temp_dir().join(format!("axiom-look-{}", std::process::id()));
        let path = dir.join("crop.png");
        // 4×2 frame: pixel (x, y) = [x, y, 7, 255]; BGRA input comes out RGBA.
        let pixels: Vec<u8> = (0..2u8)
            .flat_map(|y| (0..4u8).flat_map(move |x| [x, y, 7, 255]))
            .collect();
        let progress = crate::jobs::Progress::default();
        assert_eq!(
            write_crop(&path, 4, &pixels, false, [1, 1, 2, 1], &progress),
            Ok((2, 1))
        );
        let read = || {
            let decoder = png::Decoder::new(std::fs::File::open(&path).unwrap());
            let mut reader = decoder.read_info().unwrap();
            let mut buf = vec![0; reader.output_buffer_size()];
            reader.next_frame(&mut buf).unwrap();
            buf
        };
        assert_eq!(read(), [1, 1, 7, 255, 2, 1, 7, 255]);
        write_crop(&path, 4, &pixels, true, [1, 1, 2, 1], &progress).unwrap();
        assert_eq!(read(), [7, 1, 1, 255, 7, 1, 2, 255]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
