//! Authored lights end to end: pack load, compile, render-frame extraction and GPU packing.
use axiom_core::{ContentPackage, LightKind, World};
use axiom_runtime::{installed_registry, lighting, ActionMap, WorldRuntime};
use glam::Vec3;

fn pack(name: &str) -> World {
    ContentPackage::load(
        format!("{}/../../content/{name}", env!("CARGO_MANIFEST_DIR")),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

#[test]
fn light_lab_extracts_every_kind_in_authored_order() {
    let mut runtime = WorldRuntime::new(pack("light-lab"), 1, installed_registry()).unwrap();
    for _ in 0..4 {
        runtime.step(&ActionMap::default());
    }
    let frame = runtime.render_frame();
    let ids: Vec<_> = frame.scene_lights.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "moon",
            "lamp/west/light",
            "lamp/east/light",
            "lamp/north/light",
            "statue/spot",
            "sign/light",
            "kiosk/light"
        ]
    );
    // Children of the lamp posts follow their parents (world position).
    let west = &frame.scene_lights[1];
    assert!((west.position - Vec3::new(-5.0, 2.7, 1.0)).length() < 1e-5);
    // Spot points down; the sign panel faces +Z.
    assert!((frame.scene_lights[4].direction + Vec3::Y).length() < 1e-5);
    assert!((frame.scene_lights[5].direction - Vec3::Z).length() < 1e-5);
    // The prefab override recolours only the north lamp.
    assert_eq!(frame.scene_lights[3].light.color, [0.75, 0.85, 1.0]);
    assert_eq!(frame.scene_lights[1].light.color, [1.0, 0.72, 0.42]);

    let packed = lighting::pack(&frame);
    assert!(packed.authored);
    assert_eq!(packed.sun_count, 1);
    assert!(packed.shadow);
    assert_eq!(packed.local_count, 6);
    let kinds: Vec<f32> = packed.locals[..6]
        .iter()
        .map(|l| l.direction_kind[3])
        .collect();
    assert_eq!(
        kinds,
        [
            lighting::KIND_POINT,
            lighting::KIND_POINT,
            lighting::KIND_POINT,
            lighting::KIND_SPOT,
            lighting::KIND_RECT,
            lighting::KIND_DISK
        ]
    );
    // The moon shines along its -Z; the shader receives the direction toward it.
    let toward = Vec3::from_slice(&packed.suns[0].direction_shadow[..3]);
    assert!((toward - Vec3::new(0.4, 0.75, -0.55).normalize()).length() < 1e-4);
    // Fog density 0.025 squared.
    assert!((packed.fog[3] - 0.000625).abs() < 1e-9);
}

#[test]
fn packs_without_lights_keep_the_legacy_frame() {
    for name in ["axiom-town", "fps", "shader-lab", "effects-tour"] {
        let runtime = WorldRuntime::new(pack(name), 1, installed_registry()).unwrap();
        let frame = runtime.render_frame();
        assert!(frame.scene_lights.is_empty(), "{name}");
        let packed = lighting::pack(&frame);
        assert!(!packed.authored, "{name}");
        assert_eq!(
            packed.light_vp,
            lighting::legacy_light_vp(frame.settings.shadow_extent),
            "{name}"
        );
    }
}

#[test]
fn disabled_and_scene_inactive_lights_are_not_drawn() {
    let mut world = pack("light-lab");
    // Disabling the only directional light falls back to the default sun.
    world
        .entities
        .iter_mut()
        .find(|e| e.id == "moon")
        .unwrap()
        .components
        .get_mut("render.light")
        .unwrap()["enabled"] = false.into();
    world.scenes.push(axiom_core::Scene {
        id: "off".into(),
        entities: vec!["statue/spot".into()],
        active: false,
    });
    let runtime = WorldRuntime::new(world, 1, installed_registry()).unwrap();
    let frame = runtime.render_frame();
    assert!(frame
        .scene_lights
        .iter()
        .all(|l| l.id != "moon" && l.id != "statue/spot"));
    assert!(frame
        .scene_lights
        .iter()
        .all(|l| l.light.kind != LightKind::Directional));
    let packed = lighting::pack(&frame);
    assert_eq!(packed.local_count, 5);
    assert_eq!(packed.suns[0].color[..3], axiom_core::DEFAULT_SUN_COLOR);
}

#[test]
fn shadow_lab_spot_and_point_lights_cast() {
    let mut runtime = WorldRuntime::new(pack("shadow-lab"), 1, installed_registry()).unwrap();
    for _ in 0..4 {
        runtime.step(&ActionMap::default());
    }
    let frame = runtime.render_frame();
    let packed = lighting::pack(&frame);
    let id = |i: Option<usize>| i.map(|i| frame.scene_lights[i].id.as_str());
    assert_eq!(packed.local_shadows.spot.map(id), [Some("spot/key"), None]);
    assert_eq!(packed.local_shadows.point.map(id), [Some("point/lamp")]);
    assert_eq!(packed.local_shadows.face_count(), 7);
    assert_eq!(packed.local_shadows.ignored, 0);
    // The skinned fighter is a mesh caster in the frame.
    assert!(frame.meshes.iter().any(|m| m.id == "fighter"));
    // Turning shadows off removes every shadow pass.
    let mut world = pack("shadow-lab");
    for e in &mut world.entities {
        if let Some(light) = e.components.get_mut("render.light") {
            light["shadows"] = false.into();
        }
    }
    let runtime = WorldRuntime::new(world, 1, installed_registry()).unwrap();
    let packed = lighting::pack(&runtime.render_frame());
    assert_eq!(packed.local_shadows.face_count(), 0);
    assert!(packed.locals.iter().all(|l| l.params[2] == 0.0));
}

#[test]
fn invalid_light_content_is_rejected_by_the_store() {
    use axiom_core::{Request, Store};
    let mut store = Store::memory_with(installed_registry().authoring, pack("light-lab")).unwrap();
    for (script, reason) in [
        (
            "set sign/light render.light/shadows true",
            "area light shadows not supported yet",
        ),
        (
            "set statue/spot render.light/shadows true\nset statue/spot render.light/spot/outer_degrees 170",
            "spot light shadows require outer_degrees <= 160",
        ),
        (
            "set lamp/west/light render.light/shadows true\nset lamp/east/light render.light/shadows true",
            "point light shadow budget exceeded (2 > 1",
        ),
        (
            "set moon render.light/range 4",
            "directional light has no range",
        ),
        ("set sign/light render.light/kind \"spot\"", "requires spot"),
        ("set kiosk/light render.light/glow 1", "unknown field"),
        (
            "set resources[id=render.environment] ambient {\"mode\":\"hdri\"}",
            "unknown variant",
        ),
    ] {
        let error = store
            .execute(Request::Acl {
                text: script.into(),
                preview: false,
                expected_rev: None,
                propose: false,
            })
            .unwrap_err();
        assert!(format!("{error:#}").contains(reason), "{script}: {error:#}");
    }
}
