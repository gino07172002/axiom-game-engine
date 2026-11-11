//! Animation controller: typed parameters, states that play one clip or a 1D blend tree, and
//! ordered transitions with conditions, exit time and cross-fade duration.
//!
//! Content declares controllers once in the `animation.controllers` resource and attaches one
//! to an entity with an `animation.controller` component (next to its `animation.player`).
//! [`Compiled`] resolves names to indices once; [`State`] is a small `Copy` value, so a fixed
//! step ([`Compiled::tick`]) and its weighted clip samples ([`Compiled::samples`]) never
//! allocate. The tick is deterministic: transitions are checked in list order, the first one
//! that passes wins, and a cross-fade is not interrupted.
use crate::*;

pub const MAX_CONTROLLERS: usize = 64;
pub const MAX_PARAMS: usize = 16;
pub const MAX_STATES: usize = 32;
pub const MAX_TRANSITIONS: usize = 64;
pub const MAX_CONDITIONS: usize = 8;
pub const MAX_POINTS: usize = 8;
/// Upper bound of [`Samples`]: two layers (cross-fade) of at most two blend points each.
pub const MAX_SAMPLES: usize = 4;
/// Marks a sample of the skeleton's rest pose (a state without motion).
pub const REST: u8 = u8::MAX;

fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}
fn is_one(v: &f32) -> bool {
    *v == 1.0
}
fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    Float,
    Bool,
    /// A bool that the first transition using it resets when it fires.
    Trigger,
}

