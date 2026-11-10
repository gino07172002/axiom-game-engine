//! Editor follow-ups driven by real egui input: readable diff lines (Proposals / History),
//! the Shader source card at 1366×768, the Tags summary row, prefab override markers with
//! Revert / Apply to Prefab (compared with ACL `prefab revert|apply`) and the Game tab HUD
//! laid out like Play Maximized and scaled into the game area.
use super::{Action, Dock, Editor};
use crate::world::WorldRuntime;
use axiom_core::{gameplay::Region, Request, Store, World};
use egui::{epaint::Shape, pos2, vec2, Event, Modifiers, PointerButton, Pos2, Rect};
use serde_json::json;

fn package(name: &str) -> World {
    axiom_core::ContentPackage::load(
        format!("{}/../../content/{name}", env!("CARGO_MANIFEST_DIR")),
        &crate::installed_registry().authoring,
    )
    .unwrap()
    .world
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

fn press(at: Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn drawn(out: &egui::FullOutput, needle: &str) -> bool {
    out.shapes.iter().any(|c| match &c.shape {
        Shape::Text(t) => t.galley.text().contains(needle),
        _ => false,
    })
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    runtime: WorldRuntime,
    store: Store,
    screen: egui::Vec2,
    time: f64,
}

impl Harness {
    fn new(world: World, screen: egui::Vec2) -> Self {
        let registry = crate::installed_registry();
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_component_hints(&registry.authoring);
        let ctx = egui::Context::default();
        crate::ui::configure(&ctx);
        Self {
            ctx,
            editor,
            runtime,
            store: Store::memory_with(registry.authoring, world).unwrap(),
            screen,
            time: 0.0,
        }
    }
    fn frame_out(&mut self, events: Vec<Event>) -> (Vec<Action>, egui::FullOutput) {
        self.time += 1.0 / 60.0;
        let mut actions = vec![];
        let (editor, runtime) = (&mut self.editor, &self.runtime);
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        (actions, out)
    }
    fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
        self.frame_out(events).0
    }
    #[track_caller]
    fn rect(&self, key: &str) -> Rect {
        *self.editor.rects.get(key).unwrap_or_else(|| {
            let mut keys: Vec<_> = self.editor.rects.keys().collect();
            keys.sort();
            panic!("{key} not drawn; have {keys:?}")
        })
    }
    fn click_with(&mut self, key: &str, button: PointerButton) -> Vec<Action> {
        let at = self.rect(key).center();
        let mut actions = self.frame(vec![Event::PointerMoved(at)]);
        actions.extend(self.frame(vec![press(at, button, true)]));
        actions.extend(self.frame(vec![press(at, button, false)]));
        actions.extend(self.frame(vec![]));
        actions
    }
    fn click(&mut self, key: &str) -> Vec<Action> {
        self.click_with(key, PointerButton::Primary)
    }
    /// Commits the single write in `actions` like the host; returns the new world.
    #[track_caller]
    fn commit(&mut self, actions: Vec<Action>) -> World {
        let actions = super::coalesce(actions);
        let [Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }] = actions.as_slice()
        else {
            panic!(
                "expected exactly one write, got {}; status {:?}",
                actions.len(),
                self.editor.status
            );
        };
        self.store
            .execute(Request::Apply {
                expected_rev: None,
                edits: edits.clone(),
            })
            .unwrap();
        let (rev, world) = self.store.current().unwrap();
        self.editor.update(rev, world.clone(), true);
        self.frame(vec![]);
        self.frame(vec![]);
        world
    }
}

