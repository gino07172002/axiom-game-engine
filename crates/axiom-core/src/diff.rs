//! ID-aware changes; large values stay outside default tool responses.
use crate::{
    local::{escape, fingerprint},
    World,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}
pub(crate) fn preview(v: &Value) -> Value {
    let bytes = serde_json::to_vec(v).unwrap();
    if bytes.len() <= 256 {
        v.clone()
    } else {
        json!({"omitted":true,"bytes":bytes.len(),"fingerprint":fingerprint(v).unwrap()})
    }
}
pub fn changes(before: &Value, after: &Value) -> Vec<Change> {
    let mut out = vec![];
    walk("", Some(before), Some(after), &mut out);
    out
}
fn walk(path: &str, a: Option<&Value>, b: Option<&Value>, out: &mut Vec<Change>) {
    if a == b {
        return;
    }
    match (a, b) {
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            for k in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                walk(&format!("{path}/{}", escape(k)), a.get(k), b.get(k), out);
            }
        }
        (Some(Value::Array(a)), Some(Value::Array(b))) => {
            if path == "/relations" {
                let relation_map = |rows: &[Value]| {
                    rows.iter()
                        .map(|row| {
                            let kind = row["kind"].as_str().unwrap_or_default();
                            let source = escape(row["source"].as_str().unwrap_or_default());
                            let target = row["target"].as_str().unwrap_or_default();
                            if kind == "parent" {
                                (format!("{path}/parent/@{source}"), json!(target))
                            } else {
                                (
                                    format!(
                                        "{path}/{}/@{source}/@{}",
                                        escape(kind),
                                        escape(target)
                                    ),
                                    json!(true),
                                )
                            }
                        })
                        .collect::<BTreeMap<_, _>>()
                };
                let a = relation_map(a);
                let b = relation_map(b);
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                    walk(key, a.get(key), b.get(key), out);
                }
                if a != b {
                    return;
                }
            }
            if path.starts_with("/scenes/@")
                && path.ends_with("/entities")
                && a.iter().chain(b).all(Value::is_string)
            {
                let am: BTreeSet<_> = a.iter().filter_map(Value::as_str).collect();
                let bm: BTreeSet<_> = b.iter().filter_map(Value::as_str).collect();
                for id in am.symmetric_difference(&bm) {
                    let value = json!(id);
                    walk(
                        &format!("{path}/@{}", escape(id)),
                        am.contains(id).then_some(&value),
                        bm.contains(id).then_some(&value),
                        out,
                    );
                }
                if am != bm {
                    return;
                }
            }
            fn keyed(v: &[Value]) -> Option<BTreeMap<&str, &Value>> {
                let map = v
                    .iter()
                    .map(|v| Some((v.get("id")?.as_str()?, v)))
                    .collect::<Option<BTreeMap<_, _>>>()?;
                (map.len() == v.len()).then_some(map)
            }
            if let (Some(am), Some(bm)) = (keyed(a), keyed(b)) {
                for k in am.keys().chain(bm.keys()).collect::<BTreeSet<_>>() {
                    walk(
                        &format!("{path}/@{}", escape(k)),
                        am.get(k).copied(),
                        bm.get(k).copied(),
                        out,
                    );
                }
                let aa = a.iter().map(|v| v["id"].clone()).collect::<Vec<_>>();
                let bb = b.iter().map(|v| v["id"].clone()).collect::<Vec<_>>();
                if aa != bb {
                    out.push(Change {
                        kind: "order".into(),
                        path: format!("{path}/@order"),
                        before: Some(preview(&json!(aa))),
                        after: Some(preview(&json!(bb))),
                    });
                }
            } else {
                for i in 0..a.len().max(b.len()) {
                    walk(&format!("{path}/{i}"), a.get(i), b.get(i), out);
                }
            }
        }
        _ => out.push(Change {
            kind: if a.is_none() {
                "added"
            } else if b.is_none() {
                "removed"
            } else {
                "changed"
            }
            .into(),
            path: path.into(),
            before: a.map(preview),
            after: b.map(preview),
        }),
    }
}
pub fn summary(a: &World, b: &World) -> Value {
    let all = changes(&json!(a), &json!(b));
    let resources = a
        .resources
        .keys()
        .chain(b.resources.keys())
        .filter(|id| a.resources.get(*id) != b.resources.get(*id))
        .collect::<BTreeSet<_>>();
    json!({"total_changes":all.len(),"changes":all.iter().take(64).collect::<Vec<_>>(),"truncated":all.len()>64,"resources":resources})
}
