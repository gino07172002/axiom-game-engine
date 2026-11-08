use crate::world::{ActionMap, RuntimeRegistry, WorldRuntime};
use anyhow::{ensure, Context, Result};
use axiom_core::World;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub ticks: u32,
    #[serde(default)]
    pub inputs: Vec<ScenarioInput>,
    #[serde(default)]
    pub commands: Vec<ScenarioCommand>,
    pub assertions: Vec<Assertion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioCommand {
    pub tick: u32,
    pub operation: ScenarioOperation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScenarioOperation {
    Command {
        command: axiom_core::gameplay::RuntimeCommand,
    },
    Save {
        slot: String,
    },
    Load {
        slot: String,
    },
    Assert {
        assertion: Assertion,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioInput {
    pub tick: u32,
    pub actions: BTreeMap<String, f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Assertion {
    Equals {
        pointer: String,
        value: Value,
    },
    Choice {
        id: String,
        visible: bool,
    },
    Finite {},
    Position {
        entity: String,
        axis: usize,
        min: f32,
        max: f32,
    },
    Metric {
        name: String,
        min: f64,
        max: f64,
    },
}
pub fn run(world: World, rev: i64, registry: RuntimeRegistry, scenario: Scenario) -> Result<Value> {
    run_with_saves(world, rev, registry, scenario, None)
}
pub fn run_with_saves(
    world: World,
    rev: i64,
    registry: RuntimeRegistry,
    scenario: Scenario,
    save_root: Option<&std::path::Path>,
) -> Result<Value> {
    ensure!(
        scenario.ticks <= 1_000_000
            && scenario.inputs.len() <= 4096
            && scenario.assertions.len() <= 256
            && scenario.commands.len() <= 4096,
        "scenario budget exceeded"
    );
    let mut previous = None;
    for command in &scenario.commands {
        ensure!(
            command.tick < scenario.ticks && previous.is_none_or(|p| p <= command.tick),
            "scenario command ticks must be ordered and in range"
        );
        previous = Some(command.tick);
    }
    previous = None;
    for input in &scenario.inputs {
        ensure!(
            input.tick < scenario.ticks && previous.is_none_or(|p| p < input.tick),
            "scenario input ticks must be strictly increasing and in range"
        );
        ensure!(
            input.actions.len() <= 64
                && input
                    .actions
                    .values()
                    .all(|v| v.is_finite() && (-1.0..=1.0).contains(v)),
            "invalid action values"
        );
        previous = Some(input.tick);
    }
    for a in &scenario.assertions {
        match a {
            Assertion::Position { axis, min, max, .. } => ensure!(
                *axis < 3 && min.is_finite() && max.is_finite() && min <= max,
                "invalid position assertion"
            ),
            Assertion::Metric { min, max, .. } => ensure!(
                min.is_finite() && max.is_finite() && min <= max,
                "invalid metric assertion"
            ),
            _ => {}
        }
    }
    let mut runtime = WorldRuntime::new(world, rev, registry)?;
    for input in &scenario.inputs {
        for name in input.actions.keys() {
            ensure!(
                runtime.bindings.axes.contains_key(name)
                    || runtime.bindings.buttons.contains_key(name),
                "unknown scenario action {name}"
            );
        }
    }
    let mut input = ActionMap::default();
    let mut next = 0;
    let mut commands = scenario.commands.iter().peekable();
    let mut saves = BTreeMap::new();
    let mut intermediate = vec![];
    for tick in 0..scenario.ticks {
        while commands.peek().is_some_and(|c| c.tick == tick) {
            let command = commands.next().unwrap();
            match &command.operation {
                ScenarioOperation::Command { command } => {
                    runtime.command(command)?;
                }
                ScenarioOperation::Save { slot } => {
                    ensure!(
                        saves.contains_key(slot) || saves.len() < 16,
                        "scenario checkpoint slot budget exceeded"
                    );
                    let checkpoint = runtime.checkpoint()?;
                    if let Some(root) = save_root {
                        runtime.save_slot(root, slot)?;
                    }
                    saves.insert(slot.clone(), checkpoint);
                }
                ScenarioOperation::Load { slot } => {
                    runtime.restore_checkpoint(
                        saves.get(slot).context("scenario checkpoint missing")?,
                    )?;
                }
                ScenarioOperation::Assert { assertion } => {
                    intermediate.push(check(&runtime, assertion)?);
                }
            }
        }
        if let Some(event) = scenario.inputs.get(next) {
            if event.tick == tick {
                for (k, v) in &event.actions {
                    input.0.insert(k.clone(), *v);
                }
                next += 1;
            }
        }
        runtime.step(&input);
        runtime.assert_valid()?;
    }
    let observation = json!(runtime.observation());
    let mut results = vec![];
    for a in &scenario.assertions {
        results.push(check(&runtime, a)?);
    }
    Ok(
        json!({"ok":true,"ticks":scenario.ticks,"assertions":results,"intermediate_assertions":intermediate,"checkpoints":saves.keys().collect::<Vec<_>>(),"observation":observation}),
    )
}
fn check(runtime: &WorldRuntime, a: &Assertion) -> Result<Value> {
    match a {
        Assertion::Position { axis, min, max, .. } => ensure!(
            *axis < 3 && min.is_finite() && max.is_finite() && min <= max,
            "invalid position assertion"
        ),
        Assertion::Metric { min, max, .. } => ensure!(
            min.is_finite() && max.is_finite() && min <= max,
            "invalid metric assertion"
        ),
        _ => {}
    }
    let observation = json!(runtime.observation());
    let actual = match a {
        Assertion::Equals { pointer, value } => {
            let actual = observation
                .pointer(pointer)
                .context("assertion pointer missing")?;
            ensure!(
                actual == value,
                "equality assertion failed at {pointer}: actual {actual}, expected {value}"
            );
            actual.clone()
        }
        Assertion::Choice { id, visible } => {
            let actual = runtime
                .state
                .gameplay
                .as_ref()
                .context("gameplay missing")?
                .choices()
                .iter()
                .any(|c| &c.id == id);
            ensure!(
                actual == *visible,
                "choice visibility assertion failed for {id}"
            );
            json!(actual)
        }
        Assertion::Finite {} => {
            runtime.assert_valid()?;
            json!(true)
        }
        Assertion::Position {
            entity,
            axis,
            min,
            max,
        } => {
            let i = runtime
                .state
                .compiled
                .indices
                .get(entity)
                .context("assertion entity missing")?;
            let value = runtime.state.transforms[*i]
                .as_ref()
                .context("assertion transform missing")?
                .position[*axis];
            ensure!(
                value >= *min && value <= *max,
                "position assertion failed for {entity}: actual {value}, expected {min}..={max}"
            );
            json!(value)
        }
        Assertion::Metric { name, min, max } => {
            let value = if name.starts_with('/') {
                observation.pointer(name)
            } else {
                observation.get(name)
            }
            .and_then(Value::as_f64)
            .context("assertion metric missing or not numeric")?;
            ensure!(
                value >= *min && value <= *max,
                "metric assertion failed for {name}: actual {value}, expected {min}..={max}"
            );
            json!(value)
        }
    };
    Ok(json!({"assertion":a,"actual":actual,"passed":true}))
}
