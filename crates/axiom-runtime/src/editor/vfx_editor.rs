//! VFX tab: the over-life parameters of `vfx.library` emitters. Size over life is either the
//! two-point `size` -> `end_size` or the optional `size_curve` (2..=8 keys); colour over life
//! the two-stop `color` -> `end_color` or the optional `color_gradient` (2..=8 stops).
//!
//! The curve and the gradient bar show every key. Drag a key (vertically for its size;
//! interior keys also along life), double-click an empty spot to add one (up to 8), select one
//! and press "Remove" to drop it; the same for gradient stops. Handles, sliders and the
//! picker edit a draft that commits once when the pointer is released with no popup or text
//! field open, so a drag is one revision. A two-point emitter whose keys stay at life 0 and 1
//! keeps writing `size` / `end_size` (`ACL: vfx ID size A B` / `vfx ID color RGBA RGBA`);
//! anything else writes the whole list (`vfx ID size_curve T:S ..` / `vfx ID gradient
//! T:RGBA ..`). "Two-point" removes the list again (`.. none`).
use super::icons::Icon;
use super::tuning::mark;
use super::{theme, Action, Dock, Editor};
use axiom_core::ops::{self, VfxEdit, MAX_VFX_CURVE_KEYS};
use egui::{pos2, vec2, Color32, Rect, RichText, Sense, Stroke, Ui};
use serde_json::{json, Value};

const CURVE_H: f32 = 120.0;
const BAR_H: f32 = 24.0;
const HANDLE: f32 = 6.0;
/// Closest two keys may come along life (keeps the times strictly increasing).
const MIN_GAP: f64 = 0.01;

/// The editable over-life values of one emitter.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OverLife {
    /// `[t, size]` keys sorted by `t` (two-point emitters: `[0, size]`, `[1, end_size]`).
    pub size: Vec<[f64; 2]>,
    /// `(t, rgba)` stops sorted by `t`.
    pub color: Vec<(f64, [f64; 4])>,
    /// The emitter authors `size_curve` / `color_gradient` (else the two-point fields).
    pub size_curve: bool,
    pub color_curve: bool,
}

