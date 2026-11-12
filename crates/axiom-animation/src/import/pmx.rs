//! PMX 2.0/2.1 mesh and deform rig reader. Unsupported physics/morph blocks are diagnosed.
use super::*;
use glam::Vec4;
struct Reader {
    data: Vec<u8>,
    offset: usize,
    utf8: bool,
    sizes: [u8; 6],
}
impl Reader {
    fn bytes(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.offset.checked_add(n).context("PMX offset overflow")?;
        let b = self.data.get(self.offset..end).context("truncated PMX")?;
        self.offset = end;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into()?))
    }
    fn f32(&mut self) -> Result<f32> {
        let f = f32::from_le_bytes(self.bytes(4)?.try_into()?);
        ensure!(f.is_finite(), "non-finite PMX float");
        Ok(f)
    }
    fn v3(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn v4(&mut self) -> Result<Vec4> {
        Ok(Vec4::new(
            self.f32()?,
            self.f32()?,
            self.f32()?,
            self.f32()?,
        ))
    }
    fn text(&mut self) -> Result<String> {
        let n = self.count(1_048_576)?;
        let utf8 = self.utf8;
        let b = self.bytes(n)?;
        if utf8 {
            Ok(std::str::from_utf8(b)?.into())
        } else {
            ensure!(n.is_multiple_of(2), "odd UTF-16 string length");
            let words = b
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>();
            Ok(String::from_utf16(&words)?)
        }
    }
    fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.i32()?;
        ensure!(
            n >= 0 && (n as usize) <= max,
            "PMX count exceeds {max}: {n}"
        );
        Ok(n as usize)
    }
    fn index(&mut self, kind: usize) -> Result<i32> {
        match self.sizes[kind] {
            1 => Ok(if kind == 0 {
                self.u8()? as i32
            } else {
                self.u8()? as i8 as i32
            }),
            2 => {
                let v = self.u16()?;
                Ok(if kind == 0 { v as i32 } else { v as i16 as i32 })
            }
            4 => self.i32(),
            _ => anyhow::bail!("invalid PMX index width"),
        }
    }
}
pub fn load(path: &Path, options: &ImportOptions) -> Result<Asset> {
    let mut r = Reader {
        data: std::fs::read(path)?,
        offset: 0,
        utf8: false,
        sizes: [0; 6],
    };
    ensure!(r.bytes(4)? == b"PMX ", "invalid PMX header");
    let version = r.f32()?;
    ensure!(
        (version - 2.).abs() < 0.001 || (version - 2.1).abs() < 0.001,
        "unsupported PMX version"
    );
    let header = r.u8()?;
    ensure!(header == 8, "unsupported PMX header size");
    r.utf8 = match r.u8()? {
        0 => false,
        1 => true,
        _ => anyhow::bail!("invalid PMX text encoding"),
    };
    let extra = r.u8()?;
    ensure!(extra <= 4, "too many PMX UV channels");
    for i in 0..6 {
        r.sizes[i] = r.u8()?;
        ensure!([1, 2, 4].contains(&r.sizes[i]), "invalid PMX index size");
    }
    let _name = r.text()?;
    r.text()?;
    r.text()?;
    r.text()?;
    let mut coord = options
        .coordinates
        .unwrap_or(coordinates::Coordinates::preset("mmd")?);
    if options.coordinates.is_none() {
        coord.meters_per_unit = 0.08;
    }
    let basis = coord.matrix()?;
    let n = r.count(MAX_VERTICES)?;
    let mut vertices = Vec::with_capacity(n);
    for _ in 0..n {
        let position = coord.point(r.v3()?)?;
        let normal = basis.transform_vector3(r.v3()?).normalize_or_zero();
        let uv = Vec2::new(r.f32()?, r.f32()?);
        for _ in 0..extra {
            r.v4()?;
        }
        let kind = r.u8()?;
        let mut joints = [0; 4];
        let mut weights = [0.; 4];
        let mut sdef = None;
        let mut read_joint = |r: &mut Reader, i: usize| -> Result<()> {
            let j = r.index(3)?;
            joints[i] = if j < 0 { usize::MAX } else { j as usize };
            Ok(())
        };
        match kind {
            0 => {
                read_joint(&mut r, 0)?;
                weights[0] = 1.;
            }
            1 | 3 => {
                read_joint(&mut r, 0)?;
                read_joint(&mut r, 1)?;
                weights[0] = r.f32()?;
                weights[1] = 1. - weights[0];
                if kind == 3 {
                    sdef = Some(Sdef {
                        c: coord.point(r.v3()?)?,
                        r0: coord.point(r.v3()?)?,
                        r1: coord.point(r.v3()?)?,
                    });
                }
            }
            2 | 4 => {
                for i in 0..4 {
                    read_joint(&mut r, i)?
                }
                weights = r.v4()?.to_array();
            }
            _ => anyhow::bail!("unsupported PMX skinning mode {kind}"),
        }
        for i in 0..4 {
            if weights[i] == 0. {
                joints[i] = 0;
            }
        }
        r.f32()?;
        vertices.push(Vertex {
            position,
            normal,
            uv,
            joints,
            weights,
            sdef,
            dual_quaternion: kind == 4,
        });
    }
    let n = r.count(MAX_INDICES)?;
    ensure!(n.is_multiple_of(3), "PMX indices must be triangles");
    let mut indices = Vec::with_capacity(n);
    for _ in 0..n {
        let i = r.index(0)?;
        ensure!(i >= 0, "negative vertex index");
        indices.push(i as u32);
    }
    if coord.reflected()? {
        for t in indices.as_chunks_mut::<3>().0 {
            t.swap(1, 2)
        }
    }
    let n = r.count(256)?;
    let mut texture_paths = vec![];
    for _ in 0..n {
        texture_paths.push(r.text()?)
    }
    let mut textures = vec![];
    let mut loaded = BTreeMap::new();
    let mut materials = vec![];
    let mut offset = 0;
    let n = r.count(256)?;
    for i in 0..n {
        let name = r.text()?;
        r.text()?;
        let color = r.v4()?.to_array();
        r.v3()?;
        r.f32()?;
        r.v3()?;
        let flags = r.u8()?;
        r.v4()?;
        r.f32()?;
        let tex = r.index(1)?;
        r.index(1)?;
        r.u8()?;
        match r.u8()? {
            0 => {
                r.index(1)?;
            }
            1 => {
                r.u8()?;
            }
            _ => anyhow::bail!("invalid PMX toon flag"),
        };
        r.text()?;
        let count = r.count(MAX_INDICES)? as u32;
        let texture = if tex >= 0 {
            let source = texture_paths
                .get(tex as usize)
                .context("PMX texture index out of range")?;
            if let Some(i) = loaded.get(source) {
                Some(*i)
            } else {
                let file = confined(path.parent().unwrap_or(Path::new(".")), source)?;
                let image = super::texture(&std::fs::read(file)?)
                    .with_context(|| format!("PMX texture {source}"))?;
                let i = textures.len();
                textures.push(image);
                loaded.insert(source.clone(), i);
                Some(i)
            }
        } else {
            None
        };
        materials.push(Material {
            id: format!("{i}/{name}"),
            first_index: offset,
            index_count: count,
            color,
            texture,
            double_sided: flags & 1 != 0,
            alpha: if color[3] < 0.999 {
                AlphaMode::Blend
            } else {
                AlphaMode::Mask(0.1)
            },
            groups: vec![],
        });
        offset = offset
            .checked_add(count)
            .context("material index overflow")?;
    }
    let n = r.count(MAX_BONES)?;
    let mut bones = vec![];
    let mut globals = vec![];
    let mut ids = std::collections::BTreeSet::new();
    let mut append = 0;
    let mut ik_count = 0;
    for i in 0..n {
        let mut id = r.text()?;
        let alias = r.text()?;
        if id.is_empty() {
            id = format!("bone/{i}")
        }
        if !ids.insert(id.clone()) {
            id = format!("{id}#{i}");
            ensure!(ids.insert(id.clone()), "duplicate PMX bone name")
        }
        let position = coord.point(r.v3()?)?;
        let parent = r.index(3)?;
        r.i32()?;
        let flags = r.u16()?;
        if flags & 1 != 0 {
            r.index(3)?;
        } else {
            r.v3()?;
        }
        if flags & 0x300 != 0 {
            r.index(3)?;
            r.f32()?;
            append += 1;
        }
        if flags & 0x400 != 0 {
            r.v3()?;
        }
        if flags & 0x800 != 0 {
            r.v3()?;
            r.v3()?;
        }
        if flags & 0x2000 != 0 {
            r.i32()?;
        }
        if flags & 0x20 != 0 {
            ik_count += 1;
            r.index(3)?;
            r.i32()?;
            r.f32()?;
            let links = r.count(MAX_BONES)?;
            for _ in 0..links {
                r.index(3)?;
                if r.u8()? != 0 {
                    r.v3()?;
                    r.v3()?;
                }
            }
        }
        globals.push(position);
        bones.push(Bone {
            id,
            aliases: if alias.is_empty() {
                vec![]
            } else {
                vec![alias]
            },
            parent: if parent < 0 {
                None
            } else {
                Some(parent as usize)
            },
            rest: Trs::default(),
            inverse_bind: Mat4::from_translation(-position),
        });
    }
    for i in 0..n {
        bones[i].rest.translation = globals[i]
            - if let Some(p) = bones[i].parent {
                *globals.get(p).context("PMX parent out of range")?
            } else {
                Vec3::ZERO
            };
    }
    let skeleton = Skeleton { bones }.compile()?;
    let mut diagnostics=vec!["PMX facial/vertex/material morphs, toon/sphere shaders and rigid-body/cloth physics are not evaluated; source blocks remain in the original file".into()];
    if append > 0 {
        diagnostics.push(format!("{append} PMX append-transform constraints require explicit twist/rig constraints or a baked glTF animation"));
    }
    if ik_count > 0 {
        diagnostics.push(format!("{ik_count} authoring IK controllers found; runtime uses configured two-bone IK, not PMX controller execution"));
    }
    Ok(Asset {
        key: String::new(),
        skeleton,
        vertices,
        indices,
        materials,
        textures,
        clips: vec![],
        diagnostics,
        humanoid: None,
        morphs: Default::default(),
    })
}
