//! Asset-level inspectors: shader materials (range sliders from the shader's declared
//! parameters), prefab definitions (schema forms), and read-only behavior/narrative graphs.
use super::{form, Action, Editor};
use axiom_core::{local::Target, CoreEdit, Edit, Request, World};
use egui::{Color32, RichText, ScrollArea, Ui};
use serde_json::{json, Value};

const MUTED: Color32 = Color32::from_rgb(138, 154, 172);
const WARNING: Color32 = Color32::from_rgb(242, 176, 92);

fn escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

pub(super) struct MaterialDraft {
    pub rev: i64,
    pub id: String,
    pub(super) base: Value,
    pub(super) value: Value,
}
impl MaterialDraft {
    pub fn open(world: &World, rev: i64, id: &str) -> Option<Self> {
        let value = world
            .resources
            .get("render.shaders")?
            .pointer(&format!("/materials/{}", escape(id)))?;
        Some(Self {
            rev,
            id: id.into(),
            base: value.clone(),
            value: value.clone(),
        })
    }
    pub fn dirty(&self) -> bool {
        self.base != self.value
    }
    /// Whole-subtree writes for changed top-level material fields (parameters may be absent
    /// in authored JSON, so a leaf pointer could have no parent).
    pub(super) fn edits(&self) -> Vec<Edit> {
        let mut out = vec![];
        for key in [
            "tint",
            "parameters",
            "alpha",
            "alpha_cutoff",
            "double_sided",
        ] {
            if self.base.get(key) != self.value.get(key) {
                if let Some(v) = self.value.get(key) {
                    out.push(Edit::Core(CoreEdit::SetField {
                        target: Target::Resource {
                            id: "render.shaders".into(),
                        },
                        field: format!("/materials/{}/{key}", escape(&self.id)),
                        value: v.clone(),
                    }));
                }
            }
        }
        out
    }
}

pub(super) struct PrefabDraft {
    pub rev: i64,
    pub id: String,
    pub(super) component: String,
    pub(super) base: Value,
    pub(super) value: Option<Value>,
}
impl PrefabDraft {
    pub fn open(id: &str, rev: i64) -> Self {
        Self {
            rev,
            id: id.into(),
            component: String::new(),
            base: Value::Null,
            value: None,
        }
    }
    pub fn dirty(&self) -> bool {
        self.value.as_ref().is_some_and(|v| *v != self.base)
    }
    pub(super) fn edits(&self) -> Vec<Edit> {
        let Some(value) = self.value.as_ref() else {
            return vec![];
        };
        let mut leaves = vec![];
        form::changes(&self.base, value, "", &mut leaves);
        leaves
            .into_iter()
            .map(|(pointer, value)| {
                Edit::Core(CoreEdit::SetField {
                    target: Target::Prefab {
                        id: self.id.clone(),
                    },
                    field: format!("/components/{}{pointer}", escape(&self.component)),
                    value,
                })
            })
            .collect()
    }
}

fn buttons(
    ui: &mut Ui,
    enabled: bool,
    edits: impl Fn() -> Vec<Edit>,
    rev: i64,
    actions: &mut Vec<Action>,
) -> bool {
    let mut reload = false;
    ui.horizontal(|ui| {
        for (title, preview) in [("Apply", false), ("Preview", true)] {
            if ui.add_enabled(enabled, egui::Button::new(title)).clicked() {
                let edits = edits();
                let expected_rev = Some(rev);
                actions.push(Action::Write {
                    request: if preview {
                        Request::Preview {
                            expected_rev,
                            edits,
                        }
                    } else {
                        Request::Apply {
                            expected_rev,
                            edits,
                        }
                    },
                    select: None,
                });
            }
        }
        reload = ui.button("Reload").clicked();
    });
    reload
}

impl Editor {
    pub(super) fn open_material(&mut self, id: &str) {
        self.select(None);
        self.material = MaterialDraft::open(&self.world, self.rev, id);
    }
    pub(super) fn open_prefab(&mut self, id: &str) {
        self.select(None);
        if self.world.prefabs.contains_key(id) {
            self.prefab = Some(PrefabDraft::open(id, self.rev));
        }
    }

