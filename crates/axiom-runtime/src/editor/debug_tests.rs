//! M8 debug tools driven by real egui input and the command registry: the Console, the Event
//! trace and the Profiler run the same commands as `editor_view {command}`, keep fixed-size
//! buffers and do no work while nothing happens.
use super::debug::{Level, CONSOLE_CAPACITY, TRACE_CAPACITY};
use super::{Action, Dock, Editor};
use crate::world::{ActionMap, WorldRuntime};
use axiom_core::{ContentPackage, World};
use egui::{vec2, Event, Modifiers, PointerButton, Pos2, Rect};

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &crate::installed_registry().authoring,
    )
    .unwrap()
    .world
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
}

impl Harness {
    fn new(world: World) -> Self {
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world, 1, None);
        editor.set_component_hints(&crate::installed_registry().authoring);
        Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
        }
    }
    fn run(&mut self, events: Vec<Event>) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1366.0, 768.0))),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        actions
    }
    fn frame(&mut self) {
        self.run(vec![]);
    }
    fn click(&mut self, key: &str) -> Vec<Action> {
        let at = self
            .editor
            .rects
            .get(key)
            .unwrap_or_else(|| panic!("{key} not drawn"))
            .center();
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut actions = self.run(vec![Event::PointerMoved(at), press(true)]);
        actions.extend(self.run(vec![press(false)]));
        actions
    }
    fn open(&mut self, tab: &str) {
        self.editor.run_command(&format!("window.{tab}")).unwrap();
        self.frame();
        self.frame();
    }
}

fn problem(path: &str) -> anyhow::Error {
    axiom_core::Diagnostic::error("test_problem", path, "the crate is broken")
}

#[test]
fn console_is_in_the_default_layout_and_logs_editor_messages_with_links() {
    let mut h = Harness::new(lab());
    h.frame();
    assert!(h.editor.dock_shows(Dock::Assets));
    assert!(super::layout::is_open(
        h.editor.layout.dock.as_ref().unwrap(),
        Dock::Console
    ));
    h.editor.notice("Saved something");
    h.editor.notice("Saved something");
    h.editor.set_error("A failure");
    h.editor
        .problem(&problem("/entities/@stage~1crate/components/render.mesh"));
    let c = &h.editor.debug.console;
    // The repeated notice is one entry ×2; problem() logs once (with its object).
    assert_eq!(c.len(), 3);
    assert_eq!((c.count(Level::Info), c.count(Level::Error)), (1, 2));
    let linked = c.get(3).unwrap();
    assert_eq!(linked.entity.as_deref(), Some("stage/crate"));
    assert_eq!(c.get(1).unwrap().repeat, 2);
    // The tab title counts errors; observe reports the last one.
    assert!(h.editor.tab_label(Dock::Console).ends_with("Console (2)"));
    let observed = h.editor.observation()["debug"]["console"].clone();
    assert_eq!(observed["errors"], 2);
    assert_eq!(observed["last_error"]["entity"], "stage/crate");
    // Clicking the linked row selects the object (= console.open SEQ).
    h.open("console");
    assert!(h.editor.dock_shows(Dock::Console));
    h.click("console:row:3");
    assert_eq!(h.editor.selected.as_deref(), Some("stage/crate"));
    let mut reference = Harness::new(lab());
    reference
        .editor
        .problem(&problem("/entities/@stage~1crate"));
    reference
        .editor
        .run_command_with("console.open", Some("1"))
        .unwrap();
    assert_eq!(reference.editor.selected.as_deref(), Some("stage/crate"));
    // An entry without an object, or one no longer kept, is an error (not a no-op).
    assert!(h
        .editor
        .run_command_with("console.open", Some("1"))
        .is_err());
    assert!(h
        .editor
        .run_command_with("console.open", Some("999"))
        .is_err());
    assert!(h
        .editor
        .run_command_with("console.open", Some("x"))
        .is_err());
    // Without a number (menu, palette) it opens the Console to pick an entry.
    h.editor.run_command("window.history").unwrap();
    h.editor.run_command("console.open").unwrap();
    assert!(h.editor.dock_shows(Dock::Console));
}

