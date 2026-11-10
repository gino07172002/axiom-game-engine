//! Three-way merge of authoring revisions, field by field (re-exported from `ops`).
//!
//! `base` is the revision an edit was made on, `mine` that revision with the edit applied
//! (an Inspector draft, a proposal) and `theirs` the newer revision it now has to land on.
//! The world is split into merge units: an object's label / prefab / tags, each top-level
//! field of a component or resource (the whole component / resource / object when one side
//! adds or removes it), one parent link or relation, the relative order of objects, and the
//! remaining top-level entries (scenes, prefabs, schemas, capabilities, systems) whole.
//! A unit only `mine` changed takes mine's value, one only `theirs` changed keeps theirs, and
//! one both changed to different values is a conflict: theirs wins when its path is in
//! `keep_theirs`, otherwise mine. The result is one edit batch for one revision on `theirs`.
use crate::local::escape;
use crate::*;
use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A field both sides changed to different values. Values are `None` when absent and
/// abbreviated like `diff` changes when larger than 256 bytes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldConflict {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mine: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theirs: Option<Value>,
}

/// What merging `mine` into `theirs` involves.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MergeView {
    pub conflicts: Vec<FieldConflict>,
    /// Units only `mine` changed (taken as they are).
    pub mine_only: usize,
    /// Units only `theirs` changed (kept).
    pub theirs_only: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    WorldId,
    Title,
    Systems,
    Packages,
    Order,
    Capability(String),
    ComponentSchema(String),
    ResourceSchema(String),
    Prefab(String),
    Scene(String),
    Parent(String),
    Relation(String, String, String),
    Entity(String),
    EntityField(String, &'static str),
    Component(String, String),
    ComponentField(String, String, String),
    Resource(String),
    ResourceField(String, String),
}

impl Unit {
    /// Stable path, in the same notation as `diff` changes.
    fn path(&self) -> String {
        match self {
            Unit::WorldId => "/id".into(),
            Unit::Title => "/title".into(),
            Unit::Systems => "/systems".into(),
            Unit::Packages => "/packages".into(),
            // Not an `@ID` path, so it cannot collide with an object called "order".
            Unit::Order => "/entities#order".into(),
            Unit::Capability(k) => format!("/capabilities/{}", escape(k)),
            Unit::ComponentSchema(k) => format!("/component_schemas/{}", escape(k)),
            Unit::ResourceSchema(k) => format!("/resource_schemas/{}", escape(k)),
            Unit::Prefab(k) => format!("/prefabs/{}", escape(k)),
            Unit::Scene(k) => format!("/scenes/@{}", escape(k)),
            Unit::Parent(s) => format!("/relations/parent/@{}", escape(s)),
            Unit::Relation(k, s, t) => {
                format!("/relations/{}/@{}/@{}", escape(k), escape(s), escape(t))
            }
            Unit::Entity(e) => format!("/entities/@{}", escape(e)),
            Unit::EntityField(e, f) => format!("/entities/@{}/{f}", escape(e)),
            Unit::Component(e, c) => format!("/entities/@{}/components/{}", escape(e), escape(c)),
            Unit::ComponentField(e, c, f) => format!(
                "/entities/@{}/components/{}/{}",
                escape(e),
                escape(c),
                escape(f)
            ),
            Unit::Resource(r) => format!("/resources/{}", escape(r)),
            Unit::ResourceField(r, f) => format!("/resources/{}/{}", escape(r), escape(f)),
        }
    }
}

/// Relative order of the objects every side has (what a reorder changes).
fn order(w: &World, common: &BTreeSet<&str>) -> Vec<String> {
    w.entities
        .iter()
        .filter(|e| common.contains(e.id.as_str()))
        .map(|e| e.id.clone())
        .collect()
}

/// A world with an object index (merge units look objects up many times).
struct Indexed<'a> {
    w: &'a World,
    ids: HashMap<&'a str, &'a Entity>,
}

