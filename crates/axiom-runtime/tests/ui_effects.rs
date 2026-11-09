//! Demo step 6 headless: Axiom Town with the UI Editor's edits applied as ACL
//! (`examples/town-ui-effects.acl`) runs `examples/town-ui-effects.scenario.json`, which checks
//! the effects through `observe` (`ui_effects`) at given ticks.
use axiom_core::{ContentPackage, Request, Store};
use axiom_runtime::{installed_registry, scenario};

fn example(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../examples/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn town_ui_effects_scenario_passes_headless() {
    let town = ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world;
    let mut store = Store::memory_with(installed_registry().authoring, town).unwrap();
    store
        .execute(Request::Acl {
            text: example("town-ui-effects.acl"),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .unwrap();
    let (rev, world) = store.current().unwrap();
    let doc = &world.resources["ui.document"];
    assert_eq!(doc["regions"]["affection"], "top_right");
    assert_eq!(doc["styles"]["affection"], "value");
    let plan: scenario::Scenario =
        serde_json::from_str(&example("town-ui-effects.scenario.json")).unwrap();
    let run = |world| scenario::run(world, rev, installed_registry(), plan.clone()).unwrap();
    let result = run(world.clone());
    for a in result["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .chain(result["intermediate_assertions"].as_array().unwrap())
    {
        assert_eq!(a["passed"], true, "{a}");
    }
    // Deterministic with the fixed step: the same run gives the same state hash.
    assert_eq!(run(world)["state_hash"], result["state_hash"]);
}
