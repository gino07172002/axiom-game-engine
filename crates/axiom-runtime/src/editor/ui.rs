use super::icons::Icon;
use super::{theme, viewport, Action, Dock, Draft, Editor, Tool};
use crate::world::WorldRuntime;
use axiom_core::{
    merge_json, scene_edit::Hierarchy, CoreEdit, Edit, Entity, Request, Transform, World,
};
use egui::{Color32, Frame, Layout, Margin, RichText, ScrollArea, Ui};
use serde_json::{json, Value};

pub(super) use super::theme::{ACCENT, MUTED, SURFACE, WARNING};
use super::theme::{BACKDROP, HEADER};

pub(super) fn panel(color: Color32, padding: i8) -> Frame {
    Frame::new().fill(color).inner_margin(Margin::same(padding))
}
pub(super) fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(11.0).color(MUTED));
}

pub(super) fn component_value(world: &World, entity: &Entity, component: &str) -> Option<Value> {
    let mut inherited = entity
        .prefab
        .as_ref()
        .and_then(|p| world.prefabs.get(p))
        .and_then(|p| p.components.get(component))
        .cloned();
    if let Some(value) = entity.components.get(component) {
        merge_json(inherited.get_or_insert(Value::Null), value);
    }
    inherited
}

impl Editor {
    pub fn draw(&mut self, ctx: &egui::Context, runtime: &WorldRuntime) -> Vec<Action> {
        let mut actions = Vec::new();
        let text_focused = ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some());
        // While RMB is held in the scene view, WASD/QE fly the camera instead of switching
        // tools; a box selection in progress keeps Escape for itself.
        let scene_owns_keys = self.scene_input.marquee.is_some()
            || ctx.input(|i| super::viewport::rmb_held_in(i, self.viewport));
        // While the game has the keyboard only the Play commands stay bound; Esc gives the
        // keyboard back to the editor (the game never sees it then).
        if self.game.input {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.game.release_request = true;
            }
            let pressed = ctx.input(|i| self.shortcut_commands(i));
            for id in pressed.into_iter().filter(|id| id.starts_with("play.")) {
                if self.command_blocked(id).is_none() {
                    self.run_into(id, &mut actions);
                }
            }
        } else if !text_focused
            && self.renaming.is_none()
            && !scene_owns_keys
            && self.options.is_none()
            && !self.keys_captured(ctx)
        {
            // Every shortcut comes from the command registry (user overrides included).
            let pressed = ctx.input(|i| self.shortcut_commands(i));
            for id in pressed {
                if self.command_blocked(id).is_none() {
                    self.run_into(id, &mut actions);
                }
            }
            // egui-winit turns Ctrl+C / Ctrl+V into Copy / Paste events, not key presses.
            let events = ctx.input(|i| i.events.clone());
            for event in events {
                match event {
                    egui::Event::Copy if self.selected.is_some() => {
                        self.run_into("edit.copy", &mut actions);
                    }
                    egui::Event::Paste(text) if !self.dirty() && self.drag.is_none() => {
                        // Pasted text may be an ID list from another window or an agent.
                        let ids: Vec<String> = text
                            .lines()
                            .map(str::trim)
                            .filter(|id| self.world.entity(id).is_some())
                            .map(str::to_owned)
                            .collect();
                        if !ids.is_empty() {
                            self.clipboard = ids;
                        }
                        if !self.clipboard.is_empty() {
                            self.run_into("edit.paste", &mut actions);
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(text) = self.pending_copy.take() {
            ctx.copy_text(text);
        }
        // Filled again by whichever tabs are drawn this frame.
        self.rects.clear();
        self.tuning.begin_frame();
        self.chrome_setup(ctx);
        egui::TopBottomPanel::top("editor_menu")
            .frame(panel(BACKDROP, 3))
            .show_separator_line(false)
            .show(ctx, |ui| self.menu_row(ui, &mut actions));
        egui::TopBottomPanel::top("editor_toolbar")
            .frame(
                Frame::new()
                    .fill(HEADER)
                    .inner_margin(Margin::symmetric(8, 5)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| self.top_bar(ui, &mut actions));
        egui::TopBottomPanel::bottom("editor_status")
            .frame(
                Frame::new()
                    .fill(HEADER)
                    .inner_margin(Margin::symmetric(8, 3)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| self.status_bar(ui, &mut actions));
        // Play preview: the timeline playhead follows the runtime.
        self.tween_ui.live = if self.previewing {
            self.selected
                .as_deref()
                .and_then(|id| runtime.system::<crate::tween::TweenSystem>()?.playback(id))
                .map(|(clip, time, _)| (clip.to_owned(), time))
        } else {
            None
        };
        // Every panel is a dock tab (Window menu / `window.*` reopen closed ones). The state is
        // lent to the dock area for the frame; `show_dock` calls made meanwhile wait in
        // `layout.pending`.
        let mut dock = self
            .layout
            .dock
            .take()
            .unwrap_or_else(super::layout::default_state);
        let style = self
            .layout
            .style
            .get_or_insert_with(|| dock_style(&ctx.style()))
            .clone();
        // Filled again by the Game tab and the Inspector camera preview if they are drawn.
        self.game.rect = egui::Rect::NOTHING;
        self.game.hovered = false;
        self.camera_preview.rect = egui::Rect::NOTHING;
        self.camera_preview.camera = None;
        self.live.shown = false;
        self.debug.profiler.visible = false;
        // A floating Inspector has a window frame over the preview hole: no preview there.
        let inspector_floating = dock
            .find_tab(&Dock::Inspector)
            .is_some_and(|(surface, _, _)| !surface.is_main());
        let mut host = TabHost {
            editor: self,
            runtime,
            actions: &mut actions,
            scene: false,
            game: false,
            inspector_floating,
            alone: dock
                .main_surface()
                .iter()
                .filter_map(|n| match n {
                    egui_dock::Node::Leaf { tabs, .. } if tabs.len() == 1 => Some(tabs[0]),
                    _ => None,
                })
                .fold(0, |bits, tab| bits | tab_bit(tab)),
            bars: {
                // Each leaf's tab strip (last frame's rectangles), keyed by its active tab.
                let mut bars = [None; 16];
                // Every surface (floating windows too: their tabs close from this menu).
                let leaves = dock.iter_all_nodes().filter_map(|(_, n)| match n {
                    egui_dock::Node::Leaf {
                        rect,
                        viewport,
                        tabs,
                        active,
                        collapsed,
                        ..
                    } if !*collapsed && rect.is_positive() && viewport.min.y > rect.min.y => {
                        let bar = egui::Rect::from_min_max(
                            rect.min,
                            egui::pos2(rect.max.x, viewport.min.y),
                        );
                        tabs.get(active.0).map(|t| (*t, bar))
                    }
                    _ => None,
                });
                for (slot, leaf) in bars.iter_mut().zip(leaves) {
                    *slot = Some(leaf);
                }
                bars
            },
            panel_command: None,
        };
        egui_dock::DockArea::new(&mut dock)
            .id(egui::Id::new("editor_dock_area"))
            .style(style)
            .show_leaf_collapse_buttons(false)
            // One click must not close a whole group (e.g. all eight bottom tabs).
            .show_leaf_close_all_buttons(false)
            // Tabs close from the panel "…" menu (window.close), not an X on every tab.
            .show_close_buttons(false)
            .show(ctx, &mut host);
        let (scene, game) = (host.scene, host.game);
        let panel_command = host.panel_command.take();
        self.panel_icons(ctx, &dock);
        self.layout.dock = Some(dock);
        self.layout.apply_pending();
        if let Some((id, arg)) = panel_command {
            if let Err(e) = self.run_command_with(id, arg.as_deref()) {
                self.set_error(format!("{e:#}"));
            }
        }
        self.scene_visible = scene;
        if !scene {
            self.scene_hidden();
        }
        self.game.visible = game && self.game.rect.is_positive();
        // A click anywhere else in the editor takes the input back from the game.
        if self.game.input && !self.game.hovered && ctx.input(|i| i.pointer.primary_pressed()) {
            self.game.release_request = true;
        }
        if !self.game.visible {
            self.game.rect = egui::Rect::NOTHING;
            // Hidden Game tab: the game cannot keep the keyboard.
            self.game.capture_request = false;
            if self.game.input {
                self.game.release_request = true;
            }
        }
        self.merge_window(ctx, &mut actions);
        actions.extend(self.ghost_request());
        self.command_windows(ctx, &mut actions);
        // Not while a button is held (dragging a separator or a window would write the
        // preferences file every frame); the change is saved on release.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.save_prefs();
        }
        actions
    }

    /// The Scene tab is not drawn (closed, behind another tab, collapsed): nothing may keep
    /// acting on its last rectangle.
    fn scene_hidden(&mut self) {
        self.viewport = egui::Rect::NOTHING;
        self.scene_input.marquee = None;
        self.scene_input.flying = false;
        self.scene_input.point_drag = None;
        self.scene_input.menu = None;
        if self.drag.is_some() {
            // Same as Escape during a gizmo drag: back to the committed transform.
            self.select(self.selected.clone());
        }
    }

    /// Scene tab: tool strip and the viewport (composited by the renderer under the UI).
    fn scene_tab(&mut self, ui: &mut Ui, runtime: &WorldRuntime, actions: &mut Vec<Action>) {
        let icons = self.icons;
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut run: Option<&'static str> = None;
        Frame::new()
            .fill(SURFACE)
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                // Wraps instead of widening the central panel (which would push the viewport
                // under the inspector).
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    theme::pill_group(ui, |ui| for (tool, icon, cmd) in [
                        (Tool::Select, Icon::Select, "tool.select"),
                        (Tool::Move, Icon::Move, "tool.move"),
                        (Tool::Rotate, Icon::Rotate, "tool.rotate"),
                        (Tool::Scale, Icon::Scale, "tool.scale"),
                    ] {
                        let tip = format!("{} tool  {}", tool.name(), self.shortcut_text(cmd));
                        let r = theme::icon_button_sized(ui, icons, icon, &tip, self.tool == tool, 28.0);
                        self.rects.insert(format!("scene:{cmd}"), r.rect);
                        if r.clicked() {
                            run = Some(cmd);
                        }
                    });
                    // Snap: toggle + increments drop-down.
                    theme::pill_group(ui, |ui| {
                    let r = theme::icon_button_sized(
                        ui,
                        icons,
                        Icon::Snap,
                        "Snap while dragging gizmos (hold Ctrl to invert)",
                        self.prefs.snap,
                        28.0,
                    );
                    self.rects.insert("scene:view.snap".into(), r.rect);
                    if r.clicked() {
                        run = Some("view.snap");
                    }
                    let steps = ui
                        .add(egui::Button::image_and_text(
                            icons.image(ui.ctx(), Icon::ChevronDown, 14.0, MUTED),
                            RichText::new(format!("Snap {} m", self.prefs.snap_move)),
                        ).frame(false))
                        .on_hover_text(format!(
                            "Snap increments: {} m · {}° · {} scale",
                            self.prefs.snap_move,
                            self.prefs.snap_rotate_degrees,
                            self.prefs.snap_scale
                        ));
                    let id = ui.make_persistent_id("scene_snap_steps");
                    theme::dropdown(ui, id, &steps, |ui| {
                        ui.label(RichText::new("SNAP INCREMENTS").small().color(MUTED));
                        ui.add(egui::DragValue::new(&mut self.prefs.snap_move).speed(0.01).range(0.001..=100.0).prefix("Move ").suffix(" m"));
                        ui.add(egui::DragValue::new(&mut self.prefs.snap_rotate_degrees).speed(0.5).range(0.1..=180.0).prefix("Rotate ").suffix("°"));
                        ui.add(egui::DragValue::new(&mut self.prefs.snap_scale).speed(0.01).range(0.001..=10.0).prefix("Scale "));
                    });
                    });
                    // Gizmo axes: Global / Local.
                    let (icon, space) = if self.prefs.local_space {
                        (Icon::Local, "Local")
                    } else {
                        (Icon::Global, "Global")
                    };
                    let r = theme::pill(ui, |ui| {
                        icons.show(ui, icon, 16.0, theme::TEXT);
                        ui.label(space);
                    })
                        .on_hover_text(format!(
                            "Move/Rotate gizmo axes: world (Global) or the object's own (Local). Scale is always local.  {}",
                            self.shortcut_text("view.space")
                        ));
                    self.rects.insert("scene:view.space".into(), r.rect);
                    if r.clicked() {
                        run = Some("view.space");
                    }
                    theme::pill_group(ui, |ui| for (on, icon, cmd, tip) in [
                        (self.prefs.grid, Icon::Grid, "view.grid", "Always draw the ground grid"),
                        (
                            self.prefs.gizmos,
                            Icon::Layers,
                            "view.gizmos",
                            "Component gizmos: camera frusta, nav.blocker, colliders and FSM agents (editor view only)",
                        ),
                    ] {
                        let r = theme::icon_button_sized(ui, icons, icon, tip, on, 28.0);
                        self.rects.insert(format!("scene:{cmd}"), r.rect);
                        if r.clicked() {
                            run = Some(cmd);
                        }
                    });
                    let mode = self.orbit.mode;
                    let view = theme::pill(ui, |ui| {
                        icons.show(ui, Icon::Scene, 16.0, MUTED);
                        ui.label(mode.name());
                        icons.show(ui, Icon::ChevronDown, 14.0, MUTED);
                    })
                        .on_hover_text("Perspective / orthographic axis views and camera bookmarks");
                    self.rects.insert("scene:view_mode".into(), view.rect);
                    let id = ui.make_persistent_id("scene_view_mode");
                    theme::dropdown(ui, id, &view, |ui| {
                        for m in super::viewport::ViewMode::ALL {
                            let text = format!(
                                "{}{}",
                                m.name(),
                                if m == super::viewport::ViewMode::Perspective { "" } else { " (ortho)" }
                            );
                            if ui
                                .add(
                                    egui::Button::new(text)
                                        .selected(m == mode)
                                        .shortcut_text(self.shortcut_text(m.command())),
                                )
                                .clicked()
                            {
                                run = Some(m.command());
                                theme::close_dropdown(ui, id);
                            }
                        }
                        ui.separator();
                        for n in 1..=4 {
                            ui.horizontal(|ui| {
                                let save = format!("view.bookmark.save.{n}");
                                let recall = format!("view.bookmark.recall.{n}");
                                if ui.button(format!("Save {n}")).on_hover_text(self.shortcut_text(&save)).clicked() {
                                    self.run_into(&save, actions);
                                    theme::close_dropdown(ui, id);
                                }
                                let blocked = self.command_blocked(&recall);
                                if ui
                                    .add_enabled(blocked.is_none(), egui::Button::new(format!("Recall {n}")))
                                    .on_hover_text(self.shortcut_text(&recall))
                                    .clicked()
                                {
                                    self.run_into(&recall, actions);
                                    theme::close_dropdown(ui, id);
                                }
                            });
                        }
                    });
                });
            });
        if let Some(cmd) = run.filter(|cmd| self.command_blocked(cmd).is_none()) {
            self.run_into(cmd, actions);
        }
        self.scene_view(ui, runtime, actions);
        self.viewport_overlay(ui, actions);
    }

    /// Over the scene view: FPS chip (top left; the hint line is bottom left), Frame and
    /// Fullscreen (bottom right).
    fn viewport_overlay(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let rect = self.viewport;
        if !rect.is_positive() || rect.height() < 80.0 {
            return;
        }
        let painter = ui.painter_at(rect);
        // Rounded frame: the view is composited as a rectangle; the panel colour masks its
        // corners and a 1 px border outlines it.
        theme::round_corners(&painter, rect, 10.0, SURFACE);
        painter.rect_stroke(
            rect,
            10.0,
            egui::Stroke::new(1.0_f32, theme::BORDER),
            egui::StrokeKind::Inside,
        );
        let ms = if self.fps > 0.0 {
            1000.0 / self.fps
        } else {
            0.0
        };
        let galley = painter.layout_no_wrap(
            format!("FPS {:.0}  |  {ms:.1} ms", self.fps),
            egui::FontId::proportional(12.0),
            theme::TEXT,
        );
        let size = galley.size() + egui::vec2(20.0, 12.0);
        let chip = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 12.0, rect.bottom() - 12.0 - size.y),
            size,
        );
        painter.rect_filled(chip, 8.0, Color32::from_black_alpha(170));
        painter.galley(chip.min + egui::vec2(10.0, 6.0), galley, theme::TEXT);
        let icons = self.icons;
        let mut run = None;
        for (i, (icon, cmd)) in [
            (Icon::Fullscreen, "view.fullscreen"),
            (Icon::Frame, "edit.frame"),
        ]
        .into_iter()
        .enumerate()
        {
            let at = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 40.0 - i as f32 * 36.0, rect.bottom() - 40.0),
                egui::vec2(30.0, 30.0),
            );
            painter.rect_filled(at, 6.0, Color32::from_black_alpha(150));
            let blocked = self.command_blocked(cmd);
            let title = super::commands::command(cmd).map_or(cmd, |c| c.title);
            let tip = format!("{title}  {}", self.shortcut_text(cmd));
            let r = ui.put(at, |ui: &mut Ui| {
                ui.add_enabled_ui(blocked.is_none(), |ui| {
                    theme::icon_button_sized(ui, icons, icon, &tip, false, 30.0)
                })
                .inner
            });
            self.rects.insert(format!("scene:{cmd}"), r.rect);
            if r.clicked() {
                run = Some(cmd);
            }
        }
        if let Some(cmd) = run {
            self.run_into(cmd, actions);
        }
    }

    /// Undo / redo. While writes are saving, the step they will push may not exist yet; the
    /// host's write queue then resolves the target revision when the restore is sent.
    pub(super) fn restore_action(&self, redo: bool) -> Option<Action> {
        let stack = if redo { &self.redo } else { &self.undo };
        let rev = match stack.last() {
            Some(rev) => *rev,
            None if self.saving > 0 => 0,
            None => return None,
        };
        Some(Action::Restore { rev, redo })
    }

    /// Scenes: create, and (de)activate / delete the one the Hierarchy is filtered to.
    fn scene_menu(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.label(RichText::new("SCENES").small().color(MUTED));
        ui.add(egui::TextEdit::singleline(&mut self.new_scene).hint_text("New scene ID"));
        let exists = self
            .world
            .scenes
            .iter()
            .any(|s| s.id == self.new_scene.trim());
        if exists {
            ui.label(RichText::new("Scene ID already exists").color(WARNING));
        }
        if ui
            .add_enabled(
                !self.new_scene.trim().is_empty() && !exists,
                egui::Button::new("Create scene"),
            )
            .clicked()
        {
            let id = self.new_scene.trim().to_owned();
            actions.extend(self.scene_create(&id));
            self.new_scene.clear();
        }
        let filtered_scene = self
            .scene
            .as_ref()
            .and_then(|id| self.world.scenes.iter().find(|s| &s.id == id))
            .map(|s| (s.id.clone(), s.active));
        let Some((scene, active)) = filtered_scene else {
            caption(ui, "Filter to a scene to (de)activate or delete it.");
            return;
        };
        let title = if active {
            "Deactivate scene"
        } else {
            "Activate scene"
        };
        if ui.button(title).clicked() {
            actions.extend(self.scene_set_active(&scene, !active));
        }
        // Two clicks: Delete scene… arms, the red button deletes.
        let armed_id = ui.make_persistent_id(("scene_delete", &scene));
        let mut armed = ui.data(|d| d.get_temp::<bool>(armed_id).unwrap_or(false));
        if ui.button("Delete scene…").clicked() {
            armed = !armed;
        }
        if armed {
            caption(ui, "Removes membership. Objects are kept.");
            if ui
                .button(RichText::new(format!("Delete scene {scene}")).color(WARNING))
                .clicked()
            {
                actions.extend(self.scene_delete(&scene));
                armed = false;
            }
        }
        ui.data_mut(|d| d.insert_temp(armed_id, armed));
    }

    fn hierarchy(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let icons = self.icons;
        let busy = self.dirty();
        const TOOLS: [(Icon, &str); 5] = [
            (Icon::Duplicate, "edit.duplicate"),
            (Icon::Delete, "edit.delete"),
            (Icon::Rename, "edit.rename"),
            (Icon::Group, "object.group"),
            (Icon::Frame, "edit.frame"),
        ];
        ui.horizontal(|ui| {
            // Toolbar: Add ▾ and the selection commands (registry commands, same as the
            // menus); the "…" menu on the right. In a narrow panel Add shows only its icon
            // and the selection commands that do not fit move into "…".
            let step = 22.0 + ui.spacing().item_spacing.x;
            let room = ui.available_width();
            let compact = room < 70.0 + 12.0 + step * (TOOLS.len() as f32 + 1.0);
            let add_width = if compact { 30.0 } else { 70.0 };
            let fit = (((room - add_width - 12.0) / step).floor() as usize)
                .saturating_sub(1)
                .min(TOOLS.len());
            let add = ui
                .add_enabled_ui(!busy, |ui| {
                    if compact {
                        let image = icons.image(ui.ctx(), Icon::Add, 16.0, theme::TEXT);
                        ui.add(egui::Button::image(image).fill(theme::ACCENT_SOFT))
                    } else {
                        theme::icon_text_button(
                            ui,
                            icons,
                            Icon::Add,
                            "Add",
                            Some(theme::ACCENT_SOFT),
                            theme::TEXT,
                        )
                    }
                })
                .inner
                .on_hover_text("Create an object");
            self.rects.insert("hierarchy:add".into(), add.rect);
            let id = ui.make_persistent_id("hierarchy_add");
            let mut preset_picked = None;
            theme::dropdown(ui, id, &add, |ui| {
                // The Object menu's create commands (same validators; refused while busy).
                for (preset, cmd, label, icon) in [
                    ("empty", "object.empty", "Empty object", Icon::Entity),
                    ("cube", "object.cube", "Cube", Icon::Mesh),
                    ("camera", "object.camera", "Camera", Icon::Camera),
                    (
                        "light.directional",
                        "object.light_directional",
                        "Directional light",
                        Icon::LightDirectional,
                    ),
                    (
                        "light.point",
                        "object.light_point",
                        "Point light",
                        Icon::LightPoint,
                    ),
                    (
                        "light.spot",
                        "object.light_spot",
                        "Spot light",
                        Icon::LightSpot,
                    ),
                    (
                        "light.area",
                        "object.light_area",
                        "Area light (approximate)",
                        Icon::LightArea,
                    ),
                    (
                        "trigger.box",
                        "object.trigger_box",
                        "Box trigger",
                        Icon::Trigger,
                    ),
                    (
                        "trigger.sphere",
                        "object.trigger_sphere",
                        "Sphere trigger",
                        Icon::Trigger,
                    ),
                    ("path", "object.path", "Path", Icon::Path),
                    (
                        "obstacle",
                        "object.obstacle",
                        "Obstacle (nav.blocker)",
                        Icon::Nav,
                    ),
                ] {
                    let blocked = self.command_blocked(cmd);
                    let image = icons.image(ui.ctx(), icon, 16.0, MUTED);
                    let r = ui
                        .add_enabled(
                            blocked.is_none(),
                            egui::Button::image_and_text(image, label).frame(false),
                        )
                        .on_disabled_hover_text(blocked.unwrap_or_default());
                    self.rects.insert(format!("hierarchy:add:{preset}"), r.rect);
                    if r.clicked() {
                        preset_picked = Some(cmd);
                        theme::close_dropdown(ui, id);
                    }
                }
            });
            if let Some(cmd) = preset_picked {
                self.run_into(cmd, actions);
            }
            theme::divider(ui);
            for (icon, cmd) in TOOLS.into_iter().take(fit) {
                let blocked = self.command_blocked(cmd);
                let title = super::commands::command(cmd).map_or(cmd, |c| c.title);
                let tip = match blocked {
                    Some(reason) => format!("{title} · {reason}"),
                    None => format!("{title}  {}", self.shortcut_text(cmd)),
                };
                let r = ui
                    .add_enabled_ui(blocked.is_none(), |ui| {
                        theme::icon_button_sized(ui, icons, icon, &tip, false, 22.0)
                    })
                    .inner;
                self.rects.insert(format!("hierarchy:{cmd}"), r.rect);
                if r.clicked() {
                    self.run_into(cmd, actions);
                }
            }
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                let more =
                    theme::icon_button_sized(ui, icons, Icon::More, "Scenes and more", false, 22.0);
                self.rects.insert("hierarchy:more".into(), more.rect);
                let id = ui.make_persistent_id("hierarchy_more");
                let mut run = None;
                theme::dropdown(ui, id, &more, |ui| {
                    ui.add_enabled_ui(!busy, |ui| self.scene_menu(ui, actions));
                    ui.separator();
                    let overflow = TOOLS.iter().skip(fit).map(|(_, cmd)| *cmd);
                    for cmd in overflow.chain([
                        "edit.select_children",
                        "object.create_prefab",
                        "edit.copy",
                        "edit.toggle_hidden",
                        "edit.toggle_locked",
                    ]) {
                        let blocked = self.command_blocked(cmd);
                        let title = super::commands::command(cmd).map_or(cmd, |c| c.title);
                        let r = ui
                            .add_enabled(
                                blocked.is_none(),
                                egui::Button::new(title).shortcut_text(self.shortcut_text(cmd)),
                            )
                            .on_disabled_hover_text(blocked.unwrap_or_default());
                        if r.clicked() {
                            run = Some(cmd);
                            theme::close_dropdown(ui, id);
                        }
                    }
                });
                if let Some(cmd) = run {
                    self.run_into(cmd, actions);
                }
            });
        });
        // Search + scene filter.
        ui.horizontal(|ui| {
            let filter_on = self.scene.is_some();
            let width = (ui.available_width() - 40.0).max(60.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search…  tag: has: prefab:")
                    .desired_width(width),
            )
            .on_hover_text("Space-separated filters that must all match: tag:NAME, has:COMPONENT (includes prefab-inherited), prefab:ID, or text in the ID/label");
            let tip = match &self.scene {
                Some(scene) => format!("Scene filter: {scene}"),
                None => "Scene filter: all scenes".to_owned(),
            };
            let filter = theme::icon_button(ui, icons, Icon::Filter, &tip, filter_on);
            self.rects.insert("hierarchy:filter".into(), filter.rect);
            let id = ui.make_persistent_id("hierarchy_filter");
            theme::dropdown(ui, id, &filter, |ui| {
                ui.label(RichText::new("SHOW SCENE").small().color(MUTED));
                let mut picked = None;
                if ui
                    .selectable_label(self.scene.is_none(), "All scenes")
                    .clicked()
                {
                    picked = Some(None);
                }
                for scene in &self.world.scenes {
                    let name = if scene.active {
                        scene.id.clone()
                    } else {
                        format!("{} · inactive", scene.id)
                    };
                    let on = self.scene.as_deref() == Some(scene.id.as_str());
                    let r = ui.selectable_label(on, name);
                    self.rects.insert(format!("hierarchy:scene:{}", scene.id), r.rect);
                    if r.clicked() {
                        picked = Some(Some(scene.id.clone()));
                    }
                }
                if let Some(scene) = picked {
                    self.scene = scene;
                    theme::close_dropdown(ui, id);
                }
            });
        });
        ui.separator();
        let graph = Hierarchy::new(&self.world);
        let term = self.search.to_lowercase();
        let filtered = self.scene.is_some() || !term.is_empty();
        let mut rows = Vec::new();
        if filtered {
            rows.extend(graph.objects.iter().filter_map(|(&id, e)| {
                let matches = self.scene.as_deref().is_none_or(|scene| {
                    graph
                        .memberships
                        .get(id)
                        .is_some_and(|s| s.contains(&scene))
                }) && search_matches(&self.world, e, &term);
                matches.then_some((id, 0, false))
            }));
        } else {
            if self.scroll_selection {
                let mut parent = self.selected.as_deref();
                while let Some(id) = parent.and_then(|id| graph.parents.get(id)).copied() {
                    self.collapsed.remove(id);
                    parent = Some(id);
                }
            }
            let mut stack = vec![];
            if let Some(roots) = graph.children.get(&None) {
                stack.extend(roots.iter().rev().map(|id| (*id, 0)));
            }
            while let Some((id, depth)) = stack.pop() {
                let children = graph.children.get(&Some(id));
                rows.push((id, depth, children.is_some()));
                if let Some(children) = children.filter(|_| !self.collapsed.contains(id)) {
                    stack.extend(children.iter().rev().map(|id| (*id, depth + 1)));
                }
            }
        }
        // The count line is also where a dragged row is dropped to become a root object.
        let (_, root_drop) = ui.dnd_drop_zone::<String, ()>(egui::Frame::NONE, |ui| {
            ui.horizontal(|ui| {
                caption(ui, format!("{} objects", rows.len()));
                let extra = self.selection().len();
                if extra > 1 {
                    caption(ui, format!("· {extra} selected"));
                }
                caption(ui, "· drop here = root");
            })
            .response
            .on_hover_text(
                "Drag a row: onto a row = child · onto its top/bottom edge = reorder · here = root",
            );
        });
        let mut reparent = root_drop.map(|id| ((*id).clone(), None));
        let modifiers = ui.input(|i| i.modifiers);
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        // Editor-view flags (own = set on this row; effective = inherited from an ancestor too).
        let own_hidden = self.view_flags(false).cloned().unwrap_or_default();
        let own_locked = self.view_flags(true).cloned().unwrap_or_default();
        let hidden = self.view_flagged(false);
        let locked = self.view_flagged(true);
        let mut renaming = self.renaming.take();
        // Some(None) = cancelled, Some(Some((id, text))) = commit.
        let mut rename_done: Option<Option<(String, String)>> = None;
        let mut picked = None;
        let mut secondary = None;
        let mut toggle = None;
        let mut flag_toggle = None;
        let mut menu_run: Option<&'static str> = None;
        let mut reorder = None;
        let mut rects = std::collections::HashMap::new();
        let mut scroll = ScrollArea::vertical()
            .id_salt("editor_objects")
            .auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_selection) {
            if let Some(i) = rows
                .iter()
                .position(|(id, _, _)| Some(*id) == self.selected.as_deref())
            {
                let height = ROW_HEIGHT + ui.spacing().item_spacing.y;
                scroll = scroll.vertical_scroll_offset(
                    (i as f32 * height - ui.available_height() * 0.4).max(0.0),
                );
            }
        }
        // Only the visible slice of rows is laid out (virtualized), so thousands of objects
        // cost about the same per frame as a few dozen.
        scroll.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
            for (id, depth, branch) in rows[range].iter().copied() {
                ui.push_id(id, |ui| {
                    let selected =
                        self.selected.as_deref() == Some(id) || self.extra.contains(id);
                    // Row background (selection) goes under everything drawn in the row.
                    let background = ui.painter().add(egui::Shape::Noop);
                    let top = ui.cursor().top();
                    let hovered = pointer.is_some_and(|p| {
                        ui.max_rect().x_range().contains(p.x) && (top..top + ROW_HEIGHT).contains(&p.y)
                    });
                    let row = ui.horizontal(|ui| {
                        ui.set_min_height(ROW_HEIGHT);
                        ui.spacing_mut().item_spacing.x = 3.0;
                        ui.add_space(depth.min(7) as f32 * 12.0);
                        let entity = graph.objects[id];
                        if branch {
                            let collapsed = self.collapsed.contains(id);
                            let icon = if collapsed {
                                Icon::ChevronRight
                            } else {
                                Icon::ChevronDown
                            };
                            let r = theme::icon_button_sized(
                                ui,
                                icons,
                                icon,
                                if collapsed { "Expand" } else { "Collapse" },
                                false,
                                16.0,
                            );
                            rects.insert(format!("fold:{id}"), r.rect);
                            if r.clicked() {
                                toggle = Some(id.to_owned());
                            }
                        } else {
                            ui.add_space(16.0);
                        }
                        let (icon, tip) = entity_icon(&self.world, entity, branch);
                        let tint = if selected { theme::TEXT } else { MUTED };
                        let (rect, r) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        icons.paint(ui.painter(), rect, icon, tint);
                        r.on_hover_ui(|ui| {
                            ui.label(tip);
                            for (text, _) in badges(&self.world, entity) {
                                ui.label(RichText::new(badge_meaning(text)).small());
                            }
                        });
                        if let Some(r) = renaming.as_mut().filter(|r| r.id == id) {
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut r.text)
                                    .id(egui::Id::new(("hierarchy_rename", id)))
                                    .margin(egui::Margin::symmetric(4, 1))
                                    .desired_width(ui.available_width() - 4.0),
                            );
                            rects.insert(format!("rename:{id}"), edit.rect);
                            if !r.focused {
                                edit.request_focus();
                                r.focused = true;
                            } else if escape {
                                rename_done = Some(None);
                            } else if edit.lost_focus() {
                                // Enter, Tab or clicking elsewhere commits (as in Unity/Godot).
                                rename_done = Some(Some((id.to_owned(), r.text.clone())));
                            }
                            return;
                        }
                        let color = if hidden.contains(id) {
                            MUTED.gamma_multiply(0.6)
                        } else if !graph.active(id) {
                            MUTED
                        } else if entity.prefab.is_some() {
                            theme::PREFAB
                        } else {
                            MUTED
                        };
                        let label = entity.label.as_deref().unwrap_or(id);
                        // Filtered rows are flat; show the ancestor path so context isn't lost.
                        let crumb = if filtered {
                            let mut chain = vec![];
                            let mut cursor = graph.parents.get(id).copied();
                            while let Some(p) = cursor.filter(|_| chain.len() < 2) {
                                chain.push(graph.objects[p].label.as_deref().unwrap_or(p));
                                cursor = graph.parents.get(p).copied();
                            }
                            chain.reverse();
                            chain
                                .iter()
                                .map(|c| format!("{} › ", truncate(c, 10)))
                                .collect()
                        } else {
                            String::new()
                        };
                        let shown = format!("{crumb}{}", truncate(label, 24));
                        // One widget senses click *and* drag. (egui's `dnd_drag_source` lays a
                        // drag-only widget over the label, and egui 0.31 then routes clicks to
                        // nobody, so rows could not be clicked or right-clicked.)
                        let response = ui
                            .selectable_label(selected, RichText::new(&shown).color(color))
                            .interact(egui::Sense::click_and_drag());
                        if response.drag_started_by(egui::PointerButton::Primary) {
                            egui::DragAndDrop::set_payload(ui.ctx(), id.to_owned());
                        }
                        if response.dragged_by(egui::PointerButton::Primary) {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            if let Some(p) = pointer {
                                ui.ctx()
                                    .layer_painter(egui::LayerId::new(
                                        egui::Order::Tooltip,
                                        egui::Id::new("hierarchy_drag_ghost"),
                                    ))
                                    .text(
                                        p + egui::vec2(14.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        &shown,
                                        egui::FontId::proportional(13.0),
                                        ACCENT,
                                    );
                            }
                        }
                        let response = response.on_hover_text(id);
                        rects.insert(format!("label:{id}"), response.rect);
                        if response.clicked() {
                            picked = Some(id.to_owned());
                        }
                        if response.secondary_clicked() && !selected {
                            secondary = Some(id.to_owned());
                        }
                        response.context_menu(|ui| {
                            ui.set_min_width(220.0);
                            let mut item = |ui: &mut Ui, cmd: &'static str, title: &str| {
                                let blocked = self.command_blocked(cmd);
                                let r = ui
                                    .add_enabled(
                                        blocked.is_none(),
                                        egui::Button::new(title)
                                            .shortcut_text(self.shortcut_text(cmd)),
                                    )
                                    .on_disabled_hover_text(blocked.unwrap_or_default());
                                rects.insert(format!("menu:{cmd}"), r.rect);
                                if r.clicked() {
                                    menu_run = Some(cmd);
                                    ui.close_menu();
                                }
                            };
                            ui.menu_button("Create Child", |ui| {
                                item(ui, "object.child_empty", "Empty");
                                item(ui, "object.child_cube", "Cube");
                                item(ui, "object.child_camera", "Camera");
                            });
                            item(ui, "edit.rename", "Rename");
                            item(ui, "edit.duplicate", "Duplicate");
                            item(ui, "edit.delete", "Delete");
                            item(ui, "object.group", "Create Group");
                            ui.separator();
                            item(ui, "edit.copy", "Copy ID");
                            item(ui, "file.copy_context", "Copy Context for AI");
                            item(ui, "edit.select_children", "Select Children");
                            item(ui, "object.create_prefab", "Create Prefab From Selection");
                            item(ui, "edit.frame", "Focus");
                            ui.separator();
                            item(ui, "edit.toggle_hidden", "Hide / Show (Editor View)");
                            item(ui, "edit.toggle_locked", "Lock / Unlock (Editor View)");
                        });
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(6.0);
                            let show = selected || hovered;
                            let lock = view_toggle(ui, icons, own_locked.contains(id), locked.contains(id), true, show)
                                .on_hover_text("Lock / unlock in the editor view (L): not pickable or draggable in the scene view.");
                            rects.insert(format!("lock:{id}"), lock.rect);
                            if lock.clicked() {
                                flag_toggle = Some((id.to_owned(), true));
                            }
                            let eye = view_toggle(ui, icons, own_hidden.contains(id), hidden.contains(id), false, show)
                                .on_hover_text("Hide / show in the editor view (H). Play and content are unaffected.");
                            rects.insert(format!("eye:{id}"), eye.rect);
                            if eye.clicked() {
                                flag_toggle = Some((id.to_owned(), false));
                            }
                        });
                    });
                    let rect = row.response.rect;
                    if selected {
                        let full = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range());
                        ui.painter().set(
                            background,
                            egui::Shape::rect_filled(full, theme::RADIUS, theme::ACCENT_SOFT),
                        );
                        if self.selected.as_deref() == Some(id) {
                            ui.painter().rect_stroke(
                                full,
                                theme::RADIUS,
                                egui::Stroke::new(1.0_f32, ACCENT),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                    rects.insert(format!("row:{id}"), rect);
                    let zone = |y: f32| {
                        let edge = rect.height() * 0.28;
                        if filtered {
                            Drop::Into
                        } else if y < rect.top() + edge {
                            Drop::Before
                        } else if y > rect.bottom() - edge {
                            Drop::After
                        } else {
                            Drop::Into
                        }
                    };
                    if let (Some(dragged), Some(p)) =
                        (row.response.dnd_hover_payload::<String>(), pointer)
                    {
                        if dragged.as_str() != id {
                            let painter = ui.painter();
                            let line = egui::Stroke::new(2.0_f32, ACCENT);
                            match zone(p.y) {
                                Drop::Before => {
                                    painter.hline(rect.x_range(), rect.top(), line);
                                }
                                Drop::After => {
                                    painter.hline(rect.x_range(), rect.bottom(), line);
                                }
                                Drop::Into => {
                                    painter.rect_stroke(
                                        rect,
                                        2.0,
                                        egui::Stroke::new(1.5_f32, ACCENT),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                            }
                        }
                    }
                    if let Some(dragged) = row.response.dnd_release_payload::<String>() {
                        if dragged.as_str() != id {
                            match pointer.map_or(Drop::Into, |p| zone(p.y)) {
                                Drop::Into => {
                                    reparent = Some(((*dragged).clone(), Some(id.to_owned())))
                                }
                                Drop::Before => {
                                    reorder = Some(((*dragged).clone(), id.to_owned(), false))
                                }
                                Drop::After => {
                                    reorder = Some(((*dragged).clone(), id.to_owned(), true))
                                }
                            }
                        }
                    }
                });
            }
        });
        self.rects.extend(rects);
        self.renaming = renaming;
        if let Some(done) = rename_done {
            self.renaming = None;
            if let Some((id, text)) = done {
                actions.extend(self.rename_entity(&id, &text));
            }
        }
        if let Some(id) = toggle {
            if !self.collapsed.remove(&id) {
                self.collapsed.insert(id);
            }
        }
        if let Some(id) = picked {
            self.click_select(Some(id), modifiers);
        }
        if let Some(id) = secondary {
            self.click_select(Some(id), egui::Modifiers::NONE);
        }
        if let Some((id, locked)) = flag_toggle {
            self.set_view_flag(&[id], locked);
        }
        if let Some(cmd) = menu_run {
            self.run_into(cmd, actions);
        }
        if let Some((entity, parent)) = reparent.filter(|_| !self.dirty()) {
            actions.extend(self.reparent_drop(&entity, parent.as_deref()));
        }
        if let Some((dragged, target, after)) = reorder.filter(|_| !self.dirty()) {
            actions.extend(self.reorder_drop(&dragged, &target, after));
        }
    }

    fn inspector(
        &mut self,
        ui: &mut Ui,
        runtime: &WorldRuntime,
        actions: &mut Vec<Action>,
        floating: bool,
    ) {
        if self.resource.is_some() {
            self.resource_inspector(ui, actions);
            return;
        }
        if self.material.is_some() {
            self.material_inspector(ui, actions);
            return;
        }
        if self.prefab.is_some() {
            self.prefab_inspector(ui, actions);
            return;
        }
        let path = self.selected.as_deref().and_then(|id| self.path_spline(id));
        let mut path_point = self.scene_input.path_point;
        let mut path_command = None;
        let tween = self.selected.as_deref().and_then(|id| self.tween_clips(id));
        // Play on rows, rebuilt only when the world or the timeline clip changes (moved out
        // for the frame and put back at the end, so drawing them allocates nothing).
        let mut bindings_cache = self.bindings_cache.take();
        let no_bindings = super::timeline::Bindings::default();
        if let Some((id, clip, _)) = self.tween_target().filter(|_| tween.is_some()) {
            let key = (self.world_version, id, clip);
            if bindings_cache.as_ref().map(|c| &c.0) != Some(&key) {
                let rows = super::timeline::Bindings::of(&self.world, &key.1, &key.2);
                bindings_cache = Some((key, rows));
            }
        } else {
            bindings_cache = None;
        }
        let bindings = bindings_cache.as_ref().map_or(&no_bindings, |c| &c.1);
        let mut tween_ui = self.tween_ui.clone();
        let mut tween_request = None;
        let audio = self.selected.as_deref().and_then(|id| self.audio_clips(id));
        let mut audio_clip: Option<String> = None;
        self.refresh_tag_cache();
        self.refresh_mixed();
        let mut tag_request = None;
        let mut add_component = None;
        let mut component_request = None;
        let mut new_clip = false;
        let spec = self
            .draft
            .as_ref()
            .filter(|d| !d.component.is_empty())
            .map(|d| d.component.clone())
            .map(|c| self.specs.get(&self.registry, &self.world, &c));
        let live = self.prefs.live_inspector;
        // Esc that closes a dropdown, the entity picker, a dialog or a rename is not a cancel.
        let esc_taken = ui.ctx().memory(|m| m.any_popup_open())
            || self.picker.is_some()
            || self.options.is_some()
            || self.renaming.is_some();
        let saving = self.saving;
        // Play runs a pinned revision: the Inspector shows the live runtime state and the
        // authored values read-only (edits are refused until Stop).
        let playing = self.previewing;
        let is_camera = self.draft.as_ref().is_some_and(|d| self.is_camera(&d.id));
        let mut preview_toggled = false;
        let Some(draft) = self.draft.as_mut() else {
            caption(ui, "Select an object in the hierarchy or viewport.");
            return;
        };
        let id = draft.id.clone();
        let stale = draft.rev != self.rev;
        let mut reload = false;
        let mut copy = false;
        let mut write = None;
        let mut issue = None;
        let mut merge_draft = false;
        if stale && !playing {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Stale draft · revision {}", draft.rev)).color(WARNING));
                if draft.dirty() {
                    let r = ui
                        .button("Merge…")
                        .on_hover_text("Pick, field by field, your draft's or the newer revision's value where both changed; commit as one revision");
                    self.rects.insert("inspector:merge".into(), r.rect);
                    merge_draft = r.clicked();
                }
            });
        }
        if playing {
            ui.label(
                RichText::new("Playing · read-only (stop Play to edit)")
                    .small()
                    .color(WARNING),
            );
        }
        ui.add_enabled_ui(!playing, |ui| ui.horizontal(|ui| {
            ui.checkbox(&mut self.prefs.live_inspector, "Live").on_hover_text(
                "On: edits commit on release / Enter / focus loss (one revision each); Esc cancels. Off: edit a draft, then Apply.",
            );
            // With Live on, the draft buttons only matter while something is unapplied.
            if self.prefs.live_inspector && !draft.dirty() {
                caption(ui, "edits commit as you go");
                return;
            }
            for (title, preview) in [("Apply", false), ("Preview", true)] {
                if ui
                    .add_enabled(
                        draft.dirty() && !stale && self.drag.is_none(),
                        egui::Button::new(title),
                    )
                    .clicked()
                {
                    write = Some(preview);
                }
            }
            reload = ui
                .add_enabled(draft.dirty(), egui::Button::new("Reload"))
                .clicked();
            copy = ui
                .add_enabled(draft.dirty(), egui::Button::new("Copy JSON"))
                .clicked();
        }));
        let mut duplicate = false;
        let mut delete = false;
        let mut structural = None;
        let entity = self.world.entity(&id);
        // Another object starts at the top of the Inspector (its timeline and transform).
        let new_object = self.inspector_shown.as_deref() != Some(id.as_str());
        if new_object {
            self.inspector_shown = Some(id.clone());
        }
        if let Some(entity) = entity {
            let mut scroll = ScrollArea::vertical().auto_shrink([false, false]);
            if new_object {
                scroll = scroll.vertical_scroll_offset(0.0);
            }
            scroll.show(ui, |ui| {
                    if is_camera {
                        preview_toggled = super::game::camera_preview_hole(
                            ui,
                            self.prefs.camera_preview,
                            &mut self.camera_preview,
                            &mut self.rects,
                            &id,
                            floating,
                        );
                    }
                    if playing {
                        self.live.shown = true;
                        super::game::live_panel(ui, &self.live, runtime, &id);
                    }
                    let editable = !stale && self.drag.is_none() && !playing;
                    let icons = self.icons;
                    theme::card().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            icons.show(ui, Icon::Entity, 16.0, ACCENT);
                            ui.label(RichText::new("Entity").strong().color(theme::TEXT));
                        });
                        // Label column by fixed width, not a Grid: a new egui Grid discards
                        // its first pass, which would drop that frame's input.
                        let field = |ui: &mut Ui, name: &str| {
                            ui.add_sized([62.0, 20.0], egui::Label::new(RichText::new(name).color(MUTED)));
                        };
                        {
                            {
                                ui.horizontal(|ui| {
                                field(ui, "Name");
                                ui.add_enabled_ui(editable, |ui| {
                                    draft.label_dirty |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut draft.label)
                                                .hint_text("Uses stable ID if blank")
                                                .desired_width(f32::INFINITY),
                                        )
                                        .changed();
                                });
                                });
                                ui.horizontal(|ui| {
                                    field(ui, "ID");
                                    ui.label(RichText::new(truncate(&id, 30)).monospace().color(theme::TEXT))
                                        .on_hover_text(&id);
                                    let r = theme::icon_button_sized(ui, icons, Icon::Copy, "Copy ID", false, 20.0);
                                    self.rects.insert("inspector:copy_id".into(), r.rect);
                                    if r.clicked() {
                                        ui.ctx().copy_text(id.clone());
                                    }
                                });
                                if let Some(prefab) = &entity.prefab {
                                    ui.horizontal(|ui| {
                                        field(ui, "Prefab");
                                        ui.label(RichText::new(prefab).color(theme::PREFAB));
                                    });
                                }
                            }
                        }
                        ui.add_enabled_ui(editable, |ui| {
                            tag_request = super::world_panel::tag_chips(
                                ui,
                                &mut self.tag_ui,
                                !draft.dirty(),
                                &mut self.rects,
                            );
                        });
                    });
                    ui.add_enabled_ui(editable, |ui| {
                        if !self.extra.is_empty() {
                            ui.label(
                                RichText::new(
                                    "Several objects selected · Transform and component field edits apply to each one that has the field",
                                )
                                .small()
                                .color(WARNING),
                            );
                        }
                        ui.add_space(8.0);
                        // First for animated objects: the inspector is short on laptop screens
                        // and the timeline is what one edits on them (Transform follows).
                        if let Some(clips) = &tween {
                            theme::section(ui, icons, Icon::Tween, "Tween timeline", true, |ui| {
                                    let live = tween_ui.live.clone();
                                    let editable = !draft.dirty() && live.is_none();
                                    tween_request = super::timeline::timeline(
                                        ui,
                                        clips,
                                        &mut tween_ui,
                                        live.as_ref().map(|(c, t)| (c.as_str(), *t)),
                                        editable,
                                        bindings,
                                        Some(&mut self.rects),
                                    );
                                });
                        }
                        theme::section(ui, icons, Icon::Move, "Transform", true, |ui| {
                                if let Some(t) = &mut draft.transform {
                                    // "—" on an axis whose value differs across the selection,
                                    // until it is edited.
                                    let base = draft.base_transform.clone().unwrap_or_default();
                                    let base_angles = viewport::degrees(&base);
                                    let m = self.mixed.transform;
                                    let unchanged = |k: usize, a: [f32; 3], b: [f32; 3]| a[k] == b[k];
                                    let show = |row: usize, a: [f32; 3], b: [f32; 3]| {
                                        [0, 1, 2].map(|k| m[row][k] && unchanged(k, a, b))
                                    };
                                    let mixed_position = show(0, t.position, base.position);
                                    let mixed_angles = show(1, draft.angles, base_angles);
                                    let mixed_scale = show(2, t.scale, base.scale);
                                    let (position, z) = vector(
                                        ui,
                                        "Position",
                                        &mut t.position,
                                        0.05,
                                        -100000.0..=100000.0,
                                        mixed_position,
                                    );
                                    self.rects.insert("inspector:position.z".into(), z);
                                    let (rotate, _) = vector(
                                        ui,
                                        "Rotation°",
                                        &mut draft.angles,
                                        0.5,
                                        -36000.0..=36000.0,
                                        mixed_angles,
                                    );
                                    if rotate {
                                        t.rotation = viewport::rotation(draft.angles);
                                    }
                                    let (scale, z) = vector(
                                        ui,
                                        "Scale",
                                        &mut t.scale,
                                        0.025,
                                        0.0001..=10000.0,
                                        mixed_scale,
                                    );
                                    self.rects.insert("inspector:scale.z".into(), z);
                                    draft.transform_dirty |= position || rotate || scale;
                                } else if ui.button("Add transform").clicked() {
                                    draft.transform = Some(Transform::default());
                                    draft.transform_dirty = true;
                                }
                            });
                        theme::section(ui, icons, Icon::Hierarchy, "Parent and scenes", true, |ui| {
                                caption(ui, "PARENT ID · EMPTY FOR ROOT");
                                draft.parent_dirty |= ui
                                    .add(
                                        egui::TextEdit::singleline(&mut draft.parent)
                                            .desired_width(f32::INFINITY),
                                    )
                                    .changed();
                                ui.checkbox(&mut draft.keep_world, "Keep world pose");
                                if self.world.scenes.is_empty() {
                                    caption(ui, "Create a scene to assign membership.");
                                }
                                ui.add_enabled_ui(!draft.dirty(), |ui| {
                                    let mut assigned = false;
                                    for scene in &self.world.scenes {
                                        let mut member = scene.entities.contains(&id);
                                        assigned |= member;
                                        if ui
                                            .checkbox(&mut member, &scene.id)
                                            .on_hover_text(if scene.active {
                                                "Active scene"
                                            } else {
                                                "Inactive scene"
                                            })
                                            .changed()
                                        {
                                            structural = Some(CoreEdit::SetSceneMembership {
                                                scene: scene.id.clone(),
                                                entity: id.clone(),
                                                member,
                                            });
                                        }
                                    }
                                    if !assigned {
                                        caption(ui, "Global object · no scene membership");
                                    }
                                });
                            });
                        ui.scope(|ui| {
                                let panel = ComponentPanel {
                                    world: &self.world,
                                    world_version: self.world_version,
                                    hints: &self.component_hints,
                                    registry: &self.registry,
                                    spec: spec.as_deref(),
                                    mixed: &self.mixed.fields,
                                    filter: &mut self.field_filter,
                                    picker: &mut self.picker,
                                    rects: &mut self.rects,
                                    clipboard: self.component_clipboard.as_ref().map(|c| c.0.as_str()),
                                    live,
                                    scroll: &mut self.scroll_component,
                                    icons: self.icons,
                                };
                                component_request =
                                    component_editor(ui, panel, entity, draft, &mut add_component);
                            });
                        if let Some((clips, current)) = &audio {
                            theme::section(ui, icons, Icon::Audio, "Audio source", true, |ui| {
                                    ui.add_enabled_ui(!draft.dirty(), |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label("Clip");
                                            egui::ComboBox::from_id_salt("audio_clip")
                                                .selected_text(current.as_str())
                                                .show_ui(ui, |ui| {
                                                    for clip in clips {
                                                        if ui
                                                            .selectable_label(clip == current, clip)
                                                            .clicked()
                                                            && clip != current
                                                        {
                                                            audio_clip = Some(clip.clone());
                                                        }
                                                    }
                                                });
                                            new_clip = ui
                                                .button("New clip…")
                                                .on_hover_text("Procedural presets (no audio files); pick the preset and the clip ID")
                                                .clicked();
                                        });
                                    });
                                    caption(
                                        ui,
                                        "Edit volume, loop, distances and rolloff under Components › audio.source; play with call audio.play in Play",
                                    );
                                });
                        }
                        if let Some(spline) = &path {
                            theme::section(ui, icons, Icon::Path, "Path points", true, |ui| {
                                    caption(
                                        ui,
                                        format!(
                                            "{} points · {} · local · drag the handles in the scene view",
                                            spline.points.len(),
                                            if spline.closed { "closed" } else { "open" }
                                        ),
                                    );
                                    for (k, p) in spline.points.iter().enumerate() {
                                        let text =
                                            format!("{k}: {:.2}, {:.2}, {:.2}", p[0], p[1], p[2]);
                                        if ui
                                            .selectable_label(
                                                path_point == Some(k),
                                                RichText::new(text).monospace(),
                                            )
                                            .clicked()
                                        {
                                            path_point = Some(k);
                                        }
                                    }
                                    ui.add_enabled_ui(!draft.dirty(), |ui| {
                                        ui.horizontal(|ui| {
                                            if ui.button("Add point").clicked() {
                                                path_command = Some("path.add_point");
                                            }
                                            if ui.button("Remove point").clicked() {
                                                path_command = Some("path.remove_point");
                                            }
                                        });
                                    });
                                });
                        }
                        ui.separator();
                        ui.add_enabled_ui(!draft.dirty(), |ui| {
                            ui.horizontal(|ui| {
                                duplicate = ui.button("Duplicate").clicked();
                                ui.menu_button("Delete…", |ui| {
                                    ui.label("Delete this object?");
                                    caption(ui, "Children become roots and keep local transforms.");
                                    if ui
                                        .button(RichText::new("Delete object").color(WARNING))
                                        .clicked()
                                    {
                                        delete = true;
                                        ui.close_menu();
                                    }
                                });
                            });
                        });
                    });
                });
        } else {
            caption(
                ui,
                "Object was deleted in another revision. Copy or reload the draft.",
            );
        }
        // Live Inspector: Esc while editing goes back to the committed values; otherwise an
        // edited draft commits once nothing is held or typed any more (drag released, Enter,
        // focus left the field, checkbox / dropdown picked): one revision per interaction.
        let mut cancel = false;
        let mut live_commit = false;
        if draft.dirty() && !stale && self.drag.is_none() && write.is_none() && !copy {
            let (escape, held) =
                ui.input(|i| (i.key_pressed(egui::Key::Escape), i.pointer.any_down()));
            let typing = ui
                .ctx()
                .memory(|m| m.focused())
                .is_some_and(|f| egui::TextEdit::load_state(ui.ctx(), f).is_some());
            if escape && live && !esc_taken && !draft.component_dirty {
                cancel = true;
                // The drag or typing in progress ends with the cancel.
                let ctx = ui.ctx();
                ctx.stop_dragging();
                if let Some(f) = ctx.memory(|m| m.focused()) {
                    ctx.memory_mut(|m| m.surrender_focus(f));
                }
            } else if live && !draft.component_dirty && !held && !typing && saving == 0 {
                // While an earlier edit is saving the draft waits (kept by `apply_staged`),
                // so the writer never refuses it as "still being prepared".
                write = Some(false);
                live_commit = true;
            }
        }
        if copy || write.is_some() {
            match super::inspector_edits(&self.world, &self.registry, draft, &self.extra) {
                Ok(edits) if edits.is_empty() && live_commit => draft.mark_committed(),
                Ok(edits) if !edits.is_empty() => {
                    if copy {
                        ui.ctx().copy_text(
                            serde_json::to_string_pretty(&Request::Apply {
                                expected_rev: Some(draft.rev),
                                edits: edits.clone(),
                            })
                            .unwrap(),
                        );
                    }
                    if let Some(preview) = write {
                        let expected_rev = Some(draft.rev);
                        let request = if preview {
                            Request::Preview {
                                expected_rev,
                                edits,
                            }
                        } else {
                            Request::Apply {
                                expected_rev,
                                edits,
                            }
                        };
                        actions.push(Action::Write {
                            request,
                            select: None,
                        });
                        if live_commit {
                            draft.mark_committed();
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => issue = Some(format!("Invalid component JSON · {error}")),
            }
        }
        if let Some(error) = issue {
            self.set_error(error);
        }
        if reload || cancel {
            self.select(self.selected.clone());
            self.notice(if cancel {
                "Edit cancelled (Esc)"
            } else {
                "Reloaded the committed object"
            });
        }
        if let Some(edit) = structural {
            actions.push(self.write(vec![Edit::Core(edit)], None));
        }
        match component_request {
            Some(ComponentRequest::Remove(component)) => {
                actions.extend(self.remove_component(&id, &component));
            }
            Some(ComponentRequest::Reset(component)) => {
                actions.extend(self.reset_component(&component));
            }
            Some(ComponentRequest::Copy(component)) => self.copy_component(&component),
            Some(ComponentRequest::Paste(component)) => {
                actions.extend(self.paste_component(&component));
            }
            Some(ComponentRequest::Array {
                component,
                pointer,
                edit,
            }) => actions.extend(self.edit_component_array(&component, &pointer, &edit)),
            Some(ComponentRequest::MainCamera) => actions.extend(self.make_main_camera()),
            Some(ComponentRequest::CameraBlocker(on)) => {
                actions.extend(self.camera_blocker(std::slice::from_ref(&id), on))
            }
            Some(ComponentRequest::Play(component)) => match self.play_clip(&component) {
                Some(action) => actions.push(action),
                None => self.set_error("Nothing to play: pick a clip on the timeline"),
            },
            Some(ComponentRequest::EditGraph) => {
                let machine = self.world.entity(&id).and_then(|e| {
                    component_value(&self.world, e, "fsm.agent")?["machine"]
                        .as_str()
                        .map(str::to_owned)
                });
                if let Err(e) = machine.as_deref().map_or(Ok(()), |m| self.graph_open(m)) {
                    self.problem(&e);
                }
            }
            None => {}
        }
        if new_clip {
            self.options = super::options::OptionsDialog::for_command(self, "audio.new_clip");
        }
        self.scene_input.path_point = path_point;
        self.tween_ui = tween_ui;
        match tween_request {
            Some(super::timeline::TweenRequest::Command(command)) => {
                match self.run_command(command) {
                    Ok(more) => actions.extend(more),
                    Err(error) => self.set_error(format!("{error:#}")),
                }
            }
            Some(super::timeline::TweenRequest::MoveKey {
                clip,
                track,
                index,
                time,
            }) => {
                actions.extend(self.move_tween_key(&clip, track, index, time));
                self.tween_ui.key = None;
            }
            Some(super::timeline::TweenRequest::Options { clip, options }) => {
                actions.extend(self.set_tween_options(&clip, &options));
            }
            Some(super::timeline::TweenRequest::AddTrack { clip, property }) => {
                actions.extend(self.add_tween_track(&clip, &property));
            }
            Some(super::timeline::TweenRequest::RemoveClip(clip)) => {
                actions.extend(self.remove_tween_clip(&clip));
            }
            // ACL: remove resources[id=runtime.bindings] events/B/calls C (events B when it
            // is the binding's only call).
            Some(super::timeline::TweenRequest::Unbind(play)) => {
                let (field, index) = if play.calls > 1 {
                    (format!("/events/{}/calls", play.binding), play.call)
                } else {
                    ("/events".to_owned(), play.binding)
                };
                actions.extend(self.edit_resource_array(
                    "runtime.bindings",
                    &field,
                    &axiom_core::ops::ArrayEdit::Remove(index),
                ));
            }
            None => {}
        }
        if let Some(clip) = audio_clip {
            actions.extend(self.set_audio_clip(&clip));
        }
        if let Some(request) = tag_request {
            let before = actions.len();
            actions.extend(self.edit_tags(&request.add, &request.remove));
            // The typed tag is kept when it was refused (e.g. invalid_tag), so it can be fixed.
            if !request.add.is_empty() && (actions.len() > before || !self.error) {
                self.tag_ui.input.clear();
            }
        }
        match add_component
            .as_deref()
            .map(|c| c.strip_prefix("command:").ok_or(c))
        {
            Some(Ok(command)) => {
                if let Some(cmd) = super::commands::command(command) {
                    self.invoke(cmd.id, actions);
                }
            }
            Some(Err(component)) => actions.extend(self.add_component(component)),
            None => {}
        }
        if preview_toggled {
            self.run_into("view.camera_preview", actions);
        }
        if let Some(command) = path_command {
            match self.run_command(command) {
                Ok(more) => actions.extend(more),
                Err(error) => self.set_error(format!("{error:#}")),
            }
        }
        if duplicate {
            actions.extend(self.duplicate_ids(std::slice::from_ref(&id)));
        }
        if merge_draft {
            match self.run_command_with("merge.open", Some("draft")) {
                Ok(more) => actions.extend(more),
                Err(e) => self.set_error(format!("{e:#}")),
            }
        }
        if delete {
            actions.extend(self.delete_ids(&[id]));
        }
        self.bindings_cache = bindings_cache;
    }

    /// Body of one of the former bottom-dock tabs.
    fn dock_tab(&mut self, ui: &mut Ui, tab: Dock, actions: &mut Vec<Action>) {
        let mut jump = None;
        let mut declare: Option<String> = None;
        // Proposals / History: `ghost.show` and `merge.open` arguments picked this frame.
        let mut ghost_pick: Option<String> = None;
        let mut merge_open: Option<String> = None;
        let clean_draft = !self.dirty();
        let mut rects: Vec<(String, egui::Rect)> = vec![];
        // Resource / material / prefab drafts would be reset by the write; the object draft is not.
        let asset_draft = self.dirty() && !self.draft.as_ref().is_some_and(Draft::dirty);
        ScrollArea::vertical().id_salt("editor_dock_scroll").auto_shrink([false,false]).show(ui, |ui| match tab {
            Dock::Hierarchy
            | Dock::Scene
            | Dock::Game
            | Dock::Inspector
            | Dock::World
            | Dock::UiEditor
            | Dock::Graphs
            | Dock::Assets
            | Dock::Console
            | Dock::Profiler
            | Dock::Trace
            | Dock::InputMap
            | Dock::Localization
            | Dock::Animation
            | Dock::Vfx => {}
            Dock::Changes => {
                if self.changes.is_empty() { caption(ui,"Preview an inspector draft or commit an edit to see stable change paths."); }
                for change in &self.changes { ui.monospace(change); }
            }
            Dock::Proposals => {
                if self.proposals.is_empty() {
                    caption(ui, "No pending proposals. Agents queue edits with `axiom acl --propose …` or {\"op\":\"propose\"}.");
                }
                ui.columns(2, |cols| {
                    for p in &self.proposals {
                        let id = p["id"].as_i64().unwrap_or(0);
                        let base = p["base_rev"].as_i64().unwrap_or(0);
                        let note = p["note"].as_str().unwrap_or("").lines().next().unwrap_or("");
                        let stale = if base != self.rev { "  · stale base" } else { "" };
                        let text = format!("#{id}  rev {base}  {} change(s){stale}\n{}", p["diff"]["total_changes"], truncate(note, 48));
                        let row = cols[0].selectable_label(self.proposal_focus == Some(id), RichText::new(text).monospace().small());
                        rects.push((format!("proposal:{id}"), row.rect));
                        if row.clicked() {
                            // Focusing a proposal shows its ghosts in the Scene view.
                            ghost_pick = Some(format!("proposal:{id}"));
                        }
                    }
                    let Some(p) = self.proposals.iter().find(|p| p["id"].as_i64() == self.proposal_focus) else { return };
                    let id = p["id"].as_i64().unwrap_or(0);
                    let base = p["base_rev"].as_i64().unwrap_or(0);
                    let mut merge = false;
                    cols[1].horizontal(|ui| {
                        let clean = clean_draft;
                        if ui.add_enabled(clean && base == self.rev, egui::Button::new("Approve"))
                            .on_hover_text("Apply against the proposal's base revision")
                            .clicked() { actions.push(Action::ResolveProposal { id, approve: true, rebase: false, keep: None, expected_rev: None }); }
                        if ui.add_enabled(clean, egui::Button::new("Rebase & approve"))
                            .on_hover_text("Re-validate against the current revision, then apply")
                            .clicked() { actions.push(Action::ResolveProposal { id, approve: true, rebase: true, keep: None, expected_rev: None }); }
                        merge = base != self.rev;
                        if ui.button(RichText::new("Reject").color(WARNING)).clicked() {
                            actions.push(Action::ResolveProposal { id, approve: false, rebase: false, keep: None, expected_rev: None });
                        }
                        let showing = self.prefs.ghosts && self.ghost.source == Some(super::ghost::GhostSource::Proposal(id));
                        let g = ui.selectable_label(showing, "Ghosts")
                            .on_hover_text("Show where this proposal moves, adds and removes objects in the Scene view (preview only)");
                        rects.push((format!("proposal:ghosts:{id}"), g.rect));
                        if g.clicked() {
                            ghost_pick = Some(if showing { "off".into() } else { format!("proposal:{id}") });
                        }
                    });
                    if merge {
                        cols[1].horizontal(|ui| {
                            let r = ui.add_enabled(clean_draft, egui::Button::new("Merge…"))
                                .on_hover_text("Pick, field by field, the proposal's or the current value where both changed; approve as one revision");
                            rects.push((format!("proposal:merge:{id}"), r.rect));
                            if r.clicked() {
                                merge_open = Some(format!("proposal:{id}"));
                            }
                        });
                        cols[1].label(RichText::new(format!("Base revision {base} is behind current {}: rebase replays the same edits on the latest world; Merge… lets you pick per field where both changed.", self.rev)).small().color(WARNING));
                    }
                    for change in p["diff"]["changes"].as_array().into_iter().flatten() {
                        let path = change["path"].as_str().unwrap_or("?");
                        let short = |v: &Value| { let s = axiom_core::local::display_value(v).to_string(); if s.chars().count() > 40 { format!("{}…", s.chars().take(39).collect::<String>()) } else { s } };
                        let line = match change["kind"].as_str() {
                            Some("added") => format!("+ {path} = {}", short(&change["after"])),
                            Some("removed") => format!("- {path}"),
                            _ => format!("~ {path}: {} → {}", short(&change["before"]), short(&change["after"])),
                        };
                        let label = cols[1].label(RichText::new(line).monospace().small());
                        if let Some(target) = super::entity_in_path(path, &self.world) {
                            if label.on_hover_text("Click to select").clicked() { jump = Some(target); }
                        }
                    }
                    if p["diff"]["truncated"] == true { cols[1].label(RichText::new("… diff truncated to 64 changes").small().color(MUTED)); }
                });
            }
            Dock::History => {
                ui.columns(2, |cols| {
                    cols[0].label(RichText::new("REVISIONS · newest first").small().color(MUTED));
                    for row in &self.history {
                        let rev = row["rev"].as_i64().unwrap_or(0);
                        let current = rev == self.rev;
                        let selected = self.history_diff.as_ref().is_some_and(|(r, _)| *r == rev);
                        cols[0].horizontal(|ui| {
                            let text = format!(
                                "{}{rev:>4}  {:<14} {}",
                                if current { "● " } else { "  " },
                                truncate(row["reason"].as_str().unwrap_or(""), 14),
                                row["created_at"].as_str().unwrap_or("").get(11..19).unwrap_or("")
                            );
                            // Who made it: an approved proposal, an agent, the person, or the
                            // system (initialize / migrations / older databases).
                            let (tag, color, tip) = match (row["proposal"].as_i64(), row["author"].as_str()) {
                                (Some(p), by) => (format!("#{p}"), theme::PREFAB, format!("Approved proposal #{p} (approved by {})", by.unwrap_or("?"))),
                                (None, Some("agent")) => ("AI".into(), theme::WARNING, "Committed by an agent (bridge, CLI or editor_command)".into()),
                                (None, Some("human")) => ("You".into(), theme::SUCCESS, "Committed in the editor".into()),
                                (None, _) => ("—".into(), MUTED, "No author recorded (initialize, migration or an older build)".into()),
                            };
                            // Fixed-width cell so the revision columns line up.
                            let chip = ui
                                .allocate_ui_with_layout(egui::vec2(46.0, 20.0), Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.set_min_size(egui::vec2(46.0, 20.0));
                                    theme::chip(ui, tag, color)
                                })
                                .inner
                                .on_hover_text(tip);
                            rects.push((format!("history:author:{rev}"), chip.rect));
                            let row_label = ui.selectable_label(selected, RichText::new(text).monospace());
                            rects.push((format!("history:{rev}"), row_label.rect));
                            if row_label.clicked() && rev > 1 {
                                actions.push(Action::LoadDiff { rev });
                                // The Scene view shows what this revision moved.
                                ghost_pick = Some(format!("rev:{rev}"));
                            }
                            if !current && ui.add_enabled(clean_draft, egui::Button::new("Restore").small())
                                .on_hover_text(format!("Append a revision restoring {rev}; history is kept"))
                                .clicked()
                            {
                                actions.push(Action::RestoreTo { rev });
                            }
                        });
                    }
                    match &self.history_diff {
                        None => { cols[1].label(RichText::new("Select a revision to see its before → after changes.").small().color(MUTED)); }
                        Some((rev, diff)) => {
                            cols[1].label(RichText::new(format!("{} → {rev} · {} change(s)", rev - 1, diff["total_changes"])).small().color(MUTED));
                            for change in diff["changes"].as_array().into_iter().flatten() {
                                let path = change["path"].as_str().unwrap_or("?");
                                let short = |v: &Value| { let s = axiom_core::local::display_value(v).to_string(); if s.len() > 40 { format!("{}…", &s[..s.char_indices().nth(39).map_or(s.len(), |(i, _)| i)]) } else { s } };
                                let line = match change["kind"].as_str() {
                                    Some("added") => format!("+ {path} = {}", short(&change["after"])),
                                    Some("removed") => format!("- {path}"),
                                    _ => format!("~ {path}: {} → {}", short(&change["before"]), short(&change["after"])),
                                };
                                let label = cols[1].label(RichText::new(line).monospace().small());
                                if let Some(id) = super::entity_in_path(path, &self.world) {
                                    if label.on_hover_text("Click to select").clicked() { jump = Some(id); }
                                }
                            }
                            if diff["next_offset"].is_number() { cols[1].label(RichText::new("… more changes; use `axiom acl -e \"diff A B\"`").small().color(MUTED)); }
                        }
                    }
                });
            }
            Dock::Scenarios => {
                if self.scenarios.is_empty() {
                    caption(ui, "No examples/*.scenario.json found in the working directory.");
                }
                if let Some(job) = &self.scenario_job {
                    let (done, total) = job.progress();
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::ProgressBar::new(if total == 0 { 0.0 } else { done as f32 / total as f32 })
                                .desired_width(220.0)
                                .text(format!("{done} / {total} ticks · {:.1} s", job.elapsed().as_secs_f32())),
                        );
                        ui.label(RichText::new(&job.label).monospace().small());
                        if ui.button("Cancel").on_hover_text("Stop the scenario at its next tick").clicked() {
                            actions.push(Action::CancelScenario);
                        }
                    });
                }
                let running = self.scenario_job.is_some();
                for path in &self.scenarios {
                    let result = self.scenario_results.get(path);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!self.dirty() && !running, egui::Button::new("Run").small())
                            .on_hover_text("Headless, deterministic run against the current revision")
                            .clicked()
                        {
                            actions.push(Action::RunScenario { path: path.clone() });
                        }
                        let (badge, color) = match result.map(|r| r["ok"] == true) {
                            Some(true) => ("PASS", Color32::from_rgb(120, 220, 140)),
                            Some(false) => ("FAIL", WARNING),
                            None => ("    ", MUTED),
                        };
                        ui.label(RichText::new(badge).monospace().strong().color(color));
                        ui.label(RichText::new(path).monospace());
                        if let Some(hash) = result.and_then(|r| r["state_hash"].as_str()) {
                            ui.label(RichText::new(format!("ticks {} · hash {hash}", result.unwrap()["ticks"])).small().color(MUTED));
                        }
                    });
                    if let Some(r) = result {
                        ui.indent(path, |ui| {
                            if r["ok"] == true {
                                for a in r["assertions"].as_array().into_iter().flatten() {
                                    let kind = a["assertion"]["kind"].as_str().unwrap_or("?");
                                    let target = ["entity", "action", "name", "pointer", "id"].iter().find_map(|k| a["assertion"][*k].as_str()).unwrap_or("");
                                    ui.label(RichText::new(format!("✓ {kind} {target} = {}", axiom_core::local::display_value(&a["actual"]))).small().monospace());
                                }
                            } else {
                                ui.label(RichText::new(r["error"].as_str().unwrap_or("failed")).small().color(WARNING));
                            }
                        });
                    }
                }
            }
            Dock::Problems => {
                ui.horizontal(|ui| {
                    if ui.button("Verify world").clicked() { actions.push(Action::Verify); }
                    if !self.problems.is_empty() && ui.button("Clear").clicked() { self.problems.clear(); }
                });
                if self.problems.is_empty() { caption(ui,"No problems. Failed edits and verification errors appear here with their paths."); }
                self.graph_problem_rows(ui);
                for p in &self.problems {
                    let path = p["path"].as_str().unwrap_or("");
                    let target = super::entity_in_path(path, &self.world);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(p["code"].as_str().unwrap_or("error")).monospace().color(WARNING));
                        let link = ui.add(egui::Label::new(RichText::new(path).monospace()).sense(egui::Sense::click()));
                        if let Some(id) = &target {
                            if link.on_hover_text(format!("Select {id}")).clicked() { jump = Some(id.clone()); }
                        }
                        let message = p["message"].as_str().unwrap_or("");
                        ui.label(message);
                        // Declare-on-use: an event typed into a field that behavior.program
                        // does not declare yet (declared and applied with the Inspector draft).
                        if let Some(name) = axiom_core::ops::undeclared_event(message) {
                            let button = ui
                                .add_enabled(self.saving == 0 && !asset_draft, egui::Button::new(RichText::new(format!("Declare event {name}")).color(ACCENT)))
                                .on_hover_text("Adds it to behavior.program (ACL: event declare NAME); a waiting Inspector draft is applied with it");
                            #[cfg(test)]
                            self.rects.insert(format!("problem:declare:{name}"), button.rect);
                            if button.clicked() { declare = Some(name.to_owned()); }
                        }
                    });
                }
            }
            Dock::Agent => {
                ui.horizontal(|ui| {
                    if ui.button("Copy discovery").clicked() {
                        ui.ctx().copy_text(r#"{"op":"discover","topic":"editor"}"#.to_owned());
                    }
                    if ui.button("Copy scene query").clicked() {
                        ui.ctx().copy_text(json!({"op":"scene_tree","query":{"scene":self.scene,"limit":32}}).to_string());
                    }
                    if ui.add_enabled(self.selected.is_some(), egui::Button::new("Copy selection context"))
                        .on_hover_text("Compact text of every selected object (effective components) to paste into an AI conversation")
                        .clicked()
                    {
                        ui.ctx().copy_text(self.context_text());
                        self.status = format!("Copied context for {} object(s)", self.selection().len());
                    }
                    if let Some(request) = &self.last_request {
                        if ui.button("Copy last request").clicked() {
                            ui.ctx().copy_text(serde_json::to_string_pretty(request).unwrap());
                        }
                    }
                });
                caption(ui,"Stable IDs · expected revision · local diffs · durable execute receipts");
                if let Some((request, response)) = self.last_bytes {
                    caption(ui, format!("Last request {request} B → response {response} B (bytes, not tokens)"));
                }
                if self.selected.is_some() {
                    ui.label(RichText::new(self.context_text()).monospace().small());
                }
            }
        });
        if let Some(id) = jump {
            self.try_select(Some(id));
            self.frame_selection = true;
        }
        if let Some(name) = declare {
            actions.extend(self.declare_event(&name));
        }
        self.rects.extend(rects);
        for (id, arg) in [("ghost.show", ghost_pick), ("merge.open", merge_open)] {
            if let Some(arg) = arg {
                match self.run_command_with(id, Some(&arg)) {
                    Ok(more) => actions.extend(more),
                    Err(e) => self.set_error(format!("{e:#}")),
                }
            }
        }
    }
}

