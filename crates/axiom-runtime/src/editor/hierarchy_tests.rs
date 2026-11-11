//! Hierarchy interaction tests driven by real egui input events (pointer, keys, text).
use super::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::{CoreEdit, Edit, Entity, Request, Store, Transform, World};
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
}

impl Harness {
    fn new(world: World) -> Self {
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        Self {
            ctx: egui::Context::default(),
            editor: Editor::new(world, 1, None),
            runtime,
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0))),
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
    /// Press and release `button` at `at` over two frames; returns the actions of both.
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
    fn key(&mut self, key: Key, modifiers: Modifiers) -> Vec<Action> {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }])
    }
}

/// The single edit batch written by `actions`.
fn edits(actions: &[Action]) -> Vec<Edit> {
    let [Action::Write {
        request: Request::Apply { edits, .. },
        ..
    }] = actions
    else {
        panic!("expected exactly one write, got {}", actions.len());
    };
    edits.clone()
}

fn json(edits: Vec<Edit>) -> serde_json::Value {
    serde_json::to_value(edits).unwrap()
}

fn commit(world: &World, edits: Vec<Edit>) -> World {
    let mut store =
        Store::memory_with(crate::installed_registry().authoring, world.clone()).unwrap();
    store
        .execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
    store.current().unwrap().1
}

#[test]
fn context_menu_rename_commits_cjk_label() {
    let mut h = Harness::new(lab());
    h.frame(vec![]);
    let row = h.rect("label:stage/crate").center();
    // Right-click selects the row and opens the context menu.
    h.click(row, PointerButton::Secondary);
    assert_eq!(h.editor.selected.as_deref(), Some("stage/crate"));
    h.frame(vec![]);
    let rename = h.rect("menu:edit.rename").center();
    assert!(h.click(rename, PointerButton::Primary).is_empty());
    assert!(
        h.editor.renaming.is_some(),
        "Rename starts the inline editor"
    );
    h.frame(vec![]);
    let ctrl = Modifiers::CTRL | Modifiers::COMMAND;
    h.key(Key::A, ctrl);
    h.frame(vec![Event::Text("信號箱 A".into())]);
    assert_eq!(h.editor.renaming.as_ref().unwrap().text, "信號箱 A");
    let actions = h.key(Key::Enter, Modifiers::NONE);
    let edits = edits(&actions);
    assert_eq!(
        json(edits.clone()),
        json(axiom_core::ops::rename(&lab(), "stage/crate", "信號箱 A").unwrap())
    );
    let after = commit(&lab(), edits);
    assert_eq!(
        after.entity("stage/crate").unwrap().label.as_deref(),
        Some("信號箱 A")
    );
    assert!(h.editor.renaming.is_none());
}

#[test]
fn f2_rename_then_escape_cancels_without_writing() {
    let mut h = Harness::new(lab());
    h.editor.select(Some("stage/crate".into()));
    h.frame(vec![]);
    assert!(h.key(Key::F2, Modifiers::NONE).is_empty());
    assert!(h.editor.renaming.is_some(), "F2 starts renaming");
    h.frame(vec![]);
    h.frame(vec![Event::Text("zzz".into())]);
    assert!(h.editor.renaming.as_ref().unwrap().text.ends_with("zzz"));
    let actions = h.key(Key::Escape, Modifiers::NONE);
    assert!(actions.is_empty(), "Esc writes nothing");
    assert!(h.editor.renaming.is_none());
    assert_eq!(
        h.editor.selected.as_deref(),
        Some("stage/crate"),
        "Esc in the text box does not also clear the selection"
    );
    // Enter with unchanged text writes nothing either.
    h.key(Key::F2, Modifiers::NONE);
    h.frame(vec![]);
    assert!(h.key(Key::Enter, Modifiers::NONE).is_empty());
}

/// Drags the row of `from` to `to_y` inside the row of `onto`, releasing there.
fn drag_row(h: &mut Harness, from: &str, onto: &str, fraction: f32) -> Vec<Action> {
    h.frame(vec![]);
    let start = h.rect(&format!("row:{from}"));
    // Grab the label (the drag source), not the eye/lock toggles at the row start.
    let grab = pos2(start.left() + 80.0, start.center().y);
    let target = h.rect(&format!("row:{onto}"));
    let drop = pos2(
        target.left() + 80.0,
        target.top() + target.height() * fraction,
    );
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut actions = h.frame(vec![Event::PointerMoved(grab), button(grab, true)]);
    for step in 1..=6 {
        let p = grab + (drop - grab) * (step as f32 / 6.0);
        actions.extend(h.frame(vec![Event::PointerMoved(p)]));
    }
    actions.extend(h.frame(vec![button(drop, false)]));
    actions.extend(h.frame(vec![]));
    actions
}

