//! The Columns and Gallery views of a Files pane, and grouping in the Details view.
//!
//! Columns shows the folders above the current one at the left, each with the next folder down
//! marked, then the current folder (drawn by the Details code, names only), then what's in the
//! selected folder or a preview of the selected file. The side folders are read in the background
//! like any other listing, and dropped a few seconds after they leave the screen.
//!
//! Gallery shows a large preview of the item under the cursor above a strip of thumbnails.

use crate::fsys::PathShow;
use super::files::{order_entries, FAction, FPicks, FSort, ItemCtx, item_actions};
use super::*;
use crate::fsys::listing::{Entry, Listing};
use crate::fsys::history::Location;
use serde::{Deserialize, Serialize};
use std::path::Path;
use eframe::egui::Pos2;

/// The width of a column in the Columns view.
pub(super) const COL_W: f32 = 230.0;
/// Side columns aren't drawn for this many frames: forget them.
const SIDE_KEEP: u64 = 600;
/// Sort side folders with more entries than this on another thread.
const SIDE_INLINE_MAX: usize = 3000;
/// The Gallery's strip of thumbnails.
const STRIP_H: f32 = 118.0;
const STRIP_CELL: f32 = 104.0;

// ---- grouping ----

/// How the Details view groups items, under headings.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(super) enum FGroup {
    #[default]
    None,
    Type,
    Modified,
    Size,
}

impl FGroup {
    pub(super) const ALL: [FGroup; 4] = [FGroup::None, FGroup::Type, FGroup::Modified, FGroup::Size];

    pub(super) fn title(self) -> &'static str {
        match self {
            FGroup::None => "None",
            FGroup::Type => "Type",
            FGroup::Modified => "Date Modified",
            FGroup::Size => "Size",
        }
    }
}

/// A group heading: where its items start in the display order, and its title.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct GroupHead {
    pub start: u32,
    pub label: String,
}

/// Which group an entry falls in: a rank to order the groups by, and the heading.
fn group_of(e: &Entry, g: FGroup, today: chrono::NaiveDate) -> (i64, String) {
    use chrono::Datelike;
    match g {
        FGroup::None => (0, String::new()),
        FGroup::Type => {
            if matches!(crate::apps::of(e.bundle), Some(crate::apps::Bundle::App | crate::apps::Bundle::WinApp | crate::apps::Bundle::AppImage | crate::apps::Bundle::Launcher)) {
                (0, "Applications".into())
            } else if e.is_dir {
                (0, "Folders".into())
            } else {
                (1, browser::kind_for(&e.name, false))
            }
        }
        FGroup::Size => {
            if e.is_dir {
                return (0, "Folders".into());
            }
            const KB: u64 = 1 << 10;
            const MB: u64 = 1 << 20;
            const GB: u64 = 1 << 30;
            let (rank, label) = match e.shown_size() {
                0 => (7, "Empty"),
                s if s < 16 * KB => (6, "Tiny (under 16 KB)"),
                s if s < MB => (5, "Small (16 KB to 1 MB)"),
                s if s < 128 * MB => (4, "Medium (1 to 128 MB)"),
                s if s < GB => (3, "Large (128 MB to 1 GB)"),
                s if s < 4 * GB => (2, "Huge (1 to 4 GB)"),
                _ => (1, "Gigantic (over 4 GB)"),
            };
            (rank, label.into())
        }
        FGroup::Modified => {
            let Some(d) = chrono::DateTime::from_timestamp(e.modified, 0).filter(|_| e.modified > 0).map(|t| t.with_timezone(&chrono::Local).date_naive()) else {
                return (1 << 40, "Date unknown".into());
            };
            let days = (today - d).num_days();
            let week_start = today - chrono::Days::new(today.weekday().num_days_from_monday() as u64);
            let last_week = week_start - chrono::Days::new(7);
            let month_start = today.with_day(1).unwrap_or(today);
            let last_month = month_start.pred_opt().and_then(|x| x.with_day(1)).unwrap_or(month_start);
            if days < 0 {
                (-1, "In the future".into())
            } else if days == 0 {
                (0, "Today".into())
            } else if days == 1 {
                (1, "Yesterday".into())
            } else if d >= week_start {
                (2, "Earlier this week".into())
            } else if d >= last_week {
                (3, "Last week".into())
            } else if d >= month_start {
                (4, "Earlier this month".into())
            } else if d >= last_month {
                (5, "Last month".into())
            } else if d.year() == today.year() {
                (6, "Earlier this year".into())
            } else {
                (10 + (today.year() - d.year()) as i64, d.year().to_string())
            }
        }
    }
}

