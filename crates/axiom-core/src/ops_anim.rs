//! Animation controller operations (re-exported from [`crate::ops`]): create a controller in
//! `animation.controllers`, edit its parameters, and attach it to entities with an
//! `animation.controller` component. States, transitions and graph positions use the shared
//! graph operations with [`GraphDoc::Animator`] (`graph_edit`, `graph_set_node`,
//! `graph_remove`). Pure like the rest of `ops`; the `animation` validator checks bodies at
//! commit (clip names are resolved by the runtime against each entity's player).
use crate::local::escape;
use crate::ops_graph::{GraphDoc, ANIM_RESOURCE};
use crate::ops_world::add_component;
use crate::*;
use anyhow::Result;
use serde_json::{json, Value};

/// Budget of `animation.controllers` (the validator's), checked here for clearer messages.
pub const MAX_ANIM_PARAMS: usize = 16;

fn controller_path(id: &str) -> String {
    format!("/resources/{ANIM_RESOURCE}/controllers/{}", escape(id))
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}

/// Graphs ▸ Animator ▸ New controller / ACL `animctl C create [INITIAL]`: a controller with
/// one state (default `idle`) holding the rest pose, creating `animation.controllers` when
/// the world has none. An existing ID is refused.
pub fn anim_create(
    world: &World,
    registry: &Registry,
    id: &str,
    initial: Option<&str>,
) -> Result<Vec<Edit>> {
    let initial = initial.unwrap_or("idle");
    crate::ops_graph::insert_graph(
        world,
        registry,
        GraphDoc::Animator,
        id,
        json!({"initial": initial, "states": {initial: {}}, "transitions": []}),
    )
}

/// The controller JSON (mutable copy of the resource) and the controller object inside it.
fn controller_doc(world: &World, id: &str) -> Result<Value> {
    let doc = world
        .resources
        .get(ANIM_RESOURCE)
        .cloned()
        .unwrap_or_else(|| json!({"controllers":{}}));
    if doc["controllers"].get(id).is_none_or(|c| !c.is_object()) {
        return Err(Diagnostic::error(
            "not_found",
            controller_path(id),
            format!("no animation controller '{id}'"),
        ));
    }
    Ok(doc)
}

/// Whether controller `c` refers to parameter `name` (conditions or blend trees).
fn param_users(c: &Value, name: &str) -> Vec<String> {
    let mut out = vec![];
    if let Some(states) = c["states"].as_object() {
        for (s, def) in states {
            if def["motion"]["parameter"] == name {
                out.push(format!("state {s}"));
            }
        }
    }
    if let Some(list) = c["transitions"].as_array() {
        for (i, t) in list.iter().enumerate() {
            let uses = t["conditions"]
                .as_array()
                .is_some_and(|cs| cs.iter().any(|x| x["param"] == name));
            if uses {
                out.push(format!("transition {i}"));
            }
        }
    }
    if let Some(list) = c["layers"].as_array() {
        for l in list {
            if l["weight_parameter"] == name {
                out.push(format!("layer {}", l["name"].as_str().unwrap_or("?")));
            }
        }
    }
    out
}

/// Most layers one controller may have (the validator's budget).
pub const MAX_ANIM_LAYERS: usize = 4;

