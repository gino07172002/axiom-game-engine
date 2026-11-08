//! Static navigation blockers shared by every planar mover (FSM agents, the action controller).
//! `nav.blocker {half:[hx,hz], offset?:[ox,oz]}` is an axis-aligned XZ rectangle centered on the
//! entity's authored position (plus offset). Rotation is ignored by design: blockers are cheap,
//! readable and good enough for buildings, benches and props.
use crate::CompiledEntity;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavBlocker {
    pub half: [f32; 2],
    #[serde(default)]
    pub offset: [f32; 2],
}

/// World XZ rectangle as (min, max).
pub type Rect2 = ([f32; 2], [f32; 2]);

impl NavBlocker {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.half
                .iter()
                .all(|h| h.is_finite() && (0.01..=500.0).contains(h))
                && self
                    .offset
                    .iter()
                    .all(|o| o.is_finite() && o.abs() <= 1000.0),
            "nav.blocker half 0.01..500, offset -1000..1000"
        );
        Ok(())
    }
    pub fn rect(&self, position: [f32; 3]) -> Rect2 {
        let (cx, cz) = (position[0] + self.offset[0], position[2] + self.offset[1]);
        (
            [cx - self.half[0], cz - self.half[1]],
            [cx + self.half[0], cz + self.half[1]],
        )
    }
}

/// All blocker rectangles in a compiled world (authored positions).
pub fn blockers(entities: &[CompiledEntity]) -> Result<Vec<Rect2>> {
    let mut out = vec![];
    for e in entities {
        if let Some(v) = e.components.get("nav.blocker") {
            let b: NavBlocker = serde_json::from_value(v.clone())?;
            let p = e.transform.as_ref().map_or([0.0; 3], |t| t.position);
            out.push(b.rect(p));
        }
    }
    ensure!(out.len() <= 512, "nav.blocker budget is 512");
    Ok(out)
}

/// Push a circle (center x,z; radius r) out of a rectangle. Returns the corrected center.
pub fn push_out(p: [f32; 2], r: f32, rect: &Rect2) -> [f32; 2] {
    let (min, max) = rect;
    let q = [p[0].clamp(min[0], max[0]), p[1].clamp(min[1], max[1])];
    let (dx, dz) = (p[0] - q[0], p[1] - q[1]);
    let d2 = dx * dx + dz * dz;
    if d2 >= r * r {
        return p;
    }
    if d2 > 1e-10 {
        let d = d2.sqrt();
        return [q[0] + dx / d * r, q[1] + dz / d * r];
    }
    // Center inside the rectangle: leave through the nearest edge.
    let exits = [
        (p[0] - min[0], [min[0] - r, p[1]]),
        (max[0] - p[0], [max[0] + r, p[1]]),
        (p[1] - min[1], [p[0], min[1] - r]),
        (max[1] - p[1], [p[0], max[1] + r]),
    ];
    exits
        .iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|e| e.1)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn circles_are_pushed_out_of_rectangles() {
        let rect: Rect2 = ([-1.0, -1.0], [1.0, 1.0]);
        let outside = push_out([3.0, 0.0], 0.5, &rect);
        assert_eq!(outside, [3.0, 0.0]);
        let touching = push_out([1.3, 0.0], 0.5, &rect);
        assert!((touching[0] - 1.5).abs() < 1e-5);
        let inside = push_out([0.8, 0.1], 0.5, &rect);
        assert!((inside[0] - 1.5).abs() < 1e-5 && (inside[1] - 0.1).abs() < 1e-5);
    }
}
