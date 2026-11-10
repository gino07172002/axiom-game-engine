//! Scene-view ghost boxes: where the objects a pending proposal or a past revision changes
//! were and where they end up. Preview only (nothing is written). The boxes are computed
//! once per (source, revision) on a background job; drawing them is a few line segments.
use super::{Action, Editor};
use crate::jobs::{Job, Outcome};
use axiom_core::{RenderMesh, Store, Transform, World};
use glam::{Mat4, Quat, Vec3};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

/// Boxes drawn at most; further changed objects are only counted.
pub(crate) const MAX_GHOSTS: usize = 64;
const GHOST_TIMEOUT: Duration = Duration::from_secs(20);

/// What the ghosts show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GhostSource {
    /// A pending proposal applied to the current revision (what approving it would do).
    Proposal(i64),
    /// A revision against its predecessor.
    Revision(i64),
}

impl GhostSource {
    /// `proposal:N`, `rev:N`, or `off` (None).
    pub fn parse(arg: &str) -> anyhow::Result<Option<Self>> {
        let number = |s: &str| {
            s.trim_start_matches('#')
                .parse::<i64>()
                .map_err(|_| anyhow::anyhow!("expected a number in {arg:?}"))
        };
        match arg.trim() {
            "off" | "none" => Ok(None),
            a if a.starts_with("proposal:") => Ok(Some(Self::Proposal(number(&a[9..])?))),
            a if a.starts_with("rev:") => Ok(Some(Self::Revision(number(&a[4..])?))),
            _ => anyhow::bail!("ghost.show takes proposal:N, rev:N or off (got {arg:?})"),
        }
    }
    pub fn name(self) -> String {
        match self {
            Self::Proposal(id) => format!("proposal:{id}"),
            Self::Revision(rev) => format!("rev:{rev}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GhostKind {
    Added,
    Removed,
    /// Bounds moved (position, rotation, scale, parent or mesh).
    Moved,
    /// Same bounds, other fields changed (material, behavior…).
    Changed,
}

/// One changed object: world-space unit-box matrices of its bounds before and after.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GhostBox {
    pub id: String,
    pub kind: GhostKind,
    pub before: Option<Mat4>,
    pub after: Option<Mat4>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ghosts {
    pub boxes: Vec<GhostBox>,
    /// Changed objects with a transform (boxes beyond `MAX_GHOSTS` are counted only).
    pub total: usize,
}

/// Bounds of an object: its `render.mesh` parts, else a 0.5 m marker at the pivot.
fn bounds(matrix: Mat4, mesh: Option<&RenderMesh>) -> Mat4 {
    let parts = mesh.map_or(&[][..], |m| &m.parts[..]);
    if parts.is_empty() {
        let (_, _, pivot) = matrix.to_scale_rotation_translation();
        return Mat4::from_scale_rotation_translation(Vec3::splat(0.5), Quat::IDENTITY, pivot);
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for part in parts {
        let m = matrix * part.transform.matrix();
        for j in 0..8usize {
            let corner = Vec3::new(
                (j & 1) as f32 - 0.5,
                ((j >> 1) & 1) as f32 - 0.5,
                ((j >> 2) & 1) as f32 - 0.5,
            );
            let p = m.transform_point3(corner);
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    Mat4::from_scale_rotation_translation(
        (hi - lo).max(Vec3::splat(0.05)),
        Quat::IDENTITY,
        (lo + hi) * 0.5,
    )
}

/// Changed objects between two worlds, with their world-space bounds on each side.
pub(crate) fn ghost_boxes(
    before: &World,
    after: &World,
    registry: &axiom_core::Registry,
) -> anyhow::Result<Ghosts> {
    let (a, b) = (before.compile(registry)?, after.compile(registry)?);
    let matrices = |c: &axiom_core::CompiledWorld| {
        crate::world::compose_matrices(c, c.len(), |i| {
            c.entities[i]
                .transform
                .as_ref()
                .map_or(Mat4::IDENTITY, Transform::matrix)
        })
    };
    let (ma, mb) = (matrices(&a), matrices(&b));
    let mut out = Ghosts::default();
    let mut push = |id: &str, kind, before, after| {
        out.total += 1;
        if out.boxes.len() < MAX_GHOSTS {
            out.boxes.push(GhostBox {
                id: id.to_owned(),
                kind,
                before,
                after,
            });
        }
    };
    for (j, e) in b.entities.iter().enumerate() {
        let now = e.transform.as_ref().map(|_| bounds(mb[j], e.mesh.as_ref()));
        match a.indices.get(&e.id) {
            None => {
                if now.is_some() {
                    push(&e.id, GhostKind::Added, None, now);
                }
            }
            Some(&i) => {
                let old = &a.entities[i];
                if old.components == e.components && ma[i] == mb[j] {
                    continue;
                }
                let was = old
                    .transform
                    .as_ref()
                    .map(|_| bounds(ma[i], old.mesh.as_ref()));
                match (was, now) {
                    (None, None) => {}
                    (w, n) if w == n => push(&e.id, GhostKind::Changed, None, n),
                    (w, n) => push(&e.id, GhostKind::Moved, w, n),
                }
            }
        }
    }
    for (i, e) in a.entities.iter().enumerate() {
        if !b.indices.contains_key(&e.id) && e.transform.is_some() {
            let was = Some(bounds(ma[i], e.mesh.as_ref()));
            push(&e.id, GhostKind::Removed, was, None);
        }
    }
    Ok(out)
}

/// (source, revision) a result belongs to: a proposal is applied to the current revision; a
/// past revision's ghosts never change (revision 0).
pub(crate) type GhostKey = (GhostSource, i64);

/// Editor state of the ghost overlay.
#[derive(Default)]
pub(crate) struct GhostState {
    /// What to show (None = nothing).
    pub source: Option<GhostSource>,
    /// The last result: its key and the boxes, or why there are none.
    pub shown: Option<(GhostKey, Result<Ghosts, String>)>,
    pub job: Option<(GhostKey, Job<anyhow::Result<Ghosts>>)>,
    /// The key last asked of the host (asked again only when it changes).
    pub requested: Option<GhostKey>,
    /// Boxes drawn in the last Scene-view frame.
    pub drawn: usize,
    /// Scene-view caption of the shown result (formatted once per result).
    pub label: String,
    /// Results received (tests: once per key, never per frame).
    pub builds: usize,
    /// `ghost.show off`: new proposals do not turn the ghosts back on by themselves.
    pub off: bool,
}

impl Editor {
    /// The cache key of `source`: a proposal applies to the current revision; a past
    /// revision's ghosts never change.
    fn ghost_key(&self, source: GhostSource) -> GhostKey {
        match source {
            GhostSource::Proposal(_) => (source, self.rev),
            GhostSource::Revision(_) => (source, 0),
        }
    }

    /// Shows the ghosts of `source` (None hides them).
    pub(crate) fn set_ghost_source(&mut self, source: Option<GhostSource>) {
        self.ghost.source = source;
    }

    /// The load the host must run now, if the shown source needs (new) boxes.
    pub(crate) fn ghost_request(&mut self) -> Option<Action> {
        let source = self.ghost.source.filter(|_| self.prefs.ghosts)?;
        let key = self.ghost_key(source);
        if self.ghost.requested == Some(key) {
            return None;
        }
        self.ghost.requested = Some(key);
        Some(Action::LoadGhosts { source })
    }

    /// `ghost.show proposal:N | rev:N | off`; without an argument the History tab opens to
    /// pick a revision (proposals: the Proposals tab).
    pub(crate) fn ghost_command(&mut self, arg: Option<&str>) -> anyhow::Result<Vec<Action>> {
        let Some(arg) = arg else {
            self.show_dock(super::Dock::History);
            return Ok(vec![]);
        };
        let source = GhostSource::parse(arg)?;
        // A proposal the Proposals list has not polled yet still loads (an unknown or resolved
        // one reports its error in `observe editor.ghosts`).
        if let Some(GhostSource::Proposal(id)) = source {
            if self.proposals.iter().any(|p| p["id"] == id) {
                self.proposal_focus = Some(id);
            }
        }
        self.set_ghost_source(source);
        self.ghost.off = source.is_none();
        if source.is_some() {
            self.prefs.ghosts = true;
        }
        Ok(self.ghost_request().into_iter().collect())
    }

    /// The host could not start the load of `source` (e.g. the proposal was resolved).
    pub(crate) fn ghost_failed(&mut self, source: GhostSource, error: &anyhow::Error) {
        let key = self.ghost_key(source);
        self.ghost.shown = Some((key, Err(format!("{error:#}"))));
    }

    /// Installs a load started by the host (`load`). A previous one is dropped.
    pub(crate) fn start_ghost_job(&mut self, key: GhostKey, job: Job<anyhow::Result<Ghosts>>) {
        if let Some((_, old)) = self.ghost.job.replace((key, job)) {
            old.detach();
        }
    }

    /// Once per frame (from `poll_jobs`).
    pub(crate) fn poll_ghosts(&mut self) {
        let Some((key, job)) = &mut self.ghost.job else {
            return;
        };
        let Some(outcome) = job.poll() else {
            return;
        };
        let key = *key;
        let ms = job.elapsed().as_secs_f64() * 1000.0;
        self.ghost.job = None;
        let result = match outcome {
            Outcome::Done {
                value: Ok(ghosts),
                stop: None,
            } => Ok(ghosts),
            Outcome::Done {
                value: Err(error), ..
            } => Err(format!("{error:#}")),
            Outcome::Done { stop: Some(s), .. } | Outcome::Abandoned { stop: s } => {
                Err(format!("ghost job {}", s.name()))
            }
        };
        self.last_job = Some(
            json!({"kind":"ghosts","label":key.0.name(),"state":if result.is_ok() {"finished"} else {"failed"},"elapsed_ms":(ms * 100.0).round() / 100.0}),
        );
        self.ghost.builds += 1;
        self.ghost.label = match (&result, key.0) {
            (Ok(g), source) => {
                let what = match source {
                    GhostSource::Proposal(id) => format!("proposal #{id}"),
                    GhostSource::Revision(rev) => format!("revision {rev}"),
                };
                let more = if g.total > g.boxes.len() {
                    format!(" (first {} drawn)", g.boxes.len())
                } else {
                    String::new()
                };
                format!("Ghosts · {what} · {} object(s) changed{more}", g.total)
            }
            (Err(_), _) => String::new(),
        };
        self.ghost.shown = Some((key, result));
    }

    /// The boxes to draw now: the shown source's current result (None while loading or off).
    pub(crate) fn ghosts_now(&self) -> Option<&Ghosts> {
        let source = self.ghost.source.filter(|_| self.prefs.ghosts)?;
        let key = self.ghost_key(source);
        match &self.ghost.shown {
            Some((k, Ok(ghosts))) if *k == key => Some(ghosts),
            _ => None,
        }
    }

    pub(crate) fn ghost_observation(&self) -> Value {
        let source = self.ghost.source.map(GhostSource::name);
        let key = self.ghost.source.map(|s| self.ghost_key(s));
        let (state, boxes, total, error) = match (&self.ghost.shown, key) {
            _ if self.ghost.source.is_none() => ("off", 0, 0, None),
            _ if !self.prefs.ghosts => ("hidden", 0, 0, None),
            (Some((k, Ok(g))), Some(key)) if *k == key => ("ready", g.boxes.len(), g.total, None),
            (Some((k, Err(e))), Some(key)) if *k == key => ("error", 0, 0, Some(e.clone())),
            _ => ("loading", 0, 0, None),
        };
        json!({"source":source,"state":state,"boxes":boxes,"total":total,"drawn":self.ghost.drawn,"error":error})
    }
}

/// Host side of `Action::LoadGhosts`: reads what the job needs from the store (one row or
/// two) and starts it. Decoding, applying and compiling happen on the worker.
pub(crate) fn load(editor: &mut Editor, store: &Store, source: GhostSource) -> anyhow::Result<()> {
    let registry = editor.registry.clone();
    let key = editor.ghost_key(source);
    let job = match source {
        GhostSource::Proposal(id) => {
            let (_, edits, status) = store.proposal_raw(id)?;
            anyhow::ensure!(status == "pending", "proposal {id} is already {status}");
            let before: Arc<World> = editor.world.clone();
            Job::spawn("ghosts", source.name(), GHOST_TIMEOUT, move |_| {
                let edits: Vec<axiom_core::Edit> = serde_json::from_str(&edits)?;
                let after = axiom_core::ops::apply_edits(&before, &edits, &registry)?;
                ghost_boxes(&before, &after, &registry)
            })?
        }
        GhostSource::Revision(rev) => {
            anyhow::ensure!(
                rev > 1,
                "revision {rev} has no previous revision to compare"
            );
            let (a, b) = (store.revision_json(rev - 1)?, store.revision_json(rev)?);
            Job::spawn("ghosts", source.name(), GHOST_TIMEOUT, move |_| {
                let a = registry.decode(serde_json::from_str(&a)?)?;
                let b = registry.decode(serde_json::from_str(&b)?)?;
                ghost_boxes(&a, &b, &registry)
            })?
        }
    };
    editor.start_ghost_job(key, job);
    Ok(())
}
