//! Optional loopback adapter. The engine protocol itself has no network dependency.
use crate::world::Observation;
use anyhow::Result;
use axiom_core::{parse_request, Request, Store};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock,
    },
    thread::JoinHandle,
    time::Duration,
};

pub struct Bridge {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    address: std::net::SocketAddr,
    activity: Arc<Activity>,
}
/// Requests the bridge accepted (one per connection) and when the last one arrived, read by
/// the editor status bar without locking.
#[derive(Default)]
struct Activity {
    requests: AtomicU64,
    /// Milliseconds after `started` of the last request + 1 (0 = none yet).
    last_ms: AtomicU64,
    started: Option<std::time::Instant>,
}
pub struct RuntimeRequest {
    pub deadline: std::time::Instant,
    pub request: Request,
    pub reply: std::sync::mpsc::SyncSender<Value>,
}
impl Bridge {
    pub fn start(
        db: PathBuf,
        port: u16,
        observation: Arc<RwLock<Observation>>,
        registry: axiom_core::Registry,
    ) -> Result<Self> {
        Self::start_with_runtime(db, port, observation, registry, None, None)
    }
    pub fn start_with_runtime(
        db: PathBuf,
        port: u16,
        observation: Arc<RwLock<Observation>>,
        registry: axiom_core::Registry,
        commands: Option<std::sync::mpsc::SyncSender<RuntimeRequest>>,
        latest: Option<axiom_core::LatestRevision>,
    ) -> Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let mut store = Store::open_with(db, registry, axiom_core::World::empty("world"))?;
        // Everything committed through the bridge connection is an agent's.
        store.set_author(Some(axiom_core::Author::Agent));
        if let Some(latest) = latest {
            // Revisions committed here reach the host app without a second JSON decode.
            store.share_latest_cache(latest);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let activity = Arc::new(Activity {
            started: Some(std::time::Instant::now()),
            ..Default::default()
        });
        let seen = activity.clone();
        let handle = std::thread::spawn(move || {
            let mut history = axiom_core::observe::ObservationHistory::default();
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        seen.requests.fetch_add(1, Ordering::Relaxed);
                        let since = seen.started.map_or(0, |t| t.elapsed().as_millis() as u64);
                        seen.last_ms.store(since + 1, Ordering::Relaxed);
                        let result = (|| -> Result<Value> {
                            // Accepted sockets inherit nonblocking mode on Windows.
                            // A JSON line may arrive in several packets; wait up to the budget.
                            stream.set_nonblocking(false)?;
                            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                            let line = read_request_line(&mut stream, &stopped)?;
                            let request = parse_request(line.trim())?;
                            if request.is_runtime() {
                                // A screenshot answers when its PNG is written (read-back and
                                // encode take longer than a command in debug builds).
                                let wait = if matches!(request, Request::Screenshot { .. }) {
                                    SCREENSHOT_WAIT
                                } else {
                                    Duration::from_secs(3)
                                };
                                let (reply, result) = std::sync::mpsc::sync_channel(1);
                                commands
                                    .as_ref()
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("live command adapter unavailable")
                                    })?
                                    .try_send(RuntimeRequest {
                                        request,
                                        reply,
                                        deadline: std::time::Instant::now() + wait,
                                    })
                                    .map_err(|_| {
                                        anyhow::anyhow!(
                                            "runtime command queue full or disconnected"
                                        )
                                    })?;
                                // The main thread answers a request it did not run by its
                                // deadline itself (`runtime_timeout` with both revisions);
                                // the grace lets that reply arrive. Only a main thread that
                                // does not answer even then gets this fallback.
                                match result.recv_timeout(wait + RUNTIME_REPLY_GRACE) {
                                    Ok(reply) => Ok(reply),
                                    Err(_) => Ok(runtime_timeout_fallback(&observation)),
                                }
                            } else if let Request::ObserveDelta {
                                cursor,
                                fields,
                                trace_limit,
                                max_bytes,
                            } = request
                            {
                                let raw = json!(*observation
                                    .read()
                                    .map_err(|_| anyhow::anyhow!("telemetry unavailable"))?);
                                history.observe(
                                    &raw,
                                    cursor.as_deref(),
                                    &fields,
                                    trace_limit,
                                    max_bytes,
                                )
                            } else if matches!(request, Request::Observe {}) {
                                Ok(
                                    json!({"ok":true,"runtime":true,"observation":*observation.read().map_err(|_|anyhow::anyhow!("telemetry unavailable"))?}),
                                )
                            } else {
                                store.execute(request)
                            }
                        })();
                        let response = match result {
                            Ok(value) => value,
                            Err(error) => axiom_core::error_response(&error),
                        };
                        let _ = writeln!(stream, "{response}");
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            stop,
            handle: Some(handle),
            address,
            activity,
        })
    }
    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }
    /// Requests accepted so far and the time since the last one (None before the first).
    pub fn activity(&self) -> (u64, Option<Duration>) {
        let requests = self.activity.requests.load(Ordering::Relaxed);
        let last = self.activity.last_ms.load(Ordering::Relaxed);
        let since = (last > 0).then(|| {
            let now = self
                .activity
                .started
                .map_or(0, |t| t.elapsed().as_millis() as u64);
            Duration::from_millis(now.saturating_sub(last - 1))
        });
        (requests, since)
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            // Reads stop at the flag within one poll; a runtime request may still wait for
            // its reply (up to SCREENSHOT_WAIT), so the join is bounded and the worker is
            // otherwise detached.
            let deadline = std::time::Instant::now() + SHUTDOWN_JOIN;
            while !handle.is_finished() {
                if std::time::Instant::now() >= deadline {
                    eprintln!(
                        "bridge worker did not stop within {} ms; detached",
                        SHUTDOWN_JOIN.as_millis()
                    );
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let _ = handle.join();
        }
    }
}

