//! M10 co-editing: revision authors, field-wise merges of stale edits and proposals.
use axiom_core::{error_response, ops, Author, Entity, Registry, Request, Store, World};
use serde_json::{json, Value};

fn world() -> World {
    let mut w = World::empty("coedit");
    for (id, x) in [("crate", 0.0), ("lamp", 4.0)] {
        let mut e = Entity::new(id);
        e.set(
            "core.transform",
            json!({"position":[x,0.0,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]}),
        )
        .unwrap();
        e.set("core.health", json!({"value":100,"max":100}))
            .unwrap();
        w.entities.push(e);
    }
    w
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

fn apply(w: &World, script: &str) -> World {
    let mut s = Store::memory_with(Registry::core(), w.clone()).unwrap();
    acl(&mut s, script, false).unwrap();
    s.current().unwrap().1
}

#[test]
fn merge_takes_one_sided_changes_and_reports_true_conflicts_only() {
    let base = world();
    let mine = apply(
        &base,
        "set crate core.transform/position [1,0,0]; set crate core.health/value 50; rename lamp Mine",
    );
    let theirs = apply(
        &base,
        "set crate core.transform/position [2,0,0]; set crate core.health/max 200; set lamp core.health/value 70",
    );
    let view = ops::merge_view(&base, &mine, &theirs);
    let paths: Vec<&str> = view.conflicts.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(
        paths,
        ["/entities/@crate/components/core.transform/position"]
    );
    assert_eq!(view.conflicts[0].mine, Some(json!([1, 0, 0])));
    assert_eq!(view.conflicts[0].theirs, Some(json!([2, 0, 0])));
    // health/value and lamp label are mine only; health/max and lamp health are theirs only.
    assert_eq!((view.mine_only, view.theirs_only), (2, 2));

    let all_mine = ops::merge_worlds(&base, &mine, &theirs, &[]).unwrap();
    let crate_ = all_mine.entity("crate").unwrap();
    assert_eq!(
        crate_.components["core.transform"]["position"],
        json!([1, 0, 0])
    );
    assert_eq!(
        crate_.components["core.health"],
        json!({"value":50,"max":200})
    );
    let lamp = all_mine.entity("lamp").unwrap();
    assert_eq!(lamp.label.as_deref(), Some("Mine"));
    assert_eq!(lamp.components["core.health"]["value"], 70);

    let keep = ["/entities/@crate/components/core.transform/position".to_owned()];
    let kept = ops::merge_worlds(&base, &mine, &theirs, &keep).unwrap();
    assert_eq!(
        kept.entity("crate").unwrap().components["core.transform"]["position"],
        json!([2, 0, 0])
    );
    let typo = ops::merge_worlds(&base, &mine, &theirs, &["/nope".into()]).unwrap_err();
    assert_eq!(
        error_response(&typo)["diagnostics"][0]["code"],
        "unknown_conflict"
    );
    // Merging nothing new is no edit; merging changes is one batch.
    assert!(ops::merge(&base, &base, &theirs, &[]).unwrap().is_empty());
    assert_eq!(ops::merge(&base, &mine, &theirs, &[]).unwrap().len(), 1);
}

#[test]
fn merge_handles_added_removed_objects_and_relative_order() {
    let base = world();
    let mine = apply(&base, "create cube id=box at=1,0,1; move lamp before crate");
    let theirs = apply(&base, "delete lamp; create empty id=marker");
    let view = ops::merge_view(&base, &mine, &theirs);
    // Mine moved lamp, theirs deleted it: the order unit cannot conflict (lamp is not on
    // every side), so nothing conflicts and the delete stands.
    assert!(view.conflicts.is_empty(), "{:?}", view.conflicts);
    let merged = ops::merge_worlds(&base, &mine, &theirs, &[]).unwrap();
    let ids: Vec<&str> = merged.entities.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"box") && ids.contains(&"marker") && !ids.contains(&"lamp"));
    merged.compile(&Registry::core()).unwrap();

    // Both reorder the same objects differently: one order conflict.
    let base = apply(&world(), "create empty id=c");
    let mine = apply(&base, "move c before crate");
    let theirs = apply(&base, "move lamp before crate");
    let view = ops::merge_view(&base, &mine, &theirs);
    assert_eq!(view.conflicts.len(), 1);
    assert_eq!(view.conflicts[0].path, "/entities#order");
    let merged = ops::merge_worlds(&base, &mine, &theirs, &[]).unwrap();
    let ids: Vec<&str> = merged.entities.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["c", "crate", "lamp"]);
}

