//! What's in a file, shown on the map and beside it.
//!
//! - Tile previews: as you zoom in, a file tile that grows big enough on screen shows its
//!   contents instead of a flat colour: a picture, a video frame, a PDF's first page, or the
//!   first lines of a text file or Word/OpenDocument document. Everything is read on the
//!   thumbnail workers (see `thumbs.rs`), only for tiles on screen, newest first, and kept in a
//!   memory-limited cache, so zooming never waits for a file. Small tiles get a small picture
//!   and big ones a sharper one, with the small one shown until it's ready. Only files on this
//!   computer are read: cloud files that aren't downloaded, mounted cloud drives, servers and
//!   archives never are, so zooming can't start a download.
//! - The Storage Analysis preview pane: the clicked tile's preview and facts about it.

use super::preview::Content;
use super::thumbs::{self, Config, Pic, Thumbs};
use super::*;
use crate::fsys::listing::Entry;
use eframe::egui::Vec2;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

/// Tiles start to show a preview at this size (points) and show it fully at `FULL`.
const MIN: Vec2 = vec2(70.0, 52.0);
const FULL: Vec2 = vec2(110.0, 82.0);
/// Picture sizes: the small one for most tiles, the sharp one once a tile is bigger than
/// `SHARP_FROM` pixels across.
const SMALL: u32 = 320;
const SHARP: u32 = 1024;
const SHARP_FROM: f32 = 400.0;
/// Most tiles given a preview in one frame (the biggest first).
const MOST: usize = 160;
/// Previews fade in over this long once loaded.
const FADE_IN: f32 = 0.18;

pub(super) struct TilePreviews {
    small: Thumbs,
    sharp: Thumbs,
    /// When each preview on screen first appeared, for the fade-in.
    born: HashMap<PathBuf, Instant>,
    /// A two-tone check, repeated behind pictures so transparent parts show as transparent
    /// rather than taking the tile's colour.
    checker: Option<egui::TextureHandle>,
    /// The video tile under the pointer, playing silently on the tile.
    hover_video: Option<HoverVideo>,
    /// A video tile the pointer has just come to rest on, and since when.
    hover_wait: Option<(PathBuf, Instant)>,
}

struct HoverVideo {
    path: PathBuf,
    video: crate::video::Video,
    tex: Option<egui::TextureHandle>,
    serial: u64,
}

/// How long the pointer rests on a video tile before it plays (so sweeping across the map
/// doesn't start one after another).
const HOVER_PLAY_AFTER: f32 = 0.35;

impl Default for TilePreviews {
    fn default() -> Self {
        TilePreviews {
            small: Thumbs::with(Config { max: SMALL, local_only: true, text: true, budget: 96 << 20 }),
            sharp: Thumbs::with(Config { max: SHARP, local_only: true, text: false, budget: 160 << 20 }),
            born: HashMap::new(),
            checker: None,
            hover_video: None,
            hover_wait: None,
        }
    }
}

/// Whether a tile for this file can show what's in it.
pub(super) fn previewable(name: &str) -> bool {
    thumbs::wants_thumb(name) || thumbs::text_kind(name).is_some()
}

/// Whether reading this file is safe while zooming: its data is on this computer.
pub(super) fn readable_here(n: &Node) -> bool {
    n.flags & (scan::store::NOT_LOCAL | scan::store::MOUNT) == 0
}