#[test]
fn proposals_and_history_show_readable_paths_and_order_summaries() {
    let mut h = Harness::new(package("editor-lab"), vec2(1366.0, 768.0));
    let diff = json!({"total_changes":3,"truncated":false,"changes":[
        {"kind":"changed","path":"/entities/@stage~1crate/components/core.transform/position/0","before":4,"after":6},
        {"kind":"order","path":"/entities/@order","before":["stage/crate","stage/rack","floor"],"after":["floor","stage/crate"]},
        {"kind":"removed","path":"/entities/@stage~1rack"}
    ]});
    h.editor.set_proposals(
        (1, 1),
        &json!([{"id":1,"base_rev":1,"note":"move the crate","diff":diff.clone()}]),
    );
    h.editor.set_history_diff(2, diff.clone());
    let expected = [
        "~ stage/crate · core.transform/position[0]: 4 → 6",
        "↕ entities order: moved stage/crate (2 items)",
        "- stage/rack",
    ];
    assert_eq!(h.editor.proposal_lines, [expected]);
    assert_eq!(h.editor.history_lines, expected);
    // The panels draw the readable lines; nothing escaped or fingerprinted is shown.
    h.editor.show_dock(Dock::Proposals);
    h.frame(vec![]);
    let out = h.frame_out(vec![]).1;
    for line in expected {
        assert!(drawn(&out, line), "Proposals shows {line:?}");
    }
    assert!(!drawn(&out, "~1"), "escaped paths are not shown");
    assert!(!drawn(&out, "@order"));
    h.editor.show_dock(Dock::History);
    h.frame(vec![]);
    let out = h.frame_out(vec![]).1;
    assert!(drawn(&out, expected[0]), "History shows the readable path");
    // Every symbol the lines use exists in the panels' font (no tofu boxes).
    let font = egui::FontId::monospace(10.0);
    assert!(h.ctx.fonts(|f| f.has_glyphs(&font, "↕→‹›·…")));
}

#[test]
fn shader_card_fits_the_inspector_at_1366x768() {
    let mut h = Harness::new(package("shader-lab"), vec2(1366.0, 768.0));
    h.editor.run_command("window.inspector").unwrap();
    h.editor.open_material("scanner");
    h.frame(vec![]);
    h.frame(vec![]);
    // A long source line and a long error (as a failed check reports them).
    let long_line = format!("// {}\n", "wide ".repeat(60));
    let draft = format!("{long_line}{}", h.editor.tuning.shader.draft);
    h.editor.tuning.shader.draft = draft.clone();
    h.editor.tuning.shader.checked = Some(super::shader_editor::Checked {
        source: draft,
        issues: vec![super::shader_editor::Issue {
            line: Some(3),
            column: Some(18),
            message: format!(
                "line 3, column 18: no definition in scope for identifier: `{}`",
                "smoothstepp_".repeat(12)
            ),
        }],
        elapsed_ms: 0.7,
        state: "finished",
    });
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let panel_right = h.rect("panel:more:inspector").right() + 12.0;
    let card = h.rect("shader:card");
    assert!(
        card.right() <= panel_right,
        "card {card:?} ends past the Inspector ({panel_right})"
    );
    for key in [
        "shader:issue",
        "shader:source_view",
        "shader:errors",
        "shader:check",
    ] {
        let r = h.rect(key);
        assert!(
            r.right() <= card.right() + 0.5 && r.left() >= card.left() - 0.5,
            "{key} {r:?} outside the card {card:?}"
        );
    }
    // The error wraps onto several lines; the long source line scrolls inside the card.
    assert!(h.rect("shader:issue").height() > 20.0);
    assert!(h.rect("shader:source").width() > h.rect("shader:source_view").width());
    // The marked error line sits on the gutter row of line 3 (gutter and source share rows).
    let line = h.rect("shader:error_line");
    let source = h.rect("shader:source");
    let row = line.height();
    assert!((line.top() - (source.top() + 2.0 * row)).abs() < 1.0);
}

#[test]
fn tags_summary_row_opens_the_chip_editor() {
    let start = acl(&package("editor-lab"), "tag stage/crate +cargo +heavy");
    let mut h = Harness::new(start.clone(), vec2(1366.0, 768.0));
    h.editor.select(Some("stage/crate".into()));
    h.frame(vec![]);
    let out = h.frame_out(vec![]).1;
    assert!(drawn(&out, "Tags (2)"));
    assert!(drawn(&out, "cargo, heavy"));
    assert!(!h.editor.rects.contains_key("tag:input"), "collapsed");
    h.click("tag:summary");
    assert!(h.editor.tag_ui.expanded);
    let remove = h.click("tag:remove:heavy");
    let after = h.commit(remove);
    assert_eq!(after, acl(&start, "tag stage/crate -heavy"));
    let out = h.frame_out(vec![]).1;
    assert!(drawn(&out, "Tags (1)"));
    // Collapses again; `object.tag` without a tag opens it.
    h.click("tag:summary");
    assert!(!h.editor.tag_ui.expanded);
    h.editor.run_command("object.tag").unwrap();
    assert!(h.editor.tag_ui.expanded);
}

