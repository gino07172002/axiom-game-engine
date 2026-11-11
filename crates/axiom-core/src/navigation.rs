//! Navigation agents (`nav.agent`) and their navigation grid (`nav.grid`), executed by the
//! runtime system `nav.steer`. Genre-neutral: pedestrians, NPCs and pets walk to a point or an
//! entity around static obstacles. (RTS units keep their own grid in `axiom_rts::nav`; both
//! run on the shared A* engine in `crate::grid_search`.)
//!
//! The grid is planar (XZ) and built from static obstacles when the runtime is built, never per
//! step: `nav.blocker` rectangles (authored positions, as for FSM agents and the action
//! controller) and, optionally, `physics.collider` shapes of entities that no system moves.
//! Each cell stores its clearance (distance from the cell center to the nearest obstacle), so
//! one grid serves agents of every radius.
//!
//! This module is pure: types, validation, the grid plan (extent and obstacle list, used by
//! `World::compile` for budget errors and by the runtime), the clearance raster and the
//! line-of-sight walk used to straighten paths.
use crate::{blocker::Rect2, CompiledEntity};
use anyhow::{ensure, Context, Result};
use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Entities with `nav.agent` per world.
pub const MAX_NAV_AGENTS: usize = 1024;
/// Grid cells (512 x 512 at most; either side at most `MAX_GRID_SIDE`).
pub const MAX_NAV_CELLS: usize = 262_144;
pub const MAX_GRID_SIDE: u32 = 1024;
/// Static obstacles feeding the grid (`nav.blocker` is itself capped at 512).
pub const MAX_NAV_OBSTACLES: usize = 1024;
/// Straightened waypoints per agent path; longer paths fail with `path_too_long`.
pub const MAX_WAYPOINTS: usize = 128;
/// `on_arrive` / `on_fail` events one fixed step may emit across all agents.
pub const MAX_NAV_EVENTS_PER_STEP: usize = 64;
pub const MAX_AGENT_RADIUS: f32 = 2.0;
/// Margin (meters) around obstacles and agents when the grid bounds are automatic.
pub const AUTO_MARGIN: f32 = 4.0;
/// Colliders count as obstacles when their world box overlaps this height band above
/// `floor_y` (floors and curbs below, signs and roofs above are ignored).
pub const OBSTACLE_BAND: (f32, f32) = (0.3, 2.0);
/// A blocked goal (or start) moves to the nearest walkable cell within this distance.
pub const SNAP_DISTANCE: f32 = 1.5;
const MAX_COORDINATE: f32 = 100_000.0;

fn yes() -> bool {
    true
}
fn default_speed() -> f32 {
    1.4
}
fn default_acceleration() -> f32 {
    8.0
}
fn default_turn_rate() -> f32 {
    720.0
}
fn default_radius() -> f32 {
    0.3
}
fn default_arrive() -> f32 {
    0.2
}
fn default_cell() -> f32 {
    0.25
}
fn default_expansions() -> u32 {
    4096
}

