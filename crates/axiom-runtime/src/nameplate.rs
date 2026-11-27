//! In-world name tags: `ui.nameplate {text | key, height?, color?, class?, style?}` draws a
//! label above an entity, projected through the active game camera. Generic (players, NPCs,
//! signs), data-only. `key` reads the label from the `ui.document` catalogs; `class` /
//! `style` (or `theme.nameplate`'s defaults) take a theme text class and panel style; plates
//! fade and hide by `theme.nameplate` distances (plan N1).
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
    /// Fixed label; with `key` it is only the fallback when no catalog has the key.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Catalog key of the label (follows the locale).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Theme text class (`ui.document.theme.text`); default `theme.nameplate.class`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// Theme panel style (`ui.document.theme.panel`); default `theme.nameplate.style`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
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
            self.text.chars().count() <= 48,
            "nameplate text must be at most 48 characters"
        );
        ensure!(
            !self.text.is_empty() || self.key.is_some(),
            "nameplate needs text or key"
        );
        for (field, v) in [
            ("key", &self.key),
            ("class", &self.class),
            ("style", &self.style),
        ] {
            if let Some(v) = v {
                ensure!(
                    !v.is_empty() && v.len() <= 64,
                    "nameplate {field} must be 1..64 characters"
                );
            }
        }
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
        ui: &mut UiState,
    ) -> Option<Vec<Action>> {
        let view = Self::view(state)?;
        let screen = ctx.screen_rect();
        let bounds = [screen.min.x, screen.min.y, screen.width(), screen.height()];
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("nameplates"),
        ));
        let g = state.gameplay.as_ref();
        let default_theme = axiom_core::ui_theme::NameplateTheme::default();
        let theme = g
            .and_then(|g| g.compiled.ui.as_ref())
            .and_then(|d| d.theme.as_ref())
            .and_then(|t| t.nameplate.as_ref())
            .unwrap_or(&default_theme);
        let fonts_ready = g.is_none_or(|g| crate::bound_ui::ensure_fonts(ctx, g));
        for (slot, plate) in &self.plates {
            if !state.compiled.enabled[*slot] {
                continue;
            }
            let Some(t) = state.transforms[*slot].as_ref() else {
                continue;
            };
            let p = Vec3::from_array(t.position) + Vec3::Y * plate.height;
            // Fade and skip far labels so crowds stay readable.
            let opacity = theme.opacity((p - view.eye).length());
            if opacity <= 0.0 {
                continue;
            }
            if let Some([x, y]) = view.project(p, bounds) {
                let look = look(ctx, g, ui, fonts_ready, plate);
                paint(&painter, egui::pos2(x, y), &label(g, plate), &look, opacity);
            }
        }
        // Painting only: the generic bar and bound ui.document still draw.
        None
    }
}

/// The label: the catalog text of `key` in the current locale, else `text` (else the key).
pub fn label<'a>(
    g: Option<&crate::gameplay::Gameplay>,
    plate: &'a Nameplate,
) -> std::borrow::Cow<'a, str> {
    use std::borrow::Cow;
    match (&plate.key, g) {
        (Some(key), Some(g)) => {
            let text = g.text(key);
            if &text == key && !plate.text.is_empty() {
                Cow::Borrowed(&plate.text)
            } else {
                Cow::Owned(text)
            }
        }
        (Some(key), None) if plate.text.is_empty() => Cow::Borrowed(key),
        _ => Cow::Borrowed(&plate.text),
    }
}

/// How one plate is drawn. Without a class or style: the original 14 px label on a dark
/// rounded box.
pub struct Look {
    size: f32,
    family: egui::FontFamily,
    color: egui::Color32,
    shadow: Option<egui::Color32>,
    fill: Option<egui::Color32>,
    corner: f32,
    pad: egui::Vec2,
    frame: Option<(
        egui::TextureId,
        [usize; 2],
        [u16; 4],
        axiom_core::ui_theme::FrameCenter,
    )>,
}

