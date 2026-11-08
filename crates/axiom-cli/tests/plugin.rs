#[path = "../examples/custom_plugin/plugin.rs"]
mod plugin;

use axiom_core::{ContentPackage, World};
use axiom_runtime::world::{ActionMap, Plugin, RuntimeRegistry, WorldRuntime};
use serde_json::json;

fn spin_world() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/spin-pack"),
        &axiom_runtime::installed_registry()
            .with(plugin::SpinPlugin)
            .authoring,
    )
    .unwrap()
    .world
}

#[test]
fn external_plugin_adds_a_system_without_touching_engine_crates() {
    let registry = axiom_runtime::installed_registry().with(plugin::SpinPlugin);
    let mut r = WorldRuntime::new(spin_world(), 1, registry).unwrap();
    assert!(r.observation().systems.contains(&"spin.step".to_string()));
    for _ in 0..60 {
        r.step(&ActionMap::default());
    }
    // 90 deg/s for 0.5 s = 45 degrees about Y.
    let slot = r.state.compiled.indices["cube"];
    let q = glam::Quat::from_array(r.state.transforms[slot].as_ref().unwrap().rotation);
    let (axis, angle) = q.to_axis_angle();
    assert!(
        (angle.to_degrees() - 45.0).abs() < 0.01,
        "{}",
        angle.to_degrees()
    );
    assert!(axis.y > 0.999);
}

#[test]
fn worlds_using_the_plugin_are_rejected_by_a_registry_without_it() {
    let error = ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/spin-pack"),
        &axiom_runtime::installed_registry().authoring,
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("spin"), "{error:#}");
}

#[test]
fn plugin_schema_validation_applies_to_edits() {
    let registry = axiom_runtime::installed_registry().with(plugin::SpinPlugin);
    let mut store = axiom_core::Store::memory_with(registry.authoring, spin_world()).unwrap();
    let error = store
        .execute(axiom_core::Request::Acl {
            text: "set cube spin.rate/degrees_per_second 99999".into(),
            preview: false,
            expected_rev: None,
        })
        .unwrap_err();
    assert!(format!("{error:#}").contains("-3600..3600"), "{error:#}");
}

struct Shadowing;
impl Plugin for Shadowing {
    fn id(&self) -> &str {
        "bad.shadow"
    }
    fn register(&self, r: &mut RuntimeRegistry) {
        r.factories
            .insert("fsm.step".into(), |_| anyhow::bail!("never built"));
        r.authoring.actions.get_mut("fsm.inspect").unwrap().schema = json!({"hijacked":true});
    }
}

#[test]
fn plugins_cannot_silently_replace_installed_systems_or_actions() {
    let error = axiom_runtime::installed_registry()
        .try_with(Shadowing)
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("replaces installed"),
        "{error:#}"
    );
}