/// Where an agent walks to.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum NavTarget {
    /// A world point (Y is ignored: agents walk on their own height).
    Point([f32; 3]),
    /// Another entity's current world position (re-planned when it moves).
    Entity(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavAgent {
    /// Top speed, meters per second.
    #[serde(default = "default_speed")]
    pub speed: f32,
    /// Speeding up and braking, meters per second squared (agents brake to stop at the goal).
    #[serde(default = "default_acceleration")]
    pub acceleration: f32,
    /// Facing turn speed in degrees per second (0 = face the walking direction at once).
    #[serde(default = "default_turn_rate")]
    pub turn_rate: f32,
    /// Body radius in meters: clearance from obstacles and other agents.
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// Arrived when the agent is this close to the goal (meters).
    #[serde(default = "default_arrive")]
    pub arrive_distance: f32,
    /// Walk here when the runtime starts (`nav.go` sets it at runtime).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<NavTarget>,
    /// Entity targets only: after arriving, start walking again when the target moves away.
    #[serde(default)]
    pub follow_target: bool,
    /// Keep apart from other agents (simple separation).
    #[serde(default = "yes")]
    pub avoidance: bool,
    /// `behavior.program` event emitted on arrival ("" = none).
    #[serde(default)]
    pub on_arrive: String,
    /// `behavior.program` event emitted when the target cannot be reached ("" = none).
    #[serde(default)]
    pub on_fail: String,
    /// A disabled agent never moves the entity.
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavBounds {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// `nav.grid` resource (optional; every field has a default).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavSettings {
    /// Cell edge in meters.
    #[serde(default = "default_cell")]
    pub cell_size: f32,
    /// World XZ rectangle covered by the grid; automatic (obstacles, every placed entity and
    /// point targets, plus `AUTO_MARGIN`) when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<NavBounds>,
    /// Ground height the collider band is measured from.
    #[serde(default)]
    pub floor_y: f32,
    /// Static `physics.collider` shapes are obstacles too.
    #[serde(default = "yes")]
    pub colliders: bool,
    /// Pathfinding work per fixed step (node expansions + line-of-sight cells); requests wait
    /// for the next step when it runs out.
    #[serde(default = "default_expansions")]
    pub expansions_per_step: u32,
    /// Agent sent by a primary click on the ground (screen pointer); "" = none.
    #[serde(default)]
    pub pointer_agent: String,
}

impl Default for NavSettings {
    fn default() -> Self {
        Self {
            cell_size: default_cell(),
            bounds: None,
            floor_y: 0.0,
            colliders: true,
            expansions_per_step: default_expansions(),
            pointer_agent: String::new(),
        }
    }
}

fn in_range(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

fn event_ok(id: &str) -> Result<()> {
    if !id.is_empty() {
        crate::valid_id(id)?;
    }
    Ok(())
}

impl NavTarget {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Point(p) => ensure!(
                p.iter()
                    .all(|v| in_range(*v, -MAX_COORDINATE, MAX_COORDINATE)),
                "target point must be finite and within ±{MAX_COORDINATE} m"
            ),
            Self::Entity(id) => crate::valid_id(id).context("target entity")?,
        }
        Ok(())
    }
}

impl NavAgent {
    pub fn new() -> Self {
        Self {
            speed: default_speed(),
            acceleration: default_acceleration(),
            turn_rate: default_turn_rate(),
            radius: default_radius(),
            arrive_distance: default_arrive(),
            target: None,
            follow_target: false,
            avoidance: true,
            on_arrive: String::new(),
            on_fail: String::new(),
            enabled: true,
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(in_range(self.speed, 0.0, 50.0), "speed must be 0..50 m/s");
        ensure!(
            in_range(self.acceleration, 0.1, 200.0),
            "acceleration must be 0.1..200 m/s²"
        );
        ensure!(
            in_range(self.turn_rate, 0.0, 3600.0),
            "turn_rate must be 0..3600 degrees per second (0 = instant)"
        );
        ensure!(
            in_range(self.radius, 0.05, MAX_AGENT_RADIUS),
            "radius must be 0.05..{MAX_AGENT_RADIUS} m"
        );
        ensure!(
            in_range(self.arrive_distance, 0.01, 50.0),
            "arrive_distance must be 0.01..50 m"
        );
        if let Some(target) = &self.target {
            target.validate()?;
        }
        ensure!(
            !self.follow_target || matches!(self.target, None | Some(NavTarget::Entity(_))),
            "follow_target applies to entity targets only"
        );
        event_ok(&self.on_arrive).context("on_arrive")?;
        event_ok(&self.on_fail).context("on_fail")?;
        Ok(())
    }
}

impl Default for NavAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl NavSettings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            in_range(self.cell_size, 0.05, 4.0),
            "cell_size must be 0.05..4 m"
        );
        if let Some(b) = &self.bounds {
            ensure!(
                b.min
                    .iter()
                    .chain(&b.max)
                    .all(|v| in_range(*v, -MAX_COORDINATE, MAX_COORDINATE))
                    && b.min[0] < b.max[0]
                    && b.min[1] < b.max[1],
                "bounds need finite min < max (meters)"
            );
        }
        ensure!(
            in_range(self.floor_y, -MAX_COORDINATE, MAX_COORDINATE),
            "floor_y must be finite"
        );
        ensure!(
            (64..=262_144).contains(&self.expansions_per_step),
            "expansions_per_step must be 64..262144"
        );
        event_ok(&self.pointer_agent).context("pointer_agent")?;
        Ok(())
    }
}

pub fn agent_schema() -> Value {
    json!({
        "speed": "m/s 0..50, default 1.4",
        "acceleration": "m/s² 0.1..200, default 8 (speeds up and brakes to stop at the goal)",
        "turn_rate": "degrees/second 0..3600, default 720; 0 = face the walking direction at once (models look along +Z)",
        "radius": format!("body radius m 0.05..{MAX_AGENT_RADIUS}, default 0.3: clearance from obstacles and other agents; doorways wider than the diameter are found"),
        "arrive_distance": "m 0.01..50, default 0.2",
        "target": "optional {point:[x,y,z]} | {entity:ID}: walk there when the runtime starts (Y ignored)",
        "follow_target": "bool, default false; entity targets: walk again when the target moves away after arriving",
        "avoidance": "bool, default true: simple separation from other nav agents",
        "on_arrive": "behavior.program event on arrival (\"\" = none); may declare text fields agent, target, reason",
        "on_fail": "behavior.program event when the target is unreachable (reason: unreachable|outside_grid|goal_blocked|start_blocked|path_too_long|stuck)",
        "enabled": "bool, default true",
        "conflict": "an entity with nav.agent must not have another mover (core.motion, fsm.agent, rts.unit, action.actor, fps.*, path.follow): authoring error nav_conflict",
        "requires": "system nav.steer (capability nav); events also need behavior.execute",
        "actions": "nav.go {id, point:[x,y,z] | entity:ID}, nav.stop {id}, nav.inspect {id}, nav.summary",
        "budget": format!("{MAX_NAV_AGENTS} agents, {MAX_WAYPOINTS} waypoints per path, {MAX_NAV_EVENTS_PER_STEP} events per step"),
        "checkpoint": "not supported: worlds scheduling nav.steer refuse checkpoints explicitly"
    })
}

