//! Favourites: the user's bookmarked folders in the sidebar, in groups they name and order.
//!
//! A favourite is a bookmark. Dragging a folder *between* favourites (or onto a group's title)
//! adds a bookmark and moves nothing. Dropping items *onto* a favourite moves or copies them
//! into that folder, as with any folder. The label by the pointer always says which.

use crate::fsys::PathShow;
use super::dnd::{self, DragFav, DragItems};
use super::files::FPicks;
use super::*;
use crate::fsys::history::Location;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One bookmarked folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Fav {
    /// What the sidebar shows; empty means the folder's own name.
    #[serde(default)]
    pub label: String,
    pub path: PathBuf,
}

impl Fav {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { label: String::new(), path }
    }

    pub(super) fn shown(&self) -> String {
        if self.label.trim().is_empty() { dnd::folder_name(&self.path) } else { self.label.clone() }
    }
}

/// A named group of favourites.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct FavGroup {
    pub name: String,
    #[serde(default)]
    pub items: Vec<Fav>,
    #[serde(default)]
    pub folded: bool,
}

impl FavGroup {
    pub(super) fn new(name: &str) -> Self {
        Self { name: name.into(), items: Vec::new(), folded: false }
    }
}

/// Move folders pinned before groups existed into the first group, and make sure there's one.
pub(super) fn migrate(p: &mut files::FilesPrefs) {
    if p.favourites.is_empty() {
        p.favourites.push(FavGroup::new("Favourites"));
    }
    for path in std::mem::take(&mut p.pinned) {
        if !p.favourites.iter().any(|g| g.items.iter().any(|f| f.path == path)) {
            p.favourites[0].items.push(Fav::new(path));
        }
    }
}

/// Add folders to a group at `at` (or the end), skipping ones already in that group.
pub(super) fn add(groups: &mut [FavGroup], g: usize, at: Option<usize>, paths: impl IntoIterator<Item = PathBuf>) -> usize {
    let Some(group) = groups.get_mut(g) else { return 0 };
    let mut at = at.unwrap_or(group.items.len()).min(group.items.len());
    let mut n = 0;
    for p in paths {
        if group.items.iter().any(|f| f.path == p) {
            continue;
        }
        group.items.insert(at, Fav::new(p));
        at += 1;
        n += 1;
    }
    n
}

/// Move favourite (g, i) to group `to_g` before position `at`.
pub(super) fn move_fav(groups: &mut [FavGroup], (g, i): (usize, usize), to_g: usize, at: usize) {
    if g >= groups.len() || to_g >= groups.len() || i >= groups[g].items.len() {
        return;
    }
    let f = groups[g].items.remove(i);
    let at = if g == to_g && at > i { at - 1 } else { at };
    if let Some(dup) = groups[to_g].items.iter().position(|x| x.path == f.path) {
        // Already there: just keep that one.
        let _ = dup;
        return;
    }
    let at = at.min(groups[to_g].items.len());
    groups[to_g].items.insert(at, f);
}

/// Remove a group, keeping its favourites in the group above (or below, for the first).
pub(super) fn remove_group(groups: &mut Vec<FavGroup>, g: usize) {
    if groups.len() < 2 || g >= groups.len() {
        return;
    }
    let gone = groups.remove(g);
    let into = g.saturating_sub(1).min(groups.len() - 1);
    for f in gone.items {
        if !groups[into].items.iter().any(|x| x.path == f.path) {
            groups[into].items.push(f);
        }
    }
}

/// A favourite or group name being edited in place.
pub(super) struct FavEdit {
    pub group: usize,
    /// None: the group's name.
    pub item: Option<usize>,
    pub text: String,
    pub fresh: bool,
}

/// Where a drag over a group would land.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Slot {
    /// Between favourites: before this index.
    Before(usize),
    /// Onto favourite i's folder.
    Into(usize),
}

