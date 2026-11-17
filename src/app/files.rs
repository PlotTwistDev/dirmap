//! The File Manager workspace: browse the computer like Explorer, Finder or Dolphin, straight
//! from the disk, with no scan needed. A sidebar of places and drives, tabs with their own
//! history, a path bar you can click or type in, and a sortable Details list.
//!
//! Folders are read by `fsys::listing` on worker threads, so nothing here waits on the disk or
//! the network. File commands use the same `actions` functions as the Storage Analysis
//! workspace; when a scan covers the folder on screen, its folder sizes are shown too.

use super::*;
use crate::actions;
use crate::fsys::history::{History, Location};
use crate::fsys::listing::{Entry, Listing};
use crate::fsys::places::{self, Place};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering as Cmp;
use std::collections::BTreeSet;
use std::path::Path;

/// The three ways to use the window. Explorer + Analysis comes in Phase 3 of the plan.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(super) enum Workspace {
    /// Browse and manage files (the default).
    #[default]
    Files,
    /// The scan, folder tree, treemap and analysis tabs.
    Analysis,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(super) enum FSort {
    #[default]
    Name,
    Size,
    Type,
    Modified,
}

impl FSort {
    fn title(self) -> &'static str {
        match self {
            FSort::Name => "Name",
            FSort::Size => "Size",
            FSort::Type => "Type",
            FSort::Modified => "Date modified",
        }
    }

    /// Names and types A→Z; sizes and dates largest or newest first.
    fn natural_desc(self) -> bool {
        matches!(self, FSort::Size | FSort::Modified)
    }
}

/// What's remembered between runs.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct FilesPrefs {
    pub workspace: Workspace,
    pub tabs: Vec<Location>,
    pub active: usize,
    /// Folders pinned to the sidebar, in the user's order.
    pub pinned: Vec<PathBuf>,
    /// Folders opened most recently, newest first.
    pub recent: Vec<PathBuf>,
    pub sort: FSort,
    /// Opposite of the sort's natural direction.
    pub reversed: bool,
    pub show_hidden: bool,
    pub sidebar_width: f32,
    pub density: Density,
    /// Widths of the Size, Type and Date modified columns (Name takes the rest).
    pub col_widths: [f32; 3],
    /// The analysis dock under the list.
    pub dock: super::dock::DockPrefs,
}

/// Row height in the list and sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(super) enum Density {
    Compact,
    #[default]
    Comfortable,
}

impl Density {
    pub(super) fn row(self) -> f32 {
        match self {
            Density::Compact => 22.0,
            Density::Comfortable => 26.0,
        }
    }
}

/// Default widths of the Size, Type and Date modified columns.
const COL_WIDTHS: [f32; 3] = [92.0, 150.0, 150.0];
const COL_MIN: f32 = 56.0;

impl Default for FilesPrefs {
    fn default() -> Self {
        Self {
            workspace: Workspace::Files,
            tabs: Vec::new(),
            active: 0,
            pinned: Vec::new(),
            recent: Vec::new(),
            sort: FSort::Name,
            reversed: false,
            show_hidden: false,
            sidebar_width: 210.0,
            density: Density::Comfortable,
            col_widths: COL_WIDTHS,
            dock: Default::default(),
        }
    }
}

const RECENT_MAX: usize = 10;
/// Sidebar rows (the list's rows follow the density setting).
const ROW_F: f32 = 24.0;
const TAB_STRIP_H: f32 = 34.0;
const TAB_MIN_W: f32 = 110.0;
const TAB_MAX_W: f32 = 210.0;
/// The analysis dock and the list above it never get smaller than these.
pub(super) const DOCK_MIN: f32 = 140.0;
pub(super) const DOCK_MIN_LIST: f32 = 110.0;

/// Folder name → (bytes, files) from the scan.
type FolderSizes = HashMap<Box<str>, (u64, u64)>;

/// What an order was worked out for: (listing generation, sort, reversed, hidden, filter, sizes).
type OrderKey = (u64, FSort, bool, bool, String, usize);

/// Folders with more items than this are sorted on another thread, so the window never stalls.
const SORT_INLINE_MAX: usize = 3_000;

/// One tab: where it is, where it's been, what's in it and what's selected there.
pub(super) struct FTab {
    hist: History,
    listing: Option<Listing>,
    /// Indices into the listing's entries, in display order.
    order: Vec<u32>,
    /// What `order` was worked out for: (listing generation, sort, reversed, hidden, filter, sizes).
    order_key: Option<OrderKey>,
    /// A big folder being sorted on another thread: what for, how many entries it covers, and
    /// where the result arrives.
    sorting: Option<(OrderKey, usize, mpsc::Receiver<Vec<u32>>)>,
    selected: BTreeSet<Box<str>>,
    /// The item the keyboard moves from (the last one clicked or moved to).
    cursor: Option<Box<str>>,
    /// Where Shift-selection ranges start.
    anchor: Option<Box<str>>,
    /// Select this item once it's listed (coming up from a folder, or from another workspace).
    select_later: Option<Box<str>>,
    scroll_to_cursor: bool,
    /// (scroll offset, height) of the list last frame.
    view: (f32, f32),
    /// Folder sizes from the scan for this folder: (tree it came from, name → (bytes, files)).
    sizes: Option<(usize, FolderSizes)>,
}

impl FTab {
    fn new(at: Location) -> Self {
        Self {
            hist: History::new(at),
            listing: None,
            order: Vec::new(),
            order_key: None,
            sorting: None,
            selected: BTreeSet::new(),
            cursor: None,
            anchor: None,
            select_later: None,
            scroll_to_cursor: false,
            view: (0.0, 0.0),
            sizes: None,
        }
    }

    fn here(&self) -> &Location {
        &self.hist.here
    }

    /// Forget what was shown, after moving somewhere else.
    fn arrived(&mut self) {
        self.listing = None;
        self.order.clear();
        self.order_key = None;
        self.sorting = None;
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.sizes = None;
        self.scroll_to_cursor = true;
        self.view = (0.0, 0.0);
    }

    fn entry(&self, i: u32) -> Option<&Entry> {
        self.listing.as_ref()?.entries.get(i as usize)
    }

    fn pos_of(&self, name: &str) -> Option<usize> {
        let l = self.listing.as_ref()?;
        self.order.iter().position(|&i| &*l.entries[i as usize].name == name)
    }
}

/// The File Manager's state.
pub(super) struct Files {
    pub prefs: FilesPrefs,
    tabs: Vec<FTab>,
    active: usize,
    /// Recently closed tabs, to reopen with Ctrl+Shift+T.
    closed: Vec<Location>,
    places: Vec<Place>,
    places_rx: Option<mpsc::Receiver<Vec<Place>>>,
    /// The path being typed in the path bar (it shows breadcrumbs otherwise).
    path_edit: Option<String>,
    focus_path: bool,
    /// Show only items whose names contain this (or match it, with * or ?).
    filter: String,
    focus_filter: bool,
    typed: String,
    typed_at: f64,
    /// Keys go to the list (it or the sidebar was clicked last, not the filter box).
    pub(super) focused: bool,
    /// Results of commands run in the background (open, reveal, terminal).
    results: Vec<mpsc::Receiver<Result<(), String>>>,
}

impl Files {
    pub fn new(prefs: FilesPrefs) -> Self {
        let mut tabs: Vec<FTab> = prefs.tabs.iter().cloned().map(FTab::new).collect();
        if tabs.is_empty() {
            tabs.push(FTab::new(places::home().map(Location::Dir).unwrap_or(Location::Computer)));
        }
        let active = prefs.active.min(tabs.len() - 1);
        // The standard places are in the user's profile, but a profile folder can be
        // redirected to a server, so find them off the UI thread too.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(places::standard());
        });
        Self {
            prefs,
            tabs,
            active,
            closed: Vec::new(),
            places: Vec::new(),
            places_rx: Some(rx),
            path_edit: None,
            focus_path: false,
            filter: String::new(),
            focus_filter: false,
            typed: String::new(),
            typed_at: 0.0,
            focused: true,
            results: Vec::new(),
        }
    }

    /// The state to save: the open tabs and everything in `prefs`.
    pub fn to_save(&self) -> FilesPrefs {
        let mut p = self.prefs.clone();
        p.tabs = self.tabs.iter().map(|t| t.here().clone()).collect();
        p.active = self.active;
        p
    }

    fn tab(&mut self) -> &mut FTab {
        &mut self.tabs[self.active]
    }

    pub fn here(&self) -> &Location {
        self.tabs[self.active].here()
    }

    /// Go somewhere in the current tab, remembering where we were.
    pub fn go(&mut self, to: Location) {
        if *self.here() == to {
            return;
        }
        self.tab().hist.go(to);
        self.after_move();
    }

    fn after_move(&mut self) {
        self.tab().arrived();
        self.filter.clear();
        self.path_edit = None;
        if let Location::Dir(p) = self.here().clone() {
            let r = &mut self.prefs.recent;
            r.retain(|q| q != &p);
            r.insert(0, p);
            r.truncate(RECENT_MAX);
        }
    }

    fn back(&mut self) {
        let from = self.here().clone();
        if self.tab().hist.back() {
            self.after_move();
            self.select_child_named(&from);
        }
    }

    fn forward(&mut self) {
        if self.tab().hist.forward() {
            self.after_move();
        }
    }

    /// Up one level, selecting the folder we came from.
    fn up(&mut self) {
        let from = self.here().clone();
        if let Some(to) = from.up() {
            self.go(to);
            self.select_child_named(&from);
        }
    }

    /// After going up or back: select the folder (or drive) we were in.
    fn select_child_named(&mut self, from: &Location) {
        if let Location::Dir(p) = from {
            let name = match p.file_name() {
                Some(n) => n.to_string_lossy().into(),
                None => p.display().to_string().into(),
            };
            self.tab().select_later = Some(name);
        }
    }

    /// Show `path` in the current tab with `select` (names of items in it) selected.
    pub fn reveal(&mut self, dir: PathBuf, select: Vec<Box<str>>) {
        self.go(Location::Dir(dir));
        let t = self.tab();
        t.selected = select.iter().cloned().collect();
        t.select_later = select.last().cloned();
    }

    pub fn new_tab(&mut self, at: Location) {
        self.tabs.insert(self.active + 1, FTab::new(at));
        self.active += 1;
        self.after_move();
    }

    fn close_tab(&mut self, i: usize) {
        if self.tabs.len() < 2 || i >= self.tabs.len() {
            return;
        }
        let t = self.tabs.remove(i);
        self.closed.push(t.hist.here);
        if self.closed.len() > 20 {
            self.closed.remove(0);
        }
        if self.active > i || self.active == self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.path_edit = None;
        self.filter.clear();
    }

    /// Swap tab `i` with its neighbour `j` (dragging), keeping the same tab active.
    fn move_tab(&mut self, i: usize, j: usize) {
        if i >= self.tabs.len() || j >= self.tabs.len() || i == j {
            return;
        }
        self.tabs.swap(i, j);
        if self.active == i {
            self.active = j;
        } else if self.active == j {
            self.active = i;
        }
    }

    fn close_others(&mut self, keep: usize) {
        for i in (0..self.tabs.len()).rev() {
            if i != keep {
                let t = self.tabs.remove(i);
                self.closed.push(t.hist.here);
            }
        }
        let excess = self.closed.len().saturating_sub(20);
        self.closed.drain(..excess);
        self.active = 0;
        self.path_edit = None;
        self.filter.clear();
    }

    fn switch_tab(&mut self, i: usize) {
        if i < self.tabs.len() && i != self.active {
            self.active = i;
            self.path_edit = None;
            self.filter.clear();
            self.tab().scroll_to_cursor = true;
        }
    }

    fn refresh(&mut self) {
        let t = self.tab();
        let keep: Vec<Box<str>> = t.selected.iter().cloned().collect();
        let cursor = t.cursor.clone();
        t.listing = None;
        t.order_key = None;
        t.sizes = None;
        t.selected = keep.into_iter().collect();
        t.select_later = cursor;
    }

    fn pin(&mut self, p: PathBuf) {
        if !self.prefs.pinned.contains(&p) {
            self.prefs.pinned.push(p);
        }
    }

    fn unpin(&mut self, p: &Path) {
        self.prefs.pinned.retain(|q| q != p);
    }

    fn is_pinned(&self, p: &Path) -> bool {
        self.prefs.pinned.iter().any(|q| q == p)
    }

    // ---- for the analysis dock ----

    /// What the dock follows changes when any of this does: the tab, its folder, the item the
    /// keyboard is on, and how many are selected.
    pub(super) fn selection_key(&self) -> (usize, Location, Option<Box<str>>, usize) {
        let t = &self.tabs[self.active];
        (self.active, t.here().clone(), t.cursor.clone(), t.selected.len())
    }

    /// The item the keyboard is on (the last one clicked), if it's listed.
    pub(super) fn cursor_entry(&self) -> Option<Entry> {
        let t = &self.tabs[self.active];
        let c = t.cursor.as_deref()?;
        t.listing.as_ref()?.entries.iter().find(|e| &*e.name == c).cloned()
    }

    /// Select one item of the folder on screen by name, and scroll to it.
    pub(super) fn select_name(&mut self, name: &str) {
        let t = self.tab();
        let name: Box<str> = name.into();
        t.selected = std::iter::once(name.clone()).collect();
        t.anchor = Some(name.clone());
        t.cursor = Some(name);
        t.scroll_to_cursor = true;
    }

    /// The finished listing of the folder on screen: (folder, listing generation, entries).
    pub(super) fn finished_listing(&self) -> Option<(PathBuf, u64, Arc<Vec<Entry>>)> {
        let t = &self.tabs[self.active];
        let l = t.listing.as_ref().filter(|l| l.done && l.error.is_none())?;
        Some((t.here().dir()?.to_path_buf(), l.generation, l.entries.clone()))
    }

    /// Paths of the selected items, in display order.
    pub(super) fn chosen_paths(&self) -> Vec<PathBuf> {
        let t = &self.tabs[self.active];
        let (Some(dir), Some(l)) = (t.here().dir(), t.listing.as_ref()) else { return Vec::new() };
        t.order
            .iter()
            .map(|&i| &l.entries[i as usize])
            .filter(|e| t.selected.contains(&e.name))
            .map(|e| dir.join(&*e.name))
            .collect()
    }

    /// Run a command that might wait on the disk or another program, off the UI thread.
    fn spawn(&mut self, f: impl FnOnce() -> Result<(), String> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        self.results.push(rx);
    }
}

