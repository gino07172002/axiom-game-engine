use axiom_core::{local::*, *};
use axiom_runtime::{
    world::{RuntimeRegistry, WorldRuntime},
    ActionMap,
};
use serde_json::{json, Value};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn world() -> World {
    ContentPackage::load(root().join("content/adv"), &Registry::core())
        .unwrap()
        .world
}
fn edit(target: Target, field: &str, value: Value) -> Edit {
    Edit::Core(CoreEdit::SetField {
        target,
        field: field.into(),
        value,
    })
}
fn select(store: &mut Store, q: Value) -> Value {
    store
        .execute(serde_json::from_value(json!({"op":"select","query":q})).unwrap())
        .unwrap()
}
#[test]
fn stable_choice_edit_survives_reordering_and_returns_precise_diff() {
    let mut w = world();
    w.resources.get_mut("narrative.graph").unwrap()["nodes"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let mut store = Store::memory_with(Registry::core(), w).unwrap();
    let request = vec![edit(
        Target::Choice {
            node: "workshop".into(),
            id: "open".into(),
        },
        "next",
        json!("stranded"),
    )];
    let p = store
        .execute(Request::Preview {
            expected_rev: Some(1),
            edits: request.clone(),
        })
        .unwrap();
    assert_eq!(p["diff"]["total_changes"], 1);
    assert_eq!(p["diff"]["resources"], json!(["narrative.graph"]));
    assert_eq!(
        p["diff"]["changes"][0]["path"],
        "/resources/narrative.graph/nodes/@workshop/choices/@open/next"
    );
    assert_eq!(store.revision().unwrap(), 1);
    store.apply(Some(1), request).unwrap();
    let s = select(
        &mut store,
        json!({"from":"choices","parent":"workshop","ids":["open"],"fields":["next"]}),
    );
    assert_eq!(s["items"][0]["data"]["next"], "stranded");
}
#[test]
fn selectors_are_small_resolve_prefabs_and_report_missing_ids() {
    let mut store = Store::memory_with(
        axiom_runtime::installed_registry().authoring,
        axiom_fps::demo_world().unwrap(),
    )
    .unwrap();
    let result = select(
        &mut store,
        json!({"from":"entities","components":["core.health"],"ids":["enemy/0","missing"],"fields":["id","/components/core.health"]}),
    );
    assert!(serde_json::to_vec(&result).unwrap().len() < 1024);
    assert_eq!(result["missing_ids"], json!(["missing"]));
    assert_eq!(result["total"], 1);
    let default = select(&mut store, json!({"from":"entities"}));
    assert!(serde_json::to_vec(&default).unwrap().len() < 8192);
    assert!(default["items"][0]["data"]["components"].is_array());
}
#[test]
fn paginated_selection_is_bound_to_revision_projection_and_budget() {
    let mut store = Store::memory_with(Registry::core(), world()).unwrap();
    let first = select(&mut store, json!({"from":"nodes","limit":2}));
    let cursor = first["next_cursor"].as_str().unwrap();
    let next = select(
        &mut store,
        json!({"from":"nodes","limit":2,"cursor":cursor}),
    );
    assert_ne!(first["items"], next["items"]);
    let changed: Request = serde_json::from_value(
        json!({"op":"select","query":{"from":"nodes","fields":["text"],"cursor":cursor}}),
    )
    .unwrap();
    assert!(store.execute(changed).is_err());
    store
        .apply(Some(1), vec![Edit::set("title", json!("changed"))])
        .unwrap();
    assert!(store
        .execute(
            serde_json::from_value(json!({"op":"select","query":{"from":"nodes","cursor":cursor}}))
                .unwrap()
        )
        .is_err());
}
#[test]
fn projections_enforce_bytes_and_do_not_silently_truncate_fields() {
    let mut store = Store::memory_with(Registry::core(), world()).unwrap();
    let q:Request=serde_json::from_value(json!({"op":"select","query":{"from":"resources","ids":["ui.document"],"fields":[""],"max_bytes":256}})).unwrap();
    assert!(store.execute(q).is_err());
    let result = select(
        &mut store,
        json!({"from":"choices","parent":"workshop","ids":["open"],"fields":["not_present"]}),
    );
    assert_eq!(result["items"][0]["missing_fields"], json!(["not_present"]));
}
#[test]
fn local_edits_can_create_remove_and_override_inherited_values() {
    let mut w = world();
    w.prefabs.insert(
        "flag".into(),
        Prefab {
            components: serde_json::from_value(
                json!({"core.health":{"value":10,"max":100},"core.transform":{"position":[0,0,0]}}),
            )
            .unwrap(),
        },
    );
    let mut e = Entity::new("inherited");
    e.prefab = Some("flag".into());
    w.entities.push(e);
    let mut store = Store::memory_with(Registry::core(), w).unwrap();
    store
        .apply(
            Some(1),
            vec![
                edit(
                    Target::Entity {
                        id: "inherited".into(),
                    },
                    "/components/core.health/value",
                    json!(20),
                ),
                Edit::Core(CoreEdit::PutObject {
                    target: Target::Variable {
                        id: "new_flag".into(),
                    },
                    value: json!(false),
                }),
            ],
        )
        .unwrap();
    assert_eq!(
        store
            .current()
            .unwrap()
            .1
            .compile(&store.registry)
            .unwrap()
            .entities
            .last()
            .unwrap()
            .health
            .as_ref()
            .unwrap()
            .max,
        100.0
    );
    store
        .apply(
            Some(2),
            vec![Edit::Core(CoreEdit::RemoveObject {
                target: Target::Variable {
                    id: "new_flag".into(),
                },
            })],
        )
        .unwrap();
    assert!(Target::Variable {
        id: "new_flag".into()
    }
    .get(&json!(store.current().unwrap().1))
    .is_err());
}
#[test]
fn diagnostics_identify_stable_choice_and_rule_fields_atomically() {
    let mut store = Store::memory_with(Registry::core(), world()).unwrap();
    for (target, field, value, suffix) in [
        (
            Target::Choice {
                node: "workshop".into(),
                id: "open".into(),
            },
            "next",
            json!("missing"),
            "/next",
        ),
        (
            Target::Rule {
                id: "remember-key".into(),
            },
            "actions",
            json!([{"kind":"set","variable":"missing","value":{"kind":"literal","value":"x"}}]),
            "/actions/0/variable",
        ),
    ] {
        let path = target.path();
        let e = store
            .apply(Some(1), vec![edit(target, field, value)])
            .unwrap_err();
        let d = error_response(&e);
        assert_eq!(d["diagnostics"][0]["path"], format!("{path}{suffix}"));
        assert_eq!(store.revision().unwrap(), 1);
    }
}
#[test]
fn behavior_query_and_cacheable_machine_schema_are_discoverable() {
    let mut store = Store::memory_with(Registry::core(), world()).unwrap();
    let b = store
        .execute(Request::Query {
            scope: Scope::Behavior,
            limit: None,
        })
        .unwrap();
    assert!(b["behaviors"]["behavior.program"].is_object());
    let s = store
        .execute(Request::Discover {
            topic: Some("local".into()),
            if_hash: None,
        })
        .unwrap();
    assert!(s["schema"]["$defs"]["target"]["oneOf"].is_array());
    let cached = store
        .execute(Request::Discover {
            topic: Some("local".into()),
            if_hash: Some(s["hash"].as_str().unwrap().into()),
        })
        .unwrap();
    assert_eq!(cached["not_modified"], true);
    assert!(cached.get("schema").is_none());
}
fn once(id: &str, request: Request) -> Request {
    Request::Execute {
        request_id: id.into(),
        runtime_session: None,
        request: Box::new(request),
    }
}
#[test]
fn durable_retries_replay_original_response_after_reopen_and_other_writers() {
    let path = std::env::temp_dir().join(format!("axiom-m15-once-{}.sqlite", std::process::id()));
    let mut store = Store::open_with(&path, Registry::core(), world()).unwrap();
    let request = once(
        "edit-1",
        Request::Apply {
            expected_rev: Some(1),
            edits: vec![Edit::set("title", json!("once"))],
        },
    );
    let original = store.execute(request.clone()).unwrap();
    assert_eq!(original["rev"], 2);
    drop(store);
    let mut store = Store::open_with(&path, Registry::core(), world()).unwrap();
    store
        .apply(Some(2), vec![Edit::set("title", json!("later"))])
        .unwrap();
    assert_eq!(store.execute(request).unwrap(), original);
    assert_eq!(store.revision().unwrap(), 3);
    assert!(store
        .execute(once(
            "edit-1",
            Request::Undo {
                rev: 1,
                expected_rev: None
            }
        ))
        .is_err());
    drop(store);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn failed_idempotent_transactions_do_not_consume_ids_or_leak_revisions() {
    let mut store = Store::memory_with(Registry::core(), world()).unwrap();
    let bad = once(
        "retry",
        Request::Apply {
            expected_rev: Some(8),
            edits: vec![Edit::set("title", json!("x"))],
        },
    );
    assert!(store.execute(bad).is_err());
    assert_eq!(store.revision().unwrap(), 1);
    let good = once(
        "retry",
        Request::Apply {
            expected_rev: Some(1),
            edits: vec![Edit::set("title", json!("x"))],
        },
    );
    store.execute(good.clone()).unwrap();
    store.execute(good).unwrap();
    assert_eq!(store.revision().unwrap(), 2);
    let undo = once(
        "undo",
        Request::Undo {
            rev: 1,
            expected_rev: Some(2),
        },
    );
    let first = store.execute(undo.clone()).unwrap();
    assert_eq!(store.execute(undo).unwrap(), first);
    assert_eq!(store.revision().unwrap(), 3);
}
#[test]
fn session_receipts_reject_conflicting_payloads_and_keep_original_results() {
    let mut c = retry::SessionReceipts::default();
    let command = Request::Save { slot: "a".into() };
    assert!(c.lookup("one", "session-a", &command).unwrap().is_none());
    c.insert(
        "one".into(),
        "session-a",
        &command,
        json!({"ok":true,"tick":10}),
    )
    .unwrap();
    assert_eq!(
        c.lookup("one", "session-a", &command).unwrap().unwrap()["tick"],
        10
    );
    assert!(c.lookup("one", "session-b", &command).is_err());
    assert!(c
        .lookup("one", "session-a", &Request::Load { slot: "a".into() })
        .is_err());
}

#[test]
fn local_field_override_keeps_unmodified_prefab_fields_inherited() {
    let mut world = World::empty("inheritance");
    world.prefabs.insert(
        "unit".into(),
        Prefab {
            components: serde_json::from_value(json!({"core.health":{"value":10,"max":100}}))
                .unwrap(),
        },
    );
    let mut e = Entity::new("unit/0");
    e.prefab = Some("unit".into());
    e.set("core.transform", json!({"scale":[1,1,1]})).unwrap();
    world.entities.push(e);
    let mut store = Store::memory_with(Registry::core(), world).unwrap();
    store
        .apply(
            Some(1),
            vec![
                edit(
                    Target::Entity {
                        id: "unit/0".into(),
                    },
                    "/components/core.health/value",
                    json!(20),
                ),
                edit(
                    Target::Entity {
                        id: "unit/0".into(),
                    },
                    "/components/core.transform/scale/0",
                    json!(2),
                ),
            ],
        )
        .unwrap();
    store
        .apply(
            Some(2),
            vec![edit(
                Target::Prefab { id: "unit".into() },
                "/components/core.health/max",
                json!(200),
            )],
        )
        .unwrap();
    let (_, world) = store.current().unwrap();
    assert_eq!(
        world.entities[0].components["core.health"],
        json!({"value":20})
    );
    assert_eq!(
        world.entities[0].components["core.transform"]["scale"],
        json!([2, 1, 1])
    );
    assert_eq!(
        world.compile(&store.registry).unwrap().entities[0]
            .health
            .as_ref()
            .unwrap()
            .max,
        200.0
    );
}
#[test]
fn observe_deltas_project_exact_values_and_unchanged_snapshots() {
    let mut h = observe::ObservationHistory::default();
    let mut raw = json!({"runtime_session":"session","runtime_epoch":1,"rev":1,"tick":0,"gameplay":{"node":"arrival","variables":{"key":false}}});
    let fields = vec!["/gameplay/variables".into()];
    let first = h.observe(&raw, None, &fields, 0, None).unwrap();
    let cursor = first["cursor"].as_str().unwrap();
    let same = h.observe(&raw, Some(cursor), &fields, 0, None).unwrap();
    assert_eq!(same["unchanged"], true);
    assert_eq!(same["cursor"], cursor);
    raw["gameplay"]["variables"]["key"] = json!(true);
    let next = h.observe(&raw, Some(cursor), &fields, 0, None).unwrap();
    assert_eq!(next["reset"], false);
    assert_eq!(
        next["changes"],
        json!([{"op":"set","path":"/gameplay/variables/key","value":true}])
    );
    assert!(next.get("snapshot").is_none());
}
#[test]
fn observe_resets_on_expiry_projection_and_checkpoint_epoch() {
    let mut h = observe::ObservationHistory::default();
    let mut raw = json!({"runtime_session":"s","runtime_epoch":1,"rev":1,"tick":0});
    let first = h.observe(&raw, None, &[], 0, None).unwrap();
    let cursor = first["cursor"].as_str().unwrap();
    for n in 1..70 {
        raw["tick"] = json!(n);
        h.observe(&raw, None, &[], 0, None).unwrap();
    }
    assert_eq!(
        h.observe(&raw, Some(cursor), &[], 0, None).unwrap()["reason"],
        "cursor_expired_or_foreign"
    );
    let recent = h.observe(&raw, None, &[], 0, None).unwrap();
    let cursor = recent["cursor"].as_str().unwrap();
    assert_eq!(
        h.observe(&raw, Some(cursor), &["/rev".into()], 0, None)
            .unwrap()["reason"],
        "projection_changed"
    );
    raw["runtime_epoch"] = json!(2);
    assert_eq!(
        h.observe(&raw, Some(cursor), &[], 0, None).unwrap()["reason"],
        "runtime_restarted"
    );
}
#[test]
fn observe_trace_gaps_are_explicit_and_arrays_are_atomic() {
    let mut h = observe::ObservationHistory::default();
    let mut raw = json!({"runtime_session":"s","runtime_epoch":1,"rev":1,"tick":0,"gameplay":{"choices":[{"id":"a"}],"trace":[{"seq":1,"kind":"node"}]}});
    let f = vec!["/gameplay/choices".into()];
    let first = h.observe(&raw, None, &f, 1, None).unwrap();
    raw["gameplay"]["choices"] = json!([{"id":"b"}]);
    raw["gameplay"]["trace"] = json!([{"seq":5},{"seq":6}]);
    let next = h
        .observe(&raw, first["cursor"].as_str(), &f, 1, None)
        .unwrap();
    assert_eq!(next["trace_gap"], true);
    assert_eq!(next["trace"], json!([{"seq":6}]));
    assert_eq!(next["changes"][0]["path"], "/gameplay/choices");
    assert_eq!(next["changes"][0]["value"], json!([{"id":"b"}]));
}
#[test]
fn load_keeps_retry_session_but_resets_observation_epoch_and_reads_m1_saves() {
    let mut r = WorldRuntime::new(world(), 1, RuntimeRegistry::core()).unwrap();
    let mut c = r.checkpoint().unwrap();
    c.engine_version = "0.3.0".into();
    let session = r.runtime_session.clone();
    let epoch = r.runtime_epoch;
    r.step(&ActionMap::default());
    r.restore_checkpoint(&c).unwrap();
    assert_eq!(r.runtime_session, session);
    assert_eq!(r.runtime_epoch, epoch + 1);
}
#[test]
fn protocol_benchmark_measures_transport_and_never_fabricates_tokens() {
    let suite =
        serde_json::from_str(&std::fs::read_to_string(root().join("benchmarks/m15.json")).unwrap())
            .unwrap();
    let report =
        axiom_runtime::benchmark::run(suite, &root(), axiom_runtime::installed_registry(), None)
            .unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["measurement"], "protocol_replay");
    assert!(report["summary"]["local"]["total_model_tokens"].is_null());
    assert!(
        report["summary"]["local"]["response_bytes"]
            .as_u64()
            .unwrap()
            < report["summary"]["legacy"]["response_bytes"]
                .as_u64()
                .unwrap()
    );
}

#[test]
#[cfg(windows)]
fn agent_adapter_runs_isolated_tasks_and_fixture_usage_is_not_model_usage() {
    let suite_path = root().join("benchmarks/m15.json");
    let suite = serde_json::from_str(&std::fs::read_to_string(&suite_path).unwrap()).unwrap();
    let options = axiom_runtime::benchmark::AgentOptions {
        executable: std::path::PathBuf::from("powershell.exe"),
        args: vec![
            "-NoProfile".into(),
            "-File".into(),
            root()
                .join("crates/axiom-runtime/tests/fixtures/agent.ps1")
                .to_string_lossy()
                .into_owned(),
            "-Suite".into(),
            suite_path.to_string_lossy().into_owned(),
        ],
        timeout: std::time::Duration::from_secs(15),
    };
    let report = axiom_runtime::benchmark::run(
        suite,
        &root(),
        axiom_runtime::installed_registry(),
        Some(options),
    )
    .unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["summary"]["agent"]["successes"], 2);
    assert!(report["summary"]["agent"]["total_model_tokens"].is_null());
    assert!(
        report["results"][0]["cost"]["agent_turns"]
            .as_u64()
            .unwrap()
            >= 5
    );
}

#[test]
#[cfg(windows)]
fn agent_timeout_is_a_failed_task_and_never_a_zero_token_success() {
    let mut suite: axiom_runtime::benchmark::Suite =
        serde_json::from_str(&std::fs::read_to_string(root().join("benchmarks/m15.json")).unwrap())
            .unwrap();
    suite.tasks.truncate(1);
    let options = axiom_runtime::benchmark::AgentOptions {
        executable: std::path::PathBuf::from("powershell.exe"),
        args: vec![
            "-NoProfile".into(),
            "-File".into(),
            root()
                .join("crates/axiom-runtime/tests/fixtures/stall.ps1")
                .to_string_lossy()
                .into_owned(),
        ],
        timeout: std::time::Duration::from_secs(1),
    };
    let started = std::time::Instant::now();
    let result = axiom_runtime::benchmark::run(
        suite,
        &root(),
        axiom_runtime::installed_registry(),
        Some(options),
    )
    .unwrap();
    assert!(started.elapsed().as_secs() < 10);
    assert_eq!(result["ok"], false);
    assert!(result["summary"]["agent"]["total_model_tokens"].is_null());
}
