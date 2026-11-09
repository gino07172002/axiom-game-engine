//! Segment casts against static blocker shapes (oriented boxes and capsules), shared by the
//! follow camera and anything else that needs "how far can I go along this line".
//!
//! A sphere cast of radius `r` is a segment cast against each shape inflated by `r` (the
//! Minkowski sum). For capsules that is exact (radius + r). For boxes the inflation keeps
//! sharp edges and corners instead of rounding them, so near an edge the cast stops up to
//! `r * (sqrt(3) - 1)` early: conservative (never later than the true contact).
//!
//! Everything here is pure and allocation-free per query; [`BlockerGrid`] buckets shapes in
//! a uniform XZ grid built once so a cast tests only the shapes near the segment.
use glam::{Mat4, Vec3};

/// A static shape in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Oriented box: center, unit axes and half extents along them.
    Box {
        center: Vec3,
        axes: [Vec3; 3],
        half: Vec3,
    },
    /// Segment `a`..`b` swept by a sphere of `radius`.
    Capsule { a: Vec3, b: Vec3, radius: f32 },
}

/// Smallest root of `a t^2 + 2 b t + c = 0` that is >= 0 when the start is outside.
fn smaller_root(a: f32, b: f32, c: f32) -> Option<f32> {
    if a.abs() < 1e-12 {
        return None;
    }
    let h = b * b - a * c;
    if h < 0.0 {
        return None;
    }
    Some((-b - h.sqrt()) / a)
}

impl Shape {
    /// The unit cube (-0.5..0.5) placed by `m` (a mesh part's world matrix). Scale is part
    /// of the extents; shear (non-uniform scale under a rotated parent) is approximated by
    /// the matrix's column directions.
    pub fn unit_box(m: Mat4) -> Self {
        let columns = [
            m.x_axis.truncate(),
            m.y_axis.truncate(),
            m.z_axis.truncate(),
        ];
        let lengths = columns.map(Vec3::length);
        Self::Box {
            center: m.transform_point3(Vec3::ZERO),
            axes: [0, 1, 2].map(|i| columns[i].normalize_or(Vec3::AXES[i])),
            half: Vec3::new(lengths[0], lengths[1], lengths[2]) * 0.5,
        }
    }

    /// Axis-aligned box from world min / max corners.
    pub fn aligned(min: Vec3, max: Vec3) -> Self {
        Self::Box {
            center: (min + max) * 0.5,
            axes: Vec3::AXES,
            half: (max - min).abs() * 0.5,
        }
    }

    /// World-space bounds, grown by `inflate`.
    pub fn bounds(&self, inflate: f32) -> (Vec3, Vec3) {
        match *self {
            Self::Box { center, axes, half } => {
                let extent =
                    axes[0].abs() * half.x + axes[1].abs() * half.y + axes[2].abs() * half.z;
                let e = extent + Vec3::splat(inflate);
                (center - e, center + e)
            }
            Self::Capsule { a, b, radius } => {
                let r = Vec3::splat(radius + inflate);
                (a.min(b) - r, a.max(b) + r)
            }
        }
    }

