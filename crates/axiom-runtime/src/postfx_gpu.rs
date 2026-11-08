use crate::{postfx::Volume, presentation::RenderView};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVolume {
    center_kind: [f32; 4],
    half_strength: [f32; 4],
    color: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    inverse: [[f32; 4]; 4],
    eye_time: [f32; 4],
    viewport: [f32; 4],
    info: [f32; 4],
    volumes: [GpuVolume; 8],
}
pub struct PostGpu {
    pub scene: wgpu::TextureView,
    texture: wgpu::Texture,
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
}
impl PostGpu {
    fn target(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("postfx scene color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        depth: &wgpu::TextureView,
    ) -> Self {
        let texture = Self::target(device, format, width, height);
        let scene = texture.create_view(&Default::default());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene color/depth post effects"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<Uniform>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post effects volume bank"),
            size: 512 * axiom_core::MAX_CAMERAS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind = Self::bind(device, &layout, &scene, depth, &sampler, &uniform);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post effects / depth projected decals / volumetric fog"),
            source: wgpu::ShaderSource::Wgsl(include_str!("postfx.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post effects"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&layout],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        Self {
            scene,
            texture,
            layout,
            bind,
            uniform,
            sampler,
            pipeline,
            format,
        }
    }
    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: uniform,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<Uniform>() as u64),
                    }),
                },
            ],
        })
    }
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        depth: &wgpu::TextureView,
    ) {
        self.texture = Self::target(device, self.format, width, height);
        self.scene = self.texture.create_view(&Default::default());
        self.bind = Self::bind(
            device,
            &self.layout,
            &self.scene,
            depth,
            &self.sampler,
            &self.uniform,
        );
    }
    pub fn prepare(
        &self,
        queue: &wgpu::Queue,
        volumes: &[Volume],
        views: &[RenderView],
        width: u32,
        height: u32,
        tick: u64,
    ) {
        for (i, v) in views.iter().enumerate() {
            let vp = v.camera.viewport;
            let aspect = width as f32 * vp[2] / (height as f32 * vp[3]);
            let projection = match v.camera.projection {
                axiom_core::Projection::Perspective { fov_degrees } => Mat4::perspective_rh(
                    fov_degrees.to_radians(),
                    aspect,
                    v.camera.near,
                    v.camera.far,
                ),
                axiom_core::Projection::Orthographic { height } => Mat4::orthographic_rh(
                    -height * aspect * 0.5,
                    height * aspect * 0.5,
                    -height * 0.5,
                    height * 0.5,
                    v.camera.near,
                    v.camera.far,
                ),
            };
            let mut bank = [GpuVolume::zeroed(); 8];
            for (out, v) in bank.iter_mut().zip(volumes.iter().take(8)) {
                *out = GpuVolume {
                    center_kind: [v.center[0], v.center[1], v.center[2], v.kind_id()],
                    half_strength: [v.half_size[0], v.half_size[1], v.half_size[2], v.strength],
                    color: v.color,
                };
            }
            let u = Uniform {
                inverse: (projection * Mat4::look_at_rh(v.eye, v.eye + v.forward, v.up))
                    .inverse()
                    .to_cols_array_2d(),
                eye_time: [v.eye.x, v.eye.y, v.eye.z, tick as f32 / 120.0],
                viewport: [
                    vp[0] * width as f32,
                    vp[1] * height as f32,
                    vp[2] * width as f32,
                    vp[3] * height as f32,
                ],
                info: [volumes.len().min(8) as f32, 0.0, 0.0, 0.0],
                volumes: bank,
            };
            queue.write_buffer(&self.uniform, (i * 512) as u64, bytemuck::bytes_of(&u));
        }
    }
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        index: u32,
        viewport: [f32; 4],
        width: u32,
        height: u32,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post effects before UI"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if index == 0 {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let [x, y, w, h] = viewport;
        pass.set_viewport(
            x * width as f32,
            y * height as f32,
            w * width as f32,
            h * height as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[index * 512]);
        pass.draw(0..3, 0..1);
    }
}
