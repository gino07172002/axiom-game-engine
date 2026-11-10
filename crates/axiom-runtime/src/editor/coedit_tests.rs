//! M10 co-editing: change ghosts, field-wise merges, context menus — driven by real egui input
//! and checked against the headless protocol (`resolve_proposal {keep}`, ACL).
use super::ghost::{GhostKind, GhostSource};
use super::{merge, Action, Dock, Editor};
use crate::world::WorldRuntime;
use axiom_core::{Request, Store, World};
use egui::{vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use glam::Vec3;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const CRATE_POSITION: &str = "/entities/@stage~1crate/components/core.transform/position";

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

fn store(world: &World) -> Store {
    Store::memory_with(crate::installed_registry().authoring, world.clone()).unwrap()
}

fn acl(store: &mut Store, text: &str, propose: bool) -> Value {
    store
        .execute(Request::Acl {
            text: text.into(),
            preview: false,
            expected_rev: None,
            propose,
        })
        .unwrap_or_else(|e| panic!("{text}: {e:#}"))
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
}

impl Harness {
    fn new(world: World, rev: i64) -> Self {
        let runtime = WorldRuntime::new(world.clone(), rev, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world, rev, None);
        editor.set_component_hints(&crate::installed_registry().authoring);
        Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        actions
    }
    fn rect(&self, key: &str) -> Rect {
        *self
            .editor
            .rects
            .get(key)
            .unwrap_or_else(|| panic!("{key} not drawn; have {:?}", self.editor.rects.keys()))
    }
    fn click(&mut self, at: Pos2, button: PointerButton) -> Vec<Action> {
        let press = |pressed| Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut actions = self.frame(vec![Event::PointerMoved(at), press(true)]);
        actions.extend(self.frame(vec![press(false)]));
        actions
    }
    /// Frames (with the host's job polling) until `done` holds.
    fn wait(&mut self, what: &str, done: impl Fn(&Editor) -> bool) -> Vec<Action> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut actions = vec![];
        while !done(&self.editor) {
            assert!(Instant::now() < deadline, "{what} never finished");
            std::thread::sleep(Duration::from_millis(2));
            self.editor.poll_jobs();
            actions.extend(self.frame(vec![]));
        }
        actions
    }
    /// The host's handling of the store-reading actions (`App::editor_action_now`).
    fn host(&mut self, store: &Store, actions: &[Action]) {
        for action in actions {
            match action {
                Action::LoadGhosts { source } => {
                    super::ghost::load(&mut self.editor, store, *source).unwrap()
                }
                Action::PrepareMerge { proposal } => {
                    merge::prepare_proposal(&mut self.editor, store, *proposal).unwrap()
                }
                _ => {}
            }
        }
    }
}

fn proposals(store: &mut Store) -> ((i64, i64), Value) {
    let rows = store
        .execute(Request::Proposals {
            status: Some("pending".into()),
            limit: Some(32),
        })
        .unwrap();
    (
        store.proposal_signature().unwrap(),
        rows["proposals"].clone(),
    )
}

/// A proposal moving the crate (and renaming it), then a newer revision moving it elsewhere.
fn stale_proposal() -> (World, Store) {
    let start = lab();
    let mut s = store(&start);
    acl(
        &mut s,
        "set stage/crate core.transform/position [3,0.63,0.82]; rename stage/crate Proposed",
        true,
    );
    acl(
        &mut s,
        "set stage/crate core.transform/position [0,0.63,0.82]; set stage/beacon core.transform/position [2,0,0]",
        false,
    );
    (start, s)
}

