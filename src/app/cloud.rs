//! Keeping cloud files in the cloud: nothing Filescape does on its own reads a file that's only
//! online (that would download it). When the user asks for something that has to read such
//! files, they're told what it would download and choose.
//!
//! Which files are online-only comes from the metadata the scan and folder listings already
//! read (see `scan::on_disk`), so checking costs nothing extra.

use super::*;
use crate::kit::icons::{self, Icon};
use scan::Stored;
use std::path::Path;

/// The status icon for a storage state, and its colour; `None` for ordinary local files.
pub(super) fn stored_icon(s: Stored) -> Option<(Icon, Color32)> {
    Some(match s {
        Stored::Local => return None,
        Stored::OnlineOnly => (Icon::Cloud, Color32::from_rgb(74, 134, 222)),
        Stored::Partial => (Icon::CloudHalf, Color32::from_rgb(222, 150, 40)),
        Stored::Downloaded => (Icon::CircleCheck, Color32::from_rgb(64, 168, 92)),
        Stored::Pinned => (Icon::Pinned, Color32::from_rgb(52, 150, 80)),
        Stored::Unknown => (Icon::CloudQuestion, Color32::from_gray(140)),
    })
}

/// Draw the status icon for `s` in `rect`, with `tip` on hover. Nothing for local files.
/// Returns whether anything was drawn.
pub(super) fn paint_stored(ui: &egui::Ui, rect: Rect, s: Stored, id: egui::Id, tip: impl FnOnce() -> String) -> bool {
    let Some((icon, color)) = stored_icon(s) else { return false };
    icons::paint(ui.painter(), rect, icon, color);
    if ui.rect_contains_pointer(rect) {
        let t = tip();
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id, egui::PopupAnchor::Pointer).gap(12.0).show(|ui| {
            ui.set_max_width(320.0);
            ui.label(t);
        });
    }
    true
}

/// The states in a filter's set, in words: "Online only, Partly downloaded".
pub(super) fn stored_names(set: u8) -> String {
    Stored::ALL.iter().filter(|s| set & s.bit() != 0).map(|s| s.label()).collect::<Vec<_>>().join(", ")
}

/// The tooltip for a cloud status icon: the state, what it means, and the provider.
pub(super) fn stored_tip(s: Stored, detail: Option<String>, path: Option<&Path>) -> String {
    let mut t = match detail {
        Some(d) => format!("{d}\n\n{}", s.explain()),
        None => format!("{}\n\n{}", s.label(), s.explain()),
    };
    let roots = crate::cloud::known();
    if let Some(r) = path.and_then(|p| crate::cloud::root_of(&roots, p)) {
        t.push_str(&format!("\n\n{} ({})", r.provider.name(), r.kind.describe()));
    }
    t
}

/// "Reading these would download them": waiting for the user's answer.
pub(super) struct CloudAsk {
    /// What was asked for, e.g. "Playing the sequence".
    pub what: String,
    pub files: u64,
    /// Bytes that would be downloaded.
    pub bytes: u64,
    /// Done when the user agrees.
    pub then: Box<dyn FnOnce(&mut App)>,
}

impl App {
    /// Run `then` now if nothing it reads is only in the cloud; otherwise ask first, saying how
    /// many files and how much `what` would download.
    pub(super) fn after_cloud_ok(&mut self, what: impl Into<String>, files: u64, bytes: u64, then: impl FnOnce(&mut App) + 'static) {
        if files == 0 || !self.settings.cloud_confirm {
            then(self);
        } else {
            self.cloud_ask = Some(CloudAsk { what: what.into(), files, bytes, then: Box::new(then) });
        }
    }

