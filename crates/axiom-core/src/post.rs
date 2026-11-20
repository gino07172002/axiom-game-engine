//! Post-processing settings of `render.environment.post`: FXAA anti-aliasing, an HDR scene
//! target with exposure and ACES tone mapping, and bloom. Everything is opt-in: a world
//! without `post` (or with its defaults) renders exactly as before.
//!
//! FXAA is a post-process edge filter on the final colours (Lottes' FXAA 3.11 quality-style
//! luma edge search), not multisampling: it smooths geometric and shading edges alike and can
//! soften fine texture detail. MSAA and TAA are not implemented.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Antialiasing {
    #[default]
    None,
    Fxaa,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToneMapper {
    /// Stephen Hill's fit of the ACES RRT + sRGB ODT (BakingLab `ACESFitted`).
    #[default]
    Aces,
}

fn one() -> f32 {
    1.0
}
fn bloom_intensity() -> f32 {
    0.4
}
fn bloom_radius() -> f32 {
    0.7
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Bloom {
    /// Linear scene brightness (before exposure) where bloom starts (soft knee of half the
    /// threshold below it).
    #[serde(default = "one")]
    pub threshold: f32,
    /// Strength of the glow added to the scene before tone mapping.
    #[serde(default = "bloom_intensity")]
    pub intensity: f32,
    /// 0..1: weight of each coarser mip level in the upsample chain (0 = a tight halo from the
    /// finest level only, 1 = every level equally: the widest glow).
    #[serde(default = "bloom_radius")]
    pub radius: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self {
            threshold: one(),
            intensity: bloom_intensity(),
            radius: bloom_radius(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Hdr {
    /// Linear multiplier before tone mapping.
    #[serde(default = "one")]
    pub exposure: f32,
    #[serde(default)]
    pub tonemap: ToneMapper,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bloom: Option<Bloom>,
}

impl Default for Hdr {
    fn default() -> Self {
        Self {
            exposure: one(),
            tonemap: ToneMapper::Aces,
            bloom: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostProcess {
    #[serde(default)]
    pub antialiasing: Antialiasing,
    /// Present = the scene renders into an HDR (16-bit float) target and is tone mapped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<Hdr>,
}

/// Ranges of the numeric settings (also the editor slider ranges).
pub const EXPOSURE_RANGE: (f32, f32) = (0.05, 16.0);
pub const THRESHOLD_RANGE: (f32, f32) = (0.0, 16.0);
pub const INTENSITY_RANGE: (f32, f32) = (0.0, 4.0);
pub const RADIUS_RANGE: (f32, f32) = (0.0, 1.0);

fn within(v: f32, (lo, hi): (f32, f32)) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

impl PostProcess {
    pub fn validate(&self) -> Result<()> {
        if let Some(hdr) = &self.hdr {
            ensure!(
                within(hdr.exposure, EXPOSURE_RANGE),
                "post hdr exposure must be 0.05..16"
            );
            if let Some(b) = &hdr.bloom {
                ensure!(
                    within(b.threshold, THRESHOLD_RANGE),
                    "post bloom threshold must be 0..16"
                );
                ensure!(
                    within(b.intensity, INTENSITY_RANGE),
                    "post bloom intensity must be 0..4"
                );
                ensure!(
                    within(b.radius, RADIUS_RANGE),
                    "post bloom radius must be 0..1"
                );
            }
        }
        Ok(())
    }

    pub fn fxaa(&self) -> bool {
        self.antialiasing == Antialiasing::Fxaa
    }

    /// Nothing differs from the legacy output (no pass runs).
    pub fn is_off(&self) -> bool {
        !self.fxaa() && self.hdr.is_none()
    }
}

/// Schema text of `render.environment.post`.
pub const POST_SCHEMA: &str = "optional {antialiasing: none (default) | fxaa (post-process edge filter, not MSAA), hdr: optional {exposure 0.05..16 (default 1), tonemap: aces (default; Hill ACES fitted), bloom: optional {threshold 0..16 (default 1, linear brightness), intensity 0..4 (default 0.4), radius 0..1 (default 0.7, wider glow)}}}. hdr renders the scene into a 16-bit float target (built-in lit shaders skip their x/(x+0.7) curve) and tone maps it; omitted = the legacy 8-bit output, unchanged";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Result<PostProcess> {
        let p: PostProcess = serde_json::from_value(v)?;
        p.validate()?;
        Ok(p)
    }
    fn err(v: serde_json::Value) -> String {
        format!("{:#}", parse(v).unwrap_err())
    }

    #[test]
    fn defaults_and_strict_validation() {
        let p = parse(json!({})).unwrap();
        assert!(p.is_off());
        let p = parse(json!({"antialiasing":"fxaa","hdr":{"bloom":{}}})).unwrap();
        assert!(p.fxaa() && !p.is_off());
        let hdr = p.hdr.unwrap();
        assert_eq!(hdr.exposure, 1.0);
        assert_eq!(hdr.tonemap, ToneMapper::Aces);
        assert_eq!(hdr.bloom, Some(Bloom::default()));
        assert!(err(json!({"antialiasing":"msaa"})).contains("unknown variant"));
        assert!(err(json!({"hdr":{"tonemap":"agx"}})).contains("unknown variant"));
        assert!(err(json!({"hdr":{"gamma":2}})).contains("unknown field"));
        assert!(err(json!({"bloom":{}})).contains("unknown field"));
        assert!(err(json!({"hdr":{"exposure":0}})).contains("exposure"));
        assert!(err(json!({"hdr":{"exposure":17}})).contains("exposure"));
        assert!(err(json!({"hdr":{"bloom":{"threshold":-1}}})).contains("threshold"));
        assert!(err(json!({"hdr":{"bloom":{"intensity":5}}})).contains("intensity"));
        assert!(err(json!({"hdr":{"bloom":{"radius":1.5}}})).contains("radius"));
        // Serialised form omits what is absent (round trip).
        let v = serde_json::to_value(parse(json!({"hdr":{}})).unwrap()).unwrap();
        assert_eq!(
            v,
            json!({"antialiasing":"none","hdr":{"exposure":1.0,"tonemap":"aces"}})
        );
    }
}
