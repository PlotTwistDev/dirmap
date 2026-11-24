//! Compare two folders (the two panes' folders) and sync them.
//!
//! Compare reads both sides in the background, into and below each folder, and lists what's
//! only on one side, newer on one side, or different. Sizes and dates decide; **Compare
//! contents** also reads files of the same size and date to be sure (local folders only).
//!
//! Sync always shows its plan first, item by item. Update copies what's new or newer one way.
//! Both Ways copies in both directions and never deletes; a file changed differently on both
//! sides (same date, different size) is left alone. Mirror makes the right side match the left,
//! deleting what's only on the right to the bin, and asks again, listing those deletions,
//! before it does anything. Replaced files go to the bin too, so a sync can be undone.

use super::App;
use crate::fsys::PathShow;
use crate::fsys::remote;
use eframe::egui::{self, Color32};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

/// Times this close are the same (FAT and some servers keep 2-second times).
const SLACK: i64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub(super) enum Kind {
    OnlyLeft,
    OnlyRight,
    NewerLeft,
    NewerRight,
    /// Different, and neither is clearly newer (or a file on one side, a folder on the other).
    Conflict,
    Same,
}

impl Kind {
    const ALL: [Kind; 6] = [
        Kind::OnlyLeft,
        Kind::OnlyRight,
        Kind::NewerLeft,
        Kind::NewerRight,
        Kind::Conflict,
        Kind::Same,
    ];
    fn title(self) -> &'static str {
        match self {
            Kind::OnlyLeft => "Only left",
            Kind::OnlyRight => "Only right",
            Kind::NewerLeft => "Newer left",
            Kind::NewerRight => "Newer right",
            Kind::Conflict => "Different",
            Kind::Same => "Same",
        }
    }
    fn colour(self) -> Color32 {
        match self {
            Kind::OnlyLeft | Kind::NewerLeft => Color32::from_rgb(90, 160, 240),
            Kind::OnlyRight | Kind::NewerRight => Color32::from_rgb(110, 190, 110),
            Kind::Conflict => Color32::from_rgb(230, 150, 70),
            Kind::Same => Color32::GRAY,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) struct Info {
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
    /// Only (partly) in the cloud: its contents aren't read, as that would download it.
    pub cloud: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Diff {
    /// Path below the compared folders, with `/` between names.
    pub rel: String,
    pub kind: Kind,
    pub left: Option<Info>,
    pub right: Option<Info>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    ToRight,
    ToLeft,
    BothWays,
    MirrorToRight,
}

impl Mode {
    const ALL: [Mode; 4] = [
        Mode::ToRight,
        Mode::ToLeft,
        Mode::BothWays,
        Mode::MirrorToRight,
    ];
    fn title(self) -> &'static str {
        match self {
            Mode::ToRight => "Update Right (copy what's new or newer on the left)",
            Mode::ToLeft => "Update Left (copy what's new or newer on the right)",
            Mode::BothWays => "Both Ways (newer wins, nothing deleted)",
            Mode::MirrorToRight => "Mirror to Right (right matches left, deletes)",
        }
    }
}

/// What sync does with one item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Act {
    CopyRight,
    CopyLeft,
    DeleteRight,
    Skip,
}

pub(super) fn action(mode: Mode, k: Kind) -> Act {
    match (mode, k) {
        (_, Kind::Same) => Act::Skip,
        (
            Mode::ToRight | Mode::BothWays | Mode::MirrorToRight,
            Kind::OnlyLeft | Kind::NewerLeft,
        ) => Act::CopyRight,
        (Mode::ToLeft | Mode::BothWays, Kind::OnlyRight | Kind::NewerRight) => Act::CopyLeft,
        (Mode::MirrorToRight, Kind::OnlyRight) => Act::DeleteRight,
        // Mirror: the left side wins whatever the dates.
        (Mode::MirrorToRight, Kind::NewerRight | Kind::Conflict) => Act::CopyRight,
        _ => Act::Skip,
    }
}

type Listed = Vec<(String, Info)>;