#[test]
fn focused_proposal_shows_ghost_boxes_computed_once_and_history_shows_revision_ghosts() {
    let (_, mut s) = stale_proposal();
    let (rev, world) = s.current().unwrap();
    let mut h = Harness::new(world, rev);
    let (signature, rows) = proposals(&mut s);
    h.editor.set_proposals(signature, &rows);
    assert_eq!(h.editor.ghost.source, Some(GhostSource::Proposal(1)));
    let actions = h.frame(vec![]);
    assert!(
        actions.iter().any(|a| matches!(
            a,
            Action::LoadGhosts {
                source: GhostSource::Proposal(1)
            }
        )),
        "the host is asked to load the ghosts once"
    );
    h.host(&s, &actions);
    h.wait("proposal ghosts", |e| e.ghost.drawn > 0);
    let ghosts = h.editor.ghosts_now().unwrap();
    assert_eq!(ghosts.total, 1);
    let g = &ghosts.boxes[0];
    assert_eq!((g.id.as_str(), g.kind), ("stage/crate", GhostKind::Moved));
    // Before = where the crate is now (x 0), after = where the proposal puts it (x 3).
    let x = |m: Option<glam::Mat4>| m.unwrap().transform_point3(Vec3::ZERO).x;
    assert!((x(g.before) - 0.0).abs() < 0.2 && (x(g.after) - 3.0).abs() < 0.2);
    // Drawing is cached: 30 more frames, no reload and no rebuild, nothing written.
    let builds = h.editor.ghost.builds;
    for _ in 0..30 {
        let actions = h.frame(vec![]);
        assert!(actions.is_empty(), "{} action(s)", actions.len());
        h.editor.poll_jobs();
    }
    assert_eq!(h.editor.ghost.builds, builds);
    assert_eq!(h.editor.ghost.drawn, 1);
    let observed = h.editor.observation()["ghosts"].clone();
    assert_eq!(
        observed,
        json!({"source":"proposal:1","state":"ready","boxes":1,"total":1,"drawn":1,"error":null})
    );
    // History: clicking revision 2 shows its ghosts (crate and beacon moved).
    h.editor
        .set_history(&s.execute(Request::History { limit: Some(8) }).unwrap()["revisions"]);
    h.editor.show_dock(Dock::History);
    h.frame(vec![]);
    let row = h.rect("history:2").center();
    let actions = h.click(row, PointerButton::Primary);
    assert!(actions
        .iter()
        .any(|a| matches!(a, Action::LoadDiff { rev: 2 })));
    let load: Vec<Action> = actions
        .into_iter()
        .filter(|a| matches!(a, Action::LoadGhosts { .. }))
        .collect();
    assert!(matches!(
        load.as_slice(),
        [Action::LoadGhosts {
            source: GhostSource::Revision(2)
        }]
    ));
    h.host(&s, &load);
    h.wait("revision ghosts", |e| {
        e.ghost_observation()["state"] == "ready" && e.ghost_observation()["source"] == "rev:2"
    });
    let ids: Vec<String> = h
        .editor
        .ghosts_now()
        .unwrap()
        .boxes
        .iter()
        .map(|g| g.id.clone())
        .collect();
    assert_eq!(ids, ["stage/crate", "stage/beacon"]);
    // Agents drive the same view state: ghost.show / view.ghosts; off draws nothing.
    h.editor
        .run_command_with("ghost.show", Some("off"))
        .unwrap();
    h.frame(vec![]);
    assert_eq!(h.editor.ghost.drawn, 0);
    // An unknown proposal is reported, not drawn.
    let actions = h
        .editor
        .run_command_with("ghost.show", Some("proposal:9"))
        .unwrap();
    let [Action::LoadGhosts { source }] = actions.as_slice() else {
        panic!("expected a ghost load");
    };
    let error = super::ghost::load(&mut h.editor, &s, *source).unwrap_err();
    h.editor.ghost_failed(*source, &error);
    assert_eq!(h.editor.ghost_observation()["state"], "error");
    h.frame(vec![]);
    assert_eq!(h.editor.ghost.drawn, 0);
    assert!(h
        .editor
        .run_command_with("ghost.show", Some("banana"))
        .is_err());
    assert_eq!(s.revision().unwrap(), rev, "ghosts never write");
}

