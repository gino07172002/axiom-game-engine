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

/// Pointers under `pointer` where `own` differs from `base` (both inside an array the
/// instance overrides as a whole). Objects compare key by key (an object missing some of the
/// prefab's keys differs itself), arrays holding objects or arrays index by index (a length
/// change marks the array), anything else (numbers, strings, arrays of plain values) as a
/// whole.
fn differences(own: &Value, base: &Value, pointer: &str, out: &mut Vec<String>) {
    match (own, base) {
        (Value::Object(o), Value::Object(b)) => {
            if b.keys().any(|k| !o.contains_key(k)) {
                out.push(pointer.to_owned());
            }
            for (key, value) in o {
                let at = format!("{pointer}/{}", escape(key));
                match b.get(key) {
                    Some(prefab) => differences(value, prefab, &at, out),
                    None => out.push(at),
                }
            }
        }
        (Value::Array(o), Value::Array(b)) if o.iter().any(|v| v.is_object() || v.is_array()) => {
            if o.len() != b.len() {
                out.push(pointer.to_owned());
            }
            items(o, b, pointer, out);
        }
        _ if own != base => out.push(pointer.to_owned()),
        _ => {}
    }
}

/// Items of `own` that differ from `base`'s at the same index (or that `base` lacks).
fn items(own: &[Value], base: &[Value], pointer: &str, out: &mut Vec<String>) {
    for (i, value) in own.iter().enumerate() {
        let at = format!("{pointer}/{i}");
        match base.get(i) {
            Some(prefab) => differences(value, prefab, &at, out),
            None => out.push(at),
        }
    }
}

/// For a prefab instance: per inherited component, the values inside the arrays it overrides
/// that differ from the prefab's array at the same place (JSON pointers relative to the
/// component, escaped), in order. An array overrides as a whole ([`crate::merge_json`] never
/// merges arrays), so every item is the instance's own; these are the ones that read
/// differently from the prefab's, which Revert / Apply to Prefab can copy one at a time.
/// Arrays the prefab does not have are left out (they are overrides as a whole only).
pub fn prefab_array_differences(world: &World, id: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(entity) = world.entity(id) else {
        return out;
    };
    for (component, fields) in prefab_overrides(world, id) {
        let own = &entity.components[&component];
        let base =
            &world.prefabs[entity.prefab.as_deref().expect("an instance")].components[&component];
        let mut found = vec![];
        for field in fields {
            if let (Some(Value::Array(o)), Some(Value::Array(b))) =
                (own.pointer(&field), base.pointer(&field))
            {
                items(o, b, &field, &mut found);
            }
        }
        if !found.is_empty() {
            out.insert(component, found);
        }
    }
    out
}

/// When `path` goes inside an array the instance overrides as a whole and the prefab has that
/// array too: the number of segments naming the array.
fn array_override_depth(own: &Value, base: &Value, path: &[String]) -> Option<usize> {
    let (mut own, mut base) = (own, base);
    for (depth, key) in path.iter().enumerate() {
        match (own, base) {
            (Value::Object(o), Value::Object(b)) => {
                own = o.get(key)?;
                base = b.get(key)?;
            }
            (Value::Array(_), Value::Array(_)) => return Some(depth),
            _ => return None,
        }
    }
    None
}

fn pointer_of(path: &[String]) -> String {
    path.iter().map(|s| format!("/{}", escape(s))).collect()
}

/// The edit that leaves `id`'s override of `component` as `value` (removed when empty).
fn override_edit(entity: &Entity, component: &str, value: Value) -> Edit {
    Edit::Core(if value.as_object().is_some_and(Map::is_empty) {
        CoreEdit::RemoveComponent {
            entity: entity.id.clone(),
            component: component.to_owned(),
        }
    } else {
        CoreEdit::SetComponent {
            entity: entity.id.clone(),
            component: component.to_owned(),
            value,
        }
    })
}

/// Why a value inside an overridden array has no Revert / Apply.
enum Element {
    /// The instance's value already equals the prefab's.
    Same,
    /// The instance's array has nothing at this place (it is shorter than the prefab's).
    NotInInstance,
    /// The prefab's array has nothing at this place: only the whole array reverts / applies.
    NoPrefabValue,
}