#[test]
fn drop_on_row_edges_reorders_and_on_the_middle_reparents() {
    let mut h = Harness::new(lab());
    let actions = drag_row(&mut h, "stage/camera", "stage/floor", 0.1);
    assert_eq!(
        json(edits(&actions)),
        json(vec![Edit::Core(CoreEdit::MoveEntity {
            id: "stage/camera".into(),
            before: Some("stage/floor".into()),
        })]),
        "top edge = move before"
    );
    let mut h = Harness::new(lab());
    let actions = drag_row(&mut h, "stage/floor", "stage/beacon", 0.9);
    assert_eq!(
        json(edits(&actions)),
        json(
            axiom_core::ops::move_entity(
                &lab(),
                "stage/floor",
                &axiom_core::ops::Order::After("stage/beacon".into())
            )
            .unwrap()
        ),
        "bottom edge = move after"
    );
    let mut h = Harness::new(lab());
    let actions = drag_row(&mut h, "stage/crate", "stage/beacon", 0.5);
    assert!(
        matches!(edits(&actions).as_slice(), [Edit::Core(CoreEdit::Reparent { entity, parent: Some(p), .. })] if entity == "stage/crate" && p == "stage/beacon"),
        "middle = reparent (existing behaviour)"
    );
}

#[test]
fn hierarchy_rows_follow_the_authored_order() {
    let mut h = Harness::new(lab());
    h.frame(vec![]);
    let y = |h: &Harness, id: &str| h.rect(&format!("row:{id}")).top();
    assert!(y(&h, "stage/floor") < y(&h, "stage/camera"));
    let moved = commit(
        &lab(),
        axiom_core::ops::move_entity(
            &lab(),
            "stage/camera",
            &axiom_core::ops::Order::Before("stage/floor".into()),
        )
        .unwrap(),
    );
    h.editor.update(2, moved, true);
    h.frame(vec![]);
    assert!(y(&h, "stage/camera") < y(&h, "stage/floor"));
}