#[test]
fn console_filters_by_level_source_and_text_like_the_command() {
    let mut h = Harness::new(lab());
    h.open("console");
    h.editor.notice("alpha info");
    h.editor.set_error("beta error");
    h.editor
        .debug
        .game_output(&["ERROR game broke".to_owned(), "saved quick".into()], 7);
    h.frame();
    // Level toggle (click) = console.filter levels=…
    h.click("console:level:info");
    let mut reference = Harness::new(lab());
    reference
        .editor
        .run_command_with("console.filter", Some("levels=warn,error"))
        .unwrap();
    assert_eq!(
        h.editor.debug.console.levels,
        reference.editor.debug.console.levels
    );
    assert_eq!(h.editor.debug.console.levels, [false, true, true]);
    h.frame();
    assert!(h.editor.rects.contains_key("console:row:2"));
    assert!(!h.editor.rects.contains_key("console:row:1"), "info hidden");
    // Source drop-down = console.filter source=game.
    h.click("console:source");
    h.frame();
    h.click("console:source:game");
    assert_eq!(
        h.editor.debug.console.source,
        Some(super::debug::Source::Game)
    );
    h.frame();
    assert!(h.editor.rects.contains_key("console:row:3"));
    assert!(!h.editor.rects.contains_key("console:row:2"));
    // Text (case-insensitive), then reset.
    h.editor
        .run_command_with(
            "console.filter",
            Some("levels=info,warn,error source=all text=SAVED"),
        )
        .unwrap();
    h.frame();
    assert!(h.editor.rects.contains_key("console:row:4"));
    assert!(!h.editor.rects.contains_key("console:row:3"));
    for bad in ["levels=loud", "source=disk", "colour=red"] {
        assert!(
            h.editor
                .run_command_with("console.filter", Some(bad))
                .is_err(),
            "{bad}"
        );
    }
    h.editor
        .run_command_with("console.filter", Some("reset"))
        .unwrap();
    assert_eq!(h.editor.debug.console.levels, [true; 3]);
    assert!(h.editor.debug.console.text.is_empty());
    // Clear (button) = console.clear.
    h.frame();
    h.click("console:clear");
    assert_eq!(h.editor.debug.console.len(), 0);
}

#[test]
fn console_ring_is_bounded_and_idle_frames_do_no_work() {
    let mut h = Harness::new(lab());
    h.open("console");
    let capacity = h.editor.debug.console.capacity();
    for i in 0..CONSOLE_CAPACITY * 3 {
        h.editor.notice(format!("message {i}"));
    }
    let c = &h.editor.debug.console;
    assert_eq!(c.len(), CONSOLE_CAPACITY);
    assert_eq!(c.dropped as usize, CONSOLE_CAPACITY * 2);
    assert_eq!(c.capacity(), capacity, "no growth past the fixed capacity");
    assert!(c.get(1).is_none());
    h.frame();
    let builds = h.editor.debug.console.builds;
    for _ in 0..60 {
        h.frame();
        h.editor.debug.collect(&h.runtime, false);
    }
    assert_eq!(
        h.editor.debug.console.builds, builds,
        "view not rebuilt per frame"
    );
    assert_eq!(h.editor.debug.console.len(), CONSOLE_CAPACITY);
    assert_eq!(h.editor.debug.console.capacity(), capacity);
}

