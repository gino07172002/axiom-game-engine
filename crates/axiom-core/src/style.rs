//! Stylized looks for an entity's imported / procedural triangle mesh: `render.toon` (banded
//! light ramp, rim, face-forward shading, a wrap-lighting skin APPROXIMATION), `render.outline`
//! (inverted hull) and `render.decal` (one overlay texture layer).
//!
//! These are plain authored data; the runtime mesh renderer turns them into per-primitive
//! shader parameters. The skin term is wrap lighting plus a tinted terminator band: it is an
//! approximation of how light bleeds through skin, not subsurface scattering. The face term
//! bends a primitive's shading normal toward an authored head direction (no SDF face map).
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Upper bound of `materials` lists (a glTF asset has at most 256 materials).
const MAX_FILTER: usize = 256;

fn unit3(c: &[f32; 3]) -> bool {
    c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
}
fn in_range(v: f32, min: f32, max: f32) -> bool {
    v.is_finite() && (min..=max).contains(&v)
}
fn names(list: &[String], field: &str) -> Result<()> {
    ensure!(
        list.len() <= MAX_FILTER,
        "{field} lists at most {MAX_FILTER} materials"
    );
    for name in list {
        ensure!(
            !name.is_empty() && name.len() <= 256,
            "{field} entries are material IDs of the mesh (1..256 bytes)"
        );
    }
    Ok(())
}

/// Whether a primitive is selected by `filter` (empty = every one): an entry names the
/// primitive's material ID (`node/mesh/primitive`) or its authored material name.
pub fn selects(filter: &[String], id: &str, name: Option<&str>) -> bool {
    filter.is_empty() || filter.iter().any(|f| f == id || Some(f.as_str()) == name)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenderToon {
    /// Lit steps between the shadow colour and full light (1 = two-tone).
    #[serde(default = "default_bands")]
    pub bands: u32,
    /// Width of each band edge, 0 = hard, 1 = a smooth ramp across the band.
    #[serde(default = "default_softness")]
    pub softness: f32,
    /// Light-facing value (cosine after wrap) where the shadow side ends.
    #[serde(default)]
    pub threshold: f32,
    /// Multiplies the base colour on the shadow side (hue-shifted shadows).
    #[serde(default = "default_shadow")]
    pub shadow_color: [f32; 3],
    #[serde(default = "default_rim_color")]
    pub rim_color: [f32; 3],
    #[serde(default = "default_rim_strength")]
    pub rim_strength: f32,
    /// Fresnel exponent of the rim (higher = thinner).
    #[serde(default = "default_rim_power")]
    pub rim_power: f32,
    /// material names (glTF material.name) or node/mesh/primitive IDs the look applies to; empty = every primitive.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<ToonSkin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<ToonFace>,
}
fn default_bands() -> u32 {
    2
}
fn default_softness() -> f32 {
    0.1
}
fn default_shadow() -> [f32; 3] {
    [0.62, 0.56, 0.74]
}
fn default_rim_color() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}
fn default_rim_strength() -> f32 {
    0.3
}
fn default_rim_power() -> f32 {
    4.0
}
impl Default for RenderToon {
    fn default() -> Self {
        serde_json::from_value(json!({})).expect("toon defaults")
    }
}

/// Skin APPROXIMATION: wrap lighting (light reaches past the terminator) and a tinted band
/// along the terminator. Not subsurface scattering.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToonSkin {
    #[serde(default = "default_wrap")]
    pub wrap: f32,
    #[serde(default = "default_skin_tint")]
    pub tint: [f32; 3],
    /// Material names / IDs that are skin; empty = every primitive the toon look covers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<String>,
}
fn default_wrap() -> f32 {
    0.5
}
fn default_skin_tint() -> [f32; 3] {
    [1.0, 0.36, 0.26]
}

/// Face shading control: the shading normal of the face primitives bends toward `forward`
/// (in the space of `bone`, else the entity), so the face gets clean, head-driven light bands
/// instead of noisy per-vertex ones. Approximation; no SDF face shadow map.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToonFace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
    #[serde(default = "default_forward")]
    pub forward: [f32; 3],
    /// 0 = mesh normals, 1 = the face is shaded as if it faced `forward` everywhere.
    #[serde(default = "one")]
    pub blend: f32,
    /// Material names / IDs of the face; empty = every primitive the toon look covers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<String>,
}
fn default_forward() -> [f32; 3] {
    [0.0, 0.0, 1.0]
}
fn one() -> f32 {
    1.0
}

