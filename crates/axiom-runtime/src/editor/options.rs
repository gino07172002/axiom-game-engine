//! Options dialogs of the create commands (Object menu / command palette): the same options
//! the ACL verbs take, prefilled with the defaults, producing the same `axiom_core::ops`
//! call as the ACL line shown under the fields. Only fields changed from their default are
//! passed, exactly like an ACL line that names only those options. From a shortcut or the
//! bridge (`editor_view {command}`) these commands still run at once with the defaults.
use super::{Action, Editor};
use axiom_core::assets::AssetKind;
use axiom_core::ops::{self, NewAsset};
use egui::{Key, RichText};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Input {
    Text,
    Number,
    Bool,
    Choice(&'static [&'static str]),
    /// A clip of `audio.library` (or any name for a new library).
    Clip,
    /// A `behavior.program` event name (declared ones offered).
    Event,
    /// A shader for a new material (built-in and the world's custom ones offered).
    Shader,
}

#[derive(Clone, Debug)]
struct Field {
    /// ACL option name (`key=value`); `""` for the leading positional argument.
    key: &'static str,
    label: &'static str,
    input: Input,
    value: String,
    default: String,
}

impl Field {
    fn new(
        key: &'static str,
        label: &'static str,
        input: Input,
        default: impl Into<String>,
    ) -> Self {
        let default = default.into();
        Field {
            key,
            label,
            input,
            value: default.clone(),
            default,
        }
    }
    fn changed(&self) -> bool {
        self.value.trim() != self.default
    }
    fn number(&self) -> Result<f32, String> {
        self.value
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("{} must be a number", self.label))
    }
    fn on(&self) -> bool {
        self.value == "on"
    }
}