/// A run of numbered frames shown as one row. The row is the first frame's entry.
#[derive(Clone, Debug)]
pub(super) struct SeqRow {
    pub seq: crate::sequences::Sequence,
    /// Every frame's entry, in frame order (the first is the row's own).
    pub members: Vec<u32>,
    /// Newest frame's time.
    pub modified: i64,
    /// What the row shows: `shot010_comp.####.exr`.
    pub label: String,
}

/// Sequence rows by their entry.
pub(super) type Seqs = HashMap<u32, SeqRow>;

/// Frames needed before numbered files show as one sequence row.
const SEQ_MIN: usize = 3;

/// Fold the numbered frames in `order` into one row per sequence, where the first frame was.
pub(super) fn fold_sequences(entries: &[Entry], order: &mut Vec<u32>) -> Seqs {
    let files: Vec<u32> = order.iter().copied().filter(|&i| !entries[i as usize].is_dir).collect();
    let named: Vec<(&str, u64)> = files.iter().map(|&i| (&*entries[i as usize].name, entries[i as usize].size)).collect();
    let mut out = Seqs::new();
    let mut hide = std::collections::HashSet::new();
    let pos: HashMap<u32, usize> = order.iter().enumerate().map(|(p, &i)| (i, p)).collect();
    for (seq, members) in crate::sequences::in_names(&named, SEQ_MIN) {
        let members: Vec<u32> = members.into_iter().map(|k| files[k]).collect();
        let modified = members.iter().map(|&i| entries[i as usize].modified).max().unwrap_or(0);
        // The row stands where the earliest-listed frame was.
        let head = *members.iter().min_by_key(|&&i| pos[&i]).unwrap();
        hide.extend(members.iter().copied().filter(|&m| m != head));
        let label = seq.display();
        out.insert(head, SeqRow { seq, members, modified, label });
    }
    if !hide.is_empty() {
        order.retain(|i| !hide.contains(i));
    }
    out
}

/// The display order of a folder, and its group headings when grouped: the groups in their own
/// order (folders first; newest or largest first unless the list is reversed by that column),
/// and the items within each in the sort order.
#[allow(clippy::too_many_arguments)]
pub(super) fn arrange(
    entries: &[Entry],
    sizes: Option<&files::FolderSizes>,
    sort: FSort,
    reversed: bool,
    hidden: bool,
    filter: &str,
    group: FGroup,
    sequences: bool,
) -> (Vec<u32>, Vec<GroupHead>, Seqs) {
    let mut order = order_entries(entries, sizes, sort, reversed, hidden, filter);
    let seqs = if sequences { fold_sequences(entries, &mut order) } else { Seqs::new() };
    if group == FGroup::None || order.is_empty() {
        return (order, Vec::new(), seqs);
    }
    let today = chrono::Local::now().date_naive();
    let flip = reversed && matches!((group, sort), (FGroup::Modified, FSort::Modified) | (FGroup::Size, FSort::Size) | (FGroup::Type, FSort::Type));
    let mut keys: HashMap<u32, (i64, String)> = HashMap::with_capacity(order.len());
    for &i in &order {
        keys.insert(i, group_of(&entries[i as usize], group, today));
    }
    // Stable, so each group keeps the sort order.
    order.sort_by(|a, b| {
        let (ka, kb) = (&keys[a], &keys[b]);
        let folders_first = (ka.0 != 0 || group == FGroup::Modified).cmp(&(kb.0 != 0 || group == FGroup::Modified));
        let o = ka.0.cmp(&kb.0).then_with(|| browser::natural_cmp(&ka.1, &kb.1));
        folders_first.then(if flip { o.reverse() } else { o })
    });
    let mut heads: Vec<GroupHead> = Vec::new();
    for (pos, i) in order.iter().enumerate() {
        let label = &keys[i].1;
        if heads.last().is_none_or(|h| &h.label != label) {
            heads.push(GroupHead { start: pos as u32, label: label.clone() });
        }
    }
    (order, heads, seqs)
}

/// A line of the grouped Details list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Line {
    /// A heading: group number.
    Head(usize),
    /// An item: its position in the display order.
    Item(usize),
}

