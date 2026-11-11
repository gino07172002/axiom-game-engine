mod asset_browser;
mod assets;
mod chrome;
mod commands;
pub(crate) mod debug;
mod form;
pub(crate) mod game;
mod graph;
mod icons;
pub use commands::register_topics;
pub use thumbs::validate_thumbnails_gpu;
mod layout;
mod options;
mod panels;
pub(crate) mod record;
mod resource;
mod theme;
mod thumbs;
mod timeline;
mod ui;
mod ui_editor;
mod viewport;
mod world_panel;
// Tuning editors (M9): Input Map, Localization, Animation, VFX tabs and the shader source.
mod anim_timeline;
mod input_map;
mod localization;
mod shader_editor;
mod tuning;
mod vfx_editor;
// AI co-editing (M10): change ghosts in the Scene view, field-wise conflict merge.
pub(crate) mod ghost;
pub(crate) mod merge;
// Follow-ups: readable diff lines, prefab override markers.
mod diff_view;
mod prefab_overrides;
// Workstream E (editor look): Scene view display mode and screenshot.
mod look;

use crate::world::WorldRuntime;
#[cfg(test)]
mod asset_browser_tests;
#[cfg(test)]
mod chrome_tests;
#[cfg(test)]
mod coedit_tests;
#[cfg(test)]
mod debug_tests;
#[cfg(test)]
mod dock_tests;
#[cfg(test)]
mod followup_tests;
#[cfg(test)]
mod game_tests;
#[cfg(test)]
mod graph_tests;
#[cfg(test)]
mod hierarchy_tests;
#[cfg(test)]
mod inspector_tests;
#[cfg(test)]
mod parity;
#[cfg(test)]
mod tuning_tests;
#[cfg(test)]
mod ui_editor_tests;
#[cfg(test)]
mod world_tests;
use axiom_core::{local::Target, ops, CoreEdit, Edit, Request, Transform, World};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use viewport::{Drag, Orbit, SceneInput, Tool};

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
    /// Play ▸ Stop: back to editing (the simulation is discarded).
    PlayStop,
    /// Play ▸ Pause / Resume: stops or resumes the fixed-step simulation (UI keeps drawing).
    PlayPause,
    /// Play ▸ Step: pauses, then advances exactly one fixed step.
    PlayStep,
    /// Play ▸ Record (while recording): write the recorded scenario; Play continues.
    RecordStop,
    /// Load the before/after diff of one revision against its predecessor.
    LoadDiff {
        rev: i64,
    },
    /// Append a revision restoring `rev` (timeline restore, independent of the undo stack).
    RestoreTo {
        rev: i64,
    },
    Verify,
    /// `keep`: approve by a field-wise merge onto the current revision; listed conflicting
    /// fields keep the current value (`resolve_proposal {keep, expected_rev}`).
    ResolveProposal {
        id: i64,
        approve: bool,
        rebase: bool,
        keep: Option<Vec<String>>,
        expected_rev: Option<i64>,
    },
    /// Start the Scene-view ghost boxes of a proposal / revision (reads the store, then a
    /// background job; no write).
    LoadGhosts {
        source: ghost::GhostSource,
    },
    /// List the conflicts of a pending proposal against the current revision (reads the
    /// store, then a background job; no write).
    PrepareMerge {
        proposal: i64,
    },
    /// Run a scenario file headless against the current revision (background job).
    RunScenario {
        path: String,
    },
    /// Ask the running scenario job to stop at its next tick.
    CancelScenario,
    /// Close the editor window (File ▸ Exit).
    Quit,
    /// View ▸ Toggle Fullscreen Window (borderless on the current monitor).
    ToggleFullscreen,
    /// Inspector ▸ ▶ Play clip: start Play preview, then run `capability.action {args}` once
    /// (e.g. `tween.play {id, clip}`, `audio.play {id}`).
    PlayClip {
        capability: String,
        action: String,
        args: Value,
    },
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

    /// Live Inspector: the draft's values were just sent as a write; they become its baseline
    /// so the same change is not sent again while the write is saving. A refused write
    /// resets the draft from the committed world (`revert_staged` / reload).
    fn mark_committed(&mut self) {
        if self.label_dirty || self.transform_dirty || self.parent_dirty || self.form_dirty {
            self.base_transform = self.transform.clone();
            if let Some(form) = &self.form {
                self.form_base = form.clone();
            }
            self.label_dirty = false;
            self.transform_dirty = false;
            self.parent_dirty = false;
            self.form_dirty = false;
        }
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

/// Everything the Inspector's Apply writes: the draft's edits, preceded by what a newly staged
/// component needs and followed by the same changes on the other selected objects.
fn inspector_edits(
    world: &World,
    registry: &axiom_core::Registry,
    draft: &Draft,
    others: &BTreeSet<String>,
) -> anyhow::Result<Vec<Edit>> {
    let edits = draft.edits()?;
    let mut all = staged_component_requirements(world, registry, draft);
    all.extend(edits);
    all.extend(multi_object_edits(world, draft, others));
    Ok(all)
}

/// Inspector ▸ Stage component of a component the object has neither authored nor inherited:
/// the capability/systems it needs, written in the same batch (as `ops::add_component`).
fn staged_component_requirements(
    world: &World,
    registry: &axiom_core::Registry,
    draft: &Draft,
) -> Vec<Edit> {
    let new = draft.component_dirty
        && world
            .entity(&draft.id)
            .is_some_and(|e| !world.resolved_components(e).contains_key(&draft.component));
    if new {
        ops::component_requirement_edits(world, registry, &draft.component)
    } else {
        vec![]
    }
}

/// Multi-object Inspector: the primary's changed transform axes and component form fields,
/// applied to every other selected object that has them (`ACL: set entities[id=a,id=b] F V`).
/// Position / scale axes and Euler rotation axes are set per axis, so the other values of
/// each object stay; objects without the transform or the field are skipped.
fn multi_object_edits(world: &World, draft: &Draft, others: &BTreeSet<String>) -> Vec<Edit> {
    let mut edits = vec![];
    if others.is_empty() {
        return edits;
    }
    let transform_change = match (&draft.base_transform, &draft.transform) {
        (Some(before), Some(after)) if draft.transform_dirty => Some((before, after)),
        _ => None,
    };
    let leaves = {
        let mut leaves = vec![];
        if let Some(form) = draft.form.as_ref().filter(|_| draft.form_dirty) {
            form::changes(&draft.form_base, form, "", &mut leaves);
        }
        leaves
    };
    let component = draft.component.replace('~', "~0").replace('/', "~1");
    for id in others.iter().filter(|id| **id != draft.id) {
        let Some(entity) = world.entity(id) else {
            continue;
        };
        let resolved = world.resolved_components(entity);
        let mut fields = vec![];
        if let Some((before, after)) = transform_change {
            if let Some(mut own) = resolved
                .get("core.transform")
                .and_then(|v| serde_json::from_value::<Transform>(v.clone()).ok())
            {
                let axes = |from: [f32; 3], to: [f32; 3], into: &mut [f32; 3]| {
                    let mut changed = false;
                    for k in 0..3 {
                        if from[k] != to[k] {
                            into[k] = to[k];
                            changed = true;
                        }
                    }
                    changed
                };
                if axes(before.position, after.position, &mut own.position) {
                    fields.push((
                        "/components/core.transform/position".to_owned(),
                        json!(own.position),
                    ));
                }
                if axes(before.scale, after.scale, &mut own.scale) {
                    fields.push((
                        "/components/core.transform/scale".to_owned(),
                        json!(own.scale),
                    ));
                }
                let mut angles = viewport::degrees(&own);
                if before.rotation != after.rotation
                    && axes(viewport::degrees(before), draft.angles, &mut angles)
                {
                    fields.push((
                        "/components/core.transform/rotation".to_owned(),
                        json!(viewport::rotation(angles)),
                    ));
                }
            }
        }
        if let Some(value) = resolved.get(&draft.component) {
            for (pointer, leaf) in &leaves {
                // Only a field the other object has, with the same JSON type (a box collider's
                // half_extents is not written into a capsule).
                let same = value
                    .pointer(pointer)
                    .is_some_and(|v| std::mem::discriminant(v) == std::mem::discriminant(leaf));
                if same {
                    fields.push((format!("/components/{component}{pointer}"), leaf.clone()));
                }
            }
        }
        if !fields.is_empty() {
            edits.extend(
                ops::set_fields(world, std::slice::from_ref(id), &fields).unwrap_or_default(),
            );
        }
    }
    edits
}

/// An editor panel: one tab of the dock (names and titles in `layout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Dock {
    Hierarchy,
    Scene,
    Game,
    Inspector,
    World,
    Changes,
    Proposals,
    History,
    Problems,
    Scenarios,
    Graphs,
    Assets,
    Agent,
    UiEditor,
    Console,
    Profiler,
    Trace,
    InputMap,
    Localization,
    Animation,
    Vfx,
}

