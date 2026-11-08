//! `nav.steer`: runtime side of `nav.agent` (see `axiom_core::navigation`).
//!
//! The clearance grid is built once per runtime build from the static obstacles and shared
//! through a process-wide cache keyed by the exact grid inputs, so revisions that do not move
//! obstacles reuse it. Path requests wait in a FIFO queue; each fixed step spends at most
//! `nav.grid.expansions_per_step` units of work (A* node expansions plus line-of-sight cells)
//! on them, resuming an unfinished search on the next step, so a long query never spikes one
//! frame. Found cell paths are straightened into at most `MAX_WAYPOINTS` waypoints.
//!
//! Steering: accelerate toward the next waypoint, brake to stop at the goal, keep apart from
//! other agents (simple pairwise separation through a bucket grid), then push the body out of
//! obstacle rectangles exactly (`axiom_core::blocker::push_out`). Agents that make no
//! progress re-plan a few times, then fail with `stuck`. A steady step does not allocate.
use crate::trigger::world_matrix;
use crate::world::*;
use anyhow::{bail, ensure, Context, Result};
use axiom_core::{
    blocker::{push_out, Rect2},
    gameplay::{RuntimeCommand, Scalar},
    grid_search::{GridSearch, Neighbourhood, Progress},
    navigation::*,
};
use glam::{Quat, Vec2, Vec3};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
    f32::consts::{PI, TAU},
    sync::{Arc, Mutex},
};

/// Agents listed with state and position in `observe` (counts are always complete).
const OBSERVED: usize = 32;
const OBSERVED_FAILURES: usize = 16;
/// Ticks of no progress before a re-plan, and re-plans before failing with `stuck`.
const STUCK_TICKS: u16 = 60;
const MAX_REPATHS: u8 = 3;
/// Line-of-sight work a path may always spend on straightening, even when the step's budget
/// is spent (bounds the overshoot of one step).
const MIN_STRAIGHTEN_WORK: usize = 1024;
/// Separation bucket grid side (buckets grow to keep at most this many per side).
const MAX_BUCKET_SIDE: u32 = 128;
/// Synchronous node budget of an editor path preview.
const PREVIEW_EXPANSIONS: usize = 65_536;
const GRID_CACHE_ENTRIES: usize = 8;

static GRID_CACHE: Mutex<BTreeMap<Vec<u32>, Arc<NavGrid>>> = Mutex::new(BTreeMap::new());

/// The grid for `plan`, built or taken from the cache; `true` when cached.
fn cached_grid(plan: &NavPlan, key: &[u32]) -> (Arc<NavGrid>, bool) {
    if let Some(grid) = GRID_CACHE.lock().ok().and_then(|c| c.get(key).cloned()) {
        return (grid, true);
    }
    let grid = Arc::new(NavGrid::build(plan));
    if let Ok(mut cache) = GRID_CACHE.lock() {
        if cache.len() >= GRID_CACHE_ENTRIES {
            cache.retain(|_, g| Arc::strong_count(g) > 1);
        }
        if cache.len() < GRID_CACHE_ENTRIES {
            cache.insert(key.to_vec(), grid.clone());
        }
    }
    (grid, false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    /// Waiting for a path (queued or being searched); may still walk an older path.
    Pending,
    Moving,
    Arrived,
    Failed,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Pending => "pending",
            Self::Moving => "moving",
            Self::Arrived => "arrived",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Goal {
    Point(Vec2),
    /// Target entity slot and ID (re-resolved by ID on hot reload).
    Entity(usize, String),
}

struct Agent {
    slot: usize,
    speed: f32,
    acceleration: f32,
    /// Radians per fixed step (0 = face the walking direction at once).
    turn: f32,
    radius: f32,
    arrive: f32,
    threshold: u16,
    avoidance: bool,
    follow_target: bool,
    enabled: bool,
    on_arrive: Option<usize>,
    on_fail: Option<usize>,
    authored_target: Option<NavTarget>,
    goal: Option<Goal>,
    state: State,
    reason: &'static str,
    waypoints: Vec<Vec2>,
    next: usize,
    /// Goal position the current path was planned for (entity targets re-plan when it moves).
    planned_for: Vec2,
    velocity: Vec2,
    yaw: Option<f32>,
    queued: bool,
    /// The goal was inside an obstacle and moved to the nearest walkable cell: arrival is
    /// judged against that point, not the target itself.
    goal_snapped: bool,
    stuck: u16,
    repaths: u8,
}

struct Active {
    agent: usize,
    start: Vec2,
    goal: Vec2,
    /// The start was inside an obstacle's clearance: walk to this cell center first.
    snapped_start: Option<Vec2>,
    goal_snapped: bool,
}

struct EventInfo {
    name: String,
    agent_field: bool,
    target_field: bool,
    reason_field: bool,
}

/// Bucket grid over the navigation area (separation and obstacle lookup).
struct Buckets {
    origin: Vec2,
    size: f32,
    side: [u32; 2],
    /// Static obstacle rectangles near each bucket: `rect_items[rect_start[b]..rect_start[b+1]]`.
    rect_start: Vec<u32>,
    rect_items: Vec<u32>,
    /// Agents per bucket this step (counting sort into reused arrays).
    agent_start: Vec<u32>,
    agent_cursor: Vec<u32>,
    agent_items: Vec<u32>,
}

impl Buckets {
    fn new(spec: &GridSpec, rects: &[Rect2], max_radius: f32, agents: usize) -> Self {
        let extent = Vec2::new(spec.width as f32, spec.height as f32) * spec.cell;
        let mut size = (2.0 * max_radius + 0.25).max(1.0);
        size = size.max(extent.max_element() / MAX_BUCKET_SIDE as f32);
        let side = [
            ((extent.x / size).ceil() as u32).clamp(1, MAX_BUCKET_SIDE),
            ((extent.y / size).ceil() as u32).clamp(1, MAX_BUCKET_SIDE),
        ];
        let mut b = Self {
            origin: Vec2::from(spec.origin),
            size,
            side,
            rect_start: vec![],
            rect_items: vec![],
            agent_start: vec![0; (side[0] * side[1]) as usize + 1],
            agent_cursor: vec![0; (side[0] * side[1]) as usize],
            agent_items: vec![0; agents],
        };
        let mut lists: Vec<Vec<u32>> = vec![vec![]; (side[0] * side[1]) as usize];
        for (k, (min, max)) in rects.iter().enumerate() {
            let lo = b.coords(Vec2::from(*min) - max_radius);
            let hi = b.coords(Vec2::from(*max) + max_radius);
            for z in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    lists[(z * side[0] + x) as usize].push(k as u32);
                }
            }
        }
        b.rect_start.push(0);
        for list in &lists {
            b.rect_items.extend_from_slice(list);
            b.rect_start.push(b.rect_items.len() as u32);
        }
        b
    }

    fn coords(&self, p: Vec2) -> [u32; 2] {
        let c = ((p - self.origin) / self.size).floor();
        [
            (c.x.max(0.0) as u32).min(self.side[0] - 1),
            (c.y.max(0.0) as u32).min(self.side[1] - 1),
        ]
    }

    fn bucket(&self, p: Vec2) -> usize {
        let [x, z] = self.coords(p);
        (z * self.side[0] + x) as usize
    }

    fn rects(&self, p: Vec2) -> &[u32] {
        let b = self.bucket(p);
        &self.rect_items[self.rect_start[b] as usize..self.rect_start[b + 1] as usize]
    }

    /// Sorts agent indices into buckets by position (no allocation).
    fn sort_agents(&mut self, positions: &[Vec2]) {
        self.agent_cursor.fill(0);
        for p in positions {
            let b = self.bucket(*p);
            self.agent_cursor[b] += 1;
        }
        let mut sum = 0;
        for (b, count) in self.agent_cursor.iter_mut().enumerate() {
            self.agent_start[b] = sum;
            sum += *count;
            *count = self.agent_start[b];
        }
        let n = self.agent_cursor.len();
        self.agent_start[n] = sum;
        for (i, p) in positions.iter().enumerate() {
            let b = self.bucket(*p);
            self.agent_items[self.agent_cursor[b] as usize] = i as u32;
            self.agent_cursor[b] += 1;
        }
    }
}