impl App {
    /// The Favourites groups, each a sidebar section of its own.
    pub(super) fn favourites_sections(&mut self, ui: &mut egui::Ui, picks: &mut FPicks) {
        let here = self.files.here().clone();
        let n_groups = self.files.prefs.favourites.len();
        let dragging_items = egui::DragAndDrop::payload::<DragItems>(ui.ctx());
        let dragging_fav = egui::DragAndDrop::payload::<DragFav>(ui.ctx()).map(|d| *d);
        let pointer = ui.ctx().pointer_latest_pos();
        let released = ui.input(|i| i.pointer.any_released());
        let mods = ui.input(|i| i.modifiers);
        let pal = style::pal(ui);
        let mut edit_done: Option<(usize, Option<usize>, Option<String>)> = None;

        for g in 0..n_groups {
            let group = self.files.prefs.favourites[g].clone();
            ui.add_space(6.0);
            // The title: folds the group, takes drops at the end, and has the group menu.
            let title_resp = if let Some(e) = self.files.fav_edit.as_mut().filter(|e| e.group == g && e.item.is_none()) {
                let r = ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let id = egui::Id::new(("fav-edit", g));
                    let r = ui.add(egui::TextEdit::singleline(&mut e.text).id(id).desired_width(ui.available_width() - 8.0).font(egui::TextStyle::Small));
                    let was_fresh = settle_focus(ui, &r, id, e);
                    (r, was_fresh)
                });
                let (r, was_fresh) = r.inner;
                if r.lost_focus() && !was_fresh {
                    let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    edit_done = Some((g, None, (!cancel).then(|| e.text.clone())));
                }
                r
            } else {
                let r = ui
                    .horizontal(|ui| {
                        ui.add_space(6.0);
                        let t = style::section_title(ui, &group.name);
                        let arrow = ui.add(egui::Label::new(egui::RichText::new(if group.folded { "⏵" } else { "⏷" }).size(9.0).color(pal.weak)).sense(Sense::click()));
                        ui.allocate_space(vec2(ui.available_width(), 1.0));
                        t.union(arrow)
                    })
                    .inner;
                let r = r.on_hover_text(if group.folded { "Click to show. Drag a folder here to add it" } else { "Click to fold away. Drag a folder here to add it" });
                if r.clicked() {
                    self.files.prefs.favourites[g].folded = !group.folded;
                }
                r.context_menu(|ui| self.group_menu(ui, g));
                r
            };
            ui.add_space(2.0);

            // Rows, remembering where each is for drops between them.
            let mut rects: Vec<Rect> = Vec::new();
            if !group.folded {
                for (i, f) in group.items.iter().enumerate() {
                    if let Some(e) = self.files.fav_edit.as_mut().filter(|e| e.group == g && e.item == Some(i)) {
                        let id = egui::Id::new(("fav-edit", g, i));
                        let r = ui.add(egui::TextEdit::singleline(&mut e.text).id(id).desired_width(ui.available_width()).hint_text(dnd::folder_name(&f.path)));
                        let was_fresh = settle_focus(ui, &r, id, e);
                        if r.lost_focus() && !was_fresh {
                            let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                            edit_done = Some((g, Some(i), (!cancel).then(|| e.text.clone())));
                        }
                        rects.push(r.rect);
                        continue;
                    }
                    let loc = Location::Dir(f.path.clone());
                    let resp = files::side_row_sense(ui, "★", &f.shown(), here == loc, 24.0, Sense::click_and_drag());
                    rects.push(resp.rect);
                    if resp.drag_started() {
                        egui::DragAndDrop::set_payload(ui.ctx(), DragFav(g, i));
                    }
                    let tip = if f.label.trim().is_empty() { f.path.shown() } else { format!("{}\n{}", f.label, f.path.display()) };
                    let resp = resp.on_hover_text(tip);
                    if resp.clicked() {
                        picks.go = Some(loc.clone());
                    }
                    if resp.middle_clicked() {
                        picks.new_tab = Some(loc.clone());
                    }
                    resp.context_menu(|ui| self.fav_menu(ui, g, i, picks));
                }
                if group.items.is_empty() {
                    let r = ui.add(egui::Label::new(egui::RichText::new("Drag folders here").italics().color(pal.weak)).sense(Sense::hover()));
                    let r = r.on_hover_text("Drag a folder from the list to bookmark it here. Nothing is moved.");
                    rects.push(r.rect.expand2(vec2(ui.available_width(), 2.0)));
                }
            }

