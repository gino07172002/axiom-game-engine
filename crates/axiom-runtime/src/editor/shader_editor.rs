//! Material inspector ▸ Shader source: the custom shader's WGSL surface code with a line
//! gutter. Check validates the draft the way the engine does (`shader::validate_hook`, naga
//! parse + validate) on a background job (explicit, one at a time, cancellable, 10 s
//! watchdog, joined with a timeout when the editor goes away) and marks the reported
//! `line N, column M` in the gutter. Apply writes the source (`ACL: shader ID source ".."`);
//! the world validator compiles it again on commit and refuses an invalid hook.
use super::icons::Icon;
use super::tuning::mark;
use super::{theme, Action, Editor};
use crate::jobs::{Job, Outcome};
use axiom_core::ops;
use egui::{pos2, vec2, FontId, Rect, RichText, Ui};
use serde_json::{json, Value};
use std::time::Duration;

const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const JOIN_LIMIT: Duration = Duration::from_millis(500);
const CODE_SIZE: f32 = 12.5;

/// One validation message; `line` / `column` are 1-based in the hook (no location when the
/// error is not tied to a line, e.g. a missing `surface` function).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Issue {
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

/// What a finished check found, for the source it checked.
#[derive(Clone, Debug)]
pub(crate) struct Checked {
    pub source: String,
    pub issues: Vec<Issue>,
    pub elapsed_ms: f64,
    pub state: &'static str,
}

#[derive(Default)]
pub(crate) struct ShaderUi {
    /// The custom shader under edit and its source as authored at load.
    pub shader: Option<String>,
    pub base: String,
    pub draft: String,
    pub job: Option<Job<(String, Vec<Issue>, f64)>>,
    pub checked: Option<Checked>,
    /// Line-number gutter text, rebuilt when the line count changes.
    gutter: (usize, String),
}

impl Drop for ShaderUi {
    fn drop(&mut self) {
        if let Some(job) = &mut self.job {
            job.shutdown(JOIN_LIMIT);
        }
    }
}

impl ShaderUi {
    pub fn observation(&self) -> Value {
        json!({
            "shader":self.shader,
            "dirty":self.draft != self.base,
            "running":self.job.as_ref().map(Job::observation),
            "checked":self.checked.as_ref().map(|c| json!({
                "state":c.state,
                "current":c.source == self.draft,
                "elapsed_ms":(c.elapsed_ms * 100.0).round() / 100.0,
                "issues":c.issues.iter().map(|i| json!({"line":i.line,"column":i.column,"message":i.message})).collect::<Vec<_>>(),
            })),
        })
    }
}

/// `validate_hook`'s verdict on `source` as issues (none = valid).
pub(crate) fn check(id: &str, source: &str) -> Vec<Issue> {
    match crate::shader::validate_hook(id, source) {
        Ok(()) => vec![],
        Err(e) => {
            let message = e
                .downcast_ref::<axiom_core::Diagnostic>()
                .map_or_else(|| format!("{e:#}"), |d| d.message.clone());
            let at = crate::shader::error_location(&message);
            vec![Issue {
                line: at.map(|a| a.0),
                column: at.map(|a| a.1),
                message,
            }]
        }
    }
}

impl Editor {
    /// The custom shader of the material open in the Inspector, if it has one.
    pub(super) fn shader_under_edit(&self) -> Option<String> {
        let id = self.material.as_ref()?.value["shader"].as_str()?;
        self.world
            .resources
            .get(ops::SHADER_LIBRARY)?
            .get("shaders")?
            .get(id)?
            .get("wgsl")?;
        Some(id.to_owned())
    }

    /// Writes the shader source draft (`ACL: shader ID source ".."`).
    pub(super) fn apply_shader_source(&mut self, id: &str, wgsl: &str) -> Option<Action> {
        let result = ops::set_shader_source(&self.world, id, wgsl);
        self.tuning_write(result)
    }

    /// `shader.check`: validates the draft of the open material's shader in the background.
    pub(super) fn shader_check_command(&mut self) -> anyhow::Result<()> {
        let id = self
            .shader_under_edit()
            .ok_or_else(|| anyhow::anyhow!("open a material with a custom shader first"))?;
        self.sync_shader_draft(&id);
        let source = self.tuning.shader.draft.clone();
        let job = Job::spawn("shader_check", id.clone(), CHECK_TIMEOUT, move |progress| {
            progress.set_total(1);
            if progress.cancelled() {
                return (source, vec![], 0.0);
            }
            let started = std::time::Instant::now();
            let issues = check(&id, &source);
            progress.set_done(1);
            (source, issues, started.elapsed().as_secs_f64() * 1000.0)
        })?;
        self.tuning.shader.job = Some(job);
        self.notice("Checking shader…");
        Ok(())
    }

