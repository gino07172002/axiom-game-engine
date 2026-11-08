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
impl RenderView {
    /// World → screen for this view inside `screen` (respects the camera viewport).
    /// Returns None behind the camera or outside the view's viewport.
    pub fn project(&self, point: Vec3, screen: [f32; 4]) -> Option<[f32; 2]> {
        let [sx, sy, sw, sh] = screen;
        let [x, y, w, h] = self.camera.viewport;
        let (vx, vy, vw, vh) = (sx + x * sw, sy + y * sh, w * sw, h * sh);
        let aspect = vw / vh.max(1.0);
        let projection = match self.camera.projection {
            axiom_core::Projection::Perspective { fov_degrees } => Mat4::perspective_rh(
                fov_degrees.to_radians(),
                aspect,
                self.camera.near,
                self.camera.far,
            ),
            axiom_core::Projection::Orthographic { height } => Mat4::orthographic_rh(
                -height * aspect * 0.5,
                height * aspect * 0.5,
                -height * 0.5,
                height * 0.5,
                self.camera.near,
                self.camera.far,
            ),
        };
        let clip = projection
            * Mat4::look_at_rh(self.eye, self.eye + self.forward, self.up)
            * point.extend(1.0);
        if clip.w <= 0.0 || !clip.is_finite() {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
            return None;
        }
        Some([vx + (ndc.x + 1.0) * vw * 0.5, vy + (1.0 - ndc.y) * vh * 0.5])
    }
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
    /// VFX particle lights (point lights, fixed radius); packed after `scene_lights`.
    pub lights: Vec<crate::vfx::Light>,
    /// Enabled `render.light` entities in authored entity order.
    pub scene_lights: Vec<SceneLight>,
    /// `render.environment` (default = legacy ambient and fog).
    pub environment: axiom_core::RenderEnvironment,
    pub postfx: Vec<crate::postfx::Volume>,
}
/// One authored light in world space. `direction` is where it shines (entity -Z),
/// `right` the entity +X axis (orients area rectangles).
#[derive(Clone, Debug)]
pub struct SceneLight {
    pub id: String,
    pub light: axiom_core::RenderLight,
    pub position: Vec3,
    pub direction: Vec3,
    pub right: Vec3,
}
pub struct MeshFrame {
    pub id: String,
    pub asset: std::sync::Arc<axiom_animation::Asset>,
    pub vertices: Vec<axiom_animation::SkinnedVertex>,
    pub model: Mat4,
    pub material: Option<String>,
}
