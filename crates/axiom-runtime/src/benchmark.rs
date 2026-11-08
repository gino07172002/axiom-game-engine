//! Provider-neutral agent benchmark. Acceptance is independent of the agent's completion claim.
use crate::{
    scenario,
    world::{RuntimeRegistry, WorldRuntime},
};
use anyhow::{ensure, Context, Result};
use axiom_core::{
    local::{pointer, Target},
    *,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub id: String,
    pub tasks: Vec<Task>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub instruction: String,
    pub pack: PathBuf,
    pub checks: Vec<Check>,
    #[serde(default)]
    pub plans: BTreeMap<String, Vec<Request>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Check {
    Field {
        target: Target,
        field: String,
        value: Value,
    },
    Scenario {
        file: PathBuf,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub provider: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: Option<u64>,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentReply {
    #[serde(default)]
    pub request: Option<Request>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub usage: Option<Usage>,
}
#[derive(Clone, Debug)]
pub struct AgentOptions {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub timeout: Duration,
}
struct ProcessAgent {
    child: Child,
    input: Option<ChildStdin>,
    receiver: Option<mpsc::Receiver<std::result::Result<AgentReply, String>>>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl ProcessAgent {
    fn start(options: &AgentOptions) -> Result<Self> {
        let mut command = Command::new(&options.executable);
        command
            .args(&options.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().context("start benchmark agent adapter")?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::sync_channel(2);
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            for _ in 0..65 {
                let mut line = String::new();
                let result = (|| -> Result<AgentReply> {
                    (&mut reader).take(1_048_577).read_line(&mut line)?;
                    ensure!(
                        !line.is_empty() && line.len() <= 1_048_576 && line.ends_with('\n'),
                        "agent reply missing, oversized or not newline terminated"
                    );
                    Ok(serde_json::from_str(&line)?)
                })()
                .map_err(|e| format!("{e:#}"));
                let stop = result.is_err();
                if sender.send(result).is_err() || stop {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            receiver: Some(receiver),
            reader: Some(reader),
        })
    }
    fn turn(&mut self, frame: &Value, timeout: Duration) -> Result<AgentReply> {
        let started = Instant::now();
        let mut input = self.input.take().context("agent input unavailable")?;
        let frame = format!("{frame}\n");
        let (sent, written) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = input
                .write_all(frame.as_bytes())
                .and_then(|_| input.flush());
            let _ = sent.send((input, result));
        });
        let (input, result) = written
            .recv_timeout(timeout)
            .context("agent input timeout")?;
        self.input = Some(input);
        result?;
        self.receiver
            .as_ref()
            .unwrap()
            .recv_timeout(timeout.saturating_sub(started.elapsed()))
            .context("agent response timeout/disconnection")?
            .map_err(anyhow::Error::msg)
    }
}
impl Drop for ProcessAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.receiver.take();
        if let Some(reader) = self.reader.take() {
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }
}
struct Harness {
    store: Store,
    runtime: WorldRuntime,
    history: observe::ObservationHistory,
    receipts: retry::SessionReceipts,
    saves: BTreeMap<String, crate::checkpoint::Checkpoint>,
}
impl Harness {
    fn new(world: World, registry: RuntimeRegistry) -> Result<Self> {
        Ok(Self {
            store: Store::memory_with(registry.authoring.clone(), world.clone())?,
            runtime: WorldRuntime::new(world, 1, registry)?,
            history: Default::default(),
            receipts: Default::default(),
            saves: BTreeMap::new(),
        })
    }
    fn dispatch(&mut self, request: Request) -> Result<Value> {
        match request {
            Request::Observe {} => {
                Ok(json!({"ok":true,"runtime":true,"observation":self.runtime.observation()}))
            }
            Request::ObserveDelta {
                cursor,
                fields,
                trace_limit,
                max_bytes,
            } => self.history.observe(
                &json!(self.runtime.observation()),
                cursor.as_deref(),
                &fields,
                trace_limit,
                max_bytes,
            ),
            Request::Command {
                command,
                expected_rev,
            } => {
                ensure!(
                    expected_rev.is_none_or(|r| r == self.runtime.rev),
                    "runtime revision conflict"
                );
                self.runtime.command(&command)
            }
            Request::Save { slot } => {
                ensure!(
                    self.saves.contains_key(&slot) || self.saves.len() < 8,
                    "benchmark checkpoint budget exceeded"
                );
                self.saves.insert(slot.clone(), self.runtime.checkpoint()?);
                Ok(json!({"ok":true,"slot":slot}))
            }
            Request::Load { slot } => {
                self.runtime.restore_checkpoint(
                    self.saves
                        .get(&slot)
                        .context("benchmark checkpoint missing")?,
                )?;
                Ok(json!({"ok":true,"slot":slot}))
            }
            Request::Execute {
                request_id,
                runtime_session,
                request,
            } if request.is_runtime() => {
                let session = runtime_session.context("runtime_session required")?;
                if let Some(v) = self.receipts.lookup(&request_id, &session, &request)? {
                    return Ok(v);
                }
                ensure!(
                    session == self.runtime.runtime_session,
                    "runtime session mismatch"
                );
                let mut value = self.dispatch(*request.clone())?;
                value["request_id"] = json!(request_id);
                value["durability"] = json!("runtime_session");
                self.receipts
                    .insert(request_id, &session, &request, value.clone())?;
                Ok(value)
            }
            request => {
                let mut value = self.store.execute(request)?;
                let (rev, world) = self.store.current()?;
                if rev != self.runtime.requested_rev {
                    if let Err(e) = self.runtime.reconcile(world, rev) {
                        value["runtime_activation_error"] = error_response(&e);
                    }
                }
                Ok(value)
            }
        }
    }
    fn acceptance(&self, task: &Task, root: &Path) -> Result<Vec<Value>> {
        let (rev, world) = self.store.current()?;
        let mut out = vec![];
        for check in &task.checks {
            match check {
                Check::Field {
                    target,
                    field,
                    value,
                } => {
                    let object = target.get(&json!(world))?;
                    let actual = object
                        .pointer(&pointer(field))
                        .context("acceptance field missing")?;
                    ensure!(
                        actual == value,
                        "acceptance failed at {}{}: expected {value}, actual {actual}",
                        target.path(),
                        pointer(field)
                    );
                    out.push(json!({"kind":"field","target":target,"field":field,"passed":true}));
                }
                Check::Scenario { file } => {
                    let raw = std::fs::read_to_string(root.join(file))?;
                    ensure!(raw.len() <= 1_048_576, "acceptance scenario exceeds budget");
                    let result = scenario::run(
                        world.clone(),
                        rev,
                        self.runtime.registry_clone(),
                        serde_json::from_str(&raw)?,
                    )?;
                    ensure!(result["ok"] == true, "acceptance scenario failed");
                    out.push(json!({"kind":"scenario","file":file,"passed":true}));
                }
            }
        }
        Ok(out)
    }
}
#[derive(Default, Serialize)]
struct Cost {
    tool_calls: u64,
    failed_requests: u64,
    request_bytes: u64,
    response_bytes: u64,
    agent_turns: u64,
    adapter_input_bytes: u64,
    adapter_output_bytes: u64,
}
fn exchange(
    h: &mut Harness,
    request: Request,
    cost: &mut Cost,
    transcript: &mut Vec<Value>,
) -> Value {
    let bytes = serde_json::to_vec(&request).unwrap().len();
    let start = Instant::now();
    let response = h
        .dispatch(request.clone())
        .unwrap_or_else(|e| error_response(&e));
    cost.tool_calls += 1;
    cost.request_bytes += bytes as u64;
    cost.response_bytes += serde_json::to_vec(&response).unwrap().len() as u64;
    if response.get("ok").and_then(Value::as_bool) == Some(false) {
        cost.failed_requests += 1;
    }
    transcript.push(json!({"request":request,"response":response,"wall_ms":start.elapsed().as_secs_f64()*1000.0}));
    response
}
pub fn run(
    suite: Suite,
    root: &Path,
    registry: RuntimeRegistry,
    agent: Option<AgentOptions>,
) -> Result<Value> {
    ensure!(
        !suite.tasks.is_empty() && suite.tasks.len() <= 32,
        "benchmark task budget must be 1..32"
    );
    let mut results = vec![];
    for task in &suite.tasks {
        ensure!(
            task.instruction.len() <= 8192 && !task.checks.is_empty() && task.checks.len() <= 32,
            "benchmark definition exceeds budget"
        );
        let world = ContentPackage::load(root.join(&task.pack), &registry.authoring)?.world;
        let modes = if agent.is_some() {
            vec!["agent".to_owned()]
        } else {
            task.plans.keys().cloned().collect::<Vec<_>>()
        };
        ensure!(
            !modes.is_empty(),
            "task has no replay plans; provide --agent-executable"
        );
        for mode in modes {
            let mut h = Harness::new(world.clone(), registry.clone())?;
            let mut cost = Cost::default();
            let mut transcript = vec![];
            let mut usages = vec![];
            let started = Instant::now();
            let mut done = agent.is_none();
            let mut agent_error = None;
            if let Some(options) = &agent {
                let execution = (|| -> Result<()> {
                    let mut process = ProcessAgent::start(options)?;
                    let mut feedback = json!({"rev":1,"capabilities":h.store.registry.capabilities,"hint":"Use schema/discover for API details. Return one typed request per turn, or done:true. Acceptance checks are independent of your claim."});
                    for turn in 0..64 {
                        let frame = json!({"protocol":"axiom-benchmark/1","task_id":task.id,"instruction":task.instruction,"turn":turn,"feedback":feedback});
                        cost.adapter_input_bytes += serde_json::to_vec(&frame)?.len() as u64;
                        let reply = process.turn(&frame, options.timeout)?;
                        cost.agent_turns += 1;
                        cost.adapter_output_bytes += serde_json::to_vec(&reply)?.len() as u64;
                        ensure!(
                            reply.done != reply.request.is_some(),
                            "agent reply must contain either request or done:true"
                        );
                        if let Some(u) = &reply.usage {
                            ensure!(
                                u.provider.len() <= 128
                                    && u.model.len() <= 128
                                    && u.input_tokens <= 1_000_000_000
                                    && u.output_tokens <= 1_000_000_000
                                    && u.cached_input_tokens.is_none_or(|n| n <= u.input_tokens),
                                "invalid agent usage metadata"
                            );
                        }
                        usages.push(reply.usage.clone());
                        if reply.done {
                            done = true;
                            break;
                        }
                        feedback =
                            exchange(&mut h, reply.request.unwrap(), &mut cost, &mut transcript);
                    }
                    ensure!(done, "agent turn budget exhausted");
                    Ok(())
                })();
                if let Err(e) = execution {
                    agent_error = Some(format!("{e:#}"));
                }
            } else {
                let plan = &task.plans[&mode];
                ensure!(plan.len() <= 64, "replay plan exceeds 64 requests");
                for request in plan {
                    exchange(&mut h, request.clone(), &mut cost, &mut transcript);
                }
            }
            let acceptance = h.acceptance(task, root);
            let success = done && agent_error.is_none() && acceptance.is_ok();
            let checks = acceptance.as_ref().ok().cloned();
            let usage_interrupted = agent_error.is_some();
            let failure = agent_error.or_else(|| acceptance.err().map(|e| format!("{e:#}")));
            let complete = agent.is_some()
                && !usage_interrupted
                && !usages.is_empty()
                && usages.iter().all(|u| {
                    u.as_ref().is_some_and(|u| {
                        u.source == "provider_reported"
                            && !u.provider.is_empty()
                            && !u.model.is_empty()
                    })
                });
            let tokens = if complete {
                Some(
                    json!({"input_tokens":usages.iter().flatten().map(|u|u.input_tokens).sum::<u64>(),"output_tokens":usages.iter().flatten().map(|u|u.output_tokens).sum::<u64>(),"source":"adapter_reported_provider_usage"}),
                )
            } else {
                None
            };
            results.push(json!({"task":task.id,"mode":mode,"success":success,"checks":checks,"failure":failure,"cost":cost,"model_usage":tokens,"usage_complete":complete,"usage_records":usages,"wall_ms":started.elapsed().as_secs_f64()*1000.0,"transcript":transcript}));
        }
    }
    let mut groups = serde_json::Map::new();
    for mode in results
        .iter()
        .map(|v| v["mode"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let rows = results
            .iter()
            .filter(|v| v["mode"] == mode)
            .collect::<Vec<_>>();
        let successes = rows.iter().filter(|v| v["success"] == true).count();
        let sum = |key: &str| {
            rows.iter()
                .map(|v| v["cost"][key].as_u64().unwrap_or(0))
                .sum::<u64>()
        };
        let tokens = if rows.iter().all(|v| v["usage_complete"] == true) {
            Some(
                rows.iter()
                    .map(|v| {
                        v["model_usage"]["input_tokens"].as_u64().unwrap()
                            + v["model_usage"]["output_tokens"].as_u64().unwrap()
                    })
                    .sum::<u64>(),
            )
        } else {
            None
        };
        groups.insert(mode.into(),json!({"tasks":rows.len(),"successes":successes,"success_rate":successes as f64/rows.len() as f64,"tool_calls":sum("tool_calls"),"failed_requests":sum("failed_requests"),"request_bytes":sum("request_bytes"),"response_bytes":sum("response_bytes"),"total_model_tokens":tokens,"tokens_per_successful_task":tokens.filter(|_|successes>0).map(|n|n as f64/successes as f64)}));
    }
    Ok(
        json!({"ok":results.iter().all(|r|r["success"]==true),"benchmark_version":1,"suite":suite.id,"engine":env!("CARGO_PKG_VERSION"),"measurement":if agent.is_some(){"agent_adapter"}else{"protocol_replay"},"token_note":"Bytes are transport measurements, never token estimates. Model usage is optional adapter-reported provider metadata; missing or non-provider records keep totals null. All failed attempts count toward cost.","summary":groups,"results":results}),
    )
}
