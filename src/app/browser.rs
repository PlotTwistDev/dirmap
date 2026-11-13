//! The file manager at the top of the window: a Finder / Explorer-style browser over the scan,
//! with tabs, List, Icons and Columns layouts (and the folder tree), multi-selection and the
//! usual file commands. It follows the map: opening a folder here zooms the map to it, zooming
//! the map moves it here, and the selection is shared.

use super::*;
use crate::actions;
use menus::{ItemInfo, TreeAction};
use std::cmp::Ordering as Cmp;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum Layout {
    /// The folder tree (the list dirmap always had).
    Tree,
    #[default]
    List,
    Icons,
    Columns,
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum SortKey {
    Name,
    #[default]
    Size,
    Kind,
    Items,
    Modified,
}

impl SortKey {
    const ALL: [SortKey; 5] = [SortKey::Name, SortKey::Size, SortKey::Kind, SortKey::Items, SortKey::Modified];

    fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Size => "Size",
            SortKey::Kind => "Kind",
            SortKey::Items => "Items",
            SortKey::Modified => "Last change",
        }
    }

    /// Names and kinds sort A→Z; the rest largest, most or newest first.
    fn natural_desc(self) -> bool {
        !matches!(self, SortKey::Name | SortKey::Kind)
    }
}

/// A column of the List layout.
#[derive(Clone, Copy, PartialEq)]
enum Col {
    Name,
    Size,
    Share,
    Kind,
    Items,
    Modified,
}

impl Col {
    fn sort(self) -> Option<SortKey> {
        match self {
            Col::Name => Some(SortKey::Name),
            Col::Size => Some(SortKey::Size),
            Col::Share => None,
            Col::Kind => Some(SortKey::Kind),
            Col::Items => Some(SortKey::Items),
            Col::Modified => Some(SortKey::Modified),
        }
    }

    fn title(self) -> &'static str {
        match self {
            Col::Name => "Name",
            Col::Size => "Size",
            Col::Share => "% of folder",
            Col::Kind => "Kind",
            Col::Items => "Items",
            Col::Modified => "Last change",
        }
    }
}

/// One tab: a folder with its own layout and history. Folders are kept as paths on disk, so
/// they survive refreshes and file operations that renumber the tree.
pub(super) struct Tab {
    pub layout: Layout,
    folder: PathBuf,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
}

/// Items copied or cut, waiting to be pasted.
pub(super) struct Clip {
    paths: Vec<PathBuf>,
    cut: bool,
}

/// What's being dragged.
pub(super) struct DragItems(Vec<PathBuf>);

pub(super) struct Browser {
    tabs: Vec<Tab>,
    active: usize,
    /// The current folder, as a path in the tree on screen.
    folder: Vec<u32>,
    sort: SortKey,
    /// Opposite of the sort's natural direction.
    reversed: bool,
    /// Other items selected along with the selection, in `folder` (child indices). Empty when
    /// just one item is selected.
    picked: BTreeSet<u32>,
    /// Items to select once they're in the tree (just pasted, moved or made).
    pick_later: Vec<PathBuf>,
    /// Where Shift-click selection ranges start.
    anchor: Option<u32>,
    /// Keys go to the file manager (it was clicked last) rather than the map.
    pub focused: bool,
    /// Where the map was drawn last frame, to tell which one a click went to.
    pub map_rect: Option<Rect>,
    /// Where the file manager was drawn, so the map leaves the mouse's side buttons to it.
    pub rect: Option<Rect>,
    clip: Option<Clip>,
    /// The map's folder and the selection as of last frame, to notice changes made elsewhere.
    seen_zoom: Option<Vec<u32>>,
    seen_sel: Option<Vec<u32>>,
    /// The tree that `folder` and the sort orders belong to.
    tree_id: usize,
    orders: HashMap<Vec<u32>, Arc<Vec<u32>>>,
    order_key: (usize, SortKey, bool),
    /// Bring the selection into view next frame.
    scroll_to_sel: bool,
    /// Last scroll offset and height of the main list or grid, for keeping the selection in view.
    view: (f32, f32),
    /// Icons per row in the grid last frame.
    grid_cols: usize,
    /// Type-to-select: letters typed so far and when the last one came.
    typed: String,
    typed_at: f64,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            tabs: vec![Tab { layout: Layout::List, folder: PathBuf::new(), back: Vec::new(), fwd: Vec::new() }],
            active: 0,
            folder: Vec::new(),
            sort: SortKey::Size,
            reversed: false,
            picked: BTreeSet::new(),
            pick_later: Vec::new(),
            anchor: None,
            focused: true,
            map_rect: None,
            rect: None,
            clip: None,
            seen_zoom: None,
            seen_sel: None,
            tree_id: 0,
            orders: HashMap::new(),
            order_key: (0, SortKey::Size, false),
            scroll_to_sel: false,
            view: (0.0, 0.0),
            grid_cols: 1,
            typed: String::new(),
            typed_at: 0.0,
        }
    }
}

impl Browser {
    fn tab(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn layout(&self) -> Layout {
        self.tabs[self.active].layout
    }
}

const ROW: f32 = 22.0;
const CELL_W: f32 = 108.0;
const CELL_H: f32 = 104.0;
const COL_W: f32 = 250.0;

/// What a folder's entry is: "Folder", "PNG image", "File".
pub(super) fn kind_of(n: &Node) -> String {
    if n.is_dir {
        return "Folder".into();
    }
    let ext = n.ext();
    if ext.is_empty() {
        return "File".into();
    }
    let noun = match categories::of(&n.name) {
        Category::Audio => "audio",
        Category::Video => "video",
        Category::Images => "image",
        Category::Models => "AI model",
        Category::Archives => "archive",
        Category::Documents => "document",
        Category::Code => "source",
        Category::Programs => "program",
        Category::DiskImages => "disk image",
        Category::Other => "file",
    };
    format!("{} {noun}", ext.to_uppercase())
}

/// Case-insensitive, with runs of digits compared as numbers: "shot2" before "shot10".
pub(super) fn natural_cmp(a: &str, b: &str) -> Cmp {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Cmp::Equal,
            (None, _) => return Cmp::Less,
            (_, None) => return Cmp::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = a.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    a.next();
                }
                let mut nb = String::new();
                while let Some(c) = b.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    b.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
                if o != Cmp::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Cmp::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// The order to list a folder's children in.
pub(super) fn sorted(n: &Node, key: SortKey, reversed: bool) -> Vec<u32> {
    let c = &n.children;
    let mut o: Vec<u32> = (0..c.len() as u32).collect();
    let kinds: Vec<String> = if key == SortKey::Kind { c.iter().map(kind_of).collect() } else { Vec::new() };
    let folders_first = matches!(key, SortKey::Name | SortKey::Kind);
    o.sort_by(|&x, &y| {
        let (a, b) = (&c[x as usize], &c[y as usize]);
        let mut ord = match key {
            SortKey::Name => Cmp::Equal,
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Kind => kinds[x as usize].cmp(&kinds[y as usize]),
            SortKey::Items => (a.is_dir, a.files).cmp(&(b.is_dir, b.files)),
            SortKey::Modified => a.modified.cmp(&b.modified),
        };
        if key.natural_desc() {
            ord = ord.reverse();
        }
        let mut ord = ord.then_with(|| natural_cmp(&a.name, &b.name));
        if reversed {
            ord = ord.reverse();
        }
        if folders_first { b.is_dir.cmp(&a.is_dir).then(ord) } else { ord }
    });
    o
}

/// What was clicked or picked while drawing, acted on afterwards.
#[derive(Default)]
struct Picks {
    /// Clicked, with the modifier keys held (Ctrl/⌘ adds to the selection, Shift extends it).
    select: Option<(Vec<u32>, egui::Modifiers)>,
    /// Selected from a column: the folder that column shows becomes the current one.
    column_select: Option<(Vec<u32>, Vec<u32>)>,
    open: Option<Vec<u32>>,
    go: Option<Vec<u32>>,
    menu: Option<(TreeAction, Vec<u32>)>,
    sort: Option<SortKey>,
    hover: Option<Vec<u32>>,
    /// Dropped: these items into this folder; copy rather than move.
    drop: Option<(Vec<PathBuf>, PathBuf, bool)>,
    new_tab: Option<PathBuf>,
    switch_tab: Option<usize>,
    close_tab: Option<usize>,
    new_folder: bool,
    paste: bool,
}

impl App {
    fn browser_order(&mut self, tree: &Arc<Node>, folder: &[u32]) -> Arc<Vec<u32>> {
        let b = &mut self.browser;
        let key = (Arc::as_ptr(tree) as usize, b.sort, b.reversed);
        if key != b.order_key {
            b.orders.clear();
            b.order_key = key;
        }
        if let Some(o) = b.orders.get(folder) {
            return o.clone();
        }
        let o = Arc::new(tree.at(folder).map(|n| sorted(n, b.sort, b.reversed)).unwrap_or_default());
        b.orders.insert(folder.to_vec(), o.clone());
        o
    }

