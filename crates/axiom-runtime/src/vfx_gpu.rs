use crate::vfx::{self, Frame, Particle};
use anyhow::{ensure, Result};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::ops::Range;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct EmitterUniform {
    origin_time: [f32; 4],
    velocity_life: [f32; 4],
    accel_spread: [f32; 4],
    color_a: [f32; 4],
    color_b: [f32; 4],
    size_misc: [f32; 4],
    ids: [u32; 4],
}
struct Draw {
    range: Range<u32>,
    mesh: bool,
    additive: bool,
}
pub struct VfxGpu {
    particles: wgpu::Buffer,
    emitter_buffer: wgpu::Buffer,
    emitters: wgpu::BindGroup,
    storage: wgpu::BindGroup,
    compute: wgpu::ComputePipeline,
    draw_bind: wgpu::BindGroup,
    draw_layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::RenderPipeline>,
    ribbons: wgpu::Buffer,
    ribbon_count: u32,
    draws: Vec<Draw>,
    dispatches: Vec<(u32, u32)>,
}
impl VfxGpu {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        globals: &wgpu::Buffer,
        depth: &wgpu::TextureView,
    ) -> Self {
        let particles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("VFX CPU uploads and GPU compute output"),
            size: (vfx::MAX_PARTICLES * std::mem::size_of::<Particle>()) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let emitter_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("VFX seeded emitters"),
            size: 64 * 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let emitter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VFX emitter"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(
                        std::mem::size_of::<EmitterUniform>() as u64,
                    ),
                },
                count: None,
            }],
        });
        let storage_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VFX storage"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let emitters = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("VFX emitter bank"),
            layout: &emitter_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &emitter_buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<EmitterUniform>() as u64),
                }),
            }],
        });
        let storage = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("VFX compute storage"),
            layout: &storage_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: particles.as_entire_binding(),
            }],
        });
        let compute_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VFX analytic compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("vfx_compute.wgsl").into()),
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("VFX GPU trajectories"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&emitter_layout, &storage_layout],
                    push_constant_ranges: &[],
                }),
            ),
            module: &compute_module,
            entry_point: Some("simulate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VFX camera and opaque depth"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(crate::render::GLOBALS_SIZE),
                    },
                    count: None,
                },
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
            ],
        });
        let draw_bind = Self::bind(device, &draw_layout, globals, depth);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VFX sprite / model / ribbon"),
            source: wgpu::ShaderSource::Wgsl(crate::lighting::VFX_DRAW_WGSL.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&draw_layout],
            push_constant_ranges: &[],
        });
        let mut pipelines = vec![];
        for kind in ["sprite", "mesh", "ribbon"] {
            for additive in [false, true] {
                const ATTRS: [wgpu::VertexAttribute; 3] =
                    wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4];
                pipelines.push(
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some(kind),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some(kind),
                            compilation_options: Default::default(),
                            buffers: &[wgpu::VertexBufferLayout {
                                array_stride: 48,
                                step_mode: if kind == "ribbon" {
                                    wgpu::VertexStepMode::Vertex
                                } else {
                                    wgpu::VertexStepMode::Instance
                                },
                                attributes: &ATTRS,
                            }],
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some("fragment"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend: Some(if additive {
                                    wgpu::BlendState {
                                        color: wgpu::BlendComponent {
                                            src_factor: wgpu::BlendFactor::SrcAlpha,
                                            dst_factor: wgpu::BlendFactor::One,
                                            operation: wgpu::BlendOperation::Add,
                                        },
                                        alpha: wgpu::BlendComponent::OVER,
                                    }
                                } else {
                                    wgpu::BlendState::ALPHA_BLENDING
                                }),
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        primitive: wgpu::PrimitiveState {
                            cull_mode: None,
                            ..Default::default()
                        },
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: false,
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
        let ribbons = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("VFX ribbon strips"),
            size: 65536 * 48,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            particles,
            emitter_buffer,
            emitters,
            storage,
            compute,
            draw_bind,
            draw_layout,
            pipelines,
            ribbons,
            ribbon_count: 0,
            draws: vec![],
            dispatches: vec![],
        }
    }
    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        globals: &wgpu::Buffer,
        depth: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("VFX read-only scene"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: globals,
                        offset: 0,
                        size: std::num::NonZeroU64::new(crate::render::GLOBALS_SIZE),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        })
    }
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        globals: &wgpu::Buffer,
        depth: &wgpu::TextureView,
    ) {
        self.draw_bind = Self::bind(device, &self.draw_layout, globals, depth);
    }
    pub fn prepare(&mut self, queue: &wgpu::Queue, frame: &Frame) -> Result<()> {
        self.draws.clear();
        self.dispatches.clear();
        let mut first = 0u32;
        ensure!(frame.batches.len() <= 64, "GPU emitter bank exceeded");
        for (i, b) in frame.batches.iter().enumerate() {
            let e = &b.emitter;
            let end = first + e.count;
            ensure!(
                end as usize <= vfx::MAX_PARTICLES,
                "GPU particle budget exceeded"
            );
            if e.backend == "gpu" {
                let uniform = EmitterUniform {
                    origin_time: [b.origin.x, b.origin.y, b.origin.z, b.time],
                    velocity_life: [e.velocity[0], e.velocity[1], e.velocity[2], e.lifetime],
                    accel_spread: [
                        e.acceleration[0],
                        e.acceleration[1],
                        e.acceleration[2],
                        e.spread,
                    ],
                    color_a: e.color,
                    color_b: e.end_color,
                    size_misc: [
                        e.size,
                        e.end_size,
                        if e.renderer == "mesh" { 1.0 } else { 0.0 },
                        0.0,
                    ],
                    ids: [e.count, e.seed, first, e.field_id()],
                };
                queue.write_buffer(
                    &self.emitter_buffer,
                    (i * 256) as u64,
                    bytemuck::bytes_of(&uniform),
                );
                self.dispatches.push((i as u32, e.count));
            } else {
                ensure!(
                    b.particles.len() == e.count as usize,
                    "CPU particle count mismatch"
                );
                queue.write_buffer(
                    &self.particles,
                    u64::from(first) * 48,
                    bytemuck::cast_slice(&b.particles),
                );
            }
            self.draws.push(Draw {
                range: first..end,
                mesh: e.renderer == "mesh",
                additive: e.field != "smoke" && e.field != "snow",
            });
            first = end;
        }
        let mut vertices = vec![];
        for ribbon in &frame.ribbons {
            for i in 1..ribbon.points.len() {
                let a = ribbon.points[i - 1];
                let b = ribbon.points[i];
                let tangent = b - a;
                if tangent.length_squared() < 1e-10 {
                    continue;
                }
                for (p, side, t) in [
                    (a, -1.0, i - 1),
                    (a, 1.0, i - 1),
                    (b, 1.0, i),
                    (a, -1.0, i - 1),
                    (b, 1.0, i),
                    (b, -1.0, i),
                ] {
                    let mut color = ribbon.color;
                    color[3] *= t as f32 / ribbon.points.len() as f32;
                    vertices.push(Particle {
                        position_size: [p.x, p.y, p.z, ribbon.width],
                        color: [tangent.x, tangent.y, tangent.z, side],
                        misc: color,
                    });
                }
            }
        }
        ensure!(vertices.len() <= 65536, "ribbon vertex budget exceeded");
        self.ribbon_count = vertices.len() as u32;
        if !vertices.is_empty() {
            queue.write_buffer(&self.ribbons, 0, bytemuck::cast_slice(&vertices));
        }
        Ok(())
    }
    pub fn compute(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.dispatches.is_empty() {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("VFX GPU simulation"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.compute);
        pass.set_bind_group(1, &self.storage, &[]);
        for (i, count) in &self.dispatches {
            pass.set_bind_group(0, &self.emitters, &[*i * 256]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
    }
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera: u32) -> (u32, u64) {
        let (mut calls, mut triangles) = (0, 0);
        pass.set_bind_group(
            0,
            &self.draw_bind,
            &[camera * crate::render::GLOBALS_STRIDE],
        );
        pass.set_vertex_buffer(0, self.particles.slice(..));
        for d in &self.draws {
            pass.set_pipeline(&self.pipelines[usize::from(d.mesh) * 2 + usize::from(d.additive)]);
            pass.draw(0..if d.mesh { 36 } else { 6 }, d.range.clone());
            calls += 1;
            triangles += if d.mesh { 12 } else { 2 } * d.range.len() as u64;
        }
        if self.ribbon_count > 0 {
            pass.set_pipeline(&self.pipelines[5]);
            pass.set_vertex_buffer(0, self.ribbons.slice(..));
            pass.draw(0..self.ribbon_count, 0..1);
            calls += 1;
            triangles += u64::from(self.ribbon_count / 3);
        }
        (calls, triangles)
    }
}
/// An actual storage-buffer readback, separate from transport observations and timing claims.
pub fn validate_compute() -> Result<serde_json::Value> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(&Default::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .context("GPU adapter unavailable")?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("VFX compute readback validation"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                },
                None,
            )
            .await?;
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &vec![0u8; crate::render::GLOBALS_STRIDE as usize],
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let mut gpu = VfxGpu::new(&device, wgpu::TextureFormat::Rgba8Unorm, &globals, &depth);
        let mut cases = 0;
        let mut max_error = 0.0f32;
        for field in ["fountain", "smoke", "snow"] {
            for time in [0.0, 0.25, 2.5] {
                let e = vfx::Emitter {
                    backend: "gpu".into(),
                    count: 64,
                    seed: 1729,
                    field: field.into(),
                    ..Default::default()
                };
                let origin = Vec3::new(1.0, 2.0, -3.0);
                gpu.prepare(
                    &queue,
                    &Frame {
                        batches: vec![vfx::Batch {
                            id: "verify".into(),
                            emitter: e.clone(),
                            time,
                            origin,
                            particles: vec![],
                        }],
                        ribbons: vec![],
                    },
                )?;
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 64 * 48,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                gpu.compute(&mut encoder);
                encoder.copy_buffer_to_buffer(&gpu.particles, 0, &buffer, 0, 64 * 48);
                queue.submit([encoder.finish()]);
                let (tx, rx) = std::sync::mpsc::channel();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    let _ = tx.send(r);
                });
                device.poll(wgpu::Maintain::Wait);
                rx.recv()??;
                let bytes = buffer.slice(..).get_mapped_range();
                let data: Vec<Particle> = bytemuck::cast_slice::<u8, Particle>(&bytes).to_vec();
                for (i, actual) in data.iter().enumerate() {
                    let expected = vfx::evaluate(&e, i as u32, time, origin);
                    for (a, b) in actual
                        .position_size
                        .iter()
                        .chain(&actual.color)
                        .chain(&actual.misc)
                        .zip(
                            expected
                                .position_size
                                .iter()
                                .chain(&expected.color)
                                .chain(&expected.misc),
                        )
                    {
                        ensure!(a.is_finite(), "GPU particle not finite");
                        max_error = max_error.max((a - b).abs());
                    }
                }
                drop(bytes);
                buffer.unmap();
                cases += 1;
            }
        }
        ensure!(
            max_error < 0.002,
            "CPU/GPU particle contract mismatch: {max_error}"
        );
        Ok(
            serde_json::json!({"ok":true,"gpu":adapter.get_info().name,"cases":cases,"particles_compared":cases*64,"max_absolute_error":max_error,"measurement":"GPU compute storage readback; no GPU timing claim"}),
        )
    })
}
use anyhow::Context;