/// The folder at the end of the path the user typed: `D:` means the whole drive, `~` home.
pub(super) fn parse_typed_path(s: &str) -> Option<Location> {
    let s = s.trim().trim_matches('"');
    if s.is_empty() {
        return None;
    }
    if s.eq_ignore_ascii_case(places::computer_name()) {
        return Some(Location::Computer);
    }
    let mut p = s.to_string();
    if cfg!(windows) && p.len() == 2 && p.ends_with(':') {
        p.push('\\');
    }
    if let Some(rest) = p.strip_prefix('~')
        && (rest.is_empty() || rest.starts_with(['/', '\\']))
        && let Some(h) = places::home()
    {
        return Some(Location::Dir(h.join(rest.trim_start_matches(['/', '\\']))));
    }
    Some(Location::Dir(PathBuf::from(p)))
}

/// Does a name pass the "filter this folder" box?
pub(super) fn name_matches(filter: &str, name: &str) -> bool {
    let f = filter.trim();
    if f.is_empty() {
        return true;
    }
    if f.contains(['*', '?']) {
        scan::glob(f, name)
    } else {
        name.to_lowercase().contains(&f.to_lowercase())
    }
}

/// The display order of a folder's entries: folders first, then by the chosen key, names in
/// natural order as the tie-break.
pub(super) fn order_entries(
    entries: &[Entry],
    sizes: Option<&FolderSizes>,
    key: FSort,
    reversed: bool,
    show_hidden: bool,
    filter: &str,
) -> Vec<u32> {
    let mut o: Vec<u32> = (0..entries.len() as u32)
        .filter(|&i| {
            let e = &entries[i as usize];
            (show_hidden || !e.hidden) && name_matches(filter, &e.name)
        })
        .collect();
    let size_of = |e: &Entry| if e.is_dir { sizes.and_then(|s| s.get(&e.name)).map(|s| s.0).unwrap_or(0) } else { e.size };
    let kinds: Vec<String> = if key == FSort::Type { entries.iter().map(|e| browser::kind_for(&e.name, e.is_dir)).collect() } else { Vec::new() };
    o.sort_by(|&x, &y| {
        let (a, b) = (&entries[x as usize], &entries[y as usize]);
        let mut ord = match key {
            FSort::Name => Cmp::Equal,
            FSort::Size => size_of(a).cmp(&size_of(b)),
            FSort::Type => kinds[x as usize].cmp(&kinds[y as usize]),
            FSort::Modified => a.modified.cmp(&b.modified),
        };
        if key.natural_desc() {
            ord = ord.reverse();
        }
        let mut ord = ord.then_with(|| browser::natural_cmp(&a.name, &b.name));
        if reversed {
            ord = ord.reverse();
        }
        b.is_dir.cmp(&a.is_dir).then(ord)
    });
    o
}

/// Add the entries from `from` on that pass the filter to the end of `order`.
fn append_new(order: &mut Vec<u32>, entries: &[Entry], from: usize, show_hidden: bool, filter: &str) {
    for (i, e) in entries.iter().enumerate().skip(from) {
        if (show_hidden || !e.hidden) && name_matches(filter, &e.name) {
            order.push(i as u32);
        }
    }
}

/// Things clicked while drawing, acted on afterwards.
#[derive(Default)]
struct FPicks {
    go: Option<Location>,
    open: Option<u32>,
    select: Option<(u32, egui::Modifiers)>,
    menu: Option<(FAction, Vec<PathBuf>)>,
    sort: Option<FSort>,
    new_tab: Option<Location>,
    switch_tab: Option<usize>,
    close_tab: Option<usize>,
    close_others: Option<usize>,
    move_tab: Option<(usize, usize)>,
    clear: bool,
    unpin: Option<PathBuf>,
    move_pin: Option<(usize, bool)>,
}

/// File manager menu commands (Phase 1: the ones that don't change anything on disk).
#[derive(Clone, Copy, PartialEq, Debug)]
enum FAction {
    Open,
    OpenInNewTab,
    Pin,
    Unpin,
    CopyPath,
    CopyName,
    Reveal,
    Terminal(actions::Terminal),
    Analyse,
}

impl App {
    /// Change workspace, taking the current folder and selection along.
    pub(super) fn set_workspace(&mut self, to: Workspace) {
        let from = self.files.prefs.workspace;
        if from == to {
            return;
        }
        match to {
            Workspace::Analysis => {
                if self.job.is_none() {
                    self.analysis_follow_files();
                }
            }
            Workspace::Files => self.files_follow_analysis(),
        }
        self.files.prefs.workspace = to;
    }

    /// Storage Analysis opens at the File Manager's folder and selection, when the scan has them.
    fn analysis_follow_files(&mut self) {
        let Some(dir) = self.files.here().dir().map(Path::to_path_buf) else { return };
        let Some(tree) = self.shown_tree() else { return };
        let Some(folder) = self.tree_path_of(&dir).filter(|p| tree.at(p).is_some_and(|n| n.is_dir)) else { return };
        self.browse_to(folder);
        let chosen = self.files.chosen_paths();
        if let Some(p) = chosen.last().and_then(|p| self.tree_path_of(p)) {
            self.select(p);
        }
    }

