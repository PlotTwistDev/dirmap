//! The desktop GUI: directory tree, extension list and treemap.

use crate::categories::{self, Category};
use crate::drives::{self, Drive};
use crate::dupes;
use crate::elevate::{self, AdminMode};
use crate::help::{self, topics};
use crate::camera::{self, Feel};
use crate::icons::Icons;
use crate::mapview::{self, Look, MapView, Paint};
use crate::scan::{self, ExtStat, LiveDir, Node, Options, Progress};
use crate::settings::{self, Settings};
use crate::snapshot::{self, Change, DiffNode, Snapshot};
use crate::sunburst;
use crate::tilegpu::Backend;
use crate::treemap::{self, Cushion};
#[cfg(test)]
use crate::treemap::Tile;
use custom::{CustomUi, FilterJob, SeqState, Shown};
mod menus;

use eframe::egui::{
    self, ecolor::Hsva, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

mod custom;

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

/// Which files the maps show; everything else is blanked out. Doesn't need a rescan.
#[derive(Clone, PartialEq, Default, Hash)]
struct ViewFilter {
    /// Only files at least this many bytes (0 = off).
    larger: u64,
    /// Only files smaller than this many bytes (0 = off).
    smaller: u64,
    /// Bitmask of `Category::bit()`; 0 = every kind.
    kinds: u16,
    /// The Custom pattern, compiled; None when the box is empty.
    custom: Option<Arc<crate::pattern::Filter>>,
}

impl ViewFilter {
    fn active(&self) -> bool {
        self.larger > 0 || self.smaller > 0 || self.kinds != 0 || self.custom.is_some()
    }

    fn matches(&self, name: &str, size: u64) -> bool {
        (self.larger == 0 || size >= self.larger)
            && (self.smaller == 0 || size < self.smaller)
            && (self.kinds == 0 || self.kinds & categories::of(name).bit() != 0)
            && self.custom.as_ref().is_none_or(|f| f.matches(name))
    }
}

/// What the colors on the maps mean.
#[derive(Clone, Copy, PartialEq, Hash)]
enum ColorMode {
    Type,
    Age,
    Change,
}

#[derive(Clone, Copy, PartialEq)]
enum MapKind {
    Treemap,
    Sunburst,
}

/// Tabs of the right-hand panel.
#[derive(Clone, Copy, PartialEq)]
enum SideTab {
    Types,
    Sizes,
    Changes,
    Duplicates,
    Sequences,
}

/// Size ranges for the histogram: (lower bound, upper bound or 0 for none, label).
const SIZE_BUCKETS: [(u64, u64, &str); 9] = [
    (0, 1 << 10, "under 1 KB"),
    (1 << 10, 10 << 10, "1 – 10 KB"),
    (10 << 10, 100 << 10, "10 – 100 KB"),
    (100 << 10, 1 << 20, "100 KB – 1 MB"),
    (1 << 20, 10 << 20, "1 – 10 MB"),
    (10 << 20, 100 << 20, "10 – 100 MB"),
    (100 << 20, 1 << 30, "100 MB – 1 GB"),
    (1 << 30, 10 << 30, "1 – 10 GB"),
    (10 << 30, 0, "over 10 GB"),
];

/// Age bands for "color by age": (age in days, color, label).
const AGE_STOPS: [(f32, [u8; 3], &str); 6] = [
    (0.0, [235, 64, 52], "today"),
    (7.0, [245, 145, 40], "week"),
    (30.0, [228, 205, 60], "month"),
    (365.0, [95, 185, 90], "year"),
    (3.0 * 365.0, [50, 150, 175], "3 years"),
    (10.0 * 365.0, [70, 80, 175], "10+ years"),
];

/// A duplicate search running in the background.
struct DupJob {
    progress: Arc<dupes::Progress>,
    rx: mpsc::Receiver<Vec<dupes::Group>>,
}

/// Cached sunburst layout for one zoom level and filter.
struct SunCache {
    zoom: Vec<u32>,
    segs: Vec<sunburst::Seg>,
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
    /// Doesn't match the filter (Highlight mode): drawn faintly.
    dim: bool,
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
    tree: Option<Arc<Node>>,
    scan_secs: f32,
    exts: Vec<ExtStat>,
    colors: HashMap<String, Color32>,
    selected: Option<Vec<u32>>,
    reveal: bool,
    zoom: Vec<u32>,
    sort: SortBy,
    sort_desc: bool,
    highlight_ext: Option<String>,
    /// The treemap: camera, tiles and navigation.
    map: MapView,
    /// Paint inputs the map was last given (`paint_key`).
    map_paint: Option<u64>,
    expanded: HashSet<Box<[u32]>>,
    rows: Option<Vec<Row>>,
    drives: Vec<Drive>,
    drives_rx: Option<mpsc::Receiver<Vec<Drive>>>,
    show_drives: bool,
    filter: ViewFilter,
    /// (filter, matching files, matching bytes) for the filter summary.
    filter_stats: Option<(ViewFilter, u64, u64)>,
    /// The Custom filter's input, history and presets.
    custom: CustomUi,
    /// In "Show matches only": the filtered tree the maps and folder list show instead of `tree`.
    /// Paths in `zoom`, `selected` and `expanded` are paths in this tree while it's set.
    shown: Option<Shown>,
    filter_job: Option<FilterJob>,
    seq: SeqState,
    /// Path and size of the treemap tile under the mouse, for the status bar.
    hover_status: Option<String>,
    wheel: f32,

    color_mode: ColorMode,
    map_kind: MapKind,
    side_tab: SideTab,
    /// Bumped whenever something that changes tile colors (other than the cache's own keys) changes.
    paint_gen: u32,
    /// Size histogram for (zoom, kinds): files and bytes per bucket.
    hist: Option<(Vec<u32>, u16, [(u64, u64); 9])>,
    sun: Option<SunCache>,
    /// The snapshot being compared against, its comparison with the current scan, and status text.
    snapshot: Option<Snapshot>,
    snap_rx: Option<mpsc::Receiver<Result<Snapshot, String>>>,
    diff: Option<Arc<(DiffNode, Vec<Change>)>>,
    snap_msg: Option<String>,
    dup_min: u64,
    dup_job: Option<DupJob>,
    dup_groups: Vec<dupes::Group>,
    /// Flattened duplicate list: (group, file index or None for the group's header line).
    dup_rows: Vec<(usize, Option<usize>)>,
    dup_set: Arc<HashSet<Vec<u32>>>,
    dupes_only: bool,
    /// A duplicate search has finished for the current scan.
    dup_done: bool,
    /// Show the "run as administrator?" question (first start without admin rights).
    admin_prompt: bool,
    /// Set when a restart as administrator didn't happen, to say so.
    admin_declined: bool,
    /// Right-click menus: running file operation, their windows and the last result.
    ops: menus::Ops,
    /// Text to put on the clipboard this frame.
    ctx_copy: Option<String>,
    /// The Help Centre and About windows.
    help: help::Help,
    /// Where each part of the window was drawn last frame, with its help page, for F1.
    help_regions: Vec<(egui::Rect, &'static str)>,
    /// Folders and files the last scan couldn't read.
    scan_errors: u64,
    /// The renderer in use, for About.
    renderer: &'static str,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>, force_all_fs: bool, compare: Option<PathBuf>) -> Self {
        let mut settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, "settings")).unwrap_or_default();
        settings.admin = elevate::load_mode();
        let admin_prompt = elevate::can_elevate() && !elevate::is_elevated() && settings.admin == AdminMode::Ask;
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
            map: MapView::new(Backend::new(cc)),
            map_paint: None,
            expanded: HashSet::new(),
            rows: None,
            drives: Vec::new(),
            drives_rx: None,
            show_drives: false,
            filter: ViewFilter::default(),
            filter_stats: None,
            custom: CustomUi {
                prefs: cc.storage.and_then(|s| eframe::get_value(s, "custom_filter")).unwrap_or_default(),
                ..Default::default()
            },
            shown: None,
            filter_job: None,
            seq: SeqState::default(),
            hover_status: None,
            wheel: 0.0,
            color_mode: ColorMode::Type,
            map_kind: MapKind::Treemap,
            side_tab: SideTab::Types,
            paint_gen: 0,
            hist: None,
            sun: None,
            snapshot: None,
            snap_rx: None,
            diff: None,
            snap_msg: None,
            dup_min: 1 << 20,
            dup_job: None,
            dup_groups: Vec::new(),
            dup_rows: Vec::new(),
            dup_set: Arc::new(HashSet::new()),
            dupes_only: false,
            dup_done: false,
            admin_prompt,
            admin_declined: false,
            ops: Default::default(),
            ctx_copy: None,
            help: Default::default(),
            help_regions: Vec::new(),
            scan_errors: 0,
            renderer: if cc.wgpu_render_state.is_some() {
                "WebGPU (wgpu)"
            } else if cc.gl.is_some() {
                "OpenGL (glow)"
            } else {
                "unknown"
            },
        };
        app.refresh_drives();
        if let Some(n) = std::env::var("DIRMAP_SYNTHETIC").ok().and_then(|s| s.parse::<usize>().ok()) {
            let tree = treemap::synthetic(n, 5);
            let exts = scan::ext_stats(&tree);
            app.finish_scan(PathBuf::from("synthetic"), tree, exts, 0.0);
            return app;
        }
        // DIRMAP_FILTER=pattern starts with the Custom filter set, e.g. for screenshots.
        if let Ok(f) = std::env::var("DIRMAP_FILTER") {
            app.set_custom(&f);
        }
        if std::env::var("DIRMAP_FILTER_ONLY").is_ok() {
            app.custom.prefs.mode = custom::FilterMode::Only;
        }
        if let Some(f) = compare {
            app.load_snapshot_from(f);
        }
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
        self.options_for(&self.settings)
    }

    fn options_for(&self, s: &Settings) -> Options {
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
        if let Some(old) = self.tree.replace(Arc::new(tree)) {
            std::thread::spawn(move || drop(old));
        }
        self.scan_secs = secs;
        self.selected = None;
        self.zoom.clear();
        self.highlight_ext = None;
        self.map.clear();
        self.expanded.clear();
        self.expanded.insert(Box::new([]));
        self.rows = None;
        self.reset_custom_for_new_tree();
        self.filter_stats = None;
        self.hist = None;
        self.sun = None;
        // Duplicates refer to the old tree; a loaded snapshot is compared with the new one.
        if let Some(j) = self.dup_job.take() {
            j.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.dup_groups.clear();
        self.dup_rows.clear();
        self.dup_set = Arc::new(HashSet::new());
        self.dupes_only = false;
        self.dup_done = false;
        self.compare();
        self.paint_gen += 1;
    }

    /// Start an elevated copy and close this one. If Windows' prompt is declined, stay open and say so.
    fn restart_as_admin(&mut self, ctx: &egui::Context) {
        if elevate::relaunch_elevated() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.admin_declined = true;
        }
    }

    /// Asked once, the first time dirmap starts without administrator rights; the answer is
    /// remembered and can be changed under Settings > Advanced.
    fn admin_question(&mut self, ctx: &egui::Context) {
        if self.admin_declined {
            let mut open = true;
            egui::Window::new("Not running as administrator")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label("Windows didn't start dirmap as administrator, so it's carrying on as it is.");
                    if ui.button("OK").clicked() {
                        self.admin_declined = false;
                    }
                });
            if !open {
                self.admin_declined = false;
            }
            return;
        }
        if !self.admin_prompt {
            return;
        }
        let mut choice = None;
        egui::Window::new("Run as administrator?")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label(
                    "dirmap isn't running as administrator. Windows keeps some folders (other users' files, parts of \
                     Windows itself) closed to ordinary programs, so they would be counted as unreadable and \
                     left out of the totals.",
                );
                ui.add_space(4.0);
                ui.weak("You can change this later under Settings > Advanced.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("🛡 Yes, always").on_hover_text("Restart as administrator now and every time").clicked() {
                        choice = Some((Some(AdminMode::Always), true));
                    }
                    if ui.button("Just this once").on_hover_text("Restart as administrator now; ask again next time").clicked() {
                        choice = Some((None, true));
                    }
                    if ui.button("No, don't ask again").clicked() {
                        choice = Some((Some(AdminMode::Never), false));
                    }
                });
            });
        if let Some((mode, restart)) = choice {
            self.admin_prompt = false;
            if let Some(m) = mode {
                self.settings.admin = m;
                self.dialog.draft.admin = m;
                elevate::save_mode(m);
            }
            if restart {
                self.restart_as_admin(ctx);
            }
        }
    }

    /// Compare the current scan with the loaded snapshot.
    fn compare(&mut self) {
        self.diff = match (&self.tree, &self.snapshot) {
            (Some(t), Some(s)) => Some(Arc::new(snapshot::diff(t, &s.tree))),
            _ => None,
        };
        self.paint_gen += 1;
    }

    /// Color of a file or folder on the maps under the current color mode. `abs` is its path from the scan root.
    fn node_color(&self, n: &Node, abs: &[u32]) -> Color32 {
        match self.color_mode {
            ColorMode::Type => self.color_of(n),
            ColorMode::Age => age_color(n.modified),
            ColorMode::Change => match self.diff.as_deref().and_then(|(d, _)| d.at(abs)) {
                Some(d) => change_color(d.old, n.size),
                None => Color32::from_gray(95),
            },
        }
    }

    /// Status bar text for an entry on a map.
    fn hover_text(&self, abs: &[u32]) -> String {
        let tree = self.shown_tree().unwrap();
        let n = tree.at(abs).unwrap();
        let full = self.full_path(abs);
        let what = match (n.is_dir, n.files) {
            (true, 1) => "1 file".to_string(),
            (true, f) => format!("{} files", group(f)),
            (false, _) => fmt_time(n.modified),
        };
        let mut s = format!("{}  ·  {}  ·  {}", full.display(), scan::human(n.size), what);
        if let Some(d) = self.diff.as_deref().and_then(|(d, _)| d.at(&self.full_of(abs))) {
            if d.old == snapshot::NEW {
                s.push_str("  ·  new since the snapshot");
            } else if d.old != n.size {
                s.push_str(&format!("  ·  was {} ({})", scan::human(d.old), signed(n.size as i64 - d.old as i64)));
            }
        }
        s
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
        let tree = self.shown_tree().unwrap();
        let mut n: &Node = &tree;
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
                let feel = self.feel();
                let treemap = self.map_kind == MapKind::Treemap;
                let back = ui.add_enabled(treemap && self.map.can_go_back(), egui::Button::new("◀")).on_hover_text("Back (Alt+Left, or the mouse's back button)");
                if back.clicked() {
                    self.map.back(&feel);
                }
                let fwd = ui.add_enabled(treemap && self.map.can_go_forward(), egui::Button::new("▶")).on_hover_text("Forward (Alt+Right)");
                if fwd.clicked() {
                    self.map.forward(&feel);
                }
                let zoom_sel = ui.add_enabled(self.selected.is_some(), egui::Button::new("Zoom to selection"));
                if zoom_sel.on_hover_text("Bring the selected file or folder into view").clicked() {
                    if let (Some(sel), Some(tree)) = (self.selected.clone(), self.shown_tree()) {
                        if treemap {
                            self.map.frame_path(&tree, &sel, &feel);
                        } else {
                            let mut z = sel;
                            if !tree.at(&z).is_some_and(|n| n.is_dir) {
                                z.pop();
                            }
                            self.zoom = z;
                        }
                    }
                }
                if ui.button("Zoom out").on_hover_text("Up one level (right-click on the map)").clicked() {
                    if let (true, Some(tree)) = (treemap, self.shown_tree()) {
                        self.map.zoom_out(&tree, &feel);
                    } else {
                        self.zoom.pop();
                    }
                }
                if ui.button("⟲ Whole map").on_hover_text("Back to the whole scan (Home)").clicked() {
                    if treemap {
                        self.map.reset(&feel);
                    } else {
                        self.zoom.clear();
                    }
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙ Settings").on_hover_text("Appearance, scan filters, treemap look and more").clicked() {
                    self.dialog.open(&self.settings);
                }
                self.help_menu(ui);
            });
        });
    }

    // ---------- help ----------

    /// The ❓ Help menu on the toolbar.
    fn help_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("❓ Help", |ui| {
            let pages = [
                ("Help Centre", "F1", topics::HOME),
                ("Getting Started", "", topics::QUICK_START),
                ("Keyboard Shortcuts", "", topics::KEYBOARD),
                ("Mouse Controls", "", topics::MOUSE),
                ("Search and Filter Examples", "", topics::EXAMPLES),
                ("Troubleshooting", "", topics::TROUBLESHOOTING),
                ("What's New", "", topics::WHATS_NEW),
            ];
            for (label, key, topic) in pages {
                if ui.add(egui::Button::new(label).shortcut_text(key)).clicked() {
                    self.help.show_topic(topic);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("About dirmap").clicked() {
                self.help.about_open = true;
                ui.close();
            }
        })
        .response
        .on_hover_text("The manual, shortcuts and troubleshooting (F1 for help on what's under the pointer)");
    }

    /// F1: help for the part of the window under the pointer (as drawn last frame).
    fn help_keys(&mut self, ctx: &egui::Context) {
        if !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            return;
        }
        if self.help.hovered {
            return;
        }
        let at = ctx.pointer_hover_pos().and_then(|p| {
            if self.dialog.rect.is_some_and(|r| r.contains(p)) {
                return Some(self.dialog.topic());
            }
            self.help_regions.iter().find(|(r, _)| r.contains(p)).map(|(_, t)| *t)
        });
        let fallback = if self.job.is_some() {
            topics::WHILE_SCANNING
        } else if self.tree.is_none() {
            topics::QUICK_START
        } else {
            topics::HOME
        };
        self.help.show_topic(at.unwrap_or(fallback));
    }

    /// The help page for the side panel's current tab.
    fn side_topic(&self) -> &'static str {
        match self.side_tab {
            SideTab::Types => topics::FILE_TYPES,
            SideTab::Sizes => topics::HISTOGRAM,
            SideTab::Changes => topics::SNAPSHOTS,
            SideTab::Duplicates => topics::DUPLICATES,
            SideTab::Sequences => topics::SEQUENCES,
        }
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
            let tip = match *title {
                "Name" => "File or folder name",
                "Size" => "Total size of everything inside (the files' own sizes, not space on disk)",
                "% of parent" => "Share of the folder this item is in",
                "Files" => "How many files are inside, counting subfolders",
                _ => "The most recent change to anything inside",
            };
            let resp = resp.on_hover_text(if sortable { format!("{tip}. Click to sort.") } else { tip.to_string() });
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
        let text_color = if selected {
            vis.selection.stroke.color
        } else if line.dim {
            vis.weak_text_color().gamma_multiply(0.6)
        } else {
            vis.text_color()
        };
        let weak = if selected || line.dim { text_color } else { vis.weak_text_color() };
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
        let color = if line.dim { custom::dim(line.color) } else { line.color };
        self.icons.paint(ui, icon, line.is_dir, scan::ext_of(line.name), color);
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
                    let c = if line.is_dir { Color32::from_rgb(90, 140, 220) } else { color };
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
        let Some(tree) = self.shown_tree() else { return };
        // In Highlight mode, files that don't pass the filter are dimmed here like on the maps.
        let dim_filter = (self.filter.active() && self.shown.is_none()).then(|| self.filter.clone());
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
        let mut menu_pick = None;
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
                    dim: !n.is_dir && dim_filter.as_ref().is_some_and(|f| !f.matches(&n.name, n.size)),
                };
                let selected = self.selected.as_deref() == Some(&row.path[..]);
                let arrow = self.paint_line(ui, rect, &line, selected, resp.hovered(), (range.start + k) % 2 == 1);
                let on_arrow = resp.interact_pointer_pos().is_some_and(|p| arrow.expand2(vec2(4.0, 0.0)).contains(p));
                if n.is_dir && ((resp.clicked() && on_arrow) || resp.double_clicked()) {
                    toggled = Some(row.path.clone());
                } else if resp.clicked() || resp.secondary_clicked() {
                    clicked = Some(row.path.to_vec());
                }
                // (The tree is taken out of `self` while drawing, so describe the row here.)
                let info = menus::ItemInfo {
                    is_dir: n.is_dir,
                    open: line.open,
                    is_root: row.path.is_empty(),
                    is_drive: row.path.is_empty() && self.scanned_path.parent().is_none(),
                    busy: self.ops.job.is_some(),
                };
                resp.context_menu(|ui| {
                    if let Some(a) = Self::tree_menu(ui, &info) {
                        menu_pick = Some((a, row.path.to_vec()));
                    }
                });
            }
        });
        self.rows = Some(rows);
        if let Some(p) = toggled {
            if !self.expanded.remove(&p) {
                self.expanded.insert(p);
            }
            self.rows = None;
        }
        if let Some(p) = clicked {
            self.selected = Some(p);
        }
        if let Some((a, p)) = menu_pick {
            self.tree_action(a, p);
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
                            dim: false,
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
                            dim: false,
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
        let total: u64 = self.shown_tree().map(|t| t.size).unwrap_or(0).max(1);
        if self.shown.is_some() {
            ui.weak("Matching files only");
        }
        if self.highlight_ext.is_some() && ui.small_button("Clear highlight").clicked() {
            self.highlight_ext = None;
        }
        let mut toggle = None;
        let mut menu_pick = None;
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
        // In "Show matches only" the list covers just the matches, like the maps.
        let exts = match &mut self.shown {
            Some(s) => std::mem::take(&mut s.exts),
            None => std::mem::take(&mut self.exts),
        };
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
                let resp = resp.on_hover_text("Click to highlight in the treemap, right-click for more");
                if resp.clicked() {
                    toggle = Some(e.ext.clone());
                }
                resp.context_menu(|ui| {
                    if let Some(a) = Self::ext_menu(ui, &e.ext, on) {
                        menu_pick = Some((a, e.ext.clone()));
                    }
                });
            }
        });
        match &mut self.shown {
            Some(s) => s.exts = exts,
            None => self.exts = exts,
        }
        if let Some((a, ext)) = menu_pick {
            self.ext_action(a, ext);
        }
        if let Some(t) = toggle {
            self.highlight_ext = if self.highlight_ext.as_ref() == Some(&t) { None } else { Some(t) };
        }
    }


    // ---------- right-hand panel ----------

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label) in
                [(SideTab::Types, "File types"), (SideTab::Sizes, "Sizes"), (SideTab::Changes, "Changes"), (SideTab::Duplicates, "Duplicates"), (SideTab::Sequences, "Sequences")]
            {
                let tip = match tab {
                    SideTab::Types => "Space used by each file type; click one to highlight it on the map",
                    SideTab::Sizes => "How many files fall into each size range",
                    SideTab::Changes => "Compare with a saved snapshot to see what grew or shrank",
                    SideTab::Duplicates => "Find files with exactly the same contents",
                    SideTab::Sequences => "Numbered file sequences (renders, scans) and their missing frames",
                };
                ui.selectable_value(&mut self.side_tab, tab, label).on_hover_text(tip);
            }
            if help::link_button(ui, "Help for this tab") {
                self.help.show_topic(self.side_topic());
            }
        });
        ui.separator();
        match self.side_tab {
            SideTab::Types => self.ext_panel(ui),
            SideTab::Sizes => self.sizes_panel(ui),
            SideTab::Changes => self.changes_panel(ui),
            SideTab::Duplicates => self.dupes_panel(ui),
            SideTab::Sequences => self.sequences_panel(ui),
        }
    }

    /// Histogram of file sizes in the zoomed folder; clicking a bar filters the map to that range.
    fn sizes_panel(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.shown_tree() else { return };
        let kinds = self.filter.kinds;
        let Some(zroot) = tree.at(&self.zoom) else { return };
        if self.hist.as_ref().is_none_or(|h| h.0 != self.zoom || h.1 != kinds) {
            fn walk(n: &Node, kinds: u16, b: &mut [(u64, u64); 9]) {
                if n.is_dir {
                    n.children.iter().for_each(|c| walk(c, kinds, b));
                } else if kinds == 0 || kinds & categories::of(&n.name).bit() != 0 {
                    let i = SIZE_BUCKETS.iter().position(|&(lo, hi, _)| n.size >= lo && (hi == 0 || n.size < hi)).unwrap_or(0);
                    b[i].0 += 1;
                    b[i].1 += n.size;
                }
            }
            let mut b = [(0, 0); 9];
            walk(zroot, kinds, &mut b);
            self.hist = Some((self.zoom.clone(), kinds, b));
        }
        let b = self.hist.as_ref().unwrap().2;
        ui.label(egui::RichText::new(format!("File sizes in {}", zroot.name)).strong());
        if kinds != 0 {
            ui.weak("Only the kinds picked under Show.");
        }
        ui.weak("Click a bar to show only that size range on the map.");
        let bytes_c = Color32::from_rgb(90, 140, 225);
        let files_c = Color32::from_gray(150);
        ui.horizontal(|ui| {
            for (c, t) in [(bytes_c, "space used"), (files_c, "number of files")] {
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, c);
                ui.weak(t);
                ui.add_space(8.0);
            }
        });
        ui.add_space(4.0);
        let max_bytes = b.iter().map(|x| x.1).max().unwrap_or(0).max(1) as f32;
        let max_files = b.iter().map(|x| x.0).max().unwrap_or(0).max(1) as f32;
        let font = FontId::proportional(13.0);
        let mut pick = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, &(lo, hi, label)) in SIZE_BUCKETS.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
                let on = self.filter.larger == lo && self.filter.smaller == hi && (lo > 0 || hi > 0);
                let vis = ui.visuals();
                if on {
                    ui.painter().rect_filled(rect, 3.0, vis.selection.bg_fill);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 3.0, vis.widgets.hovered.weak_bg_fill);
                }
                let tc = if on { vis.selection.stroke.color } else { vis.text_color() };
                let p = ui.painter();
                p.text(rect.left_top() + vec2(6.0, 4.0), Align2::LEFT_TOP, label, font.clone(), tc);
                let (files, bytes) = b[i];
                let info = format!("{} files · {}", group(files), scan::human(bytes));
                p.text(pos2(rect.right() - 6.0, rect.top() + 4.0), Align2::RIGHT_TOP, info, font.clone(), vis.weak_text_color());
                let w = rect.width() - 12.0;
                let bar = Rect::from_min_size(rect.left_top() + vec2(6.0, 22.0), vec2(w * bytes as f32 / max_bytes, 9.0));
                p.rect_filled(bar, 2.0, bytes_c);
                let bar = Rect::from_min_size(rect.left_top() + vec2(6.0, 33.0), vec2(w * files as f32 / max_files, 5.0));
                p.rect_filled(bar, 2.0, files_c);
                if resp.clicked() {
                    pick = Some(if on { (0, 0) } else { (lo, hi) });
                }
            }
        });
        if let Some((lo, hi)) = pick {
            self.filter.larger = lo;
            self.filter.smaller = hi;
        }
    }

    fn save_snapshot(&mut self) {
        let Some(tree) = &self.tree else { return };
        let label: String = tree.name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let name = format!("dirmap {} {}.dmsnap", label.trim_matches('_'), chrono::Local::now().format("%Y-%m-%d %H%M"));
        let dialog = rfd::FileDialog::new().add_filter("dirmap snapshot", &["dmsnap"]).set_file_name(name);
        if let Some(f) = dialog.save_file() {
            self.snap_msg = Some(match snapshot::save(&f, &self.scanned_path, tree) {
                Ok(()) => format!("Saved {}", f.display()),
                Err(e) => format!("Couldn't save the snapshot: {e}"),
            });
        }
    }

    fn load_snapshot(&mut self) {
        if let Some(f) = rfd::FileDialog::new().add_filter("dirmap snapshot", &["dmsnap"]).pick_file() {
            self.load_snapshot_from(f);
        }
    }

    fn load_snapshot_from(&mut self, f: PathBuf) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(snapshot::load(&f).map_err(|e| format!("Couldn't open {}: {e}", f.display())));
        });
        self.snap_rx = Some(rx);
        self.snap_msg = None;
    }

    /// Compare with a saved scan: totals and the biggest changes, newest first.
    fn changes_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("💾 Save snapshot…").on_hover_text("Save this scan so a later scan can be compared with it").clicked() {
                self.save_snapshot();
            }
            if ui.button("📂 Compare with snapshot…").clicked() {
                self.load_snapshot();
            }
            if self.snapshot.is_some() && ui.button("Stop comparing").clicked() {
                self.snapshot = None;
                self.diff = None;
                if self.color_mode == ColorMode::Change {
                    self.color_mode = ColorMode::Type;
                }
                self.paint_gen += 1;
            }
        });
        if self.snap_rx.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Opening snapshot…");
            });
        }
        if let Some(m) = &self.snap_msg {
            ui.label(m);
        }
        let (Some(snap), Some(tree)) = (&self.snapshot, &self.tree) else {
            ui.add_space(8.0);
            ui.label("Save a snapshot of this scan, then later scan again and compare with it to see what is new, what was deleted and what grew.");
            return;
        };
        ui.add_space(4.0);
        ui.label(format!("Compared with {} as it was on {}.", snap.root.display(), fmt_time(snap.taken)));
        if !same_path(&snap.root, &self.scanned_path) {
            ui.colored_label(ui.visuals().warn_fg_color, "That snapshot is of a different folder.");
        }
        let delta = tree.size as i64 - snap.tree.size as i64;
        ui.strong(format!("Then {}  ·  now {}  ·  {}", scan::human(snap.tree.size), scan::human(tree.size), signed(delta)));
        let Some(diff) = self.diff.clone() else { return };
        let changes = &diff.1;
        let count = |f: &dyn Fn(&Change) -> bool| changes.iter().filter(|c| f(c)).count() as u64;
        let (new, gone) = (count(&|c| c.old == 0 && c.path.is_some()), count(&|c| c.path.is_none()));
        let (grew, shrank) = (count(&|c| c.old > 0 && c.new > c.old), count(&|c| c.path.is_some() && c.new < c.old));
        ui.label(format!("{} new · {} deleted · {} grew · {} shrank", group(new), group(gone), group(grew), group(shrank)));
        if self.color_mode != ColorMode::Change && ui.button("Color the map by change").clicked() {
            self.color_mode = ColorMode::Change;
            self.paint_gen += 1;
        }
        ui.add_space(4.0);
        let font = FontId::proportional(13.0);
        let mut clicked = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, changes.len(), |ui, range| {
            for (k, c) in changes[range.clone()].iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let vis = ui.visuals().clone();
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 2.0, vis.widgets.hovered.weak_bg_fill);
                } else if (range.start + k) % 2 == 1 && self.settings.stripes {
                    ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
                }
                let (tag, color) = if c.path.is_none() {
                    ("deleted", Color32::from_rgb(70, 150, 225))
                } else if c.old == 0 {
                    ("new", Color32::from_rgb(235, 90, 70))
                } else if c.new > c.old {
                    ("", Color32::from_rgb(240, 150, 50))
                } else {
                    ("", Color32::from_rgb(70, 150, 225))
                };
                let icon = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 8.0), vec2(16.0, 16.0));
                self.icons.paint(ui, icon, c.is_dir, scan::ext_of(&c.name), Color32::GRAY);
                let right = rect.right() - 4.0;
                let cy = rect.center().y;
                let amount = signed(c.delta());
                ui.painter().text(pos2(right, cy), Align2::RIGHT_CENTER, &amount, font.clone(), color);
                let clip = Rect::from_min_max(pos2(rect.left() + 24.0, rect.top()), pos2(right - 90.0, rect.bottom()));
                let p = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
                let mut x = clip.left();
                if !tag.is_empty() {
                    let g = p.layout_no_wrap(tag.to_string(), FontId::proportional(11.0), color);
                    let w = g.size().x;
                    p.galley(pos2(x, cy - g.size().y / 2.0), g, color);
                    x += w + 6.0;
                }
                p.text(pos2(x, cy), Align2::LEFT_CENTER, &c.name, font.clone(), vis.text_color());
                let tip = format!("{}\nwas {}, now {}", c.name, scan::human(c.old), scan::human(c.new));
                if resp.on_hover_text(tip).clicked() {
                    clicked = c.path.clone();
                }
            }
        });
        if let Some(p) = clicked {
            self.select_full(p);
        }
    }

    fn start_dupes(&mut self) {
        let Some(tree) = &self.tree else { return };
        fn walk(n: &Node, min: u64, path: &mut Vec<u32>, disk: &mut PathBuf, out: &mut Vec<dupes::Candidate>) {
            for (i, c) in n.children.iter().enumerate() {
                path.push(i as u32);
                disk.push(&*c.name);
                if c.is_dir {
                    walk(c, min, path, disk, out);
                } else if c.size >= min {
                    out.push(dupes::Candidate { tree_path: path.clone(), path: disk.clone(), size: c.size });
                }
                disk.pop();
                path.pop();
            }
        }
        let mut cands = Vec::new();
        walk(tree, self.dup_min.max(1), &mut Vec::new(), &mut self.scanned_path.clone(), &mut cands);
        let progress = Arc::new(dupes::Progress::default());
        let (tx, rx) = mpsc::channel();
        let p = progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(dupes::find(cands, &p));
        });
        self.dup_job = Some(DupJob { progress, rx });
        self.dup_groups.clear();
        self.dup_rows.clear();
        self.dup_done = false;
    }

    fn dupes_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Files of at least");
            size_edit(ui, &mut self.dup_min);
        });
        if let Some(j) = &self.dup_job {
            let (read, total) = (j.progress.read.load(Ordering::Relaxed), j.progress.to_read.load(Ordering::Relaxed));
            let frac = if total > 0 { read as f32 / total as f32 } else { 0.0 };
            let text = format!("Comparing files… {} of {}", scan::human(read), scan::human(total));
            ui.add(egui::ProgressBar::new(frac.min(1.0)).text(text));
            if ui.button("Cancel").clicked() {
                j.progress.cancel.store(true, Ordering::Relaxed);
            }
            return;
        }
        if ui.button("🔍 Find duplicates").on_hover_text("Reads the files to compare their contents").clicked() {
            self.start_dupes();
            return;
        }
        if self.dup_groups.is_empty() {
            ui.add_space(8.0);
            ui.label(if self.dup_done {
                "No duplicates found."
            } else {
                "Finds files with exactly the same contents. Files are first matched by size, so only possible duplicates are read."
            });
            if ui.link("How duplicates are found").clicked() {
                self.help.show_topic(topics::DUPLICATES_HOW);
            }
            return;
        }
        let wasted: u64 = self.dup_groups.iter().map(|g| g.wasted()).sum();
        ui.strong(format!("{} sets of duplicates · {} could be freed", group(self.dup_groups.len() as u64), scan::human(wasted)));
        if ui.checkbox(&mut self.dupes_only, "Show only duplicates on the map").changed() {
            self.paint_gen += 1;
        }
        let font = FontId::proportional(13.0);
        let mut clicked = None;
        let groups = std::mem::take(&mut self.dup_groups);
        let rows = std::mem::take(&mut self.dup_rows);
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for &(gi, fi) in &rows[range] {
                let g = &groups[gi];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let vis = ui.visuals().clone();
                let cy = rect.center().y;
                match fi {
                    None => {
                        ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
                        let t = format!("{} copies · {} each · {} extra", g.files.len(), scan::human(g.size), scan::human(g.wasted()));
                        ui.painter().text(pos2(rect.left() + 4.0, cy), Align2::LEFT_CENTER, t, font.clone(), vis.strong_text_color());
                    }
                    Some(fi) => {
                        let f = &g.files[fi];
                        let selected = self.selected.as_deref() == Some(&f.tree_path[..]);
                        if selected {
                            ui.painter().rect_filled(rect, 2.0, vis.selection.bg_fill);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 2.0, vis.widgets.hovered.weak_bg_fill);
                        }
                        let name = f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        let icon = Rect::from_min_size(pos2(rect.left() + 14.0, cy - 8.0), vec2(16.0, 16.0));
                        self.icons.paint(ui, icon, false, scan::ext_of(&name), Color32::GRAY);
                        let shown = f.path.strip_prefix(&self.scanned_path).unwrap_or(&f.path).display().to_string();
                        let tc = if selected { vis.selection.stroke.color } else { vis.text_color() };
                        let clip = Rect::from_min_max(pos2(rect.left() + 34.0, rect.top()), pos2(rect.right() - 4.0, rect.bottom()));
                        ui.painter().with_clip_rect(clip.intersect(ui.clip_rect())).text(
                            pos2(clip.left(), cy),
                            Align2::LEFT_CENTER,
                            shown,
                            font.clone(),
                            tc,
                        );
                        if resp.on_hover_text(f.path.display().to_string()).clicked() {
                            clicked = Some(f.tree_path.clone());
                        }
                    }
                }
            }
        });
        self.dup_groups = groups;
        self.dup_rows = rows;
        if let Some(p) = clicked {
            self.select_full(p);
        }
    }

    // ---------- sunburst ----------

    fn sunburst_panel(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click());
        let Some(tree) = self.shown_tree() else { return };
        let back = self.shown_back();
        if tree.at(&self.zoom).is_none() {
            self.zoom.clear();
        }
        let zroot = tree.at(&self.zoom).unwrap();
        if self.sun.as_ref().is_none_or(|c| c.zoom != self.zoom) {
            let segs = sunburst::layout(zroot, 6, 0.003);
            self.sun = Some(SunCache { zoom: self.zoom.clone(), segs });
        }
        let segs = &self.sun.as_ref().unwrap().segs;
        let rect = resp.rect;
        let center = rect.center();
        let radius = (rect.width().min(rect.height()) / 2.0 - 10.0).max(30.0);
        let rings = segs.iter().map(|s| s.ring).max().unwrap_or(1);
        let hole = radius * 0.17;
        let ring_w = (radius - hole) / rings as f32;
        let r_in = |ring: u16| hole + (ring - 1) as f32 * ring_w;
        let backdrop = if ui.visuals().dark_mode { Color32::from_gray(30) } else { Color32::from_gray(236) };
        painter.rect_filled(rect, 0.0, backdrop);

        let pointer = resp.hover_pos();
        let in_hole = pointer.is_some_and(|p| (p - center).length() < hole);
        let hover = pointer.and_then(|p| {
            let d = p - center;
            let r = d.length();
            if r < hole || r > radius {
                return None;
            }
            let ring = ((r - hole) / ring_w) as u16 + 1;
            let a = sunburst::angle_of(d.x, d.y);
            segs.iter().position(|s| s.ring == ring && a >= s.a0 && a < s.a1)
        });

        let filtering = self.filter.active() || self.dupes_only;
        let need_full = self.dupes_only || self.color_mode == ColorMode::Change;
        let mut mesh = egui::Mesh::default();
        let mut abs = self.zoom.clone();
        let zl = abs.len();
        let mut colors = Vec::with_capacity(segs.len());
        for (i, s) in segs.iter().enumerate() {
            let n = zroot.at(&s.path).unwrap();
            abs.truncate(zl);
            abs.extend_from_slice(&s.path);
            // Duplicates and changes are recorded against the whole scan.
            let full = match (&back, need_full) {
                (Some(b), true) => b.to_full(&abs),
                _ => abs.clone(),
            };
            let mut c = match (n.is_dir, self.color_mode) {
                // Folders take the hue of the top-level folder they're in, darker further out.
                (true, ColorMode::Type) => {
                    // (shifted a little by the second level, so a view dominated by one folder isn't one color)
                    let second = s.path.get(1).map_or(0.0, |&i| (i % 7) as f32 * 0.045);
                    let hue = (s.path[0] as f32 * 0.618_034 + 0.08 + second).fract();
                    Hsva::new(hue, 0.4, (0.82 - 0.07 * (s.ring - 1) as f32).max(0.4), 1.0).into()
                }
                // A folder's newest file says little about the folder; keep folders neutral so files stand out.
                (true, ColorMode::Age) => Color32::from_gray((110.0 - 8.0 * (s.ring - 1) as f32).max(60.0) as u8),
                (true, ColorMode::Change) => self.node_color(n, &full).gamma_multiply(0.75),
                (false, _) => self.node_color(n, &full),
            };
            if n.is_dir {
                if filtering && self.shown.is_none() {
                    c = c.gamma_multiply(0.4);
                }
            } else if self.dupes_only && !self.dup_set.contains(&full) {
                c = Color32::from_gray(45);
            } else if self.filter.active() && !self.filter.matches(&n.name, n.size) {
                c = custom::dim(c);
            } else if let Some(h) = &self.highlight_ext {
                if !n.ext().eq_ignore_ascii_case(h) {
                    c = c.gamma_multiply(0.2);
                }
            }
            if Some(i) == hover {
                c = lighten(c, 0.3);
            }
            colors.push(c);
            add_arc(&mut mesh, center, r_in(s.ring), r_in(s.ring) + ring_w, s.a0, s.a1, c);
        }
        painter.add(mesh);

        // Names on segments with room for them.
        if self.view().show_labels && ring_w >= 14.0 {
            let font = FontId::proportional(11.0);
            for (s, &c) in segs.iter().zip(&colors) {
                let rm = r_in(s.ring) + ring_w / 2.0;
                let span = s.a1 - s.a0;
                if rm * span < 40.0 {
                    continue;
                }
                let n = zroot.at(&s.path).unwrap();
                let tc = if luma(c) > 140.0 { Color32::from_black_alpha(220) } else { Color32::from_white_alpha(235) };
                let g = painter.layout_no_wrap(n.name.to_string(), font.clone(), tc);
                let (w, h) = (g.size().x, g.size().y);
                let a = (s.a0 + s.a1) / 2.0;
                let (sin, cos) = (a.sin().abs(), a.cos().abs());
                // The text is horizontal: check it fits along the arc and across the ring.
                if w * cos + h * sin <= rm * span * 0.9 && w * sin + h * cos <= ring_w - 2.0 {
                    let pos = center + vec2(a.sin(), -a.cos()) * rm - vec2(w, h) / 2.0;
                    painter.galley(pos, g, tc);
                }
            }
        }

        // Middle: the folder being shown; click it to go up a level.
        let mid = if in_hole && !self.zoom.is_empty() { Color32::from_gray(80) } else { Color32::from_gray(55) };
        painter.circle_filled(center, hole - 1.0, mid);
        // Shorten the name until it fits inside the circle.
        let font = FontId::proportional(13.0);
        let mut name = zroot.name.to_string();
        let fits = |t: &str| painter.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x <= hole * 1.7;
        if !fits(&name) {
            let mut chars: Vec<char> = name.chars().collect();
            while chars.len() > 1 && !fits(&format!("{}…", chars.iter().collect::<String>())) {
                chars.pop();
            }
            name = format!("{}…", chars.iter().collect::<String>());
        }
        painter.text(center - vec2(0.0, 8.0), Align2::CENTER_CENTER, name, font, Color32::WHITE);
        painter.text(center + vec2(0.0, 9.0), Align2::CENTER_CENTER, scan::human(zroot.size), FontId::proportional(12.0), Color32::LIGHT_GRAY);

        // Outline the selection (or its nearest shown ancestor).
        if let Some(sel) = self.selected.as_ref().filter(|s| s.starts_with(&self.zoom)) {
            let rel = &sel[zl..];
            if let Some(s) = segs.iter().filter(|s| rel.starts_with(&s.path)).max_by_key(|s| s.path.len()) {
                let pts = arc_outline(center, r_in(s.ring), r_in(s.ring) + ring_w, s.a0, s.a1);
                painter.add(egui::Shape::closed_line(pts, Stroke::new(2.0, self.view().selection_color)));
            }
        }

        let prev_status = self.hover_status.take();
        let child = hover.map(|i| segs[i].path[0]);
        let mut zoom_to = wheel_zoom(&mut self.wheel, ui, resp.hovered(), &tree, &self.zoom, child);
        let mut select = None;
        if let Some(i) = hover {
            let mut abs = self.zoom.clone();
            abs.extend_from_slice(&segs[i].path);
            self.hover_status = Some(self.hover_text(&abs));
            let is_dir = tree.at(&abs).is_some_and(|n| n.is_dir);
            if resp.double_clicked() {
                if !is_dir {
                    abs.pop();
                }
                zoom_to = Some(abs);
            } else if resp.clicked() {
                select = Some(abs);
            }
        } else if in_hole {
            self.hover_status = Some(format!("{}  ·  click to go up a level", self.full_path(&self.zoom).display()));
            if resp.clicked() && !self.zoom.is_empty() {
                zoom_to = Some(self.zoom[..zl - 1].to_vec());
            }
        }
        if resp.secondary_clicked() && !self.zoom.is_empty() {
            zoom_to = Some(self.zoom[..zl - 1].to_vec());
        }
        if zoom_to.is_some() || select.is_some() || self.hover_status != prev_status {
            ui.ctx().request_repaint();
        }
        if let Some(z) = zoom_to {
            self.zoom = z;
        }
        if let Some(p) = select {
            self.select(p);
        }
    }

    // ---------- treemap ----------

    /// How navigation feels, from the settings.
    fn feel(&self) -> Feel {
        let v = self.view();
        // DIRMAP_SLOWMO=N slows every animation N times, for checking them frame by frame.
        let slow = std::env::var("DIRMAP_SLOWMO").ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(1.0).max(0.01);
        Feel { smooth: v.smooth_zoom, sensitivity: v.zoom_sensitivity as f64, speed: v.anim_speed as f64 / slow }
    }

    /// Lighting and effects for the treemap shader, from the settings.
    fn look(&self) -> Look {
        let v = self.view();
        let sh = Shading::new(v);
        Look {
            ambient: sh.ambient,
            diffuse: sh.diffuse,
            gain: sh.gain,
            sun: sh.light,
            grid: sh.grid,
            lighting: v.lighting,
            light_level: v.light_level.factor(),
            bevel: v.depth_effect,
        }
    }

    /// Give the map new colouring inputs when anything that changes tile colours has changed.
    fn update_map_paint(&mut self) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.paint_gen, &self.highlight_ext, &self.filter, self.dupes_only, self.color_mode, self.view().treemap_key()).hash(&mut h);
        self.shown.as_ref().map(|s| Arc::as_ptr(&s.tree) as usize).hash(&mut h);
        let key = h.finish();
        if self.map_paint == Some(key) {
            return;
        }
        self.map_paint = Some(key);
        let filtering = self.filter.active();
        let mode = self.color_mode;
        // In "Show matches only" tiles have paths in the filtered tree; duplicates and changes
        // are recorded against the whole scan.
        let back = self.shown_back().filter(|_| self.dupes_only || mode == ColorMode::Change);
        let colors: Arc<HashMap<String, Color32>> = Arc::new(self.colors.clone());
        let diff = self.diff.clone();
        let filter = self.filter.clone();
        let dupes_only = self.dupes_only;
        let dup_set = self.dup_set.clone();
        let highlight = self.highlight_ext.clone();
        let colorize: mapview::Colorizer = Arc::new(move |n: &Node, abs: &[u32]| {
            let full;
            let abs = match &back {
                Some(b) => {
                    full = b.to_full(abs);
                    &full[..]
                }
                None => abs,
            };
            if dupes_only && !dup_set.contains(abs) {
                return None;
            }
            let c = match mode {
                ColorMode::Type => {
                    let ext = n.ext();
                    match colors.get(ext) {
                        Some(&c) => c,
                        None => colors.get(&ext.to_lowercase()).copied().unwrap_or(Color32::GRAY),
                    }
                }
                ColorMode::Age => age_color(n.modified),
                ColorMode::Change => match diff.as_deref().and_then(|(d, _)| d.at(abs)) {
                    Some(d) => change_color(d.old, n.size),
                    None => Color32::from_gray(95),
                },
            };
            // Highlight mode: what doesn't match stays in place, dimmed.
            if filtering && !filter.matches(&n.name, n.size) {
                let d = custom::dim(c);
                return Some((d, d));
            }
            let fill = match &highlight {
                Some(h) if !n.ext().eq_ignore_ascii_case(h) => c.gamma_multiply(0.15),
                _ => c,
            };
            Some((fill, c))
        });
        let background = if dupes_only { Color32::from_gray(24) } else { Color32::from_gray(45) };
        let v = self.view();
        let cushion = Cushion { height: v.height as f64, scale: v.scale as f64 };
        let look = self.look();
        self.map.set_paint(Paint { key, colorize, cushion, background, look });
    }

    fn treemap_panel(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let bounds = resp.rect;
        let Some(tree) = self.shown_tree() else { return };
        if tree.at(&self.zoom).is_none() {
            self.zoom.clear();
        }
        let ctx = ui.ctx().clone();
        let feel = self.feel();
        self.update_map_paint();
        self.map.sync(&ctx, &tree, &self.zoom, bounds, &feel);

        // ---- input: everything goes through the one camera ----
        // egui may run a frame twice (a discarded sizing pass) with the same input; act on it once.
        let first_pass = ctx.current_pass_index() == 0;
        let pointer = resp.hover_pos();
        let typing = ctx.egui_wants_keyboard_input();
        // The help window has its own Back and Forward.
        let help_hovered = self.help.hovered;
        let (wheel, pinch, back, forward, home, plus, minus) = ui.input(|i| {
            let mut wheel = 0.0f64;
            for e in &i.raw.events {
                if let egui::Event::MouseWheel { delta, unit, modifiers, .. } = e {
                    // Ctrl+wheel arrives as a pinch zoom below on most platforms; don't count it twice.
                    if modifiers.ctrl || modifiers.command {
                        continue;
                    }
                    wheel += match unit {
                        egui::MouseWheelUnit::Point => delta.y as f64 / camera::NOTCH_POINTS,
                        egui::MouseWheelUnit::Line => delta.y as f64,
                        egui::MouseWheelUnit::Page => delta.y as f64 * 4.0,
                    };
                }
            }
            let alt = i.modifiers.alt;
            (
                wheel,
                i.zoom_delta() as f64,
                (!help_hovered && i.pointer.button_pressed(egui::PointerButton::Extra1)) || (!typing && alt && i.key_pressed(egui::Key::ArrowLeft)),
                (!help_hovered && i.pointer.button_pressed(egui::PointerButton::Extra2)) || (!typing && alt && i.key_pressed(egui::Key::ArrowRight)),
                !typing && i.key_pressed(egui::Key::Home),
                !typing && (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)),
                !typing && i.key_pressed(egui::Key::Minus),
            )
        });
        if let Some(p) = pointer.filter(|_| first_pass) {
            let mut amount = wheel * camera::NOTCH * feel.sensitivity;
            if (pinch - 1.0).abs() > 1e-6 {
                amount += pinch.ln() * feel.sensitivity.sqrt();
            }
            if amount != 0.0 {
                self.map.zoom_wheel(amount, p, bounds, &feel);
            }
            // Zooming while moving the pointer: the point under the pointer *now* stays put.
            self.map.follow_pointer(p, bounds);
        }
        if first_pass && (plus || minus) {
            let at = pointer.unwrap_or(bounds.center());
            let dir = if plus { 1.0 } else { -1.0 };
            self.map.zoom_wheel(dir * camera::NOTCH * 2.0 * feel.sensitivity, at, bounds, &feel);
        }
        // Drag to pan (left or middle button). A drag never selects.
        if resp.dragged_by(egui::PointerButton::Primary) || resp.dragged_by(egui::PointerButton::Middle) {
            self.map.dragging = true;
            if first_pass {
                self.map.pan(resp.drag_delta(), bounds);
            }
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if self.map.dragging {
            self.map.dragging = false;
            let v = ui.input(|i| i.pointer.velocity());
            self.map.release(v, &feel);
        } else if resp.hovered() && !self.map.camera.is_full() {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
        }
        let mut select = None;
        if first_pass && (resp.clicked() || resp.double_clicked()) && !self.map.in_transition() {
            if let Some(p) = resp.interact_pointer_pos() {
                if let Some((rel, rect)) = self.map.hit(&tree, bounds, p) {
                    let mut abs = self.map.root().unwrap_or(&[]).to_vec();
                    abs.extend_from_slice(&rel);
                    if resp.double_clicked() {
                        // Into the folder that holds the tile (or the folder itself), animated. A
                        // file directly in the folder on screen is framed instead.
                        let n = tree.at(&abs).unwrap();
                        let mut z = abs.clone();
                        if !n.is_dir {
                            z.pop();
                        }
                        if z.len() > self.zoom.len() {
                            self.zoom = z;
                        } else {
                            let _ = rect;
                            self.map.frame_path(&tree, &abs, &feel);
                        }
                    } else {
                        select = Some(abs);
                    }
                }
            }
        }
        if first_pass {
            if resp.secondary_clicked() {
                self.map.zoom_out(&tree, &feel);
            }
            if back {
                self.map.back(&feel);
            }
            if forward {
                self.map.forward(&feel);
            }
            if home {
                self.map.reset(&feel);
            }
        }
        if let Some(z) = self.map.take_root_request() {
            self.zoom = z;
            self.map.sync(&ctx, &tree, &self.zoom, bounds, &feel);
        }

        // ---- draw ----
        let look = self.look();
        let frame = self.map.render(&painter, &tree, bounds, pointer, &look, &feel);
        let root = self.map.root().unwrap_or(&[]).to_vec();
        let zroot = tree.at(&root).unwrap_or(&tree);

        // File names on tiles with room for them, fading in with size.
        if self.view().show_labels {
            let font = FontId::proportional(11.0);
            let labels = self.map.labels(bounds);
            for (r, l, a) in labels.into_iter().take(2500) {
                let Some(n) = zroot.at(&l.path) else { continue };
                let dark_text = luma(l.color) > 140.0;
                let tc = if dark_text { Color32::from_black_alpha(220) } else { Color32::from_white_alpha(235) };
                let shadow = if dark_text { Color32::from_white_alpha(60) } else { Color32::from_black_alpha(110) };
                let p = painter.with_clip_rect(r.shrink(1.0).intersect(painter.clip_rect()));
                let at = r.min + vec2(3.0, 2.0);
                p.text(at + vec2(0.0, 1.0), Align2::LEFT_TOP, &*n.name, font.clone(), shadow.gamma_multiply(a));
                p.text(at, Align2::LEFT_TOP, &*n.name, font.clone(), tc.gamma_multiply(a));
                let sa = a * mapview_smooth(26.0, 32.0, r.height());
                if sa > 0.02 {
                    p.text(r.min + vec2(3.0, 15.0), Align2::LEFT_TOP, scan::human(n.size), font.clone(), tc.gamma_multiply(0.8 * sa));
                }
            }
        }

        // Outline the selection (or the nearest ancestor big enough to see), at any zoom.
        if !self.map.in_transition() {
            if let Some(sel) = self.selected.as_ref().filter(|s| s.starts_with(&root)) {
                let rel = &sel[root.len()..];
                let rects = treemap::ancestors(zroot, self.map.camera.world(), rel);
                if let Some(r) = rects.iter().rev().map(|r| self.map.to_screen(bounds, r)).find(|r| r.width() >= 3.0 && r.height() >= 3.0) {
                    let stroke = Stroke::new(2.0, self.view().selection_color);
                    painter.with_clip_rect(bounds).rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
                }
            }
        }

        let prev_status = self.hover_status.take();
        if let Some((rel, _)) = &frame.hover {
            // The folder around the hovered tile, faintly.
            if let Some((_, parent)) = rel.split_last() {
                if let Some(pr) = self.map.locate(&tree, parent).filter(|_| !parent.is_empty()) {
                    let stroke = Stroke::new(1.0, Color32::from_white_alpha(110));
                    painter.with_clip_rect(bounds).rect_stroke(self.map.to_screen(bounds, &pr), 0.0, stroke, egui::StrokeKind::Inside);
                }
            }
            let mut abs = root.clone();
            abs.extend_from_slice(rel);
            self.hover_status = Some(self.hover_text(&abs));
        }
        if std::env::var_os("DIRMAP_PROFILE").is_some() {
            let text = format!(
                "{} · zoom ×{:.1} · frame {:.2} ms CPU · last build {:.1} ms",
                self.map.backend.name(),
                self.map.camera.zoom(),
                self.map.frame_cpu_ms,
                self.map.last_build_ms
            );
            painter.text(bounds.right_bottom() - vec2(6.0, 6.0), Align2::RIGHT_BOTTOM, text, FontId::monospace(11.0), Color32::WHITE);
        }
        let selecting = select.is_some();
        if let Some(p) = select {
            self.select(p);
        }
        // The status bar was drawn before this panel, so repaint when what it should say has changed.
        if frame.animating || selecting || self.map.labels_fading() || self.hover_status != prev_status {
            ctx.request_repaint();
        }
    }

    /// Breadcrumbs for the zoomed folder, plus the "show only" filters for the treemap.
    fn filter_bar(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.shown_tree() else { return };
        let total = self.tree.as_ref().map_or(0, |t| t.size);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut go = None;
            if ui.small_button(format!("🏠 {}", tree.name)).clicked() {
                go = Some(0);
            }
            let mut n: &Node = &tree;
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
            ui.weak("Wheel to zoom, drag to pan, right-click to zoom out.");
            if help::link_button(ui, "Moving around the map") {
                self.help.show_topic(match self.map_kind {
                    MapKind::Treemap => topics::TREEMAP_NAV,
                    MapKind::Sunburst => topics::SUNBURST,
                });
            }
            ui.separator();
            ui.label("View:");
            ui.selectable_value(&mut self.map_kind, MapKind::Treemap, "Treemap")
                .on_hover_text("Rectangles sized by space used; zoom and pan freely");
            ui.selectable_value(&mut self.map_kind, MapKind::Sunburst, "Sunburst")
                .on_hover_text("Rings: the centre is the folder, each ring one level deeper");
            ui.separator();
            ui.label("Color by:");
            let before = self.color_mode;
            ui.selectable_value(&mut self.color_mode, ColorMode::Type, "Type").on_hover_text("A colour per file type, as in the File types list");
            ui.selectable_value(&mut self.color_mode, ColorMode::Age, "Age").on_hover_text("How long since each file last changed");
            let has_diff = self.diff.is_some();
            ui.add_enabled_ui(has_diff, |ui| {
                ui.selectable_value(&mut self.color_mode, ColorMode::Change, "Change").on_disabled_hover_text(
                    "Compare with a saved snapshot first (Changes tab on the right)",
                );
            });
            if self.color_mode != before {
                self.paint_gen += 1;
            }
        });
        // What the colors mean, for the modes that aren't self-explanatory.
        let legend: Vec<(Color32, &str)> = match self.color_mode {
            ColorMode::Type => Vec::new(),
            ColorMode::Age => AGE_STOPS.iter().map(|(_, c, t)| (Color32::from_rgb(c[0], c[1], c[2]), *t)).collect(),
            ColorMode::Change => vec![
                (change_color(snapshot::NEW, 1), "new"),
                (change_color(1, 3), "grew"),
                (change_color(3, 1), "shrank"),
                (change_color(1, 1), "unchanged"),
            ],
        };
        if !legend.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.weak(if self.color_mode == ColorMode::Age { "Last changed:" } else { "Since the snapshot:" });
                for (c, t) in legend {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 10.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, c);
                    ui.label(t);
                    ui.add_space(6.0);
                }
            });
        }
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
            self.custom_button(ui);
        });
        self.custom_row(ui);
        ui.horizontal_wrapped(|ui| {
            ui.label("Larger than");
            size_edit(ui, &mut self.filter.larger);
            ui.label("Smaller than");
            size_edit(ui, &mut self.filter.smaller);
            if (self.filter.active() || self.custom.error.is_some()) && ui.button("Clear filters").clicked() {
                self.filter = ViewFilter::default();
                self.clear_custom();
            }
            if self.filter.active() {
                self.filter_summary(ui, total);
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
pub(crate) struct Shading {
    ambient: f32,
    diffuse: f32,
    gain: f32,
    light: [f32; 3],
    grid: Option<Color32>,
}

impl Shading {
    pub(crate) fn new(s: &Settings) -> Self {
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

    /// The original CPU renderer, kept for the before/after benchmark.
    #[cfg(test)]
    pub(crate) fn paint(&self, pixels: &mut [Color32], stride: usize, t: &Tile, c: Color32) {
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
        eframe::set_value(storage, "custom_filter", &self.custom.prefs);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let view_theme = self.view().theme;
        if self.applied_theme != Some(view_theme) {
            self.view().apply_theme(&ctx);
            self.applied_theme = Some(view_theme);
        }
        scan::set_decimal_units(self.view().units == settings::SizeUnits::Decimal);

        self.help_keys(&ctx);
        let about = help::AboutInfo { renderer: self.renderer, elevated: elevate::is_elevated() };
        if let Some(help::Action::ApplyFilter(p)) = self.help.ui(&ctx, &about) {
            self.set_custom(&p);
        }
        self.help_regions.clear();

        if let settings::Outcome::Apply = self.dialog.show(&ctx) {
            if self.dialog.draft.admin != self.settings.admin {
                elevate::save_mode(self.dialog.draft.admin);
                self.admin_prompt = false;
            }
            self.settings = self.dialog.draft.clone();
            self.rows = None;
        }
        if let Some(t) = self.dialog.help_request.take() {
            self.help.show_topic(t);
        }
        if std::mem::take(&mut self.dialog.restart_as_admin) {
            self.restart_as_admin(&ctx);
        }
        self.admin_question(&ctx);
        self.tree_shortcuts(&ctx);
        self.ops_ui(&ctx);

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
                    self.scan_errors = job.progress.errors.load(Ordering::Relaxed);
                    self.finish_scan(job.root, tree, exts, job.started.elapsed().as_secs_f32());
                } else {
                    std::thread::spawn(move || drop(tree));
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        // A snapshot finished loading, or a duplicate search finished.
        if let Some(rx) = &self.snap_rx {
            match rx.try_recv() {
                Ok(Ok(snap)) => {
                    self.snap_rx = None;
                    self.snapshot = Some(snap);
                    self.compare();
                    self.side_tab = SideTab::Changes;
                }
                Ok(Err(e)) => {
                    self.snap_rx = None;
                    self.snap_msg = Some(e);
                }
                Err(_) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if let Some(job) = &self.dup_job {
            match job.rx.try_recv() {
                Ok(groups) => {
                    let cancelled = job.progress.cancel.load(Ordering::Relaxed);
                    self.dup_job = None;
                    if !cancelled {
                        self.dup_rows = groups
                            .iter()
                            .enumerate()
                            .flat_map(|(g, grp)| std::iter::once((g, None)).chain((0..grp.files.len()).map(move |f| (g, Some(f)))))
                            .collect();
                        self.dup_set = Arc::new(groups.iter().flat_map(|g| g.files.iter().map(|f| f.tree_path.clone())).collect());
                        self.dup_groups = groups;
                        self.dup_done = true;
                        self.paint_gen += 1;
                    }
                }
                Err(_) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }

        self.custom_shortcut(&ctx);
        self.drive_filter(&ctx);
        self.poll_sequences(&ctx);

        let r = egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(2.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });
        self.help_regions.push((r.response.rect, topics::TOOLBAR));

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
        let r = egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(op) = &self.ops.job {
                    ui.spinner();
                    ui.label(format!("{}…", op.label));
                    ui.separator();
                } else if let Some((text, error)) = self.ops.notice.clone() {
                    if error {
                        ui.colored_label(ui.visuals().error_fg_color, text);
                    } else {
                        ui.label(text);
                    }
                    if ui.small_button("✖").on_hover_text("Dismiss").clicked() {
                        self.ops.notice = None;
                    }
                    ui.separator();
                }
                if let Some(h) = self.hover_status.as_ref().filter(|_| self.job.is_none()) {
                    ui.strong(h);
                } else if let Some(job) = &self.job {
                    ui.spinner();
                    ui.label(format!("Scanning {}…", job.root.display()));
                } else if let Some(t) = &self.tree {
                    ui.label(format!(
                        "{}: {} in {} files, scanned in {:.1}s. Click a tile to select, double-click to open a folder, right-click to zoom out.",
                        self.scanned_path.display(),
                        scan::human(t.size),
                        group(t.files),
                        self.scan_secs
                    ));
                    if self.scan_errors > 0 {
                        let text = if self.scan_errors == 1 {
                            "1 item couldn't be read.".to_string()
                        } else {
                            format!("{} items couldn't be read.", group(self.scan_errors))
                        };
                        ui.colored_label(ui.visuals().warn_fg_color, text).on_hover_text(
                            "Folders or files dirmap wasn't allowed to open (or that vanished during the scan). \
                             Their space isn't counted. Press F1 for why and what to do.",
                        );
                    }
                    if let Some(s) = &self.shown {
                        ui.separator();
                        ui.label(format!("Showing {} matching files ({})", group(s.tree.files), scan::human(s.tree.size)));
                    }
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
        self.help_regions.push((r.response.rect, if self.scan_errors > 0 { topics::UNREADABLE } else { topics::RESULTS }));
        if rescan {
            self.path_input = self.scanned_path.display().to_string();
            self.start_scan();
        }

        if self.job.is_some() {
            let r = egui::CentralPanel::default().show(ui, |ui| self.scan_screen(ui));
            self.help_regions.push((r.response.rect, topics::WHILE_SCANNING));
            return;
        }

        if self.tree.is_none() {
            let r = egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Pick a drive to scan, or type a folder above");
                    ui.add_space(16.0);
                    if ui.link("New to dirmap? Read the Quick Start").clicked() {
                        self.help.show_topic(topics::QUICK_START);
                    }
                });
                ui.add_space(8.0);
                if let Some(root) = self.drive_table(ui) {
                    self.scan_root(root);
                }
            });
            self.help_regions.push((r.response.rect, topics::SCANNING));
            return;
        }

        let r = egui::Panel::right("exts").default_size(330.0).resizable(true).show(ui, |ui| self.side_panel(ui));
        self.help_regions.push((r.response.rect, self.side_topic()));
        let r = egui::Panel::top("tree").default_size(320.0).resizable(true).show(ui, |ui| self.tree_panel(ui));
        self.help_regions.push((r.response.rect, topics::TREE));
        // Its own panel, so it gets the theme's background (readable in light and dark themes).
        let r = egui::Panel::top("filters").show(ui, |ui| {
            ui.add_space(2.0);
            self.filter_bar(ui);
            ui.add_space(2.0);
        });
        let custom_focused = ctx.memory(|m| m.has_focus(egui::Id::new("custom-filter-input")));
        self.help_regions.push((r.response.rect, if custom_focused { topics::SYNTAX } else { topics::FILTERING }));
        let map_topic = match self.map_kind {
            MapKind::Treemap => topics::TREEMAP_NAV,
            MapKind::Sunburst => topics::SUNBURST,
        };
        let r = egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let rect = ui.max_rect();
            match self.map_kind {
                MapKind::Treemap => self.treemap_panel(ui),
                MapKind::Sunburst => self.sunburst_panel(ui),
            }
            if self.shown.as_ref().is_some_and(|s| s.tree.files == 0) {
                let msg = "No files match the filter";
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(16.0), Color32::GRAY);
            }
        });
        self.help_regions.push((r.response.rect, map_topic));
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

/// Mouse wheel over a map: up zooms into `child` (the top-level entry under the pointer, if it's
/// a folder), down zooms back out. Returns the new zoom path.
fn wheel_zoom(wheel: &mut f32, ui: &egui::Ui, hovered: bool, tree: &Node, zoom: &[u32], child: Option<u32>) -> Option<Vec<u32>> {
    if !hovered {
        *wheel = 0.0;
        return None;
    }
    *wheel += ui.input(|i| {
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
    if *wheel >= 40.0 {
        *wheel = 0.0;
        let mut z = zoom.to_vec();
        z.push(child?);
        return tree.at(&z).is_some_and(|n| n.is_dir).then_some(z);
    }
    if *wheel <= -40.0 {
        *wheel = 0.0;
        if !zoom.is_empty() {
            return Some(zoom[..zoom.len() - 1].to_vec());
        }
    }
    None
}

/// Color for "color by age": hot for recent, cool for old, blended between the bands.
fn age_color(modified: i64) -> Color32 {
    let days = ((chrono::Utc::now().timestamp() - modified).max(0) as f32 / 86_400.0).max(0.0);
    let t = |d: f32| (d + 1.0).ln();
    let x = t(days);
    for w in AGE_STOPS.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if x <= t(b.0) {
            let f = ((x - t(a.0)) / (t(b.0) - t(a.0))).clamp(0.0, 1.0);
            let mix = |i: usize| (a.1[i] as f32 + (b.1[i] as f32 - a.1[i] as f32) * f) as u8;
            return Color32::from_rgb(mix(0), mix(1), mix(2));
        }
    }
    let c = AGE_STOPS[AGE_STOPS.len() - 1].1;
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Color for "color by change": red for new, orange for grown, blue for shrunk, grey for the same.
fn change_color(old: u64, new: u64) -> Color32 {
    if old == snapshot::NEW {
        Color32::from_rgb(235, 64, 52)
    } else if new > old {
        // Stronger orange the more it grew, relative to its old size.
        let f = ((new - old) as f32 / old.max(1) as f32).min(1.0) * 0.6 + 0.4;
        let grey = 110.0;
        let mix = |c: f32| (grey + (c - grey) * f) as u8;
        Color32::from_rgb(mix(245.0), mix(150.0), mix(40.0))
    } else if new < old {
        Color32::from_rgb(70, 150, 225)
    } else {
        Color32::from_gray(95)
    }
}

/// One ring segment as triangles, darker on the inside for a little depth, with a hairline gap
/// on every side so neighbours stay distinct.
fn add_arc(mesh: &mut egui::Mesh, c: egui::Pos2, r0: f32, r1: f32, a0: f32, a1: f32, color: Color32) {
    let (r0, r1) = (r0 + 0.5, r1 - 0.5);
    let gap = (0.5 / r0.max(1.0)).min((a1 - a0) * 0.2);
    let (a0, a1) = (a0 + gap, a1 - gap);
    if a1 <= a0 || r1 <= r0 {
        return;
    }
    let inner = color.gamma_multiply(0.78);
    let steps = (((a1 - a0) / 0.03).ceil() as usize).max(1);
    let base = mesh.vertices.len() as u32;
    for k in 0..=steps {
        let a = a0 + (a1 - a0) * k as f32 / steps as f32;
        let dir = vec2(a.sin(), -a.cos());
        mesh.colored_vertex(c + dir * r0, inner);
        mesh.colored_vertex(c + dir * r1, color);
    }
    for k in 0..steps as u32 {
        let i = base + 2 * k;
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
}

/// Closed outline of a ring segment.
fn arc_outline(c: egui::Pos2, r0: f32, r1: f32, a0: f32, a1: f32) -> Vec<egui::Pos2> {
    let steps = (((a1 - a0) / 0.03).ceil() as usize).max(1);
    let at = |k: usize, r: f32| {
        let a = a0 + (a1 - a0) * k as f32 / steps as f32;
        c + vec2(a.sin(), -a.cos()) * r
    };
    let mut pts: Vec<_> = (0..=steps).map(|k| at(k, r1)).collect();
    pts.extend((0..=steps).rev().map(|k| at(k, r0)));
    pts
}

fn lighten(c: Color32, f: f32) -> Color32 {
    let m = |v: u8| (v as f32 + (255.0 - v as f32) * f) as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

/// 0 below `a`, 1 above `b`, smooth in between.
fn mapview_smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn luma(c: Color32) -> f32 {
    0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32
}

/// A size box: drag it, or click and type "500 MB", "2 GB", "100 KB". 0 shows as "off".
fn size_edit(ui: &mut egui::Ui, v: &mut u64) -> bool {
    let mut f = *v as f64;
    let speed = (f * 0.01).max(64.0 * 1024.0);
    let r = ui
        .add(
            egui::DragValue::new(&mut f)
                .range(0.0..=1e15)
                .speed(speed)
                .custom_formatter(|x, _| if x < 1.0 { "off".to_string() } else { scan::human(x as u64) })
                .custom_parser(|t| {
                    let t = t.trim();
                    if t.is_empty() || t.eq_ignore_ascii_case("off") { Some(0.0) } else { scan::parse_size(t).map(|b| b as f64) }
                }),
        )
        .on_hover_text("Drag, or click and type a size such as 500 MB, 2 GB or 100 KB. 0 turns it off.");
    if r.changed() {
        *v = f as u64;
    }
    r.changed()
}

/// "+1.2 GB" / "-300 MB"
fn signed(delta: i64) -> String {
    let sign = if delta < 0 { "−" } else { "+" };
    format!("{sign}{}", scan::human(delta.unsigned_abs()))
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

#[cfg(test)]
mod filter_tests {
    use super::*;
    use crate::pattern::{Filter, Options};

    fn custom(p: &str) -> Option<Arc<Filter>> {
        Some(Arc::new(Filter::compile(p, Options::default()).unwrap()))
    }

    #[test]
    fn categories_still_work_alone() {
        let f = ViewFilter { kinds: Category::Images.bit(), ..Default::default() };
        assert!(f.active());
        assert!(f.matches("a.PNG", 10));
        assert!(!f.matches("a.mp3", 10));
        assert!(!ViewFilter::default().active());
        assert!(ViewFilter::default().matches("anything", 0));
    }

    #[test]
    fn custom_combines_with_categories_and_sizes() {
        let f = ViewFilter { custom: custom("*beauty*"), ..Default::default() };
        assert!(f.active());
        assert!(f.matches("sh010_beauty.1001.exr", 1));
        assert!(!f.matches("sh010_depth.1001.exr", 1));
        // Every part has to agree.
        let f = ViewFilter { custom: custom("*beauty*"), kinds: Category::Audio.bit(), larger: 100, ..Default::default() };
        assert!(!f.matches("sh010_beauty.1001.exr", 1000), "not audio");
        assert!(f.matches("beauty.wav", 1000));
        assert!(!f.matches("beauty.wav", 10), "too small");
        // Equal patterns give equal filters (so results are reused), different ones don't.
        assert!(ViewFilter { custom: custom("*.exr"), ..Default::default() } == ViewFilter { custom: custom("*.exr"), ..Default::default() });
        assert!(ViewFilter { custom: custom("*.exr"), ..Default::default() } != ViewFilter { custom: custom("*.png"), ..Default::default() });
    }
}
