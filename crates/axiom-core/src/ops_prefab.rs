//! Prefab overrides field by field (Inspector override markers, ACL `prefab revert|apply`).
//!
//! An instance's component value is a merge patch over its prefab's ([`crate::merge_json`]):
//! objects merge key by key, anything else (numbers, strings, arrays) replaces the prefab's
//! value as a whole. A field "overrides the prefab" when the instance's patch provides it.
use crate::local::escape;
use crate::*;
use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

fn entity_path(id: &str) -> String {
    format!("/entities/@{}", escape(id))
}

fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

/// `/a/b~1c` -> ["a", "b/c"]; "" -> [] (the whole value).
fn segments(pointer: &str) -> Result<Vec<String>> {
    if pointer.is_empty() {
        return Ok(vec![]);
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err(Diagnostic::error(
            "invalid_field",
            pointer,
            "a field is a JSON pointer starting with /",
        ));
    };
    Ok(rest.split('/').map(unescape).collect())
}

fn walk(own: &Value, base: Option<&Value>, pointer: &str, out: &mut Vec<String>) {
    match (own, base) {
        (Value::Object(own), Some(Value::Object(base))) => {
            for (key, value) in own {
                walk(
                    value,
                    base.get(key),
                    &format!("{pointer}/{}", escape(key)),
                    out,
                );
            }
        }
        _ => out.push(pointer.to_owned()),
    }
}

/// For a prefab instance: each component its prefab also has -> the JSON pointers (relative
/// to the component value, escaped) of the fields the instance overrides, in key order.
/// Components the prefab lacks are the instance's own, not overrides. Empty for an object
/// that is not an instance.
pub fn prefab_overrides(world: &World, id: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(entity) = world.entity(id) else {
        return out;
    };
    let Some(prefab) = entity.prefab.as_ref().and_then(|p| world.prefabs.get(p)) else {
        return out;
    };
    for (component, own) in &entity.components {
        let Some(base) = prefab.components.get(component) else {
            continue;
        };
        let mut fields = vec![];
        walk(own, Some(base), "", &mut fields);
        if !fields.is_empty() {
            out.insert(component.clone(), fields);
        }
    }
    out
}

/// Removes the field at `path` from `value` (objects only), then any object left empty on
/// the way. False when the field is not there.
fn remove_at(value: &mut Value, path: &[String]) -> bool {
    let Value::Object(map) = value else {
        return false;
    };
    match path {
        [] => false,
        [key] => map.remove(key).is_some(),
        [key, rest @ ..] => {
            let Some(child) = map.get_mut(key) else {
                return false;
            };
            if !remove_at(child, rest) {
                return false;
            }
            if child.as_object().is_some_and(Map::is_empty) {
                map.remove(key);
            }
            true
        }
    }
}

/// Sets `path` in `value` to `field`, creating (or replacing non-object) parents as objects.
fn set_at(value: &mut Value, path: &[String], field: Value) {
    let Some((last, parents)) = path.split_last() else {
        *value = field;
        return;
    };
    let mut at = value;
    for key in parents {
        if !at.is_object() {
            *at = Value::Object(Map::new());
        }
        at = at
            .as_object_mut()
            .expect("made an object above")
            .entry(key.clone())
            .or_insert(Value::Null);
    }
    if !at.is_object() {
        *at = Value::Object(Map::new());
    }
    at.as_object_mut()
        .expect("made an object above")
        .insert(last.clone(), field);
}

/// The edit that drops `pointer` from `id`'s override of `component` (`None` when the
/// instance does not override it).
fn revert_edit(entity: &Entity, component: &str, path: &[String]) -> Option<Edit> {
    let own = entity.components.get(component)?;
    let mut rest = own.clone();
    let emptied = if path.is_empty() {
        true
    } else {
        if !remove_at(&mut rest, path) {
            return None;
        }
        rest.as_object().is_some_and(Map::is_empty)
    };
    Some(Edit::Core(if emptied {
        CoreEdit::RemoveComponent {
            entity: entity.id.clone(),
            component: component.to_owned(),
        }
    } else {
        CoreEdit::SetComponent {
            entity: entity.id.clone(),
            component: component.to_owned(),
            value: rest,
        }
    }))
}

fn inherited<'a>(
    world: &'a World,
    entity: &Entity,
    component: &str,
) -> Option<(&'a str, &'a Prefab)> {
    let (id, prefab) = world.prefabs.get_key_value(entity.prefab.as_deref()?)?;
    prefab
        .components
        .contains_key(component)
        .then_some((id.as_str(), prefab))
}

