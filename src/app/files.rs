//! The File Manager workspace: browse the computer like Explorer, Finder or Dolphin, straight
//! from the disk, with no scan needed. A sidebar of places and drives, tabs with their own
//! history, a path bar you can click or type in, and a sortable Details list.
//!
//! Folders are read by `fsys::listing` on worker threads, so nothing here waits on the disk or
//! the network. File commands use the same `actions` functions as the Storage Analysis
//! workspace; when a scan covers the folder on screen, its folder sizes are shown too.

use crate::fsys::PathShow;
use super::*;
use crate::actions;
use crate::fsys::history::{History, Location};
use crate::fsys::listing::{Entry, Listing};
use crate::fsys::archive;
use crate::fsys::remote;
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
    /// Servers: the File Manager with its own panes (this computer beside a server), saved
    /// sites and quick connect (`remoteui.rs`).
    Remote,
    /// The AI models on this computer (`models.rs`).
    Models,
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

/// A search kept in the sidebar: what to look for, in a folder and below.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub(super) struct SavedSearch {
    pub name: String,
    pub dir: PathBuf,
    pub query: String,
}

/// What's remembered between runs.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct FilesPrefs {
    pub workspace: Workspace,
    pub tabs: Vec<Location>,
    pub active: usize,
    /// Folders pinned to the sidebar before Favourites had groups; moved into the first group
    /// on load.
    pub pinned: Vec<PathBuf>,
    /// The Favourites section: the user's groups of bookmarked folders.
    pub favourites: Vec<favs::FavGroup>,
    /// Saved network locations (UNC paths, mount points) shown under Network & Remote.
    pub servers: Vec<PathBuf>,
    /// Saved SFTP, FTP and WebDAV servers, under Network & Remote.
    pub sites: Vec<netui::SavedSite>,
    /// Show numbered frames (`shot.1001.exr`…) as one row per sequence.
    pub sequences: bool,
    /// Searches saved in the sidebar.
    pub searches: Vec<SavedSearch>,
    /// Extra folders the AI Models workspace looks in (besides Ollama, LM Studio, Hugging Face
    /// and Downloads).
    pub model_folders: Vec<PathBuf>,
    /// Programs offered under Open With.
    pub apps: Vec<openwith::UserApp>,
    /// The sequence player's settings.
    pub player: seqplayer::PlayerPrefs,
    /// Folders opened most recently, newest first.
    pub recent: Vec<PathBuf>,
    /// Server folders opened most recently, one per server, newest first (Remote sidebar).
    pub recent_servers: Vec<PathBuf>,
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
    /// One pane or two, and how the space is shared (the first pane's fraction).
    pub split: Split,
    /// The split used last, for Ctrl+\ to bring back.
    pub last_split: Split,
    pub split_ratio: f32,
    /// The second pane's tabs, kept while it's hidden too.
    pub second: PaneSave,
    /// The preview pane at the right.
    pub preview: bool,
    pub preview_width: f32,
    /// The preview pane beside the map in Storage Analysis.
    pub map_preview: bool,
    /// The first pane's view (the second pane's is in `second`), and the Icons view's size.
    pub view: FView,
    pub icon_size: f32,
    /// Remember each folder's view, sort and grouping.
    pub remember_looks: bool,
    /// Folders shown differently from the default, most recently changed first.
    pub folder_looks: Vec<(PathBuf, FolderLook)>,
    /// How folders without one of their own are shown.
    pub base_look: Option<FolderLook>,
    /// The Remote workspace's panes and layout (the Files workspace's are the fields above).
    pub remote_desk: super::remoteui::DeskSave,
}

/// How a folder is shown: its view, sort and grouping.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct FolderLook {
    pub view: FView,
    pub sort: FSort,
    pub reversed: bool,
    pub group: views::FGroup,
}

/// Folders whose looks are remembered, at most.
const LOOKS_MAX: usize = 500;

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
            favourites: Vec::new(),
            servers: Vec::new(),
            sites: Vec::new(),
            sequences: true,
            searches: Vec::new(),
            model_folders: Vec::new(),
            apps: Vec::new(),
            player: Default::default(),
            recent: Vec::new(),
            recent_servers: Vec::new(),
            sort: FSort::Name,
            reversed: false,
            show_hidden: false,
            sidebar_width: 210.0,
            density: Density::Comfortable,
            col_widths: COL_WIDTHS,
            dock: Default::default(),
            split: Split::Single,
            last_split: Split::Vertical,
            split_ratio: 0.5,
            second: PaneSave::default(),
            preview: false,
            preview_width: 300.0,
            map_preview: true,
            view: FView::Details,
            icon_size: 96.0,
            remember_looks: true,
            folder_looks: Vec::new(),
            base_look: None,
            remote_desk: Default::default(),
        }
    }
}

const RECENT_MAX: usize = 10;
/// Sidebar rows (the list's rows follow the density setting).
pub(super) const ROW_F: f32 = 24.0;
const TAB_STRIP_H: f32 = 34.0;
const TAB_MIN_W: f32 = 110.0;
const TAB_MAX_W: f32 = 210.0;
/// The analysis dock and the list above it never get smaller than these.
pub(super) const DOCK_MIN: f32 = 140.0;
pub(super) const DOCK_MIN_LIST: f32 = 110.0;

/// Folder name → (bytes, files) from the scan.
pub(super) type FolderSizes = HashMap<Box<str>, (u64, u64)>;

/// What an order was worked out for: (listing generation, sort, reversed, hidden, filter, sizes).
type OrderKey = (u64, FSort, bool, bool, String, usize, (views::FGroup, bool));
/// A display order, its group headings and its sequence rows.
type Arranged = (Vec<u32>, Vec<views::GroupHead>, views::Seqs);
/// A folder read again, arranged, with what for.
type Rearranged = (OrderKey, Arranged);

/// Folders with more items than this are sorted on another thread, so the window never stalls.
const SORT_INLINE_MAX: usize = 3_000;

/// One tab: where it is, where it's been, what's in it and what's selected there.
pub(super) struct FTab {
    pub(super) hist: History,
    pub(super) listing: Option<Listing>,
    /// Indices into the listing's entries, in display order.
    pub(super) order: Vec<u32>,
    /// What `order` was worked out for: (listing generation, sort, reversed, hidden, filter, sizes).
    order_key: Option<OrderKey>,
    /// A big folder being sorted on another thread: what for, how many entries it covers, and
    /// where the result arrives.
    sorting: Option<(OrderKey, usize, mpsc::Receiver<Arranged>)>,
    pub(super) selected: BTreeSet<Box<str>>,
    /// The item the keyboard moves from (the last one clicked or moved to).
    pub(super) cursor: Option<Box<str>>,
    /// Where Shift-selection ranges start.
    pub(super) anchor: Option<Box<str>>,
    /// Select this item once it's listed (coming up from a folder, or from another workspace).
    pub(super) select_later: Option<Box<str>>,
    /// …and open the Rename box on it (a new folder or file).
    pub(super) rename_later: bool,
    pub(super) scroll_to_cursor: bool,
    /// (scroll offset, height) of the list last frame.
    pub(super) view: (f32, f32),
    /// Numbered frames shown as one row each, by the row's entry.
    pub(super) seqs: views::Seqs,
    /// Group headings, when the pane groups items.
    pub(super) groups: Vec<views::GroupHead>,
    /// Another program changed the folder (at this time): read it again shortly.
    dirty_at: Option<f64>,
    /// The folder being read again after a change, shown in place of the old listing once it's
    /// read and sorted (so the list doesn't blank or jump).
    fresh: Option<(Listing, Option<mpsc::Receiver<Rearranged>>)>,
    /// Folder sizes from the scan for this folder: (tree it came from, name → (bytes, files)).
    pub(super) sizes: Option<(usize, FolderSizes)>,
    /// The listing is a search of this folder and below: (what was searched for, hidden
    /// folders searched too).
    pub(super) search: Option<(String, bool)>,
    /// …and whether that search went into subfolders.
    pub(super) search_deep: bool,
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
            rename_later: false,
            search: None,
            search_deep: false,
            scroll_to_cursor: false,
            view: (0.0, 0.0),
            groups: Vec::new(),
            seqs: Default::default(),
            dirty_at: None,
            fresh: None,
            sizes: None,
        }
    }

    pub(super) fn here(&self) -> &Location {
        &self.hist.here
    }

    /// Forget what was shown, after moving somewhere else.
    fn arrived(&mut self) {
        self.listing = None;
        self.order.clear();
        self.groups.clear();
        self.dirty_at = None;
        self.fresh = None;
        self.order_key = None;
        self.sorting = None;
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.sizes = None;
        self.scroll_to_cursor = true;
        self.view = (0.0, 0.0);
    }

    /// Read the folder again, keeping the selection and the cursor.
    pub(super) fn reread(&mut self) {
        let cursor = self.cursor.clone();
        self.fresh = None;
        self.dirty_at = None;
        self.listing = None;
        self.order_key = None;
        self.sizes = None;
        if self.select_later.is_none() {
            self.select_later = cursor;
        }
    }

    fn entry(&self, i: u32) -> Option<&Entry> {
        self.listing.as_ref()?.entries.get(i as usize)
    }

    pub(super) fn entry_named(&self, name: &str) -> Option<&Entry> {
        self.listing.as_ref()?.entries.iter().find(|e| &*e.name == name)
    }

    pub(super) fn pos_of(&self, name: &str) -> Option<usize> {
        let l = self.listing.as_ref()?;
        self.order.iter().position(|&i| &*l.entries[i as usize].name == name)
    }

    /// Paths of the selected items, in display order.
    pub(super) fn chosen_paths(&self) -> Vec<PathBuf> {
        let (Some(dir), Some(l)) = (self.here().dir(), self.listing.as_ref()) else { return Vec::new() };
        let mut out = Vec::new();
        for &i in &self.order {
            let e = &l.entries[i as usize];
            if !self.selected.contains(&e.name) {
                continue;
            }
            out.extend(self.row_paths(dir, l, i));
        }
        out
    }

    /// A sequence row, ready for the player.
    pub(super) fn clip_of(&self, i: u32) -> Option<crate::player::Clip> {
        let (dir, l, s) = (self.here().dir()?, self.listing.as_ref()?, self.seqs.get(&i)?);
        let names: Vec<&str> = s.members.iter().map(|&m| &*l.entries[m as usize].name).collect();
        Some(crate::player::Clip::from_names(s.label.clone(), dir, &s.seq, &names))
    }

    /// The selected sequence rows, in display order, for the player.
    pub(super) fn chosen_clips(&self) -> Vec<crate::player::Clip> {
        let Some(l) = self.listing.as_ref() else { return Vec::new() };
        self.order.iter().filter(|&&i| self.seqs.contains_key(&i) && self.selected.contains(&l.entries[i as usize].name)).filter_map(|&i| self.clip_of(i)).collect()
    }

    /// The paths a row stands for: its item, or every frame of a sequence row.
    pub(super) fn row_paths(&self, dir: &Path, l: &Listing, i: u32) -> Vec<PathBuf> {
        match self.seqs.get(&i) {
            Some(s) => s.members.iter().map(|&m| dir.join(&*l.entries[m as usize].name)).collect(),
            None => vec![dir.join(&*l.entries[i as usize].name)],
        }
    }
}

/// How the centre is divided.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(super) enum Split {
    #[default]
    Single,
    /// Two panes side by side.
    Vertical,
    /// One pane above the other.
    Horizontal,
}

/// How a pane shows a folder.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(super) enum FView {
    /// Rows with Name, Size, Type and Date modified columns.
    #[default]
    Details,
    /// A grid of big icons, with thumbnails of images.
    Icons,
    /// Finder-style columns: the folders above this one at the left, this folder, and what's
    /// in the selected folder (or a preview of the selected file) at the right.
    Columns,
    /// A big preview of the selected item above a strip of thumbnails.
    Gallery,
}

impl FView {
    pub(super) const ALL: [FView; 4] = [FView::Details, FView::Icons, FView::Columns, FView::Gallery];

    pub(super) fn title(self) -> &'static str {
        match self {
            FView::Details => "Details",
            FView::Icons => "Icons",
            FView::Columns => "Columns",
            FView::Gallery => "Gallery",
        }
    }

    pub(super) fn tip(self) -> &'static str {
        match self {
            FView::Details => "Rows with size, type and date",
            FView::Icons => "Big icons, with pictures of images",
            FView::Columns => "The folders above this one at the left, and what's in the selected folder at the right",
            FView::Gallery => "A big preview of the selected item above a strip of thumbnails",
        }
    }

    /// A small drawing of the view, for its buttons (the fonts have no glyphs for most of them).
    pub(super) fn paint(self, p: &egui::Painter, r: Rect, c: Color32) {
        let r = Rect::from_center_size(r.center(), vec2(12.0, 10.0));
        let s = Stroke::new(1.2, c);
        match self {
            FView::Details => {
                for k in 0..3 {
                    let y = r.top() + 1.0 + k as f32 * 4.0;
                    p.hline(r.left()..=r.right(), y, s);
                }
            }
            FView::Icons => {
                for (dx, dy) in [(0.0, 0.0), (7.0, 0.0), (0.0, 6.0), (7.0, 6.0)] {
                    p.rect_filled(Rect::from_min_size(r.min + vec2(dx, dy), vec2(5.0, 4.0)), 1.0, c);
                }
            }
            FView::Columns => {
                p.rect_stroke(r, 1.0, s, egui::StrokeKind::Inside);
                for k in 1..3 {
                    let x = r.left() + k as f32 * r.width() / 3.0;
                    p.vline(x, r.y_range(), s);
                }
            }
            FView::Gallery => {
                p.rect_filled(Rect::from_min_size(r.min, vec2(12.0, 6.0)), 1.0, c);
                for k in 0..3 {
                    p.rect_filled(Rect::from_min_size(r.min + vec2(k as f32 * 4.5, 7.5), vec2(3.0, 2.5)), 0.5, c);
                }
            }
        }
    }
}

/// Icon sizes in the Icons view, smallest and largest (Ctrl+wheel changes it).
pub(super) const ICON_MIN: f32 = 48.0;
pub(super) const ICON_MAX: f32 = 256.0;

/// What's remembered of the second pane.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct PaneSave {
    pub tabs: Vec<Location>,
    pub active: usize,
    pub sort: FSort,
    pub reversed: bool,
    pub view: FView,
}

/// One side of the window: its tabs, sort and filter.
pub(super) struct Pane {
    pub(super) tabs: Vec<FTab>,
    pub(super) active: usize,
    pub(super) sort: FSort,
    pub(super) reversed: bool,
    /// Show only items whose names contain this (or match it, with * or ?).
    pub(super) filter: String,
    /// The filter box searches subfolders too.
    pub(super) deep: bool,
    /// Show only these kinds of file (`categories::Category` bits); 0 for everything.
    pub(super) kinds: u16,
    pub(super) view: FView,
    /// Group items under headings (Details view).
    pub(super) group: views::FGroup,
    /// The folder shown last frame and how: a change of look in the same folder is remembered
    /// for it; arriving somewhere else brings back its own.
    pub(super) seen: Option<(Location, FolderLook)>,
    /// The Icons view's grid last frame: (icons per row, row height), for the arrow keys.
    pub(super) grid: (usize, f32),
}

impl Pane {
    pub(super) fn new(tabs: &[Location], active: usize, sort: FSort, reversed: bool, view: FView) -> Self {
        let mut tabs: Vec<FTab> = tabs.iter().cloned().map(FTab::new).collect();
        if tabs.is_empty() {
            tabs.push(FTab::new(places::home().map(Location::Dir).unwrap_or(Location::Computer)));
        }
        let active = active.min(tabs.len() - 1);
        Self { tabs, active, sort, reversed, filter: String::new(), deep: false, kinds: 0, view, group: Default::default(), seen: None, grid: (1, 100.0) }
    }

    pub(super) fn tab(&self) -> &FTab {
        &self.tabs[self.active]
    }

    pub(super) fn tab_mut(&mut self) -> &mut FTab {
        &mut self.tabs[self.active]
    }

    pub(super) fn look(&self) -> FolderLook {
        FolderLook { view: self.view, sort: self.sort, reversed: self.reversed, group: self.group }
    }

    fn set_look(&mut self, l: FolderLook) {
        (self.view, self.sort, self.reversed, self.group) = (l.view, l.sort, l.reversed, l.group);
    }

    pub(super) fn save(&self) -> PaneSave {
        PaneSave { tabs: self.tabs.iter().map(|t| t.here().clone()).collect(), active: self.active, sort: self.sort, reversed: self.reversed, view: self.view }
    }
}

