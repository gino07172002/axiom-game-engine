//! UI Editor tab: a WYSIWYG canvas of the world's `ui.document`, drawn by the game's own HUD
//! code (`bound_ui::draw_canvas`) in a reference resolution scaled into the tab, with a live
//! locale switch and missing-string warnings, `ui.nameplate` previews and an element panel
//! (region, style, button command, typed fields, effects). Dragging a widget to another of
//! the nine screen regions commits one revision on release. Every content change goes through
//! `axiom_core::ops` (the ACL `ui ...` verbs compile to the same functions).
use super::{form, Action, Dock, Editor};
use crate::gameplay::Gameplay;
use crate::ui_effects::{contribution, EffectVisuals, Visual};
use axiom_core::gameplay::{
    BoundWidget, CompiledGameplay, Condition, EffectTrigger, Region, RuntimeCommand, Scalar,
    SlideDirection, TextStyle, UiDocument, UiEffect, UiEffectKind, UiTrigger, Widget,
};
use axiom_core::ops;
use egui::{Color32, Pos2, Rect, RichText, Stroke, Ui, Vec2};
use serde_json::Value;
use std::collections::BTreeMap;

const MUTED: Color32 = Color32::from_rgb(138, 154, 172);
const WARNING: Color32 = Color32::from_rgb(255, 178, 111);
const ACCENT: Color32 = Color32::from_rgb(102, 245, 205);
/// Reference resolutions the canvas lays the HUD out in (then scales to fit the tab).
pub(super) const RESOLUTIONS: [(&str, f32, f32); 4] = [
    ("1280x720 (16:9)", 1280.0, 720.0),
    ("1920x1080 (16:9)", 1920.0, 1080.0),
    ("1024x768 (4:3)", 1024.0, 768.0),
    ("2560x1080 (21:9)", 2560.0, 1080.0),
];
/// Session chrome height in the game window (full debug bar); compact chrome takes none.
const CHROME: f32 = 30.0;
const PANEL: f32 = 270.0;
/// Height the element panel needs when it sits below the canvas.
const PANEL_BELOW: f32 = 240.0;
const NAMEPLATES: f32 = 28.0;
/// Regions in grid order (row by row), the drop cells of the canvas.
const GRID: [Region; 9] = [
    Region::TopLeft,
    Region::Top,
    Region::TopRight,
    Region::Left,
    Region::Center,
    Region::Right,
    Region::BottomLeft,
    Region::Bottom,
    Region::BottomRight,
];

/// View state of the UI Editor (never part of the world).
#[derive(Default)]
pub(crate) struct UiEditorState {
    /// Selected widget ID.
    pub selected: Option<String>,
    /// Selected effect ID (its form is open).
    pub effect: Option<String>,
    /// Previewed locale; `None` = the document's default.
    pub locale: Option<String>,
    pub resolution: usize,
    pub hide_nameplates: bool,
    /// The previewed gameplay state for (revision, world version).
    cache: Option<Preview>,
    /// The widget pressed on the canvas (and where), and the one being dragged.
    press: Option<(String, Pos2)>,
    drag: Option<String>,
    /// Document point (reference-resolution pixels) under the pointer at the press and now,
    /// while a widget is pressed.
    press_doc: Option<(Pos2, Pos2)>,
    /// Canvas zoom relative to fitting the tab (0 = never set = 1), pan of the canvas from
    /// the fitted position in screen points, a middle-button pan in progress, and the size of
    /// the canvas view when last drawn (`ui.zoom` zooms around its center).
    zoom: f32,
    pan: Vec2,
    panning: bool,
    view_size: Vec2,
    /// egui time the effect preview of the selected widget started.
    playing: Option<f64>,
    /// Kind offered by Add effect.
    new_kind: Option<UiEffectKind>,
    /// Edited copy of the selected effect / widget (committed when nothing is held or typed).
    effect_draft: Option<UiEffect>,
    widget_draft: Option<(String, Value)>,
    canvas: crate::ui::UiState,
    /// Times the preview was rebuilt (tests: not per frame).
    pub builds: usize,
}

struct Preview {
    key: (i64, u64),
    gameplay: Result<Gameplay, String>,
    doc: Option<UiDocument>,
    /// Per locale: keys whose text is empty or still the key itself.
    missing: BTreeMap<String, Vec<String>>,
    /// Per locale: keys with the same text as the default locale.
    untranslated: BTreeMap<String, usize>,
    nameplates: Vec<(String, crate::nameplate::Nameplate)>,
}

impl Preview {
    fn build(key: (i64, u64), world: &axiom_core::World) -> Self {
        let doc: Option<UiDocument> = world
            .resources
            .get(ops::UI_DOCUMENT)
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let mut missing = BTreeMap::new();
        let mut untranslated = BTreeMap::new();
        if let Some(d) = &doc {
            let default = d.catalogs.get(&d.locale);
            for (locale, catalog) in &d.catalogs {
                missing.insert(
                    locale.clone(),
                    catalog
                        .iter()
                        .filter(|(k, v)| v.trim().is_empty() || v == k)
                        .map(|(k, _)| k.clone())
                        .collect(),
                );
                if locale != &d.locale {
                    untranslated.insert(
                        locale.clone(),
                        catalog
                            .iter()
                            .filter(|(k, v)| default.and_then(|c| c.get(*k)) == Some(*v))
                            .count(),
                    );
                }
            }
        }
        let nameplates = world
            .entities
            .iter()
            .filter_map(|e| {
                let v = e.components.get("ui.nameplate")?;
                Some((e.id.clone(), serde_json::from_value(v.clone()).ok()?))
            })
            .take(16)
            .collect();
        Self {
            key,
            gameplay: Gameplay::new(world).map_err(|e| format!("{e:#}")),
            doc,
            missing,
            untranslated,
            nameplates,
        }
    }
}

/// Effect visuals of the canvas preview: every effect of one widget, `elapsed` steps in.
struct CanvasFx<'a> {
    compiled: &'a CompiledGameplay,
    values: &'a [Scalar],
    widget: usize,
    elapsed: u32,
}