/// Draws the editor's panels as dock tabs for one frame.
struct TabHost<'a> {
    editor: &'a mut Editor,
    runtime: &'a WorldRuntime,
    actions: &'a mut Vec<Action>,
    /// The Scene tab body was drawn.
    scene: bool,
    /// The Game tab body was drawn.
    game: bool,
    /// The Inspector is in a floating window (no camera preview hole there).
    inspector_floating: bool,
    /// Tabs alone in their leaf (bits by `Dock::ALL` index): their tab is the panel's title,
    /// not a pill.
    alone: u32,
    /// Active tab -> its leaf's tab strip, for the "…" panel menu.
    bars: [Option<(Dock, egui::Rect)>; 16],
    /// A layout command picked in a panel menu (runs once the dock state is back).
    panel_command: Option<(&'static str, Option<String>)>,
}

impl egui_dock::TabViewer for TabHost<'_> {
    type Tab = Dock;
    fn title(&mut self, tab: &mut Dock) -> egui::WidgetText {
        // Room for the tab icon, painted over the leading spaces (panels.rs).
        self.editor.tab_label(*tab).into()
    }
    /// Stable per tab (the default uses the title, which changes with the counts).
    fn id(&mut self, tab: &mut Dock) -> egui::Id {
        egui::Id::new(("editor_tab", tab.name()))
    }
    fn ui(&mut self, ui: &mut Ui, tab: &mut Dock) {
        let bar = self
            .bars
            .iter()
            .flatten()
            .find(|(t, _)| t == tab)
            .map(|(_, r)| *r);
        if let Some(command) = self.editor.panel_menu(ui, *tab, bar) {
            self.panel_command = Some(command);
        }
        let (editor, actions) = (&mut *self.editor, &mut *self.actions);
        match *tab {
            Dock::Hierarchy => {
                ui.spacing_mut().item_spacing.y = 6.0;
                editor.hierarchy(ui, actions);
            }
            Dock::Inspector => {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 5.0);
                // The docked Inspector paints its own background, leaving the camera
                // preview hole (if any) transparent for the view drawn underneath.
                let background = (!self.inspector_floating)
                    .then(|| (ui.painter().add(egui::Shape::Noop), ui.clip_rect()));
                editor.inspector(ui, self.runtime, actions, self.inspector_floating);
                if let Some((index, body)) = background {
                    let hole = editor.camera_preview.rect.intersect(body);
                    let shapes = if hole.is_positive() {
                        [
                            egui::Rect::from_min_max(body.min, egui::pos2(body.max.x, hole.min.y)),
                            egui::Rect::from_min_max(egui::pos2(body.min.x, hole.max.y), body.max),
                            egui::Rect::from_min_max(
                                egui::pos2(body.min.x, hole.min.y),
                                egui::pos2(hole.min.x, hole.max.y),
                            ),
                            egui::Rect::from_min_max(
                                egui::pos2(hole.max.x, hole.min.y),
                                egui::pos2(body.max.x, hole.max.y),
                            ),
                        ]
                        .into_iter()
                        .filter(|r| r.is_positive())
                        .map(|r| egui::Shape::rect_filled(r, 0.0, SURFACE))
                        .collect()
                    } else {
                        vec![egui::Shape::rect_filled(body, 0.0, SURFACE)]
                    };
                    ui.painter().set(index, egui::Shape::Vec(shapes));
                }
            }
            Dock::Scene => {
                self.scene = true;
                editor.scene_tab(ui, self.runtime, actions);
            }
            Dock::Game => {
                self.game = true;
                editor.game_tab(ui, self.runtime, actions);
            }
            Dock::World => editor.world_tab(ui, actions),
            Dock::UiEditor => editor.ui_editor_tab(ui, actions),
            Dock::Graphs => editor.graph_tab(ui, self.runtime, actions),
            Dock::Assets => editor.asset_browser(ui, actions),
            Dock::Console => editor.console_tab(ui),
            Dock::Profiler => editor.profiler_tab(ui),
            Dock::Trace => editor.trace_tab(ui),
            Dock::InputMap => editor.input_map_tab(ui, actions),
            Dock::Localization => editor.localization_tab(ui, actions),
            Dock::Animation => editor.animation_tab(ui, self.runtime, actions),
            Dock::Vfx => editor.vfx_tab(ui, actions),
            other => editor.dock_tab(ui, other, actions),
        }
    }
    /// Scene and Game are composited by the renderer at the tab's rectangle on the main
    /// surface (a floating window would draw its frame over them).
    fn allowed_in_windows(&self, tab: &mut Dock) -> bool {
        // The UI Editor canvas is drawn in its own egui areas, which would sit under a window.
        !matches!(tab, Dock::Scene | Dock::Game | Dock::UiEditor)
    }
    /// Scene and Game bodies stay transparent so the world drawn underneath shows; the docked
    /// Inspector paints its own background around the camera preview.
    fn clear_background(&self, tab: &Dock) -> bool {
        match tab {
            Dock::Scene | Dock::Game => false,
            Dock::Inspector => self.inspector_floating,
            _ => true,
        }
    }
    /// Every panel scrolls its own content (the hierarchy is a virtual list).
    fn scroll_bars(&self, _tab: &Dock) -> [bool; 2] {
        [false, false]
    }
    fn tab_style_override(
        &self,
        tab: &Dock,
        global: &egui_dock::TabStyle,
    ) -> Option<egui_dock::TabStyle> {
        let alone = self.alone & tab_bit(*tab) != 0;
        let canvas = matches!(tab, Dock::Scene | Dock::Game);
        (alone || canvas).then(|| {
            let mut style = if alone {
                header_tab_style(global)
            } else {
                global.clone()
            };
            if canvas {
                style.tab_body.inner_margin = Margin::ZERO;
            }
            style
        })
    }
}

