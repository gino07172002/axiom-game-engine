//! Typed schemas (a JSON-Schema subset) for every registered component and resource.
//!
//! The registry describes each type in one place: a short text per field ("meters 0..100,
//! default 2", "inverse | linear", "entity ID with path.spline") plus a validator. The typed
//! schema is *derived* from that text and from the type's default value, so a new component
//! gets typed fields without a second hand-written schema that could drift. A small table adds
//! what the text cannot say (which resource a reference points into). The editor Inspector
//! builds its forms from it; agents read it with `discover typed:ID` (never in the default
//! `discover` output, which stays compact).
//!
//! Keys used besides `type` / `description` / `enum` / `minimum` / `maximum` / `default` /
//! `properties` / `additionalProperties` / `items` / `minItems` / `maxItems` / `maxLength`:
//! `x-unit` (m, °, s, m/s, …), `x-widget` (`color`, `euler` = quaternion edited as Euler
//! degrees, `vec3`, `rect`), `x-ref` (`entity`, `event` = a `behavior.program` event,
//! `key:RESOURCE/POINTER` = a key of that object in that resource), `x-has` (component an
//! entity reference needs), `x-optional`.
use crate::*;
use serde_json::{json, Map, Value};

/// Description keys that document the type rather than name a field.
const META: [&str; 12] = [
    "requires", "budget", "conflict", "order", "observe", "edit", "pan", "output", "build",
    "default", "events", "input",
];

/// References the description text cannot name: (type, field) -> `x-ref` (+ `x-has`).
const REFS: [(&str, &str, &str, Option<&str>); 7] = [
    (
        "render.material",
        "id",
        "key:render.shaders/materials",
        None,
    ),
    ("audio.source", "clip", "key:audio.library/clips", None),
    ("fsm.agent", "machine", "key:fsm.machines/machines", None),
    (
        "animation.player",
        "asset",
        "key:animation.library/assets",
        None,
    ),
    (
        "interact.target",
        "action",
        "key:input.bindings/buttons",
        None,
    ),
    ("nav.grid", "pointer_agent", "entity", Some("nav.agent")),
    ("path.follow", "path", "entity", Some("path.spline")),
];

/// The typed schema of component or resource `id`: registered types from their description
/// and default value, world-defined record types from their field schemas. `None` for an
/// unknown type.
pub fn typed_schema(registry: &Registry, world: &World, id: &str) -> Option<Value> {
    if let Some(d) = registry
        .components
        .get(id)
        .or_else(|| registry.resources.get(id))
    {
        let sample = crate::ops::default_value(world, registry, id);
        if id == "tween.clips" {
            return Some(tween(&d.schema, sample.as_ref()));
        }
        return Some(object(id, &d.schema, sample.as_ref()));
    }
    world
        .component_schemas
        .get(id)
        .or_else(|| world.resource_schemas.get(id))
        .map(|s| record(id, s))
}

fn object(id: &str, descr: &Value, sample: Option<&Value>) -> Value {
    let descr = descr.as_object();
    let sample = sample.and_then(Value::as_object);
    let mut keys: Vec<&String> = sample.map(|s| s.keys().collect()).unwrap_or_default();
    for key in descr.into_iter().flat_map(Map::keys) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let mut properties = Map::new();
    for key in keys {
        let text = descr.and_then(|d| d.get(key)).and_then(Value::as_str);
        let value = sample.and_then(|s| s.get(key));
        if value.is_none() && (text.is_none() || META.contains(&key.as_str())) {
            continue;
        }
        properties.insert(key.clone(), field(id, key, text, value));
    }
    json!({"type":"object","x-id":id,"properties":properties})
}