/// Graphs ▸ Animator ▸ Layers / ACL `animctl C layer set NAME JSON` and
/// `animctl C layer remove NAME`: replaces layer `name` in place or appends it with `def`
/// (`{"clip":ID,"mode"?:"override|additive","weight"?,"weight_parameter"?,"mask"?:{"bones"?,
/// "subtrees"?},"speed"?,"looping"?}`; its `name` is set to `name`), or removes it (`None`).
/// Unchanged = no edit. The clip and mask bones are resolved by the runtime.
pub fn anim_layer(world: &World, id: &str, name: &str, def: Option<&Value>) -> Result<Vec<Edit>> {
    let mut doc = controller_doc(world, id)?;
    let path = format!("{}/layers", controller_path(id));
    if valid_id(name).is_err() {
        return Err(Diagnostic::error(
            "invalid_id",
            path,
            format!("invalid layer name '{name}' (1..128 of letters, digits, . _ / -)"),
        ));
    }
    let c = doc["controllers"]
        .get_mut(id)
        .and_then(Value::as_object_mut)
        .expect("checked");
    let layers = c
        .entry("layers")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| invalid(&path, "layers must be an array"))?;
    let at = layers.iter().position(|l| l["name"] == name);
    match def {
        Some(def) => {
            let Some(body) = def.as_object() else {
                return Err(invalid(path, "a layer is {\"clip\":ID,\"mode\"?,\"weight\"?,\"weight_parameter\"?,\"mask\"?,\"speed\"?,\"looping\"?}"));
            };
            if body.get("clip").and_then(Value::as_str).is_none() {
                return Err(invalid(path, "a layer needs \"clip\":ID"));
            }
            let mut layer = serde_json::Map::new();
            layer.insert("name".into(), json!(name));
            for (k, v) in body {
                if k != "name" {
                    layer.insert(k.clone(), v.clone());
                }
            }
            let layer = Value::Object(layer);
            match at {
                Some(i) if layers[i] == layer => return Ok(vec![]),
                Some(i) => layers[i] = layer,
                None if layers.len() >= MAX_ANIM_LAYERS => {
                    return Err(invalid(
                        path,
                        format!("a controller has at most {MAX_ANIM_LAYERS} layers"),
                    ))
                }
                None => layers.push(layer),
            }
        }
        None => {
            let Some(i) = at else {
                return Err(Diagnostic::error(
                    "not_found",
                    path,
                    format!("controller '{id}' has no layer '{name}'"),
                ));
            };
            layers.remove(i);
            if layers.is_empty() {
                c.remove("layers");
            }
        }
    }
    Ok(vec![Edit::Core(CoreEdit::SetResource {
        id: ANIM_RESOURCE.into(),
        value: doc,
    })])
}

/// Inspector ▸ animation.player ▸ Parts / `object.mesh_part` / ACL `meshpart SEL hide|show G`:
/// adds `group` to (hide) or removes it from (show) `animation.player.hidden` on each
/// selected entity's own `animation.player` (a primitive's material ID or a glTF mesh,
/// material or node name; resolved against the asset by the runtime). Unchanged = no edit.
pub fn mesh_part(world: &World, ids: &[String], group: &str, visible: bool) -> Result<Vec<Edit>> {
    if ids.is_empty() {
        return Err(Diagnostic::error(
            "empty_selection",
            "/ids",
            "select the animated models whose parts to show or hide",
        ));
    }
    if group.is_empty() || group.len() > 256 {
        return Err(invalid("/group", "a part name is 1..256 characters"));
    }
    let mut edits = vec![];
    for id in ids {
        let path = format!("/entities/@{}/components/animation.player", escape(id));
        let Some(entity) = world.entities.iter().find(|e| &e.id == id) else {
            return Err(Diagnostic::error(
                "not_found",
                format!("/entities/@{}", escape(id)),
                format!("no entity '{id}'"),
            ));
        };
        let Some(mut player) = entity.components.get("animation.player").cloned() else {
            return Err(Diagnostic::error(
                "not_found",
                path,
                format!("'{id}' has no animation.player of its own (edit its prefab instead)"),
            ));
        };
        let Some(map) = player.as_object_mut() else {
            return Err(invalid(path, "animation.player must be an object"));
        };
        let mut hidden: Vec<Value> = map
            .get("hidden")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let listed = hidden.iter().any(|g| g == group);
        if listed != visible {
            continue;
        }
        if visible {
            hidden.retain(|g| g != group);
        } else {
            if hidden.len() >= MAX_HIDDEN_PARTS {
                return Err(invalid(
                    format!("{path}/hidden"),
                    format!("at most {MAX_HIDDEN_PARTS} hidden parts"),
                ));
            }
            hidden.push(json!(group));
        }
        if hidden.is_empty() {
            map.remove("hidden");
        } else {
            map.insert("hidden".into(), Value::Array(hidden));
        }
        edits.push(Edit::Core(CoreEdit::SetComponent {
            entity: id.clone(),
            component: "animation.player".into(),
            value: player,
        }));
    }
    Ok(edits)
}