    /// The folder at `disk` in the tree on screen, or the nearest folder above it that's there.
    fn resolve_folder(&self, disk: &Path) -> Vec<u32> {
        let Some(tree) = self.shown_tree() else { return Vec::new() };
        disk.ancestors()
            .find_map(|a| self.tree_path_of(a).filter(|p| tree.at(p).is_some_and(|n| n.is_dir)))
            .unwrap_or_default()
    }

    /// Show a folder without touching history or the map.
    fn set_folder(&mut self, folder: Vec<u32>) {
        if folder != self.browser.folder {
            self.browser.picked.clear();
            self.browser.anchor = None;
            self.browser.typed.clear();
        }
        let disk = self.full_path(&folder);
        self.browser.tab().folder = disk;
        self.browser.folder = folder;
        self.browser.scroll_to_sel = true;
    }

    /// Go to a folder, remembering where we were for Back, and take the map there too.
    fn browse_to(&mut self, folder: Vec<u32>) {
        if folder != self.browser.folder {
            let from = self.full_path(&self.browser.folder);
            let t = self.browser.tab();
            t.back.push(from);
            t.fwd.clear();
        }
        self.set_folder(folder.clone());
        self.zoom = folder;
        self.browser.seen_zoom = Some(self.zoom.clone());
    }

    /// Open a folder in the file manager (or a file with its app).
    pub(super) fn browse_into(&mut self, path: Vec<u32>) {
        let Some(tree) = self.shown_tree() else { return };
        let Some(n) = tree.at(&path) else { return };
        if !n.is_dir {
            self.tree_action(TreeAction::Open, path);
            return;
        }
        self.browse_to(path);
    }

    /// Up to the folder above, selecting the one we were in.
    fn browse_up(&mut self) {
        let from = self.browser.folder.clone();
        if from.is_empty() {
            return;
        }
        let mut up = from.clone();
        up.pop();
        self.browse_to(up);
        self.browser_select(from);
    }

    fn browser_back(&mut self) {
        let Some(p) = self.browser.tab().back.pop() else { return };
        let here = self.full_path(&self.browser.folder);
        self.browser.tab().fwd.push(here);
        let f = self.resolve_folder(&p);
        self.set_folder(f.clone());
        self.zoom = f;
        self.browser.seen_zoom = Some(self.zoom.clone());
    }

    fn browser_forward(&mut self) {
        let Some(p) = self.browser.tab().fwd.pop() else { return };
        let here = self.full_path(&self.browser.folder);
        self.browser.tab().back.push(here);
        let f = self.resolve_folder(&p);
        self.set_folder(f.clone());
        self.zoom = f;
        self.browser.seen_zoom = Some(self.zoom.clone());
    }

    /// Select one item from the file manager: shared with the folder tree and the maps.
    fn browser_select(&mut self, path: Vec<u32>) {
        self.browser.picked.clear();
        self.browser.anchor = path.last().copied();
        self.select(path.clone());
        self.browser.seen_sel = Some(path);
        self.browser.scroll_to_sel = true;
    }

    /// A click on an item, with Ctrl/⌘ (add or remove) or Shift (a range) as in a file manager.
    fn click_select(&mut self, path: Vec<u32>, m: egui::Modifiers, order: &[u32]) {
        let Some((&i, folder)) = path.split_last() else { return self.browser_select(path) };
        if folder != self.browser.folder.as_slice() {
            return self.browser_select(path);
        }
        if m.command {
            let b = &mut self.browser;
            if b.picked.is_empty() {
                if let Some(s) = self.selected.as_ref().filter(|s| s.len() == path.len() && s.starts_with(folder)) {
                    b.picked.insert(*s.last().unwrap());
                }
            }
            if !b.picked.remove(&i) {
                b.picked.insert(i);
            }
            b.anchor = Some(i);
            let primary = if b.picked.contains(&i) { Some(i) } else { b.picked.iter().next_back().copied() };
            let picked = std::mem::take(&mut self.browser.picked);
            if let Some(p) = primary {
                let mut sel = folder.to_vec();
                sel.push(p);
                self.select(sel.clone());
                self.browser.seen_sel = Some(sel);
            }
            // One left is an ordinary single selection.
            self.browser.picked = if picked.len() > 1 { picked } else { BTreeSet::new() };
        } else if let (true, Some(a)) = (m.shift, self.browser.anchor) {
            let pa = order.iter().position(|&o| o == a);
            let pi = order.iter().position(|&o| o == i);
            if let (Some(pa), Some(pi)) = (pa, pi) {
                let (lo, hi) = (pa.min(pi), pa.max(pi));
                let range: BTreeSet<u32> = order[lo..=hi].iter().copied().collect();
                self.select(path.clone());
                self.browser.seen_sel = Some(path);
                self.browser.picked = if range.len() > 1 { range } else { BTreeSet::new() };
            } else {
                self.browser_select(path);
            }
        } else {
            self.browser_select(path);
        }
    }

    /// Is this item of the current folder selected?
    fn is_chosen(&self, folder: &[u32], i: u32) -> bool {
        if folder != self.browser.folder.as_slice() {
            return self.selected_in(folder) == Some(i);
        }
        if self.browser.picked.is_empty() { self.selected_in(folder) == Some(i) } else { self.browser.picked.contains(&i) }
    }

    /// Everything selected: the picked items of the current folder, or just the selection.
    pub(super) fn chosen(&self) -> Vec<Vec<u32>> {
        if !self.browser.picked.is_empty() && self.browser.layout() != Layout::Tree {
            return self
                .browser
                .picked
                .iter()
                .map(|&i| {
                    let mut p = self.browser.folder.clone();
                    p.push(i);
                    p
                })
                .collect();
        }
        self.selected.iter().cloned().collect()
    }

    /// The selection, if it's an item of `folder`.
    fn selected_in(&self, folder: &[u32]) -> Option<u32> {
        let s = self.selected.as_ref()?;
        (s.len() == folder.len() + 1 && s.starts_with(folder)).then(|| *s.last().unwrap())
    }

    /// The file manager takes the keyboard: it was clicked last and isn't showing the tree.
    pub(super) fn browser_has_keys(&self) -> bool {
        self.browser.focused && self.browser.layout() != Layout::Tree
    }

    /// Keep the folder in step with changes made elsewhere: the map zooming somewhere,
    /// something selected on the map or in a list, a refresh, a new scan or filter.
    fn browser_sync(&mut self, tree: &Arc<Node>) {
        let id = Arc::as_ptr(tree) as usize;
        if self.browser.tree_id != id {
            // Positions in a new tree mean nothing in the old one: find the folder again by path.
            self.browser.tree_id = id;
            self.browser.picked.clear();
            let disk = self.browser.tab().folder.clone();
            self.browser.folder = if disk.as_os_str().is_empty() { self.zoom.clone() } else { self.resolve_folder(&disk) };
        }
        while !self.browser.folder.is_empty() && !tree.at(&self.browser.folder).is_some_and(|n| n.is_dir) {
            self.browser.folder.pop();
        }
        // The map moved to another folder: follow it.
        if self.browser.seen_zoom.as_ref() != Some(&self.zoom) {
            if self.browser.seen_zoom.is_some() && tree.at(&self.zoom).is_some_and(|n| n.is_dir) {
                self.set_folder(self.zoom.clone());
            }
            self.browser.seen_zoom = Some(self.zoom.clone());
        }
        // Something selected elsewhere: show it among the items around it.
        if self.selected != self.browser.seen_sel {
            if let Some(sel) = self.selected.clone().filter(|s| tree.at(s).is_some()) {
                if let Some((&i, parent)) = sel.split_last() {
                    if parent != self.browser.folder.as_slice() && sel != self.browser.folder {
                        let from = self.full_path(&self.browser.folder);
                        let t = self.browser.tab();
                        t.back.push(from);
                        t.fwd.clear();
                        self.set_folder(parent.to_vec());
                    }
                    self.browser.picked.clear();
                    self.browser.anchor = Some(i);
                }
                self.browser.scroll_to_sel = true;
            }
            self.browser.seen_sel = self.selected.clone();
        }
        // Items just pasted or moved here: select them all.
        let later = std::mem::take(&mut self.browser.pick_later);
        if later.len() > 1 {
            let here: BTreeSet<u32> = later
                .iter()
                .filter_map(|p| self.tree_path_of(p))
                .filter_map(|tp| tp.split_last().filter(|(_, parent)| *parent == self.browser.folder.as_slice()).map(|(&i, _)| i))
                .collect();
            if here.len() > 1 {
                self.browser.picked = here;
            }
        }
        let disk = self.full_path(&self.browser.folder);
        self.browser.tab().folder = disk;
    }

    /// Select these items (by path on disk) once the tree has them.
    pub(super) fn browser_pick_later(&mut self, paths: Vec<PathBuf>) {
        self.browser.pick_later = paths;
    }

    /// Paths of the items picked along with the selection, for the maps to outline too.
    pub(super) fn picked_paths(&self) -> Vec<Vec<u32>> {
        if self.browser.picked.len() < 2 || self.browser.layout() == Layout::Tree {
            return Vec::new();
        }
        self.chosen()
    }