/// `tween.clips` documents its nested shapes as `clip` / `track` next to `clips`.
fn tween(descr: &Value, sample: Option<&Value>) -> Value {
    let mut root = object("tween.clips", descr, sample);
    let mut clip = object("tween.clips", &descr["clip"], None);
    let track = object("tween.clips", &descr["track"], None);
    // "1..16 [{target, property, keys}]" is an item count.
    let tracks = &mut clip["properties"]["tracks"];
    if let Some(max) = tracks.as_object_mut().and_then(|t| t.remove("maximum")) {
        tracks["maxItems"] = max;
    }
    if let Some(t) = tracks.as_object_mut() {
        t.remove("minimum");
    }
    tracks["items"] = track;
    tracks["type"] = json!("array");
    let clips = &mut root["properties"]["clips"];
    clips["type"] = json!("object");
    clips["additionalProperties"] = clip;
    root
}

fn record(id: &str, schema: &RecordSchema) -> Value {
    let properties: Map<String, Value> = schema
        .fields
        .iter()
        .map(|(key, f)| {
            let mut node = Map::new();
            let ty = match f.ty {
                FieldType::Bool => "boolean",
                FieldType::Int => "integer",
                FieldType::Float => "number",
                FieldType::Vec3 => "array",
                FieldType::String | FieldType::Entity | FieldType::Asset => "string",
            };
            node.insert("type".into(), json!(ty));
            match f.ty {
                FieldType::Vec3 => {
                    node.insert("x-widget".into(), json!("vec3"));
                }
                FieldType::Entity => {
                    node.insert("x-ref".into(), json!("entity"));
                }
                _ => {}
            }
            if let Some(v) = f.min {
                node.insert("minimum".into(), json!(v));
            }
            if let Some(v) = f.max {
                node.insert("maximum".into(), json!(v));
            }
            if let Some(v) = &f.default {
                node.insert("default".into(), v.clone());
            }
            if f.optional {
                node.insert("x-optional".into(), json!(true));
            }
            (key.clone(), Value::Object(node))
        })
        .collect();
    json!({"type":"object","x-id":id,"properties":properties})
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn field(id: &str, key: &str, text: Option<&str>, value: Option<&Value>) -> Value {
    let mut node = Map::new();
    let mut ty = value.map(json_type).filter(|t| *t != "null");
    if let Some(text) = text {
        node.insert("description".into(), json!(text));
        let lower = text.to_ascii_lowercase();
        if ty.is_none() {
            ty = if lower.starts_with("bool") {
                Some("boolean")
            } else if lower.contains("integer") {
                Some("integer")
            } else if lower.contains("entity id")
                || lower.contains("behavior.program event")
                || enum_options(text).is_some()
            {
                Some("string")
            } else if text
                .find(['{', '['])
                .is_some_and(|i| text[i..].starts_with('{'))
            {
                // `{field, ...}` documents a nested object, not a number with a range.
                Some("object")
            } else if lower.starts_with("rgb") || lower.contains("vec3") || text.starts_with('[') {
                Some("array")
            } else if lower.contains(" chars") || lower.contains("text") {
                Some("string")
            } else if let Some((lo, hi)) = range(text) {
                // Integral bounds without a fractional unit: a count / tick / index.
                let fractional = [".", "meters", "seconds", "degrees", "m/s", " m ", "hz"]
                    .iter()
                    .any(|u| lower.contains(u));
                Some(if lo.fract() == 0.0 && hi.fract() == 0.0 && !fractional {
                    "integer"
                } else {
                    "number"
                })
            } else if lower.contains("number") {
                Some("number")
            } else {
                None
            };
        }
        let numeric = matches!(ty, Some("number" | "integer"));
        let array = ty == Some("array");
        let numbers = value
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty() && a.iter().all(Value::is_number))
            || (array && value.is_none());
        let color = lower.starts_with("rgb")
            || lower.starts_with("[r,g,b]")
            || (numbers && is_color_name(key));
        if let Some((lo, hi)) = range(text) {
            if numeric || numbers {
                node.insert("minimum".into(), json!(lo));
                node.insert("maximum".into(), json!(hi));
            } else if array {
                node.insert("minItems".into(), json!(lo));
                node.insert("maxItems".into(), json!(hi));
            } else if ty == Some("string") {
                node.insert("maxLength".into(), json!(hi));
            }
        } else if numeric {
            let (lo, hi) = bounds(&lower);
            if let Some(lo) = lo {
                node.insert("minimum".into(), json!(lo));
            }
            if let Some(hi) = hi {
                node.insert("maximum".into(), json!(hi));
            }
        }
        if matches!(ty, Some("string") | None) {
            if let Some(options) = enum_options(text) {
                node.insert("enum".into(), json!(options));
            }
        }
        if numeric || numbers {
            if let Some(unit) = unit(&lower) {
                node.insert("x-unit".into(), json!(unit));
            }
        }
        if let Some(default) = default(text) {
            node.insert("default".into(), default);
        }
        if lower.starts_with("optional") {
            node.insert("x-optional".into(), json!(true));
        }
        if lower.contains("unit quaternion") || lower.contains("quaternion xyzw") {
            node.insert("x-widget".into(), json!("euler"));
        } else if color {
            node.insert("x-widget".into(), json!("color"));
        } else if lower.starts_with("normalized [x,y,width,height]") {
            node.insert("x-widget".into(), json!("rect"));
        } else if numbers
            && (lower.starts_with("vec3")
                || lower.starts_with("positive vec3")
                || lower.contains("[x,y,z]"))
            && value.and_then(Value::as_array).is_none_or(|a| a.len() == 3)
        {
            node.insert("x-widget".into(), json!("vec3"));
        }
        if ty == Some("string") {
            if lower.contains("entity id") {
                node.insert("x-ref".into(), json!("entity"));
                if let Some(has) = lower
                    .split("entity id with ")
                    .nth(1)
                    .and_then(|r| r.split([' ', ',', ';', '(']).next())
                    .filter(|c| c.contains('.'))
                {
                    node.insert("x-has".into(), json!(has));
                }
            } else if lower.contains("behavior.program event") {
                node.insert("x-ref".into(), json!("event"));
            }
        }
    } else if value.is_some_and(|v| v.is_array()) && is_color_name(key) {
        node.insert("x-widget".into(), json!("color"));
    }
    if let Some((_, _, r, has)) = REFS.iter().find(|(t, f, _, _)| *t == id && *f == key) {
        node.insert("x-ref".into(), json!(r));
        if let Some(has) = has {
            node.insert("x-has".into(), json!(has));
        }
        ty.get_or_insert("string");
    }
    if let Some(items) = value.and_then(Value::as_array).and_then(|a| a.first()) {
        let item = json_type(items);
        if item != "object" && item != "array" {
            node.insert("items".into(), json!({"type": item}));
        }
    }
    if let Some(ty) = ty {
        node.insert("type".into(), json!(ty));
    }
    Value::Object(node)
}

