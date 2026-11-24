use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::world::{ActionMap, WorldRuntime};
use serde_json::{json, Value};

fn lab() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/behavior-lab"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> Value {
    r.command(&RuntimeCommand::Call {
        capability: "fsm".into(),
        action: action.into(),
        args,
    })
    .unwrap()["result"]
        .clone()
}

fn run(r: &mut WorldRuntime, ticks: u32) {
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
}

#[test]
fn idle_enemy_explains_why_it_is_not_chasing_then_chases_when_in_range() {
    let mut r = WorldRuntime::new(lab(), 1, axiom_runtime::installed_registry()).unwrap();
    let before = call(&mut r, "inspect", json!({"id":"enemy/0"}));
    assert_eq!(before["state"], "idle");
    let chase = before["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["to"] == "chase")
        .unwrap();
    assert_eq!(chase["when"], "near(player)<=15");
    assert_eq!(chase["pass"], false);
    assert!(chase["measured"].as_f64().unwrap() > 29.0);
    // Player walks +x at 2 m/s from x=-20; enemy/0 sits at x=10 → in range after ~7.5 s.
    run(&mut r, 1200);
    let after = call(&mut r, "inspect", json!({"id":"enemy/0"}));
    assert_eq!(after["state"], "chase");
    assert_eq!(after["last"]["from"], "idle");
    let x = after["position"][0].as_f64().unwrap();
    assert!(
        x < 9.0,
        "a chasing enemy must have moved toward the player: x={x}"
    );
    run(&mut r, 1200);
    let summary = call(&mut r, "summary", json!({}));
    assert_eq!(summary["agents"], 6);
    assert_eq!(r.observation().metrics["fsm"]["agents"], 6);
}

#[test]
fn overlay_labels_follow_agents_and_project_into_the_game_view() {
    let mut r = WorldRuntime::new(lab(), 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 1200);
    let fsm = r.system::<axiom_runtime::fsm::FsmSystem>().unwrap();
    let labels = fsm.labels(&r.state);
    assert_eq!(labels.len(), 6);
    assert!(labels.iter().any(|(_, s)| s == "chase"));
    let frame = r.render_frame();
    let view = frame.views.iter().find(|v| v.camera.active).unwrap();
    let on_screen = labels
        .iter()
        .filter_map(|(p, _)| view.project(glam::Vec3::from_array(*p), [0.0, 0.0, 1280.0, 720.0]))
        .collect::<Vec<_>>();
    assert_eq!(
        on_screen.len(),
        6,
        "every agent is in front of the lab camera"
    );
    assert!(on_screen
        .iter()
        .all(|[x, y]| (0.0..=1280.0).contains(x) && (0.0..=720.0).contains(y)));
    // A point behind the camera never projects.
    assert!(view
        .project(view.eye - view.forward * 5.0, [0.0, 0.0, 1280.0, 720.0])
        .is_none());
}

fn skirmish() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/skirmish"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

#[test]
fn data_only_combat_reaches_victory_through_events_and_rules() {
    let mut r = WorldRuntime::new(skirmish(), 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 2400);
    let summary = call(&mut r, "summary", json!({}));
    assert_eq!(summary["states"]["dead"], 6, "{summary}");
    assert_eq!(summary["emitted"], 6);
    assert!(summary["fault"].is_null());
    let obs = json!(r.observation());
    assert_eq!(obs["gameplay"]["ending"], "victory", "{}", obs["gameplay"]);
    assert_eq!(obs["gameplay"]["variables"]["kills"], 6);
    let hero = r.state.compiled.indices["hero"];
    let hp = r.state.health[hero].as_ref().unwrap().value;
    assert!(
        hp < 100.0 && hp > 0.0,
        "grunts fight back but lose: hp={hp}"
    );
    // Dead grunts are no longer targets, so the hero stops attacking corpses.
    let hits = summary["hits"].as_u64().unwrap();
    run(&mut r, 600);
    assert_eq!(
        call(&mut r, "summary", json!({}))["hits"].as_u64().unwrap(),
        hits
    );
}

#[test]
fn stronger_grunts_defeat_the_hero_by_data_edit_alone() {
    let mut w = skirmish();
    w.resources.get_mut("fsm.machines").unwrap()["machines"]["grunt"]["states"]["attack"]
        ["action"]["damage"] = json!(40);
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 3000);
    let obs = json!(r.observation());
    assert_eq!(obs["gameplay"]["ending"], "defeat", "{}", obs["gameplay"]);
    assert_eq!(obs["gameplay"]["variables"]["hero_alive"], false);
}

