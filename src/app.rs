//! The desktop GUI: directory tree, extension list and treemap.

use crate::drives::{self, Drive};
use crate::scan::{self, ExtStat, Node, Options, Progress};
use crate::treemap::{self, Tile};
use eframe::egui::{self, ecolor::Hsva, pos2, vec2, Color32, ColorImage, Rect, Sense, Stroke, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[derive(PartialEq, Clone, Copy)]
enum SortBy {
    Size,
    Name,
    Files,
}

struct Job {
    progress: Arc<Progress>,
    rx: mpsc::Receiver<Node>,
    started: Instant,
}

/// The treemap is rendered once into a texture at physical-pixel resolution and then drawn as a
/// single image, so ordinary repaints (hover, scrolling, moving the window) cost almost nothing.
struct LayoutCache {
    /// Size in physical pixels.
    px: [usize; 2],
    zoom: Vec<u32>,
    highlight: Option<String>,
    /// Tiles in pixel coordinates relative to the treemap's top-left corner.
    tiles: Vec<Tile>,
    texture: TextureHandle,
}

/// One visible line of the directory tree.
struct Row {
    path: Box<[u32]>,
    depth: u16,
}

pub struct App {
    path_input: String,
    scanned_path: PathBuf,
    opts: Options,
    job: Option<Job>,
    tree: Option<Node>,
    scan_secs: f32,
    exts: Vec<ExtStat>,
    colors: HashMap<String, Color32>,
    selected: Option<Vec<u32>>,
    reveal: bool,
    zoom: Vec<u32>,
    sort: SortBy,
    highlight_ext: Option<String>,
    cache: Option<LayoutCache>,
    /// When the treemap's size last changed; it is re-rendered once the size settles.
    resize_since: Option<Instant>,
    expanded: HashSet<Box<[u32]>>,
    rows: Option<Vec<Row>>,
    drives: Vec<Drive>,
    drives_rx: Option<mpsc::Receiver<Vec<Drive>>>,
    show_drives: bool,
}

impl App {
    pub fn new(path: Option<PathBuf>, opts: Options) -> Self {
        let mut app = Self {
            path_input: path
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            scanned_path: PathBuf::new(),
            opts,
            job: None,
            tree: None,
            scan_secs: 0.0,
            exts: Vec::new(),
            colors: HashMap::new(),
            selected: None,
            reveal: false,
            zoom: Vec::new(),
            sort: SortBy::Size,
            highlight_ext: None,
            cache: None,
            resize_since: None,
            expanded: HashSet::new(),
            rows: None,
            drives: Vec::new(),
            drives_rx: None,
            show_drives: false,
        };
        app.refresh_drives();
        if path.is_some() {
            app.start_scan();
        }
        app
    }

    fn start_scan(&mut self) {
        let mut input = self.path_input.trim().to_string();
        // "D:" alone means "current directory on D:" to Windows; the user means the whole drive.
        if cfg!(windows) && input.len() == 2 && input.ends_with(':') {
            input.push('\\');
            self.path_input = input.clone();
        }
        let path = PathBuf::from(input);
        if let Some(job) = &self.job {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let (p, opts, root) = (progress.clone(), self.opts, path.clone());
        std::thread::spawn(move || {
            let _ = tx.send(scan::scan(&root, opts, &p));
        });
        self.scanned_path = path;
        self.job = Some(Job { progress, rx, started: Instant::now() });
    }

    fn finish_scan(&mut self, tree: Node, secs: f32) {
        self.exts = scan::ext_stats(&tree);
        self.colors = self
            .exts
            .iter()
            .enumerate()
            .map(|(i, e)| (e.ext.clone(), ext_color(&e.ext, i)))
            .collect();
        self.tree = Some(tree);
        self.scan_secs = secs;
        self.selected = None;
        self.zoom.clear();
        self.highlight_ext = None;
        self.cache = None;
        self.expanded.clear();
        self.expanded.insert(Box::new([]));
        self.rows = None;
    }

    fn color_of(&self, n: &Node) -> Color32 {
        let ext = n.ext();
        match self.colors.get(ext) {
            Some(&c) => c,
            None => self.colors.get(&ext.to_lowercase()).copied().unwrap_or(Color32::GRAY),
        }
    }

    fn full_path(&self, path: &[u32]) -> PathBuf {
        let mut p = self.scanned_path.clone();
        let mut n = self.tree.as_ref().unwrap();
        for &i in path {
            n = &n.children[i as usize];
            p.push(&*n.name);
        }
        p
    }

    fn select(&mut self, path: Vec<u32>) {
        // Expand every ancestor in the tree view so the selection is visible.
        for k in 0..path.len() {
            self.expanded.insert(path[..k].into());
        }
        self.rows = None;
        self.selected = Some(path);
        self.reveal = true;
    }

    /// Enumerate drives on a background thread: a slow or disconnected network share must not
    /// freeze the window.
    fn refresh_drives(&mut self) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(drives::list());
        });
        self.drives_rx = Some(rx);
    }

    /// Table of drives like WinDirStat's "Select Drives" dialog. Returns the root to scan if one was picked.
    fn drive_table(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        if let Some(rx) = &self.drives_rx {
            match rx.try_recv() {
                Ok(d) => {
                    self.drives = d;
                    self.drives_rx = None;
                }
                Err(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Looking for drives…");
                    });
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        let mut picked = None;
        egui::Grid::new("drives").striped(true).num_columns(6).spacing([14.0, 6.0]).show(ui, |ui| {
            for h in ["Drive", "Total", "Free", "Used", "", ""] {
                ui.strong(h);
            }
            ui.end_row();
            for d in &self.drives {
                let name = if d.label.is_empty() {
                    d.root.display().to_string()
                } else {
                    format!("{}  ({})", d.root.display(), d.label)
                };
                let name = if d.network { format!("{name}  [network]") } else { name };
                if ui.link(name).on_hover_text("Scan this drive").clicked() {
                    picked = Some(d.root.clone());
                }
                ui.label(scan::human(d.total));
                ui.label(scan::human(d.free));
                let frac = if d.total > 0 { d.used() as f32 / d.total as f32 } else { 0.0 };
                let (r, _) = ui.allocate_exact_size(vec2(140.0, 14.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, ui.visuals().extreme_bg_color);
                let mut fill = r;
                fill.set_width(r.width() * frac);
                ui.painter().rect_filled(fill, 2.0, Color32::from_rgb(90, 90, 200));
                ui.label(format!("{:.1}%", frac * 100.0));
                if ui.button("Scan").clicked() {
                    picked = Some(d.root.clone());
                }
                ui.end_row();
            }
        });
        if self.drives_rx.is_none() && self.drives.is_empty() {
            ui.label("No drives found. Type a folder path above instead.");
        }
        picked
    }

    fn scan_root(&mut self, root: PathBuf) {
        self.path_input = root.display().to_string();
        self.show_drives = false;
        self.start_scan();
    }

    // ---------- toolbar ----------

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Drives…").clicked() {
                self.show_drives = !self.show_drives;
                if self.show_drives {
                    self.refresh_drives();
                }
            }
            ui.label("Folder:");
            let edit = ui.add(egui::TextEdit::singleline(&mut self.path_input).desired_width(400.0));
            if ui.button("Browse…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().set_directory(self.path_input.trim()).pick_folder() {
                    self.path_input = dir.display().to_string();
                    self.start_scan();
                }
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Scan").clicked() || enter {
                self.start_scan();
            }
            ui.checkbox(&mut self.opts.one_file_system, "Stay on this filesystem");
            ui.separator();
            ui.add_enabled_ui(self.tree.is_some(), |ui| {
                if ui.button("Zoom to selection").clicked() {
                    if let Some(sel) = &self.selected {
                        let n = self.tree.as_ref().unwrap();
                        let mut z = sel.clone();
                        if !n.at(&z).map(|n| n.is_dir).unwrap_or(false) {
                            z.pop();
                        }
                        self.zoom = z;
                    }
                }
                if ui.button("Zoom out").clicked() {
                    self.zoom.pop();
                }
            });
        });
    }

    // ---------- directory tree ----------

    fn tree_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Sort:");
            let before = self.sort;
            ui.selectable_value(&mut self.sort, SortBy::Size, "Size");
            ui.selectable_value(&mut self.sort, SortBy::Name, "Name");
            ui.selectable_value(&mut self.sort, SortBy::Files, "Files");
            if self.sort != before {
                self.rows = None;
            }
        });
        ui.separator();
        let Some(tree) = self.tree.take() else { return };
        if self.rows.is_none() {
            self.rows = Some(self.flatten(&tree));
        }
        let rows = self.rows.take().unwrap();

        // Only the rows inside the viewport are laid out, so huge folders stay fast.
        let row_h = ui.text_style_height(&egui::TextStyle::Body).max(16.0);
        let mut area = egui::ScrollArea::both().auto_shrink(false);
        if self.reveal {
            if let Some(i) = self.selected.as_ref().and_then(|s| rows.iter().position(|r| *r.path == **s)) {
                let step = row_h + ui.spacing().item_spacing.y;
                area = area.vertical_scroll_offset((i as f32 * step - ui.available_height() / 2.0).max(0.0));
            }
            self.reveal = false;
        }
        let mut clicked = None;
        let mut toggled = None;
        area.show_rows(ui, row_h, rows.len(), |ui, range| {
            for row in &rows[range] {
                let n = tree.at(&row.path).unwrap();
                let parent_size = if row.path.is_empty() { n.size } else { tree.at(&row.path[..row.path.len() - 1]).unwrap().size };
                ui.horizontal(|ui| {
                    ui.set_height(row_h);
                    ui.add_space(row.depth as f32 * 14.0);
                    if n.is_dir {
                        let icon = if self.expanded.contains(&row.path) { "⏷" } else { "⏵" };
                        if ui.add(egui::Label::new(icon).sense(Sense::click())).clicked() {
                            toggled = Some(row.path.clone());
                        }
                    } else {
                        ui.add_space(ui.spacing().icon_width);
                    }
                    self.tree_row(ui, n, parent_size, &row.path, &mut clicked, &mut toggled);
                });
            }
        });
        self.rows = Some(rows);
        self.tree = Some(tree);
        if let Some(p) = toggled {
            if !self.expanded.remove(&p) {
                self.expanded.insert(p);
            }
            self.rows = None;
        }
        if let Some(p) = clicked {
            self.selected = Some(p);
        }
    }

    fn tree_row(&self, ui: &mut egui::Ui, n: &Node, parent_size: u64, path: &[u32], clicked: &mut Option<Vec<u32>>, toggled: &mut Option<Box<[u32]>>) {
        let frac = if parent_size > 0 { n.size as f32 / parent_size as f32 } else { 0.0 };
        let (r, _) = ui.allocate_exact_size(vec2(50.0, 12.0), Sense::hover());
        ui.painter().rect_filled(r, 2.0, ui.visuals().extreme_bg_color);
        let mut fill = r;
        fill.set_width(r.width() * frac);
        let c = if n.is_dir { Color32::from_rgb(90, 140, 220) } else { self.color_of(n) };
        ui.painter().rect_filled(fill, 2.0, c);
        ui.add_sized([72.0, 16.0], egui::Label::new(scan::human(n.size)));
        ui.add_sized([44.0, 16.0], egui::Label::new(egui::RichText::new(format!("{:.1}%", frac * 100.0)).weak()));
        let label = if n.is_dir { format!("{}/  ({} files)", n.name, n.files) } else { n.name.to_string() };
        let is_sel = self.selected.as_deref() == Some(path);
        let resp = ui.selectable_label(is_sel, label);
        if resp.clicked() {
            *clicked = Some(path.to_vec());
        }
        if resp.double_clicked() && n.is_dir {
            *toggled = Some(path.into());
        }
    }

    /// The visible rows: every child of every expanded folder, in the chosen sort order.
    fn flatten(&self, tree: &Node) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut path = Vec::new();
        self.flatten_into(tree, &mut path, &mut rows);
        rows
    }

    fn flatten_into(&self, n: &Node, path: &mut Vec<u32>, rows: &mut Vec<Row>) {
        rows.push(Row { path: path.as_slice().into(), depth: path.len() as u16 });
        if !n.is_dir || !self.expanded.contains(path.as_slice()) {
            return;
        }
        let mut order: Vec<usize> = (0..n.children.len()).collect();
        match self.sort {
            SortBy::Size => {}
            SortBy::Name => order.sort_by_cached_key(|&i| n.children[i].name.to_lowercase()),
            SortBy::Files => order.sort_by(|&a, &b| n.children[b].files.cmp(&n.children[a].files)),
        }
        for i in order {
            path.push(i as u32);
            self.flatten_into(&n.children[i], path, rows);
            path.pop();
        }
    }

    // ---------- extension list ----------

    fn ext_panel(&mut self, ui: &mut egui::Ui) {
        let total: u64 = self.tree.as_ref().map(|t| t.size).unwrap_or(0).max(1);
        ui.heading("Extensions");
        if self.highlight_ext.is_some() && ui.small_button("Clear highlight").clicked() {
            self.highlight_ext = None;
        }
        let mut toggle = None;
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 16.0], egui::Label::new(egui::RichText::new("Ext").strong()));
            ui.add_sized([70.0, 16.0], egui::Label::new(egui::RichText::new("Size").strong()));
            ui.add_sized([40.0, 16.0], egui::Label::new(egui::RichText::new("%").strong()));
            ui.label(egui::RichText::new("Files").strong());
        });
        let row_h = ui.text_style_height(&egui::TextStyle::Body).max(16.0);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_h, self.exts.len(), |ui, range| {
            for e in &self.exts[range] {
                ui.horizontal(|ui| {
                    ui.set_height(row_h);
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, self.colors[&e.ext]);
                    let name = if e.ext.is_empty() { "(none)".to_string() } else { format!(".{}", e.ext) };
                    let on = self.highlight_ext.as_deref() == Some(e.ext.as_str());
                    let resp = ui.add_sized([92.0, 16.0], egui::Button::selectable(on, name).truncate());
                    if resp.on_hover_text("Click to highlight in treemap").clicked() {
                        toggle = Some(e.ext.clone());
                    }
                    ui.add_sized([70.0, 16.0], egui::Label::new(scan::human(e.bytes)));
                    ui.add_sized([40.0, 16.0], egui::Label::new(format!("{:.1}", e.bytes as f64 * 100.0 / total as f64)));
                    ui.label(e.files.to_string());
                });
            }
        });
        if let Some(t) = toggle {
            self.highlight_ext = if self.highlight_ext.as_ref() == Some(&t) { None } else { Some(t) };
        }
    }

    // ---------- treemap ----------

    fn treemap_panel(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click());
        let bounds = resp.rect;
        let ppp = ui.ctx().pixels_per_point();
        let px = [(bounds.width() * ppp).round().max(1.0) as usize, (bounds.height() * ppp).round().max(1.0) as usize];
        let Some(tree) = &self.tree else { return };
        if tree.at(&self.zoom).is_none() {
            self.zoom.clear();
        }

        let (content_changed, size_changed) = match &self.cache {
            Some(c) => (c.zoom != self.zoom || c.highlight != self.highlight_ext, c.px != px),
            None => (true, false),
        };
        // While the window is being resized or moved to a monitor with a different scale,
        // stretch the old image and only re-render once the size has settled.
        let mut rebuild = content_changed;
        if size_changed {
            let since = *self.resize_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(150) {
                rebuild = true;
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(160));
            }
        }
        if rebuild {
            self.resize_since = None;
            self.render_treemap(ui.ctx(), px);
        }
        let Some(cache) = self.cache.as_ref() else { return };
        let tree = self.tree.as_ref().unwrap();
        painter.image(cache.texture.id(), bounds, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

        // Tile rects are in the cached image's pixels; map them onto the current bounds.
        let (sx, sy) = (bounds.width() / cache.px[0] as f32, bounds.height() / cache.px[1] as f32);
        let to_screen = |r: Rect| Rect::from_min_max(
            pos2(bounds.min.x + r.min.x * sx, bounds.min.y + r.min.y * sy),
            pos2(bounds.min.x + r.max.x * sx, bounds.min.y + r.max.y * sy),
        );
        let zl = self.zoom.len();

        // Outline the selection (or the nearest visible ancestor of it).
        if let Some(sel) = self.selected.as_ref().filter(|s| s.starts_with(&self.zoom)) {
            let rel = &sel[zl..];
            if let Some(t) = cache.tiles.iter().filter(|t| rel.starts_with(&t.path)).max_by_key(|t| t.path.len()) {
                painter.rect_stroke(to_screen(t.rect), 0.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
            }
        }

        let hover = resp.hover_pos().and_then(|p| {
            let local = pos2((p.x - bounds.min.x) / sx, (p.y - bounds.min.y) / sy);
            cache.tiles.iter().rev().find(|t| t.rect.contains(local))
        });
        let mut action = None;
        if let Some(t) = hover {
            if let Some(parent) = cache.tiles.iter().find(|p| p.is_dir && p.path.len() + 1 == t.path.len() && t.path.starts_with(&p.path)) {
                painter.rect_stroke(to_screen(parent.rect), 0.0, Stroke::new(1.0, Color32::from_white_alpha(140)), egui::StrokeKind::Inside);
            }
            let mut abs = self.zoom.clone();
            abs.extend_from_slice(&t.path);
            let n = tree.at(&abs).unwrap();
            let txt = format!("{}\n{}", self.full_path(&abs).display(), scan::human(n.size));
            resp.clone().on_hover_text_at_pointer(txt);
            if resp.double_clicked() {
                // Zoom into the directory that holds the double-clicked tile.
                let mut z = abs.clone();
                if !n.is_dir {
                    z.pop();
                }
                action = Some((true, z));
            } else if resp.clicked() {
                action = Some((false, abs));
            }
        }
        match action {
            Some((true, z)) => self.zoom = z,
            Some((false, p)) => self.select(p),
            None => {}
        }
        if resp.secondary_clicked() {
            self.zoom.pop();
        }
    }

    fn render_treemap(&mut self, ctx: &egui::Context, px: [usize; 2]) {
        let tree = self.tree.as_ref().unwrap();
        let zroot = tree.at(&self.zoom).unwrap();
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(px[0] as f32, px[1] as f32));
        let tiles = treemap::layout(zroot, bounds);
        let mut pixels = vec![Color32::from_gray(45); px[0] * px[1]];
        let hl = self.highlight_ext.as_deref();
        for (i, t) in tiles.iter().enumerate() {
            // Tiles come parents-first; only paint the ones nothing else is drawn on top of.
            let has_children = tiles.get(i + 1).is_some_and(|nx| nx.path.len() > t.path.len());
            if has_children {
                continue;
            }
            let n = zroot.at(&t.path).unwrap();
            if n.is_dir {
                continue; // contents too small to draw: leave the background
            }
            let mut c = self.color_of(n);
            if let Some(h) = hl {
                if !n.ext().eq_ignore_ascii_case(h) {
                    c = c.gamma_multiply(0.15);
                }
            }
            paint_cushion(&mut pixels, px[0], t.rect, c);
        }
        let img = ColorImage::new(px, pixels);
        match &mut self.cache {
            Some(cache) => {
                cache.texture.set(img, TextureOptions::LINEAR);
                cache.px = px;
                cache.zoom = self.zoom.clone();
                cache.highlight = self.highlight_ext.clone();
                cache.tiles = tiles;
            }
            None => {
                let texture = ctx.load_texture("treemap", img, TextureOptions::LINEAR);
                self.cache = Some(LayoutCache { px, zoom: self.zoom.clone(), highlight: self.highlight_ext.clone(), tiles, texture });
            }
        }
    }
}

