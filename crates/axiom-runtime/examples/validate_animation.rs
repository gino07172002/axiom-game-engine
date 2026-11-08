//! Numerical verification against our Blender export plus the user's external rig.
use anyhow::{ensure, Context, Result};
use axiom_animation::{humanoid, import, skin};
use axiom_core::{gameplay::RuntimeCommand, ContentPackage};
use axiom_runtime::WorldRuntime;
use glam::Vec3;
use serde_json::{json, Value};
use std::path::PathBuf;
fn call(r: &mut WorldRuntime, action: &str, args: Value) -> Result<Value> {
    r.command(&RuntimeCommand::Call {
        capability: "animation".into(),
        action: action.into(),
        args,
    })
}
fn mesh(r: &WorldRuntime) -> Vec<Vec3> {
    r.render_frame()
        .meshes
        .into_iter()
        .find(|m| m.id == "character")
        .unwrap()
        .vertices
        .iter()
        .map(|v| v.position)
        .collect()
}
fn difference(a: &[Vec3], b: &[Vec3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| a.distance(*b))
        .fold(0., f32::max)
}
fn main() -> Result<()> {
    let args = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    ensure!(args.len() == 2, "usage: validate_animation PACK REPORT");
    let registry = axiom_runtime::installed_registry();
    let package = ContentPackage::load(&args[0], &registry.authoring)?;
    let library: &Value = &package.world.resources["animation.library"];
    let target_path = PathBuf::from(
        library["assets"]["character"]["path"]
            .as_str()
            .context("target path")?,
    );
    let source_path = PathBuf::from(
        library["assets"]["source"]["path"]
            .as_str()
            .context("source path")?,
    );
    let target = import::load(&target_path, &Default::default())?;
    let source = import::load(&source_path, &Default::default())?;
    let (tp, target_mapping) = humanoid::auto_map(&target.skeleton);
    let (sp, source_mapping) = humanoid::auto_map(&source.skeleton);
    tp.compile(&target.skeleton, true)?;
    source
        .humanoid
        .as_ref()
        .unwrap_or(&sp)
        .compile(&source.skeleton, true)?;
    if source.humanoid.is_some() {
        let mut runtime = WorldRuntime::new(package.world, 1, registry)?;
        call(
            &mut runtime,
            "seek",
            json!({"entity":"character","seconds":0.0,"paused":true}),
        )?;
        let start = mesh(&runtime);
        call(
            &mut runtime,
            "seek",
            json!({"entity":"character","seconds":0.43,"paused":true}),
        )?;
        let moved = mesh(&runtime);
        let feet = ["LeftFoot", "RightFoot"]
            .map(|role| target.skeleton.bone(&tp.mapping[role]))
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        let foot_delta = target
            .vertices
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                v.joints
                    .iter()
                    .zip(v.weights)
                    .any(|(j, w)| w > 0.5 && feet.contains(j))
            })
            .map(|(i, _)| start[i].distance(moved[i]))
            .fold(0.0, f32::max);
        ensure!(
            foot_delta > 0.01,
            "Unity Walking must animate weighted PMX feet: {foot_delta} meters"
        );
        runtime.assert_valid()?;
        let report = json!({"ok":true,"source":source.summary(),"target":target.summary(),"avatar_mapped_bones":source.humanoid.as_ref().map(|p|p.mapping.len()),"reference_mesh_delta_m":difference(&start,&moved),"reference_weighted_foot_delta_m":foot_delta,"foot_mapping":{"left":tp.mapping["LeftFoot"],"right":tp.mapping["RightFoot"]}});
        std::fs::write(&args[1], serde_json::to_vec_pretty(&report)?)?;
        println!("{report}");
        return Ok(());
    }
    let expected: Value = serde_json::from_slice(&std::fs::read(
        source_path.with_file_name("rigify-expected.json"),
    )?)?;
    let clip = source.clips[0].compile(&source.skeleton)?;
    let offset = clip
        .source
        .tracks
        .iter()
        .flat_map(|t| t.times.first().copied())
        .fold(f32::INFINITY, f32::min);
    let mut max_error = 0.0_f32;
    for sample in expected["expected"]
        .as_array()
        .context("expected samples")?
    {
        let p = clip.sample(
            &source.skeleton,
            sample["seconds"].as_f64().unwrap() as f32 + offset,
            false,
        )?;
        let g = source.skeleton.globals(&p)?;
        for (id, p) in sample["positions"].as_object().unwrap() {
            let p: Vec3 = serde_json::from_value(p.clone())?;
            max_error = max_error.max(g[source.skeleton.bone(id)?].w_axis.truncate().distance(p));
        }
    }
    ensure!(
        max_error < 0.001,
        "Rigify/Blender pose disagreement {max_error} m"
    );
    let rest = skin::deform(&target, &target.skeleton.rest_pose(), skin::Skinning::Auto)?;
    let rest_error = rest
        .iter()
        .zip(&target.vertices)
        .map(|(a, b)| a.position.distance(b.position))
        .fold(0., f32::max);
    ensure!(
        rest_error < 0.001,
        "reference bind pose deformation {rest_error}"
    );
    let mut runtime = WorldRuntime::new(package.world, 1, registry)?;
    call(
        &mut runtime,
        "seek",
        json!({"entity":"character","seconds":0.,"paused":true}),
    )?;
    let first = mesh(&runtime);
    call(
        &mut runtime,
        "seek",
        json!({"entity":"character","seconds":0.5,"paused":true}),
    )?;
    let animated = mesh(&runtime);
    let animation_delta = difference(&first, &animated);
    ensure!(
        animation_delta > 0.01,
        "reference mesh does not animate: {animation_delta}"
    );
    call(
        &mut runtime,
        "play",
        json!({"entity":"character","clip":"wave"}),
    )?;
    call(
        &mut runtime,
        "seek",
        json!({"entity":"character","seconds":1.,"paused":true}),
    )?;
    let corrected = mesh(&runtime);
    call(
        &mut runtime,
        "twist",
        json!({"entity":"character","id":"LeftLowerArm","enabled":false}),
    )?;
    let uncorrected = mesh(&runtime);
    let twist_delta = difference(&corrected, &uncorrected);
    ensure!(
        twist_delta > 1e-5,
        "twist configuration does not affect weighted reference geometry: {twist_delta}"
    );
    call(
        &mut runtime,
        "twist",
        json!({"entity":"character","id":"LeftLowerArm","enabled":true}),
    )?;
    let root = &tp.mapping["LeftUpperArm"];
    let mid = &tp.mapping["LeftLowerArm"];
    let end = &tp.mapping["LeftHand"];
    let observed = call(
        &mut runtime,
        "inspect",
        json!({"entity":"character","bones":[root,mid,end]}),
    )?;
    let p = &observed["result"]["bones"];
    let a: Vec3 = serde_json::from_value(p[root]["position"].clone())?;
    let b: Vec3 = serde_json::from_value(p[mid]["position"].clone())?;
    let c: Vec3 = serde_json::from_value(p[end]["position"].clone())?;
    let reach = a.distance(b) + b.distance(c);
    let target_position = a + (c - a + Vec3::new(0., 0.1, 0.2)).normalize() * reach * 0.7;
    let solved = call(
        &mut runtime,
        "ik",
        json!({"entity":"character","id":"left_hand","target":target_position,"pole":a+Vec3::new(0.,-0.5,1.),"weight":1.}),
    )?;
    let residual = solved["result"]["ik"]["left_hand"]["residual"]
        .as_f64()
        .context("IK result")?;
    ensure!(residual < 0.002, "reference IK residual {residual}");
    runtime.assert_valid()?;
    let report = json!({"ok":true,"source":source.summary(),"target":target.summary(),"source_mapping":source_mapping,"target_mapping":target_mapping,"blender":expected["blender"],"rigify_max_bone_error_m":max_error,"rigify_time_offset":offset,"reference_bind_error_m":rest_error,"reference_animated_vertex_delta_m":animation_delta,"reference_twist_vertex_delta_m":twist_delta,"reference_ik_residual_m":residual,"ik_target":target_position,"ik_pole":a+Vec3::new(0.,-0.5,1.),"ik_result":solved,"coordinate_cases":["gltf","blender","unity","unreal","custom orthonormal basis"],"unity_editor_executed":false,"scope":"Blender exported Rigify evaluated joints compared numerically; PMX real weighted geometry animated, twist toggled and IK solved. Unity bundle/parser/profile tested separately; Unity Editor execution is not claimed."});
    if let Some(parent) = args[1].parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&args[1], serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}