pub fn settings_schema() -> Value {
    json!({
        "cell_size": "m 0.05..4, default 0.25",
        "bounds": format!("optional {{min:[x,z],max:[x,z]}}; default = obstacles + every placed entity + point targets, plus {AUTO_MARGIN} m"),
        "floor_y": format!("default 0: static colliders are obstacles when their box overlaps floor_y+{}..+{} m", OBSTACLE_BAND.0, OBSTACLE_BAND.1),
        "colliders": "bool, default true: static physics.collider shapes (entities no system moves) are obstacles; nav.blocker always is",
        "expansions_per_step": "64..262144, default 4096 (about 0.7 ms in release): A* nodes + line-of-sight cells per fixed step; further requests wait",
        "pointer_agent": "agent ID sent by a primary click on the ground, \"\" = none",
        "budget": format!("{MAX_NAV_CELLS} cells (sides <= {MAX_GRID_SIDE}), {MAX_NAV_OBSTACLES} obstacles"),
        "build": "built when the runtime is built (background on revision activation), cached by content; never per step"
    })
}

/// Placement of a navigation grid: `width` x `height` cells of `cell` meters from `origin`
/// (the min XZ corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
    pub origin: [f32; 2],
    pub cell: f32,
    pub width: u32,
    pub height: u32,
}

impl GridSpec {
    pub fn cells(&self) -> usize {
        self.width as usize * self.height as usize
    }
    pub fn cell_of(&self, p: Vec2) -> Option<[i32; 2]> {
        if !p.is_finite() {
            return None;
        }
        let c = [
            ((p.x - self.origin[0]) / self.cell).floor() as i32,
            ((p.y - self.origin[1]) / self.cell).floor() as i32,
        ];
        self.index(c).map(|_| c)
    }
    pub fn index(&self, c: [i32; 2]) -> Option<usize> {
        (c[0] >= 0 && c[1] >= 0 && c[0] < self.width as i32 && c[1] < self.height as i32)
            .then(|| c[1] as usize * self.width as usize + c[0] as usize)
    }
    pub fn center(&self, c: [i32; 2]) -> Vec2 {
        Vec2::new(
            self.origin[0] + (c[0] as f32 + 0.5) * self.cell,
            self.origin[1] + (c[1] as f32 + 0.5) * self.cell,
        )
    }
}

/// Everything the runtime needs to build the grid, derived from the compiled world.
#[derive(Clone, Debug, PartialEq)]
pub struct NavPlan {
    pub settings: NavSettings,
    pub spec: GridSpec,
    /// Obstacle rectangles (world XZ min/max), blockers first.
    pub rects: Vec<Rect2>,
    pub blockers: usize,
}

impl NavPlan {
    /// Exact cache key: the grid depends on nothing else.
    pub fn key(&self) -> Vec<u32> {
        let s = &self.spec;
        let mut key = vec![
            s.origin[0].to_bits(),
            s.origin[1].to_bits(),
            s.cell.to_bits(),
            s.width,
            s.height,
        ];
        for (min, max) in &self.rects {
            key.extend([min[0], min[1], max[0], max[1]].map(f32::to_bits));
        }
        key
    }
}

/// Authored world matrix of entity `slot` (parent chain of authored transforms).
pub fn authored_matrix(
    entities: &[CompiledEntity],
    parents: &[Option<usize>],
    slot: usize,
) -> Mat4 {
    let local = |i: usize| {
        entities[i]
            .transform
            .as_ref()
            .map_or(Mat4::IDENTITY, crate::Transform::matrix)
    };
    let mut m = local(slot);
    let mut parent = parents[slot];
    let mut depth = 0;
    while let Some(p) = parent {
        m = local(p) * m;
        parent = parents[p];
        depth += 1;
        if depth > entities.len() {
            break;
        }
    }
    m
}

/// True when a system moves entity `slot` itself or through an ancestor (a body under an
/// agent): `nav.agent`, `path.follow` or any `movers` component. Such colliders are not
/// static obstacles.
pub fn moved_by_system(
    entities: &[CompiledEntity],
    parents: &[Option<usize>],
    movers: &std::collections::BTreeMap<String, String>,
    slot: usize,
) -> bool {
    let moves = |e: &CompiledEntity| {
        e.nav_agent.is_some()
            || e.follow.is_some()
            || e.components.keys().any(|c| movers.contains_key(c))
    };
    let mut moved = moves(&entities[slot]);
    let mut cursor = parents[slot];
    let mut depth = 0;
    while let (Some(p), false) = (cursor, moved) {
        moved = moves(&entities[p]);
        cursor = parents[p];
        depth += 1;
        if depth > entities.len() {
            break;
        }
    }
    moved
}

