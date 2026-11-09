//! Editor command registry. One table drives the menu bar, the command palette, keyboard
//! shortcuts and the bridge (`editor_view {command}`), so a person and an agent trigger exactly
//! the same operations through the same validators.
use super::viewport::{Bookmark, Orbit, ViewMode, FLY_MAX, FLY_MIN};
use super::{layout, Action, Dock, Editor, Tool};
use egui::{Key, RichText};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub menu: &'static str,
    /// Default shortcut(s), `|`-separated, e.g. `Ctrl+Y|Ctrl+Shift+Z`. Empty for none.
    pub keys: &'static str,
    /// The argument the command takes (`editor_view {command, arg}`), empty for none. From
    /// the menu or palette such a command opens the UI where the person picks the argument.
    pub arg: &'static str,
}
const fn c(
    id: &'static str,
    title: &'static str,
    menu: &'static str,
    keys: &'static str,
) -> Command {
    Command {
        id,
        title,
        menu,
        keys,
        arg: "",
    }
}
/// A command with an argument (no default shortcut).
const fn a(
    id: &'static str,
    title: &'static str,
    menu: &'static str,
    arg: &'static str,
) -> Command {
    Command {
        id,
        title,
        menu,
        keys: "",
        arg,
    }
}
pub(crate) const MENUS: [&str; 8] = [
    "File", "Edit", "View", "Object", "World", "Play", "Window", "Help",
];
pub(crate) const COMMANDS: &[Command] = &[
    c("file.verify", "Verify World", "File", ""),
    c(
        "file.copy_context",
        "Copy Selection Context for AI",
        "File",
        "Ctrl+Shift+C",
    ),
    c("file.exit", "Exit", "File", ""),
    c("edit.undo", "Undo", "Edit", "Ctrl+Z"),
    c("edit.redo", "Redo", "Edit", "Ctrl+Y|Ctrl+Shift+Z"),
    c("edit.copy", "Copy Object IDs", "Edit", "Ctrl+C"),
    c("edit.paste", "Paste (Duplicate Copied)", "Edit", "Ctrl+V"),
    c("edit.duplicate", "Duplicate", "Edit", "Ctrl+D"),
    c("edit.delete", "Delete", "Edit", "Delete"),
    c("edit.select_none", "Select None", "Edit", "Escape"),
    c("edit.rename", "Rename", "Edit", "F2"),
    c("edit.select_children", "Select Children", "Edit", ""),
    c("edit.frame", "Frame Selection", "Edit", "F"),
    c(
        "edit.toggle_hidden",
        "Hide / Show in Editor View",
        "Edit",
        "H",
    ),
    c(
        "edit.toggle_locked",
        "Lock / Unlock in Editor View",
        "Edit",
        "L",
    ),
    c("edit.preferences", "Preferences…", "Edit", "Ctrl+Comma"),
    c("tool.select", "Select Tool", "View", "Q"),
    c("tool.move", "Move Tool", "View", "W"),
    c("tool.rotate", "Rotate Tool", "View", "E"),
    c("tool.scale", "Scale Tool", "View", "R"),
    c("view.space", "Toggle Global / Local Axes", "View", "X"),
    c("view.snap", "Toggle Snap", "View", ""),
    c("view.grid", "Toggle Grid", "View", ""),
    c("view.palette", "Command Palette…", "View", "Ctrl+Shift+P"),
    c("view.gizmos", "Toggle Component Gizmos", "View", ""),
    c(
        "view.nav_grid",
        "Toggle Navigation Grid Overlay",
        "View",
        "",
    ),
    c("view.perspective", "Perspective View", "View", "Alt+1"),
    c("view.top", "Top View (Ortho)", "View", "Alt+2"),
    c("view.front", "Front View (Ortho)", "View", "Alt+3"),
    c("view.right", "Right View (Ortho)", "View", "Alt+4"),
    c("view.bottom", "Bottom View (Ortho)", "View", "Alt+5"),
    c("view.back", "Back View (Ortho)", "View", "Alt+6"),
    c("view.left", "Left View (Ortho)", "View", "Alt+7"),
    c(
        "view.bookmark.save.1",
        "Save Camera Bookmark 1",
        "View",
        "Ctrl+F6",
    ),
    c(
        "view.bookmark.save.2",
        "Save Camera Bookmark 2",
        "View",
        "Ctrl+F7",
    ),
    c(
        "view.bookmark.save.3",
        "Save Camera Bookmark 3",
        "View",
        "Ctrl+F8",
    ),
    c(
        "view.bookmark.save.4",
        "Save Camera Bookmark 4",
        "View",
        "Ctrl+F9",
    ),
    c(
        "view.bookmark.recall.1",
        "Recall Camera Bookmark 1",
        "View",
        "F6",
    ),
    c(
        "view.bookmark.recall.2",
        "Recall Camera Bookmark 2",
        "View",
        "F7",
    ),
    c(
        "view.bookmark.recall.3",
        "Recall Camera Bookmark 3",
        "View",
        "F8",
    ),
    c(
        "view.bookmark.recall.4",
        "Recall Camera Bookmark 4",
        "View",
        "F9",
    ),
    c("object.empty", "Create Empty", "Object", "Ctrl+Shift+N"),
    c("object.cube", "Create Cube", "Object", ""),
    c("object.camera", "Create Camera", "Object", ""),
    c(
        "object.light_directional",
        "Create Directional Light",
        "Object",
        "",
    ),
    c("object.light_point", "Create Point Light", "Object", ""),
    c("object.light_spot", "Create Spot Light", "Object", ""),
    c("object.light_area", "Create Area Light", "Object", ""),
    c("object.trigger_box", "Create Box Trigger", "Object", ""),
    c(
        "object.trigger_sphere",
        "Create Sphere Trigger",
        "Object",
        "",
    ),
    c("object.path", "Create Path", "Object", ""),
    c("object.follow_path", "Follow Path", "Object", ""),
    c("path.add_point", "Add Path Point", "Object", ""),
    c("path.remove_point", "Remove Path Point", "Object", ""),
    c("object.obstacle", "Create Obstacle", "Object", ""),
    c("object.nav_agent", "Add Nav Agent", "Object", ""),
    c(
        "nav.target_object",
        "Nav Target: Selected Object",
        "Object",
        "",
    ),
    c(
        "nav.target_point",
        "Nav Target: Click Ground…",
        "Object",
        "",
    ),
    c("nav.clear_target", "Nav Target: Clear", "Object", ""),
    c("object.interactable", "Make Interactable", "Object", ""),
    c("object.audio_source", "Add Audio Source", "Object", ""),
    c("object.audio_listener", "Make Audio Listener", "Object", ""),
    c(
        "object.camera_follow",
        "Camera Follow Selected",
        "Object",
        "",
    ),
    c("object.main_camera", "Set as Main Camera", "Object", ""),
    c("object.tween_clip", "Add Tween Clip", "Object", ""),
    c(
        "tween.record_key",
        "Record Tween Key at Playhead",
        "Object",
        "",
    ),
    c("tween.remove_key", "Remove Tween Key", "Object", ""),
    c("object.child_empty", "Create Child Empty", "Object", ""),
    c("object.child_cube", "Create Child Cube", "Object", ""),
    c("object.child_camera", "Create Child Camera", "Object", ""),
    c(
        "object.create_prefab",
        "Create Prefab From Selection",
        "Object",
        "",
    ),
    c("object.drop_to_ground", "Drop to Ground", "Object", "End"),
    a("object.tag", "Edit Tags…", "Object", "+TAG -TAG …"),
    a(
        "object.add_component",
        "Add Component…",
        "Object",
        "COMPONENT",
    ),
    c("window.world", "World Settings…", "World", "Ctrl+9"),
    a("world.set_id", "Set World ID…", "World", "ID"),
    a("world.set_title", "Set World Title…", "World", "TEXT"),
    a(
        "world.capability_add",
        "Add Capability…",
        "World",
        "CAPABILITY",
    ),
    a(
        "world.capability_remove",
        "Remove Capability…",
        "World",
        "CAPABILITY",
    ),
    a("world.system_add", "Add System…", "World", "SYSTEM"),
    a("world.system_remove", "Remove System…", "World", "SYSTEM"),
    a(
        "world.resource_create",
        "Create Resource…",
        "World",
        "RESOURCE [JSON]",
    ),
    a(
        "world.resource_remove",
        "Remove Resource…",
        "World",
        "RESOURCE",
    ),
    a("event.declare", "Declare Event…", "World", "NAME"),
    c("play.start", "Play", "Play", "F5"),
    c("window.hierarchy", "Hierarchy", "Window", ""),
    c("window.scene", "Scene", "Window", ""),
    c("window.inspector", "Inspector", "Window", ""),
    c("window.changes", "Changes", "Window", "Ctrl+1"),
    c("window.proposals", "Proposals", "Window", "Ctrl+2"),
    c("window.history", "History", "Window", "Ctrl+3"),
    c("window.problems", "Problems", "Window", "Ctrl+4"),
    c("window.scenarios", "Scenarios", "Window", "Ctrl+5"),
    c("window.graphs", "Graphs", "Window", "Ctrl+6"),
    c("window.assets", "Assets", "Window", "Ctrl+7"),
    c("window.agent", "AI / Protocol", "Window", "Ctrl+8"),
    a("window.close", "Close Tab", "Window", "TAB"),
    a("layout.save_as", "Save Layout As…", "Window", "NAME"),
    a("layout.load", "Load Layout…", "Window", "NAME"),
    a("layout.delete", "Delete Saved Layout…", "Window", "NAME"),
    c("layout.reset", "Reset Layout", "Window", ""),
    c("window.ui_editor", "UI Editor", "Window", ""),
    a("ui.select", "UI Editor: Select Widget…", "Window", "WIDGET"),
    a(
        "ui.locale",
        "UI Editor: Preview Locale…",
        "Window",
        "LOCALE",
    ),
    a(
        "ui.resolution",
        "UI Editor: Reference Resolution…",
        "Window",
        "WxH",
    ),
    c(
        "ui.preview_effects",
        "UI Editor: Preview Effects of Selected Widget",
        "Window",
        "",
    ),
    a(
        "ui.place",
        "UI: Place Widget in Region…",
        "World",
        "WIDGET REGION|none",
    ),
    a(
        "ui.style",
        "UI: Widget Style…",
        "World",
        "WIDGET STYLE|none",
    ),
    a(
        "ui.button",
        "UI: Button Command…",
        "World",
        "WIDGET emit EVENT|locale L|choose NODE CHOICE|call CAP ACTION [JSON]",
    ),
    a(
        "ui.effect_add",
        "UI: Add Effect…",
        "World",
        "ID WIDGET KIND [key=value ..]",
    ),
    a(
        "ui.effect_set",
        "UI: Edit Effect…",
        "World",
        "ID key=value ..",
    ),
    a("ui.effect_remove", "UI: Remove Effect…", "World", "ID"),
    c("help.shortcuts", "Keyboard Shortcuts…", "Help", "F1"),
    c(
        "help.copy_discovery",
        "Copy Editor Discovery for AI",
        "Help",
        "",
    ),
];
pub(crate) fn command(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.id == id)
}

