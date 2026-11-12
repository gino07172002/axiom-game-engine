//! `render.toon`, `render.outline` and `render.decal` resolved once per runtime build into
//! per-primitive shader parameters ([`MeshStyle`]); the mesh renderer adds the per-frame part
//! (bone-driven face direction and decal projector) from [`MeshFrame::style_space`].
//!
//! [`MeshFrame::style_space`]: crate::presentation::MeshFrame::style_space
use anyhow::{Context, Result};
use axiom_animation::{Asset, CompiledSkeleton, Pose};
use axiom_core::{
    local::{at, escape},
    style::{selects, DecalBlend, OutlineUnits, RenderDecal, RenderOutline, RenderToon},
};
use glam::{Mat4, Quat, Vec3, Vec4};
use std::sync::Arc;

/// Decoded decal image (RGBA8, sRGB), shared across runtime builds while the file is unchanged.
pub struct DecalImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Shader parameters of one primitive (glTF material) of the mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PrimitiveStyle {
    /// (bands, 0 = no toon look; softness; threshold; skin wrap).
    pub toon: [f32; 4],
    pub rim_power: f32,
    /// 0 = mesh normals; the face direction is added per frame.
    pub face_blend: f32,
    /// > 0 meters, < 0 pixels, 0 = no outline.
    pub outline: f32,
    /// The primitive is painted by the entity's decal.
    pub decal: bool,
    /// Packed RGBA8 (unorm): shadow colour, rim (a = strength), skin tint, outline colour.
    pub colors: [u32; 4],
}

pub enum Mapping {
    /// `[u0, v0, u1, v1]` of the mesh UVs.
    Uv([f32; 4]),
    /// Box (`local`: translation * rotation * size) in the space of `style_space[1]`.
    Project(Mat4),
}

pub struct Decal {
    pub image: Arc<DecalImage>,
    /// 1 alpha, 2 multiply, 3 add (the shader's decal mode; +4 when projected).
    pub blend: f32,
    pub opacity: f32,
    pub mapping: Mapping,
}

pub struct MeshStyle {
    /// Indexed like `Asset::materials`.
    pub primitives: Vec<PrimitiveStyle>,
    /// Face bone and the authored forward axis in its space.
    pub face: Option<(Option<usize>, Vec3)>,
    pub decal: Option<Decal>,
    /// Bone whose world matrix is `style_space[1]` (the decal projector's space).
    pub decal_bone: Option<usize>,
}

/// The decal rows the shader dots with `(uv, 0, 1)` (UV mapping) or `(world, 1)` (projected):
/// image u, image v and box depth (-0.5..0.5 inside the box).
pub fn decal_rows(mapping: &Mapping, space: Mat4) -> [Vec4; 3] {
    match mapping {
        Mapping::Uv([u0, v0, u1, v1]) => {
            let (su, sv) = (1.0 / (u1 - u0), 1.0 / (v1 - v0));
            [
                Vec4::new(su, 0.0, 0.0, -u0 * su),
                Vec4::new(0.0, sv, 0.0, -v0 * sv),
                Vec4::ZERO,
            ]
        }
        Mapping::Project(local) => {
            let inverse = (space * *local).inverse();
            let half = Vec4::new(0.0, 0.0, 0.0, 0.5);
            [inverse.row(0) + half, half - inverse.row(1), inverse.row(2)]
        }
    }
}

/// World matrix of `bone` relative to the entity, walking its parents (no allocation).
pub fn bone_global(skeleton: &CompiledSkeleton, pose: &Pose, bone: usize) -> Mat4 {
    if bone >= pose.local.len() || pose.local.len() != skeleton.source.bones.len() {
        return Mat4::IDENTITY;
    }
    let mut matrix = pose.local[bone].matrix();
    let mut parent = skeleton.source.bones[bone].parent;
    while let Some(p) = parent {
        matrix = pose.local[p].matrix() * matrix;
        parent = skeleton.source.bones[p].parent;
    }
    matrix
}

/// `[face space, decal space]` for one frame: the bone's world matrix, else the entity's.
pub fn spaces(style: &MeshStyle, model: Mat4, bone: impl Fn(usize) -> Mat4) -> [Mat4; 2] {
    let space = |b: Option<usize>| b.map_or(model, |b| model * bone(b));
    [
        space(style.face.and_then(|(b, _)| b)),
        space(style.decal_bone),
    ]
}

fn pack(rgb: [f32; 3], a: f32) -> u32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(rgb[0]) | byte(rgb[1]) << 8 | byte(rgb[2]) << 16 | byte(a) << 24
}

