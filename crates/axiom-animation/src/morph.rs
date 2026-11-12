//! Morph targets (blend shapes): sparse position / normal displacements per target.
//!
//! Deltas are stored vertex-major (`offsets` + `deltas`, like CSR), so skinning adds a
//! vertex's weighted deltas while it deforms that vertex: no extra per-frame buffer, and a
//! frame whose weights are all zero costs nothing. Importers fill a [`Builder`]; budgets are
//! checked there and reported as a typed [`MorphBudget`] error.
use crate::*;

/// Most targets one glTF mesh may declare.
pub const MAX_TARGETS_PER_MESH: usize = 64;
/// Most targets one asset may have (all meshes).
pub const MAX_TARGETS: usize = 128;
/// Most non-zero vertex deltas one asset may have (28 bytes each: about 28 MB).
pub const MAX_DELTAS: usize = 1_000_000;

/// A morph budget was exceeded at import (downcast the `anyhow::Error` to inspect it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorphBudget {
    /// `targets_per_mesh`, `targets` or `deltas`.
    pub what: &'static str,
    pub limit: usize,
    pub requested: usize,
    /// Mesh or target the import was reading.
    pub at: String,
}

impl std::fmt::Display for MorphBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "morph budget exceeded at {}: {} {} > limit {}",
            self.at, self.what, self.requested, self.limit
        )
    }
}

impl std::error::Error for MorphBudget {}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// Stable name (glTF `extras.targetNames`, else the target index), unique in the asset.
    pub id: String,
    /// Weight used when no clip track or command sets it (glTF node / mesh `weights`).
    pub default_weight: f32,
}

/// One vertex displacement of one target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delta {
    pub target: u16,
    pub position: Vec3,
    pub normal: Vec3,
}

#[derive(Clone, Debug, Default)]
pub struct Morphs {
    pub targets: Vec<Target>,
    /// Start of each vertex's deltas in `deltas` (vertex count + 1 entries; empty when the
    /// asset has no targets).
    pub offsets: Vec<u32>,
    pub deltas: Vec<Delta>,
}

impl Morphs {
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub fn index(&self, id: &str) -> Option<usize> {
        self.targets.iter().position(|t| t.id == id)
    }

    /// Default weights into a reused buffer.
    pub fn defaults_into(&self, out: &mut Vec<f32>) {
        out.clear();
        out.extend(self.targets.iter().map(|t| t.default_weight));
    }

    pub fn validate(&self, vertices: usize) -> Result<()> {
        if self.targets.is_empty() {
            ensure!(
                self.offsets.is_empty() && self.deltas.is_empty(),
                "morph deltas without targets"
            );
            return Ok(());
        }
        ensure!(
            self.targets.len() <= MAX_TARGETS && self.deltas.len() <= MAX_DELTAS,
            "morph budget exceeded"
        );
        let mut ids = std::collections::BTreeSet::new();
        for t in &self.targets {
            ensure!(
                !t.id.is_empty() && t.id.len() <= 256 && ids.insert(&t.id),
                "duplicate/invalid morph target ID: {}",
                t.id
            );
            ensure!(
                t.default_weight.is_finite(),
                "non-finite morph default weight"
            );
        }
        ensure!(
            self.offsets.len() == vertices + 1
                && self.offsets.first() == Some(&0)
                && self.offsets.windows(2).all(|w| w[0] <= w[1])
                && self.offsets.last().copied() == Some(self.deltas.len() as u32),
            "morph offsets do not cover the vertices"
        );
        ensure!(
            self.deltas
                .iter()
                .all(|d| (d.target as usize) < self.targets.len()
                    && d.position.is_finite()
                    && d.normal.is_finite()),
            "invalid morph delta"
        );
        Ok(())
    }

    /// Vertex `i`'s position and normal with `weights` (one per target) applied.
    #[inline]
    pub fn apply(&self, i: usize, weights: &[f32], position: Vec3, normal: Vec3) -> (Vec3, Vec3) {
        let (mut p, mut n) = (position, normal);
        let range = self.offsets[i] as usize..self.offsets[i + 1] as usize;
        for d in &self.deltas[range] {
            let w = weights[d.target as usize];
            if w != 0.0 {
                p += d.position * w;
                n += d.normal * w;
            }
        }
        (p, n)
    }
}

/// Collects targets and their deltas (any order), then packs them vertex-major.
#[derive(Default)]
pub struct Builder {
    targets: Vec<Target>,
    entries: Vec<(u32, Delta)>,
}