impl<'a> Indexed<'a> {
    fn new(w: &'a World) -> Self {
        Self {
            w,
            ids: w.entities.iter().map(|e| (e.id.as_str(), e)).collect(),
        }
    }
    fn entity(&self, id: &str) -> Option<&'a Entity> {
        self.ids.get(id).copied()
    }
}

fn get(side: &Indexed, unit: &Unit, common: &BTreeSet<&str>) -> Option<Value> {
    let w = side.w;
    let entity = |id: &str| side.entity(id);
    match unit {
        Unit::WorldId => Some(json!(w.id)),
        Unit::Title => Some(json!(w.title)),
        Unit::Systems => Some(json!(w.systems)),
        Unit::Packages => w.packages.as_ref().map(|p| json!(p)),
        Unit::Order => Some(json!(order(w, common))),
        Unit::Capability(k) => w.capabilities.get(k).map(|v| json!(v)),
        Unit::ComponentSchema(k) => w.component_schemas.get(k).map(|v| json!(v)),
        Unit::ResourceSchema(k) => w.resource_schemas.get(k).map(|v| json!(v)),
        Unit::Prefab(k) => w.prefabs.get(k).map(|v| json!(v)),
        Unit::Scene(k) => w.scenes.iter().find(|s| &s.id == k).map(|v| json!(v)),
        Unit::Parent(s) => w
            .relations
            .iter()
            .find(|r| r.kind == "parent" && &r.source == s)
            .map(|r| json!(r.target)),
        Unit::Relation(k, s, t) => w
            .relations
            .iter()
            .any(|r| &r.kind == k && &r.source == s && &r.target == t)
            .then_some(json!(true)),
        Unit::Entity(e) => entity(e).map(|v| json!(v)),
        Unit::EntityField(e, f) => {
            let e = entity(e)?;
            match *f {
                "label" => e.label.as_ref().map(|v| json!(v)),
                "prefab" => e.prefab.as_ref().map(|v| json!(v)),
                _ => Some(json!(e.tags)),
            }
        }
        Unit::Component(e, c) => entity(e)?.components.get(c).cloned(),
        Unit::ComponentField(e, c, f) => entity(e)?.components.get(c)?.get(f).cloned(),
        Unit::Resource(r) => w.resources.get(r).cloned(),
        Unit::ResourceField(r, f) => w.resources.get(r)?.get(f).cloned(),
    }
}

fn decode<T: serde::de::DeserializeOwned>(unit: &Unit, value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|e| {
        Diagnostic::error(
            "invalid_value",
            unit.path(),
            format!("merged value does not fit: {e}"),
        )
    })
}

