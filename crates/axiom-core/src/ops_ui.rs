//! `ui.document` operations (re-exported from [`crate::ops`]): place a widget in a screen
//! region, set its text style, pick a button's command, edit widget fields and add / edit /
//! remove data-driven UI effects. Pure like the rest of `ops`: `fn(&World, ..) ->
//! Result<Vec<Edit>>`, shared by the editor's UI Editor tab and the ACL `ui` verbs.
use crate::gameplay::{
    Program, Region, RuntimeCommand, SlideDirection, TextStyle, UiDocument, UiEffect, UiEffectKind,
    UiTrigger, Widget, MAX_UI_EFFECTS,
};
use crate::local::{escape, Target};
use crate::*;
use anyhow::Result;
use serde_json::{json, Value};

pub const UI_DOCUMENT: &str = "ui.document";
/// The runtime system that runs `ui.document.effects` (registered by the runtime).
pub const UI_EFFECTS_SYSTEM: &str = "ui.effects";
const DOC_PATH: &str = "/resources/ui.document";

fn invalid(path: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}

/// The authored document (raw, to keep everything the edit does not touch) and its typed form.
fn document(world: &World) -> Result<(Value, UiDocument)> {
    let raw = world.resources.get(UI_DOCUMENT).ok_or_else(|| {
        Diagnostic::error(
            "not_found",
            DOC_PATH,
            "the world has no ui.document (resource create ui.document first)",
        )
    })?;
    // The ops edit the authored JSON in place and need the object form (serde would also
    // decode an array as the struct).
    if !raw.is_object() {
        return Err(invalid(DOC_PATH, "ui.document must be a JSON object"));
    }
    let typed = serde_json::from_value(raw.clone())
        .map_err(|e| invalid(DOC_PATH, format!("ui.document does not parse: {e}")))?;
    Ok((raw.clone(), typed))
}

fn widget_path(id: &str) -> String {
    format!("{DOC_PATH}/widgets/@{}", escape(id))
}