/// A parsed shortcut. `ctrl` means the platform command key (Ctrl on Windows/Linux).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shortcut {
    ctrl: bool,
    shift: bool,
    alt: bool,
    key: Key,
}
impl Shortcut {
    pub fn parse(text: &str) -> Option<Self> {
        let mut s = Shortcut {
            ctrl: false,
            shift: false,
            alt: false,
            key: Key::Escape,
        };
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "command" => s.ctrl = true,
                "shift" => s.shift = true,
                "alt" => s.alt = true,
                _ => key = Some(Key::from_name(part)?),
            }
        }
        s.key = key?;
        Some(s)
    }
    fn pressed(&self, i: &egui::InputState) -> bool {
        i.key_pressed(self.key)
            && (i.modifiers.command || i.modifiers.ctrl) == self.ctrl
            && i.modifiers.shift == self.shift
            && i.modifiers.alt == self.alt
    }
}

/// Per-user editor preferences. Stored next to the database, never in world revisions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Prefs {
    pub snap: bool,
    pub snap_move: f32,
    pub snap_rotate_degrees: f32,
    pub snap_scale: f32,
    pub grid: bool,
    pub local_space: bool,
    /// Active tab of the panel holding the old bottom-dock tabs. Older builds only stored this;
    /// without `layout` it is focused in the Default layout.
    pub dock: String,
    /// Dock layout (`editor::layout` spec v1); null = Default. Kept as raw JSON so a bad
    /// layout falls back on its own instead of discarding the whole preferences file.
    pub layout: serde_json::Value,
    /// Layouts saved with Window ▸ Save Layout As (name -> spec).
    pub layouts: BTreeMap<String, serde_json::Value>,
    /// Command ID -> replacement shortcut(s) (`|`-separated; empty string unbinds).
    pub shortcuts: BTreeMap<String, String>,
    /// Editor-view only, per world ID: objects not drawn or picked in the scene view.
    pub hidden: BTreeMap<String, BTreeSet<String>>,
    /// Editor-view only, per world ID: objects drawn but not pickable or draggable there.
    pub locked: BTreeMap<String, BTreeSet<String>>,
    /// Scene-view fly speed (m/s) for RMB + WASD/QE; the wheel changes it while flying.
    pub fly_speed: f32,
    /// Draw component gizmos (camera frusta, nav.blocker, colliders, FSM agents).
    pub gizmos: bool,
    /// Scene view: outline the navigation grid cells the selected agent cannot enter.
    pub nav_grid: bool,
    /// Per world ID: saved scene-view cameras by slot ("1".."4").
    pub bookmarks: BTreeMap<String, BTreeMap<String, Bookmark>>,
    /// Inspector edits commit when a drag is released, Enter is pressed or a field loses
    /// focus (one revision per interaction); off = draft + Apply (batch mode).
    pub live_inspector: bool,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            snap: false,
            snap_move: 0.25,
            snap_rotate_degrees: 15.0,
            snap_scale: 0.1,
            grid: false,
            local_space: false,
            dock: "changes".into(),
            layout: serde_json::Value::Null,
            layouts: BTreeMap::new(),
            shortcuts: BTreeMap::new(),
            hidden: BTreeMap::new(),
            locked: BTreeMap::new(),
            fly_speed: 6.0,
            gizmos: true,
            nav_grid: false,
            bookmarks: BTreeMap::new(),
            live_inspector: true,
        }
    }
}
impl Prefs {
    fn sanitized(mut self) -> Self {
        let clamp = |v: f32, lo: f32, hi: f32, default: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        self.snap_move = clamp(self.snap_move, 0.001, 100.0, 0.25);
        self.snap_rotate_degrees = clamp(self.snap_rotate_degrees, 0.1, 180.0, 15.0);
        self.snap_scale = clamp(self.snap_scale, 0.001, 10.0, 0.1);
        self.fly_speed = clamp(self.fly_speed, FLY_MIN, FLY_MAX, 6.0);
        self.bookmarks.retain(|world, slots| {
            slots.retain(|slot, b| matches!(slot.as_str(), "1" | "2" | "3" | "4") && b.valid());
            world.len() <= 128 && !slots.is_empty()
        });
        while self.bookmarks.len() > 64 {
            self.bookmarks.pop_first();
        }
        self.shortcuts
            .retain(|id, keys| command(id).is_some() && keys.len() <= 64);
        layout::sanitize_named(&mut self.layouts);
        for sets in [&mut self.hidden, &mut self.locked] {
            sets.retain(|world, ids| world.len() <= 128 && !ids.is_empty() && ids.len() <= 4096);
            while sets.len() > 64 {
                sets.pop_first();
            }
        }
        self
    }
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Prefs>(&raw).ok())
            .unwrap_or_default()
            .sanitized()
    }
}