fn list(dir: &Path) -> Result<Listed, String> {
    if remote::is_remote(dir) {
        return Ok(remote::list_fresh(dir)?
            .into_iter()
            .map(|e| {
                (
                    e.name.to_string(),
                    Info {
                        is_dir: e.is_dir,
                        size: e.size,
                        modified: e.modified,
                        cloud: false,
                    },
                )
            })
            .collect());
    }
    let rd = std::fs::read_dir(dir).map_err(|e| format!("Couldn't read {}: {e}", dir.display()))?;
    Ok(rd
        .flatten()
        .filter_map(|d| crate::fsys::listing::entry(&d))
        .filter(|e| !e.is_link || e.is_dir)
        .map(|e| {
            (
                e.name.to_string(),
                Info {
                    is_dir: e.is_dir,
                    size: e.size,
                    modified: e.modified,
                    cloud: e.not_local(),
                },
            )
        })
        .collect())
}

pub(super) fn join(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, s| p.join(s))
}

fn same_contents(a: &Path, b: &Path, stop: &AtomicBool) -> Option<bool> {
    if remote::is_remote(a) || remote::is_remote(b) {
        return None;
    }
    let n = AtomicU64::new(0);
    Some(crate::actions::sha256(a, &n, stop).ok()? == crate::actions::sha256(b, &n, stop).ok()?)
}

/// Compare `left` and `right`, into subfolders when `deep`.
pub(super) fn compare(
    left: &Path,
    right: &Path,
    deep: bool,
    contents: bool,
    ignore_times: bool,
    stop: &AtomicBool,
    seen: &AtomicU64,
) -> Result<Vec<Diff>, String> {
    let mut out = Vec::new();
    let mut todo = vec![String::new()];
    while let Some(rel) = todo.pop() {
        if stop.load(Ordering::Relaxed) {
            return Err("Stopped.".into());
        }
        let (l, r) = (join(left, &rel), join(right, &rel));
        let (l, r) = if rel.is_empty() {
            (list(left)?, list(right)?)
        } else {
            (list(&l).unwrap_or_default(), list(&r).unwrap_or_default())
        };
        seen.fetch_add((l.len() + r.len()) as u64, Ordering::Relaxed);
        let mut both: BTreeMap<String, (Option<Info>, Option<Info>)> = BTreeMap::new();
        for (n, i) in l {
            both.entry(n).or_default().0 = Some(i);
        }
        for (n, i) in r {
            both.entry(n).or_default().1 = Some(i);
        }
        for (name, (a, b)) in both {
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            let kind = match (a, b) {
                (Some(_), None) => Kind::OnlyLeft,
                (None, Some(_)) => Kind::OnlyRight,
                (Some(a), Some(b)) if a.is_dir && b.is_dir => {
                    if deep {
                        todo.push(path);
                    }
                    continue;
                }
                (Some(a), Some(b)) if a.is_dir != b.is_dir => Kind::Conflict,
                (Some(a), Some(b)) => {
                    let close = (a.modified - b.modified).abs() <= SLACK;
                    if a.size == b.size && (close || ignore_times) {
                        // Files only in the cloud are judged by size and date alone.
                        let differ = contents
                            && !a.cloud
                            && !b.cloud
                            && same_contents(&join(left, &path), &join(right, &path), stop)
                                == Some(false);
                        if differ { Kind::Conflict } else { Kind::Same }
                    } else if close {
                        Kind::Conflict
                    } else if a.modified > b.modified {
                        Kind::NewerLeft
                    } else {
                        Kind::NewerRight
                    }
                }
                (None, None) => continue,
            };
            out.push(Diff {
                rel: path,
                kind,
                left: a,
                right: b,
            });
        }
    }
    out.sort_by_cached_key(|a| a.rel.to_lowercase());
    Ok(out)
}

pub(super) struct Compare {
    pub left: PathBuf,
    pub right: PathBuf,
    pub deep: bool,
    pub contents: bool,
    /// Same size counts as the same, whatever the times (servers that can't set times).
    pub ignore_times: bool,
    /// Items the user took out of the sync plan.
    off: std::collections::HashSet<String>,
    job: Option<(
        mpsc::Receiver<Result<Vec<Diff>, String>>,
        Arc<AtomicBool>,
        Arc<AtomicU64>,
    )>,
    diffs: Option<Result<Vec<Diff>, String>>,
    show: [bool; 6],
    mode: Mode,
    /// Mirror: the list of deletions is on screen, waiting for a second yes.
    confirm: bool,
}

