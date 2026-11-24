//! Batch rename: many items in one folder at once, with find and replace (plain text or a
//! regular expression), a change of case, numbering and a new extension. Every new name is
//! shown before anything changes, with problems (a name used twice, one already taken, a
//! character that isn't allowed) marked; nothing is renamed while any remain.
//!
//! Names are changed in two passes, through temporary names, so items can swap names
//! (`a`→`b` and `b`→`a`). Undo puts every name back.

use super::App;
use crate::fsys::PathShow;
use crate::transfer::Step;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Case {
    Keep,
    Lower,
    Upper,
    Title,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Numbering {
    None,
    /// Number before the name: `001_name`.
    Before,
    /// Number after the name, before the extension: `name_001`.
    After,
    /// The name becomes the base text and the number: `shot_001`.
    Replace,
}

pub(super) struct BatchRename {
    pub items: Vec<(PathBuf, bool)>,
    /// Names in the folder that aren't being renamed (a new name mustn't take one).
    pub others: HashSet<String>,
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub match_case: bool,
    /// Find and replace work on the extension too.
    pub whole: bool,
    pub case: Case,
    pub numbering: Numbering,
    pub base: String,
    pub start: u64,
    pub step: u64,
    pub digits: u8,
    pub sep: String,
    pub change_ext: bool,
    pub ext: String,
    pub focus: bool,
}

/// One row of the preview: the old name, the new one, and what's wrong with it.
#[derive(Debug, PartialEq)]
pub(super) struct Row {
    pub old: String,
    pub new: String,
    pub problem: Option<String>,
}

impl BatchRename {
    pub fn new(items: Vec<(PathBuf, bool)>, others: HashSet<String>) -> Self {
        Self {
            items,
            others,
            find: String::new(),
            replace: String::new(),
            regex: false,
            match_case: false,
            whole: false,
            case: Case::Keep,
            numbering: Numbering::None,
            base: String::new(),
            start: 1,
            step: 1,
            digits: 3,
            sep: "_".into(),
            change_ext: false,
            ext: String::new(),
            focus: true,
        }
    }

    /// The new names. Err is a mistake in the find pattern.
    pub fn plan(&self) -> Result<Vec<Row>, String> {
        let re = if self.find.is_empty() {
            None
        } else {
            let pat = if self.regex {
                self.find.clone()
            } else {
                regex::escape(&self.find)
            };
            Some(
                regex::RegexBuilder::new(&pat)
                    .case_insensitive(!self.match_case)
                    .build()
                    .map_err(|e| {
                        format!(
                            "The pattern isn't valid: {}",
                            e.to_string().lines().last().unwrap_or("")
                        )
                    })?,
            )
        };
        let mut rows = Vec::with_capacity(self.items.len());
        for (k, (path, is_dir)) in self.items.iter().enumerate() {
            let old = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let (stem, ext) = split_ext(&old, *is_dir);
            let mut stem = stem.to_string();
            let mut ext = ext.to_string();
            if let Some(re) = &re {
                // Replacement text is literal unless it's a regular expression ($1 for groups).
                let with = if self.regex {
                    self.replace.clone()
                } else {
                    self.replace.replace('$', "$$")
                };
                if self.whole {
                    let all = format!("{stem}{ext}");
                    let done = re.replace_all(&all, with.as_str()).to_string();
                    let (s, e) = split_ext(&done, *is_dir);
                    stem = s.to_string();
                    ext = e.to_string();
                } else {
                    stem = re.replace_all(&stem, with.as_str()).to_string();
                }
            }
            stem = match self.case {
                Case::Keep => stem,
                Case::Lower => stem.to_lowercase(),
                Case::Upper => stem.to_uppercase(),
                Case::Title => title(&stem),
            };
            let n = self.start + k as u64 * self.step;
            let num = format!("{n:0w$}", w = self.digits as usize);
            stem = match self.numbering {
                Numbering::None => stem,
                Numbering::Before => format!("{num}{}{stem}", self.sep),
                Numbering::After => format!("{stem}{}{num}", self.sep),
                Numbering::Replace => format!("{}{num}", self.base),
            };
            if self.change_ext && !is_dir {
                let e = self.ext.trim().trim_start_matches('.');
                ext = if e.is_empty() {
                    String::new()
                } else {
                    format!(".{e}")
                };
            }
            let new = format!("{stem}{ext}");
            rows.push(Row {
                problem: bad_name(&new),
                old,
                new,
            });
        }
        // Names used twice, or taken by something not being renamed.
        let key = |s: &str| {
            if cfg!(target_os = "linux") {
                s.to_string()
            } else {
                s.to_lowercase()
            }
        };
        let mut count: HashMap<String, usize> = HashMap::new();
        for r in &rows {
            *count.entry(key(&r.new)).or_default() += 1;
        }
        let others: HashSet<String> = self.others.iter().map(|o| key(o)).collect();
        for r in &mut rows {
            if r.problem.is_some() || r.new == r.old {
                continue;
            }
            if count[&key(&r.new)] > 1 {
                r.problem = Some("Two items would get this name".into());
            } else if others.contains(&key(&r.new)) {
                r.problem = Some("Something else here already has this name".into());
            }
        }
        Ok(rows)
    }
}

/// `name.ext` into (`name`, `.ext`). Folders, and names starting with a dot, have no extension.
fn split_ext(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => name.split_at(i),
        _ => (name, ""),
    }
}

