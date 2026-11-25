//! The third-party licence notices (Help > About > Third-party licences), from
//! `packaging/third-party-licences.txt`, which `packaging/licences.py` writes from the crates the
//! program is built with.

use eframe::egui::{self, RichText};

/// The notices, as generated (with plain line ends, however the file was checked out).
fn text() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let mut t = include_str!("../../packaging/third-party-licences.txt").replace("\r\n", "\n");
        // The grammars and colour themes used for code previews carry licences of their own.
        let rule = "=".repeat(72);
        let ack = two_face::acknowledgement::listing();
        for (what, list) in [("syntax grammar", ack.for_syntaxes()), ("colour theme", ack.for_themes())] {
            for l in list {
                t.push_str(&format!("{rule}\nUsed by: the code previews' {what} {}\n{rule}\n{}\n\n", l.rel_path.display(), l.text.replace("\r\n", "\n").trim_end()));
            }
        }
        t
    })
}

/// Lines are cut to this many characters for display, so every row is one line high.
const COLS: usize = 88;

/// The notices window: a filter and the text.
#[derive(Default)]
pub struct Notices {
    pub open: bool,
    filter: String,
    /// The rows shown for `filter` (rebuilt when it changes).
    rows: Option<(String, Vec<String>)>,
}

impl Notices {
    pub fn window(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Third-party licences")
            .open(&mut open)
            .collapsible(false)
            .default_size([780.0, 560.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Filter");
                    ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("A crate or licence, e.g. egui or MIT").desired_width(260.0));
                    if ui.button("Copy all").on_hover_text("Copy every notice to the clipboard").clicked() {
                        ui.ctx().copy_text(text().to_string());
                    }
                });
                ui.add_space(4.0);
                if self.rows.as_ref().is_none_or(|r| r.0 != self.filter) {
                    self.rows = Some((self.filter.clone(), rows(text(), &self.filter)));
                }
                let rows = &self.rows.as_ref().unwrap().1;
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let h = ui.fonts_mut(|f| f.row_height(&font));
                egui::ScrollArea::both().auto_shrink(false).show_rows(ui, h, rows.len(), |ui, range| {
                    for r in &rows[range] {
                        ui.add(egui::Label::new(RichText::new(r).monospace()).extend());
                    }
                });
            });
        if !open {
            self.open = false;
        }
    }
}

/// The rows to show: everything, or for a filter, the introduction, the list entries and the
/// licence texts whose crates or licence match it (ignoring case).
fn rows(text: &str, filter: &str) -> Vec<String> {
    let needle = filter.trim().to_lowercase();
    let mut out = Vec::new();
    let mut push = |line: &str| {
        let mut rest = line;
        loop {
            let cut = rest.char_indices().nth(COLS).map_or(rest.len(), |(i, _)| i);
            // Break at the last space that fits, if there is one.
            let cut = if cut < rest.len() { rest[..cut].rfind(' ').filter(|&i| i > 0).unwrap_or(cut) } else { cut };
            out.push(rest[..cut].replace('\t', "    "));
            rest = rest[cut..].trim_start();
            if rest.is_empty() {
                break;
            }
        }
    };
    if needle.is_empty() {
        text.lines().for_each(&mut push);
        return out;
    }
    // Sections start at a rule line ("=====") followed by "Used by:" lines and another rule.
    let (head, sections) = text.split_once("\n=").map_or((text, ""), |(h, s)| (h, s));
    for line in head.lines() {
        if !line.contains('\t') || line.to_lowercase().contains(&needle) {
            push(line);
        }
    }
    let sections = format!("={sections}");
    let rule = "=".repeat(72);
    let start = format!("{rule}\nUsed by:");
    let mut found = 0;
    for part in sections.split(start.as_str()).filter(|s| !s.trim().is_empty()) {
        let (users, body) = part.split_once(&format!("\n{rule}\n")).unwrap_or((part, ""));
        if users.to_lowercase().contains(&needle) || body.to_lowercase().contains(&needle) && found < 50 {
            found += 1;
            push(&rule);
            push(&format!("Used by:{}", users.replace('\n', " ")));
            push(&rule);
            body.trim_end().lines().for_each(&mut push);
            push("");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_are_there_and_filter() {
        let text = text();
        assert!(text.contains("eframe ") && text.contains("Permission is hereby granted"));
        let all = rows(text, "");
        let some = rows(text, "eframe");
        // A Windows checkout may have CRLF line ends, which the sections are split without.
        assert!(!text.contains('\r'));
        assert!(text.contains("syntax grammar"), "the grammars' licences are listed");
        assert!(some.len() < all.len() && some.iter().any(|r| r.starts_with("eframe ")));
        assert!(some.iter().any(|r| r.starts_with("Used by:") && r.contains("eframe")));
        assert!(all.iter().all(|r| r.chars().count() <= COLS + 8));
    }
}
