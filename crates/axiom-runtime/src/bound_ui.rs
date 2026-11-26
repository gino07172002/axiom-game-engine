//! Data-bound immediate-mode presentation; clicks produce the same typed commands as replay.
use crate::{
    gameplay::Gameplay,
    ui::{Action, UiState},
    ui_effects::{EffectVisuals, Visual},
};
use anyhow::{ensure, Context, Result};
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
        // One bounded read: a file that grows after a size check cannot exceed the budget.
        let (raw, (w, h, rgba)) = axiom_core::read_bounded(
            std::path::Path::new(path),
            IMAGE_BYTES as usize,
            "UI image asset",
        )
        .and_then(|raw| decode_png(&raw).map(|decoded| (raw, decoded)))
        .with_context(|| format!("ui.document image {id}"))?;
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
/// Byte budget of one image asset file (UI images, mesh decals).
pub(crate) const IMAGE_BYTES: u64 = 4_194_304;
/// Decodes a PNG image asset to RGBA8 (at most 2048 x 2048).
pub(crate) fn decode_png(raw: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(raw));
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
    Ok((w, h, rgba))
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
    /// Layout-space clip of the editor canvas (its visible part when zoomed in).
    clip: Option<egui::Rect>,
    /// Editor canvas: layout space -> screen (the panel layers combine it with their scale).
    transform: Option<egui::emath::TSTransform>,
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
        clip: None,
        transform: None,
    };
    state
        .layout
        .clear(ctx.screen_rect(), ctx.pixels_per_point());
    draw_regions(ctx, g, state, &mut actions, &mut surface);
    draw_panels(ctx, g, state, &mut actions, &mut surface);
    let placed = |w: &BoundWidget| region_of(w).is_some() || in_panel(doc, w);
    let unplaced = g.compiled.widgets.iter().any(|w| !placed(w));
    if (!doc.regions.is_empty() || !doc.panels.is_empty()) && !unplaced {
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
                let filter = |w: &BoundWidget| !placed(w);
                if doc.flow {
                    ui.horizontal_wrapped(|ui| {
                        draw_widgets(ui, g, state, &mut actions, &filter, Flow::Wrap, fx, None)
                    });
                } else {
                    draw_widgets(ui, g, state, &mut actions, &filter, Flow::Column, fx, None);
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
        // Session controls move to the top-right region when no bottom document panel is
        // drawn: every widget is placed in a region or a panel.
        let controls = region == Region::TopRight
            && doc.session_controls
            && (!doc.regions.is_empty() || !doc.panels.is_empty())
            && g.compiled
                .widgets
                .iter()
                .all(|w| region_of(w).is_some() || in_panel(doc, w));
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
        let clip = s.clip;
        let shown = area.show(ctx, |ui| {
            if let Some(clip) = clip {
                ui.set_clip_rect(ui.clip_rect().intersect(clip));
            }
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
                                Flow::Wrap,
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
                            Flow::Column,
                            fx,
                            rects.as_deref_mut(),
                        );
                    }
                });
        });
        if !canvas {
            state.layout.regions.push((region, shown.response.rect));
        }
    }
}

