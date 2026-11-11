//! Authored lights (`render.light`) and the ambient environment (`render.environment`).
//!
//! The model follows Blender / Unity light types: directional, point, spot and area.
//! Directions come from the entity transform: a light shines along its local -Z axis, the
//! same convention as `render.camera`. Area lights are an APPROXIMATION (a representative
//! point on the rectangle or disk closest to the shaded point); they are not LTC-accurate.
use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Lights that can be authored per world. Local = point + spot + area; they share the GPU
/// array (32 slots) with VFX particle lights, authored lights first.
pub const MAX_DIRECTIONAL_LIGHTS: usize = 4;
pub const MAX_LOCAL_LIGHTS: usize = 32;
/// Legacy hard-coded sun, used when a world authors no directional light.
pub const DEFAULT_SUN_TOWARD: [f32; 3] = [-0.45, 0.85, 0.35];
pub const DEFAULT_SUN_COLOR: [f32; 3] = [1.12, 1.0, 0.82];
/// Legacy two-colour hemisphere ambient (ground at n.y = -1, sky at n.y = +1).
pub const DEFAULT_AMBIENT_GROUND: [f32; 3] = [0.11, 0.16, 0.19];
pub const DEFAULT_AMBIENT_SKY: [f32; 3] = [0.37, 0.48, 0.53];
/// Legacy exponential-squared fog: `1 - exp(-d² * DEFAULT_FOG_K)`.
pub const DEFAULT_FOG_COLOR: [f32; 3] = [0.055, 0.09, 0.12];
pub const DEFAULT_FOG_K: f32 = 0.00012;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Directional,
    Point,
    Spot,
    Area,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SpotCone {
    /// Full cone angle (like Unity's inner spot angle) where falloff starts.
    pub inner_degrees: f32,
    /// Full cone angle (Unity spotAngle, Blender spot_size) where light reaches zero.
    pub outer_degrees: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AreaShape {
    Rect,
    Disk,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AreaLight {
    pub shape: AreaShape,
    /// Rect: [width, height] along local X and Y. Disk: [diameter, diameter].
    pub size: [f32; 2],
}

fn white() -> [f32; 3] {
    [1.0; 3]
}
fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenderLight {
    pub kind: LightKind,
    #[serde(default = "white")]
    pub color: [f32; 3],
    #[serde(default = "one")]
    pub intensity: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot: Option<SpotCone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<AreaLight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_radius: Option<f32>,
    #[serde(default)]
    pub shadows: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl RenderLight {
    pub fn directional(color: [f32; 3], intensity: f32, shadows: bool) -> Self {
        Self {
            kind: LightKind::Directional,
            color,
            intensity,
            range: None,
            spot: None,
            area: None,
            source_radius: None,
            shadows,
            enabled: true,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.color
                .iter()
                .all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
            "color must be rgb in 0..1"
        );
        ensure!(
            self.intensity.is_finite() && (0.0..=100.0).contains(&self.intensity),
            "intensity must be 0..100"
        );
        let local = self.kind != LightKind::Directional;
        match (local, self.range) {
            (true, Some(r)) => ensure!(
                r.is_finite() && (0.01..=500.0).contains(&r),
                "range must be 0.01..500 meters"
            ),
            (true, None) => bail!("{:?} light requires range (meters)", self.kind),
            (false, Some(_)) => bail!("directional light has no range"),
            (false, None) => {}
        }
        match (self.kind, &self.spot) {
            (LightKind::Spot, Some(s)) => ensure!(
                s.inner_degrees.is_finite()
                    && s.outer_degrees.is_finite()
                    && (1.0..=179.0).contains(&s.outer_degrees)
                    && (0.0..=s.outer_degrees).contains(&s.inner_degrees),
                "spot requires 0 <= inner_degrees <= outer_degrees, outer_degrees 1..179 (full cone angles)"
            ),
            (LightKind::Spot, None) => {
                bail!("spot light requires spot {{inner_degrees, outer_degrees}}")
            }
            (_, Some(_)) => bail!("spot settings are only valid on kind spot"),
            _ => {}
        }
        match (self.kind, &self.area) {
            (LightKind::Area, Some(a)) => {
                ensure!(
                    a.size
                        .iter()
                        .all(|s| s.is_finite() && (0.01..=100.0).contains(s)),
                    "area size must be 0.01..100 meters"
                );
                ensure!(
                    a.shape == AreaShape::Rect || a.size[0] == a.size[1],
                    "disk area light requires size [diameter, diameter]"
                );
            }
            (LightKind::Area, None) => {
                bail!("area light requires area {{shape: rect|disk, size: [w, h]}}")
            }
            (_, Some(_)) => bail!("area settings are only valid on kind area"),
            _ => {}
        }
        if let Some(r) = self.source_radius {
            ensure!(
                matches!(self.kind, LightKind::Point | LightKind::Spot),
                "source_radius is only valid on point and spot lights"
            );
            ensure!(
                r.is_finite() && r >= 0.0 && r <= self.range.unwrap_or(0.0),
                "source_radius must be 0..range"
            );
        }
        if self.shadows {
            match self.kind {
                LightKind::Directional => {}
                LightKind::Point => bail!("point light shadows not supported yet"),
                LightKind::Spot => bail!("spot light shadows not supported yet"),
                LightKind::Area => bail!("area light shadows not supported yet"),
            }
        }
        Ok(())
    }
}

/// Schema hints shown by `discover`/`schema render.light` and the inspector form.
pub fn light_schema() -> Value {
    json!({
        "kind": "directional | point | spot | area (shines along the entity's local -Z, like render.camera)",
        "color": "rgb[3] 0..1, default [1,1,1]",
        "intensity": "0..100, default 1",
        "range": "meters 0.01..500; required for point/spot/area, rejected on directional",
        "spot": "kind spot only: {inner_degrees, outer_degrees} full cone angles, 0 <= inner <= outer, outer 1..179; smooth falloff between them",
        "area": "kind area only: {shape: rect|disk, size: [w,h] meters 0.01..100; disk = [d,d]}. APPROXIMATE: lit from the point on the shape closest to the surface (not LTC-accurate), one-sided along -Z",
        "source_radius": "optional, point/spot only, 0..range: softens the light terminator (approximate sphere light)",
        "shadows": "bool, default false. Directional: the first enabled directional light casts the shadow map, others are unshadowed. point/spot/area shadows are not supported yet (rejected)",
        "enabled": "bool, default true",
        "budget": format!("per world: {MAX_DIRECTIONAL_LIGHTS} directional, {MAX_LOCAL_LIGHTS} point/spot/area; GPU array 32 local lights shared with VFX lights (authored first, then VFX in emitter order)")
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Ambient {
    Color {
        color: [f32; 3],
        #[serde(default = "one")]
        intensity: f32,
    },
    /// Unity "Trilight": sky (normal up), equator (horizontal) and ground (normal down).
    Gradient {
        sky: [f32; 3],
        equator: [f32; 3],
        ground: [f32; 3],
        #[serde(default = "one")]
        intensity: f32,
    },
    /// Sky light: the trilight is the irradiance of the authored `sky` (without the sun disk)
    /// for normals up, horizontal and down. Requires `sky`.
    Sky {
        #[serde(default = "one")]
        intensity: f32,
    },
}

fn default_sun_size() -> f32 {
    1.5
}
fn default_sky_ground() -> [f32; 3] {
    [0.22, 0.2, 0.18]
}

/// Background sky (also the source of `ambient: sky` and of surface reflections).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Sky {
    /// View-dependent gradient: horizon → zenith above the horizon (`pow(up, exponent)`),
    /// horizon → ground just below it (ground reached about 10° down).
    Gradient {
        zenith_color: [f32; 3],
        horizon_color: [f32; 3],
        ground_color: [f32; 3],
        #[serde(default = "one")]
        exponent: f32,
        #[serde(default = "one")]
        intensity: f32,
    },
    /// Analytic daylight sky lit by the first enabled directional light (else the default
    /// sun): colours follow the sun elevation (day, sunset, night), haze around the sun and a
    /// sun disk. An approximation, not a physical scattering solve.
    Procedural {
        #[serde(default = "default_sun_size")]
        sun_size_degrees: f32,
        #[serde(default = "one")]
        atmosphere: f32,
        #[serde(default = "default_sky_ground")]
        ground_color: [f32; 3],
        #[serde(default = "one")]
        exposure: f32,
    },
}

impl Sky {
    pub fn validate(&self) -> Result<()> {
        match self {
            Sky::Gradient {
                zenith_color,
                horizon_color,
                ground_color,
                exponent,
                intensity,
            } => {
                ensure!(
                    rgb_ok(zenith_color) && rgb_ok(horizon_color) && rgb_ok(ground_color),
                    "sky zenith_color/horizon_color/ground_color must be rgb in 0..1"
                );
                ensure!(
                    exponent.is_finite() && (0.1..=8.0).contains(exponent),
                    "sky exponent must be 0.1..8"
                );
                ensure!(
                    intensity.is_finite() && (0.0..=8.0).contains(intensity),
                    "sky intensity must be 0..8"
                );
            }
            Sky::Procedural {
                sun_size_degrees,
                atmosphere,
                ground_color,
                exposure,
            } => {
                ensure!(
                    sun_size_degrees.is_finite() && (0.0..=10.0).contains(sun_size_degrees),
                    "sky sun_size_degrees must be 0..10"
                );
                ensure!(
                    atmosphere.is_finite() && (0.0..=4.0).contains(atmosphere),
                    "sky atmosphere must be 0..4"
                );
                ensure!(rgb_ok(ground_color), "sky ground_color must be rgb in 0..1");
                ensure!(
                    exposure.is_finite() && (0.0..=8.0).contains(exposure),
                    "sky exposure must be 0..8"
                );
            }
        }
        Ok(())
    }

    /// Editor / ACL presets (`sky gradient|procedural`).
    pub fn preset(mode: &str) -> Option<Self> {
        match mode {
            "gradient" => Some(Sky::Gradient {
                zenith_color: [0.16, 0.32, 0.62],
                horizon_color: [0.62, 0.72, 0.82],
                ground_color: default_sky_ground(),
                exponent: 1.0,
                intensity: 1.0,
            }),
            "procedural" => Some(Sky::Procedural {
                sun_size_degrees: default_sun_size(),
                atmosphere: 1.0,
                ground_color: default_sky_ground(),
                exposure: 1.0,
            }),
            _ => None,
        }
    }
}

/// Surface response of an entity's `render.mesh` parts and imported / procedural meshes:
/// metallic tints and strengthens reflections, smoothness sharpens them. Reflections show
/// the authored sky (else the ambient trilight); they do not show other objects.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenderSurface {
    #[serde(default)]
    pub metallic: f32,
    #[serde(default = "half")]
    pub smoothness: f32,
}
fn half() -> f32 {
    0.5
}
impl Default for RenderSurface {
    fn default() -> Self {
        Self {
            metallic: 0.0,
            smoothness: half(),
        }
    }
}
impl RenderSurface {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.metallic.is_finite() && (0.0..=1.0).contains(&self.metallic),
            "metallic must be 0..1"
        );
        ensure!(
            self.smoothness.is_finite() && (0.0..=1.0).contains(&self.smoothness),
            "smoothness must be 0..1"
        );
        Ok(())
    }
}

pub fn surface_schema() -> Value {
    json!({
        "metallic": "0..1, default 0: 0 = dielectric (4% reflectance at normal incidence), 1 = metal (reflection tinted by the base colour, no diffuse)",
        "smoothness": "0..1, default 0.5: 0 = blurry reflection (the ambient trilight), 1 = mirror-sharp reflection of the sky",
        "requires": "applies to every render.mesh part of the entity and to its imported / procedural mesh (meshes only when the world authors lighting: a directional light or render.environment). Reflects the render.environment sky (else the ambient trilight), never other objects (no probes or screen-space reflections)"
    })
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fog {
    pub color: [f32; 3],
    /// Exponential-squared density (Unity ExponentialSquared): fog = 1 - exp(-(density·d)²).
    pub density: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenderEnvironment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<Ambient>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fog: Option<Fog>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sky: Option<Sky>,
}

fn rgb_ok(c: &[f32; 3]) -> bool {
    c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
}

impl RenderEnvironment {
    pub fn validate(&self) -> Result<()> {
        match &self.ambient {
            Some(Ambient::Color { color, intensity }) => {
                ensure!(rgb_ok(color), "ambient color must be rgb in 0..1");
                ensure!(
                    intensity.is_finite() && (0.0..=10.0).contains(intensity),
                    "ambient intensity must be 0..10"
                );
            }
            Some(Ambient::Gradient {
                sky,
                equator,
                ground,
                intensity,
            }) => {
                ensure!(
                    rgb_ok(sky) && rgb_ok(equator) && rgb_ok(ground),
                    "ambient sky/equator/ground must be rgb in 0..1"
                );
                ensure!(
                    intensity.is_finite() && (0.0..=10.0).contains(intensity),
                    "ambient intensity must be 0..10"
                );
            }
            Some(Ambient::Sky { intensity }) => {
                ensure!(
                    self.sky.is_some(),
                    "ambient mode sky requires a sky (set sky first, or use mode color|gradient)"
                );
                ensure!(
                    intensity.is_finite() && (0.0..=10.0).contains(intensity),
                    "ambient intensity must be 0..10"
                );
            }
            None => {}
        }
        if let Some(sky) = &self.sky {
            sky.validate()?;
        }
        if let Some(f) = &self.fog {
            ensure!(rgb_ok(&f.color), "fog color must be rgb in 0..1");
            ensure!(
                f.density.is_finite() && (0.0..=1.0).contains(&f.density),
                "fog density must be 0..1"
            );
        }
        Ok(())
    }

    /// Hemisphere ambient as (sky, equator, ground), already multiplied by intensity, for the
    /// `color` and `gradient` modes. `None` = no ambient authored (the legacy default, tinted
    /// by `render.settings.accent`) or `sky` mode (the renderer integrates the sky; see
    /// `sky_ambient`).
    pub fn trilight(&self) -> Option<[[f32; 3]; 3]> {
        let scale = |c: &[f32; 3], k: f32| c.map(|v| v * k);
        match self.ambient.as_ref()? {
            Ambient::Sky { .. } => None,
            Ambient::Color { color, intensity } => {
                let c = scale(color, *intensity);
                Some([c, c, c])
            }
            Ambient::Gradient {
                sky,
                equator,
                ground,
                intensity,
            } => Some([
                scale(sky, *intensity),
                scale(equator, *intensity),
                scale(ground, *intensity),
            ]),
        }
    }
}

impl RenderEnvironment {
    /// Intensity of `ambient: {mode: sky}` (the trilight comes from the sky).
    pub fn sky_ambient(&self) -> Option<f32> {
        match self.ambient {
            Some(Ambient::Sky { intensity }) => Some(intensity),
            _ => None,
        }
    }
}

pub fn environment_schema() -> Value {
    json!({
        "ambient": "optional; {mode: color, color: rgb, intensity 0..10} | {mode: gradient, sky, equator, ground: rgb, intensity 0..10} (Unity Trilight) | {mode: sky, intensity 0..10} (sky light: irradiance of the authored sky, requires sky). Omitted = legacy sky/ground hemisphere tinted by render.settings.accent",
        "fog": "optional {color: rgb, density 0..1}: exponential squared, 1 - exp(-(density*distance)^2). Omitted = legacy fog (density ~0.011)",
        "sky": "optional; {mode: gradient, zenith_color, horizon_color, ground_color: rgb, exponent 0.1..8 (default 1), intensity 0..8 (default 1)} | {mode: procedural, sun_size_degrees 0..10 (default 1.5, 0 = no disk), atmosphere 0..4 (default 1), ground_color rgb, exposure 0..8 (default 1)} (analytic, follows the first directional light). Omitted = the legacy screen-space background gradient. Orthographic views see one colour (the view direction)",
        "edit": "sky gradient|procedural|none (ACL) = World ▸ Sky (editor): preset sky + ambient mode sky"
    })
}

/// World-level light budget (counts every entity with `render.light`, enabled or not, so the
/// result never depends on scene state).
pub fn validate_budget(lights: impl Iterator<Item = LightKind>) -> Result<()> {
    let (mut directional, mut local) = (0, 0);
    for kind in lights {
        if kind == LightKind::Directional {
            directional += 1;
        } else {
            local += 1;
        }
    }
    ensure!(
        directional <= MAX_DIRECTIONAL_LIGHTS,
        "directional light budget exceeded ({directional} > {MAX_DIRECTIONAL_LIGHTS})"
    );
    ensure!(
        local <= MAX_LOCAL_LIGHTS,
        "point/spot/area light budget exceeded ({local} > {MAX_LOCAL_LIGHTS})"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(v: Value) -> Result<RenderLight> {
        let l: RenderLight = serde_json::from_value(v)?;
        l.validate()?;
        Ok(l)
    }
    fn err(v: Value) -> String {
        format!("{:#}", light(v).unwrap_err())
    }

    #[test]
    fn valid_lights_of_every_kind() {
        let sun = light(json!({"kind":"directional","shadows":true})).unwrap();
        assert_eq!(sun.color, [1.0; 3]);
        assert_eq!(sun.intensity, 1.0);
        assert!(sun.enabled);
        light(
            json!({"kind":"point","range":5,"source_radius":0.2,"color":[1,0.8,0.5],"intensity":3}),
        )
        .unwrap();
        light(json!({"kind":"spot","range":9,"spot":{"inner_degrees":20,"outer_degrees":40}}))
            .unwrap();
        light(json!({"kind":"area","range":6,"area":{"shape":"rect","size":[2,1]}})).unwrap();
        light(json!({"kind":"area","range":6,"area":{"shape":"disk","size":[1,1]}})).unwrap();
    }

    #[test]
    fn invalid_lights_are_rejected_with_reasons() {
        assert!(err(json!({"kind":"laser"})).contains("unknown variant"));
        assert!(err(json!({"kind":"point","range":2,"radius":3})).contains("unknown field"));
        assert!(err(
            json!({"kind":"spot","range":2,"spot":{"inner_degrees":1,"outer_degrees":2,"x":1}})
        )
        .contains("unknown field"));
        assert!(err(json!({"kind":"point"})).contains("requires range"));
        assert!(err(json!({"kind":"directional","range":3})).contains("no range"));
        assert!(err(json!({"kind":"spot","range":3})).contains("requires spot"));
        assert!(err(
            json!({"kind":"point","range":3,"spot":{"inner_degrees":1,"outer_degrees":2}})
        )
        .contains("only valid on kind spot"));
        assert!(err(
            json!({"kind":"spot","range":3,"spot":{"inner_degrees":50,"outer_degrees":40}})
        )
        .contains("inner_degrees"));
        assert!(err(
            json!({"kind":"spot","range":3,"spot":{"inner_degrees":0,"outer_degrees":180}})
        )
        .contains("outer_degrees"));
        assert!(err(json!({"kind":"area","range":3})).contains("requires area"));
        assert!(
            err(json!({"kind":"area","range":3,"area":{"shape":"disk","size":[1,2]}}))
                .contains("disk")
        );
        assert!(
            err(json!({"kind":"area","range":3,"area":{"shape":"rect","size":[0,2]}}))
                .contains("size")
        );
        assert!(
            err(json!({"kind":"point","range":3,"area":{"shape":"rect","size":[1,1]}}))
                .contains("only valid on kind area")
        );
        assert!(err(json!({"kind":"point","range":3,"color":[2,0,0]})).contains("color"));
        assert!(err(json!({"kind":"point","range":3,"intensity":-1})).contains("intensity"));
        assert!(err(json!({"kind":"point","range":900})).contains("range"));
        assert!(err(json!({"kind":"point","range":1,"source_radius":2})).contains("source_radius"));
        assert!(err(json!({"kind":"area","range":3,"source_radius":0.1,"area":{"shape":"rect","size":[1,1]}})).contains("source_radius"));
        assert!(err(json!({"kind":"point","range":3,"shadows":true}))
            .contains("point light shadows not supported yet"));
        assert!(err(json!({"kind":"spot","range":3,"shadows":true,"spot":{"inner_degrees":1,"outer_degrees":2}}))
            .contains("spot light shadows not supported yet"));
    }

    #[test]
    fn budget_counts_directional_and_local_separately() {
        use LightKind::*;
        validate_budget([Directional; 4].into_iter().chain([Point; 32])).unwrap();
        assert!(format!(
            "{:#}",
            validate_budget([Directional; 5].into_iter()).unwrap_err()
        )
        .contains("directional light budget"));
        assert!(format!(
            "{:#}",
            validate_budget([Spot, Area].into_iter().cycle().take(33)).unwrap_err()
        )
        .contains("point/spot/area light budget"));
    }

    #[test]
    fn environment_modes_and_validation() {
        let parse = |v: Value| -> Result<RenderEnvironment> {
            let e: RenderEnvironment = serde_json::from_value(v)?;
            e.validate()?;
            Ok(e)
        };
        assert_eq!(parse(json!({})).unwrap().trilight(), None);
        let c =
            parse(json!({"ambient":{"mode":"color","color":[0.5,0.5,0.5],"intensity":2}})).unwrap();
        assert_eq!(c.trilight(), Some([[1.0; 3]; 3]));
        let g = parse(json!({"ambient":{"mode":"gradient","sky":[0.2,0.3,0.4],"equator":[0.1,0.1,0.1],"ground":[0,0,0]},"fog":{"color":[0,0,0],"density":0.02}})).unwrap();
        assert_eq!(g.trilight().unwrap()[0], [0.2, 0.3, 0.4]);
        assert!(parse(json!({"ambient":{"mode":"hdri"}})).is_err());
        assert!(parse(json!({"ambient":{"mode":"color","color":[1,1,1],"sky":[1,1,1]}})).is_err());
        assert!(parse(json!({"fog":{"color":[0,0,0],"density":2}})).is_err());
        assert!(parse(json!({"skybox":1})).is_err());
    }

    #[test]
    fn sky_modes_defaults_and_validation() {
        let parse = |v: Value| -> Result<RenderEnvironment> {
            let e: RenderEnvironment = serde_json::from_value(v)?;
            e.validate()?;
            Ok(e)
        };
        let err = |v: Value| format!("{:#}", parse(v).unwrap_err());
        // Old content (no sky) round-trips byte for byte: the new field is not written.
        let old = json!({"ambient":{"mode":"color","color":[0.5,0.5,0.5],"intensity":1.0}});
        assert_eq!(json!(parse(old.clone()).unwrap()), old);
        let p = parse(json!({"sky":{"mode":"procedural"},"ambient":{"mode":"sky"}})).unwrap();
        assert_eq!(p.sky, Sky::preset("procedural"));
        assert_eq!(p.sky_ambient(), Some(1.0));
        assert_eq!(
            p.trilight(),
            None,
            "sky light is integrated by the renderer"
        );
        let g = parse(json!({"sky":{"mode":"gradient","zenith_color":[0,0,1],"horizon_color":[1,1,1],"ground_color":[0,0,0]}})).unwrap();
        assert!(
            matches!(g.sky, Some(Sky::Gradient { exponent, intensity, .. }) if exponent == 1.0 && intensity == 1.0)
        );
        assert!(err(json!({"ambient":{"mode":"sky"}})).contains("requires a sky"));
        assert!(err(json!({"sky":{"mode":"hdri"}})).contains("unknown variant"));
        assert!(err(json!({"sky":{"mode":"procedural","sun":1}})).contains("unknown field"));
        assert!(
            err(json!({"sky":{"mode":"procedural","sun_size_degrees":11}})).contains("sun_size")
        );
        assert!(err(json!({"sky":{"mode":"procedural","atmosphere":-1}})).contains("atmosphere"));
        assert!(err(json!({"sky":{"mode":"procedural","exposure":9}})).contains("exposure"));
        assert!(err(json!({"sky":{"mode":"gradient","zenith_color":[2,0,0],"horizon_color":[1,1,1],"ground_color":[0,0,0]}})).contains("rgb"));
        assert!(err(json!({"sky":{"mode":"gradient","zenith_color":[0,0,0],"horizon_color":[1,1,1],"ground_color":[0,0,0],"exponent":0}})).contains("exponent"));
        assert!(
            err(json!({"sky":{"mode":"gradient","zenith_color":[0,0,0]}}))
                .contains("missing field")
        );
        assert!(
            err(json!({"sky":{"mode":"procedural"},"ambient":{"mode":"sky","intensity":11}}))
                .contains("intensity")
        );
        for mode in ["gradient", "procedural"] {
            Sky::preset(mode).unwrap().validate().unwrap();
        }
        assert_eq!(Sky::preset("hdri"), None);
    }

    #[test]
    fn surface_defaults_and_ranges() {
        let s: RenderSurface = serde_json::from_value(json!({})).unwrap();
        assert_eq!(s, RenderSurface::default());
        assert_eq!((s.metallic, s.smoothness), (0.0, 0.5));
        for bad in [
            json!({"metallic":1.5}),
            json!({"smoothness":-0.1}),
            json!({"metallic":"x"}),
            json!({"roughness":0.2}),
        ] {
            let parsed: Result<RenderSurface> =
                serde_json::from_value(bad.clone()).map_err(Into::into);
            assert!(parsed.and_then(|s| s.validate()).is_err(), "{bad}");
        }
    }
}
