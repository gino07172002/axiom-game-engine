//! Interactables (`interact.target` + `interact.focus`): focus selection, input and
//! `interact.use`, cooldown / once / enabled / filters, prompt bindings, hot reload,
//! checkpoints, authoring errors and the axiom-town port.
use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use serde_json::{json, Value};

/// Targets and interactors on an empty plane. Event `talk` (payload interactor + target)
/// counts into variables and is bound to a native call; `bare` has no payload.
fn world(entities: Value) -> World {
    serde_json::from_value(json!({
        "schema_version":2,"id":"interact-test","title":"Interact test",
        "capabilities":{"core":1,"interact":1,"behavior":1,"ui":1},
        "systems":["interact.focus","behavior.execute"],
        "entities":entities,
        "resources":{
            "input.bindings":{"buttons":{"interact":"KeyE","use":"KeyF"}},
            "behavior.program":{
                "variables":{"talks":0,"who":"","what":"","bares":0},
                "events":{"talk":{"fields":{"interactor":"text","target":"text"}},"bare":{"fields":{}}},
                "rules":[
                    {"id":"count-talk","event":"talk","actions":[
                        {"kind":"add","variable":"talks","value":{"kind":"literal","value":1}},
                        {"kind":"set","variable":"who","value":{"kind":"event","field":"interactor"}},
                        {"kind":"set","variable":"what","value":{"kind":"event","field":"target"}}]},
                    {"id":"count-bare","event":"bare","actions":[
                        {"kind":"add","variable":"bares","value":{"kind":"literal","value":1}}]}
                ]
            },
            "ui.document":{"locale":"en","catalogs":{"en":{"greet":"Talk to {key} now","hello":"Say hello"},"zh":{"greet":"按 {key} 交談","hello":"打招呼"}},
                "widgets":[{"kind":"text","id":"prompt","binding":{"kind":"prompt"}}]},
            "runtime.bindings":{"events":[{"event":"talk","calls":[{"kind":"call","capability":"interact","action":"summary","args":{}}]}]}
        }
    }))
    .unwrap()
}

fn target(id: &str, at: [f32; 3], target: Value) -> Value {
    json!({"id":id,"components":{"core.transform":{"position":at},"interact.target":target}})
}

fn mover(id: &str, at: [f32; 3], tags: &[&str]) -> Value {
    json!({"id":id,"tags":tags,"components":{"core.transform":{"position":at}}})
}

fn talk() -> Value {
    json!({"range":2.0,"event":"talk"})
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, installed_registry()).unwrap()
}

fn step(r: &mut WorldRuntime) {
    r.step(&ActionMap::default());
}

fn press(r: &mut WorldRuntime, action: &str) {
    let mut input = ActionMap::default();
    input.0.insert(action.into(), 1.0);
    r.step(&input);
}

fn summary(r: &WorldRuntime) -> Value {
    r.observation().metrics["interact"].clone()
}

fn focus(r: &WorldRuntime, interactor: &str) -> Value {
    summary(r)["focus"][interactor].clone()
}

fn var(r: &WorldRuntime, name: &str) -> Value {
    r.observation().metrics["gameplay"]["variables"][name].clone()
}

fn prompt(r: &WorldRuntime) -> Value {
    r.observation().metrics["gameplay"]["bindings"]["prompt"].clone()
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "interact".into(),
        action: action.into(),
        args,
    })
    .map(|v| v["result"].clone())
}

fn compile_error(w: &World) -> String {
    format!(
        "{:#}",
        w.compile(&installed_registry().authoring).unwrap_err()
    )
}

