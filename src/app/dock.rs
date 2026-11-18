//! The analysis dock: the Storage Analysis treemap (or sunburst) under the File Manager's
//! list, showing the folder you're in from the analysis already made, with no rescanning.
//!
//! There is one analysis (`App::tree`) and one map (`App::map`), shared with the Storage
//! Analysis workspace: the dock only decides which folder the map shows (`App::zoom`), keeps
//! the selections of the list and the map in step, and offers Analyse / Refresh when the
//! folder isn't covered or has changed. Folders are matched by their path on disk and turned
//! into tree positions only here, at the boundary.

use crate::fsys::PathShow;
use super::*;
use crate::fsys::history::Location;
use crate::fsys::listing::Entry;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// What the dock's map shows as you browse.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(super) enum Follow {
    /// The folder open in the active tab.
    #[default]
    Folder,
    /// One folder, whatever you open in the list.
    Pinned,
    /// The selected folder, or the open folder with the selected file outlined.
    Selection,
}

impl Follow {
    fn label(self) -> &'static str {
        match self {
            Follow::Folder => "Follow folder",
            Follow::Pinned => "Pinned",
            Follow::Selection => "Follow selection",
        }
    }
}

/// Remembered between runs (part of `FilesPrefs`).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct DockPrefs {
    pub open: bool,
    pub height: f32,
    pub maximised: bool,
    pub follow: Follow,
    /// The folder a Pinned dock shows.
    pub pinned: Option<PathBuf>,
    pub sunburst: bool,
}

impl Default for DockPrefs {
    fn default() -> Self {
        Self { open: false, height: 320.0, maximised: false, follow: Follow::Folder, pinned: None, sunburst: false }
    }
}

/// An analysis replaced by a newer one, kept so that going back to a folder it covers shows
/// the map at once.
pub(super) struct Past {
    pub root: PathBuf,
    pub tree: Arc<Node>,
    pub exts: Vec<ExtStat>,
    pub secs: f32,
    pub at: i64,
    pub with: Option<Settings>,
}

/// Keep at most this many earlier analyses, and only ones up to this size (each costs memory).
const PAST_MAX: usize = 2;
const PAST_MAX_FILES: u64 = 2_000_000;

/// Facts about the folder on the map, worked out on another thread.
#[derive(Clone, Default)]
pub(super) struct Stats {
    pub size: u64,
    pub files: u64,
    pub folders: u64,
    pub largest_file: Option<(String, u64)>,
    pub largest_dir: Option<(String, u64)>,
    /// Bytes per extension, largest first (top few).
    pub types: Vec<(String, u64)>,
}

/// Where the folder the dock wants stands against the analysis.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Cover {
    /// No folder to show (the Computer view, or nothing pinned).
    Nothing,
    /// Covered: its position in the shown tree.
    In(Vec<u32>),
    /// Not part of the analysis (or there is none).
    Outside,
}

/// What the dock's folder was worked out for: (tab, folder, cursor item, selected count, tree,
/// mode, pinned folder).
type FollowKey = (usize, Location, Option<Box<str>>, usize, usize, Follow, Option<PathBuf>);

/// The dock while the program runs.
#[derive(Default)]
pub(super) struct Dock {
    /// The folder the map should show, and how it resolved, for (selection key, tree, follow).
    target: Option<PathBuf>,
    cover: Option<Cover>,
    key: Option<FollowKey>,
    /// The other selected items, as tree paths, outlined on the map.
    pub picked: Vec<Vec<u32>>,
    /// The zoom the dock last set; if the map's differs, the user moved the map.
    set_zoom: Option<Vec<u32>>,
    /// The selection the dock last set; if the map's differs, a tile was clicked.
    set_selected: Option<Option<Vec<u32>>>,
    stats: Option<(usize, Vec<u32>, Stats)>,
    stats_rx: Option<(usize, Vec<u32>, mpsc::Receiver<Stats>)>,
    /// (folder, listing generation, tree) the change check was made for, and its answer.
    stale: Option<(PathBuf, u64, usize, bool)>,
    /// Folders refreshed in place since the analysis was made, and when.
    refreshed: Vec<(PathBuf, i64)>,
    past: Vec<Past>,
    /// Right-clicked tile, for its menu.
    menu_on: Option<Vec<u32>>,
}

impl Dock {
    /// Work everything out again next frame (a new analysis, a new mode).
    pub(super) fn invalidate(&mut self) {
        self.key = None;
        self.stale = None;
    }

    /// Keep an analysis that's being replaced, if it's small enough to hold on to.
    pub(super) fn remember(&mut self, past: Past, replaced_by: &Path) {
        self.past.retain(|p| p.root != past.root && p.root != replaced_by);
        if past.tree.files <= PAST_MAX_FILES && !past.root.as_os_str().is_empty() {
            self.past.insert(0, past);
        }
        while self.past.len() > PAST_MAX {
            let old = self.past.pop();
            std::thread::spawn(move || drop(old));
        }
        self.refreshed.clear();
        self.invalidate();
    }

