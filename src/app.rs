//! The desktop GUI: directory tree, extension list and treemap.

use crate::categories::{self, Category};
use crate::drives::{self, Drive};
use crate::icons::Icons;
use crate::scan::{self, ExtStat, LiveDir, Node, Options, Progress};
use crate::settings::{self, Settings};
use crate::treemap::{self, Tile};
use eframe::egui::{
    self, ecolor::Hsva, pos2, vec2, Align2, Color32, ColorImage, FontId, Rect, Sense, Stroke, TextureHandle, TextureOptions,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[derive(PartialEq, Clone, Copy)]
enum SortBy {
    Name,
    Size,
    Files,
    Modified,
}

struct Job {
    root: PathBuf,
    progress: Arc<Progress>,
    live: Arc<LiveDir>,
    rx: mpsc::Receiver<(Node, Vec<ExtStat>)>,
    started: Instant,
    /// Used space on the drive when scanning a whole drive, to estimate percent complete.
    expected: Option<u64>,
}

/// The treemap is rendered once into a texture at physical-pixel resolution and then drawn as a
/// single image, so ordinary repaints (hover, scrolling, moving the window) cost almost nothing.
struct LayoutCache {
    /// Size in physical pixels.
    px: [usize; 2],
    zoom: Vec<u32>,
    highlight: Option<String>,
    look: [u32; 9],
    filter: ViewFilter,
    /// Tiles in pixel coordinates relative to the treemap's top-left corner.
    tiles: Vec<Tile>,
    /// Base color of each painted file tile (transparent for folders and blanked-out tiles),
    /// used to pick a readable label color.
    colors: Vec<Color32>,
    texture: TextureHandle,
}

/// Which files the treemap shows; everything else is blanked out. Doesn't need a rescan.
#[derive(Clone, PartialEq, Default)]
struct ViewFilter {
    /// Only files at least this big (MB, 0 = off).
    larger_mb: f64,
    /// Only files smaller than this (MB, 0 = off).
    smaller_mb: f64,
    /// Bitmask of `Category::bit()`; 0 = every kind.
    kinds: u16,
}

impl ViewFilter {
    fn active(&self) -> bool {
        self.larger_mb > 0.0 || self.smaller_mb > 0.0 || self.kinds != 0
    }

    fn matches(&self, name: &str, size: u64) -> bool {
        const MB: f64 = 1024.0 * 1024.0;
        let mb = size as f64 / MB;
        (self.larger_mb <= 0.0 || mb >= self.larger_mb)
            && (self.smaller_mb <= 0.0 || mb < self.smaller_mb)
            && (self.kinds == 0 || self.kinds & categories::of(name).bit() != 0)
    }
}

/// One visible line of the finished directory tree.
struct Row {
    path: Box<[u32]>,
    depth: u16,
}

/// One visible line of the tree while a scan is still running.
enum LiveRow {
    Dir { dir: Arc<LiveDir>, depth: u16, parent_size: u64 },
    File { name: Box<str>, size: u64, modified: i64, depth: u16, parent_size: u64 },
    More { count: usize, depth: u16 },
}

/// What a tree line shows, independent of whether it comes from a live or finished scan.
struct LineView<'a> {
    depth: u16,
    is_dir: bool,
    open: bool,
    name: &'a str,
    size: u64,
    parent_size: u64,
    files: u64,
    modified: i64,
    busy: bool,
    color: Color32,
}

const ROW_H: f32 = 20.0;
const INDENT: f32 = 16.0;

pub struct App {
    settings: Settings,
    dialog: settings::Dialog,
    icons: Icons,
    /// `--all-fs` on the command line overrides the "stay on drive" setting for this run.
    force_all_fs: bool,
    applied_theme: Option<settings::Theme>,

