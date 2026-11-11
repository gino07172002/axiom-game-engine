//! Choreography and interaction (`content/duet-lab`): a batch command drives two actors on
//! one tick (all validated first), `animation.choreography` stages with `after` chaining on
//! clip end, rule `call` actions, `when` conditions reading animation state, the slider
//! widget command, the orbit camera's drag / wheel / bone focus, and clicks on bone capsules.
use axiom_core::{
    gameplay::{BoundWidget, RuntimeCommand},
    ContentPackage, World,
};
use axiom_runtime::world::{ActionMap, PointerKind, WorldRuntime};
use serde_json::{json, Value};

fn lab() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/duet-lab"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap()
}

fn run(r: &mut WorldRuntime, ticks: u32) {
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
}

fn call(
    r: &mut WorldRuntime,
    capability: &str,
    action: &str,
    args: Value,
) -> Result<Value, String> {
    r.command(&RuntimeCommand::Call {
        capability: capability.into(),
        action: action.into(),
        args,
    })
    .map_err(|e| format!("{e:#}"))
}

fn player(r: &WorldRuntime, id: &str) -> Value {
    r.observation().metrics["animation"]["players"][id].clone()
}

/// The command of UI button `id` as authored (what a click sends).
fn button(r: &WorldRuntime, id: &str) -> RuntimeCommand {
    r.state
        .gameplay
        .as_ref()
        .unwrap()
        .compiled
        .widgets
        .iter()
        .find_map(|w| match w {
            BoundWidget::Button(b, _, command, _) if b == id => Some(command.clone()),
            _ => None,
        })
        .unwrap()
}

fn button_visible(r: &WorldRuntime, id: &str) -> bool {
    let g = r.state.gameplay.as_ref().unwrap();
    g.compiled.widgets.iter().any(|w| match w {
        BoundWidget::Button(b, _, _, when) => b == id && g.test(when),
        _ => false,
    })
}

fn variable(r: &WorldRuntime, id: &str) -> Value {
    r.observation().metrics["gameplay"]["variables"][id].clone()
}

#[test]
fn one_button_batch_starts_two_actors_on_the_same_tick() {
    let mut r = runtime(lab());
    run(&mut r, 30);
    let click = button(&r, "walk_both");
    assert!(matches!(click, RuntimeCommand::Batch { .. }));
    let tick = r.state.tick;
    let reply = r.command(&click).unwrap();
    assert_eq!(reply["batch"].as_array().unwrap().len(), 2, "{reply}");
    for id in ["axi", "lyn"] {
        let p = player(&r, id);
        assert_eq!(p["clip"], "walk", "{id}: {p}");
        assert_eq!(p["time"], 0.0, "{id}: {p}");
    }
    assert_eq!(r.state.tick, tick, "a command does not step");
    run(&mut r, 7);
    assert_eq!(player(&r, "axi")["time"], player(&r, "lyn")["time"]);

    // Atomic: the second item is refused, so the first does not apply either.
    let before = (player(&r, "axi"), player(&r, "lyn"));
    let bad = RuntimeCommand::Batch {
        commands: vec![
            RuntimeCommand::Call {
                capability: "animation".into(),
                action: "play".into(),
                args: json!({"entity":"axi","clip":"run"}),
            },
            RuntimeCommand::Call {
                capability: "animation".into(),
                action: "play".into(),
                args: json!({"entity":"lyn","clip":"nope"}),
            },
        ],
    };
    let e = format!("{:#}", r.command(&bad).unwrap_err());
    assert!(
        e.contains("/command/commands/1") && e.contains("clip not found: nope"),
        "{e}"
    );
    assert_eq!((player(&r, "axi"), player(&r, "lyn")), before);
    // A refused gameplay item (unknown event) also stops the whole batch.
    let bad = RuntimeCommand::Batch {
        commands: vec![
            RuntimeCommand::Call {
                capability: "animation".into(),
                action: "play".into(),
                args: json!({"entity":"axi","clip":"run"}),
            },
            RuntimeCommand::Emit {
                event: "nope".into(),
                fields: Default::default(),
            },
        ],
    };
    assert!(r.command(&bad).is_err());
    assert_eq!(player(&r, "axi"), before.0);
    // Nested batches and empty batches are refused.
    let nested = RuntimeCommand::Batch {
        commands: vec![RuntimeCommand::Batch { commands: vec![] }],
    };
    assert!(format!("{:#}", r.command(&nested).unwrap_err()).contains("cannot hold batch"));
    assert!(r
        .command(&RuntimeCommand::Batch { commands: vec![] })
        .is_err());
    // Mixed: a gameplay emit and a call in one tick.
    let mixed = RuntimeCommand::Batch {
        commands: vec![
            RuntimeCommand::Emit {
                event: "fill".into(),
                fields: Default::default(),
            },
            RuntimeCommand::Call {
                capability: "animation".into(),
                action: "speed".into(),
                args: json!({"entity":"lyn","speed":0.5}),
            },
        ],
    };
    r.command(&mixed).unwrap();
    assert_eq!(variable(&r, "gauge"), 25.0);
    assert_eq!(
        call(&mut r, "animation", "inspect", json!({"entity":"lyn"})).unwrap()["result"]["speed"],
        0.5
    );
}