    /// True when `p` lies inside the shape grown by `inflate` (boundary included).
    pub fn contains(&self, p: Vec3, inflate: f32) -> bool {
        match *self {
            Self::Box { center, axes, half } => {
                let d = p - center;
                (0..3).all(|i| d.dot(axes[i]).abs() <= half[i] + inflate)
            }
            Self::Capsule { a, b, radius } => {
                let ab = b - a;
                let t = if ab.length_squared() > 0.0 {
                    ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                p.distance_squared(a + ab * t) <= (radius + inflate) * (radius + inflate)
            }
        }
    }

    /// Entry parameter of `origin + t * segment` (t >= 0) into the shape grown by `inflate`,
    /// for an origin outside it. `None` when the line misses or the shape is behind.
    fn enter(&self, origin: Vec3, segment: Vec3, inflate: f32) -> Option<f32> {
        match *self {
            Self::Box { center, axes, half } => {
                let o = origin - center;
                let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
                for i in 0..3 {
                    let (oi, di, h) = (o.dot(axes[i]), segment.dot(axes[i]), half[i] + inflate);
                    if di.abs() < 1e-12 {
                        if oi.abs() > h {
                            return None;
                        }
                        continue;
                    }
                    let (a, b) = ((-h - oi) / di, (h - oi) / di);
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                    if near > far {
                        return None;
                    }
                }
                (far >= 0.0).then_some(near.max(0.0))
            }
            Self::Capsule { a, b, radius } => {
                let r = radius + inflate;
                let ab = b - a;
                let oa = origin - a;
                let (abab, abd, aboa) = (ab.dot(ab), ab.dot(segment), ab.dot(oa));
                let (dd, doa, oaoa) = (segment.dot(segment), segment.dot(oa), oa.dot(oa));
                let mut best: Option<f32> = None;
                let mut keep = |t: Option<f32>| {
                    if let Some(t) = t.filter(|t| *t >= 0.0) {
                        best = Some(best.map_or(t, |b: f32| b.min(t)));
                    }
                };
                // Cylinder body (only where the hit projects between the end points).
                let body = smaller_root(
                    abab * dd - abd * abd,
                    abab * doa - aboa * abd,
                    abab * oaoa - aboa * aboa - r * r * abab,
                );
                keep(body.filter(|t| {
                    let y = aboa + t * abd;
                    y >= 0.0 && y <= abab
                }));
                // End spheres.
                for c in [a, b] {
                    let oc = origin - c;
                    keep(smaller_root(dd, segment.dot(oc), oc.dot(oc) - r * r));
                }
                best
            }
        }
    }

    /// Where a sphere of radius `inflate` moving from `origin` along `segment` first touches
    /// this shape, as a fraction of the segment in 0..=1.
    ///
    /// An origin already inside the grown shape but outside the shape itself (standing
    /// closer than `inflate` to it) stops where the bare segment enters the shape, so moving
    /// away from a nearby wall is free. An origin inside the shape itself is never blocked
    /// by it (nothing sensible to stop at).
    pub fn cast(&self, origin: Vec3, segment: Vec3, inflate: f32) -> Option<f32> {
        let t = if self.contains(origin, inflate) {
            if self.contains(origin, 0.0) {
                return None;
            }
            self.enter(origin, segment, 0.0)?
        } else {
            self.enter(origin, segment, inflate)?
        };
        (t <= 1.0).then_some(t)
    }
}

/// Shapes whose XZ bounds span more cells than this are tested by every cast instead of
/// being copied into each cell (a large floor or boundary wall).
const BIG_SPAN_CELLS: i64 = 64;
/// Cell edge in meters. Cells are hashed into buckets, so the cell size does not depend on
/// how far apart the shapes are (a far-away shape cannot coarsen a dense area).
const CELL: f32 = 4.0;
/// A cast whose bounds cover more cells than this tests every shape instead (still within
/// its budget).
const MAX_QUERY_CELLS: i64 = 4096;

/// Static shapes bucketed by their XZ bounds in hashed 4 m cells (built once, read-only).
#[derive(Clone, Debug, Default)]
pub struct BlockerGrid {
    shapes: Vec<Shape>,
    /// Caller data per shape (for example the owning entity slot).
    owners: Vec<u32>,
    /// Bucket b holds `items[starts[b]..starts[b + 1]]` (power-of-two bucket count).
    starts: Vec<u32>,
    items: Vec<u32>,
    big: Vec<u32>,
    /// The largest inflation the bucketing accounts for.
    max_inflate: f32,
}

/// Per-caller scratch that keeps one shape from being tested twice in one cast.
#[derive(Clone, Debug, Default)]
pub struct CastScratch {
    marks: Vec<u32>,
    query: u32,
}

/// Result of [`BlockerGrid::cast`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cast {
    /// Nearest hit: segment fraction and the shape index.
    pub hit: Option<(f32, usize)>,
    /// Shapes tested (after the skip filter).
    pub tests: usize,
    /// More than the budget of shapes was near the segment; `hit` covers only those tested.
    pub over_budget: bool,
}

/// Cell coordinate of a world X or Z (clamped far beyond any authored position).
fn cell(v: f32) -> i64 {
    (v / CELL).floor().clamp(-1.0e9, 1.0e9) as i64
}

