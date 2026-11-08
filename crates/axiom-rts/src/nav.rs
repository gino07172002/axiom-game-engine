use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
};
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
    pub fn search(&self, start: [i32; 2], goal: [i32; 2], budget: usize) -> Search {
        let fail = Search {
            path: None,
            expanded: 0,
            exhausted: false,
        };
        if !self.walkable(start) || !self.walkable(goal) {
            return fail;
        }
        let from = self.index(start).unwrap();
        let target = self.index(goal).unwrap();
        let mut distance = vec![u32::MAX; self.blocked.len()];
        let mut parent = vec![usize::MAX; self.blocked.len()];
        let mut open = BinaryHeap::new();
        distance[from] = 0;
        let h = |c: [i32; 2]| (c[0] - goal[0]).unsigned_abs() + (c[1] - goal[1]).unsigned_abs();
        open.push(Reverse((h(start), 0, from)));
        let mut expanded = 0;
        while let Some(Reverse((_, cost, i))) = open.pop() {
            if cost != distance[i] {
                continue;
            }
            if i == target {
                let mut path = VecDeque::new();
                let mut current = i;
                while current != from {
                    path.push_front([
                        (current as u32 % self.config.width) as i32,
                        (current as u32 / self.config.width) as i32,
                    ]);
                    current = parent[current];
                }
                return Search {
                    path: Some(path),
                    expanded,
                    exhausted: false,
                };
            }
            if expanded == budget {
                return Search {
                    path: None,
                    expanded,
                    exhausted: true,
                };
            }
            expanded += 1;
            let c = [
                (i as u32 % self.config.width) as i32,
                (i as u32 / self.config.width) as i32,
            ];
            for d in [[1, 0], [0, 1], [-1, 0], [0, -1]] {
                let n = [c[0] + d[0], c[1] + d[1]];
                if !self.walkable(n) {
                    continue;
                }
                let ni = self.index(n).unwrap();
                let next = cost + 1;
                if next < distance[ni] {
                    distance[ni] = next;
                    parent[ni] = i;
                    open.push(Reverse((next + h(n), next, ni)));
                }
            }
        }
        Search {
            path: None,
            expanded,
            exhausted: false,
        }
    }
}
