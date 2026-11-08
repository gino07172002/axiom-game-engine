mod assets;
mod commands;
mod form;
pub use commands::register_topics;
mod resource;
mod ui;
mod viewport;

use crate::world::WorldRuntime;
#[cfg(test)]
mod hierarchy_tests;
#[cfg(test)]
mod parity;
use axiom_core::{local::Target, ops, CoreEdit, Edit, Request, Transform, World};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use viewport::{Drag, Orbit, Tool};

pub(crate) enum Action {
    Write {
        request: Request,
        select: Option<String>,
    },
    Restore {
        rev: i64,
        redo: bool,
    },
    Play,
    /// Load the before/after diff of one revision against its predecessor.
    LoadDiff {
        rev: i64,
    },
    /// Append a revision restoring `rev` (timeline restore, independent of the undo stack).
    RestoreTo {
        rev: i64,
    },
    Verify,
    ResolveProposal {
        id: i64,
        approve: bool,
        rebase: bool,
    },
    /// Run a scenario file headless against the current revision.
    RunScenario {
        path: String,
    },
    /// Close the editor window (File ▸ Exit).
    Quit,
}

/// Entity ID referenced by a diagnostic path, if any (`/entities/@a~1b/...` or
/// `entities/a/b/components...`).
pub(crate) fn entity_in_path(path: &str, world: &World) -> Option<String> {
    let id = if let Some(rest) = path.strip_prefix("/entities/@") {
        rest.split('/')
            .next()?
            .replace("~1", "/")
            .replace("~0", "~")
    } else {
        path.strip_prefix("entities/")?
            .split("/components")
            .next()?
            .to_owned()
    };
    world.entity(&id).map(|e| e.id.clone())
}

/// Every action drawn in one frame was built against the same revision. Adjacent applies
/// therefore merge into one atomic batch; otherwise the second would always fail CAS.
pub(crate) fn coalesce(actions: Vec<Action>) -> Vec<Action> {
    let mut out: Vec<Action> = Vec::with_capacity(actions.len());
    for action in actions {
        if let (
            Some(Action::Write {
                request:
                    Request::Apply {
                        expected_rev: previous_rev,
                        edits: previous_edits,
                    },
                select: previous_select,
            }),
            Action::Write {
                request:
                    Request::Apply {
                        expected_rev,
                        edits,
                    },
                select,
            },
        ) = (out.last_mut(), &action)
        {
            if *previous_rev == *expected_rev {
                previous_edits.extend(edits.iter().cloned());
                if select.is_some() {
                    *previous_select = select.clone();
                }
                continue;
            }
        }
        out.push(action);
    }
    out
}

struct Draft {
    rev: i64,
    id: String,
    label: String,
    base_transform: Option<Transform>,
    transform: Option<Transform>,
    angles: [f32; 3],
    parent: String,
    keep_world: bool,
    component: String,
    component_input: String,
    json: String,
    label_dirty: bool,
    transform_dirty: bool,
    parent_dirty: bool,
    component_dirty: bool,
    /// Effective (inherited + authored) value being edited in form mode, and its baseline.
    form: Option<Value>,
    form_base: Value,
    form_dirty: bool,
    json_mode: bool,
}

impl Draft {
    fn dirty(&self) -> bool {
        self.label_dirty
            || self.transform_dirty
            || self.parent_dirty
            || self.component_dirty
            || self.form_dirty
    }

    /// Starts editing `component`: the form shows the effective value, the JSON box the
    /// authored override only (so raw JSON edits never flatten prefab inheritance).
    fn open_component(&mut self, world: &World, entity: &axiom_core::Entity, component: &str) {
        self.component = component.to_owned();
        self.component_input = component.to_owned();
        self.json =
            serde_json::to_string_pretty(entity.components.get(component).unwrap_or(&json!({})))
                .unwrap();
        let effective = crate::editor::ui::component_value(world, entity, component);
        self.form_base = effective.clone().unwrap_or(Value::Null);
        self.form = effective.filter(|v| v.is_object());
        self.json_mode = self.form.is_none();
        self.form_dirty = false;
        self.component_dirty = false;
    }

