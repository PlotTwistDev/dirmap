//! File and folder pickers that don't block the frame.
//!
//! A blocking picker (`rfd::FileDialog`) runs its own event loop from inside the frame. On macOS,
//! winit then gets events (a drag passing over the window, say) while it's still in the middle
//! of handling one, panics, and the app aborts. So pickers here are the asynchronous kind (a
//! sheet on macOS), polled once a frame, and what was picked is handed to a closure along with
//! the part of the app that asked. Nothing happens when the picker is cancelled.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

pub type Dialog = rfd::AsyncFileDialog;

/// What the picker chooses.
pub enum Pick {
    File,
    Folder,
    Save,
}

/// A picker that's open, and what to do with its answer.
pub struct Asking<T> {
    answer: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>>,
    then: Box<dyn FnOnce(&mut T, PathBuf)>,
}

/// Show `dialog`; when something is picked, `then` gets it (from [`poll`]).
pub fn ask<T>(list: &mut Vec<Asking<T>>, dialog: Dialog, pick: Pick, then: impl FnOnce(&mut T, PathBuf) + 'static) {
    let answer: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>> = match pick {
        Pick::File => Box::pin(dialog.pick_file()),
        Pick::Folder => Box::pin(dialog.pick_folder()),
        Pick::Save => Box::pin(dialog.save_file()),
    };
    list.push(Asking { answer, then: Box::new(then) });
}

/// Check the pickers `target` keeps (in the list `list` gives) and act on those that have
/// answered. Returns whether any is still open (so the caller keeps repainting).
pub fn poll<T>(target: &mut T, list: impl Fn(&mut T) -> &mut Vec<Asking<T>>) -> bool {
    if list(target).is_empty() {
        return false;
    }
    let mut open = std::mem::take(list(target));
    let mut answered = Vec::new();
    let mut cx = Context::from_waker(Waker::noop());
    let mut i = 0;
    while i < open.len() {
        match open[i].answer.as_mut().poll(&mut cx) {
            Poll::Ready(answer) => {
                let a = open.remove(i);
                if let Some(h) = answer {
                    answered.push((a.then, h.path().to_path_buf()));
                }
            }
            Poll::Pending => i += 1,
        }
    }
    // Pickers asked for while acting on these go after the ones still open.
    for (then, path) in answered {
        then(target, path);
    }
    let added = std::mem::take(list(target));
    open.extend(added);
    *list(target) = open;
    !list(target).is_empty()
}

/// Where [`ask_into`] puts what was picked.
pub type Mailbox = Arc<Mutex<Option<PathBuf>>>;

/// For code with nowhere to keep a closure (a settings row, say): show `dialog`, and when
/// something is picked it lands in the returned mailbox and the window repaints. Check the
/// mailbox each frame (keep it in egui's memory).
pub fn ask_into(dialog: Dialog, pick: Pick, ctx: &eframe::egui::Context) -> Mailbox {
    let mut answer: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> = match pick {
        Pick::File => Box::pin(dialog.pick_file()),
        Pick::Folder => Box::pin(dialog.pick_folder()),
        Pick::Save => Box::pin(dialog.save_file()),
    };
    let mailbox = Mailbox::default();
    let (out, ctx) = (mailbox.clone(), ctx.clone());
    std::thread::spawn(move || {
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(a) = answer.as_mut().poll(&mut cx) {
                if let Some(h) = a {
                    if let Ok(mut m) = out.lock() {
                        *m = Some(h.path().to_path_buf());
                    }
                    ctx.request_repaint();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    mailbox
}