impl EffectVisuals for CanvasFx<'_> {
    fn visual(&self, widget: usize) -> Visual {
        let mut v = Visual::IDENTITY;
        if widget != self.widget {
            return v;
        }
        for e in self.compiled.effects.iter().filter(|e| e.widget == widget) {
            if self.elapsed >= e.ticks {
                continue;
            }
            let to = e
                .variable
                .map_or(0.0, |i| crate::ui_effects::number(&self.values[i]));
            let from = e.from.unwrap_or(0.0);
            // Nothing to count at the authored value: roll to the bar's end (or +100).
            let to = if to == from {
                match &self.compiled.widgets[widget] {
                    BoundWidget::Progress(_, _, max) => *max,
                    _ => from + 100.0,
                }
            } else {
                to
            };
            contribution(e, self.elapsed, from, to, &mut v);
        }
        v
    }
}

impl UiEditorState {
    /// Canvas zoom relative to fitting the tab (1 = fit).
    pub(super) fn zoom(&self) -> f32 {
        if self.zoom > 0.0 {
            self.zoom
        } else {
            1.0
        }
    }

    /// Sets the zoom (clamped) keeping the document point under `anchor` (relative to the
    /// view's top-left corner, screen points) where it is.
    fn zoom_around(&mut self, anchor: Vec2, zoom: f32) {
        let old = self.zoom();
        let new = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        self.pan = anchor - (anchor - self.pan) * (new / old);
        self.zoom = new;
    }

    /// Fit: 100 %, no pan.
    fn fit(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }

    /// While a widget is pressed: how far the pointer moved since the press, in document
    /// (reference-resolution) pixels.
    #[cfg(test)]
    pub(super) fn drag_doc_delta(&self) -> Option<Vec2> {
        self.press_doc.map(|(from, to)| to - from)
    }
}

/// UI Editor canvas zoom limits (`ui.zoom`, wheel), relative to fitting the tab.
pub(super) const ZOOM_MIN: f32 = 0.25;
pub(super) const ZOOM_MAX: f32 = 2.0;
/// Screen points of the canvas that always stay inside the view while panning.
const KEEP_VISIBLE: f32 = 48.0;

fn region_index(r: Region) -> usize {
    GRID.iter().position(|g| *g == r).unwrap_or(0)
}

/// Screen point -> document point (reference-resolution pixels) on a canvas drawn at `canvas`
/// for a document of `size`.
fn to_doc(canvas: Rect, size: Vec2, p: Pos2) -> Pos2 {
    let d = p - canvas.min;
    Pos2::new(
        d.x * size.x / canvas.width(),
        d.y * size.y / canvas.height(),
    )
}

/// The drop cell at document point `doc` (3 x 3 grid over the reference resolution).
fn region_at(size: Vec2, doc: Pos2) -> Region {
    let col = ((doc.x / size.x * 3.0).floor() as i32).clamp(0, 2);
    let row = ((doc.y / size.y * 3.0).floor() as i32).clamp(0, 2);
    GRID[(row * 3 + col) as usize]
}

fn cell_rect(canvas: Rect, r: Region) -> Rect {
    let i = region_index(r);
    let (col, row) = ((i % 3) as f32, (i / 3) as f32);
    let size = canvas.size() / 3.0;
    Rect::from_min_size(canvas.min + Vec2::new(col * size.x, row * size.y), size)
}

fn kind_of(w: &Widget) -> &'static str {
    match w {
        Widget::Text { .. } => "text",
        Widget::Button { .. } => "button",
        Widget::Choices { .. } => "choices",
        Widget::Image { .. } => "image",
        Widget::Progress { .. } => "progress",
        Widget::Slider { .. } => "slider",
    }
}

fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(11.0).color(MUTED));
}

impl Editor {
    // Content edits of the UI Editor: each is one `axiom_core::ops` call (= an ACL `ui` verb).

    /// Canvas drag / Region dropdown (`ACL: ui place WIDGET REGION|none`).
    pub(super) fn ui_place(&mut self, widget: &str, region: Option<Region>) -> Option<Action> {
        let result = ops::set_ui_region(&self.world, widget, region);
        self.op_write(result, None)
    }

    /// Style dropdown (`ACL: ui style WIDGET STYLE|none`).
    pub(super) fn ui_style(&mut self, widget: &str, style: Option<TextStyle>) -> Option<Action> {
        let result = ops::set_ui_style(&self.world, widget, style);
        self.op_write(result, None)
    }

    /// Button command picker (`ACL: ui button WIDGET ...`).
    pub(super) fn ui_button_command(
        &mut self,
        widget: &str,
        command: &RuntimeCommand,
    ) -> Option<Action> {
        let result = ops::set_ui_button_command(&self.world, widget, command);
        self.op_write(result, None)
    }

    /// Add effect (`ACL: ui effect add ID WIDGET KIND`); the new effect's form opens.
    pub(super) fn ui_add_effect(&mut self, effect: &UiEffect) -> Option<Action> {
        let result = ops::add_ui_effect(&self.world, &self.registry, effect);
        self.op_write(result, None)
    }

    /// Effect form (`ACL: ui effect set ID ...`).
    pub(super) fn ui_set_effect(&mut self, effect: &UiEffect) -> Option<Action> {
        let result = ops::set_ui_effect(&self.world, effect);
        self.op_write(result, None)
    }

    /// Effect row x (`ACL: ui effect remove ID`).
    pub(super) fn ui_remove_effect(&mut self, id: &str) -> Option<Action> {
        let result = ops::remove_ui_effect(&self.world, id);
        self.op_write(result, None)
    }

    /// Element form (`ACL: set widgets[id=WIDGET] F VALUE` per changed field).
    pub(super) fn ui_widget_fields(
        &mut self,
        widget: &str,
        fields: &[(String, Value)],
    ) -> Option<Action> {
        let result = ops::set_ui_widget_fields(&self.world, widget, fields);
        self.op_write(result, None)
    }

