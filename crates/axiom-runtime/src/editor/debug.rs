//! Debug tools: the Console (bounded log of editor messages, runtime faults, game output and
//! events), the Event trace (runtime event stream grouped by tick), the Profiler (main-loop
//! frame phases and per-system step time) and the state of a scenario recording.
//!
//! Memory is fixed: every list is a ring with a constant capacity. Nothing here allocates on
//! a frame in which nothing new happened (the views are cached by sequence number and filter
//! generation); event collection runs only during Play, the profiler only while its tab is
//! visible.
use super::icons::Icon;
use super::record::Recorder;
use super::theme::{self, ACCENT, CARD, ERROR, MUTED, SUCCESS, TEXT, WARNING};
use super::ui::caption;
use super::{Action, Editor};
use crate::gpu_timing::{GpuFrame, Section, SECTIONS};
use crate::world::WorldRuntime;
use egui::{RichText, Ui};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Arc;

/// Console entries kept (oldest dropped first).
pub(crate) const CONSOLE_CAPACITY: usize = 512;
/// Event trace entries kept.
pub(crate) const TRACE_CAPACITY: usize = 2048;
/// Frames in the profiler's rolling window (2 s at 120 Hz, 4 s at 60 Hz).
pub(crate) const FRAMES: usize = 240;
/// Main-loop phases per frame sample.
pub(crate) const PHASES: usize = 10;
/// Frames between per-system peak resets.
const PEAK_WINDOW: u32 = 120;
const ROW: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    const ALL: [Level; 3] = [Level::Info, Level::Warn, Level::Error];
    fn name(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
    fn color(self) -> egui::Color32 {
        match self {
            Level::Info => MUTED,
            Level::Warn => WARNING,
            Level::Error => ERROR,
        }
    }
    fn icon(self) -> Icon {
        match self {
            Level::Info => Icon::Info,
            Level::Warn => Icon::Warning,
            Level::Error => Icon::Error,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// Editor status line and failed requests (Problems).
    Editor,
    /// Runtime faults (activation, gameplay, cross-package events).
    Runtime,
    /// The game's own output lines (UI command failures, save / load results).
    Game,
    /// Gameplay and cross-package events during Play.
    Event,
}

impl Source {
    const ALL: [Source; 4] = [Source::Editor, Source::Runtime, Source::Game, Source::Event];
    fn name(self) -> &'static str {
        match self {
            Source::Editor => "editor",
            Source::Runtime => "runtime",
            Source::Game => "game",
            Source::Event => "event",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

pub(crate) struct LogEntry {
    pub seq: u64,
    pub tick: Option<u64>,
    pub level: Level,
    pub source: Source,
    pub text: Arc<str>,
    /// Object the entry is about (a click selects it).
    pub entity: Option<Arc<str>>,
    /// Identical consecutive messages are counted, not repeated.
    pub repeat: u32,
}

/// Case-insensitive (ASCII) substring test without allocating.
fn contains(haystack: &str, needle: &str) -> bool {
    needle.is_empty()
        || haystack
            .as_bytes()
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// `true` when `index` (by sequence) is still in a ring whose front has `front`.
fn index_of(front: Option<u64>, seq: u64, len: usize) -> Option<usize> {
    let i = seq.checked_sub(front?)? as usize;
    (i < len).then_some(i)
}

#[derive(Default)]
pub(crate) struct Console {
    entries: VecDeque<LogEntry>,
    seq: u64,
    pub dropped: u64,
    counts: [usize; 3],
    /// Sequence number of the newest error (observe reads it every frame without a scan).
    last_error: Option<u64>,
    /// Shown levels (Info, Warn, Error).
    pub levels: [bool; 3],
    pub source: Option<Source>,
    pub text: String,
    filter_gen: u64,
    /// Sequence numbers of the entries that pass the filter, for (newest seq, filter gen).
    view: Vec<u64>,
    view_key: (u64, u64),
    /// Times the view was rebuilt (tests: not per frame).
    pub builds: usize,
}

impl Console {
    fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(CONSOLE_CAPACITY),
            levels: [true; 3],
            view_key: (u64::MAX, u64::MAX),
            ..Default::default()
        }
    }

    /// Appends one message (or counts a repeat of the newest one).
    pub(crate) fn log(
        &mut self,
        level: Level,
        source: Source,
        text: &str,
        entity: Option<Arc<str>>,
        tick: Option<u64>,
    ) {
        if let Some(last) = self.entries.back_mut() {
            if last.level == level
                && last.source == source
                && &*last.text == text
                && last.entity == entity
            {
                last.repeat = last.repeat.saturating_add(1);
                last.tick = tick.or(last.tick);
                return;
            }
        }
        self.push(LogEntry {
            seq: 0,
            tick,
            level,
            source,
            text: text.into(),
            entity,
            repeat: 1,
        });
    }

    fn push(&mut self, mut entry: LogEntry) {
        if self.entries.len() == CONSOLE_CAPACITY {
            if let Some(old) = self.entries.pop_front() {
                self.counts[old.level as usize] -= 1;
                self.dropped += 1;
            }
        }
        self.seq += 1;
        entry.seq = self.seq;
        self.counts[entry.level as usize] += 1;
        if entry.level == Level::Error {
            self.last_error = Some(self.seq);
        }
        self.entries.push_back(entry);
    }

    pub(crate) fn clear(&mut self) {
        self.dropped = 0;
        self.counts = [0; 3];
        self.last_error = None;
        self.entries.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn count(&self, level: Level) -> usize {
        self.counts[level as usize]
    }
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.entries.capacity()
    }

    pub(crate) fn get(&self, seq: u64) -> Option<&LogEntry> {
        let i = index_of(self.entries.front().map(|e| e.seq), seq, self.entries.len())?;
        self.entries.get(i)
    }

    fn passes(&self, e: &LogEntry) -> bool {
        self.levels[e.level as usize]
            && self.source.is_none_or(|s| s == e.source)
            && (contains(&e.text, &self.text)
                || e.entity
                    .as_deref()
                    .is_some_and(|id| contains(id, &self.text)))
    }

    /// Filtered sequence numbers, rebuilt only when an entry arrived or the filter changed.
    fn refresh_view(&mut self) {
        let key = (
            self.seq ^ (self.entries.len() as u64) << 48,
            self.filter_gen,
        );
        if key == self.view_key {
            return;
        }
        self.view_key = key;
        self.builds += 1;
        let mut view = std::mem::take(&mut self.view);
        view.clear();
        view.extend(
            self.entries
                .iter()
                .filter(|e| self.passes(e))
                .map(|e| e.seq),
        );
        self.view = view;
    }

    /// Applies `console.filter` tokens: `levels=info,warn,error`, `source=all|NAME`,
    /// `text=REST` (the rest of the argument, may be empty) and `reset`.
    pub(crate) fn set_filter(&mut self, arg: &str) -> anyhow::Result<()> {
        let (mut levels, mut source, mut text) = (self.levels, self.source, None::<String>);
        // Not trimmed at the end: `text=` keeps the rest verbatim (spaces typed in the box).
        let mut rest = arg.trim_start();
        while !rest.is_empty() {
            if let Some(t) = rest.strip_prefix("text=") {
                text = Some(t.to_owned());
                break;
            }
            let (token, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            rest = tail.trim_start();
            match token.split_once('=') {
                None if token == "reset" => {
                    levels = [true; 3];
                    source = None;
                    text = Some(String::new());
                }
                Some(("levels", list)) => {
                    levels = [false; 3];
                    for name in list.split(',').filter(|n| !n.is_empty()) {
                        let level = Level::ALL
                            .into_iter()
                            .find(|l| l.name() == name || (name == "warning" && *l == Level::Warn))
                            .ok_or_else(|| {
                                anyhow::anyhow!("unknown level {name} (info, warn, error)")
                            })?;
                        levels[level as usize] = true;
                    }
                }
                Some(("source", "all")) => source = None,
                Some(("source", name)) => {
                    source = Some(Source::from_name(name).ok_or_else(|| {
                        anyhow::anyhow!("unknown source {name} (all, editor, runtime, game, event)")
                    })?)
                }
                _ => anyhow::bail!(
                    "console.filter takes levels=info,warn,error source=all|editor|runtime|game|event text=TEXT or reset"
                ),
            }
        }
        if let Some(text) = &text {
            anyhow::ensure!(
                text.chars().count() <= 128,
                "filter text is limited to 128 characters"
            );
        }
        self.levels = levels;
        self.source = source;
        if let Some(text) = text {
            self.text = text;
        }
        self.filter_gen += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceKind {
    /// Gameplay event (`behavior.program` / narrative), dispatched this tick.
    Event,
    /// Rule whose condition passed for an event.
    Rule,
    /// Narrative node, scene switch, ending, choice, executor activation.
    Narrative,
    /// Cross-package event envelope delivered between packages.
    Package,
    /// FSM agent changed state.
    State,
}

impl TraceKind {
    const ALL: [TraceKind; 5] = [
        TraceKind::Event,
        TraceKind::Rule,
        TraceKind::Narrative,
        TraceKind::Package,
        TraceKind::State,
    ];
    fn name(self) -> &'static str {
        match self {
            TraceKind::Event => "event",
            TraceKind::Rule => "rule",
            TraceKind::Narrative => "narrative",
            TraceKind::Package => "package",
            TraceKind::State => "state",
        }
    }
    fn color(self) -> egui::Color32 {
        match self {
            TraceKind::Event => ACCENT,
            TraceKind::Rule => theme::PREFAB,
            TraceKind::Narrative => WARNING,
            TraceKind::Package => SUCCESS,
            TraceKind::State => TEXT,
        }
    }
}

pub(crate) struct TraceEntry {
    pub seq: u64,
    /// Play session (1 = the first Play since the editor started).
    pub session: u32,
    pub tick: u64,
    pub kind: TraceKind,
    /// Event / rule / node / state name.
    pub name: Arc<str>,
    /// Gameplay trace kind (`node`, `scene`, …), fields of a package event, FSM machine.
    pub detail: Option<Arc<str>>,
    pub entity: Option<Arc<str>>,
}

enum TraceRow {
    Tick(u32, u64),
    Entry(u64),
}

pub(crate) struct Trace {
    entries: VecDeque<TraceEntry>,
    seq: u64,
    pub dropped: u64,
    pub paused: bool,
    /// Events that happened while paused (not kept).
    pub skipped: u64,
    /// Gameplay trace entries the runtime had already dropped before they were read.
    pub missed: u64,
    /// Shown kinds by `TraceKind` index.
    pub kinds: [bool; 5],
    pub text: String,
    filter_gen: u64,
    rows: Vec<TraceRow>,
    /// Scratch for building `rows` (kept for its capacity).
    order: Vec<(u32, u64, u64)>,
    rows_key: (u64, u64),
    pub builds: usize,
    // Collection state for the current runtime.
    runtime_session: String,
    session: u32,
    gameplay_seq: u64,
    package_seq: u64,
    fsm: Vec<(Arc<str>, Arc<str>)>,
    faults: [bool; 3],
}

impl Default for Trace {
    fn default() -> Self {
        Self {
            entries: VecDeque::with_capacity(TRACE_CAPACITY),
            seq: 0,
            dropped: 0,
            paused: false,
            skipped: 0,
            missed: 0,
            kinds: [true; 5],
            text: String::new(),
            filter_gen: 0,
            rows: vec![],
            order: vec![],
            rows_key: (u64::MAX, u64::MAX),
            builds: 0,
            runtime_session: String::new(),
            session: 0,
            gameplay_seq: 0,
            package_seq: 0,
            fsm: vec![],
            faults: [false; 3],
        }
    }
}

impl Trace {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.entries.capacity()
    }
    pub(crate) fn get(&self, seq: u64) -> Option<&TraceEntry> {
        let i = index_of(self.entries.front().map(|e| e.seq), seq, self.entries.len())?;
        self.entries.get(i)
    }
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.dropped = 0;
        self.skipped = 0;
        self.missed = 0;
    }

    fn push(
        &mut self,
        tick: u64,
        kind: TraceKind,
        name: Arc<str>,
        detail: Option<Arc<str>>,
        entity: Option<Arc<str>>,
    ) {
        if self.paused {
            self.skipped += 1;
            return;
        }
        if self.entries.len() == TRACE_CAPACITY {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.seq += 1;
        self.entries.push_back(TraceEntry {
            seq: self.seq,
            session: self.session,
            tick,
            kind,
            name,
            detail,
            entity,
        });
    }

    fn passes(&self, e: &TraceEntry) -> bool {
        self.kinds[e.kind as usize]
            && (contains(&e.name, &self.text)
                || e.detail.as_deref().is_some_and(|d| contains(d, &self.text))
                || e.entity.as_deref().is_some_and(|d| contains(d, &self.text)))
    }

    fn refresh_rows(&mut self) {
        let key = (
            self.seq ^ (self.entries.len() as u64) << 48,
            self.filter_gen,
        );
        if key == self.rows_key {
            return;
        }
        self.rows_key = key;
        self.builds += 1;
        let mut rows = std::mem::take(&mut self.rows);
        rows.clear();
        // One frame can run several fixed steps and collects gameplay, package and FSM entries
        // in turn: order by (session, tick), stable, so each tick gets a single header.
        let mut order = std::mem::take(&mut self.order);
        order.clear();
        order.extend(
            self.entries
                .iter()
                .filter(|e| self.passes(e))
                .map(|e| (e.session, e.tick, e.seq)),
        );
        order.sort_by_key(|&(session, tick, _)| (session, tick));
        let mut group = None;
        for &(session, tick, seq) in &order {
            if group != Some((session, tick)) {
                group = Some((session, tick));
                rows.push(TraceRow::Tick(session, tick));
            }
            rows.push(TraceRow::Entry(seq));
        }
        self.order = order;
        self.rows = rows;
    }

    /// `trace.filter` tokens: `kinds=event,rule,narrative,package,state|all`, `text=REST`,
    /// `reset`.
    pub(crate) fn set_filter(&mut self, arg: &str) -> anyhow::Result<()> {
        let (mut kinds, mut text) = (self.kinds, None::<String>);
        // Not trimmed at the end: `text=` keeps the rest verbatim (spaces typed in the box).
        let mut rest = arg.trim_start();
        while !rest.is_empty() {
            if let Some(t) = rest.strip_prefix("text=") {
                text = Some(t.to_owned());
                break;
            }
            let (token, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            rest = tail.trim_start();
            match token.split_once('=') {
                None if token == "reset" => {
                    kinds = [true; 5];
                    text = Some(String::new());
                }
                Some(("kinds", "all")) => kinds = [true; 5],
                Some(("kinds", list)) => {
                    kinds = [false; 5];
                    for name in list.split(',').filter(|n| !n.is_empty()) {
                        let kind = TraceKind::ALL
                            .into_iter()
                            .find(|k| k.name() == name)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "unknown kind {name} (event, rule, narrative, package, state)"
                                )
                            })?;
                        kinds[kind as usize] = true;
                    }
                }
                _ => anyhow::bail!(
                    "trace.filter takes kinds=event,rule,narrative,package,state|all text=TEXT or reset"
                ),
            }
        }
        if let Some(text) = &text {
            anyhow::ensure!(
                text.chars().count() <= 128,
                "filter text is limited to 128 characters"
            );
        }
        self.kinds = kinds;
        if let Some(text) = text {
            self.text = text;
        }
        self.filter_gen += 1;
        Ok(())
    }

    /// Reads what the runtime produced since the last call. Only during Play; allocates only
    /// for new events (sequence numbers and agent states are compared in place).
    fn collect(&mut self, console: &mut Console, runtime: &WorldRuntime) {
        let state = &runtime.state;
        if self.runtime_session != runtime.runtime_session {
            // A new Play session: its own sequences start over; FSM states are the baseline.
            self.runtime_session.clone_from(&runtime.runtime_session);
            self.session += 1;
            self.gameplay_seq = 0;
            self.package_seq = 0;
            self.fsm.clear();
            if let Some(fsm) = runtime.system::<crate::fsm::FsmSystem>() {
                self.fsm.extend(
                    fsm.agent_states()
                        .map(|(a, _, s)| (Arc::from(a), Arc::from(s))),
                );
            }
        }
        if let Some(g) = &state.gameplay {
            if g.trace.back().is_some_and(|t| t.seq < self.gameplay_seq) {
                // Restored from a checkpoint (Load): the gameplay trace counts again from 0.
                self.gameplay_seq = 0;
            }
            if let Some(first) = g.trace.iter().find(|t| t.seq > self.gameplay_seq) {
                self.missed += first.seq - self.gameplay_seq - 1;
            }
            let after = self.gameplay_seq;
            for t in g.trace.iter().filter(|t| t.seq > after) {
                let kind = match t.kind.as_str() {
                    "event" => TraceKind::Event,
                    "rule" => TraceKind::Rule,
                    _ => TraceKind::Narrative,
                };
                let name: Arc<str> = Arc::from(t.id.as_str());
                let entity = state
                    .compiled
                    .indices
                    .contains_key(&t.id)
                    .then(|| name.clone());
                if kind == TraceKind::Event {
                    let text = format!("event {}", t.id);
                    console.log(
                        Level::Info,
                        Source::Event,
                        &text,
                        entity.clone(),
                        Some(t.tick),
                    );
                }
                let detail = (kind == TraceKind::Narrative).then(|| Arc::from(t.kind.as_str()));
                self.push(t.tick, kind, name, detail, entity);
            }
            if let Some(last) = g.trace.back() {
                self.gameplay_seq = self.gameplay_seq.max(last.seq);
            }
        }
        let newest = state.events.recent().next_back().map_or(0, |e| e.seq);
        if newest < self.package_seq {
            // Restored from a checkpoint (Load): the runtime counts again from 0.
            self.package_seq = 0;
        }
        if newest > self.package_seq {
            let after = self.package_seq;
            if let Some(first) = state.events.recent().find(|e| e.seq > after) {
                // The runtime keeps the last 64 delivered envelopes.
                self.missed += first.seq - after - 1;
            }
            for e in state.events.recent().filter(|e| e.seq > after) {
                let mut detail = String::new();
                let mut entity = None;
                for (k, v) in &e.fields {
                    if !detail.is_empty() {
                        detail.push(' ');
                    }
                    detail.push_str(k);
                    detail.push('=');
                    match v {
                        axiom_core::gameplay::Scalar::Text(text) => {
                            if entity.is_none() && state.compiled.indices.contains_key(text) {
                                entity = Some(Arc::<str>::from(text.as_str()));
                            }
                            detail.push_str(text);
                        }
                        other => detail.push_str(&serde_json::to_string(other).unwrap_or_default()),
                    }
                }
                let text = format!("{} from {} {detail}", e.event, e.publisher);
                console.log(
                    Level::Info,
                    Source::Event,
                    &text,
                    entity.clone(),
                    Some(e.tick),
                );
                self.push(
                    e.tick,
                    TraceKind::Package,
                    Arc::from(e.event.as_str()),
                    Some(Arc::from(format!("{} · {detail}", e.publisher))),
                    entity,
                );
            }
            self.package_seq = newest;
        }
        if let Some(fsm) = runtime.system::<crate::fsm::FsmSystem>() {
            let tick = state.tick;
            for (i, (agent, machine, now)) in fsm.agent_states().enumerate() {
                match self.fsm.get_mut(i) {
                    Some((a, s)) if &**a == agent => {
                        if &**s != now {
                            *s = Arc::from(now);
                            let (name, entity) = (s.clone(), a.clone());
                            self.push(
                                tick,
                                TraceKind::State,
                                name,
                                Some(Arc::from(machine)),
                                Some(entity),
                            );
                        }
                    }
                    // Agents are fixed for a runtime; a mismatch only follows a rebuild.
                    _ => {
                        self.fsm.truncate(i);
                        self.fsm.push((Arc::from(agent), Arc::from(now)));
                    }
                }
            }
        }
    }

    /// Logs a runtime fault once when it appears (all three stay set until the runtime is
    /// replaced). Runs every frame: three `Option` checks.
    fn faults(&mut self, console: &mut Console, runtime: &WorldRuntime) {
        let state = &runtime.state;
        let now = [
            runtime.activation_error.as_ref(),
            state.gameplay.as_ref().and_then(|g| g.fault.as_ref()),
            state.events.fault.as_ref(),
        ];
        for (i, (fault, label)) in now
            .into_iter()
            .zip(["activation", "gameplay", "cross-package events"])
            .enumerate()
        {
            match fault {
                Some(value) if !self.faults[i] => {
                    self.faults[i] = true;
                    let message = value
                        .pointer("/diagnostics/0/message")
                        .or_else(|| value.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    let path = value.pointer("/diagnostics/0/path").and_then(Value::as_str);
                    let entity = path
                        .and_then(|p| super::entity_in_path(p, &state.compiled.source))
                        .map(Arc::from);
                    console.log(
                        Level::Error,
                        Source::Runtime,
                        &format!("{label} fault: {message}"),
                        entity,
                        Some(state.tick),
                    );
                }
                None => self.faults[i] = false,
                _ => {}
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct FrameSample {
    pub phases: [f32; PHASES],
    /// Whole frame (ms), including the swapchain wait.
    pub total: f32,
    /// Part of `total` spent waiting for the swapchain image.
    pub acquire: f32,
    pub steps: u16,
}

pub(crate) struct SystemRow {
    pub name: Box<str>,
    pub last: f32,
    pub avg: f32,
    pub peak: f32,
    next_peak: f32,
}

pub(crate) struct Profiler {
    /// The Profiler tab was drawn this frame (nothing is measured otherwise).
    pub visible: bool,
    pub paused: bool,
    frames: Box<[FrameSample; FRAMES]>,
    head: usize,
    pub len: usize,
    names: [&'static str; PHASES],
    phase_count: usize,
    systems: Vec<SystemRow>,
    systems_session: String,
    window: u32,
    /// GPU timing as the host reports it: `None` without a GPU renderer, `Some(Err(why))`
    /// when timestamp queries cannot run, `Some(Ok(()))` when they do.
    pub gpu_timing: Option<Result<(), &'static str>>,
    /// Measured GPU frames (timestamp queries), a ring like `frames`; they arrive a few
    /// frames late and not for every frame.
    gpu: Box<[GpuFrame; FRAMES]>,
    gpu_head: usize,
    pub gpu_len: usize,
    /// Frames the renderer could not measure (readbacks still pending) or discarded.
    pub gpu_dropped: u64,
}

impl Default for Profiler {
    fn default() -> Self {
        Self {
            visible: false,
            paused: false,
            frames: Box::new([FrameSample::default(); FRAMES]),
            head: 0,
            len: 0,
            names: [""; PHASES],
            phase_count: 0,
            systems: vec![],
            systems_session: String::new(),
            window: 0,
            gpu_timing: None,
            gpu: Box::new([GpuFrame::default(); FRAMES]),
            gpu_head: 0,
            gpu_len: 0,
            gpu_dropped: 0,
        }
    }
}

impl Profiler {
    /// Whether the host should measure this frame.
    pub(crate) fn wants(&self) -> bool {
        self.visible && !self.paused
    }

    /// One frame's phases (`name`, ms), its total and the fixed steps it ran.
    pub(crate) fn push_frame(&mut self, phases: &[(&'static str, f64)], total: f64, steps: u32) {
        let mut sample = FrameSample {
            total: total as f32,
            steps: steps.min(u16::MAX as u32) as u16,
            ..Default::default()
        };
        let mut count = 0;
        for (name, ms) in phases {
            if name.ends_with("acquire_wait") {
                sample.acquire = *ms as f32;
                continue;
            }
            if count == PHASES {
                break;
            }
            if self.names[count] != *name {
                // The phase list changed (first frame, or a different frame path): restart.
                self.names[count] = name;
                self.len = 0;
            }
            sample.phases[count] = *ms as f32;
            count += 1;
        }
        self.phase_count = count;
        self.frames[self.head] = sample;
        self.head = (self.head + 1) % FRAMES;
        self.len = (self.len + 1).min(FRAMES);
    }

    /// Takes the runtime's per-system step time of this frame (profiling must be on).
    pub(crate) fn take_systems(&mut self, runtime: &mut WorldRuntime) {
        if self.systems_session != runtime.runtime_session
            || self.systems.len() != runtime.system_ids().len()
        {
            self.systems_session.clone_from(&runtime.runtime_session);
            self.systems.clear();
            self.systems
                .extend(runtime.system_ids().map(|id| SystemRow {
                    name: id.into(),
                    last: 0.0,
                    avg: 0.0,
                    peak: 0.0,
                    next_peak: 0.0,
                }));
        }
        self.window += 1;
        let reset = self.window >= PEAK_WINDOW;
        if reset {
            self.window = 0;
        }
        let systems = &mut self.systems;
        runtime.take_system_profile(|i, ms| {
            if let Some(row) = systems.get_mut(i) {
                let ms = ms as f32;
                row.last = ms;
                row.avg = row.avg * 0.95 + ms * 0.05;
                row.next_peak = row.next_peak.max(ms);
                row.peak = row.peak.max(ms);
                if reset {
                    row.peak = row.next_peak;
                    row.next_peak = 0.0;
                }
            }
        });
    }

    /// Samples oldest first.
    fn samples(&self) -> impl Iterator<Item = &FrameSample> {
        let start = (self.head + FRAMES - self.len) % FRAMES;
        (0..self.len).map(move |i| &self.frames[(start + i) % FRAMES])
    }

    /// One GPU-measured frame (ignored while paused).
    pub(crate) fn push_gpu(&mut self, frame: &GpuFrame) {
        if self.paused {
            return;
        }
        self.gpu[self.gpu_head] = *frame;
        self.gpu_head = (self.gpu_head + 1) % FRAMES;
        self.gpu_len = (self.gpu_len + 1).min(FRAMES);
    }

    /// GPU section means over the window: (`[mean ms; SECTIONS]`, mean total, max of each,
    /// max total). `None` before the first measured frame.
    fn gpu_stats(&self) -> Option<([f32; SECTIONS], f32, [f32; SECTIONS], f32)> {
        if self.gpu_len == 0 {
            return None;
        }
        let start = (self.gpu_head + FRAMES - self.gpu_len) % FRAMES;
        let (mut mean, mut max, mut total, mut worst) =
            ([0.0f32; SECTIONS], [0.0f32; SECTIONS], 0.0f32, 0.0f32);
        for i in 0..self.gpu_len {
            let f = &self.gpu[(start + i) % FRAMES];
            for s in 0..SECTIONS {
                mean[s] += f.ms[s];
                max[s] = max[s].max(f.ms[s]);
            }
            total += f.total;
            worst = worst.max(f.total);
        }
        let n = self.gpu_len as f32;
        Some((mean.map(|v| v / n), total / n, max, worst))
    }

    fn gpu_last(&self) -> &GpuFrame {
        &self.gpu[(self.gpu_head + FRAMES - 1) % FRAMES]
    }

    /// `observe` → `editor.debug.profiler.cpu`: window means of the main-thread phases and of
    /// the frame without the swapchain wait (`null` before the first measured frame).
    fn cpu_observation(&self) -> Value {
        if self.len == 0 {
            return Value::Null;
        }
        let (mut sum, mut cpu) = ([0.0f32; PHASES], 0.0f32);
        for s in self.samples() {
            for (total, v) in sum.iter_mut().zip(s.phases).take(self.phase_count) {
                *total += v;
            }
            cpu += (s.total - s.acquire).max(0.0);
        }
        let n = self.len as f32;
        let phases: serde_json::Map<String, Value> = (0..self.phase_count)
            .map(|i| (self.names[i].to_owned(), json!(round3(sum[i] / n))))
            .collect();
        json!({"frame_mean_ms":round3(cpu / n),"phase_mean_ms":phases})
    }

    /// `observe` → `editor.debug.profiler.gpu`.
    fn gpu_observation(&self) -> Value {
        match self.gpu_timing {
            None => json!({"available":false,"reason":"no GPU renderer"}),
            Some(Err(why)) => json!({"available":false,"reason":why}),
            Some(Ok(())) => {
                let mut out = json!({"available":true,"source":"timestamp_queries","frames":self.gpu_len,"dropped":self.gpu_dropped});
                if let Some((mean, total, _, worst)) = self.gpu_stats() {
                    let sections: serde_json::Map<String, Value> = Section::ALL
                        .iter()
                        .filter(|s| mean[**s as usize] > 0.0)
                        .map(|s| (s.name().to_owned(), json!(round3(mean[*s as usize]))))
                        .collect();
                    out["mean_ms"] = Value::Object(sections);
                    out["total_mean_ms"] = json!(round3(total));
                    out["total_max_ms"] = json!(round3(worst));
                    out["last_total_ms"] = json!(round3(self.gpu_last().total));
                }
                out
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        self.gpu_len = 0;
        self.gpu_dropped = 0;
        self.len = 0;
        self.systems.clear();
        self.systems_session.clear();
    }
}

/// Everything the debug tabs and the recorder keep.
pub(crate) struct Debug {
    pub console: Console,
    pub trace: Trace,
    pub profiler: Profiler,
    /// The scenario being recorded (armed by `play.record`, active during Play).
    pub recorder: Option<Recorder>,
    /// Where recordings are written (`examples/` under the working directory).
    pub record_dir: std::path::PathBuf,
    /// Summary of the last finished recording (observe).
    pub last_recording: Option<Value>,
    /// Lines of the game's output already copied into the Console (host bookkeeping).
    pub game_output_seen: usize,
}

impl Default for Debug {
    fn default() -> Self {
        Self {
            console: Console::new(),
            trace: Trace::default(),
            profiler: Profiler::default(),
            recorder: None,
            record_dir: "examples".into(),
            last_recording: None,
            game_output_seen: 0,
        }
    }
}

impl Debug {
    /// Once per frame after the simulation: runtime faults always, events during Play.
    pub(crate) fn collect(&mut self, runtime: &WorldRuntime, playing: bool) {
        self.trace.faults(&mut self.console, runtime);
        if playing {
            self.trace.collect(&mut self.console, runtime);
        }
    }

    /// New lines of the game's output (`UiState::output`).
    pub(crate) fn game_output(&mut self, lines: &[String], tick: u64) {
        for line in lines {
            let level = if line.starts_with("ERROR") || line.contains("FAILED") {
                Level::Error
            } else {
                Level::Info
            };
            self.console
                .log(level, Source::Game, line, None, Some(tick));
        }
    }

    pub(crate) fn observation(&self) -> Value {
        let c = &self.console;
        let last_error = c
            .last_error
            .and_then(|seq| c.get(seq))
            .map(|e| json!({"seq":e.seq,"source":e.source.name(),"text":&*e.text,"entity":e.entity.as_deref()}));
        json!({
            "console":{"entries":c.len(),"errors":c.count(Level::Error),"warnings":c.count(Level::Warn),"dropped":c.dropped,"last_error":last_error},
            "trace":{"entries":self.trace.len(),"paused":self.trace.paused,"skipped":self.trace.skipped,"dropped":self.trace.dropped,"missed":self.trace.missed},
            "profiler":{"measuring":self.profiler.wants(),"paused":self.profiler.paused,"frames":self.profiler.len,"cpu":self.profiler.cpu_observation(),"gpu":self.profiler.gpu_observation()},
            "recording":self.recorder.as_ref().map(Recorder::observation),
            "last_recording":self.last_recording,
        })
    }
}

fn header(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(add);
        });
}

impl Editor {
    /// Logs into the Console (editor source); the status line is set separately.
    pub(super) fn console_log(&mut self, level: Level, text: &str, entity: Option<String>) {
        let tick = self.previewing.then_some(self.play.tick);
        self.debug
            .console
            .log(level, Source::Editor, text, entity.map(Arc::from), tick);
    }

    /// A warning in the Console (the status line is unchanged).
    pub(crate) fn console_warn(&mut self, text: &str) {
        self.console_log(Level::Warn, text, None);
    }

    /// Offers a new scenario file in the Scenarios tab (a finished recording).
    pub(crate) fn add_scenario(&mut self, path: String) {
        if !self.scenarios.contains(&path) && self.scenarios.len() < 64 {
            self.scenarios.push(path);
            self.scenarios.sort();
        }
    }

    /// `console.*`, `trace.*`, `profiler.*` and `play.record`.
    pub(super) fn debug_command(
        &mut self,
        id: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Vec<Action>> {
        let no_arg = |arg: Option<&str>| {
            anyhow::ensure!(arg.is_none(), "{id} takes no argument");
            Ok(())
        };
        match id {
            "console.clear" => {
                no_arg(arg)?;
                self.debug.console.clear();
            }
            "console.filter" => match arg {
                Some(arg) => self.debug.console.set_filter(arg)?,
                None => self.show_dock(super::Dock::Console),
            },
            // Without an entry number (menu, palette) it opens the tab to pick one from.
            "console.open" if arg.is_none() => self.show_dock(super::Dock::Console),
            "console.open" => {
                let seq = parse_seq(arg)?;
                let entry = self
                    .debug
                    .console
                    .get(seq)
                    .ok_or_else(|| anyhow::anyhow!("console entry {seq} is no longer kept"))?;
                let entity = entry.entity.clone();
                self.open_entity(entity.as_deref(), "console", seq)?;
            }
            "trace.pause" => {
                no_arg(arg)?;
                self.debug.trace.paused = !self.debug.trace.paused;
            }
            "trace.clear" => {
                no_arg(arg)?;
                self.debug.trace.clear();
            }
            "trace.filter" => match arg {
                Some(arg) => self.debug.trace.set_filter(arg)?,
                None => self.show_dock(super::Dock::Trace),
            },
            "trace.open" if arg.is_none() => self.show_dock(super::Dock::Trace),
            "trace.open" => {
                let seq = parse_seq(arg)?;
                let entry = self
                    .debug
                    .trace
                    .get(seq)
                    .ok_or_else(|| anyhow::anyhow!("trace entry {seq} is no longer kept"))?;
                let entity = entry.entity.clone();
                self.open_entity(entity.as_deref(), "trace", seq)?;
            }
            "profiler.pause" => {
                no_arg(arg)?;
                self.debug.profiler.paused = !self.debug.profiler.paused;
            }
            "profiler.reset" => {
                no_arg(arg)?;
                self.debug.profiler.reset();
            }
            "play.record" => return self.record_command(arg),
            other => anyhow::bail!("unknown debug command {other}"),
        }
        Ok(vec![])
    }

    fn open_entity(&mut self, entity: Option<&str>, what: &str, seq: u64) -> anyhow::Result<()> {
        let id =
            entity.ok_or_else(|| anyhow::anyhow!("{what} entry {seq} is not about an object"))?;
        anyhow::ensure!(
            self.world.entity(id).is_some(),
            "{id} is not an authored object (spawned at runtime or removed)"
        );
        self.extra.clear();
        self.select(Some(id.to_owned()));
        self.frame_selection = true;
        self.scroll_selection = true;
        Ok(())
    }

    /// `play.record [NAME]`: outside Play, arms a recording and starts Play; while recording,
    /// stops it and writes the file (Play continues).
    fn record_command(&mut self, arg: Option<&str>) -> anyhow::Result<Vec<Action>> {
        if self.debug.recorder.is_some() && self.previewing {
            anyhow::ensure!(
                arg.is_none(),
                "play.record stops the recording; it takes no name then"
            );
            return Ok(vec![Action::RecordStop]);
        }
        let name = match arg.map(str::trim).filter(|a| !a.is_empty()) {
            Some(name) => name.to_owned(),
            None => super::record::free_name(&self.debug.record_dir, &self.world.id),
        };
        self.debug.recorder = Some(Recorder::new(&self.debug.record_dir, &name)?);
        Ok(vec![Action::Play])
    }

    /// Console tab body.
    pub(super) fn console_tab(&mut self, ui: &mut Ui) {
        let icons = self.icons;
        let mut command: Option<(&'static str, Option<String>)> = None;
        header(ui, |ui| {
            for level in Level::ALL {
                let on = self.debug.console.levels[level as usize];
                let r = ui
                    .selectable_label(
                        on,
                        RichText::new(format!(
                            "{} {}",
                            level.name(),
                            self.debug.console.count(level)
                        ))
                        .color(if on { level.color() } else { MUTED }),
                    )
                    .on_hover_text("Show / hide this level (console.filter levels=…)");
                self.rects
                    .insert(format!("console:level:{}", level.name()), r.rect);
                if r.clicked() {
                    let mut levels = self.debug.console.levels;
                    levels[level as usize] = !on;
                    command = Some(("console.filter", Some(levels_arg(&levels))));
                }
            }
            theme::divider(ui);
            let source = self
                .debug
                .console
                .source
                .map_or("all sources", Source::name);
            let chevron = icons.image(ui.ctx(), Icon::ChevronDown, 14.0, MUTED);
            let r = ui.add(egui::Button::image_and_text(chevron, source));
            self.rects.insert("console:source".into(), r.rect);
            let id = ui.make_persistent_id("console_source");
            theme::dropdown(ui, id, &r, |ui| {
                for (name, value) in std::iter::once(("all", None))
                    .chain(Source::ALL.into_iter().map(|s| (s.name(), Some(s))))
                {
                    let item = ui.selectable_label(self.debug.console.source == value, name);
                    self.rects
                        .insert(format!("console:source:{name}"), item.rect);
                    if item.clicked() {
                        command = Some(("console.filter", Some(format!("source={name}"))));
                        theme::close_dropdown(ui, id);
                    }
                }
            });
            let mut text = self.debug.console.text.clone();
            let edit = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("Filter text / object")
                    .desired_width(160.0),
            );
            self.rects.insert("console:text".into(), edit.rect);
            if edit.changed() {
                command = Some(("console.filter", Some(format!("text={text}"))));
            }
            let clear = theme::icon_button(ui, icons, Icon::Delete, "Clear (console.clear)", false);
            self.rects.insert("console:clear".into(), clear.rect);
            if clear.clicked() {
                command = Some(("console.clear", None));
            }
            if self.debug.console.dropped > 0 {
                caption(
                    ui,
                    format!(
                        "{} older dropped (keeps {CONSOLE_CAPACITY})",
                        self.debug.console.dropped
                    ),
                );
            }
        });
        self.debug.console.refresh_view();
        if self.debug.console.view.is_empty() {
            caption(
                ui,
                if self.debug.console.len() == 0 {
                    "No messages. Editor messages, failed requests, runtime faults, the game's output and Play events appear here."
                } else {
                    "No message matches the filter."
                },
            );
        }
        let mut open = None;
        let console = &self.debug.console;
        let rects = &mut self.rects;
        egui::ScrollArea::vertical()
            .id_salt("console_rows")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, ROW, console.view.len(), |ui, range| {
                // Rows are clicked as a whole (selectable labels would take the click).
                ui.style_mut().interaction.selectable_labels = false;
                for &seq in &console.view[range] {
                    let Some(e) = console.get(seq) else { continue };
                    let r = ui
                        .horizontal(|ui| {
                            ui.set_min_height(ROW - 2.0);
                            icons.show(ui, e.level.icon(), 14.0, e.level.color());
                            let tick = e.tick.map_or(String::new(), |t| format!("{t:>6}"));
                            ui.label(RichText::new(tick).monospace().size(11.0).color(MUTED));
                            ui.label(RichText::new(e.source.name()).size(11.0).color(MUTED));
                            let mut text =
                                RichText::new(&*e.text).color(if e.level == Level::Info {
                                    TEXT
                                } else {
                                    e.level.color()
                                });
                            if e.level == Level::Info && e.source == Source::Event {
                                text = text.color(MUTED);
                            }
                            ui.add(egui::Label::new(text).truncate());
                            if e.repeat > 1 {
                                theme::chip(ui, format!("×{}", e.repeat), MUTED);
                            }
                            if let Some(id) = &e.entity {
                                ui.label(RichText::new(format!("→ {id}")).size(11.0).color(ACCENT));
                            }
                        })
                        .response
                        .interact(egui::Sense::click());
                    rects.insert(format!("console:row:{seq}"), r.rect);
                    let r = if e.entity.is_some() {
                        r.on_hover_text("Click to select this object")
                    } else {
                        r
                    };
                    if r.clicked() && e.entity.is_some() {
                        open = Some(seq);
                    }
                }
            });
        if let Some(seq) = open {
            command = Some(("console.open", Some(seq.to_string())));
        }
        if let Some((id, arg)) = command {
            if let Err(e) = self.run_command_with(id, arg.as_deref()) {
                self.set_error(format!("{e:#}"));
            }
        }
    }

    /// Event trace tab body.
    pub(super) fn trace_tab(&mut self, ui: &mut Ui) {
        let icons = self.icons;
        let mut command: Option<(&'static str, Option<String>)> = None;
        header(ui, |ui| {
            let paused = self.debug.trace.paused;
            let r = theme::icon_button(
                ui,
                icons,
                if paused { Icon::Play } else { Icon::Pause },
                if paused {
                    "Resume (trace.pause)"
                } else {
                    "Pause (trace.pause)"
                },
                paused,
            );
            self.rects.insert("trace:pause".into(), r.rect);
            if r.clicked() {
                command = Some(("trace.pause", None));
            }
            let r = theme::icon_button(ui, icons, Icon::Delete, "Clear (trace.clear)", false);
            self.rects.insert("trace:clear".into(), r.rect);
            if r.clicked() {
                command = Some(("trace.clear", None));
            }
            theme::divider(ui);
            for kind in TraceKind::ALL {
                let on = self.debug.trace.kinds[kind as usize];
                let r = ui.selectable_label(
                    on,
                    RichText::new(kind.name()).color(if on { kind.color() } else { MUTED }),
                );
                self.rects
                    .insert(format!("trace:kind:{}", kind.name()), r.rect);
                if r.clicked() {
                    let mut kinds = self.debug.trace.kinds;
                    kinds[kind as usize] = !on;
                    command = Some(("trace.filter", Some(kinds_arg(&kinds))));
                }
            }
            let mut text = self.debug.trace.text.clone();
            let edit = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("Filter name / object")
                    .desired_width(150.0),
            );
            self.rects.insert("trace:text".into(), edit.rect);
            if edit.changed() {
                command = Some(("trace.filter", Some(format!("text={text}"))));
            }
            let t = &self.debug.trace;
            let mut info = format!("{} kept", t.len());
            if t.paused || t.skipped > 0 {
                info.push_str(&format!(" · {} skipped while paused", t.skipped));
            }
            if t.dropped > 0 {
                info.push_str(&format!(" · {} older dropped", t.dropped));
            }
            if t.missed > 0 {
                info.push_str(&format!(
                    " · {} missed (runtime keeps only the newest 128 / 64)",
                    t.missed
                ));
            }
            caption(ui, info);
        });
        self.debug.trace.refresh_rows();
        if self.debug.trace.rows.is_empty() {
            caption(
                ui,
                if self.previewing {
                    "No events yet. Gameplay events, rules, narrative steps, cross-package events and FSM state changes appear here, grouped by tick."
                } else {
                    "Recorded during Play: gameplay events, rules, narrative steps, cross-package events and FSM state changes, grouped by tick."
                },
            );
        }
        let mut open = None;
        let trace = &self.debug.trace;
        let rects = &mut self.rects;
        egui::ScrollArea::vertical()
            .id_salt("trace_rows")
            .auto_shrink([false, false])
            .stick_to_bottom(!trace.paused)
            .show_rows(ui, ROW, trace.rows.len(), |ui, range| {
                // Rows are clicked as a whole (selectable labels would take the click).
                ui.style_mut().interaction.selectable_labels = false;
                for row in &trace.rows[range] {
                    match row {
                        TraceRow::Tick(session, tick) => {
                            ui.horizontal(|ui| {
                                ui.set_min_height(ROW - 2.0);
                                ui.label(
                                    RichText::new(format!("tick {tick}"))
                                        .monospace()
                                        .strong()
                                        .color(TEXT),
                                );
                                caption(ui, format!("Play #{session}"));
                            });
                        }
                        TraceRow::Entry(seq) => {
                            let Some(e) = trace.get(*seq) else { continue };
                            let r = ui
                                .horizontal(|ui| {
                                    ui.set_min_height(ROW - 2.0);
                                    ui.add_space(14.0);
                                    ui.label(
                                        RichText::new(format!("{:<9}", e.kind.name()))
                                            .monospace()
                                            .size(11.0)
                                            .color(e.kind.color()),
                                    );
                                    ui.label(RichText::new(&*e.name).color(TEXT));
                                    if let Some(d) = &e.detail {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&**d).size(11.0).color(MUTED),
                                            )
                                            .truncate(),
                                        );
                                    }
                                    if let Some(id) = &e.entity {
                                        ui.label(
                                            RichText::new(format!("→ {id}"))
                                                .size(11.0)
                                                .color(ACCENT),
                                        );
                                    }
                                })
                                .response
                                .interact(egui::Sense::click());
                            rects.insert(format!("trace:row:{seq}"), r.rect);
                            if r.clicked() && e.entity.is_some() {
                                open = Some(*seq);
                            }
                        }
                    }
                }
            });
        if let Some(seq) = open {
            command = Some(("trace.open", Some(seq.to_string())));
        }
        if let Some((id, arg)) = command {
            if let Err(e) = self.run_command_with(id, arg.as_deref()) {
                self.set_error(format!("{e:#}"));
            }
        }
    }

    /// Profiler tab body.
    pub(super) fn profiler_tab(&mut self, ui: &mut Ui) {
        let icons = self.icons;
        self.debug.profiler.visible = true;
        let mut command = None;
        let build = if cfg!(debug_assertions) {
            "debug build"
        } else {
            "release build"
        };
        header(ui, |ui| {
            let paused = self.debug.profiler.paused;
            let r = theme::icon_button(
                ui,
                icons,
                if paused { Icon::Play } else { Icon::Pause },
                if paused {
                    "Resume (profiler.pause)"
                } else {
                    "Pause (profiler.pause)"
                },
                paused,
            );
            self.rects.insert("profiler:pause".into(), r.rect);
            if r.clicked() {
                command = Some("profiler.pause");
            }
            let r = theme::icon_button(ui, icons, Icon::Reset, "Reset (profiler.reset)", false);
            self.rects.insert("profiler:reset".into(), r.rect);
            if r.clicked() {
                command = Some("profiler.reset");
            }
            theme::chip(
                ui,
                build,
                if cfg!(debug_assertions) {
                    WARNING
                } else {
                    SUCCESS
                },
            )
            .on_hover_text("Debug builds are several times slower; compare release numbers only");
            ui.label(
                RichText::new(format!(
                    "{} frames · CPU wall time (main thread) · hover for scope",
                    self.debug.profiler.len
                ))
                .size(11.0)
                .color(MUTED),
            )
            .on_hover_text(MEASURED);
            if let Some((_, gpu, _, worst)) = self.debug.profiler.gpu_stats() {
                ui.label(
                    RichText::new(format!(
                        "GPU {gpu:.2} ms mean · {worst:.2} max (timestamp queries)"
                    ))
                    .size(11.0)
                    .color(ACCENT),
                )
                .on_hover_text(GPU_MEASURED);
            }
        });
        let previewing = self.previewing;
        // A short dock (the Default bottom dock: about 110 points at 1366x768, about 240 at
        // 1920x1080, where the vertical tables also ran below the fold): the GPU pass groups as
        // a four-row strip on top, a thin graph and tight rows, so every GPU number is visible
        // without scrolling.
        let compact = ui.available_height() < FULL_BODY;
        let (debug, rects) = (&self.debug, &mut self.rects);
        egui::ScrollArea::vertical()
            .id_salt("profiler_body")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                #[cfg(test)]
                rects.insert("profiler:body".into(), ui.clip_rect());
                if compact {
                    ui.spacing_mut().item_spacing.y = 1.0;
                }
                let p = &debug.profiler;
                match p.gpu_timing {
                    Some(Ok(())) => {}
                    Some(Err(why)) => {
                        ui.label(
                            RichText::new(format!("GPU timing unavailable: {why}."))
                                .size(11.0)
                                .color(WARNING),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new("GPU timing unavailable: no GPU renderer.")
                                .size(11.0)
                                .color(WARNING),
                        );
                    }
                }
                let gpu_on = p.gpu_timing == Some(Ok(()));
                let strip = (compact && gpu_on && ui.available_width() >= STRIP_WIDTH)
                    .then(|| gpu_strip(p, ui));
                // Rolling graph: CPU (frame minus swapchain wait) per frame, 16.7 ms budget line.
                if !compact {
                    caption(ui, GRAPH);
                }
                let height = if compact { 28.0 } else { 64.0 };
                let (rect, graph) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width().max(60.0), height),
                    egui::Sense::hover(),
                );
                if compact {
                    graph.on_hover_text(GRAPH);
                }
                rects.insert("profiler:graph".into(), rect);
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 4.0, theme::FIELD);
                let scale = 33.4_f32;
                let y = |ms: f32| rect.max.y - (ms / scale).clamp(0.0, 1.0) * (rect.height() - 4.0);
                painter.hline(
                    rect.x_range(),
                    y(16.7),
                    egui::Stroke::new(1.0_f32, ERROR.gamma_multiply(0.5)),
                );
                let bar = rect.width() / FRAMES as f32;
                let start = FRAMES - p.len;
                for (i, s) in p.samples().enumerate() {
                    let x = rect.min.x + (start + i) as f32 * bar;
                    let cpu = (s.total - s.acquire).max(0.0);
                    let color = if cpu > 16.7 { ERROR } else { SUCCESS };
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(x, y(cpu)),
                            egui::pos2(x + bar.max(1.0), rect.max.y),
                        ),
                        0.0,
                        color.gamma_multiply(0.8),
                    );
                    if s.acquire > 0.0 {
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(x, y(s.total)),
                                egui::pos2(x + bar.max(1.0), y(cpu)),
                            ),
                            0.0,
                            MUTED.gamma_multiply(0.35),
                        );
                    }
                }
                // CPU phases and GPU pass groups side by side whenever two tables fit, both above the
                // fold.
                let (title, last) = if let Some(strip) = strip {
                    cpu_table(p, ui);
                    strip
                } else if gpu_on && ui.available_width() >= 760.0 {
                    ui.columns(2, |columns| {
                        cpu_table(p, &mut columns[0]);
                        gpu_table(p, &mut columns[1])
                    })
                } else {
                    cpu_table(p, ui);
                    if gpu_on {
                        gpu_table(p, ui)
                    } else {
                        (egui::Rect::NOTHING, None)
                    }
                };
                if gpu_on {
                    rects.insert("profiler:gpu".into(), title);
                }
                #[cfg(test)]
                if let Some(last) = last {
                    rects.insert("profiler:gpu:frame".into(), last);
                }
                #[cfg(not(test))]
                let _ = last;
                ui.add_space(4.0);
                if p.systems.is_empty() || !previewing {
                    caption(
                        ui,
                        "Systems: measured while playing (the edit runtime does not step).",
                    );
                } else {
                    table_row(ui, "system step()", ["last ms", "avg ms", "peak ms"], MUTED);
                    for row in &p.systems {
                        let v = [row.last, row.avg, row.peak];
                        table_row(ui, &row.name, v.map(|v| format!("{v:.3}")), TEXT);
                    }
                }
            });
        if let Some(id) = command {
            if let Err(e) = self.run_command(id) {
                self.set_error(format!("{e:#}"));
            }
        }
    }
}

