//! Spot and point light shadow maps.
//!
//! Every frame with shadow-casting local lights renders one depth pass per active face:
//! up to `MAX_SPOT_SHADOWS` spot maps (one perspective map each, `SPOT_SHADOW_RESOLUTION`²) and
//! `MAX_POINT_SHADOWS` point lights as six 90° perspective faces (`POINT_SHADOW_RESOLUTION`²
//! each, a cube map stored as 2D array layers so the shader picks the face itself). The depth
//! passes reuse the sun shadow pipelines: each face writes its matrix into the `light_vp` of
//! an extra per-view globals slot after the camera slots, so no pipeline variant is added.
//! Textures are allocated on the first frame that needs them (1×1 placeholders before).
//!
//! Casters (`plan`): enabled, scene-active spot / point lights with `shadows: true`, in
//! authored order, up to the budget; more (only possible through an uncommitted editor draft,
//! compile rejects them) are lit unshadowed and counted in `ignored`.
use crate::presentation::SceneLight;
use axiom_core::{
    LightKind, MAX_POINT_SHADOWS, MAX_SPOT_SHADOWS, POINT_SHADOW_RESOLUTION, SPOT_SHADOW_RESOLUTION,
};
use glam::{Mat4, Vec3};

/// Shadow faces: the spot maps, then six per point light.
pub const FACES: usize = MAX_SPOT_SHADOWS + 6 * MAX_POINT_SHADOWS;
/// First per-view globals slot used by the face matrices (after the camera slots).
pub(crate) const SLOT_BASE: usize = axiom_core::MAX_CAMERAS;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// PCF reads one texel around the centre: widen each frustum by 3 texels so the kernel of a
/// point just inside the cone (or a cube face edge) stays inside its own map.
const MARGIN_TEXELS: f32 = 3.0;

/// The cube faces in layer order: (forward, up).
const CUBE: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::NEG_Y),
    (Vec3::NEG_X, Vec3::NEG_Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::Z, Vec3::NEG_Y),
    (Vec3::NEG_Z, Vec3::NEG_Y),
];

/// This frame's shadow casters and face matrices (pure CPU; `lighting::pack`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalShadowPlan {
    pub matrices: [Mat4; FACES],
    /// `scene_lights` index of each spot slot / point slot.
    pub spot: [Option<usize>; MAX_SPOT_SHADOWS],
    pub point: [Option<usize>; MAX_POINT_SHADOWS],
    /// World size of one shadow texel per meter from the light, per spot / point slot.
    pub spot_texel: [f32; MAX_SPOT_SHADOWS],
    pub point_texel: [f32; MAX_POINT_SHADOWS],
    /// Casters beyond the budget (drawn unshadowed).
    pub ignored: usize,
}

impl Default for LocalShadowPlan {
    fn default() -> Self {
        Self {
            matrices: [Mat4::IDENTITY; FACES],
            spot: [None; MAX_SPOT_SHADOWS],
            point: [None; MAX_POINT_SHADOWS],
            spot_texel: [0.0; MAX_SPOT_SHADOWS],
            point_texel: [0.0; MAX_POINT_SHADOWS],
            ignored: 0,
        }
    }
}

impl LocalShadowPlan {
    /// Shader parameters `(1 + first face, texel size per meter)` of `scene_lights[i]` when it
    /// casts (`LocalLight.params.zw`; shadow_local.wgsl).
    pub fn params(&self, i: usize) -> Option<[f32; 2]> {
        if let Some(slot) = self.spot.iter().position(|s| *s == Some(i)) {
            return Some([1.0 + slot as f32, self.spot_texel[slot]]);
        }
        let slot = self.point.iter().position(|s| *s == Some(i))?;
        Some([
            1.0 + (MAX_SPOT_SHADOWS + 6 * slot) as f32,
            self.point_texel[slot],
        ])
    }
    /// Faces whose depth pass runs this frame.
    pub fn active(&self) -> [bool; FACES] {
        let mut active = [false; FACES];
        for (slot, light) in self.spot.iter().enumerate() {
            active[slot] = light.is_some();
        }
        for (slot, light) in self.point.iter().enumerate() {
            let first = MAX_SPOT_SHADOWS + 6 * slot;
            active[first..first + 6].fill(light.is_some());
        }
        active
    }
    pub fn face_count(&self) -> usize {
        self.active().iter().filter(|a| **a).count()
    }
}

