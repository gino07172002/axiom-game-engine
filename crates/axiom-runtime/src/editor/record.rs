//! Scenario recording during Play: the fixed-step input changes and the runtime commands of
//! one Play session, written as a `*.scenario.json` that `axiom scenario FILE` replays.
//!
//! Tick numbering follows `scenario::run`: inputs and commands with tick `t` apply before the
//! simulation's step `t` (0-based), i.e. with the runtime's tick at `t` before that step.
use crate::scenario::{
    Assertion, Scenario, ScenarioCommand, ScenarioInput, ScenarioOperation, MAX_LOOK_DELTAS,
};
use crate::world::{ActionMap, PointerKind};
use anyhow::{ensure, Context, Result};
use axiom_core::gameplay::RuntimeCommand;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The limits `scenario::run` and `axiom scenario` accept.
const MAX_TICKS: u32 = 1_000_000;
const MAX_ENTRIES: usize = 4096;
const MAX_ACTIONS: usize = 64;
const MAX_BYTES: usize = 1_048_576;
/// Upper bounds of the bytes elements add to the file: an f32 prints in at most 17
/// characters (`-0.00000123456789`), so a look delta `[x,y],` takes at most 40 and a pointer
/// operation line (or the fixed part of any line) at most 160. Every element is charged
/// against `BYTE_BUDGET` before it is recorded; the recording stops instead of growing past
/// what `axiom scenario` reads (the rest of `MAX_BYTES` is the header and assertions).
const LOOK_DELTA_BYTES: usize = 40;
const OPERATION_BYTES: usize = 160;
const BYTE_BUDGET: usize = MAX_BYTES - 4096;

/// Compact JSON of one scenario element (one line of the file).
fn line<T: serde::Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

pub(crate) struct Recorder {
    pub name: String,
    pub path: PathBuf,
    /// Fixed steps recorded so far (the next input / command gets this tick).
    pub ticks: u32,
    last: BTreeMap<String, f32>,
    inputs: Vec<ScenarioInput>,
    commands: Vec<ScenarioCommand>,
    saves: BTreeSet<String>,
    /// Mouse-look deltas and screen-pointer events recorded (`look` / `pointer` operations).
    pub pointer_events: u64,
    /// Upper bound of the file bytes recorded so far (inputs and operations).
    bytes: usize,
    /// A primary press reached the runtime and its release has not: moves are recorded only
    /// then (see `PointerKind`).
    dragging: bool,
    /// Why the file will not replay to the same state (first reason).
    pub inexact: Option<String>,
    /// A budget was reached: nothing more is recorded; the host saves the file.
    pub full: Option<String>,
}

/// Recording names: 1..=64 of `A-Z a-z 0-9 - _ .`, not starting with a dot.
fn valid_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "recording name must be 1..64 of A-Z a-z 0-9 - _ . (written as examples/NAME.scenario.json)"
    );
    Ok(())
}

/// `WORLD-recording-N` with the first N whose file does not exist yet.
pub(crate) fn free_name(dir: &Path, world: &str) -> String {
    let stem: String = world
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    let stem = if stem.is_empty() {
        "world".into()
    } else {
        stem
    };
    (1..10_000)
        .map(|n| format!("{stem}-recording-{n}"))
        .find(|name| !dir.join(format!("{name}.scenario.json")).exists())
        .unwrap_or_else(|| format!("{stem}-recording"))
}

impl Recorder {
    pub(crate) fn new(dir: &Path, name: &str) -> Result<Self> {
        let name = name.strip_suffix(".scenario.json").unwrap_or(name);
        valid_name(name)?;
        let path = dir.join(format!("{name}.scenario.json"));
        ensure!(
            !path.exists(),
            "{} already exists; recordings never overwrite a file",
            path.display()
        );
        Ok(Self {
            name: name.to_owned(),
            path,
            ticks: 0,
            last: BTreeMap::new(),
            inputs: vec![],
            commands: vec![],
            saves: BTreeSet::new(),
            pointer_events: 0,
            bytes: 0,
            dragging: false,
            inexact: None,
            full: None,
        })
    }