#[test]
fn hidden_and_locked_flags_persist_per_world_in_prefs_only() {
    let dir = std::env::temp_dir().join(format!("axiom-view-flags-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lab.editor-prefs.json");
    let mut h = Harness::new(lab());
    h.editor.set_prefs_path(path.clone());
    h.editor.select(Some("stage/rack".into()));
    h.frame(vec![]);
    assert!(
        h.key(Key::H, Modifiers::NONE).is_empty(),
        "H writes nothing"
    );
    assert!(
        h.key(Key::L, Modifiers::NONE).is_empty(),
        "L writes nothing"
    );
    assert_eq!(*h.editor.world, lab());
    // Clicking the eye of another row toggles that row only.
    let eye = h.rect("eye:stage/beacon").center();
    assert!(h.click(eye, PointerButton::Primary).is_empty());
    let mut again = Editor::new(lab(), 1, None);
    again.set_prefs_path(path.clone());
    let hidden = again.view_flagged(false);
    assert_eq!(
        hidden.iter().map(String::as_str).collect::<Vec<_>>(),
        ["stage/beacon", "stage/rack", "stage/rack/cargo"],
        "descendants inherit"
    );
    assert!(again.view_flagged(true).contains("stage/rack/cargo"));
    // Another world in the same prefs file is unaffected.
    let other = Editor::new(World::empty("other"), 1, None);
    assert!(other.view_flagged(false).is_empty());
    // Toggling again shows it.
    h.editor.select(Some("stage/rack".into()));
    h.key(Key::H, Modifiers::NONE);
    assert!(!h.editor.view_flagged(false).contains("stage/rack"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn context_menu_create_child_and_prefab_and_select_children() {
    let mut h = Harness::new(lab());
    h.frame(vec![]);
    let row = h.rect("label:stage/rack").center();
    h.click(row, PointerButton::Secondary);
    h.frame(vec![]);
    let select = h.rect("menu:edit.select_children").center();
    assert!(h.click(select, PointerButton::Primary).is_empty());
    assert_eq!(h.editor.selection(), ["stage/rack", "stage/rack/cargo"]);
    h.frame(vec![]);
    let row = h.rect("label:stage/beacon").center();
    h.click(row, PointerButton::Secondary);
    assert_eq!(
        h.editor.selection(),
        ["stage/beacon"],
        "right-click on an unselected row selects it"
    );
    h.frame(vec![]);
    let prefab = h.rect("menu:object.create_prefab").center();
    let actions = h.click(prefab, PointerButton::Primary);
    assert_eq!(
        json(edits(&actions)),
        json(
            axiom_core::ops::create_prefab(&lab(), None, "stage/beacon")
                .unwrap()
                .edits
        )
    );
}

#[test]
fn hierarchy_with_2000_objects_draws_only_visible_rows() {
    let mut world = World::empty("big");
    world.capabilities.insert("render".into(), 1);
    for i in 0..2000 {
        let mut e = Entity::new(format!("obj/{i:04}"));
        e.label = Some(format!("Object {i}"));
        let transform = Transform {
            position: [(i % 50) as f32, 0.0, (i / 50) as f32],
            ..Default::default()
        };
        e.set("core.transform", transform).unwrap();
        world.entities.push(e);
        if i % 10 != 0 {
            world.relations.push(axiom_core::Relation {
                source: format!("obj/{i:04}"),
                kind: "parent".into(),
                target: format!("obj/{:04}", i - i % 10),
            });
        }
    }
    let mut h = Harness::new(world);
    h.frame(vec![]);
    let times: Vec<f64> = (0..20)
        .map(|_| {
            let t = std::time::Instant::now();
            h.frame(vec![]);
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    let rows = h
        .editor
        .rects
        .keys()
        .filter(|k| k.starts_with("row:"))
        .count();
    assert!(rows > 10 && rows < 80, "{rows} rows laid out of 2000");
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    eprintln!(
        "hierarchy 2000 objects ({} build): {rows} rows laid out; full editor frame median {:.2} ms, max {:.2} ms over 20 frames",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        sorted[10],
        sorted[19]
    );
    // Selecting the last object scrolls it into view and lays out its row.
    h.editor.select(Some("obj/1999".into()));
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.editor.rects.contains_key("row:obj/1999"));
}

/// Workstream E: the first-party Sakura Plaza demo is organized in group folders like the
/// mockup, built with `group` (ops::create_group), and the Hierarchy shows them as a
/// collapsible folder tree.
#[test]
fn sakura_plaza_is_a_folder_tree_of_groups() {
    let registry = crate::installed_registry();
    let world = axiom_core::ContentPackage::load("../../content/plaza", &registry.authoring)
        .unwrap()
        .world;
    let graph = axiom_core::scene_edit::Hierarchy::new(&world);
    let label = |id: &str| world.entity(id).unwrap().label.clone().unwrap_or_default();
    let roots: Vec<String> = graph.children[&None].iter().map(|id| label(id)).collect();
    assert_eq!(roots, ["Environment", "Lighting", "Characters", "Cameras"]);
    let under =
        |id: &str| -> Vec<String> { graph.children[&Some(id)].iter().map(|c| label(c)).collect() };
    assert_eq!(
        under("env"),
        ["PlazaFloor", "Buildings", "Props", "Vegetation"]
    );
    assert!(under("env/props").contains(&"Fountain".to_owned()));
    assert!(under("characters").contains(&"Axiom_Long".to_owned()));
    // Groups are plain objects (no components besides the transform): folder rows.
    let mut h = Harness::new(world);
    h.frame(vec![]);
    h.frame(vec![]);
    for group in ["env", "env/buildings", "env/props", "env/vegetation"] {
        h.rect(&format!("fold:{group}"));
    }
    // A fold arrow collapses the folder: its children leave the tree.
    let fold = h.rect("fold:env").center();
    h.click(fold, PointerButton::Primary);
    assert!(h.editor.collapsed.contains("env"));
    h.editor.rects.clear();
    h.frame(vec![]);
    assert!(h.editor.rects.contains_key("row:env"));
    assert!(!h.editor.rects.contains_key("row:env/props"));
}
