//! Packs authored `render.light` entities, VFX particle lights and `render.environment`
//! into the per-view GPU globals. Pure CPU code, so the packing rules are unit-tested.
//!
//! Rules:
//! - Suns: the enabled directional lights in authored entity order (budget 4). Only the
//!   first can cast the shadow map (when its `shadows` is true). With no enabled
//!   directional light the legacy sun (direction, colour, shadow matrix) is used unchanged.
//! - Local lights (32 GPU slots): authored point/spot/area lights first, in entity order,
//!   then VFX lights in emitter order. Lights beyond 32 are dropped (counted in `dropped`).
//! - Ambient/fog: `render.environment` when authored, otherwise the legacy constants
//!   (hemisphere ambient tinted by `render.settings.accent`, exp² fog k = 0.00012).
use crate::presentation::RenderFrame;
use axiom_core::{
    AreaShape, LightKind, Sky, DEFAULT_AMBIENT_GROUND, DEFAULT_AMBIENT_SKY, DEFAULT_FOG_COLOR,
    DEFAULT_FOG_K, DEFAULT_SUN_COLOR, DEFAULT_SUN_TOWARD,
};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

pub const SCENE_WGSL: &str = concat!(
    include_str!("lighting.wgsl"),
    include_str!("shadow.wgsl"),
    include_str!("scene.wgsl")
);
pub const MESH_WGSL: &str = concat!(
    include_str!("lighting.wgsl"),
    include_str!("shadow.wgsl"),
    include_str!("mesh.wgsl")
);
pub const VFX_DRAW_WGSL: &str =
    concat!(include_str!("lighting.wgsl"), include_str!("vfx_draw.wgsl"));

pub const GPU_LOCAL_LIGHTS: usize = 32;
pub const GPU_SUNS: usize = 4;

pub const KIND_POINT: f32 = 0.0;
pub const KIND_SPOT: f32 = 1.0;
pub const KIND_RECT: f32 = 2.0;
pub const KIND_DISK: f32 = 3.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuLocalLight {
    pub position_range: [f32; 4],
    pub color_intensity: [f32; 4],
    pub direction_kind: [f32; 4],
    pub params: [f32; 4],
    pub axis: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuSun {
    pub direction_shadow: [f32; 4],
    pub color: [f32; 4],
    pub specular: [f32; 4],
}

pub const SKY_NONE: f32 = 0.0;
pub const SKY_GRADIENT: f32 = 1.0;
pub const SKY_PROCEDURAL: f32 = 2.0;

/// The authored sky reduced to what the shader evaluates (`axiom_sky` in lighting.wgsl; the
/// same formula as [`SkyParams::radiance`]). Procedural colours depend only on the sun
/// elevation, so they are resolved here once per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyParams {
    /// SKY_NONE (legacy screen gradient), SKY_GRADIENT or SKY_PROCEDURAL.
    pub mode: f32,
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
    /// Gradient: `pow(up, shape)`; procedural: haze `1 - pow(1 - up, shape)`.
    pub shape: f32,
    /// Intensity (gradient) or exposure (procedural).
    pub scale: f32,
    /// cos of the sun disk half-angle; > 1 = no disk.
    pub sun_cos: f32,
    /// Strength of the glow around the sun.
    pub haze: f32,
    pub sun_toward: Vec3,
    pub sun_color: [f32; 3],
}

impl SkyParams {
    pub const NONE: SkyParams = SkyParams {
        mode: SKY_NONE,
        zenith: [0.0; 3],
        horizon: [0.0; 3],
        ground: [0.0; 3],
        shape: 1.0,
        scale: 1.0,
        sun_cos: 2.0,
        haze: 0.0,
        sun_toward: Vec3::Y,
        sun_color: [0.0; 3],
    };

