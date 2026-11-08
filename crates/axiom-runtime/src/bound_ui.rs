//! Data-bound immediate-mode presentation; clicks produce the same typed commands as replay.
use crate::{
    gameplay::Gameplay,
    ui::{Action, UiState},
};
use anyhow::{ensure, Result};
use axiom_core::gameplay::{BoundText, BoundWidget, RuntimeCommand, Scalar};
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
    egui::TopBottomPanel::bottom("bound_document")
        .resizable(false)
        .show(ctx, |ui| {
            ui.set_max_height(ctx.screen_rect().height() * 0.87);
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                if doc.session_controls {
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
                if doc.flow {
                    ui.horizontal_wrapped(|ui| draw_widgets(ui, g, state, &mut actions));
                } else {
                    draw_widgets(ui, g, state, &mut actions);
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

fn draw_widgets(ui: &mut egui::Ui, g: &Gameplay, state: &mut UiState, actions: &mut Vec<Action>) {
    for widget in &g.compiled.widgets {
        match widget {
            BoundWidget::Text(id, binding, when) if g.test(when) => {
                let text = g.bound_text(binding);
                let size = if matches!(binding, BoundText::Speaker) {
                    22.0
                } else {
                    18.0
                };
                if g.compiled.ui.as_ref().is_some_and(|u| u.flow) {
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
                ui.push_id(id, |ui| {
                    let label = egui::Label::new(RichText::new(text).size(size));
                    ui.add(label.wrap());
                });
            }
            BoundWidget::Button(id, text, command, when) if g.test(when) => {
                let text = g.text(text);
                let flow = g.compiled.ui.as_ref().is_some_and(|u| u.flow);
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
                let flow = g.compiled.ui.as_ref().is_some_and(|doc| doc.flow);
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
