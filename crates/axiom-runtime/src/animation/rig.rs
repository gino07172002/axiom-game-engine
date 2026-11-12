//! Runtime side of `animation.spring`, `animation.look_at` and IK sockets: per-player state,
//! target resolution, checkpoints, hot-reload carry-over, metrics and debug draw.
//!
//! Ordering: at the start of each `animation.evaluate` step every target (look-at entity /
//! bone / camera, IK socket) is resolved for all players before any player advances, from
//! this step's transforms and the poses at the end of the previous step. A bone target
//! therefore lags exactly one fixed step (1/120 s) and the result does not depend on the
//! order players are evaluated in.
use super::{AnimationSystem, Controller};
use crate::{presentation::Instance, world::RuntimeState};
use anyhow::{bail, ensure, Context, Result};
use axiom_animation::{
    ik::Socket,
    rig::{
        CompiledLook, CompiledSpring, LookAtSet, LookState, LookTarget, RigScratch, SpringSet,
        SpringState, LOOK_AT, SPRING,
    },
    CompiledSkeleton, Pose,
};
use axiom_core::local::at;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

/// Where a look-at chain aims.
#[derive(Clone)]
enum Source {
    Camera,
    Socket(Socket),
    Point(Vec3),
}

#[derive(Clone)]
pub(super) struct Rig {
    spring: Option<(Arc<CompiledSpring>, SpringSet)>,
    spring_state: SpringState,
    spring_next: SpringState,
    look: Option<(Arc<CompiledLook>, LookAtSet)>,
    look_state: LookState,
    look_next: LookState,
    /// One per look-at chain (chain ID order) and the last resolved model-space point.
    look_targets: Vec<(Source, Option<Vec3>)>,
    /// IK constraints bound to a socket (constraint ID order) and the last resolved point.
    ik_sockets: Vec<(String, Socket, Option<Vec3>)>,
    /// The entity's world matrix this step (springs work in world space).
    model: Mat4,
    /// Fixed step being simulated; 0 while re-applying (commands, loads, reloads).
    pub(super) dt: f32,
    pending: bool,
    scratch: RigScratch,
}

impl Rig {
    fn empty() -> Self {
        Self {
            spring: None,
            spring_state: SpringState::default(),
            spring_next: SpringState::default(),
            look: None,
            look_state: LookState::default(),
            look_next: LookState::default(),
            look_targets: vec![],
            ik_sockets: vec![],
            model: Mat4::IDENTITY,
            dt: 0.0,
            pending: false,
            scratch: RigScratch::default(),
        }
    }
    fn used(&self) -> bool {
        self.spring.is_some() || self.look.is_some() || !self.ik_sockets.is_empty()
    }
    /// The resolved model-space target of IK constraint `id`, if it is socket-bound and has
    /// resolved at least once.
    pub(super) fn ik_target(&self, id: &str) -> Option<Vec3> {
        self.ik_sockets
            .iter()
            .find(|(k, _, _)| k == id)
            .and_then(|(_, _, t)| *t)
    }
    /// Look-at then springs on `pose`; the next state is kept until [`Self::commit`].
    pub(super) fn apply(&mut self, s: &CompiledSkeleton, pose: &mut Pose) -> Result<()> {
        let Self {
            spring,
            spring_state,
            spring_next,
            look,
            look_state,
            look_next,
            look_targets,
            model,
            dt,
            scratch,
            ..
        } = self;
        if let Some((look, _)) = look {
            // Targets are read through a fixed-size view (no allocation): at most 4 chains.
            let mut targets = [None; axiom_animation::rig::MAX_LOOK_CHAINS];
            for (slot, (_, t)) in targets.iter_mut().zip(look_targets.iter()) {
                *slot = *t;
            }
            if let Err(e) = look.apply(s, pose, *dt, &targets, look_state, look_next, scratch) {
                // The caller adds the entity's component path.
                return Err(e.context(LOOK_AT));
            }
        }
        if let Some((spring, _)) = spring {
            if let Err(e) = spring.apply(s, pose, *model, *dt, spring_state, spring_next, scratch) {
                return Err(e.context(SPRING));
            }
        }
        self.pending = true;
        Ok(())
    }
    /// Keeps the state computed by the last successful [`Self::apply`].
    pub(super) fn commit(&mut self) {
        if std::mem::take(&mut self.pending) {
            if self.spring.is_some() {
                std::mem::swap(&mut self.spring_state, &mut self.spring_next);
            }
            if self.look.is_some() {
                std::mem::swap(&mut self.look_state, &mut self.look_next);
            }
        }
    }
}

