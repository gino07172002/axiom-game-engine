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
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    thread::JoinHandle,
    time::Duration,
};

pub struct Bridge {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    address: std::net::SocketAddr,
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
        Self::start_with_runtime(db, port, observation, registry, None)
    }
    pub fn start_with_runtime(
        db: PathBuf,
        port: u16,
        observation: Arc<RwLock<Observation>>,
        registry: axiom_core::Registry,
        commands: Option<std::sync::mpsc::SyncSender<RuntimeRequest>>,
    ) -> Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let mut store = Store::open_with(db, registry, axiom_core::World::empty("world"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut history = axiom_core::observe::ObservationHistory::default();
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let result = (|| -> Result<Value> {
                            // Accepted sockets inherit nonblocking mode on Windows.
                            // A JSON line may arrive in several packets; wait up to the budget.
                            stream.set_nonblocking(false)?;
                            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                            let mut line = String::new();
                            BufReader::new((&mut stream).take(1_048_577)).read_line(&mut line)?;
                            let request = parse_request(line.trim())?;
                            if request.is_runtime() {
                                let (reply, result) = std::sync::mpsc::sync_channel(1);
                                commands
                                    .as_ref()
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("live command adapter unavailable")
                                    })?
                                    .try_send(RuntimeRequest {
                                        request,
                                        reply,
                                        deadline: std::time::Instant::now()
                                            + Duration::from_secs(3),
                                    })
                                    .map_err(|_| {
                                        anyhow::anyhow!(
                                            "runtime command queue full or disconnected"
                                        )
                                    })?;
                                result.recv_timeout(Duration::from_secs(3)).map_err(|_| {
                                    anyhow::anyhow!("runtime command response timed out")
                                })
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
        })
    }
    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn call(address: &str, request: &Request) -> Result<Value> {
    let address: std::net::SocketAddr = address.parse()?;
    anyhow::ensure!(
        address.ip().is_loopback(),
        "bridge connections must use a loopback address"
    );
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
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
}
