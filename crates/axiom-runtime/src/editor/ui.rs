use super::{viewport, Action, Dock, Draft, Editor, Tool};
use crate::world::WorldRuntime;
use axiom_core::{
    local::Target, merge_json, scene_edit::Hierarchy, Camera, CoreEdit, Edit, Entity, MeshPart,
    RenderMesh, Request, Scene, Transform, World,
};
use egui::{Color32, Frame, Key, Layout, Margin, RichText, ScrollArea, Ui};
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
        if !text_focused {
            let mut copied = None;
            ctx.input(|i| {
                // Bare letter shortcuts only; Ctrl+R / Ctrl+W etc. belong to other commands.
                let bare = !i.modifiers.any();
                for (key, tool) in [
                    (Key::Q, Tool::Select),
                    (Key::W, Tool::Move),
                    (Key::E, Tool::Rotate),
                    (Key::R, Tool::Scale),
                ] {
                    if bare && i.key_pressed(key) && self.drag.is_none() {
                        self.tool = tool;
                    }
                }
                self.frame_selection |= bare && i.key_pressed(Key::F);
                if i.key_pressed(Key::F5) && !self.dirty() {
                    actions.push(Action::Play);
                }
                let undo = i.modifiers.command && i.key_pressed(Key::Z);
                let redo =
                    i.modifiers.command && (i.key_pressed(Key::Y) || (undo && i.modifiers.shift));
                if (undo || redo) && !self.dirty() {
                    if let Some(action) = self.restore_action(redo) {
                        actions.push(action);
                    }
                }
                let idle = !self.dirty() && self.drag.is_none();
                if idle && bare && i.key_pressed(Key::Delete) {
                    actions.extend(self.delete_selection());
                }
                if idle && i.modifiers.command && i.key_pressed(Key::D) {
                    actions.extend(self.duplicate_ids(&self.selection()));
                }
                // egui-winit turns Ctrl+C / Ctrl+V into Copy / Paste events, not key presses.
                for event in &i.events {
                    match event {
                        egui::Event::Copy if self.selected.is_some() => {
                            self.clipboard = self.selection();
                            copied = Some(self.clipboard.join("\n"));
                        }
                        egui::Event::Paste(text) if idle => {
                            // Pasted text may be an ID list from another window or an agent.
                            let ids: Vec<String> = text
                                .lines()
                                .map(str::trim)
                                .filter(|id| self.world.entity(id).is_some())
                                .map(str::to_owned)
                                .collect();
                            let ids = if ids.is_empty() {
                                self.clipboard.clone()
                            } else {
                                ids
                            };
                            actions.extend(self.duplicate_ids(&ids));
                        }
                        _ => {}
                    }
                }
            });
            if let Some(text) = copied {
                ctx.copy_text(text);
                self.notice(format!("Copied {} object ID(s)", self.clipboard.len()));
            }
            let escape = ctx.input(|i| i.key_pressed(Key::Escape));
            if escape && self.drag.is_none() && !self.dirty() {
                self.extra.clear();
                self.select(None);
            }
        }
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
        egui::TopBottomPanel::bottom("editor_dock")
            .resizable(true)
            .default_height(100.0)
            .height_range(72.0..=240.0)
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
                    ui.horizontal(|ui| {
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
                        ui.toggle_value(&mut self.snap, "Snap").on_hover_text(
                            "0.25 m · 15° · 0.1 scale. Hold Ctrl while dragging to invert.",
                        );
                        ui.toggle_value(&mut self.grid, "Grid")
                            .on_hover_text("Always draw the ground grid");
                    });
                });
                self.scene_view(ui, runtime, &mut actions);
            });
        actions
    }

    fn restore_action(&self, redo: bool) -> Option<Action> {
        let rev = *if redo {
            self.redo.last()?
        } else {
            self.undo.last()?
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
                        ] {
                            if ui.button(label).clicked() {
                                actions.push(self.create_object(preset, None));
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
                .hint_text("Search objects…")
                .desired_width(f32::INFINITY),
        );
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
                        {
                            actions.push(self.write(
                                vec![Edit::Core(CoreEdit::PutScene {
                                    scene: Scene {
                                        id,
                                        entities: vec![],
                                        active: true,
                                    },
                                })],
                                None,
                            ));
                            self.new_scene.clear();
                            ui.close_menu();
                        }
                    }
                    if let Some(scene) = self
                        .scene
                        .as_ref()
                        .and_then(|id| self.world.scenes.iter().find(|s| &s.id == id))
                    {
                        let title = if scene.active {
                            "Deactivate scene"
                        } else {
                            "Activate scene"
                        };
                        if ui.button(title).clicked() {
                            actions.push(self.write(
                                vec![Edit::Core(CoreEdit::SetField {
                                    target: Target::Scene {
                                        id: scene.id.clone(),
                                    },
                                    field: "/active".into(),
                                    value: json!(!scene.active),
                                })],
                                None,
                            ));
                            ui.close_menu();
                        }
                        ui.menu_button("Delete scene…", |ui| {
                            caption(ui, "Removes membership. Objects are kept.");
                            if ui
                                .button(RichText::new("Delete scene").color(WARNING))
                                .clicked()
                            {
                                actions.push(self.write(
                                    vec![Edit::Core(CoreEdit::RemoveObject {
                                        target: Target::Scene {
                                            id: scene.id.clone(),
                                        },
                                    })],
                                    None,
                                ));
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
                }) && (id.to_lowercase().contains(&term)
                    || e.label
                        .as_ref()
                        .is_some_and(|n| n.to_lowercase().contains(&term)));
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
                RichText::new("Drop a row here to make it a root")
                    .small()
                    .color(MUTED),
            );
        });
        let mut reparent = root_drop.map(|id| ((*id).clone(), None));
        let modifiers = ui.input(|i| i.modifiers);
        let mut picked = None;
        let mut toggle = None;
        let mut scroll = ScrollArea::vertical()
            .id_salt("editor_objects")
            .auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_selection) {
            if let Some(i) = rows
                .iter()
                .position(|(id, _, _)| Some(*id) == self.selected.as_deref())
            {
                let height = 20.0 + ui.spacing().item_spacing.y;
                scroll = scroll.vertical_scroll_offset(
                    (i as f32 * height - ui.available_height() * 0.4).max(0.0),
                );
            }
        }
        scroll.show_rows(ui, 20.0, rows.len(), |ui, range| {
            for (id, depth, branch) in rows[range].iter().copied() {
                ui.push_id(id, |ui| {
                    ui.horizontal(|ui| {
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
                        let entity = graph.objects[id];
                        let icon = if entity.prefab.is_some() { "◇" } else { "·" };
                        let color = if graph.active(id) {
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
                        let row = ui.dnd_drag_source(
                            egui::Id::new(("tree_row", id)),
                            id.to_owned(),
                            |ui| {
                                ui.selectable_label(
                                    selected,
                                    RichText::new(format!(
                                        "{icon}  {crumb}{}",
                                        truncate(label, 26)
                                    ))
                                    .color(color),
                                )
                            },
                        );
                        let response = row.inner.on_hover_text(id);
                        if response.clicked() {
                            picked = Some(id.to_owned());
                        }
                        if let Some(dragged) = row.response.dnd_release_payload::<String>() {
                            if dragged.as_str() != id {
                                reparent = Some(((*dragged).clone(), Some(id.to_owned())));
                            }
                        }
                    });
                });
            }
        });
        if let Some(id) = toggle {
            if !self.collapsed.remove(&id) {
                self.collapsed.insert(id);
            }
        }
        if let Some(id) = picked {
            self.click_select(Some(id), modifiers);
        }
        if let Some((entity, parent)) = reparent.filter(|_| !self.dirty()) {
            actions.push(self.write(
                vec![Edit::Core(CoreEdit::Reparent {
                    entity,
                    parent,
                    keep_world: true,
                })],
                None,
            ));
        }
    }

    fn inspector(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        heading(ui, "INSPECTOR");
        if self.resource.is_some() {
            self.resource_inspector(ui, actions);
            return;
        }
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
                        ui.add_space(8.0);
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
                                    entity,
                                    draft,
                                )
                                .or(structural.take());
                            });
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
            match draft.edits() {
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
        if let Some(edit) = structural {
            actions.push(self.write(vec![Edit::Core(edit)], None));
        }
        if duplicate {
            let new = self.next_id(&format!("{}/copy", id.chars().take(24).collect::<String>()));
            actions.push(self.write(
                vec![Edit::Core(CoreEdit::DuplicateEntity {
                    source: id.clone(),
                    id: new.clone(),
                })],
                Some(new),
            ));
        }
        if delete {
            actions.push(self.write(vec![Edit::Core(CoreEdit::DeleteEntity { id })], None));
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
                (Dock::Assets, "Assets"),
                (Dock::Agent, "AI / Protocol"),
            ] {
                ui.selectable_value(&mut self.dock, dock, title);
            }
            ui.separator();
            ui.label(
                RichText::new(truncate(&self.status, 95)).color(if self.error {
                    WARNING
                } else {
                    MUTED
                }),
            )
            .on_hover_text(&self.status);
        });
        ui.separator();
        let mut jump = None;
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
                        ui.label(p["message"].as_str().unwrap_or(""));
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
                            actions.push(self.create_object("instance",Some(id)));
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
                    if ui.add_enabled(self.selected.is_some(), egui::Button::new("Copy selection")).clicked() {
                        ui.ctx().copy_text(json!({"op":"select","query":{"from":"entities","ids":[self.selected],"fields":["label","components/core.transform"]}}).to_string());
                    }
                    if let Some(request) = &self.last_request {
                        if ui.button("Copy last request").clicked() {
                            ui.ctx().copy_text(serde_json::to_string_pretty(request).unwrap());
                        }
                    }
                });
                caption(ui,"Stable IDs · expected revision · local diffs · durable execute receipts");
            }
        });
        if let Some(id) = resource {
            self.open_resource(&id);
        }
        if let Some(id) = jump {
            self.try_select(Some(id));
            self.frame_selection = true;
        }
    }

    fn next_id(&self, base: &str) -> String {
        if self.world.entity(base).is_none() {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base}/{n}"))
            .find(|id| self.world.entity(id).is_none())
            .unwrap()
    }

    fn create_object(&self, preset: &str, prefab: Option<&str>) -> Action {
        let mut edits = Vec::new();
        let id = self.next_id(preset);
        let mut entity = Entity::new(&id);
        entity.prefab = prefab.map(str::to_owned);
        if prefab.is_none() {
            let mut transform = Transform::default();
            if preset == "cube" {
                transform.position[1] = 0.5;
            }
            if preset == "camera" {
                transform.position = [0.0, 3.0, 6.0];
                transform.rotation = viewport::rotation([-16.0, 0.0, 0.0]);
            }
            entity.set("core.transform", transform).unwrap();
        }
        if matches!(preset, "cube" | "camera") && !self.world.capabilities.contains_key("render") {
            let mut capabilities = self.world.capabilities.clone();
            capabilities.insert("render".into(), 1);
            edits.push(Edit::set("/capabilities", json!(capabilities)));
        }
        if preset == "cube" {
            entity
                .set(
                    "render.mesh",
                    RenderMesh {
                        parts: vec![MeshPart {
                            transform: Transform::default(),
                            color: [0.20, 0.65, 0.73],
                            emission: 0.0,
                            grid: false,
                        }],
                    },
                )
                .unwrap();
        }
        if preset == "camera" {
            let mut camera = Camera::perspective();
            camera.active = !self.world.entities.iter().any(|e| {
                component_value(&self.world, e, "render.camera")
                    .and_then(|v| serde_json::from_value::<Camera>(v).ok())
                    .is_some_and(|c| c.active)
            });
            entity.set("render.camera", camera).unwrap();
        }
        edits.push(Edit::Core(CoreEdit::PutEntity { entity }));
        if let Some(scene) = &self.scene {
            edits.push(Edit::Core(CoreEdit::SetSceneMembership {
                scene: scene.clone(),
                entity: id.clone(),
                member: true,
            }));
        }
        self.write(edits, Some(id))
    }
}

fn component_editor(
    ui: &mut Ui,
    world: &World,
    hints: &std::collections::BTreeMap<String, Value>,
    entity: &Entity,
    draft: &mut Draft,
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
    ui.add_enabled_ui(!draft.component_dirty && !draft.form_dirty, |ui| {
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