    pub fn new(sky: Option<&Sky>, sun_toward: Vec3, sun_color: [f32; 3]) -> Self {
        let smooth = |a: f32, b: f32, x: f32| {
            let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let mix = |a: [f32; 3], b: [f32; 3], t: f32| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
        match sky {
            None => Self::NONE,
            Some(Sky::Gradient {
                zenith_color,
                horizon_color,
                ground_color,
                exponent,
                intensity,
            }) => SkyParams {
                mode: SKY_GRADIENT,
                zenith: *zenith_color,
                horizon: *horizon_color,
                ground: *ground_color,
                shape: *exponent,
                scale: *intensity,
                ..Self::NONE
            },
            Some(Sky::Procedural {
                sun_size_degrees,
                atmosphere,
                ground_color,
                exposure,
            }) => {
                let e = sun_toward.y;
                // 0 at night, 1 in daylight; `warm` peaks while the sun is at the horizon.
                let day = smooth(-0.12, 0.25, e);
                let warm = (1.0 - smooth(0.0, 0.35, e)) * smooth(-0.18, 0.0, e);
                let zenith = mix([0.004, 0.007, 0.018], [0.10, 0.27, 0.62], day);
                let horizon = mix([0.02, 0.028, 0.045], [0.56, 0.70, 0.86], day);
                let horizon = mix(horizon, [0.92, 0.50, 0.24], warm * atmosphere.min(1.0));
                let lit = 0.12 + 0.88 * day;
                SkyParams {
                    mode: SKY_PROCEDURAL,
                    zenith,
                    horizon,
                    ground: ground_color.map(|c| c * lit),
                    shape: 4.0 / atmosphere.max(0.05),
                    scale: *exposure,
                    sun_cos: if *sun_size_degrees > 0.0 {
                        (sun_size_degrees.to_radians() * 0.5).cos()
                    } else {
                        2.0
                    },
                    haze: 0.3 * atmosphere * (day + warm).min(1.0),
                    sun_toward,
                    sun_color,
                }
            }
        }
    }

    /// Sky radiance toward unit direction `d` (no clamp). `disk` adds the sun disk.
    pub fn radiance(&self, d: Vec3, disk: bool) -> [f32; 3] {
        if self.mode == SKY_NONE {
            return [0.0; 3];
        }
        let y = d.y.clamp(-1.0, 1.0);
        let mut c: [f32; 3] = if y >= 0.0 {
            let t = if self.mode == SKY_GRADIENT {
                y.powf(self.shape)
            } else {
                1.0 - (1.0 - y).powf(self.shape)
            };
            std::array::from_fn(|i| self.horizon[i] + (self.zenith[i] - self.horizon[i]) * t)
        } else {
            let t = ((-y) / 0.17).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            std::array::from_fn(|i| self.horizon[i] + (self.ground[i] - self.horizon[i]) * t)
        };
        for v in &mut c {
            *v *= self.scale;
        }
        if self.mode == SKY_PROCEDURAL {
            let cos = d.dot(self.sun_toward).max(0.0);
            let above = ((y + 0.02) / 0.04).clamp(0.0, 1.0);
            let mut glow = cos.powi(12) * self.haze * above;
            if disk && self.sun_cos <= 1.0 {
                let t = ((cos - (self.sun_cos - 0.0004)) / 0.0005).clamp(0.0, 1.0);
                glow += t * t * (3.0 - 2.0 * t) * 12.0 * above;
            }
            for (v, s) in c.iter_mut().zip(self.sun_color) {
                *v += s * glow * self.scale;
            }
        }
        c
    }

    /// Irradiance / π of the sky (sun disk excluded) for normals up, horizontal (mean of four
    /// azimuths) and down: the `ambient: sky` trilight. Fixed 16 x 16 direction grid (equal
    /// solid angle bands), no allocation.
    pub fn trilight(&self) -> [[f32; 3]; 3] {
        const AZIMUTHS: usize = 16;
        const BANDS: usize = 16;
        let mut out = [[0.0_f32; 3]; 3];
        let mut weight = [0.0_f32; 3];
        let horizontal = [Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z];
        for band in 0..BANDS {
            // Equal-area bands in y = sin(elevation): every sample covers the same solid angle.
            let y = -1.0 + (band as f32 + 0.5) * 2.0 / BANDS as f32;
            let r = (1.0 - y * y).max(0.0).sqrt();
            for a in 0..AZIMUTHS {
                let phi = (a as f32 + 0.5) * std::f32::consts::TAU / AZIMUTHS as f32;
                let d = Vec3::new(r * phi.cos(), y, r * phi.sin());
                let l = self.radiance(d, false);
                let cosines = [
                    y.max(0.0),
                    horizontal.iter().map(|n| n.dot(d).max(0.0)).sum::<f32>() * 0.25,
                    (-y).max(0.0),
                ];
                for (k, cos) in cosines.into_iter().enumerate() {
                    weight[k] += cos;
                    for i in 0..3 {
                        out[k][i] += l[i] * cos;
                    }
                }
            }
        }
        for k in 0..3 {
            for v in &mut out[k] {
                *v /= weight[k].max(1e-6);
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct PackedLighting {
    pub locals: [GpuLocalLight; GPU_LOCAL_LIGHTS],
    pub local_count: usize,
    /// Local lights that did not fit the 32 GPU slots.
    pub dropped: usize,
    pub suns: [GpuSun; GPU_SUNS],
    pub sun_count: usize,
    /// Sun 0 renders and samples the shadow map.
    pub shadow: bool,
    pub light_vp: Mat4,
    /// Sky, equator, ground (rgb); `ambient[1][3]` = 1 when lighting is authored.
    pub ambient: [[f32; 4]; 3],
    pub fog: [f32; 4],
    /// A directional light or `render.environment` is authored (meshes switch from the
    /// legacy flat key light to the shared lighting model).
    pub authored: bool,
    /// The authored sky (`SkyParams::NONE` = legacy screen-space background).
    pub sky: SkyParams,
}

impl PackedLighting {
    /// The four sky vectors of the per-view globals (see `Globals` in lighting.wgsl).
    pub fn sky_uniform(&self) -> [[f32; 4]; 4] {
        let s = &self.sky;
        [
            [s.zenith[0], s.zenith[1], s.zenith[2], s.mode],
            [s.horizon[0], s.horizon[1], s.horizon[2], s.shape],
            [s.ground[0], s.ground[1], s.ground[2], s.scale],
            [s.sun_cos, s.haze, 0.0, 0.0],
        ]
    }
}

/// The legacy sun shadow matrix (eye (-45, 75, 35) looking at the origin).
pub fn legacy_light_vp(extent: f32) -> Mat4 {
    Mat4::orthographic_rh(-extent, extent, -extent, extent, 1.0, 170.0)
        * Mat4::look_at_rh(Vec3::new(-45.0, 75.0, 35.0), Vec3::ZERO, Vec3::Y)
}

/// Orthographic shadow matrix for a sun shining from `toward` (unit, pointing at the
/// light), centred on the world origin like the legacy matrix.
pub fn sun_light_vp(toward: Vec3, extent: f32) -> Mat4 {
    let up = if toward.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Mat4::orthographic_rh(-extent, extent, -extent, extent, 1.0, 190.0)
        * Mat4::look_at_rh(toward * 95.0, Vec3::ZERO, up)
}

pub fn pack(frame: &RenderFrame) -> PackedLighting {
    let extent = frame.settings.shadow_extent;
    let mut suns = [GpuSun::zeroed(); GPU_SUNS];
    let directional: Vec<_> = frame
        .scene_lights
        .iter()
        .filter(|l| l.light.kind == LightKind::Directional)
        .take(GPU_SUNS)
        .collect();
    let (sun_count, shadow, light_vp) = if directional.is_empty() {
        let toward = Vec3::from_array(DEFAULT_SUN_TOWARD).normalize();
        let [r, g, b] = DEFAULT_SUN_COLOR;
        suns[0] = GpuSun {
            direction_shadow: [toward.x, toward.y, toward.z, 1.0],
            color: [r, g, b, 1.0],
            specular: [1.0; 4],
        };
        (1, true, legacy_light_vp(extent))
    } else {
        for (out, l) in suns.iter_mut().zip(&directional) {
            let toward = -l.direction;
            let c = l.light.color.map(|c| c * l.light.intensity);
            *out = GpuSun {
                direction_shadow: [toward.x, toward.y, toward.z, 0.0],
                color: [c[0], c[1], c[2], l.light.intensity],
                specular: [c[0], c[1], c[2], 0.0],
            };
        }
        let shadow = directional[0].light.shadows;
        suns[0].direction_shadow[3] = f32::from(u8::from(shadow));
        (
            directional.len(),
            shadow,
            sun_light_vp(-directional[0].direction, extent),
        )
    };

    let mut locals = [GpuLocalLight::zeroed(); GPU_LOCAL_LIGHTS];
    let authored_locals = frame
        .scene_lights
        .iter()
        .filter(|l| l.light.kind != LightKind::Directional)
        .map(|s| {
            let l = &s.light;
            let (kind, params) = match l.kind {
                LightKind::Spot => {
                    let cone = l.spot.expect("validated spot");
                    let outer = (cone.outer_degrees.to_radians() * 0.5).cos();
                    // smoothstep needs inner > outer; equal angles become a hard edge.
                    let inner = (cone.inner_degrees.to_radians() * 0.5)
                        .cos()
                        .max(outer + 1e-4);
                    (KIND_SPOT, [inner, outer, 0.0, 0.0])
                }
                LightKind::Area => {
                    let area = l.area.expect("validated area");
                    let kind = match area.shape {
                        AreaShape::Rect => KIND_RECT,
                        AreaShape::Disk => KIND_DISK,
                    };
                    (kind, [0.0, 0.0, area.size[0] * 0.5, area.size[1] * 0.5])
                }
                _ => (KIND_POINT, [0.0; 4]),
            };
            GpuLocalLight {
                position_range: [
                    s.position.x,
                    s.position.y,
                    s.position.z,
                    l.range.unwrap_or(1.0),
                ],
                color_intensity: [l.color[0], l.color[1], l.color[2], l.intensity],
                direction_kind: [s.direction.x, s.direction.y, s.direction.z, kind],
                params,
                axis: [
                    s.right.x,
                    s.right.y,
                    s.right.z,
                    l.source_radius.unwrap_or(0.0),
                ],
            }
        });
    let vfx = frame.lights.iter().map(|light| GpuLocalLight {
        position_range: [
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
        direction_kind: [0.0, 0.0, 0.0, KIND_POINT],
        params: [0.0; 4],
        axis: [0.0; 4],
    });
    let mut total = 0;
    for light in authored_locals.chain(vfx) {
        if let Some(slot) = locals.get_mut(total) {
            *slot = light;
        }
        total += 1;
    }
    let local_count = total.min(GPU_LOCAL_LIGHTS);

    let env = &frame.environment;
    let authored = !directional.is_empty()
        || env.ambient.is_some()
        || env.fog.is_some()
        || env.sky.is_some();
    // The procedural sky follows sun 0 (the first directional light or the default sun).
    let sky_params = SkyParams::new(
        env.sky.as_ref(),
        Vec3::from_slice(&suns[0].direction_shadow[..3]),
        [suns[0].color[0], suns[0].color[1], suns[0].color[2]],
    );
    let sky_light = env.sky_ambient().map(|k| {
        let [s, e, g] = sky_params.trilight();
        [s, e, g].map(|c| c.map(|v| v * k))
    });
    let [sky, equator, ground] = sky_light.or_else(|| env.trilight()).unwrap_or_else(|| {
        let a = frame.settings.accent;
        let tint = [0.85 + a[0] * 0.15, 0.85 + a[1] * 0.15, 0.85 + a[2] * 0.15];
        let sky: [f32; 3] = std::array::from_fn(|i| DEFAULT_AMBIENT_SKY[i] * tint[i]);
        let ground: [f32; 3] = std::array::from_fn(|i| DEFAULT_AMBIENT_GROUND[i] * tint[i]);
        let equator = std::array::from_fn(|i| (sky[i] + ground[i]) * 0.5);
        [sky, equator, ground]
    });
    let fog = match env.fog {
        Some(f) => [f.color[0], f.color[1], f.color[2], f.density * f.density],
        None => [
            DEFAULT_FOG_COLOR[0],
            DEFAULT_FOG_COLOR[1],
            DEFAULT_FOG_COLOR[2],
            DEFAULT_FOG_K,
        ],
    };
    PackedLighting {
        locals,
        local_count,
        dropped: total - local_count,
        suns,
        sun_count,
        shadow,
        light_vp,
        ambient: [
            [sky[0], sky[1], sky[2], sun_count as f32],
            [
                equator[0],
                equator[1],
                equator[2],
                f32::from(u8::from(authored)),
            ],
            [ground[0], ground[1], ground[2], 0.0],
        ],
        fog,
        authored,
        sky: sky_params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::SceneLight;
    use axiom_core::{RenderEnvironment, RenderLight, RenderSettings};
    use serde_json::json;

    fn frame() -> RenderFrame {
        RenderFrame {
            draw_world: true,
            tick: 0,
            settings: RenderSettings::default(),
            instances: vec![],
            views: vec![],
            meshes: vec![],
            shaders: Default::default(),
            cube_materials: vec![],
            vfx: Default::default(),
            lights: vec![],
            scene_lights: vec![],
            environment: RenderEnvironment::default(),
            postfx: vec![],
        }
    }
    fn light(id: &str, v: serde_json::Value, position: Vec3, direction: Vec3) -> SceneLight {
        let light: RenderLight = serde_json::from_value(v).unwrap();
        light.validate().unwrap();
        SceneLight {
            id: id.into(),
            light,
            position,
            direction: direction.normalize(),
            right: Vec3::X,
        }
    }
    fn vfx(x: f32) -> crate::vfx::Light {
        crate::vfx::Light {
            position: Vec3::new(x, 1.0, 0.0),
            radius: 2.0,
            color: [1.0, 0.5, 0.2],
            intensity: 3.0,
            attachment: None,
        }
    }

    #[test]
    fn defaults_reproduce_the_legacy_constants() {
        let p = pack(&frame());
        assert!(!p.authored);
        assert_eq!(p.sun_count, 1);
        assert!(p.shadow);
        let toward = Vec3::new(-0.45, 0.85, 0.35).normalize();
        assert_eq!(
            p.suns[0].direction_shadow,
            [toward.x, toward.y, toward.z, 1.0]
        );
        assert_eq!(p.suns[0].color, [1.12, 1.0, 0.82, 1.0]);
        assert_eq!(p.suns[0].specular, [1.0; 4]);
        assert_eq!(p.light_vp, legacy_light_vp(33.0));
        assert_eq!(p.fog, [0.055, 0.09, 0.12, 0.00012]);
        // Legacy ambient: mix(ground, sky, n.y*0.5+0.5) * (0.85 + accent*0.15).
        let a = RenderSettings::default().accent;
        for (ny, expect) in [(1.0_f32, 1.0_f32), (-1.0, 0.0), (0.0, 0.5), (0.4, 0.7)] {
            let trilight = |i: usize| {
                if ny >= 0.0 {
                    p.ambient[1][i] + (p.ambient[0][i] - p.ambient[1][i]) * ny
                } else {
                    p.ambient[1][i] + (p.ambient[2][i] - p.ambient[1][i]) * -ny
                }
            };
            for i in 0..3 {
                let legacy = (DEFAULT_AMBIENT_GROUND[i]
                    + (DEFAULT_AMBIENT_SKY[i] - DEFAULT_AMBIENT_GROUND[i]) * expect)
                    * (0.85 + a[i] * 0.15);
                assert!((trilight(i) - legacy).abs() < 1e-6, "n.y={ny} channel {i}");
            }
        }
        assert_eq!(p.local_count, 0);
    }

    #[test]
    fn first_directional_light_is_the_shadowed_sun() {
        let mut f = frame();
        f.scene_lights = vec![
            light(
                "fill",
                json!({"kind":"point","range":4}),
                Vec3::ONE,
                -Vec3::Z,
            ),
            light(
                "moon",
                json!({"kind":"directional","color":[0.5,0.5,1],"intensity":0.4,"shadows":true}),
                Vec3::ZERO,
                Vec3::new(0.0, -1.0, -1.0),
            ),
            light(
                "rim",
                json!({"kind":"directional","shadows":true}),
                Vec3::ZERO,
                Vec3::new(1.0, -1.0, 0.0),
            ),
        ];
        let p = pack(&f);
        assert!(p.authored);
        assert_eq!(p.sun_count, 2);
        assert!(p.shadow);
        let toward = Vec3::new(0.0, 1.0, 1.0).normalize();
        assert!((Vec3::from_slice(&p.suns[0].direction_shadow[..3]) - toward).length() < 1e-6);
        assert_eq!(p.suns[0].direction_shadow[3], 1.0);
        assert_eq!(p.suns[0].color[..3], [0.2, 0.2, 0.4]);
        // Others are unshadowed even when they ask for shadows.
        assert_eq!(p.suns[1].direction_shadow[3], 0.0);
        assert_eq!(p.light_vp, sun_light_vp(toward, 33.0));
        assert_eq!(p.local_count, 1);
        // An unshadowed first sun disables the shadow pass.
        f.scene_lights[1].light.shadows = false;
        let p = pack(&f);
        assert!(!p.shadow && p.suns[0].direction_shadow[3] == 0.0);
        // Straight-down suns still give a finite matrix.
        assert!(sun_light_vp(Vec3::Y, 10.0).is_finite());
    }

    #[test]
    fn spot_and_area_parameters() {
        let mut f = frame();
        f.scene_lights = vec![
            light(
                "spot",
                json!({"kind":"spot","range":8,"intensity":3,"source_radius":0.1,"spot":{"inner_degrees":30,"outer_degrees":60}}),
                Vec3::new(0.0, 4.0, 0.0),
                -Vec3::Y,
            ),
            light(
                "hard",
                json!({"kind":"spot","range":8,"spot":{"inner_degrees":40,"outer_degrees":40}}),
                Vec3::ZERO,
                -Vec3::Y,
            ),
            light(
                "panel",
                json!({"kind":"area","range":6,"area":{"shape":"rect","size":[2,1]}}),
                Vec3::ZERO,
                -Vec3::Y,
            ),
            light(
                "disk",
                json!({"kind":"area","range":6,"area":{"shape":"disk","size":[3,3]}}),
                Vec3::ZERO,
                -Vec3::Y,
            ),
        ];
        let p = pack(&f);
        assert!(!p.authored, "local lights alone keep the legacy model");
        let spot = p.locals[0];
        assert_eq!(spot.direction_kind, [0.0, -1.0, 0.0, KIND_SPOT]);
        assert!((spot.params[0] - 15f32.to_radians().cos()).abs() < 1e-6);
        assert!((spot.params[1] - 30f32.to_radians().cos()).abs() < 1e-6);
        assert_eq!(spot.position_range, [0.0, 4.0, 0.0, 8.0]);
        assert_eq!(spot.axis[3], 0.1);
        let hard = p.locals[1];
        assert!(
            hard.params[0] > hard.params[1],
            "smoothstep edges must differ"
        );
        assert_eq!(p.locals[2].direction_kind[3], KIND_RECT);
        assert_eq!(p.locals[2].params[2..], [1.0, 0.5]);
        assert_eq!(p.locals[2].axis[..3], [1.0, 0.0, 0.0]);
        assert_eq!(p.locals[3].direction_kind[3], KIND_DISK);
        assert_eq!(p.locals[3].params[2], 1.5);
    }

    #[test]
    fn authored_local_lights_come_before_vfx_and_overflow_is_dropped() {
        let mut f = frame();
        f.lights = (0..20).map(|i| vfx(i as f32)).collect();
        f.scene_lights = (0..20)
            .map(|i| {
                light(
                    &format!("lamp/{i}"),
                    json!({"kind":"point","range":5}),
                    Vec3::new(i as f32, 5.0, 0.0),
                    -Vec3::Z,
                )
            })
            .collect();
        let p = pack(&f);
        assert_eq!(p.local_count, 32);
        assert_eq!(p.dropped, 8);
        for i in 0..20 {
            assert_eq!(p.locals[i].position_range, [i as f32, 5.0, 0.0, 5.0]);
        }
        for i in 0..12 {
            assert_eq!(p.locals[20 + i].position_range, [i as f32, 1.0, 0.0, 2.0]);
            assert_eq!(p.locals[20 + i].color_intensity, [1.0, 0.5, 0.2, 3.0]);
        }
        // VFX only: the legacy packing (point, fixed radius) is unchanged.
        f.scene_lights.clear();
        let p = pack(&f);
        assert_eq!(p.local_count, 20);
        assert_eq!(p.locals[0].direction_kind, [0.0; 4]);
        assert_eq!(p.locals[0].axis, [0.0; 4]);
    }

    #[test]
    fn environment_overrides_ambient_and_fog() {
        let mut f = frame();
        f.environment = serde_json::from_value(json!({
            "ambient":{"mode":"gradient","sky":[0.2,0.3,0.5],"equator":[0.1,0.1,0.1],"ground":[0.05,0.04,0.03],"intensity":2},
            "fog":{"color":[0.1,0.1,0.2],"density":0.05}
        }))
        .unwrap();
        let p = pack(&f);
        assert!(p.authored);
        assert_eq!(p.ambient[0][..3], [0.4, 0.6, 1.0]);
        assert_eq!(p.ambient[1][..3], [0.2, 0.2, 0.2]);
        assert_eq!(p.ambient[2][..3], [0.1, 0.08, 0.06]);
        assert_eq!(p.ambient[1][3], 1.0);
        assert!((p.fog[3] - 0.0025).abs() < 1e-9);
        // Environment alone keeps the default sun.
        assert_eq!(p.sun_count, 1);
        assert_eq!(p.light_vp, legacy_light_vp(33.0));
    }

    #[test]
    fn no_sky_keeps_the_legacy_background() {
        let p = pack(&frame());
        assert_eq!(p.sky, SkyParams::NONE);
        assert_eq!(p.sky_uniform()[0][3], SKY_NONE);
        assert_eq!(p.sky.radiance(Vec3::Y, true), [0.0; 3]);
    }

    #[test]
    fn gradient_sky_radiance_and_sky_light() {
        let sky: Sky = serde_json::from_value(json!({"mode":"gradient","zenith_color":[0.1,0.2,0.8],"horizon_color":[0.7,0.7,0.7],"ground_color":[0.2,0.1,0.0],"exponent":2,"intensity":0.5})).unwrap();
        let s = SkyParams::new(Some(&sky), Vec3::Y, [1.0; 3]);
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(s.radiance(Vec3::Y, true), [0.05, 0.1, 0.4]));
        assert!(close(s.radiance(Vec3::X, true), [0.35, 0.35, 0.35]));
        assert!(close(s.radiance(-Vec3::Y, true), [0.1, 0.05, 0.0]));
        // pow(up, exponent): halfway up (y = 0.5) is a quarter of the way to the zenith.
        let mid = s.radiance(Vec3::new(0.75_f32.sqrt(), 0.5, 0.0), false);
        assert!((mid[2] - (0.35 + (0.4 - 0.35) * 0.25)).abs() < 1e-5);
        // A uniform sky gives the same irradiance for every normal.
        let flat: Sky = serde_json::from_value(json!({"mode":"gradient","zenith_color":[0.3,0.4,0.5],"horizon_color":[0.3,0.4,0.5],"ground_color":[0.3,0.4,0.5]})).unwrap();
        for c in SkyParams::new(Some(&flat), Vec3::Y, [1.0; 3]).trilight() {
            assert!(close(c, [0.3, 0.4, 0.5]), "{c:?}");
        }
        // Up sees mostly the zenith, down mostly the ground.
        let [up, equator, down] = s.trilight();
        assert!(up[2] > equator[2] && equator[2] > down[2]);
        assert!(down[0] < equator[0]);

        // ambient: sky = that trilight times its intensity; the sky reaches the shader.
        let mut f = frame();
        f.environment = serde_json::from_value(json!({"sky":sky,"ambient":{"mode":"sky","intensity":2}})).unwrap();
        f.environment.validate().unwrap();
        let p = pack(&f);
        assert!(p.authored);
        let expect = SkyParams::new(Some(&sky), p.sky.sun_toward, p.sky.sun_color).trilight();
        for k in 0..3 {
            assert!(close(
                [p.ambient[k][0], p.ambient[k][1], p.ambient[k][2]],
                expect[k].map(|v| v * 2.0)
            ));
        }
        let u = p.sky_uniform();
        assert_eq!(u[0], [0.1, 0.2, 0.8, SKY_GRADIENT]);
        assert_eq!(u[1][3], 2.0);
        assert_eq!(u[2][3], 0.5);
        assert!(u[3][0] > 1.0, "gradient skies have no sun disk");
        // A sky without ambient: sky shown, ambient stays the legacy default.
        f.environment.ambient = None;
        let (sky_only, legacy) = (pack(&f), pack(&frame()));
        for k in 0..3 {
            assert_eq!(sky_only.ambient[k][..3], legacy.ambient[k][..3]);
        }
        assert!(sky_only.authored && !legacy.authored);
    }

    #[test]
    fn procedural_sky_follows_the_sun() {
        let sky = Sky::preset("procedural").unwrap();
        let toward = |deg: f32| {
            let r = deg.to_radians();
            Vec3::new(r.cos(), r.sin(), 0.0)
        };
        let noon = SkyParams::new(Some(&sky), toward(60.0), [1.0; 3]);
        let dusk = SkyParams::new(Some(&sky), toward(2.0), [1.0, 0.7, 0.4]);
        let night = SkyParams::new(Some(&sky), toward(-30.0), [0.2; 3]);
        assert_eq!(noon.mode, SKY_PROCEDURAL);
        // Blue zenith by day, much darker at night; a warm horizon at sunset.
        assert!(noon.zenith[2] > noon.zenith[0] && noon.zenith[2] > 0.5);
        assert!(night.zenith[2] < 0.05);
        assert!(dusk.horizon[0] > dusk.horizon[2]);
        assert!(noon.horizon[2] > noon.horizon[0]);
        // The disk only where the sun is, and only with `disk`.
        let at_sun = noon.radiance(toward(60.0), true);
        let without = noon.radiance(toward(60.0), false);
        let away = noon.radiance(toward(120.0), true);
        assert!(at_sun[0] > without[0] + 5.0);
        assert!(away[0] < without[0]);
        // No disk when its size is 0.
        let bare = SkyParams::new(
            Some(&Sky::Procedural {
                sun_size_degrees: 0.0,
                atmosphere: 1.0,
                ground_color: [0.2; 3],
                exposure: 1.0,
            }),
            toward(60.0),
            [1.0; 3],
        );
        assert_eq!(bare.radiance(toward(60.0), true), bare.radiance(toward(60.0), false));
        // The sun of the frame is sun 0: the first directional light.
        let mut f = frame();
        f.environment = serde_json::from_value(json!({"sky":{"mode":"procedural"}})).unwrap();
        f.scene_lights = vec![light(
            "sun",
            json!({"kind":"directional","color":[1,0.9,0.8],"intensity":2}),
            Vec3::ZERO,
            Vec3::new(0.0, -1.0, -1.0),
        )];
        let p = pack(&f);
        assert!((p.sky.sun_toward - Vec3::new(0.0, 1.0, 1.0).normalize()).length() < 1e-5);
        assert_eq!(p.sky.sun_color, [2.0, 1.8, 1.6]);
        assert!(p.authored);
    }
}
