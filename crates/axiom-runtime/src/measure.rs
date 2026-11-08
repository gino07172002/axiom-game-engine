//! Load-time and memory measurements reported to agents. Every figure states its scope.
use crate::world::{RuntimeRegistry, WorldRuntime};
use anyhow::Result;
use axiom_core::Store;
use serde_json::{json, Value};
use std::time::Instant;

pub use crate::rts_stress::process_memory;

fn stats(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    json!({"min":samples[0],"median":samples[samples.len()/2],"max":samples[samples.len()-1]})
}

/// Times the authoring → runtime path for the current revision, `runs` times each:
/// SQLite read + JSON decode, IR compile, and full `WorldRuntime` construction
/// (compile + native system factories). No GPU work is included.
pub fn load_profile(store: &Store, registry: &RuntimeRegistry, runs: usize) -> Result<Value> {
    let runs = runs.clamp(1, 32);
    let (mut read, mut compile, mut runtime) = (vec![], vec![], vec![]);
    let mut bytes = 0;
    let mut entities = 0;
    let mut rev = 0;
    for _ in 0..runs {
        let t = Instant::now();
        let (r, world) = store.current()?;
        read.push(t.elapsed().as_secs_f64() * 1000.0);
        bytes = serde_json::to_vec(&world)?.len();
        rev = r;
        let t = Instant::now();
        let compiled = world.compile(&registry.authoring)?;
        compile.push(t.elapsed().as_secs_f64() * 1000.0);
        entities = compiled.len();
        let t = Instant::now();
        let built = WorldRuntime::new(world, r, registry.clone())?;
        runtime.push(t.elapsed().as_secs_f64() * 1000.0);
        drop(built);
    }
    Ok(json!({
        "rev":rev,"runs":runs,"world_json_bytes":bytes,"entities":entities,
        "read_decode_ms":stats(read),"compile_ms":stats(compile),"runtime_build_ms":stats(runtime),
        "process_memory":process_memory(),
        "build_profile":if cfg!(debug_assertions) {"dev"} else {"release"},
        "scope":"wall-clock in this process; read_decode = SQLite row + JSON parse + IR decode; runtime_build includes a second compile and native system setup; no GPU upload"
    }))
}

/// GPU allocator totals where the backend exposes them (Vulkan/D3D12 via gpu-allocator).
pub fn gpu_memory(device: &wgpu::Device) -> Value {
    match device.generate_allocator_report() {
        Some(report) => json!({
            "allocated_bytes":report.total_allocated_bytes,
            "reserved_bytes":report.total_reserved_bytes,
            "blocks":report.blocks.len(),
            "scope":"wgpu allocator only; excludes swapchain images and driver-internal memory"
        }),
        None => json!({"available":false,"scope":"backend does not expose an allocator report"}),
    }
}