impl Compare {
    pub fn new(left: PathBuf, right: PathBuf) -> Self {
        let mut c = Compare {
            left,
            right,
            deep: true,
            contents: false,
            ignore_times: false,
            off: Default::default(),
            job: None,
            diffs: None,
            show: [true, true, true, true, true, false],
            mode: Mode::ToRight,
            confirm: false,
        };
        c.start();
        c
    }

    fn start(&mut self) {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicU64::new(0));
        let ignore_times = self.ignore_times;
        let (l, r, deep, contents, st, se) = (
            self.left.clone(),
            self.right.clone(),
            self.deep,
            self.contents,
            stop.clone(),
            seen.clone(),
        );
        std::thread::spawn(move || {
            let _ = tx.send(compare(&l, &r, deep, contents, ignore_times, &st, &se));
        });
        if let Some((_, s, _)) = self.job.replace((rx, stop, seen)) {
            s.store(true, Ordering::Relaxed);
        }
        self.diffs = None;
        self.confirm = false;
        self.off.clear();
    }
}

impl Drop for Compare {
    fn drop(&mut self) {
        if let Some((_, s, _)) = &self.job {
            s.store(true, Ordering::Relaxed);
        }
    }
}

fn fmt_info(i: Option<Info>) -> String {
    match i {
        None => String::new(),
        Some(i) if i.is_dir => "folder".into(),
        Some(i) => {
            let t = chrono::DateTime::from_timestamp(i.modified, 0)
                .map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            format!("{}  {t}", crate::scan::human(i.size))
        }
    }
}

impl App {
    /// Compare the two panes' folders.
    pub(super) fn start_compare(&mut self) {
        if !self.premium(crate::licence::Feature::FolderCompare) {
            return;
        }
        let dirs: Vec<PathBuf> = self
            .files
            .panes
            .iter()
            .filter_map(|p| p.tab().here().dir().map(Path::to_path_buf))
            .collect();
        match dirs.as_slice() {
            [l, r] if l != r => self.files.compare = Some(Compare::new(l.clone(), r.clone())),
            [_, _] => {
                self.ops.notice = Some((
                    "Both panes show the same folder: open the other folder in one of them first."
                        .into(),
                    false,
                ))
            }
            _ => {
                self.ops.notice = Some((
                    "Open a second pane first (View > Side by Side), with the other folder in it."
                        .into(),
                    false,
                ))
            }
        }
    }

