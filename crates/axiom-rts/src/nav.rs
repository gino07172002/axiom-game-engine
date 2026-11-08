use anyhow::{ensure, Result};
use axiom_core::grid_search::{GridSearch, Neighbourhood, Progress};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GridConfig {
    pub width: u32,
    pub height: u32,
    pub cell_size: f32,
    pub origin: [f32; 2],
    #[serde(default)]
    pub blocked: Vec<[i32; 2]>,
}
#[derive(Clone)]
pub struct Grid {
    pub config: GridConfig,
    blocked: Vec<bool>,
}
pub struct Search {
    pub path: Option<VecDeque<[i32; 2]>>,
    pub expanded: usize,
    pub exhausted: bool,
}
impl Grid {
    pub fn new(config: GridConfig) -> Result<Self> {
        ensure!(
            (4..=128).contains(&config.width)
                && (4..=128).contains(&config.height)
                && config.cell_size.is_finite()
                && (0.5..=4.0).contains(&config.cell_size)
                && config.origin.iter().all(|x| x.is_finite())
                && config.blocked.len() <= 16384,
            "invalid navigation grid"
        );
        let mut s = Self {
            blocked: vec![false; (config.width * config.height) as usize],
            config,
        };
        for cell in &s.config.blocked {
            let i = s
                .index(*cell)
                .ok_or_else(|| anyhow::anyhow!("blocked cell outside grid"))?;
            s.blocked[i] = true;
        }
        Ok(s)
    }
    pub fn index(&self, c: [i32; 2]) -> Option<usize> {
        (c[0] >= 0
            && c[1] >= 0
            && c[0] < self.config.width as i32
            && c[1] < self.config.height as i32)
            .then(|| (c[1] as u32 * self.config.width + c[0] as u32) as usize)
    }
    pub fn cell(&self, p: [f32; 2]) -> Option<[i32; 2]> {
        if !p.iter().all(|v| v.is_finite()) {
            return None;
        }
        let c = [
            ((p[0] - self.config.origin[0]) / self.config.cell_size).floor() as i32,
            ((p[1] - self.config.origin[1]) / self.config.cell_size).floor() as i32,
        ];
        self.index(c).map(|_| c)
    }
    pub fn center(&self, c: [i32; 2]) -> [f32; 2] {
        [
            self.config.origin[0] + (c[0] as f32 + 0.5) * self.config.cell_size,
            self.config.origin[1] + (c[1] as f32 + 0.5) * self.config.cell_size,
        ]
    }
    pub fn walkable(&self, c: [i32; 2]) -> bool {
        self.index(c).is_some_and(|i| !self.blocked[i])
    }
    pub fn occupied(
        &self,
        cells: impl Iterator<Item = [i32; 2]>,
        start: [i32; 2],
        goal: [i32; 2],
    ) -> Self {
        let mut grid = self.clone();
        for c in cells {
            if c != start && c != goal {
                if let Some(i) = grid.index(c) {
                    grid.blocked[i] = true;
                }
            }
        }
        grid
    }
    pub fn clear(&self, p: [f32; 2], radius: f32) -> bool {
        let Some(cell) = self.cell(p) else {
            return false;
        };
        for dz in -1..=1 {
            for dx in -1..=1 {
                let c = [cell[0] + dx, cell[1] + dz];
                if self.walkable(c) {
                    continue;
                }
                let center = self.center(c);
                let half = self.config.cell_size * 0.5;
                let nearest = [
                    p[0].clamp(center[0] - half, center[0] + half),
                    p[1].clamp(center[1] - half, center[1] + half),
                ];
                let d = (p[0] - nearest[0]).powi(2) + (p[1] - nearest[1]).powi(2);
                if d < radius * radius - 0.00001 {
                    return false;
                }
            }
        }
        true
    }
    pub fn nearest(&self, c: [i32; 2], radius: i32) -> Option<[i32; 2]> {
        if self.walkable(c) {
            return Some(c);
        }
        for r in 1..=radius {
            let mut best = None;
            let mut best_distance = i32::MAX;
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) == r {
                        let next = [c[0] + dx, c[1] + dz];
                        if self.walkable(next) {
                            let distance = dx * dx + dz * dz;
                            if distance < best_distance {
                                best = Some(next);
                                best_distance = distance;
                            }
                        }
                    }
                }
            }
            if best.is_some() {
                return best;
            }
        }
        None
    }
    /// Shortest 4-connected path (unit cost, Manhattan heuristic) expanding at most `budget`
    /// nodes. Runs on the shared `axiom_core::grid_search` engine; `exhausted` = the budget
    /// ran out before the goal was reached.
    pub fn search(&self, start: [i32; 2], goal: [i32; 2], budget: usize) -> Search {
        if !self.walkable(start) || !self.walkable(goal) {
            return Search {
                path: None,
                expanded: 0,
                exhausted: false,
            };
        }
        // Search buffers are reused per thread (generation stamps make reuse exact), so a
        // query allocates only its result.
        thread_local! {
            static SEARCH: std::cell::RefCell<Option<GridSearch>> = const { std::cell::RefCell::new(None) };
        }
        SEARCH.with(|cell| {
            let mut slot = cell.borrow_mut();
            let (w, h) = (self.config.width, self.config.height);
            let search = match slot.as_mut() {
                Some(s) if s.width() == w && s.height() == h => s,
                _ => slot.insert(GridSearch::new(w, h)),
            };
            search.begin(start, goal, Neighbourhood::Four);
            let progress = search.run(budget, |i| !self.blocked[i]);
            let path = (progress == Progress::Found).then(|| {
                let mut cells = vec![];
                search.path(&mut cells);
                VecDeque::from(cells)
            });
            Search {
                path,
                expanded: search.expanded(),
                exhausted: progress == Progress::Pending,
            }
        })
    }
}