/// Inspector ▸ field ▸ Revert to Prefab / ACL `prefab revert SEL COMPONENT/FIELD`: each
/// selected instance stops overriding `pointer` of `component` (escaped JSON pointer relative
/// to the component; "" = the whole override, like `reset`), so the prefab's value shows
/// again. An override left empty is removed. Selected objects that do not inherit the
/// component, or do not override the field, are left as they are (like the Inspector, which
/// acts on every selected object that has a field); an error when none inherits it
/// (`not_prefab_field`) or none overrides it (`not_overridden`).
pub fn revert_prefab_field(
    world: &World,
    ids: &[String],
    component: &str,
    pointer: &str,
) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "nothing selected",
        ));
    }
    let path = segments(pointer)?;
    let mut edits = vec![];
    let mut instances = 0;
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let entity = world.entity(id).ok_or_else(|| {
            Diagnostic::error("not_found", entity_path(id), "entity ID not found")
        })?;
        if inherited(world, entity, component).is_none() {
            continue;
        }
        instances += 1;
        edits.extend(revert_edit(entity, component, &path));
    }
    if instances == 0 {
        return Err(Diagnostic::error(
            "not_prefab_field",
            format!("/components/{}", escape(component)),
            format!("no selected object inherits {component} from a prefab"),
        ));
    }
    if edits.is_empty() {
        return Err(Diagnostic::error(
            "not_overridden",
            format!("/components/{}{pointer}", escape(component)),
            format!(
                "{component}{pointer} is not overridden (a field inside an overridden array or value reverts with it): it already shows the prefab's value"
            ),
        ));
    }
    Ok(edits)
}