/// Most entries `animation.player.hidden` may list (the validator's budget).
pub const MAX_HIDDEN_PARTS: usize = 64;

/// Graphs ▸ Animator ▸ Parameters / ACL `animctl C param set NAME JSON` and
/// `animctl C param remove NAME`: adds or replaces parameter `name` with `def`
/// (`{"kind":"float|bool|trigger","default"?,"source"?:"speed"}`), or removes it (`None`;
/// refused while a condition or blend tree uses it). Unchanged = no edit.
pub fn anim_param(world: &World, id: &str, name: &str, def: Option<&Value>) -> Result<Vec<Edit>> {
    let mut doc = controller_doc(world, id)?;
    let path = format!("{}/parameters/{}", controller_path(id), escape(name));
    if valid_id(name).is_err() {
        return Err(Diagnostic::error(
            "invalid_id",
            path,
            format!("invalid parameter name '{name}' (1..128 of letters, digits, . _ / -)"),
        ));
    }
    let c = doc["controllers"]
        .get_mut(id)
        .and_then(Value::as_object_mut)
        .expect("checked");
    let params = c
        .entry("parameters")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid(&path, "parameters must be an object"))?;
    match def {
        Some(def) => {
            if !def.is_object() || def.get("kind").and_then(Value::as_str).is_none() {
                return Err(invalid(
                    path,
                    "a parameter is {\"kind\":\"float|bool|trigger\",\"default\"?:V,\"source\"?:\"speed\"}",
                ));
            }
            if params.get(name) == Some(def) {
                return Ok(vec![]);
            }
            if !params.contains_key(name) && params.len() >= MAX_ANIM_PARAMS {
                return Err(invalid(
                    path,
                    format!("a controller has at most {MAX_ANIM_PARAMS} parameters"),
                ));
            }
            params.insert(name.to_owned(), def.clone());
        }
        None => {
            if params.remove(name).is_none() {
                return Err(Diagnostic::error(
                    "not_found",
                    path,
                    format!("controller '{id}' has no parameter '{name}'"),
                ));
            }
            let users = param_users(&Value::Object(c.clone()), name);
            if !users.is_empty() {
                return Err(Diagnostic::error(
                    "param_in_use",
                    path,
                    format!(
                        "parameter '{name}' is used by {}; change those first",
                        users.join(", ")
                    ),
                ));
            }
        }
    }
    Ok(vec![Edit::Core(CoreEdit::SetResource {
        id: ANIM_RESOURCE.into(),
        value: doc,
    })])
}

/// Inspector ▸ Animation ▸ Controller / ACL `animctl SEL attach C`: an `animation.controller`
/// running controller `id` on each selected entity (they need an `animation.player`; the
/// validator checks at commit). Entities that already run it are left alone.
pub fn anim_attach(
    world: &World,
    registry: &Registry,
    ids: &[String],
    id: &str,
) -> Result<Vec<Edit>> {
    controller_doc(world, id)?;
    let value = json!({"controller": id});
    let mut add = vec![];
    let mut edits = vec![];
    for e in ids {
        let Some(entity) = world.entities.iter().find(|x| &x.id == e) else {
            return Err(Diagnostic::error(
                "not_found",
                format!("/entities/@{}", escape(e)),
                format!("no entity '{e}'"),
            ));
        };
        match entity.components.get("animation.controller") {
            Some(v) if *v == value => {}
            Some(_) => edits.push(Edit::set(
                format!("/entities/@{}/components/animation.controller", escape(e)),
                value.clone(),
            )),
            None => add.push(e.clone()),
        }
    }
    if !add.is_empty() {
        edits.extend(add_component(
            world,
            registry,
            &add,
            "animation.controller",
            Some(value),
        )?);
    }
    Ok(edits)
}

