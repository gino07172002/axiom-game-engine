//! Assets browser driven by real egui input: browse / search / virtualization, dragging an
//! asset onto the Scene and onto a typed reference field, Find Usages, rename with reference
//! fix-up, the New asset dialogs and the thumbnail job (progress, cancel, disk cache, texture
//! budget). Every write is compared with the ACL line an agent would send.
use super::{Action, Editor};
use crate::world::WorldRuntime;
use axiom_core::assets::{AssetKind, Owner};
use axiom_core::{Edit, Request, Store, World};
use egui::{vec2, Event, Modifiers, PointerButton, Pos2, Rect};
use serde_json::json;
use std::time::{Duration, Instant};

/// Axiom Town as a pack import leaves it (model paths absolute).
fn town() -> World {
    let registry = crate::installed_registry();
    axiom_core::ContentPackage::load("../../content/axiom-town", &registry.authoring)
        .unwrap()
        .world
}

fn lab() -> World {
    serde_json::from_str(include_str!("../../../../content/editor-lab/world.json")).unwrap()
}

fn acl(world: &World, script: &str) -> World {
    let mut store =
        Store::memory_with(crate::installed_registry().authoring, world.clone()).unwrap();
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

fn acl_read(world: &World, script: &str) -> String {
    let mut store =
        Store::memory_with(crate::installed_registry().authoring, world.clone()).unwrap();
    let out = store
        .execute(Request::Acl {
            text: script.into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .unwrap_or_else(|e| panic!("acl {script:?}: {e:#}"));
    out["text"].as_str().unwrap().to_owned()
}

fn press(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
    /// The editor's world as a store (None for fixtures the store would refuse, e.g. 600
    /// prefabs; those tests never commit).
    store: Option<Store>,
    start: World,
}

impl Harness {
    /// `world` in the editor; `runtime_world` (default: the same) drives the scene view.
    fn new(world: World, runtime_world: Option<World>) -> Self {
        let registry = crate::installed_registry();
        let runtime = WorldRuntime::new(
            runtime_world.unwrap_or_else(|| world.clone()),
            1,
            crate::installed_registry(),
        )
        .unwrap();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_component_hints(&registry.authoring);
        editor.run_command("window.assets").unwrap();
        let mut h = Self {
            ctx: egui::Context::default(),
            editor,
            runtime,
            store: Store::memory_with(registry.authoring, world.clone()).ok(),
            start: world,
        };
        // Windows and panels lay out on their first frame.
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }
    fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        editor.poll_jobs();
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 1000.0))),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        actions
    }
    fn rect(&self, key: &str) -> Rect {
        *self.editor.rects.get(key).unwrap_or_else(|| {
            let mut keys: Vec<&String> = self
                .editor
                .rects
                .keys()
                .filter(|k| k.starts_with("asset"))
                .collect();
            keys.sort();
            panic!("{key} not drawn; have {keys:?}")
        })
    }
    fn click(&mut self, key: &str) -> Vec<Action> {
        let at = self.rect(key).center();
        let mut actions = self.frame(vec![Event::PointerMoved(at), press(at, true)]);
        actions.extend(self.frame(vec![press(at, false)]));
        // What the click opened (details, dialogs) lays out on the next frame.
        actions.extend(self.frame(vec![]));
        actions
    }
    /// Presses on `from`, moves to `to` over `steps` frames and releases there.
    fn drag_to(&mut self, from: Pos2, to: Pos2, steps: usize) -> Vec<Action> {
        let mut actions = self.frame(vec![Event::PointerMoved(from), press(from, true)]);
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            actions.extend(self.frame(vec![Event::PointerMoved(from + (to - from) * t)]));
        }
        actions.extend(self.frame(vec![press(to, false)]));
        actions
    }
    fn type_text(&mut self, key: &str, text: &str) {
        self.click(key);
        let select_all = Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        };
        self.frame(vec![select_all, Event::Text(text.into())]);
        self.frame(vec![]);
    }
    /// Commits the single write in `actions` like the host and shows the new revision.
    fn commit(&mut self, actions: Vec<Action>) -> Vec<Edit> {
        let writes: Vec<&Action> = actions
            .iter()
            .filter(|a| matches!(a, Action::Write { .. }))
            .collect();
        let [Action::Write {
            request: Request::Apply { edits, .. },
            select,
        }] = writes.as_slice()
        else {
            panic!("expected exactly one write, got {}", writes.len());
        };
        let (edits, select) = (edits.clone(), select.clone());
        let store = self
            .store
            .as_mut()
            .expect("a committing test needs a valid world");
        store
            .execute(Request::Apply {
                expected_rev: None,
                edits: edits.clone(),
            })
            .unwrap();
        let (rev, world) = store.current().unwrap();
        self.editor.update(rev, world, true);
        if let Some(id) = select {
            self.editor.select(Some(id));
        }
        edits
    }
    fn world(&mut self) -> World {
        self.store.as_mut().unwrap().current().unwrap().1
    }
    fn assert_same(&mut self, script: &str) {
        assert_eq!(
            self.world(),
            acl(&self.start, script),
            "editor and `{script}` diverge"
        );
    }
    fn asset_cells(&self) -> usize {
        self.editor
            .rects
            .keys()
            .filter(|k| k.starts_with("asset:"))
            .count()
    }
}