fn is_color_name(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("color") || key.contains("tint") || key == "accent"
}

/// Splits at `|` outside brackets; `Some` only when every part starts with a lowercase
/// token (`linear (default) | catmull_rom (...)`, `"-z" | "+z"`).
fn enum_options(text: &str) -> Option<Vec<String>> {
    let mut parts = vec![];
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '|' if depth == 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    if parts.len() < 2 {
        return None;
    }
    let last = parts.len() - 1;
    let mut out = vec![];
    for (n, part) in parts.into_iter().enumerate() {
        let part = part.trim_start();
        let quoted = part.starts_with('"');
        let body = part.trim_start_matches('"');
        let end = body
            .find(|c: char| {
                !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '+'))
            })
            .unwrap_or(body.len());
        let token = &body[..end];
        let mut rest = &body[end..];
        if quoted {
            rest = rest.strip_prefix('"')?;
        }
        let follows = match rest.chars().next() {
            None => true,
            Some(c) => matches!(c, ' ' | '(' | ':' | ',' | ';' | ')') || (n == last && c == '.'),
        };
        if token.is_empty() || !follows {
            return None;
        }
        out.push(token.to_owned());
    }
    Some(out)
}

fn number_at(s: &str) -> Option<f64> {
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The first `LO..HI` with two numbers.
fn range(text: &str) -> Option<(f64, f64)> {
    let bytes = text.as_bytes();
    let numeric = |b: u8| b.is_ascii_digit() || b == b'.' || b == b'-';
    let mut from = 0;
    while let Some(at) = text[from..].find("..").map(|i| i + from) {
        from = at + 2;
        let mut lo = at;
        while lo > 0 && numeric(bytes[lo - 1]) {
            lo -= 1;
        }
        let mut hi = at + 2;
        while hi < bytes.len() && (numeric(bytes[hi]) && !text[hi..].starts_with("..")) {
            hi += 1;
        }
        let left = text[lo..at].trim_end_matches('.');
        let right = text[at + 2..hi].trim_end_matches('.');
        let letter_before = lo > 0 && bytes[lo - 1].is_ascii_alphabetic();
        if let (Some(a), Some(b), false) = (number_at(left), number_at(right), letter_before) {
            if a <= b {
                return Some((a, b));
            }
        }
    }
    None
}

/// `>= N` / `<= N` / `positive` bounds of a numeric field without a `LO..HI` range.
fn bounds(lower: &str) -> (Option<f64>, Option<f64>) {
    let after = |pat: &str| {
        lower.find(pat).and_then(|i| {
            let rest = lower[i + pat.len()..].trim_start();
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .unwrap_or(rest.len());
            number_at(rest[..end].trim_end_matches('.'))
        })
    };
    let lo = after(">=")
        .or_else(|| (lower.starts_with("positive") || lower.contains(" positive")).then_some(0.0));
    (lo, after("<="))
}

fn unit(lower: &str) -> Option<&'static str> {
    Some(if lower.contains("m/s²") {
        "m/s²"
    } else if lower.starts_with("m/s") {
        "m/s"
    } else if lower.contains("degrees/second") || lower.contains("deg/s") {
        "°/s"
    } else if lower.starts_with("degrees") || lower.starts_with("radians") {
        if lower.starts_with("radians") {
            "rad/s"
        } else {
            "°"
        }
    } else if lower.starts_with("seconds") || lower.starts_with("half-life seconds") {
        "s"
    } else if lower.starts_with("meters")
        || lower.starts_with("m ")
        || lower.starts_with("body radius m")
        || lower.starts_with("meters above")
    {
        "m"
    } else if lower.contains(" hz") && !lower.contains("tick") {
        "Hz"
    } else {
        return None;
    })
}

