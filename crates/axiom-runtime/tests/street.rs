//! Axiom Street (final acceptance demo): the pack is reproducible from its authoring script and
//! its headless scenario passes. The script is replayed in-process through the same protocol the
//! CLI uses (one ACL revision per stage, the seed included); see `scripts/build-street.ps1`.
use axiom_core::{ContentPackage, Request, Store, World};
use axiom_runtime::{installed_registry, scenario};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(pack: &str) -> World {
    ContentPackage::load(root().join(pack), &installed_registry().authoring)
        .unwrap()
        .world
}

/// Text round trip, so numbers compare by their written form (f32 vs parsed JSON).
fn canonical(world: &World) -> Value {
    serde_json::from_str(&serde_json::to_string(world).unwrap()).unwrap()
}

fn replay_build_script() -> (i64, World) {
    let mut store =
        Store::memory_with(installed_registry().authoring, load("content/empty")).unwrap();
    let build = root().join("content/street/build");
    let mut stages: Vec<PathBuf> = std::fs::read_dir(&build)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "acl"))
        .collect();
    stages.sort();
    // The seed (world id/title, capabilities, resource shells) is ACL too: no typed request.
    assert!(
        stages.len() >= 7 && stages[0].ends_with("00-seed.acl"),
        "expected the seed and six ACL stages, got {stages:?}"
    );
    for stage in &stages {
        let text = std::fs::read_to_string(stage).unwrap();
        let (before, _) = store.current().unwrap();
        let out = store
            .execute(Request::Acl {
                text,
                preview: false,
                expected_rev: None,
                propose: false,
            })
            .unwrap_or_else(|e| panic!("{}: {e:#}", stage.display()));
        assert_eq!(
            out["rev"].as_i64(),
            Some(before + 1),
            "{} must commit exactly one revision",
            stage.display()
        );
    }
    store.execute(Request::Verify {}).unwrap();
    store.current().unwrap()
}

/// JSON pointers where two documents differ (first `limit`), for a readable failure.
fn differences(a: &Value, b: &Value, path: String, out: &mut Vec<String>, limit: usize) {
    if out.len() >= limit || a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                let (l, r) = (
                    x.get(key).unwrap_or(&Value::Null),
                    y.get(key).unwrap_or(&Value::Null),
                );
                differences(l, r, format!("{path}/{key}"), out, limit);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (l, r)) in x.iter().zip(y).enumerate() {
                differences(l, r, format!("{path}/{i}"), out, limit);
            }
        }
        _ => out.push(format!("{path}: built {a} vs exported {b}")),
    }
}

#[test]
fn street_pack_is_reproducible_from_its_build_script() {
    let (rev, built) = replay_build_script();
    // empty pack (1) + seed (2) + six ACL stages.
    assert_eq!(rev, 8);
    // Compare with the file as exported (ContentPackage::load fills typed defaults, e.g. widget
    // `when`), then check that the package itself loads and compiles.
    let file = std::fs::read_to_string(root().join("content/street/world.json")).unwrap();
    let exported: World = serde_json::from_str(&file).unwrap();
    load("content/street");
    let mut diff = vec![];
    differences(
        &canonical(&built),
        &canonical(&exported),
        String::new(),
        &mut diff,
        12,
    );
    assert!(
        diff.is_empty(),
        "content/street/world.json is stale (run scripts/build-street.ps1):\n{}",
        diff.join("\n")
    );
    built.compile(&installed_registry().authoring).unwrap();
}

#[test]
fn street_scenario_drives_traffic_dialogue_door_and_room() {
    let scenario: scenario::Scenario =
        serde_json::from_str(include_str!("../../../examples/street.scenario.json")).unwrap();
    let report = scenario::run(load("content/street"), 1, installed_registry(), scenario).unwrap();
    assert_eq!(report["ok"], true);
    let camera = &report["observation"]["camera"]["follow"][0];
    assert_eq!(camera["hit"], "cafe/roof", "{camera}");
    let nav = &report["observation"]["nav"];
    assert_eq!(nav["failed"], 0, "{nav}");
}
