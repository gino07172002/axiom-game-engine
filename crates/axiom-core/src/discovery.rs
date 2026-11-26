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
    let predicate = record(
        json!({"field":{"type":"string","description":"JSON pointer into the record, e.g. /components/core.health/value"},"op":{"enum":["eq","ne","lt","le","gt","ge"]},"value":{}}),
        &["field", "op", "value"],
    );
    use crate::local::{MAX_SELECT_LIMIT, MAX_SET_WHERE_BYTES, MAX_SET_WHERE_MATCHES};
    // Matching is shared by reads and `set_where`; projection, paging and the match limit are
    // not (a `set_where` selector takes no cursor or fields and caps matches higher).
    let mut matching = json!({"from":{"enum":["entities","resources","prefabs","scenes","nodes","choices","rules","widgets","variables"]},"ids":{"type":"array","items":{"type":"string"},"maxItems":128},"id_glob":{"type":["string","null"],"description":"* any run, ? one char, e.g. enemy/*"},"where":{"type":"array","maxItems":16,"items":predicate,"description":"AND of clauses; entity records include prefab-inherited components; missing field never matches"},"tags":strings,"components":strings,"parent":{"type":["string","null"]}});
    let mut read = matching.clone();
    read.as_object_mut().unwrap().extend(
        json!({"format":{"enum":["json","lines"],"description":"lines returns {text:'id field=value...\\nmatched=N rev=R'}"},"fields":strings,"cursor":{"type":["string","null"]},"limit":{"type":["integer","null"],"minimum":1,"maximum":MAX_SELECT_LIMIT},"max_bytes":{"type":["integer","null"],"minimum":256,"maximum":65536}})
            .as_object()
            .unwrap()
            .clone(),
    );
    matching["limit"] = json!({"type":["integer","null"],"minimum":1,"maximum":MAX_SET_WHERE_MATCHES,"description":format!("caps matches (default {MAX_SET_WHERE_MATCHES}); more matches fail")});
    let selection = record(read, &["from"]);
    let edit_selection = record(matching, &["from"]);
    let target = json!({"oneOf":targets});
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$defs":{"target":target,"selection":selection,"edit_selection":edit_selection},"oneOf":[record(json!({"op":{"const":"select"},"query":{"$ref":"#/$defs/selection"}}),&["op","query"]),tagged("set_field",json!({"target":{"$ref":"#/$defs/target"},"field":{"type":"string","description":"JSON pointer relative to the selected object; plain root field name also accepted"},"value":{}}),&["target","field","value"]),tagged("set_where",json!({"select":{"$ref":"#/$defs/edit_selection","description":format!("zero matches fails; matches x value bytes at most {MAX_SET_WHERE_BYTES}")},"field":{"type":"string"},"value":{}}),&["select","field","value"]),tagged("put_object",json!({"target":{"$ref":"#/$defs/target"},"value":{}}),&["target","value"]),tagged("remove_object",json!({"target":{"$ref":"#/$defs/target"}}),&["target"])],"description":"Edit variants are members of an apply/preview edits array. Select uses stable IDs, id_glob and where; empty fields return metadata only. Cursor is bound to revision and selection. set_where edits every match in one revision."})
}
pub fn gameplay_schema() -> Value {
    let operand = json!({"oneOf":[tagged("literal",json!({"value":{"type":["boolean","integer","number","string"]}}),&["value"]),tagged("variable",json!({"id":{"type":"string"}}),&["id"]),tagged("event",json!({"field":{"type":"string"}}),&["field"]),tagged("animation",json!({"entity":{"type":"string"},"field":{"enum":["clip","time","normalized","speed","playing","finished","state","param"]},"param":{"type":"string","description":"controller parameter (field param only)"}}),&["entity","field"])]});
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
    actions.push(tagged(
        "call",
        json!({"capability":{"type":"string"},"action":{"type":"string"},"args":{"type":"object","description":"default {}"}}),
        &["capability", "action"],
    ));
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
    // A batch holds the plain commands only (no nested batch or slide), as the runtime checks.
    let plain = json!({"oneOf":[tagged("choose",json!({"node":{"type":"string"},"choice":{"type":"string"}}),&["node","choice"]),tagged("emit",json!({"event":{"type":"string"},"fields":{"type":"object","additionalProperties":{"type":["boolean","integer","number","string"]}}}),&["event"]),tagged("locale",json!({"locale":{"type":"string"}}),&["locale"]),tagged("call",json!({"capability":{"type":"string"},"action":{"type":"string"},"args":{"type":"object"}}),&["capability","action","args"]),tagged("open",json!({"page":{"type":"string","description":"ui.document page (plan M6), opened on top"}}),&["page"]),tagged("close",json!({"page":{"type":"string","description":"open page to close; omit for the top one"}}),&[])]});
    let mut command = plain.clone();
    let commands = command["oneOf"].as_array_mut().unwrap();
    commands.push(tagged("batch",json!({"commands":{"type":"array","minItems":1,"maxItems":crate::gameplay::MAX_BATCH,"items":{"$ref":"#/$defs/batch_command"},"description":"run in order in one tick, all validated first; no nested batch or slide"}}),&["commands"]));
    commands.push(tagged(
        "slide",
        json!({"widget":{"type":"string"},"value":{"type":"number"}}),
        &["widget", "value"],
    ));
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$defs":{"operand":operand,"condition":condition,"action":{"oneOf":actions},"choice":choice,"node":node,"graph":graph,"command":command,"batch_command":plain},"$ref":"#/$defs/graph","description":"Graph resource schema. Shared condition/action/command schemas are available through $defs. Runtime also validates types, references and budgets."})
}
