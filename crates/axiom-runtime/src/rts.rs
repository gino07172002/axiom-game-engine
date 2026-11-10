//! Native RTS capability. The renderer and app only receive generic snapshots and pointer rays.
use crate::{
    presentation::{Instance, RenderFrame},
    ui::{Action, UiState},
    world::*,
};
use anyhow::{ensure, Context, Result};
use axiom_core::*;
use axiom_rts::{
    simulation::{dist, Simulation},
    *,
};
use glam::{Mat4, Quat, Vec3};
use serde_json::{json, Value};
use std::{any::Any, collections::BTreeMap};
type DragSelection = ([f32; 2], [f32; 2], [f32; 2], bool);
pub struct RtsSystem {
    bindings: Vec<(String, usize)>,
    pub sim: Simulation,
    templates: BTreeMap<String, CompiledEntity>,
    definitions: BTreeMap<String, (UnitDef, f32)>,
    drag: Option<DragSelection>,
    /// Last move destination and the tick it was issued (fading ground marker).
    marker: Option<([f32; 2], u64)>,
}
const PLAYER_BAR: egui::Color32 = egui::Color32::from_rgb(86, 226, 128);
const ENEMY_BAR: egui::Color32 = egui::Color32::from_rgb(240, 86, 64);
/// Compiles a world with one appended template entity per prefab and returns the
/// compiled templates, in `prefabs` order.
fn compile_with_templates(state: &RuntimeState, prefabs: &[String]) -> Result<Vec<CompiledEntity>> {
    let mut world = state.compiled.source.clone();
    let ids: Vec<String> = prefabs.iter().map(|p| format!("__template/{p}")).collect();
    for (id, prefab) in ids.iter().zip(prefabs) {
        ensure!(
            !world.entities.iter().any(|e| &e.id == id),
            "reserved runtime template ID collision"
        );
        let mut entity = Entity::new(id);
        entity.prefab = Some(prefab.clone());
        world.entities.push(entity);
    }
    // The source package and prefab contracts were validated before this derived compilation.
    let mut types = state.registry.clone();
    types.world_validators.clear();
    let compiled = world.compile(&types)?;
    Ok(ids
        .iter()
        .map(|id| compiled.entities[compiled.indices[id]].clone())
        .collect())
}
/// One template compiled alone (original behavior and error reporting).
fn compile_template(state: &RuntimeState, prefab: &str) -> Result<CompiledEntity> {
    Ok(compile_with_templates(state, &[prefab.to_owned()])?.remove(0))
}
/// All recipe templates from one compilation; `None` when that fails or there is nothing
/// to share, so the caller compiles templates one by one with the original errors.
fn compile_templates(
    state: &RuntimeState,
    sim: &Simulation,
) -> Result<Option<Vec<(String, CompiledEntity)>>> {
    let mut prefabs: Vec<String> = vec![];
    for recipe in sim.session.recipes.values() {
        if !prefabs.contains(&recipe.prefab) {
            prefabs.push(recipe.prefab.clone());
        }
    }
    if prefabs.len() < 2 {
        return Ok(None);
    }
    Ok(compile_with_templates(state, &prefabs)
        .ok()
        .map(|templates| prefabs.into_iter().zip(templates).collect()))
}

