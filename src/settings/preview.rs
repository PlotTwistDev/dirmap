//! Small pictures in the Settings window that show what a setting does: the theme cards, the
//! folder list preview and a little treemap drawn with the same shading as the real one.
//!
//! The treemap preview is a fixed sample folder laid out once and shaded on the CPU into a
//! texture that's only redrawn when a look setting changes, so dragging a slider costs a few
//! hundred thousand pixel writes at most and never touches the scan or the main map.

use super::{Settings, Theme, TileStyle};
use crate::app::{Shading, ext_color};
use crate::kit;
use crate::mapview::{SOFT_LIGHT_REST, soft_shade};
use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use std::collections::HashMap;

// ---------- theme cards ----------

/// A miniature window in a theme: title bar, sidebar, a header block and some tiles.
pub fn theme_picture(p: &egui::Painter, r: Rect, theme: Theme) {
    let paint = |p: &egui::Painter, r: Rect, dark: bool| {
        let t = kit::tokens_for(dark);
        p.rect_filled(r, 5.0, t.content);
        let bar = Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + r.height() * 0.13));
        p.rect_filled(bar, egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, t.sidebar);
        for k in 0..2 {
            p.circle_filled(pos2(bar.left() + 8.0 + k as f32 * 7.0, bar.center().y), 2.0, t.weak);
        }
        let body = Rect::from_min_max(pos2(r.min.x + 6.0, bar.max.y + 6.0), r.max - vec2(6.0, 6.0));
        let side = Rect::from_min_size(body.min, vec2(body.width() * 0.24, body.height()));
        p.rect_filled(side, 3.0, t.sidebar);
        for k in 0..4 {
            let y = side.top() + 8.0 + k as f32 * 9.0;
            if y + 3.0 < side.bottom() {
                p.rect_filled(Rect::from_min_size(pos2(side.left() + 5.0, y), vec2(side.width() - 10.0, 3.0)), 1.5, t.weak.gamma_multiply(0.7));
            }
        }
        let main = Rect::from_min_max(pos2(side.right() + 6.0, body.top()), body.max);
        let head = Rect::from_min_size(main.min, vec2(main.width(), main.height() * 0.34));
        p.rect_filled(head, 3.0, t.card);
        let tiles = Rect::from_min_max(pos2(main.left(), head.bottom() + 6.0), main.max);
        let w1 = tiles.width() * 0.52;
        p.rect_filled(Rect::from_min_size(tiles.min, vec2(w1, tiles.height())), 3.0, Color32::from_rgb(0x4d, 0x86, 0xd0));
        let right = Rect::from_min_max(pos2(tiles.left() + w1 + 5.0, tiles.top()), tiles.max);
        let h1 = right.height() * 0.55;
        p.rect_filled(Rect::from_min_size(right.min, vec2(right.width(), h1)), 3.0, Color32::from_rgb(0x4a, 0xa8, 0x93));
        p.rect_filled(Rect::from_min_max(pos2(right.left(), right.top() + h1 + 4.0), right.max), 3.0, Color32::from_rgb(0x9b, 0x84, 0xc8));
    };
    match theme {
        Theme::Dark => paint(p, r, true),
        Theme::Light => paint(p, r, false),
        Theme::System => {
            let half = r.center().x;
            paint(&p.with_clip_rect(Rect::from_min_max(r.min, pos2(half, r.max.y))), r, true);
            paint(&p.with_clip_rect(Rect::from_min_max(pos2(half, r.min.y), r.max)), r, false);
            p.vline(half, r.y_range(), Stroke::new(1.0, Color32::from_gray(128)));
        }
    }
}

// ---------- folder list ----------