#[test]
fn a_stage_starts_every_actor_and_chains_on_clip_end() {
    let mut r = runtime(lab());
    run(&mut r, 10);
    let reply = call(&mut r, "animation", "stage", json!({"stage":"greet"})).unwrap();
    assert_eq!(reply["result"]["actors"], json!(["axi", "lyn"]), "{reply}");
    assert_eq!(player(&r, "axi")["clip"], "wave");
    assert_eq!(player(&r, "lyn")["clip"], "bow");
    assert_eq!(player(&r, "axi")["time"], 0.0);
    assert_eq!(player(&r, "lyn")["time"], 0.0);
    let stage = &r.observation().metrics["animation"]["stage"];
    assert_eq!(stage["active"], "greet", "{stage}");
    assert_eq!(stage["runs"], 1);
    // The wave is a one-shot: when Axi's wave completes, `rest` starts on that step.
    let clips = call(&mut r, "animation", "inspect", json!({"entity":"axi"})).unwrap();
    assert_eq!(clips["result"]["clip"], "wave");
    let mut ticks = 0;
    while player(&r, "axi")["clip"] == "wave" {
        run(&mut r, 1);
        ticks += 1;
        assert!(ticks < 2400, "the wave never ended");
    }
    assert_eq!(player(&r, "axi")["clip"], "idle");
    assert_eq!(player(&r, "lyn")["clip"], "idle");
    let stage = &r.observation().metrics["animation"]["stage"];
    assert_eq!(stage["active"], "rest", "{stage}");
    assert_eq!(stage["runs"], 2);
    // The wave lasted its duration (within one step).
    let wave = r.observation().metrics["animation"]["players"]["axi"].clone();
    assert!(ticks > 30, "{ticks} {wave}");
    // Unknown stages are refused without changing anything.
    let e = call(&mut r, "animation", "stage", json!({"stage":"nope"})).unwrap_err();
    assert!(e.contains("unknown stage nope"), "{e}");
    assert_eq!(
        r.observation().metrics["animation"]["stage"]["active"],
        "rest"
    );
}

#[test]
fn stages_are_checked_against_the_players_when_built() {
    let mut w = lab();
    w.resources.get_mut("animation.choreography").unwrap()["stages"]["rest"]["actors"][0]["clip"] =
        json!("nope");
    let e = format!(
        "{:#}",
        WorldRuntime::new(w, 1, axiom_runtime::installed_registry())
            .err()
            .unwrap()
    );
    assert!(e.contains("clip nope not found"), "{e}");
    let mut w = lab();
    w.resources.get_mut("animation.choreography").unwrap()["stages"]["rest"]["actors"][0]
        ["entity"] = json!("stage/floor");
    let e = format!(
        "{:#}",
        WorldRuntime::new(w, 1, axiom_runtime::installed_registry())
            .err()
            .unwrap()
    );
    assert!(e.contains("has no animation.player"), "{e}");
}