            // Drops: work out the slot under the pointer.
            let Some(pos) = pointer else { continue };
            if dragging_items.is_none() && dragging_fav.is_none() {
                continue;
            }
            let on_title = title_resp.rect.expand2(vec2(40.0, 2.0)).contains(pos);
            let span = rects.iter().fold(Rect::NOTHING, |a, r| a.union(*r));
            if !on_title && !(span.is_positive() && span.expand2(vec2(0.0, 3.0)).contains(pos)) {
                continue;
            }
            let empty = group.items.is_empty() || group.folded;
            let slot = if on_title || empty {
                Slot::Before(group.items.len())
            } else {
                let mut s = Slot::Before(group.items.len());
                for (i, r) in rects.iter().enumerate() {
                    if pos.y < r.bottom() {
                        let f = (pos.y - r.top()) / r.height().max(1.0);
                        s = if dragging_items.is_some() && (0.28..=0.72).contains(&f) {
                            Slot::Into(i)
                        } else if f < 0.5 {
                            Slot::Before(i)
                        } else {
                            Slot::Before(i + 1)
                        };
                        break;
                    }
                }
                s
            };
            let line_y = |at: usize| -> f32 {
                if empty || rects.is_empty() {
                    title_resp.rect.bottom() + 1.0
                } else if at < rects.len() {
                    rects[at].top()
                } else {
                    rects[rects.len() - 1].bottom()
                }
            };
            let x = ui.max_rect().x_range();
            match (slot, &dragging_items, dragging_fav) {
                (Slot::Into(i), Some(items), _) => {
                    let dest = group.items[i].path.clone();
                    let op = dnd::op_for(&items.0, &dest, mods, &self.drives);
                    let (text, bad) = dnd::drop_text(&items.0, &dest, op);
                    ui.painter().rect_stroke(rects[i].shrink(1.0), style::RADIUS as f32, Stroke::new(1.5, if bad { pal.weak } else { pal.accent }), egui::StrokeKind::Inside);
                    dnd::pointer_label(ui.ctx(), &text, bad);
                    if released && !bad {
                        egui::DragAndDrop::clear_payload(ui.ctx());
                        picks.drop = Some((items.0.clone(), dest, op));
                    }
                }
                (Slot::Before(at), Some(items), _) => {
                    let folders: Vec<PathBuf> = items.0.iter().zip(&items.1).filter(|(_, d)| **d).map(|(p, _)| p.clone()).collect();
                    let y = line_y(at);
                    let (text, bad) = if folders.is_empty() {
                        ("Only folders can be favourites".to_string(), true)
                    } else {
                        (format!("Add {} to {}  (a bookmark: nothing is moved)", dnd::what(&folders), group.name), false)
                    };
                    if !bad {
                        insertion_line(ui, x, y, pal.accent);
                    }
                    dnd::pointer_label(ui.ctx(), &text, bad);
                    if released && !bad {
                        egui::DragAndDrop::clear_payload(ui.ctx());
                        let n = add(&mut self.files.prefs.favourites, g, Some(at), folders);
                        if n == 0 {
                            self.ops.notice = Some((format!("Already in {}", group.name), false));
                        }
                    }
                }
                (Slot::Before(at), None, Some(DragFav(fg, fi))) => {
                    insertion_line(ui, x, line_y(at), pal.accent);
                    if released {
                        egui::DragAndDrop::clear_payload(ui.ctx());
                        move_fav(&mut self.files.prefs.favourites, (fg, fi), g, at);
                    }
                }
                _ => {}
            }
        }
        if let Some((g, item, text)) = edit_done {
            self.files.fav_edit = None;
            if let Some(text) = text {
                let groups = &mut self.files.prefs.favourites;
                match item {
                    None if !text.trim().is_empty() => groups[g].name = text.trim().to_string(),
                    Some(i) if i < groups[g].items.len() => groups[g].items[i].label = text.trim().to_string(),
                    _ => {}
                }
            }
        }
        // A favourite dragged out of the sidebar: say what a drop will do.
        if let Some(DragFav(g, i)) = dragging_fav
            && let Some(f) = self.files.prefs.favourites.get(g).and_then(|gr| gr.items.get(i))
            && pointer.is_some_and(|p| !ui.max_rect().contains(p))
        {
            dnd::pointer_label(ui.ctx(), &format!("Open {} (drop on a list or a tab)", f.shown()), false);
        }
    }

    fn group_menu(&mut self, ui: &mut egui::Ui, g: usize) {
        let n = self.files.prefs.favourites.len();
        if ui.add(egui::Button::new(style::mi("Rename Group…"))).clicked() {
            let text = self.files.prefs.favourites[g].name.clone();
            self.files.fav_edit = Some(FavEdit { group: g, item: None, text, fresh: true });
        }
        if ui.add(egui::Button::new(style::mi("New Group"))).clicked() {
            let groups = &mut self.files.prefs.favourites;
            groups.insert(g + 1, FavGroup::new("New group"));
            self.files.fav_edit = Some(FavEdit { group: g + 1, item: None, text: "New group".into(), fresh: true });
        }
        ui.separator();
        if ui.add_enabled(g > 0, egui::Button::new(style::mi("Move Group Up"))).clicked() {
            self.files.prefs.favourites.swap(g, g - 1);
        }
        if ui.add_enabled(g + 1 < n, egui::Button::new(style::mi("Move Group Down"))).clicked() {
            self.files.prefs.favourites.swap(g, g + 1);
        }
        ui.separator();
        let into = if g == 0 { 1 } else { g - 1 };
        let tip = if n < 2 { "There has to be at least one group".to_string() } else { format!("Its favourites move to {}", self.files.prefs.favourites[into].name) };
        if ui.add_enabled(n > 1, egui::Button::new(style::mi("Remove Group"))).on_hover_text(tip).on_disabled_hover_text("There has to be at least one group").clicked() {
            remove_group(&mut self.files.prefs.favourites, g);
        }
    }

    fn fav_menu(&mut self, ui: &mut egui::Ui, g: usize, i: usize, picks: &mut FPicks) {
        let f = self.files.prefs.favourites[g].items[i].clone();
        let loc = Location::Dir(f.path.clone());
        if ui.add(egui::Button::new(style::mi("Open in New Tab"))).clicked() {
            picks.new_tab = Some(loc.clone());
        }
        if let Some(o) = self.files.other()
            && ui.add(egui::Button::new(style::mi("Open in Other Pane"))).clicked()
        {
            picks.go_other = Some((o, loc.clone()));
        }
        ui.separator();
        if ui.add(egui::Button::new(style::mi("Rename…"))).on_hover_text("Change the name shown here. The folder itself keeps its name.").clicked() {
            self.files.fav_edit = Some(FavEdit { group: g, item: Some(i), text: f.label.clone(), fresh: true });
        }
        if !f.label.is_empty() && ui.add(egui::Button::new(style::mi("Use Folder's Name"))).clicked() {
            self.files.prefs.favourites[g].items[i].label.clear();
        }
        let groups: Vec<String> = self.files.prefs.favourites.iter().map(|g| g.name.clone()).collect();
        if groups.len() > 1 {
            ui.menu_button(style::mi("Move to Group"), |ui| {
                for (k, name) in groups.iter().enumerate() {
                    if k != g && ui.add(egui::Button::new(style::mi(name))).clicked() {
                        let at = self.files.prefs.favourites[k].items.len();
                        move_fav(&mut self.files.prefs.favourites, (g, i), k, at);
                    }
                }
            });
        }
        if ui.add(egui::Button::new(style::mi("Remove from Favourites"))).on_hover_text("Removes the bookmark only. The folder isn't touched.").clicked() {
            self.files.prefs.favourites[g].items.remove(i);
        }
        ui.separator();
        if ui.add(egui::Button::new(style::mi("Copy Path"))).clicked() {
            picks.menu = Some((files::FAction::CopyPath, vec![f.path.clone()]));
        }
        if ui.add(egui::Button::new(style::mi("Analyse This Folder"))).clicked() {
            picks.menu = Some((files::FAction::Analyse, vec![f.path.clone()]));
        }
    }

    /// Add folders to the first group (Add to Favourites in menus).
    pub(super) fn add_favourites(&mut self, paths: Vec<PathBuf>) {
        let n = add(&mut self.files.prefs.favourites, 0, None, paths);
        let name = self.files.prefs.favourites[0].name.clone();
        self.ops.notice = Some((if n == 0 { format!("Already in {name}") } else { format!("Added to {name}") }, false));
    }

    pub(super) fn remove_favourite(&mut self, p: &Path) {
        for g in &mut self.files.prefs.favourites {
            g.items.retain(|f| f.path != p);
        }
    }

    /// A favourite dropped on a list or a tab: open it there.
    pub(super) fn fav_dropped(&self, resp: &egui::Response) -> Option<PathBuf> {
        let d = resp.dnd_release_payload::<DragFav>()?;
        self.files.prefs.favourites.get(d.0)?.items.get(d.1).map(|f| f.path.clone())
    }
}

