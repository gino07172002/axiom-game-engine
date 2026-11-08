//! Data-bound immediate-mode presentation; clicks produce the same typed commands as replay.
use crate::{
    gameplay::Gameplay,
    ui::{Action, UiState},
};
use anyhow::{ensure, Result};
use axiom_core::gameplay::{BoundText, BoundWidget, Region, RuntimeCommand, Scalar, TextStyle};
use egui::{Color32, RichText, Vec2};
#[derive(Clone)]
pub struct PreparedImage {
    pub key: String,
    pub image: egui::ColorImage,
}
pub fn prepare_assets(g: Option<&mut Gameplay>) -> Result<()> {
    let Some(g) = g else { return Ok(()) };
    let Some(ui) = &g.compiled.ui else {
        return Ok(());
    };
    for (id, path) in &ui.images {
        ensure!(
            std::fs::metadata(path)?.len() <= 4_194_304,
            "UI image asset exceeds budget"
        );
        let raw = std::fs::read(path)?;
        let mut decoder = png::Decoder::new(std::io::Cursor::new(&raw));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info()?;
        let (w, h) = (reader.info().width, reader.info().height);
        ensure!(
            w > 0 && h > 0 && w <= 2048 && h <= 2048,
            "UI image dimensions exceed budget"
        );
        let mut bytes = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut bytes)?;
        let bytes = &bytes[..info.buffer_size()];
        let rgba = match info.color_type {
            png::ColorType::Rgba => bytes.to_vec(),
            png::ColorType::Rgb => bytes
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[1], c[2], 255])
                .collect(),
            png::ColorType::Grayscale => bytes.iter().flat_map(|c| [*c, *c, *c, 255]).collect(),
            png::ColorType::GrayscaleAlpha => bytes
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[0], c[0], c[1]])
                .collect(),
            _ => anyhow::bail!("unsupported PNG color format"),
        };
        let fingerprint = raw.iter().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100000001b3)
        });
        g.images.insert(
            id.clone(),
            std::sync::Arc::new(PreparedImage {
                key: format!("{path}/{fingerprint:x}"),
                image: egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
            }),
        );
    }
    Ok(())
}
pub fn draw(ctx: &egui::Context, g: &Gameplay, state: &mut UiState) -> Vec<Action> {
    let Some(doc) = &g.compiled.ui else {
        return vec![];
    };
    let mut actions = vec![];
    state.targets.clear();
    // Only the currently prepared assets remain cached after a content switch/reload.
    state
        .textures
        .retain(|key, _| g.images.values().any(|p| &p.key == key));
    let region_of = |w: &BoundWidget| doc.regions.get(widget_id(w)).copied();
    // HUD regions: compact translucent panels anchored over the 3D view.
    for region in REGIONS {
        let visible = g
            .compiled
            .widgets
            .iter()
            .any(|w| region_of(w) == Some(region) && widget_visible(g, w));
        let controls = region == Region::TopRight
            && doc.session_controls
            && !doc.regions.is_empty()
            && g.compiled.widgets.iter().all(|w| region_of(w).is_some());
        if !visible && !controls {
            continue;
        }
        let (align, mut offset, width, flow) = anchor(region);
        if matches!(region, Region::TopLeft | Region::Top | Region::TopRight) {
            // Clear whatever top panel the active chrome drew (none for compact chrome).
            offset[1] = ctx.available_rect().top() - ctx.screen_rect().top() + 10.0;
        }
        egui::Area::new(egui::Id::new(("bound_region", region)))
            .anchor(align, offset)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(12, 16, 24, 205))
                    .corner_radius(8.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_max_width(width);
                        if controls {
                            session_controls(ui, doc, g, state, &mut actions);
                        }
                        let filter = |w: &BoundWidget| region_of(w) == Some(region);
                        if flow {
                            ui.horizontal_wrapped(|ui| {
                                draw_widgets(ui, g, state, &mut actions, &filter, true)
                            });
                        } else {
                            draw_widgets(ui, g, state, &mut actions, &filter, false);
                        }
                    });
            });
    }
    let unplaced = g.compiled.widgets.iter().any(|w| region_of(w).is_none());
    if !doc.regions.is_empty() && !unplaced {
        if let Some(error) = &g.fault {
            egui::Area::new(egui::Id::new("bound_fault"))
                .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
                .show(ctx, |ui| {
                    ui.colored_label(Color32::LIGHT_RED, error.to_string())
                });
        }
        return actions;
    }
    egui::TopBottomPanel::bottom("bound_document")
        .resizable(false)
        .show(ctx, |ui| {
            ui.set_max_height(ctx.screen_rect().height() * 0.87);
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                if doc.session_controls {
                    session_controls(ui, doc, g, state, &mut actions);
                }
                let filter = |w: &BoundWidget| region_of(w).is_none();
                if doc.flow {
                    ui.horizontal_wrapped(|ui| {
                        draw_widgets(ui, g, state, &mut actions, &filter, true)
                    });
                } else {
                    draw_widgets(ui, g, state, &mut actions, &filter, false);
                }
                if let Some(error) = &g.fault {
                    ui.colored_label(Color32::LIGHT_RED, error.to_string());
                }
                if let Some(message) = state.output.last() {
                    ui.label(RichText::new(message).small().color(Color32::GRAY));
                }
                ui.add_space(8.0);
            });
        });
    actions
}

