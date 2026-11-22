//! Input Map tab: `input.bindings` as a table of button and axis actions. Clicking a key cell
//! starts "press a key": the next key press (or Mouse Left) becomes the binding in one
//! revision (`ACL: input button ACTION KEY` / `input axis ACTION POSITIVE NEGATIVE`); each row
//! also has gamepad dropdowns (`input pad ..`) and the header the stick deadzone. Keys
//! bound to more than one action are flagged (the runtime allows it; every action sees the
//! key). Esc cancels a capture; the editor's shortcuts stand down while one is running.
use super::icons::Icon;
use super::tuning::mark;
use super::{theme, Action, Dock, Editor};
use axiom_core::ops::{self, InputEdit};
use axiom_core::InputBindings;
use egui::{Key, RichText, Ui};
use serde_json::{json, Value};

/// Which key of an action a capture writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Button,
    Positive,
    Negative,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Capture {
    pub action: String,
    pub slot: Slot,
    /// A new axis: the positive key captured first, the negative one next (one revision).
    pub positive: Option<String>,
    /// The action does not exist yet (Add Button / Add Axis).
    pub new: bool,
    /// Modifiers held last frame (a modifier-only press arrives as a modifier change).
    pub modifiers: egui::Modifiers,
    /// Pass number of the frame the tab last drew (a hidden tab does not capture).
    pub drawn: u64,
    /// Started by a command since the tab last drew: this frame's keys predate it.
    pub fresh: bool,
}

#[derive(Default)]
pub(crate) struct InputUi {
    pub capture: Option<Capture>,
    /// Name typed for Add Button / Add Axis.
    pub new_action: String,
    /// Gamepad deadzone while it is being dragged or typed (written on release).
    deadzone: Option<f32>,
    cache: Option<Model>,
    /// Times the table model was rebuilt (tests: not per frame).
    pub builds: usize,
}

struct Model {
    key: (i64, u64),
    bindings: Option<InputBindings>,
    conflicts: Vec<(String, Vec<String>)>,
}

impl InputUi {
    pub fn capturing(&self, ctx: &egui::Context) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|c| c.drawn + 2 >= ctx.cumulative_pass_nr())
    }
    pub fn observation(&self) -> Value {
        json!({
            "capturing": self.capture.as_ref().map(|c| json!({"action":c.action,"slot":format!("{:?}", c.slot).to_lowercase(),"new":c.new})),
            "conflicts": self.cache.as_ref().map_or(0, |m| m.conflicts.len()),
        })
    }
}

/// The runtime's key name (winit `KeyCode` Debug form, see `app.rs`) of an egui key.
pub(crate) fn key_name(key: Key) -> Option<String> {
    let debug = format!("{key:?}");
    Some(match key {
        Key::A
        | Key::B
        | Key::C
        | Key::D
        | Key::E
        | Key::F
        | Key::G
        | Key::H
        | Key::I
        | Key::J
        | Key::K
        | Key::L
        | Key::M
        | Key::N
        | Key::O
        | Key::P
        | Key::Q
        | Key::R
        | Key::S
        | Key::T
        | Key::U
        | Key::V
        | Key::W
        | Key::X
        | Key::Y
        | Key::Z => format!("Key{debug}"),
        Key::Num0
        | Key::Num1
        | Key::Num2
        | Key::Num3
        | Key::Num4
        | Key::Num5
        | Key::Num6
        | Key::Num7
        | Key::Num8
        | Key::Num9 => format!("Digit{}", &debug[3..]),
        Key::OpenBracket => "BracketLeft".into(),
        Key::CloseBracket => "BracketRight".into(),
        Key::Backtick => "Backquote".into(),
        Key::Equals => "Equal".into(),
        Key::ArrowDown
        | Key::ArrowLeft
        | Key::ArrowRight
        | Key::ArrowUp
        | Key::Tab
        | Key::Backspace
        | Key::Enter
        | Key::Space
        | Key::Insert
        | Key::Delete
        | Key::Home
        | Key::End
        | Key::PageUp
        | Key::PageDown
        | Key::Comma
        | Key::Backslash
        | Key::Slash
        | Key::Minus
        | Key::Period
        | Key::Semicolon
        | Key::Quote => debug,
        // F1..F35 are named alike in egui and winit.
        _ if debug.len() <= 3 && debug.starts_with('F') => debug,
        // Logical-only keys (Colon, Plus, Copy ..) have no physical key code.
        _ => return None,
    })
}