/// What's on display line `r` of a list with these groups.
pub(super) fn line_at(heads: &[GroupHead], r: usize) -> Line {
    if heads.is_empty() {
        return Line::Item(r);
    }
    // The last group whose heading is at or before r (heading g sits at line start + g).
    let (mut lo, mut hi) = (0, heads.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if heads[mid].start as usize + mid <= r {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let g = lo.saturating_sub(1);
    let at = heads[g].start as usize + g;
    if r == at { Line::Head(g) } else { Line::Item(r - g - 1) }
}

/// The display line of the item at position `pos`.
pub(super) fn line_of(heads: &[GroupHead], pos: usize) -> usize {
    pos + heads.partition_point(|h| h.start as usize <= pos)
}

// ---- folders beside the current one ----

/// What a side folder's order was worked out for: (listing generation, sort, reversed, hidden).
type SideKey = (u64, FSort, bool, bool);

pub(super) struct Side {
    listing: Listing,
    order: Vec<u32>,
    key: Option<SideKey>,
    sorting: Option<(SideKey, mpsc::Receiver<Vec<u32>>)>,
    used: u64,
    /// The marked item it was last scrolled to.
    scrolled_for: Option<Box<str>>,
}

/// Listings of the folders shown beside the current one (Columns view).
#[derive(Default)]
pub(super) struct SideLists {
    map: HashMap<PathBuf, Side>,
    frame: u64,
}

impl SideLists {
    /// Once a frame: forget folders no longer shown.
    pub(super) fn tick(&mut self) {
        self.frame += 1;
        let f = self.frame;
        self.map.retain(|_, s| s.used + SIDE_KEEP > f);
    }

    /// Read them all again (F5, or a change noticed in them).
    pub(super) fn clear(&mut self) {
        self.map.clear();
    }

    /// Forget one folder, to read it again.
    pub(super) fn forget(&mut self, dir: &Path) {
        self.map.remove(dir);
    }

    /// The folders being shown.
    pub(super) fn dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.map.keys()
    }

    /// A folder's listing and display order, starting to read it if needed.
    fn get(&mut self, ctx: &egui::Context, dir: &Path, sort: FSort, reversed: bool, hidden: bool) -> &mut Side {
        let frame = self.frame;
        let s = self.map.entry(dir.to_path_buf()).or_insert_with(|| Side {
            listing: Listing::start(dir.to_path_buf()),
            order: Vec::new(),
            key: None,
            sorting: None,
            used: frame,
            scrolled_for: None,
        });
        s.used = frame;
        s.listing.poll();
        if !s.listing.done {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let key = (s.listing.generation, sort, reversed, hidden);
        if let Some((k, rx)) = &s.sorting {
            match rx.try_recv() {
                Ok(o) => {
                    if (k.1, k.2, k.3) == (sort, reversed, hidden) {
                        s.order = o;
                        s.key = Some(*k);
                    }
                    s.sorting = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(30)),
                Err(mpsc::TryRecvError::Disconnected) => s.sorting = None,
            }
        }
        if s.key != Some(key) && s.sorting.is_none() {
            let n = s.listing.entries.len();
            if n <= SIDE_INLINE_MAX {
                s.order = order_entries(&s.listing.entries, None, sort, reversed, hidden, "");
                s.key = Some(key);
            } else if !s.listing.done {
                // Still arriving: show it as read, sort once it's all here.
                s.order = (0..n as u32).filter(|&i| hidden || !s.listing.entries[i as usize].hidden).collect();
                s.key = Some(key);
            } else {
                let entries = s.listing.entries.clone();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(order_entries(&entries, None, sort, reversed, hidden, ""));
                });
                s.sorting = Some((key, rx));
                ctx.request_repaint_after(Duration::from_millis(30));
            }
        }
        s
    }
}

/// Where the Columns view puts things.
pub(super) struct ColLayout {
    /// The current folder.
    pub mid: Rect,
    /// Folders above it, the topmost first: (where, folder, the item in it to mark).
    pub left: Vec<(Rect, PathBuf, Box<str>)>,
    /// What's in the selected folder, or the selected file.
    pub right: Option<Rect>,
}

