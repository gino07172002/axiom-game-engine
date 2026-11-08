use axiom_core::{gameplay::RuntimeCommand, *};
use axiom_runtime::{
    world::{PointerKind, WorldRuntime},
    ActionMap,
};
use serde_json::{json, Value};
fn runtime() -> WorldRuntime {
    WorldRuntime::new(
        axiom_rts::content::demo_world(64).unwrap(),
        1,
        axiom_runtime::installed_registry(),
    )
    .unwrap()
}
fn call(r: &mut WorldRuntime, action: &str, args: Value) -> Value {
    r.command(&RuntimeCommand::Call {
        capability: "rts".into(),
        action: action.into(),
        args,
    })
    .unwrap()
}
fn stats(r: &WorldRuntime) -> Value {
    json!(r.observation())["rts"]["summary"].clone()
}
#[test]
fn native_capability_dispatch_is_typed_and_scoped() {
    let mut r = runtime();
    assert_eq!(stats(&r)["units_total"], 64);
    let first = call(
        &mut r,
        "inspect",
        json!({"ids":["unit/0/0"],"fields":["id","position"]}),
    );
    assert_eq!(first["result"]["units"][0]["id"], "unit/0/0");
    for command in [
        RuntimeCommand::Call {
            capability: "rts".into(),
            action: "train".into(),
            args: json!({"building":"base/0","recipe":"worker","count":99}),
        },
        RuntimeCommand::Call {
            capability: "rts".into(),
            action: "unknown".into(),
            args: json!({}),
        },
    ] {
        assert!(r.command(&command).is_err());
    }
    assert_eq!(stats(&r)["balance"], json!([300, 300]));
    let mut fps = WorldRuntime::new(
        axiom_fps::demo_world().unwrap(),
        1,
        axiom_runtime::installed_registry(),
    )
    .unwrap();
    assert!(fps
        .command(&RuntimeCommand::Call {
            capability: "rts".into(),
            action: "inspect".into(),
            args: json!({})
        })
        .is_err());
}
#[test]
fn normalized_pointer_rays_box_select_and_right_click_use_real_camera() {
    let mut r = runtime();
    let frame = r.render_frame();
    let view = &frame.views[0];
    let height = match view.camera.projection {
        Projection::Orthographic { height } => height,
        _ => unreachable!(),
    };
    let aspect = 1.6;
    let matrix = glam::Mat4::orthographic_rh(
        -height * aspect * 0.5,
        height * aspect * 0.5,
        -height * 0.5,
        height * 0.5,
        view.camera.near,
        view.camera.far,
    ) * glam::Mat4::look_at_rh(view.eye, view.eye + view.forward, view.up);
    let screen = |p: [f32; 2]| {
        let p = matrix.project_point3(glam::Vec3::new(p[0], 0.0, p[1]));
        [(p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5]
    };
    r.screen_pointer(
        PointerKind::PrimaryDown,
        screen([-25.0, -9.0]),
        aspect,
        false,
    )
    .unwrap();
    r.screen_pointer(PointerKind::Move, screen([-14.0, -2.0]), aspect, false)
        .unwrap();
    r.screen_pointer(PointerKind::PrimaryUp, screen([-14.0, -2.0]), aspect, false)
        .unwrap();
    assert_eq!(stats(&r)["selected"], 32);
    r.screen_pointer(
        PointerKind::SecondaryDown,
        screen([9.5, 0.5]),
        aspect,
        false,
    )
    .unwrap();
    assert_eq!(stats(&r)["stats"]["orders"], 1);
    for _ in 0..60 {
        r.step(&ActionMap::default());
    }
    r.assert_valid().unwrap();
}
#[test]
fn production_creates_real_generation_ids_and_survives_data_reload() {
    let mut r = runtime();
    let initial = r.state.compiled.len();
    call(
        &mut r,
        "train",
        json!({"building":"base/0","recipe":"worker","count":1}),
    );
    for _ in 0..120 {
        r.step(&ActionMap::default());
    }
    r.assert_valid().unwrap();
    assert_eq!(r.state.compiled.len(), initial + 1);
    let slot = r.state.compiled.indices["produced/0/1"];
    let handle = r.state.ids[slot];
    let mut w = r.state.compiled.source.clone();
    w.title = "Hot reload RTS".into();
    r.reconcile(w, 2).unwrap();
    assert_eq!(stats(&r)["stats"]["produced"], 1);
    let new_slot = r.state.compiled.indices["produced/0/1"];
    assert_eq!(r.state.ids[new_slot], handle);
    assert!(r.state.resolve(handle).is_some());
    assert_eq!(r.state.health[new_slot].as_ref().unwrap().value, 60.0);
    assert_eq!(stats(&r)["balance"], json!([260, 300]));
}
#[test]
fn grid_edit_failure_retains_last_good_running_rts() {
    let mut r = runtime();
    let mut w = r.state.compiled.source.clone();
    w.resources.get_mut("rts.session").unwrap()["grid"]["blocked"]
        .as_array_mut()
        .unwrap()
        .push(json!([0, 0]));
    assert!(r.reconcile(w, 2).is_err());
    assert_eq!(r.rev, 1);
    assert_eq!(r.requested_rev, 2);
    assert_eq!(r.observation().activation, "activation_failed");
    r.step(&ActionMap::default());
    r.assert_valid().unwrap();
}
#[test]
fn attack_and_victory_follow_data_and_normal_group_command() {
    let mut w = axiom_rts::content::demo_world(64).unwrap();
    w.entity_mut("unit/0/20")
        .unwrap()
        .set(
            "core.transform",
            Transform {
                position: [26.5, 0.0, 0.5],
                ..Default::default()
            },
        )
        .unwrap();
    w.entity_mut("unit/0/20")
        .unwrap()
        .components
        .get_mut("rts.unit")
        .unwrap()["damage"] = json!(200);
    let mut r = WorldRuntime::new(w, 1, axiom_runtime::installed_registry()).unwrap();
    call(
        &mut r,
        "attack",
        json!({"units":["unit/0/20"],"target":"base/1"}),
    );
    for _ in 0..600 {
        r.step(&ActionMap::default());
    }
    r.assert_valid().unwrap();
    assert_eq!(stats(&r)["victory"], true);
    let slot = r.state.compiled.indices["base/1"];
    assert_eq!(r.state.health[slot].as_ref().unwrap().value, 0.0);
    assert!(!r.state.compiled.enabled[slot]);
}
#[test]
fn stress_reports_real_samples_with_all_units_active() {
    let report =
        axiom_runtime::rts_stress::run(128, 240, axiom_runtime::installed_registry()).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["initial_units"], 128);
    assert_eq!(report["state"]["units_alive"], 128);
    assert!(report["step_ms"]["p95"].as_f64().unwrap() > 0.0);
    assert_eq!(report["overlaps"], 0);
    if cfg!(windows) {
        assert!(report["memory"]["working_set_bytes"].as_u64().unwrap() > 0);
    }
}
#[test]
fn runtime_spawns_preserve_custom_columns_and_reject_duplicate_ids() {
    let mut world = axiom_rts::content::demo_world(64).unwrap();
    world.component_schemas.insert(
        "test.flag".into(),
        serde_json::from_value(json!({"fields":{"enabled":{"ty":"bool"}}})).unwrap(),
    );
    world
        .entity_mut("unit/0/0")
        .unwrap()
        .set("test.flag", json!({"enabled":false}))
        .unwrap();
    let mut r = WorldRuntime::new(world, 1, axiom_runtime::installed_registry()).unwrap();
    let mut template = r.state.compiled.entities[r.state.compiled.indices["unit/0/0"]].clone();
    template.id = "template".into();
    template.components.get_mut("test.flag").unwrap()["enabled"] = json!(true);
    let handle = r
        .state
        .spawn(&template, "test-spawn".into(), [-40.5, 0.0, -20.5])
        .unwrap();
    assert!(r.state.resolve(handle).is_some());
    assert!(
        matches!(&r.state.columns["test.flag"].fields["enabled"],Column::Bool(v) if v.last()==Some(&Some(true)))
    );
    assert!(r
        .state
        .spawn(&template, "test-spawn".into(), [-39.5, 0.0, -20.5])
        .is_err());
}

