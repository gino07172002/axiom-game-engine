use crate::*;
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
/// Longest proposal note in UTF-8 bytes (ACL `--propose` cuts its script to fit).
pub const MAX_PROPOSAL_NOTE_BYTES: usize = 2048;
/// Shared decoded copy of the newest known revision.
pub type LatestRevision = std::sync::Arc<std::sync::Mutex<Option<(i64, std::sync::Arc<World>)>>>;
/// Aborts the SQL statement a `Store` connection is running, from any thread.
pub struct Interrupt(rusqlite::InterruptHandle);
impl Interrupt {
    pub fn interrupt(&self) {
        self.0.interrupt();
    }
}
/// Who committed a revision (`revisions.author`). Revisions without one (initialize,
/// migrations, databases from before the column) are reported without an author. An
/// approved proposal is recognizable from `proposals.resolved_rev`; its author is whoever
/// approved it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Author {
    /// A person in the editor.
    Human,
    /// An agent: the bridge, the CLI, or editor commands sent over the bridge.
    Agent,
}
impl Author {
    pub fn as_str(self) -> &'static str {
        match self {
            Author::Human => "human",
            Author::Agent => "agent",
        }
    }
}
pub struct Store {
    connection: Connection,
    pub registry: Registry,
    /// Decoded copy of the newest revision read or committed. Revisions are append-only,
    /// so a cached revision never goes stale; a newer row (from any connection) is
    /// detected by `MAX(rev)`. Connections to the same database in one process may share it.
    latest: LatestRevision,
    /// Recorded on every revision this connection commits.
    author: Option<Author>,
}
impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, Registry::core(), World::empty("world"))
    }
    pub fn memory() -> Result<Self> {
        Self::memory_with(Registry::core(), World::empty("world"))
    }
    pub fn open_with(path: impl AsRef<Path>, registry: Registry, initial: World) -> Result<Self> {
        Self::open_lazy(path, registry, || Ok(initial))
    }
    /// `open_with`, but the first revision's world is only built (e.g. a content package
    /// loaded) when the database is new; an existing database never calls `initial`.
    pub fn open_lazy(
        path: impl AsRef<Path>,
        registry: Registry,
        initial: impl FnOnce() -> Result<World>,
    ) -> Result<Self> {
        if let Some(p) = path.as_ref().parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(p)?;
        }
        Self::from_connection(Connection::open(path)?, registry, initial)
    }
    pub fn memory_with(registry: Registry, initial: World) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, registry, || Ok(initial))
    }
    fn from_connection(
        mut connection: Connection,
        registry: Registry,
        initial: impl FnOnce() -> Result<World>,
    ) -> Result<Self> {
        connection.busy_timeout(Duration::from_secs(3))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS revisions(rev INTEGER PRIMARY KEY AUTOINCREMENT,world TEXT NOT NULL,reason TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));")?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS request_receipts(request_id TEXT PRIMARY KEY,payload TEXT NOT NULL,response TEXT NOT NULL);")?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS proposals(id INTEGER PRIMARY KEY AUTOINCREMENT,status TEXT NOT NULL DEFAULT 'pending',note TEXT NOT NULL,base_rev INTEGER NOT NULL,edits TEXT NOT NULL,preview TEXT NOT NULL,resolved_rev INTEGER,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Databases from before `author`: add the column (nullable, so older builds keep
        // writing). Inside the write transaction, so connections opening at once cannot
        // both add it.
        let has_author: bool = tx.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('revisions') WHERE name='author'",
            [],
            |r| r.get::<_, i64>(0).map(|n| n > 0),
        )?;
        if !has_author {
            tx.execute_batch("ALTER TABLE revisions ADD COLUMN author TEXT")?;
        }
        // Proposals from before `targets` (what each `set_where` selected when the proposal
        // was previewed) keep NULL: approving one with a selector needs a new proposal.
        let has_targets: bool = tx.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('proposals') WHERE name='targets'",
            [],
            |r| r.get::<_, i64>(0).map(|n| n > 0),
        )?;
        if !has_targets {
            tx.execute_batch("ALTER TABLE proposals ADD COLUMN targets TEXT")?;
        }
        // `history` looks up the approved proposal of each revision; without an index that is
        // a scan of every proposal row (edits and previews included) per revision listed.
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS proposals_resolved_rev ON proposals(resolved_rev)",
        )?;
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
            let initial = initial()?;
            initial.compile(&registry)?;
            registry.validate_write(&initial, None)?;
            tx.execute(
                "INSERT INTO revisions(world,reason) VALUES (?1,'initialize')",
                [serde_json::to_string(&initial)?],
            )?;
        }
        tx.commit()?;
        Ok(Self {
            connection,
            registry,
            latest: Default::default(),
            author: None,
        })
    }
    /// The author recorded on revisions this connection commits from now on.
    pub fn set_author(&mut self, author: Option<Author>) {
        self.author = author;
    }
    pub fn author(&self) -> Option<Author> {
        self.author
    }
    /// The stored JSON of revision `rev` (decode it off the main thread when it is large).
    pub fn revision_json(&self, rev: i64) -> Result<String> {
        self.connection
            .query_row("SELECT world FROM revisions WHERE rev=?1", [rev], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| {
                Diagnostic::error("not_found", "/rev", format!("revision {rev} not found"))
            })
    }
    /// A proposal's base revision, edits and status.
    pub fn proposal_edits(&self, id: i64) -> Result<(i64, Vec<Edit>, String)> {
        let (base, edits, status) = self.proposal_raw(id)?;
        Ok((base, serde_json::from_str(&edits)?, status))
    }
    /// `proposal_edits` with the edits still as stored JSON (decode them off the main thread).
    pub fn proposal_raw(&self, id: i64) -> Result<(i64, String, String)> {
        let (base, edits, status): (i64, String, String) = self
            .connection
            .query_row(
                "SELECT base_rev,edits,status FROM proposals WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| {
                Diagnostic::error("not_found", "/id", format!("proposal {id} not found"))
            })?;
        Ok((base, edits, status))
    }
    /// Conflicts between pending proposal `id` (its edits on its base revision) and the
    /// current revision, for `resolve_proposal {keep}`.
    pub fn proposal_conflicts(&self, id: i64) -> Result<Value> {
        let (base_rev, edits, status) = self.proposal_edits(id)?;
        ensure!(status == "pending", "proposal {id} is already {status}");
        let (rev, theirs) = self.current()?;
        let base = self
            .registry
            .decode(serde_json::from_str(&self.revision_json(base_rev)?)?)?;
        let (mine, _) = crate::ops::review_edits(&base, &edits, &self.registry)?;
        let view = crate::ops::merge_view(&base, &mine, &theirs);
        Ok(
            json!({"ok":true,"proposal":id,"base_rev":base_rev,"rev":rev,"conflicts":view.conflicts,"proposal_only":view.mine_only,"current_only":view.theirs_only}),
        )
    }
    /// Newest revision as a shared world. After a commit through this store, or a
    /// repeated read, this costs one `MAX(rev)` query instead of a read and JSON decode.
    pub fn current_shared(&self) -> Result<(i64, std::sync::Arc<World>)> {
        let rev = self.revision()?;
        if let Some((cached, world)) = self.latest.lock().unwrap().as_ref() {
            if *cached == rev {
                return Ok((rev, world.clone()));
            }
        }
        let (rev, world) = self.current()?;
        let world = std::sync::Arc::new(world);
        self.remember(rev, world.clone());
        Ok((rev, world))
    }
    /// `schema`/`discover` topics. `typed:ID` also finds the current world's own
    /// `component_schemas` / `resource_schemas`; every other topic is registry-only.
    fn schema_topic(&self, topic: Option<&str>) -> Result<Value> {
        match topic.and_then(|t| t.strip_prefix("typed:")) {
            Some(id) => {
                let (_, world) = self.current_shared()?;
                crate::typed_schema::typed_schema(&self.registry, &world, id)
                    .with_context(|| format!("unknown component or resource {id}"))
            }
            None => self.registry.schema(topic),
        }
    }
    /// The answer to a request that reads the registry only (`schema` / `discover` of any
    /// topic except `typed:ID`), without a database; `None` when the request needs the world.
    /// A client can answer these before opening (or creating) a store.
    pub fn registry_response(registry: &Registry, request: &Request) -> Option<Result<Value>> {
        let topic = match request {
            Request::Schema { topic } | Request::Discover { topic, .. } => topic.as_deref(),
            _ => return None,
        };
        if topic.is_some_and(|t| t.starts_with("typed:")) {
            return None;
        }
        let schema = registry.schema(topic);
        Some(match request {
            Request::Discover { if_hash, .. } => {
                schema.and_then(|schema| Self::discover_response(schema, if_hash.as_deref()))
            }
            _ => schema,
        })
    }
    /// One page of a change list (`diff`, `proposal_diff`).
    fn diff_page(changes: &[crate::diff::Change], offset: usize, limit: usize) -> Result<Value> {
        ensure!(offset <= changes.len(), "diff offset out of range");
        let end = (offset + limit).min(changes.len());
        Ok(
            json!({"ok":true,"total_changes":changes.len(),"changes":&changes[offset..end],"next_offset":if end<changes.len(){Some(end)}else{None}}),
        )
    }
    fn discover_response(schema: Value, if_hash: Option<&str>) -> Result<Value> {
        let hash = crate::local::fingerprint(&schema)?;
        Ok(if if_hash == Some(hash.as_str()) {
            json!({"ok":true,"hash":hash,"not_modified":true})
        } else {
            json!({"ok":true,"hash":hash,"not_modified":false,"schema":schema})
        })
    }
    /// The `verify` response for one snapshot `(rev, w)`. Callers that also simulate or
    /// measure that world pass the same snapshot, so every part of the answer is about `rev`.
    pub fn verify_world(&self, rev: i64, w: &World) -> Result<Value> {
        let c = w.compile(&self.registry)?;
        // Rules newer than this revision are reported, not enforced: it stays readable.
        let mut diagnostics = crate::camera::deprecations(w);
        diagnostics.extend(self.registry.write_warnings(w));
        Ok(
            json!({"ok":true,"rev":rev,"schema_version":2,"entities":c.len(),"checks":["capabilities","typed_components","resources","references","prefabs","scene_membership","parent_cycles","system_schedule","budgets","package_constraints"],"diagnostics":diagnostics}),
        )
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
    fn remember(&self, rev: i64, world: std::sync::Arc<World>) {
        let mut latest = self.latest.lock().unwrap();
        if latest.as_ref().is_none_or(|(cached, _)| *cached <= rev) {
            *latest = Some((rev, world));
        }
    }
    /// The decoded-revision cache, to share with another connection to the same database
    /// in this process (e.g. the bridge thread), so a revision committed by one is read by
    /// the other without decoding it again.
    pub fn latest_cache(&self) -> LatestRevision {
        self.latest.clone()
    }
    /// Use `cache` from a connection to the same database file.
    pub fn share_latest_cache(&mut self, cache: LatestRevision) {
        self.latest = cache;
    }
    /// A handle another thread can use to abort the statement this connection is running
    /// (the transaction then rolls back). Used by watchdogs of background writers.
    pub fn interrupt_handle(&self) -> Interrupt {
        Interrupt(self.connection.get_interrupt_handle())
    }
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
        // A preview only reads one consistent `(rev, world)` snapshot (WAL): it must not hold
        // the write lock while it validates, or concurrent commits wait for it.
        let behavior = if commit {
            TransactionBehavior::Immediate
        } else {
            TransactionBehavior::Deferred
        };
        let tx = self.connection.transaction_with_behavior(behavior)?;
        let mut committed = None;
        let result = Self::edit_in(
            &tx,
            &self.registry,
            expected_rev,
            edits,
            commit,
            &mut committed,
            self.author,
        )?;
        if commit {
            tx.commit()?;
            if let (Some(world), Some(rev)) = (committed, result["rev"].as_i64()) {
                self.remember(rev, std::sync::Arc::new(world));
            }
        }
        Ok(result)
    }
    fn edit_in(
        tx: &Connection,
        registry: &Registry,
        expected_rev: Option<i64>,
        edits: Vec<Edit>,
        commit: bool,
        committed: &mut Option<World>,
        author: Option<Author>,
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
        Edit::apply_all(edits, &mut next, registry)?;
        let compiled = next.compile(registry)?;
        let before = previous.compile(registry)?;
        registry.validate_write(&next, Some(&previous))?;
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
            "INSERT INTO revisions(world,reason,author) VALUES (?1,'apply',?2)",
            params![serde_json::to_string(&next)?, author.map(Author::as_str)],
        )?;
        let rev = tx.last_insert_rowid();
        *committed = Some(next);
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
                let schema = self.schema_topic(topic.as_deref())?;
                Self::discover_response(schema, if_hash.as_deref())
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
                ensure!(
                    note.len() <= MAX_PROPOSAL_NOTE_BYTES,
                    "proposal note exceeds {MAX_PROPOSAL_NOTE_BYTES} bytes"
                );
                let base = match expected_rev {
                    Some(rev) => rev,
                    None => self.revision()?,
                };
                let payload = serde_json::to_string(&edits)?;
                let selects = edits.iter().any(is_set_where);
                let targets = if selects { Some(edits.clone()) } else { None };
                let preview = self.edit(Some(base), edits, false)?;
                let diff = serde_json::to_string(&preview["diff"])?;
                // What each selector resolved to in the reviewed preview, so approval can
                // refuse to commit a different selection (`targets_changed`).
                let targets = match targets {
                    Some(edits) => {
                        let world = self
                            .registry
                            .decode(serde_json::from_str(&self.revision_json(base)?)?)?;
                        Self::set_where_targets(&self.registry, world, &edits)?
                    }
                    None => vec![],
                };
                let targets = serde_json::to_string(&targets)?;
                // Count and insert in one short write transaction, so concurrent proposers
                // cannot both see 255 pending and overfill the queue.
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                let pending: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM proposals WHERE status='pending'",
                    [],
                    |r| r.get(0),
                )?;
                ensure!(
                    pending < 256,
                    "proposal queue is full; resolve pending proposals first"
                );
                tx.execute(
                    "INSERT INTO proposals(note,base_rev,edits,preview,targets) VALUES (?1,?2,?3,?4,?5)",
                    params![note, base, payload, diff, targets],
                )?;
                let id = tx.last_insert_rowid();
                tx.commit()?;
                Ok(
                    json!({"ok":true,"proposal":id,"status":"pending","base_rev":base,"diff":preview["diff"]}),
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
                keep,
                expected_rev,
            } => {
                ensure!(
                    keep.is_none() || (approve && !rebase),
                    "keep merges an approval onto the current revision; it cannot be combined with rebase or a rejection"
                );
                ensure!(
                    keep.is_none() || expected_rev.is_some(),
                    "keep needs expected_rev: the revision the conflicts were listed against (acl: conflicts N prints rev=)"
                );
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                if let Some(e) = expected_rev {
                    let current: i64 =
                        tx.query_row("SELECT MAX(rev) FROM revisions", [], |r| r.get(0))?;
                    if e != current {
                        return Err(Diagnostic::error(
                            "revision_conflict",
                            "/expected_rev",
                            format!("revision conflict: expected {e}, current {current}"),
                        ));
                    }
                }
                let (status, base, edits, targets): (String, i64, String, Option<String>) = tx
                    .query_row(
                        "SELECT status,base_rev,edits,targets FROM proposals WHERE id=?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
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
                let expected = if rebase || keep.is_some() {
                    tx.query_row("SELECT MAX(rev) FROM revisions", [], |r| r.get(0))?
                } else {
                    base
                };
                let mut edits: Vec<Edit> = serde_json::from_str(&edits)?;
                let stored = edits.clone();
                let budget = |e| Self::split_hint(&tx, &self.registry, id, base, &stored, e);
                // Each `set_where` must still select, on the reviewed base revision, exactly
                // what the reviewed preview selected: a change in selector semantics (e.g.
                // exact integer comparison) must not retarget a reviewed proposal.
                if edits.iter().any(is_set_where) {
                    let current: i64 =
                        tx.query_row("SELECT MAX(rev) FROM revisions", [], |r| r.get(0))?;
                    let Some(targets) = targets else {
                        // Stored before targets were recorded (and before batches shared one
                        // budget): say how to split it when the new proposal must be split.
                        if let Some(e) =
                            Self::over_budget(&tx, &self.registry, id, base, &stored, None)
                        {
                            return Err(e);
                        }
                        return Err(Diagnostic::error(
                            "stale_selector",
                            "/id",
                            format!(
                                "proposal {id} selects objects (set_where) but was stored before the targets of its preview were recorded, so approving it could change objects its review never showed; review `proposal_diff {id}`, then propose the same edits again on rev {current} (and reject this one)"
                            ),
                        ));
                    };
                    let reviewed: Vec<Vec<String>> = serde_json::from_str(&targets)?;
                    let raw: String =
                        tx.query_row("SELECT world FROM revisions WHERE rev=?1", [base], |r| {
                            r.get(0)
                        })?;
                    let base_world = self.registry.decode(serde_json::from_str(&raw)?)?;
                    let now = Self::set_where_targets(&self.registry, base_world, &edits)
                        .map_err(budget)?;
                    if now != reviewed {
                        return Err(Diagnostic::error(
                            "stale_selector",
                            "/id",
                            format!(
                                "proposal {id}'s set_where now selects different objects on its base rev {base} than its reviewed preview did; review `proposal_diff {id}`, then propose again on rev {current}"
                            ),
                        ));
                    }
                }
                // A selector re-evaluated on a newer revision can change objects the reviewed
                // preview never showed (e.g. entities tagged after the proposal was made, or an
                // ID that only exists now). Each `set_where` must resolve to targets the
                // reviewed base revision also resolved it to.
                if rebase
                    && expected != base
                    && edits.iter().any(is_set_where)
                    && Self::selectors_grow(&tx, &self.registry, base, &edits).map_err(budget)?
                {
                    return Err(Diagnostic::error(
                        "stale_selector",
                        "/rebase",
                        format!(
                            "proposal {id} selects objects (set_where) on rev {base}, the current revision is {expected}; a rebase would re-select on the new revision. Review `conflicts {id}` and approve with keep (resolve_proposal keep + expected_rev {expected}), or propose again on rev {expected}"
                        ),
                    ));
                }
                if let Some(keep) = &keep {
                    let raw: String =
                        tx.query_row("SELECT world FROM revisions WHERE rev=?1", [base], |r| {
                            r.get(0)
                        })?;
                    let base_world = self.registry.decode(serde_json::from_str(&raw)?)?;
                    let (_, theirs) = Self::latest_in(&tx, &self.registry)?;
                    let mine = crate::ops::apply_edits(&base_world, &edits, &self.registry)
                        .map_err(budget)?;
                    edits = crate::ops::merge(&base_world, &mine, &theirs, keep)?;
                    if edits.is_empty() {
                        // Everything the proposal changes is already in the current revision.
                        tx.execute("UPDATE proposals SET status='approved' WHERE id=?1", [id])?;
                        tx.commit()?;
                        return Ok(
                            json!({"ok":true,"proposal":id,"status":"approved","changed":false,"rev":expected}),
                        );
                    }
                }
                let mut result = Self::edit_in(
                    &tx,
                    &self.registry,
                    Some(expected),
                    edits,
                    true,
                    &mut None,
                    self.author,
                )
                .map_err(budget)?;
                // `resolved_rev` names the revision the approval created (none when it changed
                // nothing), so History never credits a proposal with someone else's revision.
                tx.execute(
                    "UPDATE proposals SET status='approved',resolved_rev=?2 WHERE id=?1",
                    params![
                        id,
                        result["rev"].as_i64().filter(|_| result["changed"] == true)
                    ],
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
                let mut page = Self::diff_page(&changes, offset, limit)?;
                page["from_rev"] = json!(from_rev);
                page["to_rev"] = json!(to_rev);
                Ok(page)
            }
            Request::ProposalDiff { id, offset, limit } => {
                let limit = limit.unwrap_or(32);
                ensure!((1..=128).contains(&limit), "diff limit must be 1..128");
                // The same change list the proposal's stored preview summarizes: its edits
                // on its base revision.
                let (base_rev, edits, status) = self.proposal_edits(id)?;
                let base = self
                    .registry
                    .decode(serde_json::from_str(&self.revision_json(base_rev)?)?)?;
                let (mine, _) = crate::ops::review_edits(&base, &edits, &self.registry)?;
                let changes = crate::diff::changes(&json!(base), &json!(mine));
                let mut page = Self::diff_page(&changes, offset, limit)?;
                page["proposal"] = json!(id);
                page["status"] = json!(status);
                page["base_rev"] = json!(base_rev);
                Ok(page)
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
            Request::Command { .. }
            | Request::Save { .. }
            | Request::Load { .. }
            | Request::EditorView { .. }
            | Request::Screenshot { .. }
            | Request::ReloadAssets { .. } => {
                anyhow::bail!("runtime request requires a live engine connection")
            }
            Request::Schema { topic } => self.schema_topic(topic.as_deref()),
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
                let result = Self::undo_in(&tx, &self.registry, rev, expected_rev, self.author)?;
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
                self.verify_world(rev, &w)
            }
            Request::Observe {} => {
                let (rev, w) = self.current()?;
                Ok(
                    json!({"rev":rev,"runtime":false,"entities":w.entities.len(),"hint":"use --connect 127.0.0.1:4242 observe for runtime activation and telemetry"}),
                )
            }
            Request::History { limit } => {
                // `author` and the approved `proposal` are left out when there is none.
                let mut s = self.connection.prepare(
                    "SELECT r.rev,r.reason,r.created_at,r.author,(SELECT p.id FROM proposals p WHERE p.resolved_rev=r.rev AND p.status='approved' LIMIT 1) FROM revisions r ORDER BY r.rev DESC LIMIT ?1",
                )?;
                let rows = s.query_map([limit.unwrap_or(12).min(100) as i64], |r| {
                    let mut row = json!({"rev":r.get::<_,i64>(0)?,"reason":r.get::<_,String>(1)?,"created_at":r.get::<_,String>(2)?});
                    if let Some(author) = r.get::<_, Option<String>>(3)? {
                        row["author"] = json!(author);
                    }
                    if let Some(proposal) = r.get::<_, Option<i64>>(4)? {
                        row["proposal"] = json!(proposal);
                    }
                    Ok(row)
                })?;
                Ok(json!({"revisions":rows.collect::<rusqlite::Result<Vec<_>>>()?}))
            }
        }
    }
    fn undo_in(
        tx: &Connection,
        registry: &Registry,
        rev: i64,
        expected_rev: Option<i64>,
        author: Option<Author>,
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
            "INSERT INTO revisions(world,reason,author) VALUES (?1,?2,?3)",
            params![
                serde_json::to_string(&world)?,
                format!("restore:{rev}"),
                author.map(Author::as_str)
            ],
        )?;
        Ok(json!({"ok":true,"rev":tx.last_insert_rowid(),"restored_from":rev}))
    }
    /// Whether a `set_where` of `edits`, replayed on the newest revision, selects a target its
    /// replay on revision `base` (what the proposal's preview showed) does not. Both replays
    /// apply the edits in order, so each selector sees the batch's earlier edits. A replay
    /// that fails on the newest revision is left to the commit, which reports it.
    fn selectors_grow(
        tx: &Connection,
        registry: &Registry,
        base: i64,
        edits: &[Edit],
    ) -> Result<bool> {
        let raw: String =
            tx.query_row("SELECT world FROM revisions WHERE rev=?1", [base], |r| {
                r.get(0)
            })?;
        let mut reviewed = registry.decode(serde_json::from_str(&raw)?)?;
        let (_, mut current) = Self::latest_in(tx, registry)?;
        let (mut reviewed_budget, mut current_budget) = (
            crate::local::MAX_SET_WHERE_BYTES,
            crate::local::MAX_SET_WHERE_BYTES,
        );
        for edit in edits {
            if let Edit::Core(CoreEdit::SetWhere { select, .. }) = edit {
                // A target's path names its kind and full identity (a choice with its node).
                let targets = |w: &World| -> Result<std::collections::BTreeSet<String>> {
                    Ok(crate::local::matches(w, registry, select)?
                        .into_iter()
                        .map(|(t, _)| t.path())
                        .collect())
                };
                let Ok(now) = targets(&current) else {
                    return Ok(false);
                };
                let then = targets(&reviewed)?;
                if !now.is_subset(&then) {
                    return Ok(true);
                }
            }
            edit.clone()
                .apply_budgeted(&mut reviewed, registry, &mut reviewed_budget)?;
            if edit
                .clone()
                .apply_budgeted(&mut current, registry, &mut current_budget)
                .is_err()
            {
                return Ok(false);
            }
        }
        Ok(false)
    }
    /// The targets (sorted paths) each `set_where` of `edits` selects when the batch is
    /// replayed in order on `world`. Replays with the review budget: the commit enforces
    /// the shared one.
    fn set_where_targets(
        registry: &Registry,
        mut world: World,
        edits: &[Edit],
    ) -> Result<Vec<Vec<String>>> {
        let mut budget = crate::ops::review_budget(edits.len());
        let mut out = vec![];
        for edit in edits {
            if let Edit::Core(CoreEdit::SetWhere { select, .. }) = edit {
                let targets: std::collections::BTreeSet<String> =
                    crate::local::matches(&world, registry, select)?
                        .into_iter()
                        .map(|(t, _)| t.path())
                        .collect();
                out.push(targets.into_iter().collect());
            }
            edit.clone()
                .apply_budgeted(&mut world, registry, &mut budget)?;
        }
        Ok(out)
    }
    /// `e` as an approval error of proposal `id`: a `budget_exceeded` (a proposal stored
    /// before its batch shared one `set_where` budget) says how to split the batch. The
    /// proposal stays pending (the approval transaction rolls back).
    fn split_hint(
        tx: &Connection,
        registry: &Registry,
        id: i64,
        base: i64,
        edits: &[Edit],
        e: anyhow::Error,
    ) -> anyhow::Error {
        if e.downcast_ref::<Diagnostic>()
            .is_none_or(|d| d.code != "budget_exceeded")
        {
            return e;
        }
        Self::over_budget(tx, registry, id, base, edits, Some(&e)).unwrap_or(e)
    }
    /// The split-the-batch error of proposal `id` when its batch expands beyond one shared
    /// `set_where` budget on its base revision (or `cause` already says so).
    fn over_budget(
        tx: &Connection,
        registry: &Registry,
        id: i64,
        base: i64,
        edits: &[Edit],
        cause: Option<&anyhow::Error>,
    ) -> Option<anyhow::Error> {
        const MAX: usize = crate::local::MAX_SET_WHERE_BYTES;
        let used = (|| -> Result<Vec<usize>> {
            let raw: String =
                tx.query_row("SELECT world FROM revisions WHERE rev=?1", [base], |r| {
                    r.get(0)
                })?;
            let world = registry.decode(serde_json::from_str(&raw)?)?;
            Ok(crate::ops::review_edits(&world, edits, registry)?.1)
        })();
        let (total, size) = match used {
            // The largest batch size whose consecutive batches all fit one budget.
            Ok(used) => (
                used.iter().sum::<usize>(),
                (1..=edits.len())
                    .rev()
                    .find(|&k| used.chunks(k).all(|c| c.iter().sum::<usize>() <= MAX))
                    .unwrap_or(1),
            ),
            Err(_) => (0, 1),
        };
        if cause.is_none() && total <= MAX {
            return None;
        }
        let cause = cause.map(|e| format!(" ({e:#})")).unwrap_or_default();
        Some(Diagnostic::error(
            "budget_exceeded",
            "/id",
            format!(
                "proposal {id} exceeds the batch budget: its {} edits expand to {total} set_where bytes, a batch may expand to {MAX} (it was stored before a batch's edits shared one budget). It stays pending and `proposal_diff {id}` still shows it; split it into batches of at most {size} edits, propose those again on the current revision and reject this one{cause}",
                edits.len()
            ),
        ))
    }
    fn latest_in(tx: &Connection, registry: &Registry) -> Result<(i64, World)> {
        let (rev, raw): (i64, String) = tx.query_row(
            "SELECT rev,world FROM revisions ORDER BY rev DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((rev, registry.decode(serde_json::from_str(&raw)?)?))
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
            } => Self::edit_in(
                &tx,
                &self.registry,
                expected_rev,
                edits,
                true,
                &mut None,
                self.author,
            )?,
            Request::Undo { rev, expected_rev } => {
                Self::undo_in(&tx, &self.registry, rev, expected_rev, self.author)?
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
/// Whether `edit` selects its targets when it is applied (`set_where`).
fn is_set_where(edit: &Edit) -> bool {
    matches!(edit, Edit::Core(CoreEdit::SetWhere { .. }))
}