/// Lay out the Columns view in `body`: as many folders above as fit (up to three), leaving room
/// for at least one column at the right.
pub(super) fn columns_layout(body: Rect, dir: &Path) -> ColLayout {
    let w = body.width();
    let colw = COL_W.min(w / 2.0).max(120.0).min(w);
    let above: Vec<&Path> = dir.ancestors().skip(1).collect();
    let room = ((w - colw * 2.0) / colw).floor().max(0.0) as usize;
    let n = above.len().min(room).min(3);
    let mut x = body.left();
    let mut left = Vec::new();
    for k in (0..n).rev() {
        let child = if k == 0 { dir } else { above[k - 1] };
        let mark: Box<str> = child.file_name().map(|n| n.to_string_lossy().into()).unwrap_or_else(|| child.shown().into());
        left.push((Rect::from_min_size(pos2(x, body.top()), vec2(colw, body.height())), above[k].to_path_buf(), mark));
        x += colw;
    }
    let mid = Rect::from_min_size(pos2(x, body.top()), vec2(colw, body.height()));
    x += colw;
    let right = (body.right() - x >= 90.0).then(|| Rect::from_min_max(pos2(x, body.top()), body.max));
    ColLayout { mid, left, right }
}

impl App {
    /// The columns around the current folder in the Columns view.
    pub(super) fn columns_sides(&mut self, ui: &mut egui::Ui, pane: usize, dir: &Path, lay: &ColLayout, picks: &mut FPicks) {
        let pal = style::pal(ui);
        for (rect, folder, mark) in &lay.left {
            self.side_column(ui, pane, *rect, folder, Some(mark), picks);
        }
        ui.painter().vline(lay.mid.right() - 0.5, lay.mid.y_range(), Stroke::new(1.0, pal.hairline));
        let Some(right) = lay.right else { return };
        let t = self.files.panes[pane].tab();
        let Some(e) = t.cursor.as_deref().and_then(|c| t.entry_named(c)).cloned() else { return };
        let path = dir.join(&*e.name);
        if e.is_dir {
            self.side_column(ui, pane, right, &path, None, picks);
        } else {
            let inner = right.shrink2(vec2(14.0, 12.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).id_salt(("fcol-file", pane)).layout(egui::Layout::top_down(egui::Align::Center)));
            child.set_clip_rect(right.intersect(ui.clip_rect()));
            let h = (inner.height() * 0.62).max(80.0);
            self.preview_body(&mut child, &path, &e, vec2(inner.width(), h));
            child.add_space(8.0);
            child.add(egui::Label::new(egui::RichText::new(&*e.name).strong()).wrap());
            child.label(egui::RichText::new(preview::facts_line(&e)).color(pal.weak));
        }
    }