#[test]
fn stale_proposal_merges_per_field_into_one_revision() {
    let mut s = Store::memory_with(Registry::core(), world()).unwrap();
    acl(
        &mut s,
        "set crate core.transform/position [1,0,0]; set crate core.health/value 50",
        true,
    )
    .unwrap();
    acl(&mut s, "set crate core.transform/position [2,0,0]", false).unwrap();
    let conflicts = acl(&mut s, "conflicts 1", false).unwrap();
    let text = conflicts["text"].as_str().unwrap();
    assert!(
        text.starts_with(
            "conflicts proposal=#1 base_rev=1 rev=2 count=1 proposal_only=1 current_only=0"
        ),
        "{text}"
    );
    assert!(
        text.contains("/entities/@crate/components/core.transform/position base=[0.0,0.0,0.0] proposal=[1,0,0] current=[2,0,0]"),
        "{text}"
    );
    let resolve = |s: &mut Store, keep: Vec<String>| {
        s.execute(Request::ResolveProposal {
            id: 1,
            approve: true,
            rebase: false,
            keep: Some(keep),
            expected_rev: Some(2),
        })
        .map_err(|e| error_response(&e))
    };
    let typo = resolve(&mut s, vec!["/x".into()]).unwrap_err();
    assert_eq!(typo["diagnostics"][0]["code"], "unknown_conflict");
    // Conflicts listed against revision 2 cannot be decided on another revision.
    let moved_on = s
        .execute(Request::ResolveProposal {
            id: 1,
            approve: true,
            rebase: false,
            keep: Some(vec![]),
            expected_rev: Some(1),
        })
        .map_err(|e| error_response(&e))
        .unwrap_err();
    assert_eq!(moved_on["diagnostics"][0]["code"], "revision_conflict");
    // And a merge without the listed revision is refused outright.
    let unlisted = s
        .execute(Request::ResolveProposal {
            id: 1,
            approve: true,
            rebase: false,
            keep: Some(vec![]),
            expected_rev: None,
        })
        .unwrap_err();
    assert!(format!("{unlisted:#}").contains("keep needs expected_rev"));
    assert_eq!(s.revision().unwrap(), 2);
    let done = resolve(
        &mut s,
        vec!["/entities/@crate/components/core.transform/position".into()],
    )
    .unwrap();
    assert_eq!(
        (done["rev"].as_i64(), done["status"].as_str()),
        (Some(3), Some("approved"))
    );
    let w = s.current().unwrap().1;
    let c = w.entity("crate").unwrap();
    assert_eq!(c.components["core.transform"]["position"], json!([2, 0, 0]));
    assert_eq!(c.components["core.health"]["value"], 50);
    let history = s.execute(Request::History { limit: Some(1) }).unwrap();
    assert_eq!(history["revisions"][0]["proposal"], 1);
    // keep cannot be combined with rebase / reject.
    let both = s
        .execute(Request::ResolveProposal {
            id: 1,
            approve: false,
            rebase: false,
            keep: Some(vec![]),
            expected_rev: None,
        })
        .unwrap_err();
    assert!(format!("{both:#}").contains("cannot be combined"));
}

#[test]
fn revisions_record_their_author_and_history_shows_it() {
    let mut s = Store::memory_with(Registry::core(), world()).unwrap();
    s.set_author(Some(Author::Human));
    acl(&mut s, "rename crate Human", false).unwrap();
    s.set_author(Some(Author::Agent));
    acl(&mut s, "rename crate Agent", false).unwrap();
    s.execute(Request::Undo {
        rev: 2,
        expected_rev: None,
    })
    .unwrap();
    let rows = s.execute(Request::History { limit: Some(8) }).unwrap()["revisions"].clone();
    let authors: Vec<Value> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.get("author").cloned().unwrap_or(Value::Null))
        .collect();
    assert_eq!(
        authors,
        [json!("agent"), json!("agent"), json!("human"), Value::Null]
    );
    let text = acl(&mut s, "history 2", false).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.lines().all(|l| l.ends_with(" by=agent")), "{text}");
}

