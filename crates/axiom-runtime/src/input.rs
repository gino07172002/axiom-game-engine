//! Generic player-control contexts. All scheduled simulations continue to run.
use crate::{ui::Action, world::RuntimeRegistry};
use anyhow::{ensure, Context, Result};
use axiom_core::{gameplay::RuntimeCommand, valid_id, World};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ContextDef {
    label: String,
    systems: BTreeSet<String>,
    cameras: BTreeSet<String>,
    capture_pointer: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Config {
    initial: String,
    contexts: BTreeMap<String, ContextDef>,
}
#[derive(Clone)]
pub struct Contexts {
    config: Config,
    active: String,
    held: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Activate {
    context: String,
}
pub fn register(registry: &mut RuntimeRegistry) {
    let r = &mut registry.authoring;
    r.capabilities.insert("input".into(), 1);
    r.resource("input.contexts","input",json!({
        "initial":"context ID","contexts":"ID -> {label,systems:[system IDs],cameras:[camera entity IDs],capture_pointer:bool}",
        "switch_action":"context.next; bind a button to cycle"
    }),|v|{
        let c:Config=serde_json::from_value(v.clone())?;
        ensure!((1..=8).contains(&c.contexts.len()) && c.contexts.contains_key(&c.initial),"invalid input contexts");
        for (id,context) in &c.contexts {
            valid_id(id)?;
            ensure!(!context.label.trim().is_empty() && context.label.len()<=64
                && (1..=8).contains(&context.systems.len()) && (1..=8).contains(&context.cameras.len()),"context budget exceeded");
        }
        Ok(())
    });
    r.action(
        "input.activate",
        "input",
        json!({"context":"input context ID"}),
        |v| {
            let a: Activate = serde_json::from_value(v.clone())?;
            valid_id(&a.context)
        },
    );
    r.world_validators.push(|world| {
        let Some(value) = world.resources.get("input.contexts") else {
            return Ok(());
        };
        let config: Config = serde_json::from_value(value.clone())?;
        for context in config.contexts.values() {
            for system in &context.systems {
                ensure!(
                    world.systems.contains(system),
                    "context system is not scheduled: {system}"
                );
            }
            for id in &context.cameras {
                let entity = world.entity(id).context("context camera missing")?;
                let values = world.resolved_components(entity);
                let camera: axiom_core::Camera = serde_json::from_value(
                    values
                        .get("render.camera")
                        .context("context camera component missing")?
                        .clone(),
                )?;
                ensure!(camera.active, "context camera must be authored active");
            }
        }
        Ok(())
    });
}
impl Contexts {
    pub fn new(world: &World) -> Result<Option<Self>> {
        world
            .resources
            .get("input.contexts")
            .map(|value| {
                let config: Config = serde_json::from_value(value.clone())?;
                Ok(Self {
                    active: config.initial.clone(),
                    config,
                    held: false,
                })
            })
            .transpose()
    }
    pub fn activate(&mut self, args: &Value) -> Result<()> {
        let a: Activate = serde_json::from_value(args.clone())?;
        ensure!(
            self.config.contexts.contains_key(&a.context),
            "unknown input context"
        );
        self.active = a.context;
        Ok(())
    }
    pub fn step(&mut self, pressed: bool) {
        if pressed && !self.held {
            let ids = self.config.contexts.keys().cloned().collect::<Vec<_>>();
            let current = ids.iter().position(|id| *id == self.active).unwrap();
            self.active = ids[(current + 1) % ids.len()].clone();
        }
        self.held = pressed;
    }
    pub fn routes(&self, system: &str) -> bool {
        !self
            .config
            .contexts
            .values()
            .any(|c| c.systems.contains(system))
            || self.config.contexts[&self.active].systems.contains(system)
    }
    pub fn camera(&self, id: &str) -> bool {
        self.config.contexts[&self.active].cameras.contains(id)
    }
    pub fn capture(&self) -> bool {
        self.config.contexts[&self.active].capture_pointer
    }
    pub fn transfer(&mut self, old: &Self) {
        if self.config == old.config {
            self.active = old.active.clone();
            self.held = old.held;
        }
    }
    pub fn observation(&self) -> Value {
        json!({"context":self.active,"capture_pointer":self.capture()})
    }
    pub fn draw(&self, ctx: &egui::Context, actions: &mut Vec<Action>, next_key: Option<&str>) {
        egui::TopBottomPanel::top("input_contexts").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("CONTROL");
                for (id, context) in &self.config.contexts {
                    if ui
                        .selectable_label(id == &self.active, &context.label)
                        .clicked()
                    {
                        actions.push(Action::Gameplay(RuntimeCommand::Call {
                            capability: "input".into(),
                            action: "activate".into(),
                            args: json!({"context":id}),
                        }));
                    }
                }
                if let Some(key) = next_key {
                    ui.label(format!(
                        "{} · switch control",
                        key.strip_prefix("Key").unwrap_or(key)
                    ));
                }
            });
        });
    }
}
