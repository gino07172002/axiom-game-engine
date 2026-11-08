//! Bounded procedural triangles; preparation is independent of animation and gameplay.
use crate::{
    presentation::{MeshFrame, RenderFrame},
    world::*,
};
use anyhow::{ensure, Result};
use axiom_animation::{AlphaMode, Asset, Bone, Material, Skeleton, SkinnedVertex, Trs, Vertex};
use axiom_core::*;
use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{any::Any, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Geometry {
    pub shape: String,
    pub segments: u32,
    pub rings: u32,
    pub barycentric_uv: bool,
    pub flat: bool,
    pub texture: Option<String>,
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            shape: "sphere".into(),
            segments: 32,
            rings: 20,
            barycentric_uv: false,
            flat: false,
            texture: None,
        }
    }
}
impl Geometry {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            ["sphere", "plane", "torus"].contains(&self.shape.as_str())
                && (3..=64).contains(&self.segments)
                && (2..=48).contains(&self.rings),
            "invalid procedural geometry"
        );
        ensure!(
            self.texture
                .as_ref()
                .is_none_or(|t| ["checker", "wood"].contains(&t.as_str())),
            "unknown procedural bitmap"
        );
        Ok(())
    }
    pub fn build(&self) -> Result<Asset> {
        self.validate()?;
        let point = |u: f32, v: f32| -> (Vec3, Vec3) {
            let a = u * std::f32::consts::TAU;
            let b = v * std::f32::consts::PI;
            match self.shape.as_str() {
                "plane" => (Vec3::new((u - 0.5) * 2.0, 0.0, (v - 0.5) * 2.0), Vec3::Y),
                "torus" => {
                    let b = v * std::f32::consts::TAU;
                    let n = Vec3::new(a.cos() * b.cos(), b.sin(), a.sin() * b.cos());
                    (Vec3::new(a.cos() * 0.72, 0.0, a.sin() * 0.72) + n * 0.28, n)
                }
                _ => {
                    let n = Vec3::new(a.sin() * b.sin(), b.cos(), a.cos() * b.sin());
                    (n, n)
                }
            }
        };
        let mut vertices = vec![];
        let mut indices = vec![];
        for y in 0..self.rings {
            for x in 0..self.segments {
                let uv = [
                    Vec2::new(
                        x as f32 / self.segments as f32,
                        y as f32 / self.rings as f32,
                    ),
                    Vec2::new(
                        (x + 1) as f32 / self.segments as f32,
                        y as f32 / self.rings as f32,
                    ),
                    Vec2::new(
                        (x + 1) as f32 / self.segments as f32,
                        (y + 1) as f32 / self.rings as f32,
                    ),
                    Vec2::new(
                        x as f32 / self.segments as f32,
                        (y + 1) as f32 / self.rings as f32,
                    ),
                ];
                for tri in [[0, 1, 2], [0, 2, 3]] {
                    let p = tri.map(|i| point(uv[i].x, uv[i].y));
                    let cross = (p[1].0 - p[0].0).cross(p[2].0 - p[0].0);
                    if cross.length_squared() < 1e-12 {
                        continue;
                    }
                    // Parameterization orientation differs for an XZ sheet and a sphere.
                    let reverse = cross.dot(p[0].1) < 0.0;
                    for k in if reverse { [0, 2, 1] } else { [0, 1, 2] } {
                        let (position, normal) = p[k];
                        indices.push(vertices.len() as u32);
                        vertices.push(Vertex {
                            position,
                            normal: if self.flat {
                                cross.normalize() * if reverse { -1.0 } else { 1.0 }
                            } else {
                                normal
                            },
                            uv: if self.barycentric_uv {
                                [Vec2::X, Vec2::Y, Vec2::ZERO][k]
                            } else {
                                uv[tri[k]]
                            },
                            joints: [0; 4],
                            weights: [1.0, 0.0, 0.0, 0.0],
                            sdef: None,
                            dual_quaternion: false,
                        });
                    }
                }
            }
        }
        let count = indices.len() as u32;
        let textures = if let Some(kind) = &self.texture {
            let mut rgba = vec![];
            for y in 0..64 {
                for x in 0..64 {
                    let v = if kind == "checker" {
                        if ((x / 8) + (y / 8)) % 2 == 0 {
                            1.0
                        } else {
                            0.08
                        }
                    } else {
                        0.5 + 0.5 * (x as f32 * 0.8 + (y as f32 * 0.2).sin() * 2.0).sin()
                    };
                    let color = if kind == "wood" {
                        [0.13 + v * 0.6, 0.035 + v * 0.28, 0.012 + v * 0.07]
                    } else {
                        [v; 3]
                    };
                    rgba.extend(color.map(|c| (c * 255.0) as u8));
                    rgba.push(255);
                }
            }
            vec![axiom_animation::Texture {
                width: 64,
                height: 64,
                rgba,
            }]
        } else {
            vec![]
        };
        let asset = Asset {
            key: format!("procedural/{}", serde_json::to_string(self)?),
            skeleton: Skeleton {
                bones: vec![Bone {
                    id: "root".into(),
                    aliases: vec![],
                    parent: None,
                    rest: Trs::default(),
                    inverse_bind: Mat4::IDENTITY,
                }],
            }
            .compile()?,
            vertices,
            indices,
            materials: vec![Material {
                id: "surface".into(),
                first_index: 0,
                index_count: count,
                color: [1.0; 4],
                texture: self.texture.as_ref().map(|_| 0),
                double_sided: false,
                alpha: AlphaMode::Opaque,
            }],
            textures,
            clips: vec![],
            diagnostics: vec![],
            humanoid: None,
        };
        asset.validate()?;
        Ok(asset)
    }
}
struct GeometrySystem {
    meshes: Vec<(usize, Arc<Asset>)>,
}
impl RuntimeSystem for GeometrySystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn checkpoint_supported(&self) -> bool {
        true
    }
    fn step(&mut self, _state: &mut RuntimeState, _input: &ActionMap) {}
    fn extract(&self, state: &RuntimeState, frame: &mut RenderFrame) {
        let matrices = state.matrices();
        for (slot, asset) in &self.meshes {
            if !state.compiled.enabled[*slot] {
                continue;
            }
            frame.meshes.push(MeshFrame {
                id: state.compiled.entities[*slot].id.clone(),
                asset: asset.clone(),
                vertices: asset
                    .vertices
                    .iter()
                    .map(|v| SkinnedVertex {
                        position: v.position,
                        normal: v.normal,
                        uv: v.uv,
                    })
                    .collect(),
                model: matrices[*slot],
                material: state.compiled.entities[*slot]
                    .material
                    .as_ref()
                    .map(|m| m.id.clone()),
            });
        }
    }
}
pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.component("render.geometry","render",json!({"shape":"sphere|plane|torus","segments":"3..64","rings":"2..48","barycentric_uv":"bool; real triangle edges for wire shading","flat":"bool"}),|v|serde_json::from_value::<Geometry>(v.clone())?.validate());
    r.authoring.systems.insert(
        "geometry.prepare".into(),
        SystemDescriptor {
            id: "geometry.prepare".into(),
            capability: "render".into(),
            phase: 5,
            reads: vec!["render.geometry".into()],
            writes: vec![],
            after: vec![],
        },
    );
    r.factories.insert("geometry.prepare".into(), |state| {
        let mut meshes = vec![];
        let mut total = 0;
        for (i, e) in state.compiled.entities.iter().enumerate() {
            if let Some(v) = e.components.get("render.geometry") {
                ensure!(
                    e.transform.is_some()
                        && e.mesh.is_none()
                        && !e.components.contains_key("animation.player"),
                    "procedural geometry requires one transform and exclusive mesh ownership"
                );
                let asset = serde_json::from_value::<Geometry>(v.clone())?.build()?;
                total += asset.vertices.len();
                ensure!(
                    total <= 500_000,
                    "procedural scene geometry budget exceeded"
                );
                meshes.push((i, Arc::new(asset)));
            }
        }
        Ok(Box::new(GeometrySystem { meshes }))
    });
}