    pub(super) fn material_inspector(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let world_rev = self.rev;
        let Some(draft) = self.material.as_mut() else {
            return;
        };
        let stale = draft.rev != world_rev;
        ui.label(
            RichText::new(format!("material · {}", draft.id))
                .monospace()
                .color(Color32::from_rgb(105, 222, 195)),
        );
        if stale {
            ui.label(
                RichText::new("Stale draft · reload to see the latest material").color(WARNING),
            );
        }
        let shader_id = draft.value["shader"].as_str().unwrap_or("").to_owned();
        ui.label(
            RichText::new(format!("shader {shader_id}"))
                .small()
                .color(MUTED),
        );
        let declared = self
            .world
            .resources
            .get("render.shaders")
            .and_then(|r| r.pointer(&format!("/shaders/{}/parameters", escape(&shader_id))))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let dirty = draft.dirty();
        let reload = {
            let d = &*draft;
            buttons(ui, dirty && !stale, || d.edits(), d.rev, actions)
        };
        ui.separator();
        ScrollArea::vertical()
            .id_salt("material_scroll")
            .show(ui, |ui| {
                ui.add_enabled_ui(!stale, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("tint").small().color(MUTED));
                        let mut tint = draft
                            .value
                            .get("tint")
                            .cloned()
                            .unwrap_or(json!([1.0, 1.0, 1.0, 1.0]));
                        if form::color(ui, &mut tint) {
                            draft.value["tint"] = tint;
                        }
                    });
                    for (name, p) in &declared {
                        let (min, max) = (
                            p["min"].as_f64().unwrap_or(0.0),
                            p["max"].as_f64().unwrap_or(1.0),
                        );
                        let current = draft.value["parameters"]
                            .get(name)
                            .and_then(Value::as_f64)
                            .or_else(|| p["default"].as_f64())
                            .unwrap_or(min);
                        let mut x = current;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).small().color(MUTED));
                            if ui.add(egui::Slider::new(&mut x, min..=max)).changed() {
                                if !draft.value["parameters"].is_object() {
                                    draft.value["parameters"] = json!({});
                                }
                                draft.value["parameters"][name] =
                                    json!((x * 10000.0).round() / 10000.0);
                            }
                        });
                    }
                    if declared.is_empty() {
                        ui.label(
                            RichText::new("This shader declares no parameters.")
                                .small()
                                .color(MUTED),
                        );
                    }
                    if let Some(cutoff) = draft.value.get("alpha_cutoff").and_then(Value::as_f64) {
                        let mut c = cutoff;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("alpha_cutoff").small().color(MUTED));
                            if ui.add(egui::Slider::new(&mut c, 0.0..=1.0)).changed() {
                                draft.value["alpha_cutoff"] = json!(c);
                            }
                        });
                    }
                });
            });
        if reload {
            let id = draft.id.clone();
            self.open_material(&id);
        }
    }

    pub(super) fn prefab_inspector(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let world_rev = self.rev;
        let world = self.world.clone();
        let hints = self.component_hints.clone();
        let Some(draft) = self.prefab.as_mut() else {
            return;
        };
        let stale = draft.rev != world_rev;
        ui.label(
            RichText::new(format!("prefab · {}", draft.id))
                .monospace()
                .color(Color32::from_rgb(105, 222, 195)),
        );
        let instances = world
            .entities
            .iter()
            .filter(|e| e.prefab.as_deref() == Some(draft.id.as_str()))
            .count();
        ui.label(
            RichText::new(format!(
                "{instances} instance(s) inherit these values unless they override them"
            ))
            .small()
            .color(MUTED),
        );
        if stale {
            ui.label(RichText::new("Stale draft · reload").color(WARNING));
        }
        let Some(prefab) = world.prefabs.get(&draft.id) else {
            ui.label(RichText::new("Prefab was removed in another revision.").color(WARNING));
            return;
        };
        let dirty = draft.dirty();
        let reload = {
            let d = &*draft;
            buttons(ui, dirty && !stale, || d.edits(), d.rev, actions)
        };
        ui.separator();
        ui.add_enabled_ui(!dirty, |ui| {
            ui.horizontal_wrapped(|ui| {
                for key in prefab.components.keys() {
                    if ui
                        .selectable_label(draft.component == *key, key.as_str())
                        .clicked()
                    {
                        draft.component = key.clone();
                        draft.base = prefab.components[key].clone();
                        draft.value = Some(draft.base.clone()).filter(Value::is_object);
                    }
                }
            });
        });
        ScrollArea::vertical().id_salt("prefab_scroll").show(ui, |ui| {
            ui.add_enabled_ui(!stale, |ui| match &mut draft.value {
                Some(value) => {
                    let hint = form::hints_for(&hints, &world, &draft.component);
                    form::edit(ui, &draft.component, value, hint.as_ref());
                }
                None if !draft.component.is_empty() => {
                    ui.label(RichText::new("Non-object component; edit it as JSON from an instance or the resource view.").small().color(MUTED));
                }
                None => {
                    ui.label(RichText::new("Pick a component to edit its prefab default.").small().color(MUTED));
                }
            });
        });
        if reload {
            let id = draft.id.clone();
            self.open_prefab(&id);
        }
    }

    /// Read-only overview of behavior and narrative graphs authored in this world.
    pub(super) fn graphs(&self, ui: &mut Ui) {
        let r = &self.world.resources;
        let mut any = false;
        if let Some(machines) = r
            .get("fsm.machines")
            .and_then(|m| m["machines"].as_object())
        {
            any = true;
            for (id, m) in machines {
                egui::CollapsingHeader::new(RichText::new(format!("fsm · {id}")).monospace())
                    .default_open(true)
                    .show(ui, |ui| {
                        for (state, def) in m["states"].as_object().into_iter().flatten() {
                            let initial = m["initial"] == state.as_str();
                            ui.label(
                                RichText::new(format!(
                                    "{}{state}  [{}]",
                                    if initial { "▶ " } else { "  " },
                                    def["action"]["kind"].as_str().unwrap_or("stop")
                                ))
                                .monospace()
                                .small(),
                            );
                            for t in m["transitions"].as_array().into_iter().flatten() {
                                if t["from"] == state.as_str()
                                    || (t["from"] == "*" && t["to"] != state.as_str())
                                {
                                    let when = &t["when"];
                                    ui.label(
                                        RichText::new(format!(
                                            "      ──{}──▶ {}",
                                            compact_condition(when),
                                            t["to"].as_str().unwrap_or("?")
                                        ))
                                        .monospace()
                                        .small()
                                        .color(MUTED),
                                    );
                                }
                            }
                        }
                    });
            }
        }
        if let Some(graph) = r.get("narrative.graph") {
            any = true;
            egui::CollapsingHeader::new(RichText::new("narrative.graph").monospace())
                .default_open(true)
                .show(ui, |ui| {
                    let start = graph["start"].as_str().unwrap_or("");
                    for node in graph["nodes"].as_array().into_iter().flatten() {
                        let id = node["id"].as_str().unwrap_or("?");
                        let text: String = node["text"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(48)
                            .collect();
                        let ending = node["ending"]
                            .as_str()
                            .map(|e| format!("  ⟂ ending {e}"))
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(format!(
                                "{}{id}  {}: {text}{ending}",
                                if id == start { "▶ " } else { "  " },
                                node["speaker"].as_str().unwrap_or("")
                            ))
                            .monospace()
                            .small(),
                        );
                        for c in node["choices"].as_array().into_iter().flatten() {
                            ui.label(
                                RichText::new(format!(
                                    "      ── {} ──▶ {}",
                                    c["id"].as_str().unwrap_or("?"),
                                    c["next"].as_str().unwrap_or("?")
                                ))
                                .monospace()
                                .small()
                                .color(MUTED),
                            );
                        }
                    }
                });
        }
        if let Some(rules) = r
            .get("behavior.program")
            .and_then(|p| p["rules"].as_array())
        {
            any = true;
            egui::CollapsingHeader::new(RichText::new("behavior.program rules").monospace()).show(
                ui,
                |ui| {
                    for rule in rules {
                        ui.label(
                            RichText::new(format!(
                                "{}  on {}",
                                rule["id"].as_str().unwrap_or("?"),
                                rule["event"].as_str().unwrap_or("tick")
                            ))
                            .monospace()
                            .small(),
                        );
                    }
                },
            );
        }
        if !any {
            ui.label(
                RichText::new(
                    "No fsm.machines, narrative.graph or behavior.program in this world.",
                )
                .small()
                .color(MUTED),
            );
        }
    }
}

fn compact_condition(when: &Value) -> String {
    let kind = when["kind"].as_str().unwrap_or("?");
    match kind {
        "near" => format!(
            "near({})<={}",
            when["tag"].as_str().unwrap_or("?"),
            when["within"]
        ),
        "far" => format!(
            "far({})>{}",
            when["tag"].as_str().unwrap_or("?"),
            when["beyond"]
        ),
        "health_below" => format!("health<{}", when["value"]),
        "in_state_ticks" => format!("ticks>={}", when["ticks"]),
        "chance" => format!("chance {}", when["per_tick"]),
        other => other.to_owned(),
    }
}