    /// Ends recording at a limit. The live session goes on unrecorded, so the file can no
    /// longer reach the session's final state.
    fn stop(&mut self, reason: String) {
        if self.full.is_none() {
            self.inexact(format!("recording stopped early: {reason}"));
            self.full = Some(reason);
        }
    }

    fn inexact(&mut self, reason: impl Into<String>) {
        if self.inexact.is_none() {
            self.inexact = Some(reason.into());
        }
    }

    /// The host found a reason the replay will differ (e.g. the revision changed).
    pub(crate) fn mark_inexact(&mut self, reason: impl Into<String>) {
        self.inexact(reason);
    }

    /// Before each fixed step: records the actions whose value changed since the last
    /// recorded input (a released action becomes 0). Allocates only when something changed.
    pub(crate) fn step(&mut self, input: &ActionMap) {
        if self.full.is_some() {
            return;
        }
        if self.ticks >= MAX_TICKS {
            self.stop(format!("{MAX_TICKS} ticks recorded (scenario limit)"));
            return;
        }
        let value = |v: f32| {
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        let changed = input
            .0
            .iter()
            .any(|(k, v)| self.last.get(k).copied().unwrap_or(0.0) != value(*v))
            || self
                .last
                .iter()
                .any(|(k, v)| *v != 0.0 && !input.0.contains_key(k));
        if changed {
            let mut actions = BTreeMap::new();
            for (k, v) in &input.0 {
                if self.last.get(k).copied().unwrap_or(0.0) != value(*v) {
                    actions.insert(k.clone(), value(*v));
                }
            }
            for (k, v) in &self.last {
                if *v != 0.0 && !input.0.contains_key(k) {
                    actions.insert(k.clone(), 0.0);
                }
            }
            if self.inputs.len() >= MAX_ENTRIES {
                self.stop(format!(
                    "{MAX_ENTRIES} input changes recorded (scenario limit)"
                ));
                return;
            }
            if actions.len() > MAX_ACTIONS {
                self.stop(format!(
                    "{} actions changed in one tick (scenario limit {MAX_ACTIONS})",
                    actions.len()
                ));
                return;
            }
            let bytes = OPERATION_BYTES
                + actions
                    .keys()
                    .map(|k| k.len() + LOOK_DELTA_BYTES)
                    .sum::<usize>();
            if !self.room(bytes) {
                return;
            }
            for (k, v) in &actions {
                self.last.insert(k.clone(), *v);
            }
            self.inputs.push(ScenarioInput {
                tick: self.ticks,
                actions,
            });
        }
        self.ticks += 1;
    }

    /// Appends an operation of at most `bytes` file bytes (or ends the recording at a limit).
    fn operation(&mut self, operation: ScenarioOperation, bytes: usize) {
        if self.full.is_some() {
            return;
        }
        if self.commands.len() >= MAX_ENTRIES {
            self.stop(format!("{MAX_ENTRIES} commands recorded (scenario limit)"));
            return;
        }
        if !self.room(bytes) {
            return;
        }
        self.commands.push(ScenarioCommand {
            tick: self.ticks,
            operation,
        });
    }

    /// Charges `bytes` against the file budget; false (and the recording ends) when the file
    /// could grow past what `axiom scenario` reads.
    fn room(&mut self, bytes: usize) -> bool {
        if self.full.is_some() {
            return false;
        }
        if self.bytes + bytes > BYTE_BUDGET {
            self.stop(format!(
                "the recording reached the {} KiB scenario file budget",
                MAX_BYTES / 1024
            ));
            return false;
        }
        self.bytes += bytes;
        true
    }

    /// A runtime command that succeeded (game UI, bridge `command`).
    pub(crate) fn command(&mut self, command: &RuntimeCommand) {
        if self.full.is_some() {
            return;
        }
        // Commands are rare (UI clicks, bridge requests): measure the actual line.
        let bytes = OPERATION_BYTES + serde_json::to_string(command).map_or(MAX_BYTES, |s| s.len());
        self.operation(
            ScenarioOperation::Command {
                command: command.clone(),
            },
            bytes,
        );
    }

    pub(crate) fn save(&mut self, slot: &str) {
        if self.full.is_some() {
            return;
        }
        if self.saves.len() >= 16 && !self.saves.contains(slot) {
            // `scenario::run` refuses a 17th slot: end the recording before it.
            self.stop(format!(
                "a 17th save slot ({slot}); scenarios keep at most 16"
            ));
            return;
        }
        self.saves.insert(slot.to_owned());
        self.operation(
            ScenarioOperation::Save { slot: slot.into() },
            OPERATION_BYTES + slot.len(),
        );
    }

    /// A load: replayable only from a save made earlier in this recording.
    pub(crate) fn load(&mut self, slot: &str) {
        if !self.saves.contains(slot) {
            self.inexact(format!(
                "loaded save {slot} that was not made during the recording"
            ));
            self.stop(format!("load of {slot} (made outside the recording)"));
            return;
        }
        self.operation(
            ScenarioOperation::Load { slot: slot.into() },
            OPERATION_BYTES + slot.len(),
        );
    }

    pub(crate) fn inputs_len(&self) -> usize {
        self.inputs.len()
    }
    pub(crate) fn commands_len(&self) -> usize {
        self.commands.len()
    }

    /// A mouse-look delta the runtime just applied (`WorldRuntime::pointer`). Consecutive
    /// deltas of one tick share a `look` operation and replay in the same order.
    pub(crate) fn look(&mut self, dx: f32, dy: f32) {
        let ticks = self.ticks;
        let extends = matches!(
            self.commands.last(),
            Some(ScenarioCommand { tick, operation: ScenarioOperation::Look { deltas } })
                if *tick == ticks && deltas.len() < MAX_LOOK_DELTAS
        );
        if !extends {
            self.operation(
                ScenarioOperation::Look {
                    deltas: vec![[dx, dy]],
                },
                OPERATION_BYTES + LOOK_DELTA_BYTES,
            );
        } else if self.room(LOOK_DELTA_BYTES) {
            if let Some(ScenarioCommand {
                operation: ScenarioOperation::Look { deltas },
                ..
            }) = self.commands.last_mut()
            {
                deltas.push([dx, dy]);
            }
        }
        if self.full.is_none() {
            self.pointer_events += 1;
        }
    }

    /// A screen pointer event the runtime just applied (`WorldRuntime::screen_pointer`) with
    /// these arguments. Moves count only during a drag, and a move right after a move of the
    /// same tick replaces it (see `PointerKind`).
    pub(crate) fn pointer(
        &mut self,
        kind: PointerKind,
        screen: [f32; 2],
        aspect: f32,
        additive: bool,
    ) {
        if self.full.is_some() {
            return;
        }
        match kind {
            PointerKind::PrimaryDown => self.dragging = true,
            PointerKind::PrimaryUp => self.dragging = false,
            PointerKind::Move if !self.dragging => return,
            _ => {}
        }
        self.pointer_events += 1;
        let ticks = self.ticks;
        if kind == PointerKind::Move {
            if let Some(ScenarioCommand {
                tick,
                operation:
                    ScenarioOperation::Pointer {
                        kind: PointerKind::Move,
                        screen: last,
                        aspect: last_aspect,
                        additive: last_additive,
                    },
            }) = self.commands.last_mut()
            {
                if *tick == ticks && *last_aspect == aspect && *last_additive == additive {
                    *last = screen;
                    return;
                }
            }
        }
        self.operation(
            ScenarioOperation::Pointer {
                kind,
                screen,
                aspect,
                additive,
            },
            OPERATION_BYTES,
        );
    }
    /// A pointer event failed part-way in the live runtime: the state it left cannot be
    /// reproduced (a replay stops at the error).
    pub(crate) fn pointer_failed(&mut self, error: &str) {
        self.inexact(format!("a pointer event failed during Play: {error}"));
    }

    pub(crate) fn observation(&self) -> Value {
        json!({"name":self.name,"path":self.path.to_string_lossy().replace('\\', "/"),"ticks":self.ticks,
            "inputs":self.inputs.len(),"commands":self.commands.len(),"pointer_events":self.pointer_events,
            "inexact":self.inexact,"stopped":self.full})
    }

    /// The scenario text: one input / command per line. `hash` = the Play runtime's
    /// `state_hash` now; it becomes a `state_hash` assertion when the replay can be exact.
    fn scenario(&mut self, hash: &str) -> Result<(String, Value)> {
        ensure!(
            self.ticks > 0,
            "nothing recorded: no fixed step ran (paused the whole time?)"
        );
        // Operations after the last step keep tick `ticks`: `scenario::run` applies them
        // after its last step, before the assertions.
        let exact = self.inexact.is_none();
        let mut assertions = vec![Assertion::Finite {}];
        if exact {
            assertions.push(Assertion::StateHash {
                value: hash.to_owned(),
            });
        }
        let mut text = format!("{{\"ticks\":{},\n \"inputs\":[", self.ticks);
        for (i, input) in self.inputs.iter().enumerate() {
            text.push_str(if i == 0 { "\n  " } else { ",\n  " });
            text.push_str(&line(input)?);
        }
        text.push_str("\n ],\n \"commands\":[");
        for (i, command) in self.commands.iter().enumerate() {
            text.push_str(if i == 0 { "\n  " } else { ",\n  " });
            text.push_str(&line(command)?);
        }
        text.push_str("\n ],\n \"assertions\":[");
        for (i, assertion) in assertions.iter().enumerate() {
            text.push_str(if i == 0 { "\n  " } else { ",\n  " });
            text.push_str(&line(assertion)?);
        }
        text.push_str("\n ]}\n");
        ensure!(
            text.len() <= MAX_BYTES,
            "recording is {} bytes; `axiom scenario` reads at most 1 MiB",
            text.len()
        );
        // The file must load as a scenario (same parser as the CLI).
        let _: Scenario = serde_json::from_str(&text).context("recorded scenario is invalid")?;
        let summary = json!({"path":self.path.to_string_lossy().replace('\\', "/"),"ticks":self.ticks,
            "inputs":self.inputs.len(),"commands":self.commands.len(),"state_hash":hash,
            "exact":exact,"inexact":self.inexact,"stopped":self.full,"bytes":text.len()});
        Ok((text, summary))
    }

    /// Writes the file (never over an existing one) and returns its summary.
    pub(crate) fn finish(mut self, hash: &str) -> Result<Value> {
        use std::io::Write;
        let (text, summary) = self.scenario(hash)?;
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .with_context(|| format!("cannot create {}", self.path.display()))?;
        if let Err(e) = file
            .write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
        {
            drop(file);
            let _ = std::fs::remove_file(&self.path);
            return Err(e).with_context(|| format!("cannot write {}", self.path.display()));
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, f32)]) -> ActionMap {
        ActionMap(pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect())
    }