impl BlockerGrid {
    /// Buckets `shapes` (with one caller value each) for casts of inflation up to
    /// `max_inflate`.
    pub fn new(shapes: Vec<(Shape, u32)>, max_inflate: f32) -> Self {
        let mut grid = Self {
            max_inflate,
            ..Self::default()
        };
        let mut ranges = Vec::with_capacity(shapes.len());
        let mut inserts = 0usize;
        for (i, (shape, _)) in shapes.iter().enumerate() {
            let (a, b) = shape.bounds(max_inflate);
            let (x0, z0, x1, z1) = (cell(a.x), cell(a.z), cell(b.x), cell(b.z));
            if (x1 - x0 + 1).saturating_mul(z1 - z0 + 1) > BIG_SPAN_CELLS {
                grid.big.push(i as u32);
                ranges.push(None);
            } else {
                inserts += ((x1 - x0 + 1) * (z1 - z0 + 1)) as usize;
                ranges.push(Some((x0, z0, x1, z1)));
            }
        }
        (grid.shapes, grid.owners) = shapes.into_iter().unzip();
        let buckets = (inserts * 2).max(16).next_power_of_two();
        let mut counts = vec![0u32; buckets + 1];
        let cells = |r: &(i64, i64, i64, i64)| {
            let &(x0, z0, x1, z1) = r;
            (z0..=z1).flat_map(move |z| (x0..=x1).map(move |x| (x, z)))
        };
        for range in ranges.iter().flatten() {
            for (x, z) in cells(range) {
                counts[Self::bucket(x, z, buckets) + 1] += 1;
            }
        }
        for b in 0..buckets {
            counts[b + 1] += counts[b];
        }
        grid.items = vec![0; counts[buckets] as usize];
        let mut fill = counts.clone();
        for (i, range) in ranges.iter().enumerate() {
            for (x, z) in range.iter().flat_map(cells) {
                let b = Self::bucket(x, z, buckets);
                grid.items[fill[b] as usize] = i as u32;
                fill[b] += 1;
            }
        }
        grid.starts = counts;
        grid
    }

    fn bucket(x: i64, z: i64, buckets: usize) -> usize {
        let h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        ((h ^ (h >> 29)) as usize) & (buckets - 1)
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }
    pub fn shape(&self, i: usize) -> &Shape {
        &self.shapes[i]
    }
    pub fn owner(&self, i: usize) -> u32 {
        self.owners[i]
    }
    /// (buckets, shapes tested by every cast, cell size in meters).
    pub fn layout(&self) -> (usize, usize, f32) {
        (self.starts.len().saturating_sub(1), self.big.len(), CELL)
    }