#[test]
fn stale_proposal_merge_picks_per_field_and_approves_as_one_revision() {
    let (start, mut s) = stale_proposal();
    let (rev, world) = s.current().unwrap();
    let mut h = Harness::new(world, rev);
    let (signature, rows) = proposals(&mut s);
    h.editor.set_proposals(signature, &rows);
    h.editor.set_ghost_source(None);
    h.editor.show_dock(Dock::Proposals);
    h.frame(vec![]);
    let actions = h.click(h.rect("proposal:merge:1").center(), PointerButton::Primary);
    assert!(matches!(
        actions.as_slice(),
        [Action::PrepareMerge { proposal: 1 }]
    ));
    h.host(&s, &actions);
    h.wait("merge listing", |e| e.merge.prepared.is_some());
    h.frame(vec![]);
    let observed = h.editor.observation()["merge"].clone();
    assert_eq!(observed["state"], "ready");
    assert_eq!(
        observed["conflicts"],
        json!([{"path":CRATE_POSITION,"pick":"mine"}])
    );
    // The label is the proposal's alone; the beacon is the newer revision's alone.
    assert_eq!(
        (
            observed["mine_only"].clone(),
            observed["theirs_only"].clone()
        ),
        (json!(1), json!(1))
    );
    h.click(h.rect("merge:theirs:0").center(), PointerButton::Primary);
    assert_eq!(
        h.editor.observation()["merge"]["conflicts"][0]["pick"],
        "theirs"
    );
    let actions = h.click(h.rect("merge:commit").center(), PointerButton::Primary);
    let [Action::ResolveProposal {
        id: 1,
        approve: true,
        rebase: false,
        keep: Some(keep),
        expected_rev: Some(listed),
    }] = actions.as_slice()
    else {
        panic!(
            "expected one merged approval, got {} action(s)",
            actions.len()
        );
    };
    assert_eq!(keep, &[CRATE_POSITION.to_owned()]);
    assert_eq!(
        *listed, rev,
        "the revision the conflicts were listed against"
    );
    assert_eq!(h.editor.merge.source, None, "the merge window closes");
    let done = s
        .execute(Request::ResolveProposal {
            id: 1,
            approve: true,
            rebase: false,
            keep: Some(keep.clone()),
            expected_rev: Some(*listed),
        })
        .unwrap();
    assert_eq!(done["rev"], rev + 1, "one revision");
    let merged = s.current().unwrap().1;
    let crate_ = merged.entity("stage/crate").unwrap();
    assert_eq!(crate_.label.as_deref(), Some("Proposed"));
    assert_eq!(
        crate_.components["core.transform"]["position"],
        json!([0, 0.63, 0.82]),
        "theirs kept"
    );
    // Headless: the ACL lists the same conflict; the protocol approval leaves the same world.
    let (_, mut headless) = stale_proposal();
    let text = acl(&mut headless, "conflicts 1", false)["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.contains(CRATE_POSITION), "{text}");
    headless
        .execute(Request::ResolveProposal {
            id: 1,
            approve: true,
            rebase: false,
            keep: Some(vec![CRATE_POSITION.into()]),
            expected_rev: Some(2),
        })
        .unwrap();
    assert_eq!(headless.current().unwrap().1, merged);
    assert_ne!(merged, start);
}

#[test]
fn stale_inspector_draft_merges_field_by_field_like_a_proposal() {
    let start = lab();
    let mut s = store(&start);
    let mut h = Harness::new(start.clone(), 1);
    h.editor.prefs.live_inspector = false;
    h.editor.select(Some("stage/crate".into()));
    {
        let d = h.editor.draft.as_mut().unwrap();
        let mut t = d.transform.clone().unwrap();
        t.position = [5.0, 0.63, 0.82];
        d.transform = Some(t);
        d.transform_dirty = true;
        d.label = "Mine".into();
        d.label_dirty = true;
    }
    let draft_edits = super::inspector_edits(
        &start,
        &h.editor.registry,
        h.editor.draft.as_ref().unwrap(),
        &h.editor.extra,
    )
    .unwrap();
    // An agent commits meanwhile: the draft goes stale.
    acl(
        &mut s,
        "set stage/crate core.transform/position [0,0.63,0.82]; set stage/beacon core.transform/position [2,0,0]",
        false,
    );
    let (rev, world) = s.current().unwrap();
    h.editor.update(rev, world, false);
    assert!(h.editor.dirty() && h.editor.merge_base.is_some());
    h.frame(vec![]);
    assert!(h
        .click(h.rect("inspector:merge").center(), PointerButton::Primary)
        .is_empty());
    h.wait("draft merge listing", |e| e.merge.prepared.is_some());
    let conflicts: Vec<String> = h
        .editor
        .merge
        .prepared
        .as_ref()
        .unwrap()
        .view
        .conflicts
        .iter()
        .map(|c| c.path.clone())
        .collect();
    assert_eq!(conflicts, [CRATE_POSITION]);
    // The agent's way to pick (editor_command) and the person's click are the same command.
    h.editor
        .run_command_with("merge.pick", Some(&format!("{CRATE_POSITION}=theirs")))
        .unwrap();
    assert!(h
        .editor
        .run_command_with("merge.pick", Some("/nope=mine"))
        .is_err());
    // Editing the draft after the conflicts were listed: Commit lists them again, no write.
    h.editor.draft.as_mut().unwrap().label = "Mine (2)".into();
    assert!(h.editor.run_command("merge.commit").unwrap().is_empty());
    assert!(h.editor.merge.prepared.is_none() && h.editor.merge.job.is_some());
    h.wait("draft merge relisting", |e| e.merge.prepared.is_some());
    h.editor.draft.as_mut().unwrap().label = "Mine".into();
    h.editor.run_command("merge.commit").unwrap();
    h.wait("draft merge relisting", |e| e.merge.prepared.is_some());
    h.editor
        .run_command_with("merge.pick", Some(&format!("{CRATE_POSITION}=theirs")))
        .unwrap();
    h.frame(vec![]);
    assert!(h
        .click(h.rect("merge:commit").center(), PointerButton::Primary)
        .is_empty());
    let actions = h.wait("draft merge commit", |e| e.merge.source.is_none());
    let [Action::Write {
        request: Request::Apply {
            expected_rev,
            edits,
        },
        ..
    }] = actions.as_slice()
    else {
        panic!("expected one write, got {} action(s)", actions.len());
    };
    assert_eq!(*expected_rev, Some(rev));
    s.execute(Request::Apply {
        expected_rev: *expected_rev,
        edits: edits.clone(),
    })
    .unwrap();
    let merged = s.current().unwrap().1;
    let crate_ = merged.entity("stage/crate").unwrap();
    assert_eq!(crate_.label.as_deref(), Some("Mine"));
    assert_eq!(
        crate_.components["core.transform"]["position"],
        json!([0, 0.63, 0.82])
    );
    // Headless twin: the draft's edits proposed on the same base, merged with the same pick.
    let mut headless = store(&start);
    headless
        .execute(Request::Propose {
            expected_rev: Some(1),
            edits: draft_edits,
            note: "draft".into(),
        })
        .unwrap();
    acl(
        &mut headless,
        "set stage/crate core.transform/position [0,0.63,0.82]; set stage/beacon core.transform/position [2,0,0]",
        false,
    );
    headless
        .execute(Request::ResolveProposal {
            id: 1,
            approve: true,
            rebase: false,
            keep: Some(vec![CRATE_POSITION.into()]),
            expected_rev: Some(2),
        })
        .unwrap();
    assert_eq!(headless.current().unwrap().1, merged);
}

#[test]
fn right_click_menus_hand_the_selection_to_an_agent() {
    let mut h = Harness::new(lab(), 1);
    h.frame(vec![]);
    h.frame(vec![]);
    // Scene view: right-click the crate -> it is selected and the menu opens.
    let model = h.runtime.state.matrices()[h.runtime.state.compiled.indices["stage/crate"]];
    let at = h
        .editor
        .scene_point(model.transform_point3(Vec3::ZERO))
        .unwrap();
    h.click(at, PointerButton::Secondary);
    assert_eq!(h.editor.selected.as_deref(), Some("stage/crate"));
    h.frame(vec![]);
    let item = h.rect("scene_menu:file.copy_context").center();
    let copy = h.click(item, PointerButton::Primary);
    assert!(copy.is_empty(), "copying writes nothing");
    assert!(
        h.editor.status.starts_with("Copied context for 1 object"),
        "{}",
        h.editor.status
    );
    assert!(
        h.editor.scene_input.menu.is_none(),
        "the menu closes after a pick"
    );
    // A right-button press that flew the camera (RMB + W) opens no menu on release.
    let press = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Secondary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let w = |pressed| Event::Key {
        key: Key::W,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    h.frame(vec![Event::PointerMoved(at), press(true)]);
    h.frame(vec![w(true)]);
    h.frame(vec![w(false), press(false)]);
    h.frame(vec![]);
    assert!(h.editor.scene_input.menu.is_none(), "no menu after flying");
    // Hierarchy: the row's context menu has the same entry.
    h.editor.status.clear();
    let row = h.rect("label:stage/beacon").center();
    h.click(row, PointerButton::Secondary);
    h.frame(vec![]);
    h.click(
        h.rect("menu:file.copy_context").center(),
        PointerButton::Primary,
    );
    assert_eq!(h.editor.selected.as_deref(), Some("stage/beacon"));
    assert!(
        h.editor.status.starts_with("Copied context for 1 object"),
        "{}",
        h.editor.status
    );
    assert!(h.editor.context_text().contains("stage/beacon"));
}

#[test]
fn merge_and_ghost_commands_explain_why_not_and_merge_commit_waits_for_play() {
    let (_, mut s) = stale_proposal();
    let (rev, world) = s.current().unwrap();
    let mut h = Harness::new(world, rev);
    for id in ["merge.pick", "merge.commit", "merge.cancel"] {
        assert!(h.editor.command_blocked(id).is_some(), "{id}");
    }
    assert!(h
        .editor
        .run_command_with("merge.open", Some("draft"))
        .is_err_and(|e| format!("{e:#}").contains("no stale Inspector draft")));
    let (signature, rows) = proposals(&mut s);
    h.editor.set_proposals(signature, &rows);
    let actions = h
        .editor
        .run_command_with("merge.open", Some("proposal:1"))
        .unwrap();
    h.host(&s, &actions);
    h.wait("merge listing", |e| e.merge.prepared.is_some());
    assert!(h.editor.command_blocked("merge.commit").is_none());
    h.editor.previewing = true;
    assert!(
        h.editor.command_blocked("merge.commit").is_some(),
        "content command during Play"
    );
    h.editor.previewing = false;
    // A newer revision arrives: commit lists the conflicts again instead of writing.
    acl(
        &mut s,
        "set stage/beacon core.transform/position [1,0,0]",
        false,
    );
    let (rev, world) = s.current().unwrap();
    h.editor.update(rev, world, false);
    let again = h.editor.run_command("merge.commit").unwrap();
    assert!(matches!(
        again.as_slice(),
        [Action::PrepareMerge { proposal: 1 }]
    ));
}

/// Cost of the M10 additions, release build
/// (`cargo test --release -p axiom-runtime --lib measure_m10_coedit -- --ignored --nocapture`).
/// Medians; not a CI gate.
#[test]
#[ignore]
fn measure_m10_coedit() {
    use super::ghost::{ghost_boxes, GhostBox, Ghosts};
    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }
    let town = axiom_core::ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &crate::installed_registry().authoring,
    )
    .unwrap()
    .world;
    let registry = crate::installed_registry().authoring;
    let mut s = store(&town);
    acl(
        &mut s,
        "set plaza/lamp-1 core.transform/position [6,0,9]; set plaza/tree-4 core.transform/position [7,0,12]; delete plaza/lamp-3",
        false,
    );
    let moved = s.current().unwrap().1;
    let ms = |f: &mut dyn FnMut()| {
        median(
            (0..40)
                .map(|_| {
                    let t = Instant::now();
                    f();
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect(),
        )
    };
    // Background jobs (once per source / revision, never per frame).
    let ghost_job = ms(&mut || {
        std::hint::black_box(ghost_boxes(&town, &moved, &registry).unwrap());
    });
    let merge_view = ms(&mut || {
        std::hint::black_box(axiom_core::ops::merge_view(&town, &moved, &town));
    });
    let merge = ms(&mut || {
        std::hint::black_box(axiom_core::ops::merge(&town, &moved, &town, &[]).unwrap());
    });
    // Scene-view frame with the maximum 64 ghost boxes vs none (whole headless editor frame).
    let mut h = Harness::new(town.clone(), 1);
    let boxes = (0..super::ghost::MAX_GHOSTS)
        .map(|i| {
            let at = glam::Mat4::from_translation(Vec3::new(
                i as f32 % 8.0 * 2.0 - 8.0,
                0.5,
                (i / 8) as f32 * 2.0 - 8.0,
            ));
            GhostBox {
                id: format!("g{i}"),
                kind: GhostKind::Moved,
                before: Some(at),
                after: Some(glam::Mat4::from_translation(Vec3::X) * at),
            }
        })
        .collect();
    let frame = |h: &mut Harness| {
        median(
            (0..300)
                .map(|_| {
                    let t = Instant::now();
                    h.frame(vec![]);
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect(),
        )
    };
    h.frame(vec![]);
    let off = frame(&mut h);
    let key = (GhostSource::Revision(2), 0);
    h.editor.ghost.source = Some(GhostSource::Revision(2));
    h.editor.ghost.requested = Some(key);
    h.editor.ghost.shown = Some((key, Ok(Ghosts { boxes, total: 64 })));
    h.editor.ghost.label = "Ghosts · revision 2 · 64 object(s) changed".into();
    h.frame(vec![]);
    assert_eq!(h.editor.ghost.drawn, 64);
    let on = frame(&mut h);
    let off_again = {
        h.editor.prefs.ghosts = false;
        frame(&mut h)
    };
    println!(
        "{}",
        json!({
            "ghost_job_town_ms": ghost_job,
            "merge_view_town_ms": merge_view,
            "merge_town_ms": merge,
            "editor_frame_ms": {"ghosts_off": off, "ghosts_64": on, "ghosts_off_again": off_again},
        })
    );
}