#[test]
fn a_rule_calls_a_capability_when_the_gauge_fills() {
    let mut r = runtime(lab());
    run(&mut r, 5);
    let fill = button(&r, "fill");
    for _ in 0..3 {
        r.command(&fill).unwrap();
    }
    assert_eq!(variable(&r, "gauge"), 75.0);
    assert_eq!(player(&r, "axi")["clip"], "idle");
    r.command(&fill).unwrap();
    // The fourth fill reaches 100: the rule resets the gauge and calls animation.stage.
    assert_eq!(variable(&r, "gauge"), 0.0);
    assert_eq!(player(&r, "axi")["clip"], "wave");
    let calls = &r.observation().metrics["calls"];
    assert_eq!(calls["run"], 1, "{calls}");
    assert_eq!(calls["failed"], 0, "{calls}");
    // A call that fails is recoverable: counted, reported, the simulation continues.
    let mut w = lab();
    w.resources.get_mut("behavior.program").unwrap()["rules"][1]["actions"][1]["args"] =
        json!({"stage":"missing"});
    let mut r = runtime(w);
    for _ in 0..4 {
        r.command(&fill).unwrap();
    }
    run(&mut r, 3);
    let calls = &r.observation().metrics["calls"];
    assert_eq!(calls["failed"], 1, "{calls}");
    assert!(
        calls["fault"]
            .as_str()
            .unwrap()
            .contains("unknown stage missing"),
        "{calls}"
    );
    assert_eq!(variable(&r, "gauge"), 0.0);
    r.assert_valid().unwrap();
    // A call to an action the registry does not know is an authoring error with its path.
    let mut w = lab();
    w.resources.get_mut("behavior.program").unwrap()["rules"][1]["actions"][1]["action"] =
        json!("explode");
    let e = format!(
        "{:#}",
        axiom_runtime::installed_registry()
            .authoring
            .validate_world(&w)
            .unwrap_err()
    );
    assert!(e.contains("unknown_action") || e.contains("explode"), "{e}");
}

#[test]
fn a_button_hides_while_an_animation_plays() {
    let mut r = runtime(lab());
    run(&mut r, 2);
    assert!(button_visible(&r, "greet"));
    let greet = button(&r, "greet");
    r.command(&greet).unwrap();
    // The read is published with the command, so the button hides at once.
    assert!(!button_visible(&r, "greet"));
    let reads = &r.observation().metrics["gameplay"]["animation_reads"];
    assert_eq!(reads["axi.clip"], "wave", "{reads}");
    let mut ticks = 0;
    while !button_visible(&r, "greet") {
        run(&mut r, 1);
        ticks += 1;
        assert!(ticks < 2400);
    }
    assert_eq!(player(&r, "axi")["clip"], "idle");
    // Reads of entities without a player / controller fail when the runtime is built.
    let mut w = lab();
    w.resources.get_mut("ui.document").unwrap()["widgets"][6]["when"] = json!({"kind":"eq","left":{"kind":"animation","entity":"axi","field":"state"},"right":{"kind":"literal","value":"x"}});
    let e = format!(
        "{:#}",
        WorldRuntime::new(w, 1, axiom_runtime::installed_registry())
            .err()
            .unwrap()
    );
    assert!(e.contains("has no animation.controller"), "{e}");
    // Type checks: clip is text.
    let mut w = lab();
    w.resources.get_mut("ui.document").unwrap()["widgets"][6]["when"] = json!({"kind":"ge","left":{"kind":"animation","entity":"axi","field":"clip"},"right":{"kind":"literal","value":1.0}});
    assert!(axiom_runtime::installed_registry()
        .authoring
        .validate_world(&w)
        .is_err());
}

