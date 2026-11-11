//! Character rig operations (re-exported from [`crate::ops`]): spring chains and colliders
//! (`animation.spring`), look-at chains (`animation.look_at`) and IK sockets on
//! `animation.player/ik`. One edit applied to every selected entity; JSON-only (the
//! `animation` validators check the bodies, the runtime resolves bones when it builds).
use crate::local::escape;
use crate::*;
use anyhow::Result;
use serde_json::{json, Map, Value};

pub const SPRING_COMPONENT: &str = "animation.spring";
pub const LOOK_AT_COMPONENT: &str = "animation.look_at";

/// Which part of the rig an edit changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RigPart {
    /// `animation.spring/chains/ID`.
    Spring,
    /// `animation.spring/colliders/ID`.
    Collider,
    /// `animation.look_at/chains/ID`.
    Look,
}

impl RigPart {
    fn location(self) -> (&'static str, &'static str) {
        match self {
            RigPart::Spring => (SPRING_COMPONENT, "chains"),
            RigPart::Collider => (SPRING_COMPONENT, "colliders"),
            RigPart::Look => (LOOK_AT_COMPONENT, "chains"),
        }
    }
    /// The field a new item cannot do without.
    fn required(self) -> &'static str {
        match self {
            RigPart::Spring => "root",
            RigPart::Collider => "bone",
            RigPart::Look => "bones",
        }
    }
}

/// One rig edit (ACL `rig SEL ..`, editor `rig.edit ..`).
#[derive(Clone, Debug, PartialEq)]
pub enum RigEdit {
    /// Creates item `id` or merges `fields` into it (`null` removes a field).
    Set {
        part: RigPart,
        id: String,
        fields: Vec<(String, Value)>,
    },
    Remove {
        part: RigPart,
        id: String,
    },
    /// Binds IK constraint `id` to a socket `{entity, bone?, offset?}`; `None` unbinds it.
    IkSocket {
        id: String,
        socket: Option<Value>,
    },
    /// `debug` on the entity's spring and look-at components.
    Debug(bool),
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}

