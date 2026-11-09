//! In-world name tags: `ui.nameplate {text, height?, color?}` draws a label above an entity,
//! projected through the active game camera. Generic (players, NPCs, signs), data-only.
use crate::{
    presentation::RenderView,
    ui::{Action, UiState},
    world::*,
};
use anyhow::{ensure, Result};
use axiom_core::*;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::any::Any;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nameplate {
    pub text: String,
    /// Height above the entity origin in meters.
    #[serde(default = "default_height")]
    pub height: f32,
    #[serde(default = "default_color")]
    pub color: [f32; 3],
}
fn default_height() -> f32 {
    1.6
}
fn default_color() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}
impl Nameplate {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.text.is_empty() && self.text.chars().count() <= 48,
            "nameplate text must be 1..48 characters"
        );
        ensure!(
            self.height.is_finite()
                && (-10.0..=50.0).contains(&self.height)
                && self
                    .color
                    .iter()
                    .all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
            "nameplate height -10..50, color 0..1"
        );
        Ok(())
    }
}

struct NameplateSystem {
    plates: Vec<(usize, Nameplate)>,
}

impl NameplateSystem {
    fn view(state: &RuntimeState) -> Option<RenderView> {
        active_view(state)
    }
}
/// The first active camera, reconstructed from its live transform (for screen-space overlays).
pub(crate) fn active_view(state: &RuntimeState) -> Option<RenderView> {
    {
        let matrices = state.matrices();
        state
            .compiled
            .entities
            .iter()
            .enumerate()
            .find(|(i, e)| {
                state.compiled.enabled[*i] && e.camera.as_ref().is_some_and(|c| c.active)
            })
            .map(|(i, e)| {
                let m = matrices[i];
                RenderView {
                    id: e.id.clone(),
                    eye: m.transform_point3(Vec3::ZERO),
                    forward: m.transform_vector3(-Vec3::Z).normalize_or_zero(),
                    up: m.transform_vector3(Vec3::Y).normalize_or_zero(),
                    camera: e.camera.clone().unwrap(),
                    instances: vec![],
                }
            })
    }
}

impl RuntimeSystem for NameplateSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn checkpoint_supported(&self) -> bool {
        true
    }
    fn step(&mut self, _state: &mut RuntimeState, _input: &ActionMap) {}
    fn draw_ui(
        &self,
        ctx: &egui::Context,
        state: &RuntimeState,
        _ui: &mut UiState,
    ) -> Option<Vec<Action>> {
        let view = Self::view(state)?;
        let screen = ctx.screen_rect();
        let bounds = [screen.min.x, screen.min.y, screen.width(), screen.height()];
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("nameplates"),
        ));
        for (slot, plate) in &self.plates {
            if !state.compiled.enabled[*slot] {
                continue;
            }
            let Some(t) = state.transforms[*slot].as_ref() else {
                continue;
            };
            let p = Vec3::from_array(t.position) + Vec3::Y * plate.height;
            // Skip far labels so crowds stay readable.
            if (p - view.eye).length() > 40.0 {
                continue;
            }
            if let Some([x, y]) = view.project(p, bounds) {
                paint(&painter, egui::pos2(x, y), &plate.text, plate.color);
            }
        }
        // Painting only: the generic bar and bound ui.document still draw.
        None
    }
}

/// One name tag centered at `at` (the game overlay and the editor's UI canvas preview).
/// Returns its rectangle.
pub fn paint(painter: &egui::Painter, at: egui::Pos2, text: &str, color: [f32; 3]) -> egui::Rect {
    let [r, g, b] = color.map(|c| (c * 255.0) as u8);
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(14.0),
        egui::Color32::from_rgb(r, g, b),
    );
    let rect = egui::Rect::from_center_size(at, galley.size() + egui::vec2(10.0, 4.0));
    painter.rect_filled(rect, 4.0, egui::Color32::from_black_alpha(140));
    painter.galley(
        rect.min + egui::vec2(5.0, 2.0),
        galley,
        egui::Color32::WHITE,
    );
    rect
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.component(
        "ui.nameplate",
        "ui",
        json!({"text":"1..48 chars","height":"meters above origin, default 1.6","color":"[r,g,b] 0..1"}),
        |v| serde_json::from_value::<Nameplate>(v.clone())?.validate(),
    );
    r.authoring.systems.insert(
        "ui.nameplates".into(),
        SystemDescriptor {
            id: "ui.nameplates".into(),
            capability: "ui".into(),
            phase: 97,
            reads: vec!["ui.nameplate".into(), "core.transform".into()],
            writes: vec![],
            after: vec![],
        },
    );
    r.factories.insert("ui.nameplates".into(), |state| {
        let mut plates = vec![];
        for (slot, e) in state.compiled.entities.iter().enumerate() {
            if let Some(v) = e.components.get("ui.nameplate") {
                ensure!(
                    state.transforms[slot].is_some(),
                    "{} has a nameplate but no core.transform",
                    e.id
                );
                plates.push((slot, serde_json::from_value(v.clone())?));
            }
        }
        Ok(Box::new(NameplateSystem { plates }) as Box<dyn RuntimeSystem>)
    });
}
