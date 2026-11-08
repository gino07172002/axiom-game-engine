//! Editor command registry. One table drives the menu bar, the command palette, keyboard
//! shortcuts and the bridge (`editor_view {command}`), so a person and an agent trigger exactly
//! the same operations through the same validators.
use super::{Action, Dock, Editor, Tool};
use egui::{Key, RichText};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub menu: &'static str,
    /// Default shortcut(s), `|`-separated, e.g. `Ctrl+Y|Ctrl+Shift+Z`. Empty for none.
    pub keys: &'static str,
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
    }
}
pub(crate) const MENUS: [&str; 7] = ["File", "Edit", "View", "Object", "Play", "Window", "Help"];
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
    c("edit.frame", "Frame Selection", "Edit", "F"),
    c("edit.preferences", "Preferences…", "Edit", "Ctrl+Comma"),
    c("tool.select", "Select Tool", "View", "Q"),
    c("tool.move", "Move Tool", "View", "W"),
    c("tool.rotate", "Rotate Tool", "View", "E"),
    c("tool.scale", "Scale Tool", "View", "R"),
    c("view.space", "Toggle Global / Local Axes", "View", "X"),
    c("view.snap", "Toggle Snap", "View", ""),
    c("view.grid", "Toggle Grid", "View", ""),
    c("view.palette", "Command Palette…", "View", "Ctrl+Shift+P"),
    c("object.empty", "Create Empty", "Object", "Ctrl+Shift+N"),
    c("object.cube", "Create Cube", "Object", ""),
    c("object.camera", "Create Camera", "Object", ""),
    c("play.start", "Play", "Play", "F5"),
    c("window.changes", "Changes", "Window", "Ctrl+1"),
    c("window.proposals", "Proposals", "Window", "Ctrl+2"),
    c("window.history", "History", "Window", "Ctrl+3"),
    c("window.problems", "Problems", "Window", "Ctrl+4"),
    c("window.scenarios", "Scenarios", "Window", "Ctrl+5"),
    c("window.graphs", "Graphs", "Window", "Ctrl+6"),
    c("window.assets", "Assets", "Window", "Ctrl+7"),
    c("window.agent", "AI / Protocol", "Window", "Ctrl+8"),
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

