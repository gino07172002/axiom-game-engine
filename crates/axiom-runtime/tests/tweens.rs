//! Property tweens (`tween.clips` + `tween.animate`): easing through the runtime, once / loop /
//! ping_pong, reverse, seek, stop, on_finish events, interact -> rules -> bindings -> play,
//! child / light / mesh / variable tracks, authoring errors (conflicts, budgets), hot reload,
//! spawns and checkpoints.
use axiom_core::{gameplay::RuntimeCommand, Diagnostic, World};
use axiom_runtime::{installed_registry, ActionMap, WorldRuntime};
use glam::{Quat, Vec3};
use serde_json::{json, Value};

/// A door (hinge `door`, leaf child `door/leaf`) that a player can use, with the
/// interact -> rule -> runtime.bindings -> tween.play chain, plus extra entities.
fn world(extra: Value) -> World {
    let mut entities = vec![
        json!({"id":"door","components":{
            "core.transform":{"position":[0,0,0]},
            "interact.target":{"range":2.0,"event":"use_door"},
            "tween.clips":{"clips":{"swing":{"on_finish":"door_moved","tracks":[
                {"property":"core.transform/yaw","keys":[{"t":0,"v":0},{"t":1,"v":90}]}]}}}}}),
        json!({"id":"door/leaf","components":{"core.transform":{"position":[0.5,1,0]},
            "render.mesh":{"parts":[{"color":[0.5,0.3,0.2]}]}}}),
        json!({"id":"hero","tags":["player"],"components":{"core.transform":{"position":[1.5,0,0]}}}),
    ];
    entities.extend(extra.as_array().cloned().unwrap_or_default());
    serde_json::from_value(json!({
        "schema_version":2,"id":"tween-test","title":"Tween test",
        "capabilities":{"core":1,"render":1,"tween":1,"interact":1,"behavior":1},
        "systems":["behavior.execute","interact.focus","tween.animate"],
        "entities":entities,
        "relations":[{"kind":"parent","source":"door/leaf","target":"door"}],
        "resources":{
            "input.bindings":{"buttons":{"interact":"KeyE"}},
            "behavior.program":{
                "variables":{"open":false,"moves":0,"last_entity":"","last_clip":"","glow":0.0,"level":0},
                "events":{
                    "use_door":{"fields":{}},
                    "open_door":{"fields":{}},
                    "close_door":{"fields":{}},
                    "door_moved":{"fields":{"entity":"text","clip":"text"}},
                    "done":{"fields":{}},
                    "bad":{"fields":{"who":"text"}}
                },
                "rules":[
                    {"id":"want-open","event":"use_door","when":{"kind":"eq","left":{"kind":"variable","id":"open"},"right":{"kind":"literal","value":false}},
                     "actions":[{"kind":"emit","event":"open_door"}]},
                    {"id":"want-close","event":"use_door","when":{"kind":"eq","left":{"kind":"variable","id":"open"},"right":{"kind":"literal","value":true}},
                     "actions":[{"kind":"emit","event":"close_door"}]},
                    {"id":"opened","event":"open_door","actions":[{"kind":"set","variable":"open","value":{"kind":"literal","value":true}}]},
                    {"id":"closed","event":"close_door","actions":[{"kind":"set","variable":"open","value":{"kind":"literal","value":false}}]},
                    {"id":"moved","event":"door_moved","actions":[
                        {"kind":"add","variable":"moves","value":{"kind":"literal","value":1}},
                        {"kind":"set","variable":"last_entity","value":{"kind":"event","field":"entity"}},
                        {"kind":"set","variable":"last_clip","value":{"kind":"event","field":"clip"}}]}
                ]
            },
            "runtime.bindings":{"events":[
                {"event":"open_door","calls":[{"kind":"call","capability":"tween","action":"play","args":{"id":"door","clip":"swing"}}]},
                {"event":"close_door","calls":[{"kind":"call","capability":"tween","action":"play","args":{"id":"door","clip":"swing","reverse":true}}]}
            ]}
        }
    }))
    .unwrap()
}

fn tweened(id: &str, clips: Value) -> Value {
    json!({"id":id,"components":{"core.transform":{},"tween.clips":{"clips":clips}}})
}