#[test]
fn rts_hud_is_data_bound_and_buttons_use_shared_capability_commands() {
    let mut r = runtime();
    assert!(
        r.state
            .gameplay
            .as_ref()
            .unwrap()
            .compiled
            .ui
            .as_ref()
            .unwrap()
            .flow
    );
    let ctx = egui::Context::default();
    axiom_runtime::ui::configure(&ctx);
    let mut ui = axiom_runtime::ui::UiState {
        playing: true,
        ..Default::default()
    };
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(960.0, 600.0),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        let _ = ctx.run(raw(), |ctx| {
            r.draw_ui(ctx, &mut ui);
        });
    }
    let pos = ui.targets["button/workers"].center();
    for (id, rect) in &ui.targets {
        assert!(
            rect.max.x <= 960.0,
            "HUD target {id} must stay inside the view: {rect:?}"
        );
    }
    assert!(
        ctx.available_rect().height() > 350.0,
        "RTS HUD must leave most of the game view visible"
    );
    let mut actions = vec![];
    for pressed in [true, false] {
        let mut input = raw();
        input.events = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let _ = ctx.run(input, |ctx| {
            actions = r.draw_ui(ctx, &mut ui);
        });
    }
    let command = actions
        .into_iter()
        .find_map(|a| {
            if let axiom_runtime::ui::Action::Gameplay(c) = a {
                Some(c)
            } else {
                None
            }
        })
        .unwrap();
    r.command(&command).unwrap();
    assert_eq!(stats(&r)["selected"], 16);
    assert_eq!(
        r.state.gameplay.as_ref().unwrap().observation()["bindings"]["value/selected"],
        "16"
    );
}

#[test]
fn full_64_unit_functional_scenario_passes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scenario = serde_json::from_str(
        &std::fs::read_to_string(root.join("examples/rts-functional.scenario.json")).unwrap(),
    )
    .unwrap();
    let result = axiom_runtime::scenario::run(
        axiom_rts::content::demo_world(64).unwrap(),
        1,
        axiom_runtime::installed_registry(),
        scenario,
    )
    .unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["observation"]["rts"]["summary"]["units_total"], 66);
}