    /// `shader.draft "WGSL"` (view state): replaces the open shader's unsaved draft, as typing
    /// in the source box would (Check / Apply then act on it).
    pub(super) fn shader_draft_command(&mut self, arg: Option<&str>) -> anyhow::Result<()> {
        let id = self
            .shader_under_edit()
            .ok_or_else(|| anyhow::anyhow!("open a material with a custom shader first"))?;
        let Some(arg) = arg else {
            // From the menu: the person types in the Shader source box.
            self.show_dock(super::Dock::Inspector);
            return Ok(());
        };
        let source: String = serde_json::from_str(arg).map_err(|_| {
            anyhow::anyhow!("shader.draft takes the source as a JSON string (\"..\\n..\")")
        })?;
        anyhow::ensure!(
            source.len() <= ops::MAX_SHADER_SOURCE,
            "shader source is at most {} bytes",
            ops::MAX_SHADER_SOURCE
        );
        self.sync_shader_draft(&id);
        self.tuning.shader.draft = source;
        Ok(())
    }

    pub(super) fn cancel_shader_check(&mut self) {
        if let Some(job) = &mut self.tuning.shader.job {
            job.cancel();
        }
    }

    /// Once per frame (from `poll_jobs`): collects a finished check.
    pub(crate) fn poll_shader_check(&mut self) {
        let Some(job) = &mut self.tuning.shader.job else {
            return;
        };
        let Some(outcome) = job.poll() else {
            return;
        };
        let label = job.label.clone();
        let wall_ms = job.elapsed().as_secs_f64() * 1000.0;
        self.tuning.shader.job = None;
        let checked = match outcome {
            Outcome::Done {
                value: (source, issues, ms),
                stop: None,
            } => Checked {
                source,
                issues,
                elapsed_ms: ms,
                state: "finished",
            },
            Outcome::Done {
                stop: Some(stop), ..
            }
            | Outcome::Abandoned { stop } => Checked {
                source: String::new(),
                issues: vec![],
                elapsed_ms: wall_ms,
                state: stop.name(),
            },
        };
        self.last_job = Some(
            json!({"kind":"shader_check","label":label,"state":checked.state,"issues":checked.issues.len(),"elapsed_ms":(wall_ms * 100.0).round() / 100.0}),
        );
        match (checked.state, checked.issues.first()) {
            ("finished", None) => {
                self.notice(format!("Shader {label}: OK · {:.1} ms", checked.elapsed_ms))
            }
            ("finished", Some(issue)) => self.set_error(format!(
                "Shader {label}: {} · {:.1} ms",
                issue.message, checked.elapsed_ms
            )),
            (state, _) => self.set_error(format!("Shader check {state} after {wall_ms:.0} ms")),
        }
        self.tuning.shader.checked = Some(checked);
    }

    /// Loads `id`'s authored source into the draft when another shader is open, or when the
    /// authored source changed and the draft had no edits.
    fn sync_shader_draft(&mut self, id: &str) {
        let authored = self
            .world
            .resources
            .get(ops::SHADER_LIBRARY)
            .and_then(|l| l.pointer(&format!("/shaders/{}/wgsl", axiom_core::local::escape(id))))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let s = &mut self.tuning.shader;
        if s.shader.as_deref() != Some(id) {
            s.shader = Some(id.to_owned());
            s.base = authored.to_owned();
            s.draft = authored.to_owned();
            s.checked = None;
        } else if s.base != authored && s.draft == s.base {
            s.base = authored.to_owned();
            s.draft = authored.to_owned();
        } else if s.base != authored && s.draft == authored {
            // Our Apply landed.
            s.base = authored.to_owned();
        }
    }