#[derive(Default)]
pub(crate) struct Palette {
    pub query: String,
    pub index: usize,
    pub opened: bool,
}

impl Editor {
    /// Shortcuts currently bound to `id` (user override first, then the default).
    pub(crate) fn shortcuts(&self, id: &str) -> Vec<Shortcut> {
        let keys = self
            .prefs
            .shortcuts
            .get(id)
            .map(String::as_str)
            .or_else(|| command(id).map(|c| c.keys))
            .unwrap_or("");
        keys.split('|')
            .filter(|k| !k.trim().is_empty())
            .filter_map(Shortcut::parse)
            .collect()
    }
    pub(crate) fn shortcut_text(&self, id: &str) -> String {
        self.prefs
            .shortcuts
            .get(id)
            .map(String::as_str)
            .or_else(|| command(id).map(|c| c.keys))
            .unwrap_or("")
            .split('|')
            .next()
            .unwrap_or("")
            .to_owned()
    }

    /// Why a command cannot run right now, if it cannot.
    pub(crate) fn command_blocked(&self, id: &str) -> Option<&'static str> {
        let busy = self.dirty() || self.drag.is_some();
        let nothing = self.selected.is_none();
        match id {
            "edit.undo" if self.undo.is_empty() && self.saving == 0 => Some("nothing to undo"),
            "edit.redo" if self.redo.is_empty() && self.saving == 0 => Some("nothing to redo"),
            "edit.paste" if self.clipboard.is_empty() => Some("nothing copied"),
            "edit.duplicate"
            | "edit.delete"
            | "edit.copy"
            | "edit.frame"
            | "file.copy_context"
            | "edit.rename"
            | "edit.select_children"
            | "edit.toggle_hidden"
            | "edit.toggle_locked"
            | "object.child_empty"
            | "object.child_cube"
            | "object.child_camera"
            | "object.create_prefab"
            | "object.follow_path"
            | "object.nav_agent"
            | "object.interactable"
            | "object.audio_source"
            | "object.audio_listener"
            | "object.camera_follow"
            | "object.main_camera"
            | "object.tween_clip"
            | "object.tag"
            | "object.add_component"
                if nothing =>
            {
                Some("nothing selected")
            }
            "tween.record_key" | "tween.remove_key" if self.tween_target().is_none() => {
                Some("select an object with tween clips")
            }
            "tween.remove_key" if self.tween_ui.key.is_none() => {
                Some("select a key on the Inspector timeline")
            }
            "tween.record_key" | "tween.remove_key" if self.previewing => {
                Some("stop Play first (keys edit the authored clip)")
            }
            "path.add_point" | "path.remove_point"
                if self
                    .selected
                    .as_deref()
                    .and_then(|id| self.path_spline(id))
                    .is_none() =>
            {
                Some("select a path")
            }
            "nav.target_object" | "nav.target_point" | "nav.clear_target"
                if self.selected_agents().is_empty() =>
            {
                Some("select a nav agent")
            }
            "play.start" if self.previewing => Some("already playing"),
            "edit.undo"
            | "edit.redo"
            | "edit.paste"
            | "edit.duplicate"
            | "edit.delete"
            | "edit.select_none"
            | "object.empty"
            | "object.cube"
            | "object.camera"
            | "object.light_directional"
            | "object.light_point"
            | "object.light_spot"
            | "object.light_area"
            | "object.trigger_box"
            | "object.trigger_sphere"
            | "object.path"
            | "object.follow_path"
            | "path.add_point"
            | "path.remove_point"
            | "object.obstacle"
            | "object.nav_agent"
            | "nav.target_object"
            | "nav.target_point"
            | "nav.clear_target"
            | "object.interactable"
            | "object.audio_source"
            | "object.audio_listener"
            | "object.camera_follow"
            | "object.main_camera"
            | "object.tween_clip"
            | "tween.record_key"
            | "tween.remove_key"
            | "play.start"
            | "edit.rename"
            | "object.child_empty"
            | "object.child_cube"
            | "object.child_camera"
            | "object.create_prefab"
            | "object.tag"
            | "object.add_component"
            | "world.set_id"
            | "world.set_title"
            | "world.capability_add"
            | "world.capability_remove"
            | "world.system_add"
            | "world.system_remove"
            | "world.resource_create"
            | "world.resource_remove"
            | "event.declare"
                if busy =>
            {
                Some("apply or reload the inspector draft first")
            }
            "tool.select" | "tool.move" | "tool.rotate" | "tool.scale" | "view.space"
            | "edit.toggle_hidden" | "edit.toggle_locked" | "window.close" | "window.scene"
            | "layout.load" | "layout.reset"
                if self.drag.is_some() =>
            {
                Some("finish the current drag first")
            }
            "object.drop_to_ground" if nothing => Some("nothing selected"),
            "object.drop_to_ground" if busy => Some("apply or reload the inspector draft first"),
            id if self.drag.is_some()
                && (ViewMode::from_command(id).is_some()
                    || id.starts_with("view.bookmark.recall.")) =>
            {
                Some("finish the current drag first")
            }
            id if id.starts_with("view.bookmark.recall.") && self.bookmark(id).is_none() => {
                Some("no camera bookmark saved in this slot")
            }
            "ui.preview_effects" if self.ui_editor.selected.is_none() => {
                Some("select a widget in the UI Editor")
            }
            "ui.place" | "ui.style" | "ui.button" | "ui.effect_add" | "ui.effect_set"
            | "ui.effect_remove"
                if busy =>
            {
                Some("apply or reload the inspector draft first")
            }
            _ => None,
        }
    }

    /// The saved camera for a `view.bookmark.*.N` command in this world.
    fn bookmark(&self, id: &str) -> Option<&Bookmark> {
        let slot = id.rsplit('.').next()?;
        self.prefs.bookmarks.get(&self.world.id)?.get(slot)
    }

    /// Runs a registered command. Unknown or blocked commands are errors, never no-ops.
    pub(crate) fn run_command(&mut self, id: &str) -> anyhow::Result<Vec<Action>> {
        self.run_command_with(id, None)
    }

    /// Runs a registered command with its argument (`editor_view {command, arg}`). A command
    /// that takes an argument but gets none opens the UI where a person picks it.
    pub(crate) fn run_command_with(
        &mut self,
        id: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Vec<Action>> {
        let Some(cmd) = command(id) else {
            anyhow::bail!("unknown editor command {id}; see discover editor (commands)");
        };
        if let Some(reason) = self.command_blocked(id) {
            anyhow::bail!("{id} is unavailable: {reason}");
        }
        if id == "window.close" || id.starts_with("layout.") {
            anyhow::ensure!(
                !cmd.arg.is_empty() || arg.is_none(),
                "{id} takes no argument"
            );
            self.layout_command(id, arg)?;
            return Ok(vec![]);
        }
        if cmd.arg.is_empty() {
            anyhow::ensure!(arg.is_none(), "{id} takes no argument");
        } else if id.starts_with("ui.") {
            return Ok(self.ui_command(id, arg));
        } else {
            return Ok(self.run_with_arg(id, arg));
        }
        let mut actions = vec![];
        match id {
            // World keeps the old window toggle; the other window.* commands open / focus.
            "window.world" => {
                let state = self.layout.dock.as_mut().ok_or_else(layout::busy)?;
                if !layout::close(state, Dock::World) {
                    layout::show(state, Dock::World);
                }
            }
            "file.verify" => actions.push(Action::Verify),
            "file.copy_context" => {
                self.pending_copy = Some(self.context_text());
                self.notice(format!(
                    "Copied context for {} object(s)",
                    self.selection().len()
                ));
            }
            "file.exit" => actions.push(Action::Quit),
            "edit.undo" | "edit.redo" => actions.extend(self.restore_action(id == "edit.redo")),
            "edit.copy" => {
                self.clipboard = self.selection();
                self.pending_copy = Some(self.clipboard.join("\n"));
                self.notice(format!("Copied {} object ID(s)", self.clipboard.len()));
            }
            "edit.paste" => actions.extend(self.duplicate_ids(&self.clipboard.clone())),
            "edit.duplicate" => actions.extend(self.duplicate_ids(&self.selection())),
            "edit.delete" => actions.extend(self.delete_selection()),
            "edit.select_none" => {
                self.extra.clear();
                self.select(None);
            }
            "edit.frame" => self.frame_selection = true,
            "edit.rename" => self.begin_rename(),
            "edit.select_children" => self.select_children(),
            "edit.toggle_hidden" => self.toggle_view_flag(false),
            "edit.toggle_locked" => self.toggle_view_flag(true),
            "object.child_empty" => actions.extend(self.create_child("empty")),
            "object.child_cube" => actions.extend(self.create_child("cube")),
            "object.child_camera" => actions.extend(self.create_child("camera")),
            "object.create_prefab" => actions.extend(self.create_prefab_from_selection()),
            "edit.preferences" => self.show_prefs = true,
            "tool.select" => self.tool = Tool::Select,
            "tool.move" => self.tool = Tool::Move,
            "tool.rotate" => self.tool = Tool::Rotate,
            "tool.scale" => self.tool = Tool::Scale,
            "view.space" => self.prefs.local_space = !self.prefs.local_space,
            "view.snap" => self.prefs.snap = !self.prefs.snap,
            "view.grid" => self.prefs.grid = !self.prefs.grid,
            "view.palette" => {
                self.palette = Some(Palette::default());
            }
            "object.empty" => actions.extend(self.create_object("empty")),
            "object.cube" => actions.extend(self.create_object("cube")),
            "object.camera" => actions.extend(self.create_object("camera")),
            "object.light_directional" => actions.extend(self.create_object("light.directional")),
            "object.light_point" => actions.extend(self.create_object("light.point")),
            "object.light_spot" => actions.extend(self.create_object("light.spot")),
            "object.light_area" => actions.extend(self.create_object("light.area")),
            "object.trigger_box" => actions.extend(self.create_object("trigger.box")),
            "object.trigger_sphere" => actions.extend(self.create_object("trigger.sphere")),
            "object.path" => actions.extend(self.create_object("path")),
            "object.follow_path" => actions.extend(self.follow_selection(&Default::default())),
            "path.add_point" => actions.extend(self.add_path_point()),
            "path.remove_point" => actions.extend(self.remove_path_point()),
            "object.obstacle" => actions.extend(self.create_object("obstacle")),
            "object.nav_agent" => actions.extend(self.add_nav_agents(&Default::default())),
            "nav.target_object" => actions.extend(self.nav_target_selection()),
            "nav.target_point" => {
                self.scene_input.nav_pick = true;
                self.notice(
                    "Click the ground in the scene view to set the nav target (Esc cancels)",
                );
            }
            "nav.clear_target" => actions.extend(self.set_nav_target(None)),
            "object.interactable" => actions.extend(self.make_interactable(&Default::default())),
            "object.audio_source" => actions.extend(self.add_audio_source(&Default::default())),
            "object.audio_listener" => actions.extend(self.add_audio_listener()),
            "object.camera_follow" => {
                actions.extend(self.camera_follow_selection(&Default::default()))
            }
            "object.main_camera" => actions.extend(self.make_main_camera()),
            "object.tween_clip" => {
                let name = axiom_core::ops::free_clip_name(&self.world, &self.selection());
                actions.extend(self.add_tween_clip(&name, &Default::default()))
            }
            "tween.record_key" => actions.extend(self.record_tween_key()),
            "tween.remove_key" => actions.extend(self.remove_tween_key()),
            "view.nav_grid" => self.prefs.nav_grid = !self.prefs.nav_grid,
            "play.start" => actions.push(Action::Play),
            "ui.preview_effects" => self.ui_preview_effects(),
            "help.shortcuts" => self.show_shortcuts = true,
            "help.copy_discovery" => {
                self.pending_copy = Some(r#"{"op":"discover","topic":"editor"}"#.to_owned());
                self.notice("Copied editor discovery request");
            }
            "view.gizmos" => self.prefs.gizmos = !self.prefs.gizmos,
            "object.drop_to_ground" => actions.extend(self.drop_selection_to_ground()),
            id if ViewMode::from_command(id).is_some() => {
                let mode = ViewMode::from_command(id).unwrap();
                self.orbit.set_mode(mode);
                self.notice(format!("{} view", mode.name()));
            }
            id if id.starts_with("view.bookmark.save.") => {
                let slot = id.rsplit('.').next().unwrap_or("1").to_owned();
                let bookmark = self.orbit.bookmark();
                self.prefs
                    .bookmarks
                    .entry(self.world.id.clone())
                    .or_default()
                    .insert(slot.clone(), bookmark);
                self.notice(format!("Saved camera bookmark {slot}"));
            }
            id if id.starts_with("view.bookmark.recall.") => {
                if let Some(b) = self.bookmark(id) {
                    self.orbit = Orbit::from_bookmark(b);
                }
                self.notice(format!(
                    "Camera bookmark {}",
                    id.rsplit('.').next().unwrap_or("")
                ));
            }
            other => {
                let dock = other
                    .strip_prefix("window.")
                    .and_then(Dock::from_name)
                    .expect("every registered window command names a dock");
                self.show_dock(dock);
            }
        }
        Ok(actions)
    }

    /// `window.close` and `layout.*`. Errors (unknown tab / layout, bad name) are returned so
    /// the bridge reports them; from the menu they show in the status line.
    fn layout_command(&mut self, id: &str, arg: Option<&str>) -> anyhow::Result<()> {
        let arg = arg.map(str::trim);
        let state = self.layout.dock.as_mut().ok_or_else(layout::busy)?;
        match (id, arg) {
            ("window.close", None) => {
                let tab = layout::focused(state)
                    .ok_or_else(|| anyhow::anyhow!("no focused tab; name one, e.g. history"))?;
                layout::close(state, tab);
                self.notice(format!("Closed {} (Window menu reopens it)", tab.title()));
            }
            ("window.close", Some(name)) => {
                let tab = Dock::from_name(name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown tab {name}; use {}",
                        Dock::ALL.map(Dock::name).join("|")
                    )
                })?;
                anyhow::ensure!(layout::close(state, tab), "{name} is not open");
                self.notice(format!("Closed {} (Window menu reopens it)", tab.title()));
            }
            ("layout.reset", _) => {
                self.layout.replace(layout::default_state(), false);
                self.notice("Layout reset to Default");
            }
            ("layout.save_as", None) => self.layout.save_as = Some(String::new()),
            ("layout.save_as", Some(name)) => {
                layout::valid_name(name).map_err(anyhow::Error::msg)?;
                anyhow::ensure!(
                    !layout::BUILTIN.contains(&name),
                    "{name} is a built-in layout; pick another name"
                );
                anyhow::ensure!(
                    self.prefs.layouts.len() < 32 || self.prefs.layouts.contains_key(name),
                    "at most 32 saved layouts; delete one first"
                );
                self.prefs
                    .layouts
                    .insert(name.to_owned(), layout::to_spec(state));
                self.notice(format!("Saved layout {name}"));
            }
            ("layout.load", None) | ("layout.delete", None) => {
                self.notice("Pick a layout under Window ▸ Layouts");
            }
            ("layout.load", Some(name)) => {
                let spec = match layout::builtin(name) {
                    Some(spec) => spec,
                    None => self.prefs.layouts.get(name).cloned().ok_or_else(|| {
                        anyhow::anyhow!("no layout named {name}; see Window ▸ Layouts")
                    })?,
                };
                let (state, dropped) = layout::from_spec(&spec)
                    .map_err(|e| anyhow::anyhow!("layout {name} is unusable: {e}"))?;
                self.layout.replace(state, false);
                if dropped.is_empty() {
                    self.notice(format!("Layout {name}"));
                } else {
                    self.set_error(format!(
                        "Layout {name}: unknown panel(s) ignored: {}",
                        dropped.join(", ")
                    ));
                }
            }
            ("layout.delete", Some(name)) => {
                anyhow::ensure!(
                    self.prefs.layouts.remove(name).is_some(),
                    "no saved layout named {name} (built-in layouts cannot be deleted)"
                );
                self.notice(format!("Deleted layout {name}"));
            }
            _ => anyhow::bail!("unknown layout command {id}"),
        }
        Ok(())
    }

    /// Keyboard dispatch for every registered shortcut (Copy/Paste arrive as events instead).
    pub(crate) fn shortcut_commands(&self, i: &egui::InputState) -> Vec<&'static str> {
        COMMANDS
            .iter()
            .filter(|c| !matches!(c.id, "edit.copy" | "edit.paste"))
            .filter(|c| self.shortcuts(c.id).iter().any(|s| s.pressed(i)))
            .map(|c| c.id)
            .collect()
    }

    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        egui::menu::bar(ui, |ui| {
            for menu in MENUS {
                ui.menu_button(menu, |ui| {
                    ui.set_min_width(240.0);
                    for cmd in COMMANDS.iter().filter(|c| c.menu == menu) {
                        if matches!(
                            cmd.id,
                            "edit.preferences"
                                | "file.exit"
                                | "help.copy_discovery"
                                | "view.perspective"
                                | "view.bookmark.save.1"
                                | "object.tag"
                                | "world.set_id"
                                | "world.capability_add"
                                | "window.changes"
                                | "window.close"
                        ) {
                            ui.separator();
                        }
                        let blocked = self.command_blocked(cmd.id);
                        let checked = match cmd.id {
                            "view.snap" => Some(self.prefs.snap),
                            "view.grid" => Some(self.prefs.grid),
                            "view.space" => Some(self.prefs.local_space),
                            "view.gizmos" => Some(self.prefs.gizmos),
                            "view.nav_grid" => Some(self.prefs.nav_grid),
                            id if ViewMode::from_command(id).is_some() => {
                                Some(ViewMode::from_command(id) == Some(self.orbit.mode))
                            }
                            // ✓ = the tab is open (in the dock or a floating window).
                            id => id
                                .strip_prefix("window.")
                                .and_then(Dock::from_name)
                                .map(|d| {
                                    self.layout
                                        .dock
                                        .as_ref()
                                        .is_some_and(|s| layout::is_open(s, d))
                                }),
                        };
                        let title = match checked {
                            Some(true) => format!("✓ {}", cmd.title),
                            _ => cmd.title.to_owned(),
                        };
                        let response = ui
                            .add_enabled(
                                blocked.is_none(),
                                egui::Button::new(title).shortcut_text(self.shortcut_text(cmd.id)),
                            )
                            .on_disabled_hover_text(blocked.unwrap_or_default());
                        if response.clicked() {
                            self.invoke(cmd.id, actions);
                            ui.close_menu();
                        }
                    }
                    if menu == "Window" {
                        ui.menu_button("Layouts", |ui| {
                            let saved: Vec<String> = self.prefs.layouts.keys().cloned().collect();
                            for name in layout::BUILTIN
                                .iter()
                                .copied()
                                .chain(saved.iter().map(String::as_str))
                            {
                                if ui.button(name).clicked() {
                                    if let Err(e) = self.run_command_with("layout.load", Some(name))
                                    {
                                        self.set_error(format!("{e:#}"));
                                    }
                                    ui.close_menu();
                                }
                            }
                            if !saved.is_empty() {
                                ui.separator();
                                ui.menu_button("Delete", |ui| {
                                    for name in &saved {
                                        if ui.button(name).clicked() {
                                            if let Err(e) =
                                                self.run_command_with("layout.delete", Some(name))
                                            {
                                                self.set_error(format!("{e:#}"));
                                            }
                                            ui.close_menu();
                                        }
                                    }
                                });
                            }
                        });
                    }
                });
            }
        });
    }

    /// Runs a command from the UI, reporting failures in the status line.
    pub(crate) fn run_into(&mut self, id: &str, actions: &mut Vec<Action>) {
        match self.run_command(id) {
            Ok(more) => actions.extend(more),
            Err(e) => self.set_error(format!("{e:#}")),
        }
    }

    pub(crate) fn command_windows(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        self.options_window(ctx, actions);
        if let Some(mut palette) = self.palette.take() {
            let mut keep = true;
            let mut run = None;
            egui::Area::new(egui::Id::new("command_palette"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, 70.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(460.0);
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut palette.query)
                                .hint_text("Type a command…")
                                .desired_width(f32::INFINITY),
                        );
                        if !palette.opened {
                            edit.request_focus();
                            palette.opened = true;
                        }
                        let query = palette.query.to_lowercase();
                        let matches: Vec<&Command> = COMMANDS
                            .iter()
                            .filter(|c| {
                                query.split_whitespace().all(|t| {
                                    c.title.to_lowercase().contains(t)
                                        || c.id.contains(t)
                                        || c.menu.to_lowercase().contains(t)
                                })
                            })
                            .collect();
                        let (down, up, enter, escape) = ui.input(|i| {
                            (
                                i.key_pressed(Key::ArrowDown),
                                i.key_pressed(Key::ArrowUp),
                                i.key_pressed(Key::Enter),
                                i.key_pressed(Key::Escape),
                            )
                        });
                        if down {
                            palette.index =
                                (palette.index + 1).min(matches.len().saturating_sub(1));
                        }
                        if up {
                            palette.index = palette.index.saturating_sub(1);
                        }
                        palette.index = palette.index.min(matches.len().saturating_sub(1));
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for (n, cmd) in matches.iter().enumerate() {
                                    let blocked = self.command_blocked(cmd.id);
                                    let text =
                                        RichText::new(format!("{} › {}", cmd.menu, cmd.title));
                                    let response = ui
                                        .add_enabled(
                                            blocked.is_none(),
                                            egui::Button::new(text)
                                                .selected(n == palette.index)
                                                .shortcut_text(self.shortcut_text(cmd.id))
                                                .min_size(egui::vec2(ui.available_width(), 0.0)),
                                        )
                                        .on_disabled_hover_text(blocked.unwrap_or_default());
                                    if response.clicked() {
                                        run = Some(cmd.id);
                                    }
                                }
                                if matches.is_empty() {
                                    ui.label("No matching command");
                                }
                            });
                        if enter {
                            run = run.or_else(|| matches.get(palette.index).map(|c| c.id));
                        }
                        keep = !escape && run.is_none();
                    });
                });
            if let Some(id) = run {
                self.invoke(id, actions);
            }
            if keep && self.palette.is_none() {
                self.palette = Some(palette);
            }
        }
        if let Some(mut name) = self.layout.save_as.take() {
            let mut open = true;
            let mut save = false;
            egui::Window::new("Save Layout As")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 90.0])
                .show(ctx, |ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .hint_text("Layout name")
                            .desired_width(240.0),
                    );
                    if !edit.has_focus() && name.is_empty() {
                        edit.request_focus();
                    }
                    let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let taken = self.prefs.layouts.contains_key(name.trim());
                    if taken {
                        ui.label(RichText::new("Replaces the saved layout with this name").small());
                    }
                    save = ui
                        .add_enabled(layout::valid_name(&name).is_ok(), egui::Button::new("Save"))
                        .clicked()
                        || (entered && layout::valid_name(&name).is_ok());
                });
            if save {
                if let Err(e) = self.run_command_with("layout.save_as", Some(&name)) {
                    self.set_error(format!("{e:#}"));
                }
            } else if open && !ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.layout.save_as = Some(name);
            }
        }
        if self.show_shortcuts {
            let mut open = true;
            egui::Window::new("Keyboard Shortcuts")
                .open(&mut open)
                .default_width(420.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        egui::Grid::new("shortcut_table").striped(true).show(ui, |ui| {
                            for cmd in COMMANDS {
                                ui.label(RichText::new(cmd.menu).small());
                                ui.label(cmd.title);
                                ui.label(RichText::new(self.shortcut_text(cmd.id)).monospace());
                                ui.end_row();
                            }
                        });
                    });
                    ui.separator();
                    ui.label(RichText::new(match &self.prefs_path {
                        Some(path) => format!("Override in {} → \"shortcuts\": {{\"edit.duplicate\": \"Ctrl+J\"}}", path.display()),
                        None => "Preferences are not persisted in this session.".into(),
                    }).small());
                });
            self.show_shortcuts = open;
        }
        if self.show_prefs {
            let mut open = true;
            egui::Window::new("Preferences")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    egui::Grid::new("prefs_grid").num_columns(2).show(ui, |ui| {
                        ui.label("Snap move");
                        ui.add(
                            egui::DragValue::new(&mut self.prefs.snap_move)
                                .speed(0.01)
                                .range(0.001..=100.0)
                                .suffix(" m"),
                        );
                        ui.end_row();
                        ui.label("Snap rotate");
                        ui.add(
                            egui::DragValue::new(&mut self.prefs.snap_rotate_degrees)
                                .speed(0.5)
                                .range(0.1..=180.0)
                                .suffix("°"),
                        );
                        ui.end_row();
                        ui.label("Snap scale");
                        ui.add(
                            egui::DragValue::new(&mut self.prefs.snap_scale)
                                .speed(0.01)
                                .range(0.001..=10.0),
                        );
                        ui.end_row();
                        ui.label("Snap enabled");
                        ui.checkbox(&mut self.prefs.snap, "");
                        ui.end_row();
                        ui.label("Live Inspector")
                            .on_hover_text("Commit Inspector edits on release / Enter (one revision each). Off: edit a draft, then Apply.");
                        ui.checkbox(&mut self.prefs.live_inspector, "");
                        ui.end_row();
                        ui.label("Always show grid");
                        ui.checkbox(&mut self.prefs.grid, "");
                        ui.end_row();
                    });
                    if ui.button("Reset shortcuts to defaults").clicked() {
                        self.prefs.shortcuts.clear();
                    }
                    ui.label(
                        RichText::new(match &self.prefs_path {
                            Some(path) => format!("Saved to {}", path.display()),
                            None => "Not persisted in this session.".into(),
                        })
                        .small(),
                    );
                });
            self.show_prefs = open;
        }
    }

    pub(crate) fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let color = if self.error {
                egui::Color32::from_rgb(255, 178, 111)
            } else {
                egui::Color32::from_rgb(155, 174, 194)
            };
            ui.label(RichText::new(&self.status).small().color(color))
                .on_hover_text(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let item = |ui: &mut egui::Ui, text: String| {
                    ui.label(RichText::new(text).small().monospace());
                    ui.separator();
                };
                item(ui, format!("{:.0} FPS", self.fps));
                if self.saving > 0 {
                    ui.label(
                        RichText::new(format!("saving {}…", self.saving))
                            .small()
                            .monospace()
                            .color(egui::Color32::from_rgb(255, 214, 120)),
                    )
                    .on_hover_text(
                        "Edits are committed in order on the background writer; the scene shows them already",
                    );
                    ui.separator();
                }
                if let Some(job) = &self.scenario_job {
                    let (done, total) = job.progress();
                    item(
                        ui,
                        format!(
                            "scenario {done}/{total} ticks · {:.1} s",
                            job.elapsed().as_secs_f32()
                        ),
                    );
                }
                item(
                    ui,
                    match self.bridge {
                        Some(port) => format!("bridge 127.0.0.1:{port}"),
                        None => "bridge off".into(),
                    },
                );
                item(ui, format!("REV {}", self.rev));
                item(ui, format!("{} proposal(s)", self.proposals.len()));
                item(ui, format!("{} problem(s)", self.problems.len()));
                item(ui, format!("{} selected", self.selection().len()));
            });
        });
    }

    pub(crate) fn snap_steps(&self) -> [f32; 3] {
        [
            self.prefs.snap_move,
            self.prefs.snap_rotate_degrees.to_radians(),
            self.prefs.snap_scale,
        ]
    }

    /// Persist preferences when they changed (dock layout included). Errors become a notice.
    pub(crate) fn save_prefs(&mut self) {
        if let Some(spec) = self.layout.sync() {
            self.prefs.layout = spec;
            if let Some(tab) = self.layout.dock.as_ref().and_then(layout::active_bottom) {
                self.prefs.dock = tab.name().into();
            }
        }
        let Some(path) = &self.prefs_path else {
            return;
        };
        let text = serde_json::to_string_pretty(&self.prefs).unwrap();
        if self.prefs_saved.as_deref() == Some(text.as_str()) {
            return;
        }
        match std::fs::write(path, &text) {
            Ok(()) => self.prefs_saved = Some(text),
            Err(e) => {
                self.prefs_saved = Some(text);
                self.set_error(format!("Could not save editor preferences: {e}"));
            }
        }
    }
    pub fn set_prefs_path(&mut self, path: std::path::PathBuf) {
        self.prefs = Prefs::load(&path);
        // A bad saved layout falls back to Default with a notice; the other preferences stay.
        let (state, problem) = if self.prefs.layout.is_null() {
            let mut state = layout::default_state();
            // Older preferences only stored the active bottom-dock tab.
            if let Some(tab) = Dock::from_name(&self.prefs.dock) {
                layout::show(&mut state, tab);
            }
            (state, None)
        } else {
            match layout::from_spec(&self.prefs.layout) {
                Ok((state, dropped)) if dropped.is_empty() => (state, None),
                Ok((state, dropped)) => (
                    state,
                    Some(format!(
                        "Saved layout: unknown panel(s) ignored: {}",
                        dropped.join(", ")
                    )),
                ),
                Err(e) => (
                    layout::default_state(),
                    Some(format!("Saved layout ignored ({e}); using Default")),
                ),
            }
        };
        self.layout.replace(state, problem.is_none());
        if let Some(problem) = problem {
            self.set_error(problem);
        }
        self.prefs_saved = serde_json::to_string_pretty(&self.prefs).ok();
        self.prefs_path = Some(path);
    }
}

