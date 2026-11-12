//! `animation.choreography` stages (`animation.stage`) and the typed animation reads of
//! `behavior.program` conditions (`Operand::Animation`). A child module of `animation`, so
//! it works on the players directly; the parent only calls in at a few points.
use super::*;
use axiom_animation::choreography::{Actor, Choreography};
use axiom_core::gameplay::{AnimationField, Scalar};
use axiom_core::Registry;

pub(super) const RESOURCE: &str = "animation.choreography";

fn stage_path(id: &str) -> String {
    format!(
        "/resources/{RESOURCE}/stages/{}",
        axiom_core::local::escape(id)
    )
}

fn invalid(path: String, message: String) -> anyhow::Error {
    axiom_core::Diagnostic::error("invalid_choreography", path, message)
}

/// The stages and the running one.
#[derive(Default)]
pub(super) struct Choreo {
    def: Arc<Choreography>,
    /// The world has the resource (only then observe shows `stage`).
    present: bool,
    active: Option<Active>,
    /// Stages started since the runtime was built (commands and `after`).
    runs: u64,
    /// Last `after` chain that could not start (the stage keeps running; recoverable).
    fault: Option<String>,
}

#[derive(Clone)]
struct Active {
    name: String,
    /// Clip cycles the `after` entity completed since the stage started.
    cycles: u32,
}

/// Checkpoint form of the choreography state (only for worlds with stages).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedStage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(default)]
    cycles: u32,
    #[serde(default)]
    runs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fault: Option<String>,
}

impl Choreo {
    /// Parses the resource and checks every actor against the built players (entity has an
    /// `animation.player`, clips exist, params need a controller that declares them).
    pub(super) fn build(
        state: &RuntimeState,
        players: &BTreeMap<String, Controller>,
    ) -> Result<Self> {
        let Some(value) = state.compiled.source.resources.get(RESOURCE) else {
            return Ok(Self::default());
        };
        let def: Choreography = serde_json::from_value(value.clone())?;
        def.validate()
            .map_err(|e| invalid(format!("/resources/{RESOURCE}"), format!("{e:#}")))?;
        for (id, stage) in &def.stages {
            for a in &stage.actors {
                check_actor(players, a).map_err(|e| invalid(stage_path(id), format!("{e:#}")))?;
            }
            if let Some(after) = &stage.after {
                if !players.contains_key(&after.entity) {
                    return Err(invalid(
                        stage_path(id),
                        format!("after.entity {} has no animation.player", after.entity),
                    ));
                }
                // A one-shot clip completes once: more cycles would never be reached.
                let one_shot = stage
                    .actors
                    .iter()
                    .find(|a| a.entity == after.entity)
                    .is_some_and(|a| a.clip.is_some() && !a.looping);
                if one_shot && after.cycles > 1 {
                    return Err(invalid(
                        stage_path(id),
                        format!(
                            "after.cycles {} on {}'s one-shot clip would never be reached (use 1)",
                            after.cycles, after.entity
                        ),
                    ));
                }
                if let Some(a) = stage.actors.iter().find(|a| a.entity == after.entity) {
                    if a.speed.is_some_and(|s| s <= 0.0) {
                        return Err(invalid(
                            stage_path(id),
                            format!(
                                "after.entity {} plays at speed <= 0, so its clip never completes a cycle",
                                after.entity
                            ),
                        ));
                    }
                }
            }
        }
        Ok(Self {
            def: Arc::new(def),
            present: true,
            ..Self::default()
        })
    }

    pub(super) fn save(&self) -> Option<SavedStage> {
        self.present.then(|| SavedStage {
            active: self.active.as_ref().map(|a| a.name.clone()),
            cycles: self.active.as_ref().map_or(0, |a| a.cycles),
            runs: self.runs,
            fault: self.fault.clone(),
        })
    }

