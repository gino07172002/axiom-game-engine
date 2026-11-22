use axiom_core::{error_response, Entity, Registry, Request, Store, World};
use serde_json::{json, Value};

fn store() -> Store {
    let mut w = World::empty("proposals");
    let mut e = Entity::new("enemy/0");
    e.set("core.health", json!({"value":100,"max":100}))
        .unwrap();
    w.entities.push(e);
    Store::memory_with(Registry::core(), w).unwrap()
}

fn acl(store: &mut Store, text: &str, propose: bool) -> Result<Value, Value> {
    store
        .execute(Request::Acl {
            text: text.into(),
            preview: false,
            expected_rev: None,
            propose,
        })
        .map_err(|e| error_response(&e))
}

fn resolve(store: &mut Store, id: i64, approve: bool, rebase: bool) -> Result<Value, Value> {
    store
        .execute(Request::ResolveProposal {
            id,
            approve,
            rebase,
            keep: None,
            expected_rev: None,
        })
        .map_err(|e| error_response(&e))
}

#[test]
fn proposals_queue_without_writing_then_apply_on_approval() {
    let mut s = store();
    let out = acl(&mut s, "set enemy/0 core.health/value 40", true).unwrap();
    let text = out["text"].as_str().unwrap();
    assert!(
        text.starts_with("proposal #1 pending base_rev=1 changes=1"),
        "{text}"
    );
    assert!(text.contains("-> 40"), "{text}");
    assert_eq!(
        s.revision().unwrap(),
        1,
        "proposing must not write a revision"
    );
    let listed = s
        .execute(Request::Proposals {
            status: Some("pending".into()),
            limit: None,
        })
        .unwrap();
    assert_eq!(listed["proposals"][0]["diff"]["total_changes"], 1);
    assert_eq!(s.proposal_signature().unwrap(), (1, 1));
    let applied = resolve(&mut s, 1, true, false).unwrap();
    assert_eq!(applied["rev"], 2);
    assert_eq!(applied["status"], "approved");
    assert_eq!(s.proposal_signature().unwrap(), (0, 1));
    let again = resolve(&mut s, 1, true, false).unwrap_err();
    assert_eq!(again["diagnostics"][0]["code"], "proposal_resolved");
}

#[test]
fn stale_proposals_conflict_until_rebased_and_rejections_never_write() {
    let mut s = store();
    acl(&mut s, "set enemy/0 core.health/value 10", true).unwrap();
    acl(&mut s, "set enemy/0 label Boss", true).unwrap();
    // Someone else writes first; proposal #1's base revision is now stale.
    acl(&mut s, "set enemy/0 core.health/value 90", false).unwrap();
    let conflict = resolve(&mut s, 1, true, false).unwrap_err();
    assert_eq!(conflict["diagnostics"][0]["code"], "revision_conflict");
    let still = s
        .execute(Request::Proposals {
            status: Some("pending".into()),
            limit: None,
        })
        .unwrap();
    assert_eq!(still["proposals"].as_array().unwrap().len(), 2);
    let rebased = resolve(&mut s, 1, true, true).unwrap();
    assert_eq!(rebased["rev"], 3);
    let rejected = resolve(&mut s, 2, false, false).unwrap();
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(s.revision().unwrap(), 3);
}

#[test]
fn invalid_proposals_are_rejected_at_submission() {
    let mut s = store();
    let err = acl(&mut s, "set enemy/0 core.health/value 999", true).unwrap_err();
    assert!(err["error"].as_str().unwrap().contains("health"), "{err}");
    assert_eq!(s.proposal_signature().unwrap(), (0, 0));
    let both = s
        .execute(Request::Acl {
            text: "set enemy/0 label x".into(),
            preview: true,
            expected_rev: None,
            propose: true,
        })
        .unwrap_err();
    assert!(format!("{both:#}").contains("either --preview or --propose"));
}