impl RenderToon {
    pub fn validate(&self) -> Result<()> {
        ensure!((1..=8).contains(&self.bands), "bands must be 1..8");
        ensure!(in_range(self.softness, 0.0, 1.0), "softness must be 0..1");
        ensure!(
            in_range(self.threshold, -0.9, 0.9),
            "threshold must be -0.9..0.9"
        );
        ensure!(unit3(&self.shadow_color), "shadow_color must be 0..1 RGB");
        ensure!(unit3(&self.rim_color), "rim_color must be 0..1 RGB");
        ensure!(
            in_range(self.rim_strength, 0.0, 1.0),
            "rim_strength must be 0..1"
        );
        ensure!(
            in_range(self.rim_power, 0.5, 16.0),
            "rim_power must be 0.5..16"
        );
        names(&self.materials, "materials")?;
        if let Some(skin) = &self.skin {
            ensure!(in_range(skin.wrap, 0.0, 1.0), "skin.wrap must be 0..1");
            ensure!(unit3(&skin.tint), "skin.tint must be 0..1 RGB");
            names(&skin.materials, "skin.materials")?;
        }
        if let Some(face) = &self.face {
            ensure!(
                face.bone.as_ref().is_none_or(|b| !b.is_empty()),
                "face.bone must name a bone"
            );
            let length = face.forward.iter().map(|v| v * v).sum::<f32>().sqrt();
            ensure!(
                length.is_finite() && length > 1e-3,
                "face.forward must be a non-zero direction"
            );
            ensure!(in_range(face.blend, 0.0, 1.0), "face.blend must be 0..1");
            names(&face.materials, "face.materials")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutlineUnits {
    /// Constant on screen (physical pixels of the view).
    #[default]
    Pixels,
    /// World meters along the surface normal (thinner with distance).
    Meters,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenderOutline {
    #[serde(default = "default_outline_width")]
    pub width: f32,
    #[serde(default)]
    pub units: OutlineUnits,
    #[serde(default = "default_outline_color")]
    pub color: [f32; 3],
    /// material names (glTF material.name) or node/mesh/primitive IDs that get the hull; empty = every opaque primitive.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<String>,
}
fn default_outline_width() -> f32 {
    2.0
}
fn default_outline_color() -> [f32; 3] {
    [0.05, 0.04, 0.07]
}
impl Default for RenderOutline {
    fn default() -> Self {
        serde_json::from_value(json!({})).expect("outline defaults")
    }
}
impl RenderOutline {
    pub fn validate(&self) -> Result<()> {
        match self.units {
            OutlineUnits::Pixels => ensure!(
                self.width.is_finite() && self.width > 0.0 && self.width <= 16.0,
                "width must be 0..16 pixels (above 0)"
            ),
            OutlineUnits::Meters => ensure!(
                self.width.is_finite() && self.width > 0.0 && self.width <= 0.2,
                "width must be 0..0.2 meters (above 0)"
            ),
        }
        ensure!(unit3(&self.color), "color must be 0..1 RGB");
        names(&self.materials, "materials")
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecalBlend {
    /// Over the base colour by the image alpha.
    #[default]
    Alpha,
    /// Base colour times the image (darkening details).
    Multiply,
    /// Image added (glows, highlights).
    Add,
}

/// Projects the decal image onto the surface along -Z of a box (`position`, `rotation`,
/// `size`) in the space of `bone` (else the entity). The image covers the box's X (left to
/// right) and Y (bottom to top); surfaces facing away from +Z are not painted.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DecalProjector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default = "identity")]
    pub rotation: [f32; 4],
    #[serde(default = "default_size")]
    pub size: [f32; 3],
}
fn identity() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}
fn default_size() -> [f32; 3] {
    [0.2, 0.2, 0.2]
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenderDecal {
    /// PNG image asset (`ui.document/images` ID).
    pub image: String,
    #[serde(default)]
    pub blend: DecalBlend,
    #[serde(default = "one")]
    pub opacity: f32,
    /// UV mapping: the rectangle `[u0, v0, u1, v1]` of the mesh's texture coordinates the
    /// image covers (glTF UV: v grows downward). Ignored with `projector`.
    #[serde(default = "full_rect")]
    pub rect: [f32; 4],
    /// Projected mapping (meshes without texture coordinates, or placement in 3D).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projector: Option<DecalProjector>,
    /// material names (glTF material.name) or node/mesh/primitive IDs painted; empty = every primitive.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<String>,
}
fn full_rect() -> [f32; 4] {
    [0.0, 0.0, 1.0, 1.0]
}
impl RenderDecal {
    pub fn validate(&self) -> Result<()> {
        crate::valid_id(&self.image)?;
        ensure!(in_range(self.opacity, 0.0, 1.0), "opacity must be 0..1");
        ensure!(
            self.rect.iter().all(|v| in_range(*v, -16.0, 16.0))
                && self.rect[2] - self.rect[0] > 1e-4
                && self.rect[3] - self.rect[1] > 1e-4,
            "rect must be [u0, v0, u1, v1] with u1 > u0 and v1 > v0 (|values| <= 16)"
        );
        if let Some(p) = &self.projector {
            ensure!(
                p.bone.as_ref().is_none_or(|b| !b.is_empty()),
                "projector.bone must name a bone"
            );
            ensure!(
                p.position.iter().all(|v| in_range(*v, -1000.0, 1000.0)),
                "projector.position must be finite (|values| <= 1000)"
            );
            let length = p.rotation.iter().map(|v| v * v).sum::<f32>().sqrt();
            ensure!(
                length.is_finite() && length > 1e-3,
                "projector.rotation must be a quaternion [x, y, z, w]"
            );
            ensure!(
                p.size
                    .iter()
                    .all(|v| v.is_finite() && *v > 1e-4 && *v <= 100.0),
                "projector.size must be 0..100 on every axis (above 0)"
            );
        }
        names(&self.materials, "materials")
    }
}

pub fn toon_schema() -> Value {
    json!({
        "bands": "1..8, default 2: lit steps between the shadow colour and full light",
        "softness": "0..1, default 0.1: band edge width (0 = hard)",
        "threshold": "-0.9..0.9, default 0: where the shadow side ends (light cosine)",
        "shadow_color": "RGB 0..1, default [0.62,0.56,0.74]: multiplies the base colour on the shadow side",
        "rim_color": "RGB 0..1, default [1,1,1]",
        "rim_strength": "0..1, default 0.3",
        "rim_power": "0.5..16, default 4: higher = thinner rim",
        "materials": "material names (glTF material.name) or node/mesh/primitive IDs of the mesh the look applies to; empty = all",
        "skin": "optional {wrap 0..1 default 0.5, tint RGB default [1,0.36,0.26], materials}: wrap lighting + tinted terminator, an APPROXIMATION of light bleeding through skin (not subsurface scattering)",
        "face": "optional {bone?, forward [x,y,z] default [0,0,1], blend 0..1 default 1, materials}: shading normal of the face primitives bends toward forward in the bone's (else the entity's) space; approximation, no SDF face map",
        "requires": "an imported (animation.player) or procedural (render.geometry) mesh; the built-in mesh shader only (ignored with render.material). Unlit display shows the flat base colour. Replaces render.surface reflections on the primitives it covers"
    })
}

pub fn outline_schema() -> Value {
    json!({
        "width": "default 2: pixels 0..16 or meters 0..0.2 (above 0)",
        "units": "pixels | meters, default pixels",
        "color": "RGB 0..1, default [0.05,0.04,0.07]",
        "materials": "material names (glTF material.name) or node/mesh/primitive IDs that get the outline; empty = all",
        "requires": "an imported or procedural mesh. Inverted hull: the primitive drawn again, pushed out along its normals with front faces culled; opaque single-sided primitives only (masked / blended / double-sided ones get no hull). Hard edges with split normals show gaps"
    })
}

pub fn decal_schema() -> Value {
    json!({
        "image": "PNG image asset ID (ui.document images)",
        "blend": "alpha | multiply | add, default alpha",
        "opacity": "0..1, default 1",
        "rect": "[u0,v0,u1,v1], default [0,0,1,1]: UV rectangle of the mesh the image covers (glTF UV, v down); unused with projector",
        "projector": "optional {bone?, position [x,y,z], rotation [x,y,z,w], size [x,y,z] default 0.2}: box projection along its -Z in the bone's (else the entity's) space, for meshes without UVs; faces turned away from +Z are not painted",
        "materials": "material names (glTF material.name) or node/mesh/primitive IDs painted; empty = all",
        "requires": "an imported or procedural mesh; one decal layer per entity. Applies with the built-in and render.material shaders"
    })
}

/// A `render.decal` must name an image the world has.
pub fn validate_world(world: &crate::World) -> Result<()> {
    let owners = world
        .entities
        .iter()
        .map(|e| (format!("@{}", crate::local::escape(&e.id)), &e.components))
        .chain(
            world
                .prefabs
                .iter()
                .map(|(id, p)| (crate::local::escape(id), &p.components)),
        );
    for (owner, components) in owners {
        let Some(value) = components.get("render.decal") else {
            continue;
        };
        let Some(image) = value.get("image").and_then(Value::as_str) else {
            continue;
        };
        if !crate::assets::exists(world, crate::assets::AssetKind::Image, image) {
            let kind = if owner.starts_with('@') {
                "entities"
            } else {
                "prefabs"
            };
            return Err(crate::Diagnostic::error(
                "unknown_image",
                format!("/{kind}/{owner}/components/render.decal/image"),
                format!("{image} is not an image of ui.document"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_serialize_and_validate() {
        let toon = RenderToon::default();
        toon.validate().unwrap();
        assert_eq!(toon.bands, 2);
        let written = serde_json::to_value(&toon).unwrap();
        assert!(written.get("skin").is_none() && written.get("materials").is_none());
        RenderOutline::default().validate().unwrap();
        let decal: RenderDecal = serde_json::from_value(json!({"image":"face"})).unwrap();
        decal.validate().unwrap();
        assert_eq!(decal.rect, [0.0, 0.0, 1.0, 1.0]);
        assert!(serde_json::from_value::<RenderDecal>(json!({})).is_err());
    }

    #[test]
    fn ranges_are_enforced() {
        for bad in [
            json!({"bands":0}),
            json!({"bands":9}),
            json!({"softness":1.5}),
            json!({"threshold":1.0}),
            json!({"shadow_color":[2,0,0]}),
            json!({"rim_power":0.1}),
            json!({"skin":{"wrap":-0.1}}),
            json!({"face":{"forward":[0,0,0]}}),
            json!({"face":{"bone":""}}),
            json!({"materials":[""]}),
            json!({"sss":1}),
        ] {
            let parsed: Result<RenderToon> =
                serde_json::from_value(bad.clone()).map_err(Into::into);
            assert!(parsed.and_then(|t| t.validate()).is_err(), "{bad}");
        }
        for bad in [
            json!({"width":0}),
            json!({"width":20}),
            json!({"units":"meters","width":0.5}),
            json!({"units":"inches"}),
        ] {
            let parsed: Result<RenderOutline> =
                serde_json::from_value(bad.clone()).map_err(Into::into);
            assert!(parsed.and_then(|t| t.validate()).is_err(), "{bad}");
        }
        for bad in [
            json!({"image":"face","rect":[0.5,0,0.5,1]}),
            json!({"image":"face","opacity":2}),
            json!({"image":"face","projector":{"size":[0,1,1]}}),
            json!({"image":"face","projector":{"rotation":[0,0,0,0]}}),
            json!({"image":"face","blend":"screen"}),
        ] {
            let parsed: Result<RenderDecal> =
                serde_json::from_value(bad.clone()).map_err(Into::into);
            assert!(parsed.and_then(|t| t.validate()).is_err(), "{bad}");
        }
    }

    #[test]
    fn material_filters_select_all_when_empty() {
        assert!(selects(&[], "0/0/1", None));
        assert!(selects(&["skin".into()], "0/0/1", Some("skin")));
        assert!(selects(&["0/0/1".into()], "0/0/1", Some("skin")));
        assert!(!selects(&["skin".into()], "0/0/2", Some("hair")));
        assert!(!selects(&["skin".into()], "0/0/2", None));
    }
}