    #[test]
    fn records_changes_only_and_releases_as_zero() {
        let dir = std::env::temp_dir().join(format!("axiom-rec-{}", std::process::id()));
        let mut r = Recorder::new(&dir, "unit").unwrap();
        r.step(&map(&[("move_z", 0.0), ("jump", 0.0)]));
        r.step(&map(&[("move_z", 1.0), ("jump", 0.0)]));
        r.step(&map(&[("move_z", 1.0), ("jump", 0.0)]));
        r.command(&RuntimeCommand::Locale {
            locale: "en".into(),
        });
        // Play ▸ Step uses an empty map: the held axis is released.
        r.step(&ActionMap::default());
        assert_eq!(r.ticks, 4);
        assert_eq!(r.inputs.len(), 2);
        assert_eq!((r.inputs[0].tick, r.inputs[0].actions["move_z"]), (1, 1.0));
        assert_eq!((r.inputs[1].tick, r.inputs[1].actions["move_z"]), (3, 0.0));
        assert_eq!(r.commands[0].tick, 3);
        r.command(&RuntimeCommand::Locale {
            locale: "ja".into(),
        });
        let (text, summary) = r.scenario("abc").unwrap();
        // The trailing command (after the last step) keeps tick 4 = ticks: replayed after
        // the last step, so the recording stays exact.
        assert_eq!(summary["exact"], true);
        let s: Scenario = serde_json::from_str(&text).unwrap();
        assert_eq!((s.ticks, s.inputs.len(), s.commands.len()), (4, 2, 2));
        assert_eq!(s.commands[1].tick, 4);
        assert!(text.contains("state_hash"));
    }

