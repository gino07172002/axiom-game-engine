//! Editor look: one palette, the egui style built from it, and the shared chrome helpers
//! (panel headers, cards, icon buttons, chips). Panels take their colours from here instead of
//! defining their own.
use super::icons::{Icon, Icons};
use egui::{Color32, CornerRadius, Frame, Margin, Response, RichText, Sense, Stroke, Ui, Vec2};
use std::sync::Arc;

/// Window background behind the panels (menu row, gaps between docked panels).
pub(crate) const BACKDROP: Color32 = Color32::from_rgb(13, 15, 21);
/// Top bar, tab bars, status bar.
pub(crate) const HEADER: Color32 = Color32::from_rgb(17, 20, 27);
/// Panel bodies.
pub(crate) const SURFACE: Color32 = Color32::from_rgb(21, 25, 33);
/// Cards, toolbars and grouped rows inside a panel.
pub(crate) const CARD: Color32 = Color32::from_rgb(28, 33, 43);
/// Text fields and other sunken areas.
pub(crate) const FIELD: Color32 = Color32::from_rgb(14, 17, 23);
/// Buttons at rest.
pub(crate) const CONTROL: Color32 = Color32::from_rgb(35, 41, 53);
pub(crate) const CONTROL_HOVER: Color32 = Color32::from_rgb(46, 53, 68);
pub(crate) const BORDER: Color32 = Color32::from_rgb(44, 50, 63);
pub(crate) const TEXT: Color32 = Color32::from_rgb(228, 231, 238);
pub(crate) const MUTED: Color32 = Color32::from_rgb(140, 149, 166);
/// Brand accent (#ff4d6d): selection, the active tab, Run, focus.
pub(crate) const ACCENT: Color32 = Color32::from_rgb(255, 77, 109);
/// Accent fill under selected rows / toggled buttons.
pub(crate) const ACCENT_SOFT: Color32 = Color32::from_rgb(86, 31, 45);
/// Outline of accent pills (active tab, Run).
pub(crate) const ACCENT_EDGE: Color32 = Color32::from_rgb(150, 48, 70);
/// Prefab instances in the Hierarchy.
pub(crate) const PREFAB: Color32 = Color32::from_rgb(122, 170, 255);
pub(crate) const WARNING: Color32 = Color32::from_rgb(255, 178, 111);
pub(crate) const SUCCESS: Color32 = Color32::from_rgb(88, 214, 141);
pub(crate) const ERROR: Color32 = Color32::from_rgb(240, 96, 96);
/// Corner radius of cards, buttons and fields.
pub(crate) const RADIUS: u8 = 5;

/// The editor's egui style, built once per context (the game keeps its own style).
pub(crate) fn style(base: &egui::Style) -> Arc<egui::Style> {
    let mut style = base.clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = None;
    v.panel_fill = SURFACE;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = Color32::from_rgb(24, 28, 37);
    v.code_bg_color = FIELD;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARNING;
    v.error_fg_color = ERROR;
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.text_cursor.stroke = Stroke::new(2.0_f32, ACCENT);
    let radius = CornerRadius::same(RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.noninteractive.corner_radius = radius;
    for (state, fill, stroke, text) in [
        (&mut w.inactive, CONTROL, Stroke::NONE, TEXT),
        (
            &mut w.hovered,
            CONTROL_HOVER,
            Stroke::new(1.0_f32, BORDER),
            Color32::WHITE,
        ),
        (
            &mut w.active,
            ACCENT_SOFT,
            Stroke::new(1.0_f32, ACCENT),
            Color32::WHITE,
        ),
        (&mut w.open, CONTROL, Stroke::new(1.0_f32, BORDER), TEXT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = stroke;
        state.fg_stroke = Stroke::new(1.0_f32, text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    // An 8 pt grid: controls 24 pt tall, 8 pt between items, 6 pt vertical.
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.interact_size.y = 24.0;
    // Taller rows: drop-down lists get more room before they scroll.
    style.spacing.combo_height = 360.0;
    style.spacing.icon_width = 16.0;
    style.spacing.icon_width_inner = 10.0;
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    for (text, size) in [
        (TextStyle::Body, 13.5),
        (TextStyle::Button, 13.5),
        (TextStyle::Small, 11.0),
        (TextStyle::Heading, 17.0),
    ] {
        style
            .text_styles
            .insert(text, FontId::new(size, Proportional));
    }
    style.spacing.menu_margin = Margin::same(6);
    Arc::new(style)
}

/// A rounded card (component sections, toolbars inside a panel).
pub(crate) fn card() -> Frame {
    Frame::new()
        .fill(CARD)
        .corner_radius(CornerRadius::same(8))
        .stroke(Stroke::new(1.0_f32, BORDER))
        .inner_margin(Margin::symmetric(12, 8))
}

/// A square icon button (`selected` = toggled on). The tooltip names the action.
pub(crate) fn icon_button(
    ui: &mut Ui,
    icons: Icons,
    icon: Icon,
    tooltip: &str,
    selected: bool,
) -> Response {
    icon_button_sized(ui, icons, icon, tooltip, selected, 24.0)
}

pub(crate) fn icon_button_sized(
    ui: &mut Ui,
    icons: Icons,
    icon: Icon,
    tooltip: &str,
    selected: bool,
    size: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = enabled && response.hovered();
        let fill = if selected {
            ACCENT_SOFT
        } else if hovered {
            CONTROL_HOVER
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, RADIUS, fill);
        }
        let tint = if !enabled {
            MUTED.gamma_multiply(0.5)
        } else if selected {
            ACCENT
        } else if hovered {
            Color32::WHITE
        } else {
            TEXT
        };
        let glyph = (size * 0.66).round();
        icons.paint(
            ui.painter(),
            egui::Rect::from_center_size(rect.center(), Vec2::splat(glyph)),
            icon,
            tint,
        );
    }
    response.on_hover_text(tooltip)
}

/// A small pill (tag summary, FPS chip, status values).
pub(crate) fn chip(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    Frame::new()
        .fill(CONTROL)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(7, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(11.5).color(color))
        })
        .response
}

