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
fn light_presets_match_acl() {
    for kind in ["directional", "point", "spot", "area"] {
        // Object menu / palette command.
        assert_parity(
            lab(),
            |e| vec![e.run_command(&format!("object.light_{kind}")).unwrap()],
            &format!("create light {kind}"),
        );
        // Hierarchy "+ Object" menu (calls create_object with the preset name).
        assert_parity(
            lab(),
            |e| {
                vec![e
                    .create_object(&format!("light.{kind}"))
                    .into_iter()
                    .collect()]
            },
            &format!("create light.{kind}"),
        );
    }
    // Empty world: the render capability is added; the scene filter places the light.
    assert_parity(
        World::empty("blank"),
        |e| vec![e.run_command("object.light_spot").unwrap()],
        "create light spot",
    );
    assert_parity(
        lab(),
        |e| {
            e.scene = Some("staging".into());
            vec![e.run_command("object.light_point").unwrap()]
        },
        "create light point scene=staging",
    );
}

#[test]
fn trigger_presets_and_field_edits_match_acl() {
    for shape in ["box", "sphere"] {
        // Object menu / palette command.
        assert_parity(
            lab(),
            |e| vec![e.run_command(&format!("object.trigger_{shape}")).unwrap()],
            &format!("create trigger {shape}"),
        );
        // Hierarchy "+ Object" menu.
        assert_parity(
            lab(),
            |e| {
                vec![e
                    .create_object(&format!("trigger.{shape}"))
                    .into_iter()
                    .collect()]
            },
            &format!("create trigger.{shape}"),
        );
    }
    // Empty world: the trigger capability and the trigger.detect system are added.
    let blank = World::empty("blank");
    assert_parity(
        blank.clone(),
        |e| vec![e.run_command("object.trigger_sphere").unwrap()],
        "create trigger sphere",
    );
    let made = headless(&blank, "create trigger sphere");
    assert_eq!(made.capabilities.get("trigger"), Some(&1));
    assert!(!made.capabilities.contains_key("render"));
    assert!(made.systems.iter().any(|s| s == "trigger.detect"));
    // Inspector form edit of the created zone (only changed leaves, as set_field) equals
    // the ACL `set` of the same fields.
    let start = headless(&lab(), "create trigger box id=gate");
    assert_parity(
        start,
        |e| {
            e.select(Some("gate".into()));
            let entity = e.world.entity("gate").unwrap().clone();
            let world = e.world.clone();
            let draft = e.draft.as_mut().unwrap();
            draft.open_component(&world, &entity, "trigger.zone");
            let form = draft.form.as_mut().unwrap();
            form["half_extents"] = serde_json::json!([2.0, 1.0, 0.5]);
            form["once"] = serde_json::json!(true);
            draft.form_dirty = true;
            let edits = draft.edits().unwrap();
            // Only changed leaves: half_extents/0, half_extents/2 and once.
            assert_eq!(edits.len(), 3);
            vec![vec![e.write(edits, None)]]
        },
        "set gate trigger.zone/half_extents [2.0,1.0,0.5]\nset gate trigger.zone/once true",
    );
}