    #[test]
    fn mouse_look_and_pointer_events_become_ordered_operations() {
        let dir = std::env::temp_dir().join(format!("axiom-rec4-{}", std::process::id()));
        let mut r = Recorder::new(&dir, "mouse").unwrap();
        // Tick 0: two deltas share one op; a click splits the run; a third delta after it.
        r.look(1.5, -2.0);
        r.look(0.1, 0.0);
        r.pointer(PointerKind::PrimaryDown, [0.25, 0.5], 1.5, false);
        r.look(-3.0, 1e-7);
        // Moves of one tick coalesce (last wins); a move in the next tick is new.
        r.pointer(PointerKind::Move, [0.3, 0.5], 1.5, false);
        r.pointer(PointerKind::Move, [0.4, 0.5], 1.5, false);
        r.step(&ActionMap::default());
        r.pointer(PointerKind::Move, [0.5, 0.5], 1.5, false);
        r.pointer(PointerKind::PrimaryUp, [0.5, 0.5], 1.5, true);
        assert_eq!(r.pointer_events, 8);
        let (text, summary) = r.scenario("h").unwrap();
        assert_eq!(summary["exact"], true, "{summary}");
        let s: Scenario = serde_json::from_str(&text).unwrap();
        let ops: Vec<(u32, String)> = s
            .commands
            .iter()
            .map(|c| (c.tick, serde_json::to_string(&c.operation).unwrap()))
            .collect();
        assert_eq!(ops.len(), 6, "{text}");
        assert_eq!(ops[0].1, r#"{"op":"look","deltas":[[1.5,-2.0],[0.1,0.0]]}"#);
        assert!(ops[1].1.contains(r#""kind":"primary_down""#));
        // f32 values round-trip exactly through the file.
        match &s.commands[2].operation {
            ScenarioOperation::Look { deltas } => assert_eq!(deltas, &vec![[-3.0, 1e-7]]),
            other => panic!("{other:?}"),
        }
        assert!(ops[3].1.contains(r#""screen":[0.4,0.5]"#), "{}", ops[3].1);
        assert_eq!((ops[4].0, ops[5].0), (1, 1));
        assert!(ops[5].1.contains(r#""additive":true"#));
        // A run of look deltas splits at the per-op limit.
        let mut r = Recorder::new(&dir, "long").unwrap();
        for _ in 0..MAX_LOOK_DELTAS + 1 {
            r.look(1.0, 0.0);
        }
        assert_eq!(r.commands.len(), 2);
        // Endless mouse input ends the recording before the file outgrows `axiom scenario`.
        let mut r = Recorder::new(&dir, "flood").unwrap();
        for i in 0..200_000 {
            r.look(1.0, 0.0);
            if i % 16 == 0 {
                r.step(&ActionMap::default());
            }
        }
        assert!(r.full.is_some() && r.inexact.is_some());
        let (text, _) = r.scenario("h").unwrap();
        assert!(text.len() <= MAX_BYTES && !text.contains("state_hash"));
        // Inputs and commands share the budget: worst-case floats plus big commands and
        // many input changes still end in a file `axiom scenario` reads.
        let mut r = Recorder::new(&dir, "mixed").unwrap();
        let big = RuntimeCommand::Locale {
            locale: "x".repeat(2000),
        };
        for i in 0..100_000 {
            r.look(-1.234_567_9e-6, f32::MIN_POSITIVE);
            if i % 5 == 0 {
                r.command(&big);
            }
            let held = if i % 2 == 0 { 1.0 } else { 0.0 };
            r.step(&map(&[("move_z", held), ("move_x", -held)]));
        }
        assert!(r.full.is_some());
        let (text, _) = r.scenario("h").unwrap();
        assert!(text.len() <= MAX_BYTES, "{}", text.len());
        // Moves without a held primary button change nothing and are not recorded.
        let mut r = Recorder::new(&dir, "hover").unwrap();
        r.pointer(PointerKind::Move, [0.1, 0.1], 1.0, false);
        r.pointer(PointerKind::SecondaryDown, [0.1, 0.1], 1.0, false);
        r.pointer(PointerKind::Move, [0.2, 0.1], 1.0, false);
        assert_eq!((r.commands.len(), r.pointer_events), (1, 1));
        // A failed pointer event makes the recording inexact.
        let mut r = Recorder::new(&dir, "failed").unwrap();
        r.step(&ActionMap::default());
        r.pointer_failed("boom");
        assert_eq!(r.scenario("h").unwrap().1["exact"], false);
    }

    #[test]
    fn names_budgets_and_loads() {
        let dir = std::env::temp_dir().join(format!("axiom-rec2-{}", std::process::id()));
        for bad in ["", "../x", "a/b", ".hidden", &"x".repeat(65)] {
            assert!(Recorder::new(&dir, bad).is_err(), "{bad}");
        }
        assert_eq!(free_name(&dir, "axiom town!"), "axiom-town--recording-1");
        let mut r = Recorder::new(&dir, "ok.scenario.json").unwrap();
        assert_eq!(r.name, "ok");
        assert!(r.scenario("h").is_err(), "no step recorded");
        r.load("quick");
        assert!(r.full.is_some() && r.inexact.is_some());
        let mut r = Recorder::new(&dir, "ok").unwrap();
        r.step(&ActionMap::default());
        r.save("a");
        r.step(&ActionMap::default());
        r.load("a");
        r.step(&ActionMap::default());
        let (text, summary) = r.scenario("h").unwrap();
        assert_eq!(summary["exact"], true);
        assert!(text.contains(r#"{"kind":"state_hash","value":"h"}"#));
        let wide: Vec<(String, f32)> = (0..65).map(|i| (format!("a{i}"), 1.0)).collect();
        let mut r = Recorder::new(&dir, "wide").unwrap();
        r.step(&ActionMap(wide.into_iter().collect()));
        assert!(r.full.is_some());
    }

    #[test]
    fn a_limit_ends_the_recording_without_a_state_hash() {
        let dir = std::env::temp_dir().join(format!("axiom-rec3-{}", std::process::id()));
        let mut r = Recorder::new(&dir, "slots").unwrap();
        r.step(&ActionMap::default());
        for i in 0..17 {
            r.save(&format!("s{i}"));
        }
        // The 17th slot (which `scenario::run` refuses) ends the recording, inexact.
        assert!(r.full.is_some() && r.inexact.is_some());
        r.step(&ActionMap::default());
        r.command(&RuntimeCommand::Locale {
            locale: "en".into(),
        });
        assert_eq!((r.ticks, r.commands.len()), (1, 16));
        let (text, summary) = r.scenario("h").unwrap();
        assert_eq!(summary["exact"], false);
        assert!(!text.contains("state_hash"));
        // The host can also mark a recording inexact (e.g. the revision changed).
        let mut r = Recorder::new(&dir, "rev").unwrap();
        r.step(&ActionMap::default());
        r.mark_inexact("revision changed");
        assert_eq!(r.scenario("h").unwrap().1["exact"], false);
    }
}
