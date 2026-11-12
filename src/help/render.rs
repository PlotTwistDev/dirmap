//! Drawing help pages with egui: headings, wrapped text with clickable links, lists, tables,
//! code with Copy (and "Use in Custom filter" for filter examples), notes and screenshots.

use super::doc::{self, Block, Docs, Span};
use eframe::egui::{
    self, pos2, text::CCursor, text::LayoutJob, vec2, Align, Color32, CursorIcon, FontId, Galley, Rect, Sense, Stroke,
    TextFormat, TextureHandle,
};
use std::collections::HashMap;
use std::sync::Arc;

pub const BODY: f32 = 14.5;

/// Where a click on the page wants to go.
pub enum Nav {
    Page(String, Option<String>),
    Url(String),
}

/// What a page asks the app to do.
pub enum Request {
    ApplyFilter(String),
}

pub struct Out<'a> {
    pub page: &'a str,
    pub docs: &'a Docs,
    pub images: &'a mut HashMap<String, Option<TextureHandle>>,
    pub nav: Option<Nav>,
    pub request: Option<Request>,
    /// Heading ids and where they were drawn, for jumping to an anchor.
    pub anchors: Vec<(String, Rect)>,
}

impl Out<'_> {
    fn follow(&mut self, link: &str) {
        self.nav = Some(match doc::resolve(self.page, link) {
            Some((p, a)) => Nav::Page(p, a),
            None => Nav::Url(link.to_string()),
        });
    }
}

pub fn blocks(ui: &mut egui::Ui, out: &mut Out, blocks: &[Block]) {
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            ui.add_space(match b {
                Block::Heading { level: 2, .. } => 14.0,
                Block::Heading { .. } => 10.0,
                _ => 7.0,
            });
        }
        block(ui, out, b);
    }
}

fn block(ui: &mut egui::Ui, out: &mut Out, b: &Block) {
    match b {
        Block::Heading { level, spans, id } => {
            let size = match level {
                1 => 25.0,
                2 => 19.5,
                3 => 16.5,
                _ => 15.0,
            };
            let rect = text(ui, out, spans, size, true);
            out.anchors.push((id.clone(), rect));
            if *level == 2 {
                let y = rect.bottom() + 3.0;
                ui.painter().hline(rect.left()..=ui.max_rect().right(), y, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color));
                ui.add_space(4.0);
            }
        }
        Block::Para(spans) => {
            text(ui, out, spans, BODY, false);
        }
        Block::List { start, items } => {
            for (k, item) in items.iter().enumerate() {
                let marker = match start {
                    Some(n) => format!("{}.", n + k as u64),
                    None => "•".to_string(),
                };
                ui.horizontal_top(|ui| {
                    ui.add_space(6.0);
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, BODY + 4.0), Sense::hover());
                    ui.painter().text(pos2(r.right() - 4.0, r.top() + 1.0), egui::Align2::RIGHT_TOP, marker, FontId::proportional(BODY), ui.visuals().text_color());
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        blocks(ui, out, item);
                    });
                });
            }
        }
        Block::Code { lang, text } => code(ui, out, lang, text),
        Block::Quote(inner) => {
            let accent = ui.visuals().hyperlink_color;
            let frame = egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .inner_margin(egui::Margin { left: 12, right: 8, top: 6, bottom: 6 })
                .corner_radius(4.0);
            let r = frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                blocks(ui, out, inner);
            });
            let rect = r.response.rect;
            ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 1.0, accent);
        }
        Block::Table { head, rows } => table(ui, out, head, rows),
        Block::Image { src, alt } => image(ui, out, src, alt),
        Block::Rule => {
            ui.separator();
        }
    }
}

/// Lay out styled spans, wrapped to the available width. Returns the char ranges of links too.
fn layout(ui: &egui::Ui, spans: &[Span], size: f32, strong: bool, width: f32) -> (Arc<Galley>, Vec<(usize, usize, String)>) {
    let vis = ui.visuals();
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let mut links = Vec::new();
    let mut chars = 0usize;
    for s in spans {
        let mut f = TextFormat {
            font_id: if s.code { FontId::monospace(size - 1.5) } else { FontId::proportional(size) },
            color: if strong || s.bold { vis.strong_text_color() } else { vis.text_color() },
            italics: s.italic,
            ..Default::default()
        };
        if s.code {
            f.background = vis.code_bg_color;
        }
        if s.link.is_some() {
            f.color = vis.hyperlink_color;
            f.underline = Stroke::new(1.0, vis.hyperlink_color.gamma_multiply(0.6));
        }
        let n = s.text.chars().count();
        if let Some(l) = &s.link {
            links.push((chars, chars + n, l.clone()));
        }
        chars += n;
        job.append(&s.text, 0.0, f);
    }
    (ui.painter().layout_job(job), links)
}