#[test]
fn remove_component_matches_acl() {
    let start = headless(&lab(), "create trigger box id=gate");
    assert_parity(
        start.clone(),
        |e| {
            vec![e
                .remove_component("gate", "trigger.zone")
                .into_iter()
                .collect()]
        },
        "remove gate trigger.zone",
    );
    // A component the entity does not author is a problem, not a write.
    let mut e = Editor::new(start, 1, None);
    assert!(e.remove_component("gate", "render.mesh").is_none());
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
    // Each copy sits right after its source in the authored (sibling) order; a second
    // duplicate lands between the source and the first copy.
    let order: Vec<&str> = world.entities.iter().map(|e| e.id.as_str()).collect();
    let at = |id: &str| order.iter().position(|e| *e == id).unwrap();
    assert_eq!(at("stage/crate/copy/2"), at("stage/crate") + 1);
    assert_eq!(at("stage/crate/copy"), at("stage/crate") + 2);
    assert_eq!(at("stage/beacon/copy/2"), at("stage/beacon") + 1);
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

#[test]
fn rename_matches_acl() {
    assert_parity(
        lab(),
        |e| {
            vec![e
                .rename_entity("stage/crate", "  信號 箱 ")
                .into_iter()
                .collect()]
        },
        "rename stage/crate \"信號 箱\"",
    );
    // An empty label clears it (the row then shows the ID).
    assert_parity(
        lab(),
        |e| vec![e.rename_entity("stage/crate", "").into_iter().collect()],
        "rename stage/crate \"\"",
    );
}

#[test]
fn create_child_matches_acl() {
    for preset in ["empty", "cube", "camera"] {
        assert_parity(
            lab(),
            |e| {
                e.select(Some("stage/rack".into()));
                vec![e.run_command(&format!("object.child_{preset}")).unwrap()]
            },
            &format!("create {preset} parent=stage/rack"),
        );
    }
}

#[test]
fn create_prefab_from_selection_matches_acl() {
    // A plain entity, and a prefab instance (its resolved components go into the new prefab).
    for id in ["stage/beacon", "stage/crate"] {
        assert_parity(
            lab(),
            |e| {
                e.select(Some(id.into()));
                vec![e.run_command("object.create_prefab").unwrap()]
            },
            &format!("prefab create from {id}"),
        );
    }
}

#[test]
fn hierarchy_reorder_drop_matches_acl() {
    assert_parity(
        lab(),
        |e| {
            vec![e
                .reorder_drop("stage/camera", "stage/floor", false)
                .into_iter()
                .collect()]
        },
        "move stage/camera before stage/floor",
    );
    assert_parity(
        lab(),
        |e| {
            vec![e
                .reorder_drop("stage/floor", "stage/beacon", true)
                .into_iter()
                .collect()]
        },
        "move stage/floor after stage/beacon",
    );
    // Dropping beside a row with another parent reparents (keeping the world pose) first.
    assert_parity(
        lab(),
        |e| {
            vec![e
                .reorder_drop("stage/crate", "stage/rack/cargo", false)
                .into_iter()
                .collect()]
        },
        "reparent stage/crate stage/rack\nmove stage/crate before stage/rack/cargo",
    );
}

#[test]
fn drop_to_ground_matches_acl() {
    assert_parity(
        lab(),
        |e| {
            e.select(Some("stage/crate".into()));
            e.extra.insert("stage/rack/cargo".into());
            vec![e.run_command("object.drop_to_ground").unwrap()]
        },
        "ground entities[id=stage/crate,id=stage/rack/cargo]",
    );
    // Once grounded, the command writes nothing on either side.
    let grounded = headless(&lab(), "ground stage/crate");
    let mut editor = Editor::new(grounded.clone(), 2, Some("stage/crate".into()));
    assert!(editor
        .run_command("object.drop_to_ground")
        .unwrap()
        .is_empty());
    assert!(editor.status.contains("Already on the ground"));
    let mut s = store(&grounded);
    let out = s
        .execute(Request::Acl {
            text: "ground stage/crate".into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .unwrap();
    assert!(out["text"].as_str().unwrap().starts_with("unchanged"));
    // The crate (authored at y=0.63, floating) moved down onto the floor.
    let crate_y = grounded.entity("stage/crate").unwrap().components["core.transform"]["position"]
        [1]
    .as_f64()
    .unwrap();
    assert!(crate_y < 0.63, "{crate_y}");
}

#[test]
fn view_commands_never_write() {
    let mut editor = Editor::new(lab(), 1, Some("stage/crate".into()));
    for cmd in [
        "view.top",
        "view.front",
        "view.right",
        "view.bottom",
        "view.back",
        "view.left",
        "view.perspective",
        "view.gizmos",
        "view.bookmark.save.1",
        "view.bookmark.recall.1",
    ] {
        assert!(editor.run_command(cmd).unwrap().is_empty(), "{cmd}");
    }
    assert_eq!(*editor.world, lab());
}

#[test]
fn view_only_hierarchy_commands_never_write() {
    let mut editor = Editor::new(lab(), 1, Some("stage/rack".into()));
    for cmd in [
        "edit.toggle_hidden",
        "edit.toggle_locked",
        "edit.select_children",
        "edit.rename",
    ] {
        assert!(editor.run_command(cmd).unwrap().is_empty(), "{cmd}");
    }
    assert_eq!(*editor.world, lab());
    assert!(editor.extra.contains("stage/rack/cargo"), "select children");
    let hidden = editor.view_flagged(false);
    assert!(hidden.contains("stage/rack") && hidden.contains("stage/rack/cargo"));
    assert!(editor.prefs.locked["editor-lab"].contains("stage/rack"));
}
