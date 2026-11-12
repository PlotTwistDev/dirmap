//! Squarified treemap layout (Bruls, Huizing, van Wijk 2000).
//!
//! The f32 `layout` below is the original whole-map, pixel-based layout; it is kept for the
//! tests and the before/after benchmark. The app uses the f64 functions further down.

use crate::scan::Node;
#[cfg(test)]
use eframe::egui::{pos2, Rect};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[cfg(test)]
pub struct Tile {
    pub rect: Rect,
    /// Child-index path from the layout root.
    pub path: Vec<u32>,
    #[allow(dead_code)]
    pub is_dir: bool,
    /// Cushion surface coefficients (x², y², x, y) accumulated from every ancestor,
    /// as in van Wijk & van de Wetering's cushion treemaps. Shading comes from these.
    pub surface: [f32; 4],
}

/// Lay out `root` into `bounds`. Tiles are emitted parents-before-children, so later tiles
/// draw on top. Anything smaller than about a pixel is dropped. `height` is the cushion
/// ridge height at the top level and `scale` how much it shrinks per level.
#[cfg(test)]
pub fn layout(root: &Node, bounds: Rect, height: f32, scale: f32) -> Vec<Tile> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    place(root, bounds, &mut path, [0.0; 4], height, scale, &mut out);
    out
}

#[cfg(test)]
fn add_ridge(x1: f32, x2: f32, h: f32, lin: &mut f32, sq: &mut f32) {
    if x2 - x1 <= 0.0 {
        return;
    }
    let h4 = 4.0 * h / (x2 - x1);
    *lin += h4 * (x2 + x1);
    *sq -= h4;
}

#[cfg(test)]
fn place(node: &Node, rect: Rect, path: &mut Vec<u32>, mut surface: [f32; 4], h: f32, f: f32, out: &mut Vec<Tile>) {
    if h > 0.0 {
        let [sq_x, sq_y, lin_x, lin_y] = &mut surface;
        add_ridge(rect.min.x, rect.max.x, h, lin_x, sq_x);
        add_ridge(rect.min.y, rect.max.y, h, lin_y, sq_y);
    }
    out.push(Tile { rect, path: path.clone(), is_dir: node.is_dir, surface });
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
        place(&node.children[i], r, path, surface, h * f, f, out);
        path.pop();
    }
}

/// Split `rect` into one rectangle per value (values must be sorted descending), keeping
/// aspect ratios close to 1. Returns rects in the same order as `values`.
#[cfg(test)]
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

// ---------- double-precision layout for the camera-based view ----------
//
// The camera can zoom millions of times into the layout, so the world geometry is kept in f64.
// Layout is scale-invariant (squarify only depends on aspect ratio, and the cushion gradients are
// dimensionless), so a folder's layout is computed once in world units and the camera just moves
// over it. Only the tiles inside the view are generated, down to about a pixel at the current
// scale (level of detail), so the cost follows what is on screen rather than the number of files.

/// An axis-aligned rectangle in world units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DRect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl DRect {
    pub const fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x0, y0, x1, y1 }
    }
    pub fn from_size(w: f64, h: f64) -> Self {
        Self::new(0.0, 0.0, w, h)
    }
    pub fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn center(&self) -> (f64, f64) {
        ((self.x0 + self.x1) * 0.5, (self.y0 + self.y1) * 0.5)
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
    pub fn contains_rect(&self, o: &DRect) -> bool {
        o.x0 >= self.x0 && o.x1 <= self.x1 && o.y0 >= self.y0 && o.y1 <= self.y1
    }
    pub fn intersects(&self, o: &DRect) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
    pub fn intersect(&self, o: &DRect) -> DRect {
        DRect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1))
    }
    /// Grow by `fx` of the width on the left and right and `fy` of the height above and below.
    pub fn grow(&self, fx: f64, fy: f64) -> DRect {
        let (dx, dy) = (self.w() * fx, self.h() * fy);
        DRect::new(self.x0 - dx, self.y0 - dy, self.x1 + dx, self.y1 + dy)
    }
    #[cfg(test)]
    pub fn to_f32(self) -> Rect {
        Rect::from_min_max(pos2(self.x0 as f32, self.y0 as f32), pos2(self.x1 as f32, self.y1 as f32))
    }
}

