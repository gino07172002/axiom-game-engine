//! Proposals / History diff lines as people read them. The protocol keeps JSON-pointer
//! paths (`/entities/@plaza~1lamp-1/components/core.transform/position/0`) and order changes
//! as ID lists; the panels show `plaza/lamp-1 · core.transform/position[0]: 4 → 6` and
//! `↕ entities order: moved lamp-1 (12 items)`. Display only: built once when the rows
//! arrive, never per frame.
use serde_json::Value;

fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

/// `a/b/0/c` (escaped pointer segments) as `a/b[0]/c`.
fn join_fields(segments: &[&str]) -> String {
    let mut out = String::new();
    for segment in segments {
        if !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()) {
            out.push('[');
            out.push_str(segment);
            out.push(']');
        } else {
            if !out.is_empty() {
                out.push('/');
            }
            out.push_str(&unescape(segment.strip_prefix('@').unwrap_or(segment)));
        }
    }
    out
}

/// A diff path as people read it.
pub(crate) fn readable_path(path: &str) -> String {
    let segments: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').collect();
    match segments.as_slice() {
        ["entities", id, rest @ ..] if id.starts_with('@') => {
            let id = unescape(&id[1..]);
            match rest {
                [] => id,
                ["components", fields @ ..] if !fields.is_empty() => {
                    format!("{id} · {}", join_fields(fields))
                }
                fields => format!("{id} · {}", join_fields(fields)),
            }
        }
        ["relations", "parent", source] if source.starts_with('@') => {
            format!("{} · parent", unescape(&source[1..]))
        }
        ["relations", kind, source, target]
            if source.starts_with('@') && target.starts_with('@') =>
        {
            format!(
                "{} · {} → {}",
                unescape(&source[1..]),
                unescape(kind),
                unescape(&target[1..])
            )
        }
        segments => join_fields(segments),
    }
}

/// Compact value text, at most 40 characters; previews the store left out show their size.
fn short(v: &Value) -> String {
    if v["omitted"] == true {
        return format!("‹{} bytes›", v["bytes"]);
    }
    let s = axiom_core::local::display_value(v).to_string();
    if s.chars().count() > 40 {
        format!("{}…", s.chars().take(39).collect::<String>())
    } else {
        s
    }
}

