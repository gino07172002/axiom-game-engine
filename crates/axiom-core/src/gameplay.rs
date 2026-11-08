//! Shared, bounded gameplay vocabulary. Authoring IDs resolve to indices before execution.
use crate::{valid_id, Diagnostic, Registry, SystemDescriptor, World};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_EVENTS: usize = 128;
pub const MAX_OPS: usize = 512;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Int(i64),
    Number(f64),
    Text(String),
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    Bool,
    Int,
    Number,
    Text,
}
impl Scalar {
    pub fn ty(&self) -> ScalarType {
        match self {
            Self::Bool(_) => ScalarType::Bool,
            Self::Int(_) => ScalarType::Int,
            Self::Number(_) => ScalarType::Number,
            Self::Text(_) => ScalarType::Text,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            match self {
                Self::Text(s) => s.len() <= 4096,
                Self::Number(n) => n.is_finite(),
                _ => true,
            },
            "invalid scalar"
        );
        Ok(())
    }
    pub fn display(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            _ => serde_json::to_string(self).unwrap(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operand {
    Literal { value: Scalar },
    Variable { id: String },
    Event { field: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    Always {},
    Eq { left: Operand, right: Operand },
    Ge { left: Operand, right: Operand },
    All { conditions: Vec<Condition> },
    Not { condition: Box<Condition> },
}
impl Default for Condition {
    fn default() -> Self {
        Self::Always {}
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GameAction {
    Set {
        variable: String,
        value: Operand,
    },
    Add {
        variable: String,
        value: Operand,
    },
    Emit {
        event: String,
        #[serde(default)]
        fields: BTreeMap<String, Operand>,
    },
    Delay {
        ticks: u32,
        event: String,
        #[serde(default)]
        fields: BTreeMap<String, Operand>,
    },
    GoTo {
        node: String,
    },
    Scene {
        scene: String,
    },
    End {
        ending: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventSchema {
    #[serde(default)]
    pub fields: BTreeMap<String, ScalarType>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub event: String,
    #[serde(default)]
    pub when: Condition,
    pub actions: Vec<GameAction>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Program {
    #[serde(default)]
    pub variables: BTreeMap<String, Scalar>,
    #[serde(default)]
    pub events: BTreeMap<String, EventSchema>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Rising edge action name -> no-payload event.
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub when: Condition,
    #[serde(default)]
    pub actions: Vec<GameAction>,
    pub next: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoryNode {
    pub id: String,
    pub speaker: String,
    pub text: String,
    #[serde(default)]
    pub on_enter: Vec<GameAction>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub ending: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Narrative {
    pub start: String,
    pub nodes: Vec<StoryNode>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeCommand {
    Call {
        capability: String,
        action: String,
        args: Value,
    },
    Choose {
        node: String,
        choice: String,
    },
    Emit {
        event: String,
        #[serde(default)]
        fields: BTreeMap<String, Scalar>,
    },
    Locale {
        locale: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiDocument {
    #[serde(default, skip_serializing_if = "is_false")]
    pub flow: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub session_controls: bool,
    pub locale: String,
    pub catalogs: BTreeMap<String, BTreeMap<String, String>>,
    pub widgets: Vec<Widget>,
    #[serde(default)]
    pub images: BTreeMap<String, String>,
    /// HUD placement: widget ID -> screen anchor. Placed widgets draw in compact floating
    /// panels over the 3D view; unplaced widgets keep the bottom document panel.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub regions: BTreeMap<String, Region>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}
fn yes() -> bool {
    true
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn is_true(v: &bool) -> bool {
    *v
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextBinding {
    Key { id: String },
    Variable { id: String },
    LocalizedVariable { id: String },
    Speaker {},
    Line {},
    Ending {},
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Widget {
    Text {
        id: String,
        binding: TextBinding,
        #[serde(default)]
        when: Condition,
    },
    Button {
        id: String,
        text: String,
        command: RuntimeCommand,
        #[serde(default)]
        when: Condition,
    },
    Choices {
        id: String,
    },
    Image {
        id: String,
        image: String,
    },
    Progress {
        id: String,
        variable: String,
        max: f64,
    },
}

#[derive(Clone, Debug)]
pub enum Read {
    Literal(Scalar),
    Variable(usize),
    Event(usize),
}
#[derive(Clone, Debug)]
pub enum Test {
    Always,
    Eq(Read, Read),
    Ge(Read, Read),
    All(Vec<Test>),
    Not(Box<Test>),
}
#[derive(Clone, Debug)]
pub enum Op {
    Set(usize, Read),
    Add(usize, Read),
    Emit(usize, Vec<Read>, u32),
    GoTo(usize),
    Scene(usize),
    End(String),
}
#[derive(Clone, Debug)]
pub struct CompiledRule {
    pub id: String,
    pub when: Test,
    pub ops: Vec<Op>,
}
#[derive(Clone, Debug)]
pub struct CompiledChoice {
    pub id: String,
    pub text: String,
    pub when: Test,
    pub ops: Vec<Op>,
    pub next: usize,
}
#[derive(Clone, Debug)]
pub struct CompiledNode {
    pub id: String,
    pub speaker: String,
    pub text: String,
    pub enter: Vec<Op>,
    pub choices: Vec<CompiledChoice>,
    pub ending: Option<String>,
}
#[derive(Clone, Debug)]
pub enum BoundText {
    Key(String),
    Variable(usize),
    LocalizedVariable(usize),
    Speaker,
    Line,
    Ending,
}
#[derive(Clone, Debug)]
pub enum BoundWidget {
    Text(String, BoundText, Test),
    Button(String, String, RuntimeCommand, Test),
    Choices(String),
    Image(String, String),
    Progress(String, usize, f64),
}
#[derive(Clone, Debug)]
pub struct CompiledGameplay {
    pub program: Program,
    pub variables: Vec<String>,
    pub initial: Vec<Scalar>,
    pub event_ids: Vec<String>,
    pub event_fields: Vec<Vec<(String, ScalarType)>>,
    pub rules: Vec<Vec<CompiledRule>>,
    pub nodes: Vec<CompiledNode>,
    pub start: Option<usize>,
    pub scenes: Vec<String>,
    pub inputs: Vec<(String, usize)>,
    pub ui: Option<UiDocument>,
    pub widgets: Vec<BoundWidget>,
}
struct Compiler<'a> {
    p: &'a Program,
    vars: Vec<String>,
    events: Vec<String>,
    nodes: Vec<String>,
    scenes: Vec<String>,
}
fn index(ids: &[String], id: &str) -> Result<usize> {
    ids.iter()
        .position(|v| v == id)
        .with_context(|| format!("missing reference {id}"))
}
impl Compiler<'_> {
    fn read_at(
        &self,
        o: &Operand,
        fields: &[(String, ScalarType)],
        path: &str,
    ) -> Result<(Read, ScalarType)> {
        let suffix = match o {
            Operand::Variable { .. } => "/id",
            Operand::Event { .. } => "/field",
            _ => "/value",
        };
        crate::local::at(&format!("{path}{suffix}"), self.read(o, fields))
    }
    fn test_at(
        &self,
        c: &Condition,
        f: &[(String, ScalarType)],
        path: &str,
        depth: usize,
    ) -> Result<Test> {
        crate::local::at(
            path,
            (|| -> Result<()> {
                ensure!(depth <= 16, "condition nesting budget exceeded");
                match c {
                    Condition::Eq { left, right } | Condition::Ge { left, right } => {
                        self.read_at(left, f, &format!("{path}/left"))?;
                        self.read_at(right, f, &format!("{path}/right"))?;
                    }
                    Condition::All { conditions } => {
                        for (i, c) in conditions.iter().enumerate() {
                            self.test_at(c, f, &format!("{path}/conditions/{i}"), depth + 1)?;
                        }
                    }
                    Condition::Not { condition } => {
                        self.test_at(condition, f, &format!("{path}/condition"), depth + 1)?;
                    }
                    _ => {}
                }
                Ok(())
            })(),
        )?;
        crate::local::at(path, self.test(c, f, depth))
    }
    fn ops_at(
        &self,
        actions: &[GameAction],
        fields: &[(String, ScalarType)],
        path: &str,
    ) -> Result<Vec<Op>> {
        crate::local::at(
            path,
            (|| -> Result<()> {
                ensure!(actions.len() <= 64, "action budget exceeded");
                Ok(())
            })(),
        )?;
        let mut out = vec![];
        for (i, a) in actions.iter().enumerate() {
            let path = format!("{path}/{i}");
            match a {
                GameAction::Set { variable, value } | GameAction::Add { variable, value } => {
                    crate::local::at(&format!("{path}/variable"), index(&self.vars, variable))?;
                    self.read_at(value, fields, &format!("{path}/value"))?;
                }
                GameAction::Emit { event, fields: f }
                | GameAction::Delay {
                    event, fields: f, ..
                } => {
                    crate::local::at(&format!("{path}/event"), index(&self.events, event))?;
                    for (id, o) in f {
                        self.read_at(
                            o,
                            fields,
                            &format!("{path}/fields/{}", crate::local::escape(id)),
                        )?;
                    }
                }
                GameAction::GoTo { node } => {
                    crate::local::at(&format!("{path}/node"), index(&self.nodes, node))?;
                }
                GameAction::Scene { scene } => {
                    crate::local::at(&format!("{path}/scene"), index(&self.scenes, scene))?;
                }
                _ => {}
            }
            out.extend(crate::local::at(
                &path,
                self.ops(std::slice::from_ref(a), fields),
            )?);
        }
        Ok(out)
    }
    fn read(&self, o: &Operand, fields: &[(String, ScalarType)]) -> Result<(Read, ScalarType)> {
        Ok(match o {
            Operand::Literal { value } => {
                value.validate()?;
                (Read::Literal(value.clone()), value.ty())
            }
            Operand::Variable { id } => {
                let i = index(&self.vars, id)?;
                (Read::Variable(i), self.p.variables[id].ty())
            }
            Operand::Event { field } => {
                let i = fields
                    .iter()
                    .position(|(id, _)| id == field)
                    .context("missing event field")?;
                (Read::Event(i), fields[i].1)
            }
        })
    }
    fn test(&self, c: &Condition, f: &[(String, ScalarType)], depth: usize) -> Result<Test> {
        ensure!(depth <= 16, "condition nesting budget exceeded");
        Ok(match c {
            Condition::Always {} => Test::Always,
            Condition::Eq { left, right } | Condition::Ge { left, right } => {
                let (a, ta) = self.read(left, f)?;
                let (b, tb) = self.read(right, f)?;
                ensure!(ta == tb, "condition operand type mismatch");
                if matches!(c, Condition::Ge { .. }) {
                    ensure!(
                        matches!(ta, ScalarType::Int | ScalarType::Number),
                        "ge requires numeric operands"
                    );
                    Test::Ge(a, b)
                } else {
                    Test::Eq(a, b)
                }
            }
            Condition::All { conditions } => {
                ensure!(conditions.len() <= 32, "condition budget exceeded");
                Test::All(
                    conditions
                        .iter()
                        .map(|c| self.test(c, f, depth + 1))
                        .collect::<Result<_>>()?,
                )
            }
            Condition::Not { condition } => {
                Test::Not(Box::new(self.test(condition, f, depth + 1)?))
            }
        })
    }
    fn ops(&self, actions: &[GameAction], fields: &[(String, ScalarType)]) -> Result<Vec<Op>> {
        ensure!(actions.len() <= 64, "action budget exceeded");
        actions
            .iter()
            .map(|a| {
                Ok(match a {
                    GameAction::Set { variable, value } | GameAction::Add { variable, value } => {
                        let i = index(&self.vars, variable)?;
                        let (v, t) = self.read(value, fields)?;
                        ensure!(
                            t == self.p.variables[variable].ty(),
                            "action type mismatch for {variable}"
                        );
                        if matches!(a, GameAction::Add { .. }) {
                            ensure!(
                                matches!(t, ScalarType::Int | ScalarType::Number),
                                "add requires numeric operands"
                            );
                            Op::Add(i, v)
                        } else {
                            Op::Set(i, v)
                        }
                    }
                    GameAction::Emit { event, fields: f }
                    | GameAction::Delay {
                        event, fields: f, ..
                    } => {
                        let i = index(&self.events, event)?;
                        let schema = &self.p.events[event].fields;
                        ensure!(schema.len() == f.len(), "event field count mismatch");
                        let values = schema
                            .iter()
                            .map(|(id, t)| {
                                let (v, ty) =
                                    self.read(f.get(id).context("missing event field")?, fields)?;
                                ensure!(*t == ty, "event field type mismatch");
                                Ok(v)
                            })
                            .collect::<Result<_>>()?;
                        let ticks = if let GameAction::Delay { ticks, .. } = a {
                            ensure!(*ticks > 0 && *ticks <= 1_000_000, "invalid delay ticks");
                            *ticks
                        } else {
                            0
                        };
                        Op::Emit(i, values, ticks)
                    }
                    GameAction::GoTo { node } => Op::GoTo(index(&self.nodes, node)?),
                    GameAction::Scene { scene } => Op::Scene(index(&self.scenes, scene)?),
                    GameAction::End { ending } => {
                        valid_id(ending)?;
                        Op::End(ending.clone())
                    }
                })
            })
            .collect()
    }
}
impl CompiledGameplay {
    pub fn compile(w: &World) -> Result<Self> {
        let p: Program = w.resource("behavior.program")?;
        ensure!(
            p.variables.len() <= 256
                && p.events.len() <= 128
                && p.rules.len() <= 256
                && p.inputs.len() <= 64,
            "behavior definition budget exceeded"
        );
        for (id, v) in &p.variables {
            valid_id(id)?;
            v.validate()?;
        }
        for (id, e) in &p.events {
            valid_id(id)?;
            ensure!(e.fields.len() <= 16, "event field budget exceeded");
            for k in e.fields.keys() {
                valid_id(k)?;
            }
        }
        let story: Option<Narrative> = w
            .resources
            .get("narrative.graph")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?;
        let c = Compiler {
            vars: p.variables.keys().cloned().collect(),
            events: p.events.keys().cloned().collect(),
            nodes: story
                .as_ref()
                .map_or(vec![], |s| s.nodes.iter().map(|n| n.id.clone()).collect()),
            scenes: w.scenes.iter().map(|s| s.id.clone()).collect(),
            p: &p,
        };
        let event_fields = c
            .events
            .iter()
            .map(|id| {
                p.events[id]
                    .fields
                    .iter()
                    .map(|(k, t)| (k.clone(), *t))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut rules = vec![vec![]; c.events.len()];
        let mut rule_ids = BTreeSet::new();
        for r in &p.rules {
            let path = crate::local::Target::Rule { id: r.id.clone() }.path();
            valid_id(&r.id)?;
            ensure!(rule_ids.insert(&r.id), "duplicate rule {}", r.id);
            let i = crate::local::at(&format!("{path}/event"), index(&c.events, &r.event))?;
            rules[i].push(CompiledRule {
                id: r.id.clone(),
                when: c.test_at(&r.when, &event_fields[i], &format!("{path}/when"), 0)?,
                ops: c.ops_at(&r.actions, &event_fields[i], &format!("{path}/actions"))?,
            });
        }
        let mut nodes = vec![];
        let mut start = None;
        if let Some(s) = &story {
            ensure!(
                !s.nodes.is_empty() && s.nodes.len() <= 256,
                "narrative node budget exceeded"
            );
            ensure!(
                c.nodes.iter().collect::<BTreeSet<_>>().len() == s.nodes.len(),
                "duplicate narrative node"
            );
            start = Some(crate::local::at(
                "/resources/narrative.graph/start",
                index(&c.nodes, &s.start),
            )?);
            for n in &s.nodes {
                let path = crate::local::Target::Node { id: n.id.clone() }.path();
                valid_id(&n.id)?;
                ensure!(n.choices.len() <= 16, "choice budget exceeded");
                ensure!(
                    n.choices
                        .iter()
                        .map(|c| &c.id)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == n.choices.len(),
                    "duplicate choice"
                );
                if let Some(e) = &n.ending {
                    valid_id(e)?;
                    ensure!(n.choices.is_empty(), "ending node cannot contain choices");
                }
                nodes.push(CompiledNode {
                    id: n.id.clone(),
                    speaker: n.speaker.clone(),
                    text: n.text.clone(),
                    enter: c.ops_at(&n.on_enter, &[], &format!("{path}/on_enter"))?,
                    ending: n.ending.clone(),
                    choices: n
                        .choices
                        .iter()
                        .map(|choice| {
                            let path = crate::local::Target::Choice {
                                node: n.id.clone(),
                                id: choice.id.clone(),
                            }
                            .path();
                            valid_id(&choice.id)?;
                            Ok(CompiledChoice {
                                id: choice.id.clone(),
                                text: choice.text.clone(),
                                when: c.test_at(&choice.when, &[], &format!("{path}/when"), 0)?,
                                ops: c.ops_at(&choice.actions, &[], &format!("{path}/actions"))?,
                                next: crate::local::at(
                                    &format!("{path}/next"),
                                    index(&c.nodes, &choice.next),
                                )?,
                            })
                        })
                        .collect::<Result<_>>()?,
                });
            }
        }
        let bindings: crate::InputBindings = w
            .resources
            .get("input.bindings")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?
            .unwrap_or_default();
        let inputs = p
            .inputs
            .iter()
            .map(|(action, event)| {
                ensure!(
                    bindings.buttons.contains_key(action) || bindings.axes.contains_key(action),
                    "unknown input action {action}"
                );
                let e = crate::local::at(
                    &format!(
                        "/resources/behavior.program/inputs/{}",
                        crate::local::escape(action)
                    ),
                    index(&c.events, event),
                )?;
                ensure!(
                    event_fields[e].is_empty(),
                    "input event must have no fields"
                );
                Ok((action.clone(), e))
            })
            .collect::<Result<_>>()?;
        let ui: Option<UiDocument> = w
            .resources
            .get("ui.document")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?;
        let mut widgets = vec![];
        if let Some(u) = &ui {
            ensure!(
                u.catalogs.contains_key(&u.locale)
                    && u.catalogs.len() <= 16
                    && u.widgets.len() <= 64
                    && u.images.len() <= 16,
                "invalid UI locale or budget"
            );
            let catalog = &u.catalogs[&u.locale];
            ensure!(catalog.len() <= 2048, "localization key budget exceeded");
            for (lang, cat) in &u.catalogs {
                valid_id(lang)?;
                ensure!(
                    cat.keys().eq(catalog.keys()) && cat.values().all(|v| v.len() <= 8192),
                    "localization keys must match with bounded strings"
                );
            }
            let key = |k: &str| -> Result<()> {
                ensure!(catalog.contains_key(k), "missing localization key {k}");
                Ok(())
            };
            for n in &nodes {
                let path = crate::local::Target::Node { id: n.id.clone() }.path();
                crate::local::at(&format!("{path}/speaker"), key(&n.speaker))?;
                crate::local::at(&format!("{path}/text"), key(&n.text))?;
                for choice in &n.choices {
                    crate::local::at(
                        &format!("{path}/choices/@{}/text", crate::local::escape(&choice.id)),
                        key(&choice.text),
                    )?;
                }
            }
            let mut ids = BTreeSet::new();
            for widget in &u.widgets {
                let (id, bound) = match widget {
                    Widget::Text { id, binding, when } => {
                        let b = match binding {
                            TextBinding::Key { id } => {
                                key(id)?;
                                BoundText::Key(id.clone())
                            }
                            TextBinding::Variable { id } => {
                                BoundText::Variable(index(&c.vars, id)?)
                            }
                            TextBinding::LocalizedVariable { id } => {
                                let i = index(&c.vars, id)?;
                                let Scalar::Text(k) = &p.variables[id] else {
                                    anyhow::bail!("localized variable requires text")
                                };
                                key(k)?;
                                BoundText::LocalizedVariable(i)
                            }
                            TextBinding::Speaker {} => {
                                ensure!(start.is_some(), "speaker binding requires narrative");
                                BoundText::Speaker
                            }
                            TextBinding::Line {} => {
                                ensure!(start.is_some(), "line binding requires narrative");
                                BoundText::Line
                            }
                            TextBinding::Ending {} => BoundText::Ending,
                        };
                        (id, BoundWidget::Text(id.clone(), b, c.test(when, &[], 0)?))
                    }
                    Widget::Button {
                        id,
                        text,
                        command,
                        when,
                    } => {
                        key(text)?;
                        validate_command(command, &p, story.as_ref(), Some(u))?;
                        (
                            id,
                            BoundWidget::Button(
                                id.clone(),
                                text.clone(),
                                command.clone(),
                                c.test(when, &[], 0)?,
                            ),
                        )
                    }
                    Widget::Choices { id } => {
                        ensure!(start.is_some(), "choices binding requires narrative");
                        (id, BoundWidget::Choices(id.clone()))
                    }
                    Widget::Image { id, image } => {
                        ensure!(u.images.contains_key(image), "missing UI image {image}");
                        (id, BoundWidget::Image(id.clone(), image.clone()))
                    }
                    Widget::Progress { id, variable, max } => {
                        let i = index(&c.vars, variable)?;
                        ensure!(
                            matches!(
                                p.variables[variable].ty(),
                                ScalarType::Int | ScalarType::Number
                            ) && max.is_finite()
                                && *max > 0.0,
                            "invalid progress binding"
                        );
                        (id, BoundWidget::Progress(id.clone(), i, *max))
                    }
                };
                valid_id(id)?;
                ensure!(ids.insert(id), "duplicate widget id");
                widgets.push(bound);
            }
            for id in u.regions.keys() {
                ensure!(
                    ids.contains(id),
                    "ui.document region refers to unknown widget {id}"
                );
            }
            for path in u.images.values() {
                ensure!(
                    path.len() <= 1024
                        && std::path::Path::new(path)
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("png")),
                    "UI image must reference PNG"
                );
            }
        }
        Ok(Self {
            initial: c.vars.iter().map(|id| p.variables[id].clone()).collect(),
            variables: c.vars.clone(),
            event_ids: c.events.clone(),
            event_fields,
            rules,
            nodes,
            start,
            scenes: c.scenes.clone(),
            inputs,
            ui,
            widgets,
            program: p,
        })
    }
}
pub fn validate_command(
    command: &RuntimeCommand,
    p: &Program,
    n: Option<&Narrative>,
    ui: Option<&UiDocument>,
) -> Result<()> {
    match command {
        RuntimeCommand::Call {
            capability,
            action,
            args,
        } => {
            crate::valid_id(capability)?;
            crate::valid_id(action)?;
            ensure!(args.is_object(), "capability arguments must be an object");
        }
        RuntimeCommand::Emit { event, fields } => {
            let s = p.events.get(event).context("unknown event")?;
            ensure!(
                s.fields.len() == fields.len(),
                "event payload field count mismatch"
            );
            for (k, t) in &s.fields {
                let v = fields.get(k).context("missing event payload field")?;
                v.validate()?;
                ensure!(v.ty() == *t, "event payload type mismatch");
            }
        }
        RuntimeCommand::Choose { node, choice } => {
            let node = n
                .context("narrative missing")?
                .nodes
                .iter()
                .find(|n| &n.id == node)
                .context("unknown narrative node")?;
            ensure!(
                node.choices.iter().any(|c| &c.id == choice),
                "unknown choice"
            );
        }
        RuntimeCommand::Locale { locale } => ensure!(
            ui.is_some_and(|u| u.catalogs.contains_key(locale)),
            "unknown locale"
        ),
    }
    Ok(())
}
pub fn register(r: &mut Registry) {
    r.event_consumers.insert("behavior.execute".into());
    r.resource_mergers
        .insert("behavior.program".into(), |a, b| {
            crate::package::merge_resource(a, b, &["variables", "events", "inputs"], &["rules"])
        });
    for id in ["behavior", "narrative", "ui", "save"] {
        r.capabilities.insert(id.into(), 1);
    }
    r.resource("behavior.program","behavior",json!({"variables":"id -> bool/i64/f64/string defaults","events":"id -> typed payload fields","rules":"event + condition + actions","inputs":"rising-edge action -> event"}),|v|{let _:Program=serde_json::from_value(v.clone())?;Ok(())});
    r.resource("narrative.graph","narrative",json!({"start":"node ID","nodes":"id,speaker/text localization keys,on_enter,conditional choices,next,ending"}),|v|{let _:Narrative=serde_json::from_value(v.clone())?;Ok(())});
    r.resource("ui.document","ui",json!({"locale":"default catalog","catalogs":"locale -> key -> text","widgets":"text binding / button command / choices / image / progress","images":"asset ID -> PNG path"}),|v|{let _:UiDocument=serde_json::from_value(v.clone())?;Ok(())});
    r.systems.insert(
        "behavior.execute".into(),
        SystemDescriptor {
            id: "behavior.execute".into(),
            capability: "behavior".into(),
            phase: 5,
            reads: vec!["behavior.program".into(), "narrative.graph".into()],
            writes: vec![
                "runtime.variables".into(),
                "runtime.events".into(),
                "runtime.scenes".into(),
            ],
            after: vec![],
        },
    );
    r.world_validators.push(|w| {
        let has = w.resources.contains_key("behavior.program");
        if has
            || w.resources.contains_key("narrative.graph")
            || w.resources.contains_key("ui.document")
        {
            ensure!(
                has && w.systems.iter().any(|s| s == "behavior.execute"),
                "gameplay resources require behavior.program and behavior.execute"
            );
            CompiledGameplay::compile(w).map_err(|e| {
                if e.downcast_ref::<Diagnostic>().is_some() {
                    e
                } else {
                    Diagnostic::error("invalid_gameplay", "/resources", format!("{e:#}"))
                }
            })?;
        } else {
            ensure!(
                !w.systems.iter().any(|s| s == "behavior.execute"),
                "behavior.execute requires behavior.program"
            );
        }
        Ok(())
    });
    r.topics.insert("gameplay".into(),json!({"conditions":["always","eq","ge","all","not"],"operands":["literal","variable","event"],"actions":["set","add","emit","delay","go_to","scene","end"],"commands":["choose","emit","locale"],"limits":{"ops_per_tick":MAX_OPS,"queued_events":MAX_EVENTS,"condition_depth":16},"save":"checkpoint v1; strict content compatibility; authoring history separate","references":"variable/event/node/scene/UI binding IDs checked at compile time"}));
}