pub fn register(registry: &mut RuntimeRegistry) {
    axiom_rts::register(&mut registry.authoring);
    registry.factories.insert("rts.simulate".into(), |state| {
        let sim = Simulation::new(&state.compiled)?;
        let mut templates = BTreeMap::new();
        let mut definitions = BTreeMap::new();
        // One derived compilation resolves every template; compiling each template alone
        // (the fallback, which also reports the original per-template errors) produced the
        // same entities at one full-world compile per prefab.
        if let Some(batch) = compile_templates(state, &sim)? {
            for (prefab, template) in batch {
                definitions.insert(
                    prefab.clone(),
                    (
                        serde_json::from_value(template.components["rts.unit"].clone())?,
                        template.health.as_ref().unwrap().value,
                    ),
                );
                templates.insert(prefab, template);
            }
        }
        for recipe in sim.session.recipes.values() {
            if templates.contains_key(&recipe.prefab) {
                continue;
            }
            let template = compile_template(state, &recipe.prefab)?;
            definitions.insert(
                recipe.prefab.clone(),
                (
                    serde_json::from_value(template.components["rts.unit"].clone())?,
                    template.health.as_ref().unwrap().value,
                ),
            );
            templates.insert(recipe.prefab.clone(), template);
        }
        let bindings = sim
            .session
            .ui_bindings
            .iter()
            .map(|(metric, variable)| {
                let gameplay = state
                    .gameplay
                    .as_ref()
                    .context("UI bindings require behavior.execute before RTS")?;
                let i = gameplay
                    .compiled
                    .variables
                    .iter()
                    .position(|id| id == variable)
                    .context("UI binding variable missing")?;
                Ok((metric.clone(), i))
            })
            .collect::<Result<Vec<_>>>()?;
        let module = RtsSystem {
            bindings,
            sim,
            templates,
            definitions,
            drag: None,
            marker: None,
        };
        module.sync(state);
        Ok(Box::new(module))
    });
}
impl RtsSystem {
    fn sync(&self, state: &mut RuntimeState) {
        if let Some(gameplay) = &mut state.gameplay {
            let player = self.sim.session.player_faction;
            let count = |mine: bool, role: Option<Role>| {
                self.sim
                    .units
                    .iter()
                    .filter(|u| {
                        u.health > 0.0
                            && (u.def.faction == player) == mine
                            && role.is_none_or(|r| u.def.role == r)
                    })
                    .count() as i64
            };
            let hq_percent = |mine: bool| {
                let (hp, max) = self
                    .sim
                    .buildings
                    .iter()
                    .filter(|b| b.def.depot && (b.def.faction == player) == mine)
                    .fold((0.0, 0.0), |(h, m), b| (h + b.health, m + b.max_health));
                if max > 0.0 {
                    (hp / max * 100.0).ceil() as i64
                } else {
                    0
                }
            };
            let alert = || {
                self.sim
                    .ai
                    .wave_tick
                    .is_some_and(|t| self.sim.tick.saturating_sub(t) < 600)
                    || self.sim.buildings.iter().any(|b| {
                        b.health > 0.0
                            && b.def.faction == player
                            && self.sim.units.iter().any(|u| {
                                u.health > 0.0
                                    && u.def.faction != player
                                    && dist(u.position, b.position) < 12.0
                            })
                    })
            };
            for (metric, i) in &self.bindings {
                use axiom_core::gameplay::Scalar;
                let value = match metric.as_str() {
                    "workers" => Scalar::Int(count(true, Some(Role::Worker))),
                    "soldiers" => Scalar::Int(count(true, Some(Role::Soldier))),
                    "enemy_units" => Scalar::Int(count(false, None)),
                    "wave" => Scalar::Int(i64::from(self.sim.ai.wave)),
                    "base_health" => Scalar::Int(hq_percent(true)),
                    "enemy_base_health" => Scalar::Int(hq_percent(false)),
                    "alert" => Scalar::Bool(!self.sim.victory && !self.sim.defeated && alert()),
                    "minerals" => Scalar::Int(
                        self.sim.balance[self.sim.session.player_faction as usize] as i64,
                    ),
                    "units" => Scalar::Int(
                        self.sim
                            .units
                            .iter()
                            .filter(|u| {
                                u.health > 0.0 && u.def.faction == self.sim.session.player_faction
                            })
                            .count() as i64,
                    ),
                    "selected" => Scalar::Int(self.sim.selected_ids().len() as i64),
                    "queued" => Scalar::Int(
                        self.sim
                            .buildings
                            .iter()
                            .filter(|b| b.def.faction == self.sim.session.player_faction)
                            .map(|b| b.queue.len())
                            .sum::<usize>() as i64,
                    ),
                    "delivered" => Scalar::Int(self.sim.stats.delivered as i64),
                    "produced" => Scalar::Int(self.sim.stats.produced as i64),
                    "victory" => Scalar::Bool(self.sim.victory),
                    "defeated" => Scalar::Bool(self.sim.defeated),
                    _ => continue,
                };
                gameplay.values[*i] = value;
            }
        }
        for u in &self.sim.units {
            state.set_position(&u.id, [u.position[0], 0.0, u.position[1]]);
            state.set_health(&u.id, u.health);
            if let Some(&slot) = state.compiled.indices.get(&u.id) {
                state.compiled.enabled[slot] = u.health > 0.0;
                if let Some(t) = &mut state.transforms[slot] {
                    t.rotation = Quat::from_rotation_y(u.yaw).to_array();
                }
            }
        }
        for b in &self.sim.buildings {
            state.set_health(&b.id, b.health);
            if let Some(&slot) = state.compiled.indices.get(&b.id) {
                state.compiled.enabled[slot] = b.health > 0.0;
            }
        }
    }
    fn spawn(&self, state: &mut RuntimeState, id: &str) -> Result<()> {
        let u = self
            .sim
            .units
            .iter()
            .find(|u| u.id == id)
            .context("produced actor missing")?;
        let prefab = u.produced.as_ref().context("unit was not produced")?;
        let mut template = self.templates[prefab].clone();
        template.components.get_mut("rts.unit").unwrap()["faction"] = json!(u.def.faction);
        if u.def.faction == 1 {
            let pairs = &self.sim.session.team_colors;
            if let Some(mesh) = template.mesh.as_mut() {
                if pairs.is_empty() {
                    if let Some(part) = mesh.parts.first_mut() {
                        part.color = [0.9, 0.26, 0.18];
                    }
                }
                for part in &mut mesh.parts {
                    if let Some(pair) = pairs.iter().find(|p| p[0] == part.color) {
                        part.color = pair[1];
                    }
                }
            }
        }
        state.spawn(&template, id.into(), [u.position[0], 0.0, u.position[1]])?;
        Ok(())
    }
    fn call(&mut self, action: &str, args: &Value) -> Result<Value> {
        match action {
            "select_role" => self.sim.select_role(serde_json::from_value(args.clone())?),
            "select" => self.sim.select(serde_json::from_value(args.clone())?),
            "select_rect" => self.sim.select_rect(serde_json::from_value(args.clone())?),
            "move" => {
                let args: MoveArgs = serde_json::from_value(args.clone())?;
                let result = self.sim.move_group(args.clone())?;
                self.marker = Some((args.destination, self.sim.tick));
                Ok(result)
            }
            "gather" => self.sim.gather(serde_json::from_value(args.clone())?),
            "attack" => self.sim.attack(serde_json::from_value(args.clone())?),
            "train" => self.sim.train(serde_json::from_value(args.clone())?),
            "stop" => self.sim.stop(serde_json::from_value(args.clone())?),
            "inspect" => self.sim.inspect(serde_json::from_value(args.clone())?),
            _ => anyhow::bail!("unknown RTS action {action}"),
        }
    }
    /// Ground point under the camera center and the visible ground half-extents (x, z).
    fn camera_footprint(
        state: &RuntimeState,
        camera: &str,
        aspect: f32,
    ) -> Option<([f32; 2], [f32; 2])> {
        let slot = *state.compiled.indices.get(camera)?;
        let t = state.transforms[slot].as_ref()?;
        let forward = Quat::from_array(t.rotation) * Vec3::NEG_Z;
        if forward.y > -1e-3 {
            return None;
        }
        let eye = Vec3::from_array(t.position);
        let ground = eye + forward * (-eye.y / forward.y);
        let height = match state.compiled.entities[slot].camera.as_ref()?.projection {
            Projection::Orthographic { height } => height,
            Projection::Perspective { .. } => 30.0,
        };
        Some((
            [ground.x, ground.z],
            [height * aspect * 0.5, height * 0.5 / -forward.y],
        ))
    }
    fn look_at(&self, state: &mut RuntimeState, center: [f32; 2]) -> Result<Value> {
        ensure!(center.iter().all(|v| v.is_finite()), "non-finite center");
        let slot = *state
            .compiled
            .indices
            .get(&self.sim.session.camera)
            .context("RTS camera missing")?;
        let t = state.transforms[slot]
            .as_mut()
            .context("RTS camera has no transform")?;
        let forward = Quat::from_array(t.rotation) * Vec3::NEG_Z;
        ensure!(forward.y < -1e-3, "RTS camera must look down");
        let reach = forward * (-t.position[1] / forward.y);
        t.position[0] = (center[0] - reach.x).clamp(-40.0, 40.0);
        t.position[2] = (center[1] - reach.z).clamp(-18.0, 60.0);
        Ok(json!({"camera":[t.position[0], t.position[2]]}))
    }
    /// Health bars over damaged/selected units and every structure, plus production progress.
    fn draw_bars(&self, ctx: &egui::Context, state: &RuntimeState) {
        use egui::{pos2, vec2, Color32, Rect};
        let Some(view) = crate::nameplate::active_view(state) else {
            return;
        };
        let screen = ctx.screen_rect();
        let bounds = [screen.min.x, screen.min.y, screen.width(), screen.height()];
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("rts_bars"),
        ));
        let player = self.sim.session.player_faction;
        let bar = |center: egui::Pos2, width: f32, height: f32, fraction: f32, color: Color32| {
            let rect = Rect::from_center_size(center, vec2(width + 2.0, height + 2.0));
            painter.rect_filled(rect, 1.5, Color32::from_black_alpha(175));
            let inner = rect.shrink(1.0);
            let mut fill = inner;
            fill.set_width(inner.width() * fraction.clamp(0.0, 1.0));
            painter.rect_filled(fill, 1.0, color);
        };
        let tint = |faction: u8| {
            if faction == player {
                PLAYER_BAR
            } else {
                ENEMY_BAR
            }
        };
        for (i, u) in self.sim.units.iter().enumerate() {
            let selected = self.sim.selected.contains(&i);
            if u.health <= 0.0 || (u.health >= u.max_health && !selected) {
                continue;
            }
            if let Some([x, y]) = view.project(Vec3::new(u.position[0], 1.3, u.position[1]), bounds)
            {
                bar(
                    pos2(x, y),
                    26.0,
                    4.0,
                    u.health / u.max_health,
                    tint(u.def.faction),
                );
            }
        }
        for b in &self.sim.buildings {
            if b.health <= 0.0 {
                continue;
            }
            let height = if b.def.depot { 4.2 } else { 4.4 };
            let Some([x, y]) =
                view.project(Vec3::new(b.position[0], height, b.position[1]), bounds)
            else {
                continue;
            };
            let width = if b.def.depot { 76.0 } else { 52.0 };
            bar(
                pos2(x, y),
                width,
                7.0,
                b.health / b.max_health.max(1.0),
                tint(b.def.faction),
            );
            if b.def.faction != player {
                continue;
            }
            if let Some(job) = b.queue.front() {
                let total = self
                    .sim
                    .session
                    .recipes
                    .get(&job.recipe)
                    .map_or(1, |r| r.build_ticks)
                    .max(1);
                let done = 1.0 - job.remaining as f32 / total as f32;
                bar(
                    pos2(x, y + 9.0),
                    width,
                    4.0,
                    done,
                    Color32::from_rgb(255, 196, 64),
                );
                painter.text(
                    pos2(x + width * 0.5 + 6.0, y + 4.0),
                    egui::Align2::LEFT_CENTER,
                    format!("×{}", b.queue.len()),
                    egui::FontId::proportional(13.0),
                    Color32::from_rgb(255, 214, 120),
                );
            }
        }
    }
    /// Minimap: terrain, structures, crystals, units and the camera footprint. Left click/drag
    /// recenters the camera, right click moves the selection (same typed commands as AI).
    fn draw_minimap(&self, ctx: &egui::Context, state: &RuntimeState, actions: &mut Vec<Action>) {
        use egui::{pos2, vec2, Color32, Rect, Sense};
        let grid = &self.sim.session.grid;
        let world = [
            grid.width as f32 * grid.cell_size,
            grid.height as f32 * grid.cell_size,
        ];
        let scale = (240.0 / world[0]).min(170.0 / world[1]);
        let size = vec2(world[0] * scale, world[1] * scale);
        let player = self.sim.session.player_faction;
        let screen = ctx.screen_rect();
        let aspect = screen.width() / screen.height().max(1.0);
        let footprint = Self::camera_footprint(state, &self.sim.session.camera, aspect);
        egui::Area::new(egui::Id::new("rts_minimap"))
            .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(10, 14, 20, 225))
                    .corner_radius(8.0)
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        let (response, painter) =
                            ui.allocate_painter(size, Sense::click_and_drag());
                        let rect = response.rect;
                        let at = |p: [f32; 2]| {
                            pos2(
                                rect.left() + (p[0] - grid.origin[0]) * scale,
                                rect.top() + (p[1] - grid.origin[1]) * scale,
                            )
                        };
                        painter.rect_filled(rect, 3.0, Color32::from_rgb(50, 72, 46));
                        let cell = grid.cell_size * scale;
                        for c in &grid.blocked {
                            let min = [
                                grid.origin[0] + c[0] as f32 * grid.cell_size,
                                grid.origin[1] + c[1] as f32 * grid.cell_size,
                            ];
                            painter.rect_filled(
                                Rect::from_min_size(at(min), vec2(cell, cell)),
                                0.0,
                                Color32::from_rgb(104, 100, 92),
                            );
                        }
                        for r in self.sim.resources.iter().filter(|r| r.remaining > 0) {
                            painter.circle_filled(
                                at(r.position),
                                3.2,
                                Color32::from_rgb(90, 235, 255),
                            );
                        }
                        for b in &self.sim.buildings {
                            let color = if b.health <= 0.0 {
                                Color32::from_rgb(60, 58, 56)
                            } else if b.def.faction == player {
                                Color32::from_rgb(70, 160, 255)
                            } else {
                                Color32::from_rgb(240, 80, 60)
                            };
                            let s = if b.def.depot { 9.0 } else { 6.0 };
                            painter.rect_filled(
                                Rect::from_center_size(at(b.position), vec2(s, s)),
                                1.0,
                                color,
                            );
                        }
                        for (i, u) in self.sim.units.iter().enumerate() {
                            if u.health <= 0.0 {
                                continue;
                            }
                            let color = if self.sim.selected.contains(&i) {
                                Color32::WHITE
                            } else if u.def.faction == player {
                                Color32::from_rgb(120, 200, 255)
                            } else {
                                Color32::from_rgb(255, 110, 90)
                            };
                            painter.circle_filled(at(u.position), 1.8, color);
                        }
                        if let Some((center, half)) = footprint {
                            let view = Rect::from_min_max(
                                at([center[0] - half[0], center[1] - half[1]]),
                                at([center[0] + half[0], center[1] + half[1]]),
                            )
                            .intersect(rect);
                            painter.rect_stroke(
                                view,
                                0.0,
                                egui::Stroke::new(1.2_f32, Color32::from_white_alpha(200)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let world_at = |p: egui::Pos2| {
                            [
                                grid.origin[0] + (p.x - rect.left()) / scale,
                                grid.origin[1] + (p.y - rect.top()) / scale,
                            ]
                        };
                        let call = |action: &str, args: Value| {
                            Action::Gameplay(axiom_core::gameplay::RuntimeCommand::Call {
                                capability: "rts".into(),
                                action: action.into(),
                                args,
                            })
                        };
                        if let Some(p) = response.interact_pointer_pos() {
                            if response.clicked()
                                || response.dragged_by(egui::PointerButton::Primary)
                            {
                                actions.push(call("camera", json!({"center": world_at(p)})));
                            } else if response.secondary_clicked() && !self.sim.selected.is_empty()
                            {
                                actions.push(call("move", json!({"destination": world_at(p)})));
                            }
                        }
                    });
            });
    }
    fn terrain(pointer: &ScreenPointer) -> Option<[f32; 2]> {
        let origin = Vec3::from_array(pointer.origin);
        let direction = Vec3::from_array(pointer.direction);
        if direction.y.abs() < 0.0001 {
            return None;
        }
        let t = -origin.y / direction.y;
        if t < 0.0 {
            return None;
        }
        let p = origin + direction * t;
        Some([p.x, p.z])
    }
}
impl RuntimeSystem for RtsSystem {
    fn input_lost(&mut self) {
        self.drag = None;
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn validate(&self, _state: &RuntimeState) -> Result<()> {
        self.sim.validate()
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "rts" {
            return None;
        }
        if action == "camera" {
            return Some(
                serde_json::from_value::<CameraArgs>(args.clone())
                    .map_err(Into::into)
                    .and_then(|a| self.look_at(state, a.center)),
            );
        }
        self.sim
            .set_combat(state.combat.as_ref().map(|c| c.scene.clone()));
        let result = self.call(action, args);
        if result.is_ok() {
            self.sync(state);
        }
        Some(result)
    }
    fn step(&mut self, state: &mut RuntimeState, input: &ActionMap) {
        self.sim
            .set_combat(state.combat.as_ref().map(|c| c.scene.clone()));
        let delivered = self.sim.stats.delivered;
        let balance = self.sim.balance;
        let production_backup = self
            .sim
            .buildings
            .iter()
            .any(|b| b.health > 0.0 && b.queue.front().is_some_and(|j| j.remaining <= 1))
            .then(|| self.sim.clone());
        self.sim.tick = state.tick.saturating_sub(1);
        self.sim.step(&self.definitions);
        let produced = std::mem::take(&mut self.sim.spawned);
        if !produced.is_empty() {
            // `RuntimeState::spawn` validates everything before it mutates, so one spawn is
            // already atomic; only a batch needs a scratch copy for all-or-nothing.
            let result = if let [id] = produced.as_slice() {
                self.spawn(state, id)
            } else {
                let mut prepared = state.clone();
                produced
                    .iter()
                    .try_for_each(|id| self.spawn(&mut prepared, id))
                    .map(|()| *state = prepared)
            };
            match result {
                Ok(()) => {
                    for id in &produced {
                        state.events.publish(
                            "rts.simulate",
                            "rts.produced",
                            state.tick,
                            [(
                                "entity".into(),
                                axiom_core::gameplay::Scalar::Text(id.clone()),
                            )]
                            .into(),
                        );
                    }
                }
                Err(e) => {
                    if let Some(previous) = production_backup {
                        self.sim = previous;
                    }
                    self.sim.last_error = Some(format!("runtime_spawn_failed:{e:#}"));
                }
            }
        }
        if self.sim.stats.delivered > delivered {
            for (faction, previous) in balance.into_iter().enumerate() {
                let amount = self.sim.balance[faction].saturating_sub(previous);
                if amount > 0 {
                    state.events.publish(
                        "rts.simulate",
                        "rts.delivered",
                        state.tick,
                        [
                            (
                                "amount".into(),
                                axiom_core::gameplay::Scalar::Int(i64::from(amount)),
                            ),
                            (
                                "faction".into(),
                                axiom_core::gameplay::Scalar::Int(faction as i64),
                            ),
                        ]
                        .into(),
                    );
                }
            }
        }
        if let Some(&slot) = state.compiled.indices.get(&self.sim.session.camera) {
            if let Some(t) = &mut state.transforms[slot] {
                t.position[0] =
                    (t.position[0] + input.axis("camera.x") * 18.0 * FIXED_DT).clamp(-40.0, 40.0);
                t.position[2] =
                    (t.position[2] + input.axis("camera.z") * 18.0 * FIXED_DT).clamp(-18.0, 60.0);
            }
            if let Some(camera) = &mut state.compiled.entities[slot].camera {
                if let Projection::Orthographic { height } = &mut camera.projection {
                    *height =
                        (*height + input.axis("camera.zoom") * 20.0 * FIXED_DT).clamp(16.0, 80.0);
                }
            }
        }
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
        let grid = &self.sim.session.grid;
        grid.blocked
            .iter()
            .map(|c| {
                let min = [
                    grid.origin[0] + c[0] as f32 * grid.cell_size,
                    grid.origin[1] + c[1] as f32 * grid.cell_size,
                ];
                axiom_combat::Rect {
                    min,
                    max: [min[0] + grid.cell_size, min[1] + grid.cell_size],
                }
            })
            .collect()
    }
    fn combat_commit(&mut self, state: &mut RuntimeState) {
        if let Some(combat) = &state.combat {
            self.sim.commit_combat(&combat.scene, &combat.receipts);
            self.sync(state);
        }
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
            .context("RTS transfer type mismatch")?;
        if previous.compiled.source.id != next.compiled.source.id {
            return Ok(());
        }
        self.sim.restore(&old.sim)?;
        for u in &self.sim.units {
            if u.produced.is_some() && !next.compiled.indices.contains_key(&u.id) {
                self.spawn(next, &u.id)?;
                if let Some(&old_slot) = previous.compiled.indices.get(&u.id) {
                    let slot = next.compiled.indices[&u.id];
                    next.ids[slot] = previous.ids[old_slot];
                }
            }
        }
        self.sync(next);
        Ok(())
    }
    fn uses_screen_pointer(&self) -> bool {
        true
    }
    fn screen_pointer(&mut self, state: &mut RuntimeState, pointer: &ScreenPointer) -> Result<()> {
        self.sim
            .set_combat(state.combat.as_ref().map(|c| c.scene.clone()));
        if pointer.kind == PointerKind::Move {
            // Off the terrain too: a run of moves must end where the last one is.
            if let Some(drag) = &mut self.drag {
                drag.2 = pointer.screen;
            }
            return Ok(());
        }
        let Some(p) = Self::terrain(pointer) else {
            return Ok(());
        };
        match pointer.kind {
            PointerKind::PrimaryDown => {
                self.drag = Some((pointer.screen, p, pointer.screen, pointer.additive))
            }
            PointerKind::Move => {}
            PointerKind::PrimaryUp => {
                if let Some((screen, start, _, additive)) = self.drag.take() {
                    if Vec3::new(
                        screen[0] - pointer.screen[0],
                        screen[1] - pointer.screen[1],
                        0.0,
                    )
                    .length()
                        < 0.008
                    {
                        let nearest = self
                            .sim
                            .units
                            .iter()
                            .filter(|u| {
                                u.health > 0.0
                                    && u.def.faction == self.sim.session.player_faction
                                    && dist(u.position, p) < 1.0
                            })
                            .min_by(|a, b| dist(a.position, p).total_cmp(&dist(b.position, p)))
                            .map(|u| u.id.clone());
                        self.sim.select(SelectArgs {
                            ids: nearest.into_iter().collect(),
                            additive,
                        })?;
                    } else {
                        self.sim.select_rect(RectArgs {
                            min: start,
                            max: p,
                            additive,
                        })?;
                    }
                }
            }
            PointerKind::SecondaryDown => {
                if self.sim.selected.is_empty() {
                    return Ok(());
                }
                if let Some(node) = self
                    .sim
                    .resources
                    .iter()
                    .find(|n| n.remaining > 0 && dist(n.position, p) < 1.8)
                {
                    self.sim.gather(TargetArgs {
                        units: None,
                        target: node.id.clone(),
                        engage: false,
                    })?;
                } else if let Some(enemy) = self
                    .sim
                    .combat
                    .as_ref()
                    .and_then(|scene| {
                        scene
                            .bodies
                            .values()
                            .filter(|b| {
                                b.alive()
                                    && b.faction != self.sim.session.player_faction
                                    && dist(b.position.to_array(), p) < b.radius.max(1.0)
                            })
                            .min_by(|a, b| {
                                a.position
                                    .distance_squared(glam::Vec2::from_array(p))
                                    .total_cmp(
                                        &b.position.distance_squared(glam::Vec2::from_array(p)),
                                    )
                            })
                            .map(|b| b.handle.id.clone())
                    })
                    .or_else(|| {
                        self.sim
                            .units
                            .iter()
                            .find(|u| {
                                u.health > 0.0
                                    && u.def.faction != self.sim.session.player_faction
                                    && dist(u.position, p) < 1.0
                            })
                            .map(|u| u.id.clone())
                            .or_else(|| {
                                self.sim
                                    .buildings
                                    .iter()
                                    .find(|b| {
                                        b.health > 0.0
                                            && b.def.faction != self.sim.session.player_faction
                                            && dist(b.position, p) < 2.0
                                    })
                                    .map(|b| b.id.clone())
                            })
                    })
                {
                    self.sim.attack(TargetArgs {
                        units: None,
                        target: enemy,
                        engage: false,
                    })?;
                } else {
                    self.sim.move_group(MoveArgs {
                        units: None,
                        destination: p,
                    })?;
                    self.marker = Some((p, self.sim.tick));
                }
            }
        }
        self.sync(state);
        Ok(())
    }
    fn extract(&self, _state: &RuntimeState, frame: &mut RenderFrame) {
        use std::f32::consts::{FRAC_PI_2, PI, TAU};
        let tick = self.sim.tick;
        let player = self.sim.session.player_faction;
        let mut push = |scale: Vec3, angle: f32, at: Vec3, color: [f32; 3], emission: f32| {
            frame.instances.push(Instance::new(
                Mat4::from_scale_rotation_translation(scale, Quat::from_rotation_y(angle), at),
                color,
                emission,
                false,
            ));
        };
        let mut ring = |center: [f32; 2], radius: f32, color: [f32; 3], emission: f32| {
            for k in 0..12 {
                let a = k as f32 / 12.0 * TAU;
                push(
                    Vec3::new(TAU * radius / 12.0 * 0.8, 0.03, 0.07),
                    -(a + FRAC_PI_2),
                    Vec3::new(
                        center[0] + a.cos() * radius,
                        0.03,
                        center[1] + a.sin() * radius,
                    ),
                    color,
                    emission,
                );
            }
        };
        for &i in &self.sim.selected {
            let u = &self.sim.units[i];
            if u.health > 0.0 {
                ring(u.position, u.def.radius + 0.22, [0.45, 1.0, 0.6], 1.2);
            }
        }
        if let Some((p, t0)) = self.marker {
            let age = tick.saturating_sub(t0) as f32;
            if age < 60.0 {
                ring(
                    p,
                    0.3 + age / 60.0 * 0.6,
                    [1.0, 0.85, 0.3],
                    2.0 * (1.0 - age / 60.0),
                );
            }
        }
        // Projectiles, impacts and melee slashes.
        for s in &self.sim.shots {
            let age = tick.saturating_sub(s.tick) as f32;
            let from = Vec3::new(s.from[0], 0.0, s.from[1]);
            let to = Vec3::new(s.to[0], 0.0, s.to[1]);
            let heading = (to.x - from.x).atan2(to.z - from.z);
            let color = if s.faction == player {
                [0.55, 0.9, 1.0]
            } else {
                [1.0, 0.6, 0.3]
            };
            if s.ranged {
                let flight = (from.distance(to) / 0.6).clamp(6.0, 18.0);
                if age <= flight {
                    let t = age / flight;
                    let p = from.lerp(to, t) + Vec3::Y * (0.9 + (t * PI).sin() * 0.8);
                    push(Vec3::new(0.09, 0.09, 0.55), heading, p, color, 3.0);
                } else if age <= flight + 8.0 {
                    let k = 1.0 - (age - flight) / 8.0;
                    push(
                        Vec3::splat(0.12 + 0.4 * k),
                        age * 0.5,
                        to + Vec3::Y * 0.6,
                        [1.0, 0.95, 0.7],
                        4.0 * k,
                    );
                }
            } else if age <= 8.0 {
                let k = 1.0 - age / 8.0;
                push(
                    Vec3::new(0.15 + 0.6 * k, 0.06, 0.12),
                    heading + FRAC_PI_2 + age * 0.35,
                    to + Vec3::Y * 0.6,
                    [1.0, 0.95, 0.75],
                    4.0 * k,
                );
            }
        }
        // Fallen units (fade into the ground), dust, and carried crystals.
        for (n, u) in self.sim.units.iter().enumerate() {
            let at = Vec3::new(u.position[0], 0.0, u.position[1]);
            if let Some(died) = u.died {
                let age = tick.saturating_sub(died) as f32;
                if age < 240.0 {
                    let body = if u.def.faction == player {
                        [0.1, 0.24, 0.45]
                    } else {
                        [0.42, 0.12, 0.1]
                    };
                    push(
                        Vec3::new(0.42, 0.14, 0.75),
                        u.yaw,
                        at + Vec3::Y * (0.07 - age / 240.0 * 0.14),
                        body,
                        0.0,
                    );
                    if age < 30.0 {
                        let k = age / 30.0;
                        for j in 0..4 {
                            let a = j as f32 * FRAC_PI_2 + n as f32;
                            push(
                                Vec3::splat(0.22 * (1.0 - k) + 0.04),
                                a,
                                at + Vec3::new(a.cos() * k * 0.7, 0.3 + k * 0.5, a.sin() * k * 0.7),
                                [0.78, 0.74, 0.68],
                                0.0,
                            );
                        }
                    }
                }
            } else if u.health > 0.0 && u.carried > 0 {
                push(
                    Vec3::splat(0.16),
                    tick as f32 * 0.05,
                    at + Vec3::Y * 1.18,
                    [0.35, 0.95, 1.0],
                    2.0,
                );
            }
        }
        // Burning and destroyed structures.
        for (k, b) in self.sim.buildings.iter().enumerate() {
            let at = Vec3::new(b.position[0], 0.0, b.position[1]);
            let fraction = b.health / b.max_health.max(1.0);
            if b.health <= 0.0 {
                for j in 0..8u32 {
                    let h = |m: u32| ((j * 37 + k as u32 * 11 + m * 7) % 13) as f32 / 13.0;
                    push(
                        Vec3::new(0.5 + h(1) * 0.8, 0.25 + h(2) * 0.6, 0.5 + h(3) * 0.7),
                        h(4) * 3.0,
                        at + Vec3::new(h(5) * 2.6 - 1.3, 0.15, h(6) * 2.6 - 1.3),
                        [0.2 + h(7) * 0.06, 0.19, 0.18],
                        0.0,
                    );
                }
                let ember = ((tick / 6 + k as u64) % 3) as f32 / 3.0;
                push(
                    Vec3::splat(0.25),
                    0.5,
                    at + Vec3::Y * 0.5,
                    [1.0, 0.45, 0.1],
                    1.5 + ember * 2.0,
                );
            } else if fraction < 0.4 {
                for j in 0..2 {
                    let flicker = ((tick / 4 + j) % 5) as f32 / 5.0;
                    push(
                        Vec3::new(0.3, 0.4 + flicker * 0.3, 0.3),
                        flicker,
                        at + Vec3::new(0.7 - j as f32 * 1.3, 2.1 + flicker * 0.2, 0.6),
                        [1.0, 0.5 + flicker * 0.2, 0.12],
                        3.0,
                    );
                }
            }
        }
    }
    fn draw_ui(
        &self,
        ctx: &egui::Context,
        state: &RuntimeState,
        ui: &mut UiState,
    ) -> Option<Vec<Action>> {
        let mut actions = crate::ui::draw_generic(ctx, state, ui);
        self.draw_bars(ctx, state);
        if self.sim.session.minimap {
            self.draw_minimap(ctx, state, &mut actions);
        }
        if let Some((start, _, end, _)) = self.drag {
            let screen = ctx.screen_rect();
            let a = egui::pos2(start[0] * screen.width(), start[1] * screen.height());
            let b = egui::pos2(end[0] * screen.width(), end[1] * screen.height());
            let rect = egui::Rect::from_two_pos(a, b);
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("rts_drag"),
            ));
            painter.rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(90, 230, 130, 28),
            );
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.5_f32, egui::Color32::LIGHT_GREEN),
                egui::StrokeKind::Inside,
            );
        }
        Some(actions)
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([(
            "rts".into(),
            json!({"summary":self.sim.summary(),"selected":self.sim.selected_ids(),"resources":self.sim.resources.iter().map(|r|json!({"id":r.id,"remaining":r.remaining})).collect::<Vec<_>>(),"buildings":self.sim.buildings.iter().map(|b|json!({"id":b.id,"health":b.health,"queue":b.queue.iter().map(|j|json!({"recipe":j.recipe,"remaining_ticks":j.remaining})).collect::<Vec<_>>()})).collect::<Vec<_>>() }),
        )])
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;

    #[test]
    fn batched_templates_equal_templates_compiled_one_by_one() {
        let registry = crate::installed_registry();
        for pack in ["rts", "rts-bench"] {
            let world = axiom_core::ContentPackage::load(
                format!("{}/../../content/{pack}", env!("CARGO_MANIFEST_DIR")),
                &registry.authoring,
            )
            .unwrap()
            .world;
            let runtime = WorldRuntime::new(world, 1, registry.clone()).unwrap();
            let state = &runtime.state;
            let sim = &runtime.system::<RtsSystem>().unwrap().sim;
            let batch = compile_templates(state, sim)
                .unwrap()
                .expect("several prefabs");
            assert!(batch.len() >= 2, "{pack}");
            for (prefab, template) in batch {
                let alone = compile_template(state, &prefab).unwrap();
                assert_eq!(
                    format!("{template:?}"),
                    format!("{alone:?}"),
                    "{pack} {prefab}"
                );
            }
        }
    }
}