    /// An earlier analysis that covers `dir`, the most recent first.
    fn past_for(&self, dir: &Path) -> Option<usize> {
        self.past.iter().position(|p| dir.starts_with(&p.root))
    }
}

pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl App {
    /// The folder on disk the dock should show now.
    fn dock_target(&self) -> Option<PathBuf> {
        let here = self.files.here().dir().map(Path::to_path_buf);
        match self.files.prefs.dock.follow {
            Follow::Folder => here,
            Follow::Pinned => self.files.prefs.dock.pinned.clone().or(here),
            Follow::Selection => {
                let dir = here?;
                match self.files.cursor_entry() {
                    Some(e) if e.is_dir => Some(dir.join(&*e.name)),
                    _ => Some(dir),
                }
            }
        }
    }

    /// Each frame in the File Manager: follow the list, pick up results, and notice the map
    /// being moved or clicked.
    pub(super) fn dock_poll(&mut self, ctx: &egui::Context) {
        // Folder facts finished.
        if let Some((tree_id, at, rx)) = &self.dock.stats_rx {
            match rx.try_recv() {
                Ok(st) => {
                    self.dock.stats = Some((*tree_id, at.clone(), st));
                    self.dock.stats_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(60)),
                Err(mpsc::TryRecvError::Disconnected) => self.dock.stats_rx = None,
            }
        }
        if !self.files.prefs.dock.open {
            return;
        }
        let tree_id = self.shown_tree().map(|t| Arc::as_ptr(&t) as usize).unwrap_or(0);
        let (tab, here, cursor, count) = self.files.selection_key();
        let d = &self.files.prefs.dock;
        let key = (tab, here, cursor, count, tree_id, d.follow, d.pinned.clone());
        if self.dock.key.as_ref() == Some(&key) {
            return;
        }
        self.dock.key = Some(key);
        let target = self.dock_target();
        let mut cover = self.cover_of(target.as_deref());
        // Not in this analysis, but in an earlier one: bring that back instead of rescanning.
        if cover == Cover::Outside
            && self.job.is_none()
            && let Some(dir) = &target
            && let Some(i) = self.dock.past_for(dir)
        {
            let p = self.dock.past.remove(i);
            self.adopt_analysis(p.root, p.tree, p.exts, p.secs, p.at, p.with);
            cover = self.cover_of(target.as_deref());
            // A new tree: work the key out again next frame against it.
            self.dock.key = None;
        }
        if let Cover::In(tp) = &cover {
            if self.zoom != *tp {
                self.zoom = tp.clone();
            }
            self.dock.set_zoom = Some(tp.clone());
            self.dock_request_stats(tp.clone());
        }
        self.dock.target = target;
        self.dock.cover = Some(cover);
        // The list's selection, outlined on the map.
        let chosen = self.files.chosen_paths();
        let following_selected_dir = self.files.prefs.dock.follow == Follow::Selection && self.files.cursor_entry().is_some_and(|e| e.is_dir);
        let cursor_path = self.files.cursor_entry().and_then(|e| self.files.here().dir().map(|d| d.join(&*e.name)));
        self.selected = if following_selected_dir { None } else { cursor_path.and_then(|p| self.tree_path_of(&p)) };
        self.dock.set_selected = Some(self.selected.clone());
        self.dock.picked = if chosen.len() > 1 { chosen.iter().take(500).filter_map(|p| self.tree_path_of(p)).collect() } else { Vec::new() };
    }

    fn cover_of(&self, dir: Option<&Path>) -> Cover {
        let Some(dir) = dir else { return Cover::Nothing };
        let Some(tree) = self.shown_tree() else { return Cover::Outside };
        match self.tree_path_of(dir) {
            Some(p) if tree.at(&p).is_some_and(|n| n.is_dir) => Cover::In(p),
            _ => Cover::Outside,
        }
    }

    fn dock_request_stats(&mut self, at: Vec<u32>) {
        let Some(tree) = self.shown_tree() else { return };
        let id = Arc::as_ptr(&tree) as usize;
        let have = |s: &Option<(usize, Vec<u32>, _)>| s.as_ref().is_some_and(|(i, p, _)| *i == id && *p == at);
        if have(&self.dock.stats) || self.dock.stats_rx.as_ref().is_some_and(|(i, p, _)| *i == id && *p == at) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let path = at.clone();
        std::thread::spawn(move || {
            if let Some(n) = tree.at(&path) {
                let _ = tx.send(stats_of(n));
            }
        });
        self.dock.stats_rx = Some((id, at, rx));
    }

    /// Has the folder on screen changed since it was analysed? Compares the listing (already
    /// read for the list, so no extra disk access) with the analysis: items gone, files whose
    /// size changed, anything modified after the analysis was made.
    fn dock_stale(&mut self) -> bool {
        let Some((dir, generation, entries)) = self.files.finished_listing() else { return false };
        let Some(tree) = self.shown_tree() else { return false };
        let id = Arc::as_ptr(&tree) as usize;
        if let Some((d, g, t, v)) = &self.dock.stale
            && *d == dir
            && *g == generation
            && *t == id
        {
            return *v;
        }
        let at = self.analysed_at(&dir);
        let v = match self.tree_path_of(&dir).and_then(|p| tree.at(&p)) {
            Some(n) if self.shown.is_none() => changed_since(&entries, n, at),
            _ => false,
        };
        self.dock.stale = Some((dir, generation, id, v));
        v
    }

    /// When the analysis of `dir` was made: the whole analysis, or a later refresh of a folder
    /// that contains it.
    fn analysed_at(&self, dir: &Path) -> i64 {
        self.dock.refreshed.iter().filter(|(p, _)| dir.starts_with(p)).map(|(_, t)| *t).max().unwrap_or(0).max(self.scanned_at)
    }

    /// A refresh of a folder inside the analysis finished: swap it in.
    pub(super) fn merge_scan(&mut self, root: PathBuf, tree: Node) {
        self.apply_updates(vec![(root.clone(), Some(tree))]);
        self.dock.refreshed.retain(|(p, _)| !p.starts_with(&root));
        self.dock.refreshed.push((root, unix_now()));
        self.dock.invalidate();
    }

    // ---------- drawing ----------

    /// Analyse / Refresh / Cancel and the dock toggle, for the main toolbar (right to left).
    pub(super) fn dock_toolbar_buttons(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let shortcut = ctx.format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::J));
        let open = self.files.prefs.dock.open;
        if style::toggle_button(ui, open, "⊞ Analysis", format!("Show or hide the analysis dock: the treemap of this folder under the list ({shortcut})")).clicked() {
            self.files.prefs.dock.open = !open;
        }
        self.analyse_button(ui, false);
    }

    /// The context-sensitive Analyse button: Analyse This Folder, Refresh Analysis, or Cancel
    /// while a scan runs.
    fn analyse_button(&mut self, ui: &mut egui::Ui, small: bool) {
        let dir = self.files.here().dir().map(Path::to_path_buf);
        let text = |s: &str| if small { egui::RichText::new(s).size(12.0) } else { egui::RichText::new(s) };
        if let Some(job) = &self.job {
            let cancelling = job.progress.cancel.load(Ordering::Relaxed);
            let b = ui.add_enabled(!cancelling, egui::Button::new(text(if cancelling { "Cancelling…" } else { "■ Cancel Scan" })));
            if b.on_hover_text(format!("Stop scanning {}", job.root.display())).clicked() {
                job.progress.cancel.store(true, Ordering::Relaxed);
            }
            return;
        }
        let Some(dir) = dir else {
            ui.add_enabled(false, egui::Button::new(text("⚡ Analyse"))).on_disabled_hover_text("Open a folder or drive to analyse it");
            return;
        };
        let inside = self.tree.is_some() && dir.starts_with(&self.scanned_path);
        let (label, tip) = if inside {
            ("⟳ Refresh Analysis", format!("Scan {} again and update it in the current analysis", dir.display()))
        } else {
            ("⚡ Analyse Folder", format!("Scan {} and show where its space goes, while you keep browsing", dir.display()))
        };
        if ui.add(egui::Button::new(text(label))).on_hover_text(tip).clicked() {
            self.analyse(dir);
        }
    }

    /// The dock: its toolbar, a line of facts, the map, and the file-type legend.
    pub(super) fn dock_ui(&mut self, ui: &mut egui::Ui) {
        let pal = style::pal(ui);
        let rect = ui.max_rect();
        // A click in the dock takes the keys from the list (so Home, + and - reach the map).
        if ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))) {
            self.files.focused = false;
        }
        ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
        style::roomy(ui);
        let bar = egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 6, top: 5, bottom: 3 });
        bar.show(ui, |ui| self.dock_bar(ui));
        let cover = self.dock.cover.clone().unwrap_or(Cover::Nothing);
        let scanning_here = self.job.as_ref().is_some_and(|j| self.dock.target.as_ref().is_some_and(|t| t.starts_with(&j.root) || j.root.starts_with(t)));
        egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 4 }).show(ui, |ui| {
            self.dock_strip(ui, &cover, scanning_here);
            if matches!(cover, Cover::In(_)) && !scanning_here {
                self.dock_filters(ui);
            }
        });
        let body = ui.available_rect_before_wrap();
        let legend_h = if matches!(cover, Cover::In(_)) && !scanning_here && self.color_mode == ColorMode::Type { 24.0 } else { 0.0 };
        let map_rect = Rect::from_min_max(body.min + vec2(8.0, 0.0), body.max - vec2(8.0, 6.0 + legend_h));
        let mut map_ui = ui.new_child(egui::UiBuilder::new().max_rect(map_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        if scanning_here {
            let job = self.job.as_ref().unwrap();
            egui::Frame::new().fill(pal.stripe).corner_radius(6).inner_margin(12).show(&mut map_ui, |ui| {
                ui.set_min_size(ui.available_size());
                scan_progress(ui, job, false);
            });
        } else {
            match &cover {
                Cover::Nothing => {
                    let msg = if matches!(self.files.here(), Location::Computer) { "Open a drive or folder to see where its space goes" } else { "Nothing to show" };
                    dock_message(&mut map_ui, "⊞", msg, None);
                }
                Cover::Outside => {
                    let target = self.dock.target.clone();
                    let detail = match (&self.tree, &target) {
                        (Some(_), Some(t)) => Some(format!("{} isn't part of the current analysis ({}).", t.display(), self.scanned_path.display())),
                        (None, Some(t)) => Some(format!("{} hasn't been analysed yet.", t.display())),
                        _ => None,
                    };
                    dock_message(&mut map_ui, "⊞", "Not analysed", detail.as_deref());
                    {
                        let r = map_ui.vertical_centered(|ui| ui.add(egui::Button::new("⚡ Analyse This Folder").min_size(vec2(170.0, 28.0)))).inner;
                        if r.on_hover_text("Scan it and show its treemap here. You can keep browsing while it runs.").clicked()
                            && let Some(t) = target
                        {
                            self.analyse(t);
                        }
                    }
                }
                Cover::In(_) => self.dock_map(&mut map_ui),
            }
        }
        if legend_h > 0.0 {
            let lr = Rect::from_min_max(pos2(body.left() + 10.0, map_rect.bottom() + 2.0), pos2(body.right() - 10.0, body.bottom()));
            let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(lr).layout(egui::Layout::left_to_right(egui::Align::Center)));
            self.dock_legend(&mut lui);
        }
        ui.allocate_rect(body, Sense::hover());
    }

    fn dock_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let covered = matches!(self.dock.cover, Some(Cover::In(_)));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            style::tool_group(ui, |ui| {
                let sb = self.files.prefs.dock.sunburst;
                if ui.add(egui::Button::selectable(!sb, "⊞ Treemap")).on_hover_text("Rectangles sized by bytes").clicked() {
                    self.files.prefs.dock.sunburst = false;
                }
                if ui.add(egui::Button::selectable(sb, "◔ Sunburst")).on_hover_text("Rings: each folder around its parent").clicked() {
                    self.files.prefs.dock.sunburst = true;
                }
            });
            let follow = self.files.prefs.dock.follow;
            let mut pick = None;
            ui.menu_button(format!("{} ⏷", follow.label()), |ui| {
                let items = [
                    (Follow::Folder, "Show the folder open in the list (and in the active tab)"),
                    (Follow::Pinned, "Keep showing one folder while you browse elsewhere"),
                    (Follow::Selection, "Show the folder selected in the list; a selected file is outlined"),
                ];
                for (f, tip) in items {
                    if ui.radio(follow == f, f.label()).on_hover_text(tip).clicked() {
                        pick = Some(f);
                    }
                }
            })
            .response
            .on_hover_text("What the map shows as you browse");
            if let Some(f) = pick {
                if f == Follow::Pinned {
                    // Pin what's on the map now.
                    self.files.prefs.dock.pinned = self.dock.target.clone().or_else(|| self.files.here().dir().map(Path::to_path_buf));
                }
                self.files.prefs.dock.follow = f;
                self.dock.invalidate();
            }
            if follow == Follow::Pinned
                && let Some(p) = &self.files.prefs.dock.pinned
            {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.shown());
                ui.label(egui::RichText::new(format!("📌 {name}")).color(style::pal(ui).weak)).on_hover_text(p.shown());
            }
            let has_diff = self.diff.is_some();
            let mode = self.color_mode;
            let label = match mode {
                ColorMode::Type => "Colour: Type",
                ColorMode::Age => "Colour: Age",
                ColorMode::Change => "Colour: Change",
            };
            ui.menu_button(format!("{label} ⏷"), |ui| {
                ui.radio_value(&mut self.color_mode, ColorMode::Type, "File type");
                ui.radio_value(&mut self.color_mode, ColorMode::Age, "Age (last modified)");
                ui.add_enabled_ui(has_diff, |ui| {
                    ui.radio_value(&mut self.color_mode, ColorMode::Change, "Change since snapshot");
                })
                .response
                .on_disabled_hover_text("Load a snapshot in Storage Analysis (Changes tab) to compare");
            });
            let feel = self.feel();
            let treemap = !self.files.prefs.dock.sunburst;
            ui.add_enabled_ui(covered, |ui| {
                style::tool_group(ui, |ui| {
                    let sel = self.selected.clone();
                    if style::icon_button(ui, sel.is_some() && treemap, "⌖", "Zoom to the selection").clicked()
                        && let (Some(sel), Some(tree)) = (sel, self.shown_tree())
                    {
                        self.map.frame_path(&tree, &sel, &feel);
                    }
                    if style::icon_button(ui, treemap, "⟲", "Whole folder: back to the full view (Home over the map)").clicked() {
                        self.map.reset(&feel);
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::icon_button(ui, true, "✖", format!("Hide the dock ({})", ctx.format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::J)))).clicked() {
                    self.files.prefs.dock.open = false;
                }
                let max = self.files.prefs.dock.maximised;
                if style::icon_button(ui, true, if max { "🗗" } else { "🗖" }, if max { "Restore the dock's size" } else { "Make the dock as tall as possible" }).clicked() {
                    self.files.prefs.dock.maximised = !max;
                }
                if style::icon_button(ui, true, "⛶", "Open in Storage Analysis: the full workspace with the folder tree, types, duplicates… (Ctrl+3)").clicked() {
                    self.set_workspace(Workspace::Analysis);
                }
                ui.add_space(4.0);
                self.analyse_button(ui, true);
            });
        });
    }

    /// One line: what state the analysis is in for this folder, and facts about it.
    fn dock_strip(&mut self, ui: &mut egui::Ui, cover: &Cover, scanning_here: bool) {
        let pal = style::pal(ui);
        let stale = matches!(cover, Cover::In(_)) && self.files.prefs.dock.follow != Follow::Pinned && self.dock_stale();
        let partial = self.scanned_with.as_ref().is_some_and(|w| w.min_file_size > 0 || w.max_age_days > 0);
        let (dot, state) = if scanning_here {
            (pal.accent, "Scanning".to_string())
        } else {
            match cover {
                Cover::Nothing => (pal.weak, "No folder".to_string()),
                Cover::Outside => (pal.weak, "Not analysed".to_string()),
                Cover::In(_) if stale => (Color32::from_rgb(214, 160, 60), "Changed since analysis".to_string()),
                Cover::In(_) if partial => (Color32::from_rgb(110, 170, 110), "Analysed with filters".to_string()),
                Cover::In(_) => (Color32::from_rgb(90, 180, 110), "Analysed".to_string()),
            }
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, dot);
            let mut tip = String::new();
            if stale {
                tip = "Items here were added, removed or changed after the analysis. Refresh Analysis scans this folder again.".into();
            } else if partial {
                tip = "The scan skipped small or old files (Settings > Filtering), so sizes may be lower than on disk.".into();
            }
            let l = ui.label(egui::RichText::new(state).size(12.0).strong().color(pal.text));
            if !tip.is_empty() {
                l.on_hover_text(tip);
            }
            let Cover::In(at) = cover else { return };
            if scanning_here {
                return;
            }
            let id = self.shown_tree().map(|t| Arc::as_ptr(&t) as usize).unwrap_or(0);
            let small = |s: String| egui::RichText::new(s).size(12.0).color(pal.weak);
            let sep = |ui: &mut egui::Ui| ui.label(small("·".into()));
            if let Some((_, _, st)) = self.dock.stats.as_ref().filter(|(i, p, _)| *i == id && p == at) {
                sep(ui);
                ui.label(egui::RichText::new(scan::human(st.size)).size(12.0).color(pal.text));
                sep(ui);
                ui.label(small(format!("{} files · {} folders", group(st.files), group(st.folders))));
                if let Some((name, size)) = &st.largest_dir {
                    sep(ui);
                    ui.label(small(format!("largest folder {name} ({})", scan::human(*size)))).on_hover_text("The biggest folder directly inside this one");
                }
                if let Some((name, size)) = &st.largest_file {
                    sep(ui);
                    ui.add(egui::Label::new(small(format!("largest file {name} ({})", scan::human(*size)))).truncate()).on_hover_text("The biggest file anywhere inside this folder");
                }
            } else {
                sep(ui);
                ui.spinner();
            }
            let at_time = self.dock.target.as_deref().map(|t| self.analysed_at(t)).unwrap_or(self.scanned_at);
            if at_time > 0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(small(format!("analysed {}", fmt_time(at_time))));
                });
            }
        });
    }

    /// The map's filters, which are shared with Storage Analysis: what they are, and a way to
    /// clear them here, so a map that's mostly dimmed or empty says why.
    fn dock_filters(&mut self, ui: &mut egui::Ui) {
        let mut parts = Vec::new();
        if self.filter.kinds != 0 {
            parts.push(Category::ALL.iter().filter(|c| self.filter.kinds & c.bit() != 0).map(|c| c.label()).collect::<Vec<_>>().join(", "));
        }
        if let Some(f) = &self.filter.custom {
            parts.push(format!("Custom: {}", f.source().trim()));
        }
        if self.filter.larger > 0 {
            parts.push(format!("larger than {}", scan::human(self.filter.larger)));
        }
        if self.filter.smaller > 0 {
            parts.push(format!("smaller than {}", scan::human(self.filter.smaller)));
        }
        if self.dupes_only {
            parts.push("duplicates only".into());
        }
        if parts.is_empty() {
            return;
        }
        let only = self.filter.active() && self.custom.prefs.mode == custom::FilterMode::Only;
        let pal = style::pal(ui);
        let warn = ui.visuals().warn_fg_color;
        ui.add_space(3.0);
        egui::Frame::new()
            .fill(style::blend(pal.paper, warn, 0.12))
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(egui::RichText::new("Filtered").size(12.0).strong().color(warn));
                    let what = parts.join(" · ");
                    let how = if only { "the map is built from matching files only" } else { "other files are blanked out, the layout is kept" };
                    ui.add(egui::Label::new(egui::RichText::new(format!("{what}: {how}")).size(12.0).color(pal.text)).truncate())
                        .on_hover_text("This filter is set in Storage Analysis and applies to both maps.");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Clear Filter").on_hover_text("Show every file on the maps again").clicked() {
                            self.filter = ViewFilter::default();
                            self.clear_custom();
                            self.dupes_only = false;
                        }
                        if ui.small_button("Change…").on_hover_text("Open Storage Analysis, where the filter is set (Ctrl+3)").clicked() {
                            self.set_workspace(Workspace::Analysis);
                        }
                    });
                });
            });
    }

    /// The map itself, in the shared renderer, plus what clicking it does to the list.
    fn dock_map(&mut self, ui: &mut egui::Ui) {
        let before_zoom = self.zoom.clone();
        self.map_in_dock = true;
        let kind = self.map_kind;
        let resp = if self.files.prefs.dock.sunburst {
            self.map_kind = MapKind::Sunburst;
            self.sunburst_panel(ui);
            None
        } else {
            self.map_kind = MapKind::Treemap;
            self.treemap_panel(ui)
        };
        self.map_in_dock = false;
        self.map_kind = kind;

        // A tile clicked: select it in the list (or the item in this folder that holds it).
        if self.dock.set_selected.as_ref() != Some(&self.selected) {
            self.dock.set_selected = Some(self.selected.clone());
            if let (Some(sel), Some(dir)) = (self.selected.clone(), self.files.here().dir().map(Path::to_path_buf)) {
                let disk = self.full_path(&sel);
                if let Ok(rel) = disk.strip_prefix(&dir)
                    && let Some(first) = rel.components().next()
                {
                    self.files.select_name(&first.as_os_str().to_string_lossy());
                    // Don't let the list's new selection replace the exact tile.
                    self.dock.key = None;
                    let keep = self.selected.clone();
                    self.dock_poll(ui.ctx());
                    self.selected = keep.clone();
                    self.dock.set_selected = Some(keep);
                }
            }
        }
        // The map moved to another folder (double-click, Back, zoom out): the list follows,
        // or a pinned dock moves its pin.
        if self.zoom != before_zoom || self.dock.set_zoom.as_ref().is_some_and(|z| *z != self.zoom) {
            let disk = self.full_path(&self.zoom);
            self.dock.set_zoom = Some(self.zoom.clone());
            match self.files.prefs.dock.follow {
                Follow::Pinned => self.files.prefs.dock.pinned = Some(disk),
                _ => {
                    if self.files.here().dir() != Some(disk.as_path()) {
                        self.files.go(Location::Dir(disk));
                    }
                }
            }
        }
        // Right-click a tile: the same menu as the list.
        let Some(resp) = resp else { return };
        if resp.secondary_clicked() {
            self.dock.menu_on = self.map_hover.clone();
        }
        let target = self.dock.menu_on.clone();
        let Some(target) = target else { return };
        let disk = self.full_path(&target);
        let is_dir = self.shown_tree().and_then(|t| t.at(&target).map(|n| n.is_dir)).unwrap_or(false);
        resp.context_menu(|ui| {
            ui.label(egui::RichText::new(disk.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).strong());
            ui.separator();
            if ui.button(if is_dir { "Open in List" } else { "Show in List" }).clicked() {
                let (dir, name): (PathBuf, Option<Box<str>>) =
                    if is_dir { (disk.clone(), None) } else { (disk.parent().map(Path::to_path_buf).unwrap_or_default(), disk.file_name().map(|n| n.to_string_lossy().into())) };
                self.files.reveal(dir, name.into_iter().collect());
                ui.close();
            }
            self.tile_menu(ui, &disk, is_dir);
        });
    }

    /// The file-type legend: the types taking the most space here, in their map colours.
    /// Click one to highlight it on the map.
    fn dock_legend(&mut self, ui: &mut egui::Ui) {
        let pal = style::pal(ui);
        let id = self.shown_tree().map(|t| Arc::as_ptr(&t) as usize).unwrap_or(0);
        let Some(Cover::In(at)) = self.dock.cover.clone() else { return };
        let Some((_, _, st)) = self.dock.stats.clone().filter(|(i, p, _)| *i == id && *p == at) else { return };
        ui.spacing_mut().item_spacing.x = 2.0;
        for (ext, bytes) in st.types.iter().take(10) {
            let on = self.highlight_ext.as_deref() == Some(ext.as_str());
            let color = self.color_for_ext(ext);
            let label = if ext.is_empty() { "(none)".to_string() } else { ext.clone() };
            let text = format!("{label}  {}", scan::human(*bytes));
            let font = FontId::proportional(11.5);
            let w = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), pal.text).size().x) + 26.0;
            if ui.available_width() < w {
                break;
            }
            let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
            if on {
                ui.painter().rect_filled(r, 4.0, pal.sel);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 4.0, pal.hover);
            }
            ui.painter().rect_filled(Rect::from_center_size(pos2(r.left() + 10.0, r.center().y), vec2(9.0, 9.0)), 2.0, color);
            ui.painter().text(pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, text, font, if on { pal.strong } else { pal.text });
            if resp.on_hover_text(if on { "Click to stop highlighting" } else { "Click to highlight these files on the map" }).clicked() {
                self.highlight_ext = if on { None } else { Some(ext.clone()) };
            }
        }
        if self.highlight_ext.is_some() && ui.add(egui::Button::new(egui::RichText::new("Clear").size(11.5)).frame_when_inactive(false)).clicked() {
            self.highlight_ext = None;
        }
    }

    /// The status bar's activity indicator: background jobs, with details on click.
    pub(super) fn activity_chip(&mut self, ui: &mut egui::Ui) {
        let mut items: Vec<String> = Vec::new();
        if let Some(job) = &self.job {
            let files = job.progress.files.load(Ordering::Relaxed);
            let what = if job.merge { "Refreshing" } else { "Scanning" };
            items.push(format!("{what} {} · {} files", job.root.display(), group(files)));
        }
        if self.dup_job.is_some() {
            items.push("Finding duplicates".into());
        }
        if self.filter_job.is_some() {
            items.push("Applying the filter".into());
        }
        // A copy or move has its own progress beside this.
        if let Some(op) = self.ops.job.as_ref().filter(|o| o.ctl.is_none()) {
            items.push(op.label.clone());
        }
        if items.is_empty() {
            return;
        }
        let summary = if items.len() == 1 { items[0].clone() } else { format!("{} and {} more", items[0], items.len() - 1) };
        let short: String = summary.chars().take(60).collect();
        let resp = ui.menu_button(format!("⟳ {short}{}", if summary.chars().count() > 60 { "…" } else { "" }), |ui| {
            ui.set_min_width(420.0);
            if let Some(job) = &self.job {
                scan_progress(ui, job, true);
                ui.separator();
            }
            for it in items.iter().skip(usize::from(self.job.is_some())) {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(it);
                });
            }
            if self.job.is_some() && ui.button("Show in Storage Analysis").clicked() {
                self.set_workspace(Workspace::Analysis);
                ui.close();
            }
        });
        resp.response.on_hover_text("Running in the background; browsing isn't affected. Click for details.");
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
}

