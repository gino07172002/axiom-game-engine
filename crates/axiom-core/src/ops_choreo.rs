//! Choreography and pointer-part operations (re-exported from [`crate::ops`]):
//! `animation.choreography` stages (ACL `stage`, editor `anim.stage`) and `animation.parts`
//! bone capsules (ACL `parts`, editor `object.parts`). Pure `fn(&World, ..) ->
//! Result<Vec<Edit>>`; the registry's validators (registered by the runtime's animation
//! capability) check the full shape when the revision commits.
use crate::local::escape;
use crate::*;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const CHOREOGRAPHY: &str = "animation.choreography";
pub const PARTS: &str = "animation.parts";

fn invalid(path: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}

/// One `stage` statement.
#[derive(Clone, Debug, PartialEq)]
pub enum StageEdit {
    /// Creates or replaces the stage with this JSON (`{actors:[..], after?}`).
    Set(Value),
    /// Removes the stage; refused while another stage's `after.next` names it.
    Remove,
}

/// Animation ▸ stage / ACL `stage ID set JSON | stage ID remove`.
pub fn edit_stage(world: &World, id: &str, edit: &StageEdit) -> Result<Vec<Edit>> {
    let path = format!("/resources/{CHOREOGRAPHY}/stages/{}", escape(id));
    valid_id(id).map_err(|e| invalid(&path, format!("{e:#}")))?;
    let mut raw = world
        .resources
        .get(CHOREOGRAPHY)
        .cloned()
        .unwrap_or_else(|| json!({"stages":{}}));
    let stages = raw
        .as_object_mut()
        .ok_or_else(|| invalid(format!("/resources/{CHOREOGRAPHY}"), "must be an object"))?
        .entry("stages")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid(&path, "stages must be an object"))?;
    match edit {
        StageEdit::Set(stage) => {
            let actors = stage.get("actors").and_then(Value::as_array);
            if !stage.is_object() || actors.is_none_or(|a| a.is_empty()) {
                return Err(invalid(
                    &path,
                    "a stage is {\"actors\":[{\"entity\":..,\"clip\"?|\"speed\"?|\"params\"?}..], \"after\"?:{\"entity\",\"next\",\"cycles\"?}}",
                ));
            }
            if stages.get(id) == Some(stage) {
                return Ok(vec![]);
            }
            for a in actors.into_iter().flatten() {
                let entity = a.get("entity").and_then(Value::as_str).unwrap_or_default();
                if world.entity(entity).is_none() {
                    return Err(Diagnostic::error(
                        "not_found",
                        &path,
                        format!("actor entity '{entity}' not found"),
                    ));
                }
            }
            stages.insert(id.to_owned(), stage.clone());
        }
        StageEdit::Remove => {
            if stages.remove(id).is_none() {
                return Err(Diagnostic::error(
                    "not_found",
                    &path,
                    format!("no stage '{id}'"),
                ));
            }
            if let Some((other, _)) = stages
                .iter()
                .find(|(_, s)| s.pointer("/after/next").and_then(Value::as_str) == Some(id))
            {
                return Err(Diagnostic::error(
                    "in_use",
                    &path,
                    format!("stage '{other}' continues with '{id}' (after.next); change it first"),
                ));
            }
        }
    }
    Ok(vec![Edit::Core(CoreEdit::SetResource {
        id: CHOREOGRAPHY.into(),
        value: raw,
    })])
}

/// One `parts` statement on selected `animation.player` entities.
#[derive(Clone, Debug, PartialEq)]
pub enum PartsEdit {
    /// One capsule per bone toward each child joint (part ID = bone name), `radius` meters;
    /// `event` (behavior.program) fires on a click. Keeps authored parts.
    Auto {
        radius: f32,
        event: Option<String>,
    },
    /// Adds or replaces part `id`: a capsule from `bone` to bone `to` (or `length` meters
    /// along the bone's +Y; neither = a sphere at the joint).
    Add {
        id: String,
        bone: String,
        radius: f32,
        to: Option<String>,
        length: Option<f32>,
    },
    Remove {
        id: String,
    },
    /// The event a click emits (`None` = no event; picks still show in observe).
    Event(Option<String>),
    /// Removes the component.
    Clear,
}