#[test]
fn event_trace_groups_play_events_by_tick_and_links_agents() {
    let mut h = Harness::new(town());
    h.editor.previewing = true;
    h.open("trace");
    // town-dance: walk to Lumi (her FSM greets), then the dance choice.
    let walk = ActionMap(
        [("move_z".to_owned(), 1.0), ("move_x".to_owned(), 0.45)]
            .into_iter()
            .collect(),
    );
    let capacity = h.editor.debug.trace.capacity();
    let interact = ActionMap([("interact".to_owned(), 1.0)].into_iter().collect());
    for tick in 0..330 {
        let input = match tick {
            0..250 => walk.clone(),
            300..302 => interact.clone(),
            _ => ActionMap::default(),
        };
        h.runtime.step(&input);
        h.editor.debug.collect(&h.runtime, true);
    }
    h.runtime
        .command(&axiom_core::gameplay::RuntimeCommand::Choose {
            node: "lumi".into(),
            choice: "dance".into(),
        })
        .unwrap();
    for _ in 0..5 {
        h.runtime.step(&ActionMap::default());
        h.editor.debug.collect(&h.runtime, true);
    }
    let t = &h.editor.debug.trace;
    assert!(t.len() > 0);
    let kinds: Vec<&str> = (1..=t.len() as u64)
        .filter_map(|s| t.get(s))
        .map(|e| match e.kind {
            super::debug::TraceKind::Event => "event",
            super::debug::TraceKind::Rule => "rule",
            super::debug::TraceKind::Narrative => "narrative",
            super::debug::TraceKind::Package => "package",
            super::debug::TraceKind::State => "state",
        })
        .collect();
    assert!(kinds.contains(&"state"), "{kinds:?}");
    assert!(kinds.contains(&"narrative"), "{kinds:?}");
    let state = (1..=t.len() as u64)
        .filter_map(|s| t.get(s))
        .find(|e| e.kind == super::debug::TraceKind::State)
        .unwrap();
    assert_eq!(state.entity.as_deref(), Some("npc/lumi"));
    let state_seq = state.seq;
    let ticks: Vec<u64> = (1..=t.len() as u64)
        .filter_map(|s| t.get(s))
        .map(|e| e.tick)
        .collect();
    assert!(ticks.windows(2).all(|w| w[0] <= w[1]), "ordered by tick");
    // Drawn grouped under tick headers (the view follows the newest rows; the FSM filter
    // brings the first state change on screen); clicking an FSM row selects the agent.
    h.editor
        .run_command_with("trace.filter", Some("kinds=state"))
        .unwrap();
    h.frame();
    h.frame();
    assert!(h
        .editor
        .rects
        .contains_key(&format!("trace:row:{state_seq}")));
    h.click(&format!("trace:row:{state_seq}"));
    assert_eq!(h.editor.selected.as_deref(), Some("npc/lumi"));
    h.editor
        .run_command_with("trace.filter", Some("reset"))
        .unwrap();
    h.frame();
    // Nothing new: collecting again adds nothing and the rows are not rebuilt.
    let (len, builds) = (h.editor.debug.trace.len(), h.editor.debug.trace.builds);
    for _ in 0..30 {
        h.editor.debug.collect(&h.runtime, true);
        h.frame();
    }
    assert_eq!(h.editor.debug.trace.len(), len);
    assert_eq!(h.editor.debug.trace.builds, builds);
    assert_eq!(h.editor.debug.trace.capacity(), capacity);
    assert!(capacity >= TRACE_CAPACITY);
    // Pause (button) = trace.pause: new events are counted as skipped, not kept.
    h.click("trace:pause");
    assert!(h.editor.debug.trace.paused);
    for _ in 0..200 {
        h.runtime.step(&walk);
        h.editor.debug.collect(&h.runtime, true);
    }
    assert_eq!(h.editor.debug.trace.len(), len);
    h.editor.run_command("trace.pause").unwrap();
    assert!(!h.editor.debug.trace.paused);
    // Kind toggle (click) = trace.filter kinds=…
    h.frame();
    h.click("trace:kind:state");
    assert!(!h.editor.debug.trace.kinds[super::debug::TraceKind::State as usize]);
    h.frame();
    assert!(!h
        .editor
        .rects
        .contains_key(&format!("trace:row:{state_seq}")));
    h.editor
        .run_command_with("trace.filter", Some("kinds=all text=lumi"))
        .unwrap();
    assert!(h
        .editor
        .run_command_with("trace.filter", Some("kinds=bogus"))
        .is_err());
    h.editor.run_command("trace.clear").unwrap();
    assert_eq!(h.editor.debug.trace.len(), 0);
    // Gameplay events also reached the Console (source event).
    assert!(h.editor.debug.console.len() > 0);
}