/// The dock look: the editor's panel colors on the egui-derived defaults.
fn dock_style(style: &egui::Style) -> egui_dock::Style {
    let mut dock = egui_dock::Style::from_egui(style);
    // Each leaf is a rounded, bordered panel; the strip on top is its header (pill tabs).
    dock.tab_bar.bg_fill = SURFACE;
    dock.tab_bar.height = 36.0;
    dock.tab_bar.hline_color = theme::BORDER;
    dock.tab_bar.corner_radius = egui::CornerRadius {
        nw: 8,
        ne: 8,
        sw: 0,
        se: 0,
    };
    dock.tab.tab_body.bg_fill = SURFACE;
    dock.tab.tab_body.inner_margin = Margin::same(10);
    dock.tab.tab_body.stroke = egui::Stroke::new(1.0_f32, theme::BORDER);
    dock.tab.tab_body.corner_radius = egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: 8,
        se: 8,
    };
    for (tab, fill, outline, text) in [
        (
            &mut dock.tab.active,
            theme::ACCENT_SOFT,
            theme::ACCENT_EDGE,
            Color32::WHITE,
        ),
        (
            &mut dock.tab.focused,
            theme::ACCENT_SOFT,
            theme::ACCENT_EDGE,
            Color32::WHITE,
        ),
        (
            &mut dock.tab.active_with_kb_focus,
            theme::ACCENT_SOFT,
            ACCENT,
            Color32::WHITE,
        ),
        (
            &mut dock.tab.focused_with_kb_focus,
            theme::ACCENT_SOFT,
            ACCENT,
            Color32::WHITE,
        ),
        (&mut dock.tab.inactive, SURFACE, SURFACE, MUTED),
        (
            &mut dock.tab.inactive_with_kb_focus,
            SURFACE,
            theme::BORDER,
            MUTED,
        ),
        (
            &mut dock.tab.hovered,
            theme::CONTROL,
            theme::CONTROL,
            theme::TEXT,
        ),
    ] {
        tab.bg_fill = fill;
        tab.text_color = text;
        tab.outline_color = outline;
        tab.corner_radius = egui::CornerRadius::same(7);
    }
    // Gaps between panels show the window backdrop.
    dock.separator.width = 6.0;
    dock.separator.color_idle = BACKDROP;
    dock.separator.color_hovered = theme::BORDER;
    dock.separator.color_dragged = ACCENT;
    dock.main_surface_border_stroke = egui::Stroke::NONE;
    dock
}