#[test]
fn a_stale_proposal_whose_selector_could_match_new_objects_is_not_rebased() {
    let mut s = store();
    acl(&mut s, "tag enemy/0 +foe", false).unwrap();
    let edits = serde_json::from_value(json!([{"kind":"set_where","select":{"from":"entities","tags":["foe"]},"field":"/label","value":"Marked"}])).unwrap();
    s.execute(Request::Propose {
        expected_rev: None,
        edits,
        note: String::new(),
    })
    .unwrap();
    // After the proposal, another entity gets the tag: a rebase would mark it too.
    acl(&mut s, "create cube id=late\ntag late +foe", false).unwrap();
    let rev = s.revision().unwrap();
    let stale = resolve(&mut s, 1, true, true).unwrap_err();
    assert_eq!(stale["diagnostics"][0]["code"], "stale_selector", "{stale}");
    assert!(stale.to_string().contains("conflicts 1"), "{stale}");
    assert_eq!(s.revision().unwrap(), rev, "nothing was written");
    assert_eq!(s.proposal_signature().unwrap().0, 1, "still pending");
    // Approving with keep evaluates the selector on the reviewed base revision only.
    let kept = s
        .execute(Request::ResolveProposal {
            id: 1,
            approve: true,
            rebase: false,
            keep: Some(vec![]),
            expected_rev: Some(rev),
        })
        .unwrap();
    assert_eq!(kept["status"], "approved", "{kept}");
    let world = s.current().unwrap().1;
    assert_eq!(
        world.entity("enemy/0").unwrap().label.as_deref(),
        Some("Marked")
    );
    assert_ne!(
        world.entity("late").unwrap().label.as_deref(),
        Some("Marked")
    );
}

fn propose_set_where(s: &mut Store, select: Value) {
    let edits = serde_json::from_value(
        json!([{"kind":"set_where","select":select,"field":"/label","value":"Marked"}]),
    )
    .unwrap();
    s.execute(Request::Propose {
        expected_rev: None,
        edits,
        note: String::new(),
    })
    .unwrap();
}

#[test]
fn an_explicit_id_that_only_exists_after_the_proposal_is_not_rebased() {
    let mut s = store();
    // On the base revision `late` does not exist: the reviewed preview marks enemy/0 only.
    propose_set_where(&mut s, json!({"from":"entities","ids":["enemy/0","late"]}));
    acl(&mut s, "create cube id=late", false).unwrap();
    let rev = s.revision().unwrap();
    let stale = resolve(&mut s, 1, true, true).unwrap_err();
    assert_eq!(stale["diagnostics"][0]["code"], "stale_selector", "{stale}");
    assert_eq!(s.revision().unwrap(), rev, "nothing was written");
    assert_eq!(s.proposal_signature().unwrap().0, 1, "still pending");
}

#[test]
fn a_selector_that_resolves_to_the_reviewed_targets_still_rebases() {
    let mut s = store();
    acl(&mut s, "tag enemy/0 +foe", false).unwrap();
    propose_set_where(&mut s, json!({"from":"entities","tags":["foe"]}));
    // An unrelated commit: the selector still picks exactly enemy/0.
    acl(&mut s, "create cube id=other", false).unwrap();
    let approved = resolve(&mut s, 1, true, true).unwrap();
    assert_eq!(approved["status"], "approved", "{approved}");
    let world = s.current().unwrap().1;
    assert_eq!(
        world.entity("enemy/0").unwrap().label.as_deref(),
        Some("Marked")
    );
    assert_ne!(
        world.entity("other").unwrap().label.as_deref(),
        Some("Marked")
    );
}

#[test]
fn a_choice_id_shared_by_a_new_node_is_a_new_target() {
    // Choices are identified by node and ID: the same choice ID under a node added later is
    // a target the review never saw.
    let graph = |nodes: Value| json!({"kind":"set_resource","id":"narrative.graph","value":{"start":"a","nodes":nodes}});
    let node = |id: &str| json!({"id":id,"speaker":"s","text":"t","choices":[{"id":"go","text":"Go","next":"a"}]});
    let mut s = store();
    let mut w = s.current().unwrap().1;
    w.capabilities.insert("narrative".into(), 1);
    w.capabilities.insert("behavior".into(), 1);
    w.systems.push("behavior.execute".into());
    w.resources.insert(
        "behavior.program".into(),
        json!(axiom_core::gameplay::Program::default()),
    );
    let setup: Vec<axiom_core::Edit> = serde_json::from_value(
        json!([{"kind":"replace_world","world":w}, graph(json!([node("a")]))]),
    )
    .unwrap();
    s.execute(Request::Apply {
        expected_rev: None,
        edits: setup,
    })
    .unwrap();
    let edits = serde_json::from_value(
        json!([{"kind":"set_where","select":{"from":"choices","ids":["go"]},"field":"/text","value":"Leave"}]),
    )
    .unwrap();
    s.execute(Request::Propose {
        expected_rev: None,
        edits,
        note: String::new(),
    })
    .unwrap();
    let edits = serde_json::from_value(json!([graph(json!([node("a"), node("b")]))])).unwrap();
    s.execute(Request::Apply {
        expected_rev: None,
        edits,
    })
    .unwrap();
    let stale = resolve(&mut s, 1, true, true).unwrap_err();
    assert_eq!(stale["diagnostics"][0]["code"], "stale_selector", "{stale}");
}

