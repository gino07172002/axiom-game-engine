mod resource;
mod ui;
mod viewport;

use crate::world::WorldRuntime;
use axiom_core::{local::Target, CoreEdit, Edit, Request, Transform, World};
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
}

impl Draft {
    fn dirty(&self) -> bool {
        self.label_dirty || self.transform_dirty || self.parent_dirty || self.component_dirty
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
        self.drag = None;
        if !local {
            self.undo.clear();
            self.redo.clear();
            if self.dirty() {
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

    pub fn dirty(&self) -> bool {
        self.draft.as_ref().is_some_and(Draft::dirty)
            || self.resource.as_ref().is_some_and(|draft| draft.dirty)
    }

    pub fn reset_draft(&mut self) {
        if let Some(resource) = &self.resource {
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
            "tool":self.tool.name()
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
