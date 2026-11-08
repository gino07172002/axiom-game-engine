use crate::{
    bridge::Bridge,
    editor::{self, Editor},
    render::Renderer,
    ui::{self, Action, UiState},
    world::{ActionMap, Observation, RuntimeRegistry, WorldRuntime, FIXED_DT},
};
use anyhow::Result;
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

impl App {
    fn new(options: RunOptions, registry: RuntimeRegistry, store: Store) -> Result<Self> {
        let (rev, world) = store.current()?;
        let editor = match &options.mode {
            RunMode::Play => None,
            RunMode::Editor { selected } => {
                let mut editor = Editor::new(world.clone(), rev, selected.clone());
                editor.set_component_hints(&registry.authoring);
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
        } = request
        {
            let editor = self.editor.as_mut().ok_or_else(|| {
                anyhow::anyhow!("editor_view requires a running editor (axiom editor)")
            })?;
            anyhow::ensure!(select.len() <= 64, "editor_view selects at most 64 objects");
            editor.remote_view(&select, focus, dock.as_deref())?;
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
        let (rev, world) = self.store.current()?;
        if let Some(editor) = &mut self.editor {
            editor.update(rev, world.clone(), local_revision == Some(rev));
            if editor.previewing {
                // Play uses a pinned authoring revision and discards its simulation on exit.
                return Ok(());
            }
        }
        if rev != self.runtime.requested_rev {
            let renderer = &mut self.renderer;
            if let Err(error) = self.runtime.reconcile_checked(world, rev, |next| {
                if let Some(renderer) = renderer {
                    renderer.prepare_shaders(next.shaders())?;
                }
                Ok(())
            }) {
                self.ui.output.push(format!("ACTIVATION FAILED: {error:#}"));
                if let Some(editor) = &mut self.editor {
                    editor.set_error(format!(
                        "Revision {rev} committed; activation failed · {error:#}"
                    ));
                }
                return Ok(());
            }
            self.ui
                .output
                .push(format!("OK  runtime synchronized  rev={rev}"));
        }
        Ok(())
    }

    fn editor_action(&mut self, action: editor::Action) -> Result<()> {
        let editor = self.editor.as_mut().unwrap();
        match action {
            editor::Action::Play => self.editor_play(true),
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
            let result = self
                .reload()
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
        let window = self.window.as_ref().unwrap().clone();
        let raw = self.egui_state.as_mut().unwrap().take_egui_input(&window);
        let ctx = self.ctx.clone();
        let editing = self.editing();
        let mut actions = vec![];
        let mut editor_actions = vec![];
        let mut stop = false;
        let output = ctx.run(raw, |ctx| {
            if editing {
                editor_actions = self.editor.as_mut().unwrap().draw(ctx, &self.runtime);
            } else {
                actions = self.runtime.draw_ui(ctx, &mut self.ui);
                if self.editor.is_some() {
                    egui::Area::new(egui::Id::new("editor_stop"))
                        .anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
                            stop = ui.button("■ Stop Preview  F8").clicked();
                        });
                }
            }
        });
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
        if stop {
            self.editor_play(false)?;
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
        self.frames += 1;
        let final_frame = self.options.smoke_frames.is_some_and(|n| self.frames >= n);
        let pending = self.pending_capture.take();
        let capture_path = if final_frame {
            self.options.screenshot.as_deref()
        } else {
            pending.as_deref()
        };
        let frame = if editing {
            self.editor
                .as_ref()
                .unwrap()
                .render_frame(&mut self.runtime, ctx.screen_rect())
        } else {
            self.runtime.render_frame()
        };
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
        let mut obs = self.observation();
        obs.fps = self.fps_smooth;
        obs.frame_ms_cpu = now.elapsed().as_secs_f32() * 1000.0;
        obs.gpu = self.ui.gpu.clone();
        if let Ok(mut value) = self.observation.write() {
            *value = obs.clone();
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
}