    /// Nearest shape a sphere of radius `inflate` (at most the grid's `max_inflate`) touches
    /// moving from `origin` along `segment`, testing at most `budget` shapes. `skip(owner)`
    /// excludes shapes by their caller value. Does not allocate once `scratch` has grown to
    /// the shape count.
    pub fn cast(
        &self,
        origin: Vec3,
        segment: Vec3,
        inflate: f32,
        budget: usize,
        scratch: &mut CastScratch,
        skip: impl Fn(u32) -> bool,
    ) -> Cast {
        let mut out = Cast::default();
        if self.shapes.is_empty() {
            return out;
        }
        debug_assert!(inflate <= self.max_inflate + 1e-6);
        if scratch.marks.len() != self.shapes.len() {
            scratch.marks = vec![0; self.shapes.len()];
            scratch.query = 0;
        }
        scratch.query = scratch.query.wrapping_add(1);
        if scratch.query == 0 {
            scratch.marks.fill(0);
            scratch.query = 1;
        }
        let query = scratch.query;
        let mut test = |i: usize, out: &mut Cast| -> bool {
            if scratch.marks[i] == query {
                return true;
            }
            scratch.marks[i] = query;
            if skip(self.owners[i]) {
                return true;
            }
            if out.tests == budget {
                out.over_budget = true;
                return false;
            }
            out.tests += 1;
            if let Some(t) = self.shapes[i].cast(origin, segment, inflate) {
                if out.hit.is_none_or(|(best, _)| t < best) {
                    out.hit = Some((t, i));
                }
            }
            true
        };
        for &i in &self.big {
            if !test(i as usize, &mut out) {
                return out;
            }
        }
        let end = origin + segment;
        let (lo, hi) = (origin.min(end), origin.max(end));
        let r = self.max_inflate;
        let (x0, z0, x1, z1) = (
            cell(lo.x - r),
            cell(lo.z - r),
            cell(hi.x + r),
            cell(hi.z + r),
        );
        if (x1 - x0 + 1).saturating_mul(z1 - z0 + 1) > MAX_QUERY_CELLS {
            for i in 0..self.shapes.len() {
                if !test(i, &mut out) {
                    return out;
                }
            }
            return out;
        }
        let buckets = self.starts.len() - 1;
        for z in z0..=z1 {
            for x in x0..=x1 {
                let b = Self::bucket(x, z, buckets);
                for k in self.starts[b]..self.starts[b + 1] {
                    if !test(self.items[k as usize] as usize, &mut out) {
                        return out;
                    }
                }
            }
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn box_casts_stop_at_the_inflated_face_and_ignore_misses() {
        let wall = Shape::aligned(Vec3::new(-2.0, 0.0, 4.0), Vec3::new(2.0, 3.0, 4.2));
        let origin = Vec3::new(0.0, 1.0, 0.0);
        // Straight +Z, 10 m: the face is at z=4, a 0.25 m sphere stops at z=3.75.
        let t = wall.cast(origin, Vec3::Z * 10.0, 0.25).unwrap();
        assert!(close(t * 10.0, 3.75), "{t}");
        assert!(close(
            wall.cast(origin, Vec3::Z * 10.0, 0.0).unwrap() * 10.0,
            4.0
        ));
        // Too short to reach, pointing away, passing beside, passing over.
        assert_eq!(wall.cast(origin, Vec3::Z * 3.0, 0.25), None);
        assert_eq!(wall.cast(origin, -Vec3::Z * 10.0, 0.25), None);
        assert_eq!(
            wall.cast(Vec3::new(3.0, 1.0, 0.0), Vec3::Z * 10.0, 0.25),
            None
        );
        assert_eq!(
            wall.cast(Vec3::new(0.0, 4.0, 0.0), Vec3::Z * 10.0, 0.25),
            None
        );
        // Rotated box (unit_box of a matrix): 45 deg about Y, 2 m wide.
        let m = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
            Vec3::new(0.0, 0.0, 5.0),
        );
        let rotated = Shape::unit_box(m);
        let t = rotated.cast(Vec3::ZERO, Vec3::Z * 10.0, 0.0).unwrap();
        // Corner at distance sqrt(2) from the center toward -Z.
        assert!(close(t * 10.0, 5.0 - 2.0_f32.sqrt()), "{t}");
    }

    #[test]
    fn origins_near_or_inside_a_shape_are_handled() {
        let wall = Shape::aligned(Vec3::new(-2.0, 0.0, 4.0), Vec3::new(2.0, 3.0, 4.2));
        // 0.1 m from the face with a 0.25 m sphere: moving away is free, moving in stops at
        // the bare face.
        let near = Vec3::new(0.0, 1.0, 3.9);
        assert_eq!(wall.cast(near, -Vec3::Z * 5.0, 0.25), None);
        let t = wall.cast(near, Vec3::Z * 5.0, 0.25).unwrap();
        assert!(close(t * 5.0, 0.1), "{t}");
        // Inside the wall itself: never blocked by it.
        assert_eq!(
            wall.cast(Vec3::new(0.0, 1.0, 4.1), Vec3::Z * 5.0, 0.25),
            None
        );
    }

    #[test]
    fn capsule_casts_hit_the_body_and_the_caps() {
        let pillar = Shape::Capsule {
            a: Vec3::new(0.0, 0.5, 5.0),
            b: Vec3::new(0.0, 2.5, 5.0),
            radius: 0.5,
        };
        // Body: radius 0.5 + 0.25.
        let t = pillar
            .cast(Vec3::new(0.0, 1.0, 0.0), Vec3::Z * 10.0, 0.25)
            .unwrap();
        assert!(close(t * 10.0, 4.25), "{t}");
        // Straight down onto the top cap: cap center y 2.5, radius 0.75 -> y 3.25.
        let t = pillar
            .cast(Vec3::new(0.0, 6.0, 5.0), -Vec3::Y * 10.0, 0.25)
            .unwrap();
        assert!(close(6.0 - t * 10.0, 3.25), "{t}");
        // Miss beside it.
        assert_eq!(
            pillar.cast(Vec3::new(1.0, 1.0, 0.0), Vec3::Z * 10.0, 0.25),
            None
        );
        assert!(pillar.contains(Vec3::new(0.0, 1.0, 4.6), 0.0));
        assert!(!pillar.contains(Vec3::new(0.0, 1.0, 4.6), -0.2));
    }

    #[test]
    fn the_grid_finds_the_nearest_shape_dedupes_and_respects_the_budget() {
        // A row of 1 m boxes along +X every 2 m, plus one huge floor (tested by every cast).
        let mut shapes: Vec<(Shape, u32)> = (0..64)
            .map(|i| {
                let x = i as f32 * 2.0;
                (
                    Shape::aligned(Vec3::new(x, 0.0, 9.5), Vec3::new(x + 1.0, 2.0, 10.5)),
                    i,
                )
            })
            .collect();
        shapes.push((
            Shape::aligned(
                Vec3::new(-1000.0, -1.0, -1000.0),
                Vec3::new(1000.0, 0.0, 1000.0),
            ),
            99,
        ));
        let grid = BlockerGrid::new(shapes, 0.5);
        assert_eq!(grid.layout().1, 1, "the floor is tested by every cast");
        let mut scratch = CastScratch::default();
        let cast = grid.cast(
            Vec3::new(10.5, 1.0, 0.0),
            Vec3::Z * 20.0,
            0.25,
            64,
            &mut scratch,
            |_| false,
        );
        let (t, i) = cast.hit.unwrap();
        assert_eq!(grid.owner(i), 5);
        assert!(close(t * 20.0, 9.25));
        assert!(
            cast.tests < 10,
            "only nearby shapes are tested: {}",
            cast.tests
        );
        // Skipping the owner lets the cast through.
        let cast = grid.cast(
            Vec3::new(10.5, 1.0, 0.0),
            Vec3::Z * 20.0,
            0.25,
            64,
            &mut scratch,
            |o| o == 5,
        );
        assert_eq!(cast.hit, None);
        // A budget of one test stops after the floor and says so.
        let cast = grid.cast(
            Vec3::new(10.5, 1.0, 0.0),
            Vec3::Z * 20.0,
            0.25,
            1,
            &mut scratch,
            |_| false,
        );
        assert!(cast.over_budget && cast.tests == 1);
        // A diagonal sweep across the row sees each box once (no double counting).
        let cast = grid.cast(
            Vec3::new(-2.0, 1.0, 10.0),
            Vec3::X * 140.0,
            0.25,
            1000,
            &mut scratch,
            |_| false,
        );
        assert_eq!(cast.tests, 65);
        assert_eq!(grid.owner(cast.hit.unwrap().1), 0);
        // A dense 1 m field plus one shape 5 km away: the far shape does not coarsen the
        // cells, so a short cast in the field tests only its neighbourhood.
        let mut field: Vec<(Shape, u32)> = (0..900)
            .map(|i| {
                let (x, z) = ((i % 30) as f32, (i / 30) as f32);
                let c = Vec3::new(x, 1.0, z);
                (
                    Shape::aligned(c - Vec3::splat(0.1), c + Vec3::splat(0.1)),
                    i,
                )
            })
            .collect();
        field.push((
            Shape::aligned(
                Vec3::new(5000.0, 0.0, 5000.0),
                Vec3::new(5001.0, 1.0, 5001.0),
            ),
            900,
        ));
        let grid = BlockerGrid::new(field, 0.2);
        let cast = grid.cast(
            Vec3::new(10.5, 1.0, 10.0),
            Vec3::new(1.0, 0.0, 0.0),
            0.2,
            512,
            &mut scratch,
            |_| false,
        );
        assert!(!cast.over_budget && cast.tests < 120, "{}", cast.tests);
        assert_eq!(grid.owner(cast.hit.unwrap().1), 311);
    }
}
