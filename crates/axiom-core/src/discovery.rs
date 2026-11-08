//! Compact, cacheable JSON Schemas for the stable authoring surface.
use serde_json::{json, Value};
fn record(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn tagged(kind: &str, mut properties: Value, required: &[&str]) -> Value {
    properties["kind"] = json!({"const":kind});
    let mut names = required.to_vec();
    names.push("kind");
    record(properties, &names)
}
pub fn local_schema() -> Value {
    let mut targets = vec![];
    for kind in [
        "entity", "resource", "prefab", "scene", "node", "rule", "widget", "variable",
    ] {
        targets.push(tagged(
            kind,
            json!({"id":{"type":"string","minLength":1,"maxLength":128}}),
            &["id"],
        ));
    }
    targets.push(tagged(
        "choice",
        json!({"node":{"type":"string"},"id":{"type":"string"}}),
        &["node", "id"],
    ));
    let strings = json!({"type":"array","items":{"type":"string"},"maxItems":32});
    let selection = record(
        json!({"from":{"enum":["entities","resources","prefabs","scenes","nodes","choices","rules","widgets","variables"]},"ids":{"type":"array","items":{"type":"string"},"maxItems":128},"tags":strings,"components":strings,"parent":{"type":["string","null"]},"fields":strings,"cursor":{"type":["string","null"]},"limit":{"type":["integer","null"],"minimum":1,"maximum":128},"max_bytes":{"type":["integer","null"],"minimum":256,"maximum":65536}}),
        &["from"],
    );
    let target = json!({"oneOf":targets});
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$defs":{"target":target,"selection":selection},"oneOf":[record(json!({"op":{"const":"select"},"query":{"$ref":"#/$defs/selection"}}),&["op","query"]),tagged("set_field",json!({"target":{"$ref":"#/$defs/target"},"field":{"type":"string","description":"JSON pointer relative to the selected object; plain root field name also accepted"},"value":{}}),&["target","field","value"]),tagged("put_object",json!({"target":{"$ref":"#/$defs/target"},"value":{}}),&["target","value"]),tagged("remove_object",json!({"target":{"$ref":"#/$defs/target"}}),&["target"])],"description":"Edit variants are members of an apply/preview edits array. Select uses stable IDs; empty fields return metadata only. Cursor is bound to revision and selection."})
}
pub fn gameplay_schema() -> Value {
    let operand = json!({"oneOf":[tagged("literal",json!({"value":{"type":["boolean","integer","number","string"]}}),&["value"]),tagged("variable",json!({"id":{"type":"string"}}),&["id"]),tagged("event",json!({"field":{"type":"string"}}),&["field"])]});
    let read = json!({"$ref":"#/$defs/operand"});
    let test = json!({"$ref":"#/$defs/condition"});
    let condition = json!({"oneOf":[tagged("always",json!({}),&[]),tagged("eq",json!({"left":read,"right":read}),&["left","right"]),tagged("ge",json!({"left":read,"right":read}),&["left","right"]),tagged("all",json!({"conditions":{"type":"array","maxItems":32,"items":test}}),&["conditions"]),tagged("not",json!({"condition":test}),&["condition"])]});
    let mut actions = vec![];
    for kind in ["set", "add"] {
        actions.push(tagged(
            kind,
            json!({"variable":{"type":"string"},"value":read}),
            &["variable", "value"],
        ));
    }
    actions.push(tagged(
        "emit",
        json!({"event":{"type":"string"},"fields":{"type":"object","additionalProperties":read}}),
        &["event"],
    ));
    actions.push(tagged("delay",json!({"event":{"type":"string"},"fields":{"type":"object","additionalProperties":read},"ticks":{"type":"integer","minimum":1,"maximum":1000000}}),&["event","ticks"]));
    for (kind, field) in [("go_to", "node"), ("scene", "scene"), ("end", "ending")] {
        let mut p = json!({});
        p[field] = json!({"type":"string"});
        actions.push(tagged(kind, p, &[field]));
    }
    let action_list = json!({"type":"array","maxItems":64,"items":{"$ref":"#/$defs/action"}});
    let choice = record(
        json!({"id":{"type":"string"},"text":{"type":"string"},"when":test,"actions":action_list,"next":{"type":"string"}}),
        &["id", "text", "next"],
    );
    let node = record(
        json!({"id":{"type":"string"},"speaker":{"type":"string"},"text":{"type":"string"},"on_enter":action_list,"choices":{"type":"array","maxItems":16,"items":{"$ref":"#/$defs/choice"}},"ending":{"type":["string","null"]}}),
        &["id", "speaker", "text"],
    );
    let graph = record(
        json!({"start":{"type":"string"},"nodes":{"type":"array","minItems":1,"maxItems":256,"items":{"$ref":"#/$defs/node"}}}),
        &["start", "nodes"],
    );
    let command = json!({"oneOf":[tagged("choose",json!({"node":{"type":"string"},"choice":{"type":"string"}}),&["node","choice"]),tagged("emit",json!({"event":{"type":"string"},"fields":{"type":"object","additionalProperties":{"type":["boolean","integer","number","string"]}}}),&["event"]),tagged("locale",json!({"locale":{"type":"string"}}),&["locale"]),tagged("call",json!({"capability":{"type":"string"},"action":{"type":"string"},"args":{"type":"object"}}),&["capability","action","args"])]});
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$defs":{"operand":operand,"condition":condition,"action":{"oneOf":actions},"choice":choice,"node":node,"graph":graph,"command":command},"$ref":"#/$defs/graph","description":"Graph resource schema. Shared condition/action/command schemas are available through $defs. Runtime also validates types, references and budgets."})
}