    /// The File Manager opens at Storage Analysis's folder, with the same item selected.
    fn files_follow_analysis(&mut self) {
        if self.tree.is_none() || self.job.is_some() {
            return;
        }
        let dir = self.full_path(self.browser.current_folder());
        let names: Vec<Box<str>> = self
            .chosen()
            .iter()
            .map(|p| self.full_path(p))
            .filter(|p| p.parent() == Some(dir.as_path()))
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into()))
            .collect();
        if self.files.here().dir() != Some(dir.as_path()) || !names.is_empty() {
            self.files.reveal(dir, names);
        }
    }

    /// Workspace switcher at the left of the toolbar, in both workspaces.
    pub(super) fn workspace_switch(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let k = |key| ctx.format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key));
        let ws = self.files.prefs.workspace;
        let mut to = None;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if ui
                .add(egui::Button::selectable(ws == Workspace::Files, "🗀 Files"))
                .on_hover_text(format!("File Manager: browse and open files, no scan needed ({})", k(egui::Key::Num1)))
                .clicked()
            {
                to = Some(Workspace::Files);
            }
            if ui
                .add(egui::Button::selectable(ws == Workspace::Analysis, "⊞ Storage Analysis"))
                .on_hover_text(format!("Scan a drive or folder: folder tree, treemap, file types, duplicates… ({})", k(egui::Key::Num3)))
                .clicked()
            {
                to = Some(Workspace::Analysis);
            }
        });
        if let Some(w) = to {
            self.set_workspace(w);
        }
        ui.separator();
    }

    /// Ctrl/⌘+1 and Ctrl/⌘+3 switch workspace from anywhere.
    pub(super) fn workspace_keys(&mut self, ctx: &egui::Context) {
        if self.ops.modal() {
            return;
        }
        let (files, analysis) = ctx.input_mut(|i| {
            (i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num1), i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num3))
        });
        if files {
            self.set_workspace(Workspace::Files);
        } else if analysis {
            self.set_workspace(Workspace::Analysis);
        }
    }

    /// Analyse `dir` without leaving the File Manager: the dock opens and shows the scan, and
    /// the treemap once it's done. A folder inside the current analysis is refreshed in place.
    pub(super) fn analyse(&mut self, dir: PathBuf) {
        self.path_input = dir.display().to_string();
        let inside = self.tree.is_some() && self.job.is_none() && dir.starts_with(&self.scanned_path);
        self.start_scan_of(dir, inside);
        self.files.prefs.dock.open = true;
        self.dock.invalidate();
    }

    /// Bytes and file count of a folder according to the current scan, if it covers it.
    fn scanned_sizes(&self, dir: &Path) -> Option<FolderSizes> {
        let tree = self.tree.as_ref()?;
        let rel = dir.strip_prefix(&self.scanned_path).ok()?;
        let mut n: &Node = tree;
        for c in rel.components() {
            let name = c.as_os_str().to_string_lossy();
            n = n.children.iter().find(|k| k.is_dir && *k.name == *name)?;
        }
        Some(n.children.iter().filter(|c| c.is_dir).map(|c| (c.name.clone(), (c.size, c.files))).collect())
    }

    // ---------- drawing ----------

    /// Toolbar of the File Manager, in groups: workspaces, navigation, the path, analysis, and
    /// on the right the filter, View, Help and Settings.
    pub(super) fn files_toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        let mut go = None;
        ui.horizontal(|ui| {
            style::roomy(ui);
            self.workspace_switch(ui);
            let (can_back, can_fwd) = {
                let h = &self.files.tabs[self.files.active].hist;
                (h.can_back(), h.can_forward())
            };
            let up = self.files.here().up();
            style::tool_group(ui, |ui| {
                if style::icon_button(ui, can_back, "⏴", format!("Back ({})", k(egui::Modifiers::ALT, egui::Key::ArrowLeft))).clicked() {
                    self.files.back();
                }
                if style::icon_button(ui, can_fwd, "⏵", format!("Forward ({})", k(egui::Modifiers::ALT, egui::Key::ArrowRight))).clicked() {
                    self.files.forward();
                }
                if style::icon_button(ui, up.is_some(), "⏶", format!("Up to the enclosing folder (Backspace or {})", k(egui::Modifiers::ALT, egui::Key::ArrowUp))).clicked() {
                    self.files.up();
                }
                if style::icon_button(ui, true, "⟳", "Read the folder again (F5)").clicked() {
                    self.files.refresh();
                }
            });
            ui.add_space(2.0);
            // Right-hand controls first, so the path takes whatever width is left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::icon_button(ui, true, "⚙", "Settings: appearance, scan filters, treemap look").clicked() {
                    self.dialog.open(&self.settings);
                }
                self.help_menu(ui);
                self.files_view_menu(ui);
                let f = ui.add(
                    egui::TextEdit::singleline(&mut self.files.filter)
                        .id(egui::Id::new("files-filter"))
                        .desired_width(160.0)
                        .hint_text(format!("🔍 Filter ({})", k(egui::Modifiers::COMMAND, egui::Key::F))),
                );
                f.on_hover_text("Show only items whose names contain this text. * and ? work as wildcards (*.exr, shot_??.png).");
                if std::mem::take(&mut self.files.focus_filter) {
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new("files-filter")));
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) && ctx.memory(|m| m.has_focus(egui::Id::new("files-filter"))) {
                    self.files.filter.clear();
                    ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("files-filter")));
                }
                ui.add_space(6.0);
                self.dock_toolbar_buttons(ui);
                ui.add_space(4.0);
                let w = ui.available_width();
                ui.allocate_ui_with_layout(vec2(w, ui.spacing().interact_size.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    go = self.path_bar(ui);
                });
            });
        });
        if let Some(to) = go {
            self.files.go(to);
        }
    }

    /// The View menu: hidden items, row density and column widths.
    fn files_view_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        ui.menu_button("View ⏷", |ui| {
            let p = &mut self.files.prefs;
            if ui
                .add(egui::Button::selectable(p.show_hidden, "Hidden Items").shortcut_text(k(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Period)))
                .on_hover_text("Show hidden files and folders: names starting with a dot, and items marked hidden on Windows")
                .clicked()
            {
                p.show_hidden = !p.show_hidden;
            }
            ui.separator();
            ui.weak("Rows");
            ui.radio_value(&mut p.density, Density::Comfortable, "Comfortable");
            ui.radio_value(&mut p.density, Density::Compact, "Compact");
            ui.separator();
            if ui.add_enabled(p.col_widths != COL_WIDTHS, egui::Button::new("Reset Column Widths")).clicked() {
                p.col_widths = COL_WIDTHS;
            }
            ui.separator();
            let dock = self.files.prefs.dock.open;
            if ui.add(egui::Button::selectable(dock, "Analysis Dock").shortcut_text(k(egui::Modifiers::COMMAND, egui::Key::J))).clicked() {
                self.files.prefs.dock.open = !dock;
            }
        })
        .response
        .on_hover_text("Hidden items, row size, columns and the analysis dock");
    }

    /// The path as clickable breadcrumbs; click the empty part (or Ctrl+L) to type a path.
    fn path_bar(&mut self, ui: &mut egui::Ui) -> Option<Location> {
        let id = egui::Id::new("files-path");
        let rect = ui.available_rect_before_wrap();
        let frame = Rect::from_min_size(rect.min, vec2(rect.width(), ui.spacing().interact_size.y));
        if let Some(text) = &mut self.files.path_edit {
            let resp = ui.add(egui::TextEdit::singleline(text).id(id).desired_width(frame.width()).hint_text("Type a folder, e.g. ~/Projects, D:\\ or \\\\server\\share"));
            if std::mem::take(&mut self.files.focus_path) {
                resp.request_focus();
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
                    let end = egui::text::CCursor::new(text.chars().count());
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), end)));
                    st.store(ui.ctx(), id);
                }
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter {
                let to = parse_typed_path(text);
                self.files.path_edit = None;
                return to;
            }
            if resp.lost_focus() {
                self.files.path_edit = None;
            }
            return None;
        }
        // Breadcrumbs.
        let bg = ui.interact(frame, id.with("bg"), Sense::click());
        let pal = style::pal(ui);
        let border = if bg.hovered() { pal.weak } else { pal.hairline };
        ui.painter().rect(frame, style::RADIUS as f32, pal.well, Stroke::new(1.0, border), egui::StrokeKind::Inside);
        let mut go = None;
        let here = self.files.here().clone();
        let crumbs = crumbs_of(&here);
        let last = crumbs.len().saturating_sub(1);
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        // Keep the end of a long path visible: drop leading crumbs that don't fit.
        let font = egui::TextStyle::Button.resolve(ui.style());
        let width_of = |s: &str| ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x) + 16.0;
        let mut total: f32 = crumbs.iter().map(|c| width_of(&c.0)).sum();
        let mut skip = 0;
        while total > frame.width() - 40.0 && skip + 1 < crumbs.len() {
            total -= width_of(&crumbs[skip].0);
            skip += 1;
        }
        if skip > 0 {
            ui.weak("…");
        }
        for (k, (label, loc)) in crumbs.into_iter().enumerate().skip(skip) {
            if k > skip {
                ui.weak("›");
            }
            let text = if k == last { egui::RichText::new(label).strong() } else { egui::RichText::new(label) };
            let resp = ui.add(egui::Button::new(text).frame(false));
            if resp.clicked() && k != last {
                go = Some(loc.clone());
            }
            if resp.middle_clicked() {
                self.files.new_tab(loc);
            }
        }
        if bg.clicked() {
            self.start_path_edit();
        }
        bg.on_hover_text("Click to type a path");
        go
    }

    fn start_path_edit(&mut self) {
        let text = match self.files.here() {
            Location::Dir(p) => p.display().to_string(),
            Location::Computer => String::new(),
        };
        self.files.path_edit = Some(text);
        self.files.focus_path = true;
    }

    /// Everything below the toolbar in the File Manager workspace.
    pub(super) fn files_workspace(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.files_poll(&ctx);
        self.dock_poll(&ctx);
        let pal = style::pal(ui);

        let status = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::symmetric(10, 3));
        let r = egui::Panel::bottom("files-status").frame(status).show(ui, |ui| self.files_status(ui));
        self.help_regions.push((r.response.rect, topics::FILE_MANAGER));
        let side = egui::Frame::new().fill(pal.side).inner_margin(egui::Margin { left: 6, right: 4, top: 8, bottom: 6 });
        let r = egui::Panel::left("places")
            .default_size(self.files.prefs.sidebar_width)
            .min_size(150.0)
            .resizable(true)
            .frame(side)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.sidebar(ui));
            });
        self.files.prefs.sidebar_width = r.response.rect.width();
        self.help_regions.push((r.response.rect, topics::FILE_MANAGER));
        let r = egui::CentralPanel::default().frame(egui::Frame::new().fill(pal.chrome)).show(ui, |ui| self.files_main(ui));
        self.help_regions.push((r.response.rect, topics::FILE_MANAGER));
    }

    /// Start listings, collect them, re-sort, and gather results of background commands.
    fn files_poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.files.places_rx {
            if let Ok(p) = rx.try_recv() {
                self.files.places = p;
                self.files.places_rx = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        let mut done = Vec::new();
        self.files.results.retain(|rx| match rx.try_recv() {
            Ok(r) => {
                done.push(r);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        for r in done {
            if let Err(e) = r {
                self.ops.notice = Some((e, true));
            }
        }
        if !self.files.results.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        let tree_id = self.tree.as_ref().map(|t| Arc::as_ptr(t) as usize).unwrap_or(0);
        let need_sizes = {
            let t = &self.files.tabs[self.files.active];
            t.sizes.as_ref().map(|s| s.0) != Some(tree_id)
        };
        if need_sizes {
            let sizes = self.files.here().dir().and_then(|d| self.scanned_sizes(d)).unwrap_or_default();
            self.files.tab().sizes = Some((tree_id, sizes));
        }
        let (sort, reversed, hidden) = (self.files.prefs.sort, self.files.prefs.reversed, self.files.prefs.show_hidden);
        let filter = self.files.filter.clone();
        let t = self.files.tab();
        if t.listing.is_none()
            && let Location::Dir(p) = t.here().clone()
        {
            t.listing = Some(Listing::start(p));
        }
        let Some(l) = t.listing.as_mut() else { return };
        l.poll();
        if !l.done {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let key: OrderKey = (l.generation, sort, reversed, hidden, filter.clone(), tree_id);
        let sizes = t.sizes.as_ref().map(|s| s.1.clone()).unwrap_or_default();
        // A sort finished on another thread: use it, adding anything listed since at the end.
        if let Some((k, covered, rx)) = &t.sorting {
            match rx.try_recv() {
                Ok(order) => {
                    let same = (k.1, k.2, k.3, &k.4, k.5) == (sort, reversed, hidden, &filter, tree_id);
                    let covered = *covered;
                    t.sorting = None;
                    if same {
                        t.order = order;
                        let l = t.listing.as_ref().unwrap();
                        append_new(&mut t.order, &l.entries, covered, hidden, &filter);
                        t.order_key = Some(key.clone());
                    }
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(30)),
                Err(mpsc::TryRecvError::Disconnected) => t.sorting = None,
            }
        }
        if t.order_key.as_ref() != Some(&key) && t.sorting.is_none() {
            let l = t.listing.as_ref().unwrap();
            let n = l.entries.len();
            let only_more = t.order_key.as_ref().is_some_and(|k| (k.1, k.2, k.3, &k.4, k.5) == (sort, reversed, hidden, &filter, tree_id));
            if n <= SORT_INLINE_MAX {
                t.order = order_entries(&l.entries, Some(&sizes), sort, reversed, hidden, &filter);
                t.order_key = Some(key);
            } else if only_more && !l.done {
                // A big folder still streaming in: show new items at the end, sort once it's all here.
                let seen: usize = t.order.iter().map(|&i| i as usize + 1).max().unwrap_or(0);
                append_new(&mut t.order, &l.entries, seen, hidden, &filter);
                t.order_key = Some(key);
            } else {
                let entries = l.entries.clone();
                let (tx, rx) = mpsc::channel();
                let f = filter.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(order_entries(&entries, Some(&sizes), sort, reversed, hidden, &f));
                });
                t.sorting = Some((key, n, rx));
                ctx.request_repaint_after(Duration::from_millis(30));
            }
        }
        // Select an item we were asked to, once it's listed.
        if let Some(name) = t.select_later.clone() {
            if t.pos_of(&name).is_some() {
                if t.selected.is_empty() {
                    t.selected.insert(name.clone());
                }
                t.cursor = Some(name.clone());
                t.anchor = Some(name);
                t.select_later = None;
                t.scroll_to_cursor = true;
            } else if t.listing.as_ref().is_some_and(|l| l.done) {
                t.select_later = None;
            }
        }
    }

    // ---------- sidebar ----------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let mut go: Option<Location> = None;
        let mut new_tab = None;
        let mut picks = FPicks::default();
        let here = self.files.here().clone();
        ui.spacing_mut().item_spacing.y = 1.0;
        let pressed = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| ui.max_rect().contains(p)));
        if pressed {
            self.files.focused = true;
        }

        let mut entry = |ui: &mut egui::Ui, icon: &str, label: &str, loc: Location, tip: String| -> egui::Response {
            let resp = side_row(ui, icon, label, here == loc, ROW_F).on_hover_text(tip);
            if resp.clicked() {
                go = Some(loc.clone());
            }
            if resp.middle_clicked() {
                new_tab = Some(loc);
            }
            resp
        };
        let drive_entry = |ui: &mut egui::Ui, icon: &str, label: &str, loc: Location, tip: String, frac: Option<f32>| -> egui::Response {
            let resp = side_row(ui, icon, label, here == loc, 30.0).on_hover_text(tip);
            // A thin capacity bar under the name, red when nearly full.
            if let Some(frac) = frac {
                let r = resp.rect;
                let bar = Rect::from_min_size(pos2(r.left() + 28.0, r.bottom() - 7.0), vec2((r.width() - 38.0).max(10.0), 3.0));
                let p = ui.painter();
                p.rect_filled(bar, 1.5, ui.visuals().extreme_bg_color);
                let mut fill = bar;
                fill.set_width(bar.width() * frac.clamp(0.0, 1.0));
                let c = if frac > 0.9 { Color32::from_rgb(210, 70, 60) } else { Color32::from_rgb(70, 130, 210) };
                p.rect_filled(fill, 1.5, c);
            }
            resp
        };

        side_section(ui, "fm-quick", "Quick access", true, |ui| {
            if self.files.places.is_empty() && self.files.places_rx.is_some() {
                ui.weak("…");
            }
            for p in self.files.places.clone() {
                let resp = entry(ui, p.kind.icon(), &p.name, Location::Dir(p.path.clone()), p.path.display().to_string());
                let pinned = self.files.is_pinned(&p.path);
                resp.context_menu(|ui| place_menu(ui, &p.path, pinned, &mut picks));
            }
        });
        if !self.files.prefs.pinned.is_empty() {
            side_section(ui, "fm-pinned", "Pinned", true, |ui| {
                let pinned = self.files.prefs.pinned.clone();
                let n = pinned.len();
                for (i, p) in pinned.iter().enumerate() {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                    let resp = entry(ui, "★", &name, Location::Dir(p.clone()), p.display().to_string());
                    resp.context_menu(|ui| {
                        place_menu(ui, p, true, &mut picks);
                        ui.separator();
                        if ui.add_enabled(i > 0, egui::Button::new("Move Up")).clicked() {
                            picks.move_pin = Some((i, true));
                        }
                        if ui.add_enabled(i + 1 < n, egui::Button::new("Move Down")).clicked() {
                            picks.move_pin = Some((i, false));
                        }
                    });
                }
            });
        }
        let computer = places::computer_name();
        side_section(ui, "fm-drives", "Drives", true, |ui| {
            entry(ui, "🖳", computer, Location::Computer, format!("All drives and volumes ({computer})"));
            if self.drives.is_empty() && self.drives_rx.is_some() {
                ui.weak("Looking for drives…");
            }
            let drives: Vec<(PathBuf, String, u64, u64, bool)> = self.drives.iter().map(|d| (d.root.clone(), d.label.clone(), d.total, d.free, d.network)).collect();
            for (root, label, total, free, network) in drives {
                let name = drive_label(&root, &label);
                let icon = if network { "🖧" } else { "🖴" };
                let used = total.saturating_sub(free);
                let tip = format!("{}\n{} free of {}{}", root.display(), scan::human(free), scan::human(total), if network { "\nNetwork drive" } else { "" });
                let frac = (total > 0).then(|| used as f32 / total as f32);
                let resp = drive_entry(ui, icon, &name, Location::Dir(root.clone()), tip, frac);
                if resp.clicked() {
                    picks.go = Some(Location::Dir(root.clone()));
                }
                if resp.middle_clicked() {
                    picks.new_tab = Some(Location::Dir(root.clone()));
                }
                let pinned = self.files.is_pinned(&root);
                resp.context_menu(|ui| place_menu(ui, &root, pinned, &mut picks));
            }
            ui.add_space(2.0);
            if ui.add(egui::Button::new(egui::RichText::new("⟳ Refresh drives").small().color(style::pal(ui).weak)).frame_when_inactive(false)).clicked() {
                self.refresh_drives();
            }
        });
        if !self.files.prefs.recent.is_empty() {
            side_section(ui, "fm-recent", "Recent", false, |ui| {
                for p in self.files.prefs.recent.clone() {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                    let resp = entry(ui, "🕘", &name, Location::Dir(p.clone()), p.display().to_string());
                    let pinned = self.files.is_pinned(&p);
                    resp.context_menu(|ui| place_menu(ui, &p, pinned, &mut picks));
                }
            });
        }
        if let Some(l) = go {
            self.files.go(l);
        }
        if let Some(l) = new_tab.or(picks.new_tab.take()) {
            self.files.new_tab(l);
        }
        if let Some(p) = picks.unpin.take() {
            self.files.unpin(&p);
        }
        if let Some((i, up)) = picks.move_pin {
            let j = if up { i - 1 } else { i + 1 };
            self.files.prefs.pinned.swap(i, j);
        }
        self.files_apply(picks);
    }

    // ---------- main area ----------



    fn files_main(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let mut picks = FPicks::default();
        ui.spacing_mut().item_spacing.y = 0.0;
        self.files_tabs_row(ui, &mut picks);
        let pal = style::pal(ui);
        // The page under the tabs: the list, and the analysis dock below it when it's open.
        let page = ui.available_rect_before_wrap();
        ui.painter().rect_filled(page, 0.0, pal.paper);
        let mut page_ui = ui.new_child(egui::UiBuilder::new().max_rect(page).layout(egui::Layout::top_down(egui::Align::Min)));
        let ui = &mut page_ui;
        if self.files.prefs.dock.open {
            let max = (page.height() - DOCK_MIN_LIST).max(DOCK_MIN);
            let mut panel = egui::Panel::bottom("files-dock")
                .resizable(true)
                .size_range(DOCK_MIN..=max)
                .default_size(self.files.prefs.dock.height.clamp(DOCK_MIN, max))
                .frame(egui::Frame::new().fill(pal.paper));
            if self.files.prefs.dock.maximised {
                panel = panel.exact_size(max);
            }
            let r = panel.show(ui, |ui| self.dock_ui(ui));
            if !self.files.prefs.dock.maximised {
                self.files.prefs.dock.height = r.response.rect.height();
            }
            // A hairline above the dock, over the panel's own separator.
            let top = r.response.rect.top();
            ui.painter().hline(r.response.rect.x_range(), top, Stroke::new(1.0, pal.hairline));
            self.help_regions.push((r.response.rect, topics::ANALYSIS_DOCK));
        }
        let list = ui.available_rect_before_wrap();
        let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        if pressed.is_some_and(|p| list.contains(p) || rect.top() + TAB_STRIP_H > p.y && rect.contains(p)) {
            self.files.focused = true;
        }
        self.files_keys(ui.ctx(), ui.rect_contains_pointer(list), &mut picks);
        self.files_apply(std::mem::take(&mut picks));

        match self.files.here().clone() {
            Location::Computer => {
                egui::Frame::new().inner_margin(egui::Margin::same(14)).show(ui, |ui| self.computer_view(ui, &mut picks));
            }
            Location::Dir(dir) => self.details_view(ui, &dir, &mut picks),
        }
        self.files_apply(picks);
    }

    /// The tabs: each with its folder icon, the active one joined to the page below; drag one
    /// to reorder, middle-click or ✖ to close. Too many to fit: the strip scrolls sideways.
    fn files_tabs_row(&mut self, ui: &mut egui::Ui, picks: &mut FPicks) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        let pal = style::pal(ui);
        let strip = Rect::from_min_size(ui.available_rect_before_wrap().min, vec2(ui.available_width(), TAB_STRIP_H));
        ui.allocate_rect(strip, Sense::hover());
        ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, pal.hairline));
        let n = self.files.tabs.len();
        let plus_w = 30.0;
        let avail = (strip.width() - plus_w - 8.0).max(80.0);
        // Tabs share the width, between a minimum and a maximum.
        let tab_w = (avail / n as f32).clamp(TAB_MIN_W, TAB_MAX_W);
        let total = tab_w * n as f32;
        let mut strip_ui = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Max)));
        let mut drag_to = None;
        egui::ScrollArea::horizontal()
            .id_salt("fm-tabs")
            .max_width(avail)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut strip_ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut rects = Vec::with_capacity(n);
                for i in 0..n {
                    let here = self.files.tabs[i].here().clone();
                    let on = i == self.files.active;
                    let (r, resp) = ui.allocate_exact_size(vec2(tab_w, TAB_STRIP_H - 4.0), Sense::click_and_drag());
                    rects.push(r);
                    let hovered = resp.hovered() || resp.dragged();
                    let p = ui.painter();
                    let shape = egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 };
                    if on {
                        // Joined to the page: paper colour, a hairline round it, an accent line on top.
                        let body = Rect::from_min_max(r.min, pos2(r.right(), r.bottom() + 1.0));
                        p.rect(body, shape, pal.paper, Stroke::new(1.0, pal.hairline), egui::StrokeKind::Inside);
                        p.rect_filled(Rect::from_min_max(pos2(r.left() + 1.0, r.bottom() - 1.0), pos2(r.right() - 1.0, r.bottom() + 1.5)), 0.0, pal.paper);
                        p.rect_filled(Rect::from_min_size(r.min + vec2(6.0, 0.0), vec2(r.width() - 12.0, 2.0)), 1.0, pal.accent);
                    } else if hovered {
                        p.rect_filled(r.shrink2(vec2(1.0, 2.0)), shape, pal.hover);
                    } else if i + 1 != self.files.active && i + 1 < n {
                        // Separators between inactive tabs.
                        p.vline(r.right() - 0.5, r.y_range().shrink(7.0), Stroke::new(1.0, pal.hairline));
                    }
                    let title = location_title(&here);
                    let tc = if on { pal.strong } else { pal.text };
                    let icon = if matches!(here, Location::Computer) { "🖳" } else { "🗀" };
                    let cy = r.center().y + 1.0;
                    p.text(pos2(r.left() + 10.0, cy), Align2::LEFT_CENTER, icon, FontId::proportional(13.0), if on { pal.accent } else { pal.weak });
                    let show_x = n > 1 && (on || hovered);
                    let clip = Rect::from_min_max(pos2(r.left() + 28.0, r.top()), pos2(r.right() - if show_x { 24.0 } else { 8.0 }, r.bottom()));
                    p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(clip.left(), cy), Align2::LEFT_CENTER, &title, FontId::proportional(12.5), tc);
                    let tip = match &here {
                        Location::Dir(p) => p.display().to_string(),
                        Location::Computer => places::computer_name().to_string(),
                    };
                    let mut closed = false;
                    if show_x {
                        let xr = Rect::from_center_size(pos2(r.right() - 13.0, cy), vec2(16.0, 16.0));
                        let x = ui.interact(xr, ui.id().with(("tab-x", i)), Sense::click());
                        if x.hovered() {
                            ui.painter().rect_filled(xr, 3.0, pal.hover);
                        }
                        ui.painter().text(xr.center(), Align2::CENTER_CENTER, "✖", FontId::proportional(9.5), if x.hovered() { pal.strong } else { pal.weak });
                        if x.on_hover_text(format!("Close tab ({})", k(egui::Modifiers::COMMAND, egui::Key::W))).clicked() {
                            picks.close_tab = Some(i);
                            closed = true;
                        }
                    }
                    if resp.dragged()
                        && let Some(p) = ctx.pointer_interact_pos()
                    {
                        drag_to = Some((i, p.x));
                    }
                    let resp = resp.on_hover_text(tip);
                    if !closed && resp.clicked() && !on {
                        picks.switch_tab = Some(i);
                    }
                    if resp.middle_clicked() && n > 1 {
                        picks.close_tab = Some(i);
                    }
                    resp.context_menu(|ui| {
                        if ui.button("New Tab").clicked() {
                            picks.new_tab = Some(here.clone());
                        }
                        if ui.add_enabled(n > 1, egui::Button::new("Close Tab")).clicked() {
                            picks.close_tab = Some(i);
                        }
                        if ui.add_enabled(n > 1, egui::Button::new("Close Other Tabs")).clicked() {
                            picks.close_others = Some(i);
                        }
                    });
                }
                // Dragging a tab past the middle of its neighbour swaps them.
                if let Some((i, x)) = drag_to {
                    if i > 0 && x < rects[i - 1].center().x {
                        picks.move_tab = Some((i, i - 1));
                    } else if i + 1 < n && x > rects[i + 1].center().x {
                        picks.move_tab = Some((i, i + 1));
                    }
                }
            });
        let plus_at = pos2((strip.left() + 6.0 + total.min(avail) + 4.0).min(strip.right() - plus_w), strip.top() + 4.0);
        let plus_r = Rect::from_min_size(plus_at, vec2(26.0, TAB_STRIP_H - 8.0));
        let plus = ui.interact(plus_r, ui.id().with("tab-plus"), Sense::click());
        if plus.hovered() {
            ui.painter().rect_filled(plus_r, 5.0, pal.hover);
        }
        ui.painter().text(plus_r.center(), Align2::CENTER_CENTER, "+", FontId::proportional(17.0), if plus.hovered() { pal.strong } else { pal.weak });
        if plus.on_hover_text(format!("New tab ({})", k(egui::Modifiers::COMMAND, egui::Key::T))).clicked() {
            picks.new_tab = Some(self.files.here().clone());
        }
    }

    /// "This PC" / "Computer": the drives and volumes, with their space.
    fn computer_view(&mut self, ui: &mut egui::Ui, picks: &mut FPicks) {
        ui.add_space(4.0);
        ui.heading(places::computer_name());
        ui.add_space(6.0);
        if self.drives.is_empty() {
            ui.weak(if self.drives_rx.is_some() { "Looking for drives…" } else { "No drives found." });
            return;
        }
        let drives: Vec<(PathBuf, String, u64, u64, bool)> = self.drives.iter().map(|d| (d.root.clone(), d.label.clone(), d.total, d.free, d.network)).collect();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                for (root, label, total, free, network) in drives {
                    let (r, resp) = ui.allocate_exact_size(vec2(250.0, 58.0), Sense::click());
                    let vis = ui.visuals();
                    let bg = if resp.hovered() { vis.widgets.hovered.weak_bg_fill } else { vis.faint_bg_color };
                    let p = ui.painter();
                    p.rect_filled(r, 6.0, bg);
                    p.text(pos2(r.left() + 12.0, r.top() + 18.0), Align2::LEFT_CENTER, if network { "🖧" } else { "🖴" }, FontId::proportional(22.0), vis.text_color());
                    p.text(pos2(r.left() + 44.0, r.top() + 14.0), Align2::LEFT_CENTER, drive_label(&root, &label), FontId::proportional(14.0), vis.strong_text_color());
                    let used = total.saturating_sub(free);
                    let frac = if total > 0 { used as f32 / total as f32 } else { 0.0 };
                    let bar = Rect::from_min_size(pos2(r.left() + 44.0, r.top() + 28.0), vec2(r.width() - 56.0, 7.0));
                    p.rect_filled(bar, 2.0, vis.extreme_bg_color);
                    let mut fill = bar;
                    fill.set_width(bar.width() * frac.clamp(0.0, 1.0));
                    p.rect_filled(fill, 2.0, if frac > 0.9 { Color32::from_rgb(210, 70, 60) } else { vis.selection.bg_fill });
                    p.text(
                        pos2(r.left() + 44.0, r.top() + 46.0),
                        Align2::LEFT_CENTER,
                        format!("{} free of {}", scan::human(free), scan::human(total)),
                        FontId::proportional(12.0),
                        vis.weak_text_color(),
                    );
                    let resp = resp.on_hover_text(root.display().to_string());
                    if resp.double_clicked() || resp.clicked() {
                        picks.go = Some(Location::Dir(root.clone()));
                    }
                    if resp.middle_clicked() {
                        picks.new_tab = Some(Location::Dir(root.clone()));
                    }
                    let pinned = self.files.is_pinned(&root);
                    resp.context_menu(|ui| {
                        if ui.button("Analyse This Drive").on_hover_text("Scan it and show its treemap in the analysis dock").clicked() {
                            picks.menu = Some((FAction::Analyse, vec![root.clone()]));
                        }
                        ui.separator();
                        place_menu(ui, &root, pinned, picks);
                    });
                }
            });
        });
    }

    /// Column widths: Size, Type and Date modified as the user sized them (dropping Type, then
    /// the date, when the window is too narrow), and Name gets the rest.
    fn details_columns(width: f32, widths: [f32; 3]) -> Vec<(Option<FSort>, f32)> {
        let mut cols = vec![(Some(FSort::Size), widths[0]), (Some(FSort::Type), widths[1]), (Some(FSort::Modified), widths[2])];
        for drop in [FSort::Type, FSort::Modified] {
            if width - cols.iter().map(|c| c.1).sum::<f32>() >= 220.0 {
                break;
            }
            cols.retain(|c| c.0 != Some(drop));
        }
        let fixed: f32 = cols.iter().map(|c| c.1).sum();
        cols.insert(0, (Some(FSort::Name), (width - fixed).max(160.0)));
        cols
    }

    /// The Details list of a folder.
    fn details_view(&mut self, ui: &mut egui::Ui, dir: &Path, picks: &mut FPicks) {
        let width = ui.available_width();
        let cols = Self::details_columns(width, self.files.prefs.col_widths);
        let row_h = self.files.prefs.density.row();
        let pal = style::pal(ui);
        let footer_h = 22.0;
        let mut body = ui.available_rect_before_wrap();
        body.max.y -= footer_h;
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom()), pos2(body.right(), body.bottom() + footer_h));
        // The background: a click on empty space clears the selection; right-click for a menu.
        let empty = ui.interact(body, ui.id().with("fm-files-body"), Sense::click());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        let ui2 = &mut child;

        // Headings, with dividers you can drag to resize the column to their right.
        let hdr_h = 26.0;
        let (hdr, _) = ui2.allocate_exact_size(vec2(width, hdr_h), Sense::hover());
        ui2.painter().hline(hdr.x_range(), hdr.bottom() - 0.5, Stroke::new(1.0, pal.hairline));
        let mut x = hdr.left();
        let (sort, reversed) = (self.files.prefs.sort, self.files.prefs.reversed);
        let mut resized = None;
        for (i, (col, w)) in cols.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(x, hdr.top()), vec2(*w, hdr_h));
            x += w;
            let Some(col) = *col else { continue };
            let resp = ui2.interact(cell, ui2.id().with(("fhdr", i)), Sense::click());
            let on = col == sort;
            if resp.hovered() {
                ui2.painter().rect_filled(cell.shrink2(vec2(1.0, 3.0)), style::RADIUS as f32, pal.hover);
            }
            let color = if resp.hovered() || on { pal.strong } else { pal.weak };
            let font = FontId::proportional(12.0);
            let (at, align) = match col {
                FSort::Size => (pos2(cell.right() - 10.0, cell.center().y), Align2::RIGHT_CENTER),
                _ => (pos2(cell.left() + if col == FSort::Name { 10.0 } else { 8.0 }, cell.center().y), Align2::LEFT_CENTER),
            };
            let g = ui2.painter().layout_no_wrap(col.title().to_string(), font, color);
            let tr = align.anchor_size(at, g.size());
            ui2.painter().galley(tr.min, g, color);
            if on {
                // A small chevron beside the title: up for A→Z / smallest first.
                let up = reversed == col.natural_desc();
                let ax = if align == Align2::RIGHT_CENTER { tr.left() - 9.0 } else { tr.right() + 9.0 };
                let c = pos2(ax, cell.center().y);
                let d = if up { -1.0 } else { 1.0 };
                let pts = vec![c + vec2(-3.5, 1.75 * d), c + vec2(0.0, -1.75 * d), c + vec2(3.5, 1.75 * d)];
                ui2.painter().add(egui::Shape::line(pts, Stroke::new(1.4, pal.accent)));
            }
            if resp.on_hover_text("Click to sort; again to reverse").clicked() {
                picks.sort = Some(col);
            }
            // Divider at this column's left edge (not before Name); dragging it resizes this column.
            if i > 0 {
                let line = cell.left();
                ui2.painter().vline(line, hdr.y_range().shrink(6.0), Stroke::new(1.0, pal.hairline));
                let grip = Rect::from_center_size(pos2(line, hdr.center().y), vec2(8.0, hdr_h));
                let g = ui2.interact(grip, ui2.id().with(("fgrip", i)), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeColumn);
                if g.dragged() {
                    resized = Some((col, -g.drag_delta().x));
                }
                if g.double_clicked() {
                    resized = Some((col, f32::NAN));
                }
            }
        }
        if let Some((col, dx)) = resized {
            let k = match col {
                FSort::Size => 0,
                FSort::Type => 1,
                _ => 2,
            };
            let w = &mut self.files.prefs.col_widths[k];
            *w = if dx.is_nan() { COL_WIDTHS[k] } else { (*w + dx).clamp(COL_MIN, 600.0) };
        }

        let focused = self.files.focused;
        let stripes = self.view().stripes;
        let t = &self.files.tabs[self.files.active];
        let Some(l) = t.listing.as_ref() else { return };
        let (done, error, slow, total) = (l.done, l.error.clone(), l.slow(), l.entries.len());
        if t.order.is_empty() {
            ui2.add_space((body.height() * 0.22).clamp(24.0, 120.0));
            ui2.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                // A large faint symbol, then one line saying why there's nothing to show.
                let big = |ui: &mut egui::Ui, s: &str| {
                    ui.label(egui::RichText::new(s).size(40.0).color(style::blend(pal.paper, pal.weak, 0.45)));
                };
                if !done {
                    ui.spinner();
                    if slow {
                        ui.label(format!("Still waiting for {}…", dir.display()));
                        ui.weak("Network drives and sleeping disks can take a while. You can keep using dirmap meanwhile.");
                        if ui.button("Cancel").clicked() {
                            picks.clear = true;
                        }
                    }
                } else if let Some(e) = &error {
                    big(ui, "⚠");
                    ui.colored_label(ui.visuals().error_fg_color, e);
                    if ui.button("Go up").clicked() {
                        picks.go = Location::Dir(dir.to_path_buf()).up();
                    }
                } else if total > 0 && !self.files.filter.trim().is_empty() {
                    big(ui, "🔍");
                    ui.label(egui::RichText::new("Nothing here matches the filter").color(pal.weak));
                } else if total > 0 {
                    big(ui, "🗀");
                    ui.label(egui::RichText::new("Only hidden items here. Choose View > Hidden Items to show them.").color(pal.weak));
                } else {
                    big(ui, "🗀");
                    ui.label(egui::RichText::new("This folder is empty").color(pal.weak));
                }
            });
        } else {
            let sizes = t.sizes.as_ref().map(|s| &s.1);
            let cursor_row = t.cursor.as_deref().and_then(|c| t.pos_of(c));
            let mut area = egui::ScrollArea::vertical().id_salt(("flist", dir)).auto_shrink(false);
            if let Some(r) = cursor_row.filter(|_| t.scroll_to_cursor) {
                area = area.vertical_scroll_offset(browser::keep_in_view(t.view, r as f32 * row_h, row_h));
            }
            ui2.spacing_mut().item_spacing.y = 0.0;
            let vis = ui2.visuals().clone();
            let (sel_bg, sel_fg) = if focused { (pal.sel, pal.strong) } else { (pal.sel_dim, pal.strong) };
            let font = FontId::proportional(13.0);
            let order = &t.order;
            let colors = &self.colors;
            let color_for = |ext: &str| colors.get(ext).or_else(|| colors.get(&ext.to_lowercase())).copied().unwrap_or(Color32::GRAY);
            let mut hovered_row = None;
            let out = area.show_rows(ui2, row_h, order.len(), |ui, range| {
                for r in range {
                    let i = order[r];
                    let e = &l.entries[i as usize];
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                    let selected = t.selected.contains(&e.name);
                    let p = ui.painter().clone();
                    if selected {
                        p.rect_filled(rect.shrink2(vec2(2.0, 0.5)), style::RADIUS as f32, sel_bg);
                    } else if resp.hovered() {
                        p.rect_filled(rect.shrink2(vec2(2.0, 0.5)), style::RADIUS as f32, pal.hover);
                    } else if r % 2 == 1 && stripes {
                        p.rect_filled(rect, 0.0, pal.stripe);
                    }
                    if focused && t.cursor.as_deref() == Some(&*e.name) && !selected {
                        p.rect_stroke(rect.shrink2(vec2(2.0, 0.5)), style::RADIUS as f32, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                    }
                    let faint = e.hidden;
                    let tc = if selected { sel_fg } else if faint { pal.weak } else { pal.text };
                    let weak = if selected { sel_fg } else { pal.weak };
                    let cy = rect.center().y;
                    let mut x = rect.left();
                    for (col, w) in &cols {
                        let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, row_h));
                        x += w;
                        let right = pos2(cell.right() - 10.0, cy);
                        match col {
                            Some(FSort::Name) => {
                                let icon = Rect::from_min_size(pos2(cell.left() + 10.0, cy - 8.0), vec2(16.0, 16.0));
                                let tint = if faint { Color32::from_gray(120) } else { color_for(e.ext()) };
                                self.icons.paint(ui, icon, e.is_dir, e.ext(), tint);
                                if e.is_link {
                                    p.text(icon.left_bottom() + vec2(-3.0, 2.0), Align2::LEFT_BOTTOM, "↪", FontId::proportional(10.0), vis.strong_text_color());
                                }
                                let clip = Rect::from_min_max(pos2(cell.left() + 33.0, rect.top()), pos2(cell.right() - 6.0, rect.bottom()));
                                p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(clip.left(), cy), Align2::LEFT_CENTER, &*e.name, font.clone(), tc);
                            }
                            Some(FSort::Size) => {
                                if !e.is_dir {
                                    p.text(right, Align2::RIGHT_CENTER, scan::human(e.size), font.clone(), tc);
                                } else if let Some(&(bytes, _)) = sizes.and_then(|s| s.get(&e.name)) {
                                    // From the scan: shown in italics so it doesn't read as live.
                                    let f = egui::text::LayoutJob::simple_singleline(
                                        scan::human(bytes),
                                        font.clone(),
                                        weak,
                                    );
                                    let mut f = f;
                                    f.sections[0].format.italics = true;
                                    let g = ui.fonts_mut(|fo| fo.layout_job(f));
                                    p.galley(pos2(right.x - g.size().x, cy - g.size().y / 2.0), g, weak);
                                }
                            }
                            Some(FSort::Type) => {
                                let kind = if e.is_link { format!("Link to {}", browser::kind_for(&e.name, e.is_dir).to_lowercase()) } else { browser::kind_for(&e.name, e.is_dir) };
                                p.with_clip_rect(cell.shrink2(vec2(6.0, 0.0)).intersect(p.clip_rect())).text(pos2(cell.left() + 8.0, cy), Align2::LEFT_CENTER, kind, font.clone(), weak);
                            }
                            Some(FSort::Modified) if e.modified > 0 => {
                                p.text(pos2(cell.left() + 8.0, cy), Align2::LEFT_CENTER, fmt_time(e.modified), font.clone(), weak);
                            }
                            Some(FSort::Modified) => {}
                            None => {}
                        }
                    }
                    if resp.hovered() {
                        hovered_row = Some(i);
                    }
                    if resp.double_clicked() {
                        picks.open = Some(i);
                    } else if resp.clicked() {
                        picks.select = Some((i, ui.input(|inp| inp.modifiers)));
                    } else if resp.secondary_clicked() && !selected {
                        picks.select = Some((i, egui::Modifiers::NONE));
                    }
                    if resp.middle_clicked() && e.is_dir {
                        picks.new_tab = Some(Location::Dir(dir.join(&*e.name)));
                    }
                    let tip_dir = e.is_dir.then(|| sizes.and_then(|s| s.get(&e.name)).copied()).flatten();
                    let resp = if let Some((bytes, files)) = tip_dir {
                        resp.on_hover_text_at_pointer(format!("{}: {} in {} files (from the last scan)", e.name, scan::human(bytes), group(files)))
                    } else {
                        resp
                    };
                    let count = if selected { t.selected.len() } else { 1 };
                    let path = dir.join(&*e.name);
                    let is_dir = e.is_dir;
                    let pinned = self.files.is_pinned(&path);
                    resp.context_menu(|ui| {
                        if let Some(a) = item_menu(ui, is_dir, count, pinned) {
                            let paths = if selected { self.files.chosen_paths() } else { vec![path.clone()] };
                            picks.menu = Some((a, paths));
                        }
                    });
                }
            });
            let _ = hovered_row;
            let t = &mut self.files.tabs[self.files.active];
            t.view = (out.state.offset.y, out.inner_rect.height());
            t.scroll_to_cursor = false;
        }
        if empty.clicked() {
            picks.clear = true;
        }
        let here = dir.to_path_buf();
        let pinned = self.files.is_pinned(&here);
        empty.context_menu(|ui| {
            if ui.button("Refresh").clicked() {
                picks.menu = Some((FAction::Open, Vec::new()));
            }
            if ui.button("Open in New Tab").clicked() {
                picks.new_tab = Some(Location::Dir(here.clone()));
            }
            ui.separator();
            if let Some(a) = folder_menu(ui, pinned) {
                picks.menu = Some((a, vec![here.clone()]));
            }
        });
        self.files_footer(ui, footer);
        ui.allocate_rect(Rect::from_min_max(body.min, footer.max), Sense::hover());
    }

    /// Count of items, what's selected, and how the listing is going.
    fn files_footer(&self, ui: &egui::Ui, rect: Rect) {
        let p = ui.painter();
        let pal = style::pal(ui);
        p.rect_filled(rect, 0.0, pal.paper);
        p.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, pal.hairline));
        let t = &self.files.tabs[self.files.active];
        let Some(l) = t.listing.as_ref() else { return };
        let shown = t.order.len();
        let mut text = format!("{} item{}", group(shown as u64), if shown == 1 { "" } else { "s" });
        // Only once the order covers everything read so far, or the count would be off.
        let current = t.order_key.as_ref().is_some_and(|k| k.0 == l.generation);
        let left_out = if current { l.entries.len() - shown } else { 0 };
        if left_out > 0 {
            let why = if self.files.filter.trim().is_empty() { "hidden" } else { "filtered out" };
            text.push_str(&format!(" ({} {why})", group(left_out as u64)));
        }
        if !l.done {
            text.push_str(" · reading…");
        }
        let sel: Vec<&Entry> = t.order.iter().map(|&i| &l.entries[i as usize]).filter(|e| t.selected.contains(&e.name)).collect();
        if !sel.is_empty() {
            let bytes: u64 = sel.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
            let dirs = sel.iter().filter(|e| e.is_dir).count();
            text.push_str(&format!("  ·  {} selected", group(sel.len() as u64)));
            if sel.len() > dirs {
                text.push_str(&format!(" ({}{})", scan::human(bytes), if dirs > 0 { " in files" } else { "" }));
            }
        }
        if let Some(e) = &l.error {
            text.push_str(&format!("  ·  {e}"));
        }
        p.text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), pal.weak);
    }

    /// The status bar: notices on the left, the folder, and background activity on the right.
    fn files_status(&mut self, ui: &mut egui::Ui) {
        let pal = style::pal(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some((text, error)) = self.ops.notice.clone() {
                if error {
                    ui.colored_label(ui.visuals().error_fg_color, text);
                } else {
                    ui.label(text);
                }
                if ui.small_button("✖").on_hover_text("Dismiss").clicked() {
                    self.ops.notice = None;
                }
            } else if let Some(dir) = self.files.here().dir() {
                ui.label(egui::RichText::new(dir.display().to_string()).size(12.0).color(pal.weak));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.activity_chip(ui);
            });
        });
    }

    // ---------- input ----------

    fn files_keys(&mut self, ctx: &egui::Context, pointer_in: bool, picks: &mut FPicks) {
        use egui::{Key, Modifiers as M};
        if self.ops.modal() {
            return;
        }
        // The mouse's side buttons go to the file manager when the pointer is over it.
        let mouse = pointer_in && !self.help.hovered;
        let side = ctx.input(|i| {
            if mouse && i.pointer.button_pressed(egui::PointerButton::Extra1) {
                Some(false)
            } else if mouse && i.pointer.button_pressed(egui::PointerButton::Extra2) {
                Some(true)
            } else {
                None
            }
        });
        match side {
            Some(false) => return self.files.back(),
            Some(true) => return self.files.forward(),
            None => {}
        }
        // Shortcuts that work even while typing in the filter box.
        let (find, path, dock) = ctx.input_mut(|i| (i.consume_key(M::COMMAND, Key::F), i.consume_key(M::COMMAND, Key::L), i.consume_key(M::COMMAND, Key::J)));
        if dock {
            self.files.prefs.dock.open = !self.files.prefs.dock.open;
        }
        if find {
            self.files.focus_filter = true;
        }
        if path {
            self.start_path_edit();
        }
        if ctx.egui_wants_keyboard_input() || !self.files.focused {
            return;
        }
        enum Do {
            Move(i64),
            To(i64),
            Page(i64),
            Open,
            Up,
            Back,
            Forward,
            All,
            Clear,
            NewTab,
            CloseTab,
            Reopen,
            NextTab(bool),
            Refresh,
            Hidden,
            CopyPath,
            Find(String),
        }
        let act = ctx.input_mut(|i| {
            if i.consume_key(M::ALT, Key::ArrowLeft) {
                return Some(Do::Back);
            }
            if i.consume_key(M::ALT, Key::ArrowRight) {
                return Some(Do::Forward);
            }
            if i.consume_key(M::ALT, Key::ArrowUp) || i.consume_key(M::COMMAND, Key::ArrowUp) || i.consume_key(M::NONE, Key::Backspace) {
                return Some(Do::Up);
            }
            if i.consume_key(M::COMMAND, Key::ArrowDown) || i.consume_key(M::NONE, Key::Enter) {
                return Some(Do::Open);
            }
            if i.consume_key(M::COMMAND, Key::A) {
                return Some(Do::All);
            }
            if i.consume_key(M::NONE, Key::Escape) {
                return Some(Do::Clear);
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::T) {
                return Some(Do::Reopen);
            }
            if i.consume_key(M::COMMAND, Key::T) {
                return Some(Do::NewTab);
            }
            if i.consume_key(M::COMMAND, Key::W) {
                return Some(Do::CloseTab);
            }
            if i.consume_key(M::CTRL | M::SHIFT, Key::Tab) {
                return Some(Do::NextTab(false));
            }
            if i.consume_key(M::CTRL, Key::Tab) {
                return Some(Do::NextTab(true));
            }
            if i.consume_key(M::NONE, Key::F5) || i.consume_key(M::COMMAND, Key::R) {
                return Some(Do::Refresh);
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::Period) {
                return Some(Do::Hidden);
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::C) {
                return Some(Do::CopyPath);
            }
            let shift = i.modifiers.shift;
            let m = if shift { M::SHIFT } else { M::NONE };
            if i.consume_key(m, Key::ArrowDown) {
                return Some(Do::Move(1));
            }
            if i.consume_key(m, Key::ArrowUp) {
                return Some(Do::Move(-1));
            }
            if i.consume_key(m, Key::Home) {
                return Some(Do::To(0));
            }
            if i.consume_key(m, Key::End) {
                return Some(Do::To(i64::MAX));
            }
            if i.consume_key(m, Key::PageDown) {
                return Some(Do::Page(1));
            }
            if i.consume_key(m, Key::PageUp) {
                return Some(Do::Page(-1));
            }
            // Type-to-select: plain letters and digits, no modifiers.
            if !i.modifiers.command && !i.modifiers.alt && !i.modifiers.ctrl {
                let typed: String = i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect();
                if !typed.is_empty() && !typed.chars().all(char::is_whitespace) {
                    return Some(Do::Find(typed));
                }
            }
            None
        });
        let Some(act) = act else { return };
        let shift = ctx.input(|i| i.modifiers.shift);
        let now = ctx.input(|i| i.time);
        let page = (((self.files.tabs[self.files.active].view.1 / self.files.prefs.density.row()) as i64) - 1).max(1);
        match act {
            Do::Back => self.files.back(),
            Do::Forward => self.files.forward(),
            Do::Up => self.files.up(),
            Do::Open => {
                let t = &self.files.tabs[self.files.active];
                if let Some(i) = t.cursor.as_deref().and_then(|c| t.pos_of(c)).map(|r| t.order[r]) {
                    picks.open = Some(i);
                }
            }
            Do::All => {
                let t = self.files.tab();
                if let Some(l) = &t.listing {
                    t.selected = t.order.iter().map(|&i| l.entries[i as usize].name.clone()).collect();
                }
            }
            Do::Clear => self.files.tab().selected.clear(),
            Do::NewTab => picks.new_tab = Some(self.files.here().clone()),
            Do::CloseTab => picks.close_tab = Some(self.files.active),
            Do::Reopen => {
                if let Some(l) = self.files.closed.pop() {
                    self.files.new_tab(l);
                }
            }
            Do::NextTab(fwd) => {
                let n = self.files.tabs.len();
                let i = if fwd { (self.files.active + 1) % n } else { (self.files.active + n - 1) % n };
                picks.switch_tab = Some(i);
            }
            Do::Refresh => self.files.refresh(),
            Do::Hidden => self.files.prefs.show_hidden = !self.files.prefs.show_hidden,
            Do::CopyPath => {
                let paths = self.files.chosen_paths();
                if !paths.is_empty() {
                    picks.menu = Some((FAction::CopyPath, paths));
                }
            }
            Do::Move(d) => self.files_move_cursor(|pos, _| pos.map(|p| p as i64 + d).unwrap_or(0), shift),
            Do::To(x) => self.files_move_cursor(|_, _| x, shift),
            Do::Page(d) => self.files_move_cursor(|pos, _| pos.map(|p| p as i64 + d * page).unwrap_or(0), shift),
            Do::Find(s) => {
                if now - self.files.typed_at > 1.0 {
                    self.files.typed.clear();
                }
                self.files.typed_at = now;
                self.files.typed.push_str(&s.to_lowercase());
                let typed = self.files.typed.clone();
                let t = &self.files.tabs[self.files.active];
                let found = t.listing.as_ref().and_then(|l| t.order.iter().position(|&i| l.entries[i as usize].name.to_lowercase().starts_with(&typed)));
                if let Some(r) = found {
                    self.files_move_cursor(|_, _| r as i64, false);
                }
            }
        }
    }

    /// Move the keyboard cursor to the row `to(current row, row count)` returns, selecting it
    /// (or, with Shift, everything from the anchor to it).
    fn files_move_cursor(&mut self, to: impl Fn(Option<usize>, usize) -> i64, extend: bool) {
        let t = self.files.tab();
        let Some(l) = t.listing.as_ref() else { return };
        let n = t.order.len();
        if n == 0 {
            return;
        }
        let pos = t.cursor.as_deref().and_then(|c| t.pos_of(c));
        let r = to(pos, n).clamp(0, n as i64 - 1) as usize;
        let name = l.entries[t.order[r] as usize].name.clone();
        if extend {
            let a = t.anchor.as_deref().and_then(|a| t.pos_of(a)).or(pos).unwrap_or(r);
            let (lo, hi) = (a.min(r), a.max(r));
            t.selected = t.order[lo..=hi].iter().map(|&i| l.entries[i as usize].name.clone()).collect();
            if t.anchor.is_none() {
                t.anchor = pos.map(|p| l.entries[t.order[p] as usize].name.clone());
            }
        } else {
            t.selected = std::iter::once(name.clone()).collect();
            t.anchor = Some(name.clone());
        }
        t.cursor = Some(name);
        t.scroll_to_cursor = true;
    }

    fn files_apply(&mut self, picks: FPicks) {
        if let Some(k) = picks.sort {
            let p = &mut self.files.prefs;
            if p.sort == k {
                p.reversed = !p.reversed;
            } else {
                p.sort = k;
                p.reversed = false;
            }
        }
        if picks.clear {
            let t = self.files.tab();
            if t.listing.as_ref().is_some_and(|l| !l.done) && t.order.is_empty() {
                // Cancel a listing that's taking too long: go back where we were.
                if !self.files.tab().hist.back() {
                    let up = self.files.here().up();
                    if let Some(u) = up {
                        self.files.tab().hist.go(u);
                    }
                }
                self.files.after_move();
            } else {
                t.selected.clear();
            }
        }
        if let Some((i, m)) = picks.select {
            let t = self.files.tab();
            if let Some(name) = t.entry(i).map(|e| e.name.clone()) {
                if m.command {
                    if !t.selected.remove(&name) {
                        t.selected.insert(name.clone());
                    }
                    t.anchor = Some(name.clone());
                    t.cursor = Some(name);
                } else if m.shift && t.anchor.is_some() {
                    let a = t.anchor.as_deref().and_then(|a| t.pos_of(a));
                    let b = t.pos_of(&name);
                    if let (Some(a), Some(b), Some(l)) = (a, b, t.listing.as_ref()) {
                        let (lo, hi) = (a.min(b), a.max(b));
                        t.selected = t.order[lo..=hi].iter().map(|&i| l.entries[i as usize].name.clone()).collect();
                    }
                    t.cursor = Some(name);
                } else {
                    t.selected = std::iter::once(name.clone()).collect();
                    t.anchor = Some(name.clone());
                    t.cursor = Some(name);
                }
            }
        }
        if let Some(i) = picks.open {
            let t = &self.files.tabs[self.files.active];
            if let (Some(e), Some(dir)) = (t.entry(i), t.here().dir()) {
                let path = dir.join(&*e.name);
                if e.is_dir {
                    self.files.go(Location::Dir(path));
                } else {
                    self.files.spawn(move || actions::open(&path));
                }
            }
        }
        if let Some(to) = picks.go {
            self.files.go(to);
        }
        if let Some((a, paths)) = picks.menu {
            self.files_action(a, paths);
        }
        if let Some(l) = picks.new_tab {
            self.files.new_tab(l);
        }
        if let Some(i) = picks.close_tab {
            self.files.close_tab(i);
        }
        if let Some(i) = picks.close_others {
            self.files.close_others(i);
        }
        if let Some((i, j)) = picks.move_tab {
            self.files.move_tab(i, j);
        }
        if let Some(i) = picks.switch_tab {
            self.files.switch_tab(i);
        }
        if let Some(p) = picks.unpin {
            self.files.unpin(&p);
        }
    }

    /// The list's item menu for a tile on the dock's map, run through the same commands.
    pub(super) fn tile_menu(&mut self, ui: &mut egui::Ui, disk: &Path, is_dir: bool) {
        let pinned = self.files.is_pinned(disk);
        let Some(a) = item_menu(ui, is_dir, 1, pinned) else { return };
        match a {
            FAction::Open if is_dir => self.files.go(Location::Dir(disk.to_path_buf())),
            FAction::Open => {
                let p = disk.to_path_buf();
                self.files.spawn(move || actions::open(&p));
            }
            a => self.files_action(a, vec![disk.to_path_buf()]),
        }
        ui.close();
    }

    fn files_action(&mut self, a: FAction, paths: Vec<PathBuf>) {
        match a {
            FAction::Open if paths.is_empty() => self.files.refresh(),
            FAction::Open => {
                // Folders open here (the first one); files open in their apps.
                let mut went = false;
                for p in paths {
                    let t = &self.files.tabs[self.files.active];
                    let is_dir = t.listing.as_ref().and_then(|l| p.file_name().and_then(|n| l.entries.iter().find(|e| *e.name == *n.to_string_lossy()))).is_some_and(|e| e.is_dir);
                    if is_dir || p.parent().is_none() {
                        if !went {
                            self.files.go(Location::Dir(p));
                            went = true;
                        }
                    } else {
                        self.files.spawn(move || actions::open(&p));
                    }
                }
            }
            FAction::OpenInNewTab => {
                for p in paths {
                    self.files.new_tab(Location::Dir(p));
                }
            }
            FAction::Pin => {
                for p in paths {
                    self.files.pin(p);
                }
            }
            FAction::Unpin => {
                for p in paths {
                    self.files.unpin(&p);
                }
            }
            FAction::CopyPath => self.ctx_copy = Some(paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")),
            FAction::CopyName => {
                self.ctx_copy = Some(paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).collect::<Vec<_>>().join("\n"))
            }
            FAction::Reveal => {
                if let Some(p) = paths.into_iter().next() {
                    self.files.spawn(move || actions::reveal(&p));
                }
            }
            FAction::Terminal(which) => {
                if let Some(p) = paths.into_iter().next() {
                    self.files.spawn(move || actions::terminal(&p, which));
                }
            }
            FAction::Analyse => {
                if let Some(p) = paths.into_iter().next() {
                    self.analyse(p);
                }
            }
        }
    }
}