fn find_widget<'a>(doc: &'a UiDocument, id: &str) -> Result<(usize, &'a Widget)> {
    doc.widgets
        .iter()
        .enumerate()
        .find(|(_, w)| w.id() == id)
        .ok_or_else(|| {
            Diagnostic::error(
                "not_found",
                widget_path(id),
                format!(
                    "ui.document has no widget '{id}' (widgets: {})",
                    doc.widgets
                        .iter()
                        .map(Widget::id)
                        .take(16)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })
}

fn put(raw: Value) -> Vec<Edit> {
    vec![Edit::Core(CoreEdit::SetResource {
        id: UI_DOCUMENT.into(),
        value: raw,
    })]
}

/// Sets or clears `raw[map][widget]`; an emptied map is removed like the typed form omits it.
/// No edit when nothing changes.
fn map_entry(mut raw: Value, map: &str, widget: &str, value: Option<Value>) -> Vec<Edit> {
    let before = raw.get(map).and_then(|m| m.get(widget)).cloned();
    if before == value {
        return vec![];
    }
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    match value {
        Some(v) => {
            object
                .entry(map)
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .map(|m| m.insert(widget.to_owned(), v));
        }
        None => {
            let empty = object
                .get_mut(map)
                .and_then(Value::as_object_mut)
                .map(|m| {
                    m.remove(widget);
                    m.is_empty()
                })
                .unwrap_or(false);
            if empty {
                object.remove(map);
            }
        }
    }
    put(raw)
}

pub fn region_name(r: Region) -> String {
    json!(r).as_str().unwrap_or_default().to_owned()
}
pub fn style_name(s: TextStyle) -> String {
    json!(s).as_str().unwrap_or_default().to_owned()
}
pub const REGION_NAMES: [&str; 9] = [
    "top_left",
    "top",
    "top_right",
    "left",
    "center",
    "right",
    "bottom_left",
    "bottom",
    "bottom_right",
];
pub const STYLE_NAMES: [&str; 6] = ["title", "heading", "banner", "value", "label", "muted"];

/// `REGION` or `none` (unplaced: the bottom document panel).
pub fn parse_region(text: &str) -> Result<Option<Region>> {
    if text == "none" {
        return Ok(None);
    }
    serde_json::from_value(json!(text)).map(Some).map_err(|_| {
        invalid(
            "/regions",
            format!(
                "unknown region '{text}' (use {} or none)",
                REGION_NAMES.join(", ")
            ),
        )
    })
}

/// `STYLE` or `none` (the default look).
pub fn parse_style(text: &str) -> Result<Option<TextStyle>> {
    if text == "none" {
        return Ok(None);
    }
    serde_json::from_value(json!(text)).map(Some).map_err(|_| {
        invalid(
            "/styles",
            format!(
                "unknown style '{text}' (use {} or none)",
                STYLE_NAMES.join(", ")
            ),
        )
    })
}

/// UI Editor ▸ drag a widget to a region / ACL `ui place WIDGET REGION|none`.
pub fn set_ui_region(world: &World, widget: &str, region: Option<Region>) -> Result<Vec<Edit>> {
    let (raw, doc) = document(world)?;
    find_widget(&doc, widget)?;
    Ok(map_entry(raw, "regions", widget, region.map(|r| json!(r))))
}

/// UI Editor ▸ Style / ACL `ui style WIDGET STYLE|none`. Text and progress widgets only.
pub fn set_ui_style(world: &World, widget: &str, style: Option<TextStyle>) -> Result<Vec<Edit>> {
    let (raw, doc) = document(world)?;
    let (_, w) = find_widget(&doc, widget)?;
    if style.is_some() && !matches!(w, Widget::Text { .. } | Widget::Progress { .. }) {
        return Err(invalid(
            widget_path(widget),
            format!("'{widget}' is not a text or progress widget; styles apply to those"),
        ));
    }
    Ok(map_entry(raw, "styles", widget, style.map(|s| json!(s))))
}

/// UI Editor ▸ button Command / ACL `ui button WIDGET emit|locale|choose|call ...`.
pub fn set_ui_button_command(
    world: &World,
    widget: &str,
    command: &RuntimeCommand,
) -> Result<Vec<Edit>> {
    let (mut raw, doc) = document(world)?;
    let (i, w) = find_widget(&doc, widget)?;
    if !matches!(w, Widget::Button { .. }) {
        return Err(invalid(
            widget_path(widget),
            format!("'{widget}' is not a button"),
        ));
    }
    if let Ok(program) = world.resource::<Program>("behavior.program") {
        let story = world
            .resources
            .get("narrative.graph")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        crate::local::at(
            &format!("{}/command", widget_path(widget)),
            crate::gameplay::validate_command(command, &program, story.as_ref(), Some(&doc)),
        )?;
    }
    let value = json!(command);
    if raw["widgets"][i]["command"] == value {
        return Ok(vec![]);
    }
    raw["widgets"][i]["command"] = value;
    Ok(put(raw))
}

/// UI Editor ▸ Add Slider / ACL `ui slider add ID TEXT MIN MAX [k=v..]`: appends a slider
/// widget. The label key must exist in the default catalog; the world's compile checks the
/// rest (variable type, command, range) when the revision commits.
pub fn add_ui_slider(world: &World, slider: &Widget) -> Result<Vec<Edit>> {
    let (mut raw, doc) = document(world)?;
    let Widget::Slider { id, text, .. } = slider else {
        return Err(invalid(
            format!("{DOC_PATH}/widgets"),
            "not a slider widget",
        ));
    };
    valid_id(id).map_err(|e| invalid(format!("{DOC_PATH}/widgets"), format!("{e:#}")))?;
    if doc.widgets.iter().any(|w| w.id() == id) {
        return Err(Diagnostic::error(
            "id_taken",
            format!("{DOC_PATH}/widgets"),
            format!("ui.document already has a widget '{id}'"),
        ));
    }
    if !doc
        .catalogs
        .get(&doc.locale)
        .is_some_and(|c| c.contains_key(text))
    {
        return Err(invalid(
            format!("{DOC_PATH}/catalogs/{}", escape(&doc.locale)),
            format!("missing localization key {text} (loc key add {text} TEXT first)"),
        ));
    }
    raw.as_object_mut()
        .expect("ui.document parsed as an object")
        .entry("widgets")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| invalid(format!("{DOC_PATH}/widgets"), "widgets is not a list"))?
        .push(json!(slider));
    Ok(put(raw))
}

/// Builds a slider from ACL options: `variable=V`, `step=S`, `value=X`,
/// `call=CAPABILITY.ACTION`, `args={..}`, `arg=NAME`, `when=CONDITION_JSON`.
pub fn slider_widget(
    id: &str,
    text: &str,
    min: f64,
    max: f64,
    options: &[(String, String)],
) -> Result<Widget> {
    let number = |k: &str, v: &str| -> Result<f64> {
        v.parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| anyhow::anyhow!("{k} must be a number, not '{v}'"))
    };
    let (mut step, mut variable, mut value, mut call, mut args, mut arg) =
        (None, None, None, None, None, None);
    let mut when = crate::gameplay::Condition::default();
    for (k, v) in options {
        match k.as_str() {
            "step" => step = Some(number(k, v)?),
            "value" => value = Some(number(k, v)?),
            "variable" => variable = Some(v.clone()),
            "arg" => arg = Some(v.clone()),
            "call" => {
                let (c, a) = v
                    .split_once('.')
                    .ok_or_else(|| anyhow::anyhow!("call must be CAPABILITY.ACTION"))?;
                call = Some((c.to_owned(), a.to_owned()));
            }
            "args" => {
                args = Some(
                    serde_json::from_str::<Value>(v)
                        .map_err(|e| anyhow::anyhow!("args must be a JSON object: {e}"))?,
                )
            }
            "when" => {
                when = serde_json::from_str(v)
                    .map_err(|e| anyhow::anyhow!("when must be a condition JSON: {e}"))?
            }
            other => anyhow::bail!(
                "unknown slider option '{other}' (variable, step, value, call, args, arg, when)"
            ),
        }
    }
    if args.is_some() && call.is_none() {
        anyhow::bail!("args go with call=CAPABILITY.ACTION");
    }
    let command = call.map(|(capability, action)| RuntimeCommand::Call {
        capability,
        action,
        args: args.unwrap_or_else(|| json!({})),
    });
    Ok(Widget::Slider {
        id: id.to_owned(),
        text: text.to_owned(),
        min,
        max,
        step,
        variable,
        value,
        command,
        arg,
        when,
    })
}