/// A few rows of the Storage Analysis folder list with the chosen columns, stripes and units.
pub fn folder_list(ui: &mut egui::Ui, d: &Settings) {
    let t = kit::tokens(ui);
    struct Item {
        name: &'static str,
        dir: bool,
        size: u64,
        files: u64,
        changed: &'static str,
    }
    let items = [
        Item { name: "Projects", dir: true, size: 51_800_000_000, files: 12_408, changed: "2026-11-20 16:42" },
        Item { name: "Photos", dir: true, size: 18_300_000_000, files: 6_115, changed: "2026-11-02 09:13" },
        Item { name: "Music", dir: true, size: 9_700_000_000, files: 2_380, changed: "2026-10-28 21:05" },
        Item { name: "backup.zip", dir: false, size: 3_100_000_000, files: 1, changed: "2026-09-14 11:30" },
        Item { name: "Downloads", dir: true, size: 1_600_000_000, files: 418, changed: "2026-11-24 08:57" },
    ];
    let total: u64 = items.iter().map(|i| i.size).sum();
    // Columns: (title, width, right-aligned).
    let mut cols: Vec<(&str, f32, bool)> = vec![("Name", 0.0, false), ("Size", 78.0, true)];
    if d.col_percent {
        cols.push(("% of parent", 112.0, false));
    }
    if d.col_files {
        cols.push(("Files", 64.0, true));
    }
    if d.col_modified {
        cols.push(("Last change", 120.0, false));
    }
    let w = ui.available_width();
    let fixed: f32 = cols.iter().map(|c| c.1).sum();
    cols[0].1 = (w - fixed).max(110.0);
    let row_h = 24.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, row_h * (items.len() + 1) as f32 + 2.0), egui::Sense::hover());
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 6.0, t.pal.paper);
    p.rect_stroke(rect, 6.0, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    let font = egui::FontId::proportional(12.0);
    let head = Rect::from_min_size(rect.min, vec2(w, row_h));
    p.rect_filled(head, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, t.pal.chrome);
    let mut x = rect.left();
    for (title, cw, right) in &cols {
        let pos = if *right { pos2(x + cw - 10.0, head.center().y) } else { pos2(x + 10.0, head.center().y) };
        p.text(pos, if *right { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER }, *title, font.clone(), t.weak);
        x += cw;
    }
    p.hline(rect.x_range(), head.bottom(), Stroke::new(1.0, t.card_border));
    for (k, it) in items.iter().enumerate() {
        let y = head.bottom() + k as f32 * row_h;
        let row = Rect::from_min_size(pos2(rect.left(), y), vec2(w, row_h));
        if k == 0 {
            p.rect_filled(row.shrink2(vec2(1.0, 0.0)), 0.0, t.pal.sel);
        } else if d.stripes && k % 2 == 1 {
            p.rect_filled(row.shrink2(vec2(1.0, 0.0)), 0.0, t.pal.stripe);
        }
        let mut x = rect.left();
        for (title, cw, right) in &cols {
            let cell = Rect::from_min_size(pos2(x, y), vec2(*cw, row_h));
            let text_at = |s: String, right: bool| {
                let pos = if right { pos2(cell.right() - 10.0, cell.center().y) } else { pos2(cell.left() + 10.0, cell.center().y) };
                p.text(pos, if right { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER }, s, font.clone(), t.text);
            };
            match *title {
                "Name" => {
                    let ic = Rect::from_center_size(pos2(cell.left() + 17.0, cell.center().y), vec2(13.0, 13.0));
                    kit::icons::paint(&p, ic, if it.dir { kit::Icon::Folder } else { kit::Icon::File }, if it.dir { Color32::from_rgb(0xe0, 0xb0, 0x4a) } else { t.weak });
                    p.text(pos2(cell.left() + 30.0, cell.center().y), egui::Align2::LEFT_CENTER, it.name, font.clone(), t.strong);
                }
                "Size" => text_at(crate::scan::human(it.size), *right),
                "% of parent" => {
                    let frac = it.size as f32 / total as f32;
                    let bar = Rect::from_min_size(pos2(cell.left() + 10.0, cell.center().y - 4.0), vec2(56.0, 8.0));
                    p.rect_filled(bar, 2.0, t.track);
                    p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height())), 2.0, t.accent);
                    p.text(pos2(bar.right() + 6.0, cell.center().y), egui::Align2::LEFT_CENTER, format!("{:.0}%", frac * 100.0), font.clone(), t.text);
                }
                "Files" => text_at(format!("{}", it.files).as_bytes().rchunks(3).rev().map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(","), *right),
                _ => text_at(it.changed.into(), *right),
            }
            x += cw;
        }
    }
}

// ---------- treemap ----------

/// The sample folder: top-level folders, each with files of a type (by rank).
const SAMPLE: &[(&str, &[(f64, usize)])] = &[
    ("Footage", &[(30.0, 0), (14.0, 0), (9.0, 1), (5.0, 3), (3.0, 0), (2.0, 7)]),
    ("Renders", &[(16.0, 2), (9.0, 2), (6.0, 2), (4.0, 5), (2.0, 2)]),
    ("Audio", &[(9.0, 4), (6.0, 4), (3.0, 4), (2.0, 9)]),
    ("Docs", &[(5.0, 6), (3.0, 8), (2.0, 6), (1.5, 10)]),
    ("Apps", &[(4.0, 11), (2.0, 12), (1.0, 13)]),
    ("Misc", &[(2.0, 14), (1.2, 15), (0.8, 5)]),
];

struct Leaf {
    rect: Rect,
    rank: usize,
    surface: [f32; 4],
}

