//! Graphs ▸ Animator: the `animation.controllers` graph on the shared Graphs canvas, its
//! parameter table and the state / transition forms (motion, blend points, conditions,
//! cross-fade, exit time), plus the live view of a controlled entity during Play.
//!
//! Content changes are `axiom_core::ops` calls shared with ACL `animctl ...`
//! (`graph_edit` / `graph_set_node` / `anim_param` / `anim_create` / `anim_attach`), one
//! revision per interaction: a form edit commits when the pointer is released and nothing is
//! being typed, like the FSM forms. Live parameter pokes in Play are runtime commands
//! (`animation.param`), never revisions.
use super::{settled, Draft, Sel, ACCENT, LIVE, MUTED, WARNING};
use crate::editor::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::ops::{self, FsmEdit, GraphDoc};
use egui::{RichText, Ui};
use serde_json::{json, Map, Value};

const KINDS: [&str; 3] = ["float", "bool", "trigger"];
const OPS: [(&str, &str); 7] = [
    ("gt", ">"),
    ("lt", "<"),
    ("eq", "="),
    ("ne", "≠"),
    ("true", "is true"),
    ("false", "is false"),
    ("trigger", "triggered"),
];

/// The first controlled entity of the open controller during Play (buffers reused).
#[derive(Default)]
pub(crate) struct Live {
    pub entity: String,
    pub entities: usize,
    pub state: String,
    pub previous: Option<String>,
    pub names: Vec<String>,
    pub values: Vec<f32>,
    pub floats: Vec<bool>,
    /// (node of the current state, its blend-in weight) while a cross-fade runs.
    pub fade: Option<(usize, f32)>,
}

impl Live {
    pub(crate) fn observe(&self) -> Value {
        let params: Map<String, Value> = self
            .names
            .iter()
            .zip(self.values.iter().zip(&self.floats))
            .map(|(n, (v, float))| (n.clone(), if *float { json!(v) } else { json!(*v != 0.0) }))
            .collect();
        json!({"entity":self.entity,"entities":self.entities,"state":self.state,"previous":self.previous,"params":params})
    }
}

/// Parameter table being edited (committed like a graph element draft).
pub(crate) struct ParamsDraft {
    controller: String,
    base: Value,
    value: Value,
    dirty: bool,
}

fn number(v: &Value) -> String {
    match v.as_f64() {
        Some(n) => format!("{}", (n * 100.0).round() / 100.0),
        None => v.to_string(),
    }
}

/// Node subtitle of an animation state.
pub(super) fn state_label(state: &Value) -> String {
    let m = &state["motion"];
    match m["kind"].as_str() {
        Some("clip") => {
            let mut s = format!("clip {}", m["clip"].as_str().unwrap_or("?"));
            if m["looping"] == false {
                s.push_str(" (once)");
            }
            s
        }
        Some("blend1d") => {
            let clips: Vec<&str> = m["points"]
                .as_array()
                .map(|p| p.iter().filter_map(|x| x["clip"].as_str()).collect())
                .unwrap_or_default();
            format!(
                "blend {}: {}",
                m["parameter"].as_str().unwrap_or("?"),
                clips.join("·")
            )
        }
        Some(other) => other.to_owned(),
        None => "rest pose".into(),
    }
}