#[test]
fn key_press_uses_the_focus_and_reaches_rules_and_bindings() {
    let mut r = runtime(world(json!([
        target("npc", [0.0; 3], talk()),
        mover("hero", [1.5, 0.0, 0.0], &["player"]),
    ])));
    // Focus needs one step; pressing in the same step as the first focus does nothing (the
    // prompt was not shown yet).
    press(&mut r, "interact");
    assert_eq!(focus(&r, "hero"), "npc");
    assert_eq!(var(&r, "talks"), 0);
    // Holding the key is not a new press; release and press again uses the target.
    press(&mut r, "interact");
    assert_eq!(var(&r, "talks"), 0);
    step(&mut r);
    press(&mut r, "interact");
    assert_eq!(var(&r, "talks"), 1);
    assert_eq!(var(&r, "who"), "hero");
    assert_eq!(var(&r, "what"), "npc");
    let s = summary(&r);
    assert_eq!(s["recent"], json!([[4, "hero", "npc", "key"]]), "{s}");
    assert_eq!(s["used"], 1);
    // runtime.bindings run at the end of the step the event fired in.
    assert_eq!(r.observation().metrics["bindings"]["calls"], 1);
    // Another button does not use a target bound to `interact`.
    step(&mut r);
    press(&mut r, "use");
    assert_eq!(var(&r, "talks"), 1);
    // Out of range: no focus, pressing does nothing.
    r.state.set_position("hero", [5.0, 0.0, 0.0]);
    step(&mut r);
    assert!(focus(&r, "hero").is_null());
    press(&mut r, "interact");
    assert_eq!(var(&r, "talks"), 1);
}

#[test]
fn range_uses_world_pivots_and_exit_margin_holds_focus() {
    let mut r = runtime(world(json!([
        {"id":"base","components":{"core.transform":{"position":[10.0,0.0,0.0]}}},
        target("door", [0.0; 3], json!({"range":2.0,"exit_margin":0.5,"event":"talk"})),
        mover("hero", [12.2, 0.0, 0.0], &["player"]),
    ])));
    // The door is a child of base: world pivot (10,0,0).
    let mut w = r.state.compiled.source.clone();
    w.relations.push(
        serde_json::from_value(json!({"kind":"parent","source":"door","target":"base"})).unwrap(),
    );
    r = runtime(w);
    step(&mut r);
    assert!(focus(&r, "hero").is_null(), "2.2 m > range 2");
    r.state.set_position("hero", [11.9, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "door");
    // Inside range + margin the focus holds; beyond it, it is lost; re-entering needs range.
    r.state.set_position("hero", [12.4, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "door");
    r.state.set_position("hero", [12.6, 0.0, 0.0]);
    step(&mut r);
    assert!(focus(&r, "hero").is_null());
    r.state.set_position("hero", [12.3, 0.0, 0.0]);
    step(&mut r);
    assert!(focus(&r, "hero").is_null());
    // Distance is 3D: the same XZ spot two floors up is out of range.
    r.state.set_position("hero", [11.0, 6.0, 0.0]);
    step(&mut r);
    assert!(focus(&r, "hero").is_null());
}

#[test]
fn view_angle_requires_the_interactor_to_face_the_target() {
    let mut r = runtime(world(json!([
        target(
            "sign",
            [0.0, 0.0, 1.5],
            json!({"view_angle":90.0,"event":"talk"})
        ),
        mover("hero", [0.0; 3], &["player"]),
    ])));
    // Identity rotation faces +Z: the sign is straight ahead.
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "sign");
    // view_angle 90 = within +-45 degrees: turned 180 (behind) or 60 degrees away, no focus.
    let set_yaw = |r: &mut WorldRuntime, degrees: f32| {
        let slot = r.state.compiled.indices["hero"];
        r.state.transforms[slot].as_mut().unwrap().rotation =
            glam::Quat::from_rotation_y(degrees.to_radians()).to_array();
    };
    set_yaw(&mut r, 180.0);
    step(&mut r);
    assert!(focus(&r, "hero").is_null());
    set_yaw(&mut r, 60.0);
    step(&mut r);
    assert!(focus(&r, "hero").is_null());
    set_yaw(&mut r, 40.0);
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "sign");
    let inspect = call(&mut r, "inspect", json!({"id":"hero"})).unwrap();
    assert_eq!(inspect["focus"], "sign");
    set_yaw(&mut r, -90.0);
    step(&mut r);
    let inspect = call(&mut r, "inspect", json!({"id":"hero"})).unwrap();
    assert_eq!(
        inspect["targets"][0]["reason"],
        "outside the target's view_angle"
    );
}