/// Inspector ▸ field ▸ Apply to Prefab / ACL `prefab apply ID COMPONENT/FIELD`: the
/// instance's value of the overridden field becomes the prefab's (every instance that does
/// not override it changes too) and the instance stops overriding it, in one revision.
pub fn apply_prefab_field(
    world: &World,
    id: &str,
    component: &str,
    pointer: &str,
) -> Result<Vec<Edit>> {
    let path = segments(pointer)?;
    let entity = world
        .entity(id)
        .ok_or_else(|| Diagnostic::error("not_found", entity_path(id), "entity ID not found"))?;
    let Some((prefab_id, prefab)) = inherited(world, entity, component) else {
        return Err(Diagnostic::error(
            "not_prefab_field",
            format!("{}/components/{}", entity_path(id), escape(component)),
            format!("'{id}' does not inherit {component} from a prefab"),
        ));
    };
    let Some(revert) = revert_edit(entity, component, &path) else {
        return Err(Diagnostic::error(
            "not_overridden",
            format!(
                "{}/components/{}{pointer}",
                entity_path(id),
                escape(component)
            ),
            format!("{component}{pointer} is not overridden on '{id}'"),
        ));
    };
    let base = &prefab.components[component];
    let mut effective = base.clone();
    merge_json(&mut effective, &entity.components[component]);
    let value = effective
        .pointer(pointer)
        .cloned()
        .expect("an overridden field is in the merged value");
    let mut updated = prefab.clone();
    set_at(
        updated
            .components
            .get_mut(component)
            .expect("inherited above"),
        &path,
        value,
    );
    Ok(vec![
        Edit::Core(CoreEdit::PutPrefab {
            id: prefab_id.to_owned(),
            prefab: updated,
        }),
        revert,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn code(e: anyhow::Error) -> String {
        e.downcast_ref::<Diagnostic>().unwrap().code.clone()
    }

    fn world() -> World {
        let mut w = World::empty("t");
        let mut lamp = Prefab {
            components: Default::default(),
        };
        lamp.components.insert(
            "core.transform".into(),
            json!({"position":[0.0,0.0,0.0],"scale":[1.0,1.0,1.0]}),
        );
        lamp.components.insert(
            "x.light".into(),
            json!({"color":{"r":1,"g":1},"a/b":1,"parts":[1,2]}),
        );
        w.prefabs.insert("lamp".into(), lamp);
        for (id, own) in [
            (
                "a",
                json!({"core.transform":{"position":[4.0,0.0,1.0]},"x.light":{"color":{"r":0.5},"a/b":2,"parts":[3]},"x.own":{"k":1}}),
            ),
            ("b", json!({"x.light":{"color":{"g":0.2}}})),
            ("c", json!({})),
        ] {
            let mut e = Entity::new(id);
            e.prefab = Some("lamp".into());
            e.components = serde_json::from_value(own).unwrap();
            w.entities.push(e);
        }
        // An object with the component of its own (no prefab).
        let mut plain = Entity::new("plain");
        plain
            .components
            .insert("x.light".into(), json!({"color":{"r":0.1}}));
        w.entities.push(plain);
        w
    }

    fn apply(w: &World, edits: Vec<Edit>) -> World {
        let mut w = w.clone();
        let r = Registry::core();
        for e in edits {
            e.apply(&mut w, &r).unwrap();
        }
        w
    }

    #[test]
    fn overrides_are_the_patch_leaves_of_inherited_components() {
        let w = world();
        let a = prefab_overrides(&w, "a");
        assert_eq!(a["core.transform"], ["/position"]);
        assert_eq!(a["x.light"], ["/a~1b", "/color/r", "/parts"]);
        assert!(!a.contains_key("x.own"), "own components are not overrides");
        assert!(prefab_overrides(&w, "c").is_empty());
        assert!(prefab_overrides(&w, "nope").is_empty());
    }

    #[test]
    fn revert_drops_the_field_and_empty_overrides() {
        let w = world();
        let ids = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
        // `a` and `b` override color fields; `c` does not and is left as it is.
        let edits = revert_prefab_field(&w, &ids, "x.light", "/color").unwrap();
        assert_eq!(edits.len(), 2);
        let after = apply(&w, edits);
        assert_eq!(
            after.entity("a").unwrap().components["x.light"],
            json!({"a/b":2,"parts":[3]})
        );
        assert!(
            !after
                .entity("b")
                .unwrap()
                .components
                .contains_key("x.light"),
            "an override left empty is removed"
        );
        // Escaped keys, a leaf inside an object, the last override of a component.
        let after = apply(
            &w,
            revert_prefab_field(&w, &ids[..1], "x.light", "/a~1b").unwrap(),
        );
        assert_eq!(
            after.entity("a").unwrap().components["x.light"],
            json!({"color":{"r":0.5},"parts":[3]})
        );
        let after = apply(
            &w,
            revert_prefab_field(&w, &ids[..1], "core.transform", "/position").unwrap(),
        );
        assert!(!after
            .entity("a")
            .unwrap()
            .components
            .contains_key("core.transform"));
        // Not overridden anywhere / inside an array / not inherited.
        assert_eq!(
            code(revert_prefab_field(&w, &ids[2..], "x.light", "/color").unwrap_err()),
            "not_overridden"
        );
        assert_eq!(
            code(revert_prefab_field(&w, &ids[..1], "x.light", "/parts/0").unwrap_err()),
            "not_overridden"
        );
        assert_eq!(
            code(revert_prefab_field(&w, &ids[..1], "x.own", "/k").unwrap_err()),
            "not_prefab_field"
        );
        assert_eq!(
            code(revert_prefab_field(&w, &[], "x.light", "/color").unwrap_err()),
            "empty_selection"
        );
        assert_eq!(
            code(revert_prefab_field(&w, &ids[..1], "x.light", "color").unwrap_err()),
            "invalid_field"
        );
        // A mixed selection: the object that does not inherit the component is left alone.
        let mixed = ["plain".to_owned(), "a".to_owned()];
        let after = apply(
            &w,
            revert_prefab_field(&w, &mixed, "x.light", "/color").unwrap(),
        );
        assert_eq!(after.entity("plain"), w.entity("plain"));
        assert!(after.entity("a").unwrap().components["x.light"]
            .get("color")
            .is_none());
        // "" = the whole override (like reset).
        let after = apply(
            &w,
            revert_prefab_field(&w, &ids[..1], "x.light", "").unwrap(),
        );
        assert!(!after
            .entity("a")
            .unwrap()
            .components
            .contains_key("x.light"));
    }

    #[test]
    fn apply_moves_the_value_to_the_prefab_and_drops_the_override() {
        let w = world();
        let edits = apply_prefab_field(&w, "a", "x.light", "/color/r").unwrap();
        let after = apply(&w, edits);
        assert_eq!(
            after.prefabs["lamp"].components["x.light"]["color"],
            json!({"r":0.5,"g":1})
        );
        assert_eq!(
            after.entity("a").unwrap().components["x.light"],
            json!({"a/b":2,"parts":[3]})
        );
        // An overridden object applies merged with the prefab's keys it does not override.
        let after = apply(
            &w,
            apply_prefab_field(&w, "b", "x.light", "/color").unwrap(),
        );
        assert_eq!(
            after.prefabs["lamp"].components["x.light"]["color"],
            json!({"r":1,"g":0.2})
        );
        // Arrays apply whole.
        let after = apply(
            &w,
            apply_prefab_field(&w, "a", "x.light", "/parts").unwrap(),
        );
        assert_eq!(
            after.prefabs["lamp"].components["x.light"]["parts"],
            json!([3])
        );
        assert_eq!(
            code(apply_prefab_field(&w, "c", "x.light", "/color").unwrap_err()),
            "not_overridden"
        );
        assert_eq!(
            code(apply_prefab_field(&w, "a", "x.own", "/k").unwrap_err()),
            "not_prefab_field"
        );
    }
}