fn tab_bit(tab: Dock) -> u32 {
    1 << Dock::ALL.iter().position(|d| *d == tab).unwrap_or(31)
}

/// The tab of a single-tab leaf is drawn as a plain panel title (no pill).
fn header_tab_style(global: &egui_dock::TabStyle) -> egui_dock::TabStyle {
    let mut style = global.clone();
    for tab in [
        &mut style.active,
        &mut style.focused,
        &mut style.active_with_kb_focus,
        &mut style.focused_with_kb_focus,
        &mut style.hovered,
        &mut style.inactive,
    ] {
        tab.bg_fill = SURFACE;
        tab.outline_color = SURFACE;
        tab.text_color = theme::TEXT;
    }
    style
}

/// Hierarchy row height (also the virtualization step of `show_rows`).
const ROW_HEIGHT: f32 = 26.0;

/// Where a dragged hierarchy row lands relative to the row under the pointer.
#[derive(Clone, Copy)]
enum Drop {
    Before,
    Into,
    After,
}

fn has_component(world: &World, e: &Entity, component: &str) -> bool {
    e.components.contains_key(component)
        || e.prefab
            .as_ref()
            .and_then(|p| world.prefabs.get(p))
            .is_some_and(|p| p.components.contains_key(component))
}

/// Small type badges for a hierarchy row. Plain ASCII: the bundled fonts lack most symbols.
pub(super) fn badges(world: &World, e: &Entity) -> Vec<(&'static str, Color32)> {
    let mut out = vec![];
    for (component, text, color) in [
        ("render.camera", "CAM", Color32::from_rgb(120, 180, 255)),
        ("render.light", "LIGHT", Color32::from_rgb(255, 222, 120)),
        ("render.mesh", "MESH", Color32::from_rgb(105, 205, 190)),
        ("render.geometry", "GEO", Color32::from_rgb(140, 210, 120)),
        ("animation.player", "SKIN", Color32::from_rgb(200, 150, 255)),
        ("fsm.agent", "FSM", Color32::from_rgb(255, 170, 100)),
        ("trigger.zone", "TRIG", Color32::from_rgb(120, 230, 210)),
        ("path.spline", "PATH", Color32::from_rgb(255, 176, 72)),
        ("path.follow", "FOLLOW", Color32::from_rgb(255, 214, 150)),
        ("nav.agent", "NAV", Color32::from_rgb(110, 200, 255)),
        ("interact.target", "USE", Color32::from_rgb(255, 205, 90)),
        ("tween.clips", "TWEEN", Color32::from_rgb(240, 150, 200)),
        ("audio.source", "SND", Color32::from_rgb(150, 230, 140)),
        ("audio.listener", "EAR", Color32::from_rgb(190, 170, 255)),
        ("camera.follow", "RIG", Color32::from_rgb(255, 214, 92)),
    ] {
        if has_component(world, e, component) {
            out.push((text, color));
        }
    }
    if e.prefab.is_some() {
        out.push(("PFB", Color32::from_rgb(235, 205, 110)));
    }
    out
}

