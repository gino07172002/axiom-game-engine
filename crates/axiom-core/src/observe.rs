//! Bounded incremental state observations. Cursors are scoped to a projection and runtime epoch.
use crate::{
    local::{fingerprint, pointer},
    Diagnostic,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::collections::VecDeque;
struct Snapshot {
    cursor: String,
    signature: String,
    state: Value,
    session: String,
    epoch: u64,
    trace_seq: u64,
    bytes: usize,
}
#[derive(Default)]
pub struct ObservationHistory {
    history: VecDeque<Snapshot>,
    next: u64,
    bytes: usize,
}
impl ObservationHistory {
    pub fn observe(
        &mut self,
        raw: &Value,
        cursor: Option<&str>,
        fields: &[String],
        trace_limit: usize,
        max: Option<usize>,
    ) -> Result<Value> {
        ensure!(
            fields.len() <= 32 && fields.iter().all(|f| f.len() <= 512) && trace_limit <= 128,
            "observation projection budget exceeded"
        );
        let max = max.unwrap_or(8192);
        ensure!((256..=65536).contains(&max), "max_bytes must be 256..65536");
        let explicit = !fields.is_empty();
        let fields = if fields.is_empty() {
            [
                "/world_id",
                "/rev",
                "/requested_rev",
                "/activation",
                "/activation_error",
                "/editor",
                "/tick",
                "/entities",
                "/cameras",
                "/systems",
                "/player_health",
                "/enemies_alive",
                "/enemies_total",
                "/gameplay/node",
                "/gameplay/ending",
                "/gameplay/variables",
                "/gameplay/choices",
                "/gameplay/active_scenes",
                "/gameplay/error",
                "/rts/summary",
                "/rts/selected",
                "/animation",
                "/shader",
                "/rhythm",
                "/action",
                "/events",
                "/input",
                "/combat",
                "/showcase",
                "/audio",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        } else {
            fields.iter().map(|f| pointer(f)).collect()
        };
        ensure!(
            fields
                .iter()
                .all(|f| !f.is_empty() && !f.starts_with("/gameplay/trace")),
            "use trace_limit for traces; full observation requires legacy observe"
        );
        // A malformed escape is a request error, not a field that happens to be absent.
        if let Some(bad) = fields.iter().find(|f| !valid_pointer(f)) {
            return Err(Diagnostic::error(
                "invalid_field",
                bad.as_str(),
                "invalid JSON pointer escape: `~` must be followed by 0 or 1",
            ));
        }
        let signature = fingerprint(&(&fields, trace_limit))?;
        let session = raw["runtime_session"]
            .as_str()
            .context("observation session missing")?
            .to_owned();
        let epoch = raw["runtime_epoch"].as_u64().unwrap_or(0);
        let mut state = json!({});
        // Requested fields this observation does not have (a typo, or an optional value that
        // is absent right now); at most 32, like the projection itself.
        let mut missing = vec![];
        for field in &fields {
            // A field inside another requested field is already part of that value: projecting
            // it as well would depend on the order the two were listed in.
            let covered = fields.iter().any(|other| {
                field.len() > other.len()
                    && field.starts_with(other.as_str())
                    && field.as_bytes()[other.len()] == b'/'
            });
            match raw.pointer(field) {
                Some(value) if !covered => insert(&mut state, field, value.clone())?,
                Some(_) => {}
                None if explicit => missing.push(field.as_str()),
                None => {}
            }
        }
        let traces = raw
            .pointer("/gameplay/trace")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let trace_seq = traces.last().and_then(|v| v["seq"].as_u64()).unwrap_or(0);
        let previous = cursor.and_then(|c| self.history.iter().find(|s| s.cursor == c));
        let reason = match previous {
            None if cursor.is_some() => Some("cursor_expired_or_foreign"),
            None => Some("initial"),
            Some(p) if p.signature != signature => Some("projection_changed"),
            Some(p) if p.session != session || p.epoch != epoch => Some("runtime_restarted"),
            _ => None,
        };
        let previous_seq = if reason.is_none() {
            previous.map_or(0, |p| p.trace_seq)
        } else {
            0
        };
        let available = traces
            .iter()
            .filter(|t| t["seq"].as_u64().unwrap_or(0) > previous_seq)
            .collect::<Vec<_>>();
        let trace_gap = trace_limit > 0
            && (available.len() > trace_limit
                || (reason.is_none()
                    && traces
                        .first()
                        .and_then(|t| t["seq"].as_u64())
                        .is_some_and(|first| first > previous_seq.saturating_add(1))));
        let trace = available
            .into_iter()
            .rev()
            .take(trace_limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        let unchanged = reason.is_none()
            && previous.is_some_and(|p| {
                p.state == state && (trace_limit == 0 || p.trace_seq == trace_seq)
            });
        let next_cursor = if unchanged {
            previous.unwrap().cursor.clone()
        } else {
            format!("o1.{session}.{}", self.next + 1)
        };
        let mut response = json!({"ok":true,"runtime":true,"runtime_session":session,"runtime_epoch":epoch,"rev":raw["rev"],"tick":raw["tick"],"cursor":next_cursor,"reset":reason.is_some(),"unchanged":unchanged});
        if let Some(reason) = reason {
            response["reason"] = json!(reason);
            response["snapshot"] = state.clone();
        } else {
            // State deltas contain exact values, never the compact diff previews used for authoring.
            response["changes"] = json!(state_changes(&previous.unwrap().state, &state));
        }
        if !missing.is_empty() {
            response["missing_fields"] = json!(missing);
        }
        if trace_limit > 0 {
            response["trace"] = json!(trace);
            response["trace_gap"] = json!(trace_gap);
        }
        ensure!(
            serde_json::to_vec(&response)?.len() <= max,
            "observation exceeds max_bytes; select fewer fields or reduce trace_limit"
        );
        if !unchanged {
            self.next += 1;
            let bytes = serde_json::to_vec(&state)?.len();
            self.bytes += bytes;
            self.history.push_back(Snapshot {
                cursor: next_cursor,
                signature,
                state,
                session,
                epoch,
                trace_seq,
                bytes,
            });
            while self.history.len() > 64 || self.bytes > 4_194_304 {
                if let Some(old) = self.history.pop_front() {
                    self.bytes -= old.bytes;
                }
            }
        }
        Ok(response)
    }
}
/// RFC 6901 escapes: every `~` starts `~0` or `~1`.
fn valid_pointer(path: &str) -> bool {
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}
fn insert(root: &mut Value, path: &str, value: Value) -> Result<()> {
    let parts = path
        .strip_prefix('/')
        .context("observation fields must be JSON pointers")?
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>();
    let mut node = root;
    for part in &parts[..parts.len() - 1] {
        node = node
            .as_object_mut()
            .ok_or_else(|| {
                Diagnostic::error("invalid_field", path, "overlapping scalar projection")
            })?
            .entry(part.clone())
            .or_insert_with(|| json!({}));
    }
    node.as_object_mut()
        .context("overlapping observation fields")?
        .insert(parts.last().unwrap().clone(), value);
    Ok(())
}
/// RFC-6901 object paths with arrays replaced as a unit, so IDs and sequence order remain unambiguous.
pub fn state_changes(a: &Value, b: &Value) -> Vec<Value> {
    let mut out = vec![];
    fn walk(path: &str, a: Option<&Value>, b: Option<&Value>, out: &mut Vec<Value>) {
        if a == b {
            return;
        }
        if let (Some(Value::Object(a)), Some(Value::Object(b))) = (a, b) {
            for k in a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                walk(
                    &format!("{path}/{}", crate::local::escape(k)),
                    a.get(k),
                    b.get(k),
                    out,
                );
            }
        } else {
            out.push(if let Some(v) = b {
                json!({"op":"set","path":path,"value":v})
            } else {
                json!({"op":"remove","path":path})
            });
        }
    }
    walk("", Some(a), Some(b), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_fields_that_are_absent_are_listed_not_silently_dropped() {
        let mut h = ObservationHistory::default();
        let raw =
            json!({"runtime_session":"s","runtime_epoch":1,"rev":3,"tick":0,"requested_rev":3});
        let fields = vec!["/requested_rev".to_owned(), "/requsted_rev".to_owned()];
        let first = h.observe(&raw, None, &fields, 0, None).unwrap();
        assert_eq!(first["snapshot"], json!({"requested_rev":3}));
        assert_eq!(first["missing_fields"], json!(["/requsted_rev"]));
        // The default projection names optional values and reports none of them missing.
        let default = h.observe(&raw, None, &[], 0, None).unwrap();
        assert!(default.get("missing_fields").is_none(), "{default}");
    }

    #[test]
    fn a_malformed_pointer_escape_is_an_error_but_an_absent_optional_field_is_not() {
        let mut h = ObservationHistory::default();
        let raw = json!({"runtime_session":"s","runtime_epoch":1,"rev":3,"tick":0,"a/b":1,"c~d":2});
        // Valid escapes resolve; a valid optional field that is absent right now is listed.
        let ok = h
            .observe(
                &raw,
                None,
                &["/a~1b".into(), "/c~0d".into(), "/activation_error".into()],
                0,
                None,
            )
            .unwrap();
        assert_eq!(ok["snapshot"], json!({"a/b":1,"c~d":2}), "{ok}");
        assert_eq!(ok["missing_fields"], json!(["/activation_error"]));
        for bad in ["/a~2b", "/c~", "/x~~0"] {
            let error = h.observe(&raw, None, &[bad.into()], 0, None).unwrap_err();
            let d = error.downcast_ref::<Diagnostic>().expect("diagnostic");
            assert_eq!((d.code.as_str(), d.path.as_str()), ("invalid_field", bad));
        }
    }

    #[test]
    fn a_parent_and_child_field_project_the_same_in_either_order() {
        let raw = json!({"runtime_session":"s","runtime_epoch":1,"rev":3,"tick":0,"systems":["a","b"],"gameplay":{"node":"n"}});
        let observe = |fields: &[&str]| {
            let fields = fields.iter().map(|f| f.to_string()).collect::<Vec<_>>();
            ObservationHistory::default()
                .observe(&raw, None, &fields, 0, None)
                .unwrap()
        };
        let parent_first = observe(&["/systems", "/systems/0", "/gameplay", "/gameplay/node"]);
        let child_first = observe(&["/systems/0", "/systems", "/gameplay/node", "/gameplay"]);
        let expected = json!({"systems":["a","b"],"gameplay":{"node":"n"}});
        assert_eq!(parent_first["snapshot"], expected, "{parent_first}");
        assert_eq!(child_first["snapshot"], expected, "{child_first}");
        // A child of a present parent that itself is absent is still reported missing.
        let missing = observe(&["/systems", "/systems/9"]);
        assert_eq!(
            missing["missing_fields"],
            json!(["/systems/9"]),
            "{missing}"
        );
    }
}