/// World XZ footprint of a static collider when it overlaps the obstacle band.
fn collider_rect(collider: &crate::Collider, m: Mat4, floor_y: f32) -> Option<Rect2> {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut add = |p: Vec3, r: f32| {
        lo = lo.min(p - Vec3::splat(r));
        hi = hi.max(p + Vec3::splat(r));
    };
    match collider.shape {
        crate::ColliderShape::Box { half_extents } => {
            let h = Vec3::from_array(half_extents);
            for j in 0..8 {
                let s = |bit: usize| if j & bit == 0 { -1.0 } else { 1.0 };
                add(m.transform_point3(h * Vec3::new(s(1), s(2), s(4))), 0.0);
            }
        }
        crate::ColliderShape::Capsule {
            half_height,
            radius,
        } => {
            let (scale, _, _) = m.to_scale_rotation_translation();
            let r = radius * scale.abs().max_element();
            add(m.transform_point3(Vec3::Y * half_height), r);
            add(m.transform_point3(-Vec3::Y * half_height), r);
        }
        crate::ColliderShape::Sphere { radius } => {
            let (scale, _, _) = m.to_scale_rotation_translation();
            add(
                m.transform_point3(Vec3::ZERO),
                radius * scale.abs().max_element(),
            );
        }
    }
    let band = (floor_y + OBSTACLE_BAND.0, floor_y + OBSTACLE_BAND.1);
    (lo.is_finite() && hi.is_finite() && hi.y > band.0 && lo.y < band.1)
        .then_some(([lo.x, lo.z], [hi.x, hi.z]))
}

fn budget(message: String) -> anyhow::Error {
    crate::Diagnostic::error("nav_budget", "/resources/nav.grid", message)
}

/// The grid plan of a compiled world, or `None` when it has no `nav.agent`. `movers` lists
/// the components whose entities move (their colliders are not static obstacles).
pub fn plan(
    world: &crate::World,
    movers: &std::collections::BTreeMap<String, String>,
    entities: &[CompiledEntity],
    parents: &[Option<usize>],
) -> Result<Option<NavPlan>> {
    let agents: Vec<usize> = (0..entities.len())
        .filter(|&i| entities[i].nav_agent.is_some())
        .collect();
    if agents.is_empty() {
        return Ok(None);
    }
    let settings: NavSettings = match world.resources.get("nav.grid") {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| {
            crate::Diagnostic::error("invalid_nav", "/resources/nav.grid", format!("{e}"))
        })?,
        None => NavSettings::default(),
    };
    settings.validate().map_err(|e| {
        crate::Diagnostic::error("invalid_nav", "/resources/nav.grid", format!("{e:#}"))
    })?;
    let mut rects = crate::blocker::blockers(entities).map_err(|e| budget(format!("{e:#}")))?;
    let blockers = rects.len();
    if settings.colliders {
        for (i, e) in entities.iter().enumerate() {
            let Some(collider) = &e.collider else {
                continue;
            };
            if moved_by_system(entities, parents, movers, i) {
                continue;
            }
            if let Some(r) = collider_rect(
                collider,
                authored_matrix(entities, parents, i),
                settings.floor_y,
            ) {
                rects.push(r);
            }
        }
    }
    if rects.len() > MAX_NAV_OBSTACLES {
        return Err(budget(format!(
            "navigation obstacle budget exceeded ({} > {MAX_NAV_OBSTACLES} nav.blocker + static colliders)",
            rects.len()
        )));
    }
    let cell = settings.cell_size;
    let (min, max) = match settings.bounds {
        Some(b) => (Vec2::from(b.min), Vec2::from(b.max)),
        None => {
            let mut lo = Vec2::splat(f32::INFINITY);
            let mut hi = Vec2::splat(f32::NEG_INFINITY);
            for (a, b) in &rects {
                lo = lo.min(Vec2::from(*a));
                hi = hi.max(Vec2::from(*b));
            }
            // Every placed entity (agents, possible entity targets, props) and point targets.
            for (i, e) in entities.iter().enumerate() {
                if e.transform.is_none() {
                    continue;
                }
                let p = authored_matrix(entities, parents, i).transform_point3(Vec3::ZERO);
                lo = lo.min(Vec2::new(p.x, p.z));
                hi = hi.max(Vec2::new(p.x, p.z));
                if let Some(NavTarget::Point(t)) =
                    e.nav_agent.as_ref().and_then(|a| a.target.as_ref())
                {
                    lo = lo.min(Vec2::new(t[0], t[2]));
                    hi = hi.max(Vec2::new(t[0], t[2]));
                }
            }
            (lo - AUTO_MARGIN, hi + AUTO_MARGIN)
        }
    };
    // Snap the origin to whole cells so small edits keep the cell alignment (and the cache).
    let origin = (min / cell).floor() * cell;
    // The max edge belongs to the grid too (a target exactly on it is inside).
    let size = ((max - origin) / cell).floor() + 1.0;
    let (width, height) = (size.x.max(1.0), size.y.max(1.0));
    let cells = width as f64 * height as f64;
    if width > MAX_GRID_SIDE as f32 || height > MAX_GRID_SIDE as f32 || cells > MAX_NAV_CELLS as f64
    {
        return Err(budget(format!(
            "navigation grid {width}x{height} at {cell} m exceeds {MAX_NAV_CELLS} cells (sides <= {MAX_GRID_SIDE}); set nav.grid bounds to the walkable area or a larger cell_size"
        )));
    }
    Ok(Some(NavPlan {
        spec: GridSpec {
            origin: origin.to_array(),
            cell,
            width: width as u32,
            height: height as u32,
        },
        settings,
        rects,
        blockers,
    }))
}