/// Squarified layout of `values` (largest first) into `r`.
fn squarify(values: &[f64], r: Rect) -> Vec<Rect> {
    let total: f64 = values.iter().sum();
    let mut out = Vec::with_capacity(values.len());
    let mut rest = r;
    let mut i = 0;
    let mut left = total;
    while i < values.len() {
        let wide = rest.width() >= rest.height();
        let side = if wide { rest.height() } else { rest.width() } as f64;
        let area = (rest.width() * rest.height()) as f64;
        let scale = area / left;
        let worst = |row: &[f64]| {
            let s: f64 = row.iter().sum::<f64>() * scale;
            let (mx, mn) = row.iter().fold((0.0f64, f64::MAX), |(a, b), v| (a.max(*v * scale), b.min(*v * scale)));
            (side * side * mx / (s * s)).max(s * s / (side * side * mn))
        };
        let mut j = i + 1;
        while j < values.len() && worst(&values[i..=j]) <= worst(&values[i..j]) {
            j += 1;
        }
        let row = &values[i..j];
        let s: f64 = row.iter().sum();
        let thick = (s * scale / side) as f32;
        let mut pos = 0.0f32;
        for v in row {
            let len = (*v / s * side) as f32;
            out.push(if wide {
                Rect::from_min_size(pos2(rest.left(), rest.top() + pos), vec2(thick, len))
            } else {
                Rect::from_min_size(pos2(rest.left() + pos, rest.top()), vec2(len, thick))
            });
            pos += len;
        }
        rest = if wide { Rect::from_min_max(pos2(rest.left() + thick, rest.top()), rest.max) } else { Rect::from_min_max(pos2(rest.left(), rest.top() + thick), rest.max) };
        left -= s;
        i = j;
    }
    out
}

/// Add a cushion ridge of height `h` over `r` to the surface (x², y², x, y), as the map does.
fn ridges(r: Rect, h: f32, s: &mut [f32; 4]) {
    for (x1, x2, sq, lin) in [(r.left(), r.right(), 0, 2), (r.top(), r.bottom(), 1, 3)] {
        if x2 - x1 > 0.0 {
            let h4 = 4.0 * h / (x2 - x1);
            s[lin] += h4 * (x2 + x1);
            s[sq] -= h4;
        }
    }
}

/// Lay the sample out in a `w`×`h` pixel image with the cushion surfaces for `d`. Returns the
/// leaves and the folder rectangles.
fn layout(d: &Settings, w: f32, h: f32) -> (Vec<Leaf>, Vec<(Rect, &'static str)>) {
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(w, h));
    let sizes: Vec<f64> = SAMPLE.iter().map(|(_, f)| f.iter().map(|x| x.0).sum()).collect();
    let mut root = [0.0f32; 4];
    let (h0, f) = (d.height, d.scale);
    ridges(bounds, h0, &mut root);
    let mut leaves = Vec::new();
    let mut folders = Vec::new();
    for ((name, files), fr) in SAMPLE.iter().zip(squarify(&sizes, bounds)) {
        let mut s = root;
        let h1 = h0 * f;
        ridges(fr, h1, &mut s);
        folders.push((fr, *name));
        let vals: Vec<f64> = files.iter().map(|x| x.0).collect();
        for (&(_, rank), r) in files.iter().zip(squarify(&vals, fr)) {
            let mut s2 = s;
            let h2 = h1 * f;
            ridges(r, h2, &mut s2);
            leaves.push(Leaf { rect: r, rank, surface: s2 });
        }
    }
    (leaves, folders)
}

/// Shade the sample into an image, as the map would with these settings.
pub fn render(d: &Settings, w: usize, h: usize) -> ColorImage {
    let (leaves, _) = layout(d, w as f32, h as f32);
    let mut px = vec![Color32::BLACK; w * h];
    let soft = d.tile_style == TileStyle::Modern;
    let sh = Shading::new(d);
    for l in &leaves {
        let x0 = (l.rect.left().round().max(0.0) as usize).min(w);
        let x1 = (l.rect.right().round().max(0.0) as usize).min(w);
        let y0 = (l.rect.top().round().max(0.0) as usize).min(h);
        let y1 = (l.rect.bottom().round().max(0.0) as usize).min(h);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let c = ext_color(&format!("e{}", l.rank), l.rank, d.tile_style);
        if soft {
            let size = ((x1 - x0) as f32, (y1 - y0) as f32);
            let base = [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0];
            for y in y0..y1 {
                for x in x0..x1 {
                    let mut v = soft_shade(base, ((x - x0) as f32 + 0.5, (y - y0) as f32 + 0.5), size, SOFT_LIGHT_REST);
                    if x == x0 || y == y0 || x + 1 == x1 || y + 1 == y1 {
                        v = v.map(|q| q * 0.5);
                    }
                    px[y * w + x] = Color32::from_rgb((v[0] * 255.0) as u8, (v[1] * 255.0) as u8, (v[2] * 255.0) as u8);
                }
            }
            continue;
        }
        let [s0, s1, s2, s3] = l.surface;
        let (cr, cg, cb) = (c.r() as f32 * sh.gain, c.g() as f32 * sh.gain, c.b() as f32 * sh.gain);
        let [lx, ly, lz] = sh.light;
        for y in y0..y1 {
            let ny = -(2.0 * s1 * (y as f32 + 0.5) + s3);
            for x in x0..x1 {
                let nx = -(2.0 * s0 * (x as f32 + 0.5) + s2);
                let cosa = ((nx * lx + ny * ly + lz) / (nx * nx + ny * ny + 1.0).sqrt()).clamp(0.0, 1.0);
                let f = sh.ambient + sh.diffuse * cosa;
                px[y * w + x] = Color32::from_rgb((cr * f).min(255.0) as u8, (cg * f).min(255.0) as u8, (cb * f).min(255.0) as u8);
            }
        }
        if let Some(gc) = sh.grid {
            px[y0 * w + x0..y0 * w + x1].fill(gc);
            for y in y0..y1 {
                px[y * w + x0] = gc;
            }
        }
    }
    ColorImage::new([w, h], px)
}