/// Writes `value` (None = remove) for `unit` into `w`. `mine` places objects mine added.
fn put(w: &mut World, unit: &Unit, value: Option<Value>, mine: &World) -> Result<()> {
    fn set_map<T>(map: &mut BTreeMap<String, T>, key: &str, value: Option<T>) {
        match value {
            Some(v) => {
                map.insert(key.to_owned(), v);
            }
            None => {
                map.remove(key);
            }
        }
    }
    match unit {
        Unit::WorldId => w.id = decode(unit, value.unwrap_or_default())?,
        Unit::Title => w.title = decode(unit, value.unwrap_or_default())?,
        Unit::Systems => w.systems = decode(unit, value.unwrap_or_else(|| json!([])))?,
        Unit::Packages => w.packages = value.map(|v| decode(unit, v)).transpose()?,
        Unit::Order => {
            let wanted: Vec<String> = decode(unit, value.unwrap_or_else(|| json!([])))?;
            let rank: BTreeMap<&str, usize> = wanted
                .iter()
                .enumerate()
                .map(|(i, id)| (id.as_str(), i))
                .collect();
            // The listed objects take mine's relative order in the slots they occupy;
            // every other object keeps its place.
            let slots: Vec<usize> = (0..w.entities.len())
                .filter(|&i| rank.contains_key(w.entities[i].id.as_str()))
                .collect();
            let mut moved: Vec<Entity> = slots.iter().map(|&i| w.entities[i].clone()).collect();
            moved.sort_by_key(|e| rank[e.id.as_str()]);
            for (slot, entity) in slots.into_iter().zip(moved) {
                w.entities[slot] = entity;
            }
        }
        Unit::Capability(k) => {
            let v = value.map(|v| decode(unit, v)).transpose()?;
            set_map(&mut w.capabilities, k, v)
        }
        Unit::ComponentSchema(k) => {
            let v = value.map(|v| decode(unit, v)).transpose()?;
            set_map(&mut w.component_schemas, k, v)
        }
        Unit::ResourceSchema(k) => {
            let v = value.map(|v| decode(unit, v)).transpose()?;
            set_map(&mut w.resource_schemas, k, v)
        }
        Unit::Prefab(k) => {
            let v = value.map(|v| decode(unit, v)).transpose()?;
            set_map(&mut w.prefabs, k, v)
        }
        Unit::Scene(k) => {
            let scene: Option<Scene> = value.map(|v| decode(unit, v)).transpose()?;
            match (w.scenes.iter().position(|s| &s.id == k), scene) {
                (Some(i), Some(s)) => w.scenes[i] = s,
                (Some(i), None) => {
                    w.scenes.remove(i);
                }
                (None, Some(s)) => w.scenes.push(s),
                (None, None) => {}
            }
        }
        Unit::Parent(s) => {
            w.relations
                .retain(|r| !(r.kind == "parent" && &r.source == s));
            if let Some(v) = value {
                w.relations.push(Relation {
                    source: s.clone(),
                    kind: "parent".into(),
                    target: decode(unit, v)?,
                });
            }
        }
        Unit::Relation(k, s, t) => {
            w.relations
                .retain(|r| !(&r.kind == k && &r.source == s && &r.target == t));
            if value.is_some() {
                w.relations.push(Relation {
                    source: s.clone(),
                    kind: k.clone(),
                    target: t.clone(),
                });
            }
        }
        Unit::Entity(id) => {
            let entity: Option<Entity> = value.map(|v| decode(unit, v)).transpose()?;
            match (w.entities.iter().position(|e| &e.id == id), entity) {
                (Some(i), Some(e)) => w.entities[i] = e,
                (Some(i), None) => {
                    w.entities.remove(i);
                }
                (None, Some(e)) => {
                    // After the nearest object before it in mine's order that is here too.
                    // Walks back from it and stops at the first one found (usually the
                    // previous object), so adding k objects is not k × n × n.
                    let own = mine.entities.iter().position(|m| &m.id == id);
                    let at = mine.entities[..own.unwrap_or(mine.entities.len())]
                        .iter()
                        .rev()
                        .find_map(|m| w.entities.iter().position(|x| x.id == m.id))
                        .map_or(0, |i| i + 1);
                    w.entities.insert(at, e);
                }
                (None, None) => {}
            }
        }
        Unit::EntityField(id, f) => {
            let missing = || Diagnostic::error("not_found", unit.path(), "object missing");
            let e = w
                .entities
                .iter_mut()
                .find(|e| &e.id == id)
                .ok_or_else(missing)?;
            match *f {
                "label" => e.label = value.map(|v| decode(unit, v)).transpose()?,
                "prefab" => e.prefab = value.map(|v| decode(unit, v)).transpose()?,
                _ => e.tags = decode(unit, value.unwrap_or_else(|| json!([])))?,
            }
        }
        Unit::Component(id, c) | Unit::ComponentField(id, c, _) => {
            let missing = || Diagnostic::error("not_found", unit.path(), "object missing");
            let e = w
                .entities
                .iter_mut()
                .find(|e| &e.id == id)
                .ok_or_else(missing)?;
            match unit {
                Unit::ComponentField(_, _, f) => {
                    let record = e
                        .components
                        .get_mut(c)
                        .and_then(Value::as_object_mut)
                        .ok_or_else(missing)?;
                    set_map_value(record, f, value);
                }
                _ => match value {
                    Some(v) => {
                        e.components.insert(c.clone(), v);
                    }
                    None => {
                        e.components.remove(c);
                    }
                },
            }
        }
        Unit::Resource(r) => set_map(&mut w.resources, r, value),
        Unit::ResourceField(r, f) => {
            let record = w
                .resources
                .get_mut(r)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| Diagnostic::error("not_found", unit.path(), "resource missing"))?;
            set_map_value(record, f, value);
        }
    }
    Ok(())
}

