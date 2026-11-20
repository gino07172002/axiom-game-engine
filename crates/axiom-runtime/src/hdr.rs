//! Post settings of `render.environment.post` on the GPU: the HDR scene target, bloom mip chain,
//! exposure + ACES tone mapping (`hdr.wgsl`) and FXAA (`fxaa.wgsl`).
//!
//! Pass order with every setting on, per camera view: world + VFX draw into the HDR target ->
//! bloom (prefilter, downsample chain, additive upsample chain) -> tone map into the scene colour
//! of the post effects (when the world has `postfx` volumes, which then run per view as before)
//! or the low dynamic range target; after every view, FXAA from that target into the swapchain,
//! then the interface. With everything off no target exists and no pass runs.
//!
//! Targets are created on first use and recreated only when the frame size or the needed set
//! changes (never per frame); turning every setting off frees them. The five pipelines here are
//! built once, on a worker thread (`hdr_build`), never on the main thread.
use crate::gpu_timing::{GpuFrame, GpuTimer, Section};
use axiom_core::post::{Hdr, PostProcess};

/// Scene colour format while HDR is on (built-in world pipelines have a variant for it).
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Bloom levels at most (the first at half resolution); fewer when the frame is small.
pub const MAX_BLOOM_MIPS: u32 = 6;
/// Pipelines this module owns (prefilter, downsample, upsample, tone map, FXAA).
pub const PIPELINES: usize = 5;
const UNIFORM_STRIDE: usize = 256;

/// What the frame needs, from `render.environment.post`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Settings {
    pub hdr: Option<Hdr>,
    pub fxaa: bool,
}
impl Settings {
    pub fn of(post: Option<&PostProcess>) -> Self {
        post.map_or_else(Self::default, |p| Self {
            hdr: p.hdr,
            fxaa: p.fxaa(),
        })
    }
    pub fn bloom(&self) -> bool {
        self.hdr.is_some_and(|h| h.bloom.is_some())
    }
    pub fn any(&self) -> bool {
        self.hdr.is_some() || self.fxaa
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    rect: [f32; 4],
    bloom: [f32; 4],
    tone: [f32; 4],
}
const _: () = assert!(std::mem::size_of::<Uniform>() <= UNIFORM_STRIDE);

/// Bloom levels for a frame: halving from half resolution while the smaller side stays >= 8.
pub fn bloom_mips(width: u32, height: u32) -> u32 {
    let mut side = (width.min(height) / 2).max(1);
    let mut mips = 1;
    while mips < MAX_BLOOM_MIPS && side / 2 >= 8 {
        side /= 2;
        mips += 1;
    }
    mips
}

/// The `Finish` uniform of one view (also unit-tested: bloom level weights).
fn uniform(settings: &Settings, rect: [f32; 4], mips: u32) -> Uniform {
    let hdr = settings.hdr.unwrap_or_default();
    let (bloom, on) = match hdr.bloom {
        Some(b) => {
            // Level k is weighted scatter^k; dividing by the sum keeps `intensity` the share of
            // the glow whatever the radius.
            let total: f32 = (0..mips as i32).map(|k| b.radius.powi(k)).sum();
            (
                [
                    b.threshold,
                    b.threshold * 0.5,
                    b.radius,
                    b.intensity / total,
                ],
                1.0,
            )
        }
        None => ([0.0; 4], 0.0),
    };
    Uniform {
        rect,
        bloom,
        tone: [hdr.exposure, on, 0.0, 0.0],
    }
}

pub(crate) struct Pipelines {
    prefilter: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    upsample: wgpu::RenderPipeline,
    tonemap: wgpu::RenderPipeline,
    fxaa: wgpu::RenderPipeline,
}

struct Bloom {
    _texture: wgpu::Texture,
    /// One view per level (render target), and the bind group sampling that level.
    levels: Vec<(wgpu::TextureView, wgpu::BindGroup, [u32; 2])>,
}

struct Targets {
    width: u32,
    height: u32,
    needs: (bool, bool, bool),
    hdr: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
    /// The tone map bind group (scene + glow, or the dummy glow without bloom).
    tonemap: Option<wgpu::BindGroup>,
    bloom: Option<Bloom>,
    ldr: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
    bytes: u64,
}

/// Observation of the post passes (`observe` -> `render_post`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Times the targets were (re)created (first use, resize, settings change).
    pub target_builds: u64,
    /// Milliseconds the background build took (`hdr_build`, worker thread): this module's
    /// pipelines and the HDR variants of the world pipelines.
    pub pipeline_ms: f64,
    /// Mean GPU milliseconds per measured frame (bloom, tone map, FXAA) and the frame count.
    pub gpu_ms: [f64; 3],
    pub gpu_frames: u64,
}

