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
    /// How much isn't known (a folder in the file manager, never scanned): `files` and `bytes`
    /// are then 0 and the question says so.
    pub unknown: bool,
}

impl App {
    /// Run `then` now if nothing it reads is only in the cloud; otherwise ask first, saying how
    /// many files and how much `what` would download.
    pub(super) fn after_cloud_ok(&mut self, what: impl Into<String>, files: u64, bytes: u64, then: impl FnOnce(&mut App) + 'static) {
        if files == 0 || !self.settings.cloud_confirm {
            then(self);
        } else {
            self.cloud_ask = Some(CloudAsk { what: what.into(), files, bytes, then: Box::new(then), unknown: false });
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
                if ask.unknown {
                    ui.label(format!("{} downloads everything in it that is only in the cloud to this computer. Filescape can't tell how much that is without a scan.", ask.what));
                } else {
                    ui.label(format!("{files} only in the cloud. {} has to read them, which downloads about {} to this computer.", ask.what, scan::human(ask.bytes)));
                }
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

    /// The storage breakdown under the map (Storage, in the summary strip): a bar splitting the
    /// scan's data into what's only on this computer, downloaded from the cloud, only in the
    /// cloud and unknown, with the services involved and the reclaimable space beside it. Shown
    /// only when the scan has cloud files. Clicking a part filters the map to it (Pro). Nothing
    /// here downloads anything.
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
            (local, "Not in the cloud", "Files on this computer that aren't in a cloud folder.", Stored::Local),
            (downloaded, "Downloaded", "Cloud files with their data on this computer, pinned ones included.", Stored::Downloaded),
            (away, "Only in the cloud", "Data in the cloud that isn't on this computer: online-only files, and the missing part of partly downloaded ones. Opening them downloads it.", Stored::OnlineOnly),
            (t.unknown_bytes, "Unknown", "Files on mounted cloud drives, which don't say how much of each file is stored on this computer.", Stored::Unknown),
        ];
        let here = total.saturating_sub(away).saturating_sub(t.unknown_bytes);
        let k = crate::kit::tokens(ui);
        let mut pick = None;
        let mut reclaim = false;
        let services = self.services();
        let counts = if t.online_only + t.partial > 0 {
            format!("{} of {} cloud files aren't fully downloaded ({} online only, {} partly)", group(t.online_only + t.partial), group(t.cloud_files), group(t.online_only), group(t.partial))
        } else {
            format!("{} cloud files, all downloaded", group(t.cloud_files))
        };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(egui::RichText::new("STORAGE").size(10.0).color(k.weak)).on_hover_text("Where the data in this scan is, by logical size. The whole scan is counted, whatever the filters.");
            let names = if services.is_empty() { "Cloud".to_string() } else { services.join(", ") };
            ui.label(egui::RichText::new(names).size(12.0).color(k.strong));
            let pct = here as f64 * 100.0 / total as f64;
            let pct = if here > 0 && pct < 1.0 { "<1%".to_string() } else { format!("{pct:.0}%") };
            ui.label(egui::RichText::new(format!("· {pct} on this computer · {counts}")).size(12.0).color(k.weak));
        });
        // The bar, as wide as the summary.
        let (bar, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
        ui.painter().rect_filled(bar, 4.0, k.track);
        let mut x = bar.left();
        let mut hovered = None;
        for (i, (bytes, ..)) in parts.iter().enumerate() {
            let w = bar.width() * (*bytes as f64 / total as f64) as f32;
            if w > 0.0 {
                let r = Rect::from_min_max(pos2(x, bar.top()), pos2((x + w).min(bar.right()), bar.bottom()));
                let first = x <= bar.left() + 0.5;
                let last = x + w >= bar.right() - 0.5;
                let cr = egui::CornerRadius { nw: if first { 4 } else { 0 }, sw: if first { 4 } else { 0 }, ne: if last { 4 } else { 0 }, se: if last { 4 } else { 0 } };
                ui.painter().rect_filled(r, cr, BAR[i]);
                if resp.hover_pos().is_some_and(|p| r.contains(p)) {
                    hovered = Some(i);
                }
                x += w;
            }
        }
        if let Some(i) = hovered {
            let (bytes, name, tip, _) = parts[i];
            resp.on_hover_text(format!("{name}: {} ({:.1}%)\n\n{tip}", scan::human(bytes), bytes as f64 * 100.0 / total as f64));
        }
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let font = FontId::proportional(12.0);
            for (i, (bytes, name, tip, st)) in parts.iter().enumerate() {
                if *bytes == 0 {
                    continue;
                }
                let text = format!("{name}  {}", scan::human(*bytes));
                let w = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), k.text).size().x) + 24.0;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
                let on = self.filter.stored == st.bit() || (*st == Stored::Downloaded && self.filter.stored == Stored::Downloaded.bit() | Stored::Pinned.bit()) || (*st == Stored::OnlineOnly && self.filter.stored == Stored::OnlineOnly.bit() | Stored::Partial.bit());
                if on {
                    ui.painter().rect_filled(r, 5.0, k.accent_soft);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 5.0, k.pal.hover);
                }
                let sw = Rect::from_center_size(pos2(r.left() + 11.0, r.center().y), vec2(10.0, 10.0));
                ui.painter().rect_filled(sw, 2.5, BAR[i]);
                ui.painter().text(pos2(r.left() + 21.0, r.center().y), Align2::LEFT_CENTER, text, font.clone(), if on { k.strong } else { k.text });
                let more = if on { "Click to show everything again." } else { "Click to show only these files on the map." };
                if resp.on_hover_text(format!("{tip}\n\n{more}")).clicked() {
                    pick = Some((*st, on));
                }
            }
            if t.reclaimable_bytes > 0 && crate::cloudops::supported() {
                ui.add_space(8.0);
                let pinned = if t.pinned_alloc > 0 { format!(" Files set to always stay on this device ({}) aren't counted.", scan::human(t.pinned_alloc)) } else { String::new() };
                let r = ui.add(egui::Button::new(egui::RichText::new(format!("Reclaimable  {}", scan::human(t.reclaimable_bytes))).size(12.0).color(k.text)).corner_radius(5.0));
                if r.on_hover_text(format!(
                    "Downloaded cloud files ({}) whose space your cloud app can free up while keeping them in the cloud.{pinned} Click to see where they are (the Reclaimable view), then right-click items and choose {}.",
                    group(t.reclaimable), crate::cloudops::Action::FreeUp.label()
                )).clicked() {
                    reclaim = true;
                }
            }
            if self.cloud_fade_on() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Faded tiles: only in the cloud").size(11.5).color(k.weak))
                    .on_hover_text("At Logical size, files not on this computer are drawn faded so what's here stands out (half-faded: partly downloaded). Turn this off in Display, or in Settings > Cloud storage.");
            }
        });
        if reclaim && self.premium(crate::licence::Feature::CloudManagement) {
            self.set_metric(scan::Metric::Reclaimable);
        }
        if let Some((s, on)) = pick
            && (on || self.premium(crate::licence::Feature::Filters))
        {
            self.filter.stored = match (on, s) {
                (true, _) => 0,
                (false, Stored::Downloaded) => Stored::Downloaded.bit() | Stored::Pinned.bit(),
                (false, Stored::OnlineOnly) => Stored::OnlineOnly.bit() | Stored::Partial.bit(),
                (false, s) => s.bit(),
            };
            // As in the Filters panel: online-only files have no area at Size: On disk.
            if self.filter.stored & AWAY != 0 && self.metric == scan::Metric::Disk {
                self.set_metric(scan::Metric::Logical);
            }
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

}