/// Problems the Graphs Animator view shows: dangling transition ends, states the initial
/// state cannot reach, and `animation.controller`s naming a missing controller.
pub fn anim_problems(world: &World) -> Vec<crate::ops_graph::GraphProblem> {
    let mut out = vec![];
    let controllers = world
        .resources
        .get(ANIM_RESOURCE)
        .and_then(|r| r["controllers"].as_object());
    for e in world.entities.iter().take(65_536) {
        if let Some(c) = e.components.get("animation.controller") {
            let name = c["controller"].as_str().unwrap_or("");
            if controllers.is_none_or(|m| !m.contains_key(name)) {
                out.push(crate::ops_graph::GraphProblem {
                    code: "anim_missing_controller",
                    machine: name.to_owned(),
                    state: None,
                    transition: None,
                    path: format!(
                        "/entities/@{}/components/animation.controller",
                        escape(&e.id)
                    ),
                    message: format!("{}: animation controller '{name}' does not exist", e.id),
                });
            }
        }
    }
    let Some(controllers) = controllers else {
        return out;
    };
    for (id, c) in controllers.iter().take(64) {
        crate::ops_graph::graph_shape_problems(GraphDoc::Animator, id, c, &mut out);
    }
    out
}

/// One edit of a clip's markers in `animation.library.markers` (ACL `marker CLIP ..`).
#[derive(Clone, Debug, PartialEq)]
pub enum MarkerEdit {
    /// New marker at `at` seconds emitting `event` and / or playing audio `cue`.
    Add {
        at: f32,
        event: Option<String>,
        cue: Option<String>,
    },
    Remove(usize),
    /// Moves marker I to a new time (the list stays sorted by time).
    Move(usize, f32),
}

/// Most markers one clip may have (the animation validator's budget).
pub const MAX_CLIP_MARKERS: usize = 64;
const LIBRARY: &str = "animation.library";

/// A time as its shortest decimal text (0.1, not the widened 0.10000000149).
fn short(x: f32) -> Value {
    json!(x.to_string().parse::<f64>().unwrap_or(0.0))
}

