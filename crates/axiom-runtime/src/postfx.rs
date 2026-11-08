//! Scene-color and depth effects, bounded by authored world-space volumes.
use crate::{presentation::RenderFrame, world::*};
use anyhow::{ensure, Result};
use axiom_core::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::any::Any;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Volume {
    pub id: String,
    pub kind: String,
    pub center: [f32; 3],
    pub half_size: [f32; 3],
    pub strength: f32,
    pub color: [f32; 4],
}
impl Volume {
    pub fn kind_id(&self) -> f32 {
        match self.kind.as_str() {
            "heat" => 1.0,
            "fog" => 2.0,
            "decal" => 3.0,
            _ => 0.0,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub volumes: Vec<Volume>,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.volumes.len() <= 8,
            "post effect volume budget exceeded"
        );
        let mut ids = std::collections::BTreeSet::new();
        for v in &self.volumes {
            valid_id(&v.id)?;
            ensure!(ids.insert(&v.id), "duplicate post effect volume ID");
            ensure!(
                ["chromatic", "heat", "fog", "decal"].contains(&v.kind.as_str())
                    && v.center.iter().all(|x| x.is_finite() && x.abs() <= 1000.0)
                    && v.half_size
                        .iter()
                        .all(|x| x.is_finite() && (0.01..=100.0).contains(x))
                    && v.strength.is_finite()
                    && (0.0..=4.0).contains(&v.strength)
                    && v.color
                        .iter()
                        .all(|x| x.is_finite() && (0.0..=1.0).contains(x)),
                "invalid post effect volume {}",
                v.id
            );
        }
        Ok(())
    }
}
struct PostSystem(Settings);
impl RuntimeSystem for PostSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn checkpoint_supported(&self) -> bool {
        true
    }
    fn step(&mut self, _state: &mut RuntimeState, _input: &ActionMap) {}
    fn extract(&self, _state: &RuntimeState, frame: &mut RenderFrame) {
        frame.postfx = self.0.volumes.clone();
    }
}
pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.capabilities.insert("postfx".into(), 1);
    r.authoring.resource("postfx.settings","postfx",json!({"volumes":"up to 8 {id,kind:chromatic|heat|fog|decal,center,half_size,strength,color}"}),|v|serde_json::from_value::<Settings>(v.clone())?.validate());
    r.authoring.systems.insert(
        "postfx.prepare".into(),
        SystemDescriptor {
            id: "postfx.prepare".into(),
            capability: "postfx".into(),
            phase: 95,
            reads: vec!["postfx.settings".into()],
            writes: vec![],
            after: vec![],
        },
    );
    r.factories.insert("postfx.prepare".into(), |state| {
        Ok(Box::new(PostSystem(
            state.compiled.source.resource("postfx.settings")?,
        )))
    });
}
