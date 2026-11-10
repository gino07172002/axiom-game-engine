use axiom_core::{gameplay::RuntimeCommand, ContentPackage, World};
use axiom_runtime::{
    geometry::Geometry,
    installed_registry,
    ui::{Action, UiState},
    vfx::{self, Emitter},
    ActionMap, WorldRuntime,
};
use glam::Vec3;
use serde_json::{json, Value};
fn world() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/effects-tour"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}
fn runtime() -> WorldRuntime {
    WorldRuntime::new(world(), 1, installed_registry()).unwrap()
}
fn call(r: &mut WorldRuntime, capability: &str, action: &str, args: Value) -> Value {
    r.command(&RuntimeCommand::Call {
        capability: capability.into(),
        action: action.into(),
        args,
    })
    .unwrap()
}
fn steps(r: &mut WorldRuntime, count: u32) {
    for _ in 0..count {
        r.step(&ActionMap::default());
        r.assert_valid().unwrap();
    }
}
#[test]
fn pinned_official_inventory_maps_24_materials_and_12_particles_without_templates() {
    let inventory: Value =
        serde_json::from_str(include_str!("../../../content/effects-tour/inventory.json")).unwrap();
    let items = inventory["stations"].as_array().unwrap();
    assert_eq!(items.len(), 36);
    assert_eq!(inventory["commit"].as_str().unwrap().len(), 40);
    let w = world();
    let catalog = &w.resources["showcase.tour"]["stations"];
    for (category, count) in [("materials", 24), ("particles", 12)] {
        let cases = items
            .iter()
            .filter(|v| v["category"] == category)
            .collect::<Vec<_>>();
        assert_eq!(cases.len(), count);
        assert_eq!(
            cases
                .iter()
                .map(|v| v["number"].as_u64().unwrap())
                .collect::<std::collections::BTreeSet<_>>(),
            (1..=count as u64).collect()
        );
        for item in cases {
            let id = item["id"].as_str().unwrap();
            assert_eq!(catalog[id]["source_title"], item["source_title"]);
            assert!(!item["description"].as_str().unwrap().is_empty());
            assert!(!item["axiom_entities"].as_array().unwrap().is_empty());
            assert_ne!(item["source_title"], "1. Title");
        }
    }
    assert!(catalog["mat/15-tessellation"]["implementation"]
        .as_str()
        .unwrap()
        .contains("CPU"));
    assert!(catalog["mat/22-subsurface"]["description"]
        .as_str()
        .unwrap()
        .contains("不是"));
}
#[test]
fn all_stations_focus_with_valid_frames_and_shared_reusable_render_components() {
    let mut r = runtime();
    let ids = r.state.compiled.source.resources["showcase.tour"]["stations"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for id in ids {
        call(&mut r, "showcase", "focus", json!({"station":id}));
        steps(&mut r, 3);
        assert_eq!(r.observation().metrics["showcase"]["active"], id);
        let frame = r.render_frame();
        assert_eq!(frame.views.len(), 1);
        assert!(frame.meshes.len() > 40);
        assert_eq!(frame.lights.len(), 16);
        assert_eq!(frame.vfx.ribbons.len(), 3);
        assert_eq!(
            frame
                .vfx
                .batches
                .iter()
                .filter(|b| b.emitter.backend == "gpu")
                .count(),
            2
        );
        if id == "mat/21-decals" {
            assert!(frame.postfx.iter().any(|v| v.kind == "decal"));
        }
        if id == "fx/02-gpu" {
            assert!(
                frame.postfx.is_empty(),
                "GPU reference view must not inherit neighboring heat distortion"
            );
        }
    }
    call(&mut r, "showcase", "overview", json!({}));
    assert!(r.observation().metrics["showcase"]["active"].is_null());
    call(&mut r, "showcase", "next", json!({"direction":1}));
    assert_eq!(r.observation().metrics["showcase"]["active"], "mat/01-pbr");
    call(&mut r, "showcase", "next", json!({"direction":i64::MAX}));
    assert_eq!(
        r.observation().metrics["showcase"]["active"],
        "mat/02-color"
    );
}
#[test]
fn procedural_geometry_has_real_triangles_valid_normals_and_bitmap_textures() {
    for shape in ["sphere", "plane", "torus"] {
        let g = Geometry {
            shape: shape.into(),
            segments: 12,
            rings: 8,
            texture: Some("checker".into()),
            ..Default::default()
        };
        let asset = g.build().unwrap();
        asset.validate().unwrap();
        assert!(asset.indices.len() > 100);
        assert_eq!(asset.textures[0].rgba.len(), 64 * 64 * 4);
        assert_eq!(asset.materials[0].texture, Some(0));
        for t in asset.indices.as_chunks::<3>().0 {
            let (a, b, c) = (
                &asset.vertices[t[0] as usize],
                &asset.vertices[t[1] as usize],
                &asset.vertices[t[2] as usize],
            );
            assert!(
                (b.position - a.position)
                    .cross(c.position - a.position)
                    .dot(a.normal)
                    > 0.0
            );
        }
    }
    let wire = Geometry {
        barycentric_uv: true,
        flat: true,
        ..Default::default()
    }
    .build()
    .unwrap();
    for tri in wire.vertices.as_chunks::<3>().0 {
        assert_eq!(
            tri.iter()
                .map(|v| (v.uv.x as i32, v.uv.y as i32))
                .collect::<std::collections::BTreeSet<_>>(),
            [(0, 0), (1, 0), (0, 1)].into_iter().collect()
        );
        assert!(tri[0].normal.abs_diff_eq(tri[1].normal, 1e-5));
    }
    assert!(Geometry {
        segments: 4096,
        ..Default::default()
    }
    .build()
    .is_err());
}
#[test]
fn seeded_cpu_particles_and_plane_collisions_are_reproducible() {
    let e = Emitter::default();
    let a = vfx::evaluate(&e, 17, 2.0, Vec3::ZERO);
    let b = vfx::evaluate(&e, 17, 2.0, Vec3::ZERO);
    assert_eq!(a.position_size, b.position_size);
    assert_eq!(a.color, b.color);
    assert_ne!(
        a.position_size,
        vfx::evaluate(
            &Emitter {
                seed: 99,
                ..e.clone()
            },
            17,
            2.0,
            Vec3::ZERO
        )
        .position_size
    );
    let e = Emitter {
        seed: 0,
        origin: [0.0; 3],
        spread: 0.0,
        lifetime: 30.0,
        velocity: [0.0; 3],
        acceleration: [0.0, -9.8, 0.0],
        floor: Some(0.0),
        bounce: 0.8,
        ..Default::default()
    };
    let before = vfx::evaluate(&e, 0, 0.2, Vec3::Y * 0.5);
    let after = vfx::evaluate(&e, 0, 0.4, Vec3::Y * 0.5);
    assert!(before.position_size[1] > 0.0);
    assert!(after.position_size[1] > 0.0);
    assert_eq!(
        vfx::evaluate(
            &Emitter {
                bounce: 0.0,
                ..e.clone()
            },
            0,
            0.4,
            Vec3::Y * 0.5
        )
        .position_size[1],
        0.0
    );
    for i in 0..1000 {
        assert!(vfx::evaluate(&e, 0, i as f32 * 0.01, Vec3::Y * 0.5).position_size[1] >= 0.0);
    }
    assert!(Emitter {
        backend: "gpu".into(),
        floor: Some(0.0),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Emitter {
        backend: "gpu".into(),
        renderer: "ribbon".into(),
        count: 1,
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Emitter {
        bone: Some("fore.R".into()),
        ..Default::default()
    }
    .validate()
    .is_err());
}
#[test]
fn size_curves_and_gradients_replace_the_two_point_values_and_are_bounded() {
    let two = Emitter::default();
    // Absent curves: exactly the old two-point formulas.
    for i in 0..64 {
        let p = vfx::evaluate(&two, i, 1.3, Vec3::ZERO);
        let t = p.misc[2];
        assert_eq!(p.position_size[3], two.size + (two.end_size - two.size) * t);
        let old = glam::Vec4::from_array(two.color)
            .lerp(glam::Vec4::from_array(two.end_color), t)
            .to_array();
        assert_eq!(p.color, old);
    }
    let e = Emitter {
        size_curve: Some(vec![
            vfx::SizeKey { t: 0.2, size: 0.1 },
            vfx::SizeKey { t: 0.5, size: 0.5 },
            vfx::SizeKey { t: 0.9, size: 0.0 },
        ]),
        color_gradient: Some(vec![
            vfx::ColorStop {
                t: 0.0,
                color: [1.0, 0.0, 0.0, 1.0],
            },
            vfx::ColorStop {
                t: 0.5,
                color: [0.0, 1.0, 0.0, 1.0],
            },
            vfx::ColorStop {
                t: 1.0,
                color: [0.0, 0.0, 1.0, 0.0],
            },
        ]),
        ..Default::default()
    };
    e.validate().unwrap();
    // Held before the first key / after the last, linear between.
    assert_eq!(vfx::size_at(&e, 0.0), 0.1);
    assert_eq!(vfx::size_at(&e, 0.95), 0.0);
    assert!((vfx::size_at(&e, 0.35) - 0.3).abs() < 1e-6);
    assert_eq!(vfx::color_at(&e, 0.5), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(vfx::color_at(&e, 0.75), [0.0, 0.5, 0.5, 0.5]);
    for i in 0..64 {
        let p = vfx::evaluate(&e, i, 0.7, Vec3::ZERO);
        assert_eq!(p.position_size[3], vfx::size_at(&e, p.misc[2]));
        assert_eq!(p.color, vfx::color_at(&e, p.misc[2]));
    }
    // Backward compatible JSON: old emitters parse, new fields round-trip, unknown keys fail.
    let old: Emitter = serde_json::from_value(json!({"size":0.2})).unwrap();
    assert!(old.size_curve.is_none() && old.color_gradient.is_none());
    assert!(serde_json::to_value(&old)
        .unwrap()
        .get("size_curve")
        .is_none());
    let back: Emitter = serde_json::from_value(serde_json::to_value(&e).unwrap()).unwrap();
    assert_eq!(back, e);
    assert!(serde_json::from_value::<Emitter>(
        json!({"size_curve":[{"t":0,"size":1,"ease":"in"}]})
    )
    .is_err());
    let keys = |n: usize| {
        Some(
            (0..n)
                .map(|i| vfx::SizeKey {
                    t: i as f32 / n as f32,
                    size: 0.1,
                })
                .collect(),
        )
    };
    for bad in [
        keys(1),
        keys(vfx::MAX_CURVE_KEYS + 1),
        Some(vec![
            vfx::SizeKey { t: 0.5, size: 0.1 },
            vfx::SizeKey { t: 0.4, size: 0.1 },
        ]),
        Some(vec![
            vfx::SizeKey { t: 0.0, size: 0.1 },
            vfx::SizeKey { t: 1.0, size: 9.0 },
        ]),
    ] {
        assert!(Emitter {
            size_curve: bad,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
    assert!(Emitter {
        size_curve: keys(vfx::MAX_CURVE_KEYS),
        ..Default::default()
    }
    .validate()
    .is_ok());
    assert!(Emitter {
        color_gradient: Some(vec![
            vfx::ColorStop {
                t: 0.0,
                color: [1.0; 4]
            },
            vfx::ColorStop {
                t: 1.0,
                color: [1.5, 0.0, 0.0, 0.0]
            },
        ]),
        ..Default::default()
    }
    .validate()
    .is_err());
}
#[test]
fn bitmaps_really_control_spawn_and_invalid_masks_fail_before_activation() {
    let e = Emitter {
        field: "texture".into(),
        texture_mask: Some(vec!["101".into(), "010".into()]),
        velocity: [0.0; 3],
        acceleration: [0.0; 3],
        spread: 0.0,
        ..Default::default()
    };
    e.validate().unwrap();
    let points = (0..3)
        .map(|i| vfx::evaluate(&e, i, 0.0, Vec3::ZERO).position_size)
        .collect::<Vec<_>>();
    assert_eq!(points[0][0], -1.5);
    assert_eq!(points[1][0], 1.5);
    assert_eq!(points[2][0], 0.0);
    assert!(Emitter {
        texture_mask: Some(vec!["000".into()]),
        ..e.clone()
    }
    .validate()
    .is_err());
    assert!(Emitter {
        texture_mask: Some(vec!["11".into(), "1".into()]),
        ..e
    }
    .validate()
    .is_err());
}
#[test]
fn bone_trails_restart_pause_and_hot_reload_preserve_unmodified_effects() {
    let mut r = runtime();
    call(
        &mut r,
        "showcase",
        "focus",
        json!({"station":"fx/06-attached"}),
    );
    steps(&mut r, 130);
    let frame = r.render_frame();
    assert!(frame.vfx.ribbons.iter().all(|v| v.points.len() == 96));
    assert!(
        frame
            .vfx
            .ribbons
            .iter()
            .filter(|v| v
                .points
                .first()
                .unwrap()
                .distance(*v.points.last().unwrap())
                > 0.1)
            .count()
            >= 2
    );
    let before = r.observation().metrics["vfx"].clone();
    let mut updated = r.state.compiled.source.clone();
    updated.resources.get_mut("render.shaders").unwrap()["materials"]["rough-0"]["parameters"]
        ["roughness"] = json!(0.7);
    r.reconcile(updated, 2).unwrap();
    assert_eq!(r.observation().metrics["vfx"], before);
    assert_eq!(
        r.observation().metrics["showcase"]["active"],
        "fx/06-attached"
    );
    call(
        &mut r,
        "vfx",
        "pause",
        json!({"id":"fx/06-attached","paused":true}),
    );
    let time = r.observation().metrics["vfx"]["emitters"]["fx/06-attached"]["time"].clone();
    steps(&mut r, 20);
    assert_eq!(
        r.observation().metrics["vfx"]["emitters"]["fx/06-attached"]["time"],
        time
    );
    call(&mut r, "vfx", "restart", json!({"id":"fx/06-attached"}));
    assert_eq!(
        r.observation().metrics["vfx"]["emitters"]["fx/06-attached"]["trail_points"],
        0
    );
    let inspected = call(&mut r, "vfx", "inspect", json!({"id":"fx/02-gpu"}));
    assert!(inspected["result"]["gpu_note"]
        .as_str()
        .unwrap()
        .contains("reference"));
    assert_eq!(
        inspected["result"]["cpu_reference_sample"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert!(r.checkpoint().is_err());
}
#[test]
fn invalid_references_and_budgets_retain_the_last_usable_world() {
    let mut r = runtime();
    steps(&mut r, 2);
    let original = r.state.compiled.source.clone();
    let mut invalid = original.clone();
    invalid.resources.get_mut("vfx.library").unwrap()["emitters"]["fx/06-attached"]["bone"] =
        json!("missing-bone");
    assert!(r.reconcile(invalid, 2).is_err());
    assert_eq!(r.rev, 1);
    let mut invalid = original.clone();
    invalid.resources.get_mut("vfx.library").unwrap()["emitters"]["fx/02-gpu"]["count"] =
        json!(999999);
    assert!(r.reconcile(invalid, 3).is_err());
    let mut invalid = original.clone();
    invalid.resources.get_mut("postfx.settings").unwrap()["volumes"][0]["half_size"] =
        json!([0, 1, 1]);
    assert!(r.reconcile(invalid, 4).is_err());
    let mut invalid = original;
    invalid.resources.get_mut("showcase.tour").unwrap()["initial"] = json!("missing");
    assert!(r.reconcile(invalid, 5).is_err());
    steps(&mut r, 2);
}
#[test]
fn ui_navigation_layout_and_slider_edits_use_the_registered_protocol() {
    let mut r = runtime();
    let ctx = egui::Context::default();
    let mut ui = UiState {
        playing: true,
        ..Default::default()
    };
    let mut run = |r: &WorldRuntime, events: Vec<egui::Event>| {
        let mut actions = vec![];
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 720.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                actions = r.draw_ui(ctx, &mut ui);
            },
        );
        actions
    };
    run(&r, vec![]);
    let initial = run(&r, vec![]);
    assert!(initial.iter().any(
        |a| matches!(a,Action::Gameplay(RuntimeCommand::Call{action,..}) if action=="viewport")
    ));
    call(
        &mut r,
        "showcase",
        "focus",
        json!({"station":"mat/03-metalness"}),
    );
    run(&r, vec![]);
    drop(run);
    let target = ui.targets["showcase/control/mat/03-metalness/0"];
    let position = target.left_center() + egui::vec2(40.0, 0.0);
    let mut actions = vec![];
    for pressed in [true, false] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 720.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                actions.extend(r.draw_ui(ctx, &mut ui));
            },
        );
    }
    let request = actions
        .iter()
        .find_map(|a| {
            if let Action::Apply(s) = a {
                Some(serde_json::from_str::<Value>(s).unwrap())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(request["expected_rev"], 1);
    assert_eq!(request["edits"][0]["target"]["id"], "render.shaders");
    assert_eq!(
        request["edits"][0]["field"],
        "/materials/metal/parameters/metalness"
    );
}
#[test]
fn vertex_hook_signature_and_blend_modes_are_validated_as_material_contracts() {
    let valid="fn vertex_offset(p:vec3<f32>,n:vec3<f32>,uv:vec2<f32>,t:f32,v:vec4<f32>)->vec3<f32>{return p+n*sin(t)*v.x;} fn surface(s:SurfaceInput)->vec4<f32>{return s.base_color;}";
    axiom_runtime::shader::validate_hook("valid", valid).unwrap();
    assert!(axiom_runtime::shader::validate_hook(
        "bad",
        &valid.replace("return p+n*sin(t)*v.x;", "return 0.0;")
    )
    .is_err());
    let w = world();
    let prepared = axiom_runtime::shader::Prepared::from_world(&w).unwrap();
    assert_eq!(
        prepared.materials["blend-additive"].alpha,
        axiom_runtime::shader::Alpha::Additive
    );
    assert_eq!(
        prepared.materials["blend-multiply"].alpha,
        axiom_runtime::shader::Alpha::Multiply
    );
    for source in [
        include_str!("../src/vfx_compute.wgsl"),
        axiom_runtime::lighting::VFX_DRAW_WGSL,
        axiom_runtime::lighting::SCENE_WGSL,
        axiom_runtime::lighting::MESH_WGSL,
        include_str!("../src/postfx.wgsl"),
    ] {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }
}

fn camera(r: &WorldRuntime) -> (Vec3, Value) {
    let slot = r.state.compiled.indices["tour/camera"];
    (
        Vec3::from_array(r.state.transforms[slot].as_ref().unwrap().position),
        json!(
            r.state.compiled.entities[slot]
                .camera
                .as_ref()
                .unwrap()
                .projection
        ),
    )
}
#[test]
fn free_camera_orbits_zooms_and_pans_within_limits() {
    let mut r = runtime();
    call(&mut r, "showcase", "focus", json!({"station":"mat/01-pbr"}));
    let target = Vec3::new(-25.0, 1.25, 0.0);
    let (eye, _) = camera(&r);
    assert!((eye.distance(target) - 7.8).abs() < 1e-3);
    // Orbit a quarter turn: the eye moves from +Z to +X of the pedestal.
    call(
        &mut r,
        "showcase",
        "view",
        json!({"orbit":std::f32::consts::FRAC_PI_2}),
    );
    let (eye, _) = camera(&r);
    assert!(
        eye.x > target.x + 7.0 && (eye.z - target.z).abs() < 0.1,
        "{eye:?}"
    );
    // Zoom is multiplicative and clamped.
    call(&mut r, "showcase", "view", json!({"zoom":0.5}));
    assert!((camera(&r).0.distance(target) - 3.9).abs() < 1e-3);
    call(&mut r, "showcase", "view", json!({"zoom":0.01}));
    assert!((camera(&r).0.distance(target) - 2.5).abs() < 1e-3);
    // Pan moves the look-at point sideways but stays near the exhibit.
    call(
        &mut r,
        "showcase",
        "view",
        json!({"pan":[5.0, 0.0],"aspect":1.6}),
    );
    let after = camera(&r).0;
    assert!(after.distance(target) > 2.6 && after.distance(target) < 2.5 + 6.0 * 3.0);
    call(&mut r, "showcase", "reset_view", json!({}));
    assert!((camera(&r).0.distance(target) - 7.8).abs() < 1e-3);
    // Overview: pan and zoom the orthographic map; reset returns home.
    call(&mut r, "showcase", "overview", json!({}));
    let (home, projection) = camera(&r);
    assert_eq!(projection["height"], json!(44.0));
    call(
        &mut r,
        "showcase",
        "view",
        json!({"zoom":0.5,"pan":[0.2, 0.0],"aspect":1.6}),
    );
    let (moved, projection) = camera(&r);
    assert_eq!(projection["height"], json!(22.0));
    assert!(moved.x < home.x - 4.0, "{moved:?}");
    for bad in [
        json!({"zoom":0.0}),
        json!({"orbit":"x"}),
        json!({"pan":[1.0]}),
    ] {
        assert!(r
            .command(&RuntimeCommand::Call {
                capability: "showcase".into(),
                action: "view".into(),
                args: bad,
            })
            .is_err());
    }
    call(&mut r, "showcase", "reset_view", json!({}));
    assert!(camera(&r).0.distance(home) < 1e-3);
}
#[test]
fn mouse_drags_wheel_and_map_clicks_drive_the_camera_through_commands() {
    let mut r = runtime();
    let ctx = egui::Context::default();
    let mut ui = UiState {
        playing: true,
        ..Default::default()
    };
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 720.0));
    let mut frame = |r: &mut WorldRuntime, events: Vec<egui::Event>| {
        let mut actions = vec![];
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ctx| actions = r.draw_ui(ctx, &mut ui),
        );
        let mut names = vec![];
        for a in actions {
            if let Action::Gameplay(command) = a {
                if let RuntimeCommand::Call { action, .. } = &command {
                    names.push(action.clone());
                }
                r.command(&command).unwrap();
            }
        }
        names
    };
    frame(&mut r, vec![]);
    frame(&mut r, vec![]);
    let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    // Overview: drag pans the map; nothing is entered by a drag release.
    let start = egui::pos2(700.0, 300.0);
    let (before, _) = camera(&r);
    frame(
        &mut r,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    for k in 1..=6 {
        frame(
            &mut r,
            vec![egui::Event::PointerMoved(
                start + egui::vec2(k as f32 * 20.0, 0.0),
            )],
        );
    }
    let end = start + egui::vec2(120.0, 0.0);
    frame(&mut r, vec![button(end, false)]);
    let (after, _) = camera(&r);
    assert!(
        after.x < before.x - 1.0,
        "drag right moves the map: {before:?} -> {after:?}"
    );
    assert!(r.observation().metrics["showcase"]["active"].is_null());
    // Clicking a pedestal label enters that exhibit.
    call(&mut r, "showcase", "reset_view", json!({}));
    frame(&mut r, vec![]);
    let slot = r.state.compiled.indices["tour/camera"];
    let view = axiom_runtime::presentation::RenderView {
        id: "tour/camera".into(),
        eye: camera(&r).0,
        forward: glam::Quat::from_array(r.state.transforms[slot].as_ref().unwrap().rotation)
            * -Vec3::Z,
        up: glam::Quat::from_array(r.state.transforms[slot].as_ref().unwrap().rotation) * Vec3::Y,
        camera: r.state.compiled.entities[slot].camera.clone().unwrap(),
        instances: vec![],
    };
    let label = view
        .project(Vec3::new(-25.0, 0.25, 3.0), [0.0, 0.0, 1100.0, 720.0])
        .unwrap();
    let label = egui::pos2(label[0], label[1]);
    frame(&mut r, vec![egui::Event::PointerMoved(label)]);
    frame(&mut r, vec![button(label, true)]);
    frame(&mut r, vec![button(label, false)]);
    assert_eq!(r.observation().metrics["showcase"]["active"], "mat/01-pbr");
    // Station view: wheel zooms in, a left drag orbits.
    let center = egui::pos2(700.0, 330.0);
    let target = Vec3::new(-25.0, 1.25, 0.0);
    frame(&mut r, vec![egui::Event::PointerMoved(center)]);
    let names = frame(
        &mut r,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 120.0),
            modifiers: egui::Modifiers::NONE,
        }],
    );
    for _ in 0..30 {
        frame(&mut r, vec![]);
    }
    assert!(names.contains(&"view".to_string()) || camera(&r).0.distance(target) < 7.7);
    assert!(camera(&r).0.distance(target) < 7.7, "wheel should zoom in");
    let (before, _) = camera(&r);
    frame(&mut r, vec![button(center, true)]);
    for k in 1..=5 {
        frame(
            &mut r,
            vec![egui::Event::PointerMoved(
                center + egui::vec2(k as f32 * 25.0, 0.0),
            )],
        );
    }
    frame(&mut r, vec![button(center + egui::vec2(125.0, 0.0), false)]);
    let (after, _) = camera(&r);
    assert!((after.distance(target) - before.distance(target)).abs() < 1e-3);
    assert!(
        after.distance(before) > 0.5,
        "drag should orbit: {before:?} -> {after:?}"
    );
}