/// The editor's UI canvas: the document's regions and widgets drawn by the same code as the
/// game HUD, laid out in a virtual `size` screen (with `top` pixels of session chrome) and
/// scaled into `rect`, painted only inside `clip` (screen space). Nothing in it is
/// interactive; button clicks never run commands.
/// Returns each drawn widget's rectangle in screen space (widget index, rect).
#[allow(clippy::too_many_arguments)]
pub fn draw_canvas(
    ctx: &egui::Context,
    rect: egui::Rect,
    size: Vec2,
    top: f32,
    g: &Gameplay,
    state: &mut UiState,
    fx: Option<&dyn EffectVisuals>,
    clip: egui::Rect,
) -> Vec<(usize, egui::Rect)> {
    let Some(doc) = &g.compiled.ui else {
        return vec![];
    };
    let scale = (rect.width() / size.x)
        .min(rect.height() / size.y)
        .max(0.05);
    let screen = egui::Rect::from_min_size(rect.min, size);
    let transform = egui::emath::TSTransform::new(rect.min.to_vec2() * (1.0 - scale), scale);
    let layout_clip = transform.inverse().mul_rect(clip);
    let mut actions = vec![];
    let mut rects = vec![];
    state.targets.clear();
    let mut surface = Surface {
        area: Some(screen),
        top,
        salt: "ui_canvas_region",
        fx,
        rects: Some(&mut rects),
        clip: Some(layout_clip),
        transform: Some(transform),
    };
    draw_regions(ctx, g, state, &mut actions, &mut surface);
    // Panel layers get the canvas transform times the document's reference scale.
    draw_panels(ctx, g, state, &mut actions, &mut surface);
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
    let placed = |w: &BoundWidget| region_of(w).is_some() || in_panel(doc, w);
    if (doc.regions.is_empty() && doc.panels.is_empty())
        || g.compiled.widgets.iter().any(|w| !placed(w))
    {
        let id = egui::Id::new("ui_canvas_document");
        egui::Area::new(id)
            .constrain_to(screen)
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, 0.0])
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(layout_clip));
                ui.style_mut().interaction.selectable_labels = false;
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(size.x - 16.0);
                        if doc.session_controls {
                            session_controls(ui, doc, g, state, &mut actions);
                        }
                        let filter = |w: &BoundWidget| !placed(w);
                        if doc.flow {
                            ui.horizontal_wrapped(|ui| {
                                draw_widgets(
                                    ui,
                                    g,
                                    state,
                                    &mut actions,
                                    &filter,
                                    Flow::Wrap,
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
                                Flow::Column,
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
    axiom_core::gameplay::bound_widget_id(w)
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
        // Engine label is developer chrome; the shipped player keeps only the game's controls.
        if !state.player {
            ui.label(
                RichText::new("AXIOM / NARRATIVE")
                    .small()
                    .color(Color32::from_rgb(102, 245, 205)),
            );
        }
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
    mode: Flow,
    fx: Option<&dyn EffectVisuals>,
    mut rects: Option<&mut Vec<(usize, egui::Rect)>>,
) {
    let flow = mode == Flow::Wrap;
    // Natural-size buttons (no 220 px column minimum) in wrapping rows and panel cells.
    let compact = mode != Flow::Column;
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
                if let Some(class) = text_class(g, id) {
                    drawn = Some(
                        ui.push_id(id, |ui| {
                            with_effects(ui, visual, |ui| {
                                themed_text(ui, text, class);
                            })
                        })
                        .inner,
                    );
                } else {
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
                            let enabled = state.playing && !state.console;
                            let min = Vec2::new(if compact { 0.0 } else { 220.0 }, 32.0);
                            let response = match button_class(g, id) {
                                Some(class) => {
                                    themed_button(ui, g, state, text, class, enabled, min)
                                }
                                None => {
                                    ui.add_enabled(enabled, egui::Button::new(text).min_size(min))
                                }
                            };
                            // Kept on screen only by an exit effect: shown, not usable.
                            if g.test(when) {
                                state.target(format!("button/{id}"), response.rect);
                                if response.clicked() {
                                    actions.push(Action::Gameplay(command.clone()));
                                }
                            }
                        })
                    })
                    .inner,
                );
            }
            BoundWidget::Slider(s) if visual.forced || g.test(&s.when) => {
                if flow {
                    // Label + track + value box; wrap first like buttons do.
                    let label = ui
                        .painter()
                        .layout_no_wrap(
                            g.text(&s.text),
                            egui::TextStyle::Body.resolve(ui.style()),
                            ui.visuals().text_color(),
                        )
                        .size()
                        .x;
                    let width = label + ui.spacing().slider_width + 80.0;
                    if width > ui.available_size_before_wrap().x {
                        ui.end_row();
                    }
                }
                drawn = Some(
                    ui.push_id(&s.id, |ui| {
                        with_effects(ui, visual, |ui| slider(ui, g, s, state, actions))
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
                        with_effects(ui, visual, |ui| {
                            choice_buttons(ui, g, state, actions, compact)
                        })
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

/// A slider: dragging only moves a local draft; the release (or a keyboard / click change)
/// sends one `slide` command, so a drag never floods the simulation with commands.
fn slider(
    ui: &mut egui::Ui,
    g: &Gameplay,
    s: &axiom_core::gameplay::BoundSlider,
    state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let committed = g.slider_value(s);
    let mut value = state.slider_drafts.get(&s.id).copied().unwrap_or(committed);
    let label = g.text(&s.text);
    ui.horizontal(|ui| {
        ui.label(label);
        let mut widget = egui::Slider::new(&mut value, s.min..=s.max);
        if let Some(step) = s.step {
            widget = widget.step_by(step);
        }
        if s.integer {
            widget = widget.integer();
        }
        let response = ui.add_enabled(state.playing && !state.console, widget);
        // Kept on screen only by an exit effect: shown, not usable.
        if !g.test(&s.when) {
            return;
        }
        state.target(format!("slider/{}", s.id), response.rect);
        let release = response.drag_stopped();
        if response.dragged() && !release {
            state.slider_drafts.insert(s.id.clone(), value);
            return;
        }
        let draft = state.slider_drafts.remove(&s.id);
        if (release || response.changed()) && draft.unwrap_or(value) != committed {
            let value = draft.unwrap_or(value);
            actions.push(Action::Gameplay(RuntimeCommand::Slide {
                widget: s.id.clone(),
                value,
            }));
        }
    });
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
        state.target(format!("choice/{}", choice.id), response.rect);
        if response.clicked() {
            actions.push(Action::Gameplay(RuntimeCommand::Choose {
                node: g.compiled.nodes[g.node.unwrap()].id.clone(),
                choice: choice.id.clone(),
            }));
        }
    }
}

// ---------------------------------------------------------------------------------------
// ui.document v2 panels (plan M1b): anchored panels with a panel style (fill, corner,
// padding) and an optional nine-slice frame image drawn behind their widgets.
// ---------------------------------------------------------------------------------------

/// Whether a widget sits in one of the document's panels.
fn in_panel(doc: &axiom_core::gameplay::UiDocument, w: &BoundWidget) -> bool {
    let id = widget_id(w);
    doc.panels
        .iter()
        .any(|p| p.children.iter().any(|c| c == id))
}

/// Nine quads of `size`-pixel texture `tex` over `rect`: corners keep their pixel size
/// (shrunk proportionally when the rect is smaller than two corners), edges and centre
/// stretch.
pub(crate) fn nine_slice(
    rect: egui::Rect,
    tex: egui::TextureId,
    size: [usize; 2],
    slice: [u16; 4],
    center: axiom_core::ui_theme::FrameCenter,
) -> egui::epaint::Mesh {
    use axiom_core::ui_theme::FrameCenter;
    let [w, h] = [size[0].max(1) as f32, size[1].max(1) as f32];
    let [l, t, r, b] = slice.map(f32::from);
    // Margins on screen: the source pixels, shrunk if the rect cannot fit both sides.
    let fx = (rect.width() / (l + r).max(1.0)).min(1.0);
    let fy = (rect.height() / (t + b).max(1.0)).min(1.0);
    let xs = [
        rect.left(),
        rect.left() + l * fx,
        rect.right() - r * fx,
        rect.right(),
    ];
    let ys = [
        rect.top(),
        rect.top() + t * fy,
        rect.bottom() - b * fy,
        rect.bottom(),
    ];
    let us = [0.0, (l / w).min(1.0), (1.0 - r / w).max(0.0), 1.0];
    let vs = [0.0, (t / h).min(1.0), (1.0 - b / h).max(0.0), 1.0];
    let mut mesh = egui::epaint::Mesh::with_texture(tex);
    let uv = |i: usize, j: usize| {
        egui::Rect::from_min_max(egui::pos2(us[i], vs[j]), egui::pos2(us[i + 1], vs[j + 1]))
    };
    for j in 0..3 {
        for i in 0..3 {
            let quad = egui::Rect::from_min_max(
                egui::pos2(xs[i], ys[j]),
                egui::pos2(xs[i + 1], ys[j + 1]),
            );
            if quad.width() <= 0.0 || quad.height() <= 0.0 {
                continue;
            }
            if (i, j) != (1, 1) {
                mesh.add_rect_with_uv(quad, uv(i, j), Color32::WHITE);
                continue;
            }
            // The centre: stretched, left empty, or tiled at its source size.
            let tile = egui::vec2(w - l - r, h - t - b);
            let count =
                (quad.width() / tile.x.max(1.0)).ceil() * (quad.height() / tile.y.max(1.0)).ceil();
            match center {
                FrameCenter::Hidden => {}
                FrameCenter::Tile if tile.x >= 1.0 && tile.y >= 1.0 && count <= 1024.0 => {
                    let mut y = quad.top();
                    while y < quad.bottom() {
                        let th = tile.y.min(quad.bottom() - y);
                        let mut x = quad.left();
                        while x < quad.right() {
                            let tw = tile.x.min(quad.right() - x);
                            let cell = uv(1, 1);
                            let part = egui::Rect::from_min_size(
                                cell.min,
                                egui::vec2(cell.width() * tw / tile.x, cell.height() * th / tile.y),
                            );
                            mesh.add_rect_with_uv(
                                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(tw, th)),
                                part,
                                Color32::WHITE,
                            );
                            x += tw;
                        }
                        y += th;
                    }
                }
                // Stretch, or a tile too small / too many to draw (budget: 1024 tiles).
                _ => mesh.add_rect_with_uv(quad, uv(1, 1), Color32::WHITE),
            }
        }
    }
    mesh
}

/// How `draw_widgets` lays widgets out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// One per line; buttons at least 220 px wide (regions, panel columns, the document).
    Column,
    /// Left to right, wrapping (flow regions and flow documents).
    Wrap,
    /// One cell per widget at its natural size (panel rows and grids).
    Cells,
}

/// Screen rectangles of the game HUD drawn last frame (plan M3), reported by `observe` as
/// `ui_layout`. Buffers are reused frame to frame.
#[derive(Default)]
pub struct UiLayout {
    /// The window the HUD was drawn in (egui points); none before the first frame.
    pub screen: Option<egui::Rect>,
    /// Window pixels per egui point (display scaling); `observe` reports window pixels.
    pub pixels_per_point: f32,
    /// Panel layer scale (`theme.reference`), 1 without a reference.
    pub scale: f32,
    pub regions: Vec<(Region, egui::Rect)>,
    /// (index in `ui.document.panels`, rect), drawn panels only.
    pub panels: Vec<(usize, egui::Rect)>,
    /// (index in the compiled widgets, rect), widgets drawn inside panels.
    pub widgets: Vec<(usize, egui::Rect)>,
}
impl UiLayout {
    fn clear(&mut self, screen: egui::Rect, pixels_per_point: f32) {
        self.screen = Some(screen);
        self.pixels_per_point = pixels_per_point;
        self.scale = 1.0;
        self.regions.clear();
        self.panels.clear();
        self.widgets.clear();
    }
}

/// What the panel drawing helpers share.
struct PanelCtx<'a> {
    g: &'a Gameplay,
    doc: &'a axiom_core::gameplay::UiDocument,
    fx: Option<&'a dyn EffectVisuals>,
    /// The layout space: the reference screen (or the window) in panel-layer coordinates.
    virt: egui::Rect,
}

