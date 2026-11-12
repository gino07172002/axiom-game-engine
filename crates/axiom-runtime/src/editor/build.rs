//! File > Build Game… (packaging P4, docs/plan-packaging.zh-TW.md).
//!
//! Runs the same `packaging::package` as `axiom package` on a bounded background job: countable
//! progress (files copied / total), cancellable, a watchdog deadline, duration logged, joined
//! with a timeout on shutdown. The build ships the `--pack` directory on disk; edits that live
//! only in the authoring database are not part of it (plan decision #1). Not a content edit: no
//! revision, no ACL.
use super::theme::{ACCENT, MUTED, SUCCESS, WARNING};
use super::Editor;
use crate::jobs::{Job, Outcome, STOP_GRACE};
use crate::packaging::{self, PackageOptions, Progress};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{atomic::Ordering::Relaxed, Arc};
use std::time::Duration;

/// Watchdog: a debug build of a large pack copies a few hundred MB; ten minutes is far beyond it.
pub(crate) const BUILD_TIMEOUT: Duration = Duration::from_secs(600);
/// How long shutdown waits for a cancelled build to remove its `*.partial` folder.
const JOIN_LIMIT: Duration = Duration::from_secs(5);
pub(crate) const NO_PACK: &str = "the editor was not opened with --pack: Build Game ships a content package directory, not the authoring database";

struct Dialog {
    out: String,
    title: String,
    force: bool,
}

struct Running {
    job: Job<Result<Value>>,
    /// The packager's own counters and cancel flag (the job's are mirrored from it).
    progress: Arc<Progress>,
    out: PathBuf,
}

#[derive(Default)]
pub(crate) struct BuildGame {
    /// `--pack` directory; `None` disables the command.
    pub pack: Option<PathBuf>,
    dialog: Option<Dialog>,
    running: Option<Running>,
    /// Summary of the last finished build (`observe editor.build.last`).
    last: Option<Value>,
    /// Full report of the last successful build (file list with SHA-256).
    pub(crate) last_report: Option<Value>,
    /// Output folder of the last successful build (Open folder).
    built: Option<PathBuf>,
    /// Tests: copy this file instead of the running executable.
    pub(crate) player: Option<PathBuf>,
    #[cfg(test)]
    pub(crate) cancel_before_start: bool,
}

impl Drop for BuildGame {
    fn drop(&mut self) {
        if let Some(run) = &mut self.running {
            run.progress.cancel.store(true, Relaxed);
            if !run.job.shutdown(JOIN_LIMIT) {
                eprintln!(
                    "build_game: worker did not stop within {} s on shutdown; detached ({}.partial may remain)",
                    JOIN_LIMIT.as_secs(),
                    run.out.display()
                );
            }
        }
    }
}

fn default_out(title: &str) -> String {
    let name = packaging::sanitize(title);
    format!("dist/{}", if name.is_empty() { "game" } else { &name })
}

fn open_folder(path: &Path) -> Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .with_context(|| format!("open {}", path.display()))?;
    Ok(())
}

impl Editor {
    pub fn set_build_pack(&mut self, pack: Option<PathBuf>) {
        self.build.pack = pack;
    }