#[test]
fn profiler_measures_only_while_visible_and_reports_per_system_time() {
    let mut h = Harness::new(town());
    h.frame();
    assert!(!h.editor.debug.profiler.wants(), "hidden: nothing measured");
    h.open("profiler");
    assert!(h.editor.debug.profiler.wants());
    // The host feeds phases and system times only then.
    h.runtime.set_system_profiling(true);
    for _ in 0..3 {
        h.runtime.step(&ActionMap::default());
    }
    h.editor.previewing = true;
    h.editor.debug.profiler.take_systems(&mut h.runtime);
    h.editor.debug.profiler.push_frame(
        &[
            ("simulate", 1.5),
            ("render_present", 4.0),
            ("render_present_acquire_wait", 3.0),
        ],
        9.0,
        3,
    );
    h.frame();
    assert_eq!(h.editor.debug.profiler.len, 1);
    assert!(h.editor.rects.contains_key("profiler:graph"));
    let observed = h.editor.observation()["debug"]["profiler"].clone();
    assert_eq!(observed["measuring"], true);
    // Pause (button) = profiler.pause; hiding the tab stops measuring.
    h.click("profiler:pause");
    assert!(h.editor.debug.profiler.paused && !h.editor.debug.profiler.wants());
    h.editor.run_command("profiler.pause").unwrap();
    h.editor.run_command("window.console").unwrap();
    h.frame();
    assert!(!h.editor.debug.profiler.wants());
    // Turning runtime profiling off drops the per-system buffer.
    h.runtime.set_system_profiling(false);
    let mut called = false;
    h.runtime.take_system_profile(|_, _| called = true);
    assert!(!called);
}

#[test]
fn profiler_shows_gpu_timestamp_times_or_says_why_not() {
    use crate::gpu_timing::{GpuFrame, Section};
    let mut h = Harness::new(town());
    h.open("profiler");
    // Unsupported: the reason, no GPU table, observe says why.
    h.editor.debug.profiler.gpu_timing = Some(Err("the adapter has no TIMESTAMP_QUERY"));
    h.frame();
    assert!(!h.editor.rects.contains_key("profiler:gpu"));
    let gpu = h.editor.observation()["debug"]["profiler"]["gpu"].clone();
    assert_eq!(gpu["available"], false);
    assert_eq!(gpu["reason"], "the adapter has no TIMESTAMP_QUERY");
    // Supported: measured frames (from the renderer's readbacks) fill the GPU table.
    h.editor.debug.profiler.gpu_timing = Some(Ok(()));
    let mut frame = GpuFrame::default();
    frame.ms[Section::World as usize] = 2.0;
    frame.ms[Section::Ui as usize] = 0.5;
    frame.total = 2.6;
    h.editor.debug.profiler.push_gpu(&frame);
    frame.ms[Section::World as usize] = 4.0;
    frame.total = 4.6;
    h.editor.debug.profiler.push_gpu(&frame);
    h.editor.debug.profiler.gpu_dropped = 3;
    h.frame();
    assert!(h.editor.rects.contains_key("profiler:gpu"));
    let gpu = h.editor.observation()["debug"]["profiler"]["gpu"].clone();
    assert_eq!(gpu["source"], "timestamp_queries");
    assert_eq!(
        (gpu["frames"].as_u64(), gpu["dropped"].as_u64()),
        (Some(2), Some(3))
    );
    assert_eq!(gpu["mean_ms"]["world"], 3.0);
    assert_eq!(gpu["mean_ms"]["ui"], 0.5);
    assert!(
        gpu["mean_ms"].get("shadows").is_none(),
        "unmeasured groups are left out"
    );
    assert_eq!(
        (gpu["total_mean_ms"].as_f64(), gpu["total_max_ms"].as_f64()),
        (Some(3.6), Some(4.6))
    );
    // The CPU side is summarized next to it (frame minus swapchain wait).
    h.editor.debug.profiler.push_frame(
        &[("simulate", 1.0), ("render_present_acquire_wait", 2.0)],
        5.0,
        1,
    );
    let cpu = h.editor.observation()["debug"]["profiler"]["cpu"].clone();
    assert_eq!(cpu["frame_mean_ms"], 3.0, "{cpu}");
    assert_eq!(cpu["phase_mean_ms"]["simulate"], 1.0);
    // Paused: nothing is added; reset clears the GPU window too.
    h.editor.run_command("profiler.pause").unwrap();
    h.editor.debug.profiler.push_gpu(&frame);
    assert_eq!(h.editor.debug.profiler.gpu_len, 2);
    h.editor.run_command("profiler.reset").unwrap();
    assert_eq!(h.editor.debug.profiler.gpu_len, 0);
}

