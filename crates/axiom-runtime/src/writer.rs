//! Editor authoring writes on a dedicated writer thread with its own SQLite connection, so
//! the editor's main thread never waits for durability (WAL fsync), IR compiles or the
//! whole-world JSON round trip of a commit (docs/perf-baseline.zh-TW.md).
//!
//! The queue itself lives on the main thread (`App`): first in, first out, one request in
//! flight, its CAS `expected_rev` resolved when it is sent. Edits queued behind a pending
//! commit are built against an optimistic world staged off the main thread by `stage`.
//! The bridge/AI path and the CLI keep their own synchronous stores.
use anyhow::Result;
use axiom_core::{Author, Edit, Interrupt, LatestRevision, Registry, Request, Store, World};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// At most this many editor writes wait or run at once; more are refused with a reason.
pub const MAX_PENDING: usize = 8;
/// Watchdog: a commit running longer is interrupted (its SQLite statement aborts and the
/// transaction rolls back).
pub const COMMIT_TIMEOUT: Duration = Duration::from_secs(30);
/// After an interrupt, how long the writer may take to answer before it is detached.
pub const COMMIT_GRACE: Duration = Duration::from_secs(5);
/// Watchdog of one optimistic staging job.
pub const STAGE_TIMEOUT: Duration = Duration::from_secs(30);
/// Bounded wait for queued commits when the editor closes.
pub const SHUTDOWN_FLUSH: Duration = Duration::from_secs(30);

/// One finished request, in submission order.
pub struct Finished {
    pub seq: u64,
    pub result: Result<Value>,
    /// Writer-thread wall clock of `Store::execute`.
    pub ms: f64,
}

struct Op {
    seq: u64,
    request: Request,
    author: Author,
}

/// The writer thread. Requests run strictly in the order they are sent.
pub struct Writer {
    ops: Option<mpsc::Sender<Op>>,
    results: mpsc::Receiver<Finished>,
    handle: Option<JoinHandle<()>>,
    interrupt: Arc<Mutex<Option<Interrupt>>>,
    /// Set when the main thread gave up on the writer: requests not yet started are refused.
    abort: Arc<AtomicBool>,
}

impl Writer {
    /// Starts the thread; it opens its own connection to `db` (sharing the decoded-revision
    /// cache `latest` with the main connection). `gate`, when given, is locked around each
    /// request (tests hold it to keep requests queued).
    pub fn spawn(
        db: PathBuf,
        registry: Registry,
        latest: LatestRevision,
        gate: Option<Arc<Mutex<()>>>,
    ) -> Result<Self> {
        let (ops, inbox) = mpsc::channel::<Op>();
        let (outbox, results) = mpsc::channel();
        let interrupt = Arc::new(Mutex::new(None));
        let abort = Arc::new(AtomicBool::new(false));
        let (shared_interrupt, shared_abort) = (interrupt.clone(), abort.clone());
        let handle = std::thread::Builder::new()
            .name("axiom-editor-writer".into())
            .spawn(move || {
                let store = Store::open_with(&db, registry, World::empty("world")).map(|mut s| {
                    s.share_latest_cache(latest);
                    *shared_interrupt.lock().unwrap() = Some(s.interrupt_handle());
                    s
                });
                let mut store = match store {
                    Ok(store) => Ok(store),
                    Err(error) => Err(format!("{error:#}")),
                };
                while let Ok(op) = inbox.recv() {
                    let _held = gate.as_ref().map(|g| g.lock());
                    let started = Instant::now();
                    let result = if shared_abort.load(Relaxed) {
                        Err(anyhow::anyhow!(
                            "editor writer was stopped before this edit started; it was not saved"
                        ))
                    } else {
                        match &mut store {
                            Ok(store) => {
                                store.set_author(Some(op.author));
                                store.execute(op.request)
                            }
                            Err(error) => Err(anyhow::anyhow!(
                                "editor writer could not open the database: {error}"
                            )),
                        }
                    };
                    let done = Finished {
                        seq: op.seq,
                        result,
                        ms: started.elapsed().as_secs_f64() * 1000.0,
                    };
                    if outbox.send(done).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            ops: Some(ops),
            results,
            handle: Some(handle),
            interrupt,
            abort,
        })
    }

    pub fn send(&self, seq: u64, request: Request, author: Author) -> Result<()> {
        self.ops
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("editor writer is closed"))?
            .send(Op {
                seq,
                request,
                author,
            })
            .map_err(|_| anyhow::anyhow!("editor writer thread has stopped"))
    }

    /// Non-blocking unless `wait` is non-zero (bounded by the caller).
    pub fn finished(&self, wait: Duration) -> Option<Finished> {
        if wait.is_zero() {
            self.results.try_recv().ok()
        } else {
            self.results.recv_timeout(wait).ok()
        }
    }