#[test]
fn browse_filters_by_kind_folder_and_search_and_lists_the_pack() {
    let mut h = Harness::new(town(), None);
    for cell in [
        "asset:model:axiom-short",
        "asset:model:axiom-long",
        "asset:prefab:bench",
        "asset:machine:lumi",
        "asset:ui:ui.document",
        "asset:input:input.bindings",
    ] {
        h.rect(cell);
    }
    // Type tree: only prefabs.
    h.click("assets:kind:prefab");
    h.editor.rects.clear();
    h.frame(vec![]);
    assert!(h.editor.rects.contains_key("asset:prefab:lamp"));
    assert!(!h.editor.rects.contains_key("asset:model:axiom-short"));
    // Search (all kinds again): "long" leaves the one model.
    h.click("assets:kind:model");
    h.editor.assets.filter = super::asset_browser::Filter::All;
    h.type_text("assets:search", "long");
    h.editor.rects.clear();
    h.frame(vec![]);
    let cells: Vec<&String> = h
        .editor
        .rects
        .keys()
        .filter(|k| k.starts_with("asset:"))
        .collect();
    assert_eq!(cells, ["asset:model:axiom-long"], "{cells:?}");
    // List mode shows the same entries.
    h.editor.assets.list = true;
    h.editor.rects.clear();
    h.frame(vec![]);
    assert!(h.editor.rects.contains_key("asset:model:axiom-long"));
    // The list is built once per world version, the filter once per query.
    let (builds, filters) = (h.editor.assets.builds, h.editor.assets.filters);
    for _ in 0..20 {
        h.frame(vec![]);
    }
    assert_eq!(
        (h.editor.assets.builds, h.editor.assets.filters),
        (builds, filters)
    );
}

#[test]
fn a_large_pack_lays_out_only_the_visible_cells() {
    let mut world = lab();
    for i in 0..600 {
        world.prefabs.insert(
            format!("bulk/{i:03}"),
            axiom_core::Prefab {
                components: [("core.transform".to_owned(), json!({}))].into(),
            },
        );
    }
    let mut h = Harness::new(world, Some(lab()));
    let drawn = h.asset_cells();
    assert!(drawn > 0 && drawn < 120, "{drawn} cells laid out of 600+");
    h.editor.assets.list = true;
    h.editor.rects.clear();
    h.frame(vec![]);
    let drawn = h.asset_cells();
    assert!(
        drawn > 0 && drawn < 80,
        "{drawn} list rows laid out of 600+"
    );
}