/// The File Manager's state.
pub(super) struct Files {
    pub prefs: FilesPrefs,
    /// One pane, or two in split view; `focus` is the one the toolbar and keys work on.
    pub(super) panes: Vec<Pane>,
    pub(super) focus: usize,
    /// Recently closed tabs, to reopen with Ctrl+Shift+T.
    pub(super) closed: Vec<Location>,
    pub(super) places: Vec<Place>,
    places_rx: Option<mpsc::Receiver<Vec<Place>>>,
    /// The path being typed in the path bar (it shows breadcrumbs otherwise).
    pub(super) path_edit: Option<String>,
    /// The pane whose path is being typed.
    pub(super) path_pane: usize,
    pub(super) focus_path: bool,
    focus_filter: bool,
    typed: String,
    typed_at: f64,
    /// Keys go to the list (it or the sidebar was clicked last, not the filter box).
    pub(super) focused: bool,
    /// Results of commands run in the background (open, reveal, terminal).
    results: Vec<mpsc::Receiver<Result<(), String>>>,
    /// While dragging over a tab: which one (pane, tab) and since when, to switch to it.
    pub(super) drag_hover_tab: Option<((usize, usize), f64)>,
    /// A drop target under the pointer this frame already shows what a drop will do.
    pub(super) drop_claimed: bool,
    /// The user's templates for the New menu, and the background read of them.
    pub(super) user_templates: Vec<crate::templates::UserTemplate>,
    pub(super) user_templates_rx: Option<mpsc::Receiver<Vec<crate::templates::UserTemplate>>>,
    /// A favourite or group being renamed in the sidebar.
    pub(super) fav_edit: Option<favs::FavEdit>,
    /// The address bar's suggestions while typing a path.
    pub(super) suggest: addr::Suggest,
    /// Quick Look and the preview pane.
    pub(super) look: preview::Look,
    /// Where the address bar's suggestion list was drawn.
    pub(super) suggest_rect: Option<Rect>,
    /// Text worked out in the background for the clipboard (UNC paths).
    pub(super) text_rx: Option<mpsc::Receiver<String>>,
    /// Network & Remote: what the system reports, the background read, the Connect box, and
    /// connections or disconnections running.
    pub(super) net: Vec<crate::netdrives::NetPlace>,
    pub(super) net_rx: Option<mpsc::Receiver<Vec<crate::netdrives::NetPlace>>>,
    pub(super) connect: Option<netui::ConnectBox>,
    pub(super) net_jobs: Vec<mpsc::Receiver<Result<Option<PathBuf>, String>>>,
    pub(super) net_asked: bool,
    /// Questions from server connections, and the one on screen.
    pub(super) asks: Option<netui::AskRx>,
    pub(super) asking: Option<netui::AskBox>,
    /// Thumbnails for the Icons view.
    pub(super) thumbs: thumbs::Thumbs,
    /// The sequence player, while it's open.
    pub(super) player: Option<seqplayer::Player>,
    /// Compare Folders, while it's open.
    pub(super) compare: Option<compare::Compare>,
    /// Colours, tags and notes.
    pub(super) marks: crate::fsys::marks::Marks,
    /// The Tags and Note box, while it's open.
    pub(super) mark_edit: Option<marksui::MarkEdit>,
    /// Open With's Manage Apps box, while it's open.
    pub(super) apps_box: Option<openwith::AppsBox>,
    /// Folders shown beside the current one in the Columns view.
    pub(super) side: views::SideLists,
    /// Notices of changes other programs make to the folders on screen.
    watcher: crate::fsys::watch::Watcher,
    /// The Remote workspace: the other desk, the Site Manager, quick connect.
    pub(super) rws: super::remoteui::RemoteState,
}

impl Files {
    pub fn new(mut prefs: FilesPrefs) -> Self {
        let first = Pane::new(&prefs.tabs, prefs.active, prefs.sort, prefs.reversed, prefs.view);
        let mut panes = vec![first];
        if prefs.split != Split::Single {
            let s = &prefs.second;
            panes.push(Pane::new(&s.tabs, s.active, s.sort, s.reversed, s.view));
        }
        favs::migrate(&mut prefs);
        if prefs.base_look.is_none() {
            prefs.base_look = Some(panes[0].look());
        }
        // The standard places are in the user's profile, but a profile folder can be
        // redirected to a server, so find them off the UI thread too.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(places::standard());
        });
        Self {
            prefs,
            panes,
            focus: 0,
            closed: Vec::new(),
            places: Vec::new(),
            places_rx: Some(rx),
            path_edit: None,
            path_pane: 0,
            focus_path: false,
            focus_filter: false,
            typed: String::new(),
            typed_at: 0.0,
            focused: true,
            results: Vec::new(),
            drag_hover_tab: None,
            drop_claimed: false,
            user_templates: Vec::new(),
            user_templates_rx: None,
            fav_edit: None,
            suggest: Default::default(),
            look: Default::default(),
            suggest_rect: None,
            text_rx: None,
            net: Vec::new(),
            net_rx: None,
            connect: None,
            net_jobs: Vec::new(),
            net_asked: false,
            asks: None,
            asking: None,
            thumbs: Default::default(),
            player: None,
            compare: None,
            marks: crate::fsys::marks::Marks::load(),
            mark_edit: None,
            apps_box: None,
            side: Default::default(),
            watcher: Default::default(),
            rws: Default::default(),
        }
    }

    /// The state to save: the open tabs and everything in `prefs`.
    pub fn to_save(&self) -> FilesPrefs {
        let mut p = self.prefs.clone();
        let first = self.panes[0].save();
        p.tabs = first.tabs;
        p.active = first.active;
        p.sort = first.sort;
        p.reversed = first.reversed;
        p.view = first.view;
        if let Some(second) = self.panes.get(1) {
            p.second = second.save();
        }
        super::remoteui::finish_save(self, &mut p);
        p
    }

    pub(super) fn pane(&self) -> &Pane {
        &self.panes[self.focus]
    }

    pub(super) fn pane_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.focus]
    }

    /// The focused pane's active tab.
    pub(super) fn cur(&self) -> &FTab {
        self.pane().tab()
    }

    pub(super) fn tab(&mut self) -> &mut FTab {
        self.pane_mut().tab_mut()
    }

    pub fn here(&self) -> &Location {
        self.cur().here()
    }

    /// The pane that isn't focused, in split view.
    pub(super) fn other(&self) -> Option<usize> {
        (self.panes.len() > 1).then_some(1 - self.focus)
    }

    /// Show one pane or two.
    pub(super) fn set_split(&mut self, s: Split) {
        self.prefs.split = s;
        if s == Split::Single {
            if self.panes.len() > 1 {
                self.prefs.second = self.panes[1].save();
                // Keep the pane that has the focus.
                let keep = self.panes.swap_remove(self.focus);
                self.panes = vec![keep];
                self.focus = 0;
            }
        } else if self.panes.len() == 1 {
            let s = self.prefs.second.clone();
            let mut p = if s.tabs.is_empty() {
                let here = self.here().clone();
                Pane::new(&[here], 0, self.pane().sort, self.pane().reversed, self.pane().view)
            } else {
                Pane::new(&s.tabs, s.active, s.sort, s.reversed, s.view)
            };
            p.filter.clear();
            self.panes.push(p);
        }
    }

    /// Exchange the two panes' places.
    pub(super) fn swap_panes(&mut self) {
        if self.panes.len() == 2 {
            self.panes.swap(0, 1);
            self.focus = 1 - self.focus;
        }
    }

    pub(super) fn set_focus(&mut self, pane: usize) {
        if pane < self.panes.len() && pane != self.focus {
            self.focus = pane;
            self.path_edit = None;
        }
    }

    /// Go somewhere in the current tab, remembering where we were.
    pub fn go(&mut self, to: Location) {
        if *self.here() == to {
            return;
        }
        self.tab().hist.go(to);
        self.after_move();
    }

    /// Go somewhere in a pane's current tab.
    pub(super) fn go_in(&mut self, pane: usize, to: Location) {
        let old = self.focus;
        self.focus = pane.min(self.panes.len() - 1);
        self.go(to);
        self.focus = old;
    }

    fn after_move(&mut self) {
        self.tab().arrived();
        self.pane_mut().filter.clear();
        self.path_edit = None;
        if let Location::Dir(p) = self.here().clone() {
            let r = &mut self.prefs.recent;
            r.retain(|q| q != &p);
            super::remoteui::note_recent(&mut self.prefs.recent_servers, &p);
            r.insert(0, p);
            r.truncate(RECENT_MAX);
        }
    }

    pub(super) fn back(&mut self) {
        let from = self.here().clone();
        if self.tab().hist.back() {
            self.after_move();
            self.select_child_named(&from);
        }
    }

    pub(super) fn forward(&mut self) {
        if self.tab().hist.forward() {
            self.after_move();
        }
    }

    /// Up one level, selecting the folder we came from.
    pub(super) fn up(&mut self) {
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
                None => p.shown().into(),
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
        let p = self.pane_mut();
        p.tabs.insert(p.active + 1, FTab::new(at));
        p.active += 1;
        self.after_move();
    }

    fn close_tab(&mut self, i: usize) {
        let p = self.pane_mut();
        if p.tabs.len() < 2 || i >= p.tabs.len() {
            return;
        }
        let t = p.tabs.remove(i);
        if p.active > i || p.active == p.tabs.len() {
            p.active = p.active.saturating_sub(1);
        }
        p.filter.clear();
        self.closed.push(t.hist.here);
        if self.closed.len() > 20 {
            self.closed.remove(0);
        }
        self.path_edit = None;
    }

    /// Swap tab `i` with its neighbour `j` (dragging), keeping the same tab active.
    fn move_tab(&mut self, i: usize, j: usize) {
        let p = self.pane_mut();
        if i >= p.tabs.len() || j >= p.tabs.len() || i == j {
            return;
        }
        p.tabs.swap(i, j);
        if p.active == i {
            p.active = j;
        } else if p.active == j {
            p.active = i;
        }
    }

    fn close_others(&mut self, keep: usize) {
        let p = self.pane_mut();
        let mut gone = Vec::new();
        for i in (0..p.tabs.len()).rev() {
            if i != keep {
                gone.push(p.tabs.remove(i).hist.here);
            }
        }
        p.active = 0;
        p.filter.clear();
        self.closed.extend(gone);
        let excess = self.closed.len().saturating_sub(20);
        self.closed.drain(..excess);
        self.path_edit = None;
    }

    pub(super) fn switch_tab(&mut self, i: usize) {
        let p = self.pane_mut();
        if i < p.tabs.len() && i != p.active {
            p.active = i;
            p.filter.clear();
            self.path_edit = None;
            self.tab().scroll_to_cursor = true;
        }
    }

    pub(super) fn refresh(&mut self) {
        self.side.clear();
        if let Some(d) = self.here().dir() {
            remote::forget(d);
        }
        self.tab().reread();
    }

    /// Once a frame for each pane: remember a change of view, sort or grouping for the folder
    /// it's in, or show a folder just arrived in the way it was last shown.
    pub(super) fn keep_looks(&mut self) {
        for pane in &mut self.panes {
            let here = pane.tab().here().clone();
            let now = pane.look();
            match &pane.seen {
                Some((loc, was)) if *loc == here => {
                    if *was != now {
                        if let Location::Dir(d) = &here
                            && self.prefs.remember_looks
                        {
                            let looks = &mut self.prefs.folder_looks;
                            looks.retain(|(p, _)| p != d);
                            if Some(now) != self.prefs.base_look {
                                looks.insert(0, (d.clone(), now));
                                looks.truncate(LOOKS_MAX);
                            }
                        }
                        pane.seen = Some((here, now));
                    }
                }
                _ => {
                    let own = match &here {
                        Location::Dir(d) if self.prefs.remember_looks => self.prefs.folder_looks.iter().find(|(p, _)| p == d).map(|x| x.1),
                        _ => None,
                    };
                    if let Some(l) = own.or(if self.prefs.remember_looks { self.prefs.base_look } else { None }) {
                        pane.set_look(l);
                    }
                    pane.seen = Some((here, pane.look()));
                }
            }
        }
    }

    /// Whether the folder in the focused pane has a look of its own.
    pub(super) fn has_own_look(&self) -> bool {
        let Location::Dir(d) = self.here() else { return false };
        self.prefs.folder_looks.iter().any(|(p, _)| p == d)
    }

    /// Show every folder the way the focused pane's folder is shown now.
    pub(super) fn look_for_all(&mut self) {
        self.prefs.base_look = Some(self.pane().look());
        self.prefs.folder_looks.clear();
    }

    /// Show the focused pane's folder the default way again.
    pub(super) fn forget_look(&mut self) {
        if let Location::Dir(d) = self.here().clone() {
            self.prefs.folder_looks.retain(|(p, _)| *p != d);
        }
        if let Some(b) = self.prefs.base_look {
            let p = self.pane_mut();
            p.set_look(b);
            p.seen = None;
        }
    }

    pub(super) fn is_pinned(&self, p: &Path) -> bool {
        self.prefs.favourites.iter().any(|g| g.items.iter().any(|f| f.path == p))
    }

    // ---- for the analysis dock ----

    /// What the dock follows changes when any of this does: the pane and tab, its folder, the
    /// item the keyboard is on, and how many are selected.
    pub(super) fn selection_key(&self) -> (usize, Location, Option<Box<str>>, usize) {
        let t = self.cur();
        (self.focus * 1000 + self.pane().active, t.here().clone(), t.cursor.clone(), t.selected.len())
    }

    /// The item the keyboard is on (the last one clicked), if it's listed.
    pub(super) fn cursor_entry(&self) -> Option<Entry> {
        let t = self.cur();
        t.entry_named(t.cursor.as_deref()?).cloned()
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
        let t = self.cur();
        let l = t.listing.as_ref().filter(|l| l.done && l.error.is_none())?;
        Some((t.here().dir()?.to_path_buf(), l.generation, l.entries.clone()))
    }

    /// Paths of the selected items, in display order.
    pub(super) fn chosen_paths(&self) -> Vec<PathBuf> {
        self.cur().chosen_paths()
    }

    /// Run a command that might wait on the disk or another program, off the UI thread.
    pub(super) fn spawn(&mut self, f: impl FnOnce() -> Result<(), String> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        self.results.push(rx);
    }
}

/// Does a name pass the "filter this folder" box?
#[cfg(test)]
pub(super) fn name_matches(filter: &str, name: &str) -> bool {
    crate::fsys::listing::name_matches(filter, name)
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
            (show_hidden || !e.hidden) && crate::fsys::listing::entry_matches(filter, &e.name, e.is_dir)
        })
        .collect();
    let size_of = |e: &Entry| if e.is_dir { sizes.and_then(|s| s.get(&e.name)).map(|s| s.0).unwrap_or(0) } else { e.shown_size() };
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
        if (show_hidden || !e.hidden) && crate::fsys::listing::entry_matches(filter, &e.name, e.is_dir) {
            order.push(i as u32);
        }
    }
}

/// Things clicked while drawing, acted on afterwards.
#[derive(Default)]
pub(super) struct FPicks {
    pub go: Option<Location>,
    /// Go somewhere in a given pane.
    pub go_other: Option<(usize, Location)>,
    pub open: Option<u32>,
    pub select: Option<(u32, egui::Modifiers)>,
    pub menu: Option<(FAction, Vec<PathBuf>)>,
    pub sort: Option<FSort>,
    pub new_tab: Option<Location>,
    pub switch_tab: Option<usize>,
    pub close_tab: Option<usize>,
    pub close_others: Option<usize>,
    pub move_tab: Option<(usize, usize)>,
    pub clear: bool,
    pub drop: Option<fileops::Drop>,
    pub new_item: Option<(fileops::NewPick, PathBuf)>,
    /// Show a folder with one item in it selected (a click in a side column).
    pub reveal: Option<(PathBuf, Box<str>)>,
}

/// File manager commands, from menus and keys.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum FAction {
    Open,
    OpenInNewTab,
    OpenInOtherPane,
    Pin,
    Unpin,
    CopyPath,
    /// The \\server\share form of a path on a mapped drive.
    CopyUnc,
    CopyName,
    Reveal,
    Terminal(actions::Terminal),
    Analyse,
    Copy,
    Cut,
    /// Into the folder given (the first path).
    Paste,
    Rename,
    Trash,
    Delete,
    CopyToOther,
    MoveToOther,
    Properties,
    /// A terminal on the item's server (SSH), in its folder.
    SshTerminal,
    /// Read the folder again (after an error).
    Reload,
    QuickLook,
    /// Archives: take everything out, next to the archive or into a folder picked.
    ExtractHere,
    ExtractTo,
    /// An archive in the app the system opens it with (Open goes inside it).
    OpenWithApp,
    /// The selected image sequences, in the player (two: A/B).
    Play,
    /// Set (or clear) the items' colour.
    Colour(Option<u8>),
    /// Open With: one of the user's apps, or choose or manage them.
    OpenWith(openwith::Pick),
    /// The Tags and Note box.
    Tags,
    /// Windows' own right-click menu, with other apps' items (true: with the Shift extras).
    SystemMenu(bool),
    /// Cloud files: remove the local copy, or keep them on this computer (`cloudops`).
    FreeUp,
    KeepHere,
    /// The "What Is This?" window for a system file or folder.
    Explain,
    /// Ask AI about the items: their names, sizes and dates.
    AskAi,
}

