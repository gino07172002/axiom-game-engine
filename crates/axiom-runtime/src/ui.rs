use egui::{Align2, Color32, RichText, Stroke, Vec2};

const MINT: Color32 = Color32::from_rgb(102, 245, 205);
const WHITE: Color32 = Color32::from_rgb(231, 241, 240);

#[derive(Default)]
pub struct UiState {
    /// Current logical widget targets, usable by accessibility and native UI tests.
    pub targets: std::collections::BTreeMap<String, egui::Rect>,
    pub textures: std::collections::BTreeMap<String, egui::TextureHandle>,
    pub numeric_drafts: std::collections::BTreeMap<String, f64>,
    /// `ui.document` sliders: the value while a drag is held (sent once on release) and the
    /// last value sent by sliders without a variable (shown until the next change).
    pub slider_drafts: std::collections::BTreeMap<String, f64>,
    pub slider_values: std::collections::BTreeMap<String, f64>,
    pub playing: bool,
    pub console: bool,
    /// Shipped-game chrome (`RunMode::Play { player: Some(_) }`): no developer bar or console.
    pub player: bool,
    pub command: String,
    pub output: Vec<String>,
    pub fps: f32,
    pub gpu: String,
    pub port: u16,
}
pub enum Action {
    Gameplay(axiom_core::gameplay::RuntimeCommand),
    Save,
    Load,
    Start,
    Resume,
    Restart,
    Quit,
    Apply(String),
}

pub fn configure(ctx: &egui::Context) {
    // Windows CJK fallback; never redistribute proprietary system font files. Read once per
    // process: a second context (the editor's Game tab) shares the same font bytes.
    static FONTS: std::sync::OnceLock<Option<egui::FontDefinitions>> = std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let font = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("Fonts/msjh.ttc"))
            .and_then(|p| std::fs::read(p).ok())?;
        // Borrowed bytes: epaint copies owned font data into every context's atlas; the
        // process keeps one copy for all of them.
        let font: &'static [u8] = Box::leak(font.into_boxed_slice());
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("cjk".into(), egui::FontData::from_static(font).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push("cjk".into());
        Some(fonts)
    });
    if let Some(fonts) = fonts {
        ctx.set_fonts(fonts.clone());
    }
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::from_rgb(12, 22, 27);
    style.visuals.window_fill = Color32::from_rgba_unmultiplied(12, 22, 27, 248);
    style.visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(55, 79, 83));
    style.visuals.override_text_color = Some(WHITE);
    style.visuals.selection.bg_fill = Color32::from_rgb(32, 95, 82);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(27, 45, 48);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(39, 77, 69);
    style.spacing.item_spacing = Vec2::new(12.0, 12.0);
    ctx.set_style(style);
}

pub fn draw_generic(
    ctx: &egui::Context,
    state: &crate::world::RuntimeState,
    ui: &mut UiState,
) -> Vec<Action> {
    let compact = state
        .gameplay
        .as_ref()
        .and_then(|g| g.compiled.ui.as_ref())
        .is_some_and(|d| d.chrome == axiom_core::gameplay::Chrome::Compact);
    if ui.player {
        return draw_player(ctx, state, ui);
    }
    if compact {
        return draw_compact(ctx, state, ui);
    }
    let mut actions = vec![];
    egui::TopBottomPanel::top("world_status").show(ctx, |p| {
        p.horizontal(|p| {
            p.label(RichText::new("AXIOM / WORLD IR v2").color(MINT));
            p.separator();
            p.label(&state.compiled.source.title);
            p.separator();
            p.label(format!(
                "{} entities | {} cameras | tick {}",
                state.compiled.len(),
                state
                    .compiled
                    .entities
                    .iter()
                    .filter(|e| e.camera.as_ref().is_some_and(|c| c.active))
                    .count(),
                state.tick
            ));
            if p.button(if ui.playing { "PAUSE" } else { "RUN" }).clicked() {
                if ui.playing {
                    ui.playing = false;
                } else {
                    actions.push(Action::Start);
                }
            }
            if p.button("RESTART").clicked() {
                actions.push(Action::Restart);
            }
            if p.button("CONSOLE [TAB]").clicked() {
                ui.console = !ui.console;
            }
            if p.button("EXIT").clicked() {
                actions.push(Action::Quit);
            }
        });
    });
    draw_world_overlays(ctx, state, ui, &mut actions);
    actions
}