#[test]
fn record_command_arms_play_and_refuses_bad_names() {
    let dir = std::env::temp_dir().join(format!("axiom-m8-record-{}", std::process::id()));
    let mut h = Harness::new(lab());
    h.editor.debug.record_dir = dir.clone();
    // Outside Play: arms a recording and asks the host to Play.
    let actions = h
        .editor
        .run_command_with("play.record", Some("take-1"))
        .unwrap();
    assert!(matches!(actions.as_slice(), [Action::Play]));
    let path = h.editor.debug.recorder.as_ref().unwrap().path.clone();
    assert_eq!(path, dir.join("take-1.scenario.json"));
    // While recording, the same command stops it (Play continues).
    h.editor.previewing = true;
    assert!(matches!(
        h.editor.run_command("play.record").unwrap().as_slice(),
        [Action::RecordStop]
    ));
    // During Play without a recording it is refused with the reason.
    h.editor.debug.recorder = None;
    let Err(error) = h.editor.run_command("play.record") else {
        panic!("play.record during Play without a recording must be refused")
    };
    assert!(error.to_string().contains("stop Play first"), "{error}");
    h.editor.previewing = false;
    for bad in ["../up", "a b", ".x"] {
        assert!(
            h.editor.run_command_with("play.record", Some(bad)).is_err(),
            "{bad}"
        );
    }
    // The default name is free; the toolbar's record button is the same command.
    h.editor.debug.recorder = None;
    h.frame();
    let clicked = h.click("play:play.record");
    assert!(matches!(clicked.as_slice(), [Action::Play]));
    assert!(h
        .editor
        .debug
        .recorder
        .as_ref()
        .unwrap()
        .name
        .starts_with("editor-lab-recording-"));
}