/// UI Editor ▸ element form: changed fields of one widget (pointers relative to the widget),
/// the same `set_field`s as ACL `set widgets[id=WIDGET] F VALUE`.
pub fn set_ui_widget_fields(
    world: &World,
    widget: &str,
    fields: &[(String, Value)],
) -> Result<Vec<Edit>> {
    let (_, doc) = document(world)?;
    find_widget(&doc, widget)?;
    Ok(fields
        .iter()
        .map(|(field, value)| {
            Edit::Core(CoreEdit::SetField {
                target: Target::Widget {
                    id: widget.to_owned(),
                },
                field: field.clone(),
                value: value.clone(),
            })
        })
        .collect())
}

/// `{widget}.{kind}`, then `/2`, `/3`.. until unused.
pub fn free_ui_effect_id(world: &World, widget: &str, kind: UiEffectKind) -> String {
    let taken: Vec<String> = document(world)
        .map(|(_, d)| d.effects.into_iter().map(|e| e.id).collect())
        .unwrap_or_default();
    // IDs are at most 128 bytes; leave room for the kind and a `/N` suffix.
    let mut end = widget.len().min(100);
    while !widget.is_char_boundary(end) {
        end -= 1;
    }
    let base = format!("{}.{}", &widget[..end], kind.name());
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}/{n}"))
        .find(|id| !taken.contains(id))
        .unwrap()
}

fn effect_path(i: usize) -> String {
    format!("{DOC_PATH}/effects/{i}")
}

/// UI Editor ▸ Add effect / ACL `ui effect add ID WIDGET KIND [k=v..]`. Adds the `ui.effects`
/// system (and what it runs after) when the world lacks it and the registry has it.
pub fn add_ui_effect(world: &World, registry: &Registry, effect: &UiEffect) -> Result<Vec<Edit>> {
    let (mut raw, doc) = document(world)?;
    valid_id(&effect.id).map_err(|e| invalid("/id", format!("{e:#}")))?;
    if doc.effects.iter().any(|e| e.id == effect.id) {
        return Err(Diagnostic::error(
            "id_taken",
            format!("{DOC_PATH}/effects"),
            format!("UI effect '{}' already exists", effect.id),
        ));
    }
    if doc.effects.len() >= MAX_UI_EFFECTS {
        return Err(invalid(
            format!("{DOC_PATH}/effects"),
            format!("at most {MAX_UI_EFFECTS} UI effects"),
        ));
    }
    find_widget(&doc, &effect.widget)?;
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    object
        .entry("effects")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| invalid(format!("{DOC_PATH}/effects"), "effects is not a list"))?
        .push(json!(effect));
    let mut edits = put(raw);
    if registry.systems.contains_key(UI_EFFECTS_SYSTEM)
        && !world.systems.iter().any(|s| s == UI_EFFECTS_SYSTEM)
    {
        edits.extend(crate::ops::add_system(world, registry, UI_EFFECTS_SYSTEM)?);
    }
    Ok(edits)
}

