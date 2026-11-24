//! Colours, tags and notes in the Files workspace: the marks drawn beside names, the Colour
//! menu, the Tags and Note box, and the Tags section of the sidebar.

use super::App;
use super::files::FAction;
use crate::fsys::history::Location;
use crate::fsys::marks::{self, COLOURS, Mark};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, vec2};
use std::path::PathBuf;

/// The box for editing tags and a note, for one or more items.
pub(super) struct MarkEdit {
    pub paths: Vec<PathBuf>,
    pub tags: String,
    pub note: String,
    /// The tags all the items had when the box opened (what the tags field replaces).
    pub was: Vec<String>,
    pub focus: bool,
}

pub(super) fn colour(i: u8) -> Color32 {
    let [r, g, b] = COLOURS[(i as usize).min(COLOURS.len() - 1)].1;
    Color32::from_rgb(r, g, b)
}

/// Draw an item's colour as a dot on its icon.
pub(super) fn paint_dot(p: &egui::Painter, icon: Rect, m: &Mark, bg: Color32) {
    if let Some(c) = m.colour {
        let at = icon.right_bottom() - vec2(1.0, 1.0);
        let r = (icon.width() * 0.22).clamp(3.5, 9.0);
        p.circle(at, r, colour(c), Stroke::new(1.5, bg));
    }
}