#[test]
fn the_slider_sends_one_command_per_change() {
    let mut r = runtime(lab());
    run(&mut r, 2);
    let slide = |r: &mut WorldRuntime, value: f64| {
        r.command(&RuntimeCommand::Slide {
            widget: "speed".into(),
            value,
        })
        .map_err(|e| format!("{e:#}"))
    };
    let reply = slide(&mut r, 1.6).unwrap();
    // Snapped to the 0.25 step; the variable and the call argument get the same value.
    assert_eq!(reply["value"], 1.5, "{reply}");
    assert_eq!(variable(&r, "rate"), 1.5);
    assert_eq!(reply["result"]["speed"], 1.5, "{reply}");
    assert_eq!(
        r.observation().metrics["gameplay"]["bindings"]["speed"],
        1.5
    );
    // Clamped to max.
    assert_eq!(slide(&mut r, 9.0).unwrap()["value"], 2.0);
    // Refused: not finite, not a slider; nothing changes.
    assert!(slide(&mut r, f64::NAN).is_err());
    assert!(r
        .command(&RuntimeCommand::Slide {
            widget: "fill".into(),
            value: 1.0
        })
        .is_err());
    assert_eq!(variable(&r, "rate"), 2.0);
    // A slider whose call is refused leaves its variable unchanged.
    let mut w = lab();
    w.resources.get_mut("ui.document").unwrap()["widgets"][10]["command"]["args"]["entity"] =
        json!("stage/floor");
    let mut r = runtime(w);
    assert!(slide(&mut r, 0.5).is_err());
    assert_eq!(variable(&r, "rate"), 1.0);
}

#[test]
fn drag_orbits_the_wheel_zooms_and_the_focus_follows_a_bone() {
    let mut r = runtime(lab());
    run(&mut r, 60);
    let cam = |r: &mut WorldRuntime| {
        call(r, "camera", "inspect", json!({"id":"camera/orbit"})).unwrap()["result"].clone()
    };
    let before = cam(&mut r);
    assert_eq!(before["bone"], "chest", "{before}");
    // The look point is Axi's animated chest, not the pivot on the floor.
    let focus_y = before["focus"][1].as_f64().unwrap();
    assert!(focus_y > 0.3, "{before}");
    // Drag right by a tenth of the view: yaw goal -36 degrees (360 per view width).
    r.screen_pointer(PointerKind::PrimaryDown, [0.5, 0.5], 1.6, false)
        .unwrap();
    r.screen_pointer(PointerKind::Move, [0.55, 0.5], 1.6, false)
        .unwrap();
    r.screen_pointer(PointerKind::Move, [0.6, 0.5], 1.6, false)
        .unwrap();
    r.screen_pointer(PointerKind::PrimaryUp, [0.6, 0.5], 1.6, false)
        .unwrap();
    run(&mut r, 240);
    let after = cam(&mut r);
    let yaw = |v: &Value| v["yaw"].as_f64().unwrap();
    assert!(
        (yaw(&after) - (yaw(&before) - 36.0)).abs() < 0.5,
        "{before} {after}"
    );
    // A drag is not a click on a part.
    assert_eq!(variable(&r, "part"), "-");
    // Wheel up = the pseudo key WheelUp on the zoom axis for one step: 60 m/s / 120 closer.
    let goal = after["goal"].as_f64().unwrap();
    let mut wheel = ActionMap::default();
    wheel.0.insert("zoom".into(), -1.0);
    r.step(&wheel);
    let zoomed = cam(&mut r)["goal"].as_f64().unwrap();
    assert!((goal - zoomed - 0.5).abs() < 1e-3, "{goal} {zoomed}");
}