/// Picks the casters (indices into `lights`) and builds their matrices.
pub fn plan(lights: &[SceneLight]) -> LocalShadowPlan {
    let mut plan = LocalShadowPlan::default();
    let (mut spots, mut points) = (0, 0);
    for (i, s) in lights.iter().enumerate() {
        if !s.light.shadows {
            continue;
        }
        match s.light.kind {
            LightKind::Spot if spots < MAX_SPOT_SHADOWS => {
                plan.spot[spots] = Some(i);
                spots += 1;
            }
            LightKind::Point if points < MAX_POINT_SHADOWS => {
                plan.point[points] = Some(i);
                points += 1;
            }
            LightKind::Spot | LightKind::Point => plan.ignored += 1,
            _ => {}
        }
    }
    for slot in 0..MAX_SPOT_SHADOWS {
        if let Some(i) = plan.spot[slot] {
            (plan.matrices[slot], plan.spot_texel[slot]) = spot_matrix(&lights[i]);
        }
    }
    for slot in 0..MAX_POINT_SHADOWS {
        if let Some(i) = plan.point[slot] {
            let first = MAX_SPOT_SHADOWS + 6 * slot;
            let (faces, texel) = point_matrices(&lights[i]);
            plan.matrices[first..first + 6].copy_from_slice(&faces);
            plan.point_texel[slot] = texel;
        }
    }
    plan
}

fn near_far(range: f32) -> (f32, f32) {
    ((range * 0.01).clamp(0.01, 0.1).min(range * 0.5), range)
}

/// Spot shadow matrix (perspective along the light's direction covering the outer cone plus
/// the PCF margin) and the world size of one texel per meter of distance.
pub fn spot_matrix(s: &SceneLight) -> (Mat4, f32) {
    let outer = s.light.spot.map_or(90.0, |c| c.outer_degrees).min(170.0);
    let res = SPOT_SHADOW_RESOLUTION as f32;
    let tan_half = (outer.to_radians() * 0.5).tan() * (1.0 + 2.0 * MARGIN_TEXELS / res);
    let (near, far) = near_far(s.light.range.unwrap_or(1.0));
    let up = if s.direction.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let m = Mat4::perspective_rh(2.0 * tan_half.atan(), 1.0, near, far)
        * Mat4::look_at_rh(s.position, s.position + s.direction, up);
    (m, 2.0 * tan_half / res)
}

/// The six cube face matrices of a point light (layer order of `CUBE`) and the world texel
/// size per meter.
pub fn point_matrices(s: &SceneLight) -> ([Mat4; 6], f32) {
    let res = POINT_SHADOW_RESOLUTION as f32;
    let tan_half = 1.0 + 2.0 * MARGIN_TEXELS / res;
    let (near, far) = near_far(s.light.range.unwrap_or(1.0));
    let projection = Mat4::perspective_rh(2.0 * tan_half.atan(), 1.0, near, far);
    let faces = CUBE
        .map(|(forward, up)| projection * Mat4::look_at_rh(s.position, s.position + forward, up));
    (faces, 2.0 * tan_half / res)
}

/// What `observe` reports (`metrics.local_shadows`); ids are copied only when they change.
#[derive(Debug, Default)]
pub struct LocalShadowStats {
    pub casters: Vec<(String, &'static str)>,
    pub faces: usize,
    pub ignored: usize,
}

impl LocalShadowStats {
    fn update(&mut self, plan: &LocalShadowPlan, lights: &[SceneLight]) {
        let mut ids = plan
            .spot
            .iter()
            .flatten()
            .map(|i| (*i, "spot"))
            .chain(plan.point.iter().flatten().map(|i| (*i, "point")));
        let same = {
            let mut current = self.casters.iter();
            let mut ids = ids.clone();
            loop {
                match (current.next(), ids.next()) {
                    (None, None) => break true,
                    (Some((id, k)), Some((i, kind))) if *id == lights[i].id && *k == kind => {}
                    _ => break false,
                }
            }
        };
        if !same {
            self.casters = ids
                .by_ref()
                .map(|(i, k)| (lights[i].id.clone(), k))
                .collect();
        }
        self.faces = plan.face_count();
        self.ignored = plan.ignored;
    }