    /// Watchdog action: abort the running SQLite statement.
    pub fn interrupt(&self) {
        if let Some(handle) = self.interrupt.lock().unwrap().as_ref() {
            handle.interrupt();
        }
    }

    /// Refuses requests that have not started yet (they answer with an error).
    pub fn abort(&self) {
        self.abort.store(true, Relaxed);
    }

    /// Closes the queue and waits at most `limit` for the thread to finish what it was sent.
    /// Returns whether it was joined; otherwise it is detached.
    pub fn shutdown(&mut self, limit: Duration) -> bool {
        self.ops.take();
        let Some(handle) = self.handle.take() else {
            return true;
        };
        let deadline = Instant::now() + limit;
        while !handle.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = handle.join();
        true
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if self.handle.is_some() && !self.shutdown(Duration::from_secs(2)) {
            eprintln!("editor writer did not finish within 2 s; detached");
        }
    }
}

/// Optimistic world after `edits`: what the editor shows (and builds further edits on)
/// while the commit is queued or running. Compiles it so an invalid edit is refused before
/// the editor swaps its draft for it. Runs on a worker (cloning a 4096-entity world alone
/// is ~8 ms).
pub fn stage(base: &World, edits: &[Edit], registry: &Registry) -> Result<Arc<World>> {
    let mut next = base.clone();
    for (i, edit) in edits.iter().cloned().enumerate() {
        axiom_core::local::at(&format!("/edits/{i}"), edit.apply(&mut next, registry))?;
    }
    next.compile(registry)?;
    Ok(Arc::new(next))
}

/// Compact description of a request for problem messages (never silently lost).
pub fn describe(request: &Request) -> String {
    match request {
        Request::Apply { edits, .. } | Request::Preview { edits, .. } => {
            let text = serde_json::to_string(edits).unwrap_or_default();
            let mut short: String = text.chars().take(160).collect();
            if short.len() < text.len() {
                short.push('…');
            }
            let op = if matches!(request, Request::Apply { .. }) {
                "apply"
            } else {
                "preview"
            };
            format!("{op} of {} edit(s) {short}", edits.len())
        }
        Request::Undo { rev, .. } => format!("restore of revision {rev}"),
        Request::ResolveProposal { id, approve, .. } => {
            format!(
                "{} of proposal #{id}",
                if *approve { "approval" } else { "rejection" }
            )
        }
        other => serde_json::to_value(other)
            .ok()
            .and_then(|v| v["op"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "request".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_db(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axiom-writer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        path
    }

    #[test]
    fn writer_runs_requests_in_order_on_its_own_connection() {
        let path = temp_db("order.sqlite");
        let main = Store::open_with(&path, Registry::core(), World::empty("w")).unwrap();
        let mut writer =
            Writer::spawn(path.clone(), Registry::core(), main.latest_cache(), None).unwrap();
        for (i, title) in ["a", "b", "c"].into_iter().enumerate() {
            writer
                .send(
                    i as u64,
                    Request::Apply {
                        expected_rev: Some(1 + i as i64),
                        edits: vec![Edit::set("title", json!(title))],
                    },
                    if i == 1 { Author::Agent } else { Author::Human },
                )
                .unwrap();
        }
        for i in 0..3 {
            let done = writer.finished(Duration::from_secs(20)).unwrap();
            assert_eq!(done.seq, i);
            assert_eq!(done.result.unwrap()["rev"], 2 + i as i64);
        }
        // The main connection sees the commits through the shared decoded cache.
        let (rev, world) = main.current_shared().unwrap();
        assert_eq!((rev, world.title.as_str()), (4, "c"));
        // Each revision records the author it was sent with.
        let mut main = main;
        let history = main.execute(Request::History { limit: Some(3) }).unwrap();
        let authors: Vec<&str> = history["revisions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["author"].as_str().unwrap())
            .collect();
        assert_eq!(authors, ["human", "agent", "human"]);
        assert!(writer.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn aborted_writer_refuses_requests_that_did_not_start() {
        let path = temp_db("abort.sqlite");
        let main = Store::open_with(&path, Registry::core(), World::empty("w")).unwrap();
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let writer = Writer::spawn(
            path,
            Registry::core(),
            main.latest_cache(),
            Some(gate.clone()),
        )
        .unwrap();
        writer
            .send(
                7,
                Request::Apply {
                    expected_rev: Some(1),
                    edits: vec![Edit::set("title", json!("never"))],
                },
                Author::Human,
            )
            .unwrap();
        writer.abort();
        drop(held);
        let done = writer.finished(Duration::from_secs(20)).unwrap();
        assert_eq!(done.seq, 7);
        assert!(format!("{:#}", done.result.unwrap_err()).contains("not saved"));
        assert_eq!(main.revision().unwrap(), 1);
    }
}
