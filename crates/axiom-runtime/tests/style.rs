//! Stylized meshes (`render.toon`, `render.outline`, `render.decal`): what the runtime hands
//! the renderer for `content/style-lab`, bone-driven face / projector spaces, and the errors
//! for unknown materials, bones and images.
use axiom_core::{ContentPackage, World};
use axiom_runtime::world::{ActionMap, WorldRuntime};
use glam::Mat4;
use serde_json::json;

fn lab() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/style-lab"),
        &axiom_runtime::installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn runtime(world: World) -> anyhow::Result<WorldRuntime> {
    WorldRuntime::new(world, 1, axiom_runtime::installed_registry())
}

#[test]
fn style_lab_presents_per_primitive_styles() {
    let mut runtime = runtime(lab()).unwrap();
    for _ in 0..30 {
        runtime.step(&ActionMap::default());
    }
    let frame = runtime.render_frame();
    let mesh = |id: &str| frame.meshes.iter().find(|m| m.id == id).unwrap();
    // No style component: nothing changes for the plain mascot.
    let plain = mesh("mascot/plain");
    assert!(plain.style.is_none());
    assert_eq!(plain.style_space, [Mat4::IDENTITY; 2]);

    let toon = mesh("mascot/toon");
    let style = toon.style.as_ref().unwrap();
    let index = |id: &str| {
        toon.asset
            .materials
            .iter()
            .position(|m| m.name.as_deref() == Some(id))
            .unwrap()
    };
    let skin = style.primitives[index("skin")];
    let hair = style.primitives[index("hair")];
    assert_eq!(skin.toon[0], 1.0, "bands");
    assert_eq!(skin.toon[3], 0.45, "skin wrap on skin");
    assert_eq!(hair.toon[3], 0.0, "no skin wrap on hair");
    assert_eq!(skin.face_blend, 0.75);
    assert_eq!(hair.face_blend, 0.0);
    assert_eq!(skin.outline, -2.5, "pixels are negative");
    assert!(style.decal.is_none());
    // The face space is the head bone in world space (above the entity origin).
    let head = toon.style_space[0].w_axis.truncate();
    assert!(head.y > 0.8, "{head}");

    let decal = mesh("mascot/decal");
    let style = decal.style.as_ref().unwrap();
    let image = &style.decal.as_ref().unwrap().image;
    assert_eq!((image.width, image.height), (256, 256));
    assert_eq!(image.rgba.len(), 256 * 256 * 4);
    assert!(style.primitives.iter().all(|p| p.decal && p.outline > 0.0));
    let chest = decal.style_space[1].w_axis.truncate();
    assert!(chest.y > 0.6 && chest.y < 1.0, "{chest}");

    let ball = mesh("ball");
    let style = ball.style.as_ref().unwrap();
    assert_eq!(style.primitives[0].toon[0], 3.0);
    assert!(matches!(
        style.decal.as_ref().unwrap().mapping,
        axiom_runtime::mesh_style::Mapping::Uv(_)
    ));
}

#[test]
fn unknown_materials_bones_and_images_are_reported_at_their_field() {
    let edit = |entity: &str, component: &str, value: serde_json::Value| {
        let mut world = lab();
        world
            .entities
            .iter_mut()
            .find(|e| e.id == entity)
            .unwrap()
            .components
            .insert(component.into(), value);
        runtime(world).err().map(|e| format!("{e:#}"))
    };
    let error = edit("mascot/toon", "render.toon", json!({"materials":["cape"]})).unwrap();
    assert!(
        error.contains("render.toon") && error.contains("unknown material cape"),
        "{error}"
    );
    let error = edit(
        "mascot/toon",
        "render.toon",
        json!({"face":{"bone":"tail"}}),
    )
    .unwrap();
    assert!(error.contains("face.bone: unknown bone tail"), "{error}");
    let error = edit(
        "mascot/decal",
        "render.decal",
        json!({"image":"emblem","projector":{"bone":"wing"}}),
    )
    .unwrap();
    assert!(
        error.contains("projector.bone: unknown bone wing"),
        "{error}"
    );
    // The image must exist in ui.document (world validation, before any file is read).
    let mut world = lab();
    world
        .entities
        .iter_mut()
        .find(|e| e.id == "ball")
        .unwrap()
        .components
        .insert("render.decal".into(), json!({"image":"nope"}));
    let error = axiom_runtime::installed_registry()
        .authoring
        .validate_world(&world)
        .unwrap_err();
    let error = format!("{error:#}");
    assert!(
        error.contains("unknown_image") && error.contains("@ball/components/render.decal/image"),
        "{error}"
    );
}

#[test]
fn style_changes_hot_reload_and_keep_animation_time() {
    let world = lab();
    let mut runtime = runtime(world.clone()).unwrap();
    for _ in 0..60 {
        runtime.step(&ActionMap::default());
    }
    let time = |r: &WorldRuntime| {
        r.observation().metrics["animation"]["players"]["mascot/toon"]["time"].clone()
    };
    let before = time(&runtime);
    let mut edited = world;
    edited
        .entities
        .iter_mut()
        .find(|e| e.id == "mascot/toon")
        .unwrap()
        .components
        .insert("render.toon".into(), json!({"bands":4}));
    runtime.reconcile(edited, 2).unwrap();
    assert_eq!(time(&runtime), before);
    let frame = runtime.render_frame();
    let toon = frame.meshes.iter().find(|m| m.id == "mascot/toon").unwrap();
    assert!(toon
        .style
        .as_ref()
        .unwrap()
        .primitives
        .iter()
        .all(|p| p.toon[0] == 4.0 && p.outline == -2.5));
}