/// An open create-command options dialog.
#[derive(Clone, Debug)]
pub(crate) struct OptionsDialog {
    pub command: &'static str,
    title: &'static str,
    fields: Vec<Field>,
    focused: bool,
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

/// `"text with spaces"` for an ACL token, bare otherwise.
fn token(v: &str) -> String {
    if v.is_empty() || v.chars().any(|c| c.is_whitespace() || c == '"') {
        serde_json::to_string(v).unwrap_or_default()
    } else {
        v.to_owned()
    }
}

impl OptionsDialog {
    /// The dialog for `command`, prefilled for the current selection; `None` when the
    /// command has no options.
    pub fn for_command(editor: &Editor, command: &'static str) -> Option<Self> {
        use Input::*;
        let schema_default = |component: &str, field: &str, fallback: &str| -> String {
            axiom_core::typed_schema::typed_schema(&editor.registry, &editor.world, component)
                .and_then(|s| s["properties"][field].get("default").cloned())
                .map(|d| match d {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Bool(b) => on_off(b).to_owned(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| fallback.to_owned())
        };
        let (title, fields) = match command {
            "object.interactable" => (
                "Make Interactable",
                vec![
                    Field::new("event", "Event", Event, axiom_core::interact::DEFAULT_EVENT),
                    Field::new("prompt", "Prompt", Text, ""),
                    Field::new(
                        "range",
                        "Range (m)",
                        Number,
                        schema_default("interact.target", "range", "2"),
                    ),
                    Field::new(
                        "action",
                        "Input button",
                        Text,
                        axiom_core::interact::DEFAULT_ACTION,
                    ),
                ],
            ),
            "object.tween_clip" => (
                "Add Tween Clip",
                vec![
                    Field::new(
                        "",
                        "Clip name",
                        Text,
                        ops::free_clip_name(&editor.world, &editor.selection()),
                    ),
                    Field::new(
                        "property",
                        "Property",
                        Choice(&axiom_core::tween::PROPERTIES),
                        ops::DEFAULT_TWEEN_PROPERTY,
                    ),
                    Field::new("target", "Target (child ID)", Text, ""),
                    Field::new("duration", "Duration (s)", Number, "1"),
                    Field::new(
                        "mode",
                        "Mode",
                        Choice(&["once", "loop", "ping_pong"]),
                        "once",
                    ),
                    Field::new("speed", "Speed", Number, "1"),
                    Field::new("autoplay", "Autoplay", Bool, "off"),
                    Field::new("on_finish", "On finish event", Event, ""),
                    Field::new("play_on", "Play on event", Event, ""),
                ],
            ),
            "object.audio_source" => {
                let first = editor
                    .world
                    .resources
                    .get("audio.library")
                    .and_then(|l| l.get("clips"))
                    .and_then(serde_json::Value::as_object)
                    .and_then(|c| c.keys().next().cloned())
                    .unwrap_or_else(|| "tone".into());
                (
                    "Add Audio Source",
                    vec![
                        Field::new("clip", "Clip", Clip, first),
                        Field::new(
                            "volume",
                            "Volume",
                            Number,
                            schema_default("audio.source", "volume", "1"),
                        ),
                        Field::new("loop", "Loop", Bool, "off"),
                        Field::new("autoplay", "Autoplay", Bool, "off"),
                        Field::new("spatial", "Spatial", Bool, "on"),
                        Field::new(
                            "min",
                            "Min distance (m)",
                            Number,
                            schema_default("audio.source", "min_distance", "1"),
                        ),
                        Field::new(
                            "max",
                            "Max distance (m)",
                            Number,
                            schema_default("audio.source", "max_distance", "30"),
                        ),
                        Field::new(
                            "rolloff",
                            "Rolloff",
                            Choice(&["inverse", "linear"]),
                            "inverse",
                        ),
                    ],
                )
            }
            "audio.new_clip" => (
                "New Audio Clip",
                vec![
                    Field::new(
                        "",
                        "Preset",
                        Choice(&axiom_core::audio::CLIP_PRESETS),
                        "tone",
                    ),
                    Field::new(
                        "",
                        "Clip ID",
                        Text,
                        ops::free_audio_clip_id(&editor.world, "tone"),
                    ),
                ],
            ),
            "object.follow_path" => (
                "Follow Path",
                vec![
                    Field::new(
                        "speed",
                        "Speed (m/s)",
                        Number,
                        schema_default("path.follow", "speed", "1.5"),
                    ),
                    Field::new(
                        "end",
                        "At the end",
                        Choice(&["loop", "ping_pong", "stop"]),
                        "loop",
                    ),
                    Field::new("offset", "Side offset (m)", Number, "0"),
                ],
            ),
            "object.nav_agent" => (
                "Add Nav Agent",
                vec![
                    Field::new(
                        "speed",
                        "Speed (m/s)",
                        Number,
                        schema_default("nav.agent", "speed", "1.4"),
                    ),
                    Field::new(
                        "radius",
                        "Radius (m)",
                        Number,
                        schema_default("nav.agent", "radius", "0.3"),
                    ),
                ],
            ),
            "object.camera_follow" => (
                "Camera Follow Selected",
                vec![
                    Field::new(
                        "distance",
                        "Distance (m)",
                        Number,
                        schema_default("camera.follow", "distance", "5"),
                    ),
                    Field::new(
                        "min",
                        "Min distance (m)",
                        Number,
                        schema_default("camera.follow", "min_distance", "1"),
                    ),
                    Field::new(
                        "max",
                        "Max distance (m)",
                        Number,
                        schema_default("camera.follow", "max_distance", "12"),
                    ),
                    Field::new(
                        "yaw",
                        "Yaw (°)",
                        Number,
                        schema_default("camera.follow", "yaw", "0"),
                    ),
                    Field::new(
                        "pitch",
                        "Pitch (°)",
                        Number,
                        schema_default("camera.follow", "pitch", "20"),
                    ),
                    Field::new(
                        "height",
                        "Look height (m)",
                        Number,
                        schema_default("camera.follow", "look_height", "1.5"),
                    ),
                    Field::new("radius", "Collision radius (m)", Number, "0.2"),
                    Field::new("collide", "Stop at walls", Bool, "on"),
                ],
            ),
            "asset.new_material" => (
                "New Material",
                vec![
                    Field::new(
                        "",
                        "Material ID",
                        Text,
                        ops::free_asset_id(&editor.world, AssetKind::Material, "material"),
                    ),
                    Field::new("shader", "Shader", Shader, "lit"),
                    Field::new("tint", "Tint (r,g,b,a)", Text, ""),
                ],
            ),
            "asset.new_prefab" => (
                "New Prefab",
                vec![
                    Field::new(
                        "",
                        "Prefab ID",
                        Text,
                        ops::free_asset_id(&editor.world, AssetKind::Prefab, "prefab"),
                    ),
                    Field::new("from", "From object (optional)", Text, ""),
                ],
            ),
            "asset.new_machine" => (
                "New FSM Machine",
                vec![
                    Field::new(
                        "",
                        "Machine ID",
                        Text,
                        ops::free_asset_id(&editor.world, AssetKind::Machine, "machine"),
                    ),
                    Field::new("initial", "Initial state", Text, "idle"),
                ],
            ),
            "asset.new_ui_document" => (
                "New UI Document",
                vec![Field::new("locale", "Locale", Text, "en")],
            ),
            "asset.new_input_map" => ("New Input Map", vec![]),
            _ => return None,
        };
        Some(OptionsDialog {
            command,
            title,
            fields,
            focused: false,
        })
    }

    fn field(&self, label: &str) -> &Field {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .expect("dialog field")
    }

    /// The value of the option named `key` when it differs from its default.
    fn get(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key == key && f.changed())
    }

    fn text(&self, key: &str) -> Option<String> {
        self.get(key).map(|f| f.value.trim().to_owned())
    }

    fn number(&self, key: &str) -> Result<Option<f32>, String> {
        self.get(key).map(Field::number).transpose()
    }

    fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).map(Field::on)
    }