    /// One folder beside the current one: its items, with `mark` (the folder open to its right)
    /// highlighted. A click selects that item in its folder, as the current folder; a double-click
    /// opens it; items can be dropped on its folders.
    fn side_column(&mut self, ui: &mut egui::Ui, pane: usize, rect: Rect, dir: &Path, mark: Option<&str>, picks: &mut FPicks) {
        let pal = style::pal(ui);
        let ctx = ui.ctx().clone();
        let row_h = self.files.prefs.density.row();
        let (sort, reversed) = (self.files.panes[pane].sort, self.files.panes[pane].reversed);
        let hidden = self.files.prefs.show_hidden;
        ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, pal.hairline));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(0.0, 2.0))).id_salt(("fcol", dir)).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let ui = &mut child;
        let icons = &mut self.icons;
        let colors = &self.colors;
        let color_for = |ext: &str| colors.get(ext).or_else(|| colors.get(&ext.to_lowercase())).copied().unwrap_or(Color32::GRAY);
        let side = self.files.side.get(&ctx, dir, sort, reversed, hidden);
        let l = &side.listing;
        if side.order.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                if !l.done {
                    ui.spinner();
                } else if let Some(e) = &l.error {
                    ui.label(egui::RichText::new(e).color(pal.weak).size(12.0));
                } else {
                    ui.label(egui::RichText::new(if l.entries.is_empty() { "Empty folder" } else { "Only hidden items" }).color(pal.weak).size(12.0));
                }
            });
            return;
        }
        let mut area = egui::ScrollArea::vertical().id_salt(("fcol-scroll", dir)).auto_shrink(false);
        if side.scrolled_for.as_deref() != mark
            && let Some(m) = mark
            && let Some(r) = side.order.iter().position(|&i| &*l.entries[i as usize].name == m)
        {
            // Bring the marked folder into the middle the first time it's shown.
            area = area.vertical_scroll_offset((r as f32 * row_h - rect.height() / 2.0).max(0.0));
            side.scrolled_for = Some(m.into());
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let font = FontId::proportional(13.0);
        let mut drops: Vec<(egui::Response, PathBuf)> = Vec::new();
        let order = &side.order;
        area.show_rows(ui, row_h, order.len(), |ui, range| {
            for r in range {
                let e = &l.entries[order[r] as usize];
                let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let marked = mark == Some(&*e.name);
                let p = ui.painter();
                if marked {
                    p.rect_filled(row.shrink2(vec2(3.0, 0.5)), style::RADIUS as f32, pal.sel_dim);
                } else if resp.hovered() {
                    p.rect_filled(row.shrink2(vec2(3.0, 0.5)), style::RADIUS as f32, pal.hover);
                }
                let cy = row.center().y;
                let icon = Rect::from_min_size(pos2(row.left() + 10.0, cy - 8.0), vec2(16.0, 16.0));
                icons.paint_entry(ui, icon, dir, e, if e.hidden { Color32::from_gray(120) } else { color_for(e.ext()) }, false);
                let app = crate::apps::is_app(e.bundle);
                let right = if e.is_dir && !app { row.right() - 18.0 } else { row.right() - 6.0 };
                let clip = Rect::from_min_max(pos2(row.left() + 32.0, row.top()), pos2(right, row.bottom()));
                let tc = if marked { pal.strong } else if e.hidden { pal.weak } else { pal.text };
                let p = ui.painter();
                p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(clip.left(), cy), Align2::LEFT_CENTER, crate::apps::shown_name(&e.name, e.bundle), font.clone(), tc);
                if e.is_dir && !app {
                    chevron(p, pos2(row.right() - 11.0, cy), if marked { pal.strong } else { pal.weak });
                }
                let path = dir.join(&*e.name);
                if resp.double_clicked() {
                    if crate::apps::launches(e.bundle) {
                        picks.menu = Some((FAction::LaunchApp, vec![path.clone()]));
                    } else if e.is_dir || (crate::fsys::archive::is_archive(&e.name) && !crate::fsys::archive::inside(&path)) {
                        picks.go = Some(Location::Dir(path.clone()));
                    } else {
                        picks.menu = Some((FAction::Open, vec![path.clone()]));
                    }
                } else if resp.clicked() {
                    picks.reveal = Some((dir.to_path_buf(), e.name.clone()));
                }
                if resp.middle_clicked() && e.is_dir {
                    picks.new_tab = Some(Location::Dir(path.clone()));
                }
                if e.is_dir {
                    drops.push((resp, path));
                }
            }
        });
        for (resp, p) in drops {
            if let Some(d) = self.folder_drop(ui, &resp, &p) {
                picks.drop = Some(d);
            }
        }
    }

    /// The Gallery view: the item under the cursor large, a strip of thumbnails below.
    pub(super) fn gallery(&mut self, ui: &mut egui::Ui, pane: usize, focused: bool, dir: &Path, picks: &mut FPicks) {
        let pal = style::pal(ui);
        let rect = ui.max_rect();
        let strip = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STRIP_H), rect.max);
        let big = Rect::from_min_max(rect.min, pos2(rect.right(), strip.top()));
        // Something is always shown: the first item, until another is picked.
        {
            let t = self.files.panes[pane].tab_mut();
            if t.cursor.as_deref().and_then(|c| t.pos_of(c)).is_none()
                && let (Some(&i), Some(l)) = (t.order.first(), t.listing.as_ref())
            {
                let name = l.entries[i as usize].name.clone();
                t.selected = std::iter::once(name.clone()).collect();
                t.anchor = Some(name.clone());
                t.cursor = Some(name);
                t.scroll_to_cursor = true;
            }
        }
        ui.painter().hline(strip.x_range(), strip.top() + 0.5, Stroke::new(1.0, pal.hairline));
        let can_paste = self.can_paste();
        let two_panes = self.files.panes.len() > 1;
        let files = &mut self.files;
        let (panes, thumb_cache, favourites, marks, apps) = (&files.panes, &mut files.thumbs, &files.prefs.favourites, &files.marks, &files.prefs.apps);
        let icons = &mut self.icons;
        let colors = &self.colors;
        let color_for = |ext: &str| colors.get(ext).or_else(|| colors.get(&ext.to_lowercase())).copied().unwrap_or(Color32::GRAY);
        let t = panes[pane].tab();
        let Some(l) = t.listing.as_ref() else { return };
        let n = t.order.len();
        let sizes = t.sizes.as_ref().map(|s| &s.1);
        let cursor = t.cursor.as_deref().and_then(|c| t.pos_of(c));
        let mut strip_ui = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(0.0, 4.0))).id_salt(("fstrip", pane)).layout(egui::Layout::left_to_right(egui::Align::Min)));
        strip_ui.set_clip_rect(strip.intersect(ui.clip_rect()));
        let mut area = egui::ScrollArea::horizontal().id_salt(("fstrip", pane, dir)).auto_shrink(false);
        if let Some(k) = cursor.filter(|_| t.scroll_to_cursor) {
            area = area.horizontal_scroll_offset(browser::keep_in_view((t.view.0, strip.width()), k as f32 * STRIP_CELL, STRIP_CELL));
        }
        let (sel_bg, sel_fg) = if focused { (pal.sel, pal.strong) } else { (pal.sel_dim, pal.strong) };
        let mut drops: Vec<(egui::Response, PathBuf)> = Vec::new();
        let cell_h = STRIP_H - 14.0;
        let out = area.show_viewport(&mut strip_ui, |ui, vp| {
            ui.set_min_size(vec2(n as f32 * STRIP_CELL, cell_h));
            let origin = ui.max_rect().min;
            let first = (vp.min.x / STRIP_CELL).floor().max(0.0) as usize;
            let last = ((vp.max.x / STRIP_CELL).ceil() as usize + 1).min(n);
            for k in first..last {
                let i = t.order[k];
                let e = &l.entries[i as usize];
                let cell = Rect::from_min_size(origin + vec2(k as f32 * STRIP_CELL, 0.0), vec2(STRIP_CELL, cell_h)).shrink(3.0);
                let resp = ui.interact(cell, ui.id().with(("fstrip-cell", k)), Sense::click_and_drag());
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
                let size = 64.0;
                let pic = Rect::from_center_size(pos2(cell.center().x, cell.top() + 6.0 + size / 2.0), vec2(size, size));
                let path = dir.join(&*e.name);
                let thumb = if !e.is_dir && !e.not_local() && thumbs::wants_thumb(&e.name) { thumb_cache.get(&path, thumbs::stamp(e.size, e.modified), e.size).map(|t| (t.id(), t.size_vec2())) } else { None };
                if let Some((id, tsize)) = thumb {
                    let scale = (size / tsize.x).min(size / tsize.y);
                    let r = Rect::from_center_size(pic.center(), tsize * scale);
                    p.image(id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    let tint = if e.hidden { Color32::from_gray(120) } else { color_for(e.ext()) };
                    icons.paint_entry(ui, Rect::from_center_size(pic.center(), vec2(size * 0.8, size * 0.8)), dir, e, tint, true);
                }
                let tc = if selected { sel_fg } else if e.hidden { pal.weak } else { pal.text };
                let shown = t.seqs.get(&i).map_or_else(|| crate::apps::shown_name(&e.name, e.bundle).to_string(), |s| s.label.clone());
                let mut job = egui::text::LayoutJob::simple(shown, FontId::proportional(11.5), tc, cell.width() - 8.0);
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.halign = egui::Align::Center;
                let g = ui.fonts_mut(|f| f.layout_job(job));
                p.galley(pos2(cell.center().x, pic.bottom() + 6.0), g, tc);
                let pinned = e.is_dir && favourites.iter().any(|g| g.items.iter().any(|f| f.path == path));
                let ic = ItemCtx { t, l, dir, sizes, two: two_panes, can_paste, marks, apps };
                let resp = resp.on_hover_text_at_pointer(e.name.to_string());
                item_actions(ui, resp, i, &ic, pinned, picks, &mut drops);
            }
        });
        for (resp, p) in drops {
            if let Some(d) = self.folder_drop(ui, &resp, &p) {
                picks.drop = Some(d);
            }
        }
        let t = self.files.panes[pane].tab_mut();
        t.view = (out.state.offset.x, out.inner_rect.width());
        t.scroll_to_cursor = false;
        // The big preview, with the name and facts under it.
        let t = self.files.panes[pane].tab();
        let Some(e) = t.cursor.as_deref().and_then(|c| t.entry_named(c)).cloned() else { return };
        let path = dir.join(&*e.name);
        let inner = big.shrink2(vec2(16.0, 12.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).id_salt(("fgallery", pane)).layout(egui::Layout::top_down(egui::Align::Center)));
        child.set_clip_rect(big.intersect(ui.clip_rect()));
        let text_h = 40.0;
        self.preview_body(&mut child, &path, &e, vec2(inner.width(), (inner.height() - text_h).max(60.0)));
        child.add_space(6.0);
        child.label(egui::RichText::new(&*e.name).strong());
        child.label(egui::RichText::new(preview::facts_line(&e)).color(pal.weak).size(12.0));
    }
}

