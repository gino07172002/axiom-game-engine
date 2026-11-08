//! Generic features behind social/town content: event→native bindings, FSM entry clips,
//! wander and face actions. Uses the mascot pack's existing clips (idle, sway, …).
use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::world::{ActionMap, WorldRuntime};
use glam::{Quat, Vec3};
use serde_json::{json, Value};

fn mascot() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/mascot"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn run(r: &mut WorldRuntime, ticks: u32) {
    for _ in 0..ticks {
        r.step(&ActionMap::default());
    }
}

fn clip_of(r: &WorldRuntime, id: &str) -> Value {
    json!(r.observation().metrics["animation"]["players"][id]["clip"])
}

fn position(r: &WorldRuntime, id: &str) -> Vec3 {
    let slot = r.state.compiled.indices[id];
    Vec3::from_array(r.state.transforms[slot].as_ref().unwrap().position)
}

#[test]
fn gameplay_events_trigger_bound_native_actions() {
    let mut w = mascot();
    w.capabilities.insert("behavior".into(), 1);
    w.systems.push("behavior.execute".into());
    w.resources.insert(
        "behavior.program".into(),
        json!({"variables":{},"events":{"ping":{"fields":{}}},"rules":[]}),
    );
    w.resources.insert(
        "runtime.bindings".into(),
        json!({"events":[{"event":"ping","calls":[{"kind":"call","capability":"animation","action":"play","args":{"entity":"mascot/short","clip":"sway","looping":true}}]}]}),
    );
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 5);
    assert!(!clip_of(&r, "mascot/short").to_string().contains("sway"));
    r.command(&RuntimeCommand::Emit {
        event: "ping".into(),
        fields: Default::default(),
    })
    .unwrap();
    run(&mut r, 5);
    let clip = clip_of(&r, "mascot/short").to_string();
    assert!(
        clip.contains("sway"),
        "bound animation.play must run: {clip}"
    );
    let bindings = &r.observation().metrics["bindings"];
    assert_eq!(bindings["calls"], 1, "{bindings}");
    assert!(bindings["fault"].is_null());
}

#[test]
fn bindings_reject_non_call_commands_and_report_runtime_failures() {
    let registry = axiom_runtime::installed_registry();
    let mut looped = mascot();
    looped.resources.insert(
        "runtime.bindings".into(),
        json!({"events":[{"event":"ping","calls":[{"kind":"emit","event":"ping"}]}]}),
    );
    let error = format!("{:#}", looped.compile(&registry.authoring).unwrap_err());
    assert!(error.contains("only call commands"), "{error}");

    let mut w = mascot();
    w.capabilities.insert("behavior".into(), 1);
    w.systems.push("behavior.execute".into());
    w.resources.insert(
        "behavior.program".into(),
        json!({"variables":{},"events":{"ping":{"fields":{}}},"rules":[]}),
    );
    w.resources.insert(
        "runtime.bindings".into(),
        json!({"events":[{"event":"ping","calls":[{"kind":"call","capability":"animation","action":"play","args":{"entity":"mascot/short","clip":"no-such-clip"}}]}]}),
    );
    let mut r = WorldRuntime::new(w, 1, registry).unwrap();
    r.command(&RuntimeCommand::Emit {
        event: "ping".into(),
        fields: Default::default(),
    })
    .unwrap();
    let fault = r.observation().metrics["bindings"]["fault"].clone();
    assert!(
        fault.as_str().is_some_and(|f| f.starts_with("ping:")),
        "{fault}"
    );
}

fn with_machine(machine: Value) -> World {
    let mut w = mascot();
    w.capabilities.insert("fsm".into(), 1);
    w.systems.push("fsm.step".into());
    w.resources
        .insert("fsm.machines".into(), json!({"machines":{"m":machine}}));
    let long = w.entity_mut("mascot/long").unwrap();
    long.components
        .insert("fsm.agent".into(), json!({"machine":"m"}));
    w.entity_mut("mascot/short")
        .unwrap()
        .tags
        .push("player".into());
    w
}

#[test]
fn fsm_entry_clips_and_wander_stay_near_home() {
    let w = with_machine(json!({"initial":"stroll","states":{
        "stroll":{"action":{"kind":"wander","radius":2.0,"speed":1.0},"clip":"sway"}
    }}));
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    let home = position(&r, "mascot/long");
    let mut farthest: f32 = 0.0;
    for _ in 0..12 {
        run(&mut r, 100);
        let p = position(&r, "mascot/long");
        farthest = farthest.max(Vec3::new(p.x - home.x, 0.0, p.z - home.z).length());
    }
    assert!(farthest > 0.3, "wander must move: {farthest}");
    assert!(
        farthest <= 2.05,
        "wander stays within its radius: {farthest}"
    );
    assert!(clip_of(&r, "mascot/long").to_string().contains("sway"));
    let summary = r
        .command(&RuntimeCommand::Call {
            capability: "fsm".into(),
            action: "summary".into(),
            args: json!({}),
        })
        .unwrap();
    assert!(summary["result"]["fault"].is_null(), "{summary}");
}

#[test]
fn face_turns_toward_the_target_and_unknown_clips_are_reported() {
    let w = with_machine(json!({"initial":"look","states":{
        "look":{"action":{"kind":"face","tag":"player"},"clip":"no-such-clip"}
    }}));
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    run(&mut r, 2);
    let me = position(&r, "mascot/long");
    let target = position(&r, "mascot/short");
    let slot = r.state.compiled.indices["mascot/long"];
    let forward = Quat::from_array(r.state.transforms[slot].as_ref().unwrap().rotation) * Vec3::Z;
    let want = Vec3::new(target.x - me.x, 0.0, target.z - me.z).normalize();
    assert!(forward.dot(want) > 0.999, "{forward:?} vs {want:?}");
    let summary = r
        .command(&RuntimeCommand::Call {
            capability: "fsm".into(),
            action: "summary".into(),
            args: json!({}),
        })
        .unwrap();
    assert!(summary["result"]["fault"]
        .as_str()
        .is_some_and(|f| f.contains("no-such-clip")));
}
