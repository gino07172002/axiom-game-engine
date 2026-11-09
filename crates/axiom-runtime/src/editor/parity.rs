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
    // The host installs the same registry as the store's (world ops read its types).
    editor.set_component_hints(&crate::installed_registry().authoring);
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
fn path_presets_follow_and_point_edits_match_acl() {
    // Create path: Object menu / palette and the hierarchy "+ Object" menu.
    assert_parity(
        lab(),
        |e| vec![e.run_command("object.path").unwrap()],
        "create path",
    );
    assert_parity(
        lab(),
        |e| vec![e.create_object("path").into_iter().collect()],
        "create path",
    );
    // Empty world: the path capability and the path.move system are added (not render).
    let blank = World::empty("blank");
    assert_parity(
        blank.clone(),
        |e| vec![e.run_command("object.path").unwrap()],
        "create path",
    );
    let made = headless(&blank, "create path");
    assert_eq!(made.capabilities.get("path"), Some(&1));
    assert!(!made.capabilities.contains_key("render"));
    assert!(made.systems.iter().any(|s| s == "path.move"));
    // Follow Path: the selected objects follow the selected path; two start evenly spaced.
    let start = headless(
        &lab(),
        "create path id=road\ncreate cube id=car\ncreate cube id=bus",
    );
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("car".into()));
            e.extra.insert("bus".into());
            e.extra.insert("road".into());
            vec![e.run_command("object.follow_path").unwrap()]
        },
        "follow entities[id=car,id=bus] road",
    );
    let both = headless(&start, "follow entities[id=car,id=bus] road");
    let fraction =
        |id: &str| both.entity(id).unwrap().components["path.follow"]["start_fraction"].clone();
    assert_eq!((fraction("car"), fraction("bus")), (0.0.into(), 0.5.into()));
    // With no path selected, the world's only path is used.
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("car".into()));
            vec![e.run_command("object.follow_path").unwrap()]
        },
        "follow car road",
    );
    // Two paths and none selected: a problem, not a write.
    let two = headless(&start, "create path id=other");
    let mut e = Editor::new(two, 1, Some("car".into()));
    assert!(e.run_command("object.follow_path").unwrap().is_empty());
    // Point edits (Path ▸ Add / Remove Point, Inspector buttons, handle drag).
    let road = headless(&lab(), "create path id=road");
    // No point selected on a closed path: split the closing piece, appended at the end.
    assert_parity(
        road.clone(),
        |e| {
            e.select(Some("road".into()));
            vec![e.run_command("path.add_point").unwrap()]
        },
        "path road add",
    );
    // After the selected point 1: split the piece 1 -> 2.
    assert_parity(
        road.clone(),
        |e| {
            e.select(Some("road".into()));
            e.scene_input.path_point = Some(1);
            vec![e.run_command("path.add_point").unwrap()]
        },
        "path road insert 2",
    );
    assert_parity(
        road.clone(),
        |e| {
            e.select(Some("road".into()));
            e.scene_input.path_point = Some(0);
            vec![e.run_command("path.remove_point").unwrap()]
        },
        "path road remove 0",
    );
    // The handle drag commits through edit_path_point (driven with egui in viewport tests).
    assert_parity(
        road.clone(),
        |e| {
            vec![e
                .edit_path_point("road", axiom_core::ops::PointEdit::Move(2, [5.0, 0.0, 4.5]))
                .into_iter()
                .collect()]
        },
        "path road move 2 5,0,4.5",
    );
    // Removing below the minimum (3 points on a closed path) is a problem, not a write.
    let small = headless(&road, "path road remove 0");
    let mut e = Editor::new(small, 1, Some("road".into()));
    assert!(e.run_command("path.remove_point").unwrap().is_empty());
    assert!(e.run_command("object.path_unknown").is_err());
}

