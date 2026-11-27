//! The summary under the Storage Analysis map: a strip of the scan's totals (on disk, logical,
//! only in the cloud, reclaimable, files, folders), the file-type legend, and on request the
//! scan's details and where its data is stored (the cloud breakdown).
//!
//! The totals are the whole scan's, so cloud data is never counted as local whatever the view;
//! with a filter on, a Matching total says how much of it matches. The legend follows the
//! filters. Nothing is counted per frame: the totals come from the tree's own sums, the cached
//! cloud tally and folder count, and what the filter work already produces in the background
//! (`App::exts`, the filtered tree's list, or the Highlight match counts), so this stays instant
//! on millions of files.

use super::*;
use super::viz::LegendItem;
use crate::fsys::PathShow;
use crate::kit;
use eframe::egui::RichText;

/// How many types the legend can list at most.
const MAX_TYPES: usize = 300;

/// What the summary shows.
#[derive(Debug, PartialEq)]
pub(super) struct Summary {
    /// Files and bytes counted (the matches, when filtering).
    pub files: u64,
    pub bytes: u64,
    /// The whole scan, to say "of" when filtering.
    pub all_files: u64,
    pub all_bytes: u64,
    /// The types, biggest first.
    pub types: Vec<LegendItem>,
    /// A filter is on but its totals are still being worked out (the whole scan is shown).
    pub pending: bool,
}

/// The active filters in words, e.g. "Images · Video · Larger than 100 MB · Name: *beauty*".
fn describe(f: &ViewFilter) -> String {
    super::viz::chips_of(f, false).into_iter().map(|(_, s)| s).collect::<Vec<_>>().join(" · ")
}

/// Folders at or below `n` (not counting `n`).
fn count_folders(n: &Node) -> u64 {
    let mut count = 0;
    let mut stack = vec![n];
    while let Some(x) = stack.pop() {
        for c in x.children.iter().filter(|c| c.is_dir) {
            count += 1;
            stack.push(c);
        }
    }
    count
}

/// One total in the strip.
struct Card {
    label: &'static str,
    value: String,
    tip: String,
    /// The size measure clicking it switches to.
    measure: Option<scan::Metric>,
    /// Lower drops first when the window is narrow.
    rank: u8,
}

/// Draws a total: a small label over a large value. Highlighted when it's the map's measure.
fn card(ui: &mut egui::Ui, c: &Card, active: bool) -> egui::Response {
    let t = kit::tokens(ui);
    let label = ui.painter().layout_no_wrap(c.label.to_string(), FontId::proportional(10.0), t.weak);
    let value = ui.painter().layout_no_wrap(c.value.clone(), FontId::proportional(15.5), t.strong);
    let w = label.size().x.max(value.size().x) + 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 42.0), if c.measure.is_some() { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered() && c.measure.is_some();
        let fill = if active { t.accent_soft } else if hovered { t.pal.hover } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, 6.0, fill);
        let x = rect.left() + 11.0;
        ui.painter().galley_with_override_text_color(pos2(x, rect.top() + 6.0), label, if active { t.accent } else { t.weak });
        ui.painter().galley_with_override_text_color(pos2(x, rect.top() + 19.0), value, t.strong);
        kit::focus_ring(ui, rect, &resp, 6.0);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(if c.measure.is_some() { egui::WidgetType::Button } else { egui::WidgetType::Label }, true, format!("{} {}", c.label, c.value)));
    let more = match c.measure {
        Some(_) if active => "\n\nThe map is sized by this.",
        Some(_) => "\n\nClick to size the map by this.",
        None => "",
    };
    resp.on_hover_text(format!("{}{more}", c.tip))
}

/// A thin divider between groups of totals.
fn divider(ui: &mut egui::Ui) {
    let t = kit::tokens(ui);
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 28.0), Sense::hover());
    ui.painter().vline(r.center().x, r.y_range(), egui::Stroke::new(1.0, t.divider));
}

impl App {
    /// The numbers for the summary, from results already worked out. None before a scan.
    pub(super) fn summary(&self) -> Option<Summary> {
        let tree = self.tree.as_ref()?;
        let top = |v: &mut dyn Iterator<Item = LegendItem>| v.filter(|e| e.bytes > 0).take(MAX_TYPES).collect::<Vec<_>>();
        let stats = |v: &[scan::ExtStat]| top(&mut v.iter().map(|e| LegendItem { ext: e.ext.clone(), bytes: e.bytes, files: Some(e.files) }));
        let whole = || Summary { files: tree.files, bytes: tree.size, all_files: tree.files, all_bytes: tree.size, types: stats(&self.exts), pending: false };
        if !self.filter.active() {
            return Some(whole());
        }
        // Only matches: the filtered tree and its own type list.
        if let Some(s) = self.shown.as_ref().filter(|s| s.filter == self.filter) {
            return Some(Summary { files: s.tree.files, bytes: s.tree.size, types: stats(&s.exts), ..whole() });
        }
        // Highlight: the match counts, with bytes per type.
        if let Some(h) = self.current_hits() {
            return Some(Summary { files: h.files, bytes: h.bytes, types: top(&mut h.exts.iter().map(|(e, b)| LegendItem { ext: e.clone(), bytes: *b, files: None })), ..whole() });
        }
        Some(Summary { pending: true, ..whole() })
    }