    /// Faint (Highlight mode): a file that doesn't match, or a folder with no matches inside.
    fn browser_dim(&self, n: &Node, path: &[u32], hits: &Option<Arc<crate::subset::Hits>>) -> bool {
        if self.shown.is_some() || !self.filter.active() {
            return false;
        }
        if n.is_dir { hits.as_ref().is_some_and(|h| h.at(path).is_none()) } else { !self.filter.matches(&n.name, n.size) }
    }

    fn icon_tint(&self, n: &Node, path: &[u32]) -> Color32 {
        self.node_color(n, &self.full_of(path))
    }

    /// Selection colours: strong when the file manager has the keyboard, quieter when the map has.
    fn sel_colors(&self, vis: &egui::Visuals) -> (Color32, Color32) {
        if self.browser.focused {
            (vis.selection.bg_fill, vis.selection.stroke.color)
        } else {
            (vis.widgets.active.weak_bg_fill, vis.strong_text_color())
        }
    }

    // ---------- commands ----------

    /// A command from the right-click menu or the keyboard, applied to everything selected
    /// when `path` is one of several selected items.
    pub(super) fn browser_action(&mut self, a: TreeAction, path: Vec<u32>) {
        let chosen = self.chosen();
        let multi = matches!(
            a,
            TreeAction::Delete
                | TreeAction::Recycle
                | TreeAction::MoveTo
                | TreeAction::CopyPath
                | TreeAction::Open
                | TreeAction::Exclude
                | TreeAction::CopyItems
                | TreeAction::CutItems
        );
        if multi && chosen.len() > 1 && chosen.contains(&path) {
            self.items_action(a, chosen);
        } else {
            self.tree_action(a, path);
        }
    }

    /// Commands that work on several items, and the file manager's own commands.
    pub(super) fn items_action(&mut self, a: TreeAction, paths: Vec<Vec<u32>>) {
        let Some(tree) = self.shown_tree() else { return };
        let nodes: Vec<&Node> = paths.iter().filter_map(|p| tree.at(p)).collect();
        if nodes.len() != paths.len() || paths.is_empty() {
            return;
        }
        let disks: Vec<PathBuf> = paths.iter().map(|p| self.full_path(p)).collect();
        let count = |n: usize| if n == 1 { "1 item".to_string() } else { format!("{n} items") };
        match a {
            TreeAction::Rename => self.start_rename(disks[0].clone()),
            TreeAction::CopyItems | TreeAction::CutItems => {
                let cut = a == TreeAction::CutItems;
                if cut && paths.iter().any(|p| p.is_empty()) {
                    return;
                }
                // The paths go on the system clipboard as text too, for pasting into a terminal
                // or another app (and so Ctrl+V reaches dirmap as a paste).
                self.ctx_copy = Some(text_of(&disks));
                let what = if disks.len() == 1 { nodes[0].name.to_string() } else { count(disks.len()) };
                let keys = if cfg!(target_os = "macos") { "⌘V" } else { "Ctrl+V" };
                self.ops.notice = Some((format!("{} {what}. Open a folder and press {keys} to paste.", if cut { "Cut" } else { "Copied" }), false));
                self.browser.clip = Some(Clip { paths: disks, cut });
            }
            TreeAction::Paste => {
                let dest = if nodes.len() == 1 && nodes[0].is_dir { disks[0].clone() } else { self.full_path(&self.browser.folder) };
                self.paste_into(dest);
            }
            TreeAction::NewFolder => self.new_folder(),
            TreeAction::NewTab => {
                if nodes[0].is_dir {
                    self.new_tab(disks[0].clone());
                }
            }
            TreeAction::Delete | TreeAction::Recycle => {
                if paths.iter().any(|p| p.is_empty()) {
                    return;
                }
                self.ops.confirm = Some(menus::Confirm {
                    action: a,
                    paths: disks,
                    is_dir: nodes[0].is_dir,
                    size: nodes.iter().map(|n| n.size).sum(),
                    files: nodes.iter().map(|n| n.files).sum(),
                });
            }
            TreeAction::MoveTo => {
                if paths.iter().any(|p| p.is_empty()) {
                    return;
                }
                let start = disks[0].parent().map(Path::to_path_buf).unwrap_or_default();
                if let Some(dest) = rfd::FileDialog::new().set_title("Move to folder").set_directory(start).pick_folder() {
                    self.transfer(disks, dest, false);
                }
            }
            TreeAction::CopyPath => {
                self.ctx_copy = Some(text_of(&disks));
                self.ops.notice = Some((format!("Copied the paths of {}", count(disks.len())), false));
            }
            TreeAction::Open => {
                for d in &disks {
                    if let Err(e) = actions::open(d) {
                        self.ops.notice = Some((e, true));
                    }
                }
            }
            _ => {
                for p in paths {
                    self.tree_action(a, p);
                }
            }
        }
    }

    pub(super) fn start_rename(&mut self, path: PathBuf) {
        let text = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.ops.rename = Some(menus::RenameBox { path, text, fresh: true, error: None });
    }

    /// A new folder in the current folder, then the Rename box for it.
    fn new_folder(&mut self) {
        if self.ops.job.is_some() {
            return;
        }
        let dir = self.full_path(&self.browser.folder);
        self.ops.then_rename = true;
        self.run_op_select("Making a new folder".into(), move || match actions::new_folder(&dir) {
            Ok(p) => (Ok(None), vec![p.clone()], vec![p]),
            Err(e) => (Err(e), vec![], vec![]),
        });
    }

    /// Paste what was copied or cut into `dest`.
    fn paste_into(&mut self, dest: PathBuf) {
        let Some(clip) = &self.browser.clip else { return };
        let (paths, cut) = (clip.paths.clone(), clip.cut);
        if cut {
            // Cut items move once; after that there's nothing left to paste.
            self.browser.clip = None;
        }
        self.transfer(paths, dest, !cut);
    }

    /// Copy or move items into a folder, in the background, then select the last of them.
    fn transfer(&mut self, sources: Vec<PathBuf>, dest: PathBuf, copy: bool) {
        if self.ops.job.is_some() {
            self.ops.notice = Some(("Another file operation is still running.".into(), true));
            return;
        }
        // Moving something to where it already is does nothing.
        let sources: Vec<PathBuf> = sources.into_iter().filter(|s| copy || s.parent() != Some(dest.as_path())).collect();
        if sources.is_empty() {
            return;
        }
        let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| dest.display().to_string());
        let what = if sources.len() == 1 {
            sources[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            format!("{} items", sources.len())
        };
        let label = format!("{} {what} to {name}", if copy { "Copying" } else { "Moving" });
        let root = self.scanned_path.clone();
        // Select the results only when they land in the folder on screen (a paste); a drop
        // onto another folder leaves you where you are, as in Finder and Explorer.
        let here = dest == self.full_path(&self.browser.folder);
        self.run_op_select(label, move || {
            let mut affected = Vec::new();
            let mut made = Vec::new();
            let mut errors = Vec::new();
            for s in &sources {
                let r = if copy { actions::copy_into(s, &dest) } else { actions::move_into(s, &dest) };
                match r {
                    Ok(to) => {
                        if !copy {
                            affected.push(s.clone());
                        }
                        if to.starts_with(&root) {
                            affected.push(to.clone());
                        }
                        made.push(to);
                    }
                    Err(e) => errors.push(e),
                }
            }
            let note = match (copy, sources.len() - errors.len()) {
                (_, 0) => None,
                (true, 1) => Some(format!("Copied to {}", dest.display())),
                (false, 1) => Some(format!("Moved to {}", dest.display())),
                (true, n) => Some(format!("Copied {n} items to {}", dest.display())),
                (false, n) => Some(format!("Moved {n} items to {}", dest.display())),
            };
            let r = if errors.is_empty() { Ok(note) } else { Err(errors.join(" ")) };
            (r, affected, if here { made } else { Vec::new() })
        });
    }

    // ---------- tabs ----------

    fn new_tab(&mut self, folder: PathBuf) {
        let layout = self.browser.layout();
        let b = &mut self.browser;
        b.tabs.insert(b.active + 1, Tab { layout, folder: folder.clone(), back: Vec::new(), fwd: Vec::new() });
        self.switch_tab(self.browser.active + 1);
    }

    fn switch_tab(&mut self, i: usize) {
        if i >= self.browser.tabs.len() {
            return;
        }
        let here = self.full_path(&self.browser.folder);
        self.browser.tab().folder = here;
        self.browser.active = i;
        self.load_tab();
    }

    /// Show the active tab's folder, and take the map there.
    fn load_tab(&mut self) {
        let disk = self.browser.tab().folder.clone();
        let f = self.resolve_folder(&disk);
        self.set_folder(f.clone());
        self.zoom = f;
        self.browser.seen_zoom = Some(self.zoom.clone());
    }

    fn close_tab(&mut self, i: usize) {
        let b = &mut self.browser;
        if b.tabs.len() < 2 || i >= b.tabs.len() {
            return;
        }
        b.tabs.remove(i);
        if i == b.active {
            b.active = i.min(b.tabs.len() - 1);
            self.load_tab();
        } else if i < b.active {
            b.active -= 1;
        }
    }

    fn tab_title(&self, i: usize) -> String {
        let t = &self.browser.tabs[i];
        if i == self.browser.active {
            if let Some(n) = self.shown_tree().as_ref().and_then(|tr| tr.at(&self.browser.folder).map(|n| n.name.to_string())) {
                return n;
            }
        }
        t.folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| t.folder.display().to_string())
    }

