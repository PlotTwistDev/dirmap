//! Keeping cloud files in the cloud: nothing Filescape does on its own reads a file that's only
//! online (that would download it). When the user asks for something that has to read such
//! files, they're told what it would download and choose.
//!
//! Which files are online-only comes from the metadata the scan and folder listings already
//! read (see `scan::on_disk`), so checking costs nothing extra.

use super::*;
use std::path::Path;

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
        if files == 0 {
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
