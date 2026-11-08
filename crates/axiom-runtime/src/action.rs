//! Native Action adapter. Input, HUD and replay use registered capability commands.
use crate::{
    animation::AnimationCues,
    presentation::{Instance, RenderFrame},
    ui::{self, Action, UiState},
    world::*,
};
use anyhow::{ensure, Context, Result};
use axiom_action::{
    simulation::{forward, walkable, Simulation},
    *,
};
use axiom_animation::authoring::PlayArgs;
use axiom_core::{gameplay::RuntimeCommand, local::escape, *};
use glam::{Mat4, Quat, Vec2, Vec3};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
};

pub struct ActionSystem {
    pub sim: Simulation,
    yaw: f32,
    held: BTreeSet<String>,
    clips: BTreeMap<String, String>,
    definitions: Value,
    buttons: BTreeMap<String, String>,
    victory_sent: bool,
}

pub fn register(registry: &mut RuntimeRegistry) {
    axiom_action::register(&mut registry.authoring);
    let descriptor = registry
        .authoring
        .systems
        .get_mut("action.simulate")
        .unwrap();
    descriptor.after = vec!["animation.evaluate".into(), "audio.events".into()];
    registry
        .factories
        .insert("action.simulate".into(), |state| {
            let sim = Simulation::new(&state.compiled)?;
            let bindings: InputBindings = state.compiled.source.resource("input.bindings")?;
            ensure!(
                ["attack", "dodge", "lock"]
                    .iter()
                    .all(|k| bindings.buttons.contains_key(*k))
                    && ["move_x", "move_z", "orbit"]
                        .iter()
                        .all(|k| bindings.axes.contains_key(*k)),
                "action input bindings missing"
            );
            let check = |entity: &str, clip: &str| -> Result<()> {
                if !state
                    .animation_cues
                    .catalog
                    .get(entity)
                    .is_some_and(|c| c.contains_key(clip))
                {
                    return Err(Diagnostic::error(
                        "action_clip",
                        format!("/entities/@{}/components/action.actor", escape(entity)),
                        format!("clip {clip} is missing"),
                    ));
                }
                Ok(())
            };
            for actor in &sim.actors {
                for clip in [
                    &actor.def.idle_clip,
                    &actor.def.run_clip,
                    &actor.def.death_clip,
                ] {
                    check(&actor.id, clip)?;
                }
                // Only abilities reachable from this actor's attack chain need its skeleton.
                let mut reachable = BTreeSet::from([actor.def.dodge.as_str()]);
                let mut id = Some(actor.def.attack.as_str());
                while let Some(ability) = id {
                    if !reachable.insert(ability) {
                        break;
                    }
                    id = sim.library.abilities[ability].chain.as_deref();
                }
                for id in reachable {
                    check(&actor.id, &sim.library.abilities[id].clip)?;
                    let a = &sim.library.abilities[id];
                    let duration = state.animation_cues.catalog[&actor.id][&a.clip];
                    ensure!(
                        duration > 0.0 && duration / (a.duration as f32 * FIXED_DT) <= 8.0,
                        "ability {id} duration is too short for the prepared clip"
                    );
                }
            }
            for sound in sim
                .library
                .abilities
                .values()
                .flat_map(|a| a.events.iter().map(|e| e.sound.as_str()))
                .chain([sim.session.hit_sound.as_str()])
            {
                ensure!(
                    state.audio.waves.contains_key(sound),
                    "audio cue missing {sound}"
                );
            }
            let mut system = ActionSystem::new_system(sim, state, bindings.buttons);
            system.sync(state);
            Ok(Box::new(system))
        });
}
impl ActionSystem {
    fn new_system(
        sim: Simulation,
        state: &RuntimeState,
        buttons: BTreeMap<String, String>,
    ) -> Self {
        Self {
            yaw: sim.session.camera_yaw,
            sim,
            held: BTreeSet::new(),
            clips: BTreeMap::new(),
            definitions: definitions(state),
            buttons,
            victory_sent: false,
        }
    }
    fn cue(cues: &mut AnimationCues, entity: &str, clip: &str, duration: Option<u32>) {
        let speed = duration
            .map_or(1.0, |ticks| {
                cues.catalog[entity][clip] / (ticks as f32 * FIXED_DT)
            })
            .clamp(0.01, 8.0);
        cues.pending.retain(|p| p.entity != entity);
        cues.pending.push(PlayArgs {
            entity: entity.into(),
            clip: clip.into(),
            fade_seconds: 0.07,
            looping: duration.is_none(),
            speed,
        });
    }
    fn sync(&mut self, state: &mut RuntimeState) {
        for event in self.sim.drain_events() {
            match event.kind.as_str() {
                "ability" => {
                    let a = &self.sim.library.abilities[event.value.as_ref().unwrap()];
                    Self::cue(
                        &mut state.animation_cues,
                        &event.actor,
                        &a.clip,
                        Some(a.duration),
                    );
                    self.clips.insert(event.actor, a.clip.clone());
                }
                "death" => {
                    let faction = self
                        .sim
                        .actors
                        .iter()
                        .find(|a| a.id == event.actor)
                        .unwrap()
                        .def
                        .faction;
                    state.events.publish(
                        "action.simulate",
                        "action.defeated",
                        state.tick,
                        [
                            (
                                "entity".into(),
                                axiom_core::gameplay::Scalar::Text(event.actor.clone()),
                            ),
                            (
                                "faction".into(),
                                axiom_core::gameplay::Scalar::Int(i64::from(faction)),
                            ),
                        ]
                        .into(),
                    );
                    let clip = event.value.unwrap();
                    Self::cue(&mut state.animation_cues, &event.actor, &clip, Some(70));
                    self.clips.insert(event.actor, clip);
                }
                "sound" => state.audio.emit(event.tick, event.value.as_ref().unwrap()),
                _ => {}
            }
        }
        if self.sim.victory() && !self.victory_sent {
            state.events.publish(
                "action.simulate",
                "action.victory",
                state.tick,
                BTreeMap::new(),
            );
            self.victory_sent = true;
        }
        for actor in &self.sim.actors {
            let slot = state.compiled.indices[&actor.id];
            let t = state.transforms[slot].as_mut().unwrap();
            t.position = [actor.position.x, 0.0, actor.position.y];
            t.rotation = Quat::from_rotation_y(actor.yaw + actor.def.model_yaw).to_array();
            state.set_health(&actor.id, actor.health);
            if actor.active.is_none() && actor.health > 0.0 {
                let clip = if actor.moving {
                    &actor.def.run_clip
                } else {
                    &actor.def.idle_clip
                };
                if self.clips.get(&actor.id) != Some(clip) {
                    Self::cue(&mut state.animation_cues, &actor.id, clip, None);
                    self.clips.insert(actor.id.clone(), clip.clone());
                }
            }
        }
        self.camera(state);
    }
    fn camera(&self, state: &mut RuntimeState) {
        let hero = &self.sim.actors[self.sim.hero];
        let target = Vec3::new(hero.position.x, 1.0, hero.position.y);
        let offset = Vec3::new(
            self.yaw.sin() * self.sim.session.camera_distance,
            self.sim.session.camera_height,
            self.yaw.cos() * self.sim.session.camera_distance,
        );
        let eye = target + offset;
        let rotation =
            Quat::from_mat4(&Mat4::look_at_rh(eye, target, Vec3::Y).inverse()).normalize();
        let slot = state.compiled.indices[&self.sim.session.camera];
        let transform = state.transforms[slot].as_mut().unwrap();
        transform.position = eye.to_array();
        transform.rotation = rotation.to_array();
    }
    fn input(&mut self, input: &ActionMap) {
        let pressed = ["attack", "dodge", "lock"]
            .into_iter()
            .filter(|id| input.pressed(id))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let direction = self.direction(input);
        for id in pressed.difference(&self.held) {
            match id.as_str() {
                "lock" => self.sim.lock(),
                "attack" => {
                    let _ = self.sim.request(Kind::Attack, direction);
                }
                _ => {
                    let _ = self.sim.request(Kind::Dodge, direction);
                }
            }
        }
        self.held = pressed;
    }
    fn direction(&self, input: &ActionMap) -> Vec2 {
        let forward = Vec2::new(-self.yaw.sin(), -self.yaw.cos());
        let right = Vec2::new(self.yaw.cos(), -self.yaw.sin());
        (right * input.axis("move_x") + forward * input.axis("move_z")).clamp_length_max(1.0)
    }
}
fn definitions(state: &RuntimeState) -> Value {
    json!({"world":state.compiled.source.id,"session":state.compiled.source.resources[SESSION],
        "abilities":state.compiled.source.resources[ABILITIES],
        "actors":state.compiled.entities.iter().filter(|e|e.components.contains_key("action.actor"))
        .map(|e|(&e.id,json!({"actor":e.components["action.actor"],"animation":e.components["animation.player"],"transform":e.transform,"health":e.health,"enabled":state.compiled.enabled[state.compiled.indices[&e.id]]})))
        .collect::<BTreeMap<_,_>>()})
}
impl RuntimeSystem for ActionSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn pointer(&mut self, dx: f32, _dy: f32) {
        self.yaw = (self.yaw - dx * 0.0035).rem_euclid(std::f32::consts::TAU);
    }
    fn step(&mut self, state: &mut RuntimeState, input: &ActionMap) {
        self.sim.combat = state.combat.as_ref().map(|c| c.scene.clone());
        self.yaw =
            (self.yaw + input.axis("orbit") * FIXED_DT * 1.5).rem_euclid(std::f32::consts::TAU);
        self.input(input);
        self.sim.step(self.direction(input));
        if state.combat.is_some() {
            crate::combat::snapshot(state, self.sim.combat_bodies());
            crate::combat::submit(state, std::mem::take(&mut self.sim.hits));
        }
        self.sync(state);
    }
    fn combat_bodies(&self) -> Vec<axiom_combat::Body> {
        self.sim.combat_bodies()
    }
    fn combat_obstacles(&self) -> Vec<axiom_combat::Rect> {
        self.sim
            .session
            .obstacles
            .iter()
            .map(|o| axiom_combat::Rect {
                min: o.min,
                max: o.max,
            })
            .collect()
    }
    fn combat_commit(&mut self, state: &mut RuntimeState) {
        if let Some(combat) = &state.combat {
            self.sim.commit_combat(&combat.scene, &combat.receipts);
            self.sync(state);
        }
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        _args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "action" {
            return None;
        }
        self.sim.combat = state.combat.as_ref().map(|c| c.scene.clone());
        Some((|| {
            let result = match action {
                "lock" => {
                    self.sim.lock();
                    "toggled"
                }
                "attack" => self.sim.request(Kind::Attack, Vec2::ZERO)?,
                "dodge" => self.sim.request(Kind::Dodge, Vec2::ZERO)?,
                _ => anyhow::bail!("unknown action command {action}"),
            };
            self.sync(state);
            Ok(json!({"status":result,"hero":self.sim.observation()["hero"]}))
        })())
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        let old = old
            .as_any()
            .downcast_ref::<Self>()
            .context("action transfer type")?;
        if self.definitions == old.definitions {
            self.victory_sent = old.victory_sent;
            self.sim = old.sim.clone();
            self.held = old.held.clone();
            self.clips = old.clips.clone();
            self.yaw = old.yaw;
            next.animation_cues
                .pending
                .retain(|p| !self.clips.contains_key(&p.entity));
            next.animation_cues.pending.extend(
                previous
                    .animation_cues
                    .pending
                    .iter()
                    .filter(|p| self.clips.contains_key(&p.entity))
                    .cloned(),
            );
        } else {
            // Timeline changes start a new round at the host's current fixed tick.
            self.sim.tick = next.tick;
            self.clips.clear();
        }
        self.sync(next);
        Ok(())
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("action".into(), self.sim.observation())])
    }
    fn validate(&self, state: &RuntimeState) -> Result<()> {
        ensure!(
            self.sim.actors.iter().all(|a| a.position.is_finite()
                && a.health.is_finite()
                && a.stamina.is_finite()
                && (0.0..=a.def.stamina).contains(&a.stamina)
                && walkable(&self.sim.session, a.position, a.def.radius)),
            "invalid action state"
        );
        state.validate()
    }
    fn extract(&self, _state: &RuntimeState, frame: &mut RenderFrame) {
        for (i, actor) in self.sim.actors.iter().enumerate() {
            if actor.health <= 0.0 {
                continue;
            }
            let color = if i == self.sim.hero {
                [0.1, 0.85, 0.87]
            } else {
                [1.0, 0.25, 0.10]
            };
            if let Some(active) = &actor.active {
                let ability = &self.sim.library.abilities[&active.ability];
                let elapsed = (self.sim.tick - active.started) as u32;
                if let Some(window) = ability.active {
                    let active_now = window.contains(elapsed);
                    let radius = ability.range;
                    let arc = ability.arc_degrees.to_radians() * 0.5;
                    for j in 0..14 {
                        let angle = actor.yaw - arc + 2.0 * arc * j as f32 / 13.0;
                        let p = actor.position + forward(angle) * radius;
                        frame.instances.push(Instance::new(
                            Mat4::from_scale_rotation_translation(
                                Vec3::new(0.07, if active_now { 0.25 } else { 0.025 }, 0.07),
                                Quat::IDENTITY,
                                Vec3::new(p.x, if active_now { 0.5 } else { 0.025 }, p.y),
                            ),
                            color,
                            if active_now { 1.4 } else { 0.4 },
                            false,
                        ));
                    }
                }
            }
            if self.sim.target == Some(i) {
                for j in 0..24 {
                    let angle = j as f32 * std::f32::consts::TAU / 24.0;
                    let p = actor.position + Vec2::new(angle.sin(), angle.cos()) * 0.5;
                    frame.instances.push(Instance::new(
                        Mat4::from_scale_rotation_translation(
                            Vec3::new(0.045, 0.014, 0.045),
                            Quat::IDENTITY,
                            Vec3::new(p.x, 0.018, p.y),
                        ),
                        [1.0, 0.65, 0.15],
                        1.0,
                        false,
                    ));
                }
            }
        }
    }
    fn draw_ui(
        &self,
        ctx: &egui::Context,
        state: &RuntimeState,
        ui_state: &mut UiState,
    ) -> Option<Vec<Action>> {
        if !self.sim.session.hud {
            // Fall back to the generic bar plus the content's bound ui.document.
            return None;
        }
        let mut actions = ui::draw_generic(ctx, state, ui_state);
        egui::Area::new("action_status".into())
            .anchor(egui::Align2::LEFT_TOP, [22.0, 48.0])
            .interactable(false)
            .show(ctx, |ui| {
                let gold = egui::Color32::from_rgb(247, 184, 112);
                ui.label(
                    egui::RichText::new("AXIOM / ACTION LAB")
                        .monospace()
                        .color(gold),
                );
                ui.label(
                    egui::RichText::new(
                        state
                            .compiled
                            .source
                            .title
                            .split('/')
                            .next()
                            .unwrap_or(&state.compiled.source.title)
                            .trim(),
                    )
                    .size(27.0)
                    .strong(),
                );
                let h = &self.sim.actors[self.sim.hero];
                ui.add(
                    egui::ProgressBar::new(h.health / h.max_health)
                        .desired_width(195.0)
                        .text(format!("HP {:.0}/{:.0}", h.health, h.max_health)),
                );
                ui.add(
                    egui::ProgressBar::new(h.stamina / h.def.stamina)
                        .desired_width(195.0)
                        .fill(egui::Color32::from_rgb(69, 169, 135))
                        .text(format!("STAMINA {:.0}", h.stamina)),
                );
                if let Some(active) = &h.active {
                    let a = &self.sim.library.abilities[&active.ability];
                    let elapsed = (self.sim.tick - active.started) as u32;
                    let phase = if a.invulnerable.is_some_and(|w| w.contains(elapsed)) {
                        "INVULNERABLE"
                    } else if a.active.is_some_and(|w| w.contains(elapsed)) {
                        "ACTIVE"
                    } else {
                        "WINDUP / RECOVERY"
                    };
                    ui.label(format!("{} · {phase}", active.ability));
                }
                ui.label(format!(
                    "LOCK · {}",
                    self.sim.external_target.as_deref().unwrap_or_else(|| self
                        .sim
                        .target
                        .map_or("free", |i| &self.sim.actors[i].id))
                ));
                if self.sim.victory() {
                    ui.label(egui::RichText::new("COURT CLEARED").size(23.0).color(gold));
                }
                if let Some(i) = self.sim.target {
                    let target = &self.sim.actors[i];
                    ui.add(
                        egui::ProgressBar::new(target.health / target.max_health)
                            .desired_width(195.0)
                            .fill(egui::Color32::from_rgb(183, 94, 62))
                            .text(format!(
                                "TARGET {:.0}/{:.0}",
                                target.health, target.max_health
                            )),
                    );
                }
                if let Some(target) = self
                    .sim
                    .external_target
                    .as_ref()
                    .and_then(|id| self.sim.combat.as_ref()?.bodies.get(id))
                {
                    ui.add(
                        egui::ProgressBar::new(target.health / target.max_health)
                            .desired_width(195.0)
                            .fill(egui::Color32::from_rgb(183, 94, 62))
                            .text(format!(
                                "TARGET {:.0}/{:.0}",
                                target.health, target.max_health
                            )),
                    );
                }
                if self.sim.defeated() {
                    ui.label(
                        egui::RichText::new("FALLEN · Restart to retry")
                            .size(22.0)
                            .color(gold),
                    );
                }
            });
        if state.compiled.source.resources.contains_key("ui.document") {
            return Some(actions);
        }
        egui::Area::new("action_controls".into())
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -20.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_black_alpha(205))
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.label("WASD move  ·  Mouse / Q E orbit  ·  Esc pause");
                        ui.horizontal(|ui| {
                            for name in ["attack", "dodge", "lock"] {
                                let label = format!(
                                    "{} [{}]",
                                    name,
                                    self.buttons[name]
                                        .strip_prefix("Key")
                                        .unwrap_or(&self.buttons[name])
                                );
                                if ui.button(label).clicked() {
                                    actions.push(Action::Gameplay(RuntimeCommand::Call {
                                        capability: "action".into(),
                                        action: name.into(),
                                        args: json!({}),
                                    }));
                                }
                            }
                        });
                    });
            });
        Some(actions)
    }
}