fn panel_index(doc: &axiom_core::gameplay::UiDocument, id: &str) -> Option<usize> {
    doc.panels.iter().position(|p| p.id == id)
}

/// A root panel is not any panel's child.
fn is_root(doc: &axiom_core::gameplay::UiDocument, i: usize) -> bool {
    let id = &doc.panels[i].id;
    !doc.panels
        .iter()
        .any(|p| p.children.iter().any(|c| c == id))
}

/// Whether any widget in the panel (or its child panels) would draw.
fn panel_visible(pc: &PanelCtx, i: usize, depth: usize) -> bool {
    depth <= axiom_core::ui_theme::MAX_PANEL_DEPTH
        && pc.doc.panels[i]
            .children
            .iter()
            .any(|c| match panel_index(pc.doc, c) {
                Some(j) => j != i && panel_visible(pc, j, depth + 1),
                None => {
                    pc.g.compiled
                        .widgets
                        .iter()
                        .enumerate()
                        .find(|(_, w)| widget_id(w) == c)
                        .is_some_and(|(k, w)| {
                            widget_visible(pc.g, w) || pc.fx.is_some_and(|f| f.visual(k).forced)
                        })
                }
            })
}

fn panel_layer(salt: &'static str, id: &str) -> egui::LayerId {
    egui::LayerId::new(egui::Order::Middle, egui::Id::new((salt, "panel", id)))
}