#[test]
fn selection_is_priority_then_distance_then_facing_then_authored_order() {
    let mut r = runtime(world(json!([
        target(
            "far-important",
            [0.0, 0.0, 1.9],
            json!({"priority":5,"event":"talk"})
        ),
        target("near", [0.0, 0.0, 0.5], talk()),
        target("nearer", [0.0, 0.0, -0.4], talk()),
        mover("hero", [0.0; 3], &["player"]),
    ])));
    step(&mut r);
    assert_eq!(
        focus(&r, "hero"),
        "far-important",
        "priority beats distance"
    );
    // Same priority: nearer wins.
    let mut w = r.state.compiled.source.clone();
    w.entities[0].components.get_mut("interact.target").unwrap()["priority"] = json!(0);
    r.reconcile(w, 2).unwrap();
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "nearer");
    // Equal distance: the one in front (+Z) wins over the one behind.
    let mut r = runtime(world(json!([
        target("behind", [0.0, 0.0, -1.0], talk()),
        target("front", [0.0, 0.0, 1.0], talk()),
        target("side", [1.0, 0.0, 0.0], talk()),
        mover("hero", [0.0; 3], &["player"]),
    ])));
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "front");
    // Exact tie on everything: authored order.
    let mut r = runtime(world(json!([
        target("first", [1.0, 0.0, 0.0], talk()),
        target("second", [1.0, 0.0, 0.0], talk()),
        mover("hero", [0.0; 3], &["player"]),
    ])));
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "first");
}

#[test]
fn cooldown_once_disabled_and_inactive_targets_are_not_focused() {
    let mut w = world(json!([
        target(
            "well",
            [1.0, 0.0, 0.0],
            json!({"cooldown":0.5,"event":"talk"})
        ),
        target(
            "chest",
            [-1.0, 0.0, 0.0],
            json!({"once":true,"event":"bare","priority":-1})
        ),
        target(
            "off",
            [0.0, 0.0, 0.5],
            json!({"enabled":false,"event":"talk","priority":9})
        ),
        target(
            "hidden",
            [0.0, 0.0, -0.5],
            json!({"event":"talk","priority":9})
        ),
        mover("hero", [0.0; 3], &["player"]),
    ]));
    w.scenes =
        vec![
            serde_json::from_value(json!({"id":"cellar","entities":["hidden"],"active":false}))
                .unwrap(),
        ];
    let mut r = runtime(w);
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "well");
    press(&mut r, "interact");
    assert_eq!(var(&r, "talks"), 1);
    // Cooling down (60 ticks): the chest takes the focus meanwhile, the prompt follows.
    assert_eq!(focus(&r, "hero"), "chest");
    let left =
        call(&mut r, "inspect", json!({"id":"well"})).unwrap()["cooldown_ticks_left"].clone();
    assert_eq!(left, 60);
    let error = format!(
        "{:#}",
        call(&mut r, "use", json!({"target":"well"})).unwrap_err()
    );
    assert!(error.contains("cooling down (60 ticks left)"), "{error}");
    step(&mut r);
    press(&mut r, "interact");
    assert_eq!(var(&r, "bares"), 1);
    // The chest is spent for good; the well returns when its cooldown ends (tick 2 + 60).
    for _ in 0..57 {
        step(&mut r);
    }
    assert_eq!(r.state.tick, 61);
    assert!(focus(&r, "hero").is_null());
    step(&mut r);
    assert_eq!(focus(&r, "hero"), "well");
    let error = format!(
        "{:#}",
        call(&mut r, "use", json!({"target":"chest"})).unwrap_err()
    );
    assert!(error.contains("once and already used"), "{error}");
    let error = format!(
        "{:#}",
        call(&mut r, "use", json!({"target":"off"})).unwrap_err()
    );
    assert!(error.contains("disabled"), "{error}");
    let error = format!(
        "{:#}",
        call(&mut r, "use", json!({"target":"hidden"})).unwrap_err()
    );
    assert!(error.contains("inactive scene"), "{error}");
}