pub(crate) struct Editor {
    /// Shared with the store cache and the runtime build; never copied per revision.
    pub world: std::sync::Arc<World>,
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
    /// Dock panels (tabs, splits, floating windows) and their persisted layout.
    layout: layout::Layout,
    /// The Scene tab was drawn this frame (else the editor view is not rendered).
    scene_visible: bool,
    pub(crate) status: String,
    pub(crate) error: bool,
    pub(crate) changes: Vec<String>,
    last_request: Option<Request>,
    undo: Vec<i64>,
    redo: Vec<i64>,
    frame_selection: bool,
    scroll_selection: bool,
    logo: Option<egui::TextureHandle>,
    /// Icon atlas uploaded to the editor context (None until the first frame or if the
    /// embedded atlas failed to decode, which is reported once).
    icon_atlas: Option<icons::Atlas>,
    icons_tried: bool,
    /// Copy handle of the atlas for drawing.
    icons: icons::Icons,
    /// The editor egui style (theme), built once.
    theme: Option<std::sync::Arc<egui::Style>>,
    /// Last frame's renderer counters and the GPU allocator total (host-supplied).
    pub(crate) frame_stats: crate::render::FrameStats,
    pub(crate) gpu_alloc: Option<u64>,
    /// This process's video memory on the render adapter (host-supplied, every 2 s).
    pub(crate) vram: Option<crate::vram::VramReading>,
    /// Bridge requests accepted so far and the time since the last one (host-supplied).
    pub(crate) bridge_activity: (u64, Option<std::time::Duration>),
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
    pub(crate) scenario_results: std::collections::BTreeMap<String, Value>,
    /// The scenario running on a worker thread (at most one).
    pub(crate) scenario_job: Option<crate::jobs::Job<Value>>,
    /// Summary of the last finished background job (observation).
    last_job: Option<Value>,
    material: Option<assets::MaterialDraft>,
    prefab: Option<assets::PrefabDraft>,
    /// Inline hierarchy rename in progress (F2 / context menu).
    renaming: Option<Rename>,
    /// Screen rects of hierarchy rows and context-menu items drawn last frame (tests, focus).
    pub(crate) rects: std::collections::HashMap<String, egui::Rect>,
    /// Scene-view input state: box selection, fly mode, speed readout.
    scene_input: SceneInput,
    /// Inspector tween timeline: clip, playhead, selected key, scrub preview.
    tween_ui: timeline::TweenUi,
    /// Editor writes queued or running on the background writer (set by the host). While
    /// non-zero, `world` is the optimistic result of those writes and `rev` the last
    /// revision this editor adopted.
    pub(crate) saving: usize,
    /// Writer counters for `observe` (set by the host): last commit duration etc.
    pub(crate) writes: Value,
    /// Installed types (capabilities, systems, resource and component schemas) for the world
    /// operations; the host's registry (`set_component_hints`), else the core one.
    registry: std::sync::Arc<axiom_core::Registry>,
    /// Window ▸ World: open flag, inputs and the rows cached per revision.
    world_panel: world_panel::WorldPanel,
    /// Inspector tag chips: typed tag and the chips cached per revision and selection.
    tag_ui: world_panel::TagUi,
    /// Readable lines of each proposal's diff (parallel to `proposals`) and of the shown
    /// History diff, built when the rows arrive.
    proposal_lines: Vec<Vec<String>>,
    history_lines: Vec<String>,
    /// Inspector prefab override markers: the selection's override sets per revision.
    overrides: prefab_overrides::OverrideCache,
    /// Component to open in the Inspector once the Add component write is shown.
    open_component_after: Option<String>,
    /// Bumped whenever `world` is replaced (with `rev`, the key of the per-world caches;
    /// an address could be reused after the old world is freed).
    world_version: u64,
    /// Typed form specs per component / resource type (cleared with the registry).
    specs: form::SpecCache,
    /// The open entity picker of an Inspector field.
    picker: Option<form::Picker>,
    /// Inspector field filter (component form and resource form).
    field_filter: String,
    /// Fields that differ across the selection, per (world version, selection, component).
    mixed: MixedCache,
    /// Inspector ▸ component ▸ Copy: (component, effective value) for Paste.
    component_clipboard: Option<(String, Value)>,
    /// Create-command options dialog (Object menu / palette).
    options: Option<options::OptionsDialog>,
    /// Scroll the Inspector to the open component on the next frame (`open: component:ID`).
    scroll_component: bool,
    /// The object the Inspector showed last frame (a new one scrolls to the top).
    inspector_shown: Option<String>,
    /// Timeline Play on rows per (world version, object, clip).
    bindings_cache: Option<((u64, String, String), timeline::Bindings)>,
    /// Game tab: composed area, view-only camera choice, input routing flags.
    pub(crate) game: game::GameState,
    /// Play toolbar state (paused / time scale / tick), kept current by the host.
    pub(crate) play: game::PlayView,
    /// Inspector preview hole of the selected camera this frame.
    pub(crate) camera_preview: game::CameraPreview,
    /// Read-only runtime state of the selection during Play (host refreshes it).
    pub(crate) live: game::LiveInspect,
    /// UI Editor tab: selection, preview locale / resolution and the cached preview.
    ui_editor: ui_editor::UiEditorState,
    /// Graphs tab: open machine, selection, cached graph model and checks.
    graph: graph::GraphUi,
    /// Assets tab: filters, selection, Find Usages, thumbnails.
    assets: asset_browser::AssetBrowser,
    /// Scene view display mode and the screenshot job.
    look: look::Look,
    /// Console, Event trace, Profiler and the scenario recording (M8).
    pub(crate) debug: debug::Debug,
    /// Input Map, Localization, Animation, VFX tabs and the shader source (editor state).
    tuning: tuning::Tuning,
    /// Scene-view ghost boxes of a proposal or a revision.
    pub(crate) ghost: ghost::GhostState,
    /// The open field-wise merge (stale draft or proposal against the newer revision).
    pub(crate) merge: merge::MergeState,
    /// The revision a stale, dirty Inspector draft was made on (kept for Merge…).
    merge_base: Option<(i64, std::sync::Arc<World>)>,
}

/// Multi-object Inspector: which fields differ between the selected objects.
#[derive(Default)]
pub(crate) struct MixedCache {
    key: Option<(u64, Vec<String>, String)>,
    /// Component field pointers (`form::mixed_leaves`).
    pub fields: std::collections::HashSet<String>,
    /// Transform axes that differ: position, rotation (Euler degrees), scale.
    pub transform: [[bool; 3]; 3],
    /// Times recomputed (tests: not per frame).
    pub builds: usize,
}

/// Inline rename state: the row being renamed, the text being typed, focus requested yet.
pub(crate) struct Rename {
    pub id: String,
    pub text: String,
    pub focused: bool,
}