#[test]
fn nav_agents_obstacles_and_targets_match_acl() {
    // Create obstacle: Object menu / palette and the hierarchy "+ Object" menu.
    assert_parity(
        lab(),
        |e| vec![e.run_command("object.obstacle").unwrap()],
        "create obstacle",
    );
    assert_parity(
        lab(),
        |e| vec![e.create_object("obstacle").into_iter().collect()],
        "create obstacle",
    );
    // Empty world: the render capability is added (nav.blocker is core).
    let blank = World::empty("blank");
    assert_parity(
        blank.clone(),
        |e| vec![e.run_command("object.obstacle").unwrap()],
        "create obstacle",
    );
    // Add Nav Agent on the selection (selection order); nav capability + nav.steer once.
    let start = headless(
        &lab(),
        "create cube id=bob\ncreate cube id=ann\ncreate empty id=home at=4,0,2",
    );
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("bob".into()));
            e.extra.insert("ann".into());
            vec![e.run_command("object.nav_agent").unwrap()]
        },
        "nav entities[id=bob,id=ann] agent",
    );
    let agents = headless(&start, "nav entities[id=bob,id=ann] agent");
    assert_eq!(agents.capabilities.get("nav"), Some(&1));
    assert!(agents.systems.iter().any(|s| s == "nav.steer"));
    // Nav Target: Selected Object — the selected agents walk to the one non-agent selected.
    assert_parity(
        agents.clone(),
        |e| {
            e.select(Some("bob".into()));
            e.extra.insert("ann".into());
            e.extra.insert("home".into());
            vec![e.run_command("nav.target_object").unwrap()]
        },
        "nav entities[id=bob,id=ann] target home",
    );
    // The scene-view ground pick lands in set_nav_target (driven with egui in viewport tests).
    assert_parity(
        agents.clone(),
        |e| {
            e.select(Some("ann".into()));
            vec![e
                .set_nav_target(Some(axiom_core::navigation::NavTarget::Point([
                    1.5, 0.0, -2.25,
                ])))
                .into_iter()
                .collect()]
        },
        "nav ann target 1.5,0,-2.25",
    );
    // Nav Target: Clear.
    let targeted = headless(&agents, "nav bob target home");
    assert_parity(
        targeted.clone(),
        |e| {
            e.select(Some("bob".into()));
            vec![e.run_command("nav.clear_target").unwrap()]
        },
        "nav bob target none",
    );
    // Inspector form edit of the agent equals the ACL set of the same field.
    assert_parity(
        targeted.clone(),
        |e| {
            e.select(Some("bob".into()));
            let entity = e.world.entity("bob").unwrap().clone();
            let world = e.world.clone();
            let draft = e.draft.as_mut().unwrap();
            draft.open_component(&world, &entity, "nav.agent");
            let form = draft.form.as_mut().unwrap();
            form["speed"] = serde_json::json!(2.5);
            draft.form_dirty = true;
            let edits = draft.edits().unwrap();
            vec![vec![e.write(edits, None)]]
        },
        "set bob nav.agent/speed 2.5",
    );
    // No target object, or two: a problem, not a write. No agent selected: blocked.
    let mut e = Editor::new(agents.clone(), 1, Some("bob".into()));
    assert!(e.run_command("nav.target_object").unwrap().is_empty());
    e.extra.insert("home".into());
    e.extra.insert("stage/floor".into());
    assert!(e.run_command("nav.target_object").unwrap().is_empty());
    let mut e = Editor::new(agents, 1, Some("home".into()));
    assert!(e.run_command("nav.clear_target").is_err());
    assert!(e.run_command("nav.target_point").is_err());
}
#[test]
fn make_interactable_and_field_edits_match_acl() {
    // Object ▸ Make Interactable on the selection (selection order) = ACL interact SEL.
    let start = headless(&lab(), "create cube id=npc\ncreate cube id=door at=3,0,0");
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("door".into()));
            e.extra.insert("npc".into());
            vec![e.run_command("object.interactable").unwrap()]
        },
        "interact entities[id=door,id=npc]",
    );
    // Empty world: capability, systems, program with the event and the KeyE binding appear.
    let blank = headless(&World::empty("blank"), "create empty id=sign");
    assert_parity(
        blank.clone(),
        |e| {
            e.select(Some("sign".into()));
            vec![e.run_command("object.interactable").unwrap()]
        },
        "interact sign",
    );
    let made = headless(&blank, "interact sign");
    for system in ["interact.focus", "behavior.execute"] {
        assert!(made.systems.iter().any(|s| s == system), "{system}");
    }
    assert_eq!(
        made.resources["input.bindings"]["buttons"]["interact"],
        "KeyE"
    );
    // Inspector form edit of the created target (changed leaves only) = ACL set.
    let target = headless(&start, "interact npc");
    assert_parity(
        target,
        |e| {
            e.select(Some("npc".into()));
            let entity = e.world.entity("npc").unwrap().clone();
            let world = e.world.clone();
            let draft = e.draft.as_mut().unwrap();
            draft.open_component(&world, &entity, "interact.target");
            let form = draft.form.as_mut().unwrap();
            form["range"] = serde_json::json!(3.0);
            form["prompt"] = serde_json::json!("和露米聊聊");
            form["priority"] = serde_json::json!(2);
            draft.form_dirty = true;
            let edits = draft.edits().unwrap();
            assert_eq!(edits.len(), 3);
            vec![vec![e.write(edits, None)]]
        },
        "set npc interact.target/range 3.0\nset npc interact.target/prompt \"和露米聊聊\"\nset npc interact.target/priority 2",
    );
    // Nothing selected: the command is unavailable.
    let mut e = Editor::new(start, 1, None);
    assert!(e.run_command("object.interactable").is_err());
}
#[test]
fn audio_sources_listeners_and_clips_match_acl() {
    // Object ▸ Add Audio Source on the selection (selection order) = ACL audio SEL source; the
    // lab has no library, so clip `tone` is created in the same revision.
    let start = headless(
        &lab(),
        "create cube id=car\ncreate cube id=fountain at=3,0,0",
    );
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("fountain".into()));
            e.extra.insert("car".into());
            vec![e.run_command("object.audio_source").unwrap()]
        },
        "audio entities[id=fountain,id=car] source",
    );
    // Empty world: capability, system and library appear.
    let blank = headless(&World::empty("blank"), "create empty id=speaker");
    assert_parity(
        blank,
        |e| {
            e.select(Some("speaker".into()));
            vec![e.run_command("object.audio_source").unwrap()]
        },
        "audio speaker source",
    );
    // Object ▸ Make Audio Listener (primary selection) = ACL audio ID listener.
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("car".into()));
            vec![e.run_command("object.audio_listener").unwrap()]
        },
        "audio car listener",
    );
    // Inspector ▸ Audio source ▸ New clip (twice: engine, engine2), then the clip dropdown.
    let sourced = headless(&start, "audio car source");
    assert_parity(
        sourced.clone(),
        |e| {
            e.select(Some("car".into()));
            let mut dialog =
                super::options::OptionsDialog::for_command(e, "audio.new_clip").unwrap();
            dialog.set("Preset", "engine");
            let write = dialog.run(e).unwrap();
            assert_eq!(dialog.acl(e), "audio clip engine engine");
            vec![write.into_iter().collect()]
        },
        "audio clip engine engine",
    );
    let engined = headless(&sourced, "audio clip engine engine");
    assert_parity(
        engined.clone(),
        |e| {
            e.select(Some("car".into()));
            let mut dialog =
                super::options::OptionsDialog::for_command(e, "audio.new_clip").unwrap();
            dialog.set("Preset", "engine");
            vec![dialog.run(e).unwrap().into_iter().collect()]
        },
        "audio clip engine2 engine",
    );
    assert_parity(
        engined.clone(),
        |e| {
            e.select(Some("car".into()));
            let (clips, current) = e.audio_clips("car").unwrap();
            assert_eq!(
                (clips, current.as_str()),
                (vec!["engine".to_owned(), "tone".to_owned()], "tone")
            );
            vec![e.set_audio_clip("engine").into_iter().collect()]
        },
        "set car audio.source/clip \"engine\"",
    );
    // Inspector form edit of the source fields (changed leaves only) = ACL set.
    assert_parity(
        engined,
        |e| {
            e.select(Some("car".into()));
            let entity = e.world.entity("car").unwrap().clone();
            let world = e.world.clone();
            let draft = e.draft.as_mut().unwrap();
            draft.open_component(&world, &entity, "audio.source");
            let form = draft.form.as_mut().unwrap();
            form["loop"] = serde_json::json!(true);
            form["max_distance"] = serde_json::json!(45.0);
            form["rolloff"] = serde_json::json!("linear");
            draft.form_dirty = true;
            let edits = draft.edits().unwrap();
            assert_eq!(edits.len(), 3);
            vec![vec![e.write(edits, None)]]
        },
        "set car audio.source/loop true\nset car audio.source/max_distance 45.0\nset car audio.source/rolloff \"linear\"",
    );
    // A missing clip is a problem, not a write; nothing selected = unavailable.
    let mut e = Editor::new(sourced.clone(), 1, None);
    e.select(Some("car".into()));
    assert!(e.set_audio_clip("nope").is_none());
    let mut e = Editor::new(start, 1, None);
    assert!(e.run_command("object.audio_source").is_err());
    assert!(e.run_command("object.audio_listener").is_err());
}