/// The accent line showing where a favourite will go.
fn insertion_line(ui: &egui::Ui, x: egui::Rangef, y: f32, c: Color32) {
    let p = ui.painter();
    p.hline(egui::Rangef::new(x.min + 8.0, x.max - 4.0), y, Stroke::new(2.0, c));
    p.circle_stroke(pos2(x.min + 5.0, y), 3.0, Stroke::new(1.5, c));
}

/// Give a just-opened edit field the focus with its text selected, ready to type over. The menu
/// item that opened it acts on the press, so the release that follows would count as a click
/// elsewhere and take the focus away: keep asking until the pointer is quiet. Returns whether the
/// field was still settling (when losing the focus doesn't end the edit).
fn settle_focus(ui: &egui::Ui, r: &egui::Response, id: egui::Id, e: &mut FavEdit) -> bool {
    if !e.fresh {
        return false;
    }
    r.request_focus();
    let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(e.text.chars().count()))));
    st.store(ui.ctx(), id);
    if !ui.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
        e.fresh = false;
    }
    ui.ctx().request_repaint();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(name: &str, paths: &[&str]) -> FavGroup {
        FavGroup { name: name.into(), items: paths.iter().map(|p| Fav::new(PathBuf::from(p))).collect(), folded: false }
    }

    fn paths(g: &FavGroup) -> Vec<String> {
        g.items.iter().map(|f| f.path.shown()).collect()
    }

    #[test]
    fn pinned_folders_become_favourites() {
        let mut p = files::FilesPrefs { pinned: vec!["/a".into(), "/b".into()], ..Default::default() };
        migrate(&mut p);
        assert_eq!(p.favourites.len(), 1);
        assert_eq!(paths(&p.favourites[0]), ["/a", "/b"]);
        assert!(p.pinned.is_empty());
        migrate(&mut p);
        assert_eq!(p.favourites[0].items.len(), 2);
    }

    #[test]
    fn adding_inserts_once_where_dropped() {
        let mut gs = vec![g("A", &["/1", "/3"])];
        assert_eq!(add(&mut gs, 0, Some(1), [PathBuf::from("/2"), PathBuf::from("/1")]), 1);
        assert_eq!(paths(&gs[0]), ["/1", "/2", "/3"]);
        assert_eq!(add(&mut gs, 0, None, [PathBuf::from("/4")]), 1);
        assert_eq!(paths(&gs[0]), ["/1", "/2", "/3", "/4"]);
    }

    #[test]
    fn reordering_and_moving_between_groups() {
        let mut gs = vec![g("A", &["/1", "/2", "/3"]), g("B", &["/x"])];
        move_fav(&mut gs, (0, 0), 0, 3);
        assert_eq!(paths(&gs[0]), ["/2", "/3", "/1"]);
        move_fav(&mut gs, (0, 2), 0, 0);
        assert_eq!(paths(&gs[0]), ["/1", "/2", "/3"]);
        move_fav(&mut gs, (0, 1), 1, 0);
        assert_eq!(paths(&gs[0]), ["/1", "/3"]);
        assert_eq!(paths(&gs[1]), ["/2", "/x"]);
    }

    #[test]
    fn removing_a_group_keeps_its_favourites() {
        let mut gs = vec![g("A", &["/1"]), g("B", &["/1", "/2"])];
        remove_group(&mut gs, 1);
        assert_eq!(gs.len(), 1);
        assert_eq!(paths(&gs[0]), ["/1", "/2"]);
        remove_group(&mut gs, 0);
        assert_eq!(gs.len(), 1);
    }

    #[test]
    fn labels_fall_back_to_the_folder_name() {
        let mut f = Fav::new(PathBuf::from("/work/Projects"));
        assert_eq!(f.shown(), "Projects");
        f.label = "Work".into();
        assert_eq!(f.shown(), "Work");
    }
}