/// World-level checks run by `World::compile`: required systems, mover conflicts, targets,
/// event contracts and the grid budget.
pub fn validate_world(
    world: &crate::World,
    registry: &crate::Registry,
    entities: &[CompiledEntity],
    indices: &std::collections::BTreeMap<String, usize>,
    parents: &[Option<usize>],
) -> Result<()> {
    let count = entities.iter().filter(|e| e.nav_agent.is_some()).count();
    if count == 0 {
        if let Some(v) = world.resources.get("nav.grid") {
            let s: NavSettings = serde_json::from_value(v.clone())?;
            if !s.pointer_agent.is_empty() {
                return Err(crate::Diagnostic::error(
                    "invalid_nav",
                    "/resources/nav.grid/pointer_agent",
                    format!("{} is not a nav.agent", s.pointer_agent),
                ));
            }
        }
        return Ok(());
    }
    if count > MAX_NAV_AGENTS {
        return Err(crate::Diagnostic::error(
            "nav_budget",
            "/entities",
            format!("nav agent budget exceeded ({count} > {MAX_NAV_AGENTS})"),
        ));
    }
    let at = |id: &str, field: &str| {
        format!(
            "/entities/@{}/components/nav.agent{field}",
            crate::local::escape(id)
        )
    };
    let has_system = |s: &str| world.systems.iter().any(|x| x == s);
    let program = world.resources.get("behavior.program");
    for (slot, e) in entities.iter().enumerate() {
        let Some(agent) = &e.nav_agent else { continue };
        let invalid = |field: &str, message: String| {
            crate::Diagnostic::error("invalid_nav", at(&e.id, field), message)
        };
        if !has_system("nav.steer") {
            return Err(invalid(
                "",
                "nav.agent needs the nav.steer system (add it to systems)".into(),
            ));
        }
        let other = e
            .components
            .keys()
            .filter(|c| c.as_str() != "nav.agent")
            .find_map(|c| {
                if c == "path.follow" {
                    Some((c.as_str(), "path.move"))
                } else {
                    registry.movers.get(c).map(|s| (c.as_str(), s.as_str()))
                }
            });
        if let Some((component, system)) = other {
            return Err(crate::Diagnostic::error(
                "nav_conflict",
                at(&e.id, ""),
                format!(
                    "{} has nav.agent and {component} (moved by {system}); an entity takes one mover. Put the agent on a parent or child entity instead",
                    e.id
                ),
            ));
        }
        if let Some(NavTarget::Entity(target)) = &agent.target {
            match indices.get(target) {
                None => {
                    return Err(invalid(
                        "/target/entity",
                        format!("target entity {target} does not exist"),
                    ))
                }
                Some(&t) if t == slot => {
                    return Err(invalid(
                        "/target/entity",
                        format!("{} cannot walk to itself", e.id),
                    ))
                }
                Some(&t) if entities[t].transform.is_none() => {
                    return Err(invalid(
                        "/target/entity",
                        format!("target entity {target} has no core.transform"),
                    ))
                }
                _ => {}
            }
        }
        for (field, event) in [
            ("/on_arrive", &agent.on_arrive),
            ("/on_fail", &agent.on_fail),
        ] {
            if event.is_empty() {
                continue;
            }
            if !has_system("behavior.execute") {
                return Err(invalid(
                    field,
                    "nav events need the behavior.execute system".into(),
                ));
            }
            let fields = program
                .and_then(|p| p.pointer(&format!("/events/{}/fields", crate::local::escape(event))))
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    invalid(
                        field,
                        format!("agent emits {event}, which behavior.program does not declare"),
                    )
                })?;
            if !fields
                .iter()
                .all(|(k, t)| matches!(k.as_str(), "agent" | "target" | "reason") && t == "text")
            {
                return Err(invalid(
                    field,
                    format!("nav event {event} may only declare text fields `agent`, `target` and `reason`"),
                ));
            }
        }
    }
    let plan = plan(world, &registry.movers, entities, parents)?.expect("agents exist");
    let pointer = &plan.settings.pointer_agent;
    if !pointer.is_empty()
        && !indices
            .get(pointer)
            .is_some_and(|&i| entities[i].nav_agent.is_some())
    {
        return Err(crate::Diagnostic::error(
            "invalid_nav",
            "/resources/nav.grid/pointer_agent",
            format!("{pointer} is not a nav.agent"),
        ));
    }
    Ok(())
}

/// Clearance raster: per cell, the distance in millimeters from the cell center to the nearest
/// obstacle (0 = the center is inside one), capped at `MAX_AGENT_RADIUS` + one cell.
pub struct NavGrid {
    pub spec: GridSpec,
    clearance: Vec<u16>,
    /// Cells whose center lies inside an obstacle.
    pub blocked: usize,
}

