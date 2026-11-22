use anyhow::{bail, ensure, Context, Result};
use axiom_core::{
    error_response, parse_request, ContentPackage, CoreEdit, Edit, Request, Scope, Store, World,
};
use clap::{Parser, Subcommand};
use std::{
    io::{BufRead, Read},
    path::PathBuf,
};
#[derive(Parser)]
#[command(
    name = "axiom",
    version,
    about = "AI-native World IR v2 engine and content packages"
)]
struct Cli {
    /// Deduplicate a successful mutation. Runtime commands also require --runtime-session.
    #[arg(long, global = true)]
    request_id: Option<String>,
    #[arg(long, global = true)]
    runtime_session: Option<String>,
    /// World database. Defaults to data/editor.sqlite for editor, data/world.sqlite otherwise.
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[arg(long, global = true)]
    connect: Option<String>,
    #[arg(long, global = true)]
    pretty: bool,
    /// Content package directory. Switches play worlds; initializes new headless databases.
    #[arg(long, global = true)]
    pack: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Inspect a skeletal asset without returning mesh/texture payloads.
    AnimationInspect {
        model: PathBuf,
        #[arg(long)]
        profile_output: Option<PathBuf>,
        #[arg(long)]
        bone: Vec<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 32)]
        limit: usize,
    },
    /// Create a local animation lab content package, referencing artwork in place.
    AnimationDemo {
        model: PathBuf,
        #[arg(long)]
        source: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Bake a source clip to another humanoid skeleton using calibrated profiles.
    AnimationRetarget {
        source: PathBuf,
        target: PathBuf,
        #[arg(long)]
        clip: String,
        #[arg(long)]
        source_profile: Option<PathBuf>,
        #[arg(long)]
        target_profile: Option<PathBuf>,
        #[arg(long, default_value_t = 30)]
        fps: u32,
        #[arg(long)]
        output: PathBuf,
    },
    RtsStress {
        #[arg(long, default_value_t = 128)]
        units: usize,
        #[arg(long, default_value_t = 2400)]
        ticks: u32,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Submit any typed JSON protocol request from a file (or stdin).
    Rpc {
        file: Option<PathBuf>,
    },
    Select {
        file: PathBuf,
    },
    /// Call a registered runtime action, e.g. `call fsm.inspect '{"id":"enemy/3"}'`.
    /// Live with --connect; otherwise headless after simulating --ticks fixed steps.
    Call {
        /// capability.action
        action: String,
        /// JSON object arguments (default {}).
        args: Option<String>,
        #[arg(long, default_value_t = 0)]
        ticks: u32,
    },
    /// Run a terse ACL script (`discover acl` lists the grammar). Prints compact lines.
    Acl {
        /// Script file; omit to read stdin.
        file: Option<PathBuf>,
        /// Inline script; `;` separates statements.
        #[arg(short = 'e', long = "eval")]
        eval: Option<String>,
        /// Validate and show the diff without writing a revision.
        #[arg(long)]
        preview: bool,
        /// Queue the edits for human review (editor Proposals panel) instead of applying.
        #[arg(long)]
        propose: bool,
        #[arg(long)]
        expected_rev: Option<i64>,
    },
    /// Replace deprecated authoring in one revision (= `acl -e "migrate camera_blocker"`):
    /// tag camera_blocker -> camera.blocker component. `verify` lists what is deprecated.
    Migrate {
        /// Show the diff without writing a revision.
        #[arg(long)]
        preview: bool,
    },
    /// Build a portable game folder from --pack: pack files, player executable, game.json.
    Package {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        exe_name: Option<String>,
        /// Extra exclusions: `dir/`, `*.ext` or a pack-relative path (repeatable).
        #[arg(long)]
        exclude: Vec<String>,
        /// Replace an existing packaged game folder.
        #[arg(long)]
        force: bool,
        /// Refuse to publish unless the shipped pack files hash to this report `pack_digest`.
        #[arg(long)]
        expected_pack_digest: Option<String>,
    },
    /// Drive a running editor (--connect): select/frame objects, switch dock, capture a PNG.
    View {
        #[arg(long)]
        select: Vec<String>,
        #[arg(long)]
        focus: bool,
        #[arg(long)]
        dock: Option<String>,
        /// Writes artifacts/captures/<NAME>.png (relative to the editor's working directory).
        #[arg(long)]
        capture: Option<String>,
        /// Start (true) or stop (false) the Play-in-editor preview.
        #[arg(long)]
        play: Option<bool>,
        /// Open an asset inspector (material:ID, prefab:ID, resource:ID), a component of the
        /// selection (component:ID) or a create command's options dialog (options:COMMAND).
        #[arg(long)]
        open: Option<String>,
        /// Run an editor command by ID (same as the menu bar / palette), e.g. edit.duplicate.
        #[arg(long)]
        command: Option<String>,
        /// Argument of a command that takes one, e.g. --command world.capability_add --arg physics.
        #[arg(long, allow_hyphen_values = true)]
        arg: Option<String>,
    },
    /// List queued edit proposals (newest first).
    Proposals {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Approve or reject a queued proposal.
    Resolve {
        id: i64,
        #[arg(long, conflicts_with = "reject")]
        approve: bool,
        #[arg(long)]
        reject: bool,
        /// Re-validate against the current revision instead of the proposal's base.
        #[arg(long)]
        rebase: bool,
        /// Approve by a field-wise merge onto the current revision (see `acl -e "conflicts ID"`).
        #[arg(long, conflicts_with_all = ["rebase", "reject"], requires = "expected_rev")]
        merge: bool,
        /// With --merge: a conflicting field path that keeps the current value (repeatable).
        #[arg(long, requires = "merge")]
        keep: Vec<String>,
        /// Refuse unless this is the current revision (the one `conflicts` listed against).
        #[arg(long)]
        expected_rev: Option<i64>,
    },
    Discover {
        topic: Option<String>,
        #[arg(long)]
        if_hash: Option<String>,
    },
    ObserveDelta {
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        field: Vec<String>,
        #[arg(long, default_value_t = 0)]
        trace_limit: usize,
        #[arg(long)]
        max_bytes: Option<usize>,
    },
    Diff {
        from_rev: i64,
        to_rev: i64,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long)]
        limit: Option<usize>,
    },
    Benchmark {
        suite: PathBuf,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        agent_executable: Option<PathBuf>,
        #[arg(long)]
        agent_arg: Vec<String>,
        #[arg(long, default_value_t = 30)]
        timeout_seconds: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Play {
        #[arg(long)]
        load_slot: Option<String>,
        #[arg(long, default_value_t = 4242)]
        port: u16,
        #[arg(long)]
        start: bool,
        #[arg(long)]
        smoke_frames: Option<u32>,
        #[arg(long)]
        screenshot: Option<PathBuf>,
    },
    /// Open the native scene editor with the same authoring protocol used by AI agents.
    Editor {
        #[arg(long, default_value_t = 4252)]
        port: u16,
        #[arg(long)]
        select: Option<String>,
        #[arg(long)]
        smoke_frames: Option<u32>,
        #[arg(long)]
        screenshot: Option<PathBuf>,
    },
    Schema {
        topic: Option<String>,
    },
    Query {
        #[arg(default_value = "summary")]
        scope: String,
        #[arg(long)]
        limit: Option<usize>,
    },
    Apply {
        file: Option<PathBuf>,
    },
    Preview {
        file: Option<PathBuf>,
    },
    Verify {
        #[arg(long, default_value_t = 0)]
        ticks: u32,
        /// Time read/decode, compile and runtime build (median of N runs) and report RSS.
        #[arg(long)]
        measure: Option<usize>,
    },
    Observe,
    /// Typed gameplay command JSON, dispatched to a live engine with --connect.
    #[command(name = "command")]
    Dispatch {
        file: PathBuf,
        #[arg(long)]
        expected_rev: Option<i64>,
    },
    Save {
        slot: String,
    },
    Load {
        slot: String,
    },
    Undo {
        rev: i64,
        #[arg(long)]
        expected_rev: Option<i64>,
    },
    History {
        #[arg(long)]
        limit: Option<usize>,
    },
    Import {
        file: PathBuf,
        /// Refuse the import unless this is still the current revision (someone else may have
        /// committed since the world was exported).
        #[arg(long)]
        expected_rev: Option<i64>,
    },
    Export {
        file: PathBuf,
    },
    Scenario {
        file: PathBuf,
        /// Also write scenario `save` checkpoints as slot files into this directory. By default
        /// checkpoints stay in memory so a scenario never touches the game's real saves.
        #[arg(long)]
        save_dir: Option<PathBuf>,
    },
    NarrativeCheck {
        #[arg(long, default_value_t = 4096)]
        budget: usize,
    },
    Serve,
}
/// Runs the full `axiom` command line with the given runtime registry. Game crates that add
/// native plugins ship their own executable with
/// `fn main() { axiom::main_with(axiom_runtime::installed_registry().with(MyPlugin)) }`.
pub fn main_with(registry: axiom_runtime::world::RuntimeRegistry) {
    if let Err(error) = execute(registry) {
        eprintln!("{}", error_response(&error));
        std::process::exit(1);
    }
}
fn initial(pack: Option<&std::path::Path>, registry: &axiom_core::Registry) -> Result<World> {
    if let Some(path) = pack {
        return Ok(ContentPackage::load(path, registry)?.world);
    }
    let default = std::path::Path::new("content/fps");
    if default.join("world.json").exists() {
        Ok(ContentPackage::load(default, registry)?.world)
    } else {
        axiom_fps::demo_world()
    }
}
fn execute(registry: axiom_runtime::world::RuntimeRegistry) -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Play {
        load_slot: None,
        port: 4242,
        start: false,
        smoke_frames: None,
        screenshot: None,
    });
    match &command {
        Command::AnimationInspect {
            model,
            profile_output,
            bone,
            offset,
            limit,
        } => {
            ensure!(
                (1..=128).contains(limit) && bone.len() <= 128,
                "bone query limit must be 1..128"
            );
            let asset = axiom_animation::import::load(model, &Default::default())?;
            let (mut profile, mut mapping) = axiom_animation::humanoid::auto_map(&asset.skeleton);
            if let Some(p) = &asset.humanoid {
                profile = p.clone();
                mapping = serde_json::json!({"mapping":profile.mapping,"source":"embedded Unity humanoid profile"});
            }
            if let Some(path) = profile_output {
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, serde_json::to_vec_pretty(&profile)?)?;
            }
            let rows = asset
                .skeleton
                .source
                .bones
                .iter()
                .enumerate()
                .filter(|(_, b)| bone.is_empty() || bone.contains(&b.id))
                .collect::<Vec<_>>();
            let items=rows.iter().skip(*offset).take(*limit).map(|(i,b)|serde_json::json!({"id":b.id,"parent":b.parent.map(|p|&asset.skeleton.source.bones[p].id),"rest":b.rest,"position":asset.skeleton.rest_global[*i].w_axis.truncate()})).collect::<Vec<_>>();
            print(
                &serde_json::json!({"ok":true,"asset":asset.summary(),"humanoid":mapping,"bones":items,"total":rows.len(),"next_offset":if offset+limit<rows.len(){Some(offset+limit)}else{None}}),
                cli.pretty,
            )?;
            return Ok(());
        }
        Command::AnimationDemo {
            model,
            source,
            output,
        } => {
            print(
                &axiom_runtime::animation_demo::create(model, source.as_deref(), output)?,
                cli.pretty,
            )?;
            return Ok(());
        }
        Command::AnimationRetarget {
            source,
            target,
            clip,
            source_profile,
            target_profile,
            fps,
            output,
        } => {
            print(
                &axiom_runtime::animation_demo::retarget(
                    source,
                    target,
                    clip,
                    source_profile.as_deref(),
                    target_profile.as_deref(),
                    *fps,
                    output,
                )?,
                cli.pretty,
            )?;
            return Ok(());
        }
        _ => {}
    }
    if let Command::RtsStress {
        units,
        ticks,
        output,
    } = command
    {
        ensure!(
            cli.connect.is_none() && cli.request_id.is_none(),
            "stress uses an isolated runtime"
        );
        let result = axiom_runtime::rts_stress::run(units, ticks, registry)?;
        if let Some(path) = output {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, serde_json::to_vec_pretty(&result)?)?;
        }
        print(&result, cli.pretty)?;
        return Ok(());
    }
    if let Command::Benchmark {
        suite,
        root,
        agent_executable,
        agent_arg,
        timeout_seconds,
        output,
    } = command
    {
        ensure!(
            cli.connect.is_none() && cli.request_id.is_none(),
            "benchmark uses isolated databases, no --connect or request ID"
        );
        ensure!(
            (1..=60).contains(&timeout_seconds),
            "agent timeout must be 1..60 seconds"
        );
        let raw = axiom_core::read_bounded_string(&suite, 1_048_576, "benchmark suite")?;
        let agent = agent_executable.map(|executable| axiom_runtime::benchmark::AgentOptions {
            executable,
            args: agent_arg,
            timeout: std::time::Duration::from_secs(timeout_seconds),
        });
        let result =
            axiom_runtime::benchmark::run(serde_json::from_str(&raw)?, &root, registry, agent)?;
        if let Some(path) = output {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, serde_json::to_vec_pretty(&result)?)?;
        }
        let mut summary = result.clone();
        summary.as_object_mut().unwrap().remove("results");
        print(&summary, cli.pretty)?;
        return Ok(());
    }
    if let Command::Play {
        load_slot,
        port,
        start,
        smoke_frames,
        screenshot,
    } = command
    {
        ensure!(cli.connect.is_none(), "play cannot use --connect");
        ensure!(smoke_frames != Some(0), "smoke_frames must be positive");
        ensure!(
            screenshot.is_none() || smoke_frames.is_some(),
            "screenshot requires smoke_frames"
        );
        let world = initial(cli.pack.as_deref(), &registry.authoring)?;
        return axiom_runtime::run(
            axiom_runtime::RunOptions {
                mode: axiom_runtime::RunMode::Play { player: None },
                load_slot,
                db: cli.db.unwrap_or_else(|| "data/world.sqlite".into()),
                replace_world: cli.pack.is_some(),
                port,
                start_play: start,
                smoke_frames,
                screenshot,
            },
            registry,
            world,
        );
    }
    if let Command::Package {
        out,
        title,
        exe_name,
        exclude,
        force,
        expected_pack_digest,
    } = command
    {
        ensure!(cli.connect.is_none(), "package cannot use --connect");
        let pack = cli.pack.context("package requires --pack DIR")?;
        if axiom_runtime::packaging::inside_repo(&out) {
            eprintln!("warning: output is inside a git work tree; do not commit packaged games");
        }
        let report = axiom_runtime::packaging::package(
            &registry.authoring,
            &axiom_runtime::packaging::PackageOptions {
                pack,
                out,
                title,
                exe_name,
                excludes: exclude,
                force,
                player_exe: axiom_runtime::packaging::default_player_exe()?,
                expected_pack_digest,
            },
            &Default::default(),
        )?;
        return print(&report, cli.pretty);
    }
    if let Command::Editor {
        port,
        select,
        smoke_frames,
        screenshot,
    } = command
    {
        ensure!(cli.connect.is_none(), "editor cannot use --connect");
        ensure!(smoke_frames != Some(0), "smoke_frames must be positive");
        ensure!(
            screenshot.is_none() || smoke_frames.is_some(),
            "screenshot requires smoke_frames"
        );
        let world = if let Some(path) = cli.pack.as_deref() {
            initial(Some(path), &registry.authoring)?
        } else {
            registry
                .authoring
                .decode(serde_json::from_str(include_str!(
                    "../../../content/editor-lab/world.json"
                ))?)?
        };
        return axiom_runtime::run(
            axiom_runtime::RunOptions {
                mode: axiom_runtime::RunMode::Editor {
                    selected: select,
                    pack: cli.pack.clone(),
                },
                load_slot: None,
                db: cli.db.unwrap_or_else(|| "data/editor.sqlite".into()),
                replace_world: cli.pack.is_some(),
                port,
                start_play: false,
                smoke_frames,
                screenshot,
            },
            registry,
            world,
        );
    }
    let db = cli.db.unwrap_or_else(|| "data/world.sqlite".into());
    let mut store = if cli.connect.is_none() {
        // The initial world (default: the content/fps package) is only loaded for a new
        // database, so `schema`/`query` on an existing one never depend on that package.
        let mut store = Store::open_lazy(&db, registry.authoring.clone(), || {
            initial(cli.pack.as_deref(), &registry.authoring)
        })?;
        // The CLI is how agents author (people use the editor).
        store.set_author(Some(axiom_core::Author::Agent));
        Some(store)
    } else {
        None
    };
    let mut dispatch = |request: Request| -> Result<serde_json::Value> {
        let request = if let Some(id) = &cli.request_id {
            ensure!(
                !matches!(request, Request::Execute { .. }),
                "request already has an execute envelope"
            );
            Request::Execute {
                request_id: id.clone(),
                runtime_session: cli.runtime_session.clone(),
                request: Box::new(request),
            }
        } else {
            request
        };
        if let Some(address) = &cli.connect {
            axiom_runtime::bridge::call(address, &request)
        } else {
            store.as_mut().unwrap().execute(request)
        }
    };
    let request = match command {
        Command::RtsStress { .. } => unreachable!(),
        Command::Benchmark { .. } => unreachable!(),
        Command::Rpc { file } => {
            let raw = if let Some(path) = file {
                axiom_core::read_bounded_string(&path, MAX_REQUEST_BYTES, "request")?
            } else {
                let mut raw = String::new();
                std::io::stdin().take(1_048_577).read_to_string(&mut raw)?;
                raw
            };
            parse_request(&raw)?
        }
        Command::Select { file } => {
            let raw = axiom_core::read_bounded_string(&file, MAX_REQUEST_BYTES, "selection")?;
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            if value.get("op").is_some() {
                let request = parse_request(&raw)?;
                ensure!(
                    matches!(request, Request::Select { .. }),
                    "select expects a select request"
                );
                request
            } else {
                Request::Select {
                    query: serde_json::from_value(value)?,
                }
            }
        }
        Command::Call {
            action,
            args,
            ticks,
        } => {
            let (capability, name) = action
                .split_once('.')
                .context("action must be capability.action, e.g. fsm.inspect")?;
            let args: serde_json::Value = match args {
                Some(raw) => serde_json::from_str(&raw).context("args must be a JSON object")?,
                None => serde_json::json!({}),
            };
            let command = axiom_core::gameplay::RuntimeCommand::Call {
                capability: capability.into(),
                action: name.into(),
                args,
            };
            if cli.connect.is_some() {
                ensure!(ticks == 0, "--ticks only applies to headless calls");
                Request::Command {
                    command,
                    expected_rev: None,
                }
            } else {
                ensure!(ticks <= 1_000_000, "tick budget exceeds 1000000");
                let (rev, w) = store.as_ref().unwrap().current()?;
                let mut runtime = axiom_runtime::WorldRuntime::new(w, rev, registry)?;
                for _ in 0..ticks {
                    runtime.step(&axiom_runtime::ActionMap::default());
                }
                let result = runtime.command(&command)?;
                print(&axiom_core::local::display_value(&result), cli.pretty)?;
                return Ok(());
            }
        }
        Command::View {
            select,
            focus,
            dock,
            capture,
            play,
            open,
            command,
            arg,
        } => {
            ensure!(
                cli.connect.is_some(),
                "view drives a running editor; pass --connect"
            );
            Request::EditorView {
                select,
                focus,
                dock,
                capture,
                play,
                open,
                command,
                arg,
            }
        }
        Command::Proposals { status, limit } => Request::Proposals { status, limit },
        Command::Resolve {
            id,
            approve,
            reject,
            rebase,
            merge,
            keep,
            expected_rev,
        } => {
            ensure!(
                approve != reject,
                "pass exactly one of --approve or --reject"
            );
            Request::ResolveProposal {
                id,
                approve,
                rebase,
                keep: merge.then_some(keep),
                expected_rev,
            }
        }
        Command::Acl {
            file,
            eval,
            preview,
            propose,
            expected_rev,
        } => {
            let text = match (eval, file) {
                (Some(_), Some(_)) => anyhow::bail!("use either -e or a script file"),
                (Some(inline), None) => inline,
                (None, Some(path)) => axiom_core::read_bounded_string(&path, 65_536, "ACL script")?,
                (None, None) => {
                    let mut raw = String::new();
                    std::io::stdin().take(65_537).read_to_string(&mut raw)?;
                    raw
                }
            };
            Request::Acl {
                text,
                preview,
                expected_rev,
                propose,
            }
        }
        Command::Migrate { preview } => Request::Acl {
            text: "migrate camera_blocker".into(),
            preview,
            expected_rev: None,
            propose: false,
        },
        Command::Discover { topic, if_hash } => Request::Discover { topic, if_hash },
        Command::ObserveDelta {
            cursor,
            field,
            trace_limit,
            max_bytes,
        } => Request::ObserveDelta {
            cursor,
            fields: field,
            trace_limit,
            max_bytes,
        },
        Command::Diff {
            from_rev,
            to_rev,
            offset,
            limit,
        } => Request::Diff {
            from_rev,
            to_rev,
            offset,
            limit,
        },
        Command::NarrativeCheck { budget } => {
            ensure!(
                cli.connect.is_none(),
                "narrative-check requires a local content database"
            );
            let (_, w) = store.as_ref().unwrap().current()?;
            print(&axiom_runtime::narrative::analyze(&w, budget)?, cli.pretty)?;
            return Ok(());
        }
        Command::Dispatch { file, expected_rev } => {
            ensure!(cli.connect.is_some(), "command requires --connect");
            let raw = axiom_core::read_bounded_string(&file, MAX_REQUEST_BYTES, "command")?;
            Request::Command {
                command: serde_json::from_str(&raw)?,
                expected_rev,
            }
        }
        Command::Save { slot } => Request::Save { slot },
        Command::Load { slot } => Request::Load { slot },
        Command::Schema { topic } => Request::Schema { topic },
        Command::Query { scope, limit } => Request::Query {
            scope: serde_json::from_value(serde_json::Value::String(scope))
                .context("invalid query scope")?,
            limit,
        },
        Command::Apply { ref file } | Command::Preview { ref file } => {
            let preview = matches!(command, Command::Preview { .. });
            let raw = if let Some(p) = file {
                axiom_core::read_bounded_string(p, MAX_REQUEST_BYTES, "request")?
            } else {
                let mut text = String::new();
                std::io::stdin().take(1_048_577).read_to_string(&mut text)?;
                text
            };
            match parse_request(&raw)? {
                Request::Apply {
                    expected_rev,
                    edits,
                }
                | Request::Preview {
                    expected_rev,
                    edits,
                } => {
                    if preview {
                        Request::Preview {
                            expected_rev,
                            edits,
                        }
                    } else {
                        Request::Apply {
                            expected_rev,
                            edits,
                        }
                    }
                }
                _ => anyhow::bail!("apply/preview expects an editing request"),
            }
        }
        Command::Verify { ticks, measure } => {
            ensure!(ticks <= 1_000_000, "tick budget exceeds 1000000");
            // Verification and simulation share one snapshot, so the outer `rev` is the
            // revision that was both validated and simulated.
            let (mut result, snapshot) =
                if ticks > 0 && cli.connect.is_none() && cli.request_id.is_none() {
                    let local = store.as_ref().unwrap();
                    let (rev, w) = local.current()?;
                    (local.verify_world(rev, &w)?, Some((rev, w)))
                } else {
                    (dispatch(Request::Verify {})?, None)
                };
            if let Some(runs) = measure {
                ensure!(
                    cli.connect.is_none(),
                    "load measurement requires a local database"
                );
                let load =
                    axiom_runtime::measure::load_profile(store.as_ref().unwrap(), &registry, runs)?;
                // The profile re-reads the database: a commit in between would make it about
                // another revision than the one verified.
                ensure!(
                    load["rev"] == result["rev"],
                    "revision changed from {} to {} during verify --measure; run it again",
                    result["rev"],
                    load["rev"]
                );
                result["load"] = load;
            }
            if ticks > 0 {
                ensure!(
                    cli.connect.is_none(),
                    "headless simulation requires a local database"
                );
                let (rev, w) = match snapshot {
                    Some(snapshot) => snapshot,
                    None => store.as_ref().unwrap().current()?,
                };
                let mut runtime = axiom_runtime::WorldRuntime::new(w, rev, registry)?;
                let start = std::time::Instant::now();
                for _ in 0..ticks {
                    runtime.step(&axiom_runtime::ActionMap::default());
                    runtime.assert_valid()?;
                }
                result["simulation"] = serde_json::json!({"requested_ticks":ticks,"wall_ms":start.elapsed().as_secs_f64()*1000.0,"observation":runtime.observation()});
            }
            print(&result, cli.pretty)?;
            return Ok(());
        }
        Command::Scenario { file, save_dir } => {
            ensure!(
                cli.connect.is_none(),
                "scenario uses a local isolated runtime"
            );
            let raw = axiom_core::read_bounded_string(&file, 1_048_576, "scenario")?;
            let scenario = serde_json::from_str(&raw)?;
            let (rev, w) = store.as_ref().unwrap().current()?;
            print(
                &axiom_runtime::scenario::run_with_saves(
                    w,
                    rev,
                    registry,
                    scenario,
                    save_dir.as_deref(),
                )?,
                cli.pretty,
            )?;
            return Ok(());
        }
        Command::Observe => Request::Observe {},
        Command::Undo { rev, expected_rev } => Request::Undo { rev, expected_rev },
        Command::History { limit } => Request::History { limit },
        Command::Import { file, expected_rev } => Request::Apply {
            expected_rev,
            edits: vec![Edit::Core(CoreEdit::ReplaceWorld {
                world: serde_json::from_str(&axiom_core::read_bounded_string(
                    &file,
                    MAX_IMPORT_BYTES,
                    "import world",
                )?)?,
            })],
        },
        Command::Export { file } => {
            let result = dispatch(Request::Query {
                scope: Scope::World,
                limit: None,
            })?;
            // A remote error (or any response without a world) must not overwrite the file.
            if result["ok"] == false {
                print(&result, cli.pretty)?;
                bail!("export failed; {} was not written", file.display());
            }
            let world: World = serde_json::from_value(result["world"].clone())
                .context("export response has no valid world; nothing was written")?;
            std::fs::write(&file, serde_json::to_string_pretty(&world)?)?;
            print(
                &serde_json::json!({"ok":true,"file":file,"rev":result["rev"]}),
                cli.pretty,
            )?;
            return Ok(());
        }
        Command::Serve => {
            let mut input = std::io::stdin().lock();
            let mut line = Vec::new();
            while let Some(read) = read_bounded_line(&mut input, &mut line, MAX_REQUEST_LINE)? {
                let response = match read {
                    Line::TooLong => Err(anyhow::anyhow!(
                        "request line exceeds {MAX_REQUEST_LINE} bytes; it was discarded"
                    )),
                    Line::Complete => match std::str::from_utf8(&line) {
                        Ok(text) if text.trim().is_empty() => continue,
                        Ok(text) => parse_request(text).and_then(&mut dispatch),
                        Err(_) => Err(anyhow::anyhow!("request line is not valid UTF-8")),
                    },
                };
                let result = match response {
                    Ok(v) => v,
                    Err(e) => error_response(&e),
                };
                println!("{result}");
            }
            return Ok(());
        }
        Command::Play { .. }
        | Command::Editor { .. }
        | Command::Package { .. }
        | Command::AnimationInspect { .. }
        | Command::AnimationDemo { .. }
        | Command::AnimationRetarget { .. } => unreachable!(),
    };
    print(&dispatch(request)?, cli.pretty)
}
/// Protocol request limit (`parse_request` refuses more); request files are read bounded by it.
const MAX_REQUEST_BYTES: usize = 1_048_576;
/// `import FILE` reads at most this much world JSON (the store's total receipt budget).
const MAX_IMPORT_BYTES: usize = 67_108_864;
/// Longest `serve` request line read into memory (the protocol limit is 1 MiB; this leaves
/// room for `\r\n`). Longer lines are skipped without being buffered.
const MAX_REQUEST_LINE: usize = MAX_REQUEST_BYTES + 2;
enum Line {
    Complete,
    TooLong,
}
/// Reads one `\n`-terminated line into `line` (without the terminator), keeping at most `max`
/// bytes in memory: the rest of an over-long line is read and dropped in bounded chunks.
/// `None` at end of input.
fn read_bounded_line(
    input: &mut impl BufRead,
    line: &mut Vec<u8>,
    max: usize,
) -> Result<Option<Line>> {
    line.clear();
    let read = input
        .by_ref()
        .take(max as u64 + 1)
        .read_until(b'\n', line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        return Ok(Some(Line::Complete));
    }
    if line.len() <= max {
        // End of input without a newline.
        return Ok(Some(Line::Complete));
    }
    line.clear();
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        if let Some(end) = chunk.iter().position(|b| *b == b'\n') {
            input.consume(end + 1);
            break;
        }
        let n = chunk.len();
        input.consume(n);
    }
    Ok(Some(Line::TooLong))
}
fn print(value: &serde_json::Value, pretty: bool) -> Result<()> {
    let text = if pretty {
        serde_json::to_string_pretty(value)?
    } else {
        serde_json::to_string(value)?
    };
    if value.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        eprintln!("{text}");
        std::process::exit(1);
    }
    // Lines-format results are already compact text meant for agents; don't re-wrap in JSON.
    if !pretty && value.get("format").and_then(|v| v.as_str()) == Some("lines") {
        if let Some(lines) = value.get("text").and_then(|v| v.as_str()) {
            println!("{lines}");
            return Ok(());
        }
    }
    println!("{text}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_lines_are_bounded_and_an_overlong_line_does_not_stop_the_stream() {
        let mut input = std::io::Cursor::new(b"ok\r\n0123456789abcdef\nnext\nlast".to_vec());
        let mut line = Vec::new();
        let mut seen = vec![];
        while let Some(read) = read_bounded_line(&mut input, &mut line, 8).unwrap() {
            seen.push(match read {
                Line::Complete => String::from_utf8(line.clone()).unwrap(),
                Line::TooLong => "<too long>".into(),
            });
            assert!(line.capacity() <= 64, "buffer stays bounded");
        }
        assert_eq!(seen, ["ok", "<too long>", "next", "last"]);
    }
}