#[test]
fn tween_clips_keys_and_options_match_acl() {
    // Object ▸ Add Tween Clip on the selection = ACL tween SEL clip NAME (default name clip).
    let start = headless(&lab(), "create cube id=door at=3,0,0\ncreate cube id=npc");
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("door".into()));
            e.extra.insert("npc".into());
            vec![e.run_command("object.tween_clip").unwrap()]
        },
        "tween entities[id=door,id=npc] clip clip",
    );
    // A second clip gets the next free name; the empty world gains capability and system.
    let one = headless(&start, "tween door clip clip");
    assert_parity(
        one.clone(),
        |e| {
            e.select(Some("door".into()));
            vec![e.run_command("object.tween_clip").unwrap()]
        },
        "tween door clip clip2",
    );
    let blank = headless(&World::empty("blank"), "create empty id=sign");
    assert_parity(
        blank,
        |e| {
            e.select(Some("sign".into()));
            vec![e.run_command("object.tween_clip").unwrap()]
        },
        "tween sign clip clip",
    );
    // Timeline ▸ Record key at the playhead (track 0) after moving the door = ACL key add T.
    let moved = headless(&one, "set door core.transform/position [4,1,0]");
    assert_parity(
        moved.clone(),
        |e| {
            e.select(Some("door".into()));
            e.tween_ui.time = 0.5;
            vec![e.run_command("tween.record_key").unwrap()]
        },
        "tween door key clip 0 add 0.5",
    );
    let recorded = headless(&moved, "tween door key clip 0 add 0.5");
    assert_eq!(
        recorded.entity("door").unwrap().components["tween.clips"]["clips"]["clip"]["tracks"][0]
            ["keys"][1]["v"],
        serde_json::json!([4.0, 1.0, 0.0])
    );
    // Key drag (the timeline calls move_tween_key on release) and Delete key.
    assert_parity(
        recorded.clone(),
        |e| {
            e.select(Some("door".into()));
            vec![e.move_tween_key("clip", 0, 1, 0.75).into_iter().collect()]
        },
        "tween door key clip 0 move 1 0.75",
    );
    assert_parity(
        recorded.clone(),
        |e| {
            e.select(Some("door".into()));
            e.tween_ui.key = Some(1);
            vec![e.run_command("tween.remove_key").unwrap()]
        },
        "tween door key clip 0 remove 1",
    );
    // Mode / autoplay controls, Add track, Remove clip.
    for (options, script) in [
        (
            axiom_core::ops::TweenClipOptions {
                mode: Some(axiom_core::tween::TweenMode::Loop),
                ..Default::default()
            },
            "tween door clip clip set mode=loop",
        ),
        (
            axiom_core::ops::TweenClipOptions {
                autoplay: Some(true),
                ..Default::default()
            },
            "tween door clip clip set autoplay=on",
        ),
    ] {
        assert_parity(
            recorded.clone(),
            |e| {
                e.select(Some("door".into()));
                vec![e.set_tween_options("clip", &options).into_iter().collect()]
            },
            script,
        );
    }
    assert_parity(
        recorded.clone(),
        |e| {
            e.select(Some("door".into()));
            vec![e
                .add_tween_track("clip", "core.transform/scale")
                .into_iter()
                .collect()]
        },
        "tween door track clip add core.transform/scale",
    );
    assert_parity(
        recorded.clone(),
        |e| {
            e.select(Some("door".into()));
            vec![e.remove_tween_clip("clip").into_iter().collect()]
        },
        "tween door clip clip remove",
    );
    // Unavailable: nothing selected, no clips, no key selected.
    let mut e = Editor::new(recorded.clone(), 1, None);
    assert!(e.run_command("object.tween_clip").is_err());
    assert!(e.run_command("tween.record_key").is_err());
    e.select(Some("npc".into()));
    assert!(e.run_command("tween.record_key").is_err());
    e.select(Some("door".into()));
    assert!(e.run_command("tween.remove_key").is_err());
    assert!(e.run_command("tween.record_key").is_ok());
}