fn yaw_clip(extra: Value) -> Value {
    let mut clip =
        json!({"tracks":[{"property":"core.transform/yaw","keys":[{"t":0,"v":0},{"t":1,"v":90}]}]});
    clip.as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap_or_default());
    clip
}

fn runtime(w: World) -> WorldRuntime {
    WorldRuntime::new(w, 1, installed_registry()).unwrap()
}

fn steps(r: &mut WorldRuntime, n: usize) {
    for _ in 0..n {
        r.step(&ActionMap::default());
    }
}

fn press(r: &mut WorldRuntime) {
    let mut input = ActionMap::default();
    input.0.insert("interact".into(), 1.0);
    r.step(&input);
}

fn call(r: &mut WorldRuntime, action: &str, args: Value) -> anyhow::Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "tween".into(),
        action: action.into(),
        args,
    })
    .map(|v| v["result"].clone())
}

fn yaw(r: &WorldRuntime, id: &str) -> f32 {
    let t = r.state.transforms[r.state.compiled.indices[id]]
        .as_ref()
        .unwrap();
    axiom_core::tween::yaw_degrees(Quat::from_array(t.rotation))
}

fn var(r: &WorldRuntime, name: &str) -> Value {
    r.observation().metrics["gameplay"]["variables"][name].clone()
}

