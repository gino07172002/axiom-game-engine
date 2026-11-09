//! Repeatable engine performance baseline (see docs/perf-baseline.zh-TW.md).
//!
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --pack content/rts`
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated 4096`
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated-triggers 256x1024`
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated-paths 64x1024`
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated-nav 256 [--nav-budget 4096]
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated-interact 256x64``
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated-tween 1024x4`
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated-audio 256`
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated-camera 8x1024`
//!
//! Prints one JSON object: headless step time (p50/p95/p99), allocations per step and per
//! `WorldRuntime::render_frame`, observation building, `RuntimeState::clone` cost,
//! edit → live latency through the editor write path (background writer by default,
//! `--sync-writes` for the old main-thread commit; editor-side main-thread time per commit
//! in `edit_live.editor_main_ms`; phases from `App::reload_after`), and process working
//! set. A counting global allocator exists in this binary only.
use anyhow::{Context, Result};
use axiom_core::{CoreEdit, Edit, World};
use axiom_runtime::{world::RuntimeRegistry, ActionMap, WorldRuntime};
use serde_json::{json, Value};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::Instant,
};

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(new_size as u64, Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

fn counters() -> (u64, u64) {
    (ALLOCS.load(Relaxed), BYTES.load(Relaxed))
}

/// Sorted-sample distribution. Percentiles use the nearest-rank (ceil) convention of
/// `rts_stress`.
fn dist(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    let p = |q: f64| samples[((n - 1) as f64 * q).ceil() as usize];
    json!({"n":n,"min":samples[0],"p50":p(0.5),"p95":p(0.95),"p99":p(0.99),"max":samples[n-1],"mean":samples.iter().sum::<f64>()/n as f64})
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// A flat world of `count` entities: one camera, one floor and moving/static boxes.
fn generated(count: usize) -> Result<World> {
    let mut entities = vec![
        json!({"id":"camera","components":{"core.transform":{"position":[0,40,90],"rotation":[-0.2588190,0,0,0.9659258]},"render.camera":{"projection":{"kind":"perspective","fov_degrees":60},"near":0.1,"far":400,"viewport":[0,0,1,1],"active":true}}}),
        json!({"id":"floor","components":{"core.transform":{"position":[0,-0.5,0],"scale":[140,1,140]},"render.mesh":{"parts":[{"color":[0.15,0.19,0.23],"grid":true}]}}}),
    ];
    for i in 0..count.saturating_sub(2) {
        let (x, z) = ((i % 64) as f32 * 2.0 - 63.0, (i / 64) as f32 * 2.0 - 63.0);
        let mut components = json!({"core.transform":{"position":[x,0.5,z]},"render.mesh":{"parts":[{"color":[0.2+(i%7) as f32*0.1,0.5,0.6]}]}});
        if i % 4 == 0 {
            components["core.motion"] = json!({"angular_velocity":[0,0.7,0]});
        }
        entities.push(json!({"id":format!("box/{i}"),"components":components}));
    }
    let ids: Vec<Value> = entities.iter().map(|e| e["id"].clone()).collect();
    Ok(serde_json::from_value(json!({
        "schema_version":2,"id":format!("generated-{count}"),"title":format!("Generated {count}"),
        "capabilities":{"core":1,"render":1,"motion":1},"systems":["motion.integrate"],
        "entities":entities,
        "scenes":[{"id":"main","entities":ids,"active":true}],
        "resources":{"render.settings":{"accent":[0.2,0.9,0.7],"shadow_extent":60}}
    }))?)
}

/// `zones` sphere trigger zones (radius 1, 4 m grid) and `movers` tagged movers drifting
/// through them (2 m grid, 0.5 m/s): the brute-force overlap test at `zones x movers`.
fn generated_triggers(zones: usize, movers: usize) -> Result<World> {
    let mut entities = vec![];
    for i in 0..zones {
        let (x, z) = ((i % 16) as f32 * 4.0 - 30.0, (i / 16) as f32 * 4.0 - 30.0);
        entities.push(
            json!({"id":format!("zone/{i}"),"components":{"core.transform":{"position":[x,0,z]},
            "trigger.zone":{"shape":"sphere","radius":1.0,"filter":{"tags":["player"]}}}}),
        );
    }
    for i in 0..movers {
        let (x, z) = ((i % 32) as f32 * 2.0 - 31.0, (i / 32) as f32 * 2.0 - 31.0);
        entities.push(json!({"id":format!("mover/{i}"),"tags":["player"],"components":{"core.transform":{"position":[x,0,z]},
            "core.motion":{"velocity":[0.5,0,0.25]}}}));
    }
    Ok(serde_json::from_value(json!({
        "schema_version":2,"id":format!("triggers-{zones}x{movers}"),"title":"Trigger bench",
        "capabilities":{"core":1,"motion":1,"trigger":1},"systems":["motion.integrate","trigger.detect"],
        "entities":entities
    }))?)
}

/// `targets` interactables (range 2 m, 3 m grid, alternate priorities, every third with a
/// 120 degree view angle) and `players` tagged interactors drifting through them (0.6 m/s,
/// spinning so facing changes): `targets x players` brute-force focus tests per step.
fn generated_interact(targets: usize, players: usize) -> Result<World> {
    let mut entities = vec![];
    for i in 0..targets {
        let (x, z) = ((i % 16) as f32 * 3.0 - 22.5, (i / 16) as f32 * 3.0 - 22.5);
        let mut target = json!({"range":2.0,"exit_margin":0.5,"priority":(i % 3) as i32});
        if i % 3 == 0 {
            target["view_angle"] = json!(120.0);
        }
        entities.push(json!({"id":format!("target/{i}"),"components":{"core.transform":{"position":[x,0,z]},"interact.target":target}}));
    }
    for i in 0..players {
        let (x, z) = ((i % 8) as f32 * 5.0 - 20.0, (i / 8) as f32 * 5.0 - 20.0);
        entities.push(json!({"id":format!("player/{i}"),"tags":["player"],"components":{"core.transform":{"position":[x,0,z]},
            "core.motion":{"velocity":[0.6,0,0.3],"angular_velocity":[0,0.8,0]}}}));
    }
    Ok(serde_json::from_value(json!({
        "schema_version":2,"id":format!("interact-{targets}x{players}"),"title":"Interaction bench",
        "capabilities":{"core":1,"motion":1,"interact":1},"systems":["motion.integrate","interact.focus"],
        "entities":entities,
        "resources":{"input.bindings":{"buttons":{"interact":"KeyE"}}}
    }))?)
}

/// `components` cubes 2 m apart, each with one autoplay clip of `tracks` (1..4) tracks on
/// itself: yaw, position, scale, mesh colour, 8 keys over 2 s with mixed easing; clips
/// alternate loop / ping_pong with speeds 0.5..2, so every component samples every step.
fn generated_tween(components: usize, tracks: usize) -> Result<World> {
    anyhow::ensure!((1..=4).contains(&tracks), "1..4 tracks per component");
    let eases = ["linear", "ease_in", "ease_out", "ease_in_out", "step"];
    let mut entities = vec![];
    for i in 0..components {
        let (x, z) = ((i % 32) as f32 * 2.0, (i / 32) as f32 * 2.0);
        let keys = |f: &dyn Fn(usize) -> Value| -> Vec<Value> {
            (0..8)
                .map(|k| json!({"t":k as f32 * 2.0 / 7.0,"v":f(k),"ease":eases[(i + k) % eases.len()]}))
                .collect()
        };
        let all = [
            json!({"property":"core.transform/yaw","keys":keys(&|k| json!(k as f32 * 45.0))}),
            json!({"property":"core.transform/position","keys":keys(&|k| json!([x, (k % 3) as f32 * 0.5, z]))}),
            json!({"property":"core.transform/scale","keys":keys(&|k| json!([1.0 + (k % 2) as f32 * 0.5, 1.0, 1.0]))}),
            json!({"property":"render.mesh/color","keys":keys(&|k| json!([(k % 2) as f32, 0.5, 1.0 - (k % 2) as f32]))}),
        ];
        let clip = json!({"autoplay":true,"mode":if i % 2 == 0 {"loop"} else {"ping_pong"},
            "speed":0.5 + (i % 4) as f32 * 0.5,"tracks":all[..tracks]});
        entities.push(
            json!({"id":format!("tween/{i}"),"components":{"core.transform":{"position":[x,0,z]},
            "render.mesh":{"parts":[{"color":[0.5,0.5,0.5]}]},"tween.clips":{"clips":{"a":clip}}}}),
        );
    }
    Ok(serde_json::from_value(json!({
        "schema_version":2,"id":format!("tween-{components}x{tracks}"),"title":"Tween bench",
        "capabilities":{"core":1,"render":1,"tween":1},"systems":["tween.animate"],
        "entities":entities
    }))?)
}

/// `sources` looping sounds (4 procedural clips) on a 3 m grid drifting at 0.5 m/s, priorities
/// 0..2, all within reach of a listener that circles the grid centre (it orbits on a parent
/// that spins at 0.5 rad/s): every step attenuates, pans and ranks every source for the
/// 32 voices, and the ranking keeps changing.
fn generated_audio(sources: usize) -> Result<World> {
    let mut entities = vec![
        json!({"id":"pivot","components":{"core.transform":{},"core.motion":{"angular_velocity":[0,0.5,0]}}}),
        json!({"id":"ear","components":{"core.transform":{"position":[12,1,0]},"audio.listener":{}}}),
    ];
    let clips = ["tone", "engine", "fountain", "creak"];
    for i in 0..sources {
        let (x, z) = ((i % 16) as f32 * 3.0 - 22.5, (i / 16) as f32 * 3.0 - 22.5);
        entities.push(
            json!({"id":format!("src/{i}"),"components":{"core.transform":{"position":[x,0,z]},
            "core.motion":{"velocity":[0.5,0,0.25]},
            "audio.source":{"clip":clips[i % 4],"loop":true,"autoplay":true,"max_distance":80,
                "priority":(i % 3),"rolloff":if i % 2 == 0 {"inverse"} else {"linear"}}}}),
        );
    }
    let library: serde_json::Map<String, Value> = clips
        .iter()
        .map(|c| {
            (
                (*c).to_owned(),
                json!(axiom_core::audio::AudioClip::preset(c).unwrap()),
            )
        })
        .collect();
    Ok(serde_json::from_value(json!({
        "schema_version":2,"id":format!("audio-{sources}"),"title":"Audio bench",
        "capabilities":{"core":1,"motion":1,"audio":1},"systems":["motion.integrate","audio.spatial"],
        "entities":entities,
        "relations":[{"kind":"parent","source":"ear","target":"pivot"}],
        "resources":{"audio.library":{"clips":library}}
    }))?)
}

/// `cameras` follow cameras, each orbiting a target that circles (radius 6..20 m, on a
/// spinning parent) through a field of `colliders` static wall boxes (1.6 x 2.5 x 0.3 m,
/// every other one turned 90 deg, a 2.5 m grid): every step casts the look point and the
/// camera through the field, and the cameras keep hitting, pulling in and recovering.
fn generated_camera(cameras: usize, colliders: usize) -> Result<World> {
    let side = (colliders as f32).sqrt().ceil() as usize;
    let half = side as f32 * 2.5 * 0.5;
    let mut entities = vec![];
    let mut relations = vec![];
    for i in 0..colliders {
        let (x, z) = (
            (i % side) as f32 * 2.5 - half,
            (i / side) as f32 * 2.5 - half,
        );
        let rotation = if i % 2 == 0 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            [
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
            ]
        };
        entities.push(json!({"id":format!("wall/{i}"),"components":{"core.transform":{"position":[x,1.25,z],"rotation":rotation},
            "physics.collider":{"shape":{"shape":"box","half_extents":[0.8,1.25,0.15]}}}}));
    }
    for c in 0..cameras {
        let radius = 6.0 + (c as f32 * 1.75) % 14.0;
        entities.push(json!({"id":format!("pivot/{c}"),"components":{"core.transform":{"position":[(c % 3) as f32 * 4.0 - 4.0,0,(c / 3) as f32 * 4.0 - 4.0]},
            "core.motion":{"angular_velocity":[0,0.3 + 0.05 * c as f32,0]}}}));
        entities.push(json!({"id":format!("target/{c}"),"components":{"core.transform":{"position":[radius,0,0]}}}));
        relations.push(
            json!({"kind":"parent","source":format!("target/{c}"),"target":format!("pivot/{c}")}),
        );
        entities.push(json!({"id":format!("cam/{c}"),"components":{"core.transform":{"position":[0,10,0]},
            "render.camera":{"projection":{"kind":"perspective","fov_degrees":60},"near":0.05,"far":200,"viewport":[0,0,1,1],"active":c == 0},
            "camera.follow":{"target":format!("target/{c}"),"distance":7,"yaw":(c * 45) as f32,"pitch":15}}}));
    }
    Ok(serde_json::from_value(json!({
        "schema_version":2,"id":format!("camera-{cameras}x{colliders}"),"title":"Camera bench",
        "capabilities":{"core":1,"motion":1,"render":1,"physics":1,"camera":1},"systems":["motion.integrate","camera.rig"],
        "entities":entities,"relations":relations
    }))?)
}

/// `paths` closed paths 30 m apart (even: linear 20 x 10 m rectangles, odd: Catmull-Rom
/// circles of 16 points, radius 8 m) and `followers` spread evenly over them (start fraction,
/// speed 1.5..3 m/s, lateral offset +-1 m, every other one with a 180 deg/s turn rate).
fn generated_paths(paths: usize, followers: usize) -> Result<World> {
    let mut entities = vec![];
    for i in 0..paths {
        let (x, z) = ((i % 8) as f32 * 30.0, (i / 8) as f32 * 30.0);
        let spline = if i % 2 == 0 {
            json!({"points":[[-10,0,-5],[10,0,-5],[10,0,5],[-10,0,5]],"closed":true})
        } else {
            let points: Vec<[f32; 3]> = (0..16)
                .map(|k| {
                    let a = k as f32 / 16.0 * std::f32::consts::TAU;
                    [8.0 * a.cos(), 0.0, 8.0 * a.sin()]
                })
                .collect();
            json!({"points":points,"closed":true,"interpolation":"catmull_rom"})
        };
        entities.push(json!({"id":format!("path/{i}"),"components":{"core.transform":{"position":[x,0,z]},"path.spline":spline}}));
    }
    let per_path = followers.div_ceil(paths.max(1));
    for i in 0..followers {
        let (path, k) = (i % paths.max(1), i / paths.max(1));
        let mut follow = json!({"path":format!("path/{path}"),"speed":1.5 + (i % 4) as f32 * 0.5,
            "start_fraction":k as f32 / per_path as f32,"offset":if i % 2 == 0 { 1.0 } else { -1.0 }});
        if i % 2 == 1 {
            follow["turn_rate"] = json!(180.0);
        }
        entities.push(json!({"id":format!("follower/{i}"),"components":{"core.transform":{},"path.follow":follow}}));
    }
    Ok(serde_json::from_value(json!({
        "schema_version":2,"id":format!("paths-{paths}x{followers}"),"title":"Path bench",
        "capabilities":{"core":1,"path":1},"systems":["path.move"],
        "entities":entities
    }))?)
}

/// `agents` navigation agents on a 64 x 64 m town (8 x 8 blocks of 4 x 4 m `nav.blocker`,
/// 4 m streets; grid 256 x 256 cells of 0.25 m) chasing `agents` markers that circle two
/// street loops at 2 m/s. Agents walk at 1.4 m/s with `follow_target`, so they never catch
/// up and re-plan every time their marker moves a meter: the pathfinding queue stays full
/// and every step spends its whole search budget.
fn generated_nav(agents: usize, budget: Option<u32>) -> Result<World> {
    let mut entities = vec![];
    for j in 0..8 {
        for i in 0..8 {
            let (x, z) = (-28.0 + 8.0 * i as f32, -28.0 + 8.0 * j as f32);
            entities.push(json!({"id":format!("block/{i}/{j}"),"components":{"core.transform":{"position":[x,0,z]},"nav.blocker":{"half":[2.0,2.0]}}}));
        }
    }
    for (k, (lo, hi)) in [(-24.0, 24.0), (-8.0, 8.0)].into_iter().enumerate() {
        entities.push(
            json!({"id":format!("loop/{k}"),"components":{"core.transform":{},
            "path.spline":{"points":[[lo,0,lo],[hi,0,lo],[hi,0,hi],[lo,0,hi]],"closed":true}}}),
        );
    }
    for k in 0..agents {
        entities.push(json!({"id":format!("marker/{k}"),"components":{"core.transform":{},
            "path.follow":{"path":format!("loop/{}", k % 2),"speed":2.0,"start_fraction":(k / 2) as f32 / agents.div_ceil(2) as f32,"orient":false}}}));
    }
    let per_street = agents.div_ceil(7).max(1);
    for k in 0..agents {
        let x = -24.0 + 8.0 * (k % 7) as f32;
        let z = -30.0 + 60.0 * (k / 7) as f32 / per_street as f32;
        let target = (k * 7919 + 13) % agents;
        entities.push(json!({"id":format!("agent/{k}"),"components":{"core.transform":{"position":[x,0,z]},
            "nav.agent":{"speed":1.4,"radius":0.3,"target":{"entity":format!("marker/{target}")},"follow_target":true}}}));
    }
    let mut grid = json!({"bounds":{"min":[-32,-32],"max":[31.99,31.99]}});
    if let Some(budget) = budget {
        grid["expansions_per_step"] = json!(budget);
    }
    Ok(serde_json::from_value(json!({
        "schema_version":2,"id":format!("nav-{agents}"),"title":"Navigation bench",
        "capabilities":{"core":1,"path":1,"nav":1},"systems":["path.move","nav.steer"],
        "entities":entities,
        "resources":{"nav.grid":grid}
    }))?)
}

/// A one-field edit on an entity that owns its transform position (non-camera preferred).
fn edit_target(world: &World) -> Result<(String, Vec<f64>)> {
    let pick = |skip_camera: bool| {
        world.entities.iter().find_map(|e| {
            if skip_camera && e.components.contains_key("render.camera") {
                return None;
            }
            let p = e.components.get("core.transform")?.get("position")?;
            let p: Vec<f64> = serde_json::from_value(p.clone()).ok()?;
            (p.len() == 3).then(|| (e.id.clone(), p))
        })
    };
    pick(true)
        .or_else(|| pick(false))
        .context("no entity owns a transform position")
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let ticks: usize = arg("--ticks").map_or(Ok(1200), |v| v.parse())?;
    let edits: usize = arg("--edits").map_or(Ok(15), |v| v.parse())?;
    let scratch = PathBuf::from(arg("--scratch").unwrap_or("artifacts/perf-scratch".into()));
    std::fs::create_dir_all(&scratch)?;
    let registry: RuntimeRegistry = axiom_runtime::installed_registry();
    let load = Instant::now();
    let (name, world) = if let Some(n) = arg("--generated") {
        let n: usize = n.parse()?;
        (format!("generated-{n}"), generated(n)?)
    } else if let Some(spec) = arg("--generated-triggers") {
        let (zones, movers) = spec
            .split_once('x')
            .context("--generated-triggers ZONESxMOVERS")?;
        let world = generated_triggers(zones.parse()?, movers.parse()?)?;
        (world.id.clone(), world)
    } else if let Some(spec) = arg("--generated-paths") {
        let (paths, followers) = spec
            .split_once('x')
            .context("--generated-paths PATHSxFOLLOWERS")?;
        let world = generated_paths(paths.parse()?, followers.parse()?)?;
        (world.id.clone(), world)
    } else if let Some(spec) = arg("--generated-interact") {
        let (targets, players) = spec
            .split_once('x')
            .context("--generated-interact TARGETSxPLAYERS")?;
        let world = generated_interact(targets.parse()?, players.parse()?)?;
        (world.id.clone(), world)
    } else if let Some(spec) = arg("--generated-tween") {
        let (components, tracks) = spec
            .split_once('x')
            .context("--generated-tween COMPONENTSxTRACKS")?;
        let world = generated_tween(components.parse()?, tracks.parse()?)?;
        (world.id.clone(), world)
    } else if let Some(spec) = arg("--generated-camera") {
        let (cameras, colliders) = spec
            .split_once('x')
            .context("--generated-camera CAMERASxCOLLIDERS")?;
        let world = generated_camera(cameras.parse()?, colliders.parse()?)?;
        (world.id.clone(), world)
    } else if let Some(sources) = arg("--generated-audio") {
        let world = generated_audio(sources.parse()?)?;
        (world.id.clone(), world)
    } else if let Some(agents) = arg("--generated-nav") {
        let budget = arg("--nav-budget").map(|b| b.parse()).transpose()?;
        let world = generated_nav(agents.parse()?, budget)?;
        (world.id.clone(), world)
    } else {
        let pack = arg("--pack").context("--pack DIR or --generated N")?;
        let world = axiom_core::ContentPackage::load(&pack, &registry.authoring)?.world;
        (
            pack.trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap()
                .to_owned(),
            world,
        )
    };
    let load_ms = ms(load);
    if let Some(dir) = arg("--dump-pack") {
        // Writes the benchmark world as a content package (for the live GPU script).
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            PathBuf::from(&dir).join("manifest.json"),
            json!({"id":format!("bench.{}", world.id),"version":1,"world":"world.json","capabilities":world.capabilities}).to_string(),
        )?;
        std::fs::write(
            PathBuf::from(&dir).join("world.json"),
            serde_json::to_vec(&world)?,
        )?;
        println!("{}", json!({"ok":true,"pack":dir}));
        return Ok(());
    }
    let world_bytes = serde_json::to_vec(&world)?.len();
    let entity_count = world.entities.len();

    // Headless simulation.
    let mut runtime = WorldRuntime::new(world.clone(), 1, registry.clone())?;
    let build = runtime.build_timings;
    let system_build_ms = runtime.system_build_ms.clone();
    let input = ActionMap::default();
    for _ in 0..120 {
        runtime.step(&input);
    }
    let (mut step_ms, mut step_allocs, mut step_bytes) = (vec![], vec![], vec![]);
    for _ in 0..ticks {
        let (a, b) = counters();
        let t = Instant::now();
        runtime.step(&input);
        step_ms.push(ms(t));
        let (a2, b2) = counters();
        step_allocs.push((a2 - a) as f64);
        step_bytes.push((b2 - b) as f64);
    }
    runtime.assert_valid()?;
    // Navigation: final counters, and one cold editor path preview (main-thread cost).
    let preview = runtime
        .system::<axiom_runtime::nav_agent::NavSystem>()
        .and_then(|s| {
            let slot = *runtime.state.compiled.indices.get("agent/0")?;
            let t = Instant::now();
            let path = s.preview_path(&runtime.state, slot);
            Some(json!({"ms":ms(t),"waypoints":path.map(|p| p.len())}))
        });
    let nav = runtime.observation().metrics.get("nav").map(|n| {
        json!({"grid":n["grid"],"budget":n["budget"],"moving":n["moving"],"pending":n["pending"],"failed":n["failed"],"arrived":n["arrived"],"preview":preview})
    });
    // Follow cameras: what the last step's casts did.
    let camera = runtime.observation().metrics.get("camera").map(|c| {
        let follow = c["follow"].as_array().cloned().unwrap_or_default();
        let hits = follow.iter().filter(|f| !f["hit"].is_null()).count();
        let tests: u64 = follow.iter().filter_map(|f| f["tests"].as_u64()).sum();
        json!({"cameras":follow.len(),"blocked":hits,"tests_last_step":tests,"blockers":c["blockers"],"grid":c["grid"],"over_budget":c["over_budget"]})
    });
    // Audio: the audio thread's work per device buffer (512 frames = 23.2 ms of sound) with
    // this world's voices, and the voice counters after the measured steps.
    let audio = runtime
        .system::<axiom_runtime::spatial_audio::AudioSpatialSystem>()
        .map(|s| {
            use axiom_runtime::audio_mix::Mixer;
            let mut mixer = Mixer::default();
            let _ = mixer.set_clips(s.clip_table().clone());
            let mut out = vec![0i16; 1024];
            // Pre-sized so recording a sample never allocates inside the counted window.
            let (mut mix_ms, mut mix_allocs) = (Vec::with_capacity(2000), Vec::with_capacity(2000));
            // The first buffer starts every voice; it stays in the distribution.
            for _ in 0..2000 {
                let (a, _) = counters();
                let t = Instant::now();
                mixer.set_params(s.voices());
                mixer.render(&mut out);
                mix_ms.push(ms(t));
                mix_allocs.push((counters().0 - a) as f64);
            }
            std::hint::black_box(&out);
            let o = &runtime.observation().metrics["audio"];
            json!({"sources":o["sources"],"audible":o["audible"],"voiced":o["voiced"],"virtual":o["virtual"],
                "steals":o["steals"],"clip_bytes":o["clips"]["bytes"],"mixed_voices":mixer.stats.voices,
                "mix_512_frames":{"ms":dist(mix_ms),"allocs":dist(mix_allocs)}})
        });

    // Render-frame extraction, the renderer's per-frame instance copy, observation building.
    let (mut frame_ms, mut frame_allocs, mut frame_bytes) = (vec![], vec![], vec![]);
    let (mut copy_ms, mut copy_allocs) = (vec![], vec![]);
    let (mut obs_ms, mut obs_allocs, mut obs_json_ms) = (vec![], vec![], vec![]);
    let mut instances = 0;
    for _ in 0..240 {
        runtime.step(&input);
        let (a, b) = counters();
        let t = Instant::now();
        let frame = runtime.render_frame();
        frame_ms.push(ms(t));
        let (a2, b2) = counters();
        frame_allocs.push((a2 - a) as f64);
        frame_bytes.push((b2 - b) as f64);
        instances =
            frame.instances.len() + frame.views.iter().map(|v| v.instances.len()).sum::<usize>();
        let (a, _) = counters();
        let t = Instant::now();
        let copy = std::hint::black_box(frame.instances.clone());
        copy_ms.push(ms(t));
        copy_allocs.push((counters().0 - a) as f64);
        drop(copy);
        let (a, _) = counters();
        let t = Instant::now();
        let obs = runtime.observation();
        obs_ms.push(ms(t));
        obs_allocs.push((counters().0 - a) as f64);
        let t = Instant::now();
        std::hint::black_box(serde_json::to_vec(&obs)?);
        obs_json_ms.push(ms(t));
    }
    let (mut clone_ms, mut clone_allocs) = (vec![], vec![]);
    for _ in 0..20 {
        let (a, _) = counters();
        let t = Instant::now();
        let copy = std::hint::black_box(runtime.state.clone());
        clone_ms.push(ms(t));
        clone_allocs.push((counters().0 - a) as f64);
        drop(copy);
    }
    let (mut world_clone_ms, mut world_clone_allocs) = (vec![], vec![]);
    for _ in 0..20 {
        let (a, _) = counters();
        let t = Instant::now();
        let copy = std::hint::black_box(world.clone());
        world_clone_ms.push(ms(t));
        world_clone_allocs.push((counters().0 - a) as f64);
        drop(copy);
    }
    let memory_runtime = axiom_runtime::measure::process_memory();
    drop(runtime);

    // Edit → live through the editor write path, against an isolated file database.
    let (target, position) = edit_target(&world)?;
    let batches = (0..=edits)
        .map(|i| {
            let mut p = position.clone();
            p[0] += 0.001 * (i as f64 + 1.0);
            vec![Edit::Core(CoreEdit::SetField {
                target: axiom_core::local::Target::Entity { id: target.clone() },
                field: "/components/core.transform/position".into(),
                value: json!(p),
            })]
        })
        .collect();
    let db = scratch.join(format!("bench-{name}.sqlite"));
    let mode = arg("--mode").unwrap_or("editor".into());
    let samples = axiom_runtime::app::measure_edit_live(
        world,
        registry,
        db,
        mode == "editor",
        !args.iter().any(|a| a == "--sync"),
        // Editor commits on the background writer (the editor's real path); `--sync-writes`
        // measures the old main-thread commit.
        !args.iter().any(|a| a == "--sync-writes"),
        batches,
    )?;
    let samples = &samples[1..]; // first edit warms SQLite statement caches
    let pick = |f: &dyn Fn(&axiom_runtime::app::EditLiveSample) -> f64| {
        dist(samples.iter().map(f).collect())
    };
    let reconcile = |f: &dyn Fn(&axiom_runtime::world::ReconcileTimings) -> f64| {
        pick(&|s| s.reload.reconcile.as_ref().map_or(0.0, f))
    };
    let result = json!({
        "pack":name,
        "build_profile":if cfg!(debug_assertions) {"debug"} else {"release"},
        "world_json_bytes":world_bytes,
        "pack_load_ms":load_ms,
        "runtime_new":build,
        "system_build_ms":system_build_ms,
        "entities":entity_count,
        "instances":instances,
        "step":{"ticks":ticks,"ms":dist(step_ms),"allocs":dist(step_allocs),"alloc_bytes":dist(step_bytes)},
        "render_frame":{"ms":dist(frame_ms),"allocs":dist(frame_allocs),"alloc_bytes":dist(frame_bytes)},
        "instance_copy":{"ms":dist(copy_ms),"allocs":dist(copy_allocs)},
        "observation":{"ms":dist(obs_ms),"allocs":dist(obs_allocs),"json_ms":dist(obs_json_ms)},
        "state_clone":{"ms":dist(clone_ms),"allocs":dist(clone_allocs)},
        "world_clone":{"ms":dist(world_clone_ms),"allocs":dist(world_clone_allocs)},
        "edit_live":{
            "mode":mode,"edits":samples.len(),"target":target,
            "edit_to_live_ms":pick(&|s| s.edit_to_live_ms),
            "commit_ms":pick(&|s| s.commit_ms),
            "writer":!args.iter().any(|a| a == "--sync-writes"),
            "editor_main_ms":pick(&|s| s.editor_main_ms),
            "editor_piece_max_ms":pick(&|s| s.editor_piece_max_ms),
            "staged_ms":pick(&|s| s.staged_ms),
            "main_thread_reload_ms":pick(&|s| s.reload.total_ms),
            "read_ms":pick(&|s| s.reload.read_ms),
            "editor_clone_ms":pick(&|s| s.reload.editor_clone_ms),
            "editor_update_ms":pick(&|s| s.reload.editor_update_ms),
            "compile_ms":reconcile(&|r| r.build.compile_ms),
            "shader_library_ms":reconcile(&|r| r.build.shader_library_ms),
            "systems_ms":reconcile(&|r| r.build.systems_ms),
            "transfer_ms":reconcile(&|r| r.transfer_ms),
            "prepare_ms":reconcile(&|r| r.prepare_ms),
            "reconcile_ms":reconcile(&|r| r.total_ms),
            "background":samples[0].reload.background,
            "request_ms":pick(&|s| s.reload.request_ms),
            "adopt_ms":pick(&|s| s.reload.adopt_ms),
            "build_ms":pick(&|s| s.reload.build_ms),
        },
        "nav":nav,
        "audio":audio,
        "camera":camera,
        "process_memory_after_runtime":memory_runtime,
        "process_memory_end":axiom_runtime::measure::process_memory(),
    });
    println!("{result}");
    Ok(())
}
