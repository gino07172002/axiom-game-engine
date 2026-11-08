//! Editor ↔ headless parity: every content-changing editor feature, driven through the
//! editor (command registry or the method its widget calls), must leave the same world as
//! the equivalent ACL script run against a store with no editor at all.
use super::{Action, Editor};
use axiom_core::{Request, Store, World};

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

fn store(world: &World) -> Store {
    Store::memory_with(crate::installed_registry().authoring, world.clone()).unwrap()
}

/// Commits every editor write in order, syncing the editor after each like the host does.
fn commit(editor: &mut Editor, store: &mut Store, actions: Vec<Action>) {
    assert!(!actions.is_empty(), "the editor produced no write");
    for action in actions {
        let Action::Write { request, .. } = action else {
            panic!("expected a write");
        };
        assert!(matches!(request, Request::Apply { .. }));
        store.execute(request).unwrap();
        let (rev, world) = store.current().unwrap();
        editor.update(rev, world, true);
    }
}

fn headless(start: &World, script: &str) -> World {
    let mut store = store(start);
    store
        .execute(Request::Acl {
            text: script.into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .unwrap_or_else(|e| panic!("acl {script:?}: {e:#}"));
    store.current().unwrap().1
}

/// Runs `drive` against an editor on `start`, then checks the ACL script leaves an equal world.
fn assert_parity(start: World, drive: impl FnOnce(&mut Editor) -> Vec<Vec<Action>>, script: &str) {
    let mut editor = Editor::new(start.clone(), 1, None);
    let mut store = store(&start);
    for batch in drive(&mut editor) {
        commit(&mut editor, &mut store, batch);
    }
    let via_editor = store.current().unwrap().1;
    assert_ne!(via_editor, start, "the editor path changed nothing");
    assert_eq!(
        via_editor,
        headless(&start, script),
        "editor and `{script}` diverge"
    );
}

#[test]
fn create_presets_match_acl() {
    for preset in ["empty", "cube", "camera"] {
        assert_parity(
            lab(),
            |e| vec![e.run_command(&format!("object.{preset}")).unwrap()],
            &format!("create {preset}"),
        );
    }
    // Empty world: cube/camera add the render capability; the first camera becomes active.
    for preset in ["cube", "camera"] {
        assert_parity(
            World::empty("blank"),
            |e| vec![e.run_command(&format!("object.{preset}")).unwrap()],
            &format!("create {preset}"),
        );
    }
}

#[test]
fn repeated_creates_and_scene_filter_match_acl() {
    // Two commits in the editor vs. two statements in one ACL revision: same IDs.
    let mut editor = Editor::new(lab(), 1, None);
    let mut store = store(&lab());
    for _ in 0..2 {
        let actions = editor.run_command("object.cube").unwrap();
        commit(&mut editor, &mut store, actions);
    }
    let world = store.current().unwrap().1;
    assert!(world.entity("cube/2").is_some());
    assert_eq!(world, headless(&lab(), "create cube\ncreate cube"));
    // The hierarchy scene filter places new objects in that scene.
    assert_parity(
        lab(),
        |e| {
            e.scene = Some("staging".into());
            vec![e.run_command("object.camera").unwrap()]
        },
        "create camera scene=staging",
    );
}

#[test]
fn instantiate_prefab_matches_acl() {
    assert_parity(
        lab(),
        |e| vec![e.instantiate_prefab("cargo-crate").into_iter().collect()],
        "instantiate cargo-crate",
    );
    assert_parity(
        lab(),
        |e| {
            e.scene = Some("staging".into());
            vec![e.instantiate_prefab("cargo-crate").into_iter().collect()]
        },
        "instantiate cargo-crate scene=staging",
    );
}

#[test]
fn duplicate_and_paste_match_acl() {
    let select = |e: &mut Editor| {
        e.select(Some("stage/crate".into()));
        e.extra.insert("stage/beacon".into());
    };
    assert_parity(
        lab(),
        |e| {
            select(e);
            vec![e.run_command("edit.duplicate").unwrap()]
        },
        "duplicate entities[id=stage/crate,id=stage/beacon]",
    );
    // Duplicating twice yields /copy then /copy/2 on both sides.
    let mut editor = Editor::new(lab(), 1, None);
    let mut store = store(&lab());
    for _ in 0..2 {
        select(&mut editor);
        let actions = editor.run_command("edit.duplicate").unwrap();
        commit(&mut editor, &mut store, actions);
    }
    let world = store.current().unwrap().1;
    assert!(world.entity("stage/crate/copy/2").is_some());
    assert_eq!(
        world,
        headless(
            &lab(),
            "duplicate entities[id=stage/crate,id=stage/beacon]\nduplicate entities[id=stage/crate,id=stage/beacon]"
        )
    );
    // Paste duplicates the clipboard IDs that still exist.
    assert_parity(
        lab(),
        |e| {
            e.clipboard = vec!["stage/rack".into(), "gone".into()];
            vec![e.run_command("edit.paste").unwrap()]
        },
        "duplicate stage/rack",
    );
    // A child keeps its parent: stage/rack/cargo/copy is parented to stage/rack.
    assert_parity(
        lab(),
        |e| {
            e.select(Some("stage/rack/cargo".into()));
            vec![e.run_command("edit.duplicate").unwrap()]
        },
        "duplicate stage/rack/cargo",
    );
}

#[test]
fn delete_matches_acl() {
    assert_parity(
        lab(),
        |e| {
            e.select(Some("stage/rack".into()));
            e.extra.insert("stage/camera".into());
            vec![e.run_command("edit.delete").unwrap()]
        },
        "delete entities[id=stage/rack,id=stage/camera]",
    );
}

#[test]
fn reparent_drag_matches_acl() {
    assert_parity(
        lab(),
        |e| {
            vec![e
                .reparent_drop("stage/crate", Some("stage/rack"))
                .into_iter()
                .collect()]
        },
        "reparent stage/crate stage/rack",
    );
    assert_parity(
        lab(),
        |e| {
            vec![e
                .reparent_drop("stage/rack/cargo", None)
                .into_iter()
                .collect()]
        },
        "reparent stage/rack/cargo root",
    );
}

#[test]
fn scene_operations_match_acl() {
    assert_parity(
        lab(),
        |e| vec![e.scene_create("arena").into_iter().collect()],
        "scene create arena",
    );
    assert_parity(
        lab(),
        |e| vec![e.scene_set_active("staging", true).into_iter().collect()],
        "scene activate staging on",
    );
    assert_parity(
        lab(),
        |e| {
            vec![e
                .scene_set_active("assembly-yard", false)
                .into_iter()
                .collect()]
        },
        "scene activate assembly-yard off",
    );
    assert_parity(
        lab(),
        |e| vec![e.scene_delete("assembly-yard").into_iter().collect()],
        "scene delete assembly-yard",
    );
}

#[test]
fn failed_ops_become_problems_not_writes() {
    let mut editor = Editor::new(lab(), 1, None);
    assert!(editor.scene_create("staging").is_none());
    assert_eq!(editor.problems[0]["code"], "id_taken");
    assert!(editor.instantiate_prefab("missing").is_none());
    assert_eq!(editor.problems[0]["code"], "not_found");
}
