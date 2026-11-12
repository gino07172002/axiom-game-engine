//! Morph targets (glTF import, weight tracks, deformation, budgets) and primitive groups, on an
//! engine-authored GLB written by this file. `content/character-lab/morph-head.glb` is that
//! file's output (regenerate with `AXIOM_WRITE_FIXTURES=1 cargo test -p axiom-animation --test
//! morph the_checked_in`; the other tests read the file).
use axiom_animation::{
    import::{load, ImportOptions},
    morph::{MorphBudget, MAX_TARGETS_PER_MESH},
    skin::{deform_with, Skinning},
};
use glam::Vec3;
use serde_json::{json, Value};

const CENTER: f32 = 0.5;
const RADIUS: f32 = 0.5;

/// Little-endian GLB writer: one buffer, one view per accessor.
#[derive(Default)]
struct Glb {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Glb {
    fn push(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut view = json!({"buffer":0,"byteOffset":self.bin.len(),"byteLength":bytes.len()});
        if let Some(t) = target {
            view["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        self.views.len() - 1
    }
    fn floats(&mut self, data: &[f32], kind: &str, width: usize, bounds: bool) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.push(&bytes, (kind != "SCALAR").then_some(34962));
        let mut a =
            json!({"bufferView":view,"componentType":5126,"count":data.len() / width,"type":kind});
        if bounds {
            let (mut lo, mut hi) = (vec![f32::MAX; width], vec![f32::MIN; width]);
            for chunk in data.chunks(width) {
                for k in 0..width {
                    lo[k] = lo[k].min(chunk[k]);
                    hi[k] = hi[k].max(chunk[k]);
                }
            }
            a["min"] = json!(lo);
            a["max"] = json!(hi);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }
    fn indices(&mut self, data: &[u32]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|i| i.to_le_bytes()).collect();
        let view = self.push(&bytes, Some(34963));
        self.accessors.push(
            json!({"bufferView":view,"componentType":5125,"count":data.len(),"type":"SCALAR"}),
        );
        self.accessors.len() - 1
    }
    fn finish(self, mut doc: Value) -> Vec<u8> {
        let mut bin = self.bin;
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        doc["asset"] = json!({"version":"2.0","generator":"axiom morph fixture"});
        doc["buffers"] = json!([{"byteLength":bin.len()}]);
        doc["bufferViews"] = Value::Array(self.views);
        doc["accessors"] = Value::Array(self.accessors);
        let mut text = serde_json::to_vec(&doc).unwrap();
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let total = 12 + 8 + text.len() + 8 + bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&text);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }
}

/// Positions, normals and triangle indices.
type Shape = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>);

