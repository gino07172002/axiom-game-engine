//! Typed cross-package events, delivered at a fixed-step boundary.
use anyhow::{ensure, Result};
use axiom_core::{gameplay::Scalar, local::escape, Diagnostic, NativeEvent, World};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize)]
pub struct Envelope {
    pub seq: u64,
    pub tick: u64,
    pub event: String,
    pub version: u32,
    pub publisher: String,
    pub fields: BTreeMap<String, Scalar>,
}
#[derive(Clone, Default)]
pub struct Events {
    specs: BTreeMap<String, NativeEvent>,
    routes: BTreeMap<String, BTreeSet<String>>,
    pub pending: Vec<Envelope>,
    recent: VecDeque<Envelope>,
    sequence: u64,
    delivered: u64,
    pub fault: Option<Value>,
}
impl Events {
    pub fn new(world: &World) -> Self {
        let mut events = Self::default();
        if let Some(lock) = &world.packages {
            for package in lock.packages.values() {
                events.specs.extend(package.exports.events.clone());
                for (id, import) in &package.imports.events {
                    events
                        .routes
                        .entry(id.clone())
                        .or_default()
                        .insert(import.consumer.clone());
                }
            }
        }
        events
    }
    /// The last delivered envelopes (at most 64), oldest first.
    pub fn recent(&self) -> impl DoubleEndedIterator<Item = &Envelope> {
        self.recent.iter()
    }
    pub fn owns(&self, event: &str) -> bool {
        self.specs.contains_key(event)
    }
    pub fn publish(
        &mut self,
        publisher: &str,
        event: &str,
        tick: u64,
        fields: BTreeMap<String, Scalar>,
    ) {
        // Worlds without this interface retain their original execution path.
        let Some(spec) = self.specs.get(event) else {
            return;
        };
        if self.fault.is_some() {
            return;
        }
        let result = (|| -> Result<()> {
            ensure!(
                spec.publisher == publisher,
                "event publisher does not own this contract"
            );
            ensure!(
                fields.len() == spec.fields.len(),
                "event field count differs from contract"
            );
            for (id, ty) in &spec.fields {
                let value = fields.get(id).ok_or_else(|| {
                    Diagnostic::error(
                        "event_payload",
                        format!("/events/@{}/fields/{}", escape(event), escape(id)),
                        "field missing",
                    )
                })?;
                value.validate()?;
                ensure!(
                    value.ty() == *ty,
                    "event payload type differs from contract"
                );
            }
            ensure!(
                self.pending.len() < 128,
                "cross-package event queue exceeds 128 per tick"
            );
            Ok(())
        })();
        if let Err(error) = result {
            self.fault = Some(axiom_core::error_response(&Diagnostic::error(
                "event_contract",
                format!("/events/@{}", escape(event)),
                error.to_string(),
            )));
            return;
        }
        self.sequence += 1;
        self.pending.push(Envelope {
            seq: self.sequence,
            tick,
            event: event.into(),
            publisher: publisher.into(),
            version: spec.version,
            fields,
        });
    }
    pub fn for_consumer(&self, consumer: &str) -> Vec<Envelope> {
        self.pending
            .iter()
            .filter(|e| {
                self.routes
                    .get(&e.event)
                    .is_some_and(|r| r.contains(consumer))
            })
            .cloned()
            .collect()
    }
    pub fn acknowledge(&mut self) {
        for event in self.pending.drain(..) {
            self.delivered += 1;
            self.recent.push_back(event);
            if self.recent.len() > 64 {
                self.recent.pop_front();
            }
        }
    }
    pub fn transfer(&mut self, old: &Self) -> Result<()> {
        ensure!(old.pending.is_empty() || (self.specs==old.specs && self.routes==old.routes),
            "pending events require the same contract graph; repair the consumer before changing contracts");
        self.sequence = old.sequence;
        self.delivered = old.delivered;
        self.recent = old.recent.clone();
        // An undelivered batch may only move into the exact same contract graph.
        if self.specs == old.specs && self.routes == old.routes {
            self.pending = old.pending.clone();
        }
        Ok(())
    }
    pub fn observation(&self) -> Value {
        json!({"published":self.sequence,"delivered":self.delivered,"pending":self.pending.len(),
            "error":self.fault,"recent":self.recent.iter().rev().take(12).collect::<Vec<_>>()})
    }
}
