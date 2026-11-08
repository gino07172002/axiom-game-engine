use crate::{
    bridge::Bridge,
    editor::{self, Editor},
    render::Renderer,
    ui::{self, Action, UiState},
    world::{ActionMap, Observation, RuntimeRegistry, WorldRuntime, FIXED_DT},
};
use anyhow::{Context, Result};
use axiom_core::{parse_request, Request, Store, World};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Instant,
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

/// Watchdog for an editor scenario job (1,000,000 debug-build ticks take about 13 min).
pub const SCENARIO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20 * 60);

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Drops a replaced runtime on a short-lived thread: freeing thousands of entities is
/// measurable main-thread work and nothing observes it.
fn retire<T: Send + 'static>(value: T) {
    if std::thread::Builder::new()
        .name("axiom-retire".into())
        .spawn(move || drop(value))
        .is_err()
    {
        // Thread creation failed; the runtime was dropped with the closure on this thread.
    }
}

pub struct RunOptions {
    pub mode: RunMode,
    pub load_slot: Option<String>,
    pub replace_world: bool,
    pub db: PathBuf,
    pub port: u16,
    pub smoke_frames: Option<u32>,
    pub screenshot: Option<PathBuf>,
    pub start_play: bool,
}
pub enum RunMode {
    Play,
    Editor { selected: Option<String> },
}
struct App {
    cursor: [f32; 2],
    receipts: axiom_core::retry::SessionReceipts,
    history: axiom_core::observe::ObservationHistory,
    commands: std::sync::mpsc::Receiver<crate::bridge::RuntimeRequest>,
    options: RunOptions,
    store: Store,
    runtime: WorldRuntime,
    editor: Option<Editor>,
    edit_runtime: Option<WorldRuntime>,
    ui: UiState,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    keys: HashSet<KeyCode>,
    mouse_down: bool,
    focused: bool,
    captured: bool,
    last: Instant,
    accumulator: f32,
    poll_elapsed: f32,
    frames: u32,
    /// On-demand capture requested over the bridge; written on the next rendered frame.
    pending_capture: Option<PathBuf>,
    observation: Arc<RwLock<Observation>>,
    _bridge: Option<Bridge>,
    error: Option<anyhow::Error>,
    fps_smooth: f32,
    audio: crate::audio::Output,
    /// File ▸ Exit requested from the editor.
    quit: bool,
    /// Main-thread phases of the last revision reload; reported as `metrics.reload`.
    last_reload: Option<ReloadTimings>,
    /// Reloads whose main-thread total exceeded the 16 ms operation budget.
    slow_reloads: u64,
    reloads: u64,
    /// Watchdog deadline for editor scenario jobs.
    scenario_timeout: std::time::Duration,
    /// Build the next revision's runtime on a worker thread while the last good runtime
    /// keeps running (windowed app and measurements); headless tests stay synchronous.
    background_activation: bool,
    activation: Option<Activation>,
    /// Editor Changes-list diff for an external revision, computed off the main thread.
    changes_job: Option<(i64, crate::jobs::Job<serde_json::Value>)>,
    /// Frames over the 16 ms budget and the phases of the last one (`observe` -> `frame`).
    slow_frames: u64,
    last_slow_frame: Option<serde_json::Value>,
}

/// Wall-clock phases of one main-loop frame; only frames over budget are kept.
struct FramePhases {
    started: Instant,
    last: Instant,
    phases: [(&'static str, f64); 8],
    count: usize,
}
impl FramePhases {
    fn start(now: Instant) -> Self {
        Self {
            started: now,
            last: now,
            phases: [("", 0.0); 8],
            count: 0,
        }
    }
    fn mark(&mut self, name: &'static str) {
        let now = Instant::now();
        if self.count < self.phases.len() {
            self.phases[self.count] = (name, (now - self.last).as_secs_f64() * 1000.0);
            self.count += 1;
        }
        self.last = now;
    }
    /// The phase breakdown when the frame exceeded the 16 ms budget.
    fn finish(self) -> Option<serde_json::Value> {
        let total = ms(self.started);
        (total > 16.0).then(|| {
            let mut phases = serde_json::Map::new();
            for (name, value) in &self.phases[..self.count] {
                phases.insert((*name).into(), serde_json::json!(value));
            }
            serde_json::json!({"total_ms":total,"phases_ms":phases})
        })
    }
}

/// A runtime for `rev` being built off the main thread.
struct Activation {
    rev: i64,
    job: crate::jobs::Job<Result<WorldRuntime>>,
    timings: ReloadTimings,
}

/// Watchdog for one background runtime build.
const ACTIVATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Main-thread wall-clock phases of applying one new authoring revision (milliseconds).
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct ReloadTimings {
    pub rev: i64,
    /// `Store::current`: SQLite row read + JSON decode + IR decode.
    pub read_ms: f64,
    /// Copy of the world handed to the editor.
    pub editor_clone_ms: f64,
    /// `Editor::update` (diff summary, draft reset).
    pub editor_update_ms: f64,
    /// Runtime activation phases; absent when Play preview pins its revision.
    pub reconcile: Option<crate::world::ReconcileTimings>,
    pub activation_failed: bool,
    /// The runtime was built on a worker thread (`build_ms` there, not on this thread).
    pub background: bool,
    /// Worker wall clock of the background build.
    pub build_ms: f64,
    /// Main thread when the revision was noticed: read, editor update, job start.
    pub request_ms: f64,
    /// Main thread when the built runtime was adopted: transfer, GPU prepare, swap.
    pub adopt_ms: f64,
    /// Main-thread total (`request_ms + adopt_ms` for background activations).
    pub total_ms: f64,
}

pub fn run(options: RunOptions, registry: RuntimeRegistry, initial: World) -> Result<()> {
    let mut store = Store::open_with(&options.db, registry.authoring.clone(), initial.clone())?;
    if options.replace_world {
        let revision = store.revision()?;
        store.apply(Some(revision), vec![axiom_core::Edit::replace(initial)])?;
    }
    let mut app = App::new(options, registry, store)?;
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}

/// One measured edit → live sample (milliseconds, main-thread wall clock).
#[derive(Clone, Debug, serde::Serialize)]
pub struct EditLiveSample {
    /// From submitting the editor write until the active runtime runs the new revision.
    pub edit_to_live_ms: f64,
    /// `Store::execute(apply)`: validation, diff, SQLite commit.
    pub commit_ms: f64,
    pub reload: ReloadTimings,
}

/// Headless edit → live measurement through the editor's real write path
/// (`editor::Action::Write` → `Store::execute` → `App::reload_after`). No window or GPU,
/// so `reload.reconcile.prepare_ms` excludes pipeline creation; the GPU path reports the
/// same phases live in `observe` → `metrics.reload`.
pub fn measure_edit_live(
    world: World,
    registry: RuntimeRegistry,
    db: PathBuf,
    editor: bool,
    background: bool,
    batches: Vec<Vec<axiom_core::Edit>>,
) -> Result<Vec<EditLiveSample>> {
    for path in [
        db.clone(),
        db.with_extension("sqlite-wal"),
        db.with_extension("sqlite-shm"),
    ] {
        let _ = std::fs::remove_file(path);
    }
    let store = Store::open_with(&db, registry.authoring.clone(), world)?;
    let mut app = App::new(
        RunOptions {
            mode: if editor {
                RunMode::Editor { selected: None }
            } else {
                RunMode::Play
            },
            load_slot: None,
            replace_world: false,
            db,
            port: 0,
            smoke_frames: None,
            screenshot: None,
            start_play: false,
        },
        registry,
        store,
    )?;
    let mut samples = Vec::with_capacity(batches.len());
    app.background_activation = background;
    for edits in batches {
        let started = Instant::now();
        let expected = app.store.revision()?;
        let request = Request::Apply {
            expected_rev: Some(expected),
            edits,
        };
        let commit = if editor {
            app.editor_action(editor::Action::Write {
                request,
                select: None,
            })?;
            None
        } else {
            let phase = Instant::now();
            app.store.execute(request)?;
            let commit = ms(phase);
            app.reload()?;
            Some(commit)
        };
        let submitted_ms = ms(started);
        // A background build completes on a later frame; wait for it like a frame loop would.
        app.finish_activation()?;
        let edit_to_live_ms = ms(started);
        let reload = app.last_reload.context("edit did not reload the runtime")?;
        anyhow::ensure!(
            app.runtime.rev == expected + 1 && app.runtime.activation_error.is_none(),
            "runtime did not go live at revision {}",
            expected + 1
        );
        samples.push(EditLiveSample {
            edit_to_live_ms,
            commit_ms: commit.unwrap_or(
                submitted_ms
                    - if reload.background {
                        reload.request_ms
                    } else {
                        reload.total_ms
                    },
            ),
            reload,
        });
    }
    Ok(samples)
}

impl App {
    fn new(options: RunOptions, registry: RuntimeRegistry, store: Store) -> Result<Self> {
        let (rev, world) = store.current()?;
        let editor = match &options.mode {
            RunMode::Play => None,
            RunMode::Editor { selected } => {
                let mut editor = Editor::new(world.clone(), rev, selected.clone());
                editor.set_component_hints(&registry.authoring);
                // Offer the repository's scenario files (bounded) in the Scenarios tab.
                let mut files: Vec<String> = std::fs::read_dir("examples")
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.to_string_lossy().ends_with(".scenario.json"))
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .collect();
                files.sort();
                files.truncate(64);
                editor.set_scenarios(files);
                // Per-user editor preferences live beside the database, outside revisions.
                editor.set_prefs_path(options.db.with_extension("editor-prefs.json"));
                editor.bridge = (options.port > 0).then_some(options.port);
                Some(editor)
            }
        };
        let mut runtime = WorldRuntime::new(world, rev, registry.clone())?;
        if let Some(slot) = &options.load_slot {
            anyhow::ensure!(editor.is_none(), "editor starts from the committed scene");
            runtime.load_slot(
                &options
                    .db
                    .parent()
                    .unwrap_or(std::path::Path::new("data"))
                    .join("saves"),
                slot,
            )?;
        }
        let mut first_observation = runtime.observation();
        if let Some(editor) = &editor {
            first_observation
                .metrics
                .insert("editor".into(), editor.observation());
        }
        let observation = Arc::new(RwLock::new(first_observation));
        let (sender, commands) = std::sync::mpsc::sync_channel(32);
        let bridge = if options.port > 0 {
            Some(Bridge::start_with_runtime(
                options.db.clone(),
                options.port,
                observation.clone(),
                registry.authoring.clone(),
                Some(sender),
                Some(store.latest_cache()),
            )?)
        } else {
            None
        };
        let ctx = egui::Context::default();
        ui::configure(&ctx);
        let ui = UiState {
            playing: editor.is_none() && options.start_play,
            port: options.port,
            command: r#"{"op":"query"}"#.into(),
            output: vec![format!("OK  world compiled  rev={rev}")],
            ..Default::default()
        };
        Ok(Self {
            cursor: [0.5; 2],
            receipts: Default::default(),
            history: Default::default(),
            commands,
            options,
            store,
            runtime,
            editor,
            edit_runtime: None,
            ui,
            window: None,
            renderer: None,
            ctx,
            egui_state: None,
            keys: HashSet::new(),
            mouse_down: false,
            focused: true,
            captured: false,
            last: Instant::now(),
            accumulator: 0.0,
            poll_elapsed: 0.0,
            frames: 0,
            pending_capture: None,
            observation,
            _bridge: bridge,
            error: None,
            fps_smooth: 60.0,
            quit: false,
            last_reload: None,
            slow_reloads: 0,
            reloads: 0,
            scenario_timeout: SCENARIO_TIMEOUT,
            background_activation: false,
            activation: None,
            changes_job: None,
            slow_frames: 0,
            last_slow_frame: None,
            audio: crate::audio::Output::new(),
        })
    }
    fn editing(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| !e.previewing)
    }