#[test]
fn dragging_a_model_onto_the_scene_creates_one_object_like_acl() {
    let mut h = Harness::new(town(), None);
    let from = h.rect("asset:model:axiom-short").center();
    let to = h.editor.viewport.center();
    let actions = h.drag_to(from, to, 6);
    h.commit(actions);
    let world = h.world();
    let entity = world.entity("axiom-short").expect("object created");
    assert_eq!(
        entity.components["animation.player"],
        json!({"asset":"axiom-short"})
    );
    let p = &entity.components["core.transform"]["position"];
    assert_eq!(p[1], 0.0, "dropped on the ground: {p}");
    let at = format!("{},{},{}", p[0], p[1], p[2]);
    h.assert_same(&format!("asset place model:axiom-short at={at}"));
    // The new object is selected and the status line shows the ACL.
    assert_eq!(h.editor.selected.as_deref(), Some("axiom-short"));
    assert!(
        h.editor
            .status
            .contains("asset place model:axiom-short at="),
        "{}",
        h.editor.status
    );
    // Images do not create objects: refused as a problem, nothing written.
    let actions = h
        .editor
        .run_command_with("asset.place", Some("ui:ui.document"));
    assert!(actions.is_err());
}

#[test]
fn dropping_an_asset_on_a_reference_field_sets_it_like_acl() {
    let mut h = Harness::new(town(), None);
    h.editor
        .run_command_with("layout.load", Some("Tall Inspector"))
        .unwrap();
    h.editor.run_command("window.assets").unwrap();
    h.frame(vec![]);
    h.editor.select(Some("hero".into()));
    h.frame(vec![]);
    h.click("component:animation.player");
    h.frame(vec![]);
    let field = h.rect("form:/asset").center();
    // A model cell onto the animation.player asset field: one write on release.
    let from = h.rect("asset:model:axiom-long").center();
    let mut actions = h.drag_to(from, field, 6);
    actions.extend(h.frame(vec![]));
    h.commit(actions);
    h.assert_same("set hero animation.player/asset \"axiom-long\"");
    // A prefab is not a model: dropping it there changes nothing.
    h.frame(vec![]);
    let from = h.rect("asset:prefab:bench").center();
    let field = h.rect("form:/asset").center();
    let mut actions = h.drag_to(from, field, 6);
    actions.extend(h.frame(vec![]));
    assert!(
        !actions.iter().any(|a| matches!(a, Action::Write { .. })),
        "a prefab dropped on a model field wrote something"
    );
}

#[test]
fn find_usages_lists_every_reference_and_jumps_to_it() {
    let world = town();
    let mut h = Harness::new(world.clone(), None);
    h.click("asset:model:axiom-short");
    h.click("assets:usages");
    h.frame(vec![]);
    let view = h.editor.assets.usages.as_ref().expect("usages listed");
    let owners: Vec<&Owner> = view.rows.iter().map(|u| &u.owner).collect();
    assert_eq!(
        owners,
        [
            &Owner::Entity("hero".into()),
            &Owner::Entity("npc/rin".into()),
            &Owner::Entity("pc/kiri".into())
        ]
    );
    // Same list as the ACL read an agent sends.
    let text = acl_read(&world, "usages model:axiom-short");
    assert!(text.starts_with("usages model:axiom-short=3"), "{text}");
    for id in ["hero", "npc/rin", "pc/kiri"] {
        assert!(
            text.contains(&format!("entity {id} /components/animation.player/asset")),
            "{text}"
        );
    }
    // Clicking a row selects that object.
    h.click("usage:1");
    assert_eq!(h.editor.selected.as_deref(), Some("npc/rin"));
    // The bridge command does the same.
    h.editor
        .run_command_with("asset.find_usages", Some("machine:stroller"))
        .unwrap();
    assert_eq!(h.editor.assets.usages.as_ref().unwrap().rows.len(), 2);
    assert!(h
        .editor
        .run_command_with("asset.find_usages", Some("machine:nope"))
        .is_err());
}

#[test]
fn rename_fixes_references_in_one_revision_like_acl() {
    let mut h = Harness::new(town(), None);
    h.click("asset:machine:stroller");
    h.click("assets:rename");
    h.type_text("assets:rename_text", "wanderer");
    let actions = h.click("assets:rename_apply");
    let edits = h.commit(actions);
    assert!(edits.len() >= 3, "map + 2 agents in one batch: {edits:?}");
    h.assert_same("asset rename machine:stroller wanderer");
    let world = h.world();
    assert!(axiom_core::assets::usages(&world, AssetKind::Machine, "stroller").is_empty());
    assert_eq!(
        axiom_core::assets::usages(&world, AssetKind::Machine, "wanderer").len(),
        2
    );
    assert_eq!(
        h.editor.assets.selected,
        Some((AssetKind::Machine, "wanderer".into()))
    );
    // A taken name is a problem, not a write.
    let actions = h
        .editor
        .run_command_with("asset.rename", Some("machine:wanderer lumi"));
    assert!(actions.is_err());
    assert_eq!(h.editor.problem_codes()[0], "id_taken");
}

