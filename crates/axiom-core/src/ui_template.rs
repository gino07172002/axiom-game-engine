//! `ui.document` templates (plan M5): a template is a small set of widgets (and optionally a
//! panel holding them) whose strings may contain `{PARAM}` and `{i}` placeholders; a use
//! instantiates it once per parameter set. Expansion happens when the document is compiled,
//! so bindings, effects, classes, panels and the runtime see ordinary widgets and panels.
//!
//! Instances are appended after the authored widgets and panels, in use order. A use either
//! appends each instance's panel (template with a panel) or its widgets (template without
//! one) to the `into` panel; without `into`, each instance panel is a root panel.
//! `count` names a number variable: instance `i` is shown while that variable is > i.
use crate::gameplay::{UiDocument, Widget};
use crate::ui_theme::UiPanel;
use crate::{valid_id, Diagnostic};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MAX_TEMPLATES: usize = 16;
pub const MAX_TEMPLATE_WIDGETS: usize = 16;
pub const MAX_PARAMS: usize = 8;
pub const MAX_USES: usize = 32;
pub const MAX_INSTANCES: usize = 32;
/// Widgets a document may have after expansion (the document widget budget).
pub const MAX_EXPANDED_WIDGETS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiTemplate {
    /// Parameter names every instance must set (`{NAME}` in the template's strings).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
    /// Widgets in `ui.document` widget form; string fields may use `{PARAM}` and `{i}`.
    pub widgets: Vec<Value>,
    /// Optional panel (`ui.document` panel form without `children`) holding the widgets in
    /// order; its `id` should use a parameter or `{i}` when a use has several instances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<Value>,
    /// Template widget ID -> theme class (both may use placeholders).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub classes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiUse {
    pub id: String,
    pub template: String,
    /// One parameter set per instance, 1..=32.
    pub instances: Vec<BTreeMap<String, String>>,
    /// Panel that receives the instances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// Number variable: instance `i` is shown while it is > i.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<String>,
}

const DOC: &str = "/resources/ui.document";
fn invalid(path: String, message: String) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}

/// A parameter value: short, and only characters that are safe inside IDs, keys and events.
fn valid_value(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
}

fn substitute(value: &mut Value, params: &BTreeMap<String, String>, index: usize) {
    match value {
        Value::String(s) if s.contains('{') => *s = fill(s, params, index),
        Value::Array(items) => items.iter_mut().for_each(|v| substitute(v, params, index)),
        Value::Object(map) => map.values_mut().for_each(|v| substitute(v, params, index)),
        _ => {}
    }
}

