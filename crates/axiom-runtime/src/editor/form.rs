//! Schema-guided component forms. Widgets follow the value's JSON shape; registry and
//! world schemas supply per-field hints. Only changed leaves are written back, as
//! `set_field` edits, so untouched prefab-inherited fields stay inherited.
use egui::{RichText, Ui};
use serde_json::Value;

const MUTED: egui::Color32 = egui::Color32::from_rgb(138, 154, 172);

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Hint text for one field: registry descriptors are `{field: "range text"}`;
/// world `RecordSchema`s are `{fields: {field: {kind, ...}}}`.
fn hint(hints: Option<&Value>, key: &str) -> Option<String> {
    let h = hints?;
    let v = h
        .get(key)
        .or_else(|| h.get("fields").and_then(|f| f.get(key)))?;
    Some(match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

fn number(ui: &mut Ui, v: &mut Value) -> bool {
    if let Some(i) = v.as_i64().filter(|_| !v.is_f64()) {
        let mut x = i;
        let changed = ui.add(egui::DragValue::new(&mut x).speed(1.0)).changed();
        if changed {
            *v = Value::from(x);
        }
        return changed;
    }
    let mut x = v.as_f64().unwrap_or(0.0);
    let speed = (x.abs() * 0.01).max(0.01);
    let changed = ui
        .add(egui::DragValue::new(&mut x).speed(speed).max_decimals(4))
        .changed();
    if changed && x.is_finite() {
        *v = serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number);
    }
    changed
}

fn small_numeric_array(v: &Value) -> bool {
    v.as_array()
        .is_some_and(|a| (1..=4).contains(&a.len()) && a.iter().all(Value::is_number))
}

/// Renders an editable form for `value`; returns true when anything changed.
pub(super) fn edit(ui: &mut Ui, id: &str, value: &mut Value, hints: Option<&Value>) -> bool {
    ui.push_id(id, |ui| field(ui, value, hints, 0)).inner
}

fn field(ui: &mut Ui, value: &mut Value, hints: Option<&Value>, depth: usize) -> bool {
    let mut changed = false;
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                let child = map.get_mut(&key).unwrap();
                let tip = hint(hints, &key);
                let nested = matches!(child, Value::Object(_))
                    || (child.is_array() && !small_numeric_array(child));
                if nested && depth < 6 {
                    let header = egui::CollapsingHeader::new(&key)
                        .id_salt((&key, depth))
                        .default_open(depth == 0);
                    let response = header.show(ui, |ui| {
                        field(ui, child, hints.and_then(|h| h.get(&key)), depth + 1)
                    });
                    if let Some(tip) = tip {
                        response.header_response.on_hover_text(tip);
                    }
                    changed |= response.body_returned.unwrap_or(false);
                } else {
                    ui.horizontal(|ui| {
                        let label = ui.label(RichText::new(&key).small().color(MUTED));
                        if let Some(tip) = &tip {
                            label.on_hover_text(tip);
                        }
                        changed |= if is_color(&key, child) {
                            color(ui, child)
                        } else {
                            leaf(ui, child)
                        };
                    });
                    if let Some(tip) = tip.filter(|t| t.len() <= 48) {
                        ui.label(RichText::new(tip).small().weak());
                    }
                }
            }
        }
        Value::Array(items) if !small_numeric_array(&Value::Array(items.clone())) => {
            for (i, item) in items.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    ui.label(RichText::new(format!("[{i}]")).small().color(MUTED));
                    ui.indent("item", |ui| changed |= field(ui, item, None, depth + 1));
                });
            }
        }
        other => changed |= leaf(ui, other),
    }
    changed
}

/// `color`/`tint`-named fields holding 3–4 numbers in 0..=1 get a color picker.
pub(super) fn is_color(key: &str, v: &Value) -> bool {
    let key = key.to_ascii_lowercase();
    (key.contains("color") || key.contains("tint") || key == "accent")
        && v.as_array().is_some_and(|a| {
            (3..=4).contains(&a.len())
                && a.iter()
                    .all(|x| x.as_f64().is_some_and(|x| (0.0..=1.0).contains(&x)))
        })
}