#[test]
fn clicking_a_body_part_reports_entity_and_part() {
    let mut r = runtime(lab());
    run(&mut r, 120);
    // The camera looks at Axi's chest: the view center hits Axi.
    r.screen_pointer(PointerKind::PrimaryDown, [0.5, 0.5], 1.6, false)
        .unwrap();
    r.screen_pointer(PointerKind::PrimaryUp, [0.5, 0.5], 1.6, false)
        .unwrap();
    assert_eq!(variable(&r, "who"), "axi");
    let part = variable(&r, "part");
    assert!(part != "-", "{part}");
    let pick = &r.observation().metrics["animation"]["pick"];
    assert_eq!(pick["hits"], 1, "{pick}");
    assert_eq!(pick["last"]["entity"], "axi", "{pick}");
    // The same query as an action: straight down onto Lyn's head part.
    let head = call(
        &mut r,
        "animation",
        "inspect",
        json!({"entity":"lyn","bones":["head"]}),
    )
    .unwrap()["result"]["bones"]["head"]["position"]
        .clone();
    let (hx, hz) = (head[0].as_f64().unwrap(), head[2].as_f64().unwrap());
    let hit = call(
        &mut r,
        "animation",
        "pick",
        json!({"origin":[0.42 + hx * 0.0, 5.0, hz],"direction":[0,-1,0]}),
    )
    .unwrap();
    assert_eq!(hit["result"]["entity"], "lyn", "{hit}");
    assert_eq!(hit["result"]["part"], "head", "{hit}");
    // Missing everything.
    let miss = call(
        &mut r,
        "animation",
        "pick",
        json!({"origin":[5,5,5],"direction":[0,1,0]}),
    )
    .unwrap();
    assert_eq!(miss["result"], Value::Null);
    // pick is a query: nothing was counted.
    assert_eq!(r.observation().metrics["animation"]["pick"]["hits"], 1);
}

/// duet-lab without the camera rig (no checkpoint support) and with the save capability.
fn savable() -> World {
    let mut w = lab();
    w.capabilities.insert("save".into(), 1);
    w.systems.retain(|s| s != "camera.rig");
    w.entities.retain(|e| e.id != "camera/orbit");
    w.capabilities.remove("camera");
    let cam = json!({"core.transform":{"position":[0,0.9,3.2]},"render.camera":{"projection":{"kind":"perspective","fov_degrees":40},"near":0.04,"far":40,"viewport":[0,0,1,1],"active":true}});
    w.entities
        .push(serde_json::from_value(json!({"id":"camera/fixed","components":cam})).unwrap());
    w
}

#[test]
fn checkpoints_keep_the_stage_calls_and_clicks_and_replay_matches() {
    // The gameplay trace (diagnostic ring buffer) is not part of checkpoints (cleared on
    // load, as before this workstream); everything else must match.
    let state = |r: &WorldRuntime| {
        let mut m = r.observation().metrics;
        m.get_mut("gameplay").unwrap()["trace"] = Value::Null;
        (r.state.tick, m)
    };
    let mut r = runtime(savable());
    run(&mut r, 20);
    let fill = button(&r, "fill");
    for _ in 0..4 {
        r.command(&fill).unwrap();
    }
    r.screen_pointer(PointerKind::PrimaryDown, [0.38, 0.45], 1.6, false)
        .unwrap();
    r.screen_pointer(PointerKind::PrimaryUp, [0.38, 0.45], 1.6, false)
        .unwrap();
    run(&mut r, 30);
    assert_eq!(
        r.observation().metrics["animation"]["stage"]["active"],
        "greet"
    );
    assert_eq!(r.observation().metrics["calls"]["run"], 1);
    let saved = r.checkpoint().unwrap();
    let text = serde_json::to_string(&saved).unwrap();
    let saved: axiom_runtime::checkpoint::Checkpoint = serde_json::from_str(&text).unwrap();
    let mut restored = runtime(savable());
    restored.restore_checkpoint(&saved).unwrap();
    assert_eq!(state(&restored), state(&r), "restored state differs");
    run(&mut r, 400);
    run(&mut restored, 400);
    assert_eq!(
        state(&restored),
        state(&r),
        "replay after the checkpoint differs"
    );
    assert_eq!(r.state.transforms, restored.state.transforms);
    assert_eq!(
        r.observation().metrics["animation"]["stage"]["active"],
        "rest"
    );
}

#[test]
fn scenario_replay_is_deterministic() {
    let scenario = || -> axiom_runtime::scenario::Scenario {
        serde_json::from_str(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/duet-lab.scenario.json"
            ))
            .unwrap(),
        )
        .unwrap()
    };
    let a = axiom_runtime::scenario::run(lab(), 1, axiom_runtime::installed_registry(), scenario())
        .unwrap();
    assert_eq!(a["ok"], true, "{a}");
    let b = axiom_runtime::scenario::run(lab(), 1, axiom_runtime::installed_registry(), scenario())
        .unwrap();
    assert_eq!(a["state_hash"], b["state_hash"]);
}