/// Animation tab ▸ markers / `anim.marker` / ACL `marker CLIP add|remove|move ..`: edits
/// the markers of `clip` (kept sorted by time; an emptied clip drops its entry). Event and
/// cue names are resolved by the runtime against `behavior.program` and `audio.library`.
pub fn anim_marker(world: &World, clip: &str, edit: &MarkerEdit) -> Result<Vec<Edit>> {
    let base = format!("/resources/{LIBRARY}/markers/{}", escape(clip));
    if valid_id(clip).is_err() {
        return Err(Diagnostic::error(
            "invalid_id",
            base,
            format!("invalid clip ID '{clip}'"),
        ));
    }
    let Some(mut lib) = world.resources.get(LIBRARY).cloned() else {
        return Err(Diagnostic::error(
            "not_found",
            format!("/resources/{LIBRARY}"),
            "the world has no animation.library",
        ));
    };
    let lib_map = lib
        .as_object_mut()
        .ok_or_else(|| invalid(format!("/resources/{LIBRARY}"), "must be an object"))?;
    let markers = lib_map
        .entry("markers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid(&base, "markers must be an object"))?;
    let mut list: Vec<Value> = markers
        .get(clip)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let at_of = |m: &Value| m["at"].as_f64().unwrap_or(0.0);
    let check_time = |at: f32| -> Result<()> {
        if !(at.is_finite() && (0.0..=86400.0).contains(&at)) {
            return Err(invalid(&base, "marker time must be 0..86400 seconds"));
        }
        Ok(())
    };
    let check_index = |i: usize, len: usize| -> Result<()> {
        if i >= len {
            return Err(Diagnostic::error(
                "index_out_of_range",
                &base,
                format!("marker {i} is out of range: clip {clip} has {len} marker(s)"),
            ));
        }
        Ok(())
    };
    match edit {
        MarkerEdit::Add { at, event, cue } => {
            check_time(*at)?;
            if event.is_none() && cue.is_none() {
                return Err(invalid(&base, "a marker needs event=E and / or cue=C"));
            }
            for id in event.iter().chain(cue) {
                if valid_id(id).is_err() {
                    return Err(Diagnostic::error(
                        "invalid_id",
                        &base,
                        format!("invalid event / cue '{id}'"),
                    ));
                }
            }
            if list.len() >= MAX_CLIP_MARKERS {
                return Err(invalid(
                    &base,
                    format!("a clip has at most {MAX_CLIP_MARKERS} markers"),
                ));
            }
            let mut m = json!({ "at": short(*at) });
            if let Some(e) = event {
                m["event"] = json!(e);
            }
            if let Some(c) = cue {
                m["cue"] = json!(c);
            }
            let i = list.partition_point(|x| at_of(x) <= short(*at).as_f64().unwrap_or(0.0));
            list.insert(i, m);
        }
        MarkerEdit::Remove(i) => {
            check_index(*i, list.len())?;
            list.remove(*i);
        }
        MarkerEdit::Move(i, at) => {
            check_index(*i, list.len())?;
            check_time(*at)?;
            if list[*i]["at"] == short(*at) {
                return Ok(vec![]);
            }
            let mut m = list.remove(*i);
            m["at"] = short(*at);
            let j = list.partition_point(|x| at_of(x) <= short(*at).as_f64().unwrap_or(0.0));
            list.insert(j, m);
        }
    }
    if list.is_empty() {
        markers.remove(clip);
    } else {
        markers.insert(clip.to_owned(), Value::Array(list));
    }
    if markers.is_empty() {
        lib_map.remove("markers");
    }
    Ok(vec![Edit::Core(CoreEdit::SetResource {
        id: LIBRARY.into(),
        value: lib,
    })])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops_graph::{graph_edit, graph_remove, graph_set_node, FsmEdit};

    fn apply(w: &World, edits: Vec<Edit>) -> World {
        let mut w = w.clone();
        for e in edits {
            e.apply(&mut w, &Registry::core()).unwrap();
        }
        w
    }

    fn code(e: anyhow::Error) -> String {
        e.downcast_ref::<Diagnostic>().unwrap().code.clone()
    }

    fn lab() -> World {
        let mut w = World::empty("t");
        let edits = anim_create(&w, &Registry::core(), "loco", None).unwrap();
        w = apply(&w, edits);
        w
    }

    fn ctl(w: &World) -> &Value {
        &w.resources[ANIM_RESOURCE]["controllers"]["loco"]
    }

    #[test]
    fn create_add_states_transitions_and_parameters() {
        let w = lab();
        assert_eq!(ctl(&w)["initial"], "idle");
        assert_eq!(ctl(&w)["states"]["idle"], json!({}));
        let again = anim_create(&w, &Registry::core(), "loco", None).unwrap_err();
        assert_eq!(code(again), "id_taken");
        let add = |w: &World, e: FsmEdit| {
            apply(w, graph_edit(w, GraphDoc::Animator, "loco", &e).unwrap())
        };
        let w = add(
            &w,
            FsmEdit::AddState {
                name: "walk".into(),
                state: Some(json!({"motion":{"kind":"clip","clip":"walk"}})),
            },
        );
        // A transition without a body: after one cycle, 0.2 s fade; a body is merged.
        let w = add(
            &w,
            FsmEdit::AddTransition {
                from: "walk".into(),
                to: "idle".into(),
                when: None,
            },
        );
        assert_eq!(
            ctl(&w)["transitions"][0],
            json!({"from":"walk","to":"idle","duration":0.2,"exit_time":1.0})
        );
        let w = add(
            &w,
            FsmEdit::AddTransition {
                from: "idle".into(),
                to: "walk".into(),
                when: Some(
                    json!({"conditions":[{"param":"speed","op":"gt","value":0.1}],"duration":0.3}),
                ),
            },
        );
        assert_eq!(ctl(&w)["transitions"][1]["duration"], 0.3);
        assert!(ctl(&w)["transitions"][1].get("exit_time").is_none());
        // Parameters: add, unchanged = no edit, refused removal while used.
        let w = apply(
            &w,
            anim_param(&w, "loco", "speed", Some(&json!({"kind":"float"}))).unwrap(),
        );
        assert!(
            anim_param(&w, "loco", "speed", Some(&json!({"kind":"float"})))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            code(anim_param(&w, "loco", "speed", None).unwrap_err()),
            "param_in_use"
        );
        assert_eq!(
            code(anim_param(&w, "loco", "nope", None).unwrap_err()),
            "not_found"
        );
        assert_eq!(
            code(anim_param(&w, "ghost", "x", None).unwrap_err()),
            "not_found"
        );
        // Graph positions live under anim/ID; removing the controller drops them.
        let w = apply(
            &w,
            graph_set_node(&w, GraphDoc::Animator, "loco", "walk", [10.0, 20.0]).unwrap(),
        );
        assert_eq!(
            w.resources["editor.layout"]["graphs"]["anim/loco"]["walk"],
            json!([10.0, 20.0])
        );
        let w = apply(&w, graph_remove(&w, GraphDoc::Animator, "loco").unwrap());
        assert!(w.resources[ANIM_RESOURCE]["controllers"]
            .get("loco")
            .is_none());
        assert!(w
            .resources
            .get("editor.layout")
            .is_none_or(|l| l["graphs"].get("anim/loco").is_none()));
    }

    #[test]
    fn removal_is_refused_while_attached_and_problems_name_dangling_parts() {
        let mut w = lab();
        let mut e = Entity::new("bot");
        e.components
            .insert("animation.controller".into(), json!({"controller":"loco"}));
        w.entities.push(e);
        let refused = graph_remove(&w, GraphDoc::Animator, "loco").unwrap_err();
        assert_eq!(code(refused), "machine_in_use");
        // A state nobody reaches and an entity naming a missing controller.
        let w = apply(
            &w,
            graph_edit(
                &w,
                GraphDoc::Animator,
                "loco",
                &FsmEdit::AddState {
                    name: "lonely".into(),
                    state: None,
                },
            )
            .unwrap(),
        );
        let mut w = w;
        w.entities[0]
            .components
            .insert("animation.controller".into(), json!({"controller":"ghost"}));
        let codes: Vec<&str> = anim_problems(&w).iter().map(|p| p.code).collect();
        assert_eq!(codes, ["anim_missing_controller", "anim_unreachable"]);
    }
}

#[cfg(test)]
mod layer_tests {
    use super::*;

    fn apply(w: &World, edits: Vec<Edit>) -> World {
        let mut w = w.clone();
        for e in edits {
            e.apply(&mut w, &Registry::core()).unwrap();
        }
        w
    }

    fn code(e: anyhow::Error) -> String {
        e.downcast_ref::<Diagnostic>().unwrap().code.clone()
    }

    #[test]
    fn layers_append_replace_remove_and_guard_their_parameter() {
        let w = World::empty("t");
        let w = apply(&w, anim_create(&w, &Registry::core(), "c", None).unwrap());
        let w = apply(
            &w,
            anim_param(&w, "c", "k", Some(&json!({"kind":"float"}))).unwrap(),
        );
        let def = json!({"clip":"wave","name":"ignored","weight_parameter":"k"});
        let w = apply(&w, anim_layer(&w, "c", "a", Some(&def)).unwrap());
        let w = apply(
            &w,
            anim_layer(&w, "c", "b", Some(&json!({"clip":"hop"}))).unwrap(),
        );
        let layers = &w.resources[ANIM_RESOURCE]["controllers"]["c"]["layers"];
        assert_eq!(
            layers[0],
            json!({"name":"a","clip":"wave","weight_parameter":"k"})
        );
        assert_eq!(layers[1]["name"], "b");
        // Same body = no edit; a new body replaces in place.
        assert!(anim_layer(&w, "c", "a", Some(&def)).unwrap().is_empty());
        let w = apply(
            &w,
            anim_layer(&w, "c", "a", Some(&json!({"clip":"run"}))).unwrap(),
        );
        assert_eq!(
            w.resources[ANIM_RESOURCE]["controllers"]["c"]["layers"][0]["clip"],
            "run"
        );
        assert_eq!(
            code(anim_layer(&w, "c", "a", Some(&json!({"weight":1}))).unwrap_err()),
            "invalid_value"
        );
        assert_eq!(
            code(anim_layer(&w, "c", "zz", None).unwrap_err()),
            "not_found"
        );
        assert_eq!(
            code(anim_layer(&w, "c", "bad name", None).unwrap_err()),
            "invalid_id"
        );
        // The budget.
        let mut full = w.clone();
        for n in ["x", "y"] {
            full = apply(
                &full,
                anim_layer(&full, "c", n, Some(&json!({"clip":"hop"}))).unwrap(),
            );
        }
        assert_eq!(
            code(anim_layer(&full, "c", "z", Some(&json!({"clip":"hop"}))).unwrap_err()),
            "invalid_value"
        );
        // A parameter a layer uses cannot be removed; removing the last layer drops the key.
        let w = apply(&w, anim_layer(&w, "c", "a", Some(&def)).unwrap());
        assert_eq!(
            code(anim_param(&w, "c", "k", None).unwrap_err()),
            "param_in_use"
        );
        let w = apply(&w, anim_layer(&w, "c", "a", None).unwrap());
        let w = apply(&w, anim_layer(&w, "c", "b", None).unwrap());
        assert!(w.resources[ANIM_RESOURCE]["controllers"]["c"]
            .get("layers")
            .is_none());
    }

    #[test]
    fn mesh_parts_toggle_the_own_hidden_list() {
        let mut w = World::empty("t");
        let mut e = Entity::new("hero");
        e.components
            .insert("animation.player".into(), json!({"asset":"a"}));
        w.entities.push(e);
        w.entities.push(Entity::new("bare"));
        let ids = vec!["hero".to_owned()];
        let w = apply(&w, mesh_part(&w, &ids, "hat", false).unwrap());
        assert_eq!(
            w.entities[0].components["animation.player"]["hidden"],
            json!(["hat"])
        );
        assert!(mesh_part(&w, &ids, "hat", false).unwrap().is_empty());
        let w = apply(&w, mesh_part(&w, &ids, "hat", true).unwrap());
        assert_eq!(
            w.entities[0].components["animation.player"],
            json!({"asset":"a"})
        );
        assert!(mesh_part(&w, &ids, "hat", true).unwrap().is_empty());
        assert_eq!(
            code(mesh_part(&w, &["bare".into()], "hat", false).unwrap_err()),
            "not_found"
        );
        assert_eq!(
            code(mesh_part(&w, &[], "hat", false).unwrap_err()),
            "empty_selection"
        );
    }
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    fn world() -> World {
        let mut w = World::empty("t");
        w.resources
            .insert(LIBRARY.into(), json!({"assets":{},"clips":{}}));
        w
    }

    fn apply(w: &World, edits: Vec<Edit>) -> World {
        let mut w = w.clone();
        for e in edits {
            e.apply(&mut w, &Registry::core()).unwrap();
        }
        w
    }

    #[test]
    fn markers_stay_sorted_and_an_emptied_clip_drops_its_entry() {
        let w = world();
        let add = |w: &World, at: f32, event: Option<&str>, cue: Option<&str>| {
            let edit = MarkerEdit::Add {
                at,
                event: event.map(str::to_owned),
                cue: cue.map(str::to_owned),
            };
            apply(w, anim_marker(w, "walk", &edit).unwrap())
        };
        let w = add(&w, 0.6, Some("step"), None);
        let w = add(&w, 0.1, None, Some("chime"));
        let list = &w.resources[LIBRARY]["markers"]["walk"];
        assert_eq!(
            list,
            &json!([{"at":0.1,"cue":"chime"},{"at":0.6,"event":"step"}])
        );
        let w = apply(
            &w,
            anim_marker(&w, "walk", &MarkerEdit::Move(0, 0.9)).unwrap(),
        );
        assert_eq!(w.resources[LIBRARY]["markers"]["walk"][1]["cue"], "chime");
        assert!(anim_marker(&w, "walk", &MarkerEdit::Move(1, 0.9))
            .unwrap()
            .is_empty());
        let none = MarkerEdit::Add {
            at: 0.2,
            event: None,
            cue: None,
        };
        assert!(anim_marker(&w, "walk", &none).is_err());
        let out = anim_marker(&w, "walk", &MarkerEdit::Remove(5)).unwrap_err();
        assert_eq!(
            out.downcast_ref::<Diagnostic>().unwrap().code,
            "index_out_of_range"
        );
        let w = apply(&w, anim_marker(&w, "walk", &MarkerEdit::Remove(0)).unwrap());
        let w = apply(&w, anim_marker(&w, "walk", &MarkerEdit::Remove(0)).unwrap());
        assert!(w.resources[LIBRARY].get("markers").is_none());
    }
}