    path_input: String,
    scanned_path: PathBuf,
    /// Filters in effect for the tree on screen, to tell when a rescan is needed.
    scanned_with: Option<Settings>,
    job: Option<Job>,
    live_open: HashSet<usize>,
    /// Last order shown for each live folder's subfolders, reused while the mouse is over the
    /// list so rows don't jump around as sizes change.
    live_order: HashMap<usize, Vec<Arc<LiveDir>>>,
    tree: Option<Node>,
    scan_secs: f32,
    exts: Vec<ExtStat>,
    colors: HashMap<String, Color32>,
    selected: Option<Vec<u32>>,
    reveal: bool,
    zoom: Vec<u32>,
    sort: SortBy,
    sort_desc: bool,
    highlight_ext: Option<String>,
    cache: Option<LayoutCache>,
    /// When the treemap's size last changed; it is re-rendered once the size settles.
    resize_since: Option<Instant>,
    expanded: HashSet<Box<[u32]>>,
    rows: Option<Vec<Row>>,
    drives: Vec<Drive>,
    drives_rx: Option<mpsc::Receiver<Vec<Drive>>>,
    show_drives: bool,
    filter: ViewFilter,
    /// (filter, matching files, matching bytes) for the filter summary.
    filter_stats: Option<(ViewFilter, u64, u64)>,
    /// Path and size of the treemap tile under the mouse, for the status bar.
    hover_status: Option<String>,
    wheel: f32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>, force_all_fs: bool) -> Self {
        let settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, "settings")).unwrap_or_default();
        let mut app = Self {
            settings,
            dialog: Default::default(),
            icons: Icons::default(),
            force_all_fs,
            applied_theme: None,
            path_input: path
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            scanned_path: PathBuf::new(),
            scanned_with: None,
            job: None,
            live_open: HashSet::new(),
            live_order: HashMap::new(),
            tree: None,
            scan_secs: 0.0,
            exts: Vec::new(),
            colors: HashMap::new(),
            selected: None,
            reveal: false,
            zoom: Vec::new(),
            sort: SortBy::Size,
            sort_desc: true,
            highlight_ext: None,
            cache: None,
            resize_since: None,
            expanded: HashSet::new(),
            rows: None,
            drives: Vec::new(),
            drives_rx: None,
            show_drives: false,
            filter: ViewFilter::default(),
            filter_stats: None,
            hover_status: None,
            wheel: 0.0,
        };
        app.refresh_drives();
        if path.is_some() {
            app.start_scan();
        }
        app
    }

    /// Settings that drive what's on screen: the draft while the Settings window is open,
    /// so changes preview live.
    fn view(&self) -> &Settings {
        if self.dialog.open { &self.dialog.draft } else { &self.settings }
    }

    fn scan_options(&self) -> Options {
        let s = &self.settings;
        let lines = |t: &str| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect::<Vec<_>>();
        let now = chrono::Utc::now().timestamp();
        Options {
            one_file_system: s.one_file_system && !self.force_all_fs,
            skip_hidden: s.skip_hidden,
            exclude_folders: lines(&s.exclude_folders),
            exclude_files: lines(&s.exclude_files),
            include_files: lines(&s.include_files),
            min_file_size: s.min_file_size,
            min_modified: if s.max_age_days > 0 { now - s.max_age_days as i64 * 86_400 } else { 0 },
            threads: s.threads,
        }
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
        let live = scan::new_live(&path);
        let (tx, rx) = mpsc::channel();
        let (p, l, opts, root) = (progress.clone(), live.clone(), self.scan_options(), path.clone());
        std::thread::spawn(move || {
            let tree = scan::scan(&root, &opts, &p, &l);
            // Summarise here too, so the window doesn't stall when a big scan lands.
            let exts = scan::ext_stats(&tree);
            let _ = tx.send((tree, exts));
        });
        self.live_open.clear();
        self.live_order.clear();
        self.live_open.insert(Arc::as_ptr(&live) as usize);
        self.job = Some(Job { root: path, progress, live, rx, started: Instant::now(), expected: None });
    }

    fn finish_scan(&mut self, root: PathBuf, tree: Node, exts: Vec<ExtStat>, secs: f32) {
        self.scanned_path = root;
        self.scanned_with = Some(self.settings.clone());
        self.exts = exts;
        self.colors = self.exts.iter().enumerate().map(|(i, e)| (e.ext.clone(), ext_color(&e.ext, i))).collect();
        // Freeing millions of nodes can take a while; do it off the UI thread.
        if let Some(old) = self.tree.replace(tree) {
            std::thread::spawn(move || drop(old));
        }
        self.scan_secs = secs;
        self.selected = None;
        self.zoom.clear();
        self.highlight_ext = None;
        self.cache = None;
        self.expanded.clear();
        self.expanded.insert(Box::new([]));
        self.rows = None;
        self.filter_stats = None;
    }

    fn color_for_ext(&self, ext: &str) -> Color32 {
        match self.colors.get(ext) {
            Some(&c) => c,
            None => self.colors.get(&ext.to_lowercase()).copied().unwrap_or(Color32::GRAY),
        }
    }

    fn color_of(&self, n: &Node) -> Color32 {
        self.color_for_ext(n.ext())
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
        if self.drives_rx.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Looking for drives…");
            });
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
            if ui.button("🖴 Drives…").clicked() {
                self.show_drives = !self.show_drives;
                if self.show_drives {
                    self.refresh_drives();
                }
            }
            let edit = ui.add(egui::TextEdit::singleline(&mut self.path_input).desired_width(380.0).hint_text("Folder to scan"));
            if ui.button("Browse…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().set_directory(self.path_input.trim()).pick_folder() {
                    self.path_input = dir.display().to_string();
                    self.start_scan();
                }
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("⟳ Scan").clicked() || enter {
                self.start_scan();
            }
            ui.separator();
            ui.add_enabled_ui(self.tree.is_some() && self.job.is_none(), |ui| {
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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙ Settings").clicked() {
                    self.dialog.open(&self.settings);
                }
            });
        });
    }

    // ---------- tree table (shared by live and finished scans) ----------

    /// Column layout: name takes what's left after the enabled fixed-width columns.
    fn columns(&self, width: f32) -> Vec<(SortBy, &'static str, f32)> {
        let v = self.view();
        let mut cols = vec![(SortBy::Size, "Size", 80.0)];
        if v.col_percent {
            cols.push((SortBy::Size, "% of parent", 120.0));
        }
        if v.col_files {
            cols.push((SortBy::Files, "Files", 80.0));
        }
        if v.col_modified {
            cols.push((SortBy::Modified, "Last change", 130.0));
        }
        let fixed: f32 = cols.iter().map(|c| c.2).sum();
        cols.insert(0, (SortBy::Name, "Name", (width - fixed).max(160.0)));
        cols
    }

    /// Column headers; clicking one sorts by it (if `sortable`).
    fn header(&mut self, ui: &mut egui::Ui, sortable: bool) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
        let mut x = rect.left();
        let cols = self.columns(width);
        for (i, (key, title, w)) in cols.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, ROW_H));
            let resp = ui.interact(cell, ui.id().with(("hdr", i)), if sortable { Sense::click() } else { Sense::hover() });
            let arrow = if sortable && self.sort == *key && *title != "% of parent" {
                if self.sort_desc { " ⏷" } else { " ⏶" }
            } else {
                ""
            };
            let text = format!("{title}{arrow}");
            let color = if resp.hovered() && sortable { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
            let (pos, align) = if i == 0 {
                (pos2(cell.left() + 6.0, cell.center().y), Align2::LEFT_CENTER)
            } else {
                (pos2(cell.right() - 8.0, cell.center().y), Align2::RIGHT_CENTER)
            };
            ui.painter().text(pos, align, text, FontId::proportional(13.0), color);
            if resp.clicked() {
                if self.sort == *key {
                    self.sort_desc = !self.sort_desc;
                } else {
                    self.sort = *key;
                    self.sort_desc = *key != SortBy::Name;
                }
                self.rows = None;
            }
            x += w;
        }
    }

    /// Paint one tree line. Returns the rect of the expand arrow, so clicks there toggle.
    fn paint_line(&mut self, ui: &egui::Ui, rect: Rect, line: &LineView, selected: bool, hovered: bool, stripe: bool) -> Rect {
        let vis = ui.visuals().clone();
        let p = ui.painter().clone();
        if selected {
            p.rect_filled(rect, 2.0, vis.selection.bg_fill);
        } else if hovered {
            p.rect_filled(rect, 2.0, vis.widgets.hovered.weak_bg_fill);
        } else if stripe && self.view().stripes {
            p.rect_filled(rect, 0.0, vis.faint_bg_color);
        }
        let text_color = if selected { vis.selection.stroke.color } else { vis.text_color() };
        let weak = if selected { text_color } else { vis.weak_text_color() };
        let font = FontId::proportional(13.0);
        let cols = self.columns(rect.width());
        let cy = rect.center().y;

        // Name column: indent, arrow, icon, name.
        let name_w = cols[0].2;
        let mut x = rect.left() + 4.0 + line.depth as f32 * INDENT;
        let arrow = Rect::from_center_size(pos2(x + 6.0, cy), vec2(14.0, ROW_H));
        if line.is_dir {
            let c = arrow.center();
            let pts = if line.open {
                vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
            } else {
                vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
            };
            p.add(egui::Shape::convex_polygon(pts, weak, Stroke::NONE));
        }
        x += 14.0;
        let icon = Rect::from_min_size(pos2(x, cy - 8.0), vec2(16.0, 16.0));
        self.icons.paint(ui, icon, line.is_dir, scan::ext_of(line.name), line.color);
        x += 21.0;
        let name_clip = Rect::from_min_max(pos2(x, rect.top()), pos2(rect.left() + name_w - 6.0, rect.bottom()));
        let np = p.with_clip_rect(name_clip.intersect(p.clip_rect()));
        let galley = np.layout_no_wrap(line.name.to_string(), font.clone(), text_color);
        let text_w = galley.size().x;
        np.galley(pos2(x, cy - galley.size().y / 2.0), galley, text_color);
        if line.busy {
            let sr = Rect::from_center_size(pos2((x + text_w + 12.0).min(name_clip.right() - 8.0), cy), vec2(12.0, 12.0));
            egui::Spinner::new().size(12.0).paint_at(ui, sr);
        }

        // Fixed columns, right-aligned.
        let mut cx = rect.left() + name_w;
        for (_, title, w) in &cols[1..] {
            let cell = Rect::from_min_size(pos2(cx, rect.top()), vec2(*w, ROW_H));
            let right = pos2(cell.right() - 8.0, cy);
            match *title {
                "Size" => {
                    p.text(right, Align2::RIGHT_CENTER, scan::human(line.size), font.clone(), text_color);
                }
                "% of parent" => {
                    let frac = if line.parent_size > 0 { line.size as f32 / line.parent_size as f32 } else { 0.0 };
                    let bar = Rect::from_min_size(pos2(cell.left() + 6.0, cy - 5.0), vec2(56.0, 10.0));
                    p.rect_filled(bar, 2.0, vis.extreme_bg_color);
                    let mut fill = bar;
                    fill.set_width(bar.width() * frac.clamp(0.0, 1.0));
                    let c = if line.is_dir { Color32::from_rgb(90, 140, 220) } else { line.color };
                    p.rect_filled(fill, 2.0, c);
                    p.text(right, Align2::RIGHT_CENTER, format!("{:.1}%", frac * 100.0), font.clone(), weak);
                }
                "Files" => {
                    if line.is_dir {
                        p.text(right, Align2::RIGHT_CENTER, group(line.files), font.clone(), weak);
                    }
                }
                "Last change" => {
                    if line.modified > 0 {
                        p.text(right, Align2::RIGHT_CENTER, fmt_time(line.modified), font.clone(), weak);
                    }
                }
                _ => {}
            }
            cx += w;
        }
        arrow
    }

    // ---------- finished tree ----------

    fn tree_panel(&mut self, ui: &mut egui::Ui) {
        self.header(ui, true);
        let Some(tree) = self.tree.take() else { return };
        if self.rows.is_none() {
            self.rows = Some(self.flatten(&tree));
        }
        let rows = self.rows.take().unwrap();

        // Only the rows inside the viewport are laid out, so huge folders stay fast.
        let mut area = egui::ScrollArea::vertical().auto_shrink(false);
        if self.reveal {
            if let Some(i) = self.selected.as_ref().and_then(|s| rows.iter().position(|r| *r.path == **s)) {
                area = area.vertical_scroll_offset((i as f32 * ROW_H - ui.available_height() / 2.0).max(0.0));
            }
            self.reveal = false;
        }
        let mut clicked = None;
        let mut toggled = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for (k, row) in rows[range.clone()].iter().enumerate() {
                let n = tree.at(&row.path).unwrap();
                let parent_size = if row.path.is_empty() { n.size } else { tree.at(&row.path[..row.path.len() - 1]).unwrap().size };
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let line = LineView {
                    depth: row.depth,
                    is_dir: n.is_dir,
                    open: self.expanded.contains(&row.path),
                    name: &n.name,
                    size: n.size,
                    parent_size,
                    files: n.files,
                    modified: n.modified,
                    busy: false,
                    color: self.color_of(n),
                };
                let selected = self.selected.as_deref() == Some(&row.path[..]);
                let arrow = self.paint_line(ui, rect, &line, selected, resp.hovered(), (range.start + k) % 2 == 1);
                let on_arrow = resp.interact_pointer_pos().is_some_and(|p| arrow.expand2(vec2(4.0, 0.0)).contains(p));
                if n.is_dir && ((resp.clicked() && on_arrow) || resp.double_clicked()) {
                    toggled = Some(row.path.clone());
                } else if resp.clicked() {
                    clicked = Some(row.path.to_vec());
                }
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
        let c = &n.children;
        match self.sort {
            SortBy::Size => {}
            SortBy::Name => order.sort_by_cached_key(|&i| c[i].name.to_lowercase()),
            SortBy::Files => order.sort_by(|&a, &b| c[b].files.cmp(&c[a].files)),
            SortBy::Modified => order.sort_by(|&a, &b| c[b].modified.cmp(&c[a].modified)),
        }
        // Name sorts A→Z; every other key above sorts high-to-low.
        let natural_desc = self.sort != SortBy::Name;
        if self.sort_desc != natural_desc {
            order.reverse();
        }
        for i in order {
            path.push(i as u32);
            self.flatten_into(&c[i], path, rows);
            path.pop();
        }
    }

    // ---------- live tree (during a scan) ----------

    fn live_rows(&mut self, root: &Arc<LiveDir>, hold: bool) -> Vec<LiveRow> {
        let mut rows = Vec::new();
        let size = root.size.load(Ordering::Relaxed);
        self.live_into(root, 0, size, hold, &mut rows);
        rows
    }

    fn live_into(&mut self, d: &Arc<LiveDir>, depth: u16, parent_size: u64, hold: bool, rows: &mut Vec<LiveRow>) {
        rows.push(LiveRow::Dir { dir: d.clone(), depth, parent_size });
        if !self.live_open.contains(&(Arc::as_ptr(d) as usize)) {
            return;
        }
        // Show the largest files of each open folder; listing 100k files every frame isn't useful.
        const MAX_FILES: usize = 200;
        let size = d.size.load(Ordering::Relaxed);
        let (mut dirs, mut files, total_files) = {
            let k = d.kids.lock().unwrap();
            let mut idx: Vec<usize> = (0..k.files.len()).collect();
            if idx.len() > MAX_FILES {
                idx.select_nth_unstable_by(MAX_FILES, |&a, &b| k.files[b].size.cmp(&k.files[a].size));
                idx.truncate(MAX_FILES);
            }
            let files: Vec<(Box<str>, u64, i64)> =
                idx.iter().map(|&i| (k.files[i].name.clone(), k.files[i].size, k.files[i].modified)).collect();
            (k.dirs.clone(), files, k.files.len())
        };
        let key = Arc::as_ptr(d) as usize;
        match self.live_order.get(&key).filter(|_| hold) {
            // Keep the previous order; anything new goes at the end.
            Some(prev) => {
                let known: HashSet<usize> = prev.iter().map(|c| Arc::as_ptr(c) as usize).collect();
                let mut ordered = prev.clone();
                ordered.extend(dirs.into_iter().filter(|c| !known.contains(&(Arc::as_ptr(c) as usize))));
                dirs = ordered;
            }
            None => dirs.sort_by_cached_key(|c| std::cmp::Reverse(c.size.load(Ordering::Relaxed))),
        }
        self.live_order.insert(key, dirs.clone());
        files.sort_by(|a, b| b.1.cmp(&a.1));
        for c in &dirs {
            self.live_into(c, depth + 1, size, hold, rows);
        }
        for (name, fsize, modified) in files {
            rows.push(LiveRow::File { name, size: fsize, modified, depth: depth + 1, parent_size: size });
        }
        if total_files > MAX_FILES {
            rows.push(LiveRow::More { count: total_files - MAX_FILES, depth: depth + 1 });
        }
    }

    fn live_panel(&mut self, ui: &mut egui::Ui, live: Arc<LiveDir>) {
        self.header(ui, false);
        let hold = ui.rect_contains_pointer(ui.available_rect_before_wrap());
        let rows = self.live_rows(&live, hold);
        let mut toggled = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for (k, row) in rows[range.clone()].iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let stripe = (range.start + k) % 2 == 1;
                match row {
                    LiveRow::Dir { dir, depth, parent_size } => {
                        let key = Arc::as_ptr(dir) as usize;
                        let line = LineView {
                            depth: *depth,
                            is_dir: true,
                            open: self.live_open.contains(&key),
                            name: &dir.name,
                            size: dir.size.load(Ordering::Relaxed),
                            parent_size: *parent_size,
                            files: dir.files.load(Ordering::Relaxed),
                            modified: dir.modified.load(Ordering::Relaxed),
                            busy: !dir.done.load(Ordering::Acquire),
                            color: Color32::GRAY,
                        };
                        self.paint_line(ui, rect, &line, false, resp.hovered(), stripe);
                        if resp.clicked() {
                            toggled = Some(key);
                        }
                    }
                    LiveRow::File { name, size, modified, depth, parent_size } => {
                        let line = LineView {
                            depth: *depth,
                            is_dir: false,
                            open: false,
                            name,
                            size: *size,
                            parent_size: *parent_size,
                            files: 1,
                            modified: *modified,
                            busy: false,
                            color: ext_color(&scan::ext_of(name).to_lowercase(), usize::MAX),
                        };
                        self.paint_line(ui, rect, &line, false, resp.hovered(), stripe);
                    }
                    LiveRow::More { count, depth } => {
                        let x = rect.left() + 4.0 + *depth as f32 * INDENT + 35.0;
                        ui.painter().text(
                            pos2(x, rect.center().y),
                            Align2::LEFT_CENTER,
                            format!("… and {} smaller files", group(*count as u64)),
                            FontId::proportional(13.0),
                            ui.visuals().weak_text_color(),
                        );
                    }
                }
            }
        });
        if let Some(k) = toggled {
            if !self.live_open.remove(&k) {
                self.live_open.insert(k);
            }
        }
    }

    // ---------- extension list ----------

    fn ext_panel(&mut self, ui: &mut egui::Ui) {
        let total: u64 = self.tree.as_ref().map(|t| t.size).unwrap_or(0).max(1);
        ui.horizontal(|ui| {
            ui.heading("File types");
            if self.highlight_ext.is_some() && ui.small_button("Clear highlight").clicked() {
                self.highlight_ext = None;
            }
        });
        let mut toggle = None;
        let font = FontId::proportional(13.0);
        let (hdr, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
        let w = hdr.width();
        let strong = ui.visuals().strong_text_color();
        for (x, align, t) in [
            (hdr.left() + 30.0, Align2::LEFT_CENTER, "Type"),
            (hdr.left() + w - 110.0, Align2::RIGHT_CENTER, "Size"),
            (hdr.left() + w - 60.0, Align2::RIGHT_CENTER, "%"),
            (hdr.right() - 4.0, Align2::RIGHT_CENTER, "Files"),
        ] {
            ui.painter().text(pos2(x, hdr.center().y), align, t, font.clone(), strong);
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let exts = std::mem::take(&mut self.exts);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, exts.len(), |ui, range| {
            for (k, e) in exts[range.clone()].iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let on = self.highlight_ext.as_deref() == Some(e.ext.as_str());
                let vis = ui.visuals().clone();
                if on {
                    ui.painter().rect_filled(rect, 2.0, vis.selection.bg_fill);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 2.0, vis.widgets.hovered.weak_bg_fill);
                } else if (range.start + k) % 2 == 1 && self.view().stripes {
                    ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
                }
                let c = self.colors[&e.ext];
                let sw = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 6.0), vec2(4.0, 12.0));
                ui.painter().rect_filled(sw, 1.0, c);
                let icon = Rect::from_min_size(pos2(rect.left() + 11.0, rect.center().y - 8.0), vec2(16.0, 16.0));
                self.icons.paint(ui, icon, false, &e.ext, c);
                let name = if e.ext.is_empty() { "(none)".to_string() } else { format!(".{}", e.ext) };
                let tc = if on { vis.selection.stroke.color } else { vis.text_color() };
                let clip = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(rect.right() - 175.0, rect.bottom()));
                ui.painter()
                    .with_clip_rect(clip.intersect(ui.clip_rect()))
                    .text(pos2(clip.left(), rect.center().y), Align2::LEFT_CENTER, name, font.clone(), tc);
                let cy = rect.center().y;
                ui.painter().text(pos2(rect.right() - 110.0, cy), Align2::RIGHT_CENTER, scan::human(e.bytes), font.clone(), tc);
                let pct = format!("{:.1}", e.bytes as f64 * 100.0 / total as f64);
                ui.painter().text(pos2(rect.right() - 60.0, cy), Align2::RIGHT_CENTER, pct, font.clone(), vis.weak_text_color());
                ui.painter().text(pos2(rect.right() - 4.0, cy), Align2::RIGHT_CENTER, group(e.files), font.clone(), vis.weak_text_color());
                if resp.on_hover_text("Click to highlight in the treemap").clicked() {
                    toggle = Some(e.ext.clone());
                }
            }
        });
        self.exts = exts;
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
        let look = self.view().treemap_key();

        let (content_changed, size_changed) = match &self.cache {
            Some(c) => {
                let changed = c.zoom != self.zoom || c.highlight != self.highlight_ext || c.look != look || c.filter != self.filter;
                (changed, c.px != px)
            }
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
            self.render_treemap(ui.ctx(), px, look);
        }
        let Some(cache) = self.cache.as_ref() else { return };
        let tree = self.tree.as_ref().unwrap();
        painter.image(cache.texture.id(), bounds, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

        // Tile rects are in the cached image's pixels; map them onto the current bounds.
        let (sx, sy) = (bounds.width() / cache.px[0] as f32, bounds.height() / cache.px[1] as f32);
        let to_screen = |r: Rect| {
            Rect::from_min_max(
                pos2(bounds.min.x + r.min.x * sx, bounds.min.y + r.min.y * sy),
                pos2(bounds.min.x + r.max.x * sx, bounds.min.y + r.max.y * sy),
            )
        };
        let zl = self.zoom.len();
        let zroot = tree.at(&self.zoom).unwrap();

        // File names on tiles with room for them.
        if self.view().show_labels {
            let font = FontId::proportional(11.0);
            for (t, &c) in cache.tiles.iter().zip(&cache.colors) {
                if c == Color32::TRANSPARENT {
                    continue;
                }
                let r = to_screen(t.rect);
                if r.width() < 44.0 || r.height() < 15.0 {
                    continue;
                }
                let n = zroot.at(&t.path).unwrap();
                let luma = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
                let tc = if luma > 140.0 { Color32::from_black_alpha(220) } else { Color32::from_white_alpha(235) };
                let p = painter.with_clip_rect(r.shrink(1.0).intersect(painter.clip_rect()));
                p.text(r.min + vec2(3.0, 2.0), Align2::LEFT_TOP, &*n.name, font.clone(), tc);
                if r.height() >= 30.0 {
                    p.text(r.min + vec2(3.0, 15.0), Align2::LEFT_TOP, scan::human(n.size), font.clone(), tc.gamma_multiply(0.8));
                }
            }
        }

        // Outline the selection (or the nearest visible ancestor of it).
        if let Some(sel) = self.selected.as_ref().filter(|s| s.starts_with(&self.zoom)) {
            let rel = &sel[zl..];
            if let Some(t) = cache.tiles.iter().filter(|t| rel.starts_with(&t.path)).max_by_key(|t| t.path.len()) {
                let stroke = Stroke::new(2.0, self.view().selection_color);
                painter.rect_stroke(to_screen(t.rect), 0.0, stroke, egui::StrokeKind::Inside);
            }
        }

        let hover = resp.hover_pos().and_then(|p| {
            let local = pos2((p.x - bounds.min.x) / sx, (p.y - bounds.min.y) / sy);
            cache.tiles.iter().rev().find(|t| t.rect.contains(local))
        });
        let mut action = None;
        let prev_status = self.hover_status.take();
        // Mouse wheel: up zooms into the folder under the pointer, down zooms back out.
        if resp.hovered() {
            self.wheel += ui.input(|i| {
                i.raw
                    .events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { delta, unit, .. } => match unit {
                            egui::MouseWheelUnit::Point => delta.y,
                            egui::MouseWheelUnit::Line => delta.y * 50.0,
                            egui::MouseWheelUnit::Page => delta.y * 400.0,
                        },
                        _ => 0.0,
                    })
                    .sum::<f32>()
            });
            if self.wheel >= 40.0 {
                self.wheel = 0.0;
                if let Some(&first) = hover.and_then(|t| t.path.first()) {
                    let mut z = self.zoom.clone();
                    z.push(first);
                    if tree.at(&z).is_some_and(|n| n.is_dir) {
                        action = Some((true, z));
                    }
                }
            } else if self.wheel <= -40.0 {
                self.wheel = 0.0;
                if !self.zoom.is_empty() {
                    let mut z = self.zoom.clone();
                    z.pop();
                    action = Some((true, z));
                }
            }
        } else {
            self.wheel = 0.0;
        }
        if let Some(t) = hover {
            let parent = cache.tiles.iter().find(|p| p.is_dir && p.path.len() + 1 == t.path.len() && t.path.starts_with(&p.path));
            if let Some(parent) = parent {
                let stroke = Stroke::new(1.0, Color32::from_white_alpha(140));
                painter.rect_stroke(to_screen(parent.rect), 0.0, stroke, egui::StrokeKind::Inside);
            }
            let mut abs = self.zoom.clone();
            abs.extend_from_slice(&t.path);
            let n = tree.at(&abs).unwrap();
            let full = self.full_path(&abs);
            let what = if n.is_dir { format!("{} files", group(n.files)) } else { fmt_time(n.modified) };
            self.hover_status = Some(format!("{}  ·  {}  ·  {}", full.display(), scan::human(n.size), what));
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
        // The status bar was drawn before this panel, so repaint when what it should say has changed.
        if action.is_some() || self.hover_status != prev_status {
            ui.ctx().request_repaint();
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

    fn render_treemap(&mut self, ctx: &egui::Context, px: [usize; 2], look: [u32; 9]) {
        let v = self.view().clone();
        let tree = self.tree.as_ref().unwrap();
        let zroot = tree.at(&self.zoom).unwrap();
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(px[0] as f32, px[1] as f32));
        let tiles = treemap::layout(zroot, bounds, v.height, v.scale);
        let filtering = self.filter.active();
        // With a filter on, everything that isn't a match (including folders too small to draw) is blanked.
        let background = if filtering { Color32::from_gray(24) } else { Color32::from_gray(45) };
        let mut pixels = vec![background; px[0] * px[1]];
        let mut colors = vec![Color32::TRANSPARENT; tiles.len()];
        let hl = self.highlight_ext.as_deref();
        let shade = Shading::new(&v);
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
            if filtering && !self.filter.matches(&n.name, n.size) {
                fill_rect(&mut pixels, px[0], t.rect, Color32::from_gray(24));
                continue;
            }
            let mut c = self.color_of(n);
            colors[i] = c;
            if let Some(h) = hl {
                if !n.ext().eq_ignore_ascii_case(h) {
                    c = c.gamma_multiply(0.15);
                }
            }
            shade.paint(&mut pixels, px[0], t, c);
        }
        let img = ColorImage::new(px, pixels);
        match &mut self.cache {
            Some(cache) => {
                cache.texture.set(img, TextureOptions::LINEAR);
                cache.px = px;
                cache.zoom = self.zoom.clone();
                cache.highlight = self.highlight_ext.clone();
                cache.look = look;
                cache.filter = self.filter.clone();
                cache.tiles = tiles;
                cache.colors = colors;
            }
            None => {
                let texture = ctx.load_texture("treemap", img, TextureOptions::LINEAR);
                let (zoom, highlight) = (self.zoom.clone(), self.highlight_ext.clone());
                let filter = self.filter.clone();
                self.cache = Some(LayoutCache { px, zoom, highlight, look, filter, tiles, colors, texture });
            }
        }
    }

    /// Breadcrumbs for the zoomed folder, plus the "show only" filters for the treemap.
    fn filter_bar(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.tree.as_ref() else { return };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut go = None;
            if ui.small_button(format!("🏠 {}", tree.name)).clicked() {
                go = Some(0);
            }
            let mut n = tree;
            for (k, &i) in self.zoom.iter().enumerate() {
                n = &n.children[i as usize];
                ui.weak("›");
                if ui.small_button(&*n.name).clicked() {
                    go = Some(k + 1);
                }
            }
            if let Some(k) = go {
                self.zoom.truncate(k);
            }
            ui.add_space(12.0);
            ui.weak("Wheel up/down over the map to zoom in/out.");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Show:");
            if ui.selectable_label(self.filter.kinds == 0, "All").clicked() {
                self.filter.kinds = 0;
            }
            for c in Category::ALL {
                let on = self.filter.kinds & c.bit() != 0;
                if ui.selectable_label(on, c.label()).on_hover_text(c.tooltip()).clicked() {
                    self.filter.kinds ^= c.bit();
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Larger than");
            ui.add(egui::DragValue::new(&mut self.filter.larger_mb).range(0.0..=1e9).speed(1.0).suffix(" MB"));
            ui.label("Smaller than");
            ui.add(egui::DragValue::new(&mut self.filter.smaller_mb).range(0.0..=1e9).speed(1.0).suffix(" MB"));
            ui.weak("(0 = off)");
            if self.filter.active() {
                if ui.button("Clear filters").clicked() {
                    self.filter = ViewFilter::default();
                }
                let stale = self.filter_stats.as_ref().is_none_or(|(f, _, _)| *f != self.filter);
                if stale {
                    let (files, bytes) = count_matches(tree, &self.filter);
                    self.filter_stats = Some((self.filter.clone(), files, bytes));
                }
                if let Some((_, files, bytes)) = &self.filter_stats {
                    let pct = *bytes as f64 * 100.0 / tree.size.max(1) as f64;
                    ui.separator();
                    ui.strong(format!("{} files match · {} ({pct:.1}% of the scan)", group(*files), scan::human(*bytes)));
                }
            }
            ui.separator();
            let mut labels = self.settings.show_labels;
            if ui.checkbox(&mut labels, "File names on tiles").changed() {
                self.settings.show_labels = labels;
                self.dialog.draft.show_labels = labels;
            }
        });
    }

    // ---------- scan screen ----------

    fn scan_screen(&mut self, ui: &mut egui::Ui) {
        let job = self.job.as_ref().unwrap();
        let live = job.live.clone();
        let finishing = job.progress.finishing.load(Ordering::Relaxed);
        let show_tree = self.view().live_tree;
        scan_progress(ui, job, show_tree);
        if show_tree && !finishing {
            ui.add_space(6.0);
            self.live_panel(ui, live);
        }
    }
}

