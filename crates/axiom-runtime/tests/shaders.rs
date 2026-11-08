use axiom_core::{ContentPackage, Request, Store};
use axiom_runtime::{installed_registry, WorldRuntime};
use serde_json::{json, Value};
use std::path::PathBuf;

fn pack() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/shader-lab")
}

fn edit(op: &str, field: &str, value: Value) -> Request {
    serde_json::from_value(json!({
        "op":op, "expected_rev":1, "edits":[{
            "kind":"set_field", "target":{"kind":"resource","id":"render.shaders"},
            "field":field, "value":value
        }]
    }))
    .unwrap()
}

#[test]
fn shader_assets_resolve_to_immutable_sources_and_material_runs() {
    let registry = installed_registry();
    let pkg = ContentPackage::load(pack(), &registry.authoring).unwrap();
    let scan = &pkg.world.resources["render.shaders"]["shaders"]["scan"];
    assert!(scan.get("file").is_none());
    assert!(scan["wgsl"].as_str().unwrap().contains("fn surface"));
    let runtime = WorldRuntime::new(pkg.world, 1, registry).unwrap();
    let frame = runtime.render_frame();
    assert_eq!(frame.cube_materials.len(), 6);
    assert_eq!(frame.shaders.materials["scanner"].values, [8., 1., 0., 0.]);
    assert_eq!(frame.shaders.materials["violet"].values, [3., 0.18, 0., 0.]);
}

#[test]
fn shader_preview_rejects_wgsl_and_parameters_at_stable_paths_without_a_commit() {
    let registry = installed_registry();
    let pkg = ContentPackage::load(pack(), &registry.authoring).unwrap();
    let mut store = Store::memory_with(registry.authoring, pkg.world).unwrap();
    let result = store.execute(edit(
        "preview",
        "/shaders/scan/wgsl",
        json!("fn surface(s: SurfaceInput) -> vec4<f32> {\n    return missing;\n}"),
    ));
    let error = axiom_core::error_response(&result.unwrap_err());
    assert_eq!(
        error["diagnostics"][0]["path"],
        "/resources/render.shaders/shaders/scan/wgsl"
    );
    assert!(error["error"].as_str().unwrap().contains("line 2"));
    let result = store.execute(edit(
        "apply",
        "/materials/scanner/parameters/frequency",
        json!(99),
    ));
    let error = axiom_core::error_response(&result.unwrap_err());
    assert_eq!(
        error["diagnostics"][0]["path"],
        "/resources/render.shaders/materials/scanner/parameters/frequency"
    );
    assert_eq!(store.revision().unwrap(), 1);
    let preview = store
        .execute(edit(
            "preview",
            "/materials/scanner/parameters/frequency",
            json!(12),
        ))
        .unwrap();
    assert!(preview["diff"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["path"].as_str().unwrap().contains("frequency")));
    assert_eq!(store.revision().unwrap(), 1);
    store
        .execute(edit(
            "apply",
            "/materials/scanner/parameters/frequency",
            json!(12),
        ))
        .unwrap();
    let (rev, world) = store.current().unwrap();
    let runtime = WorldRuntime::new(world, rev, installed_registry()).unwrap();
    assert_eq!(
        runtime.render_frame().shaders.materials["scanner"].values[0],
        12.0
    );
}

#[test]
fn surface_hooks_cannot_install_entry_points_bindings_or_loops() {
    use axiom_runtime::shader::validate_hook;
    let valid = "fn surface(s: SurfaceInput) -> vec4<f32> { return s.base_color; }";
    assert!(validate_hook("test", valid).is_ok());
    for suffix in [
        "@group(3) @binding(0) var<uniform> external_value: vec4<f32>;",
        "@compute @workgroup_size(1) fn extra() {}",
        "fn helper() { loop { break; } }",
    ] {
        assert!(validate_hook("test", &format!("{valid}\n{suffix}")).is_err());
    }
}