/// The main-thread phase table (last, mean and max over the window).
fn cpu_table(p: &Profiler, ui: &mut Ui) {
    if p.len == 0 {
        caption(ui, if p.paused { "Paused." } else { "Collecting…" });
    } else {
        // Phase table: last, mean and max over the window.
        let mut sum = [0.0f32; PHASES];
        let mut max = [0.0f32; PHASES];
        let (mut total, mut acquire, mut steps, mut worst) = (0.0f32, 0.0f32, 0u32, 0.0f32);
        for s in p.samples() {
            for i in 0..p.phase_count {
                sum[i] += s.phases[i];
                max[i] = max[i].max(s.phases[i]);
            }
            total += s.total;
            acquire += s.acquire;
            steps += u32::from(s.steps);
            worst = worst.max(s.total - s.acquire);
        }
        let n = p.len as f32;
        let last = p.frames[(p.head + FRAMES - 1) % FRAMES];
        ui.add_space(4.0);
        table_row(ui, "phase", ["last ms", "mean ms", "max ms"], MUTED);
        for i in 0..p.phase_count {
            let v = [last.phases[i], sum[i] / n, max[i]];
            table_row(ui, p.names[i], v.map(|v| format!("{v:.2}")), TEXT);
        }
        let v = [last.acquire, acquire / n];
        table_row(
            ui,
            "acquire wait",
            [
                format!("{:.2}", v[0]),
                format!("{:.2}", v[1]),
                String::new(),
            ],
            MUTED,
        );
        let v = [last.total - last.acquire, (total - acquire) / n, worst];
        table_row(ui, "frame CPU", v.map(|v| format!("{v:.2}")), ACCENT);
        caption(
            ui,
            format!("{:.2} fixed steps / frame (mean)", steps as f32 / n),
        );
    }
}