    pub fn observation(&self) -> Option<serde_json::Value> {
        (!self.casters.is_empty() || self.ignored > 0).then(|| {
            serde_json::json!({
                "casters": self.casters.iter().map(|(id, kind)| serde_json::json!({"id":id,"kind":kind})).collect::<Vec<_>>(),
                "faces": self.faces,
                "ignored": self.ignored,
                "resolution": {"spot": SPOT_SHADOW_RESOLUTION, "point_face": POINT_SHADOW_RESOLUTION},
                "rendered": "every frame",
            })
        })
    }
}

/// GPU side: the face matrix uniform (binding 3) and the two depth arrays (bindings 4, 5).
pub(crate) struct LocalShadowGpu {
    matrices: wgpu::Buffer,
    spot: wgpu::TextureView,
    point: wgpu::TextureView,
    /// One render view per face; empty while the 1×1 placeholders are bound.
    faces: Vec<wgpu::TextureView>,
    pub stats: LocalShadowStats,
}

fn depth_array(device: &wgpu::Device, size: u32, layers: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

impl LocalShadowGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        let placeholder = |label| array_view(&depth_array(device, 1, 1, label));
        Self {
            matrices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("local shadow matrices"),
                size: (FACES * std::mem::size_of::<Mat4>()) as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            spot: placeholder("spot shadow placeholder"),
            point: placeholder("point shadow placeholder"),
            faces: vec![],
            stats: LocalShadowStats::default(),
        }
    }