    fn form_edits(&self) -> Vec<Edit> {
        let Some(form) = self.form.as_ref().filter(|_| self.form_dirty) else {
            return vec![];
        };
        let mut leaves = vec![];
        form::changes(&self.form_base, form, "", &mut leaves);
        let component = self.component.replace('~', "~0").replace('/', "~1");
        leaves
            .into_iter()
            .map(|(pointer, value)| {
                Edit::Core(CoreEdit::SetField {
                    target: self.target(),
                    field: format!("/components/{component}{pointer}"),
                    value,
                })
            })
            .collect()
    }

    fn target(&self) -> Target {
        Target::Entity {
            id: self.id.clone(),
        }
    }

    fn transform_edits(&self) -> Vec<Edit> {
        let Some(next) = &self.transform else {
            return vec![];
        };
        let edit = |suffix: &str, value| {
            Edit::Core(CoreEdit::SetField {
                target: self.target(),
                field: format!("/components/core.transform{suffix}"),
                value,
            })
        };
        let Some(before) = &self.base_transform else {
            return vec![edit("", json!(next))];
        };
        [
            (
                before.position != next.position,
                "/position",
                json!(next.position),
            ),
            (
                before.rotation != next.rotation,
                "/rotation",
                json!(next.rotation),
            ),
            (before.scale != next.scale, "/scale", json!(next.scale)),
        ]
        .into_iter()
        .filter(|(changed, _, _)| *changed)
        .map(|(_, field, value)| edit(field, value))
        .collect()
    }