    /// The Shader source card under the material's parameters.
    pub(super) fn shader_section(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let Some(id) = self.shader_under_edit() else {
            if let Some(shader) = self
                .material
                .as_ref()
                .and_then(|m| m.value["shader"].as_str())
            {
                ui.label(
                    RichText::new(format!("Shader {shader} is built in (no editable source)."))
                        .small()
                        .color(theme::MUTED),
                );
            }
            return;
        };
        self.sync_shader_draft(&id);
        let icons = self.icons;
        let editable = !self.previewing;
        let authored_changed = {
            let s = &self.tuning.shader;
            s.draft != s.base
                && self
                    .world
                    .resources
                    .get(ops::SHADER_LIBRARY)
                    .and_then(|l| {
                        l.pointer(&format!("/shaders/{}/wgsl", axiom_core::local::escape(&id)))
                    })
                    .and_then(Value::as_str)
                    != Some(s.base.as_str())
        };
        ui.add_space(6.0);
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                icons.show(ui, Icon::Script, 16.0, theme::ACCENT);
                ui.label(RichText::new(format!("Shader source · {id}")).strong().color(theme::TEXT));
                let s = &self.tuning.shader;
                match &s.checked {
                    Some(c) if c.state == "finished" && c.source == s.draft => {
                        if c.issues.is_empty() {
                            theme::chip(ui, format!("valid · {:.1} ms", c.elapsed_ms), theme::SUCCESS);
                        } else {
                            let chip = theme::chip(ui, format!("{} error", c.issues.len()), theme::ERROR);
                            mark!(self.rects, "shader:errors".into(), chip.rect);
                        }
                    }
                    Some(_) => {
                        theme::chip(ui, "not checked since edit", theme::MUTED);
                    }
                    None => {}
                }
                if s.draft != s.base {
                    theme::chip(ui, "edited", theme::WARNING);
                }
            });
            if authored_changed {
                ui.label(
                    RichText::new("The shader changed in another revision; Apply would replace it, Revert loads it.")
                        .small()
                        .color(theme::WARNING),
                );
            }
            ui.horizontal(|ui| {
                let running = self.tuning.shader.job.is_some();
                let check = ui
                    .add_enabled(!running, egui::Button::new("Check"))
                    .on_hover_text("Validate the source as the engine does (naga), in the background");
                mark!(self.rects, "shader:check".into(), check.rect);
                if check.clicked() {
                    self.run_into("shader.check", actions);
                }
                if let Some(job) = &self.tuning.shader.job {
                    let (done, total) = job.progress();
                    ui.add(egui::ProgressBar::new(if total == 0 { 0.0 } else { done as f32 / total as f32 }).desired_width(60.0));
                    if ui.small_button("Cancel").clicked() {
                        self.run_into("shader.cancel_check", actions);
                    }
                }
                let dirty = self.tuning.shader.draft != self.tuning.shader.base;
                let apply = ui.add_enabled(editable && dirty, egui::Button::new("Apply source"));
                mark!(self.rects, "shader:apply".into(), apply.rect);
                if apply.clicked() {
                    let draft = self.tuning.shader.draft.clone();
                    actions.extend(self.apply_shader_source(&id, &draft));
                }
                let revert = ui.add_enabled(dirty || authored_changed, egui::Button::new("Revert"));
                if revert.clicked() {
                    self.tuning.shader.shader = None;
                    self.sync_shader_draft(&id);
                }
            });
            // Error list (click marks nothing extra: the gutter shows the line).
            let issues: Vec<Issue> = self
                .tuning
                .shader
                .checked
                .as_ref()
                .filter(|c| c.source == self.tuning.shader.draft)
                .map(|c| c.issues.clone())
                .unwrap_or_default();
            for issue in &issues {
                ui.horizontal_wrapped(|ui| {
                    icons.show(ui, Icon::Error, 14.0, theme::ERROR);
                    let r = ui.label(RichText::new(&issue.message).monospace().size(11.5).color(theme::ERROR));
                    mark!(self.rects, "shader:issue".into(), r.rect);
                });
            }
            // Code with a line gutter; error lines are marked in the gutter.
            let font = FontId::monospace(CODE_SIZE);
            let row = ui.fonts(|f| f.row_height(&font));
            let lines = self.tuning.shader.draft.lines().count().max(1)
                + usize::from(self.tuning.shader.draft.ends_with('\n'));
            if self.tuning.shader.gutter.0 != lines {
                self.tuning.shader.gutter = (lines, (1..=lines).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"));
            }
            egui::ScrollArea::both()
                .id_salt("shader_code")
                .max_height(320.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let gutter = ui.add(egui::Label::new(
                            RichText::new(&self.tuning.shader.gutter.1).font(font.clone()).color(theme::MUTED),
                        ));
                        for issue in &issues {
                            if let Some(line) = issue.line {
                                // TextEdit's frame insets its text by its margin (4 pt).
                                let top = gutter.rect.top() + (line - 1) as f32 * row;
                                let r = Rect::from_min_size(pos2(gutter.rect.left() - 2.0, top), vec2(gutter.rect.width() + 4.0, row));
                                ui.painter().rect_filled(r, 2.0, theme::ERROR.gamma_multiply(0.45));
                                mark!(self.rects, "shader:error_line".into(), r);
                            }
                        }
                        let edit = ui.add_enabled(
                            editable,
                            egui::TextEdit::multiline(&mut self.tuning.shader.draft)
                                .id_salt("shader_source")
                                .font(font.clone())
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .margin(egui::Margin::ZERO)
                                .frame(false),
                        );
                        mark!(self.rects, "shader:source".into(), edit.rect);
                    });
                });
        });
    }
}
