//! Window chrome: the menu row (logo + registry menus), the top bar (world, revision, undo /
//! redo, Play group, Agent API, preferences) and the status bar. Every button runs a registry
//! command or the same action its menu item produces.
use super::icons::Icon;
use super::theme::{self, divider, ACCENT, MUTED, SUCCESS, TEXT, WARNING};
use super::{Action, Editor};
use egui::{Color32, Layout, Margin, RichText, Ui};

const MIB: f64 = 1024.0 * 1024.0;

impl Editor {
    /// Theme, icon atlas and logo for this context: built and uploaded once.
    pub(super) fn chrome_setup(&mut self, ctx: &egui::Context) {
        let style = self
            .theme
            .get_or_insert_with(|| theme::style(&ctx.style()))
            .clone();
        if !std::sync::Arc::ptr_eq(&ctx.style(), &style) {
            ctx.set_style(style);
        }
        if !self.icons_tried {
            self.icons_tried = true;
            match super::icons::Atlas::load(ctx) {
                Ok(atlas) => {
                    self.icons = atlas.icons();
                    self.icon_atlas = Some(atlas);
                }
                Err(e) => self.set_error(format!("Editor icons unavailable: {e:#}")),
            }
        }
        if self.logo.is_none() {
            self.logo = crate::branding::egui_icon()
                .map(|image| ctx.load_texture("axiom-logo", image, egui::TextureOptions::LINEAR));
        }
    }

