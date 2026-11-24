//! Noticing changes other programs make to the folders on screen.
//!
//! The system's change notices (inotify, ReadDirectoryChangesW, FSEvents) are asked for on a
//! thread of their own, because starting to watch a folder on a network share can wait on the
//! server. Only the folders on screen are watched, each without its subfolders. The UI asks with
//! `set` and collects the folders that changed with `changed`; it reads them again itself.

use notify::{RecursiveMode, Watcher as _};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

/// Called when a change arrives, to wake the UI.
type Wake = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

pub struct Watcher {
    /// The folders to watch, sent to the watching thread.
    want: Option<mpsc::Sender<HashSet<PathBuf>>>,
    /// Folders something changed in.
    got: mpsc::Receiver<PathBuf>,
    last: HashSet<PathBuf>,
    wake: Wake,
}

impl Default for Watcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Watcher {
    pub fn new() -> Self {
        let (want_tx, want_rx) = mpsc::channel::<HashSet<PathBuf>>();
        let (got_tx, got_rx) = mpsc::channel::<PathBuf>();
        let wake: Wake = Arc::new(Mutex::new(None));
        let wake2 = wake.clone();
        let spawned = std::thread::Builder::new().name("dirmap-watch".into()).spawn(move || {
            let tx = got_tx.clone();
            let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(ev) = res else { return };
                if matches!(ev.kind, notify::EventKind::Access(_)) {
                    return;
                }
                for p in ev.paths {
                    // A change to an item is a change to the folder it's in; the folder itself
                    // may be the one watched too (renamed or removed).
                    if let Some(parent) = p.parent() {
                        let _ = tx.send(parent.to_path_buf());
                    }
                    let _ = tx.send(p);
                }
                if let Some(f) = wake2.lock().ok().as_deref().and_then(|w| w.as_ref()) {
                    f();
                }
            });
            let Ok(mut w) = w else { return };
            let mut watching: HashSet<PathBuf> = HashSet::new();
            while let Ok(mut want) = want_rx.recv() {
                // Only the latest wish matters.
                while let Ok(newer) = want_rx.try_recv() {
                    want = newer;
                }
                for gone in watching.difference(&want) {
                    let _ = w.unwatch(gone);
                }
                let mut now = HashSet::new();
                for d in want {
                    if watching.contains(&d) || w.watch(&d, RecursiveMode::NonRecursive).is_ok() {
                        now.insert(d);
                    }
                }
                watching = now;
            }
        });
        Watcher { want: spawned.ok().map(|_| want_tx), got: got_rx, last: HashSet::new(), wake }
    }

    /// Call this when a change arrives (to wake the UI), if nothing is set yet.
    pub fn wake_with(&self, f: impl Fn() + Send + 'static) {
        if let Ok(mut w) = self.wake.lock()
            && w.is_none()
        {
            *w = Some(Box::new(f));
        }
    }

    /// Watch these folders (and stop watching others).
    pub fn set(&mut self, dirs: HashSet<PathBuf>) {
        if dirs != self.last {
            if let Some(tx) = &self.want {
                let _ = tx.send(dirs.clone());
            }
            self.last = dirs;
        }
    }

    /// The watched folders that changed since last asked.
    pub fn changed(&self) -> HashSet<PathBuf> {
        let mut out = HashSet::new();
        while let Ok(p) = self.got.try_recv() {
            if self.last.contains(&p) {
                out.insert(p);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn notices_a_new_file() {
        let dir = std::env::temp_dir().join(format!("dirmap-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let mut w = Watcher::new();
        w.set([dir.clone()].into_iter().collect());
        // Give the thread a moment to start watching.
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(dir.join("new.txt"), b"hi").unwrap();
        let start = Instant::now();
        let mut seen = false;
        while start.elapsed() < Duration::from_secs(5) {
            if w.changed().contains(&dir) {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::remove_dir_all(&dir).ok();
        assert!(seen);
    }
}