pub struct Finish {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    fxaa_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    scratch: Box<[u8]>,
    dummy: wgpu::TextureView,
    pipelines: Option<Pipelines>,
    targets: Option<Targets>,
    pub settings: Settings,
    pub stats: Stats,
    gpu_sum: [f64; 3],
}

fn target(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    mips: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// Bytes of a level chain of `format` (8 bytes per texel for Rgba16Float, 4 otherwise).
fn bytes(format: wgpu::TextureFormat, size: [u32; 2], mips: u32) -> u64 {
    let texel = if format == HDR_FORMAT { 8 } else { 4 };
    (0..mips)
        .map(|k| u64::from((size[0] >> k).max(1)) * u64::from((size[1] >> k).max(1)) * texel)
        .sum()
}

/// Viewport of normalized `rect` on a `size` target, kept inside it (wgpu validates bounds).
fn viewport(rect: [f32; 4], size: [u32; 2]) -> [f32; 4] {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let x = (rect[0] * w).clamp(0.0, w - 1.0);
    let y = (rect[1] * h).clamp(0.0, h - 1.0);
    [
        x,
        y,
        (rect[2] * w).clamp(1.0, w - x),
        (rect[3] * h).clamp(1.0, h - y),
    ]
}

impl Finish {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hdr finish"),
            entries: &[
                texture(0),
                sampler_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<Uniform>() as u64
                        ),
                    },
                    count: None,
                },
                texture(3),
            ],
        });
        let fxaa_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fxaa"),
            entries: &[texture(0), sampler_entry],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hdr finish per view"),
            size: (UNIFORM_STRIDE * axiom_core::MAX_CAMERAS) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dummy =
            target(device, "no bloom", HDR_FORMAT, [1, 1], 1).create_view(&Default::default());
        Self {
            format,
            layout,
            fxaa_layout,
            sampler,
            uniform,
            scratch: vec![0; UNIFORM_STRIDE * axiom_core::MAX_CAMERAS].into_boxed_slice(),
            dummy,
            pipelines: None,
            targets: None,
            settings: Settings::default(),
            stats: Stats::default(),
            gpu_sum: [0.0; 3],
        }
    }

    /// What a worker thread needs to build this module's pipelines (`build_pipelines`).
    pub(crate) fn post_inputs(&self) -> PostInputs {
        PostInputs {
            format: self.format,
            layout: self.layout.clone(),
            fxaa_layout: self.fxaa_layout.clone(),
        }
    }

    /// Pipelines built by `hdr_build` (off the main thread); `prepare` runs nothing until then.
    pub(crate) fn install(&mut self, pipelines: Pipelines) {
        self.pipelines = Some(pipelines);
    }

    pub(crate) fn has_pipelines(&self) -> bool {
        self.pipelines.is_some()
    }
}

/// Layouts and output format of the post pipelines (cheap clones of wgpu handles).
#[derive(Clone)]
pub(crate) struct PostInputs {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    fxaa_layout: wgpu::BindGroupLayout,
}

