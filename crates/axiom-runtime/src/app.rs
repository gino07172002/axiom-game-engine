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
    /// Editor commits on the background writer (windowed editor and measurements);
    /// without it (headless tests, in-memory stores) they commit on this thread.
    writes: Option<Writes>,
    /// A bridge request postponed to the next frame because the write queue cannot take it
    /// yet (its order relative to later requests is kept).
    deferred_command: Option<crate::bridge::RuntimeRequest>,
    /// Closing: pending commits are flushed, no new runtime builds start.
    closing: bool,
}

/// Wall-clock phases of one main-loop frame; only frames over budget are kept.
struct FramePhases {
    started: Instant,
    last: Instant,
    phases: [(&'static str, f64); 9],
    count: usize,
}
impl FramePhases {
    fn start(now: Instant) -> Self {
        Self {
            started: now,
            last: now,
            phases: [("", 0.0); 9],
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
    /// A part of the phase that ends at the next `mark` (reported beside it, not subtracted).
    fn note(&mut self, name: &'static str, ms: f64) {
        if self.count < self.phases.len() {
            self.phases[self.count] = (name, ms);
            self.count += 1;
        }
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

/// One editor write in the background writer's queue (see `crate::writer`).
struct PendingWrite {
    seq: u64,
    kind: WriteKind,
    /// Submitted while earlier writes were queued: built on their optimistic world, so it is
    /// sent with the revision the previous write produced and discarded if one fails.
    chained: bool,
    /// The editor revision the request was built against (`editor.rev` at submission).
    view_rev: i64,
    stage: Stage,
    /// The request as sent (CAS revision resolved) and its expected revision.
    sent: Option<(Request, i64)>,
    /// Bridge `editor_view` replies waiting for this write: (reply, response so far).
    replies: Vec<(
        std::sync::mpsc::SyncSender<serde_json::Value>,
        serde_json::Value,
    )>,
}

enum WriteKind {
    Write {
        request: Request,
        select: Option<String>,
    },
    /// Undo / redo; the target revision is resolved from the undo stack when sent.
    Restore {
        rev: i64,
        redo: bool,
    },
    RestoreTo {
        rev: i64,
    },
    Resolve {
        id: i64,
        approve: bool,
        rebase: bool,
    },
}

impl WriteKind {
    /// For problem messages: what was (not) saved.
    fn label(&self) -> String {
        match self {
            WriteKind::Write { request, .. } => crate::writer::describe(request),
            WriteKind::Restore { redo: true, .. } => "redo".into(),
            WriteKind::Restore { .. } => "undo".into(),
            WriteKind::RestoreTo { rev } => format!("restore of revision {rev}"),
            WriteKind::Resolve { id, approve, .. } => format!(
                "{} of proposal #{id}",
                if *approve { "approval" } else { "rejection" }
            ),
        }
    }
}

/// What the editor shows of a queued write.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// Optimistic world being computed off the main thread; no further writes meanwhile.
    Staging,
    /// Shown (or changes nothing in the world): later writes may queue behind it.
    Ready,
    /// Staging refused the edit; the draft stays as typed and the commit reports the error.
    Refused,
    /// The resulting world is only known after the commit (undo, restore, proposals).
    Barrier,
}

/// Editor writes on the background writer: FIFO on the main thread, one in flight.
struct Writes {
    writer: crate::writer::Writer,
    registry: Arc<axiom_core::Registry>,
    queue: std::collections::VecDeque<PendingWrite>,
    /// Revision the next request is sent against: the editor's revision when the queue was
    /// empty, then each queued commit's result.
    chain_rev: i64,
    staging: Option<(u64, crate::jobs::Job<Result<Arc<World>>>)>,
    in_flight: Option<InFlight>,
    next_seq: u64,
    committed: u64,
    failed: u64,
    discarded: u64,
    last_commit: Option<(i64, f64)>,
    recent_ms: std::collections::VecDeque<f64>,
    commit_timeout: std::time::Duration,
    commit_grace: std::time::Duration,
}

struct InFlight {
    seq: u64,
    sent: Instant,
    interrupted: Option<Instant>,
    what: String,
}

impl Writes {
    fn busy(&self) -> Option<String> {
        if self.queue.len() >= crate::writer::MAX_PENDING {
            return Some(format!(
                "{} edits are still saving; wait for them to finish",
                self.queue.len()
            ));
        }
        self.queue.iter().find_map(|w| match w.stage {
            Stage::Ready => None,
            Stage::Staging => {
                Some("Saving: the previous edit is still being prepared; try again".into())
            }
            Stage::Refused => {
                Some("Saving: the previous edit was refused; waiting for its result".into())
            }
            Stage::Barrier => {
                Some("Saving: waiting for an undo/restore to commit; try again".into())
            }
        })
    }

    fn observation(&self) -> serde_json::Value {
        serde_json::json!({
            "pending":self.queue.len(),
            "max_pending":crate::writer::MAX_PENDING,
            "in_flight":self.in_flight.as_ref().map(|f| serde_json::json!({"what":f.what,"elapsed_ms":f.sent.elapsed().as_millis() as u64,"interrupted":f.interrupted.is_some()})),
            "committed":self.committed,
            "failed":self.failed,
            "discarded":self.discarded,
            "last_commit_rev":self.last_commit.map(|c| c.0),
            "last_commit_ms":self.last_commit.map(|c| c.1),
            "recent_ms":self.recent_ms,
            "timeout_ms":self.commit_timeout.as_millis() as u64,
            "scope":"background writer wall clock of each editor commit (main thread does not wait)",
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
    let editor_mode = matches!(options.mode, RunMode::Editor { .. });
    let mut app = App::new(options, registry, store)?;
    if editor_mode {
        app.enable_writer(None)?;
    }
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let ran = event_loop.run_app(&mut app);
    // Edits submitted before the window closed are committed (or reported) before exit.
    let flushed = app.close_writes(crate::writer::SHUTDOWN_FLUSH);
    ran?;
    if let Some(error) = app.error {
        return Err(error);
    }
    flushed
}

/// One measured edit → live sample (milliseconds, main-thread wall clock).
#[derive(Clone, Debug, serde::Serialize)]
pub struct EditLiveSample {
    /// From submitting the editor write until the active runtime runs the new revision.
    pub edit_to_live_ms: f64,
    /// `Store::execute(apply)`: validation, diff, SQLite commit (on the writer thread when
    /// `writer`, otherwise on the main thread).
    pub commit_ms: f64,
    pub reload: ReloadTimings,
    /// Editor-side main-thread time attributable to this commit: submitting it, showing its
    /// optimistic world, handling its result (with the writer); the whole synchronous
    /// `editor_action` otherwise.
    pub editor_main_ms: f64,
    /// Longest single main-thread piece of the above (one frame's share).
    pub editor_piece_max_ms: f64,
    /// Submission until the optimistic world was shown (writer only).
    pub staged_ms: f64,
}

/// Headless edit → live measurement through the editor's real write path
/// (`editor::Action::Write` → background writer or `Store::execute` → `App::reload_after`).
/// No window or GPU, so `reload.reconcile.prepare_ms` excludes pipeline creation; the GPU
/// path reports the same phases live in `observe` → `metrics.reload`.
pub fn measure_edit_live(
    world: World,
    registry: RuntimeRegistry,
    db: PathBuf,
    editor: bool,
    background: bool,
    writer: bool,
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
    let writer = editor && writer;
    if writer {
        app.enable_writer(None)?;
    }
    for edits in batches {
        let started = Instant::now();
        let expected = app.store.revision()?;
        let request = Request::Apply {
            expected_rev: Some(expected),
            edits,
        };
        if writer {
            app.editor_action(editor::Action::Write {
                request,
                select: None,
            })?;
            let submit_ms = ms(started);
            let (mut main_ms, mut piece_max, mut staged_ms) = (submit_ms, submit_ms, 0.0);
            // Frame loop stand-in: poll until the commit result was handled.
            while app.writes_pending() > 0 {
                let phase = Instant::now();
                let was_staging = app.writes.as_ref().is_some_and(|w| w.staging.is_some());
                if app.poll_writes(std::time::Duration::ZERO)? {
                    let piece = ms(phase);
                    main_ms += piece;
                    piece_max = piece_max.max(piece);
                    if was_staging && app.writes.as_ref().is_some_and(|w| w.staging.is_none()) {
                        staged_ms = ms(started);
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
            }
            let writes = app.writes.as_ref().unwrap();
            anyhow::ensure!(
                writes.failed == 0,
                "editor write failed: {}",
                app.editor.as_ref().unwrap().status
            );
            let commit_ms = writes.last_commit.map_or(0.0, |c| c.1);
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
                commit_ms,
                reload,
                editor_main_ms: main_ms,
                editor_piece_max_ms: piece_max,
                staged_ms,
            });
            continue;
        }
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
            editor_main_ms: submitted_ms,
            editor_piece_max_ms: submitted_ms,
            staged_ms: 0.0,
        });
    }
    app.close_writes(crate::writer::SHUTDOWN_FLUSH)?;
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
            writes: None,
            deferred_command: None,
            closing: false,
            audio: crate::audio::Output::new(),
        })
    }

    /// Editor commits go to a writer thread with its own connection to the database file.
    fn enable_writer(&mut self, gate: Option<Arc<std::sync::Mutex<()>>>) -> Result<()> {
        let registry = self.store.registry.clone();
        let writer = crate::writer::Writer::spawn(
            self.options.db.clone(),
            registry.clone(),
            self.store.latest_cache(),
            gate,
        )?;
        let chain_rev = self.editor.as_ref().map_or(0, |e| e.rev);
        self.writes = Some(Writes {
            writer,
            registry: Arc::new(registry),
            queue: Default::default(),
            chain_rev,
            staging: None,
            in_flight: None,
            next_seq: 0,
            committed: 0,
            failed: 0,
            discarded: 0,
            last_commit: None,
            recent_ms: Default::default(),
            commit_timeout: crate::writer::COMMIT_TIMEOUT,
            commit_grace: crate::writer::COMMIT_GRACE,
        });
        self.sync_write_stats();
        Ok(())
    }

    fn writes_pending(&self) -> usize {
        self.writes.as_ref().map_or(0, |w| w.queue.len())
    }

    fn sync_write_stats(&mut self) {
        if let (Some(writes), Some(editor)) = (&self.writes, &mut self.editor) {
            editor.saving = writes.queue.len();
            editor.writes = writes.observation();
        }
    }

    /// Queues an editor write (main thread: bookkeeping only). Refused with a visible
    /// reason when the queue is full or the editor cannot build on the pending state yet.
    fn submit_write(&mut self, kind: WriteKind) -> Result<()> {
        let writes = self.writes.as_mut().expect("writer enabled");
        if let Some(reason) = writes.busy() {
            return Err(axiom_core::Diagnostic::error("editor_saving", "/", reason));
        }
        let editor = self.editor.as_mut().unwrap();
        let chained = !writes.queue.is_empty();
        if !chained {
            writes.chain_rev = editor.rev;
            // Without queued writes the target is known now; nothing to undo is an error
            // right away, exactly as before.
            if let WriteKind::Restore { redo, .. } = kind {
                anyhow::ensure!(
                    editor.restore_target(redo).is_some(),
                    "nothing to {}",
                    if redo { "redo" } else { "undo" }
                );
            }
        }
        let seq = writes.next_seq;
        writes.next_seq += 1;
        let stage = match &kind {
            WriteKind::Write {
                request: Request::Apply { edits, .. },
                ..
            } => {
                let base = editor.world.clone();
                let edits = edits.clone();
                let registry = writes.registry.clone();
                let job = crate::jobs::Job::spawn(
                    "stage",
                    format!("write {seq}"),
                    crate::writer::STAGE_TIMEOUT,
                    move |_| crate::writer::stage(&base, &edits, &registry),
                )?;
                if let Some((_, stale)) = writes.staging.replace((seq, job)) {
                    stale.detach();
                }
                Stage::Staging
            }
            WriteKind::Write {
                request: Request::Preview { .. },
                ..
            } => Stage::Ready,
            _ => Stage::Barrier,
        };
        writes.queue.push_back(PendingWrite {
            seq,
            kind,
            chained,
            view_rev: editor.rev,
            stage,
            sent: None,
            replies: vec![],
        });
        let pending = writes.queue.len();
        editor.notice(format!("Saving… {pending} edit(s) pending"));
        self.dispatch_write()?;
        self.sync_write_stats();
        Ok(())
    }

    /// Sends the oldest queued write when nothing is in flight, resolving its CAS revision
    /// (and an undo/redo target) from the commits before it.
    fn dispatch_write(&mut self) -> Result<()> {
        let (Some(writes), Some(editor)) = (&mut self.writes, &mut self.editor) else {
            return Ok(());
        };
        if writes.in_flight.is_some() {
            return Ok(());
        }
        let Some(item) = writes.queue.front_mut() else {
            return Ok(());
        };
        let base = writes.chain_rev;
        let request = match &item.kind {
            WriteKind::Write { request, .. } => {
                let mut request = request.clone();
                if item.chained {
                    if let Request::Apply { expected_rev, .. }
                    | Request::Preview { expected_rev, .. } = &mut request
                    {
                        // Built on the optimistic world of the writes before it: expect
                        // the revision they produced (a foreign commit in between still
                        // conflicts). A request built on an older view keeps its own.
                        if *expected_rev == Some(item.view_rev) {
                            *expected_rev = Some(base);
                        }
                    }
                }
                Ok(request)
            }
            WriteKind::Restore { rev, redo } => {
                let target = if item.chained {
                    editor.restore_target(*redo)
                } else {
                    Some(*rev)
                };
                target
                    .map(|rev| Request::Undo {
                        rev,
                        expected_rev: Some(base),
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("nothing to {}", if *redo { "redo" } else { "undo" })
                    })
            }
            WriteKind::RestoreTo { rev } => Ok(Request::Undo {
                rev: *rev,
                expected_rev: Some(base),
            }),
            WriteKind::Resolve {
                id,
                approve,
                rebase,
            } => Ok(Request::ResolveProposal {
                id: *id,
                approve: *approve,
                rebase: *rebase,
            }),
        };
        let seq = item.seq;
        let sent = request.and_then(|request| {
            let what = crate::writer::describe(&request);
            writes.writer.send(seq, request.clone())?;
            item.sent = Some((request, base));
            writes.in_flight = Some(InFlight {
                seq,
                sent: Instant::now(),
                interrupted: None,
                what,
            });
            Ok(())
        });
        if let Err(error) = sent {
            // Could not even be sent (nothing to undo, writer gone): same as a failed commit.
            self.finish_write(crate::writer::Finished {
                seq,
                result: Err(error),
                ms: 0.0,
            })?;
        }
        Ok(())
    }

    /// Delivers staged worlds and commit results; once per frame (and while flushing).
    /// Never waits unless `wait` is non-zero. Returns whether anything was handled.
    fn poll_writes(&mut self, wait: std::time::Duration) -> Result<bool> {
        let Some(writes) = &mut self.writes else {
            return Ok(false);
        };
        let mut worked = false;
        let staged = writes
            .staging
            .as_mut()
            .and_then(|(seq, job)| job.poll().map(|outcome| (*seq, outcome)));
        if let Some((seq, outcome)) = staged {
            writes.staging = None;
            worked = true;
            self.staged(seq, outcome);
        }
        let mut wait = wait;
        while let Some(done) = self
            .writes
            .as_ref()
            .and_then(|w| w.writer.finished(std::mem::take(&mut wait)))
        {
            worked = true;
            self.finish_write(done)?;
            self.dispatch_write()?;
        }
        self.write_watchdog();
        self.sync_write_stats();
        Ok(worked)
    }

    fn staged(&mut self, seq: u64, outcome: crate::jobs::Outcome<Result<Arc<World>>>) {
        let (Some(writes), Some(editor)) = (&mut self.writes, &mut self.editor) else {
            return;
        };
        let Some(item) = writes.queue.iter_mut().find(|w| w.seq == seq) else {
            return;
        };
        match outcome {
            crate::jobs::Outcome::Done {
                value: Ok(world), ..
            } => {
                item.stage = Stage::Ready;
                let select = match &mut item.kind {
                    WriteKind::Write { select, .. } => select.take(),
                    _ => None,
                };
                retire(editor.apply_staged(world, select));
                let pending = writes.queue.len();
                editor.notice(format!("Saving… {pending} edit(s) pending"));
            }
            // The draft stays as typed; the commit (same validation) reports the error.
            _ => item.stage = Stage::Refused,
        }
    }

    /// One commit result, in submission order.
    fn finish_write(&mut self, done: crate::writer::Finished) -> Result<()> {
        let Some(writes) = &mut self.writes else {
            return Ok(());
        };
        if writes.queue.front().is_none_or(|w| w.seq != done.seq) {
            eprintln!("editor writer: result {} out of order; ignored", done.seq);
            return Ok(());
        }
        let mut item = writes.queue.pop_front().unwrap();
        if writes.in_flight.as_ref().is_some_and(|f| f.seq == done.seq) {
            writes.in_flight = None;
        }
        if writes.staging.as_ref().is_some_and(|(s, _)| *s == done.seq) {
            // Committed before its optimistic world arrived; the commit result is adopted.
            writes.staging.take().unwrap().1.detach();
        }
        let editor = self.editor.as_mut().unwrap();
        let what = item.kind.label();
        let (request, expected) = item.sent.take().unwrap_or((Request::Verify {}, 0));
        match done.result {
            Ok(value) => {
                writes.committed += 1;
                let rev = value["rev"].as_i64();
                if let Some(rev) = rev {
                    writes.last_commit = Some((rev, done.ms));
                }
                writes.recent_ms.push_back(done.ms);
                if writes.recent_ms.len() > 16 {
                    writes.recent_ms.pop_front();
                }
                let mut select = None;
                match item.kind {
                    WriteKind::Write { select: s, .. } => {
                        let apply = matches!(request, Request::Apply { .. });
                        editor.record(request, &value);
                        if apply {
                            if value["changed"] == true {
                                editor.record_apply(expected);
                            }
                            writes.chain_rev = rev.unwrap_or(writes.chain_rev);
                        }
                        select = s;
                    }
                    WriteKind::Restore { redo, .. } => {
                        editor.record_restore(expected, redo);
                        editor.record(request, &value);
                        writes.chain_rev = rev.unwrap_or(writes.chain_rev);
                    }
                    WriteKind::RestoreTo { .. } => {
                        editor.record(request, &value);
                        writes.chain_rev = rev.unwrap_or(writes.chain_rev);
                    }
                    WriteKind::Resolve { id, approve, .. } => {
                        editor.proposal_signature = None;
                        if approve {
                            editor.record_apply(expected);
                            editor.notice(format!(
                                "Approved proposal #{id} · committed revision {}",
                                value["rev"]
                            ));
                            writes.chain_rev = rev.unwrap_or(writes.chain_rev);
                        } else {
                            editor.notice(format!("Rejected proposal #{id}; nothing was written"));
                        }
                    }
                }
                editor.needs_history = true;
                let idle = writes.queue.is_empty();
                let chain_rev = writes.chain_rev;
                if !self.closing {
                    if idle {
                        // Adopt the committed world (same content as the optimistic one).
                        self.reload_after(Some(chain_rev))?;
                        if let Some(id) = select {
                            self.editor.as_mut().unwrap().select(Some(id));
                        }
                    } else {
                        // The runtime follows each commit; the editor keeps showing the
                        // optimistic world of the writes still queued.
                        self.reload()?;
                    }
                }
                self.sync_write_stats();
                let observation = self.editor.as_ref().unwrap().observation();
                for (reply, mut response) in item.replies {
                    response["editor"] = observation.clone();
                    let _ = reply.send(response);
                }
            }
            Err(error) => {
                writes.failed += 1;
                let shown = matches!(item.stage, Stage::Ready)
                    && matches!(
                        item.kind,
                        WriteKind::Write {
                            request: Request::Apply { .. },
                            ..
                        }
                    );
                // Everything queued behind it was built on its optimistic world.
                let discarded: Vec<PendingWrite> = writes.queue.drain(..).collect();
                if let Some((_, job)) = writes.staging.take() {
                    job.detach();
                }
                writes.discarded += discarded.len() as u64;
                let shown = shown || discarded.iter().any(|w| w.stage == Stage::Ready);
                let lost: Vec<String> = discarded.iter().map(|w| w.kind.label()).collect();
                let response = axiom_core::error_response(&error);
                for (reply, _) in item.replies {
                    let _ = reply.send(response.clone());
                }
                let skipped = axiom_core::error_response(&anyhow::anyhow!(
                    "not saved: an earlier queued editor write failed"
                ));
                for w in discarded {
                    for (reply, _) in w.replies {
                        let _ = reply.send(skipped.clone());
                    }
                }
                let chain_rev = writes.chain_rev;
                if !self.closing {
                    // Back to what the database holds: the commits that succeeded, or a
                    // foreign revision that made this one conflict (never overwritten).
                    let (latest, world) = self.store.current_shared()?;
                    let editor = self.editor.as_mut().unwrap();
                    if latest == editor.rev {
                        if shown {
                            retire(editor.revert_staged(world));
                        }
                        self.reload_after(None)?;
                    } else {
                        // Our earlier commits (adopted as local) or a foreign revision
                        // (adopted like any external one; its Changes list is computed
                        // against what the editor showed).
                        drop(world);
                        self.reload_after(Some(chain_rev))?;
                    }
                }
                self.sync_write_stats();
                let editor = self.editor.as_mut().unwrap();
                if !lost.is_empty() {
                    editor.problem(&axiom_core::Diagnostic::error(
                        "write_discarded",
                        "/",
                        format!(
                            "{} queued edit(s) built on a failed edit were not saved: {}",
                            lost.len(),
                            lost.join("; ")
                        ),
                    ));
                }
                editor.problem(&error);
                if !matches!(item.stage, Stage::Refused) {
                    editor.set_error(format!("Not saved ({what}): {error:#}"));
                }
                if self.closing {
                    eprintln!("editor writer: not saved ({what}): {error:#}");
                }
            }
        }
        Ok(())
    }

    /// Interrupts a commit over its deadline; detaches a writer that does not answer.
    fn write_watchdog(&mut self) {
        let Some(writes) = &mut self.writes else {
            return;
        };
        let Some(flight) = &mut writes.in_flight else {
            return;
        };
        match flight.interrupted {
            None if flight.sent.elapsed() >= writes.commit_timeout => {
                eprintln!(
                    "editor writer: {} exceeded {} ms; interrupting",
                    flight.what,
                    writes.commit_timeout.as_millis()
                );
                writes.writer.interrupt();
                flight.interrupted = Some(Instant::now());
            }
            Some(at) if at.elapsed() >= writes.commit_grace => {
                let what = flight.what.clone();
                let elapsed = flight.sent.elapsed();
                let writes = self.writes.take().unwrap();
                writes.writer.abort();
                let pending = writes.queue.len();
                for w in &writes.queue {
                    for (reply, _) in &w.replies {
                        let _ = reply.send(axiom_core::error_response(&anyhow::anyhow!(
                            "editor writer stalled; this edit was not confirmed"
                        )));
                    }
                }
                let message = format!(
                    "Editor writer stalled: {what} did not finish within {:.1} s and was detached; it may still land (check History). {} queued edit(s) not confirmed. Further edits commit on the main thread.",
                    elapsed.as_secs_f64(),
                    pending.saturating_sub(1)
                );
                eprintln!("{message}");
                // Never wait for the stuck thread here.
                retire(writes);
                if let Some(editor) = &mut self.editor {
                    editor.saving = 0;
                    editor.writes = serde_json::json!({"state":"stalled","detail":message});
                    if let Ok((latest, world)) = self.store.current_shared() {
                        if latest == editor.rev {
                            retire(editor.revert_staged(world));
                        }
                    }
                    let _ = self.reload_after(None);
                    if let Some(editor) = &mut self.editor {
                        editor.problem(&axiom_core::Diagnostic::error(
                            "writer_stalled",
                            "/",
                            message,
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    /// Flushes queued editor writes on shutdown (bounded), then joins the writer with the
    /// rest of the budget. Unconfirmed edits are reported, never dropped silently.
    fn close_writes(&mut self, limit: std::time::Duration) -> Result<()> {
        self.closing = true;
        let Some(writes) = &self.writes else {
            return Ok(());
        };
        let started = Instant::now();
        let queued = writes.queue.len();
        while self.writes_pending() > 0 && started.elapsed() < limit {
            let left = limit.saturating_sub(started.elapsed());
            self.poll_writes(left.min(std::time::Duration::from_millis(50)))?;
        }
        let Some(mut writes) = self.writes.take() else {
            anyhow::bail!("editor writer stalled during shutdown; see the messages above");
        };
        let unconfirmed = writes.queue.len();
        let joined = writes
            .writer
            .shutdown(limit.saturating_sub(started.elapsed()));
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        if queued > 0 || unconfirmed > 0 || !joined {
            eprintln!(
                "editor writer: flushed {} of {queued} pending edit(s) in {ms:.0} ms; {} commit(s) this session, {} failed",
                queued - unconfirmed,
                writes.committed,
                writes.failed
            );
        }
        if unconfirmed > 0 || !joined {
            let message = format!(
                "editor writer: {unconfirmed} edit(s) not confirmed within {:.0} s; writer {}",
                limit.as_secs_f64(),
                if joined {
                    "joined"
                } else {
                    "detached (join timed out)"
                }
            );
            writes.writer.abort();
            retire(writes);
            anyhow::bail!(message);
        }
        Ok(())
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
            arg,
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
                Some(id) => Some(editor.run_command_with(id, arg.as_deref())?),
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
        if self.closing {
            return Ok(());
        }
        let latest = self.store.revision()?;
        // While editor writes are queued the editor shows their optimistic world; it adopts
        // the database again when the queue drains (`finish_write`). The runtime follows.
        let writing = self.writes_pending() > 0;
        if (writing || self.editor.as_ref().is_none_or(|e| e.rev == latest))
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
        if let Some(editor) = self.editor.as_mut().filter(|_| !writing) {
            let phase = Instant::now();
            let copy = world.clone();
            timings.editor_clone_ms = ms(phase);
            let phase = Instant::now();
            let local = local_revision == Some(rev);
            if local && self.writes.is_some() {
                // This editor's own queued writes landed: keep undo, drafts and drags.
                if let Some(previous) = editor.commit_landed(rev, copy) {
                    retire(previous);
                }
            } else if self.background_activation {
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
        if self.writes.is_some() {
            // Every editor commit goes to the background writer, in order.
            let kind = match action {
                editor::Action::Write { request, select } => WriteKind::Write { request, select },
                editor::Action::Restore { rev, redo } => WriteKind::Restore { rev, redo },
                editor::Action::RestoreTo { rev } => WriteKind::RestoreTo { rev },
                editor::Action::ResolveProposal {
                    id,
                    approve,
                    rebase,
                } => WriteKind::Resolve {
                    id,
                    approve,
                    rebase,
                },
                other => return self.editor_action_now(other),
            };
            return self.submit_write(kind);
        }
        self.editor_action_now(action)
    }

    /// Editor actions on this thread (all of them without a background writer).
    fn editor_action_now(&mut self, action: editor::Action) -> Result<()> {
        let editor = self.editor.as_mut().unwrap();
        match action {
            editor::Action::Play => self.editor_play(true),
            editor::Action::Quit => {
                self.quit = true;
                Ok(())
            }
            editor::Action::PlayClip {
                capability,
                action,
                args,
            } => {
                self.editor_play(true)?;
                let command = axiom_core::gameplay::RuntimeCommand::Call {
                    capability,
                    action,
                    args,
                };
                if let Err(error) = self.runtime.command(&command) {
                    self.editor.as_mut().unwrap().problem(&error);
                }
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
        let saving = self.writes_pending();
        if play && saving > 0 {
            return Err(axiom_core::Diagnostic::error(
                "editor_saving",
                "/play",
                format!("{saving} edit(s) are still saving; Play starts from committed revisions, try again when they finish"),
            ));
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
    /// One bridge runtime request. Returns it when it has to wait for the editor write queue
    /// (it is retried first on the next frame, keeping the order of later requests).
    fn handle_command(
        &mut self,
        command: crate::bridge::RuntimeRequest,
    ) -> Option<crate::bridge::RuntimeRequest> {
        if std::time::Instant::now() >= command.deadline {
            return None;
        }
        if let Request::EditorView {
            command: run, play, ..
        } = &command.request
        {
            let busy = self.writes.as_ref().is_some_and(|w| w.busy().is_some());
            if (run.is_some() && busy) || (*play == Some(true) && self.writes_pending() > 0) {
                return Some(command);
            }
        }
        let editor_view = matches!(command.request, Request::EditorView { .. });
        let before = self.writes.as_ref().map_or(0, |w| w.next_seq);
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
        match result {
            Ok(response) => {
                // An editor command that queued a write answers when that write is committed
                // (or fails), as it did when commits ran on this thread.
                match self
                    .writes
                    .as_mut()
                    .and_then(|w| w.queue.back_mut().filter(|last| last.seq >= before))
                {
                    Some(last) => last.replies.push((command.reply, response)),
                    None => {
                        let _ = command.reply.send(response);
                    }
                }
            }
            Err(error) => {
                let _ = command.reply.send(axiom_core::error_response(&error));
            }
        }
        None
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        // The frame profile includes write results and bridge requests handled here.
        let mut phases = FramePhases::start(Instant::now());
        self.poll_writes(std::time::Duration::ZERO)?;
        for _ in 0..32 {
            let command = match self.deferred_command.take() {
                Some(command) => command,
                None => match self.commands.try_recv() {
                    Ok(command) => command,
                    Err(_) => break,
                },
            };
            if let Some(waiting) = self.handle_command(command) {
                self.deferred_command = Some(waiting);
                break;
            }
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
        phases.mark("writes_bridge");
        let poll = self.poll_elapsed >= 0.25;
        if poll {
            self.poll_elapsed = 0.0;
            // While editor writes are queued, each commit result moves the runtime on (after
            // the writer cached the decoded revision); polling meanwhile could race that
            // cache and decode the world here. A foreign revision then conflicts the next
            // queued write and is adopted when the queue drains.
            if self.writes_pending() == 0 {
                self.reload()?;
            }
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
        // Interactors to their focused interactables (same overlay switch).
        let mut focus_lines: Vec<([f32; 2], [f32; 2])> = vec![];
        if let (true, Some(interact), Some(view)) = (
            !editing && self.editor.as_ref().is_some_and(|e| e.overlay),
            self.runtime.system::<crate::interact::InteractSystem>(),
            crate::nameplate::active_view(&self.runtime.state),
        ) {
            let screen = self.ctx.screen_rect();
            let bounds = [screen.min.x, screen.min.y, screen.width(), screen.height()];
            for (a, b) in interact.focus_lines(&self.runtime.state) {
                if let (Some(a), Some(b)) = (view.project(a, bounds), view.project(b, bounds)) {
                    focus_lines.push((a, b));
                }
            }
        }
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
                    for ([ax, ay], [bx, by]) in &focus_lines {
                        painter.line_segment(
                            [egui::pos2(*ax, *ay), egui::pos2(*bx, *by)],
                            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 205, 90)),
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
            self.runtime
                .system::<crate::spatial_audio::AudioSpatialSystem>(),
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
        let acquire = self.renderer.as_ref().unwrap().last_acquire;
        phases.note(
            "render_present_acquire_wait",
            acquire.as_secs_f64() * 1000.0,
        );
        // A background build usually completes while this frame waits for the swapchain;
        // adopting here instead of next frame saves one frame of edit -> live latency.
        self.poll_activation()?;
        self.poll_changes();
        // Likewise a commit that finished during the present.
        self.poll_writes(std::time::Duration::ZERO)?;
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
            if let Some(editor) = &mut self.editor {
                // Asset thumbnails render off-screen on this device (from their worker).
                editor.set_gpu(renderer.device.clone(), renderer.queue.clone());
            }
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

    // ---- Editor writes on the background writer ----

    /// Editor app on a file database (the writer needs its own connection) with the writer
    /// enabled; `gate` lets a test hold commits in the queue.
    fn writer_app(name: &str, gate: Option<Arc<std::sync::Mutex<()>>>) -> App {
        let dir = std::env::temp_dir().join(format!("axiom-editor-writes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join(name);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
        }
        let world =
            serde_json::from_str(include_str!("../../../content/two-cameras/world.json")).unwrap();
        let registry = crate::installed_registry();
        let store = Store::open_with(&db, registry.authoring.clone(), world).unwrap();
        let mut app = App::new(
            RunOptions {
                mode: RunMode::Editor {
                    selected: Some("box".into()),
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
        )
        .unwrap();
        app.background_activation = true;
        app.enable_writer(gate).unwrap();
        app
    }

    /// Holds `gate` on another thread (the writer then waits before each commit) until the
    /// returned sender is dropped.
    fn hold(gate: &Arc<std::sync::Mutex<()>>) -> std::sync::mpsc::Sender<()> {
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let (locked, ready) = std::sync::mpsc::channel();
        let gate = gate.clone();
        std::thread::spawn(move || {
            let _held = gate.lock().unwrap();
            locked.send(()).unwrap();
            let _ = wait.recv();
        });
        ready.recv().unwrap();
        release
    }

    fn move_box(app: &App, x: f32) -> editor::Action {
        editor::Action::Write {
            request: Request::Apply {
                expected_rev: Some(app.editor.as_ref().unwrap().rev),
                edits: vec![Edit::Core(axiom_core::CoreEdit::SetField {
                    target: axiom_core::local::Target::Entity { id: "box".into() },
                    field: "/components/core.transform/position".into(),
                    value: json!([x, 1.0, 0.0]),
                })],
            },
            select: None,
        }
    }

    fn box_x(world: &World) -> f64 {
        world.entity("box").unwrap().components["core.transform"]["position"][0]
            .as_f64()
            .unwrap()
    }

    /// Frame-loop stand-in: polls until the optimistic world of the last write is shown.
    fn wait_ready(app: &mut App) {
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while app.writes.as_ref().unwrap().busy().is_some() {
            app.poll_writes(std::time::Duration::ZERO).unwrap();
            assert!(Instant::now() < deadline, "write never staged");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn drain(app: &mut App) {
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while app.writes_pending() > 0 {
            app.poll_writes(std::time::Duration::from_millis(5))
                .unwrap();
            assert!(Instant::now() < deadline, "writes never drained");
        }
    }

    #[test]
    fn queued_editor_writes_commit_in_order_without_blocking_the_main_thread() {
        let gate = Arc::new(std::sync::Mutex::new(()));
        let mut app = writer_app("order.sqlite", Some(gate.clone()));
        let base = app.editor.as_ref().unwrap().rev;
        let held = hold(&gate);
        for x in [1.0, 2.0, 3.0] {
            let started = Instant::now();
            let action = move_box(&app, x);
            app.editor_action(action).unwrap();
            // The commit waits on the writer; submitting never does (debug build bound).
            assert!(started.elapsed() < std::time::Duration::from_millis(250));
            wait_ready(&mut app);
            // Later edits are built on the optimistic world of the queued ones.
            assert_eq!(box_x(&app.editor.as_ref().unwrap().world), x as f64);
        }
        assert_eq!(app.store.revision().unwrap(), base, "nothing committed yet");
        let observed = app.observation().metrics["editor"].clone();
        assert_eq!(observed["pending_commits"], 3);
        assert_eq!(observed["authoring_rev"], base);
        assert_eq!(observed["writes"]["in_flight"]["interrupted"], false);
        assert!(app.editor.as_ref().unwrap().status.contains("Saving"));
        drop(held);
        drain(&mut app);
        // Same order, consecutive revisions, each with exactly its own change.
        assert_eq!(app.store.revision().unwrap(), base + 3);
        for (k, x) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            let rev = base + 1 + k as i64;
            let diff = app
                .store
                .execute(Request::Diff {
                    from_rev: rev - 1,
                    to_rev: rev,
                    offset: 0,
                    limit: Some(8),
                })
                .unwrap();
            let moved = diff["changes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| {
                    c["path"]
                        .as_str()
                        .unwrap()
                        .ends_with("/core.transform/position/0")
                })
                .unwrap_or_else(|| panic!("{diff}"));
            assert_eq!(moved["after"], json!(x), "{diff}");
        }
        let history = app
            .store
            .execute(Request::History { limit: Some(3) })
            .unwrap();
        assert!(history["revisions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["reason"] == "apply"));
        let editor = app.editor.as_ref().unwrap();
        assert_eq!((editor.rev, editor.saving), (base + 3, 0));
        assert_eq!(box_x(&editor.world), 3.0);
        assert_eq!(editor.restore_target(false), Some(base + 2));
        let observed = app.observation().metrics["editor"].clone();
        assert_eq!(observed["pending_commits"], 0);
        assert_eq!(observed["writes"]["committed"], 3);
        assert!(observed["last_commit_ms"].as_f64().unwrap() > 0.0);
        // The runtime follows the last commit.
        app.finish_activation().unwrap();
        assert_eq!(app.runtime.rev, base + 3);
        // Undo goes through the same queue and restores the revision before the last apply.
        app.editor_action(editor::Action::Restore {
            rev: base + 2,
            redo: false,
        })
        .unwrap();
        drain(&mut app);
        let editor = app.editor.as_ref().unwrap();
        assert_eq!(editor.rev, base + 4);
        assert_eq!(box_x(&editor.world), 2.0);
        assert_eq!(editor.restore_target(true), Some(base + 3));
        app.close_writes(std::time::Duration::from_secs(10))
            .unwrap();
    }

    #[test]
    fn ai_commit_during_an_in_flight_editor_write_is_a_conflict_not_an_overwrite() {
        let gate = Arc::new(std::sync::Mutex::new(()));
        let mut app = writer_app("conflict.sqlite", Some(gate.clone()));
        let base = app.editor.as_ref().unwrap().rev;
        let held = hold(&gate);
        for x in [5.0, 6.0] {
            let action = move_box(&app, x);
            app.editor_action(action).unwrap();
            wait_ready(&mut app);
        }
        // An AI edit lands through another connection while the first write waits.
        let mut bridge = Store::open_with(
            &app.options.db,
            app.store.registry.clone(),
            World::empty("x"),
        )
        .unwrap();
        bridge
            .apply(Some(base), vec![Edit::set("title", json!("AI edit"))])
            .unwrap();
        drop(held);
        drain(&mut app);
        // Nothing of the editor's landed on top of it; both edits are reported.
        assert_eq!(app.store.revision().unwrap(), base + 1);
        let (_, world) = app.store.current().unwrap();
        assert_eq!(world.title, "AI edit");
        assert_eq!(box_x(&world), 0.0);
        let editor = app.editor.as_ref().unwrap();
        let codes = editor.problem_codes();
        assert_eq!(codes[0], "revision_conflict", "{codes:?}");
        assert_eq!(codes[1], "write_discarded", "{codes:?}");
        assert!(editor.error && editor.status.contains("Not saved"));
        // The editor shows the database again (the AI revision), not its optimistic world.
        assert_eq!(editor.rev, base + 1);
        assert_eq!(editor.world.title, "AI edit");
        assert_eq!(box_x(&editor.world), 0.0);
        assert_eq!(editor.saving, 0);
        let writes = &app.observation().metrics["editor"]["writes"];
        assert_eq!(
            (writes["failed"].clone(), writes["discarded"].clone()),
            (json!(1), json!(1))
        );
        // Recoverable: the next edit commits normally.
        let action = move_box(&app, 7.0);
        app.editor_action(action).unwrap();
        drain(&mut app);
        assert_eq!(app.store.revision().unwrap(), base + 2);
        assert_eq!(box_x(&app.store.current().unwrap().1), 7.0);
    }

    #[test]
    fn undo_queued_behind_a_pending_write_and_blocked_writes_say_why() {
        let gate = Arc::new(std::sync::Mutex::new(()));
        let mut app = writer_app("undo.sqlite", Some(gate.clone()));
        let base = app.editor.as_ref().unwrap().rev;
        let held = hold(&gate);
        let action = move_box(&app, 4.0);
        app.editor_action(action).unwrap();
        wait_ready(&mut app);
        // Ctrl+Z right after a drag: the step it undoes is still being committed.
        assert!(app.editor.as_ref().unwrap().restore_target(false).is_none());
        app.editor_action(editor::Action::Restore {
            rev: 0,
            redo: false,
        })
        .unwrap();
        // Nothing can be built on an undo before it commits: refused with a reason.
        let action = move_box(&app, 9.0);
        let refused = app.editor_action(action).unwrap_err();
        let response = axiom_core::error_response(&refused);
        assert_eq!(response["diagnostics"][0]["code"], "editor_saving");
        // Play waits for committed revisions too.
        assert!(app.editor_action(editor::Action::Play).is_err());
        drop(held);
        drain(&mut app);
        assert_eq!(app.store.revision().unwrap(), base + 2);
        let history = app
            .store
            .execute(Request::History { limit: Some(1) })
            .unwrap();
        assert_eq!(history["revisions"][0]["reason"], format!("restore:{base}"));
        let editor = app.editor.as_ref().unwrap();
        assert_eq!(box_x(&editor.world), 0.0);
        assert_eq!(editor.restore_target(true), Some(base + 1));
    }

    #[test]
    fn closing_flushes_queued_writes_and_reports_unconfirmed_ones() {
        let gate = Arc::new(std::sync::Mutex::new(()));
        let mut app = writer_app("flush.sqlite", Some(gate.clone()));
        let base = app.editor.as_ref().unwrap().rev;
        let held = hold(&gate);
        for x in [1.5, 2.5] {
            let action = move_box(&app, x);
            app.editor_action(action).unwrap();
            wait_ready(&mut app);
        }
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(held);
        });
        app.close_writes(std::time::Duration::from_secs(20))
            .unwrap();
        assert_eq!(app.store.revision().unwrap(), base + 2);
        assert_eq!(box_x(&app.store.current().unwrap().1), 2.5);

        // A writer that cannot finish in time: reported, never silently dropped.
        let mut app = writer_app("flush-timeout.sqlite", Some(gate.clone()));
        let held = hold(&gate);
        let action = move_box(&app, 8.0);
        app.editor_action(action).unwrap();
        let error = app
            .close_writes(std::time::Duration::from_millis(200))
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("1 edit(s) not confirmed"),
            "{error:#}"
        );
        drop(held);
        // The aborted writer refuses the edit that never started; nothing half-lands.
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(app.store.revision().unwrap(), base);
    }

    #[test]
    fn failed_editor_write_is_a_problem_and_the_queue_keeps_working() {
        let mut app = writer_app("failure.sqlite", None);
        let base = app.editor.as_ref().unwrap().rev;
        app.editor_action(editor::Action::Write {
            request: Request::Apply {
                expected_rev: Some(base),
                edits: vec![Edit::Core(axiom_core::CoreEdit::SetField {
                    target: axiom_core::local::Target::Entity {
                        id: "missing".into(),
                    },
                    field: "/label".into(),
                    value: json!("x"),
                })],
            },
            select: None,
        })
        .unwrap();
        drain(&mut app);
        assert_eq!(app.store.revision().unwrap(), base);
        let editor = app.editor.as_ref().unwrap();
        assert!(editor.error);
        assert_eq!(editor.problem_codes().len(), 1);
        assert_eq!(app.writes.as_ref().unwrap().failed, 1);
        // Refused while staging: the editor never showed it.
        assert_eq!(box_x(&editor.world), 0.0);
        let action = move_box(&app, 3.0);
        app.editor_action(action).unwrap();
        drain(&mut app);
        assert_eq!(app.store.revision().unwrap(), base + 1);
    }

    #[test]
    fn editor_view_command_replies_after_its_commit_and_waits_its_turn() {
        let gate = Arc::new(std::sync::Mutex::new(()));
        let mut app = writer_app("bridge.sqlite", Some(gate.clone()));
        let base = app.editor.as_ref().unwrap().rev;
        let entities = app.editor.as_ref().unwrap().world.entities.len();
        let held = hold(&gate);
        let command = |app: &mut App| {
            let (reply, response) = std::sync::mpsc::sync_channel(1);
            let request = crate::bridge::RuntimeRequest {
                deadline: Instant::now() + std::time::Duration::from_secs(30),
                request: Request::EditorView {
                    select: vec![],
                    focus: false,
                    dock: None,
                    capture: None,
                    play: None,
                    open: None,
                    command: Some("object.cube".into()),
                    arg: None,
                },
                reply,
            };
            (app.handle_command(request), response)
        };
        let (deferred, first) = command(&mut app);
        assert!(deferred.is_none());
        // While the first cube is being staged a second command waits for the next frame.
        let (deferred, second) = command(&mut app);
        let deferred = deferred.expect("postponed while staging");
        wait_ready(&mut app);
        assert!(app.handle_command(deferred).is_none());
        wait_ready(&mut app);
        assert!(first.try_recv().is_err(), "replied before the commit");
        drop(held);
        drain(&mut app);
        // Each reply is sent once its own write committed. The editor adopts revisions when
        // its queue drains, so the first reply names its commit in `writes`.
        let reply = first.recv().unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["editor"]["writes"]["last_commit_rev"], base + 1);
        let reply = second.recv().unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["editor"]["authoring_rev"], base + 2);
        assert_eq!(reply["editor"]["pending_commits"], 0);
        assert_eq!(app.store.revision().unwrap(), base + 2);
        assert_eq!(
            app.editor.as_ref().unwrap().world.entities.len(),
            entities + 2
        );
    }

    #[test]
    fn stalled_writer_is_detached_reported_and_editing_continues() {
        let gate = Arc::new(std::sync::Mutex::new(()));
        let mut app = writer_app("stall.sqlite", Some(gate.clone()));
        let base = app.editor.as_ref().unwrap().rev;
        {
            let writes = app.writes.as_mut().unwrap();
            writes.commit_timeout = std::time::Duration::from_millis(20);
            writes.commit_grace = std::time::Duration::from_millis(40);
        }
        let held = hold(&gate);
        let action = move_box(&app, 6.0);
        app.editor_action(action).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while app.writes.is_some() {
            app.poll_writes(std::time::Duration::ZERO).unwrap();
            assert!(Instant::now() < deadline, "watchdog never fired");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let editor = app.editor.as_ref().unwrap();
        assert_eq!(editor.problem_codes()[0], "writer_stalled");
        assert_eq!(editor.saving, 0);
        assert_eq!(box_x(&editor.world), 0.0, "optimistic world reverted");
        drop(held);
        std::thread::sleep(std::time::Duration::from_millis(200));
        // The detached writer refused the edit it had not started.
        assert_eq!(app.store.revision().unwrap(), base);
        // Editing continues on the main thread.
        let action = move_box(&app, 2.0);
        app.editor_action(action).unwrap();
        assert_eq!(app.store.revision().unwrap(), base + 1);
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