    pub(super) fn load(&mut self, saved: Option<&SavedStage>) -> Result<()> {
        let Some(s) = saved else {
            ensure!(
                !self.present,
                "checkpoint lacks the animation.choreography state (saved before stages existed)"
            );
            return Ok(());
        };
        ensure!(
            self.present,
            "checkpoint holds stages this world does not have"
        );
        self.active = match &s.active {
            Some(name) => {
                ensure!(
                    self.def.stages.contains_key(name),
                    "checkpoint names unknown stage {name}"
                );
                Some(Active {
                    name: name.clone(),
                    cycles: s.cycles,
                })
            }
            None => None,
        };
        self.runs = s.runs;
        self.fault.clone_from(&s.fault);
        Ok(())
    }

    /// Hot reload: the running stage continues (its cycle count kept) while it still exists.
    pub(super) fn transfer(&mut self, old: &Self) {
        self.runs = old.runs;
        self.fault.clone_from(&old.fault);
        self.active = old
            .active
            .clone()
            .filter(|a| self.def.stages.contains_key(&a.name));
    }

    pub(super) fn metrics(&self) -> Option<Value> {
        self.present.then(|| {
            json!({
                "active": self.active.as_ref().map(|a| &a.name),
                "cycles": self.active.as_ref().map_or(0, |a| a.cycles),
                "runs": self.runs,
                "fault": self.fault,
            })
        })
    }
}

fn check_actor(players: &BTreeMap<String, Controller>, a: &Actor) -> Result<()> {
    let p = players
        .get(&a.entity)
        .with_context(|| format!("actor {} has no animation.player", a.entity))?;
    if let Some(clip) = &a.clip {
        ensure!(
            p.animator.is_none(),
            "actor {} is driven by animation controller; give it params instead of a clip",
            a.entity
        );
        ensure!(
            p.clips.contains_key(clip),
            "actor {}: clip {clip} not found",
            a.entity
        );
    }
    if !a.params.is_empty() {
        let c = p.animator.as_ref().with_context(|| {
            format!("actor {} has params but no animation.controller", a.entity)
        })?;
        for name in a.params.keys() {
            ensure!(
                c.compiled.param(name).is_some(),
                "actor {}: controller {} has no parameter {name}",
                a.entity,
                c.id
            );
        }
    }
    Ok(())
}

/// `{"stage": ID}` of `animation.stage`.
pub(super) fn stage_arg(args: &Value) -> Result<&str> {
    args.get("stage")
        .and_then(Value::as_str)
        .context("animation.stage needs {\"stage\":\"stage ID\"}")
}

pub(super) fn register(r: &mut Registry) {
    r.resource_mergers.insert(RESOURCE.into(), |a, b| {
        axiom_core::package::merge_resource(a, b, &["stages"], &[])
    });
    r.resource(
        RESOURCE,
        "animation",
        json!({"stages":"stable ID -> {actors:[1..16 {entity, clip?, fade_seconds?, looping?, speed?, ramp_seconds?, params?:{NAME: number|bool}}], after?:{entity, cycles? (default 1), next: stage ID}}; animation.stage starts every actor on the same fixed step (all checked first); after: next stage once entity completed that many clip cycles (one-shot: reached its end)"}),
        |v| serde_json::from_value::<Choreography>(v.clone())?.validate(),
    );
    r.action(
        "animation.stage",
        "animation",
        json!({"stage":"animation.choreography stage ID","effect":"starts every actor of the stage on this step, or none of them when one is refused (unknown clip, controller conflict, unknown parameter)"}),
        |v| stage_arg(v).and_then(axiom_core::valid_id),
    );
}

/// A resolved `Operand::Animation`: the player it reads and, for `param`, the index.
pub(super) struct ReadSlot {
    entity: String,
    field: AnimationField,
    param: Option<usize>,
}