/// `own` with the value at `pointer` (inside the array the first `depth` segments of `path`
/// name) replaced by `with` (`None`: the key is removed from its object), and that array's
/// override dropped when it then equals `base`'s.
fn element_override(
    own: &Value,
    base: &Value,
    pointer: &str,
    path: &[String],
    depth: usize,
    with: Option<&Value>,
) -> Value {
    let mut next = own.clone();
    match with {
        Some(with) => *next.pointer_mut(pointer).expect("checked by the caller") = with.clone(),
        None => {
            let (key, parent) = path.split_last().expect("inside an array");
            next.pointer_mut(&pointer_of(parent))
                .and_then(Value::as_object_mut)
                .expect("checked by the caller")
                .remove(key);
        }
    }
    let array = pointer_of(&path[..depth]);
    if next.pointer(&array) == base.pointer(&array) {
        if depth == 0 {
            // The component value is the array itself: the whole override goes.
            return Value::Object(Map::new());
        }
        remove_at(&mut next, &path[..depth]);
    }
    next
}

/// The object holding the last segment of `path` in `value`, when the key is an object key
/// (not an array index): where a key the other side lacks can be added or removed.
fn parent_object<'a>(value: &'a Value, path: &[String]) -> Option<&'a Map<String, Value>> {
    let (_, parent) = path.split_last()?;
    value.pointer(&pointer_of(parent))?.as_object()
}

/// Revert of one value inside an array the instance overrides as a whole: the instance's
/// value becomes the prefab's at the same place, or a key the prefab's item lacks is removed
/// (the array stays the instance's own unless it then equals the prefab's, when its override
/// is dropped).
fn element_revert(
    entity: &Entity,
    component: &str,
    base: &Value,
    pointer: &str,
    path: &[String],
    depth: usize,
) -> std::result::Result<Edit, Element> {
    let own = &entity.components[component];
    let current = own.pointer(pointer).ok_or(Element::NotInInstance)?;
    let prefab = match base.pointer(pointer) {
        Some(prefab) if prefab == current => return Err(Element::Same),
        Some(prefab) => Some(prefab),
        // A key of an item the prefab's item lacks: removing it is the revert.
        None if path.len() > depth + 1
            && parent_object(base, path).is_some()
            && parent_object(own, path).is_some() =>
        {
            None
        }
        None => return Err(Element::NoPrefabValue),
    };
    let next = element_override(own, base, pointer, path, depth, prefab);
    Ok(override_edit(entity, component, next))
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
    if path.is_empty() {
        return Some(override_edit(entity, component, Value::Object(Map::new())));
    }
    let mut rest = own.clone();
    if !remove_at(&mut rest, path) {
        return None;
    }
    Some(override_edit(entity, component, rest))
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
///
/// A pointer inside an array the instance overrides (arrays override as a whole, see
/// [`prefab_array_differences`]) reverts that one value: it becomes the prefab's value at the
/// same place (a key the prefab's item lacks is removed), and the array override is dropped
/// once the whole array equals the prefab's. `no_prefab_value` when the prefab's array has no
/// item there (revert the whole array).
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
    let mut lacking = None;
    let mut missing = false;
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let entity = world.entity(id).ok_or_else(|| {
            Diagnostic::error("not_found", entity_path(id), "entity ID not found")
        })?;
        let Some((_, prefab)) = inherited(world, entity, component) else {
            continue;
        };
        instances += 1;
        let base = &prefab.components[component];
        let inside = entity
            .components
            .get(component)
            .and_then(|own| array_override_depth(own, base, &path));
        match inside {
            Some(depth) => match element_revert(entity, component, base, pointer, &path, depth) {
                Ok(edit) => edits.push(edit),
                Err(Element::NoPrefabValue) => lacking = Some(depth),
                Err(Element::NotInInstance) => missing = true,
                Err(Element::Same) => {}
            },
            None => edits.extend(revert_edit(entity, component, &path)),
        }
    }
    if let Some(depth) = lacking.filter(|_| edits.is_empty()) {
        return Err(no_prefab_value(
            component,
            pointer,
            &pointer_of(&path[..depth]),
        ));
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
            if missing {
                format!(
                    "{component}{pointer} is not in the instance's array (shorter than the prefab's): revert the whole array to get the prefab's items back"
                )
            } else {
                format!(
                    "{component}{pointer} is not overridden (a field inside an overridden value reverts with it): it already shows the prefab's value"
                )
            },
        ));
    }
    Ok(edits)
}