/// Per-pixel cushion shading (van Wijk & van de Wetering), with WinDirStat's knobs.
struct Shading {
    ambient: f32,
    diffuse: f32,
    gain: f32,
    light: [f32; 3],
    grid: Option<Color32>,
}

impl Shading {
    fn new(s: &Settings) -> Self {
        let (lx, ly, lz) = (s.light_x, s.light_y, 10.0f32);
        let len = (lx * lx + ly * ly + lz * lz).sqrt();
        let ambient = 1.0 - s.cushion.clamp(0.0, 1.0);
        Self {
            ambient,
            diffuse: 1.0 - ambient,
            // Normalise so the average tile keeps the same brightness however strong the
            // shading is (0.826 is the average light at the default cushion setting).
            gain: s.brightness / 0.6 * 0.826 / (ambient + (1.0 - ambient) * 0.8),
            light: [lx / len, ly / len, lz / len],
            grid: s.show_grid.then_some(s.grid_color),
        }
    }

    fn paint(&self, pixels: &mut [Color32], stride: usize, t: &Tile, c: Color32) {
        let height = pixels.len() / stride;
        let r = t.rect;
        let (x0, x1) = (r.min.x.round().max(0.0) as usize, (r.max.x.round() as usize).min(stride));
        let (y0, y1) = (r.min.y.round().max(0.0) as usize, (r.max.y.round() as usize).min(height));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let [s0, s1, s2, s3] = t.surface;
        let (cr, cg, cb) = (c.r() as f32 * self.gain, c.g() as f32 * self.gain, c.b() as f32 * self.gain);
        let [lx, ly, lz] = self.light;
        for y in y0..y1 {
            let ny = -(2.0 * s1 * (y as f32 + 0.5) + s3);
            let row = &mut pixels[y * stride + x0..y * stride + x1];
            for (k, p) in row.iter_mut().enumerate() {
                let nx = -(2.0 * s0 * ((x0 + k) as f32 + 0.5) + s2);
                let cosa = ((nx * lx + ny * ly + lz) / (nx * nx + ny * ny + 1.0).sqrt()).clamp(0.0, 1.0);
                let f = self.ambient + self.diffuse * cosa;
                *p = Color32::from_rgb((cr * f).min(255.0) as u8, (cg * f).min(255.0) as u8, (cb * f).min(255.0) as u8);
            }
        }
        if let Some(g) = self.grid {
            pixels[y0 * stride + x0..y0 * stride + x1].fill(g);
            for y in y0..y1 {
                pixels[y * stride + x0] = g;
            }
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "settings", &self.settings);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let view_theme = self.view().theme;
        if self.applied_theme != Some(view_theme) {
            self.view().apply_theme(&ctx);
            self.applied_theme = Some(view_theme);
        }
        scan::set_decimal_units(self.view().units == settings::SizeUnits::Decimal);

        if let settings::Outcome::Apply = self.dialog.show(&ctx) {
            self.settings = self.dialog.draft.clone();
            self.rows = None;
        }

        if let Some(rx) = &self.drives_rx {
            match rx.try_recv() {
                Ok(d) => {
                    self.drives = d;
                    self.drives_rx = None;
                }
                Err(_) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        // Scanning a whole drive: its used space gives an estimate of how far along we are.
        // Checked every frame because the drive list may arrive after the scan starts.
        if let Some(job) = self.job.as_mut().filter(|j| j.expected.is_none()) {
            job.expected = self.drives.iter().find(|d| same_path(&d.root, &job.root)).map(|d| d.used()).filter(|&u| u > 0);
        }

        // Collect a finished scan.
        if let Some(job) = &self.job {
            if let Ok((tree, exts)) = job.rx.try_recv() {
                let job = self.job.take().unwrap();
                if !job.progress.cancel.load(Ordering::Relaxed) {
                    self.finish_scan(job.root, tree, exts, job.started.elapsed().as_secs_f32());
                } else {
                    std::thread::spawn(move || drop(tree));
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(2.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });

        if self.show_drives && self.tree.is_some() {
            let mut open = true;
            let mut picked = None;
            egui::Window::new("Select drive").open(&mut open).collapsible(false).show(&ctx, |ui| {
                picked = self.drive_table(ui);
            });
            self.show_drives = open;
            if let Some(root) = picked {
                self.scan_root(root);
            }
        }

        let mut rescan = false;
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(h) = self.hover_status.as_ref().filter(|_| self.job.is_none()) {
                    ui.strong(h);
                } else if let Some(job) = &self.job {
                    ui.spinner();
                    ui.label(format!("Scanning {}…", job.root.display()));
                } else if let Some(t) = &self.tree {
                    ui.label(format!(
                        "{}: {} in {} files, scanned in {:.1}s. Click a tile to select, double-click to zoom in, right-click to zoom out.",
                        self.scanned_path.display(),
                        scan::human(t.size),
                        group(t.files),
                        self.scan_secs
                    ));
                    let stale = self.scanned_with.as_ref().is_some_and(|s| s.scan_key() != self.settings.scan_key());
                    if stale {
                        ui.separator();
                        ui.colored_label(ui.visuals().warn_fg_color, "Scan settings changed.");
                        rescan = ui.small_button("Rescan").clicked();
                    }
                } else {
                    ui.label("Pick a folder and press Scan.");
                }
                if let Some(sel) = self.selected.as_ref().filter(|_| self.job.is_none() && self.hover_status.is_none()) {
                    ui.separator();
                    ui.label(self.full_path(sel).display().to_string());
                }
            });
        });
        if rescan {
            self.path_input = self.scanned_path.display().to_string();
            self.start_scan();
        }

        if self.job.is_some() {
            egui::CentralPanel::default().show(ui, |ui| self.scan_screen(ui));
            return;
        }

        if self.tree.is_none() {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.heading("Pick a drive to scan, or type a folder above");
                ui.add_space(8.0);
                if let Some(root) = self.drive_table(ui) {
                    self.scan_root(root);
                }
            });
            return;
        }

        egui::Panel::right("exts").default_size(300.0).resizable(true).show(ui, |ui| self.ext_panel(ui));
        egui::Panel::top("tree").default_size(320.0).resizable(true).show(ui, |ui| self.tree_panel(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| self.filter_bar(ui));
            self.treemap_panel(ui);
        });
    }
}

