//! Reading one folder for the file manager, independently of any scan.
//!
//! Each listing runs on its own short-lived thread and streams entries back in batches, so a
//! huge folder starts showing at once and a slow or dead network share never blocks the
//! window. Dropping the `Listing` cancels it; a thread stuck in the OS just finishes later.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One item of a folder.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: Box<str>,
    /// A folder, or a link to one (so it opens like a folder).
    pub is_dir: bool,
    /// A symbolic link, junction or other reparse point.
    pub is_link: bool,
    pub hidden: bool,
    pub readonly: bool,
    /// Bytes for files; 0 for folders.
    pub size: u64,
    /// Seconds since the Unix epoch, 0 when unknown.
    pub modified: i64,
    /// Creation time where the file system records it.
    pub created: Option<i64>,
}

impl Entry {
    pub fn ext(&self) -> &str {
        if self.is_dir { "" } else { crate::scan::ext_of(&self.name) }
    }
}

/// Sent from the reading thread.
enum Msg {
    Batch(Vec<Entry>),
    Done(Option<String>),
}

/// A folder being read (or read).
pub struct Listing {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub done: bool,
    /// Why the folder couldn't be read (it may still have delivered some entries).
    pub error: Option<String>,
    /// Entries that couldn't be read (vanished or not allowed).
    pub skipped: u64,
    pub started: Instant,
    /// Bumped whenever `entries` changes, so views know to sort again.
    pub generation: u64,
    rx: mpsc::Receiver<Msg>,
    cancel: Arc<AtomicBool>,
}

/// How many entries to collect before handing them over, at most.
const BATCH: usize = 512;
/// …or how long, so a slow share still shows its first items promptly.
const BATCH_TIME: Duration = Duration::from_millis(40);

impl Listing {
    /// Start reading `path` in the background.
    pub fn start(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c) = (path.clone(), cancel.clone());
        let spawned = std::thread::Builder::new().name("dirmap-list".into()).spawn(move || read(&p, &c, &tx));
        let mut l = Listing {
            path,
            entries: Vec::new(),
            done: false,
            error: None,
            skipped: 0,
            started: Instant::now(),
            generation: 0,
            rx,
            cancel,
        };
        if let Err(e) = spawned {
            l.done = true;
            l.error = Some(format!("Couldn't start reading: {e}"));
        }
        l
    }

    /// Take whatever has arrived. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Batch(b)) => {
                    self.entries.extend(b);
                    changed = true;
                }
                Ok(Msg::Done(err)) => {
                    self.done = true;
                    self.error = err;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.done {
                        self.done = true;
                        changed = true;
                    }
                    break;
                }
            }
        }
        if changed {
            self.generation += 1;
        }
        changed
    }

    /// Still reading after this long: worth telling the user (a slow or unreachable share).
    pub fn slow(&self) -> bool {
        !self.done && self.started.elapsed() > Duration::from_secs(2)
    }
}

impl Drop for Listing {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn read(path: &Path, cancel: &AtomicBool, tx: &mpsc::Sender<Msg>) {
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            let _ = tx.send(Msg::Done(Some(describe(&e))));
            return;
        }
    };
    let mut batch = Vec::with_capacity(64);
    let mut last = Instant::now();
    let mut skipped = 0u64;
    for e in rd {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        match e.ok().and_then(|e| entry(&e)) {
            Some(en) => batch.push(en),
            None => skipped += 1,
        }
        if batch.len() >= BATCH || (!batch.is_empty() && last.elapsed() >= BATCH_TIME) {
            if tx.send(Msg::Batch(std::mem::take(&mut batch))).is_err() {
                return;
            }
            last = Instant::now();
        }
    }
    if !batch.is_empty() && tx.send(Msg::Batch(batch)).is_err() {
        return;
    }
    let note = (skipped > 0).then(|| format!("{skipped} item{} couldn't be read.", if skipped == 1 { "" } else { "s" }));
    let _ = tx.send(Msg::Done(note));
}