/// A small › pointing right: "this folder opens to the right".
pub(super) fn chevron(p: &egui::Painter, c: Pos2, color: Color32) {
    let pts = vec![c + vec2(-2.0, -4.0), c + vec2(2.0, 0.0), c + vec2(-2.0, 4.0)];
    p.add(egui::Shape::line(pts, Stroke::new(1.3, color)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64, modified: i64) -> Entry {
        Entry { name: name.into(), is_dir, is_link: false, hidden: false, size, alloc: size, flags: 0, modified, bundle: 0 }
    }

    #[test]
    fn groups_and_lines() {
        let es = vec![
            entry("b.png", false, 20 << 20, 0),
            entry("docs", true, 0, 0),
            entry("a.txt", false, 100, 0),
            entry("c.png", false, 5, 0),
            entry("empty", false, 0, 0),
        ];
        let (order, heads, _) = arrange(&es, None, FSort::Name, false, true, "", FGroup::Type, false);
        let names: Vec<&str> = order.iter().map(|&i| &*es[i as usize].name).collect();
        assert_eq!(names[0], "docs");
        assert_eq!(heads[0], GroupHead { start: 0, label: "Folders".into() });
        assert_eq!(heads.len(), 4, "{heads:?}");
        // The two PNGs together, in name order.
        let png = heads.iter().position(|h| h.label.contains("PNG")).unwrap();
        let s = heads[png].start as usize;
        assert_eq!(&names[s..s + 2], &["b.png", "c.png"]);

        let (order, heads, _) = arrange(&es, None, FSort::Name, false, true, "", FGroup::Size, false);
        let labels: Vec<&str> = heads.iter().map(|h| h.label.as_str()).collect();
        assert_eq!(labels, ["Folders", "Medium (1 to 128 MB)", "Tiny (under 16 KB)", "Empty"]);
        assert_eq!(order.len(), 5);

        // Lines: heading, item, heading, item...
        let heads = vec![GroupHead { start: 0, label: "A".into() }, GroupHead { start: 2, label: "B".into() }];
        let lines: Vec<Line> = (0..6).map(|r| line_at(&heads, r)).collect();
        assert_eq!(lines, [Line::Head(0), Line::Item(0), Line::Item(1), Line::Head(1), Line::Item(2), Line::Item(3)]);
        for pos in 0..4 {
            assert_eq!(line_at(&heads, line_of(&heads, pos)), Line::Item(pos));
        }
        assert_eq!(line_at(&[], 3), Line::Item(3));
    }

    #[test]
    fn dates_group_by_age() {
        let now = chrono::Local::now();
        let today = now.date_naive();
        let e = entry("x", false, 1, now.timestamp());
        assert_eq!(group_of(&e, FGroup::Modified, today).1, "Today");
        let old = entry("y", false, 1, 946_684_800 + 86_400 * 40);
        assert_eq!(group_of(&old, FGroup::Modified, today).1, "2000");
        assert_eq!(group_of(&entry("z", false, 1, 0), FGroup::Modified, today).1, "Date unknown");
    }

    #[test]
    fn columns_fit_the_width() {
        let body = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 400.0));
        let lay = columns_layout(body, Path::new("/a/b/c/d"));
        // 1000 px: two columns above, this one, and the rest at the right.
        assert_eq!(lay.left.len(), 2);
        assert_eq!(lay.left[0].1, Path::new("/a/b"));
        assert_eq!(&*lay.left[0].2, "c");
        assert_eq!(&*lay.left[1].2, "d");
        assert!(lay.right.is_some());
        let narrow = columns_layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 400.0)), Path::new("/a/b"));
        assert!(narrow.left.is_empty());
        let root = columns_layout(body, Path::new("/"));
        assert!(root.left.is_empty());
    }
}
