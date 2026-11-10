//! Axiom Command Language (ACL): a terse, line-oriented front end for AI agents.
//!
//! ACL never bypasses the typed protocol. Every statement compiles to an ordinary
//! `Request`/`CoreEdit`, so transactions, CAS, receipts, diffs and undo are unchanged.
//! A script is either one read statement, or any number of edit statements that commit
//! together as one revision (compiled against, and CAS-checked at, the current revision).
use crate::local::{Collection, Format, Op, Predicate, Selection};
use crate::*;
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};

pub const GRAMMAR: &str = "\
ACL: one statement per line; # comments. Edits in one script = one revision.
READ (alone in a script):
  select SEL [where F OP V [and ...]] [fields F,F] [limit N] [cursor C]
  inspect ID                      full compiled record of one entity
  history [N] | diff REV REV | undo REV
  conflicts PROPOSAL              fields a pending proposal and newer revisions both changed
                                  (approve with resolve_proposal {keep:[PATH..]}: listed keep the current value)
  assets [KIND]                   models, images, materials, shaders, prefabs, machines, audio, ui, input
  usages KIND:ID                  every field that names the asset (Find Usages; e.g. usages model:hero)
EDIT (any number, atomic):
  set SEL F VALUE [where F OP V [and ...]]
  spawn PREFAB [count=N] [id=enemy/{i}] [from=0] [at=x,y,z] [step=x,y,z] [tag=a,b] [label=TEXT]
  delete SEL
  remove SEL COMPONENT            remove an authored component (not one inherited from a prefab)
 editor operations (same code as the editor UI; P = [id=ID] [at=x,y,z] [parent=ID] [scene=ID]):
  create empty|cube|camera P      new object; default id = preset name, then NAME/2, NAME/3..
                                  (P also takes rot=x,y,z: Euler degrees XYZ, like the Inspector)
  create light directional|point|spot|area P   render.light aimed along local -Z (id light.KIND)
  create trigger box|sphere P      trigger.zone for tag player, no events yet (id trigger.SHAPE)
  create path P                   path.spline: closed 8x6 m rectangle on local XZ (id path)
  follow SEL PATH [speed=M/S] [end=loop|ping_pong|stop] [offset=M]   path.follow on each object;
                                  several followers start evenly spaced along the path
  path ID add [x,y,z] | path ID insert I [x,y,z] | path ID move I x,y,z | path ID remove I
                                  edit points (path-local); no x,y,z = extend the end / split the piece
  create obstacle P               1 m cube + nav.blocker (agents walk around it; id obstacle)
  nav SEL agent [speed=M/S] [radius=M]   nav.agent on each object (no target yet)
  nav SEL target x,y,z|ENTITY|none       authored walk target (runtime: call nav.go)
  interact SEL [event=E] [prompt=TEXT] [range=M] [action=BUTTON]   interact.target on each object
                                  (defaults: event interacted declared with interactor/target, action interact -> KeyE)
  audio clip ID tone|engine|fountain|creak|footstep   procedural clip in audio.library (no audio files)
  audio SEL source [clip=ID] [volume=V] [loop=on|off] [autoplay=on|off] [spatial=on|off] [min=M] [max=M]
        [rolloff=inverse|linear]  audio.source on each object (default clip: the first; none -> clip tone)
  audio ID listener               audio.listener on ID; every other listener becomes inactive
                                  (change the clip: set ID audio.source/clip \"ID\"; play: call audio.play)
  tween SEL clip NAME [property=P] [target=ID] [duration=S|auto] [mode=once|loop|ping_pong] [speed=X]
        [autoplay=on|off] [on_finish=E] [play_on=E]   new tween clip; first track records P (default
                                  core.transform/position) of target (default the object) at 0 and at the end (default 1 s);
                                  play_on=E: runtime.bindings plays it when event E fires (e.g. the interact event)
  tween ID clip NAME set [duration=..] [mode=..] [speed=..] [autoplay=..] [on_finish=E|\"\"] [play_on=E] | tween ID clip NAME remove
  tween ID track CLIP add P [target=ID] | tween ID track CLIP remove I
  tween ID key CLIP TRACK add T [V] [ease=linear|step|ease_in|ease_out|ease_in_out]   no V = record the current value
  tween ID key CLIP TRACK move I T | tween ID key CLIP TRACK remove I     (V: 90 | x,y,z)
  instantiate PREFAB P            one instance, default id = prefab id (+/2..); spawn = bulk, tags, labels
  duplicate SEL                   copies keep parent+scenes; ids SRC/copy, SRC/copy/2..
                                  (entities[id=a,id=b] keeps that order; otherwise ID order)
  group SEL [id=ID]               new empty parent (default id group, group/2..) for SEL, world poses kept;
                                  under their shared parent at their mean position, else at the root
  reparent ID PARENT|root [keep=world|local]   default keeps world pose (editor drag)
  scene create ID [on|off] | scene activate ID on|off | scene delete ID (objects kept)
  scene member SCENE SEL on|off
  camera main ID                  active=true on ID, false on every other camera
  camera SEL follow TARGET [distance=M] [min=M] [max=M] [yaw=DEG] [pitch=DEG] [height=M] [orbit=AXIS]
        [zoom=AXIS] [radius=M] [collide=on|off]   camera.follow on each camera: orbits TARGET, stops in
                                  front of static blockers (runtime: call camera.follow.set / camera.activate)
  camera SEL blocker on|off       camera.blocker on each mesh: its parts block follow cameras
  migrate camera_blocker          deprecated tag camera_blocker -> camera.blocker component (verify lists it)
  rename ID LABEL                 display label (\"quoted\" for spaces/CJK); \"\" clears it
  move ID before OTHER|after OTHER|end   reorder (authored order = sibling order)
  prefab create [PID] from ID     ID becomes an instance of new prefab PID (default: ID, ID/2..);
                                  prefab gets ID's resolved components, ID keeps its core.transform
  ground SEL                      drop to the highest render.mesh top below (else y=0); moves position.y
  rotate SEL x,y,z                core.transform rotation from Euler degrees (XYZ)
  tag SEL +TAG -TAG ..            add / remove entity tags (other tags and their order kept)
  add SEL COMPONENT [JSON]        component with JSON or its default, + the capability/systems it needs
  append SEL F VALUE | insert SEL F INDEX VALUE | remove SEL F INDEX | reorder SEL F FROM TO
                                  one array element (set F replaces the whole array; it warns when
                                  resource list items are dropped)
  reset SEL COMPONENT             component back to its default (prefab instance: drop the override)
  event declare NAME [FIELD=text|number|int|bool ..]   declare a behavior.program event
 ui.document (UI Editor tab):
  ui place WIDGET REGION|none     top_left|top|top_right|left|center|right|bottom_left|bottom|bottom_right
  ui style WIDGET STYLE|none      title|heading|banner|value|label|muted (text and progress widgets)
  ui button WIDGET emit EVENT [{FIELDS}] | locale L | choose NODE CHOICE | call CAP ACTION [{ARGS}]
  ui effect add ID WIDGET KIND [on=show|hide|change|event|when] [event=E] [when={COND}] [ticks=N]
        [ease=E] [direction=left|right|up|down] [amount=X] [from=X]   KIND: fade_in|fade_out|slide_in|
        slide_out|scale_in|scale_out|pulse|shake|number_roll (+ system ui.effects when missing)
  ui effect set ID [kind=..] [widget=..] [same options; OPTION=default clears] | ui effect remove ID
 tuning editors (Input Map, Localization, Animation, VFX tabs; material shader source):
  input button ACTION KEY | input axis ACTION POSITIVE NEGATIVE | input remove ACTION
                                  KEY: winit names KeyW Space Digit1 ShiftLeft ArrowUp .. or MouseLeft
  loc set LOCALE KEY TEXT | loc key add KEY [TEXT] | loc key remove KEY   (new key: TEXT in the
                                  default locale, empty elsewhere) | loc locale add L [from=L2] | loc locale remove L
  ability ID event add TICK SOUND | ability ID event move I TICK | ability ID event sound I SOUND
        | ability ID event remove I   action.abilities timeline (TICK < duration, SOUND = audio.library cue)
  vfx ID size START END | vfx ID color r,g,b,a r,g,b,a   vfx.library size curve / colour gradient over life
  vfx ID size_curve T:S T:S .. | none     2..8 keys (t 0..1 increasing) replace size->end_size; none removes
  vfx ID gradient T:r,g,b,a .. | none     2..8 stops replace color->end_color; none removes
  shader ID source \"WGSL\"         custom shader surface code (JSON string; refused with line N if invalid)
 state machines (Graphs editor; M = fsm.machines machine; graph positions live in editor.layout):
  fsm M create [INITIAL] | fsm M remove        one stop state (default idle) | refused while used
  fsm M state add NAME [JSON] | fsm M state remove NAME | fsm M state rename OLD NEW
  fsm M state set NAME JSON                    whole state {\"action\":{\"kind\":..},\"emit\":E,\"clip\":C}
  fsm M initial NAME
  fsm M transition add FROM|* TO [JSON] | fsm M transition remove I | fsm M transition move I J
  fsm M transition set I JSON                  whole {\"from\",\"to\",\"when\":{\"kind\":..}}; first passing wins
                                  (state remove drops its transitions; new transitions wait 120 ticks)
  fsm M node STATE x,y            graph position (editor metadata, ignored by the runtime)
  fsm SEL agent MACHINE [radius=M]   fsm.agent on each object (a missing machine is created)
  behavior SEL patrol|guard|dialogue [machine=ID] [tag=T] [range=M] [speed=M/S] [event=E]
                                  new machine from the template + fsm.agent (dialogue declares E)
  rules: append|remove|reorder resources[id=behavior.program] rules ..; set rules[id=ID] when|actions|event V
 assets (Assets browser):
  asset place model|prefab|material:ID P   new object: model -> animation.player, material -> cube
  asset create material ID [shader=lit|unlit|toon|ID] [tint=r,g,b,a] | asset create prefab ID [from=ENTITY]
  asset create machine ID [initial=STATE] | asset create ui [locale=L] | asset create input
  asset rename KIND:ID NEW        new ID + every reference `usages` lists, one revision
 world (World panel):
  world id ID | world title TEXT
  capability add|remove ID        installed capability; remove refuses while components/resources/systems use it
  system add|remove ID            scheduled system (+ its capability and the systems it runs after)
  resource create ID [JSON]       registered resource with JSON or its default (+ capability/systems)
  resource remove ID
  Later statements see earlier ones (create cube; create cube -> cube, cube/2).
SEL: enemy/7 | enemy/* | entities[tag=enemy,has=core.health,id=boss*] | prefabs | resources[id=x]
  collections: entities resources prefabs scenes nodes choices rules widgets variables
F (field): /raw/json/pointer | label | core.health/value (first segment with a dot = component)
OP: < <= > >= == != (= is ==).  VALUE: JSON literal (42, \"text\", true, [1,2,3], {..}) or bare word.
Output: compact lines; edits print `ok rev=R changes=N` then up to 12 changes.";

const COLLECTIONS: [(&str, Collection); 9] = [
    ("entities", Collection::Entities),
    ("resources", Collection::Resources),
    ("prefabs", Collection::Prefabs),
    ("scenes", Collection::Scenes),
    ("nodes", Collection::Nodes),
    ("choices", Collection::Choices),
    ("rules", Collection::Rules),
    ("widgets", Collection::Widgets),
    ("variables", Collection::Variables),
];

fn diag(line: usize, message: impl std::fmt::Display) -> anyhow::Error {
    Diagnostic::error(
        "acl_syntax",
        format!("/line/{line}"),
        format!("line {line}: {message}"),
    )
}

/// Splits on whitespace, keeping quoted strings and bracketed/braced groups intact.
fn tokenize(line: &str) -> Result<Vec<String>> {
    let mut out = vec![];
    let mut cur = String::new();
    let (mut depth, mut quote, mut escape) = (0i32, false, false);
    for c in line.chars() {
        if quote {
            cur.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                quote = false;
            }
            continue;
        }
        match c {
            '"' => {
                quote = true;
                cur.push(c);
            }
            '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ']' | '}' => {
                depth -= 1;
                ensure!(depth >= 0, "unbalanced '{c}'");
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    ensure!(!quote, "unterminated string");
    ensure!(depth == 0, "unbalanced brackets");
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// Splits a line on `;` outside quotes and brackets so one-liners can hold several statements.
fn statements_in(line: &str) -> Vec<&str> {
    let mut out = vec![];
    let (mut depth, mut quote, mut escape, mut start) = (0i32, false, false, 0);
    for (i, c) in line.char_indices() {
        if quote {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                quote = false;
            }
            continue;
        }
        match c {
            '"' => quote = true,
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            ';' if depth == 0 => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out
}

fn value(token: &str) -> Value {
    serde_json::from_str(token).unwrap_or_else(|_| Value::String(token.to_owned()))
}

/// A token that must be JSON (resource and component bodies), unlike `value`'s bare words.
fn json_value(token: &str) -> Result<Value> {
    serde_json::from_str(token).with_context(|| format!("'{token}' is not valid JSON"))
}

/// Short field syntax → JSON pointer. A first segment containing a dot names a component.
pub fn field(f: &str, from: Collection) -> String {
    if f.starts_with('/') {
        return f.to_owned();
    }
    let component_root = matches!(from, Collection::Entities | Collection::Prefabs)
        && f.split('/').next().is_some_and(|s| s.contains('.'));
    if component_root {
        format!("/components/{f}")
    } else {
        format!("/{f}")
    }
}

fn selector(token: &str) -> Result<Selection> {
    if let Some((name, rest)) = token.split_once('[') {
        let filters = rest
            .strip_suffix(']')
            .context("selector filter must end with ']'")?;
        let from = COLLECTIONS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| *c)
            .with_context(|| format!("unknown collection '{name}'"))?;
        let mut s = Selection::all(from);
        for part in filters.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (k, v) = part
                .split_once('=')
                .with_context(|| format!("filter '{part}' must be key=value"))?;
            match k.trim() {
                "id" if v.contains(['*', '?']) => s.id_glob = Some(v.into()),
                "id" => s.ids.push(v.into()),
                "tag" => s.tags.push(v.into()),
                "has" => s.components.push(v.into()),
                "parent" => s.parent = Some(v.into()),
                other => bail!("unknown filter '{other}' (use id, tag, has, parent)"),
            }
        }
        return Ok(s);
    }
    if let Some((_, c)) = COLLECTIONS.iter().find(|(n, _)| *n == token) {
        return Ok(Selection::all(*c));
    }
    let mut s = Selection::all(Collection::Entities);
    if token.contains(['*', '?']) {
        s.id_glob = Some(token.into());
    } else {
        s.ids.push(token.into());
    }
    Ok(s)
}

fn clause(tokens: &[String], i: &mut usize, from: Collection) -> Result<Predicate> {
    const OPS: [(&str, Op); 7] = [
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("!=", Op::Ne),
        ("==", Op::Eq),
        ("<", Op::Lt),
        (">", Op::Gt),
        ("=", Op::Eq),
    ];
    let t = &tokens[*i];
    *i += 1;
    // Compact form: hp<50. Earliest operator position wins; longer symbols first at a tie.
    let found = OPS
        .iter()
        .filter_map(|(sym, op)| t.find(sym).map(|p| (p, sym, *op)))
        .min_by_key(|(p, sym, _)| (*p, usize::MAX - sym.len()));
    if let Some((p, sym, op)) = found.filter(|(p, _, _)| *p > 0) {
        let rhs = &t[p + sym.len()..];
        ensure!(!rhs.is_empty(), "missing value after '{sym}'");
        return Ok(Predicate {
            field: field(&t[..p], from),
            op,
            value: value(rhs),
        });
    }
    // Spaced form: hp < 50
    let sym = tokens.get(*i).context("expected an operator")?;
    let op = OPS
        .iter()
        .find(|(s, _)| s == sym)
        .map(|(_, op)| *op)
        .with_context(|| format!("unknown operator '{sym}'"))?;
    let v = tokens.get(*i + 1).context("expected a value")?;
    *i += 2;
    Ok(Predicate {
        field: field(t, from),
        op,
        value: value(v),
    })
}

fn where_clauses(tokens: &[String], i: &mut usize, s: &mut Selection) -> Result<()> {
    loop {
        s.filter.push(clause(tokens, i, s.from)?);
        if tokens.get(*i).map(String::as_str) == Some("and") {
            *i += 1;
        } else {
            return Ok(());
        }
    }
}

enum Statement {
    Select(Box<Selection>, Vec<(String, String)>),
    Inspect(String),
    History(usize),
    Diff(i64, i64),
    Undo(i64),
    Conflicts(i64),
    Set(Box<Selection>, String, Value),
    Spawn(Vec<(String, String)>, String),
    Delete(Box<Selection>),
    Remove(Box<Selection>, String),
    Create(String, Box<crate::ops::Placement>),
    Instantiate(String, Box<crate::ops::Placement>),
    Duplicate(Box<Selection>),
    Group(Box<Selection>, Option<String>),
    Reparent(String, Option<String>, bool),
    SceneCreate(String, bool),
    SceneActive(String, bool),
    SceneDelete(String),
    SceneMember(String, Box<Selection>, bool),
    CameraMain(String),
    CameraFollow(Box<Selection>, String, Box<crate::ops::CameraFollowOptions>),
    CameraBlocker(Box<Selection>, bool),
    MigrateCameraBlockers,
    Rename(String, String),
    Move(String, crate::ops::Order),
    PrefabCreate(Option<String>, String),
    Ground(Box<Selection>),
    Follow(Box<Selection>, String, crate::ops::FollowOptions),
    PathPoint(String, crate::ops::PointEdit),
    NavAgent(Box<Selection>, crate::ops::NavOptions),
    NavTarget(Box<Selection>, Option<crate::navigation::NavTarget>),
    Interact(Box<Selection>, crate::ops::InteractOptions),
    AudioClip(String, String),
    AudioSource(Box<Selection>, Box<crate::ops::AudioSourceOptions>),
    AudioListener(String),
    TweenClip(Box<Selection>, String, Box<crate::ops::TweenClipOptions>),
    TweenClipSet(String, String, Box<crate::ops::TweenClipOptions>),
    TweenClipRemove(String, String),
    TweenTrackAdd(String, String, String, String),
    TweenTrackRemove(String, String, usize),
    TweenKey(String, String, usize, crate::ops::KeyEdit),
    Tag(Box<Selection>, Vec<String>, Vec<String>),
    WorldId(String),
    WorldTitle(String),
    CapabilityAdd(String),
    CapabilityRemove(String),
    SystemAdd(String),
    SystemRemove(String),
    ResourceCreate(String, Option<Value>),
    ResourceRemove(String),
    AddComponent(Box<Selection>, String, Option<Value>),
    Array(Box<Selection>, String, crate::ops::ArrayEdit),
    Reset(Box<Selection>, String),
    EventDeclare(String, Vec<(String, crate::gameplay::ScalarType)>),
    Rotate(Box<Selection>, [f32; 3]),
    Assets(Option<crate::assets::AssetKind>),
    Usages(crate::assets::AssetKind, String),
    AssetPlace(crate::assets::AssetKind, String, Box<crate::ops::Placement>),
    AssetCreate(crate::ops::NewAsset),
    AssetRename(crate::assets::AssetKind, String, String),
    FsmCreate(String, Option<String>),
    FsmRemove(String),
    FsmEdit(String, crate::ops::FsmEdit),
    FsmNode(String, String, [f32; 2]),
    FsmAgent(Box<Selection>, String, Option<f32>),
    Behavior(
        Box<Selection>,
        crate::ops::BehaviorTemplate,
        Box<crate::ops::BehaviorOptions>,
    ),
    Ui(UiStatement),
    Tuning(TuningStatement),
}

/// `input` / `loc` / `ability` / `vfx` / `shader` statements (tuning editor operations).
enum TuningStatement {
    Input(crate::ops::InputEdit),
    Loc(crate::ops::LocEdit),
    Ability(String, crate::ops::AbilityEventEdit),
    Vfx(String, crate::ops::VfxEdit),
    Shader(String, String),
}

fn tuning_statement(tokens: &[String]) -> Result<TuningStatement> {
    use crate::ops::{AbilityEventEdit, InputEdit, LocEdit, VfxEdit};
    let text = |t: &str| match value(t) {
        Value::String(s) => s,
        _ => t.to_owned(),
    };
    let words: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let index = |t: &str| -> Result<usize> {
        t.parse()
            .with_context(|| format!("'{t}' is not an index (0, 1, ..)"))
    };
    let tick = |t: &str| -> Result<u32> {
        t.parse()
            .with_context(|| format!("'{t}' is not a tick (0, 1, .. at 120 per second)"))
    };
    let rgba = |t: &str| -> Result<[f64; 4]> {
        let v = numbers(t)?;
        ensure!(v.len() == 4, "colour '{t}' must be r,g,b,a");
        Ok([v[0], v[1], v[2], v[3]].map(f64::from))
    };
    Ok(match words.as_slice() {
        ["input", "button", action, key] => TuningStatement::Input(InputEdit::Button {
            action: (*action).into(),
            key: (*key).into(),
        }),
        ["input", "axis", action, positive, negative] => {
            TuningStatement::Input(InputEdit::Axis {
                action: (*action).into(),
                positive: (*positive).into(),
                negative: (*negative).into(),
            })
        }
        ["input", "remove", action] => TuningStatement::Input(InputEdit::Remove {
            action: (*action).into(),
        }),
        ["input", ..] => bail!(
            "use: input button ACTION KEY | input axis ACTION POSITIVE NEGATIVE | input remove ACTION"
        ),
        ["loc", "set", locale, key, t] => TuningStatement::Loc(LocEdit::Set {
            locale: (*locale).into(),
            key: (*key).into(),
            text: text(t),
        }),
        ["loc", "key", "add", key] => TuningStatement::Loc(LocEdit::AddKey {
            key: (*key).into(),
            text: String::new(),
        }),
        ["loc", "key", "add", key, t] => TuningStatement::Loc(LocEdit::AddKey {
            key: (*key).into(),
            text: text(t),
        }),
        ["loc", "key", "remove", key] => TuningStatement::Loc(LocEdit::RemoveKey {
            key: (*key).into(),
        }),
        ["loc", "locale", "add", locale, rest @ ..] if rest.len() <= 1 => {
            let from = match rest.first() {
                Some(t) => Some(
                    t.strip_prefix("from=")
                        .with_context(|| format!("option '{t}' must be from=LOCALE"))?
                        .to_owned(),
                ),
                None => None,
            };
            TuningStatement::Loc(LocEdit::AddLocale {
                locale: (*locale).into(),
                from,
            })
        }
        ["loc", "locale", "remove", locale] => TuningStatement::Loc(LocEdit::RemoveLocale {
            locale: (*locale).into(),
        }),
        ["loc", ..] => bail!(
            "use: loc set LOCALE KEY TEXT | loc key add KEY [TEXT] | loc key remove KEY | loc locale add L [from=L2] | loc locale remove L"
        ),
        ["ability", id, "event", "add", t, sound] => TuningStatement::Ability(
            (*id).into(),
            AbilityEventEdit::Add {
                tick: tick(t)?,
                sound: (*sound).into(),
            },
        ),
        ["ability", id, "event", "move", i, t] => TuningStatement::Ability(
            (*id).into(),
            AbilityEventEdit::Move {
                index: index(i)?,
                tick: tick(t)?,
            },
        ),
        ["ability", id, "event", "sound", i, sound] => TuningStatement::Ability(
            (*id).into(),
            AbilityEventEdit::Sound {
                index: index(i)?,
                sound: (*sound).into(),
            },
        ),
        ["ability", id, "event", "remove", i] => TuningStatement::Ability(
            (*id).into(),
            AbilityEventEdit::Remove { index: index(i)? },
        ),
        ["ability", ..] => bail!(
            "use: ability ID event add TICK SOUND | ability ID event move I TICK | ability ID event sound I SOUND | ability ID event remove I"
        ),
        ["vfx", id, "size", start, end] => {
            let n = |t: &str| -> Result<f64> {
                t.parse().with_context(|| format!("'{t}' is not a number"))
            };
            TuningStatement::Vfx(
                (*id).into(),
                VfxEdit::Size {
                    start: n(start)?,
                    end: n(end)?,
                },
            )
        }
        ["vfx", id, "color", start, end] => TuningStatement::Vfx(
            (*id).into(),
            VfxEdit::Color {
                start: rgba(start)?,
                end: rgba(end)?,
            },
        ),
        ["vfx", id, "size_curve", keys @ ..] if !keys.is_empty() => {
            let keys = if keys == ["none"] {
                vec![]
            } else {
                keys.iter()
                    .map(|k| -> Result<[f64; 2]> {
                        let (t, s) = k
                            .split_once(':')
                            .with_context(|| format!("size key '{k}' must be T:SIZE"))?;
                        let n = |x: &str| -> Result<f64> {
                            x.parse().with_context(|| format!("'{x}' is not a number"))
                        };
                        Ok([n(t)?, n(s)?])
                    })
                    .collect::<Result<_>>()?
            };
            TuningStatement::Vfx((*id).into(), VfxEdit::SizeCurve { keys })
        }
        ["vfx", id, "gradient", stops @ ..] if !stops.is_empty() => {
            let stops = if stops == ["none"] {
                vec![]
            } else {
                stops
                    .iter()
                    .map(|s| -> Result<(f64, [f64; 4])> {
                        let (t, c) = s
                            .split_once(':')
                            .with_context(|| format!("gradient stop '{s}' must be T:r,g,b,a"))?;
                        let t = t.parse().with_context(|| format!("'{t}' is not a number"))?;
                        Ok((t, rgba(c)?))
                    })
                    .collect::<Result<_>>()?
            };
            TuningStatement::Vfx((*id).into(), VfxEdit::ColorGradient { stops })
        }
        ["vfx", ..] => bail!(
            "use: vfx ID size START END | vfx ID color r,g,b,a r,g,b,a | vfx ID size_curve T:S .. | none | vfx ID gradient T:r,g,b,a .. | none"
        ),
        ["shader", id, "source", t] => TuningStatement::Shader((*id).into(), text(t)),
        ["shader", ..] => bail!("use: shader ID source \"WGSL\" (JSON string; \\n for new lines)"),
        _ => bail!("unknown tuning statement"),
    })
}

fn tuning_edits(w: &World, stmt: &TuningStatement) -> Result<Vec<Edit>> {
    match stmt {
        TuningStatement::Input(edit) => crate::ops::edit_input(w, edit),
        TuningStatement::Loc(edit) => crate::ops::edit_localization(w, edit),
        TuningStatement::Ability(id, edit) => crate::ops::edit_ability_event(w, id, edit),
        TuningStatement::Vfx(id, edit) => crate::ops::edit_vfx(w, id, edit),
        TuningStatement::Shader(id, wgsl) => crate::ops::set_shader_source(w, id, wgsl),
    }
}

/// One `input|loc|ability|vfx|shader ...` statement compiled against `world`: the editor's
/// tuning commands run their argument through this, so they match the ACL verb exactly.
pub fn tuning_command_edits(world: &World, statement: &str) -> Result<Vec<Edit>> {
    tuning_edits(world, &tuning_statement(&tokenize(statement)?)?)
}

/// `ui ...` statements (UI Editor operations).
enum UiStatement {
    Place(String, Option<crate::gameplay::Region>),
    Style(String, Option<crate::gameplay::TextStyle>),
    Button(String, crate::gameplay::RuntimeCommand),
    EffectAdd(Box<crate::gameplay::UiEffect>),
    EffectSet(String, Vec<(String, String)>),
    EffectRemove(String),
}

fn key_values(tokens: &[String]) -> Result<Vec<(String, String)>> {
    tokens
        .iter()
        .map(|t| {
            let (k, v) = t
                .split_once('=')
                .with_context(|| format!("option '{t}' must be key=value"))?;
            let v = match value(v) {
                Value::String(s) => s,
                _ => v.to_owned(),
            };
            Ok((k.to_owned(), v))
        })
        .collect()
}

fn ui_statement(tokens: &[String]) -> Result<UiStatement> {
    use crate::gameplay::RuntimeCommand;
    let arg = |n: usize| {
        tokens
            .get(n)
            .with_context(|| "ui needs more arguments (see grammar: ui place|style|button|effect)")
    };
    Ok(match arg(1)?.as_str() {
        "place" => {
            ensure!(tokens.len() == 4, "use: ui place WIDGET REGION|none");
            UiStatement::Place(arg(2)?.clone(), crate::ops::parse_region(arg(3)?)?)
        }
        "style" => {
            ensure!(tokens.len() == 4, "use: ui style WIDGET STYLE|none");
            UiStatement::Style(arg(2)?.clone(), crate::ops::parse_style(arg(3)?)?)
        }
        "button" => {
            let usage = "use: ui button WIDGET emit EVENT [{FIELDS}] | locale L | choose NODE CHOICE | call CAP ACTION [{ARGS}]";
            let widget = arg(2)?.clone();
            let n = tokens.len();
            let command = match (tokens.get(3).map(String::as_str), n) {
                (Some("emit"), 5 | 6) => RuntimeCommand::Emit {
                    event: tokens[4].clone(),
                    fields: match tokens.get(5) {
                        Some(t) => serde_json::from_value(json_value(t)?)
                            .context("emit fields must be a JSON object of scalars")?,
                        None => Default::default(),
                    },
                },
                (Some("locale"), 5) => RuntimeCommand::Locale {
                    locale: tokens[4].clone(),
                },
                (Some("choose"), 6) => RuntimeCommand::Choose {
                    node: tokens[4].clone(),
                    choice: tokens[5].clone(),
                },
                (Some("call"), 6 | 7) => RuntimeCommand::Call {
                    capability: tokens[4].clone(),
                    action: tokens[5].clone(),
                    args: match tokens.get(6) {
                        Some(t) => json_value(t)?,
                        None => json!({}),
                    },
                },
                _ => bail!("{usage}"),
            };
            UiStatement::Button(widget, command)
        }
        "effect" => match arg(2)?.as_str() {
            "add" => {
                ensure!(
                    tokens.len() >= 6,
                    "use: ui effect add ID WIDGET KIND [key=value ..]"
                );
                let kind = serde_json::from_value(json!(tokens[5])).map_err(|_| {
                    anyhow::anyhow!(
                        "unknown effect kind '{}' (use {})",
                        tokens[5],
                        crate::gameplay::UiEffectKind::ALL
                            .map(|k| k.name())
                            .join(", ")
                    )
                })?;
                let mut effect =
                    crate::gameplay::UiEffect::new(tokens[3].clone(), tokens[4].clone(), kind);
                crate::ops::apply_effect_options(&mut effect, &key_values(&tokens[6..])?)?;
                UiStatement::EffectAdd(Box::new(effect))
            }
            "set" => {
                ensure!(
                    tokens.len() >= 5,
                    "use: ui effect set ID key=value [key=value ..]"
                );
                UiStatement::EffectSet(tokens[3].clone(), key_values(&tokens[4..])?)
            }
            "remove" => {
                ensure!(tokens.len() == 4, "use: ui effect remove ID");
                UiStatement::EffectRemove(tokens[3].clone())
            }
            other => bail!("unknown ui effect verb '{other}' (add, set, remove)"),
        },
        other => bail!("unknown ui verb '{other}' (place, style, button, effect)"),
    })
}

fn ui_edits(w: &World, r: &Registry, stmt: &UiStatement) -> Result<Vec<Edit>> {
    match stmt {
        UiStatement::Place(widget, region) => crate::ops::set_ui_region(w, widget, *region),
        UiStatement::Style(widget, style) => crate::ops::set_ui_style(w, widget, *style),
        UiStatement::Button(widget, command) => {
            crate::ops::set_ui_button_command(w, widget, command)
        }
        UiStatement::EffectAdd(effect) => crate::ops::add_ui_effect(w, r, effect),
        UiStatement::EffectSet(id, options) => crate::ops::edit_ui_effect(w, id, options),
        UiStatement::EffectRemove(id) => crate::ops::remove_ui_effect(w, id),
    }
}

/// One `ui ...` statement (without the leading `ui`) compiled against `world`: the editor's
/// `ui.*` commands run their argument through this, so they match the ACL verb exactly.
pub fn ui_command_edits(world: &World, registry: &Registry, statement: &str) -> Result<Vec<Edit>> {
    let mut tokens = vec!["ui".to_owned()];
    tokens.extend(tokenize(statement)?);
    ui_edits(world, registry, &ui_statement(&tokens)?)
}

impl Statement {
    fn is_edit(&self) -> bool {
        !matches!(
            self,
            Self::Select(..)
                | Self::Inspect(_)
                | Self::History(_)
                | Self::Diff(..)
                | Self::Undo(_)
                | Self::Conflicts(_)
                | Self::Assets(_)
                | Self::Usages(..)
        )
    }
}

fn on_off(token: &str) -> Result<bool> {
    match token {
        "on" | "true" => Ok(true),
        "off" | "false" => Ok(false),
        other => bail!("expected on or off, got '{other}'"),
    }
}

fn placement(tokens: &[String]) -> Result<crate::ops::Placement> {
    let mut p = crate::ops::Placement::default();
    for t in tokens {
        let (k, v) = t
            .split_once('=')
            .with_context(|| format!("option '{t}' must be key=value"))?;
        match k {
            "id" => p.id = Some(v.into()),
            "at" => p.position = Some(vec3(v)?),
            "rot" => p.rotation = Some(vec3(v)?),
            "parent" => p.parent = Some(v.into()),
            "scene" => p.scene = Some(v.into()),
            other => bail!("unknown option '{other}' (use id, at, rot, parent, scene)"),
        }
    }
    Ok(p)
}

fn entity_ids(
    world: &World,
    registry: &Registry,
    select: &Selection,
    line: usize,
) -> Result<Vec<String>> {
    if select.from != Collection::Entities {
        return Err(diag(line, "selector must name entities"));
    }
    let found = crate::local::matches(world, registry, select)?;
    if found.is_empty() {
        return Err(Diagnostic::error(
            "no_match",
            format!("/line/{line}"),
            format!("line {line}: selector matched no entities"),
        ));
    }
    let mut ids: Vec<String> = found.into_iter().map(|(t, _)| t.id().to_owned()).collect();
    // Matches come back in ID order; explicitly listed IDs keep the order they were written
    // in (the editor's selection order), which decides generated IDs and batch order.
    ids.sort_by_key(|id| select.ids.iter().position(|s| s == id));
    Ok(ids)
}

fn parse(tokens: &[String]) -> Result<Statement> {
    let arg = |n: usize| {
        tokens
            .get(n)
            .with_context(|| format!("'{}' needs more arguments", tokens[0]))
    };
    Ok(match tokens[0].as_str() {
        "select" => {
            let mut s = selector(arg(1)?)?;
            s.format = Format::Lines;
            let mut aliases = vec![];
            let mut i = 2;
            while i < tokens.len() {
                match tokens[i].as_str() {
                    "where" => {
                        i += 1;
                        where_clauses(tokens, &mut i, &mut s)?;
                    }
                    "fields" => {
                        for f in arg(i + 1)?.split(',').filter(|f| !f.is_empty()) {
                            let p = field(f, s.from);
                            aliases.push((p.clone(), f.to_owned()));
                            s.fields.push(p);
                        }
                        i += 2;
                    }
                    "limit" => {
                        s.limit = Some(arg(i + 1)?.parse().context("limit must be an integer")?);
                        i += 2;
                    }
                    "cursor" => {
                        s.cursor = Some(arg(i + 1)?.clone());
                        i += 2;
                    }
                    other => bail!("unexpected '{other}' (expected where, fields, limit, cursor)"),
                }
            }
            Statement::Select(Box::new(s), aliases)
        }
        "inspect" => Statement::Inspect(arg(1)?.clone()),
        "history" => Statement::History(match tokens.get(1) {
            Some(n) => n.parse().context("history count must be an integer")?,
            None => 12,
        }),
        "diff" => Statement::Diff(
            arg(1)?.parse().context("diff needs revision numbers")?,
            arg(2)?.parse().context("diff needs revision numbers")?,
        ),
        "undo" => Statement::Undo(arg(1)?.parse().context("undo needs a revision number")?),
        "conflicts" => Statement::Conflicts(
            arg(1)?
                .trim_start_matches('#')
                .parse()
                .context("conflicts needs a proposal number")?,
        ),
        "set" => {
            let mut s = selector(arg(1)?)?;
            let f = field(arg(2)?, s.from);
            let v = value(arg(3)?);
            let mut i = 4;
            if i < tokens.len() {
                ensure!(
                    tokens[i] == "where",
                    "unexpected '{}' (expected where)",
                    tokens[i]
                );
                i += 1;
                where_clauses(tokens, &mut i, &mut s)?;
                ensure!(i == tokens.len(), "unexpected '{}'", tokens[i]);
            }
            Statement::Set(Box::new(s), f, v)
        }
        "spawn" => {
            let prefab = arg(1)?.clone();
            let mut opts = vec![];
            for t in &tokens[2..] {
                let (k, v) = t
                    .split_once('=')
                    .with_context(|| format!("spawn option '{t}' must be key=value"))?;
                opts.push((k.to_owned(), v.to_owned()));
            }
            Statement::Spawn(opts, prefab)
        }
        "delete" => {
            ensure!(tokens.len() == 2, "delete takes one selector");
            Statement::Delete(Box::new(selector(arg(1)?)?))
        }
        "remove" if tokens.len() == 4 => {
            let s = selector(arg(1)?)?;
            let f = field(arg(2)?, s.from);
            let index = arg(3)?
                .parse()
                .context("use: remove SEL FIELD INDEX (index = non-negative integer)")?;
            Statement::Array(Box::new(s), f, crate::ops::ArrayEdit::Remove(index))
        }
        "remove" => {
            ensure!(
                tokens.len() == 3,
                "use: remove SEL COMPONENT | remove SEL FIELD INDEX"
            );
            Statement::Remove(Box::new(selector(arg(1)?)?), arg(2)?.clone())
        }
        "append" => {
            ensure!(tokens.len() == 4, "use: append SEL FIELD VALUE");
            let s = selector(arg(1)?)?;
            let f = field(arg(2)?, s.from);
            Statement::Array(
                Box::new(s),
                f,
                crate::ops::ArrayEdit::Append(value(arg(3)?)),
            )
        }
        "insert" => {
            ensure!(tokens.len() == 5, "use: insert SEL FIELD INDEX VALUE");
            let s = selector(arg(1)?)?;
            let f = field(arg(2)?, s.from);
            let index = arg(3)?
                .parse()
                .context("insert index must be a non-negative integer")?;
            Statement::Array(
                Box::new(s),
                f,
                crate::ops::ArrayEdit::Insert(index, value(arg(4)?)),
            )
        }
        "reorder" => {
            ensure!(tokens.len() == 5, "use: reorder SEL FIELD FROM TO");
            let s = selector(arg(1)?)?;
            let f = field(arg(2)?, s.from);
            let index = |n: usize| -> Result<usize> {
                arg(n)?
                    .parse()
                    .context("reorder indexes must be non-negative integers")
            };
            let edit = crate::ops::ArrayEdit::Move(index(3)?, index(4)?);
            Statement::Array(Box::new(s), f, edit)
        }
        "reset" => {
            ensure!(tokens.len() == 3, "use: reset SEL COMPONENT");
            Statement::Reset(Box::new(selector(arg(1)?)?), arg(2)?.clone())
        }
        "tag" => {
            ensure!(tokens.len() >= 3, "use: tag SEL +TAG -TAG ...");
            let (mut add, mut remove) = (vec![], vec![]);
            for t in &tokens[2..] {
                match (t.strip_prefix('+'), t.strip_prefix('-')) {
                    (Some(tag), _) => add.push(tag.to_owned()),
                    (_, Some(tag)) => remove.push(tag.to_owned()),
                    _ => bail!("tag '{t}' needs + (add) or - (remove): tag SEL +{t}"),
                }
            }
            Statement::Tag(Box::new(selector(arg(1)?)?), add, remove)
        }
        "world" => {
            ensure!(tokens.len() == 3, "use: world id ID | world title TEXT");
            let text = match value(arg(2)?) {
                Value::String(s) => s,
                _ => arg(2)?.clone(),
            };
            match arg(1)?.as_str() {
                "id" => Statement::WorldId(text),
                "title" => Statement::WorldTitle(text),
                other => bail!("unknown world field '{other}' (id, title)"),
            }
        }
        "capability" | "system" => {
            ensure!(
                tokens.len() == 3,
                "use: {0} add ID | {0} remove ID",
                tokens[0]
            );
            let id = arg(2)?.clone();
            match (tokens[0].as_str(), arg(1)?.as_str()) {
                ("capability", "add") => Statement::CapabilityAdd(id),
                ("capability", "remove") => Statement::CapabilityRemove(id),
                ("system", "add") => Statement::SystemAdd(id),
                ("system", "remove") => Statement::SystemRemove(id),
                (_, other) => bail!("unknown {} verb '{other}' (add, remove)", tokens[0]),
            }
        }
        "resource" => match (arg(1)?.as_str(), tokens.len()) {
            ("create", 3) => Statement::ResourceCreate(arg(2)?.clone(), None),
            ("create", 4) => Statement::ResourceCreate(arg(2)?.clone(), Some(json_value(arg(3)?)?)),
            ("remove", 3) => Statement::ResourceRemove(arg(2)?.clone()),
            _ => bail!("use: resource create ID [JSON] | resource remove ID"),
        },
        "add" => {
            ensure!(
                (3..=4).contains(&tokens.len()),
                "use: add SEL COMPONENT [JSON]"
            );
            let value = tokens.get(3).map(|t| json_value(t)).transpose()?;
            Statement::AddComponent(Box::new(selector(arg(1)?)?), arg(2)?.clone(), value)
        }
        "event" => {
            ensure!(
                tokens.get(1).is_some_and(|t| t == "declare") && tokens.len() >= 3,
                "use: event declare NAME [FIELD=text|number|int|bool ...]"
            );
            let mut fields = vec![];
            for t in &tokens[3..] {
                let (k, v) = t
                    .split_once('=')
                    .with_context(|| format!("field '{t}' must be NAME=TYPE"))?;
                let ty = serde_json::from_value(json!(v)).with_context(|| {
                    format!("unknown field type '{v}' (text, number, int, bool)")
                })?;
                fields.push((k.to_owned(), ty));
            }
            Statement::EventDeclare(arg(2)?.clone(), fields)
        }
        "ui" => Statement::Ui(ui_statement(tokens)?),
        "input" | "loc" | "ability" | "vfx" | "shader" => {
            Statement::Tuning(tuning_statement(tokens)?)
        }
        "rotate" => {
            ensure!(tokens.len() == 3, "use: rotate SEL x,y,z (Euler degrees)");
            Statement::Rotate(Box::new(selector(arg(1)?)?), vec3(arg(2)?)?)
        }
        "create" if tokens.get(1).is_some_and(|t| t == "trigger") => {
            let shape = tokens
                .get(2)
                .with_context(|| "create trigger needs a shape: box|sphere".to_owned())?;
            Statement::Create(
                format!("trigger.{shape}"),
                Box::new(placement(&tokens[3..])?),
            )
        }
        "create" if tokens.get(1).is_some_and(|t| t == "light") => {
            let kind = tokens.get(2).with_context(|| {
                "create light needs a kind: directional|point|spot|area".to_owned()
            })?;
            Statement::Create(format!("light.{kind}"), Box::new(placement(&tokens[3..])?))
        }
        "create" => Statement::Create(arg(1)?.clone(), Box::new(placement(&tokens[2..])?)),
        "instantiate" => {
            Statement::Instantiate(arg(1)?.clone(), Box::new(placement(&tokens[2..])?))
        }
        "duplicate" => {
            ensure!(tokens.len() == 2, "duplicate takes one selector");
            Statement::Duplicate(Box::new(selector(arg(1)?)?))
        }
        "group" => {
            let id = match tokens.get(2).map(String::as_str) {
                None => None,
                Some(t) => Some(
                    t.strip_prefix("id=")
                        .with_context(|| format!("unexpected '{t}' (expected id=ID)"))?
                        .to_owned(),
                ),
            };
            ensure!(tokens.len() <= 3, "use: group SEL [id=ID]");
            Statement::Group(Box::new(selector(arg(1)?)?), id)
        }
        "reparent" => {
            let parent = arg(2)?;
            let keep_world = match tokens.get(3).map(String::as_str) {
                None | Some("keep=world") => true,
                Some("keep=local") => false,
                Some(other) => bail!("unexpected '{other}' (expected keep=world|local)"),
            };
            ensure!(tokens.len() <= 4, "unexpected '{}'", tokens[4]);
            Statement::Reparent(
                arg(1)?.clone(),
                (parent != "root").then(|| parent.clone()),
                keep_world,
            )
        }
        "scene" => {
            let verb = arg(1)?.as_str();
            let id = arg(2)?.clone();
            let max = match verb {
                "create" => 4,
                "activate" => 4,
                "delete" => 3,
                "member" => 5,
                other => bail!("unknown scene verb '{other}' (create, activate, delete, member)"),
            };
            ensure!(tokens.len() <= max, "unexpected '{}'", tokens[max]);
            match verb {
                "create" => {
                    Statement::SceneCreate(id, tokens.get(3).map_or(Ok(true), |t| on_off(t))?)
                }
                "activate" => Statement::SceneActive(id, on_off(arg(3)?)?),
                "delete" => Statement::SceneDelete(id),
                _ => Statement::SceneMember(id, Box::new(selector(arg(3)?)?), on_off(arg(4)?)?),
            }
        }
        "camera" if tokens.len() == 3 && tokens[1] == "main" => {
            Statement::CameraMain(tokens[2].clone())
        }
        "camera" if tokens.len() == 4 && tokens[2] == "blocker" => {
            Statement::CameraBlocker(Box::new(selector(&tokens[1])?), on_off(&tokens[3])?)
        }
        "camera" => camera_follow(tokens)?,
        "migrate" => {
            ensure!(
                tokens.len() == 2 && tokens[1] == "camera_blocker",
                "use: migrate camera_blocker"
            );
            Statement::MigrateCameraBlockers
        }
        "rename" => {
            ensure!(
                tokens.len() == 3,
                "use: rename ID LABEL (\"\" clears the label)"
            );
            let label = match value(arg(2)?) {
                Value::String(s) => s,
                _ => arg(2)?.clone(),
            };
            Statement::Rename(arg(1)?.clone(), label)
        }
        "move" => {
            let to = match (tokens.get(2).map(String::as_str), tokens.get(3)) {
                (Some("before"), Some(o)) if tokens.len() == 4 => {
                    crate::ops::Order::Before(o.clone())
                }
                (Some("after"), Some(o)) if tokens.len() == 4 => {
                    crate::ops::Order::After(o.clone())
                }
                (Some("end"), None) => crate::ops::Order::End,
                _ => bail!("use: move ID before OTHER | move ID after OTHER | move ID end"),
            };
            Statement::Move(arg(1)?.clone(), to)
        }
        "ground" => {
            ensure!(tokens.len() == 2, "ground takes one selector");
            Statement::Ground(Box::new(selector(arg(1)?)?))
        }
        "follow" => {
            let select = selector(arg(1)?)?;
            let path = arg(2)?.clone();
            let mut options = crate::ops::FollowOptions::default();
            for t in &tokens[3..] {
                let (k, v) = t
                    .split_once('=')
                    .with_context(|| format!("option '{t}' must be key=value"))?;
                let number = || {
                    v.parse::<f32>()
                        .with_context(|| format!("{k} must be a number"))
                };
                match k {
                    "speed" => options.speed = Some(number()?),
                    "offset" => options.offset = Some(number()?),
                    "end" => {
                        options.end = Some(match v {
                            "loop" => crate::path::PathEnd::Loop,
                            "ping_pong" | "pingpong" => crate::path::PathEnd::PingPong,
                            "stop" => crate::path::PathEnd::Stop,
                            other => bail!("unknown end '{other}' (use loop, ping_pong, stop)"),
                        })
                    }
                    other => bail!("unknown option '{other}' (use speed, end, offset)"),
                }
            }
            Statement::Follow(Box::new(select), path, options)
        }
        "path" => {
            use crate::ops::PointEdit;
            let id = arg(1)?.clone();
            let index = |n: usize| -> Result<usize> {
                arg(n)?
                    .parse()
                    .context("point index must be a non-negative integer")
            };
            let at = |n: usize| tokens.get(n).map(|t| vec3(t)).transpose();
            let (edit, max) = match arg(2)?.as_str() {
                "add" => (PointEdit::Add(at(3)?), 4),
                "insert" => (PointEdit::Insert(index(3)?, at(4)?), 5),
                "move" => (PointEdit::Move(index(3)?, vec3(arg(4)?)?), 5),
                "remove" => (PointEdit::Remove(index(3)?), 4),
                other => bail!("unknown path verb '{other}' (add, insert, move, remove)"),
            };
            ensure!(tokens.len() <= max, "unexpected '{}'", tokens[max]);
            Statement::PathPoint(id, edit)
        }
        "nav" => {
            let select = Box::new(selector(arg(1)?)?);
            match arg(2)?.as_str() {
                "agent" => {
                    let mut options = crate::ops::NavOptions::default();
                    for t in &tokens[3..] {
                        let (k, v) = t
                            .split_once('=')
                            .with_context(|| format!("option '{t}' must be key=value"))?;
                        let number = v
                            .parse::<f32>()
                            .with_context(|| format!("{k} must be a number"))?;
                        match k {
                            "speed" => options.speed = Some(number),
                            "radius" => options.radius = Some(number),
                            other => bail!("unknown option '{other}' (use speed, radius)"),
                        }
                    }
                    Statement::NavAgent(select, options)
                }
                "target" => {
                    ensure!(
                        tokens.len() == 4,
                        "use: nav SEL target x,y,z | nav SEL target ENTITY | nav SEL target none"
                    );
                    let goal = arg(3)?;
                    let target = if goal == "none" {
                        None
                    } else if goal.contains(',') {
                        Some(crate::navigation::NavTarget::Point(vec3(goal)?))
                    } else {
                        Some(crate::navigation::NavTarget::Entity(goal.clone()))
                    };
                    Statement::NavTarget(select, target)
                }
                other => bail!("unknown nav verb '{other}' (agent, target)"),
            }
        }
        "interact" => {
            let select = Box::new(selector(arg(1)?)?);
            let mut options = crate::ops::InteractOptions::default();
            for t in &tokens[2..] {
                let (k, v) = t
                    .split_once('=')
                    .with_context(|| format!("option '{t}' must be key=value"))?;
                let text = match value(v) {
                    Value::String(s) => s,
                    _ => v.to_owned(),
                };
                match k {
                    "event" => options.event = Some(text),
                    "prompt" => options.prompt = Some(text),
                    "action" => options.action = Some(text),
                    "range" => {
                        options.range = Some(
                            v.parse::<f32>()
                                .with_context(|| "range must be a number".to_owned())?,
                        )
                    }
                    other => bail!("unknown option '{other}' (use event, prompt, range, action)"),
                }
            }
            Statement::Interact(select, options)
        }
        "tween" => tween(tokens)?,
        "audio" => audio(tokens)?,
        "fsm" => fsm(tokens)?,
        "behavior" => behavior(tokens)?,
        "prefab" => {
            let rest: Vec<&str> = tokens[1..].iter().map(String::as_str).collect();
            match rest.as_slice() {
                ["create", "from", entity] => Statement::PrefabCreate(None, (*entity).into()),
                ["create", id, "from", entity] => {
                    Statement::PrefabCreate(Some((*id).into()), (*entity).into())
                }
                _ => bail!("use: prefab create [PREFAB_ID] from ENTITY"),
            }
        }
        "assets" => {
            ensure!(tokens.len() <= 2, "use: assets [KIND]");
            Statement::Assets(
                tokens
                    .get(1)
                    .map(|k| {
                        crate::assets::AssetKind::parse(k)
                            .with_context(|| format!("unknown asset kind '{k}'"))
                    })
                    .transpose()?,
            )
        }
        "usages" => {
            ensure!(tokens.len() == 2, "use: usages KIND:ID");
            let (kind, id) = crate::assets::parse_reference(&tokens[1])?;
            Statement::Usages(kind, id)
        }
        "asset" => asset(tokens)?,
        other => bail!("unknown statement '{other}'"),
    })
}

/// `asset place KIND:ID P` | `asset create KIND ..` | `asset rename KIND:ID NEW`.
fn asset(tokens: &[String]) -> Result<Statement> {
    use crate::ops::NewAsset;
    const USE: &str = "use: asset place KIND:ID [id=] [at=x,y,z] [rot=] [parent=] [scene=] | asset create material|prefab|machine ID [..] | asset create ui|input | asset rename KIND:ID NEW";
    let text = |t: &str| match value(t) {
        Value::String(s) => s,
        _ => t.to_owned(),
    };
    match tokens.get(1).map(String::as_str) {
        Some("place") => {
            let (kind, id) = crate::assets::parse_reference(tokens.get(2).context(USE)?)?;
            Ok(Statement::AssetPlace(
                kind,
                id,
                Box::new(placement(&tokens[3..])?),
            ))
        }
        Some("rename") => {
            ensure!(tokens.len() == 4, "use: asset rename KIND:ID NEW");
            let (kind, id) = crate::assets::parse_reference(&tokens[2])?;
            Ok(Statement::AssetRename(kind, id, text(&tokens[3])))
        }
        Some("create") => {
            let kind = tokens.get(2).context(USE)?.as_str();
            let keyed = matches!(kind, "material" | "prefab" | "machine");
            let id = if keyed {
                Some(text(tokens.get(3).with_context(|| {
                    format!("asset create {kind} needs an ID")
                })?))
            } else {
                None
            };
            let mut options = std::collections::BTreeMap::new();
            for t in &tokens[if keyed { 4 } else { 3 }..] {
                let (k, v) = t
                    .split_once('=')
                    .with_context(|| format!("option '{t}' must be key=value"))?;
                options.insert(k.to_owned(), text(v));
            }
            let allowed: &[&str] = match kind {
                "material" => &["shader", "tint"],
                "prefab" => &["from"],
                "machine" => &["initial"],
                "ui" => &["locale"],
                "input" => &[],
                other => bail!(
                    "unknown asset kind to create '{other}' (material, prefab, machine, ui, input)"
                ),
            };
            if let Some(k) = options.keys().find(|k| !allowed.contains(&k.as_str())) {
                bail!(
                    "unknown option '{k}' for asset create {kind} (use {})",
                    allowed.join(", ")
                );
            }
            let id = id.unwrap_or_default();
            let mut take = |k: &str| options.remove(k);
            Ok(Statement::AssetCreate(match kind {
                "material" => NewAsset::Material {
                    id,
                    shader: take("shader"),
                    tint: take("tint")
                        .map(|t| {
                            let v = numbers(&t)?;
                            ensure!(
                                v.len() == 3 || v.len() == 4,
                                "tint must be r,g,b or r,g,b,a"
                            );
                            Ok([v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0)])
                        })
                        .transpose()?,
                },
                "prefab" => NewAsset::Prefab {
                    id,
                    from: take("from"),
                },
                "machine" => NewAsset::Machine {
                    id,
                    initial: take("initial"),
                },
                "ui" => NewAsset::Ui {
                    locale: take("locale"),
                },
                _ => NewAsset::Input,
            }))
        }
        _ => bail!("{USE}"),
    }
}

/// `camera SEL follow TARGET [k=v..]` (`camera main ID` is matched before this).
fn camera_follow(tokens: &[String]) -> Result<Statement> {
    const USE: &str = "use: camera main ID | camera SEL follow TARGET [distance=M] [min=M] [max=M] [yaw=DEG] [pitch=DEG] [height=M] [orbit=AXIS] [zoom=AXIS] [radius=M] [collide=on|off]";
    let (Some(select), Some("follow"), Some(target)) = (
        tokens.get(1),
        tokens.get(2).map(String::as_str),
        tokens.get(3),
    ) else {
        bail!("{USE}");
    };
    let mut o = crate::ops::CameraFollowOptions::default();
    for t in &tokens[4..] {
        let (k, v) = t
            .split_once('=')
            .with_context(|| format!("option '{t}' must be key=value"))?;
        let number = || {
            v.parse::<f32>()
                .with_context(|| format!("{k} must be a number"))
        };
        match k {
            "distance" => o.distance = Some(number()?),
            "min" => o.min_distance = Some(number()?),
            "max" => o.max_distance = Some(number()?),
            "yaw" => o.yaw = Some(number()?),
            "pitch" => o.pitch = Some(number()?),
            "height" => o.look_height = Some(number()?),
            "orbit" => o.yaw_axis = Some(v.to_owned()),
            "zoom" => o.zoom_axis = Some(v.to_owned()),
            "radius" => o.radius = Some(number()?),
            "collide" => o.collision = Some(on_off(v)?),
            other => bail!(
                "unknown option '{other}' (use distance, min, max, yaw, pitch, height, orbit, zoom, radius, collide)"
            ),
        }
    }
    Ok(Statement::CameraFollow(
        Box::new(selector(select)?),
        target.clone(),
        Box::new(o),
    ))
}

/// `audio clip ID PRESET` | `audio SEL source [k=v..]` | `audio ID listener`. Four tokens
/// starting `audio clip` with a bare last token add a library clip (so clips may be named
/// `source` or `listener`); otherwise `source` / `listener` in third place select entities
/// (an entity named `clip` still works).
fn audio(tokens: &[String]) -> Result<Statement> {
    const USE: &str = "use: audio clip ID PRESET | audio SEL source [clip=..] [volume=..] [loop=..] [autoplay=..] [spatial=..] [min=..] [max=..] [rolloff=..] | audio ID listener";
    let arg = |n: usize| tokens.get(n).map(String::as_str).context(USE);
    if tokens.len() == 4 && tokens[1] == "clip" && !tokens[3].contains('=') {
        return Ok(Statement::AudioClip(tokens[2].clone(), tokens[3].clone()));
    }
    match (arg(1)?, arg(2)?) {
        (sel, "source") => {
            let mut o = crate::ops::AudioSourceOptions::default();
            let number = |k: &str, v: &str| -> Result<f32> {
                v.parse::<f32>()
                    .with_context(|| format!("{k} must be a number"))
            };
            for t in &tokens[3..] {
                let (k, v) = t
                    .split_once('=')
                    .with_context(|| format!("option '{t}' must be key=value"))?;
                match k {
                    "clip" => {
                        o.clip = Some(match value(v) {
                            Value::String(s) => s,
                            _ => v.to_owned(),
                        })
                    }
                    "volume" => o.volume = Some(number(k, v)?),
                    "loop" => o.looping = Some(on_off(v)?),
                    "autoplay" => o.autoplay = Some(on_off(v)?),
                    "spatial" => o.spatial = Some(on_off(v)?),
                    "min" => o.min_distance = Some(number(k, v)?),
                    "max" => o.max_distance = Some(number(k, v)?),
                    "rolloff" => {
                        o.rolloff = Some(match v {
                            "inverse" => crate::audio::Rolloff::Inverse,
                            "linear" => crate::audio::Rolloff::Linear,
                            other => bail!("rolloff must be inverse or linear, got '{other}'"),
                        })
                    }
                    other => bail!(
                        "unknown option '{other}' (use clip, volume, loop, autoplay, spatial, min, max, rolloff)"
                    ),
                }
            }
            Ok(Statement::AudioSource(
                Box::new(selector(sel)?),
                Box::new(o),
            ))
        }
        (id, "listener") => {
            ensure!(tokens.len() == 3, "audio ID listener takes no options");
            Ok(Statement::AudioListener(id.to_owned()))
        }
        _ => bail!("{USE}"),
    }
}

/// `tween` statements (see GRAMMAR).
fn tween(tokens: &[String]) -> Result<Statement> {
    use crate::ops::{KeyEdit, TweenClipOptions};
    let arg = |n: usize| {
        tokens.get(n).map(String::as_str).context(
            "use: tween SEL clip NAME ... | tween ID track CLIP ... | tween ID key CLIP TRACK ...",
        )
    };
    let index = |n: usize, what: &str| -> Result<usize> {
        arg(n)?
            .parse()
            .with_context(|| format!("{what} index must be a non-negative integer"))
    };
    let number = |text: &str, what: &str| -> Result<f32> {
        text.parse::<f32>()
            .with_context(|| format!("{what} must be a number"))
    };
    let text = |v: &str| match value(v) {
        Value::String(s) => s,
        _ => v.to_owned(),
    };
    let options = |from: usize, adding: bool| -> Result<TweenClipOptions> {
        let mut o = TweenClipOptions::default();
        for t in &tokens[from.min(tokens.len())..] {
            let (k, v) = t
                .split_once('=')
                .with_context(|| format!("option '{t}' must be key=value"))?;
            match k {
                "property" if adding => o.property = Some(text(v)),
                "target" if adding => o.target = Some(text(v)),
                "duration" => {
                    o.duration = Some(if v == "auto" {
                        None
                    } else {
                        Some(number(v, "duration")?)
                    })
                }
                "mode" => o.mode = Some(crate::tween::TweenMode::parse(v)?),
                "speed" => o.speed = Some(number(v, "speed")?),
                "autoplay" => o.autoplay = Some(on_off(v)?),
                "on_finish" => o.on_finish = Some(text(v)),
                "play_on" => o.play_on = Some(text(v)),
                other => bail!(
                    "unknown option '{other}' (use {}duration, mode, speed, autoplay, on_finish, play_on)",
                    if adding { "property, target, " } else { "" }
                ),
            }
        }
        Ok(o)
    };
    let id = arg(1)?.to_owned();
    Ok(match arg(2)? {
        "clip" => {
            let name = arg(3)?.to_owned();
            match tokens.get(4).map(String::as_str) {
                Some("set") => Statement::TweenClipSet(id, name, Box::new(options(5, false)?)),
                Some("remove") => {
                    ensure!(tokens.len() == 5, "use: tween ID clip NAME remove");
                    Statement::TweenClipRemove(id, name)
                }
                _ => Statement::TweenClip(
                    Box::new(selector(&id)?),
                    name,
                    Box::new(options(4, true)?),
                ),
            }
        }
        "track" => {
            let clip = arg(3)?.to_owned();
            match arg(4)? {
                "add" => {
                    let property = arg(5)?.to_owned();
                    let mut target = String::new();
                    for t in &tokens[6..] {
                        match t.split_once('=') {
                            Some(("target", v)) => target = text(v),
                            _ => bail!("unexpected '{t}' (expected target=ID)"),
                        }
                    }
                    Statement::TweenTrackAdd(id, clip, property, target)
                }
                "remove" => {
                    ensure!(tokens.len() == 6, "use: tween ID track CLIP remove I");
                    Statement::TweenTrackRemove(id, clip, index(5, "track")?)
                }
                other => bail!("unknown tween track verb '{other}' (add, remove)"),
            }
        }
        "key" => {
            let clip = arg(3)?.to_owned();
            let track = index(4, "track")?;
            let edit = match arg(5)? {
                "add" => {
                    let time = number(arg(6)?, "key time")?;
                    let (mut value, mut ease) = (None, None);
                    for t in &tokens[7..] {
                        if let Some(e) = t.strip_prefix("ease=") {
                            ease = Some(crate::tween::Ease::parse(e)?);
                        } else if value.is_none() {
                            value = Some(numbers(t)?);
                        } else {
                            bail!("unexpected '{t}' (expected one value and ease=E)");
                        }
                    }
                    KeyEdit::Add { time, value, ease }
                }
                "move" => {
                    ensure!(tokens.len() == 8, "use: tween ID key CLIP TRACK move I T");
                    KeyEdit::Move {
                        index: index(6, "key")?,
                        time: number(arg(7)?, "key time")?,
                    }
                }
                "remove" => {
                    ensure!(tokens.len() == 7, "use: tween ID key CLIP TRACK remove I");
                    KeyEdit::Remove(index(6, "key")?)
                }
                other => bail!("unknown tween key verb '{other}' (add, move, remove)"),
            };
            Statement::TweenKey(id, clip, track, edit)
        }
        other => bail!("unknown tween verb '{other}' (clip, track, key)"),
    })
}

/// `fsm` statements (see GRAMMAR).
fn fsm(tokens: &[String]) -> Result<Statement> {
    use crate::ops::FsmEdit;
    const USE: &str = "use: fsm M create|remove | fsm M state add|remove|rename|set .. | fsm M initial S | fsm M transition add|remove|move|set .. | fsm M node S x,y | fsm SEL agent MACHINE";
    let arg = |n: usize| tokens.get(n).map(String::as_str).context(USE);
    let name = |n: usize| -> Result<String> {
        Ok(match value(arg(n)?) {
            Value::String(s) => s,
            _ => arg(n)?.to_owned(),
        })
    };
    let index = |n: usize| -> Result<usize> {
        arg(n)?
            .parse()
            .context("transition index must be a non-negative integer")
    };
    let count = |n: usize| -> Result<()> {
        ensure!(tokens.len() <= n, "unexpected '{}' ({USE})", tokens[n]);
        Ok(())
    };
    if tokens.get(2).is_some_and(|t| t == "agent") {
        let select = selector(arg(1)?)?;
        let machine = name(3)?;
        let mut radius = None;
        for t in &tokens[4..] {
            match t.split_once('=') {
                Some(("radius", v)) => {
                    radius = Some(v.parse::<f32>().context("radius must be a number")?)
                }
                _ => bail!("unexpected '{t}' (expected radius=M)"),
            }
        }
        return Ok(Statement::FsmAgent(Box::new(select), machine, radius));
    }
    let machine = name(1)?;
    let edit = |e: FsmEdit| Statement::FsmEdit(machine.clone(), e);
    Ok(match arg(2)? {
        "create" => {
            count(4)?;
            Statement::FsmCreate(machine.clone(), tokens.get(3).map(|_| name(3)).transpose()?)
        }
        "remove" => {
            count(3)?;
            Statement::FsmRemove(machine.clone())
        }
        "initial" => {
            count(4)?;
            edit(FsmEdit::SetInitial(name(3)?))
        }
        "node" => {
            count(5)?;
            let parts = numbers(arg(4)?)?;
            ensure!(parts.len() == 2, "node position is x,y");
            Statement::FsmNode(machine.clone(), name(3)?, [parts[0], parts[1]])
        }
        "state" => match arg(3)? {
            "add" => {
                count(6)?;
                let state = tokens.get(5).map(|t| json_value(t)).transpose()?;
                edit(FsmEdit::AddState {
                    name: name(4)?,
                    state,
                })
            }
            "remove" => {
                count(5)?;
                edit(FsmEdit::RemoveState(name(4)?))
            }
            "rename" => {
                count(6)?;
                edit(FsmEdit::RenameState {
                    from: name(4)?,
                    to: name(5)?,
                })
            }
            "set" => {
                count(6)?;
                edit(FsmEdit::SetState {
                    name: name(4)?,
                    state: json_value(arg(5)?)?,
                })
            }
            other => bail!("unknown fsm state verb '{other}' (add, remove, rename, set)"),
        },
        "transition" => match arg(3)? {
            "add" => {
                count(7)?;
                let when = tokens.get(6).map(|t| json_value(t)).transpose()?;
                edit(FsmEdit::AddTransition {
                    from: name(4)?,
                    to: name(5)?,
                    when,
                })
            }
            "remove" => {
                count(5)?;
                edit(FsmEdit::RemoveTransition(index(4)?))
            }
            "move" => {
                count(6)?;
                edit(FsmEdit::MoveTransition(index(4)?, index(5)?))
            }
            "set" => {
                count(6)?;
                edit(FsmEdit::SetTransition(index(4)?, json_value(arg(5)?)?))
            }
            other => bail!("unknown fsm transition verb '{other}' (add, remove, move, set)"),
        },
        other => bail!("unknown fsm verb '{other}' ({USE})"),
    })
}

/// `behavior SEL TEMPLATE [k=v ..]` (Inspector ▸ Behavior templates).
fn behavior(tokens: &[String]) -> Result<Statement> {
    const USE: &str = "use: behavior SEL patrol|guard|dialogue [machine=ID] [tag=T] [range=M] [speed=M/S] [event=E]";
    let (Some(select), Some(template)) = (tokens.get(1), tokens.get(2)) else {
        bail!("{USE}");
    };
    let template = crate::ops::BehaviorTemplate::parse(template)
        .with_context(|| format!("unknown behavior template '{template}' ({USE})"))?;
    let mut o = crate::ops::BehaviorOptions::default();
    for t in &tokens[3..] {
        let (k, v) = t
            .split_once('=')
            .with_context(|| format!("option '{t}' must be key=value"))?;
        let text = || match value(v) {
            Value::String(s) => s,
            _ => v.to_owned(),
        };
        let number = || {
            v.parse::<f32>()
                .with_context(|| format!("{k} must be a number"))
        };
        match k {
            "machine" => o.machine = Some(text()),
            "tag" => o.tag = Some(text()),
            "event" => o.event = Some(text()),
            "range" => o.range = Some(number()?),
            "speed" => o.speed = Some(number()?),
            other => bail!("unknown option '{other}' (use machine, tag, range, speed, event)"),
        }
    }
    Ok(Statement::Behavior(
        Box::new(selector(select)?),
        template,
        Box::new(o),
    ))
}

/// A key value: `90`, `1,2,3` or `[1,2,3]`.
fn numbers(text: &str) -> Result<Vec<f32>> {
    text.trim_matches(['[', ']'])
        .split(',')
        .map(|p| p.trim().parse::<f32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("'{text}' must be a number or x,y,z numbers"))
}

fn vec3(text: &str) -> Result<[f32; 3]> {
    let parts = text
        .trim_matches(['[', ']'])
        .split(',')
        .map(|p| p.trim().parse::<f32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("'{text}' must be x,y,z numbers"))?;
    ensure!(parts.len() == 3, "'{text}' must have three components");
    Ok([parts[0], parts[1], parts[2]])
}

fn spawn(
    world: &World,
    prefab: &str,
    opts: &[(String, String)],
    taken: &mut std::collections::BTreeSet<String>,
) -> Result<Vec<Edit>> {
    ensure!(
        world.prefabs.contains_key(prefab),
        "prefab '{prefab}' not found"
    );
    let get = |k: &str| {
        opts.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    for (k, _) in opts {
        ensure!(
            ["count", "id", "from", "at", "step", "tag", "label"].contains(&k.as_str()),
            "unknown spawn option '{k}'"
        );
    }
    let count: usize = get("count")
        .map_or(Ok(1), str::parse)
        .context("count must be an integer")?;
    ensure!((1..=256).contains(&count), "spawn count must be 1..256");
    let from: usize = get("from")
        .map_or(Ok(0), str::parse)
        .context("from must be an integer")?;
    let pattern = get("id").map_or_else(|| format!("{prefab}/{{i}}"), str::to_owned);
    ensure!(
        count == 1 || pattern.contains("{i}"),
        "id pattern needs {{i}} when count > 1"
    );
    let at = get("at").map(vec3).transpose()?;
    let step = get("step").map(vec3).transpose()?.unwrap_or([0.0; 3]);
    let tags: Vec<String> = get("tag")
        .map(|t| t.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let mut edits = vec![];
    for n in 0..count {
        let i = from + n;
        let id = pattern.replace("{i}", &i.to_string());
        valid_id(&id)?;
        if world.entity(&id).is_some() || !taken.insert(id.clone()) {
            return Err(Diagnostic::error(
                "id_taken",
                format!("/entities/@{id}"),
                format!("entity '{id}' already exists"),
            ));
        }
        let mut e = Entity::new(id);
        e.prefab = Some(prefab.into());
        e.tags = tags.clone();
        e.label = get("label").map(|l| {
            if count > 1 {
                format!("{l} {i}")
            } else {
                l.to_owned()
            }
        });
        if let Some(at) = at {
            let p = [0, 1, 2].map(|k| at[k] + step[k] * n as f32);
            e.components
                .insert("core.transform".into(), json!({"position":p}));
        }
        edits.push(Edit::Core(CoreEdit::PutEntity { entity: e }));
    }
    Ok(edits)
}

fn short(v: &Value) -> String {
    let s = crate::local::display_value(v).to_string();
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(59).collect::<String>())
    } else {
        s
    }
}

fn edit_summary(result: &Value, preview: bool) -> String {
    let diff = &result["diff"];
    let mut out = format!(
        "{} rev={} changes={}",
        if preview {
            "preview"
        } else if result["changed"] == false {
            "unchanged"
        } else {
            "ok"
        },
        result["rev"],
        diff["total_changes"]
    );
    let rows = diff["changes"].as_array().cloned().unwrap_or_default();
    for row in rows.iter().take(12) {
        let path = row["path"].as_str().unwrap_or("?");
        match row["kind"].as_str() {
            Some("added") => out.push_str(&format!("\n+ {path} = {}", short(&row["after"]))),
            Some("removed") => out.push_str(&format!("\n- {path}")),
            _ => out.push_str(&format!(
                "\n~ {path}: {} -> {}",
                short(&row["before"]),
                short(&row["after"])
            )),
        }
    }
    let total = diff["total_changes"].as_u64().unwrap_or(0) as usize;
    if total > 12 {
        out.push_str(&format!("\n… +{} more", total - 12));
        if !preview {
            if let Some(rev) = result["rev"].as_i64() {
                out.push_str(&format!(" (diff {} {rev})", rev - 1));
            }
        }
    }
    out
}

fn lines(text: String) -> Value {
    json!({"ok":true,"format":"lines","text":text})
}

pub fn run(
    store: &mut Store,
    text: &str,
    preview: bool,
    expected_rev: Option<i64>,
    propose: bool,
) -> Result<Value> {
    ensure!(text.len() <= 65536, "ACL script exceeds 64 KiB");
    ensure!(
        !(preview && propose),
        "use either --preview or --propose (a proposal already includes a preview)"
    );
    // Editors on Windows often save UTF-8 with a byte order mark; it is not part of line 1.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut statements = vec![];
    for (n, raw) in text.lines().enumerate() {
        if raw.trim_start().starts_with('#') {
            continue;
        }
        for line in statements_in(raw).into_iter().map(str::trim) {
            if line.is_empty() {
                continue;
            }
            let tokens = tokenize(line).map_err(|e| diag(n + 1, e))?;
            let stmt = parse(&tokens).map_err(|e| diag(n + 1, format!("{e:#}")))?;
            statements.push((n + 1, stmt));
        }
    }
    ensure!(!statements.is_empty(), "empty ACL script");
    let reads = statements.iter().filter(|(_, s)| !s.is_edit()).count();
    if reads > 0 {
        if statements.len() != 1 {
            return Err(diag(
                statements[0].0,
                "read statements (select/inspect/history/diff/undo/conflicts/assets/usages) must be alone in a script",
            ));
        }
        let (line, stmt) = statements.pop().unwrap();
        return read(store, stmt, expected_rev).map_err(|e| {
            if e.downcast_ref::<Diagnostic>().is_some() {
                e
            } else {
                diag(line, format!("{e:#}"))
            }
        });
    }
    let (rev, world) = store.current()?;
    if let Some(e) = expected_rev {
        if e != rev {
            return Err(Diagnostic::error(
                "revision_conflict",
                "/expected_rev",
                format!("revision conflict: expected {e}, current {rev}"),
            ));
        }
    }
    let mut edits = vec![];
    let mut warnings: Vec<String> = vec![];
    let mut taken = std::collections::BTreeSet::new();
    let mut scratch = world.clone();
    for (line, stmt) in statements {
        let wrap = |e: anyhow::Error| {
            if e.downcast_ref::<Diagnostic>().is_some() {
                e
            } else {
                diag(line, format!("{e:#}"))
            }
        };
        // Each statement compiles against the world as left by the previous statements, so
        // generated IDs stay unique and later lines can refer to objects created earlier.
        let w = &scratch;
        let r = &store.registry;
        let compiled = match stmt {
            Statement::Set(select, f, v) => {
                if let Value::Array(items) = &v {
                    warnings.extend(dropped_items(w, r, &select, &f, items, line));
                }
                Ok(vec![Edit::Core(CoreEdit::SetWhere {
                    select,
                    field: f,
                    value: v,
                })])
            }
            Statement::Spawn(opts, prefab) => spawn(w, &prefab, &opts, &mut taken),
            Statement::Delete(select) => {
                if select.from != Collection::Entities {
                    return Err(diag(line, "delete only removes entities"));
                }
                entity_ids(w, r, &select, line).and_then(|ids| crate::ops::delete(w, &ids))
            }
            Statement::Remove(select, component) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::remove_component(w, &ids, &component)),
            Statement::Create(preset, place) => {
                crate::ops::create_object(w, &preset, &place).map(|c| c.edits)
            }
            Statement::Instantiate(prefab, place) => {
                crate::ops::instantiate_prefab(w, &prefab, &place).map(|c| c.edits)
            }
            Statement::Duplicate(select) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::duplicate(w, &ids))
                .map(|c| c.edits),
            Statement::Group(select, id) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::create_group(w, &ids, id.as_deref()))
                .map(|c| c.edits),
            Statement::Reparent(id, parent, keep_world) => {
                crate::ops::reparent(w, &id, parent.as_deref(), keep_world)
            }
            Statement::SceneCreate(id, active) => crate::ops::create_scene(w, &id, active),
            Statement::SceneActive(id, active) => crate::ops::set_scene_active(w, &id, active),
            Statement::SceneDelete(id) => crate::ops::delete_scene(w, &id),
            Statement::SceneMember(scene, select, member) => entity_ids(w, r, &select, line)
                .and_then(|ids| {
                    ids.iter().try_fold(vec![], |mut all, id| {
                        all.extend(crate::ops::set_scene_membership(w, &scene, id, member)?);
                        Ok(all)
                    })
                }),
            Statement::CameraMain(id) => crate::ops::make_main_camera(w, &id),
            Statement::CameraFollow(select, target, options) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::add_camera_follow(w, &ids, &target, &options)),
            Statement::CameraBlocker(select, on) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::set_camera_blocker(w, &ids, on)),
            Statement::MigrateCameraBlockers => crate::ops::migrate_camera_blocker_tags(w),
            Statement::Rename(id, label) => crate::ops::rename(w, &id, &label),
            Statement::Move(id, to) => crate::ops::move_entity(w, &id, &to),
            Statement::PrefabCreate(id, entity) => {
                crate::ops::create_prefab(w, id.as_deref(), &entity).map(|c| c.edits)
            }
            Statement::Ground(select) => {
                entity_ids(w, r, &select, line).and_then(|ids| crate::ops::drop_to_ground(w, &ids))
            }
            Statement::Follow(select, path, options) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::follow_path(w, &ids, &path, &options)),
            Statement::PathPoint(id, edit) => crate::ops::edit_path_point(w, &id, &edit),
            Statement::NavAgent(select, options) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::add_nav_agent(w, &ids, &options)),
            Statement::NavTarget(select, target) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::set_nav_target(w, &ids, target.as_ref())),
            Statement::Interact(select, options) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::make_interactable(w, &ids, &options)),
            Statement::AudioClip(id, preset) => crate::ops::add_audio_clip(w, &id, &preset),
            Statement::AudioSource(select, options) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::add_audio_source(w, &ids, &options)),
            Statement::AudioListener(id) => crate::ops::add_audio_listener(w, &id),
            Statement::TweenClip(select, name, options) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::add_tween_clip(w, &ids, &name, &options)),
            Statement::TweenClipSet(id, name, options) => {
                crate::ops::set_tween_clip(w, &id, &name, &options)
            }
            Statement::TweenClipRemove(id, name) => crate::ops::remove_tween_clip(w, &id, &name),
            Statement::TweenTrackAdd(id, clip, property, target) => {
                crate::ops::add_tween_track(w, &id, &clip, &property, &target)
            }
            Statement::TweenTrackRemove(id, clip, index) => {
                crate::ops::remove_tween_track(w, &id, &clip, index)
            }
            Statement::TweenKey(id, clip, track, edit) => {
                crate::ops::edit_tween_key(w, &id, &clip, track, &edit)
            }
            Statement::Tag(select, add, remove) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::edit_tags(w, &ids, &add, &remove)),
            Statement::WorldId(id) => crate::ops::set_world_id(w, &id),
            Statement::WorldTitle(title) => crate::ops::set_world_title(w, &title),
            Statement::CapabilityAdd(id) => crate::ops::add_capability(w, r, &id),
            Statement::CapabilityRemove(id) => crate::ops::remove_capability(w, r, &id),
            Statement::SystemAdd(id) => crate::ops::add_system(w, r, &id),
            Statement::SystemRemove(id) => crate::ops::remove_system(w, r, &id),
            Statement::ResourceCreate(id, value) => crate::ops::create_resource(w, r, &id, value),
            Statement::ResourceRemove(id) => crate::ops::remove_resource(w, &id),
            Statement::AddComponent(select, component, value) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::add_component(w, r, &ids, &component, value)),
            Statement::Array(select, f, edit) => crate::ops::edit_array(w, r, &select, &f, &edit),
            Statement::Reset(select, component) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::reset_component(w, r, &ids, &component)),
            Statement::EventDeclare(name, fields) => crate::ops::declare_event(w, &name, &fields),
            Statement::Rotate(select, degrees) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::set_rotation(w, &ids, degrees)),
            Statement::Ui(stmt) => ui_edits(w, r, &stmt),
            Statement::Tuning(stmt) => tuning_edits(w, &stmt),
            Statement::FsmCreate(machine, initial) => {
                crate::ops::fsm_create_machine(w, r, &machine, initial.as_deref())
            }
            Statement::FsmRemove(machine) => crate::ops::fsm_remove_machine(w, &machine),
            Statement::FsmEdit(machine, edit) => crate::ops::fsm_edit(w, &machine, &edit),
            Statement::FsmNode(machine, state, at) => {
                crate::ops::fsm_set_node(w, &machine, &state, at)
            }
            Statement::FsmAgent(select, machine, radius) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::fsm_add_agent(w, r, &ids, &machine, radius)),
            Statement::Behavior(select, template, options) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::add_behavior(w, r, &ids, template, &options)),
            Statement::AssetPlace(kind, id, place) => {
                crate::ops::place_asset(w, r, kind, &id, &place).map(|c| c.edits)
            }
            Statement::AssetCreate(new) => crate::ops::create_asset(w, r, &new),
            Statement::AssetRename(kind, id, new) => crate::ops::rename_asset(w, kind, &id, &new),
            _ => unreachable!(),
        }
        .map_err(wrap)?;
        for edit in &compiled {
            edit.clone().apply(&mut scratch, r).map_err(wrap)?;
        }
        edits.extend(compiled);
    }
    if edits.is_empty() {
        // e.g. `move X end` when X is already last: nothing to commit, not an error.
        return Ok(with_warnings(
            json!({"ok":true,"format":"lines","rev":rev,"text":format!("unchanged rev={rev} changes=0")}),
            &warnings,
        ));
    }
    ensure!(
        edits.len() <= 256,
        "script expands to {} edits; the batch limit is 256",
        edits.len()
    );
    if propose {
        let note: String = text.trim().chars().take(2048).collect();
        let result = store
            .execute(Request::Propose {
                expected_rev: Some(rev),
                edits,
                note,
            })
            .map_err(with_fix_hint)?;
        let summary = edit_summary(&json!({"rev":rev,"diff":result["diff"]}), true);
        let body = summary.split_once('\n').map_or("", |(_, rest)| rest);
        let mut text = format!(
            "proposal #{} pending base_rev={rev} changes={}",
            result["proposal"], result["diff"]["total_changes"]
        );
        if !body.is_empty() {
            text.push('\n');
            text.push_str(body);
        }
        return Ok(with_warnings(
            json!({"ok":true,"format":"lines","proposal":result["proposal"],"text":text}),
            &warnings,
        ));
    }
    let result = store
        .execute(if preview {
            Request::Preview {
                expected_rev: Some(rev),
                edits,
            }
        } else {
            Request::Apply {
                expected_rev: Some(rev),
                edits,
            }
        })
        .map_err(with_fix_hint)?;
    Ok(with_warnings(
        json!({"ok":true,"format":"lines","rev":result["rev"],"text":edit_summary(&result, preview)}),
        &warnings,
    ))
}

/// Appends warning lines to the text output and lists them under `warnings`.
fn with_warnings(mut out: Value, warnings: &[String]) -> Value {
    if !warnings.is_empty() {
        let text = format!(
            "{}\n{}",
            out["text"].as_str().unwrap_or(""),
            warnings.join("\n")
        );
        out["text"] = json!(text);
        out["warnings"] = json!(warnings);
    }
    out
}

/// `set` that replaces a resource-side list (bindings, rules, widgets, nodes) without some of
/// its current items: those items are dropped silently by design of `set`, so say so. Entity
/// arrays (mesh parts, vectors, path points) are usually replaced on purpose and not checked.
fn dropped_items(
    w: &World,
    r: &Registry,
    select: &Selection,
    field: &str,
    new: &[Value],
    line: usize,
) -> Vec<String> {
    if select.from == Collection::Entities {
        return vec![];
    }
    let Ok(found) = crate::local::matches(w, r, select) else {
        return vec![];
    };
    found
        .iter()
        .filter_map(|(target, data)| {
            let old = data.pointer(field)?.as_array()?;
            let lost = dropped(old, new);
            (lost > 0).then(|| {
                format!(
                    "warn line {line}: set replaced {}{field} ({} -> {} items); {lost} current item(s) are not in the new value (dropped, or changed without an id). To change one item use append/insert SEL FIELD [INDEX] VALUE or remove SEL FIELD INDEX",
                    target.path(),
                    old.len(),
                    new.len()
                )
            })
        })
        .collect()
}

/// Items of `old` that `new` no longer has. An object item whose `id` (or, for bindings,
/// `event`) still exists in `new` was edited, not dropped; the items of its own lists are
/// checked the same way (e.g. a binding that lost one of its calls).
fn dropped(old: &[Value], new: &[Value]) -> usize {
    let identity = |v: &Value| {
        ["id", "event"]
            .iter()
            .find_map(|k| v.get(k).filter(|x| x.is_string()).map(|x| (*k, x.clone())))
    };
    old.iter()
        .map(|item| {
            if new.contains(item) {
                return 0;
            }
            let Some((key, id)) = identity(item) else {
                return 1;
            };
            let Some(next) = new.iter().find(|n| n.get(key) == Some(&id)) else {
                return 1;
            };
            item.as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| Some((v.as_array()?, k)))
                .map(|(list, k)| {
                    let empty = vec![];
                    dropped(
                        list,
                        next.get(k).and_then(Value::as_array).unwrap_or(&empty),
                    )
                })
                .sum()
        })
        .sum()
}

/// Commit errors about an event no `behavior.program` declares get the fix in the message.
fn with_fix_hint(e: anyhow::Error) -> anyhow::Error {
    let Some(d) = e.downcast_ref::<Diagnostic>() else {
        let text = format!("{e:#}");
        return match crate::ops::undeclared_event(&text) {
            Some(name) => anyhow::anyhow!("{text}; declare it first: event declare {name}"),
            None => e,
        };
    };
    match crate::ops::undeclared_event(&d.message) {
        Some(name) => Diagnostic::error(
            d.code.clone(),
            d.path.clone(),
            format!("{}; declare it first: event declare {name}", d.message),
        ),
        None => e,
    }
}

fn read(store: &mut Store, stmt: Statement, expected_rev: Option<i64>) -> Result<Value> {
    Ok(match stmt {
        Statement::Select(sel, aliases) => {
            let mut out = store.execute(Request::Select { query: sel })?;
            if let Some(text) = out["text"].as_str() {
                let mut text = text.to_owned();
                for (pointer, alias) in &aliases {
                    text = text.replace(&format!(" {pointer}="), &format!(" {alias}="));
                }
                out["text"] = json!(text);
            }
            out
        }
        Statement::Inspect(id) => {
            let mut s = Selection::all(Collection::Entities);
            s.ids.push(id.clone());
            s.fields = vec![
                "/label".into(),
                "/prefab".into(),
                "/tags".into(),
                "/components".into(),
            ];
            s.max_bytes = Some(65536);
            let out = store.execute(Request::Select { query: Box::new(s) })?;
            let item = out["items"]
                .get(0)
                .with_context(|| format!("entity '{id}' not found"))?;
            let d = &item["data"];
            let mut text = id.clone();
            for (k, key) in [("/label", "label"), ("/prefab", "prefab")] {
                if d[k].is_string() {
                    text.push_str(&format!(" {key}={}", d[k]));
                }
            }
            if d["/tags"].as_array().is_some_and(|t| !t.is_empty()) {
                text.push_str(&format!(" tags={}", d["/tags"]));
            }
            for (component, v) in d["/components"].as_object().into_iter().flatten() {
                text.push_str(&format!(
                    "\n  {component} {}",
                    crate::local::display_value(v)
                ));
            }
            text.push_str(&format!("\nrev={}", out["rev"]));
            lines(text)
        }
        Statement::History(n) => {
            let out = store.execute(Request::History { limit: Some(n) })?;
            let rows = out["revisions"].as_array().cloned().unwrap_or_default();
            lines(
                rows.iter()
                    .map(|r| {
                        let mut line = format!(
                            "{} {} {}",
                            r["rev"],
                            r["reason"].as_str().unwrap_or(""),
                            r["created_at"].as_str().unwrap_or("")
                        );
                        if let Some(author) = r["author"].as_str() {
                            line.push_str(&format!(" by={author}"));
                        }
                        if let Some(proposal) = r["proposal"].as_i64() {
                            line.push_str(&format!(" proposal=#{proposal}"));
                        }
                        line
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
        Statement::Diff(a, b) => {
            let out = store.execute(Request::Diff {
                from_rev: a,
                to_rev: b,
                offset: 0,
                limit: Some(64),
            })?;
            let mut text = format!("diff {a}..{b} changes={}", out["total_changes"]);
            for row in out["changes"].as_array().into_iter().flatten() {
                text.push_str(&format!(
                    "\n{} {} {} -> {}",
                    match row["kind"].as_str() {
                        Some("added") => "+",
                        Some("removed") => "-",
                        _ => "~",
                    },
                    row["path"].as_str().unwrap_or("?"),
                    short(&row["before"]),
                    short(&row["after"])
                ));
            }
            if let Some(next) = out["next_offset"].as_u64() {
                text.push_str(&format!("\n… more from offset {next}"));
            }
            lines(text)
        }
        Statement::Conflicts(id) => {
            let out = store.proposal_conflicts(id)?;
            let missing = || "(none)".to_owned();
            let mut text = format!(
                "conflicts proposal=#{id} base_rev={} rev={} count={} proposal_only={} current_only={}",
                out["base_rev"],
                out["rev"],
                out["conflicts"].as_array().map_or(0, Vec::len),
                out["proposal_only"],
                out["current_only"]
            );
            for c in out["conflicts"].as_array().into_iter().flatten() {
                let value = |key: &str| c.get(key).map_or_else(missing, short);
                text.push_str(&format!(
                    "\n{} base={} proposal={} current={}",
                    c["path"].as_str().unwrap_or("?"),
                    value("base"),
                    value("mine"),
                    value("theirs")
                ));
            }
            lines(text)
        }
        Statement::Undo(rev) => {
            let out = store.execute(Request::Undo { rev, expected_rev })?;
            lines(format!(
                "ok rev={} restored_from={}",
                out["rev"], out["restored_from"]
            ))
        }
        Statement::Assets(kind) => {
            let (rev, world) = store.current()?;
            let counts = crate::assets::reference_counts(&world);
            let rows: Vec<String> = crate::assets::list(&world)
                .into_iter()
                .filter(|a| kind.is_none_or(|k| k == a.kind))
                .map(|a| {
                    let refs = counts.get(&(a.kind, a.id.clone())).copied().unwrap_or(0);
                    match &a.path {
                        Some(p) => format!(
                            "{} {} file={} refs={refs}",
                            a.kind.name(),
                            a.id,
                            crate::assets::file_name(p)
                        ),
                        None => format!("{} {} refs={refs}", a.kind.name(), a.id),
                    }
                })
                .collect();
            let mut text = format!("assets={} rev={rev}", rows.len());
            for row in rows {
                text.push('\n');
                text.push_str(&row);
            }
            lines(text)
        }
        Statement::Usages(kind, id) => {
            let (rev, world) = store.current()?;
            if !crate::assets::exists(&world, kind, &id) {
                return Err(Diagnostic::error(
                    "not_found",
                    "/asset",
                    format!(
                        "no {} asset '{id}' (list them: assets {})",
                        kind.name(),
                        kind.name()
                    ),
                ));
            }
            let found = crate::assets::usages(&world, kind, &id);
            let mut text = format!("usages {}:{id}={} rev={rev}", kind.name(), found.len());
            for usage in &found {
                text.push('\n');
                text.push_str(&usage.line());
            }
            lines(text)
        }
        _ => unreachable!(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokenizer_keeps_json_and_selectors_whole() {
        assert_eq!(
            tokenize(
                r#"set entities[tag=enemy, has=core.health] core.transform/position [1, 2, 3]"#
            )
            .unwrap(),
            [
                "set",
                "entities[tag=enemy, has=core.health]",
                "core.transform/position",
                "[1, 2, 3]"
            ]
        );
        assert_eq!(
            tokenize(r#"set a label "two words""#).unwrap()[3],
            r#""two words""#
        );
        assert!(tokenize("set a b [1,2").is_err());
    }
    #[test]
    fn short_fields_map_components() {
        assert_eq!(
            field("core.health/value", Collection::Entities),
            "/components/core.health/value"
        );
        assert_eq!(field("label", Collection::Entities), "/label");
        assert_eq!(field("/raw/x", Collection::Entities), "/raw/x");
        assert_eq!(field("start", Collection::Resources), "/start");
    }
    #[test]
    fn where_clause_forms_agree() {
        for line in [
            "select e/* where core.health/value<=50",
            "select e/* where core.health/value <= 50",
        ] {
            let Statement::Select(s, _) = parse(&tokenize(line).unwrap()).unwrap() else {
                panic!()
            };
            assert_eq!(s.filter[0].op, Op::Le);
            assert_eq!(s.filter[0].field, "/components/core.health/value");
            assert_eq!(s.filter[0].value, json!(50));
        }
    }
}