/// Inspector ▸ field ▸ Apply to Prefab / ACL `prefab apply ID COMPONENT/FIELD`: the
/// instance's value of the overridden field becomes the prefab's (every instance that does
/// not override it changes too) and the instance stops overriding it, in one revision.
/// A pointer inside an array the instance overrides sets that one value in the prefab's
/// array (a key its item lacks is added; `no_prefab_value` when it has no item there); the
/// instance keeps its array until it equals the prefab's.
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
    let base = &prefab.components[component];
    if let Some(depth) = entity
        .components
        .get(component)
        .and_then(|own| array_override_depth(own, base, &path))
    {
        return apply_element(entity, prefab_id, prefab, component, pointer, &path, depth);
    }
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

/// Revert / Apply of a value inside an overridden array whose place the prefab's array lacks.
fn no_prefab_value(component: &str, pointer: &str, array: &str) -> anyhow::Error {
    Diagnostic::error(
        "no_prefab_value",
        format!("/components/{}{pointer}", escape(component)),
        format!(
            "the prefab's {component}{array} has no value at {pointer}: arrays override as a whole, so revert or apply {component}{array} instead"
        ),
    )
}

/// Apply to Prefab of one value inside an array the instance overrides as a whole: the
/// prefab's array takes the instance's value at the same place; when the instance's array
/// then equals the prefab's, its override is dropped in the same revision.
fn apply_element(
    entity: &Entity,
    prefab_id: &str,
    prefab: &Prefab,
    component: &str,
    pointer: &str,
    path: &[String],
    depth: usize,
) -> Result<Vec<Edit>> {
    let own = &entity.components[component];
    let not_overridden = |why: &str| {
        Diagnostic::error(
            "not_overridden",
            format!(
                "{}/components/{}{pointer}",
                entity_path(&entity.id),
                escape(component)
            ),
            format!("{component}{pointer} {why} on '{}'", entity.id),
        )
    };
    let Some(value) = own.pointer(pointer) else {
        return Err(not_overridden("is not in the instance's array"));
    };
    let mut updated = prefab.clone();
    let base = updated
        .components
        .get_mut(component)
        .expect("inherited by the caller");
    match base.pointer_mut(pointer) {
        Some(slot) if slot == value => {
            return Err(not_overridden("already equals the prefab's value"));
        }
        Some(slot) => *slot = value.clone(),
        // A key the prefab's item lacks: the prefab's item gets it.
        None if path.len() > depth + 1 => {
            let (key, parent) = path.split_last().expect("inside an array");
            let Some(item) = base
                .pointer_mut(&pointer_of(parent))
                .and_then(Value::as_object_mut)
            else {
                return Err(no_prefab_value(
                    component,
                    pointer,
                    &pointer_of(&path[..depth]),
                ));
            };
            item.insert(key.clone(), value.clone());
        }
        None => {
            return Err(no_prefab_value(
                component,
                pointer,
                &pointer_of(&path[..depth]),
            ));
        }
    }
    let mut edits = vec![];
    let array = pointer_of(&path[..depth]);
    let drop_array = own.pointer(&array) == base.pointer(&array);
    edits.push(Edit::Core(CoreEdit::PutPrefab {
        id: prefab_id.to_owned(),
        prefab: updated,
    }));
    if drop_array {
        edits.extend(revert_edit(entity, component, &path[..depth]));
    }
    Ok(edits)
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
        // Not overridden anywhere / not inherited.
        assert_eq!(
            code(revert_prefab_field(&w, &ids[2..], "x.light", "/color").unwrap_err()),
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

    /// A prefab with an array of parts and instances overriding that array (as a whole).
    fn mesh_world() -> World {
        let mut w = World::empty("t");
        let mut prefab = Prefab {
            components: Default::default(),
        };
        prefab.components.insert(
            "x.mesh".into(),
            json!({"parts":[{"color":[0.1,0.1,0.1],"emission":1},{"color":[1,0.8,0.6],"size":2}],"name":"lamp"}),
        );
        w.prefabs.insert("lamp".into(), prefab);
        let same = json!({"color":[0.1,0.1,0.1],"emission":1});
        let blue = json!({"color":[0,0,1],"size":2});
        for (id, own) in [
            // parts[1].color differs, parts[2] is new.
            (
                "m",
                json!({"x.mesh":{"parts":[same, blue, {"color":[1,1,1]}],"name":"m"}}),
            ),
            // Only parts[1].color differs.
            ("n", json!({"x.mesh":{"parts":[same, blue]}})),
            // parts[0] lacks the prefab's emission.
            (
                "o",
                json!({"x.mesh":{"parts":[{"color":[0.1,0.1,0.1]},{"color":[1,0.8,0.6],"size":2}]}}),
            ),
            ("c", json!({})),
        ] {
            let mut e = Entity::new(id);
            e.prefab = Some("lamp".into());
            e.components = serde_json::from_value(own).unwrap();
            w.entities.push(e);
        }
        w
    }

    #[test]
    fn array_differences_name_the_values_that_read_differently() {
        let w = mesh_world();
        assert_eq!(prefab_overrides(&w, "m")["x.mesh"], ["/name", "/parts"]);
        assert_eq!(
            prefab_array_differences(&w, "m")["x.mesh"],
            ["/parts/1/color", "/parts/2"]
        );
        assert_eq!(
            prefab_array_differences(&w, "n")["x.mesh"],
            ["/parts/1/color"]
        );
        assert_eq!(prefab_array_differences(&w, "o")["x.mesh"], ["/parts/0"]);
        assert!(prefab_array_differences(&w, "c").is_empty());
        assert!(prefab_array_differences(&w, "nope").is_empty());
        // Top-level arrays of plain values compare index by index.
        let w = world();
        assert_eq!(prefab_array_differences(&w, "a")["x.light"], ["/parts/0"]);
        assert_eq!(
            prefab_array_differences(&w, "a")["core.transform"],
            ["/position/0", "/position/2"]
        );
        assert!(!prefab_array_differences(&w, "b").contains_key("x.light"));
    }

    #[test]
    fn revert_inside_an_overridden_array_copies_the_prefab_value() {
        let w = mesh_world();
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // The array stays the instance's own while it still differs.
        let after = apply(
            &w,
            revert_prefab_field(&w, &ids(&["m"]), "x.mesh", "/parts/1/color").unwrap(),
        );
        assert_eq!(
            after.entity("m").unwrap().components["x.mesh"]["parts"][1],
            json!({"color":[1,0.8,0.6],"size":2})
        );
        assert_eq!(
            after.entity("m").unwrap().components["x.mesh"]["parts"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        // Equal to the prefab's array again: the override goes (here the whole component).
        let after = apply(
            &w,
            revert_prefab_field(&w, &ids(&["n"]), "x.mesh", "/parts/1/color").unwrap(),
        );
        assert!(!after.entity("n").unwrap().components.contains_key("x.mesh"));
        // A multi-selection reverts every instance whose value differs.
        let edits =
            revert_prefab_field(&w, &ids(&["m", "n", "o", "c"]), "x.mesh", "/parts/1/color")
                .unwrap();
        assert_eq!(edits.len(), 2);
        // A whole item, and an item lacking one of the prefab's keys.
        let after = apply(
            &w,
            revert_prefab_field(&w, &ids(&["o"]), "x.mesh", "/parts/0").unwrap(),
        );
        assert!(!after.entity("o").unwrap().components.contains_key("x.mesh"));
        // Equal already / the prefab has no item there / not in the array.
        assert_eq!(
            code(revert_prefab_field(&w, &ids(&["m"]), "x.mesh", "/parts/0").unwrap_err()),
            "not_overridden"
        );
        assert_eq!(
            code(revert_prefab_field(&w, &ids(&["m"]), "x.mesh", "/parts/2").unwrap_err()),
            "no_prefab_value"
        );
        assert_eq!(
            code(revert_prefab_field(&w, &ids(&["n"]), "x.mesh", "/parts/7").unwrap_err()),
            "not_overridden"
        );
        // The whole array still reverts as before.
        let after = apply(
            &w,
            revert_prefab_field(&w, &ids(&["m"]), "x.mesh", "/parts").unwrap(),
        );
        assert_eq!(
            after.entity("m").unwrap().components["x.mesh"],
            json!({"name":"m"})
        );
    }

    #[test]
    fn apply_inside_an_overridden_array_sets_the_prefab_value() {
        let w = mesh_world();
        // The prefab takes the value; `m` keeps its (still different) array.
        let edits = apply_prefab_field(&w, "m", "x.mesh", "/parts/1/color").unwrap();
        assert_eq!(edits.len(), 1);
        let after = apply(&w, edits);
        assert_eq!(
            after.prefabs["lamp"].components["x.mesh"]["parts"][1],
            json!({"color":[0,0,1],"size":2})
        );
        assert_eq!(after.entity("m"), w.entity("m"));
        // `n`'s array now equals the prefab's: its override goes in the same revision, and
        // instances that do not override the array (`c`) follow the prefab.
        let edits = apply_prefab_field(&w, "n", "x.mesh", "/parts/1/color").unwrap();
        assert_eq!(edits.len(), 2);
        let after = apply(&w, edits);
        assert!(!after.entity("n").unwrap().components.contains_key("x.mesh"));
        assert_eq!(
            after.prefabs["lamp"].components["x.mesh"]["parts"][1]["color"],
            json!([0, 0, 1])
        );
        assert_eq!(after.entity("o"), w.entity("o"));
        assert_eq!(
            code(apply_prefab_field(&w, "m", "x.mesh", "/parts/2").unwrap_err()),
            "no_prefab_value"
        );
        assert_eq!(
            code(apply_prefab_field(&w, "n", "x.mesh", "/parts/0/color").unwrap_err()),
            "not_overridden"
        );
        assert_eq!(
            code(apply_prefab_field(&w, "n", "x.mesh", "/parts/9").unwrap_err()),
            "not_overridden"
        );
    }

    #[test]
    fn keys_an_item_lacks_revert_and_apply_and_a_shorter_array_says_so() {
        let mut w = mesh_world();
        for (id, parts) in [
            // parts[0] has a key the prefab's item lacks.
            (
                "x",
                json!([{"color":[0.1,0.1,0.1],"emission":1,"glow":2},{"color":[1,0.8,0.6],"size":2}]),
            ),
            // One item only: the prefab's second item is hidden.
            ("s", json!([{"color":[0.1,0.1,0.1],"emission":1}])),
        ] {
            let mut e = Entity::new(id);
            e.prefab = Some("lamp".into());
            e.components
                .insert("x.mesh".into(), json!({ "parts": parts }));
            w.entities.push(e);
        }
        assert_eq!(
            prefab_array_differences(&w, "x")["x.mesh"],
            ["/parts/0/glow"]
        );
        let x = ["x".to_owned()];
        // Revert removes the key; the array then equals the prefab's, so the override goes.
        let after = apply(
            &w,
            revert_prefab_field(&w, &x, "x.mesh", "/parts/0/glow").unwrap(),
        );
        assert!(!after.entity("x").unwrap().components.contains_key("x.mesh"));
        // Apply adds the key to the prefab's item, in one revision with dropping the override.
        let edits = apply_prefab_field(&w, "x", "x.mesh", "/parts/0/glow").unwrap();
        assert_eq!(edits.len(), 2);
        let after = apply(&w, edits);
        assert_eq!(
            after.prefabs["lamp"].components["x.mesh"]["parts"][0]["glow"],
            json!(2)
        );
        assert!(!after.entity("x").unwrap().components.contains_key("x.mesh"));
        // A place the shorter instance array lacks is reported as such.
        let e = revert_prefab_field(&w, &["s".to_owned()], "x.mesh", "/parts/1").unwrap_err();
        let d = e.downcast_ref::<Diagnostic>().unwrap();
        assert_eq!(d.code, "not_overridden");
        assert!(d.message.contains("shorter"), "{}", d.message);
    }
}