/// A thin vertical divider between groups in a toolbar row.
pub(crate) fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 18.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0_f32, BORDER),
    );
}

/// A text button with a leading icon. `fill` = solid background (e.g. the accent Run button).
pub(crate) fn icon_text_button(
    ui: &mut Ui,
    icons: Icons,
    icon: Icon,
    text: &str,
    fill: Option<Color32>,
    tint: Color32,
) -> Response {
    let image = icons.image(ui.ctx(), icon, 16.0, tint);
    let mut button = egui::Button::image_and_text(image, RichText::new(text).color(tint));
    if let Some(fill) = fill {
        button = button.fill(fill);
    }
    ui.add(button)
}

/// A drop-down attached to `button`: a click toggles it, a click outside closes it. Unlike
/// `menu_button` it also opens in headless egui tests (same popup memory as a ComboBox).
pub(crate) fn dropdown<R>(
    ui: &mut Ui,
    id: egui::Id,
    button: &Response,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    if button.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup_below_widget(
        ui,
        id,
        button,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(180.0);
            add_contents(ui)
        },
    )
}

/// Closes the drop-down `id` (after an item in it was picked).
pub(crate) fn close_dropdown(ui: &Ui, id: egui::Id) {
    ui.memory_mut(|m| {
        if m.is_popup_open(id) {
            m.close_popup();
        }
    });
}

/// A collapsible card with an icon + title header (Inspector sections).
pub(crate) fn section(
    ui: &mut Ui,
    icons: Icons,
    icon: Icon,
    title: &str,
    default_open: bool,
    add: impl FnOnce(&mut Ui),
) {
    card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        let id = ui.make_persistent_id(("section", title));
        egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            default_open,
        )
        .show_header(ui, |ui| {
            icons.show(ui, icon, 16.0, ACCENT);
            ui.label(RichText::new(title).strong().color(TEXT));
        })
        .body(add);
    });
    ui.add_space(8.0);
}

/// Height of the top bar's pill controls.
pub(crate) const PILL_H: f32 = 32.0;

/// A rounded, bordered pill holding `add` (left to right, vertically centered); the whole
/// pill senses clicks (drop-down pills).
pub(crate) fn pill(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> Response {
    Frame::new()
        .fill(CONTROL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 0))
        .show(ui, |ui| {
            ui.set_min_height(PILL_H - 2.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            // horizontal keeps the parent's direction (with_layout would take the whole width
            // of a right-to-left parent).
            ui.horizontal(add);
        })
        .response
        .interact(Sense::click())
}

/// A pill grouping icon buttons (undo / redo, transport, settings).
pub(crate) fn pill_group<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    Frame::new()
        .fill(CONTROL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(1))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(add).inner
        })
}

/// Paints `color` over the four corners of `rect` outside a rounded rectangle of `radius`
/// (rounds a view the renderer composites as a plain rectangle). Four small fans.
pub(crate) fn round_corners(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    color: Color32,
) {
    const STEPS: usize = 6;
    for (corner, center, start) in [
        (
            rect.left_top(),
            rect.left_top() + Vec2::splat(radius),
            std::f32::consts::PI,
        ),
        (
            rect.right_top(),
            rect.right_top() + egui::vec2(-radius, radius),
            -std::f32::consts::FRAC_PI_2,
        ),
        (
            rect.right_bottom(),
            rect.right_bottom() - Vec2::splat(radius),
            0.0,
        ),
        (
            rect.left_bottom(),
            rect.left_bottom() + egui::vec2(radius, -radius),
            std::f32::consts::FRAC_PI_2,
        ),
    ] {
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(corner, color);
        for k in 0..=STEPS {
            let a = start + std::f32::consts::FRAC_PI_2 * k as f32 / STEPS as f32;
            mesh.colored_vertex(center + radius * egui::vec2(a.cos(), a.sin()), color);
        }
        for k in 0..STEPS as u32 {
            mesh.add_triangle(0, k + 1, k + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// The component "enabled" box: accent filled with a check when on (`changed()` reports a
/// toggle). `mixed` draws a dash (differs across the selection).
pub(crate) fn accent_check(ui: &mut Ui, icons: Icons, on: &mut bool, mixed: bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let fill = if (*on || mixed) && enabled {
        ACCENT
    } else if *on || mixed {
        ACCENT_EDGE
    } else {
        FIELD
    };
    ui.painter().rect_filled(rect, 4.0, fill);
    if !*on && !mixed {
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
    }
    if mixed {
        ui.painter().hline(
            rect.x_range().shrink(5.0),
            rect.center().y,
            Stroke::new(2.0_f32, Color32::WHITE),
        );
    } else if *on {
        icons.paint(ui.painter(), rect.shrink(2.0), Icon::Check, Color32::WHITE);
    }
    response
}