/// The GPU pass-group table (timestamp queries); returns its title's rect and, once measured,
/// its last row's (frame GPU).
fn gpu_table(p: &Profiler, ui: &mut Ui) -> (egui::Rect, Option<egui::Rect>) {
    ui.add_space(4.0);
    let r = ui.label(
        RichText::new(format!(
            "GPU (timestamp queries) · {} frames measured · {} not measured · hover for scope",
            p.gpu_len, p.gpu_dropped
        ))
        .size(11.0)
        .color(MUTED),
    );
    let rect = r.rect;
    r.on_hover_text(GPU_MEASURED);
    let last = match p.gpu_stats() {
        None => {
            caption(
                ui,
                if p.paused {
                    "Paused."
                } else {
                    "Waiting for the first readback…"
                },
            );
            None
        }
        Some((mean, total, max, worst)) => {
            let last = p.gpu_last();
            table_row(
                ui,
                "GPU pass group",
                ["last ms", "mean ms", "max ms"],
                MUTED,
            );
            for s in Section::ALL {
                let i = s as usize;
                if max[i] > 0.0 {
                    let v = [last.ms[i], mean[i], max[i]];
                    table_row(ui, s.name(), v.map(|v| format!("{v:.3}")), TEXT);
                }
            }
            let v = [last.total, total, worst];
            Some(table_row(
                ui,
                "frame GPU",
                v.map(|v| format!("{v:.3}")),
                ACCENT,
            ))
        }
    };
    (rect, last)
}