#[test]
fn new_asset_dialogs_show_and_run_their_acl() {
    for (command, fields, expected) in [
        (
            "asset.new_material",
            vec![
                ("Material ID", "glass"),
                ("Shader", "unlit"),
                ("Tint (r,g,b,a)", "0.2,0.4,0.9"),
            ],
            "asset create material glass shader=unlit tint=0.2,0.4,0.9,1",
        ),
        (
            "asset.new_prefab",
            vec![("Prefab ID", "crate2"), ("From object (optional)", "")],
            "asset create prefab crate2",
        ),
        (
            "asset.new_machine",
            vec![("Machine ID", "guard"), ("Initial state", "patrol")],
            "asset create machine guard initial=patrol",
        ),
    ] {
        let mut h = Harness::new(town(), None);
        let mut actions = vec![];
        h.editor.invoke(command, &mut actions);
        let dialog = h.editor.options.as_mut().expect("dialog open");
        for (label, value) in &fields {
            dialog.set(label, value);
        }
        h.frame(vec![]);
        h.frame(vec![]);
        let shown = h.editor.options.as_ref().unwrap().acl(&h.editor);
        assert_eq!(shown, expected);
        let actions = h.click("options:create");
        h.commit(actions);
        h.assert_same(expected);
    }
    // UI document and input map: the empty lab world has neither.
    for (command, expected) in [
        ("asset.new_ui_document", "asset create ui locale=zh-TW"),
        ("asset.new_input_map", "asset create input"),
    ] {
        let mut h = Harness::new(lab(), None);
        let mut actions = vec![];
        h.editor.invoke(command, &mut actions);
        if command == "asset.new_ui_document" {
            h.editor.options.as_mut().unwrap().set("Locale", "zh-TW");
        }
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.editor.options.as_ref().unwrap().acl(&h.editor), expected);
        let actions = h.click("options:create");
        h.commit(actions);
        h.assert_same(expected);
        // Created once: the command is refused afterwards.
        assert!(h.editor.command_blocked(command).is_some());
    }
}

