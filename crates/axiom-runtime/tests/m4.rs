use axiom_core::{
    gameplay::{RuntimeCommand, Scalar},
    ContentPackage, World,
};
use axiom_runtime::{installed_registry, scenario, ActionMap, WorldRuntime};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn world() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/m4"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}
fn runtime() -> WorldRuntime {
    WorldRuntime::new(world(), 1, installed_registry()).unwrap()
}
fn call(r: &mut WorldRuntime, cap: &str, action: &str, args: Value) -> Value {
    r.command(&RuntimeCommand::Call {
        capability: cap.into(),
        action: action.into(),
        args,
    })
    .unwrap()
}
fn choose(r: &mut WorldRuntime, node: &str, choice: &str) {
    r.command(&RuntimeCommand::Choose {
        node: node.into(),
        choice: choice.into(),
    })
    .unwrap();
}
fn steps(r: &mut WorldRuntime, n: u32) {
    for _ in 0..n {
        r.step(&ActionMap::default());
        r.assert_valid().unwrap();
    }
}
fn gather(r: &mut WorldRuntime) {
    call(
        r,
        "rts",
        "gather",
        json!({"units":["squad/worker/0","squad/worker/1","squad/worker/2","squad/worker/3"],"target":"squad/ore"}),
    );
}
#[test]
fn joint_scenario_completes_from_real_mining_production_and_melee() {
    let scenario =
        serde_json::from_str(include_str!("../../../examples/m4-joint.scenario.json")).unwrap();
    let result = scenario::run(world(), 1, installed_registry(), scenario).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(
        result["observation"]["packages"]["packages"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(result["observation"]["events"]["pending"], 0);
}
#[test]
fn input_contexts_route_controls_pointers_and_cameras_without_pausing_simulation() {
    let mut r = runtime();
    for (kind, point) in [
        (axiom_runtime::world::PointerKind::PrimaryDown, [0.1, 0.1]),
        (axiom_runtime::world::PointerKind::PrimaryUp, [0.9, 0.9]),
    ] {
        r.screen_pointer(kind, point, 1.6, false).unwrap();
    }
    assert_eq!(
        r.observation().metrics["rts"]["selected"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    call(&mut r, "rts", "select", json!({"ids":[]}));
    let hero = r.state.compiled.indices["hero"];
    let camera = r.state.compiled.indices["squad/camera"];
    let old_hero = r.state.transforms[hero].clone();
    let old_camera = r.state.transforms[camera].clone();
    let input = ActionMap([("move_x".into(), 1.0), ("camera.x".into(), 1.0)].into());
    r.step(&input);
    assert_eq!(r.state.transforms[hero], old_hero);
    assert!(r.observation().metrics["rts"]["selected"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_ne!(r.state.transforms[camera], old_camera);
    assert_eq!(r.render_frame().views[0].id, "squad/camera");
    let old_camera = r.state.transforms[camera].clone();
    call(&mut r, "input", "activate", json!({"context":"hero"}));
    assert!(r.pointer_allowed());
    r.step(&input);
    assert_ne!(r.state.transforms[hero], old_hero);
    assert_eq!(r.state.transforms[camera], old_camera);
    assert_eq!(r.render_frame().views[0].id, "camera");
    let old_hero = r.state.transforms[hero].clone();
    r.screen_pointer(
        axiom_runtime::world::PointerKind::SecondaryDown,
        [0.3, 0.4],
        1.6,
        false,
    )
    .unwrap();
    assert_eq!(r.state.transforms[hero], old_hero);
    assert_eq!(r.observation().metrics["rts"]["summary"]["tick"], 2);
    let switch = ActionMap([("context.next".into(), 1.0)].into());
    r.step(&switch);
    assert_eq!(r.observation().metrics["input"]["context"], "squad");
    r.step(&switch);
    assert_eq!(r.observation().metrics["input"]["context"], "squad");
    assert_eq!(r.observation().metrics["action"]["tick"], 4);
}
#[test]
fn native_events_cannot_be_impersonated_by_player_commands() {
    let mut source = world();
    source.resources.get_mut("behavior.program").unwrap()["inputs"] =
        json!({"spoof":"action.victory"});
    source.resources.get_mut("input.bindings").unwrap()["buttons"]["spoof"] = json!("KeyH");
    assert!(source
        .compile(&installed_registry().authoring)
        .unwrap_err()
        .to_string()
        .contains("authored input"));
    let mut r = runtime();
    let error = r
        .command(&RuntimeCommand::Emit {
            event: "rts.delivered".into(),
            fields: [
                ("amount".into(), Scalar::Int(100)),
                ("faction".into(), Scalar::Int(0)),
            ]
            .into(),
        })
        .unwrap_err();
    assert!(error.to_string().contains("registered system"));
    assert_eq!(
        r.observation().metrics["gameplay"]["variables"]["quest.supplies"],
        0
    );
    r.state
        .events
        .publish("action.simulate", "rts.delivered", 0, BTreeMap::new());
    assert!(r.assert_valid().is_err());
    assert_eq!(r.state.events.pending.len(), 0);
}
#[test]
fn unrelated_reload_preserves_progress_and_generated_entity_handles() {
    let mut r = runtime();
    choose(&mut r, "quest.arrival", "accept");
    gather(&mut r);
    call(
        &mut r,
        "rts",
        "train",
        json!({"building":"squad/base","recipe":"soldier","count":1}),
    );
    steps(&mut r, 600);
    let slot = r.state.compiled.indices["produced/0/1"];
    let handle = r.state.ids[slot];
    let before = r.observation().metrics["gameplay"]["variables"].clone();
    let mut world = r.state.compiled.source.clone();
    world.title = "Joint operation hot edit".into();
    r.reconcile(world, 2).unwrap();
    steps(&mut r, 10);
    assert_eq!(
        r.state.ids[r.state.compiled.indices["produced/0/1"]],
        handle
    );
    assert_eq!(
        r.observation().metrics["gameplay"]["variables"]["quest.trained"],
        1
    );
    assert!(
        r.observation().metrics["gameplay"]["variables"]["quest.supplies"]
            .as_i64()
            .unwrap()
            >= before["quest.supplies"].as_i64().unwrap()
    );
    assert_eq!(r.state.events.pending.len(), 0);
}
#[test]
fn event_delivery_is_atomic_and_an_edited_consumer_can_retry_the_pending_batch() {
    let mut r = runtime();
    r.state.events.publish(
        "rts.simulate",
        "rts.produced",
        0,
        [("entity".into(), Scalar::Text("fixture".into()))].into(),
    );
    for amount in [i64::MAX, 1] {
        r.state.events.publish(
            "rts.simulate",
            "rts.delivered",
            0,
            [
                ("amount".into(), Scalar::Int(amount)),
                ("faction".into(), Scalar::Int(0)),
            ]
            .into(),
        );
    }
    r.step(&ActionMap::default());
    assert!(r.state.events.fault.is_some());
    assert_eq!(r.state.events.pending.len(), 3);
    let stopped_tick = r.state.tick;
    r.step(&ActionMap::default());
    assert_eq!(r.state.tick, stopped_tick);
    assert_eq!(
        r.observation().metrics["gameplay"]["variables"]["quest.trained"],
        0
    );
    assert_eq!(
        r.observation().metrics["gameplay"]["variables"]["quest.supplies"],
        0
    );
    let mut source = r.state.compiled.source.clone();
    source
        .packages
        .as_mut()
        .unwrap()
        .packages
        .get_mut("m4.quest")
        .unwrap()
        .imports
        .events
        .remove("rts.produced");
    assert!(r.reconcile(source, 2).is_err());
    assert_eq!(r.rev, 1);
    let mut source = r.state.compiled.source.clone();
    let program = source.resources.get_mut("behavior.program").unwrap();
    program["rules"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|v| v["id"] == "quest.delivery")
        .unwrap()["actions"][0]["kind"] = json!("set");
    r.reconcile(source, 3).unwrap();
    r.step(&ActionMap::default());
    r.assert_valid().unwrap();
    assert_eq!(r.state.events.pending.len(), 0);
    assert_eq!(
        r.observation().metrics["gameplay"]["variables"]["quest.trained"],
        1
    );
    assert_eq!(
        r.observation().metrics["gameplay"]["variables"]["quest.supplies"],
        1
    );
}
#[test]
fn contract_queue_overflow_and_unsupported_checkpoints_are_explicit() {
    let mut r = runtime();
    for _ in 0..129 {
        r.state
            .events
            .publish("action.simulate", "action.victory", 0, BTreeMap::new());
    }
    assert_eq!(r.state.events.pending.len(), 128);
    assert!(r.state.events.fault.is_some());
    assert!(r.checkpoint().is_err());
}
#[test]
fn failed_native_spawn_does_not_publish_a_production_completion() {
    let mut r = runtime();
    let template = r.state.compiled.entities[r.state.compiled.indices["squad/soldier/4"]].clone();
    r.state
        .spawn(&template, "produced/0/1".into(), [9.5, 0.0, 5.5])
        .unwrap();
    call(
        &mut r,
        "rts",
        "train",
        json!({"building":"squad/base","recipe":"soldier","count":1}),
    );
    steps(&mut r, 190);
    assert_eq!(
        r.observation().metrics["gameplay"]["variables"]["quest.trained"],
        0
    );
    assert_eq!(
        r.observation().metrics["rts"]["summary"]["stats"]["produced"],
        0
    );
    assert!(r.observation().metrics["rts"]["summary"]["last_error"]
        .as_str()
        .unwrap()
        .contains("runtime_spawn_failed"));
}
#[test]
fn declared_quest_endings_follow_real_choices_and_hero_death() {
    let mut r = runtime();
    choose(&mut r, "quest.arrival", "decline");
    assert_eq!(r.observation().metrics["gameplay"]["ending"], "declined");
    let mut source = world();
    source
        .entity_mut("hero")
        .unwrap()
        .components
        .get_mut("core.health")
        .unwrap()["value"] = json!(1);
    let mut r = WorldRuntime::new(source, 1, installed_registry()).unwrap();
    choose(&mut r, "quest.arrival", "accept");
    call(&mut r, "action", "attack", json!({}));
    steps(&mut r, 300);
    assert_eq!(r.observation().metrics["gameplay"]["ending"], "hero_fell");
}
#[test]
fn compact_quest_choices_stay_visible_and_click_through_the_shared_command_path() {
    let mut r = runtime();
    let ctx = egui::Context::default();
    axiom_runtime::ui::configure(&ctx);
    let mut ui = axiom_runtime::ui::UiState {
        playing: true,
        ..Default::default()
    };
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 600.0));
    let mut frame = |events: Vec<egui::Event>| {
        let mut actions = vec![];
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ctx| actions = r.draw_ui(ctx, &mut ui),
        );
        (actions, ui.targets.clone())
    };
    frame(vec![]);
    let (_, targets) = frame(vec![]);
    assert!(
        ctx.available_rect().height() > 300.0,
        "quest panel covered the playable viewport"
    );
    let rect = targets["choice/accept"];
    assert!(screen.contains_rect(rect), "{rect:?}");
    assert!(
        rect.height() < 60.0,
        "choice expanded to fill the viewport: {rect:?}"
    );
    let position = rect.center();
    frame(vec![
        egui::Event::PointerMoved(position),
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    let (actions, _) = frame(vec![egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    let command = actions
        .into_iter()
        .find_map(|a| {
            if let axiom_runtime::ui::Action::Gameplay(command) = a {
                Some(command)
            } else {
                None
            }
        })
        .expect("choice click did not emit a gameplay command");
    r.command(&command).unwrap();
    assert_eq!(r.observation().metrics["gameplay"]["node"], "quest.active");
}