/// A centred message in the dock: a faint symbol, a title and an explanation (the caller may
/// add a button under it).
fn dock_message(ui: &mut egui::Ui, icon: &str, title: &str, detail: Option<&str>) {
    let pal = style::pal(ui);
    let h = ui.available_height();
    ui.vertical_centered(|ui| {
        ui.add_space((h * 0.5 - 70.0).max(4.0));
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(egui::RichText::new(icon).size(34.0).color(style::blend(pal.paper, pal.weak, 0.45)));
        ui.label(egui::RichText::new(title).size(15.0).color(pal.text));
        if let Some(d) = detail {
            ui.label(egui::RichText::new(d).size(12.0).color(pal.weak));
        }
        ui.add_space(4.0);
    });
}

/// Totals, the largest file and subfolder, and the biggest file types below `n`.
pub(super) fn stats_of(n: &Node) -> Stats {
    let mut st = Stats { size: n.size, files: n.files, ..Default::default() };
    st.largest_dir = n.children.iter().filter(|c| c.is_dir).max_by_key(|c| c.size).map(|c| (c.name.to_string(), c.size));
    let mut types: HashMap<String, u64> = HashMap::new();
    let mut stack: Vec<&Node> = vec![n];
    let mut best: Option<&Node> = None;
    while let Some(x) = stack.pop() {
        for c in &x.children {
            if c.is_dir {
                st.folders += 1;
                stack.push(c);
            } else {
                if best.is_none_or(|b| c.size > b.size) {
                    best = Some(c);
                }
                let e = c.ext();
                match types.get_mut(e) {
                    Some(v) => *v += c.size,
                    None => {
                        types.insert(e.to_string(), c.size);
                    }
                }
            }
        }
    }
    st.largest_file = best.map(|b| (b.name.to_string(), b.size));
    let mut t: Vec<(String, u64)> = types.into_iter().filter(|(_, b)| *b > 0).collect();
    t.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    t.truncate(12);
    st.types = t;
    st
}

