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
    AreaShape, LightKind, DEFAULT_AMBIENT_GROUND, DEFAULT_AMBIENT_SKY, DEFAULT_FOG_COLOR,
    DEFAULT_FOG_K, DEFAULT_SUN_COLOR, DEFAULT_SUN_TOWARD,
};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

pub const SCENE_WGSL: &str = concat!(include_str!("lighting.wgsl"), include_str!("scene.wgsl"));
pub const MESH_WGSL: &str = concat!(include_str!("lighting.wgsl"), include_str!("mesh.wgsl"));
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
    let authored = !directional.is_empty() || env.ambient.is_some() || env.fog.is_some();
    let [sky, equator, ground] = env.trilight().unwrap_or_else(|| {
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
}
