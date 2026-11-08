use crate::*;
use axiom_core::CompiledWorld;
use glam::Vec2;
use std::collections::{BTreeSet, VecDeque};
type CachedPath = (([i32; 2], [i32; 2]), VecDeque<[i32; 2]>);
pub const DT: f32 = 1.0 / 120.0;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Order {
    Idle,
    Move { destination: [f32; 2] },
    Gather { target: String, returning: bool },
    Attack { target: String },
}
#[derive(Clone)]
pub struct Unit {
    pub id: String,
    pub def: UnitDef,
    pub position: [f32; 2],
    pub health: f32,
    pub max_health: f32,
    pub order: Order,
    pub carried: u32,
    pub produced: Option<String>,
    pub yaw: f32,
    path: VecDeque<[i32; 2]>,
    goal: Option<[i32; 2]>,
    generation: u64,
    pending: bool,
    cooldown: u32,
    stuck: u32,
    retry_after: u64,
    moving_to: Option<[i32; 2]>,
}
#[derive(Clone)]
pub struct Job {
    pub recipe: String,
    pub remaining: u32,
    pub cost: u32,
}
#[derive(Clone)]
pub struct Building {
    pub id: String,
    pub def: BuildingDef,
    pub position: [f32; 2],
    pub health: f32,
    pub queue: VecDeque<Job>,
}
#[derive(Clone)]
pub struct Resource {
    pub id: String,
    pub position: [f32; 2],
    pub remaining: u32,
}
#[derive(Clone)]
struct NavJob {
    avoid: bool,
    unit: usize,
    generation: u64,
    start: [i32; 2],
    goal: [i32; 2],
}
#[derive(Clone, Default, Serialize)]
pub struct Stats {
    pub replans: u64,
    pub transient_blocked: u64,
    pub orders: u64,
    pub completed_moves: u64,
    pub path_requests: u64,
    pub paths_solved: u64,
    pub no_path: u64,
    pub search_exhausted: u64,
    pub expanded_nodes: u64,
    pub cache_hits: u64,
    pub max_nav_queue: usize,
    pub harvested: u64,
    pub delivered: u64,
    pub spent: u64,
    pub lost: u64,
    pub produced: u64,
    pub attacks: u64,
    pub kills: u64,
}
#[derive(Clone)]
pub struct Simulation {
    pub session: Session,
    pub grid: nav::Grid,
    pub units: Vec<Unit>,
    pub buildings: Vec<Building>,
    pub resources: Vec<Resource>,
    pub selected: BTreeSet<usize>,
    pub balance: [u32; 2],
    pub tick: u64,
    pub stats: Stats,
    pub last_error: Option<String>,
    pub victory: bool,
    pub defeated: bool,
    pub initial_economy: u64,
    pub spawned: Vec<String>,
    nav_queue: VecDeque<NavJob>,
    cache: VecDeque<CachedPath>,
    serial: u64,
    pub combat: Option<axiom_combat::Scene>,
    pub hits: Vec<axiom_combat::Hit>,
}
impl Simulation {
    pub fn select_role(&mut self, args: RoleArgs) -> Result<Value> {
        let ids = self
            .units
            .iter()
            .filter(|u| {
                u.def.faction == self.session.player_faction
                    && u.health > 0.0
                    && u.def.role == args.role
            })
            .map(|u| u.id.clone())
            .collect();
        self.select(SelectArgs {
            ids,
            additive: args.additive,
        })
    }
    /// Native AI contexts use the same checked group-order path with their own faction authority.
    pub fn move_for_faction(&mut self, faction: u8, args: MoveArgs) -> Result<Value> {
        ensure!(faction < 2, "unknown faction");
        let player = self.session.player_faction;
        self.session.player_faction = faction;
        let result = self.move_group(args);
        self.session.player_faction = player;
        result
    }
    pub fn new(w: &CompiledWorld) -> Result<Self> {
        let session: Session = w.source.resource("rts.session")?;
        let grid = nav::Grid::new(session.grid.clone())?;
        let mut s = Self {
            balance: session.initial_balance,
            session,
            grid,
            units: vec![],
            buildings: vec![],
            resources: vec![],
            selected: BTreeSet::new(),
            tick: 0,
            stats: Stats::default(),
            last_error: None,
            victory: false,
            defeated: false,
            initial_economy: 0,
            spawned: vec![],
            nav_queue: VecDeque::new(),
            cache: VecDeque::new(),
            serial: 0,
            combat: None,
            hits: vec![],
        };
        for e in &w.entities {
            let p = e
                .transform
                .as_ref()
                .map_or([0.0; 2], |t| [t.position[0], t.position[2]]);
            if let Some(v) = e.components.get("rts.unit") {
                s.units.push(Unit {
                    id: e.id.clone(),
                    def: serde_json::from_value(v.clone())?,
                    position: p,
                    health: e.health.as_ref().unwrap().value,
                    max_health: e.health.as_ref().unwrap().max,
                    order: Order::Idle,
                    carried: 0,
                    produced: None,
                    yaw: 0.0,
                    path: VecDeque::new(),
                    goal: None,
                    generation: 0,
                    pending: false,
                    cooldown: 0,
                    stuck: 0,
                    retry_after: 0,
                    moving_to: None,
                });
            }
            if let Some(v) = e.components.get("rts.building") {
                s.buildings.push(Building {
                    id: e.id.clone(),
                    def: serde_json::from_value(v.clone())?,
                    position: p,
                    health: e.health.as_ref().unwrap().value,
                    queue: VecDeque::new(),
                });
            }
            if let Some(v) = e.components.get("rts.resource") {
                s.resources.push(Resource {
                    id: e.id.clone(),
                    position: p,
                    remaining: serde_json::from_value::<ResourceDef>(v.clone())?.amount,
                });
            }
        }
        s.initial_economy = s.balance.iter().map(|v| *v as u64).sum::<u64>()
            + s.resources.iter().map(|r| r.remaining as u64).sum::<u64>();
        Ok(s)
    }
    fn owned(&self, ids: Option<&[String]>) -> Result<Vec<usize>> {
        let indices = if let Some(ids) = ids {
            ensure!(
                ids.len() <= self.session.max_units,
                "unit command budget exceeded"
            );
            ids.iter()
                .map(|id| {
                    self.units
                        .iter()
                        .position(|u| &u.id == id)
                        .ok_or_else(|| Diagnostic::error("not_found", "/command/args/units", id))
                })
                .collect::<Result<BTreeSet<_>>>()?
        } else {
            self.selected.clone()
        };
        ensure!(!indices.is_empty(), "no units selected");
        for i in &indices {
            let u = &self.units[*i];
            if u.def.faction != self.session.player_faction {
                return Err(Diagnostic::error("not_owned", "/command/args/units", &u.id));
            }
            ensure!(u.health > 0.0, "unit is dead: {}", u.id);
        }
        Ok(indices.into_iter().collect())
    }
    pub fn select(&mut self, a: SelectArgs) -> Result<Value> {
        let ids = if a.ids.is_empty() {
            vec![]
        } else {
            self.owned(Some(&a.ids))?
        };
        if !a.additive {
            self.selected.clear();
        }
        self.selected.extend(ids);
        Ok(json!({"selected":self.selected_ids()}))
    }
    pub fn select_rect(&mut self, a: RectArgs) -> Result<Value> {
        ensure!(
            a.min.iter().chain(&a.max).all(|n| n.is_finite()),
            "invalid rectangle"
        );
        let min = [a.min[0].min(a.max[0]), a.min[1].min(a.max[1])];
        let max = [a.min[0].max(a.max[0]), a.min[1].max(a.max[1])];
        if !a.additive {
            self.selected.clear();
        }
        for (i, u) in self.units.iter().enumerate() {
            if u.def.faction == self.session.player_faction
                && u.health > 0.0
                && u.position[0] >= min[0]
                && u.position[0] <= max[0]
                && u.position[1] >= min[1]
                && u.position[1] <= max[1]
            {
                self.selected.insert(i);
            }
        }
        Ok(json!({"selected":self.selected_ids()}))
    }
    pub fn selected_ids(&self) -> Vec<String> {
        self.selected
            .iter()
            .filter(|i| self.units[**i].health > 0.0)
            .map(|i| self.units[*i].id.clone())
            .collect()
    }
    fn request_path(&mut self, i: usize, goal: [i32; 2]) {
        if self.units[i].goal == Some(goal)
            && (self.units[i].pending || !self.units[i].path.is_empty())
        {
            return;
        }
        let Some(start) = self.grid.cell(self.units[i].position) else {
            return;
        };
        let u = &mut self.units[i];
        u.generation += 1;
        u.path.clear();
        u.goal = Some(goal);
        u.pending = true;
        u.stuck = 0;
        u.moving_to = None;
        self.nav_queue.retain(|j| j.unit != i);
        self.nav_queue.push_back(NavJob {
            avoid: false,
            unit: i,
            generation: u.generation,
            start,
            goal,
        });
        self.stats.path_requests += 1;
        self.stats.max_nav_queue = self.stats.max_nav_queue.max(self.nav_queue.len());
    }
    fn clear_path(&mut self, i: usize) {
        let u = &mut self.units[i];
        u.path.clear();
        u.goal = None;
        u.pending = false;
        u.moving_to = None;
        u.generation += 1;
        self.nav_queue.retain(|j| j.unit != i);
    }
    pub fn move_group(&mut self, a: MoveArgs) -> Result<Value> {
        let units = self.owned(a.units.as_deref())?;
        let center = self
            .grid
            .cell(a.destination)
            .context("destination outside grid")?;
        ensure!(self.grid.walkable(center), "destination is blocked");
        let columns = (units.len() as f32).sqrt().ceil() as i32;
        let rows = (units.len() as i32 + columns - 1) / columns;
        let mut used = BTreeSet::new();
        let mut goals = vec![];
        for (n, i) in units.iter().enumerate() {
            let base = [
                center[0] + (n as i32 % columns - columns / 2) * 2,
                center[1] + (n as i32 / columns - rows / 2) * 2,
            ];
            let mut goal = None;
            for radius in 0..=12 {
                for dz in -radius..=radius {
                    for dx in -radius..=radius {
                        let c = [base[0] + dx, base[1] + dz];
                        if self.grid.walkable(c) && !used.contains(&c) {
                            goal = Some(c);
                            break;
                        }
                    }
                    if goal.is_some() {
                        break;
                    }
                }
                if goal.is_some() {
                    break;
                }
            }
            let goal = goal.context("formation does not fit in reachable grid")?;
            used.insert(goal);
            goals.push((*i, goal));
        }
        for (i, goal) in goals {
            self.units[i].order = Order::Move {
                destination: self.grid.center(goal),
            };
            self.request_path(i, goal);
        }
        self.stats.orders += 1;
        Ok(json!({"accepted":units.len(),"destination":a.destination}))
    }
    pub fn gather(&mut self, a: TargetArgs) -> Result<Value> {
        let units = self.owned(a.units.as_deref())?;
        let node = self
            .resources
            .iter()
            .find(|r| r.id == a.target)
            .context("resource target missing")?;
        ensure!(node.remaining > 0, "resource depleted");
        let goal = self
            .grid
            .cell(node.position)
            .and_then(|c| self.grid.nearest(c, 4))
            .context("resource inaccessible")?;
        for &i in &units {
            ensure!(
                self.units[i].def.role == Role::Worker && self.units[i].def.carry_capacity > 0,
                "only workers can gather"
            );
            ensure!(
                self.depot(self.units[i].def.faction, self.units[i].position)
                    .is_some(),
                "no friendly depot available"
            );
        }
        for &i in &units {
            self.units[i].order = Order::Gather {
                target: a.target.clone(),
                returning: self.units[i].carried > 0,
            };
            self.request_path(i, goal);
        }
        self.stats.orders += 1;
        Ok(json!({"accepted":units.len(),"target":a.target}))
    }
    pub fn target(&self, id: &str) -> Option<([f32; 2], u8, f32)> {
        self.units
            .iter()
            .find(|u| u.id == id)
            .map(|u| (u.position, u.def.faction, u.health))
            .or_else(|| {
                self.buildings
                    .iter()
                    .find(|b| b.id == id)
                    .map(|b| (b.position, b.def.faction, b.health))
            })
            .or_else(|| {
                self.combat
                    .as_ref()?
                    .bodies
                    .get(id)
                    .map(|b| (b.position.to_array(), b.faction, b.health))
            })
    }
    pub fn attack(&mut self, a: TargetArgs) -> Result<Value> {
        let units = self.owned(a.units.as_deref())?;
        let (p, faction, hp) = self.target(&a.target).context("attack target missing")?;
        ensure!(
            hp > 0.0 && faction != self.session.player_faction,
            "attack requires a live enemy target"
        );
        let goal = self
            .grid
            .cell(p)
            .and_then(|c| self.grid.nearest(c, 4))
            .context("attack target inaccessible")?;
        for &i in &units {
            self.units[i].order = Order::Attack {
                target: a.target.clone(),
            };
            self.request_path(i, goal);
        }
        self.stats.orders += 1;
        Ok(json!({"accepted":units.len()}))
    }
    pub fn stop(&mut self, a: SelectArgs) -> Result<Value> {
        let units = self.owned(if a.ids.is_empty() { None } else { Some(&a.ids) })?;
        for &i in &units {
            self.units[i].order = Order::Idle;
            self.clear_path(i);
        }
        self.stats.orders += 1;
        Ok(json!({"accepted":units.len()}))
    }
    pub fn train(&mut self, a: TrainArgs) -> Result<Value> {
        ensure!((1..=8).contains(&a.count), "production count must be 1..8");
        let i = self
            .buildings
            .iter()
            .position(|b| b.id == a.building)
            .context("building missing")?;
        let b = &self.buildings[i];
        ensure!(
            b.health > 0.0 && b.def.faction == self.session.player_faction,
            "building is not owned/alive"
        );
        ensure!(
            b.def.produces.contains(&a.recipe),
            "building cannot produce this recipe"
        );
        let recipe = self
            .session
            .recipes
            .get(&a.recipe)
            .context("recipe missing")?;
        ensure!(
            b.queue.len() + a.count as usize <= 16,
            "production queue full"
        );
        let queued = self.buildings.iter().map(|b| b.queue.len()).sum::<usize>();
        ensure!(
            self.units.len() + queued + a.count as usize <= self.session.max_units,
            "population budget exceeded"
        );
        let cost = recipe
            .cost
            .checked_mul(a.count)
            .context("production cost overflow")?;
        let faction = b.def.faction as usize;
        ensure!(self.balance[faction] >= cost, "insufficient minerals");
        self.balance[faction] -= cost;
        self.stats.spent += cost as u64;
        for _ in 0..a.count {
            self.buildings[i].queue.push_back(Job {
                recipe: a.recipe.clone(),
                remaining: recipe.build_ticks,
                cost: recipe.cost,
            });
        }
        Ok(json!({"queued":a.count,"cost":cost,"balance":self.balance[faction]}))
    }
    fn depot(&self, faction: u8, p: [f32; 2]) -> Option<usize> {
        self.buildings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.def.faction == faction && b.def.depot && b.health > 0.0)
            .min_by(|(_, a), (_, b)| dist(a.position, p).total_cmp(&dist(b.position, p)))
            .map(|(i, _)| i)
    }
    fn navigate(&mut self) {
        for _ in 0..self.session.path_budget {
            let Some(job) = self.nav_queue.pop_front() else {
                break;
            };
            if self.units[job.unit].generation != job.generation
                || self.units[job.unit].health <= 0.0
            {
                continue;
            }
            let cached = if job.avoid {
                None
            } else {
                self.cache
                    .iter()
                    .find(|(key, _)| *key == (job.start, job.goal))
                    .map(|(_, path)| path.clone())
            };
            let path = if let Some(path) = cached {
                self.stats.cache_hits += 1;
                Some(path)
            } else {
                let occupied = if job.avoid {
                    Some(
                        self.grid.occupied(
                            self.units
                                .iter()
                                .filter(|u| u.health > 0.0)
                                .filter_map(|u| self.grid.cell(u.position)),
                            job.start,
                            job.goal,
                        ),
                    )
                } else {
                    None
                };
                let result = occupied.as_ref().unwrap_or(&self.grid).search(
                    job.start,
                    job.goal,
                    self.session.search_budget,
                );
                self.stats.expanded_nodes += result.expanded as u64;
                if result.exhausted {
                    self.stats.search_exhausted += 1;
                }
                if let Some(path) = &result.path {
                    if !job.avoid {
                        if self.cache.len() == 128 {
                            self.cache.pop_front();
                        }
                        self.cache.push_back(((job.start, job.goal), path.clone()));
                    }
                }
                result.path
            };
            let u = &mut self.units[job.unit];
            u.pending = false;
            if let Some(mut path) = path {
                if dist(u.position, self.grid.center(job.start)) > 0.03 {
                    path.push_front(job.start);
                }
                u.path = path;
                self.stats.paths_solved += 1;
            } else if job.avoid {
                u.retry_after = self.tick + 60;
                self.stats.transient_blocked += 1;
            } else {
                u.order = Order::Idle;
                u.goal = None;
                self.stats.no_path += 1;
                self.last_error = Some(format!("no_path:{}", u.id));
            }
        }
    }
    fn decision(&mut self) {
        for i in 0..self.units.len() {
            if self.units[i].health <= 0.0 {
                continue;
            }
            if self.units[i].path.is_empty()
                && !self.units[i].pending
                && self.tick >= self.units[i].retry_after
                && !matches!(self.units[i].order, Order::Idle)
            {
                if let Some(goal) = self.units[i].goal {
                    if dist(self.units[i].position, self.grid.center(goal)) > 0.03 {
                        self.units[i].goal = None;
                        self.request_path(i, goal);
                        if let Some(job) = self.nav_queue.back_mut() {
                            job.avoid = true;
                        }
                        self.stats.replans += 1;
                    }
                }
            }
            match self.units[i].order.clone() {
                Order::Move { destination } => {
                    if dist(self.units[i].position, destination) < 0.03 && !self.units[i].pending {
                        self.units[i].order = Order::Idle;
                        self.stats.completed_moves += 1;
                        self.clear_path(i);
                    }
                }
                Order::Gather { target, returning } => {
                    let Some(node) = self.resources.iter().position(|r| r.id == target) else {
                        self.units[i].order = Order::Idle;
                        continue;
                    };
                    let p = self.units[i].position;
                    if returning {
                        if let Some(depot) = self.depot(self.units[i].def.faction, p) {
                            let bp = self.buildings[depot].position;
                            if dist(p, bp) <= 2.2 {
                                let amount = self.units[i].carried;
                                let faction = self.units[i].def.faction as usize;
                                self.balance[faction] += amount;
                                self.stats.delivered += amount as u64;
                                self.units[i].carried = 0;
                                self.clear_path(i);
                                if self.resources[node].remaining == 0 {
                                    self.units[i].order = Order::Idle;
                                } else {
                                    self.units[i].order = Order::Gather {
                                        target,
                                        returning: false,
                                    };
                                }
                            } else if let Some(goal) =
                                self.grid.cell(bp).and_then(|c| self.grid.nearest(c, 5))
                            {
                                self.request_path(i, goal);
                            }
                        } else {
                            self.last_error = Some("friendly_depot_destroyed".into());
                            self.units[i].order = Order::Idle;
                            self.clear_path(i);
                        }
                    } else if self.resources[node].remaining == 0
                        || self.units[i].carried >= self.units[i].def.carry_capacity
                    {
                        self.units[i].order = Order::Gather {
                            target,
                            returning: true,
                        };
                        self.clear_path(i);
                    } else if dist(p, self.resources[node].position) <= 1.6 {
                        self.clear_path(i);
                        if self.units[i].cooldown == 0 {
                            self.resources[node].remaining -= 1;
                            self.units[i].carried += 1;
                            self.stats.harvested += 1;
                            self.units[i].cooldown = self.units[i].def.gather_ticks;
                        }
                    } else if let Some(goal) = self
                        .grid
                        .cell(self.resources[node].position)
                        .and_then(|c| self.grid.nearest(c, 4))
                    {
                        self.request_path(i, goal);
                    }
                }
                Order::Attack { target } => {
                    if let Some((p, _, hp)) = self.target(&target) {
                        if hp <= 0.0 {
                            self.units[i].order = Order::Idle;
                            self.clear_path(i);
                        } else if dist(self.units[i].position, p)
                            <= self.units[i].def.attack_range
                                + self
                                    .combat
                                    .as_ref()
                                    .and_then(|s| s.bodies.get(&target))
                                    .map_or(0.0, |b| b.radius)
                        {
                            self.clear_path(i);
                            if self.units[i].cooldown == 0 {
                                let damage = self.units[i].def.damage;
                                self.units[i].cooldown = self.units[i].def.attack_ticks;
                                if let Some(scene) = &self.combat {
                                    if let Some(hit) = scene.attempt(
                                        &self.units[i].id,
                                        &target,
                                        damage,
                                        self.units[i].def.attack_range,
                                        None,
                                    ) {
                                        self.hits.push(hit);
                                    }
                                } else {
                                    self.damage(&target, damage);
                                }
                                self.stats.attacks += 1;
                            }
                        } else if let Some(goal) =
                            self.grid.cell(p).and_then(|c| self.grid.nearest(c, 4))
                        {
                            self.request_path(i, goal);
                        }
                    } else {
                        self.units[i].order = Order::Idle;
                        self.clear_path(i);
                    }
                }
                Order::Idle => {}
            }
        }
        if self.session.auto_combat && self.tick.is_multiple_of(24) {
            let mut orders = vec![];
            for (i, u) in self.units.iter().enumerate() {
                if u.health > 0.0 && u.def.role == Role::Soldier && matches!(u.order, Order::Idle) {
                    if let Some(target) = self
                        .units
                        .iter()
                        .filter(|t| {
                            t.health > 0.0
                                && t.def.faction != u.def.faction
                                && dist(t.position, u.position) <= 4.0
                        })
                        .min_by(|a, b| {
                            dist(a.position, u.position).total_cmp(&dist(b.position, u.position))
                        })
                    {
                        orders.push((i, target.id.clone()));
                    }
                }
            }
            if let Some(scene) = &self.combat {
                for (i, u) in self.units.iter().enumerate() {
                    if u.health > 0.0
                        && u.def.role == Role::Soldier
                        && matches!(u.order, Order::Idle)
                    {
                        if let Some(b) = scene
                            .bodies
                            .values()
                            .filter(|b| {
                                b.alive()
                                    && b.faction != u.def.faction
                                    && b.position.distance(Vec2::from_array(u.position)) <= 4.0
                            })
                            .min_by(|a, b| {
                                a.position
                                    .distance_squared(Vec2::from_array(u.position))
                                    .total_cmp(
                                        &b.position.distance_squared(Vec2::from_array(u.position)),
                                    )
                            })
                        {
                            orders.retain(|(j, _)| *j != i);
                            orders.push((i, b.handle.id.clone()));
                        }
                    }
                }
            }
            for (i, target) in orders {
                self.units[i].order = Order::Attack { target };
            }
        }
    }
    fn damage(&mut self, id: &str, damage: f32) {
        if let Some(i) = self.units.iter().position(|u| u.id == id) {
            let before = self.units[i].health;
            self.units[i].health = (before - damage).max(0.0);
            if before > 0.0 && self.units[i].health == 0.0 {
                self.stats.kills += 1;
                self.stats.lost += self.units[i].carried as u64;
                self.units[i].carried = 0;
                self.clear_path(i);
                self.selected.remove(&i);
            }
        } else if let Some(i) = self.buildings.iter().position(|b| b.id == id) {
            let b = &mut self.buildings[i];
            b.health = (b.health - damage).max(0.0);
            if b.health == 0.0 {
                self.stats.lost += b.queue.iter().map(|j| j.cost as u64).sum::<u64>();
                self.stats.spent -= b.queue.iter().map(|j| j.cost as u64).sum::<u64>();
                b.queue.clear();
            }
        }
    }
    fn spawn_position(&self, p: [f32; 2], radius: f32) -> Option<[f32; 2]> {
        let center = self.grid.cell(p)?;
        for r in 2i32..=8 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let c = [center[0] + dx, center[1] + dz];
                    if self.grid.walkable(c) {
                        let point = self.grid.center(c);
                        if self
                            .combat
                            .as_ref()
                            .is_none_or(|s| s.clear("", Vec2::from_array(point), radius))
                            && self.units.iter().all(|u| {
                                u.health <= 0.0
                                    || dist(u.position, point) >= u.def.radius + radius + 0.05
                            })
                        {
                            return Some(point);
                        }
                    }
                }
            }
        }
        None
    }
    fn production(&mut self, templates: &BTreeMap<String, (UnitDef, f32)>) {
        for i in 0..self.buildings.len() {
            if self.buildings[i].health <= 0.0 {
                continue;
            }
            let Some(job) = self.buildings[i].queue.front_mut() else {
                continue;
            };
            job.remaining = job.remaining.saturating_sub(1);
            if job.remaining != 0 {
                continue;
            }
            let recipe_id = job.recipe.clone();
            let recipe = &self.session.recipes[&recipe_id];
            let (def, hp) = templates[&recipe.prefab].clone();
            let prefab = recipe.prefab.clone();
            let Some(position) = self.spawn_position(self.buildings[i].position, def.radius) else {
                continue;
            };
            self.buildings[i].queue.pop_front();
            self.serial += 1;
            let id = format!("produced/{}/{}", self.buildings[i].def.faction, self.serial);
            let mut def = def;
            def.faction = self.buildings[i].def.faction;
            self.units.push(Unit {
                id: id.clone(),
                def,
                position,
                health: hp,
                max_health: hp,
                order: Order::Idle,
                carried: 0,
                produced: Some(prefab),
                yaw: 0.0,
                path: VecDeque::new(),
                goal: None,
                generation: 0,
                pending: false,
                cooldown: 0,
                stuck: 0,
                retry_after: 0,
                moving_to: None,
            });
            self.spawned.push(id);
            self.stats.produced += 1;
        }
    }
    fn move_units(&mut self) {
        // Reserve both occupied and destination cells during interpolation. This avoids head-on
        // penetration and lets the bounded replanner route around waiting formations.
        let mut buckets = BTreeMap::<[i32; 2], Vec<usize>>::new();
        for (i, u) in self.units.iter().enumerate() {
            if u.health <= 0.0 {
                continue;
            }
            if let Some(cell) = self.grid.cell(u.position) {
                buckets.entry(cell).or_default().push(i);
                if let Some(next) = u.moving_to {
                    if next != cell {
                        buckets.entry(next).or_default().push(i);
                    }
                }
            }
        }
        let n = self.units.len();
        for offset in 0..n {
            let i = (offset + (self.tick / 4) as usize) % n;
            if self.units[i].health <= 0.0 {
                continue;
            }
            let Some(next) = self.units[i].path.front().copied() else {
                continue;
            };
            let current = Vec2::from_array(self.units[i].position);
            let target = Vec2::from_array(self.grid.center(next));
            let delta = target - current;
            let step = self.units[i].def.speed * DT;
            let desired = if delta.length() <= step {
                target
            } else {
                current + delta.normalize() * step
            };
            let old_cell = self.grid.cell(self.units[i].position).unwrap();
            let reserved = buckets
                .get(&next)
                .is_some_and(|rows| rows.iter().any(|j| *j != i));
            let mut clear = !reserved
                && self.combat.as_ref().map_or_else(
                    || {
                        self.grid
                            .clear(desired.to_array(), self.units[i].def.radius)
                    },
                    |scene| scene.clear(&self.units[i].id, desired, self.units[i].def.radius),
                );
            if clear {
                if let Some(cell) = self.grid.cell(desired.to_array()) {
                    for dz in -1..=1 {
                        for dx in -1..=1 {
                            if let Some(rows) = buckets.get(&[cell[0] + dx, cell[1] + dz]) {
                                for &j in rows {
                                    if i != j
                                        && dist(desired.to_array(), self.units[j].position)
                                            < self.units[i].def.radius
                                                + self.units[j].def.radius
                                                + 0.01
                                    {
                                        clear = false;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    clear = false;
                }
            }
            if clear {
                if let Some(scene) = &self.combat {
                    let steps = (desired.distance(current) / 0.08).ceil().max(1.0) as usize;
                    clear = (1..=steps).all(|k| {
                        scene.clear(
                            &self.units[i].id,
                            current.lerp(desired, k as f32 / steps as f32),
                            self.units[i].def.radius,
                        )
                    });
                }
            }
            if clear {
                if self.units[i].moving_to != Some(next) {
                    self.units[i].moving_to = Some(next);
                    let rows = buckets.entry(next).or_default();
                    if !rows.contains(&i) {
                        rows.push(i);
                    }
                }
                self.units[i].position = desired.to_array();
                if let Some(scene) = &mut self.combat {
                    scene.set_position(&self.units[i].id, desired);
                }
                self.units[i].yaw = delta.x.atan2(delta.y);
                self.units[i].stuck = 0;
                let cell = self.grid.cell(desired.to_array()).unwrap();
                if cell != old_cell {
                    if let Some(rows) = buckets.get_mut(&old_cell) {
                        rows.retain(|j| *j != i);
                    }
                    let rows = buckets.entry(cell).or_default();
                    if !rows.contains(&i) {
                        rows.push(i);
                    }
                }
                if desired.distance(target) < 0.001 {
                    self.units[i].path.pop_front();
                    self.units[i].moving_to = None;
                }
            } else {
                self.units[i].stuck += 1;
                if self.units[i].stuck > 45 && self.units[i].moving_to.is_none() {
                    if let Some(goal) = self.units[i].goal {
                        self.units[i].goal = None;
                        self.request_path(i, goal);
                        if let Some(job) = self.nav_queue.back_mut() {
                            job.avoid = true;
                        }
                        self.stats.replans += 1;
                    }
                }
            }
        }
    }
    pub fn step(&mut self, templates: &BTreeMap<String, (UnitDef, f32)>) {
        self.tick += 1;
        for u in &mut self.units {
            u.cooldown = u.cooldown.saturating_sub(1);
        }
        self.production(templates);
        if self.tick.is_multiple_of(self.session.nav_stride) {
            self.decision();
            self.navigate();
        }
        self.move_units();
        let player = self.session.player_faction;
        self.victory = self
            .buildings
            .iter()
            .any(|b| b.def.faction != player && b.health == 0.0);
        self.defeated = self
            .buildings
            .iter()
            .any(|b| b.def.faction == player && b.health == 0.0);
    }
    pub fn set_combat(&mut self, scene: Option<axiom_combat::Scene>) {
        if let Some(scene) = &scene {
            if self
                .combat
                .as_ref()
                .is_none_or(|old| old.config != scene.config)
            {
                let mut config = self.session.grid.clone();
                for z in 0..config.height as i32 {
                    for x in 0..config.width as i32 {
                        let center = Vec2::new(
                            config.origin[0] + (x as f32 + 0.5) * config.cell_size,
                            config.origin[1] + (z as f32 + 0.5) * config.cell_size,
                        );
                        if scene
                            .config
                            .obstacles
                            .iter()
                            .any(|o| !o.clear(center, config.cell_size * 0.49))
                            && !config.blocked.contains(&[x, z])
                        {
                            config.blocked.push([x, z]);
                        }
                    }
                }
                self.grid = nav::Grid::new(config).expect("validated shared navigation grid");
                self.cache.clear();
            }
        }
        self.combat = scene;
    }
    pub fn combat_bodies(&self) -> Vec<axiom_combat::Body> {
        self.units
            .iter()
            .map(|u| axiom_combat::Body {
                handle: axiom_combat::Handle {
                    id: u.id.clone(),
                    generation: 0,
                },
                owner: "rts.simulate".into(),
                position: Vec2::from_array(u.position),
                radius: u.def.radius,
                faction: u.def.faction,
                health: u.health,
                max_health: u.max_health,
                invulnerable: false,
            })
            .chain(self.buildings.iter().map(|b| axiom_combat::Body {
                handle: axiom_combat::Handle {
                    id: b.id.clone(),
                    generation: 0,
                },
                owner: "rts.simulate".into(),
                position: Vec2::from_array(b.position),
                radius: 1.0,
                faction: b.def.faction,
                health: b.health,
                max_health: b.health.max(1.0),
                invulnerable: false,
            }))
            .collect()
    }
    pub fn commit_combat(
        &mut self,
        scene: &axiom_combat::Scene,
        receipts: &[axiom_combat::Receipt],
    ) {
        let losses = self
            .units
            .iter()
            .map(|u| (&u.id, u.health))
            .chain(self.buildings.iter().map(|b| (&b.id, b.health)))
            .filter_map(|(id, h)| {
                scene
                    .bodies
                    .get(id)
                    .filter(|b| b.health < h)
                    .map(|b| (id.clone(), h - b.health))
            })
            .collect::<Vec<_>>();
        let kills = self.stats.kills;
        for (id, amount) in losses {
            self.damage(&id, amount);
        }
        self.stats.kills = kills
            + receipts
                .iter()
                .filter(|r| r.owner == "rts.simulate" && r.killed)
                .count() as u64;
        self.victory = self
            .buildings
            .iter()
            .any(|b| b.def.faction != self.session.player_faction && b.health == 0.0);
        self.defeated = self
            .buildings
            .iter()
            .any(|b| b.def.faction == self.session.player_faction && b.health == 0.0);
        self.combat = Some(scene.clone());
    }
    pub fn inspect(&self, args: InspectArgs) -> Result<Value> {
        ensure!(
            args.ids.len() <= 128 && args.fields.len() <= 16,
            "inspection budget exceeded"
        );
        let mut out = vec![];
        let rows = if args.ids.is_empty() {
            self.units.iter().collect::<Vec<_>>()
        } else {
            args.ids
                .iter()
                .map(|id| {
                    self.units
                        .iter()
                        .find(|u| &u.id == id)
                        .context("unit missing")
                })
                .collect::<Result<Vec<_>>>()?
        };
        let limit = args.limit.unwrap_or(64);
        ensure!(
            (1..=128).contains(&limit) && args.offset <= rows.len(),
            "invalid inspection page"
        );
        for u in rows.iter().skip(args.offset).take(limit) {
            let data = json!({"id":u.id,"position":u.position,"health":u.health,"faction":u.def.faction,"role":u.def.role,"order":u.order,"carried":u.carried});
            let fields = if args.fields.is_empty() {
                vec![
                    "id".into(),
                    "position".into(),
                    "faction".into(),
                    "role".into(),
                ]
            } else {
                args.fields.clone()
            };
            let mut row = serde_json::Map::new();
            for field in fields {
                row.insert(
                    field.clone(),
                    data.get(&field)
                        .context("unknown unit inspection field")?
                        .clone(),
                );
            }
            out.push(Value::Object(row));
        }
        let next = args.offset + out.len();
        Ok(
            json!({"units":out,"total":rows.len(),"tick":self.tick,"next_offset":if next<rows.len(){Some(next)}else{None}}),
        )
    }
    pub fn summary(&self) -> Value {
        json!({"tick":self.tick,"units_total":self.units.len(),"units_alive":self.units.iter().filter(|u|u.health>0.0).count(),"friendly_alive":self.units.iter().filter(|u|u.health>0.0&&u.def.faction==self.session.player_faction).count(),"selected":self.selected_ids().len(),"balance":self.balance,"queued_units":self.buildings.iter().map(|b|b.queue.len()).sum::<usize>(),"nav_queue":self.nav_queue.len(),"stats":self.stats,"remaining_resources":self.resources.iter().map(|r|r.remaining as u64).sum::<u64>(),"carried":self.units.iter().map(|u|u.carried as u64).sum::<u64>(),"last_error":self.last_error,"victory":self.victory,"defeated":self.defeated})
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.nav_queue.len() <= self.units.len(),
            "duplicate navigation jobs"
        );
        for u in &self.units {
            ensure!(
                u.position.iter().all(|v| v.is_finite()) && u.health.is_finite() && u.health >= 0.0,
                "non-finite RTS unit"
            );
            if u.health > 0.0 {
                ensure!(
                    self.combat.as_ref().map_or_else(
                        || self.grid.clear(u.position, u.def.radius),
                        |scene| scene.clear(&u.id, Vec2::from_array(u.position), u.def.radius)
                    ),
                    "unit intersects static navigation obstacle: {}",
                    u.id
                );
            }
        }
        let total = self.balance.iter().map(|v| *v as u64).sum::<u64>()
            + self
                .resources
                .iter()
                .map(|r| r.remaining as u64)
                .sum::<u64>()
            + self.units.iter().map(|u| u.carried as u64).sum::<u64>()
            + self.stats.spent
            + self.stats.lost;
        ensure!(
            total == self.initial_economy,
            "RTS resource conservation failed: {total} vs {}",
            self.initial_economy
        );
        Ok(())
    }
    pub fn overlaps(&self) -> usize {
        let mut n = 0;
        for (i, u) in self.units.iter().enumerate() {
            if u.health <= 0.0 {
                continue;
            }
            for other in &self.units[i + 1..] {
                if other.health > 0.0
                    && dist(u.position, other.position) < u.def.radius + other.def.radius - 0.001
                {
                    n += 1;
                }
            }
        }
        n
    }
    pub fn restore(&mut self, old: &Self) -> Result<()> {
        ensure!(
            self.session.grid == old.session.grid,
            "navigation grid changed; restart RTS to rebuild paths"
        );
        ensure!(
            old.units.len() <= self.session.max_units,
            "new population budget excludes existing units"
        );
        for unit in &mut self.units {
            if let Some(previous) = old.units.iter().find(|u| u.id == unit.id) {
                ensure!(
                    unit.def.faction == previous.def.faction,
                    "unit faction changed during simulation"
                );
                let def = unit.def.clone();
                let max = unit.max_health;
                *unit = previous.clone();
                unit.def = def;
                unit.health = (previous.health + max - previous.max_health).clamp(0.0, max);
                unit.max_health = max;
            }
        }
        for unit in &old.units {
            if let Some(prefab) = &unit.produced {
                ensure!(
                    self.session.recipes.values().any(|r| &r.prefab == prefab),
                    "produced unit prefab no longer available"
                );
                self.units.push(unit.clone());
            }
        }
        for building in &mut self.buildings {
            if let Some(previous) = old.buildings.iter().find(|b| b.id == building.id) {
                ensure!(
                    building.def == previous.def,
                    "active building contract changed; restart RTS"
                );
                for job in &previous.queue {
                    ensure!(
                        self.session.recipes.contains_key(&job.recipe),
                        "queued recipe removed"
                    );
                }
                building.health = previous.health;
                building.queue = previous.queue.clone();
            }
        }
        for resource in &mut self.resources {
            if let Some(previous) = old.resources.iter().find(|r| r.id == resource.id) {
                ensure!(
                    resource.position == previous.position,
                    "active resource moved; restart RTS"
                );
                resource.remaining = previous.remaining;
            }
        }
        self.balance = old.balance;
        self.initial_economy = old.initial_economy;
        self.stats = old.stats.clone();
        self.tick = old.tick;
        self.serial = old.serial;
        self.selected = old
            .selected
            .iter()
            .filter_map(|i| self.units.iter().position(|u| u.id == old.units[*i].id))
            .collect();
        self.nav_queue.clear();
        for i in 0..self.units.len() {
            let goal = self.units[i].goal;
            self.units[i].pending = false;
            self.units[i].path.clear();
            if let Some(goal) = goal {
                self.request_path(i, goal);
            }
        }
        Ok(())
    }
}
pub fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    Vec2::from_array(a).distance(Vec2::from_array(b))
}