    /// `ui.*` registry commands. Content commands take the ACL `ui` statement's arguments and
    /// compile through the same parser; view commands select / switch the preview.
    pub(super) fn ui_command(&mut self, id: &str, arg: Option<&str>) -> Vec<Action> {
        let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
            self.show_dock(Dock::UiEditor);
            self.notice("UI Editor: pick the widget on the canvas and use the element panel");
            return vec![];
        };
        let statement = match id {
            "ui.place" => format!("place {arg}"),
            "ui.style" => format!("style {arg}"),
            "ui.button" => format!("button {arg}"),
            "ui.slider" => format!("slider {arg}"),
            "ui.effect_add" => format!("effect add {arg}"),
            "ui.effect_set" => format!("effect set {arg}"),
            "ui.effect_remove" => format!("effect remove {arg}"),
            "ui.select" => {
                let known = self
                    .ui_document()
                    .is_some_and(|d| d.widgets.iter().any(|w| w.id() == arg));
                if known {
                    self.ui_editor.selected = Some(arg.to_owned());
                    self.ui_editor.effect = None;
                    self.ui_editor.effect_draft = None;
                    self.ui_editor.widget_draft = None;
                    self.show_dock(Dock::UiEditor);
                } else {
                    self.set_error(format!("ui.select: no widget '{arg}' in ui.document"));
                }
                return vec![];
            }
            "ui.locale" => {
                let known = self
                    .ui_document()
                    .is_some_and(|d| d.catalogs.contains_key(arg));
                if known {
                    self.ui_editor.locale = Some(arg.to_owned());
                    self.show_dock(Dock::UiEditor);
                } else {
                    self.set_error(format!("ui.locale: ui.document has no catalog '{arg}'"));
                }
                return vec![];
            }
            "ui.resolution" => {
                match RESOLUTIONS
                    .iter()
                    .position(|(name, ..)| name.starts_with(arg))
                {
                    Some(i) => self.ui_editor.resolution = i,
                    None => self.set_error(format!(
                        "ui.resolution: use {}",
                        RESOLUTIONS
                            .map(|r| r.0.split(' ').next().unwrap_or(""))
                            .join(", ")
                    )),
                }
                return vec![];
            }
            other => {
                self.set_error(format!("{other} is not a UI Editor command"));
                return vec![];
            }
        };
        let result = axiom_core::acl::ui_command_edits(&self.world, &self.registry, &statement);
        self.op_write(result, None).into_iter().collect()
    }

    fn ui_document(&self) -> Option<UiDocument> {
        self.world
            .resources
            .get(ops::UI_DOCUMENT)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Plays the effects of the selected widget on the canvas (`ui.preview_effects`).
    pub(super) fn ui_preview_effects(&mut self) {
        self.ui_editor.playing = Some(f64::NAN);
    }

    /// `ui.zoom in|out|reset|fit|FACTOR|N%`: canvas zoom around the view's center (view only).
    pub(super) fn ui_zoom_command(&mut self, arg: Option<&str>) -> anyhow::Result<()> {
        let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
            self.show_dock(Dock::UiEditor);
            self.notice(
                "Wheel over the UI canvas zooms around the pointer; middle-drag pans; the % button fits",
            );
            return Ok(());
        };
        let st = &mut self.ui_editor;
        let current = st.zoom();
        let zoom = match arg {
            "in" => current * 1.25,
            "out" => current / 1.25,
            "reset" | "fit" => {
                st.fit();
                return Ok(());
            }
            number => super::graph::parse_zoom(number, ZOOM_MIN, ZOOM_MAX).ok_or_else(|| {
                anyhow::anyhow!(
                    "ui.zoom takes in|out|reset|fit or a zoom {ZOOM_MIN}..{ZOOM_MAX} of the fitted canvas (e.g. 1.5 or 150%)"
                )
            })?,
        };
        let center = st.view_size / 2.0;
        st.zoom_around(center, zoom);
        Ok(())
    }

    /// Commits the open effect form and field drafts (before the selection moves on).
    fn ui_flush(&mut self, st: &mut UiEditorState, doc: &UiDocument, actions: &mut Vec<Action>) {
        if let Some(draft) = st.effect_draft.take() {
            if doc.effects.iter().any(|e| e.id == draft.id && *e != draft) {
                actions.extend(self.ui_set_effect(&draft));
            }
        }
        if let Some((id, draft)) = st.widget_draft.take() {
            let mut changed = vec![];
            if let Some(i) = doc.widgets.iter().position(|w| w.id() == id) {
                form::changes(
                    &self.world.resources[ops::UI_DOCUMENT]["widgets"][i],
                    &draft,
                    "",
                    &mut changed,
                );
            }
            if !changed.is_empty() {
                actions.extend(self.ui_widget_fields(&id, &changed));
            }
        }
    }

    /// The UI Editor tab body.
    pub(super) fn ui_editor_tab(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let mut st = std::mem::take(&mut self.ui_editor);
        let key = (self.rev, self.world_version);
        if st.cache.as_ref().is_none_or(|c| c.key != key) {
            st.cache = Some(Preview::build(key, &self.world));
            st.builds += 1;
            st.effect_draft = None;
            st.widget_draft = None;
        }
        let mut preview = st.cache.take().unwrap();
        self.ui_editor_body(ui, &mut st, &mut preview, actions);
        st.cache = Some(preview);
        self.ui_editor = st;
    }

    fn ui_editor_body(
        &mut self,
        ui: &mut Ui,
        st: &mut UiEditorState,
        preview: &mut Preview,
        actions: &mut Vec<Action>,
    ) {
        let Some(doc) = &preview.doc else {
            caption(
                ui,
                "This world has no ui.document. World › Create Resource… ui.document (it needs behavior.program).",
            );
            return;
        };
        let g = match &mut preview.gameplay {
            Ok(g) => g,
            Err(e) => {
                ui.colored_label(WARNING, format!("The UI document does not compile: {e}"));
                return;
            }
        };
        if st
            .selected
            .as_ref()
            .is_some_and(|s| !doc.widgets.iter().any(|w| w.id() == s))
        {
            st.selected = None;
        }
        if st
            .effect
            .as_ref()
            .is_some_and(|s| !doc.effects.iter().any(|e| &e.id == s))
        {
            st.effect = None;
            st.effect_draft = None;
        }
        let locale = st
            .locale
            .clone()
            .filter(|l| doc.catalogs.contains_key(l))
            .unwrap_or_else(|| doc.locale.clone());
        // Toolbar: resolution, locale, warnings, nameplates.
        ui.horizontal_wrapped(|ui| {
            ui.label("Resolution");
            let r = egui::ComboBox::from_id_salt("ui_editor_resolution")
                .selected_text(RESOLUTIONS[st.resolution.min(3)].0)
                .show_ui(ui, |ui| {
                    for (i, (name, ..)) in RESOLUTIONS.iter().enumerate() {
                        ui.selectable_value(&mut st.resolution, i, *name);
                    }
                });
            self.rects
                .insert("ui_editor:resolution".into(), r.response.rect);
            ui.separator();
            ui.label("Locale");
            for l in doc.catalogs.keys() {
                let r = ui.selectable_label(&locale == l, l);
                self.rects.insert(format!("ui_locale:{l}"), r.rect);
                if r.clicked() {
                    st.locale = Some(l.clone());
                }
            }
            let missing = preview.missing.get(&locale).map_or(0, Vec::len);
            let same = preview.untranslated.get(&locale).copied().unwrap_or(0);
            if missing > 0 {
                ui.colored_label(WARNING, format!("{missing} missing string(s)"))
                    .on_hover_text(
                        preview.missing[&locale]
                            .iter()
                            .take(24)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
            } else {
                ui.label(RichText::new("no missing strings").color(MUTED));
            }
            if same > 0 {
                caption(ui, format!("{same} same as {}", doc.locale));
            }
            ui.separator();
            let mut show = !st.hide_nameplates;
            if ui.checkbox(&mut show, "Nameplates").changed() {
                st.hide_nameplates = !show;
            }
            ui.separator();
            // Fixed width: the toolbar must not rewrap (and move the canvas) while zooming.
            let r = ui
                .add(
                    egui::Button::new(format!("{:.0}%", st.zoom() * 100.0))
                        .min_size(Vec2::new(56.0, 0.0)),
                )
                .on_hover_text(
                    "Zoom (100 % = fit to the tab): wheel over the canvas zooms around the pointer, middle-drag pans; click to fit",
                );
            self.rects.insert("ui_editor:zoom".into(), r.rect);
            if r.clicked() {
                st.fit();
            }
        });
        let area = ui.available_rect_before_wrap();
        let (_, w, h) = RESOLUTIONS[st.resolution.min(3)];
        let size = Vec2::new(w, h);
        // The element panel goes beside or below the canvas, whichever leaves it larger; a
        // strip under the canvas holds the nameplate preview.
        let plates = if st.hide_nameplates || preview.nameplates.is_empty() {
            0.0
        } else {
            NAMEPLATES
        };
        let fit = |room: Vec2| (room.x / w).min(room.y / h).max(0.05);
        let beside = fit(Vec2::new(
            area.width() - PANEL - 8.0,
            area.height() - plates,
        ));
        let below = fit(Vec2::new(
            area.width(),
            area.height() - plates - PANEL_BELOW,
        ));
        let fitted = beside.max(below);
        // The view is where the canvas fits the tab; zoom and pan move the canvas inside it
        // (view state only, never a revision).
        let view = Rect::from_min_size(area.min, size * fitted);
        let panel = if beside >= below {
            Rect::from_min_max(Pos2::new(view.max.x + 8.0, area.min.y), area.max)
        } else {
            Rect::from_min_max(Pos2::new(area.min.x, view.max.y + plates + 4.0), area.max)
        };
        st.view_size = view.size();
        // Only pointer positions on the view itself (not on a menu or popup above it).
        let on_view = |p: Pos2| {
            view.contains(p) && ui.ctx().layer_id_at(p).is_none_or(|l| l == ui.layer_id())
        };
        let (hover, wheel, pinch, middle_pressed, middle_down, moved) = ui.input(|i| {
            (
                i.pointer.hover_pos(),
                i.raw_scroll_delta.y,
                i.zoom_delta(),
                i.pointer.button_pressed(egui::PointerButton::Middle),
                i.pointer.middle_down(),
                i.pointer.delta(),
            )
        });
        // Wheel (or pinch / Ctrl+wheel; egui reports Ctrl+wheel as both, use one) zooms around
        // the pointer and uses the scroll up; not while a widget press or a pan is held.
        let held = st.press.is_some() || st.panning;
        if let Some(p) = hover.filter(|p| !held && on_view(*p)) {
            let factor = if (pinch - 1.0).abs() > 1e-4 {
                pinch
            } else {
                (wheel * 0.003).exp()
            };
            if (factor - 1.0).abs() > 1e-4 {
                st.zoom_around(p - view.min, st.zoom() * factor);
                ui.input_mut(|i| {
                    i.raw_scroll_delta = Vec2::ZERO;
                    i.smooth_scroll_delta = Vec2::ZERO;
                });
            }
        }
        // Middle-drag pans (from the frame after the press: this frame's pointer delta may
        // come from before it).
        if st.panning {
            if middle_down {
                st.pan += moved;
            } else {
                st.panning = false;
            }
        }
        if middle_pressed && st.press.is_none() && hover.is_some_and(on_view) {
            st.panning = true;
        }
        // `draw_canvas` never draws smaller than 5 %.
        let scale = (fitted * st.zoom()).max(0.05);
        let canvas_size = size * scale;
        // Keep part of the canvas in the view so it cannot be panned out of reach.
        let keep = Vec2::splat(KEEP_VISIBLE).min(canvas_size);
        let lo = keep - canvas_size;
        let hi = (view.size() - keep).max(lo);
        st.pan = st.pan.clamp(lo, hi);
        let canvas = Rect::from_min_size(view.min + st.pan, canvas_size);
        self.rects.insert("ui_editor:canvas".into(), canvas);
        self.rects.insert("ui_editor:view".into(), view);
        // Preview locale: set on the cached state when switched (never rebuilt or copied).
        if g.locale.as_ref() != Some(&locale) {
            g.locale = Some(locale.clone());
        }
        let g: &Gameplay = g;
        let selected_index = st
            .selected
            .as_ref()
            .and_then(|s| doc.widgets.iter().position(|w| w.id() == s));
        let now = ui.input(|i| i.time);
        if st.playing.is_some_and(f64::is_nan) {
            st.playing = Some(now);
        }
        let longest = selected_index.map_or(0, |w| {
            g.compiled
                .effects
                .iter()
                .filter(|e| e.widget == w)
                .map(|e| e.ticks)
                .max()
                .unwrap_or(0)
        });
        let elapsed = st
            .playing
            .map(|start| ((now - start).max(0.0) * 120.0) as u32);
        if elapsed.is_some_and(|e| e >= longest) {
            st.playing = None;
        }
        let fx = match (st.playing, selected_index) {
            (Some(_), Some(widget)) => Some(CanvasFx {
                compiled: &g.compiled,
                values: &g.values,
                widget,
                elapsed: elapsed.unwrap_or(0),
            }),
            _ => None,
        };
        if fx.is_some() {
            ui.ctx().request_repaint();
        }
        // Canvas background, chrome and drop grid (clipped to the view when zoomed in).
        let painter = ui.painter_at(view);
        painter.rect_filled(view, 0.0, super::theme::FIELD);
        painter.rect_filled(canvas, 0.0, Color32::from_rgb(38, 52, 58));
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(canvas.min.x, canvas.center().y), canvas.max),
            0.0,
            Color32::from_rgb(48, 64, 52),
        );
        let top = match doc.chrome {
            axiom_core::gameplay::Chrome::Full => {
                let bar =
                    Rect::from_min_size(canvas.min, Vec2::new(canvas.width(), CHROME * scale));
                painter.rect_filled(bar, 0.0, Color32::from_rgb(12, 22, 27));
                painter.text(
                    bar.left_center() + Vec2::new(8.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("AXIOM / WORLD IR v2 | {}", self.world.title),
                    egui::FontId::proportional((14.0 * scale).max(7.0)),
                    ACCENT,
                );
                CHROME
            }
            axiom_core::gameplay::Chrome::Compact => 0.0,
        };
        let overlay = ui
            .ctx()
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("ui_canvas_overlay"),
            ))
            .with_clip_rect(canvas.intersect(view));
        if st.drag.is_some() {
            for r in GRID {
                let cell = cell_rect(canvas, r);
                overlay.rect_stroke(
                    cell.shrink(2.0),
                    4.0,
                    Stroke::new(1.0_f32, Color32::from_white_alpha(40)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        for r in GRID {
            self.rects.insert(
                format!("ui_cell:{}", ops::region_name(r)),
                cell_rect(canvas, r),
            );
        }
        // The HUD itself, by the game's code.
        st.canvas.playing = true;
        let rects = crate::bound_ui::draw_canvas(
            ui.ctx(),
            canvas,
            size,
            top,
            g,
            &mut st.canvas,
            fx.as_ref().map(|f| f as &dyn EffectVisuals),
            view.intersect(ui.clip_rect()),
        );
        if plates > 0.0 {
            // The game draws these over the 3D view at each entity; here they are listed in a
            // row under the canvas view, painted by the same function.
            let strip = Rect::from_min_size(
                Pos2::new(view.min.x, view.max.y),
                Vec2::new(view.width().max(panel.min.x - view.min.x - 8.0), plates),
            );
            let painter = ui.painter_at(strip);
            caption_at(&painter, strip.min + Vec2::new(2.0, 6.0), "ui.nameplate");
            let mut x = strip.min.x + 92.0;
            for (_, plate) in &preview.nameplates {
                let width = painter
                    .layout_no_wrap(
                        plate.text.clone(),
                        egui::FontId::proportional(14.0),
                        Color32::WHITE,
                    )
                    .size()
                    .x
                    + 10.0;
                let at = Pos2::new(x + width / 2.0, strip.center().y);
                let r = crate::nameplate::paint(&painter, at, &plate.text, plate.color);
                self.rects.insert(format!("ui_nameplate:{}", plate.text), r);
                x += width + 8.0;
            }
        }
        // Selection and drag between regions (one revision on release). Read from the raw
        // pointer: the canvas areas pass clicks through, and widgets inside them must not
        // take the press (text selection, buttons).
        for (index, rect) in &rects {
            self.rects
                .insert(format!("ui_canvas:{}", doc.widgets[*index].id()), *rect);
        }
        let (pressed, released, down, pos) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
            )
        });
        let hit = |p: Pos2| {
            rects
                .iter()
                .rev()
                .find(|(_, r)| r.contains(p))
                .map(|(i, _)| doc.widgets[*i].id().to_owned())
        };
        // Only presses that land on the visible canvas (not on a menu or popup above it).
        let on_canvas = |p: Pos2| canvas.contains(p) && on_view(p);
        if pressed && !st.panning {
            st.press = pos
                .filter(|p| on_canvas(*p))
                .and_then(|p| hit(p).map(|id| (id, p)));
            st.press_doc = st.press.as_ref().map(|(_, p)| {
                let at = to_doc(canvas, size, *p);
                (at, at)
            });
            if let Some((id, _)) = st.press.clone() {
                if st.selected.as_deref() != Some(&id) {
                    self.ui_flush(st, doc, actions);
                    st.effect = None;
                }
                st.selected = Some(id);
            }
        }
        if let (Some((id, origin)), Some(p), true) = (&st.press, pos, down) {
            if let Some((_, now)) = &mut st.press_doc {
                *now = to_doc(canvas, size, p);
            }
            if st.drag.is_none() && origin.distance(p) > 6.0 {
                st.drag = Some(id.clone());
            }
        }
        // Where a drop would land: the region of the document point under the pointer.
        let drop_at = |p: Pos2| {
            (canvas.contains(p) && view.contains(p))
                .then(|| region_at(size, to_doc(canvas, size, p)))
        };
        let mut place = None;
        if released {
            if let (Some(id), Some(p)) = (st.drag.take(), pos) {
                if let Some(to) = drop_at(p) {
                    if doc.regions.get(&id) != Some(&to) {
                        place = Some((id, to));
                    }
                }
            }
            st.press = None;
            st.press_doc = None;
        } else if !down {
            st.press = None;
            st.press_doc = None;
            st.drag = None;
        }
        let hovered = pos.filter(|p| on_canvas(*p)).and_then(hit);
        for (index, rect) in &rects {
            let id = doc.widgets[*index].id();
            let selected = st.selected.as_deref() == Some(id);
            if selected || hovered.as_deref() == Some(id) {
                overlay.rect_stroke(
                    rect.expand(2.0),
                    3.0,
                    Stroke::new(if selected { 2.0_f32 } else { 1.0 }, ACCENT),
                    egui::StrokeKind::Outside,
                );
            }
        }
        if let (Some(dragged), Some((p, to))) = (&st.drag, pos.and_then(|p| Some((p, drop_at(p)?))))
        {
            overlay.rect_filled(
                cell_rect(canvas, to).shrink(2.0),
                4.0,
                ACCENT.gamma_multiply(0.18),
            );
            caption_at(
                &overlay,
                p + Vec2::new(12.0, 12.0),
                &match st.press_doc {
                    // The document point too (reference-resolution pixels, any zoom).
                    Some((_, at)) => format!(
                        "{dragged} -> {} ({:.0}, {:.0})",
                        ops::region_name(to),
                        at.x,
                        at.y
                    ),
                    None => format!("{dragged} -> {}", ops::region_name(to)),
                },
            );
            ui.ctx().request_repaint();
        }
        if let Some((id, to)) = place {
            actions.extend(self.ui_place(&id, Some(to)));
        }
        // Element panel.
        let mut panel_ui = ui.new_child(egui::UiBuilder::new().max_rect(panel));
        egui::ScrollArea::vertical()
            .id_salt("ui_editor_panel")
            .auto_shrink([false, false])
            .show(&mut panel_ui, |ui| {
                self.ui_element_panel(ui, st, doc, g, actions);
            });
        ui.advance_cursor_after_rect(area);
    }

    fn ui_element_panel(
        &mut self,
        ui: &mut Ui,
        st: &mut UiEditorState,
        doc: &UiDocument,
        g: &Gameplay,
        actions: &mut Vec<Action>,
    ) {
        let Some(widget) = st
            .selected
            .as_ref()
            .and_then(|s| doc.widgets.iter().find(|w| w.id() == s))
            .cloned()
        else {
            caption(ui, "Click a widget on the canvas to edit it; drag it to another screen region to move it.");
            ui.add_space(6.0);
            ui.label(RichText::new("Widgets").strong());
            for w in &doc.widgets {
                let place = doc
                    .regions
                    .get(w.id())
                    .map_or("(bottom panel)".to_owned(), |r| ops::region_name(*r));
                let r = ui.selectable_label(false, format!("{}  {}  {place}", w.id(), kind_of(w)));
                self.rects.insert(format!("ui_widget:{}", w.id()), r.rect);
                if r.clicked() {
                    self.ui_flush(st, doc, actions);
                    st.effect = None;
                    st.selected = Some(w.id().to_owned());
                }
            }
            return;
        };
        let id = widget.id().to_owned();
        let mut deselect = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&id).strong().color(ACCENT));
            caption(ui, kind_of(&widget));
            if ui.small_button("Deselect").clicked() {
                deselect = true;
            }
        });
        if deselect {
            self.ui_flush(st, doc, actions);
            st.selected = None;
            st.effect = None;
            return;
        }
        // Region.
        let current = doc.regions.get(&id).copied();
        let mut region = current;
        ui.horizontal(|ui| {
            ui.label("Region");
            let r = egui::ComboBox::from_id_salt("ui_editor_region")
                .selected_text(region.map_or("(bottom panel)".to_owned(), ops::region_name))
                .show_ui(ui, |ui| {
                    let r = ui.selectable_value(&mut region, None, "(bottom panel)");
                    self.rects.insert("ui_region:none".into(), r.rect);
                    for r in GRID {
                        let name = ops::region_name(r);
                        let item = ui.selectable_value(&mut region, Some(r), &name);
                        self.rects.insert(format!("ui_region:{name}"), item.rect);
                    }
                });
            self.rects
                .insert("ui_editor:region".into(), r.response.rect);
        });
        if region != current {
            actions.extend(self.ui_place(&id, region));
        }
        // Style (text and progress).
        if matches!(widget, Widget::Text { .. } | Widget::Progress { .. }) {
            let current = doc.styles.get(&id).copied();
            let mut style = current;
            ui.horizontal(|ui| {
                ui.label("Style");
                let r = egui::ComboBox::from_id_salt("ui_editor_style")
                    .selected_text(style.map_or("(default)".to_owned(), ops::style_name))
                    .show_ui(ui, |ui| {
                        let r = ui.selectable_value(&mut style, None, "(default)");
                        self.rects.insert("ui_style:none".into(), r.rect);
                        for name in ops::STYLE_NAMES {
                            let s: TextStyle = serde_json::from_value(Value::from(name)).unwrap();
                            let item = ui.selectable_value(&mut style, Some(s), name);
                            self.rects.insert(format!("ui_style:{name}"), item.rect);
                        }
                    });
                self.rects.insert("ui_editor:style".into(), r.response.rect);
            });
            if style != current {
                actions.extend(self.ui_style(&id, style));
            }
        }
        // Button command (typed gameplay commands).
        if let Widget::Button { command, .. } = &widget {
            self.ui_command_picker(ui, &id, command, g, doc, actions);
        }
        // All fields, typed form.
        ui.collapsing("Fields", |ui| {
            self.ui_widget_form(ui, st, doc, &id, actions);
        });
        ui.separator();
        self.ui_effects_panel(ui, st, doc, g, &id, actions);
    }

    fn ui_command_picker(
        &mut self,
        ui: &mut Ui,
        id: &str,
        command: &RuntimeCommand,
        g: &Gameplay,
        doc: &UiDocument,
        actions: &mut Vec<Action>,
    ) {
        let kind = match command {
            RuntimeCommand::Emit { .. } => "emit",
            RuntimeCommand::Locale { .. } => "locale",
            RuntimeCommand::Choose { .. } => "choose",
            RuntimeCommand::Call { .. } => "call",
            RuntimeCommand::Batch { .. } => "batch",
            RuntimeCommand::Slide { .. } => "slide",
        };
        let mut picked: Option<RuntimeCommand> = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Command");
            caption(ui, kind);
            match command {
                RuntimeCommand::Emit { event, .. } | RuntimeCommand::Locale { locale: event } => {
                    let r = egui::ComboBox::from_id_salt("ui_editor_command")
                        .selected_text(event.as_str())
                        .show_ui(ui, |ui| {
                            // Field-less events (a button sends no payload) and the locales.
                            for (e, schema) in &g.compiled.program.events {
                                if schema.fields.is_empty() {
                                    let item = ui.selectable_label(
                                        event == e && kind == "emit",
                                        format!("emit {e}"),
                                    );
                                    self.rects.insert(format!("ui_command:emit {e}"), item.rect);
                                    if item.clicked() {
                                        picked = Some(RuntimeCommand::Emit {
                                            event: e.clone(),
                                            fields: Default::default(),
                                        });
                                    }
                                }
                            }
                            for l in doc.catalogs.keys() {
                                let item = ui.selectable_label(
                                    event == l && kind == "locale",
                                    format!("locale {l}"),
                                );
                                self.rects
                                    .insert(format!("ui_command:locale {l}"), item.rect);
                                if item.clicked() {
                                    picked = Some(RuntimeCommand::Locale { locale: l.clone() });
                                }
                            }
                        });
                    self.rects
                        .insert("ui_editor:command".into(), r.response.rect);
                }
                RuntimeCommand::Choose { node, choice } => {
                    caption(ui, format!("{node} / {choice} (Fields to change)"));
                }
                RuntimeCommand::Call {
                    capability, action, ..
                } => {
                    caption(ui, format!("{capability}.{action} (Fields to change)"));
                }
                RuntimeCommand::Batch { commands } => {
                    caption(
                        ui,
                        format!(
                            "{} commands in one tick (Fields to change; ACL ui button {id} batch [..])",
                            commands.len()
                        ),
                    );
                }
                RuntimeCommand::Slide { widget, .. } => {
                    caption(ui, format!("slide {widget} (Fields to change)"));
                }
            }
        });
        if let Some(c) = picked.filter(|c| c != command) {
            actions.extend(self.ui_button_command(id, &c));
        }
    }

    /// The widget's fields with the Inspector's typed form; changed leaves commit once nothing
    /// is held or typed.
    fn ui_widget_form(
        &mut self,
        ui: &mut Ui,
        st: &mut UiEditorState,
        doc: &UiDocument,
        id: &str,
        actions: &mut Vec<Action>,
    ) {
        let Some(index) = doc.widgets.iter().position(|w| w.id() == id) else {
            return;
        };
        let base = self.world.resources[ops::UI_DOCUMENT]["widgets"][index].clone();
        if st.widget_draft.as_ref().is_none_or(|(w, _)| w != id) {
            st.widget_draft = Some((id.to_owned(), base.clone()));
        }
        let spec = self
            .specs
            .get(&self.registry, &self.world, ops::UI_DOCUMENT);
        let spec = spec
            .at(&format!("/widgets/{index}"))
            .cloned()
            .unwrap_or_else(|| form::Spec::compile(&Value::Null));
        let mixed = Default::default();
        let (_, draft) = st.widget_draft.as_mut().unwrap();
        let out = {
            let mut ctx = form::Ctx {
                world: &self.world,
                world_version: self.world_version,
                base: &base,
                mixed: &mixed,
                filter: "",
                structural: false,
                picker: &mut self.picker,
                rects: Some(&mut self.rects),
                overrides: None,
            };
            form::show(ui, &mut ctx, &spec, draft, &format!("ui_widget:{id}"))
        };
        let _ = out;
        let idle =
            !ui.input(|i| i.pointer.any_down()) && ui.ctx().memory(|m| m.focused().is_none());
        if idle && *draft != base {
            let mut changed = vec![];
            form::changes(&base, draft, "", &mut changed);
            if !changed.is_empty() {
                actions.extend(self.ui_widget_fields(id, &changed));
            }
            st.widget_draft = None;
        }
    }

    fn ui_effects_panel(
        &mut self,
        ui: &mut Ui,
        st: &mut UiEditorState,
        doc: &UiDocument,
        g: &Gameplay,
        widget: &str,
        actions: &mut Vec<Action>,
    ) {
        ui.label(RichText::new("Effects").strong());
        let mine: Vec<&UiEffect> = doc.effects.iter().filter(|e| e.widget == widget).collect();
        if mine.is_empty() {
            caption(ui, "No effects on this widget.");
        }
        let mut remove = None;
        let mut open = None;
        for e in &mine {
            ui.horizontal(|ui| {
                let x = ui.small_button("x").on_hover_text("Remove effect");
                self.rects
                    .insert(format!("ui_effect_remove:{}", e.id), x.rect);
                if x.clicked() {
                    remove = Some(e.id.clone());
                }
                let label = format!(
                    "{} · on {}{}",
                    e.kind.name(),
                    e.trigger().name(),
                    e.ticks.map_or(String::new(), |t| format!(" · {t}"))
                );
                let r = ui
                    .selectable_label(st.effect.as_deref() == Some(&e.id), label)
                    .on_hover_text(&e.id);
                self.rects.insert(format!("ui_effect:{}", e.id), r.rect);
                if r.clicked() && st.effect.as_deref() != Some(&e.id) {
                    open = Some(e.id.clone());
                }
            });
        }
        if let Some(id) = open {
            self.ui_flush(st, doc, actions);
            st.effect = Some(id);
        }
        if let Some(id) = remove {
            let action = self.ui_remove_effect(&id);
            if action.is_some() && st.effect.as_deref() == Some(id.as_str()) {
                st.effect = None;
                st.effect_draft = None;
            }
            actions.extend(action);
        }
        // Add effect: kind + Add (defaults; edit below).
        ui.horizontal(|ui| {
            let mut kind = st.new_kind.unwrap_or(UiEffectKind::FadeIn);
            let r = egui::ComboBox::from_id_salt("ui_editor_new_effect")
                .selected_text(kind.name())
                .show_ui(ui, |ui| {
                    for k in UiEffectKind::ALL {
                        let item = ui.selectable_value(&mut kind, k, k.name());
                        self.rects
                            .insert(format!("ui_new_kind:{}", k.name()), item.rect);
                    }
                });
            self.rects
                .insert("ui_editor:new_kind".into(), r.response.rect);
            st.new_kind = Some(kind);
            let add = ui.button("Add effect");
            self.rects.insert("ui_editor:add_effect".into(), add.rect);
            if add.clicked() {
                let effect = UiEffect::new(
                    ops::free_ui_effect_id(&self.world, widget, kind),
                    widget,
                    kind,
                );
                let action = self.ui_add_effect(&effect);
                if action.is_some() {
                    st.effect = Some(effect.id.clone());
                    st.effect_draft = None;
                }
                actions.extend(action);
            }
            if !mine.is_empty() {
                let play = ui.button("▶ Preview");
                self.rects.insert("ui_editor:preview".into(), play.rect);
                if play.clicked() {
                    st.playing = Some(f64::NAN);
                }
            }
        });
        // Form of the selected effect.
        let Some(selected) = st
            .effect
            .as_ref()
            .and_then(|id| doc.effects.iter().find(|e| &e.id == id))
        else {
            return;
        };
        if st.effect_draft.as_ref().is_none_or(|d| d.id != selected.id) {
            st.effect_draft = Some(selected.clone());
        }
        let program_events: Vec<String> = self
            .world
            .resources
            .get("behavior.program")
            .and_then(|p| p.get("events"))
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        let draft = st.effect_draft.as_mut().unwrap();
        egui::Grid::new("ui_effect_form")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Kind");
                let before = draft.kind;
                egui::ComboBox::from_id_salt("ui_effect_kind")
                    .selected_text(draft.kind.name())
                    .show_ui(ui, |ui| {
                        for k in UiEffectKind::ALL {
                            ui.selectable_value(&mut draft.kind, k, k.name());
                        }
                    });
                if draft.kind != before {
                    // Amounts mean different things per kind; start from the new kind's defaults.
                    draft.amount = None;
                    draft.direction = None;
                    draft.from = None;
                    draft.ticks = None;
                    draft.ease = None;
                }
                ui.end_row();
                ui.label("Trigger");
                let mut on = draft.on;
                let r = egui::ComboBox::from_id_salt("ui_effect_on")
                    .selected_text(
                        on.map_or(format!("default ({})", draft.trigger().name()), |t| {
                            t.name().to_owned()
                        }),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut on, None, "default");
                        // when needs a condition (ACL ui effect set ID when={..}).
                        let conditional = !matches!(draft.when, Condition::Always {});
                        for t in UiTrigger::ALL
                            .into_iter()
                            .filter(|t| *t != UiTrigger::When || conditional)
                        {
                            let item = ui.selectable_value(&mut on, Some(t), t.name());
                            self.rects
                                .insert(format!("ui_effect_on:{}", t.name()), item.rect);
                        }
                    });
                self.rects
                    .insert("ui_editor:effect_on".into(), r.response.rect);
                if on != draft.on {
                    draft.on = on;
                    if on != Some(UiTrigger::Event) {
                        draft.event = None;
                    } else if draft.event.is_none() {
                        draft.event = program_events.first().cloned();
                    }
                }
                ui.end_row();
                if draft.on == Some(UiTrigger::Event) {
                    ui.label("Event");
                    let current = draft.event.clone().unwrap_or_default();
                    egui::ComboBox::from_id_salt("ui_effect_event")
                        .selected_text(&current)
                        .show_ui(ui, |ui| {
                            for e in &program_events {
                                if ui.selectable_label(&current == e, e).clicked() {
                                    draft.event = Some(e.clone());
                                }
                            }
                        });
                    ui.end_row();
                }
                ui.label("Steps");
                let mut ticks = draft
                    .ticks
                    .unwrap_or(if draft.kind == UiEffectKind::NumberRoll {
                        60
                    } else {
                        30
                    });
                let r = ui.add(
                    egui::DragValue::new(&mut ticks)
                        .range(1..=axiom_core::gameplay::MAX_UI_EFFECT_TICKS)
                        .suffix(" (120/s)"),
                );
                self.rects.insert("ui_editor:effect_ticks".into(), r.rect);
                if r.changed() {
                    draft.ticks = Some(ticks);
                }
                ui.end_row();
                ui.label("Ease");
                let mut ease = draft.ease;
                egui::ComboBox::from_id_salt("ui_effect_ease")
                    .selected_text(ease.map_or("default".to_owned(), |e| {
                        serde_json::to_value(e)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default()
                    }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut ease, None, "default");
                        for name in ["linear", "step", "ease_in", "ease_out", "ease_in_out"] {
                            let e = axiom_core::tween::Ease::parse(name).unwrap();
                            ui.selectable_value(&mut ease, Some(e), name);
                        }
                    });
                draft.ease = ease;
                ui.end_row();
                if matches!(draft.kind, UiEffectKind::SlideIn | UiEffectKind::SlideOut) {
                    ui.label("Direction");
                    let mut d = draft.direction.unwrap_or_default();
                    egui::ComboBox::from_id_salt("ui_effect_direction")
                        .selected_text(format!("{d:?}").to_lowercase())
                        .show_ui(ui, |ui| {
                            for (v, name) in [
                                (SlideDirection::Left, "left"),
                                (SlideDirection::Right, "right"),
                                (SlideDirection::Up, "up"),
                                (SlideDirection::Down, "down"),
                            ] {
                                ui.selectable_value(&mut d, v, name);
                            }
                        });
                    if Some(d) != draft.direction
                        && (draft.direction.is_some() || d != SlideDirection::Left)
                    {
                        draft.direction = Some(d);
                    }
                    ui.end_row();
                } else {
                    draft.direction = None;
                }
                if let Some((default, lo, hi)) = draft.kind.amount_range() {
                    ui.label("Amount");
                    let mut a = draft.amount.unwrap_or(default);
                    if ui
                        .add(egui::DragValue::new(&mut a).range(lo..=hi).speed(0.05))
                        .changed()
                    {
                        draft.amount = Some(a);
                    }
                    ui.end_row();
                } else {
                    draft.amount = None;
                }
                if draft.kind != UiEffectKind::NumberRoll {
                    draft.from = None;
                }
            });
        caption(
            ui,
            "Changes commit when the mouse is released; ACL: ui effect set ID key=value",
        );
        let idle =
            !ui.input(|i| i.pointer.any_down()) && ui.ctx().memory(|m| m.focused().is_none());
        if idle && draft != selected {
            let effect = draft.clone();
            actions.extend(self.ui_set_effect(&effect));
            st.effect_draft = None;
        }
        // The trigger the runtime will use, for the person reading the form.
        if let Some(e) = effect_summary(g, &selected.id) {
            caption(ui, e);
        }
    }
}

/// How the compiled effect runs, in words (defaults resolved).
fn effect_summary(g: &Gameplay, id: &str) -> Option<String> {
    let e = g.compiled.effects.iter().find(|e| e.id == id)?;
    Some(format!(
        "runs {} steps ({:.2} s), {}",
        e.ticks,
        e.ticks as f32 / 120.0,
        match e.trigger {
            EffectTrigger::Show => "when the widget appears (and at session start)".to_owned(),
            EffectTrigger::Hide => "when the widget disappears".to_owned(),
            EffectTrigger::Change => "when its variable changes".to_owned(),
            EffectTrigger::Event(i) => format!("when event {} fires", g.compiled.event_ids[i]),
            EffectTrigger::When => "when its condition turns true".to_owned(),
        }
    ))
}

fn caption_at(painter: &egui::Painter, at: Pos2, text: &str) {
    painter.text(
        at,
        egui::Align2::LEFT_TOP,
        text,
        egui::FontId::proportional(12.0),
        Color32::from_rgb(220, 230, 235),
    );
}
