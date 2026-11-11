//! Bounded skeletal assets, poses and constraints. No renderer, window or genre dependencies.
pub mod authoring;
pub mod choreography;
pub mod clip;
pub mod controller;
pub mod coordinates;
pub mod humanoid;
pub mod ik;
pub mod import;
pub mod skin;
pub mod twist;
use anyhow::{ensure, Context, Result};
use glam::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_BONES: usize = 1024;
pub const MAX_VERTICES: usize = 500_000;
pub const MAX_INDICES: usize = 3_000_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Trs {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}
impl Default for Trs {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}
impl Trs {
    pub fn matrix(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
    pub fn from_matrix(m: Mat4) -> Result<Self> {
        ensure!(
            m.is_finite() && m.determinant().abs() > 1e-10,
            "singular/non-finite bone matrix"
        );
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        let result = Self {
            scale,
            rotation: rotation.normalize(),
            translation,
        };
        result.validate()?;
        ensure!(
            result.matrix().abs_diff_eq(m, 2e-3),
            "sheared transforms must be baked to TRS before import"
        );
        Ok(result)
    }
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.translation.is_finite()
                && self.scale.is_finite()
                && self.scale.abs().min_element() > 1e-6
                && self.rotation.is_finite()
                && (self.rotation.length_squared() - 1.0).abs() < 0.002,
            "invalid bone TRS"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bone {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub parent: Option<usize>,
    pub rest: Trs,
    pub inverse_bind: Mat4,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}
#[derive(Clone, Debug)]
pub struct CompiledSkeleton {
    pub source: Skeleton,
    pub indices: BTreeMap<String, usize>,
    pub order: Vec<usize>,
    pub rest_global: Vec<Mat4>,
}
impl Skeleton {
    pub fn compile(&self) -> Result<CompiledSkeleton> {
        ensure!(
            !self.bones.is_empty() && self.bones.len() <= MAX_BONES,
            "bone count must be 1..{MAX_BONES}"
        );
        let mut indices = BTreeMap::new();
        for (i, b) in self.bones.iter().enumerate() {
            ensure!(
                !b.id.is_empty() && b.id.len() <= 256 && indices.insert(b.id.clone(), i).is_none(),
                "duplicate/invalid bone ID: {}",
                b.id
            );
            b.rest.validate()?;
            ensure!(
                b.inverse_bind.is_finite() && b.inverse_bind.determinant().abs() > 1e-10,
                "invalid inverse bind: {}",
                b.id
            );
            ensure!(
                b.parent.is_none_or(|p| p < self.bones.len() && p != i),
                "invalid bone parent: {}",
                b.id
            );
        }
        fn visit(i: usize, b: &[Bone], marks: &mut [u8], order: &mut Vec<usize>) -> Result<()> {
            ensure!(marks[i] != 1, "bone parent cycle: {}", b[i].id);
            if marks[i] == 2 {
                return Ok(());
            }
            marks[i] = 1;
            if let Some(p) = b[i].parent {
                visit(p, b, marks, order)?
            }
            marks[i] = 2;
            order.push(i);
            Ok(())
        }
        let mut order = vec![];
        let mut marks = vec![0; self.bones.len()];
        for i in 0..self.bones.len() {
            visit(i, &self.bones, &mut marks, &mut order)?
        }
        let mut rest_global = vec![Mat4::IDENTITY; self.bones.len()];
        for &i in &order {
            rest_global[i] = self.bones[i]
                .parent
                .map_or(Mat4::IDENTITY, |p| rest_global[p])
                * self.bones[i].rest.matrix();
        }
        Ok(CompiledSkeleton {
            source: self.clone(),
            indices,
            order,
            rest_global,
        })
    }
}
impl CompiledSkeleton {
    pub fn bone(&self, id: &str) -> Result<usize> {
        self.indices
            .get(id)
            .copied()
            .with_context(|| format!("bone not found: {id}"))
    }
    pub fn rest_pose(&self) -> Pose {
        Pose {
            local: self.source.bones.iter().map(|b| b.rest).collect(),
        }
    }
    /// Writes the rest pose into `out` (its buffer is reused once sized).
    pub fn rest_into(&self, out: &mut Pose) {
        out.local.clear();
        out.local.extend(self.source.bones.iter().map(|b| b.rest));
    }
    /// [`Self::globals`] into a reused buffer.
    pub fn globals_into(&self, pose: &Pose, out: &mut Vec<Mat4>) -> Result<()> {
        ensure!(
            pose.local.len() == self.source.bones.len(),
            "pose/skeleton size mismatch"
        );
        out.clear();
        out.resize(pose.local.len(), Mat4::IDENTITY);
        for &i in &self.order {
            pose.local[i].validate()?;
            out[i] = self.source.bones[i]
                .parent
                .map_or(Mat4::IDENTITY, |p| out[p])
                * pose.local[i].matrix();
        }
        Ok(())
    }
    pub fn globals(&self, pose: &Pose) -> Result<Vec<Mat4>> {
        ensure!(
            pose.local.len() == self.source.bones.len(),
            "pose/skeleton size mismatch"
        );
        let mut out = vec![Mat4::IDENTITY; pose.local.len()];
        for &i in &self.order {
            pose.local[i].validate()?;
            out[i] = self.source.bones[i]
                .parent
                .map_or(Mat4::IDENTITY, |p| out[p])
                * pose.local[i].matrix();
        }
        Ok(out)
    }
    pub fn rebind(&mut self) {
        for (i, b) in self.source.bones.iter_mut().enumerate() {
            b.inverse_bind = self.rest_global[i].inverse()
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pose {
    pub local: Vec<Trs>,
}
impl Pose {
    /// In place: `self = blend(self, other, weight)` (same math as [`Self::blend`]).
    pub fn blend_toward(&mut self, other: &Self, weight: f32) -> Result<()> {
        ensure!(
            self.local.len() == other.local.len()
                && weight.is_finite()
                && (0.0..=1.0).contains(&weight),
            "invalid pose blend"
        );
        if weight == 0.0 {
            return Ok(());
        }
        for (a, b) in self.local.iter_mut().zip(&other.local) {
            if weight == 1.0 {
                *a = *b;
            } else {
                *a = Trs {
                    translation: a.translation.lerp(b.translation, weight),
                    scale: a.scale.lerp(b.scale, weight),
                    rotation: a.rotation.slerp(b.rotation, weight).normalize(),
                };
            }
        }
        Ok(())
    }
    /// In place: `self = blend(from, self, weight)` (a cross-fade out of `from`).
    pub fn blend_from(&mut self, from: &Self, weight: f32) -> Result<()> {
        ensure!(
            self.local.len() == from.local.len()
                && weight.is_finite()
                && (0.0..=1.0).contains(&weight),
            "invalid pose blend"
        );
        if weight == 1.0 {
            return Ok(());
        }
        for (b, a) in self.local.iter_mut().zip(&from.local) {
            if weight == 0.0 {
                *b = *a;
            } else {
                *b = Trs {
                    translation: a.translation.lerp(b.translation, weight),
                    scale: a.scale.lerp(b.scale, weight),
                    rotation: a.rotation.slerp(b.rotation, weight).normalize(),
                };
            }
        }
        Ok(())
    }
    pub fn blend(a: &Self, b: &Self, weight: f32) -> Result<Self> {
        ensure!(
            a.local.len() == b.local.len() && weight.is_finite() && (0.0..=1.0).contains(&weight),
            "invalid pose blend"
        );
        if weight == 0.0 {
            return Ok(a.clone());
        }
        if weight == 1.0 {
            return Ok(b.clone());
        }
        Ok(Self {
            local: a
                .local
                .iter()
                .zip(&b.local)
                .map(|(a, b)| Trs {
                    translation: a.translation.lerp(b.translation, weight),
                    scale: a.scale.lerp(b.scale, weight),
                    rotation: a.rotation.slerp(b.rotation, weight).normalize(),
                })
                .collect(),
        })
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Sdef {
    pub c: Vec3,
    pub r0: Vec3,
    pub r1: Vec3,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub joints: [usize; 4],
    pub weights: [f32; 4],
    #[serde(default)]
    pub sdef: Option<Sdef>,
    #[serde(default)]
    pub dual_quaternion: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct SkinnedVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
}
#[derive(Clone, Debug)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub first_index: u32,
    pub index_count: u32,
    pub color: [f32; 4],
    pub texture: Option<usize>,
    pub double_sided: bool,
    pub alpha: AlphaMode,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    Opaque,
    Mask(f32),
    Blend,
}
#[derive(Clone, Debug)]
pub struct Asset {
    pub key: String,
    pub skeleton: CompiledSkeleton,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub clips: Vec<clip::Clip>,
    pub diagnostics: Vec<String>,
    pub humanoid: Option<humanoid::Profile>,
}
impl Asset {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.vertices.len() <= MAX_VERTICES
                && self.indices.len() <= MAX_INDICES
                && self.indices.len().is_multiple_of(3)
                && self.materials.len() <= 256,
            "mesh budget/topology invalid"
        );
        for v in &self.vertices {
            ensure!(
                v.position.is_finite() && v.normal.is_finite() && v.uv.is_finite(),
                "non-finite vertex"
            );
            let sum: f32 = v.weights.iter().sum();
            ensure!(
                (sum - 1.0).abs() < 0.001 && v.weights.iter().all(|w| w.is_finite() && *w >= 0.0),
                "invalid skin weights"
            );
            ensure!(
                v.joints
                    .iter()
                    .all(|j| *j < self.skeleton.source.bones.len()),
                "skin joint out of range"
            );
            ensure!(
                v.sdef.is_none_or(|d| d.c.is_finite()
                    && d.r0.is_finite()
                    && d.r1.is_finite()
                    && v.weights[2] == 0.0
                    && v.weights[3] == 0.0),
                "invalid SDEF vertex"
            );
        }
        ensure!(
            self.indices
                .iter()
                .all(|i| (*i as usize) < self.vertices.len()),
            "triangle index out of range"
        );
        let mut expected = 0;
        for m in &self.materials {
            ensure!(
                m.first_index as usize == expected && m.index_count.is_multiple_of(3),
                "non-contiguous material range"
            );
            expected += m.index_count as usize;
            ensure!(
                m.color.iter().all(|v| v.is_finite())
                    && m.texture.is_none_or(|i| i < self.textures.len()),
                "invalid material"
            );
        }
        ensure!(
            expected == self.indices.len(),
            "material index coverage mismatch"
        );
        let mut clip_ids = std::collections::BTreeSet::new();
        for clip in &self.clips {
            ensure!(
                clip_ids.insert(&clip.id),
                "duplicate asset clip ID: {}",
                clip.id
            );
            clip.compile(&self.skeleton)?;
        }
        Ok(())
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"key":self.key,"bones":self.skeleton.source.bones.len(),"vertices":self.vertices.len(),"triangles":self.indices.len()/3,"materials":self.materials.len(),"textures":self.textures.len(),"clips":self.clips.iter().map(|c|serde_json::json!({"id":c.id,"duration":c.duration,"tracks":c.tracks.len()})).collect::<Vec<_>>(),"sdef_vertices":self.vertices.iter().filter(|v|v.sdef.is_some()).count(),"qdef_vertices":self.vertices.iter().filter(|v|v.dual_quaternion).count(),"diagnostics":self.diagnostics})
    }
}