/// The menu for an item in the list.
fn item_menu(ui: &mut egui::Ui, is_dir: bool, count: usize, pinned: bool) -> Option<FAction> {
    let ctx = ui.ctx().clone();
    let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
    let mut out = None;
    let many = count > 1;
    let open = if many { format!("Open {count} Items") } else { "Open".to_string() };
    if ui.add(egui::Button::new(open).shortcut_text(k(egui::Modifiers::NONE, egui::Key::Enter))).clicked() {
        out = Some(FAction::Open);
    }
    if is_dir && ui.button(if many { "Open in New Tabs" } else { "Open in New Tab" }).clicked() {
        out = Some(FAction::OpenInNewTab);
    }
    ui.separator();
    if ui.add(egui::Button::new(if many { "Copy Paths" } else { "Copy Path" }).shortcut_text(k(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::C))).clicked() {
        out = Some(FAction::CopyPath);
    }
    if ui.button(if many { "Copy Names" } else { "Copy Name" }).clicked() {
        out = Some(FAction::CopyName);
    }
    if !many {
        let reveal = if cfg!(windows) {
            "Show in Explorer"
        } else if cfg!(target_os = "macos") {
            "Show in Finder"
        } else {
            "Show in System File Manager"
        };
        if ui.button(reveal).clicked() {
            out = Some(FAction::Reveal);
        }
        for (label, which) in actions::terminals() {
            if ui.button(*label).clicked() {
                out = Some(FAction::Terminal(*which));
            }
        }
    }
    if is_dir && !many {
        ui.separator();
        if let Some(a) = folder_menu(ui, pinned) {
            out = Some(a);
        }
    }
    out
}