/// Read one directory entry's details. Never follows links except to learn whether a link
/// points at a folder.
pub fn entry(e: &fs::DirEntry) -> Option<Entry> {
    let md = e.metadata().ok()?;
    let name: Box<str> = e.file_name().to_string_lossy().into();
    let is_link = md.file_type().is_symlink() || is_reparse(&md);
    let is_dir = if md.file_type().is_symlink() { fs::metadata(e.path()).is_ok_and(|t| t.is_dir()) } else { md.is_dir() };
    Some(Entry {
        hidden: crate::scan::is_hidden(&name, &md),
        readonly: md.permissions().readonly(),
        size: if md.is_file() { md.len() } else { 0 },
        modified: secs(md.modified()),
        created: md.created().ok().map(|t| secs(Ok(t))),
        is_dir,
        is_link,
        name,
    })
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(windows)]
fn is_reparse(md: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    md.file_attributes() & REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse(_md: &fs::Metadata) -> bool {
    false
}

/// A short, plain explanation of why a folder couldn't be read.
pub fn describe(e: &std::io::Error) -> String {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::NotFound => "This folder doesn't exist (any more).".into(),
        K::PermissionDenied => "You don't have permission to open this folder.".into(),
        K::NotADirectory => "This is a file, not a folder.".into(),
        K::TimedOut => "The drive or server didn't answer in time.".into(),
        _ => format!("Couldn't read this folder: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dirmap-listing-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn finish(mut l: Listing) -> Listing {
        let t = Instant::now();
        while !l.done {
            l.poll();
            assert!(t.elapsed() < Duration::from_secs(20), "listing never finished");
            std::thread::sleep(Duration::from_millis(2));
        }
        l
    }

    #[test]
    fn lists_files_and_folders_with_details() {
        let d = tmp("basic");
        fs::create_dir(d.join("sub")).unwrap();
        fs::write(d.join("a.txt"), "hello").unwrap();
        fs::write(d.join(".hidden"), "").unwrap();
        let l = finish(Listing::start(d.clone()));
        assert!(l.error.is_none(), "{:?}", l.error);
        let get = |n: &str| l.entries.iter().find(|e| &*e.name == n).unwrap().clone();
        assert_eq!(l.entries.len(), 3);
        assert!(get("sub").is_dir && get("sub").size == 0);
        let a = get("a.txt");
        assert!(!a.is_dir && a.size == 5 && a.modified > 0 && a.ext() == "txt");
        if cfg!(unix) {
            assert!(get(".hidden").hidden);
        }
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn many_entries_arrive_in_batches() {
        let d = tmp("many");
        for i in 0..20_000 {
            fs::write(d.join(format!("f{i}")), "").unwrap();
        }
        let mut l = Listing::start(d.clone());
        let mut batches = 0;
        let t = Instant::now();
        while !l.done {
            let before = l.entries.len();
            l.poll();
            if l.entries.len() > before {
                batches += 1;
            }
            assert!(t.elapsed() < Duration::from_secs(30));
        }
        assert_eq!(l.entries.len(), 20_000);
        assert!(batches >= 1);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_folder_reports_an_error() {
        let l = finish(Listing::start(std::env::temp_dir().join("dirmap-no-such-folder-xyz")));
        assert!(l.entries.is_empty());
        assert!(l.error.as_deref().unwrap().contains("doesn't exist"));
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let d = tmp("file");
        fs::write(d.join("f"), "").unwrap();
        let l = finish(Listing::start(d.join("f")));
        assert!(l.error.is_some());
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn links_to_folders_open_like_folders() {
        let d = tmp("links");
        fs::create_dir(d.join("real")).unwrap();
        fs::write(d.join("file"), "x").unwrap();
        std::os::unix::fs::symlink(d.join("real"), d.join("to-dir")).unwrap();
        std::os::unix::fs::symlink(d.join("file"), d.join("to-file")).unwrap();
        std::os::unix::fs::symlink(d.join("gone"), d.join("dangling")).unwrap();
        let l = finish(Listing::start(d.clone()));
        let get = |n: &str| l.entries.iter().find(|e| &*e.name == n).unwrap().clone();
        assert!(get("to-dir").is_dir && get("to-dir").is_link);
        assert!(!get("to-file").is_dir && get("to-file").is_link);
        assert!(!get("dangling").is_dir && get("dangling").is_link);
        assert!(!get("real").is_link);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn dropping_cancels_quietly() {
        let d = tmp("drop");
        for i in 0..2_000 {
            fs::write(d.join(format!("f{i}")), "").unwrap();
        }
        drop(Listing::start(d.clone()));
        fs::remove_dir_all(&d).unwrap();
    }
}
