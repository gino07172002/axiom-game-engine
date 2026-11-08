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
    marker: Option<[f32; 2]>,
}
pub fn register(registry: &mut RuntimeRegistry) {
    axiom_rts::register(&mut registry.authoring);
    registry.factories.insert("rts.simulate".into(), |state| {
        let sim = Simulation::new(&state.compiled)?;
        let mut templates = BTreeMap::new();
        let mut definitions = BTreeMap::new();
        for recipe in sim.session.recipes.values() {
            if templates.contains_key(&recipe.prefab) {
                continue;
            }
            let mut world = state.compiled.source.clone();
            let id = format!("__template/{}", recipe.prefab);
            ensure!(
                !world.entities.iter().any(|e| e.id == id),
                "reserved runtime template ID collision"
            );
            let mut entity = Entity::new(&id);
            entity.prefab = Some(recipe.prefab.clone());
            world.entities.push(entity);
            // The source package and prefab contracts were validated before this derived compilation.
            let mut types = state.registry.clone();
            types.world_validators.clear();
            let compiled = world.compile(&types)?;
            let template = compiled.entities[compiled.indices[&id]].clone();
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
            for (metric, i) in &self.bindings {
                use axiom_core::gameplay::Scalar;
                let value = match metric.as_str() {
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
            if let Some(part) = template
                .mesh
                .as_mut()
                .and_then(|mesh| mesh.parts.first_mut())
            {
                part.color = [0.9, 0.26, 0.18];
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
                self.marker = Some(args.destination);
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
            let mut prepared = state.clone();
            let result = produced
                .iter()
                .try_for_each(|id| self.spawn(&mut prepared, id));
            match result {
                Ok(()) => {
                    *state = prepared;
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
    fn screen_pointer(&mut self, state: &mut RuntimeState, pointer: &ScreenPointer) -> Result<()> {
        self.sim
            .set_combat(state.combat.as_ref().map(|c| c.scene.clone()));
        let Some(p) = Self::terrain(pointer) else {
            return Ok(());
        };
        match pointer.kind {
            PointerKind::PrimaryDown => {
                self.drag = Some((pointer.screen, p, pointer.screen, pointer.additive))
            }
            PointerKind::Move => {
                if let Some(drag) = &mut self.drag {
                    drag.2 = pointer.screen;
                }
            }
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
                    })?;
                } else {
                    self.sim.move_group(MoveArgs {
                        units: None,
                        destination: p,
                    })?;
                    self.marker = Some(p);
                }
            }
        }
        self.sync(state);
        Ok(())
    }
    fn extract(&self, _state: &RuntimeState, frame: &mut RenderFrame) {
        for &i in &self.sim.selected {
            let u = &self.sim.units[i];
            if u.health <= 0.0 {
                continue;
            }
            for (x, z, sx, sz) in [
                (-0.5, 0.0, 0.06, 1.0),
                (0.5, 0.0, 0.06, 1.0),
                (0.0, -0.5, 1.0, 0.06),
                (0.0, 0.5, 1.0, 0.06),
            ] {
                frame.instances.push(Instance::new(
                    Mat4::from_scale_rotation_translation(
                        Vec3::new(sx, 0.03, sz),
                        Quat::IDENTITY,
                        Vec3::new(u.position[0] + x, 0.02, u.position[1] + z),
                    ),
                    [0.55, 1.0, 0.65],
                    0.5,
                    false,
                ));
            }
        }
        if let Some(p) = self.marker {
            frame.instances.push(Instance::new(
                Mat4::from_scale_rotation_translation(
                    Vec3::new(0.6, 0.05, 0.6),
                    Quat::IDENTITY,
                    Vec3::new(p[0], 0.06, p[1]),
                ),
                [1.0, 0.8, 0.2],
                0.5,
                false,
            ));
        }
    }
    fn draw_ui(
        &self,
        ctx: &egui::Context,
        state: &RuntimeState,
        ui: &mut UiState,
    ) -> Option<Vec<Action>> {
        let actions = crate::ui::draw_generic(ctx, state, ui);
        if let Some((start, _, end, _)) = self.drag {
            let screen = ctx.screen_rect();
            let a = egui::pos2(start[0] * screen.width(), start[1] * screen.height());
            let b = egui::pos2(end[0] * screen.width(), end[1] * screen.height());
            let rect = egui::Rect::from_two_pos(a, b);
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("rts_drag"),
            ))
            .rect_stroke(
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
