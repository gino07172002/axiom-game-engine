use crate::*;
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skinning {
    #[default]
    Auto,
    Linear,
    DualQuaternion,
}
pub fn deform(asset: &Asset, pose: &Pose, mode: Skinning) -> Result<Vec<SkinnedVertex>> {
    deform_with(asset, pose, mode, &[])
}
/// [`deform`] with morph target `weights` (one per target; empty or all zero = none) added to
/// each vertex before skinning.
pub fn deform_with(
    asset: &Asset,
    pose: &Pose,
    mode: Skinning,
    weights: &[f32],
) -> Result<Vec<SkinnedVertex>> {
    let morphs = &asset.morphs;
    let morphed = !morphs.is_empty() && weights.iter().any(|w| *w != 0.);
    ensure!(
        !morphed || weights.len() == morphs.targets.len(),
        "morph weight count mismatch"
    );
    let globals = asset.skeleton.globals(pose)?;
    let matrices = globals
        .iter()
        .zip(&asset.skeleton.source.bones)
        .map(|(g, b)| *g * b.inverse_bind)
        .collect::<Vec<_>>();
    let normals = matrices
        .iter()
        .map(|m| m.inverse().transpose())
        .collect::<Vec<_>>();
    let rotations = matrices
        .iter()
        .map(|m| m.to_scale_rotation_translation().1.normalize())
        .collect::<Vec<_>>();
    let rigid = matrices.iter().all(|m| {
        m.to_scale_rotation_translation()
            .0
            .abs_diff_eq(Vec3::ONE, 1e-4)
    });
    let mut out = Vec::with_capacity(asset.vertices.len());
    for (i, v) in asset.vertices.iter().enumerate() {
        let (base, base_normal) = if morphed {
            morphs.apply(i, weights, v.position, v.normal)
        } else {
            (v.position, v.normal)
        };
        let sdef = v.sdef.filter(|_| rigid && matches!(mode, Skinning::Auto));
        let (position, normal) = if let Some(d) = sdef {
            let a = v.joints[0];
            let b = v.joints[1];
            let w = v.weights[0];
            let rw = d.r0 * w + d.r1 * (1. - w);
            let r0 = d.c + (d.r0 - rw) * 0.5;
            let r1 = d.c + (d.r1 - rw) * 0.5;
            let other = if rotations[a].dot(rotations[b]) < 0. {
                -rotations[b]
            } else {
                rotations[b]
            };
            let q = (rotations[a] * w + other * (1. - w)).normalize();
            (
                q * (base - d.c)
                    + matrices[a].transform_point3(r0) * w
                    + matrices[b].transform_point3(r1) * (1. - w),
                q * base_normal,
            )
        } else if rigid
            && (matches!(mode, Skinning::DualQuaternion)
                || (matches!(mode, Skinning::Auto) && v.dual_quaternion))
        {
            let mut real = Quat::from_xyzw(0., 0., 0., 0.);
            let mut dual = real;
            let reference = rotations[v.joints[0]];
            for (&j, w) in v.joints.iter().zip(v.weights) {
                if w == 0. {
                    continue;
                }
                let q = rotations[j];
                let t = matrices[j].w_axis.truncate();
                let d = Quat::from_xyzw(t.x, t.y, t.z, 0.) * q * 0.5;
                let sign = if q.dot(reference) < 0. { -1. } else { 1. };
                real = real + q * (w * sign);
                dual = dual + d * (w * sign);
            }
            let length = real.length();
            ensure!(length > 1e-6, "degenerate dual quaternion blend");
            let q = real / length;
            let d = dual / length;
            let d = d - q * q.dot(d);
            let translation = (d * q.conjugate()).xyz() * 2.;
            (q * base + translation, q * base_normal)
        } else {
            let mut p = Vec3::ZERO;
            let mut n = Vec3::ZERO;
            for (&j, w) in v.joints.iter().zip(v.weights) {
                if w > 0. {
                    p += matrices[j].transform_point3(base) * w;
                    n += normals[j].transform_vector3(base_normal) * w;
                }
            }
            (p, n)
        };
        ensure!(
            position.is_finite() && normal.is_finite(),
            "non-finite deformed vertex"
        );
        out.push(SkinnedVertex {
            position,
            normal: normal.try_normalize().unwrap_or(Vec3::Y),
            uv: v.uv,
        });
    }
    Ok(out)
}
