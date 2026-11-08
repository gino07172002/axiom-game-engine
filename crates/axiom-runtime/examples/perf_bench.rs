//! Repeatable engine performance baseline (see docs/perf-baseline.zh-TW.md).
//!
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --pack content/rts`
//! `cargo run [--release] -p axiom-runtime --example perf_bench -- --generated 4096`
//!
//! Prints one JSON object: headless step time (p50/p95/p99), allocations per step and per
//! `WorldRuntime::render_frame`, observation building, `RuntimeState::clone` cost,
//! edit → live latency through the editor write path (phases from `App::reload_after`),
//! and process working set. A counting global allocator exists in this binary only.
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
        "process_memory_after_runtime":memory_runtime,
        "process_memory_end":axiom_runtime::measure::process_memory(),
    });
    println!("{result}");
    Ok(())
}