/// Fill a tile with a "cushion": a gently domed surface lit from the top-left, like WinDirStat.
fn paint_cushion(pixels: &mut [Color32], stride: usize, r: Rect, c: Color32) {
    let height = pixels.len() / stride;
    let (x0, x1) = (r.min.x.round().max(0.0) as usize, (r.max.x.round() as usize).min(stride));
    let (y0, y1) = (r.min.y.round().max(0.0) as usize, (r.max.y.round() as usize).min(height));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let (w, h) = ((x1 - x0) as f32, (y1 - y0) as f32);
    let (cr, cg, cb) = (c.r() as f32, c.g() as f32, c.b() as f32);
    // Light direction (-1, -1, 2) normalised.
    const L: [f32; 3] = [-0.408, -0.408, 0.816];
    for y in y0..y1 {
        let v = (y - y0) as f32 / h + 0.5 / h;
        let ny = -(1.0 - 2.0 * v);
        let row = &mut pixels[y * stride + x0..y * stride + x1];
        for (k, p) in row.iter_mut().enumerate() {
            let u = k as f32 / w + 0.5 / w;
            let nx = -(1.0 - 2.0 * u);
            let len = (nx * nx + ny * ny + 1.0).sqrt();
            let lit = ((nx * L[0] + ny * L[1] + L[2]) / len).max(0.0);
            let f = 0.25 + 0.95 * lit;
            *p = Color32::from_rgb((cr * f).min(255.0) as u8, (cg * f).min(255.0) as u8, (cb * f).min(255.0) as u8);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Collect a finished scan.
        if let Some(job) = &self.job {
            if let Ok(tree) = job.rx.try_recv() {
                let secs = job.started.elapsed().as_secs_f32();
                let cancelled = job.progress.cancel.load(Ordering::Relaxed);
                self.job = None;
                if !cancelled {
                    self.finish_scan(tree, secs);
                }
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));

        if self.show_drives && self.tree.is_some() {
            let mut open = true;
            let mut picked = None;
            egui::Window::new("Select drive").open(&mut open).collapsible(false).show(ui.ctx(), |ui| {
                picked = self.drive_table(ui);
            });
            self.show_drives = open;
            if let Some(root) = picked {
                self.scan_root(root);
            }
        }

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(job) = &self.job {
                    ui.spinner();
                    let p = &job.progress;
                    ui.label(format!(
                        "Scanning… {} files, {} folders, {} ({:.1}s)",
                        p.files.load(Ordering::Relaxed),
                        p.dirs.load(Ordering::Relaxed),
                        scan::human(p.bytes.load(Ordering::Relaxed)),
                        job.started.elapsed().as_secs_f32()
                    ));
                    if ui.button("Cancel").clicked() {
                        p.cancel.store(true, Ordering::Relaxed);
                    }
                } else if let Some(t) = &self.tree {
                    ui.label(format!(
                        "{}: {} in {} files, scanned in {:.1}s. Click a tile to select, double-click to zoom in, right-click to zoom out.",
                        self.scanned_path.display(),
                        scan::human(t.size),
                        t.files,
                        self.scan_secs
                    ));
                } else {
                    ui.label("Pick a folder and press Scan.");
                }
                if let Some(sel) = &self.selected {
                    ui.separator();
                    ui.label(self.full_path(sel).display().to_string());
                }
            });
        });

        if self.tree.is_none() {
            egui::CentralPanel::default().show(ui, |ui| {
                if self.job.is_some() {
                    ui.centered_and_justified(|ui| ui.heading("Scanning…"));
                } else {
                    ui.heading("Pick a drive to scan, or type a folder above");
                    ui.add_space(8.0);
                    if let Some(root) = self.drive_table(ui) {
                        self.scan_root(root);
                    }
                }
            });
            return;
        }

        egui::Panel::right("exts").default_size(260.0).resizable(true).show(ui, |ui| self.ext_panel(ui));
        egui::Panel::top("tree").default_size(320.0).resizable(true).show(ui, |ui| self.tree_panel(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.treemap_panel(ui));
    }
}


/// The biggest extensions get bright, well-separated hues; the long tail gets muted ones.
fn ext_color(ext: &str, rank: usize) -> Color32 {
    if rank < 16 {
        let hue = (rank as f32 * 0.618_034).fract();
        Hsva::new(hue, 0.75, 0.95, 1.0).into()
    } else {
        let h = ext.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
        Hsva::new((h % 360) as f32 / 360.0, 0.3, 0.65, 1.0).into()
    }
}
