//! `ui.effects`: data-driven UI effects (`ui.document.effects`) on the fixed step.
//!
//! The effects are compiled with the gameplay document (`axiom_core::gameplay::CompiledEffect`:
//! widget / event / condition indices, defaults applied). This system keeps one fixed slot per
//! effect and one baseline per widget, allocated when the runtime is built; a step only reads
//! the gameplay state and updates those slots (no allocation, no JSON). At most
//! `MAX_ACTIVE_UI_EFFECTS` run at once; a trigger beyond that is dropped and counted.
//! Drawing (`bound_ui`) asks [`EffectVisuals::visual`] for each widget's opacity, offset, scale
//! and rolled number. State survives hot reload by effect / widget ID; checkpoints are refused
//! while an effect runs (its progress is not saved).
use crate::{gameplay::Gameplay, world::*};
use anyhow::{Context, Result};
use axiom_core::gameplay::{
    BoundText, BoundWidget, CompiledEffect, CompiledGameplay, EffectTrigger, Scalar,
    SlideDirection, UiEffectKind, MAX_ACTIVE_UI_EFFECTS,
};
use axiom_core::SystemDescriptor;
use serde_json::{json, Value};
use std::{any::Any, collections::BTreeMap, sync::Arc};

/// How effects change one widget this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visual {
    pub alpha: f32,
    /// Pixels, added to the widget's laid-out position.
    pub offset: [f32; 2],
    /// About the widget's center.
    pub scale: f32,
    /// The number a number_roll shows instead of the variable's value.
    pub number: Option<f64>,
    /// An exit effect runs: draw the widget even though it is hidden now.
    pub forced: bool,
}
impl Visual {
    pub const IDENTITY: Visual = Visual {
        alpha: 1.0,
        offset: [0.0, 0.0],
        scale: 1.0,
        number: None,
        forced: false,
    };
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

/// Per-widget effect visuals for `bound_ui` (the runtime system, or the editor's preview).
pub trait EffectVisuals {
    fn visual(&self, widget: usize) -> Visual;
}

/// Adds one running effect's contribution at `elapsed` steps (`from` → `to` for number_roll).
pub fn contribution(e: &CompiledEffect, elapsed: u32, from: f64, to: f64, v: &mut Visual) {
    let u = (elapsed as f32 / e.ticks as f32).clamp(0.0, 1.0);
    let k = e.ease.apply(u);
    let [dx, dy] = match e.direction {
        SlideDirection::Left => [-1.0, 0.0],
        SlideDirection::Right => [1.0, 0.0],
        SlideDirection::Up => [0.0, -1.0],
        SlideDirection::Down => [0.0, 1.0],
    };
    match e.kind {
        UiEffectKind::FadeIn => v.alpha *= k,
        UiEffectKind::FadeOut => {
            v.alpha *= 1.0 - k;
            v.forced = true;
        }
        UiEffectKind::SlideIn => {
            v.offset[0] += dx * e.amount * (1.0 - k);
            v.offset[1] += dy * e.amount * (1.0 - k);
        }
        UiEffectKind::SlideOut => {
            v.offset[0] += dx * e.amount * k;
            v.offset[1] += dy * e.amount * k;
            v.forced = true;
        }
        UiEffectKind::ScaleIn => v.scale *= e.amount + (1.0 - e.amount) * k,
        UiEffectKind::ScaleOut => {
            v.scale *= 1.0 + (e.amount - 1.0) * k;
            v.forced = true;
        }
        UiEffectKind::Pulse => v.scale *= 1.0 + e.amount * (std::f32::consts::PI * k).sin(),
        // Five decaying swings.
        UiEffectKind::Shake => {
            v.offset[0] += e.amount * (1.0 - k) * (std::f32::consts::TAU * 5.0 * k).sin()
        }
        // The curve is f32; round off its noise (3.0000001 -> 3) so observe reads cleanly.
        UiEffectKind::NumberRoll => {
            v.number = Some(((from + (to - from) * k as f64) * 1e4).round() / 1e4)
        }
    }
}

/// Numeric value of a scalar (0 for text / bool).
pub fn number(s: &Scalar) -> f64 {
    match s {
        Scalar::Int(v) => *v as f64,
        Scalar::Number(v) => *v,
        _ => 0.0,
    }
}

/// FNV-1a of a scalar: detects a changed value without keeping a copy of its text.
fn fingerprint(s: &Scalar) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    match s {
        Scalar::Bool(b) => eat(&[1, *b as u8]),
        Scalar::Int(v) => {
            eat(&[2]);
            eat(&v.to_le_bytes())
        }
        Scalar::Number(v) => {
            eat(&[3]);
            eat(&v.to_bits().to_le_bytes())
        }
        Scalar::Text(t) => {
            eat(&[4]);
            eat(t.as_bytes())
        }
    }
    h
}