/// Where the runtime reads a parameter from every fixed step (instead of commands only).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParamSource {
    /// Planar (XZ) speed of the entity's transform in meters per second, measured over the
    /// last fixed step (character controller, FSM movement, path following, tweens...).
    Speed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Number(f32),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParamDef {
    pub kind: ParamKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ParamValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ParamSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlendPoint {
    /// Parameter value at which this clip has full weight.
    pub at: f32,
    pub clip: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Motion {
    Clip {
        clip: String,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        speed: f32,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        looping: bool,
    },
    /// 1D blend tree: the two points around the float `parameter` blend linearly; clips are
    /// time-synchronized by normalized time (walk / run cycles stay in step).
    Blend1d {
        parameter: String,
        points: Vec<BlendPoint>,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        speed: f32,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        looping: bool,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StateDef {
    /// `None` holds the skeleton's rest pose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CondOp {
    Gt,
    Lt,
    Eq,
    Ne,
    True,
    False,
    Trigger,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub param: String,
    pub op: CondOp,
    /// Compared value of `gt` / `lt` / `eq` / `ne`.
    #[serde(default)]
    pub value: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransitionDef {
    /// Source state, or `*` for any state (never to itself).
    pub from: String,
    pub to: String,
    /// Every condition must pass.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
    /// Cross-fade seconds (0 = cut).
    #[serde(default)]
    pub duration: f32,
    /// Also require the source state's normalized time (1 = one full cycle) to reach this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_time: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ControllerDef {
    #[serde(default)]
    pub parameters: BTreeMap<String, ParamDef>,
    pub initial: String,
    pub states: BTreeMap<String, StateDef>,
    #[serde(default)]
    pub transitions: Vec<TransitionDef>,
}

/// The `animation.controllers` resource.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Controllers {
    #[serde(default)]
    pub controllers: BTreeMap<String, ControllerDef>,
}

/// The `animation.controller` component.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Attach {
    pub controller: String,
}

fn finite(x: f32, lo: f32, hi: f32) -> bool {
    x.is_finite() && (lo..=hi).contains(&x)
}

impl Controllers {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.controllers.len() <= MAX_CONTROLLERS,
            "at most {MAX_CONTROLLERS} animation controllers"
        );
        for (id, c) in &self.controllers {
            axiom_core::valid_id(id)?;
            c.validate().with_context(|| format!("controller {id}"))?;
        }
        Ok(())
    }
}

impl ParamDef {
    fn value(&self) -> f32 {
        match self.default {
            Some(ParamValue::Bool(b)) => f32::from(u8::from(b)),
            Some(ParamValue::Number(n)) => n,
            None => 0.0,
        }
    }
}

impl ControllerDef {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.parameters.len() <= MAX_PARAMS,
            "at most {MAX_PARAMS} parameters"
        );
        ensure!(
            (1..=MAX_STATES).contains(&self.states.len()),
            "1..{MAX_STATES} states"
        );
        ensure!(
            self.transitions.len() <= MAX_TRANSITIONS,
            "at most {MAX_TRANSITIONS} transitions"
        );
        for (name, p) in &self.parameters {
            axiom_core::valid_id(name)?;
            match (p.kind, p.default) {
                (ParamKind::Float, Some(ParamValue::Number(n))) => {
                    ensure!(n.is_finite(), "parameter {name}: default must be finite")
                }
                (ParamKind::Float, Some(ParamValue::Bool(_))) => {
                    anyhow::bail!("parameter {name}: a float default is a number")
                }
                (ParamKind::Bool | ParamKind::Trigger, Some(ParamValue::Number(_))) => {
                    anyhow::bail!("parameter {name}: a bool / trigger default is true or false")
                }
                _ => {}
            }
            ensure!(
                p.source.is_none() || p.kind == ParamKind::Float,
                "parameter {name}: only float parameters take a source"
            );
        }
        ensure!(
            self.states.contains_key(&self.initial),
            "unknown initial state {}",
            self.initial
        );
        for (name, s) in &self.states {
            axiom_core::valid_id(name)?;
            ensure!(
                name != "*",
                "'*' is reserved for transitions from any state"
            );
            match &s.motion {
                None => {}
                Some(Motion::Clip { clip, speed, .. }) => {
                    axiom_core::valid_id(clip).with_context(|| format!("state {name}: clip"))?;
                    ensure!(finite(*speed, 0.0, 8.0), "state {name}: speed 0..8");
                }
                Some(Motion::Blend1d {
                    parameter,
                    points,
                    speed,
                    ..
                }) => {
                    let p = self.parameters.get(parameter).with_context(|| {
                        format!("state {name}: unknown blend parameter {parameter}")
                    })?;
                    ensure!(
                        p.kind == ParamKind::Float,
                        "state {name}: blend parameter {parameter} must be a float"
                    );
                    ensure!(
                        (1..=MAX_POINTS).contains(&points.len()),
                        "state {name}: 1..{MAX_POINTS} blend points"
                    );
                    for point in points {
                        axiom_core::valid_id(&point.clip)
                            .with_context(|| format!("state {name}: blend clip"))?;
                        ensure!(
                            point.at.is_finite(),
                            "state {name}: blend point must be finite"
                        );
                    }
                    ensure!(
                        points.windows(2).all(|w| w[1].at > w[0].at),
                        "state {name}: blend points must be strictly increasing"
                    );
                    ensure!(finite(*speed, 0.0, 8.0), "state {name}: speed 0..8");
                }
            }
        }
        for (i, t) in self.transitions.iter().enumerate() {
            let at = || format!("transition {i}");
            ensure!(
                t.from == "*" || self.states.contains_key(&t.from),
                "{}: unknown state {}",
                at(),
                t.from
            );
            ensure!(
                self.states.contains_key(&t.to),
                "{}: unknown state {}",
                at(),
                t.to
            );
            ensure!(t.from != t.to, "{}: a state cannot go to itself", at());
            ensure!(
                finite(t.duration, 0.0, 10.0),
                "{}: duration 0..10 seconds",
                at()
            );
            ensure!(
                t.exit_time.is_none_or(|e| finite(e, 0.0, 1000.0)),
                "{}: exit_time 0..1000",
                at()
            );
            ensure!(
                !t.conditions.is_empty() || t.exit_time.is_some(),
                "{}: needs a condition or an exit_time (else it fires at once)",
                at()
            );
            ensure!(
                t.conditions.len() <= MAX_CONDITIONS,
                "{}: at most {MAX_CONDITIONS} conditions",
                at()
            );
            for c in &t.conditions {
                let p = self
                    .parameters
                    .get(&c.param)
                    .with_context(|| format!("{}: unknown parameter {}", at(), c.param))?;
                let ok = match c.op {
                    CondOp::Gt | CondOp::Lt | CondOp::Eq | CondOp::Ne => p.kind == ParamKind::Float,
                    CondOp::True | CondOp::False => p.kind == ParamKind::Bool,
                    CondOp::Trigger => p.kind == ParamKind::Trigger,
                };
                ensure!(
                    ok,
                    "{}: condition {:?} does not apply to {:?} parameter {}",
                    at(),
                    c.op,
                    p.kind,
                    c.param
                );
                ensure!(
                    c.value.is_finite(),
                    "{}: condition value must be finite",
                    at()
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Point {
    at: f32,
    clip: String,
    duration: f32,
}

#[derive(Clone, Debug)]
enum CMotion {
    Rest,
    Clip {
        clip: String,
        duration: f32,
        speed: f32,
        looping: bool,
    },
    Blend {
        param: usize,
        points: Vec<Point>,
        speed: f32,
        looping: bool,
    },
}

#[derive(Clone, Copy, Debug)]
struct CCond {
    param: usize,
    op: CondOp,
    value: f32,
}

#[derive(Clone, Debug)]
struct CTransition {
    from: Option<u16>,
    to: u16,
    conditions: Vec<CCond>,
    duration: f32,
    exit_time: Option<f32>,
}

/// A controller with names resolved and clip durations known (shared, read-only).
#[derive(Clone, Debug)]
pub struct Compiled {
    pub params: Vec<String>,
    kinds: Vec<ParamKind>,
    sources: Vec<Option<ParamSource>>,
    defaults: [f32; MAX_PARAMS],
    pub states: Vec<String>,
    motions: Vec<CMotion>,
    transitions: Vec<CTransition>,
    initial: u16,
}

/// One state's playback position: normalized time = `loops + frac`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub state: u16,
    pub loops: u32,
    pub frac: f32,
}

impl Layer {
    fn enter(state: u16) -> Self {
        Self {
            state,
            loops: 0,
            frac: 0.0,
        }
    }
    pub fn normalized(&self) -> f32 {
        self.loops as f32 + self.frac
    }
}

/// Per-entity controller state (`Copy`, so stepping and rollback never allocate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub current: Layer,
    /// The state being faded out (cross-fade in progress).
    pub previous: Option<Layer>,
    pub fade: f32,
    pub fade_duration: f32,
    pub params: [f32; MAX_PARAMS],
    pub last_transition: Option<u16>,
    /// Transitions fired since start.
    pub fired: u32,
}

/// One weighted clip sample (`point` = blend point, 0 for a clip, [`REST`] = rest pose).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub state: u16,
    pub point: u8,
    pub time: f32,
    pub weight: f32,
}

/// At most [`MAX_SAMPLES`] samples whose weights sum to 1 (fixed size, no allocation).
#[derive(Clone, Copy, Debug, Default)]
pub struct Samples {
    pub items: [Sample; MAX_SAMPLES],
    pub len: usize,
}

impl Samples {
    pub fn as_slice(&self) -> &[Sample] {
        &self.items[..self.len]
    }
    fn push(&mut self, s: Sample) {
        if self.len < MAX_SAMPLES && s.weight > 0.0 {
            self.items[self.len] = s;
            self.len += 1;
        }
    }
}

impl Compiled {
    /// Resolves `def` (validated first); `duration` gives a clip's length in seconds, `None`
    /// when the entity's player has no such clip (an error naming the state).
    pub fn compile(def: &ControllerDef, duration: &dyn Fn(&str) -> Option<f32>) -> Result<Self> {
        def.validate()?;
        let params: Vec<String> = def.parameters.keys().cloned().collect();
        let param = |name: &str| params.iter().position(|p| p == name);
        let mut defaults = [0.0; MAX_PARAMS];
        for (i, p) in def.parameters.values().enumerate() {
            defaults[i] = p.value();
        }
        let states: Vec<String> = def.states.keys().cloned().collect();
        let state = |name: &str| states.iter().position(|s| s == name).map(|i| i as u16);
        let clip_len = |state: &str, clip: &str| {
            duration(clip).with_context(|| {
                format!("state {state}: clip {clip} not found on the entity's animation.player")
            })
        };
        let mut motions = vec![];
        for (name, s) in &def.states {
            motions.push(match &s.motion {
                None => CMotion::Rest,
                Some(Motion::Clip {
                    clip,
                    speed,
                    looping,
                }) => CMotion::Clip {
                    duration: clip_len(name, clip)?,
                    clip: clip.clone(),
                    speed: *speed,
                    looping: *looping,
                },
                Some(Motion::Blend1d {
                    parameter,
                    points,
                    speed,
                    looping,
                }) => CMotion::Blend {
                    param: param(parameter).expect("validated"),
                    points: points
                        .iter()
                        .map(|p| {
                            Ok(Point {
                                at: p.at,
                                duration: clip_len(name, &p.clip)?,
                                clip: p.clip.clone(),
                            })
                        })
                        .collect::<Result<_>>()?,
                    speed: *speed,
                    looping: *looping,
                },
            });
        }
        let transitions = def
            .transitions
            .iter()
            .map(|t| CTransition {
                from: (t.from != "*").then(|| state(&t.from).expect("validated")),
                to: state(&t.to).expect("validated"),
                conditions: t
                    .conditions
                    .iter()
                    .map(|c| CCond {
                        param: param(&c.param).expect("validated"),
                        op: c.op,
                        value: c.value,
                    })
                    .collect(),
                duration: t.duration,
                exit_time: t.exit_time,
            })
            .collect();
        Ok(Self {
            kinds: def.parameters.values().map(|p| p.kind).collect(),
            sources: def.parameters.values().map(|p| p.source).collect(),
            params,
            defaults,
            initial: state(&def.initial).expect("validated"),
            states,
            motions,
            transitions,
        })
    }

    pub fn start(&self) -> State {
        State {
            current: Layer::enter(self.initial),
            previous: None,
            fade: 0.0,
            fade_duration: 0.0,
            params: self.defaults,
            last_transition: None,
            fired: 0,
        }
    }

    pub fn param(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p == name)
    }

    pub fn param_kind(&self, i: usize) -> ParamKind {
        self.kinds[i]
    }

    pub fn param_source(&self, i: usize) -> Option<ParamSource> {
        self.sources[i]
    }

    /// Typed value of parameter `i` (bool / trigger as bool).
    pub fn param_value(&self, s: &State, i: usize) -> ParamValue {
        match self.kinds[i] {
            ParamKind::Float => ParamValue::Number(s.params[i]),
            _ => ParamValue::Bool(s.params[i] != 0.0),
        }
    }

    /// Sets parameter `name` (a float takes a number; a bool / trigger takes true / false).
    pub fn set_param(&self, s: &mut State, name: &str, value: ParamValue) -> Result<()> {
        let i = self.param(name).with_context(|| {
            format!(
                "unknown parameter {name} (parameters: {})",
                self.params.join(", ")
            )
        })?;
        s.params[i] = match (self.kinds[i], value) {
            (ParamKind::Float, ParamValue::Number(n)) => {
                ensure!(n.is_finite(), "parameter {name} must be finite");
                n
            }
            (ParamKind::Float, ParamValue::Bool(_)) => {
                anyhow::bail!("parameter {name} is a float; pass a number")
            }
            (_, ParamValue::Bool(b)) => f32::from(u8::from(b)),
            (_, ParamValue::Number(_)) => {
                anyhow::bail!("parameter {name} is a bool / trigger; pass true or false")
            }
        };
        Ok(())
    }

    /// Weights of a blend tree's points: one point, or the two around `value` (clamped).
    fn blend(points: &[Point], value: f32) -> ([(usize, f32); 2], usize) {
        let last = points.len() - 1;
        if points.len() == 1 || value <= points[0].at {
            return ([(0, 1.0), (0, 0.0)], 1);
        }
        if value >= points[last].at {
            return ([(last, 1.0), (0, 0.0)], 1);
        }
        let b = points.partition_point(|p| p.at <= value);
        let a = b - 1;
        let t = (value - points[a].at) / (points[b].at - points[a].at);
        ([(a, 1.0 - t), (b, t)], 2)
    }

    /// Seconds one normalized cycle of `state` lasts now (0 = does not advance).
    fn cycle(&self, s: &State, state: u16) -> (f32, bool) {
        match &self.motions[state as usize] {
            CMotion::Rest => (1.0, true),
            CMotion::Clip {
                duration,
                speed,
                looping,
                ..
            } => (
                if *speed > 0.0 {
                    duration.max(FRAME) / speed
                } else {
                    0.0
                },
                *looping,
            ),
            CMotion::Blend {
                param,
                points,
                speed,
                looping,
            } => {
                let (w, n) = Self::blend(points, s.params[*param]);
                let d: f32 = w[..n]
                    .iter()
                    .map(|(i, w)| points[*i].duration.max(FRAME) * w)
                    .sum();
                (if *speed > 0.0 { d / speed } else { 0.0 }, *looping)
            }
        }
    }

    fn advance(&self, s: &State, layer: &mut Layer, dt: f32) {
        let (cycle, looping) = self.cycle(s, layer.state);
        if cycle <= 0.0 {
            return;
        }
        layer.frac += dt / cycle;
        if layer.frac >= 1.0 {
            if looping {
                let whole = layer.frac.floor();
                layer.loops = layer.loops.saturating_add(whole as u32);
                layer.frac -= whole;
            } else {
                layer.frac = 1.0;
            }
        }
    }

    fn passes(&self, s: &State, t: &CTransition) -> bool {
        if t.exit_time.is_some_and(|e| s.current.normalized() < e) {
            return false;
        }
        t.conditions.iter().all(|c| {
            let v = s.params[c.param];
            match c.op {
                CondOp::Gt => v > c.value,
                CondOp::Lt => v < c.value,
                CondOp::Eq => v == c.value,
                CondOp::Ne => v != c.value,
                CondOp::True | CondOp::Trigger => v != 0.0,
                CondOp::False => v == 0.0,
            }
        })
    }

    /// One fixed step of `dt` seconds: advance the playing state(s) and the cross-fade, then
    /// (when not fading) take the first passing transition. Returns its index.
    pub fn tick(&self, s: &mut State, dt: f32) -> Option<u16> {
        let mut current = s.current;
        self.advance(s, &mut current, dt);
        s.current = current;
        if let Some(mut previous) = s.previous {
            self.advance(s, &mut previous, dt);
            s.fade += dt;
            s.previous = (s.fade < s.fade_duration).then_some(previous);
        }
        if s.previous.is_some() {
            return None;
        }
        let i = self.transitions.iter().position(|t| {
            t.from.is_none_or(|f| f == s.current.state)
                && t.to != s.current.state
                && self.passes(s, t)
        })?;
        let t = &self.transitions[i];
        for c in &t.conditions {
            if c.op == CondOp::Trigger {
                s.params[c.param] = 0.0;
            }
        }
        s.previous = (t.duration > 0.0).then_some(s.current);
        s.fade = 0.0;
        s.fade_duration = t.duration;
        s.current = Layer::enter(t.to);
        s.last_transition = Some(i as u16);
        s.fired = s.fired.saturating_add(1);
        Some(i as u16)
    }

    fn layer_samples(&self, s: &State, layer: &Layer, weight: f32, out: &mut Samples) {
        let (_, looping) = self.cycle(s, layer.state);
        let phase = if looping {
            layer.frac
        } else {
            layer.normalized().min(1.0)
        };
        match &self.motions[layer.state as usize] {
            CMotion::Rest => out.push(Sample {
                state: layer.state,
                point: REST,
                time: 0.0,
                weight,
            }),
            CMotion::Clip { duration, .. } => out.push(Sample {
                state: layer.state,
                point: 0,
                time: phase * duration,
                weight,
            }),
            CMotion::Blend { param, points, .. } => {
                let (w, n) = Self::blend(points, s.params[*param]);
                for (i, pw) in &w[..n] {
                    out.push(Sample {
                        state: layer.state,
                        point: *i as u8,
                        time: phase * points[*i].duration,
                        weight: weight * pw,
                    });
                }
            }
        }
    }

    /// The weighted clip samples of the current pose (fading-out state first).
    pub fn samples(&self, s: &State) -> Samples {
        let mut out = Samples::default();
        let w = match s.previous {
            Some(previous) if s.fade_duration > 0.0 => {
                let w = (s.fade / s.fade_duration).clamp(0.0, 1.0);
                self.layer_samples(s, &previous, 1.0 - w, &mut out);
                w
            }
            _ => 1.0,
        };
        self.layer_samples(s, &s.current, w, &mut out);
        out
    }

    /// The clip a sample plays (`None` = rest pose).
    pub fn clip(&self, sample: &Sample) -> Option<&str> {
        match &self.motions[sample.state as usize] {
            CMotion::Rest => None,
            CMotion::Clip { clip, .. } => Some(clip),
            CMotion::Blend { points, .. } => points.get(sample.point as usize).map(|p| &*p.clip),
        }
    }

    /// `old` (of `from`) carried into this controller by state and parameter names (hot
    /// reload); a state that no longer exists restarts at the initial state.
    pub fn carry(&self, from: &Compiled, old: &State) -> State {
        let mut s = self.start();
        for (i, name) in self.params.iter().enumerate() {
            if let Some(j) = from.param(name) {
                if from.kinds[j] == self.kinds[i] {
                    s.params[i] = old.params[j];
                }
            }
        }
        let map = |l: &Layer| {
            let name = &from.states[l.state as usize];
            self.states
                .iter()
                .position(|n| n == name)
                .map(|state| Layer {
                    state: state as u16,
                    ..*l
                })
        };
        if let Some(current) = map(&old.current) {
            s.current = current;
            s.previous = old.previous.as_ref().and_then(map);
            s.fade = old.fade;
            s.fade_duration = old.fade_duration;
            s.fired = old.fired;
        }
        s
    }

    /// Compact observation of `s` (inspect / metrics / editor).
    pub fn describe(&self, s: &State) -> serde_json::Value {
        let params: BTreeMap<&str, ParamValue> = self
            .params
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), self.param_value(s, i)))
            .collect();
        serde_json::json!({
            "state": self.states[s.current.state as usize],
            "time": (s.current.normalized() * 1000.0).round() / 1000.0,
            "previous": s.previous.map(|p| &self.states[p.state as usize]),
            "fade": if s.previous.is_some() && s.fade_duration > 0.0 { ((s.fade / s.fade_duration).clamp(0.0, 1.0) * 1000.0).round() / 1000.0 } else { 1.0 },
            "params": params,
            "last_transition": s.last_transition,
            "transitions": s.fired,
        })
    }
}

