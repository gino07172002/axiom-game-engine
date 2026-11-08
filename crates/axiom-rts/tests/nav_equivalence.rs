//! Characterization: `Grid::search` runs on the shared `axiom_core::grid_search` engine. This
//! keeps a verbatim copy of the previous RTS implementation and checks both return the same
//! path, expansion count and exhaustion flag on many seeded random grids and budgets.
use axiom_rts::nav::{Grid, GridConfig, Search};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
};

/// The RTS A* before the shared engine (copied unchanged apart from `self` -> `grid`).
fn previous(grid: &Grid, start: [i32; 2], goal: [i32; 2], budget: usize) -> Search {
    let fail = Search {
        path: None,
        expanded: 0,
        exhausted: false,
    };
    if !grid.walkable(start) || !grid.walkable(goal) {
        return fail;
    }
    let cells = (grid.config.width * grid.config.height) as usize;
    let from = grid.index(start).unwrap();
    let target = grid.index(goal).unwrap();
    let mut distance = vec![u32::MAX; cells];
    let mut parent = vec![usize::MAX; cells];
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
                    (current as u32 % grid.config.width) as i32,
                    (current as u32 / grid.config.width) as i32,
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
            (i as u32 % grid.config.width) as i32,
            (i as u32 / grid.config.width) as i32,
        ];
        for d in [[1, 0], [0, 1], [-1, 0], [0, -1]] {
            let n = [c[0] + d[0], c[1] + d[1]];
            if !grid.walkable(n) {
                continue;
            }
            let ni = grid.index(n).unwrap();
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

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> i32 {
        (self.next() % n) as i32
    }
}

#[test]
fn shared_engine_matches_the_previous_rts_search() {
    let mut rng = Rng(7);
    let mut compared = 0;
    let mut found = 0;
    let mut exhausted = 0;
    for _ in 0..400 {
        let width = 4 + rng.below(60) as u32;
        let height = 4 + rng.below(60) as u32;
        let density = rng.below(45) as u64;
        let mut blocked = vec![];
        for z in 0..height as i32 {
            for x in 0..width as i32 {
                if (rng.next() % 100) < density {
                    blocked.push([x, z]);
                }
            }
        }
        let grid = Grid::new(GridConfig {
            width,
            height,
            cell_size: 1.0,
            origin: [0.0, 0.0],
            blocked,
        })
        .unwrap();
        for _ in 0..8 {
            let start = [rng.below(width as u64), rng.below(height as u64)];
            let goal = [rng.below(width as u64), rng.below(height as u64)];
            let budget = match rng.below(4) {
                0 => 0,
                1 => rng.below(16) as usize,
                2 => rng.below(400) as usize,
                _ => 4096,
            };
            let a = previous(&grid, start, goal, budget);
            let b = grid.search(start, goal, budget);
            assert_eq!(
                (&a.path, a.expanded, a.exhausted),
                (&b.path, b.expanded, b.exhausted),
                "{width}x{height} {start:?}->{goal:?} budget {budget}"
            );
            compared += 1;
            found += usize::from(a.path.is_some());
            exhausted += usize::from(a.exhausted);
        }
    }
    // The sample covers found, exhausted and unreachable outcomes.
    assert!(found > 500 && exhausted > 200 && compared - found - exhausted > 100);
}