#[test]
fn emits_and_attacks_are_validated_before_the_world_runs() {
    let registry = axiom_runtime::installed_registry();
    let mut undeclared = skirmish();
    undeclared.resources.get_mut("fsm.machines").unwrap()["machines"]["grunt"]["states"]["dead"]
        ["emit"] = json!("ghost_event");
    let error = WorldRuntime::new(undeclared, 1, registry.clone())
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("does not declare"),
        "{error:#}"
    );
    let mut bad = skirmish();
    bad.resources.get_mut("fsm.machines").unwrap()["machines"]["grunt"]["states"]["attack"]
        ["action"]["interval_ticks"] = json!(0);
    let error = format!("{:#}", bad.compile(&registry.authoring).unwrap_err());
    assert!(error.contains("interval_ticks"), "{error}");
}

#[test]
fn identical_runs_are_deterministic() {
    let snapshot = || {
        let mut r = WorldRuntime::new(lab(), 1, axiom_runtime::installed_registry()).unwrap();
        run(&mut r, 2000);
        (0..6)
            .map(|i| call(&mut r, "inspect", json!({"id":format!("enemy/{i}")})))
            .collect::<Vec<_>>()
    };
    assert_eq!(snapshot(), snapshot());
}

#[test]
fn hot_reload_keeps_agent_state_when_the_machine_survives() {
    let world = lab();
    let mut r = WorldRuntime::new(world.clone(), 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 1200);
    assert_eq!(
        call(&mut r, "inspect", json!({"id":"enemy/0"}))["state"],
        "chase"
    );
    let mut edited = world;
    edited.resources.get_mut("fsm.machines").unwrap()["machines"]["enemy.basic"]["states"]
        ["chase"]["action"]["speed"] = json!(4.0);
    r.reconcile(edited, 2).unwrap();
    let after = call(&mut r, "inspect", json!({"id":"enemy/0"}));
    assert_eq!(after["state"], "chase");
    assert_eq!(after["action"]["speed"], 4.0);
}

#[test]
fn invalid_machines_and_unknown_agents_fail_validation() {
    let registry = axiom_runtime::installed_registry();
    let mut bad = lab();
    bad.resources.get_mut("fsm.machines").unwrap()["machines"]["enemy.basic"]["transitions"][0]
        ["to"] = json!("ghost");
    let error = format!("{:#}", bad.compile(&registry.authoring).unwrap_err());
    assert!(error.contains("unknown state ghost"), "{error}");
    let mut orphan = lab();
    orphan
        .prefabs
        .get_mut("enemy.basic")
        .unwrap()
        .components
        .insert("fsm.agent".into(), json!({"machine":"missing"}));
    let error = WorldRuntime::new(orphan, 1, registry).err().unwrap();
    assert!(format!("{error:#}").contains("unknown fsm machine missing"));
}

#[test]
fn invalid_agent_references_fail_preview_and_apply_before_a_revision_is_written() {
    // R5 F2: the CLI and the editor both submit requests to the store, which compiles the
    // edited world; the fsm world validator must reject what fsm.step could not start.
    use axiom_core::{Edit, Request, Store};
    let registry = axiom_runtime::installed_registry();
    let mut store = Store::memory_with(registry.authoring.clone(), lab()).unwrap();
    let rev = store.revision().unwrap();
    let bad = Edit::set(
        "/prefabs/enemy.basic/components/fsm.agent/machine",
        json!("missing"),
    );
    let preview = store.execute(Request::Preview {
        expected_rev: Some(rev),
        edits: vec![bad.clone()],
    });
    let error = format!("{:#}", preview.unwrap_err());
    assert!(error.contains("unknown fsm machine missing"), "{error}");
    assert!(store.apply(Some(rev), vec![bad]).is_err());
    assert_eq!(store.revision().unwrap(), rev);
    // An agent without core.transform and an undeclared emit fail compile the same way.
    let mut no_transform = lab();
    let agent = no_transform
        .entities
        .iter()
        .position(|e| {
            no_transform
                .resolved_components(e)
                .contains_key("fsm.agent")
        })
        .unwrap();
    no_transform.entities[agent]
        .components
        .insert("fsm.agent".into(), json!({"machine":"enemy.basic"}));
    no_transform.entities[agent].prefab = None;
    no_transform.entities[agent]
        .components
        .remove("core.transform");
    let error = format!("{:#}", {
        no_transform.compile(&registry.authoring).unwrap();
        registry
            .authoring
            .validate_write(&no_transform, None)
            .unwrap_err()
    });
    assert!(error.contains("needs core.transform"), "{error}");
    let mut undeclared = skirmish();
    undeclared.resources.get_mut("fsm.machines").unwrap()["machines"]["grunt"]["states"]["dead"]
        ["emit"] = json!("ghost_event");
    let error = format!(
        "{:#}",
        registry
            .authoring
            .validate_write(&undeclared, None)
            .unwrap_err()
    );
    assert!(error.contains("does not declare"), "{error}");
}

