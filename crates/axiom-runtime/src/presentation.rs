//! Immutable renderer input. This module contains no gameplay state or genre types.
use axiom_core::{Camera, MeshPart, RenderSettings};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Instance {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
    pub material: [f32; 4],
}
impl Instance {
    pub fn new(model: Mat4, color: [f32; 3], emission: f32, grid: bool) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            color: [color[0], color[1], color[2], 1.0],
            material: [emission, f32::from(grid), 0.0, 0.0],
        }
    }
    pub fn part(parent: Mat4, p: &MeshPart) -> Self {
        Self::new(parent * p.transform.matrix(), p.color, p.emission, p.grid)
    }
}
#[derive(Clone)]
pub struct RenderView {
    pub id: String,
    pub eye: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub camera: Camera,
    pub instances: Vec<Instance>,
}
pub struct RenderFrame {
    pub tick: u64,
    pub settings: RenderSettings,
    pub instances: Vec<Instance>,
    pub views: Vec<RenderView>,
    pub meshes: Vec<MeshFrame>,
    pub shaders: std::sync::Arc<crate::shader::Prepared>,
    pub cube_materials: Vec<(std::ops::Range<u32>, String)>,
    pub vfx: crate::vfx::Frame,
    pub lights: Vec<crate::vfx::Light>,
    pub postfx: Vec<crate::postfx::Volume>,
}
pub struct MeshFrame {
    pub id: String,
    pub asset: std::sync::Arc<axiom_animation::Asset>,
    pub vertices: Vec<axiom_animation::SkinnedVertex>,
    pub model: Mat4,
    pub material: Option<String>,
}
