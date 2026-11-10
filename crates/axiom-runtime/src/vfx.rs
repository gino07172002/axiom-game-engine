//! Seeded, bounded effects. CPU and GPU evaluation share the same analytic particle contract.
use crate::{presentation::RenderFrame, world::*};
use anyhow::{ensure, Context, Result};
use axiom_core::*;
use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
};

pub const MAX_PARTICLES: usize = 8192;
/// Keys of `size_curve` / stops of `color_gradient` (the GPU uniform has room for this many).
pub const MAX_CURVE_KEYS: usize = 8;
/// One `size_curve` key: particle size (m) at normalized age `t`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SizeKey {
    pub t: f32,
    pub size: f32,
}
/// One `color_gradient` stop: RGBA at normalized age `t`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ColorStop {
    pub t: f32,
    pub color: [f32; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Emitter {
    pub backend: String,
    pub renderer: String,
    pub count: u32,
    pub seed: u32,
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    pub acceleration: [f32; 3],
    pub spread: f32,
    pub lifetime: f32,
    pub size: f32,
    pub end_size: f32,
    pub color: [f32; 4],
    pub end_color: [f32; 4],
    /// Size over life through 2..=8 keys (linear between keys, held outside them); replaces
    /// `size` -> `end_size` for particles when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_curve: Option<Vec<SizeKey>>,
    /// Colour over life through 2..=8 stops; replaces `color` -> `end_color` for particles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_gradient: Option<Vec<ColorStop>>,
    pub field: String,
    pub floor: Option<f32>,
    pub bounce: f32,
    pub attachment: Option<String>,
    pub bone: Option<String>,
    pub texture_mask: Option<Vec<String>>,
}
impl Default for Emitter {
    fn default() -> Self {
        Self {
            backend: "cpu".into(),
            renderer: "sprite".into(),
            count: 128,
            seed: 1,
            origin: [0.0; 3],
            velocity: [0.0, 2.0, 0.0],
            acceleration: [0.0, -1.0, 0.0],
            spread: 0.6,
            lifetime: 3.0,
            size: 0.12,
            end_size: 0.03,
            color: [0.2, 0.8, 1.0, 0.9],
            end_color: [0.2, 0.5, 1.0, 0.0],
            size_curve: None,
            color_gradient: None,
            field: "fountain".into(),
            floor: None,
            bounce: 0.65,
            attachment: None,
            bone: None,
            texture_mask: None,
        }
    }
}
impl Emitter {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.bone.is_none() || self.attachment.is_some(),
            "bone attachment requires an entity"
        );
        ensure!(
            self.renderer != "ribbon" || self.count == 1,
            "one ribbon emitter represents one trail"
        );
        ensure!(
            ["cpu", "gpu"].contains(&self.backend.as_str())
                && ["sprite", "mesh", "ribbon", "light"].contains(&self.renderer.as_str())
                && ["fountain", "smoke", "snow", "texture"].contains(&self.field.as_str()),
            "unknown VFX evaluator or renderer"
        );
        ensure!(
            (1..=2048).contains(&self.count)
                && (0.05..=30.0).contains(&self.lifetime)
                && (0.0..=20.0).contains(&self.spread)
                && (0.005..=4.0).contains(&self.size)
                && (0.0..=4.0).contains(&self.end_size)
                && (0.0..=1.0).contains(&self.bounce),
            "VFX count/lifetime/size budget invalid"
        );
        ensure!(
            self.origin
                .iter()
                .chain(&self.velocity)
                .chain(&self.acceleration)
                .all(|v| v.is_finite() && v.abs() <= 1000.0)
                && self
                    .color
                    .iter()
                    .chain(&self.end_color)
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                && self
                    .floor
                    .is_none_or(|v| v.is_finite() && v.abs() <= 1000.0),
            "invalid VFX values"
        );
        ensure!(
            self.backend != "gpu" || self.floor.is_none(),
            "GPU analytic backend does not support CPU collision constraints"
        );
        ensure!(self.backend!="gpu"||["sprite","mesh"].contains(&self.renderer.as_str()),"GPU backend supports sprite and mesh trajectories; ribbon/light require the CPU backend");
        ensure!(
            self.backend != "gpu" || self.texture_mask.is_none(),
            "GPU analytic backend does not sample authored bitmap masks"
        );
        let times_ok = |ts: &[f32]| {
            (2..=MAX_CURVE_KEYS).contains(&ts.len())
                && ts.iter().all(|t| t.is_finite() && (0.0..=1.0).contains(t))
                && ts.windows(2).all(|w| w[0] < w[1])
        };
        if let Some(keys) = &self.size_curve {
            ensure!(
                times_ok(&keys.iter().map(|k| k.t).collect::<Vec<_>>())
                    && keys
                        .iter()
                        .all(|k| k.size.is_finite() && (0.0..=4.0).contains(&k.size)),
                "size_curve needs 2..={MAX_CURVE_KEYS} keys with t 0..1 strictly increasing and size 0..4"
            );
        }
        if let Some(stops) = &self.color_gradient {
            ensure!(
                times_ok(&stops.iter().map(|s| s.t).collect::<Vec<_>>())
                    && stops
                        .iter()
                        .flat_map(|s| s.color)
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(&v)),
                "color_gradient needs 2..={MAX_CURVE_KEYS} stops with t 0..1 strictly increasing and channels 0..1"
            );
        }
        if let Some(mask) = &self.texture_mask {
            ensure!(
                !mask.is_empty()
                    && mask.len() <= 64
                    && !mask[0].is_empty()
                    && mask[0].len() <= 64
                    && mask
                        .iter()
                        .all(|r| r.len() == mask[0].len()
                            && r.bytes().all(|b| b == b'0' || b == b'1'))
                    && mask.iter().any(|r| r.contains('1')),
                "VFX bitmap mask must be a nonempty rectangular 1-bit image <=64x64"
            );
        }
        Ok(())
    }
    pub fn field_id(&self) -> u32 {
        match self.field.as_str() {
            "smoke" => 1,
            "snow" => 2,
            "texture" => 3,
            _ => 0,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub emitters: BTreeMap<String, Emitter>,
}
impl Library {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.emitters.len() <= 64, "VFX emitter budget exceeded");
        let mut total = 0;
        for (id, e) in &self.emitters {
            valid_id(id)?;
            axiom_core::local::at(
                &format!(
                    "/resources/vfx.library/emitters/{}",
                    axiom_core::local::escape(id)
                ),
                e.validate(),
            )?;
            total += e.count as usize;
        }
        ensure!(total <= MAX_PARTICLES, "VFX particle budget exceeded");
        ensure!(
            self.emitters
                .values()
                .filter(|e| e.renderer == "light")
                .map(|e| e.count)
                .sum::<u32>()
                <= 32,
            "VFX local light budget exceeded (32)"
        );
        Ok(())
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Particle {
    pub position_size: [f32; 4],
    pub color: [f32; 4],
    pub misc: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct Batch {
    pub id: String,
    pub emitter: Emitter,
    pub time: f32,
    pub origin: Vec3,
    pub particles: Vec<Particle>,
}
#[derive(Clone, Debug)]
pub struct Ribbon {
    pub points: Vec<Vec3>,
    pub width: f32,
    pub color: [f32; 4],
    /// Entity the emitter is attached to (editor-view hiding), if any.
    pub attachment: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Light {
    pub position: Vec3,
    pub radius: f32,
    pub color: [f32; 3],
    pub intensity: f32,
    /// Entity the emitting VFX is attached to (editor-view hiding), if any.
    pub attachment: Option<String>,
}
#[derive(Clone, Default)]
pub struct Frame {
    pub batches: Vec<Batch>,
    pub ribbons: Vec<Ribbon>,
}
pub fn random(seed: u32, index: u32, salt: u32) -> f32 {
    let mut v = seed
        .wrapping_add(index.wrapping_mul(747796405))
        .wrapping_add(salt.wrapping_mul(2891336453));
    v = ((v >> ((v >> 28) + 4)) ^ v).wrapping_mul(277803737);
    v = (v >> 22) ^ v;
    (v & 0x00ffffff) as f32 / 16777216.0
}
/// Piecewise-linear value at `t` through `(t, value)` keys sorted by `t`: the first value
/// before the first key, the last after the last. The GPU compute pass (vfx_compute.wgsl,
/// `curve`) uses the same operations in the same order.
fn piecewise<const N: usize>(keys: impl Iterator<Item = (f32, [f32; N])>, t: f32) -> [f32; N] {
    let mut prev: Option<(f32, [f32; N])> = None;
    for (kt, v) in keys {
        match prev {
            None if t <= kt => return v,
            Some((pt, pv)) if t <= kt => {
                let u = (t - pt) / (kt - pt);
                return std::array::from_fn(|i| pv[i] + (v[i] - pv[i]) * u);
            }
            _ => prev = Some((kt, v)),
        }
    }
    prev.map_or([0.0; N], |(_, v)| v)
}
pub fn size_at(e: &Emitter, t: f32) -> f32 {
    match &e.size_curve {
        Some(keys) => piecewise(keys.iter().map(|k| (k.t, [k.size])), t)[0],
        None => e.size + (e.end_size - e.size) * t,
    }
}
pub fn color_at(e: &Emitter, t: f32) -> [f32; 4] {
    match &e.color_gradient {
        Some(stops) => piecewise(stops.iter().map(|s| (s.t, s.color)), t),
        None => Vec4::from_array(e.color)
            .lerp(Vec4::from_array(e.end_color), t)
            .to_array(),
    }
}
pub fn evaluate(e: &Emitter, index: u32, time: f32, origin: Vec3) -> Particle {
    let r = |s| random(e.seed, index, s);
    let age = (time / e.lifetime + r(0)).fract() * e.lifetime;
    let t = age / e.lifetime;
    let mut spawn = Vec3::new((r(1) - 0.5) * e.spread, 0.0, (r(2) - 0.5) * e.spread);
    let mut velocity = Vec3::from_array(e.velocity)
        + Vec3::new(
            (r(3) - 0.5) * e.spread,
            r(4) * e.spread,
            (r(5) - 0.5) * e.spread,
        );
    if e.field == "snow" {
        spawn = Vec3::new((r(1) - 0.5) * 3.5, 3.5, (r(2) - 0.5) * 3.5);
        velocity.y = -velocity.y.abs();
    }
    if let Some(mask) = &e.texture_mask {
        let pixels = mask
            .iter()
            .enumerate()
            .flat_map(|(y, row)| {
                row.bytes()
                    .enumerate()
                    .filter_map(move |(x, b)| (b == b'1').then_some((x, y)))
            })
            .collect::<Vec<_>>();
        let (x, y) = pixels[index as usize % pixels.len()];
        spawn = Vec3::new(
            (x as f32 / (mask[0].len().max(2) - 1) as f32 - 0.5) * 3.0,
            (1.0 - y as f32 / (mask.len().max(2) - 1) as f32) * 2.5,
            0.0,
        );
    }
    let acceleration = Vec3::from_array(e.acceleration);
    let mut p = origin + spawn + velocity * age + acceleration * age * age * 0.5;
    if e.field == "smoke" {
        p.x += (age * 2.1 + r(5) * 6.0).sin() * age * 0.16;
        p.z += (age * 1.7 + r(2) * 6.0).cos() * age * 0.16;
    }
    if let Some(floor) = e.floor {
        // Solve each impact against the same analytic ballistic trajectory, then reflect velocity.
        let mut remaining = age;
        let mut y = (origin.y + spawn.y).max(floor);
        let mut vy = velocity.y;
        for _ in 0..8 {
            let g = acceleration.y;
            let impact = if g.abs() < 1e-6 {
                if vy < 0.0 {
                    (floor - y) / vy
                } else {
                    f32::INFINITY
                }
            } else {
                let d = vy * vy - 2.0 * g * (y - floor);
                if d >= 0.0 {
                    (-vy - d.sqrt()) / g
                } else {
                    f32::INFINITY
                }
            };
            if impact > 0.00001 && impact < remaining {
                y = floor;
                vy = -(vy + g * impact) * e.bounce;
                remaining -= impact;
            } else {
                y += vy * remaining + g * remaining * remaining * 0.5;
                break;
            }
        }
        p.y = y.max(floor);
    }
    Particle {
        position_size: [p.x, p.y, p.z, size_at(e, t)],
        color: color_at(e, t),
        misc: [
            r(6) * std::f32::consts::TAU + age * 0.6,
            if e.renderer == "mesh" { 1.0 } else { 0.0 },
            t,
            0.0,
        ],
    }
}
struct Running {
    time: f32,
    paused: bool,
    trail: VecDeque<Vec3>,
}
pub struct VfxSystem {
    pub library: Library,
    running: BTreeMap<String, Running>,
}
impl VfxSystem {
    fn origin(&self, state: &RuntimeState, e: &Emitter) -> Vec3 {
        if let Some(id) = &e.attachment {
            if let Some(bone) = &e.bone {
                if let Some(m) = state
                    .animation_cues
                    .sockets
                    .get(&(id.clone(), bone.clone()))
                {
                    return m.transform_point3(Vec3::from_array(e.origin));
                }
            }
            if let Some(&slot) = state.compiled.indices.get(id) {
                return state.matrices()[slot].transform_point3(Vec3::from_array(e.origin));
            }
        }
        Vec3::from_array(e.origin)
    }
}
impl RuntimeSystem for VfxSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, _input: &ActionMap) {
        for (id, e) in &self.library.emitters {
            let origin = self.origin(state, e);
            let r = self.running.get_mut(id).unwrap();
            if r.paused {
                continue;
            }
            r.time += FIXED_DT;
            if e.renderer == "ribbon" {
                r.trail.push_back(origin);
                if r.trail.len() > 96 {
                    r.trail.pop_front();
                }
            }
        }
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        cap: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if cap != "vfx" {
            return None;
        }
        Some((|| {
            let id = args["id"].as_str().context("emitter ID required")?;
            let e = self.library.emitters.get(id).context("emitter not found")?;
            let r = self.running.get_mut(id).unwrap();
            match action {
                "restart" => {
                    r.time = 0.0;
                    r.trail.clear();
                }
                "pause" => {
                    r.paused = args["paused"].as_bool().context("paused bool required")?;
                }
                "inspect" => {}
                _ => anyhow::bail!("unknown VFX action"),
            }
            let time = r.time;
            let paused = r.paused;
            let trail = r.trail.len();
            let origin = self.origin(state, e);
            let sample = (0..e.count.min(8))
                .map(|i| evaluate(e, i, time, origin).position_size)
                .collect::<Vec<_>>();
            Ok(
                json!({"id":id,"backend":e.backend,"renderer":e.renderer,"time":time,"paused":paused,"count":e.count,"trail_points":trail,"cpu_reference_sample":sample,"gpu_note":if e.backend=="gpu"{Some("reference values; GPU execution is checked separately by readback")}else{None}}),
            )
        })())
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        _next: &mut RuntimeState,
    ) -> Result<()> {
        let old = old
            .as_any()
            .downcast_ref::<Self>()
            .context("VFX transfer type")?;
        for (id, r) in &mut self.running {
            if self.library.emitters.get(id) == old.library.emitters.get(id) {
                if let Some(p) = old.running.get(id) {
                    r.time = p.time;
                    r.paused = p.paused;
                    r.trail = p.trail.clone();
                }
            }
        }
        Ok(())
    }
    fn extract(&self, state: &RuntimeState, frame: &mut RenderFrame) {
        for (id, e) in &self.library.emitters {
            let r = &self.running[id];
            let origin = self.origin(state, e);
            if e.renderer == "ribbon" {
                frame.vfx.ribbons.push(Ribbon {
                    points: r.trail.iter().copied().collect(),
                    width: e.size,
                    color: e.color,
                    attachment: e.attachment.clone(),
                });
                continue;
            }
            let particles = if e.backend == "cpu" {
                (0..e.count)
                    .map(|i| evaluate(e, i, r.time, origin))
                    .collect()
            } else {
                vec![]
            };
            if e.renderer == "light" {
                for i in 0..e.count.min(32) {
                    let p = evaluate(e, i, r.time, origin);
                    frame.lights.push(Light {
                        position: Vec3::new(
                            p.position_size[0],
                            p.position_size[1],
                            p.position_size[2],
                        ),
                        radius: 2.0,
                        color: [p.color[0], p.color[1], p.color[2]],
                        intensity: 1.8 * p.color[3],
                        attachment: e.attachment.clone(),
                    });
                }
            }
            frame.vfx.batches.push(Batch {
                id: id.clone(),
                emitter: e.clone(),
                time: r.time,
                origin,
                particles,
            });
        }
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([(
            "vfx".into(),
            json!({"emitters":self.running.iter().map(|(id,r)|(id,json!({"backend":self.library.emitters[id].backend,"count":self.library.emitters[id].count,"time":r.time,"paused":r.paused,"trail_points":r.trail.len()}))).collect::<BTreeMap<_,_>>() }),
        )])
    }
}
pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.capabilities.insert("vfx".into(), 1);
    r.authoring.resource("vfx.library","vfx",json!({"emitters":"stable ID -> seeded emitter: backend cpu|gpu, renderer sprite|mesh|ribbon|light, count, lifetime, origin, velocity, acceleration, color/end_color, size/end_size, optional size_curve [{t,size}] and color_gradient [{t,color}] (2..8 keys, t 0..1 increasing; replace the two-point size/colour for particles), field fountain|smoke|snow|texture, optional floor/bounce, attachment/bone and 1-bit texture_mask"}),|v|serde_json::from_value::<Library>(v.clone())?.validate());
    r.authoring.systems.insert(
        "vfx.simulate".into(),
        SystemDescriptor {
            id: "vfx.simulate".into(),
            capability: "vfx".into(),
            phase: 70,
            reads: vec!["vfx.library".into()],
            writes: vec![],
            after: vec!["animation.evaluate".into(), "motion.integrate".into()],
        },
    );
    for action in ["restart", "pause", "inspect"] {
        r.authoring.action(
            &format!("vfx.{action}"),
            "vfx",
            json!({"id":"emitter stable ID","paused":"required bool for pause"}),
            |v| {
                ensure!(
                    v["id"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty() && s.len() <= 128),
                    "invalid emitter ID"
                );
                Ok(())
            },
        );
    }
    r.factories.insert("vfx.simulate".into(), |state| {
        let library: Library = state.compiled.source.resource("vfx.library")?;
        library.validate()?;
        let mut running = BTreeMap::new();
        for (id, e) in &library.emitters {
            if let Some(entity) = &e.attachment {
                ensure!(
                    state.compiled.indices.contains_key(entity),
                    "VFX attachment entity missing"
                );
                if let Some(bone) = &e.bone {
                    ensure!(
                        state
                            .animation_cues
                            .bone_names
                            .get(entity)
                            .is_some_and(|names| names.contains(bone)),
                        "VFX attachment bone missing"
                    );
                    state
                        .animation_cues
                        .socket_requests
                        .entry(entity.clone())
                        .or_default()
                        .insert(bone.clone());
                }
            }
            running.insert(
                id.clone(),
                Running {
                    time: 0.0,
                    paused: false,
                    trail: VecDeque::new(),
                },
            );
        }
        Ok(Box::new(VfxSystem { library, running }))
    });
    r.authoring.topics.insert("vfx".into(),json!({"library":"vfx.library","actions":["vfx.restart","vfx.pause","vfx.inspect"],"observe":"project /vfx/emitters/<stable-id>","gpu":"analytic trajectories evaluated in a compute pass; validation readback compares CPU reference","checkpoints":"explicitly unsupported for active VFX state"}));
}