    pub(super) fn compare_window(&mut self, ctx: &egui::Context) {
        let Some(c) = &mut self.files.compare else {
            return;
        };
        if let Some((rx, _, _)) = &c.job {
            match rx.try_recv() {
                Ok(r) => {
                    c.diffs = Some(r);
                    c.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(150))
                }
                Err(_) => c.job = None,
            }
        }
        let mut close = false;
        let mut sync: Option<Vec<(Act, String)>> = None;
        let mut pick: Option<Vec<(Kind, String)>> = None;
        let screen = ctx.content_rect();
        egui::Modal::new(egui::Id::new("compare")).show(ctx, |ui| {
            ui.set_width((screen.width() - 80.0).clamp(400.0, 1100.0));
            ui.horizontal(|ui| {
                ui.heading("Compare Folders");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = ui.button("Close").clicked();
                });
            });
            egui::Grid::new("cmp-sides").num_columns(2).show(ui, |ui| {
                ui.colored_label(Kind::OnlyLeft.colour(), "Left");
                ui.label(c.left.shown());
                ui.end_row();
                ui.colored_label(Kind::OnlyRight.colour(), "Right");
                ui.label(c.right.shown());
                ui.end_row();
            });
            ui.horizontal(|ui| {
                let mut again = ui.checkbox(&mut c.deep, "Include subfolders").changed();
                again |= ui.checkbox(&mut c.ignore_times, "Ignore times").on_hover_text("Files of the same size count as the same. For servers that can't set a file's time, where every copy looks newer").changed();
                again |= ui.checkbox(&mut c.contents, "Compare contents").on_hover_text("Also read files with the same size and date, to be sure they match. Slower; local folders only. Files that are only in the cloud aren't read (that would download them)").changed();
                if ui.button("Compare Again").clicked() || again {
                    c.start();
                }
                if let Some((_, stop, seen)) = &c.job {
                    ui.spinner();
                    ui.weak(format!("{} items read…", seen.load(Ordering::Relaxed)));
                    if ui.button("Stop").clicked() {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
            });
            ui.separator();
            let diffs = match &c.diffs {
                Some(Ok(d)) => d,
                Some(Err(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                    return;
                }
                None => return,
            };
            ui.horizontal_wrapped(|ui| {
                for (n, k) in Kind::ALL.iter().enumerate() {
                    let count = diffs.iter().filter(|d| d.kind == *k).count();
                    let text = egui::RichText::new(format!("{} {count}", k.title())).color(k.colour());
                    if ui.add(egui::Button::selectable(c.show[n], text)).on_hover_text("Show or hide these").clicked() {
                        c.show[n] = !c.show[n];
                    }
                }
            });
            let shown: Vec<&Diff> = diffs.iter().filter(|d| c.show[Kind::ALL.iter().position(|k| *k == d.kind).unwrap()]).collect();
            if diffs.iter().all(|d| d.kind == Kind::Same) {
                ui.label(format!("The folders match ({} items).", diffs.len()));
            }
            let row_h = 20.0;
            let mode = c.mode;
            let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
            let cols = columns(head.left(), head.width());
            for (i, t) in [(2, "Name"), (3, "Left"), (4, "Right")] {
                ui.painter().text(egui::pos2(cols[i], head.center().y), egui::Align2::LEFT_CENTER, t, egui::FontId::proportional(11.0), ui.visuals().weak_text_color());
            }
            egui::ScrollArea::vertical().max_height((screen.height() - 360.0).max(160.0)).auto_shrink([false, true]).show_rows(ui, row_h, shown.len(), |ui, range| {
                for d in &shown[range] {
                    let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::click());
                    let act = action(mode, d.kind);
                    let off = act != Act::Skip && c.off.contains(&d.rel);
                    if act != Act::Skip && resp.clicked() {
                        if off {
                            c.off.remove(&d.rel);
                        } else {
                            c.off.insert(d.rel.clone());
                        }
                    }
                    if resp.hovered() && act != Act::Skip {
                        ui.painter().rect_filled(row, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                    }
                    let (arrow, tip) = if off { ("☐", "Left out of the sync (click to put it back)") } else { match act {
                        Act::CopyRight => ("➡", "Copied to the right"),
                        Act::CopyLeft => ("⬅", "Copied to the left"),
                        Act::DeleteRight => ("✖", "Deleted on the right (to the bin)"),
                        Act::Skip => ("", "Left as it is"),
                    } };
                    let cols = columns(row.left(), row.width());
                    let p = ui.painter_at(row);
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let (text, weak) = (ui.visuals().text_color(), ui.visuals().weak_text_color());
                    let y = row.center().y;
                    let put = |i: usize, s: &str, c: egui::Color32| {
                        let clip = egui::Rect::from_x_y_ranges(cols[i]..=cols[i + 1] - 8.0, row.y_range());
                        p.with_clip_rect(clip).text(egui::pos2(cols[i], y), egui::Align2::LEFT_CENTER, s, font.clone(), c);
                    };
                    put(0, d.kind.title(), d.kind.colour());
                    put(1, arrow, text);
                    put(2, &d.rel, if off { weak } else { text });
                    put(3, &fmt_info(d.left), weak);
                    put(4, &fmt_info(d.right), weak);
                    let tip = if act != Act::Skip && !off { format!("{}: {tip} (click to leave it out)", d.rel) } else { format!("{}: {tip}", d.rel) };
                    resp.on_hover_text(tip);
                }
            });
            ui.separator();
            let plan: Vec<(Act, String)> = diffs.iter().filter(|d| !c.off.contains(&d.rel)).map(|d| (action(c.mode, d.kind), d.rel.clone())).filter(|(a, _)| *a != Act::Skip).collect();
            let count = |a: Act| plan.iter().filter(|(x, _)| *x == a).count();
            let (to_r, to_l, del) = (count(Act::CopyRight), count(Act::CopyLeft), count(Act::DeleteRight));
            let skipped = diffs.iter().filter(|d| d.kind != Kind::Same && (action(c.mode, d.kind) == Act::Skip || c.off.contains(&d.rel))).count();
            if c.confirm {
                ui.colored_label(ui.visuals().warn_fg_color, format!("Mirror will delete {del} item{} on the right (to the {}):", if del == 1 { "" } else { "s" }, crate::actions::BIN));
                egui::ScrollArea::vertical().id_salt("cmp-del").max_height(120.0).show(ui, |ui| {
                    for (_, rel) in plan.iter().filter(|(a, _)| *a == Act::DeleteRight) {
                        ui.label(rel);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(format!("Delete {del} and Sync")).clicked() {
                        sync = Some(plan.clone());
                    }
                    if ui.button("Back").clicked() {
                        c.confirm = false;
                    }
                });
                return;
            }
            ui.horizontal(|ui| {
                ui.label("Sync");
                egui::ComboBox::from_id_salt("cmp-mode").selected_text(c.mode.title()).width(320.0).show_ui(ui, |ui| {
                    for m in Mode::ALL {
                        ui.selectable_value(&mut c.mode, m, m.title());
                    }
                });
                let mut what = Vec::new();
                if to_r > 0 {
                    what.push(format!("{to_r} copied to the right"));
                }
                if to_l > 0 {
                    what.push(format!("{to_l} copied to the left"));
                }
                if del > 0 {
                    what.push(format!("{del} deleted"));
                }
                if skipped > 0 {
                    what.push(format!("{skipped} left alone"));
                }
                ui.weak(if what.is_empty() { "Nothing to do".to_string() } else { what.join(" · ") });
                let any = to_r + to_l + del > 0;
                if ui.add_enabled(any && c.job.is_none(), egui::Button::new(if del > 0 { "Review Deletions…" } else { "Sync" })).clicked() {
                    if del > 0 {
                        c.confirm = true;
                    } else {
                        sync = Some(plan.clone());
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.weak("Click an item to leave it out. Replaced and deleted items go to the bin (on a server, they're deleted), and Undo (Ctrl+Z) reverses a sync.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Select in Panes").on_hover_text("Close this and select what differs in both panes (the top-level item of each difference)").clicked() {
                        pick = Some(diffs.iter().filter(|d| d.kind != Kind::Same).map(|d| (d.kind, d.rel.split('/').next().unwrap_or("").to_string())).collect());
                    }
                });
            });
        });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.files.compare = None;
            return;
        }
        if let Some(pick) = pick {
            let (l, r) = (c.left.clone(), c.right.clone());
            self.files.compare = None;
            self.select_differences(&l, &r, &pick);
            return;
        }
        if let Some(plan) = sync {
            let (l, r) = (c.left.clone(), c.right.clone());
            self.files.compare = None;
            self.run_sync(l, r, plan);
        }
    }