/// Localized text of `key` in the current locale, borrowed (the key when missing).
fn text<'a>(g: &'a Gameplay, key: &'a str) -> &'a str {
    g.compiled
        .ui
        .as_ref()
        .and_then(|u| g.locale.as_ref().and_then(|l| u.catalogs.get(l)))
        .and_then(|c| c.get(key))
        .map_or(key, String::as_str)
}

fn bound_text_empty(g: &Gameplay, b: &BoundText) -> bool {
    match b {
        BoundText::Key(k) => text(g, k).is_empty(),
        BoundText::Variable(i) => matches!(&g.values[*i], Scalar::Text(t) if t.is_empty()),
        BoundText::LocalizedVariable(i) => match &g.values[*i] {
            Scalar::Text(k) => text(g, k).is_empty(),
            _ => false,
        },
        BoundText::Speaker => g
            .node
            .is_none_or(|n| text(g, &g.compiled.nodes[n].speaker).is_empty()),
        BoundText::Line => g
            .node
            .is_none_or(|n| text(g, &g.compiled.nodes[n].text).is_empty()),
        BoundText::Ending => g.ending.as_ref().is_none_or(String::is_empty),
        BoundText::Prompt => g
            .prompt
            .as_ref()
            .is_none_or(|p| p.key.is_empty() && text(g, &p.text).is_empty()),
    }
}