fn badge_meaning(text: &str) -> &'static str {
    match text {
        "CAM" => "render.camera",
        "LIGHT" => "render.light (directional, point, spot or area)",
        "MESH" => "render.mesh (primitive parts)",
        "GEO" => "render.geometry (procedural)",
        "SKIN" => "animation.player (glTF skinned model)",
        "FSM" => "fsm.agent (state machine behaviour)",
        "TRIG" => "trigger.zone (enter/exit events)",
        "PATH" => "path.spline (route for path followers)",
        "FOLLOW" => "path.follow (moves along a path)",
        "NAV" => "nav.agent (walks to targets around obstacles)",
        "USE" => "interact.target (talk / open with a button near it)",
        "TWEEN" => "tween.clips (keyframed property animation)",
        "SND" => "audio.source (positional sound)",
        "EAR" => "audio.listener (where sounds are heard from)",
        "RIG" => "camera.follow (follows a target, stops in front of walls)",
        _ => "prefab instance",
    }
}

/// Eye or padlock toggle of a Hierarchy row. `own` = flag set on this row, `inherited` = set
/// on an ancestor. Unflagged toggles are drawn only on hovered / selected rows (`show`) but
/// always take their place, so a click lands the same way.
fn view_toggle(
    ui: &mut Ui,
    icons: super::icons::Icons,
    own: bool,
    inherited: bool,
    lock: bool,
    show: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 16.0), egui::Sense::click());
    let on = own || inherited;
    let color = if own {
        WARNING
    } else if inherited {
        WARNING.gamma_multiply(0.5)
    } else if response.hovered() {
        theme::TEXT
    } else if show {
        MUTED
    } else {
        return response;
    };
    let icon = match (lock, on) {
        (true, true) => Icon::Lock,
        (true, false) => Icon::Unlock,
        (false, true) => Icon::EyeOff,
        (false, false) => Icon::Eye,
    };
    icons.paint(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), egui::vec2(15.0, 15.0)),
        icon,
        color,
    );
    response
}