fn dock_name(dock: Dock) -> &'static str {
    match dock {
        Dock::Changes => "changes",
        Dock::Proposals => "proposals",
        Dock::History => "history",
        Dock::Problems => "problems",
        Dock::Scenarios => "scenarios",
        Dock::Graphs => "graphs",
        Dock::Assets => "assets",
        Dock::Agent => "agent",
    }
}
pub(crate) fn dock_from_name(name: &str) -> Option<Dock> {
    [
        Dock::Changes,
        Dock::Proposals,
        Dock::History,
        Dock::Problems,
        Dock::Scenarios,
        Dock::Graphs,
        Dock::Assets,
        Dock::Agent,
    ]
    .into_iter()
    .find(|d| dock_name(*d) == name)
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
    pub dock: String,
    /// Command ID -> replacement shortcut(s) (`|`-separated; empty string unbinds).
    pub shortcuts: BTreeMap<String, String>,
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
            shortcuts: BTreeMap::new(),
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
        self.shortcuts
            .retain(|id, keys| command(id).is_some() && keys.len() <= 64);
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
            "edit.undo" if self.undo.is_empty() => Some("nothing to undo"),
            "edit.redo" if self.redo.is_empty() => Some("nothing to redo"),
            "edit.paste" if self.clipboard.is_empty() => Some("nothing copied"),
            "edit.duplicate" | "edit.delete" | "edit.copy" | "edit.frame" | "file.copy_context"
                if nothing =>
            {
                Some("nothing selected")
            }
            "play.start" if self.previewing => Some("already playing"),
            "edit.undo" | "edit.redo" | "edit.paste" | "edit.duplicate" | "edit.delete"
            | "edit.select_none" | "object.empty" | "object.cube" | "object.camera"
            | "play.start"
                if busy =>
            {
                Some("apply or reload the inspector draft first")
            }
            "tool.select" | "tool.move" | "tool.rotate" | "tool.scale" | "view.space"
                if self.drag.is_some() =>
            {
                Some("finish the current drag first")
            }
            _ => None,
        }
    }

    /// Runs a registered command. Unknown or blocked commands are errors, never no-ops.
    pub(crate) fn run_command(&mut self, id: &str) -> anyhow::Result<Vec<Action>> {
        anyhow::ensure!(
            command(id).is_some(),
            "unknown editor command {id}; see discover editor (commands)"
        );
        if let Some(reason) = self.command_blocked(id) {
            anyhow::bail!("{id} is unavailable: {reason}");
        }
        let mut actions = vec![];
        match id {
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
            "play.start" => actions.push(Action::Play),
            "help.shortcuts" => self.show_shortcuts = true,
            "help.copy_discovery" => {
                self.pending_copy = Some(r#"{"op":"discover","topic":"editor"}"#.to_owned());
                self.notice("Copied editor discovery request");
            }
            other => {
                let dock = other
                    .strip_prefix("window.")
                    .and_then(dock_from_name)
                    .expect("every registered window command names a dock");
                self.dock = dock;
            }
        }
        Ok(actions)
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
                            "edit.preferences" | "file.exit" | "help.copy_discovery"
                        ) {
                            ui.separator();
                        }
                        let blocked = self.command_blocked(cmd.id);
                        let checked = match cmd.id {
                            "view.snap" => Some(self.prefs.snap),
                            "view.grid" => Some(self.prefs.grid),
                            "view.space" => Some(self.prefs.local_space),
                            id => id
                                .strip_prefix("window.")
                                .and_then(dock_from_name)
                                .map(|d| d == self.dock),
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
                            self.run_into(cmd.id, actions);
                            ui.close_menu();
                        }
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
                self.run_into(id, actions);
            }
            if keep && self.palette.is_none() {
                self.palette = Some(palette);
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

    /// Persist preferences when they changed (dock tab included). Errors become a notice.
    pub(crate) fn save_prefs(&mut self) {
        self.prefs.dock = dock_name(self.dock).into();
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
        if let Some(dock) = dock_from_name(&self.prefs.dock) {
            self.dock = dock;
        }
        self.prefs_saved = serde_json::to_string_pretty(&self.prefs).ok();
        self.prefs_path = Some(path);
    }
}

/// `discover editor_commands`: every command ID an agent can run with `editor_view {command}`.
pub fn register_topics(r: &mut axiom_core::Registry) {
    r.topics.insert(
        "editor_commands".into(),
        serde_json::json!({
            "run": {"op":"editor_view","command":"edit.duplicate"},
            "cli": "axiom --connect 127.0.0.1:4252 view --command edit.duplicate",
            "rule": "same registry as the menu bar, palette and shortcuts; refused while a human draft is dirty or when the command is unavailable (the error says why)",
            "commands": COMMANDS.iter().map(|c| serde_json::json!([c.id, c.title])).collect::<Vec<_>>(),
            // Content-changing commands share axiom_core::ops with these ACL verbs, so an agent
            // gets the same result with no editor running (axiom --db DB acl -e "...").
            "headless": {
                "object.empty": "create empty",
                "object.cube": "create cube",
                "object.camera": "create camera",
                "edit.duplicate": "duplicate SEL",
                "edit.paste": "duplicate SEL",
                "edit.delete": "delete SEL",
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
            if let Some(dock) = cmd.id.strip_prefix("window.") {
                assert!(dock_from_name(dock).is_some(), "{}", cmd.id);
            }
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
        assert!(editor.dock == Dock::History);
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
        assert!(editor.dock == Dock::History, "Ctrl+3 opens History");
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
    fn preferences_persist_and_bad_values_are_clamped() {
        let dir = std::env::temp_dir().join(format!("axiom-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("world.editor-prefs.json");
        let (mut editor, _) = fixture();
        editor.set_prefs_path(path.clone());
        editor.prefs.snap = true;
        editor.prefs.snap_move = 0.5;
        editor.dock = Dock::Assets;
        editor.save_prefs();
        let (mut again, _) = fixture();
        again.set_prefs_path(path.clone());
        assert!(again.prefs.snap && again.prefs.snap_move == 0.5);
        assert!(again.dock == Dock::Assets);
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