/// UI Editor ▸ effect form: replaces the effect with the same ID.
pub fn set_ui_effect(world: &World, effect: &UiEffect) -> Result<Vec<Edit>> {
    let (mut raw, doc) = document(world)?;
    let i = effect_index(&doc, &effect.id)?;
    find_widget(&doc, &effect.widget)?;
    let value = json!(effect);
    if raw["effects"][i] == value {
        return Ok(vec![]);
    }
    raw["effects"][i] = value;
    Ok(put(raw))
}

/// ACL `ui effect set ID [k=v..]`: the effect with options changed (see [`apply_effect_options`]).
pub fn edit_ui_effect(world: &World, id: &str, options: &[(String, String)]) -> Result<Vec<Edit>> {
    let (_, doc) = document(world)?;
    let i = effect_index(&doc, id)?;
    let mut effect = doc.effects[i].clone();
    crate::local::at(&effect_path(i), apply_effect_options(&mut effect, options))?;
    set_ui_effect(world, &effect)
}

/// UI Editor ▸ effect x / ACL `ui effect remove ID`. The `ui.effects` system stays scheduled.
pub fn remove_ui_effect(world: &World, id: &str) -> Result<Vec<Edit>> {
    let (mut raw, doc) = document(world)?;
    let i = effect_index(&doc, id)?;
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    let empty = object
        .get_mut("effects")
        .and_then(Value::as_array_mut)
        .map(|list| {
            list.remove(i);
            list.is_empty()
        })
        .unwrap_or(false);
    if empty {
        object.remove("effects");
    }
    Ok(put(raw))
}