/// Pin / Unpin and Analyse, for a folder.
fn folder_menu(ui: &mut egui::Ui, pinned: bool) -> Option<FAction> {
    let mut out = None;
    if pinned {
        if ui.button("Unpin from Sidebar").clicked() {
            out = Some(FAction::Unpin);
        }
    } else if ui.button("Pin to Sidebar").clicked() {
        out = Some(FAction::Pin);
    }
    if ui.button("Analyse This Folder").on_hover_text("Scan it and show its treemap in the analysis dock, while you keep browsing").clicked() {
        out = Some(FAction::Analyse);
    }
    out
}

/// The menu for a place in the sidebar.
fn place_menu(ui: &mut egui::Ui, path: &Path, pinned: bool, picks: &mut FPicks) {
    if ui.button("Open in New Tab").clicked() {
        picks.new_tab = Some(Location::Dir(path.to_path_buf()));
    }
    if pinned {
        if ui.button("Unpin from Sidebar").clicked() {
            picks.unpin = Some(path.to_path_buf());
        }
    } else if ui.button("Pin to Sidebar").clicked() {
        picks.menu = Some((FAction::Pin, vec![path.to_path_buf()]));
    }
    if ui.button("Copy Path").clicked() {
        picks.menu = Some((FAction::CopyPath, vec![path.to_path_buf()]));
    }
    if ui.button("Analyse This Folder").clicked() {
        picks.menu = Some((FAction::Analyse, vec![path.to_path_buf()]));
    }
}