const REGIONS: [Region; 9] = [
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

/// (anchor, offset, max width, horizontal flow). Top offsets are recomputed against the
/// available rect when drawn.
fn anchor(region: Region) -> (egui::Align2, [f32; 2], f32, bool) {
    use egui::Align2 as A;
    match region {
        Region::TopLeft => (A::LEFT_TOP, [12.0, 44.0], 300.0, false),
        Region::Top => (A::CENTER_TOP, [0.0, 44.0], 760.0, true),
        Region::TopRight => (A::RIGHT_TOP, [-12.0, 44.0], 340.0, false),
        Region::Left => (A::LEFT_CENTER, [12.0, 0.0], 240.0, true),
        Region::Center => (A::CENTER_CENTER, [0.0, 0.0], 560.0, false),
        Region::Right => (A::RIGHT_CENTER, [-12.0, 0.0], 240.0, true),
        Region::BottomLeft => (A::LEFT_BOTTOM, [12.0, -12.0], 340.0, false),
        Region::Bottom => (A::CENTER_BOTTOM, [0.0, -12.0], 900.0, true),
        Region::BottomRight => (A::RIGHT_BOTTOM, [-12.0, -12.0], 420.0, false),
    }
}

fn widget_id(w: &BoundWidget) -> &str {
    match w {
        BoundWidget::Text(id, ..)
        | BoundWidget::Button(id, ..)
        | BoundWidget::Choices(id)
        | BoundWidget::Image(id, _)
        | BoundWidget::Progress(id, ..) => id,
    }
}

/// Whether a widget would draw anything now (empty regions are not drawn at all).
fn widget_visible(g: &Gameplay, w: &BoundWidget) -> bool {
    match w {
        BoundWidget::Text(_, binding, when) => g.test(when) && !g.bound_text(binding).is_empty(),
        BoundWidget::Button(_, _, _, when) => g.test(when),
        BoundWidget::Choices(_) => !g.choices().is_empty(),
        BoundWidget::Image(_, image) => g.images.contains_key(image),
        BoundWidget::Progress(..) => true,
    }
}

fn session_controls(
    ui: &mut egui::Ui,
    doc: &axiom_core::gameplay::UiDocument,
    g: &Gameplay,
    state: &UiState,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("AXIOM / NARRATIVE")
                .small()
                .color(Color32::from_rgb(102, 245, 205)),
        );
        for locale in doc.catalogs.keys() {
            if ui
                .selectable_label(g.locale.as_ref() == Some(locale), locale)
                .clicked()
            {
                actions.push(Action::Gameplay(RuntimeCommand::Locale {
                    locale: locale.clone(),
                }));
            }
        }
        if ui.button("SAVE / 存檔").clicked() {
            actions.push(Action::Save);
        }
        if ui.button("LOAD / 讀檔").clicked() {
            actions.push(Action::Load);
        }
        ui.label(if state.playing {
            ""
        } else {
            "PAUSED / 按 RUN 繼續"
        });
    });
}

