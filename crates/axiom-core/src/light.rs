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
            None => {}
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

    /// Hemisphere ambient as (sky, equator, ground), already multiplied by intensity.
    /// `None` = the legacy default (renderer tints it by `render.settings.accent`).
    pub fn trilight(&self) -> Option<[[f32; 3]; 3]> {
        let scale = |c: &[f32; 3], k: f32| c.map(|v| v * k);
        match self.ambient.as_ref()? {
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

pub fn environment_schema() -> Value {
    json!({
        "ambient": "optional; {mode: color, color: rgb, intensity 0..10} | {mode: gradient, sky, equator, ground: rgb, intensity 0..10} (Unity Trilight). Omitted = legacy sky/ground hemisphere tinted by render.settings.accent",
        "fog": "optional {color: rgb, density 0..1}: exponential squared, 1 - exp(-(density*distance)^2). Omitted = legacy fog (density ~0.011)",
        "sky": "not configurable yet (background gradient stays the engine default)"
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
}
