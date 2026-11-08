use super::{Action, Editor};
use anyhow::{Context, Result};
use axiom_core::{local::Target, CoreEdit, Edit, Request, World};
use egui::{RichText, ScrollArea};
use serde_json::Value;

pub(super) struct ResourceDraft {
    pub rev: i64,
    pub id: String,
    pub field: String,
    pub field_input: String,
    text: String,
    pub dirty: bool,
}
impl ResourceDraft {
    pub fn load(&mut self, world: &World) -> Result<()> {
        let source = world
            .resources
            .get(&self.id)
            .context("resource was removed")?;
        let field = self.field_input.trim();
        let value = if field.is_empty() {
            source
        } else {
            source
                .pointer(field)
                .context("JSON pointer does not exist")?
        };
        let text = serde_json::to_string_pretty(value)?;
        anyhow::ensure!(
            text.len() <= 65536,
            "resource projection exceeds 64 KiB; choose a narrower JSON pointer"
        );
        self.field = field.into();
        self.text = text;
        self.dirty = false;
        Ok(())
    }
    fn request(&self, preview: bool) -> Result<Request> {
        let value: Value = serde_json::from_str(&self.text)?;
        let target = Target::Resource {
            id: self.id.clone(),
        };
        let edit = if self.field.is_empty() {
            CoreEdit::PutObject { target, value }
        } else {
            CoreEdit::SetField {
                target,
                field: self.field.clone(),
                value,
            }
        };
        let expected_rev = Some(self.rev);
        let edits = vec![Edit::Core(edit)];
        Ok(if preview {
            Request::Preview {
                expected_rev,
                edits,
            }
        } else {
            Request::Apply {
                expected_rev,
                edits,
            }
        })
    }
}
impl Editor {
    pub(super) fn open_resource(&mut self, id: &str) {
        self.select(None);
        if !self.world.resources.contains_key(id) {
            return;
        }
        let mut draft = ResourceDraft {
            rev: self.rev,
            id: id.into(),
            field: String::new(),
            field_input: String::new(),
            text: String::new(),
            dirty: false,
        };
        if let Err(error) = draft.load(&self.world) {
            self.set_error(format!("{error:#}"));
        }
        self.resource = Some(draft);
    }
    pub(super) fn resource_inspector(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let draft = self.resource.as_mut().unwrap();
        let stale = draft.rev != self.rev;
        ui.label(
            RichText::new(&draft.id)
                .monospace()
                .color(egui::Color32::from_rgb(105, 222, 195)),
        );
        if stale {
            ui.label("Stale draft. Copy your edits or reload the committed resource.");
        }
        let mut reload = false;
        let mut issue = None;
        ui.horizontal(|ui| {
            for (title, preview) in [("Apply", false), ("Preview", true)] {
                if ui
                    .add_enabled(draft.dirty && !stale, egui::Button::new(title))
                    .clicked()
                {
                    match draft.request(preview) {
                        Ok(request) => actions.push(Action::Write {
                            request,
                            select: None,
                        }),
                        Err(error) => issue = Some(format!("{error:#}")),
                    }
                }
            }
            reload = ui.button("Reload").clicked();
            if ui.button("Copy JSON").clicked() {
                match draft.request(false) {
                    Ok(request) => ui
                        .ctx()
                        .copy_text(serde_json::to_string_pretty(&request).unwrap()),
                    Err(error) => issue = Some(format!("{error:#}")),
                }
            }
        });
        ui.separator();
        ui.label(RichText::new("JSON POINTER").small());
        ui.add_enabled_ui(!draft.dirty && !stale, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut draft.field_input)
                    .hint_text("/abilities/slash2")
                    .desired_width(f32::INFINITY),
            );
            if ui.small_button("Load fields").clicked() {
                if let Err(error) = draft.load(&self.world) {
                    issue = Some(format!("{error:#}"));
                }
            }
        });
        ui.label(
            RichText::new(if draft.field.is_empty() {
                "Editing resource".into()
            } else {
                format!("Editing {}", draft.field)
            })
            .small(),
        );
        ScrollArea::vertical()
            .id_salt("editor_resource_json")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                draft.dirty |= ui
                    .add_enabled(
                        !stale,
                        egui::TextEdit::multiline(&mut draft.text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(24),
                    )
                    .changed();
            });
        if let Some(error) = issue {
            self.set_error(error);
        }
        if reload {
            self.reset_draft();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn resource_inspector_edits_one_projection_and_retains_a_conflicted_draft() {
        let registry = crate::installed_registry();
        let world = axiom_core::ContentPackage::load(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/action"),
            &registry.authoring,
        )
        .unwrap()
        .world;
        let mut store = axiom_core::Store::memory_with(registry.authoring, world.clone()).unwrap();
        let mut editor = Editor::new(world, 1, Some("action.abilities".into()));
        let draft = editor.resource.as_mut().unwrap();
        draft.field_input = "/abilities/slash2/dodge_window".into();
        draft.load(&editor.world).unwrap();
        draft.text = json!({"start":25,"end":70}).to_string();
        draft.dirty = true;
        let preview = store.execute(draft.request(true).unwrap()).unwrap();
        assert_eq!(preview["diff"]["changes"].as_array().unwrap().len(), 1);
        let apply = draft.request(false).unwrap();
        assert_eq!(store.execute(apply).unwrap()["rev"], 2);
        let (rev, world) = store.current().unwrap();
        editor.update(rev, world, false);
        assert!(editor.dirty());
        assert_eq!(editor.resource.as_ref().unwrap().rev, 1);
        assert!(store
            .execute(editor.resource.as_ref().unwrap().request(false).unwrap())
            .is_err());
        editor.reset_draft();
        assert!(!editor.dirty());
        assert_eq!(
            editor.resource.as_ref().unwrap().field,
            "/abilities/slash2/dodge_window"
        );
        assert_eq!(editor.resource.as_ref().unwrap().rev, 2);
    }
}