/// Applies `edit` to every entity in `ids` (each needs an `animation.player`). Items are
/// created on first use (the spring / look-at component too); an unchanged result writes
/// nothing. The result is checked with the registered validator before it is returned.
pub fn rig_edit(
    world: &World,
    registry: &Registry,
    ids: &[String],
    edit: &RigEdit,
) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the animated characters to rig",
        ));
    }
    let mut edits = vec![];
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let path = format!("/entities/@{}", escape(id));
        let entity = world.entity(id).ok_or_else(|| {
            Diagnostic::error("not_found", &path, format!("entity '{id}' not found"))
        })?;
        let components = world.resolved_components(entity);
        let Some(player) = components.get("animation.player") else {
            return Err(Diagnostic::error(
                "missing_component",
                format!("{path}/components/animation.player"),
                format!("{id} has no animation.player (rigging needs an animated model)"),
            ));
        };
        let mut changed: Vec<(&str, Value)> = vec![];
        match edit {
            RigEdit::Set {
                part,
                id: item,
                fields,
            } => {
                let (component, map) = part.location();
                let mut value = components
                    .get(component)
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let at = format!("{path}/components/{component}/{map}/{}", escape(item));
                if valid_id(item).is_err() {
                    return Err(Diagnostic::error(
                        "invalid_id",
                        at,
                        format!("invalid ID '{item}' (1..128 of letters, digits, . _ / -)"),
                    ));
                }
                let object = value
                    .as_object_mut()
                    .ok_or_else(|| invalid(&at, format!("{component} must be an object")))?
                    .entry(map)
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| invalid(&at, format!("{map} must be an object")))?;
                let fresh = !object.contains_key(item);
                let entry = object
                    .entry(item.clone())
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| invalid(&at, "must be an object"))?;
                for (k, v) in fields {
                    if v.is_null() {
                        entry.remove(k);
                    } else {
                        entry.insert(k.clone(), v.clone());
                    }
                }
                if fresh && !entry.contains_key(part.required()) {
                    return Err(invalid(
                        at,
                        format!("a new item needs {}=..", part.required()),
                    ));
                }
                changed.push((component, value));
            }
            RigEdit::Remove { part, id: item } => {
                let (component, map) = part.location();
                let at = format!("{path}/components/{component}/{map}/{}", escape(item));
                let mut value = components.get(component).cloned().unwrap_or(Value::Null);
                let removed = value
                    .get_mut(map)
                    .and_then(Value::as_object_mut)
                    .and_then(|m| m.remove(item));
                let Some(_) = removed else {
                    return Err(Diagnostic::error(
                        "not_found",
                        at,
                        format!("{id} has no {component} {map} '{item}'"),
                    ));
                };
                if *part == RigPart::Collider {
                    let users: Vec<String> = value["chains"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(_, c)| {
                            c["colliders"]
                                .as_array()
                                .is_some_and(|l| l.iter().any(|x| x == item.as_str()))
                        })
                        .map(|(k, _)| k.clone())
                        .collect();
                    if !users.is_empty() {
                        return Err(Diagnostic::error(
                            "in_use",
                            at,
                            format!(
                                "collider '{item}' is listed by chain(s) {}; change those first",
                                users.join(", ")
                            ),
                        ));
                    }
                }
                changed.push((component, value));
            }
            RigEdit::IkSocket { id: ik, socket } => {
                let at = format!("{path}/components/animation.player/ik/{}", escape(ik));
                let mut value = player.clone();
                let Some(entry) = value
                    .get_mut("ik")
                    .and_then(|m| m.get_mut(ik.as_str()))
                    .and_then(Value::as_object_mut)
                else {
                    return Err(Diagnostic::error(
                        "not_found",
                        at,
                        format!("{id} has no IK constraint '{ik}' (animation.player/ik)"),
                    ));
                };
                match socket {
                    Some(s) => {
                        let target = s["entity"].as_str().unwrap_or("");
                        if world.entity(target).is_none() {
                            return Err(Diagnostic::error(
                                "not_found",
                                format!("{at}/socket"),
                                format!("socket entity '{target}' not found"),
                            ));
                        }
                        entry.insert("socket".into(), s.clone());
                    }
                    None => {
                        if entry.remove("socket").is_none() {
                            return Err(Diagnostic::error(
                                "not_found",
                                format!("{at}/socket"),
                                format!("IK constraint '{ik}' of {id} has no socket"),
                            ));
                        }
                    }
                }
                changed.push(("animation.player", value));
            }
            RigEdit::Debug(on) => {
                for component in [SPRING_COMPONENT, LOOK_AT_COMPONENT] {
                    if let Some(mut value) = components.get(component).cloned() {
                        if let Some(m) = value.as_object_mut() {
                            m.insert("debug".into(), json!(on));
                        }
                        changed.push((component, value));
                    }
                }
                if changed.is_empty() {
                    return Err(Diagnostic::error(
                        "not_found",
                        format!("{path}/components/{SPRING_COMPONENT}"),
                        format!("{id} has no spring or look-at to debug"),
                    ));
                }
            }
        }
        for (component, value) in changed {
            if components.get(component) == Some(&value) {
                continue;
            }
            let at = format!("{path}/components/{component}");
            if let Some(d) = registry.components.get(component) {
                (d.validate)(&value).map_err(|e| invalid(&at, format!("{e:#}")))?;
            }
            edits.push(Edit::Core(CoreEdit::SetComponent {
                entity: id.clone(),
                component: component.into(),
                value,
            }));
        }
    }
    Ok(edits)
}