fn draw_panels(
    ctx: &egui::Context,
    g: &Gameplay,
    state: &mut UiState,
    actions: &mut Vec<Action>,
    s: &mut Surface,
) {
    use egui::emath::TSTransform;
    let Some(doc) = &g.compiled.ui else {
        return;
    };
    if doc.panels.is_empty() {
        return;
    }
    let theme = doc.theme.as_ref();
    let screen = s.area.unwrap_or_else(|| ctx.screen_rect());
    let scaled = theme.is_some_and(|t| t.reference.is_some());
    let scale = theme.map_or(1.0, |t| t.scale_factor([screen.width(), screen.height()]));
    // Panels lay out in `virt` (the reference screen at the window's origin); one transform
    // scales the whole panel layer onto the window (plan Spike S2).
    let virt = egui::Rect::from_min_size(screen.min, screen.size() / scale);
    let ui_t = TSTransform::new(screen.min.to_vec2() * (1.0 - scale), scale);
    let layer_t = s.transform.map_or(ui_t, |canvas| canvas * ui_t);
    let pc = PanelCtx {
        g,
        doc,
        fx: s.fx,
        virt,
    };
    // Root panels by z (stable: ties keep list order). Buffers are reused every frame.
    let mut order = std::mem::take(&mut state.panel_order);
    order.clear();
    order.extend((0..doc.panels.len()).filter(|&i| is_root(doc, i)));
    order.sort_by_key(|&i| doc.panels[i].z.unwrap_or(0));
    let mut widgets = std::mem::take(&mut state.layout_widgets);
    widgets.clear();
    let mut panels = std::mem::take(&mut state.layout_panels);
    panels.clear();
    for &i in &order {
        let panel = &doc.panels[i];
        if !panel_visible(&pc, i, 1) {
            continue;
        }
        let (align, _, _, _) = anchor(panel.anchor);
        let mut offset = panel.offset;
        if matches!(
            panel.anchor,
            Region::TopLeft | Region::Top | Region::TopRight
        ) {
            offset[1] += s.top / scale;
        }
        let layer = panel_layer(s.salt, &panel.id);
        let mut area = egui::Area::new(layer.id).anchor(align, offset);
        if s.area.is_some() {
            area = area.constrain_to(virt).interactable(false);
        } else if scaled {
            area = area.constrain_to(virt);
        }
        let clip = s.clip.map(|c| ui_t.inverse().mul_rect(c));
        let canvas = s.area.is_some();
        state.target_transform = layer_t;
        area.show(ctx, |ui| {
            if let Some(clip) = clip {
                ui.set_clip_rect(ui.clip_rect().intersect(clip));
            }
            if canvas {
                ui.style_mut().interaction.selectable_labels = false;
            }
            panel_body(ui, &pc, state, actions, &mut widgets, &mut panels, i, 1);
        });
        state.target_transform = TSTransform::IDENTITY;
        // Identity removes the layer transform (an unscaled game HUD stays untransformed).
        ctx.set_transform_layer(layer, layer_t);
    }
    if doc.panels.iter().any(|p| p.z.is_some()) {
        for &i in &order {
            ctx.move_to_top(panel_layer(s.salt, &doc.panels[i].id));
        }
    }
    if s.area.is_none() {
        state.layout.scale = scale;
        state
            .layout
            .panels
            .extend(panels.iter().map(|&(i, r)| (i, ui_t.mul_rect(r))));
        state
            .layout
            .widgets
            .extend(widgets.iter().map(|&(i, r)| (i, ui_t.mul_rect(r))));
    }
    if let Some(rects) = s.rects.as_deref_mut() {
        rects.extend(widgets.iter().map(|&(i, r)| (i, ui_t.mul_rect(r))));
    }
    state.panel_order = order;
    state.layout_widgets = widgets;
    state.layout_panels = panels;
}