#[derive(Default)]
pub(crate) struct VfxUi {
    pub selected: Option<String>,
    /// Unsaved values of the selected emitter (committed on release).
    pub draft: Option<(String, OverLife)>,
    /// Gradient stop being edited.
    pub stop: usize,
    /// Size key being edited.
    pub key: usize,
    /// Curve key being dragged and the y scale fixed for the drag.
    pub drag: Option<(usize, f64)>,
    /// Gradient stop being dragged along life.
    pub stop_drag: Option<usize>,
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
                    .map(|(id, e)| {
                        let curve = e["size_curve"].as_array().map(|keys| {
                            keys.iter()
                                .map(|k| [num(&k["t"], 0.0), num(&k["size"], 0.0)])
                                .collect::<Vec<_>>()
                        });
                        let gradient = e["color_gradient"].as_array().map(|stops| {
                            stops
                                .iter()
                                .map(|s| (num(&s["t"], 0.0), rgba(&s["color"], [0.0; 4])))
                                .collect::<Vec<_>>()
                        });
                        Emitter {
                            id: id.clone(),
                            values: OverLife {
                                size_curve: curve.is_some(),
                                color_curve: gradient.is_some(),
                                size: curve.unwrap_or_else(|| {
                                    vec![
                                        [0.0, num(&e["size"], f(d.size))],
                                        [1.0, num(&e["end_size"], f(d.end_size))],
                                    ]
                                }),
                                color: gradient.unwrap_or_else(|| {
                                    vec![
                                        (0.0, rgba(&e["color"], d.color.map(f))),
                                        (1.0, rgba(&e["end_color"], d.end_color.map(f))),
                                    ]
                                }),
                            },
                            summary: format!(
                                "{} · {} × {} · {:.1} s",
                                e["backend"].as_str().unwrap_or(&d.backend),
                                e["renderer"].as_str().unwrap_or(&d.renderer),
                                e["count"].as_u64().unwrap_or(u64::from(d.count)),
                                num(&e["lifetime"], f(d.lifetime))
                            ),
                        }
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

/// Piecewise-linear value at `t` (held outside the keys), as the runtime evaluates it.
fn sample<const N: usize>(keys: impl Iterator<Item = (f64, [f64; N])>, t: f64) -> [f64; N] {
    let mut prev: Option<(f64, [f64; N])> = None;
    for (kt, v) in keys {
        match prev {
            None if t <= kt => return v,
            Some((pt, pv)) if t <= kt => {
                let u = (t - pt) / (kt - pt);
                return std::array::from_fn(|i| pv[i] + (v[i] - pv[i]) * u);
            }
            _ => prev = Some((kt, v)),
        }
    }
    prev.map_or([0.0; N], |(_, v)| v)
}

fn size_at(v: &OverLife, t: f64) -> f64 {
    sample(v.size.iter().map(|k| (k[0], [k[1]])), t)[0]
}

fn color_at(v: &OverLife, t: f64) -> [f64; 4] {
    sample(v.color.iter().copied(), t)
}

/// Where a new key goes: the middle of the widest gap between keys (also before the first and
/// after the last key, which may sit inside 0..1). `None` when no gap leaves `MIN_GAP` on both
/// sides of the new key.
fn widest_gap(times: impl Iterator<Item = f64>) -> Option<f64> {
    let mut edges = vec![0.0];
    edges.extend(times);
    edges.push(1.0);
    edges
        .windows(2)
        .map(|w| (w[1] - w[0], (w[0] + w[1]) / 2.0))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .filter(|(gap, _)| *gap >= 2.0 * MIN_GAP)
        .map(|(_, mid)| mid)
}

/// Life range key `i` may move along (strictly between its neighbours; the end keys stay).
/// `None` when the neighbours leave no room (authored keys may be closer than `MIN_GAP`).
fn t_range(times: &[f64], i: usize) -> Option<(f64, f64)> {
    (i > 0 && i + 1 < times.len())
        .then(|| (times[i - 1] + MIN_GAP, times[i + 1] - MIN_GAP))
        .filter(|(a, b)| a <= b)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

impl Editor {
    /// One emitter edit (`ACL: vfx ID size|color|size_curve|gradient ..`).
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
            let vfx = &mut self.tuning.vfx;
            vfx.selected = Some(id.to_owned());
            vfx.draft = None;
            vfx.drag = None;
            vfx.stop_drag = None;
            vfx.key = 0;
            vfx.stop = 0;
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
            self.tuning.vfx.stop_drag = None;
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
                                ui.painter()
                                    .rect_filled(r, 3.0, color32(e.values.color[0].1));
                                let row = ui.selectable_label(
                                    on,
                                    RichText::new(&e.id).monospace().size(12.0),
                                );
                                mark!(self.rects, format!("vfx:emitter:{}", e.id), row.rect);
                                if row.clicked() && !on {
                                    self.tuning.vfx.selected = Some(e.id.clone());
                                    self.tuning.vfx.draft = None;
                                    self.tuning.vfx.drag = None;
                                    self.tuning.vfx.stop_drag = None;
                                    self.tuning.vfx.key = 0;
                                    self.tuning.vfx.stop = 0;
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
        let authored = &e.values;
        // A drag whose release this widget never saw (tab hidden, focus lost) is over.
        if ui.input(|i| !i.pointer.any_down()) {
            self.tuning.vfx.drag = None;
            self.tuning.vfx.stop_drag = None;
        }
        let mut work = match &self.tuning.vfx.draft {
            Some((d, v)) if *d == id => v.clone(),
            _ => authored.clone(),
        };
        // Immediate whole-list removals ("Two-point" buttons), written outside the draft.
        let mut reset: Option<VfxEdit> = None;
        let editable = !self.previewing;
        let icons = self.icons;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&e.id).strong().color(theme::TEXT));
            theme::chip(ui, e.summary.as_str(), theme::MUTED);
            if self.tuning.vfx.draft.is_some() {
                theme::chip(ui, "editing…", theme::WARNING);
            }
        });
        ui.add_space(4.0);
        // Side by side when the tab is wide enough, else stacked.
        let wide = ui.available_width() >= 2.0 * 300.0 + 24.0;
        let width = if wide {
            ((ui.available_width() - 24.0) / 2.0).min(640.0)
        } else {
            ui.available_width().clamp(220.0, 640.0)
        };
        let layout = if wide {
            egui::Layout::left_to_right(egui::Align::Min)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        };
        let vfx = &mut self.tuning.vfx;
        let rects = &mut self.rects;
        vfx.key = vfx.key.min(work.size.len() - 1);
        vfx.stop = vfx.stop.min(work.color.len() - 1);
        ui.add_enabled_ui(editable, |ui| {
            ui.with_layout(layout, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(width);
                    // Size over life.
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("SIZE OVER LIFE")
                                .size(11.0)
                                .color(theme::MUTED),
                        );
                        theme::chip(
                            ui,
                            if work.size_curve {
                                format!("curve · {} keys", work.size.len())
                            } else {
                                "two-point".to_owned()
                            },
                            theme::MUTED,
                        );
                    });
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(width, CURVE_H), Sense::click_and_drag());
                    mark!(rects, "vfx:curve".into(), rect);
                    let inner = rect.shrink2(vec2(14.0, 10.0));
                    let peak = work.size.iter().map(|k| k[1]).fold(0.0, f64::max);
                    let ymax = vfx
                        .drag
                        .map_or_else(|| (peak * 1.6).clamp(0.2, 4.0), |(_, y)| y);
                    let to_screen = |t: f64, v: f64| {
                        pos2(
                            inner.left() + t as f32 * inner.width(),
                            inner.bottom() - (v / ymax) as f32 * inner.height(),
                        )
                    };
                    let from_screen = |p: egui::Pos2| {
                        (
                            ((p.x - inner.left()) / inner.width()) as f64,
                            ((inner.bottom() - p.y) / inner.height()) as f64 * ymax,
                        )
                    };
                    let handles: Vec<egui::Pos2> =
                        work.size.iter().map(|k| to_screen(k[0], k[1])).collect();
                    let near = |p: egui::Pos2| {
                        handles.iter().position(|h| {
                            (h.x - p.x).abs() <= 3.0 * HANDLE && (h.y - p.y).abs() <= 3.0 * HANDLE
                        })
                    };
                    if response.double_clicked() && editable {
                        if let Some(p) = response.interact_pointer_pos() {
                            let (t, v) = from_screen(p);
                            let t = round3(t.clamp(0.0, 1.0));
                            let free = work.size.iter().all(|k| (k[0] - t).abs() >= MIN_GAP);
                            if near(p).is_none() && free && work.size.len() < MAX_VFX_CURVE_KEYS {
                                let at = work.size.partition_point(|k| k[0] < t);
                                work.size.insert(at, [t, round3(v.clamp(0.0, 4.0))]);
                                vfx.key = at;
                            }
                        }
                    } else if response.drag_started() && editable {
                        if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                            // A key under the pointer, else the nearest one along life.
                            let which = near(p).unwrap_or_else(|| {
                                handles
                                    .iter()
                                    .enumerate()
                                    .min_by(|a, b| {
                                        (a.1.x - p.x).abs().total_cmp(&(b.1.x - p.x).abs())
                                    })
                                    .map_or(0, |(i, _)| i)
                            });
                            vfx.drag = Some((which, ymax));
                            vfx.key = which;
                        }
                    } else if response.clicked() {
                        if let Some(i) = response.interact_pointer_pos().and_then(near) {
                            vfx.key = i;
                        }
                    }
                    if let (Some((h, ymax)), Some(p)) = (vfx.drag, response.interact_pointer_pos())
                    {
                        if response.dragged() && h < work.size.len() {
                            let v = ((inner.bottom() - p.y) / inner.height()) as f64 * ymax;
                            // A two-point start size is at least 0.005 (the `size` field).
                            let lo = if h == 0 && !work.size_curve { 0.005 } else { 0.0 };
                            work.size[h][1] = round3(v.clamp(lo, 4.0));
                            let times: Vec<f64> = work.size.iter().map(|k| k[0]).collect();
                            if let Some((a, b)) = t_range(&times, h) {
                                let t = ((p.x - inner.left()) / inner.width()) as f64;
                                work.size[h][0] = round3(t.clamp(a, b));
                            }
                        }
                    }
                    if response.drag_stopped() {
                        vfx.drag = None;
                    }
                    let handles: Vec<egui::Pos2> =
                        work.size.iter().map(|k| to_screen(k[0], k[1])).collect();
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
                    // The curve held flat before the first and after the last key.
                    let mut line = vec![pos2(inner.left(), handles[0].y)];
                    line.extend(handles.iter().copied());
                    line.push(pos2(inner.right(), handles[handles.len() - 1].y));
                    // Filled one segment at a time (each quad is convex).
                    for w in line.windows(2) {
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                pos2(w[0].x, inner.bottom()),
                                w[0],
                                w[1],
                                pos2(w[1].x, inner.bottom()),
                            ],
                            theme::ACCENT.gamma_multiply(0.15),
                            Stroke::NONE,
                        ));
                    }
                    painter.add(egui::Shape::line(
                        line,
                        Stroke::new(2.0_f32, theme::ACCENT),
                    ));
                    // Particles sampled along life: size and colour at 9 points.
                    for i in 0..9 {
                        let t = i as f64 / 8.0;
                        let r = ((size_at(&work, t) / ymax) as f32 * inner.height() * 0.25)
                            .clamp(1.5, 16.0);
                        painter.circle_filled(
                            pos2(
                                inner.left() + t as f32 * inner.width(),
                                inner.bottom() - 2.0 - r,
                            ),
                            r,
                            color32(color_at(&work, t)),
                        );
                    }
                    for (i, h) in handles.iter().enumerate() {
                        let hot = vfx.drag.is_some_and(|(d, _)| d == i);
                        let picked = vfx.key == i;
                        painter.circle(
                            *h,
                            HANDLE,
                            if hot { theme::WARNING } else { theme::TEXT },
                            Stroke::new(
                                if picked { 2.5_f32 } else { 1.5 },
                                if picked { theme::ACCENT } else { Color32::BLACK },
                            ),
                        );
                        mark!(
                            rects,
                            format!("vfx:size_handle:{i}"),
                            Rect::from_center_size(*h, vec2(2.0 * HANDLE, 2.0 * HANDLE))
                        );
                    }
                    // The selected key: life and size fields, add / remove, back to two-point.
                    ui.horizontal(|ui| {
                        let k = vfx.key;
                        ui.label(RichText::new(format!("key {}", k + 1)).color(theme::MUTED));
                        let times: Vec<f64> = work.size.iter().map(|k| k[0]).collect();
                        if let Some((a, b)) = t_range(&times, k) {
                            let r = ui.add(
                                egui::DragValue::new(&mut work.size[k][0])
                                    .range(a..=b)
                                    .speed(0.005)
                                    .max_decimals(3)
                                    .prefix("life "),
                            );
                            mark!(rects, "vfx:key_t".into(), r.rect);
                        }
                        let lo = if k == 0 && !work.size_curve { 0.005 } else { 0.0 };
                        let r = ui.add(
                            egui::DragValue::new(&mut work.size[k][1])
                                .range(lo..=4.0)
                                .speed(0.005)
                                .max_decimals(3)
                                .suffix(" m"),
                        );
                        mark!(rects, format!("vfx:size:{k}"), r.rect);
                        let gap = widest_gap(work.size.iter().map(|k| k[0]))
                            .filter(|_| work.size.len() < MAX_VFX_CURVE_KEYS);
                        let add = ui
                            .add_enabled_ui(gap.is_some(), |ui| {
                                theme::icon_button(
                                    ui,
                                    icons,
                                    Icon::Add,
                                    "Add a key in the widest gap (or double-click the curve)",
                                    false,
                                )
                            })
                            .inner;
                        mark!(rects, "vfx:key_add".into(), add.rect);
                        if let (true, Some(t)) = (add.clicked(), gap) {
                            let t = round3(t);
                            let at = work.size.partition_point(|k| k[0] < t);
                            work.size.insert(at, [t, round3(size_at(&work, t))]);
                            vfx.key = at;
                        }
                        let remove = ui
                            .add_enabled_ui(work.size.len() > 2, |ui| {
                                theme::icon_button(
                                    ui,
                                    icons,
                                    Icon::Delete,
                                    "Remove the selected key (at least two stay)",
                                    false,
                                )
                            })
                            .inner;
                        mark!(rects, "vfx:key_remove".into(), remove.rect);
                        if remove.clicked() {
                            work.size.remove(k);
                            vfx.key = k.min(work.size.len() - 1);
                        }
                        if authored.size_curve {
                            let r = ui
                                .small_button("Two-point")
                                .on_hover_text("Remove size_curve: back to size -> end_size (ACL: vfx ID size_curve none)");
                            mark!(rects, "vfx:size_reset".into(), r.rect);
                            if r.clicked() {
                                reset = Some(VfxEdit::SizeCurve { keys: vec![] });
                            }
                        }
                    });
                });
                ui.add_space(if wide { 24.0 } else { 8.0 });
                ui.vertical(|ui| {
                    ui.set_width(width);
                    // Colour over life.
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("COLOUR OVER LIFE")
                                .size(11.0)
                                .color(theme::MUTED),
                        );
                        theme::chip(
                            ui,
                            if work.color_curve {
                                format!("gradient · {} stops", work.color.len())
                            } else {
                                "two-stop".to_owned()
                            },
                            theme::MUTED,
                        );
                    });
                    let (bar, bar_response) = ui
                        .allocate_exact_size(vec2(width, BAR_H + 12.0), Sense::click_and_drag());
                    mark!(rects, "vfx:gradient".into(), bar);
                    let strip = Rect::from_min_size(
                        bar.min + vec2(HANDLE, 0.0),
                        vec2(bar.width() - 2.0 * HANDLE, BAR_H),
                    );
                    let x_of = |t: f64| strip.left() + t as f32 * strip.width();
                    let t_of = |x: f32| ((x - strip.left()) / strip.width()) as f64;
                    let nearest = |x: f32, stops: &[(f64, [f64; 4])]| {
                        stops
                            .iter()
                            .enumerate()
                            .min_by(|a, b| {
                                (x_of(a.1 .0) - x).abs().total_cmp(&(x_of(b.1 .0) - x).abs())
                            })
                            .map(|(i, s)| (i, (x_of(s.0) - x).abs()))
                    };
                    if bar_response.double_clicked() && editable {
                        if let Some(p) = bar_response.interact_pointer_pos() {
                            let t = round3(t_of(p.x).clamp(0.0, 1.0));
                            let free = work.color.iter().all(|s| (s.0 - t).abs() >= MIN_GAP);
                            if free && work.color.len() < MAX_VFX_CURVE_KEYS {
                                let at = work.color.partition_point(|s| s.0 < t);
                                let c = color_at(&work, t).map(round3);
                                work.color.insert(at, (t, c));
                                vfx.stop = at;
                            }
                        }
                    } else if bar_response.drag_started() && editable {
                        if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                            if let Some((i, d)) = nearest(p.x, &work.color) {
                                if d <= 2.0 * HANDLE {
                                    vfx.stop = i;
                                    vfx.stop_drag = Some(i);
                                }
                            }
                        }
                    } else if bar_response.clicked() {
                        if let Some(p) = bar_response.interact_pointer_pos() {
                            if let Some((i, _)) = nearest(p.x, &work.color) {
                                vfx.stop = i;
                            }
                        }
                    }
                    if let (Some(i), Some(p)) = (vfx.stop_drag, bar_response.interact_pointer_pos())
                    {
                        let times: Vec<f64> = work.color.iter().map(|s| s.0).collect();
                        if let (true, Some((a, b))) =
                            (bar_response.dragged() && i < times.len(), t_range(&times, i))
                        {
                            work.color[i].0 = round3(t_of(p.x).clamp(a, b));
                        }
                    }
                    if bar_response.drag_stopped() {
                        vfx.stop_drag = None;
                    }
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
                    // One quad per segment, held colour before the first / after the last stop.
                    let mut mesh = egui::Mesh::default();
                    let mut edges = vec![(0.0, color_at(&work, 0.0))];
                    edges.extend(work.color.iter().copied());
                    edges.push((1.0, color_at(&work, 1.0)));
                    for w in edges.windows(2) {
                        let (x0, x1) = (x_of(w[0].0), x_of(w[1].0));
                        let (a, b) = (color32(w[0].1), color32(w[1].1));
                        let base = mesh.vertices.len() as u32;
                        mesh.colored_vertex(pos2(x0, strip.top()), a);
                        mesh.colored_vertex(pos2(x1, strip.top()), b);
                        mesh.colored_vertex(pos2(x1, strip.bottom()), b);
                        mesh.colored_vertex(pos2(x0, strip.bottom()), a);
                        mesh.add_triangle(base, base + 1, base + 2);
                        mesh.add_triangle(base, base + 2, base + 3);
                    }
                    painter.add(mesh);
                    painter.rect_stroke(
                        strip,
                        2.0,
                        Stroke::new(1.0_f32, theme::BORDER),
                        egui::StrokeKind::Inside,
                    );
                    for (i, (t, c)) in work.color.iter().enumerate() {
                        let x = x_of(*t);
                        let y = strip.bottom() + 1.0;
                        let selected = vfx.stop == i;
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                pos2(x, y),
                                pos2(x + HANDLE, y + 10.0),
                                pos2(x - HANDLE, y + 10.0),
                            ],
                            // Opaque, so a fully transparent stop stays visible.
                            color32([c[0], c[1], c[2], 1.0]),
                            Stroke::new(
                                if selected { 2.0_f32 } else { 1.0 },
                                if selected { theme::ACCENT } else { theme::TEXT },
                            ),
                        ));
                        mark!(
                            rects,
                            format!("vfx:stop:{i}"),
                            Rect::from_center_size(pos2(x, y + 5.0), vec2(2.0 * HANDLE, 10.0))
                        );
                    }
                    let s = vfx.stop;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("stop {}", s + 1))
                                .strong()
                                .color(theme::TEXT),
                        );
                        let mut c = work.color[s].1.map(|x| x as f32);
                        if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                            work.color[s].1 = c.map(|x| round3(f64::from(x)));
                        }
                        let times: Vec<f64> = work.color.iter().map(|s| s.0).collect();
                        if let Some((a, b)) = t_range(&times, s) {
                            let r = ui.add(
                                egui::DragValue::new(&mut work.color[s].0)
                                    .range(a..=b)
                                    .speed(0.005)
                                    .max_decimals(3)
                                    .prefix("life "),
                            );
                            mark!(rects, "vfx:stop_t".into(), r.rect);
                        }
                        let gap = widest_gap(work.color.iter().map(|s| s.0))
                            .filter(|_| work.color.len() < MAX_VFX_CURVE_KEYS);
                        let add = ui
                            .add_enabled_ui(gap.is_some(), |ui| {
                                theme::icon_button(
                                    ui,
                                    icons,
                                    Icon::Add,
                                    "Add a stop in the widest gap (or double-click the bar)",
                                    false,
                                )
                            })
                            .inner;
                        mark!(rects, "vfx:stop_add".into(), add.rect);
                        if let (true, Some(t)) = (add.clicked(), gap) {
                            let t = round3(t);
                            let at = work.color.partition_point(|s| s.0 < t);
                            let c = color_at(&work, t).map(round3);
                            work.color.insert(at, (t, c));
                            vfx.stop = at;
                        }
                        let remove = ui
                            .add_enabled_ui(work.color.len() > 2, |ui| {
                                theme::icon_button(
                                    ui,
                                    icons,
                                    Icon::Delete,
                                    "Remove the selected stop (at least two stay)",
                                    false,
                                )
                            })
                            .inner;
                        mark!(rects, "vfx:stop_remove".into(), remove.rect);
                        if remove.clicked() {
                            work.color.remove(s);
                            vfx.stop = s.min(work.color.len() - 1);
                        }
                        if authored.color_curve {
                            let r = ui
                                .small_button("Two-stop")
                                .on_hover_text("Remove color_gradient: back to color -> end_color (ACL: vfx ID gradient none)");
                            mark!(rects, "vfx:color_reset".into(), r.rect);
                            if r.clicked() {
                                reset = Some(VfxEdit::ColorGradient { stops: vec![] });
                            }
                        }
                    });
                    let s = vfx.stop.min(work.color.len() - 1);
                    for (ch, name) in ["R", "G", "B", "A"].into_iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).monospace().color(theme::MUTED));
                            let r = ui.add(
                                egui::Slider::new(&mut work.color[s].1[ch], 0.0..=1.0)
                                    .max_decimals(3)
                                    .show_value(true),
                            );
                            mark!(rects, format!("vfx:channel:{ch}"), r.rect);
                        });
                    }
                });
            })
        });
        if let Some(edit) = reset {
            self.tuning.vfx.draft = None;
            self.tuning.vfx.key = 0;
            self.tuning.vfx.stop = 0;
            actions.extend(self.vfx_edit(&id, &edit));
            return;
        }
        // Draft bookkeeping and the single commit once the interaction is over.
        self.tuning.vfx.draft = (&work != authored).then(|| (id.clone(), work));
        let released = ui.input(|i| !i.pointer.any_down())
            && !ui.memory(|m| m.any_popup_open())
            && ui
                .memory(|m| m.focused())
                .is_none_or(|f| egui::TextEdit::load_state(ui.ctx(), f).is_none());
        let idle = self.tuning.vfx.drag.is_none() && self.tuning.vfx.stop_drag.is_none();
        if released && idle {
            if let Some((id, v)) = self.tuning.vfx.draft.take() {
                // Both writes of this frame share `expected_rev`; the host's `coalesce` merges
                // them into one revision (= `vfx ID size ..; vfx ID gradient ..` in one script).
                if v.size != authored.size {
                    actions.extend(self.vfx_edit(&id, &size_edit(&v)));
                }
                if v.color != authored.color {
                    actions.extend(self.vfx_edit(&id, &color_edit(&v)));
                }
            }
        }
    }
}