    /// Logo, wordmark and the registry menus.
    pub(super) fn menu_row(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(24.0, 24.0)));
            }
            ui.label(RichText::new("Axiom").strong().size(16.0).color(TEXT));
            ui.label(RichText::new("Editor").size(12.0).color(MUTED));
            ui.add_space(10.0);
            self.menu_bar(ui, actions);
        });
    }

    /// Pills: world (drop-down), scene, revision / writing, undo / redo, the Play group;
    /// Agent API and preferences on the right.
    pub(super) fn top_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let icons = self.icons;
        let mut run: Option<&'static str> = None;
        ui.horizontal(|ui| {
            ui.set_min_height(theme::PILL_H + 4.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            // Right group first (gear, Agent API), then the left group fills what is left.
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let r = theme::pill_group(ui, |ui| {
                    theme::icon_button_sized(ui, icons, Icon::Settings, "Preferences…  Ctrl+,", false, theme::PILL_H - 4.0)
                })
                .inner;
                self.rects.insert("top:edit.preferences".into(), r.rect);
                if r.clicked() {
                    run = Some("edit.preferences");
                }
                self.agent_api(ui, actions);
                ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
            // World / pack: title with a drop-down of world-level commands.
            let world = theme::pill(ui, |ui| {
                icons.show(ui, Icon::Folder, 16.0, MUTED);
                ui.label(RichText::new(super::ui::truncate(&self.world.title, 28)).color(TEXT));
                icons.show(ui, Icon::ChevronDown, 14.0, MUTED);
            })
            .on_hover_text(format!("{} · world ID {}", self.world.title, self.world.id));
            self.rects.insert("top:world".into(), world.rect);
            let id = ui.make_persistent_id("top_world");
            theme::dropdown(ui, id, &world, |ui| {
                ui.label(RichText::new(format!("World ID  {}", self.world.id)).small().color(MUTED));
                for cmd in ["window.world", "file.verify", "file.copy_context"] {
                    let title = super::commands::command(cmd).map_or(cmd, |c| c.title);
                    let blocked = self.command_blocked(cmd);
                    if ui.add_enabled(blocked.is_none(), egui::Button::new(title)).clicked() {
                        run = Some(cmd);
                        theme::close_dropdown(ui, id);
                    }
                }
            });
            // Scene: the Hierarchy scene filter (view only), else the active scene(s).
            let scene_text = match &self.scene {
                Some(scene) => scene.clone(),
                None => {
                    let mut active = self.world.scenes.iter().filter(|s| s.active);
                    match (active.next(), active.count()) {
                        (None, _) => "All objects".into(),
                        (Some(s), 0) => s.id.clone(),
                        (Some(s), more) => format!("{} +{more}", s.id),
                    }
                }
            };
            let scene = theme::pill(ui, |ui| {
                icons.show(ui, Icon::Scene, 16.0, MUTED);
                ui.label(RichText::new(super::ui::truncate(&scene_text, 20)).color(TEXT));
                icons.show(ui, Icon::ChevronDown, 14.0, MUTED);
            })
            .on_hover_text("Scene shown in the Hierarchy (view only; scenes are managed under its … menu)");
            self.rects.insert("top:scene".into(), scene.rect);
            let id = ui.make_persistent_id("top_scene");
            theme::dropdown(ui, id, &scene, |ui| {
                let mut picked = None;
                if ui.selectable_label(self.scene.is_none(), "All scenes").clicked() {
                    picked = Some(None);
                }
                for s in &self.world.scenes {
                    let on = self.scene.as_deref() == Some(s.id.as_str());
                    let text = if s.active { s.id.clone() } else { format!("{} · inactive", s.id) };
                    if ui.selectable_label(on, text).clicked() {
                        picked = Some(Some(s.id.clone()));
                    }
                }
                if let Some(scene) = picked {
                    self.scene = scene;
                    theme::close_dropdown(ui, id);
                }
            });
            // No unsaved state: every edit is a revision committed by the background writer.
            theme::pill(ui, |ui| {
                icons.show(ui, Icon::History, 16.0, MUTED);
                ui.label(RichText::new(format!("REV {}", self.rev)).color(TEXT));
                if self.saving > 0 {
                    ui.label(RichText::new(format!("· writing {}", self.saving)).color(WARNING));
                }
            })
            .on_hover_text(if self.saving > 0 {
                "Edits queued on the background writer (the scene already shows them). Every edit is committed as a revision; there is nothing to save."
            } else {
                "Current revision. Every edit is committed as a revision (nothing to save); History restores any of them."
            });
            theme::pill_group(ui, |ui| {
                for (icon, redo, cmd) in [
                    (Icon::Undo, false, "edit.undo"),
                    (Icon::Redo, true, "edit.redo"),
                ] {
                    let action = self.restore_action(redo);
                    let blocked = self.command_blocked(cmd);
                    let title = super::commands::command(cmd).map_or(cmd, |c| c.title);
                    let tip = format!("{title}  {}", self.shortcut_text(cmd));
                    let r = ui
                        .add_enabled_ui(
                            !self.dirty() && action.is_some() && blocked.is_none(),
                            |ui| theme::icon_button_sized(ui, icons, icon, &tip, false, theme::PILL_H - 4.0),
                        )
                        .inner;
                    self.rects.insert(format!("top:{cmd}"), r.rect);
                    if r.clicked() {
                        actions.extend(action);
                    }
                }
            });
            self.play_toolbar(ui, actions);
                });
            });
        });
        if let Some(cmd) = run {
            self.run_into(cmd, actions);
        }
    }

    /// Bridge status: port, requests served, recent activity. Opens the AI / Protocol tab.
    fn agent_api(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let (requests, since) = self.bridge_activity;
        let recent = since.is_some_and(|d| d.as_secs_f32() < 5.0);
        let (dot, state) = match self.bridge {
            None => (MUTED.gamma_multiply(0.6), "off".to_owned()),
            Some(_) if recent => (SUCCESS, "active".to_owned()),
            Some(_) => (SUCCESS.gamma_multiply(0.55), "idle".to_owned()),
        };
        let response = egui::Frame::new()
            .fill(theme::CONTROL)
            .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 0))
            .show(ui, |ui| {
                ui.set_min_height(theme::PILL_H - 2.0);
                // The bar lays out right to left: added last = leftmost (dot, title, detail).
                let detail = match self.bridge {
                    Some(port) => format!(":{port} · {requests} req"),
                    None => "off".into(),
                };
                ui.label(RichText::new(detail).size(11.0).color(MUTED));
                ui.label(RichText::new("Agent API").color(TEXT));
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 16.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, dot);
            })
            .response
            .interact(egui::Sense::click());
        let tip = match self.bridge {
            Some(port) => format!(
                "Loopback bridge 127.0.0.1:{port} · {state} · {requests} request(s){}. Click: AI / Protocol tab.",
                since.map_or(String::new(), |d| format!(", last {:.0} s ago", d.as_secs_f32()))
            ),
            None => "Bridge off (start the editor with --port to let agents connect). Click: AI / Protocol tab.".into(),
        };
        let response = response.on_hover_text(tip);
        self.rects.insert("top:agent".into(), response.rect);
        if response.clicked() {
            self.run_into("window.agent", actions);
        }
    }

    /// Status text, world facts, render counters, Edit / Play mode, bridge, FPS, version.
    pub(crate) fn status_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let icons = self.icons;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                // Right to left: the first item is the rightmost.
                let build = if cfg!(debug_assertions) { "debug" } else { "release" };
                ui.label(
                    RichText::new(format!("v{} ({build})", env!("CARGO_PKG_VERSION")))
                        .size(11.0)
                        .color(MUTED),
                );
                if let Some(logo) = &self.logo {
                    ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(16.0, 16.0)));
                }
                divider(ui);
                item(ui, format!("{:.0} FPS", self.fps), TEXT);
                item(
                    ui,
                    match self.bridge {
                        Some(port) => format!("bridge 127.0.0.1:{port}"),
                        None => "bridge off".into(),
                    },
                    MUTED,
                );
                divider(ui);
                // Edit / Play mode: the same commands as Play ▸ Play / Stop.
                for (title, play) in [("Play Mode", true), ("Edit Mode", false)] {
                    let on = self.previewing == play;
                    let cmd = if play { "play.start" } else { "play.stop" };
                    let blocked = !on && self.command_blocked(cmd).is_some();
                    let text = RichText::new(title).size(11.5).color(if on { TEXT } else { MUTED });
                    let r = ui
                        .add_enabled(
                            !blocked,
                            egui::Button::new(text)
                                .selected(on)
                                .min_size(egui::vec2(72.0, 18.0)),
                        )
                        .on_hover_text(format!(
                            "{} ({})",
                            if play { "Play in the Game tab" } else { "Stop Play and edit" },
                            self.shortcut_text(cmd)
                        ));
                    self.rects.insert(format!("status:{cmd}"), r.rect);
                    if r.clicked() && !on {
                        self.run_into(cmd, actions);
                    }
                }
                divider(ui);
                let (gpu, tip) = vram_text(self.vram.as_ref(), self.gpu_alloc);
                let r = item(ui, gpu, MUTED).on_hover_text(tip);
                self.rects.insert("status:vram".into(), r.rect);
                let s = self.frame_stats;
                item(ui, format!("Tris {}", grouped(s.triangles)), MUTED)
                    .on_hover_text("Triangles submitted by the world passes last frame (primitives × instances)");
                item(
                    ui,
                    format!("Draw {} + {} UI", s.draw_calls, s.ui_draw_calls),
                    MUTED,
                )
                .on_hover_text("Draw calls recorded last frame: world passes + interface meshes (CPU count, not GPU time)");
                if let Some(job) = self.assets.thumbs.running() {
                    let (done, total) = job.progress();
                    item(ui, format!("thumbnails {done}/{total}"), WARNING);
                }
                if let Some(job) = &self.scenario_job {
                    let (done, total) = job.progress();
                    item(
                        ui,
                        format!(
                            "scenario {done}/{total} ticks · {:.1} s",
                            job.elapsed().as_secs_f32()
                        ),
                        WARNING,
                    );
                }
                if self.saving > 0 {
                    item(ui, format!("writing {}…", self.saving), WARNING).on_hover_text(
                        "Edits are committed in order on the background writer; the scene shows them already",
                    );
                }
                divider(ui);
                if !self.problems.is_empty() {
                    item(ui, format!("{} problem(s)", self.problems.len()), WARNING);
                }
                if !self.proposals.is_empty() {
                    item(ui, format!("{} proposal(s)", self.proposals.len()), ACCENT);
                }
                item(ui, format!("{} selected", self.selection().len()), MUTED);
                item(ui, format!("Entities {}", self.world.entities.len()), MUTED);
                let scene = match &self.scene {
                    Some(scene) => scene.clone(),
                    None => {
                        let mut active = self.world.scenes.iter().filter(|s| s.active);
                        match (active.next(), active.count()) {
                            (None, _) => "global".into(),
                            (Some(s), 0) => s.id.clone(),
                            (Some(s), more) => format!("{} +{more}", s.id),
                        }
                    }
                };
                item(ui, format!("Scene: {}", super::ui::truncate(&scene, 18)), MUTED)
                    .on_hover_text("Hierarchy scene filter, else the active scene(s)");
                divider(ui);
                // Status text takes what is left (truncated).
                ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                    let color = if self.error { WARNING } else { SUCCESS };
                    icons.show(ui, Icon::Dot, 14.0, color);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&self.status)
                                .size(11.5)
                                .color(if self.error { WARNING } else { TEXT }),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&self.status);
                });
            });
        });
    }
}

