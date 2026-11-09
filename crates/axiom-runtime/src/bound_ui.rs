//! Data-bound immediate-mode presentation; clicks produce the same typed commands as replay.
use crate::{
    gameplay::Gameplay,
    ui::{Action, UiState},
    ui_effects::{EffectVisuals, Visual},
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
/// Where the HUD regions are laid out: the whole window (`draw`), or the editor's canvas.
struct Surface<'a> {
    /// Rectangle the regions anchor in (`None` = the screen).
    area: Option<egui::Rect>,
    /// Height taken at the top by session chrome.
    top: f32,
    /// Area ID prefix (the editor canvas must not share the game's areas).
    salt: &'static str,
    fx: Option<&'a dyn EffectVisuals>,
    /// Widget rectangles drawn (widget index, layout rect), for the editor canvas.
    rects: Option<&'a mut Vec<(usize, egui::Rect)>>,
}

pub fn draw(
    ctx: &egui::Context,
    g: &Gameplay,
    state: &mut UiState,
    fx: Option<&dyn EffectVisuals>,
) -> Vec<Action> {
    // Default interaction prompt when no ui.document widget binds `{kind: prompt}`.
    if let Some(prompt) = &g.prompt {
        let authored = g
            .compiled
            .widgets
            .iter()
            .any(|w| matches!(w, BoundWidget::Text(_, BoundText::Prompt, _)));
        if !authored {
            egui::Area::new(egui::Id::new("bound_prompt"))
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 110.0])
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(12, 16, 24, 205))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(g.prompt_text(prompt))
                                    .size(18.0)
                                    .color(Color32::WHITE),
                            );
                        });
                });
        }
    }
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
    let mut surface = Surface {
        area: None,
        // Clear whatever top panel the active chrome drew (none for compact chrome).
        top: ctx.available_rect().top() - ctx.screen_rect().top(),
        salt: "bound_region",
        fx,
        rects: None,
    };
    draw_regions(ctx, g, state, &mut actions, &mut surface);
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
                        draw_widgets(ui, g, state, &mut actions, &filter, true, fx, None)
                    });
                } else {
                    draw_widgets(ui, g, state, &mut actions, &filter, false, fx, None);
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

/// HUD regions: compact translucent panels anchored over the 3D view (or the canvas).
fn draw_regions(
    ctx: &egui::Context,
    g: &Gameplay,
    state: &mut UiState,
    actions: &mut Vec<Action>,
    s: &mut Surface,
) {
    let Some(doc) = &g.compiled.ui else {
        return;
    };
    let region_of = |w: &BoundWidget| doc.regions.get(widget_id(w)).copied();
    let fx = s.fx;
    for region in REGIONS {
        let visible = g.compiled.widgets.iter().enumerate().any(|(i, w)| {
            region_of(w) == Some(region)
                && (widget_visible(g, w) || fx.is_some_and(|f| f.visual(i).forced))
        });
        let controls = region == Region::TopRight
            && doc.session_controls
            && !doc.regions.is_empty()
            && g.compiled.widgets.iter().all(|w| region_of(w).is_some());
        if !visible && !controls {
            continue;
        }
        let (align, mut offset, width, flow) = anchor(region);
        if matches!(region, Region::TopLeft | Region::Top | Region::TopRight) {
            offset[1] = s.top + 10.0;
        }
        let mut area = egui::Area::new(egui::Id::new((s.salt, region))).anchor(align, offset);
        if let Some(rect) = s.area {
            area = area.constrain_to(rect).interactable(false);
        }
        let rects = &mut s.rects;
        let canvas = s.area.is_some();
        area.show(ctx, |ui| {
            if canvas {
                // The editor reads the pointer itself; canvas text is not selectable.
                ui.style_mut().interaction.selectable_labels = false;
            }
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(12, 16, 24, 205))
                .corner_radius(8.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_max_width(width);
                    if controls {
                        session_controls(ui, doc, g, state, actions);
                    }
                    let filter = |w: &BoundWidget| region_of(w) == Some(region);
                    if flow {
                        ui.horizontal_wrapped(|ui| {
                            draw_widgets(
                                ui,
                                g,
                                state,
                                actions,
                                &filter,
                                true,
                                fx,
                                rects.as_deref_mut(),
                            )
                        });
                    } else {
                        draw_widgets(
                            ui,
                            g,
                            state,
                            actions,
                            &filter,
                            false,
                            fx,
                            rects.as_deref_mut(),
                        );
                    }
                });
        });
    }
}

