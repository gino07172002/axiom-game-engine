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
