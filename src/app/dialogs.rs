//! The app's file and folder pickers: see `asking.rs` for why they don't block the frame.

use super::App;
use eframe::egui;
use std::path::PathBuf;
use std::time::Duration;

pub(super) use crate::asking::{Dialog, Pick};

pub(super) type Asking = crate::asking::Asking<App>;

/// [`App::ask_path`] for code that holds a borrow of another part of the app: pass `&mut
/// app.asking`.
pub(super) fn ask(asking: &mut Vec<Asking>, dialog: Dialog, pick: Pick, then: impl FnOnce(&mut App, PathBuf) + 'static) {
    crate::asking::ask(asking, dialog, pick, then);
}

impl App {
    /// Show `dialog`, and when something is picked, call `then` with it.
    pub(super) fn ask_path(&mut self, dialog: Dialog, pick: Pick, then: impl FnOnce(&mut App, PathBuf) + 'static) {
        ask(&mut self.asking, dialog, pick, then);
    }

    /// Check the open pickers (the app's and Licensing's); act on those that have answered.
    /// Call once per frame.
    pub(super) fn poll_dialogs(&mut self, ctx: &egui::Context) {
        let mine = crate::asking::poll(self, |a| &mut a.asking);
        let licence = crate::asking::poll(&mut self.licence, |l| &mut l.ui.asking);
        if mine || licence {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
