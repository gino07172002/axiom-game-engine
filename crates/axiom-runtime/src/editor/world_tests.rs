//! Inspector tag chips and Window ▸ World driven by real egui input (pointer, keys, text).
use super::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::{ops, Edit, Request, Store, World};
use egui::{vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
    store: Store,
}

impl Harness {
    fn new(world: World) -> Self {
        let registry = crate::installed_registry();
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_component_hints(&registry.authoring);
        Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
            store: Store::memory_with(registry.authoring, world).unwrap(),
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
    fn click(&mut self, key: &str) -> Vec<Action> {
        let at = self.rect(key).center();
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut actions = self.frame(vec![Event::PointerMoved(at), press(true)]);
        actions.extend(self.frame(vec![press(false)]));
        actions
    }
    /// Commits the single write in `actions` and shows the new revision, like the host.
    fn commit(&mut self, actions: Vec<Action>) -> Vec<Edit> {
        let [Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }] = actions.as_slice()
        else {
            panic!("expected exactly one write, got {}", actions.len());
        };
        let edits = edits.clone();
        self.store
            .execute(Request::Apply {
                expected_rev: None,
                edits: edits.clone(),
            })
            .unwrap();
        let (rev, world) = self.store.current().unwrap();
        self.editor.update(rev, world, true);
        edits
    }
}

fn json(edits: &[Edit]) -> serde_json::Value {
    serde_json::to_value(edits).unwrap()
}

#[test]
fn tag_chips_add_suggest_and_remove_on_every_selected_object() {
    let start = {
        let mut store = Store::memory_with(crate::installed_registry().authoring, lab()).unwrap();
        // A trigger zone filters on tag `player`: the chips suggest it before anything has it.
        store
            .execute(Request::Acl {
                text: "create trigger box id=gate".into(),
                preview: false,
                expected_rev: None,
                propose: false,
            })
            .unwrap();
        store.current().unwrap().1
    };
    let mut h = Harness::new(start.clone());
    h.editor.select(Some("stage/crate".into()));
    h.editor.extra.insert("stage/rack".into());
    let both = ["stage/crate".to_owned(), "stage/rack".to_owned()];
    h.frame(vec![]);
    // Type a new tag (CJK works) and press Enter: one write for both objects.
    h.click("tag:input");
    h.frame(vec![Event::Text("貨物".into())]);
    assert_eq!(h.editor.tag_ui.input, "貨物");
    let actions = h.frame(vec![Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    let edits = h.commit(actions);
    assert_eq!(
        json(&edits),
        json(&ops::edit_tags(&start, &both, &["貨物".into()], &[]).unwrap())
    );
    assert!(h.editor.tag_ui.input.is_empty());
    // The world-wide suggestion `player` (from the trigger filter) adds with one click.
    h.frame(vec![]);
    let after_first = h.editor.world.clone();
    let clicked = h.click("tag:suggest:player");
    let edits = h.commit(clicked);
    assert_eq!(
        json(&edits),
        json(&ops::edit_tags(&after_first, &both, &["player".into()], &[]).unwrap())
    );
    for id in &both {
        assert_eq!(h.editor.world.entity(id).unwrap().tags, ["貨物", "player"]);
    }
    // x on a chip removes the tag from every selected object.
    h.frame(vec![]);
    let before = h.editor.world.clone();
    let clicked = h.click("tag:remove:貨物");
    let edits = h.commit(clicked);
    assert_eq!(
        json(&edits),
        json(&ops::edit_tags(&before, &both, &[], &["貨物".into()]).unwrap())
    );
    assert_eq!(
        h.editor.world.entity("stage/rack").unwrap().tags,
        ["player"]
    );
    // The chips are rebuilt per revision/selection, not per frame.
    h.frame(vec![]);
    let builds = h.editor.tag_ui.builds;
    for _ in 0..5 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.tag_ui.builds, builds);
    h.editor.extra.clear();
    h.frame(vec![]);
    assert_eq!(h.editor.tag_ui.builds, builds + 1);
}

#[test]
fn world_window_adds_a_capability_and_caches_rows_per_revision() {
    let start = World::empty("blank");
    let mut h = Harness::new(start.clone());
    h.editor.run_command("window.world").unwrap();
    assert!(h.editor.dock_shows(super::Dock::World));
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(
        h.editor.world_panel.builds, 1,
        "rows built once, not per frame"
    );
    // Pick "audio" in the capability combo box (near the top of its scrolled list), then
    // press Add capability.
    h.click("world:capability_combo");
    h.frame(vec![]);
    h.click("world:capability:audio");
    h.frame(vec![]);
    assert_eq!(h.editor.world_panel.capability, "audio");
    let clicked = h.click("world:capability_add");
    let edits = h.commit(clicked);
    let registry = crate::installed_registry().authoring;
    assert_eq!(
        json(&edits),
        json(&ops::add_capability(&start, &registry, "audio").unwrap())
    );
    assert_eq!(h.editor.world.capabilities.get("audio"), Some(&1));
    h.frame(vec![]);
    assert_eq!(
        h.editor.world_panel.builds, 2,
        "a new revision rebuilds the rows once"
    );
    for _ in 0..5 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.world_panel.builds, 2);
    // The command toggles the window.
    h.editor.run_command("window.world").unwrap();
    assert!(!h.editor.dock_shows(super::Dock::World));
}

/// Measures the editor frame with the World window open vs closed on the street world
/// (debug build when run with `cargo test`; prints, asserts only the 16 ms budget on the
/// window's share).
#[test]
fn world_window_cost_on_the_street_world() {
    let world = axiom_core::ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/street"),
        &crate::installed_registry().authoring,
    )
    .unwrap()
    .world;
    let mut h = Harness::new(world);
    h.editor.select(Some("hero".into()));
    let time = |h: &mut Harness, open: bool| {
        if open != h.editor.dock_shows(super::Dock::World) {
            h.editor.run_command("window.world").unwrap();
        }
        for _ in 0..3 {
            h.frame(vec![]);
        }
        let mut samples: Vec<f64> = (0..20)
            .map(|_| {
                let t = std::time::Instant::now();
                h.frame(vec![]);
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        (samples[10], samples[19])
    };
    let closed = time(&mut h, false);
    let open = time(&mut h, true);
    let closed2 = time(&mut h, false);
    let open2 = time(&mut h, true);
    println!(
        "editor frame ms (p50, max) street: closed {closed:?} {closed2:?} / World open {open:?} {open2:?}; row builds {}",
        h.editor.world_panel.builds
    );
    assert_eq!(h.editor.world_panel.builds, 1);
    assert!(open.0.min(open2.0) - closed.0.max(closed2.0) < 16.0);
}