    pub(super) fn build_blocked(&self) -> Option<&'static str> {
        self.build.pack.is_none().then_some(NO_PACK)
    }

    #[cfg(test)]
    pub(super) fn build_running(&self) -> bool {
        self.build.running.is_some()
    }

    /// `file.build_game`: no argument / `open` = the dialog; `{"out","title","force"}` (all
    /// optional) = start a build; `start` = start with the dialog's values; `cancel`.
    pub(super) fn build_command(&mut self, arg: Option<&str>) -> Result<()> {
        let arg = arg.map(str::trim).filter(|a| !a.is_empty());
        match arg {
            None | Some("open") => self.open_build_dialog(),
            Some("cancel") => {
                anyhow::ensure!(self.build.running.is_some(), "no build is running");
                self.cancel_build();
            }
            Some("start") => {
                if self.build.dialog.is_none() {
                    self.open_build_dialog();
                }
                let d = self.build.dialog.as_ref().unwrap();
                let (out, title, force) = (d.out.clone(), d.title.clone(), d.force);
                self.start_build(&out, &title, force)?;
            }
            Some(text) => {
                let spec: Value = serde_json::from_str(text).context(
                    "file.build_game takes open | start | cancel | {\"out\":DIR,\"title\":T,\"force\":BOOL}",
                )?;
                anyhow::ensure!(spec.is_object(), "file.build_game JSON must be an object");
                let title = spec["title"]
                    .as_str()
                    .map_or_else(|| self.world.title.clone(), str::to_owned);
                let out = spec["out"]
                    .as_str()
                    .map_or_else(|| default_out(&title), str::to_owned);
                let force = spec["force"].as_bool().unwrap_or(false);
                self.start_build(&out, &title, force)?;
            }
        }
        Ok(())
    }

    fn open_build_dialog(&mut self) {
        if self.build.dialog.is_none() {
            let title = self.world.title.clone();
            self.build.dialog = Some(Dialog {
                out: default_out(&title),
                title,
                force: false,
            });
        }
    }

    pub(super) fn start_build(&mut self, out: &str, title: &str, force: bool) -> Result<()> {
        let started = std::time::Instant::now();
        anyhow::ensure!(self.build.running.is_none(), "a build is already running");
        let pack = self.build.pack.clone().context(NO_PACK)?;
        let (out, title) = (out.trim(), title.trim());
        anyhow::ensure!(!out.is_empty(), "output folder must not be empty");
        anyhow::ensure!(!title.is_empty(), "title must not be empty");
        let out = PathBuf::from(out);
        let registry = self.registry.clone();
        let player = self.build.player.clone();
        let progress = Arc::new(Progress::default());
        #[cfg(test)]
        if self.build.cancel_before_start {
            progress.cancel.store(true, Relaxed);
        }
        let shared = progress.clone();
        let mut options = PackageOptions {
            pack,
            out: out.clone(),
            title: Some(title.to_owned()),
            exe_name: None,
            excludes: vec![],
            force,
            player_exe: PathBuf::new(),
        };
        let job = Job::spawn(
            "build_game",
            out.to_string_lossy(),
            BUILD_TIMEOUT,
            move |_| {
                options.player_exe = match player {
                    Some(p) => p,
                    None => packaging::default_player_exe()?,
                };
                packaging::package(&registry, &options, &shared)
            },
        )?;
        eprintln!(
            "build_game: started {} ({:.2} ms on the main thread)",
            out.display(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        self.notice(format!("Building game · {}", out.display()));
        self.build.running = Some(Running { job, progress, out });
        Ok(())
    }

    pub(super) fn cancel_build(&mut self) {
        if let Some(run) = &mut self.build.running {
            run.progress.cancel.store(true, Relaxed);
            run.job.cancel();
            let out = run.out.display().to_string();
            self.notice(format!("Cancelling build · {out}"));
        }
    }

    /// Non-blocking; once per frame from `poll_jobs`.
    pub(super) fn poll_build(&mut self) {
        let Some(run) = &mut self.build.running else {
            return;
        };
        // The job's watchdog stops its own flag; the packager reads its own.
        if run.job.elapsed() >= BUILD_TIMEOUT {
            run.progress.cancel.store(true, Relaxed);
        }
        let polled = std::time::Instant::now();
        let outcome = run.job.poll();
        let poll_ms = polled.elapsed().as_secs_f64() * 1000.0;
        if poll_ms > 4.0 {
            eprintln!("build_game: slow poll {poll_ms:.1} ms on the main thread");
        }
        let Some(outcome) = outcome else {
            return;
        };
        let run = self.build.running.take().unwrap();
        let elapsed_ms = run.job.elapsed().as_millis() as u64;
        let (done, total) = (
            run.progress.done.load(Relaxed),
            run.progress.total.load(Relaxed),
        );
        let out = run.out.display().to_string();
        let (state, error) = match outcome {
            Outcome::Done {
                value: Ok(report), ..
            } => {
                let files = report["files"].as_array().map_or(0, Vec::len);
                let bytes = report["bytes"].as_u64().unwrap_or(0);
                let path = report["out"]
                    .as_str()
                    .map_or_else(|| run.out.clone(), PathBuf::from);
                self.last_job = Some(json!({"kind":"build_game","label":out,"state":"finished","done":done,"total":total,"elapsed_ms":elapsed_ms}));
                // packaging::package checks cancellation before each file and before the player
                // copy; a cancel during the player copy or validation arrives too late.
                let late = run.progress.cancel.load(Relaxed);
                self.build.last = Some(json!({"ok":true,"out":path.to_string_lossy(),"files":files,"bytes":bytes,"elapsed_ms":elapsed_ms,"cancel_too_late":late}));
                eprintln!(
                    "build_game: ok {} · {files} files · {bytes} bytes · {elapsed_ms} ms{}",
                    path.display(),
                    if late { " (cancel arrived after the last checkpoint)" } else { "" }
                );
                self.notice(format!(
                    "Built game{} · {} · {files} files · {:.1} MB · {:.1} s",
                    if late { " (cancel came after the last checkpoint)" } else { "" },
                    path.display(),
                    bytes as f64 / 1048576.0,
                    elapsed_ms as f64 / 1000.0
                ));
                self.build.built = Some(path);
                self.build.last_report = Some(report);
                return;
            }
            Outcome::Done {
                value: Err(e),
                stop,
            } => {
                // The packager's own flag also counts (set by the watchdog mirror or shutdown).
                let flagged = run.progress.cancel.load(Relaxed).then_some(
                    if run.job.elapsed() >= BUILD_TIMEOUT {
                        "timed_out"
                    } else {
                        "cancelled"
                    },
                );
                (
                    stop.map(|s| s.name()).or(flagged).unwrap_or("failed"),
                    format!("{e:#}"),
                )
            }
            Outcome::Abandoned { stop } => (
                "abandoned",
                format!(
                    "build worker did not stop within {} s after {}; detached ({out}.partial may remain)",
                    STOP_GRACE.as_secs(),
                    stop.name()
                ),
            ),
        };
        eprintln!("build_game: {state} {out} after {elapsed_ms} ms: {error}");
        self.last_job = Some(
            json!({"kind":"build_game","label":out,"state":state,"done":done,"total":total,"elapsed_ms":elapsed_ms}),
        );
        self.build.last =
            Some(json!({"ok":false,"state":state,"out":out,"error":error,"elapsed_ms":elapsed_ms}));
        if state == "cancelled" {
            self.notice(format!("Build cancelled · {out}"));
        } else {
            self.set_error(format!("Build {state} · {error}"));
        }
    }

    /// The running build for `observe editor.jobs` (counts from the packager).
    pub(super) fn build_job_observation(&self) -> Option<Value> {
        self.build.running.as_ref().map(|run| {
            let mut v = run.job.observation();
            v["done"] = json!(run.progress.done.load(Relaxed));
            v["total"] = json!(run.progress.total.load(Relaxed));
            v
        })
    }

    pub(super) fn build_observation(&self) -> Value {
        json!({
            "available":self.build.pack.is_some(),
            "pack":self.build.pack.as_ref().map(|p| p.to_string_lossy()),
            "dialog":self.build.dialog.as_ref().map(|d| json!({"out":d.out,"title":d.title,"force":d.force})),
            "running":self.build.running.is_some(),
            "last":self.build.last,
        })
    }

    /// Status bar: progress while running, Open folder after a successful build.
    pub(super) fn build_status(&mut self, ui: &mut egui::Ui) {
        if let Some(run) = &self.build.running {
            let (done, total) = (
                run.progress.done.load(Relaxed),
                run.progress.total.load(Relaxed),
            );
            let r = ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "build {done}/{total} · {:.1} s",
                            run.job.elapsed().as_secs_f32()
                        ))
                        .size(11.5)
                        .color(WARNING),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_text(format!(
                    "Building {} in the background (click: Build Game dialog with Cancel)",
                    run.out.display()
                ));
            self.rects.insert("status:build".into(), r.rect);
            if r.clicked() {
                self.open_build_dialog();
            }
            ui.add_space(10.0);
        } else if let Some(path) = &self.build.built {
            let r = ui
                .add(
                    egui::Label::new(
                        egui::RichText::new("Open built game")
                            .size(11.5)
                            .color(ACCENT),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_text(format!("Open {} in Explorer", path.display()));
            self.rects.insert("status:build_open".into(), r.rect);
            if r.clicked() {
                let path = path.clone();
                if let Err(e) = open_folder(&path) {
                    self.set_error(format!("{e:#}"));
                }
            }
            ui.add_space(10.0);
        }
    }

    /// The Build Game dialog (allocates only while open).
    pub(super) fn build_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.build.dialog.take() else {
            return;
        };
        let pack = self
            .build
            .pack
            .as_ref()
            .map_or_else(String::new, |p| p.display().to_string());
        let running = self.build.running.as_ref().map(|run| {
            (
                run.progress.done.load(Relaxed),
                run.progress.total.load(Relaxed),
                run.job.elapsed().as_secs_f32(),
                run.progress.cancel.load(Relaxed),
            )
        });
        let (mut build, mut cancel, mut close, mut open) = (false, false, false, false);
        let mut rects = vec![];
        egui::Window::new("Build Game")
            .id(egui::Id::new("build_game"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 90.0])
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(egui::RichText::new(format!("Pack: {pack}")).color(MUTED));
                ui.label(
                    egui::RichText::new(
                        "Builds from the pack folder on disk. Edits that exist only in the editor database are not included.",
                    )
                    .small()
                    .color(WARNING),
                );
                ui.add_space(6.0);
                ui.add_enabled_ui(running.is_none(), |ui| {
                    egui::Grid::new("build_game_grid")
                        .num_columns(2)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Output folder");
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut dialog.out).desired_width(300.0),
                            );
                            rects.push(("build:out", r.rect));
                            ui.end_row();
                            ui.label("Title");
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut dialog.title)
                                    .desired_width(300.0),
                            );
                            rects.push(("build:title", r.rect));
                            ui.end_row();
                            ui.label("");
                            let r = ui
                                .checkbox(&mut dialog.force, "Replace an existing build")
                                .on_hover_text("Only a folder that holds game.json is replaced");
                            rects.push(("build:force", r.rect));
                            ui.end_row();
                        });
                });
                ui.label(
                    egui::RichText::new(
                        "Relative paths are under the editor's working folder (dist/ is git-ignored).",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.add_space(6.0);
                if let Some((done, total, seconds, stopping)) = running {
                    let fraction = if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .text(format!("{done}/{total} files · {seconds:.1} s")),
                    );
                    ui.horizontal(|ui| {
                        let r = ui.add_enabled(!stopping, egui::Button::new("Cancel"));
                        rects.push(("build:cancel", r.rect));
                        cancel = r.clicked();
                        if stopping {
                            ui.label(egui::RichText::new("cancelling…").color(WARNING));
                        }
                    });
                } else {
                    if let Some(last) = &self.build.last {
                        if last["ok"] == true {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Built {} · {} files · {:.1} MB · {:.1} s",
                                    last["out"].as_str().unwrap_or(""),
                                    last["files"],
                                    last["bytes"].as_f64().unwrap_or(0.0) / 1048576.0,
                                    last["elapsed_ms"].as_f64().unwrap_or(0.0) / 1000.0
                                ))
                                .color(SUCCESS),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Build {}: {}",
                                    last["state"].as_str().unwrap_or("failed"),
                                    last["error"].as_str().unwrap_or("")
                                ))
                                .color(WARNING),
                            );
                        }
                        ui.add_space(4.0);
                    }
                    ui.horizontal(|ui| {
                        let ok = !dialog.out.trim().is_empty() && !dialog.title.trim().is_empty();
                        let r = ui.add_enabled(ok, egui::Button::new("Build"));
                        rects.push(("build:build", r.rect));
                        build = r.clicked();
                        let r = ui.button("Close");
                        rects.push(("build:close", r.rect));
                        close = r.clicked();
                        if self.build.built.is_some()
                            && self.build.last.as_ref().is_some_and(|l| l["ok"] == true)
                        {
                            let r = ui.button("Open folder");
                            rects.push(("build:open", r.rect));
                            open = r.clicked();
                        }
                    });
                }
            });
        for (key, rect) in rects {
            self.rects.insert(key.into(), rect);
        }
        let (out, title, force) = (dialog.out.clone(), dialog.title.clone(), dialog.force);
        if !close {
            self.build.dialog = Some(dialog);
        }
        if cancel {
            self.cancel_build();
        }
        if build {
            if let Err(e) = self.start_build(&out, &title, force) {
                self.set_error(format!("{e:#}"));
            }
        }
        if open {
            if let Some(path) = self.build.built.clone() {
                if let Err(e) = open_folder(&path) {
                    self.set_error(format!("{e:#}"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldRuntime;
    use std::time::Instant;

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }

    fn hashes(report: &Value) -> Vec<(String, String)> {
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["path"].to_string(), f["sha256"].to_string()))
            .collect()
    }

    struct Fixture {
        base: PathBuf,
        pack: PathBuf,
        player: PathBuf,
        editor: Editor,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("axiom-build-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pack = base.join("pack");
        copy_dir(
            Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../content/two-cameras"
            )),
            &pack,
        );
        let player = base.join("player.exe");
        std::fs::write(&player, b"stand-in player").unwrap();
        let registry = crate::installed_registry().authoring;
        let world = axiom_core::ContentPackage::load(&pack, &registry)
            .unwrap()
            .world;
        let mut editor = Editor::new(world, 1, None);
        editor.set_component_hints(&registry);
        editor.set_build_pack(Some(pack.clone()));
        editor.build.player = Some(player.clone());
        Fixture {
            base,
            pack,
            player,
            editor,
        }
    }

    fn finish(editor: &mut Editor) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while editor.build_running() {
            assert!(Instant::now() < deadline, "build never finished");
            editor.poll_jobs();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn editor_job_builds_the_same_files_as_a_direct_package_call() {
        let mut f = fixture("parity");
        let job_out = f.base.join("out/job");
        let arg = json!({"out":job_out,"title":"Two Cams"}).to_string();
        f.editor
            .run_command_with("file.build_game", Some(&arg))
            .unwrap();
        // Running: visible in observe with the packager's counts, and a second start refused.
        let jobs = f.editor.observation()["jobs"].clone();
        assert!(
            jobs.as_array()
                .unwrap()
                .iter()
                .any(|j| j["kind"] == "build_game"),
            "{jobs}"
        );
        assert!(f
            .editor
            .run_command_with("file.build_game", Some(&arg))
            .is_err());
        finish(&mut f.editor);
        let via_job = f.editor.build.last_report.clone().expect("build succeeded");
        let last = f.editor.observation()["build"]["last"].clone();
        assert_eq!(last["ok"], true, "{last}");
        assert_eq!(f.editor.observation()["last_job"]["kind"], "build_game");

        let direct = packaging::package(
            &crate::installed_registry().authoring,
            &PackageOptions {
                pack: f.pack.clone(),
                out: f.base.join("out/direct"),
                title: Some("Two Cams".into()),
                exe_name: None,
                excludes: vec![],
                force: false,
                player_exe: f.player.clone(),
            },
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(hashes(&via_job), hashes(&direct));
        assert!(!hashes(&direct).is_empty());
        assert!(job_out.join("Two Cams.exe").is_file());
    }

    #[test]
    fn cancelled_build_leaves_no_output_and_no_partial_folder() {
        let mut f = fixture("cancel");
        f.editor.build.cancel_before_start = true;
        let out = f.base.join("out/cancelled");
        let arg = json!({ "out": out }).to_string();
        f.editor
            .run_command_with("file.build_game", Some(&arg))
            .unwrap();
        finish(&mut f.editor);
        let last = f.editor.observation()["build"]["last"].clone();
        assert_eq!(last["ok"], false, "{last}");
        assert!(
            last["error"].as_str().unwrap().contains("cancelled"),
            "{last}"
        );
        assert!(!out.exists() && !f.base.join("out/cancelled.partial").exists());
        assert!(!f.editor.error, "a cancel is a notice, not an error");
        // An existing output without force is a recoverable error, shown in the status line.
        f.editor.build.cancel_before_start = false;
        std::fs::create_dir_all(&out).unwrap();
        f.editor
            .run_command_with("file.build_game", Some(&arg))
            .unwrap();
        finish(&mut f.editor);
        assert!(
            f.editor.error && f.editor.status.contains("--force"),
            "{}",
            f.editor.status
        );
    }

    #[test]
    fn without_a_pack_the_command_is_unavailable_and_says_why() {
        let mut f = fixture("nopack");
        f.editor.set_build_pack(None);
        assert_eq!(f.editor.command_blocked("file.build_game"), Some(NO_PACK));
        let Err(error) = f.editor.run_command("file.build_game") else {
            panic!("file.build_game ran without a pack");
        };
        assert!(format!("{error:#}").contains("--pack"), "{error:#}");
        assert_eq!(f.editor.observation()["build"]["available"], false);
    }

    #[test]
    fn dialog_build_button_starts_the_job_and_reports_the_result() {
        let mut f = fixture("dialog");
        let runtime = WorldRuntime::new(
            f.editor.world.as_ref().clone(),
            1,
            crate::installed_registry(),
        )
        .unwrap();
        f.editor.run_command("file.build_game").unwrap();
        let dialog = f.editor.observation()["build"]["dialog"].clone();
        assert_eq!(dialog["title"], f.editor.world.title.as_str());
        assert!(
            dialog["out"].as_str().unwrap().starts_with("dist/"),
            "{dialog}"
        );
        // Point the dialog at the temp folder (typing is ordinary egui text input).
        f.editor.build.dialog.as_mut().unwrap().out =
            f.base.join("out/dialog").to_string_lossy().into_owned();
        let ctx = egui::Context::default();
        let frame = |editor: &mut Editor, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1366.0, 768.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    editor.draw(ctx, &runtime);
                },
            );
        };
        frame(&mut f.editor, vec![]);
        frame(&mut f.editor, vec![]);
        let at = f.editor.rects["build:build"].center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &mut f.editor,
            vec![egui::Event::PointerMoved(at), press(true)],
        );
        frame(&mut f.editor, vec![press(false)]);
        assert!(f.editor.build_running() || f.editor.build.last.is_some());
        finish(&mut f.editor);
        frame(&mut f.editor, vec![]);
        assert_eq!(f.editor.build.last.as_ref().unwrap()["ok"], true);
        assert!(f.editor.rects.contains_key("build:open"));
        assert!(f.editor.rects.contains_key("status:build_open"));
        assert!(f.base.join("out/dialog/game.json").is_file());
    }
}