/// Agent slot, start and goal cells, threshold and the exact goal point (bits).
type PreviewKey = (usize, [i32; 2], [i32; 2], u16, [u32; 2]);

/// Editor path preview, cached by its inputs.
#[derive(Default)]
struct Preview {
    search: Option<GridSearch>,
    key: Option<PreviewKey>,
    path: Option<Vec<Vec2>>,
    cells: Vec<[i32; 2]>,
    corners: Vec<Vec2>,
}

pub struct NavSystem {
    grid: Option<Arc<NavGrid>>,
    key: Vec<u32>,
    rects: Vec<Rect2>,
    obstacles: usize,
    blockers: usize,
    grid_cached: bool,
    build_ms: f64,
    settings: NavSettings,
    search: GridSearch,
    raw: Vec<[i32; 2]>,
    corners: Vec<Vec2>,
    agents: Vec<Agent>,
    ids: Vec<String>,
    /// World XZ per agent this step.
    positions: Vec<Vec2>,
    buckets: Option<Buckets>,
    queue: VecDeque<usize>,
    active: Option<Active>,
    events: Vec<EventInfo>,
    /// (agent, arrived?) events of this step (capacity = event budget).
    fired: Vec<(usize, bool)>,
    used: usize,
    used_max: usize,
    deferred_steps: u64,
    searches: u64,
    emitted: u64,
    pointer_agent: Option<usize>,
    scanned: usize,
    halted: Option<String>,
    fault: Option<String>,
    preview: Mutex<Preview>,
}

/// Approaches `to` from `from` (radians) by at most `max`, the short way round.
fn approach(from: f32, to: f32, max: f32) -> f32 {
    let delta = (to - from + PI).rem_euclid(TAU) - PI;
    (from + delta.clamp(-max, max) + PI).rem_euclid(TAU) - PI
}

fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// World position of entity `slot` (pivot).
fn world_position(state: &RuntimeState, slot: usize) -> Vec3 {
    let p = state.transforms[slot]
        .as_ref()
        .map_or(Vec3::ZERO, |t| Vec3::from_array(t.position));
    match state.compiled.parents[slot] {
        None => p,
        Some(parent) => world_matrix(state, parent).transform_point3(p),
    }
}

fn round(v: f32, scale: f64) -> f64 {
    (f64::from(v) * scale).round() / scale
}