/// One panel: background slot, children in the panel's layout, then the style fill and
/// nine-slice frame filled in behind them at the size the content took.
#[allow(clippy::too_many_arguments)]
fn panel_body(
    ui: &mut egui::Ui,
    pc: &PanelCtx,
    state: &mut UiState,
    actions: &mut Vec<Action>,
    widgets: &mut Vec<(usize, egui::Rect)>,
    panels: &mut Vec<(usize, egui::Rect)>,
    i: usize,
    depth: usize,
) {
    use axiom_core::ui_theme::PanelLayout;
    let panel = &pc.doc.panels[i];
    let theme = pc.doc.theme.as_ref();
    let style = panel
        .style
        .as_ref()
        .and_then(|c| theme.and_then(|t| t.panel.get(c)));
    let frame = style
        .and_then(|s| s.frame.as_ref())
        .and_then(|f| theme.and_then(|t| t.frames.get(f)));
    // The frame's texture (the same cache the image widget uses).
    let texture = frame.and_then(|f| pc.g.images.get(&f.image)).map(|p| {
        let t = state.textures.entry(p.key.clone()).or_insert_with(|| {
            ui.ctx()
                .load_texture(&p.key, p.image.clone(), egui::TextureOptions::LINEAR)
        });
        (t.id(), p.image.size)
    });
    let (_, _, default_width, _) = anchor(panel.anchor);
    // Background first (under the widgets), filled in once the panel's size is known.
    let slot = ui.painter().add(egui::Shape::Noop);
    let padding = style.and_then(|s| s.padding).unwrap_or(10.0);
    let inner = egui::Frame::new().inner_margin(padding).show(ui, |ui| {
        match panel.width.map(|l| l.resolve(pc.virt.width())) {
            Some(w) => ui.set_width((w - 2.0 * padding).max(0.0)),
            None => ui.set_max_width(default_width),
        }
        if let Some(h) = panel.height.map(|l| l.resolve(pc.virt.height())) {
            let h = (h - 2.0 * padding).max(0.0);
            ui.set_min_height(h);
            ui.set_max_height(h);
        }
        if let Some(gap) = panel.gap {
            ui.spacing_mut().item_spacing = Vec2::splat(gap);
        }
        let mut child = |ui: &mut egui::Ui, c: &str, mode: Flow| {
            if let Some(j) = panel_index(pc.doc, c) {
                if j != i && panel_visible(pc, j, depth + 1) {
                    ui.push_id(c, |ui| {
                        panel_body(ui, pc, state, actions, widgets, panels, j, depth + 1)
                    });
                }
                return;
            }
            let only = |w: &BoundWidget| widget_id(w) == c;
            draw_widgets(
                ui,
                pc.g,
                state,
                actions,
                &only,
                mode,
                pc.fx,
                Some(&mut *widgets),
            );
        };
        match panel.layout {
            PanelLayout::Column => {
                for c in &panel.children {
                    child(ui, c, Flow::Column);
                }
            }
            PanelLayout::Row => {
                ui.horizontal(|ui| {
                    for c in &panel.children {
                        child(ui, c, Flow::Cells);
                    }
                });
            }
            PanelLayout::Grid => {
                let columns = usize::from(panel.columns.unwrap_or(2).max(1));
                let spacing = ui.spacing().item_spacing;
                egui::Grid::new(("panel_grid", panel.id.as_str()))
                    .spacing(spacing)
                    .show(ui, |ui| {
                        for (k, c) in panel.children.iter().enumerate() {
                            child(ui, c, Flow::Cells);
                            if (k + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            }
            PanelLayout::Stack => {
                let area = egui::Rect::from_min_size(ui.cursor().min, ui.available_size());
                for c in &panel.children {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                        child(ui, c, Flow::Column)
                    });
                }
            }
        }
    });
    let rect = inner.response.rect;
    panels.push((i, rect));
    let mut background = vec![];
    if let Some([r, gr, b, a]) = style.and_then(|s| s.fill) {
        let corner = style.and_then(|s| s.corner).unwrap_or(0.0);
        background.push(egui::Shape::rect_filled(
            rect,
            corner,
            Color32::from_rgba_unmultiplied(r, gr, b, a),
        ));
    }
    if let (Some(f), Some((tex, size))) = (frame, texture) {
        background.push(egui::Shape::mesh(nine_slice(
            rect, tex, size, f.slice, f.center,
        )));
    }
    ui.painter().set(slot, egui::Shape::Vec(background));
}

/// `observe` `ui_layout` (plan M3): every drawn region, panel and panel widget as a
/// rect [x, y, w, h] in window pixels (egui points times the display scaling), with `ui_overlap` / `ui_offscreen` diagnostics (at most 32).
/// Widgets that share a stack panel may overlap by design and are not reported.
pub fn layout_observation(g: &Gameplay, l: &UiLayout) -> Option<serde_json::Value> {
    use serde_json::json;
    let doc = g.compiled.ui.as_ref()?;
    let screen = l.screen?;
    if l.panels.is_empty() && l.regions.is_empty() {
        return None;
    }
    let ppp = if l.pixels_per_point > 0.0 {
        l.pixels_per_point
    } else {
        1.0
    };
    let round = |v: f32| (v * ppp * 10.0).round() / 10.0;
    let rect = |r: egui::Rect| {
        [
            round(r.min.x),
            round(r.min.y),
            round(r.width()),
            round(r.height()),
        ]
    };
    let parent = |id: &str| {
        doc.panels
            .iter()
            .find(|p| p.children.iter().any(|c| c == id))
    };
    let region_name = |r: Region| {
        serde_json::to_value(r)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    };
    let mut named: Vec<(String, egui::Rect)> = vec![];
    let mut diagnostics = vec![];
    let regions: Vec<_> = l
        .regions
        .iter()
        .map(|&(r, at)| {
            named.push((format!("region:{}", region_name(r)), at));
            json!({"region": region_name(r), "rect": rect(at)})
        })
        .collect();
    let panels: Vec<_> = l
        .panels
        .iter()
        .map(|&(i, at)| {
            let id = &doc.panels[i].id;
            named.push((format!("panel:{id}"), at));
            match parent(id) {
                Some(p) => json!({"id": id, "parent": p.id, "rect": rect(at)}),
                None => json!({"id": id, "rect": rect(at)}),
            }
        })
        .collect();
    let widget_names: Vec<&str> = l
        .widgets
        .iter()
        .map(|&(i, _)| widget_id(&g.compiled.widgets[i]))
        .collect();
    let widgets: Vec<_> = l
        .widgets
        .iter()
        .zip(&widget_names)
        .map(|(&(_, at), id)| {
            named.push((format!("widget:{id}"), at));
            json!({"id": id, "panel": parent(id).map(|p| p.id.as_str()), "rect": rect(at)})
        })
        .collect();
    let mut push = |d: serde_json::Value| {
        if diagnostics.len() < 32 {
            diagnostics.push(d);
        }
    };
    for (name, at) in &named {
        if !screen.expand(0.5).contains_rect(*at) {
            push(json!({"code": "ui_offscreen", "id": name, "rect": rect(*at)}));
        }
    }
    let overlap = |a: egui::Rect, b: egui::Rect| {
        let i = a.intersect(b);
        i.is_positive() && i.area() > 1.0
    };
    // Widgets against widgets (not two children of one stack panel).
    for (a, (&(_, ra), ida)) in l.widgets.iter().zip(&widget_names).enumerate() {
        for (&(_, rb), idb) in l.widgets.iter().zip(&widget_names).skip(a + 1) {
            let stacked = parent(ida).is_some_and(|p| {
                p.layout == axiom_core::ui_theme::PanelLayout::Stack
                    && p.children.iter().any(|c| c == idb)
            });
            if !stacked && overlap(ra, rb) {
                push(
                    json!({"code": "ui_overlap", "ids": [format!("widget:{ida}"), format!("widget:{idb}")]}),
                );
            }
        }
    }
    // Root panels and regions against each other.
    let roots: Vec<(String, egui::Rect)> = l
        .regions
        .iter()
        .map(|&(r, at)| (format!("region:{}", region_name(r)), at))
        .chain(
            l.panels
                .iter()
                .filter(|&&(i, _)| is_root(doc, i))
                .map(|&(i, at)| (format!("panel:{}", doc.panels[i].id), at)),
        )
        .collect();
    for (a, (na, ra)) in roots.iter().enumerate() {
        for (nb, rb) in roots.iter().skip(a + 1) {
            if overlap(*ra, *rb) {
                push(json!({"code": "ui_overlap", "ids": [na, nb]}));
            }
        }
    }
    Some(json!({
        "screen": [round(screen.width()), round(screen.height())],
        // Window pixels per reference pixel.
        "scale": (l.scale * ppp * 1000.0).round() / 1000.0,
        "regions": regions,
        "panels": panels,
        "widgets": widgets,
        "diagnostics": diagnostics,
    }))
}

#[cfg(test)]
mod panel_tests {
    use super::*;

    fn bounds(m: &egui::epaint::Mesh) -> (Vec<f32>, Vec<f32>) {
        let mut xs: Vec<f32> = m.vertices.iter().map(|v| v.pos.x).collect();
        let mut us: Vec<f32> = m.vertices.iter().map(|v| v.uv.x).collect();
        xs.sort_by(f32::total_cmp);
        xs.dedup();
        us.sort_by(f32::total_cmp);
        us.dedup();
        (xs, us)
    }

    #[test]
    fn nine_slice_keeps_corner_pixels_and_maps_the_slice_uvs() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(600.0, 200.0));
        let m = nine_slice(
            rect,
            egui::TextureId::Managed(7),
            [96, 96],
            [32, 24, 16, 8],
            Default::default(),
        );
        assert_eq!(m.vertices.len(), 36);
        let (xs, us) = bounds(&m);
        assert_eq!(
            xs,
            [100.0, 132.0, 684.0, 700.0],
            "corners keep their pixel width"
        );
        assert_eq!(us, [0.0, 32.0 / 96.0, 1.0 - 16.0 / 96.0, 1.0]);
        assert_eq!(m.texture_id, egui::TextureId::Managed(7));
    }

    #[test]
    fn nine_slice_shrinks_corners_in_a_rect_smaller_than_both() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(32.0, 200.0));
        let m = nine_slice(
            rect,
            egui::TextureId::Managed(1),
            [96, 96],
            [32, 32, 32, 32],
            Default::default(),
        );
        let (xs, _) = bounds(&m);
        // 64 px of corners in 32 px: halved, and the empty middle column is skipped.
        assert_eq!(xs, [0.0, 16.0, 32.0]);
        assert!(m.vertices.iter().all(|v| v.pos.x >= 0.0 && v.pos.x <= 32.0));
    }
}