impl PostInputs {
    pub(crate) fn build_pipelines(&self, device: &wgpu::Device) -> Pipelines {
        let hdr = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom / tone map"),
            source: wgpu::ShaderSource::Wgsl(include_str!("hdr.wgsl").into()),
        });
        let fxaa = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fxaa"),
            source: wgpu::ShaderSource::Wgsl(include_str!("fxaa.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hdr finish"),
            bind_group_layouts: &[&self.layout],
            push_constant_ranges: &[],
        });
        let fxaa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fxaa"),
            bind_group_layouts: &[&self.fxaa_layout],
            push_constant_ranges: &[],
        });
        let pipeline = |label: &str,
                        module: &wgpu::ShaderModule,
                        layout: &wgpu::PipelineLayout,
                        entry: &str,
                        format: wgpu::TextureFormat,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        Pipelines {
            prefilter: pipeline(
                "bloom prefilter",
                &hdr,
                &layout,
                "prefilter",
                HDR_FORMAT,
                None,
            ),
            downsample: pipeline(
                "bloom downsample",
                &hdr,
                &layout,
                "downsample",
                HDR_FORMAT,
                None,
            ),
            upsample: pipeline(
                "bloom upsample",
                &hdr,
                &layout,
                "upsample",
                HDR_FORMAT,
                Some(add),
            ),
            tonemap: pipeline("tone map", &hdr, &layout, "tonemap", self.format, None),
            fxaa: pipeline("fxaa", &fxaa, &fxaa_layout, "fragment", self.format, None),
        }
    }
}

