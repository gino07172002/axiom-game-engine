//! Signed axis bases include reflection. Rotations use B R B^-1, never quaternion-component guesses.
use crate::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Coordinates {
    pub basis: [[f32; 3]; 3],
    pub meters_per_unit: f32,
}

pub(crate) fn scale_in_basis(basis: Mat4, scale: Vec3) -> Result<Vec3> {
    let transformed = basis * Mat4::from_scale(scale) * basis.transpose();
    let diagonal = Vec3::new(
        transformed.x_axis.x,
        transformed.y_axis.y,
        transformed.z_axis.z,
    );
    ensure!(transformed.abs_diff_eq(Mat4::from_scale(diagonal), 0.0001), "coordinate conversion would shear an animated scale; use a signed axis basis or bake uniform bone scales");
    Ok(diagonal)
}
impl Default for Coordinates {
    fn default() -> Self {
        Self::preset("gltf").unwrap()
    }
}
impl Coordinates {
    pub fn preset(name: &str) -> Result<Self> {
        let basis = match name {
            "axiom" | "gltf" => [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            "unity" | "mmd" => [[1., 0., 0.], [0., 1., 0.], [0., 0., -1.]],
            "blender" => [[1., 0., 0.], [0., 0., -1.], [0., 1., 0.]],
            "unreal" => [[0., 0., -1.], [1., 0., 0.], [0., 1., 0.]],
            _ => anyhow::bail!("unknown coordinate preset {name}"),
        };
        Ok(Self {
            basis,
            meters_per_unit: if name == "unreal" { 0.01 } else { 1.0 },
        })
    }
    pub fn matrix(self) -> Result<Mat4> {
        let m = Mat4::from_cols(
            Vec3::from(self.basis[0]).extend(0.),
            Vec3::from(self.basis[1]).extend(0.),
            Vec3::from(self.basis[2]).extend(0.),
            glam::Vec4::W,
        );
        ensure!(
            self.meters_per_unit.is_finite()
                && self.meters_per_unit > 0.000001
                && self.meters_per_unit <= 1000.,
            "invalid coordinate unit scale"
        );
        ensure!(
            m.is_finite() && (m.transpose() * m).abs_diff_eq(Mat4::IDENTITY, 0.0001),
            "coordinate basis must be orthonormal"
        );
        Ok(m)
    }
    pub fn point(self, p: Vec3) -> Result<Vec3> {
        Ok(self.matrix()?.transform_vector3(p) * self.meters_per_unit)
    }
    pub fn transform(self, m: Mat4) -> Result<Mat4> {
        let b = self.matrix()? * Mat4::from_scale(Vec3::splat(self.meters_per_unit));
        Ok(b * m * b.inverse())
    }
    pub fn rotation(self, q: Quat) -> Result<Quat> {
        Ok(Trs::from_matrix(self.transform(Mat4::from_quat(q))?)?.rotation)
    }
    pub fn reflected(self) -> Result<bool> {
        Ok(self.matrix()?.determinant() < 0.)
    }
}