    /// Select each difference's top-level item in the pane showing its side.
    fn select_differences(&mut self, left: &Path, right: &Path, pick: &[(Kind, String)]) {
        let mut n = 0;
        for p in &mut self.files.panes {
            let Some(dir) = p.tab().here().dir().map(Path::to_path_buf) else { continue };
            let side_left = dir == left;
            if !side_left && dir != right {
                continue;
            }
            let names: std::collections::HashSet<Box<str>> = pick
                .iter()
                .filter(|(k, _)| match k {
                    Kind::OnlyLeft => side_left,
                    Kind::OnlyRight => !side_left,
                    _ => true,
                })
                .map(|(_, name)| name.as_str().into())
                .collect();
            n += names.len();
            let t = p.tab_mut();
            t.selected = names.into_iter().collect();
        }
        self.ops.notice = Some((format!("Selected {n} item{} that differ.", if n == 1 { "" } else { "s" }), false));
    }

    fn run_sync(&mut self, left: PathBuf, right: PathBuf, plan: Vec<(Act, String)>) {
        // With a server on either side: the transfer queue, in the background.
        if remote::is_remote(&left) || remote::is_remote(&right) {
            self.queue_sync(left, right, plan);
            return;
        }
        if self.ops.job.is_some() {
            self.ops.notice = Some(("Another file operation is still running.".into(), true));
            return;
        }
        let rec = self.ops.record();
        let ctl = Arc::new(crate::transfer::Ctl::default());
        let c2 = ctl.clone();
        self.run_op(format!("Syncing {} items", plan.len()), move || {
            let out = sync_items(&left, &right, &plan, false, &c2);
            let (steps, touched, errors, done) = (out.steps, out.touched, out.errors, out.made.len());
            *rec.lock().unwrap() = steps;
            let r = if errors.is_empty() {
                Ok(Some(format!(
                    "Synced {done} item{}.",
                    if done == 1 { "" } else { "s" }
                )))
            } else {
                Err(errors.join(" "))
            };
            (r, touched)
        });
        if let Some(j) = &mut self.ops.job {
            j.ctl = Some(ctl);
        }
    }
}