#[test]
fn interactor_filters_select_entities_and_the_local_interactor_drives_input() {
    let mut r = runtime(world(json!([
        target("terminal", [0.0; 3], json!({"event":"talk","interactor":{"tags":["crew"],"components":["core.health"]}})),
        target("npc", [0.0, 0.0, 3.0], talk()),
        mover("drone", [0.5, 0.0, 0.0], &["crew"]),
        {"id":"dead","tags":["crew","player"],"components":{"core.transform":{"position":[0.3,0.0,0.0]},"core.health":{"value":0,"max":10}}},
        {"id":"engineer","tags":["crew"],"components":{"core.transform":{"position":[-0.5,0.0,0.0]},"core.health":{"value":10,"max":10}}},
        mover("hero", [0.0, 0.0, 2.0], &["player"]),
    ])));
    step(&mut r);
    let s = summary(&r);
    // drone lacks core.health; dead is dead; the terminal never interacts with itself.
    assert_eq!(
        s["focus"],
        json!({"engineer":"terminal","hero":"npc"}),
        "{s}"
    );
    assert_eq!(s["interactors"], 3);
    // The local interactor is the first enabled, living candidate in authored order.
    assert_eq!(s["local"], "engineer");
    press(&mut r, "interact");
    step(&mut r);
    press(&mut r, "interact");
    assert_eq!(var(&r, "who"), "engineer");
    // Other interactors use targets through interact.use.
    let used = call(&mut r, "use", json!({"interactor":"hero"})).unwrap();
    assert_eq!(
        used,
        json!({"interactor":"hero","target":"npc","event":"talk","uses":1})
    );
    assert_eq!(var(&r, "who"), "hero");
    let error = format!(
        "{:#}",
        call(
            &mut r,
            "use",
            json!({"interactor":"hero","target":"terminal"})
        )
        .unwrap_err()
    );
    assert!(error.contains("interactor filter"), "{error}");
    let error = format!(
        "{:#}",
        call(&mut r, "use", json!({"interactor":"drone"})).unwrap_err()
    );
    assert!(
        error.contains("not_found") || error.contains("matches no"),
        "{error}"
    );
}

#[test]
fn interact_use_reports_recoverable_errors_and_dispatches_bindings() {
    let mut r = runtime(world(json!([
        target("npc", [0.0; 3], talk()),
        target("far", [9.0, 0.0, 0.0], talk()),
        mover("hero", [1.0, 0.0, 0.0], &["player"]),
    ])));
    step(&mut r);
    let used = call(&mut r, "use", json!({})).unwrap();
    assert_eq!(used["target"], "npc");
    assert_eq!(var(&r, "what"), "npc");
    // The bound native call runs at the next step boundary.
    step(&mut r);
    assert_eq!(r.observation().metrics["bindings"]["calls"], 1);
    let recent = &summary(&r)["recent"];
    assert_eq!(recent[0][3], "call");
    for (args, expect) in [
        (json!({"target":"far"}), "out of range (8.00 m > 2 m)"),
        (json!({"target":"nobody"}), "is not an interact.target"),
        (
            json!({"interactor":"npc"}),
            "matches no interact.target interactor filter",
        ),
    ] {
        let error = format!("{:#}", call(&mut r, "use", args).unwrap_err());
        assert!(error.contains(expect), "{error}");
    }
    // Argument validation happens before the system runs.
    let error = format!(
        "{:#}",
        call(&mut r, "use", json!({"who":"hero"})).unwrap_err()
    );
    assert!(error.contains("interactor and target"), "{error}");
    r.state.set_position("hero", [5.0, 0.0, 0.0]);
    step(&mut r);
    let error = format!("{:#}", call(&mut r, "use", json!({})).unwrap_err());
    assert!(error.contains("no target in range"), "{error}");
    assert_eq!(var(&r, "talks"), 1);
}