/// Edge label of an animation transition: conditions, exit time and fade.
pub(super) fn transition_label(t: &Value) -> String {
    let mut parts: Vec<String> = t["conditions"]
        .as_array()
        .map(|cs| {
            cs.iter()
                .map(|c| {
                    let p = c["param"].as_str().unwrap_or("?");
                    match c["op"].as_str().unwrap_or("?") {
                        "true" | "trigger" => p.to_owned(),
                        "false" => format!("!{p}"),
                        op => {
                            let sym = OPS.iter().find(|(o, _)| *o == op).map_or(op, |o| o.1);
                            format!("{p} {sym} {}", number(&c["value"]))
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(e) = t.get("exit_time").filter(|e| !e.is_null()) {
        parts.push(format!("exit {}", number(e)));
    }
    let mut s = parts.join(" & ");
    if let Some(d) = t["duration"].as_f64().filter(|d| *d > 0.0) {
        s.push_str(&format!("  ·{}s", (d * 100.0).round() / 100.0));
    }
    s
}

/// Text field + optional numeric drag for an `f64`; returns whether it changed.
fn drag(ui: &mut Ui, value: &mut Value, range: std::ops::RangeInclusive<f64>, speed: f64) -> bool {
    let mut v = value.as_f64().unwrap_or(0.0);
    let changed = ui
        .add(
            egui::DragValue::new(&mut v)
                .range(range)
                .speed(speed)
                .max_decimals(3),
        )
        .changed();
    if changed {
        *value = json!(v);
    }
    changed
}

/// Clip combo over `clips` (a clip the list lacks is shown and kept).
fn clip_combo(ui: &mut Ui, id: impl std::hash::Hash, clips: &[String], value: &mut Value) -> bool {
    let current = value.as_str().unwrap_or("").to_owned();
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if current.is_empty() {
            "(pick a clip)"
        } else {
            current.as_str()
        })
        .width(110.0)
        .show_ui(ui, |ui| {
            for c in clips {
                if ui.selectable_label(*c == current, c).clicked() && *c != current {
                    *value = json!(c);
                    changed = true;
                }
            }
        });
    changed
}

impl Editor {
    /// Live state of the open controller (Play): node counts and the first entity's
    /// parameters, current / previous state and fade.
    pub(super) fn anim_graph_live(&mut self, runtime: &WorldRuntime) {
        let (Some(system), Some(model)) = (
            runtime.system::<crate::animation::AnimationSystem>(),
            self.graph.model.as_ref(),
        ) else {
            self.graph.anim_live = None;
            return;
        };
        let mut live = self.graph.anim_live.take().unwrap_or_default();
        live.entities = 0;
        live.fade = None;
        for (entity, a) in system.animators() {
            if *a.id != *model.machine {
                continue;
            }
            let c = &a.compiled;
            let current = &c.states[a.state.current.state as usize];
            let node = model.node(current);
            if let Some(i) = node {
                self.graph.live[i] = self.graph.live[i].saturating_add(1);
            }
            if live.entities == 0 {
                live.entity.clear();
                live.entity.push_str(entity);
                live.state.clone_from(current);
                live.previous = a.state.previous.map(|p| c.states[p.state as usize].clone());
                live.names.clone_from(&c.params);
                live.values.clear();
                live.values
                    .extend_from_slice(&a.state.params[..c.params.len()]);
                live.floats.clear();
                live.floats.extend(
                    (0..c.params.len())
                        .map(|i| c.param_kind(i) == axiom_animation::controller::ParamKind::Float),
                );
                if a.state.previous.is_some() && a.state.fade_duration > 0.0 {
                    let w = (a.state.fade / a.state.fade_duration).clamp(0.0, 1.0);
                    live.fade = node.map(|i| (i, w));
                }
            }
            live.entities += 1;
        }
        self.graph.anim_live = (live.entities > 0).then_some(live);
    }

    /// Clip IDs offered by the forms: those of the players that run the open controller
    /// (every player's when none does yet). Cached per world version.
    pub(super) fn anim_refresh_clips(&mut self, runtime: &WorldRuntime) {
        if self
            .graph
            .anim_clips
            .as_ref()
            .is_some_and(|(v, _)| *v == self.world_version)
        {
            return;
        }
        let controller = self.graph.machine.clone().unwrap_or_default();
        let catalog = &runtime.state.animation_cues.catalog;
        let users: Vec<&str> = self
            .world
            .entities
            .iter()
            .filter(|e| {
                e.components
                    .get("animation.controller")
                    .is_some_and(|c| c["controller"] == controller.as_str())
            })
            .map(|e| e.id.as_str())
            .collect();
        let mut clips = std::collections::BTreeSet::new();
        for (entity, list) in catalog {
            if users.is_empty() || users.contains(&entity.as_str()) {
                clips.extend(list.keys().cloned());
            }
        }
        self.graph.anim_clips = Some((self.world_version, clips.into_iter().collect()));
    }

    /// Graphs ▸ Animator ▸ New controller / `graph.new_controller ID` (ACL `animctl ID create`).
    pub(super) fn anim_new_controller(&mut self, id: &str) -> Option<Action> {
        let result = ops::anim_create(self.graph_world(), &self.registry, id, None);
        let action = self.graph_write(result)?;
        self.graph_set_doc(GraphDoc::Animator);
        self.graph.machine = Some(id.to_owned());
        self.graph.view = super::View::Animator;
        self.graph_select(None);
        Some(action)
    }

    /// `graph.attach_controller ID` (ACL `animctl SEL attach ID`): runs controller `ID` on
    /// the selected objects.
    pub(super) fn anim_attach(&mut self, id: &str) -> Option<Action> {
        let ids = self.selection();
        let result = ops::anim_attach(self.graph_world(), &self.registry, &ids, id);
        let action = self.graph_write(result);
        if action.is_some() {
            self.open_component_after = Some("animation.controller".into());
        }
        action
    }

    /// `graph.param "NAME float|bool|trigger|JSON|remove"` on the open controller (ACL
    /// `animctl C param set NAME JSON` / `animctl C param remove NAME`).
    pub(super) fn anim_param_command(&mut self, spec: &str) -> anyhow::Result<Option<Action>> {
        let (name, rest) = spec.trim().split_once(char::is_whitespace).ok_or_else(|| {
            anyhow::anyhow!("graph.param takes NAME float|bool|trigger|JSON|remove")
        })?;
        let rest = rest.trim();
        let def = match rest {
            "remove" => None,
            kind if KINDS.contains(&kind) => Some(json!({ "kind": kind })),
            json => Some(serde_json::from_str::<Value>(json)?),
        };
        let controller = self
            .graph
            .machine
            .clone()
            .ok_or_else(|| anyhow::anyhow!("open an animation controller first"))?;
        let result = ops::anim_param(self.graph_world(), &controller, name, def.as_ref());
        Ok(self.graph_write(result))
    }

    /// The Animator side panel: parameters, then the selected state / transition.
    pub(super) fn anim_panel(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        self.anim_params_panel(ui, actions);
        ui.separator();
        let Some(sel) = self.graph.selected.clone() else {
            ui.label(RichText::new("Click a state or transition to edit it. Drag from a state's port to another state to add a transition; double-click empty space to add a state.").small().color(MUTED));
            return;
        };
        let Some(committed) = self.graph_element(&sel) else {
            return;
        };
        let reload = match &self.graph.element {
            Some(d) => d.sel != sel || (!d.dirty && d.value != committed),
            None => true,
        };
        if reload {
            self.graph.element = Some(Draft {
                sel: sel.clone(),
                base: committed.clone(),
                value: committed,
                dirty: false,
            });
        }
        let model = self.graph.model.as_ref().expect("set by machine_view");
        let states: Vec<String> = model.nodes.iter().map(|n| n.name.clone()).collect();
        let initial = model.initial.clone();
        let transitions = model.transitions;
        let params: Vec<(String, String)> = self
            .graph_element_root()
            .and_then(|c| c["parameters"].as_object().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|(n, d)| (n, d["kind"].as_str().unwrap_or("float").to_owned()))
            .collect();
        let clips = self
            .graph
            .anim_clips
            .as_ref()
            .map(|(_, c)| c.clone())
            .unwrap_or_default();
        let mut draft = self.graph.element.take().expect("set above");
        let mut request = None;
        let mut rename = None;
        match &sel {
            Sel::State(name) => {
                ui.label(
                    RichText::new(format!("STATE  {name}"))
                        .strong()
                        .color(ACCENT),
                );
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.graph.name)
                            .id(egui::Id::new("graph_rename"))
                            .desired_width(150.0),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let changed = self.graph.name.trim() != name;
                    if (ui
                        .add_enabled(changed, egui::Button::new("Rename"))
                        .clicked()
                        || enter)
                        && changed
                    {
                        rename = Some((name.clone(), self.graph.name.clone()));
                    }
                });
                if *name == initial {
                    ui.label(RichText::new("Initial state").small().color(LIVE));
                } else if ui.small_button("Make initial").clicked() {
                    request = Some(FsmEdit::SetInitial(name.clone()));
                }
                draft.dirty |= motion_form(ui, &mut draft.value, &params, &clips);
            }
            Sel::Transition(i) => {
                ui.label(
                    RichText::new(format!("TRANSITION  #{i}  (priority {})", i + 1))
                        .strong()
                        .color(ACCENT),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(*i > 0, egui::Button::new("Up").small())
                        .on_hover_text("Checked earlier: the first passing transition wins")
                        .clicked()
                    {
                        request = Some(FsmEdit::MoveTransition(*i, i - 1));
                    }
                    if ui
                        .add_enabled(i + 1 < transitions, egui::Button::new("Down").small())
                        .clicked()
                    {
                        request = Some(FsmEdit::MoveTransition(*i, i + 1));
                    }
                });
                draft.dirty |= transition_form(ui, &mut draft.value, &states, &params);
            }
        }
        ui.add_space(6.0);
        let delete = ui
            .button(
                RichText::new(match &sel {
                    Sel::State(_) => "Delete state",
                    Sel::Transition(_) => "Delete transition",
                })
                .color(WARNING),
            )
            .clicked();
        let mut write = None;
        if draft.dirty {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                draft.value = draft.base.clone();
                draft.dirty = false;
            } else if settled(ui, self.saving) {
                write = self.element_edit(&draft);
                draft.base = draft.value.clone();
                draft.dirty = false;
            }
        }
        self.graph.element = Some(draft);
        if let Some(edit) = write {
            actions.extend(self.graph_edit(edit));
        }
        if let Some(edit) = request {
            if let FsmEdit::MoveTransition(_, to) = &edit {
                let to = *to;
                let action = self.graph_edit(edit);
                if action.is_some() {
                    self.graph_select(Some(Sel::Transition(to)));
                }
                actions.extend(action);
            } else {
                actions.extend(self.graph_edit(edit));
            }
        }
        if let Some((from, to)) = rename {
            actions.extend(self.graph_rename(&from, &to));
        }
        if delete {
            actions.extend(self.graph_delete(sel));
        }
    }

    /// The open controller's committed value (with queued graph writes).
    fn graph_element_root(&self) -> Option<&Value> {
        self.graph
            .doc
            .graph(self.graph_world(), self.graph.machine.as_deref()?)
    }

    /// Parameter table: kind, default, `source`, live value and (in Play) a live setter.
    fn anim_params_panel(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let Some(controller) = self.graph.machine.clone() else {
            return;
        };
        let committed = self
            .graph_element_root()
            .map(|c| c.get("parameters").cloned().unwrap_or_else(|| json!({})))
            .unwrap_or_else(|| json!({}));
        let reload = match &self.graph.anim_params {
            Some(d) => d.controller != controller || (!d.dirty && d.value != committed),
            None => true,
        };
        if reload {
            self.graph.anim_params = Some(ParamsDraft {
                controller: controller.clone(),
                base: committed.clone(),
                value: committed,
                dirty: false,
            });
        }
        let mut draft = self.graph.anim_params.take().expect("set above");
        let live = self.graph.anim_live.as_ref();
        ui.horizontal(|ui| {
            ui.label(RichText::new("PARAMETERS").strong().color(ACCENT));
            match live {
                Some(l) => ui.label(
                    RichText::new(format!("live: {} ({} entities)", l.entity, l.entities))
                        .small()
                        .color(LIVE),
                ),
                None => ui.label(RichText::new("live values in Play").small().color(MUTED)),
            };
        });
        let mut remove = None;
        let mut pokes = vec![];
        if let Some(map) = draft.value.as_object_mut() {
            egui::Grid::new("anim_params")
                .num_columns(4)
                .spacing([6.0, 3.0])
                .show(ui, |ui| {
                    for (name, def) in map.iter_mut() {
                        let kind = def["kind"].as_str().unwrap_or("float").to_owned();
                        ui.label(RichText::new(name).strong());
                        ui.label(RichText::new(&kind).small().color(MUTED));
                        ui.horizontal(|ui| match kind.as_str() {
                            "float" => {
                                let mut d = def.get("default").cloned().unwrap_or(json!(0.0));
                                if drag(ui, &mut d, -1000.0..=1000.0, 0.05) {
                                    def["default"] = d;
                                    draft.dirty = true;
                                }
                                let mut bound = def["source"] == "speed";
                                if ui
                                    .checkbox(&mut bound, "speed")
                                    .on_hover_text("Read every step from the entity's planar speed (m/s)")
                                    .changed()
                                {
                                    if bound {
                                        def["source"] = json!("speed");
                                    } else if let Some(o) = def.as_object_mut() {
                                        o.remove("source");
                                    }
                                    draft.dirty = true;
                                }
                            }
                            "bool" => {
                                let mut d = def["default"].as_bool().unwrap_or(false);
                                if ui.checkbox(&mut d, "default").changed() {
                                    def["default"] = json!(d);
                                    draft.dirty = true;
                                }
                            }
                            _ => {
                                ui.label(RichText::new("—").small().color(MUTED));
                            }
                        });
                        ui.horizontal(|ui| {
                            if let Some(l) = live {
                                if let Some(i) = l.names.iter().position(|n| n == name) {
                                    let float = l.floats[i];
                                    let mut v = l.values[i];
                                    if float {
                                        if ui
                                            .add(egui::DragValue::new(&mut v).speed(0.05).max_decimals(2))
                                            .on_hover_text("Live value (Play): drag to set it (animation.param)")
                                            .changed()
                                        {
                                            pokes.push((name.clone(), json!(v)));
                                        }
                                    } else if kind == "trigger" {
                                        let label = if v != 0.0 { "set" } else { "Fire" };
                                        if ui.small_button(label).clicked() {
                                            pokes.push((name.clone(), json!(true)));
                                        }
                                    } else {
                                        let mut b = v != 0.0;
                                        if ui.checkbox(&mut b, "").changed() {
                                            pokes.push((name.clone(), json!(b)));
                                        }
                                    }
                                }
                            }
                            if ui
                                .small_button(RichText::new("✕").color(WARNING))
                                .on_hover_text("Remove parameter (ACL: animctl C param remove NAME)")
                                .clicked()
                            {
                                remove = Some(name.clone());
                            }
                        });
                        ui.end_row();
                    }
                });
        }
        let mut add = None;
        ui.horizontal(|ui| {
            let id = egui::Id::new("anim_param_new");
            let (mut text, mut kind) = ui
                .data(|d| d.get_temp::<(String, usize)>(id))
                .unwrap_or_default();
            ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("new parameter")
                    .desired_width(110.0),
            );
            egui::ComboBox::from_id_salt("anim_param_kind")
                .selected_text(KINDS[kind.min(2)])
                .width(70.0)
                .show_ui(ui, |ui| {
                    for (i, k) in KINDS.iter().enumerate() {
                        ui.selectable_value(&mut kind, i, *k);
                    }
                });
            if ui.button("Add").clicked() && !text.trim().is_empty() {
                add = Some((text.trim().to_owned(), KINDS[kind.min(2)]));
                text.clear();
            }
            ui.data_mut(|d| d.insert_temp(id, (text, kind)));
        });
        // One settled edit = one revision (the parameter whose definition changed).
        let mut write = vec![];
        if draft.dirty && settled(ui, self.saving) {
            if let (Some(base), Some(value)) = (draft.base.as_object(), draft.value.as_object()) {
                for (name, def) in value {
                    if base.get(name) != Some(def) {
                        write.push((name.clone(), def.clone()));
                    }
                }
            }
            draft.base = draft.value.clone();
            draft.dirty = false;
        }
        self.graph.anim_params = Some(draft);
        for (name, def) in write {
            let result = ops::anim_param(self.graph_world(), &controller, &name, Some(&def));
            actions.extend(self.graph_write(result));
        }
        if let Some((name, kind)) = add {
            let result = ops::anim_param(
                self.graph_world(),
                &controller,
                &name,
                Some(&json!({"kind":kind})),
            );
            actions.extend(self.graph_write(result));
        }
        if let Some(name) = remove {
            let result = ops::anim_param(self.graph_world(), &controller, &name, None);
            actions.extend(self.graph_write(result));
        }
        if let Some(entity) = self.graph.anim_live.as_ref().map(|l| l.entity.clone()) {
            for (name, value) in pokes {
                actions.push(Action::PlayClip {
                    capability: "animation".into(),
                    action: "param".into(),
                    args: json!({"entity": entity, "name": name, "value": value}),
                });
            }
        }
    }
}