#[test]
fn tween_scrub_previews_without_writing_or_moving_the_simulation() {
    let world = headless(
        &lab(),
        "create cube id=door at=3,0,0\ntween door clip open property=core.transform/yaw\ntween door key open 0 add 1 90",
    );
    let mut runtime =
        crate::world::WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
    let mut e = Editor::new(world, 1, Some("door".into()));
    let slot = runtime.state.compiled.indices["door"];
    let yaw_of = |frame: &crate::presentation::RenderFrame| {
        frame
            .instances
            .iter()
            .map(|i| glam::Mat4::from_cols_array_2d(&i.model))
            .find(|m| (m.w_axis.x - 3.0).abs() < 1e-4)
            .map(|m| {
                let (_, q, _) = m.to_scale_rotation_translation();
                axiom_core::tween::yaw_degrees(q)
            })
            .unwrap()
    };
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 760.0));
    assert!(yaw_of(&e.render_frame(&mut runtime, screen)).abs() < 1e-3);
    e.tween_ui.time = 0.5;
    e.tween_ui.preview = true;
    let frame = e.render_frame(&mut runtime, screen);
    assert!((yaw_of(&frame) - 45.0).abs() < 1e-2, "{}", yaw_of(&frame));
    // The simulation keeps the authored pose and nothing was written.
    let q = glam::Quat::from_array(runtime.state.transforms[slot].as_ref().unwrap().rotation);
    assert!(axiom_core::tween::yaw_degrees(q).abs() < 1e-3);
    assert_eq!(e.rev, 1);
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