/// A wrapped run of text with working links. Returns where it was drawn.
fn text(ui: &mut egui::Ui, out: &mut Out, spans: &[Span], size: f32, strong: bool) -> Rect {
    let (galley, links) = layout(ui, spans, size, strong, ui.available_width());
    let sense = if links.is_empty() { Sense::hover() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(galley.size(), sense);
    if let Some(l) = hovered_link(&galley, rect, &resp, &links) {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        let target = describe(out, &l);
        let resp = resp.clone().on_hover_text(target);
        if resp.clicked() {
            out.follow(&l);
        }
    }
    ui.painter().galley(rect.min, galley, ui.visuals().text_color());
    rect
}

/// The link under the pointer, if any.
fn hovered_link(galley: &Galley, rect: Rect, resp: &egui::Response, links: &[(usize, usize, String)]) -> Option<String> {
    if links.is_empty() {
        return None;
    }
    let p = resp.hover_pos()? - rect.min;
    let c = galley.cursor_from_pos(p).index.0;
    // The cursor lands on the nearest boundary: the character under the pointer is just before
    // or just after it.
    for i in [c.saturating_sub(1), c] {
        let a = galley.pos_from_cursor(CCursor::new(i));
        let b = galley.pos_from_cursor(CCursor::new(i + 1));
        let same_row = (a.min.y - b.min.y).abs() < 1.0;
        if same_row && p.x >= a.min.x && p.x < b.min.x && p.y >= a.min.y && p.y <= a.max.y {
            return links.iter().find(|(s, e, _)| i >= *s && i < *e).map(|l| l.2.clone());
        }
    }
    None
}

/// Tooltip for a link: the page title, or the web address.
fn describe(out: &Out, link: &str) -> String {
    match doc::resolve(out.page, link) {
        Some((p, _)) => format!("Go to: {}", out.docs.title(&p)),
        None => format!("Open in your browser: {link}"),
    }
}

fn code(ui: &mut egui::Ui, out: &mut Out, lang: &str, text: &str) {
    let frame = egui::Frame::new().fill(ui.visuals().code_bg_color).inner_margin(8).corner_radius(4.0);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let w = (ui.available_width() - if lang == "filter" { 210.0 } else { 70.0 }).max(120.0);
            ui.vertical(|ui| {
                ui.set_max_width(w);
                ui.add(egui::Label::new(egui::RichText::new(text).monospace().size(BODY - 1.0)).wrap().selectable(true));
            });
            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                if ui.small_button("Copy").on_hover_text("Copy to the clipboard").clicked() {
                    ui.ctx().copy_text(text.to_string());
                }
                if lang == "filter"
                    && ui.small_button("Use in Custom filter").on_hover_text("Apply this pattern to the map now").clicked()
                {
                    out.request = Some(Request::ApplyFilter(text.trim().to_string()));
                }
            });
        });
    });
}

/// Tables are laid out by hand so cells wrap: each column gets its natural width when
/// everything fits, otherwise the widest columns give way.
fn table(ui: &mut egui::Ui, out: &mut Out, head: &[Vec<Span>], rows: &[Vec<Vec<Span>>]) {
    let cols = head.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if cols == 0 {
        return;
    }
    let pad = 8.0;
    let avail = ui.available_width();
    let mut natural = vec![30.0f32; cols];
    for (r, row) in std::iter::once(head).chain(rows.iter().map(|r| r.as_slice())).enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let (g, _) = layout(ui, cell, BODY, r == 0, f32::INFINITY);
            natural[c] = natural[c].max(g.size().x + 2.0 * pad);
        }
    }
    let widths = fit(&natural, avail);
    let vis = ui.visuals().clone();
    let line = vis.widgets.noninteractive.bg_stroke.color;
    for (r, row) in std::iter::once(head).chain(rows.iter().map(|r| r.as_slice())).enumerate() {
        let cells: Vec<_> = (0..cols)
            .map(|c| row.get(c).map(|cell| layout(ui, cell, BODY, r == 0, widths[c] - 2.0 * pad)))
            .collect();
        let h = cells.iter().flatten().map(|(g, _)| g.size().y).fold(0.0f32, f32::max) + 2.0 * 5.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(widths.iter().sum(), h), Sense::click());
        if r == 0 {
            ui.painter().rect_filled(rect, 0.0, vis.widgets.inactive.weak_bg_fill);
        } else if r % 2 == 0 {
            ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
        }
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, line));
        let mut x = rect.left();
        for (c, cell) in cells.into_iter().enumerate() {
            if let Some((g, links)) = cell {
                let at = pos2(x + pad, rect.top() + 5.0);
                let grect = Rect::from_min_size(at, g.size());
                if let Some(l) = hovered_link(&g, grect, &resp, &links) {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    if resp.clicked() {
                        out.follow(&l);
                    }
                }
                ui.painter().galley(at, g, vis.text_color());
            }
            x += widths[c];
        }
    }
}

/// Column widths that fit `avail`: natural widths if they fit, otherwise the widest columns are
/// narrowed to a common limit so short columns keep their width.
pub fn fit(natural: &[f32], avail: f32) -> Vec<f32> {
    let total: f32 = natural.iter().sum();
    if total <= avail {
        return natural.to_vec();
    }
    // Find the cap c where sum(min(w, c)) == avail.
    let (mut lo, mut hi) = (0.0f32, natural.iter().cloned().fold(0.0, f32::max));
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        let s: f32 = natural.iter().map(|w| w.min(mid)).sum();
        if s > avail { hi = mid } else { lo = mid }
    }
    natural.iter().map(|w| w.min(lo).max(40.0)).collect()
}

fn image(ui: &mut egui::Ui, out: &mut Out, src: &str, alt: &str) {
    let path = doc::join(out.page, src);
    let ctx = ui.ctx().clone();
    let tex = out
        .images
        .entry(path.clone())
        .or_insert_with(|| {
            let bytes = out.docs.image(&path)?;
            let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            Some(ctx.load_texture(format!("help:{path}"), ci, egui::TextureOptions::LINEAR))
        })
        .clone();
    match tex {
        Some(t) => {
            // Screenshots are taken at 1 pixel per point; never draw them larger than that.
            let [w, h] = t.size();
            let scale = (ui.available_width() / w as f32).min(1.0);
            let size = vec2(w as f32, h as f32) * scale;
            let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Outside);
            if !alt.is_empty() {
                resp.on_hover_text(alt);
                ui.label(egui::RichText::new(alt).italics().weak().size(BODY - 2.0));
            }
        }
        None => {
            ui.weak(format!("[{alt}]"));
        }
    }
}