/// Short dock: the GPU pass groups as columns and last / mean / max as rows (four rows, so
/// it fits the 1366x768 bottom dock); returns the title cell's rect and the last row's.
fn gpu_strip(p: &Profiler, ui: &mut Ui) -> (egui::Rect, Option<egui::Rect>) {
    let Some((mean, total, max, worst)) = p.gpu_stats() else {
        let r = ui.label(
            RichText::new(if p.paused {
                "GPU (timestamp queries): paused."
            } else {
                "GPU (timestamp queries): waiting for the first readback…"
            })
            .size(11.0)
            .color(MUTED),
        );
        return (r.rect, None);
    };
    let last = p.gpu_last();
    let shown = |s: &Section| max[*s as usize] > 0.0;
    let columns = Section::ALL.iter().filter(|s| shown(s)).count() + 1;
    let width = ((ui.available_width() - NAME_WIDTH) / columns as f32 - 6.0).clamp(44.0, 96.0);
    let mut title = egui::Rect::NOTHING;
    let mut bottom = None;
    for row in 0..4 {
        let rect = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.spacing_mut().interact_size.y = 16.0;
                if row == 0 {
                    let r = cell(ui, NAME_WIDTH, egui::Align::Min, mono("GPU ms", MUTED));
                    title = r.rect;
                    r.on_hover_ui(|ui| {
                        ui.label(format!(
                            "Timestamp queries: {} frames measured, {} not measured.",
                            p.gpu_len, p.gpu_dropped
                        ));
                        ui.label(GPU_MEASURED);
                    });
                } else {
                    let name = ["last", "mean", "max"][row - 1];
                    cell(ui, NAME_WIDTH, egui::Align::Min, mono(name, MUTED));
                }
                let value = |v: [f32; 3]| format!("{:.3}", v[row - 1]);
                for s in Section::ALL.into_iter().filter(shown) {
                    let i = s as usize;
                    let text = if row == 0 {
                        mono(s.name(), MUTED)
                    } else {
                        mono(value([last.ms[i], mean[i], max[i]]), TEXT)
                    };
                    cell(ui, width, egui::Align::Max, text);
                }
                let text = if row == 0 {
                    mono("frame", ACCENT)
                } else {
                    mono(value([last.total, total, worst]), ACCENT)
                };
                cell(ui, width, egui::Align::Max, text);
            })
            .response
            .rect;
        if row == 3 {
            bottom = Some(rect);
        }
    }
    (title, bottom)
}