impl Editor {
    pub fn new(
        world: impl Into<std::sync::Arc<World>>,
        rev: i64,
        selected: Option<String>,
    ) -> Self {
        let mut editor = Self {
            world: world.into(),
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
            layout: Default::default(),
            scene_visible: true,
            status: "Ready".into(),
            error: false,
            changes: Vec::new(),
            last_request: None,
            undo: Vec::new(),
            redo: Vec::new(),
            frame_selection: false,
            scroll_selection: false,
            logo: None,
            icon_atlas: None,
            icons_tried: false,
            icons: Default::default(),
            theme: None,
            frame_stats: Default::default(),
            gpu_alloc: None,
            vram: None,
            bridge_activity: (0, None),
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
            scenario_job: None,
            last_job: None,
            material: None,
            prefab: None,
            renaming: None,
            rects: Default::default(),
            scene_input: SceneInput::default(),
            tween_ui: Default::default(),
            saving: 0,
            writes: Value::Null,
            registry: std::sync::Arc::new(axiom_core::Registry::core()),
            world_panel: Default::default(),
            tag_ui: Default::default(),
            proposal_lines: Vec::new(),
            history_lines: Vec::new(),
            overrides: Default::default(),
            open_component_after: None,
            world_version: 0,
            specs: Default::default(),
            picker: None,
            field_filter: String::new(),
            mixed: Default::default(),
            component_clipboard: None,
            options: None,
            scroll_component: false,
            inspector_shown: None,
            bindings_cache: None,
            game: Default::default(),
            play: Default::default(),
            camera_preview: Default::default(),
            live: Default::default(),
            ui_editor: Default::default(),
            tuning: Default::default(),
            graph: Default::default(),
            assets: Default::default(),
            look: Default::default(),
            debug: Default::default(),
            ghost: Default::default(),
            merge: Default::default(),
            merge_base: None,
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

    pub fn update(&mut self, rev: i64, world: impl Into<std::sync::Arc<World>>, local: bool) {
        if let Some((previous, true)) = self.update_deferred(rev, world, local) {
            let diff = axiom_core::diff::summary(&previous, &self.world);
            self.record_changes(&diff);
        }
    }

    /// `update` without the change-list diff. Returns the replaced world (so the host can
    /// free it off the main thread) and whether the Changes list needs
    /// `diff::summary(previous, self.world)`, delivered later with `set_changes_for`.
    pub(crate) fn update_deferred(
        &mut self,
        rev: i64,
        world: impl Into<std::sync::Arc<World>>,
        local: bool,
    ) -> Option<(std::sync::Arc<World>, bool)> {
        let world = world.into();
        if self.rev == rev {
            return None;
        }
        let diff = !local || self.changes.is_empty();
        let previous_rev = std::mem::replace(&mut self.rev, rev);
        let previous = std::mem::replace(&mut self.world, world);
        // A dirty draft made on the replaced revision goes stale: keep that revision so the
        // draft can be merged field by field onto the new one (Merge…).
        if !local
            && self.merge_base.is_none()
            && self
                .draft
                .as_ref()
                .is_some_and(|d| d.dirty() && d.rev == previous_rev)
        {
            self.merge_base = Some((previous_rev, previous.clone()));
        }
        self.world_version += 1;
        self.needs_history = true;
        self.forget_missing();
        // A drag only starts from a clean draft, so its dirtiness is transient gizmo state.
        // Dropping the drag without resetting would strand an uncommittable stale draft.
        let interrupted = self.drag.take().is_some() | self.scene_input.point_drag.take().is_some();
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
        if local || !self.dirty() {
            self.reset_draft();
        }
        Some((previous, diff))
    }

    /// Drops view state that refers to objects or scenes the current world no longer has.
    fn forget_missing(&mut self) {
        let world = &self.world;
        self.extra.retain(|id| world.entity(id).is_some());
        if self
            .renaming
            .as_ref()
            .is_some_and(|r| world.entity(&r.id).is_none())
        {
            self.renaming = None;
        }
        if self
            .scene
            .as_ref()
            .is_some_and(|id| !world.scenes.iter().any(|s| &s.id == id))
        {
            self.scene = None;
        }
        // The Game tab's camera choice falls back to the active camera(s).
        if self
            .game
            .camera
            .as_ref()
            .is_some_and(|id| world.entity(id).is_none())
        {
            self.game.camera = None;
        }
    }

    /// Shows the optimistic world of a queued write (staged off the main thread) and builds
    /// further edits on it. `rev` stays the last adopted revision. The submitted draft is
    /// replaced by the staged values; `select` (e.g. a created object) is selected.
    /// Returns the replaced world so the host can free it off the main thread.
    pub(crate) fn apply_staged(
        &mut self,
        world: std::sync::Arc<World>,
        select: Option<String>,
    ) -> std::sync::Arc<World> {
        let previous = std::mem::replace(&mut self.world, world);
        self.world_version += 1;
        self.forget_missing();
        // Live Inspector: an edit or drag started while the earlier write was staging stays
        // (its baseline is that write's values); it commits once the queue drains.
        let busy = self.dirty() || self.drag.is_some() || self.scene_input.point_drag.is_some();
        if !(self.prefs.live_inspector && busy) {
            self.reset_draft();
        }
        if let Some(id) = select {
            self.select(Some(id));
        }
        previous
    }

    /// The writer committed this editor's own queued writes up to `rev` and nothing else is
    /// pending: adopt the committed world (same content as the staged one). Unlike an
    /// external revision this keeps the undo stack, a draft being edited meanwhile (its
    /// revision tag moves to `rev`) and a gizmo drag in progress.
    pub(crate) fn commit_landed(
        &mut self,
        rev: i64,
        world: std::sync::Arc<World>,
    ) -> Option<std::sync::Arc<World>> {
        if self.rev == rev {
            return None;
        }
        let old = std::mem::replace(&mut self.rev, rev);
        let previous = std::mem::replace(&mut self.world, world);
        self.world_version += 1;
        self.needs_history = true;
        self.forget_missing();
        if self.dirty() || self.drag.is_some() || self.scene_input.point_drag.is_some() {
            if let Some(d) = self.draft.as_mut().filter(|d| d.rev == old) {
                d.rev = rev;
            }
            if let Some(d) = self.resource.as_mut().filter(|d| d.rev == old) {
                d.rev = rev;
            }
            if let Some(d) = self.material.as_mut().filter(|d| d.rev == old) {
                d.rev = rev;
            }
            if let Some(d) = self.prefab.as_mut().filter(|d| d.rev == old) {
                d.rev = rev;
            }
        } else {
            self.reset_draft();
        }
        Some(previous)
    }

    /// A queued write failed after its optimistic world was shown: return to `world` (the
    /// last committed state at the same revision).
    pub(crate) fn revert_staged(&mut self, world: std::sync::Arc<World>) -> std::sync::Arc<World> {
        let previous = std::mem::replace(&mut self.world, world);
        self.world_version += 1;
        self.forget_missing();
        self.drag = None;
        self.scene_input.point_drag = None;
        self.reset_draft();
        previous
    }

    /// Change list computed off the main thread for revision `rev`; ignored if stale.
    pub(crate) fn set_changes_for(&mut self, rev: i64, diff: &Value) {
        if self.rev == rev {
            self.record_changes(diff);
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

    /// Inspector ▸ Remove component (`ACL: remove ID COMPONENT`).
    pub(super) fn remove_component(&mut self, id: &str, component: &str) -> Option<Action> {
        let result = ops::remove_component(&self.world, &[id.to_owned()], component);
        self.op_write(result, None)
    }

    /// Selected objects that have `component` (own or inherited), primary first.
    fn selection_with(&self, component: &str) -> Vec<String> {
        self.selection()
            .into_iter()
            .filter(|id| {
                self.world
                    .entity(id)
                    .is_some_and(|e| ui::component_value(&self.world, e, component).is_some())
            })
            .collect()
    }

    /// Inspector ▸ component ▸ Reset (`ACL: reset SEL COMPONENT`) on every selected object
    /// that has it.
    pub(super) fn reset_component(&mut self, component: &str) -> Option<Action> {
        let ids = self.selection_with(component);
        let result = ops::reset_component(&self.world, &self.registry, &ids, component);
        self.op_write(result, None)
    }

    /// Inspector ▸ component ▸ Copy: the effective value, kept for Paste and placed on the
    /// system clipboard as `{"component":ID,"value":{..}}`.
    pub(super) fn copy_component(&mut self, component: &str) {
        let Some(value) = self
            .selected
            .as_deref()
            .and_then(|id| self.world.entity(id))
            .and_then(|e| ui::component_value(&self.world, e, component))
        else {
            return;
        };
        self.pending_copy = Some(json!({"component":component,"value":value}).to_string());
        self.component_clipboard = Some((component.to_owned(), value));
        self.notice(format!("Copied {component} values"));
    }

    /// Inspector ▸ component ▸ Paste values (`ACL: set SEL /components/C VALUE`) on every
    /// selected object that has the component.
    pub(super) fn paste_component(&mut self, component: &str) -> Option<Action> {
        let value = self
            .component_clipboard
            .as_ref()
            .filter(|(c, _)| c == component)?
            .1
            .clone();
        let ids = self.selection_with(component);
        let field = format!(
            "/components/{}",
            component.replace('~', "~0").replace('/', "~1")
        );
        let result = ops::set_fields(&self.world, &ids, &[(field, value)]);
        self.op_write(result, None)
    }

    /// Inspector form array buttons (`ACL: append|insert|remove|reorder ID C/F ..`) on the
    /// primary selection's authored component.
    pub(super) fn edit_component_array(
        &mut self,
        component: &str,
        pointer: &str,
        edit: &ops::ArrayEdit,
    ) -> Option<Action> {
        let id = self.selected.clone()?;
        let mut select = axiom_core::local::Selection::all(axiom_core::local::Collection::Entities);
        select.ids = vec![id];
        let field = format!(
            "/components/{}{pointer}",
            component.replace('~', "~0").replace('/', "~1")
        );
        let result = ops::edit_array(&self.world, &self.registry, &select, &field, edit);
        self.op_write(result, None)
    }

    /// Inspector ▸ ▶ Play: enters Play preview and plays the timeline clip (`tween.play`) or
    /// the audio source (`audio.play`) of the primary selection.
    pub(super) fn play_clip(&self, component: &str) -> Option<Action> {
        let id = self.selected.clone()?;
        let (capability, args) = match component {
            "tween.clips" => {
                let (_, clip, _) = self.tween_target()?;
                ("tween", json!({"id":id,"clip":clip,"restart":true}))
            }
            "audio.source" => ("audio", json!({"id":id})),
            _ => return None,
        };
        Some(Action::PlayClip {
            capability: capability.into(),
            action: "play".into(),
            args,
        })
    }

    /// Recomputes which fields differ across the selection when the world, the selection
    /// or the open component changed (never per frame).
    pub(super) fn refresh_mixed(&mut self) {
        let selection = self.selection();
        let component = self
            .draft
            .as_ref()
            .map(|d| d.component.clone())
            .unwrap_or_default();
        let key = (self.world_version, selection, component);
        if self.mixed.key.as_ref() == Some(&key) {
            return;
        }
        self.mixed.builds += 1;
        self.mixed.fields.clear();
        self.mixed.transform = Default::default();
        if key.1.len() > 1 {
            let world = self.world.clone();
            let entities: Vec<&axiom_core::Entity> =
                key.1.iter().filter_map(|id| world.entity(id)).collect();
            let transforms: Vec<Transform> = entities
                .iter()
                .filter_map(|e| ui::component_value(&world, e, "core.transform"))
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect();
            if let Some((first, rest)) = transforms.split_first() {
                let angles = viewport::degrees(first);
                for t in rest {
                    let other = viewport::degrees(t);
                    for k in 0..3 {
                        self.mixed.transform[0][k] |= t.position[k] != first.position[k];
                        self.mixed.transform[1][k] |= (other[k] - angles[k]).abs() > 1e-3;
                        self.mixed.transform[2][k] |= t.scale[k] != first.scale[k];
                    }
                }
            }
            if !key.2.is_empty() {
                let values: Vec<Value> = entities
                    .iter()
                    .filter_map(|e| ui::component_value(&world, e, &key.2))
                    .collect();
                if let Some((first, rest)) = values.split_first() {
                    let spec = self.specs.get(&self.registry, &world, &key.2);
                    self.mixed.fields = form::mixed_leaves(first, rest, &spec);
                }
            }
        }
        self.mixed.key = Some(key);
    }

    /// The effective `path.spline` of `id`, if it has a valid one.
    pub(super) fn path_spline(&self, id: &str) -> Option<axiom_core::path::PathSpline> {
        let entity = self.world.entity(id)?;
        serde_json::from_value(
            self.world
                .resolved_components(entity)
                .remove("path.spline")?,
        )
        .ok()
    }

    /// Object ▸ Follow Path (`ACL: follow SEL PATH`): the selected objects follow the one
    /// selected path, or the world's only path when no path is selected.
    pub(super) fn follow_selection(&mut self, options: &ops::FollowOptions) -> Option<Action> {
        let Some((followers, path)) = self.follow_targets() else {
            self.set_error(
                "Select one path together with the objects that should follow it (Ctrl+click)",
            );
            return None;
        };
        let result = ops::follow_path(&self.world, &followers, &path, options);
        self.op_write(result, None)
    }

    /// (followers, path) of Follow Path: the one selected path, else the world's only path.
    pub(super) fn follow_targets(&self) -> Option<(Vec<String>, String)> {
        let selection = self.selection();
        let mut paths: Vec<String> = selection
            .iter()
            .filter(|id| self.path_spline(id).is_some())
            .cloned()
            .collect();
        if paths.is_empty() {
            paths = self
                .world
                .entities
                .iter()
                .filter(|e| self.path_spline(&e.id).is_some())
                .map(|e| e.id.clone())
                .collect();
        }
        let [path] = paths.as_slice() else {
            return None;
        };
        let followers: Vec<String> = selection.into_iter().filter(|id| id != path).collect();
        Some((followers, path.clone()))
    }

    /// Path point handles and Inspector ▸ Path points (`ACL: path ID add|insert|move|remove`).
    pub(super) fn edit_path_point(&mut self, id: &str, edit: ops::PointEdit) -> Option<Action> {
        let result = ops::edit_path_point(&self.world, id, &edit);
        self.op_write(result, None)
    }

    /// Path ▸ Add Point: after the selected point (splitting the piece to the next point, or
    /// extending an open path's end), else at the end. Selects the new point.
    pub(super) fn add_path_point(&mut self) -> Option<Action> {
        let id = self.selected.clone()?;
        let spline = self.path_spline(&id)?;
        let n = spline.points.len();
        let at = match self.scene_input.path_point.filter(|&i| i < n) {
            Some(i) if i + 1 < n || spline.closed => i + 1,
            _ => n,
        };
        let edit = if at == n && !spline.closed {
            ops::PointEdit::Add(None)
        } else {
            ops::PointEdit::Insert(at, None)
        };
        let action = self.edit_path_point(&id, edit);
        if action.is_some() {
            self.scene_input.path_point = Some(at);
        }
        action
    }

    /// Path ▸ Remove Point: the selected point, else the last one.
    pub(super) fn remove_path_point(&mut self) -> Option<Action> {
        let id = self.selected.clone()?;
        let n = self.path_spline(&id)?.points.len();
        let index = self
            .scene_input
            .path_point
            .filter(|&i| i < n)
            .unwrap_or(n - 1);
        let action = self.edit_path_point(&id, ops::PointEdit::Remove(index));
        if action.is_some() {
            self.scene_input.path_point = index.checked_sub(1);
        }
        action
    }

    /// The effective `nav.agent` of `id`, if it has a valid one.
    pub(super) fn nav_agent(&self, id: &str) -> Option<axiom_core::navigation::NavAgent> {
        let entity = self.world.entity(id)?;
        serde_json::from_value(self.world.resolved_components(entity).remove("nav.agent")?).ok()
    }

    /// Selected objects that are navigation agents (selection order).
    pub(super) fn selected_agents(&self) -> Vec<String> {
        self.selection()
            .into_iter()
            .filter(|id| self.nav_agent(id).is_some())
            .collect()
    }

    /// Object ▸ Add Nav Agent (`ACL: nav SEL agent [speed=] [radius=]`).
    pub(super) fn add_nav_agents(&mut self, options: &ops::NavOptions) -> Option<Action> {
        let result = ops::add_nav_agent(&self.world, &self.selection(), options);
        self.op_write(result, None)
    }

    /// Object ▸ Nav Target: Selected Object (`ACL: nav AGENTS target ID`): the selected agents
    /// walk to the one selected object that is not an agent.
    pub(super) fn nav_target_selection(&mut self) -> Option<Action> {
        let selection = self.selection();
        let others: Vec<&String> = selection
            .iter()
            .filter(|id| self.nav_agent(id).is_none())
            .collect();
        let [target] = others.as_slice() else {
            self.set_error(
                "Select the agents together with one target object that is not an agent (Ctrl+click)",
            );
            return None;
        };
        let target = axiom_core::navigation::NavTarget::Entity((*target).clone());
        self.set_nav_target(Some(target))
    }

    /// Writes the selected agents' target (`ACL: nav SEL target x,y,z|ID|none`); the scene
    /// view's ground pick and Nav Target: Clear land here.
    pub(super) fn set_nav_target(
        &mut self,
        target: Option<axiom_core::navigation::NavTarget>,
    ) -> Option<Action> {
        let result = ops::set_nav_target(&self.world, &self.selected_agents(), target.as_ref());
        self.op_write(result, None)
    }

    /// Object ▸ Make Interactable (`ACL: interact SEL [event=..]`): `interact.target` with the
    /// dialog's options (defaults from a shortcut / the bridge) on the selection, with its event
    /// declared and its button bound.
    pub(super) fn make_interactable(&mut self, options: &ops::InteractOptions) -> Option<Action> {
        let result = ops::make_interactable(&self.world, &self.selection(), options);
        self.op_write(result, None)
    }

    /// True when `id` has a `render.camera` (own or inherited).
    fn is_camera(&self, id: &str) -> bool {
        self.world.entity(id).is_some_and(|e| {
            self.world
                .resolved_components(e)
                .contains_key("render.camera")
        })
    }

    /// Object ▸ Camera Follow Selected (`ACL: camera CAMERAS follow TARGET`): the selected
    /// cameras follow the one selected object that is not a camera, with default
    /// `camera.follow` fields (edit them in the Inspector).
    pub(super) fn camera_follow_selection(
        &mut self,
        options: &ops::CameraFollowOptions,
    ) -> Option<Action> {
        let Some((cameras, target)) = self.camera_targets() else {
            self.set_error(
                "Select the camera(s) together with one object to follow that is not a camera (Ctrl+click)",
            );
            return None;
        };
        let result = ops::add_camera_follow(&self.world, &cameras, &target, options);
        self.op_write(result, None)
    }

    /// (cameras, target) of Camera Follow Selected: the selected cameras and the one other
    /// selected object.
    pub(super) fn camera_targets(&self) -> Option<(Vec<String>, String)> {
        let (cameras, others): (Vec<String>, Vec<String>) = self
            .selection()
            .into_iter()
            .partition(|id| self.is_camera(id));
        let ([target], false) = (others.as_slice(), cameras.is_empty()) else {
            return None;
        };
        Some((cameras, target.clone()))
    }

    /// Object ▸ Set as Main Camera (`ACL: camera main ID`) on the primary selection.
    pub(super) fn make_main_camera(&mut self) -> Option<Action> {
        let id = self.selected.clone()?;
        let result = ops::make_main_camera(&self.world, &id);
        self.op_write(result, None)
    }

    /// Inspector render.mesh "Blocks follow cameras" / Object ▸ Blocks Follow Cameras… on|off
    /// (`ACL: camera SEL blocker on|off`) on the selection.
    pub(super) fn camera_blocker_selection(&mut self, on: bool) -> Option<Action> {
        let ids = self.selection();
        self.camera_blocker(&ids, on)
    }

    /// `camera.blocker` on|off on `ids` (the Inspector checkbox passes the inspected object).
    pub(super) fn camera_blocker(&mut self, ids: &[String], on: bool) -> Option<Action> {
        let result = ops::set_camera_blocker(&self.world, ids, on);
        if result.as_ref().is_ok_and(Vec::is_empty) {
            return None;
        }
        self.op_write(result, None)
    }

    /// World ▸ Migrate Legacy Camera Blockers (`ACL: migrate camera_blocker`).
    pub(super) fn migrate_camera_blockers(&mut self) -> Option<Action> {
        let result = ops::migrate_camera_blocker_tags(&self.world);
        if result.as_ref().is_ok_and(Vec::is_empty) {
            self.notice("No entity uses the deprecated camera_blocker tag");
            return None;
        }
        self.op_write(result, None)
    }

    /// Object ▸ Add Audio Source (`ACL: audio SEL source [clip=..]`): `audio.source` with the
    /// dialog's options on the selection (default clip: the library's first; a world without
    /// clips gets clip `tone`).
    pub(super) fn add_audio_source(&mut self, options: &ops::AudioSourceOptions) -> Option<Action> {
        let result = ops::add_audio_source(&self.world, &self.selection(), options);
        self.op_write(result, None)
    }

    /// Object ▸ Make Audio Listener (`ACL: audio ID listener`) on the primary selection.
    pub(super) fn add_audio_listener(&mut self) -> Option<Action> {
        let id = self.selected.clone()?;
        let result = ops::add_audio_listener(&self.world, &id);
        self.op_write(result, None)
    }

    /// Inspector ▸ Audio source ▸ Clip (`ACL: set ID audio.source/clip "CLIP"`).
    pub(super) fn set_audio_clip(&mut self, clip: &str) -> Option<Action> {
        let result = ops::set_audio_clip(&self.world, &self.selection(), clip);
        self.op_write(result, None)
    }

    /// Inspector ▸ Audio source ▸ New clip (`ACL: audio clip ID PRESET`): a library clip from a
    /// preset; the dialog proposes the preset name (`engine`, `engine2`, ...) as its ID.
    pub(super) fn add_audio_clip(&mut self, id: &str, preset: &str) -> Option<Action> {
        let result = ops::add_audio_clip(&self.world, id, preset);
        self.op_write(result, None)
    }

    /// Clip IDs of `audio.library` and the clip of `id`'s `audio.source`, when it has one.
    pub(super) fn audio_clips(&self, id: &str) -> Option<(Vec<String>, String)> {
        let entity = self.world.entity(id)?;
        let source: axiom_core::audio::AudioSource = serde_json::from_value(
            self.world
                .resolved_components(entity)
                .remove("audio.source")?,
        )
        .ok()?;
        let clips = self
            .world
            .resources
            .get("audio.library")
            .and_then(|l| l.get("clips"))
            .and_then(Value::as_object)
            .map(|c| c.keys().cloned().collect())
            .unwrap_or_default();
        Some((clips, source.clip))
    }

    /// The `tween.clips` authored on `id`, if it has a valid one.
    pub(super) fn tween_clips(&self, id: &str) -> Option<axiom_core::tween::TweenClips> {
        serde_json::from_value(
            self.world
                .entity(id)?
                .components
                .get("tween.clips")?
                .clone(),
        )
        .ok()
    }

    /// The timeline clip of the selected object: the one picked in the Inspector, else the
    /// first; with its track index clamped to the clip.
    pub(super) fn tween_target(&self) -> Option<(String, String, usize)> {
        let id = self.selected.clone()?;
        let clips = self.tween_clips(&id)?;
        let clip = self
            .tween_ui
            .clip
            .clone()
            .filter(|c| clips.clips.contains_key(c))
            .or_else(|| clips.clips.keys().next().cloned())?;
        let track = self
            .tween_ui
            .track
            .min(clips.clips[&clip].tracks.len().saturating_sub(1));
        Some((id, clip, track))
    }

    /// Object ▸ Add Tween Clip (`ACL: tween SEL clip NAME [..]`): clip `name` (the dialog
    /// proposes `clip`, `clip2`, ...) with its options on the selection; it becomes the timeline
    /// clip.
    pub(super) fn add_tween_clip(
        &mut self,
        name: &str,
        options: &ops::TweenClipOptions,
    ) -> Option<Action> {
        let ids = self.selection();
        let result = ops::add_tween_clip(&self.world, &ids, name, options);
        let action = self.op_write(result, None);
        if action.is_some() {
            self.tween_ui = timeline::TweenUi {
                clip: Some(name.to_owned()),
                ..Default::default()
            };
        }
        action
    }

    /// Timeline ▸ Record key (`ACL: tween ID key CLIP TRACK add T`): the selected track's
    /// current authored value at the playhead.
    pub(super) fn record_tween_key(&mut self) -> Option<Action> {
        let (id, clip, track) = self.tween_target()?;
        let edit = ops::KeyEdit::Add {
            time: self.tween_ui.time,
            value: None,
            ease: None,
        };
        let result = ops::edit_tween_key(&self.world, &id, &clip, track, &edit);
        self.op_write(result, None)
    }

    /// Timeline ▸ Delete key (`ACL: tween ID key CLIP TRACK remove I`).
    pub(super) fn remove_tween_key(&mut self) -> Option<Action> {
        let (id, clip, track) = self.tween_target()?;
        let index = self.tween_ui.key?;
        let result =
            ops::edit_tween_key(&self.world, &id, &clip, track, &ops::KeyEdit::Remove(index));
        let action = self.op_write(result, None);
        if action.is_some() {
            self.tween_ui.key = None;
        }
        action
    }

    /// Timeline key drag (`ACL: tween ID key CLIP TRACK move I T`).
    pub(super) fn move_tween_key(
        &mut self,
        clip: &str,
        track: usize,
        index: usize,
        time: f32,
    ) -> Option<Action> {
        let id = self.selected.clone()?;
        let result = ops::edit_tween_key(
            &self.world,
            &id,
            clip,
            track,
            &ops::KeyEdit::Move { index, time },
        );
        self.op_write(result, None)
    }

    /// Timeline mode / autoplay controls (`ACL: tween ID clip NAME set ...`).
    pub(super) fn set_tween_options(
        &mut self,
        clip: &str,
        options: &ops::TweenClipOptions,
    ) -> Option<Action> {
        let id = self.selected.clone()?;
        let result = ops::set_tween_clip(&self.world, &id, clip, options);
        self.op_write(result, None)
    }

    /// Timeline ▸ Add track (`ACL: tween ID track CLIP add PROPERTY`).
    pub(super) fn add_tween_track(&mut self, clip: &str, property: &str) -> Option<Action> {
        let id = self.selected.clone()?;
        let result = ops::add_tween_track(&self.world, &id, clip, property, "");
        self.op_write(result, None)
    }

    /// Timeline ▸ Remove clip (`ACL: tween ID clip NAME remove`).
    pub(super) fn remove_tween_clip(&mut self, clip: &str) -> Option<Action> {
        let id = self.selected.clone()?;
        let result = ops::remove_tween_clip(&self.world, &id, clip);
        let action = self.op_write(result, None);
        if action.is_some() {
            self.tween_ui = Default::default();
        }
        action
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

    /// Create Group (`ACL: group SEL`): a new empty parent for the selection, world poses kept.
    fn group_selection(&mut self) -> Option<Action> {
        let result = ops::create_group(&self.world, &self.selection(), None);
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

    /// Object ▸ Drop to Ground (`ACL: ground SEL`). Already grounded writes nothing.
    pub(super) fn drop_selection_to_ground(&mut self) -> Option<Action> {
        let ids = self.selection();
        match ops::drop_to_ground(&self.world, &ids) {
            Ok(edits) if edits.is_empty() => {
                self.notice("Already on the ground");
                None
            }
            result => self.op_write(result, None),
        }
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
            // Opens the tab if it was closed, then makes it the active, focused one.
            let tab = Dock::from_name(dock).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown dock {dock}; use {}",
                    Dock::ALL.map(Dock::name).join("|")
                )
            })?;
            self.show_dock(tab);
        }
        if let Some((first, rest)) = select.split_first() {
            self.select(Some(first.clone()));
            self.extra = rest.iter().cloned().collect();
        }
        self.frame_selection |= focus;
        Ok(())
    }

    /// Remote inspector: `material:ID`, `prefab:ID`, `resource:ID`, `component:ID` (of the
    /// selection), `options:COMMAND` (a create command's options dialog), `graph:MACHINE`
    /// or `rules:` (Graphs tab).
    pub(crate) fn remote_open(&mut self, target: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!self.dirty(), "the editor has an unsaved draft");
        let (kind, id) = target.split_once(':').ok_or_else(|| {
            anyhow::anyhow!(
                "open expects material:ID, prefab:ID, resource:ID, component:ID, options:COMMAND, graph:MACHINE or rules:"
            )
        })?;
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
            // The selected object's component in the Inspector (view only).
            "component" => {
                let entity = self
                    .selected
                    .as_deref()
                    .and_then(|s| self.world.entity(s))
                    .ok_or_else(|| anyhow::anyhow!("open component:ID needs a selected object"))?;
                if ui::component_value(&self.world, entity, id).is_none() {
                    return Err(missing());
                }
                let (world, entity) = (self.world.clone(), entity.clone());
                if let Some(draft) = self.draft.as_mut() {
                    draft.open_component(&world, &entity, id);
                }
                self.scroll_component = true;
            }
            // A create command's options dialog, prefilled for the selection (view only;
            // `command` + `arg`-less runs it with the defaults).
            "options" => {
                let command = commands::command(id).map(|c| c.id).ok_or_else(missing)?;
                self.options = Some(
                    options::OptionsDialog::for_command(self, command)
                        .ok_or_else(|| anyhow::anyhow!("{id} has no options dialog"))?,
                );
            }
            // Graphs tab on a state machine / on the rule table (view only).
            "graph" => self.graph_open(id)?,
            "rules" => self.graph_open_rules(id)?,
            other => anyhow::bail!(
                "unknown kind {other}; use material, prefab, resource, component, options, graph or rules"
            ),
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
        self.show_dock(Dock::Scenarios);
        if passed {
            self.notice(format!("Scenario passed · {path}"));
        } else {
            self.set_error(format!("Scenario failed · {path}"));
        }
    }

    /// Hands a running scenario job to the editor (progress, cancel, result delivery).
    pub(crate) fn start_scenario(&mut self, job: crate::jobs::Job<Value>) {
        self.notice(format!("Running scenario in background · {}", job.label));
        self.show_dock(Dock::Scenarios);
        self.scenario_job = Some(job);
    }

    pub(crate) fn cancel_scenario(&mut self) {
        if let Some(job) = &mut self.scenario_job {
            job.cancel();
            let label = job.label.clone();
            self.notice(format!("Cancelling scenario · {label}"));
        }
    }

    /// Non-blocking; called once per frame by the host. Delivers a finished result.
    pub(crate) fn poll_jobs(&mut self) {
        use crate::jobs::Outcome;
        self.poll_assets();
        self.poll_shader_check();
        self.poll_ghosts();
        self.poll_merge();
        let Some(job) = &mut self.scenario_job else {
            return;
        };
        let Some(outcome) = job.poll() else {
            return;
        };
        let (done, total) = job.progress();
        let path = job.label.clone();
        let elapsed_ms = job.elapsed().as_millis() as u64;
        self.scenario_job = None;
        let (mut result, state) = match outcome {
            Outcome::Done { value, stop } => {
                let state = match (stop, value["ok"] == true) {
                    (_, true) => "finished",
                    (Some(stop), false) => stop.name(),
                    (None, false) => "finished",
                };
                (value, state)
            }
            Outcome::Abandoned { stop } => (
                json!({"ok":false,"error":format!("scenario worker did not stop within {} s after {}; detached", crate::jobs::STOP_GRACE.as_secs(), stop.name())}),
                "abandoned",
            ),
        };
        if state != "finished" {
            result["stopped"] = json!(state);
        }
        result["elapsed_ms"] = json!(elapsed_ms);
        self.last_job = Some(
            json!({"kind":"scenario","label":path,"state":state,"done":done,"total":total,"elapsed_ms":elapsed_ms}),
        );
        self.set_scenario_result(&path, result);
        let seconds = elapsed_ms as f64 / 1000.0;
        if state == "finished" {
            self.status = format!("{} · {seconds:.1} s", self.status);
        } else {
            self.set_error(format!(
                "Scenario {state} at tick {done}/{total} · {path} · {seconds:.1} s"
            ));
        }
    }

    /// Running background jobs for the status bar and `observe`.
    pub(crate) fn jobs_observation(&self) -> Value {
        json!(self
            .scenario_job
            .iter()
            .map(|job| job.observation())
            .chain(self.asset_job_observation())
            .chain(self.tuning.shader.job.as_ref().map(|j| j.observation()))
            .chain(self.merge_jobs_observation())
            .collect::<Vec<_>>())
    }

    pub fn set_proposals(&mut self, signature: (i64, i64), rows: &Value) {
        let previous = self.proposals.len();
        self.proposals = rows.as_array().cloned().unwrap_or_default();
        self.proposal_lines = self
            .proposals
            .iter()
            .map(|p| diff_view::lines(&p["diff"]))
            .collect();
        self.proposal_signature = Some(signature);
        if self
            .proposal_focus
            .is_none_or(|id| !self.proposals.iter().any(|p| p["id"] == id))
        {
            self.proposal_focus = self.proposals.first().and_then(|p| p["id"].as_i64());
        }
        // Ghosts follow the focused proposal (a resolved one disappears) unless a History
        // revision is shown.
        if !self.ghost.off && !matches!(self.ghost.source, Some(ghost::GhostSource::Revision(_))) {
            self.set_ghost_source(self.proposal_focus.map(ghost::GhostSource::Proposal));
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
        self.history_lines = diff_view::lines(&diff);
        self.history_diff = Some((rev, diff));
        self.show_dock(Dock::History);
    }

    /// Opens `tab` if it is closed and makes it the active, focused tab. While the dock is
    /// being drawn (its state is borrowed) the request waits until the end of the frame.
    fn show_dock(&mut self, tab: Dock) {
        match &mut self.layout.dock {
            Some(state) => layout::show(state, tab),
            None => self.layout.pending.push(tab),
        }
    }

    /// Whether `tab` is the active tab of its leaf (shown unless its leaf is collapsed).
    #[cfg(test)]
    fn dock_shows(&self, tab: Dock) -> bool {
        self.layout
            .dock
            .as_ref()
            .is_some_and(|state| layout::is_active(state, tab))
    }

    /// Records a failed request as a structured problem and shows it in the status line.
    pub fn problem(&mut self, error: &anyhow::Error) {
        self.open_component_after = None;
        let response = axiom_core::error_response(error);
        let d = response["diagnostics"][0].clone();
        let entity = d["path"]
            .as_str()
            .and_then(|path| entity_in_path(path, &self.world));
        self.problems.insert(0, d);
        self.problems.truncate(32);
        // One Console entry, linked to the object the diagnostic points at.
        let text = format!("{error:#}");
        self.console_log(debug::Level::Error, &text, entity);
        self.error = true;
        self.status = text;
    }

    /// Codes of the recorded problems, newest first (tests).
    #[cfg(test)]
    pub(crate) fn problem_codes(&self) -> Vec<String> {
        self.problems
            .iter()
            .map(|p| p["code"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    pub fn set_component_hints(&mut self, registry: &axiom_core::Registry) {
        self.component_hints = registry
            .components
            .iter()
            .map(|(id, d)| (id.clone(), d.schema.clone()))
            .collect();
        self.registry = std::sync::Arc::new(registry.clone());
        self.world_panel.invalidate();
        self.specs.clear();
    }

    pub fn dirty(&self) -> bool {
        self.draft.as_ref().is_some_and(Draft::dirty)
            || self.resource.as_ref().is_some_and(|draft| draft.dirty)
            || self.material.as_ref().is_some_and(|d| d.dirty())
            || self.prefab.as_ref().is_some_and(|d| d.dirty())
    }

    pub fn reset_draft(&mut self) {
        // The stale draft is gone: nothing left to merge.
        self.merge_base = None;
        if self.merge.source == Some(merge::MergeSource::Draft) && self.merge.write.is_none() {
            self.close_merge();
        }
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
        let selected = entity.map(|e| e.id.clone());
        // Reloading the same object (after a live commit, Reload, Esc) keeps its open
        // component and Form / JSON choice.
        let keep = self
            .draft
            .as_ref()
            .filter(|d| selected.as_ref() == Some(&d.id) && !d.component.is_empty())
            .map(|d| (d.component.clone(), d.json_mode));
        if selected != self.selected {
            self.scene_input.path_point = None;
            self.tween_ui = Default::default();
            self.picker = None;
        }
        self.selected = selected;
        self.drag = None;
        self.scene_input.point_drag = None;
        self.scroll_selection = true;
        let open = self
            .open_component_after
            .take()
            .filter(|c| entity.is_some_and(|e| self.world.resolved_components(e).contains_key(c)));
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
        let resolves =
            |c: &String| entity.is_some_and(|e| ui::component_value(&self.world, e, c).is_some());
        let reopen = open
            .map(|c| (c, None))
            .or_else(|| keep.map(|(c, json)| (c, Some(json))))
            .filter(|(c, _)| resolves(c));
        if let (Some((component, json)), Some(draft), Some(e)) =
            (reopen, self.draft.as_mut(), entity)
        {
            draft.open_component(&self.world, e, &component);
            if let Some(json) = json {
                draft.json_mode = json || draft.form.is_none();
            }
        }
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
        self.debug.console.log(
            debug::Level::Error,
            debug::Source::Editor,
            &self.status,
            None,
            self.previewing.then_some(self.play.tick),
        );
    }

    pub fn notice(&mut self, message: impl Into<String>) {
        self.error = false;
        self.status = message.into();
        self.debug.console.log(
            debug::Level::Info,
            debug::Source::Editor,
            &self.status,
            None,
            self.previewing.then_some(self.play.tick),
        );
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

    /// Revision an undo (or redo) restores now, if any.
    pub(crate) fn restore_target(&self, redo: bool) -> Option<i64> {
        if redo {
            self.redo.last().copied()
        } else {
            self.undo.last().copied()
        }
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
        let (play, game) = self.play_observation();
        json!({
            "play":play,
            "game":game,
            "mode":if self.previewing {"play"} else {"edit"},
            "authoring_rev":self.rev,
            "selected":self.selected,
            "resource":self.resource.as_ref().map(|draft|&draft.id),
            "scene":self.scene,
            "draft_rev":self.resource.as_ref().map(|draft|draft.rev).or_else(||self.draft.as_ref().map(|draft| draft.rev)),
            "draft_dirty":self.dirty(),
            "tool":self.tool.name(),
            "view":self.orbit.mode.name(),
            "gizmos":self.prefs.gizmos,
            "tabs":self.layout.observed(),
            "scene_visible":self.scene_visible,
            "pending_proposals":self.proposals.len(),
            "hidden":self.prefs.hidden.get(&self.world.id),
            "locked":self.prefs.locked.get(&self.world.id),
            "selection":self.selection(),
            "jobs":self.jobs_observation(),
            "last_job":self.last_job,
            "thumbnails":self.assets.thumbs.observation(),
            "pending_commits":self.saving,
            "last_commit_ms":self.writes.get("last_commit_ms"),
            "writes":self.writes,
            "graph":self.graph_observation(),
            "ui_editor":{"zoom":(self.ui_editor.zoom() * 1000.0).round() / 1000.0},
            "debug":self.debug.observation(),
            "tuning":self.tuning_observation(),
            "ghosts":self.ghost_observation(),
            "merge":self.merge_observation(),
            "look":self.look_observation(),
            "vram":match &self.vram {
                Some(crate::vram::VramReading::Bytes { local, shared, budget }) => json!({"source":"dxgi","local_bytes":local,"shared_bytes":shared,"budget_bytes":budget}),
                Some(crate::vram::VramReading::Unavailable(why)) => json!({"available":false,"reason":why}),
                None => Value::Null,
            }
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

    /// The draft whose transform the scene view shows instead of the runtime's: while it is
    /// being dragged or typed, while the runtime for the editor's newest revision is still
    /// being built in the background, and while queued writes are saving (the editor world
    /// is then their optimistic result). No snap-back after a gizmo commit.
    fn shown_draft(&self, runtime: &WorldRuntime) -> Option<&Draft> {
        let pending = runtime.rev != self.rev && runtime.requested_rev == self.rev;
        self.draft.as_ref().filter(|d| {
            (d.transform_dirty && d.rev == runtime.rev)
                || ((pending || self.saving > 0) && d.rev == self.rev)
        })
    }

    /// Swaps the Inspector draft of a `render.light` / `render.mesh` being edited into the
    /// compiled entity for one frame; returns how to put the committed value back. Only a
    /// valid draft on the runtime's revision is shown.
    #[allow(clippy::type_complexity)]
    fn draft_component_preview(
        &self,
        runtime: &mut WorldRuntime,
    ) -> Option<Box<dyn FnOnce(&mut WorldRuntime)>> {
        let d = self
            .draft
            .as_ref()
            .filter(|d| d.form_dirty && d.rev == runtime.rev)?;
        let form = d.form.as_ref()?;
        let &slot = runtime.state.compiled.indices.get(&d.id)?;
        let entity = &mut runtime.state.compiled.entities[slot];
        match d.component.as_str() {
            "render.light" => {
                let light: axiom_core::light::RenderLight =
                    serde_json::from_value(form.clone()).ok()?;
                light.validate().ok()?;
                let old = entity.light.replace(light);
                Some(Box::new(move |rt: &mut WorldRuntime| {
                    rt.state.compiled.entities[slot].light = old;
                }))
            }
            "render.mesh" => {
                let mesh: axiom_core::RenderMesh = serde_json::from_value(form.clone()).ok()?;
                mesh.validate().ok()?;
                let old = entity.mesh.replace(mesh);
                Some(Box::new(move |rt: &mut WorldRuntime| {
                    rt.state.compiled.entities[slot].mesh = old;
                }))
            }
            _ => None,
        }
    }

    pub fn render_frame(
        &self,
        runtime: &mut WorldRuntime,
        screen: egui::Rect,
    ) -> crate::presentation::RenderFrame {
        // Scene and Game tabs closed, behind other tabs or collapsed, and no camera preview:
        // nothing shows a world view, so skip extraction and the world passes (the renderer
        // only draws the interface).
        if !self.world_visible() {
            self.game.views.set(0);
            self.game.dropped.set(0);
            return runtime.interface_frame();
        }
        // While the runtime for the editor's newest revision is still being built in the
        // background, show the selected object where that revision put it (no snap-back
        // after a gizmo commit). Play shows the simulation as it is.
        let draft = self.shown_draft(runtime).filter(|_| !self.previewing);
        // The override only lives during extraction, so all descendants and native mesh
        // extractors see the draft, while the simulation retains the committed transform.
        let original = draft.and_then(|d| {
            let &slot = runtime.state.compiled.indices.get(&d.id)?;
            let transform = d.transform.clone()?;
            transform.validate().ok()?;
            Some((slot, runtime.state.transforms[slot].replace(transform)))
        });
        // Gameplay may choose a different scene during its initial entry actions. While playing,
        // the game's own (scene-switched) visibility is what the views show.
        let mut visibility = if self.previewing {
            runtime.state.compiled.enabled.clone()
        } else {
            runtime.authored_visibility.clone()
        };
        visibility.resize(runtime.state.compiled.len(), false);
        // Editor-view "hidden" objects (and their descendants) are not drawn in the Scene view;
        // play is unaffected. The views share one extraction, so a Game view shown at the same
        // time as the Scene view hides them too.
        let hidden = if self.scene_visible {
            self.view_flagged(false)
        } else {
            BTreeSet::new()
        };
        for id in &hidden {
            if let Some(&slot) = runtime.state.compiled.indices.get(id) {
                visibility[slot] = false;
            }
        }
        let original_visibility =
            std::mem::replace(&mut runtime.state.compiled.enabled, visibility);
        // Timeline scrub: the selected clip at the playhead, for this frame only (edit mode;
        // nothing is written to the revision or kept in the simulation).
        let scrub = self
            .tween_target()
            .filter(|_| self.tween_ui.preview && !self.previewing)
            .and_then(|(id, clip, _)| {
                let writes = runtime.system::<crate::tween::TweenSystem>()?.preview(
                    &id,
                    &clip,
                    self.tween_ui.time,
                )?;
                Some(crate::tween::TweenSystem::apply_preview(
                    &mut runtime.state,
                    &writes,
                ))
            });
        // Animation tab scrub: the model posed at the playhead for this frame only; the
        // simulation's controller is put back after extraction.
        let pose = self.anim_scrub_target().and_then(|(id, clip, time)| {
            runtime
                .system_mut::<crate::animation::AnimationSystem>()?
                .begin_scrub(id, clip, time)
        });
        // A light / mesh / camera field being dragged or typed in the Inspector: its draft
        // value is drawn for this frame only (other components show once committed).
        let component = if self.previewing {
            None
        } else {
            self.draft_component_preview(runtime)
        };
        let mut frame = runtime.render_frame();
        if let Some(restore) = component {
            restore(runtime);
        }
        if let Some(restore) = scrub {
            restore.restore(&mut runtime.state);
        }
        if let Some(pose) = pose {
            if let Some(system) = runtime.system_mut::<crate::animation::AnimationSystem>() {
                system.end_scrub(pose);
            }
        }
        runtime.state.compiled.enabled = original_visibility;
        // VFX live in a resource, not on entities: drop the emitters attached to hidden objects.
        // (Meshes, procedural geometry and skinned models already skip disabled slots.)
        if !hidden.is_empty() {
            let gone = |a: &Option<String>| a.as_ref().is_some_and(|id| hidden.contains(id));
            frame.vfx.batches.retain(|b| !gone(&b.emitter.attachment));
            frame.vfx.ribbons.retain(|r| !gone(&r.attachment));
            frame.lights.retain(|l| !gone(&l.attachment));
        }
        if let Some((slot, transform)) = original {
            runtime.state.transforms[slot] = transform;
        }
        // The runtime's camera views go to the Game tab; the Scene tab gets the editor camera.
        let game = std::mem::take(&mut frame.views);
        if self.scene_visible {
            frame.views.push(self.orbit.view(self.viewport, screen));
        }
        self.compose_views(&mut frame, game, runtime, screen);
        if frame.views.is_empty() {
            // E.g. a Game tab without an active camera: no world pass (no fallback view over
            // the whole window).
            frame.draw_world = false;
        }
        frame
    }
}