    fn edits(&self) -> anyhow::Result<Vec<Edit>> {
        let mut edits = Vec::new();
        if self.label_dirty {
            let label = self.label.trim();
            edits.push(Edit::Core(CoreEdit::SetField {
                target: self.target(),
                field: "/label".into(),
                value: if label.is_empty() {
                    Value::Null
                } else {
                    json!(label)
                },
            }));
        }
        if self.transform_dirty {
            edits.extend(self.transform_edits());
        }
        if self.parent_dirty {
            let parent = self.parent.trim();
            edits.push(Edit::Core(CoreEdit::Reparent {
                entity: self.id.clone(),
                parent: (!parent.is_empty()).then(|| parent.to_owned()),
                keep_world: self.keep_world,
            }));
        }
        edits.extend(self.form_edits());
        if self.component_dirty {
            let value: Value = serde_json::from_str(&self.json)?;
            edits.push(Edit::Core(CoreEdit::SetComponent {
                entity: self.id.clone(),
                component: self.component.clone(),
                value,
            }));
        }
        Ok(edits)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Dock {
    Changes,
    Proposals,
    History,
    Problems,
    Scenarios,
    Graphs,
    Assets,
    Agent,
}

pub(crate) struct Editor {
    pub world: World,
    pub rev: i64,
    pub previewing: bool,
    selected: Option<String>,
    draft: Option<Draft>,
    resource: Option<resource::ResourceDraft>,
    orbit: Orbit,
    tool: Tool,
    drag: Option<Drag>,
    viewport: egui::Rect,
    search: String,
    scene: Option<String>,
    collapsed: BTreeSet<String>,
    new_scene: String,
    dock: Dock,
    status: String,
    error: bool,
    changes: Vec<String>,
    last_request: Option<Request>,
    undo: Vec<i64>,
    redo: Vec<i64>,
    frame_selection: bool,
    scroll_selection: bool,
    logo: Option<egui::TextureHandle>,
    /// Registry component descriptors, used as form hints.
    component_hints: std::collections::BTreeMap<String, Value>,
    /// Newest-first revision rows `{rev, reason, created_at}`; refreshed on revision change.
    history: Vec<Value>,
    pub(crate) needs_history: bool,
    history_diff: Option<(i64, Value)>,
    /// Recent structured diagnostics `{code, path, message}`, newest first.
    problems: Vec<Value>,
    /// Additional selected objects (Ctrl/Shift click); `selected` stays the inspector target.
    extra: BTreeSet<String>,
    clipboard: Vec<String>,
    /// Per-user preferences (snap steps and toggles, grid, axes, dock, shortcut overrides).
    prefs: commands::Prefs,
    prefs_path: Option<std::path::PathBuf>,
    prefs_saved: Option<String>,
    palette: Option<commands::Palette>,
    show_shortcuts: bool,
    show_prefs: bool,
    /// Text to place on the system clipboard on the next drawn frame.
    pending_copy: Option<String>,
    /// Status-bar readouts supplied by the host window.
    pub(crate) fps: f32,
    pub(crate) bridge: Option<u16>,
    /// Pending AI proposals awaiting review, newest first, and the store signature they match.
    proposals: Vec<Value>,
    pub(crate) proposal_signature: Option<(i64, i64)>,
    proposal_focus: Option<i64>,
    /// (request bytes, response bytes) of the last authoring request. Bytes, not tokens.
    last_bytes: Option<(usize, usize)>,
    /// Play-preview overlay of live behavior state above agents.
    pub(crate) overlay: bool,
    /// Scenario files offered in the Scenarios tab, and the last run per file.
    scenarios: Vec<String>,
    scenario_results: std::collections::BTreeMap<String, Value>,
    material: Option<assets::MaterialDraft>,
    prefab: Option<assets::PrefabDraft>,
    /// Inline hierarchy rename in progress (F2 / context menu).
    renaming: Option<Rename>,
    /// Screen rects of hierarchy rows and context-menu items drawn last frame (tests, focus).
    pub(crate) rects: std::collections::HashMap<String, egui::Rect>,
}

/// Inline rename state: the row being renamed, the text being typed, focus requested yet.
pub(crate) struct Rename {
    pub id: String,
    pub text: String,
    pub focused: bool,
}

impl Editor {
    pub fn new(world: World, rev: i64, selected: Option<String>) -> Self {
        let mut editor = Self {
            world,
            rev,
            previewing: false,
            selected: None,
            draft: None,
            resource: None,
            orbit: Orbit::default(),
            tool: Tool::Move,
            drag: None,
            viewport: egui::Rect::NOTHING,
            search: String::new(),
            scene: None,
            collapsed: BTreeSet::new(),
            new_scene: String::new(),
            dock: Dock::Changes,
            status: "Ready".into(),
            error: false,
            changes: Vec::new(),
            last_request: None,
            undo: Vec::new(),
            redo: Vec::new(),
            frame_selection: false,
            scroll_selection: false,
            logo: None,
            component_hints: Default::default(),
            history: Vec::new(),
            needs_history: true,
            history_diff: None,
            problems: Vec::new(),
            extra: BTreeSet::new(),
            clipboard: Vec::new(),
            prefs: Default::default(),
            prefs_path: None,
            prefs_saved: None,
            palette: None,
            show_shortcuts: false,
            show_prefs: false,
            pending_copy: None,
            fps: 0.0,
            bridge: None,
            proposals: Vec::new(),
            proposal_signature: None,
            proposal_focus: None,
            last_bytes: None,
            overlay: true,
            scenarios: Vec::new(),
            scenario_results: Default::default(),
            material: None,
            prefab: None,
            renaming: None,
            rects: Default::default(),
        };
        if selected
            .as_ref()
            .is_some_and(|id| editor.world.resources.contains_key(id))
        {
            editor.open_resource(selected.as_deref().unwrap());
        } else {
            editor.select(selected);
        }
        editor
    }

    pub fn update(&mut self, rev: i64, world: World, local: bool) {
        if self.rev == rev {
            return;
        }
        if !local || self.changes.is_empty() {
            self.record_changes(&axiom_core::diff::summary(&self.world, &world));
        }
        self.rev = rev;
        self.world = world;
        self.needs_history = true;
        let world = &self.world;
        self.extra.retain(|id| world.entity(id).is_some());
        if self
            .renaming
            .as_ref()
            .is_some_and(|r| world.entity(&r.id).is_none())
        {
            self.renaming = None;
        }
        // A drag only starts from a clean draft, so its dirtiness is transient gizmo state.
        // Dropping the drag without resetting would strand an uncommittable stale draft.
        let interrupted = self.drag.take().is_some();
        if interrupted {
            self.select(self.selected.clone());
        }
        if !local {
            self.undo.clear();
            self.redo.clear();
            if interrupted {
                self.set_error(format!(
                    "Drag cancelled: revision {rev} arrived from another writer. Drag again."
                ));
            } else if self.dirty() {
                self.set_error("World changed outside this window. Copy your draft or reload it before applying.");
            } else {
                self.status = format!("Synchronized external revision {rev}");
                self.error = false;
            }
        }
        if self
            .scene
            .as_ref()
            .is_some_and(|id| !self.world.scenes.iter().any(|s| &s.id == id))
        {
            self.scene = None;
        }
        if local || !self.dirty() {
            self.reset_draft();
        }
    }

    /// Primary plus extra selections, primary first.
    pub(crate) fn selection(&self) -> Vec<String> {
        self.selected
            .iter()
            .cloned()
            .chain(
                self.extra
                    .iter()
                    .filter(|id| Some(*id) != self.selected.as_ref())
                    .cloned(),
            )
            .collect()
    }

    /// Ctrl toggles, Shift adds, a plain click replaces the selection.
    pub(crate) fn click_select(&mut self, id: Option<String>, modifiers: egui::Modifiers) {
        match id {
            Some(id) if (modifiers.command || modifiers.shift) && self.selected.is_some() => {
                if self.selected.as_ref() == Some(&id) {
                    return;
                }
                if modifiers.command && self.extra.contains(&id) {
                    self.extra.remove(&id);
                } else {
                    self.extra.insert(id);
                }
            }
            other => {
                self.extra.clear();
                self.try_select(other);
            }
        }
    }

    // Content-changing editor features compile through `axiom_core::ops`, the same code the
    // headless ACL verbs use, so a human in the editor and an AI without it get identical
    // edit batches. Failures become structured problems instead of writes.

    /// Turns a shared operation into a write that selects `select`, or records its error.
    fn op_write(
        &mut self,
        result: anyhow::Result<Vec<Edit>>,
        select: Option<String>,
    ) -> Option<Action> {
        match result {
            Ok(edits) => Some(self.write(edits, select)),
            Err(error) => {
                self.problem(&error);
                None
            }
        }
    }

    /// Writes a creation op and selects the first created object.
    fn created(&mut self, result: anyhow::Result<ops::Created>) -> Option<Action> {
        match result {
            Ok(created) => {
                let first = created.ids.first().cloned();
                Some(self.write(created.edits, first))
            }
            Err(error) => {
                self.problem(&error);
                None
            }
        }
    }

    /// New objects land in the scene the hierarchy is filtered to, if any.
    fn placement(&self) -> ops::Placement {
        ops::Placement {
            scene: self.scene.clone(),
            ..Default::default()
        }
    }

    /// "+ Object" presets (`ACL: create PRESET [scene=..]`).
    pub(super) fn create_object(&mut self, preset: &str) -> Option<Action> {
        let result = ops::create_object(&self.world, preset, &self.placement());
        self.created(result)
    }

    /// Assets ▸ prefab button (`ACL: instantiate PREFAB [scene=..]`).
    pub(super) fn instantiate_prefab(&mut self, prefab: &str) -> Option<Action> {
        let result = ops::instantiate_prefab(&self.world, prefab, &self.placement());
        self.created(result)
    }

    fn delete_selection(&mut self) -> Option<Action> {
        self.delete_ids(&self.selection())
    }

    fn delete_ids(&mut self, ids: &[String]) -> Option<Action> {
        if ids.is_empty() {
            return None;
        }
        let result = ops::delete(&self.world, ids);
        self.op_write(result, None)
    }

    /// Duplicates the existing IDs among `ids` (a pasted clipboard may hold stale ones).
    fn duplicate_ids(&mut self, ids: &[String]) -> Option<Action> {
        let ids: Vec<String> = ids
            .iter()
            .filter(|id| self.world.entity(id).is_some())
            .cloned()
            .collect();
        if ids.is_empty() {
            return None;
        }
        let result = ops::duplicate(&self.world, &ids);
        self.created(result)
    }

    /// Hierarchy drag-and-drop keeps the world pose (`ACL: reparent ID PARENT|root`).
    fn reparent_drop(&mut self, entity: &str, parent: Option<&str>) -> Option<Action> {
        let result = ops::reparent(&self.world, entity, parent, true);
        self.op_write(result, None)
    }

    pub(super) fn scene_create(&mut self, id: &str) -> Option<Action> {
        let result = ops::create_scene(&self.world, id, true);
        self.op_write(result, None)
    }

    pub(super) fn scene_set_active(&mut self, id: &str, active: bool) -> Option<Action> {
        let result = ops::set_scene_active(&self.world, id, active);
        self.op_write(result, None)
    }

    pub(super) fn scene_delete(&mut self, id: &str) -> Option<Action> {
        let result = ops::delete_scene(&self.world, id);
        self.op_write(result, None)
    }

    /// Object ▸ Create Child (`ACL: create PRESET parent=ID [scene=..]`).
    pub(super) fn create_child(&mut self, preset: &str) -> Option<Action> {
        let place = ops::Placement {
            parent: self.selected.clone(),
            ..self.placement()
        };
        let result = ops::create_object(&self.world, preset, &place);
        self.created(result)
    }

    /// Object ▸ Create Prefab From Selection (`ACL: prefab create from ID`).
    pub(super) fn create_prefab_from_selection(&mut self) -> Option<Action> {
        let id = self.selected.clone()?;
        let result = ops::create_prefab(&self.world, None, &id);
        if let Ok(created) = &result {
            self.notice(format!("Prefab {} created from {id}", created.ids[0]));
        }
        self.op_write(result.map(|c| c.edits), Some(id))
    }

    /// Starts the inline hierarchy rename of the primary selection (F2).
    pub(super) fn begin_rename(&mut self) {
        let Some(e) = self
            .selected
            .as_deref()
            .and_then(|id| self.world.entity(id))
        else {
            return;
        };
        self.renaming = Some(Rename {
            id: e.id.clone(),
            text: e.label.clone().unwrap_or_else(|| e.id.clone()),
            focused: false,
        });
        self.scroll_selection = true;
    }

    /// Commits an inline rename (`ACL: rename ID "LABEL"`); unchanged text writes nothing.
    pub(super) fn rename_entity(&mut self, id: &str, text: &str) -> Option<Action> {
        let entity = self.world.entity(id)?;
        let shown = entity.label.as_deref().unwrap_or(id);
        if text.trim() == shown {
            return None;
        }
        let result = ops::rename(&self.world, id, text);
        self.op_write(result, None)
    }

    /// Hierarchy drop between rows: reorder among siblings, reparenting first when the row
    /// came from another parent (`ACL: reparent ID P|root` + `move ID before|after OTHER`).
    pub(super) fn reorder_drop(
        &mut self,
        dragged: &str,
        target: &str,
        after: bool,
    ) -> Option<Action> {
        if dragged == target {
            return None;
        }
        let parent_of = |id: &str| {
            self.world
                .relations
                .iter()
                .find(|r| r.kind == "parent" && r.source == id)
                .map(|r| r.target.clone())
        };
        let parent = parent_of(target);
        let order = if after {
            ops::Order::After(target.to_owned())
        } else {
            ops::Order::Before(target.to_owned())
        };
        let mut edits = vec![];
        if parent_of(dragged) != parent {
            match ops::reparent(&self.world, dragged, parent.as_deref(), true) {
                Ok(more) => edits.extend(more),
                Err(e) => {
                    self.problem(&e);
                    return None;
                }
            }
        }
        match ops::move_entity(&self.world, dragged, &order) {
            Ok(more) => edits.extend(more),
            Err(e) => {
                self.problem(&e);
                return None;
            }
        }
        if edits.is_empty() {
            return None;
        }
        Some(self.write(edits, None))
    }

    /// Adds every descendant of the selection to the selection (view state only).
    pub(super) fn select_children(&mut self) {
        let graph = axiom_core::scene_edit::Hierarchy::new(&self.world);
        let selection = self.selection();
        let mut stack = selection.clone();
        let mut found = BTreeSet::new();
        while let Some(id) = stack.pop() {
            for child in graph.children.get(&Some(id.as_str())).into_iter().flatten() {
                if !selection.iter().any(|s| s == child) && found.insert(child.to_string()) {
                    stack.push(child.to_string());
                }
            }
        }
        let added = found.len();
        self.extra.extend(found);
        self.notice(format!("Selected {added} descendant(s)"));
    }

    /// Hidden (`locked == false`) or locked IDs set for this world in the editor prefs.
    pub(crate) fn view_flags(&self, locked: bool) -> Option<&BTreeSet<String>> {
        let sets = if locked {
            &self.prefs.locked
        } else {
            &self.prefs.hidden
        };
        sets.get(&self.world.id).filter(|s| !s.is_empty())
    }

    /// Objects hidden/locked in the editor view, directly or through an ancestor.
    pub(crate) fn view_flagged(&self, locked: bool) -> BTreeSet<String> {
        let Some(set) = self.view_flags(locked) else {
            return BTreeSet::new();
        };
        let parents: std::collections::HashMap<&str, &str> = self
            .world
            .relations
            .iter()
            .filter(|r| r.kind == "parent")
            .map(|r| (r.source.as_str(), r.target.as_str()))
            .collect();
        self.world
            .entities
            .iter()
            .filter(|e| {
                let mut cursor = Some(e.id.as_str());
                for _ in 0..64 {
                    let Some(id) = cursor else { break };
                    if set.contains(id) {
                        return true;
                    }
                    cursor = parents.get(id).copied();
                }
                false
            })
            .map(|e| e.id.clone())
            .collect()
    }

    /// Toggles the editor-view hidden or locked flag of `ids` (all on unless all already on).
    /// Never touches the world; persisted per world in the editor prefs.
    pub(crate) fn set_view_flag(&mut self, ids: &[String], locked: bool) {
        let world = self.world.id.clone();
        let sets = if locked {
            &mut self.prefs.locked
        } else {
            &mut self.prefs.hidden
        };
        let set = sets.entry(world.clone()).or_default();
        let on = !ids.iter().all(|id| set.contains(id));
        for id in ids {
            if on {
                set.insert(id.clone());
            } else {
                set.remove(id);
            }
        }
        if set.is_empty() {
            sets.remove(&world);
        }
        let what = match (locked, on) {
            (false, true) => "Hidden in the editor view",
            (false, false) => "Shown in the editor view",
            (true, true) => "Locked in the editor view",
            (true, false) => "Unlocked in the editor view",
        };
        self.notice(format!(
            "{what}: {} object(s); play and content unchanged",
            ids.len()
        ));
    }

    fn toggle_view_flag(&mut self, locked: bool) {
        let ids = self.selection();
        self.set_view_flag(&ids, locked);
    }

    /// Remote view control from the bridge (`editor_view`). Never touches drafts.
    pub(crate) fn remote_view(
        &mut self,
        select: &[String],
        focus: bool,
        dock: Option<&str>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.dirty() && self.drag.is_none(),
            "the editor has an unsaved draft; ask the user to apply or reload it first"
        );
        for id in select {
            if self.world.entity(id).is_none() {
                return Err(axiom_core::Diagnostic::error(
                    "not_found",
                    "/select",
                    format!("entity {id} not found"),
                ));
            }
        }
        if let Some(dock) = dock {
            self.dock = commands::dock_from_name(dock).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown dock {dock}; use changes|proposals|history|problems|scenarios|graphs|assets|agent"
                )
            })?;
        }
        if let Some((first, rest)) = select.split_first() {
            self.select(Some(first.clone()));
            self.extra = rest.iter().cloned().collect();
        }
        self.frame_selection |= focus;
        Ok(())
    }

    /// Remote asset inspector: `material:ID`, `prefab:ID` or `resource:ID`.
    pub(crate) fn remote_open(&mut self, target: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!self.dirty(), "the editor has an unsaved draft");
        let (kind, id) = target
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("open expects material:ID, prefab:ID or resource:ID"))?;
        let missing = || {
            axiom_core::Diagnostic::error("not_found", "/open", format!("{kind} {id} not found"))
        };
        match kind {
            "material" => {
                self.open_material(id);
                if self.material.is_none() {
                    return Err(missing());
                }
            }
            "prefab" => {
                self.open_prefab(id);
                if self.prefab.is_none() {
                    return Err(missing());
                }
            }
            "resource" => {
                if !self.world.resources.contains_key(id) {
                    return Err(missing());
                }
                self.open_resource(id);
            }
            other => anyhow::bail!("unknown asset kind {other}; use material, prefab or resource"),
        }
        Ok(())
    }

    /// Compact, agent-ready description of the current selection (same shape as
    /// `axiom acl -e "inspect ID"`), for pasting into an AI conversation.
    pub(crate) fn context_text(&self) -> String {
        let mut out = format!(
            "# world {} rev {} · selection {}\n",
            self.world.id,
            self.rev,
            self.selection().len()
        );
        for id in self.selection() {
            let Some(e) = self.world.entity(&id) else {
                continue;
            };
            out.push_str(&id);
            if let Some(label) = &e.label {
                out.push_str(&format!(" label={}", json!(label)));
            }
            if let Some(prefab) = &e.prefab {
                out.push_str(&format!(" prefab={}", json!(prefab)));
            }
            if !e.tags.is_empty() {
                out.push_str(&format!(" tags={}", json!(e.tags)));
            }
            let mut keys: BTreeSet<&String> = e.components.keys().collect();
            if let Some(p) = e.prefab.as_ref().and_then(|p| self.world.prefabs.get(p)) {
                keys.extend(p.components.keys());
            }
            for key in keys {
                if let Some(v) = ui::component_value(&self.world, e, key) {
                    out.push_str(&format!(
                        "\n  {key} {}",
                        axiom_core::local::display_value(&v)
                    ));
                }
            }
            out.push('\n');
        }
        out.push_str("# edit with: axiom acl -e \"set ID component/field VALUE\" (add --propose for review)\n");
        out
    }

    pub fn set_scenarios(&mut self, files: Vec<String>) {
        self.scenarios = files;
    }

    pub fn set_scenario_result(&mut self, path: &str, result: Value) {
        let passed = result["ok"] == true;
        self.scenario_results.insert(path.to_owned(), result);
        self.dock = Dock::Scenarios;
        if passed {
            self.notice(format!("Scenario passed · {path}"));
        } else {
            self.set_error(format!("Scenario failed · {path}"));
        }
    }

    pub fn set_proposals(&mut self, signature: (i64, i64), rows: &Value) {
        let previous = self.proposals.len();
        self.proposals = rows.as_array().cloned().unwrap_or_default();
        self.proposal_signature = Some(signature);
        if self
            .proposal_focus
            .is_none_or(|id| !self.proposals.iter().any(|p| p["id"] == id))
        {
            self.proposal_focus = self.proposals.first().and_then(|p| p["id"].as_i64());
        }
        if self.proposals.len() > previous && !self.error {
            self.status = format!(
                "{} proposal(s) awaiting review in the Proposals tab",
                self.proposals.len()
            );
        }
    }

    pub fn set_history(&mut self, rows: &Value) {
        self.history = rows.as_array().cloned().unwrap_or_default();
        self.needs_history = false;
    }

    pub fn set_history_diff(&mut self, rev: i64, diff: Value) {
        self.history_diff = Some((rev, diff));
        self.dock = Dock::History;
    }

    /// Records a failed request as a structured problem and shows it in the status line.
    pub fn problem(&mut self, error: &anyhow::Error) {
        let response = axiom_core::error_response(error);
        let d = response["diagnostics"][0].clone();
        self.problems.insert(0, d);
        self.problems.truncate(32);
        self.set_error(format!("{error:#}"));
    }

    pub fn set_component_hints(&mut self, registry: &axiom_core::Registry) {
        self.component_hints = registry
            .components
            .iter()
            .map(|(id, d)| (id.clone(), d.schema.clone()))
            .collect();
    }

    pub fn dirty(&self) -> bool {
        self.draft.as_ref().is_some_and(Draft::dirty)
            || self.resource.as_ref().is_some_and(|draft| draft.dirty)
            || self.material.as_ref().is_some_and(|d| d.dirty())
            || self.prefab.as_ref().is_some_and(|d| d.dirty())
    }

    pub fn reset_draft(&mut self) {
        if let Some(id) = self.material.as_ref().map(|d| d.id.clone()) {
            self.open_material(&id);
        } else if let Some(id) = self.prefab.as_ref().map(|d| d.id.clone()) {
            self.open_prefab(&id);
        } else if let Some(resource) = &self.resource {
            let id = resource.id.clone();
            let field = resource.field.clone();
            self.open_resource(&id);
            if let Some(draft) = &mut self.resource {
                draft.field_input = field;
                let _ = draft.load(&self.world);
            }
        } else {
            self.select(self.selected.clone());
        }
    }

    pub fn select(&mut self, id: Option<String>) {
        self.resource = None;
        self.material = None;
        self.prefab = None;
        let entity = id.as_deref().and_then(|id| self.world.entity(id));
        self.selected = entity.map(|e| e.id.clone());
        self.drag = None;
        self.scroll_selection = true;
        self.draft = entity.map(|e| {
            let transform = crate::editor::ui::component_value(&self.world, e, "core.transform")
                .and_then(|value| serde_json::from_value(value).ok());
            let angles = transform.as_ref().map_or([0.0; 3], viewport::degrees);
            let parent = self
                .world
                .relations
                .iter()
                .find(|r| r.kind == "parent" && r.source == e.id);
            Draft {
                rev: self.rev,
                id: e.id.clone(),
                label: e.label.clone().unwrap_or_default(),
                base_transform: transform.clone(),
                transform,
                angles,
                parent: parent.map(|r| r.target.clone()).unwrap_or_default(),
                keep_world: true,
                component: String::new(),
                component_input: String::new(),
                json: "{}".into(),
                label_dirty: false,
                transform_dirty: false,
                parent_dirty: false,
                component_dirty: false,
                form: None,
                form_base: Value::Null,
                form_dirty: false,
                json_mode: false,
            }
        });
    }

    fn try_select(&mut self, id: Option<String>) {
        if self.selected == id && self.resource.is_none() {
            return;
        }
        if self.dirty() {
            self.set_error("Apply or reload the inspector draft before switching objects.");
        } else {
            self.select(id);
        }
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = true;
        self.status = message.into();
    }

    pub fn notice(&mut self, message: impl Into<String>) {
        self.error = false;
        self.status = message.into();
    }

    pub fn record(&mut self, request: Request, result: &Value) {
        self.last_bytes = Some((
            serde_json::to_vec(&request).map_or(0, |v| v.len()),
            serde_json::to_vec(result).map_or(0, |v| v.len()),
        ));
        self.record_changes(&result["diff"]);
        self.notice(if matches!(request, Request::Preview { .. }) {
            format!(
                "Preview · {} changes · no revision written",
                result["diff"]["total_changes"]
            )
        } else if result["changed"] == false {
            "Already up to date".into()
        } else {
            format!("Committed revision {}", result["rev"])
        });
        self.last_request = Some(request);
    }

    fn record_changes(&mut self, diff: &Value) {
        self.changes = diff["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some(format!(
                    "{}  {}",
                    row["kind"].as_str()?,
                    row["path"].as_str()?
                ))
            })
            .collect();
    }

    pub fn record_apply(&mut self, previous: i64) {
        self.undo.push(previous);
        self.redo.clear();
    }

    pub fn record_restore(&mut self, previous: i64, redo: bool) {
        if redo {
            self.redo.pop();
            self.undo.push(previous);
        } else {
            self.undo.pop();
            self.redo.push(previous);
        }
    }

    pub fn observation(&self) -> Value {
        json!({
            "mode":if self.previewing {"play"} else {"edit"},
            "authoring_rev":self.rev,
            "selected":self.selected,
            "resource":self.resource.as_ref().map(|draft|&draft.id),
            "scene":self.scene,
            "draft_rev":self.resource.as_ref().map(|draft|draft.rev).or_else(||self.draft.as_ref().map(|draft| draft.rev)),
            "draft_dirty":self.dirty(),
            "tool":self.tool.name(),
            "pending_proposals":self.proposals.len(),
            "hidden":self.prefs.hidden.get(&self.world.id),
            "locked":self.prefs.locked.get(&self.world.id),
            "selection":self.selection()
        })
    }

    fn write(&self, edits: Vec<Edit>, select: Option<String>) -> Action {
        Action::Write {
            request: Request::Apply {
                expected_rev: Some(self.rev),
                edits,
            },
            select,
        }
    }

    pub fn render_frame(
        &self,
        runtime: &mut WorldRuntime,
        screen: egui::Rect,
    ) -> crate::presentation::RenderFrame {
        let draft = self
            .draft
            .as_ref()
            .filter(|d| d.transform_dirty && d.rev == runtime.rev);
        // The override only lives during extraction, so all descendants and native mesh
        // extractors see the draft, while the simulation retains the committed transform.
        let original = draft.and_then(|d| {
            let &slot = runtime.state.compiled.indices.get(&d.id)?;
            let transform = d.transform.clone()?;
            transform.validate().ok()?;
            Some((slot, runtime.state.transforms[slot].replace(transform)))
        });
        // Gameplay may choose a different scene during its initial entry actions.
        let mut visibility = runtime.authored_visibility.clone();
        visibility.resize(runtime.state.compiled.len(), false);
        // Editor-view "hidden" objects (and their descendants) are not drawn; play is unaffected.
        for id in self.view_flagged(false) {
            if let Some(&slot) = runtime.state.compiled.indices.get(&id) {
                visibility[slot] = false;
            }
        }
        let original_visibility =
            std::mem::replace(&mut runtime.state.compiled.enabled, visibility);
        let mut frame = runtime.render_frame();
        runtime.state.compiled.enabled = original_visibility;
        if let Some((slot, transform)) = original {
            runtime.state.transforms[slot] = transform;
        }
        frame.views.clear();
        frame.views.push(self.orbit.view(self.viewport, screen));
        frame
    }
}