#[test]
fn camera_follow_and_main_camera_match_acl() {
    // Object ▸ Camera Follow Selected: the selected camera(s) follow the one selected
    // non-camera object (selection order of the cameras is kept).
    let start = headless(
        &lab(),
        "create camera id=rig\ncreate camera id=rig2\ncreate cube id=hero_box at=0,0.5,-2",
    );
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("rig".into()));
            e.extra.insert("hero_box".into());
            vec![e.run_command("object.camera_follow").unwrap()]
        },
        "camera rig follow hero_box",
    );
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("hero_box".into()));
            e.extra.insert("rig2".into());
            e.extra.insert("rig".into());
            vec![e.run_command("object.camera_follow").unwrap()]
        },
        "camera entities[id=rig,id=rig2] follow hero_box",
    );
    let following = headless(&start, "camera rig follow hero_box");
    assert_eq!(following.capabilities.get("camera"), Some(&1));
    assert!(following.systems.iter().any(|s| s == "camera.rig"));
    // Empty world: the camera capability and camera.rig are added once.
    let blank = headless(
        &World::empty("blank"),
        "create camera id=cam\ncreate cube id=box",
    );
    assert_parity(
        blank.clone(),
        |e| {
            e.select(Some("cam".into()));
            e.extra.insert("box".into());
            vec![e.run_command("object.camera_follow").unwrap()]
        },
        "camera cam follow box",
    );
    // Inspector form edit of the follow (nested collision radius too) = ACL set.
    assert_parity(
        following.clone(),
        |e| {
            e.select(Some("rig".into()));
            let entity = e.world.entity("rig").unwrap().clone();
            let world = e.world.clone();
            let draft = e.draft.as_mut().unwrap();
            draft.open_component(&world, &entity, "camera.follow");
            let form = draft.form.as_mut().unwrap();
            form["distance"] = serde_json::json!(7.5);
            form["collision"]["radius"] = serde_json::json!(0.4);
            draft.form_dirty = true;
            let edits = draft.edits().unwrap();
            assert_eq!(edits.len(), 2);
            vec![vec![e.write(edits, None)]]
        },
        "set rig camera.follow/distance 7.5\nset rig camera.follow/collision/radius 0.4",
    );
    // Set as Main Camera on the primary selection = camera main ID.
    assert_parity(
        following.clone(),
        |e| {
            e.select(Some("rig".into()));
            vec![e.run_command("object.main_camera").unwrap()]
        },
        "camera main rig",
    );
    // No target, two targets, or no camera: a problem, not a write.
    let mut e = Editor::new(start.clone(), 1, Some("rig".into()));
    assert!(e.run_command("object.camera_follow").unwrap().is_empty());
    e.extra.insert("hero_box".into());
    e.extra.insert("stage/floor".into());
    assert!(e.run_command("object.camera_follow").unwrap().is_empty());
    let mut e = Editor::new(start.clone(), 1, Some("hero_box".into()));
    e.extra.insert("stage/floor".into());
    assert!(e.run_command("object.camera_follow").unwrap().is_empty());
    let mut e = Editor::new(start, 1, Some("hero_box".into()));
    assert!(e.run_command("object.main_camera").unwrap().is_empty());
}

#[test]
fn tags_match_acl() {
    // Multi-selection: Object ▸ Edit Tags (editor_view {command, arg}) on both objects.
    assert_parity(
        lab(),
        |e| {
            e.select(Some("stage/crate".into()));
            e.extra.insert("stage/rack".into());
            vec![e
                .run_command_with("object.tag", Some("+player +npc"))
                .unwrap()]
        },
        "tag entities[id=stage/crate,id=stage/rack] +player +npc",
    );
    // Removing keeps the other tags and their order (Inspector chip x = edit_tags remove).
    let tagged = headless(&lab(), "tag stage/crate +a +b +c");
    assert_parity(
        tagged.clone(),
        |e| {
            e.select(Some("stage/crate".into()));
            vec![e.edit_tags(&[], &["b".into()]).into_iter().collect()]
        },
        "tag stage/crate -b",
    );
    assert_eq!(
        headless(&tagged, "tag stage/crate -b +player")
            .entity("stage/crate")
            .unwrap()
            .tags,
        ["a", "c", "player"]
    );
    // Nothing to change: no write in either path.
    let mut e = Editor::new(tagged, 1, Some("stage/crate".into()));
    assert!(e.edit_tags(&["a".into()], &[]).is_none());
}