#[test]
fn a_proposal_with_more_than_64_changes_can_be_paged_before_approval() {
    let mut s = store();
    let mut edits = vec![];
    for i in 0..100 {
        edits.push(json!({"kind":"put_entity","entity":{"id":format!("crate/{i:03}")}}));
    }
    s.execute(Request::Propose {
        expected_rev: None,
        edits: serde_json::from_value(json!(edits)).unwrap(),
        note: String::new(),
    })
    .unwrap();
    let listed = s
        .execute(Request::Proposals {
            status: Some("pending".into()),
            limit: None,
        })
        .unwrap();
    let stored = &listed["proposals"][0]["diff"];
    assert_eq!(stored["truncated"], true, "{stored}");
    let total = stored["total_changes"].as_u64().unwrap() as usize;
    let mut paths = vec![];
    let mut offset = Some(0);
    while let Some(o) = offset {
        let page = s
            .execute(Request::ProposalDiff {
                id: 1,
                offset: o,
                limit: Some(32),
            })
            .unwrap();
        assert_eq!(page["status"], "pending");
        assert_eq!(page["total_changes"].as_u64().unwrap() as usize, total);
        for c in page["changes"].as_array().unwrap() {
            paths.push(c["path"].as_str().unwrap().to_owned());
        }
        offset = page["next_offset"].as_u64().map(|n| n as usize);
    }
    assert_eq!(paths.len(), total);
    assert!(
        paths.iter().any(|p| p.contains("crate~1099")),
        "the 100th entity is reachable: {paths:?}"
    );
    assert_eq!(s.revision().unwrap(), 1, "reading wrote nothing");
    let missing = s
        .execute(Request::ProposalDiff {
            id: 9,
            offset: 0,
            limit: None,
        })
        .unwrap_err();
    assert!(format!("{missing:#}").contains("not found"));
}

#[test]
fn an_acl_proposal_note_is_cut_to_the_byte_limit_on_a_character_boundary() {
    let mut s = store();
    // A long Traditional Chinese comment (3 bytes per character) next to one valid edit.
    let script = format!("# {}\nset enemy/0 core.health/value 40", "註解".repeat(800));
    assert!(script.len() > 2048 && script.chars().count() < 2048);
    acl(&mut s, &script, true).unwrap();
    let listed = s
        .execute(Request::Proposals {
            status: Some("pending".into()),
            limit: None,
        })
        .unwrap();
    let note = listed["proposals"][0]["note"].as_str().unwrap();
    assert!(note.len() <= 2048 && note.len() > 2040, "{}", note.len());
    assert!(script.starts_with(note));
}