/// The editor's UI canvas: the document's regions and widgets drawn by the same code as the
/// game HUD, laid out in a virtual `size` screen (with `top` pixels of session chrome) and
/// scaled into `rect`. Nothing in it is interactive; button clicks never run commands.
/// Returns each drawn widget's rectangle in screen space (widget index, rect).
pub fn draw_canvas(
    ctx: &egui::Context,
    rect: egui::Rect,
    size: Vec2,
    top: f32,
    g: &Gameplay,
    state: &mut UiState,
    fx: Option<&dyn EffectVisuals>,
) -> Vec<(usize, egui::Rect)> {
    let Some(doc) = &g.compiled.ui else {
        return vec![];
    };
    let scale = (rect.width() / size.x)
        .min(rect.height() / size.y)
        .max(0.05);
    let screen = egui::Rect::from_min_size(rect.min, size);
    let transform = egui::emath::TSTransform::new(rect.min.to_vec2() * (1.0 - scale), scale);
    let mut actions = vec![];
    let mut rects = vec![];
    state.targets.clear();
    let mut surface = Surface {
        area: Some(screen),
        top,
        salt: "ui_canvas_region",
        fx,
        rects: Some(&mut rects),
    };
    draw_regions(ctx, g, state, &mut actions, &mut surface);
    for region in REGIONS {
        ctx.set_transform_layer(
            egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new(("ui_canvas_region", region)),
            ),
            transform,
        );
    }
    // Unplaced widgets: the game draws them in a bottom panel across the window.
    let region_of = |w: &BoundWidget| doc.regions.get(widget_id(w)).copied();
    if doc.regions.is_empty() || g.compiled.widgets.iter().any(|w| region_of(w).is_none()) {
        let id = egui::Id::new("ui_canvas_document");
        egui::Area::new(id)
            .constrain_to(screen)
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, 0.0])
            .interactable(false)
            .show(ctx, |ui| {
                ui.style_mut().interaction.selectable_labels = false;
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(size.x - 16.0);
                        if doc.session_controls {
                            session_controls(ui, doc, g, state, &mut actions);
                        }
                        let filter = |w: &BoundWidget| region_of(w).is_none();
                        if doc.flow {
                            ui.horizontal_wrapped(|ui| {
                                draw_widgets(
                                    ui,
                                    g,
                                    state,
                                    &mut actions,
                                    &filter,
                                    true,
                                    fx,
                                    Some(&mut rects),
                                )
                            });
                        } else {
                            draw_widgets(
                                ui,
                                g,
                                state,
                                &mut actions,
                                &filter,
                                false,
                                fx,
                                Some(&mut rects),
                            );
                        }
                    });
            });
        ctx.set_transform_layer(egui::LayerId::new(egui::Order::Middle, id), transform);
    }
    for (_, r) in &mut rects {
        *r = transform.mul_rect(*r);
    }
    rects
}

/// Runs `add` with the widget's effect visuals: opacity, then offset and scale about the
/// center of what it drew. Returns the laid-out (untransformed) rectangle.
fn with_effects(ui: &mut egui::Ui, v: Visual, add: impl FnOnce(&mut egui::Ui)) -> egui::Rect {
    if v.is_identity() {
        add(ui);
        return ui.min_rect();
    }
    let layer = ui.layer_id();
    let next = |ui: &egui::Ui| {
        ui.ctx().graphics(|g| {
            g.get(layer)
                .map_or(egui::layers::ShapeIdx(0), |l| l.next_idx())
        })
    };
    let start = next(ui);
    ui.multiply_opacity(v.alpha.clamp(0.0, 1.0));
    add(ui);
    let rect = ui.min_rect();
    if v.scale != 1.0 || v.offset != [0.0, 0.0] {
        let end = next(ui);
        let scale = v.scale.max(0.0);
        let transform = egui::emath::TSTransform::new(
            rect.center().to_vec2() * (1.0 - scale) + Vec2::from(v.offset),
            scale,
        );
        ui.ctx()
            .graphics_mut(|g| g.entry(layer).transform_range(start, end, transform));
    }
    rect
}

