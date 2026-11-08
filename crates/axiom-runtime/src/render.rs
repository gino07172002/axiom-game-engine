use crate::presentation::{RenderFrame, RenderView};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::{path::Path, sync::Arc};
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}
pub(crate) fn cube_vertex_layouts() -> [wgpu::VertexBufferLayout<'static>; 2] {
    const VERTEX: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3];
    const INSTANCE: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4];
    [
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX,
        },
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::presentation::Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE,
        },
    ]
}
pub(crate) const GLOBALS_STRIDE: u32 = 1280;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LocalLight {
    position_radius: [f32; 4],
    color_intensity: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_vp: [[f32; 4]; 4],
    eye_time: [f32; 4],
    accent: [f32; 4],
    lights: [LocalLight; 32],
    view_params: [f32; 4],
}

pub struct Renderer {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub egui: egui_wgpu::Renderer,
    pub gpu: String,
    pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    shadow_bind: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    depth: wgpu::TextureView,
    shadow: wgpu::TextureView,
    can_capture: bool,
    vfx: crate::vfx_gpu::VfxGpu,
    post: crate::postfx_gpu::PostGpu,
    meshes: crate::mesh_render::MeshRenderer,
    shaders: crate::shader_gpu::ShaderGpu,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no compatible GPU adapter")?;
        let info = adapter.get_info();
        let gpu = format!("{} / {:?}", info.name, info.backend);
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("axiom device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let can_capture = caps.usages.contains(wgpu::TextureUsages::COPY_SRC);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if can_capture {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let depth = depth_view(&device, config.width, config.height, "scene depth", true);
        let shadow = depth_view(&device, 2048, 2048, "sun shadow", true);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow PCF"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera globals"),
            size: u64::from(GLOBALS_STRIDE) * axiom_core::MAX_CAMERAS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene bindings"),
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
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow uniform only"),
            entries: &[uniform_entry],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<Globals>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shadow_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow bind"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<Globals>() as u64),
                }),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&shadow_layout],
                push_constant_ranges: &[],
            });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("axiom scene WGSL"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let buffers = cube_vertex_layouts();
        let depth_state = wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("instanced geometry"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth_state.clone()),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("instanced shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
                ..depth_state
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared unit cube"),
            contents: bytemuck::cast_slice(&cube_vertices()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance SoA upload"),
            size: 8192 * 96,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let egui = egui_wgpu::Renderer::new(&device, format, None, 1, false);
        let meshes = crate::mesh_render::MeshRenderer::new(&device, format, &uniform);
        let shaders = crate::shader_gpu::ShaderGpu::new(&device, format, &layout, meshes.layouts());
        let vfx = crate::vfx_gpu::VfxGpu::new(&device, format, &uniform, &depth);
        let post =
            crate::postfx_gpu::PostGpu::new(&device, format, config.width, config.height, &depth);
        Ok(Self {
            surface,
            vfx,
            post,
            device,
            queue,
            config,
            egui,
            gpu,
            pipeline,
            shadow_pipeline,
            sky_pipeline,
            uniform,
            bind,
            shadow_bind,
            vertices,
            instances,
            depth,
            shadow,
            can_capture,
            meshes,
            shaders,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = depth_view(&self.device, size.width, size.height, "scene depth", true);
        self.vfx.resize(&self.device, &self.uniform, &self.depth);
        self.post
            .resize(&self.device, size.width, size.height, &self.depth);
    }

    pub fn prepare_shaders(&mut self, book: &Arc<crate::shader::Prepared>) -> Result<()> {
        self.shaders.prepare(&self.device, &self.queue, book)
    }

    pub fn render(
        &mut self,
        snapshot: &RenderFrame,
        ctx: &egui::Context,
        output: egui::FullOutput,
        screenshot: Option<&Path>,
    ) -> Result<bool> {
        self.prepare_shaders(&snapshot.shaders)?;
        self.vfx.prepare(&self.queue, &snapshot.vfx)?;
        let fallback = vec![RenderView {
            id: "engine.preview".into(),
            eye: Vec3::new(0.0, 3.0, 8.0),
            forward: Vec3::new(0.0, -0.15, -1.0).normalize(),
            up: Vec3::Y,
            camera: axiom_core::Camera::perspective(),
            instances: vec![],
        }];
        let views = if snapshot.views.is_empty() {
            &fallback
        } else {
            &snapshot.views
        };
        anyhow::ensure!(
            views.len() <= axiom_core::MAX_CAMERAS,
            "camera budget exceeded"
        );
        if !snapshot.postfx.is_empty() {
            self.post.prepare(
                &self.queue,
                &snapshot.postfx,
                views,
                self.config.width,
                self.config.height,
                snapshot.tick,
            );
        }
        let extent = snapshot.settings.shadow_extent;
        let light_vp = Mat4::orthographic_rh(-extent, extent, -extent, extent, 1.0, 170.0)
            * Mat4::look_at_rh(Vec3::new(-45.0, 75.0, 35.0), Vec3::ZERO, Vec3::Y);
        let mut uniforms = vec![0_u8; views.len() * GLOBALS_STRIDE as usize];
        let mut geometry = snapshot.instances.clone();
        let shared = geometry.len() as u32;
        let mut cube_runs = Vec::with_capacity(snapshot.cube_materials.len() * 2 + 1);
        let mut first = 0;
        for (range, id) in &snapshot.cube_materials {
            if first < range.start {
                cube_runs.push((first..range.start, None));
            }
            let material = self
                .shaders
                .materials
                .get(id)
                .context("prepared cube material missing")?;
            cube_runs.push((range.clone(), Some(material)));
            first = range.end;
        }
        if first < shared {
            cube_runs.push((first..shared, None));
        }
        let mut ranges = vec![];
        for (i, v) in views.iter().enumerate() {
            let viewport = v.camera.viewport;
            let aspect = (self.config.width as f32 * viewport[2])
                / (self.config.height as f32 * viewport[3]);
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
            let mut lights = [LocalLight::zeroed(); 32];
            for (out, light) in lights.iter_mut().zip(snapshot.lights.iter().take(32)) {
                *out = LocalLight {
                    position_radius: [
                        light.position.x,
                        light.position.y,
                        light.position.z,
                        light.radius,
                    ],
                    color_intensity: [
                        light.color[0],
                        light.color[1],
                        light.color[2],
                        light.intensity,
                    ],
                };
            }
            let global = Globals {
                view_proj: (projection * Mat4::look_at_rh(v.eye, v.eye + v.forward, v.up))
                    .to_cols_array_2d(),
                light_vp: light_vp.to_cols_array_2d(),
                eye_time: [v.eye.x, v.eye.y, v.eye.z, snapshot.tick as f32 / 120.0],
                accent: [
                    snapshot.settings.accent[0],
                    snapshot.settings.accent[1],
                    snapshot.settings.accent[2],
                    snapshot.lights.len().min(32) as f32,
                ],
                lights,
                view_params: [
                    v.camera.near,
                    v.camera.far,
                    if matches!(
                        v.camera.projection,
                        axiom_core::Projection::Orthographic { .. }
                    ) {
                        1.0
                    } else {
                        0.0
                    },
                    0.0,
                ],
            };
            let bytes = bytemuck::bytes_of(&global);
            uniforms[i * GLOBALS_STRIDE as usize..i * GLOBALS_STRIDE as usize + bytes.len()]
                .copy_from_slice(bytes);
            let first = geometry.len() as u32;
            geometry.extend_from_slice(&v.instances);
            ranges.push(first..geometry.len() as u32);
        }
        self.queue.write_buffer(&self.uniform, 0, &uniforms);
        self.meshes
            .prepare(&self.device, &self.queue, &snapshot.meshes)?;
        anyhow::ensure!(geometry.len() <= 8192, "instance budget exceeded");
        self.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(&geometry));
        // Preserve font/texture uploads even if this surface frame must be skipped.
        for (id, delta) in &output.textures_delta.set {
            self.egui
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                for id in &output.textures_delta.free {
                    self.egui.free_texture(id);
                }
                return Ok(false);
            }
            Err(wgpu::SurfaceError::Timeout) => {
                for id in &output.textures_delta.free {
                    self.egui.free_texture(id);
                }
                return Ok(false);
            }
            Err(e) => return Err(e.into()),
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        self.vfx.compute(&mut encoder);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sun depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_bind, &[0]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            for (range, material) in &cube_runs {
                use crate::shader::Alpha;
                if material.is_none_or(|m| matches!(m.alpha, Alpha::Inherit | Alpha::Opaque)) {
                    pass.draw(0..36, range.clone());
                }
            }
        }
        let world_color = if snapshot.postfx.is_empty() {
            &view
        } else {
            &self.post.scene
        };
        for (i, camera_view) in views.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: world_color,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if i == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let [x, y, w, h] = camera_view.camera.viewport;
            let width = self.config.width as f32;
            let height = self.config.height as f32;
            let sx = (x * width).floor() as u32;
            let sy = (y * height).floor() as u32;
            let sw = ((w * width).ceil() as u32)
                .max(1)
                .min(self.config.width - sx);
            let sh = ((h * height).ceil() as u32)
                .max(1)
                .min(self.config.height - sy);
            pass.set_viewport(x * width, y * height, w * width, h * height, 0.0, 1.0);
            pass.set_scissor_rect(sx, sy, sw, sh);
            pass.set_bind_group(0, &self.bind, &[i as u32 * GLOBALS_STRIDE]);
            pass.set_pipeline(&self.sky_pipeline);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            for blend in [false, true] {
                if blend {
                    pass.set_bind_group(0, &self.bind, &[i as u32 * GLOBALS_STRIDE]);
                    pass.set_vertex_buffer(0, self.vertices.slice(..));
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                }
                for (range, material) in &cube_runs {
                    if material.is_some_and(|m| m.alpha.blended()) != blend {
                        continue;
                    }
                    if let Some(material) = material {
                        pass.set_pipeline(material.pipeline(
                            false,
                            blend,
                            material.double_sided.unwrap_or(false),
                        ));
                        pass.set_bind_group(1, &self.shaders.bind, &[material.offset]);
                    } else {
                        pass.set_pipeline(&self.pipeline);
                    }
                    pass.draw(0..36, range.clone());
                }
                if !blend && !ranges[i].is_empty() {
                    pass.set_pipeline(&self.pipeline);
                    pass.draw(0..36, ranges[i].clone());
                }
                self.meshes.draw(&mut pass, i as u32, &self.shaders, blend);
            }
            drop(pass);
            if !snapshot.vfx.batches.is_empty() || !snapshot.vfx.ribbons.is_empty() {
                let mut effects = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("VFX transparent read-only depth"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: world_color,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth,
                        depth_ops: None,
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                effects.set_viewport(x * width, y * height, w * width, h * height, 0.0, 1.0);
                effects.set_scissor_rect(sx, sy, sw, sh);
                self.vfx.draw(&mut effects, i as u32);
            }
            if !snapshot.postfx.is_empty() {
                self.post.draw(
                    &mut encoder,
                    &view,
                    i as u32,
                    camera_view.camera.viewport,
                    self.config.width,
                    self.config.height,
                );
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: output.pixels_per_point,
        };
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let extra =
            self.egui
                .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("interface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.egui
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        let capture = if screenshot.is_some() {
            anyhow::ensure!(
                self.can_capture,
                "surface does not support screenshot readback"
            );
            let row = (self.config.width * 4).div_ceil(256) * 256;
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("screenshot readback"),
                size: row as u64 * self.config.height as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &frame.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
            );
            Some((buffer, row))
        } else {
            None
        };
        self.queue
            .submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        frame.present();
        if let Some((buffer, row)) = capture {
            let slice = buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            self.device.poll(wgpu::Maintain::Wait);
            rx.recv()??;
            let mapped = slice.get_mapped_range();
            let mut pixels =
                Vec::with_capacity((self.config.width * self.config.height * 4) as usize);
            for chunk in mapped.chunks(row as usize) {
                pixels.extend_from_slice(&chunk[..self.config.width as usize * 4]);
            }
            if matches!(
                self.config.format,
                wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
            ) {
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
            }
            let path = screenshot.unwrap();
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            let mut png = png::Encoder::new(
                std::io::BufWriter::new(std::fs::File::create(path)?),
                self.config.width,
                self.config.height,
            );
            png.set_color(png::ColorType::Rgba);
            png.set_depth(png::BitDepth::Eight);
            png.write_header()?.write_image_data(&pixels)?;
            drop(mapped);
            buffer.unmap();
        }
        for id in &output.textures_delta.free {
            self.egui.free_texture(id);
        }
        Ok(true)
    }
}

fn depth_view(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
    sample: bool,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if sample {
                    wgpu::TextureUsages::TEXTURE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn cube_vertices() -> Vec<Vertex> {
    let mut vertices = vec![];
    for (normal, a, b) in [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (-Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (-Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, Vec3::Y, Vec3::X),
    ] {
        let p = [
            (normal - a - b) * 0.5,
            (normal + a - b) * 0.5,
            (normal + a + b) * 0.5,
            (normal - a + b) * 0.5,
        ];
        for index in [0, 1, 2, 0, 2, 3] {
            vertices.push(Vertex {
                position: p[index].to_array(),
                normal: normal.to_array(),
            });
        }
    }
    vertices
}