/// State form: motion kind (rest pose / clip / 1D blend) and its fields.
fn motion_form(
    ui: &mut Ui,
    state: &mut Value,
    params: &[(String, String)],
    clips: &[String],
) -> bool {
    let mut changed = false;
    if !state.is_object() {
        return false;
    }
    let kind = state["motion"]["kind"]
        .as_str()
        .unwrap_or("rest")
        .to_owned();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Motion").small().color(MUTED));
        egui::ComboBox::from_id_salt("anim_motion_kind")
            .selected_text(match kind.as_str() {
                "clip" => "Clip",
                "blend1d" => "1D blend tree",
                _ => "Rest pose",
            })
            .width(120.0)
            .show_ui(ui, |ui| {
                for (k, title) in [("rest", "Rest pose"), ("clip", "Clip"), ("blend1d", "1D blend tree")] {
                    if ui.selectable_label(kind == k, title).clicked() && kind != k {
                        let first_clip = clips.first().cloned().unwrap_or_default();
                        let float = params
                            .iter()
                            .find(|(_, kind)| kind == "float")
                            .map(|(n, _)| n.clone())
                            .unwrap_or_default();
                        let o = state.as_object_mut().expect("checked");
                        match k {
                            "clip" => {
                                o.insert("motion".into(), json!({"kind":"clip","clip":first_clip}));
                            }
                            "blend1d" => {
                                o.insert(
                                    "motion".into(),
                                    json!({"kind":"blend1d","parameter":float,"points":[{"at":0.0,"clip":first_clip}]}),
                                );
                            }
                            _ => {
                                o.remove("motion");
                            }
                        }
                        changed = true;
                    }
                }
            });
    });
    let Some(motion) = state.get_mut("motion").filter(|m| m.is_object()) else {
        return changed;
    };
    match kind.as_str() {
        "clip" => {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Clip").small().color(MUTED));
                changed |= clip_combo(ui, "anim_clip", clips, &mut motion["clip"]);
            });
        }
        "blend1d" => {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Parameter").small().color(MUTED));
                let current = motion["parameter"].as_str().unwrap_or("").to_owned();
                egui::ComboBox::from_id_salt("anim_blend_param")
                    .selected_text(if current.is_empty() {
                        "(float)"
                    } else {
                        current.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for (n, k) in params {
                            if k == "float"
                                && ui.selectable_label(*n == current, n).clicked()
                                && *n != current
                            {
                                motion["parameter"] = json!(n);
                                changed = true;
                            }
                        }
                    });
            });
            let mut remove = None;
            if let Some(points) = motion["points"].as_array_mut() {
                let n = points.len();
                for (i, p) in points.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("at").small().color(MUTED));
                        changed |= drag(ui, &mut p["at"], -1000.0..=1000.0, 0.05);
                        changed |= clip_combo(ui, ("anim_point", i), clips, &mut p["clip"]);
                        if n > 1 && ui.small_button("✕").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    points.remove(i);
                    changed = true;
                }
                if n < 8 && ui.small_button("+ Point").clicked() {
                    let at = points.last().and_then(|p| p["at"].as_f64()).unwrap_or(0.0) + 1.0;
                    let clip = clips.first().cloned().unwrap_or_default();
                    points.push(json!({"at": at, "clip": clip}));
                    changed = true;
                }
                ui.label(
                    RichText::new("Points must increase; clips stay in step by normalized time")
                        .small()
                        .color(MUTED),
                );
            }
        }
        _ => {}
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Speed").small().color(MUTED));
        let mut speed = motion.get("speed").cloned().unwrap_or(json!(1.0));
        if drag(ui, &mut speed, 0.0..=8.0, 0.02) {
            if speed.as_f64() == Some(1.0) {
                motion.as_object_mut().expect("object").remove("speed");
            } else {
                motion["speed"] = speed;
            }
            changed = true;
        }
        let mut looping = motion["looping"].as_bool().unwrap_or(true);
        if ui.checkbox(&mut looping, "loop").changed() {
            if looping {
                motion.as_object_mut().expect("object").remove("looping");
            } else {
                motion["looping"] = json!(false);
            }
            changed = true;
        }
    });
    changed
}