#[test]
fn prefab_override_markers_revert_and_apply_like_acl() {
    // lamp-1 overrides its position (authored) and nav.blocker/half (set here).
    let start = acl(
        &package("axiom-town"),
        "set plaza/lamp-1 nav.blocker/half [0.3,0.3]",
    );
    let mut h = Harness::new(start.clone(), vec2(1700.0, 1100.0));
    h.editor.run_command("window.inspector").unwrap();
    h.editor.select(Some("plaza/lamp-1".into()));
    h.frame(vec![]);
    h.frame(vec![]);
    // Transform: position is an override, rotation / scale are inherited.
    assert!(h.editor.rects.contains_key("override:/position"));
    assert!(!h.editor.rects.contains_key("override:/rotation"));
    assert!(!h.editor.rects.contains_key("override:/scale"));
    // The sets are built per revision and selection, not per frame.
    let builds = h.editor.overrides.builds;
    for _ in 0..5 {
        h.frame(vec![]);
    }
    assert_eq!(h.editor.overrides.builds, builds);
    // Open nav.blocker: `half` is marked; render.mesh (inherited) has no marker.
    h.click("component:render.mesh");
    assert!(
        !h.editor
            .rects
            .keys()
            .any(|k| k.starts_with("override:/parts")),
        "inherited fields look normal"
    );
    h.click("component:nav.blocker");
    assert!(h.editor.rects.contains_key("override:/half"));
    // Right-click ▸ Apply to Prefab = ACL prefab apply.
    h.click_with("override:/half", PointerButton::Secondary);
    let actions = h.click("override:apply");
    let after = h.commit(actions);
    let expected = acl(&start, "prefab apply plaza/lamp-1 nav.blocker/half");
    assert_eq!(after, expected);
    assert_eq!(
        after.prefabs["lamp"].components["nav.blocker"]["half"],
        json!([0.3, 0.3])
    );
    assert!(
        !h.editor.rects.contains_key("override:/half"),
        "no longer overridden"
    );
    // Transform position ▸ Revert to Prefab = ACL prefab revert.
    h.click_with("override:/position", PointerButton::Secondary);
    let actions = h.click("override:revert");
    let after = h.commit(actions);
    assert_eq!(
        after,
        acl(
            &expected,
            "prefab revert plaza/lamp-1 core.transform/position"
        )
    );
    assert!(!after
        .entity("plaza/lamp-1")
        .unwrap()
        .components
        .contains_key("core.transform"));
    assert!(!h.editor.rects.contains_key("override:/position"));
    // Commands: the same ops (bridge path); errors are reported, not ignored.
    h.editor.select(Some("plaza/lamp-2".into()));
    h.frame(vec![]);
    let actions = h
        .editor
        .run_command_with("prefab.revert", Some("core.transform/position"))
        .unwrap();
    let after = h.commit(actions);
    assert_eq!(
        after,
        acl(
            &acl(
                &expected,
                "prefab revert plaza/lamp-1 core.transform/position"
            ),
            "prefab revert plaza/lamp-2 core.transform/position"
        )
    );
    assert!(h
        .editor
        .run_command_with("prefab.apply", Some("core.transform/position"))
        .is_err());
    h.editor.extra.insert("plaza/lamp-3".into());
    assert_eq!(
        h.editor.command_blocked("prefab.apply"),
        Some("Apply to Prefab takes one selected object")
    );
}

#[test]
fn prefab_commands_are_blocked_during_play_and_without_selection() {
    let mut h = Harness::new(package("axiom-town"), vec2(1366.0, 768.0));
    assert_eq!(
        h.editor.command_blocked("prefab.revert"),
        Some("nothing selected")
    );
    h.editor.select(Some("plaza/lamp-1".into()));
    h.frame(vec![]);
    assert_eq!(h.editor.command_blocked("prefab.revert"), None);
    h.editor.previewing = true;
    assert!(h
        .editor
        .command_blocked("prefab.apply")
        .is_some_and(|r| r.contains("stop Play")));
}

/// Area rectangles of the HUD regions drawn in `ctx`, by region.
fn hud_areas(ctx: &egui::Context) -> Vec<(Region, Rect)> {
    [
        Region::TopLeft,
        Region::Top,
        Region::TopRight,
        Region::Left,
        Region::Center,
        Region::Right,
        Region::BottomLeft,
        Region::Bottom,
        Region::BottomRight,
    ]
    .into_iter()
    .filter_map(|region| {
        ctx.memory(|m| m.area_rect(egui::Id::new(("bound_region", region))))
            .map(|r| (region, r))
    })
    .collect()
}