/// A sidebar line: icon and label, left-aligned. Where we are gets the selection tint and an
/// accent bar at its left edge.
fn side_row(ui: &mut egui::Ui, icon: &str, label: &str, on: bool, h: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let pal = style::pal(ui);
    let p = ui.painter();
    if on {
        p.rect_filled(r, style::RADIUS as f32, pal.sel);
        p.rect_filled(Rect::from_min_size(pos2(r.left(), r.top() + 5.0), vec2(3.0, ROW_F - 10.0)), 1.5, pal.accent);
    } else if resp.hovered() {
        p.rect_filled(r, style::RADIUS as f32, pal.hover);
    }
    let color = if on { pal.strong } else { pal.text };
    let cy = r.top() + ROW_F / 2.0;
    p.text(pos2(r.left() + 11.0, cy), Align2::LEFT_CENTER, icon, FontId::proportional(13.0), if on { pal.accent } else { pal.weak });
    let clip = Rect::from_min_max(pos2(r.left() + 31.0, r.top()), pos2(r.right() - 4.0, r.bottom()));
    p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(clip.left(), cy), Align2::LEFT_CENTER, label, FontId::proportional(13.0), color);
    resp
}

/// A sidebar group: a small-caps title that folds the group away when clicked.
fn side_section(ui: &mut egui::Ui, salt: &str, title: &str, default_open: bool, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.id().with(salt);
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(default_open);
    ui.add_space(6.0);
    let resp = ui
        .horizontal(|ui| {
            ui.add_space(6.0);
            let t = style::section_title(ui, title);
            let arrow = ui.add(egui::Label::new(egui::RichText::new(if open { "⏷" } else { "⏵" }).size(9.0).color(style::pal(ui).weak)).sense(Sense::click()));
            t.union(arrow)
        })
        .inner;
    if resp.on_hover_text(if open { "Click to fold away" } else { "Click to show" }).clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    ui.add_space(2.0);
    if open {
        add(ui);
    }
}