/// Transition form: ends, conditions, cross-fade and exit time.
fn transition_form(
    ui: &mut Ui,
    t: &mut Value,
    states: &[String],
    params: &[(String, String)],
) -> bool {
    let mut changed = false;
    for (end, any) in [("from", true), ("to", false)] {
        ui.horizontal(|ui| {
            ui.label(RichText::new(end).small().color(MUTED));
            let current = t[end].as_str().unwrap_or("").to_owned();
            egui::ComboBox::from_id_salt(("anim_end", end))
                .selected_text(if current == "*" {
                    "Any state"
                } else {
                    current.as_str()
                })
                .show_ui(ui, |ui| {
                    let options = any
                        .then_some("*")
                        .into_iter()
                        .chain(states.iter().map(String::as_str));
                    for s in options {
                        let title = if s == "*" { "Any state" } else { s };
                        if ui.selectable_label(s == current, title).clicked() && s != current {
                            t[end] = json!(s);
                            changed = true;
                        }
                    }
                });
        });
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Cross-fade s").small().color(MUTED));
        let mut d = t.get("duration").cloned().unwrap_or(json!(0.0));
        if drag(ui, &mut d, 0.0..=10.0, 0.01) {
            t["duration"] = d;
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        let mut on = t.get("exit_time").is_some_and(|e| !e.is_null());
        if ui
            .checkbox(&mut on, "Exit time")
            .on_hover_text("Also wait until the source state's normalized time reaches this (1 = one full cycle)")
            .changed()
        {
            if on {
                t["exit_time"] = json!(1.0);
            } else if let Some(o) = t.as_object_mut() {
                o.remove("exit_time");
            }
            changed = true;
        }
        if on {
            changed |= drag(ui, &mut t["exit_time"], 0.0..=1000.0, 0.01);
        }
    });
    ui.label(
        RichText::new("Conditions (all must pass)")
            .small()
            .color(MUTED),
    );
    let list = t
        .as_object_mut()
        .expect("transitions are objects")
        .entry("conditions")
        .or_insert_with(|| json!([]));
    let mut remove = None;
    if let Some(conds) = list.as_array_mut() {
        for (i, c) in conds.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let current = c["param"].as_str().unwrap_or("").to_owned();
                egui::ComboBox::from_id_salt(("anim_cond_param", i))
                    .selected_text(if current.is_empty() {
                        "?"
                    } else {
                        current.as_str()
                    })
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for (n, k) in params {
                            if ui.selectable_label(*n == current, n).clicked() && *n != current {
                                c["param"] = json!(n);
                                c["op"] = json!(match k.as_str() {
                                    "bool" => "true",
                                    "trigger" => "trigger",
                                    _ => "gt",
                                });
                                changed = true;
                            }
                        }
                    });
                let kind = params
                    .iter()
                    .find(|(n, _)| *n == current)
                    .map_or("float", |(_, k)| k.as_str());
                let allowed: &[&str] = match kind {
                    "bool" => &["true", "false"],
                    "trigger" => &["trigger"],
                    _ => &["gt", "lt", "eq", "ne"],
                };
                let op = c["op"].as_str().unwrap_or("").to_owned();
                egui::ComboBox::from_id_salt(("anim_cond_op", i))
                    .selected_text(OPS.iter().find(|(o, _)| *o == op).map_or("?", |o| o.1))
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for (o, title) in OPS.iter().filter(|(o, _)| allowed.contains(o)) {
                            if ui.selectable_label(*o == op, *title).clicked() && *o != op {
                                c["op"] = json!(o);
                                changed = true;
                            }
                        }
                    });
                if kind == "float" {
                    let mut v = c.get("value").cloned().unwrap_or(json!(0.0));
                    if drag(ui, &mut v, -1000.0..=1000.0, 0.05) {
                        c["value"] = v;
                        changed = true;
                    }
                }
                if ui.small_button("✕").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            conds.remove(i);
            changed = true;
        }
        if conds.len() < 8 && !params.is_empty() && ui.small_button("+ Condition").clicked() {
            let (n, k) = &params[0];
            let op = match k.as_str() {
                "bool" => "true",
                "trigger" => "trigger",
                _ => "gt",
            };
            conds.push(json!({"param": n, "op": op, "value": 0.0}));
            changed = true;
        }
        if params.is_empty() {
            ui.label(
                RichText::new("Add a parameter above to use conditions")
                    .small()
                    .color(MUTED),
            );
        }
    }
    if list.as_array().is_some_and(Vec::is_empty) {
        t.as_object_mut().expect("object").remove("conditions");
    }
    changed
}
