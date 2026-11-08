//! A native extension written outside `axiom-runtime`, using only public APIs.
//! It adds a `spin` capability: entities with `spin.rate` rotate about Y every fixed step.
use anyhow::{ensure, Result};
use axiom_core::SystemDescriptor;
use axiom_runtime::world::{
    ActionMap, Plugin, RuntimeRegistry, RuntimeState, RuntimeSystem, FIXED_DT,
};
use glam::Quat;
use serde_json::{json, Value};
use std::any::Any;

pub struct SpinPlugin;

struct Spin {
    /// (entity slot, radians per second): dense, built once per activation.
    spinners: Vec<(usize, f32)>,
}

fn rate(v: &Value) -> Result<f32> {
    let r = v
        .get("degrees_per_second")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow::anyhow!("spin.rate needs degrees_per_second"))?;
    ensure!(
        r.is_finite() && r.abs() <= 3600.0,
        "degrees_per_second must be -3600..3600"
    );
    Ok(r as f32)
}

impl RuntimeSystem for Spin {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn checkpoint_supported(&self) -> bool {
        // Stateless: everything lives in transforms, which checkpoints already store.
        true
    }
    fn step(&mut self, state: &mut RuntimeState, _input: &ActionMap) {
        for &(slot, radians) in &self.spinners {
            if !state.compiled.enabled[slot] {
                continue;
            }
            if let Some(t) = state.transforms[slot].as_mut() {
                let q = Quat::from_rotation_y(radians * FIXED_DT) * Quat::from_array(t.rotation);
                t.rotation = q.normalize().to_array();
            }
        }
    }
}

impl Plugin for SpinPlugin {
    fn id(&self) -> &str {
        "example.spin"
    }
    fn register(&self, r: &mut RuntimeRegistry) {
        r.authoring.capabilities.insert("spin".into(), 1);
        r.authoring.component(
            "spin.rate",
            "spin",
            json!({"degrees_per_second":"-3600..3600"}),
            |v| rate(v).map(|_| ()),
        );
        r.authoring.systems.insert(
            "spin.step".into(),
            SystemDescriptor {
                id: "spin.step".into(),
                capability: "spin".into(),
                phase: 9,
                reads: vec!["spin.rate".into()],
                writes: vec!["core.transform".into()],
                after: vec![],
            },
        );
        r.factories.insert("spin.step".into(), |state| {
            let mut spinners = vec![];
            for (slot, e) in state.compiled.entities.iter().enumerate() {
                if let Some(v) = e.components.get("spin.rate") {
                    ensure!(
                        e.transform.is_some(),
                        "{} spins without core.transform",
                        e.id
                    );
                    spinners.push((slot, rate(v)?.to_radians()));
                }
            }
            Ok(Box::new(Spin { spinners }) as Box<dyn RuntimeSystem>)
        });
    }
}