fn draw_widgets(
    ui: &mut egui::Ui,
    g: &Gameplay,
    state: &mut UiState,
    actions: &mut Vec<Action>,
    filter: &dyn Fn(&BoundWidget) -> bool,
    flow: bool,
) {
    for widget in g.compiled.widgets.iter().filter(|w| filter(w)) {
        match widget {
            BoundWidget::Text(id, binding, when) if g.test(when) => {
                let text = g.bound_text(binding);
                let style = g.compiled.ui.as_ref().and_then(|d| d.styles.get(id));
                let (size, color, strong) = match style {
                    Some(TextStyle::Title) => (42.0, Some(Color32::from_rgb(255, 214, 102)), true),
                    Some(TextStyle::Heading) => {
                        (17.0, Some(Color32::from_rgb(102, 245, 205)), true)
                    }
                    Some(TextStyle::Banner) => (20.0, Some(Color32::from_rgb(255, 118, 84)), true),
                    Some(TextStyle::Value) => (20.0, Some(Color32::WHITE), true),
                    Some(TextStyle::Label) => (13.0, Some(Color32::from_rgb(158, 176, 182)), false),
                    Some(TextStyle::Muted) => (13.0, Some(Color32::from_rgb(130, 142, 148)), false),
                    None if matches!(binding, BoundText::Speaker) => (22.0, None, false),
                    None => (18.0, None, false),
                };
                if flow {
                    let width = ui
                        .painter()
                        .layout_no_wrap(
                            text.clone(),
                            egui::FontId::proportional(size),
                            ui.visuals().text_color(),
                        )
                        .size()
                        .x;
                    if width > ui.available_size_before_wrap().x {
                        ui.end_row();
                    }
                }
                if matches!(style, Some(TextStyle::Banner)) && flow {
                    ui.end_row();
                }
                ui.push_id(id, |ui| {
                    let mut rich = RichText::new(text).size(size);
                    if let Some(color) = color {
                        rich = rich.color(color);
                    }
                    if strong {
                        rich = rich.strong();
                    }
                    ui.add(egui::Label::new(rich).wrap());
                });
                if matches!(style, Some(TextStyle::Value)) && flow {
                    ui.add_space(10.0);
                }
            }
            BoundWidget::Button(id, text, command, when) if g.test(when) => {
                let text = g.text(text);
                if flow {
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    let width = ui
                        .painter()
                        .layout_no_wrap(text.clone(), font, ui.visuals().text_color())
                        .size()
                        .x
                        + 2.0 * ui.spacing().button_padding.x;
                    if width > ui.available_size_before_wrap().x {
                        ui.end_row();
                    }
                }
                ui.push_id(id, |ui| {
                    let response = ui.add_enabled(
                        state.playing && !state.console,
                        egui::Button::new(text)
                            .min_size(Vec2::new(if flow { 0.0 } else { 220.0 }, 32.0)),
                    );
                    state.targets.insert(format!("button/{id}"), response.rect);
                    if response.clicked() {
                        actions.push(Action::Gameplay(command.clone()));
                    }
                });
            }
            BoundWidget::Choices(id) => {
                if flow {
                    ui.end_row();
                }
                ui.push_id(id, |ui| {
                    for choice in g.choices() {
                        let response = ui.add_enabled(
                            state.playing && !state.console,
                            egui::Button::new(RichText::new(g.text(&choice.text)).size(18.0))
                                .min_size(Vec2::new(
                                    if flow {
                                        0.0
                                    } else {
                                        ui.available_width().min(900.0)
                                    },
                                    38.0,
                                )),
                        );
                        state
                            .targets
                            .insert(format!("choice/{}", choice.id), response.rect);
                        if response.clicked() {
                            actions.push(Action::Gameplay(RuntimeCommand::Choose {
                                node: g.compiled.nodes[g.node.unwrap()].id.clone(),
                                choice: choice.id.clone(),
                            }));
                        }
                    }
                });
                if flow {
                    ui.end_row();
                }
            }
            BoundWidget::Image(id, image) => {
                if let Some(p) = g.images.get(image) {
                    let texture = state.textures.entry(p.key.clone()).or_insert_with(|| {
                        ui.ctx()
                            .load_texture(&p.key, p.image.clone(), egui::TextureOptions::LINEAR)
                    });
                    ui.push_id(id, |ui| {
                        ui.add(
                            egui::Image::new(&*texture)
                                .max_width(ui.available_width().min(900.0))
                                .max_height(170.0),
                        );
                    });
                }
            }
            BoundWidget::Progress(id, i, max) => {
                let value = match &g.values[*i] {
                    Scalar::Int(v) => *v as f64,
                    Scalar::Number(v) => *v,
                    _ => 0.0,
                };
                ui.push_id(id, |ui| {
                    ui.add(
                        egui::ProgressBar::new((value / max).clamp(0.0, 1.0) as f32)
                            .show_percentage(),
                    );
                });
            }
            _ => {}
        }
    }
}