impl NavGrid {
    pub fn build(plan: &NavPlan) -> Self {
        let spec = plan.spec;
        let cap_m = MAX_AGENT_RADIUS + spec.cell;
        let cap = (cap_m * 1000.0) as u16;
        let mut clearance = vec![cap; spec.cells()];
        for (min, max) in &plan.rects {
            let lo = Vec2::from(*min) - cap_m;
            let hi = Vec2::from(*max) + cap_m;
            let c0 = ((lo - Vec2::from(spec.origin)) / spec.cell).floor();
            let c1 = ((hi - Vec2::from(spec.origin)) / spec.cell).floor();
            let x0 = c0.x.max(0.0) as i32;
            let z0 = c0.y.max(0.0) as i32;
            let x1 = c1.x.min(spec.width as f32 - 1.0) as i32;
            let z1 = c1.y.min(spec.height as f32 - 1.0) as i32;
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let p = spec.center([x, z]);
                    let q = p.clamp(Vec2::from(*min), Vec2::from(*max));
                    let d = (p - q).length();
                    let mm = if p == q {
                        0
                    } else {
                        (d * 1000.0).min(cap as f32) as u16
                    };
                    let i = z as usize * spec.width as usize + x as usize;
                    clearance[i] = clearance[i].min(mm);
                }
            }
        }
        let blocked = clearance.iter().filter(|&&c| c == 0).count();
        Self {
            spec,
            clearance,
            blocked,
        }
    }

    /// Minimum clearance (mm) of a walkable cell for an agent of `radius` meters. Permissive by
    /// half a cell, so a doorway wider than the agent's diameter always has a walkable cell
    /// (the agent itself is kept out of obstacles by the exact push-out at runtime).
    pub fn threshold(&self, radius: f32) -> u16 {
        (((radius - self.spec.cell * 0.5) * 1000.0).ceil() as i32).clamp(1, u16::MAX as i32) as u16
    }

    pub fn clearance(&self, index: usize) -> u16 {
        self.clearance[index]
    }

    pub fn walkable(&self, index: usize, threshold: u16) -> bool {
        self.clearance[index] >= threshold
    }

    pub fn walkable_cell(&self, c: [i32; 2], threshold: u16) -> bool {
        self.spec
            .index(c)
            .is_some_and(|i| self.clearance[i] >= threshold)
    }

    /// Nearest walkable cell to `p` within `SNAP_DISTANCE` (rings around its cell).
    pub fn nearest_walkable(&self, p: Vec2, threshold: u16) -> Option<[i32; 2]> {
        self.nearest_walkable_counted(p, threshold).0
    }

    /// `nearest_walkable` and the cells it examined (work units for the step budget).
    pub fn nearest_walkable_counted(&self, p: Vec2, threshold: u16) -> (Option<[i32; 2]>, usize) {
        let Some(c) = self.spec.cell_of(p) else {
            return (None, 1);
        };
        if self.walkable_cell(c, threshold) {
            return (Some(c), 1);
        }
        let examined = |r: i32| ((2 * r + 1) * (2 * r + 1)) as usize;
        let rings = (SNAP_DISTANCE / self.spec.cell).ceil() as i32;
        let mut best: Option<([i32; 2], f32)> = None;
        for r in 1..=rings {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let n = [c[0] + dx, c[1] + dz];
                    if self.walkable_cell(n, threshold) {
                        let d = self.spec.center(n).distance_squared(p);
                        if best.is_none_or(|(_, b)| d < b) {
                            best = Some((n, d));
                        }
                    }
                }
            }
            if let Some((n, d)) = best {
                // A farther ring can still hold a closer center than a corner of this one.
                if d.sqrt() <= r as f32 * self.spec.cell {
                    return (Some(n), examined(r));
                }
            }
        }
        (
            best.filter(|(_, d)| d.sqrt() <= SNAP_DISTANCE + self.spec.cell)
                .map(|(n, _)| n),
            examined(rings),
        )
    }

    /// Whether the segment `a`-`b` crosses only walkable cells (every cell it touches, including
    /// corner-touching pairs). Returns the verdict and the cells visited (work units).
    pub fn line_of_sight(&self, a: Vec2, b: Vec2, threshold: u16) -> (bool, usize) {
        let spec = &self.spec;
        let (Some(mut c), Some(end)) = (spec.cell_of(a), spec.cell_of(b)) else {
            return (false, 1);
        };
        let d = b - a;
        let step = [d.x.signum() as i32, d.y.signum() as i32];
        let origin = Vec2::from(spec.origin);
        let next_boundary =
            |p: f32, o: f32, c: i32, s: i32| o + (c + i32::from(s > 0)) as f32 * spec.cell - p;
        let mut t_max = Vec2::new(
            if step[0] != 0 {
                next_boundary(a.x, origin.x, c[0], step[0]) / d.x
            } else {
                f32::INFINITY
            },
            if step[1] != 0 {
                next_boundary(a.y, origin.y, c[1], step[1]) / d.y
            } else {
                f32::INFINITY
            },
        );
        let t_delta = Vec2::new(
            if step[0] != 0 {
                spec.cell / d.x.abs()
            } else {
                f32::INFINITY
            },
            if step[1] != 0 {
                spec.cell / d.y.abs()
            } else {
                f32::INFINITY
            },
        );
        let limit = (spec.width + spec.height) as usize + 2;
        let mut visited = 0;
        loop {
            visited += 1;
            if !self.walkable_cell(c, threshold) {
                return (false, visited);
            }
            if c == end || visited > limit {
                return (c == end, visited);
            }
            if (t_max.x - t_max.y).abs() < 1e-6 {
                // Through a cell corner: both side cells must be walkable too.
                visited += 2;
                if !self.walkable_cell([c[0] + step[0], c[1]], threshold)
                    || !self.walkable_cell([c[0], c[1] + step[1]], threshold)
                {
                    return (false, visited);
                }
                c = [c[0] + step[0], c[1] + step[1]];
                t_max += t_delta;
            } else if t_max.x < t_max.y {
                c[0] += step[0];
                t_max.x += t_delta.x;
            } else {
                c[1] += step[1];
                t_max.y += t_delta.y;
            }
        }
    }
}