impl NavSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let started = std::time::Instant::now();
        let compiled = &state.compiled;
        let plan = plan(
            &compiled.source,
            &state.registry.movers,
            &compiled.entities,
            &compiled.parents,
        )?;
        let mut system = Self {
            grid: None,
            key: vec![],
            rects: vec![],
            obstacles: 0,
            blockers: 0,
            grid_cached: false,
            build_ms: 0.0,
            settings: NavSettings::default(),
            search: GridSearch::new(0, 0),
            raw: vec![],
            corners: vec![],
            agents: vec![],
            ids: vec![],
            positions: vec![],
            buckets: None,
            queue: VecDeque::new(),
            active: None,
            events: vec![],
            fired: Vec::with_capacity(MAX_NAV_EVENTS_PER_STEP),
            used: 0,
            used_max: 0,
            deferred_steps: 0,
            searches: 0,
            emitted: 0,
            pointer_agent: None,
            scanned: compiled.len(),
            halted: None,
            fault: None,
            preview: Mutex::new(Preview::default()),
        };
        let Some(plan) = plan else {
            return Ok(system);
        };
        let key = plan.key();
        let (grid, cached) = cached_grid(&plan, &key);
        let program = compiled.source.resources.get("behavior.program");
        let mut order = vec![0; compiled.len()];
        for (rank, &slot) in compiled.transform_order.iter().enumerate() {
            order[slot] = rank;
        }
        let mut slots: Vec<usize> = (0..compiled.len())
            .filter(|&i| compiled.entities[i].nav_agent.is_some())
            .collect();
        slots.sort_by_key(|&i| order[i]);
        ensure!(
            slots.len() <= MAX_NAV_AGENTS,
            "nav agent budget exceeded ({} > {MAX_NAV_AGENTS})",
            slots.len()
        );
        let mut event = |name: &str| -> Result<Option<usize>> {
            if name.is_empty() {
                return Ok(None);
            }
            if let Some(i) = system.events.iter().position(|v| v.name == name) {
                return Ok(Some(i));
            }
            let fields = program
                .and_then(|p| {
                    p.pointer(&format!(
                        "/events/{}/fields",
                        axiom_core::local::escape(name)
                    ))
                })
                .and_then(Value::as_object)
                .with_context(|| {
                    format!("nav agent emits {name}, which behavior.program does not declare")
                })?;
            system.events.push(EventInfo {
                name: name.to_owned(),
                agent_field: fields.contains_key("agent"),
                target_field: fields.contains_key("target"),
                reason_field: fields.contains_key("reason"),
            });
            Ok(Some(system.events.len() - 1))
        };
        let mut agents = vec![];
        let mut ids = vec![];
        for &slot in &slots {
            let e = &compiled.entities[slot];
            let def = e.nav_agent.as_ref().unwrap();
            def.validate()
                .with_context(|| format!("nav agent {}", e.id))?;
            ensure!(
                state.transforms[slot].is_some(),
                "nav agent {} needs core.transform",
                e.id
            );
            agents.push(Agent {
                slot,
                speed: def.speed,
                acceleration: def.acceleration,
                turn: def.turn_rate.to_radians() * FIXED_DT,
                radius: def.radius,
                arrive: def.arrive_distance,
                threshold: grid.threshold(def.radius),
                avoidance: def.avoidance,
                follow_target: def.follow_target,
                enabled: def.enabled,
                on_arrive: event(&def.on_arrive)?,
                on_fail: event(&def.on_fail)?,
                authored_target: def.target.clone(),
                goal: None,
                state: State::Idle,
                reason: "",
                waypoints: Vec::with_capacity(MAX_WAYPOINTS),
                next: 0,
                planned_for: Vec2::ZERO,
                velocity: Vec2::ZERO,
                yaw: None,
                queued: false,
                goal_snapped: false,
                stuck: 0,
                repaths: 0,
            });
            ids.push(e.id.clone());
        }
        if !system.events.is_empty() {
            ensure!(
                compiled
                    .source
                    .systems
                    .iter()
                    .any(|s| s == "behavior.execute"),
                "nav events need the behavior.execute system"
            );
        }
        system.agents = agents;
        system.ids = ids;
        for a in 0..system.agents.len() {
            if let Some(target) = system.agents[a].authored_target.clone() {
                let goal = system.resolve(state, &target)?;
                system.set_goal(a, goal);
            }
        }
        let max_radius = system
            .agents
            .iter()
            .map(|a| a.radius)
            .fold(0.0f32, f32::max);
        system.buckets = Some(Buckets::new(
            &grid.spec,
            &plan.rects,
            max_radius,
            system.agents.len(),
        ));
        system.search = GridSearch::new(grid.spec.width, grid.spec.height);
        // Where the agents stand before the first step (inspect / observe at tick 0).
        system.positions = system
            .agents
            .iter()
            .map(|a| xz(world_position(state, a.slot)))
            .collect();
        system.queue.reserve(system.agents.len());
        system.pointer_agent = system
            .ids
            .iter()
            .position(|id| *id == plan.settings.pointer_agent);
        system.obstacles = plan.rects.len();
        system.blockers = plan.blockers;
        system.rects = plan.rects;
        system.settings = plan.settings;
        system.key = key;
        system.grid = Some(grid);
        system.grid_cached = cached;
        system.build_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(system)
    }

    /// The navigation grid (None when the world has no agents).
    pub fn grid(&self) -> Option<&NavGrid> {
        self.grid.as_deref()
    }

    /// Walkable-cell threshold of the agent on entity slot `slot` (editor overlays).
    pub fn threshold(&self, slot: usize) -> Option<u16> {
        self.agents
            .iter()
            .find(|a| a.slot == slot)
            .map(|a| a.threshold)
    }

    fn resolve(&self, state: &RuntimeState, target: &NavTarget) -> Result<Goal> {
        Ok(match target {
            NavTarget::Point(p) => Goal::Point(Vec2::new(p[0], p[2])),
            NavTarget::Entity(id) => {
                let &slot = state.compiled.indices.get(id).ok_or_else(|| {
                    axiom_core::Diagnostic::error(
                        "not_found",
                        "/args/entity",
                        format!("target entity {id} does not exist"),
                    )
                })?;
                ensure!(
                    state.transforms[slot].is_some(),
                    "target entity {id} has no core.transform"
                );
                Goal::Entity(slot, id.clone())
            }
        })
    }

    fn goal_position(state: &RuntimeState, goal: &Goal) -> Vec2 {
        match goal {
            Goal::Point(p) => *p,
            Goal::Entity(slot, _) => xz(world_position(state, *slot)),
        }
    }

    /// Sets a new goal and queues a path request (cancelling one in progress for the agent).
    fn set_goal(&mut self, a: usize, goal: Goal) {
        if self.active.as_ref().is_some_and(|x| x.agent == a) {
            self.active = None;
        }
        let agent = &mut self.agents[a];
        agent.goal = Some(goal);
        agent.state = State::Pending;
        agent.reason = "";
        agent.waypoints.clear();
        agent.next = 0;
        agent.stuck = 0;
        agent.repaths = 0;
        self.enqueue(a);
    }

    fn enqueue(&mut self, a: usize) {
        let agent = &mut self.agents[a];
        if agent.state != State::Moving || agent.waypoints.is_empty() {
            agent.state = State::Pending;
        }
        if !agent.queued && !self.active.as_ref().is_some_and(|x| x.agent == a) {
            agent.queued = true;
            self.queue.push_back(a);
        }
    }

    /// Drops the agent's queued or running path request (a stale result must not reopen an
    /// arrived, failed or stopped agent).
    fn cancel(&mut self, a: usize) {
        if self.active.as_ref().is_some_and(|x| x.agent == a) {
            self.active = None;
        }
        self.agents[a].queued = false;
    }

    fn stop(&mut self, a: usize) {
        self.cancel(a);
        let agent = &mut self.agents[a];
        agent.goal = None;
        agent.state = State::Idle;
        agent.reason = "";
        agent.waypoints.clear();
        agent.queued = false;
    }

    fn fail(&mut self, a: usize, reason: &'static str) {
        self.cancel(a);
        let agent = &mut self.agents[a];
        agent.state = State::Failed;
        agent.reason = reason;
        agent.waypoints.clear();
        agent.queued = false;
        if agent.on_fail.is_some() {
            self.fire(a, false);
        }
    }

    fn fire(&mut self, a: usize, arrived: bool) {
        if self.fired.len() < MAX_NAV_EVENTS_PER_STEP {
            self.fired.push((a, arrived));
        } else {
            self.halted.get_or_insert_with(|| {
                format!("nav event budget exceeded (> {MAX_NAV_EVENTS_PER_STEP} on_arrive/on_fail events in one step)")
            });
        }
    }

    /// Starts the next queued request that needs a search; false when the queue is empty or
    /// the budget is spent (requests answered by a straight line finish here).
    fn begin_next(&mut self, budget: usize) -> bool {
        let grid = self.grid.clone().unwrap();
        loop {
            if self.used >= budget {
                return false;
            }
            let Some(a) = self.queue.pop_front() else {
                return false;
            };
            let agent = &mut self.agents[a];
            if !agent.queued {
                continue;
            }
            agent.queued = false;
            let Some(goal) = &agent.goal else { continue };
            let goal = match goal {
                Goal::Point(p) => *p,
                Goal::Entity(..) => agent.planned_for,
            };
            let start = self.positions[a];
            let threshold = agent.threshold;
            self.searches += 1;
            let spec = &grid.spec;
            let (Some(start_cell), Some(goal_cell)) = (spec.cell_of(start), spec.cell_of(goal))
            else {
                self.fail(a, "outside_grid");
                continue;
            };
            let (from, work) = grid.nearest_walkable_counted(start, threshold);
            self.used += work;
            let Some(from) = from else {
                self.fail(a, "start_blocked");
                continue;
            };
            let (to, work) = grid.nearest_walkable_counted(goal, threshold);
            self.used += work;
            let Some(to) = to else {
                self.fail(a, "goal_blocked");
                continue;
            };
            let snapped_start = (from != start_cell).then(|| spec.center(from));
            let goal_point = if to == goal_cell {
                goal
            } else {
                spec.center(to)
            };
            let begin = snapped_start.unwrap_or(start);
            self.agents[a].goal_snapped = to != goal_cell;
            let (clear, work) = grid.line_of_sight(begin, goal_point, threshold);
            self.used += work;
            if clear {
                let agent = &mut self.agents[a];
                agent.waypoints.clear();
                agent.waypoints.extend(snapped_start);
                agent.waypoints.push(goal_point);
                agent.next = 0;
                agent.state = State::Moving;
                continue;
            }
            self.search.begin(from, to, Neighbourhood::Eight);
            self.active = Some(Active {
                agent: a,
                start: begin,
                goal: goal_point,
                snapped_start,
                goal_snapped: to != goal_cell,
            });
            return true;
        }
    }

    /// Spends this step's pathfinding budget on queued requests.
    fn service(&mut self) {
        let budget = self.settings.expansions_per_step as usize;
        while self.used < budget {
            if self.active.is_none() && !self.begin_next(budget) {
                break;
            }
            let Some(active) = &self.active else { continue };
            let a = active.agent;
            let grid = self.grid.clone().unwrap();
            let threshold = self.agents[a].threshold;
            let before = self.search.expanded();
            let progress = self.search.run(budget.saturating_sub(self.used), |i| {
                grid.walkable(i, threshold)
            });
            self.used += self.search.expanded() - before;
            match progress {
                Progress::Pending => break,
                Progress::Unreachable => {
                    self.active = None;
                    self.fail(a, "unreachable");
                }
                Progress::Found => {
                    let active = self.active.take().unwrap();
                    self.search.path(&mut self.raw);
                    let leg = Leg {
                        start: active.start,
                        cells: &self.raw,
                        goal: active.goal,
                    };
                    let work = budget.saturating_sub(self.used).max(MIN_STRAIGHTEN_WORK);
                    let agent = &mut self.agents[a];
                    let spent = straighten(
                        &grid,
                        leg,
                        threshold,
                        work,
                        &mut self.corners,
                        &mut agent.waypoints,
                    );
                    let room =
                        active.snapped_start.is_none() || agent.waypoints.len() < MAX_WAYPOINTS;
                    match spent {
                        Some(spent) if room => {
                            self.used += spent;
                            if let Some(s) = active.snapped_start {
                                agent.waypoints.insert(0, s);
                            }
                            agent.next = 0;
                            agent.goal_snapped = active.goal_snapped;
                            agent.state = State::Moving;
                        }
                        _ => self.fail(a, "path_too_long"),
                    }
                }
            }
        }
        if self.active.is_some() || self.queue.iter().any(|&a| self.agents[a].queued) {
            self.deferred_steps += 1;
        }
    }

    /// Entity targets: re-plan when the target moved away from the planned goal; resume
    /// following after arrival when `follow_target` is set.
    fn track(&mut self, state: &RuntimeState) {
        let cell = self.grid.as_ref().map_or(0.25, |g| g.spec.cell);
        let repath = (4.0 * cell).max(1.0);
        for a in 0..self.agents.len() {
            let agent = &self.agents[a];
            let Some(goal) = &agent.goal else { continue };
            if !agent.enabled || !state.compiled.enabled[agent.slot] {
                continue;
            }
            let target = Self::goal_position(state, goal);
            let is_entity = matches!(goal, Goal::Entity(..));
            let (current, queued, follow, arrive, planned) = (
                agent.state,
                agent.queued,
                agent.follow_target,
                agent.arrive,
                agent.planned_for,
            );
            match current {
                State::Pending if queued => {
                    // Not started yet: plan for where the target is now.
                    self.agents[a].planned_for = target;
                }
                State::Moving if is_entity && target.distance(planned) > repath => {
                    // A search still running for the old position is restarted.
                    self.cancel(a);
                    self.agents[a].planned_for = target;
                    self.enqueue(a);
                }
                State::Arrived
                    if is_entity && follow && target.distance(self.positions[a]) > arrive + 0.5 =>
                {
                    self.agents[a].planned_for = target;
                    self.agents[a].repaths = 0;
                    self.enqueue(a);
                }
                _ => {}
            }
        }
    }

    fn steer(&mut self, state: &mut RuntimeState) {
        let Some(buckets) = self.buckets.as_mut() else {
            return;
        };
        buckets.sort_agents(&self.positions);
        for a in 0..self.agents.len() {
            let slot = self.agents[a].slot;
            if !self.agents[a].enabled || !state.compiled.enabled[slot] {
                continue;
            }
            let here = self.positions[a];
            let goal_entity = match &self.agents[a].goal {
                Some(Goal::Entity(s, _)) => Some(xz(world_position(state, *s))),
                _ => None,
            };
            let agent = &mut self.agents[a];
            let mut desired = Vec2::ZERO;
            let walking = matches!(agent.state, State::Moving | State::Pending)
                && agent.next < agent.waypoints.len();
            let mut arrived = false;
            if walking {
                // Pass reached corners (at most a couple per step).
                let corner = (agent.radius * 0.75).clamp(0.1, 0.6);
                while agent.next + 1 < agent.waypoints.len()
                    && agent.waypoints[agent.next].distance(here) <= corner
                {
                    agent.next += 1;
                }
                let last = agent.next + 1 == agent.waypoints.len();
                // The last leg to an entity heads for where it is now (the planned goal is
                // stale once it moves; a large move re-plans), unless the goal was snapped.
                let wp = match goal_entity {
                    Some(g) if last && !agent.goal_snapped => g,
                    _ => agent.waypoints[agent.next],
                };
                let to = wp - here;
                let distance = to.length();
                let remaining = if last { distance } else { f32::INFINITY };
                if last && remaining <= agent.arrive && agent.state == State::Moving {
                    arrived = true;
                } else if distance > 1e-5 {
                    let mut speed = agent.speed;
                    if last {
                        let brake =
                            (2.0 * agent.acceleration * (remaining - agent.arrive * 0.5).max(0.0))
                                .sqrt();
                        speed = speed.min(brake);
                    }
                    desired = to / distance * speed;
                }
            }
            if arrived {
                agent.state = State::Arrived;
                agent.waypoints.clear();
                agent.next = 0;
                agent.repaths = 0;
                agent.stuck = 0;
                let event = agent.on_arrive.is_some();
                self.cancel(a);
                if event {
                    self.fire(a, true);
                }
            }
            let agent = &mut self.agents[a];
            let dv = desired - agent.velocity;
            agent.velocity += dv.clamp_length_max(agent.acceleration * FIXED_DT);
            if agent.velocity.length_squared() < 1e-8 && desired == Vec2::ZERO {
                agent.velocity = Vec2::ZERO;
            }
            let intended = agent.velocity * FIXED_DT;
            let mut next = here + intended;
            let r = agent.radius;
            if agent.avoidance {
                let buckets = self.buckets.as_ref().unwrap();
                let [bx, bz] = buckets.coords(next);
                for z in bz.saturating_sub(1)..=(bz + 1).min(buckets.side[1] - 1) {
                    for x in bx.saturating_sub(1)..=(bx + 1).min(buckets.side[0] - 1) {
                        let b = (z * buckets.side[0] + x) as usize;
                        let items = &buckets.agent_items
                            [buckets.agent_start[b] as usize..buckets.agent_start[b + 1] as usize];
                        for &o in items {
                            let o = o as usize;
                            let other = &self.agents[o];
                            if o == a
                                || !other.avoidance
                                || !other.enabled
                                || !state.compiled.enabled[other.slot]
                            {
                                continue;
                            }
                            let d = next - self.positions[o];
                            let len = d.length();
                            let min = r + other.radius;
                            if len < min {
                                // Both agents yield half; a perfect overlap splits along X.
                                let dir = if len > 1e-4 { d / len } else { Vec2::X };
                                let push = (min - len) * 0.5;
                                next += dir * push;
                                // Head-on: also step to the walker's right (both do, so
                                // they pass instead of pushing each other back).
                                let heading = desired.normalize_or_zero();
                                if dir.dot(heading) < -0.5 {
                                    next += Vec2::new(-heading.y, heading.x) * push;
                                }
                            }
                        }
                    }
                }
            }
            let buckets = self.buckets.as_ref().unwrap();
            for &k in buckets.rects(next) {
                let [x, z] = push_out(next.to_array(), r, &self.rects[k as usize]);
                next = Vec2::new(x, z);
            }
            let agent = &mut self.agents[a];
            let moved = next - here;
            if agent.state == State::Moving && intended.length_squared() > 1e-10 {
                let progress = moved.dot(intended) / intended.length_squared();
                agent.stuck = if progress < 0.25 {
                    agent.stuck.saturating_add(1)
                } else {
                    agent.stuck.saturating_sub(1)
                };
            }
            let facing = (agent.velocity.length_squared() > 0.0025).then(|| {
                let target = agent.velocity.x.atan2(agent.velocity.y);
                let yaw = match agent.yaw {
                    Some(yaw) if agent.turn > 0.0 => approach(yaw, target, agent.turn),
                    _ => target,
                };
                agent.yaw = Some(yaw);
                yaw
            });
            self.positions[a] = next;
            if moved.length_squared() > 0.0 || facing.is_some() {
                let world = world_position(state, slot);
                let world = Vec3::new(next.x, world.y, next.y);
                let rotation = facing.map(Quat::from_rotation_y);
                let (position, rotation) = match state.compiled.parents[slot] {
                    None => (world, rotation),
                    Some(parent) => {
                        let m = world_matrix(state, parent);
                        let (_, parent_rotation, _) = m.to_scale_rotation_translation();
                        (
                            m.inverse().transform_point3(world),
                            rotation.map(|q| parent_rotation.inverse() * q),
                        )
                    }
                };
                if let Some(t) = &mut state.transforms[slot] {
                    t.position = position.to_array();
                    if let Some(q) = rotation {
                        t.rotation = q.normalize().to_array();
                    }
                }
            }
            if self.agents[a].stuck > STUCK_TICKS {
                self.agents[a].stuck = 0;
                if self.agents[a].repaths >= MAX_REPATHS {
                    self.fail(a, "stuck");
                } else {
                    self.agents[a].repaths += 1;
                    if let Some(Goal::Point(p)) = self.agents[a].goal {
                        self.agents[a].planned_for = p;
                    }
                    self.enqueue(a);
                }
            }
        }
    }

    fn emit(&mut self, state: &mut RuntimeState, a: usize, arrived: bool) {
        let agent = &self.agents[a];
        let Some(event) = (if arrived {
            agent.on_arrive
        } else {
            agent.on_fail
        }) else {
            return;
        };
        let info = &self.events[event];
        let mut fields = BTreeMap::new();
        if info.agent_field {
            fields.insert("agent".to_owned(), Scalar::Text(self.ids[a].clone()));
        }
        if info.target_field {
            let target = match &agent.goal {
                Some(Goal::Entity(_, id)) => id.clone(),
                Some(Goal::Point(p)) => format!("{},{}", round(p.x, 100.0), round(p.y, 100.0)),
                None => String::new(),
            };
            fields.insert("target".to_owned(), Scalar::Text(target));
        }
        if info.reason_field {
            let reason = if arrived { "arrived" } else { agent.reason };
            fields.insert("reason".to_owned(), Scalar::Text(reason.to_owned()));
        }
        let tick = state.tick;
        let result = match state.gameplay.as_mut() {
            Some(g) => g.command(
                &RuntimeCommand::Emit {
                    event: info.name.clone(),
                    fields,
                },
                tick,
            ),
            None => Err(anyhow::anyhow!("behavior.execute is not running")),
        };
        match result {
            Ok(()) => self.emitted += 1,
            Err(e) => {
                let name = &self.events[event].name;
                self.fault
                    .get_or_insert_with(|| format!("emit {name}: {e:#}"));
            }
        }
    }

    fn find(&self, args: &Value) -> Result<usize> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .context("nav actions need {\"id\":\"agent entity\"}")?;
        self.ids.iter().position(|x| x == id).ok_or_else(|| {
            axiom_core::Diagnostic::error(
                "not_found",
                "/args/id",
                format!("{id} is not a nav agent"),
            )
        })
    }

    fn target_json(goal: &Option<Goal>) -> Value {
        match goal {
            None => Value::Null,
            Some(Goal::Point(p)) => json!({"point":[round(p.x, 1000.0), round(p.y, 1000.0)]}),
            Some(Goal::Entity(_, id)) => json!({ "entity": id }),
        }
    }

    fn agent_json(&self, a: usize) -> Value {
        let agent = &self.agents[a];
        let p = self.positions[a];
        let waypoints: Vec<Value> = agent.waypoints[agent.next.min(agent.waypoints.len())..]
            .iter()
            .take(OBSERVED)
            .map(|w| json!([round(w.x, 100.0), round(w.y, 100.0)]))
            .collect();
        json!({
            "id":self.ids[a],
            "state":agent.state.name(),
            "reason":(agent.state == State::Failed).then_some(agent.reason),
            "target":Self::target_json(&agent.goal),
            "position":[round(p.x, 1000.0), round(p.y, 1000.0)],
            "speed":round(agent.velocity.length(), 1000.0),
            "waypoints":waypoints,
            "remaining":agent.waypoints.len().saturating_sub(agent.next),
            "queued":agent.queued,
            "searching":self.active.as_ref().is_some_and(|x| x.agent == a),
            "repaths":agent.repaths,
            "radius":agent.radius,
            "enabled":agent.enabled,
        })
    }

    fn summary(&self) -> Value {
        let count = |s: State| self.agents.iter().filter(|a| a.state == s).count();
        let at: serde_json::Map<String, Value> = self
            .agents
            .iter()
            .zip(&self.ids)
            .zip(&self.positions)
            .take(OBSERVED)
            .map(|((a, id), p)| {
                (
                    id.clone(),
                    json!([a.state.name(), round(p.x, 100.0), round(p.y, 100.0)]),
                )
            })
            .collect();
        let failures: serde_json::Map<String, Value> = self
            .agents
            .iter()
            .zip(&self.ids)
            .filter(|(a, _)| a.state == State::Failed)
            .take(OBSERVED_FAILURES)
            .map(|(a, id)| (id.clone(), json!(a.reason)))
            .collect();
        let grid = self.grid.as_ref().map(|g| {
            json!({
                "size":[g.spec.width, g.spec.height],
                "cell":g.spec.cell,
                "origin":[round(g.spec.origin[0], 1000.0), round(g.spec.origin[1], 1000.0)],
                "obstacles":self.obstacles,
                "blockers":self.blockers,
                "blocked_cells":g.blocked,
                "build_ms":(self.build_ms * 1000.0).round() / 1000.0,
                "cached":self.grid_cached,
            })
        });
        json!({
            "grid":grid,
            "agents":self.agents.len(),
            "idle":count(State::Idle),
            "pending":count(State::Pending),
            "moving":count(State::Moving),
            "arrived":count(State::Arrived),
            "failed":count(State::Failed),
            "at":at,
            "more":self.agents.len().saturating_sub(OBSERVED),
            "failures":failures,
            "queue":self.queue.iter().filter(|&&a| self.agents[a].queued).count(),
            "searching":self.active.as_ref().map(|x| self.ids[x.agent].as_str()),
            "budget":{
                "per_step":self.settings.expansions_per_step,
                "last":self.used,
                "max":self.used_max,
                "deferred_steps":self.deferred_steps,
                "searches":self.searches,
            },
            "emitted":self.emitted,
            "halted":self.halted,
            "fault":self.fault,
        })
    }

    /// Play-preview overlay: (agent world position, remaining waypoints at the agent's height).
    pub fn routes(&self, state: &RuntimeState) -> Vec<(Vec3, Vec<Vec3>)> {
        self.agents
            .iter()
            .filter(|a| a.enabled && state.compiled.enabled[a.slot])
            .map(|a| {
                let p = world_position(state, a.slot);
                let rest = a.waypoints[a.next.min(a.waypoints.len())..]
                    .iter()
                    .map(|w| Vec3::new(w.x, p.y, w.y))
                    .collect();
                (p, rest)
            })
            .collect()
    }

    /// Editor preview: the path the agent on `slot` would walk to its authored target from
    /// where it stands, as world XZ waypoints. Synchronous but bounded
    /// (`PREVIEW_EXPANSIONS`) and cached by its inputs; `None` when there is no target or no
    /// path within the bound.
    pub fn preview_path(&self, state: &RuntimeState, slot: usize) -> Option<Vec<Vec2>> {
        let grid = self.grid.as_ref()?;
        let agent = self.agents.iter().find(|a| a.slot == slot)?;
        let target = match agent.authored_target.as_ref()? {
            NavTarget::Point(p) => Vec2::new(p[0], p[2]),
            NavTarget::Entity(id) => xz(world_position(state, *state.compiled.indices.get(id)?)),
        };
        let start = xz(world_position(state, slot));
        let from = grid.nearest_walkable(start, agent.threshold)?;
        let to = grid.nearest_walkable(target, agent.threshold)?;
        let key = (
            slot,
            from,
            to,
            agent.threshold,
            [target.x.to_bits(), target.y.to_bits()],
        );
        let mut preview = self.preview.lock().ok()?;
        if preview.key == Some(key) {
            return preview.path.clone();
        }
        let p = &mut *preview;
        let search = p
            .search
            .get_or_insert_with(|| GridSearch::new(grid.spec.width, grid.spec.height));
        search.begin(from, to, Neighbourhood::Eight);
        let threshold = agent.threshold;
        let goal = if grid.spec.cell_of(target) == Some(to) {
            target
        } else {
            grid.spec.center(to)
        };
        p.path = match search.run(PREVIEW_EXPANSIONS, |i| grid.walkable(i, threshold)) {
            Progress::Found => {
                search.path(&mut p.cells);
                let mut out = vec![];
                let leg = Leg {
                    start: grid.spec.center(from),
                    cells: &p.cells,
                    goal,
                };
                straighten(
                    grid,
                    leg,
                    threshold,
                    PREVIEW_EXPANSIONS,
                    &mut p.corners,
                    &mut out,
                )
                .map(|_| out)
            }
            _ => None,
        };
        p.key = Some(key);
        p.path.clone()
    }
}