/// The style of slot `slot`'s mesh, or `None` when it has no style component. Errors name the
/// component field (unknown material or bone, unreadable image).
pub fn prepare(
    state: &crate::world::RuntimeState,
    slot: usize,
    asset: &Asset,
) -> Result<Option<Arc<MeshStyle>>> {
    let entity = &state.compiled.entities[slot];
    let get = |name: &str| entity.components.get(name);
    if get("render.toon").is_none()
        && get("render.outline").is_none()
        && get("render.decal").is_none()
    {
        return Ok(None);
    }
    let path = |c: &str| format!("/entities/@{}/components/{c}", escape(&entity.id));
    let ids: Vec<(&str, Option<&str>)> = asset
        .materials
        .iter()
        .map(|m| (m.id.as_str(), m.name.as_deref()))
        .collect();
    let listed = ids
        .iter()
        .map(|(id, name)| name.unwrap_or(id))
        .collect::<Vec<_>>()
        .join(", ");
    let known = |list: &[String], field: &str| -> Result<()> {
        for name in list {
            anyhow::ensure!(
                ids.iter()
                    .any(|(id, n)| *id == name || *n == Some(name.as_str())),
                "{field}: unknown material {name} (the mesh has: {})",
                listed
            );
        }
        Ok(())
    };
    let bone = |name: &Option<String>, field: &str| -> Result<Option<usize>> {
        name.as_ref()
            .map(|b| {
                asset
                    .skeleton
                    .indices
                    .get(b)
                    .copied()
                    .with_context(|| format!("{field}: unknown bone {b}"))
            })
            .transpose()
    };
    let mut primitives = vec![PrimitiveStyle::default(); asset.materials.len()];
    let mut face = None;
    if let Some(value) = get("render.toon") {
        let toon: RenderToon = serde_json::from_value(value.clone())?;
        at(
            &path("render.toon"),
            (|| -> Result<()> {
                known(&toon.materials, "materials")?;
                if let Some(skin) = &toon.skin {
                    known(&skin.materials, "skin.materials")?;
                }
                if let Some(f) = &toon.face {
                    known(&f.materials, "face.materials")?;
                    let axis = Vec3::from(f.forward).normalize();
                    face = Some((bone(&f.bone, "face.bone")?, axis));
                }
                Ok(())
            })(),
        )?;
        for (p, &(id, name)) in primitives.iter_mut().zip(&ids) {
            if !selects(&toon.materials, id, name) {
                continue;
            }
            let skin = toon
                .skin
                .as_ref()
                .filter(|s| selects(&s.materials, id, name));
            p.toon = [
                toon.bands as f32,
                toon.softness,
                toon.threshold,
                skin.map_or(0.0, |s| s.wrap),
            ];
            p.rim_power = toon.rim_power;
            p.face_blend = toon
                .face
                .as_ref()
                .filter(|f| selects(&f.materials, id, name))
                .map_or(0.0, |f| f.blend);
            p.colors[0] = pack(toon.shadow_color, 1.0);
            p.colors[1] = pack(toon.rim_color, toon.rim_strength);
            p.colors[2] = pack(skin.map_or([0.0; 3], |s| s.tint), 1.0);
        }
    }
    if let Some(value) = get("render.outline") {
        let outline: RenderOutline = serde_json::from_value(value.clone())?;
        at(
            &path("render.outline"),
            known(&outline.materials, "materials"),
        )?;
        let width = match outline.units {
            OutlineUnits::Meters => outline.width,
            OutlineUnits::Pixels => -outline.width,
        };
        for (p, &(id, name)) in primitives.iter_mut().zip(&ids) {
            if selects(&outline.materials, id, name) {
                p.outline = width;
                p.colors[3] = pack(outline.color, 1.0);
            }
        }
    }
    let mut decal = None;
    let mut decal_bone = None;
    if let Some(value) = get("render.decal") {
        let d: RenderDecal = serde_json::from_value(value.clone())?;
        let mapping = at(
            &path("render.decal"),
            (|| -> Result<Mapping> {
                known(&d.materials, "materials")?;
                Ok(match &d.projector {
                    None => Mapping::Uv(d.rect),
                    Some(p) => {
                        decal_bone = bone(&p.bone, "projector.bone")?;
                        Mapping::Project(Mat4::from_scale_rotation_translation(
                            Vec3::from(p.size),
                            Quat::from_array(p.rotation).normalize(),
                            Vec3::from(p.position),
                        ))
                    }
                })
            })(),
        )?;
        let image = at(
            &format!("{}/image", path("render.decal")),
            image_path(state, &d.image).and_then(|p| load_image(&p)),
        )?;
        for (p, &(id, name)) in primitives.iter_mut().zip(&ids) {
            p.decal = selects(&d.materials, id, name);
        }
        decal = Some(Decal {
            image,
            blend: match d.blend {
                DecalBlend::Alpha => 1.0,
                DecalBlend::Multiply => 2.0,
                DecalBlend::Add => 3.0,
            },
            opacity: d.opacity,
            mapping,
        });
    }
    Ok(Some(Arc::new(MeshStyle {
        primitives,
        face,
        decal,
        decal_bone,
    })))
}

fn image_path(state: &crate::world::RuntimeState, id: &str) -> Result<String> {
    state
        .compiled
        .source
        .resources
        .get("ui.document")
        .and_then(|ui| ui.get("images"))
        .and_then(|images| images.get(id))
        .and_then(|p| p.as_str())
        .map(str::to_owned)
        .with_context(|| format!("{id} is not an image of ui.document"))
}

