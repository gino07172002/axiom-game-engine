//! Localization tab: the `ui.document` catalogs as a keys × locales grid. A cell commits once
//! on Enter or when it loses focus (`ACL: loc set LOCALE KEY TEXT`); Esc cancels. Empty texts
//! (or the key itself) are flagged missing and texts equal to the default locale's in another
//! locale untranslated, with per-locale counts; a filter (key or text) and "missing only" cut
//! the rows. Rows are a virtual list over a model cached per revision.
use super::icons::Icon;
use super::tuning::mark;
use super::{theme, Action, Editor};
use axiom_core::ops::{self, LocEdit, LocIssue};
use egui::{Color32, RichText, Ui};
use serde_json::{json, Value};

const ROW: f32 = 24.0;
const KEY_COLUMN: f32 = 190.0;

#[derive(Default)]
pub(crate) struct LocUi {
    pub filter: String,
    pub missing_only: bool,
    /// Cell being edited: (key, locale, text, focus requested yet).
    pub editing: Option<(String, String, String, bool)>,
    pub new_key: String,
    pub new_locale: String,
    model: Option<Model>,
    rows: Option<Rows>,
    /// Times the model / the filtered rows were rebuilt (tests: not per frame).
    pub builds: usize,
    pub filters: usize,
}

pub(crate) struct Model {
    key: (i64, u64),
    /// Default locale first.
    pub locales: Vec<String>,
    pub keys: Vec<String>,
    /// `texts[key][locale]`.
    pub texts: Vec<Vec<String>>,
    pub issues: Vec<Vec<Option<LocIssue>>>,
    /// Per locale: (missing, untranslated).
    pub counts: Vec<(usize, usize)>,
}

struct Rows {
    key: (i64, u64),
    filter: String,
    missing_only: bool,
    rows: Vec<u32>,
}

