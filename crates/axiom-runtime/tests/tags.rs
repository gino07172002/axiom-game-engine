//! Interned tags at runtime: the compiled world's tag index stays one set per entity slot,
//! and a template compiled in another world (other tag ids) is re-interned by name when it
//! is spawned, so FSM conditions and trigger filters see tags that no authored entity had.
use axiom_core::{gameplay::RuntimeCommand, World};
use axiom_runtime::{installed_registry, trigger::TriggerSystem, ActionMap, WorldRuntime};
use serde_json::{json, Value};

fn world() -> World {
    serde_json::from_value(json!({
        "schema_version":2,"id":"tags","title":"Tags",
        "capabilities":{"core":1,"fsm":1,"trigger":1},
        "systems":["fsm.step","trigger.detect"],
        "entities":[
            {"id":"guard","tags":["npc"],"components":{"core.transform":{"position":[0,0,0]},"fsm.agent":{"machine":"m"}}},
            {"id":"zone","components":{"core.transform":{"position":[5,0,0]},
                "trigger.zone":{"shape":"sphere","radius":1.0,"filter":{"tags":["beacon","lit"]}}}}
        ],
        "resources":{"fsm.machines":{"machines":{"m":{"initial":"idle",
            "states":{"idle":{"action":{"kind":"stop"}},"alert":{"action":{"kind":"stop"}}},
            "transitions":[{"from":"idle","to":"alert","when":{"kind":"near","tag":"beacon","within":10}}]}}}}
    }))
    .unwrap()
}

fn inspect(r: &mut WorldRuntime) -> Value {
    r.command(&RuntimeCommand::Call {
        capability: "fsm".into(),
        action: "inspect".into(),
        args: json!({"id":"guard"}),
    })
    .unwrap()["result"]
        .clone()
}

#[test]
fn spawned_templates_from_another_compilation_are_interned_by_name() {
    let mut r = WorldRuntime::new(world(), 1, installed_registry()).unwrap();
    let index = &r.state.compiled.tag_index;
    assert_eq!((index.len(), index.slots()), (1, 2));
    assert_eq!(index.id("beacon"), None);
    r.step(&ActionMap::default());
    assert!(inspect(&mut r)["transitions"][0]["measured"].is_null());
    // A template whose world interns other tags first: its ids mean nothing here.
    let mut other = world();
    other.entities.insert(
        0,
        serde_json::from_value(
            json!({"id":"t","tags":["x","y","lit","beacon"],"components":{"core.transform":{}}}),
        )
        .unwrap(),
    );
    let compiled = other.compile(&installed_registry().authoring).unwrap();
    assert_eq!(compiled.tag_index.id("beacon"), Some(3));
    let template = compiled.entities[0].clone();
    r.state
        .spawn(&template, "beacon/1".into(), [5.0, 0.0, 0.0])
        .unwrap();
    r.step(&ActionMap::default());
    r.assert_valid().unwrap();
    let index = &r.state.compiled.tag_index;
    assert_eq!(index.slots(), 3);
    let beacon = index.id("beacon").unwrap();
    assert!(index.has(2, beacon) && !index.has(0, beacon));
    // FSM `near(beacon)` measures the spawned entity; the trigger filter (both tags) holds it.
    let i = inspect(&mut r);
    assert_eq!(i["state"], "alert", "{i}");
    let trigger = r.system::<TriggerSystem>().unwrap();
    assert_eq!(trigger.occupants("zone").unwrap(), ["beacon/1"]);
    // Without one of the filter's tags the trigger ignores it.
    let mut partial = template.clone();
    partial.tags = vec!["beacon".into()];
    r.state
        .spawn(&partial, "beacon/2".into(), [5.0, 0.0, 0.2])
        .unwrap();
    r.step(&ActionMap::default());
    let trigger = r.system::<TriggerSystem>().unwrap();
    assert_eq!(trigger.occupants("zone").unwrap(), ["beacon/1"]);
}