    fn observation(&self) -> Observation {
        let mut observation = self.runtime.observation();
        if observation.metrics.contains_key("audio") {
            observation.metrics.get_mut("audio").unwrap()["output"] = self.audio.observation();
        }
        if let Some(editor) = &self.editor {
            observation
                .metrics
                .insert("editor".into(), editor.observation());
        }
        if let Some(reload) = self.reload_metrics() {
            observation.metrics.insert("reload".into(), reload);
        }
        if self.slow_frames > 0 {
            observation.metrics.insert(
                "frame".into(),
                serde_json::json!({"frames":self.frames,"slow_frames":self.slow_frames,"budget_ms":16.0,"last_slow":self.last_slow_frame}),
            );
        }
        if let Some(renderer) = &self.renderer {
            let stats = renderer.shader_stats();
            if stats.active_programs > 0 {
                observation.metrics.insert(
                    "shader_gpu".into(),
                    serde_json::json!({"last":stats,"budget_pipelines":crate::shader_gpu::MAX_PIPELINE_VARIANTS}),
                );
            }
        }
        observation
    }

    fn screen_pointer(&mut self, kind: crate::world::PointerKind) {
        if let Some(window) = &self.window {
            let size = window.inner_size();
            let aspect = size.width.max(1) as f32 / size.height.max(1) as f32;
            let additive =
                self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
            if let Err(e) = self
                .runtime
                .screen_pointer(kind, self.cursor, aspect, additive)
            {
                self.ui.output.push(format!("{e:#}"));
            }
        }
    }
    fn runtime_request(&mut self, request: Request) -> Result<serde_json::Value> {
        if let Request::EditorView {
            select,
            focus,
            dock,
            capture,
            play,
            open,
            command,
        } = request
        {
            if let Some(play) = play {
                anyhow::ensure!(
                    self.editor.is_some(),
                    "editor_view requires a running editor"
                );
                self.editor_play(play)?;
            }
            let editor = self.editor.as_mut().ok_or_else(|| {
                anyhow::anyhow!("editor_view requires a running editor (axiom editor)")
            })?;
            anyhow::ensure!(select.len() <= 64, "editor_view selects at most 64 objects");
            editor.remote_view(&select, focus, dock.as_deref())?;
            if let Some(target) = open.as_deref() {
                editor.remote_open(target)?;
            }
            // Registered editor commands run exactly as from the menu bar or palette.
            let commanded = match command.as_deref() {
                Some(id) => Some(editor.run_command(id)?),
                None => None,
            };
            for action in editor::coalesce(commanded.unwrap_or_default()) {
                self.editor_action(action)?;
            }
            let path = match capture {
                Some(name) => {
                    anyhow::ensure!(
                        !name.is_empty()
                            && name.len() <= 64
                            && name
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                            && !name.contains(".."),
                        "capture name must be 1..64 of [A-Za-z0-9-_.]"
                    );
                    let dir = std::env::current_dir()?.join("artifacts").join("captures");
                    std::fs::create_dir_all(&dir)?;
                    let path = dir.join(format!("{name}.png"));
                    self.pending_capture = Some(path.clone());
                    Some(path)
                }
                None => None,
            };
            return Ok(
                serde_json::json!({"ok":true,"editor":self.editor.as_ref().unwrap().observation(),"capture":path,"capture_ready":"written on the next rendered frame"}),
            );
        }
        anyhow::ensure!(
            !self.editing(),
            "runtime commands are available during Play preview"
        );
        if let Request::Execute {
            request_id,
            runtime_session,
            request,
        } = request
        {
            let session = runtime_session.ok_or_else(|| {
                axiom_core::Diagnostic::error(
                    "session_required",
                    "/runtime_session",
                    "get the current runtime_session from observe",
                )
            })?;
            if let Some(response) = self.receipts.lookup(&request_id, &session, &request)? {
                return Ok(response);
            }
            anyhow::ensure!(
                session == self.runtime.runtime_session,
                "runtime session changed; refresh observe before issuing a new command"
            );
            anyhow::ensure!(
                matches!(
                    *request,
                    Request::Command { .. } | Request::Save { .. } | Request::Load { .. }
                ),
                "runtime execute only supports command/save/load"
            );
            let mut response = self.runtime_request(*request.clone())?;
            response["request_id"] = serde_json::json!(request_id);
            response["durability"] = serde_json::json!("runtime_session");
            self.receipts
                .insert(request_id, &session, &request, response.clone())?;
            return Ok(response);
        }
        let root = if self.editor.is_some() {
            self.options.db.with_extension("preview-saves")
        } else {
            self.options
                .db
                .parent()
                .unwrap_or(std::path::Path::new("data"))
                .join("saves")
        };
        match request {
            Request::Command {
                command,
                expected_rev,
            } => {
                anyhow::ensure!(
                    expected_rev.is_none_or(|rev| rev == self.runtime.rev),
                    "runtime revision conflict"
                );
                self.runtime.command(&command)
            }
            Request::Save { slot } => self.runtime.save_slot(&root, &slot),
            Request::Load { slot } => self.runtime.load_slot(&root, &slot),
            _ => anyhow::bail!("not a runtime request"),
        }
    }
    fn capture(&mut self) {
        let should = self.ui.playing
            && !self.ui.console
            && self.runtime.pointer_allowed()
            && self.focused
            && self.options.smoke_frames.is_none();
        if let Some(window) = &self.window {
            if should != self.captured {
                if should {
                    let _ = window
                        .set_cursor_grab(CursorGrabMode::Locked)
                        .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
                } else {
                    let _ = window.set_cursor_grab(CursorGrabMode::None);
                    self.keys.clear();
                    self.mouse_down = false;
                }
                window.set_cursor_visible(!should);
                self.captured = should;
            }
        }
    }
    fn reload(&mut self) -> Result<()> {
        self.reload_after(None)
    }