#[test]
fn world_panel_operations_match_acl() {
    let blank = World::empty("blank");
    // World ▸ ID / Title, capabilities, systems (with their capability and `after` systems),
    // resources (default or typed JSON) — the World window calls the same methods.
    for (command, arg, script) in [
        ("world.set_id", "street", "world id street"),
        ("world.set_title", "黃昏街口 / Street", "world title \"黃昏街口 / Street\""),
        ("world.capability_add", "physics", "capability add physics"),
        ("world.system_add", "trigger.detect", "system add trigger.detect"),
        ("world.resource_create", "nav.grid", "resource create nav.grid"),
        ("world.resource_create", "behavior.program", "resource create behavior.program"),
        (
            "world.resource_create",
            "render.environment {\"ambient\":{\"mode\":\"color\",\"color\":[0.2,0.2,0.2],\"intensity\":1}}",
            "resource create render.environment {\"ambient\":{\"mode\":\"color\",\"color\":[0.2,0.2,0.2],\"intensity\":1}}",
        ),
    ] {
        assert_parity(
            blank.clone(),
            |e| vec![e.run_command_with(command, Some(arg)).unwrap()],
            script,
        );
    }
    let program = headless(&blank, "resource create behavior.program");
    assert!(program.systems.iter().any(|s| s == "behavior.execute"));
    assert_eq!(program.capabilities.get("behavior"), Some(&1));

    // Removals.
    let full = headless(
        &blank,
        "capability add physics\nsystem add trigger.detect\nresource create nav.grid",
    );
    for (command, arg, script) in [
        (
            "world.capability_remove",
            "physics",
            "capability remove physics",
        ),
        (
            "world.system_remove",
            "trigger.detect",
            "system remove trigger.detect",
        ),
        (
            "world.resource_remove",
            "nav.grid",
            "resource remove nav.grid",
        ),
    ] {
        assert_parity(
            full.clone(),
            |e| vec![e.run_command_with(command, Some(arg)).unwrap()],
            script,
        );
    }
    // In use: a problem, not a write (and the same structured code headless).
    let mut e = Editor::new(full.clone(), 1, None);
    e.set_component_hints(&crate::installed_registry().authoring);
    assert!(e
        .run_command_with("world.capability_remove", Some("trigger"))
        .unwrap()
        .is_empty());
    assert_eq!(e.problem_codes()[0], "capability_in_use");
    let mut s = store(&full);
    let error = s
        .execute(Request::Acl {
            text: "capability remove trigger".into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .unwrap_err();
    assert_eq!(
        axiom_core::error_response(&error)["diagnostics"][0]["code"],
        "capability_in_use"
    );
}

#[test]
fn add_component_brings_its_capability_and_systems_like_acl() {
    // Inspector ▸ Add component on a world without the trigger capability or system.
    let start = headless(
        &World::empty("blank"),
        "create empty id=gate\ncreate empty id=door",
    );
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("gate".into()));
            e.extra.insert("door".into());
            vec![e.add_component("trigger.zone").into_iter().collect()]
        },
        "add entities[id=gate,id=door] trigger.zone",
    );
    let made = headless(&start, "add gate trigger.zone");
    assert_eq!(made.capabilities.get("trigger"), Some(&1));
    assert!(made.systems.iter().any(|s| s == "trigger.detect"));
    // physics.collider: capability only; the component then opens in the Inspector form.
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("gate".into()));
            vec![e
                .run_command_with("object.add_component", Some("physics.collider"))
                .unwrap()]
        },
        "add gate physics.collider",
    );
    let mut e = Editor::new(start.clone(), 1, Some("gate".into()));
    e.set_component_hints(&crate::installed_registry().authoring);
    let mut s = store(&start);
    let added = e.add_component("physics.collider").into_iter().collect();
    commit(&mut e, &mut s, added);
    assert_eq!(e.draft.as_ref().unwrap().component, "physics.collider");
    // No default (camera.follow needs a target): the JSON draft opens; Stage component +
    // Apply writes it with the camera capability and camera.rig, like `add SEL COMP JSON`.
    let cameras = headless(&start, "create camera id=cam");
    assert!(!cameras.capabilities.contains_key("camera"));
    assert_parity(
        cameras,
        |e| {
            e.select(Some("cam".into()));
            assert!(e.add_component("camera.follow").is_none());
            let draft = e.draft.as_mut().unwrap();
            assert!(draft.json_mode && draft.component == "camera.follow");
            draft.json = r#"{"target":"door"}"#.into();
            draft.component_dirty = true;
            let edits = super::inspector_edits(&e.world, &e.registry, draft, &e.extra).unwrap();
            vec![vec![e.write(edits, None)]]
        },
        r#"add cam camera.follow {"target":"door"}"#,
    );
}

#[test]
fn declare_event_and_rotation_match_acl() {
    let start = headless(&lab(), "create trigger box id=gate");
    assert_parity(
        start.clone(),
        |e| {
            vec![e
                .run_command_with("event.declare", Some("door_bump"))
                .unwrap()]
        },
        "event declare door_bump",
    );
    // The commit error names the event; the Problems button declares exactly it.
    let mut s = store(&start);
    let error = s
        .execute(Request::Acl {
            text: "set gate trigger.zone/on_enter \"door_bump\"".into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("event declare door_bump"), "{message}");
    assert_eq!(
        axiom_core::ops::undeclared_event(&message),
        Some("door_bump")
    );
    // Inspector Rotation° (Euler degrees) = ACL rotate SEL x,y,z.
    assert_parity(
        lab(),
        |e| {
            e.select(Some("stage/crate".into()));
            let draft = e.draft.as_mut().unwrap();
            draft.angles = [0.0, 90.0, 0.0];
            draft.transform.as_mut().unwrap().rotation = super::viewport::rotation(draft.angles);
            draft.transform_dirty = true;
            let edits = draft.edits().unwrap();
            vec![vec![e.write(edits, None)]]
        },
        "rotate stage/crate 0,90,0",
    );
    // create … rot= = create, then Rotation° in the Inspector.
    let rotated = headless(&lab(), "create cube id=c rot=0,90,0");
    let turned = headless(&lab(), "create cube id=c\nrotate c 0,90,0");
    assert_eq!(rotated, turned);
}

#[test]
fn multi_object_inspector_edits_match_acl() {
    let start = headless(
        &lab(),
        "create cube id=a at=1,0,0\ncreate cube id=b at=5,2,3\ncreate trigger box id=t1\ncreate trigger box id=t2",
    );
    // Position X typed with two cubes selected: X set on both, Y/Z of each kept.
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("a".into()));
            e.extra.insert("b".into());
            let draft = e.draft.as_mut().unwrap();
            draft.transform.as_mut().unwrap().position[0] = 9.0;
            draft.transform_dirty = true;
            let edits = super::inspector_edits(&e.world, &e.registry, draft, &e.extra).unwrap();
            vec![vec![e.write(edits, None)]]
        },
        "set a core.transform/position [9.0,0.0,0.0]\nset b core.transform/position [9.0,2.0,3.0]",
    );
    // A component form field on two triggers = set entities[id=..] FIELD VALUE.
    assert_parity(
        start,
        |e| {
            e.select(Some("t1".into()));
            e.extra.insert("t2".into());
            let entity = e.world.entity("t1").unwrap().clone();
            let world = e.world.clone();
            let draft = e.draft.as_mut().unwrap();
            draft.open_component(&world, &entity, "trigger.zone");
            draft.form.as_mut().unwrap()["once"] = serde_json::json!(true);
            draft.form_dirty = true;
            let edits = super::inspector_edits(&e.world, &e.registry, draft, &e.extra).unwrap();
            assert_eq!(edits.len(), 2);
            vec![vec![e.write(edits, None)]]
        },
        "set entities[id=t1,id=t2] trigger.zone/once true",
    );
}

