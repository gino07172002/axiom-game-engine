use axiom_core::Store;

#[test]
fn load_profile_reports_each_stage_with_scope() {
    let registry = axiom_runtime::installed_registry();
    let world = axiom_fps::demo_world().unwrap();
    let store = Store::memory_with(registry.authoring.clone(), world).unwrap();
    let p = axiom_runtime::measure::load_profile(&store, &registry, 3).unwrap();
    assert_eq!(p["runs"], 3);
    assert!(p["world_json_bytes"].as_u64().unwrap() > 1000);
    assert!(p["entities"].as_u64().unwrap() > 0);
    for stage in ["read_decode_ms", "compile_ms", "runtime_build_ms"] {
        let s = &p[stage];
        let (min, med, max) = (
            s["min"].as_f64().unwrap(),
            s["median"].as_f64().unwrap(),
            s["max"].as_f64().unwrap(),
        );
        assert!(0.0 <= min && min <= med && med <= max, "{stage}: {s}");
    }
    assert!(p["scope"].as_str().unwrap().contains("no GPU"));
    #[cfg(windows)]
    assert!(p["process_memory"]["working_set_bytes"].as_u64().unwrap() > 0);
}