/// Carry out a sync plan between `left` and `right`: copies replace what's there (to the bin
/// on this computer), Mirror deletes. One job: the totals cover the whole plan.
/// `retry`: a second go after a failure, where files already copied (same size, times within
/// two seconds) are left alone.
pub(super) fn sync_items(left: &Path, right: &Path, plan: &[(Act, String)], retry: bool, ctl: &crate::transfer::Ctl) -> crate::transfer::Outcome {
    ctl.items_total.store(plan.len() as u64, Ordering::Relaxed);
    ctl.counting.store(true, Ordering::Relaxed);
    let total: u64 = plan
        .iter()
        .map(|(act, rel)| match act {
            Act::CopyRight => crate::transfer::bytes_in(&join(left, rel), ctl),
            Act::CopyLeft => crate::transfer::bytes_in(&join(right, rel), ctl),
            _ => 0,
        })
        .sum();
    ctl.bytes_total.store(total, Ordering::Relaxed);
    ctl.counting.store(false, Ordering::Relaxed);
    let mut steps = Vec::new();
    let mut touched = Vec::new();
    let mut errors = Vec::new();
    let mut made = Vec::new();
    let mut skipped = 0;
    for (act, rel) in plan {
        if !ctl.go_on() {
            break;
        }
        let (src, dest) = match act {
            Act::CopyRight => (join(left, rel), join(right, rel)),
            Act::CopyLeft => (join(right, rel), join(left, rel)),
            Act::DeleteRight => {
                let p = join(right, rel);
                let r = if remote::is_remote(&p) {
                    remote::delete(&p)
                        .map(|_| crate::transfer::Step::Deleted(p.clone()))
                        .map_err(|e| remote::describe(&e))
                } else {
                    crate::actions::trash_keep(&p).map(|receipt| {
                        crate::transfer::Step::Trashed {
                            original: p.clone(),
                            receipt,
                        }
                    })
                };
                match r {
                    Ok(s) => {
                        steps.push(s);
                        made.push(p.clone());
                    }
                    Err(e) => errors.push(format!("Couldn't delete {}: {e}", p.shown())),
                }
                touched.push(p);
                continue;
            }
            Act::Skip => continue,
        };
        let Some(dir) = dest.parent().map(Path::to_path_buf) else {
            continue;
        };
        if retry && already_there(&src, &dest) {
            skipped += 1;
            continue;
        }
        // Nothing is sent to a server through a link that leads out of the compared folder.
        if remote::is_remote(&dest) && !remote::is_remote(&src) {
            let top = if matches!(act, Act::CopyRight) { left } else { right };
            if let (Ok(t), Ok(s)) = (std::fs::canonicalize(top), std::fs::canonicalize(&src))
                && !s.starts_with(&t)
            {
                errors.push(format!("Skipped {}: it's a link to something outside {}", src.shown(), top.shown()));
                continue;
            }
        }
        let out = crate::transfer::transfer_with(
            std::slice::from_ref(&src),
            &dir,
            true,
            &mut |_| crate::transfer::Choice::Replace,
            ctl,
        );
        steps.extend(out.steps);
        touched.extend(out.touched);
        errors.extend(out.errors);
        made.extend(out.made);
        skipped += out.skipped;
    }
    crate::transfer::Outcome { made, touched, steps, errors, skipped, stopped: ctl.stop.load(Ordering::Relaxed) }
}