impl Builder {
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    pub fn has(&self, id: &str) -> bool {
        self.targets.iter().any(|t| t.id == id)
    }

    /// Adds a target; `at` names the mesh for budget errors.
    pub fn add_target(&mut self, id: String, default_weight: f32, at: &str) -> Result<u16> {
        if self.targets.len() >= MAX_TARGETS {
            return Err(MorphBudget {
                what: "targets",
                limit: MAX_TARGETS,
                requested: self.targets.len() + 1,
                at: at.to_owned(),
            }
            .into());
        }
        ensure!(
            default_weight.is_finite(),
            "non-finite morph default weight at {at}"
        );
        ensure!(!self.has(&id), "duplicate morph target ID {id}");
        self.targets.push(Target { id, default_weight });
        Ok((self.targets.len() - 1) as u16)
    }

    /// Adds one displacement (an all-zero one is skipped: deltas are sparse).
    pub fn push(&mut self, vertex: u32, target: u16, position: Vec3, normal: Vec3) -> Result<()> {
        ensure!(
            position.is_finite() && normal.is_finite(),
            "non-finite morph displacement"
        );
        if position == Vec3::ZERO && normal == Vec3::ZERO {
            return Ok(());
        }
        if self.entries.len() >= MAX_DELTAS {
            return Err(MorphBudget {
                what: "deltas",
                limit: MAX_DELTAS,
                requested: self.entries.len() + 1,
                at: self.targets[target as usize].id.clone(),
            }
            .into());
        }
        self.entries.push((
            vertex,
            Delta {
                target,
                position,
                normal,
            },
        ));
        Ok(())
    }

    /// Packs the deltas by vertex (stable: a vertex keeps its targets in insertion order).
    pub fn finish(self, vertices: usize) -> Result<Morphs> {
        if self.targets.is_empty() {
            return Ok(Morphs::default());
        }
        let mut offsets = vec![0u32; vertices + 1];
        for (v, _) in &self.entries {
            ensure!((*v as usize) < vertices, "morph delta vertex out of range");
            offsets[*v as usize + 1] += 1;
        }
        for i in 0..vertices {
            offsets[i + 1] += offsets[i];
        }
        let mut cursor = offsets.clone();
        let mut deltas = vec![
            Delta {
                target: 0,
                position: Vec3::ZERO,
                normal: Vec3::ZERO,
            };
            self.entries.len()
        ];
        for (v, d) in self.entries {
            let slot = &mut cursor[v as usize];
            deltas[*slot as usize] = d;
            *slot += 1;
        }
        Ok(Morphs {
            targets: self.targets,
            offsets,
            deltas,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_pack_by_vertex_and_apply_weighted() {
        let mut b = Builder::default();
        let smile = b.add_target("smile".into(), 0.0, "face").unwrap();
        let puff = b.add_target("puff".into(), 0.25, "face").unwrap();
        b.push(2, smile, Vec3::Y, Vec3::ZERO).unwrap();
        b.push(0, puff, Vec3::X, Vec3::Z).unwrap();
        b.push(2, puff, Vec3::X, Vec3::ZERO).unwrap();
        // All-zero deltas are not stored.
        b.push(1, smile, Vec3::ZERO, Vec3::ZERO).unwrap();
        let m = b.finish(3).unwrap();
        m.validate(3).unwrap();
        assert_eq!(m.offsets, [0, 1, 1, 3]);
        let (p, _) = m.apply(2, &[0.5, 1.0], Vec3::ZERO, Vec3::Y);
        assert_eq!(p, Vec3::new(1.0, 0.5, 0.0));
        let mut d = vec![];
        m.defaults_into(&mut d);
        assert_eq!(d, [0.0, 0.25]);
        // Zero weights leave the vertex untouched.
        assert_eq!(
            m.apply(0, &[0.0, 0.0], Vec3::ONE, Vec3::Y),
            (Vec3::ONE, Vec3::Y)
        );
    }

    #[test]
    fn budgets_fail_with_a_typed_error() {
        let mut b = Builder::default();
        for i in 0..MAX_TARGETS {
            b.add_target(format!("t{i}"), 0.0, "mesh").unwrap();
        }
        let e = b.add_target("one_more".into(), 0.0, "mesh").unwrap_err();
        let budget = e.downcast_ref::<MorphBudget>().expect("typed");
        assert_eq!(
            (budget.what, budget.limit, budget.requested),
            ("targets", MAX_TARGETS, MAX_TARGETS + 1)
        );
    }
}