/// Whether a widget would draw anything now (without building its text).
pub fn widget_visible(g: &Gameplay, w: &BoundWidget) -> bool {
    match w {
        BoundWidget::Text(_, binding, when) => g.test(when) && !bound_text_empty(g, binding),
        BoundWidget::Button(_, _, _, when) => g.test(when),
        BoundWidget::Choices(_) => g
            .node
            .is_some_and(|n| g.compiled.nodes[n].choices.iter().any(|c| g.test(&c.when))),
        BoundWidget::Image(_, image) => g.images.contains_key(image),
        BoundWidget::Progress(..) => true,
        BoundWidget::Slider(s) => g.test(&s.when),
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    active: bool,
    /// A show effect of a widget visible when the session was built: drawn at its first
    /// frame before the first step, started by that step (not running, not counted yet).
    armed: bool,
    elapsed: u32,
    from: f64,
    /// number_roll target: the variable's value, refreshed every step.
    to: f64,
}

/// What a widget showed at the end of the last step.
#[derive(Clone, Copy, Debug, Default)]
struct Seen {
    visible: bool,
    value: u64,
    number: f64,
}

const SHOWN: u8 = 1;
const HIDDEN: u8 = 2;
const CHANGED: u8 = 4;

fn enters(kind: UiEffectKind) -> bool {
    matches!(
        kind,
        UiEffectKind::FadeIn | UiEffectKind::SlideIn | UiEffectKind::ScaleIn
    )
}

pub struct UiEffects {
    compiled: Arc<CompiledGameplay>,
    slots: Vec<Slot>,
    seen: Vec<Seen>,
    /// Per widget, this step's SHOWN / HIDDEN / CHANGED flags (reused buffer).
    flags: Vec<u8>,
    /// Per effect, the `when` value of the last step (`on: when` fires on its rising edge).
    when_was: Vec<bool>,
    /// Last gameplay trace sequence scanned for events.
    seq: u64,
    /// No step has run since this system was built for a new session: the next step starts
    /// the armed show effects. Cleared by a checkpoint load and carried over by hot reload.
    session_start: bool,
    active: usize,
    started: u64,
    dropped: u64,
}

impl UiEffects {
    fn new(g: &Gameplay) -> Self {
        let compiled = g.compiled.clone();
        let mut fx = Self {
            slots: vec![Slot::default(); compiled.effects.len()],
            seen: vec![Seen::default(); compiled.widgets.len()],
            flags: vec![0; compiled.widgets.len()],
            when_was: vec![false; compiled.effects.len()],
            seq: g.trace.back().map_or(0, |t| t.seq),
            session_start: true,
            active: 0,
            started: 0,
            dropped: 0,
            compiled,
        };
        fx.baseline(g);
        fx.arm(g);
        fx
    }

    /// Baselines, `when` values and nothing running: the state right after a load.
    fn rebuild(&mut self, g: &Gameplay) {
        self.baseline(g);
        for slot in &mut self.slots {
            slot.active = false;
            slot.armed = false;
        }
        self.active = 0;
    }

    fn baseline(&mut self, g: &Gameplay) {
        for (w, seen) in self.compiled.widgets.iter().zip(&mut self.seen) {
            *seen = observe(g, w);
        }
        for (was, e) in self.when_was.iter_mut().zip(&self.compiled.effects) {
            *was = g.test(&e.when);
        }
    }

    /// Arms the show effects of the widgets visible now (session start).
    fn arm(&mut self, g: &Gameplay) {
        for (slot, e) in self.slots.iter_mut().zip(&self.compiled.effects) {
            slot.armed = !slot.active
                && e.trigger == EffectTrigger::Show
                && self.seen[e.widget].visible
                && g.test(&e.when);
        }
    }

    /// Starts (or restarts) effect `i`; false when the running budget is full. An enter
    /// effect ends the widget's running exit effects and the other way round, so a widget
    /// shown again mid fade-out fades in from where it is instead of fading twice.
    fn start(&mut self, i: usize) -> bool {
        let e = &self.compiled.effects[i];
        let (widget, kind) = (e.widget, e.kind);
        for (j, other) in self.compiled.effects.iter().enumerate() {
            let opposite =
                (enters(kind) && other.kind.exit()) || (kind.exit() && enters(other.kind));
            if j != i && other.widget == widget && opposite && self.slots[j].active {
                self.slots[j].active = false;
                self.active -= 1;
            }
        }
        let e = &self.compiled.effects[i];
        let seen = self.seen[e.widget];
        let slot = &mut self.slots[i];
        slot.armed = false;
        let from = match (e.from, e.trigger, slot.active) {
            (Some(from), ..) => from,
            // A restarted roll continues from what it shows now.
            (None, _, true) => {
                slot.from
                    + (slot.to - slot.from)
                        * e.ease
                            .apply((slot.elapsed as f32 / e.ticks as f32).clamp(0.0, 1.0))
                            as f64
            }
            (None, EffectTrigger::Change, false) => seen.number,
            (None, ..) => 0.0,
        };
        if !slot.active {
            if self.active >= MAX_ACTIVE_UI_EFFECTS {
                self.dropped += 1;
                return false;
            }
            self.active += 1;
        }
        *slot = Slot {
            active: true,
            armed: false,
            elapsed: 0,
            from,
            to: seen.number,
        };
        self.started += 1;
        true
    }

    fn step_effects(&mut self, g: &Gameplay) {
        // Running effects advance; the ones that reached their length end.
        for (slot, e) in self.slots.iter_mut().zip(&self.compiled.effects) {
            if slot.active {
                slot.elapsed += 1;
                if slot.elapsed >= e.ticks {
                    slot.active = false;
                    self.active -= 1;
                }
            }
        }
        // Widget changes since the last step.
        for i in 0..self.seen.len() {
            let now = observe(g, &self.compiled.widgets[i]);
            let was = self.seen[i];
            self.flags[i] = (u8::from(now.visible && !was.visible) * SHOWN)
                | (u8::from(!now.visible && was.visible) * HIDDEN)
                | (u8::from(now.value != was.value) * CHANGED);
            // Keep the old number until the triggers below have read it.
            self.seen[i].visible = now.visible;
            self.seen[i].value = now.value;
            if self.flags[i] & CHANGED == 0 {
                self.seen[i].number = now.number;
            }
        }
        let last = g.trace.back().map_or(self.seq, |t| t.seq);
        for i in 0..self.slots.len() {
            let e = &self.compiled.effects[i];
            let fire = match e.trigger {
                EffectTrigger::Show => self.flags[e.widget] & SHOWN != 0 && g.test(&e.when),
                EffectTrigger::Hide => self.flags[e.widget] & HIDDEN != 0 && g.test(&e.when),
                EffectTrigger::Change => self.flags[e.widget] & CHANGED != 0 && g.test(&e.when),
                EffectTrigger::Event(event) => {
                    let id = &self.compiled.event_ids[event];
                    g.trace
                        .iter()
                        .any(|t| t.seq > self.seq && t.kind == "event" && &t.id == id)
                        && g.test(&e.when)
                }
                EffectTrigger::When => {
                    let now = g.test(&e.when);
                    let rose = now && !self.when_was[i];
                    self.when_was[i] = now;
                    rose
                }
            };
            if fire {
                self.start(i);
            }
        }
        self.seq = last;
        // Rolls follow the live value; changed widgets now record their new number.
        for i in 0..self.seen.len() {
            if self.flags[i] & CHANGED != 0 {
                self.seen[i].number = observe(g, &self.compiled.widgets[i]).number;
            }
        }
        for (slot, e) in self.slots.iter_mut().zip(&self.compiled.effects) {
            if slot.active {
                slot.to = self.seen[e.widget].number;
            }
        }
    }

    /// Running effects for `observe` (and tests).
    pub fn summary(&self) -> Value {
        let active: Vec<Value> = self
            .slots
            .iter()
            .zip(&self.compiled.effects)
            .filter(|(s, _)| s.active)
            .map(|(s, e)| {
                let mut v = Visual::IDENTITY;
                contribution(e, s.elapsed, s.from, s.to, &mut v);
                let mut row = json!({
                    "id": e.id,
                    "widget": widget_name(&self.compiled.widgets[e.widget]),
                    "kind": e.kind.name(),
                    "elapsed": s.elapsed,
                    "ticks": e.ticks,
                    "alpha": v.alpha,
                    "offset": v.offset,
                    "scale": v.scale,
                });
                if let Some(n) = v.number {
                    row["number"] = json!(n);
                }
                row
            })
            .collect();
        json!({"defined": self.slots.len(), "active": active, "started": self.started,
            "dropped": self.dropped, "budget": MAX_ACTIVE_UI_EFFECTS})
    }

    /// Effects running now (steady state: none; checkpoints need none).
    pub fn active(&self) -> usize {
        self.active
    }
}

fn widget_name(w: &BoundWidget) -> &str {
    match w {
        BoundWidget::Text(id, ..)
        | BoundWidget::Button(id, ..)
        | BoundWidget::Choices(id)
        | BoundWidget::Image(id, _)
        | BoundWidget::Progress(id, ..) => id,
        BoundWidget::Slider(s) => &s.id,
    }
}

fn observe(g: &Gameplay, w: &BoundWidget) -> Seen {
    let variable = axiom_core::gameplay::widget_variable(w).map(|i| &g.values[i]);
    Seen {
        visible: widget_visible(g, w),
        value: variable.map_or(0, fingerprint),
        number: variable.map_or(0.0, number),
    }
}

impl EffectVisuals for UiEffects {
    fn visual(&self, widget: usize) -> Visual {
        let mut v = Visual::IDENTITY;
        for (s, e) in self.slots.iter().zip(&self.compiled.effects) {
            if (s.active || s.armed) && e.widget == widget {
                contribution(e, s.elapsed, s.from, s.to, &mut v);
            }
        }
        v
    }
}

impl RuntimeSystem for UiEffects {
    fn as_any(&self) -> &dyn Any {
        self
    }
    /// Idle effect state is derived from the gameplay state, so idle checkpoints are complete;
    /// `checkpoint_ready` refuses while one runs.
    fn checkpoint_supported(&self) -> bool {
        true
    }
    fn checkpoint_ready(&self) -> Result<()> {
        if self.active == 0 {
            return Ok(());
        }
        let running: Vec<&str> = self
            .slots
            .iter()
            .zip(&self.compiled.effects)
            .filter(|(s, _)| s.active)
            .map(|(_, e)| e.id.as_str())
            .take(4)
            .collect();
        anyhow::bail!(
            "{} UI effect(s) are running ({}); their progress is not saved in checkpoints. Save again after they finish (at most {} steps)",
            self.active,
            running.join(", "),
            axiom_core::gameplay::MAX_UI_EFFECT_TICKS
        )
    }
    /// A checkpoint was loaded: nothing is entering or running; baselines are the loaded state.
    fn restored(&mut self, state: &RuntimeState) {
        self.session_start = false;
        if let Some(g) = &state.gameplay {
            self.rebuild(g);
            self.seq = g.trace.back().map_or(self.seq, |t| t.seq);
        }
    }
    fn step(&mut self, _state: &mut RuntimeState, _input: &ActionMap) {}
    /// After event delivery, so changes made by events this step trigger this step.
    fn finish_step(&mut self, state: &mut RuntimeState) {
        let Some(g) = &state.gameplay else {
            return;
        };
        if std::mem::take(&mut self.session_start) {
            if state.tick != 1 {
                // Scheduled into a session already running (or a content switch): what the
                // widgets show now is the baseline, and the visible ones enter.
                self.baseline(g);
                self.arm(g);
            }
            for i in 0..self.slots.len() {
                if self.slots[i].armed {
                    self.start(i);
                }
            }
        }
        self.step_effects(g);
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        let Some(old) = old.as_any().downcast_ref::<UiEffects>() else {
            return Ok(());
        };
        // Another world: its gameplay starts over (its own trace), so this is a session start.
        if previous.compiled.source.id != next.compiled.source.id {
            return Ok(());
        }
        let Some(g) = &next.gameplay else {
            return Ok(());
        };
        // Baselines of new widgets come from the carried-over gameplay state.
        self.baseline(g);
        for (w, seen) in self.compiled.widgets.iter().zip(&mut self.seen) {
            if let Some(j) = old
                .compiled
                .widgets
                .iter()
                .position(|o| widget_name(o) == widget_name(w))
            {
                *seen = old.seen[j];
            }
        }
        self.seq = old.seq;
        self.session_start = old.session_start;
        self.started = old.started;
        self.dropped = old.dropped;
        self.active = 0;
        let mut kept = vec![false; self.slots.len()];
        for (i, e) in self.compiled.effects.iter().enumerate() {
            self.slots[i] = Slot::default();
            if let Some(j) = old.compiled.effects.iter().position(|o| o.id == e.id) {
                self.slots[i] = old.slots[j];
                self.when_was[i] = old.when_was[j];
                kept[i] = true;
                self.active += usize::from(self.slots[i].active);
            }
        }
        // New show effects whose widget is visible in the live state enter once (an effect
        // added during Play is seen at once); before the first step they wait armed.
        for (i, kept) in kept.into_iter().enumerate() {
            let e = &self.compiled.effects[i];
            let enter = !kept
                && e.trigger == EffectTrigger::Show
                && self.seen[e.widget].visible
                && g.test(&e.when);
            if enter {
                if self.session_start {
                    self.slots[i].armed = true;
                } else {
                    self.start(i);
                }
            }
        }
        // An edit may shorten an effect below its progress: it ends on the next step.
        Ok(())
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("ui_effects".into(), self.summary())])
    }
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.systems.insert(
        axiom_core::ops::UI_EFFECTS_SYSTEM.into(),
        SystemDescriptor {
            id: axiom_core::ops::UI_EFFECTS_SYSTEM.into(),
            capability: "ui".into(),
            phase: 96,
            // Not `ui.document`: creating a document must not schedule this system.
            reads: vec!["ui.document/effects".into()],
            writes: vec![],
            after: vec!["behavior.execute".into()],
        },
    );
    r.factories.insert(axiom_core::ops::UI_EFFECTS_SYSTEM.into(), |state| {
        let g = state
            .gameplay
            .as_ref()
            .context("ui.effects runs ui.document effects; the world needs behavior.execute and a ui.document")?;
        Ok(Box::new(UiEffects::new(g)) as Box<dyn RuntimeSystem>)
    });
    r.authoring.topics.insert(
        "ui_effects".into(),
        json!({"resource":"ui.document/effects (system ui.effects)",
            "kinds":UiEffectKind::ALL.map(|k| k.name()),
            "on":"show|hide|change|event|when; default show (*_in), hide (*_out), change (pulse, shake, number_roll)",
            "options":{"ticks":"1..1200 fixed steps (120/s), default 30 (number_roll 60)","ease":"linear|step|ease_in|ease_out|ease_in_out","direction":"left|right|up|down (slides)","amount":"slide/shake px, scale_in/out scale, pulse extra scale","from":"number_roll start"},
            "budget":{"effects":axiom_core::gameplay::MAX_UI_EFFECTS,"running":MAX_ACTIVE_UI_EFFECTS,"over_budget":"trigger dropped and counted"},
            "observe":"ui_effects {defined, active:[{id, widget, kind, elapsed, ticks, alpha, offset, scale, number?}], started, dropped}",
            "checkpoint":"refused while an effect runs",
            "acl":"ui place|style|button|effect add|effect set|effect remove"}),
    );
    r.authoring.world_validators.push(|w| {
        let has_effects = w
            .resources
            .get(axiom_core::ops::UI_DOCUMENT)
            .and_then(|d| d.get("effects"))
            .and_then(Value::as_array)
            .is_some_and(|e| !e.is_empty());
        anyhow::ensure!(
            !has_effects
                || w.systems
                    .iter()
                    .any(|s| s == axiom_core::ops::UI_EFFECTS_SYSTEM),
            "ui.document effects need the ui.effects system (system add ui.effects)"
        );
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_core::gameplay::RuntimeCommand;
    use axiom_core::World;

    fn world(effects: Value) -> World {
        let mut w = World::empty("fx");
        for c in ["behavior", "ui", "save"] {
            w.capabilities.insert(c.into(), 1);
        }
        w.systems = vec!["behavior.execute".into(), "ui.effects".into()];
        w.resources.insert(
            "behavior.program".into(),
            json!({"variables":{"score":0,"shown":true},
                "events":{"bump":{},"ping":{},"hide":{}},
                "rules":[
                    {"id":"bump","event":"bump","actions":[{"kind":"add","variable":"score","value":{"kind":"literal","value":10}}]},
                    {"id":"hide","event":"hide","actions":[{"kind":"set","variable":"shown","value":{"kind":"literal","value":false}}]}]}),
        );
        w.resources.insert(
            "ui.document".into(),
            json!({"locale":"en","catalogs":{"en":{"title":"Title"}},
                "widgets":[
                    {"kind":"text","id":"score","binding":{"kind":"variable","id":"score"}},
                    {"kind":"text","id":"title","binding":{"kind":"key","id":"title"},
                     "when":{"kind":"eq","left":{"kind":"variable","id":"shown"},"right":{"kind":"literal","value":true}}},
                    {"kind":"progress","id":"bar","variable":"score","max":100}],
                "effects":effects}),
        );
        w
    }

    fn runtime(effects: Value) -> WorldRuntime {
        WorldRuntime::new(world(effects), 1, crate::installed_registry()).unwrap()
    }

    fn emit(rt: &mut WorldRuntime, event: &str) {
        rt.command(&RuntimeCommand::Emit {
            event: event.into(),
            fields: Default::default(),
        })
        .unwrap();
    }

    fn steps(rt: &mut WorldRuntime, n: usize) {
        for _ in 0..n {
            rt.step(&ActionMap::default());
        }
    }

    fn fx(rt: &WorldRuntime) -> &UiEffects {
        rt.system::<UiEffects>().unwrap()
    }

    fn compiled(kind: UiEffectKind, amount: f32) -> CompiledEffect {
        CompiledEffect {
            id: "e".into(),
            widget: 0,
            kind,
            trigger: EffectTrigger::Show,
            when: axiom_core::gameplay::Test::Always,
            ticks: 100,
            ease: axiom_core::tween::Ease::Linear,
            direction: SlideDirection::Right,
            amount,
            from: None,
            variable: None,
        }
    }

    fn at(kind: UiEffectKind, amount: f32, elapsed: u32) -> Visual {
        let mut v = Visual::IDENTITY;
        contribution(&compiled(kind, amount), elapsed, 0.0, 10.0, &mut v);
        v
    }

    #[test]
    fn every_kind_has_its_value_at_given_steps() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        use UiEffectKind::*;
        assert!(close(at(FadeIn, 0.0, 0).alpha, 0.0) && close(at(FadeIn, 0.0, 25).alpha, 0.25));
        assert!(close(at(FadeOut, 0.0, 75).alpha, 0.25) && at(FadeOut, 0.0, 75).forced);
        assert_eq!(at(SlideIn, 40.0, 0).offset, [40.0, 0.0]);
        assert!(close(at(SlideIn, 40.0, 50).offset[0], 20.0));
        assert!(close(at(SlideOut, 40.0, 50).offset[0], 20.0) && at(SlideOut, 40.0, 1).forced);
        assert!(close(at(ScaleIn, 0.6, 0).scale, 0.6) && close(at(ScaleIn, 0.6, 50).scale, 0.8));
        assert!(close(at(ScaleOut, 0.6, 50).scale, 0.8) && at(ScaleOut, 0.6, 50).forced);
        assert!(close(at(Pulse, 0.2, 0).scale, 1.0) && close(at(Pulse, 0.2, 50).scale, 1.2));
        assert!(close(at(Shake, 6.0, 0).offset[0], 0.0));
        // Five swings that decay: a quarter swing in (u = 0.05) is +amount * 0.95.
        assert!(close(at(Shake, 6.0, 5).offset[0], 6.0 * 0.95));
        assert_eq!(at(NumberRoll, 0.0, 0).number, Some(0.0));
        assert_eq!(at(NumberRoll, 0.0, 30).number, Some(3.0));
        // The full length is identity again for the in-effects (they end there).
        assert!(close(at(FadeIn, 0.0, 100).alpha, 1.0) && close(at(ScaleIn, 0.6, 100).scale, 1.0));
    }

    #[test]
    fn triggers_start_effects_on_the_fixed_step() {
        let mut rt = runtime(json!([
            {"id":"title.in","widget":"title","kind":"fade_in","ticks":20,"ease":"linear"},
            {"id":"title.out","widget":"title","kind":"fade_out","ticks":10,"ease":"linear"},
            {"id":"score.roll","widget":"score","kind":"number_roll","ticks":60,"ease":"linear"},
            {"id":"bar.roll","widget":"bar","kind":"number_roll","ticks":60,"ease":"linear"},
            {"id":"score.shake","widget":"score","kind":"shake","on":"event","event":"ping"},
            {"id":"bar.pulse","widget":"bar","kind":"pulse","on":"when",
             "when":{"kind":"ge","left":{"kind":"variable","id":"score"},"right":{"kind":"literal","value":20}}}
        ]));
        // Session start: the visible title enters (started at build, before the first step).
        assert_eq!(fx(&rt).visual(1).alpha, 0.0);
        steps(&mut rt, 10);
        assert!((fx(&rt).visual(1).alpha - 0.5).abs() < 1e-5);
        steps(&mut rt, 10);
        assert_eq!(fx(&rt).active(), 0);
        assert!(fx(&rt).visual(1).is_identity());
        // change: score 0 -> 10 rolls on text and progress from the old value.
        emit(&mut rt, "bump");
        steps(&mut rt, 1);
        assert_eq!(fx(&rt).visual(0).number, Some(0.0));
        steps(&mut rt, 30);
        assert_eq!(fx(&rt).visual(0).number, Some(5.0));
        assert_eq!(fx(&rt).visual(2).number, Some(5.0));
        // A second change mid-roll continues from the number shown at that step (31 / 60 of
        // the way to 10) to the new value.
        emit(&mut rt, "bump");
        steps(&mut rt, 1);
        let shown = 10.0 * 31.0 / 60.0;
        let number = |rt: &WorldRuntime| fx(rt).visual(0).number.unwrap();
        assert!((number(&rt) - shown).abs() < 1e-4, "{}", number(&rt));
        // when: score >= 20 rose this step -> pulse on the bar.
        assert!(fx(&rt).summary()["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "bar.pulse"));
        steps(&mut rt, 30);
        assert!((number(&rt) - (shown + (20.0 - shown) * 0.5)).abs() < 1e-4);
        // event: ping shakes the score text.
        emit(&mut rt, "ping");
        steps(&mut rt, 1);
        assert!(fx(&rt).summary()["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "score.shake"));
        steps(&mut rt, 60);
        assert_eq!(fx(&rt).active(), 0);
        // hide: the title's `when` turns false; the exit effect keeps it drawn while it runs.
        emit(&mut rt, "hide");
        steps(&mut rt, 1);
        let v = fx(&rt).visual(1);
        assert!(v.forced && v.alpha == 1.0, "{v:?}");
        steps(&mut rt, 5);
        assert!((fx(&rt).visual(1).alpha - 0.5).abs() < 1e-5);
        steps(&mut rt, 5);
        assert!(fx(&rt).visual(1).is_identity());
        let s = fx(&rt).summary();
        assert_eq!(s["dropped"], 0);
        assert_eq!(s["defined"], 6);
    }

    #[test]
    fn same_inputs_give_the_same_effect_state_every_run() {
        let effects = json!([
            {"id":"a","widget":"score","kind":"number_roll","ticks":45},
            {"id":"b","widget":"bar","kind":"pulse"},
            {"id":"c","widget":"title","kind":"slide_in","direction":"up","amount":30}
        ]);
        let run = || {
            let mut rt = runtime(effects.clone());
            let mut trace = vec![];
            for t in 0..200 {
                if t % 37 == 5 {
                    emit(&mut rt, "bump");
                }
                steps(&mut rt, 1);
                trace.push(fx(&rt).summary());
            }
            trace
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn triggers_beyond_the_running_budget_are_dropped_and_counted() {
        let effects: Vec<Value> = (0..20)
            .map(|i| json!({"id":format!("p{i}"),"widget":"score","kind":"pulse","on":"event","event":"ping","ticks":50}))
            .collect();
        let mut rt = runtime(json!(effects));
        steps(&mut rt, 1);
        emit(&mut rt, "ping");
        steps(&mut rt, 1);
        let s = fx(&rt).summary();
        assert_eq!(fx(&rt).active(), MAX_ACTIVE_UI_EFFECTS);
        assert_eq!(s["dropped"], 20 - MAX_ACTIVE_UI_EFFECTS);
        assert_eq!(s["active"].as_array().unwrap().len(), MAX_ACTIVE_UI_EFFECTS);
        // A retrigger of a running effect restarts it without spending budget.
        steps(&mut rt, 10);
        emit(&mut rt, "ping");
        steps(&mut rt, 1);
        assert_eq!(
            fx(&rt).summary()["dropped"],
            2 * (20 - MAX_ACTIVE_UI_EFFECTS)
        );
        assert_eq!(fx(&rt).summary()["active"][0]["elapsed"], 0);
    }

    #[test]
    fn steps_reuse_the_storage_built_with_the_runtime() {
        let mut rt = runtime(json!([
            {"id":"a","widget":"score","kind":"number_roll"},
            {"id":"b","widget":"title","kind":"fade_in"}
        ]));
        let storage = |rt: &WorldRuntime| {
            let f = fx(rt);
            (
                f.slots.as_ptr() as usize,
                f.slots.capacity(),
                f.seen.as_ptr() as usize,
                f.flags.as_ptr() as usize,
                f.when_was.as_ptr() as usize,
            )
        };
        let before = storage(&rt);
        for t in 0..300 {
            if t % 50 == 0 {
                emit(&mut rt, "bump");
            }
            steps(&mut rt, 1);
        }
        assert_eq!(storage(&rt), before);
    }

    #[test]
    fn hot_reload_keeps_running_effects_by_id() {
        let mut rt = runtime(json!([
            {"id":"roll","widget":"score","kind":"number_roll","ticks":100,"ease":"linear"}
        ]));
        steps(&mut rt, 1);
        emit(&mut rt, "bump");
        steps(&mut rt, 41);
        assert_eq!(fx(&rt).summary()["active"][0]["elapsed"], 40);
        // Edit: the roll is slower, the widget order changes and a new effect appears.
        let mut w = world(json!([
            {"id":"fresh","widget":"bar","kind":"pulse","on":"event","event":"ping"},
            {"id":"roll","widget":"score","kind":"number_roll","ticks":200,"ease":"linear"}
        ]));
        let doc = w.resources.get_mut("ui.document").unwrap();
        doc["widgets"].as_array_mut().unwrap().reverse();
        rt.reconcile(w, 2).unwrap();
        let s = fx(&rt).summary();
        assert_eq!(s["active"].as_array().unwrap().len(), 1, "{s}");
        assert_eq!(s["active"][0]["id"], "roll");
        assert_eq!(s["active"][0]["elapsed"], 40);
        assert_eq!(s["active"][0]["number"], 2.0);
        steps(&mut rt, 1);
        // No spurious change trigger from the reordered widgets; the roll continues.
        assert_eq!(fx(&rt).summary()["active"][0]["elapsed"], 41);
        assert_eq!(fx(&rt).summary()["started"], 1);
    }

    #[test]
    fn checkpoints_are_refused_while_an_effect_runs() {
        let effects = json!([
            {"id":"roll","widget":"score","kind":"number_roll","ticks":30},
            {"id":"enter","widget":"title","kind":"fade_in","ticks":20}
        ]);
        let mut rt = runtime(effects.clone());
        steps(&mut rt, 1);
        emit(&mut rt, "bump");
        steps(&mut rt, 1);
        let error = format!("{:#}", rt.checkpoint().unwrap_err());
        assert!(
            error.contains("ui.effects") && error.contains("roll") && error.contains("not saved"),
            "{error}"
        );
        steps(&mut rt, 30);
        let saved = rt.checkpoint().unwrap();
        // Loading builds a fresh runtime; nothing replays or enters after the load.
        let mut other = runtime(effects);
        other.restore_checkpoint(&saved).unwrap();
        // Right after the load: nothing is entering (the title is drawn as is) and saving works.
        assert!(fx(&other).visual(1).is_identity());
        other.checkpoint().unwrap();
        // The first step after the load still sees what changes in it.
        emit(&mut other, "bump");
        steps(&mut other, 1);
        let s = fx(&other).summary();
        assert_eq!(s["active"].as_array().unwrap().len(), 1, "{s}");
        assert_eq!(s["active"][0]["id"], "roll");
        assert_eq!(s["started"], 1);
    }

    #[test]
    fn showing_again_mid_exit_replaces_the_exit_effect() {
        let mut rt = runtime(json!([
            {"id":"in","widget":"title","kind":"fade_in","ticks":20,"ease":"linear"},
            {"id":"out","widget":"title","kind":"fade_out","ticks":20,"ease":"linear"}
        ]));
        steps(&mut rt, 30);
        emit(&mut rt, "hide");
        steps(&mut rt, 11);
        // Started on the first of those steps: 10 of 20 steps played.
        assert!((fx(&rt).visual(1).alpha - 0.5).abs() < 1e-5);
        // Shown again: the fade-out ends, only the fade-in runs.
        let g = rt.state.gameplay.as_mut().unwrap();
        let shown = g
            .compiled
            .variables
            .iter()
            .position(|v| v == "shown")
            .unwrap();
        g.values[shown] = axiom_core::gameplay::Scalar::Bool(true);
        steps(&mut rt, 1);
        let s = fx(&rt).summary();
        let ids: Vec<&str> = s["active"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["id"].as_str())
            .filter(|id| *id == "in" || *id == "out")
            .collect();
        assert_eq!(ids, ["in"], "{s}");
        assert_eq!(fx(&rt).visual(1).alpha, 0.0);
        assert!(!fx(&rt).visual(1).forced);
    }

    #[test]
    fn effects_without_the_system_are_rejected() {
        let mut w = world(json!([{"id":"a","widget":"score","kind":"pulse"}]));
        w.systems.retain(|s| s != "ui.effects");
        let error = format!(
            "{:#}",
            WorldRuntime::new(w, 1, crate::installed_registry())
                .err()
                .unwrap()
        );
        assert!(error.contains("system add ui.effects"), "{error}");
    }
}