enum Got {
    Tex(egui::TextureId, Vec2),
    Text(Arc<str>),
    Wave(Arc<[f32]>),
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl App {
    /// Draw previews on the file tiles big enough for one. Returns the tiles (paths from the
    /// folder on show) that got one, so their names are drawn on the preview's caption instead.
    /// A video tile under the pointer (`hover`, a path from the folder on show) plays silently.
    pub(super) fn draw_tile_previews(&mut self, painter: &egui::Painter, tree: &Node, root: &[u32], bounds: Rect, hover: Option<&[u32]>) -> HashSet<Vec<u32>> {
        let mut shown = HashSet::new();
        let ctx = painter.ctx().clone();
        self.tile_prev.small.poll(&ctx);
        self.tile_prev.sharp.poll(&ctx);
        if !self.settings.tile_previews || self.map.in_transition() {
            self.tile_prev.hover_video = None;
            self.tile_prev.hover_wait = None;
            return shown;
        }
        let mut hover_seen = false;
        let Some(zroot) = tree.at(root) else { return shown };
        let mut tiles: Vec<(Rect, Vec<u32>, f32)> = self
            .map
            .labels(bounds)
            .into_iter()
            .filter(|(r, _, _)| r.width() >= MIN.x && r.height() >= MIN.y)
            .map(|(r, l, a)| (r, l.path.clone(), a))
            .collect();
        tiles.sort_by(|a, b| (b.0.area()).total_cmp(&a.0.area()));
        tiles.truncate(MOST);
        let ppp = ctx.pixels_per_point();
        let now = Instant::now();
        let mut fading = false;
        let mut seen = HashSet::new();
        for (r, rel, a) in tiles {
            let Some(n) = zroot.at(&rel) else { continue };
            if n.is_dir || !previewable(&n.name) || !readable_here(n) {
                continue;
            }
            let mut abs = root.to_vec();
            abs.extend_from_slice(&rel);
            let path = self.full_path(&abs);
            let stamp = thumbs::stamp(n.logical, n.modified);
            let big = r.width().max(r.height()) * ppp > SHARP_FROM;
            let tp = &mut self.tile_prev;
            let got = {
                let pick = |p: &Pic| match p {
                    Pic::Tex(t) => Got::Tex(t.id(), t.size_vec2()),
                    Pic::Text(s) => Got::Text(s.clone()),
                    Pic::Wave(w) => Got::Wave(w.clone()),
                };
                let sound = crate::media::media_of(&n.name) == Some(crate::media::Media::Audio);
                let sharp = if big && !sound && thumbs::text_kind(&n.name).is_none() { tp.sharp.pic(&path, stamp, n.logical).map(pick) } else { None };
                match sharp {
                    Some(g) => Some(g),
                    None => tp.small.pic(&path, stamp, n.logical).map(pick),
                }
            };
            let Some(got) = got else { continue };
            let born = *tp.born.entry(path.clone()).or_insert(now);
            seen.insert(path.clone());
            let fade_in = (now.duration_since(born).as_secs_f32() / FADE_IN).min(1.0);
            fading |= fade_in < 1.0;
            let alpha = a * fade_in * smooth(MIN.x, FULL.x, r.width()) * smooth(MIN.y, FULL.y, r.height());
            if alpha < 0.02 {
                continue;
            }
            // A frame of the tile's own colour stays round the preview, so the map still reads
            // by type.
            let inner = r.shrink((r.width().min(r.height()) * 0.03).clamp(2.0, 6.0));
            let clip = inner.intersect(painter.clip_rect());
            if clip.width() <= 0.0 || clip.height() <= 0.0 {
                continue;
            }
            let p = painter.with_clip_rect(clip);
            let pdf = crate::media::media_of(&n.name) == Some(crate::media::Media::Pdf);
            // The video under the pointer plays on its tile, silently, round and round.
            let mut got = got;
            if hover == Some(&rel[..]) && alpha > 0.9 && crate::media::media_of(&n.name) == Some(crate::media::Media::Video) && crate::video::supported() {
                hover_seen = true;
                if let Some(frame) = self.hover_video_frame(&ctx, &path, now) {
                    got = frame;
                }
            }
            match got {
                Got::Tex(id, size) if pdf => {
                    // A page: all of it, on a dark backdrop.
                    p.rect_filled(inner, 2.0, Color32::from_black_alpha((150.0 * alpha) as u8));
                    let s = ((inner.width() - 8.0) / size.x).min((inner.height() - 8.0) / size.y).max(0.0);
                    let page = Rect::from_center_size(inner.center(), size * s);
                    p.image(id, page, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(alpha));
                }
                Got::Tex(id, size) => {
                    // Checks behind it, seen only through transparent pixels.
                    let checker = self.tile_prev.checker.get_or_insert_with(|| {
                        let (a, b) = (Color32::from_gray(46), Color32::from_gray(62));
                        let img = egui::ColorImage::new([2, 2], vec![a, b, b, a]);
                        let opts = egui::TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Nearest, wrap_mode: egui::TextureWrapMode::Repeat, mipmap_mode: None };
                        ctx.load_texture("tile-checker", img, opts)
                    });
                    // Two checks per 16 points, fixed to the tile so they don't crawl as it moves.
                    let cells = inner.size() / 16.0;
                    p.image(checker.id(), inner, Rect::from_min_max(pos2(0.0, 0.0), pos2(cells.x, cells.y)), Color32::WHITE.gamma_multiply(alpha));
                    // A picture fills the tile, cropped to its shape from the middle.
                    let s = (inner.width() / size.x).max(inner.height() / size.y);
                    let (fx, fy) = (inner.width() / (size.x * s), inner.height() / (size.y * s));
                    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(fx, fy));
                    p.image(id, inner, uv, Color32::WHITE.gamma_multiply(alpha));
                }
                Got::Wave(w) => {
                    // A sound: its waveform in the tile's colour, on a dark version of it.
                    let c = self.node_color(n, &abs);
                    p.rect_filled(inner, 2.0, style::blend(Color32::from_rgb(14, 15, 18), c, 0.22).gamma_multiply(alpha));
                    let strip = if inner.height() >= 60.0 { 22.0 } else { 0.0 };
                    let area = Rect::from_min_max(inner.min + vec2(6.0, 6.0), pos2(inner.max.x - 6.0, inner.max.y - 6.0 - strip));
                    let bars = ((area.width() / 3.0) as usize).clamp(8, w.len().max(8));
                    let bw = area.width() / bars as f32;
                    let mid = area.center().y;
                    let ink = style::blend(c, Color32::WHITE, 0.15).gamma_multiply(alpha);
                    for i in 0..bars {
                        let v = w[i * w.len() / bars];
                        let h = (v * area.height() * 0.5).max(0.75);
                        let x = area.left() + i as f32 * bw;
                        p.rect_filled(Rect::from_min_max(pos2(x + bw * 0.15, mid - h), pos2(x + bw * 0.85, mid + h)), 1.0, ink);
                    }
                }
                Got::Text(text) => {
                    // A page of text, as much as fits; the type grows as the tile does.
                    p.rect_filled(inner, 2.0, Color32::from_rgb(20, 22, 26).gamma_multiply(alpha));
                    let size = (inner.width() / 46.0).clamp(7.0, 15.0);
                    let font = FontId::monospace(size);
                    let row = size * 1.3;
                    let rows = ((inner.height() - 22.0) / row).max(1.0) as usize;
                    let cols = ((inner.width() - 10.0) / (size * 0.6)).max(4.0) as usize;
                    let mut shown_text = String::new();
                    for line in text.lines().take(rows) {
                        shown_text.extend(line.chars().take(cols));
                        shown_text.push('\n');
                    }
                    p.text(inner.min + vec2(6.0, 5.0), Align2::LEFT_TOP, shown_text.trim_end(), font, Color32::from_rgb(214, 218, 224).gamma_multiply(alpha));
                }
            }
            // The name on a dark strip along the bottom.
            if inner.height() >= 60.0 {
                let font = FontId::proportional((r.width() / 22.0).clamp(11.0, 14.0));
                let strip = Rect::from_min_max(pos2(inner.left(), inner.bottom() - font.size - 8.0), inner.max);
                p.rect_filled(strip, 0.0, Color32::from_black_alpha((165.0 * alpha) as u8));
                let text = format!("{}  ·  {}", n.name, scan::human(n.size));
                p.text(pos2(strip.left() + 6.0, strip.center().y), Align2::LEFT_CENTER, text, font, Color32::from_white_alpha(240).gamma_multiply(alpha));
            }
            if alpha > 0.35 {
                shown.insert(rel);
            }
        }
        let tp = &mut self.tile_prev;
        if !hover_seen {
            tp.hover_video = None;
            tp.hover_wait = None;
        }
        tp.born.retain(|p, _| seen.contains(p));
        tp.small.flush();
        tp.sharp.flush();
        if fading {
            ctx.request_repaint();
        }
        shown
    }

    /// The newest picture of the video playing under the pointer, starting it once the pointer
    /// has rested there a moment.
    fn hover_video_frame(&mut self, ctx: &egui::Context, path: &Path, now: Instant) -> Option<Got> {
        let tp = &mut self.tile_prev;
        if tp.hover_video.as_ref().is_some_and(|h| h.path != path) {
            tp.hover_video = None;
        }
        if tp.hover_video.is_none() {
            match &tp.hover_wait {
                Some((p, since)) if p == path => {
                    if now.duration_since(*since).as_secs_f32() < HOVER_PLAY_AFTER {
                        ctx.request_repaint_after(std::time::Duration::from_millis(60));
                        return None;
                    }
                }
                _ => {
                    tp.hover_wait = Some((path.to_path_buf(), now));
                    ctx.request_repaint_after(std::time::Duration::from_millis(60));
                    return None;
                }
            }
            let c = ctx.clone();
            let video = crate::video::Video::start(path, false, move || c.request_repaint());
            tp.hover_video = Some(HoverVideo { path: path.to_path_buf(), video, tex: None, serial: 0 });
        }
        let h = tp.hover_video.as_mut()?;
        if h.video.state().ended {
            h.video.play();
        }
        if let Some(f) = h.video.take_frame(&mut h.serial) {
            let img = egui::ColorImage::from_rgba_unmultiplied(f.size, &f.rgba);
            match &mut h.tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => h.tex = Some(ctx.load_texture("tile-video", img, egui::TextureOptions::LINEAR)),
            }
        }
        h.tex.as_ref().map(|t| Got::Tex(t.id(), t.size_vec2()))
    }

    // ---------- the Storage Analysis preview pane ----------

    /// The pane at the right of the map: the clicked tile's preview and what's known about it.
    pub(super) fn map_preview_pane(&mut self, ui: &mut egui::Ui) {
        let pal = style::pal(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            style::section_title(ui, "Preview");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::icon_button(ui, true, "✖", "Close the preview pane (Alt+P)").clicked() {
                    self.files.prefs.map_preview = false;
                }
            });
        });
        let Some(tree) = self.shown_tree() else { return };
        let Some(sel) = self.selected.clone() else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("Click a tile to preview it").color(pal.weak)));
            return;
        };
        let Some(n) = tree.at(&sel) else { return };
        let path = self.full_path(&sel);
        let e = Entry {
            name: n.name.clone(),
            is_dir: n.is_dir,
            is_link: false,
            hidden: false,
            size: n.logical,
            alloc: n.alloc,
            flags: n.flags,
            modified: n.modified,
        };
        ui.add_space(4.0);
        let w = ui.available_width();
        if n.is_dir {
            let (r, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
            self.icons.paint(ui, Rect::from_center_size(r.center(), vec2(64.0, 64.0)), true, "", Color32::GRAY);
        } else if crate::fsys::remote::is_remote(&path) && !previewable(&n.name) {
            // Nothing worth fetching from a server.
        } else {
            let h = (ui.available_height() * 0.5).clamp(120.0, w * 1.2);
            self.preview_body(ui, &path, &e, vec2(w, h));
        }
        ui.add_space(8.0);
        ui.add(egui::Label::new(egui::RichText::new(&*n.name).strong()).wrap());
        ui.add_space(4.0);
        let total = tree.size.max(1);
        let folder = tree.at(&self.zoom).filter(|_| sel.starts_with(&self.zoom) && sel.len() > self.zoom.len());
        let mut go = None;
        egui::Grid::new("map-preview-facts").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            ui.weak("Kind");
            ui.label(browser::kind_for(&n.name, n.is_dir));
            ui.end_row();
            ui.weak("Size");
            ui.add(egui::Label::new(sizes_text(n, self.metric)).wrap());
            ui.end_row();
            if !n.is_dir {
                ui.weak("Bytes");
                ui.label(group(n.logical));
                ui.end_row();
            }
            if n.is_dir {
                ui.weak("Files");
                ui.label(group(n.files));
                ui.end_row();
            }
            ui.weak("Share");
            let mut share = format!("{:.1}% of the analysis", n.size as f64 * 100.0 / total as f64);
            if let Some(f) = folder.filter(|f| f.size > 0 && !std::ptr::eq(*f, &*tree)) {
                share.push_str(&format!(", {:.1}% of {}", n.size as f64 * 100.0 / f.size as f64, f.name));
            }
            ui.add(egui::Label::new(share).wrap());
            ui.end_row();
            if n.modified > 0 {
                ui.weak(if n.is_dir { "Newest" } else { "Modified" });
                ui.label(fmt_time(n.modified));
                ui.end_row();
            }
            if n.flags & scan::store::CLOUD != 0 || n.flags & scan::store::NOT_LOCAL != 0 {
                ui.weak("Stored");
                ui.label(if n.flags & scan::store::ONLINE_ONLY != 0 {
                    "Only in the cloud"
                } else if n.flags & scan::store::PARTIAL != 0 {
                    "Partly downloaded"
                } else if n.flags & scan::store::MOUNT != 0 {
                    "On a mounted cloud drive"
                } else if n.flags & scan::store::PINNED != 0 {
                    "Kept on this computer"
                } else {
                    "Downloaded"
                });
                ui.end_row();
            }
            if let Some(Content::Image { size, info, poster: false, .. }) = self.files.look.cached(&path) {
                ui.weak("Dimensions");
                ui.label(format!("{} × {}", size[0], size[1]));
                ui.end_row();
                ui.weak("Pixels");
                ui.label(info);
                ui.end_row();
            }
            ui.weak("Where");
            ui.add(egui::Label::new(path.parent().map(|p| p.shown()).unwrap_or_default()).wrap());
            ui.end_row();
        });
        // A folder: what takes the most room in it.
        if n.is_dir && !n.children.is_empty() {
            ui.add_space(8.0);
            ui.weak("Largest inside");
            ui.add_space(2.0);
            let mut kids: Vec<(usize, &Node)> = n.children.iter().enumerate().collect();
            kids.sort_by(|a, b| b.1.size.cmp(&a.1.size));
            egui::Grid::new("map-preview-largest").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
                for (i, k) in kids.into_iter().take(8) {
                    let name = if k.is_dir { format!("{}/", k.name) } else { k.name.to_string() };
                    if ui.add(egui::Button::selectable(false, name).truncate()).on_hover_text("Select it").clicked() {
                        let mut p = sel.clone();
                        p.push(i as u32);
                        go = Some(p);
                    }
                    ui.label(egui::RichText::new(scan::human(k.size)).color(pal.weak));
                    ui.end_row();
                }
            });
        }
        if self.settings.system_notes && !crate::fsys::remote::is_remote(&path) {
            super::explain::preview_section(ui, &path, &n.name);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Open").clicked() {
                let p = path.clone();
                self.files.spawn(move || crate::actions::open(&p));
            }
            if ui.button(if cfg!(target_os = "macos") { "Show in Finder" } else { "Show in folder" }).clicked() {
                let p = path.clone();
                self.files.spawn(move || crate::actions::reveal(&p));
            }
        });
        if let Some(p) = go {
            self.select(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_tiles_can_show_contents() {
        assert!(previewable("a.png"));
        assert!(previewable("report.pdf"));
        assert!(previewable("notes.md"));
        assert!(previewable("letter.docx"));
        assert!(previewable("essay.odt"));
        assert!(!previewable("app.exe"));
        assert!(!previewable("icon.svg") || thumbs::wants_thumb("icon.svg"));
    }

    #[test]
    fn cloud_files_are_never_read_for_tiles() {
        let mut n = Node { name: "a.png".into(), size: 10, logical: 10, alloc: 0, files: 1, modified: 0, is_dir: false, flags: 0, children: vec![] };
        assert!(readable_here(&n));
        n.flags = scan::store::CLOUD;
        assert!(readable_here(&n), "a downloaded cloud file is on this computer");
        for f in [scan::store::ONLINE_ONLY, scan::store::PARTIAL, scan::store::MOUNT] {
            n.flags = scan::store::CLOUD | f;
            assert!(!readable_here(&n));
        }
    }

    #[test]
    fn documents_give_their_text() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("filescape-docx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.docx");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        z.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(br#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t xml:space="preserve"> world</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        z.finish().unwrap();
        assert_eq!(thumbs::doc_text(&p, thumbs::TextKind::Docx).unwrap(), "Hello world\nSecond\n");
        let o = dir.join("b.odt");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&o).unwrap());
        z.start_file("content.xml", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(br#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:h>Title</text:h><text:p>Some <text:span>styled</text:span> text</text:p></office:text></office:body></office:document-content>"#).unwrap();
        z.finish().unwrap();
        assert_eq!(thumbs::doc_text(&o, thumbs::TextKind::Odt).unwrap(), "Title\nSome styled text\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}