/// Milliseconds with three decimals for `observe` (f32 noise trimmed).
fn round3(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

/// One profiler table row: a name column and three right-aligned value columns (fixed
/// widths, no `Grid`: a new Grid discards its first pass and with it that frame's clicks).
fn table_row(
    ui: &mut Ui,
    name: &str,
    values: [impl Into<String>; 3],
    color: egui::Color32,
) -> egui::Rect {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let name_width = (ui.available_width() - 3.0 * 70.0).clamp(90.0, 220.0);
        cell(ui, name_width, egui::Align::Min, mono(name, color));
        for v in values {
            cell(ui, 64.0, egui::Align::Max, mono(v, color));
        }
    })
    .response
    .rect
}

/// One fixed-width, truncated table cell.
fn cell(ui: &mut Ui, width: f32, align: egui::Align, text: RichText) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 16.0),
        egui::Layout::left_to_right(egui::Align::Center).with_main_align(align),
        |ui| {
            ui.set_min_width(width);
            ui.add(egui::Label::new(text).truncate())
        },
    )
    .inner
}

fn mono(text: impl Into<String>, color: egui::Color32) -> RichText {
    RichText::new(text).monospace().size(11.5).color(color)
}

fn parse_seq(arg: Option<&str>) -> anyhow::Result<u64> {
    arg.and_then(|a| a.trim().parse().ok())
        .ok_or_else(|| anyhow::anyhow!("expects the entry number (SEQ)"))
}