impl LocUi {
    pub fn observation(&self) -> Value {
        let counts = self.model.as_ref().map(|m| {
            m.locales
                .iter()
                .zip(&m.counts)
                .map(|(l, (missing, untranslated))| {
                    (
                        l.clone(),
                        json!({"missing":missing,"untranslated":untranslated}),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
        });
        json!({
            "filter":self.filter,
            "missing_only":self.missing_only,
            "rows":self.rows.as_ref().map(|r| r.rows.len()),
            "locales":counts,
            "editing":self.editing.as_ref().map(|(k, l, ..)| format!("{l}/{k}")),
        })
    }
}

impl Model {
    fn build(key: (i64, u64), world: &axiom_core::World) -> Option<Self> {
        let doc: axiom_core::gameplay::UiDocument =
            serde_json::from_value(world.resources.get(ops::UI_DOCUMENT)?.clone()).ok()?;
        let mut locales: Vec<String> = vec![doc.locale.clone()];
        locales.extend(doc.catalogs.keys().filter(|l| **l != doc.locale).cloned());
        let default = doc.catalogs.get(&doc.locale)?;
        let keys: Vec<String> = default.keys().cloned().collect();
        let mut texts = Vec::with_capacity(keys.len());
        let mut issues = Vec::with_capacity(keys.len());
        let mut counts = vec![(0, 0); locales.len()];
        for k in &keys {
            let base = default.get(k).map(String::as_str);
            let mut row = Vec::with_capacity(locales.len());
            let mut row_issues = Vec::with_capacity(locales.len());
            for (i, l) in locales.iter().enumerate() {
                let text = doc
                    .catalogs
                    .get(l)
                    .and_then(|c| c.get(k))
                    .cloned()
                    .unwrap_or_default();
                let issue = ops::loc_issue(k, &text, base, i == 0);
                match issue {
                    Some(LocIssue::Missing) => counts[i].0 += 1,
                    Some(LocIssue::Untranslated) => counts[i].1 += 1,
                    None => {}
                }
                row.push(text);
                row_issues.push(issue);
            }
            texts.push(row);
            issues.push(row_issues);
        }
        Some(Self {
            key,
            locales,
            keys,
            texts,
            issues,
            counts,
        })
    }
}

enum Intent {
    Edit(LocEdit),
    Begin(String, String, String),
    Cancel,
}

impl Editor {
    /// One catalog edit (`ACL: loc set|key|locale ..`).
    pub(super) fn loc_edit(&mut self, edit: &LocEdit) -> Option<Action> {
        let result = ops::edit_localization(&self.world, edit);
        self.tuning_write(result)
    }

    /// The Localization tab body.
    pub(super) fn localization_tab(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let key = (self.rev, self.world_version);
        let st = &mut self.tuning.loc;
        if st.model.as_ref().is_none_or(|m| m.key != key) {
            st.model = Model::build(key, &self.world);
            st.builds += 1;
            // A cell being edited stays open across revisions (its key/locale are names).
        }
        let stale_rows = st.rows.as_ref().is_none_or(|r| {
            r.key != key || r.filter != st.filter || r.missing_only != st.missing_only
        });
        if stale_rows {
            if let Some(m) = &st.model {
                let needle = st.filter.trim().to_lowercase();
                let rows = (0..m.keys.len())
                    .filter(|&i| {
                        (!st.missing_only || m.issues[i].iter().any(Option::is_some))
                            && (needle.is_empty()
                                || m.keys[i].to_lowercase().contains(&needle)
                                || m.texts[i]
                                    .iter()
                                    .any(|t| t.to_lowercase().contains(&needle)))
                    })
                    .map(|i| i as u32)
                    .collect();
                st.rows = Some(Rows {
                    key,
                    filter: st.filter.clone(),
                    missing_only: st.missing_only,
                    rows,
                });
                st.filters += 1;
            }
        }
        let Some(model) = st.model.take() else {
            ui.label(
                RichText::new("This world has no ui.document, so there is nothing to translate.")
                    .color(theme::MUTED),
            );
            if ui
                .add_enabled(!self.previewing, egui::Button::new("Create UI Document"))
                .clicked()
            {
                match self.run_command("asset.new_ui_document") {
                    Ok(more) => actions.extend(more),
                    Err(e) => self.set_error(format!("{e:#}")),
                }
            }
            return;
        };
        // Kept as built: a filter typed this frame no longer matches `Rows::filter`, so the
        // rows rebuild next frame.
        let rows = st.rows.take();
        let mut intents = vec![];
        self.localization_body(
            ui,
            &model,
            rows.as_ref().map_or(&[][..], |r| &r.rows),
            &mut intents,
        );
        let st = &mut self.tuning.loc;
        st.rows = rows;
        st.model = Some(model);
        for intent in intents {
            match intent {
                Intent::Begin(k, l, text) => self.tuning.loc.editing = Some((k, l, text, false)),
                Intent::Cancel => self.tuning.loc.editing = None,
                Intent::Edit(edit) => {
                    if matches!(edit, LocEdit::Set { .. }) {
                        self.tuning.loc.editing = None;
                    }
                    actions.extend(self.loc_edit(&edit));
                }
            }
        }
    }

    fn localization_body(
        &mut self,
        ui: &mut Ui,
        m: &Model,
        rows: &[u32],
        intents: &mut Vec<Intent>,
    ) {
        let icons = self.icons;
        let editable = !self.previewing;
        // Toolbar: filter, missing only, add key / locale.
        ui.horizontal(|ui| {
            icons.show(ui, Icon::Localization, 16.0, theme::ACCENT);
            ui.label(
                RichText::new(format!(
                    "{} keys · {} locales",
                    m.keys.len(),
                    m.locales.len()
                ))
                .color(theme::TEXT),
            );
            theme::divider(ui);
            icons.show(ui, Icon::Search, 14.0, theme::MUTED);
            let filter = ui.add(
                egui::TextEdit::singleline(&mut self.tuning.loc.filter)
                    .id_salt("loc_filter")
                    .hint_text("Filter keys or text…")
                    .desired_width(170.0),
            );
            mark!(self.rects, "loc:filter".into(), filter.rect);
            let missing = ui.checkbox(&mut self.tuning.loc.missing_only, "Missing only");
            mark!(self.rects, "loc:missing_only".into(), missing.rect);
            ui.label(
                RichText::new(format!("{} shown", rows.len()))
                    .size(11.0)
                    .color(theme::MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(editable, |ui| {
                    let locale = self.tuning.loc.new_locale.trim().to_owned();
                    let add_locale =
                        ui.add_enabled(!locale.is_empty(), egui::Button::new("+ Locale"));
                    mark!(self.rects, "loc:add_locale".into(), add_locale.rect);
                    if add_locale.clicked() {
                        intents.push(Intent::Edit(LocEdit::AddLocale { locale, from: None }));
                        self.tuning.loc.new_locale.clear();
                    }
                    let f = ui.add(
                        egui::TextEdit::singleline(&mut self.tuning.loc.new_locale)
                            .id_salt("loc_new_locale")
                            .hint_text("ja")
                            .desired_width(56.0),
                    );
                    mark!(self.rects, "loc:new_locale".into(), f.rect);
                    theme::divider(ui);
                    let key = self.tuning.loc.new_key.trim().to_owned();
                    let add_key = ui.add_enabled(!key.is_empty(), egui::Button::new("+ Key"));
                    mark!(self.rects, "loc:add_key".into(), add_key.rect);
                    if add_key.clicked() {
                        intents.push(Intent::Edit(LocEdit::AddKey {
                            key,
                            text: String::new(),
                        }));
                        self.tuning.loc.new_key.clear();
                    }
                    let f = ui.add(
                        egui::TextEdit::singleline(&mut self.tuning.loc.new_key)
                            .id_salt("loc_new_key")
                            .hint_text("new key…")
                            .desired_width(120.0),
                    );
                    mark!(self.rects, "loc:new_key".into(), f.rect);
                });
            });
        });
        // Column widths: key column + the locales sharing the rest (with a floor; the grid
        // scrolls sideways when there are many locales).
        let n = m.locales.len().max(1) as f32;
        let column = ((ui.available_width() - KEY_COLUMN - 40.0) / n).max(170.0);
        // Header: locale name, default marker, missing / untranslated counts, remove.
        ui.horizontal(|ui| {
            ui.add_sized(
                [KEY_COLUMN, 22.0],
                egui::Label::new(RichText::new("KEY").size(11.0).color(theme::MUTED)),
            );
            for (i, l) in m.locales.iter().enumerate() {
                ui.allocate_ui(egui::vec2(column, 22.0), |ui| {
                    ui.set_width(column);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(l).strong().color(theme::TEXT));
                        if i == 0 {
                            theme::chip(ui, "default", theme::MUTED);
                        }
                        let (missing, untranslated) = m.counts[i];
                        if missing > 0 {
                            let c = theme::chip(ui, format!("{missing} missing"), theme::ERROR);
                            mark!(self.rects, format!("loc:missing:{l}"), c.rect);
                        }
                        if untranslated > 0 {
                            theme::chip(ui, format!("{untranslated} untranslated"), theme::WARNING);
                        }
                        if i > 0 && editable {
                            let x = theme::icon_button_sized(
                                ui,
                                icons,
                                Icon::Close,
                                "Remove this locale",
                                false,
                                18.0,
                            );
                            mark!(self.rects, format!("loc:remove_locale:{l}"), x.rect);
                            if x.clicked() {
                                intents.push(Intent::Edit(LocEdit::RemoveLocale {
                                    locale: l.clone(),
                                }));
                            }
                        }
                    });
                });
            }
        });
        let editing = self.tuning.loc.editing.take();
        let mut keep_editing = editing.clone();
        egui::ScrollArea::both()
            .id_salt("loc_grid")
            .auto_shrink([false, false])
            .show_rows(ui, ROW, rows.len(), |ui, range| {
                for &r in &rows[range] {
                    let r = r as usize;
                    let key = &m.keys[r];
                    ui.horizontal(|ui| {
                        ui.set_height(ROW);
                        let label = ui.add_sized(
                            [KEY_COLUMN - 24.0, ROW],
                            egui::Label::new(
                                RichText::new(key).monospace().size(12.0).color(theme::TEXT),
                            )
                            .truncate(),
                        );
                        label.on_hover_text(key);
                        let x = ui
                            .add_enabled_ui(editable, |ui| {
                                theme::icon_button_sized(
                                    ui,
                                    icons,
                                    Icon::Delete,
                                    "Remove this key from every locale",
                                    false,
                                    18.0,
                                )
                            })
                            .inner;
                        if x.clicked() {
                            intents.push(Intent::Edit(LocEdit::RemoveKey { key: key.clone() }));
                        }
                        for (i, l) in m.locales.iter().enumerate() {
                            let text = &m.texts[r][i];
                            let here = editing
                                .as_ref()
                                .is_some_and(|(k, el, ..)| k == key && el == l);
                            if here {
                                let (_, _, draft, focused) =
                                    keep_editing.as_mut().expect("editing this cell");
                                let edit = ui.add_sized(
                                    [column - 6.0, ROW - 2.0],
                                    egui::TextEdit::singleline(draft).id_salt(("loc_cell", key, l)),
                                );
                                mark!(self.rects, format!("loc:cell:{l}:{key}"), edit.rect);
                                if !*focused {
                                    edit.request_focus();
                                    *focused = true;
                                } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                    intents.push(Intent::Cancel);
                                } else if edit.lost_focus() {
                                    // Enter or a click elsewhere: one revision (none if unchanged).
                                    intents.push(Intent::Edit(LocEdit::Set {
                                        locale: l.clone(),
                                        key: key.clone(),
                                        text: draft.clone(),
                                    }));
                                }
                                continue;
                            }
                            let (fill, color) = match m.issues[r][i] {
                                Some(LocIssue::Missing) => {
                                    (theme::ERROR.gamma_multiply(0.18), theme::MUTED)
                                }
                                Some(LocIssue::Untranslated) => {
                                    (theme::WARNING.gamma_multiply(0.14), theme::TEXT)
                                }
                                None => (Color32::TRANSPARENT, theme::TEXT),
                            };
                            let shown = if text.is_empty() {
                                "(missing)"
                            } else {
                                text.as_str()
                            };
                            let cell = ui.add_sized(
                                [column - 6.0, ROW - 2.0],
                                egui::Button::new(RichText::new(shown).size(12.5).color(color))
                                    .fill(fill)
                                    .truncate(),
                            );
                            mark!(self.rects, format!("loc:cell:{l}:{key}"), cell.rect);
                            let cell = match m.issues[r][i] {
                                Some(LocIssue::Missing) => {
                                    cell.on_hover_text("Missing: empty or still the key")
                                }
                                Some(LocIssue::Untranslated) => {
                                    cell.on_hover_text(format!("Same text as {}", m.locales[0]))
                                }
                                None => cell,
                            };
                            if cell.clicked() && editable {
                                intents.push(Intent::Begin(key.clone(), l.clone(), text.clone()));
                            }
                        }
                    });
                }
            });
        // A click on another cell replaces the open one (its edit commits first, above).
        if !intents
            .iter()
            .any(|i| matches!(i, Intent::Cancel | Intent::Edit(LocEdit::Set { .. })))
        {
            self.tuning.loc.editing = keep_editing;
        }
    }
}