/// A rolled number: whole for integer variables, else up to two decimals.
fn rolled(n: f64, integer: bool) -> String {
    if integer {
        format!("{}", n.round() as i64)
    } else {
        let text = format!("{n:.2}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
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

/// Whether a widget would draw anything now (empty regions are not drawn at all). The same
/// test decides when `ui.effects` show / hide effects start.
fn widget_visible(g: &Gameplay, w: &BoundWidget) -> bool {
    crate::ui_effects::widget_visible(g, w)
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

#[allow(clippy::too_many_arguments)]
fn draw_widgets(
    ui: &mut egui::Ui,
    g: &Gameplay,
    state: &mut UiState,
    actions: &mut Vec<Action>,
    filter: &dyn Fn(&BoundWidget) -> bool,
    flow: bool,
    fx: Option<&dyn EffectVisuals>,
    mut rects: Option<&mut Vec<(usize, egui::Rect)>>,
) {
    for (index, widget) in g.compiled.widgets.iter().enumerate() {
        if !filter(widget) {
            continue;
        }
        let visual = fx.map_or(Visual::IDENTITY, |f| f.visual(index));
        let mut drawn = None;
        match widget {
            BoundWidget::Text(id, binding, when) if visual.forced || g.test(when) => {
                let text = match (visual.number, binding) {
                    (Some(n), BoundText::Variable(i)) => {
                        rolled(n, matches!(g.compiled.initial[*i], Scalar::Int(_)))
                    }
                    _ => g.bound_text(binding),
                };
                let style = g.compiled.ui.as_ref().and_then(|d| d.styles.get(id));
                let (size, color, strong) = text_look(style.copied(), binding);
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
                drawn = Some(
                    ui.push_id(id, |ui| {
                        with_effects(ui, visual, |ui| {
                            ui.add(egui::Label::new(styled(text, size, color, strong)).wrap());
                        })
                    })
                    .inner,
                );
                if matches!(style, Some(TextStyle::Value)) && flow {
                    ui.add_space(10.0);
                }
            }
            BoundWidget::Button(id, text, command, when) if visual.forced || g.test(when) => {
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
                drawn = Some(
                    ui.push_id(id, |ui| {
                        with_effects(ui, visual, |ui| {
                            let response = ui.add_enabled(
                                state.playing && !state.console,
                                egui::Button::new(text)
                                    .min_size(Vec2::new(if flow { 0.0 } else { 220.0 }, 32.0)),
                            );
                            // Kept on screen only by an exit effect: shown, not usable.
                            if g.test(when) {
                                state.targets.insert(format!("button/{id}"), response.rect);
                                if response.clicked() {
                                    actions.push(Action::Gameplay(command.clone()));
                                }
                            }
                        })
                    })
                    .inner,
                );
            }
            BoundWidget::Choices(id) => {
                if flow {
                    ui.end_row();
                }
                drawn = Some(
                    ui.push_id(id, |ui| {
                        with_effects(ui, visual, |ui| choice_buttons(ui, g, state, actions, flow))
                    })
                    .inner,
                );
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
                    drawn = Some(
                        ui.push_id(id, |ui| {
                            with_effects(ui, visual, |ui| {
                                ui.add(
                                    egui::Image::new(&*texture)
                                        .max_width(ui.available_width().min(900.0))
                                        .max_height(170.0),
                                );
                            })
                        })
                        .inner,
                    );
                }
            }
            BoundWidget::Progress(id, i, max) => {
                let value = visual
                    .number
                    .unwrap_or_else(|| crate::ui_effects::number(&g.values[*i]));
                let style = g.compiled.ui.as_ref().and_then(|d| d.styles.get(id));
                drawn = Some(
                    ui.push_id(id, |ui| {
                        with_effects(ui, visual, |ui| {
                            // A styled progress also shows its number in that style.
                            if let Some(style) = style {
                                let integer = matches!(g.compiled.initial[*i], Scalar::Int(_));
                                let (size, color, strong) =
                                    text_look(Some(*style), &BoundText::Variable(*i));
                                ui.label(styled(
                                    format!(
                                        "{} / {}",
                                        rolled(value, integer),
                                        rolled(*max, integer)
                                    ),
                                    size,
                                    color,
                                    strong,
                                ));
                            }
                            ui.add(
                                egui::ProgressBar::new((value / max).clamp(0.0, 1.0) as f32)
                                    .show_percentage(),
                            );
                        })
                    })
                    .inner,
                );
            }
            _ => {}
        }
        if let (Some(rects), Some(rect)) = (rects.as_deref_mut(), drawn) {
            rects.push((index, rect));
        }
    }
}

/// (size, color, strong) of a text style; unstyled speaker lines are larger.
fn text_look(style: Option<TextStyle>, binding: &BoundText) -> (f32, Option<Color32>, bool) {
    match style {
        Some(TextStyle::Title) => (42.0, Some(Color32::from_rgb(255, 214, 102)), true),
        Some(TextStyle::Heading) => (17.0, Some(Color32::from_rgb(102, 245, 205)), true),
        Some(TextStyle::Banner) => (20.0, Some(Color32::from_rgb(255, 118, 84)), true),
        Some(TextStyle::Value) => (20.0, Some(Color32::WHITE), true),
        Some(TextStyle::Label) => (13.0, Some(Color32::from_rgb(158, 176, 182)), false),
        Some(TextStyle::Muted) => (13.0, Some(Color32::from_rgb(130, 142, 148)), false),
        None if matches!(binding, BoundText::Speaker) => (22.0, None, false),
        None => (18.0, None, false),
    }
}

fn styled(text: String, size: f32, color: Option<Color32>, strong: bool) -> RichText {
    let mut rich = RichText::new(text).size(size);
    if let Some(color) = color {
        rich = rich.color(color);
    }
    if strong {
        rich = rich.strong();
    }
    rich
}

fn choice_buttons(
    ui: &mut egui::Ui,
    g: &Gameplay,
    state: &mut UiState,
    actions: &mut Vec<Action>,
    flow: bool,
) {
    for choice in g.choices() {
        let response = ui.add_enabled(
            state.playing && !state.console,
            egui::Button::new(RichText::new(g.text(&choice.text)).size(18.0)).min_size(Vec2::new(
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
}