/// The type icon of a Hierarchy row (most telling component first) and its tooltip.
fn entity_icon(world: &World, e: &Entity, branch: bool) -> (Icon, &'static str) {
    let prefab = e.prefab.as_ref().and_then(|p| world.prefabs.get(p));
    let has = |c: &str| {
        e.components.contains_key(c) || prefab.is_some_and(|p| p.components.contains_key(c))
    };
    let light_kind = || {
        [
            e.components.get("render.light"),
            prefab.and_then(|p| p.components.get("render.light")),
        ]
        .into_iter()
        .flatten()
        .find_map(|v| v.get("kind")?.as_str())
    };
    if has("camera.follow") {
        (Icon::CameraFollow, "Follow camera")
    } else if has("render.camera") {
        (Icon::Camera, "Camera")
    } else if has("render.light") {
        match light_kind() {
            Some("directional") => (Icon::LightDirectional, "Directional light"),
            Some("spot") => (Icon::LightSpot, "Spot light"),
            Some("area") => (Icon::LightArea, "Area light"),
            _ => (Icon::LightPoint, "Point light"),
        }
    } else if has("trigger.zone") {
        (Icon::Trigger, "Trigger zone")
    } else if has("path.spline") {
        (Icon::Path, "Path")
    } else if has("audio.source") {
        (Icon::Audio, "Audio source")
    } else if has("audio.listener") {
        (Icon::AudioListener, "Audio listener")
    } else if has("animation.player") {
        (Icon::Skeleton, "Animated model")
    } else if has("render.mesh") || has("render.geometry") {
        (Icon::Mesh, "Mesh")
    } else if has("nav.agent") {
        (Icon::Nav, "Navigation agent")
    } else if has("fsm.agent") {
        (Icon::Fsm, "State machine agent")
    } else if has("interact.target") {
        (Icon::Interact, "Interactable")
    } else if has("tween.clips") {
        (Icon::Tween, "Tweened object")
    } else if e.prefab.is_some() {
        (Icon::Prefab, "Prefab instance")
    } else if branch {
        (Icon::Folder, "Group (parent of other objects)")
    } else {
        (Icon::Entity, "Object")
    }
}