/// The tags (and a note marker) after a name; returns the width used.
pub(super) fn paint_tags(p: &egui::Painter, at: Pos2, m: &Mark, weak: Color32) -> f32 {
    let mut x = at.x;
    let font = FontId::proportional(11.0);
    for t in &m.tags {
        let g = p.layout_no_wrap(format!("#{t}"), font.clone(), weak);
        let r = Rect::from_min_size(
            Pos2::new(x, at.y - g.size().y / 2.0 - 1.0),
            g.size() + vec2(8.0, 2.0),
        );
        p.rect_stroke(
            r,
            6.0,
            Stroke::new(1.0, weak.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
        p.galley(r.min + vec2(4.0, 1.0), g, weak);
        x = r.right() + 4.0;
    }
    if !m.note.trim().is_empty() {
        let g = p.text(Pos2::new(x, at.y), Align2::LEFT_CENTER, "📝", font, weak);
        x = g.right() + 4.0;
    }
    x - at.x
}

/// The Colour submenu and Tags and Note item, for the right-click menu.
pub(super) fn menu(ui: &mut egui::Ui, current: Option<u8>) -> Option<FAction> {
    let mut out = None;
    ui.menu_button(crate::app::style::mi("Colour"), |ui| {
        for (i, (name, _)) in COLOURS.iter().enumerate() {
            let i = i as u8;
            let (r, resp) = ui.allocate_exact_size(vec2(150.0, 22.0), egui::Sense::click());
            if resp.hovered() {
                ui.painter()
                    .rect_filled(r, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
            }
            ui.painter()
                .circle_filled(r.left_center() + vec2(12.0, 0.0), 6.0, colour(i));
            let text = if current == Some(i) {
                format!("{name}  ✔")
            } else {
                name.to_string()
            };
            ui.painter().text(
                r.left_center() + vec2(26.0, 0.0),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(13.0),
                ui.visuals().text_color(),
            );
            if resp.clicked() {
                out = Some(FAction::Colour(Some(i)));
                ui.close();
            }
        }
        ui.separator();
        if ui
            .add_enabled(current.is_some(), egui::Button::new(crate::app::style::mi("No Colour")))
            .clicked()
        {
            out = Some(FAction::Colour(None));
        }
    });
    if ui.add(egui::Button::new(crate::app::style::mi("Tags and Note…"))).clicked() {
        out = Some(FAction::Tags);
    }
    out
}

impl App {
    pub(super) fn open_mark_edit(&mut self, paths: Vec<PathBuf>) {
        let Some(first) = paths.first() else { return };
        let m = self.files.marks.get(first).cloned().unwrap_or_default();
        // Several items: start from what they have in common.
        let common: Vec<String> = m
            .tags
            .iter()
            .filter(|t| {
                paths
                    .iter()
                    .all(|p| self.files.marks.get(p).is_some_and(|k| k.tags.contains(t)))
            })
            .cloned()
            .collect();
        let note = if paths.len() == 1 {
            m.note
        } else {
            String::new()
        };
        self.files.mark_edit = Some(MarkEdit {
            paths,
            tags: common.join(", "),
            note,
            was: common,
            focus: true,
        });
    }

    pub(super) fn set_colour(&mut self, paths: &[PathBuf], c: Option<u8>) {
        for p in paths {
            self.files.marks.edit(p, |m| m.colour = c);
        }
    }

    pub(super) fn mark_edit_window(&mut self, ctx: &egui::Context) {
        let Some(e) = &mut self.files.mark_edit else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let n = e.paths.len();
        let all_tags = self.files.marks.tags();
        egui::Modal::new(egui::Id::new("mark-edit")).show(ctx, |ui| {
            ui.set_width(420.0);
            let what = if n == 1 { e.paths[0].file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() } else { format!("{n} items") };
            ui.heading(format!("Tags and Note: {what}"));
            ui.add_space(6.0);
            ui.label("Tags, separated by commas");
            let r = ui.add(egui::TextEdit::singleline(&mut e.tags).desired_width(400.0).hint_text("client, final, to review"));
            if e.focus {
                r.request_focus();
                e.focus = false;
            }
            let have = marks::parse_tags(&e.tags);
            let unused: Vec<&String> = all_tags.iter().filter(|t| !have.iter().any(|h| h.eq_ignore_ascii_case(t))).collect();
            if !unused.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.weak("Add:");
                    for t in unused.into_iter().take(16) {
                        if ui.small_button(format!("#{t}")).clicked() {
                            if !e.tags.trim().is_empty() {
                                e.tags.push_str(", ");
                            }
                            e.tags.push_str(t);
                        }
                    }
                });
            }
            ui.add_space(6.0);
            if n == 1 {
                ui.label("Note");
                ui.add(egui::TextEdit::multiline(&mut e.note).desired_width(400.0).desired_rows(4));
            } else {
                ui.weak("These tags replace the ones the items share. Notes are kept.");
            }
            ui.add_space(4.0);
            ui.weak("Kept by Filescape, not in the files: renaming or moving them in another program loses them.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                save = ui.button("Save").clicked();
                cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if cancel {
            self.files.mark_edit = None;
        } else if save {
            let e = self.files.mark_edit.take().unwrap();
            let tags = marks::parse_tags(&e.tags);
            let single = e.paths.len() == 1;
            for p in &e.paths {
                let (tags, note, was) = (tags.clone(), e.note.clone(), e.was.clone());
                self.files.marks.edit(p, |m| {
                    if single {
                        m.tags = tags;
                        m.note = note;
                    } else {
                        // The shared tags were what was edited; others stay.
                        m.tags.retain(|x| {
                            !was.iter().chain(&tags).any(|y| y.eq_ignore_ascii_case(x))
                        });
                        m.tags.extend(tags);
                    }
                });
            }
        }
    }

    /// The sidebar's Tags section: each tag lists its items.
    pub(super) fn tags_section(&mut self, ui: &mut egui::Ui) -> Option<(PathBuf, String)> {
        let tags = self.files.marks.tags();
        if tags.is_empty() {
            return None;
        }
        let mut pick = None;
        super::files::side_section(ui, "fm-tags", "Tags", true, |ui| {
            for t in tags {
                let items = self.files.marks.tagged(&t);
                let resp = super::files::side_row(ui, "#", &t, false, super::files::ROW_F)
                    .on_hover_text(format!(
                        "{} item{} tagged {t}",
                        items.len(),
                        if items.len() == 1 { "" } else { "s" }
                    ));
                if resp.clicked()
                    && let Some(dir) = marks::common_folder(&items)
                {
                    pick = Some((dir, t.clone()));
                }
            }
        });
        pick
    }

    /// Show what's tagged `tag`, from `dir` down.
    pub(super) fn show_tag(&mut self, dir: PathBuf, tag: String) {
        self.files.go(Location::Dir(dir));
        let p = self.files.pane_mut();
        p.filter = format!("#{tag}");
        p.deep = true;
    }

    /// Changes made in dirmap carry marks along.
    pub(super) fn marks_follow(&mut self, steps: &[crate::transfer::Step]) {
        if self.files.marks.is_empty() {
            return;
        }
        for s in steps {
            if let crate::transfer::Step::Moved { from, to } = s {
                self.files.marks.moved(from, to);
            }
        }
    }
}