/// Shipped-game chrome: content UI only. Esc still pauses, so a paused game says how to resume;
/// there is no console, restart or exit button (the window closes the game).
fn draw_player(
    ctx: &egui::Context,
    state: &crate::world::RuntimeState,
    ui: &mut UiState,
) -> Vec<Action> {
    let mut actions = vec![];
    ui.console = false;
    if !ui.playing {
        egui::Area::new(egui::Id::new("player_paused"))
            .anchor(Align2::CENTER_TOP, [0.0, 12.0])
            .order(egui::Order::Foreground)
            .show(ctx, |p| {
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(10, 14, 20, 215))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .show(p, |p| {
                        if p.button(RichText::new("PAUSED  ·  ESC").size(14.0))
                            .clicked()
                        {
                            actions.push(Action::Start);
                        }
                    });
            });
    }
    draw_world_overlays(ctx, state, ui, &mut actions);
    actions
}

/// Game-style chrome (`ui.document.chrome: "compact"`): a small session pill in the top-right
/// corner instead of the full-width debug bar. Console and pause behave the same.
fn draw_compact(
    ctx: &egui::Context,
    state: &crate::world::RuntimeState,
    ui: &mut UiState,
) -> Vec<Action> {
    let mut actions = vec![];
    egui::Area::new(egui::Id::new("compact_session"))
        .anchor(Align2::RIGHT_TOP, [-10.0, 10.0])
        .order(egui::Order::Foreground)
        .show(ctx, |p| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(10, 14, 20, 215))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(p, |p| {
                    p.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                    p.horizontal(|p| {
                        let small = |t: &str| RichText::new(t).size(13.0);
                        if !ui.playing {
                            p.label(RichText::new("PAUSED").size(13.0).color(MINT));
                        }
                        if p.button(small(if ui.playing { "PAUSE" } else { "RUN" }))
                            .clicked()
                        {
                            if ui.playing {
                                ui.playing = false;
                            } else {
                                actions.push(Action::Start);
                            }
                        }
                        if p.button(small("RESTART")).clicked() {
                            actions.push(Action::Restart);
                        }
                        if p.button(small("CONSOLE")).clicked() {
                            ui.console = !ui.console;
                        }
                        if p.button(small("EXIT")).clicked() {
                            actions.push(Action::Quit);
                        }
                    });
                });
        });
    draw_world_overlays(ctx, state, ui, &mut actions);
    actions
}

fn draw_world_overlays(
    ctx: &egui::Context,
    state: &crate::world::RuntimeState,
    ui: &mut UiState,
    actions: &mut Vec<Action>,
) {
    if state.compiled.is_empty() {
        egui::Area::new("empty_world".into())
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |p| {
                p.label(RichText::new("EMPTY WORLD").size(38.0).color(MINT));
                p.label("No player, weapon, health or physics required.");
                p.label("Use the console to add typed entities and components.");
            });
    }
    if ui.console {
        egui::Window::new("WORLD CONSOLE / v2")
            .default_width(500.0)
            .show(ctx, |p| {
                p.label("schema / query / preview / apply / verify / observe / undo");
                p.add(
                    egui::TextEdit::multiline(&mut ui.command)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(9)
                        .desired_width(f32::INFINITY),
                );
                if p.button("EXECUTE").clicked() {
                    actions.push(Action::Apply(ui.command.clone()));
                }
                egui::ScrollArea::vertical().max_height(180.0).show(p, |p| {
                    for line in &ui.output {
                        p.monospace(line);
                    }
                });
            });
    }
}