/// Hierarchy search: whitespace-separated tokens that must all match. `tag:x` (exact tag),
/// `has:component` (authored or prefab-inherited), `prefab:id`; any other token matches a
/// substring of the ID or label. `term` is already lowercase.
pub(crate) fn search_matches(world: &World, e: &Entity, term: &str) -> bool {
    term.split_whitespace().all(|token| {
        if let Some(tag) = token.strip_prefix("tag:") {
            e.tags.iter().any(|t| t.to_lowercase() == tag)
        } else if let Some(c) = token.strip_prefix("has:") {
            e.components.keys().any(|k| k.to_lowercase() == c)
                || e.prefab
                    .as_ref()
                    .and_then(|p| world.prefabs.get(p))
                    .is_some_and(|p| p.components.keys().any(|k| k.to_lowercase() == c))
        } else if let Some(p) = token.strip_prefix("prefab:") {
            e.prefab.as_ref().is_some_and(|x| x.to_lowercase() == p)
        } else {
            e.id.to_lowercase().contains(token)
                || e.label
                    .as_ref()
                    .is_some_and(|n| n.to_lowercase().contains(token))
        }
    })
}

/// What the component section asks the Inspector to do after drawing.
pub(super) enum ComponentRequest {
    Remove(String),
    Reset(String),
    Copy(String),
    Paste(String),
    Array {
        component: String,
        pointer: String,
        edit: axiom_core::ops::ArrayEdit,
    },
    MainCamera,
    /// render.mesh "Blocks follow cameras" checkbox (`camera SEL blocker on|off`).
    CameraBlocker(bool),
    Play(String),
    EditGraph,
}

/// Everything the component section reads besides the object and its draft.
struct ComponentPanel<'a> {
    world: &'a World,
    world_version: u64,
    hints: &'a std::collections::BTreeMap<String, Value>,
    registry: &'a axiom_core::Registry,
    /// Typed form spec of the open component (cached per type).
    spec: Option<&'a super::form::Spec>,
    mixed: &'a std::collections::HashSet<String>,
    filter: &'a mut String,
    picker: &'a mut Option<super::form::Picker>,
    rects: &'a mut std::collections::HashMap<String, egui::Rect>,
    /// Component whose values were copied (Paste is offered on the same type).
    clipboard: Option<&'a str>,
    live: bool,
    /// Scroll the Inspector so the open component starts at the top (once).
    scroll: &'a mut bool,
    icons: super::icons::Icons,
}

/// Icon of a component card (by the component's family).
fn component_icon(key: &str) -> Icon {
    let family = key.split('.').next().unwrap_or(key);
    match key {
        "render.camera" => Icon::Camera,
        "render.light" => Icon::LightPoint,
        "render.material" => Icon::Material,
        "render.mesh" | "render.geometry" => Icon::Mesh,
        "audio.listener" => Icon::AudioListener,
        "camera.follow" => Icon::CameraFollow,
        "fsm.agent" => Icon::Fsm,
        _ => match family {
            "animation" => Icon::Animation,
            "audio" => Icon::Audio,
            "trigger" => Icon::Trigger,
            "path" => Icon::Path,
            "nav" => Icon::Nav,
            "interact" => Icon::Interact,
            "tween" => Icon::Tween,
            "physics" | "combat" => Icon::Collider,
            "vfx" | "postfx" => Icon::Vfx,
            "ui" => Icon::UiDocument,
            "input" => Icon::Input,
            "behavior" | "fsm" => Icon::Behavior,
            "render" => Icon::Mesh,
            _ => Icon::Entity,
        },
    }
}

/// Component cards: one row per component (authored or prefab-inherited); the open one shows
/// its form. Header: fold, enabled, icon, title, then reset / play / edit graph and a "…" menu
/// (copy, paste, remove, make main camera). Add Component / Behavior pickers on top.
fn component_editor(
    ui: &mut Ui,
    panel: ComponentPanel,
    entity: &Entity,
    draft: &mut Draft,
    add_component: &mut Option<String>,
) -> Option<ComponentRequest> {
    let ComponentPanel {
        world,
        world_version,
        hints,
        registry,
        spec,
        mixed,
        filter,
        picker,
        rects,
        clipboard,
        live,
        scroll,
        icons,
    } = panel;
    let inherited = entity.prefab.as_ref().and_then(|p| world.prefabs.get(p));
    let keys: std::collections::BTreeSet<_> = entity
        .components
        .keys()
        .chain(inherited.into_iter().flat_map(|p| p.components.keys()))
        .filter(|key| key.as_str() != "core.transform")
        .cloned()
        .collect();
    let mut request = None;
    // Picking in Add component writes at once, so any unapplied draft edit blocks it.
    ui.add_enabled_ui(!draft.dirty(), |ui| {
        ui.horizontal(|ui| {
            icons.show(ui, Icon::Layers, 16.0, ACCENT);
            ui.label(RichText::new("Components").strong().color(theme::TEXT));
        });
        ui.horizontal_wrapped(|ui| {
            // Picker over every installed component the object lacks. Picking one writes it with
            // its default and whatever capability/systems it needs (ops::add_component).
            egui::ComboBox::from_id_salt("add_component")
                .selected_text("Add Component…")
                .width(150.0)
                .show_ui(ui, |ui| {
                    let mut available: Vec<&String> = registry
                        .components
                        .keys()
                        .chain(world.component_schemas.keys())
                        .filter(|id| !keys.contains(*id) && id.as_str() != "core.transform")
                        .collect();
                    available.sort();
                    available.dedup();
                    for id in available {
                        let needs = axiom_core::ops::requirements(registry, id);
                        let missing: Vec<&str> = needs
                            .capabilities
                            .iter()
                            .filter(|c| !world.capabilities.contains_key(*c))
                            .chain(needs.systems.iter().filter(|s| !world.systems.contains(s)))
                            .map(String::as_str)
                            .collect();
                        let text = if missing.is_empty() {
                            RichText::new(id.as_str())
                        } else {
                            RichText::new(format!("{id}   + {}", missing.join(", "))).color(MUTED)
                        };
                        let response = ui.selectable_label(false, text);
                        let response = if missing.is_empty() {
                            response
                        } else {
                            response.on_hover_text(
                                "Adds what it needs to the world in the same revision",
                            )
                        };
                        if response.clicked() {
                            *add_component = Some(id.clone());
                        }
                    }
                });
            // Behavior ▸ templates: option dialogs (`command:ID` runs the registered command).
            let menu = egui::ComboBox::from_id_salt("add_behavior")
                .selected_text("Behavior…")
                .width(110.0)
                .show_ui(ui, |ui| {
                    for (id, title) in [
                        ("behavior.fsm_agent", "FSM Agent…"),
                        ("behavior.patrol", "Patrol…"),
                        ("behavior.guard", "Guard…"),
                        ("behavior.dialogue", "NPC Dialogue…"),
                    ] {
                        let r = ui.selectable_label(false, title);
                        rects.insert(format!("behavior:{id}"), r.rect);
                        if r.clicked() {
                            *add_component = Some(format!("command:{id}"));
                        }
                    }
                });
            rects.insert("behavior:menu".into(), menu.response.rect);
        });
    });
    // A component loaded by ID that the object neither authors nor inherits gets a card too
    // (stage it with JSON).
    let staged = (!draft.component.is_empty()
        && draft.component != "core.transform"
        && !keys.contains(&draft.component))
    .then(|| draft.component.clone());
    for key in keys.iter().chain(staged.iter()) {
        let open = draft.component == *key;
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let fold = theme::icon_button_sized(
                    ui,
                    icons,
                    if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    if open { "Close" } else { "Open" },
                    false,
                    18.0,
                );
                rects.insert(format!("component:fold:{key}"), fold.rect);
                // Enabled: a live form edit on the open card; read-only on the others.
                let mut shown_enabled = false;
                if open {
                    let base_enabled = draft.form_base.get("enabled").cloned();
                    if let Some(Value::Bool(enabled)) =
                        draft.form.as_mut().and_then(|f| f.get_mut("enabled"))
                    {
                        let shown_mixed = mixed.contains("/enabled")
                            && base_enabled == Some(Value::Bool(*enabled));
                        let r = theme::accent_check(ui, icons, enabled, shown_mixed)
                            .on_hover_text("Enabled");
                        rects.insert("component:enabled".into(), r.rect);
                        if r.changed() {
                            draft.form_dirty = true;
                        }
                        shown_enabled = true;
                    }
                }
                if !shown_enabled {
                    let enabled = entity
                        .components
                        .get(key)
                        .and_then(|v| v.get("enabled"))
                        .or_else(|| inherited?.components.get(key)?.get("enabled"))
                        .and_then(Value::as_bool);
                    if let Some(mut enabled) = enabled {
                        ui.add_enabled_ui(false, |ui| {
                            theme::accent_check(ui, icons, &mut enabled, false)
                                .on_disabled_hover_text("Open the component to toggle it")
                        });
                    }
                }
                icons.show(
                    ui,
                    component_icon(key),
                    16.0,
                    if open { ACCENT } else { MUTED },
                );
                let title = ui
                    .add_enabled(
                        !draft.dirty() || open,
                        egui::Button::new(RichText::new(key.as_str()).color(theme::TEXT))
                            .frame(false),
                    )
                    .on_hover_text(if open {
                        "Open component"
                    } else {
                        "Open component (edit its fields)"
                    });
                rects.insert(format!("component:{key}"), title.rect);
                if (title.clicked() || fold.clicked()) && !open && !draft.dirty() {
                    draft.open_component(world, entity, key);
                } else if fold.clicked() && open && !draft.dirty() {
                    draft.open_component(world, entity, "");
                }
                if open {
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(r) = component_actions(
                            ui, icons, draft, rects, spec, clipboard, key, entity, inherited,
                        ) {
                            request = Some(r);
                        }
                    });
                }
            });
            if open {
                if std::mem::take(scroll) {
                    ui.scroll_to_cursor(Some(egui::Align::TOP));
                }
                let body = ComponentBody {
                    world,
                    world_version,
                    hints,
                    mixed,
                    filter: &mut *filter,
                    picker: &mut *picker,
                    rects: &mut *rects,
                    live,
                    spec,
                };
                if let Some(r) = component_body(ui, body, entity, inherited, draft) {
                    request = Some(r);
                }
            }
        });
        ui.add_space(6.0);
    }
    egui::CollapsingHeader::new(RichText::new("Advanced").color(MUTED))
        .id_salt("components_advanced")
        .show(ui, |ui| {
            ui.add_enabled_ui(!draft.dirty(), |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.component_input)
                            .hint_text("Component ID")
                            .desired_width(150.0),
                    );
                    if ui
                        .small_button("Load")
                        .on_hover_text(
                            "Open any component by ID (one the object lacks can be staged as JSON)",
                        )
                        .clicked()
                    {
                        let id = draft.component_input.trim().to_owned();
                        draft.open_component(world, entity, &id);
                    }
                });
            });
        });
    request
}