/// Color picker that writes back numbers rounded to 4 decimals (stable diffs).
pub(super) fn color(ui: &mut Ui, v: &mut Value) -> bool {
    let Some(a) = v.as_array() else { return false };
    let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
    let round = |x: f32| ((x as f64) * 10000.0).round() / 10000.0;
    let changed = if c.len() == 4 {
        let mut rgba = [c[0], c[1], c[2], c[3]];
        let changed = ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed();
        if changed {
            *v = serde_json::json!(rgba.map(round));
        }
        changed
    } else {
        let mut rgb = [c[0], c[1], c[2]];
        let changed = ui.color_edit_button_rgb(&mut rgb).changed();
        if changed {
            *v = serde_json::json!(rgb.map(round));
        }
        changed
    };
    ui.label(RichText::new(v.to_string()).small().weak());
    changed
}

fn leaf(ui: &mut Ui, v: &mut Value) -> bool {
    match v {
        Value::Bool(b) => ui.checkbox(b, "").changed(),
        Value::Number(_) => number(ui, v),
        Value::String(s) => ui
            .add(egui::TextEdit::singleline(s).desired_width(160.0))
            .changed(),
        Value::Array(items) => {
            let mut changed = false;
            for item in items.iter_mut() {
                changed |= number(ui, item);
            }
            changed
        }
        Value::Null => {
            ui.label(RichText::new("null").small().weak());
            false
        }
        Value::Object(_) => {
            ui.label(RichText::new("{…}").small().weak());
            false
        }
    }
}

/// Changed leaves between two versions of the same shape, as (JSON pointer, new value).
/// Shape changes (different types or array lengths) replace the whole subtree.
pub(super) fn changes(base: &Value, next: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match (base, next) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                let path = format!("{prefix}/{}", escape(k));
                match a.get(k) {
                    Some(old) => changes(old, v, &path, out),
                    None => out.push((path, v.clone())),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (old, new)) in a.iter().zip(b).enumerate() {
                changes(old, new, &format!("{prefix}/{i}"), out);
            }
        }
        _ if base != next => out.push((prefix.to_owned(), next.clone())),
        _ => {}
    }
}

/// Registry descriptors keyed by component ID, merged with the world's typed schemas.
pub(super) fn hints_for(
    registry: &std::collections::BTreeMap<String, Value>,
    world: &axiom_core::World,
    component: &str,
) -> Option<Value> {
    if let Some(schema) = world.component_schemas.get(component) {
        return serde_json::to_value(schema).ok();
    }
    registry.get(component).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_changed_leaves_are_reported() {
        let base =
            json!({"speed":1.8,"damage":8,"shape":{"radius":0.4,"kind":"capsule"},"p":[0,1,2]});
        let mut next = base.clone();
        next["speed"] = json!(2.5);
        next["shape"]["radius"] = json!(0.5);
        next["p"][1] = json!(3);
        let mut out = vec![];
        changes(&base, &next, "", &mut out);
        assert_eq!(
            out,
            vec![
                ("/p/1".to_owned(), json!(3)),
                ("/shape/radius".to_owned(), json!(0.5)),
                ("/speed".to_owned(), json!(2.5)),
            ]
        );
        let mut none = vec![];
        changes(&base, &base, "", &mut none);
        assert!(none.is_empty());
    }
    #[test]
    fn hints_read_descriptor_and_record_shapes() {
        assert_eq!(
            hint(Some(&json!({"speed":"0..12"})), "speed").as_deref(),
            Some("0..12")
        );
        assert!(hint(Some(&json!({"fields":{"hp":{"kind":"int"}}})), "hp")
            .unwrap()
            .contains("int"));
        assert!(hint(None, "x").is_none());
    }
}
