//! Facing monitor: every animated character's model front versus where it actually moves.
//!
//! Engine convention (glTF, `fsm`, `nav.agent`, `path.follow`): a model's front is its entity
//! +Z after the entity's world rotation. A character that keeps moving at speed with that
//! front pointing away from its motion runs backwards on screen — a wrong `model_yaw`, a
//! controller using another convention, or an asset exported facing the other way. The
//! monitor reports each character's front, speed and alignment in `observe facing`, and
//! latches a `facing_mismatch` diagnostic for it. It reads transforms only (already part of
//! the state hash), so the `facing` section is left out of `state_hash`.
use crate::world::{RuntimeState, FIXED_DT};
use glam::Vec3;
use serde_json::{json, Value};

/// Planar speed (m/s) below which the motion direction is not judged.
const MIN_SPEED: f32 = 0.5;
/// cos of the angle between the model front and the motion beyond which the character runs
/// backwards (about 135°). Strafing (90°) and short knock-backs stay below it.
const BACKWARD_DOT: f32 = -0.7;
/// Consecutive fixed steps of backward motion before it is reported (0.5 s at 120 Hz).
const REPORT_AFTER: u32 = 60;

#[derive(Clone, Debug, Default)]
struct Track {
    slot: usize,
    id: String,
    last: Option<[f32; 2]>,
    front: [f32; 2],
    speed: f32,
    /// Alignment of the front with the motion while moving at `MIN_SPEED` or faster.
    dot: Option<f32>,
    backward: u32,
    mismatch: Option<Mismatch>,
}
#[derive(Clone, Debug)]
struct Mismatch {
    since_tick: u64,
    ticks: u32,
    worst_dot: f32,
    speed: f32,
}

/// Planar position and model front of `slot` in world space.
fn planar(state: &RuntimeState, slot: usize) -> ([f32; 2], [f32; 2]) {
    let m = crate::trigger::world_matrix(state, slot);
    let p = m.transform_point3(Vec3::ZERO);
    let f = m.transform_vector3(Vec3::Z);
    (
        [p.x, p.z],
        glam::Vec2::new(f.x, f.z).normalize_or_zero().to_array(),
    )
}

#[derive(Clone, Debug, Default)]
pub struct Monitor {
    tracks: Vec<Track>,
}
impl Monitor {
    /// Characters = entities with an animation player (the cue catalog), in ID order, starting
    /// from their current positions. A diagnostic: like the gameplay trace it is not part of
    /// checkpoints, and a restored runtime starts a new window from the restored transforms.
    pub fn new(state: &RuntimeState) -> Self {
        let tracks = state
            .animation_cues
            .catalog
            .keys()
            .filter_map(|id| {
                let slot = *state.compiled.indices.get(id)?;
                state.transforms[slot].as_ref()?;
                Some(Track {
                    slot,
                    id: id.clone(),
                    last: Some(planar(state, slot).0),
                    front: planar(state, slot).1,
                    ..Track::default()
                })
            })
            .collect();
        Self { tracks }
    }
    /// After a fixed step: no allocation (the tracks are built once).
    pub fn step(&mut self, state: &RuntimeState) {
        for t in &mut self.tracks {
            if !state.compiled.enabled[t.slot] || state.transforms[t.slot].is_none() {
                t.last = None;
                t.backward = 0;
                t.dot = None;
                continue;
            }
            let m = crate::trigger::world_matrix(state, t.slot);
            let position = m.transform_point3(Vec3::ZERO);
            let front = m.transform_vector3(Vec3::Z);
            let front = glam::Vec2::new(front.x, front.z).normalize_or_zero();
            t.front = front.to_array();
            let now = [position.x, position.z];
            let Some(last) = t.last.replace(now) else {
                continue;
            };
            let motion = glam::Vec2::new(now[0] - last[0], now[1] - last[1]);
            t.speed = motion.length() / FIXED_DT;
            if t.speed < MIN_SPEED || front == glam::Vec2::ZERO {
                t.dot = None;
                t.backward = 0;
                continue;
            }
            let dot = front.dot(motion / motion.length());
            t.dot = Some(dot);
            if dot >= BACKWARD_DOT {
                t.backward = 0;
                continue;
            }
            t.backward += 1;
            if t.backward >= REPORT_AFTER {
                let m = t.mismatch.get_or_insert(Mismatch {
                    since_tick: state.tick + 1 - u64::from(REPORT_AFTER),
                    ticks: 0,
                    worst_dot: dot,
                    speed: t.speed,
                });
                m.ticks = m.ticks.max(t.backward);
                if dot < m.worst_dot {
                    m.worst_dot = dot;
                    m.speed = t.speed;
                }
            }
        }
    }
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
    /// `{characters: {ID: {front:[x,z], speed, dot}}, mismatches: [diagnostic…]}`.
    pub fn observation(&self) -> Value {
        let round = |v: f32| (v * 1000.0).round() / 1000.0;
        let characters: serde_json::Map<String, Value> = self
            .tracks
            .iter()
            .map(|t| {
                (
                    t.id.clone(),
                    json!({"front":[round(t.front[0]),round(t.front[1])],"speed":round(t.speed),"dot":t.dot.map(round)}),
                )
            })
            .collect();
        let mismatches: Vec<Value> = self
            .tracks
            .iter()
            .filter_map(|t| {
                let m = t.mismatch.as_ref()?;
                Some(json!({
                    "code":"facing_mismatch",
                    "severity":"warning",
                    "entity":t.id,
                    "since_tick":m.since_tick,
                    "ticks":m.ticks,
                    "dot":round(m.worst_dot),
                    "speed":round(m.speed),
                    "message":format!(
                        "{} moved backwards for {} steps: its model front (entity +Z after rotation) points {:.0}° away from its motion; check the controller's model_yaw (action.actor) or the asset's forward axis",
                        t.id, m.ticks, m.worst_dot.clamp(-1.0, 1.0).acos().to_degrees()
                    ),
                }))
            })
            .collect();
        json!({"characters":characters,"mismatches":mismatches})
    }
}