/// Textures for the previews, redrawn only when what they show changes.
#[derive(Default)]
pub struct Cache {
    textures: HashMap<&'static str, (u64, TextureHandle)>,
}

fn look_key(d: &Settings, w: usize, h: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    (d.treemap_key(), w, h).hash(&mut hs);
    hs.finish()
}

impl Cache {
    fn texture(&mut self, ctx: &egui::Context, slot: &'static str, d: &Settings, w: usize, h: usize) -> TextureHandle {
        let key = look_key(d, w, h);
        if let Some((k, t)) = self.textures.get(slot)
            && *k == key
        {
            return t.clone();
        }
        let tex = ctx.load_texture(format!("settings-preview-{slot}"), render(d, w, h), TextureOptions::LINEAR);
        self.textures.insert(slot, (key, tex.clone()));
        tex
    }

    /// The big preview, with labels, folder tags and the selection border drawn on top.
    pub fn treemap(&mut self, ui: &mut egui::Ui, d: &Settings, height: f32) {
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, height), egui::Sense::hover());
        let ppp = ui.ctx().pixels_per_point();
        let (pw, ph) = ((w * ppp).round().max(1.0) as usize, (height * ppp).round().max(1.0) as usize);
        let tex = self.texture(ui.ctx(), "main", d, pw, ph);
        let p = ui.painter().with_clip_rect(rect);
        p.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        let (leaves, folders) = layout(d, w, height);
        let off = rect.min.to_vec2();
        let mono = egui::FontId::monospace(10.5);
        if d.show_labels {
            for (i, l) in leaves.iter().enumerate() {
                let r = l.rect.translate(off);
                if r.width() > 64.0 && r.height() > 30.0 {
                    let name = ["clip_0412.mov", "take_07.mov", "shot.exr", "grade.drp", "stems.wav", "render.exr", "mix.wav", "notes.pdf"][i % 8];
                    let fg = if d.tile_style == TileStyle::Modern { Color32::from_black_alpha(200) } else { Color32::from_white_alpha(230) };
                    p.text(r.min + vec2(6.0, 5.0), egui::Align2::LEFT_TOP, name, mono.clone(), fg);
                }
            }
        }
        if d.tile_style == TileStyle::Modern {
            for (fr, name) in &folders {
                let r = fr.translate(off);
                p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_black_alpha(140)), egui::StrokeKind::Inside);
                if r.width() >= 90.0 && r.height() >= 40.0 {
                    let g = p.layout_no_wrap(format!("{name}/"), mono.clone(), Color32::WHITE);
                    let tag = Rect::from_min_size(pos2(r.left() + 6.0, r.bottom() - 26.0), vec2((g.size().x + 14.0).min(r.width() - 12.0), 19.0));
                    p.rect_filled(tag, 5.0, Color32::from_rgba_unmultiplied(4, 5, 6, 235));
                    p.galley(pos2(tag.left() + 7.0, tag.center().y - g.size().y / 2.0), g, Color32::WHITE);
                }
            }
        }
        // The selection border, on the second-biggest file.
        if let Some(l) = leaves.get(1) {
            p.rect_stroke(l.rect.translate(off).shrink(1.0), 0.0, Stroke::new(2.0, d.selection_color), egui::StrokeKind::Inside);
        }
        let t = kit::tokens(ui);
        p.rect_stroke(rect, 6.0, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    }

    /// A thumbnail for a choice card (tile styles, presets).
    pub fn thumb(&mut self, ctx: &egui::Context, p: &egui::Painter, r: Rect, slot: &'static str, d: &Settings) {
        let ppp = ctx.pixels_per_point();
        let (pw, ph) = ((r.width() * ppp).round().max(1.0) as usize, (r.height() * ppp).round().max(1.0) as usize);
        let tex = self.texture(ctx, slot, d, pw, ph);
        p.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    }
}
