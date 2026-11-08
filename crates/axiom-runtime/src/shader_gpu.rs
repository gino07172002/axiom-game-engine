//! GPU material cache. Replacement pipelines and uniforms are staged as one revision.
use crate::shader::{self, Alpha, Prepared};
use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use std::{collections::BTreeMap, sync::Arc};

const STRIDE: u32 = 256;
const MATERIAL_WGSL: &str = include_str!("surface_material.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    tint: [f32; 4],
    params: [f32; 4],
    alpha: [f32; 4],
}

pub struct GpuMaterial {
    shader: Arc<Program>,
    pub alpha: Alpha,
    pub double_sided: Option<bool>,
    pub offset: u32,
}
impl GpuMaterial {
    pub fn pipeline(&self, mesh: bool, blend: bool, double_sided: bool) -> &wgpu::RenderPipeline {
        let variants = if mesh {
            &self.shader.mesh
        } else {
            &self.shader.cube
        };
        &variants[(if !blend {
            0
        } else {
            match self.alpha {
                Alpha::Additive => 2,
                Alpha::Multiply => 3,
                _ => 1,
            }
        }) * 2
            + usize::from(double_sided)]
    }
}

struct Program {
    hook: String,
    cube: Vec<wgpu::RenderPipeline>,
    mesh: Vec<wgpu::RenderPipeline>,
}

pub struct ShaderGpu {
    format: wgpu::TextureFormat,
    cube: wgpu::PipelineLayout,
    mesh: wgpu::PipelineLayout,
    uniforms: wgpu::Buffer,
    pub bind: wgpu::BindGroup,
    active: Option<Arc<Prepared>>,
    programs: BTreeMap<String, Arc<Program>>,
    pub materials: BTreeMap<String, GpuMaterial>,
}
impl ShaderGpu {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        scene: &wgpu::BindGroupLayout,
        mesh_layouts: [&wgpu::BindGroupLayout; 2],
    ) -> Self {
        let size = std::num::NonZeroU64::new(std::mem::size_of::<Uniform>() as u64);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface material parameters"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: size,
                },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("surface material bank"),
            size: shader::MAX_MATERIALS as u64 * u64::from(STRIDE),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface material bank"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size,
                }),
            }],
        });
        let pipeline_layout = |layouts: &[&wgpu::BindGroupLayout]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("surface pipeline layout"),
                bind_group_layouts: layouts,
                push_constant_ranges: &[],
            })
        };
        Self {
            format,
            cube: pipeline_layout(&[scene, &layout]),
            mesh: pipeline_layout(&[mesh_layouts[0], mesh_layouts[1], &layout]),
            uniforms,
            bind,
            active: None,
            programs: BTreeMap::new(),
            materials: BTreeMap::new(),
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        book: &Arc<Prepared>,
    ) -> Result<()> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, book))
        {
            return Ok(());
        }
        let mut programs = BTreeMap::new();
        for (id, hook) in &book.programs {
            let program = match self.programs.get(id).filter(|p| &p.hook == hook) {
                Some(p) => p.clone(),
                None => Arc::new(self.compile(device, id, hook)?),
            };
            programs.insert(id.clone(), program);
        }
        let mut upload = vec![0_u8; book.materials.len() * STRIDE as usize];
        let mut materials = BTreeMap::new();
        for (i, (id, m)) in book.materials.iter().enumerate() {
            let offset = i as u32 * STRIDE;
            let mode = match m.alpha {
                Alpha::Inherit => -1.0,
                Alpha::Opaque => 0.0,
                Alpha::Mask => 1.0,
                Alpha::Blend | Alpha::Additive | Alpha::Multiply => 2.0,
            };
            let uniform = Uniform {
                tint: m.tint,
                params: m.values,
                alpha: [mode, m.cutoff, 0.0, 0.0],
            };
            let bytes = bytemuck::bytes_of(&uniform);
            upload[offset as usize..offset as usize + bytes.len()].copy_from_slice(bytes);
            materials.insert(
                id.clone(),
                GpuMaterial {
                    shader: programs[&m.shader].clone(),
                    alpha: m.alpha,
                    double_sided: m.double_sided,
                    offset,
                },
            );
        }
        if !upload.is_empty() {
            queue.write_buffer(&self.uniforms, 0, &upload);
        }
        self.programs = programs;
        self.materials = materials;
        self.active = Some(book.clone());
        Ok(())
    }

    fn compile(&self, device: &wgpu::Device, id: &str, hook: &str) -> Result<Program> {
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let cube = self.variants(device, id, hook, false);
        let mesh = self.variants(device, id, hook, true);
        if let Some(err) = pollster::block_on(device.pop_error_scope()) {
            let message = err.to_string();
            let compact = message
                .lines()
                .take(14)
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(1400)
                .collect::<String>();
            return Err(axiom_core::Diagnostic::error(
                "shader_pipeline",
                shader::shader_path(id),
                compact,
            ));
        }
        Ok(Program {
            hook: hook.into(),
            cube,
            mesh,
        })
    }

    fn variants(
        &self,
        device: &wgpu::Device,
        id: &str,
        hook: &str,
        mesh: bool,
    ) -> Vec<wgpu::RenderPipeline> {
        let (base, wrapper, vertex, layout) = if mesh {
            (
                crate::lighting::MESH_WGSL,
                include_str!("mesh_surface.wgsl"),
                "vertex_main",
                &self.mesh,
            )
        } else {
            (
                crate::lighting::SCENE_WGSL,
                include_str!("cube_surface.wgsl"),
                "vs_main",
                &self.cube,
            )
        };
        let source = shader::fragment(base, hook, &format!("{MATERIAL_WGSL}\n{wrapper}"));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(id),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let triangle = [crate::mesh_render::mesh_vertex_layout()];
        let cube = crate::render::cube_vertex_layouts();
        let buffers = if mesh {
            triangle.as_slice()
        } else {
            cube.as_slice()
        };
        let mut variants = Vec::with_capacity(8);
        for blend_mode in 0..4 {
            let blend = blend_mode != 0;
            for double_sided in [false, true] {
                variants.push(
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some(id),
                        layout: Some(layout),
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some(vertex),
                            compilation_options: Default::default(),
                            buffers,
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some("fs_surface"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: self.format,
                                blend: match blend_mode {
                                    0 => None,
                                    2 => Some(wgpu::BlendState {
                                        color: wgpu::BlendComponent {
                                            src_factor: wgpu::BlendFactor::SrcAlpha,
                                            dst_factor: wgpu::BlendFactor::One,
                                            operation: wgpu::BlendOperation::Add,
                                        },
                                        alpha: wgpu::BlendComponent::OVER,
                                    }),
                                    3 => Some(wgpu::BlendState {
                                        color: wgpu::BlendComponent {
                                            src_factor: wgpu::BlendFactor::Dst,
                                            dst_factor: wgpu::BlendFactor::Zero,
                                            operation: wgpu::BlendOperation::Add,
                                        },
                                        alpha: wgpu::BlendComponent::OVER,
                                    }),
                                    _ => Some(wgpu::BlendState::ALPHA_BLENDING),
                                },
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        primitive: wgpu::PrimitiveState {
                            cull_mode: (!double_sided).then_some(wgpu::Face::Back),
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
        variants
    }
}