/// Whole-request budget for reading one JSON line from a connection (not per read).
pub const REQUEST_READ_LIMIT: Duration = Duration::from_secs(2);
/// Largest request line in bytes.
const MAX_REQUEST_BYTES: usize = 1_048_576;
/// Bounded join of the listener worker when the bridge is dropped.
const SHUTDOWN_JOIN: Duration = Duration::from_secs(2);
/// Granularity at which a pending read checks its deadline and the stop flag.
const READ_POLL: Duration = Duration::from_millis(50);

/// Reads one `\n`-terminated request (several packets allowed) within
/// [`REQUEST_READ_LIMIT`] in total, stopping early when `stop` is set, so one slow or
/// silent connection cannot hold the single listener worker (and shutdown) indefinitely.
fn read_request_line(stream: &mut TcpStream, stop: &AtomicBool) -> Result<String> {
    let deadline = std::time::Instant::now() + REQUEST_READ_LIMIT;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        anyhow::ensure!(!stop.load(Ordering::Relaxed), "bridge is shutting down");
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        anyhow::ensure!(
            !left.is_zero(),
            "request not received within {} ms",
            REQUEST_READ_LIMIT.as_millis()
        );
        stream.set_read_timeout(Some(left.min(READ_POLL)))?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let end = chunk[..n].iter().position(|b| *b == b'\n');
                bytes.extend_from_slice(&chunk[..end.map_or(n, |i| i + 1)]);
                anyhow::ensure!(
                    bytes.len() <= MAX_REQUEST_BYTES + 1,
                    "request exceeds limit"
                );
                if end.is_some() {
                    break;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(String::from_utf8(bytes)?)
}

/// How long past a runtime request's deadline the bridge still waits for the main thread's
/// own reply (it answers an expired request on its next frame). Below `call`'s 5 s read
/// timeout together with the 3 s request deadline.
pub const RUNTIME_REPLY_GRACE: Duration = Duration::from_secs(1);

/// Reply when the main thread did not answer a runtime request within its deadline plus
/// [`RUNTIME_REPLY_GRACE`]: the same code as its own expiry reply, with the revisions last
/// published in the observation.
fn runtime_timeout_fallback(observation: &RwLock<Observation>) -> Value {
    let mut reply = axiom_core::error_response(&axiom_core::Diagnostic::error(
        "runtime_timeout",
        "/",
        "the runtime did not answer within the request deadline (main thread busy); observe before retrying",
    ));
    if let Ok(observation) = observation.read() {
        reply["requested_rev"] = json!(observation.requested_rev);
        reply["live_rev"] = json!(observation.rev);
    }
    reply
}

/// How long the bridge waits for a `screenshot` reply: a frame to pick it up
/// (`screenshot::PENDING_LIMIT`) plus the encode job's watchdog (`screenshot::JOB_TIMEOUT`).
pub const SCREENSHOT_WAIT: Duration = Duration::from_secs(32);

pub fn call(address: &str, request: &Request) -> Result<Value> {
    let address: std::net::SocketAddr = address.parse()?;
    anyhow::ensure!(
        address.ip().is_loopback(),
        "bridge connections must use a loopback address"
    );
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    // A screenshot answers when its PNG is written (see `SCREENSHOT_WAIT`).
    let read = if matches!(request, Request::Screenshot { .. }) {
        SCREENSHOT_WAIT + Duration::from_secs(2)
    } else {
        Duration::from_secs(5)
    };
    stream.set_read_timeout(Some(read))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    writeln!(stream, "{}", serde_json::to_string(request)?)?;
    let mut line = String::new();
    BufReader::new(stream.take(4_194_305)).read_line(&mut line)?;
    anyhow::ensure!(line.len() <= 4_194_304, "response exceeds limit");
    Ok(serde_json::from_str(&line)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_core::{Edit, Scope};
    #[test]
    fn fragmented_tcp_request_and_atomic_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "axiom-bridge-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let registry = crate::installed_registry();
        let seed = axiom_fps::demo_world().unwrap();
        let seed_store = Store::open_with(&path, registry.authoring.clone(), seed.clone()).unwrap();
        drop(seed_store);
        let sim = crate::WorldRuntime::new(seed, 1, registry.clone()).unwrap();
        let observation = Arc::new(RwLock::new(sim.observation()));
        let bridge = Bridge::start(path.clone(), 0, observation, registry.authoring).unwrap();
        let mut stream = TcpStream::connect(bridge.address()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(b"{\"op\":").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        stream.write_all(b"\"observe\"}\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["observation"]["rev"], 1);
        let address = bridge.address().to_string();
        let applied = call(
            &address,
            &Request::Apply {
                expected_rev: Some(1),
                edits: vec![Edit::set("enemies.count", json!(12))],
            },
        )
        .unwrap();
        assert_eq!(applied["rev"], 2);
        // Revisions committed over the bridge are an agent's.
        let history = call(&address, &Request::History { limit: Some(1) }).unwrap();
        assert_eq!(history["revisions"][0]["author"], "agent");
        let queried = call(
            &address,
            &Request::Query {
                scope: Scope::Summary,
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(queried["enemies"], 12);
        drop(bridge);
        std::fs::remove_file(path).unwrap();
    }

    /// A connection that trickles bytes of a never-terminated line must not hold the single
    /// listener worker past the whole-request budget, nor the bridge's shutdown.
    #[test]
    fn a_trickling_unterminated_request_cannot_hold_the_bridge() {
        let path = std::env::temp_dir().join(format!(
            "axiom-bridge-slow-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let registry = crate::installed_registry();
        let seed = axiom_core::World::empty("slow");
        drop(Store::open_with(&path, registry.authoring.clone(), seed.clone()).unwrap());
        let sim = crate::WorldRuntime::new(seed, 1, registry.clone()).unwrap();
        let observation = Arc::new(RwLock::new(sim.observation()));
        let bridge = Bridge::start(path.clone(), 0, observation, registry.authoring).unwrap();
        let address = bridge.address();
        let trickle = |seconds: u64| {
            let mut slow = TcpStream::connect(address).unwrap();
            std::thread::spawn(move || {
                // One byte every 200 ms (each read waits far less than the old 2 s per-read
                // timeout) for `seconds`, never a newline.
                let until = std::time::Instant::now() + Duration::from_secs(seconds);
                let _ = slow.write_all(b"{\"op\":");
                while std::time::Instant::now() < until {
                    if slow.write_all(b" ").is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            })
        };
        let slow = trickle(8);
        std::thread::sleep(Duration::from_millis(100));
        let started = std::time::Instant::now();
        let observed = call(&address.to_string(), &Request::Observe {}).unwrap();
        assert_eq!(observed["ok"], true, "{observed}");
        let waited = started.elapsed();
        assert!(
            waited < REQUEST_READ_LIMIT + Duration::from_millis(1500),
            "a later request waited {waited:?} behind the trickling one"
        );
        slow.join().unwrap();
        // Shutdown while a trickling connection is being read is bounded too.
        let slow = trickle(8);
        std::thread::sleep(Duration::from_millis(300));
        let started = std::time::Instant::now();
        drop(bridge);
        let waited = started.elapsed();
        assert!(
            waited < Duration::from_millis(1500),
            "drop waited {waited:?}"
        );
        slow.join().unwrap();
        let _ = std::fs::remove_file(path);
    }
}
