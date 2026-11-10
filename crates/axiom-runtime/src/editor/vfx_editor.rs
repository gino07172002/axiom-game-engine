//! VFX tab: the over-life parameters `vfx.library` emitters already have. Size is linear from
//! `size` to `end_size` (a two-handle curve), colour a two-stop gradient from `color` to
//! `end_color`. Handles, sliders and the picker edit a draft; it commits once when the
//! pointer is released with no popup or text field open (`ACL: vfx ID size A B` / `vfx ID
//! color RGBA RGBA`), so a drag is one revision.
use super::icons::Icon;
use super::tuning::mark;
use super::{theme, Action, Dock, Editor};
use axiom_core::ops::{self, VfxEdit};
use egui::{pos2, vec2, Color32, Rect, RichText, Sense, Stroke, Ui};
use serde_json::{json, Value};

const CURVE_H: f32 = 120.0;
const BAR_H: f32 = 24.0;
const HANDLE: f32 = 6.0;

/// The editable over-life values of one emitter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverLife {
    pub size: [f64; 2],
    pub color: [[f64; 4]; 2],
}

#[derive(Default)]
pub(crate) struct VfxUi {
    pub selected: Option<String>,
    /// Unsaved values of the selected emitter (committed on release).
    pub draft: Option<(String, OverLife)>,
    /// Gradient stop being edited (0 = start, 1 = end).
    pub stop: usize,
    /// Curve handle being dragged and the y scale fixed for the drag.
    pub drag: Option<(usize, f64)>,
    cache: Option<Model>,
    /// Times the emitter list was rebuilt (tests: not per frame).
    pub builds: usize,
}

struct Emitter {
    id: String,
    values: OverLife,
    summary: String,
}

struct Model {
    key: (i64, u64),
    emitters: Vec<Emitter>,
}

fn num(v: &Value, default: f64) -> f64 {
    v.as_f64().unwrap_or(default)
}

fn rgba(v: &Value, default: [f64; 4]) -> [f64; 4] {
    let mut out = default;
    if let Some(a) = v.as_array() {
        for (o, x) in out.iter_mut().zip(a) {
            *o = x.as_f64().unwrap_or(*o);
        }
    }
    out
}

impl Model {
    fn build(key: (i64, u64), world: &axiom_core::World) -> Self {
        // Defaults of `vfx::Emitter` (fields are optional in authored JSON).
        let d = crate::vfx::Emitter::default();
        let f = |x: f32| f64::from(x);
        let emitters = world
            .resources
            .get(ops::VFX_LIBRARY)
            .and_then(|l| l.get("emitters"))
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(id, e)| Emitter {
                        id: id.clone(),
                        values: OverLife {
                            size: [
                                num(&e["size"], f(d.size)),
                                num(&e["end_size"], f(d.end_size)),
                            ],
                            color: [
                                rgba(&e["color"], d.color.map(f)),
                                rgba(&e["end_color"], d.end_color.map(f)),
                            ],
                        },
                        summary: format!(
                            "{} · {} × {} · {:.1} s",
                            e["backend"].as_str().unwrap_or(&d.backend),
                            e["renderer"].as_str().unwrap_or(&d.renderer),
                            e["count"].as_u64().unwrap_or(u64::from(d.count)),
                            num(&e["lifetime"], f(d.lifetime))
                        ),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { key, emitters }
    }
}

impl VfxUi {
    pub fn observation(&self) -> Value {
        json!({"selected":self.selected,"draft":self.draft.is_some(),"emitters":self.cache.as_ref().map(|m| m.emitters.len())})
    }
}

fn color32(c: [f64; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
        (c[3] * 255.0) as u8,
    )
}

fn lerp(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
}

impl Editor {
    /// One emitter edit (`ACL: vfx ID size|color ..`).
    pub(super) fn vfx_edit(&mut self, emitter: &str, edit: &VfxEdit) -> Option<Action> {
        let result = ops::edit_vfx(&self.world, emitter, edit);
        self.tuning_write(result)
    }