/// Header actions of the open component card, right to left: "…" menu (Copy, Paste, Remove,
/// Make Main Camera), Reset, then Play / Edit Graph where they apply.
#[allow(clippy::too_many_arguments)]
fn component_actions(
    ui: &mut Ui,
    icons: super::icons::Icons,
    draft: &Draft,
    rects: &mut std::collections::HashMap<String, egui::Rect>,
    spec: Option<&super::form::Spec>,
    clipboard: Option<&str>,
    key: &str,
    entity: &Entity,
    inherited: Option<&axiom_core::Prefab>,
) -> Option<ComponentRequest> {
    let own = entity.components.contains_key(key);
    let inherits = inherited.is_some_and(|p| p.components.contains_key(key));
    if !(own || inherits) {
        return None;
    }
    let mut request = None;
    let component = key.to_owned();
    let resettable = own && (inherits || spec.is_some_and(|s| s.resettable));
    let playable = matches!(component.as_str(), "tween.clips" | "audio.source");
    ui.add_enabled_ui(!draft.dirty(), |ui| {
        let more = theme::icon_button_sized(ui, icons, Icon::More, "More actions", false, 22.0);
        rects.insert("component:menu".into(), more.rect);
        let id = ui.make_persistent_id(("component_menu", key));
        theme::dropdown(ui, id, &more, |ui| {
            let mut item =
                |ui: &mut Ui, name: &str, icon: Icon, text: &str, tip: &str, enabled: bool| {
                    let image = icons.image(ui.ctx(), icon, 16.0, MUTED);
                    let r = ui
                        .add_enabled(
                            enabled,
                            egui::Button::image_and_text(image, text).frame(false),
                        )
                        .on_hover_text(tip);
                    rects.insert(format!("component:{name}"), r.rect);
                    let clicked = r.clicked();
                    if clicked {
                        theme::close_dropdown(ui, id);
                    }
                    clicked
                };
            if item(
                ui,
                "menu:reset",
                Icon::Reset,
                "Reset",
                "Back to the default values, or the prefab's (ACL: reset SEL COMPONENT)",
                resettable,
            ) {
                request = Some(ComponentRequest::Reset(component.clone()));
            }
            if playable
                && item(
                    ui,
                    "menu:play",
                    Icon::Play,
                    "Play",
                    "Start Play preview and play this clip / source",
                    true,
                )
            {
                request = Some(ComponentRequest::Play(component.clone()));
            }
            if item(
                ui,
                "copy",
                Icon::Copy,
                "Copy values",
                "Copy the values (Paste on the same component of other objects)",
                true,
            ) {
                request = Some(ComponentRequest::Copy(component.clone()));
            }
            if item(
                ui,
                "paste",
                Icon::Paste,
                "Paste values",
                "Paste the copied values on every selected object with this component",
                clipboard == Some(component.as_str()),
            ) {
                request = Some(ComponentRequest::Paste(component.clone()));
            }
            if component == "render.camera"
                && item(
                    ui,
                    "main_camera",
                    Icon::Camera,
                    "Make Main Camera",
                    "active=true here, false on every other camera (ACL: camera main ID)",
                    draft.form.as_ref().is_some_and(|f| f["active"] != true),
                )
            {
                request = Some(ComponentRequest::MainCamera);
            }
            ui.separator();
            if item(
                ui,
                "remove",
                Icon::Delete,
                "Remove component",
                "Remove the authored component (ACL: remove SEL COMPONENT)",
                own,
            ) {
                request = Some(ComponentRequest::Remove(component.clone()));
            }
        });
        let tip = if inherits {
            "Reset: drop the override, back to the prefab's values"
        } else {
            "Reset to the default values (ACL: reset SEL COMPONENT)"
        };
        let r = ui
            .add_enabled_ui(resettable, |ui| {
                theme::icon_button_sized(ui, icons, Icon::Reset, tip, false, 22.0)
            })
            .inner;
        rects.insert("component:reset".into(), r.rect);
        if r.clicked() {
            request = Some(ComponentRequest::Reset(component.clone()));
        }
        if component == "fsm.agent" {
            let enabled = draft
                .form
                .as_ref()
                .is_some_and(|f| f["machine"].is_string());
            let r = ui
                .add_enabled_ui(enabled, |ui| {
                    theme::icon_button_sized(
                        ui,
                        icons,
                        Icon::Graphs,
                        "Edit Graph: open this agent's state machine in the Graphs tab",
                        false,
                        22.0,
                    )
                })
                .inner;
            rects.insert("component:edit_graph".into(), r.rect);
            if r.clicked() {
                request = Some(ComponentRequest::EditGraph);
            }
        }
        if playable {
            let r = theme::icon_button_sized(
                ui,
                icons,
                Icon::Play,
                "Play: start Play preview and play this clip / source",
                false,
                22.0,
            );
            rects.insert("component:play".into(), r.rect);
            if r.clicked() {
                request = Some(ComponentRequest::Play(component.clone()));
            }
        }
    });
    request
}

/// What the open card's body reads.
struct ComponentBody<'a> {
    world: &'a World,
    world_version: u64,
    hints: &'a std::collections::BTreeMap<String, Value>,
    mixed: &'a std::collections::HashSet<String>,
    filter: &'a mut String,
    picker: &'a mut Option<super::form::Picker>,
    rects: &'a mut std::collections::HashMap<String, egui::Rect>,
    live: bool,
    spec: Option<&'a super::form::Spec>,
}

/// The open card's body: typed form (with Form / JSON switch and field filter) or JSON.
fn component_body(
    ui: &mut Ui,
    body: ComponentBody,
    entity: &Entity,
    inherited: Option<&axiom_core::Prefab>,
    draft: &mut Draft,
) -> Option<ComponentRequest> {
    let ComponentBody {
        world,
        world_version,
        hints,
        mixed,
        filter,
        picker,
        rects,
        live,
        spec,
    } = body;
    let mut request = None;
    let own = entity.components.contains_key(&draft.component);
    let inherits = inherited.is_some_and(|p| p.components.contains_key(&draft.component));
    let known = hints.contains_key(&draft.component)
        || world.component_schemas.contains_key(&draft.component);
    if !known && !own && !inherits {
        ui.label(
            RichText::new(format!(
                "Unknown component '{}'. Check the ID (schema components lists them).",
                draft.component
            ))
            .color(WARNING),
        );
    }
    if draft.component == "render.mesh" {
        use axiom_core::camera::{BLOCKER_COMPONENT, BLOCKER_TAG};
        let mut blocks = entity.components.contains_key(BLOCKER_COMPONENT)
            || inherited.is_some_and(|p| p.components.contains_key(BLOCKER_COMPONENT))
            || entity.tags.iter().any(|t| t == BLOCKER_TAG);
        let r = ui
            .add_enabled_ui(!draft.dirty(), |ui| {
                ui.checkbox(&mut blocks, "Blocks follow cameras").on_hover_text(
                    "camera.blocker: every mesh part stops follow cameras (ACL: camera SEL blocker on|off)",
                )
            })
            .inner;
        rects.insert("form:camera_blocker".into(), r.rect);
        if r.changed() {
            request = Some(ComponentRequest::CameraBlocker(blocks));
        }
    }
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!draft.dirty(), |ui| {
            if ui
                .selectable_label(!draft.json_mode, "Form")
                .on_hover_text(
                    "Typed fields of the effective value; only changed fields are written",
                )
                .clicked()
                && draft.form.is_some()
            {
                draft.json_mode = false;
            }
            if ui
                .selectable_label(draft.json_mode, "JSON")
                .on_hover_text("Edit the authored override as raw JSON (Apply writes it)")
                .clicked()
            {
                draft.json_mode = true;
            }
        });
        if !draft.json_mode {
            let r = ui.add(
                egui::TextEdit::singleline(filter)
                    .hint_text("Filter fields")
                    .desired_width(110.0),
            );
            rects.insert("form:filter".into(), r.rect);
        }
    });
    if !draft.json_mode {
        if let (Some(form), Some(spec)) = (&mut draft.form, spec) {
            let needle = filter.trim().to_lowercase();
            let mut ctx = super::form::Ctx {
                world,
                world_version,
                base: &draft.form_base,
                mixed,
                filter: &needle,
                structural: !draft.form_dirty && own,
                picker,
                rects: Some(rects),
            };
            let out = super::form::show(ui, &mut ctx, spec, form, &draft.component);
            draft.form_dirty |= out.changed;
            if let Some((pointer, edit)) = out.array {
                request = Some(ComponentRequest::Array {
                    component: draft.component.clone(),
                    pointer,
                    edit,
                });
            }
            if !own && inherits {
                caption(ui, "Inherited from the prefab: add / remove items on the prefab (Assets) or override the component first");
            }
            if draft.form_dirty && !live {
                let mut leaves = vec![];
                super::form::changes(&draft.form_base, form, "", &mut leaves);
                caption(
                    ui,
                    format!(
                        "{} changed field(s): {}",
                        leaves.len(),
                        leaves
                            .iter()
                            .map(|(p, _)| p.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
        }
        return request;
    }
    caption(
        ui,
        match (own, inherits) {
            (true, true) => "Prefab override JSON",
            (false, true) => "Inherited · enter an override",
            (true, false) => "Authored JSON",
            (false, false) => "New component JSON",
        },
    );
    if !own && !inherits {
        // Show the registry/world schema so a new component can be typed without docs.
        if let Some(schema) = super::form::hints_for(hints, world, &draft.component) {
            ui.label(
                RichText::new(format!("schema {}", schema))
                    .small()
                    .monospace()
                    .color(MUTED),
            );
        }
    }
    draft.component_dirty |= ui
        .add(
            egui::TextEdit::multiline(&mut draft.json)
                .code_editor()
                .desired_rows(5)
                .desired_width(f32::INFINITY),
        )
        .changed();
    ui.horizontal(|ui| {
        if !own && ui.small_button("Stage component").clicked() {
            draft.component_dirty = true;
        }
        if ui
            .add_enabled(own && !draft.dirty(), egui::Button::new("Remove component"))
            .clicked()
        {
            request = Some(ComponentRequest::Remove(draft.component.clone()));
        }
    });
    request
}

/// One Transform row: label and X / Y / Z drag fields. The fields share the visible width
/// (never wider than the panel); when that leaves them under 44 pt each, the label goes on
/// its own line. Returns whether a value changed and the Z field's rectangle.
fn vector(
    ui: &mut Ui,
    name: &str,
    value: &mut [f32; 3],
    speed: f32,
    range: std::ops::RangeInclusive<f32>,
    mixed: [bool; 3],
) -> (bool, egui::Rect) {
    const LABEL: f32 = 62.0;
    const GAP: f32 = 4.0;
    // A wide row elsewhere in the card can make `available_width` exceed what is visible.
    let visible = ui
        .available_width()
        .min(ui.clip_rect().max.x - ui.cursor().min.x - 2.0)
        .max(1.0);
    let stacked = (visible - LABEL - 3.0 * GAP) / 3.0 < 44.0;
    let mut changed = false;
    let mut z = egui::Rect::NOTHING;
    if stacked {
        ui.label(RichText::new(name).color(MUTED));
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let room = if stacked {
            visible
        } else {
            ui.add_sized(
                [LABEL, 20.0],
                egui::Label::new(RichText::new(name).color(MUTED)),
            );
            visible - LABEL - GAP
        };
        let width = ((room - 2.0 * GAP) / 3.0).clamp(24.0, 96.0);
        // `add_sized` cannot make a widget narrower than its own minimum.
        ui.spacing_mut().interact_size.x = width;
        ui.spacing_mut().button_padding.x = 3.0;
        for ((v, axis), mixed) in value.iter_mut().zip(["X ", "Y ", "Z "]).zip(mixed) {
            let mut drag = egui::DragValue::new(v)
                .prefix(axis)
                .speed(speed)
                .range(range.clone())
                .max_decimals(3);
            if mixed {
                drag = drag.custom_formatter(|_, _| super::form::MIXED.to_owned());
            }
            let r = ui.add_sized([width, 20.0], drag);
            changed |= r.changed();
            z = r.rect;
        }
    });
    (changed, z)
}

pub(super) fn truncate(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut display: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        display.push('…');
    }
    display
}