#[test]
fn defeated_attacker_deals_no_damage() {
    // R5 F6: a grunt with 0 health stuck in `attack` next to the hero must not hit.
    let mut w = skirmish();
    let hero = w.entities.iter().position(|e| e.id == "hero").unwrap();
    w.entities[hero].components.remove("fsm.agent");
    let grunt = &mut w.resources.get_mut("fsm.machines").unwrap()["machines"]["grunt"];
    grunt["initial"] = json!("attack");
    grunt["transitions"] = json!([]);
    let e = w.entities.iter_mut().find(|e| e.id == "enemy/0").unwrap();
    e.components
        .insert("core.health".into(), json!({"value":0.0,"max":60.0}));
    e.components
        .insert("core.transform".into(), json!({"position":[-11.0,0.5,0.0]}));
    let mut r = WorldRuntime::new(w.clone(), 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 300);
    let hero_hp = |r: &WorldRuntime| {
        r.state.health[r.state.compiled.indices["hero"]]
            .as_ref()
            .unwrap()
            .value
    };
    assert_eq!(hero_hp(&r), 100.0);
    assert_eq!(call(&mut r, "summary", json!({}))["hits"], 0);
    // Control: the same grunt alive does hit.
    w.entities
        .iter_mut()
        .find(|e| e.id == "enemy/0")
        .unwrap()
        .components
        .insert("core.health".into(), json!({"value":60.0,"max":60.0}));
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 300);
    assert!(hero_hp(&r) < 100.0);
}

#[test]
fn public_schema_lists_the_editor_kinds_and_state_fields() {
    // R5 F15: one source for the editor forms and `schema fsm.machines`.
    let registry = axiom_runtime::installed_registry();
    let schema = registry.authoring.resources["fsm.machines"].schema["machines"]
        .as_str()
        .unwrap()
        .to_owned();
    for condition in [false, true] {
        for (kind, _, _) in axiom_runtime::fsm::editor_kinds(condition) {
            assert!(schema.contains(kind), "{kind} missing from {schema}");
        }
    }
    for field in ["clip", "clip_loop", "emit", "radius", "stop_within"] {
        let agent = registry.authoring.components["fsm.agent"]
            .schema
            .to_string();
        assert!(
            schema.contains(field) || agent.contains(field),
            "{field} missing"
        );
    }
}

#[test]
fn checkpoints_fail_explicitly_for_fsm_worlds() {
    let mut w = lab();
    w.capabilities.insert("save".into(), 1);
    let r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    let error = format!("{:#}", r.checkpoint().err().unwrap());
    assert!(
        error.contains("fsm.step does not implement checkpoints"),
        "{error}"
    );
}

#[test]
fn fps_inspect_reports_sentinel_state_and_transition_verdicts() {
    let w = axiom_fps::demo_world().unwrap();
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    let id = r.state.compiled.entities[1].id.clone();
    let out = r
        .command(&RuntimeCommand::Call {
            capability: "fps".into(),
            action: "inspect".into(),
            args: json!({ "id": id }),
        })
        .unwrap();
    let result = &out["result"];
    assert_eq!(result["id"], json!(id));
    assert!(result["distance_to_player"].as_f64().unwrap() > 0.0);
    assert!(result["transitions"].as_array().is_some());
    let missing = r.command(&RuntimeCommand::Call {
        capability: "fps".into(),
        action: "inspect".into(),
        args: json!({"id":"nobody"}),
    });
    assert!(format!("{:#}", missing.unwrap_err()).contains("not a sentinel"));
}

