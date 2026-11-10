//! Trigger zones (`trigger.zone`): a box or sphere placed by the entity's world transform
//! that reports which matching entities are inside and emits `behavior.program` events when
//! one enters or leaves. Detection runs in the runtime system `trigger.detect`.
//!
//! An entity is "inside" when its pivot (world position) lies in the shape; entity bounds
//! are not used. The shape follows the world position and rotation of the trigger entity
//! (parents included); transform scale is ignored, so sizes are always meters.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Trigger zones per world (every entity with `trigger.zone`, enabled or not).
pub const MAX_TRIGGERS: usize = 256;
/// Enter + exit events one fixed step may emit across all zones.
pub const MAX_TRIGGER_EVENTS_PER_STEP: usize = 64;
/// Worst-case point-in-shape tests per step: the sum over zones of their candidate entities
/// (what the grid degrades to when everything shares a cell or zones are huge). Measured at
/// this bound in release; see docs/triggers.zh-TW.md.
pub const MAX_TRIGGER_PAIRS: usize = 256 * 1024;
/// Tags / components per filter.
pub const MAX_FILTER_TERMS: usize = 8;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerShape {
    Box,
    Sphere,
}

/// Which entities can trigger the zone. Same vocabulary and AND semantics as a selection's
/// `tags` / `components` (ACL `entities[tag=..,has=..]`): an entity must carry every tag
/// and every component. Empty = any entity with `core.transform`. The trigger entity itself
/// never triggers its own zone.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TriggerFilter {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub components: Vec<String>,
}

