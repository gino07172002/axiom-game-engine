use axiom_core::{gameplay::*, *};
use axiom_runtime::{world::RuntimeRegistry, ActionMap, WorldRuntime};
use serde_json::{json, Value};
use std::collections::BTreeMap;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn world() -> World {
    ContentPackage::load(root().join("content/adv"), &Registry::core())
        .unwrap()
        .world
}
fn runtime() -> WorldRuntime {
    WorldRuntime::new(world(), 1, RuntimeRegistry::core()).unwrap()
}
fn choose(r: &mut WorldRuntime, node: &str, choice: &str) {
    r.command(&RuntimeCommand::Choose {
        node: node.into(),
        choice: choice.into(),
    })
    .unwrap();
}
fn obs(r: &WorldRuntime) -> Value {
    json!(r.observation())["gameplay"].clone()
}
fn key(r: &mut WorldRuntime) {
    choose(r, "arrival", "help");
    choose(r, "workshop", "inspect");
    choose(r, "clue", "return");
}
#[test]
fn all_three_endings_and_save_replay_scenarios_pass() {
    for name in ["rescue", "departure", "stranded"] {
        let s = serde_json::from_str(
            &std::fs::read_to_string(root().join(format!("examples/adv-{name}.scenario.json")))
                .unwrap(),
        )
        .unwrap();
        let result = axiom_runtime::scenario::run(world(), 1, RuntimeRegistry::core(), s).unwrap();
        assert_eq!(result["observation"]["gameplay"]["ending"], name);
    }
}
#[test]
fn key_gates_choices_and_stale_or_hidden_commands_are_recoverable() {
    let mut r = runtime();
    choose(&mut r, "arrival", "help");
    let before = obs(&r);
    assert!(r
        .command(&RuntimeCommand::Choose {
            node: "workshop".into(),
            choice: "open".into()
        })
        .is_err());
    assert_eq!(obs(&r), before);
    assert!(r
        .command(&RuntimeCommand::Choose {
            node: "arrival".into(),
            choice: "help".into()
        })
        .is_err());
    choose(&mut r, "workshop", "inspect");
    choose(&mut r, "clue", "return");
    assert!(r
        .state
        .gameplay
        .as_ref()
        .unwrap()
        .choices()
        .iter()
        .any(|c| c.id == "open"));
    choose(&mut r, "workshop", "open");
    assert_eq!(obs(&r)["ending"], "rescue");
    assert!(!json!(r.observation())
        .as_object()
        .unwrap()
        .contains_key("player_health"));
}
#[test]
fn scene_activation_reaches_render_extraction() {
    let mut r = runtime();
    let before = r.render_frame().instances.len();
    assert!(r.state.compiled.enabled[r.state.compiled.indices["platform"]]);
    choose(&mut r, "arrival", "help");
    assert!(!r.state.compiled.enabled[r.state.compiled.indices["platform"]]);
    assert!(r.state.compiled.enabled[r.state.compiled.indices["workbench"]]);
    assert_eq!(r.render_frame().views.len(), 1);
    assert_eq!(r.render_frame().instances.len(), before);
}
#[test]
fn typed_payloads_and_overflow_are_atomic() {
    let mut w = World::empty("counter");
    w.capabilities.insert("behavior".into(), 1);
    w.systems.push("behavior.execute".into());
    w.resources.insert("behavior.program".into(),json!({"variables":{"score":9223372036854775807i64},"events":{"gain":{"fields":{"amount":"int"}}},"rules":[{"id":"gain","event":"gain","actions":[{"kind":"add","variable":"score","value":{"kind":"event","field":"amount"}}]}]}));
    let mut r = WorldRuntime::new(w, 1, RuntimeRegistry::core()).unwrap();
    let before = obs(&r);
    for amount in [Scalar::Text("bad".into()), Scalar::Int(1)] {
        assert!(r
            .command(&RuntimeCommand::Emit {
                event: "gain".into(),
                fields: BTreeMap::from([("amount".into(), amount)])
            })
            .is_err());
        assert_eq!(obs(&r), before);
    }
}
#[test]
fn recursive_events_stop_at_budget_without_partial_writes() {
    let mut w = world();
    let program = w.resources.get_mut("behavior.program").unwrap();
    program["rules"] = json!([{"id":"loop","event":"inspect","actions":[{"kind":"set","variable":"has_key","value":{"kind":"literal","value":true}},{"kind":"emit","event":"inspect"}]}]);
    let mut r = WorldRuntime::new(w, 1, RuntimeRegistry::core()).unwrap();
    let before = obs(&r);
    let err = r
        .command(&RuntimeCommand::Emit {
            event: "inspect".into(),
            fields: BTreeMap::new(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("budget"));
    assert_eq!(obs(&r), before);
    r.assert_valid().unwrap();
}
#[test]
fn native_inputs_and_narrative_use_same_rules_once() {
    let mut r = runtime();
    choose(&mut r, "arrival", "help");
    let mut input = ActionMap::default();
    input.0.insert("inspect".into(), 1.0);
    for _ in 0..12 {
        r.step(&input);
    }
    assert_eq!(obs(&r)["variables"]["clues"], 1);
    assert_eq!(obs(&r)["variables"]["chapter"], "brass_key");
    assert_eq!(obs(&r)["variables"]["signal_received"], true);
    assert_eq!(
        obs(&r)["trace"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["kind"] == "event" && t["id"] == "inspect")
            .count(),
        1
    );
}
#[test]
fn ui_bindings_localize_and_update_after_rules() {
    let mut r = runtime();
    assert!(obs(&r)["bindings"]["dialogue"]
        .as_str()
        .unwrap()
        .contains("岑"));
    r.command(&RuntimeCommand::Locale {
        locale: "en".into(),
    })
    .unwrap();
    assert!(obs(&r)["bindings"]["dialogue"]
        .as_str()
        .unwrap()
        .contains("Cen"));
    key(&mut r);
    assert_eq!(obs(&r)["bindings"]["chapter"], "Collected: Brass key");
    assert_eq!(obs(&r)["bindings"]["progress"], 1.0);
    assert_eq!(r.state.gameplay.as_ref().unwrap().images.len(), 1);
    let ctx = egui::Context::default();
    axiom_runtime::ui::configure(&ctx);
    let mut ui = axiom_runtime::ui::UiState {
        playing: true,
        ..Default::default()
    };
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        r.draw_ui(ctx, &mut ui);
    });
    assert_eq!(ui.textures.len(), 1);
}
#[test]
fn hot_reload_preserves_progress_and_applies_data_only_condition_edits() {
    let mut store = Store::memory_with(Registry::core(), world()).unwrap();
    let mut r = runtime();
    key(&mut r);
    let request = parse_request(
        &std::fs::read_to_string(root().join("examples/adv-branch-edit.json")).unwrap(),
    )
    .unwrap();
    let preview = match request.clone() {
        Request::Apply {
            expected_rev,
            edits,
        } => Request::Preview {
            expected_rev,
            edits,
        },
        _ => unreachable!(),
    };
    store.execute(preview).unwrap();
    assert_eq!(store.revision().unwrap(), 1);
    store.execute(request).unwrap();
    let (rev, w) = store.current().unwrap();
    r.reconcile(w, rev).unwrap();
    assert_eq!(obs(&r)["variables"]["has_key"], true);
    choose(&mut r, "workshop", "open");
    assert_eq!(obs(&r)["ending"], "rescue");
}
#[test]
fn removing_live_node_retains_last_good_runtime() {
    let mut r = runtime();
    choose(&mut r, "arrival", "help");
    let before = obs(&r);
    let mut w = world();
    w.resources.get_mut("narrative.graph").unwrap()["nodes"][1]["id"] = json!("new-workshop");
    w.resources.get_mut("narrative.graph").unwrap()["nodes"][0]["choices"][0]["next"] =
        json!("new-workshop");
    w.resources.get_mut("narrative.graph").unwrap()["nodes"][2]["choices"][0]["next"] =
        json!("new-workshop");
    assert!(r.reconcile(w, 2).is_err());
    assert_eq!(r.rev, 1);
    assert_eq!(r.requested_rev, 2);
    assert_eq!(obs(&r), before);
}
#[test]
fn checkpoints_restore_flags_timers_node_scene_and_do_not_reenter() {
    let mut r = runtime();
    key(&mut r);
    for _ in 0..2 {
        r.step(&ActionMap::default());
    }
    let c = r.checkpoint().unwrap();
    choose(&mut r, "workshop", "open");
    let mut fresh = WorldRuntime::new(world(), 19, RuntimeRegistry::core()).unwrap();
    fresh.restore_checkpoint(&c).unwrap();
    assert_eq!(fresh.rev, 19);
    assert_eq!(fresh.state.tick, 2);
    assert_eq!(
        fresh.state.gameplay.as_ref().unwrap().progress(),
        c.gameplay.clone().unwrap()
    );
    for _ in 0..4 {
        fresh.step(&ActionMap::default());
    }
    assert_eq!(obs(&fresh)["variables"]["signal_received"], true);
    assert_eq!(obs(&fresh)["variables"]["clues"], 1);
}
#[test]
fn checkpoint_file_roundtrip_and_corruption_leave_state_intact() {
    let dir = std::env::temp_dir().join(format!("axiom-m1-save-{}", std::process::id()));
    let mut r = runtime();
    key(&mut r);
    r.save_slot(&dir, "test").unwrap();
    choose(&mut r, "workshop", "open");
    r.load_slot(&dir, "test").unwrap();
    assert_eq!(obs(&r)["node"], "workshop");
    for slot in ["../escape", "x/y", "CON.txt", ""] {
        assert!(r.save_slot(&dir, slot).is_err());
    }
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("slot-test.json");
    std::fs::write(&path, b"{broken").unwrap();
    let before = obs(&r);
    assert!(r.load_slot(&dir, "test").is_err());
    assert_eq!(obs(&r), before);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn incompatible_or_malformed_checkpoints_fail_atomically() {
    let mut r = runtime();
    key(&mut r);
    let c = r.checkpoint().unwrap();
    let before = obs(&r);
    let mut variants = vec![];
    let mut bad = c.clone();
    bad.save_version = 9;
    variants.push(bad);
    let mut bad = c.clone();
    bad.content.title = "other".into();
    variants.push(bad);
    let mut bad = c.clone();
    bad.gameplay
        .as_mut()
        .unwrap()
        .variables
        .insert("has_key".into(), Scalar::Text("false".into()));
    variants.push(bad);
    let mut bad = c.clone();
    bad.gameplay.as_mut().unwrap().node = Some("missing".into());
    variants.push(bad);
    let mut bad = c.clone();
    bad.entities
        .get_mut("camera")
        .unwrap()
        .transform
        .as_mut()
        .unwrap()
        .scale = [0.0; 3];
    variants.push(bad);
    for bad in variants {
        assert!(r.restore_checkpoint(&bad).is_err());
        assert_eq!(obs(&r), before);
    }
}
#[test]
fn checkpoint_restores_typed_columns_and_core_motion() {
    let mut w = World::empty("generic-save");
    w.capabilities
        .extend([("save".into(), 1), ("motion".into(), 1)]);
    w.systems.push("motion.integrate".into());
    w.component_schemas.insert(
        "game.flags".into(),
        serde_json::from_value(json!({"fields":{"flag":{"ty":"bool"},"target":{"ty":"entity"}}}))
            .unwrap(),
    );
    let mut e = Entity::new("box");
    e.set("core.transform", Transform::default()).unwrap();
    e.set("core.motion", json!({"velocity":[1,0,0]})).unwrap();
    e.set("game.flags", json!({"flag":false,"target":"box"}))
        .unwrap();
    w.entities.push(e);
    let mut r = WorldRuntime::new(w, 1, RuntimeRegistry::core()).unwrap();
    if let Column::Bool(v) = r
        .state
        .columns
        .get_mut("game.flags")
        .unwrap()
        .fields
        .get_mut("flag")
        .unwrap()
    {
        v[0] = Some(true);
    }
    r.step(&ActionMap::default());
    let checkpoint = r.checkpoint().unwrap();
    r.step(&ActionMap::default());
    r.restore_checkpoint(&checkpoint).unwrap();
    assert_eq!(r.state.tick, 1);
    assert!(
        matches!(&r.state.columns["game.flags"].fields["flag"],Column::Bool(v) if v[0]==Some(true))
    );
    r.step(&ActionMap::default());
    assert!((r.state.transforms[0].as_ref().unwrap().position[0] - 2.0 / 120.0).abs() < 0.0001);
}
#[test]
fn reachability_proves_three_endings_and_reports_exhausted_budget() {
    let r = axiom_runtime::narrative::analyze(&world(), 4096).unwrap();
    assert_eq!(r["complete"], true);
    assert_eq!(r["reachable_endings"].as_object().unwrap().len(), 3);
    let short = axiom_runtime::narrative::analyze(&world(), 1).unwrap();
    assert_eq!(short["complete"], false);
}
#[test]
fn invalid_world_references_types_and_bindings_fail_before_commit() {
    let mutations = [
        (
            "/resources/narrative.graph/nodes/0/choices/0/next",
            json!("missing"),
        ),
        (
            "/resources/behavior.program/rules/0/event",
            json!("missing"),
        ),
        (
            "/resources/behavior.program/rules/0/actions/0/variable",
            json!("missing"),
        ),
        (
            "/resources/behavior.program/rules/0/actions/0/value/value",
            json!(5),
        ),
        (
            "/resources/narrative.graph/nodes/1/on_enter/0/scene",
            json!("missing"),
        ),
        (
            "/resources/ui.document/widgets/5/variable",
            json!("missing"),
        ),
        ("/resources/ui.document/locale", json!("missing")),
        ("/resources/ui.document/catalogs/en", json!({})),
    ];
    let mut store = Store::memory_with(Registry::core(), world()).unwrap();
    for (path, value) in mutations {
        let e = store
            .apply(Some(1), vec![Edit::set(path, value)])
            .unwrap_err();
        assert!(!e.to_string().is_empty());
        assert_eq!(store.revision().unwrap(), 1);
    }
}
#[test]
fn fps_still_uses_its_registered_system_and_rejects_unsupported_save() {
    let w = axiom_fps::demo_world().unwrap();
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    for _ in 0..120 {
        r.step(&ActionMap::default());
    }
    r.assert_valid().unwrap();
    assert_eq!(r.observation().metrics["enemies_total"], 8);
    assert!(r.state.gameplay.is_none());
    assert!(r.checkpoint().is_err());
}

#[test]
fn actual_egui_choice_click_produces_the_shared_typed_command() {
    let mut r = runtime();
    let ctx = egui::Context::default();
    axiom_runtime::ui::configure(&ctx);
    let mut ui = axiom_runtime::ui::UiState {
        playing: true,
        ..Default::default()
    };
    let mut actions = vec![];
    for _ in 0..2 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| {
                actions = r.draw_ui(ctx, &mut ui);
            },
        );
    }
    let pos = ui.targets["choice/help"].center();
    for pressed in [true, false] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 900.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                actions = r.draw_ui(ctx, &mut ui);
            },
        );
    }
    let command = actions
        .into_iter()
        .find_map(|a| {
            if let axiom_runtime::ui::Action::Gameplay(c) = a {
                Some(c)
            } else {
                None
            }
        })
        .expect("choice click command");
    assert_eq!(
        command,
        RuntimeCommand::Choose {
            node: "arrival".into(),
            choice: "help".into()
        }
    );
    r.command(&command).unwrap();
    assert_eq!(obs(&r)["node"], "workshop");
}