    /// Assets ▸ New: the asset the fields describe (only changed options are passed).
    fn new_asset(&self) -> Result<Option<NewAsset>, String> {
        let id = |label: &str| self.field(label).value.trim().to_owned();
        Ok(Some(match self.command {
            "asset.new_material" => NewAsset::Material {
                id: id("Material ID"),
                shader: self.text("shader"),
                tint: self
                    .text("tint")
                    .map(|t| {
                        let v: Vec<f32> = t
                            .split(',')
                            .map(|p| p.trim().parse::<f32>())
                            .collect::<Result<_, _>>()
                            .map_err(|_| "tint is r,g,b or r,g,b,a numbers".to_owned())?;
                        match v.as_slice() {
                            [r, g, b] => Ok([*r, *g, *b, 1.0]),
                            [r, g, b, a] => Ok([*r, *g, *b, *a]),
                            _ => Err("tint is r,g,b or r,g,b,a numbers".to_owned()),
                        }
                    })
                    .transpose()?,
            },
            "asset.new_prefab" => NewAsset::Prefab {
                id: id("Prefab ID"),
                from: self.text("from"),
            },
            "asset.new_machine" => NewAsset::Machine {
                id: id("Machine ID"),
                initial: self.text("initial"),
            },
            "asset.new_ui_document" => NewAsset::Ui {
                locale: self.text("locale"),
            },
            "asset.new_input_map" => NewAsset::Input,
            _ => return Ok(None),
        }))
    }