fn title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if start {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        start = !c.is_alphanumeric();
    }
    out
}

fn bad_name(n: &str) -> Option<String> {
    if n.trim().is_empty() || n == "." || n == ".." {
        Some("The name would be empty".into())
    } else if n.contains('/')
        || (cfg!(windows) && n.contains(['\\', ':', '*', '?', '"', '<', '>', '|']))
    {
        Some(if cfg!(windows) {
            "A name can't contain \\ / : * ? \" < > |".into()
        } else {
            "A name can't contain /".into()
        })
    } else if n != n.trim_end() || (cfg!(windows) && n.ends_with('.')) {
        Some("A name can't end with a space or a dot".into())
    } else {
        None
    }
}

/// Rename `(path, new name)` pairs through temporary names, so names can swap. On a failure
/// the ones already changed are put back. Returns the steps (for Undo) and the paths touched.
pub(super) fn apply(
    pairs: Vec<(PathBuf, String)>,
) -> (Result<Option<String>, String>, Vec<Step>, Vec<PathBuf>) {
    let mut steps = Vec::new();
    let mut touched = Vec::new();
    let tag = format!(".dirmap-rename-{}-", std::process::id());
    let mut temps = Vec::new();
    let undo_all = |steps: &[Step]| {
        crate::transfer::undo(steps);
    };
    for (i, (path, _)) in pairs.iter().enumerate() {
        let tmp = format!("{tag}{i}");
        match crate::actions::rename(path, &tmp) {
            Ok(t) => {
                steps.push(Step::Moved {
                    from: path.clone(),
                    to: t.clone(),
                });
                temps.push(t);
            }
            Err(e) => {
                undo_all(&steps);
                return (
                    Err(format!("Couldn't rename {}: {e}", path.shown())),
                    Vec::new(),
                    Vec::new(),
                );
            }
        }
    }
    for (t, (path, new)) in temps.iter().zip(&pairs) {
        match crate::actions::rename(t, new) {
            Ok(d) => {
                steps.push(Step::Moved {
                    from: t.clone(),
                    to: d.clone(),
                });
                touched.push(path.clone());
                touched.push(d);
            }
            Err(e) => {
                undo_all(&steps);
                return (
                    Err(format!(
                        "Couldn't rename {} to {new}: {e} Nothing was changed.",
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    )),
                    Vec::new(),
                    Vec::new(),
                );
            }
        }
    }
    let n = pairs.len();
    (
        Ok(Some(format!(
            "Renamed {n} item{}.",
            if n == 1 { "" } else { "s" }
        ))),
        steps,
        touched,
    )
}

