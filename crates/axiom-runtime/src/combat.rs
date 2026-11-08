//! Shared collision snapshots and one damage commit per fixed tick.
use crate::world::*;
use anyhow::{ensure, Context, Result};
use axiom_combat::{Body, Config, Hit, Outcome, Receipt, Scene};
use axiom_core::{
    gameplay::{Scalar, ScalarType},
    *,
};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
};

#[derive(Clone)]
pub struct Combat {
    pub scene: Scene,
    pub pending: Vec<Hit>,
    pub receipts: Vec<Receipt>,
    history: VecDeque<Value>,
    hits: u64,
    deaths: u64,
    evaded: u64,
    rejected: u64,
    sequence: u64,
}
impl Combat {
    pub fn new(world: &World) -> Result<Option<Self>> {
        world
            .resources
            .get("combat.session")
            .map(|v| {
                Ok(Self {
                    scene: Scene::new(serde_json::from_value(v.clone())?),
                    pending: vec![],
                    receipts: vec![],
                    history: VecDeque::new(),
                    hits: 0,
                    deaths: 0,
                    evaded: 0,
                    rejected: 0,
                    sequence: 0,
                })
            })
            .transpose()
    }
    pub fn snapshot(
        &mut self,
        mut body: Body,
        state_ids: &[RuntimeEntityId],
        indices: &BTreeMap<String, usize>,
    ) {
        if let Some(&i) = indices.get(&body.handle.id) {
            body.handle.generation = state_ids[i].generation;
            self.scene.bodies.insert(body.handle.id.clone(), body);
        }
    }
    pub fn submit(&mut self, hits: Vec<Hit>) -> Result<()> {
        ensure!(
            self.pending.len() + hits.len() <= 4096,
            "combat hit budget exceeded (4096 per fixed tick)"
        );
        self.pending.extend(hits);
        Ok(())
    }
    pub fn observation(&self) -> Value {
        json!({"bodies":self.scene.bodies.len(),"alive":self.scene.bodies.values().filter(|b|b.alive()).count(),
            "hits":self.hits,"deaths":self.deaths,"evaded":self.evaded,"rejected":self.rejected,
            "pending":self.pending.len(),"sequence":self.sequence,"recent":self.history.iter().rev().take(12).collect::<Vec<_>>()})
    }
    pub fn inherit(&mut self, old: &Self) {
        self.history = old.history.clone();
        self.sequence = old.sequence;
        self.hits = old.hits;
        self.deaths = old.deaths;
        self.evaded = old.evaded;
        self.rejected = old.rejected;
    }
}
pub fn snapshot(state: &mut RuntimeState, bodies: Vec<Body>) {
    if let Some(combat) = &mut state.combat {
        for mut b in bodies {
            if let Some(&slot) = state.compiled.indices.get(&b.handle.id) {
                if !state.compiled.enabled[slot] && b.alive() {
                    continue;
                }
                if let Some(h) = &state.health[slot] {
                    b.max_health = h.max;
                }
            }
            combat.snapshot(b, &state.ids, &state.compiled.indices);
        }
    }
}
pub fn resolve(state: &mut RuntimeState) {
    let Some(combat) = &mut state.combat else {
        return;
    };
    combat.receipts = combat.scene.resolve(std::mem::take(&mut combat.pending));
    for receipt in &combat.receipts {
        combat.sequence += 1;
        match receipt.outcome {
            Outcome::Hit => {
                combat.hits += 1;
                state.events.publish(
                    "combat.resolve",
                    "combat.hit",
                    state.tick,
                    [
                        ("source".into(), Scalar::Text(receipt.source.clone())),
                        ("target".into(), Scalar::Text(receipt.target.clone())),
                        ("amount".into(), Scalar::Number(f64::from(receipt.amount))),
                    ]
                    .into(),
                );
                if receipt.killed {
                    combat.deaths += 1;
                    let faction = combat.scene.bodies[&receipt.target].faction;
                    state.events.publish(
                        "combat.resolve",
                        "combat.defeated",
                        state.tick,
                        [
                            ("entity".into(), Scalar::Text(receipt.target.clone())),
                            ("source".into(), Scalar::Text(receipt.source.clone())),
                            ("faction".into(), Scalar::Int(i64::from(faction))),
                        ]
                        .into(),
                    );
                }
            }
            Outcome::Evaded => combat.evaded += 1,
            Outcome::Rejected => combat.rejected += 1,
        }
        combat
            .history
            .push_back(json!({"seq":combat.sequence,"tick":state.tick,"receipt":receipt}));
        if combat.history.len() > 64 {
            combat.history.pop_front();
        }
    }
}
pub fn submit(state: &mut RuntimeState, hits: Vec<Hit>) {
    if let Some(combat) = &mut state.combat {
        if let Err(error) = combat.submit(hits) {
            state.events.fault = Some(error_response(&error));
        }
    }
}
struct CombatSystem;
impl RuntimeSystem for CombatSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, _state: &mut RuntimeState, _input: &ActionMap) {}
    fn command(
        &mut self,
        state: &mut RuntimeState,
        cap: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if cap != "combat" {
            return None;
        }
        Some((|| {
            ensure!(action == "inspect", "unknown combat action");
            let ids: Vec<String> = serde_json::from_value(args["ids"].clone())?;
            ensure!(ids.len() <= 128, "combat inspection budget exceeded");
            let combat = state.combat.as_ref().context("combat not active")?;
            let bodies = ids
                .iter()
                .map(|id| {
                    combat
                        .scene
                        .bodies
                        .get(id)
                        .with_context(|| format!("combat body missing: {id}"))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(json!({"tick":state.tick,"bodies":bodies}))
        })())
    }
}
pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.capabilities.insert("combat".into(), 1);
    r.authoring.resource("combat.session","combat",json!({"bounds":"{min:[x,z],max:[x,z]}","obstacles":"up to 2048 axis-aligned rectangles", "policy":"solid discs; hostile factions only; shared XZ world space"}),|v|serde_json::from_value::<Config>(v.clone())?.validate());
    r.authoring.systems.insert(
        "combat.resolve".into(),
        SystemDescriptor {
            id: "combat.resolve".into(),
            capability: "combat".into(),
            phase: 90,
            reads: vec!["combat.session".into()],
            writes: vec!["core.health".into()],
            after: vec!["rts.simulate".into(), "action.simulate".into()],
        },
    );
    for (id, fields) in [
        (
            "combat.hit",
            BTreeMap::from([
                ("source".into(), ScalarType::Text),
                ("target".into(), ScalarType::Text),
                ("amount".into(), ScalarType::Number),
            ]),
        ),
        (
            "combat.defeated",
            BTreeMap::from([
                ("entity".into(), ScalarType::Text),
                ("source".into(), ScalarType::Text),
                ("faction".into(), ScalarType::Int),
            ]),
        ),
    ] {
        r.authoring.native_events.insert(
            id.into(),
            NativeEvent {
                version: 1,
                publisher: "combat.resolve".into(),
                fields,
            },
        );
    }
    r.authoring.action("combat.inspect","combat",json!({"ids":"up to 128 stable body IDs; returns generation, owner, position, faction, health, invulnerability"}),|v| {
        ensure!(v.as_object().is_some_and(|o|o.len()==1)&&v["ids"].as_array().is_some_and(|a|a.len()<=128&&a.iter().all(Value::is_string)),"invalid combat inspection"); Ok(())
    });
    r.authoring.topics.insert("combat".into(),json!({"resource":"combat.session","system":"combat.resolve","collision":"XZ discs, shared rectangles, substepped controller movement","damage":"one commit after controller movement; friendly fire off; dodge windows apply to both controllers","events":["combat.hit","combat.defeated"],"observe":"/combat","inspect":{"kind":"call","capability":"combat","action":"inspect","args":{"ids":["hero","enemy/guard"]}}}));
    r.authoring.world_validators.push(|w| {
        let active = w.resources.contains_key("combat.session");
        ensure!(
            active == w.systems.iter().any(|s| s == "combat.resolve"),
            "combat.session and combat.resolve must be enabled together"
        );
        if !active {
            return Ok(());
        }
        let config: Config = w.resource("combat.session")?;
        config.validate()?;
        for e in &w.entities {
            let c = w.resolved_components(e);
            let owners = ["rts.unit", "rts.building", "action.actor"]
                .iter()
                .filter(|id| c.contains_key(**id))
                .count();
            ensure!(owners <= 1, "competing combat owners for {}", e.id);
            if owners == 0 {
                continue;
            }
            ensure!(
                !w.relations
                    .iter()
                    .any(|r| r.kind == "parent" && r.source == e.id),
                "shared combat bodies require world-space root transforms: {}",
                e.id
            );
        }
        Ok(())
    });
    r.factories
        .insert("combat.resolve".into(), |_| Ok(Box::new(CombatSystem)));
}