impl Finish {
    fn bind(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        glow: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hdr finish"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniform,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<Uniform>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(glow),
                },
            ],
        })
    }

    /// Makes the targets match `settings` at `width` x `height` (no-op when they
    /// already do) and uploads one uniform per view. Never allocates when nothing changed.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        settings: Settings,
        rects: impl Iterator<Item = [f32; 4]>,
        width: u32,
        height: u32,
    ) {
        self.settings = settings;
        let needs = (settings.hdr.is_some(), settings.bloom(), settings.fxaa);
        if !settings.any() {
            self.targets = None;
            return;
        }
        if self.pipelines.is_none() {
            // Not built yet (`hdr_build`): the renderer keeps the legacy path meanwhile.
            self.settings = Settings::default();
            self.targets = None;
            return;
        }
        if self
            .targets
            .as_ref()
            .is_none_or(|t| (t.width, t.height, t.needs) != (width, height, needs))
        {
            self.targets = None;
            self.targets = Some(self.build_targets(device, width, height, needs));
            self.stats.target_builds += 1;
        }
        let mips = self
            .targets
            .as_ref()
            .and_then(|t| t.bloom.as_ref())
            .map_or(0, |b| b.levels.len() as u32);
        let mut count = 0;
        for (i, rect) in rects.take(axiom_core::MAX_CAMERAS).enumerate() {
            let u = uniform(&settings, rect, mips.max(1));
            self.scratch[i * UNIFORM_STRIDE..i * UNIFORM_STRIDE + std::mem::size_of::<Uniform>()]
                .copy_from_slice(bytemuck::bytes_of(&u));
            count = i + 1;
        }
        if count > 0 {
            queue.write_buffer(&self.uniform, 0, &self.scratch[..count * UNIFORM_STRIDE]);
        }
    }

    fn build_targets(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        (hdr, bloom, fxaa): (bool, bool, bool),
    ) -> Targets {
        let size = [width, height];
        let mut out = Targets {
            width,
            height,
            needs: (hdr, bloom, fxaa),
            hdr: None,
            tonemap: None,
            bloom: None,
            ldr: None,
            bytes: 0,
        };
        if hdr {
            let texture = target(device, "hdr scene colour", HDR_FORMAT, size, 1);
            let view = texture.create_view(&Default::default());
            let prefilter = self.bind(device, &view, &self.dummy);
            out.bytes += bytes(HDR_FORMAT, size, 1);
            if bloom {
                let mips = bloom_mips(width, height);
                let half = [(width / 2).max(1), (height / 2).max(1)];
                let texture = target(device, "bloom levels", HDR_FORMAT, half, mips);
                let levels: Vec<_> = (0..mips)
                    .map(|k| {
                        let level = texture.create_view(&wgpu::TextureViewDescriptor {
                            label: Some("bloom level"),
                            base_mip_level: k,
                            mip_level_count: Some(1),
                            ..Default::default()
                        });
                        let bind = self.bind(device, &level, &self.dummy);
                        (level, bind, [(half[0] >> k).max(1), (half[1] >> k).max(1)])
                    })
                    .collect();
                out.tonemap = Some(self.bind(device, &view, &levels[0].0));
                out.bytes += bytes(HDR_FORMAT, half, mips);
                out.bloom = Some(Bloom {
                    _texture: texture,
                    levels,
                });
            } else {
                out.tonemap = Some(self.bind(device, &view, &self.dummy));
            }
            out.hdr = Some((texture, view, prefilter));
        }
        if fxaa {
            let texture = target(device, "fxaa input", self.format, size, 1);
            let view = texture.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fxaa"),
                layout: &self.fxaa_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            out.bytes += bytes(self.format, size, 1);
            out.ldr = Some((texture, view, bind));
        }
        out
    }

    /// Where the world and VFX of every view draw while HDR is on.
    pub fn hdr_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref()?.hdr.as_ref().map(|(_, v, _)| v)
    }

    /// The low dynamic range frame FXAA reads (the world draws here without HDR and postfx).
    pub fn ldr_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref()?.ldr.as_ref().map(|(_, v, _)| v)
    }

    /// Bloom and tone map of view `index` (normalized `rect`) from the HDR target into `dest`
    /// (frame size, the surface format). Returns the draw count.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_view(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timer: &mut GpuTimer,
        dest: &wgpu::TextureView,
        index: u32,
        rect: [f32; 4],
    ) -> u32 {
        let (Some(p), Some(t)) = (&self.pipelines, &self.targets) else {
            return 0;
        };
        let (Some((_, _, prefilter)), Some(tonemap)) = (&t.hdr, &t.tonemap) else {
            return 0;
        };
        let offset = [index * UNIFORM_STRIDE as u32];
        let mut draws = 0;
        let mut pass = |encoder: &mut wgpu::CommandEncoder,
                        view: &wgpu::TextureView,
                        size: [u32; 2],
                        load: wgpu::LoadOp<wgpu::Color>,
                        pipeline: &wgpu::RenderPipeline,
                        bind: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hdr finish"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let [x, y, w, h] = viewport(rect, size);
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &offset);
            pass.draw(0..3, 0..1);
            draws += 1;
        };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        if let Some(bloom) = &t.bloom {
            let levels = &bloom.levels;
            pass(
                encoder,
                &levels[0].0,
                levels[0].2,
                clear,
                &p.prefilter,
                prefilter,
            );
            for k in 1..levels.len() {
                pass(
                    encoder,
                    &levels[k].0,
                    levels[k].2,
                    clear,
                    &p.downsample,
                    &levels[k - 1].1,
                );
            }
            for k in (1..levels.len()).rev() {
                pass(
                    encoder,
                    &levels[k - 1].0,
                    levels[k - 1].2,
                    wgpu::LoadOp::Load,
                    &p.upsample,
                    &levels[k].1,
                );
            }
            timer.mark(encoder, Section::Bloom);
        }
        pass(
            encoder,
            dest,
            [t.width, t.height],
            if index == 0 {
                clear
            } else {
                wgpu::LoadOp::Load
            },
            &p.tonemap,
            tonemap,
        );
        timer.mark(encoder, Section::Tonemap);
        draws
    }

    /// FXAA of the whole low dynamic range frame into `dest`. Returns the draw count.
    pub fn fxaa(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timer: &mut GpuTimer,
        dest: &wgpu::TextureView,
    ) -> u32 {
        let (Some(p), Some((_, _, bind))) = (
            &self.pipelines,
            self.targets.as_ref().and_then(|t| t.ldr.as_ref()),
        ) else {
            return 0;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fxaa"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dest,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&p.fxaa);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        timer.mark(encoder, Section::Fxaa);
        1
    }

    /// Accumulates the GPU time of the post sections of a measured frame.
    pub fn record(&mut self, frame: &GpuFrame) {
        if !self.settings.any() {
            return;
        }
        for (sum, section) in
            self.gpu_sum
                .iter_mut()
                .zip([Section::Bloom, Section::Tonemap, Section::Fxaa])
        {
            *sum += f64::from(frame.ms[section as usize]);
        }
        self.stats.gpu_frames += 1;
        let n = self.stats.gpu_frames as f64;
        self.stats.gpu_ms = self.gpu_sum.map(|s| s / n);
    }

    /// `observe` -> `render_post` while a setting is on.
    pub fn observation(&self) -> Option<serde_json::Value> {
        let s = &self.settings;
        if !s.any() {
            return None;
        }
        let round = |v: f64| (v * 1000.0).round() / 1000.0;
        let t = &self.stats;
        Some(serde_json::json!({
            "antialiasing": if s.fxaa { "fxaa" } else { "none" },
            // Through text: f32 settings print as authored (0.4), not widened (0.4000000059604645).
            "hdr": serde_json::to_string(&s.hdr).ok().and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()),
            "target": if s.hdr.is_some() { "rgba16float" } else { "surface" },
            "target_bytes": self.targets.as_ref().map_or(0, |t| t.bytes),
            "bloom_mips": self.targets.as_ref().and_then(|t| t.bloom.as_ref()).map_or(0, |b| b.levels.len()),
            "target_builds": t.target_builds,
            "pipeline_ms": round(t.pipeline_ms),
            "gpu_frames": t.gpu_frames,
            "gpu_ms": if t.gpu_frames > 0 {
                serde_json::json!({"bloom": round(t.gpu_ms[0]), "tonemap": round(t.gpu_ms[1]), "fxaa": round(t.gpu_ms[2])})
            } else {
                serde_json::Value::Null
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_core::post::Bloom;

    #[test]
    fn bloom_levels_are_bounded_and_shrink_with_the_frame() {
        assert_eq!(bloom_mips(1920, 1080), 6);
        // 32, 16, 8 px levels.
        assert_eq!(bloom_mips(64, 64), 3);
        assert_eq!(bloom_mips(1, 1), 1);
        assert!(bloom_mips(16384, 16384) <= MAX_BLOOM_MIPS);
    }

    #[test]
    fn uniform_normalizes_the_level_weights_and_defaults_off() {
        let off = uniform(&Settings::default(), [0.0, 0.0, 1.0, 1.0], 6);
        assert_eq!(off.tone, [1.0, 0.0, 0.0, 0.0]);
        let s = Settings {
            hdr: Some(Hdr {
                exposure: 2.0,
                bloom: Some(Bloom {
                    threshold: 1.0,
                    intensity: 0.6,
                    radius: 0.5,
                }),
                ..Default::default()
            }),
            fxaa: false,
        };
        let u = uniform(&s, [0.5, 0.0, 0.5, 1.0], 3);
        // 1 + 0.5 + 0.25 = 1.75.
        assert!((u.bloom[3] - 0.6 / 1.75).abs() < 1e-6);
        assert_eq!(u.bloom[..3], [1.0, 0.5, 0.5]);
        assert_eq!(u.tone[..2], [2.0, 1.0]);
        // Radius 0: only the finest level, intensity unchanged.
        let mut tight = s;
        tight.hdr.as_mut().unwrap().bloom.as_mut().unwrap().radius = 0.0;
        assert_eq!(uniform(&tight, [0.0; 4], 6).bloom[3], 0.6);
    }

    #[test]
    fn viewports_stay_inside_the_target() {
        assert_eq!(
            viewport([0.0, 0.0, 1.0, 1.0], [960, 540]),
            [0.0, 0.0, 960.0, 540.0]
        );
        assert_eq!(
            viewport([0.5, 0.5, 0.5000001, 0.5], [7, 5]),
            [3.5, 2.5, 3.5, 2.5]
        );
        let tiny = viewport([0.999, 0.0, 0.0001, 1.0], [30, 17]);
        assert!(tiny[0] + tiny[2] <= 30.0 && tiny[2] >= 1.0 - 1e-6);
    }

    /// GPU time of each pass at 1920x1080 (`cargo test -p axiom-runtime --lib
    /// measure_post_passes -- --ignored --nocapture`): a headless device, the HDR target and the
    /// FXAA input filled with an 8 px checker (edges everywhere: FXAA's worst case), 300 frames,
    /// medians of the timestamp intervals. The shaders run the same in debug and release builds;
    /// only the CPU side differs.
    #[test]
    #[ignore]
    fn measure_post_passes_1920x1080() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: GpuTimer::features(adapter.features()),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .expect("device");
        let mut timer = GpuTimer::new(adapter.features(), device.features(), &queue);
        assert!(timer.unavailable().is_none(), "{:?}", timer.unavailable());
        timer.set_enabled(true);
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let (w, h) = (1920, 1080);
        let mut finish = Finish::new(&device, format);
        let pipelines = finish.post_inputs().build_pipelines(&device);
        finish.install(pipelines);
        let settings = Settings {
            hdr: Some(Hdr {
                bloom: Some(Bloom::default()),
                ..Default::default()
            }),
            fxaa: true,
        };
        finish.prepare(
            &device,
            &queue,
            settings,
            std::iter::once([0.0, 0.0, 1.0, 1.0]),
            w,
            h,
        );
        // An 8 px checker drawn into the HDR target (4.0: above the bloom threshold) and the
        // FXAA input (white / black); the targets are render attachments only.
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("checker"),
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn v(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                  let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                  return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0); }
                 @fragment fn f(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
                  let c = (u32(p.x) / 8u + u32(p.y) / 8u) % 2u;
                  return vec4<f32>(vec3<f32>(f32(c) * 4.0), 1.0); }"
                    .into(),
            ),
        });
        let checker = |view: &wgpu::TextureView, format: wgpu::TextureFormat| {
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("v"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("f"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
            drop(pass);
            queue.submit(std::iter::once(encoder.finish()));
        };
        checker(finish.hdr_view().unwrap(), HDR_FORMAT);
        checker(finish.ldr_view().unwrap(), format);
        let out = target(&device, "out", format, [w, h], 1).create_view(&Default::default());
        let tonemapped =
            target(&device, "tone mapped", format, [w, h], 1).create_view(&Default::default());
        let mut samples: [Vec<f32>; 3] = Default::default();
        for _ in 0..300 {
            let mut encoder = device.create_command_encoder(&Default::default());
            timer.begin(&device, &mut encoder);
            finish.resolve_view(
                &mut encoder,
                &mut timer,
                &tonemapped,
                0,
                [0.0, 0.0, 1.0, 1.0],
            );
            finish.fxaa(&mut encoder, &mut timer, &out);
            timer.end(&mut encoder);
            queue.submit(std::iter::once(encoder.finish()));
            timer.submitted();
            device.poll(wgpu::Maintain::Wait);
            timer.take(|f| {
                for (s, section) in
                    samples
                        .iter_mut()
                        .zip([Section::Bloom, Section::Tonemap, Section::Fxaa])
                {
                    s.push(f.ms[section as usize]);
                }
            });
        }
        let median = |v: &mut Vec<f32>| {
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        };
        let [b, tm, fx] = &mut samples;
        println!(
            "{} 1920x1080, {} measured frames, median GPU ms: bloom ({} levels) {:.3}, tone map {:.3}, fxaa {:.3}",
            adapter.get_info().name,
            b.len(),
            bloom_mips(w, h),
            median(b),
            median(tm),
            median(fx)
        );
        assert!(b.len() > 100);
    }

    #[test]
    fn shaders_validate() {
        for source in [include_str!("hdr.wgsl"), include_str!("fxaa.wgsl")] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