    fn reload_after(&mut self, local_revision: Option<i64>) -> Result<()> {
        let latest = self.store.revision()?;
        if self.editor.as_ref().is_none_or(|e| e.rev == latest)
            && (self.editor.as_ref().is_some_and(|e| e.previewing)
                || latest == self.runtime.requested_rev)
        {
            return Ok(());
        }
        let started = Instant::now();
        let (rev, world) = self.store.current_shared()?;
        let mut timings = ReloadTimings {
            rev,
            read_ms: ms(started),
            ..Default::default()
        };
        if let Some(editor) = &mut self.editor {
            let phase = Instant::now();
            let copy = world.clone();
            timings.editor_clone_ms = ms(phase);
            let phase = Instant::now();
            let local = local_revision == Some(rev);
            if self.background_activation {
                // The Changes-list diff and freeing the replaced world run off this thread.
                if let Some((previous, diff)) = editor.update_deferred(rev, copy, local) {
                    if diff {
                        let next = world.clone();
                        let job = crate::jobs::Job::spawn(
                            "changes",
                            format!("revision {rev}"),
                            ACTIVATION_TIMEOUT,
                            move |_| axiom_core::diff::summary(&previous, &next),
                        )?;
                        if let Some((_, stale)) = self.changes_job.replace((rev, job)) {
                            stale.detach();
                        }
                    } else {
                        retire(previous);
                    }
                }
            } else {
                editor.update(rev, copy, local);
            }
            timings.editor_update_ms = ms(phase);
            if editor.previewing {
                // Play uses a pinned authoring revision and discards its simulation on exit.
                timings.total_ms = ms(started);
                self.record_reload(timings);
                return Ok(());
            }
        }
        if rev != self.runtime.requested_rev {
            if self.background_activation {
                // The last good runtime keeps running and rendering; the replacement is
                // built on a worker and adopted by `poll_activation` on a later frame.
                self.runtime.requested_rev = rev;
                timings.background = true;
                if self.activation.is_none() {
                    self.start_activation(world, rev, timings, started)?;
                }
                // Otherwise the in-flight build is superseded when it finishes.
                return Ok(());
            }
            let renderer = &mut self.renderer;
            let result = self.runtime.reconcile_checked(&*world, rev, |next| {
                if let Some(renderer) = renderer {
                    renderer.prepare_shaders(next.shaders())?;
                }
                Ok(())
            });
            if let Err(error) = result {
                timings.activation_failed = true;
                timings.total_ms = ms(started);
                self.record_reload(timings);
                self.activation_failed(rev, &error);
                return Ok(());
            }
            timings.reconcile = self.runtime.reconcile_timings;
            self.ui
                .output
                .push(format!("OK  runtime synchronized  rev={rev}"));
        }
        timings.total_ms = ms(started);
        self.record_reload(timings);
        Ok(())
    }

    fn activation_failed(&mut self, rev: i64, error: &anyhow::Error) {
        self.ui.output.push(format!("ACTIVATION FAILED: {error:#}"));
        if let Some(editor) = &mut self.editor {
            editor.set_error(format!(
                "Revision {rev} committed; activation failed · {error:#}"
            ));
        }
    }

    fn start_activation(
        &mut self,
        world: Arc<World>,
        rev: i64,
        mut timings: ReloadTimings,
        started: Instant,
    ) -> Result<()> {
        let registry = self.runtime.registry_clone();
        let job = crate::jobs::Job::spawn(
            "activation",
            format!("revision {rev}"),
            ACTIVATION_TIMEOUT,
            move |_| WorldRuntime::new(&*world, rev, registry),
        )?;
        timings.request_ms = ms(started);
        self.activation = Some(Activation { rev, job, timings });
        Ok(())
    }

    /// Non-blocking; once per frame. Adopts a finished background build on this thread
    /// (state transfer, GPU preparation, swap) or restarts it for a newer revision.
    fn poll_activation(&mut self) -> Result<()> {
        self.poll_activation_within(std::time::Duration::ZERO)
    }