#[test]
fn a_layout_saved_under_version_2_gets_the_console_and_is_kept_as_previous() {
    let dir = std::env::temp_dir().join(format!("axiom-m8-layout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.editor-prefs.json");
    // The M-UI Default (layout version 2) with the user's own change: History moved first.
    let classic = super::layout::builtin("Classic").unwrap();
    let v2 = serde_json::json!({"v":1,"main":{
        "split":"h","fraction":0.17,
        "a":{"tabs":["hierarchy"],"active":0},
        "b":{"split":"h","fraction":0.77,
            "a":{"split":"v","fraction":0.7,
                "a":{"tabs":["scene","game","graphs"],"active":0},
                "b":{"tabs":["history","assets","problems"],"active":0}},
            "b":{"tabs":["inspector"],"active":0}}},"windows":[]});
    std::fs::write(
        &path,
        serde_json::to_vec(
            &serde_json::json!({"snap":true,"layout":v2.clone(),"layout_version":2,
                "layouts":{"Previous":classic.clone()}}),
        )
        .unwrap(),
    )
    .unwrap();
    let mut h = Harness::new(lab());
    h.editor.set_prefs_path(path.clone());
    assert!(!h.editor.error, "{}", h.editor.status);
    assert!(
        h.editor.status.contains("layout updated"),
        "{}",
        h.editor.status
    );
    let state = h.editor.layout.dock.as_ref().unwrap();
    assert!(super::layout::is_open(state, Dock::Console));
    assert!(!super::layout::is_open(state, Dock::Profiler));
    h.frame();
    let saved: super::commands::Prefs =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.layout_version, super::layout::LAYOUT_VERSION);
    // The Previous left by the v1 migration stays; this one is "Previous 2".
    assert_eq!(saved.layouts.get("Previous"), Some(&classic));
    assert_eq!(saved.layouts.get("Previous 2"), Some(&v2));
    assert!(saved.snap);
    // Loading it again changes nothing.
    let mut again = Harness::new(lab());
    again.editor.set_prefs_path(path.clone());
    assert!(!again.editor.status.contains("layout updated"));
    // Window ▸ Layouts ▸ Previous 2 brings the user's arrangement back.
    again
        .editor
        .run_command_with("layout.load", Some("Previous 2"))
        .unwrap();
    assert_eq!(
        super::layout::to_spec(again.editor.layout.dock.as_ref().unwrap()),
        v2
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn filter_text_keeps_spaces_and_the_trace_follows_a_load() {
    let mut h = Harness::new(adv());
    h.editor
        .run_command_with("console.filter", Some("text=emote wave "))
        .unwrap();
    assert_eq!(h.editor.debug.console.text, "emote wave ");
    h.editor
        .run_command_with("trace.filter", Some("text= lumi"))
        .unwrap();
    assert_eq!(h.editor.debug.trace.text, " lumi");
    h.editor
        .run_command_with("trace.filter", Some("reset"))
        .unwrap();
    // Events before and after restoring a checkpoint both reach the trace (the restored
    // runtime counts its sequences from 0 again, in the same session).
    let emit = |event: &str| axiom_core::gameplay::RuntimeCommand::Emit {
        event: event.into(),
        fields: Default::default(),
    };
    let checkpoint = h.runtime.checkpoint().unwrap();
    for _ in 0..3 {
        h.runtime.command(&emit("inspect")).unwrap();
        h.runtime.step(&ActionMap::default());
        h.editor.debug.collect(&h.runtime, true);
    }
    let named = |h: &Harness, name: &str| {
        (1..=h.editor.debug.trace.len() as u64 + 64)
            .filter_map(|s| h.editor.debug.trace.get(s))
            .filter(|e| &*e.name == name)
            .count()
    };
    assert_eq!(named(&h, "inspect"), 3);
    h.runtime.restore_checkpoint(&checkpoint).unwrap();
    h.runtime.command(&emit("inspect")).unwrap();
    h.runtime.step(&ActionMap::default());
    h.editor.debug.collect(&h.runtime, true);
    assert_eq!(named(&h, "inspect"), 4, "an event after the load is shown");
}

fn adv() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/adv"),
        &crate::installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// Cost of the M8 hooks per frame (run in release:
/// `cargo test --release -p axiom-runtime --lib measure_m8_debug_hooks -- --ignored --nocapture`).
/// Prints medians of 2000 calls each; not a CI gate.
#[test]
#[ignore]
fn measure_m8_debug_hooks() {
    use std::time::Instant;
    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }
    let mut h = Harness::new(town());
    h.editor.previewing = true;
    let walk = ActionMap(
        [("move_z".to_owned(), 1.0), ("move_x".to_owned(), 0.45)]
            .into_iter()
            .collect(),
    );
    for _ in 0..300 {
        h.runtime.step(&walk);
        h.editor.debug.collect(&h.runtime, true);
    }
    let us = |f: &mut dyn FnMut()| {
        median(
            (0..2000)
                .map(|_| {
                    let t = Instant::now();
                    f();
                    t.elapsed().as_secs_f64() * 1e6
                })
                .collect(),
        )
    };
    // Hidden / idle: what every Play frame pays when nothing new happened.
    let (editor, runtime) = (&mut h.editor, &mut h.runtime);
    let collect = us(&mut || editor.debug.collect(runtime, true));
    let collect_edit = us(&mut || editor.debug.collect(runtime, false));
    let observe = us(&mut || {
        std::hint::black_box(editor.debug.observation());
    });
    let toggle = us(&mut || runtime.set_system_profiling(false));
    // One fixed step with and without per-system timing (the Profiler's visible cost).
    let step_off = us(&mut || runtime.step(&ActionMap::default()));
    runtime.set_system_profiling(true);
    let step_on = us(&mut || runtime.step(&ActionMap::default()));
    runtime.set_system_profiling(false);
    // Whole editor UI frame (Default layout: Console behind Assets) vs Console / Profiler shown.
    let frame = |h: &mut Harness| {
        median(
            (0..300)
                .map(|_| {
                    let t = Instant::now();
                    h.frame();
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect(),
        )
    };
    let ui_hidden = frame(&mut h);
    h.open("console");
    let ui_console = frame(&mut h);
    h.open("profiler");
    let ui_profiler = frame(&mut h);
    println!(
        "{}",
        serde_json::json!({
            "build": if cfg!(debug_assertions) {"debug"} else {"release"},
            "collect_play_idle_us": collect, "collect_edit_us": collect_edit,
            "debug_observation_us": observe, "profiling_toggle_noop_us": toggle,
            "step_us_profiling_off": step_off, "step_us_profiling_on": step_on,
            "ui_frame_ms_console_hidden": ui_hidden, "ui_frame_ms_console_visible": ui_console,
            "ui_frame_ms_profiler_visible": ui_profiler,
            "console_entries": h.editor.debug.console.len(), "trace_entries": h.editor.debug.trace.len(),
        })
    );
}
