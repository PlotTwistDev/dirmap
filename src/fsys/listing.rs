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
    /// Bytes for files; 0 for folders.
    pub size: u64,
    /// Space on disk for files (see `scan::on_disk`); 0 for folders.
    pub alloc: u64,
    /// `scan::store` bits: whether a cloud file is downloaded.
    pub flags: u8,
    /// Seconds since the Unix epoch, 0 when unknown.
    pub modified: i64,
}

impl Entry {
    pub fn ext(&self) -> &str {
        if self.is_dir { "" } else { crate::scan::ext_of(&self.name) }
    }

    /// The file's size by the metric chosen in the app (see `scan::metric`).
    pub fn shown_size(&self) -> u64 {
        crate::scan::metric().pick(self.size, self.alloc, self.flags)
    }

    /// Reading this file would download it from the cloud.
    pub fn not_local(&self) -> bool {
        self.flags & crate::scan::store::NOT_LOCAL != 0
    }
}

/// Sent from the reading thread.
enum Msg {
    Batch(Vec<Entry>),
    Done(Option<String>),
}

/// A folder being read (or read).
pub struct Listing {
    /// Shared so a large folder can be sorted on another thread without copying it.
    pub entries: Arc<Vec<Entry>>,
    pub done: bool,
    /// Why the folder couldn't be read (it may still have delivered some entries).
    pub error: Option<String>,
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
        let c = cancel.clone();
        let spawned = std::thread::Builder::new().name("dirmap-list".into()).spawn(move || read(&path, &c, &tx));
        let mut l = Listing {
            entries: Arc::new(Vec::new()),
            done: false,
            error: None,
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

    /// Search `path` and every folder below it for names matching `query` (as the filter box
    /// matches). Each result's name is its path below `path`. Hidden folders are only searched
    /// when `hidden`.
    /// With `tagged`, items match by being in that set instead of by name; without `deep`
    /// only the folder itself is looked in.
    pub fn search(
        path: PathBuf,
        query: String,
        hidden: bool,
        deep: bool,
        tagged: Option<Arc<std::collections::HashSet<PathBuf>>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("dirmap-search".into())
            .spawn(move || search(&path, &query, hidden, deep, tagged.as_deref(), &c, &tx));
        let mut l = Listing {
            entries: Arc::new(Vec::new()),
            done: false,
            error: None,
            started: Instant::now(),
            generation: 0,
            rx,
            cancel,
        };
        if let Err(e) = spawned {
            l.done = true;
            l.error = Some(format!("Couldn't start searching: {e}"));
        }
        l
    }

    /// Take whatever has arrived. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Batch(b)) => {
                    Arc::make_mut(&mut self.entries).extend(b);
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
    // A folder on a server.
    if super::remote::is_remote(path) {
        let msg = match super::remote::list(path) {
            Ok(entries) => {
                let _ = tx.send(Msg::Batch(entries));
                Msg::Done(None)
            }
            Err(e) => Msg::Done(Some(e)),
        };
        let _ = tx.send(msg);
        return;
    }
    // Inside an archive (or the archive itself): its table of contents.
    if let Some((archive, inner)) = super::archive::split(path)
        && archive.is_file()
    {
        let msg = match super::archive::list(&archive, &inner) {
            Ok(entries) => {
                let _ = tx.send(Msg::Batch(entries));
                Msg::Done(None)
            }
            Err(e) => Msg::Done(Some(e)),
        };
        let _ = tx.send(msg);
        return;
    }
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

/// Most results a search gathers.
const FOUND_MAX: usize = 100_000;

/// A filter that also keeps only some kinds of file (a `categories::Category` bit mask), in
/// one string so it travels with the text through sorting and searching: `\u{1}bits\u{1}text`.
pub fn pack_filter(text: &str, kinds: u16) -> String {
    if kinds == 0 { text.to_string() } else { format!("\u{1}{kinds}\u{1}{text}") }
}

/// The text and kinds of a filter made by `pack_filter`.
pub fn unpack_filter(f: &str) -> (&str, u16) {
    if let Some(rest) = f.strip_prefix('\u{1}')
        && let Some((bits, text)) = rest.split_once('\u{1}')
    {
        return (text, bits.parse().unwrap_or(0));
    }
    (f, 0)
}

/// Whether an item passes a filter made by `pack_filter`. With kinds chosen, folders don't.
pub fn entry_matches(filter: &str, name: &str, is_dir: bool) -> bool {
    let (text, kinds) = unpack_filter(filter);
    (kinds == 0 || (!is_dir && kinds & crate::categories::of(name).bit() != 0)) && name_matches(text, name)
}

/// Whether a name matches what's typed in the filter box: wildcards (`*`, `?`) match the whole
/// name, anything else is found anywhere in it; case is ignored.
pub fn name_matches(filter: &str, name: &str) -> bool {
    let f = filter.trim();
    if f.is_empty() {
        return true;
    }
    if f.contains(['*', '?']) {
        crate::scan::glob(f, name)
    } else {
        name.to_lowercase().contains(&f.to_lowercase())
    }
}

fn search(
    root: &Path,
    query: &str,
    hidden: bool,
    deep: bool,
    tagged: Option<&std::collections::HashSet<PathBuf>>,
    cancel: &AtomicBool,
    tx: &mpsc::Sender<Msg>,
) {
    let remote = super::remote::is_remote(root);
    let sep = if remote {
        '/'
    } else {
        std::path::MAIN_SEPARATOR
    };
    let mut todo = vec![(root.to_path_buf(), String::new())];
    let mut batch = Vec::new();
    let mut last = Instant::now();
    let mut found = 0usize;
    let mut unread = 0u64;
    while let Some((dir, rel)) = todo.pop() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let items: Vec<Entry> = if remote {
            match super::remote::list(&dir) {
                Ok(v) => v,
                Err(e) if rel.is_empty() => {
                    let _ = tx.send(Msg::Done(Some(e)));
                    return;
                }
                Err(_) => {
                    unread += 1;
                    continue;
                }
            }
        } else {
            match fs::read_dir(&dir) {
                Ok(rd) => rd.flatten().filter_map(|e| entry(&e)).collect(),
                Err(e) if rel.is_empty() => {
                    let _ = tx.send(Msg::Done(Some(describe(&e))));
                    return;
                }
                Err(_) => {
                    unread += 1;
                    continue;
                }
            }
        };
        for e in items {
            let path = if rel.is_empty() {
                e.name.to_string()
            } else {
                format!("{rel}{sep}{}", e.name)
            };
            if deep && e.is_dir && !e.is_link && (hidden || !e.hidden) {
                todo.push((dir.join(&*e.name), path.clone()));
            }
            let hit = match tagged {
                Some(set) => set.contains(&dir.join(&*e.name)),
                None => entry_matches(query, &e.name, e.is_dir),
            };
            if hit {
                found += 1;
                batch.push(Entry {
                    name: path.into(),
                    ..e
                });
            }
        }
        if !batch.is_empty() && (batch.len() >= BATCH || last.elapsed() >= BATCH_TIME) {
            if tx.send(Msg::Batch(std::mem::take(&mut batch))).is_err() {
                return;
            }
            last = Instant::now();
        }
        if found >= FOUND_MAX {
            let _ = tx.send(Msg::Batch(batch));
            let _ = tx.send(Msg::Done(Some(format!(
                "Stopped at {FOUND_MAX} results: type more to narrow the search."
            ))));
            return;
        }
    }
    let _ = tx.send(Msg::Batch(batch));
    let note = (unread > 0).then(|| {
        format!(
            "{unread} folder{} couldn't be searched.",
            if unread == 1 { "" } else { "s" }
        )
    });
    let _ = tx.send(Msg::Done(note));
}

/// Read one directory entry's details. Never follows links except to learn whether a link
/// points at a folder.
pub fn entry(e: &fs::DirEntry) -> Option<Entry> {
    let md = e.metadata().ok()?;
    let name: Box<str> = e.file_name().to_string_lossy().into();
    // Links and junctions only. Cloud placeholders (OneDrive, Dropbox…) are reparse points too,
    // but the standard library tells them apart by their reparse tag and calls them files.
    let is_link = md.file_type().is_symlink();
    let is_dir = if md.file_type().is_symlink() { fs::metadata(e.path()).is_ok_and(|t| t.is_dir()) } else { md.is_dir() };
    // Metadata only, as the scan does: listing a cloud folder never downloads its files.
    let (alloc, mut flags) = if md.is_file() { crate::scan::on_disk(&md, || e.path(), cluster_for(e)) } else { (0, 0) };
    if md.is_file() {
        flags |= cloud_for(e);
    }
    Some(Entry {
        hidden: crate::scan::is_hidden(&name, &md),
        size: if md.is_file() { md.len() } else { 0 },
        alloc,
        flags,
        modified: secs(md.modified()),
        is_dir,
        is_link,
        name,
    })
}

/// The drive's allocation unit for an entry, worked out once per folder (Windows only).
#[cfg(windows)]
fn cluster_for(e: &fs::DirEntry) -> u64 {
    use std::cell::RefCell;
    thread_local!(static LAST: RefCell<(PathBuf, u64)> = RefCell::new((PathBuf::new(), 0)));
    let path = e.path();
    let dir = path.parent().unwrap_or(&path).to_path_buf();
    LAST.with_borrow_mut(|(d, c)| {
        if *d != dir || *c == 0 {
            *c = crate::scan::cluster_of(&dir);
            *d = dir;
        }
        *c
    })
}

#[cfg(not(windows))]
fn cluster_for(_e: &fs::DirEntry) -> u64 {
    512
}

/// `scan::store` bits for files in the cloud folder this entry is in, worked out once per folder.
fn cloud_for(e: &fs::DirEntry) -> u8 {
    use std::cell::RefCell;
    thread_local!(static LAST: RefCell<(PathBuf, u8)> = RefCell::new((PathBuf::new(), 0)));
    let path = e.path();
    let dir = path.parent().unwrap_or(&path);
    LAST.with_borrow_mut(|(d, bits)| {
        if d.as_path() != dir {
            *bits = crate::cloud::root_of(&crate::cloud::roots(), dir).map_or(0, |r| r.bits());
            *d = dir.to_path_buf();
        }
        *bits
    })
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
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