    /// Folders in the scan, counted once per tree.
    fn folder_total(&mut self) -> u64 {
        let Some(tree) = self.tree.as_ref() else { return 0 };
        let key = Arc::as_ptr(tree) as usize;
        if self.folder_sum.is_none_or(|f| f.0 != key) {
            self.folder_sum = Some((key, count_folders(tree)));
        }
        self.folder_sum.map(|f| f.1).unwrap_or(0)
    }

    /// The totals the strip shows, most important first.
    fn cards(&mut self, sum: &Summary) -> Vec<Card> {
        let Some(tree) = self.tree.clone() else { return Vec::new() };
        let mut v = Vec::new();
        let no_alloc = tree.flags & scan::store::NO_ALLOC != 0;
        if !no_alloc {
            v.push(Card {
                label: "ON DISK",
                value: scan::human(tree.alloc),
                tip: "Space the scanned files take on this computer's drive, as `du` and Explorer's \"Size on disk\" count it. Cloud files that are only online take none, so this is what deleting or freeing up would recover.".into(),
                measure: Some(scan::Metric::Disk),
                rank: 9,
            });
        }
        v.push(Card {
            label: "LOGICAL SIZE",
            value: scan::human(tree.logical),
            tip: if no_alloc {
                "Each file's full size. Space on disk isn't known for this analysis (a server, or a snapshot from an older version).".into()
            } else {
                "Each file's full size, wherever its data is: what Explorer calls \"Size\". Cloud files that are only online count in full, and compressed or sparse files count more than they take.".into()
            },
            measure: Some(scan::Metric::Logical),
            rank: 7,
        });
        let cloud = self.cloud_summary().filter(|c| c.any());
        if let Some(c) = cloud.filter(|_| tree.flags & scan::store::NOT_LOCAL != 0) {
            v.push(Card {
                label: "CLOUD ONLY",
                value: scan::human(c.online_bytes),
                tip: format!(
                    "Data in the cloud that isn't on this computer: {} online-only and {} partly downloaded files. It isn't counted on disk, and opening those files downloads it.",
                    group(c.online_only),
                    group(c.partial)
                ),
                measure: Some(scan::Metric::CloudOnly),
                rank: 5,
            });
        }
        if let Some(c) = cloud.filter(|_| tree.flags & scan::store::ON_DEMAND != 0 && crate::cloudops::supported()) {
            let pinned = if c.pinned_alloc > 0 { format!(" Files set to always stay on this device ({}) aren't counted.", scan::human(c.pinned_alloc)) } else { String::new() };
            v.push(Card {
                label: "RECLAIMABLE",
                value: scan::human(c.reclaimable_bytes),
                tip: format!(
                    "Space your cloud app could free up on this computer while keeping {} downloaded files in the cloud.{pinned} Nothing is downloaded or freed up until you ask.",
                    group(c.reclaimable)
                ),
                measure: Some(scan::Metric::Reclaimable),
                rank: 4,
            });
        }
        v.push(Card { label: "FILES", value: group(tree.files), tip: "Files in the scan, whatever the filters.".into(), measure: None, rank: 8 });
        let folders = self.folder_total();
        v.push(Card { label: "FOLDERS", value: group(folders), tip: "Folders in the scan, whatever the filters.".into(), measure: None, rank: 3 });
        if self.filter.active() {
            let (value, tip) = if sum.pending {
                ("Counting…".to_string(), "The files matching the filters are still being counted.".to_string())
            } else {
                (
                    format!("{} · {}", scan::human(sum.bytes), group(sum.files)),
                    format!(
                        "{} files ({}) match the filters: {}. Sized by the map's measure ({}).",
                        group(sum.files),
                        scan::human(sum.bytes),
                        describe(&self.filter),
                        self.metric.label().to_lowercase()
                    ),
                )
            };
            v.push(Card { label: "MATCHING", value, tip, measure: None, rank: 6 });
        }
        v
    }