#[test]
fn databases_from_before_the_author_column_open_and_keep_their_history() {
    let path = std::env::temp_dir().join(format!(
        "axiom-legacy-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        // The schema older builds created, with two revisions.
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE revisions(rev INTEGER PRIMARY KEY AUTOINCREMENT,world TEXT NOT NULL,reason TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));").unwrap();
        let w = serde_json::to_string(&world()).unwrap();
        for reason in ["initialize", "apply"] {
            c.execute(
                "INSERT INTO revisions(world,reason) VALUES (?1,?2)",
                [&w, reason],
            )
            .unwrap();
        }
    }
    let mut s = Store::open_with(&path, Registry::core(), World::empty("x")).unwrap();
    s.set_author(Some(Author::Human));
    acl(&mut s, "rename lamp L", false).unwrap();
    let rows = s.execute(Request::History { limit: Some(8) }).unwrap()["revisions"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 3);
    assert_eq!(rows[0]["author"], "human");
    assert!(rows[1].get("author").is_none() && rows[2].get("author").is_none());
    // A second connection opening the migrated file does not add the column again; an
    // older build's insert (no author) still works.
    drop(s);
    let again = Store::open_with(&path, Registry::core(), World::empty("x")).unwrap();
    assert_eq!(again.revision().unwrap(), 3);
    drop(again);
    let c = rusqlite::Connection::open(&path).unwrap();
    let w = serde_json::to_string(&world()).unwrap();
    c.execute(
        "INSERT INTO revisions(world,reason) VALUES (?1,'apply')",
        [&w],
    )
    .unwrap();
    drop(c);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

/// `history` with the author column and the approved-proposal lookup vs the query before M10,
/// on the same database (`cargo test --release -p axiom-core --test coediting measure -- --ignored --nocapture`).
#[test]
#[ignore]
fn measure_history_query() {
    let path = std::env::temp_dir().join(format!("axiom-history-{}.sqlite", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let mut s = Store::open_with(&path, Registry::core(), world()).unwrap();
    s.set_author(Some(Author::Agent));
    // 200 approved proposals and 200 plain revisions.
    for i in 0..200 {
        acl(
            &mut s,
            &format!("set crate core.health/value {}", i % 90 + 1),
            true,
        )
        .unwrap();
        s.execute(Request::ResolveProposal {
            id: i + 1,
            approve: true,
            rebase: false,
            keep: None,
            expected_rev: None,
        })
        .unwrap();
        acl(
            &mut s,
            &format!("set lamp core.health/value {}", i % 90 + 1),
            false,
        )
        .unwrap();
    }
    let raw = rusqlite::Connection::open(&path).unwrap();
    let median = |f: &mut dyn FnMut()| {
        let mut v: Vec<f64> = (0..300)
            .map(|_| {
                let t = std::time::Instant::now();
                f();
                t.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let new = median(&mut || {
        std::hint::black_box(s.execute(Request::History { limit: Some(64) }).unwrap());
    });
    let old = median(&mut || {
        let mut q = raw
            .prepare("SELECT rev,reason,created_at FROM revisions ORDER BY rev DESC LIMIT ?1")
            .unwrap();
        let rows = q
            .query_map([64], |r| {
                Ok(json!({"rev":r.get::<_,i64>(0)?,"reason":r.get::<_,String>(1)?,"created_at":r.get::<_,String>(2)?}))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        std::hint::black_box(rows);
    });
    println!(
        "{}",
        json!({"history_64_us":{"before_m10_query":old,"m10":new},"revisions":s.revision().unwrap(),"proposals":200})
    );
    drop((s, raw));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn an_approval_that_changes_nothing_credits_no_revision_to_the_proposal() {
    let mut s = Store::memory_with(Registry::core(), world()).unwrap();
    acl(&mut s, "rename crate Same", true).unwrap();
    acl(&mut s, "rename crate Same", false).unwrap();
    // Approved on the current revision: nothing changes, so no revision is the proposal's.
    let done = s
        .execute(Request::ResolveProposal {
            id: 1,
            approve: true,
            rebase: true,
            keep: None,
            expected_rev: None,
        })
        .unwrap();
    assert_eq!(
        (done["changed"].clone(), done["status"].clone()),
        (json!(false), json!("approved"))
    );
    let rows = s.execute(Request::History { limit: Some(4) }).unwrap()["revisions"].clone();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("proposal").is_none()),
        "{rows}"
    );
}
