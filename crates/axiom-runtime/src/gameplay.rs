//! Deterministic bounded event/rule/narrative executor, shared by UI and test commands.
use anyhow::{bail, ensure, Context, Result};
use axiom_core::{gameplay::*, World};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PendingEvent {
    pub due: u64,
    pub event: String,
    pub fields: BTreeMap<String, Scalar>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub variables: BTreeMap<String, Scalar>,
    pub node: Option<String>,
    pub ending: Option<String>,
    pub active_scenes: Vec<String>,
    pub pending: Vec<PendingEvent>,
    pub locale: Option<String>,
    pub held_inputs: Vec<String>,
    /// Capability calls of `call` actions (absent when none ran: older saves load).
    #[serde(default, skip_serializing_if = "CallStats::is_empty")]
    pub calls: CallStats,
}
/// Outcome of the capability calls `call` actions queued (observe `calls`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CallStats {
    pub run: u64,
    pub failed: u64,
    /// The last failure (calls are recoverable: the simulation continues).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<String>,
}
impl CallStats {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Trace {
    pub seq: u64,
    pub tick: u64,
    pub kind: String,
    pub id: String,
}
/// The focused interactable's prompt, published by `interact.focus` for the `prompt` text
/// binding. Shared strings: publishing never copies text.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    /// Key label of the target's input button ("E"); empty when it has no button.
    pub key: std::sync::Arc<str>,
    /// Localisation key or literal text.
    pub text: std::sync::Arc<str>,
}
#[derive(Clone)]
pub struct Gameplay {
    trace_sequence: u64,
    entry_depth: usize,
    pub images: BTreeMap<String, std::sync::Arc<crate::bound_ui::PreparedImage>>,
    pub compiled: std::sync::Arc<CompiledGameplay>,
    pub values: Vec<Scalar>,
    pub node: Option<usize>,
    pub ending: Option<String>,
    pub active: Vec<bool>,
    pub pending: VecDeque<PendingEvent>,
    pub locale: Option<String>,
    pub held: Vec<String>,
    pub trace: VecDeque<Trace>,
    pub fault: Option<Value>,
    /// Derived each step by `interact.focus`; not part of saves.
    pub prompt: Option<Prompt>,
    /// Capability calls queued by `call` actions, dispatched by the runtime right after the
    /// step or command that queued them (always empty between steps; not part of saves).
    pub calls: Vec<RuntimeCommand>,
    /// Animation reads (`CompiledGameplay::animation_reads`), published by
    /// `animation.evaluate` each step; derived, not part of saves.
    pub animation: Vec<Scalar>,
    pub call_stats: CallStats,
}
struct Budget(usize);
impl Budget {
    fn spend(&mut self) -> Result<()> {
        ensure!(self.0 < MAX_OPS, "gameplay instruction budget exceeded");
        self.0 += 1;
        Ok(())
    }
}
impl Gameplay {
    pub fn new(w: &World) -> Result<Self> {
        let compiled = CompiledGameplay::compile(w)?;
        let mut g = Self {
            trace_sequence: 0,
            entry_depth: 0,
            images: BTreeMap::new(),
            values: compiled.initial.clone(),
            node: None,
            ending: None,
            active: w.scenes.iter().map(|s| s.active).collect(),
            pending: VecDeque::new(),
            locale: compiled.ui.as_ref().map(|u| u.locale.clone()),
            held: vec![],
            trace: VecDeque::new(),
            fault: None,
            prompt: None,
            calls: vec![],
            call_stats: CallStats::default(),
            animation: compiled
                .animation_reads
                .iter()
                .map(AnimationRead::initial)
                .collect(),
            compiled: std::sync::Arc::new(compiled),
        };
        if let Some(n) = g.compiled.start {
            let mut budget = Budget(0);
            g.enter(n, 0, &mut budget)?;
            g.drain(0, &mut budget)?;
        }
        Ok(g)
    }
    fn record(&mut self, tick: u64, kind: &str, id: &str) {
        self.trace_sequence = self.trace_sequence.saturating_add(1);
        if self.trace.len() == 128 {
            self.trace.pop_front();
        }
        self.trace.push_back(Trace {
            seq: self.trace_sequence,
            tick,
            kind: kind.into(),
            id: id.into(),
        });
    }
    /// Borrowed, so conditions evaluate without copying text values.
    fn read<'a>(&'a self, r: &'a Read, payload: &'a [Scalar]) -> &'a Scalar {
        match r {
            Read::Literal(v) => v,
            Read::Variable(i) => &self.values[*i],
            Read::Event(i) => &payload[*i],
            Read::Animation(i) => &self.animation[*i],
        }
    }
    fn test_budget(&self, t: &Test, p: &[Scalar], b: &mut Budget) -> Result<bool> {
        b.spend()?;
        Ok(match t {
            Test::Always => true,
            Test::Eq(a, c) => self.read(a, p) == self.read(c, p),
            Test::Ge(a, c) => match (self.read(a, p), self.read(c, p)) {
                (Scalar::Int(a), Scalar::Int(c)) => a >= c,
                (Scalar::Number(a), Scalar::Number(c)) => a >= c,
                _ => false,
            },
            Test::Not(t) => !self.test_budget(t, p, b)?,
            Test::All(ts) => {
                for t in ts {
                    if !self.test_budget(t, p, b)? {
                        return Ok(false);
                    }
                }
                true
            }
        })
    }
    pub fn test(&self, t: &Test) -> bool {
        self.test_budget(t, &[], &mut Budget(0)).unwrap_or(false)
    }
    fn ops(&mut self, ops: &[Op], payload: &[Scalar], tick: u64, b: &mut Budget) -> Result<()> {
        for op in ops {
            b.spend()?;
            match op {
                Op::Set(i, r) => self.values[*i] = self.read(r, payload).clone(),
                Op::Add(i, r) => {
                    self.values[*i] = match (&self.values[*i], self.read(r, payload)) {
                        (Scalar::Int(a), Scalar::Int(c)) => {
                            Scalar::Int(a.checked_add(*c).context("integer overflow")?)
                        }
                        (Scalar::Number(a), Scalar::Number(c)) => {
                            ensure!((a + c).is_finite(), "number overflow");
                            Scalar::Number(a + c)
                        }
                        _ => bail!("invalid compiled add"),
                    };
                }
                Op::Emit(event, values, delay) => {
                    let fields = self.compiled.event_fields[*event]
                        .iter()
                        .zip(values)
                        .map(|((id, _), r)| (id.clone(), self.read(r, payload).clone()))
                        .collect();
                    self.enqueue(PendingEvent {
                        due: tick.checked_add(*delay as u64).context("tick overflow")?,
                        event: self.compiled.event_ids[*event].clone(),
                        fields,
                    })?;
                }
                Op::GoTo(n) => self.enter(*n, tick, b)?,
                Op::Scene(s) => {
                    self.active.fill(false);
                    self.active[*s] = true;
                    let id = self.compiled.scenes[*s].clone();
                    self.record(tick, "scene", &id);
                }
                Op::End(e) => {
                    self.ending = Some(e.clone());
                    self.record(tick, "ending", e);
                }
                Op::Call(call) => {
                    let (capability, action, args) = &**call;
                    ensure!(
                        self.calls.len() < MAX_QUEUED_CALLS,
                        "capability call budget exceeded ({MAX_QUEUED_CALLS} queued)"
                    );
                    self.calls.push(RuntimeCommand::Call {
                        capability: capability.clone(),
                        action: action.clone(),
                        args: args.clone(),
                    });
                    self.record(tick, "call", &format!("{capability}.{action}"));
                }
            }
        }
        Ok(())
    }
    fn enter(&mut self, n: usize, tick: u64, b: &mut Budget) -> Result<()> {
        b.spend()?;
        ensure!(
            self.entry_depth < 16,
            "narrative entry depth budget exceeded"
        );
        self.node = Some(n);
        self.ending = None;
        let node = self.compiled.nodes[n].clone();
        self.record(tick, "node", &node.id);
        self.entry_depth += 1;
        let result = self.ops(&node.enter, &[], tick, b);
        self.entry_depth -= 1;
        result?;
        if self.node == Some(n) {
            if let Some(e) = node.ending {
                self.ending = Some(e.clone());
                self.record(tick, "ending", &e);
            }
        }
        Ok(())
    }
    fn enqueue(&mut self, e: PendingEvent) -> Result<()> {
        ensure!(
            self.pending.len() < MAX_EVENTS,
            "event queue budget exceeded"
        );
        self.pending.push_back(e);
        Ok(())
    }
    fn drain(&mut self, tick: u64, b: &mut Budget) -> Result<()> {
        while let Some(i) = self.pending.iter().position(|e| e.due <= tick) {
            b.spend()?;
            let e = self.pending.remove(i).unwrap();
            let event = self
                .compiled
                .event_ids
                .iter()
                .position(|id| id == &e.event)
                .context("missing queued event")?;
            self.record(tick, "event", &e.event);
            let payload = self.compiled.event_fields[event]
                .iter()
                .map(|(id, _)| e.fields[id].clone())
                .collect::<Vec<_>>();
            for rule in self.compiled.rules[event].clone() {
                if self.test_budget(&rule.when, &payload, b)? {
                    self.record(tick, "rule", &rule.id);
                    self.ops(&rule.ops, &payload, tick, b)?;
                }
            }
        }
        Ok(())
    }
    pub fn command(&mut self, command: &RuntimeCommand, tick: u64) -> Result<()> {
        let mut next = self.clone();
        let mut b = Budget(0);
        next.apply(command, tick, &mut b)?;
        next.drain(tick, &mut b)?;
        next.fault = None;
        *self = next;
        Ok(())
    }
    /// The slider `widget` of the document.
    pub fn slider(&self, widget: &str) -> Result<&BoundSlider> {
        self.compiled
            .widgets
            .iter()
            .find_map(|w| match w {
                BoundWidget::Slider(s) if s.id == widget => Some(&**s),
                _ => None,
            })
            .with_context(|| format!("unknown slider {widget}"))
    }
    /// The value slider `s` shows: its variable, else `fallback` (the UI's last value).
    pub fn slider_value(&self, s: &BoundSlider, fallback: Option<f64>) -> f64 {
        match s.variable {
            Some(i) => crate::ui_effects::number(&self.values[i]),
            None => fallback.unwrap_or(s.value),
        }
    }
    fn apply(&mut self, command: &RuntimeCommand, tick: u64, b: &mut Budget) -> Result<()> {
        let next = self;
        match command {
            RuntimeCommand::Call { .. } => {
                anyhow::bail!("native capability calls must be dispatched by WorldRuntime")
            }
            RuntimeCommand::Batch { commands } => {
                ensure!(
                    (1..=MAX_BATCH).contains(&commands.len()),
                    "a batch holds 1..{MAX_BATCH} commands"
                );
                for (i, c) in commands.iter().enumerate() {
                    ensure!(
                        !matches!(
                            c,
                            RuntimeCommand::Batch { .. } | RuntimeCommand::Slide { .. }
                        ),
                        "a batch cannot hold batch or slide commands"
                    );
                    axiom_core::local::at(&format!("/commands/{i}"), next.apply(c, tick, b))?;
                }
            }
            RuntimeCommand::Slide { widget, value } => {
                let s = next.slider(widget)?.clone();
                ensure!(
                    next.test_budget(&s.when, &[], b)?,
                    "slider {widget} is hidden (its when condition is false)"
                );
                let v = s.snap(*value)?;
                if let Some(i) = s.variable {
                    next.values[i] = if s.integer {
                        Scalar::Int(v as i64)
                    } else {
                        Scalar::Number(v)
                    };
                }
                next.record(tick, "slide", widget);
            }
            RuntimeCommand::Choose { node, choice } => {
                let n = next.node.context("no active narrative")?;
                ensure!(
                    next.compiled.nodes[n].id == *node && next.ending.is_none(),
                    "stale narrative command"
                );
                let c = next.compiled.nodes[n]
                    .choices
                    .iter()
                    .find(|c| &c.id == choice)
                    .context("unknown choice")?
                    .clone();
                ensure!(
                    next.test_budget(&c.when, &[], b)?,
                    "choice condition is false"
                );
                next.record(tick, "choice", choice);
                next.ops(&c.ops, &[], tick, b)?;
                next.enter(c.next, tick, b)?;
            }
            RuntimeCommand::Emit { event, fields } => {
                let schema = next
                    .compiled
                    .program
                    .events
                    .get(event)
                    .context("unknown event")?;
                ensure!(
                    fields.len() == schema.fields.len(),
                    "event payload field count mismatch"
                );
                for (id, ty) in &schema.fields {
                    let v = fields.get(id).context("missing event field")?;
                    v.validate()?;
                    ensure!(v.ty() == *ty, "event payload type mismatch");
                }
                next.enqueue(PendingEvent {
                    due: tick,
                    event: event.clone(),
                    fields: fields.clone(),
                })?;
            }
            RuntimeCommand::Locale { locale } => {
                ensure!(
                    next.compiled
                        .ui
                        .as_ref()
                        .is_some_and(|u| u.catalogs.contains_key(locale)),
                    "unknown locale"
                );
                next.locale = Some(locale.clone());
            }
        }
        Ok(())
    }
    pub fn step(&mut self, input: &crate::ActionMap, tick: u64) {
        if self.fault.is_some() {
            return;
        }
        let result = (|| -> Result<Self> {
            let mut next = self.clone();
            for (action, event) in next.compiled.inputs.clone() {
                if input.pressed(&action) && !next.held.contains(&action) {
                    next.enqueue(PendingEvent {
                        due: tick,
                        event: next.compiled.event_ids[event].clone(),
                        fields: BTreeMap::new(),
                    })?;
                }
            }
            next.held = next
                .compiled
                .inputs
                .iter()
                .filter(|(a, _)| input.pressed(a))
                .map(|(a, _)| a.clone())
                .collect();
            next.drain(tick, &mut Budget(0))?;
            Ok(next)
        })();
        match result {
            Ok(next) => *self = next,
            Err(e) => self.fault = Some(axiom_core::error_response(&e)),
        }
    }
    pub fn choices(&self) -> Vec<&CompiledChoice> {
        self.node.map_or(vec![], |n| {
            self.compiled.nodes[n]
                .choices
                .iter()
                .filter(|c| self.test(&c.when))
                .collect()
        })
    }
    pub fn text(&self, key: &str) -> String {
        self.compiled
            .ui
            .as_ref()
            .and_then(|u| self.locale.as_ref().and_then(|l| u.catalogs.get(l)))
            .and_then(|c| c.get(key))
            .cloned()
            .unwrap_or_else(|| key.into())
    }
    pub fn progress(&self) -> Progress {
        Progress {
            variables: self
                .compiled
                .variables
                .iter()
                .cloned()
                .zip(self.values.clone())
                .collect(),
            node: self.node.map(|n| self.compiled.nodes[n].id.clone()),
            ending: self.ending.clone(),
            active_scenes: self
                .compiled
                .scenes
                .iter()
                .zip(&self.active)
                .filter(|(_, v)| **v)
                .map(|(id, _)| id.clone())
                .collect(),
            pending: self.pending.iter().cloned().collect(),
            locale: self.locale.clone(),
            held_inputs: self.held.clone(),
            calls: self.call_stats.clone(),
        }
    }
    pub fn restore(&mut self, p: &Progress, tick: u64) -> Result<()> {
        ensure!(
            p.variables.len() == self.compiled.variables.len(),
            "save variable set mismatch"
        );
        let mut next = self.clone();
        for (i, id) in self.compiled.variables.iter().enumerate() {
            let v = p.variables.get(id).context("save variable missing")?;
            v.validate()?;
            ensure!(
                v.ty() == self.compiled.initial[i].ty(),
                "save variable type mismatch"
            );
            next.values[i] = v.clone();
        }
        next.node = p
            .node
            .as_ref()
            .map(|id| {
                self.compiled
                    .nodes
                    .iter()
                    .position(|n| &n.id == id)
                    .context("save node missing")
            })
            .transpose()?;
        ensure!(
            next.node.is_some() == self.compiled.start.is_some(),
            "save narrative state missing"
        );
        next.ending = p.ending.clone();
        if let Some(e) = &next.ending {
            ensure!(
                self.compiled
                    .nodes
                    .iter()
                    .any(|n| n.ending.as_ref() == Some(e))
                    || self
                        .compiled
                        .nodes
                        .iter()
                        .flat_map(|n| n.enter.iter().chain(n.choices.iter().flat_map(|c| &c.ops)))
                        .any(|op| matches!(op,Op::End(ending) if ending==e))
                    || self
                        .compiled
                        .program
                        .rules
                        .iter()
                        .flat_map(|r| &r.actions)
                        .any(|a| matches!(a,GameAction::End{ending} if ending==e)),
                "save ending missing"
            );
        }
        ensure!(
            p.active_scenes.len() <= self.compiled.scenes.len(),
            "save scene budget exceeded"
        );
        for id in &p.active_scenes {
            ensure!(self.compiled.scenes.contains(id), "save scene missing");
        }
        next.active = self
            .compiled
            .scenes
            .iter()
            .map(|id| p.active_scenes.contains(id))
            .collect();
        ensure!(p.pending.len() <= MAX_EVENTS, "save event budget exceeded");
        next.pending.clear();
        for e in &p.pending {
            ensure!(
                e.due >= tick && e.due <= tick.saturating_add(1_000_000),
                "invalid saved event due tick"
            );
            validate_command(
                &RuntimeCommand::Emit {
                    event: e.event.clone(),
                    fields: e.fields.clone(),
                },
                &self.compiled.program,
                None,
                None,
            )?;
            next.pending.push_back(e.clone());
        }
        if let Some(l) = &p.locale {
            ensure!(
                self.compiled
                    .ui
                    .as_ref()
                    .is_some_and(|u| u.catalogs.contains_key(l)),
                "save locale missing"
            );
        } else {
            ensure!(self.compiled.ui.is_none(), "save locale missing");
        }
        next.locale = p.locale.clone();
        ensure!(
            p.held_inputs.len() <= 64
                && p.held_inputs
                    .iter()
                    .all(|id| self.compiled.inputs.iter().any(|(a, _)| a == id)),
            "invalid saved inputs"
        );
        next.held = p.held_inputs.clone();
        next.calls.clear();
        next.call_stats = p.calls.clone();
        next.fault = None;
        next.trace.clear();
        *self = next;
        Ok(())
    }
    pub fn transfer(&mut self, old: &Self, tick: u64) -> Result<()> {
        // Compatible value edits retain progress; schema/default changes take their new defaults.
        for (i, id) in self.compiled.variables.iter().enumerate() {
            if let Some(j) = old.compiled.variables.iter().position(|v| v == id) {
                if self.compiled.initial[i] == old.compiled.initial[j] {
                    self.values[i] = old.values[j].clone();
                }
            }
        }
        if let Some(n) = old.node {
            let id = &old.compiled.nodes[n].id;
            let new = self
                .compiled
                .nodes
                .iter()
                .position(|n| &n.id == id)
                .context("active narrative node removed; restart or migrate progress")?;
            self.node = Some(new);
            self.ending = old.ending.clone();
        }
        for (i, id) in self.compiled.scenes.iter().enumerate() {
            if let Some(j) = old.compiled.scenes.iter().position(|s| s == id) {
                self.active[i] = old.active[j];
            }
        }
        for e in &old.pending {
            validate_command(
                &RuntimeCommand::Emit {
                    event: e.event.clone(),
                    fields: e.fields.clone(),
                },
                &self.compiled.program,
                None,
                None,
            )?;
        }
        self.pending = old.pending.clone();
        if old.locale.as_ref().is_some_and(|l| {
            self.compiled
                .ui
                .as_ref()
                .is_some_and(|u| u.catalogs.contains_key(l))
        }) {
            self.locale = old.locale.clone();
        }
        self.held = old
            .held
            .iter()
            .filter(|a| self.compiled.inputs.iter().any(|(id, _)| id == *a))
            .cloned()
            .collect();
        self.trace = old.trace.clone();
        self.trace_sequence = old.trace_sequence;
        self.call_stats = old.call_stats.clone();
        self.record(tick, "activation", "behavior.execute");
        Ok(())
    }
    pub fn bound_text(&self, b: &BoundText) -> String {
        match b {
            BoundText::Key(k) => self.text(k),
            BoundText::Variable(i) => self.values[*i].display(),
            BoundText::LocalizedVariable(i) => self.text(&self.values[*i].display()),
            BoundText::Speaker => self.node.map_or(String::new(), |n| {
                self.text(&self.compiled.nodes[n].speaker)
            }),
            BoundText::Line => self
                .node
                .map_or(String::new(), |n| self.text(&self.compiled.nodes[n].text)),
            BoundText::Ending => self.ending.clone().unwrap_or_default(),
            BoundText::Prompt => self
                .prompt
                .as_ref()
                .map_or(String::new(), |p| self.prompt_text(p)),
        }
    }
    /// "[E] localized text", or the text with `{key}` replaced when it places the key itself.
    pub fn prompt_text(&self, p: &Prompt) -> String {
        let text = self.text(&p.text);
        if p.key.is_empty() {
            text
        } else if text.contains("{key}") {
            text.replace("{key}", &p.key)
        } else {
            format!("[{}] {text}", p.key)
        }
    }
    pub fn observation(&self) -> Value {
        let bindings = self
            .compiled
            .widgets
            .iter()
            .filter_map(|w| match w {
                BoundWidget::Text(id, b, when) if self.test(when) => {
                    Some((id.clone(), json!(self.bound_text(b))))
                }
                BoundWidget::Progress(id, i, max) => {
                    let n = match &self.values[*i] {
                        Scalar::Int(v) => *v as f64,
                        Scalar::Number(v) => *v,
                        _ => 0.0,
                    };
                    Some((id.clone(), json!((n / max).clamp(0.0, 1.0))))
                }
                BoundWidget::Slider(s) if s.variable.is_some() && self.test(&s.when) => {
                    Some((s.id.clone(), json!(self.slider_value(s, None))))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let mut out = json!({"bindings":bindings,"variables":self.progress().variables,"node":self.node.map(|n|&self.compiled.nodes[n].id),"ending":self.ending,"choices":self.choices().iter().map(|c|json!({"id":c.id,"text":self.text(&c.text)})).collect::<Vec<_>>(),"active_scenes":self.progress().active_scenes,"pending_events":self.pending.len(),"locale":self.locale,"trace":self.trace,"error":self.fault});
        // Only when used, so other worlds keep their observation (and state_hash).
        if !self.animation.is_empty() {
            out["animation_reads"] = self
                .compiled
                .animation_reads
                .iter()
                .zip(&self.animation)
                .map(|(r, v)| {
                    let name = json!(r.field);
                    let mut key = format!("{}.{}", r.entity, name.as_str().unwrap_or("?"));
                    if let Some(p) = &r.param {
                        key.push('.');
                        key.push_str(p);
                    }
                    (key, json!(v))
                })
                .collect::<serde_json::Map<_, _>>()
                .into();
        }
        out
    }
}
pub struct BehaviorSystem;
impl crate::world::RuntimeSystem for BehaviorSystem {
    fn receive_events(
        &mut self,
        state: &mut crate::world::RuntimeState,
        events: &[crate::events::Envelope],
    ) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut next = state
            .gameplay
            .clone()
            .context("event consumer has no gameplay state")?;
        for event in events {
            next.command(
                &RuntimeCommand::Emit {
                    event: event.event.clone(),
                    fields: event.fields.clone(),
                },
                state.tick,
            )?;
        }
        let scene_changed = state
            .gameplay
            .as_ref()
            .is_none_or(|old| old.active != next.active);
        state.gameplay = Some(next);
        if scene_changed {
            state.sync_scenes();
        }
        Ok(())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn step(&mut self, state: &mut crate::world::RuntimeState, input: &crate::ActionMap) {
        if let Some(g) = &mut state.gameplay {
            g.step(input, state.tick);
        }
        state.sync_scenes();
    }
    fn transfer(
        &mut self,
        _old: &dyn crate::world::RuntimeSystem,
        previous: &crate::world::RuntimeState,
        next: &mut crate::world::RuntimeState,
    ) -> Result<()> {
        if previous.compiled.source.id == next.compiled.source.id {
            if let (Some(g), Some(old)) = (&mut next.gameplay, &previous.gameplay) {
                g.transfer(old, next.tick)?;
            }
        }
        next.sync_scenes();
        Ok(())
    }
    fn checkpoint_supported(&self) -> bool {
        true
    }
}