/// One status value; the gap after it separates it from the next one (the bar is laid out
/// right to left).
fn item(ui: &mut Ui, text: String, color: Color32) -> egui::Response {
    let response = ui.label(RichText::new(text).size(11.5).color(color));
    ui.add_space(10.0);
    response
}

/// Status bar video memory: this process's dedicated video memory from the OS (`vram.rs`),
/// else the wgpu allocator total, else a dash with the reason in the tooltip.
pub(super) fn vram_text(
    vram: Option<&crate::vram::VramReading>,
    alloc: Option<u64>,
) -> (String, String) {
    use crate::vram::VramReading;
    let alloc_line = match alloc {
        Some(bytes) => format!("wgpu allocator total: {:.0} MiB", bytes as f64 / MIB),
        None => {
            "wgpu allocator total: not reported by this backend (wgpu 24 reports it only on D3D12)"
                .into()
        }
    };
    match vram {
        Some(VramReading::Bytes {
            local,
            shared,
            budget,
        }) => {
            let gib = |b: u64| b as f64 / (MIB * 1024.0);
            let text = if *local >= 1 << 30 {
                format!("VRAM {:.2} GB", gib(*local))
            } else {
                format!("VRAM {:.0} MiB", *local as f64 / MIB)
            };
            (
                text,
                format!(
                    "Video memory this process uses on the render adapter (DXGI QueryVideoMemoryInfo, all APIs, swapchain and driver memory included), refreshed every 2 s.\nDedicated: {:.0} MiB of a {:.2} GB budget\nShared system memory: {:.0} MiB\n{alloc_line}",
                    *local as f64 / MIB,
                    gib(*budget),
                    *shared as f64 / MIB,
                ),
            )
        }
        other => {
            let why = match other {
                Some(VramReading::Unavailable(why)) => why.as_str(),
                _ => "not read yet (the first reading comes with the first rendered frame)",
            };
            match alloc {
                Some(bytes) => (
                    format!("GPU alloc {:.0} MiB", bytes as f64 / MIB),
                    format!("wgpu allocator total, refreshed every 2 s (excludes swapchain images and driver memory). Per-process video memory unavailable: {why}"),
                ),
                None => (
                    "VRAM –".into(),
                    format!("Video memory unavailable: {why}.\n{alloc_line}"),
                ),
            }
        }
    }
}

