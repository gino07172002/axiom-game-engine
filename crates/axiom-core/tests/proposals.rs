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