fn set_map_value(record: &mut serde_json::Map<String, Value>, key: &str, value: Option<Value>) {
    match value {
        Some(v) => {
            record.insert(key.to_owned(), v);
        }
        None => {
            record.remove(key);
        }
    }
}

/// Every merge unit of the three worlds. An object, component or resource is split into
/// fields only when all three have it (as a record); otherwise it is one unit.
fn units(sides: [&Indexed; 3]) -> BTreeSet<Unit> {
    let mut out = BTreeSet::from([Unit::WorldId, Unit::Title, Unit::Systems, Unit::Packages]);
    let all = |f: &dyn Fn(&Indexed) -> bool| sides.iter().all(|s| f(s));
    for side in sides {
        let w = side.w;
        out.extend(w.capabilities.keys().cloned().map(Unit::Capability));
        out.extend(
            w.component_schemas
                .keys()
                .cloned()
                .map(Unit::ComponentSchema),
        );
        out.extend(w.resource_schemas.keys().cloned().map(Unit::ResourceSchema));
        out.extend(w.prefabs.keys().cloned().map(Unit::Prefab));
        out.extend(w.scenes.iter().map(|s| Unit::Scene(s.id.clone())));
        for r in &w.relations {
            out.insert(if r.kind == "parent" {
                Unit::Parent(r.source.clone())
            } else {
                Unit::Relation(r.kind.clone(), r.source.clone(), r.target.clone())
            });
        }
        for e in &w.entities {
            if !all(&|s| s.entity(&e.id).is_some()) {
                out.insert(Unit::Entity(e.id.clone()));
                continue;
            }
            for f in ["label", "prefab", "tags"] {
                out.insert(Unit::EntityField(e.id.clone(), f));
            }
            for (c, value) in &e.components {
                let record = |s: &Indexed| {
                    s.entity(&e.id)
                        .and_then(|x| x.components.get(c))
                        .is_some_and(Value::is_object)
                };
                if all(&record) {
                    let keys = value.as_object().into_iter().flat_map(|o| o.keys());
                    out.extend(
                        keys.map(|f| Unit::ComponentField(e.id.clone(), c.clone(), f.clone())),
                    );
                } else {
                    out.insert(Unit::Component(e.id.clone(), c.clone()));
                }
            }
        }
        for (r, value) in &w.resources {
            if all(&|s| s.w.resources.get(r).is_some_and(Value::is_object)) {
                let keys = value.as_object().into_iter().flat_map(|o| o.keys());
                out.extend(keys.map(|f| Unit::ResourceField(r.clone(), f.clone())));
            } else {
                out.insert(Unit::Resource(r.clone()));
            }
        }
    }
    out.insert(Unit::Order);
    out
}

struct Plan {
    /// (unit, mine's value, conflict) for every unit mine changed.
    take: Vec<(Unit, Option<Value>, Option<FieldConflict>)>,
    theirs_only: usize,
}