fn source(target: &LookTarget) -> Source {
    match target {
        LookTarget::Camera => Source::Camera,
        LookTarget::Entity {
            entity,
            bone,
            offset,
        } => Source::Socket(Socket {
            entity: entity.clone(),
            bone: bone.clone(),
            offset: *offset,
        }),
        LookTarget::Point { position } => Source::Point(*position),
    }
}

/// World matrix of entity `slot` without allocating (same composition as `matrices()`).
pub(super) fn model_of(state: &RuntimeState, slot: usize) -> Mat4 {
    let local = |i: usize| {
        state.transforms[i]
            .as_ref()
            .map_or(Mat4::IDENTITY, axiom_core::Transform::matrix)
    };
    let mut m = local(slot);
    let mut parent = state.compiled.parents[slot];
    for _ in 0..state.compiled.parents.len() {
        let Some(p) = parent else { break };
        m = local(p) * m;
        parent = state.compiled.parents[p];
    }
    m
}

impl Controller {
    /// Builds the rig of a player from its entity's components (none = no rig).
    /// `model` is the entity's world matrix now: the springs start from the animated pose in
    /// world space.
    pub(super) fn attach_rig(
        &mut self,
        components: &BTreeMap<String, Value>,
        path: &str,
        model: Mat4,
    ) -> Result<()> {
        let mut rig = Rig::empty();
        rig.model = model;
        let skeleton = &self.asset.skeleton;
        if let Some(v) = components.get(SPRING) {
            let p = format!("{path}/components/{SPRING}");
            let set: SpringSet = at(&p, serde_json::from_value(v.clone()).map_err(Into::into))?;
            let compiled = at(&p, set.compile(skeleton))?;
            rig.spring = Some((Arc::new(compiled), set));
        }
        if let Some(v) = components.get(LOOK_AT) {
            let p = format!("{path}/components/{LOOK_AT}");
            let set: LookAtSet = at(&p, serde_json::from_value(v.clone()).map_err(Into::into))?;
            let compiled = at(&p, set.compile(skeleton))?;
            rig.look_targets = set
                .chains
                .values()
                .map(|c| (source(&c.target), None))
                .collect();
            rig.look = Some((Arc::new(compiled), set));
        }
        self.rig = Some(Box::new(rig));
        self.rebind_ik();
        Ok(())
    }
    /// Re-reads which IK constraints follow a socket (after `animation.ik` / loads); points
    /// already resolved for an unchanged socket are kept.
    pub(super) fn rebind_ik(&mut self) {
        let mut rig = self.rig.take().unwrap_or_else(|| Box::new(Rig::empty()));
        let old = std::mem::take(&mut rig.ik_sockets);
        for (id, c) in &self.ik {
            if let Some(socket) = &c.source.socket {
                let kept = old
                    .iter()
                    .find(|(k, s, _)| k == id && s == socket)
                    .and_then(|(_, _, t)| *t);
                rig.ik_sockets.push((id.clone(), socket.clone(), kept));
            }
        }
        self.rig = rig.used().then_some(rig);
    }
}

/// A socket as a world point: the entity's origin or bone (posed at the end of the previous
/// step) plus the local offset. `None` while that entity is disabled or not posed.
fn socket_point(
    players: &BTreeMap<String, Controller>,
    state: &RuntimeState,
    socket: &Socket,
) -> Option<Vec3> {
    let slot = *state.compiled.indices.get(&socket.entity)?;
    if !state.compiled.enabled[slot] {
        return None;
    }
    let mut m = model_of(state, slot);
    if let Some(bone) = &socket.bone {
        let p = players.get(&socket.entity)?;
        let i = *p.asset.skeleton.indices.get(bone)?;
        m *= *p.globals.get(i)?;
    }
    Some(m.transform_point3(socket.offset))
}

/// Position of the first enabled active camera (the one the game draws first).
fn camera_point(state: &RuntimeState) -> Option<Vec3> {
    let slot = state
        .compiled
        .entities
        .iter()
        .enumerate()
        .position(|(i, e)| {
            state.compiled.enabled[i]
                && e.camera.as_ref().is_some_and(|c| c.active)
                && state.contexts.as_ref().is_none_or(|c| c.camera(&e.id))
        })?;
    Some(model_of(state, slot).w_axis.truncate())
}

/// Checks that a socket names an existing entity and, with a bone, an animated one that has
/// that bone.
fn check_socket(
    players: &BTreeMap<String, Controller>,
    state: &RuntimeState,
    socket: &Socket,
) -> Result<()> {
    ensure!(
        state.compiled.indices.contains_key(&socket.entity),
        "socket entity {} does not exist",
        socket.entity
    );
    if let Some(bone) = &socket.bone {
        let p = players.get(&socket.entity).with_context(|| {
            format!(
                "socket entity {} has no animation.player (bone {bone})",
                socket.entity
            )
        })?;
        p.asset.skeleton.bone(bone)?;
    }
    Ok(())
}