/// Cushion surface coefficients (x², y², x, y), as in `Tile::surface`, in world units.
pub type Surface = [f64; 4];

fn add_ridge64(x1: f64, x2: f64, h: f64, lin: &mut f64, sq: &mut f64) {
    if x2 - x1 <= 0.0 {
        return;
    }
    let h4 = 4.0 * h / (x2 - x1);
    *lin += h4 * (x2 + x1);
    *sq -= h4;
}

fn with_ridges(mut s: Surface, r: &DRect, h: f64) -> Surface {
    if h > 0.0 {
        let [sq_x, sq_y, lin_x, lin_y] = &mut s;
        add_ridge64(r.x0, r.x1, h, lin_x, sq_x);
        add_ridge64(r.y0, r.y1, h, lin_y, sq_y);
    }
    s
}

/// Cushion parameters: ridge height at the top level and how much it shrinks per level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cushion {
    pub height: f64,
    pub scale: f64,
}

/// How finely to lay out: `unit` is the size of one screen pixel in world units. Folders narrower
/// than two pixels are not subdivided and tiles under a square pixel are dropped, which matches
/// what the original pixel-based layout showed at the same zoom.
#[derive(Clone, Copy, Debug)]
pub struct Detail {
    pub unit: f64,
}

impl Detail {
    fn subdivide(&self, r: &DRect) -> bool {
        r.w() >= 2.0 * self.unit && r.h() >= 2.0 * self.unit
    }
    fn visible(&self, r: &DRect) -> bool {
        r.w() * r.h() >= self.unit * self.unit
    }
}

/// One drawn tile from `build`.
pub struct Leaf<'a> {
    pub node: &'a Node,
    pub rect: DRect,
    pub surface: Surface,
    /// Child-index path from the layout root.
    pub path: &'a [u32],
    /// For a folder: its children from this index on are too small to draw. Reported before
    /// the children that are drawn (so it lies underneath them), letting the gap they leave be
    /// filled with something representative instead of empty background.
    pub hidden_from: usize,
}

/// Child rectangles of a folder: a prefix of its full squarified layout (see `child_rects`).
pub struct Kids {
    all: Arc<Vec<DRect>>,
    len: usize,
}

impl std::ops::Deref for Kids {
    type Target = [DRect];
    fn deref(&self) -> &[DRect] {
        &self.all[..self.len]
    }
}

/// Folders with at least this many children have their layout remembered in a `Memo`.
const MEMO_MIN_CHILDREN: usize = 2048;
/// Upper bound on remembered rectangles (32 bytes each) before the memo starts over.
const MEMO_MAX_RECTS: usize = 4 << 20;

/// Remembers the full layout of folders with very many children, shared between the tile
/// builders and hit-testing, so zooming around inside a folder of 100k files doesn't squarify
/// it again for every frame or rebuild. Keyed by node address, so it must be dropped with the tree.
#[derive(Default)]
pub struct Memo {
    inner: Mutex<(HashMap<(usize, [u64; 4]), Arc<Vec<DRect>>>, usize)>,
}

impl Memo {
    fn layout(&self, node: &Node, rect: &DRect, n: usize) -> Arc<Vec<DRect>> {
        let key = (node as *const Node as usize, [rect.x0.to_bits(), rect.y0.to_bits(), rect.x1.to_bits(), rect.y1.to_bits()]);
        if let Some(v) = self.inner.lock().unwrap().0.get(&key) {
            return v.clone();
        }
        let kids = &node.children;
        let v = Arc::new(squarify_with(|i| kids[i].size as f64, n, node.size as f64, rect, n));
        let mut g = self.inner.lock().unwrap();
        if g.1 + n > MEMO_MAX_RECTS {
            g.0.clear();
            g.1 = 0;
        }
        g.1 += n;
        g.0.insert(key, v.clone());
        v
    }
}

