//! Invariant over every content pack: an animated character never keeps running backwards
//! (its model front, entity +Z after rotation, pointing away from its motion). Each pack
//! is driven through its own `move*` input axes (forward, right, back, left), then left idle
//! so NPC controllers (fsm, nav, path, tween) move on their own; `observe facing` must not
//! report a `facing_mismatch`.
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const PHASE: u32 = 150;

fn content() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content")
}
fn load(dir: &std::path::Path) -> World {
    ContentPackage::load(dir, &installed_registry().authoring)
        .unwrap_or_else(|e| panic!("{}: {e:#}", dir.display()))
        .world
}
/// Runs the pack's movement axes one by one in both directions, then idles.
fn drive(world: World) -> (WorldRuntime, Value) {
    let mut r = WorldRuntime::new(world, 1, installed_registry()).unwrap();
    if !r.observation().metrics.contains_key("facing") {
        return (r, Value::Null); // No animated character: nothing to drive.
    }
    let mut moves: Vec<String> = r
        .bindings
        .axes
        .keys()
        .filter(|k| k.contains("move"))
        .cloned()
        .collect();
    moves.sort();
    let mut phases: Vec<ActionMap> = vec![];
    for sign in [1.0, -1.0] {
        for axis in &moves {
            phases.push(ActionMap(BTreeMap::from([(axis.clone(), sign)])));
        }
    }
    phases.push(ActionMap::default());
    phases.push(ActionMap::default());
    for input in &phases {
        for _ in 0..PHASE {
            r.step(input);
        }
    }
    let facing = r
        .observation()
        .metrics
        .get("facing")
        .cloned()
        .unwrap_or(Value::Null);
    (r, facing)
}

#[test]
fn no_content_pack_runs_a_character_backwards() {
    let mut checked = vec![];
    let mut failures = vec![];
    let mut dirs: Vec<_> = std::fs::read_dir(content())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let (_, facing) = drive(load(&dir));
        if facing.is_null() {
            continue; // No animated character.
        }
        let mismatches = facing["mismatches"].as_array().unwrap();
        checked.push(format!(
            "{name} ({})",
            facing["characters"].as_object().unwrap().len()
        ));
        for m in mismatches {
            failures.push(format!("{name}: {}", m["message"]));
        }
    }
    eprintln!("facing checked: {checked:?}");
    assert!(
        failures.is_empty(),
        "checked {checked:?}\n{}",
        failures.join("\n")
    );
    assert!(
        checked.iter().any(|c| c.starts_with("axiom-town")),
        "the town pack must be among the checked packs: {checked:?}"
    );
}

#[test]
fn the_monitor_catches_the_town_hero_with_the_old_model_yaw() {
    // The bug that shipped: model_yaw 0 made the hero run backwards. The monitor must flag
    // it (and only the hero), within the 0.5 s window, with a message naming the fix.
    let mut w = load(&content().join("axiom-town"));
    let hero = w.entities.iter_mut().find(|e| e.id == "hero").unwrap();
    hero.components.get_mut("action.actor").unwrap()["model_yaw"] = json!(0.0);
    let (_, facing) = drive(w);
    let mismatches = facing["mismatches"].as_array().unwrap();
    assert_eq!(mismatches.len(), 1, "{facing}");
    let m = &mismatches[0];
    assert_eq!(m["code"], "facing_mismatch");
    assert_eq!(m["entity"], "hero");
    assert!(m["dot"].as_f64().unwrap() < -0.99, "{m}");
    assert!(m["message"].as_str().unwrap().contains("model_yaw"), "{m}");
}

#[test]
fn observe_reports_each_characters_front_and_motion() {
    // Running forward (camera yaw 0 = -Z): the hero's front and motion agree.
    let w = load(&content().join("axiom-town"));
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    let run = ActionMap(BTreeMap::from([("move_z".into(), 1.0)]));
    for _ in 0..30 {
        r.step(&run);
    }
    let facing = r.observation().metrics["facing"].clone();
    let hero = &facing["characters"]["hero"];
    assert!(hero["speed"].as_f64().unwrap() > 2.0, "{hero}");
    assert!(hero["dot"].as_f64().unwrap() > 0.99, "{hero}");
    let front = hero["front"].as_array().unwrap();
    assert!(front[1].as_f64().unwrap() < -0.99, "{hero}");
    // A standing character has no alignment to judge.
    for _ in 0..30 {
        r.step(&ActionMap::default());
    }
    let idle = &r.observation().metrics["facing"]["characters"]["hero"];
    assert_eq!(idle["dot"], Value::Null, "{idle}");
}