/// 1780321 -> "1,780,321".
pub(super) fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn grouped_thousands() {
        assert_eq!(super::grouped(0), "0");
        assert_eq!(super::grouped(999), "999");
        assert_eq!(super::grouped(1000), "1,000");
        assert_eq!(super::grouped(1_780_321), "1,780,321");
    }

    #[test]
    fn vram_text_prefers_the_os_reading_and_explains_a_missing_one() {
        use crate::vram::VramReading;
        let mib = 1024 * 1024;
        let bytes = VramReading::Bytes {
            local: 300 * mib,
            shared: 20 * mib,
            budget: 1536 * mib,
        };
        let (text, tip) = super::vram_text(Some(&bytes), None);
        assert_eq!(text, "VRAM 300 MiB");
        assert!(tip.contains("Shared system memory: 20 MiB") && tip.contains("1.50 GB budget"));
        let big = VramReading::Bytes {
            local: 1290 * mib,
            shared: 0,
            budget: 4096 * mib,
        };
        assert_eq!(super::vram_text(Some(&big), None).0, "VRAM 1.26 GB");
        let gone = VramReading::Unavailable("no DXGI adapter matches PCI 10de:1d12".into());
        let (text, tip) = super::vram_text(Some(&gone), None);
        assert_eq!(text, "VRAM –");
        assert!(tip.contains("no DXGI adapter matches") && tip.contains("D3D12"));
        assert_eq!(
            super::vram_text(Some(&gone), Some(64 * mib)).0,
            "GPU alloc 64 MiB"
        );
        assert!(super::vram_text(None, None).1.contains("not read yet"));
    }
}