/// Key label for the table (`KeyW` → `W`, `Digit1` → `1`).
fn short(key: &str) -> &str {
    key.strip_prefix("Key")
        .or_else(|| key.strip_prefix("Digit"))
        .unwrap_or(key)
}

/// The key captured this frame: Esc → `Err(())` (cancel), else the first physical key press or
/// a newly held modifier.
fn captured(ctx: &egui::Context, before: egui::Modifiers) -> Option<Result<String, ()>> {
    ctx.input(|i| {
        for event in &i.events {
            if let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat: false,
                ..
            } = event
            {
                if *key == Key::Escape {
                    return Some(Err(()));
                }
                if let Some(name) = key_name(physical_key.unwrap_or(*key)) {
                    return Some(Ok(name));
                }
            }
        }
        let m = i.modifiers;
        for (now, was, name) in [
            (m.shift, before.shift, "ShiftLeft"),
            (m.ctrl, before.ctrl, "ControlLeft"),
            (m.alt, before.alt, "AltLeft"),
        ] {
            if now && !was {
                return Some(Ok(name.to_owned()));
            }
        }
        None
    })
}

/// The natural other half of an axis made from `input` (stick and d-pad directions, paired
/// buttons, triggers and shoulders).
fn opposite(input: &str) -> &'static str {
    const PAIRS: [(&str, &str); 12] = [
        ("PadA", "PadB"),
        ("PadX", "PadY"),
        ("PadLB", "PadRB"),
        ("PadBack", "PadStart"),
        ("PadLS", "PadRS"),
        ("PadDpadUp", "PadDpadDown"),
        ("PadDpadRight", "PadDpadLeft"),
        ("PadRT", "PadLT"),
        ("PadLStickUp", "PadLStickDown"),
        ("PadLStickRight", "PadLStickLeft"),
        ("PadRStickUp", "PadRStickDown"),
        ("PadRStickRight", "PadRStickLeft"),
    ];
    PAIRS
        .iter()
        .find_map(|&(a, b)| {
            if a == input {
                Some(b)
            } else if b == input {
                Some(a)
            } else {
                None
            }
        })
        .unwrap_or("PadLStickDown")
}

/// A gamepad input dropdown; `Some(pick)` when the person chose a different entry
/// (`Some(None)` = "—", no gamepad input).
fn pad_choice(
    ui: &mut Ui,
    salt: String,
    current: Option<&str>,
    enabled: bool,
) -> Option<Option<String>> {
    let mut picked = None;
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(salt)
            .width(118.0)
            .selected_text(RichText::new(current.unwrap_or("—")).monospace())
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), "—").clicked() && current.is_some() {
                    picked = Some(None);
                }
                for name in axiom_core::PAD_INPUTS {
                    if ui.selectable_label(current == Some(name), name).clicked()
                        && current != Some(name)
                    {
                        picked = Some(Some(name.to_owned()));
                    }
                }
            });
    });
    picked
}

enum Intent {
    Capture(String, Slot, bool),
    Bind(InputEdit),
    Pad(String, Slot, Option<String>),
    Cancel,
}

impl Editor {
    /// One `input.bindings` edit (`ACL: input button|axis|remove ..`).
    pub(super) fn input_edit(&mut self, edit: &InputEdit) -> Option<Action> {
        let result = ops::edit_input(&self.world, edit);
        self.tuning_write(result)
    }

    /// Input Map ▸ Pad cell (`ACL: input pad ACTION ..`): the gamepad input of `action`'s
    /// `slot`; `None` removes the action's gamepad binding. Picking one side of an axis that
    /// has no gamepad binding yet fills the other side with the opposite input.
    pub(super) fn input_pad_pick(
        &mut self,
        action: &str,
        slot: Slot,
        pick: Option<String>,
    ) -> Option<Action> {
        let bound = self
            .bindings()
            .and_then(|b| b.gamepad.axes.get(action).cloned());
        let action = action.to_owned();
        let edit = match (slot, pick) {
            (_, None) => InputEdit::PadRemove { action },
            (Slot::Button, Some(input)) => InputEdit::PadButton { action, input },
            (Slot::Positive, Some(input)) => InputEdit::PadAxis {
                negative: bound.map_or_else(|| opposite(&input).to_owned(), |a| a.negative),
                positive: input,
                action,
            },
            (Slot::Negative, Some(input)) => InputEdit::PadAxis {
                positive: bound.map_or_else(|| opposite(&input).to_owned(), |a| a.positive),
                negative: input,
                action,
            },
        };
        self.input_edit(&edit)
    }

