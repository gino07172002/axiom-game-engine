//! Scenario recording during Play: the fixed-step input changes and the runtime commands of
//! one Play session, written as a `*.scenario.json` that `axiom scenario FILE` replays.
//!
//! Tick numbering follows `scenario::run`: inputs and commands with tick `t` apply before the
//! simulation's step `t` (0-based), i.e. with the runtime's tick at `t` before that step.
use crate::scenario::{Assertion, Scenario, ScenarioCommand, ScenarioInput, ScenarioOperation};
use crate::world::ActionMap;
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
    /// Mouse-look / screen-pointer events: not representable in a scenario.
    pub pointer_events: u64,
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

    fn operation(&mut self, operation: ScenarioOperation) {
        if self.full.is_some() {
            return;
        }
        if self.commands.len() >= MAX_ENTRIES {
            self.stop(format!("{MAX_ENTRIES} commands recorded (scenario limit)"));
            return;
        }
        self.commands.push(ScenarioCommand {
            tick: self.ticks,
            operation,
        });
    }

    /// A runtime command that succeeded (game UI, bridge `command`).
    pub(crate) fn command(&mut self, command: &RuntimeCommand) {
        self.operation(ScenarioOperation::Command {
            command: command.clone(),
        });
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
        self.operation(ScenarioOperation::Save { slot: slot.into() });
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
        self.operation(ScenarioOperation::Load { slot: slot.into() });
    }

    pub(crate) fn inputs_len(&self) -> usize {
        self.inputs.len()
    }
    pub(crate) fn commands_len(&self) -> usize {
        self.commands.len()
    }

    pub(crate) fn pointer(&mut self) {
        if self.full.is_none() {
            self.pointer_events += 1;
        }
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
        // Commands after the last step would need one more step to replay.
        let trailing = self
            .commands
            .iter()
            .filter(|c| c.tick >= self.ticks)
            .count();
        self.commands.retain(|c| c.tick < self.ticks);
        if trailing > 0 {
            self.inexact(format!(
                "{trailing} command(s) after the last tick were dropped (pause, command, stop)"
            ));
        }
        if self.pointer_events > 0 {
            self.inexact(format!(
                "{} mouse-look / pointer event(s) are not part of the scenario format",
                self.pointer_events
            ));
        }
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
        // The trailing command (after the last step) is dropped: no hash assertion.
        assert_eq!(summary["exact"], false);
        let s: Scenario = serde_json::from_str(&text).unwrap();
        assert_eq!((s.ticks, s.inputs.len(), s.commands.len()), (4, 2, 1));
        assert!(!text.contains("state_hash"));
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