fn sphere() -> Shape {
    let (rings, segments) = (8, 16);
    let (mut p, mut n, mut idx) = (vec![], vec![], vec![]);
    for i in 0..=rings {
        let theta = std::f32::consts::PI * i as f32 / rings as f32;
        for j in 0..=segments {
            let phi = std::f32::consts::TAU * j as f32 / segments as f32;
            let d = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            p.push((d * RADIUS + Vec3::Y * CENTER).to_array());
            n.push(d.to_array());
        }
    }
    let row = segments + 1;
    for i in 0..rings {
        for j in 0..segments {
            let a = i * row + j;
            let b = a + row;
            idx.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    (p, n, idx)
}

fn boxes(list: &[([f32; 3], [f32; 3])]) -> Shape {
    let (mut p, mut n, mut idx) = (vec![], vec![], vec![]);
    for (center, half) in list {
        let (c, h) = (Vec3::from(*center), Vec3::from(*half));
        for axis in 0..3 {
            for sign in [-1.0f32, 1.0] {
                let mut normal = Vec3::ZERO;
                normal[axis] = sign;
                let u = normal.any_orthonormal_vector();
                let v = normal.cross(u);
                let base = p.len() as u32;
                for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    p.push((c + (normal + u * a + v * b) * h).to_array());
                    n.push(normal.to_array());
                }
                idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
    }
    (p, n, idx)
}

/// `stretch` pulls the upper half up; `puff` widens around the vertical axis.
fn deltas(p: &[[f32; 3]], target: usize) -> Vec<f32> {
    p.iter()
        .flat_map(|v| {
            let v = Vec3::from(*v);
            match target {
                0 => [0.0, 0.35 * ((v.y - CENTER) / RADIUS).max(0.0), 0.0],
                1 => [0.4 * v.x, 0.0, 0.4 * v.z],
                _ => [0.0, 0.0, 0.01 * target as f32],
            }
        })
        .collect()
}

/// The fixture: node `head` with mesh `face` (primitives `skin` and `eyes`, `targets` morph
/// targets) and node `hat` with mesh `hat` (material `felt`); clips `pulse` (linear weights)
/// and `pulse_cubic` (cubic spline weights).
fn fixture(targets: usize) -> Vec<u8> {
    let mut g = Glb::default();
    let primitive = |g: &mut Glb, shape: &Shape, material: usize, morph: bool| {
        let flat = |v: &[[f32; 3]]| v.iter().flatten().copied().collect::<Vec<f32>>();
        let position = g.floats(&flat(&shape.0), "VEC3", 3, true);
        let normal = g.floats(&flat(&shape.1), "VEC3", 3, false);
        let indices = g.indices(&shape.2);
        let mut out = json!({"attributes":{"POSITION":position,"NORMAL":normal},"indices":indices,"material":material});
        if morph {
            out["targets"] = Value::Array(
                (0..targets)
                    .map(|t| json!({"POSITION": g.floats(&deltas(&shape.0, t), "VEC3", 3, true)}))
                    .collect(),
            );
        }
        out
    };
    let eyes = boxes(&[
        ([-0.16, 0.62, 0.44], [0.05, 0.06, 0.04]),
        ([0.16, 0.62, 0.44], [0.05, 0.06, 0.04]),
    ]);
    let hat = boxes(&[
        ([0.0, 1.06, 0.0], [0.36, 0.03, 0.36]),
        ([0.0, 1.22, 0.0], [0.22, 0.16, 0.22]),
    ]);
    let skin = primitive(&mut g, &sphere(), 0, true);
    let eyes = primitive(&mut g, &eyes, 1, true);
    let hat = primitive(&mut g, &hat, 2, false);
    let times = g.floats(&[0.0, 1.0, 2.0], "SCALAR", 1, true);
    // Per key: stretch, puff (and zeros for further targets).
    let key = |a: f32, b: f32| {
        let mut k = vec![0.0; targets];
        k[0] = a;
        if targets > 1 {
            k[1] = b;
        }
        k
    };
    let linear: Vec<f32> = [key(0.0, 0.0), key(1.0, 0.5), key(0.0, 0.0)].concat();
    let linear = g.floats(&linear, "SCALAR", 1, false);
    let zero = vec![0.0; targets];
    let cubic: Vec<f32> = [
        zero.clone(),
        key(0.0, 0.0),
        zero.clone(),
        zero.clone(),
        key(1.0, 0.5),
        zero.clone(),
        zero.clone(),
        key(0.0, 0.0),
        zero,
    ]
    .concat();
    let cubic = g.floats(&cubic, "SCALAR", 1, false);
    let names: Vec<String> = (0..targets)
        .map(|t| match t {
            0 => "stretch".to_owned(),
            1 => "puff".to_owned(),
            t => format!("extra{t}"),
        })
        .collect();
    let doc = json!({
        "scene": 0,
        "scenes": [{"nodes":[0,1]}],
        "nodes": [
            {"name":"head","mesh":0},
            {"name":"hat","mesh":1}
        ],
        "meshes": [
            {"name":"face","primitives":[skin, eyes],"weights":vec![0.0; targets],"extras":{"targetNames":names}},
            {"name":"hat","primitives":[hat]}
        ],
        "materials": [
            {"name":"skin","pbrMetallicRoughness":{"baseColorFactor":[0.96,0.78,0.62,1.0],"metallicFactor":0.0}},
            {"name":"eyes","pbrMetallicRoughness":{"baseColorFactor":[0.08,0.08,0.12,1.0],"metallicFactor":0.0}},
            {"name":"felt","pbrMetallicRoughness":{"baseColorFactor":[0.75,0.12,0.16,1.0],"metallicFactor":0.0}}
        ],
        "animations": [
            {"name":"pulse","samplers":[{"input":times,"output":linear,"interpolation":"LINEAR"}],
             "channels":[{"sampler":0,"target":{"node":0,"path":"weights"}}]},
            {"name":"pulse_cubic","samplers":[{"input":times,"output":cubic,"interpolation":"CUBICSPLINE"}],
             "channels":[{"sampler":0,"target":{"node":0,"path":"weights"}}]}
        ]
    });
    g.finish(doc)
}

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../content/character-lab/morph-head.glb")
}

fn scratch(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("axiom-morph-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn the_checked_in_fixture_is_this_generator_output() {
    let bytes = fixture(2);
    if std::env::var_os("AXIOM_WRITE_FIXTURES").is_some() {
        std::fs::create_dir_all(fixture_path().parent().unwrap()).unwrap();
        std::fs::write(fixture_path(), &bytes).unwrap();
    }
    assert!(
        std::fs::read(fixture_path()).unwrap() == bytes,
        "content/character-lab/morph-head.glb differs from the generator"
    );
}

#[test]
fn morph_targets_import_with_names_defaults_tracks_and_groups() {
    let a = load(&fixture_path(), &ImportOptions::default()).unwrap();
    let ids: Vec<&str> = a.morphs.targets.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["stretch", "puff"]);
    assert_eq!(a.summary()["morph_targets"], json!(["stretch", "puff"]));
    // Primitives answer to their material, mesh and node names.
    assert_eq!(a.materials.len(), 3);
    assert_eq!(a.materials[2].groups, ["hat", "felt"]);
    assert_eq!(a.materials[1].groups, ["face", "eyes", "head"]);
    assert_eq!(a.parts("hat"), [0b100, 0, 0, 0]);
    assert_eq!(a.parts("face"), [0b011, 0, 0, 0]);
    assert_eq!(a.parts("nope"), [0; 4]);
    // Weight channels become one track per target, linear and cubic alike.
    for clip in ["pulse", "pulse_cubic"] {
        let c = a.clips.iter().find(|c| c.id == clip).unwrap();
        assert!(c.tracks.is_empty() && c.morphs.len() == 2, "{clip}");
        let compiled = c.compile_for(&a.skeleton, &a.morphs).unwrap();
        let mut w = [0.0; 2];
        compiled.morph_weights_into(1.0, false, &mut w);
        assert!(
            (w[0] - 1.0).abs() < 1e-5 && (w[1] - 0.5).abs() < 1e-5,
            "{clip} {w:?}"
        );
        compiled.morph_weights_into(0.5, false, &mut w);
        assert!(w[0] > 0.2 && w[0] < 0.8, "{clip} {w:?}");
    }
    // Without the asset's targets a morph track is refused, not dropped.
    let e = a.clips[0].compile(&a.skeleton).unwrap_err();
    assert!(format!("{e:#}").contains("morph target"), "{e:#}");
}

#[test]
fn weights_deform_vertices_and_zero_weights_change_nothing() {
    let a = load(&fixture_path(), &ImportOptions::default()).unwrap();
    let pose = a.skeleton.rest_pose();
    let rest = deform_with(&a, &pose, Skinning::Auto, &[]).unwrap();
    let zero = deform_with(&a, &pose, Skinning::Auto, &[0.0, 0.0]).unwrap();
    let puffed = deform_with(&a, &pose, Skinning::Auto, &[0.0, 1.0]).unwrap();
    // The equator vertex at +X (ring 4, segment 0) moves out by 40 %.
    let i = 4 * 17;
    assert_eq!(rest[i].position, zero[i].position);
    assert!((rest[i].position.x - RADIUS).abs() < 1e-5);
    assert!(
        (puffed[i].position.x - RADIUS * 1.4).abs() < 1e-4,
        "{:?}",
        puffed[i]
    );
    // The hat (no targets) does not move.
    let hat = a.materials[2].first_index as usize;
    let v = a.indices[hat] as usize;
    assert_eq!(rest[v].position, puffed[v].position);
    assert!(deform_with(&a, &pose, Skinning::Auto, &[1.0]).is_err());
}

#[test]
fn too_many_targets_per_mesh_is_a_typed_budget_error() {
    let path = scratch("too-many.glb", &fixture(MAX_TARGETS_PER_MESH + 1));
    let e = load(&path, &ImportOptions::default()).unwrap_err();
    let budget = e.downcast_ref::<MorphBudget>().expect("typed budget error");
    assert_eq!(budget.what, "targets_per_mesh");
    assert_eq!(budget.limit, MAX_TARGETS_PER_MESH);
    assert_eq!(budget.requested, MAX_TARGETS_PER_MESH + 1);
    assert_eq!(budget.at, "face");
    // At the limit it loads.
    let ok = scratch("at-limit.glb", &fixture(MAX_TARGETS_PER_MESH));
    assert_eq!(
        load(&ok, &ImportOptions::default())
            .unwrap()
            .morphs
            .targets
            .len(),
        MAX_TARGETS_PER_MESH
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