    /// Input Map ▸ Deadzone (`ACL: input deadzone V`).
    pub(super) fn input_deadzone(&mut self, value: f32) -> Option<Action> {
        self.input_edit(&InputEdit::Deadzone { value })
    }

    /// `input.capture ACTION[+|-]`: starts "press a key" for an existing action's key.
    pub(super) fn input_capture_command(&mut self, arg: Option<&str>) -> anyhow::Result<()> {
        self.show_dock(Dock::InputMap);
        let Some(arg) = arg else {
            return Ok(());
        };
        let b = self.bindings();
        // A button may itself end in + or - (valid_id allows it): an exact button name wins.
        let button = b.as_ref().is_some_and(|b| b.buttons.contains_key(arg));
        let (action, slot) = match (button, arg.strip_suffix('+'), arg.strip_suffix('-')) {
            (true, ..) => (arg, Slot::Button),
            (false, Some(a), _) => (a, Slot::Positive),
            (false, None, Some(a)) => (a, Slot::Negative),
            (false, None, None) => (arg, Slot::Button),
        };
        let ok = b.as_ref().is_some_and(|b| match slot {
            Slot::Button => b.buttons.contains_key(action),
            _ => b.axes.contains_key(action),
        });
        anyhow::ensure!(
            ok,
            "input.bindings has no {} {action} (axes take ACTION+ / ACTION-)",
            if slot == Slot::Button {
                "button"
            } else {
                "axis"
            }
        );
        self.tuning.input.capture = Some(Capture {
            action: action.to_owned(),
            slot,
            positive: None,
            new: false,
            modifiers: Default::default(),
            drawn: 0,
            fresh: true,
        });
        Ok(())
    }