/// `default X` where X is a number, bool, quoted string or `[..]`.
fn default(text: &str) -> Option<Value> {
    let at = text.find("default ")? + "default ".len();
    let rest = text[at..].trim_start();
    if rest.starts_with('[') {
        let end = rest.find(']')?;
        return serde_json::from_str(&rest[..=end]).ok();
    }
    if let Some(quoted) = rest.strip_prefix('"') {
        return Some(json!(quoted.split('"').next()?));
    }
    let token = rest
        .split([',', ';', ')', ':', ' '])
        .next()?
        .trim_end_matches('.');
    match token {
        "true" => Some(json!(true)),
        "false" => Some(json!(false)),
        _ => serde_json::from_str::<serde_json::Number>(token)
            .ok()
            .map(Value::Number),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(id: &str) -> Value {
        typed_schema(&Registry::core(), &World::empty("t"), id).unwrap()
    }

    #[test]
    fn description_text_becomes_types_ranges_enums_units_and_refs() {
        let light = core("render.light");
        let p = &light["properties"];
        assert_eq!(
            p["kind"]["enum"],
            json!(["directional", "point", "spot", "area"])
        );
        assert_eq!(p["color"]["x-widget"], "color");
        assert_eq!(
            (
                p["intensity"]["minimum"].as_f64(),
                p["intensity"]["maximum"].as_f64()
            ),
            (Some(0.0), Some(100.0))
        );
        assert_eq!(p["range"]["x-unit"], "m");
        assert_eq!(p["enabled"]["type"], "boolean");
        let audio = core("audio.source");
        let p = &audio["properties"];
        assert_eq!(p["rolloff"]["enum"], json!(["inverse", "linear"]));
        assert_eq!(p["clip"]["x-ref"], "key:audio.library/clips");
        assert_eq!(p["priority"]["type"], "integer");
        assert_eq!(p["priority"]["minimum"], -1000.0);
        let follow = core("path.follow");
        assert_eq!(follow["properties"]["path"]["x-ref"], "entity");
        assert_eq!(follow["properties"]["path"]["x-has"], "path.spline");
        assert_eq!(
            follow["properties"]["end"]["enum"],
            json!(["loop", "ping_pong", "stop"])
        );
        assert_eq!(follow["properties"]["on_arrive"]["x-ref"], "event");
        let t = core("core.transform");
        assert_eq!(t["properties"]["rotation"]["x-widget"], "euler");
        assert_eq!(t["properties"]["position"]["x-widget"], "vec3");
        let listener = core("audio.listener");
        assert_eq!(
            listener["properties"]["forward"]["enum"],
            json!(["-z", "+z"])
        );
        let cam = core("camera.follow");
        assert_eq!(cam["properties"]["target"]["x-ref"], "entity");
        assert_eq!(cam["properties"]["pitch"]["minimum"], -89.0);
        // Pipes inside parentheses are not options; meta keys are not fields.
        let nav = core("nav.agent");
        assert!(nav["properties"]["on_fail"].get("enum").is_none());
        assert!(nav["properties"].get("conflict").is_none());
        // Nested objects documented as `{..}` are objects, not ranged numbers; numeric arrays
        // get element bounds, item lists get counts.
        assert_eq!(core("render.light")["properties"]["spot"]["type"], "object");
        assert_eq!(cam["properties"]["collision"]["type"], "object");
        let half = &core("nav.blocker")["properties"]["half"];
        assert_eq!(
            (half["minimum"].as_f64(), half.get("minItems")),
            (Some(0.01), None)
        );
        let clips = core("tween.clips");
        let tracks = &clips["properties"]["clips"]["additionalProperties"]["properties"]["tracks"];
        assert_eq!(
            (tracks["maxItems"].as_f64(), tracks.get("maximum")),
            (Some(16.0), None)
        );
        let clip = &clips["properties"]["clips"]["additionalProperties"];
        assert_eq!(
            clip["properties"]["mode"]["enum"],
            json!(["once", "loop", "ping_pong"])
        );
        assert_eq!(
            clip["properties"]["tracks"]["items"]["properties"]["target"]["x-ref"],
            "entity"
        );
    }

    #[test]
    fn every_core_type_has_an_object_schema_and_unknown_is_none() {
        let registry = Registry::core();
        let world = World::empty("t");
        for id in registry.components.keys().chain(registry.resources.keys()) {
            let s = typed_schema(&registry, &world, id).unwrap();
            assert_eq!(s["type"], "object", "{id}");
            assert!(s["properties"].is_object(), "{id}");
        }
        assert!(typed_schema(&registry, &world, "nope.nothing").is_none());
    }

    #[test]
    fn parsers_ignore_lookalikes() {
        assert_eq!(
            range("within min_pitch..max_pitch (-89..89)"),
            Some((-89.0, 89.0))
        );
        assert_eq!(range("0.1..10000"), Some((0.1, 10000.0)));
        assert_eq!(range("e.g. ... more"), None);
        assert_eq!(enum_options("optional {point:[x,y,z]} | {entity:ID}"), None);
        assert_eq!(
            enum_options("core.transform/position | core.transform/yaw"),
            None
        );
        assert_eq!(
            enum_options("box | sphere. Placed"),
            Some(vec!["box".into(), "sphere".into()])
        );
        assert_eq!(default("0..4, default 1"), Some(json!(1)));
        assert_eq!(
            default("rgb[3] 0..1, default [1,1,1]"),
            Some(json!([1, 1, 1]))
        );
        assert_eq!(
            bounds("meters > min_distance, <= 10000, default 30"),
            (None, Some(10000.0))
        );
        assert_eq!(bounds("number >= 0"), (Some(0.0), None));
    }
}