impl RuntimeSystem for NavSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, _input: &ActionMap) {
        if self.halted.is_some() {
            return;
        }
        let len = state.compiled.len();
        if len > self.scanned {
            if let Some(e) = state.compiled.entities[self.scanned..]
                .iter()
                .find(|e| e.nav_agent.is_some())
            {
                self.halted = Some(format!(
                    "runtime-spawned entity {} has nav.agent; spawned agents are not supported (author them instead)",
                    e.id
                ));
                return;
            }
            self.scanned = len;
        }
        if self.agents.is_empty() {
            return;
        }
        for a in 0..self.agents.len() {
            self.positions[a] = xz(world_position(state, self.agents[a].slot));
        }
        self.fired.clear();
        self.used = 0;
        self.track(state);
        self.service();
        self.used_max = self.used_max.max(self.used);
        self.steer(state);
        if self.halted.is_some() {
            // Event budget exceeded: this step emits none and the system stops.
            return;
        }
        if self.fired.is_empty() {
            return;
        }
        for k in 0..self.fired.len() {
            let (a, arrived) = self.fired[k];
            self.emit(state, a, arrived);
        }
        state.sync_scenes();
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        _previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        // Hot reload keeps each agent's progress by entity ID while its authored target and
        // radius are unchanged. With an unchanged grid the current path is kept; otherwise
        // the agent re-plans to the same goal.
        let Some(old) = old.as_any().downcast_ref::<Self>() else {
            return Ok(());
        };
        let same_grid = old.key == self.key;
        let previous: BTreeMap<&str, usize> = old
            .ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        for a in 0..self.agents.len() {
            let Some(&o) = previous.get(self.ids[a].as_str()) else {
                continue;
            };
            let was = &old.agents[o];
            let agent = &self.agents[a];
            if was.authored_target != agent.authored_target || was.radius != agent.radius {
                continue;
            }
            // Re-resolve an entity goal by ID in the new world.
            let goal = match &was.goal {
                Some(Goal::Entity(_, id)) => match next.compiled.indices.get(id) {
                    Some(&slot) if next.transforms[slot].is_some() => {
                        Some(Goal::Entity(slot, id.clone()))
                    }
                    _ => None,
                },
                other => other.clone(),
            };
            if was.goal.is_some() && goal.is_none() {
                continue;
            }
            let replan = matches!(was.state, State::Pending)
                || (matches!(was.state, State::Moving) && !same_grid)
                || was.queued
                || old.active.as_ref().is_some_and(|x| x.agent == o);
            if self.queue.contains(&a) {
                self.agents[a].queued = false;
            }
            let agent = &mut self.agents[a];
            agent.goal = goal;
            agent.state = was.state;
            agent.reason = was.reason;
            agent.velocity = was.velocity;
            agent.yaw = was.yaw;
            agent.planned_for = was.planned_for;
            agent.repaths = was.repaths;
            agent.waypoints.clear();
            agent.next = 0;
            if same_grid && !replan {
                agent.waypoints.extend_from_slice(&was.waypoints);
                agent.next = was.next;
            }
            if replan && agent.goal.is_some() {
                agent.state = State::Pending;
                self.enqueue(a);
            }
        }
        self.queue.retain(|&a| self.agents[a].queued);
        self.deferred_steps = old.deferred_steps;
        self.searches = old.searches;
        self.used_max = old.used_max;
        self.emitted = old.emitted;
        self.fault = old.fault.clone();
        Ok(())
    }
    fn screen_pointer(&mut self, _state: &mut RuntimeState, pointer: &ScreenPointer) -> Result<()> {
        let (Some(a), PointerKind::PrimaryDown) = (self.pointer_agent, pointer.kind) else {
            return Ok(());
        };
        let origin = Vec3::from_array(pointer.origin);
        let direction = Vec3::from_array(pointer.direction);
        if direction.y.abs() < 1e-5 {
            return Ok(());
        }
        let t = (self.settings.floor_y - origin.y) / direction.y;
        if t <= 0.0 || !t.is_finite() {
            return Ok(());
        }
        let hit = origin + direction * t;
        self.set_goal(a, Goal::Point(xz(hit)));
        Ok(())
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "nav" {
            return None;
        }
        Some(match action {
            "summary" => Ok(self.summary()),
            "inspect" => self.find(args).map(|a| self.agent_json(a)),
            "stop" => self.find(args).map(|a| {
                self.stop(a);
                self.agent_json(a)
            }),
            "go" => (|| {
                let a = self.find(args)?;
                let target: NavTarget = match (args.get("point"), args.get("entity")) {
                    (Some(p), None) => NavTarget::Point(
                        serde_json::from_value(p.clone()).context("point must be [x,y,z]")?,
                    ),
                    (None, Some(Value::String(id))) => NavTarget::Entity(id.clone()),
                    _ => bail!("nav.go needs {{\"id\":..}} and either point:[x,y,z] or entity:ID"),
                };
                target.validate()?;
                let goal = self.resolve(state, &target)?;
                if let Goal::Entity(slot, id) = &goal {
                    ensure!(*slot != self.agents[a].slot, "{id} cannot walk to itself");
                }
                ensure!(
                    self.agents[a].enabled,
                    "{} is a disabled agent (enabled false)",
                    self.ids[a]
                );
                // Plan for where the target is now.
                self.agents[a].planned_for = Self::goal_position(state, &goal);
                self.set_goal(a, goal);
                Ok(self.agent_json(a))
            })(),
            other => Err(anyhow::anyhow!("unknown nav action {other}")),
        })
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("nav".into(), self.summary())])
    }
}