    /// The equivalent ACL statement on the editor's selection (shown under the fields; the
    /// parity tests run it).
    pub fn acl(&self, editor: &Editor) -> String {
        match self.new_asset() {
            Ok(Some(new)) => return new.acl(),
            Err(e) => return e,
            Ok(None) => {}
        }
        let list = |ids: &[String]| match ids {
            [] => "SEL".to_owned(),
            [one] => one.clone(),
            many => format!(
                "entities[{}]",
                many.iter()
                    .map(|id| format!("id={id}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        };
        let sel = list(&editor.selection());
        let options: Vec<String> = self
            .fields
            .iter()
            .filter(|f| !f.key.is_empty() && f.changed())
            .map(|f| format!("{}={}", f.key, token(f.value.trim())))
            .collect();
        let options = options.join(" ");
        let line = match self.command {
            "object.interactable" => format!("interact {sel} {options}"),
            "object.tween_clip" => format!(
                "tween {sel} clip {} {options}",
                token(self.field("Clip name").value.trim())
            ),
            "object.audio_source" => format!("audio {sel} source {options}"),
            "audio.new_clip" => format!(
                "audio clip {} {}",
                token(self.field("Clip ID").value.trim()),
                self.field("Preset").value
            ),
            "object.follow_path" => match editor.follow_targets() {
                Some((followers, path)) => format!("follow {} {path} {options}", list(&followers)),
                None => format!("follow {sel} PATH {options}"),
            },
            "object.nav_agent" => format!("nav {sel} agent {options}"),
            "object.camera_follow" => match editor.camera_targets() {
                Some((cameras, target)) => {
                    format!("camera {} follow {target} {options}", list(&cameras))
                }
                None => format!("camera {sel} follow TARGET {options}"),
            },
            _ => String::new(),
        };
        line.trim_end().to_owned()
    }

    /// New audio clip: an untouched Clip ID follows the preset (`engine`, `engine2`, ...).
    fn sync(&mut self, world: &axiom_core::World) {
        if self.command != "audio.new_clip" {
            return;
        }
        let preset = self.field("Preset").value.clone();
        let proposed = ops::free_audio_clip_id(world, &preset);
        let id = self
            .fields
            .iter_mut()
            .find(|f| f.label == "Clip ID")
            .expect("dialog field");
        if !id.changed() && id.default != proposed {
            id.default = proposed.clone();
            id.value = proposed;
        }
    }

    /// Runs the command with the dialog's options (same ops call as the ACL line).
    pub fn run(&mut self, editor: &mut Editor) -> Result<Option<Action>, String> {
        self.sync(&editor.world);
        if let Some(new) = self.new_asset()? {
            return Ok(editor.create_asset(&new));
        }
        Ok(match self.command {
            "object.interactable" => {
                let options = ops::InteractOptions {
                    event: self.text("event"),
                    prompt: self.text("prompt"),
                    range: self.number("range")?,
                    action: self.text("action"),
                };
                editor.make_interactable(&options)
            }
            "object.tween_clip" => {
                let mode = self
                    .text("mode")
                    .map(|m| axiom_core::tween::TweenMode::parse(&m).map_err(|e| e.to_string()))
                    .transpose()?;
                let options = ops::TweenClipOptions {
                    property: self.text("property"),
                    target: self.text("target"),
                    duration: self.number("duration")?.map(Some),
                    mode,
                    speed: self.number("speed")?,
                    autoplay: self.bool("autoplay"),
                    on_finish: self.text("on_finish"),
                    play_on: self.text("play_on"),
                };
                let name = token(self.field("Clip name").value.trim()).to_owned();
                editor.add_tween_clip(&name, &options)
            }
            "object.audio_source" => {
                let rolloff = self.text("rolloff").map(|r| match r.as_str() {
                    "linear" => axiom_core::audio::Rolloff::Linear,
                    _ => axiom_core::audio::Rolloff::Inverse,
                });
                let options = ops::AudioSourceOptions {
                    clip: self.text("clip"),
                    volume: self.number("volume")?,
                    looping: self.bool("loop"),
                    autoplay: self.bool("autoplay"),
                    spatial: self.bool("spatial"),
                    min_distance: self.number("min")?,
                    max_distance: self.number("max")?,
                    rolloff,
                };
                editor.add_audio_source(&options)
            }
            "audio.new_clip" => {
                let id = self.field("Clip ID").value.trim().to_owned();
                let preset = self.field("Preset").value.clone();
                editor.add_audio_clip(&id, &preset)
            }
            "object.follow_path" => {
                let end = self.text("end").map(|e| match e.as_str() {
                    "ping_pong" => axiom_core::path::PathEnd::PingPong,
                    "stop" => axiom_core::path::PathEnd::Stop,
                    _ => axiom_core::path::PathEnd::Loop,
                });
                let options = ops::FollowOptions {
                    speed: self.number("speed")?,
                    end,
                    offset: self.number("offset")?,
                };
                editor.follow_selection(&options)
            }
            "object.nav_agent" => {
                let options = ops::NavOptions {
                    speed: self.number("speed")?,
                    radius: self.number("radius")?,
                };
                editor.add_nav_agents(&options)
            }
            "object.camera_follow" => {
                let options = ops::CameraFollowOptions {
                    distance: self.number("distance")?,
                    min_distance: self.number("min")?,
                    max_distance: self.number("max")?,
                    yaw: self.number("yaw")?,
                    pitch: self.number("pitch")?,
                    look_height: self.number("height")?,
                    radius: self.number("radius")?,
                    collision: self.bool("collide"),
                    ..Default::default()
                };
                editor.camera_follow_selection(&options)
            }
            _ => None,
        })
    }

    /// Sets the field labelled `label` (tests drive the dialog through its fields).
    #[cfg(test)]
    pub fn set(&mut self, label: &str, value: &str) {
        self.fields
            .iter_mut()
            .find(|f| f.label == label)
            .unwrap_or_else(|| panic!("no field {label}"))
            .value = value.to_owned();
    }
}

impl Editor {
    /// Menu / palette entry point: commands with options open their dialog; every other
    /// command runs at once.
    pub(crate) fn invoke(&mut self, id: &'static str, actions: &mut Vec<Action>) {
        if self.command_blocked(id).is_none() {
            if let Some(dialog) = OptionsDialog::for_command(self, id) {
                self.options = Some(dialog);
                return;
            }
        }
        self.run_into(id, actions);
    }

    /// Draws the open options dialog; Create runs it, Esc / Cancel closes it.
    pub(crate) fn options_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(mut dialog) = self.options.take() else {
            return;
        };
        let events: Vec<String> = self
            .world
            .resources
            .get("behavior.program")
            .and_then(|p| p.get("events"))
            .and_then(serde_json::Value::as_object)
            .map(|e| e.keys().cloned().collect())
            .unwrap_or_default();
        let clips: Vec<String> = self
            .world
            .resources
            .get("audio.library")
            .and_then(|l| l.get("clips"))
            .and_then(serde_json::Value::as_object)
            .map(|c| c.keys().cloned().collect())
            .unwrap_or_default();
        let shaders: Vec<String> = axiom_core::assets::map(&self.world, AssetKind::Shader)
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default();
        dialog.sync(&self.world);
        let acl = dialog.acl(self);
        let first_frame = !dialog.focused;
        let mut create = false;
        let mut cancel = false;
        let mut problem = None;
        egui::Window::new(dialog.title)
            .id(egui::Id::new("create_options"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 90.0])
            .show(ctx, |ui| {
                egui::Grid::new("create_options_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        for (n, f) in dialog.fields.iter_mut().enumerate() {
                            ui.label(f.label);
                            let r = match f.input {
                                Input::Text => ui.add(egui::TextEdit::singleline(&mut f.value).desired_width(180.0)),
                                Input::Number => {
                                    let ok = f.number().is_ok();
                                    ui.add(
                                        egui::TextEdit::singleline(&mut f.value)
                                            .desired_width(80.0)
                                            .text_color(if ok { egui::Color32::LIGHT_GRAY } else { egui::Color32::from_rgb(240, 96, 96) }),
                                    )
                                }
                                Input::Bool => {
                                    let mut b = f.on();
                                    let r = ui.checkbox(&mut b, "");
                                    if r.changed() {
                                        f.value = on_off(b).into();
                                    }
                                    r
                                }
                                Input::Choice(options) => choice(ui, n, &mut f.value, options.iter().copied()),
                                Input::Clip => choice(ui, n, &mut f.value, clips.iter().map(String::as_str)),
                                Input::Shader => choice(ui, n, &mut f.value, axiom_core::assets::BUILTIN_SHADERS.into_iter().chain(shaders.iter().map(String::as_str))),
                                Input::Event => {
                                    ui.horizontal(|ui| {
                                        let r = ui.add(egui::TextEdit::singleline(&mut f.value).desired_width(140.0).hint_text("(none)"));
                                        ui.menu_button("…", |ui| {
                                            for e in &events {
                                                if ui.button(e).clicked() {
                                                    f.value = e.clone();
                                                    ui.close_menu();
                                                }
                                            }
                                            if events.is_empty() {
                                                ui.label("No declared events");
                                            }
                                        });
                                        r
                                    })
                                    .inner
                                }
                            };
                            if n == 0 && !dialog.focused {
                                r.request_focus();
                                dialog.focused = true;
                            }
                            #[cfg(test)]
                            self.rects.insert(format!("options:{}", f.label), r.rect);
                            ui.end_row();
                        }
                    });
                // A dialog without fields (New Input Map) has nothing to focus.
                dialog.focused |= dialog.fields.is_empty();
                if dialog.command == "object.tween_clip" {
                    ui.label(RichText::new("Play on event: the clip plays whenever the event fires (runtime.bindings)").small().weak());
                }
                ui.separator();
                ui.label(RichText::new(format!("ACL: {acl}")).monospace().small().weak());
                ui.horizontal(|ui| {
                    let r = ui.button("Create");
                    #[cfg(test)]
                    self.rects.insert("options:create".into(), r.rect);
                    create = r.clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        }
        if !first_frame
            && ctx.input(|i| i.key_pressed(Key::Enter))
            && !ctx.memory(|m| m.any_popup_open())
        {
            create = true;
        }
        if create && !cancel {
            match dialog.run(self) {
                Ok(Some(action)) => {
                    actions.push(action);
                    return;
                }
                Ok(None) => return,
                Err(e) => problem = Some(e),
            }
        }
        if let Some(problem) = problem {
            self.set_error(problem);
        }
        if !cancel {
            self.options = Some(dialog);
        }
    }
}

fn choice<'a>(
    ui: &mut egui::Ui,
    n: usize,
    value: &mut String,
    options: impl Iterator<Item = &'a str>,
) -> egui::Response {
    egui::ComboBox::from_id_salt(("create_option", n))
        .selected_text(value.as_str())
        .show_ui(ui, |ui| {
            for option in options {
                if ui.selectable_label(value == option, option).clicked() {
                    *value = option.to_owned();
                }
            }
        })
        .response
}