#[cfg(test)]
mod frame_center_tests {
    use super::*;
    use axiom_core::ui_theme::FrameCenter;

    #[test]
    fn a_hidden_centre_draws_only_the_border() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 200.0));
        let m = nine_slice(
            rect,
            egui::TextureId::Managed(1),
            [96, 96],
            [32; 4],
            FrameCenter::Hidden,
        );
        assert_eq!(m.vertices.len(), 8 * 4, "8 border quads");
        let inside = egui::Rect::from_min_max(egui::pos2(33.0, 33.0), egui::pos2(267.0, 167.0));
        assert!(m.vertices.iter().all(|v| !inside.contains(v.pos)));
    }

    #[test]
    fn a_tiled_centre_repeats_the_source_centre_with_partial_edge_tiles() {
        // Centre 236 x 136 px, source centre 32 x 32 px: 8 x 5 tiles (last ones partial).
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 200.0));
        let m = nine_slice(
            rect,
            egui::TextureId::Managed(1),
            [96, 96],
            [32; 4],
            FrameCenter::Tile,
        );
        assert_eq!(m.vertices.len(), (8 + 8 * 5) * 4);
        // The last column maps only 12 of the 32 source pixels (only a tile can).
        let uv_w: Vec<f32> = m
            .vertices
            .chunks(4)
            .map(|q| {
                q.iter().map(|v| v.uv.x).fold(f32::MIN, f32::max)
                    - q.iter().map(|v| v.uv.x).fold(f32::MAX, f32::min)
            })
            .collect();
        assert!(uv_w
            .iter()
            .any(|w| (w - (236.0 - 7.0 * 32.0) / 96.0).abs() < 1e-5));
    }

    #[test]
    fn a_tiled_centre_over_budget_falls_back_to_one_stretched_quad() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(4000.0, 4000.0));
        let m = nine_slice(
            rect,
            egui::TextureId::Managed(1),
            [66, 66],
            [32; 4],
            FrameCenter::Tile,
        );
        assert_eq!(
            m.vertices.len(),
            9 * 4,
            "2 px tiles would be 3.9M quads: stretched instead"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Theme classes (plan M2): text classes and button classes with per-state looks.
// ---------------------------------------------------------------------------------------

fn rgba(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// A theme frame and its texture (the same cache as image widgets and panels).
fn frame_texture<'a>(
    ctx: &egui::Context,
    g: &'a Gameplay,
    state: &mut UiState,
    id: &str,
) -> Option<(
    &'a axiom_core::ui_theme::UiFrame,
    egui::TextureId,
    [usize; 2],
)> {
    let frame = g.compiled.ui.as_ref()?.theme.as_ref()?.frames.get(id)?;
    let p = g.images.get(&frame.image)?;
    let texture = state
        .textures
        .entry(p.key.clone())
        .or_insert_with(|| ctx.load_texture(&p.key, p.image.clone(), egui::TextureOptions::LINEAR));
    Some((frame, texture.id(), p.image.size))
}

fn text_class<'a>(g: &'a Gameplay, id: &str) -> Option<&'a axiom_core::ui_theme::TextClass> {
    let doc = g.compiled.ui.as_ref()?;
    doc.theme.as_ref()?.text.get(doc.classes.get(id)?)
}

