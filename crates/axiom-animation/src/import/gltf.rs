//! glTF 2.0/GLB interchange: indexed triangles, skins, TRS clips and base-color textures.
use super::*;
use ::gltf::{animation::util::ReadOutputs, buffer::Source};
use base64::Engine;
fn uri(root: &Path, value: &str) -> Result<Vec<u8>> {
    if let Some(data) = value.strip_prefix("data:") {
        let (header, body) = data.split_once(',').context("invalid data URI")?;
        ensure!(
            header.ends_with(";base64") && body.len() <= 178_956_972,
            "unsupported/oversize data URI"
        );
        Ok(base64::engine::general_purpose::STANDARD.decode(body)?)
    } else {
        let mut bytes = vec![];
        let mut input = value.as_bytes().iter().copied();
        while let Some(b) = input.next() {
            if b == b'%' {
                let a = input.next().context("invalid URI escape")?;
                let b = input.next().context("invalid URI escape")?;
                let digits = [a, b];
                bytes.push(u8::from_str_radix(std::str::from_utf8(&digits)?, 16)?)
            } else {
                bytes.push(b)
            }
        }
        let path = confined(root, std::str::from_utf8(&bytes)?)?;
        ensure!(
            std::fs::metadata(&path)?.len() <= 134_217_728,
            "glTF dependency exceeds 128 MiB"
        );
        Ok(std::fs::read(path)?)
    }
}
pub fn load(path: &Path, options: &ImportOptions) -> Result<Asset> {
    let gltf = ::gltf::Gltf::open(path)?;
    let doc = &gltf.document;
    let root = path.parent().unwrap_or(Path::new("."));
    ensure!(
        doc.nodes().count() <= MAX_BONES
            && doc.images().count() <= 128
            && doc.materials().count() <= 256
            && doc.animations().count() <= 128,
        "glTF asset budget exceeded"
    );
    for extension in doc.extensions_required() {
        ensure!(
            extension == "KHR_materials_unlit",
            "required glTF extension is not supported: {extension}"
        );
    }
    let coord = options.coordinates.unwrap_or_default();
    let basis = coord.matrix()?;
    let mut buffers = vec![];
    let mut total = 0;
    for b in doc.buffers() {
        let data = match b.source() {
            Source::Bin => gltf.blob.as_ref().context("missing GLB BIN chunk")?.clone(),
            Source::Uri(value) => uri(root, value)?,
        };
        ensure!(data.len() >= b.length(), "truncated glTF buffer");
        total += data.len();
        ensure!(total <= 268_435_456, "total buffers exceed 256 MiB");
        buffers.push(data);
    }
    let mut textures = vec![];
    for image in doc.images() {
        let data = match image.source() {
            ::gltf::image::Source::View { view, .. } => buffers
                .get(view.buffer().index())
                .and_then(|b| b.get(view.offset()..view.offset() + view.length()))
                .context("invalid image buffer view")?
                .to_vec(),
            ::gltf::image::Source::Uri { uri: value, .. } => uri(root, value)?,
        };
        textures.push(super::texture(&data)?);
        ensure!(
            textures.iter().map(|t| t.rgba.len()).sum::<usize>() <= 268_435_456,
            "decoded textures exceed 256 MiB"
        );
    }
    let n = doc.nodes().count();
    ensure!(n > 0, "glTF has no nodes");
    let mut parents = vec![None; n];
    let mut bones = vec![];
    let mut names = BTreeMap::new();
    for node in doc.nodes() {
        for c in node.children() {
            ensure!(
                parents[c.index()].replace(node.index()).is_none(),
                "glTF node has multiple parents"
            );
        }
    }
    for node in doc.nodes() {
        let original = node.name().unwrap_or("").to_owned();
        let mut id = if original.is_empty() {
            format!("node/{}", node.index())
        } else {
            original.clone()
        };
        if names.contains_key(&id) {
            id = format!("{id}#{}", node.index())
        }
        ensure!(
            names.insert(id.clone(), node.index()).is_none(),
            "duplicate glTF stable node ID"
        );
        let m = Mat4::from_cols_array_2d(&node.transform().matrix());
        let rest = Trs::from_matrix(coord.transform(m)?)?;
        bones.push(Bone {
            id,
            aliases: vec![original],
            parent: parents[node.index()],
            rest,
            inverse_bind: Mat4::IDENTITY,
        });
    }
    let mut binds = BTreeMap::new();
    for skin in doc.skins() {
        let joints = skin.joints().collect::<Vec<_>>();
        let reader = skin.reader(|b| Some(buffers[b.index()].as_slice()));
        let matrices = reader
            .read_inverse_bind_matrices()
            .map(|r| r.map(|m| Mat4::from_cols_array_2d(&m)).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![Mat4::IDENTITY; joints.len()]);
        ensure!(
            matrices.len() == joints.len(),
            "skin inverse bind count mismatch"
        );
        for (j, m) in joints.iter().zip(matrices) {
            let m = coord.transform(m)?;
            if let Some(previous) = binds.insert(j.index(), m) {
                ensure!(m.abs_diff_eq(previous,1e-4),"multiple skins require different inverse binds for the same joint; split the assets");
            }
            bones[j.index()].inverse_bind = m;
        }
    }
    let skeleton = Skeleton { bones }.compile()?;
    let mut vertices = vec![];
    let mut indices = vec![];
    let mut materials = vec![];
    let mut diagnostics = vec![];
    let mut morphs = morph::Builder::default();
    // Node index -> the asset morph targets of its mesh (weight channels target nodes).
    let mut morph_nodes: BTreeMap<usize, Vec<u16>> = BTreeMap::new();
    for node in doc.nodes() {
        if let Some(mesh) = node.mesh() {
            let joint_map = node
                .skin()
                .map(|s| s.joints().map(|j| j.index()).collect::<Vec<_>>());
            let node_id = skeleton.source.bones[node.index()].id.clone();
            let targets = mesh_targets(&node, &mesh, &node_id, &mut morphs)?;
            if !targets.is_empty() {
                morph_nodes.insert(node.index(), targets.clone());
            }
            for primitive in mesh.primitives() {
                ensure!(
                    primitive.mode() == ::gltf::mesh::Mode::Triangles,
                    "only triangle primitives are supported"
                );
                ensure!(
                    primitive.morph_targets().count() == targets.len(),
                    "every primitive of glTF mesh {} must have the same morph targets",
                    mesh.index()
                );
                ensure!(
                    primitive.get(&::gltf::Semantic::Joints(1)).is_none(),
                    "more than four skin influences must be reduced by the exporter"
                );
                let reader = primitive.reader(|b| Some(buffers[b.index()].as_slice()));
                let positions = reader
                    .read_positions()
                    .context("mesh has no POSITION")?
                    .collect::<Vec<_>>();
                ensure!(
                    vertices.len() + positions.len() <= MAX_VERTICES,
                    "vertex budget exceeded"
                );
                let normals = reader.read_normals().map(|v| v.collect::<Vec<_>>());
                let uv = reader
                    .read_tex_coords(0)
                    .map(|v| v.into_f32().collect::<Vec<_>>());
                let js = reader
                    .read_joints(0)
                    .map(|v| v.into_u16().collect::<Vec<_>>());
                let ws = reader
                    .read_weights(0)
                    .map(|v| v.into_f32().collect::<Vec<_>>());
                if joint_map.is_some() {
                    ensure!(
                        js.as_ref().is_some_and(|x| x.len() == positions.len())
                            && ws.as_ref().is_some_and(|x| x.len() == positions.len()),
                        "skinned primitive requires JOINTS_0 and WEIGHTS_0"
                    );
                }
                ensure!(
                    normals.as_ref().is_none_or(|v| v.len() == positions.len())
                        && uv.as_ref().is_none_or(|v| v.len() == positions.len()),
                    "vertex attribute count mismatch"
                );
                let base = vertices.len() as u32;
                for (i, p) in positions.iter().enumerate() {
                    let mut joints = [node.index(), 0, 0, 0];
                    let mut weights = [1., 0., 0., 0.];
                    if let Some(map) = &joint_map {
                        let j = js.as_ref().unwrap()[i];
                        weights = ws.as_ref().unwrap()[i];
                        let sum: f32 = weights.iter().sum();
                        ensure!(
                            sum > 1e-6 && weights.iter().all(|w| w.is_finite() && *w >= 0.),
                            "invalid glTF weights"
                        );
                        for k in 0..4 {
                            weights[k] /= sum;
                            joints[k] = if weights[k] > 0. {
                                *map.get(j[k] as usize)
                                    .context("joint index out of skin range")?
                            } else {
                                0
                            };
                        }
                    }
                    vertices.push(Vertex {
                        position: coord.point(Vec3::from_array(*p))?,
                        normal: basis
                            .transform_vector3(
                                normals.as_ref().map_or(Vec3::Y, |v| Vec3::from_array(v[i])),
                            )
                            .normalize_or_zero(),
                        uv: uv.as_ref().map_or(Vec2::ZERO, |v| Vec2::from_array(v[i])),
                        joints,
                        weights,
                        sdef: None,
                        dual_quaternion: false,
                    });
                }
                let start = indices.len() as u32;
                let mut local = reader
                    .read_indices()
                    .map(|v| v.into_u32().collect::<Vec<_>>())
                    .unwrap_or_else(|| (0..positions.len() as u32).collect());
                ensure!(
                    local.len().is_multiple_of(3)
                        && indices.len() + local.len() <= MAX_INDICES
                        && local.iter().all(|i| (*i as usize) < positions.len()),
                    "invalid mesh index buffer"
                );
                if coord.reflected()? {
                    for t in local.as_chunks_mut::<3>().0 {
                        t.swap(1, 2)
                    }
                }
                if normals.is_none() {
                    let mut generated = vec![Vec3::ZERO; positions.len()];
                    for t in local.as_chunks::<3>().0 {
                        let p = |i: u32| vertices[(base + i) as usize].position;
                        let normal = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
                        for &i in t {
                            generated[i as usize] += normal;
                        }
                    }
                    for (i, n) in generated.iter().enumerate() {
                        vertices[base as usize + i].normal = n.try_normalize().unwrap_or(Vec3::Y);
                    }
                }
                indices.extend(local.into_iter().map(|i| i + base));
                for (j, (dp, dn, _)) in reader.read_morph_targets().enumerate() {
                    let dp = dp.map(|v| v.collect::<Vec<_>>());
                    let dn = dn.map(|v| v.collect::<Vec<_>>());
                    ensure!(
                        dp.as_ref().is_none_or(|v| v.len() == positions.len())
                            && dn.as_ref().is_none_or(|v| v.len() == positions.len()),
                        "morph target attribute count mismatch"
                    );
                    for i in 0..positions.len() {
                        let p = dp.as_ref().map_or(Vec3::ZERO, |v| Vec3::from_array(v[i]));
                        let n = dn.as_ref().map_or(Vec3::ZERO, |v| Vec3::from_array(v[i]));
                        morphs.push(
                            base + i as u32,
                            targets[j],
                            // `coord.point` without re-validating the basis per vertex.
                            basis.transform_vector3(p) * coord.meters_per_unit,
                            basis.transform_vector3(n),
                        )?;
                    }
                }
                let mat = primitive.material();
                let pbr = mat.pbr_metallic_roughness();
                let texture = pbr
                    .base_color_texture()
                    .map(|t| {
                        ensure!(t.tex_coord() == 0, "base color requires TEXCOORD_0");
                        Ok(t.texture().source().index())
                    })
                    .transpose()?;
                let alpha = match mat.alpha_mode() {
                    ::gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                    ::gltf::material::AlphaMode::Mask => {
                        AlphaMode::Mask(mat.alpha_cutoff().unwrap_or(0.5))
                    }
                    ::gltf::material::AlphaMode::Blend => AlphaMode::Blend,
                };
                materials.push(Material {
                    id: format!("{}/{}/{}", node.index(), mesh.index(), primitive.index()),
                    name: mat.name().map(str::to_owned),
                    first_index: start,
                    index_count: indices.len() as u32 - start,
                    color: pbr.base_color_factor(),
                    texture,
                    double_sided: mat.double_sided(),
                    alpha,
                    groups: part_groups(&[mesh.name(), mat.name(), node.name()]),
                });
            }
        }
    }
    let morphs = morphs.finish(vertices.len())?;
    let mut clips = vec![];
    for animation in doc.animations() {
        let mut tracks = vec![];
        let mut morph_tracks = vec![];
        let mut duration = 0.0_f32;
        for channel in animation.channels() {
            let sampler = channel.sampler();
            let reader = channel.reader(|b| Some(buffers[b.index()].as_slice()));
            let times = reader
                .read_inputs()
                .context("animation input missing")?
                .collect::<Vec<_>>();
            ensure!(times.len() <= 1_000_000, "animation key budget exceeded");
            if let Some(t) = times.last() {
                duration = duration.max(*t)
            }
            let interpolation = match sampler.interpolation() {
                ::gltf::animation::Interpolation::Step => clip::Interpolation::Step,
                ::gltf::animation::Interpolation::Linear => clip::Interpolation::Linear,
                ::gltf::animation::Interpolation::CubicSpline => clip::Interpolation::CubicSpline,
            };
            let outputs = reader.read_outputs().context("animation output missing")?;
            if let ReadOutputs::MorphTargetWeights(weights) = outputs {
                let node = channel.target().node().index();
                let targets = morph_nodes.get(&node).with_context(|| {
                    format!("weights channel targets node {node}, whose mesh has no morph targets")
                })?;
                let values = weights.into_f32().collect::<Vec<_>>();
                let n = targets.len();
                let factor = if interpolation == clip::Interpolation::CubicSpline {
                    3
                } else {
                    1
                };
                ensure!(
                    values.len() == times.len() * n * factor,
                    "morph weight output count mismatch"
                );
                // glTF interleaves the targets per key (cubic: in-tangents, values,
                // out-tangents, each n wide); a track holds one target's keys.
                for (j, &target) in targets.iter().enumerate() {
                    let mut track = Vec::with_capacity(times.len() * factor);
                    for k in 0..times.len() {
                        for part in 0..factor {
                            track.push(values[(k * factor + part) * n + j]);
                        }
                    }
                    morph_tracks.push(clip::MorphTrack {
                        target: morphs.targets[target as usize].id.clone(),
                        interpolation,
                        times: times.clone(),
                        values: track,
                    });
                }
                continue;
            }
            let (property, values) = match outputs {
                ReadOutputs::Translations(v) => (
                    clip::Property::Translation,
                    v.map(|v| coord.point(Vec3::from_array(v)).map(|v| v.extend(0.)))
                        .collect::<Result<Vec<_>>>()?,
                ),
                ReadOutputs::Scales(v) => (
                    clip::Property::Scale,
                    v.map(|v| {
                        coordinates::scale_in_basis(basis, Vec3::from_array(v))
                            .map(|scale| scale.extend(0.))
                    })
                    .collect::<Result<Vec<_>>>()?,
                ),
                ReadOutputs::Rotations(v) => (
                    clip::Property::Rotation,
                    v.into_f32()
                        .map(|q| {
                            let xyz = basis.transform_vector3(Vec3::new(q[0], q[1], q[2]))
                                * basis.determinant().signum();
                            xyz.extend(q[3])
                        })
                        .collect(),
                ),
                ReadOutputs::MorphTargetWeights(_) => unreachable!("handled above"),
            };
            tracks.push(clip::Track {
                bone: skeleton.source.bones[channel.target().node().index()]
                    .id
                    .clone(),
                property,
                interpolation,
                times,
                values,
            });
        }
        clips.push(clip::Clip {
            id: animation
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("animation/{}", animation.index())),
            duration,
            tracks,
            morphs: morph_tracks,
        });
    }
    if doc.materials().any(|m| {
        m.normal_texture().is_some()
            || m.occlusion_texture().is_some()
            || m.emissive_texture().is_some()
            || m.pbr_metallic_roughness()
                .metallic_roughness_texture()
                .is_some()
    }) {
        diagnostics.push("Renderer uses base color/alpha and diffuse lighting; normal, metallic, occlusion and emissive maps are not rendered".into());
    }
    Ok(Asset {
        key: String::new(),
        skeleton,
        vertices,
        indices,
        materials,
        textures,
        clips,
        diagnostics,
        humanoid: None,
        morphs,
    })
}

