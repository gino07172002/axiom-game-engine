//! Interactables (`interact.target`): something an interactor (by default an entity tagged
//! `player`) can use when it stands close enough — talk to a person, open a door. The runtime
//! system `interact.focus` picks at most one focused target per interactor; the authored input
//! button (an `input.bindings` button name) or the `interact.use` action then emits the
//! target's `behavior.program` event, so rules and `runtime.bindings` run without Rust.
//!
//! Distances are between world pivots (parents included). An interactor faces along its
//! world +Z axis (the engine's model convention, as `fsm.agent` and `path.follow` orient).
use crate::trigger::{TriggerFilter, MAX_FILTER_TERMS};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// `interact.target` components per world (enabled or not).
pub const MAX_INTERACTABLES: usize = 256;
/// Interactor x target tests per step: the sum over targets of the entities their
/// interactor filter matches. Measured at this bound in release; see docs/interaction.zh-TW.md.
pub const MAX_INTERACT_PAIRS: usize = 16 * 1024;
/// Prompt text bytes (a localisation key or literal text).
pub const MAX_PROMPT_BYTES: usize = 256;
/// Prompt shown when a target does not author one (also looked up in the UI catalog).
pub const DEFAULT_PROMPT: &str = "Interact";
/// Default `action` and the `behavior.program` event the editor / ACL op declares.
pub const DEFAULT_ACTION: &str = "interact";
pub const DEFAULT_EVENT: &str = "interacted";