impl AnimationSystem {
    /// Build-time check of every socket / look-at entity target (after all players exist).
    pub(super) fn check_rig_targets(&self, state: &RuntimeState) -> Result<()> {
        for (id, p) in &self.players {
            let Some(rig) = &p.rig else { continue };
            for (k, socket, _) in &rig.ik_sockets {
                let path = format!(
                    "{}/ik/{}/socket",
                    super::component_path(id),
                    axiom_core::local::escape(k)
                );
                at(&path, check_socket(&self.players, state, socket))?;
            }
            if let Some((_, set)) = &rig.look {
                for ((chain, _), (source, _)) in set.chains.iter().zip(&rig.look_targets) {
                    if let Source::Socket(socket) = source {
                        let path = format!(
                            "/entities/@{}/components/{LOOK_AT}/chains/{}/target",
                            axiom_core::local::escape(id),
                            axiom_core::local::escape(chain)
                        );
                        at(&path, check_socket(&self.players, state, socket))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `animation.ik` with a socket: refused unless the socket resolves to something.
    pub(super) fn check_socket(&self, state: &RuntimeState, socket: &Socket) -> Result<()> {
        check_socket(&self.players, state, socket)
    }

    /// `socket` now, in the model space of `entity` (a binding made by a command poses the
    /// hand at once instead of at the old fallback target until the next step).
    pub(super) fn socket_now(
        &self,
        state: &RuntimeState,
        entity: &str,
        socket: &Socket,
    ) -> Option<Vec3> {
        let slot = *state.compiled.indices.get(entity)?;
        let inverse = model_of(state, slot).inverse();
        socket_point(&self.players, state, socket)
            .map(|w| inverse.transform_point3(w))
            .filter(|p| p.is_finite())
    }

    /// Resolves every rig target for this step (see the module docs for the ordering) and
    /// sets each rig's world matrix. Allocation-free once the scratch list is sized.
    pub(super) fn resolve_rig_targets(&mut self, state: &RuntimeState) {
        let mut points = std::mem::take(&mut self.rig_points);
        points.clear();
        let camera = camera_point(state);
        for (id, p) in &self.players {
            let Some(rig) = &p.rig else { continue };
            let slot = state.compiled.indices[id];
            if !state.compiled.enabled[slot] {
                continue;
            }
            let model = model_of(state, slot);
            let inverse = model.inverse();
            // A degenerate transform (scale 0) resolves to nothing rather than NaN.
            let local = |w: Option<Vec3>| {
                w.map(|w| inverse.transform_point3(w))
                    .filter(|p| p.is_finite())
            };
            for (source, _) in &rig.look_targets {
                points.push(local(match source {
                    Source::Camera => camera,
                    Source::Socket(s) => socket_point(&self.players, state, s),
                    Source::Point(w) => Some(*w),
                }));
            }
            for (_, socket, _) in &rig.ik_sockets {
                points.push(local(socket_point(&self.players, state, socket)));
            }
        }
        let mut next = points.iter();
        for (id, p) in &mut self.players {
            let Some(rig) = &mut p.rig else { continue };
            let slot = state.compiled.indices[id];
            if !state.compiled.enabled[slot] {
                continue;
            }
            rig.model = model_of(state, slot);
            for (_, t) in &mut rig.look_targets {
                // An unresolved look-at target returns to the animated pose (smoothly).
                *t = next.next().copied().flatten();
            }
            for (_, _, t) in &mut rig.ik_sockets {
                // An unresolved socket keeps the last point (the hand stays where it was).
                if let Some(point) = next.next().copied().flatten() {
                    *t = Some(point);
                }
            }
        }
        self.rig_points = points;
    }
}

/// Checkpoint form of a rig.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedRig {
    /// Chain ID -> (current, previous) world point per segment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    springs: BTreeMap<String, Vec<[Vec3; 2]>>,
    #[serde(default)]
    spring_ready: bool,
    /// Chain ID -> smoothed (yaw, pitch) radians and its resolved target.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    look: BTreeMap<String, ([f32; 2], Option<Vec3>)>,
    /// IK constraint ID -> resolved socket point (model space).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sockets: BTreeMap<String, Option<Vec3>>,
    model: Mat4,
}

impl Rig {
    pub(super) fn save(&self) -> SavedRig {
        let mut springs = BTreeMap::new();
        if let Some((spring, _)) = &self.spring {
            let st = &self.spring_state;
            for (id, slot, n) in spring.chains() {
                let points = (slot..slot + n)
                    .map(|i| {
                        [
                            st.current.get(i).copied().unwrap_or(Vec3::ZERO),
                            st.previous.get(i).copied().unwrap_or(Vec3::ZERO),
                        ]
                    })
                    .collect();
                springs.insert(id.to_owned(), points);
            }
        }
        let look = match &self.look {
            Some((look, _)) => look
                .ids()
                .enumerate()
                .map(|(i, id)| {
                    let angles = self.look_state.angles.get(i).copied().unwrap_or([0.0; 2]);
                    (id.to_owned(), (angles, self.look_targets[i].1))
                })
                .collect(),
            None => BTreeMap::new(),
        };
        SavedRig {
            springs,
            spring_ready: self.spring_state.ready,
            look,
            sockets: self
                .ik_sockets
                .iter()
                .map(|(id, _, t)| (id.clone(), *t))
                .collect(),
            model: self.model,
        }
    }

    pub(super) fn load(&mut self, saved: &SavedRig) -> Result<()> {
        let finite = |v: &Vec3| v.is_finite();
        ensure!(saved.model.is_finite(), "invalid checkpoint rig transform");
        match &self.spring {
            Some((spring, _)) => {
                ensure!(
                    saved.springs.len() == spring.chains().count(),
                    "checkpoint spring chains do not match the content"
                );
                let mut state = SpringState {
                    current: vec![Vec3::ZERO; spring.segments],
                    previous: vec![Vec3::ZERO; spring.segments],
                    ready: saved.spring_ready,
                };
                for (id, slot, n) in spring.chains() {
                    let points = saved
                        .springs
                        .get(id)
                        .with_context(|| format!("checkpoint lacks spring chain {id}"))?;
                    ensure!(
                        points.len() == n && points.iter().flatten().all(finite),
                        "checkpoint spring chain {id} does not match the content"
                    );
                    for (k, [c, p]) in points.iter().enumerate() {
                        state.current[slot + k] = *c;
                        state.previous[slot + k] = *p;
                    }
                }
                self.spring_state = state;
            }
            None => ensure!(
                saved.springs.is_empty(),
                "checkpoint has spring chains the content does not"
            ),
        }
        match &self.look {
            Some((look, _)) => {
                ensure!(
                    saved.look.len() == look.len(),
                    "checkpoint look-at chains do not match the content"
                );
                let mut angles = vec![];
                for (i, id) in look.ids().enumerate() {
                    let (a, t) = saved
                        .look
                        .get(id)
                        .with_context(|| format!("checkpoint lacks look-at chain {id}"))?;
                    ensure!(
                        a.iter().all(|x| x.is_finite()) && t.as_ref().is_none_or(finite),
                        "invalid checkpoint look-at chain {id}"
                    );
                    angles.push(*a);
                    self.look_targets[i].1 = *t;
                }
                self.look_state = LookState { angles };
            }
            None => ensure!(
                saved.look.is_empty(),
                "checkpoint has look-at chains the content does not"
            ),
        }
        ensure!(
            saved
                .sockets
                .keys()
                .eq(self.ik_sockets.iter().map(|(k, _, _)| k)),
            "checkpoint IK sockets do not match"
        );
        for (id, _, t) in &mut self.ik_sockets {
            let point = saved.sockets[id.as_str()];
            ensure!(
                point.as_ref().is_none_or(finite),
                "invalid checkpoint socket point"
            );
            *t = point;
        }
        self.model = saved.model;
        self.pending = false;
        Ok(())
    }

    /// Keeps the simulation of an unchanged spring / look-at component across a rebuild
    /// (hot reload, live edits); a changed component restarts from the animated pose.
    pub(super) fn carry(&mut self, old: &Rig) {
        // Toggling the debug draw is not a change of the simulation.
        if let (Some((_, a)), Some((_, b))) = (&self.spring, &old.spring) {
            if (SpringSet {
                debug: b.debug,
                ..a.clone()
            }) == *b
            {
                self.spring_state.clone_from(&old.spring_state);
            }
        }
        if let (Some((_, a)), Some((_, b))) = (&self.look, &old.look) {
            if (LookAtSet {
                debug: b.debug,
                ..a.clone()
            }) == *b
            {
                self.look_state.clone_from(&old.look_state);
                self.look_targets.clone_from(&old.look_targets);
            }
        }
        for (id, socket, t) in &mut self.ik_sockets {
            if let Some((_, _, old)) = old
                .ik_sockets
                .iter()
                .find(|(k, s, _)| k == id && s == socket)
            {
                *t = *old;
            }
        }
    }
    /// Sets the resolved point of socket-bound IK constraint `id` (a fresh binding).
    pub(super) fn set_ik_point(&mut self, id: &str, point: Option<Vec3>) {
        if let Some((_, _, t)) = self.ik_sockets.iter_mut().find(|(k, _, _)| k == id) {
            if point.is_some() {
                *t = point;
            }
        }
    }

    /// `metrics.animation.players.ID.rig` (in `state_hash`): spring tip points, look-at
    /// angles (degrees) and resolved socket points.
    pub(super) fn metrics(&self) -> Value {
        let mut out = json!({});
        if let Some((spring, _)) = &self.spring {
            out["spring"] = spring
                .chains()
                .map(|(id, slot, n)| {
                    let tip = self.spring_state.current.get(slot + n - 1).copied();
                    (id.to_owned(), json!(tip))
                })
                .collect::<serde_json::Map<_, _>>()
                .into();
        }
        if let Some((look, _)) = &self.look {
            out["look_at"] = look
                .ids()
                .enumerate()
                .map(|(i, id)| {
                    let [y, p] = self.look_state.angles.get(i).copied().unwrap_or([0.0; 2]);
                    (id.to_owned(), json!([y.to_degrees(), p.to_degrees()]))
                })
                .collect::<serde_json::Map<_, _>>()
                .into();
        }
        if !self.ik_sockets.is_empty() {
            out["sockets"] = self
                .ik_sockets
                .iter()
                .map(|(id, _, t)| (id.clone(), json!(t)))
                .collect::<serde_json::Map<_, _>>()
                .into();
        }
        out
    }

    /// Scene-view debug draw (`debug: true` on the component): spring joints (cyan) and
    /// colliders (magenta: center and the six axis extremes of each end sphere), look-at facing
    /// rays (yellow).
    pub(super) fn debug_draw(
        &self,
        s: &CompiledSkeleton,
        pose: &Pose,
        model: Mat4,
        out: &mut Vec<Instance>,
    ) {
        let cube = |at: Vec3, size: f32, color: [f32; 3]| {
            Instance::new(
                Mat4::from_scale_rotation_translation(Vec3::splat(size), Quat::IDENTITY, at),
                color,
                1.0,
                false,
            )
        };
        if let Some((spring, set)) = &self.spring {
            if set.debug {
                let shapes = std::cell::RefCell::new(&mut *out);
                spring.debug_shapes(
                    s,
                    pose,
                    model,
                    |p| shapes.borrow_mut().push(cube(p, 0.025, [0.1, 0.9, 1.0])),
                    |a, b, r| {
                        // Center plus the six axis extremes of each end sphere: shows the
                        // extent without hiding the character.
                        let mut out = shapes.borrow_mut();
                        for c in [a, b] {
                            out.push(cube(c, 0.02, [1.0, 0.2, 0.9]));
                            for d in [Vec3::X, Vec3::Y, Vec3::Z] {
                                out.push(cube(c + d * r, 0.012, [1.0, 0.2, 0.9]));
                                out.push(cube(c - d * r, 0.012, [1.0, 0.2, 0.9]));
                            }
                            if a == b {
                                break;
                            }
                        }
                    },
                );
            }
        }
        if let Some((look, set)) = &self.look {
            if set.debug {
                look.debug_rays(s, pose, model, |p, d| {
                    let len = 0.35;
                    out.push(Instance::new(
                        Mat4::from_scale_rotation_translation(
                            Vec3::new(0.008, len, 0.008),
                            Quat::from_rotation_arc(Vec3::Y, d),
                            p + d * len * 0.5,
                        ),
                        [1.0, 0.85, 0.1],
                        1.0,
                        false,
                    ));
                });
                for (_, t) in &self.look_targets {
                    if let Some(t) = t {
                        out.push(cube(model.transform_point3(*t), 0.04, [1.0, 0.85, 0.1]));
                    }
                }
            }
        }
    }
}

/// Loads a player's saved rig (`None` in the checkpoint while the content has one, or the
/// other way round, is refused rather than silently reset).
pub(super) fn load_rig(p: &mut Controller, saved: Option<&SavedRig>) -> Result<()> {
    match (&mut p.rig, saved) {
        (None, None) => Ok(()),
        (Some(rig), Some(saved)) => rig.load(saved),
        (Some(_), None) => {
            bail!("checkpoint lacks spring / look-at / IK socket state for this player")
        }
        (None, Some(_)) => {
            bail!("checkpoint has spring / look-at / IK socket state the content does not")
        }
    }
}