fn levels_arg(levels: &[bool; 3]) -> String {
    let names: Vec<&str> = Level::ALL
        .into_iter()
        .filter(|l| levels[*l as usize])
        .map(Level::name)
        .collect();
    format!("levels={}", names.join(","))
}

fn kinds_arg(kinds: &[bool; 5]) -> String {
    let names: Vec<&str> = TraceKind::ALL
        .into_iter()
        .filter(|k| kinds[*k as usize])
        .map(TraceKind::name)
        .collect();
    format!("kinds={}", names.join(","))
}

/// Profiler body height below which the graph is thin and rows are tight (graph, captions and
/// two full tables need about this much).
const FULL_BODY: f32 = 300.0;
/// Row-name column of the short-dock GPU strip.
const NAME_WIDTH: f32 = 56.0;
/// Narrowest body the strip fits with every pass group and the frame total (8 columns of at
/// least 44 + 6 spacing); narrower docks keep the vertical table.
const STRIP_WIDTH: f32 = NAME_WIDTH + 8.0 * 50.0;
const GRAPH: &str = "CPU ms per frame, last 240 frames (grey = swapchain wait; red line = 16.7 ms; bars over it are red)";
/// What the Profiler measures (its header's tooltip).
const MEASURED: &str = "Main-thread wall time of each frame phase (Instant). render_present = world + UI encoding, submit and present, including the swapchain wait shown separately as \"acquire wait\" (grey in the graph). Systems = each runtime system's step(), summed over this frame's fixed steps (Play only). Not GPU time.";
const GPU_MEASURED: &str = "GPU time from timestamp queries the renderer writes between its pass groups (only while this tab is visible): vfx_compute, shadows (sun depth), world (each camera view), vfx_draw, post, game_ui (Game tab interface), ui (editor interface). Each value is the GPU time between the previous timestamp and the one after that group, summed per frame. Results are read back asynchronously a few frames late; a frame is not measured while every readback slot is still busy. Not CPU time.";