/// Live progress for a running scan: counts, an estimated percentage for whole drives, and
/// the folder being read right now, so a long scan never looks frozen. Compact when the live
/// tree is shown below it.
fn scan_progress(ui: &mut egui::Ui, job: &Job, compact: bool) {
    let p = &job.progress;
    let files = p.files.load(Ordering::Relaxed);
    let dirs = p.dirs.load(Ordering::Relaxed);
    let bytes = p.bytes.load(Ordering::Relaxed);
    let errors = p.errors.load(Ordering::Relaxed);
    let secs = job.started.elapsed().as_secs_f32();
    let cancelling = p.cancel.load(Ordering::Relaxed);
    let finishing = p.finishing.load(Ordering::Relaxed);

    let bar = if finishing {
        egui::ProgressBar::new(1.0).text("Sorting and building the map…").animate(true)
    } else {
        match job.expected {
            // Logical file sizes don't exactly match used disk space, so cap below 100%.
            Some(total) => {
                let frac = (bytes as f32 / total as f32).min(0.99);
                let text = format!("about {:.0}%  ({} of {})", frac * 100.0, scan::human(bytes), scan::human(total));
                egui::ProgressBar::new(frac).text(text)
            }
            None => egui::ProgressBar::new(0.0).animate(true).text(scan::human(bytes)),
        }
    };
    let rate = if secs > 0.5 { files as f32 / secs } else { 0.0 };
    let unreadable = if errors > 0 { format!(" · {} unreadable", group(errors)) } else { String::new() };
    let stats = format!(
        "{} files · {} folders · {} · {:.0} files/s · {}:{:02}{unreadable}",
        group(files),
        group(dirs),
        scan::human(bytes),
        rate,
        secs as u64 / 60,
        secs as u64 % 60,
    );
    let current = p.current.lock().map(|c| c.display().to_string()).unwrap_or_default();

    if compact {
        ui.horizontal(|ui| {
            ui.heading(format!("Scanning {}", job.root.display()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if cancelling {
                    ui.label("Cancelling…");
                } else if ui.button("Cancel").clicked() {
                    p.cancel.store(true, Ordering::Relaxed);
                }
            });
        });
        ui.add(bar.desired_height(18.0));
        ui.horizontal(|ui| {
            ui.strong(stats);
            ui.add(egui::Label::new(egui::RichText::new(current).weak().monospace()).truncate());
        });
        return;
    }

    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 100.0).max(16.0));
        ui.set_max_width(640.0);
        ui.heading(format!("Scanning {}", job.root.display()));
        ui.add_space(12.0);
        ui.add(bar.desired_height(22.0));
        ui.add_space(10.0);
        ui.strong(stats);
        ui.add_space(8.0);
        ui.add(egui::Label::new(egui::RichText::new(current).weak().monospace()).truncate());
        ui.add_space(12.0);
        if cancelling {
            ui.label("Cancelling…");
        } else if ui.button("Cancel").clicked() {
            p.cancel.store(true, Ordering::Relaxed);
        }
    });
}