/// `discover editor_commands`: every command ID an agent can run with `editor_view {command}`.
/// `discover editor_layout`: the dock tabs and how to show, close and arrange them.
pub fn register_topics(r: &mut axiom_core::Registry) {
    r.topics.insert(
        "editor_layout".into(),
        serde_json::json!({
            "tabs": Dock::ALL.map(Dock::name),
            "show": {"op":"editor_view","dock":"inspector"},
            "close": {"op":"editor_view","command":"window.close","arg":"history"},
            "layouts": {
                "builtin": layout::BUILTIN,
                "load": {"op":"editor_view","command":"layout.load","arg":"Tall Inspector"},
                "save": {"op":"editor_view","command":"layout.save_as","arg":"NAME"},
                "delete": {"op":"editor_view","command":"layout.delete","arg":"NAME"},
                "reset": {"op":"editor_view","command":"layout.reset"}
            },
            "observe": "editor_view replies and observe metrics.editor list open tabs ({tab, active, focused?, floating?}) and scene_visible",
            "rules": [
                "dock opens a closed tab and focuses it; the old dock names are tab names",
                "the editor view is not rendered while the scene tab is closed or behind another tab",
                "layouts are view-only preferences in *.editor-prefs.json, never in world revisions"
            ]
        }),
    );
    r.topics.insert(
        "editor_inspector".into(),
        serde_json::json!({
            "forms": "typed fields from discover typed:COMPONENT_OR_RESOURCE (enum, range+unit, colour, Euler for quaternions, entity / event / resource-key references, arrays)",
            "open": {"op":"editor_view","select":["lamp"],"open":"component:render.light"},
            "options_dialog": {"op":"editor_view","select":["npc"],"open":"options:object.interactable"},
            "live": "a person's field edit commits on release / Enter / focus loss (one revision per interaction; Esc cancels); prefs live_inspector=false = draft + Apply",
            "headless": {
                "field": "set SEL COMPONENT/FIELD VALUE (several objects: set entities[id=a,id=b] ...)",
                "array": "append|insert|remove|reorder SEL COMPONENT/FIELD ..",
                "reset": "reset SEL COMPONENT",
                "paste": "set SEL /components/COMPONENT {..}",
                "play_on": "tween ID clip NAME set play_on=EVENT (unbind: remove resources[id=runtime.bindings] events I)"
            }
        }),
    );
    r.topics.insert(
        "editor_commands".into(),
        serde_json::json!({
            "run": {"op":"editor_view","command":"edit.duplicate"},
            "run_with_arg": {"op":"editor_view","command":"world.capability_add","arg":"physics"},
            "cli": "axiom --connect 127.0.0.1:4252 view --command edit.duplicate",
            "rule": "same registry as the menu bar, palette and shortcuts; refused while a human draft is dirty or when the command is unavailable (the error says why)",
            "commands": COMMANDS.iter().map(|c| if c.arg.is_empty() { serde_json::json!([c.id, c.title]) } else { serde_json::json!([c.id, c.title, c.arg]) }).collect::<Vec<_>>(),
            // Content-changing commands share axiom_core::ops with these ACL verbs, so an agent
            // gets the same result with no editor running (axiom --db DB acl -e "...").
            "headless": {
                "object.empty": "create empty",
                "object.cube": "create cube",
                "object.camera": "create camera",
                "object.light_point": "create light point (also directional, spot, area)",
                "object.trigger_box": "create trigger box (also sphere)",
                "object.path": "create path",
                "object.follow_path": "follow SEL PATH [speed=] [end=loop|ping_pong|stop] [offset=]",
                "path.add_point": "path ID add [x,y,z] | path ID insert I [x,y,z]",
                "path.remove_point": "path ID remove I",
                "path.point_drag": "path ID move I x,y,z",
                "object.obstacle": "create obstacle",
                "object.nav_agent": "nav SEL agent [speed=] [radius=]",
                "nav.target_object": "nav AGENTS target ENTITY",
                "nav.target_point": "nav AGENTS target x,y,z (scene view: click the ground)",
                "nav.clear_target": "nav AGENTS target none",
                "object.interactable": "interact SEL [event=] [prompt=] [range=] [action=]",
                "object.audio_source": "audio SEL source [clip=] [volume=] [loop=] [autoplay=] [spatial=] [min=] [max=] [rolloff=]",
                "object.audio_listener": "audio ID listener",
                "object.camera_follow": "camera CAMERAS follow TARGET [distance=] [min=] [max=] [yaw=] [pitch=] [height=] [orbit=] [zoom=] [radius=] [collide=]",
                "object.main_camera": "camera main ID",
                "inspector.audio_clip": "set ID audio.source/clip \"CLIP\"",
                "inspector.new_audio_clip": "audio clip ID tone|engine|fountain|creak|footstep",
                "object.tween_clip": "tween SEL clip NAME [property=] [target=] [duration=] [mode=] [autoplay=] [on_finish=]",
                "tween.record_key": "tween ID key CLIP TRACK add T (no value = record the current one)",
                "tween.remove_key": "tween ID key CLIP TRACK remove I",
                "tween.timeline": "key drag: tween ID key CLIP TRACK move I T; mode/autoplay: tween ID clip NAME set mode=|autoplay=; Add track: tween ID track CLIP add P; Remove clip: tween ID clip NAME remove; scrub = view only",
                "inspector.remove_component": "remove SEL COMPONENT",
                "edit.duplicate": "duplicate SEL",
                "edit.paste": "duplicate SEL",
                "edit.delete": "delete SEL",
                "edit.rename": "rename ID \"LABEL\"",
                "object.child_cube": "create cube parent=ID (also empty, camera)",
                "object.create_prefab": "prefab create [PID] from ID",
                "hierarchy.reorder": "move ID before OTHER|after OTHER|end",
                "object.drop_to_ground": "ground SEL",
                "object.tag": "tag SEL +TAG -TAG (Inspector tag chips)",
                "object.add_component": "add SEL COMPONENT [JSON] (adds the capability/systems it needs)",
                "window.world": "World panel: world id|title, capability add|remove, system add|remove, resource create|remove (open a resource: editor_view {open: resource:ID})",
                "world.set_id": "world id ID",
                "world.set_title": "world title TEXT",
                "world.capability_add": "capability add ID",
                "world.capability_remove": "capability remove ID",
                "world.system_add": "system add ID",
                "world.system_remove": "system remove ID",
                "world.resource_create": "resource create ID [JSON]",
                "world.resource_remove": "resource remove ID",
                "event.declare": "event declare NAME (Problems: Declare event on an undeclared-event error)",
                "inspector.multi_edit": "set entities[id=a,id=b] FIELD VALUE (Inspector with several objects selected)",
                "inspector.rotation": "rotate SEL x,y,z (Euler degrees) or create … rot=x,y,z",
                "view_only": "edit.toggle_hidden, edit.toggle_locked, edit.select_children, view.top|front|right|bottom|back|left|perspective, view.gizmos, view.nav_grid, view.bookmark.save|recall.N, RMB+WASD fly, box select (editor view state, never content)",
                "edit.undo": "undo REV",
                "file.verify": "axiom verify",
                "hierarchy": "reparent ID PARENT|root; scene create|activate|delete ID; scene member SCENE SEL on|off",
                "assets.prefab": "instantiate PREFAB",
                "camera.main": "camera main ID",
                "docs": "docs/editor-parity.zh-TW.md"
            },
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldRuntime;
    use axiom_core::World;
    use egui::{pos2, vec2, Rect};

    fn fixture() -> (Editor, WorldRuntime) {
        let world: World =
            serde_json::from_str(include_str!("../../../../content/editor-lab/world.json"))
                .unwrap();
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        (Editor::new(world, 1, None), runtime)
    }
    fn key_frame(
        ctx: &egui::Context,
        editor: &mut Editor,
        runtime: &WorldRuntime,
        key: Key,
        modifiers: egui::Modifiers,
    ) -> Vec<Action> {
        let mut actions = vec![];
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 760.0))),
                modifiers,
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        actions
    }

    #[test]
    fn registry_is_consistent_and_every_default_shortcut_parses() {
        let mut ids = std::collections::BTreeSet::new();
        let mut keys = std::collections::BTreeMap::new();
        for cmd in COMMANDS {
            assert!(ids.insert(cmd.id), "duplicate command {}", cmd.id);
            assert!(MENUS.contains(&cmd.menu), "{} has unknown menu", cmd.id);
            for k in cmd.keys.split('|').filter(|k| !k.is_empty()) {
                let parsed = Shortcut::parse(k).unwrap_or_else(|| panic!("{}: {k}", cmd.id));
                if let Some(other) = keys.insert(format!("{parsed:?}"), cmd.id) {
                    panic!("{k} bound to both {other} and {}", cmd.id);
                }
            }
            if let Some(dock) = cmd.id.strip_prefix("window.").filter(|d| *d != "close") {
                assert!(Dock::from_name(dock).is_some(), "{}", cmd.id);
            }
            assert!(cmd.arg.is_empty() || cmd.keys.is_empty(), "{}", cmd.id);
        }
        assert_eq!(
            Shortcut::parse("Ctrl+Shift+Z"),
            Some(Shortcut {
                ctrl: true,
                shift: true,
                alt: false,
                key: Key::Z
            })
        );
        assert!(Shortcut::parse("Ctrl+NoSuchKey").is_none());
    }

    #[test]
    fn every_command_runs_or_explains_why_not() {
        let (mut editor, _) = fixture();
        for cmd in COMMANDS {
            match editor.run_command(cmd.id) {
                Ok(_) => {}
                Err(e) => {
                    let text = format!("{e:#}");
                    assert!(
                        text.contains("unavailable") && editor.command_blocked(cmd.id).is_some(),
                        "{}: {text}",
                        cmd.id
                    );
                }
            }
        }
        assert!(editor.run_command("no.such").is_err());
        // Spot checks of effects.
        let (mut editor, _) = fixture();
        assert!(editor
            .run_command("edit.undo")
            .is_err_and(|e| format!("{e:#}").contains("nothing to undo")));
        assert!(matches!(
            editor.run_command("object.cube").unwrap().as_slice(),
            [Action::Write { .. }]
        ));
        editor.run_command("tool.rotate").unwrap();
        assert!(editor.tool == Tool::Rotate);
        editor.run_command("window.history").unwrap();
        assert!(editor.dock_shows(Dock::History));
        editor.run_command("view.snap").unwrap();
        assert!(editor.prefs.snap);
        editor.select(Some("stage/crate".into()));
        assert!(matches!(
            editor.run_command("edit.duplicate").unwrap().as_slice(),
            [Action::Write { .. }]
        ));
        editor.run_command("edit.copy").unwrap();
        assert_eq!(editor.pending_copy.as_deref(), Some("stage/crate"));
        assert!(matches!(
            editor.run_command("file.exit").unwrap().as_slice(),
            [Action::Quit]
        ));
    }

    #[test]
    fn keyboard_shortcuts_dispatch_through_the_registry_and_honor_overrides() {
        let (mut editor, runtime) = fixture();
        let ctx = egui::Context::default();
        let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        editor.select(Some("stage/crate".into()));
        let actions = key_frame(&ctx, &mut editor, &runtime, Key::D, ctrl);
        assert!(
            matches!(actions.as_slice(), [Action::Write { .. }]),
            "Ctrl+D duplicates"
        );
        key_frame(&ctx, &mut editor, &runtime, Key::Num3, ctrl);
        assert!(editor.dock_shows(Dock::History), "Ctrl+3 opens History");
        // Rebind duplicate to Ctrl+J: Ctrl+D stops working, Ctrl+J works.
        editor
            .prefs
            .shortcuts
            .insert("edit.duplicate".into(), "Ctrl+J".into());
        assert!(key_frame(&ctx, &mut editor, &runtime, Key::D, ctrl).is_empty());
        let actions = key_frame(&ctx, &mut editor, &runtime, Key::J, ctrl);
        assert!(matches!(actions.as_slice(), [Action::Write { .. }]));
        // The palette opens on Ctrl+Shift+P and suppresses shortcuts while typing.
        key_frame(
            &ctx,
            &mut editor,
            &runtime,
            Key::P,
            ctrl | egui::Modifiers::SHIFT,
        );
        assert!(editor.palette.is_some());
        key_frame(&ctx, &mut editor, &runtime, Key::A, egui::Modifiers::NONE);
        let tool = editor.tool;
        key_frame(&ctx, &mut editor, &runtime, Key::R, egui::Modifiers::NONE);
        assert!(
            editor.tool == tool,
            "typing R in the palette must not switch tools"
        );
    }

    #[test]
    fn palette_filters_and_runs_the_selected_command() {
        let (mut editor, runtime) = fixture();
        let ctx = egui::Context::default();
        editor.run_command("view.palette").unwrap();
        let run = |editor: &mut Editor, events: Vec<egui::Event>| {
            let mut actions = vec![];
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 760.0))),
                    events,
                    ..Default::default()
                },
                |ctx| actions = editor.draw(ctx, &runtime),
            );
            actions
        };
        run(&mut editor, vec![]);
        run(&mut editor, vec![egui::Event::Text("create cube".into())]);
        assert_eq!(editor.palette.as_ref().unwrap().query, "create cube");
        let actions = run(
            &mut editor,
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            matches!(actions.as_slice(), [Action::Write { .. }]),
            "Enter runs Create Cube"
        );
        assert!(editor.palette.is_none());
    }

    #[test]
    fn view_modes_bookmarks_and_drop_to_ground_have_working_shortcuts() {
        let (mut editor, runtime) = fixture();
        let ctx = egui::Context::default();
        let alt = egui::Modifiers::ALT;
        key_frame(&ctx, &mut editor, &runtime, Key::Num2, alt);
        assert!(editor.orbit.mode == ViewMode::Top, "Alt+2 = Top");
        key_frame(&ctx, &mut editor, &runtime, Key::Num4, alt);
        assert!(editor.orbit.mode == ViewMode::Right, "Alt+4 = Right");
        // Ctrl+F7 saves slot 2; recall restores the exact camera after it moved.
        let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        assert!(editor.command_blocked("view.bookmark.recall.2").is_some());
        key_frame(&ctx, &mut editor, &runtime, Key::F7, ctrl);
        let saved = editor.orbit.clone();
        key_frame(&ctx, &mut editor, &runtime, Key::Num1, alt);
        editor.orbit.target += glam::Vec3::new(3.0, 0.0, -2.0);
        assert!(editor.orbit != saved);
        key_frame(&ctx, &mut editor, &runtime, Key::F7, egui::Modifiers::NONE);
        assert!(editor.orbit == saved, "F7 recalls bookmark 2");
        // Bookmarks persist per world in the prefs file.
        let dir = std::env::temp_dir().join(format!("axiom-bookmarks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.editor-prefs.json");
        let bookmarks = editor.prefs.bookmarks.clone();
        editor.set_prefs_path(path.clone());
        editor.prefs.bookmarks = bookmarks;
        editor.prefs.fly_speed = 12.5;
        editor.save_prefs();
        let (mut again, _) = fixture();
        again.set_prefs_path(path.clone());
        assert_eq!(again.prefs.fly_speed, 12.5);
        again.run_command("view.bookmark.recall.2").unwrap();
        assert!(again.orbit == saved);
        // Another world does not see this world's bookmarks.
        std::sync::Arc::make_mut(&mut again.world).id = "other".into();
        assert!(again.run_command("view.bookmark.recall.2").is_err());
        let _ = std::fs::remove_dir_all(dir);
        // End drops the selection to the ground (one write).
        editor.select(Some("stage/crate".into()));
        let actions = key_frame(&ctx, &mut editor, &runtime, Key::End, egui::Modifiers::NONE);
        assert!(matches!(actions.as_slice(), [Action::Write { .. }]));
    }

    #[test]
    fn preferences_persist_and_bad_values_are_clamped() {
        let dir = std::env::temp_dir().join(format!("axiom-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("world.editor-prefs.json");
        let (mut editor, _) = fixture();
        editor.set_prefs_path(path.clone());
        editor.prefs.snap = true;
        editor.prefs.snap_move = 0.5;
        editor.show_dock(Dock::Assets);
        editor.save_prefs();
        let (mut again, _) = fixture();
        again.set_prefs_path(path.clone());
        assert!(again.prefs.snap && again.prefs.snap_move == 0.5);
        assert!(again.dock_shows(Dock::Assets));
        std::fs::write(
            &path,
            r#"{"snap_move":-3,"snap_rotate_degrees":1e9,"shortcuts":{"no.such":"Ctrl+Q"}}"#,
        )
        .unwrap();
        let loaded = Prefs::load(&path);
        assert_eq!(loaded.snap_move, 0.001);
        assert_eq!(loaded.snap_rotate_degrees, 180.0);
        assert!(loaded.shortcuts.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