/// Resolves the program's animation reads against the players. Unknown entities, and
/// `state` / `param` reads of entities without a controller, are errors.
pub(super) fn resolve_reads(
    state: &RuntimeState,
    players: &BTreeMap<String, Controller>,
) -> Result<Vec<ReadSlot>> {
    let Some(g) = &state.gameplay else {
        return Ok(vec![]);
    };
    g.compiled
        .animation_reads
        .iter()
        .map(|r| {
            let fail = |m: String| {
                axiom_core::Diagnostic::error("invalid_gameplay", "/resources/behavior.program", m)
            };
            let p = players.get(&r.entity).ok_or_else(|| {
                fail(format!(
                    "animation read of {}: entity has no animation.player",
                    r.entity
                ))
            })?;
            let param = match (r.field, &r.param) {
                (AnimationField::Param, Some(name)) => {
                    let c = p.animator.as_ref().ok_or_else(|| {
                        fail(format!(
                            "animation read {}.param {name}: entity has no animation.controller",
                            r.entity
                        ))
                    })?;
                    Some(c.compiled.param(name).ok_or_else(|| {
                        fail(format!(
                            "animation read {}.param: controller {} has no parameter {name}",
                            r.entity, c.id
                        ))
                    })?)
                }
                (AnimationField::State, _) if p.animator.is_none() => {
                    return Err(fail(format!(
                        "animation read {}.state: entity has no animation.controller",
                        r.entity
                    )))
                }
                _ => None,
            };
            Ok(ReadSlot {
                entity: r.entity.clone(),
                field: r.field,
                param,
            })
        })
        .collect()
}

impl AnimationSystem {
    /// Writes the animation reads into the gameplay's typed slots. Allocates only when a
    /// text value (clip / state name) changes.
    pub(super) fn publish(&self, state: &mut RuntimeState) {
        if self.reads.is_empty() {
            return;
        }
        let finished = &state.animation_cues.finished;
        let Some(g) = state.gameplay.as_mut() else {
            return;
        };
        for (slot, value) in self.reads.iter().zip(g.animation.iter_mut()) {
            let Some(p) = self.players.get(&slot.entity) else {
                continue;
            };
            let primary = match &p.animator {
                Some(a) => a.compiled.primary(&a.state).map(|(c, d, _)| (c, d)),
                None => p
                    .active
                    .as_deref()
                    .and_then(|c| Some((c, p.clips.get(c)?.source.duration))),
            };
            let number = |v: f32| Scalar::Number(f64::from(v));
            let text = |value: &mut Scalar, s: &str| {
                if !matches!(value, Scalar::Text(old) if old == s) {
                    *value = Scalar::Text(s.to_owned());
                }
            };
            match slot.field {
                AnimationField::Clip => text(value, primary.map_or("", |(c, _)| c)),
                AnimationField::State => text(
                    value,
                    p.animator
                        .as_ref()
                        .map_or("", |a| &a.compiled.states[a.state.current.state as usize]),
                ),
                AnimationField::Time => {
                    *value = number(match &p.animator {
                        Some(a) => a.state.current.frac * primary.map_or(0.0, |(_, d)| d),
                        None => p.time,
                    })
                }
                AnimationField::Normalized => {
                    *value = number(match &p.animator {
                        Some(a) => a.state.current.frac,
                        None => match primary {
                            Some((_, d)) if d > 0.0 => (p.time / d).clamp(0.0, 1.0),
                            _ => 0.0,
                        },
                    })
                }
                AnimationField::Speed => *value = number(p.speed),
                AnimationField::Playing => *value = Scalar::Bool(p.playing),
                AnimationField::Finished => *value = Scalar::Bool(finished.contains(&slot.entity)),
                AnimationField::Param => {
                    if let (Some(a), Some(i)) = (&p.animator, slot.param) {
                        *value = number(a.state.params[i]);
                    }
                }
            }
        }
    }