/// The folder as listed now differs from the analysis: something analysed is gone, a file's
/// size changed, or something was modified after the analysis (`at`, seconds) was made.
/// Items the scan skipped on purpose (filters) don't count as changes.
pub(super) fn changed_since(entries: &[Entry], n: &Node, at: i64) -> bool {
    let listed: HashMap<&str, &Entry> = entries.iter().map(|e| (&*e.name, e)).collect();
    for c in &n.children {
        match listed.get(&*c.name) {
            None => return true,
            Some(e) if !c.is_dir && !e.is_dir && e.size != c.size => return true,
            _ => {}
        }
    }
    // A couple of seconds' slack for file systems with coarse timestamps.
    at > 0 && entries.iter().any(|e| e.modified > at + 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> Node {
        Node { name: name.into(), size, files: 1, modified: 100, is_dir: false, children: vec![] }
    }

    fn dir(name: &str, children: Vec<Node>) -> Node {
        let size = children.iter().map(|c| c.size).sum();
        let files = children.iter().map(|c| c.files).sum();
        Node { name: name.into(), size, files, modified: 100, is_dir: true, children }
    }

    fn entry(name: &str, size: u64, is_dir: bool, modified: i64) -> Entry {
        Entry { name: name.into(), is_dir, is_link: false, hidden: false, size, modified }
    }

    #[test]
    fn stats_find_totals_largest_and_types() {
        let t = dir("root", vec![dir("a", vec![file("big.mp4", 900), file("x.txt", 5)]), dir("b", vec![dir("c", vec![file("y.MP4", 50)])]), file("z.txt", 10)]);
        let st = stats_of(&t);
        assert_eq!(st.size, 965);
        assert_eq!(st.files, 4);
        assert_eq!(st.folders, 3);
        assert_eq!(st.largest_file, Some(("big.mp4".into(), 900)));
        assert_eq!(st.largest_dir, Some(("a".into(), 905)));
        assert_eq!(st.types[0].0, "mp4");
        assert!(st.types.iter().any(|(e, b)| e == "txt" && *b == 15));
    }

    #[test]
    fn unchanged_folder_is_not_stale() {
        let t = dir("root", vec![file("a.txt", 5), dir("sub", vec![file("b", 1)])]);
        let entries = vec![entry("a.txt", 5, false, 90), entry("sub", 0, true, 90), entry("new-but-filtered.tmp", 0, false, 50)];
        assert!(!changed_since(&entries, &t, 100));
    }

    #[test]
    fn removed_resized_or_newer_items_make_it_stale() {
        let t = dir("root", vec![file("a.txt", 5), file("gone.txt", 1)]);
        assert!(changed_since(&[entry("a.txt", 5, false, 90)], &t, 100), "an item was removed");
        let t = dir("root", vec![file("a.txt", 5)]);
        assert!(changed_since(&[entry("a.txt", 6, false, 90)], &t, 100), "a size changed");
        assert!(changed_since(&[entry("a.txt", 5, false, 90), entry("new.txt", 1, false, 500)], &t, 100), "something newer appeared");
        assert!(!changed_since(&[entry("a.txt", 5, false, 101)], &t, 100), "within the timestamp slack");
    }

    #[test]
    fn earlier_analyses_are_kept_and_found() {
        let mut d = Dock::default();
        let past = |root: &str, files: u64| Past { root: root.into(), tree: Arc::new(Node { files, ..dir("r", vec![]) }), exts: vec![], secs: 0.0, at: 0, with: None };
        d.remember(past("/a", 10), Path::new("/b"));
        d.remember(past("/c", 10), Path::new("/d"));
        d.remember(past("/e", 10), Path::new("/f"));
        assert_eq!(d.past.len(), PAST_MAX);
        assert_eq!(d.past_for(Path::new("/e/x")), Some(0));
        assert_eq!(d.past_for(Path::new("/a/x")), None, "the oldest was dropped");
        d.remember(past("/huge", PAST_MAX_FILES + 1), Path::new("/g"));
        assert!(d.past_for(Path::new("/huge")).is_none(), "too big to keep");
        // Re-analysing the same root replaces it rather than keeping a stale copy.
        d.remember(past("/x", 1), Path::new("/e"));
        assert!(d.past_for(Path::new("/e/y")).is_none());
    }
}
