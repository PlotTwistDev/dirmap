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
        return Ok(remote::list(dir)?
            .into_iter()
            .map(|e| {
                (
                    e.name.to_string(),
                    Info {
                        is_dir: e.is_dir,
                        size: e.size,
                        modified: e.modified,
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
                    if a.size == b.size && close {
                        let differ = contents
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
        let (l, r, deep, contents, st, se) = (
            self.left.clone(),
            self.right.clone(),
            self.deep,
            self.contents,
            stop.clone(),
            seen.clone(),
        );
        std::thread::spawn(move || {
            let _ = tx.send(compare(&l, &r, deep, contents, &st, &se));
        });
        if let Some((_, s, _)) = self.job.replace((rx, stop, seen)) {
            s.store(true, Ordering::Relaxed);
        }
        self.diffs = None;
        self.confirm = false;
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
                again |= ui.checkbox(&mut c.contents, "Compare contents").on_hover_text("Also read files with the same size and date, to be sure they match. Slower; local folders only").changed();
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
                    let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::hover());
                    let (arrow, tip) = match action(mode, d.kind) {
                        Act::CopyRight => ("➡", "Copied to the right"),
                        Act::CopyLeft => ("⬅", "Copied to the left"),
                        Act::DeleteRight => ("✖", "Deleted on the right (to the bin)"),
                        Act::Skip => ("", "Left as it is"),
                    };
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
                    put(2, &d.rel, text);
                    put(3, &fmt_info(d.left), weak);
                    put(4, &fmt_info(d.right), weak);
                    resp.on_hover_text(format!("{}: {tip}", d.rel));
                }
            });
            ui.separator();
            let plan: Vec<(Act, String)> = diffs.iter().map(|d| (action(c.mode, d.kind), d.rel.clone())).filter(|(a, _)| *a != Act::Skip).collect();
            let count = |a: Act| plan.iter().filter(|(x, _)| *x == a).count();
            let (to_r, to_l, del) = (count(Act::CopyRight), count(Act::CopyLeft), count(Act::DeleteRight));
            let skipped = diffs.iter().filter(|d| d.kind != Kind::Same && action(c.mode, d.kind) == Act::Skip).count();
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
            ui.weak("Replaced and deleted items go to the bin, and Undo (Ctrl+Z) reverses a sync.");
        });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.files.compare = None;
            return;
        }
        if let Some(plan) = sync {
            let (l, r) = (c.left.clone(), c.right.clone());
            self.files.compare = None;
            self.run_sync(l, r, plan);
        }
    }

    fn run_sync(&mut self, left: PathBuf, right: PathBuf, plan: Vec<(Act, String)>) {
        if self.ops.job.is_some() {
            self.ops.notice = Some(("Another file operation is still running.".into(), true));
            return;
        }
        let rec = self.ops.record();
        let ctl = Arc::new(crate::transfer::Ctl::default());
        let c2 = ctl.clone();
        self.run_op(format!("Syncing {} items", plan.len()), move || {
            let mut steps = Vec::new();
            let mut touched = Vec::new();
            let mut errors = Vec::new();
            let mut done = 0;
            for (act, rel) in &plan {
                if !c2.go_on() {
                    break;
                }
                let (src, dest) = match act {
                    Act::CopyRight => (join(&left, rel), join(&right, rel)),
                    Act::CopyLeft => (join(&right, rel), join(&left, rel)),
                    Act::DeleteRight => {
                        let p = join(&right, rel);
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
                                done += 1;
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
                let out = crate::transfer::transfer_with(
                    std::slice::from_ref(&src),
                    &dir,
                    true,
                    &mut |_| crate::transfer::Choice::Replace,
                    &c2,
                );
                steps.extend(out.steps);
                touched.extend(out.touched);
                errors.extend(out.errors);
                done += out.made.len();
            }
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
        let got = compare(&l, &r, true, false, &stop, &seen).unwrap();
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
        let got = compare(&l, &r, true, true, &stop, &seen).unwrap();
        assert_eq!(
            got.iter().find(|d| d.rel == "sneaky.txt").unwrap().kind,
            Kind::Conflict
        );
        let shallow = compare(&l, &r, false, false, &stop, &seen).unwrap();
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