/// Stored states whose data is (mostly) not on this computer.
const AWAY: u8 = Stored::OnlineOnly.bit() | Stored::Partial.bit();

/// A folder in a cloud folder whose files are kept on demand (from what's already detected).
pub(super) fn on_demand_dir(p: &Path) -> bool {
    crate::cloud::root_of(&crate::cloud::known(), p).is_some_and(|r| r.kind == crate::cloud::Kind::OnDemand)
}

/// Whether the system can free up or keep this item's cloud files.
pub(super) fn manageable(n: &Node) -> bool {
    crate::cloudops::supported() && n.flags & scan::store::ON_DEMAND != 0 && n.flags & scan::store::MOUNT == 0
}

impl App {
    /// Free up or keep items from the tree, with what the scan knows about them.
    pub(super) fn cloud_manage(&mut self, a: crate::cloudops::Action, items: Vec<(PathBuf, &Node)>) {
        let items: Vec<(PathBuf, &Node)> = items.into_iter().filter(|(_, n)| manageable(n)).collect();
        if items.is_empty() {
            return;
        }
        let bulk = items.len() > 1 || items.iter().any(|(_, n)| n.is_dir);
        let (mut files, mut bytes) = (0, 0);
        for (_, n) in &items {
            let t = scan::cloud_tally(n);
            files += t.online_only + t.partial;
            bytes += t.online_bytes;
        }
        self.cloud_run(a, items.into_iter().map(|(p, _)| p).collect(), Some((files, bytes)), bulk);
    }

    /// Ask the system to free up or keep `paths` on a background thread, then read them again.
    /// Folders and several items at once are part of Filescape Professional; keeping files asks
    /// first when it would download (`download`: files and bytes, `None` when not known).
    pub(super) fn cloud_run(&mut self, a: crate::cloudops::Action, paths: Vec<PathBuf>, download: Option<(u64, u64)>, bulk: bool) {
        use crate::cloudops::{self, Action};
        if paths.is_empty() || (bulk && !self.premium(crate::licence::Feature::CloudManagement)) {
            return;
        }
        let what = if paths.len() == 1 {
            paths[0].file_name().map(|n| format!("\"{}\"", n.to_string_lossy())).unwrap_or_else(|| "the item".into())
        } else {
            format!("{} items", paths.len())
        };
        let label = format!("{} {what}", a.label());
        let go = move |app: &mut App| {
            app.run_op(label, move || {
                let o = cloudops::apply(&paths, a, &std::sync::atomic::AtomicBool::new(false));
                let r = if o.done == 0 && o.failed > 0 { Err(o.summary(a)) } else { Ok(Some(o.summary(a))) };
                (r, paths)
            });
        };
        match (a, download) {
            (Action::FreeUp, _) | (Action::Keep, Some((0, _))) => go(self),
            (Action::Keep, Some((files, bytes))) => self.after_cloud_ok(format!("{} {what}", a.label()), files, bytes, go),
            (Action::Keep, None) if !self.settings.cloud_confirm => go(self),
            (Action::Keep, None) => {
                self.cloud_ask = Some(CloudAsk { what: format!("{} {what}", a.label()), files: 0, bytes: 0, then: Box::new(go), unknown: true });
            }
        }
    }
}