fn button_class<'a>(g: &'a Gameplay, id: &str) -> Option<&'a axiom_core::ui_theme::ButtonClass> {
    let doc = g.compiled.ui.as_ref()?;
    doc.theme.as_ref()?.button.get(doc.classes.get(id)?)
}

/// A text widget in a theme text class: its size and colour, wrapped to the width, with the
/// optional drop shadow painted 2 px down-right underneath.
fn themed_text(
    ui: &mut egui::Ui,
    text: String,
    class: &axiom_core::ui_theme::TextClass,
) -> egui::Response {
    let size = class.size.unwrap_or(15.0);
    let color = class
        .color
        .map(rgba)
        .unwrap_or_else(|| ui.visuals().text_color());
    let galley = ui.painter().layout(
        text,
        egui::FontId::proportional(size),
        Color32::PLACEHOLDER,
        ui.available_width(),
    );
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        if let Some(shadow) = class.shadow {
            ui.painter()
                .galley(rect.min + Vec2::splat(2.0), galley.clone(), rgba(shadow));
        }
        ui.painter().galley(rect.min, galley, color);
    }
    response
}

/// Which look a themed button shows now.
pub(crate) fn button_state(enabled: bool, response: &egui::Response) -> &'static str {
    if !enabled {
        "disabled"
    } else if response.is_pointer_button_down_on() {
        "pressed"
    } else if response.hovered() {
        "hover"
    } else {
        "normal"
    }
}