/// The look of `plate`: its class / style, else `theme.nameplate`'s, else the default.
pub fn look(
    ctx: &egui::Context,
    g: Option<&crate::gameplay::Gameplay>,
    state: &mut UiState,
    fonts_ready: bool,
    plate: &Nameplate,
) -> Look {
    let rgba = |[r, g, b, a]: [u8; 4]| egui::Color32::from_rgba_unmultiplied(r, g, b, a);
    let [r, gr, b] = plate.color.map(|c| (c * 255.0) as u8);
    let mut look = Look {
        size: 14.0,
        family: egui::FontFamily::Proportional,
        color: egui::Color32::from_rgb(r, gr, b),
        shadow: None,
        fill: Some(egui::Color32::from_black_alpha(140)),
        corner: 4.0,
        pad: egui::vec2(5.0, 2.0),
        frame: None,
    };
    let Some(g) = g else {
        return look;
    };
    let Some(theme) = g.compiled.ui.as_ref().and_then(|d| d.theme.as_ref()) else {
        return look;
    };
    let defaults = theme.nameplate.as_ref();
    let class = plate
        .class
        .as_ref()
        .or_else(|| defaults.and_then(|d| d.class.as_ref()))
        .and_then(|c| theme.text.get(c));
    if let Some(class) = class {
        look.size = class.size.unwrap_or(look.size);
        look.family = crate::bound_ui::font_family(g, fonts_ready, &class.font);
        if let Some(c) = class.color {
            look.color = rgba(c);
        }
        look.shadow = class.shadow.map(rgba);
    }
    let style = plate
        .style
        .as_ref()
        .or_else(|| defaults.and_then(|d| d.style.as_ref()))
        .and_then(|s| theme.panel.get(s));
    if let Some(style) = style {
        look.fill = style.fill.map(rgba);
        look.corner = style.corner.unwrap_or(0.0);
        if let Some(p) = style.padding {
            look.pad = egui::vec2(p, p);
        }
        look.frame = style
            .frame
            .as_ref()
            .and_then(|f| crate::bound_ui::frame_texture(ctx, g, state, f))
            .map(|(f, tex, size)| (tex, size, f.slice, f.center));
    }
    look
}

/// The size of a plate (label plus padding) without painting it.
pub fn measure(painter: &egui::Painter, text: &str, look: &Look) -> egui::Vec2 {
    painter
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::new(look.size, look.family.clone()),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        + look.pad * 2.0
}

/// One name tag centered at `at` with `opacity` 0..1 (the game overlay and the editor's UI
/// canvas preview). Returns its rectangle.
pub fn paint(
    painter: &egui::Painter,
    at: egui::Pos2,
    text: &str,
    look: &Look,
    opacity: f32,
) -> egui::Rect {
    let fade = |c: egui::Color32| {
        if opacity >= 1.0 {
            c
        } else {
            c.gamma_multiply(opacity)
        }
    };
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::new(look.size, look.family.clone()),
        egui::Color32::PLACEHOLDER,
    );
    let rect = egui::Rect::from_center_size(at, galley.size() + look.pad * 2.0);
    if let Some(fill) = look.fill {
        painter.rect_filled(rect, look.corner, fade(fill));
    }
    if let Some((tex, size, slice, center)) = look.frame {
        let mut mesh = crate::bound_ui::nine_slice(rect, tex, size, slice, center);
        if opacity < 1.0 {
            for v in &mut mesh.vertices {
                v.color = fade(v.color);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    let at = rect.min + look.pad;
    if let Some(shadow) = look.shadow {
        painter.galley(at + egui::vec2(2.0, 2.0), galley.clone(), fade(shadow));
    }
    painter.galley(at, galley, fade(look.color));
    rect
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.component(
        "ui.nameplate",
        "ui",
        json!({"text":"0..48 chars (the fallback with key)","key":"catalog key of the label (follows the locale)","height":"meters above origin, default 1.6","color":"[r,g,b] 0..1 (a class colour wins)","class":"ui.document theme.text class (default theme.nameplate.class)","style":"ui.document theme.panel style: fill, corner, padding, frame (default theme.nameplate.style)"}),
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
                let plate: Nameplate = serde_json::from_value(v.clone())?;
                plate
                    .validate()
                    .map_err(|err| err.context(format!("{} ui.nameplate", e.id)))?;
                plates.push((slot, plate));
            }
        }
        Ok(Box::new(NameplateSystem { plates }) as Box<dyn RuntimeSystem>)
    });
}