/// A database in a fresh temp directory opened with `world`, and a raw connection to it for
/// writing rows the way an older build did.
fn file_store(name: &str, world: World) -> (std::path::PathBuf, Store, rusqlite::Connection) {
    let dir = std::env::temp_dir().join(format!("axiom-proposal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("world.sqlite");
    let store = Store::open_with(&path, Registry::core(), world).unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    (path, store, raw)
}

/// A pending proposal as a build before recorded targets stored it (no `targets`).
fn insert_legacy_proposal(raw: &rusqlite::Connection, edits: &Value) -> i64 {
    raw.execute(
        "INSERT INTO proposals(note,base_rev,edits,preview) VALUES ('legacy',1,?1,'{}')",
        [edits.to_string()],
    )
    .unwrap();
    raw.last_insert_rowid()
}

fn status(s: &mut Store, id: i64) -> Value {
    let listed = s
        .execute(Request::Proposals {
            status: None,
            limit: Some(64),
        })
        .unwrap();
    listed["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap()["status"]
        .clone()
}

/// Two counters that are the same f64 (2^53 and 2^53 + 1).
fn counters() -> World {
    let mut world = World::empty("ints");
    world.component_schemas.insert(
        "game.counter".into(),
        serde_json::from_value(json!({"fields":{"n":{"ty":"int"}}})).unwrap(),
    );
    for (id, n) in [
        ("a", 9_007_199_254_740_992_i64),
        ("b", 9_007_199_254_740_993),
    ] {
        let mut e = Entity::new(id);
        e.set("game.counter", json!({ "n": n })).unwrap();
        world.entities.push(e);
    }
    world
}

#[test]
fn a_selector_proposal_stored_before_recorded_targets_needs_a_new_proposal() {
    let (path, mut s, raw) = file_store("legacy-targets", counters());
    // Reviewed with f64 comparison: `lt 2^53+1` matched nothing then, and matches `a` now.
    let edits = json!([{"kind":"set_where","select":{"from":"entities","where":[{"field":"/components/game.counter/n","op":"lt","value":9_007_199_254_740_993_i64}]},"field":"/label","value":"hit"}]);
    let id = insert_legacy_proposal(&raw, &edits);
    // Review still works.
    let page = s
        .execute(Request::ProposalDiff {
            id,
            offset: 0,
            limit: None,
        })
        .unwrap();
    assert_eq!(page["status"], "pending", "{page}");
    let refused = resolve(&mut s, id, true, false).unwrap_err();
    assert_eq!(
        refused["diagnostics"][0]["code"], "stale_selector",
        "{refused}"
    );
    assert!(
        refused.to_string().contains("propose the same edits again"),
        "{refused}"
    );
    assert_eq!(s.revision().unwrap(), 1, "nothing committed");
    assert_eq!(status(&mut s, id), "pending");
    // Proposed again, it records its targets and approves.
    let again = s
        .execute(Request::Propose {
            expected_rev: Some(1),
            edits: serde_json::from_value(edits).unwrap(),
            note: String::new(),
        })
        .unwrap();
    let approved = resolve(&mut s, again["proposal"].as_i64().unwrap(), true, false).unwrap();
    assert_eq!(approved["rev"], 2, "{approved}");
    let world = s.current().unwrap().1;
    assert_eq!(world.entity("a").unwrap().label.as_deref(), Some("hit"));
    assert_eq!(world.entity("b").unwrap().label, None);
    drop((s, raw));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_proposal_whose_selector_now_resolves_differently_is_not_approved() {
    let (path, mut s, raw) = file_store("changed-targets", counters());
    let edits = json!([{"kind":"set_where","select":{"from":"entities","where":[{"field":"/components/game.counter/n","op":"ge","value":0}]},"field":"/label","value":"hit"}]);
    let proposed = s
        .execute(Request::Propose {
            expected_rev: Some(1),
            edits: serde_json::from_value(edits).unwrap(),
            note: String::new(),
        })
        .unwrap();
    let id = proposed["proposal"].as_i64().unwrap();
    let recorded: String = raw
        .query_row("SELECT targets FROM proposals WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    let recorded: Vec<Vec<String>> = serde_json::from_str(&recorded).unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].len(), 2, "both counters: {recorded:?}");
    // Simulate a change of selector semantics after review: the recorded preview selected
    // only the first of them.
    raw.execute(
        "UPDATE proposals SET targets=?2 WHERE id=?1",
        rusqlite::params![id, json!([[recorded[0][0]]]).to_string()],
    )
    .unwrap();
    let refused = resolve(&mut s, id, true, false).unwrap_err();
    assert_eq!(
        refused["diagnostics"][0]["code"], "stale_selector",
        "{refused}"
    );
    assert_eq!(s.revision().unwrap(), 1);
    assert_eq!(status(&mut s, id), "pending");
    drop((s, raw));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_large_proposal_stored_before_the_shared_budget_stays_reviewable() {
    // 120 enemies; each edit writes 120 x ~4 KB = ~481 KB: 36 of them (~17.3 MB) were a valid
    // proposal when each edit had its own 16 MiB budget, and exceed one shared budget.
    let mut world = World::empty("budget");
    world.component_schemas.insert(
        "game.note".into(),
        serde_json::from_value(json!({"fields":{"text":{"ty":"string"}}})).unwrap(),
    );
    for i in 0..120 {
        world.entities.push(Entity::new(format!("enemy/{i:03}")));
    }
    let (path, mut s, raw) = file_store("budget", world);
    let edit = json!({"kind":"set_where","select":{"from":"entities","id_glob":"enemy/*"},
        "field":"/components/game.note","value":{"text":"x".repeat(4000)}});
    let id = insert_legacy_proposal(&raw, &json!(vec![edit; 36]));
    let page = s
        .execute(Request::ProposalDiff {
            id,
            offset: 0,
            limit: Some(8),
        })
        .expect("a stored proposal stays reviewable");
    assert_eq!(page["status"], "pending", "{page}");
    assert_eq!(page["total_changes"], 120, "{page}");
    let conflicts = s
        .execute(Request::ResolveProposal {
            id,
            approve: true,
            rebase: false,
            keep: None,
            expected_rev: None,
        })
        .map_err(|e| error_response(&e))
        .unwrap_err();
    assert_eq!(
        conflicts["diagnostics"][0]["code"], "budget_exceeded",
        "{conflicts}"
    );
    let text = conflicts.to_string();
    assert!(
        text.contains("split it into batches of at most 34 edits"),
        "{text}"
    );
    assert_eq!(status(&mut s, id), "pending");
    assert_eq!(s.revision().unwrap(), 1);
    assert!(s.proposal_conflicts(id).is_ok(), "conflicts are listed too");
    drop((s, raw));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
