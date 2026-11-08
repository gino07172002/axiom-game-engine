pub mod simulation;
mod ui;
use crate::{
    presentation::{Instance, RenderFrame},
    ui::{Action, UiState},
    world::{ActionMap, RuntimeRegistry, RuntimeState, RuntimeSystem},
};
use anyhow::Result;
use axiom_core::Transform;
use axiom_fps::{ModelPart, Session, Visuals};
use glam::{Mat4, Quat, Vec3};
use serde_json::Value;
use std::{any::Any, collections::BTreeMap};
pub fn register(registry: &mut RuntimeRegistry) {
    axiom_fps::register(&mut registry.authoring);
    registry.authoring.action(
        "fps.inspect",
        "fps",
        serde_json::json!({"id":"sentinel entity ID; returns state, health, distance to player, cooldown and each outgoing transition with its verdict"}),
        |v| {
            anyhow::ensure!(
                v.get("id").and_then(Value::as_str).is_some(),
                "fps.inspect needs id"
            );
            Ok(())
        },
    );
    registry.factories.insert("fps.simulate".into(), |state| {
        let sim = simulation::Simulation::from_compiled(&state.compiled, state.revision)?;
        let mut module = FpsSystem {
            sim,
            session: state.compiled.source.resource("fps.session")?,
            visuals: state.compiled.source.resource("fps.visuals")?,
        };
        module.sync(state);
        Ok(Box::new(module))
    });
}
struct FpsSystem {
    sim: simulation::Simulation,
    session: Session,
    visuals: Visuals,
}
impl FpsSystem {
    /// Per-sentinel diagnosis: why is it (not) chasing or attacking right now?
    fn inspect(&self, id: &str) -> Result<Value> {
        use axiom_fps::{Condition, State};
        let i = (0..self.sim.actors.positions.len())
            .find(|&i| self.sim.compiled[i + 1].id == id)
            .ok_or_else(|| {
                axiom_core::Diagnostic::error(
                    "not_found",
                    "/args/id",
                    format!("{id} is not a sentinel"),
                )
            })?;
        let state = self.sim.actors.states[i];
        let health = self.sim.actors.health[i];
        let distance = (self.sim.position - self.sim.actors.positions[i]).length();
        let behavior = &self.sim.world.behavior;
        let transitions: Vec<Value> = if health <= 0.0 {
            vec![]
        } else {
            behavior
                .transitions
                .iter()
                .filter(|t| t.from == state)
                .map(|t| {
                    let (when, measured, pass) = match t.when {
                        Condition::DistanceBelow(v) => {
                            (format!("distance<{v}"), distance, distance < v)
                        }
                        Condition::DistanceAbove(v) => {
                            (format!("distance>{v}"), distance, distance > v)
                        }
                        Condition::HealthBelow(v) => (format!("health<{v}"), health, health < v),
                    };
                    serde_json::json!({"to":t.to,"when":when,"measured":measured,"pass":pass})
                })
                .collect()
        };
        let note = match state {
            State::Attack if distance >= 2.5 => {
                Some("in attack state but beyond the 2.5 m strike range")
            }
            State::Attack if self.sim.actors.cooldowns[i] > 0.0 => Some("waiting for cooldown"),
            _ if transitions.is_empty() && health > 0.0 => {
                Some("no outgoing transitions from this state")
            }
            _ => None,
        };
        Ok(serde_json::json!({
            "id":id,"state":state,"health":health,"distance_to_player":distance,
            "cooldown":self.sim.actors.cooldowns[i],"position":self.sim.actors.positions[i].to_array(),
            "transitions":transitions,"note":note
        }))
    }
    fn sync(&mut self, state: &mut RuntimeState) {
        state.set_position(&self.session.player, self.sim.position.to_array());
        state.set_health(&self.session.player, self.sim.health);
        for i in 0..self.sim.actors.positions.len() {
            let id = &self.sim.compiled[i + 1].id;
            state.set_position(id, self.sim.actors.positions[i].to_array());
            state.set_health(id, self.sim.actors.health[i]);
        }
        if let Some(&i) = state.compiled.indices.get(&self.session.camera) {
            state.transforms[i] = Some(Transform {
                position: self.sim.eye().to_array(),
                rotation: (Quat::from_rotation_y(-self.sim.yaw)
                    * Quat::from_rotation_x(self.sim.pitch))
                .to_array(),
                ..Default::default()
            });
        }
    }
    fn model(out: &mut Vec<Instance>, parent: Mat4, parts: &[ModelPart], phase: Option<f32>) {
        for p in parts {
            let mut part = p.part.clone();
            if let Some(phase) = phase {
                if p.name == "left_leg" {
                    part.transform.position[2] -= phase.sin() * 0.2;
                }
                if p.name == "right_leg" {
                    part.transform.position[2] += phase.sin() * 0.2;
                }
            }
            out.push(Instance::part(parent, &part));
        }
    }
}
impl RuntimeSystem for FpsSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, input: &ActionMap) {
        self.sim.step(simulation::Input {
            forward: input.axis("move.forward"),
            right: input.axis("move.right"),
            sprint: input.pressed("sprint"),
            jump: input.pressed("jump"),
            fire: input.pressed("fire"),
            reload: input.pressed("reload"),
        });
        self.sync(state);
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        if let Some(old) = old.as_any().downcast_ref::<Self>() {
            self.sim.restore_from(&old.sim);
            self.sync(next);
        }
        Ok(())
    }
    fn command(
        &mut self,
        _state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "fps" {
            return None;
        }
        Some(match action {
            "inspect" => args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("fps.inspect needs id"))
                .and_then(|id| self.inspect(id)),
            other => Err(anyhow::anyhow!("unknown fps action {other}")),
        })
    }
    fn pointer(&mut self, dx: f32, dy: f32) {
        self.sim.look(dx, dy);
    }
    fn pointer_allowed(&self) -> bool {
        self.sim.health > 0.0 && !self.sim.victory()
    }
    fn draw_ui(
        &self,
        ctx: &egui::Context,
        _state: &RuntimeState,
        ui: &mut UiState,
    ) -> Option<Vec<Action>> {
        Some(ui::draw(ctx, &self.sim, ui))
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        let mut map = serde_json::to_value(self.sim.observation())
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        for key in [
            "rev",
            "tick",
            "elapsed_seconds",
            "frame_ms_cpu",
            "fps",
            "gpu",
        ] {
            map.remove(key);
        }
        map.insert("fps_revision".into(), serde_json::json!(self.sim.rev));
        map.into_iter().collect()
    }
    fn extract(&self, _state: &RuntimeState, frame: &mut RenderFrame) {
        for i in 0..self.sim.actors.positions.len() {
            let p = self.sim.actors.positions[i];
            let target = self.sim.position - p;
            let q = Quat::from_rotation_y(target.x.atan2(target.z));
            if self.sim.actors.health[i] <= 0.0 {
                Self::model(
                    &mut frame.instances,
                    Mat4::from_rotation_translation(q, Vec3::new(p.x, 0.0, p.z)),
                    &self.visuals.corpse,
                    None,
                );
                continue;
            }
            let walking = self.sim.actors.states[i] == axiom_fps::State::Chase;
            let phase = self.sim.tick as f32 * 0.07 + i as f32;
            let bob = if walking { phase.sin() * 0.035 } else { 0.0 };
            Self::model(
                &mut frame.instances,
                Mat4::from_rotation_translation(q, p + Vec3::Y * bob),
                &self.visuals.sentinel,
                walking.then_some(phase),
            );
        }
        for (start, end, _) in &self.sim.tracers {
            let d = *end - *start;
            if d.length_squared() > 0.000001 {
                let q = Quat::from_rotation_arc(Vec3::Z, d.normalize());
                frame.instances.push(Instance::new(
                    Mat4::from_scale_rotation_translation(
                        Vec3::new(0.012, 0.012, d.length()),
                        q,
                        (*start + *end) * 0.5,
                    ),
                    [1.0, 0.72, 0.24],
                    4.0,
                    false,
                ));
            }
        }
        if let Some(view) = frame.views.iter_mut().find(|v| v.id == self.session.camera) {
            // The content camera follows exactly the same FPS aim direction as hitscan.
            view.eye = self.sim.eye();
            view.forward = self.sim.forward();
            view.up = Vec3::Y;
            let forward = self.sim.forward();
            let right = forward.cross(Vec3::Y).normalize();
            let up = right.cross(forward);
            let q = Quat::from_mat4(&Mat4::look_to_rh(Vec3::ZERO, forward, Vec3::Y).inverse());
            let reload = if self.sim.reload_left > 0.0 {
                (self.sim.reload_left / self.sim.world.weapon.reload_seconds * std::f32::consts::PI)
                    .sin()
                    * 0.35
            } else {
                0.0
            };
            let gun = self.sim.eye() + forward * (0.60 - self.sim.muzzle) + right * 0.26
                - up * (0.23 + reload);
            Self::model(
                &mut view.instances,
                Mat4::from_rotation_translation(q, gun),
                &self.visuals.rifle,
                None,
            );
            if self.sim.muzzle > 0.0 {
                view.instances.push(Instance::new(
                    Mat4::from_scale_rotation_translation(
                        Vec3::new(0.13, 0.13, 0.15),
                        q,
                        gun + forward * 0.48,
                    ),
                    [1.0, 0.70, 0.25],
                    5.0,
                    false,
                ));
            }
        }
    }
}