/// One FSM rule added after content was stored: how an older build stored a violation, the
/// pointer and value of a repair, and what a new write of the violation reports.
struct Legacy {
    name: &'static str,
    break_world: fn(&mut World),
    repair: (&'static str, fn(&World) -> Value),
    error: &'static str,
}

fn legacy_cases() -> Vec<Legacy> {
    vec![
        Legacy {
            name: "unknown-machine",
            break_world: |w| {
                w.prefabs
                    .get_mut("enemy.basic")
                    .unwrap()
                    .components
                    .insert("fsm.agent".into(), json!({"machine":"ghost"}));
            },
            repair: ("/prefabs/enemy.basic/components/fsm.agent", |w| {
                w.prefabs["enemy.basic"].components["fsm.agent"].clone()
            }),
            error: "unknown fsm machine ghost",
        },
        Legacy {
            name: "undeclared-emit",
            break_world: |w| {
                w.resources.get_mut("fsm.machines").unwrap()["machines"]["enemy.basic"]["states"]
                    ["chase"]["emit"] = json!("ghost_event");
            },
            repair: ("/resources/fsm.machines", |w| {
                w.resources["fsm.machines"].clone()
            }),
            error: "ghost_event",
        },
        Legacy {
            name: "radius",
            break_world: |w| {
                w.prefabs
                    .get_mut("enemy.basic")
                    .unwrap()
                    .components
                    .get_mut("fsm.agent")
                    .unwrap()["radius"] = json!(9.0);
            },
            repair: ("/prefabs/enemy.basic/components/fsm.agent", |w| {
                w.prefabs["enemy.basic"].components["fsm.agent"].clone()
            }),
            error: "radius must be 0.05..5",
        },
    ]
}

#[test]
fn fsm_worlds_stored_before_the_fsm_rules_open_repair_and_undo() {
    use axiom_core::{Edit, Request, Store};
    let registry = axiom_runtime::installed_registry().authoring;
    for case in legacy_cases() {
        let dir = std::env::temp_dir().join(format!(
            "axiom-fsm-legacy-{}-{}",
            case.name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("world.sqlite");
        let open = || Store::open_with(&path, registry.clone(), lab());
        drop(open().unwrap());
        let mut legacy = lab();
        (case.break_world)(&mut legacy);
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "INSERT INTO revisions(world,reason) VALUES (?1,'legacy')",
                [json!(legacy).to_string()],
            )
            .unwrap();
        let name = case.name;
        let mut s = open().unwrap_or_else(|e| panic!("{name}: {e:#}"));
        let verify = s.execute(Request::Verify {}).unwrap();
        assert!(
            verify["ok"] == true && verify.to_string().contains(case.error),
            "{name}: the old violation is reported, not enforced: {verify}"
        );
        // An unrelated edit commits on the old world; the repair commits too.
        let unrelated = s.apply(Some(2), vec![Edit::set("title", json!("Renamed"))]);
        assert_eq!(unrelated.unwrap()["rev"], 3, "{name}");
        let (pointer, value) = case.repair;
        let repair = Edit::set(pointer, value(&lab()));
        assert_eq!(s.apply(Some(3), vec![repair]).unwrap()["rev"], 4, "{name}");
        // Writing the violation again is refused once the world is clean.
        let again = Edit::set(pointer, (case.repair.1)(&legacy));
        let e = format!("{:#}", s.apply(Some(4), vec![again]).unwrap_err());
        assert!(e.contains(case.error), "{name}: {e}");
        let undone = s
            .execute(Request::Undo {
                rev: 2,
                expected_rev: Some(4),
            })
            .unwrap();
        assert_eq!(undone["rev"], 5, "{name}: {undone}");
        drop(s);
        let reopened = open().unwrap();
        assert_eq!(reopened.current().unwrap().1, legacy, "{name}");
        drop(reopened);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Final round C0 R5-F2: a new prefab no entity instantiates yet is checked too, so an
/// out-of-range radius is refused when it is written, not when it is first instantiated.
#[test]
fn an_uninstantiated_prefab_with_an_out_of_range_radius_is_refused_on_write() {
    use axiom_core::{CoreEdit, Edit, Request, Store};
    let registry = axiom_runtime::installed_registry();
    let mut store = Store::memory_with(registry.authoring.clone(), lab()).unwrap();
    let rev = store.revision().unwrap();
    let prefab = |radius: f64| {
        Edit::Core(CoreEdit::PutPrefab {
            id: "enemy.unused".into(),
            prefab: serde_json::from_value(
                json!({"components":{"fsm.agent":{"machine":"enemy.basic","radius":radius}}}),
            )
            .unwrap(),
        })
    };
    let error = store
        .execute(Request::Apply {
            expected_rev: Some(rev),
            edits: vec![prefab(9.0)],
        })
        .unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("radius must be 0.05..5"), "{error}");
    assert_eq!(store.revision().unwrap(), rev, "nothing committed");
    let applied = store
        .execute(Request::Apply {
            expected_rev: Some(rev),
            edits: vec![prefab(0.5)],
        })
        .unwrap();
    assert_eq!(applied["rev"], rev + 1, "{applied}");
}
