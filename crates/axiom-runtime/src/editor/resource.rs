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
    /// True only after a projection loaded successfully. A failed root load must never
    /// allow an empty editor to be written back as a whole-resource replacement.
    pub loaded: bool,
    /// Schema-guided form over the loaded projection (objects only) and its baseline; only
    /// changed leaves are written, as `set_field` edits.
    pub form: Option<Value>,
    form_base: Value,
    pub form_mode: bool,
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
        self.form = value.is_object().then(|| value.clone());
        self.form_base = value.clone();
        self.form_mode = self.form.is_some();
        self.text = text;
        self.dirty = false;
        self.loaded = true;
        Ok(())
    }
    /// Live Inspector: the form values were just sent as a write and become the baseline.
    pub(super) fn mark_committed(&mut self) {
        if let Some(form) = &self.form {
            self.form_base = form.clone();
        }
        self.dirty = false;
    }

    pub(super) fn request(&self, preview: bool) -> Result<Request> {
        anyhow::ensure!(
            self.loaded,
            "no resource projection is loaded; choose a narrower JSON pointer and load it first"
        );
        let target = Target::Resource {
            id: self.id.clone(),
        };
        let edits = match self.form.as_ref().filter(|_| self.form_mode) {
            Some(form) => {
                let mut leaves = vec![];
                super::form::changes(&self.form_base, form, "", &mut leaves);
                anyhow::ensure!(!leaves.is_empty(), "no field changed");
                leaves
                    .into_iter()
                    .map(|(pointer, value)| {
                        Edit::Core(CoreEdit::SetField {
                            target: target.clone(),
                            field: format!("{}{pointer}", self.field),
                            value,
                        })
                    })
                    .collect()
            }
            None => {
                let value: Value = serde_json::from_str(&self.text)?;
                vec![Edit::Core(if self.field.is_empty() {
                    CoreEdit::PutObject { target, value }
                } else {
                    CoreEdit::SetField {
                        target,
                        field: self.field.clone(),
                        value,
                    }
                })]
            }
        };
        let expected_rev = Some(self.rev);
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
            loaded: false,
            form: None,
            form_base: Value::Null,
            form_mode: false,
        };
        if let Err(error) = draft.load(&self.world) {
            self.set_error(format!("{error:#}"));
        }
        self.resource = Some(draft);
    }
    /// Resource form array buttons (`ACL: append|insert|remove|reorder resources[id=R] F ..`).
    pub(super) fn edit_resource_array(
        &mut self,
        id: &str,
        field: &str,
        edit: &axiom_core::ops::ArrayEdit,
    ) -> Option<Action> {
        let mut select =
            axiom_core::local::Selection::all(axiom_core::local::Collection::Resources);
        select.ids = vec![id.to_owned()];
        let result = axiom_core::ops::edit_array(&self.world, &self.registry, &select, field, edit);
        self.op_write(result, None)
    }

    pub(super) fn resource_inspector(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let spec = self
            .resource
            .as_ref()
            .map(|d| d.id.clone())
            .map(|id| self.specs.get(&self.registry, &self.world, &id));
        let live = self.prefs.live_inspector;
        let esc_taken = ui.ctx().memory(|m| m.any_popup_open())
            || self.picker.is_some()
            || self.options.is_some();
        let saving = self.saving;
        let world = self.world.clone();
        let world_version = self.world_version;
        let mut array = None;
        let draft = self.resource.as_mut().unwrap();
        let stale = draft.rev != self.rev;
        let loaded = draft.loaded;
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
                    .add_enabled(draft.dirty && !stale && loaded, egui::Button::new(title))
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
            if ui
                .add_enabled(loaded, egui::Button::new("Copy JSON"))
                .clicked()
            {
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
        if !loaded {
            ui.label(
                RichText::new("Nothing loaded. Enter a JSON pointer and press Load fields.")
                    .color(egui::Color32::from_rgb(242, 176, 92)),
            );
        }
        ui.label(
            RichText::new(if !loaded {
                "Read-only until a projection loads".into()
            } else if draft.field.is_empty() {
                "Editing resource".into()
            } else {
                format!("Editing {}", draft.field)
            })
            .small(),
        );
        if draft.form.is_some() {
            ui.add_enabled_ui(!draft.dirty, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(draft.form_mode, "Form")
                        .on_hover_text("Schema-guided fields; only changed fields are written")
                        .clicked()
                    {
                        draft.form_mode = true;
                    }
                    if ui
                        .selectable_label(!draft.form_mode, "JSON")
                        .on_hover_text("Edit the projection as raw JSON (replaces it)")
                        .clicked()
                    {
                        draft.form_mode = false;
                    }
                });
            });
        }
        if draft.form.is_some() && draft.form_mode {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.field_filter)
                    .hint_text("Filter fields")
                    .desired_width(160.0),
            );
            self.rects.insert("form:filter".into(), r.rect);
        }
        let needle = self.field_filter.trim().to_lowercase();
        ScrollArea::vertical()
            .id_salt("editor_resource_json")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                match draft.form.as_mut().filter(|_| draft.form_mode) {
                    Some(form) => {
                        let fallback = super::form::Spec::compile(&serde_json::Value::Null);
                        let spec = spec
                            .as_deref()
                            .and_then(|s| s.at(&draft.field))
                            .unwrap_or(&fallback);
                        let mixed = Default::default();
                        ui.add_enabled_ui(!stale && loaded, |ui| {
                            let mut ctx = super::form::Ctx {
                                world: &world,
                                world_version,
                                base: &draft.form_base,
                                mixed: &mixed,
                                filter: &needle,
                                structural: !draft.dirty,
                                picker: &mut self.picker,
                                rects: Some(&mut self.rects),
                                overrides: None,
                            };
                            let out = super::form::show(ui, &mut ctx, spec, form, &draft.id);
                            draft.dirty |= out.changed;
                            if let Some((pointer, edit)) = out.array {
                                array = Some((format!("{}{pointer}", draft.field), edit));
                            }
                        });
                    }
                    None => {
                        draft.dirty |= ui
                            .add_enabled(
                                !stale && loaded,
                                egui::TextEdit::multiline(&mut draft.text)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(24),
                            )
                            .changed();
                    }
                }
            });
        // Live: Esc goes back to the committed values; an edited form commits once nothing
        // is held or typed any more (one revision per interaction).
        let mut cancel = false;
        if draft.dirty && draft.form_mode && loaded && !stale {
            let (escape, held) =
                ui.input(|i| (i.key_pressed(egui::Key::Escape), i.pointer.any_down()));
            let typing = ui
                .ctx()
                .memory(|m| m.focused())
                .is_some_and(|f| egui::TextEdit::load_state(ui.ctx(), f).is_some());
            if escape && live && !esc_taken {
                cancel = true;
                let ctx = ui.ctx();
                ctx.stop_dragging();
                if let Some(f) = ctx.memory(|m| m.focused()) {
                    ctx.memory_mut(|m| m.surrender_focus(f));
                }
            } else if live && !held && !typing && saving == 0 {
                match draft.request(false) {
                    Ok(request) => {
                        actions.push(Action::Write {
                            request,
                            select: None,
                        });
                        draft.mark_committed();
                    }
                    Err(_) => draft.mark_committed(),
                }
            }
        }
        let id = draft.id.clone();
        if let Some(error) = issue {
            self.set_error(error);
        }
        if reload || cancel {
            self.reset_draft();
        }
        if let Some((field, edit)) = array {
            actions.extend(self.edit_resource_array(&id, &field, &edit));
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
        // The projection is an object, so it opens as a form; this test edits raw JSON.
        assert!(draft.form_mode);
        draft.form_mode = false;
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

    #[test]
    fn failed_root_load_cannot_replace_the_whole_resource() {
        let mut world = World::empty("huge");
        let big = "x".repeat(70_000);
        world
            .resources
            .insert("huge.blob".into(), json!({"keep":1,"payload":big}));
        let mut editor = Editor::new(world, 1, Some("huge.blob".into()));
        let draft = editor.resource.as_mut().unwrap();
        assert!(
            !draft.loaded,
            "root projection exceeds 64 KiB and must not load"
        );
        // Simulate a user typing into the (disabled) editor anyway.
        draft.text = json!({"keep":2}).to_string();
        draft.dirty = true;
        let error = draft.request(false).unwrap_err().to_string();
        assert!(
            error.contains("no resource projection is loaded"),
            "{error}"
        );
        // Loading a narrow pointer re-enables writes, scoped to that field only.
        draft.field_input = "/keep".into();
        draft.load(&editor.world).unwrap();
        assert!(draft.loaded);
        match draft.request(false).unwrap() {
            Request::Apply { edits, .. } => assert!(matches!(
                &edits[0],
                Edit::Core(CoreEdit::SetField { field, .. }) if field == "/keep"
            )),
            other => panic!("unexpected request {other:?}"),
        }
    }
}
