//! Data-authored duet timing. The stage, actors and clips are content.
use crate::{
    ui::{self, Action, UiState},
    world::{ActionMap, RuntimeRegistry, RuntimeState, RuntimeSystem, FIXED_DT},
};
use anyhow::{ensure, Context, Result};
use axiom_animation::authoring::PlayArgs;
use axiom_core::{
    gameplay::RuntimeCommand,
    local::{at, escape},
    valid_id, Diagnostic, InputBindings, SystemDescriptor,
};
use egui::{Color32, RichText};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
};

const RESOURCE: &str = "rhythm.sequence";
const PATH: &str = "/resources/rhythm.sequence";
const INVITE: &str = "rhythm_invite";
const STOP: &str = "rhythm_stop";

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Move {
    label: String,
    input: String,
    clips: [String; 2],
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Sequence {
    performers: [String; 2],
    idle: [String; 2],
    finale: [String; 2],
    accepts_invite: bool,
    beat_ticks: u32,
    count_in: u32,
    window_ticks: u32,
    pass_hits: usize,
    moves: BTreeMap<String, Move>,
    order: Vec<String>,
}
impl Sequence {
    fn validate(&self) -> Result<()> {
        at(
            &format!("{PATH}/beat_ticks"),
            (|| {
                ensure!(
                    (30..=240).contains(&self.beat_ticks),
                    "beat interval must be 30..240 ticks"
                );
                Ok(())
            })(),
        )?;
        at(
            &format!("{PATH}/window_ticks"),
            (|| {
                ensure!(
                    (1..self.beat_ticks / 2).contains(&self.window_ticks),
                    "hit window must be positive and less than half a beat"
                );
                Ok(())
            })(),
        )?;
        ensure!(
            (1..=8).contains(&self.count_in),
            "count-in must be 1..8 beats"
        );
        ensure!(
            (1..=12).contains(&self.moves.len()) && (1..=64).contains(&self.order.len()),
            "duet sequence budget exceeded"
        );
        ensure!(
            (1..=self.order.len()).contains(&self.pass_hits),
            "pass_hits must be within the number of beats"
        );
        ensure!(
            self.performers[0] != self.performers[1],
            "performers must be distinct"
        );
        for id in self.performers.iter().chain(&self.idle).chain(&self.finale) {
            valid_id(id)?;
        }
        for (id, movement) in &self.moves {
            at(
                &format!("{PATH}/moves/{}", escape(id)),
                (|| {
                    valid_id(id)?;
                    valid_id(&movement.input)?;
                    ensure!(
                        !matches!(movement.input.as_str(), INVITE | STOP),
                        "movement input conflicts with duet controls"
                    );
                    ensure!(
                        !movement.label.trim().is_empty()
                            && movement.label.len() <= 64
                            && !movement.label.chars().any(char::is_control),
                        "invalid move label"
                    );
                    for clip in &movement.clips {
                        valid_id(clip)?;
                    }
                    Ok(())
                })(),
            )?;
        }
        for id in &self.order {
            ensure!(self.moves.contains_key(id), "unknown move in order: {id}");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    #[default]
    Waiting,
    Ready,
    Declined,
    Dancing,
    Finishing,
    Finale,
    Retry,
    Stopped,
}

#[derive(Clone, Default)]
struct Progress {
    phase: Phase,
    started: u64,
    index: usize,
    hits: usize,
    misses: usize,
    wrong: usize,
    streak: usize,
    best: usize,
    ending_at: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Press {
    input: String,
}

struct Duet {
    sequence: Sequence,
    progress: Progress,
    inputs: BTreeMap<String, String>,
    invite_key: String,
    stop_key: String,
    held: BTreeSet<String>,
    tick: u64,
}

pub(crate) fn register(registry: &mut RuntimeRegistry) {
    let r = &mut registry.authoring;
    r.capabilities.insert("rhythm".into(), 1);
    r.resource(
        RESOURCE,
        "rhythm",
        json!({
            "performers":"two stable entity IDs with animation.player",
            "idle":"two clip IDs","finale":"two clip IDs","accepts_invite":"bool",
            "beat_ticks":"30..240 fixed 120 Hz ticks per beat","count_in":"1..8 beats",
            "window_ticks":"inclusive ± ticks, less than half a beat","pass_hits":"1..order length",
            "moves":"stable move ID -> {label,input,clips:[performer 1 clip, performer 2 clip]}",
            "order":"1..64 move IDs in beat order"
        }),
        |value| {
            at(
                PATH,
                serde_json::from_value::<Sequence>(value.clone()).map_err(Into::into),
            )?
            .validate()
        },
    );
    r.systems.insert(
        "rhythm.play".into(),
        SystemDescriptor {
            id: "rhythm.play".into(),
            capability: "rhythm".into(),
            phase: 35,
            after: vec!["animation.evaluate".into()],
            reads: vec![RESOURCE.into(), "input.bindings".into()],
            writes: vec!["animation.cues".into()],
        },
    );
    r.world_validators.push(|world| {
        if !world.capabilities.contains_key("rhythm") {
            return Ok(());
        }
        let sequence: Sequence = world.resource(RESOURCE)?;
        let bindings: InputBindings = world.resource("input.bindings")?;
        ensure!(
            world.systems.iter().any(|s| s == "rhythm.play"),
            "rhythm capability requires rhythm.play"
        );
        for id in &sequence.performers {
            ensure!(world.entity(id).is_some(), "unknown performer {id}");
        }
        let inputs = sequence
            .moves
            .values()
            .map(|m| m.input.as_str())
            .chain([INVITE, STOP])
            .collect::<BTreeSet<_>>();
        let keys = inputs
            .into_iter()
            .map(|id| {
                bindings
                    .buttons
                    .get(id)
                    .with_context(|| format!("rhythm input requires a button binding: {id}"))
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            keys.iter().all(|k| !k.is_empty())
                && keys.iter().collect::<BTreeSet<_>>().len() == keys.len(),
            "rhythm button bindings must be nonempty and distinct"
        );
        Ok(())
    });
    for name in ["invite", "begin", "stop"] {
        r.action(&format!("rhythm.{name}"), "rhythm", json!({}), |v| {
            ensure!(
                v.as_object().is_some_and(|obj| obj.is_empty()),
                "expected empty arguments"
            );
            Ok(())
        });
    }
    r.action(
        "rhythm.press",
        "rhythm",
        json!({"input":"bound action ID from the move"}),
        |v| {
            let args: Press = serde_json::from_value(v.clone())?;
            valid_id(&args.input)
        },
    );
    r.topics.insert("rhythm".into(), json!({
        "authoring":{"target":{"kind":"resource","id":RESOURCE},"fields":["beat_ticks","window_ticks","pass_hits","moves","order"]},
        "timing":"120 Hz, inclusive hit window, one score per beat",
        "actions":["invite","begin","press","stop"], "observe":"/rhythm",
        "activation":"clip IDs and animation performers are checked against prepared assets",
        "reload":"sequence changes restart the duet; unrelated edits preserve progress"
    }));
    registry.factories.insert("rhythm.play".into(), |state| {
        Ok(Box::new(Duet::new(state)?))
    });
}

impl Duet {
    fn new(state: &RuntimeState) -> Result<Self> {
        let sequence: Sequence = state.compiled.source.resource(RESOURCE)?;
        let bindings: InputBindings = state.compiled.source.resource("input.bindings")?;
        let check_pair = |pair: &[String; 2], path: &str| -> Result<()> {
            for (entity, clip) in sequence.performers.iter().zip(pair) {
                ensure!(
                    state
                        .compiled
                        .indices
                        .get(entity)
                        .is_some_and(|&slot| state.compiled.enabled[slot]),
                    "performer must belong to an active scene: {entity}"
                );
                if !state
                    .animation_cues
                    .catalog
                    .get(entity)
                    .is_some_and(|clips| clips.contains_key(clip))
                {
                    return Err(Diagnostic::error(
                        "rhythm_clip",
                        path,
                        format!("clip {clip} is missing on {entity}"),
                    ));
                }
            }
            Ok(())
        };
        check_pair(&sequence.idle, &format!("{PATH}/idle"))?;
        check_pair(&sequence.finale, &format!("{PATH}/finale"))?;
        for (id, movement) in &sequence.moves {
            check_pair(
                &movement.clips,
                &format!("{PATH}/moves/{}/clips", escape(id)),
            )?;
        }
        let inputs = sequence
            .moves
            .values()
            .map(|m| (m.input.clone(), bindings.buttons[&m.input].clone()))
            .collect();
        Ok(Self {
            sequence,
            inputs,
            invite_key: bindings.buttons[INVITE].clone(),
            stop_key: bindings.buttons[STOP].clone(),
            progress: Progress::default(),
            held: BTreeSet::new(),
            tick: 0,
        })
    }

    fn target(&self) -> u64 {
        self.progress.started.saturating_add(
            (u64::from(self.sequence.count_in) + self.progress.index as u64)
                * u64::from(self.sequence.beat_ticks),
        )
    }

    fn play(&self, state: &mut RuntimeState, pair: &[String; 2], looping: bool) {
        for (entity, clip) in self.sequence.performers.iter().zip(pair) {
            let pending = &mut state.animation_cues.pending;
            // Several commands may arrive before a simulation tick. Keep the latest cue per actor.
            pending.retain(|c| c.entity != *entity);
            pending.push(PlayArgs {
                entity: entity.clone(),
                clip: clip.clone(),
                fade_seconds: 0.10,
                looping,
                speed: if looping {
                    0.75
                } else {
                    (state.animation_cues.catalog[entity][clip]
                        / (self.sequence.beat_ticks as f32 * FIXED_DT))
                        .clamp(0.1, 8.0)
                },
            });
        }
    }

    fn invite(&mut self) -> Result<()> {
        ensure!(
            !matches!(
                self.progress.phase,
                Phase::Dancing | Phase::Finishing | Phase::Ready
            ),
            "duet is already active or accepted"
        );
        self.progress = Progress {
            phase: if self.sequence.accepts_invite {
                Phase::Ready
            } else {
                Phase::Declined
            },
            ..Default::default()
        };
        Ok(())
    }

    fn begin(&mut self, tick: u64) -> Result<()> {
        ensure!(
            self.progress.phase == Phase::Ready,
            "both performers must be ready"
        );
        self.progress.phase = Phase::Dancing;
        self.progress.started = tick;
        Ok(())
    }

    fn settle(&mut self, state: &mut RuntimeState) {
        if self.progress.index < self.sequence.order.len() {
            return;
        }
        self.progress.phase = Phase::Finishing;
        self.progress.ending_at = state
            .tick
            .saturating_add(u64::from(self.sequence.beat_ticks));
    }

    fn end(&mut self, state: &mut RuntimeState) {
        if self.progress.hits >= self.sequence.pass_hits {
            self.progress.phase = Phase::Finale;
            self.play(state, &self.sequence.finale, false);
        } else {
            self.progress.phase = Phase::Retry;
            self.play(state, &self.sequence.idle, true);
        }
    }

    fn expire(&mut self, state: &mut RuntimeState) {
        while self.progress.phase == Phase::Dancing
            && state.tick
                > self
                    .target()
                    .saturating_add(u64::from(self.sequence.window_ticks))
        {
            self.progress.misses += 1;
            self.progress.streak = 0;
            self.progress.index += 1;
            self.settle(state);
        }
        if self.progress.phase == Phase::Finishing && state.tick >= self.progress.ending_at {
            self.end(state);
        }
    }

    fn press(&mut self, state: &mut RuntimeState, input: &str) -> Result<bool> {
        ensure!(
            self.inputs.contains_key(input),
            "unknown movement input {input}"
        );
        self.expire(state);
        if self.progress.phase != Phase::Dancing {
            return Ok(false);
        }
        let movement = &self.sequence.moves[&self.sequence.order[self.progress.index]];
        if self.target().abs_diff(state.tick) > u64::from(self.sequence.window_ticks)
            || movement.input != input
        {
            self.progress.wrong += 1;
            return Ok(false);
        }
        self.play(state, &movement.clips, false);
        self.progress.index += 1;
        self.progress.hits += 1;
        self.progress.streak += 1;
        self.progress.best = self.progress.best.max(self.progress.streak);
        self.settle(state);
        Ok(true)
    }

    fn stop(&mut self, state: &mut RuntimeState) {
        self.progress.phase = Phase::Stopped;
        self.play(state, &self.sequence.idle, true);
    }

    fn observation(&self) -> Value {
        let target = (self.progress.phase == Phase::Dancing).then(|| self.target());
        let move_id = target.map(|_| &self.sequence.order[self.progress.index]);
        json!({
            "phase":self.progress.phase,"index":self.progress.index,"beats":self.sequence.order.len(),
            "hits":self.progress.hits,"misses":self.progress.misses,"wrong":self.progress.wrong,
            "best_streak":self.progress.best,"target_tick":target,"move":move_id,
            "input":move_id.map(|id| &self.sequence.moves[id].input),
            "window_ticks":self.sequence.window_ticks
        })
    }
}

impl RuntimeSystem for Duet {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "rhythm" {
            return None;
        }
        Some((|| {
            match action {
                "invite" => self.invite()?,
                "begin" => self.begin(state.tick)?,
                "stop" => self.stop(state),
                "press" => {
                    let args: Press = serde_json::from_value(args.clone())?;
                    return Ok(
                        json!({"hit":self.press(state,&args.input)?,"duet":self.observation()}),
                    );
                }
                _ => anyhow::bail!("unknown rhythm action {action}"),
            }
            Ok(self.observation())
        })())
    }

    fn step(&mut self, state: &mut RuntimeState, input: &ActionMap) {
        self.tick = state.tick;
        self.expire(state);
        let pressed = self
            .inputs
            .keys()
            .map(String::as_str)
            .chain([INVITE, STOP])
            .filter(|id| input.pressed(id))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let edges = pressed.difference(&self.held).cloned().collect::<Vec<_>>();
        self.held = pressed;
        if edges.iter().any(|id| id == STOP) {
            self.stop(state);
            return;
        }
        for id in edges {
            if id == INVITE {
                if self.progress.phase == Phase::Ready {
                    let _ = self.begin(state.tick);
                } else if !matches!(self.progress.phase, Phase::Dancing | Phase::Finishing) {
                    let _ = self.invite();
                }
            } else {
                let _ = self.press(state, &id);
            }
        }
    }

    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        if let Some(old) = old.as_any().downcast_ref::<Self>().filter(|old| {
            old.sequence == self.sequence && previous.compiled.source.id == next.compiled.source.id
        }) {
            self.progress = old.progress.clone();
            self.held = old.held.clone();
            next.animation_cues.pending.extend(
                previous
                    .animation_cues
                    .pending
                    .iter()
                    .filter(|cue| self.sequence.performers.contains(&cue.entity))
                    .cloned(),
            );
        } else {
            self.play(next, &self.sequence.idle, true);
        }
        self.tick = next.tick;
        Ok(())
    }

    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("rhythm".into(), self.observation())])
    }

    fn draw_ui(
        &self,
        ctx: &egui::Context,
        state: &RuntimeState,
        ui_state: &mut UiState,
    ) -> Option<Vec<Action>> {
        let mut actions = ui::draw_generic(ctx, state, ui_state);
        self.hud(ctx, ui_state.playing && !ui_state.console, &mut actions);
        Some(actions)
    }
}