fn yes() -> bool {
    true
}
fn default_range() -> f32 {
    2.0
}
fn full_circle() -> f32 {
    360.0
}
fn default_action() -> String {
    DEFAULT_ACTION.into()
}
fn players() -> TriggerFilter {
    TriggerFilter {
        tags: vec!["player".into()],
        components: vec![],
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InteractTarget {
    /// Meters between the interactor's and the target's pivots.
    #[serde(default = "default_range")]
    pub range: f32,
    /// Hysteresis: the focused target stays focused until farther than range + margin.
    #[serde(default)]
    pub exit_margin: f32,
    /// Degrees: the target must lie within ±view_angle/2 of the interactor's facing (XZ);
    /// 360 = no facing requirement.
    #[serde(default = "full_circle")]
    pub view_angle: f32,
    /// Localisation key, or literal text when the UI catalog has no such key ("" = default).
    #[serde(default)]
    pub prompt: String,
    /// `input.bindings` button that uses the focused target ("" = only `interact.use`).
    #[serde(default = "default_action")]
    pub action: String,
    /// `behavior.program` event emitted on use ("" = none).
    #[serde(default)]
    pub event: String,
    /// Seconds after a use before the target can be focused or used again.
    #[serde(default)]
    pub cooldown: f32,
    /// Usable once per runtime (across all interactors).
    #[serde(default)]
    pub once: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Higher wins when several targets are in range (then nearer, then more in front).
    #[serde(default)]
    pub priority: i32,
    /// Which entities can interact: same AND semantics as `trigger.zone` filters and
    /// `entities[tag=..,has=..]`. Default: tag `player`.
    #[serde(default = "players")]
    pub interactor: TriggerFilter,
}

impl Default for InteractTarget {
    fn default() -> Self {
        Self {
            range: default_range(),
            exit_margin: 0.0,
            view_angle: full_circle(),
            prompt: String::new(),
            action: default_action(),
            event: String::new(),
            cooldown: 0.0,
            once: false,
            enabled: true,
            priority: 0,
            interactor: players(),
        }
    }
}

impl InteractTarget {
    pub fn validate(&self) -> Result<()> {
        let within = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        ensure!(
            within(self.range, 0.1, 100.0),
            "range must be 0.1..100 meters"
        );
        ensure!(
            within(self.exit_margin, 0.0, 100.0),
            "exit_margin must be 0..100 meters"
        );
        ensure!(
            within(self.view_angle, 1.0, 360.0),
            "view_angle must be 1..360 degrees (360 = any facing)"
        );
        ensure!(
            self.prompt.len() <= MAX_PROMPT_BYTES && !self.prompt.chars().any(char::is_control),
            "prompt must be at most {MAX_PROMPT_BYTES} bytes without control characters"
        );
        if !self.action.is_empty() {
            crate::valid_id(&self.action).context("action")?;
        }
        if !self.event.is_empty() {
            crate::valid_id(&self.event).context("event")?;
        }
        ensure!(
            within(self.cooldown, 0.0, 3600.0),
            "cooldown must be 0..3600 seconds"
        );
        ensure!(
            (-1000..=1000).contains(&self.priority),
            "priority must be -1000..1000"
        );
        ensure!(
            self.interactor.tags.len() <= MAX_FILTER_TERMS
                && self.interactor.components.len() <= MAX_FILTER_TERMS,
            "interactor allows up to {MAX_FILTER_TERMS} tags and {MAX_FILTER_TERMS} components"
        );
        for id in self
            .interactor
            .tags
            .iter()
            .chain(&self.interactor.components)
        {
            crate::valid_id(id).context("interactor")?;
        }
        Ok(())
    }
    /// The prompt to show: the authored one, or [`DEFAULT_PROMPT`].
    pub fn prompt_text(&self) -> &str {
        if self.prompt.is_empty() {
            DEFAULT_PROMPT
        } else {
            &self.prompt
        }
    }
}

pub fn target_schema() -> Value {
    json!({
        "range": "meters 0.1..100, default 2: distance between the interactor's and the target's world pivots",
        "exit_margin": "meters 0..100, default 0: the focused target stays focused until farther than range + exit_margin",
        "view_angle": "degrees 1..360, default 360 (any): the target must lie within +-view_angle/2 of the interactor's facing (world +Z, XZ plane)",
        "prompt": format!("localisation key or literal text (<= {MAX_PROMPT_BYTES} bytes); \"\" = \"{DEFAULT_PROMPT}\". ui.document text binding {{kind:prompt}} shows \"[KEY] text\" (or text with {{key}} replaced) for the local interactor's focus"),
        "action": format!("input.bindings button name that uses the focused target, default \"{DEFAULT_ACTION}\"; \"\" = only the interact.use action. Keys are never hard-coded: the binding names the key"),
        "event": "behavior.program event emitted on use (\"\" = none). It may declare text fields `interactor` and/or `target`, nothing else",
        "cooldown": "seconds 0..3600, default 0: after a use the target cannot be focused or used for this long",
        "once": "bool, default false: usable once per runtime (all interactors together)",
        "enabled": "bool, default true. Disabled targets (or targets in an inactive scene) are never focused",
        "priority": "integer -1000..1000, default 0. Focus: highest priority, then nearest, then most in front, then authored order",
        "interactor": format!("{{tags:[..], components:[..]}} up to {MAX_FILTER_TERMS} each, default {{tags:[player]}}; an entity needs every tag and component (like entities[tag=..,has=..]); never the target itself; dead entities (core.health 0) never interact"),
        "requires": "system interact.focus (capability interact); events need behavior.execute; action must be an input.bindings button",
        "input": "the bound button (rising edge) uses the focus of the local interactor = the first enabled, living entity (authored order) matching any target's interactor filter, as shown by the prompt in the previous step",
        "budget": format!("{MAX_INTERACTABLES} targets per world, {MAX_INTERACT_PAIRS} interactor x target tests per step"),
        "observe": "observe metrics `interact`: focus per interactor and recent uses; actions interact.use / interact.inspect / interact.summary"
    })
}

/// World-level checks run by `World::compile` on resolved components: budget, required
/// systems, input binding names and event payload contracts against `behavior.program`.
pub fn validate_world(world: &crate::World, targets: &[(&str, &InteractTarget)]) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    if targets.len() > MAX_INTERACTABLES {
        return Err(crate::Diagnostic::error(
            "interact_budget",
            "/entities",
            format!(
                "interactable budget exceeded ({} > {MAX_INTERACTABLES})",
                targets.len()
            ),
        ));
    }
    let invalid = |id: &str, field: &str, message: String| {
        crate::Diagnostic::error(
            "invalid_interact",
            format!(
                "/entities/@{}/components/interact.target{field}",
                crate::local::escape(id)
            ),
            message,
        )
    };
    let has_system = |s: &str| world.systems.iter().any(|x| x == s);
    if !has_system("interact.focus") {
        return Err(invalid(
            targets[0].0,
            "",
            "interact.target needs the interact.focus system (add it to systems)".into(),
        ));
    }
    let buttons: crate::InputBindings = world
        .resources
        .get("input.bindings")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?
        .unwrap_or_default();
    let program = world.resources.get("behavior.program");
    for (id, target) in targets {
        if !target.action.is_empty() && !buttons.buttons.contains_key(&target.action) {
            return Err(invalid(
                id,
                "/action",
                format!(
                    "input.bindings has no button {}; bind a key to it or use an existing button name",
                    target.action
                ),
            ));
        }
        if target.event.is_empty() {
            continue;
        }
        if !has_system("behavior.execute") {
            return Err(invalid(
                id,
                "/event",
                "interaction events need the behavior.execute system".into(),
            ));
        }
        let fields = program
            .and_then(|p| {
                p.pointer(&format!(
                    "/events/{}/fields",
                    crate::local::escape(&target.event)
                ))
            })
            .and_then(Value::as_object)
            .ok_or_else(|| {
                invalid(
                    id,
                    "/event",
                    format!(
                        "target emits {}, which behavior.program does not declare",
                        target.event
                    ),
                )
            })?;
        if !fields
            .iter()
            .all(|(k, t)| (k == "interactor" || k == "target") && t == "text")
        {
            return Err(invalid(
                id,
                "/event",
                format!(
                    "interaction event {} may only declare text fields `interactor` and `target`",
                    target.event
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(v: Value) -> Result<InteractTarget> {
        let t: InteractTarget = serde_json::from_value(v)?;
        t.validate()?;
        Ok(t)
    }
    fn err(v: Value) -> String {
        format!("{:#}", target(v).unwrap_err())
    }

    #[test]
    fn defaults_and_round_trip() {
        let t = target(json!({})).unwrap();
        assert_eq!(t, InteractTarget::default());
        assert_eq!(t.range, 2.0);
        assert_eq!(t.view_angle, 360.0);
        assert_eq!(t.action, "interact");
        assert_eq!(t.interactor.tags, ["player"]);
        assert!(t.enabled && !t.once && t.event.is_empty());
        assert_eq!(t.prompt_text(), DEFAULT_PROMPT);
        let full = json!({"range":2.4,"exit_margin":0.8,"view_angle":90.0,"prompt":"talk_lumi","action":"use","event":"talk","cooldown":1.5,"once":true,"enabled":false,"priority":3,"interactor":{"tags":["hero"],"components":["core.health"]}});
        let t = target(full.clone()).unwrap();
        assert_eq!(t.prompt_text(), "talk_lumi");
        let back = serde_json::to_value(&t).unwrap();
        assert_eq!(
            serde_json::from_value::<InteractTarget>(back).unwrap(),
            t,
            "serialization round-trips"
        );
        // An explicitly empty filter means "any entity" (not the default players).
        let any = target(json!({"interactor":{}})).unwrap();
        assert!(any.interactor.tags.is_empty());
    }

    #[test]
    fn invalid_targets_are_rejected_with_reasons() {
        assert!(err(json!({"range":0})).contains("range"));
        assert!(err(json!({"range":101})).contains("range"));
        assert!(err(json!({"exit_margin":-1})).contains("exit_margin"));
        assert!(err(json!({"view_angle":0})).contains("view_angle"));
        assert!(err(json!({"view_angle":361})).contains("view_angle"));
        assert!(err(json!({"cooldown":-1})).contains("cooldown"));
        assert!(err(json!({"priority":5000})).contains("priority"));
        assert!(err(json!({"action":"bad key"})).contains("action"));
        assert!(err(json!({"event":"bad event"})).contains("event"));
        assert!(err(json!({"prompt":"a\nb"})).contains("prompt"));
        assert!(err(json!({"prompt":"x".repeat(MAX_PROMPT_BYTES + 1)})).contains("prompt"));
        assert!(err(json!({"interactor":{"tags":["a b"]}})).contains("interactor"));
        assert!(err(json!({"key":"KeyE"})).contains("unknown field"));
        assert!(err(json!({"interactor":{"where":[]}})).contains("unknown field"));
        // Non-ASCII prompts are fine (literal text or a catalog key).
        assert!(target(json!({"prompt":"和露米說話"})).is_ok());
    }
}
