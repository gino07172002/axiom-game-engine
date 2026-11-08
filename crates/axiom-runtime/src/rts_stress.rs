//! Measured headless engine workload; renderer/GPU work is deliberately reported separately.
use crate::{
    rts::RtsSystem,
    world::{ActionMap, RuntimeRegistry, WorldRuntime},
};
use anyhow::{ensure, Result};
use axiom_core::gameplay::RuntimeCommand;
use serde_json::{json, Value};
use std::time::Instant;
pub fn run(population: usize, ticks: u32, registry: RuntimeRegistry) -> Result<Value> {
    ensure!(
        [64, 128, 256].contains(&population) && (120..=12000).contains(&ticks),
        "stress supports 64/128/256 units and 120..12000 measured ticks"
    );
    let setup = Instant::now();
    let mut world = axiom_rts::content::demo_world(population)?;
    world.resources.get_mut("rts.session").unwrap()["auto_combat"] = json!(false);
    let mut runtime = WorldRuntime::new(world, 1, registry)?;
    let setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
    for _ in 0..240 {
        runtime.step(&ActionMap::default());
    }
    runtime.assert_valid()?;
    let owned = runtime
        .system::<RtsSystem>()
        .unwrap()
        .sim
        .units
        .iter()
        .filter(|u| u.def.faction == 0)
        .map(|u| u.id.clone())
        .collect::<Vec<_>>();
    runtime.command(&RuntimeCommand::Call {
        capability: "rts".into(),
        action: "select".into(),
        args: json!({"ids":owned}),
    })?;
    let started = Instant::now();
    let mut samples = Vec::with_capacity(ticks as usize);
    let mut command_ms = 0.0;
    let mut validation_ms = 0.0;
    for tick in 0..ticks {
        if tick % 480 == 0 {
            let call = Instant::now();
            let system = runtime.system_mut::<RtsSystem>().unwrap();
            for faction in 0..2 {
                let z = if tick % 960 == 0 { -10.5 } else { 10.5 };
                let x = if faction == 0 { -20.5 } else { 20.5 };
                let ids = system
                    .sim
                    .units
                    .iter()
                    .filter(|u| u.def.faction == faction && u.health > 0.0)
                    .map(|u| u.id.clone())
                    .collect();
                system.sim.move_for_faction(
                    faction,
                    axiom_rts::MoveArgs {
                        units: Some(ids),
                        destination: [x, if faction == 0 { z } else { -z }],
                    },
                )?;
            }
            command_ms += call.elapsed().as_secs_f64() * 1000.0;
        }
        let step = Instant::now();
        runtime.step(&ActionMap::default());
        samples.push(step.elapsed().as_secs_f64() * 1000.0);
        if tick % 120 == 0 {
            let check = Instant::now();
            runtime.assert_valid()?;
            ensure!(
                runtime.system::<RtsSystem>().unwrap().sim.overlaps() == 0,
                "unit footprints overlap during stress"
            );
            validation_ms += check.elapsed().as_secs_f64() * 1000.0;
        }
    }
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    runtime.assert_valid()?;
    let sim = &runtime.system::<RtsSystem>().unwrap().sim;
    ensure!(sim.units.len() == population, "stress population changed");
    ensure!(
        sim.units.iter().all(|u| u.health > 0.0),
        "active stress population changed"
    );
    ensure!(sim.overlaps() == 0, "final footprints overlap");
    samples.sort_by(f64::total_cmp);
    let percentile = |p: f64| samples[((samples.len() - 1) as f64 * p).ceil() as usize];
    Ok(
        json!({"ok":true,"engine":env!("CARGO_PKG_VERSION"),"build_profile":if cfg!(debug_assertions){"dev (unoptimized engine, opt-level 2 dependencies)"}else{"release"},"platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"initial_units":population,"final_units":sim.units.len(),"warmup_ticks":240,"measured_ticks":ticks,"simulated_seconds":ticks as f64/120.0,"decision_stride":sim.session.nav_stride,"path_budget":sim.session.path_budget,"search_budget":sim.session.search_budget,"setup_ms":setup_ms,"wall_ms":wall_ms,"command_ms":command_ms,"validation_ms":validation_ms,"step_ms":{"mean":samples.iter().sum::<f64>()/samples.len()as f64,"p50":percentile(0.50),"p95":percentile(0.95),"p99":percentile(0.99),"max":samples.last()},"memory":process_memory(),"state":sim.summary(),"overlaps":sim.overlaps(),"measurement":"single-thread fixed 120Hz headless WorldRuntime.step; both factions issue group replacements every 480 ticks; combat disabled in stress content; validation and command time outside step samples; no GPU/FPS claim"}),
    )
}
#[cfg(windows)]
pub fn process_memory() -> Value {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak_working: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut counters: Counters = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<Counters>() as u32;
    if unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<Counters>() as u32,
        )
    } != 0
    {
        json!({"working_set_bytes":counters.working,"peak_working_set_bytes":counters.peak_working,"private_bytes":counters.private,"scope":"whole process, fresh process required for independent peak comparison"})
    } else {
        Value::Null
    }
}
#[cfg(not(windows))]
pub fn process_memory() -> Value {
    Value::Null
}