impl App {
    /// Change workspace, taking the current folder and selection along.
    pub(super) fn set_workspace(&mut self, to: Workspace) {
        let from = self.files.prefs.workspace;
        if from == to {
            return;
        }
        // Files and Remote each have their own panes; Storage Analysis keeps the last ones.
        super::remoteui::use_desk(&mut self.files, to);
        match to {
            Workspace::Analysis => {
                if self.job.is_none() {
                    self.analysis_follow_files();
                }
            }
            Workspace::Files if from == Workspace::Analysis => self.files_follow_analysis(),
            Workspace::Files | Workspace::Remote | Workspace::Models => {}
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
        let narrow = style::narrow(ui);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if ui
                .add(egui::Button::selectable(ws == Workspace::Files, if narrow { "🗀" } else { "🗀 Files" }))
                .on_hover_text(format!("File Manager: browse and open files, no scan needed ({})", k(egui::Key::Num1)))
                .clicked()
            {
                to = Some(Workspace::Files);
            }
            if ui
                .add(egui::Button::selectable(ws == Workspace::Remote, if narrow { "🖧" } else { "🖧 Remote" }))
                .on_hover_text(format!("Servers: this computer beside a server, saved sites, quick connect ({})", k(egui::Key::Num2)))
                .clicked()
            {
                to = Some(Workspace::Remote);
            }
            if ui
                .add(egui::Button::selectable(ws == Workspace::Analysis, if narrow { "⊞ Storage" } else { "⊞ Storage Analysis" }))
                .on_hover_text(format!("Scan a drive or folder: folder tree, treemap, file types, duplicates… ({})", k(egui::Key::Num3)))
                .clicked()
            {
                to = Some(Workspace::Analysis);
            }
            if ui
                .add(egui::Button::selectable(ws == Workspace::Models, if narrow { "✨" } else { "✨ AI Models" }))
                .on_hover_text(format!("The AI models on this computer: Ollama, LM Studio, Hugging Face and model files ({})", k(egui::Key::Num4)))
                .clicked()
            {
                to = Some(Workspace::Models);
            }
        });
        if let Some(w) = to {
            self.set_workspace(w);
        }
        ui.separator();
    }

    /// Ctrl/⌘+1 to 4 switch workspace from anywhere.
    pub(super) fn workspace_keys(&mut self, ctx: &egui::Context) {
        if self.ops.modal() {
            return;
        }
        let (files, remote, analysis, models) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num1),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num2),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num3),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num4),
            )
        });
        if files {
            self.set_workspace(Workspace::Files);
        } else if remote {
            self.set_workspace(Workspace::Remote);
        } else if analysis {
            self.set_workspace(Workspace::Analysis);
        } else if models {
            self.set_workspace(Workspace::Models);
        }
    }

    /// Analyse `dir` without leaving the File Manager: the dock opens and shows the scan, and
    /// the treemap once it's done. A folder inside the current analysis is refreshed in place.
    pub(super) fn analyse(&mut self, dir: PathBuf) {
        self.path_input = dir.shown();
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

    /// Toolbar of the File Manager, in groups: workspaces, New, and on the right analysis, the
    /// view buttons, View, Help and Settings. It works on the focused pane.
    pub(super) fn files_toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        ui.horizontal(|ui| {
            style::roomy(ui);
            self.workspace_switch(ui);
            self.remote_toolbar(ui);
            // Navigation, the path and the filter are on each pane's own bar (`pane_bar`).
            let dir = self.files.here().dir().map(Path::to_path_buf);
            let mut pick = None;
            style::tool_group(ui, |ui| {
                ui.add_enabled_ui(dir.is_some(), |ui| {
                    ui.menu_button(egui::RichText::new("✚ New").size(13.0), |ui| {
                        if let Some(d) = &dir {
                            pick = self.new_menu_items(ui, d);
                        }
                    })
                    .response
                    .on_hover_text("Make a new folder or file here");
                });
            });
            if let (Some(p), Some(d)) = (pick, dir) {
                self.apply_new(p, d);
            }
            ui.add_space(2.0);
            // Right-hand controls first, so the path takes whatever width is left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::icon_button(ui, true, "⚙", "Settings: appearance, scan filters, treemap look").clicked() {
                    self.dialog.open(&self.settings);
                }
                self.help_menu(ui);
                self.files_view_menu(ui);
                style::tool_group(ui, |ui| {
                    let split = self.files.prefs.split != Split::Single;
                    if style::toggle_button(ui, self.files.prefs.preview, "👁", format!("Preview pane ({})", k(egui::Modifiers::ALT, egui::Key::P))).clicked() {
                        self.files.prefs.preview = !self.files.prefs.preview;
                    }
                    if style::toggle_button(ui, split, "⬌", format!("Two panes side by side ({})", k(egui::Modifiers::COMMAND, egui::Key::Backslash))).clicked() {
                        self.toggle_split();
                    }
                });
                ui.add_space(6.0);
                self.dock_toolbar_buttons(ui);
            });
        });
    }

    /// Back, Forward, Up and Refresh, for the pane being drawn.
    fn nav_buttons(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        let (can_back, can_fwd) = {
            let h = &self.files.cur().hist;
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
    }

    /// The filter box (and subfolder search and Save) for the pane being drawn. `focused`: it's
    /// the pane with the keyboard, which Ctrl+F puts the cursor in. Laid out right to left.
    /// `width` None: no room for the box, only its buttons.
    fn filter_box(&mut self, ui: &mut egui::Ui, focused: bool, width: Option<f32>) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        let fid = egui::Id::new(("files-filter", self.files.focus));
        if let Some(width) = width {
            let hint = if width < 130.0 { "🔍 Filter".to_string() } else { format!("🔍 Filter ({})", k(egui::Modifiers::COMMAND, egui::Key::F)) };
            let f = ui.add(egui::TextEdit::singleline(&mut self.files.pane_mut().filter).id(fid).desired_width(width).hint_text(hint));
            f.on_hover_text("Show only items whose names contain this text. * and ? work as wildcards (*.exr, shot_??.png).");
        }
        let deep = self.files.pane().deep;
        if style::toggle_button(ui, deep, "🗐", "Search subfolders too: the filter and Show look in this folder and every folder below it").clicked() {
            self.files.pane_mut().deep = !deep;
        }
        self.show_menu(ui);
        let query = self.files.pane().filter.trim().to_string();
        if deep
            && !query.is_empty()
            && let Some(dir) = self.files.here().dir().map(Path::to_path_buf)
        {
            let saved = self.files.prefs.searches.iter().any(|s| s.dir == dir && s.query == query);
            if style::icon_button(ui, !saved, "☆", if saved { "This search is in the sidebar" } else { "Save this search in the sidebar" }).clicked() {
                let name = format!("{query} in {}", dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| dir.shown()));
                if self.premium(crate::licence::Feature::SavedSearches) {
                    self.files.prefs.searches.push(SavedSearch { name, dir, query });
                }
            }
        }
        if focused && std::mem::take(&mut self.files.focus_filter) {
            ctx.memory_mut(|m| m.request_focus(fid));
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && ctx.memory(|m| m.has_focus(fid)) {
            self.files.pane_mut().filter.clear();
            ctx.memory_mut(|m| m.surrender_focus(fid));
        }
    }

    /// Show: only some kinds of file (Audio, Video, Images…), like Storage Analysis's Show row.
    /// With 🗐 on it gathers them from every subfolder too.
    fn show_menu(&mut self, ui: &mut egui::Ui) {
        use crate::categories::Category;
        let kinds = self.files.pane().kinds;
        let label = match kinds {
            0 => "Show ⏷".to_string(),
            k if k.count_ones() == 1 => format!("{} ⏷", Category::ALL.iter().find(|c| c.bit() == k).map_or("", |c| c.label())),
            k => format!("{} kinds ⏷", k.count_ones()),
        };
        let mut picked = None;
        ui.scope(|ui| {
            if kinds != 0 {
                // Lit up while it hides things, like a toggle that's on.
                let v = ui.visuals_mut();
                v.widgets.inactive.weak_bg_fill = v.selection.bg_fill;
                v.widgets.inactive.fg_stroke.color = v.selection.stroke.color;
            }
            let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            egui::containers::menu::MenuButton::new(label).config(config).ui(ui, |ui| {
                if ui.add(egui::Button::selectable(kinds == 0, style::mi("All"))).clicked() {
                    picked = Some(0);
                }
                ui.separator();
                for c in Category::ALL {
                    let on = kinds & c.bit() != 0;
                    if ui.add(egui::Button::selectable(on, style::mi(c.label()))).on_hover_text(c.tooltip()).clicked() {
                        picked = Some(kinds ^ c.bit());
                    }
                }
                ui.separator();
                ui.weak("Pick several to show them all.\nWith 🗐 on, subfolders are searched too.");
            })
            .0
            .on_hover_text("Show only some kinds of file: audio, video, images, documents…");
        });
        if let Some(k) = picked
            && (k & !kinds == 0 || self.premium(crate::licence::Feature::Filters))
        {
            self.files.pane_mut().kinds = k;
        }
    }

    /// The pane's own Back, Forward, Up, Refresh, path and filter, above its tabs.
    fn pane_bar(&mut self, ui: &mut egui::Ui, focused: bool) {
        let mut go = None;
        egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 6, top: 4, bottom: 2 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                style::roomy(ui);
                self.nav_buttons(ui);
                ui.add_space(2.0);
                // Fit a narrow pane: the filter narrows first, then the path goes (the tab still
                // names the folder), then the filter box itself, leaving its 🗐 button.
                let avail = ui.available_width();
                let h = ui.spacing().interact_size.y;
                let star = self.files.pane().deep && !self.files.pane().filter.trim().is_empty();
                // 🗐 and ☆, and the Show menu.
                let buttons = (h + 4.0) * if star { 2.0 } else { 1.0 } + 70.0;
                let mut filter = (avail * 0.3).clamp(60.0, 150.0);
                let mut path = avail - filter - buttons - 8.0;
                if path < 70.0 {
                    path = 0.0;
                    filter = (avail - buttons - 4.0).min(150.0);
                }
                if path > 0.0 {
                    ui.allocate_ui_with_layout(vec2(path, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                        go = self.path_bar(ui);
                    });
                    ui.add_space(4.0);
                }
                let rest = ui.available_width().max(0.0);
                ui.allocate_ui_with_layout(vec2(rest, h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    // TextEdit adds its margins to the width it's given.
                    self.filter_box(ui, focused, (filter >= 50.0).then_some(filter - 8.0));
                });
            });
        });
        if let Some(to) = go {
            self.files.go(to);
        }
    }

    /// The View menu: panes, preview, hidden items, row density and column widths.
    fn files_view_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        let mut split_to = None;
        let mut swap = false;
        let mut same = false;
        let mut compare = false;
        ui.menu_button("View ⏷", |ui| {
            let split = self.files.prefs.split;
            ui.weak("Panes");
            for (s, label, keys) in [(Split::Single, "One Pane", String::new()), (Split::Vertical, "Side by Side", k(egui::Modifiers::COMMAND, egui::Key::Backslash)), (Split::Horizontal, "One Above the Other", String::new())] {
                if ui.add(egui::Button::selectable(split == s, style::mi(label)).shortcut_text(keys)).clicked() {
                    split_to = Some(s);
                }
            }
            let two = split != Split::Single;
            if ui.add_enabled(two, egui::Button::new(style::mi("Swap Panes")).shortcut_text(k(egui::Modifiers::COMMAND, egui::Key::U))).clicked() {
                swap = true;
            }
            if ui.add_enabled(two, egui::Button::new(style::mi("Same Folder in Other Pane"))).on_hover_text("Open this pane's folder in the other pane too").clicked() {
                same = true;
            }
            if ui.add_enabled(two, egui::Button::new(style::mi("Compare Folders…"))).on_hover_text("Compare the two panes' folders, and sync them").clicked() {
                compare = true;
            }
            ui.separator();
            ui.weak(if two { "This pane" } else { "Layout" });
            let pane = self.files.pane_mut();
            for v in FView::ALL {
                if ui.add(egui::Button::selectable(pane.view == v, style::mi(v.title()))).on_hover_text(v.tip()).clicked() {
                    pane.view = v;
                }
            }
            if pane.view == FView::Details {
                ui.menu_button(style::mi("Group By"), |ui| {
                    for g in views::FGroup::ALL {
                        if ui.add(egui::Button::selectable(pane.group == g, style::mi(g.title()))).clicked() {
                            pane.group = g;
                        }
                    }
                });
            }
            if pane.view != FView::Details {
                ui.menu_button(style::mi("Sort By"), |ui| {
                    for col in [FSort::Name, FSort::Size, FSort::Type, FSort::Modified] {
                        if ui.add(egui::Button::selectable(pane.sort == col, style::mi(col.title()))).clicked() {
                            pane.sort = col;
                        }
                    }
                    ui.separator();
                    if ui.add(egui::Button::selectable(pane.reversed, style::mi("Reverse Order"))).clicked() {
                        pane.reversed = !pane.reversed;
                    }
                });
            }
            if pane.view == FView::Icons {
                let size = &mut self.files.prefs.icon_size;
                ui.add(egui::Slider::new(size, ICON_MIN..=ICON_MAX).text("Size").show_value(false))
                    .on_hover_text(format!("Or hold {} and turn the mouse wheel over the icons", if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }));
            }
            let own = self.files.has_own_look();
            if ui.add(egui::Button::new(style::mi("Use This View for All Folders"))).on_hover_text("Show every folder with this view, sort and grouping, forgetting the ones set for single folders").clicked() {
                self.files.look_for_all();
            }
            if ui.add_enabled(own, egui::Button::new(style::mi("Reset This Folder's View"))).on_hover_text("Show this folder the way folders are shown by default").clicked() {
                self.files.forget_look();
            }
            let p = &mut self.files.prefs;
            ui.toggle_value(&mut p.remember_looks, style::mi("Remember Each Folder's View")).on_hover_text("A change of view, sort or grouping applies to that folder only, and comes back when you return to it");
            ui.separator();
            if ui.add(egui::Button::selectable(p.preview, style::mi("Preview Pane")).shortcut_text(k(egui::Modifiers::ALT, egui::Key::P))).clicked() {
                p.preview = !p.preview;
            }
            if ui
                .add(egui::Button::selectable(p.show_hidden, style::mi("Hidden Items")).shortcut_text(k(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Period)))
                .on_hover_text("Show hidden files and folders: names starting with a dot, and items marked hidden on Windows")
                .clicked()
            {
                p.show_hidden = !p.show_hidden;
            }
            if ui
                .add(egui::Button::selectable(p.sequences, style::mi("Group Image Sequences")))
                .on_hover_text("Show numbered frames (shot.1001.exr, shot.1002.exr…) as one row. Enter or a double-click plays the sequence")
                .clicked()
            {
                p.sequences = !p.sequences;
            }
            ui.separator();
            ui.weak("Rows");
            if ui.add(egui::Button::selectable(p.density == Density::Comfortable, style::mi("Comfortable"))).clicked() {
                p.density = Density::Comfortable;
            }
            if ui.add(egui::Button::selectable(p.density == Density::Compact, style::mi("Compact"))).clicked() {
                p.density = Density::Compact;
            }
            ui.separator();
            if ui.add_enabled(p.col_widths != COL_WIDTHS, egui::Button::new(style::mi("Reset Column Widths"))).clicked() {
                p.col_widths = COL_WIDTHS;
            }
            ui.separator();
            let dock = self.files.prefs.dock.open;
            if ui.add(egui::Button::selectable(dock, style::mi("Analysis Dock")).shortcut_text(k(egui::Modifiers::COMMAND, egui::Key::J))).clicked() {
                self.files.prefs.dock.open = !dock;
            }
        })
        .response
        .on_hover_text("Panes, Details or Icons, preview, hidden items, row size, columns and the analysis dock");
        if let Some(s) = split_to {
            if s != Split::Single {
                self.files.prefs.last_split = s;
            }
            self.files.set_split(s);
        }
        if swap {
            self.files.swap_panes();
        }
        if compare {
            self.start_compare();
        }
        if same && let Some(o) = self.files.other() {
            let here = self.files.here().clone();
            self.files.go_in(o, here);
        }
    }

    /// Everything below the toolbar in the File Manager workspace.
    pub(super) fn files_workspace(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.files.drop_claimed = false;
        self.files_poll(&ctx);
        self.dock_poll(&ctx);
        self.remote_frame(&ctx);
        self.connect_window(&ctx);
        if !self.files.look.from_map {
            self.quick_look(&ctx);
        }
        self.sequence_player(&ctx);
        self.compare_window(&ctx);
        self.mark_edit_window(&ctx);
        self.apps_window(&ctx);
        let pal = style::pal(ui);

        let status = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::symmetric(10, 3));
        let r = egui::Panel::bottom("files-status").frame(status).show(ui, |ui| self.files_status(ui));
        self.help_regions.push((r.response.rect, topics::FILE_MANAGER));
        self.transfers_panel(ui);
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
        // Thumbnails asked for while drawing last frame, then any that are ready.
        self.files.thumbs.flush();
        self.files.thumbs.poll(ctx);
        self.files.side.tick();
        self.files.keep_looks();
        self.watch_folders(ctx);
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
        if let Some(rx) = &self.files.text_rx
            && let Ok(t) = rx.try_recv()
        {
            self.ops.notice = Some((format!("Copied {}", t.lines().next().unwrap_or_default()), false));
            self.ctx_copy = Some(t);
            self.files.text_rx = None;
        }
        self.net_poll(ctx);
        for pane in 0..self.files.panes.len() {
            self.poll_pane(ctx, pane);
        }
        // A new folder or file is listed: open the Rename box on it.
        let t = self.files.cur();
        if t.rename_later
            && t.select_later.is_none()
            && let (Some(dir), Some(c)) = (t.here().dir(), t.cursor.as_deref())
        {
            let p = dir.join(c);
            self.files.tab().rename_later = false;
            if !self.ops.modal() {
                self.start_rename(p);
            }
        }
    }

    /// Watch the folders open in tabs and beside them, and mark the ones that changed to be read
    /// again.
    fn watch_folders(&mut self, ctx: &egui::Context) {
        let mut dirs: HashSet<PathBuf> = HashSet::new();
        for p in &self.files.panes {
            dirs.extend(p.tabs.iter().filter_map(|t| t.here().dir().map(Path::to_path_buf)));
        }
        dirs.extend(self.files.side.dirs().cloned());
        // Servers send no change notices, and archives don't change.
        dirs.retain(|d| !remote::is_remote(d) && archive::split(d).is_none());
        let c = ctx.clone();
        self.files.watcher.wake_with(move || c.request_repaint());
        self.files.watcher.set(dirs);
        let changed = self.files.watcher.changed();
        if changed.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        for p in &mut self.files.panes {
            for t in &mut p.tabs {
                if t.here().dir().is_some_and(|d| changed.contains(d)) {
                    t.dirty_at.get_or_insert(now);
                }
            }
        }
        for d in &changed {
            self.files.side.forget(d);
        }
        ctx.request_repaint();
    }

    /// Keep one pane's visible tab listed and sorted.
    fn poll_pane(&mut self, ctx: &egui::Context, pane: usize) {
        // Changes with the tree and with the size shown (pointers are aligned, so the low bit is free).
        let tree_id = self.tree.as_ref().map(|t| Arc::as_ptr(t) as usize).unwrap_or(0) | self.metric as usize;
        let need_sizes = {
            let t = self.files.panes[pane].tab();
            t.sizes.as_ref().map(|s| s.0) != Some(tree_id)
        };
        if need_sizes {
            let sizes = self.files.panes[pane].tab().here().dir().and_then(|d| self.scanned_sizes(d)).unwrap_or_default();
            self.files.panes[pane].tab_mut().sizes = Some((tree_id, sizes));
        }
        let hidden = self.files.prefs.show_hidden;
        let seqs_on = self.files.prefs.sequences;
        let p = &mut self.files.panes[pane];
        let (sort, reversed, group) = (p.sort, p.reversed, if p.view == FView::Details { p.group } else { views::FGroup::None });
        let group = (group, seqs_on);
        let tag = p
            .filter
            .trim()
            .strip_prefix('#')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let kinds = p.kinds;
        let wanted = (tag.is_some() || (p.deep && (!p.filter.trim().is_empty() || kinds != 0)))
            .then(|| (crate::fsys::listing::pack_filter(p.filter.trim(), if tag.is_some() { 0 } else { kinds }), hidden));
        let deep = p.deep;
        let tagged = tag.map(|t| {
            Arc::new(
                self.files
                    .marks
                    .tagged(&t)
                    .into_iter()
                    .collect::<HashSet<PathBuf>>(),
            )
        });
        let p = &mut self.files.panes[pane];
        // A search's results already match: the filter isn't applied to their paths again.
        let filter = if wanted.is_some() { String::new() } else { crate::fsys::listing::pack_filter(&p.filter, p.kinds) };
        let t = p.tab_mut();
        if t.search != wanted || (wanted.is_some() && t.search_deep != deep) {
            t.search = wanted.clone();
            t.search_deep = deep;
            t.listing = None;
            t.order_key = None;
            t.sorting = None;
            t.fresh = None;
            t.selected.clear();
        }
        if t.listing.is_none()
            && let Location::Dir(p) = t.here().clone()
        {
            t.listing = Some(match &wanted {
                Some((q, h)) => Listing::search(p, q.clone(), *h, deep, tagged),
                None => Listing::start(p),
            });
        }
        if t.search.is_some() {
            // Results aren't read again when something changes; F5 searches again.
            t.dirty_at = None;
        }
        let Some(l) = t.listing.as_mut() else { return };
        l.poll();
        if !l.done {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let sizes = t.sizes.as_ref().map(|s| s.1.clone()).unwrap_or_default();
        // Changed by another program: read it again once the changes settle (longer for big
        // folders), sort the new listing off the UI thread, then swap it in.
        let done = l.done;
        let n = l.entries.len();
        if let Some(at) = t.dirty_at
            && t.fresh.is_none()
            && done
            && let Some(d) = t.here().dir()
        {
            let now = ctx.input(|i| i.time);
            let wait = 0.25 + (n as f64 / 20_000.0).min(5.0);
            if now - at >= wait {
                t.fresh = Some((Listing::start(d.to_path_buf()), None));
                t.dirty_at = None;
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(wait - (now - at)));
            }
        }
        if let Some((fl, sorting)) = &mut t.fresh {
            fl.poll();
            ctx.request_repaint_after(Duration::from_millis(40));
            if fl.done && fl.error.is_some() {
                t.fresh = None;
            } else if fl.done && sorting.is_none() {
                let (tx, rx) = mpsc::channel();
                let (entries, f, sz) = (fl.entries.clone(), filter.clone(), sizes.clone());
                let k: OrderKey = (fl.generation, sort, reversed, hidden, filter.clone(), tree_id, group);
                std::thread::spawn(move || {
                    let _ = tx.send((k, views::arrange(&entries, Some(&sz), sort, reversed, hidden, &f, group.0, group.1)));
                });
                *sorting = Some(rx);
            } else if let Some(rx) = sorting {
                match rx.try_recv() {
                    Ok((k, (o, g, q))) => {
                        let (fl, _) = t.fresh.take().unwrap();
                        t.listing = Some(fl);
                        t.sorting = None;
                        t.order = o;
                        t.groups = g;
                        t.seqs = q;
                        t.order_key = Some(k);
                        // Items that went away aren't selected any more.
                        let l = t.listing.as_ref().unwrap();
                        let names: HashSet<&str> = l.entries.iter().map(|e| &*e.name).collect();
                        t.selected.retain(|s| names.contains(&**s));
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => t.fresh = None,
                }
            }
        }
        let Some(l) = t.listing.as_ref() else { return };
        let key: OrderKey = (l.generation, sort, reversed, hidden, filter.clone(), tree_id, group);
        // A sort finished on another thread: use it, adding anything listed since at the end.
        if let Some((k, covered, rx)) = &t.sorting {
            match rx.try_recv() {
                Ok((order, heads, seqs)) => {
                    let same = (k.1, k.2, k.3, &k.4, k.5, k.6) == (sort, reversed, hidden, &filter, tree_id, group);
                    let covered = *covered;
                    t.sorting = None;
                    if same {
                        t.order = order;
                        t.groups = heads;
                        t.seqs = seqs;
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
            let only_more = t.order_key.as_ref().is_some_and(|k| (k.1, k.2, k.3, &k.4, k.5, k.6) == (sort, reversed, hidden, &filter, tree_id, group));
            if n <= SORT_INLINE_MAX {
                (t.order, t.groups, t.seqs) = views::arrange(&l.entries, Some(&sizes), sort, reversed, hidden, &filter, group.0, group.1);
                t.order_key = Some(key);
            } else if only_more && !l.done {
                // A big folder still streaming in: show new items at the end, sort once it's all here.
                let seen: usize = t.order.iter().map(|&i| i as usize + 1).max().unwrap_or(0);
                append_new(&mut t.order, &l.entries, seen, hidden, &filter);
                t.groups.clear();
                t.seqs.clear();
                t.order_key = Some(key);
            } else {
                let entries = l.entries.clone();
                let (tx, rx) = mpsc::channel();
                let f = filter.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(views::arrange(&entries, Some(&sizes), sort, reversed, hidden, &f, group.0, group.1));
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
                t.rename_later = false;
            }
        }
    }

    // ---------- sidebar ----------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let mut picks = FPicks::default();
        ui.ctx().data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<egui::Id>>(egui::Id::new(SIDE_ROWS))
                .clear()
        });
        let here = self.files.here().clone();
        ui.spacing_mut().item_spacing.y = 1.0;
        let pressed = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| ui.max_rect().contains(p)));
        if pressed {
            self.files.focused = true;
        }

        self.remote_sidebar(ui, &here, &mut picks);
        side_section(ui, "fm-quick", "Quick access", true, |ui| {
            if self.files.places.is_empty() && self.files.places_rx.is_some() {
                ui.weak("…");
            }
            for p in self.files.places.clone() {
                let loc = Location::Dir(p.path.clone());
                let resp = side_row(ui, p.kind.icon(), &p.name, here == loc, ROW_F).on_hover_text(p.path.shown());
                self.place_row(ui, &resp, &p.path, &mut picks);
            }
        });
        self.favourites_sections(ui, &mut picks);
        let computer = places::computer_name();
        side_section(ui, "fm-drives", "Drives", true, |ui| {
            let resp = side_row(ui, "🖳", computer, here == Location::Computer, ROW_F).on_hover_text(format!("All drives and volumes ({computer})"));
            if resp.clicked() {
                picks.go = Some(Location::Computer);
            }
            if self.drives.is_empty() && self.drives_rx.is_some() {
                ui.weak("Looking for drives…");
            }
            let drives: Vec<(PathBuf, String, u64, u64, bool)> = self.drives.iter().filter(|d| !d.network).map(|d| (d.root.clone(), d.label.clone(), d.total, d.free, d.network)).collect();
            for (root, label, total, free, _) in drives {
                let name = drive_label(&root, &label);
                let used = total.saturating_sub(free);
                let tip = format!("{}\n{} free of {}", root.display(), scan::human(free), scan::human(total));
                let frac = (total > 0).then(|| used as f32 / total as f32);
                let resp = side_row(ui, "🖴", &name, here == Location::Dir(root.clone()), 30.0).on_hover_text(tip);
                capacity_bar(ui, resp.rect, frac);
                self.place_row(ui, &resp, &root, &mut picks);
            }
            ui.add_space(2.0);
            if ui.add(egui::Button::new(egui::RichText::new("⟳ Refresh drives").small().color(style::pal(ui).weak)).frame_when_inactive(false)).clicked() {
                self.refresh_drives();
                self.net_refresh();
            }
        });
        if !self.wsl.is_empty() {
            side_section(ui, "fm-linux", "Linux", true, |ui| {
                for root in self.wsl.clone() {
                    // `\\wsl.localhost\Ubuntu` is all prefix to `Path`, so take the name from the text.
                    let name = root.to_string_lossy().rsplit('\\').next().unwrap_or_default().to_string();
                    let resp = side_row(ui, "🐧", &name, here == Location::Dir(root.clone()), ROW_F).on_hover_text(format!("{}\nWSL: starts the distribution if it isn't running", root.display()));
                    self.place_row(ui, &resp, &root, &mut picks);
                }
            });
        }
        self.network_section(ui, &here, &mut picks);
        let mut run_search = None;
        if !self.files.prefs.searches.is_empty() {
            let mut remove = None;
            side_section(ui, "fm-searches", "Saved searches", true, |ui| {
                for (i, s) in self.files.prefs.searches.iter().enumerate() {
                    let resp = side_row(ui, "🔍", &s.name, false, ROW_F).on_hover_text(format!(
                        "{} in {} and below",
                        s.query,
                        s.dir.shown()
                    ));
                    if resp.clicked() {
                        run_search = Some(s.clone());
                    }
                    resp.context_menu(|ui| {
                        if ui.add(egui::Button::new(style::mi("Remove from This List"))).clicked() {
                            remove = Some(i);
                            ui.close();
                        }
                    });
                }
            });
            if let Some(i) = remove {
                self.files.prefs.searches.remove(i);
            }
        }
        if !self.files.prefs.recent.is_empty() {
            side_section(ui, "fm-recent", "Recent", false, |ui| {
                for p in self.files.prefs.recent.clone() {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.shown());
                    let resp = side_row(ui, "🕘", &name, here == Location::Dir(p.clone()), ROW_F).on_hover_text(p.shown());
                    self.place_row(ui, &resp, &p, &mut picks);
                }
            });
        }
        let tag = self.tags_section(ui);
        // A row opened from the keyboard hands the keyboard back to the list.
        let ctx = ui.ctx().clone();
        let side_focus = Self::side_focus(&ctx);
        if let Some(id) = side_focus
            && (picks.go.is_some()
                || run_search.is_some()
                || tag.is_some()
                || ctx.input(|i| i.key_pressed(egui::Key::Escape)))
        {
            ctx.memory_mut(|m| m.surrender_focus(id));
            self.files.focused = true;
        }
        self.files_apply(picks);
        if let Some((dir, t)) = tag {
            self.show_tag(dir, t);
        }
        if let Some(s) = run_search {
            self.files.go(Location::Dir(s.dir));
            let p = self.files.pane_mut();
            p.filter = s.query;
            p.deep = true;
        }
    }

    /// The sidebar row that has the keyboard, if one does.
    fn side_focus(ctx: &egui::Context) -> Option<egui::Id> {
        let f = ctx.memory(|m| m.focused())?;
        ctx.data(|d| d.get_temp::<Vec<egui::Id>>(egui::Id::new(SIDE_ROWS)))
            .filter(|v| v.contains(&f))
            .map(|_| f)
    }

    /// F6: the keyboard to the sidebar (its first row), or back to the list.
    fn toggle_side_focus(&mut self, ctx: &egui::Context) {
        if let Some(id) = Self::side_focus(ctx) {
            ctx.memory_mut(|m| m.surrender_focus(id));
            self.files.focused = true;
        } else if let Some(first) = ctx
            .data(|d| d.get_temp::<Vec<egui::Id>>(egui::Id::new(SIDE_ROWS)))
            .and_then(|v| v.first().copied())
        {
            ctx.memory_mut(|m| m.request_focus(first));
        }
    }

    /// A folder row in the sidebar: click to go, middle-click for a tab, a menu, and drops.
    pub(super) fn place_row(&mut self, ui: &egui::Ui, resp: &egui::Response, path: &Path, picks: &mut FPicks) {
        let loc = Location::Dir(path.to_path_buf());
        if resp.clicked() {
            picks.go = Some(loc.clone());
        }
        if resp.middle_clicked() {
            picks.new_tab = Some(loc);
        }
        if let Some(d) = self.folder_drop(ui, resp, path) {
            picks.drop = Some(d);
        }
        let pinned = self.files.is_pinned(path);
        let other = self.files.other();
        resp.context_menu(|ui| place_menu(ui, path, pinned, other.is_some(), picks));
    }

    // ---------- main area ----------



    fn files_main(&mut self, ui: &mut egui::Ui) {
        let pal = style::pal(ui);
        ui.spacing_mut().item_spacing.y = 0.0;
        let area = ui.max_rect();
        // The analysis dock under everything, full width.
        if self.files.prefs.dock.open {
            let max = (area.height() - TAB_STRIP_H - DOCK_MIN_LIST).max(DOCK_MIN);
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
            let top = r.response.rect.top();
            ui.painter().hline(r.response.rect.x_range(), top, Stroke::new(1.0, pal.hairline));
            self.help_regions.push((r.response.rect, topics::ANALYSIS_DOCK));
        }
        // The preview pane at the right.
        if self.files.prefs.preview {
            let r = egui::Panel::right("files-preview")
                .resizable(true)
                .size_range(220.0..=(area.width() * 0.5).max(240.0))
                .default_size(self.files.prefs.preview_width)
                .frame(egui::Frame::new().fill(pal.side).inner_margin(egui::Margin { left: 10, right: 10, top: 4, bottom: 8 }))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("preview-scroll").auto_shrink(false).show(ui, |ui| self.preview_pane(ui));
                });
            self.files.prefs.preview_width = r.response.rect.width();
            ui.painter().vline(r.response.rect.left(), r.response.rect.y_range(), Stroke::new(1.0, pal.hairline));
        }
        // One pane or two.
        let rest = ui.available_rect_before_wrap();
        let rects = self.pane_rects(ui, rest);
        let pointer_in = rects.iter().any(|r| ui.rect_contains_pointer(*r));
        let mut picks = FPicks::default();
        self.files_keys(ui.ctx(), pointer_in, &mut picks);
        self.files_apply(picks);
        for (pane, r) in rects.into_iter().enumerate() {
            // A key in the first pane can close the second (Ctrl+\).
            if pane >= self.files.panes.len() {
                break;
            }
            let mut cui = ui.new_child(egui::UiBuilder::new().id_salt(("pane", pane)).max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
            cui.set_clip_rect(r.intersect(ui.clip_rect()));
            self.pane_ui(&mut cui, pane);
        }
        ui.allocate_rect(rest, Sense::hover());
    }

    /// Where each pane goes, with a divider to drag between two.
    fn pane_rects(&mut self, ui: &mut egui::Ui, rest: Rect) -> Vec<Rect> {
        if self.files.panes.len() < 2 {
            return vec![rest];
        }
        let pal = style::pal(ui);
        let vertical = self.files.prefs.split != Split::Horizontal;
        let ratio = self.files.prefs.split_ratio.clamp(0.15, 0.85);
        let gap = 5.0;
        let (a, b, line) = if vertical {
            let x = rest.left() + rest.width() * ratio;
            (Rect::from_min_max(rest.min, pos2(x - gap / 2.0, rest.bottom())), Rect::from_min_max(pos2(x + gap / 2.0, rest.top()), rest.max), Rect::from_center_size(pos2(x, rest.center().y), vec2(gap + 2.0, rest.height())))
        } else {
            let y = rest.top() + rest.height() * ratio;
            (Rect::from_min_max(rest.min, pos2(rest.right(), y - gap / 2.0)), Rect::from_min_max(pos2(rest.left(), y + gap / 2.0), rest.max), Rect::from_center_size(pos2(rest.center().x, y), vec2(rest.width(), gap + 2.0)))
        };
        let resp = ui.interact(line, ui.id().with("pane-divider"), Sense::click_and_drag()).on_hover_cursor(if vertical { egui::CursorIcon::ResizeColumn } else { egui::CursorIcon::ResizeRow });
        ui.painter().rect_filled(line, 0.0, if resp.hovered() || resp.dragged() { pal.hairline } else { pal.chrome });
        if resp.dragged()
            && let Some(p) = ui.ctx().pointer_interact_pos()
        {
            let r = if vertical { (p.x - rest.left()) / rest.width() } else { (p.y - rest.top()) / rest.height() };
            self.files.prefs.split_ratio = r.clamp(0.15, 0.85);
        }
        if resp.double_clicked() {
            self.files.prefs.split_ratio = 0.5;
        }
        resp.on_hover_text("Drag to share the space; double-click to make the panes equal");
        vec![a, b]
    }

    /// One pane: its tabs, and the folder or Computer view under them.
    fn pane_ui(&mut self, ui: &mut egui::Ui, pane: usize) {
        let rect = ui.max_rect();
        let pal = style::pal(ui);
        let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        if pressed.is_some_and(|p| rect.contains(p)) {
            self.files.set_focus(pane);
            self.files.focused = true;
        }
        // Everything this pane does applies to it, even while the other one has the focus.
        let keep = self.files.focus;
        self.files.focus = pane;
        let mut picks = FPicks::default();
        self.pane_bar(ui, keep == pane);
        self.files_tabs_row(ui, pane, keep == pane, &mut picks);
        let page = ui.available_rect_before_wrap();
        ui.painter().rect_filled(page, 0.0, pal.paper);
        if self.files.panes.len() > 1 && keep == pane {
            // The focused pane: a thin accent line along the top of its page.
            ui.painter().hline(page.x_range(), page.top() + 0.5, Stroke::new(1.0, style::blend(pal.paper, pal.accent, 0.6)));
        }
        let mut page_ui = ui.new_child(egui::UiBuilder::new().max_rect(page).layout(egui::Layout::top_down(egui::Align::Min)));
        page_ui.set_clip_rect(page.intersect(ui.clip_rect()));
        match self.files.here().clone() {
            Location::Computer => {
                egui::Frame::new().inner_margin(egui::Margin::same(14)).show(&mut page_ui, |ui| self.computer_view(ui, &mut picks));
            }
            Location::Dir(dir) => self.details_view(&mut page_ui, pane, keep == pane, &dir, &mut picks),
        }
        self.files_apply(picks);
        self.files.focus = keep.min(self.files.panes.len() - 1);
    }

    /// The tabs: each with its folder icon, the active one joined to the page below; drag one
    /// to reorder, middle-click or ✖ to close. Too many to fit: the strip scrolls sideways.
    /// Dragging items over a tab opens it after a moment; dropping puts them in its folder.
    fn files_tabs_row(&mut self, ui: &mut egui::Ui, pane: usize, focused: bool, picks: &mut FPicks) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        let pal = style::pal(ui);
        let strip = Rect::from_min_size(ui.available_rect_before_wrap().min, vec2(ui.available_width(), TAB_STRIP_H));
        ui.allocate_rect(strip, Sense::hover());
        ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, pal.hairline));
        let n = self.files.panes[pane].tabs.len();
        let active = self.files.panes[pane].active;
        let focused_pane = self.files.panes.len() < 2 || focused;
        let plus_w = 30.0;
        let avail = (strip.width() - plus_w - 8.0).max(80.0);
        let tab_w = (avail / n as f32).clamp(TAB_MIN_W.min(avail), TAB_MAX_W);
        let total = tab_w * n as f32;
        let mut strip_ui = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Max)));
        let mut drag_to = None;
        let mut hover_tab = None;
        let mut fav_to_tab = None;
        let now = ctx.input(|i| i.time);
        egui::ScrollArea::horizontal()
            .id_salt(("fm-tabs", pane))
            .max_width(avail)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut strip_ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut rects = Vec::with_capacity(n);
                for i in 0..n {
                    let here = self.files.panes[pane].tabs[i].here().clone();
                    let on = i == active;
                    let (r, resp) = ui.allocate_exact_size(vec2(tab_w, TAB_STRIP_H - 4.0), Sense::click_and_drag());
                    rects.push(r);
                    let hovered = resp.hovered() || resp.dragged();
                    let p = ui.painter();
                    let shape = egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 };
                    if on {
                        let body = Rect::from_min_max(r.min, pos2(r.right(), r.bottom() + 1.0));
                        p.rect(body, shape, pal.paper, Stroke::new(1.0, pal.hairline), egui::StrokeKind::Inside);
                        p.rect_filled(Rect::from_min_max(pos2(r.left() + 1.0, r.bottom() - 1.0), pos2(r.right() - 1.0, r.bottom() + 1.5)), 0.0, pal.paper);
                        let line = if focused_pane { pal.accent } else { pal.hairline };
                        p.rect_filled(Rect::from_min_size(r.min + vec2(6.0, 0.0), vec2(r.width() - 12.0, 2.0)), 1.0, line);
                    } else if hovered {
                        p.rect_filled(r.shrink2(vec2(1.0, 2.0)), shape, pal.hover);
                    } else if i + 1 != active && i + 1 < n {
                        p.vline(r.right() - 0.5, r.y_range().shrink(7.0), Stroke::new(1.0, pal.hairline));
                    }
                    let title = location_title(&here);
                    let tc = if on { pal.strong } else { pal.text };
                    let icon = if matches!(here, Location::Computer) { "🖳" } else { "🗀" };
                    let cy = r.center().y + 1.0;
                    p.text(pos2(r.left() + 10.0, cy), Align2::LEFT_CENTER, icon, FontId::proportional(13.0), if on && focused_pane { pal.accent } else { pal.weak });
                    let show_x = n > 1 && (on || hovered);
                    let clip = Rect::from_min_max(pos2(r.left() + 28.0, r.top()), pos2(r.right() - if show_x { 24.0 } else { 8.0 }, r.bottom()));
                    p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(clip.left(), cy), Align2::LEFT_CENTER, &title, FontId::proportional(12.5), tc);
                    let tip = match &here {
                        Location::Dir(p) => p.shown(),
                        Location::Computer => places::computer_name().to_string(),
                    };
                    let mut closed = false;
                    if show_x {
                        let xr = Rect::from_center_size(pos2(r.right() - 13.0, cy), vec2(16.0, 16.0));
                        let x = ui.interact(xr, ui.id().with(("tab-x", pane, i)), Sense::click());
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
                    // Items dragged over another tab: open it after a moment; a drop goes there.
                    if resp.dnd_hover_payload::<dnd::DragItems>().is_some() && !on {
                        hover_tab = Some(i);
                    }
                    if let Location::Dir(d) = &here
                        && let Some(drop) = self.folder_drop(ui, &resp, d)
                    {
                        picks.drop = Some(drop);
                    }
                    if let Some(p) = self.fav_dropped(&resp) {
                        fav_to_tab = Some((i, p));
                    }
                    let resp = resp.on_hover_text(tip);
                    if !closed && resp.clicked() && !on {
                        picks.switch_tab = Some(i);
                    }
                    if resp.middle_clicked() && n > 1 {
                        picks.close_tab = Some(i);
                    }
                    resp.context_menu(|ui| {
                        if ui.add(egui::Button::new(style::mi("New Tab"))).clicked() {
                            picks.new_tab = Some(here.clone());
                        }
                        if ui.add_enabled(n > 1, egui::Button::new(style::mi("Close Tab"))).clicked() {
                            picks.close_tab = Some(i);
                        }
                        if ui.add_enabled(n > 1, egui::Button::new(style::mi("Close Other Tabs"))).clicked() {
                            picks.close_others = Some(i);
                        }
                    });
                }
                if let Some((i, x)) = drag_to {
                    if i > 0 && x < rects[i - 1].center().x {
                        picks.move_tab = Some((i, i - 1));
                    } else if i + 1 < n && x > rects[i + 1].center().x {
                        picks.move_tab = Some((i, i + 1));
                    }
                }
            });
        match hover_tab {
            Some(i) => match self.files.drag_hover_tab {
                Some((key, since)) if key == (pane, i) => {
                    if now - since > 0.6 {
                        picks.switch_tab = Some(i);
                        self.files.drag_hover_tab = None;
                    } else {
                        ctx.request_repaint_after(Duration::from_millis(50));
                    }
                }
                _ => self.files.drag_hover_tab = Some(((pane, i), now)),
            },
            None if self.files.drag_hover_tab.is_some_and(|((p, _), _)| p == pane) => self.files.drag_hover_tab = None,
            None => {}
        }
        if let Some((i, p)) = fav_to_tab {
            self.files.panes[pane].active = i;
            picks.go = Some(Location::Dir(p));
        }
        let plus_at = pos2((strip.left() + 6.0 + total.min(avail) + 4.0).min(strip.right() - plus_w), strip.top() + 4.0);
        let plus_r = Rect::from_min_size(plus_at, vec2(26.0, TAB_STRIP_H - 8.0));
        let plus = ui.interact(plus_r, ui.id().with(("tab-plus", pane)), Sense::click());
        if plus.hovered() {
            ui.painter().rect_filled(plus_r, 5.0, pal.hover);
        }
        ui.painter().text(plus_r.center(), Align2::CENTER_CENTER, "+", FontId::proportional(17.0), if plus.hovered() { pal.strong } else { pal.weak });
        if plus.on_hover_text(format!("New tab ({})", k(egui::Modifiers::COMMAND, egui::Key::T))).clicked() {
            picks.new_tab = Some(self.files.panes[pane].tab().here().clone());
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
                    let resp = resp.on_hover_text(root.shown());
                    if resp.double_clicked() || resp.clicked() {
                        picks.go = Some(Location::Dir(root.clone()));
                    }
                    if resp.middle_clicked() {
                        picks.new_tab = Some(Location::Dir(root.clone()));
                    }
                    let pinned = self.files.is_pinned(&root);
                    resp.context_menu(|ui| {
                        if ui.add(egui::Button::new(style::mi("Analyse This Drive"))).on_hover_text("Scan it and show its treemap in the analysis dock").clicked() {
                            picks.menu = Some((FAction::Analyse, vec![root.clone()]));
                        }
                        ui.separator();
                        place_menu(ui, &root, pinned, false, picks);
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
    fn details_view(&mut self, ui: &mut egui::Ui, pane: usize, pane_focused: bool, dir: &Path, picks: &mut FPicks) {
        let view = self.files.pane().view;
        let row_h = self.files.prefs.density.row();
        let pal = style::pal(ui);
        let footer_h = 22.0;
        let mut body = ui.available_rect_before_wrap();
        body.max.y -= footer_h;
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom()), pos2(body.right(), body.bottom() + footer_h));
        // In Columns, this folder is one column among others.
        let lay = (view == FView::Columns).then(|| views::columns_layout(body, dir));
        let list_rect = lay.as_ref().map_or(body, |l| l.mid);
        let width = list_rect.width();
        let cols = if view == FView::Columns { vec![(Some(FSort::Name), width)] } else { Self::details_columns(width, self.files.prefs.col_widths) };
        // The background: a click on empty space clears the selection; right-click for a menu.
        let empty = ui.interact(list_rect, ui.id().with("fm-files-body"), Sense::click());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(list_rect.intersect(ui.clip_rect()));
        let ui2 = &mut child;

        if view == FView::Details {
            // Headings, with dividers you can drag to resize the column to their right.
            let hdr_h = 26.0;
            let (hdr, _) = ui2.allocate_exact_size(vec2(width, hdr_h), Sense::hover());
            ui2.painter().hline(hdr.x_range(), hdr.bottom() - 0.5, Stroke::new(1.0, pal.hairline));
            let mut x = hdr.left();
            let (sort, reversed) = (self.files.pane().sort, self.files.pane().reversed);
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
                let title = match (col, scan::metric()) {
                    (FSort::Size, scan::Metric::Disk) => "On disk",
                    (FSort::Size, scan::Metric::CloudOnly) => "Cloud only",
                    (FSort::Size, scan::Metric::Reclaimable) => "Reclaimable",
                    _ => col.title(),
                };
                let g = ui2.painter().layout_no_wrap(title.to_string(), font, color);
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
        }

        let focused = self.files.focused && pane_focused;
        let stripes = self.view().stripes;
        let t = self.files.cur();
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
                        ui.weak("Network drives and sleeping disks can take a while. You can keep using Filescape meanwhile.");
                        if ui.button("Cancel").clicked() {
                            picks.clear = true;
                        }
                    }
                } else if let Some(e) = &error {
                    big(ui, "⚠");
                    ui.colored_label(ui.visuals().error_fg_color, e);
                    ui.horizontal(|ui| {
                        if ui.button("Try Again").clicked() {
                            picks.menu = Some((FAction::Reload, Vec::new()));
                        }
                        if ui.button("Go up").clicked() {
                            picks.go = Location::Dir(dir.to_path_buf()).up();
                        }
                    });
                } else if total > 0 && !self.files.pane().filter.trim().is_empty() {
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
        } else if view == FView::Icons {
            self.icons_grid(ui2, pane, focused, dir, picks);
        } else if view == FView::Gallery {
            self.gallery(ui2, pane, focused, dir, picks);
        } else {
            let sizes = t.sizes.as_ref().map(|s| &s.1);
            let cursor_row = t.cursor.as_deref().and_then(|c| t.pos_of(c));
            let mut area = egui::ScrollArea::vertical().id_salt(("flist", pane, dir)).auto_shrink(false);
            if let Some(r) = cursor_row.filter(|_| t.scroll_to_cursor) {
                let line = views::line_of(&t.groups, r);
                area = area.vertical_scroll_offset(browser::keep_in_view(t.view, line as f32 * row_h, row_h));
            }
            let heads = &t.groups;
            let chevrons = view == FView::Columns;
            let cloud_icons = self.view().cloud_icons;
            let sys_notes = self.settings.system_notes && !crate::fsys::remote::is_remote(dir);
            let mut explain_pick = None;
            ui2.spacing_mut().item_spacing.y = 0.0;
            let vis = ui2.visuals().clone();
            let (sel_bg, sel_fg) = if focused { (pal.sel, pal.strong) } else { (pal.sel_dim, pal.strong) };
            let font = FontId::proportional(13.0);
            let order = &t.order;
            let colors = &self.colors;
            let color_for = |ext: &str| colors.get(ext).or_else(|| colors.get(&ext.to_lowercase())).copied().unwrap_or(Color32::GRAY);
            let mut hovered_row = None;
            let mut rows_drop: Vec<(egui::Response, PathBuf)> = Vec::new();
            let two_panes = self.files.panes.len() > 1;
            let can_paste = self.can_paste();
            let out = area.show_rows(ui2, row_h, order.len() + heads.len(), |ui, range| {
                for line in range {
                    let r = match views::line_at(heads, line) {
                        views::Line::Item(pos) => pos,
                        views::Line::Head(g) => {
                            // A group heading: its title, how many items, and a rule.
                            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                            let h = &heads[g];
                            let end = heads.get(g + 1).map_or(order.len(), |n| n.start as usize);
                            let p = ui.painter();
                            let g1 = p.layout_no_wrap(h.label.clone(), FontId::proportional(12.5), pal.accent);
                            let x = rect.left() + 10.0;
                            let y = rect.center().y + 2.0;
                            let w1 = g1.size().x;
                            p.galley(pos2(x, y - g1.size().y / 2.0), g1, pal.accent);
                            let g2 = p.layout_no_wrap(format!("({})", group((end - h.start as usize) as u64)), FontId::proportional(11.5), pal.weak);
                            let w2 = g2.size().x;
                            p.galley(pos2(x + w1 + 6.0, y - g2.size().y / 2.0), g2, pal.weak);
                            let rule = x + w1 + w2 + 14.0;
                            if rule < rect.right() - 10.0 {
                                p.hline(rule..=rect.right() - 10.0, y, Stroke::new(1.0, pal.hairline));
                            }
                            continue;
                        }
                    };
                    let i = order[r];
                    let e = &l.entries[i as usize];
                    let seq = t.seqs.get(&i);
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
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
                                let mark = if self.files.marks.is_empty() {
                                    None
                                } else {
                                    self.files.marks.get(&dir.join(&*e.name))
                                };
                                if let Some(m) = mark {
                                    marksui::paint_dot(
                                        &p,
                                        icon,
                                        m,
                                        if selected { sel_bg } else { pal.paper },
                                    );
                                }
                                if e.is_link {
                                    p.text(icon.left_bottom() + vec2(-3.0, 2.0), Align2::LEFT_BOTTOM, "↪", FontId::proportional(10.0), vis.strong_text_color());
                                }
                                let mut end = if chevrons && e.is_dir {
                                    views::chevron(&p, pos2(cell.right() - 11.0, cy), if selected { sel_fg } else { pal.weak });
                                    cell.right() - 18.0
                                } else {
                                    cell.right() - 6.0
                                };
                                let st = scan::Stored::of(e.flags);
                                if cloud_icons && !e.is_dir && st != scan::Stored::Local {
                                    let at = Rect::from_center_size(pos2(end - 8.0, cy), vec2(14.0, 14.0));
                                    let tip = || cloud::stored_tip(st, None, Some(dir));
                                    cloud::paint_stored(ui, at, st, ui.id().with(("fcloud", &*e.name)), tip);
                                    end -= 20.0;
                                }
                                if sys_notes && let Some(kn) = crate::known::explain(&dir.join(&*e.name)) {
                                    let at = Rect::from_center_size(pos2(end - 8.0, cy), vec2(14.0, 14.0));
                                    if explain::paint_badge(ui, at, kn, ui.id().with(("fsys", &*e.name))) {
                                        explain_pick = Some(dir.join(&*e.name));
                                    }
                                    end -= 20.0;
                                }
                                let clip = Rect::from_min_max(pos2(cell.left() + 33.0, rect.top()), pos2(end, rect.bottom()));
                                let name = seq.map_or(&*e.name, |s| s.label.as_str());
                                let g = p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(clip.left(), cy), Align2::LEFT_CENTER, name, font.clone(), tc);
                                let mut after = g.right();
                                if let Some(s) = seq {
                                    // The frame range after the name, and a warning for missing frames.
                                    let gaps = if s.seq.missing > 0 { format!("  ⚠ {} missing", group(s.seq.missing)) } else { String::new() };
                                    let note = format!("  {}{gaps}", s.seq.range_text());
                                    after = p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(g.right(), cy), Align2::LEFT_CENTER, note, font.clone(), if s.seq.missing > 0 && !selected { vis.warn_fg_color } else { weak }).right();
                                }
                                if let Some(m) = mark {
                                    marksui::paint_tags(&p.with_clip_rect(clip.intersect(p.clip_rect())), pos2(after + 8.0, cy), m, weak);
                                }
                            }
                            Some(FSort::Size) if seq.is_some() => {
                                p.text(right, Align2::RIGHT_CENTER, scan::human(seq.unwrap().seq.bytes), font.clone(), tc);
                            }
                            Some(FSort::Type) if seq.is_some() => {
                                let s = seq.unwrap();
                                let kind = format!("{} sequence, {} frames", s.seq.extension().to_uppercase(), group(s.seq.frames));
                                p.with_clip_rect(cell.shrink2(vec2(6.0, 0.0)).intersect(p.clip_rect())).text(pos2(cell.left() + 8.0, cy), Align2::LEFT_CENTER, kind, font.clone(), weak);
                            }
                            Some(FSort::Modified) if seq.is_some_and(|s| s.modified > 0) => {
                                p.text(pos2(cell.left() + 8.0, cy), Align2::LEFT_CENTER, fmt_time(seq.unwrap().modified), font.clone(), weak);
                            }
                            Some(FSort::Size) => {
                                if !e.is_dir {
                                    p.text(right, Align2::RIGHT_CENTER, scan::human(e.shown_size()), font.clone(), tc);
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
                    let pinned = e.is_dir && self.files.is_pinned(&dir.join(&*e.name));
                    let ic = ItemCtx { t, l, dir, sizes, two: two_panes, can_paste, marks: &self.files.marks, apps: &self.files.prefs.apps };
                    item_actions(ui, resp, i, &ic, pinned, picks, &mut rows_drop);
                }
            });
            let _ = hovered_row;
            if let Some(p) = explain_pick {
                self.explain_path(p);
            }
            for (resp, p) in rows_drop {
                if let Some(d) = self.folder_drop(ui, &resp, &p) {
                    picks.drop = Some(d);
                }
            }
            let t = self.files.tab();
            t.view = (out.state.offset.y, out.inner_rect.height());
            t.scroll_to_cursor = false;
        }
        if let Some(lay) = &lay {
            self.columns_sides(ui, pane, dir, lay, picks);
        }
        if empty.clicked() {
            picks.clear = true;
        }
        let here = dir.to_path_buf();
        // A drop on the empty part of the list goes into this folder; a favourite opens here.
        if !self.files.drop_claimed
            && let Some(d) = self.folder_drop(ui, &empty, &here)
        {
            picks.drop = Some(d);
        }
        if let Some(p) = self.fav_dropped(&empty) {
            picks.go = Some(Location::Dir(p));
        }
        let pinned = self.files.is_pinned(&here);
        let can_paste = self.can_paste();
        let mut new_pick = None;
        let undo_label = self.ops.undo_label().map(str::to_string);
        let redo_label = self.ops.redo_label().map(str::to_string);
        let busy = self.ops.job.is_some();
        let mut undo_now = false;
        let mut redo_now = false;
        empty.context_menu(|ui| {
            let ctx = ui.ctx().clone();
            let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
            ui.menu_button(style::mi("New"), |ui| {
                new_pick = self.new_menu_items(ui, &here);
            });
            if ui.add_enabled(can_paste, egui::Button::new(style::mi("Paste")).shortcut_text(k(egui::Modifiers::COMMAND, egui::Key::V))).clicked() {
                picks.menu = Some((FAction::Paste, vec![here.clone()]));
            }
            let undo = undo_label.as_deref().map_or("Undo".to_string(), |w| format!("Undo {w}"));
            if ui.add_enabled(undo_label.is_some() && !busy, egui::Button::new(style::mi(undo)).shortcut_text(k(egui::Modifiers::COMMAND, egui::Key::Z))).clicked() {
                undo_now = true;
            }
            if let Some(w) = &redo_label
                && ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(style::mi(format!("Redo {w}")))
                            .shortcut_text(k(egui::Modifiers::COMMAND, egui::Key::Y)),
                    )
                    .clicked()
            {
                redo_now = true;
            }
            ui.separator();
            if ui.add(egui::Button::new(style::mi("Refresh")).shortcut_text(k(egui::Modifiers::NONE, egui::Key::F5))).clicked() {
                picks.menu = Some((FAction::Open, Vec::new()));
            }
            if ui.add(egui::Button::new(style::mi("Open in New Tab"))).clicked() {
                picks.new_tab = Some(Location::Dir(here.clone()));
            }
            ui.separator();
            if ui.add(egui::Button::new(style::mi("Copy Path"))).clicked() {
                picks.menu = Some((FAction::CopyPath, vec![here.clone()]));
            }
            if let Some(a) = folder_menu(ui, pinned) {
                picks.menu = Some((a, vec![here.clone()]));
            }
        });
        if let Some(p) = new_pick {
            picks.new_item = Some((p, here.clone()));
        }
        if undo_now {
            self.undo_last();
        }
        if redo_now {
            self.redo_last();
        }
        // The four views, at the right of the footer as in Explorer.
        let reserve = 104.0;
        self.files_footer(ui, footer, pane, reserve);
        let mut x = footer.right() - reserve - 4.0;
        for v in FView::ALL {
            let r = Rect::from_min_size(pos2(x + 4.0, footer.top() + 2.0), vec2(22.0, footer_h - 4.0));
            x += 26.0;
            let on = self.files.panes[pane].view == v;
            let resp = ui.interact(r, ui.id().with(("fview", pane, v.title())), Sense::click()).on_hover_text(v.title());
            let pal = style::pal(ui);
            if on || resp.hovered() {
                ui.painter().rect_filled(r, style::RADIUS as f32, if on { pal.sel } else { pal.hover });
            }
            v.paint(ui.painter(), r, if on { pal.strong } else { pal.weak });
            if resp.clicked() {
                self.files.panes[pane].view = v;
            }
        }
        ui.allocate_rect(Rect::from_min_max(body.min, footer.max), Sense::hover());
    }

    /// The Icons view: a grid of big icons, images showing their thumbnails, names under them.
    /// Clicks, drags and menus work as in Details. Ctrl+wheel (⌘ on a Mac) changes the size.
    fn icons_grid(&mut self, ui: &mut egui::Ui, pane: usize, focused: bool, dir: &Path, picks: &mut FPicks) {
        let pal = style::pal(ui);
        // Ctrl+wheel or a pinch over the grid: bigger or smaller icons.
        if ui.rect_contains_pointer(ui.max_rect()) {
            let z = ui.input(|i| i.zoom_delta());
            if z != 1.0 {
                let s = &mut self.files.prefs.icon_size;
                *s = (*s * z).clamp(ICON_MIN, ICON_MAX);
            }
        }
        let size = self.files.prefs.icon_size.clamp(ICON_MIN, ICON_MAX).round();
        let width = (ui.available_width() - 8.0).max(1.0);
        let min_w = (size + 28.0).max(116.0);
        let cols = ((width / min_w).floor() as usize).max(1);
        let cell_w = width / cols as f32;
        let cell_h = size + 52.0;
        self.files.panes[pane].grid = (cols, cell_h);
        let can_paste = self.can_paste();
        let two_panes = self.files.panes.len() > 1;
        let sys_notes = self.settings.system_notes && !crate::fsys::remote::is_remote(dir);
        let files = &mut self.files;
        let (panes, thumb_cache, favourites, marks, apps) = (&files.panes, &mut files.thumbs, &files.prefs.favourites, &files.marks, &files.prefs.apps);
        let icons = &mut self.icons;
        let t = panes[pane].tab();
        let Some(l) = t.listing.as_ref() else { return };
        let n = t.order.len();
        let rows = n.div_ceil(cols);
        let sizes = t.sizes.as_ref().map(|s| &s.1);
        let cursor = t.cursor.as_deref().and_then(|c| t.pos_of(c));
        let mut area = egui::ScrollArea::vertical().id_salt(("fgrid", pane, dir)).auto_shrink(false);
        if let Some(r) = cursor.filter(|_| t.scroll_to_cursor) {
            area = area.vertical_scroll_offset(browser::keep_in_view(t.view, (r / cols) as f32 * cell_h, cell_h));
        }
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (sel_bg, sel_fg) = if focused { (pal.sel, pal.strong) } else { (pal.sel_dim, pal.strong) };
        let colors = &self.colors;
        let color_for = |ext: &str| colors.get(ext).or_else(|| colors.get(&ext.to_lowercase())).copied().unwrap_or(Color32::GRAY);
        let mut drops: Vec<(egui::Response, PathBuf)> = Vec::new();
        let out = area.show_rows(ui, cell_h, rows, |ui, range| {
            for row in range {
                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), cell_h), Sense::hover());
                for c in 0..cols {
                    let k = row * cols + c;
                    if k >= n {
                        break;
                    }
                    let i = t.order[k];
                    let e = &l.entries[i as usize];
                    let cell = Rect::from_min_size(pos2(line.left() + 4.0 + c as f32 * cell_w, line.top()), vec2(cell_w, cell_h)).shrink(3.0);
                    let resp = ui.interact(cell, ui.id().with(("fcell", k)), Sense::click_and_drag());
                    let selected = t.selected.contains(&e.name);
                    let p = ui.painter().clone();
                    if selected {
                        p.rect_filled(cell, style::RADIUS as f32 + 2.0, sel_bg);
                    } else if resp.hovered() {
                        p.rect_filled(cell, style::RADIUS as f32 + 2.0, pal.hover);
                    }
                    if focused && cursor == Some(k) && !selected {
                        p.rect_stroke(cell, style::RADIUS as f32 + 2.0, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                    }
                    // The picture: a thumbnail for images once it's ready, else the type's icon.
                    let pic = Rect::from_center_size(pos2(cell.center().x, cell.top() + 6.0 + size / 2.0), vec2(size, size));
                    let path = dir.join(&*e.name);
                    let thumb = if !e.is_dir && !e.not_local() && thumbs::wants_thumb(&e.name) { thumb_cache.get(&path, thumbs::stamp(e.size, e.modified), e.size).map(|t| (t.id(), t.size_vec2())) } else { None };
                    if let Some((id, tsize)) = thumb {
                        let scale = (size / tsize.x).min(size / tsize.y).min(2.0);
                        let r = Rect::from_center_size(pic.center(), tsize * scale);
                        p.rect_filled(r.expand(1.0), 2.0, pal.hairline);
                        p.image(id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    } else {
                        let faint = e.hidden;
                        let tint = if faint { Color32::from_gray(120) } else { color_for(e.ext()) };
                        icons.paint_big(ui, Rect::from_center_size(pic.center(), vec2(size * 0.8, size * 0.8)), e.is_dir, e.ext(), tint);
                    }
                    if e.is_link {
                        p.text(pic.left_bottom() + vec2(size * 0.1, -2.0), Align2::LEFT_BOTTOM, "↪", FontId::proportional(14.0), pal.strong);
                    }
                    if let Some(m) = (!marks.is_empty()).then(|| marks.get(&path)).flatten() {
                        marksui::paint_dot(&p, pic.shrink(size * 0.1), m, pal.paper);
                        if !m.tags.is_empty() || !m.note.trim().is_empty() {
                            let tip = m
                                .tags
                                .iter()
                                .map(|t| format!("#{t}"))
                                .collect::<Vec<_>>()
                                .join(" ");
                            ui.interact(cell, egui::Id::new(("mark-tip", i)), Sense::hover())
                                .on_hover_text(if m.note.trim().is_empty() {
                                    tip
                                } else {
                                    format!("{tip}\n{}", m.note)
                                });
                        }
                    }
                    // The name: up to two lines, centred, shortened with … when longer.
                    let tc = if selected { sel_fg } else if e.hidden { pal.weak } else { pal.text };
                    let shown = t.seqs.get(&i).map_or_else(|| e.name.to_string(), |s| format!("{}\n{} frames", s.label, group(s.seq.frames)));
                    let mut job = egui::text::LayoutJob::simple(shown, FontId::proportional(12.5), tc, cell.width() - 10.0);
                    job.wrap.max_rows = 2;
                    job.wrap.break_anywhere = true;
                    job.halign = egui::Align::Center;
                    let g = ui.fonts_mut(|f| f.layout_job(job));
                    p.galley(pos2(cell.center().x, pic.bottom() + 6.0), g, tc);
                    let pinned = e.is_dir && favourites.iter().any(|g| g.items.iter().any(|f| f.path == path));
                    let ic = ItemCtx { t, l, dir, sizes, two: two_panes, can_paste, marks, apps };
                    let mut tip = if e.is_dir { e.name.to_string() } else { format!("{}\n{}", e.name, scan::human(e.size)) };
                    if let Some(kn) = crate::known::explain(&path).filter(|_| sys_notes) {
                        tip = format!("{tip}\n\n{}", kn.text().replace('`', ""));
                    }
                    let resp = resp.on_hover_text_at_pointer(tip);
                    item_actions(ui, resp, i, &ic, pinned, picks, &mut drops);
                }
            }
        });
        for (resp, p) in drops {
            if let Some(d) = self.folder_drop(ui, &resp, &p) {
                picks.drop = Some(d);
            }
        }
        let t = self.files.panes[pane].tab_mut();
        t.view = (out.state.offset.y, out.inner_rect.height());
        t.scroll_to_cursor = false;
    }

    /// Count of items, what's selected, and how the listing is going.
    fn files_footer(&self, ui: &egui::Ui, rect: Rect, pane: usize, reserve: f32) {
        let p = ui.painter();
        let pal = style::pal(ui);
        p.rect_filled(rect, 0.0, pal.paper);
        p.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, pal.hairline));
        let t = self.files.panes[pane].tab();
        let Some(l) = t.listing.as_ref() else { return };
        let shown = t.order.len();
        let mut text = format!("{} item{}", group(shown as u64), if shown == 1 { "" } else { "s" });
        if t.search.is_some() {
            text.push_str(if t.search_deep {
                " found here and in subfolders"
            } else {
                " found"
            });
        }
        // Only once the order covers everything read so far, or the count would be off.
        let current = t.order_key.as_ref().is_some_and(|k| k.0 == l.generation);
        let folded: usize = t.seqs.values().map(|s| s.members.len() - 1).sum();
        if !t.seqs.is_empty() {
            let frames: usize = t.seqs.values().map(|s| s.members.len()).sum();
            let n = t.seqs.len();
            text.push_str(&format!(", {} frames in {} sequence{}", group(frames as u64), group(n as u64), if n == 1 { "" } else { "s" }));
        }
        let left_out = if current { (l.entries.len() - shown).saturating_sub(folded) } else { 0 };
        if left_out > 0 {
            let why = if self.files.panes[pane].filter.trim().is_empty() { "hidden" } else { "filtered out" };
            text.push_str(&format!(" ({} {why})", group(left_out as u64)));
        }
        if !l.done {
            text.push_str(if t.search.is_some() { " · searching…" } else { " · reading…" });
        } else if t.search.is_some()
            && let Some(e) = &l.error
        {
            text.push_str(&format!(" · {e}"));
        }
        let sel_rows: Vec<u32> = t.order.iter().copied().filter(|&i| t.selected.contains(&l.entries[i as usize].name)).collect();
        let sel: Vec<&Entry> = sel_rows.iter().map(|&i| &l.entries[i as usize]).collect();
        if !sel.is_empty() {
            let bytes: u64 = sel_rows.iter().filter(|&&i| !l.entries[i as usize].is_dir).map(|i| t.seqs.get(i).map_or(l.entries[*i as usize].shown_size(), |s| s.seq.bytes)).sum();
            let dirs = sel.iter().filter(|e| e.is_dir).count();
            text.push_str(&format!("  ·  {} selected", group(sel.len() as u64)));
            if sel.len() > dirs {
                text.push_str(&format!(" ({}{})", scan::human(bytes), if dirs > 0 { " in files" } else { "" }));
            }
        }
        if let Some(e) = &l.error {
            text.push_str(&format!("  ·  {e}"));
        }
        // In split view each pane says where it is, at the right of its footer.
        let mut right = rect.right() - 10.0 - reserve;
        if self.files.panes.len() > 1
            && let Some(d) = t.here().dir()
        {
            let g = p.layout_no_wrap(d.shown(), FontId::proportional(11.5), pal.weak);
            let w = g.size().x.min(rect.width() * 0.5);
            let clip = Rect::from_min_max(pos2(right - w, rect.top()), pos2(right, rect.bottom()));
            p.with_clip_rect(clip).galley(pos2(right - g.size().x.min(w), rect.center().y - g.size().y / 2.0), g, pal.weak);
            right -= w + 12.0;
        }
        let clip = Rect::from_min_max(rect.min, pos2(right, rect.bottom()));
        p.with_clip_rect(clip).text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), pal.weak);
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
                if !error
                    && self.ops.undo_fresh
                    && let Some(what) = self.ops.undo_label()
                    && ui.small_button("Undo").on_hover_text(format!("Undo {what} ({})", if cfg!(target_os = "macos") { "⌘Z" } else { "Ctrl+Z" })).clicked()
                {
                    self.undo_last();
                }
                if ui.small_button("✖").on_hover_text("Dismiss").clicked() {
                    self.ops.notice = None;
                    self.ops.undo_fresh = false;
                }
            } else if let Some(dir) = self.files.here().dir() {
                ui.label(egui::RichText::new(dir.shown()).size(12.0).color(pal.weak));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.activity_chip(ui);
                self.transfers_chip(ui);
                self.transfer_strip(ui);
            });
        });
    }

    // ---------- input ----------

    fn files_keys(&mut self, ctx: &egui::Context, pointer_in: bool, picks: &mut FPicks) {
        use egui::{Key, Modifiers as M};
        // Not while a box is open over the list (Connect to Server, a server's question).
        if self.ops.modal() || self.files.connect.is_some() || self.files.asking.is_some() || self.files.player.is_some() || self.files.compare.is_some() || self.files.mark_edit.is_some() || self.files.apps_box.is_some() {
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
        // The Remote workspace with two panes: F5 copies and F6 moves to the other pane, as in
        // WinSCP's Commander (Ctrl+R reads the folder again).
        if let Some(a) = self.remote_keys(ctx) {
            picks.menu = Some((a, self.files.chosen_paths()));
            return;
        }
        // Shortcuts that work even while typing in the filter box.
        if ctx.input_mut(|i| i.consume_key(M::NONE, Key::F6)) {
            self.toggle_side_focus(ctx);
            return;
        }
        let (find, path, dock, split, preview) = ctx.input_mut(|i| {
            let path = i.consume_key(M::COMMAND, Key::L) || (!cfg!(target_os = "macos") && (i.consume_key(M::ALT, Key::D) || i.consume_key(M::NONE, Key::F4)));
            (i.consume_key(M::COMMAND, Key::F), path, i.consume_key(M::COMMAND, Key::J), i.consume_key(M::COMMAND, Key::Backslash), i.consume_key(M::ALT, Key::P))
        });
        if dock {
            self.files.prefs.dock.open = !self.files.prefs.dock.open;
        }
        if find {
            self.files.focus_filter = true;
        }
        if path {
            self.start_path_edit();
        }
        if split {
            self.toggle_split();
        }
        if preview {
            self.files.prefs.preview = !self.files.prefs.preview;
        }
        if ctx.egui_wants_keyboard_input() || !self.files.focused {
            return;
        }
        // Quick Look takes the arrow keys while it's open; Space and Escape close it there.
        let look = self.files.look.open;
        // In the Icons view ← and → move along a row, ↑ and ↓ a row up or down.
        let icons = self.files.pane().view == FView::Icons;
        // In Gallery ← and → move along the strip; in Columns they go up a folder, or into one.
        let strip = self.files.pane().view == FView::Gallery;
        let columns = self.files.pane().view == FView::Columns;
        let cursor_dir = self.files.cursor_entry().is_some_and(|e| e.is_dir);
        let cols = self.files.pane().grid.0.max(1) as i64;
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
            Act(FAction),
            NewFolder,
            Undo,
            Redo,
            OtherPane,
            Swap,
            Find(String),
        }
        let act = ctx.input_mut(|i| {
            // Ctrl+C, X and V arrive as Copy, Cut and Paste events (Paste only when the
            // clipboard holds text, so Ctrl+V is checked as a key too). Shift+Ctrl+C copies paths;
            // on Windows Shift+Delete can arrive as Cut.
            let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            let cut = i.events.iter().any(|e| matches!(e, egui::Event::Cut));
            let paste = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))) || i.consume_key(M::COMMAND, Key::V);
            if cut {
                return Some(Do::Act(if i.modifiers.shift && !i.modifiers.command { FAction::Delete } else { FAction::Cut }));
            }
            if copy {
                return Some(Do::Act(if i.modifiers.shift { FAction::CopyPath } else { FAction::Copy }));
            }
            if paste {
                return Some(Do::Act(FAction::Paste));
            }
            if !look && i.consume_key(M::NONE, Key::Space) {
                return Some(Do::Act(FAction::QuickLook));
            }
            if i.consume_key(M::SHIFT, Key::Delete) {
                return Some(Do::Act(FAction::Delete));
            }
            if i.consume_key(M::NONE, Key::Delete) || (cfg!(target_os = "macos") && i.consume_key(M::COMMAND, Key::Backspace)) {
                return Some(Do::Act(FAction::Trash));
            }
            if i.consume_key(M::NONE, Key::F2) {
                return Some(Do::Act(FAction::Rename));
            }
            if i.consume_key(M::ALT, Key::Enter) {
                return Some(Do::Act(FAction::Properties));
            }
            if cfg!(windows) && i.consume_key(M::SHIFT, Key::F10) {
                return Some(Do::Act(FAction::SystemMenu(false)));
            }
            if i.consume_key(M::SHIFT, Key::F5) {
                return Some(Do::Act(FAction::CopyToOther));
            }
            if i.consume_key(M::SHIFT, Key::F6) {
                return Some(Do::Act(FAction::MoveToOther));
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::N) {
                return Some(Do::NewFolder);
            }
            if i.consume_key(M::COMMAND, Key::U) {
                return Some(Do::Swap);
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::Z) || i.consume_key(M::COMMAND, Key::Y) {
                return Some(Do::Redo);
            }
            if i.consume_key(M::COMMAND, Key::Z) {
                return Some(Do::Undo);
            }
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
            if !look && i.consume_key(M::NONE, Key::Escape) {
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
            if i.consume_key(M::NONE, Key::Tab) {
                return Some(Do::OtherPane);
            }
            if i.consume_key(M::NONE, Key::F5) || i.consume_key(M::COMMAND, Key::R) {
                return Some(Do::Refresh);
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::Period) {
                return Some(Do::Hidden);
            }
            let shift = i.modifiers.shift;
            let m = if shift { M::SHIFT } else { M::NONE };
            if icons && i.consume_key(m, Key::ArrowDown) {
                return Some(Do::Move(cols));
            }
            if icons && i.consume_key(m, Key::ArrowUp) {
                return Some(Do::Move(-cols));
            }
            if columns && !look && i.consume_key(M::NONE, Key::ArrowLeft) {
                return Some(Do::Up);
            }
            if columns && !look && cursor_dir && i.consume_key(M::NONE, Key::ArrowRight) {
                return Some(Do::Open);
            }
            if i.consume_key(m, Key::ArrowDown) || ((look || icons || strip) && i.consume_key(m, Key::ArrowRight)) {
                return Some(Do::Move(1));
            }
            if i.consume_key(m, Key::ArrowUp) || ((look || icons || strip) && i.consume_key(m, Key::ArrowLeft)) {
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
        let page = match self.files.pane().view {
            FView::Details | FView::Columns => (((self.files.cur().view.1 / self.files.prefs.density.row()) as i64) - 1).max(1),
            FView::Gallery => (((self.files.cur().view.1 / 104.0) as i64) - 1).max(1),
            FView::Icons => {
                let (cols, h) = self.files.pane().grid;
                (((self.files.cur().view.1 / h) as i64) - 1).max(1) * cols.max(1) as i64
            }
        };
        let here = self.files.here().dir().map(Path::to_path_buf);
        match act {
            Do::Back => self.files.back(),
            Do::Forward => self.files.forward(),
            Do::Up => self.files.up(),
            Do::Open => {
                let t = self.files.cur();
                if let Some(i) = t.cursor.as_deref().and_then(|c| t.pos_of(c)).map(|r| t.order[r]) {
                    // On selected sequences: play them together (two compare A/B).
                    let clips = if t.seqs.contains_key(&i) { t.chosen_clips() } else { Vec::new() };
                    if clips.len() > 1 {
                        self.play_sequences(clips);
                    } else {
                        picks.open = Some(i);
                    }
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
            Do::CloseTab => picks.close_tab = Some(self.files.pane().active),
            Do::Reopen => {
                if let Some(l) = self.files.closed.pop() {
                    self.files.new_tab(l);
                }
            }
            Do::NextTab(fwd) => {
                let n = self.files.pane().tabs.len();
                let a = self.files.pane().active;
                picks.switch_tab = Some(if fwd { (a + 1) % n } else { (a + n - 1) % n });
            }
            Do::OtherPane => {
                if let Some(o) = self.files.other() {
                    self.files.set_focus(o);
                }
            }
            Do::Swap => self.files.swap_panes(),
            Do::Undo => self.undo_last(),
            Do::Redo => self.redo_last(),
            Do::Refresh => self.files.refresh(),
            Do::Hidden => self.files.prefs.show_hidden = !self.files.prefs.show_hidden,
            Do::NewFolder => {
                if let Some(d) = here {
                    self.files_new_folder(d);
                }
            }
            Do::Act(FAction::Open) if !self.files.cur().chosen_clips().is_empty() => {
                let clips = self.files.cur().chosen_clips();
                self.play_sequences(clips);
            }
            Do::Act(a) => {
                let paths = match a {
                    FAction::Paste => here.into_iter().collect(),
                    _ => self.files.chosen_paths(),
                };
                if !paths.is_empty() || a == FAction::QuickLook {
                    picks.menu = Some((a, paths));
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
                let t = self.files.cur();
                let found = t.listing.as_ref().and_then(|l| t.order.iter().position(|&i| l.entries[i as usize].name.to_lowercase().starts_with(&typed)));
                if let Some(r) = found {
                    self.files_move_cursor(|_, _| r as i64, false);
                }
            }
        }
    }

    /// Move the keyboard cursor to the row `to(current row, row count)` returns, selecting it
    /// (or, with Shift, everything from the anchor to it).
    pub(super) fn files_move_cursor(&mut self, to: impl Fn(Option<usize>, usize) -> i64, extend: bool) {
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
            let p = self.files.pane_mut();
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
            let t = self.files.cur();
            if let Some(c) = t.clip_of(i) {
                self.play_sequences(vec![c]);
            } else if let (Some(e), Some(dir)) = (t.entry(i), t.here().dir()) {
                let path = dir.join(&*e.name);
                if e.is_dir || (archive::is_archive(&e.name) && !archive::inside(&path)) {
                    // Archives open like folders (with a licence).
                    if e.is_dir || self.premium(crate::licence::Feature::Archives) {
                        self.files.go(Location::Dir(path));
                    }
                } else {
                    self.open_file(path);
                }
            }
        }
        if let Some(to) = picks.go {
            self.files.go(to);
        }
        if let Some((dir, name)) = picks.reveal {
            self.files.reveal(dir, vec![name]);
        }
        if let Some((pane, to)) = picks.go_other {
            self.files.go_in(pane, to);
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
        if let Some((items, dest, op)) = picks.drop {
            self.files_transfer(items, dest, op);
        }
        if let Some((p, dir)) = picks.new_item {
            self.apply_new(p, dir);
        }
    }

    /// The list's item menu for a tile on the dock's map, run through the same commands.
    pub(super) fn tile_menu(&mut self, ui: &mut egui::Ui, disk: &Path, is_dir: bool) {
        let archive = !is_dir && disk.file_name().is_some_and(|n| archive::is_archive(&n.to_string_lossy()));
        let colour = self.files.marks.get(disk).and_then(|m| m.colour);
        let caps = crate::fsys::remote::is_remote(disk).then(|| crate::fsys::remote::caps(disk));
        let m = RowMenu { is_dir, count: 1, seq: false, colour, pinned: self.files.is_pinned(disk), two: self.files.panes.len() > 1, can_paste: self.can_paste(), archive, read_only: false, server: caps.is_some(), ssh: caps.is_some_and(|c| c.ssh), cloud: caps.is_none() && crate::cloudops::supported() && super::cloud::on_demand_dir(disk), known: caps.is_none() && crate::known::about(disk).is_some() };
        let ext = disk
            .file_name()
            .map(|n| crate::scan::ext_of(&n.to_string_lossy()).to_string())
            .unwrap_or_default();
        let Some(a) = item_menu(ui, &m, &self.files.prefs.apps.clone(), &ext) else { return };
        match a {
            FAction::Open if is_dir || (archive && self.premium(crate::licence::Feature::Archives)) => self.files.go(Location::Dir(disk.to_path_buf())),
            FAction::Open if archive => {}
            FAction::Open => {
                let p = disk.to_path_buf();
                self.files.spawn(move || actions::open(&p));
            }
            a => self.files_action(a, vec![disk.to_path_buf()]),
        }
        ui.close();
    }

    /// Is this path a folder, as far as the list on screen knows?
    fn listed_dir(&self, p: &Path) -> bool {
        let t = self.files.cur();
        let archive = p.file_name().is_some_and(|n| archive::is_archive(&n.to_string_lossy())) && !archive::inside(p);
        archive || p.parent().is_none() || (t.here().dir() == p.parent() && p.file_name().and_then(|n| t.entry_named(&n.to_string_lossy())).is_some_and(|e| e.is_dir))
    }

    /// Open a file in its app. One inside an archive is taken out to a temporary folder first.
    pub(super) fn open_file(&mut self, p: PathBuf) {
        self.files.spawn(move || {
            if archive::inside(&p) {
                let tmp = archive_temp(&p).map_err(|e| format!("Couldn't take it out of the archive: {e}"))?;
                actions::open(&tmp)
            } else if remote::is_remote(&p) {
                // A copy fetched from the server; saves to it are sent back.
                let tmp = remote::edit::open_copy(&p).map_err(|e| format!("Couldn't fetch it from the server: {}", remote::describe(&e)))?;
                actions::open(&tmp)
            } else {
                actions::open(&p)
            }
        });
    }

    /// Take archives apart into new folders named after them: beside each archive, or in `dest`.
    pub(super) fn extract_archives(&mut self, archives: Vec<PathBuf>, dest: Option<PathBuf>) {
        if !self.premium(crate::licence::Feature::Archives) {
            return;
        }
        if self.ops.job.is_some() {
            self.ops.notice = Some(("Another file operation is still running.".into(), true));
            return;
        }
        let label = match archives.as_slice() {
            [one] => format!("Extracting {}", one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
            many => format!("Extracting {} archives", many.len()),
        };
        let rec = self.ops.record();
        let ctl = Arc::new(crate::transfer::Ctl::default());
        let c2 = ctl.clone();
        let here = self.files.here().dir().map(Path::to_path_buf);
        self.run_op_select(label, move || {
            let ctl = c2;
            ctl.items_total.store(archives.len() as u64, std::sync::atomic::Ordering::Relaxed);
            let total: u64 = archives.iter().filter_map(|a| archive::index(a).ok()).map(|ix| ix.items.iter().map(|i| i.size).sum::<u64>()).sum();
            ctl.bytes_total.store(total, std::sync::atomic::Ordering::Relaxed);
            let (mut made, mut errors, mut steps, mut touched) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for a in &archives {
                let Some(dir) = dest.clone().or_else(|| a.parent().map(Path::to_path_buf)) else { continue };
                let name = a.file_name().map(|n| archive::stem(&n.to_string_lossy())).unwrap_or_else(|| "Extracted".into());
                let to = actions::free_name(&dir, &name, "");
                match archive::extract_path(a, &to, &ctl) {
                    Ok(()) => {
                        steps.push(crate::transfer::Step::Created(to.clone()));
                        touched.push(to.clone());
                        made.push(to);
                    }
                    Err(e) => {
                        let _ = actions::delete(&to);
                        if e.kind() == std::io::ErrorKind::Interrupted {
                            break;
                        }
                        errors.push(format!("Couldn't extract {}: {e}", a.display()));
                    }
                }
                ctl.items_done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            *rec.lock().unwrap() = steps;
            let note = match made.as_slice() {
                [] => None,
                [one] => Some(format!("Extracted to {}", one.display())),
                many => Some(format!("Extracted {} archives", many.len())),
            };
            let r = if errors.is_empty() { Ok(note) } else { Err(errors.join(" ")) };
            // Select the new folders when they're in the folder on screen.
            let select = made.iter().filter(|m| m.parent() == here.as_deref()).cloned().collect();
            (r, touched, select)
        });
        if let Some(j) = &mut self.ops.job {
            j.ctl = Some(ctl);
        }
    }

    pub(super) fn files_action(&mut self, a: FAction, paths: Vec<PathBuf>) {
        // Items on servers: SSH terminal, their own Properties (`remoteui.rs`).
        if self.remote_action(a, &paths) {
            return;
        }
        let other_dir = self.files.other().and_then(|o| self.files.panes[o].tab().here().dir().map(Path::to_path_buf));
        // Archives are only read: changes inside them are refused, with what to do instead.
        let changes = matches!(a, FAction::Cut | FAction::Rename | FAction::Trash | FAction::Delete);
        let into = matches!(a, FAction::Paste) && paths.first().is_some_and(|p| archive::split(p).is_some());
        if (changes && paths.iter().any(|p| archive::inside(p))) || into {
            self.ops.notice = Some((READ_ONLY_NOTE.into(), true));
            return;
        }
        match a {
            FAction::FreeUp | FAction::KeepHere => {
                let act = if a == FAction::FreeUp { crate::cloudops::Action::FreeUp } else { crate::cloudops::Action::Keep };
                // Files in the listing on show: what would download is known. Anything else
                // (folders, items from the map) is asked about without a figure.
                let here = self.files.here().dir().map(Path::to_path_buf);
                let listed_files = self.files.cur().listing.as_ref().is_some_and(|l| {
                    paths.iter().all(|p| {
                        p.parent() == here.as_deref() && p.file_name().is_some_and(|n| l.entries.iter().any(|e| !e.is_dir && *e.name == *n.to_string_lossy()))
                    })
                });
                let known = listed_files.then(|| self.cloud_in_listing(paths.iter().map(|p| p.as_path())));
                let bulk = !listed_files || paths.len() > 1;
                self.cloud_run(act, paths, known, bulk);
            }
            FAction::Open if paths.is_empty() => self.files.refresh(),
            FAction::Open => {
                // Folders open here (the first one); files open in their apps.
                let mut went = false;
                for p in paths {
                    if self.listed_dir(&p) {
                        if !went {
                            self.files.go(Location::Dir(p));
                            went = true;
                        }
                    } else {
                        self.open_file(p);
                    }
                }
            }
            FAction::OpenInNewTab => {
                for p in paths {
                    self.files.new_tab(Location::Dir(p));
                }
            }
            FAction::OpenInOtherPane => {
                if self.files.other().is_none() {
                    self.files.set_split(if self.files.prefs.last_split == Split::Single { Split::Vertical } else { self.files.prefs.last_split });
                }
                if let (Some(o), Some(p)) = (self.files.other(), paths.into_iter().next()) {
                    self.files.go_in(o, Location::Dir(p));
                }
            }
            FAction::Pin => self.add_favourites(paths),
            FAction::Unpin => {
                for p in paths {
                    self.remove_favourite(&p);
                }
            }
            FAction::CopyPath => {
                self.ctx_copy = Some(paths.iter().map(|p| p.shown()).collect::<Vec<_>>().join("\n"));
                let what = if paths.len() == 1 { "the path".to_string() } else { format!("{} paths", paths.len()) };
                self.ops.notice = Some((format!("Copied {what}"), false));
            }
            FAction::Explain => {
                if let Some(p) = paths.into_iter().next() {
                    self.explain_path(p);
                }
            }
            FAction::AskAi => self.ask_about_paths(paths),
            FAction::CopyUnc => {
                // Asking Windows can wait on the network, so it's done in the background.
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(paths.iter().map(|p| crate::netdrives::unc_path(p).unwrap_or_else(|| p.clone()).shown()).collect::<Vec<_>>().join("\n"));
                });
                self.files.text_rx = Some(rx);
            }
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
            FAction::Copy => self.files_clip(paths, false),
            FAction::Cut => self.files_clip(paths, true),
            FAction::Paste => {
                // Into the folder given, or into a selected folder when that's all that's chosen.
                if let Some(dest) = paths.into_iter().next() {
                    if self.can_paste() {
                        self.paste_into(dest);
                    } else if self.browser.clip.is_none() {
                        self.ops.notice = Some(("Nothing to paste: copy or cut something first.".into(), false));
                    }
                }
            }
            FAction::Colour(c) => self.set_colour(&paths, c),
            FAction::OpenWith(pick) => self.open_with(pick, paths),
            FAction::Tags => self.open_mark_edit(paths),
            FAction::Play => {
                let clips = self.files.cur().chosen_clips();
                self.play_sequences(clips);
            }
            FAction::Rename if paths.len() > 1 => self.start_batch_rename(paths),
            FAction::Rename => {
                if let Some(p) = paths.into_iter().next().filter(|p| p.parent().is_some()) {
                    self.start_rename(p);
                }
            }
            FAction::Trash => self.files_delete(paths, false),
            FAction::Delete => self.files_delete(paths, true),
            FAction::CopyToOther | FAction::MoveToOther => match other_dir {
                Some(dest) => {
                    let from_archive = paths.iter().any(|p| archive::inside(p));
                    let op = if a == FAction::CopyToOther || from_archive { dnd::DropOp::Copy } else { dnd::DropOp::Move };
                    self.files_transfer(paths, dest, op);
                }
                None => self.ops.notice = Some(("Open a second pane first (View > Side by Side), with a folder in it.".into(), false)),
            },
            FAction::SystemMenu(extended) => match crate::shellmenu::show(self.hwnd, &paths, extended) {
                Ok(crate::shellmenu::Picked::Rename) => {
                    if let Some(p) = paths.into_iter().next().filter(|p| p.parent().is_some()) {
                        self.start_rename(p);
                    }
                }
                // The folder is being watched, but not every change made elsewhere is seen at once.
                Ok(crate::shellmenu::Picked::Done) => self.files.refresh(),
                Ok(crate::shellmenu::Picked::Nothing) => {}
                Err(e) => self.ops.notice = Some((e, true)),
            },
            // Handled in `remote_action`.
            FAction::SshTerminal | FAction::Reload => {}
            FAction::Properties => {
                if let Some(p) = paths.into_iter().next() {
                    let q = p.clone();
                    if !actions::shell_properties(&q) {
                        self.ops.props = Some(p);
                    }
                }
            }
            FAction::ExtractHere => {
                let archives: Vec<PathBuf> = paths.into_iter().filter(|p| !archive::inside(p)).collect();
                self.extract_archives(archives, None);
            }
            FAction::ExtractTo => {
                let start = paths.first().and_then(|p| archive::split(p)).and_then(|(a, _)| a.parent().map(Path::to_path_buf));
                let mut d = super::dialogs::Dialog::new().set_title("Extract to folder");
                if let Some(s) = start {
                    d = d.set_directory(s);
                }
                self.ask_path(d, super::dialogs::Pick::Folder, move |app, dest| {
                    if paths.iter().all(|p| archive::inside(p)) {
                        // Items from inside an archive: copied out, asking about names taken.
                        app.transfer(paths, dest, true);
                    } else {
                        app.extract_archives(paths.into_iter().filter(|p| !archive::inside(p)).collect(), Some(dest));
                    }
                });
            }
            FAction::OpenWithApp => {
                if let Some(p) = paths.into_iter().next() {
                    self.files.spawn(move || actions::open(&p));
                }
            }
            FAction::QuickLook => {
                if self.files.cursor_entry().is_some_and(|e| !e.is_dir) {
                    self.files.look.open = true;
                    self.files.look.from_map = false;
                } else if let Some(p) = paths.into_iter().next().filter(|p| self.listed_dir(p)) {
                    self.files.go(Location::Dir(p));
                }
            }
        }
    }
}

/// What the item menu needs to know.
pub(super) struct RowMenu {
    pub is_dir: bool,
    pub count: usize,
    /// The row is an image sequence.
    pub seq: bool,
    /// The item's colour, if it has one.
    pub colour: Option<u8>,
    pub pinned: bool,
    /// Split view is on (Copy and Move to Other Pane).
    pub two: bool,
    pub can_paste: bool,
    /// The item is an archive (Open goes inside; Extract).
    pub archive: bool,
    /// The item is inside an archive: it can be copied out but not changed.
    pub read_only: bool,
    /// The item is on a server (no Explorer, system menu or local terminal), and whether an SSH
    /// terminal can open there.
    pub server: bool,
    pub ssh: bool,
    /// In a cloud folder whose files the system can free up or keep.
    pub cloud: bool,
    /// A system file or folder Filescape can explain (What Is This?).
    pub known: bool,
}

/// Said when something would change an archive.
pub(super) const READ_ONLY_NOTE: &str = "That's inside an archive, which Filescape only reads. Copy or extract the items to a folder to change them.";

/// A file in an archive, taken out to a temporary folder (to open it in its app or preview it).
pub(super) fn archive_temp(p: &Path) -> std::io::Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("dirmap-open-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir)?;
    let to = dir.join(p.file_name().unwrap_or_default());
    archive::extract_path(p, &to, &crate::transfer::Ctl::default())?;
    Ok(to)
}

/// The menu for an item in the list.
/// What a row or an icon needs to know to handle clicks, drags and its menu.
pub(super) struct ItemCtx<'a> {
    pub(super) t: &'a FTab,
    pub(super) l: &'a Listing,
    pub(super) dir: &'a Path,
    pub(super) sizes: Option<&'a FolderSizes>,
    pub(super) two: bool,
    pub(super) can_paste: bool,
    pub(super) marks: &'a crate::fsys::marks::Marks,
    pub(super) apps: &'a [openwith::UserApp],
}

/// Clicks, double-clicks, middle-clicks, dragging and the right-click menu for one item (a row
/// in Details, an icon in Icons). Folders are added to `drops` to take dropped items.
pub(super) fn item_actions(ui: &egui::Ui, resp: egui::Response, i: u32, c: &ItemCtx, pinned: bool, picks: &mut FPicks, drops: &mut Vec<(egui::Response, PathBuf)>) {
    let (t, l, dir, sizes) = (c.t, c.l, c.dir, c.sizes);
    let e = &l.entries[i as usize];
    let order = &t.order;
    let selected = t.selected.contains(&e.name);
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
    // Drag what's selected (or just this item, if it isn't).
    if resp.drag_started() {
        let rows: Vec<u32> = if selected { order.iter().copied().filter(|&k| t.selected.contains(&l.entries[k as usize].name)).collect() } else { vec![i] };
        let (items, dirs): (Vec<PathBuf>, Vec<bool>) = rows.iter().flat_map(|&k| t.row_paths(dir, l, k).into_iter().map(move |p| (p, l.entries[k as usize].is_dir))).unzip();
        egui::DragAndDrop::set_payload(ui.ctx(), dnd::DragItems(items, dirs));
    }
    if is_dir {
        drops.push((resp.clone(), path.clone()));
    }
    let read_only = archive::inside(&path);
    let seq = t.seqs.contains_key(&i);
    let colour = c.marks.get(&path).and_then(|m| m.colour);
    let caps = crate::fsys::remote::is_remote(dir).then(|| crate::fsys::remote::caps(dir));
    let server = caps.is_some();
    let ctx_menu = RowMenu { is_dir, count, seq, colour, pinned, two: c.two, can_paste: c.can_paste && !read_only, archive: !is_dir && archive::is_archive(&e.name) && !read_only, read_only, server, ssh: caps.is_some_and(|c| c.ssh), cloud: false, known: !server && !read_only && crate::known::about(&path).is_some() };
    let ctx_menu = RowMenu {
        cloud: !server && !read_only && crate::cloudops::supported() && (e.flags & scan::store::ON_DEMAND != 0 || (e.is_dir && super::cloud::on_demand_dir(&path))),
        ..ctx_menu
    };
    // Shift+right-click: straight to Windows' own menu, with its Shift extras, as in Explorer.
    if cfg!(windows) && !read_only && !server && resp.secondary_clicked() && ui.input(|i| i.modifiers.shift) {
        let paths = if selected { t.chosen_paths() } else { t.row_paths(dir, l, i) };
        picks.menu = Some((FAction::SystemMenu(true), paths));
        return;
    }
    resp.context_menu(|ui| {
        if let Some(a) = item_menu(ui, &ctx_menu, c.apps, e.ext()) {
            let paths = if selected { t.chosen_paths() } else { t.row_paths(dir, l, i) };
            picks.menu = Some((a, paths));
        }
    });
}

fn item_menu(ui: &mut egui::Ui, m: &RowMenu, apps: &[openwith::UserApp], ext: &str) -> Option<FAction> {
    use egui::{Key, Modifiers as M};
    let ctx = ui.ctx().clone();
    let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
    let mut out = None;
    let mut item = |ui: &mut egui::Ui, enabled: bool, label: &str, keys: String, a: FAction| {
        if ui.add_enabled(enabled, egui::Button::new(style::mi(label)).shortcut_text(keys)).clicked() {
            out = Some(a);
        }
    };
    let many = m.count > 1;
    let open = if many { format!("Open {} Items", m.count) } else { "Open".to_string() };
    if m.seq {
        item(ui, true, if m.count == 2 { "Play and Compare (A/B)" } else { "Play" }, k(M::NONE, Key::Enter), FAction::Play);
    } else {
        item(ui, true, &open, k(M::NONE, Key::Enter), FAction::Open);
    }
    if m.is_dir {
        item(ui, true, if many { "Open in New Tabs" } else { "Open in New Tab" }, String::new(), FAction::OpenInNewTab);
        if !many {
            item(ui, true, "Open in Other Pane", String::new(), FAction::OpenInOtherPane);
        }
    } else if !many {
        item(ui, true, "Quick Look", k(M::NONE, Key::Space), FAction::QuickLook);
    }
    let with = if m.seq {
        None
    } else {
        openwith::menu(ui, apps, ext, m.is_dir)
    };
    if m.archive {
        if !many {
            item(ui, true, "Open with Its App", String::new(), FAction::OpenWithApp);
        }
        ui.separator();
        item(ui, true, "Extract Here", String::new(), FAction::ExtractHere);
        item(ui, true, "Extract To…", String::new(), FAction::ExtractTo);
    } else if m.read_only {
        ui.separator();
        item(ui, true, "Extract To…", String::new(), FAction::ExtractTo);
    }
    let rw = !m.read_only;
    ui.separator();
    item(ui, rw, "Cut", k(M::COMMAND, Key::X), FAction::Cut);
    item(ui, true, "Copy", k(M::COMMAND, Key::C), FAction::Copy);
    if m.is_dir && !many {
        item(ui, m.can_paste && rw, "Paste Into Folder", String::new(), FAction::Paste);
    }
    if m.two {
        item(ui, true, "Copy to Other Pane", k(M::SHIFT, Key::F5), FAction::CopyToOther);
        item(ui, rw, "Move to Other Pane", k(M::SHIFT, Key::F6), FAction::MoveToOther);
    }
    ui.separator();
    let rename = if m.seq && !many {
        "Rename Frames…".to_string()
    } else if many {
        format!("Rename {} Items…", m.count)
    } else {
        "Rename…".to_string()
    };
    item(ui, rw, &rename, k(M::NONE, Key::F2), FAction::Rename);
    let bin = if cfg!(windows) { "Delete (to Recycle Bin)…" } else { "Move to Trash…" };
    let bin_key = if cfg!(target_os = "macos") { k(M::COMMAND, Key::Backspace) } else { k(M::NONE, Key::Delete) };
    // A server has no bin: only Delete Permanently.
    if !m.server {
        item(ui, rw, bin, bin_key, FAction::Trash);
    }
    item(ui, rw, "Delete Permanently…", k(M::SHIFT, Key::Delete), FAction::Delete);
    let mut marked = None;
    if !m.read_only {
        ui.separator();
        marked = marksui::menu(ui, m.colour);
    }
    ui.separator();
    item(ui, true, if many { "Copy Paths" } else { "Copy Path" }, k(M::COMMAND | M::SHIFT, Key::C), FAction::CopyPath);
    if cfg!(windows) && !m.server {
        item(ui, true, "Copy Network (UNC) Path", String::new(), FAction::CopyUnc);
    }
    item(ui, true, if many { "Copy Names" } else { "Copy Name" }, String::new(), FAction::CopyName);
    if !many && m.server {
        // On a server: an SSH terminal instead of Explorer and the local terminals.
        if m.ssh {
            item(ui, true, "Open SSH Terminal Here…", k(M::CTRL | M::ALT, Key::T), FAction::SshTerminal);
        }
        item(ui, true, "Properties", k(M::ALT, Key::Enter), FAction::Properties);
    } else if !many {
        let reveal = if cfg!(windows) {
            "Show in Explorer"
        } else if cfg!(target_os = "macos") {
            "Show in Finder"
        } else {
            "Show in System File Manager"
        };
        item(ui, true, reveal, String::new(), FAction::Reveal);
        for (label, which) in actions::terminals() {
            item(ui, true, label, String::new(), FAction::Terminal(*which));
        }
        item(ui, true, "Properties", k(M::ALT, Key::Enter), FAction::Properties);
    }
    if m.known && !many {
        item(ui, true, "What Is This?", String::new(), FAction::Explain);
    }
    if !m.server {
        item(ui, true, if many { "Ask AI About These…" } else { "Ask AI About This…" }, String::new(), FAction::AskAi);
    }
    if m.cloud {
        ui.separator();
        item(ui, true, crate::cloudops::Action::FreeUp.label(), String::new(), FAction::FreeUp);
        item(ui, true, crate::cloudops::Action::Keep.label(), String::new(), FAction::KeepHere);
    }
    if cfg!(windows) && !m.read_only && !m.server {
        ui.separator();
        // Explorer's full menu, with what other apps add (7-Zip, Notepad++, Send to…).
        item(ui, true, "Show More Options", k(M::SHIFT, Key::F10), FAction::SystemMenu(false));
    }
    if m.is_dir && !many {
        ui.separator();
        if let Some(a) = folder_menu(ui, m.pinned) {
            out = Some(a);
        }
    }
    out.or(marked).or(with.map(FAction::OpenWith))
}

/// Add to Favourites (or remove) and Analyse, for a folder.
fn folder_menu(ui: &mut egui::Ui, pinned: bool) -> Option<FAction> {
    let mut out = None;
    if pinned {
        if ui.add(egui::Button::new(style::mi("Remove from Favourites"))).clicked() {
            out = Some(FAction::Unpin);
        }
    } else if ui.add(egui::Button::new(style::mi("Add to Favourites"))).on_hover_text("A bookmark in the sidebar. You can also drag the folder there.").clicked() {
        out = Some(FAction::Pin);
    }
    if ui.add(egui::Button::new(style::mi("Analyse This Folder"))).on_hover_text("Scan it and show its treemap in the analysis dock, while you keep browsing").clicked() {
        out = Some(FAction::Analyse);
    }
    out
}

/// The menu for a place in the sidebar.
fn place_menu(ui: &mut egui::Ui, path: &Path, pinned: bool, two: bool, picks: &mut FPicks) {
    if ui.add(egui::Button::new(style::mi("Open in New Tab"))).clicked() {
        picks.new_tab = Some(Location::Dir(path.to_path_buf()));
    }
    if two && ui.add(egui::Button::new(style::mi("Open in Other Pane"))).clicked() {
        picks.menu = Some((FAction::OpenInOtherPane, vec![path.to_path_buf()]));
    }
    if pinned {
        if ui.add(egui::Button::new(style::mi("Remove from Favourites"))).clicked() {
            picks.menu = Some((FAction::Unpin, vec![path.to_path_buf()]));
        }
    } else if ui.add(egui::Button::new(style::mi("Add to Favourites"))).clicked() {
        picks.menu = Some((FAction::Pin, vec![path.to_path_buf()]));
    }
    if ui.add(egui::Button::new(style::mi("Copy Path"))).clicked() {
        picks.menu = Some((FAction::CopyPath, vec![path.to_path_buf()]));
    }
    if ui.add(egui::Button::new(style::mi("Analyse This Folder"))).clicked() {
        picks.menu = Some((FAction::Analyse, vec![path.to_path_buf()]));
    }
}

/// A thin capacity bar under a drive's name, red when nearly full.
pub(super) fn capacity_bar(ui: &egui::Ui, r: Rect, frac: Option<f32>) {
    let Some(frac) = frac else { return };
    let bar = Rect::from_min_size(pos2(r.left() + 28.0, r.bottom() - 7.0), vec2((r.width() - 38.0).max(10.0), 3.0));
    let p = ui.painter();
    p.rect_filled(bar, 1.5, ui.visuals().extreme_bg_color);
    let mut fill = bar;
    fill.set_width(bar.width() * frac.clamp(0.0, 1.0));
    let c = if frac > 0.9 { Color32::from_rgb(210, 70, 60) } else { Color32::from_rgb(70, 130, 210) };
    p.rect_filled(fill, 1.5, c);
}

/// A sidebar line: icon and label, left-aligned. Where we are gets the selection tint and an
/// accent bar at its left edge.
pub(super) fn side_row(ui: &mut egui::Ui, icon: &str, label: &str, on: bool, h: f32) -> egui::Response {
    side_row_sense(ui, icon, label, on, h, Sense::click())
}

/// `side_row` that can also be dragged.
/// Where the sidebar's rows are listed each frame, for keyboard focus.
const SIDE_ROWS: &str = "fm-side-rows";
pub(super) fn side_row_sense(ui: &mut egui::Ui, icon: &str, label: &str, on: bool, h: f32, sense: Sense) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), sense);
    // Remembered for F6, which moves the keyboard to the sidebar and back.
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<egui::Id>>(egui::Id::new(SIDE_ROWS))
            .push(resp.id)
    });
    if resp.gained_focus() {
        resp.scroll_to_me(None);
    }
    let pal = style::pal(ui);
    let p = ui.painter();
    if resp.has_focus() {
        p.rect_stroke(
            r.shrink(1.0),
            style::RADIUS as f32,
            Stroke::new(1.5, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
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
pub(super) fn side_section(ui: &mut egui::Ui, salt: &str, title: &str, default_open: bool, add: impl FnOnce(&mut egui::Ui)) {
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
    let r = root.shown();
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
        Location::Dir(p) if remote::is_root(p) => remote::split(p).map(|(s, _)| s.short()).unwrap_or_default(),
        Location::Dir(p) => p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.shown()),
    }
}

/// The breadcrumbs for a place: (label, where it goes).
pub(super) fn crumbs_of(l: &Location) -> Vec<(String, Location)> {
    let mut out = vec![(format!("🖳 {}", places::computer_name()), Location::Computer)];
    let Location::Dir(p) = l else { return out };
    if let Some((site, rest)) = remote::split(p) {
        // A server: its address, then the folders on it.
        out.push((format!("🌐 {}", site.short()), Location::Dir(remote::path_of(&site, "/"))));
        let mut acc = String::new();
        for n in rest.split('/').filter(|n| !n.is_empty()) {
            acc.push('/');
            acc.push_str(n);
            out.push((n.to_string(), Location::Dir(remote::path_of(&site, &acc))));
        }
        return out;
    }
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
                let label = if acc.as_os_str().len() <= 1 { "/".to_string() } else { acc.shown().trim_end_matches('\\').to_string() };
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
        Entry { name: name.into(), is_dir, is_link: false, hidden, size, alloc: size, flags: 0, modified }
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
