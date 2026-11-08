//! Versioned Unity editor interchange; evaluated bone TRS, not serialized Mecanim muscle curves.
use super::*;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub version: u32,
    pub coordinates: coordinates::Coordinates,
    pub skeleton: Skeleton,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub materials: Vec<Material>,
    pub textures: Vec<String>,
    pub clips: Vec<clip::Clip>,
    pub humanoid: humanoid::Profile,
}
pub fn load(path: &Path, options: &ImportOptions) -> Result<Asset> {
    let mut b: Bundle = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(b.version == 1, "unsupported .axrig version");
    let c = options.coordinates.unwrap_or(b.coordinates);
    let matrix = c.matrix()?;
    for bone in &mut b.skeleton.bones {
        bone.rest = Trs::from_matrix(c.transform(bone.rest.matrix())?)?;
        bone.inverse_bind = c.transform(bone.inverse_bind)?;
    }
    for v in &mut b.vertices {
        v.position = c.point(v.position)?;
        v.normal = matrix.transform_vector3(v.normal).normalize_or_zero();
        if let Some(d) = &mut v.sdef {
            d.c = c.point(d.c)?;
            d.r0 = c.point(d.r0)?;
            d.r1 = c.point(d.r1)?;
        }
    }
    if c.reflected()? {
        for t in b.indices.as_chunks_mut::<3>().0 {
            t.swap(1, 2)
        }
    }
    for clip in &mut b.clips {
        for t in &mut clip.tracks {
            for v in &mut t.values {
                *v = match t.property {
                    clip::Property::Translation => c.point(v.truncate())?.extend(0.),
                    clip::Property::Rotation => (matrix.transform_vector3(v.truncate())
                        * matrix.determinant().signum())
                    .extend(v.w),
                    clip::Property::Scale => {
                        coordinates::scale_in_basis(matrix, v.truncate())?.extend(0.)
                    }
                };
            }
        }
    }
    for reference in b.humanoid.reference.values_mut() {
        *reference = Trs::from_matrix(c.transform(reference.matrix())?)?;
    }
    ensure!(b.textures.len() <= 128, "texture count budget exceeded");
    let mut textures = vec![];
    for source in b.textures {
        textures.push(super::texture(&std::fs::read(confined(
            path.parent().unwrap_or(Path::new(".")),
            &source,
        )?)?)?);
        ensure!(
            textures.iter().map(|t| t.rgba.len()).sum::<usize>() <= 268_435_456,
            "decoded textures exceed 256 MiB"
        );
    }
    let skeleton = b.skeleton.compile()?;
    b.humanoid.compile(&skeleton, true)?;
    Ok(Asset {
        key: String::new(),
        skeleton,
        vertices: b.vertices,
        indices: b.indices,
        materials: b.materials,
        textures,
        clips: b.clips,
        diagnostics: vec![],
        humanoid: Some(b.humanoid),
    })
}
