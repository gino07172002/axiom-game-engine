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
EDIT (any number, atomic):
  set SEL F VALUE [where F OP V [and ...]]
  spawn PREFAB [count=N] [id=enemy/{i}] [from=0] [at=x,y,z] [step=x,y,z] [tag=a,b] [label=TEXT]
  delete SEL
 editor operations (same code as the editor UI; P = [id=ID] [at=x,y,z] [parent=ID] [scene=ID]):
  create empty|cube|camera P      new object; default id = preset name, then NAME/2, NAME/3..
  instantiate PREFAB P            one instance, default id = prefab id (+/2..); spawn = bulk, tags, labels
  duplicate SEL                   copies keep parent+scenes; ids SRC/copy, SRC/copy/2..
                                  (entities[id=a,id=b] keeps that order; otherwise ID order)
  reparent ID PARENT|root [keep=world|local]   default keeps world pose (editor drag)
  scene create ID [on|off] | scene activate ID on|off | scene delete ID (objects kept)
  scene member SCENE SEL on|off
  camera main ID                  active=true on ID, false on every other camera
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
    Set(Box<Selection>, String, Value),
    Spawn(Vec<(String, String)>, String),
    Delete(Box<Selection>),
    Create(String, Box<crate::ops::Placement>),
    Instantiate(String, Box<crate::ops::Placement>),
    Duplicate(Box<Selection>),
    Reparent(String, Option<String>, bool),
    SceneCreate(String, bool),
    SceneActive(String, bool),
    SceneDelete(String),
    SceneMember(String, Box<Selection>, bool),
    CameraMain(String),
}

impl Statement {
    fn is_edit(&self) -> bool {
        !matches!(
            self,
            Self::Select(..) | Self::Inspect(_) | Self::History(_) | Self::Diff(..) | Self::Undo(_)
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
            "parent" => p.parent = Some(v.into()),
            "scene" => p.scene = Some(v.into()),
            other => bail!("unknown option '{other}' (use id, at, parent, scene)"),
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
        "create" => Statement::Create(arg(1)?.clone(), Box::new(placement(&tokens[2..])?)),
        "instantiate" => {
            Statement::Instantiate(arg(1)?.clone(), Box::new(placement(&tokens[2..])?))
        }
        "duplicate" => {
            ensure!(tokens.len() == 2, "duplicate takes one selector");
            Statement::Duplicate(Box::new(selector(arg(1)?)?))
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
        "camera" => {
            ensure!(
                arg(1)? == "main" && tokens.len() == 3,
                "use: camera main ID"
            );
            Statement::CameraMain(arg(2)?.clone())
        }
        other => bail!("unknown statement '{other}'"),
    })
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
                "read statements (select/inspect/history/diff/undo) must be alone in a script",
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
            Statement::Set(select, f, v) => Ok(vec![Edit::Core(CoreEdit::SetWhere {
                select,
                field: f,
                value: v,
            })]),
            Statement::Spawn(opts, prefab) => spawn(w, &prefab, &opts, &mut taken),
            Statement::Delete(select) => {
                if select.from != Collection::Entities {
                    return Err(diag(line, "delete only removes entities"));
                }
                entity_ids(w, r, &select, line).and_then(|ids| crate::ops::delete(w, &ids))
            }
            Statement::Create(preset, place) => {
                crate::ops::create_object(w, &preset, &place).map(|c| c.edits)
            }
            Statement::Instantiate(prefab, place) => {
                crate::ops::instantiate_prefab(w, &prefab, &place).map(|c| c.edits)
            }
            Statement::Duplicate(select) => entity_ids(w, r, &select, line)
                .and_then(|ids| crate::ops::duplicate(w, &ids))
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
            _ => unreachable!(),
        }
        .map_err(wrap)?;
        for edit in &compiled {
            edit.clone().apply(&mut scratch, r).map_err(wrap)?;
        }
        edits.extend(compiled);
    }
    ensure!(
        edits.len() <= 256,
        "script expands to {} edits; the batch limit is 256",
        edits.len()
    );
    if propose {
        let note: String = text.trim().chars().take(2048).collect();
        let result = store.execute(Request::Propose {
            expected_rev: Some(rev),
            edits,
            note,
        })?;
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
        return Ok(json!({"ok":true,"format":"lines","proposal":result["proposal"],"text":text}));
    }
    let result = store.execute(if preview {
        Request::Preview {
            expected_rev: Some(rev),
            edits,
        }
    } else {
        Request::Apply {
            expected_rev: Some(rev),
            edits,
        }
    })?;
    Ok(
        json!({"ok":true,"format":"lines","rev":result["rev"],"text":edit_summary(&result, preview)}),
    )
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
                        format!(
                            "{} {} {}",
                            r["rev"],
                            r["reason"].as_str().unwrap_or(""),
                            r["created_at"].as_str().unwrap_or("")
                        )
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
        Statement::Undo(rev) => {
            let out = store.execute(Request::Undo { rev, expected_rev })?;
            lines(format!(
                "ok rev={} restored_from={}",
                out["rev"], out["restored_from"]
            ))
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