    // ---------- the panel ----------

    /// The file manager: tabs, the path bar and the chosen layout.
    pub(super) fn file_manager(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.shown_tree() else { return };
        self.browser_sync(&tree);
        let rect = ui.max_rect();
        self.browser.rect = Some(rect);
        // Which part of the window has the keyboard: the one clicked last.
        let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        if let Some(p) = pressed {
            if rect.contains(p) {
                self.browser.focused = true;
            } else if self.browser.map_rect.is_some_and(|r| r.contains(p)) {
                self.browser.focused = false;
            }
        }
        let prev_status = self.hover_status.clone();
        let mut picks = Picks::default();

        ui.spacing_mut().item_spacing.y = 2.0;
        self.tabs_row(ui, &mut picks);
        let bar = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| self.browser_bar(ui, &tree, &mut picks));
        let bar_rect = bar.response.rect;
        ui.painter().hline(rect.x_range(), bar_rect.bottom() + 1.0, ui.visuals().widgets.noninteractive.bg_stroke);
        ui.add_space(2.0);

        let mut folder = self.browser.folder.clone();
        let mut order = self.browser_order(&tree, &folder);
        if self.browser.layout() == Layout::Tree {
            self.tree_panel(ui);
        } else {
            let pointer_in = ui.rect_contains_pointer(rect);
            self.browser_keys(ui.ctx(), &tree, &order, pointer_in, &mut picks);
            // The keys may have opened another folder (or tab): draw that one.
            if self.browser.folder != folder {
                folder = self.browser.folder.clone();
                order = self.browser_order(&tree, &folder);
            }
            let footer_h = 20.0;
            let mut body = ui.available_rect_before_wrap();
            body.max.y -= footer_h;
            // The background (before the items, so they're on top of it): drops and a menu.
            let empty = ui.interact(body, ui.id().with("fm-body"), Sense::click());
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
            child.set_clip_rect(body.intersect(ui.clip_rect()));
            match self.browser.layout() {
                Layout::Icons => self.browser_icons(&mut child, &tree, &order, &mut picks),
                Layout::Columns => self.browser_columns(&mut child, &tree, &mut picks),
                _ => self.browser_list(&mut child, &tree, &order, &mut picks),
            }
            // A drop on the empty part of the view goes into the current folder.
            let busy = self.ops.job.is_some();
            let can_paste = self.browser.clip.is_some() && !busy;
            empty.context_menu(|ui| {
                let ctx = ui.ctx().clone();
                let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
                if ui.add_enabled(!busy, egui::Button::new("New Folder").shortcut_text(k(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::N))).clicked() {
                    picks.new_folder = true;
                }
                if ui.add_enabled(can_paste, egui::Button::new("Paste").shortcut_text(k(egui::Modifiers::COMMAND, egui::Key::V))).clicked() {
                    picks.paste = true;
                }
                if ui.button("Open in New Tab").clicked() {
                    picks.new_tab = Some(self.full_path(&folder));
                }
            });
            if empty.clicked() {
                self.browser.picked.clear();
            }
            if picks.drop.is_none() {
                if let Some(p) = empty.dnd_release_payload::<DragItems>() {
                    let copy = ui.input(|i| i.modifiers.alt);
                    picks.drop = Some((p.0.clone(), self.full_path(&folder), copy));
                }
            }
            let footer = Rect::from_min_max(pos2(rect.left(), body.bottom()), pos2(rect.right(), body.bottom() + footer_h));
            self.browser_footer(ui, footer, &tree, &order);
            ui.allocate_rect(Rect::from_min_max(body.min, footer.max), Sense::hover());
        }
        self.browser.scroll_to_sel = false;
        self.drag_preview(ui.ctx());
        self.apply_picks(picks, &order);
        if self.hover_status != prev_status {
            ui.ctx().request_repaint();
        }
    }

    fn apply_picks(&mut self, picks: Picks, order: &[u32]) {
        if let Some(p) = picks.hover {
            self.hover_status = Some(self.hover_text(&p));
        }
        if let Some(k) = picks.sort {
            if self.browser.sort == k {
                self.browser.reversed = !self.browser.reversed;
            } else {
                self.browser.sort = k;
                self.browser.reversed = false;
            }
        }
        if let Some((folder, p)) = picks.column_select {
            // Moving along the columns: the map follows, but it isn't a place to go Back to.
            self.set_folder(folder.clone());
            self.zoom = folder;
            self.browser.seen_zoom = Some(self.zoom.clone());
            self.browser_select(p);
        }
        if let Some((p, m)) = picks.select {
            self.click_select(p, m, order);
        }
        if let Some(p) = picks.go {
            self.browse_to(p);
        }
        if let Some(p) = picks.open {
            self.browser_select(p.clone());
            self.browse_into(p);
        }
        if let Some((a, p)) = picks.menu {
            // A right-click on an item outside the selection selects just that item first.
            if !self.chosen().contains(&p) {
                self.browser_select(p.clone());
            }
            match a {
                TreeAction::Toggle => self.browse_into(p),
                a => self.browser_action(a, p),
            }
            self.browser.seen_sel = self.selected.clone();
        }
        if let Some((items, dest, copy)) = picks.drop {
            self.transfer(items, dest, copy);
        }
        if let Some(f) = picks.new_tab {
            self.new_tab(f);
        }
        if let Some(i) = picks.switch_tab {
            self.switch_tab(i);
        }
        if let Some(i) = picks.close_tab {
            self.close_tab(i);
        }
        if picks.new_folder {
            self.new_folder();
        }
        if picks.paste {
            let dest = self.full_path(&self.browser.folder);
            self.paste_into(dest);
        }
    }

    /// While dragging: what will happen, next to the pointer.
    fn drag_preview(&self, ctx: &egui::Context) {
        let Some(items) = egui::DragAndDrop::payload::<DragItems>(ctx) else { return };
        let Some(p) = ctx.pointer_latest_pos() else { return };
        let copy = ctx.input(|i| i.modifiers.alt);
        let what = match items.0.len() {
            1 => items.0[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            n => format!("{n} items"),
        };
        let alt = if cfg!(target_os = "macos") { "⌥" } else { "Alt" };
        let text = if copy { format!("Copy {what}") } else { format!("Move {what}  (hold {alt} to copy)") };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("fm-drag")));
        let font = FontId::proportional(13.0);
        let galley = painter.layout_no_wrap(text, font, Color32::WHITE);
        let r = Rect::from_min_size(p + vec2(14.0, 10.0), galley.size() + vec2(12.0, 6.0));
        painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(30, 90, 170, 235));
        painter.galley(r.min + vec2(6.0, 3.0), galley, Color32::WHITE);
        ctx.request_repaint();
    }

    /// Tabs above the path bar; each can take a drop.
    fn tabs_row(&mut self, ui: &mut egui::Ui, picks: &mut Picks) {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let n = self.browser.tabs.len();
            for i in 0..n {
                let title = self.tab_title(i);
                let short: String = title.chars().take(24).collect();
                let short = if title.chars().count() > 24 { format!("{short}…") } else { short };
                let on = i == self.browser.active;
                let text = if on { egui::RichText::new(short.clone()).strong() } else { egui::RichText::new(short) };
                let resp = ui.add(egui::Button::selectable(on, text)).on_hover_text(self.browser.tabs[i].folder.display().to_string());
                if resp.clicked() && !on {
                    picks.switch_tab = Some(i);
                }
                if resp.middle_clicked() {
                    picks.close_tab = Some(i);
                }
                if resp.dnd_hover_payload::<DragItems>().is_some() {
                    ui.painter().rect_stroke(resp.rect, 3.0, Stroke::new(2.0, ui.visuals().selection.stroke.color), egui::StrokeKind::Inside);
                }
                if let Some(p) = resp.dnd_release_payload::<DragItems>() {
                    let copy = ui.input(|i| i.modifiers.alt);
                    let dest = if on { self.full_path(&self.browser.folder) } else { self.browser.tabs[i].folder.clone() };
                    picks.drop = Some((p.0.clone(), dest, copy));
                }
                if n > 1 {
                    let x = ui.add(egui::Button::new(egui::RichText::new("✖").small()).frame(false)).on_hover_text(format!("Close tab ({})", k(egui::Modifiers::COMMAND, egui::Key::W)));
                    if x.clicked() {
                        picks.close_tab = Some(i);
                    }
                }
                ui.add_space(4.0);
            }
            let plus = ui.add(egui::Button::new("➕").frame(false)).on_hover_text(format!("New tab ({})", k(egui::Modifiers::COMMAND, egui::Key::T)));
            if plus.clicked() {
                picks.new_tab = Some(self.full_path(&self.browser.folder));
            }
        });
    }

    /// Back, Forward, Up, the path, file commands, and the layout and sort controls.
    fn browser_bar(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, picks: &mut Picks) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let ctx = ui.ctx().clone();
            let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
            let (can_back, can_fwd) = {
                let t = &self.browser.tabs[self.browser.active];
                (!t.back.is_empty(), !t.fwd.is_empty())
            };
            if ui.add_enabled(can_back, egui::Button::new("◀")).on_hover_text(format!("Back ({})", k(egui::Modifiers::ALT, egui::Key::ArrowLeft))).clicked() {
                self.browser_back();
            }
            if ui.add_enabled(can_fwd, egui::Button::new("▶")).on_hover_text(format!("Forward ({})", k(egui::Modifiers::ALT, egui::Key::ArrowRight))).clicked() {
                self.browser_forward();
            }
            if ui.add_enabled(!self.browser.folder.is_empty(), egui::Button::new("⏶")).on_hover_text(format!("Up to the enclosing folder (Backspace or {})", k(egui::Modifiers::COMMAND, egui::Key::ArrowUp))).clicked() {
                self.browse_up();
            }
            ui.add_space(6.0);
            // The path, each part clickable and a place to drop.
            ui.spacing_mut().item_spacing.x = 3.0;
            let mut n: &Node = tree;
            let crumbs: Vec<(usize, String)> = std::iter::once((0, format!("🖴 {}", tree.name)))
                .chain(self.browser.folder.iter().enumerate().filter_map(|(k, &i)| {
                    n = n.children.get(i as usize)?;
                    Some((k + 1, n.name.to_string()))
                }))
                .collect();
            let last = crumbs.len() - 1;
            for (k, (depth, label)) in crumbs.into_iter().enumerate() {
                if k > 0 {
                    ui.weak("›");
                }
                let text = if k == last { egui::RichText::new(label).strong() } else { egui::RichText::new(label) };
                let resp = ui.add(egui::Button::new(text).frame(false));
                if resp.clicked() && k != last {
                    picks.go = Some(self.browser.folder[..depth].to_vec());
                }
                if resp.dnd_hover_payload::<DragItems>().is_some() {
                    ui.painter().rect_stroke(resp.rect, 3.0, Stroke::new(2.0, ui.visuals().selection.stroke.color), egui::StrokeKind::Inside);
                }
                if let Some(p) = resp.dnd_release_payload::<DragItems>() {
                    let copy = ui.input(|i| i.modifiers.alt);
                    picks.drop = Some((p.0.clone(), self.full_path(&self.browser.folder[..depth]), copy));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if self.browser.layout() != Layout::Tree {
                    let b = &mut self.browser;
                    let arrow = if b.reversed != b.sort.natural_desc() { "⏷" } else { "⏶" };
                    egui::ComboBox::from_id_salt("browser-sort")
                        .selected_text(format!("Sort: {} {arrow}", b.sort.label()))
                        .width(140.0)
                        .show_ui(ui, |ui| {
                            for key in SortKey::ALL {
                                if ui.selectable_label(b.sort == key, key.label()).clicked() {
                                    picks.sort = Some(key);
                                }
                            }
                        })
                        .response
                        .on_hover_text("Choose the order; pick the same one again to reverse it");
                    ui.add_space(6.0);
                }
                let t = self.browser.tab();
                for (layout, label, tip) in [
                    (Layout::Columns, "◫ Columns", "A column per folder, from the top of the scan down to here"),
                    (Layout::Icons, "⊞ Icons", "Large icons in a grid"),
                    (Layout::List, "☰ List", "One line per item, with sizes and dates"),
                    (Layout::Tree, "🌲 Tree", "The whole scan as a folder tree"),
                ] {
                    ui.selectable_value(&mut t.layout, layout, label).on_hover_text(tip);
                }
                ui.add_space(6.0);
                let busy = self.ops.job.is_some();
                let paste_tip = match &self.browser.clip {
                    Some(c) => format!("Paste {} here ({})", if c.paths.len() == 1 { "1 item".into() } else { format!("{} items", c.paths.len()) }, k(egui::Modifiers::COMMAND, egui::Key::V)),
                    None => "Paste: copy or cut something first".into(),
                };
                if ui.add_enabled(self.browser.clip.is_some() && !busy, egui::Button::new("Paste")).on_hover_text(paste_tip).on_disabled_hover_text("Paste: copy or cut something first").clicked() {
                    picks.paste = true;
                }
                if ui.add_enabled(!busy, egui::Button::new("New Folder")).on_hover_text(format!("New folder here ({})", k(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::N))).clicked() {
                    picks.new_folder = true;
                }
            });
        });
    }

    /// Count and total of the folder, and what's selected.
    fn browser_footer(&mut self, ui: &mut egui::Ui, rect: Rect, tree: &Arc<Node>, order: &[u32]) {
        let p = ui.painter();
        p.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
        let Some(folder) = tree.at(&self.browser.folder) else { return };
        let items = order.len();
        let mut text = format!("{} item{} · {}", group(items as u64), if items == 1 { "" } else { "s" }, scan::human(folder.size));
        let picked = &self.browser.picked;
        if picked.len() > 1 {
            let size: u64 = picked.iter().filter_map(|&i| folder.children.get(i as usize)).map(|n| n.size).sum();
            let share = if folder.size > 0 { size as f64 * 100.0 / folder.size as f64 } else { 0.0 };
            text.push_str(&format!("  ·  {} selected: {} ({share:.1}% of this folder)", picked.len(), scan::human(size)));
        } else if let Some(n) = self.selected_in(&self.browser.folder).and_then(|i| folder.children.get(i as usize)) {
            let share = if folder.size > 0 { n.size as f64 * 100.0 / folder.size as f64 } else { 0.0 };
            text.push_str(&format!("  ·  “{}” selected: {} ({share:.1}% of this folder)", n.name, scan::human(n.size)));
        }
        if let Some(c) = &self.browser.clip {
            text.push_str(&format!("  ·  {} {} to paste", if c.cut { "cut:" } else { "copied:" }, c.paths.len()));
        }
        p.text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), ui.visuals().weak_text_color());
    }

    /// Arrow keys, Enter, Backspace, Back/Forward, tabs, select all, new folder, paste and
    /// type-to-select.
    fn browser_keys(&mut self, ctx: &egui::Context, tree: &Arc<Node>, order: &[u32], pointer_in: bool, picks: &mut Picks) {
        use egui::{Key, Modifiers as M};
        if self.ops.modal() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let help_hovered = self.help.hovered;
        // The mouse's side buttons go to whichever part is under the pointer.
        let mouse = pointer_in && !help_hovered;
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
            Some(false) => return self.browser_back(),
            Some(true) => return self.browser_forward(),
            None => {}
        }
        if !self.browser.focused {
            return;
        }
        let cols = match self.browser.layout() {
            Layout::Icons => self.browser.grid_cols.max(1) as i64,
            _ => 1,
        };
        let folder = self.browser.folder.clone();
        let pos = self.selected_in(&folder).and_then(|i| order.iter().position(|&o| o == i)).map(|p| p as i64);
        let now = ctx.input(|i| i.time);
        enum Do {
            Move(i64),
            To(i64),
            Up,
            Into,
            Open,
            Back,
            Forward,
            All,
            NewTab,
            CloseTab,
            NextTab(bool),
            NewFolder,
            Paste(String),
            Find(String),
        }
        let layout = self.browser.layout();
        let act = ctx.input_mut(|i| {
            if i.consume_key(M::ALT, Key::ArrowLeft) {
                return Some(Do::Back);
            }
            if i.consume_key(M::ALT, Key::ArrowRight) {
                return Some(Do::Forward);
            }
            if i.consume_key(M::COMMAND, Key::ArrowUp) || i.consume_key(M::NONE, Key::Backspace) {
                return Some(Do::Up);
            }
            if i.consume_key(M::COMMAND, Key::ArrowDown) {
                return Some(Do::Open);
            }
            if i.consume_key(M::COMMAND, Key::A) {
                return Some(Do::All);
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
            if i.consume_key(M::COMMAND | M::SHIFT, Key::N) {
                return Some(Do::NewFolder);
            }
            let pasted = i.events.iter().find_map(|e| match e {
                egui::Event::Paste(t) => Some(t.clone()),
                _ => None,
            });
            if let Some(t) = pasted {
                return Some(Do::Paste(t));
            }
            if i.consume_key(M::NONE, Key::ArrowDown) {
                return Some(Do::Move(cols));
            }
            if i.consume_key(M::NONE, Key::ArrowUp) {
                return Some(Do::Move(-cols));
            }
            if i.consume_key(M::NONE, Key::Home) {
                return Some(Do::To(0));
            }
            if i.consume_key(M::NONE, Key::End) {
                return Some(Do::To(i64::MAX));
            }
            if i.consume_key(M::NONE, Key::PageDown) {
                return Some(Do::Move(20 * cols));
            }
            if i.consume_key(M::NONE, Key::PageUp) {
                return Some(Do::Move(-20 * cols));
            }
            match layout {
                Layout::Icons => {
                    if i.consume_key(M::NONE, Key::ArrowRight) {
                        return Some(Do::Move(1));
                    }
                    if i.consume_key(M::NONE, Key::ArrowLeft) {
                        return Some(Do::Move(-1));
                    }
                }
                Layout::Columns => {
                    if i.consume_key(M::NONE, Key::ArrowRight) {
                        return Some(Do::Into);
                    }
                    if i.consume_key(M::NONE, Key::ArrowLeft) {
                        return Some(Do::Up);
                    }
                }
                _ => {}
            }
            if i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt {
                return None;
            }
            let typed: String = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect();
            (!typed.is_empty()).then_some(Do::Find(typed))
        });
        let Some(act) = act else { return };
        let len = order.len() as i64;
        let pick = |app: &mut Self, p: i64| {
            if len > 0 {
                let mut path = app.browser.folder.clone();
                path.push(order[p.clamp(0, len - 1) as usize]);
                app.browser_select(path);
            }
        };
        match act {
            Do::Move(d) => pick(self, pos.map_or(if d > 0 { 0 } else { len - 1 }, |p| p + d)),
            Do::To(p) => pick(self, p),
            Do::Up => self.browse_up(),
            Do::Back => self.browser_back(),
            Do::Forward => self.browser_forward(),
            Do::All => {
                if len > 0 {
                    if pos.is_none() {
                        pick(self, 0);
                    }
                    self.browser.picked = order.iter().copied().collect();
                    if self.browser.picked.len() < 2 {
                        self.browser.picked.clear();
                    }
                }
            }
            Do::NewTab => picks.new_tab = Some(self.full_path(&folder)),
            Do::CloseTab => picks.close_tab = Some(self.browser.active),
            Do::NextTab(fwd) => {
                let n = self.browser.tabs.len();
                picks.switch_tab = Some(if fwd { (self.browser.active + 1) % n } else { (self.browser.active + n - 1) % n });
            }
            Do::NewFolder => picks.new_folder = true,
            Do::Paste(text) => {
                let ours = self.browser.clip.as_ref().is_some_and(|c| text_of(&c.paths) == text.trim_end());
                if ours || self.browser.clip.is_some() && text.trim().is_empty() {
                    picks.paste = true;
                } else {
                    // Paths copied elsewhere (one per line), e.g. "Copy as path" in Explorer.
                    let paths: Vec<PathBuf> = text
                        .lines()
                        .map(|l| PathBuf::from(l.trim().trim_matches('"')))
                        .filter(|p| p.is_absolute() && fs_exists(p))
                        .collect();
                    if !paths.is_empty() && paths.len() == text.lines().filter(|l| !l.trim().is_empty()).count() {
                        let dest = self.full_path(&folder);
                        self.transfer(paths, dest, true);
                    } else if self.browser.clip.is_some() {
                        picks.paste = true;
                    }
                }
            }
            Do::Open => {
                if let Some(s) = self.selected.clone().filter(|_| pos.is_some()) {
                    self.browse_into(s);
                }
            }
            Do::Into => {
                // Columns: into the selected folder, selecting its first item.
                if let Some(s) = self.selected.clone().filter(|_| pos.is_some()) {
                    if tree.at(&s).is_some_and(|n| n.is_dir && !n.children.is_empty()) {
                        let first = self.browser_order(tree, &s).first().copied();
                        self.set_folder(s.clone());
                        self.zoom = s.clone();
                        self.browser.seen_zoom = Some(self.zoom.clone());
                        if let Some(f) = first {
                            let mut p = s;
                            p.push(f);
                            self.browser_select(p);
                        }
                    }
                }
            }
            Do::Find(t) => {
                if now - self.browser.typed_at > 1.0 {
                    self.browser.typed.clear();
                }
                self.browser.typed_at = now;
                self.browser.typed.push_str(&t.to_lowercase());
                let want = self.browser.typed.clone();
                if let Some(f) = tree.at(&folder) {
                    let hit = order.iter().position(|&i| f.children[i as usize].name.to_lowercase().starts_with(&want));
                    if let Some(p) = hit {
                        pick(self, p as i64);
                    }
                }
            }
        }
    }

    // ---------- List ----------

    fn list_columns(width: f32) -> Vec<(Col, f32)> {
        let mut cols = vec![(Col::Size, 84.0), (Col::Share, 112.0), (Col::Kind, 130.0), (Col::Items, 72.0), (Col::Modified, 132.0)];
        // Narrow windows lose the less important columns first.
        for drop in [Col::Items, Col::Kind, Col::Share, Col::Modified] {
            if width - cols.iter().map(|c| c.1).sum::<f32>() >= 220.0 {
                break;
            }
            cols.retain(|c| c.0 != drop);
        }
        let fixed: f32 = cols.iter().map(|c| c.1).sum();
        cols.insert(0, (Col::Name, (width - fixed).max(160.0)));
        cols
    }

    fn browser_list(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, order: &Arc<Vec<u32>>, picks: &mut Picks) {
        let folder_path = self.browser.folder.clone();
        let Some(folder) = tree.at(&folder_path) else { return };
        let width = ui.available_width();
        let cols = Self::list_columns(width);

        // Headings.
        let (hdr, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
        ui.painter().rect_filled(hdr, 0.0, ui.visuals().faint_bg_color);
        let mut x = hdr.left();
        for (i, (col, w)) in cols.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(x, hdr.top()), vec2(*w, ROW));
            x += w;
            let sense = if col.sort().is_some() { Sense::click() } else { Sense::hover() };
            let resp = ui.interact(cell, ui.id().with(("bhdr", i)), sense);
            let on = col.sort() == Some(self.browser.sort);
            let arrow = if !on { "" } else if self.browser.reversed != self.browser.sort.natural_desc() { " ⏷" } else { " ⏶" };
            let color = if resp.hovered() || on { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
            let (at, align) = match col {
                Col::Name | Col::Kind | Col::Share => (pos2(cell.left() + 8.0, cell.center().y), Align2::LEFT_CENTER),
                _ => (pos2(cell.right() - 8.0, cell.center().y), Align2::RIGHT_CENTER),
            };
            ui.painter().text(at, align, format!("{}{arrow}", col.title()), FontId::proportional(13.0), color);
            if resp.on_hover_text(if col.sort().is_some() { "Click to sort; again to reverse" } else { "Share of this folder's size" }).clicked() {
                picks.sort = col.sort();
            }
        }

        if order.is_empty() {
            Self::empty_note(ui, self.shown.is_some());
            return;
        }
        let hits = self.current_hits();
        let sel_row = self.selected_in(&folder_path).and_then(|s| order.iter().position(|&o| o == s));
        let mut area = egui::ScrollArea::vertical().id_salt(("blist", &folder_path)).auto_shrink(false);
        if let Some(r) = sel_row.filter(|_| self.browser.scroll_to_sel) {
            area = area.vertical_scroll_offset(keep_in_view(self.browser.view, r as f32 * ROW, ROW));
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let out = area.show_rows(ui, ROW, order.len(), |ui, range| {
            for r in range {
                let i = order[r];
                let n = &folder.children[i as usize];
                let mut path = folder_path.clone();
                path.push(i);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click_and_drag());
                let selected = self.is_chosen(&folder_path, i);
                let dim = self.browser_dim(n, &path, &hits);
                let vis = ui.visuals().clone();
                let (sel_bg, sel_fg) = self.sel_colors(&vis);
                let p = ui.painter().clone();
                if selected {
                    p.rect_filled(rect, 3.0, sel_bg);
                } else if resp.hovered() {
                    p.rect_filled(rect, 3.0, vis.widgets.hovered.weak_bg_fill);
                } else if r % 2 == 1 && self.view().stripes {
                    p.rect_filled(rect, 0.0, vis.faint_bg_color);
                }
                let tc = if selected {
                    sel_fg
                } else if dim {
                    vis.weak_text_color().gamma_multiply(0.6)
                } else {
                    vis.text_color()
                };
                let weak = if selected || dim { tc } else { vis.weak_text_color() };
                let font = FontId::proportional(13.0);
                let cy = rect.center().y;
                let tint = self.icon_tint(n, &path);
                let tint = if dim { custom::dim(tint) } else { tint };
                let mut x = rect.left();
                for (col, w) in &cols {
                    let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, ROW));
                    x += w;
                    let right = pos2(cell.right() - 8.0, cy);
                    match col {
                        Col::Name => {
                            let icon = Rect::from_min_size(pos2(cell.left() + 8.0, cy - 8.0), vec2(16.0, 16.0));
                            self.icons.paint(ui, icon, n.is_dir, n.ext(), tint);
                            let clip = Rect::from_min_max(pos2(cell.left() + 30.0, rect.top()), pos2(cell.right() - 6.0, rect.bottom()));
                            let np = p.with_clip_rect(clip.intersect(p.clip_rect()));
                            np.text(pos2(clip.left(), cy), Align2::LEFT_CENTER, &*n.name, font.clone(), tc);
                        }
                        Col::Size => {
                            p.text(right, Align2::RIGHT_CENTER, scan::human(n.size), font.clone(), tc);
                        }
                        Col::Share => {
                            let frac = if folder.size > 0 { n.size as f32 / folder.size as f32 } else { 0.0 };
                            let bar = Rect::from_min_size(pos2(cell.left() + 8.0, cy - 4.0), vec2(54.0, 8.0));
                            p.rect_filled(bar, 2.0, vis.extreme_bg_color);
                            let mut fill = bar;
                            fill.set_width(bar.width() * frac.clamp(0.0, 1.0));
                            let c = if n.is_dir { Color32::from_rgb(90, 140, 220) } else { tint };
                            p.rect_filled(fill, 2.0, if dim { custom::dim(c) } else { c });
                            p.text(right, Align2::RIGHT_CENTER, format!("{:.1}%", frac * 100.0), font.clone(), weak);
                        }
                        Col::Kind => {
                            let clip = cell.shrink2(vec2(6.0, 0.0));
                            p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(cell.left() + 8.0, cy), Align2::LEFT_CENTER, kind_of(n), font.clone(), weak);
                        }
                        Col::Items => {
                            let t = if n.is_dir { group(n.files) } else { "—".into() };
                            p.text(right, Align2::RIGHT_CENTER, t, font.clone(), weak);
                        }
                        Col::Modified => {
                            if n.modified > 0 {
                                p.text(right, Align2::RIGHT_CENTER, fmt_time(n.modified), font.clone(), weak);
                            }
                        }
                    }
                }
                self.item_input(ui, &resp, &path, n, picks);
            }
        });
        self.browser.view = (out.state.offset.y, out.inner_rect.height());
    }

    /// Clicks, double-clicks, hover, dragging, dropping and the right-click menu, the same in
    /// every layout.
    fn item_input(&self, ui: &egui::Ui, resp: &egui::Response, path: &[u32], n: &Node, picks: &mut Picks) {
        if resp.double_clicked() {
            picks.open = Some(path.to_vec());
        } else if resp.clicked() {
            picks.select = Some((path.to_vec(), ui.input(|i| i.modifiers)));
        } else if resp.secondary_clicked() && !self.chosen().iter().any(|c| c == path) {
            picks.select = Some((path.to_vec(), egui::Modifiers::NONE));
        }
        if resp.middle_clicked() && n.is_dir {
            picks.new_tab = Some(self.full_path(path));
        }
        if resp.hovered() {
            picks.hover = Some(path.to_vec());
        }
        // Drag what's selected (or just this, if it isn't), drop onto folders.
        if resp.drag_started() && !path.is_empty() {
            let chosen = self.chosen();
            let items: Vec<PathBuf> = if chosen.iter().any(|c| c == path) {
                chosen.iter().map(|c| self.full_path(c)).collect()
            } else {
                vec![self.full_path(path)]
            };
            resp.dnd_set_drag_payload(DragItems(items));
        }
        if n.is_dir {
            if resp.dnd_hover_payload::<DragItems>().is_some() {
                ui.painter().rect_stroke(resp.rect, 3.0, Stroke::new(2.0, ui.visuals().selection.stroke.color), egui::StrokeKind::Inside);
            }
            if let Some(p) = resp.dnd_release_payload::<DragItems>() {
                let copy = ui.input(|i| i.modifiers.alt);
                let dest = self.full_path(path);
                if !p.0.contains(&dest) {
                    picks.drop = Some((p.0.clone(), dest, copy));
                }
            }
        }
        let count = if self.chosen().iter().any(|c| c == path) { self.chosen().len() } else { 1 };
        let info = ItemInfo {
            is_dir: n.is_dir,
            open: false,
            is_root: path.is_empty(),
            is_drive: path.is_empty() && self.scanned_path.parent().is_none(),
            busy: self.ops.job.is_some(),
            in_browser: true,
            count,
            can_paste: self.browser.clip.is_some(),
        };
        resp.context_menu(|ui| {
            if let Some(a) = Self::tree_menu(ui, &info) {
                picks.menu = Some((a, path.to_vec()));
            }
        });
    }

    fn empty_note(ui: &mut egui::Ui, filtered: bool) {
        let msg = if filtered { "Nothing in this folder matches the filter" } else { "This folder is empty" };
        ui.add_space(30.0);
        ui.vertical_centered(|ui| ui.weak(msg));
    }

    // ---------- Icons ----------

    fn browser_icons(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, order: &Arc<Vec<u32>>, picks: &mut Picks) {
        let folder_path = self.browser.folder.clone();
        let Some(folder) = tree.at(&folder_path) else { return };
        if order.is_empty() {
            Self::empty_note(ui, self.shown.is_some());
            return;
        }
        let width = ui.available_width() - 12.0;
        let per_row = ((width / CELL_W).floor() as usize).max(1);
        self.browser.grid_cols = per_row;
        let rows = order.len().div_ceil(per_row);
        let hits = self.current_hits();
        let sel_row = self.selected_in(&folder_path).and_then(|s| order.iter().position(|&o| o == s)).map(|p| p / per_row);
        let mut area = egui::ScrollArea::vertical().id_salt(("bicons", &folder_path)).auto_shrink(false);
        if let Some(r) = sel_row.filter(|_| self.browser.scroll_to_sel) {
            area = area.vertical_scroll_offset(keep_in_view(self.browser.view, r as f32 * CELL_H, CELL_H));
        }
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let out = area.show_rows(ui, CELL_H, rows, |ui, range| {
            for row in range {
                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), CELL_H), Sense::hover());
                for c in 0..per_row {
                    let Some(&i) = order.get(row * per_row + c) else { break };
                    let n = &folder.children[i as usize];
                    let mut path = folder_path.clone();
                    path.push(i);
                    let cell = Rect::from_min_size(pos2(line.left() + 6.0 + c as f32 * CELL_W, line.top()), vec2(CELL_W, CELL_H)).shrink(3.0);
                    let resp = ui.interact(cell, ui.id().with(("bicon", i)), Sense::click_and_drag());
                    let selected = self.is_chosen(&folder_path, i);
                    let dim = self.browser_dim(n, &path, &hits);
                    let vis = ui.visuals().clone();
                    let (sel_bg, sel_fg) = self.sel_colors(&vis);
                    let p = ui.painter().clone();
                    if selected {
                        p.rect_filled(cell, 6.0, sel_bg);
                    } else if resp.hovered() {
                        p.rect_filled(cell, 6.0, vis.widgets.hovered.weak_bg_fill);
                    }
                    let tint = self.icon_tint(n, &path);
                    let tint = if dim { custom::dim(tint) } else { tint };
                    let icon = Rect::from_center_size(pos2(cell.center().x, cell.top() + 28.0), vec2(42.0, 42.0));
                    self.icons.paint(ui, icon, n.is_dir, n.ext(), tint);
                    if dim {
                        p.rect_filled(icon, 0.0, vis.panel_fill.gamma_multiply(0.6));
                    }
                    // Share of the folder, as a thin bar under the icon.
                    let frac = if folder.size > 0 { n.size as f32 / folder.size as f32 } else { 0.0 };
                    let bar = Rect::from_center_size(pos2(cell.center().x, cell.top() + 55.0), vec2(56.0, 3.0));
                    p.rect_filled(bar, 1.5, vis.extreme_bg_color);
                    let mut fill = bar;
                    fill.set_width((bar.width() * frac).max(if frac > 0.0 { 1.0 } else { 0.0 }));
                    p.rect_filled(fill, 1.5, if n.is_dir { Color32::from_rgb(90, 140, 220) } else { tint });
                    let tc = if selected {
                        sel_fg
                    } else if dim {
                        vis.weak_text_color().gamma_multiply(0.6)
                    } else {
                        vis.text_color()
                    };
                    let mut job = egui::text::LayoutJob::simple(n.name.to_string(), FontId::proportional(12.0), tc, cell.width() - 8.0);
                    job.wrap.max_rows = 2;
                    job.wrap.break_anywhere = true;
                    job.halign = egui::Align::Center;
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let gh = galley.size().y;
                    p.galley(pos2(cell.center().x, cell.top() + 61.0), galley, tc);
                    let weak = if selected || dim { tc } else { vis.weak_text_color() };
                    p.text(pos2(cell.center().x, cell.top() + 63.0 + gh), Align2::CENTER_TOP, scan::human(n.size), FontId::proportional(11.0), weak);
                    self.item_input(ui, &resp, &path, n, picks);
                }
            }
        });
        self.browser.view = (out.state.offset.y, out.inner_rect.height());
    }

    // ---------- Columns ----------

    fn browser_columns(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, picks: &mut Picks) {
        // One column per folder from the top down to the current one, then the selected
        // folder's contents or the selected file's details.
        let current = self.browser.folder.clone();
        let mut folders: Vec<Vec<u32>> = (0..=current.len()).map(|k| current[..k].to_vec()).collect();
        let sel_here = self.selected_in(&current).filter(|_| self.browser.picked.len() < 2).map(|i| {
            let mut p = current.clone();
            p.push(i);
            p
        });
        let mut preview = None;
        if let Some(s) = sel_here {
            match tree.at(&s) {
                Some(n) if n.is_dir => folders.push(s),
                Some(_) => preview = Some(s),
                None => {}
            }
        }
        let height = ui.available_height();
        let hits = self.current_hits();
        egui::ScrollArea::horizontal().id_salt("bcols").auto_shrink(false).stick_to_right(true).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for (k, folder_path) in folders.iter().enumerate() {
                    let order = self.browser_order(tree, folder_path);
                    let Some(folder) = tree.at(folder_path) else { continue };
                    let active = k == current.len();
                    // The item that leads to the next column.
                    let mark = if k < current.len() { Some(current[k]) } else { None };
                    let (col_rect, col_resp) = ui.allocate_exact_size(vec2(COL_W, height), Sense::hover());
                    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(col_rect).layout(egui::Layout::top_down(egui::Align::Min)));
                    cui.set_clip_rect(col_rect.intersect(ui.clip_rect()));
                    ui.painter().vline(col_rect.right(), col_rect.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);
                    if col_resp.dnd_hover_payload::<DragItems>().is_some() {
                        ui.painter().rect_stroke(col_rect, 0.0, Stroke::new(1.0, ui.visuals().selection.stroke.color), egui::StrokeKind::Inside);
                    }
                    if let Some(p) = col_resp.dnd_release_payload::<DragItems>() {
                        picks.drop = Some((p.0.clone(), self.full_path(folder_path), ui.input(|i| i.modifiers.alt)));
                    }
                    if order.is_empty() {
                        Self::empty_note(&mut cui, self.shown.is_some());
                        continue;
                    }
                    let mark_row = mark.or_else(|| if active { self.selected_in(folder_path) } else { None }).and_then(|m| order.iter().position(|&o| o == m));
                    let mut area = egui::ScrollArea::vertical().id_salt(("bcol", folder_path)).auto_shrink(false);
                    if let Some(r) = mark_row.filter(|_| self.browser.scroll_to_sel) {
                        let y = r as f32 * ROW;
                        area = area.vertical_scroll_offset((y - height / 2.0).max(0.0));
                    }
                    cui.spacing_mut().item_spacing.y = 0.0;
                    area.show_rows(&mut cui, ROW, order.len(), |ui, range| {
                        for r in range {
                            let i = order[r];
                            let n = &folder.children[i as usize];
                            let mut path = folder_path.clone();
                            path.push(i);
                            let (rect, resp) = ui.allocate_exact_size(vec2(COL_W - 1.0, ROW), Sense::click_and_drag());
                            let on = if active { self.is_chosen(folder_path, i) } else { mark == Some(i) };
                            let vis = ui.visuals().clone();
                            let (sel_bg, sel_fg) = self.sel_colors(&vis);
                            let p = ui.painter().clone();
                            let dim = self.browser_dim(n, &path, &hits);
                            if on {
                                // The trail to the current folder is drawn quieter than the selection.
                                let fill = if active { sel_bg } else { vis.widgets.inactive.weak_bg_fill };
                                p.rect_filled(rect.shrink2(vec2(3.0, 0.0)), 3.0, fill);
                            } else if resp.hovered() {
                                p.rect_filled(rect.shrink2(vec2(3.0, 0.0)), 3.0, vis.widgets.hovered.weak_bg_fill);
                            }
                            let tc = if on && active {
                                sel_fg
                            } else if dim {
                                vis.weak_text_color().gamma_multiply(0.6)
                            } else {
                                vis.text_color()
                            };
                            let weak = if (on && active) || dim { tc } else { vis.weak_text_color() };
                            let cy = rect.center().y;
                            let tint = self.icon_tint(n, &path);
                            let tint = if dim { custom::dim(tint) } else { tint };
                            self.icons.paint(ui, Rect::from_min_size(pos2(rect.left() + 8.0, cy - 8.0), vec2(16.0, 16.0)), n.is_dir, n.ext(), tint);
                            let size_w = 74.0;
                            p.text(pos2(rect.right() - 20.0, cy), Align2::RIGHT_CENTER, scan::human(n.size), FontId::proportional(11.5), weak);
                            if n.is_dir {
                                p.text(pos2(rect.right() - 8.0, cy), Align2::CENTER_CENTER, "›", FontId::proportional(14.0), weak);
                            }
                            let clip = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(rect.right() - 20.0 - size_w, rect.bottom()));
                            p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(clip.left(), cy), Align2::LEFT_CENTER, &*n.name, FontId::proportional(13.0), tc);
                            // A plain click in another column makes that column's folder the current one.
                            let before = picks.select.is_some();
                            self.item_input(ui, &resp, &path, n, picks);
                            if !before && !active {
                                if let Some((s, _)) = picks.select.take() {
                                    picks.column_select = Some((folder_path.clone(), s));
                                }
                            }
                        }
                    });
                }
                if let Some(p) = preview {
                    self.file_preview(ui, tree, &p, height);
                }
            });
        });
    }

    /// The last column when a file is selected: a big icon and what's known about it.
    fn file_preview(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, path: &[u32], height: f32) {
        let Some(n) = tree.at(path) else { return };
        let (rect, _) = ui.allocate_exact_size(vec2(COL_W + 40.0, height), Sense::hover());
        let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(14.0)).layout(egui::Layout::top_down(egui::Align::Center)));
        cui.add_space(10.0);
        let (icon, _) = cui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
        let tint = self.icon_tint(n, path);
        self.icons.paint(&cui, icon, false, n.ext(), tint);
        cui.add_space(8.0);
        cui.add(egui::Label::new(egui::RichText::new(&*n.name).strong()).wrap());
        cui.weak(format!("{} · {}", kind_of(n), scan::human(n.size)));
        cui.add_space(8.0);
        egui::Grid::new("bpreview").num_columns(2).spacing([10.0, 4.0]).show(&mut cui, |ui| {
            ui.weak("Size");
            ui.label(format!("{} ({} bytes)", scan::human(n.size), group(n.size)));
            ui.end_row();
            if n.modified > 0 {
                ui.weak("Last change");
                ui.label(fmt_time(n.modified));
                ui.end_row();
            }
            if let Some(parent) = tree.at(&path[..path.len() - 1]).filter(|p| p.size > 0) {
                ui.weak("Of folder");
                ui.label(format!("{:.1}%", n.size as f64 * 100.0 / parent.size as f64));
                ui.end_row();
            }
        });
        cui.add_space(10.0);
        let mut act = None;
        cui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui.button("Open").clicked() {
                act = Some(TreeAction::Open);
            }
            if ui.button(format!("Show in {}", actions::FILE_MANAGER)).clicked() {
                act = Some(TreeAction::Reveal);
            }
        });
        if let Some(a) = act {
            self.tree_action(a, path.to_vec());
        }
    }
}