/// A found cell path: from `start` through `cells` (the cell after the start's cell .. the
/// goal's cell) to `goal`.
#[derive(Clone, Copy)]
pub struct Leg<'a> {
    pub start: Vec2,
    pub cells: &'a [[i32; 2]],
    pub goal: Vec2,
}

/// Straightens a cell path into waypoints; the last waypoint is the goal itself. Corners of
/// the cell path are kept unless the previous waypoint sees past them. At most `max_work` line-of-sight cells are walked;
/// past that the remaining corners are kept as they are (still a valid path). Returns the
/// work spent, or `None` when more than `MAX_WAYPOINTS` waypoints remain.
pub fn straighten(
    grid: &NavGrid,
    leg: Leg,
    threshold: u16,
    max_work: usize,
    corners: &mut Vec<Vec2>,
    out: &mut Vec<Vec2>,
) -> Option<usize> {
    let Leg { start, cells, goal } = leg;
    corners.clear();
    out.clear();
    // Cell path corners: where the step direction changes, then the goal.
    for k in 0..cells.len().saturating_sub(1) {
        let prev = if k == 0 { None } else { Some(cells[k - 1]) };
        let d1 = prev.map(|p| [cells[k][0] - p[0], cells[k][1] - p[1]]);
        let d2 = [cells[k + 1][0] - cells[k][0], cells[k + 1][1] - cells[k][1]];
        if d1 != Some(d2) {
            corners.push(grid.spec.center(cells[k]));
        }
    }
    corners.push(goal);
    let mut work = 0;
    let mut anchor = start;
    let mut i = 0;
    while i < corners.len() {
        let mut last = i;
        if work < max_work {
            let mut k = i + 1;
            while k < corners.len() && work < max_work {
                let (ok, w) = grid.line_of_sight(anchor, corners[k], threshold);
                work += w;
                if !ok {
                    break;
                }
                last = k;
                k += 1;
            }
        }
        if out.len() == MAX_WAYPOINTS {
            return None;
        }
        out.push(corners[last]);
        anchor = corners[last];
        i = last + 1;
    }
    Some(work)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(v: Value) -> Result<NavAgent> {
        let a: NavAgent = serde_json::from_value(v)?;
        a.validate()?;
        Ok(a)
    }
    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        format!("{:#}", r.unwrap_err())
    }

    fn grid(rects: Vec<Rect2>, w: u32, h: u32, cell: f32) -> NavGrid {
        NavGrid::build(&NavPlan {
            settings: NavSettings::default(),
            spec: GridSpec {
                origin: [0.0, 0.0],
                cell,
                width: w,
                height: h,
            },
            blockers: rects.len(),
            rects,
        })
    }

    #[test]
    fn defaults_round_trip_and_validation() {
        let a = agent(json!({})).unwrap();
        assert_eq!(a, NavAgent::new());
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            r#"{"speed":1.4,"acceleration":8.0,"turn_rate":720.0,"radius":0.3,"arrive_distance":0.2,"follow_target":false,"avoidance":true,"on_arrive":"","on_fail":"","enabled":true}"#
        );
        let t = agent(json!({"target":{"point":[1,0,2]}})).unwrap();
        assert_eq!(t.target, Some(NavTarget::Point([1.0, 0.0, 2.0])));
        let e = agent(json!({"target":{"entity":"door"},"follow_target":true})).unwrap();
        assert_eq!(e.target, Some(NavTarget::Entity("door".into())));
        assert!(err(agent(json!({"speed":-1}))).contains("speed"));
        assert!(err(agent(json!({"radius":3}))).contains("radius"));
        assert!(err(agent(json!({"arrive_distance":0}))).contains("arrive_distance"));
        assert!(err(agent(json!({"target":{"point":[1,0]}}))).contains("invalid length"));
        assert!(err(agent(json!({"target":{"entity":"a b"}}))).contains("target entity"));
        assert!(err(agent(
            json!({"target":{"point":[1,0,0]},"follow_target":true})
        ))
        .contains("entity targets only"));
        assert!(err(agent(json!({"on_fail":"x y"}))).contains("on_fail"));
        assert!(err(agent(json!({"sped":1}))).contains("unknown field"));
        let s: NavSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(s, NavSettings::default());
        s.validate().unwrap();
        let bad: NavSettings = serde_json::from_value(json!({"cell_size":0.01})).unwrap();
        assert!(err(bad.validate()).contains("cell_size"));
        let bad: NavSettings =
            serde_json::from_value(json!({"bounds":{"min":[0,0],"max":[0,5]}})).unwrap();
        assert!(err(bad.validate()).contains("bounds"));
        let bad: NavSettings = serde_json::from_value(json!({"expansions_per_step":1})).unwrap();
        assert!(err(bad.validate()).contains("expansions_per_step"));
    }

    #[test]
    fn clearance_threshold_and_doorways() {
        // Two walls along z = 2..2.2 with a 0.7 m gap between x = 2.0 and 2.7.
        let g = grid(
            vec![([0.0, 2.0], [2.0, 2.2]), ([2.7, 2.0], [5.0, 2.2])],
            20,
            20,
            0.25,
        );
        assert!(g.blocked > 0);
        // Inside a wall: clearance 0. Far away: capped.
        let i = g
            .spec
            .index(g.spec.cell_of(Vec2::new(1.0, 2.1)).unwrap())
            .unwrap();
        assert_eq!(g.clearance(i), 0);
        let far = g.spec.index([0, 19]).unwrap();
        assert!(g.clearance(far) >= 2000);
        // A 0.3 m agent (0.6 m diameter < 0.7 m gap) finds a walkable cell in the doorway and
        // a straight line through it; a 0.5 m agent does not fit.
        let small = g.threshold(0.3);
        let big = g.threshold(0.5);
        let doorway: Vec<[i32; 2]> = (0..20).map(|x| [x, 8]).collect();
        assert!(doorway.iter().any(|c| g.walkable_cell(*c, small)));
        assert!(!doorway.iter().any(|c| g.walkable_cell(*c, big)));
        let (ok, work) = g.line_of_sight(Vec2::new(2.35, 1.0), Vec2::new(2.35, 3.5), small);
        assert!(ok && work >= 10);
        assert!(
            !g.line_of_sight(Vec2::new(1.0, 1.0), Vec2::new(1.0, 3.5), small)
                .0
        );
        // A diagonal that only touches a blocked corner is refused.
        let corner = grid(vec![([1.0, 1.0], [2.0, 2.0])], 8, 8, 1.0);
        let t = corner.threshold(0.05);
        assert!(
            !corner
                .line_of_sight(Vec2::new(0.5, 1.5), Vec2::new(1.5, 2.5), t)
                .0
        );
        assert!(
            corner
                .line_of_sight(Vec2::new(0.5, 2.5), Vec2::new(1.5, 3.5), t)
                .0
        );
        assert!(
            corner
                .line_of_sight(Vec2::new(0.5, 0.5), Vec2::new(0.5, 6.5), t)
                .0
        );
        // Snapping a blocked goal to the nearest walkable cell.
        let snapped = corner.nearest_walkable(Vec2::new(1.4, 1.5), t).unwrap();
        assert_eq!(snapped, [0, 1]);
        assert!(corner.nearest_walkable(Vec2::new(-5.0, 0.0), t).is_none());
    }

    #[test]
    fn straightening_keeps_only_needed_corners() {
        let g = grid(vec![([4.0, 0.0], [5.0, 6.0])], 10, 10, 1.0);
        let t = g.threshold(0.2);
        // An L-shaped cell path around the wall's top end.
        let mut cells = vec![];
        for z in 1..=7 {
            cells.push([2, z]);
        }
        for x in 3..=7 {
            cells.push([x, 7]);
        }
        for z in (1..=6).rev() {
            cells.push([7, z]);
        }
        let (mut corners, mut out) = (vec![], vec![]);
        let leg = Leg {
            start: Vec2::new(2.5, 0.5),
            cells: &cells,
            goal: Vec2::new(7.5, 1.5),
        };
        let work = straighten(&g, leg, t, usize::MAX, &mut corners, &mut out).unwrap();
        assert!(work > 0);
        assert_eq!(*out.last().unwrap(), Vec2::new(7.5, 1.5));
        assert!(out.len() <= 3, "{out:?}");
        // Each leg is clear.
        let mut from = Vec2::new(2.5, 0.5);
        for p in &out {
            assert!(g.line_of_sight(from, *p, t).0, "{from} -> {p}");
            from = *p;
        }
        // With no work allowed the corners stay as they are.
        straighten(&g, leg, t, 0, &mut corners, &mut out).unwrap();
        assert_eq!(out.len(), corners.len());
    }
}