    /// `poll_activation` that may wait up to `wait` (the unused part of this frame's 16 ms
    /// budget) so small revisions still go live in the frame that noticed them.
    fn poll_activation_within(&mut self, wait: std::time::Duration) -> Result<()> {
        let Some(activation) = &mut self.activation else {
            return Ok(());
        };
        let Some(outcome) = activation.job.poll_within(wait) else {
            return Ok(());
        };
        let Activation {
            rev,
            mut timings,
            job,
        } = self.activation.take().unwrap();
        timings.build_ms = job.elapsed().as_secs_f64() * 1000.0;
        drop(job);
        let built = match outcome {
            crate::jobs::Outcome::Done { value, .. } => value,
            crate::jobs::Outcome::Abandoned { stop } => Err(anyhow::anyhow!(
                "runtime build for revision {rev} {}",
                stop.name()
            )),
        };
        let previewing = self.editor.as_ref().is_some_and(|e| e.previewing);
        if previewing || rev != self.runtime.requested_rev {
            // Superseded by a newer revision (or Play preview pinned one); never adopt it.
            if let Ok(stale) = built {
                retire(stale);
            }
            if !previewing && self.runtime.requested_rev != self.runtime.rev {
                let started = Instant::now();
                let (rev, world) = self.store.current_shared()?;
                let timings = ReloadTimings {
                    rev,
                    read_ms: ms(started),
                    background: true,
                    ..Default::default()
                };
                self.runtime.requested_rev = rev;
                self.start_activation(world, rev, timings, started)?;
            }
            return Ok(());
        }
        let started = Instant::now();
        let renderer = &mut self.renderer;
        let adopted = self.runtime.adopt(
            built,
            |next| {
                if let Some(renderer) = renderer {
                    renderer.prepare_shaders(next.shaders())?;
                }
                Ok(())
            },
            started,
        );
        timings.adopt_ms = ms(started);
        timings.total_ms = timings.request_ms + timings.adopt_ms;
        match adopted {
            Ok(old) => {
                timings.reconcile = self.runtime.reconcile_timings;
                // Freeing a large runtime is real work; it is not needed on this thread.
                retire(old);
                self.ui
                    .output
                    .push(format!("OK  runtime synchronized  rev={rev}"));
            }
            Err(error) => {
                timings.activation_failed = true;
                self.activation_failed(rev, &error);
            }
        }
        self.record_reload(timings);
        Ok(())
    }

    fn poll_changes(&mut self) {
        let Some((rev, job)) = &mut self.changes_job else {
            return;
        };
        let rev = *rev;
        let Some(outcome) = job.poll() else {
            return;
        };
        self.changes_job = None;
        if let (crate::jobs::Outcome::Done { value, .. }, Some(editor)) =
            (outcome, &mut self.editor)
        {
            editor.set_changes_for(rev, &value);
        }
    }