/// Both are files of the same size with times within two seconds: copied on an earlier try.
fn already_there(a: &Path, b: &Path) -> bool {
    let info = |p: &Path| -> Option<(bool, u64, i64)> {
        if remote::is_remote(p) {
            remote::meta(p).ok().map(|m| (m.is_dir, m.len, m.modified))
        } else {
            let m = std::fs::metadata(p).ok()?;
            let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
            Some((m.is_dir(), m.len(), t))
        }
    };
    matches!((info(a), info(b)), (Some((false, la, ta)), Some((false, lb, tb))) if la == lb && (ta - tb).abs() <= SLACK)
}

/// Where the list's columns start: what differs, the action, the name, the left and right
/// sides; and its right edge.
fn columns(x: f32, w: f32) -> [f32; 6] {
    let side = ((w - 130.0) * 0.26).max(130.0);
    [x, x + 92.0, x + 116.0, x + w - side * 2.0, x + w - side, x + w]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn set_time(p: &Path, secs: i64) {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn finds_differences_and_plans() {
        let d = std::env::temp_dir().join(format!("dirmap-cmp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let (l, r) = (d.join("l"), d.join("r"));
        for s in [&l, &r] {
            fs::create_dir_all(s.join("sub")).unwrap();
            fs::write(s.join("same.txt"), "same").unwrap();
            set_time(&s.join("same.txt"), 1_000_000);
        }
        fs::write(l.join("only.txt"), "l").unwrap();
        fs::create_dir_all(r.join("rdir/deep")).unwrap();
        fs::write(l.join("sub/new.txt"), "newer").unwrap();
        fs::write(r.join("sub/new.txt"), "old").unwrap();
        set_time(&l.join("sub/new.txt"), 2_000_000);
        set_time(&r.join("sub/new.txt"), 1_000_000);
        fs::write(l.join("clash.txt"), "aaaa").unwrap();
        fs::write(r.join("clash.txt"), "bb").unwrap();
        set_time(&l.join("clash.txt"), 1_000_000);
        set_time(&r.join("clash.txt"), 1_000_001);
        // Same size and date, different bytes: only Compare contents sees it.
        fs::write(l.join("sneaky.txt"), "abc").unwrap();
        fs::write(r.join("sneaky.txt"), "xyz").unwrap();
        set_time(&l.join("sneaky.txt"), 1_000_000);
        set_time(&r.join("sneaky.txt"), 1_000_000);
        let (stop, seen) = (AtomicBool::new(false), AtomicU64::new(0));
        let got = compare(&l, &r, true, false, false, &stop, &seen).unwrap();
        let kinds: Vec<(&str, Kind)> = got.iter().map(|d| (d.rel.as_str(), d.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("clash.txt", Kind::Conflict),
                ("only.txt", Kind::OnlyLeft),
                ("rdir", Kind::OnlyRight),
                ("same.txt", Kind::Same),
                ("sneaky.txt", Kind::Same),
                ("sub/new.txt", Kind::NewerLeft)
            ]
        );
        let got = compare(&l, &r, true, true, false, &stop, &seen).unwrap();
        assert_eq!(
            got.iter().find(|d| d.rel == "sneaky.txt").unwrap().kind,
            Kind::Conflict
        );
        let shallow = compare(&l, &r, false, false, false, &stop, &seen).unwrap();
        assert!(shallow.iter().all(|d| !d.rel.contains('/')));
        // Plans.
        assert_eq!(action(Mode::ToRight, Kind::OnlyRight), Act::Skip);
        assert_eq!(action(Mode::BothWays, Kind::Conflict), Act::Skip);
        assert_eq!(action(Mode::BothWays, Kind::OnlyRight), Act::CopyLeft);
        assert_eq!(
            action(Mode::MirrorToRight, Kind::OnlyRight),
            Act::DeleteRight
        );
        assert_eq!(
            action(Mode::MirrorToRight, Kind::NewerRight),
            Act::CopyRight
        );
        assert_eq!(action(Mode::ToLeft, Kind::NewerLeft), Act::Skip);
        let _ = fs::remove_dir_all(&d);
    }
}
