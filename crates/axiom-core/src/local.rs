//! Stable object addresses and bounded, projected authoring queries.
use crate::*;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
pub fn escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
pub fn fingerprint(value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    Ok(format!(
        "{:016x}",
        bytes
            .iter()
            .fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64)
                .wrapping_mul(0x100000001b3))
    ))
}
pub fn at<T>(path: &str, result: Result<T>) -> Result<T> {
    result.map_err(|e| {
        if e.downcast_ref::<Diagnostic>().is_some() {
            e
        } else {
            Diagnostic::error("invalid_value", path, format!("{e:#}"))
        }
    })
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Entity { id: String },
    Resource { id: String },
    Prefab { id: String },
    Scene { id: String },
    Node { id: String },
    Choice { node: String, id: String },
    Rule { id: String },
    Widget { id: String },
    Variable { id: String },
}
impl Target {
    pub fn id(&self) -> &str {
        match self {
            Self::Entity { id }
            | Self::Resource { id }
            | Self::Prefab { id }
            | Self::Scene { id }
            | Self::Node { id }
            | Self::Choice { id, .. }
            | Self::Rule { id }
            | Self::Widget { id }
            | Self::Variable { id } => id,
        }
    }
    pub fn path(&self) -> String {
        let id = escape(self.id());
        match self {
            Self::Entity { .. } => format!("/entities/@{id}"),
            Self::Resource { .. } => format!("/resources/{id}"),
            Self::Prefab { .. } => format!("/prefabs/{id}"),
            Self::Scene { .. } => format!("/scenes/@{id}"),
            Self::Node { .. } => format!("/resources/narrative.graph/nodes/@{id}"),
            Self::Choice { node, .. } => format!(
                "/resources/narrative.graph/nodes/@{}/choices/@{id}",
                escape(node)
            ),
            Self::Rule { .. } => format!("/resources/behavior.program/rules/@{id}"),
            Self::Widget { .. } => format!("/resources/ui.document/widgets/@{id}"),
            Self::Variable { .. } => format!("/resources/behavior.program/variables/{id}"),
        }
    }
    fn location(&self, doc: &Value) -> Result<(String, bool)> {
        valid_id(self.id())?;
        let (collection, key, array) = match self {
            Self::Entity { id } => ("/entities".into(), id, true),
            Self::Resource { id } => ("/resources".into(), id, false),
            Self::Prefab { id } => ("/prefabs".into(), id, false),
            Self::Scene { id } => ("/scenes".into(), id, true),
            Self::Node { id } => ("/resources/narrative.graph/nodes".into(), id, true),
            Self::Rule { id } => ("/resources/behavior.program/rules".into(), id, true),
            Self::Widget { id } => ("/resources/ui.document/widgets".into(), id, true),
            Self::Variable { id } => ("/resources/behavior.program/variables".into(), id, false),
            Self::Choice { node, id } => {
                valid_id(node)?;
                let (p, _) = Self::Node { id: node.clone() }.location(doc)?;
                ensure!(doc.pointer(&p).is_some(), "node not found");
                (format!("{p}/choices"), id, true)
            }
        };
        let parent = doc.pointer(&collection).ok_or_else(|| {
            Diagnostic::error("not_found", self.path(), "object collection missing")
        })?;
        if array {
            let list = parent.as_array().context("expected object collection")?;
            Ok((
                format!(
                    "{collection}/{}",
                    list.iter()
                        .position(|v| v.get("id").and_then(Value::as_str) == Some(key))
                        .unwrap_or(list.len())
                ),
                true,
            ))
        } else {
            ensure!(parent.is_object(), "expected object map");
            Ok((format!("{collection}/{}", escape(key)), false))
        }
    }
    pub fn get(&self, doc: &Value) -> Result<Value> {
        at(
            &self.path(),
            (|| {
                let (p, _) = self.location(doc)?;
                doc.pointer(&p).cloned().ok_or_else(|| {
                    Diagnostic::error("not_found", self.path(), "object ID not found")
                })
            })(),
        )
    }
    pub fn edit(&self, world: &mut World, field: Option<&str>, value: Option<Value>) -> Result<()> {
        let mut doc = json!(world);
        if let (Self::Entity { id }, Some(field)) = (self, field) {
            let parts = pointer(field)
                .split('/')
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if parts.get(1).is_some_and(|p| p == "components") {
                if let Some(component) = parts.get(2) {
                    let component = component.replace("~1", "/").replace("~0", "~");
                    let inherited = world
                        .entity(id)
                        .and_then(|e| e.prefab.as_ref())
                        .and_then(|p| world.prefabs.get(p))
                        .and_then(|p| p.components.get(&component))
                        .cloned();
                    if let Some(inherited) = inherited {
                        let (path, _) = self.location(&doc)?;
                        let components = doc
                            .pointer_mut(&path)
                            .and_then(|v| v.get_mut("components"))
                            .and_then(Value::as_object_mut)
                            .context("entity components missing")?;
                        let value = components.entry(component).or_insert_with(|| {
                            if inherited.is_object() {
                                json!({})
                            } else {
                                inherited.clone()
                            }
                        });
                        materialize_parents(
                            value,
                            &inherited,
                            &parts[3..parts.len().saturating_sub(1).max(3)],
                        )?;
                    }
                }
            }
        }
        if field.is_none() && value.is_some() {
            let location = match self {
                Self::Rule { .. } => Some(("/resources/behavior.program", "rules", json!([]))),
                Self::Variable { .. } => {
                    Some(("/resources/behavior.program", "variables", json!({})))
                }
                Self::Choice { node, .. } => {
                    let (p, _) = Self::Node { id: node.clone() }.location(&doc)?;
                    if let Some(parent) = doc.pointer_mut(&p).and_then(Value::as_object_mut) {
                        parent.entry("choices").or_insert_with(|| json!([]));
                    }
                    None
                }
                _ => None,
            };
            if let Some((parent, key, default)) = location {
                if let Some(parent) = doc.pointer_mut(parent).and_then(Value::as_object_mut) {
                    parent.entry(key).or_insert(default);
                }
            }
        }
        let logical = self.path();
        at(
            &logical,
            (|| -> Result<()> {
                let (path, array) = self.location(&doc)?;
                if let Some(field) = field {
                    let field = pointer(field);
                    ensure!(
                        !field.is_empty() && field != "/id",
                        "use put_object for replacement; object ID is immutable"
                    );
                    let object = doc.pointer_mut(&path).ok_or_else(|| {
                        Diagnostic::error("not_found", &logical, "object ID not found")
                    })?;
                    let (parent, key) = field.rsplit_once('/').context("invalid field pointer")?;
                    let parent = object.pointer_mut(parent).ok_or_else(|| {
                        Diagnostic::error(
                            "invalid_field",
                            format!("{logical}{field}"),
                            "field parent missing",
                        )
                    })?;
                    let key = key.replace("~1", "/").replace("~0", "~");
                    if let Some(map) = parent.as_object_mut() {
                        map.insert(key, value.context("field value missing")?);
                    } else if let Some(list) = parent.as_array_mut() {
                        let index = key.parse::<usize>()?;
                        *list.get_mut(index).context("field index out of range")? =
                            value.context("field value missing")?;
                    } else {
                        anyhow::bail!("field parent is not a record or array");
                    }
                } else {
                    let (parent, key) = path.rsplit_once('/').unwrap();
                    let parent = doc.pointer_mut(parent).unwrap();
                    if array {
                        let list = parent.as_array_mut().unwrap();
                        let index = key.parse::<usize>()?;
                        match value {
                            Some(v) => {
                                ensure!(
                                    v.get("id").and_then(Value::as_str) == Some(self.id()),
                                    "replacement ID must match target ID"
                                );
                                if index == list.len() {
                                    list.push(v)
                                } else {
                                    list[index] = v
                                }
                            }
                            None => {
                                ensure!(index < list.len(), "object ID not found");
                                list.remove(index);
                            }
                        }
                    } else {
                        let map = parent.as_object_mut().unwrap();
                        let key = key.replace("~1", "/").replace("~0", "~");
                        if let Some(v) = value {
                            map.insert(key, v);
                        } else {
                            ensure!(map.remove(&key).is_some(), "object ID not found");
                        }
                    }
                }
                *world = serde_json::from_value(doc)?;
                Ok(())
            })(),
        )
    }
}
fn materialize_parents(value: &mut Value, inherited: &Value, parts: &[String]) -> Result<()> {
    if parts.is_empty() {
        return Ok(());
    }
    let key = parts[0].replace("~1", "/").replace("~0", "~");
    let default = if inherited.is_object() {
        inherited.get(&key)
    } else {
        key.parse::<usize>().ok().and_then(|i| inherited.get(i))
    };
    if let Some(map) = value.as_object_mut() {
        if !map.contains_key(&key) {
            let default = default.context("inherited field parent missing")?;
            map.insert(
                key.clone(),
                if default.is_object() {
                    json!({})
                } else {
                    default.clone()
                },
            );
        }
        materialize_parents(
            map.get_mut(&key).unwrap(),
            default.unwrap_or(&Value::Null),
            &parts[1..],
        )
    } else if let Some(list) = value.as_array_mut() {
        let i = key.parse::<usize>()?;
        materialize_parents(
            list.get_mut(i).context("field index missing")?,
            default.unwrap_or(&Value::Null),
            &parts[1..],
        )
    } else {
        anyhow::bail!("field parent is not a record")
    }
}
pub fn pointer(field: &str) -> String {
    if field.is_empty() || field.starts_with('/') {
        field.into()
    } else {
        format!("/{}", escape(field))
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    Entities,
    Resources,
    Prefabs,
    Scenes,
    Nodes,
    Choices,
    Rules,
    Widgets,
    Variables,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub from: Collection,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub max_bytes: Option<usize>,
}
impl Selection {
    pub fn all(from: Collection) -> Self {
        Self {
            from,
            ids: vec![],
            tags: vec![],
            components: vec![],
            parent: None,
            fields: vec![],
            limit: None,
            cursor: None,
            max_bytes: None,
        }
    }
}
pub fn select(w: &World, r: &Registry, rev: i64, q: &Selection) -> Result<Value> {
    ensure!(
        q.ids.len() <= 128
            && q.tags.len() <= 32
            && q.components.len() <= 32
            && q.fields.len() <= 32,
        "selection budget exceeded"
    );
    for id in q.ids.iter().chain(&q.tags).chain(&q.components) {
        valid_id(id)?;
    }
    for field in &q.fields {
        ensure!(field.len() <= 512, "field path exceeds budget");
    }
    ensure!(
        (q.tags.is_empty() && q.components.is_empty()) || q.from == Collection::Entities,
        "tags/components only filter entities"
    );
    ensure!(
        q.from == Collection::Choices || q.parent.is_none(),
        "parent only applies to choices"
    );
    let limit = q.limit.unwrap_or(16);
    ensure!((1..=128).contains(&limit), "selection limit must be 1..128");
    let max = q.max_bytes.unwrap_or(8192);
    ensure!((256..=65536).contains(&max), "max_bytes must be 256..65536");
    let signature = fingerprint(&(q.from, &q.ids, &q.tags, &q.components, &q.parent, &q.fields))?;
    let prefix = format!("l1.{rev}.{signature}.");
    let offset = if let Some(cursor) = &q.cursor {
        cursor
            .strip_prefix(&prefix)
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or_else(|| {
                Diagnostic::error(
                    "stale_cursor",
                    "/query/cursor",
                    "revision or selection changed; restart selection",
                )
            })?
    } else {
        0
    };
    let doc = json!(w);
    let mut records = vec![];
    let mut add = |target: Target, data: Value| {
        if q.ids.is_empty() || q.ids.iter().any(|id| id == target.id()) {
            records.push((target, data));
        }
    };
    match q.from {
        Collection::Entities => {
            let compiled = w.compile(r)?;
            for (source, e) in w.entities.iter().zip(compiled.entities) {
                if q.tags.iter().all(|t| e.tags.contains(t))
                    && q.components.iter().all(|c| e.components.contains_key(c))
                {
                    add(
                        Target::Entity { id: e.id.clone() },
                        json!({"id":e.id,"label":source.label,"prefab":source.prefab,"tags":e.tags,"components":e.components}),
                    );
                }
            }
        }
        Collection::Resources => {
            for (id, v) in &w.resources {
                add(Target::Resource { id: id.clone() }, v.clone());
            }
        }
        Collection::Prefabs => {
            for (id, v) in &w.prefabs {
                add(Target::Prefab { id: id.clone() }, json!(v));
            }
        }
        Collection::Scenes => {
            for v in &w.scenes {
                add(Target::Scene { id: v.id.clone() }, json!(v));
            }
        }
        Collection::Variables => {
            if let Some(vars) = doc
                .pointer("/resources/behavior.program/variables")
                .and_then(Value::as_object)
            {
                for (id, v) in vars {
                    add(Target::Variable { id: id.clone() }, v.clone());
                }
            }
        }
        Collection::Nodes | Collection::Choices => {
            if let Some(nodes) = doc
                .pointer("/resources/narrative.graph/nodes")
                .and_then(Value::as_array)
            {
                for node in nodes {
                    let id = node["id"].as_str().unwrap();
                    if q.from == Collection::Nodes {
                        add(Target::Node { id: id.into() }, node.clone());
                    } else if q.parent.as_ref().is_none_or(|p| p == id) {
                        if let Some(choices) = node["choices"].as_array() {
                            for c in choices {
                                add(
                                    Target::Choice {
                                        node: id.into(),
                                        id: c["id"].as_str().unwrap().into(),
                                    },
                                    c.clone(),
                                );
                            }
                        }
                    }
                }
            }
        }
        Collection::Rules | Collection::Widgets => {
            let (path, rule) = if q.from == Collection::Rules {
                ("/resources/behavior.program/rules", true)
            } else {
                ("/resources/ui.document/widgets", false)
            };
            if let Some(list) = doc.pointer(path).and_then(Value::as_array) {
                for v in list {
                    let id = v["id"].as_str().unwrap().to_owned();
                    add(
                        if rule {
                            Target::Rule { id }
                        } else {
                            Target::Widget { id }
                        },
                        v.clone(),
                    );
                }
            }
        }
    }
    records.sort_by_key(|(t, _)| t.path());
    ensure!(offset <= records.len(), "cursor offset out of range");
    let total = records.len();
    let found = records.iter().map(|(t, _)| t.id()).collect::<BTreeSet<_>>();
    let missing = q
        .ids
        .iter()
        .filter(|id| !found.contains(id.as_str()))
        .collect::<Vec<_>>();
    let mut items = vec![];
    let mut next = offset;
    while next < total && items.len() < limit {
        let (t, v) = &records[next];
        let mut missing_fields = vec![];
        let data = if q.fields.is_empty() {
            summary(t, v)
        } else {
            let mut out = serde_json::Map::new();
            for field in &q.fields {
                if let Some(value) = v.pointer(&pointer(field)) {
                    out.insert(field.clone(), value.clone());
                } else {
                    missing_fields.push(field.clone());
                }
            }
            Value::Object(out)
        };
        let mut item = json!({"target":t,"data":data});
        if !missing_fields.is_empty() {
            item["missing_fields"] = json!(missing_fields);
        }
        items.push(item);
        let candidate = json!({"ok":true,"rev":rev,"total":total,"items":items,"missing_ids":missing,"next_cursor":format!("{prefix}{}",next+1)});
        if serde_json::to_vec(&candidate)?.len() > max {
            items.pop();
            ensure!(
                !items.is_empty(),
                "selected object exceeds max_bytes; request fewer fields or raise max_bytes"
            );
            break;
        }
        next += 1;
    }
    let result = json!({"ok":true,"rev":rev,"total":total,"items":items,"missing_ids":missing,"next_cursor":if next<total{Some(format!("{prefix}{next}"))}else{None}});
    ensure!(
        serde_json::to_vec(&result)?.len() <= max,
        "selection metadata exceeds max_bytes; request fewer IDs"
    );
    Ok(result)
}
fn summary(target: &Target, v: &Value) -> Value {
    match target {
        Target::Variable { .. } => v.clone(),
        Target::Entity { .. } => {
            let mut row = json!({"id":target.id(),"tags":v["tags"],"components":v["components"].as_object().map(|m|m.keys().collect::<Vec<_>>())});
            for key in ["label", "prefab"] {
                if !v[key].is_null() {
                    row[key] = v[key].clone();
                }
            }
            row
        }
        _ => {
            let mut out = serde_json::Map::from_iter([("id".into(), json!(target.id()))]);
            for k in ["speaker", "text", "event", "next", "kind", "active"] {
                if let Some(value) = v.get(k) {
                    out.insert(k.into(), value.clone());
                }
            }
            if let Some(list) = v.get("choices").and_then(Value::as_array) {
                out.insert(
                    "choices".into(),
                    json!(list.iter().map(|c| &c["id"]).collect::<Vec<_>>()),
                );
            }
            if matches!(target, Target::Resource { .. } | Target::Prefab { .. }) {
                out.insert(
                    "keys".into(),
                    json!(v.as_object().map(|m| m.keys().collect::<Vec<_>>())),
                );
            }
            Value::Object(out)
        }
    }
}
