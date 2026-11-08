//! Resumable A* over a uniform planar grid, shared by RTS unit navigation (`axiom_rts::nav`)
//! and generic navigation agents (`nav.agent`, see `crate::navigation`).
//!
//! The searcher owns its buffers and reuses them across searches: per-cell cost and parent
//! arrays are valid only where the cell's stamp equals the current search generation, so
//! starting a search never clears memory, and the open heap keeps its capacity. A search can
//! be advanced in slices (`run` with a node budget) and resumed on a later frame; slicing does
//! not change the result.
//!
//! Ordering is fully deterministic: the open list pops the smallest (f, tie, cell index), so
//! two searches over the same grid expand the same nodes in the same order. `tie` is g in the
//! four-neighbour mode (the original RTS order, kept bit for bit) and `u32::MAX - g` in the
//! eight-neighbour mode, which prefers the deeper of equal-f nodes and expands far fewer
//! nodes across open ground.
use std::{cmp::Reverse, collections::BinaryHeap};

/// Which neighbours a cell has and what a step costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Neighbourhood {
    /// Right, up, left, down; unit cost; Manhattan heuristic.
    Four,
    /// The four sides (cost 10) and diagonals (cost 14, only when both adjacent sides are
    /// walkable, so paths never cut a blocked corner); octile heuristic.
    Eight,
}

/// Outcome of `GridSearch::run`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The goal was reached; read it with `path`.
    Found,
    /// The open list ran empty: no path exists.
    Unreachable,
    /// The node budget of this call ran out; call `run` again to continue.
    Pending,
}

const SIDES: [[i32; 2]; 4] = [[1, 0], [0, 1], [-1, 0], [0, -1]];
const CORNERS: [[i32; 2]; 4] = [[1, 1], [-1, 1], [-1, -1], [1, -1]];

/// Search state of one cell; valid only when `stamp` equals the search generation.
#[derive(Clone, Copy, Default)]
struct Visit {
    stamp: u32,
    cost: u32,
    parent: u32,
}

pub struct GridSearch {
    width: u32,
    height: u32,
    /// Per cell, in one array so a visit touches one cache line.
    cells: Vec<Visit>,
    generation: u32,
    /// (f, tie, cell index); see the module notes for `tie`.
    open: BinaryHeap<Reverse<(u32, u32, u32)>>,
    start: u32,
    goal: u32,
    goal_cell: [i32; 2],
    neighbourhood: Neighbourhood,
    expanded: usize,
    done: Option<Progress>,
}