    /// The question, while one is open.
    pub(super) fn cloud_ask_ui(&mut self, ctx: &egui::Context) {
        let Some(ask) = &self.cloud_ask else { return };
        let mut answer = None;
        let files = if ask.files == 1 { "1 file is".to_string() } else { format!("{} files are", group(ask.files)) };
        egui::Window::new("Download from the cloud?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                ui.label(format!("{files} only in the cloud. {} has to read them, which downloads about {} to this computer.", ask.what, scan::human(ask.bytes)));
                ui.add_space(4.0);
                ui.weak("They then take space here until your cloud app frees it again.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Download and continue").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        answer = Some(false);
                    }
                });
            });
        match answer {
            Some(true) => {
                let ask = self.cloud_ask.take().unwrap();
                (ask.then)(self);
            }
            Some(false) => self.cloud_ask = None,
            None => {}
        }
    }

    /// Of these paths, how many files in the focused folder's listing are only (partly) in the
    /// cloud, and how much reading them would download. Paths outside that folder aren't counted.
    pub(super) fn cloud_in_listing<'a>(&self, paths: impl Iterator<Item = &'a Path>) -> (u64, u64) {
        let (Some(dir), Some(l)) = (self.files.here().dir(), self.files.cur().listing.as_ref()) else { return (0, 0) };
        if !l.entries.iter().any(|e| e.not_local()) {
            return (0, 0);
        }
        let away: HashMap<&str, u64> = l.entries.iter().filter(|e| e.not_local()).map(|e| (&*e.name, e.size.saturating_sub(e.alloc))).collect();
        let (mut files, mut bytes) = (0, 0);
        for p in paths {
            if p.parent() == Some(dir)
                && let Some(b) = p.file_name().and_then(|n| away.get(&*n.to_string_lossy()))
            {
                files += 1;
                bytes += b;
            }
        }
        (files, bytes)
    }
}

/// Colours of the storage breakdown: local files, downloaded cloud files, only in the cloud,
/// unknown.
const BAR: [Color32; 4] = [Color32::from_rgb(120, 128, 140), Color32::from_rgb(64, 168, 92), Color32::from_rgb(74, 134, 222), Color32::from_rgb(170, 170, 170)];

impl App {
    /// Where the scan's data is, worked out once per tree (only walks folders with cloud files).
    pub(super) fn cloud_summary(&mut self) -> Option<scan::CloudTally> {
        let tree = self.tree.as_ref()?;
        let key = Arc::as_ptr(tree) as usize;
        if self.cloud_sum.as_ref().is_none_or(|c| c.0 != key) {
            self.cloud_sum = Some((key, scan::cloud_tally(tree)));
        }
        self.cloud_sum.map(|c| c.1)
    }

