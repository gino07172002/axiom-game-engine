//! Textured indexed triangles from immutable mesh snapshots; independent of animation/gameplay.
use crate::presentation::MeshFrame;
use anyhow::{ensure, Result};
use axiom_animation::AlphaMode;
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeMap, BTreeSet};
use wgpu::util::DeviceExt;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}
pub(crate) fn mesh_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Draw {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    color: [f32; 4],
    params: [f32; 4],
    /// Stylized look (mesh.wgsl documents each field; all zero = none).
    toon: [f32; 4],
    style: [f32; 4],
    face: [f32; 4],
    decal: [[f32; 4]; 3],
    colors: [u32; 4],
}
/// Bytes between two draws' uniforms (dynamic offsets are multiples of 256).
const DRAW_STRIDE: usize = 512;
const _: () = assert!(std::mem::size_of::<Draw>() <= DRAW_STRIDE);
/// Mesh primitive draws per frame (the uniform buffer holds this many strides).
const MAX_DRAWS: usize = 8192;
struct Material {
    bind: wgpu::BindGroup,
    range: std::ops::Range<u32>,
    alpha: AlphaMode,
    double_sided: bool,
    color: [f32; 4],
}
struct Mesh {
    vertices: wgpu::Buffer,
    /// Vertex data last written to `vertices`; an identical shared buffer is not re-uploaded.
    uploaded: Option<std::sync::Arc<Vec<axiom_animation::SkinnedVertex>>>,
    indices: wgpu::Buffer,
    materials: Vec<Material>,
    /// The decal image bound into materials (rebuilt when the entity's decal changes).
    decal: Option<std::sync::Arc<crate::mesh_style::DecalImage>>,
}
pub struct MeshRenderer {
    camera: wgpu::BindGroup,
    camera_layout: wgpu::BindGroupLayout,
    /// Uniform-only camera group and depth-only pipeline of the sun shadow pass.
    shadow_camera: wgpu::BindGroup,
    shadow_pipeline: wgpu::RenderPipeline,
    material_layout: wgpu::BindGroupLayout,
    draws: wgpu::Buffer,
    pipelines: Vec<wgpu::RenderPipeline>,
    /// `render.outline` inverted hull (front faces culled).
    outline_pipeline: wgpu::RenderPipeline,
    /// Clamped sampler of decal images (no repeat bleeding at the image border).
    decal_sampler: wgpu::Sampler,
    meshes: BTreeMap<String, Mesh>,
    active: Vec<ActiveMesh>,
}
struct ActiveMesh {
    key: String,
    first: usize,
    material: Option<String>,
    /// The style when some primitive has a `render.outline` width.
    outline: Option<std::sync::Arc<crate::mesh_style::MeshStyle>>,
}
impl MeshRenderer {
    pub fn layouts(&self) -> [&wgpu::BindGroupLayout; 2] {
        [&self.camera_layout, &self.material_layout]
    }
    /// `shadow` / `shadow_sampler`: the sun shadow map the meshes sample (bindings 1 and 2 of
    /// the camera group, like the cube pipeline) and cast into (`draw_shadow`).
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        globals: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
    ) -> Self {
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: None,
            },
            count: None,
        };
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("triangle camera"),
            entries: &[
                uniform_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let globals_binding = || {
            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: globals,
                offset: 0,
                size: std::num::NonZeroU64::new(crate::render::GLOBALS_SIZE),
            })
        };
        let camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("triangle camera"),
            layout: &camera_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(shadow),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
            ],
        });
        // The depth pass writes the shadow map, so its camera group must not also bind it.
        let shadow_camera_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("triangle shadow camera"),
                entries: &[uniform_entry],
            });
        let shadow_camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("triangle shadow camera"),
            layout: &shadow_camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_binding(),
            }],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("triangle material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("textured triangles"),
            bind_group_layouts: &[&camera_layout, &material_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("triangle shader"),
            source: wgpu::ShaderSource::Wgsl(crate::lighting::MESH_WGSL.into()),
        });
        let mut pipelines = vec![];
        for blend in [false, true] {
            for double_sided in [false, true] {
                pipelines.push(
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("textured triangle pipeline"),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: &shader,
                            entry_point: Some("vertex_main"),
                            compilation_options: Default::default(),
                            buffers: &[mesh_vertex_layout()],
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &shader,
                            entry_point: Some("fragment_main"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend: if blend {
                                    Some(wgpu::BlendState::ALPHA_BLENDING)
                                } else {
                                    None
                                },
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        primitive: wgpu::PrimitiveState {
                            cull_mode: if double_sided {
                                None
                            } else {
                                Some(wgpu::Face::Back)
                            },
                            ..Default::default()
                        },
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: !blend,
                            depth_compare: wgpu::CompareFunction::LessEqual,
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: Default::default(),
                        multiview: None,
                        cache: None,
                    }),
                );
            }
        }
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("triangle shadow"),
            bind_group_layouts: &[&shadow_camera_layout, &material_layout],
            push_constant_ranges: &[],
        });
        // Same depth bias as the cube shadow pipeline; no culling so open / double-sided
        // imported meshes still cast.
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("triangle shadow pipeline"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_mesh_shadow"),
                compilation_options: Default::default(),
                buffers: &[mesh_vertex_layout()],
            },
            fragment: None,
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        // Same layout and camera group as the lit pipelines; position-only vertex stage.
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh outline pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_outline"),
                compilation_options: Default::default(),
                buffers: &[mesh_vertex_layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_outline"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Front),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let decal_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("decal sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let draws = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("triangle draws"),
            size: (MAX_DRAWS * DRAW_STRIDE) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            camera,
            camera_layout,
            shadow_camera,
            shadow_pipeline,
            material_layout,
            draws,
            pipelines,
            outline_pipeline,
            decal_sampler,
            meshes: BTreeMap::new(),
            active: vec![],
        }
    }
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frames: &[MeshFrame],
    ) -> Result<()> {
        let mut active = vec![];
        let mut keep = BTreeSet::new();
        let mut uniforms = vec![];
        let mut draw_index = 0;
        for frame in frames {
            if frame.vertices.is_empty() || frame.asset.indices.is_empty() {
                continue;
            }
            ensure!(
                frame.vertices.len() == frame.asset.vertices.len(),
                "mesh snapshot vertex mismatch"
            );
            let key = format!("{}:{}", frame.id, frame.asset.key);
            keep.insert(key.clone());
            let decal = frame.style.as_ref().and_then(|s| s.decal.as_ref());
            let image = decal.map(|d| d.image.clone());
            if self.meshes.get(&key).is_some_and(|m| {
                m.decal.as_ref().map(std::sync::Arc::as_ptr)
                    != image.as_ref().map(std::sync::Arc::as_ptr)
            }) {
                // The entity's decal image changed (edit or file reload): rebind its materials.
                self.meshes.remove(&key);
            }
            let mesh = self.meshes.entry(key.clone()).or_insert_with(|| {
                let asset = &frame.asset;
                let vertices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("deformed vertices"),
                    size: (frame.vertices.len() * 32) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh indices"),
                    contents: bytemuck::cast_slice(&asset.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                let mut images = vec![];
                for t in &asset.textures {
                    images.push(upload_texture(device, queue, t.width, t.height, &t.rgba));
                }
                let white = upload_texture(device, queue, 1, 1, &[255, 255, 255, 255]);
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("material sampler"),
                    address_mode_u: wgpu::AddressMode::Repeat,
                    address_mode_v: wgpu::AddressMode::Repeat,
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                });
                let overlay = image
                    .as_ref()
                    .map(|i| upload_texture(device, queue, i.width, i.height, &i.rgba));
                let materials = asset
                    .materials
                    .iter()
                    .map(|m| {
                        let texture = m.texture.map_or(&white, |i| &images[i]);
                        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("mesh material"),
                            layout: &self.material_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                        buffer: &self.draws,
                                        offset: 0,
                                        size: std::num::NonZeroU64::new(
                                            std::mem::size_of::<Draw>() as u64,
                                        ),
                                    }),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(texture),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(
                                        overlay.as_ref().unwrap_or(&white),
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: wgpu::BindingResource::Sampler(&self.decal_sampler),
                                },
                            ],
                        });
                        Material {
                            bind,
                            range: m.first_index..m.first_index + m.index_count,
                            alpha: m.alpha,
                            double_sided: m.double_sided,
                            color: m.color,
                        }
                    })
                    .collect();
                Mesh {
                    vertices,
                    uploaded: None,
                    indices,
                    materials,
                    decal: image,
                }
            });
            if !mesh
                .uploaded
                .as_ref()
                .is_some_and(|last| std::sync::Arc::ptr_eq(last, &frame.vertices))
            {
                let vertices = frame
                    .vertices
                    .iter()
                    .map(|v| Vertex {
                        position: v.position.to_array(),
                        normal: v.normal.to_array(),
                        uv: v.uv.to_array(),
                    })
                    .collect::<Vec<_>>();
                queue.write_buffer(&mesh.vertices, 0, bytemuck::cast_slice(&vertices));
                mesh.uploaded = Some(frame.vertices.clone());
            }
            let first = draw_index;
            let normal = frame.model.inverse().transpose();
            ensure!(normal.is_finite(), "singular mesh model transform");
            let face =
                frame
                    .style
                    .as_ref()
                    .and_then(|s| s.face)
                    .map_or(glam::Vec3::ZERO, |(_, axis)| {
                        frame.style_space[0]
                            .transform_vector3(axis)
                            .try_normalize()
                            .unwrap_or(glam::Vec3::ZERO)
                    });
            let decal_rows = decal.map(|d| {
                crate::mesh_style::decal_rows(&d.mapping, frame.style_space[1])
                    .map(|r| r.to_array())
            });
            let mut outline = false;
            for (i, m) in mesh.materials.iter().enumerate() {
                ensure!(draw_index < MAX_DRAWS, "triangle draw budget exceeded");
                let [metallic, smooth] = crate::presentation::surface_params(frame.surface);
                let params = match m.alpha {
                    AlphaMode::Opaque => [0., 0., metallic, smooth],
                    AlphaMode::Mask(c) => [1., c, metallic, smooth],
                    AlphaMode::Blend => [2., 0., metallic, smooth],
                };
                let style = frame
                    .style
                    .as_ref()
                    .and_then(|s| s.primitives.get(i))
                    .copied()
                    .unwrap_or_default();
                outline |= style.outline != 0.0;
                let (decal_mode, decal_opacity, rows) = match (decal, decal_rows) {
                    (Some(d), Some(rows)) if style.decal => {
                        let projected = matches!(d.mapping, crate::mesh_style::Mapping::Project(_));
                        (d.blend + if projected { 4.0 } else { 0.0 }, d.opacity, rows)
                    }
                    _ => (0.0, 0.0, [[0.0; 4]; 3]),
                };
                let draw = Draw {
                    model: frame.model.to_cols_array_2d(),
                    normal: normal.to_cols_array_2d(),
                    color: m.color,
                    params,
                    toon: style.toon,
                    style: [
                        style.rim_power,
                        style.face_blend,
                        style.outline,
                        decal_opacity,
                    ],
                    face: [face.x, face.y, face.z, decal_mode],
                    decal: rows,
                    colors: style.colors,
                };
                uniforms.resize((draw_index + 1) * DRAW_STRIDE, 0);
                let bytes = bytemuck::bytes_of(&draw);
                uniforms[draw_index * DRAW_STRIDE..draw_index * DRAW_STRIDE + bytes.len()]
                    .copy_from_slice(bytes);
                draw_index += 1;
            }
            active.push(ActiveMesh {
                key,
                first,
                material: frame.material.clone(),
                outline: frame.style.clone().filter(|_| outline),
            });
        }
        if !uniforms.is_empty() {
            queue.write_buffer(&self.draws, 0, &uniforms);
        }
        self.meshes.retain(|k, _| keep.contains(k));
        self.active = active;
        Ok(())
    }
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        camera: u32,
        shaders: &crate::shader_gpu::ShaderGpu,
        blend: bool,
    ) -> (u32, u64) {
        let (mut calls, mut triangles) = (0, 0);
        if self.active.is_empty() {
            return (calls, triangles);
        }
        pass.set_bind_group(0, &self.camera, &[camera * crate::render::GLOBALS_STRIDE]);
        for active in &self.active {
            let mesh = &self.meshes[&active.key];
            let surface = active
                .material
                .as_ref()
                .and_then(|id| shaders.materials.get(id));
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            for (i, m) in mesh.materials.iter().enumerate() {
                use crate::shader::Alpha;
                let is_blended = match surface.map(|s| s.alpha).unwrap_or(Alpha::Inherit) {
                    Alpha::Inherit => matches!(m.alpha, AlphaMode::Blend),
                    Alpha::Blend | Alpha::Additive | Alpha::Multiply => true,
                    _ => false,
                };
                if is_blended != blend || m.range.is_empty() {
                    continue;
                }
                if let Some(surface) = surface {
                    pass.set_pipeline(surface.pipeline(
                        true,
                        blend,
                        surface.double_sided.unwrap_or(m.double_sided),
                    ));
                    pass.set_bind_group(2, &shaders.bind, &[surface.offset]);
                } else {
                    let pipeline = usize::from(blend) * 2 + usize::from(m.double_sided);
                    pass.set_pipeline(&self.pipelines[pipeline]);
                }
                pass.set_bind_group(1, &m.bind, &[((active.first + i) * DRAW_STRIDE) as u32]);
                pass.draw_indexed(m.range.clone(), 0, 0..1);
                calls += 1;
                triangles += m.range.len() as u64 / 3;
            }
        }
        if !blend {
            let (c, t) = self.draw_outlines(pass, shaders);
            calls += c;
            triangles += t;
        }
        (calls, triangles)
    }
    /// `render.outline` hulls of the opaque primitives (after any surface material's alpha,
    /// the rule of [casts_shadow]), after the opaque meshes so they fill only around them.
    fn draw_outlines(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        shaders: &crate::shader_gpu::ShaderGpu,
    ) -> (u32, u64) {
        let (mut calls, mut triangles) = (0, 0);
        let mut bound = false;
        for active in &self.active {
            let Some(style) = &active.outline else {
                continue;
            };
            let mesh = &self.meshes[&active.key];
            let surface = active
                .material
                .as_ref()
                .and_then(|id| shaders.materials.get(id));
            if !bound {
                pass.set_pipeline(&self.outline_pipeline);
                bound = true;
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            for (i, m) in mesh.materials.iter().enumerate() {
                let width = style.primitives.get(i).map_or(0.0, |p| p.outline);
                let double_sided = surface
                    .and_then(|s| s.double_sided)
                    .unwrap_or(m.double_sided);
                if !outlined(width, surface.map(|s| s.alpha), m.alpha, double_sided)
                    || m.range.is_empty()
                {
                    continue;
                }
                pass.set_bind_group(1, &m.bind, &[((active.first + i) * DRAW_STRIDE) as u32]);
                pass.draw_indexed(m.range.clone(), 0, 0..1);
                calls += 1;
                triangles += m.range.len() as u64 / 3;
            }
        }
        (calls, triangles)
    }
    /// Draws the opaque mesh primitives (after any surface material's alpha) into the sun
    /// depth pass. Masked and blended primitives do not cast.
    pub fn draw_shadow(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        shaders: &crate::shader_gpu::ShaderGpu,
    ) -> (u32, u64) {
        let (mut calls, mut triangles) = (0, 0);
        if self.active.is_empty() {
            return (calls, triangles);
        }
        pass.set_pipeline(&self.shadow_pipeline);
        pass.set_bind_group(0, &self.shadow_camera, &[0]);
        for active in &self.active {
            let mesh = &self.meshes[&active.key];
            let surface = active
                .material
                .as_ref()
                .and_then(|id| shaders.materials.get(id))
                .map(|s| s.alpha);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            for (i, m) in mesh.materials.iter().enumerate() {
                if !casts_shadow(surface, m.alpha) || m.range.is_empty() {
                    continue;
                }
                pass.set_bind_group(1, &m.bind, &[((active.first + i) * DRAW_STRIDE) as u32]);
                pass.draw_indexed(m.range.clone(), 0, 0..1);
                calls += 1;
                triangles += m.range.len() as u64 / 3;
            }
        }
        (calls, triangles)
    }
}
/// A primitive gets its `render.outline` hull when it has a width, is opaque after any surface
/// material (the [`casts_shadow`] rule) and is single-sided: a double-sided sheet has no inside,
/// so the hull of a sheet wound away from the camera would cover the sheet itself.
pub(crate) fn outlined(
    width: f32,
    surface: Option<crate::shader::Alpha>,
    primitive: AlphaMode,
    double_sided: bool,
) -> bool {
    width != 0.0 && !double_sided && casts_shadow(surface, primitive)
}
/// A mesh primitive casts into the sun depth pass when it is opaque after any surface material
/// (`inherit` keeps the primitive's own glTF alpha); masked and blended primitives do not.
pub(crate) fn casts_shadow(surface: Option<crate::shader::Alpha>, primitive: AlphaMode) -> bool {
    use crate::shader::Alpha;
    match surface.unwrap_or(Alpha::Inherit) {
        Alpha::Inherit => matches!(primitive, AlphaMode::Opaque),
        Alpha::Opaque => true,
        Alpha::Mask | Alpha::Blend | Alpha::Additive | Alpha::Multiply => false,
    }
}
fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("base color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::Alpha;

    /// The Rust `Draw` and the WGSL `Draw` have the same size, and the mesh module (with the
    /// outline and decal entry points / bindings) validates, also as a custom-material
    /// template.
    #[test]
    fn rust_and_wgsl_draw_match_and_the_mesh_module_validates() {
        let validate = |source: &str| {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap();
            module
        };
        let module = validate(crate::lighting::MESH_WGSL);
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("Draw"))
            .unwrap();
        let naga::TypeInner::Struct { span, .. } = ty.inner else {
            panic!("Draw is not a struct");
        };
        assert_eq!(span as usize, std::mem::size_of::<Draw>());
        for entry in [
            "vertex_main",
            "fragment_main",
            "vs_mesh_shadow",
            "vs_outline",
            "fs_outline",
        ] {
            assert!(
                module.entry_points.iter().any(|e| e.name == entry),
                "{entry}"
            );
        }
        // A custom surface material compiles the same module plus its wrapper (decal applied).
        let toon = "fn surface(s: SurfaceInput) -> vec4<f32> { return s.base_color; }";
        validate(&crate::shader::fragment(
            crate::lighting::MESH_WGSL,
            toon,
            &format!(
                "{}\n{}",
                include_str!("surface_material.wgsl"),
                include_str!("mesh_surface.wgsl")
            ),
        ));
    }

    #[test]
    fn outlines_skip_empty_widths_double_sided_and_non_opaque_primitives() {
        assert!(outlined(-2.0, None, AlphaMode::Opaque, false));
        assert!(outlined(0.01, Some(Alpha::Opaque), AlphaMode::Blend, false));
        assert!(!outlined(0.0, None, AlphaMode::Opaque, false));
        assert!(!outlined(-2.0, None, AlphaMode::Opaque, true));
        assert!(!outlined(-2.0, None, AlphaMode::Mask(0.5), false));
        assert!(!outlined(-2.0, Some(Alpha::Blend), AlphaMode::Opaque, false));
    }

    #[test]
    fn only_opaque_primitives_cast_sun_shadows() {
        assert!(casts_shadow(None, AlphaMode::Opaque));
        assert!(!casts_shadow(None, AlphaMode::Mask(0.5)));
        assert!(!casts_shadow(None, AlphaMode::Blend));
        assert!(casts_shadow(Some(Alpha::Inherit), AlphaMode::Opaque));
        assert!(!casts_shadow(Some(Alpha::Inherit), AlphaMode::Blend));
        // A surface material's own alpha wins over the primitive's.
        assert!(casts_shadow(Some(Alpha::Opaque), AlphaMode::Blend));
        for alpha in [Alpha::Mask, Alpha::Blend, Alpha::Additive, Alpha::Multiply] {
            assert!(!casts_shadow(Some(alpha), AlphaMode::Opaque));
        }
    }
}
