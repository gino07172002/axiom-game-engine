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
    /// Mouse-look deltas (raw device units, as the window delivered them), applied in order
    /// with `WorldRuntime::pointer`. One op holds the consecutive deltas between two other
    /// operations of the same tick, at most `MAX_LOOK_DELTAS`.
    Look {
        deltas: Vec<[f32; 2]>,
    },
    /// A screen pointer event: `screen` is the game area's normalized position (0..1, y
    /// down) and `aspect` its width / height; the ray is derived from the camera state at
    /// replay time exactly as live (`WorldRuntime::screen_pointer`).
    Pointer {
        kind: crate::world::PointerKind,
        screen: [f32; 2],
        aspect: f32,
        #[serde(default)]
        additive: bool,
    },
}
/// Deltas one `look` operation may hold.
pub const MAX_LOOK_DELTAS: usize = 256;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioInput {
    pub tick: u32,
    pub actions: BTreeMap<String, f32>,
    /// A virtual controller from this tick on (until the next input with a `pad`), read
    /// through `input.bindings.gamepad` like a real one; `actions` override its results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<crate::gamepad::PadInput>,
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
    /// Compare one live entity field. The record is
    /// `{"position":[x,y,z],"rotation":[x,y,z,w],"scale":[x,y,z],"health":v,"enabled":b,
    /// "components":{...}}` (local transform as simulated); `field` is a JSON
    /// pointer into it (or a bare key such as `health`).
    Field {
        entity: String,
        field: String,
        op: axiom_core::local::Op,
        value: Value,
    },
    /// Compare part of a side-effect-free runtime query (`*.inspect` / `*.summary`).
    Call {
        action: String,
        #[serde(default)]
        args: Value,
        pointer: String,
        op: axiom_core::local::Op,
        value: Value,
    },
    /// The run's `state_hash` equals `value` (recordings carry the hash the editor's Play
    /// session ended with, so a replay checks itself).
    StateHash {
        value: String,
    },
}
/// Order-stable hash of simulated state (tick, RNG, transforms, health, runtime metrics),
/// excluding wall-clock and GPU fields. Equal hashes across processes mean identical runs.
pub fn state_hash(runtime: &WorldRuntime) -> Result<String> {
    let mut metrics = runtime.observation().metrics;
    for volatile in ["fps", "frame_ms_cpu", "gpu"] {
        metrics.remove(volatile);
    }
    // Derived from the transforms (hashed below): recordings made before the facing monitor
    // keep their hash.
    metrics.remove("facing");
    // The navigation grid's build wall time and process-cache hit describe this process, not
    // the simulation (an editor Play and a fresh `axiom scenario` process differ there).
    if let Some(grid) = metrics
        .get_mut("nav")
        .and_then(|nav| nav.get_mut("grid"))
        .and_then(Value::as_object_mut)
    {
        grid.remove("build_ms");
        grid.remove("cached");
    }
    axiom_core::local::fingerprint(&(
        runtime.state.tick,
        runtime.state.rng,
        &runtime.state.transforms,
        &runtime.state.health,
        &metrics,
    ))
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
    run_observed(world, rev, registry, scenario, save_root, None)
}
/// `run_with_saves` that reports tick progress and stops between ticks when `progress`
/// is cancelled (editor background job). The simulation itself is identical.
pub fn run_observed(
    world: World,
    rev: i64,
    registry: RuntimeRegistry,
    scenario: Scenario,
    save_root: Option<&std::path::Path>,
    progress: Option<&crate::jobs::Progress>,
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
            command.tick <= scenario.ticks && previous.is_none_or(|p| p <= command.tick),
            "scenario command ticks must be ordered and in range (0..=ticks)"
        );
        previous = Some(command.tick);
        match &command.operation {
            ScenarioOperation::Look { deltas } => ensure!(
                !deltas.is_empty()
                    && deltas.len() <= MAX_LOOK_DELTAS
                    && deltas.iter().flatten().all(|v| v.is_finite()),
                "scenario look needs 1..={MAX_LOOK_DELTAS} finite deltas"
            ),
            ScenarioOperation::Pointer { screen, aspect, .. } => ensure!(
                aspect.is_finite()
                    && *aspect > 0.0
                    && screen
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                "scenario pointer needs screen in 0..1 and a positive aspect"
            ),
            _ => {}
        }
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
        if let Some(pad) = &input.pad {
            pad.state()
                .with_context(|| format!("scenario input at tick {}", input.tick))?;
        }
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
    let mut pads = crate::gamepad::Pads::default();
    let no_keys = std::collections::BTreeSet::new();
    let mut next = 0;
    let mut commands = scenario.commands.iter().peekable();
    let mut saves = BTreeMap::new();
    if let Some(p) = progress {
        p.set_total(u64::from(scenario.ticks));
    }
    let mut intermediate = vec![];
    // Commands at tick `ticks` run after the last step, before the final assertions.
    for tick in 0..=scenario.ticks {
        if let Some(p) = progress.filter(|_| tick < scenario.ticks) {
            if p.cancelled() {
                return Err(axiom_core::Diagnostic::error(
                    "scenario_cancelled",
                    "/ticks",
                    format!("scenario stopped at tick {tick} of {}", scenario.ticks),
                ));
            }
            p.set_done(u64::from(tick));
        }
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
                    intermediate.push(check(&mut runtime, assertion)?);
                }
                ScenarioOperation::Look { deltas } => {
                    for [dx, dy] in deltas {
                        runtime.pointer(*dx, *dy);
                    }
                }
                ScenarioOperation::Pointer {
                    kind,
                    screen,
                    aspect,
                    additive,
                } => runtime.screen_pointer(*kind, *screen, *aspect, *additive)?,
            }
        }
        if tick == scenario.ticks {
            break;
        }
        if let Some(event) = scenario.inputs.get(next) {
            if event.tick == tick {
                for (k, v) in &event.actions {
                    input.0.insert(k.clone(), *v);
                }
                if let Some(p) = &event.pad {
                    pads.0[0] = Some(p.state()?);
                }
                next += 1;
            }
        }
        if pads.connected() > 0 {
            let mut merged = ActionMap::from_input(&runtime.bindings, &no_keys, &pads);
            for (k, v) in &input.0 {
                merged.0.insert(k.clone(), *v);
            }
            runtime.step(&merged);
        } else {
            runtime.step(&input);
        }
        runtime.assert_valid()?;
    }
    if let Some(p) = progress {
        p.set_done(u64::from(scenario.ticks));
    }
    let observation = json!(runtime.observation());
    let hash = state_hash(&runtime)?;
    let mut results = vec![];
    for a in &scenario.assertions {
        results.push(check(&mut runtime, a)?);
    }
    Ok(
        json!({"ok":true,"ticks":scenario.ticks,"state_hash":hash,"assertions":results,"intermediate_assertions":intermediate,"checkpoints":saves.keys().collect::<Vec<_>>(),"observation":observation}),
    )
}
fn compare(label: &str, actual: &Value, op: axiom_core::local::Op, value: &Value) -> Result<()> {
    let probe = axiom_core::local::Predicate {
        field: "/v".into(),
        op,
        value: value.clone(),
    };
    ensure!(
        probe.test(&json!({ "v": actual })),
        "assertion failed for {label}: actual {actual}, expected {} {value}",
        op.symbol()
    );
    Ok(())
}
/// A live `f32` value (read back as the nearest f64, 0.1 -> 0.100000001…) as the shortest
/// decimal that round-trips through `f32` (0.1; 16777216), the number an author writes for
/// it. Every operator compares that with the expected value as written: `eq 0.1` holds for a
/// stored 0.1 while `gt 0.1` does not, and an ordered threshold keeps its precision
/// (16777216 `lt` 16777217 holds although 16777217 rounds to 16777216 in `f32`).
fn as_written_f32(value: &Value) -> Value {
    match value {
        Value::Number(n) => n
            .as_f64()
            .and_then(|v| (v as f32).to_string().parse::<f64>().ok())
            .map_or_else(|| value.clone(), |v| json!(v)),
        Value::Array(items) => Value::Array(items.iter().map(as_written_f32).collect()),
        other => other.clone(),
    }
}
fn check(runtime: &mut WorldRuntime, a: &Assertion) -> Result<Value> {
    if let Assertion::Field {
        entity,
        field,
        op,
        value,
    } = a
    {
        let i = *runtime
            .state
            .compiled
            .indices
            .get(entity)
            .with_context(|| format!("assertion entity {entity} missing"))?;
        // Custom components read their live typed columns (as checkpoints save them), not
        // the authored values the runtime was compiled from.
        let mut components = runtime.state.compiled.entities[i].components.clone();
        // Pointers of fields stored as `f32` (float / vec3 columns).
        let mut single = vec![];
        for (name, table) in &runtime.state.columns {
            let Some(row) = table.entities.iter().position(|slot| *slot == i) else {
                continue;
            };
            let Some(Value::Object(component)) = components.get_mut(name) else {
                continue;
            };
            for (field, column) in &table.fields {
                if matches!(
                    column,
                    axiom_core::Column::Float(_) | axiom_core::Column::Vec3(_)
                ) {
                    single.push(format!(
                        "/components/{}/{}",
                        axiom_core::local::escape(name),
                        axiom_core::local::escape(field)
                    ));
                }
                match crate::checkpoint::column_value(column, row, &runtime.state) {
                    Some(value) => {
                        component.insert(field.clone(), value);
                    }
                    None => {
                        component.remove(field);
                    }
                }
            }
        }
        let record = json!({
            "position":runtime.state.transforms[i].as_ref().map(|t|t.position),
            "rotation":runtime.state.transforms[i].as_ref().map(|t|t.rotation),
            "scale":runtime.state.transforms[i].as_ref().map(|t|t.scale),
            "health":runtime.state.health[i].as_ref().map(|h|h.value),
            "enabled":runtime.state.compiled.enabled[i],
            "components":components,
        });
        let pointer = axiom_core::local::pointer(field);
        let actual = record
            .pointer(&pointer)
            .cloned()
            .with_context(|| format!("field {pointer} missing on {entity}"))?;
        // A live `f32` column is compared as the decimal written for it (`as_written_f32`);
        // the expected value keeps its own precision.
        let compared = if single
            .iter()
            .any(|p| pointer == *p || pointer.starts_with(&format!("{p}/")))
        {
            as_written_f32(&actual)
        } else {
            actual.clone()
        };
        compare(&format!("{entity}{pointer}"), &compared, *op, value)?;
        return Ok(json!({"assertion":a,"actual":actual,"passed":true}));
    }
    if let Assertion::Call {
        action,
        args,
        pointer,
        op,
        value,
    } = a
    {
        let (capability, name) = action
            .split_once('.')
            .context("call assertion action must be capability.action")?;
        ensure!(
            name == "inspect" || name == "summary",
            "call assertions only run side-effect-free inspect/summary actions"
        );
        let args = if args.is_null() {
            json!({})
        } else {
            args.clone()
        };
        let out = runtime.command(&axiom_core::gameplay::RuntimeCommand::Call {
            capability: capability.into(),
            action: name.into(),
            args,
        })?;
        let actual = out["result"]
            .pointer(pointer)
            .cloned()
            .with_context(|| format!("{action} result has no {pointer}"))?;
        compare(&format!("{action}{pointer}"), &actual, *op, value)?;
        return Ok(json!({"assertion":a,"actual":actual,"passed":true}));
    }
    if let Assertion::StateHash { value } = a {
        let actual = state_hash(runtime)?;
        ensure!(
            &actual == value,
            "state_hash assertion failed: actual {actual}, expected {value}"
        );
        return Ok(json!({"assertion":a,"actual":actual,"passed":true}));
    }
    let runtime = &*runtime;
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
        Assertion::Field { .. } | Assertion::Call { .. } | Assertion::StateHash { .. } => {
            unreachable!("handled above")
        }
    };
    Ok(json!({"assertion":a,"actual":actual,"passed":true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_json(text: &str) -> Result<Value> {
        let registry = crate::installed_registry();
        let world = axiom_core::ContentPackage::load(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/fps"),
            &registry.authoring,
        )?
        .world;
        run(world, 1, registry, serde_json::from_str(text)?)
    }

    #[test]
    fn field_assertions_read_live_custom_columns_not_authored_values() {
        let mut w = axiom_core::World::empty("flags");
        w.component_schemas.insert(
            "game.flags".into(),
            serde_json::from_value(json!({"fields":{"flag":{"ty":"bool"}}})).unwrap(),
        );
        let mut a = axiom_core::Entity::new("a");
        a.set("game.flags", json!({"flag":false})).unwrap();
        w.entities = vec![a];
        let mut runtime = WorldRuntime::new(w, 1, crate::world::RuntimeRegistry::core()).unwrap();
        // The simulation (or a loaded checkpoint) set the flag; the authored value is false.
        if let axiom_core::Column::Bool(v) = runtime
            .state
            .columns
            .get_mut("game.flags")
            .unwrap()
            .fields
            .get_mut("flag")
            .unwrap()
        {
            v[0] = Some(true);
        }
        let assertion = |value: bool| -> Assertion {
            serde_json::from_value(json!({"kind":"field","entity":"a",
                "field":"/components/game.flags/flag","op":"eq","value":value}))
            .unwrap()
        };
        let passed = check(&mut runtime, &assertion(true)).unwrap();
        assert_eq!(passed["actual"], true);
        assert!(check(&mut runtime, &assertion(false)).is_err());
    }

    /// A custom `float` / `vec3` field the simulation never touched still equals its
    /// authored decimal value, although the live column holds it as `f32`.
    #[test]
    fn float_field_assertions_compare_at_the_columns_precision() {
        let mut w = axiom_core::World::empty("floats");
        w.component_schemas.insert(
            "game.stats".into(),
            serde_json::from_value(json!({"fields":{"speed":{"ty":"float"},"aim":{"ty":"vec3"}}}))
                .unwrap(),
        );
        let mut a = axiom_core::Entity::new("a");
        a.set("game.stats", json!({"speed":0.1,"aim":[0.1,0.2,0.3]}))
            .unwrap();
        // 2^24 (16777217 is not an f32) and two adjacent f32 values (1.0, 1.0000001).
        let mut b = axiom_core::Entity::new("b");
        b.set(
            "game.stats",
            json!({"speed":16_777_216.0,"aim":[1.0,1.000_000_1,0.0]}),
        )
        .unwrap();
        w.entities = vec![a, b];
        let mut runtime = WorldRuntime::new(w, 1, crate::world::RuntimeRegistry::core()).unwrap();
        let on = |entity: &str, field: &str, op: &str, value: Value| -> Assertion {
            serde_json::from_value(json!({"kind":"field","entity":entity,
                "field":field,"op":op,"value":value}))
            .unwrap()
        };
        // A-F14 (round 3): ordered comparisons keep the threshold's precision.
        let (speed, aim) = ("/components/game.stats/speed", "/components/game.stats/aim");
        for (field, op, value, holds) in [
            (speed.to_owned(), "lt", json!(16_777_217), true),
            (speed.to_owned(), "le", json!(16_777_216), true),
            (speed.to_owned(), "gt", json!(16_777_215), true),
            (speed.to_owned(), "eq", json!(16_777_217), false),
            (speed.to_owned(), "ge", json!(16_777_217), false),
            (format!("{aim}/0"), "lt", json!(1.000_000_1), true),
            (format!("{aim}/0"), "eq", json!(1.000_000_1), false),
            (format!("{aim}/1"), "gt", json!(1.0), true),
            (format!("{aim}/1"), "eq", json!(1.000_000_1), true),
            (format!("{aim}/1"), "le", json!(1.0), false),
        ] {
            let result = check(&mut runtime, &on("b", &field, op, value.clone()));
            assert_eq!(result.is_ok(), holds, "{field} {op} {value}: {result:?}");
        }
        let assertion = |field: &str, op: &str, value: Value| on("a", field, op, value);
        for (field, op, value) in [
            ("/components/game.stats/speed", "eq", json!(0.1)),
            ("/components/game.stats/speed", "le", json!(0.1)),
            ("/components/game.stats/aim/1", "eq", json!(0.2)),
            ("/components/game.stats/aim", "eq", json!([0.1, 0.2, 0.3])),
        ] {
            check(&mut runtime, &assertion(field, op, value.clone()))
                .unwrap_or_else(|e| panic!("{field} {op} {value}: {e:#}"));
        }
        // Still a real comparison.
        assert!(check(
            &mut runtime,
            &assertion("/components/game.stats/speed", "eq", json!(0.11))
        )
        .is_err());
        assert!(check(
            &mut runtime,
            &assertion("/components/game.stats/speed", "gt", json!(0.1))
        )
        .is_err());
    }

    #[test]
    fn look_and_pointer_operations_are_validated_and_may_follow_the_last_step() {
        let ok = run_json(
            r#"{"ticks":2,"commands":[
            {"tick":0,"operation":{"op":"look","deltas":[[4.0,-1.0],[2.5,0.0]]}},
            {"tick":1,"operation":{"op":"pointer","kind":"primary_down","screen":[0.5,0.5],"aspect":1.5}},
            {"tick":2,"operation":{"op":"look","deltas":[[-9.0,3.0]]}}],
            "assertions":[{"kind":"finite"}]}"#,
        )
        .unwrap();
        assert_eq!(ok["ticks"], 2);
        // The deltas before the steps turned the player (FPS publishes its view in `step`).
        let without = run_json(
            r#"{"ticks":2,"commands":[
            {"tick":1,"operation":{"op":"pointer","kind":"primary_down","screen":[0.5,0.5],"aspect":1.5}},
            {"tick":2,"operation":{"op":"look","deltas":[[-9.0,3.0]]}}],
            "assertions":[]}"#,
        )
        .unwrap();
        assert_ne!(ok["state_hash"], without["state_hash"]);
        for bad in [
            r#"{"op":"look","deltas":[]}"#,
            r#"{"op":"look","deltas":[[1e40,0.0]]}"#,
            r#"{"op":"pointer","kind":"move","screen":[1.5,0.5],"aspect":1.0}"#,
            r#"{"op":"pointer","kind":"move","screen":[0.5,0.5],"aspect":0.0}"#,
            r#"{"op":"pointer","kind":"hover","screen":[0.5,0.5],"aspect":1.0}"#,
        ] {
            let text = format!(
                r#"{{"ticks":1,"commands":[{{"tick":0,"operation":{bad}}}],"assertions":[]}}"#
            );
            assert!(run_json(&text).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..=MAX_LOOK_DELTAS).map(|_| "[1.0,0.0]".into()).collect();
        let text = format!(
            r#"{{"ticks":1,"commands":[{{"tick":0,"operation":{{"op":"look","deltas":[{}]}}}}],"assertions":[]}}"#,
            many.join(",")
        );
        assert!(run_json(&text).is_err(), "too many deltas in one op");
        // Past `ticks` is still refused.
        assert!(run_json(
            r#"{"ticks":1,"commands":[{"tick":2,"operation":{"op":"look","deltas":[[1.0,0.0]]}}],"assertions":[]}"#
        )
        .is_err());
    }
}