impl App {
    /// Open the batch rename window on these items (all in one folder).
    pub(super) fn start_batch_rename(&mut self, paths: Vec<PathBuf>) {
        if !self.premium(crate::licence::Feature::BatchRename) {
            return;
        }
        let t = self.files.cur();
        let names: HashSet<String> = t
            .listing
            .as_ref()
            .map(|l| l.entries.iter().map(|e| e.name.to_string()).collect())
            .unwrap_or_default();
        let dirs: HashSet<String> = t
            .listing
            .as_ref()
            .map(|l| {
                l.entries
                    .iter()
                    .filter(|e| e.is_dir)
                    .map(|e| e.name.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let mine: HashSet<String> = paths
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .collect();
        let others = names.difference(&mine).cloned().collect();
        let items = paths
            .into_iter()
            .map(|p| {
                let d = p
                    .file_name()
                    .is_some_and(|n| dirs.contains(&*n.to_string_lossy()));
                (p, d)
            })
            .collect();
        self.ops.batch = Some(BatchRename::new(items, others));
    }

    pub(super) fn batch_rename_window(&mut self, ctx: &egui::Context) {
        let Some(b) = &mut self.ops.batch else { return };
        let plan = b.plan();
        let mut go = false;
        let mut cancel = false;
        let n = b.items.len();
        egui::Modal::new(egui::Id::new("batch-rename")).show(ctx, |ui| {
            ui.set_width(720.0f32.min(ctx.content_rect().width() - 60.0));
            ui.heading(format!("Rename {n} Items"));
            ui.add_space(6.0);
            egui::Grid::new("batch-grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Find");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut b.find)
                            .desired_width(260.0)
                            .hint_text("text to change (empty: none)"),
                    );
                    if b.focus {
                        r.request_focus();
                        b.focus = false;
                    }
                    ui.end_row();
                    ui.label("Replace with");
                    ui.add(
                        egui::TextEdit::singleline(&mut b.replace)
                            .desired_width(260.0)
                            .hint_text(if b.regex { "$1 is the first group" } else { "" }),
                    );
                    ui.end_row();
                    ui.label("");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut b.match_case, "Match case");
                        ui.checkbox(&mut b.regex, "Regular expression");
                        ui.checkbox(&mut b.whole, "Include extension");
                    });
                    ui.end_row();
                    ui.label("Case");
                    ui.horizontal(|ui| {
                        for (c, l) in [
                            (Case::Keep, "Keep"),
                            (Case::Lower, "lower"),
                            (Case::Upper, "UPPER"),
                            (Case::Title, "Title"),
                        ] {
                            ui.selectable_value(&mut b.case, c, l);
                        }
                    });
                    ui.end_row();
                    ui.label("Number");
                    ui.horizontal(|ui| {
                        for (c, l) in [
                            (Numbering::None, "None"),
                            (Numbering::Before, "Before Name"),
                            (Numbering::After, "After Name"),
                            (Numbering::Replace, "Replace Name"),
                        ] {
                            ui.selectable_value(&mut b.numbering, c, l);
                        }
                    });
                    ui.end_row();
                    if b.numbering != Numbering::None {
                        ui.label("");
                        ui.horizontal(|ui| {
                            if b.numbering == Numbering::Replace {
                                ui.add(
                                    egui::TextEdit::singleline(&mut b.base)
                                        .desired_width(120.0)
                                        .hint_text("name before the number"),
                                );
                            } else {
                                ui.label("Between");
                                ui.add(egui::TextEdit::singleline(&mut b.sep).desired_width(30.0));
                            }
                            ui.label("Start");
                            ui.add(egui::DragValue::new(&mut b.start).range(0..=999_999_999));
                            ui.label("Step");
                            ui.add(egui::DragValue::new(&mut b.step).range(1..=10_000));
                            ui.label("Digits");
                            ui.add(egui::DragValue::new(&mut b.digits).range(1..=10));
                        });
                        ui.end_row();
                    }
                    ui.label("Extension");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut b.change_ext, "Change to");
                        ui.add_enabled(
                            b.change_ext,
                            egui::TextEdit::singleline(&mut b.ext)
                                .desired_width(80.0)
                                .hint_text("none"),
                        );
                    });
                    ui.end_row();
                });
            ui.add_space(8.0);
            ui.separator();
            let mut ok = false;
            match &plan {
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                Ok(rows) => {
                    let changed = rows.iter().filter(|r| r.new != r.old).count();
                    let bad = rows.iter().filter(|r| r.problem.is_some()).count();
                    ok = changed > 0 && bad == 0;
                    let weak = ui.visuals().weak_text_color();
                    let err = ui.visuals().error_fg_color;
                    let row_h = 20.0;
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .auto_shrink([false, true])
                        .show_rows(ui, row_h, rows.len(), |ui, range| {
                            for r in &rows[range] {
                                let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::hover());
                                let p = ui.painter_at(row);
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                let half = (row.width() - 30.0) / 2.0;
                                let y = row.center().y;
                                let new_colour = if r.problem.is_some() {
                                    err
                                } else if r.new == r.old {
                                    weak
                                } else {
                                    ui.visuals().strong_text_color()
                                };
                                let clip = |x0: f32, x1: f32| egui::Rect::from_x_y_ranges(x0..=x1, row.y_range());
                                p.with_clip_rect(clip(row.left(), row.left() + half - 6.0)).text(egui::pos2(row.left(), y), egui::Align2::LEFT_CENTER, &r.old, font.clone(), weak);
                                p.text(egui::pos2(row.left() + half + 15.0, y), egui::Align2::CENTER_CENTER, "➡", font.clone(), weak);
                                let x = row.left() + half + 30.0;
                                p.with_clip_rect(clip(x, row.right())).text(egui::pos2(x, y), egui::Align2::LEFT_CENTER, &r.new, font, new_colour);
                                if let Some(why) = &r.problem {
                                    resp.on_hover_text(why);
                                }
                            }
                        });
                    ui.add_space(4.0);
                    if bad > 0 {
                        let first = rows.iter().find_map(|r| r.problem.as_deref()).unwrap_or("");
                        ui.colored_label(
                            err,
                            format!(
                                "{bad} name{} can't be used: {first}.",
                                if bad == 1 { "" } else { "s" }
                            ),
                        );
                    } else {
                        ui.weak(format!("{changed} of {n} will change."));
                    }
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                go = ui.add_enabled(ok, egui::Button::new("Rename")).clicked()
                    || (ok && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                cancel =
                    ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if cancel || go {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
        if cancel {
            self.ops.batch = None;
            return;
        }
        if !go {
            return;
        }
        let Ok(rows) = plan else { return };
        let Some(b) = self.ops.batch.take() else { return };
        let pairs: Vec<(PathBuf, String)> = b
            .items
            .iter()
            .zip(rows)
            .filter(|(_, r)| r.new != r.old)
            .map(|((p, _), r)| (p.clone(), r.new))
            .collect();
        let rec = self.ops.record();
        self.run_op(format!("Renaming {} items", pairs.len()), move || {
            let (r, steps, touched) = apply(pairs);
            *rec.lock().unwrap() = steps;
            (r, touched)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn br(names: &[&str]) -> BatchRename {
        BatchRename::new(
            names
                .iter()
                .map(|n| (PathBuf::from("/x").join(n), false))
                .collect(),
            ["taken.txt".to_string()].into(),
        )
    }

    fn news(b: &BatchRename) -> Vec<String> {
        b.plan().unwrap().into_iter().map(|r| r.new).collect()
    }

    #[test]
    fn find_replace_case_and_numbers() {
        let mut b = br(&["Shot_A.EXR", "shot_b.exr"]);
        b.find = "shot".into();
        b.replace = "plate".into();
        assert_eq!(news(&b), ["plate_A.EXR", "plate_b.exr"]);
        b.match_case = true;
        assert_eq!(news(&b), ["Shot_A.EXR", "plate_b.exr"]);
        b.match_case = false;
        b.case = Case::Upper;
        assert_eq!(news(&b), ["PLATE_A.EXR", "PLATE_B.exr"]);
        b.case = Case::Title;
        b.numbering = Numbering::After;
        b.start = 9;
        b.step = 2;
        assert_eq!(news(&b), ["Plate_A_009.EXR", "Plate_B_011.exr"]);
        b.numbering = Numbering::Replace;
        b.base = "bg_".into();
        b.change_ext = true;
        b.ext = ".png".into();
        assert_eq!(news(&b), ["bg_009.png", "bg_011.png"]);
    }

    #[test]
    fn regex_groups_and_literal_dollars() {
        let mut b = br(&["img_0012.jpg"]);
        b.find = r"img_(\d+)".into();
        b.replace = "photo-$1".into();
        b.regex = true;
        assert_eq!(news(&b), ["photo-0012.jpg"]);
        b.regex = false;
        b.find = "img".into();
        b.replace = "$1".into();
        assert_eq!(news(&b), ["$1_0012.jpg"]);
        b.regex = true;
        b.find = "(".into();
        assert!(b.plan().is_err());
    }

    #[test]
    fn problems_are_marked() {
        let mut b = br(&["a.txt", "b.txt", "c.txt"]);
        b.find = "[ab]".into();
        b.regex = true;
        b.replace = "x".into();
        let rows = b.plan().unwrap();
        assert!(rows[0].problem.as_deref().unwrap().contains("Two items"));
        assert!(rows[2].problem.is_none());
        b.find = "c".into();
        b.replace = "taken".into();
        assert!(
            b.plan().unwrap()[2]
                .problem
                .as_deref()
                .unwrap()
                .contains("already")
        );
        b.find = "^.*$".into();
        b.replace = "".into();
        b.whole = true;
        assert!(
            b.plan().unwrap()[0]
                .problem
                .as_deref()
                .unwrap()
                .contains("empty")
        );
    }

    #[test]
    fn names_swap_and_undo() {
        let d = std::env::temp_dir().join(format!("dirmap-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a"), "A").unwrap();
        std::fs::write(d.join("b"), "B").unwrap();
        let (r, steps, _) = apply(vec![(d.join("a"), "b".into()), (d.join("b"), "a".into())]);
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(std::fs::read_to_string(d.join("a")).unwrap(), "B");
        assert_eq!(std::fs::read_to_string(d.join("b")).unwrap(), "A");
        let (errs, _, _) = crate::transfer::undo(&steps);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(std::fs::read_to_string(d.join("a")).unwrap(), "A");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2);
        // A clash part-way leaves everything as it was.
        let (r, _, _) = apply(vec![
            (d.join("a"), "c".into()),
            (d.join("missing"), "d".into()),
        ]);
        assert!(r.is_err());
        assert!(d.join("a").exists() && !d.join("c").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