#[test]
fn resource_form_edits_match_acl() {
    let start = headless(&World::empty("blank"), "resource create nav.grid");
    assert_parity(
        start,
        |e| {
            e.open_resource("nav.grid");
            let draft = e.resource.as_mut().unwrap();
            assert!(draft.form_mode, "resources open as a schema-guided form");
            draft.form.as_mut().unwrap()["cell_size"] = serde_json::json!(0.5);
            draft.dirty = true;
            let request = draft.request(false).unwrap();
            vec![vec![Action::Write {
                request,
                select: None,
            }]]
        },
        "set resources[id=nav.grid] cell_size 0.5",
    );
}

#[test]
fn declare_event_applies_the_waiting_draft_and_multi_edits_skip_mismatched_fields() {
    // Inspector: on_enter typed as an undeclared event, Apply refused; Problems ▸ Declare
    // event writes the declaration and the draft in one revision (the draft is not lost).
    let start = headless(&lab(), "create trigger box id=gate");
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("gate".into()));
            let entity = e.world.entity("gate").unwrap().clone();
            let world = e.world.clone();
            let draft = e.draft.as_mut().unwrap();
            draft.open_component(&world, &entity, "trigger.zone");
            draft.form.as_mut().unwrap()["on_enter"] = serde_json::json!("door_bump");
            draft.form_dirty = true;
            vec![e.declare_event("door_bump").into_iter().collect()]
        },
        "event declare door_bump\nset gate trigger.zone/on_enter \"door_bump\"",
    );
    // Multi-object form edit: a box collider's half_extents is not written into a capsule.
    let colliders = headless(
        &lab(),
        "create cube id=a\ncreate cube id=b\nadd a physics.collider\nadd b physics.collider {\"shape\":{\"shape\":\"capsule\",\"half_height\":0.5,\"radius\":0.3}}",
    );
    assert_parity(
        colliders,
        |e| {
            e.select(Some("a".into()));
            e.extra.insert("b".into());
            let entity = e.world.entity("a").unwrap().clone();
            let world = e.world.clone();
            let draft = e.draft.as_mut().unwrap();
            draft.open_component(&world, &entity, "physics.collider");
            draft.form.as_mut().unwrap()["shape"]["half_extents"] =
                serde_json::json!([1.0, 0.5, 0.5]);
            draft.form_dirty = true;
            let edits = super::inspector_edits(&e.world, &e.registry, draft, &e.extra).unwrap();
            vec![vec![e.write(edits, None)]]
        },
        "set a physics.collider/shape/half_extents [1.0,0.5,0.5]",
    );
}

#[test]
fn inspector_arrays_reset_paste_and_play_on_match_acl() {
    use axiom_core::ops::ArrayEdit;
    // Form array buttons on a component = append / insert / reorder / remove.
    let parts = lab().entity("stage/rack").unwrap().components["render.mesh"]["parts"].clone();
    for (edit, script) in [
        (
            ArrayEdit::Append(parts[0].clone()),
            format!("append stage/rack render.mesh/parts {}", parts[0]),
        ),
        (
            ArrayEdit::Insert(0, parts[0].clone()),
            format!("insert stage/rack render.mesh/parts 0 {}", parts[0]),
        ),
        (
            ArrayEdit::Move(0, 1),
            "reorder stage/rack render.mesh/parts 0 1".to_owned(),
        ),
        (
            ArrayEdit::Remove(0),
            "remove stage/rack render.mesh/parts 0".to_owned(),
        ),
    ] {
        assert_parity(
            lab(),
            |e| {
                e.select(Some("stage/rack".into()));
                vec![e
                    .edit_component_array("render.mesh", "/parts", &edit)
                    .into_iter()
                    .collect()]
            },
            &script,
        );
    }
    // Component header: Reset and Copy / Paste values on the other selected light.
    let lights = headless(
        &lab(),
        "create light point id=lamp\ncreate light point id=lamp2\nset lamp render.light/intensity 7",
    );
    assert_parity(
        lights.clone(),
        |e| {
            e.select(Some("lamp".into()));
            vec![e.reset_component("render.light").into_iter().collect()]
        },
        "reset lamp render.light",
    );
    let copied = lights.entity("lamp").unwrap().components["render.light"].clone();
    assert_parity(
        lights,
        |e| {
            e.select(Some("lamp".into()));
            e.copy_component("render.light");
            e.select(Some("lamp2".into()));
            vec![e.paste_component("render.light").into_iter().collect()]
        },
        &format!("set lamp2 /components/render.light {copied}"),
    );
    // Timeline ▸ Play on: bind = `tween .. set play_on=E`; ✕ = remove the binding.
    let door = headless(&lab(), "create cube id=door\ntween door clip open");
    assert_parity(
        door.clone(),
        |e| {
            e.select(Some("door".into()));
            let options = axiom_core::ops::TweenClipOptions {
                play_on: Some("go".into()),
                ..Default::default()
            };
            vec![e.set_tween_options("open", &options).into_iter().collect()]
        },
        "tween door clip open set play_on=go",
    );
    let bound = headless(&door, "tween door clip open set play_on=go");
    assert_parity(
        bound,
        |e| {
            e.select(Some("door".into()));
            let plays = super::timeline::play_ons(&e.world, "door", "open");
            assert_eq!(plays.len(), 1);
            vec![e
                .edit_resource_array(
                    "runtime.bindings",
                    "/events",
                    &ArrayEdit::Remove(plays[0].binding),
                )
                .into_iter()
                .collect()]
        },
        "remove resources[id=runtime.bindings] events 0",
    );
}