impl GridSearch {
    /// Buffers for a `width` x `height` grid (12 bytes per cell, allocated once).
    pub fn new(width: u32, height: u32) -> Self {
        let cells = width as usize * height as usize;
        Self {
            width,
            height,
            cells: vec![Visit::default(); cells],
            generation: 0,
            open: BinaryHeap::new(),
            start: 0,
            goal: 0,
            goal_cell: [0, 0],
            neighbourhood: Neighbourhood::Four,
            expanded: 0,
            done: Some(Progress::Unreachable),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn index(&self, c: [i32; 2]) -> Option<usize> {
        (c[0] >= 0 && c[1] >= 0 && c[0] < self.width as i32 && c[1] < self.height as i32)
            .then(|| c[1] as usize * self.width as usize + c[0] as usize)
    }

    pub fn cell(&self, index: usize) -> [i32; 2] {
        [
            (index % self.width as usize) as i32,
            (index / self.width as usize) as i32,
        ]
    }

    fn heuristic(&self, c: [i32; 2]) -> u32 {
        let dx = (c[0] - self.goal_cell[0]).unsigned_abs();
        let dz = (c[1] - self.goal_cell[1]).unsigned_abs();
        match self.neighbourhood {
            Neighbourhood::Four => dx + dz,
            Neighbourhood::Eight => 10 * dx.max(dz) + 4 * dx.min(dz),
        }
    }

    /// Heap tie-break key for cost `g` (and back: the mapping is its own inverse).
    fn tie(&self, g: u32) -> u32 {
        match self.neighbourhood {
            Neighbourhood::Four => g,
            Neighbourhood::Eight => u32::MAX - g,
        }
    }

    fn known(&self, i: usize) -> u32 {
        let v = self.cells[i];
        if v.stamp == self.generation {
            v.cost
        } else {
            u32::MAX
        }
    }

    /// Starts a new search from `start` to `goal`. The caller checks that both cells are
    /// inside the grid and walkable; out-of-range cells make the search unreachable.
    pub fn begin(&mut self, start: [i32; 2], goal: [i32; 2], neighbourhood: Neighbourhood) {
        self.open.clear();
        self.expanded = 0;
        self.neighbourhood = neighbourhood;
        let (Some(from), Some(to)) = (self.index(start), self.index(goal)) else {
            self.done = Some(Progress::Unreachable);
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            // Stamps wrapped: forget every stale entry once every 2^32 searches.
            self.cells.fill(Visit::default());
            self.generation = 1;
        }
        self.start = from as u32;
        self.goal = to as u32;
        self.goal_cell = goal;
        self.done = None;
        self.cells[from] = Visit {
            stamp: self.generation,
            cost: 0,
            parent: from as u32,
        };
        self.open
            .push(Reverse((self.heuristic(start), self.tie(0), from as u32)));
    }

    /// Nodes expanded by the current search so far.
    pub fn expanded(&self) -> usize {
        self.expanded
    }

    /// Whether the current search has finished (`Found` or `Unreachable`).
    pub fn finished(&self) -> Option<Progress> {
        self.done
    }

    /// Expands at most `budget` more nodes. `walkable(index)` decides which cells may be
    /// entered; it must not change while a search is in progress.
    pub fn run(&mut self, budget: usize, walkable: impl Fn(usize) -> bool) -> Progress {
        if let Some(done) = self.done {
            return done;
        }
        let mut used = 0;
        while let Some(Reverse((f, tie, i))) = self.open.pop() {
            let index = i as usize;
            let cost = self.tie(tie);
            if cost != self.known(index) {
                continue;
            }
            if i == self.goal {
                self.done = Some(Progress::Found);
                return Progress::Found;
            }
            if used == budget {
                // Put it back: it is the minimum, so the next call pops it first.
                self.open.push(Reverse((f, tie, i)));
                return Progress::Pending;
            }
            used += 1;
            self.expanded += 1;
            let c = self.cell(index);
            let (side, corner) = match self.neighbourhood {
                Neighbourhood::Four => (1, 0),
                Neighbourhood::Eight => (10, 14),
            };
            let mut open_side = [false; 4];
            for (k, d) in SIDES.iter().enumerate() {
                let n = [c[0] + d[0], c[1] + d[1]];
                let Some(ni) = self.index(n).filter(|&ni| walkable(ni)) else {
                    continue;
                };
                open_side[k] = true;
                self.relax(index, ni, n, cost + side);
            }
            if corner > 0 {
                for (k, d) in CORNERS.iter().enumerate() {
                    // Corner k lies between sides k and k + 1 (right/up, up/left, ...).
                    if !(open_side[k] && open_side[(k + 1) % 4]) {
                        continue;
                    }
                    let n = [c[0] + d[0], c[1] + d[1]];
                    let Some(ni) = self.index(n).filter(|&ni| walkable(ni)) else {
                        continue;
                    };
                    self.relax(index, ni, n, cost + corner);
                }
            }
        }
        self.done = Some(Progress::Unreachable);
        Progress::Unreachable
    }

    fn relax(&mut self, from: usize, to: usize, cell: [i32; 2], next: u32) {
        if next < self.known(to) {
            self.cells[to] = Visit {
                stamp: self.generation,
                cost: next,
                parent: from as u32,
            };
            let f = next + self.heuristic(cell);
            let tie = self.tie(next);
            self.open.push(Reverse((f, tie, to as u32)));
        }
    }

    /// After `Found`: the cells from the start (excluded) to the goal (included), appended to
    /// `out` (cleared first). Empty when start == goal.
    pub fn path(&self, out: &mut Vec<[i32; 2]>) {
        out.clear();
        if self.done != Some(Progress::Found) {
            return;
        }
        let mut current = self.goal as usize;
        while current != self.start as usize {
            out.push(self.cell(current));
            current = self.cells[current].parent as usize;
        }
        out.reverse();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_grid(blocked: &[[i32; 2]], w: u32) -> impl Fn(usize) -> bool + '_ {
        move |i| {
            let c = [(i % w as usize) as i32, (i / w as usize) as i32];
            !blocked.contains(&c)
        }
    }

    #[test]
    fn finds_paths_around_walls_in_both_neighbourhoods() {
        // A wall at x = 3 from z = 0..=4 on an 8 x 8 grid.
        let wall: Vec<[i32; 2]> = (0..5).map(|z| [3, z]).collect();
        let mut s = GridSearch::new(8, 8);
        let mut out = vec![];
        for n in [Neighbourhood::Four, Neighbourhood::Eight] {
            s.begin([1, 1], [5, 1], n);
            assert_eq!(s.run(usize::MAX, open_grid(&wall, 8)), Progress::Found);
            s.path(&mut out);
            assert_eq!(*out.last().unwrap(), [5, 1]);
            assert!(out.iter().all(|c| !wall.contains(c)));
            assert!(
                out.iter().any(|c| c[1] >= 5),
                "goes round the wall: {out:?}"
            );
            // Consecutive cells are neighbours; diagonals never cut a blocked corner.
            let mut prev = [1, 1];
            for c in &out {
                let (dx, dz) = (c[0] - prev[0], c[1] - prev[1]);
                assert!(dx.abs() <= 1 && dz.abs() <= 1 && (dx, dz) != (0, 0));
                if dx != 0 && dz != 0 {
                    assert_eq!(n, Neighbourhood::Eight);
                    assert!(!wall.contains(&[prev[0] + dx, prev[1]]));
                    assert!(!wall.contains(&[prev[0], prev[1] + dz]));
                }
                prev = *c;
            }
        }
    }

    #[test]
    fn slicing_does_not_change_the_result_and_buffers_are_reused() {
        let wall: Vec<[i32; 2]> = (0..30).map(|z| [16, z]).collect();
        let mut whole = GridSearch::new(32, 32);
        whole.begin([2, 2], [30, 2], Neighbourhood::Eight);
        assert_eq!(whole.run(usize::MAX, open_grid(&wall, 32)), Progress::Found);
        let mut expected = vec![];
        whole.path(&mut expected);
        let mut sliced = GridSearch::new(32, 32);
        let mut got = vec![];
        for _ in 0..3 {
            sliced.begin([2, 2], [30, 2], Neighbourhood::Eight);
            let mut calls = 0;
            while sliced.run(7, open_grid(&wall, 32)) == Progress::Pending {
                calls += 1;
            }
            assert!(calls > 10);
            sliced.path(&mut got);
            assert_eq!(got, expected);
            assert_eq!(sliced.expanded(), whole.expanded());
        }
        // Unreachable: the goal is walled in.
        let ring = [[9, 10], [11, 10], [10, 9], [10, 11]];
        sliced.begin([2, 2], [10, 10], Neighbourhood::Four);
        assert_eq!(
            sliced.run(usize::MAX, open_grid(&ring, 32)),
            Progress::Unreachable
        );
        sliced.path(&mut got);
        assert!(got.is_empty());
        // Out-of-range cells are unreachable without expanding anything.
        sliced.begin([2, 2], [40, 2], Neighbourhood::Four);
        assert_eq!(sliced.run(10, |_| true), Progress::Unreachable);
        assert_eq!(sliced.expanded(), 0);
        // Same cell: found at once with an empty path.
        sliced.begin([3, 3], [3, 3], Neighbourhood::Eight);
        assert_eq!(sliced.run(0, |_| true), Progress::Found);
        sliced.path(&mut got);
        assert!(got.is_empty());
    }
}
