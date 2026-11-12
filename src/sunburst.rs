//! Sunburst (ring chart) layout: the zoomed folder in the middle, each ring one level deeper,
//! each entry's angle proportional to its size.

use crate::scan::Node;
use std::f32::consts::TAU;

pub struct Seg {
    /// Child indices from the chart's root.
    pub path: Vec<u32>,
    /// Ring number, 1 = innermost.
    pub ring: u16,
    /// Start and end angle in radians, clockwise from 12 o'clock.
    pub a0: f32,
    pub a1: f32,
}

/// Lay out up to `max_rings` levels below `root`. Entries thinner than `min_angle` are left
/// out (their space shows as a gap), which keeps the chart readable and cheap to draw.
pub fn layout(root: &Node, max_rings: u16, min_angle: f32) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    add(root, 0.0, TAU, 1, max_rings, min_angle, &mut path, &mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn add(n: &Node, a0: f32, a1: f32, ring: u16, max: u16, min_angle: f32, path: &mut Vec<u32>, out: &mut Vec<Seg>) {
    if ring > max || n.size == 0 {
        return;
    }
    let scale = (a1 - a0) / n.size as f32;
    let mut a = a0;
    for (i, c) in n.children.iter().enumerate() {
        let span = c.size as f32 * scale;
        if span < min_angle {
            break; // children are sorted largest first, so the rest are smaller still
        }
        path.push(i as u32);
        out.push(Seg { path: path.clone(), ring, a0: a, a1: a + span });
        if c.is_dir {
            add(c, a, a + span, ring + 1, max, min_angle, path, out);
        }
        path.pop();
        a += span;
    }
}

/// Angle of a point relative to the centre, clockwise from 12 o'clock, in 0..TAU.
pub fn angle_of(dx: f32, dy: f32) -> f32 {
    let a = dx.atan2(-dy);
    if a < 0.0 { a + TAU } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: u64) -> Node {
        Node { name: "f".into(), size, files: 1, modified: 0, is_dir: false, children: vec![] }
    }

    #[test]
    fn angles_follow_sizes() {
        let sub = Node { name: "d".into(), size: 50, files: 2, modified: 0, is_dir: true, children: vec![file(30), file(20)] };
        let root = Node { name: "r".into(), size: 100, files: 3, modified: 0, is_dir: true, children: vec![sub, file(50)] };
        let segs = layout(&root, 5, 0.0);
        assert_eq!(segs.len(), 4);
        assert!((segs[0].a1 - TAU / 2.0).abs() < 1e-4);
        assert_eq!(segs[1].ring, 2);
        assert!((segs[1].a1 - TAU * 0.3).abs() < 1e-4);
        assert!((angle_of(1.0, 0.0) - TAU / 4.0).abs() < 1e-4); // 3 o'clock
    }
}