/// Object ▸ Pointer Parts / ACL `parts SEL auto RADIUS [event=E] | add ID BONE radius=R
/// [to=BONE] [length=M] | remove ID | event E|none | clear`.
pub fn edit_parts(world: &World, ids: &[String], edit: &PartsEdit) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the animated characters whose parts can be clicked",
        ));
    }
    let mut edits = vec![];
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let path = format!("/entities/@{}/components/{PARTS}", escape(id));
        let entity = world.entity(id).ok_or_else(|| {
            Diagnostic::error("not_found", &path, format!("entity '{id}' not found"))
        })?;
        let resolved = world.resolved_components(entity);
        if !resolved.contains_key("animation.player") {
            return Err(invalid(
                &path,
                format!("'{id}' has no animation.player; parts are capsules on its bones"),
            ));
        }
        let mut value = resolved
            .get(PARTS)
            .cloned()
            .unwrap_or_else(|| json!({"parts":[]}));
        let before = value.clone();
        let object = value
            .as_object_mut()
            .ok_or_else(|| invalid(&path, "must be an object"))?;
        let radius_ok = |r: f32| r.is_finite() && (0.005..=2.0).contains(&r);
        match edit {
            PartsEdit::Clear => {
                if resolved.contains_key(PARTS) {
                    edits.push(Edit::Core(CoreEdit::RemoveComponent {
                        entity: id.clone(),
                        component: PARTS.into(),
                    }));
                }
                continue;
            }
            PartsEdit::Auto { radius, event } => {
                if !radius_ok(*radius) {
                    return Err(invalid(&path, "radius must be 0.005..2 meters"));
                }
                object.insert("auto_radius".into(), json!(radius));
                if let Some(e) = event {
                    valid_id(e).map_err(|err| invalid(&path, format!("{err:#}")))?;
                    object.insert("event".into(), json!(e));
                }
            }
            PartsEdit::Event(event) => match event {
                Some(e) => {
                    valid_id(e).map_err(|err| invalid(&path, format!("{err:#}")))?;
                    object.insert("event".into(), json!(e));
                }
                None => {
                    object.remove("event");
                }
            },
            PartsEdit::Add {
                id: part,
                bone,
                radius,
                to,
                length,
            } => {
                for name in [part, bone].into_iter().chain(to) {
                    valid_id(name).map_err(|err| invalid(&path, format!("{err:#}")))?;
                }
                if !radius_ok(*radius) {
                    return Err(invalid(&path, "radius must be 0.005..2 meters"));
                }
                if to.is_some() && length.is_some() {
                    return Err(invalid(&path, "give to=BONE or length=M, not both"));
                }
                if length.is_some_and(|l| !(l.is_finite() && (0.0..=10.0).contains(&l))) {
                    return Err(invalid(&path, "length must be 0..10 meters"));
                }
                let mut p = json!({"id": part, "bone": bone, "radius": radius});
                if let Some(t) = to {
                    p["to"] = json!(t);
                }
                if let Some(l) = length {
                    p["length"] = json!(l);
                }
                let list = object
                    .entry("parts")
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .ok_or_else(|| invalid(&path, "parts must be a list"))?;
                match list
                    .iter_mut()
                    .find(|q| q.get("id").and_then(Value::as_str) == Some(part.as_str()))
                {
                    Some(q) => *q = p,
                    None => list.push(p),
                }
            }
            PartsEdit::Remove { id: part } => {
                let list = object
                    .get_mut("parts")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| invalid(&path, format!("no part '{part}'")))?;
                let n = list.len();
                list.retain(|q| q.get("id").and_then(Value::as_str) != Some(part.as_str()));
                if list.len() == n {
                    return Err(Diagnostic::error(
                        "not_found",
                        &path,
                        format!("'{id}' has no part '{part}'"),
                    ));
                }
            }
        }
        if value != before || !resolved.contains_key(PARTS) {
            edits.push(Edit::Core(CoreEdit::SetComponent {
                entity: id.clone(),
                component: PARTS.into(),
                value,
            }));
        }
    }
    Ok(edits)
}