#[test]
fn gameplay_and_fps_compose_without_genre_branch() {
    let mut w = axiom_fps::demo_world().unwrap();
    w.capabilities.insert("behavior".into(), 1);
    w.systems.push("behavior.execute".into());
    w.resources.insert("behavior.program".into(),json!({"variables":{"quest_done":false},"events":{"quest":{"fields":{}}},"rules":[{"id":"quest","event":"quest","actions":[{"kind":"set","variable":"quest_done","value":{"kind":"literal","value":true}}]}]}));
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    r.command(&RuntimeCommand::Emit {
        event: "quest".into(),
        fields: BTreeMap::new(),
    })
    .unwrap();
    r.step(&ActionMap::default());
    assert_eq!(obs(&r)["variables"]["quest_done"], true);
    assert_eq!(r.observation().metrics["enemies_total"], 8);
}

#[test]
fn bounded_node_entry_cycles_reject_activation() {
    let mut w = world();
    w.resources.get_mut("narrative.graph").unwrap()["nodes"][0]["on_enter"] =
        json!([{"kind":"go_to","node":"arrival"}]);
    assert!(WorldRuntime::new(w, 1, RuntimeRegistry::core())
        .err()
        .unwrap()
        .to_string()
        .contains("budget"));
}

#[test]
fn ui_button_events_and_common_end_action_work_without_narrative() {
    let mut w = World::empty("button");
    w.capabilities
        .extend([("behavior".into(), 1), ("ui".into(), 1), ("save".into(), 1)]);
    w.systems.push("behavior.execute".into());
    w.resources.insert("behavior.program".into(),json!({"variables":{"done":false},"events":{"finish":{"fields":{}}},"rules":[{"id":"finish","event":"finish","actions":[{"kind":"set","variable":"done","value":{"kind":"literal","value":true}},{"kind":"end","ending":"complete"}]}]}));
    w.resources.insert("ui.document".into(),json!({"locale":"en","catalogs":{"en":{"finish":"Finish"}},"widgets":[{"kind":"button","id":"finish","text":"finish","command":{"kind":"emit","event":"finish"},"when":{"kind":"not","condition":{"kind":"eq","left":{"kind":"variable","id":"done"},"right":{"kind":"literal","value":true}}}}]}));
    let mut r = WorldRuntime::new(w, 1, RuntimeRegistry::core()).unwrap();
    r.command(&RuntimeCommand::Emit {
        event: "finish".into(),
        fields: BTreeMap::new(),
    })
    .unwrap();
    assert_eq!(obs(&r)["ending"], "complete");
    let c = r.checkpoint().unwrap();
    r.restore_checkpoint(&c).unwrap();
    assert_eq!(obs(&r)["variables"]["done"], true);
}

