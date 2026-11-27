//! Comparing two splat scenes (Pro): side by side or split by a line to drag across, or as
//! their difference, with the cameras turning together, how different the two pictures are,
//! and the two scenes' details next to each other with the differences picked out.
//!
//! Both pictures are drawn together on the graphics chip ([`gpu::want_pair`]), so a split
//! or a difference always compares pictures from the same moment.

use super::*;
use egui::{Pos2, Vec2};
use crate::splat::{Limits, Scene, SceneKind, View, gpu};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    SideBySide,
    Split,
    Difference,
}

impl Mode {
    const ALL: [Mode; 3] = [Mode::SideBySide, Mode::Split, Mode::Difference];

    fn name(self) -> &'static str {
        match self {
            Mode::SideBySide => "Side by side",
            Mode::Split => "Split",
            Mode::Difference => "Difference",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Mode::SideBySide => "The two scenes next to each other",
            Mode::Split => "One picture, the first scene left of the line and the second right of it: drag the line",
            Mode::Difference => "Where the two pictures differ: black is the same, bright is different",
        }
    }
}

/// One scene being compared.
enum Slot {
    Loading,
    Ready(Arc<Scene>),
    Failed(String),
}

pub(super) struct SceneCompare {
    paths: [PathBuf; 2],
    sizes: [u64; 2],
    slots: [Slot; 2],
    /// Each scene as it's read (started with the window, which has the context to wake).
    rx: Option<mpsc::Receiver<(usize, Result<Scene, String>)>>,
    mode: Mode,
    /// Both cameras follow either scene (side by side; split and difference always do).
    together: bool,
    views: [View; 2],
    /// Where the split line is, from 0 (left) to 1 (right).
    split: f32,
    tex: [Option<egui::TextureHandle>; 2],
    diff: Option<egui::TextureHandle>,
    /// The views and size of each picture on screen, and whether they're the real splatting.
    shown: Option<([View; 2], [usize; 2], bool)>,
    pending: Option<(u64, [View; 2], [usize; 2])>,
    /// Pairs asked for that may still come (any recent one is shown, so it keeps up while turning).
    asked: VecDeque<(u64, [View; 2], [usize; 2])>,
    /// How alike the two pictures on screen are (PSNR, in decibels), and drawing time.
    psnr: Option<f32>,
    ms: Option<f32>,
}

impl SceneCompare {
    fn new(a: PathBuf, b: PathBuf, sizes: [u64; 2]) -> Self {
        SceneCompare {
            paths: [a, b],
            sizes,
            slots: [Slot::Loading, Slot::Loading],
            rx: None,
            mode: Mode::SideBySide,
            together: true,
            views: [View::default(); 2],
            split: 0.5,
            tex: [None, None],
            diff: None,
            shown: None,
            pending: None,
            asked: VecDeque::new(),
            psnr: None,
            ms: None,
        }
    }

    fn start_reading(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        for (i, p) in self.paths.clone().into_iter().enumerate() {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let r = if crate::splat::claims(&p) { crate::splat::read(&p, Limits::PREVIEW) } else { Err("This file doesn't hold a splat scene.".into()) };
                let _ = tx.send((i, r));
                ctx.request_repaint();
            });
        }
        self.rx = Some(rx);
    }
}

impl App {
    /// Compare Scenes, for two splat files.
    pub(super) fn open_scene_compare(&mut self, paths: Vec<PathBuf>) {
        if paths.len() != 2 || !self.premium(crate::licence::Feature::SplatCompare) {
            return;
        }
        let size = |p: &PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let sizes = [size(&paths[0]), size(&paths[1])];
        let mut it = paths.into_iter();
        let (a, b) = (it.next().unwrap(), it.next().unwrap());
        self.files.scene_compare = Some(SceneCompare::new(a, b, sizes));
    }