/// A button in a theme button class: the state's fill and nine-slice frame behind a centred
/// label; unset state fields fall back to `normal`. Clicks behave like the plain button.
fn themed_button(
    ui: &mut egui::Ui,
    g: &Gameplay,
    state: &mut UiState,
    text: String,
    class: &axiom_core::ui_theme::ButtonClass,
    enabled: bool,
    min: Vec2,
) -> egui::Response {
    let padding = class.padding.unwrap_or(10.0);
    let galley = ui.painter().layout_no_wrap(
        text,
        egui::FontId::proportional(class.size.unwrap_or(15.0)),
        Color32::PLACEHOLDER,
    );
    let desired = (galley.size() + Vec2::splat(2.0 * padding)).max(min);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(desired, sense);
    if ui.is_rect_visible(rect) {
        let name = button_state(enabled, &response);
        let look = class
            .state(name)
            .map_or_else(|| class.normal.clone(), |l| l.over(&class.normal));
        if let Some(fill) = look.fill {
            ui.painter().rect_filled(rect, 4.0, rgba(fill));
        }
        if let Some((frame, tex, size)) = look
            .frame
            .as_deref()
            .and_then(|f| frame_texture(ui.ctx(), g, state, f))
        {
            ui.painter().add(egui::Shape::mesh(nine_slice(
                rect,
                tex,
                size,
                frame.slice,
                frame.center,
            )));
        }
        let color = look.text.map(rgba).unwrap_or_else(|| {
            if enabled {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            }
        });
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, color);
    }
    response
}
