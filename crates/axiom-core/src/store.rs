use crate::*;
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
pub struct Store {
    connection: Connection,
    pub registry: Registry,
}
impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, Registry::core(), World::empty("world"))
    }
    pub fn memory() -> Result<Self> {
        Self::memory_with(Registry::core(), World::empty("world"))
    }
    pub fn open_with(path: impl AsRef<Path>, registry: Registry, initial: World) -> Result<Self> {
        if let Some(p) = path.as_ref().parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(p)?;
        }
        Self::from_connection(Connection::open(path)?, registry, initial)
    }
    pub fn memory_with(registry: Registry, initial: World) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, registry, initial)
    }
    fn from_connection(
        mut connection: Connection,
        registry: Registry,
        initial: World,
    ) -> Result<Self> {
        connection.busy_timeout(Duration::from_secs(3))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS revisions(rev INTEGER PRIMARY KEY AUTOINCREMENT,world TEXT NOT NULL,reason TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));")?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS request_receipts(request_id TEXT PRIMARY KEY,payload TEXT NOT NULL,response TEXT NOT NULL);")?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS proposals(id INTEGER PRIMARY KEY AUTOINCREMENT,status TEXT NOT NULL DEFAULT 'pending',note TEXT NOT NULL,base_rev INTEGER NOT NULL,edits TEXT NOT NULL,preview TEXT NOT NULL,resolved_rev INTEGER,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(i64, String)> = tx
            .query_row(
                "SELECT rev,world FROM revisions ORDER BY rev DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((_, raw)) = old {
            let value: Value = serde_json::from_str(&raw)?;
            let version = value["schema_version"].as_u64();
            let w = registry.decode(value)?;
            w.compile(&registry)?;
            if version != Some(2) {
                tx.execute(
                    "INSERT INTO revisions(world,reason) VALUES (?1,'migrate:1->2')",
                    [serde_json::to_string(&w)?],
                )?;
            }
        } else {
            initial.compile(&registry)?;
            tx.execute(
                "INSERT INTO revisions(world,reason) VALUES (?1,'initialize')",
                [serde_json::to_string(&initial)?],
            )?;
        }
        tx.commit()?;
        Ok(Self {
            connection,
            registry,
        })
    }
    pub fn current(&self) -> Result<(i64, World)> {
        let (rev, raw): (i64, String) = self.connection.query_row(
            "SELECT rev,world FROM revisions ORDER BY rev DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((rev, self.registry.decode(serde_json::from_str(&raw)?)?))
    }
    /// Cheap change signal for polling UIs: (pending count, newest proposal ID).
    pub fn proposal_signature(&self) -> Result<(i64, i64)> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FILTER (WHERE status='pending'),COALESCE(MAX(id),0) FROM proposals",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }
    pub fn revision(&self) -> Result<i64> {
        Ok(self
            .connection
            .query_row("SELECT MAX(rev) FROM revisions", [], |r| r.get(0))?)
    }
    pub fn apply(&mut self, expected_rev: Option<i64>, edits: Vec<Edit>) -> Result<Value> {
        self.edit(expected_rev, edits, true)
    }
    fn edit(&mut self, expected_rev: Option<i64>, edits: Vec<Edit>, commit: bool) -> Result<Value> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = Self::edit_in(&tx, &self.registry, expected_rev, edits, commit)?;
        if commit {
            tx.commit()?;
        }
        Ok(result)
    }
    fn edit_in(
        tx: &Connection,
        registry: &Registry,
        expected_rev: Option<i64>,
        edits: Vec<Edit>,
        commit: bool,
    ) -> Result<Value> {
        ensure!(
            !edits.is_empty() && edits.len() <= 256,
            "batch must contain 1..256 edits"
        );
        let (rev, raw): (i64, String) = tx.query_row(
            "SELECT rev,world FROM revisions ORDER BY rev DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if let Some(e) = expected_rev {
            if e != rev {
                return Err(Diagnostic::error(
                    "revision_conflict",
                    "/expected_rev",
                    format!("revision conflict: expected {e}, current {rev}"),
                ));
            }
        }
        let previous = registry.decode(serde_json::from_str(&raw)?)?;
        let mut next = previous.clone();
        for (i, edit) in edits.into_iter().enumerate() {
            crate::local::at(&format!("/edits/{i}"), edit.apply(&mut next, registry))?;
        }
        let compiled = next.compile(registry)?;
        let before = previous.compile(registry)?;
        let added = compiled
            .indices
            .keys()
            .filter(|id| !before.indices.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        let removed = before
            .indices
            .keys()
            .filter(|id| !compiled.indices.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        let changed = compiled
            .entities
            .iter()
            .filter(|e| {
                before
                    .indices
                    .get(&e.id)
                    .is_some_and(|&i| before.entities[i].components != e.components)
            })
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let layout = previous.entities.iter().any(|e| {
            next.entity(&e.id).is_none_or(|n| {
                n.components.get("physics.collider") != e.components.get("physics.collider")
                    || n.components.get("core.transform") != e.components.get("core.transform")
            })
        });
        let mut diff = crate::diff::summary(&previous, &next);
        diff.as_object_mut().unwrap().extend(json!({"added":added.len(),"removed":removed.len(),"changed":changed.len(),"sample_ids":added.iter().chain(removed.iter()).chain(changed.iter()).take(16).collect::<Vec<_>>(),"layout_changed":layout}).as_object().unwrap().clone());
        if !commit {
            return Ok(
                json!({"ok":true,"preview":true,"rev":rev,"changed":previous!=next,"diff":diff,"entities":compiled.len(),"diagnostics":[]}),
            );
        }
        if previous == next {
            return Ok(json!({"ok":true,"rev":rev,"changed":false,"entities_delta":0,"diff":diff}));
        }
        tx.execute(
            "INSERT INTO revisions(world,reason) VALUES (?1,'apply')",
            [serde_json::to_string(&next)?],
        )?;
        let rev = tx.last_insert_rowid();
        Ok(
            json!({"ok":true,"rev":rev,"changed":true,"entities_delta":compiled.len() as i64-before.len() as i64,"diff":diff,"activation":"authoring_committed"}),
        )
    }
    pub fn execute(&mut self, request: Request) -> Result<Value> {
        match request {
            Request::SceneTree { query } => {
                let (rev, world) = self.current()?;
                crate::scene_edit::tree(&world, rev, &query)
            }
            Request::Discover { topic, if_hash } => {
                let schema = self.registry.schema(topic.as_deref())?;
                let hash = crate::local::fingerprint(&schema)?;
                if if_hash.as_ref() == Some(&hash) {
                    Ok(json!({"ok":true,"hash":hash,"not_modified":true}))
                } else {
                    Ok(json!({"ok":true,"hash":hash,"not_modified":false,"schema":schema}))
                }
            }
            Request::Select { query } => {
                let (rev, w) = self.current()?;
                crate::local::select(&w, &self.registry, rev, &query)
            }
            Request::Acl {
                text,
                preview,
                expected_rev,
                propose,
            } => crate::acl::run(self, &text, preview, expected_rev, propose),
            Request::Propose {
                expected_rev,
                edits,
                note,
            } => {
                ensure!(note.len() <= 2048, "proposal note exceeds 2 KiB");
                let base = match expected_rev {
                    Some(rev) => rev,
                    None => self.revision()?,
                };
                let payload = serde_json::to_string(&edits)?;
                let preview = self.edit(Some(base), edits, false)?;
                let pending: i64 = self.connection.query_row(
                    "SELECT COUNT(*) FROM proposals WHERE status='pending'",
                    [],
                    |r| r.get(0),
                )?;
                ensure!(
                    pending < 256,
                    "proposal queue is full; resolve pending proposals first"
                );
                self.connection.execute(
                    "INSERT INTO proposals(note,base_rev,edits,preview) VALUES (?1,?2,?3,?4)",
                    params![
                        note,
                        base,
                        payload,
                        serde_json::to_string(&preview["diff"])?
                    ],
                )?;
                Ok(
                    json!({"ok":true,"proposal":self.connection.last_insert_rowid(),"status":"pending","base_rev":base,"diff":preview["diff"]}),
                )
            }
            Request::Proposals { status, limit } => {
                let limit = limit.unwrap_or(16);
                ensure!((1..=64).contains(&limit), "proposal limit must be 1..64");
                if let Some(s) = &status {
                    ensure!(
                        ["pending", "approved", "rejected"].contains(&s.as_str()),
                        "status must be pending, approved or rejected"
                    );
                }
                let mut q = self.connection.prepare(
                    "SELECT id,status,note,base_rev,preview,resolved_rev,created_at FROM proposals WHERE (?1 IS NULL OR status=?1) ORDER BY id DESC LIMIT ?2",
                )?;
                let rows = q
                    .query_map(params![status, limit as i64], |r| {
                        let preview: String = r.get(4)?;
                        Ok(json!({"id":r.get::<_,i64>(0)?,"status":r.get::<_,String>(1)?,"note":r.get::<_,String>(2)?,"base_rev":r.get::<_,i64>(3)?,"diff":serde_json::from_str::<Value>(&preview).unwrap_or(Value::Null),"resolved_rev":r.get::<_,Option<i64>>(5)?,"created_at":r.get::<_,String>(6)?}))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(json!({"ok":true,"proposals":rows}))
            }
            Request::ResolveProposal {
                id,
                approve,
                rebase,
            } => {
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                let (status, base, edits): (String, i64, String) = tx
                    .query_row(
                        "SELECT status,base_rev,edits FROM proposals WHERE id=?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?
                    .ok_or_else(|| {
                        Diagnostic::error("not_found", "/id", format!("proposal {id} not found"))
                    })?;
                if status != "pending" {
                    return Err(Diagnostic::error(
                        "proposal_resolved",
                        "/id",
                        format!("proposal {id} is already {status}"),
                    ));
                }
                if !approve {
                    tx.execute("UPDATE proposals SET status='rejected' WHERE id=?1", [id])?;
                    tx.commit()?;
                    return Ok(json!({"ok":true,"proposal":id,"status":"rejected"}));
                }
                let expected = if rebase {
                    tx.query_row("SELECT MAX(rev) FROM revisions", [], |r| r.get(0))?
                } else {
                    base
                };
                let edits: Vec<Edit> = serde_json::from_str(&edits)?;
                let mut result = Self::edit_in(&tx, &self.registry, Some(expected), edits, true)?;
                tx.execute(
                    "UPDATE proposals SET status='approved',resolved_rev=?2 WHERE id=?1",
                    params![id, result["rev"].as_i64()],
                )?;
                tx.commit()?;
                result["proposal"] = json!(id);
                result["status"] = json!("approved");
                Ok(result)
            }
            Request::Diff {
                from_rev,
                to_rev,
                offset,
                limit,
            } => {
                let limit = limit.unwrap_or(32);
                ensure!((1..=128).contains(&limit), "diff limit must be 1..128");
                let read = |rev: i64| -> Result<Value> {
                    let raw: String = self.connection.query_row(
                        "SELECT world FROM revisions WHERE rev=?1",
                        [rev],
                        |r| r.get(0),
                    )?;
                    Ok(json!(self.registry.decode(serde_json::from_str(&raw)?)?))
                };
                let changes = crate::diff::changes(&read(from_rev)?, &read(to_rev)?);
                ensure!(offset <= changes.len(), "diff offset out of range");
                let end = (offset + limit).min(changes.len());
                Ok(
                    json!({"ok":true,"from_rev":from_rev,"to_rev":to_rev,"total_changes":changes.len(),"changes":&changes[offset..end],"next_offset":if end<changes.len(){Some(end)}else{None}}),
                )
            }
            Request::Execute {
                request_id,
                runtime_session,
                request,
            } => {
                ensure!(
                    runtime_session.is_none(),
                    "authoring execute must not specify runtime_session"
                );
                self.execute_once(&request_id, *request)
            }
            Request::ObserveDelta { .. } => {
                anyhow::bail!("observe_delta requires a live engine connection")
            }
            Request::Command { .. } | Request::Save { .. } | Request::Load { .. } => {
                anyhow::bail!("runtime request requires a live engine connection")
            }
            Request::Schema { topic } => self.registry.schema(topic.as_deref()),
            Request::Apply {
                expected_rev,
                edits,
            } => self.apply(expected_rev, edits),
            Request::Preview {
                expected_rev,
                edits,
            } => self.edit(expected_rev, edits, false),
            Request::Undo { rev, expected_rev } => {
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                let result = Self::undo_in(&tx, &self.registry, rev, expected_rev)?;
                tx.commit()?;
                Ok(result)
            }
            Request::Query { scope, limit } => {
                let (rev, w) = self.current()?;
                let c = w.compile(&self.registry)?;
                Ok(match scope {
                    Scope::Packages => {
                        json!({"rev":rev,"packages":w.packages.as_ref().map(|p|p.summary())})
                    }
                    Scope::Summary => {
                        let mut v = json!({"rev":rev,"schema_version":2,"id":w.id,"title":w.title,"entities":c.len(),"capabilities":w.capabilities,"systems":w.systems});
                        for f in &self.registry.summaries {
                            if let Some(m) = f(&w).as_object() {
                                v.as_object_mut().unwrap().extend(m.clone());
                            }
                        }
                        v
                    }
                    Scope::World => json!({"rev":rev,"world":w}),
                    Scope::Entities => {
                        json!({"rev":rev,"total":c.len(),"entities":c.entities.iter().take(limit.unwrap_or(32).min(256)).map(|e|json!({"id":e.id,"tags":e.tags,"components":e.components})).collect::<Vec<_>>() })
                    }
                    Scope::Components => {
                        json!({"rev":rev,"component_schemas":w.component_schemas,"columns":c.columns.keys().collect::<Vec<_>>() })
                    }
                    Scope::Resources => json!({"rev":rev,"resources":w.resources}),
                    Scope::Systems => json!({"rev":rev,"systems":c.schedule}),
                    Scope::Behavior => {
                        json!({"rev":rev,"behaviors":w.resources.iter().filter(|(id,_)|id.ends_with(".behavior")||id.as_str()=="behavior.program"||id.as_str()=="narrative.graph").collect::<std::collections::BTreeMap<_,_>>()})
                    }
                })
            }
            Request::Verify {} => {
                let (rev, w) = self.current()?;
                let c = w.compile(&self.registry)?;
                Ok(
                    json!({"ok":true,"rev":rev,"schema_version":2,"entities":c.len(),"checks":["capabilities","typed_components","resources","references","prefabs","scene_membership","parent_cycles","system_schedule","budgets","package_constraints"],"diagnostics":[]}),
                )
            }
            Request::Observe {} => {
                let (rev, w) = self.current()?;
                Ok(
                    json!({"rev":rev,"runtime":false,"entities":w.entities.len(),"hint":"use --connect 127.0.0.1:4242 observe for runtime activation and telemetry"}),
                )
            }
            Request::History { limit } => {
                let mut s = self.connection.prepare(
                    "SELECT rev,reason,created_at FROM revisions ORDER BY rev DESC LIMIT ?1",
                )?;
                let rows=s.query_map([limit.unwrap_or(12).min(100) as i64],|r|Ok(json!({"rev":r.get::<_,i64>(0)?,"reason":r.get::<_,String>(1)?,"created_at":r.get::<_,String>(2)?})))?;
                Ok(json!({"revisions":rows.collect::<rusqlite::Result<Vec<_>>>()?}))
            }
        }
    }
    fn undo_in(
        tx: &Connection,
        registry: &Registry,
        rev: i64,
        expected_rev: Option<i64>,
    ) -> Result<Value> {
        let current: i64 = tx.query_row("SELECT MAX(rev) FROM revisions", [], |r| r.get(0))?;
        if let Some(e) = expected_rev {
            if e != current {
                return Err(Diagnostic::error(
                    "revision_conflict",
                    "/expected_rev",
                    format!("revision conflict: expected {e}, current {current}"),
                ));
            }
        }
        let raw: String = tx
            .query_row("SELECT world FROM revisions WHERE rev=?1", [rev], |r| {
                r.get(0)
            })
            .context("revision not found")?;
        let world = registry.decode(serde_json::from_str(&raw)?)?;
        world.compile(registry)?;
        tx.execute(
            "INSERT INTO revisions(world,reason) VALUES (?1,?2)",
            params![serde_json::to_string(&world)?, format!("restore:{rev}")],
        )?;
        Ok(json!({"ok":true,"rev":tx.last_insert_rowid(),"restored_from":rev}))
    }
    fn execute_once(&mut self, id: &str, request: Request) -> Result<Value> {
        crate::retry::validate_id(id)?;
        ensure!(
            matches!(request, Request::Apply { .. } | Request::Undo { .. }),
            "durable execute supports apply and undo; runtime operations require a live session"
        );
        let payload = serde_json::to_string(&request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT payload,response FROM request_receipts WHERE request_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old, response)) = previous {
            ensure_same(id, &old, &payload)?;
            return Ok(serde_json::from_str(&response)?);
        }
        let(count,bytes):(i64,i64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(payload AS BLOB))+length(CAST(response AS BLOB))),0) FROM request_receipts",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(
            count < 4096 && bytes + payload.len() as i64 + 1_048_576 <= 67_108_864,
            "durable receipt budget exhausted; use a new database or explicitly archive receipts"
        );
        let mut result = match request {
            Request::Apply {
                expected_rev,
                edits,
            } => Self::edit_in(&tx, &self.registry, expected_rev, edits, true)?,
            Request::Undo { rev, expected_rev } => {
                Self::undo_in(&tx, &self.registry, rev, expected_rev)?
            }
            _ => unreachable!(),
        };
        result["request_id"] = json!(id);
        result["durability"] = json!("sqlite_transaction");
        let response = serde_json::to_string(&result)?;
        ensure!(
            response.len() <= 1_048_576,
            "receipt response exceeds budget"
        );
        tx.execute(
            "INSERT INTO request_receipts(request_id,payload,response) VALUES (?1,?2,?3)",
            params![id, payload, response],
        )?;
        tx.commit()?;
        Ok(result)
    }
}
fn ensure_same(id: &str, a: &str, b: &str) -> Result<()> {
    if a != b {
        return Err(Diagnostic::error(
            "idempotency_conflict",
            "/request_id",
            format!("{id} was already used with a different request"),
        ));
    }
    Ok(())
}