    /// `vfx.select EMITTER` (view only).
    pub(super) fn vfx_select_command(&mut self, arg: Option<&str>) -> anyhow::Result<()> {
        self.show_dock(Dock::Vfx);
        if let Some(id) = arg {
            anyhow::ensure!(
                self.world
                    .resources
                    .get(ops::VFX_LIBRARY)
                    .and_then(|l| l.pointer(&format!("/emitters/{}", axiom_core::local::escape(id))))
                    .is_some(),
                "vfx.select: vfx.library has no emitter {id}"
            );
            self.tuning.vfx.selected = Some(id.to_owned());
            self.tuning.vfx.draft = None;
        }
        Ok(())
    }

    /// The VFX tab body.
    pub(super) fn vfx_tab(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let key = (self.rev, self.world_version);
        if self.tuning.vfx.cache.as_ref().is_none_or(|m| m.key != key) {
            self.tuning.vfx.cache = Some(Model::build(key, &self.world));
            self.tuning.vfx.builds += 1;
            self.tuning.vfx.draft = None;
            self.tuning.vfx.drag = None;
        }
        let model = self.tuning.vfx.cache.take().expect("built above");
        if model.emitters.is_empty() {
            ui.label(RichText::new("This world has no vfx.library emitters.").color(theme::MUTED));
            self.tuning.vfx.cache = Some(model);
            return;
        }
        if self
            .tuning
            .vfx
            .selected
            .as_ref()
            .is_none_or(|s| !model.emitters.iter().any(|e| &e.id == s))
        {
            self.tuning.vfx.selected = Some(model.emitters[0].id.clone());
            self.tuning.vfx.draft = None;
        }
        let icons = self.icons;
        // Emitter list on the left, the selected emitter beside it.
        egui::SidePanel::left("vfx_emitter_list")
            .resizable(false)
            .exact_width(210.0)
            .frame(egui::Frame::new())
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    icons.show(ui, Icon::Vfx, 16.0, theme::ACCENT);
                    ui.label(
                        RichText::new(format!("{} emitters", model.emitters.len()))
                            .color(theme::TEXT),
                    );
                });
                egui::ScrollArea::vertical()
                    .id_salt("vfx_emitters")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for e in &model.emitters {
                            let on = self.tuning.vfx.selected.as_deref() == Some(e.id.as_str());
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                ui.painter().rect_filled(r, 3.0, color32(e.values.color[0]));
                                let row = ui.selectable_label(
                                    on,
                                    RichText::new(&e.id).monospace().size(12.0),
                                );
                                mark!(self.rects, format!("vfx:emitter:{}", e.id), row.rect);
                                if row.clicked() && !on {
                                    self.tuning.vfx.selected = Some(e.id.clone());
                                    self.tuning.vfx.draft = None;
                                    self.tuning.vfx.drag = None;
                                }
                            });
                        }
                    });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 10,
                ..Default::default()
            }))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("vfx_details")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.vfx_details(ui, &model, actions));
            });
        self.tuning.vfx.cache = Some(model);
    }

    fn vfx_details(&mut self, ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
        let id = self
            .tuning
            .vfx
            .selected
            .clone()
            .expect("chosen by the caller");
        let e = model
            .emitters
            .iter()
            .find(|e| e.id == id)
            .expect("chosen from the model");
        let authored = e.values;
        let mut work = match &self.tuning.vfx.draft {
            Some((d, v)) if *d == id => *v,
            _ => authored,
        };
        let editable = !self.previewing;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&e.id).strong().color(theme::TEXT));
            theme::chip(ui, e.summary.as_str(), theme::MUTED);
            if self.tuning.vfx.draft.is_some() {
                theme::chip(ui, "editing…", theme::WARNING);
            }
        });
        ui.add_space(4.0);
        ui.add_enabled_ui(editable, |ui| {
            // Size over life.
            ui.label(
                RichText::new("SIZE OVER LIFE")
                    .size(11.0)
                    .color(theme::MUTED),
            );
            let width = ui.available_width().clamp(220.0, 640.0);
            let (rect, response) =
                ui.allocate_exact_size(vec2(width, CURVE_H), Sense::click_and_drag());
            mark!(self.rects, "vfx:curve".into(), rect);
            let inner = rect.shrink2(vec2(14.0, 10.0));
            let ymax = self.tuning.vfx.drag.map_or_else(
                || (work.size[0].max(work.size[1]) * 1.6).clamp(0.2, 4.0),
                |(_, y)| y,
            );
            let to_screen = |t: f64, v: f64| {
                pos2(
                    inner.left() + t as f32 * inner.width(),
                    inner.bottom() - (v / ymax) as f32 * inner.height(),
                )
            };
            let handles = [to_screen(0.0, work.size[0]), to_screen(1.0, work.size[1])];
            if response.drag_started() && editable {
                if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                    let near = |h: egui::Pos2| {
                        (h.x - p.x).abs() <= 3.0 * HANDLE && (h.y - p.y).abs() <= 3.0 * HANDLE
                    };
                    let which = if near(handles[0]) {
                        Some(0)
                    } else if near(handles[1]) {
                        Some(1)
                    } else {
                        // Anywhere else: the nearer end.
                        Some(usize::from(p.x > inner.center().x))
                    };
                    self.tuning.vfx.drag = which.map(|h| (h, ymax));
                }
            }
            if let (Some((h, ymax)), Some(p)) =
                (self.tuning.vfx.drag, response.interact_pointer_pos())
            {
                if response.dragged() {
                    let v = ((inner.bottom() - p.y) / inner.height()) as f64 * ymax;
                    let (lo, hi) = if h == 0 { (0.005, 4.0) } else { (0.0, 4.0) };
                    work.size[h] = (v.clamp(lo, hi) * 1000.0).round() / 1000.0;
                }
            }
            if response.drag_stopped() {
                self.tuning.vfx.drag = None;
            }
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, theme::RADIUS, theme::FIELD);
            for i in 0..=4 {
                let y = inner.bottom() - inner.height() * i as f32 / 4.0;
                painter.line_segment(
                    [pos2(inner.left(), y), pos2(inner.right(), y)],
                    Stroke::new(1.0_f32, theme::BORDER),
                );
            }
            painter.text(
                pos2(rect.left() + 3.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{ymax:.2} m"),
                egui::FontId::monospace(10.0),
                theme::MUTED,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(inner.left(), inner.bottom()),
                    handles[0],
                    handles[1],
                    pos2(inner.right(), inner.bottom()),
                ],
                theme::ACCENT.gamma_multiply(0.15),
                Stroke::NONE,
            ));
            painter.line_segment(
                [handles[0], handles[1]],
                Stroke::new(2.0_f32, theme::ACCENT),
            );
            // Particles sampled along life: size and colour at 9 points.
            for i in 0..9 {
                let t = i as f64 / 8.0;
                let size = work.size[0] + (work.size[1] - work.size[0]) * t;
                let c = lerp(work.color[0], work.color[1], t);
                let r = ((size / ymax) as f32 * inner.height() * 0.25).clamp(1.5, 16.0);
                painter.circle_filled(
                    pos2(
                        inner.left() + t as f32 * inner.width(),
                        inner.bottom() - 2.0 - r,
                    ),
                    r,
                    color32(c),
                );
            }
            for (i, h) in handles.iter().enumerate() {
                let hot = self.tuning.vfx.drag.is_some_and(|(d, _)| d == i);
                painter.circle(
                    *h,
                    HANDLE,
                    if hot { theme::WARNING } else { theme::TEXT },
                    Stroke::new(1.5_f32, Color32::BLACK),
                );
                mark!(
                    self.rects,
                    format!("vfx:size_handle:{i}"),
                    Rect::from_center_size(*h, vec2(2.0 * HANDLE, 2.0 * HANDLE))
                );
            }
            ui.horizontal(|ui| {
                for (i, label) in ["start", "end"].into_iter().enumerate() {
                    ui.label(RichText::new(label).color(theme::MUTED));
                    let lo = if i == 0 { 0.005 } else { 0.0 };
                    let r = ui.add(
                        egui::DragValue::new(&mut work.size[i])
                            .range(lo..=4.0)
                            .speed(0.005)
                            .max_decimals(3)
                            .suffix(" m"),
                    );
                    mark!(self.rects, format!("vfx:size:{i}"), r.rect);
                }
            });
            ui.add_space(8.0);
            // Colour over life.
            ui.label(
                RichText::new("COLOUR OVER LIFE")
                    .size(11.0)
                    .color(theme::MUTED),
            );
            let (bar, bar_response) =
                ui.allocate_exact_size(vec2(width, BAR_H + 12.0), Sense::click());
            mark!(self.rects, "vfx:gradient".into(), bar);
            let strip = Rect::from_min_size(
                bar.min + vec2(HANDLE, 0.0),
                vec2(bar.width() - 2.0 * HANDLE, BAR_H),
            );
            let painter = ui.painter_at(bar);
            // Checkerboard under the (alpha) gradient.
            let cell = BAR_H / 2.0;
            let mut x = strip.left();
            let mut col = 0;
            while x < strip.right() {
                for row in 0..2 {
                    let shade = if (col + row) % 2 == 0 { 90 } else { 60 };
                    painter.rect_filled(
                        Rect::from_min_size(
                            pos2(x, strip.top() + row as f32 * cell),
                            vec2(cell.min(strip.right() - x), cell),
                        ),
                        0.0,
                        Color32::from_gray(shade),
                    );
                }
                x += cell;
                col += 1;
            }
            let mut mesh = egui::Mesh::default();
            let (a, b) = (color32(work.color[0]), color32(work.color[1]));
            mesh.colored_vertex(strip.left_top(), a);
            mesh.colored_vertex(strip.right_top(), b);
            mesh.colored_vertex(strip.right_bottom(), b);
            mesh.colored_vertex(strip.left_bottom(), a);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
            painter.rect_stroke(
                strip,
                2.0,
                Stroke::new(1.0_f32, theme::BORDER),
                egui::StrokeKind::Inside,
            );
            for (i, x) in [strip.left(), strip.right()].into_iter().enumerate() {
                let y = strip.bottom() + 1.0;
                let selected = self.tuning.vfx.stop == i;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(x, y),
                        pos2(x + HANDLE, y + 10.0),
                        pos2(x - HANDLE, y + 10.0),
                    ],
                    color32(work.color[i]),
                    Stroke::new(
                        if selected { 2.0_f32 } else { 1.0 },
                        if selected { theme::ACCENT } else { theme::TEXT },
                    ),
                ));
                mark!(
                    self.rects,
                    format!("vfx:stop:{i}"),
                    Rect::from_center_size(pos2(x, y + 5.0), vec2(2.0 * HANDLE, 10.0))
                );
            }
            if bar_response.clicked() {
                if let Some(p) = bar_response.interact_pointer_pos() {
                    self.tuning.vfx.stop = usize::from(p.x > strip.center().x);
                }
            }
            ui.horizontal(|ui| {
                let s = self.tuning.vfx.stop;
                ui.label(
                    RichText::new(if s == 0 { "start" } else { "end" })
                        .strong()
                        .color(theme::TEXT),
                );
                let mut c = work.color[s].map(|x| x as f32);
                if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                    work.color[s] = c.map(|x| (f64::from(x) * 1000.0).round() / 1000.0);
                }
                for (ch, name) in ["R", "G", "B", "A"].into_iter().enumerate() {
                    ui.label(RichText::new(name).color(theme::MUTED));
                    let r = ui.add(
                        egui::Slider::new(&mut work.color[s][ch], 0.0..=1.0)
                            .max_decimals(3)
                            .show_value(true),
                    );
                    mark!(self.rects, format!("vfx:channel:{ch}"), r.rect);
                }
            });
        });
        // Draft bookkeeping and the single commit once the interaction is over.
        self.tuning.vfx.draft = (work != authored).then(|| (id.clone(), work));
        let released = ui.input(|i| !i.pointer.any_down())
            && !ui.memory(|m| m.any_popup_open())
            && ui
                .memory(|m| m.focused())
                .is_none_or(|f| egui::TextEdit::load_state(ui.ctx(), f).is_none());
        if released && self.tuning.vfx.drag.is_none() {
            if let Some((id, v)) = self.tuning.vfx.draft.take() {
                if v.size != authored.size {
                    actions.extend(self.vfx_edit(
                        &id,
                        &VfxEdit::Size {
                            start: v.size[0],
                            end: v.size[1],
                        },
                    ));
                }
                if v.color != authored.color {
                    actions.extend(self.vfx_edit(
                        &id,
                        &VfxEdit::Color {
                            start: v.color[0],
                            end: v.color[1],
                        },
                    ));
                }
            }
        }
    }
}