fn control(action: &str, args: Value) -> Action {
    Action::Gameplay(RuntimeCommand::Call {
        capability: "rhythm".into(),
        action: action.into(),
        args,
    })
}

impl Duet {
    fn hud(&self, ctx: &egui::Context, interactive: bool, actions: &mut Vec<Action>) {
        let gold = Color32::from_rgb(252, 195, 133);
        let pink = Color32::from_rgb(242, 128, 172);
        let pale = Color32::from_rgb(238, 215, 224);
        let scene_width = ctx.screen_rect().width();
        egui::Area::new("duet_title".into())
            .anchor(egui::Align2::LEFT_TOP, [26.0, 52.0])
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("AFTER HOURS  /  SIDE A")
                        .monospace()
                        .size(11.0)
                        .color(gold),
                );
                ui.label(
                    RichText::new("MAKE LOVE\nNO WAR")
                        .size(30.0)
                        .strong()
                        .color(pale),
                );
            });
        egui::Area::new("duet_hud".into())
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -18.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(24, 11, 25, 238))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(105, 56, 89)))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(18, 11))
                    .show(ui, |ui| {
                        ui.set_min_width((scene_width - 60.0).clamp(200.0, 500.0));
                        ui.spacing_mut().item_spacing = egui::vec2(9.0, 6.0);
                        let (title, detail, action) = match self.progress.phase {
                            Phase::Waiting => (
                                "The Velvet Hour",
                                "Ask for a dance. Give your partner a moment to answer.",
                                Some(("Invite", "invite")),
                            ),
                            Phase::Ready => (
                                "“I’d love to.”",
                                "Catch the gold window as you count in together.",
                                Some(("Begin together", "begin")),
                            ),
                            Phase::Declined => (
                                "“Maybe another night.”",
                                "Give them space. The floor is yours to enjoy.",
                                None,
                            ),
                            Phase::Stopped => (
                                "Take a breath",
                                "You can invite your partner again whenever you’re ready.",
                                Some(("Invite", "invite")),
                            ),
                            Phase::Finale => (
                                "One last turn",
                                "You found each other in the rhythm.",
                                Some(("Dance again", "invite")),
                            ),
                            Phase::Retry => (
                                "Find your rhythm",
                                "Breathe, watch the gold window and try the phrase again.",
                                Some(("Invite again", "invite")),
                            ),
                            Phase::Finishing => {
                                ("Hold that feeling", "Let the last move breathe.", None)
                            }
                            Phase::Dancing => (
                                "Stay with me",
                                "Tap the indicated move inside the gold window.",
                                None,
                            ),
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(title).strong().size(18.0).color(pale));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{:02}/{:02}  SYNC",
                                            self.progress.hits,
                                            self.sequence.order.len()
                                        ))
                                        .monospace()
                                        .color(gold),
                                    );
                                },
                            );
                        });
                        ui.label(RichText::new(detail).size(12.0).color(pale));
                        if let Some((label, cmd)) = action {
                            if ui
                                .add_enabled(
                                    interactive,
                                    egui::Button::new(
                                        RichText::new(format!(
                                            "{label}  [{}]",
                                            short_key(&self.invite_key)
                                        ))
                                        .color(gold),
                                    ),
                                )
                                .clicked()
                            {
                                actions.push(control(cmd, json!({})));
                            }
                        }
                        if self.progress.phase == Phase::Dancing {
                            let movement =
                                &self.sequence.moves[&self.sequence.order[self.progress.index]];
                            let beat = self.sequence.beat_ticks as f32;
                            let remaining =
                                (i128::from(self.target()) - i128::from(self.tick)) as f32;
                            let center = (0.70 - remaining / beat * 0.70).clamp(0.0, 1.0);
                            let window = self.sequence.window_ticks as f32 / beat * 0.70;
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 12.0),
                                egui::Sense::hover(),
                            );
                            let x = |f| rect.left() + rect.width() * f;
                            ui.painter()
                                .rect_filled(rect, 4, Color32::from_rgb(73, 35, 61));
                            ui.painter().rect_filled(
                                egui::Rect::from_min_max(
                                    egui::pos2(x(0.70 - window), rect.top()),
                                    egui::pos2(x(0.70 + window), rect.bottom()),
                                ),
                                3,
                                gold,
                            );
                            ui.painter().circle_filled(
                                egui::pos2(x(center), rect.center().y),
                                5.0,
                                Color32::WHITE,
                            );
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{}  [{}]",
                                        movement.label,
                                        short_key(&self.inputs[&movement.input])
                                    ))
                                    .strong()
                                    .color(pink),
                                );
                                for (id, key) in &self.inputs {
                                    if ui
                                        .add_enabled(interactive, egui::Button::new(short_key(key)))
                                        .clicked()
                                    {
                                        actions.push(control("press", json!({"input":id})));
                                    }
                                }
                                if ui
                                    .add_enabled(
                                        interactive,
                                        egui::Button::new(format!(
                                            "Step away  [{}]",
                                            short_key(&self.stop_key)
                                        )),
                                    )
                                    .clicked()
                                {
                                    actions.push(control("stop", json!({})));
                                }
                            });
                        }
                    });
            });
    }
}

fn short_key(key: &str) -> &str {
    key.strip_prefix("Key").unwrap_or(key)
}