    /// The summary under the map: the totals strip, the legend, and the details and cloud
    /// breakdown when they're open.
    pub(super) fn summary_bar(&mut self, ui: &mut egui::Ui) {
        let Some(sum) = self.summary() else { return };
        let t = kit::tokens(ui);
        let mut cards = self.cards(&sum);
        let has_cloud = self.cloud_summary().is_some_and(|c| c.any());
        let details_id = egui::Id::new("summary-details");
        let mut details = ui.data_mut(|d| *d.get_persisted_mut_or_default::<bool>(details_id));
        let mut breakdown = self.settings.cloud_breakdown;
        // Drop the least important totals until the strip fits beside its toggles.
        let toggles = if has_cloud { 210.0 } else { 110.0 };
        let fits = |ui: &egui::Ui, cards: &[Card]| {
            let w: f32 = cards
                .iter()
                .map(|c| {
                    let lw = ui.fonts_mut(|f| f.layout_no_wrap(c.label.to_string(), FontId::proportional(10.0), t.weak).size().x);
                    let vw = ui.fonts_mut(|f| f.layout_no_wrap(c.value.clone(), FontId::proportional(15.5), t.strong).size().x);
                    lw.max(vw) + 24.0
                })
                .sum();
            w + 20.0 <= ui.available_width() - toggles
        };
        while cards.len() > 1 && !fits(ui, &cards) {
            let (i, _) = cards.iter().enumerate().min_by_key(|(_, c)| c.rank).unwrap();
            cards.remove(i);
        }
        let mut measure = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut last_kind = None;
            for c in &cards {
                // A divider between sizes, counts and the match total.
                let kind = if c.measure.is_some() { 0 } else if c.label == "MATCHING" { 2 } else { 1 };
                if last_kind.is_some_and(|k| k != kind) {
                    divider(ui);
                }
                last_kind = Some(kind);
                if card(ui, c, c.measure == Some(self.metric)).clicked() {
                    measure = c.measure;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if viz::pill(ui, Some(kit::Icon::ChevronDown), "Details", details, false, false).on_hover_text("Where and when this was scanned, both sizes, and anything that couldn't be read").clicked() {
                    details = !details;
                }
                if has_cloud
                    && viz::pill(ui, Some(kit::Icon::Layers), "Storage", breakdown, false, false)
                        .on_hover_text("Where the data in this scan is: on this computer, downloaded from the cloud, or only in the cloud")
                        .clicked()
                {
                    breakdown = !breakdown;
                }
            });
        });
        ui.data_mut(|d| d.insert_persisted(details_id, details));
        if breakdown != self.settings.cloud_breakdown {
            self.settings.cloud_breakdown = breakdown;
            self.dialog.draft.cloud_breakdown = breakdown;
        }
        if let Some(m) = measure
            && m != self.metric
            && (m != scan::Metric::Reclaimable || self.premium(crate::licence::Feature::CloudManagement))
        {
            self.set_metric(m);
        }
        if self.color_mode == ColorMode::Type && !sum.types.is_empty() {
            ui.add_space(2.0);
            let total = sum.types.iter().map(|i| i.bytes).sum::<u64>().max(sum.bytes);
            self.type_legend(ui, &sum.types, total, false, "analysis");
        } else {
            self.highlight_ext.set_hover(None);
        }
        if details {
            ui.add_space(2.0);
            self.summary_details(ui);
        }
    }

    /// The details row: the folder, when and how fast it was scanned, both sizes, and what
    /// couldn't be read.
    fn summary_details(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.tree.clone() else { return };
        let t = kit::tokens(ui);
        let small = |s: String| RichText::new(s).size(12.0).color(t.text);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(self.scanned_path.shown()).size(12.0).color(t.strong));
            ui.label(small(format!("· {}", sizes_text(&tree, self.metric))));
            if self.scan_secs > 0.0 {
                ui.label(small(format!("· scanned in {:.1}s", self.scan_secs)));
            }
            if self.scan_errors > 0 {
                let text = if self.scan_errors == 1 { "· 1 item couldn't be read".to_string() } else { format!("· {} items couldn't be read", group(self.scan_errors)) };
                ui.label(RichText::new(text).size(12.0).color(t.warn)).on_hover_text("Their space isn't counted. Press F1 for why and what to do.");
            }
            if self.filter.active() {
                ui.label(RichText::new(format!("· Filters: {}", describe(&self.filter))).size(12.0).color(t.weak));
            }
        });
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
        assert!(d.starts_with("Video · Images · Larger than 100"), "{d}");
        let f = ViewFilter {
            smaller: 4096,
            custom: Some(Arc::new(crate::pattern::Filter::compile("*beauty*", crate::pattern::Options::default()).unwrap())),
            ..Default::default()
        };
        assert_eq!(describe(&f), format!("Smaller than {} · Name: *beauty*", scan::human(4096)));
    }
}