    /// The storage breakdown under the map: a bar split into data that's only on this computer,
    /// downloaded from the cloud, only in the cloud and unknown, with the services involved.
    /// Shown only when the scan has cloud files. Clicking a part filters the map to it (Pro).
    pub(super) fn cloud_bar(&mut self, ui: &mut egui::Ui) {
        if !self.view().cloud_breakdown {
            return;
        }
        let Some(t) = self.cloud_summary().filter(|t| t.any()) else { return };
        let Some(tree) = self.tree.clone() else { return };
        let total = tree.logical.max(1);
        let away = t.online_bytes;
        let downloaded = t.cloud_logical.saturating_sub(away).saturating_sub(t.unknown_bytes);
        let local = tree.logical.saturating_sub(t.cloud_logical);
        let parts = [
            (local, "Not in the cloud", "Files on this computer that aren't in a cloud folder.", Some(Stored::Local)),
            (downloaded, "Downloaded", "Cloud files with their data on this computer, pinned ones included.", Some(Stored::Downloaded)),
            (away, "Only in the cloud", "Data in the cloud that isn't on this computer: online-only files, and the missing part of partly downloaded ones. Opening them downloads it.", Some(Stored::OnlineOnly)),
            (t.unknown_bytes, "Unknown", "Files on mounted cloud drives, which don't say how much of each file is stored on this computer.", Some(Stored::Unknown)),
        ];
        let here = total.saturating_sub(away).saturating_sub(t.unknown_bytes);
        let pal = style::pal(ui);
        let mono = FontId::monospace(11.5);
        let mut pick = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let services = self.services();
            let label = if services.is_empty() { "Cloud".to_string() } else { services.join(", ") };
            ui.label(egui::RichText::new(format!("{label}:")).font(mono.clone()).color(pal.strong)).on_hover_text(
                "Where the data in this scan is, by logical size. The whole scan is counted, whatever the filters.",
            );
            let (bar, resp) = ui.allocate_exact_size(vec2(180.0, 10.0), Sense::hover());
            ui.painter().rect_filled(bar, 3.0, pal.hover);
            let mut x = bar.left();
            for (i, (bytes, ..)) in parts.iter().enumerate() {
                let w = bar.width() * (*bytes as f64 / total as f64) as f32;
                if w > 0.0 {
                    let r = Rect::from_min_max(pos2(x, bar.top()), pos2((x + w).min(bar.right()), bar.bottom()));
                    ui.painter().rect_filled(r, 0.0, BAR[i]);
                    x += w;
                }
            }
            resp.on_hover_text(format!("{:.0}% of the data in this scan is on this computer.", here as f64 * 100.0 / total as f64));
            for (i, (bytes, name, tip, st)) in parts.iter().enumerate() {
                if *bytes == 0 {
                    continue;
                }
                let text = format!("{name} {}", scan::human(*bytes));
                let w = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), mono.clone(), pal.text).size().x) + 22.0;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
                let on = st.is_some_and(|s| self.filter.stored == s.bit() || (s == Stored::Downloaded && self.filter.stored == Stored::Downloaded.bit() | Stored::Pinned.bit()));
                if on {
                    ui.painter().rect_filled(r, 4.0, pal.sel);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, pal.hover);
                }
                let sw = Rect::from_center_size(pos2(r.left() + 9.0, r.center().y), vec2(9.0, 9.0));
                ui.painter().rect_filled(sw, 2.0, BAR[i]);
                ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, text, mono.clone(), if on { pal.strong } else { pal.text });
                let more = if on { "Click to show everything again." } else { "Click to show only these files on the map." };
                if resp.on_hover_text(format!("{tip}\n\n{more}")).clicked() {
                    pick = Some((*st, on));
                }
            }
            let counts = if t.online_only + t.partial > 0 {
                format!("{} of {} cloud files aren't downloaded", group(t.online_only + t.partial), group(t.cloud_files))
            } else {
                format!("{} cloud files, all downloaded", group(t.cloud_files))
            };
            ui.label(egui::RichText::new(format!("{:.0}% here", here as f64 * 100.0 / total as f64)).font(mono.clone()).color(pal.weak))
                .on_hover_text(format!("{:.0}% of the data in this scan is on this computer. {counts}.", here as f64 * 100.0 / total as f64));
        });
        if let Some((Some(s), on)) = pick
            && (on || self.premium(crate::licence::Feature::Filters))
        {
            self.filter.stored = match (on, s) {
                (true, _) => 0,
                (false, Stored::Downloaded) => Stored::Downloaded.bit() | Stored::Pinned.bit(),
                (false, Stored::OnlineOnly) => Stored::OnlineOnly.bit() | Stored::Partial.bit(),
                (false, s) => s.bit(),
            };
        }
    }

    /// The cloud services the scanned folder is in or holds, by name.
    fn services(&self) -> Vec<&'static str> {
        let roots = crate::cloud::known();
        let mut v: Vec<&'static str> = crate::cloud::root_of(&roots, &self.scanned_path).into_iter().chain(crate::cloud::roots_below(&roots, &self.scanned_path)).map(|r| r.provider.name()).collect();
        v.dedup();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// "Stored:" chips in the filter bar, when the scan has cloud files: show only files that
    /// are online only, downloaded, pinned… Choosing is part of the Pro filters.
    pub(super) fn stored_filter_row(&mut self, ui: &mut egui::Ui) {
        let has = self.tree.as_ref().is_some_and(|t| t.flags & (scan::store::CLOUD | scan::store::NOT_LOCAL) != 0);
        if !has && self.filter.stored == 0 {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Stored:").on_hover_text("Where each file's data is. Only files in cloud folders are anything but \"Not in the cloud\".");
            if ui.selectable_label(self.filter.stored == 0, "Anywhere").clicked() {
                self.filter.stored = 0;
            }
            for s in Stored::ALL {
                let on = self.filter.stored & s.bit() != 0;
                let resp = ui.selectable_label(on, s.label()).on_hover_text(s.explain());
                if resp.clicked() && (on || self.premium(crate::licence::Feature::Filters)) {
                    self.filter.stored ^= s.bit();
                }
            }
        });
    }
}