/// Parses `key=value` rig options into JSON fields (shared by ACL `rig` and the editor's
/// `rig.edit`): numbers, `x,y,z` vectors, `on|off`, bone lists `B[:W],..`, look targets
/// `camera | point:x,y,z | ENTITY[:BONE]`; `tip=none`, `tail=none` and `colliders=all` remove
/// those fields.
pub fn rig_fields(part: RigPart, options: &[&str]) -> Result<Vec<(String, Value)>> {
    let mut out = vec![];
    for t in options {
        let (k, v) = t
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("option '{t}' must be key=value"))?;
        let number = || -> Result<Value> {
            let x: f64 = v
                .parse()
                .map_err(|_| anyhow::anyhow!("{k} must be a number"))?;
            Ok(json!(x))
        };
        let vector = || -> Result<Value> {
            let xs: Vec<f64> = v
                .split(',')
                .map(|x| x.trim().parse::<f64>())
                .collect::<std::result::Result<_, _>>()
                .map_err(|_| anyhow::anyhow!("{k} must be x,y,z"))?;
            anyhow::ensure!(xs.len() == 3, "{k} must be x,y,z");
            Ok(json!(xs))
        };
        let flag = || -> Result<Value> {
            match v {
                "on" | "true" => Ok(json!(true)),
                "off" | "false" => Ok(json!(false)),
                _ => anyhow::bail!("{k} must be on|off"),
            }
        };
        let value = match (part, k) {
            (RigPart::Spring, "tip") | (RigPart::Collider, "tail") if v == "none" => Value::Null,
            (RigPart::Spring, "colliders") if v == "all" => Value::Null,
            (RigPart::Spring, "root" | "tip") | (RigPart::Collider, "bone") => json!(v),
            (RigPart::Spring, "stiffness" | "drag" | "radius")
            | (RigPart::Collider, "radius")
            | (RigPart::Look, "max_yaw" | "max_pitch" | "weight" | "smoothing") => number()?,
            (RigPart::Spring, "gravity")
            | (RigPart::Collider, "offset" | "tail")
            | (RigPart::Look, "forward" | "up") => vector()?,
            (RigPart::Spring | RigPart::Look, "enabled") => flag()?,
            (RigPart::Spring, "colliders") => {
                json!(v.split(',').filter(|x| !x.is_empty()).collect::<Vec<_>>())
            }
            (RigPart::Look, "bones") => {
                let mut list = vec![];
                for b in v.split(',') {
                    let (bone, weight) = match b.rsplit_once(':') {
                        Some((bone, w)) => (
                            bone,
                            w.parse::<f64>()
                                .map_err(|_| anyhow::anyhow!("bone weight must be a number"))?,
                        ),
                        None => (b, 1.0),
                    };
                    let mut entry = Map::new();
                    entry.insert("bone".into(), json!(bone));
                    entry.insert("weight".into(), json!(weight));
                    list.push(Value::Object(entry));
                }
                Value::Array(list)
            }
            (RigPart::Look, "target") => look_target(v)?,
            _ => anyhow::bail!(
                "unknown option '{k}' (spring: root tip stiffness drag gravity radius colliders enabled; collider: bone offset tail radius; look: bones target forward up max_yaw max_pitch weight smoothing enabled)"
            ),
        };
        out.push((k.to_owned(), value));
    }
    Ok(out)
}

/// `camera`, `point:x,y,z`, `ENTITY` or `ENTITY:BONE` (an `offset=` is a separate key of
/// the socket, so it is given with `ik` / not supported for look targets in text form).
pub fn look_target(v: &str) -> Result<Value> {
    if v == "camera" {
        return Ok(json!({"kind":"camera"}));
    }
    if let Some(p) = v.strip_prefix("point:") {
        let xs: Vec<f64> = p
            .split(',')
            .map(|x| x.trim().parse::<f64>())
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| anyhow::anyhow!("target point must be point:x,y,z"))?;
        anyhow::ensure!(xs.len() == 3, "target point must be point:x,y,z");
        return Ok(json!({"kind":"point","position":xs}));
    }
    let socket = socket_value(v, None)?;
    let mut target = json!({"kind":"entity"});
    for (k, x) in socket.as_object().expect("object") {
        target[k] = x.clone();
    }
    Ok(target)
}

/// `ENTITY[:BONE]` (+ an optional `x,y,z` offset) as a socket object.
pub fn socket_value(v: &str, offset: Option<&str>) -> Result<Value> {
    let (entity, bone) = match v.split_once(':') {
        Some((e, b)) => (e, Some(b)),
        None => (v, None),
    };
    valid_id(entity).map_err(|_| anyhow::anyhow!("invalid socket entity '{entity}'"))?;
    let mut socket = json!({"entity": entity});
    if let Some(b) = bone {
        anyhow::ensure!(!b.is_empty(), "socket bone missing after ':'");
        socket["bone"] = json!(b);
    }
    if let Some(o) = offset {
        let xs: Vec<f64> = o
            .split(',')
            .map(|x| x.trim().parse::<f64>())
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| anyhow::anyhow!("offset must be x,y,z"))?;
        anyhow::ensure!(xs.len() == 3, "offset must be x,y,z");
        socket["offset"] = json!(xs);
    }
    Ok(socket)
}