fn summary(r: &WorldRuntime) -> Value {
    r.observation().metrics["tween"].clone()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn code(w: &World) -> (String, String) {
    let e = w.compile(&installed_registry().authoring).unwrap_err();
    let d = e
        .downcast_ref::<Diagnostic>()
        .unwrap_or_else(|| panic!("unstructured error: {e:#}"));
    (d.code.clone(), d.message.clone())
}

#[test]
fn play_stop_seek_and_inspect() {
    let mut r = runtime(world(json!([])));
    assert_eq!(summary(&r)["at"], json!({}), "nothing plays before play");
    let played = call(&mut r, "play", json!({"id":"door","clip":"swing"})).unwrap();
    assert_eq!(played["state"], "playing");
    steps(&mut r, 60);
    assert!(close(yaw(&r, "door"), 45.0), "{}", yaw(&r, "door"));
    assert_eq!(summary(&r)["at"]["door"], json!(["swing", 0.5, "playing"]));
    call(&mut r, "stop", json!({"id":"door"})).unwrap();
    steps(&mut r, 30);
    assert!(close(yaw(&r, "door"), 45.0), "stop holds the pose");
    let seek = call(&mut r, "seek", json!({"id":"door","time":0.25})).unwrap();
    assert_eq!(seek["state"], "stopped");
    assert!(close(yaw(&r, "door"), 22.5), "seek writes at once");
    let inspect = call(&mut r, "inspect", json!({"id":"door"})).unwrap();
    assert_eq!(inspect["clip"], "swing");
    assert_eq!(inspect["length"], 1.0);
    assert_eq!(
        inspect["clips"][0]["tracks"],
        json!([["", "core.transform/yaw", 2]])
    );
    // Recoverable request errors.
    let e = format!(
        "{:#}",
        call(&mut r, "play", json!({"id":"door","clip":"nope"})).unwrap_err()
    );
    assert!(e.contains("no clip nope (clips: swing)"), "{e}");
    assert!(call(&mut r, "seek", json!({"id":"door","time":2})).is_err());
    assert!(call(&mut r, "stop", json!({"id":"hero"})).is_err());
    assert!(call(&mut r, "play", json!({"id":"door"})).is_err());
}

#[test]
fn easing_shapes_runtime_values() {
    let clips = |ease: &str| {
        json!({"a":{"autoplay":true,"tracks":[{"property":"core.transform/position",
            "keys":[{"t":0,"v":[0,0,0]},{"t":1,"v":[4,0,0],"ease":ease}]}]}})
    };
    let mut r = runtime(world(json!([
        tweened("linear", clips("linear")),
        tweened("in", clips("ease_in")),
        tweened("out", clips("ease_out")),
        tweened("inout", clips("ease_in_out")),
        tweened("step", clips("step")),
    ])));
    steps(&mut r, 60);
    let x = |r: &WorldRuntime, id: &str| {
        r.state.transforms[r.state.compiled.indices[id]]
            .as_ref()
            .unwrap()
            .position[0]
    };
    assert!(close(x(&r, "linear"), 2.0));
    assert!(close(x(&r, "in"), 1.0), "{}", x(&r, "in"));
    assert!(close(x(&r, "out"), 3.0));
    assert!(close(x(&r, "inout"), 2.0));
    assert_eq!(x(&r, "step"), 0.0, "step holds until the key");
    steps(&mut r, 61);
    assert_eq!(x(&r, "step"), 4.0);
}

#[test]
fn once_loop_ping_pong_and_reverse() {
    let mut r = runtime(world(json!([
        tweened("once", json!({"a":yaw_clip(json!({"autoplay":true}))})),
        tweened(
            "loop",
            json!({"a":yaw_clip(json!({"autoplay":true,"mode":"loop"}))})
        ),
        tweened(
            "pong",
            json!({"a":yaw_clip(json!({"autoplay":true,"mode":"ping_pong"}))})
        ),
        tweened(
            "slow",
            json!({"a":yaw_clip(json!({"autoplay":true,"mode":"loop","speed":0.5}))})
        ),
    ])));
    steps(&mut r, 180); // 1.5 s
    let at = summary(&r)["at"].clone();
    assert_eq!(at["once"], json!(["a", 1.0, "stopped"]));
    assert_eq!(at["loop"], json!(["a", 0.5, "playing"]));
    assert_eq!(at["pong"], json!(["a", 0.5, "reversing"]));
    assert_eq!(at["slow"], json!(["a", 0.75, "playing"]));
    assert!(close(yaw(&r, "once"), 90.0));
    assert!(close(yaw(&r, "pong"), 45.0));
    // Replaying the finished once clip forward does nothing; reverse plays back to 0.
    call(&mut r, "play", json!({"id":"once","clip":"a"})).unwrap();
    steps(&mut r, 10);
    assert_eq!(summary(&r)["at"]["once"], json!(["a", 1.0, "stopped"]));
    let rev = call(
        &mut r,
        "play",
        json!({"id":"once","clip":"a","reverse":true}),
    )
    .unwrap();
    assert_eq!(rev["state"], "reversing");
    steps(&mut r, 30);
    assert!(close(yaw(&r, "once"), 67.5));
    steps(&mut r, 200);
    assert_eq!(summary(&r)["at"]["once"], json!(["a", 0.0, "stopped"]));
    assert!(close(yaw(&r, "once"), 0.0));
    // restart from the end of a reversed play.
    call(
        &mut r,
        "play",
        json!({"id":"once","clip":"a","reverse":true,"restart":true}),
    )
    .unwrap();
    assert!(
        close(yaw(&r, "once"), 90.0),
        "restart reversed jumps to the end pose"
    );
}

#[test]
fn interact_plays_open_and_close_through_rules_and_bindings() {
    let mut r = runtime(world(json!([])));
    steps(&mut r, 1); // focus
    press(&mut r);
    assert_eq!(var(&r, "open"), true);
    steps(&mut r, 130);
    assert!(close(yaw(&r, "door"), 90.0), "{}", yaw(&r, "door"));
    // on_finish reached the rule with the payload.
    assert_eq!(var(&r, "moves"), 1);
    assert_eq!(var(&r, "last_entity"), "door");
    assert_eq!(var(&r, "last_clip"), "swing");
    assert_eq!(summary(&r)["emitted"], 1);
    // The leaf (child) swings with the hinge: world position rotated by 90 degrees.
    let m = r.state.matrices()[r.state.compiled.indices["door/leaf"]];
    let p = m.transform_point3(Vec3::ZERO);
    assert!((p - Vec3::new(0.0, 1.0, -0.5)).length() < 1e-3, "{p}");
    // Press again (release first): closes by reversing the same clip.
    steps(&mut r, 1);
    press(&mut r);
    assert_eq!(var(&r, "open"), false);
    steps(&mut r, 60);
    assert!(close(yaw(&r, "door"), 45.0), "{}", yaw(&r, "door"));
    // Interrupt mid-swing: the door turns around from where it is (no jump).
    steps(&mut r, 1);
    press(&mut r);
    let before = yaw(&r, "door");
    steps(&mut r, 1);
    assert!(yaw(&r, "door") > before && (yaw(&r, "door") - before) < 1.0);
    steps(&mut r, 200);
    assert!(close(yaw(&r, "door"), 90.0));
    assert_eq!(var(&r, "moves"), 2, "the interrupted close never finished");
}

#[test]
fn child_light_mesh_and_variable_tracks() {
    let lamp = json!([
        {"id":"signal","components":{"core.transform":{},"tween.clips":{"clips":{"cycle":{"autoplay":true,"mode":"loop","duration":2,"tracks":[
            {"target":"signal/lamp","property":"render.light/color","keys":[{"t":0,"v":[1,0,0]},{"t":1,"v":[0,1,0],"ease":"step"}]},
            {"target":"signal/lamp","property":"render.light/intensity","keys":[{"t":0,"v":0},{"t":1,"v":4}]},
            {"target":"signal/lens","property":"render.mesh/color","keys":[{"t":0,"v":[0,0,0]},{"t":1,"v":[1,1,0]}]},
            {"target":"signal/lens","property":"render.mesh/emission","keys":[{"t":0,"v":0},{"t":1,"v":2}]},
            {"target":"signal/lens","property":"core.transform/scale","keys":[{"t":0,"v":[1,1,1]},{"t":1,"v":[2,2,2]}]},
            {"property":"behavior.program/variables/glow","keys":[{"t":0,"v":0},{"t":1,"v":1}]},
            {"property":"behavior.program/variables/level","keys":[{"t":0,"v":0},{"t":1,"v":10}]}
        ]}}}}},
        {"id":"signal/lamp","components":{"core.transform":{},"render.light":{"kind":"point","range":5,"color":[1,1,1],"intensity":1}}},
        {"id":"signal/lens","components":{"core.transform":{},"render.mesh":{"parts":[{"color":[0,0,0]},{"color":[0,0,0]}]}}}
    ]);
    let mut w = world(lamp);
    for child in ["signal/lamp", "signal/lens"] {
        w.relations.push(
            serde_json::from_value(json!({"kind":"parent","source":child,"target":"signal"}))
                .unwrap(),
        );
    }
    let mut r = runtime(w);
    // Autoplay writes nothing before the first step (edit-mode runtimes are never stepped).
    let light = |r: &WorldRuntime| {
        r.state.compiled.entities[r.state.compiled.indices["signal/lamp"]]
            .light
            .clone()
            .unwrap()
    };
    assert_eq!(light(&r).color, [1.0, 1.0, 1.0]);
    assert_eq!(
        summary(&r)["at"]["signal"],
        json!(["cycle", 0.0, "playing"])
    );
    steps(&mut r, 60);
    assert_eq!(light(&r).color, [1.0, 0.0, 0.0], "step colour holds");
    assert!(close(light(&r).intensity, 2.0));
    let lens = r.state.compiled.indices["signal/lens"];
    let mesh = r.state.compiled.entities[lens].mesh.clone().unwrap();
    assert!(mesh
        .parts
        .iter()
        .all(|p| close(p.color[0], 0.5) && close(p.emission, 1.0)));
    assert!(close(
        r.state.transforms[lens].as_ref().unwrap().scale[1],
        1.5
    ));
    assert!((var(&r, "glow").as_f64().unwrap() - 0.5).abs() < 1e-4);
    assert_eq!(var(&r, "level"), 5, "integer variables are rounded");
    // The renderer sees the animated light and mesh.
    let frame = r.render_frame();
    let scene_light = frame
        .scene_lights
        .iter()
        .find(|l| l.id == "signal/lamp")
        .unwrap();
    assert!(close(scene_light.light.intensity, 2.0));
    steps(&mut r, 70);
    assert_eq!(light(&r).color, [0.0, 1.0, 0.0]);
    assert!(close(light(&r).intensity, 4.0));
}

#[test]
fn authoring_errors_are_structured() {
    let with = |extra: Value| world(extra);
    // Target outside the hierarchy.
    let (c, m) = code(&with(json!([tweened(
        "x",
        json!({"a":{"tracks":[{"target":"hero","property":"core.transform/yaw","keys":[{"t":1,"v":1}]}]}})
    )])));
    assert_eq!(c, "invalid_tween");
    assert!(m.contains("not x or one of its descendants"), "{m}");
    // Missing component on the target.
    let (c, m) = code(&with(json!([tweened(
        "x",
        json!({"a":{"tracks":[{"property":"render.light/intensity","keys":[{"t":1,"v":1}]}]}})
    )])));
    assert_eq!(c, "invalid_tween");
    assert!(m.contains("lacks"), "{m}");
    // Mover conflicts: core.motion and path.follow / nav.agent on the animated entity.
    let mut mover = tweened("x", json!({"a":yaw_clip(json!({}))}));
    mover["components"]["core.motion"] = json!({"velocity":[1,0,0]});
    let mut w = with(json!([mover]));
    w.capabilities.insert("motion".into(), 1);
    w.systems.push("motion.integrate".into());
    let (c, m) = code(&w);
    assert_eq!(c, "tween_conflict");
    assert!(
        m.contains("core.motion") && m.contains("motion.integrate"),
        "{m}"
    );
    let mut follower = tweened("x", json!({"a":yaw_clip(json!({}))}));
    follower["components"]["path.follow"] = json!({"path":"road"});
    let mut w = with(json!([
        follower,
        {"id":"road","components":{"core.transform":{},"path.spline":{"points":[[0,0,0],[5,0,0]]}}}
    ]));
    w.capabilities.insert("path".into(), 1);
    w.systems.push("path.move".into());
    let (c, m) = code(&w);
    assert_eq!(c, "tween_conflict");
    assert!(m.contains("path.follow"), "{m}");
    // A light-only tween on a moving entity is fine.
    let mut lit = json!({"id":"x","components":{"core.transform":{},"core.motion":{"velocity":[1,0,0]},
        "render.light":{"kind":"point","range":3},
        "tween.clips":{"clips":{"a":{"tracks":[{"property":"render.light/intensity","keys":[{"t":1,"v":2}]}]}}}}});
    let mut w = with(json!([lit.clone()]));
    w.capabilities.insert("motion".into(), 1);
    w.systems.push("motion.integrate".into());
    w.compile(&installed_registry().authoring).unwrap();
    lit["components"]["tween.clips"]["clips"]["a"]["tracks"][0] =
        json!({"property":"core.transform/position","keys":[{"t":1,"v":[1,1,1]}]});
    let mut w = with(json!([lit]));
    w.capabilities.insert("motion".into(), 1);
    w.systems.push("motion.integrate".into());
    assert_eq!(code(&w).0, "tween_conflict");
    // Static obstacle below an animated transform.
    let mut w = with(json!([]));
    w.entities[1]
        .components
        .insert("nav.blocker".into(), json!({"half":[0.5,0.1]}));
    let (c, m) = code(&w);
    assert_eq!(c, "tween_conflict");
    assert!(m.contains("nav.blocker") && m.contains("door"), "{m}");
    // Two components animating the same property.
    let mut w = with(json!([tweened(
        "x",
        json!({"a":{"tracks":[{"target":"","property":"core.transform/scale","keys":[{"t":1,"v":[1,1,1]}]}]}})
    )]));
    w.entities[3].components.get_mut("tween.clips").unwrap()["clips"]["a"]["tracks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"property":"behavior.program/variables/glow","keys":[{"t":1,"v":1}]}));
    w.entities[0].components.get_mut("tween.clips").unwrap()["clips"]["swing"]["tracks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"property":"behavior.program/variables/glow","keys":[{"t":1,"v":1}]}));
    let (c, m) = code(&w);
    assert_eq!(c, "tween_conflict");
    assert!(m.contains("variable glow"), "{m}");
    // Undeclared variable, text variable, undeclared event, bad payload, missing system.
    let var_clip = |name: &str| json!({"a":{"tracks":[{"property":format!("behavior.program/variables/{name}"),"keys":[{"t":1,"v":1}]}]}});
    assert!(code(&with(json!([tweened("x", var_clip("nope"))])))
        .1
        .contains("declares no variable nope"));
    assert!(code(&with(json!([tweened("x", var_clip("last_clip"))])))
        .1
        .contains("not a number"));
    assert!(code(&with(json!([tweened(
        "x",
        json!({"a":yaw_clip(json!({"on_finish":"nope"}))})
    )])))
    .1
    .contains("does not declare"));
    assert!(code(&with(json!([tweened(
        "x",
        json!({"a":yaw_clip(json!({"on_finish":"bad"}))})
    )])))
    .1
    .contains("may only declare"));
    let mut w = with(json!([]));
    w.systems.retain(|s| s != "tween.animate");
    assert!(code(&w).1.contains("needs the tween.animate system"));
    // Component-level errors stay recoverable and name the field.
    let (c, m) = code(&with(json!([tweened(
        "x",
        json!({"a":{"tracks":[{"property":"core.transform/yaw","keys":[{"t":1,"v":[1,2,3]}]}]}})
    )])));
    assert_eq!(c, "invalid_component");
    assert!(m.contains("tween.clips") && m.contains("a number"), "{m}");
}

#[test]
fn budgets_are_explicit() {
    // The door has one too: 2048 more exceed the budget by one.
    let many: Vec<Value> = (0..axiom_core::tween::MAX_TWEENS)
        .map(|i| tweened(&format!("t/{i}"), json!({"a":yaw_clip(json!({}))})))
        .collect();
    let (c, m) = code(&world(Value::Array(many)));
    assert_eq!(c, "tween_budget");
    assert!(m.contains("2049 > 2048"), "{m}");
    // Keys: 16 tracks x 256 keys per clip, 16 clips per component = 65,536 keys each; three
    // such components (plus the door's 2 keys) exceed the world's key arrays.
    let keys: Vec<Value> = (0..256)
        .map(|k| json!({"t":k as f32 * 0.01,"v":[1,1,1]}))
        .collect();
    let clip = |c: usize| {
        let tracks: Vec<Value> = (0..16)
            .map(|t| json!({"target":format!("k{c}/{t}"),"property":"core.transform/scale","keys":keys}))
            .collect();
        json!({"tracks":tracks})
    };
    let mut entities = vec![];
    let mut w = world(json!([]));
    for owner in 0..3 {
        let clips: serde_json::Map<String, Value> =
            (0..16).map(|c| (format!("c{c}"), clip(owner))).collect();
        entities.push(json!({"id":format!("k{owner}"),"components":{"core.transform":{},"tween.clips":{"clips":clips}}}));
        for t in 0..16 {
            entities.push(json!({"id":format!("k{owner}/{t}"),"components":{"core.transform":{}}}));
            w.relations.push(
                serde_json::from_value(json!({"kind":"parent","source":format!("k{owner}/{t}"),"target":format!("k{owner}")}))
                    .unwrap(),
            );
        }
    }
    w.entities.extend(
        entities
            .into_iter()
            .map(|e| serde_json::from_value(e).unwrap()),
    );
    let (c, m) = code(&w);
    assert_eq!(c, "tween_budget");
    assert!(m.contains("key budget exceeded (196610 > 131072)"), "{m}");
    // Two such components alone are exactly at the budget (131,072 keys) and fit.
    w.entities.retain(|e| !e.id.starts_with("k2"));
    w.entities[0].components.remove("tween.clips");
    w.relations.retain(|r| !r.source.starts_with("k2"));
    w.compile(&installed_registry().authoring).unwrap();
}

#[test]
fn hot_reload_keeps_playback_unless_the_clip_changes() {
    let w = world(json!([]));
    let mut r = runtime(w.clone());
    call(&mut r, "play", json!({"id":"door","clip":"swing"})).unwrap();
    steps(&mut r, 30);
    let mut edited = w.clone();
    edited.entities.push(
        serde_json::from_value(json!({"id":"lamp","components":{"core.transform":{}}})).unwrap(),
    );
    r.reconcile(edited.clone(), 2).unwrap();
    assert!(close(yaw(&r, "door"), 22.5), "pose re-applied after reload");
    steps(&mut r, 30);
    assert_eq!(summary(&r)["at"]["door"], json!(["swing", 0.5, "playing"]));
    // Moving the hinge (authored transform) keeps the playback and the animated yaw.
    let mut moved = edited.clone();
    moved.entities[0]
        .components
        .insert("core.transform".into(), json!({"position":[0,0,1]}));
    r.reconcile(moved.clone(), 3).unwrap();
    assert!(close(yaw(&r, "door"), 45.0));
    // Changing the clip drops the playback (idle). The transform keeps its runtime pose, as
    // for every runtime-moved transform whose authored value did not change.
    let mut changed = moved.clone();
    changed.entities[0]
        .components
        .get_mut("tween.clips")
        .unwrap()["clips"]["swing"]["tracks"][0]["keys"][1]["v"] = json!(120);
    r.reconcile(changed, 4).unwrap();
    assert_eq!(summary(&r)["at"], json!({}));
    assert!(close(yaw(&r, "door"), 45.0));
}

#[test]
fn spawned_tweens_halt_and_checkpoints_refuse_explicitly() {
    let mut r = runtime(world(json!([tweened(
        "spinner",
        json!({"a":yaw_clip(json!({"autoplay":true,"mode":"loop"}))})
    )])));
    steps(&mut r, 1);
    let template = r.state.compiled.entities[r.state.compiled.indices["spinner"]].clone();
    r.state
        .spawn(&template, "spinner/2".into(), [0.0; 3])
        .unwrap();
    steps(&mut r, 1);
    assert!(summary(&r)["halted"]
        .as_str()
        .unwrap()
        .contains("spawned tweens are not supported"));
    assert!(call(&mut r, "stop", json!({"id":"spinner"})).is_err());
    let mut w = world(json!([]));
    w.capabilities.insert("save".into(), 1);
    // Only tween.animate (interact.focus refuses checkpoints too).
    w.systems.retain(|s| s != "interact.focus");
    w.entities[0].components.remove("interact.target");
    let error = format!("{:#}", runtime(w).checkpoint().unwrap_err());
    assert!(
        error.contains("tween.animate does not implement checkpoints"),
        "{error}"
    );
}

#[test]
fn many_finishes_wait_for_the_event_budget() {
    let n = axiom_core::tween::MAX_TWEEN_EVENTS_PER_STEP + 6;
    let many: Vec<Value> = (0..n)
        .map(|i| {
            tweened(
                &format!("t/{i}"),
                json!({"a":yaw_clip(json!({"autoplay":true,"on_finish":"door_moved"}))}),
            )
        })
        .collect();
    let mut r = runtime(world(Value::Array(many)));
    let mut guard = 0;
    while var(&r, "moves") == 0 {
        steps(&mut r, 1);
        guard += 1;
        assert!(guard < 200);
    }
    assert_eq!(var(&r, "moves"), 64, "64 events in the finishing step");
    assert_eq!(summary(&r)["queued"], 6);
    steps(&mut r, 1);
    assert_eq!(var(&r, "moves"), n as i64);
    assert_eq!(summary(&r)["fault"], Value::Null);
}

#[test]
fn scenario_uses_the_door_and_asserts_its_rotation() {
    let scenario: axiom_runtime::scenario::Scenario = serde_json::from_value(json!({
        "ticks":200,
        "commands":[{"tick":5,"operation":{"op":"command","command":{"kind":"call","capability":"interact","action":"use","args":{"interactor":"hero","target":"door"}}}}],
        "assertions":[
            {"kind":"field","entity":"door","field":"/rotation/1","op":"ge","value":0.7070},
            {"kind":"field","entity":"door","field":"/rotation/3","op":"le","value":0.7072},
            {"kind":"call","action":"tween.inspect","args":{"id":"door"},"pointer":"/state","op":"eq","value":"stopped"},
            {"kind":"equals","pointer":"/gameplay/variables/open","value":true}
        ]
    }))
    .unwrap();
    let out =
        axiom_runtime::scenario::run(world(json!([])), 1, installed_registry(), scenario).unwrap();
    assert_eq!(out["ok"], true, "{out}");
}