impl TriggerFilter {
    pub fn matches(&self, tags: &[String], has: impl Fn(&str) -> bool) -> bool {
        self.tags.iter().all(|t| tags.contains(t)) && self.components.iter().all(|c| has(c))
    }
    /// [`Self::matches`] on interned tags: `required` is `index.resolve(&self.tags)` of the
    /// world's [`crate::tags::TagIndex`] (`None`: a filter tag no entity carries).
    pub fn matches_interned(
        &self,
        required: Option<&crate::tags::TagSet>,
        tags: &crate::tags::TagSet,
        has: impl Fn(&str) -> bool,
    ) -> bool {
        required.is_some_and(|r| tags.contains_all(r)) && self.components.iter().all(|c| has(c))
    }
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TriggerZone {
    pub shape: TriggerShape,
    /// Box only (required): half size along the entity's local X, Y, Z in meters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_extents: Option<[f32; 3]>,
    /// Sphere only (required): radius in meters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default)]
    pub filter: TriggerFilter,
    /// `behavior.program` event emitted when a matching entity enters ("" = none).
    #[serde(default)]
    pub on_enter: String,
    /// `behavior.program` event emitted when an occupant leaves ("" = none).
    #[serde(default)]
    pub on_exit: String,
    /// Emit at most one enter and one exit event (occupancy is still tracked).
    #[serde(default)]
    pub once: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Hysteresis: an occupant leaves only when it is farther than the shape grown by this
    /// many meters (0 = leave at the same boundary it entered).
    #[serde(default)]
    pub exit_margin: f32,
}

fn event_ok(id: &str) -> Result<()> {
    if !id.is_empty() {
        crate::valid_id(id)?;
    }
    Ok(())
}

impl TriggerZone {
    pub fn sphere(radius: f32) -> Self {
        Self {
            shape: TriggerShape::Sphere,
            half_extents: None,
            radius: Some(radius),
            filter: TriggerFilter::default(),
            on_enter: String::new(),
            on_exit: String::new(),
            once: false,
            enabled: true,
            exit_margin: 0.0,
        }
    }
    pub fn boxed(half_extents: [f32; 3]) -> Self {
        Self {
            shape: TriggerShape::Box,
            half_extents: Some(half_extents),
            radius: None,
            ..Self::sphere(1.0)
        }
    }
    pub fn validate(&self) -> Result<()> {
        let size = |v: f32| v.is_finite() && (0.01..=1000.0).contains(&v);
        match (self.shape, self.half_extents, self.radius) {
            (TriggerShape::Box, Some(h), None) => ensure!(
                h.iter().all(|v| size(*v)),
                "box half_extents must be 0.01..1000 meters"
            ),
            (TriggerShape::Sphere, None, Some(r)) => {
                ensure!(size(r), "sphere radius must be 0.01..1000 meters")
            }
            (TriggerShape::Box, _, Some(_)) => bail!("radius is only valid on shape sphere"),
            (TriggerShape::Sphere, Some(_), _) => {
                bail!("half_extents is only valid on shape box")
            }
            (TriggerShape::Box, None, None) => bail!("box trigger requires half_extents [x,y,z]"),
            (TriggerShape::Sphere, None, None) => bail!("sphere trigger requires radius"),
        }
        ensure!(
            self.exit_margin.is_finite() && (0.0..=100.0).contains(&self.exit_margin),
            "exit_margin must be 0..100 meters"
        );
        ensure!(
            self.filter.tags.len() <= MAX_FILTER_TERMS
                && self.filter.components.len() <= MAX_FILTER_TERMS,
            "filter allows up to {MAX_FILTER_TERMS} tags and {MAX_FILTER_TERMS} components"
        );
        for id in self.filter.tags.iter().chain(&self.filter.components) {
            crate::valid_id(id).context("filter")?;
        }
        event_ok(&self.on_enter).context("on_enter")?;
        event_ok(&self.on_exit).context("on_exit")?;
        Ok(())
    }
    /// Events this zone emits, without the empty ones.
    pub fn events(&self) -> impl Iterator<Item = &str> {
        [self.on_enter.as_str(), self.on_exit.as_str()]
            .into_iter()
            .filter(|e| !e.is_empty())
    }
}

pub fn trigger_schema() -> Value {
    json!({
        "shape": "box | sphere. Placed by the entity's world position and rotation (parents included); scale is ignored",
        "half_extents": "box only (required): [x,y,z] meters 0.01..1000 along local axes",
        "radius": "sphere only (required): meters 0.01..1000",
        "filter": format!("{{tags:[..], components:[..]}} up to {MAX_FILTER_TERMS} each; an entity needs every tag and component (like entities[tag=..,has=..]); empty = any entity with core.transform; never the trigger itself"),
        "on_enter": "behavior.program event ID emitted when a matching entity's pivot enters (\"\" = none). The event may declare text fields `trigger` (zone entity ID) and/or `entity` (occupant ID), nothing else",
        "on_exit": "behavior.program event ID emitted when an occupant leaves or is disabled (\"\" = none); same payload rule",
        "once": "bool, default false: emit at most one enter and one exit event; occupancy is still tracked",
        "enabled": "bool, default true. A disabled zone (or a zone entity in an inactive scene) is frozen: no events, occupancy kept",
        "exit_margin": "meters 0..100, default 0: hysteresis, an occupant leaves only beyond the shape grown by this margin",
        "requires": "system trigger.detect (capability trigger); events also require behavior.execute",
        "order": "per step: zones in authored order; per zone exits then enters, occupants in authored order",
        "budget": format!("{MAX_TRIGGERS} zones per world, {MAX_TRIGGER_EVENTS_PER_STEP} events per step, {MAX_TRIGGER_PAIRS} candidate tests per step (sum over zones of matching entities)"),
        "observe": "observe metrics `trigger`: occupied zones with occupant IDs and the recent enter/exit events"
    })
}

/// World-level checks run by `World::compile` on resolved components: budget, required
/// system, and event payload contracts against `behavior.program`.
pub fn validate_world(world: &crate::World, zones: &[(&str, &TriggerZone)]) -> Result<()> {
    if zones.is_empty() {
        return Ok(());
    }
    if zones.len() > MAX_TRIGGERS {
        return Err(crate::Diagnostic::error(
            "trigger_budget",
            "/entities",
            format!(
                "trigger zone budget exceeded ({} > {MAX_TRIGGERS})",
                zones.len()
            ),
        ));
    }
    let invalid = |id: &str, message: String| {
        crate::Diagnostic::error(
            "invalid_trigger",
            format!(
                "/entities/@{}/components/trigger.zone",
                crate::local::escape(id)
            ),
            message,
        )
    };
    if !world.systems.iter().any(|s| s == "trigger.detect") {
        return Err(invalid(
            zones[0].0,
            "trigger.zone needs the trigger.detect system (add it to systems)".into(),
        ));
    }
    let program = world.resources.get("behavior.program");
    for (id, zone) in zones {
        for event in zone.events() {
            let fields = program
                .and_then(|p| p.pointer(&format!("/events/{}/fields", crate::local::escape(event))))
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    invalid(
                        id,
                        format!("trigger emits {event}, which behavior.program does not declare"),
                    )
                })?;
            if !fields
                .iter()
                .all(|(k, t)| (k == "trigger" || k == "entity") && t == "text")
            {
                return Err(invalid(
                    id,
                    format!(
                        "trigger event {event} may only declare text fields `trigger` and `entity`"
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(v: Value) -> Result<TriggerZone> {
        let z: TriggerZone = serde_json::from_value(v)?;
        z.validate()?;
        Ok(z)
    }
    fn err(v: Value) -> String {
        format!("{:#}", zone(v).unwrap_err())
    }

    #[test]
    fn valid_zones_and_defaults() {
        let s = zone(json!({"shape":"sphere","radius":1.5})).unwrap();
        assert!(s.enabled && !s.once && s.on_enter.is_empty() && s.exit_margin == 0.0);
        assert_eq!(s, TriggerZone::sphere(1.5));
        let b = zone(json!({"shape":"box","half_extents":[1,2,3],"filter":{"tags":["player"],"components":["core.health"]},"on_enter":"in","on_exit":"out","once":true,"enabled":false,"exit_margin":0.5})).unwrap();
        assert_eq!(b.events().collect::<Vec<_>>(), ["in", "out"]);
        assert_eq!(
            serde_json::to_value(TriggerZone::boxed([1.0; 3])).unwrap(),
            json!({"shape":"box","half_extents":[1.0,1.0,1.0],"filter":{"tags":[],"components":[]},"on_enter":"","on_exit":"","once":false,"enabled":true,"exit_margin":0.0})
        );
    }

    #[test]
    fn invalid_zones_are_rejected_with_reasons() {
        assert!(err(json!({"shape":"cone","radius":1})).contains("unknown variant"));
        assert!(err(json!({"shape":"sphere","radius":1,"size":2})).contains("unknown field"));
        assert!(err(json!({"shape":"sphere"})).contains("requires radius"));
        assert!(err(json!({"shape":"box"})).contains("requires half_extents"));
        assert!(
            err(json!({"shape":"box","half_extents":[1,1,1],"radius":1}))
                .contains("radius is only valid")
        );
        assert!(
            err(json!({"shape":"sphere","radius":1,"half_extents":[1,1,1]}))
                .contains("half_extents is only valid")
        );
        assert!(err(json!({"shape":"sphere","radius":0})).contains("radius"));
        assert!(err(json!({"shape":"box","half_extents":[1,-1,1]})).contains("half_extents"));
        assert!(err(json!({"shape":"sphere","radius":1,"exit_margin":-1})).contains("exit_margin"));
        assert!(
            err(json!({"shape":"sphere","radius":1,"on_enter":"bad event"})).contains("on_enter")
        );
        assert!(
            err(json!({"shape":"sphere","radius":1,"filter":{"tags":["a b"]}})).contains("filter")
        );
        assert!(err(json!({"shape":"sphere","radius":1,"filter":{"tags":["a","b","c","d","e","f","g","h","i"]}})).contains("filter allows"));
        assert!(
            err(json!({"shape":"sphere","radius":1,"filter":{"where":[]}}))
                .contains("unknown field")
        );
    }

    #[test]
    fn filter_uses_selection_and_semantics() {
        let f = TriggerFilter {
            tags: vec!["player".into(), "hero".into()],
            components: vec!["core.health".into()],
        };
        let tags = ["player".to_owned(), "hero".to_owned()];
        assert!(f.matches(&tags, |c| c == "core.health"));
        assert!(!f.matches(&tags[..1], |c| c == "core.health"));
        assert!(!f.matches(&tags, |_| false));
        assert!(TriggerFilter::default().matches(&[], |_| false));
    }

    #[test]
    fn interned_filter_agrees_with_the_string_filter() {
        let mut index = crate::tags::TagIndex::default();
        let rows: [&[&str]; 4] = [
            &["player", "hero"],
            &["player"],
            &[],
            &["hero", "x", "player"],
        ];
        for r in rows {
            index.push(&r.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        }
        let filters = [
            vec!["player".to_owned(), "hero".to_owned()],
            vec!["player".to_owned()],
            vec![],
            vec!["missing".to_owned()],
            vec!["player".to_owned(), "missing".to_owned()],
        ];
        for tags in filters {
            for health in [true, false] {
                let f = TriggerFilter {
                    tags: tags.clone(),
                    components: if health {
                        vec!["core.health".into()]
                    } else {
                        vec![]
                    },
                };
                let required = index.resolve(&f.tags);
                for (slot, r) in rows.iter().enumerate() {
                    let strings: Vec<String> = r.iter().map(|s| s.to_string()).collect();
                    let has = |c: &str| c == "core.health" && slot % 2 == 0;
                    assert_eq!(
                        f.matches_interned(required.as_ref(), index.set(slot), has),
                        f.matches(&strings, has),
                        "{tags:?} health={health} slot={slot}"
                    );
                }
            }
        }
    }
}