/// Registers the morph targets of `mesh` as instanced by `node` (names from the
/// `extras.targetNames` convention, else the index; default weights from the node, else the
/// mesh) and returns their asset indices. A name an earlier node already took is prefixed with
/// this node's bone ID.
fn mesh_targets(
    node: &::gltf::Node,
    mesh: &::gltf::Mesh,
    node_id: &str,
    morphs: &mut morph::Builder,
) -> Result<Vec<u16>> {
    let count = mesh
        .primitives()
        .next()
        .map_or(0, |p| p.morph_targets().count());
    if count == 0 {
        return Ok(vec![]);
    }
    let at = mesh
        .name()
        .map_or_else(|| format!("mesh/{}", mesh.index()), str::to_owned);
    if count > morph::MAX_TARGETS_PER_MESH {
        return Err(morph::MorphBudget {
            what: "targets_per_mesh",
            limit: morph::MAX_TARGETS_PER_MESH,
            requested: count,
            at,
        }
        .into());
    }
    let names: Vec<String> = mesh
        .extras()
        .as_ref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.get()).ok())
        .and_then(|v| {
            v["targetNames"].as_array().map(|a| {
                a.iter()
                    .map(|n| n.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
        })
        .unwrap_or_default();
    let defaults = node.weights().or(mesh.weights()).unwrap_or(&[]);
    let mut out = Vec::with_capacity(count);
    for j in 0..count {
        let name = names
            .get(j)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| j.to_string());
        let id = if morphs.has(&name) {
            format!("{node_id}/{name}")
        } else {
            name
        };
        let default = defaults.get(j).copied().unwrap_or(0.0);
        out.push(morphs.add_target(id, default, &at)?);
    }
    Ok(out)
}

/// The non-empty, distinct names that also select a primitive.
fn part_groups(names: &[Option<&str>]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for n in names.iter().flatten() {
        if !n.is_empty() && !out.iter().any(|g| g == n) {
            out.push((*n).to_owned());
        }
    }
    out
}