/// Squarify a folder's children into `rect` (f64). Returns one rect per child with a size above
/// zero, stopping at the first child whose area would be under `min_area` (children are sorted
/// largest first, so all the rest are smaller still; the ones returned are placed exactly as in
/// a full layout).
pub fn child_rects(node: &Node, rect: &DRect, min_area: f64, memo: Option<&Memo>) -> Kids {
    // Children are sorted largest first, so both cut-offs are binary searches; a folder with a
    // million files costs only for the children actually laid out.
    let kids = &node.children;
    let n = kids.partition_point(|c| c.size > 0);
    // A folder's size is the sum of its children's.
    let total = node.size as f64;
    let none = || Kids { all: Arc::new(Vec::new()), len: 0 };
    if n == 0 || total <= 0.0 {
        return none();
    }
    let per_byte = rect.w() * rect.h() / total;
    let shown = if min_area > 0.0 { kids[..n].partition_point(|c| c.size as f64 * per_byte >= min_area) } else { n };
    if shown == 0 {
        return none();
    }
    match memo {
        Some(m) if n >= MEMO_MIN_CHILDREN => Kids { all: m.layout(node, rect, n), len: shown },
        _ => Kids { all: Arc::new(squarify_with(|i| kids[i].size as f64, n, total, rect, shown)), len: shown },
    }
}

/// Generate the tiles of `root` laid out into `world` that intersect `cull`, at level of detail
/// `detail`. `leaf` is called for every tile drawn on top (files, and folders whose contents are
/// too small to show), parents before children; it returns false to stop early. Folders that are
/// subdivided are not reported.
pub fn build(
    root: &Node,
    world: DRect,
    cull: &DRect,
    detail: Detail,
    cushion: Cushion,
    memo: Option<&Memo>,
    leaf: &mut dyn FnMut(Leaf<'_>) -> bool,
) {
    let mut path = Vec::new();
    let mut ctx = Walk { cull, detail, memo, leaf, f: cushion.scale };
    build_into(&mut ctx, root, world, [0.0; 4], cushion.height, &mut path);
}

struct Walk<'a> {
    cull: &'a DRect,
    detail: Detail,
    memo: Option<&'a Memo>,
    leaf: &'a mut dyn FnMut(Leaf<'_>) -> bool,
    f: f64,
}

/// Returns false once the callback asked to stop.
fn build_into(w: &mut Walk<'_>, node: &Node, rect: DRect, surface: Surface, h: f64, path: &mut Vec<u32>) -> bool {
    let surface = with_ridges(surface, &rect, h);
    if !node.is_dir || !w.detail.subdivide(&rect) {
        return (w.leaf)(Leaf { node, rect, surface, path, hidden_from: 0 });
    }
    let unit = w.detail.unit;
    let rects = child_rects(node, &rect, unit * unit, w.memo);
    if rects.is_empty() {
        return (w.leaf)(Leaf { node, rect, surface, path, hidden_from: 0 });
    }
    let nonzero = node.children.partition_point(|c| c.size > 0);
    if rects.len() < nonzero && !(w.leaf)(Leaf { node, rect, surface, path, hidden_from: rects.len() }) {
        return false;
    }
    for (i, r) in rects.iter().enumerate() {
        if !r.intersects(w.cull) {
            continue;
        }
        path.push(i as u32);
        let go = build_into(w, &node.children[i], *r, surface, h * w.f, path);
        path.pop();
        if !go {
            return false;
        }
    }
    true
}

/// Where `path` (relative to `root`) lies when `root` is laid out into `world`, with its cushion
/// surface. None if the path doesn't exist or has no area.
pub fn locate(root: &Node, world: DRect, path: &[u32], cushion: Cushion) -> Option<(DRect, Surface)> {
    let mut n = root;
    let mut r = world;
    let mut h = cushion.height;
    let mut s = with_ridges([0.0; 4], &r, h);
    for &i in path {
        let rects = child_rects(n, &r, 0.0, None);
        r = *rects.get(i as usize)?;
        n = &n.children[i as usize];
        h *= cushion.scale;
        s = with_ridges(s, &r, h);
    }
    Some((r, s))
}

/// Rectangles of every folder on the way to `path` (including the root and the path itself).
pub fn ancestors(root: &Node, world: DRect, path: &[u32]) -> Vec<DRect> {
    let mut out = vec![world];
    let mut n = root;
    let mut r = world;
    for &i in path {
        let rects = child_rects(n, &r, 0.0, None);
        let Some(&c) = rects.get(i as usize) else { break };
        r = c;
        n = &n.children[i as usize];
        out.push(r);
    }
    out
}