/// Items of `after` that left the longest common subsequence with `before`: the ones that
/// moved (lists are at most 256 bytes, so the quadratic table is small).
fn moved<'a>(before: &[&str], after: &[&'a str]) -> Vec<&'a str> {
    let (n, m) = (before.len(), after.len());
    let mut table = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if before[i] == after[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut kept = vec![false; m];
    while i < n && j < m {
        if before[i] == after[j] {
            kept[j] = true;
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    after
        .iter()
        .zip(kept)
        .filter(|(_, k)| !k)
        .map(|(s, _)| *s)
        .collect()
}

/// What an `order` change means for people. The store emits one whenever the ID lists
/// differ, so additions and removals (listed as their own changes) produce it too: only IDs
/// in both lists can have moved, and `None` = nothing moved (the line is left out).
fn order_summary(change: &Value) -> Option<String> {
    fn ids(v: &Value) -> Option<Vec<&str>> {
        v.as_array()?.iter().map(Value::as_str).collect()
    }
    match (ids(&change["before"]), ids(&change["after"])) {
        (Some(before), Some(after)) => {
            let common_before: Vec<&str> = before
                .iter()
                .copied()
                .filter(|id| after.contains(id))
                .collect();
            let common_after: Vec<&str> = after
                .iter()
                .copied()
                .filter(|id| before.contains(id))
                .collect();
            let moved = moved(&common_before, &common_after);
            if moved.is_empty() {
                return None;
            }
            let mut names = moved.iter().take(3).copied().collect::<Vec<_>>().join(", ");
            if moved.len() > 3 {
                names.push_str(&format!(" +{}", moved.len() - 3));
            }
            Some(format!("moved {names} ({} items)", after.len()))
        }
        // The store left the lists out (over 256 bytes): additions / removals show as their
        // own changes; whether anything else moved is not known here.
        _ => Some("changed (list too long to show; ACL `diff A B` has it)".into()),
    }
}

/// One change of a `diff` result as a panel line; empty for an order change that only
/// reflects additions / removals listed separately.
pub(crate) fn change_line(change: &Value) -> String {
    let path = change["path"].as_str().unwrap_or("?");
    match change["kind"].as_str() {
        Some("added") => format!("+ {} = {}", readable_path(path), short(&change["after"])),
        Some("removed") => format!("- {}", readable_path(path)),
        Some("order") => match order_summary(change) {
            Some(summary) => {
                let container = path.strip_suffix("/@order").unwrap_or(path);
                format!("↕ {} order: {summary}", readable_path(container))
            }
            None => String::new(),
        },
        _ => format!(
            "~ {}: {} → {}",
            readable_path(path),
            short(&change["before"]),
            short(&change["after"])
        ),
    }
}
/// Lines for every change of a `diff` value (`{changes:[..]}`), in order.
pub(crate) fn lines(diff: &Value) -> Vec<String> {
    diff["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(change_line)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_read_as_object_component_and_field() {
        assert_eq!(
            readable_path("/entities/@plaza~1lamp-1/components/core.transform/position/0"),
            "plaza/lamp-1 · core.transform/position[0]"
        );
        assert_eq!(
            readable_path("/entities/@plaza~1lamp-1/label"),
            "plaza/lamp-1 · label"
        );
        assert_eq!(readable_path("/entities/@a~0b"), "a~b");
        assert_eq!(
            readable_path("/relations/parent/@stage~1crate"),
            "stage/crate · parent"
        );
        assert_eq!(
            readable_path("/relations/attached/@a/@b~1c"),
            "a · attached → b/c"
        );
        assert_eq!(
            readable_path("/resources/render.shaders/shaders/scan/wgsl"),
            "resources/render.shaders/shaders/scan/wgsl"
        );
        assert_eq!(
            readable_path("/scenes/@plaza/entities/@plaza~1lamp-1"),
            "scenes/plaza/entities/plaza/lamp-1"
        );
        assert_eq!(readable_path("/title"), "title");
    }

    #[test]
    fn lines_show_values_and_order_moves() {
        let changed = json!({"kind":"changed","path":"/entities/@plaza~1lamp-1/components/core.transform/position/0","before":4.0,"after":6.0});
        assert_eq!(
            change_line(&changed),
            "~ plaza/lamp-1 · core.transform/position[0]: 4.0 → 6.0"
        );
        let order = json!({"kind":"order","path":"/entities/@order","before":["a","b","c","d"],"after":["b","c","a","d"]});
        assert_eq!(change_line(&order), "↕ entities order: moved a (4 items)");
        let omitted = json!({"kind":"order","path":"/entities/@order","before":{"omitted":true,"bytes":900,"fingerprint":"x"},"after":{"omitted":true,"bytes":900,"fingerprint":"y"}});
        assert_eq!(
            change_line(&omitted),
            "↕ entities order: changed (list too long to show; ACL `diff A B` has it)"
        );
        // Additions and removals alone (listed as their own changes) are not moves.
        let added = json!({"kind":"order","path":"/entities/@order","before":["a","b"],"after":["a","b","c"]});
        assert_eq!(change_line(&added), "");
        let removed = json!({"kind":"order","path":"/entities/@order","before":["a","b","c"],"after":["a","c"]});
        assert_eq!(change_line(&removed), "");
        // IDs are shown as they are (order lists are not JSON-pointer escaped).
        let raw = json!({"kind":"order","path":"/entities/@order","before":["x~1y","z"],"after":["z","x~1y"]});
        assert_eq!(change_line(&raw), "↕ entities order: moved x~1y (2 items)");
        let big = json!({"kind":"added","path":"/entities/@x","after":{"omitted":true,"bytes":312,"fingerprint":"f"}});
        assert_eq!(change_line(&big), "+ x = ‹312 bytes›");
        assert_eq!(
            change_line(&json!({"kind":"removed","path":"/entities/@stage~1crate"})),
            "- stage/crate"
        );
        // An entity whose ID is `order` is a normal change, not an order summary.
        assert_eq!(
            change_line(&json!({"kind":"removed","path":"/entities/@order"})),
            "- order"
        );
        // Reversed: one item stays in place, the other four moved.
        let many = json!({"kind":"order","path":"/scenes/@order","before":["a","b","c","d","e"],"after":["e","d","c","b","a"]});
        assert_eq!(
            change_line(&many),
            "↕ scenes order: moved d, c, b +1 (5 items)"
        );
    }
}