#[test]
fn state_machine_commands_and_templates_match_acl() {
    // A world without the fsm capability: FSM Agent with a new machine adds the machine,
    // the capability and fsm.step, like `fsm SEL agent M`.
    let start = headless(&lab(), "create cube id=bot\ncreate cube id=pal");
    assert!(!start.capabilities.contains_key("fsm"));
    assert_parity(
        start.clone(),
        |e| {
            e.select(Some("bot".into()));
            e.extra.insert("pal".into());
            vec![e
                .run_command_with("behavior.fsm_agent", Some("sentry"))
                .unwrap()]
        },
        "fsm entities[id=bot,id=pal] agent sentry",
    );
    let made = headless(&start, "fsm bot agent sentry");
    assert!(made.systems.iter().any(|s| s == "fsm.step"));
    // Templates with their defaults (shortcut / bridge path), on one and on two objects.
    for template in ["patrol", "guard", "dialogue"] {
        assert_parity(
            start.clone(),
            |e| {
                e.select(Some("bot".into()));
                vec![e.run_command(&format!("behavior.{template}")).unwrap()]
            },
            &format!("behavior bot {template}"),
        );
    }
    // A second template of the same kind gets the next free machine ID.
    let patrolled = headless(&start, "behavior bot patrol");
    assert_parity(
        patrolled.clone(),
        |e| {
            e.select(Some("pal".into()));
            vec![e.run_command("behavior.patrol").unwrap()]
        },
        "behavior pal patrol machine=patrol_2",
    );
    // Graph toolbar commands on the open machine.
    assert_parity(
        patrolled.clone(),
        |e| {
            e.run_command_with("graph.open", Some("patrol")).unwrap();
            vec![e.run_command("graph.add_state").unwrap()]
        },
        "fsm patrol state add state",
    );
    assert_parity(
        patrolled.clone(),
        |e| {
            e.run_command_with("graph.open", Some("patrol")).unwrap();
            e.graph.selected = Some(super::graph::Sel::State("pause".into()));
            let initial = e.run_command("graph.set_initial").unwrap();
            vec![initial]
        },
        "fsm patrol initial pause",
    );
    assert_parity(
        patrolled,
        |e| {
            e.run_command_with("graph.open", Some("patrol")).unwrap();
            e.graph.selected = Some(super::graph::Sel::Transition(1));
            vec![e.run_command("graph.delete").unwrap()]
        },
        "fsm patrol transition remove 1",
    );
    // Without an open machine or selection the graph commands explain why.
    let mut e = Editor::new(lab(), 1, None);
    assert_eq!(
        e.command_blocked("graph.add_state"),
        Some("open a state machine in the Graphs tab")
    );
    assert!(e.run_command("behavior.patrol").is_err());
    assert!(e.run_command_with("graph.open", Some("nope")).is_err());
}

#[test]
fn graph_edits_made_in_one_frame_build_on_each_other() {
    // Two whole-resource graph writes before either is shown: the second includes the
    // first (one coalesced batch), like the two-line ACL script.
    let start = headless(&lab(), "create cube id=bot\nbehavior bot patrol");
    assert_parity(
        start,
        |e| {
            e.run_command_with("graph.open", Some("patrol")).unwrap();
            let mut frame = e.run_command("graph.add_state").unwrap();
            e.graph.selected = Some(super::graph::Sel::State("pause".into()));
            frame.extend(e.run_command("graph.set_initial").unwrap());
            assert_eq!(frame.len(), 2);
            vec![super::coalesce(frame)]
        },
        "fsm patrol state add state\nfsm patrol initial pause",
    );
}