/// Compare paths ignoring a trailing separator and, on Windows, case (`d:\` vs `D:\`).
fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    let norm = |p: &std::path::Path| {
        let s = p.to_string_lossy();
        let s = s.trim_end_matches(['/', '\\']);
        if cfg!(windows) { s.to_lowercase() } else { s.to_string() }
    };
    norm(a) == norm(b)
}

fn fill_rect(pixels: &mut [Color32], stride: usize, r: Rect, c: Color32) {
    let height = pixels.len() / stride;
    let (x0, x1) = (r.min.x.round().max(0.0) as usize, (r.max.x.round() as usize).min(stride));
    let (y0, y1) = (r.min.y.round().max(0.0) as usize, (r.max.y.round() as usize).min(height));
    for y in y0..y1.max(y0) {
        if x0 < x1 {
            pixels[y * stride + x0..y * stride + x1].fill(c);
        }
    }
}

/// Files (and their total size) anywhere in the tree that pass the filter.
fn count_matches(n: &Node, f: &ViewFilter) -> (u64, u64) {
    if !n.is_dir {
        return if f.matches(&n.name, n.size) { (1, n.size) } else { (0, 0) };
    }
    n.children.iter().fold((0, 0), |(a, b), c| {
        let (x, y) = count_matches(c, f);
        (a + x, b + y)
    })
}

/// 1234567 -> "1,234,567"
fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn fmt_time(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
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