    pub(super) fn scene_compare_window(&mut self, ctx: &egui::Context) {
        let Some(c) = self.files.scene_compare.as_mut() else { return };
        if c.rx.is_none() {
            c.start_reading(ctx);
        }
        while let Some(Ok((i, r))) = c.rx.as_ref().map(|rx| rx.try_recv()) {
            c.slots[i] = match r {
                Ok(s) => Slot::Ready(Arc::new(s)),
                Err(e) => Slot::Failed(e),
            };
        }
        let mut open = true;
        let screen = ctx.content_rect();
        egui::Window::new(egui::RichText::new("Compare Scenes").strong())
            .id(egui::Id::new("scene-compare"))
            .open(&mut open)
            .default_size([(screen.width() - 80.0).min(1200.0), (screen.height() - 80.0).min(760.0)])
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| compare_body(ui, c));
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            self.files.scene_compare = None;
            gpu::let_go_pair();
        }
    }
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn compare_body(ui: &mut egui::Ui, c: &mut SceneCompare) {
    ui.horizontal(|ui| {
        for m in Mode::ALL {
            if ui.selectable_label(c.mode == m, m.name()).on_hover_text(m.about()).clicked() {
                c.mode = m;
            }
        }
        ui.separator();
        ui.add_enabled_ui(c.mode == Mode::SideBySide, |ui| {
            ui.checkbox(&mut c.together, "Turn together").on_hover_text("Turning, moving or zooming either scene moves both. Split and Difference always do.");
        });
        if ui.button("Reset Views").on_hover_text("Both cameras back to the start (or double-click a scene)").clicked() {
            c.views = [View::default(); 2];
        }
        if let Some(p) = c.psnr {
            ui.separator();
            let alike = if p >= 40.0 {
                "practically the same"
            } else if p >= 30.0 {
                "close"
            } else if p >= 20.0 {
                "visibly different"
            } else {
                "very different"
            };
            ui.label(format!("Pictures: {}", if p.is_infinite() { "identical".to_string() } else { format!("{p:.1} dB PSNR, {alike}") }))
                .on_hover_text("How alike the two pictures on screen are (peak signal-to-noise ratio): higher is more alike, 40 dB and up looks the same. Only meaningful for two versions of one scene.");
        }
    });
    ui.add_space(4.0);
    let facts_h = 190.0f32.min(ui.available_height() * 0.35);
    let size = vec2(ui.available_width(), (ui.available_height() - facts_h - 8.0).max(120.0));
    let ready: Option<[Arc<Scene>; 2]> = match &c.slots {
        [Slot::Ready(a), Slot::Ready(b)] => Some([a.clone(), b.clone()]),
        _ => None,
    };
    match ready {
        Some(scenes) => views(ui, c, &scenes, size),
        None => {
            let (r, _) = ui.allocate_exact_size(size, Sense::hover());
            let pal = style::pal(ui);
            ui.painter().rect_filled(r, style::RADIUS, pal.well);
            let msg: Vec<String> = c
                .slots
                .iter()
                .zip(&c.paths)
                .map(|(s, p)| match s {
                    Slot::Loading => format!("Reading {}…", name_of(p)),
                    Slot::Failed(e) => format!("{}: {e}", name_of(p)),
                    Slot::Ready(_) => format!("{} is ready", name_of(p)),
                })
                .collect();
            ui.painter().text(r.center(), Align2::CENTER_CENTER, msg.join("\n"), FontId::proportional(13.0), ui.visuals().weak_text_color());
        }
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("scene-compare-facts").auto_shrink(false).show(ui, |ui| facts_table(ui, c));
}

/// The two scenes on screen, as the mode says.
fn views(ui: &mut egui::Ui, c: &mut SceneCompare, scenes: &[Arc<Scene>; 2], size: Vec2) {
    let pal = style::pal(ui);
    let gap = 6.0;
    let side = c.mode == Mode::SideBySide;
    let (all, _) = ui.allocate_exact_size(size, Sense::hover());
    let rects = if side {
        let w = (size.x - gap) / 2.0;
        [Rect::from_min_size(all.min, vec2(w, size.y)), Rect::from_min_size(all.min + vec2(w + gap, 0.0), vec2(w, size.y))]
    } else {
        [all, all]
    };
    // Steering: whichever scene the pointer is on.
    let mut busy = false;
    for i in 0..2 {
        if !side && i == 1 {
            break;
        }
        let resp = ui.interact(rects[i], egui::Id::new(("scene-compare-view", i)), Sense::click_and_drag());
        let resp = resp.on_hover_cursor(egui::CursorIcon::Grab);
        let mut v = c.views[i];
        // The split line takes its own drags.
        let on_line = !side && c.mode == Mode::Split && ui.input(|inp| inp.pointer.interact_pos()).is_some_and(|p| (p.x - (all.left() + all.width() * c.split)).abs() < 6.0);
        if !on_line {
            busy |= super::preview::steer_splat_view(ui, &resp, rects[i], &mut v);
        }
        if v != c.views[i] {
            c.views[i] = v;
            if c.together || !side {
                c.views[1 - i] = v;
            }
        }
    }
    if !side {
        c.views[1] = c.views[0];
    }
    let ppp = ui.ctx().pixels_per_point();
    let px = [((rects[0].width() * ppp) as usize).clamp(16, 1600), ((rects[0].height() * ppp) as usize).clamp(16, 1600)];
    let gpu_ok = !matches!(gpu::status(), gpu::Status::Unavailable(_)) && scenes.iter().all(|s| s.kind != SceneKind::PointCloud);
    let set = |ui: &egui::Ui, tex: &mut Option<egui::TextureHandle>, name: &str, img: egui::ColorImage| match tex {
        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
        None => *tex = Some(ui.ctx().load_texture(name, img, egui::TextureOptions::LINEAR)),
    };
    let mut fresh: Option<[(Vec<u8>, [usize; 2]); 2]> = None;
    if gpu_ok {
        if let Some([a, b]) = gpu::take_pair() {
            if let Some(&(_, v, s)) = c.asked.iter().find(|(id, _, _)| *id == a.id) {
                c.shown = Some((v, s, true));
                c.ms = Some(a.ms.max(b.ms));
                c.asked.retain(|(id, _, _)| *id > a.id);
                if c.pending.is_some_and(|(id, _, _)| id <= a.id) {
                    c.pending = None;
                }
                fresh = Some([(a.rgba, a.size), (b.rgba, b.size)]);
            }
        }
    }
    let gpu_px = if busy && gpu_ok { [(px[0] / 2).max(16), (px[1] / 2).max(16)] } else { px };
    let want = (c.views, gpu_px);
    if c.shown.map(|(v, s, _)| (v, s)) != Some(want) || c.tex[0].is_none() {
        if gpu_ok {
            if c.pending.map(|(_, v, s)| (v, s)) != Some(want) {
                let size = [gpu_px[0] as u32, gpu_px[1] as u32];
                let id = gpu::want_pair((scenes[0].clone(), None, c.views[0], size), (scenes[1].clone(), None, c.views[1], size), Some(ui.ctx().clone()));
                c.pending = Some((id, c.views, gpu_px));
                c.asked.push_back((id, c.views, gpu_px));
                while c.asked.len() > 32 {
                    c.asked.pop_front();
                }
            }
            if c.tex[0].is_none() && fresh.is_none() {
                fresh = Some(points(scenes, &c.views, px, true));
                c.shown = Some((c.views, px, false));
            }
        } else {
            fresh = Some(points(scenes, &c.views, px, busy));
            c.shown = if busy { None } else { Some((c.views, px, false)) };
            if busy {
                ui.ctx().request_repaint();
            }
        }
    }
    if let Some([(a, sa), (b, sb)]) = fresh {
        // How alike, and where they differ, from the same-sized pictures.
        if sa == sb {
            let (psnr, diff) = difference(&a, &b);
            c.psnr = Some(psnr);
            set(ui, &mut c.diff, "scene-compare-diff", egui::ColorImage::from_rgba_unmultiplied(sa, &diff));
        }
        set(ui, &mut c.tex[0], "scene-compare-a", egui::ColorImage::from_rgba_premultiplied(sa, &a));
        set(ui, &mut c.tex[1], "scene-compare-b", egui::ColorImage::from_rgba_premultiplied(sb, &b));
    }
    if c.pending.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    }
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let p = ui.painter().with_clip_rect(all.intersect(ui.clip_rect()));
    for r in rects.iter().take(if side { 2 } else { 1 }) {
        p.rect_filled(*r, style::RADIUS, pal.well);
    }
    match c.mode {
        Mode::SideBySide => {
            for i in 0..2 {
                if let Some(t) = &c.tex[i] {
                    p.image(t.id(), rects[i], uv, Color32::WHITE);
                }
            }
        }
        Mode::Split => {
            let x = all.left() + all.width() * c.split;
            let (left, right) = (Rect::from_min_max(all.min, pos2(x, all.bottom())), Rect::from_min_max(pos2(x, all.top()), all.max));
            if let Some(t) = &c.tex[0] {
                p.image(t.id(), left, Rect::from_min_max(pos2(0.0, 0.0), pos2(c.split, 1.0)), Color32::WHITE);
            }
            if let Some(t) = &c.tex[1] {
                p.image(t.id(), right, Rect::from_min_max(pos2(c.split, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            // The line, with a handle to drag.
            let line = Rect::from_center_size(pos2(x, all.center().y), vec2(12.0, all.height()));
            let lr = ui.interact(line, egui::Id::new("scene-compare-line"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if lr.dragged() {
                c.split = ((c.split * all.width() + lr.drag_delta().x) / all.width()).clamp(0.02, 0.98);
            }
            p.line_segment([pos2(x, all.top()), pos2(x, all.bottom())], Stroke::new(2.0, Color32::from_white_alpha(220)));
            p.circle_filled(pos2(x, all.center().y), 9.0, Color32::from_white_alpha(230));
            p.text(pos2(x, all.center().y), Align2::CENTER_CENTER, "⟷", FontId::proportional(11.0), Color32::from_gray(40));
        }
        Mode::Difference => {
            p.rect_filled(all, style::RADIUS, Color32::BLACK);
            if let Some(t) = &c.diff {
                p.image(t.id(), all, uv, Color32::WHITE);
            }
        }
    }
    // Names in the corners, and how it's drawn.
    let tag = |p: &egui::Painter, at: Pos2, align: Align2, text: String| {
        let g = p.layout_no_wrap(text, FontId::proportional(11.0), Color32::from_gray(225));
        let r = align.anchor_size(at, g.size());
        p.rect_filled(r.expand2(vec2(4.0, 2.0)), 3.0, Color32::from_black_alpha(150));
        p.galley(r.min, g, Color32::from_gray(225));
    };
    let names = [name_of(&c.paths[0]), name_of(&c.paths[1])];
    match c.mode {
        Mode::SideBySide => {
            for i in 0..2 {
                tag(&p, rects[i].left_top() + vec2(8.0, 8.0), Align2::LEFT_TOP, names[i].clone());
            }
        }
        Mode::Split => {
            tag(&p, all.left_top() + vec2(8.0, 8.0), Align2::LEFT_TOP, names[0].clone());
            tag(&p, all.right_top() + vec2(-8.0, 8.0), Align2::RIGHT_TOP, names[1].clone());
        }
        Mode::Difference => tag(&p, all.left_top() + vec2(8.0, 8.0), Align2::LEFT_TOP, format!("{} against {}", names[0], names[1])),
    }
    let how = match (gpu::status(), c.shown.is_some_and(|s| s.2), c.ms) {
        (gpu::Status::Ready(name), true, Some(ms)) => format!("{name} · {ms:.0} ms a scene"),
        (gpu::Status::Unavailable(why), _, _) => format!("Splat centres, drawn as points: {why}"),
        _ => "Splat centres, drawn as points (the graphics chip is starting)".to_string(),
    };
    tag(&p, all.left_bottom() + vec2(8.0, -6.0), Align2::LEFT_BOTTOM, how);
}

/// Both scenes drawn as points on the processor (a sample while moving).
fn points(scenes: &[Arc<Scene>; 2], views: &[View; 2], px: [usize; 2], quick: bool) -> [(Vec<u8>, [usize; 2]); 2] {
    let draw = |i: usize| {
        let s = &scenes[i];
        let step = if quick { (s.len() / 150_000).max(1) } else { 1 };
        // Straight alpha to premultiplied, as the graphics chip gives.
        let mut rgba = s.draw(px[0], px[1], &views[i], step);
        for c in rgba.chunks_exact_mut(4) {
            for k in 0..3 {
                c[k] = ((c[k] as u32 * c[3] as u32 + 127) / 255) as u8;
            }
        }
        (rgba, px)
    };
    [draw(0), draw(1)]
}

/// How alike two premultiplied pictures of one size are (PSNR over colour, as seen on black,
/// in decibels; infinite when identical), and a picture of where they differ.
pub(super) fn difference(a: &[u8], b: &[u8]) -> (f32, Vec<u8>) {
    let mut sum = 0f64;
    let mut out = Vec::with_capacity(a.len());
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let mut most = 0u8;
        for k in 0..3 {
            let d = pa[k].abs_diff(pb[k]);
            sum += (d as f64) * (d as f64);
            most = most.max(d);
        }
        // Small differences brought up so they show.
        let v = ((most as f32 / 255.0).sqrt() * 255.0) as u8;
        out.extend_from_slice(&[v, v / 3, v.saturating_sub(40), 255]);
    }
    let mse = sum / (a.len() / 4 * 3).max(1) as f64;
    let psnr = if mse == 0.0 { f32::INFINITY } else { (10.0 * (255.0f64 * 255.0 / mse).log10()) as f32 };
    (psnr, out)
}

/// The two scenes' details side by side, with rows that differ picked out.
fn facts_table(ui: &mut egui::Ui, c: &SceneCompare) {
    let none: Vec<(&'static str, String)> = Vec::new();
    let facts = |i: usize| match &c.slots[i] {
        Slot::Ready(s) => &s.facts,
        _ => &none,
    };
    let rows = compared_facts([facts(0), facts(1)], c.sizes);
    let differ = Color32::from_rgb(230, 160, 40);
    // Rows of three: the detail's name, then each scene's half of what's left.
    let key_w = 150.0;
    let col = ((ui.available_width() - key_w - 24.0) / 2.0).max(120.0);
    let row = |ui: &mut egui::Ui, cells: [egui::RichText; 3]| {
        ui.horizontal_top(|ui| {
            for (k, t) in cells.into_iter().enumerate() {
                let w = if k == 0 { key_w } else { col };
                ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(w);
                    ui.add(egui::Label::new(t).wrap());
                });
            }
        });
    };
    row(ui, [egui::RichText::new(""), egui::RichText::new(name_of(&c.paths[0])).strong(), egui::RichText::new(name_of(&c.paths[1])).strong()]);
    for (k, a, b) in rows {
        let name = if a == b { egui::RichText::new(k).weak() } else { egui::RichText::new(k).color(differ) };
        row(ui, [name, egui::RichText::new(a.unwrap_or_else(|| "—".into())), egui::RichText::new(b.unwrap_or_else(|| "—".into()))]);
    }
}

/// Rows of details for two scenes: the file size first, then each detail of the first scene
/// in its order, then any only the second has.
pub(super) fn compared_facts(facts: [&Vec<(&'static str, String)>; 2], sizes: [u64; 2]) -> Vec<(&'static str, Option<String>, Option<String>)> {
    let mut rows = vec![("File size", Some(crate::scan::human(sizes[0])), Some(crate::scan::human(sizes[1])))];
    let get = |i: usize, k: &str| facts[i].iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
    for (k, _) in facts[0].iter() {
        if !rows.iter().any(|(n, _, _)| n == k) {
            rows.push((k, get(0, k), get(1, k)));
        }
    }
    for (k, _) in facts[1].iter() {
        if !rows.iter().any(|(n, _, _)| n == k) {
            rows.push((k, None, get(1, k)));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    #[test]
    fn identical_pictures_are_infinitely_alike_and_differences_show() {
        let a = vec![10u8, 20, 30, 255, 200, 100, 50, 255];
        let (p, d) = super::difference(&a, &a);
        assert!(p.is_infinite());
        assert!(d.chunks_exact(4).all(|px| px[0] == 0));
        let mut b = a.clone();
        b[0] = 74; // off by 64 in one of six channels
        let (p, d) = super::difference(&a, &b);
        assert!((p - 10.0 * (255.0f32 * 255.0 / (64.0 * 64.0 / 6.0)).log10()).abs() < 0.01, "{p}");
        assert!(d[0] > 100 && d[4] == 0);
    }

    #[test]
    fn details_line_up_by_name() {
        let a = vec![("Format", "PLY".to_string()), ("Splats", "150,000".to_string()), ("Made by", "a trainer".to_string())];
        let b = vec![("Format", "SPZ v3".to_string()), ("Splats", "150,000".to_string()), ("Floaters", "3".to_string())];
        let rows = super::compared_facts([&a, &b], [2048, 1024]);
        let keys: Vec<&str> = rows.iter().map(|r| r.0).collect();
        assert_eq!(keys, ["File size", "Format", "Splats", "Made by", "Floaters"]);
        assert_eq!(rows[2].1, rows[2].2);
        assert_eq!(rows[3].2, None);
        assert_eq!(rows[4].1, None);
    }
}