fn plan(base: &World, mine: &World, theirs: &World) -> Plan {
    let (b_side, m_side, t_side) = (Indexed::new(base), Indexed::new(mine), Indexed::new(theirs));
    let common: BTreeSet<&str> = base
        .entities
        .iter()
        .map(|e| e.id.as_str())
        .filter(|id| m_side.entity(id).is_some() && t_side.entity(id).is_some())
        .collect();
    let mut take = vec![];
    let mut theirs_only = 0;
    for unit in units([&b_side, &m_side, &t_side]) {
        let b = get(&b_side, &unit, &common);
        let m = get(&m_side, &unit, &common);
        let t = get(&t_side, &unit, &common);
        match (m != b, t != b) {
            (true, true) if m != t => {
                let conflict = FieldConflict {
                    path: unit.path(),
                    base: b.as_ref().map(diff::preview),
                    mine: m.as_ref().map(diff::preview),
                    theirs: t.as_ref().map(diff::preview),
                };
                take.push((unit, m, Some(conflict)));
            }
            (true, true) => {}
            (true, false) => take.push((unit, m, None)),
            (false, true) => theirs_only += 1,
            (false, false) => {}
        }
    }
    Plan { take, theirs_only }
}

/// The units both sides changed to different values, and how many each side changed alone.
pub fn merge_view(base: &World, mine: &World, theirs: &World) -> MergeView {
    let plan = plan(base, mine, theirs);
    let conflicts: Vec<FieldConflict> =
        plan.take.iter().filter_map(|(_, _, c)| c.clone()).collect();
    MergeView {
        mine_only: plan.take.len() - conflicts.len(),
        conflicts,
        theirs_only: plan.theirs_only,
    }
}

/// `theirs` with mine's changes merged in (conflicts listed in `keep_theirs` keep theirs).
/// Every `keep_theirs` path must name a conflict (`unknown_conflict` otherwise).
pub fn merge_worlds(
    base: &World,
    mine: &World,
    theirs: &World,
    keep_theirs: &[String],
) -> Result<World> {
    let plan = plan(base, mine, theirs);
    let conflicts: BTreeSet<String> = plan
        .take
        .iter()
        .filter_map(|(_, _, c)| c.as_ref().map(|c| c.path.clone()))
        .collect();
    if let Some(unknown) = keep_theirs.iter().find(|p| !conflicts.contains(*p)) {
        return Err(Diagnostic::error(
            "unknown_conflict",
            unknown.clone(),
            format!(
                "{unknown} is not a conflicting field ({} conflict(s): {})",
                conflicts.len(),
                conflicts
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    let keep: BTreeSet<&str> = keep_theirs.iter().map(String::as_str).collect();
    let mut merged = theirs.clone();
    // Objects first (fields of a re-added object need it), the relative order last.
    let mut take = plan.take;
    take.sort_by_key(|(u, _, _)| match u {
        Unit::Entity(_) => 0,
        Unit::Order => 2,
        _ => 1,
    });
    for (unit, value, conflict) in take {
        if conflict.is_some() && keep.contains(unit.path().as_str()) {
            continue;
        }
        put(&mut merged, &unit, value, mine)?;
    }
    Ok(merged)
}

/// `base` with `edits` applied in order, errors located like a commit's (`/edits/N`).
pub fn apply_edits(base: &World, edits: &[Edit], registry: &Registry) -> Result<World> {
    let mut next = base.clone();
    for (i, edit) in edits.iter().cloned().enumerate() {
        crate::local::at(&format!("/edits/{i}"), edit.apply(&mut next, registry))?;
    }
    Ok(next)
}

/// The edit batch that commits the merge on `theirs` as one revision (empty when the merge
/// leaves `theirs` unchanged).
pub fn merge(
    base: &World,
    mine: &World,
    theirs: &World,
    keep_theirs: &[String],
) -> Result<Vec<Edit>> {
    let merged = merge_worlds(base, mine, theirs, keep_theirs)?;
    Ok(if &merged == theirs {
        vec![]
    } else {
        vec![Edit::replace(merged)]
    })
}
