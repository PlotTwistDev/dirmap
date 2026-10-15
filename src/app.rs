//! The desktop GUI: directory tree, extension list and treemap.

use crate::scan::{self, ExtStat, Node, Options, Progress};
use crate::treemap::{self, Tile};
use eframe::egui::{self, ecolor::Hsva, pos2, vec2, Color32, Id, Mesh, Rect, Sense, Stroke};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::Instant;

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

struct LayoutCache {
    bounds: Rect,
    zoom: Vec<u32>,
    tiles: Vec<Tile>,
    mesh: Mesh,
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
        };
        if path.is_some() {
            app.start_scan();
        }
        app
    }

    fn start_scan(&mut self) {
        let path = PathBuf::from(self.path_input.trim());
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
    }

    fn color_of(&self, n: &Node) -> Color32 {
        self.colors.get(&n.ext().to_lowercase()).copied().unwrap_or(Color32::GRAY)
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

    fn select(&mut self, path: Vec<u32>, ctx: &egui::Context) {
        // Expand every ancestor in the tree view so the selection is visible.
        for k in 0..path.len() {
            let id = node_id(&path[..k]);
            let mut st = egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, false);
            st.set_open(true);
            st.store(ctx);
        }
        self.selected = Some(path);
        self.reveal = true;
    }

    // ---------- toolbar ----------

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
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
            ui.selectable_value(&mut self.sort, SortBy::Size, "Size");
            ui.selectable_value(&mut self.sort, SortBy::Name, "Name");
            ui.selectable_value(&mut self.sort, SortBy::Files, "Files");
        });
        ui.separator();
        let Some(tree) = self.tree.take() else { return };
        let mut clicked = None;
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            let mut path = Vec::new();
            self.tree_node(ui, &tree, tree.size, &mut path, &mut clicked);
        });
        self.tree = Some(tree);
        self.reveal = false;
        if let Some(p) = clicked {
            self.selected = Some(p);
        }
    }

    fn tree_node(&self, ui: &mut egui::Ui, n: &Node, parent_size: u64, path: &mut Vec<u32>, clicked: &mut Option<Vec<u32>>) {
        let is_sel = self.selected.as_deref() == Some(path.as_slice());
        let row = |ui: &mut egui::Ui, clicked: &mut Option<Vec<u32>>| {
            let frac = if parent_size > 0 { n.size as f32 / parent_size as f32 } else { 0.0 };
            let (r, _) = ui.allocate_exact_size(vec2(50.0, 12.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, ui.visuals().extreme_bg_color);
            let mut fill = r;
            fill.set_width(r.width() * frac);
            let c = if n.is_dir { Color32::from_rgb(90, 140, 220) } else { self.color_of(n) };
            ui.painter().rect_filled(fill, 2.0, c);
            ui.add_sized([72.0, 16.0], egui::Label::new(scan::human(n.size)));
            ui.add_sized([44.0, 16.0], egui::Label::new(egui::RichText::new(format!("{:.1}%", frac * 100.0)).weak()));
            let label = if n.is_dir {
                format!("{}/  ({} files)", n.name, n.files)
            } else {
                n.name.to_string()
            };
            let resp = ui.selectable_label(is_sel, label);
            if resp.clicked() {
                *clicked = Some(path.clone());
            }
            if is_sel && self.reveal {
                resp.scroll_to_me(Some(egui::Align::Center));
            }
        };

        if !n.is_dir {
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                row(ui, clicked);
            });
            return;
        }
        let id = node_id(path);
        let st = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, path.is_empty());
        st.show_header(ui, |ui| row(ui, clicked)).body(|ui| {
            let mut order: Vec<usize> = (0..n.children.len()).collect();
            match self.sort {
                SortBy::Size => {}
                SortBy::Name => order.sort_by_cached_key(|&i| n.children[i].name.to_lowercase()),
                SortBy::Files => order.sort_by(|&a, &b| n.children[b].files.cmp(&n.children[a].files)),
            }
            const MAX_ROWS: usize = 2000;
            let sel_child = self.selected.as_ref().filter(|s| s.len() > path.len() && s.starts_with(path)).map(|s| s[path.len()] as usize);
            for (k, &i) in order.iter().enumerate() {
                // Past the row cap, still render the child that leads to the selection.
                if k >= MAX_ROWS && Some(i) != sel_child {
                    continue;
                }
                path.push(i as u32);
                self.tree_node(ui, &n.children[i], n.size, path, clicked);
                path.pop();
            }
            if order.len() > MAX_ROWS {
                ui.weak(format!("… {} more items not listed", order.len() - MAX_ROWS));
            }
        });
    }

    // ---------- extension list ----------

    fn ext_panel(&mut self, ui: &mut egui::Ui) {
        let total: u64 = self.tree.as_ref().map(|t| t.size).unwrap_or(0).max(1);
        ui.heading("Extensions");
        if self.highlight_ext.is_some() && ui.small_button("Clear highlight").clicked() {
            self.highlight_ext = None;
            self.cache = None;
        }
        let mut toggle = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("exts").striped(true).num_columns(4).show(ui, |ui| {
                ui.strong("Ext");
                ui.strong("Size");
                ui.strong("%");
                ui.strong("Files");
                ui.end_row();
                for e in &self.exts {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, self.colors[&e.ext]);
                        let name = if e.ext.is_empty() { "(none)".to_string() } else { format!(".{}", e.ext) };
                        let on = self.highlight_ext.as_deref() == Some(e.ext.as_str());
                        if ui.selectable_label(on, name).on_hover_text("Click to highlight in treemap").clicked() {
                            toggle = Some(e.ext.clone());
                        }
                    });
                    ui.label(scan::human(e.bytes));
                    ui.label(format!("{:.1}", e.bytes as f64 * 100.0 / total as f64));
                    ui.label(e.files.to_string());
                    ui.end_row();
                }
            });
        });
        if let Some(t) = toggle {
            self.highlight_ext = if self.highlight_ext.as_ref() == Some(&t) { None } else { Some(t) };
            self.cache = None;
        }
    }

    // ---------- treemap ----------

    fn treemap_panel(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click());
        let bounds = resp.rect;
        let Some(tree) = &self.tree else { return };
        let Some(zroot) = tree.at(&self.zoom) else {
            self.zoom.clear();
            return;
        };

        let stale = match &self.cache {
            Some(c) => c.bounds != bounds || c.zoom != self.zoom,
            None => true,
        };
        if stale {
            let tiles = treemap::layout(zroot, bounds);
            let mesh = self.build_mesh(zroot, &tiles);
            self.cache = Some(LayoutCache { bounds, zoom: self.zoom.clone(), tiles, mesh });
        }
        let cache = self.cache.as_ref().unwrap();
        painter.add(egui::Shape::mesh(cache.mesh.clone()));

        let zl = self.zoom.len();
        // Outline the selection (or the nearest visible ancestor of it).
        if let Some(sel) = self.selected.as_ref().filter(|s| s.starts_with(&self.zoom)) {
            let rel = &sel[zl..];
            if let Some(t) = cache.tiles.iter().filter(|t| rel.starts_with(&t.path)).max_by_key(|t| t.path.len()) {
                painter.rect_stroke(t.rect, 0.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
            }
        }

        let hover = resp.hover_pos().and_then(|p| cache.tiles.iter().rev().find(|t| t.rect.contains(p)));
        if let Some(t) = hover {
            if let Some(parent) = cache.tiles.iter().find(|p| p.is_dir && p.path.len() + 1 == t.path.len() && t.path.starts_with(&p.path)) {
                painter.rect_stroke(parent.rect, 0.0, Stroke::new(1.0, Color32::from_white_alpha(140)), egui::StrokeKind::Inside);
            }
            let mut abs = self.zoom.clone();
            abs.extend_from_slice(&t.path);
            let n = tree.at(&abs).unwrap();
            let full = self.full_path(&abs);
            let txt = format!("{}\n{}", full.display(), scan::human(n.size));
            resp.clone().on_hover_text_at_pointer(txt);
            let (single, double) = (resp.clicked(), resp.double_clicked());
            if double {
                // Zoom into the directory that holds the double-clicked tile.
                let mut z = abs.clone();
                if !n.is_dir {
                    z.pop();
                }
                self.zoom = z;
            } else if single {
                self.select(abs, ui.ctx());
            }
        }
        if resp.secondary_clicked() {
            self.zoom.pop();
        }
    }

    fn build_mesh(&self, zroot: &Node, tiles: &[Tile]) -> Mesh {
        let mut mesh = Mesh::default();
        let hl = self.highlight_ext.as_deref();
        for t in tiles {
            let n = zroot.at(&t.path).unwrap();
            let r = t.rect;
            if n.is_dir {
                // Background for directories whose contents were too small to draw.
                mesh.add_colored_rect(r, Color32::from_gray(45));
                continue;
            }
            let mut c = self.color_of(n);
            if let Some(h) = hl {
                if n.ext().to_lowercase() != h {
                    c = c.gamma_multiply(0.15);
                }
            }
            // Fake "cushion" shading: light top-left, dark bottom-right.
            let light = scale(c, 1.35);
            let dark = scale(c, 0.45);
            let base = mesh.vertices.len() as u32;
            for (p, col) in [(r.left_top(), light), (r.right_top(), c), (r.right_bottom(), dark), (r.left_bottom(), c)] {
                mesh.colored_vertex(pos2(p.x, p.y), col);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        mesh
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
                ui.centered_and_justified(|ui| {
                    ui.heading(if self.job.is_some() { "Scanning…" } else { "No scan yet" });
                });
            });
            return;
        }

        egui::Panel::right("exts").default_size(260.0).resizable(true).show(ui, |ui| self.ext_panel(ui));
        egui::Panel::top("tree").default_size(320.0).resizable(true).show(ui, |ui| self.tree_panel(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.treemap_panel(ui));
    }
}

fn node_id(path: &[u32]) -> Id {
    Id::new(("dirmap-node", path))
}

fn scale(c: Color32, f: f32) -> Color32 {
    let s = |v: u8| ((v as f32 * f).min(255.0)) as u8;
    Color32::from_rgba_premultiplied(s(c.r()), s(c.g()), s(c.b()), c.a())
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