/// `count` small PNGs in `dir` and a world whose ui.document lists them as images.
fn image_world(dir: &std::path::Path, count: usize) -> World {
    std::fs::create_dir_all(dir).unwrap();
    let mut images = serde_json::Map::new();
    for i in 0..count {
        let path = dir.join(format!("img-{i:03}.png"));
        let pixels: Vec<u8> = (0..16 * 8)
            .flat_map(|p| [(p * 2 + i) as u8, (i * 3) as u8, 200, 255])
            .collect();
        let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), 16, 8);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        images.insert(format!("img-{i:03}"), json!(path.to_string_lossy()));
    }
    let mut world = lab();
    world.resources.insert(
        "ui.document".into(),
        json!({"locale":"en","catalogs":{"en":{}},"widgets":[],"images":images}),
    );
    world
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("axiom-m5-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wait_for(h: &mut Harness, what: &str, mut done: impl FnMut(&Harness) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done(h) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        h.frame(vec![]);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn thumbnail_job_reports_progress_and_cancels() {
    let dir = scratch("cancel");
    let world = image_world(&dir.join("src"), 40);
    let mut h = Harness::new(world, Some(lab()));
    h.editor.assets.thumbs.dir = Some(dir.join("thumbs"));
    h.editor.assets.thumbs.delay = Duration::from_millis(30);
    // An explicit job replaces the lazy one the visible cells started.
    h.click("assets:thumbnails");
    assert!(h.editor.assets.thumbs.explicit_running());
    wait_for(&mut h, "progress", |h| {
        h.editor
            .assets
            .thumbs
            .running()
            .is_some_and(|j| j.progress().0 >= 2)
    });
    // Visible to agents: observe lists the running job; the status bar shows it.
    let jobs = h.editor.observation()["jobs"].clone();
    assert_eq!(jobs[0]["kind"], "thumbnails", "{jobs}");
    assert_eq!(jobs[0]["total"], 40);
    assert!(h
        .editor
        .run_command("asset.thumbnails")
        .is_err_and(|e| e.to_string().contains("being generated")));
    h.click("assets:cancel");
    // Checked as soon as it ends: the visible cells then start their own lazy job.
    wait_for(&mut h, "the cancelled job to end", |h| {
        !h.editor.assets.thumbs.explicit_running()
    });
    let last = h.editor.assets.thumbs.last.clone().unwrap();
    assert_eq!(last["state"], "cancelled", "{last}");
    assert!(last["done"].as_u64().unwrap() < 40, "{last}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thumbnails_are_cached_by_content_hash_and_textures_stay_in_budget() {
    let dir = scratch("budget");
    let count = 100;
    let world = image_world(&dir.join("src"), count);
    let mut h = Harness::new(world, Some(lab()));
    let cache = dir.join("thumbs");
    h.editor.assets.thumbs.dir = Some(cache.clone());
    h.editor.run_command("asset.thumbnails").unwrap();
    wait_for(&mut h, "the explicit job", |h| {
        h.editor.assets.thumbs.running().is_none()
    });
    let last = h.editor.assets.thumbs.last.clone().unwrap();
    assert_eq!(last["state"], "finished", "{last}");
    assert_eq!(last["made"], count as u64, "{last}");
    let files = std::fs::read_dir(&cache).unwrap().count();
    assert_eq!(files, count, "one cache file per distinct image");
    // Show every image through the search (10 at a time): uploads stay within the per-frame
    // budget and the GPU textures within the texture budget (least recently drawn evicted).
    let budgeted_frame = |h: &mut Harness| {
        let before = h.editor.assets.thumbs.uploaded;
        h.frame(vec![]);
        let uploads = h.editor.assets.thumbs.uploaded - before;
        assert!(
            uploads <= super::thumbs::UPLOADS_PER_FRAME as u64,
            "{uploads} uploads in one frame"
        );
        assert!(h.editor.assets.thumbs.textures() <= super::thumbs::TEXTURE_BUDGET);
    };
    for group in 0..10 {
        h.editor.assets.query = format!("img-0{group}");
        for _ in 0..3 {
            budgeted_frame(&mut h);
        }
        wait_for(&mut h, "the group's thumbnails", |h| {
            h.editor.assets.thumbs.running().is_none()
        });
        for _ in 0..8 {
            budgeted_frame(&mut h);
        }
    }
    assert!(
        h.editor.assets.thumbs.uploaded >= count as u64,
        "{} uploads",
        h.editor.assets.thumbs.uploaded
    );
    assert!(h.editor.assets.thumbs.evicted > 0, "nothing was evicted");
    assert!(h.editor.assets.thumbs.textures() <= super::thumbs::TEXTURE_BUDGET);
    // A second editor (new session) gets them from the cache: nothing is made again.
    let mut again = Harness::new(image_world(&dir.join("src"), count), Some(lab()));
    again.editor.assets.thumbs.dir = Some(cache.clone());
    again.editor.run_command("asset.thumbnails").unwrap();
    wait_for(&mut again, "the cached job", |h| {
        h.editor.assets.thumbs.running().is_none()
    });
    let last = again.editor.assets.thumbs.last.clone().unwrap();
    assert_eq!(
        (last["cached"].as_u64(), last["made"].as_u64()),
        (Some(count as u64), Some(0)),
        "{last}"
    );
    let observed = again.editor.observation()["thumbnails"].clone();
    assert_eq!(observed["texture_budget"], super::thumbs::TEXTURE_BUDGET);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn model_thumbnails_without_a_gpu_fail_visibly_instead_of_hanging() {
    let mut h = Harness::new(town(), None);
    wait_for(&mut h, "the lazy job", |h| {
        h.editor.assets.thumbs.last.is_some() && h.editor.assets.thumbs.running().is_none()
    });
    let observed = h.editor.observation()["thumbnails"].clone();
    assert_eq!(observed["gpu"], false);
    assert!(observed["failed"].as_u64().unwrap() >= 2, "{observed}");
}