fn id_arg(v: &Value) -> Result<()> {
    ensure!(
        v.get("id").and_then(Value::as_str).is_some(),
        "needs {{\"id\":\"agent entity ID\"}}"
    );
    Ok(())
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.action(
        "nav.go",
        "nav",
        json!({"id":"agent entity ID","point":"[x,y,z] world point (Y ignored)","entity":"target entity ID (instead of point; followed while it moves)","effect":"queues a path request; on_arrive / on_fail fire later"}),
        |v| {
            id_arg(v)?;
            match (v.get("point"), v.get("entity")) {
                (Some(Value::Array(p)), None)
                    if p.len() == 3 && p.iter().all(|x| x.as_f64().is_some_and(f64::is_finite)) => {}
                (None, Some(Value::String(_))) => {}
                _ => bail!("nav.go needs either point:[x,y,z] or entity:ID"),
            }
            Ok(())
        },
    );
    r.authoring.action(
        "nav.stop",
        "nav",
        json!({"id":"agent entity ID","effect":"clears the target; the agent brakes to a stop"}),
        id_arg,
    );
    r.authoring.action(
        "nav.inspect",
        "nav",
        json!({"id":"agent entity ID; state, reason, target, position, speed, remaining waypoints"}),
        id_arg,
    );
    r.authoring
        .action("nav.summary", "nav", json!({}), |_| Ok(()));
    r.factories.insert("nav.steer".into(), |state| {
        Ok(Box::new(NavSystem::new(state)?))
    });
}