    fn bindings(&self) -> Option<InputBindings> {
        self.world
            .resources
            .get(ops::INPUT_BINDINGS)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// The Input Map tab body.
    pub(super) fn input_map_tab(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let key = (self.rev, self.world_version);
        if self
            .tuning
            .input
            .cache
            .as_ref()
            .is_none_or(|m| m.key != key)
        {
            let bindings = self.bindings();
            let conflicts = bindings
                .as_ref()
                .map(ops::input_conflicts)
                .unwrap_or_default();
            self.tuning.input.cache = Some(Model {
                key,
                bindings,
                conflicts,
            });
            self.tuning.input.builds += 1;
        }
        let model = self.tuning.input.cache.take().expect("built above");
        let mut intents = vec![];
        if self.previewing {
            // Play pins a revision and owns the keys: no capture survives into it.
            self.tuning.input.capture = None;
        }
        self.input_map_body(ui, &model, &mut intents, actions);
        self.tuning.input.cache = Some(model);
        for intent in intents {
            match intent {
                Intent::Capture(action, slot, new) => {
                    self.tuning.input.capture = Some(Capture {
                        action,
                        slot,
                        positive: None,
                        new,
                        modifiers: ui.input(|i| i.modifiers),
                        drawn: ui.ctx().cumulative_pass_nr(),
                        fresh: false,
                    });
                }
                Intent::Cancel => self.tuning.input.capture = None,
                Intent::Bind(edit) => {
                    self.tuning.input.capture = None;
                    actions.extend(self.input_edit(&edit));
                }
                Intent::Pad(action, slot, pick) => {
                    actions.extend(self.input_pad_pick(&action, slot, pick));
                }
            }
        }
    }

    fn input_map_body(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        intents: &mut Vec<Intent>,
        actions: &mut Vec<Action>,
    ) {
        let icons = self.icons;
        let editable = !self.previewing;
        let Some(bindings) = &model.bindings else {
            ui.label(RichText::new("This world has no input map.").color(theme::MUTED));
            let create = ui.add_enabled(editable, egui::Button::new("Create Input Map"));
            mark!(self.rects, "input:create".into(), create.rect);
            if create.clicked() {
                match self.run_command("asset.new_input_map") {
                    Ok(more) => actions.extend(more),
                    Err(e) => self.set_error(format!("{e:#}")),
                }
            }
            return;
        };
        // A running capture: the key pressed this frame (the prompt itself is in the row).
        let pass = ui.ctx().cumulative_pass_nr();
        let typing = ui
            .ctx()
            .memory(|m| m.focused())
            .is_some_and(|f| egui::TextEdit::load_state(ui.ctx(), f).is_some());
        if let Some(c) = self.tuning.input.capture.as_mut() {
            // Hidden for a while: keys pressed meanwhile went elsewhere; start listening anew.
            let fresh = std::mem::take(&mut c.fresh) || c.drawn + 2 < pass;
            c.drawn = pass;
            let got = if typing {
                Some(Err(()))
            } else if fresh {
                None
            } else {
                captured(ui.ctx(), c.modifiers)
            };
            c.modifiers = ui.input(|i| i.modifiers);
            // An existing action removed by another revision mid-capture: nothing to rebind.
            let axis = bindings.axes.get(&c.action);
            let gone = !c.new
                && if c.slot == Slot::Button {
                    !bindings.buttons.contains_key(&c.action)
                } else {
                    axis.is_none()
                };
            match got {
                Some(Err(())) => intents.push(Intent::Cancel),
                Some(Ok(_)) if gone => intents.push(Intent::Cancel),
                Some(Ok(key)) => match c.slot {
                    Slot::Button => intents.push(Intent::Bind(InputEdit::Button {
                        action: c.action.clone(),
                        key,
                    })),
                    Slot::Positive if c.new => {
                        c.positive = Some(key);
                        c.slot = Slot::Negative;
                    }
                    Slot::Positive => {
                        let negative = axis.map(|a| a.negative.clone()).unwrap_or_default();
                        intents.push(Intent::Bind(InputEdit::Axis {
                            action: c.action.clone(),
                            positive: key,
                            negative,
                        }));
                    }
                    Slot::Negative => {
                        let positive = c
                            .positive
                            .clone()
                            .or_else(|| axis.map(|a| a.positive.clone()))
                            .unwrap_or_default();
                        intents.push(Intent::Bind(InputEdit::Axis {
                            action: c.action.clone(),
                            positive,
                            negative: key,
                        }));
                    }
                },
                None => {}
            }
        }
        let capture = self.tuning.input.capture.clone();
        // Header: counts, conflicts, add.
        ui.horizontal(|ui| {
            icons.show(ui, Icon::Input, 16.0, theme::ACCENT);
            ui.label(RichText::new("input.bindings").strong().color(theme::TEXT));
            theme::chip(
                ui,
                format!(
                    "{} buttons · {} axes",
                    bindings.buttons.len(),
                    bindings.axes.len()
                ),
                theme::MUTED,
            );
            if model.conflicts.is_empty() {
                theme::chip(ui, "no conflicts", theme::SUCCESS);
            } else {
                let chip = theme::chip(
                    ui,
                    format!("{} conflicting key(s)", model.conflicts.len()),
                    theme::WARNING,
                );
                mark!(self.rects, "input:conflicts".into(), chip.rect);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(editable && capture.is_none(), |ui| {
                    let name = self.tuning.input.new_action.trim().to_owned();
                    let known =
                        bindings.buttons.contains_key(&name) || bindings.axes.contains_key(&name);
                    let ok = !name.is_empty() && !known && axiom_core::valid_id(&name).is_ok();
                    let axis = ui
                        .add_enabled(ok, egui::Button::new("+ Axis"))
                        .on_hover_text("Press the positive key, then the negative key");
                    mark!(self.rects, "input:add_axis".into(), axis.rect);
                    if axis.clicked() {
                        intents.push(Intent::Capture(name.clone(), Slot::Positive, true));
                    }
                    let button = ui
                        .add_enabled(ok, egui::Button::new("+ Button"))
                        .on_hover_text("Then press the key to bind");
                    mark!(self.rects, "input:add_button".into(), button.rect);
                    if button.clicked() {
                        intents.push(Intent::Capture(name, Slot::Button, true));
                    }
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.tuning.input.new_action)
                            .id_salt("input_new_action")
                            .hint_text("new action…")
                            .desired_width(130.0),
                    );
                    mark!(self.rects, "input:new_action".into(), field.rect);
                });
            });
        });
        if !model.conflicts.is_empty() {
            ui.horizontal_wrapped(|ui| {
                icons.show(ui, Icon::Warning, 14.0, theme::WARNING);
                for (key, users) in &model.conflicts {
                    ui.label(
                        RichText::new(format!("{} → {}", short(key), users.join(", ")))
                            .size(11.5)
                            .color(theme::WARNING),
                    );
                }
            });
        }
        if let Some(c) = &capture {
            if c.new {
                theme::card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        icons.show(ui, Icon::Record, 14.0, theme::ACCENT);
                        let what = match (c.slot, &c.positive) {
                            (Slot::Button, _) => format!("New button {}: press a key…", c.action),
                            (Slot::Negative, Some(p)) => {
                                format!(
                                    "New axis {}: + {}; press the negative key…",
                                    c.action,
                                    short(p)
                                )
                            }
                            _ => format!("New axis {}: press the positive key…", c.action),
                        };
                        ui.label(RichText::new(what).color(theme::TEXT));
                        self.capture_extras(ui, c, intents);
                    });
                });
            }
        }
        let pad_editable = editable && capture.is_none();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Gamepad (XInput) deadzone").color(theme::MUTED));
            let authored = bindings.gamepad.deadzone;
            let mut value = self.tuning.input.deadzone.unwrap_or(authored);
            let r = ui.add_enabled(
                pad_editable,
                egui::DragValue::new(&mut value)
                    .range(0.0..=0.9)
                    .speed(0.005)
                    .fixed_decimals(2),
            );
            mark!(self.rects, "input:deadzone".into(), r.rect);
            self.tuning.input.deadzone = Some(value);
            if r.drag_stopped() || r.lost_focus() {
                self.tuning.input.deadzone = None;
                if (value - authored).abs() > 1e-6 {
                    actions.extend(self.input_deadzone(value));
                }
            } else if !r.dragged() && !r.has_focus() {
                self.tuning.input.deadzone = None;
            }
            ui.label(
                RichText::new("pad cells: a second input of each action (— = none)")
                    .size(11.0)
                    .color(theme::MUTED),
            );
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("input_map_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let conflicted = |k: &str| model.conflicts.iter().any(|(c, _)| c == k);
                ui.label(RichText::new("BUTTONS").size(11.0).color(theme::MUTED));
                for (action, key) in &bindings.buttons {
                    ui.horizontal(|ui| {
                        super::tuning::cell_label(
                            ui,
                            egui::vec2(150.0, 22.0),
                            RichText::new(action).monospace().color(theme::TEXT),
                        );
                        self.key_cell(
                            ui,
                            action,
                            Slot::Button,
                            key,
                            conflicted(key),
                            capture.as_ref(),
                            intents,
                        );
                        let pad = bindings.gamepad.buttons.get(action).map(String::as_str);
                        ui.label(RichText::new("pad").color(theme::MUTED));
                        if let Some(pick) =
                            pad_choice(ui, format!("input_pad:{action}"), pad, pad_editable)
                        {
                            intents.push(Intent::Pad(action.clone(), Slot::Button, pick));
                        }
                        self.remove_cell(ui, action, editable && capture.is_none(), intents);
                    });
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new("AXES  (+ / −)")
                        .size(11.0)
                        .color(theme::MUTED),
                );
                for (action, axis) in &bindings.axes {
                    ui.horizontal(|ui| {
                        super::tuning::cell_label(
                            ui,
                            egui::vec2(150.0, 22.0),
                            RichText::new(action).monospace().color(theme::TEXT),
                        );
                        ui.label(RichText::new("+").color(theme::MUTED));
                        self.key_cell(
                            ui,
                            action,
                            Slot::Positive,
                            &axis.positive,
                            conflicted(&axis.positive),
                            capture.as_ref(),
                            intents,
                        );
                        ui.label(RichText::new("−").color(theme::MUTED));
                        self.key_cell(
                            ui,
                            action,
                            Slot::Negative,
                            &axis.negative,
                            conflicted(&axis.negative),
                            capture.as_ref(),
                            intents,
                        );
                        let pad = bindings.gamepad.axes.get(action);
                        ui.label(RichText::new("pad +").color(theme::MUTED));
                        let positive = pad.map(|a| a.positive.as_str());
                        if let Some(pick) =
                            pad_choice(ui, format!("input_pad+:{action}"), positive, pad_editable)
                        {
                            intents.push(Intent::Pad(action.clone(), Slot::Positive, pick));
                        }
                        ui.label(RichText::new("−").color(theme::MUTED));
                        let negative = pad.map(|a| a.negative.as_str());
                        if let Some(pick) =
                            pad_choice(ui, format!("input_pad-:{action}"), negative, pad_editable)
                        {
                            intents.push(Intent::Pad(action.clone(), Slot::Negative, pick));
                        }
                        self.remove_cell(ui, action, editable && capture.is_none(), intents);
                    });
                }
            });
    }

    /// Mouse Left / Cancel buttons shown while a key is being captured.
    fn capture_extras(&mut self, ui: &mut Ui, c: &Capture, intents: &mut Vec<Intent>) {
        let mouse = ui.small_button("Mouse Left");
        mark!(self.rects, "input:mouse_left".into(), mouse.rect);
        if mouse.clicked() {
            let key = ops::MOUSE_LEFT.to_owned();
            match (c.slot, &c.positive) {
                (Slot::Button, _) => intents.push(Intent::Bind(InputEdit::Button {
                    action: c.action.clone(),
                    key,
                })),
                (Slot::Negative, Some(p)) => intents.push(Intent::Bind(InputEdit::Axis {
                    action: c.action.clone(),
                    positive: p.clone(),
                    negative: key,
                })),
                // Existing axes and the first key of a new one: same path as a key press.
                _ => {
                    if let Some(cap) = self.tuning.input.capture.as_mut() {
                        if cap.new {
                            cap.positive = Some(key);
                            cap.slot = Slot::Negative;
                        } else {
                            let b = self.bindings();
                            if let Some(axis) = b.as_ref().and_then(|b| b.axes.get(&c.action)) {
                                let (positive, negative) = if c.slot == Slot::Positive {
                                    (key, axis.negative.clone())
                                } else {
                                    (axis.positive.clone(), key)
                                };
                                intents.push(Intent::Bind(InputEdit::Axis {
                                    action: c.action.clone(),
                                    positive,
                                    negative,
                                }));
                            }
                        }
                    }
                }
            }
        }
        if ui.small_button("Cancel (Esc)").clicked() {
            intents.push(Intent::Cancel);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn key_cell(
        &mut self,
        ui: &mut Ui,
        action: &str,
        slot: Slot,
        key: &str,
        conflict: bool,
        capture: Option<&Capture>,
        intents: &mut Vec<Intent>,
    ) {
        let waiting = capture.is_some_and(|c| !c.new && c.action == action && c.slot == slot);
        let (text, color) = if waiting {
            ("press a key…".to_owned(), theme::ACCENT)
        } else if conflict {
            (format!("⚠ {}", short(key)), theme::WARNING)
        } else {
            (short(key).to_owned(), theme::TEXT)
        };
        let enabled = !self.previewing && (capture.is_none() || waiting);
        let button = ui
            .add_enabled(
                enabled,
                egui::Button::new(RichText::new(text).monospace().color(color))
                    .min_size(egui::vec2(96.0, 22.0))
                    .selected(waiting),
            )
            .on_hover_text(format!(
                "{key} · click, then press the new key (Esc cancels)"
            ));
        mark!(
            self.rects,
            format!("input:key:{action}:{}", format!("{slot:?}").to_lowercase()),
            button.rect
        );
        if waiting {
            self.capture_extras(ui, capture.expect("waiting implies a capture"), intents);
        } else if button.clicked() {
            intents.push(Intent::Capture(action.to_owned(), slot, false));
        }
    }

    fn remove_cell(&mut self, ui: &mut Ui, action: &str, enabled: bool, intents: &mut Vec<Intent>) {
        let icons = self.icons;
        let remove = ui
            .add_enabled_ui(enabled, |ui| {
                theme::icon_button(ui, icons, Icon::Delete, "Remove this action", false)
            })
            .inner;
        mark!(self.rects, format!("input:remove:{action}"), remove.rect);
        if remove.clicked() {
            intents.push(Intent::Bind(InputEdit::Remove {
                action: action.to_owned(),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical key list of `axiom_core` is exactly what the native host produces:
    /// every named winit `KeyCode` (Debug form, as `app.rs` formats it) plus the pointer
    /// names; every key the Input Map can capture is in it.
    #[test]
    fn core_key_names_match_winit_and_captured_keys() {
        macro_rules! winit_names {
            ($($k:ident),* $(,)?) => {
                vec![$({
                    let name = format!("{:?}", winit::keyboard::KeyCode::$k);
                    assert_eq!(name, stringify!($k));
                    name
                }),*]
            };
        }
        let mut winit = winit_names!(
            Backquote,
            Backslash,
            BracketLeft,
            BracketRight,
            Comma,
            Digit0,
            Digit1,
            Digit2,
            Digit3,
            Digit4,
            Digit5,
            Digit6,
            Digit7,
            Digit8,
            Digit9,
            Equal,
            IntlBackslash,
            IntlRo,
            IntlYen,
            KeyA,
            KeyB,
            KeyC,
            KeyD,
            KeyE,
            KeyF,
            KeyG,
            KeyH,
            KeyI,
            KeyJ,
            KeyK,
            KeyL,
            KeyM,
            KeyN,
            KeyO,
            KeyP,
            KeyQ,
            KeyR,
            KeyS,
            KeyT,
            KeyU,
            KeyV,
            KeyW,
            KeyX,
            KeyY,
            KeyZ,
            Minus,
            Period,
            Quote,
            Semicolon,
            Slash,
            AltLeft,
            AltRight,
            Backspace,
            CapsLock,
            ContextMenu,
            ControlLeft,
            ControlRight,
            Enter,
            SuperLeft,
            SuperRight,
            ShiftLeft,
            ShiftRight,
            Space,
            Tab,
            Convert,
            KanaMode,
            Lang1,
            Lang2,
            Lang3,
            Lang4,
            Lang5,
            NonConvert,
            Delete,
            End,
            Help,
            Home,
            Insert,
            PageDown,
            PageUp,
            ArrowDown,
            ArrowLeft,
            ArrowRight,
            ArrowUp,
            NumLock,
            Numpad0,
            Numpad1,
            Numpad2,
            Numpad3,
            Numpad4,
            Numpad5,
            Numpad6,
            Numpad7,
            Numpad8,
            Numpad9,
            NumpadAdd,
            NumpadBackspace,
            NumpadClear,
            NumpadClearEntry,
            NumpadComma,
            NumpadDecimal,
            NumpadDivide,
            NumpadEnter,
            NumpadEqual,
            NumpadHash,
            NumpadMemoryAdd,
            NumpadMemoryClear,
            NumpadMemoryRecall,
            NumpadMemoryStore,
            NumpadMemorySubtract,
            NumpadMultiply,
            NumpadParenLeft,
            NumpadParenRight,
            NumpadStar,
            NumpadSubtract,
            Escape,
            Fn,
            FnLock,
            PrintScreen,
            ScrollLock,
            Pause,
            BrowserBack,
            BrowserFavorites,
            BrowserForward,
            BrowserHome,
            BrowserRefresh,
            BrowserSearch,
            BrowserStop,
            Eject,
            LaunchApp1,
            LaunchApp2,
            LaunchMail,
            MediaPlayPause,
            MediaSelect,
            MediaStop,
            MediaTrackNext,
            MediaTrackPrevious,
            Power,
            Sleep,
            AudioVolumeDown,
            AudioVolumeMute,
            AudioVolumeUp,
            WakeUp,
            Meta,
            Hyper,
            Turbo,
            Abort,
            Resume,
            Suspend,
            Again,
            Copy,
            Cut,
            Find,
            Open,
            Paste,
            Props,
            Select,
            Undo,
            Hiragana,
            Katakana,
            F1,
            F2,
            F3,
            F4,
            F5,
            F6,
            F7,
            F8,
            F9,
            F10,
            F11,
            F12,
            F13,
            F14,
            F15,
            F16,
            F17,
            F18,
            F19,
            F20,
            F21,
            F22,
            F23,
            F24,
            F25,
            F26,
            F27,
            F28,
            F29,
            F30,
            F31,
            F32,
            F33,
            F34,
            F35
        );
        winit.extend(["MouseLeft", "WheelUp", "WheelDown"].map(String::from));
        let mut core: Vec<String> = axiom_core::ops::KEY_NAMES
            .iter()
            .map(|k| (*k).to_owned())
            .collect();
        winit.sort();
        core.sort();
        assert_eq!(core, winit);
        for key in Key::ALL {
            if let Some(name) = key_name(*key) {
                assert!(axiom_core::ops::valid_key_name(&name), "{key:?} -> {name}");
            }
        }
        for name in ["ShiftLeft", "ControlLeft", "AltLeft"] {
            assert!(axiom_core::ops::valid_key_name(name));
        }
    }
}
