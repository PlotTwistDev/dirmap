//! The summary under the Storage Analysis map: the scanned folder, how much it holds, and the
//! file types taking the most space, each with its map colour and total (as on filescape.app).
//!
//! Everything here follows the filters: with a type, size or Custom filter on, the totals are
//! those of the matching files only. Nothing is counted per frame: the totals come from what the
//! filter work already produces in the background (`App::exts`, the filtered tree's list, or the
//! Highlight match counts), so this stays instant on millions of files.

use super::*;
use crate::fsys::PathShow;

/// How many types the summary lists at most.
const MAX_TYPES: usize = 10;

/// What the summary shows.
#[derive(Debug, PartialEq)]
pub(super) struct Summary {
    /// Files and bytes counted (the matches, when filtering).
    pub files: u64,
    pub bytes: u64,
    /// The whole scan, to say "of" when filtering.
    pub all_files: u64,
    pub all_bytes: u64,
    /// The biggest types, (lowercased extension, bytes), biggest first.
    pub types: Vec<(String, u64)>,
    /// A filter is on but its totals are still being worked out (the whole scan is shown).
    pub pending: bool,
}

/// The active filters in words, e.g. "Images, Video · larger than 100 MB · \"*beauty*\"".
fn describe(f: &ViewFilter) -> String {
    let mut parts = Vec::new();
    if f.kinds != 0 {
        let names: Vec<&str> = Category::ALL
            .iter()
            .filter(|c| f.kinds & c.bit() != 0)
            // The labels start with an emoji; the words are enough here.
            .map(|c| c.label().split_once(' ').map_or(c.label(), |(_, t)| t))
            .collect();
        parts.push(names.join(", "));
    }
    if f.larger > 0 {
        parts.push(format!("larger than {}", scan::human(f.larger)));
    }
    if f.smaller > 0 {
        parts.push(format!("smaller than {}", scan::human(f.smaller)));
    }
    if let Some(c) = &f.custom {
        parts.push(format!("\"{}\"", c.source()));
    }
    parts.join(" · ")
}

impl App {
    /// The numbers for the summary, from results already worked out. None before a scan.
    pub(super) fn summary(&self) -> Option<Summary> {
        let tree = self.tree.as_ref()?;
        let top = |v: &mut dyn Iterator<Item = (String, u64)>| v.filter(|e| e.1 > 0).take(MAX_TYPES).collect::<Vec<_>>();
        let whole = || Summary {
            files: tree.files,
            bytes: tree.size,
            all_files: tree.files,
            all_bytes: tree.size,
            types: top(&mut self.exts.iter().map(|e| (e.ext.clone(), e.bytes))),
            pending: false,
        };
        if !self.filter.active() {
            return Some(whole());
        }
        // Show matches only: the filtered tree and its own type list.
        if let Some(s) = self.shown.as_ref().filter(|s| s.filter == self.filter) {
            return Some(Summary { files: s.tree.files, bytes: s.tree.size, types: top(&mut s.exts.iter().map(|e| (e.ext.clone(), e.bytes))), ..whole() });
        }
        // Highlight: the match counts, with bytes per type.
        if let Some(h) = self.current_hits() {
            return Some(Summary { files: h.files, bytes: h.bytes, types: top(&mut h.exts.iter().cloned()), ..whole() });
        }
        Some(Summary { pending: true, ..whole() })
    }

    /// The summary row: folder, size and file count, the active filters, then a chip per big
    /// type in its map colour. Clicking a chip highlights that type on the map.
    pub(super) fn summary_bar(&mut self, ui: &mut egui::Ui) {
        let Some(sum) = self.summary() else { return };
        let pal = style::pal(ui);
        let mono = FontId::monospace(11.5);
        let filtering = self.filter.active() && !sum.pending;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut size = if filtering {
                format!("{} of {}", scan::human(sum.bytes), scan::human(sum.all_bytes))
            } else {
                scan::human(sum.bytes)
            };
            // Which size this is, and the other one when they differ (cloud files, compression).
            if let Some(t) = self.tree.as_ref().filter(|t| t.flags & scan::store::NO_ALLOC == 0) {
                let other = if self.metric == scan::Metric::Disk { scan::Metric::Logical } else { scan::Metric::Disk };
                let (a, b) = (scan::human(t.size), scan::human(t.size_by(other)));
                size.push_str(if self.metric == scan::Metric::Disk { " on disk" } else { " logical" });
                if !filtering && a != b {
                    size.push_str(&format!(" ({b} {})", if other == scan::Metric::Disk { "on disk" } else { "logical" }));
                }
            }
            let files = if filtering {
                format!("{} of {} files", group(sum.files), group(sum.all_files))
            } else {
                format!("{} files", group(sum.files))
            };
            ui.label(egui::RichText::new(format!("{} · {size} · {files}", self.scanned_path.shown())).font(mono.clone()).color(pal.strong));
            if self.filter.active() {
                ui.label(egui::RichText::new(format!(" · {}", describe(&self.filter))).font(mono.clone()).color(pal.weak));
            }
            if sum.pending {
                ui.add_space(6.0);
                ui.spinner();
                ui.weak("Counting…");
            }
        });
        if self.color_mode != ColorMode::Type || sum.types.is_empty() {
            return;
        }
        let mut toggle = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for (ext, bytes) in &sum.types {
                let on = self.highlight_ext.as_deref() == Some(ext.as_str());
                let label = if ext.is_empty() { "(none)" } else { ext.as_str() };
                let text = format!("{label} {}", scan::human(*bytes));
                let w = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), mono.clone(), pal.text).size().x) + 26.0;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
                if on {
                    ui.painter().rect_filled(r, 4.0, pal.sel);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, pal.hover);
                }
                let swatch = Rect::from_center_size(pos2(r.left() + 10.0, r.center().y), vec2(9.0, 9.0));
                ui.painter().rect_filled(swatch, 2.0, self.color_for_ext(ext));
                ui.painter().text(pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, text, mono.clone(), if on { pal.strong } else { pal.text });
                let pct = *bytes as f64 * 100.0 / sum.bytes.max(1) as f64;
                let tip = format!(
                    "{pct:.1}% of {}. {}",
                    if filtering { "the matching files" } else { "the scan" },
                    if on { "Click to stop highlighting." } else { "Click to highlight these files on the map." }
                );
                if resp.on_hover_text(tip).clicked() {
                    toggle = Some((ext.clone(), on));
                }
            }
            if self.highlight_ext.is_some() && ui.add(egui::Button::new(egui::RichText::new("Clear").size(11.5)).frame_when_inactive(false)).clicked() {
                self.highlight_ext = None;
            }
        });
        if let Some((ext, on)) = toggle
            && (on || self.premium(crate::licence::Feature::Filters))
        {
            self.highlight_ext = if on { None } else { Some(ext) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_the_active_filters() {
        assert_eq!(describe(&ViewFilter::default()), "");
        let f = ViewFilter { kinds: Category::Images.bit() | Category::Video.bit(), larger: 100 * 1024 * 1024, ..Default::default() };
        let d = describe(&f);
        assert!(d.starts_with("Video, Images · larger than 100"), "{d}");
        let f = ViewFilter {
            smaller: 4096,
            custom: Some(Arc::new(crate::pattern::Filter::compile("*beauty*", crate::pattern::Options::default()).unwrap())),
            ..Default::default()
        };
        assert_eq!(describe(&f), format!("smaller than {} · \"*beauty*\"", scan::human(4096)));
    }
}