/// Remembers the child rectangles of the folders along the last hit-tested path, so moving the
/// mouse within the same folders doesn't squarify them again (a folder can hold 100k files).
#[derive(Default)]
pub struct HitCache {
    /// (node address, its rect, smallest area laid out, child rects) per depth.
    levels: Vec<(usize, DRect, f64, Vec<DRect>)>,
}

impl HitCache {
    pub fn clear(&mut self) {
        self.levels.clear();
    }

    fn children(&mut self, depth: usize, node: &Node, rect: &DRect, min_area: f64, memo: Option<&Memo>) -> &[DRect] {
        let key = node as *const Node as usize;
        let hit = self.levels.get(depth).is_some_and(|(k, r, a, _)| *k == key && r == rect && *a == min_area);
        if !hit {
            self.levels.truncate(depth);
            self.levels.push((key, *rect, min_area, child_rects(node, rect, min_area, memo).to_vec()));
        }
        &self.levels[depth].3
    }

    /// The tile drawn at world point (x, y) at the given level of detail: its path from `root`
    /// and rectangle. Matches what `build` draws. None outside `world`.
    pub fn hit(&mut self, root: &Node, world: DRect, x: f64, y: f64, detail: Detail, memo: Option<&Memo>) -> Option<(Vec<u32>, DRect)> {
        if !world.contains(x, y) {
            return None;
        }
        let mut path = Vec::new();
        let mut n = root;
        let mut r = world;
        loop {
            if !n.is_dir || !detail.subdivide(&r) {
                return Some((path, r));
            }
            let rects = self.children(path.len(), n, &r, detail.unit * detail.unit, memo);
            if rects.is_empty() {
                return Some((path, r));
            }
            match rects.iter().position(|c| c.contains(x, y)) {
                Some(i) if detail.visible(&rects[i]) => {
                    r = rects[i];
                    n = &n.children[i];
                    path.push(i as u32);
                }
                // Over a gap left by tiles too small to draw: the folder's background.
                _ => return Some((path, r)),
            }
        }
    }
}