fn fill(text: &str, params: &BTreeMap<String, String>, index: usize) -> String {
    let mut out = text.replace("{i}", &index.to_string());
    for (k, v) in params {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// The document with every use expanded and `templates` / `uses` removed. Errors name the
/// field (`/resources/ui.document/uses/0/instances/2`, `.../templates/T/widgets/1`).
pub fn expand(doc: &UiDocument) -> Result<UiDocument> {
    if doc.templates.is_empty() && doc.uses.is_empty() {
        return Ok(doc.clone());
    }
    if doc.templates.len() > MAX_TEMPLATES || doc.uses.len() > MAX_USES {
        return Err(invalid(
            format!("{DOC}/templates"),
            format!("at most {MAX_TEMPLATES} templates and {MAX_USES} uses"),
        ));
    }
    for (id, t) in &doc.templates {
        let path = format!("{DOC}/templates/{id}");
        valid_id(id).map_err(|e| invalid(path.clone(), e.to_string()))?;
        if t.widgets.is_empty() || t.widgets.len() > MAX_TEMPLATE_WIDGETS {
            return Err(invalid(
                format!("{path}/widgets"),
                format!("a template has 1..={MAX_TEMPLATE_WIDGETS} widgets"),
            ));
        }
        if t.params.len() > MAX_PARAMS {
            return Err(invalid(
                format!("{path}/params"),
                format!("at most {MAX_PARAMS} parameters"),
            ));
        }
        for p in &t.params {
            valid_id(p).map_err(|e| invalid(format!("{path}/params"), e.to_string()))?;
            if p == "i" {
                return Err(invalid(
                    format!("{path}/params"),
                    "'i' is the instance index; pick another parameter name".into(),
                ));
            }
        }
        if t.panel
            .as_ref()
            .is_some_and(|p| p.get("children").is_some())
        {
            return Err(invalid(
                format!("{path}/panel/children"),
                "a template panel's children are its widgets; leave children out".into(),
            ));
        }
    }
    // Budget before building anything: instances x template widgets.
    let mut total = doc.widgets.len();
    for u in &doc.uses {
        if let Some(t) = doc.templates.get(&u.template) {
            total += u.instances.len().min(MAX_INSTANCES) * t.widgets.len();
        }
    }
    if total > MAX_EXPANDED_WIDGETS {
        return Err(invalid(
            format!("{DOC}/uses"),
            format!("{total} widgets after expansion (at most {MAX_EXPANDED_WIDGETS})"),
        ));
    }
    let mut out = doc.clone();
    out.templates.clear();
    out.uses.clear();
    for (n, u) in doc.uses.iter().enumerate() {
        let path = format!("{DOC}/uses/{n}");
        valid_id(&u.id).map_err(|e| invalid(format!("{path}/id"), e.to_string()))?;
        if doc.uses[..n].iter().any(|o| o.id == u.id) {
            return Err(invalid(
                format!("{path}/id"),
                format!("duplicate use id '{}'", u.id),
            ));
        }
        let Some(t) = doc.templates.get(&u.template) else {
            return Err(invalid(
                format!("{path}/template"),
                format!(
                    "unknown template '{}' (templates: {})",
                    u.template,
                    doc.templates.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
            ));
        };
        if u.instances.is_empty() || u.instances.len() > MAX_INSTANCES {
            return Err(invalid(
                format!("{path}/instances"),
                format!("a use has 1..={MAX_INSTANCES} instances"),
            ));
        }
        if t.panel.is_none() && u.into.is_none() {
            return Err(invalid(
                format!("{path}/into"),
                format!(
                    "template '{}' has no panel: name the panel its widgets go into",
                    u.template
                ),
            ));
        }
        if let Some(var) = &u.count {
            valid_id(var).map_err(|e| invalid(format!("{path}/count"), e.to_string()))?;
        }
        for (i, params) in u.instances.iter().enumerate() {
            let at = format!("{path}/instances/{i}");
            for p in &t.params {
                if !params.contains_key(p) {
                    return Err(invalid(at, format!("missing parameter '{p}'")));
                }
            }
            for (k, v) in params {
                if !t.params.contains(k) {
                    return Err(invalid(
                        at,
                        format!(
                            "unknown parameter '{k}' (template '{}' takes {})",
                            u.template,
                            t.params.join(", ")
                        ),
                    ));
                }
                if !valid_value(v) {
                    return Err(invalid(
                        format!("{at}/{k}"),
                        "parameter values are 1..=64 of A-Z a-z 0-9 _ - . /".into(),
                    ));
                }
            }
            let mut ids = vec![];
            for (j, w) in t.widgets.iter().enumerate() {
                let mut v = w.clone();
                substitute(&mut v, params, i);
                if let Some(var) = &u.count {
                    let kind = v.get("kind").and_then(Value::as_str).unwrap_or_default();
                    if matches!(kind, "text" | "button" | "slider") {
                        let shown = json!({"kind": "ge",
                            "left": {"kind": "variable", "id": var},
                            "right": {"kind": "literal", "value": i + 1}});
                        let when = match v.get("when") {
                            Some(w) => json!({"kind": "all", "conditions": [w, shown]}),
                            None => shown,
                        };
                        v["when"] = when;
                    }
                }
                let widget: Widget = serde_json::from_value(v).map_err(|e| {
                    invalid(
                        format!("{DOC}/templates/{}/widgets/{j}", u.template),
                        format!("instance {i} of use '{}': {e}", u.id),
                    )
                })?;
                ids.push(widget.id().to_owned());
                out.widgets.push(widget);
            }
            for (k, c) in &t.classes {
                out.classes.insert(fill(k, params, i), fill(c, params, i));
            }
            let placed = match &t.panel {
                Some(panel) => {
                    let mut v = panel.clone();
                    substitute(&mut v, params, i);
                    v["children"] = json!(ids);
                    let p: UiPanel = serde_json::from_value(v).map_err(|e| {
                        invalid(
                            format!("{DOC}/templates/{}/panel", u.template),
                            format!("instance {i} of use '{}': {e}", u.id),
                        )
                    })?;
                    let id = p.id.clone();
                    out.panels.push(p);
                    vec![id]
                }
                None => ids,
            };
            if let Some(into) = &u.into {
                let Some(target) = out.panels.iter_mut().find(|p| &p.id == into) else {
                    return Err(invalid(
                        format!("{path}/into"),
                        format!("no panel '{into}' (authored or made by an earlier use)"),
                    ));
                };
                target.children.extend(placed);
            }
        }
    }
    Ok(out)
}

/// `ui use ID TEMPLATE [into=PANEL] [count=VAR] [INSTANCES]`: INSTANCES is a JSON list of
/// parameter objects (default `[{}]`, one instance of a template without parameters).
pub fn use_from(
    id: &str,
    template: &str,
    options: &[(String, String)],
    instances: Option<Value>,
) -> Result<UiUse> {
    let mut u = UiUse {
        id: id.to_owned(),
        template: template.to_owned(),
        instances: vec![BTreeMap::new()],
        into: None,
        count: None,
    };
    for (k, v) in options {
        match k.as_str() {
            "into" => u.into = Some(v.clone()),
            "count" => u.count = Some(v.clone()),
            _ => anyhow::bail!("unknown use option '{k}' (into, count)"),
        }
    }
    if let Some(list) = instances {
        u.instances = serde_json::from_value(list).map_err(|e| {
            anyhow::anyhow!("instances must be a JSON list of {{\"PARAM\":\"value\"}} objects: {e}")
        })?;
    }
    Ok(u)
}