    /// Every actor of stage `name` applied to copies of the players; nothing changes here.
    pub(super) fn stage_plan(&self, name: &str) -> Result<Vec<(String, Controller, bool)>> {
        let stage = self.choreo.def.stages.get(name).with_context(|| {
            format!(
                "unknown stage {name} (stages: {})",
                self.choreo
                    .def
                    .stages
                    .keys()
                    .take(16)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let mut out = Vec::with_capacity(stage.actors.len());
        for a in &stage.actors {
            let mut p = self
                .players
                .get(&a.entity)
                .with_context(|| format!("actor {} has no animation.player", a.entity))?
                .clone();
            if let Some(clip) = &a.clip {
                p.play(PlayArgs {
                    entity: a.entity.clone(),
                    clip: clip.clone(),
                    fade_seconds: a.fade_seconds,
                    looping: a.looping,
                    speed: a.speed.unwrap_or(1.0),
                })?;
            } else if let Some(speed) = a.speed {
                if a.ramp_seconds > 0.0 {
                    p.ramp = Some((p.speed, speed, 0.0, a.ramp_seconds));
                } else {
                    p.speed = speed;
                    p.ramp = None;
                }
            }
            if !a.params.is_empty() {
                let c = p.animator.as_mut().with_context(|| {
                    format!("actor {} has params but no animation.controller", a.entity)
                })?;
                for (n, v) in &a.params {
                    c.compiled.set_param(&mut c.state, n, *v)?;
                }
            }
            p.evaluate(0.0)
                .with_context(|| format!("stage {name}, actor {}", a.entity))?;
            out.push((a.entity.clone(), p, a.clip.is_some()));
        }
        Ok(out)
    }

    /// Starts stage `name`: every actor on this step, or none when one is refused.
    pub(super) fn start_stage(&mut self, state: &mut RuntimeState, name: &str) -> Result<Value> {
        let plan = self.stage_plan(name)?;
        let actors: Vec<String> = plan.iter().map(|(id, ..)| id.clone()).collect();
        for (id, p, played) in plan {
            if played {
                state.animation_cues.finished.remove(&id);
            }
            self.players.insert(id, p);
        }
        self.choreo.active = Some(Active {
            name: name.to_owned(),
            cycles: 0,
        });
        self.choreo.runs += 1;
        self.publish(state);
        Ok(json!({"stage": name, "actors": actors, "tick": state.tick}))
    }

    /// Counts the clip cycles the running stage's `after` entity completed this step (from
    /// the step's playback interval, before the markers consume it). True once enough.
    pub(super) fn stage_cycles(&mut self) -> bool {
        let Some(active) = &mut self.choreo.active else {
            return false;
        };
        let Some(after) = self
            .choreo
            .def
            .stages
            .get(&active.name)
            .and_then(|s| s.after.as_ref())
        else {
            return false;
        };
        let Some(p) = self.players.get(&after.entity) else {
            return false;
        };
        let mut done = 0u32;
        let mut count = |duration: f32, looping: bool, (prev, end): (f32, f32), scale: f32| {
            super::crossings(&[], duration, looping, prev * scale, end * scale, |m| {
                if m.is_none() {
                    done += 1;
                }
            });
        };
        match &p.animator {
            Some(a) => {
                if let Some((left, prev, end)) = p.exit_crossing {
                    if let Some((_, d, looping)) = a.compiled.primary_of(&a.state, left) {
                        count(d, looping, (prev, end), d);
                    }
                }
                if let (Some((prev, end, _)), Some((_, d, looping))) =
                    (p.crossing, a.compiled.primary(&a.state))
                {
                    count(d, looping, (prev, end), d);
                }
            }
            None => {
                if let (Some((prev, end, _)), Some(clip)) =
                    (p.crossing, p.active.as_ref().and_then(|c| p.clips.get(c)))
                {
                    count(clip.source.duration, p.looping, (prev, end), 1.0);
                }
            }
        }
        active.cycles = active.cycles.saturating_add(done);
        active.cycles >= after.cycles
    }

    /// Starts the running stage's `after.next` (same step). A refused next stage leaves the
    /// current one running and is reported in observe (`animation.stage.fault`).
    pub(super) fn advance_stage(&mut self, state: &mut RuntimeState) {
        let Some(next) = self
            .choreo
            .active
            .as_ref()
            .and_then(|a| self.choreo.def.stages.get(&a.name))
            .and_then(|s| s.after.as_ref())
            .map(|a| a.next.clone())
        else {
            return;
        };
        if let Err(e) = self.start_stage(state, &next) {
            self.choreo.fault = Some(format!("after -> {next}: {e:#}"));
            // Do not retry every step.
            self.choreo.active = None;
        }
    }
}