#[test]
fn prompt_binding_follows_the_local_focus_and_locale() {
    let mut r = runtime(world(json!([
        target("npc", [0.0; 3], json!({"prompt":"greet","event":"talk"})),
        target(
            "sign",
            [6.0, 0.0, 0.0],
            json!({"prompt":"Read the sign","event":"talk"})
        ),
        target(
            "plain",
            [12.0, 0.0, 0.0],
            json!({"event":"bare","action":"use"})
        ),
        target(
            "silent",
            [18.0, 0.0, 0.0],
            json!({"prompt":"hello","event":"talk","action":""})
        ),
        mover("hero", [1.0, 0.0, 0.0], &["player"]),
    ])));
    assert_eq!(prompt(&r), "");
    step(&mut r);
    // Catalog key with {key}; literal text with the [KEY] prefix; the default prompt; no key.
    assert_eq!(prompt(&r), "Talk to E now");
    r.state.set_position("hero", [7.0, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(prompt(&r), "[E] Read the sign");
    r.state.set_position("hero", [13.0, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(prompt(&r), "[F] Interact");
    r.state.set_position("hero", [19.0, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(prompt(&r), "Say hello");
    r.command(&RuntimeCommand::Locale {
        locale: "zh".into(),
    })
    .unwrap();
    assert_eq!(prompt(&r), "打招呼");
    r.state.set_position("hero", [1.0, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(prompt(&r), "按 E 交談");
    r.state.set_position("hero", [30.0, 0.0, 0.0]);
    step(&mut r);
    assert_eq!(prompt(&r), "");
    assert!(r.state.gameplay.as_ref().unwrap().prompt.is_none());
}

#[test]
fn hot_reload_keeps_uses_cooldown_focus_and_held_keys() {
    let w = world(json!([
        target("chest", [0.0; 3], json!({"once":true,"event":"talk"})),
        target(
            "well",
            [0.0, 0.0, 1.0],
            json!({"cooldown":10.0,"event":"talk","priority":-1})
        ),
        mover("hero", [0.5, 0.0, 0.0], &["player"]),
    ]));
    let mut r = runtime(w.clone());
    step(&mut r);
    press(&mut r, "interact");
    assert_eq!(var(&r, "what"), "chest");
    assert_eq!(focus(&r, "hero"), "well");
    step(&mut r);
    press(&mut r, "interact");
    assert_eq!(var(&r, "talks"), 2);
    // Unrelated edit while the key is still held: no new use, the once / cooldown state and
    // the focus survive (nothing is in range now: chest spent, well cooling).
    let mut edited = w.clone();
    edited
        .entities
        .push(serde_json::from_value(mover("other", [9.0, 0.0, 0.0], &[])).unwrap());
    r.reconcile(edited.clone(), 2).unwrap();
    press(&mut r, "interact");
    assert_eq!(var(&r, "talks"), 2);
    assert!(focus(&r, "hero").is_null());
    let chest = call(&mut r, "inspect", json!({"id":"chest"})).unwrap();
    assert_eq!(chest["uses"], 1);
    let well = call(&mut r, "inspect", json!({"id":"well"})).unwrap();
    assert!(
        well["cooldown_ticks_left"].as_u64().unwrap() > 1000,
        "{well}"
    );
    assert_eq!(summary(&r)["used"], 2);
    // Focus is kept by ID across a reload that changes the target.
    let w = world(json!([
        target("npc", [0.0; 3], talk()),
        mover("hero", [0.5, 0.0, 0.0], &["player"]),
    ]));
    let mut r = runtime(w.clone());
    step(&mut r);
    let mut wider = w;
    wider.entities[0]
        .components
        .get_mut("interact.target")
        .unwrap()["range"] = json!(3.0);
    r.reconcile(wider, 2).unwrap();
    assert_eq!(prompt(&r), "[E] Interact", "prompt republished on reload");
    press(&mut r, "interact");
    assert_eq!(var(&r, "talks"), 1);
}

#[test]
fn checkpoints_refuse_and_spawned_targets_halt_explicitly() {
    let mut w = world(json!([
        target("npc", [0.0; 3], talk()),
        mover("hero", [0.5, 0.0, 0.0], &["player"])
    ]));
    w.capabilities.insert("save".into(), 1);
    let mut r = runtime(w);
    let error = format!("{:#}", r.checkpoint().unwrap_err());
    assert!(
        error.contains("interact.focus does not implement checkpoints"),
        "{error}"
    );
    // A spawned interactor becomes a candidate; a spawned target halts.
    step(&mut r);
    let hero = r.state.compiled.entities[r.state.compiled.indices["hero"]].clone();
    r.state
        .spawn(&hero, "hero/2".into(), [0.0, 0.0, 0.5])
        .unwrap();
    step(&mut r);
    assert_eq!(summary(&r)["focus"]["hero/2"], "npc");
    let npc = r.state.compiled.entities[r.state.compiled.indices["npc"]].clone();
    r.state.spawn(&npc, "npc/2".into(), [0.0; 3]).unwrap();
    step(&mut r);
    let halted = summary(&r)["halted"].as_str().unwrap().to_owned();
    assert!(
        halted.contains("spawned interactables are not supported"),
        "{halted}"
    );
    let error = format!("{:#}", call(&mut r, "use", json!({})).unwrap_err());
    assert!(error.contains("halted"), "{error}");
}

#[test]
fn authoring_errors_are_recoverable_diagnostics() {
    let base = || world(json!([target("npc", [0.0; 3], talk())]));
    let mut w = base();
    w.systems.retain(|s| s != "interact.focus");
    assert!(compile_error(&w).contains("needs the interact.focus system"));
    let mut w = base();
    w.entities[0].components.get_mut("interact.target").unwrap()["event"] = json!("missing");
    let e = compile_error(&w);
    assert!(
        e.contains("invalid_interact") && e.contains("does not declare"),
        "{e}"
    );
    let mut w = base();
    w.resources.get_mut("behavior.program").unwrap()["events"]["talk"]["fields"]["extra"] =
        json!("int");
    assert!(compile_error(&w).contains("may only declare text fields `interactor` and `target`"));
    let mut w = base();
    w.entities[0].components.get_mut("interact.target").unwrap()["action"] = json!("jump");
    let e = compile_error(&w);
    assert!(e.contains("/action") && e.contains("no button jump"), "{e}");
    let mut w = base();
    w.entities[0].components.get_mut("interact.target").unwrap()["range"] = json!(-1);
    assert!(compile_error(&w).contains("range must be"));
    let mut w = base();
    w.entities[0].components.remove("core.transform");
    assert!(compile_error(&w).contains("requires core.transform"));
    let mut w = base();
    w.capabilities.remove("interact");
    assert!(compile_error(&w).contains("interact"));
    // Budget: 257 targets.
    let many: Vec<Value> = (0..257)
        .map(|i| target(&format!("t/{i}"), [i as f32, 0.0, 0.0], json!({})))
        .collect();
    let e = compile_error(&world(Value::Array(many)));
    assert!(e.contains("interact_budget"), "{e}");
}

#[test]
fn pair_budget_fails_explicitly_at_build_and_on_spawn() {
    // 256 targets x 64 players = 16384 tests: the limit is accepted, one more is not.
    let mut entities: Vec<Value> = (0..256)
        .map(|i| target(&format!("t/{i}"), [i as f32 * 3.0, 0.0, 0.0], json!({})))
        .collect();
    entities.extend((0..64).map(|i| mover(&format!("p/{i}"), [i as f32, 0.0, 5.0], &["player"])));
    let mut r = runtime(world(Value::Array(entities.clone())));
    step(&mut r);
    assert_eq!(summary(&r)["pairs"], 16384);
    let p = r.state.compiled.entities[r.state.compiled.indices["p/0"]].clone();
    r.state.spawn(&p, "p/64".into(), [0.0; 3]).unwrap();
    step(&mut r);
    let halted = summary(&r)["halted"].as_str().unwrap().to_owned();
    assert!(halted.contains("candidate budget exceeded"), "{halted}");
    entities.push(mover("p/64", [0.0; 3], &["player"]));
    let error = WorldRuntime::new(world(Value::Array(entities)), 1, installed_registry())
        .err()
        .map(|e| format!("{e:#}"))
        .unwrap();
    assert!(error.contains("candidate budget exceeded"), "{error}");
}

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// The pre-interactable town: FSM greet/serve states set `nearby` and the `interact` input
/// event talks to whoever `nearby` names (rules copied from the previous world.json).
fn legacy_town() -> World {
    let mut w = town();
    w.systems.retain(|s| s != "interact.focus");
    w.capabilities.remove("interact");
    for id in ["npc/lumi", "npc/rin"] {
        w.entity_mut(id)
            .unwrap()
            .components
            .remove("interact.target");
    }
    let program = w.resources.get_mut("behavior.program").unwrap();
    program["variables"]["nearby"] = json!("none");
    program["events"]["interact"] = json!({"fields":{}});
    program["inputs"]["interact"] = json!("interact");
    let nearby =
        |v: &str| json!({"kind":"set","variable":"nearby","value":{"kind":"literal","value":v}});
    let talk = |who: &str| json!({"id":format!("talk-{who}"),"event":"interact","when":{"kind":"eq","left":{"kind":"variable","id":"nearby"},"right":{"kind":"literal","value":who}},"actions":[{"kind":"go_to","node":who}]});
    let rules = program["rules"].as_array_mut().unwrap();
    rules.retain(|r| !r["id"].as_str().unwrap().starts_with("talk-"));
    for r in rules.iter_mut() {
        if r["id"] == "lumi-left" || r["id"] == "rin-left" {
            r["actions"]
                .as_array_mut()
                .unwrap()
                .insert(0, nearby("none"));
        }
    }
    rules.push(json!({"id":"lumi-near","event":"near_lumi","actions":[nearby("lumi")]}));
    rules.push(json!({"id":"rin-near","event":"near_rin","actions":[nearby("rin")]}));
    rules.push(talk("lumi"));
    rules.push(talk("rin"));
    w
}

/// Walks the hero to Lumi, away, to Rin and away, tapping E every other tick the whole way.
/// Returns every narrative node entry (tick, node) and the hero / Lumi positions per tick.
fn talk_walk(world: World) -> (Vec<(u64, String)>, Vec<[f32; 3]>, Value) {
    let mut r = runtime(world);
    // (ticks, move_x, move_z); move_z = +1 walks toward -Z.
    let plan: [(u32, f32, f32); 8] = [
        (120, 1.0, 0.0),
        (250, 0.0, 1.0),
        (240, 0.0, 0.0),
        (200, 0.0, -1.0),
        (220, 1.0, 0.0),
        (470, 0.0, 1.0),
        (180, 0.0, 0.0),
        (240, 0.0, -1.0),
    ];
    let (mut nodes, mut poses) = (vec![], vec![]);
    let mut seen = 0;
    for (ticks, x, z) in plan {
        for _ in 0..ticks {
            let mut input = ActionMap::default();
            input.0.insert("move_x".into(), x);
            input.0.insert("move_z".into(), z);
            input.0.insert("interact".into(), (r.state.tick % 2) as f32);
            r.step(&input);
            let g = r.state.gameplay.as_ref().unwrap();
            for t in g.trace.iter().filter(|t| t.seq > seen) {
                if t.kind == "node" {
                    nodes.push((t.tick, t.id.clone()));
                }
            }
            seen = g.trace.back().map_or(seen, |t| t.seq);
            for id in ["hero", "npc/lumi", "npc/rin"] {
                poses.push(
                    r.state.transforms[r.state.compiled.indices[id]]
                        .as_ref()
                        .unwrap()
                        .position,
                );
            }
        }
    }
    let mut variables = r.observation().metrics["gameplay"]["variables"].clone();
    variables.as_object_mut().unwrap().remove("nearby");
    (nodes, poses, variables)
}

#[test]
fn axiom_town_interactables_match_the_fsm_nearby_workaround() {
    let (nodes, poses, variables) = talk_walk(town());
    let (old_nodes, old_poses, old_variables) = talk_walk(legacy_town());
    let visited: Vec<&str> = nodes.iter().map(|(_, n)| n.as_str()).collect();
    assert!(
        visited.contains(&"lumi") && visited.contains(&"rin"),
        "{nodes:?}"
    );
    let talks = |n: &str| visited.iter().filter(|v| **v == n).count();
    // Many taps landed while in range (each re-enters the node).
    assert!(talks("lumi") > 50 && talks("rin") > 50, "{nodes:?}");
    // Same simulation, same node entries on the same ticks, same variables.
    assert_eq!(poses, old_poses);
    assert_eq!(nodes, old_nodes);
    assert_eq!(variables, old_variables);
}

#[test]
fn review_regressions_reload_disable_halt_prompt_and_routed_input() {
    use axiom_runtime::world::RuntimeSystem;
    // Disabling the focused target by hot reload: no use, no stale prompt.
    let w = world(json!([
        target("npc", [0.0; 3], talk()),
        mover("hero", [0.5, 0.0, 0.0], &["player"])
    ]));
    let mut r = runtime(w.clone());
    step(&mut r);
    assert_eq!(prompt(&r), "[E] Interact");
    let mut off = w.clone();
    off.entities[0]
        .components
        .get_mut("interact.target")
        .unwrap()["enabled"] = json!(false);
    r.reconcile(off, 2).unwrap();
    assert_eq!(prompt(&r), "");
    let error = format!("{:#}", call(&mut r, "use", json!({})).unwrap_err());
    assert!(error.contains("no target in range"), "{error}");
    press(&mut r, "interact");
    assert_eq!(var(&r, "talks"), 0);
    // A halt clears the prompt.
    let mut r = runtime(w.clone());
    step(&mut r);
    assert_eq!(prompt(&r), "[E] Interact");
    let npc = r.state.compiled.entities[r.state.compiled.indices["npc"]].clone();
    r.state
        .spawn(&npc, "npc/2".into(), [9.0, 0.0, 0.0])
        .unwrap();
    step(&mut r);
    assert_eq!(prompt(&r), "");
    // Input routed away while the key is held: coming back with it still held is no press.
    let mut r = runtime(w);
    let mut system = axiom_runtime::interact::InteractSystem::new(&r.state).unwrap();
    let mut held = ActionMap::default();
    held.0.insert("interact".into(), 1.0);
    r.state.tick += 1;
    system.step(&mut r.state, &ActionMap::default());
    system.input_lost();
    r.state.tick += 1;
    system.step(&mut r.state, &ActionMap::default());
    r.state.tick += 1;
    system.step(&mut r.state, &held);
    assert_eq!(
        var(&r, "talks"),
        0,
        "held key after routing back is not a press"
    );
    r.state.tick += 1;
    system.step(&mut r.state, &ActionMap::default());
    r.state.tick += 1;
    system.step(&mut r.state, &held);
    assert_eq!(var(&r, "talks"), 1);
}