/// Paths as clipboard text, one per line.
fn text_of(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")
}

fn fs_exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

/// Scroll offset that brings a row at `y` (height `h`) into a view last at `(offset, height)`,
/// moving as little as possible.
fn keep_in_view(view: (f32, f32), y: f32, h: f32) -> f32 {
    let (off, height) = view;
    if y < off {
        y
    } else if y + h > off + height {
        (y + h - height).max(0.0)
    } else {
        off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, modified: i64) -> Node {
        Node { name: name.into(), size, files: 1, modified, is_dir: false, children: Vec::new() }
    }

    fn dir(name: &str, children: Vec<Node>) -> Node {
        let size = children.iter().map(|c| c.size).sum();
        let files = children.iter().map(|c| c.files).sum();
        Node { name: name.into(), size, files, modified: 0, is_dir: true, children }
    }

    fn names(n: &Node, key: SortKey, reversed: bool) -> Vec<&str> {
        sorted(n, key, reversed).into_iter().map(|i| &*n.children[i as usize].name).collect()
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("shot2.exr", "shot10.exr"), Cmp::Less);
        assert_eq!(natural_cmp("Apple", "banana"), Cmp::Less);
        assert_eq!(natural_cmp("a01", "a1"), Cmp::Greater);
        assert_eq!(natural_cmp("a", "a"), Cmp::Equal);
    }

    #[test]
    fn sorting() {
        let root = dir(
            "r",
            vec![
                file("b10.png", 50, 3),
                dir("zeta", vec![file("x", 500, 1)]),
                file("b2.png", 10, 9),
                dir("Alpha", vec![file("y", 1, 2), file("z", 1, 2)]),
                file("notes.txt", 100, 5),
            ],
        );
        // Names: folders first, then files in natural order; reversed keeps folders first.
        assert_eq!(names(&root, SortKey::Name, false), ["Alpha", "zeta", "b2.png", "b10.png", "notes.txt"]);
        assert_eq!(names(&root, SortKey::Name, true), ["zeta", "Alpha", "notes.txt", "b10.png", "b2.png"]);
        assert_eq!(names(&root, SortKey::Size, false), ["zeta", "notes.txt", "b10.png", "b2.png", "Alpha"]);
        assert_eq!(names(&root, SortKey::Size, true)[0], "Alpha");
        assert_eq!(names(&root, SortKey::Modified, false)[0], "b2.png");
        assert_eq!(names(&root, SortKey::Items, false)[..2], ["Alpha", "zeta"]);
        assert_eq!(names(&root, SortKey::Kind, false), ["Alpha", "zeta", "b2.png", "b10.png", "notes.txt"]);
        assert_eq!(kind_of(&root.children[0]), "PNG image");
        assert_eq!(kind_of(&root.children[1]), "Folder");
        assert_eq!(kind_of(&file("README", 1, 0)), "File");
    }

    #[test]
    fn scrolling_moves_as_little_as_possible() {
        assert_eq!(keep_in_view((100.0, 200.0), 150.0, 20.0), 100.0);
        assert_eq!(keep_in_view((100.0, 200.0), 40.0, 20.0), 40.0);
        assert_eq!(keep_in_view((100.0, 200.0), 300.0, 20.0), 120.0);
    }
}