#[test]
fn game_tab_hud_keeps_the_maximized_layout_scaled_into_the_game_area() {
    let runtime = WorldRuntime::new(package("axiom-town"), 1, crate::installed_registry()).unwrap();
    // Play Maximized: the HUD in the whole window (1280×720 points).
    let window = vec2(1280.0, 720.0);
    let maximized = egui::Context::default();
    let mut ui = crate::ui::UiState::default();
    for _ in 0..3 {
        let _ = maximized.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, window)),
                ..Default::default()
            },
            |ctx| {
                runtime.draw_ui(ctx, &mut ui);
            },
        );
    }
    let reference = hud_areas(&maximized);
    assert!(!reference.is_empty(), "Axiom Town draws HUD regions");
    // Game tab: a 640×360 game area at (300, 120) of the same window.
    let area = Rect::from_min_size(pos2(300.0, 120.0), vec2(640.0, 360.0));
    let scale = super::game::hud_scale(area.size(), window);
    assert_eq!(scale, 0.5);
    let game = egui::Context::default();
    let mut ui = crate::ui::UiState::default();
    let mut out = None;
    for _ in 0..3 {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, window)),
            ..Default::default()
        };
        let mut input = crate::app::game_raw_input(raw, area, false, false, false);
        super::game::scale_game_input(&mut input, scale, 1.0);
        out = Some(game.run(input, |ctx| {
            runtime.draw_ui(ctx, &mut ui);
        }));
    }
    assert_eq!(out.unwrap().pixels_per_point, 0.5, "drawn at half size");
    let origin = super::game::hud_rect(area, scale).min;
    let tab = hud_areas(&game);
    assert_eq!(tab.len(), reference.len());
    for ((region, max), (r2, small)) in reference.iter().zip(&tab) {
        assert_eq!(region, r2);
        // Same layout in the game's points (relative to the game area) up to text rows
        // snapped to whole physical pixels at half the density (a few points per panel) ...
        let moved = small.translate(-origin.to_vec2());
        // (an anchored edge moves by at most the size difference).
        let near = |a: f32, b: f32, size: f32| (a - b).abs() <= 2.0 + 0.05 * size;
        assert!(
            near(moved.min.x, max.min.x, max.width())
                && near(moved.min.y, max.min.y, max.height())
                && near(moved.width(), max.width(), max.width())
                && near(moved.height(), max.height(), max.height()),
            "{region:?}: tab {moved:?} vs maximized {max:?}"
        );
        // ... which on screen is the maximized rectangle at half size inside the area.
        let on_screen = Rect::from_min_max(
            (small.min.to_vec2() * scale).to_pos2(),
            (small.max.to_vec2() * scale).to_pos2(),
        );
        assert!(
            area.expand(1.0).contains_rect(on_screen),
            "{region:?} {on_screen:?}"
        );
    }
    // Pointer positions follow: editor (400, 200) is game (800, 400).
    let mut input = egui::RawInput {
        screen_rect: Some(area),
        events: vec![
            Event::PointerMoved(pos2(400.0, 200.0)),
            press(pos2(400.0, 200.0), PointerButton::Primary, true),
        ],
        ..Default::default()
    };
    super::game::scale_game_input(&mut input, scale, 1.5);
    assert_eq!(input.screen_rect, Some(super::game::hud_rect(area, 0.5)));
    assert!(matches!(input.events[0], Event::PointerMoved(p) if p == pos2(800.0, 400.0)));
    assert!(
        matches!(input.events[1], Event::PointerButton { pos, .. } if pos == pos2(800.0, 400.0))
    );
    assert_eq!(
        input.viewports[&egui::ViewportId::ROOT].native_pixels_per_point,
        Some(0.75)
    );
    // Bounds: a full-window game area is 1×; a tiny one stops at the minimum.
    assert_eq!(super::game::hud_scale(window, window), 1.0);
    assert_eq!(
        super::game::hud_scale(vec2(50.0, 30.0), window),
        super::game::HUD_SCALE_MIN
    );
    assert_eq!(
        super::game::hud_scale(vec2(640.0, 360.0), vec2(0.0, 0.0)),
        1.0
    );
}