/// "C: (Windows)", "/home", "Backup (/media/rich/Backup)".
fn drive_label(root: &Path, label: &str) -> String {
    let r = root.display().to_string();
    if cfg!(windows) {
        let r = r.trim_end_matches('\\');
        if label.is_empty() { r.to_string() } else { format!("{label} ({r})") }
    } else if label.is_empty() || label.len() > 12 || !label.chars().next().is_some_and(char::is_uppercase) {
        // On Linux the label is the file system type (ext4, nfs…): the path says more.
        r
    } else {
        format!("{label} ({r})")
    }
}

/// A tab's title: the folder's name, or the drive.
fn location_title(l: &Location) -> String {
    match l {
        Location::Computer => places::computer_name().into(),
        Location::Dir(p) => p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string()),
    }
}

/// The breadcrumbs for a place: (label, where it goes).
pub(super) fn crumbs_of(l: &Location) -> Vec<(String, Location)> {
    let mut out = vec![(format!("🖳 {}", places::computer_name()), Location::Computer)];
    let Location::Dir(p) = l else { return out };
    let mut acc = PathBuf::new();
    for c in p.components() {
        use std::path::Component as C;
        match c {
            C::Prefix(_) => {
                acc.push(c.as_os_str());
                continue;
            }
            C::RootDir => {
                acc.push(c.as_os_str());
                let label = if acc.as_os_str().len() <= 1 { "/".to_string() } else { acc.display().to_string().trim_end_matches('\\').to_string() };
                out.push((format!("🖴 {label}"), Location::Dir(acc.clone())));
            }
            C::Normal(n) => {
                acc.push(n);
                out.push((n.to_string_lossy().to_string(), Location::Dir(acc.clone())));
            }
            C::CurDir | C::ParentDir => acc.push(c.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, is_dir: bool, size: u64, modified: i64, hidden: bool) -> Entry {
        Entry { name: name.into(), is_dir, is_link: false, hidden, size, modified }
    }

    fn names(es: &[Entry], o: &[u32]) -> Vec<String> {
        o.iter().map(|&i| es[i as usize].name.to_string()).collect()
    }

    #[test]
    fn folders_first_then_natural_names() {
        let es = vec![e("b10.txt", false, 5, 1, false), e("src", true, 0, 3, false), e("b2.txt", false, 50, 2, false), e(".git", true, 0, 4, true), e("A", true, 0, 1, false)];
        let o = order_entries(&es, None, FSort::Name, false, false, "");
        assert_eq!(names(&es, &o), ["A", "src", "b2.txt", "b10.txt"]);
        let o = order_entries(&es, None, FSort::Name, false, true, "");
        assert_eq!(names(&es, &o), [".git", "A", "src", "b2.txt", "b10.txt"]);
        let o = order_entries(&es, None, FSort::Name, true, false, "");
        assert_eq!(names(&es, &o), ["src", "A", "b10.txt", "b2.txt"]);
        let o = order_entries(&es, None, FSort::Size, false, false, "");
        assert_eq!(names(&es, &o), ["A", "src", "b2.txt", "b10.txt"]);
        let o = order_entries(&es, None, FSort::Modified, false, false, "");
        assert_eq!(names(&es, &o), ["src", "A", "b2.txt", "b10.txt"]);
    }

    #[test]
    fn scanned_folder_sizes_sort_folders() {
        let es = vec![e("small", true, 0, 0, false), e("big", true, 0, 0, false), e("f", false, 1, 0, false)];
        let sizes: HashMap<Box<str>, (u64, u64)> = [("small".into(), (10, 1)), ("big".into(), (1000, 5))].into_iter().collect();
        let o = order_entries(&es, Some(&sizes), FSort::Size, false, false, "");
        assert_eq!(names(&es, &o), ["big", "small", "f"]);
    }

    #[test]
    fn filter_box() {
        assert!(name_matches("", "x"));
        assert!(name_matches("REN", "render_01.exr"));
        assert!(name_matches("*.exr", "render_01.EXR"));
        assert!(!name_matches("*.exr", "render_01.exr.bak"));
        assert!(name_matches("shot_??.png", "shot_01.png"));
        assert!(!name_matches("abc", "xyz"));
    }

    /// `DIRMAP_BENCH_DIR=/some/big/folder cargo test --release bench_listing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_listing() {
        let Some(dir) = std::env::var_os("DIRMAP_BENCH_DIR") else { return };
        let t = Instant::now();
        let mut l = Listing::start(PathBuf::from(dir));
        let mut first = None;
        while !l.done {
            l.poll();
            if first.is_none() && !l.entries.is_empty() {
                first = Some(t.elapsed());
            }
        }
        let all = t.elapsed();
        let t = Instant::now();
        let o = order_entries(&l.entries, None, FSort::Name, false, true, "");
        let by_name = t.elapsed();
        let t = Instant::now();
        let _ = order_entries(&l.entries, None, FSort::Size, false, true, "");
        let by_size = t.elapsed();
        println!("{} entries: first batch {:?}, all {:?}; sort by name {:?}, by size {:?}", o.len(), first.unwrap_or_default(), all, by_name, by_size);
    }

    #[test]
    fn typed_paths() {
        assert_eq!(parse_typed_path("  "), None);
        assert_eq!(parse_typed_path(places::computer_name()), Some(Location::Computer));
        assert_eq!(parse_typed_path("\"/tmp/x\""), Some(Location::Dir(PathBuf::from("/tmp/x"))));
        if let Some(h) = places::home() {
            assert_eq!(parse_typed_path("~"), Some(Location::Dir(h.join(""))));
            assert_eq!(parse_typed_path("~/Docs"), Some(Location::Dir(h.join("Docs"))));
        }
        if cfg!(windows) {
            assert_eq!(parse_typed_path("d:"), Some(Location::Dir(PathBuf::from("d:\\"))));
        }
    }

    #[test]
    fn breadcrumbs() {
        let c = crumbs_of(&Location::Computer);
        assert_eq!(c.len(), 1);
        if cfg!(unix) {
            let c = crumbs_of(&Location::Dir(PathBuf::from("/home/rich/Projects")));
            let labels: Vec<&str> = c.iter().map(|c| c.0.as_str()).collect();
            assert_eq!(labels[1..], ["🖴 /", "home", "rich", "Projects"]);
            assert_eq!(c[3].1, Location::Dir(PathBuf::from("/home/rich")));
        }
        if cfg!(windows) {
            let c = crumbs_of(&Location::Dir(PathBuf::from(r"C:\Users\rich")));
            let labels: Vec<&str> = c.iter().map(|c| c.0.as_str()).collect();
            assert_eq!(labels[1..], ["🖴 C:", "Users", "rich"]);
            assert_eq!(c[1].1, Location::Dir(PathBuf::from(r"C:\")));
            let c = crumbs_of(&Location::Dir(PathBuf::from(r"\\server\share\x")));
            assert_eq!(c.last().unwrap().1, Location::Dir(PathBuf::from(r"\\server\share\x")));
        }
    }
}