/// Decoded decal images keyed by path, reused while the file bytes hash the same. Bounded;
/// unused entries are evicted first.
const IMAGE_CACHE_ENTRIES: usize = 32;
type ImageCache = std::collections::BTreeMap<String, (u64, Arc<DecalImage>)>;
static IMAGE_CACHE: std::sync::Mutex<ImageCache> = std::sync::Mutex::new(ImageCache::new());

fn load_image(path: &str) -> Result<Arc<DecalImage>> {
    use std::hash::{Hash, Hasher};
    anyhow::ensure!(
        std::fs::metadata(path)
            .with_context(|| format!("missing decal image {path}"))?
            .len()
            <= crate::bound_ui::IMAGE_BYTES,
        "decal image exceeds 4 MiB"
    );
    let raw = std::fs::read(path)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut hasher);
    let hash = hasher.finish();
    if let Some((_, image)) = IMAGE_CACHE
        .lock()
        .ok()
        .and_then(|c| c.get(path).filter(|(h, _)| *h == hash).cloned())
    {
        return Ok(image);
    }
    let (width, height, rgba) = crate::bound_ui::decode_png(&raw)?;
    let image = Arc::new(DecalImage {
        width,
        height,
        rgba,
    });
    if let Ok(mut cache) = IMAGE_CACHE.lock() {
        if cache.len() >= IMAGE_CACHE_ENTRIES && !cache.contains_key(path) {
            cache.retain(|_, (_, i)| Arc::strong_count(i) > 1);
        }
        if cache.len() < IMAGE_CACHE_ENTRIES || cache.contains_key(path) {
            cache.insert(path.to_owned(), (hash, image.clone()));
        }
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv_rows_map_the_rect_to_the_unit_square() {
        let rows = decal_rows(&Mapping::Uv([0.25, 0.5, 0.75, 1.0]), Mat4::IDENTITY);
        let at = |u: f32, v: f32| {
            let p = Vec4::new(u, v, 0.0, 1.0);
            (rows[0].dot(p), rows[1].dot(p), rows[2].dot(p))
        };
        assert_eq!(at(0.25, 0.5), (0.0, 0.0, 0.0));
        assert_eq!(at(0.75, 1.0), (1.0, 1.0, 0.0));
        assert_eq!(at(0.5, 0.75), (0.5, 0.5, 0.0));
    }

    #[test]
    fn projected_rows_follow_the_space_and_put_the_image_top_at_plus_y() {
        let local = Mat4::from_scale_rotation_translation(
            Vec3::new(0.2, 0.1, 0.4),
            Quat::IDENTITY,
            Vec3::new(0.0, 1.0, 0.0),
        );
        // The bone (space) moved 2 m along X: the box moves with it.
        let space = Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0));
        let rows = decal_rows(&Mapping::Project(local), space);
        let at = |p: Vec3| {
            let p = p.extend(1.0);
            [rows[0].dot(p), rows[1].dot(p), rows[2].dot(p)]
        };
        let centre = at(Vec3::new(2.0, 1.0, 0.0));
        assert!(centre
            .iter()
            .zip([0.5, 0.5, 0.0])
            .all(|(a, b)| (a - b).abs() < 1e-5));
        // Top-left corner of the image: -X, +Y of the box.
        let top_left = at(Vec3::new(1.9, 1.05, 0.0));
        assert!((top_left[0]).abs() < 1e-5 && (top_left[1]).abs() < 1e-5);
        // Depth spans -0.5..0.5 over the box's Z size; the row points along +Z.
        assert!((at(Vec3::new(2.0, 1.0, 0.2))[2] - 0.5).abs() < 1e-5);
        assert!(rows[2].truncate().normalize().abs_diff_eq(Vec3::Z, 1e-5));
    }

    #[test]
    fn bone_global_walks_the_parent_chain() {
        use axiom_animation::{Bone, Skeleton, Trs};
        let bone = |id: &str, parent, y: f32| Bone {
            id: id.into(),
            aliases: vec![],
            parent,
            rest: Trs {
                translation: Vec3::new(0.0, y, 0.0),
                ..Trs::default()
            },
            inverse_bind: Mat4::IDENTITY,
        };
        let skeleton = Skeleton {
            bones: vec![bone("root", None, 1.0), bone("head", Some(0), 0.5)],
        }
        .compile()
        .unwrap();
        let pose = skeleton.rest_pose();
        let head = bone_global(&skeleton, &pose, 1);
        assert!(head
            .w_axis
            .truncate()
            .abs_diff_eq(Vec3::new(0.0, 1.5, 0.0), 1e-6));
        assert_eq!(bone_global(&skeleton, &pose, 9), Mat4::IDENTITY);
    }

    #[test]
    fn colours_pack_as_rgba8_unorm() {
        assert_eq!(pack([1.0, 0.0, 0.0], 1.0), 0xff0000ff);
        assert_eq!(pack([0.0, 1.0, 2.0], 0.0), 0x00ffff00);
    }
}
