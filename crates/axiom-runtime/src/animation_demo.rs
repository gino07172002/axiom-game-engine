//! Local lab pack generation. Referenced artwork stays outside the repository.
use anyhow::{ensure, Context, Result};
use axiom_animation::{
    authoring::*,
    clip::*,
    humanoid,
    ik::TwoBoneIk,
    twist::{Twist, TwistBone},
    *,
};
use axiom_core::*;
use glam::{Mat4, Quat, Vec3, Vec4};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};
pub fn profile(asset: &Asset) -> Result<humanoid::Profile> {
    let p = asset
        .humanoid
        .clone()
        .unwrap_or_else(|| humanoid::auto_map(&asset.skeleton).0);
    p.compile(&asset.skeleton, true)?;
    Ok(p)
}
pub fn create(model: &Path, source: Option<&Path>, output: &Path) -> Result<serde_json::Value> {
    let path = model.canonicalize()?;
    let asset = import::load(&path, &Default::default())?;
    let target_profile = profile(&asset)?;
    let mut lib = Library::default();
    lib.assets.insert(
        "character".into(),
        Source {
            path: path.clone(),
            external: true,
            options: Default::default(),
        },
    );
    lib.profiles.insert("target".into(), target_profile.clone());
    let source_asset = source
        .map(|p| import::load(p, &Default::default()))
        .transpose()?;
    let source_profile = source_asset.as_ref().map(profile).transpose()?;
    let (driver, driver_profile) = if let (Some(a), Some(p)) = (&source_asset, &source_profile) {
        (a, p)
    } else {
        (&asset, &target_profile)
    };
    let s = &driver.skeleton;
    let arm = s.bone(&driver_profile.mapping["LeftUpperArm"])?;
    let lower = s.bone(&driver_profile.mapping["LeftLowerArm"])?;
    let delta = Quat::from_axis_angle(Vec3::Z, 0.5);
    let arm_rest = s.source.bones[arm].rest.rotation;
    let lower_rest = s.source.bones[lower].rest.rotation;
    for (id, angle) in [("wave", 1.4), ("gentle", 0.4)] {
        let rotation = |q: Quat| Vec4::from_array(q.to_array());
        lib.clips.insert(
            id.into(),
            Clip {
                id: id.into(),
                duration: 2.,
                tracks: vec![
                    Track {
                        bone: s.source.bones[arm].id.clone(),
                        property: Property::Rotation,
                        interpolation: Interpolation::Linear,
                        times: vec![0., 1., 2.],
                        values: vec![
                            rotation(arm_rest),
                            rotation(arm_rest * delta),
                            rotation(arm_rest),
                        ],
                    },
                    Track {
                        bone: s.source.bones[lower].id.clone(),
                        property: Property::Rotation,
                        interpolation: Interpolation::Linear,
                        times: vec![0., 1., 2.],
                        values: vec![
                            rotation(lower_rest),
                            rotation(lower_rest * Quat::from_axis_angle(Vec3::Y, angle)),
                            rotation(lower_rest),
                        ],
                    },
                ],
                morphs: vec![],
            },
        );
    }
    let retarget = if let (Some(source), Some(p)) = (source, &source_profile) {
        lib.assets.insert(
            "source".into(),
            Source {
                path: source.canonicalize()?,
                external: true,
                options: Default::default(),
            },
        );
        lib.profiles.insert("source".into(), p.clone());
        Some(Retarget {
            source_asset: "source".into(),
            source_profile: "source".into(),
            target_profile: "target".into(),
            root_motion: false,
            require_complete: true,
        })
    } else {
        None
    };
    let mut twists = BTreeMap::new();
    let target = &asset.skeleton;
    for (side, jp) in [("Left", "左"), ("Right", "右")] {
        for (role, next_role, twist_name) in [
            ("UpperArm", "LowerArm", "腕捩"),
            ("LowerArm", "Hand", "手捩"),
        ] {
            let driver = &target_profile.mapping[&format!("{side}{role}")];
            let end = &target_profile.mapping[&format!("{side}{next_role}")];
            let start = target.bone(driver)?;
            let end_i = target.bone(end)?;
            let axis_world = (target.rest_global[end_i].w_axis - target.rest_global[start].w_axis)
                .truncate()
                .normalize();
            let axis = target.rest_global[start]
                .to_scale_rotation_translation()
                .1
                .inverse()
                * axis_world;
            let mut bones = vec![];
            for (n, fraction) in [("", 1.), ("1", 0.25), ("2", 0.5), ("3", 0.75)] {
                let id = format!("{jp}{twist_name}{n}");
                if target.indices.contains_key(&id) {
                    bones.push(TwistBone { bone: id, fraction });
                }
            }
            if !bones.is_empty() {
                twists.insert(
                    format!("{side}{role}"),
                    Twist {
                        driver: driver.clone(),
                        axis,
                        bones,
                        driver_fraction: 0.,
                        preserve: vec![
                            end.clone(),
                            target_profile.mapping[&format!("{side}Hand")].clone(),
                        ],
                        ..Default::default()
                    },
                );
            }
        }
    }
    let end = &target_profile.mapping["LeftHand"];
    let end_position = target.rest_global[target.bone(end)?].w_axis.truncate();
    let shoulder = &target_profile.mapping["LeftUpperArm"];
    let middle = &target_profile.mapping["LeftLowerArm"];
    let ik = TwoBoneIk {
        root: shoulder.clone(),
        middle: middle.clone(),
        end: end.clone(),
        target: end_position + Vec3::new(-0.08, 0.08, 0.13),
        pole: end_position + Vec3::new(0., -0.3, 0.5),
        weight: 0.,
        end_rotation: None,
    };
    let initial = source_asset
        .as_ref()
        .unwrap_or(&asset)
        .clips
        .first()
        .map_or_else(|| "wave".into(), |clip| clip.id.clone());
    let mut player = Player {
        asset: "character".into(),
        clip: Some(initial.clone()),
        retarget,
        twists,
        ik: BTreeMap::from([("left_hand".into(), ik.clone())]),
        ..Default::default()
    };
    let mut w = World::empty("animation-lab");
    w.title = "Animation Lab / Retarget + Twist + IK".into();
    w.capabilities.extend([
        ("render".into(), 1),
        ("animation".into(), 1),
        ("behavior".into(), 1),
        ("ui".into(), 1),
    ]);
    w.systems = vec!["behavior.execute".into(), "animation.evaluate".into()];
    let mut e = Entity::new("character");
    e.set(
        "core.transform",
        Transform {
            position: [if source_asset.is_some() { 0.65 } else { 0.0 }, 0., 0.],
            ..Default::default()
        },
    )?;
    e.set("animation.player", &player)?;
    w.entities.push(e);
    if let Some(source) = &source_asset {
        let mut e = Entity::new("source");
        let rotation = target_profile.reference_heading(&asset.skeleton)?
            * source_profile
                .as_ref()
                .context("source profile missing")?
                .reference_heading(&source.skeleton)?
                .inverse();
        let (bottom, top) = source
            .vertices
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), v| {
                (min.min(v.position.y), max.max(v.position.y))
            });
        let scale = if (top - bottom).is_finite() && top - bottom > 0.001 {
            (1.85 / (top - bottom)).clamp(0.0001, 100.0)
        } else {
            1.0
        };
        let floor_offset = if bottom.is_finite() {
            -bottom * scale
        } else {
            0.0
        };
        e.set(
            "core.transform",
            Transform {
                position: [-0.85, floor_offset, 0.],
                scale: [scale; 3],
                rotation: rotation.normalize().to_array(),
            },
        )?;
        player.asset = "source".into();
        player.retarget = None;
        player.twists.clear();
        player.ik.clear();
        e.set("animation.player", &player)?;
        w.entities.push(e);
    }
    let mut floor = Entity::new("floor");
    floor.set(
        "core.transform",
        Transform {
            position: [0., -0.04, 0.],
            scale: [7., 0.04, 4.],
            ..Default::default()
        },
    )?;
    floor.set(
        "render.mesh",
        RenderMesh {
            parts: vec![MeshPart {
                color: [0.12, 0.17, 0.2],
                grid: true,
                transform: Transform::default(),
                emission: 0.0,
            }],
        },
    )?;
    w.entities.push(floor);
    let mut camera = Entity::new("camera");
    let m = Mat4::look_at_rh(Vec3::new(0., 1.28, 4.2), Vec3::new(0., 0.79, 0.), Vec3::Y).inverse();
    let (_, r, t) = m.to_scale_rotation_translation();
    camera.set(
        "core.transform",
        Transform {
            position: t.to_array(),
            rotation: r.to_array(),
            ..Default::default()
        },
    )?;
    let mut cam = Camera::perspective();
    cam.projection = Projection::Orthographic { height: 3.25 };
    camera.set("render.camera", cam)?;
    w.entities.push(camera);
    w.resources
        .insert("animation.library".into(), serde_json::to_value(lib)?);
    w.resources
        .insert("behavior.program".into(), json!({"variables":{}}));
    w.resources.insert(
        "input.bindings".into(),
        json!({"axes":{},"buttons":{},"capture_pointer":false}),
    );
    let mut catalog = BTreeMap::from([
        (
            "title".to_owned(),
            if source_asset.is_some() {
                "ANIMATION LAB / 左：來源　右：重定向"
            } else {
                "ANIMATION LAB / 骨架動畫"
            }
            .to_owned(),
        ),
        (
            "hint".into(),
            "Rigify / Humanoid retarget → Twist correction → Two-bone IK → Skinning".into(),
        ),
    ]);
    let mut widgets =
        vec![json!({"kind":"text","id":"title","binding":{"kind":"key","id":"title"}})];
    let mut buttons = vec![
        (
            "wave",
            "Wave",
            json!({"kind":"call","capability":"animation","action":"play","args":{"entity":"character","clip":"wave","fade_seconds":0.2}}),
        ),
        (
            "gentle",
            "Blend",
            json!({"kind":"call","capability":"animation","action":"play","args":{"entity":"character","clip":"gentle","fade_seconds":0.4}}),
        ),
        (
            "freeze",
            "Pose 0.5s",
            json!({"kind":"call","capability":"animation","action":"seek","args":{"entity":"character","seconds":0.5,"paused":true}}),
        ),
        (
            "ik-on",
            "IK ON",
            json!({"kind":"call","capability":"animation","action":"ik","args":{"entity":"character","id":"left_hand","target":ik.target,"weight":1.}}),
        ),
        (
            "ik-off",
            "IK OFF",
            json!({"kind":"call","capability":"animation","action":"ik","args":{"entity":"character","id":"left_hand","target":ik.target,"weight":0.}}),
        ),
    ];
    if initial != "wave" {
        buttons.push(("source-clip","Imported clip",json!({"kind":"call","capability":"animation","action":"play","args":{"entity":"character","clip":initial,"fade_seconds":0.2}})));
    }
    if let Some(id) = w.entities[0].components["animation.player"]["twists"]
        .as_object()
        .and_then(|m| {
            m.get_key_value("LeftLowerArm")
                .map(|(id, _)| id)
                .or_else(|| m.keys().next())
        })
        .cloned()
    {
        buttons.push(("twist-off","Twist OFF",json!({"kind":"call","capability":"animation","action":"twist","args":{"entity":"character","id":id,"enabled":false}})));
        buttons.push(("twist-on","Twist ON",json!({"kind":"call","capability":"animation","action":"twist","args":{"entity":"character","id":id,"enabled":true}})));
    }
    for (id, label, command) in buttons {
        catalog.insert(id.into(), label.into());
        widgets.push(json!({"kind":"button","id":id,"text":id,"command":command}));
    }
    widgets.push(json!({"kind":"text","id":"hint","binding":{"kind":"key","id":"hint"}}));
    w.resources.insert("ui.document".into(),json!({"flow":true,"session_controls":false,"locale":"zh-TW","catalogs":{"zh-TW":catalog},"images":{},"widgets":widgets}));
    let registry = crate::installed_registry();
    w.compile(&registry.authoring)?;
    crate::WorldRuntime::new(w.clone(), 1, registry)?;
    std::fs::create_dir_all(output)?;
    std::fs::write(output.join("world.json"), serde_json::to_vec_pretty(&w)?)?;
    std::fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(
            &json!({"id":"animation-lab","version":1,"world":"world.json","capabilities":w.capabilities}),
        )?,
    )?;
    Ok(
        json!({"ok":true,"pack":output,"reference":asset.summary(),"source":source_asset.map(|a|a.summary()),"initial_clip":initial,"note":"External artwork is referenced in place, not copied into the pack"}),
    )
}
pub fn retarget(
    source: &Path,
    target: &Path,
    clip_id: &str,
    sp: Option<&Path>,
    tp: Option<&Path>,
    fps: u32,
    output: &Path,
) -> Result<serde_json::Value> {
    ensure!((1..=120).contains(&fps), "sampling FPS must be 1..120");
    let source = import::load(source, &Default::default())?;
    let target = import::load(target, &Default::default())?;
    let sp = sp
        .map(|p| -> Result<humanoid::Profile> { Ok(serde_json::from_slice(&std::fs::read(p)?)?) })
        .transpose()?
        .map_or_else(|| profile(&source), Ok)?;
    let tp = tp
        .map(|p| -> Result<humanoid::Profile> { Ok(serde_json::from_slice(&std::fs::read(p)?)?) })
        .transpose()?
        .map_or_else(|| profile(&target), Ok)?;
    let clip = source
        .clips
        .iter()
        .find(|c| c.id == clip_id)
        .context("source clip not found")?
        // Bone tracks only: morph weights are not retargeted (the output has none).
        .compile_for(&source.skeleton, &source.morphs)?;
    let retarget = humanoid::Retargeter::new(&source.skeleton, &sp, &target.skeleton, &tp, true)?;
    ensure!(
        clip.source.duration <= 600.,
        "retarget bake exceeds 600 seconds"
    );
    let count = (clip.source.duration * fps as f32).ceil() as usize + 1;
    ensure!(
        count * target.skeleton.source.bones.len() * 3 <= 2_000_000,
        "retarget key budget exceeded"
    );
    let times = (0..count)
        .map(|i| (i as f32 / fps as f32).min(clip.source.duration))
        .collect::<Vec<_>>();
    let poses = times
        .iter()
        .map(|t| retarget.apply(&clip.sample(&source.skeleton, *t, false)?, true))
        .collect::<Result<Vec<_>>>()?;
    let mut tracks = vec![];
    for (i, b) in target.skeleton.source.bones.iter().enumerate() {
        for property in [Property::Translation, Property::Rotation, Property::Scale] {
            let value = |trs: Trs| match property {
                Property::Translation => trs.translation.extend(0.),
                Property::Rotation => Vec4::from_array(trs.rotation.to_array()),
                Property::Scale => trs.scale.extend(0.),
            };
            let values = poses.iter().map(|p| value(p.local[i])).collect::<Vec<_>>();
            let rest = value(b.rest);
            let same = |a: Vec4, b: Vec4| {
                a.abs_diff_eq(b, 1e-6)
                    || (property == Property::Rotation && a.abs_diff_eq(-b, 1e-6))
            };
            if values.iter().all(|&v| same(v, rest)) {
                continue;
            }
            let (times, values) = if values.iter().all(|&v| same(v, values[0])) {
                (vec![0.0], vec![values[0]])
            } else {
                (times.clone(), values)
            };
            tracks.push(Track {
                bone: b.id.clone(),
                property,
                interpolation: Interpolation::Linear,
                times,
                values,
            });
        }
    }
    let out = Clip {
        id: format!("retarget/{clip_id}"),
        duration: clip.source.duration,
        tracks,
        morphs: vec![],
    };
    out.compile(&target.skeleton)?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, serde_json::to_vec(&out)?)?;
    Ok(
        json!({"ok":true,"output":output,"frames":count,"tracks":out.tracks.len(),"root_translation_scale":retarget.root_scale}),
    )
}
