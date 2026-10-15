//! Squarified treemap layout (Bruls, Huizing, van Wijk 2000).

use crate::scan::Node;
use eframe::egui::{pos2, Rect};

pub struct Tile {
    pub rect: Rect,
    /// Child-index path from the layout root.
    pub path: Vec<u32>,
    pub is_dir: bool,
}

/// Lay out `root` into `bounds`. Tiles are emitted parents-before-children, so later tiles
/// draw on top. Anything smaller than about a pixel is dropped.
pub fn layout(root: &Node, bounds: Rect) -> Vec<Tile> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    place(root, bounds, &mut path, &mut out);
    out
}

fn place(node: &Node, rect: Rect, path: &mut Vec<u32>, out: &mut Vec<Tile>) {
    out.push(Tile { rect, path: path.clone(), is_dir: node.is_dir });
    if !node.is_dir || rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let n = node.children.iter().take_while(|c| c.size > 0).count();
    if n == 0 {
        return;
    }
    let values: Vec<f64> = node.children[..n].iter().map(|c| c.size as f64).collect();
    let rects = squarify(&values, rect);
    for (i, r) in rects.into_iter().enumerate() {
        if r.width() * r.height() < 1.0 {
            continue;
        }
        path.push(i as u32);
        place(&node.children[i], r, path, out);
        path.pop();
    }
}

/// Split `rect` into one rectangle per value (values must be sorted descending), keeping
/// aspect ratios close to 1. Returns rects in the same order as `values`.
pub fn squarify(values: &[f64], rect: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(values.len());
    let mut remaining: f64 = values.iter().sum();
    if remaining <= 0.0 {
        return vec![Rect::from_min_size(rect.min, eframe::egui::Vec2::ZERO); values.len()];
    }
    let (mut x, mut y) = (rect.min.x as f64, rect.min.y as f64);
    let (mut w, mut h) = (rect.width() as f64, rect.height() as f64);
    let mut i = 0;
    while i < values.len() {
        let scale = (w * h) / remaining;
        let short = w.min(h);
        // Grow the row while the worst aspect ratio keeps improving.
        let mut j = i;
        let mut sum = 0.0;
        let mut best = f64::INFINITY;
        while j < values.len() {
            let a = values[j] * scale;
            let s = sum + a;
            let rmax = values[i] * scale;
            let worst = worst_ratio(rmax, a, s, short);
            if worst > best {
                break;
            }
            best = worst;
            sum = s;
            j += 1;
        }
        let row_vals = sum / scale;
        if sum <= 0.0 || short <= 0.0 {
            for _ in i..values.len() {
                out.push(Rect::from_min_size(pos2(x as f32, y as f32), eframe::egui::Vec2::ZERO));
            }
            break;
        }
        let thick = sum / short;
        let mut off = 0.0;
        for v in &values[i..j] {
            let len = v * scale / thick;
            let r = if w >= h {
                // Column on the left, items stacked vertically.
                Rect::from_min_max(
                    pos2(x as f32, (y + off) as f32),
                    pos2((x + thick) as f32, (y + off + len) as f32),
                )
            } else {
                // Row on top, items side by side.
                Rect::from_min_max(
                    pos2((x + off) as f32, y as f32),
                    pos2((x + off + len) as f32, (y + thick) as f32),
                )
            };
            out.push(r);
            off += len;
        }
        if w >= h {
            x += thick;
            w -= thick;
        } else {
            y += thick;
            h -= thick;
        }
        remaining -= row_vals;
        i = j;
    }
    out
}

fn worst_ratio(rmax: f64, rmin: f64, sum: f64, short: f64) -> f64 {
    let s2 = short * short;
    let sum2 = sum * sum;
    (s2 * rmax / sum2).max(sum2 / (s2 * rmin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_preserves_area() {
        let vals = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let r = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(6.0, 4.0));
        let rects = squarify(&vals, r);
        assert_eq!(rects.len(), vals.len());
        for (v, rr) in vals.iter().zip(&rects) {
            let area = (rr.width() * rr.height()) as f64;
            assert!((area - v).abs() < 1e-3, "{area} vs {v}");
            assert!(r.expand(1e-3).contains_rect(*rr));
        }
    }
}
