use axiom_animation::{
    clip::*, coordinates::Coordinates, humanoid::*, ik::*, skin::*, twist::*, *,
};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn chain() -> CompiledSkeleton {
    let mut s = Skeleton {
        bones: (0..4)
            .map(|i| Bone {
                id: format!("b{i}"),
                aliases: vec![],
                parent: if i == 0 { None } else { Some(i - 1) },
                rest: Trs {
                    translation: if i == 0 { Vec3::ZERO } else { Vec3::X },
                    ..Default::default()
                },
                inverse_bind: Mat4::IDENTITY,
            })
            .collect(),
    }
    .compile()
    .unwrap();
    s.rebind();
    s
}
fn asset(s: CompiledSkeleton) -> Asset {
    Asset {
        key: "test".into(),
        skeleton: s,
        vertices: vec![Vertex {
            position: Vec3::new(1., 1., 0.),
            normal: Vec3::Z,
            uv: Vec2::ZERO,
            joints: [0, 1, 0, 0],
            weights: [0.5, 0.5, 0., 0.],
            sdef: None,
            dual_quaternion: false,
        }],
        indices: vec![0, 0, 0],
        materials: vec![Material {
            id: "m".into(),
            name: None,
            first_index: 0,
            index_count: 3,
            color: [1.; 4],
            texture: None,
            double_sided: false,
            alpha: AlphaMode::Opaque,
        }],
        textures: vec![],
        clips: vec![],
        diagnostics: vec![],
        humanoid: None,
    }
}
fn near(a: Vec3, b: Vec3) {
    assert!(a.distance(b) < 2e-4, "{a:?} != {b:?}")
}
#[test]
fn skeleton_validates_cycles_duplicates_and_parent_order() {
    let mut source = chain().source;
    source.bones[0].parent = Some(3);
    assert!(source.compile().is_err());
    source.bones[0].parent = None;
    source.bones[3].id = "b0".into();
    assert!(source.compile().is_err());
    let mut source = chain().source;
    source.bones.swap(0, 3);
    source.bones[0].parent = Some(2);
    source.bones[1].parent = Some(3);
    source.bones[3].parent = None;
    let s = source.compile().unwrap();
    assert_eq!(s.order, vec![3, 1, 2, 0]);
}
#[test]
fn coordinate_presets_convert_points_rotations_units_and_reflection() {
    near(
        Coordinates::preset("blender")
            .unwrap()
            .point(Vec3::Z)
            .unwrap(),
        Vec3::Y,
    );
    near(
        Coordinates::preset("unity")
            .unwrap()
            .point(Vec3::Z)
            .unwrap(),
        -Vec3::Z,
    );
    near(
        Coordinates::preset("unreal")
            .unwrap()
            .point(Vec3::new(0., 0., 100.))
            .unwrap(),
        Vec3::Y,
    );
    for name in ["gltf", "blender", "unity", "unreal"] {
        let c = Coordinates::preset(name).unwrap();
        let q = Quat::from_rotation_y(0.7);
        let p = Vec3::new(0.2, 0.7, 1.1);
        near(
            c.point(q * p).unwrap(),
            c.rotation(q).unwrap() * c.point(p).unwrap(),
        );
    }
    let mut invalid = Coordinates::default();
    invalid.basis[0] = [2., 0., 0.];
    assert!(invalid.matrix().is_err());
}
#[test]
fn clips_interpolate_step_linear_cubic_and_antipodal_rotations() {
    let s = chain();
    let mut track = Track {
        bone: "b0".into(),
        property: Property::Translation,
        interpolation: Interpolation::Linear,
        times: vec![0., 1.],
        values: vec![Vec4::ZERO, Vec3::X.extend(0.)],
    };
    near(track.sample(0.25).truncate(), Vec3::X * 0.25);
    track.interpolation = Interpolation::Step;
    near(track.sample(0.5).truncate(), Vec3::ZERO);
    track.interpolation = Interpolation::CubicSpline;
    track.values = vec![
        Vec4::ZERO,
        Vec4::ZERO,
        Vec3::X.extend(0.),
        Vec3::X.extend(0.),
        Vec3::X.extend(0.),
        Vec4::ZERO,
    ];
    near(track.sample(0.5).truncate(), Vec3::X * 0.5);
    track.property = Property::Rotation;
    track.interpolation = Interpolation::Linear;
    track.values = vec![Vec4::W, -Vec4::W];
    assert!(Quat::from_array(track.sample(0.5).to_array()).abs_diff_eq(Quat::IDENTITY, 1e-5));
    let mut clip = Clip {
        id: "clip".into(),
        duration: 1.,
        tracks: vec![track],
    };
    let c = clip.compile(&s).unwrap();
    c.sample(&s, -3.5, true).unwrap();
    clip.tracks[0].times = vec![1., 0.];
    assert!(clip.compile(&s).is_err());
}
#[test]
fn linear_sdef_and_dual_quaternion_preserve_bind_pose_and_rigid_translation() {
    let s = chain();
    let mut a = asset(s);
    a.validate().unwrap();
    let mut p = a.skeleton.rest_pose();
    for mode in [Skinning::Linear, Skinning::Auto, Skinning::DualQuaternion] {
        near(
            deform(&a, &p, mode).unwrap()[0].position,
            a.vertices[0].position,
        );
    }
    a.vertices[0].sdef = Some(Sdef {
        c: Vec3::new(0.3, 0.2, 0.),
        r0: Vec3::X,
        r1: Vec3::Y,
    });
    near(
        deform(&a, &p, Skinning::Auto).unwrap()[0].position,
        a.vertices[0].position,
    );
    p.local[0].translation = Vec3::new(2., 3., 4.);
    for mode in [Skinning::Linear, Skinning::Auto, Skinning::DualQuaternion] {
        near(
            deform(&a, &p, mode).unwrap()[0].position,
            a.vertices[0].position + Vec3::new(2., 3., 4.),
        );
    }
}
#[test]
fn skinning_uses_inverse_transpose_normals_under_nonuniform_scale() {
    let mut a = asset(chain());
    a.vertices[0].normal = Vec3::new(1., 1., 0.).normalize();
    a.vertices[0].weights = [1., 0., 0., 0.];
    let mut p = a.skeleton.rest_pose();
    p.local[0].scale = Vec3::new(2., 1., 1.);
    let v = deform(&a, &p, Skinning::Linear).unwrap()[0];
    near(v.normal, Vec3::new(0.5, 1., 0.).normalize());
}
#[test]
fn twist_fractions_are_absolute_avoid_double_twist_and_preserve_end_rotation() {
    let s = chain();
    let mut p = s.rest_pose();
    p.local[0].rotation = Quat::from_rotation_x(2.);
    let old = s.globals(&p).unwrap()[3].to_scale_rotation_translation().1;
    let config = Twist {
        driver: "b0".into(),
        axis: Vec3::X,
        driver_fraction: 0.,
        bones: vec![
            TwistBone {
                bone: "b1".into(),
                fraction: 0.5,
            },
            TwistBone {
                bone: "b2".into(),
                fraction: 0.75,
            },
        ],
        preserve: vec!["b3".into()],
        ..Default::default()
    };
    config.compile(&s).unwrap().apply(&s, &mut p).unwrap();
    let g = s.globals(&p).unwrap();
    for (i, angle) in [(0, 0.), (1, 1.), (2, 1.5)] {
        assert!(g[i]
            .to_scale_rotation_translation()
            .1
            .abs_diff_eq(Quat::from_rotation_x(angle), 1e-4));
    }
    assert!(g[3]
        .to_scale_rotation_translation()
        .1
        .abs_diff_eq(old, 1e-4));
    let mut off = config.clone();
    off.enabled = false;
    let original = json!(p);
    off.compile(&s).unwrap().apply(&s, &mut p).unwrap();
    assert_eq!(json!(p), original);
    let mut clamp = config;
    clamp.max_angle = 0.4;
    let mut p = s.rest_pose();
    p.local[0].rotation = Quat::from_rotation_x(2.);
    let angle = clamp.compile(&s).unwrap().apply(&s, &mut p).unwrap();
    assert!((angle - 0.4).abs() < 1e-5);
}
#[test]
fn two_bone_ik_uses_pole_preserves_lengths_and_reports_unreachable() {
    let s = chain();
    let mut p = s.rest_pose();
    let config = TwoBoneIk {
        root: "b0".into(),
        middle: "b1".into(),
        end: "b2".into(),
        target: Vec3::new(1., 1., 0.),
        pole: Vec3::Y,
        weight: 1.,
        end_rotation: None,
    };
    let result = config.compile(&s).unwrap().apply(&s, &mut p).unwrap();
    assert!(result.reachable && result.residual < 1e-4);
    let g = s.globals(&p).unwrap();
    near(g[2].w_axis.truncate(), config.target);
    assert!((g[0].w_axis.truncate().distance(g[1].w_axis.truncate()) - 1.).abs() < 1e-5);
    let mut far = config;
    far.target = Vec3::X * 10.;
    let r = far.compile(&s).unwrap().apply(&s, &mut p).unwrap();
    assert!(!r.reachable && r.residual > 7.9);
    far.weight = 0.;
    let before = json!(p);
    far.compile(&s).unwrap().apply(&s, &mut p).unwrap();
    assert_eq!(json!(p), before);
}
#[test]
fn retarget_respects_reference_frames_without_changing_limb_lengths() {
    let source = chain();
    let mut target = chain();
    target.source.bones[1].rest.rotation = Quat::from_rotation_z(0.4);
    target = target.source.compile().unwrap();
    target.rebind();
    let map = Profile {
        mapping: BTreeMap::from([
            ("LeftUpperArm".into(), "b0".into()),
            ("LeftLowerArm".into(), "b1".into()),
            ("LeftHand".into(), "b2".into()),
        ]),
        reference: BTreeMap::new(),
    };
    let rt = Retargeter::new(&source, &map, &target, &map, false).unwrap();
    let mut pose = source.rest_pose();
    pose.local[0].rotation = Quat::from_rotation_y(0.8);
    pose.local[1].rotation = Quat::from_rotation_x(0.3);
    let out = rt.apply(&pose, false).unwrap();
    let a = source.globals(&pose).unwrap();
    let b = target.globals(&out).unwrap();
    let expected = a[1].to_scale_rotation_translation().1
        * target.rest_global[1].to_scale_rotation_translation().1;
    assert!(b[1]
        .to_scale_rotation_translation()
        .1
        .abs_diff_eq(expected, 1e-4));
    for (i, t) in out.local.iter().enumerate() {
        near(t.translation, target.source.bones[i].rest.translation);
    }
}
#[test]
fn actual_blender_rigify_glb_maps_complete_humanoid_and_matches_evaluated_bones() {
    let a = import::load(
        &root().join("content/animation/rigify.glb"),
        &Default::default(),
    )
    .unwrap();
    let (profile, report) = auto_map(&a.skeleton);
    assert_eq!(report["missing_required"], json!([]));
    profile.compile(&a.skeleton, true).unwrap();
    assert!(a.vertices.len() > 1000);
    let expected: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("content/animation/rigify-expected.json")).unwrap(),
    )
    .unwrap();
    let clip = a.clips[0].compile(&a.skeleton).unwrap();
    let offset = clip
        .source
        .tracks
        .iter()
        .flat_map(|t| t.times.first().copied())
        .fold(f32::INFINITY, f32::min);
    let mut error = 0.0_f32;
    for sample in expected["expected"].as_array().unwrap() {
        let time = sample["seconds"].as_f64().unwrap() as f32 + offset;
        let pose = clip.sample(&a.skeleton, time, false).unwrap();
        let g = a.skeleton.globals(&pose).unwrap();
        for (id, value) in sample["positions"].as_object().unwrap() {
            let expected: Vec3 = serde_json::from_value(value.clone()).unwrap();
            error = error.max(
                g[a.skeleton.bone(id).unwrap()]
                    .w_axis
                    .truncate()
                    .distance(expected),
            );
        }
    }
    assert!(
        error < 0.001,
        "Blender evaluated-bone disagreement: {error} meters, key offset {offset}"
    );
}
#[test]
fn unity_bundle_roundtrip_converts_handedness_and_preserves_humanoid_mapping() {
    let glb = import::load(
        &root().join("content/animation/rigify.glb"),
        &Default::default(),
    )
    .unwrap();
    let (profile, _) = auto_map(&glb.skeleton);
    let c = Coordinates::preset("unity").unwrap();
    let mut skeleton = glb.skeleton.source.clone();
    for b in &mut skeleton.bones {
        b.rest = Trs::from_matrix(c.transform(b.rest.matrix()).unwrap()).unwrap();
        b.inverse_bind = c.transform(b.inverse_bind).unwrap();
    }
    let bundle = import::bundle::Bundle {
        version: 1,
        coordinates: c,
        skeleton,
        vertices: vec![],
        indices: vec![],
        materials: vec![],
        textures: vec![],
        clips: vec![],
        humanoid: profile,
    };
    let path = std::env::temp_dir().join(format!("axiom-unity-{}.axrig", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(&bundle).unwrap()).unwrap();
    let imported = import::load(&path, &Default::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    imported
        .humanoid
        .as_ref()
        .unwrap()
        .compile(&imported.skeleton, true)
        .unwrap();
    for (a, b) in glb
        .skeleton
        .rest_global
        .iter()
        .zip(&imported.skeleton.rest_global)
    {
        assert!(a.abs_diff_eq(*b, 1e-4));
    }
}
#[test]
fn malformed_assets_fail_recoverably_and_clip_queries_stay_compact() {
    let file = std::env::temp_dir().join(format!("axiom-bad-{}.pmx", std::process::id()));
    std::fs::write(&file, b"PMX \0\0").unwrap();
    assert!(import::load(&file, &Default::default()).is_err());
    std::fs::remove_file(file).unwrap();
    let a = import::load(
        &root().join("content/animation/rigify.glb"),
        &Default::default(),
    )
    .unwrap();
    assert!(serde_json::to_vec(&a.summary()).unwrap().len() < 4096);
}