#[test]
fn overwritten_save_keeps_a_recoverable_backup() {
    let dir = std::env::temp_dir().join(format!("axiom-m1-backup-{}", std::process::id()));
    let mut r = runtime();
    r.save_slot(&dir, "backup").unwrap();
    key(&mut r);
    r.save_slot(&dir, "backup").unwrap();
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("slot-backup.json");
    std::fs::remove_file(path).unwrap();
    r.load_slot(&dir, "backup").unwrap();
    assert_eq!(obs(&r)["node"], "arrival");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_assets_fail_activation_and_package_paths_cannot_escape() {
    let mut r = runtime();
    let mut w = world();
    w.resources.get_mut("ui.document").unwrap()["images"]["station"] = json!("missing-image.png");
    assert!(r.reconcile(w, 2).is_err());
    assert_eq!(r.rev, 1);
    assert_eq!(obs(&r)["node"], "arrival");
    let dir = std::env::temp_dir().join(format!("axiom-m1-pack-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        std::fs::read(root().join("content/adv/manifest.json")).unwrap(),
    )
    .unwrap();
    let mut source: Value =
        serde_json::from_slice(&std::fs::read(root().join("content/adv/world.json")).unwrap())
            .unwrap();
    source["resources"]["ui.document"]["images"]["station"] =
        json!(root().join("content/adv/assets/station.png"));
    std::fs::write(dir.join("world.json"), serde_json::to_vec(&source).unwrap()).unwrap();
    assert!(ContentPackage::load(&dir, &Registry::core())
        .err()
        .unwrap()
        .to_string()
        .contains("escapes"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn newly_activated_gameplay_rebases_initial_timers_and_node_end_saves() {
    let mut r = WorldRuntime::new(World::empty("empty"), 1, RuntimeRegistry::core()).unwrap();
    for _ in 0..100 {
        r.step(&ActionMap::default());
    }
    let mut w = world();
    w.resources.get_mut("narrative.graph").unwrap()["nodes"][0]["on_enter"] =
        json!([{"kind":"delay","ticks":6,"event":"signal"},{"kind":"end","ending":"intro"}]);
    r.reconcile(w, 2).unwrap();
    let c = r.checkpoint().unwrap();
    assert_eq!(c.gameplay.as_ref().unwrap().pending[0].due, 106);
    r.restore_checkpoint(&c).unwrap();
    assert_eq!(obs(&r)["ending"], "intro");
    for _ in 0..5 {
        r.step(&ActionMap::default());
    }
    assert_eq!(obs(&r)["variables"]["signal_received"], false);
    r.step(&ActionMap::default());
    assert_eq!(obs(&r)["variables"]["signal_received"], true);
}