/// Two-point fields while a two-point emitter keeps exactly two keys at life 0 and 1, else
/// the whole `size_curve`.
pub(crate) fn size_edit(v: &OverLife) -> VfxEdit {
    match v.size.as_slice() {
        [a, b] if !v.size_curve && a[0] == 0.0 && b[0] == 1.0 => VfxEdit::Size {
            start: a[1],
            end: b[1],
        },
        keys => VfxEdit::SizeCurve {
            keys: keys.to_vec(),
        },
    }
}

/// Two-stop fields while a two-stop emitter keeps exactly two stops at life 0 and 1, else the
/// whole `color_gradient`.
pub(crate) fn color_edit(v: &OverLife) -> VfxEdit {
    match v.color.as_slice() {
        [a, b] if !v.color_curve && a.0 == 0.0 && b.0 == 1.0 => VfxEdit::Color {
            start: a.1,
            end: b.1,
        },
        stops => VfxEdit::ColorGradient {
            stops: stops.to_vec(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_sampling_matches_the_runtime_curve() {
        let v = OverLife {
            size: vec![[0.2, 0.1], [0.5, 0.4], [0.9, 0.0]],
            color: vec![(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 0.0])],
            size_curve: true,
            color_curve: false,
        };
        let e = crate::vfx::Emitter {
            size_curve: Some(
                v.size
                    .iter()
                    .map(|k| crate::vfx::SizeKey {
                        t: k[0] as f32,
                        size: k[1] as f32,
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        for i in 0..=20 {
            let t = i as f64 / 20.0;
            let cpu = crate::vfx::size_at(&e, t as f32);
            assert!((size_at(&v, t) - f64::from(cpu)).abs() < 1e-5, "t={t}");
        }
        assert_eq!(size_at(&v, 0.0), 0.1);
        assert_eq!(size_at(&v, 1.0), 0.0);
        assert_eq!(color_at(&v, 0.5), [0.5, 0.0, 0.5, 0.5]);
        assert_eq!(MAX_VFX_CURVE_KEYS, crate::vfx::MAX_CURVE_KEYS);
    }

    #[test]
    fn two_point_emitters_keep_two_point_fields_until_they_get_a_third_key() {
        let mut v = OverLife {
            size: vec![[0.0, 0.1], [1.0, 0.0]],
            color: vec![(0.0, [1.0; 4]), (1.0, [0.0; 4])],
            size_curve: false,
            color_curve: false,
        };
        assert!(matches!(size_edit(&v), VfxEdit::Size { .. }));
        assert!(matches!(color_edit(&v), VfxEdit::Color { .. }));
        v.size.insert(1, [0.5, 0.3]);
        v.color[1].0 = 0.8;
        assert_eq!(
            size_edit(&v),
            VfxEdit::SizeCurve {
                keys: vec![[0.0, 0.1], [0.5, 0.3], [1.0, 0.0]]
            }
        );
        assert!(matches!(color_edit(&v), VfxEdit::ColorGradient { .. }));
        assert_eq!(widest_gap([0.0, 0.2, 1.0].into_iter()), Some(0.6));
        // Gaps at the ends count (keys inside 0..1); no room = no key.
        assert_eq!(widest_gap([0.1, 0.2, 0.3].into_iter()), Some(0.65));
        let crowded: Vec<f64> = (0..=60).map(|i| f64::from(i) / 60.0).collect();
        assert_eq!(widest_gap(crowded.into_iter()), None);
        assert_eq!(t_range(&[0.0, 0.5, 1.0], 1), Some((0.01, 0.99)));
        assert_eq!(t_range(&[0.0, 1.0], 0), None);
        // Authored keys closer than the editor's gap: no drag range (no inverted clamp).
        assert_eq!(t_range(&[0.0, 0.005, 0.01, 1.0], 1), None);
    }
}
