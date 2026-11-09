use super::{viewport, Action, Dock, Draft, Editor, Tool};
use crate::world::WorldRuntime;
use axiom_core::{
    merge_json, scene_edit::Hierarchy, CoreEdit, Edit, Entity, Request, Transform, World,
};
use egui::{Color32, Frame, Layout, Margin, RichText, ScrollArea, Ui};
use serde_json::{json, Value};

const SURFACE: Color32 = Color32::from_rgb(23, 28, 37);
const HEADER: Color32 = Color32::from_rgb(18, 22, 30);
const CARD: Color32 = Color32::from_rgb(32, 40, 52);
const MUTED: Color32 = Color32::from_rgb(155, 174, 194);
const ACCENT: Color32 = Color32::from_rgb(105, 222, 195);
const WARNING: Color32 = Color32::from_rgb(255, 178, 111);

fn panel(color: Color32, padding: i8) -> Frame {
    Frame::new().fill(color).inner_margin(Margin::same(padding))
}
fn heading(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).strong().size(13.0));
}
fn caption(ui: &mut Ui, text: impl Into<String>) {
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
        // An inline rename owns the keyboard (egui drops focus on Escape before we get here).
        if !text_focused && self.renaming.is_none() && !scene_owns_keys {
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
        egui::TopBottomPanel::top("editor_menu")
            .frame(panel(HEADER, 4))
            .show(ctx, |ui| self.menu_bar(ui, &mut actions));
        egui::TopBottomPanel::top("editor_toolbar")
            .frame(panel(HEADER, 9))
            .show(ctx, |ui| {
                if self.logo.is_none() {
                    self.logo = crate::branding::egui_icon().map(|image| {
                        ctx.load_texture("axiom-logo", image, egui::TextureOptions::LINEAR)
                    });
                }
                let logo = self.logo.clone();
                ui.horizontal(|ui| {
                    if let Some(logo) = &logo {
                        ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(24.0, 24.0)));
                    }
                    ui.label(
                        RichText::new("AXIOM")
                            .strong()
                            .size(17.0)
                            .color(egui::Color32::from_rgb(240, 72, 72)),
                    );
                    ui.label(RichText::new("EDITOR").monospace().size(10.0).color(MUTED));
                    ui.separator();
                    ui.label(RichText::new(truncate(&self.world.title, 32)).strong())
                        .on_hover_text(&self.world.title);
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.dirty(),
                                egui::Button::new(RichText::new("▶  Play  F5").color(ACCENT)),
                            )
                            .clicked()
                        {
                            actions.push(Action::Play);
                        }
                        ui.label(
                            RichText::new(format!("REV  {:04}", self.rev))
                                .monospace()
                                .color(ACCENT),
                        );
                        ui.separator();
                        for (title, redo) in [("Redo", true), ("Undo", false)] {
                            let action = self.restore_action(redo);
                            if ui
                                .add_enabled(
                                    !self.dirty() && action.is_some(),
                                    egui::Button::new(title),
                                )
                                .clicked()
                            {
                                actions.extend(action);
                            }
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("editor_status")
            .frame(panel(HEADER, 4))
            .show(ctx, |ui| self.status_bar(ui));
        let dock_max = (ctx.screen_rect().height() * 0.7).max(160.0);
        egui::TopBottomPanel::bottom("editor_dock")
            .resizable(true)
            .default_height(200.0)
            .height_range(72.0..=dock_max)
            .frame(panel(HEADER, 8))
            .show(ctx, |ui| {
                self.dock(ui, &mut actions);
            });
        egui::SidePanel::left("editor_hierarchy")
            .default_width(224.0)
            .width_range(178.0..=345.0)
            .frame(panel(SURFACE, 9))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                self.hierarchy(ui, &mut actions);
            });
        // Play preview: the timeline playhead follows the runtime.
        self.tween_ui.live = if self.previewing {
            self.selected
                .as_deref()
                .and_then(|id| runtime.system::<crate::tween::TweenSystem>()?.playback(id))
                .map(|(clip, time, _)| (clip.to_owned(), time))
        } else {
            None
        };
        egui::SidePanel::right("editor_inspector")
            .default_width(302.0)
            .width_range(270.0..=440.0)
            .frame(panel(SURFACE, 9))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 5.0);
                self.inspector(ui, &mut actions);
            });
        egui::CentralPanel::default()
            .frame(Frame::NONE)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                panel(CARD, 6).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    // Wraps instead of widening the central panel (which would push the viewport
                    // under the inspector).
                    ui.horizontal_wrapped(|ui| {
                        heading(ui, "SCENE");
                        ui.separator();
                        for (key, tool) in [
                            ("Q", Tool::Select),
                            ("W", Tool::Move),
                            ("E", Tool::Rotate),
                            ("R", Tool::Scale),
                        ] {
                            if ui
                                .selectable_label(self.tool == tool, tool.name())
                                .on_hover_text(key)
                                .clicked()
                                && self.drag.is_none()
                            {
                                self.tool = tool;
                            }
                        }
                        if ui.small_button("Frame").on_hover_text("F").clicked() {
                            self.frame_selection = true;
                        }
                        ui.separator();
                        ui.toggle_value(&mut self.prefs.snap, "Snap").on_hover_text(format!(
                            "{} m · {}° · {} scale. Hold Ctrl while dragging to invert.",
                            self.prefs.snap_move, self.prefs.snap_rotate_degrees, self.prefs.snap_scale
                        ));
                        ui.menu_button("…", |ui| {
                            ui.label(RichText::new("Snap increments").small());
                            ui.add(egui::DragValue::new(&mut self.prefs.snap_move).speed(0.01).range(0.001..=100.0).prefix("Move ").suffix(" m"));
                            ui.add(egui::DragValue::new(&mut self.prefs.snap_rotate_degrees).speed(0.5).range(0.1..=180.0).prefix("Rotate ").suffix("°"));
                            ui.add(egui::DragValue::new(&mut self.prefs.snap_scale).speed(0.01).range(0.001..=10.0).prefix("Scale "));
                        })
                        .response
                        .on_hover_text("Snap increments");
                        let space = if self.prefs.local_space { "Local" } else { "Global" };
                        if ui
                            .button(space)
                            .on_hover_text("Move/Rotate gizmo axes: world (Global) or the object's own (Local). Scale is always local.")
                            .clicked()
                            && self.drag.is_none()
                        {
                            self.prefs.local_space = !self.prefs.local_space;
                        }
                        ui.toggle_value(&mut self.prefs.grid, "Grid")
                            .on_hover_text("Always draw the ground grid");
                        ui.toggle_value(&mut self.prefs.gizmos, "Gizmos").on_hover_text(
                            "Camera frusta, nav.blocker, colliders and FSM agents (editor view only)",
                        );
                        ui.separator();
                        let mode = self.orbit.mode;
                        ui.menu_button(format!("View: {}", mode.name()), |ui| {
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
                                    self.run_into(m.command(), &mut actions);
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                            for n in 1..=4 {
                                ui.horizontal(|ui| {
                                    let save = format!("view.bookmark.save.{n}");
                                    let recall = format!("view.bookmark.recall.{n}");
                                    if ui.button(format!("Save {n}")).on_hover_text(self.shortcut_text(&save)).clicked() {
                                        self.run_into(&save, &mut actions);
                                        ui.close_menu();
                                    }
                                    let blocked = self.command_blocked(&recall);
                                    if ui
                                        .add_enabled(blocked.is_none(), egui::Button::new(format!("Recall {n}")))
                                        .on_hover_text(self.shortcut_text(&recall))
                                        .clicked()
                                    {
                                        self.run_into(&recall, &mut actions);
                                        ui.close_menu();
                                    }
                                });
                            }
                        })
                        .response
                        .on_hover_text("Perspective / orthographic axis views and camera bookmarks");
                    });
                });
                self.scene_view(ui, runtime, &mut actions);
            });
        self.command_windows(ctx, &mut actions);
        self.world_window(ctx, &mut actions);
        self.save_prefs();
        actions
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

    fn hierarchy(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            heading(ui, "HIERARCHY");
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!self.dirty(), |ui| {
                    ui.menu_button("+ Object", |ui| {
                        for (preset, label) in [
                            ("empty", "Empty object"),
                            ("cube", "Cube"),
                            ("camera", "Camera"),
                            ("light.directional", "Directional light"),
                            ("light.point", "Point light"),
                            ("light.spot", "Spot light"),
                            ("light.area", "Area light (approximate)"),
                            ("trigger.box", "Box trigger"),
                            ("trigger.sphere", "Sphere trigger"),
                            ("path", "Path"),
                            ("obstacle", "Obstacle (nav.blocker)"),
                        ] {
                            if ui.button(label).clicked() {
                                actions.extend(self.create_object(preset));
                                ui.close_menu();
                            }
                        }
                    });
                });
            });
        });
        ui.add_space(6.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search…  tag: has: prefab:")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text("Space-separated filters that must all match: tag:NAME, has:COMPONENT (includes prefab-inherited), prefab:ID, or text in the ID/label");
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("scene_filter")
                .selected_text(
                    self.scene
                        .as_deref()
                        .map_or_else(|| "All scenes".into(), |s| truncate(s, 19)),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.scene, None, "All scenes");
                    for scene in &self.world.scenes {
                        let name = if scene.active {
                            scene.id.clone()
                        } else {
                            format!("{} · inactive", scene.id)
                        };
                        ui.selectable_value(&mut self.scene, Some(scene.id.clone()), name);
                    }
                });
            ui.add_enabled_ui(!self.dirty(), |ui| {
                ui.menu_button("Scene", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_scene).hint_text("New scene ID"),
                    );
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
                        ui.close_menu();
                    }
                    let filtered_scene = self
                        .scene
                        .as_ref()
                        .and_then(|id| self.world.scenes.iter().find(|s| &s.id == id))
                        .map(|s| (s.id.clone(), s.active));
                    if let Some((scene, active)) = filtered_scene {
                        let title = if active {
                            "Deactivate scene"
                        } else {
                            "Activate scene"
                        };
                        if ui.button(title).clicked() {
                            actions.extend(self.scene_set_active(&scene, !active));
                            ui.close_menu();
                        }
                        ui.menu_button("Delete scene…", |ui| {
                            caption(ui, "Removes membership. Objects are kept.");
                            if ui
                                .button(RichText::new("Delete scene").color(WARNING))
                                .clicked()
                            {
                                actions.extend(self.scene_delete(&scene));
                                ui.close_menu();
                            }
                        });
                    }
                });
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
        ui.horizontal(|ui| {
            caption(ui, format!("{} OBJECTS", rows.len()));
            let extra = self.selection().len();
            if extra > 1 {
                caption(ui, format!("· {extra} selected"));
            }
        });
        // Dropping a dragged row here makes it a root object.
        let (_, root_drop) = ui.dnd_drop_zone::<String, ()>(egui::Frame::NONE, |ui| {
            ui.label(
                RichText::new(
                    "Drop on a row = child · on its top/bottom edge = reorder · here = root",
                )
                .small()
                .color(MUTED),
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
                    let row = ui.horizontal(|ui| {
                        ui.set_min_height(ROW_HEIGHT);
                        ui.add_space(depth.min(7) as f32 * 9.0);
                        if branch {
                            let text = if self.collapsed.contains(id) {
                                "+"
                            } else {
                                "-"
                            };
                            if ui.small_button(text).clicked() {
                                toggle = Some(id.to_owned());
                            }
                        } else {
                            ui.add_space(17.0);
                        }
                        let eye = view_toggle(ui, own_hidden.contains(id), hidden.contains(id), false)
                            .on_hover_text("Hide / show in the editor view (H). Play and content are unaffected.");
                        rects.insert(format!("eye:{id}"), eye.rect);
                        if eye.clicked() {
                            flag_toggle = Some((id.to_owned(), false));
                        }
                        let lock = view_toggle(ui, own_locked.contains(id), locked.contains(id), true)
                            .on_hover_text("Lock / unlock in the editor view (L): not pickable or draggable in the scene view.");
                        rects.insert(format!("lock:{id}"), lock.rect);
                        if lock.clicked() {
                            flag_toggle = Some((id.to_owned(), true));
                        }
                        let entity = graph.objects[id];
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
                        } else if graph.active(id) {
                            Color32::from_rgb(227, 234, 242)
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
                        let selected =
                            self.selected.as_deref() == Some(id) || self.extra.contains(id);
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
                            ui.separator();
                            item(ui, "edit.copy", "Copy ID");
                            item(ui, "edit.select_children", "Select Children");
                            item(ui, "object.create_prefab", "Create Prefab From Selection");
                            item(ui, "edit.frame", "Focus");
                            ui.separator();
                            item(ui, "edit.toggle_hidden", "Hide / Show (Editor View)");
                            item(ui, "edit.toggle_locked", "Lock / Unlock (Editor View)");
                        });
                        for (text, badge) in badges(&self.world, entity) {
                            ui.label(RichText::new(text).monospace().size(9.0).color(badge))
                                .on_hover_text(badge_meaning(text));
                        }
                    });
                    let rect = row.response.rect;
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
        self.rects = rects;
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

    fn inspector(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        heading(ui, "INSPECTOR");
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
        let mut tween_ui = self.tween_ui.clone();
        let mut tween_request = None;
        let audio = self.selected.as_deref().and_then(|id| self.audio_clips(id));
        let mut audio_clip: Option<String> = None;
        let mut audio_preset: Option<&'static str> = None;
        self.refresh_tag_cache();
        let mut tag_request = None;
        let mut add_component = None;
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
        ui.horizontal(|ui| {
            ui.label(RichText::new(truncate(&id, 28)).monospace().color(ACCENT))
                .on_hover_text(&id);
            if ui.small_button("Copy ID").clicked() {
                ui.ctx().copy_text(id.clone());
            }
        });
        if stale {
            ui.label(RichText::new(format!("Stale draft · revision {}", draft.rev)).color(WARNING));
        }
        ui.horizontal(|ui| {
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
        });
        ui.separator();
        let mut duplicate = false;
        let mut delete = false;
        let mut structural = None;
        let entity = self.world.entity(&id);
        if let Some(entity) = entity {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_enabled_ui(!stale && self.drag.is_none(), |ui| {
                        caption(ui, "DISPLAY LABEL");
                        draft.label_dirty |= ui
                            .add(
                                egui::TextEdit::singleline(&mut draft.label)
                                    .hint_text("Uses stable ID if blank")
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                        if let Some(prefab) = &entity.prefab {
                            caption(ui, format!("Prefab · {prefab}"));
                        }
                        tag_request = super::world_panel::tag_chips(
                            ui,
                            &mut self.tag_ui,
                            !draft.dirty(),
                            &mut self.rects,
                        );
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
                            egui::CollapsingHeader::new("Tween timeline")
                                .default_open(true)
                                .show(ui, |ui| {
                                    let live = tween_ui.live.clone();
                                    let editable = !draft.dirty() && live.is_none();
                                    tween_request = super::timeline::timeline(
                                        ui,
                                        clips,
                                        &mut tween_ui,
                                        live.as_ref().map(|(c, t)| (c.as_str(), *t)),
                                        editable,
                                    );
                                });
                        }
                        egui::CollapsingHeader::new("Transform")
                            .default_open(true)
                            .show(ui, |ui| {
                                if let Some(t) = &mut draft.transform {
                                    let position = vector(
                                        ui,
                                        "Position",
                                        &mut t.position,
                                        0.05,
                                        -100000.0..=100000.0,
                                    );
                                    let rotate = vector(
                                        ui,
                                        "Rotation°",
                                        &mut draft.angles,
                                        0.5,
                                        -36000.0..=36000.0,
                                    );
                                    if rotate {
                                        t.rotation = viewport::rotation(draft.angles);
                                    }
                                    let scale =
                                        vector(ui, "Scale", &mut t.scale, 0.025, 0.0001..=10000.0);
                                    draft.transform_dirty |= position || rotate || scale;
                                } else if ui.button("Add transform").clicked() {
                                    draft.transform = Some(Transform::default());
                                    draft.transform_dirty = true;
                                }
                            });
                        egui::CollapsingHeader::new("Parent and scenes")
                            .default_open(true)
                            .show(ui, |ui| {
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
                        egui::CollapsingHeader::new("Components")
                            .default_open(true)
                            .show(ui, |ui| {
                                structural = component_editor(
                                    ui,
                                    &self.world,
                                    &self.component_hints,
                                    &self.registry,
                                    entity,
                                    draft,
                                    &mut add_component,
                                )
                                .or(structural.take());
                            });
                        if let Some((clips, current)) = &audio {
                            egui::CollapsingHeader::new("Audio source")
                                .default_open(true)
                                .show(ui, |ui| {
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
                                            ui.menu_button("New clip", |ui| {
                                                caption(ui, "Procedural presets (no audio files)");
                                                for preset in axiom_core::audio::CLIP_PRESETS {
                                                    if ui.button(preset).clicked() {
                                                        audio_preset = Some(preset);
                                                        ui.close_menu();
                                                    }
                                                }
                                            });
                                        });
                                    });
                                    caption(
                                        ui,
                                        "Edit volume, loop, distances and rolloff under Components ▸ audio.source; play with call audio.play in Play",
                                    );
                                });
                        }
                        if let Some(spline) = &path {
                            egui::CollapsingHeader::new("Path points")
                                .default_open(true)
                                .show(ui, |ui| {
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
        if copy || write.is_some() {
            match super::inspector_edits(&self.world, &self.registry, draft, &self.extra) {
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
                    }
                }
                Ok(_) => {}
                Err(error) => issue = Some(format!("Invalid component JSON · {error}")),
            }
        }
        if let Some(error) = issue {
            self.set_error(error);
        }
        if reload {
            self.select(self.selected.clone());
            self.notice("Reloaded the committed object");
        }
        match structural {
            Some(CoreEdit::RemoveComponent { entity, component }) => {
                actions.extend(self.remove_component(&entity, &component));
            }
            Some(edit) => actions.push(self.write(vec![Edit::Core(edit)], None)),
            None => {}
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
        if let Some(component) = add_component {
            actions.extend(self.add_component(&component));
        }
        if let Some(preset) = audio_preset {
            actions.extend(self.add_audio_clip(preset));
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
        if delete {
            actions.extend(self.delete_ids(&[id]));
        }
    }

    fn dock(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let mut resource = None;
        ui.horizontal(|ui| {
            let problems = if self.problems.is_empty() {
                "Problems".to_owned()
            } else {
                format!("Problems ({})", self.problems.len())
            };
            let proposals = if self.proposals.is_empty() {
                "Proposals".to_owned()
            } else {
                format!("Proposals ({})", self.proposals.len())
            };
            for (dock, title) in [
                (Dock::Changes, "Changes"),
                (Dock::Proposals, proposals.as_str()),
                (Dock::History, "History"),
                (Dock::Problems, problems.as_str()),
                (Dock::Scenarios, "Scenarios"),
                (Dock::Graphs, "Graphs"),
                (Dock::Assets, "Assets"),
                (Dock::Agent, "AI / Protocol"),
            ] {
                ui.selectable_value(&mut self.dock, dock, title);
            }
        });
        ui.separator();
        let mut jump = None;
        let mut open_prefab = None;
        let mut instantiate: Option<String> = None;
        let mut open_material = None;
        let mut declare: Option<String> = None;
        // Resource / material / prefab drafts would be reset by the write; the object draft is not.
        let asset_draft = self.dirty() && !self.draft.as_ref().is_some_and(Draft::dirty);
        ScrollArea::vertical().id_salt("editor_dock_scroll").auto_shrink([false,false]).show(ui, |ui| match self.dock {
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
                        if cols[0].selectable_label(self.proposal_focus == Some(id), RichText::new(text).monospace().small()).clicked() {
                            self.proposal_focus = Some(id);
                        }
                    }
                    let Some(p) = self.proposals.iter().find(|p| p["id"].as_i64() == self.proposal_focus) else { return };
                    let id = p["id"].as_i64().unwrap_or(0);
                    let base = p["base_rev"].as_i64().unwrap_or(0);
                    cols[1].horizontal(|ui| {
                        let clean = !self.dirty();
                        if ui.add_enabled(clean && base == self.rev, egui::Button::new("Approve"))
                            .on_hover_text("Apply against the proposal's base revision")
                            .clicked() { actions.push(Action::ResolveProposal { id, approve: true, rebase: false }); }
                        if ui.add_enabled(clean, egui::Button::new("Rebase & approve"))
                            .on_hover_text("Re-validate against the current revision, then apply")
                            .clicked() { actions.push(Action::ResolveProposal { id, approve: true, rebase: true }); }
                        if ui.button(RichText::new("Reject").color(WARNING)).clicked() {
                            actions.push(Action::ResolveProposal { id, approve: false, rebase: false });
                        }
                    });
                    if base != self.rev {
                        cols[1].label(RichText::new(format!("Base revision {base} is behind current {}. Approving as-is would conflict; rebase re-validates the same edits on top of the latest world.", self.rev)).small().color(WARNING));
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
                            if ui.selectable_label(selected, RichText::new(text).monospace()).clicked() && rev > 1 {
                                actions.push(Action::LoadDiff { rev });
                            }
                            if !current && ui.add_enabled(!self.dirty(), egui::Button::new("Restore").small())
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
            Dock::Graphs => self.graphs(ui),
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
            Dock::Assets => {
                ui.horizontal(|ui| {
                    caption(ui,"PREFABS");
                    if self.world.prefabs.is_empty() { caption(ui,"No prefabs in this world"); }
                });
                ui.horizontal_wrapped(|ui| {
                    for id in self.world.prefabs.keys() {
                        if ui.add_enabled(!self.dirty(),egui::Button::new(format!("◇  {id}"))).on_hover_text("Instantiate prefab").clicked() {
                            instantiate = Some(id.clone());
                        }
                        if ui.add_enabled(!self.dirty(),egui::Button::new("edit").small()).on_hover_text("Edit the prefab defaults all instances inherit").clicked() {
                            open_prefab = Some(id.clone());
                        }
                    }
                });
                let materials: Vec<String> = self.world.resources.get("render.shaders").and_then(|r| r["materials"].as_object()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
                if !materials.is_empty() { caption(ui,"MATERIALS"); }
                ui.horizontal_wrapped(|ui| {
                    for id in &materials {
                        if ui.add_enabled(!self.dirty(),egui::Button::new(format!("● {id}"))).on_hover_text("Edit material parameters").clicked() {
                            open_material = Some(id.clone());
                        }
                    }
                });
                caption(ui,"RESOURCES");
                ui.horizontal_wrapped(|ui| {
                    for id in self.world.resources.keys() {
                        if ui.add_enabled(!self.dirty(),egui::Button::new(id)).on_hover_text("Inspect and edit resource").clicked() {
                            resource=Some(id.clone());
                        }
                    }
                });
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
        if let Some(id) = resource {
            self.open_resource(&id);
        }
        if let Some(id) = jump {
            self.try_select(Some(id));
            self.frame_selection = true;
        }
        if let Some(id) = instantiate {
            actions.extend(self.instantiate_prefab(&id));
        }
        if let Some(id) = open_prefab {
            self.open_prefab(&id);
        }
        if let Some(id) = open_material {
            self.open_material(&id);
        }
        if let Some(name) = declare {
            actions.extend(self.declare_event(&name));
        }
    }
}

/// Hierarchy row height (also the virtualization step of `show_rows`).
const ROW_HEIGHT: f32 = 20.0;

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

/// A 14 px painted toggle (eye or padlock), so it does not depend on symbol glyphs.
/// `own` = flag set on this row, `inherited` = set on an ancestor.
fn view_toggle(ui: &mut Ui, own: bool, inherited: bool, lock: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    let on = own || inherited;
    let color = if own {
        WARNING
    } else if inherited {
        WARNING.gamma_multiply(0.5)
    } else if response.hovered() {
        Color32::from_gray(210)
    } else {
        Color32::from_gray(95)
    };
    let stroke = egui::Stroke::new(1.2_f32, color);
    let painter = ui.painter();
    let c = rect.center();
    if lock {
        let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 2.5), egui::vec2(9.0, 6.0));
        if on {
            painter.rect_filled(body, 1.0, color);
        } else {
            painter.rect_stroke(body, 1.0, stroke, egui::StrokeKind::Inside);
        }
        let (left, right, top) = (c.x - 2.5, c.x + 2.5, c.y - 4.5);
        painter.line_segment(
            [egui::pos2(left, body.top()), egui::pos2(left, top)],
            stroke,
        );
        painter.line_segment([egui::pos2(left, top), egui::pos2(right, top)], stroke);
        // An open padlock has a shorter right leg.
        let leg = if on { body.top() } else { top + 2.0 };
        painter.line_segment([egui::pos2(right, top), egui::pos2(right, leg)], stroke);
    } else {
        let points = (0..16)
            .map(|i| {
                let a = i as f32 / 16.0 * std::f32::consts::TAU;
                c + egui::vec2(6.0 * a.cos(), 3.4 * a.sin())
            })
            .collect();
        painter.add(egui::Shape::closed_line(points, stroke));
        painter.circle_filled(c, 1.7, color);
        if on {
            painter.line_segment(
                [c + egui::vec2(-6.0, -5.0), c + egui::vec2(6.0, 5.0)],
                stroke,
            );
        }
    }
    response
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

fn component_editor(
    ui: &mut Ui,
    world: &World,
    hints: &std::collections::BTreeMap<String, Value>,
    registry: &axiom_core::Registry,
    entity: &Entity,
    draft: &mut Draft,
    add_component: &mut Option<String>,
) -> Option<CoreEdit> {
    let inherited = entity.prefab.as_ref().and_then(|p| world.prefabs.get(p));
    let keys: std::collections::BTreeSet<_> = entity
        .components
        .keys()
        .chain(inherited.into_iter().flat_map(|p| p.components.keys()))
        .filter(|key| key.as_str() != "core.transform")
        .cloned()
        .collect();
    let mut remove = None;
    // Picking in Add component writes at once, so any unapplied draft edit blocks it.
    ui.add_enabled_ui(!draft.dirty(), |ui| {
        ui.horizontal_wrapped(|ui| {
            for key in &keys {
                if ui
                    .selectable_label(draft.component == *key, key.as_str())
                    .clicked()
                {
                    draft.open_component(world, entity, key);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut draft.component_input)
                    .hint_text("Component ID")
                    .desired_width(135.0),
            );
            if ui.small_button("Load").clicked() {
                let id = draft.component_input.trim().to_owned();
                draft.open_component(world, entity, &id);
            }
        });
        // Picker over every installed component the object lacks. Picking one writes it with
        // its default and whatever capability/systems it needs (ops::add_component).
        egui::ComboBox::from_id_salt("add_component")
            .selected_text("Add component…")
            .width(220.0)
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
                        response
                            .on_hover_text("Adds what it needs to the world in the same revision")
                    };
                    if response.clicked() {
                        *add_component = Some(id.clone());
                    }
                }
            });
    });
    if !draft.component.is_empty() && draft.component != "core.transform" {
        let known = hints.contains_key(&draft.component)
            || world.component_schemas.contains_key(&draft.component);
        if !known && !keys.contains(&draft.component) {
            ui.label(
                RichText::new(format!(
                    "Unknown component '{}'. Check the ID (schema components lists them).",
                    draft.component
                ))
                .color(WARNING),
            );
        }
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!draft.dirty(), |ui| {
                if ui
                    .selectable_label(!draft.json_mode, "Form")
                    .on_hover_text("Edit the effective value; only changed fields are written")
                    .clicked()
                    && draft.form.is_some()
                {
                    draft.json_mode = false;
                }
                if ui
                    .selectable_label(draft.json_mode, "JSON")
                    .on_hover_text("Edit the authored override as raw JSON")
                    .clicked()
                {
                    draft.json_mode = true;
                }
            });
        });
        if !draft.json_mode {
            if let Some(form) = &mut draft.form {
                let hint = super::form::hints_for(hints, world, &draft.component);
                draft.form_dirty |= super::form::edit(ui, &draft.component, form, hint.as_ref());
                if draft.form_dirty {
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
            return remove;
        }
    }
    if !draft.component.is_empty() && draft.component != "core.transform" {
        let own = entity.components.contains_key(&draft.component);
        let inherits = inherited.is_some_and(|p| p.components.contains_key(&draft.component));
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
                remove = Some(CoreEdit::RemoveComponent {
                    entity: entity.id.clone(),
                    component: draft.component.clone(),
                });
            }
        });
    }
    remove
}

fn vector(
    ui: &mut Ui,
    name: &str,
    value: &mut [f32; 3],
    speed: f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    let mut changed = false;
    caption(ui, name);
    ui.horizontal(|ui| {
        for (v, axis) in value.iter_mut().zip(["X ", "Y ", "Z "]) {
            changed |= ui
                .add_sized(
                    [68.0, 20.0],
                    egui::DragValue::new(v)
                        .prefix(axis)
                        .speed(speed)
                        .range(range.clone())
                        .max_decimals(3),
                )
                .changed();
        }
    });
    changed
}

fn truncate(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut display: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        display.push('…');
    }
    display
}