    /// Bindings 3-5 of the scene and mesh camera groups.
    pub fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
        let depth = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        [
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            depth(4),
            depth(5),
        ]
    }

    pub fn bind_entries(&self) -> [wgpu::BindGroupEntry<'_>; 3] {
        [
            wgpu::BindGroupEntry {
                binding: 3,
                resource: self.matrices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&self.spot),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&self.point),
            },
        ]
    }

    /// Allocates the real maps the first time a frame has casters; true when the bind groups
    /// that use `bind_entries` must be rebuilt.
    pub fn ensure(&mut self, device: &wgpu::Device, plan: &LocalShadowPlan) -> bool {
        if !self.faces.is_empty() || plan.face_count() == 0 {
            return false;
        }
        let spot = depth_array(
            device,
            SPOT_SHADOW_RESOLUTION,
            MAX_SPOT_SHADOWS as u32,
            "spot shadows",
        );
        let point = depth_array(
            device,
            POINT_SHADOW_RESOLUTION,
            6 * MAX_POINT_SHADOWS as u32,
            "point shadows",
        );
        let layer = |texture: &wgpu::Texture, layer: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        self.faces = (0..MAX_SPOT_SHADOWS as u32)
            .map(|l| layer(&spot, l))
            .chain((0..6 * MAX_POINT_SHADOWS as u32).map(|l| layer(&point, l)))
            .collect();
        self.spot = array_view(&spot);
        self.point = array_view(&point);
        true
    }

    /// Writes the face matrices (binding 3) and each active face's `light_vp` into its globals
    /// slot (`globals_stride` bytes apart, `light_vp_offset` into the slot).
    pub fn upload(
        &mut self,
        queue: &wgpu::Queue,
        globals: &wgpu::Buffer,
        globals_stride: u64,
        light_vp_offset: u64,
        plan: &LocalShadowPlan,
        lights: &[SceneLight],
    ) {
        self.stats.update(plan, lights);
        if plan.face_count() == 0 {
            return;
        }
        let matrices = plan.matrices.map(|m| m.to_cols_array_2d());
        queue.write_buffer(&self.matrices, 0, bytemuck::cast_slice(&matrices));
        for (face, active) in plan.active().into_iter().enumerate() {
            if active {
                queue.write_buffer(
                    globals,
                    (SLOT_BASE + face) as u64 * globals_stride + light_vp_offset,
                    bytemuck::cast_slice(&matrices[face]),
                );
            }
        }
    }

    /// One depth pass per active face; `draw` records the casters with the face's globals
    /// slot (dynamic offset). Returns the number of passes.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &LocalShadowPlan,
        globals_stride: u32,
        mut draw: impl FnMut(&mut wgpu::RenderPass<'_>, u32),
    ) -> usize {
        let mut passes = 0;
        for (face, active) in plan.active().into_iter().enumerate() {
            let Some(view) = self.faces.get(face).filter(|_| active) else {
                continue;
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("local light depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            draw(&mut pass, (SLOT_BASE + face) as u32 * globals_stride);
            passes += 1;
        }
        passes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn light(id: &str, v: serde_json::Value, position: Vec3, direction: Vec3) -> SceneLight {
        let light: axiom_core::RenderLight = serde_json::from_value(v).unwrap();
        light.validate().unwrap();
        SceneLight {
            id: id.into(),
            light,
            position,
            direction: direction.normalize(),
            right: Vec3::X,
        }
    }
    fn spot(id: &str, shadows: bool) -> SceneLight {
        light(
            id,
            json!({"kind":"spot","range":10,"shadows":shadows,"spot":{"inner_degrees":30,"outer_degrees":60}}),
            Vec3::new(0.0, 5.0, 0.0),
            -Vec3::Y,
        )
    }
    fn point(id: &str, shadows: bool) -> SceneLight {
        light(
            id,
            json!({"kind":"point","range":8,"shadows":shadows}),
            Vec3::new(1.0, 2.0, 3.0),
            -Vec3::Z,
        )
    }
    fn ndc(m: Mat4, p: Vec3) -> Vec3 {
        let c = m * p.extend(1.0);
        c.truncate() / c.w
    }

    #[test]
    fn casters_follow_authored_order_within_the_budget() {
        let lights = [
            point("p0", false),
            spot("s0", true),
            point("p1", true),
            spot("s1", false),
            spot("s2", true),
            spot("s3", true),
            point("p2", true),
        ];
        let plan = plan(&lights);
        assert_eq!(plan.spot, [Some(1), Some(4)]);
        assert_eq!(plan.point, [Some(2)]);
        assert_eq!(plan.ignored, 2, "s3 and p2 exceed the budget");
        assert_eq!(plan.active(), [true; FACES]);
        assert_eq!(plan.params(0), None);
        assert_eq!(plan.params(1), Some([1.0, spot_matrix(&lights[1]).1]));
        assert_eq!(plan.params(4).unwrap()[0], 2.0);
        assert_eq!(plan.params(2), Some([3.0, point_matrices(&lights[2]).1]));
        assert_eq!(plan.params(5), None, "over budget: unshadowed");
        assert_eq!(plan.matrices[0], spot_matrix(&lights[1]).0);
        assert_eq!(plan.matrices[2..8], point_matrices(&lights[2]).0);
        let none = super::plan(&lights[..1]);
        assert_eq!(none.face_count(), 0);
        assert_eq!(none, LocalShadowPlan::default());
        // Only a spot: one face.
        let one = super::plan(&lights[1..2]);
        assert_eq!(one.face_count(), 1);
        assert!(one.active()[0]);
    }

    #[test]
    fn spot_matrix_covers_the_cone_and_the_range() {
        let s = spot("s", true);
        let (m, texel) = spot_matrix(&s);
        // On the axis at the range: centre of the map, far plane.
        let far = ndc(m, s.position + s.direction * 10.0);
        assert!(far.x.abs() < 1e-4 && far.y.abs() < 1e-4 && (far.z - 1.0).abs() < 1e-4);
        // The cone edge (30 degrees off axis) is inside the map by the PCF margin.
        let edge = s.position + (s.direction + Vec3::X * 30f32.to_radians().tan()) * 5.0;
        let e = ndc(m, edge);
        assert!(e.x.abs() < 1.0 && e.x.abs() > 0.98, "{e}");
        assert!((0.0..1.0).contains(&e.z));
        // Texel size per meter: the map spans 2 tan(30°) (+ margin) over 1024 texels.
        assert!((texel - 2.0 * 30f32.to_radians().tan() / 1024.0).abs() < 1e-5);
    }

    #[test]
    fn point_faces_cover_every_direction_once() {
        let p = point("p", true);
        let (faces, texel) = point_matrices(&p);
        assert!((texel - 2.0 / 512.0).abs() < 1e-4);
        for (k, (forward, _)) in CUBE.iter().enumerate() {
            let c = ndc(faces[k], p.position + *forward * 4.0);
            assert!(c.x.abs() < 1e-4 && c.y.abs() < 1e-4, "face {k}: {c}");
            assert!((0.0..1.0).contains(&c.z));
        }
        // The shader picks the face of the major axis; a direction near a cube edge lands
        // inside that face's map.
        let d = Vec3::new(1.0, 0.99, 0.2);
        let c = ndc(faces[0], p.position + d * 3.0);
        assert!(c.x.abs() <= 1.0 && c.y.abs() <= 1.0, "{c}");
    }

    #[test]
    fn stats_copy_ids_only_when_they_change() {
        let lights = [spot("a", true), point("b", true)];
        let plan = plan(&lights);
        let mut stats = LocalShadowStats::default();
        assert!(stats.observation().is_none());
        stats.update(&plan, &lights);
        assert_eq!(
            stats.casters,
            [("a".to_owned(), "spot"), ("b".to_owned(), "point")]
        );
        assert_eq!(stats.faces, 7);
        let ptr = stats.casters.as_ptr();
        stats.update(&plan, &lights);
        assert_eq!(
            ptr,
            stats.casters.as_ptr(),
            "unchanged casters are not copied"
        );
        let o = stats.observation().unwrap();
        assert_eq!(o["casters"][1]["id"], "b");
        assert_eq!(o["faces"], 7);
        let none = super::plan(&[]);
        stats.update(&none, &[]);
        assert!(stats.observation().is_none());
    }
}