    /// Completes a pending activation before a runtime request so protocol semantics
    /// (commands see the newest committed revision) do not depend on frame timing.
    fn finish_activation(&mut self) -> Result<()> {
        while self.activation.is_some() {
            self.poll_activation()?;
            if self.activation.is_some() {
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
        Ok(())
    }

    fn record_reload(&mut self, timings: ReloadTimings) {
        self.reloads += 1;
        if timings.total_ms > 16.0 {
            self.slow_reloads += 1;
        }
        self.last_reload = Some(timings);
    }

    fn reload_metrics(&self) -> Option<serde_json::Value> {
        if self.last_reload.is_none() && self.activation.is_none() {
            return None;
        }
        Some(
            serde_json::json!({"last":self.last_reload,"pending":self.activation.as_ref().map(|a| a.job.observation()),"reloads":self.reloads,"slow_reloads":self.slow_reloads,"budget_ms":16.0,"scope":"main-thread wall clock of applying a new authoring revision"}),
        )
    }

    fn editor_action(&mut self, action: editor::Action) -> Result<()> {
        let editor = self.editor.as_mut().unwrap();
        match action {
            editor::Action::Play => self.editor_play(true),
            editor::Action::Quit => {
                self.quit = true;
                Ok(())
            }
            editor::Action::Write { request, select } => {
                let previous = editor.rev;
                let result = self.store.execute(request.clone())?;
                let committed = matches!(request, Request::Apply { .. });
                let next = result["rev"].as_i64();
                editor.record(request, &result);
                if committed {
                    if result["changed"] == true {
                        editor.record_apply(previous);
                    } else {
                        editor.reset_draft();
                    }
                    self.reload_after(next)?;
                    if let Some(id) = select {
                        let editor = self.editor.as_mut().unwrap();
                        editor.select(Some(id));
                    }
                }
                Ok(())
            }
            editor::Action::LoadDiff { rev } => {
                let diff = self.store.execute(Request::Diff {
                    from_rev: rev - 1,
                    to_rev: rev,
                    offset: 0,
                    limit: Some(64),
                })?;
                self.editor.as_mut().unwrap().set_history_diff(rev, diff);
                Ok(())
            }
            editor::Action::RestoreTo { rev } => {
                let request = Request::Undo {
                    rev,
                    expected_rev: Some(editor.rev),
                };
                let result = self.store.execute(request.clone())?;
                editor.record(request, &result);
                self.reload_after(result["rev"].as_i64())
            }
            editor::Action::ResolveProposal {
                id,
                approve,
                rebase,
            } => {
                let result = self.store.execute(Request::ResolveProposal {
                    id,
                    approve,
                    rebase,
                })?;
                let editor = self.editor.as_mut().unwrap();
                editor.proposal_signature = None;
                if approve {
                    editor.record_apply(editor.rev);
                    editor.notice(format!(
                        "Approved proposal #{id} · committed revision {}",
                        result["rev"]
                    ));
                    self.reload_after(result["rev"].as_i64())
                } else {
                    editor.notice(format!("Rejected proposal #{id}; nothing was written"));
                    Ok(())
                }
            }
            editor::Action::RunScenario { path } => {
                anyhow::ensure!(
                    editor.scenario_job.is_none(),
                    "a scenario is already running; cancel it first"
                );
                // Reading, parsing and the revision snapshot are bounded (1 MiB); the
                // simulation (up to 1,000,000 ticks) runs on a worker thread.
                let prepared = (|| -> Result<(crate::scenario::Scenario, i64, World)> {
                    let raw = std::fs::read_to_string(&path)?;
                    anyhow::ensure!(raw.len() <= 1_048_576, "scenario exceeds 1 MiB");
                    let scenario = serde_json::from_str(&raw)?;
                    let (rev, world) = self.store.current()?;
                    Ok((scenario, rev, world))
                })();
                let editor = self.editor.as_mut().unwrap();
                match prepared {
                    Ok((scenario, rev, world)) => {
                        let registry = self.runtime.registry_clone();
                        let job = crate::jobs::Job::spawn(
                            "scenario",
                            path,
                            self.scenario_timeout,
                            move |progress| {
                                crate::scenario::run_observed(
                                    world,
                                    rev,
                                    registry,
                                    scenario,
                                    None,
                                    Some(progress),
                                )
                                .unwrap_or_else(|e| axiom_core::error_response(&e))
                            },
                        )?;
                        editor.start_scenario(job);
                    }
                    Err(e) => editor.set_scenario_result(&path, axiom_core::error_response(&e)),
                }
                Ok(())
            }
            editor::Action::CancelScenario => {
                editor.cancel_scenario();
                Ok(())
            }
            editor::Action::Verify => {
                let result = self.store.execute(Request::Verify {})?;
                self.editor.as_mut().unwrap().notice(format!(
                    "Verify passed · revision {} · {} entities",
                    result["rev"], result["entities"]
                ));
                Ok(())
            }
            editor::Action::Restore { rev, redo } => {
                let previous = editor.rev;
                let request = Request::Undo {
                    rev,
                    expected_rev: Some(previous),
                };
                let result = self.store.execute(request.clone())?;
                editor.record_restore(previous, redo);
                editor.record(request, &result);
                self.reload_after(result["rev"].as_i64())
            }
        }
    }

    fn prepare_runtime(&mut self, world: World, rev: i64) -> Result<WorldRuntime> {
        let next = WorldRuntime::new(world, rev, self.runtime.registry_clone())?;
        if let Some(renderer) = &mut self.renderer {
            renderer.prepare_shaders(next.shaders())?;
        }
        Ok(next)
    }

    fn editor_play(&mut self, play: bool) -> Result<()> {
        if self.editor.as_ref().unwrap().previewing == play {
            return Ok(());
        }
        if let Some(pending) = self.activation.take() {
            // Preview and stop build their runtimes from the current revision themselves.
            pending.job.detach();
        }
        let (rev, world) = self.store.current()?;
        self.editor
            .as_mut()
            .unwrap()
            .update(rev, world.clone(), false);
        if play {
            anyhow::ensure!(
                !self.editor.as_ref().unwrap().dirty(),
                "apply or reload the inspector draft before playing"
            );
            let preview = self.prepare_runtime(world, rev)?;
            self.edit_runtime = Some(std::mem::replace(&mut self.runtime, preview));
        } else {
            let mut edit = self
                .edit_runtime
                .take()
                .expect("preview has an edit snapshot");
            if edit.rev != rev {
                match self.prepare_runtime(world, rev) {
                    Ok(next) => edit = next,
                    Err(error) => {
                        self.editor
                            .as_mut()
                            .unwrap()
                            .set_error(format!("Showing last renderable revision · {error:#}"));
                        edit.requested_rev = rev;
                        edit.activation_error = Some(axiom_core::error_response(&error));
                    }
                }
            }
            self.runtime = edit;
        }
        self.editor.as_mut().unwrap().previewing = play;
        self.ui.playing = play;
        self.ui.console = false;
        self.keys.clear();
        self.mouse_down = false;
        self.accumulator = 0.0;
        self.capture();
        Ok(())
    }
    fn frame(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        for _ in 0..32 {
            let Ok(command) = self.commands.try_recv() else {
                break;
            };
            if std::time::Instant::now() >= command.deadline {
                continue;
            }
            let editor_view = matches!(command.request, Request::EditorView { .. });
            let result = self
                .reload()
                .and_then(|_| {
                    if editor_view {
                        Ok(())
                    } else {
                        self.finish_activation()
                    }
                })
                .and_then(|_| self.runtime_request(command.request));
            let response = result.unwrap_or_else(|e| axiom_core::error_response(&e));
            let _ = command.reply.send(response);
        }
        let now = Instant::now();
        let elapsed = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.fps_smooth = self.fps_smooth * 0.95 + elapsed.max(0.001).recip() * 0.05;
        self.ui.fps = self.fps_smooth;
        self.poll_elapsed += elapsed;
        // Proposals load immediately at startup / after a resolve, then follow the poll.
        let proposals_unknown = self
            .editor
            .as_ref()
            .is_some_and(|e| e.proposal_signature.is_none());
        let mut phases = FramePhases::start(now);
        let poll = self.poll_elapsed >= 0.25;
        if poll {
            self.poll_elapsed = 0.0;
            self.reload()?;
        }
        if poll || proposals_unknown {
            if let Some(editor) = &self.editor {
                let signature = self.store.proposal_signature()?;
                if editor.proposal_signature != Some(signature) {
                    let rows = self.store.execute(Request::Proposals {
                        status: Some("pending".into()),
                        limit: Some(32),
                    })?;
                    self.editor
                        .as_mut()
                        .unwrap()
                        .set_proposals(signature, &rows["proposals"]);
                }
            }
        }
        phases.mark("poll_reload_proposals");
        if self.ui.playing
            && !self.ui.console
            && (self.focused || self.options.smoke_frames.is_some())
        {
            self.accumulator += if self.options.smoke_frames.is_some() {
                1.0 / 60.0
            } else {
                elapsed
            };
            let mut keys = self
                .keys
                .iter()
                .map(|k| format!("{k:?}"))
                .collect::<std::collections::BTreeSet<_>>();
            if self.mouse_down {
                keys.insert("MouseLeft".into());
            }
            let input = ActionMap::from_bindings(&self.runtime.bindings, &keys);
            let mut steps = 0;
            while self.accumulator >= FIXED_DT && steps < 12 {
                self.runtime.step(&input);
                self.accumulator -= FIXED_DT;
                steps += 1;
            }
            if steps == 12 {
                self.accumulator = 0.0;
            }
        } else {
            self.accumulator = 0.0;
        }
        if let Some(editor) = &mut self.editor {
            // Background jobs deliver results here; never waited for on this thread.
            editor.poll_jobs();
        }
        self.poll_activation()?;
        self.poll_changes();
        phases.mark("simulate");
        let window = self.window.as_ref().unwrap().clone();
        let raw = self.egui_state.as_mut().unwrap().take_egui_input(&window);
        let ctx = self.ctx.clone();
        let editing = self.editing();
        let mut actions = vec![];
        let mut editor_actions = vec![];
        let mut stop = false;
        let mut toggle = false;
        let mut step = false;
        // Play-in-editor debug overlay: live FSM state above each agent, projected through
        // the first game view of this frame.
        // Trigger zones are outlined too, pink while something is inside.
        let mut zone_lines: Vec<([f32; 2], [f32; 2], bool)> = vec![];
        // Navigation agents' remaining routes.
        let mut route_lines: Vec<([f32; 2], [f32; 2])> = vec![];
        let overlay: Vec<([f32; 2], String)> =
            if !editing && self.editor.as_ref().is_some_and(|e| e.overlay) {
                let screen = self.ctx.screen_rect();
                let bounds = [screen.min.x, screen.min.y, screen.width(), screen.height()];
                let views = self.runtime.render_frame().views;
                let view = views.iter().find(|v| v.camera.active);
                if let (Some(triggers), Some(view)) =
                    (self.runtime.system::<crate::trigger::TriggerSystem>(), view)
                {
                    zone_lines = triggers
                        .wireframes(&self.runtime.state)
                        .into_iter()
                        .filter_map(|(a, b, occupied)| {
                            Some((view.project(a, bounds)?, view.project(b, bounds)?, occupied))
                        })
                        .collect();
                }
                if let (Some(nav), Some(view)) =
                    (self.runtime.system::<crate::nav_agent::NavSystem>(), view)
                {
                    for (at, rest) in nav.routes(&self.runtime.state) {
                        let mut from = at;
                        for to in rest {
                            if let (Some(a), Some(b)) =
                                (view.project(from, bounds), view.project(to, bounds))
                            {
                                route_lines.push((a, b));
                            }
                            from = to;
                        }
                    }
                }
                match (self.runtime.system::<crate::fsm::FsmSystem>(), view) {
                    (Some(fsm), Some(view)) => fsm
                        .labels(&self.runtime.state)
                        .into_iter()
                        .filter_map(|(p, text)| {
                            view.project(glam::Vec3::from_array(p), bounds)
                                .map(|s| (s, text))
                        })
                        .collect(),
                    _ => vec![],
                }
            } else {
                vec![]
            };
        let output = ctx.run(raw, |ctx| {
            if editing {
                let editor = self.editor.as_mut().unwrap();
                editor.fps = self.ui.fps;
                editor_actions = editor.draw(ctx, &self.runtime);
            } else {
                actions = self.runtime.draw_ui(ctx, &mut self.ui);
                if self.editor.is_some() {
                    egui::Area::new(egui::Id::new("editor_stop"))
                        .anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                stop = ui.button("■ Stop Preview  F8").clicked();
                                toggle = ui
                                    .button(if self.ui.playing { "Pause" } else { "Resume" })
                                    .on_hover_text("Esc")
                                    .clicked();
                                step = ui
                                    .add_enabled(!self.ui.playing, egui::Button::new("Step"))
                                    .on_hover_text("Advance one fixed tick while paused")
                                    .clicked();
                            });
                            if let Some(editor) = self.editor.as_mut() {
                                ui.checkbox(&mut editor.overlay, "Behavior overlay");
                            }
                        });
                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new("behavior_overlay"),
                    ));
                    for ([ax, ay], [bx, by], occupied) in &zone_lines {
                        let color = if *occupied {
                            egui::Color32::from_rgb(255, 92, 140)
                        } else {
                            egui::Color32::from_rgb(120, 230, 210)
                        };
                        painter.line_segment(
                            [egui::pos2(*ax, *ay), egui::pos2(*bx, *by)],
                            egui::Stroke::new(if *occupied { 2.0_f32 } else { 1.0 }, color),
                        );
                    }
                    for ([ax, ay], [bx, by]) in &route_lines {
                        painter.line_segment(
                            [egui::pos2(*ax, *ay), egui::pos2(*bx, *by)],
                            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(170, 240, 120)),
                        );
                    }
                    for ([x, y], text) in &overlay {
                        let galley = painter.layout_no_wrap(
                            text.clone(),
                            egui::FontId::monospace(12.0),
                            egui::Color32::WHITE,
                        );
                        let rect = egui::Rect::from_center_size(
                            egui::pos2(*x, *y),
                            galley.size() + egui::vec2(8.0, 4.0),
                        );
                        painter.rect_filled(rect, 3.0, egui::Color32::from_black_alpha(170));
                        painter.galley(
                            rect.min + egui::vec2(4.0, 2.0),
                            galley,
                            egui::Color32::WHITE,
                        );
                    }
                }
            }
        });
        phases.mark("jobs_overlay_ui");
        self.egui_state
            .as_mut()
            .unwrap()
            .handle_platform_output(&window, output.platform_output.clone());
        for action in editor::coalesce(editor_actions) {
            if let Err(error) = self.editor_action(action) {
                self.editor.as_mut().unwrap().problem(&error);
                self.reload()?;
            }
        }
        if self.editor.as_ref().is_some_and(|e| e.needs_history) {
            let rows = self.store.execute(Request::History { limit: Some(64) })?;
            self.editor
                .as_mut()
                .unwrap()
                .set_history(&rows["revisions"]);
        }
        if toggle {
            self.ui.playing = !self.ui.playing;
            self.capture();
        }
        if step && !self.ui.playing {
            self.runtime.step(&ActionMap::default());
        }
        if stop {
            self.editor_play(false)?;
        }
        if self.quit {
            event_loop.exit();
            return Ok(());
        }
        for action in actions {
            match action {
                Action::Gameplay(command) => {
                    if let Err(e) = self.runtime.command(&command) {
                        self.ui.output.push(format!("ERROR {e:#}"));
                    }
                }
                Action::Save | Action::Load => {
                    let request = if matches!(action, Action::Save) {
                        Request::Save {
                            slot: "quick".into(),
                        }
                    } else {
                        Request::Load {
                            slot: "quick".into(),
                        }
                    };
                    let result = self.runtime_request(request);
                    self.ui.output.push(match result {
                        Ok(v) => v.to_string(),
                        Err(e) => format!("ERROR {e:#}"),
                    });
                }
                Action::Start | Action::Resume => {
                    self.ui.playing = true;
                    self.ui.console = false;
                }
                Action::Restart => {
                    let (rev, world) = if self.editor.is_some() {
                        (self.runtime.rev, self.runtime.state.compiled.source.clone())
                    } else {
                        self.store.current()?
                    };
                    let replacement = self.prepare_runtime(world, rev);
                    match replacement {
                        Ok(next) => {
                            self.runtime = next;
                            self.ui.playing = true;
                            self.ui.console = false;
                        }
                        Err(error) => self.ui.output.push(format!("RESTART FAILED: {error:#}")),
                    }
                }
                Action::Quit => event_loop.exit(),
                Action::Apply(command) => {
                    let response =
                        parse_request(&command).and_then(|request| if matches!(request,Request::Observe{}) {Ok(serde_json::json!({"ok":true,"runtime":true,"observation":self.observation()}))}else if let Request::ObserveDelta{cursor,fields,trace_limit,max_bytes}=request {let snapshot=serde_json::json!(self.observation());self.history.observe(&snapshot,cursor.as_deref(),&fields,trace_limit,max_bytes)}else if request.is_runtime(){self.runtime_request(request)}else{self.store.execute(request)});
                    self.ui.output.push(match response {
                        Ok(v) => v.to_string(),
                        Err(e) => format!("ERROR  {e:#}"),
                    });
                    self.reload()?;
                }
            }
        }
        if self.ui.output.len() > 32 {
            self.ui.output.drain(..self.ui.output.len() - 32);
        }
        self.capture();
        self.audio.update(
            &self.runtime.state.audio,
            &self.runtime.runtime_session,
            self.ui.playing && !self.editing() && !self.ui.console,
        );
        if self.activation.is_some() {
            // Spend what is left of this frame's budget (minus a margin for extraction and
            // submission) waiting for a background build, so small revisions go live in the
            // frame that noticed them, as the old synchronous path did.
            let left = std::time::Duration::from_millis(12).saturating_sub(now.elapsed());
            self.poll_activation_within(left)?;
        }
        self.frames += 1;
        let final_frame = self.options.smoke_frames.is_some_and(|n| self.frames >= n);
        let pending = self.pending_capture.take();
        let capture_path = if final_frame {
            self.options.screenshot.as_deref()
        } else {
            pending.as_deref()
        };
        phases.mark("actions_history_audio");
        let frame = if editing {
            self.editor
                .as_ref()
                .unwrap()
                .render_frame(&mut self.runtime, ctx.screen_rect())
        } else {
            self.runtime.render_frame()
        };
        phases.mark("extract");
        let rendered =
            self.renderer
                .as_mut()
                .unwrap()
                .render(&frame, &ctx, output, capture_path)?;
        if !rendered {
            self.frames -= 1;
            // Keep an on-demand capture for the next frame that actually renders.
            if !final_frame {
                self.pending_capture = pending;
            }
            return Ok(());
        }
        phases.mark("render_present");
        // A background build usually completes while this frame waits for the swapchain;
        // adopting here instead of next frame saves one frame of edit -> live latency.
        self.poll_activation()?;
        self.poll_changes();
        let mut obs = self.observation();
        obs.fps = self.fps_smooth;
        obs.frame_ms_cpu = now.elapsed().as_secs_f32() * 1000.0;
        obs.gpu = self.ui.gpu.clone();
        if let Ok(mut value) = self.observation.write() {
            *value = obs.clone();
        }
        phases.mark("observation");
        if let Some(slow) = phases.finish() {
            self.slow_frames += 1;
            self.last_slow_frame = Some(slow);
        }
        if final_frame {
            if let Some(path) = &self.options.screenshot {
                let json_path = path.with_extension("json");
                std::fs::write(json_path, serde_json::to_string_pretty(&obs)?)?;
            }
            let gpu_memory = self
                .renderer
                .as_ref()
                .map(|r| crate::measure::gpu_memory(&r.device));
            println!(
                "{}",
                serde_json::json!({"ok":true,"frames":self.frames,"gpu":self.ui.gpu,"gpu_memory":gpu_memory,"process_memory":crate::measure::process_memory(),"observation":obs,"screenshot":self.options.screenshot})
            );
            event_loop.exit();
        }
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result<()> {
            let size = event_loop
                .primary_monitor()
                .map(|monitor| {
                    let screen = monitor.size().to_logical::<f64>(monitor.scale_factor());
                    winit::dpi::LogicalSize::new(
                        (screen.width * 0.94).clamp(960.0, 1440.0),
                        (screen.height * 0.88).clamp(600.0, 900.0),
                    )
                })
                .unwrap_or(winit::dpi::LogicalSize::new(1280.0, 800.0));
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title(format!(
                            "AXIOM{} / {}",
                            if self.editor.is_some() { " Editor" } else { "" },
                            self.runtime.state.compiled.source.title
                        ))
                        .with_window_icon(crate::branding::window_icon())
                        .with_inner_size(size)
                        .with_min_inner_size(winit::dpi::LogicalSize::new(960.0, 600.0))
                        .with_visible(self.options.smoke_frames.is_none()),
                )?,
            );
            let mut renderer = pollster::block_on(Renderer::new(window.clone()))?;
            renderer.prepare_shaders(self.runtime.shaders())?;
            // Interactive window: revisions activate in the background from now on.
            self.background_activation = true;
            self.ui.gpu = renderer.gpu.clone();
            self.egui_state = Some(egui_winit::State::new(
                self.ctx.clone(),
                egui::ViewportId::ROOT,
                &window,
                Some(window.scale_factor() as f32),
                None,
                None,
            ));
            self.window = Some(window);
            self.renderer = Some(renderer);
            self.last = Instant::now();
            self.capture();
            Ok(())
        })();
        if let Err(e) = result {
            self.error = Some(e);
            event_loop.exit();
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let consumed = if let (Some(egui), Some(window)) = (&mut self.egui_state, &self.window) {
            egui.on_window_event(window, &event).consumed
        } else {
            false
        };
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(window) = &self.window {
                    let size = window.inner_size();
                    self.cursor = [
                        (position.x as f32 / size.width.max(1) as f32).clamp(0.0, 1.0),
                        (position.y as f32 / size.height.max(1) as f32).clamp(0.0, 1.0),
                    ];
                }
                if self.ui.playing && !self.ui.console && !self.captured {
                    self.screen_pointer(crate::world::PointerKind::Move);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                if self.ui.playing && !self.ui.console && !self.captured && !consumed {
                    self.screen_pointer(crate::world::PointerKind::SecondaryDown);
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size);
                }
            }
            WindowEvent::Focused(value) => {
                self.focused = value;
                self.capture();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    let down = event.state == ElementState::Pressed;
                    if self.editing() {
                        return;
                    }
                    if down && !event.repeat {
                        if key == KeyCode::F8 && self.editor.is_some() {
                            if let Err(error) = self.editor_play(false) {
                                self.ui.output.push(format!("STOP FAILED: {error:#}"));
                            }
                            return;
                        }
                        if key == KeyCode::Escape {
                            if self.ui.console {
                                self.ui.console = false;
                            } else {
                                self.ui.playing = !self.ui.playing;
                            }
                            self.capture();
                        }
                        if key == KeyCode::Tab {
                            self.ui.console = !self.ui.console;
                            self.capture();
                        }
                        if key == KeyCode::Enter && !self.ui.playing && !self.ui.console {
                            self.ui.playing = true;
                            self.capture();
                        }
                    }
                    if down {
                        self.keys.insert(key);
                    } else {
                        self.keys.remove(&key);
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if self.ui.playing
                    && !self.ui.console
                    && !self.captured
                    && (!consumed || state == ElementState::Released)
                {
                    self.screen_pointer(if state == ElementState::Pressed {
                        crate::world::PointerKind::PrimaryDown
                    } else {
                        crate::world::PointerKind::PrimaryUp
                    });
                }
                if state == ElementState::Released {
                    self.mouse_down = false;
                } else if self.ui.playing && !self.ui.console && (self.captured || !consumed) {
                    self.mouse_down = true;
                }
            }
            WindowEvent::RedrawRequested if self.options.smoke_frames.is_none() => {
                if let Err(e) = self.frame(event_loop) {
                    self.error = Some(e);
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if self.captured {
            if let DeviceEvent::MouseMotion { delta } = event {
                self.runtime.pointer(delta.0 as f32, delta.1 as f32);
            }
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if event_loop.exiting() {
            return;
        }
        // Windows suppresses WM_PAINT for invisible windows. Drive GPU QA explicitly.
        if self.options.smoke_frames.is_some() && self.renderer.is_some() {
            if let Err(error) = self.frame(event_loop) {
                self.error = Some(error);
                event_loop.exit();
            }
        } else if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_core::Edit;
    use serde_json::json;

    #[test]
    fn editor_preview_pins_authoring_and_discards_simulation_on_stop() {
        let world =
            serde_json::from_str(include_str!("../../../content/two-cameras/world.json")).unwrap();
        let registry = crate::installed_registry();
        let store = Store::memory_with(registry.authoring.clone(), world).unwrap();
        let mut app = App::new(
            RunOptions {
                mode: RunMode::Editor {
                    selected: Some("box".into()),
                },
                load_slot: None,
                replace_world: false,
                db: PathBuf::new(),
                port: 0,
                smoke_frames: None,
                screenshot: None,
                start_play: false,
            },
            registry,
            store,
        )
        .unwrap();
        let slot = app.runtime.state.compiled.indices["box"];
        let initial = app.runtime.state.transforms[slot].clone();
        let edit_session = app.runtime.runtime_session.clone();
        app.editor_action(editor::Action::Play).unwrap();
        assert!(!app.editing());
        assert_ne!(app.runtime.runtime_session, edit_session);
        app.runtime.step(&ActionMap::default());
        assert_ne!(app.runtime.state.transforms[slot], initial);
        let change = app
            .store
            .apply(
                Some(1),
                vec![Edit::set("title", json!("Concurrent authoring"))],
            )
            .unwrap();
        assert_eq!(change["rev"], 2);
        app.reload().unwrap();
        assert_eq!(app.runtime.rev, 1);
        assert_eq!(app.observation().metrics["editor"]["authoring_rev"], 2);
        app.editor_play(false).unwrap();
        assert!(app.editing());
        assert_eq!(app.runtime.rev, 2);
        assert_eq!(app.runtime.state.tick, 0);
        assert_eq!(app.runtime.state.transforms[slot], initial);
        assert_eq!(app.store.revision().unwrap(), 2);
    }

    fn editor_app() -> App {
        let world =
            serde_json::from_str(include_str!("../../../content/two-cameras/world.json")).unwrap();
        let registry = crate::installed_registry();
        let store = Store::memory_with(registry.authoring.clone(), world).unwrap();
        App::new(
            RunOptions {
                mode: RunMode::Editor { selected: None },
                load_slot: None,
                replace_world: false,
                db: PathBuf::new(),
                port: 0,
                smoke_frames: None,
                screenshot: None,
                start_play: false,
            },
            registry,
            store,
        )
        .unwrap()
    }

    fn scenario_file(name: &str, ticks: u32) -> String {
        let dir = std::env::temp_dir().join(format!("axiom-scenario-jobs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(
            &path,
            json!({"ticks":ticks,"assertions":[{"kind":"finite"}]}).to_string(),
        )
        .unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Draws one editor frame the way `App::frame` does (poll jobs, then draw).
    fn draw(app: &mut App, ctx: &egui::Context) -> std::time::Duration {
        let started = Instant::now();
        let editor = app.editor.as_mut().unwrap();
        editor.poll_jobs();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 760.0),
                )),
                ..Default::default()
            },
            |ctx| {
                let _ = editor.draw(ctx, &app.runtime);
            },
        );
        started.elapsed()
    }

    fn wait_result(app: &mut App, ctx: &egui::Context, path: &str) -> serde_json::Value {
        let deadline = Instant::now() + std::time::Duration::from_secs(60);
        loop {
            draw(app, ctx);
            let editor = app.editor.as_ref().unwrap();
            if editor.scenario_job.is_none() {
                return editor.scenario_results[path].clone();
            }
            assert!(Instant::now() < deadline, "scenario result never arrived");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn editor_scenario_runs_in_background_with_progress_cancel_and_result() {
        let mut app = editor_app();
        let ctx = egui::Context::default();
        let long = scenario_file("long.scenario.json", 1_000_000);
        let started = Instant::now();
        app.editor_action(editor::Action::RunScenario { path: long.clone() })
            .unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "starting a scenario must not run it on the main thread"
        );
        // The UI keeps drawing while the worker advances.
        let mut frames = 0;
        let mut last = 0;
        let mut slowest = std::time::Duration::ZERO;
        while frames < 5 || last == 0 {
            slowest = slowest.max(draw(&mut app, &ctx));
            frames += 1;
            last = app
                .editor
                .as_ref()
                .unwrap()
                .scenario_job
                .as_ref()
                .unwrap()
                .progress()
                .0;
            assert!(frames < 100_000, "scenario never progressed");
        }
        println!("{frames} editor frames drawn while the scenario ran; slowest {slowest:?}");
        assert!(slowest < std::time::Duration::from_millis(250));
        let observed = app.observation().metrics["editor"]["jobs"].clone();
        assert_eq!(observed[0]["kind"], "scenario");
        assert_eq!(observed[0]["total"], 1_000_000);
        assert!(observed[0]["done"].as_u64().unwrap() < 1_000_000);
        // Only one scenario job at a time.
        assert!(app
            .editor_action(editor::Action::RunScenario { path: long.clone() })
            .is_err());
        app.editor_action(editor::Action::CancelScenario).unwrap();
        let result = wait_result(&mut app, &ctx, &long);
        assert_eq!(result["ok"], false);
        assert_eq!(result["stopped"], "cancelled");
        assert_eq!(result["diagnostics"][0]["code"], "scenario_cancelled");
        let editor = app.editor.as_ref().unwrap();
        assert!(editor.error && editor.status.contains("cancelled"));
        assert_eq!(app.observation().metrics["editor"]["jobs"], json!([]));
        assert_eq!(
            app.observation().metrics["editor"]["last_job"]["state"],
            "cancelled"
        );
        // A short scenario delivers its PASS result to the tab.
        let short = scenario_file("short.scenario.json", 30);
        app.editor_action(editor::Action::RunScenario {
            path: short.clone(),
        })
        .unwrap();
        let result = wait_result(&mut app, &ctx, &short);
        assert_eq!(result["ok"], true);
        assert_eq!(result["ticks"], 30);
        assert!(result["state_hash"].is_string());
        assert!(app.editor.as_ref().unwrap().status.contains("passed"));
    }

    #[test]
    fn editor_scenario_watchdog_stops_a_long_run() {
        let mut app = editor_app();
        app.scenario_timeout = std::time::Duration::from_millis(30);
        let ctx = egui::Context::default();
        let long = scenario_file("watchdog.scenario.json", 1_000_000);
        app.editor_action(editor::Action::RunScenario { path: long.clone() })
            .unwrap();
        let result = wait_result(&mut app, &ctx, &long);
        assert_eq!(result["ok"], false);
        assert_eq!(result["stopped"], "timed_out");
        // Same input and revision through the synchronous CLI path is unaffected.
        let scenario =
            serde_json::from_value(json!({"ticks":30,"assertions":[{"kind":"finite"}]})).unwrap();
        let (rev, world) = app.store.current().unwrap();
        let cli = crate::scenario::run(world, rev, app.runtime.registry_clone(), scenario).unwrap();
        let short = scenario_file("same.scenario.json", 30);
        app.editor_action(editor::Action::RunScenario {
            path: short.clone(),
        })
        .unwrap();
        let background = wait_result(&mut app, &ctx, &short);
        assert_eq!(background["state_hash"], cli["state_hash"]);
    }

    #[test]
    fn background_activation_keeps_the_last_runtime_until_the_build_is_adopted() {
        let mut app = editor_app();
        app.background_activation = true;
        for _ in 0..10 {
            app.runtime.step(&ActionMap::default());
        }
        let slot = app.runtime.state.compiled.indices["box"];
        let spun = app.runtime.state.transforms[slot].clone();
        let session = app.runtime.runtime_session.clone();
        let generation = app.runtime.state.ids[slot];
        let rev = app.store.revision().unwrap();
        app.editor_action(editor::Action::Write {
            request: Request::Apply {
                expected_rev: Some(rev),
                edits: vec![Edit::set("title", json!("Background activation"))],
            },
            select: None,
        })
        .unwrap();
        // The editor follows the commit at once; the runtime keeps the last good revision.
        assert_eq!(app.editor.as_ref().unwrap().rev, rev + 1);
        assert_eq!(app.runtime.rev, rev);
        assert_eq!(app.runtime.requested_rev, rev + 1);
        assert!(app.observation().metrics["reload"]["pending"]["kind"] == "activation");
        app.runtime.step(&ActionMap::default());
        let tick = app.runtime.state.tick;
        app.finish_activation().unwrap();
        assert_eq!(app.runtime.rev, rev + 1);
        assert_eq!(
            app.runtime.state.compiled.source.title,
            "Background activation"
        );
        // Same carry-over as a synchronous reconcile: session, tick, ids, transforms.
        assert_eq!(app.runtime.runtime_session, session);
        assert_eq!(app.runtime.state.tick, tick);
        assert_eq!(app.runtime.state.ids[slot], generation);
        assert_ne!(
            app.runtime.state.transforms[slot], spun,
            "kept simulating meanwhile"
        );
        let last = app.last_reload.unwrap();
        assert!(last.background && !last.activation_failed && last.rev == rev + 1);
        assert!(app.observation().metrics["reload"]["pending"].is_null());

        // A failed build keeps the last renderable runtime and reports the error.
        app.runtime.requested_rev = rev + 7;
        app.activation = Some(Activation {
            rev: rev + 7,
            job: crate::jobs::Job::spawn("activation", "failing", ACTIVATION_TIMEOUT, |_| {
                Err(anyhow::anyhow!("factory exploded"))
            })
            .unwrap(),
            timings: ReloadTimings::default(),
        });
        app.finish_activation().unwrap();
        assert_eq!(app.runtime.rev, rev + 1);
        assert_eq!(app.observation().activation, "activation_failed");
        assert!(app.last_reload.unwrap().activation_failed);
        assert!(app
            .editor
            .as_ref()
            .unwrap()
            .status
            .contains("activation failed"));
    }

    #[test]
    fn external_revision_change_list_is_computed_off_the_main_thread() {
        let mut app = editor_app();
        app.background_activation = true;
        let rev = app.store.revision().unwrap();
        app.store
            .apply(
                Some(rev),
                vec![Edit::set("title", json!("External writer"))],
            )
            .unwrap();
        app.reload().unwrap();
        assert_eq!(app.editor.as_ref().unwrap().rev, rev + 1);
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        while app.changes_job.is_some() {
            app.poll_changes();
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let changes = &app.editor.as_ref().unwrap().changes;
        assert!(changes.iter().any(|c| c.contains("/title")), "{changes:?}");
        app.finish_activation().unwrap();
        assert_eq!(app.runtime.rev, rev + 1);
    }

    #[test]
    fn superseded_background_build_is_restarted_for_the_newest_revision() {
        let mut app = editor_app();
        app.background_activation = true;
        let base = app.store.revision().unwrap();
        for (i, title) in ["first", "second", "third"].into_iter().enumerate() {
            app.editor_action(editor::Action::Write {
                request: Request::Apply {
                    expected_rev: Some(base + i as i64),
                    edits: vec![Edit::set("title", json!(title))],
                },
                select: None,
            })
            .unwrap();
        }
        app.finish_activation().unwrap();
        // Adoption follows the newest committed revision, never an intermediate one.
        assert_eq!(app.runtime.rev, base + 3);
        assert_eq!(app.runtime.state.compiled.source.title, "third");
        assert!(app.activation.is_none());
    }
}
