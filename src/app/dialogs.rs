//! File and folder pickers that don't block the frame.
//!
//! A blocking picker (`rfd::FileDialog`) runs its own event loop from inside the frame. On macOS,
//! winit then gets events (a drag passing over the window, say) while it's still in the middle
//! of handling one, panics, and the app aborts. So pickers here are the asynchronous kind (a
//! sheet on macOS), polled once a frame, and what was picked is handed to a closure when it
//! comes back. Nothing happens when the picker is cancelled.

use super::App;
use eframe::egui;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

pub(super) type Dialog = rfd::AsyncFileDialog;

/// What the picker chooses.
pub(super) enum Pick {
    File,
    Folder,
    Save,
}

type Then = Box<dyn FnOnce(&mut App, PathBuf)>;

pub(super) struct Asking {
    answer: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>>,
    then: Then,
}

impl App {
    /// Show `dialog`, and when something is picked, call `then` with it.
    pub(super) fn ask_path(&mut self, dialog: Dialog, pick: Pick, then: impl FnOnce(&mut App, PathBuf) + 'static) {
        let answer: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>> = match pick {
            Pick::File => Box::pin(dialog.pick_file()),
            Pick::Folder => Box::pin(dialog.pick_folder()),
            Pick::Save => Box::pin(dialog.save_file()),
        };
        self.asking.push(Asking { answer, then: Box::new(then) });
    }

    /// Check the open pickers; act on those that have answered. Call once per frame.
    pub(super) fn poll_dialogs(&mut self, ctx: &egui::Context) {
        if self.asking.is_empty() {
            return;
        }
        let mut cx = Context::from_waker(Waker::noop());
        let mut i = 0;
        while i < self.asking.len() {
            match self.asking[i].answer.as_mut().poll(&mut cx) {
                Poll::Ready(answer) => {
                    let a = self.asking.remove(i);
                    if let Some(h) = answer {
                        (a.then)(self, h.path().to_path_buf());
                    }
                }
                Poll::Pending => i += 1,
            }
        }
        if !self.asking.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