fn effect_index(doc: &UiDocument, id: &str) -> Result<usize> {
    doc.effects.iter().position(|e| e.id == id).ok_or_else(|| {
        Diagnostic::error(
            "not_found",
            format!("{DOC_PATH}/effects"),
            format!(
                "no UI effect '{id}' (effects: {})",
                doc.effects
                    .iter()
                    .map(|e| e.id.as_str())
                    .take(16)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    })
}

fn word<T: serde::de::DeserializeOwned>(key: &str, v: &str, options: &[&str]) -> Result<T> {
    serde_json::from_value(json!(v)).map_err(|_| {
        anyhow::anyhow!(
            "unknown {key} '{v}' (use {}, or default)",
            options.join(", ")
        )
    })
}

/// Applies `key=value` options to an effect: `kind`, `widget`, `on`
/// (show|hide|change|event|when), `event`, `when` (condition JSON), `ticks`, `ease`,
/// `direction`, `amount`, `from`. `default` clears an option back to the kind's default.
pub fn apply_effect_options(effect: &mut UiEffect, options: &[(String, String)]) -> Result<()> {
    for (k, v) in options {
        let reset = v == "default";
        match k.as_str() {
            "kind" => {
                let names: Vec<&str> = UiEffectKind::ALL.iter().map(|k| k.name()).collect();
                effect.kind = word("kind", v, &names)?;
            }
            "widget" => effect.widget = v.clone(),
            "on" if reset => effect.on = None,
            "on" => {
                let names: Vec<&str> = UiTrigger::ALL.iter().map(|t| t.name()).collect();
                effect.on = Some(word("trigger", v, &names)?);
            }
            "event" if reset => effect.event = None,
            "event" => {
                valid_id(v)?;
                effect.event = Some(v.clone());
            }
            "when" if reset => effect.when = Default::default(),
            "when" => {
                effect.when = serde_json::from_str(v)
                    .map_err(|e| anyhow::anyhow!("when must be a condition JSON: {e}"))?
            }
            "ticks" if reset => effect.ticks = None,
            "ticks" => {
                effect.ticks = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("ticks must be a whole number"))?,
                )
            }
            "ease" if reset => effect.ease = None,
            "ease" => effect.ease = Some(crate::tween::Ease::parse(v)?),
            "direction" if reset => effect.direction = None,
            "direction" => {
                effect.direction = Some(word::<SlideDirection>(
                    "direction",
                    v,
                    &["left", "right", "up", "down"],
                )?)
            }
            "amount" if reset => effect.amount = None,
            "amount" => {
                effect.amount = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("amount must be a number"))?,
                )
            }
            "from" if reset => effect.from = None,
            "from" => {
                effect.from = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("from must be a number"))?,
                )
            }
            other => anyhow::bail!(
                "unknown effect option '{other}' (kind, widget, on, event, when, ticks, ease, direction, amount, from)"
            ),
        }
    }
    // `event=E` alone means "on that event".
    if effect.event.is_some() && effect.on.is_none() {
        effect.on = Some(UiTrigger::Event);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// ui.document v2, plan M1: theme frames, panel styles and anchored panels. Each op checks
// the edited document with the same validation `compile` runs, so a refusal names the
// exact field (`/resources/ui.document/theme/...`).
// ---------------------------------------------------------------------------------------

/// Sets or removes `theme.<map>.<id>`, dropping empty maps (and an empty theme).
fn theme_entry(mut raw: Value, map: &str, id: &str, value: Option<Value>) -> Vec<Edit> {
    let before = raw
        .get("theme")
        .and_then(|t| t.get(map))
        .and_then(|m| m.get(id))
        .cloned();
    if before == value {
        return vec![];
    }
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    match value {
        Some(v) => {
            let theme = object.entry("theme").or_insert_with(|| json!({}));
            if let Some(t) = theme.as_object_mut() {
                let m = t.entry(map).or_insert_with(|| json!({}));
                if let Some(m) = m.as_object_mut() {
                    m.insert(id.to_owned(), v);
                }
            }
        }
        None => {
            if let Some(t) = object.get_mut("theme").and_then(Value::as_object_mut) {
                let empty = t
                    .get_mut(map)
                    .and_then(Value::as_object_mut)
                    .map(|m| {
                        m.remove(id);
                        m.is_empty()
                    })
                    .unwrap_or(false);
                if empty {
                    t.remove(map);
                }
                if t.is_empty() {
                    object.remove("theme");
                }
            }
        }
    }
    put(raw)
}

/// The edited document must still pass the theme / panel checks.
fn checked(raw: &Value) -> Result<()> {
    let doc: UiDocument = serde_json::from_value(raw.clone())
        .map_err(|e| invalid(DOC_PATH, format!("ui.document does not parse: {e}")))?;
    // Templates first (plan M5): panels and classes may name widgets only instances create.
    let doc = crate::ui_template::expand(&doc)?;
    let names: std::collections::BTreeSet<&str> = doc.widgets.iter().map(|w| w.id()).collect();
    crate::ui_theme::validate(&doc, &names)
}

fn checked_edits(edits: Vec<Edit>) -> Result<Vec<Edit>> {
    if let Some(Edit::Core(CoreEdit::SetResource { value, .. })) = edits.first() {
        checked(value)?;
    }
    Ok(edits)
}

/// UI Editor ▸ theme frame / ACL `ui frame ID IMAGE L T R B | ui frame ID none`.
/// Removing a frame a panel style still uses is refused.
pub fn set_ui_frame(
    world: &World,
    id: &str,
    frame: Option<crate::ui_theme::UiFrame>,
) -> Result<Vec<Edit>> {
    let (raw, _) = document(world)?;
    checked_edits(theme_entry(raw, "frames", id, frame.map(|f| json!(f))))
}

/// UI Editor ▸ panel style / ACL `ui panelstyle CLASS [fill=] [corner=] [padding=] [frame=] | none`.
pub fn set_ui_panel_style(
    world: &World,
    class: &str,
    style: Option<crate::ui_theme::PanelStyle>,
) -> Result<Vec<Edit>> {
    let (raw, _) = document(world)?;
    checked_edits(theme_entry(raw, "panel", class, style.map(|s| json!(s))))
}

/// UI Editor ▸ panel / ACL `ui panel ID ANCHOR [at=X,Y] [width=] [style=] [widgets=A,B] | none`.
/// Replaces the panel with that ID in place (keeping its z order) or appends a new one.
pub fn set_ui_panel(
    world: &World,
    id: &str,
    panel: Option<crate::ui_theme::UiPanel>,
) -> Result<Vec<Edit>> {
    let (mut raw, doc) = document(world)?;
    let at = doc.panels.iter().position(|p| p.id == id);
    if at.is_none() && panel.is_none() {
        return Err(Diagnostic::error(
            "not_found",
            format!("{DOC_PATH}/panels"),
            format!("no panel '{id}'"),
        ));
    }
    if at.map(|i| &doc.panels[i]) == panel.as_ref() {
        return Ok(vec![]);
    }
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    let panels = object.entry("panels").or_insert_with(|| json!([]));
    let list = panels
        .as_array_mut()
        .ok_or_else(|| invalid(format!("{DOC_PATH}/panels"), "panels must be an array"))?;
    match (at, panel) {
        (Some(i), Some(p)) => list[i] = json!(p),
        (None, Some(p)) => list.push(json!(p)),
        (Some(i), None) => {
            list.remove(i);
        }
        (None, None) => unreachable!("refused above"),
    }
    if list.is_empty() {
        object.remove("panels");
    }
    checked_edits(put(raw))
}

/// UI Editor ▸ screen / ACL `ui screen W H [scale=MODE] | ui screen none` (plan M3): the
/// reference screen the panels are authored for, and how it maps to the window.
pub fn set_ui_screen(
    world: &World,
    reference: Option<[f32; 2]>,
    scale: Option<crate::ui_theme::UiScale>,
) -> Result<Vec<Edit>> {
    let (mut raw, _) = document(world)?;
    let before = raw.get("theme").cloned();
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    let theme = object.entry("theme").or_insert_with(|| json!({}));
    if let Some(t) = theme.as_object_mut() {
        match reference {
            Some(r) => t.insert("reference".into(), json!(r)),
            None => t.remove("reference"),
        };
        match scale {
            Some(s) => t.insert("scale".into(), json!(s)),
            None => t.remove("scale"),
        };
    }
    if theme.as_object().is_some_and(|t| t.is_empty()) {
        object.remove("theme");
    }
    if raw.get("theme") == before.as_ref() {
        return Ok(vec![]);
    }
    checked_edits(put(raw))
}

/// UI Editor ▸ nameplates / ACL `ui nameplate [fade=M] [hide=M] [class=C] [style=S] | ui nameplate
/// none` (plan N1): the default look and fade / hide distances of `ui.nameplate` tags.
pub fn set_ui_nameplate(
    world: &World,
    nameplate: Option<crate::ui_theme::NameplateTheme>,
) -> Result<Vec<Edit>> {
    let (mut raw, _) = document(world)?;
    let before = raw.get("theme").cloned();
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    let theme = object.entry("theme").or_insert_with(|| json!({}));
    if let Some(t) = theme.as_object_mut() {
        match nameplate.filter(|n| n != &Default::default()) {
            Some(n) => t.insert("nameplate".into(), json!(n)),
            None => t.remove("nameplate"),
        };
    }
    if theme.as_object().is_some_and(|t| t.is_empty()) {
        object.remove("theme");
    }
    if raw.get("theme") == before.as_ref() {
        return Ok(vec![]);
    }
    checked_edits(put(raw))
}

/// UI Editor ▸ font / ACL `ui font ID PATH license=SPDX | ui font ID none` (plan M4). Removing a
/// font a class still uses is refused.
pub fn set_ui_font(
    world: &World,
    id: &str,
    font: Option<crate::ui_theme::UiFont>,
) -> Result<Vec<Edit>> {
    let (raw, _) = document(world)?;
    checked_edits(theme_entry(raw, "fonts", id, font.map(|f| json!(f))))
}

/// UI Editor ▸ page / ACL `ui page ID panels=A,B [modal=] [pause=] [escape=] | ui page ID none`
/// (plan M6). Removing a page a button still opens is refused by the command check.
pub fn set_ui_page(
    world: &World,
    id: &str,
    page: Option<crate::ui_theme::UiPage>,
) -> Result<Vec<Edit>> {
    let (raw, _) = document(world)?;
    checked_edits(map_entry(raw, "pages", id, page.map(|p| json!(p))))
}

/// UI Editor ▸ template / ACL `ui template ID {JSON} | ui template ID none` (plan M5).
/// Removing a template a use still names is refused.
pub fn set_ui_template(
    world: &World,
    id: &str,
    template: Option<crate::ui_template::UiTemplate>,
) -> Result<Vec<Edit>> {
    let (raw, _) = document(world)?;
    checked_edits(map_entry(raw, "templates", id, template.map(|t| json!(t))))
}

/// UI Editor ▸ use / ACL `ui use ID TEMPLATE [into=] [count=] [INSTANCES] | ui use ID none`
/// (plan M5). Replaces the use with that ID in place (expansion order) or appends one.
pub fn set_ui_use(
    world: &World,
    id: &str,
    entry: Option<crate::ui_template::UiUse>,
) -> Result<Vec<Edit>> {
    let (mut raw, doc) = document(world)?;
    let at = doc.uses.iter().position(|u| u.id == id);
    if at.is_none() && entry.is_none() {
        return Err(Diagnostic::error(
            "not_found",
            format!("{DOC_PATH}/uses"),
            format!("no use '{id}'"),
        ));
    }
    if at.map(|i| &doc.uses[i]) == entry.as_ref() {
        return Ok(vec![]);
    }
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    let list = object.entry("uses").or_insert_with(|| json!([]));
    let list = list
        .as_array_mut()
        .ok_or_else(|| invalid(format!("{DOC_PATH}/uses"), "uses must be an array"))?;
    match (at, entry) {
        (Some(i), Some(u)) => list[i] = json!(u),
        (None, Some(u)) => list.push(json!(u)),
        (Some(i), None) => {
            list.remove(i);
        }
        (None, None) => unreachable!("refused above"),
    }
    if list.is_empty() {
        object.remove("uses");
    }
    checked_edits(put(raw))
}

/// UI Editor ▸ text class / ACL `ui textclass CLASS [size=] [color=] [shadow=] | none` (plan M2).
/// UI Editor ▸ progress class / ACL `ui progressclass CLASS … | ui progressclass CLASS none`
/// (plan U1). Removing a class a progress widget still uses is refused.
pub fn set_ui_progress_class(
    world: &World,
    class: &str,
    progress: Option<crate::ui_theme::ProgressClass>,
) -> Result<Vec<Edit>> {
    let (raw, _) = document(world)?;
    checked_edits(theme_entry(
        raw,
        "progress",
        class,
        progress.map(|p| json!(p)),
    ))
}

/// UI Editor ▸ prompt / ACL `ui prompt [class=C] [style=S] | ui prompt none` (plan U1): the look
/// of the engine's default interaction prompt.
pub fn set_ui_prompt(
    world: &World,
    prompt: Option<crate::ui_theme::PromptTheme>,
) -> Result<Vec<Edit>> {
    let (mut raw, _) = document(world)?;
    let before = raw.get("theme").cloned();
    let object = raw
        .as_object_mut()
        .expect("ui.document parsed as an object");
    let theme = object.entry("theme").or_insert_with(|| json!({}));
    if let Some(t) = theme.as_object_mut() {
        match prompt.filter(|p| p != &Default::default()) {
            Some(p) => t.insert("prompt".into(), json!(p)),
            None => t.remove("prompt"),
        };
    }
    if theme.as_object().is_some_and(|t| t.is_empty()) {
        object.remove("theme");
    }
    if raw.get("theme") == before.as_ref() {
        return Ok(vec![]);
    }
    checked_edits(put(raw))
}

pub fn set_ui_text_class(
    world: &World,
    class: &str,
    text: Option<crate::ui_theme::TextClass>,
) -> Result<Vec<Edit>> {
    let (raw, _) = document(world)?;
    checked_edits(theme_entry(raw, "text", class, text.map(|t| json!(t))))
}

/// UI Editor ▸ button class / ACL `ui buttonclass CLASS [size=] [padding=] [STATE.frame|fill|text=] | none`.
pub fn set_ui_button_class(
    world: &World,
    class: &str,
    button: Option<crate::ui_theme::ButtonClass>,
) -> Result<Vec<Edit>> {
    let (raw, _) = document(world)?;
    checked_edits(theme_entry(raw, "button", class, button.map(|b| json!(b))))
}

/// UI Editor ▸ widget class / ACL `ui class WIDGET CLASS|none`: a theme text class for text
/// widgets, a button class for buttons.
pub fn set_ui_class(world: &World, widget: &str, class: Option<&str>) -> Result<Vec<Edit>> {
    let (raw, doc) = document(world)?;
    find_widget(&doc, widget)?;
    checked_edits(map_entry(raw, "classes", widget, class.map(|c| json!(c))))
}