/// Parses the arguments after `rig SEL` (the editor's `rig.edit` argument):
/// `spring CHAIN [k=v..] | spring CHAIN remove | collider ID [k=v..] | collider ID remove |
/// look CHAIN [k=v..] | look CHAIN remove | ik ID socket=ENTITY[:BONE] [offset=x,y,z] |
/// ik ID socket=none | debug on|off`.
pub fn parse_rig_edit(tokens: &[&str]) -> Result<RigEdit> {
    let part = |k: &str| match k {
        "spring" => Some(RigPart::Spring),
        "collider" => Some(RigPart::Collider),
        "look" => Some(RigPart::Look),
        _ => None,
    };
    Ok(match tokens {
        ["debug", on] => RigEdit::Debug(match *on {
            "on" => true,
            "off" => false,
            _ => anyhow::bail!("use: debug on|off"),
        }),
        [kind, id, "remove"] if part(kind).is_some() => RigEdit::Remove {
            part: part(kind).expect("checked"),
            id: (*id).into(),
        },
        [kind, id, options @ ..] if part(kind).is_some() => {
            let part = part(kind).expect("checked");
            anyhow::ensure!(!options.is_empty(), "give at least one option key=value");
            RigEdit::Set {
                part,
                id: (*id).into(),
                fields: rig_fields(part, options)?,
            }
        }
        ["ik", id, options @ ..] => {
            let mut socket = None;
            let mut offset = None;
            for t in options {
                match t.split_once('=') {
                    Some(("socket", v)) => socket = Some(v),
                    Some(("offset", v)) => offset = Some(v),
                    _ => anyhow::bail!("use: ik ID socket=ENTITY[:BONE]|none [offset=x,y,z]"),
                }
            }
            let socket = socket.ok_or_else(|| {
                anyhow::anyhow!("use: ik ID socket=ENTITY[:BONE]|none [offset=x,y,z]")
            })?;
            RigEdit::IkSocket {
                id: (*id).into(),
                socket: if socket == "none" {
                    anyhow::ensure!(offset.is_none(), "socket=none takes no offset");
                    None
                } else {
                    Some(socket_value(socket, offset)?)
                },
            }
        }
        _ => anyhow::bail!(
            "use: rig SEL spring|collider|look ID key=value.. | spring|collider|look ID remove | ik ID socket=ENTITY[:BONE]|none [offset=x,y,z] | debug on|off"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two animated entities and a plain one (core registry: component bodies are not
    /// validated here, the animation crate's validators run in the runtime parity tests).
    fn world() -> World {
        let mut w = World::empty("t");
        for id in ["a", "b", "prop"] {
            let mut e = Entity::new(id);
            if id != "prop" {
                e.components.insert(
                    "animation.player".into(),
                    json!({"asset":"x","ik":{"hand":{"root":"u","middle":"f","end":"h","target":[0,0,0],"pole":[0,0,1]}}}),
                );
            }
            w.entities.push(e);
        }
        w
    }

    fn apply(w: &World, edits: Vec<Edit>) -> World {
        let mut w = w.clone();
        for e in edits {
            e.apply(&mut w, &Registry::core()).unwrap();
        }
        w
    }

    fn run(w: &World, ids: &[&str], text: &str) -> Result<World> {
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let ids: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
        let edit = parse_rig_edit(&tokens)?;
        Ok(apply(w, rig_edit(w, &Registry::core(), &ids, &edit)?))
    }

    #[test]
    fn spring_chains_and_colliders_are_created_merged_and_removed() {
        let w = run(
            &world(),
            &["a", "b"],
            "spring hair root=h0 stiffness=2 gravity=0,-1,0",
        )
        .unwrap();
        for id in ["a", "b"] {
            assert_eq!(
                w.entity(id).unwrap().components[SPRING_COMPONENT],
                json!({"chains":{"hair":{"root":"h0","stiffness":2.0,"gravity":[0.0,-1.0,0.0]}}})
            );
        }
        let w = run(&w, &["a"], "spring hair drag=0.5 tip=h3 stiffness=1").unwrap();
        let w = run(&w, &["a"], "spring hair tip=none").unwrap();
        assert_eq!(
            w.entity("a").unwrap().components[SPRING_COMPONENT]["chains"]["hair"],
            json!({"root":"h0","stiffness":1.0,"gravity":[0.0,-1.0,0.0],"drag":0.5})
        );
        let w = run(
            &w,
            &["a"],
            "collider skull bone=head radius=0.1 offset=0,0.1,0",
        )
        .unwrap();
        let w = run(&w, &["a"], "spring hair colliders=skull").unwrap();
        let e = format!(
            "{:#}",
            run(&w, &["a"], "collider skull remove").err().unwrap()
        );
        assert!(e.contains("listed by chain(s) hair"), "{e}");
        let w = run(&w, &["a"], "spring hair colliders=all").unwrap();
        let w = run(&w, &["a"], "collider skull remove").unwrap();
        assert_eq!(
            w.entity("a").unwrap().components[SPRING_COMPONENT]["colliders"],
            json!({})
        );
        // Re-setting an unchanged value writes nothing.
        let ids = vec!["a".to_owned()];
        let edit = parse_rig_edit(&["spring", "hair", "drag=0.5"]).unwrap();
        assert!(rig_edit(&w, &Registry::core(), &ids, &edit)
            .unwrap()
            .is_empty());
        let w = run(&w, &["a"], "spring hair remove").unwrap();
        assert_eq!(
            w.entity("a").unwrap().components[SPRING_COMPONENT]["chains"],
            json!({})
        );
    }

    #[test]
    fn look_chains_targets_and_sockets() {
        let w = run(
            &world(),
            &["a"],
            "look gaze bones=neck:0.4,head:0.6 target=b:head max_yaw=45",
        )
        .unwrap();
        assert_eq!(
            w.entity("a").unwrap().components[LOOK_AT_COMPONENT]["chains"]["gaze"],
            json!({"bones":[{"bone":"neck","weight":0.4},{"bone":"head","weight":0.6}],"target":{"kind":"entity","entity":"b","bone":"head"},"max_yaw":45.0})
        );
        let w = run(&w, &["a"], "look gaze target=point:1,2,3").unwrap();
        assert_eq!(
            w.entity("a").unwrap().components[LOOK_AT_COMPONENT]["chains"]["gaze"]["target"],
            json!({"kind":"point","position":[1.0,2.0,3.0]})
        );
        let w = run(&w, &["a"], "ik hand socket=b:hand offset=0,0.1,0").unwrap();
        assert_eq!(
            w.entity("a").unwrap().components["animation.player"]["ik"]["hand"]["socket"],
            json!({"entity":"b","bone":"hand","offset":[0.0,0.1,0.0]})
        );
        let w = run(&w, &["a"], "ik hand socket=none").unwrap();
        assert!(
            w.entity("a").unwrap().components["animation.player"]["ik"]["hand"]
                .get("socket")
                .is_none()
        );
        let w = run(&w, &["a"], "debug on").unwrap();
        assert_eq!(
            w.entity("a").unwrap().components[LOOK_AT_COMPONENT]["debug"],
            true
        );
    }

    #[test]
    fn refusals_name_the_reason() {
        let w = world();
        let err = |ids: &[&str], text: &str| format!("{:#}", run(&w, ids, text).err().unwrap());
        assert!(err(&["prop"], "spring hair root=h").contains("no animation.player"));
        assert!(err(&["a"], "spring hair stiffness=2").contains("needs root="));
        assert!(err(&["a"], "look gaze target=camera").contains("needs bones="));
        assert!(err(&["a"], "spring hair remove").contains("no animation.spring chains 'hair'"));
        assert!(err(&["a"], "ik foot socket=b:head").contains("no IK constraint 'foot'"));
        assert!(
            err(&["a"], "ik hand socket=ghost:head").contains("socket entity 'ghost' not found")
        );
        assert!(err(&["a"], "ik hand socket=none").contains("has no socket"));
        assert!(err(&["a"], "debug on").contains("no spring or look-at"));
        assert!(err(&["a"], "spring hair gravity=1,2").contains("x,y,z"));
        assert!(err(&["a"], "spring hair bogus=1").contains("unknown option 'bogus'"));
        assert!(err(&["a"], "look gaze bones=head:x").contains("weight must be a number"));
        assert!(err(&["a"], "spring bad/../id! root=x").contains("invalid ID"));
        assert!(err(&[], "debug on").contains("select"));
    }
}