/// Squarify `len` values (read through `value`, sorted descending, summing to `total`) into
/// `rect`, computing only the first `count` rectangles.
fn squarify_with(value: impl Fn(usize) -> f64, len: usize, total: f64, rect: &DRect, count: usize) -> Vec<DRect> {
    let mut out = Vec::with_capacity(count);
    let mut remaining = total;
    let empty = |x: f64, y: f64| DRect::new(x, y, x, y);
    if remaining <= 0.0 {
        return vec![empty(rect.x0, rect.y0); count];
    }
    let (mut x, mut y) = (rect.x0, rect.y0);
    let (mut w, mut h) = (rect.w(), rect.h());
    let mut i = 0;
    while i < len && out.len() < count {
        let scale = (w * h) / remaining;
        let short = w.min(h);
        // Grow the row while the worst aspect ratio keeps improving.
        let mut j = i;
        let mut sum = 0.0;
        let mut best = f64::INFINITY;
        let rmax = value(i) * scale;
        while j < len {
            let a = value(j) * scale;
            let s = sum + a;
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
            while out.len() < count {
                out.push(empty(x, y));
            }
            break;
        }
        let thick = sum / short;
        let mut off = 0.0;
        for k in i..j {
            let l = value(k) * scale / thick;
            out.push(if w >= h {
                DRect::new(x, y + off, x + thick, y + off + l)
            } else {
                DRect::new(x + off, y, x + off + l, y + thick)
            });
            off += l;
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
    out.truncate(count);
    out
}

/// A made-up folder tree for tests and benchmarks: about `files` files with log-normal-ish
/// sizes, in folders nested a few levels deep with a realistic mix of fan-outs. Also opened by
/// the app when DIRMAP_SYNTHETIC=<files> is set, to try the map at scale without a real disk.
pub fn synthetic(files: usize, seed: u64) -> Node {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
    }
    const EXTS: [&str; 12] = ["rs", "txt", "png", "jpg", "mp4", "dll", "exe", "zip", "json", "js", "pdf", "gguf"];
    fn make(rng: &mut Rng, budget: usize, depth: u32) -> Node {
        let mut children = Vec::new();
        let mut left = budget;
        // Some files directly in this folder, the rest split over subfolders.
        let here = if depth >= 7 || left < 40 { left } else { (left as f64 * rng.unit() * 0.3) as usize };
        for k in 0..here {
            let size = (rng.unit() * 18.0).exp() as u64 + 1;
            let name = format!("f{k}.{}", EXTS[(rng.next() % 12) as usize]);
            children.push(Node { name: name.into(), size, files: 1, modified: 1_700_000_000, is_dir: false, children: Vec::new() });
        }
        left -= here;
        let mut d = 0;
        while left > 0 {
            let part = ((left as f64) * (0.05 + rng.unit() * 0.5)).ceil() as usize;
            let part = part.min(left);
            let mut sub = make(rng, part, depth + 1);
            sub.name = format!("dir{d}").into();
            children.push(sub);
            left -= part;
            d += 1;
        }
        children.sort_by(|a, b| b.size.cmp(&a.size));
        let size = children.iter().map(|c| c.size).sum();
        let files = children.iter().map(|c| c.files).sum();
        Node { name: "root".into(), size, files, modified: 1_700_000_000, is_dir: true, children }
    }
    make(&mut Rng(seed | 1), files, 0)
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

    #[test]
    fn f64_layout_matches_f32_layout() {
        let tree = synthetic(20_000, 7);
        let world = DRect::from_size(1200.0, 700.0);
        let old = layout(&tree, world.to_f32(), 0.38, 0.91);
        let mut new = Vec::new();
        let detail = Detail { unit: 1.0 };
        let cushion = Cushion { height: 0.38, scale: 0.91 };
        build(&tree, world, &world, detail, cushion, None, &mut |l| {
            if l.hidden_from > 0 { return true; }
            new.push((l.path.to_vec(), l.rect, l.surface));
            true
        });
        // Every leaf of the new layout is a tile of the old one at the same place.
        let by_path: std::collections::HashMap<_, _> = old.iter().map(|t| (t.path.clone(), t)).collect();
        assert!(new.len() > 1000);
        for (p, r, s) in &new {
            let t = by_path.get(p).expect("leaf missing from the f32 layout");
            assert!((t.rect.min.x as f64 - r.x0).abs() < 0.05 && (t.rect.max.y as f64 - r.y1).abs() < 0.05);
            for k in 0..4 {
                assert!((t.surface[k] as f64 - s[k]).abs() <= 1e-3 * (1.0 + s[k].abs()), "{k}: {} vs {}", t.surface[k], s[k]);
            }
        }
    }

    #[test]
    fn culled_build_is_a_subset_and_covers_the_view() {
        let tree = synthetic(50_000, 3);
        let world = DRect::from_size(1000.0, 600.0);
        let cushion = Cushion { height: 0.38, scale: 0.91 };
        let detail = Detail { unit: 0.01 };
        let view = DRect::new(400.0, 200.0, 440.0, 224.0);
        let mut all = std::collections::HashSet::new();
        build(&tree, world, &world, detail, cushion, None, &mut |l| all.insert(l.path.to_vec()) || true);
        let mut part = Vec::new();
        let memo = Memo::default();
        build(&tree, world, &view, detail, cushion, Some(&memo), &mut |l| {
            part.push((l.path.to_vec(), l.rect));
            true
        });
        assert!(part.len() < all.len() / 4, "{} of {}", part.len(), all.len());
        for (p, r) in &part {
            assert!(all.contains(p));
            assert!(r.intersects(&view));
        }
    }

    #[test]
    fn hit_test_agrees_with_build_and_locate() {
        let tree = synthetic(30_000, 11);
        let world = DRect::from_size(800.0, 500.0);
        let cushion = Cushion { height: 0.38, scale: 0.91 };
        let detail = Detail { unit: 0.05 };
        let mut tiles = Vec::new();
        build(&tree, world, &world, detail, cushion, None, &mut |l| {
            if l.hidden_from > 0 { return true; }
            tiles.push((l.path.to_vec(), l.rect));
            true
        });
        let mut cache = HitCache::default();
        for (p, r) in tiles.iter().step_by(37) {
            let (cx, cy) = r.center();
            let (hp, hr) = cache.hit(&tree, world, cx, cy, detail, None).unwrap();
            assert_eq!(&hp, p);
            assert_eq!(&hr, r);
            let (lr, _) = locate(&tree, world, p, cushion).unwrap();
            assert_eq!(&lr, r);
        }
        assert!(cache.hit(&tree, world, -1.0, 5.0, detail, None).is_none());
    }
}