/// Shortest cycle a clip counts as (one 120 Hz step), so zero-length clips still advance.
const FRAME: f32 = 1.0 / 120.0;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(v: serde_json::Value) -> ControllerDef {
        serde_json::from_value(v).unwrap()
    }
    fn locomotion() -> ControllerDef {
        def(json!({
            "parameters": {"speed": {"kind":"float","default":0.0}, "jump": {"kind":"trigger"}},
            "initial": "idle",
            "states": {
                "idle": {"motion": {"kind":"clip","clip":"idle"}},
                "move": {"motion": {"kind":"blend1d","parameter":"speed","points":[{"at":0.5,"clip":"walk"},{"at":3.0,"clip":"run"}]}},
                "hop": {"motion": {"kind":"clip","clip":"hop","looping":false}}
            },
            "transitions": [
                {"from":"idle","to":"move","conditions":[{"param":"speed","op":"gt","value":0.1}],"duration":0.25},
                {"from":"move","to":"idle","conditions":[{"param":"speed","op":"lt","value":0.1}],"duration":0.25},
                {"from":"*","to":"hop","conditions":[{"param":"jump","op":"trigger"}]},
                {"from":"hop","to":"idle","exit_time":1.0,"duration":0.1}
            ]
        }))
    }
    fn lengths(clip: &str) -> Option<f32> {
        match clip {
            "idle" => Some(2.0),
            "walk" => Some(1.0),
            "run" => Some(0.6),
            "hop" => Some(0.6),
            _ => None,
        }
    }
    const DT: f32 = 1.0 / 120.0;

    #[test]
    fn a_parameter_switches_state_with_a_cross_fade_and_back() {
        let c = Compiled::compile(&locomotion(), &lengths).unwrap();
        let mut s = c.start();
        assert_eq!(c.tick(&mut s, DT), None);
        c.set_param(&mut s, "speed", ParamValue::Number(1.0))
            .unwrap();
        assert_eq!(c.tick(&mut s, DT), Some(0));
        assert_eq!(c.states[s.current.state as usize], "move");
        // Half way through the 0.25 s fade both states are drawn.
        for _ in 0..15 {
            c.tick(&mut s, DT);
        }
        let samples = c.samples(&s);
        let total: f32 = samples.as_slice().iter().map(|x| x.weight).sum();
        assert!((total - 1.0).abs() < 1e-5, "{samples:?}");
        assert_eq!(c.clip(&samples.items[0]), Some("idle"));
        assert!((samples.items[0].weight - 0.5).abs() < 0.01, "{samples:?}");
        // speed 1.0 sits between walk (0.5) and run (3.0): both blend in the move layer.
        assert_eq!(samples.len, 3);
        for _ in 0..30 {
            c.tick(&mut s, DT);
        }
        assert!(s.previous.is_none());
        c.set_param(&mut s, "speed", ParamValue::Number(0.0))
            .unwrap();
        assert_eq!(c.tick(&mut s, DT), Some(1));
        assert_eq!(c.states[s.current.state as usize], "idle");
    }

    #[test]
    fn a_trigger_is_consumed_and_exit_time_returns() {
        let c = Compiled::compile(&locomotion(), &lengths).unwrap();
        let mut s = c.start();
        c.set_param(&mut s, "jump", ParamValue::Bool(true)).unwrap();
        assert_eq!(c.tick(&mut s, DT), Some(2));
        assert_eq!(s.params[c.param("jump").unwrap()], 0.0);
        // The one-shot hop (0.6 s) returns to idle once its normalized time reaches 1.
        let mut ticks = 0;
        while c.states[s.current.state as usize] == "hop" {
            c.tick(&mut s, DT);
            ticks += 1;
            assert!(ticks < 200);
        }
        assert!((70..=74).contains(&ticks), "{ticks}");
    }

    #[test]
    fn blend_weights_follow_the_parameter() {
        let c = Compiled::compile(&locomotion(), &lengths).unwrap();
        let mut s = c.start();
        s.current = Layer::enter(c.states.iter().position(|n| n == "move").unwrap() as u16);
        for (speed, walk) in [(0.0, 1.0), (0.5, 1.0), (1.75, 0.5), (3.0, 0.0), (9.0, 0.0)] {
            c.set_param(&mut s, "speed", ParamValue::Number(speed))
                .unwrap();
            let samples = c.samples(&s);
            let w = samples
                .as_slice()
                .iter()
                .filter(|x| c.clip(x) == Some("walk"))
                .map(|x| x.weight)
                .sum::<f32>();
            assert!((w - walk).abs() < 1e-5, "{speed}: {samples:?}");
        }
    }

    #[test]
    fn invalid_controllers_are_refused_with_reasons() {
        let mut d = locomotion();
        d.transitions[0].conditions[0].op = CondOp::True;
        let e = format!("{:#}", d.validate().unwrap_err());
        assert!(e.contains("does not apply"), "{e}");
        let mut d = locomotion();
        d.transitions[3].exit_time = None;
        assert!(format!("{:#}", d.validate().unwrap_err()).contains("exit_time"));
        let mut d = locomotion();
        d.initial = "nope".into();
        assert!(d.validate().is_err());
        let e = Compiled::compile(&locomotion(), &|c| (c != "run").then_some(1.0)).unwrap_err();
        assert!(format!("{e:#}").contains("clip run not found"), "{e:#}");
    }

    #[test]
    fn hot_reload_carries_state_and_parameters_by_name() {
        let c = Compiled::compile(&locomotion(), &lengths).unwrap();
        let mut s = c.start();
        c.set_param(&mut s, "speed", ParamValue::Number(2.0))
            .unwrap();
        c.tick(&mut s, DT);
        let mut d = locomotion();
        d.states.insert("crouch".into(), StateDef::default());
        let next = Compiled::compile(&d, &lengths).unwrap();
        let carried = next.carry(&c, &s);
        assert_eq!(next.states[carried.current.state as usize], "move");
        assert_eq!(carried.params[next.param("speed").unwrap()], 2.0);
    }

    #[test]
    fn json_round_trips_without_defaults() {
        let d = locomotion();
        let v = serde_json::to_value(&d).unwrap();
        assert!(v["states"]["idle"]["motion"].get("speed").is_none());
        assert_eq!(serde_json::from_value::<ControllerDef>(v).unwrap(), d);
    }
}
