//! Keeping Storage Analysis up to date with changes made after the scan: a download landing in
//! Downloads, a file deleted in Finder, a folder moved in.
//!
//! A thread asks the system for change notices below the scanned folder (FSEvents on macOS and
//! ReadDirectoryChangesW on Windows watch the whole tree at once; on Linux, inotify watches each
//! scanned folder on its own, up to a limit). Notices are gathered until things go quiet, then
//! only the items they name are read again: a file costs one `stat`, and a folder is scanned only
//! when it is new (created or moved in). Nothing is opened, so cloud files stay in the cloud.
//! The fresh items go to the UI, which puts them into the tree with `apply_updates`.

use super::*;
use notify::{EventKind, RecursiveMode, Watcher as _};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};

/// Wait this long after the last notice before reading again…
const QUIET: Duration = Duration::from_millis(900);
/// …but no longer than this while something keeps changing (a download being written).
const MOST: Duration = Duration::from_secs(4);
/// More folders than this aren't watched on Linux (each takes one inotify watch).
#[cfg(target_os = "linux")]
const LINUX_MAX_WATCHES: usize = 30_000;

/// Fresh copies of changed items (`None`: gone), by path on disk.
pub(super) type Batch = Vec<(PathBuf, Option<Node>)>;

/// The watch on the analysis on screen.
pub(super) struct LiveScan {
    /// The analysis it belongs to: its root and when it was scanned.
    pub key: (PathBuf, i64),
    rx: mpsc::Receiver<Batch>,
    /// Set by the UI once a batch is in the tree; the thread sends nothing while it's clear.
    taken: Arc<AtomicBool>,
    /// Least time between batches, in ms, raised when putting one into the tree took long.
    gap_ms: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    /// A batch that arrived while something else was changing the tree.
    held: Option<Batch>,
}

impl Drop for LiveScan {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What notices said about one path.
#[derive(Default, Clone, Copy)]
struct Seen {
    /// Created, moved or renamed (a folder is only scanned again for these).
    arrived: bool,
    /// The system lost track and asks for this folder to be read again.
    rescan: bool,
}

impl LiveScan {
    /// Start watching `root`, whose analysis is `tree`. `opts` are the scan's own, so items
    /// read again follow the same rules (skipped folders, hidden files, cloud drives…).
    pub fn start(root: PathBuf, at: i64, tree: Arc<Node>, opts: Options, ctx: egui::Context) -> LiveScan {
        let (tx, rx) = mpsc::channel();
        let taken = Arc::new(AtomicBool::new(true));
        let gap_ms = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (t, g, s, r) = (taken.clone(), gap_ms.clone(), stop.clone(), root.clone());
        let _ = std::thread::Builder::new().name("filescape-livescan".into()).spawn(move || {
            run(r, tree, opts, tx, t, g, s, move || ctx.request_repaint());
        });
        LiveScan { key: (root, at), rx, taken, gap_ms, stop, held: None }
    }

    /// The next batch to put into the tree, if one is ready.
    pub fn take(&mut self) -> Option<Batch> {
        self.held.take().or_else(|| self.rx.try_recv().ok())
    }

    /// Keep `b` for later (the tree is busy).
    pub fn hold(&mut self, b: Batch) {
        self.held = Some(b);
    }

    /// A batch went into the tree, which took `took`. Wait at least ten times that before the
    /// next one, so a huge analysis stays responsive while a folder in it churns.
    pub fn applied(&self, took: Duration) {
        let ms = (took.as_millis() as u64).saturating_mul(10).min(30_000);
        self.gap_ms.store(ms, Ordering::Relaxed);
        self.taken.store(true, Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    root: PathBuf,
    tree: Arc<Node>,
    opts: Options,
    tx: mpsc::Sender<Batch>,
    taken: Arc<AtomicBool>,
    gap_ms: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    wake: impl Fn(),
) {
    let (ev_tx, ev_rx) = mpsc::channel::<notify::Event>();
    let Ok(mut w) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            let _ = ev_tx.send(ev);
        }
    }) else {
        return;
    };
    // The system may name paths by their real location (/private/var for /var on macOS).
    let real = root.canonicalize().unwrap_or_else(|_| root.clone());
    #[cfg(target_os = "linux")]
    let mut watched = {
        let dirs = tree_dirs(&root, &tree, LINUX_MAX_WATCHES);
        let mut n = 0usize;
        for d in &dirs {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if w.watch(d, RecursiveMode::NonRecursive).is_err() {
                break; // out of inotify watches: what's watched so far stays live
            }
            n += 1;
        }
        n
    };
    #[cfg(not(target_os = "linux"))]
    if w.watch(&root, RecursiveMode::Recursive).is_err() {
        return;
    }
    drop(tree);

    let mut pending: HashMap<PathBuf, Seen> = HashMap::new();
    let mut first = Instant::now();
    let mut last = Instant::now();
    let mut sent = Instant::now() - Duration::from_secs(60);
    while !stop.load(Ordering::Relaxed) {
        match ev_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(ev) => {
                if matches!(ev.kind, EventKind::Access(_)) {
                    continue;
                }
                let arrived = matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)) | EventKind::Any | EventKind::Other);
                let rescan = ev.need_rescan();
                for p in ev.paths {
                    let Some(p) = inside(&p, &root, &real) else { continue };
                    if pending.is_empty() {
                        first = Instant::now();
                    }
                    let s = pending.entry(p).or_default();
                    s.arrived |= arrived;
                    s.rescan |= rescan;
                }
                last = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if pending.is_empty() || !taken.load(Ordering::Acquire) {
            continue;
        }
        let now = Instant::now();
        let quiet = now - last >= QUIET || now - first >= MOST;
        if !quiet || now - sent < Duration::from_millis(gap_ms.load(Ordering::Relaxed)) {
            continue;
        }
        let todo = std::mem::take(&mut pending);
        let batch = reread_all(&root, todo, &opts);
        if batch.is_empty() {
            continue;
        }
        #[cfg(target_os = "linux")]
        for (p, n) in &batch {
            if let Some(n) = n.as_ref().filter(|n| n.is_dir) {
                for d in tree_dirs(p, n, LINUX_MAX_WATCHES.saturating_sub(watched)) {
                    if w.watch(&d, RecursiveMode::NonRecursive).is_ok() {
                        watched += 1;
                    }
                }
            }
        }
        taken.store(false, Ordering::Release);
        sent = Instant::now();
        if tx.send(batch).is_err() {
            return;
        }
        wake();
    }
}

/// `p` as a path below `root` (as the analysis names it), if it is one. The root itself counts.
fn inside(p: &Path, root: &Path, real: &Path) -> Option<PathBuf> {
    if p.starts_with(root) {
        return Some(p.to_path_buf());
    }
    if let Ok(rel) = p.strip_prefix(real) {
        return Some(root.join(rel));
    }
    // FSEvents may name a home folder by where it really is on the data volume.
    #[cfg(target_os = "macos")]
    if let Ok(rel) = p.strip_prefix("/System/Volumes/Data")
        && let Some(q) = inside(&Path::new("/").join(rel), root, real)
    {
        return Some(q);
    }
    None
}

/// The folders of `tree` (found at `at` on disk), widest first, at most `max`. Folders of
/// mounted cloud drives are left out: watching them would ask the service about each one.
#[cfg(target_os = "linux")]
fn tree_dirs(at: &Path, tree: &Node, max: usize) -> Vec<PathBuf> {
    let mounts: Vec<String> = crate::cloud::known().iter().filter(|r| r.kind == crate::cloud::Kind::Mount).map(|r| crate::cloud::norm(&r.path)).collect();
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(at.to_path_buf(), tree)]);
    while let Some((p, n)) = queue.pop_front() {
        if out.len() >= max {
            break;
        }
        if mounts.contains(&crate::cloud::norm(&p)) {
            continue;
        }
        for c in n.children.iter().filter(|c| c.is_dir) {
            queue.push_back((p.join(&*c.name), c));
        }
        out.push(p);
    }
    out
}

/// A download still being written: Chrome's .crdownload, Firefox's .part, Safari's .download
/// bundle (and anything in it). It's left out until the browser gives it its real name.
fn partial_download(rel: &Path) -> bool {
    let n = rel.components().count();
    rel.components().enumerate().any(|(i, c)| {
        let name = c.as_os_str().to_string_lossy().to_lowercase();
        name.ends_with(".download") || (i + 1 == n && [".crdownload", ".part", ".partial", ".opdownload"].iter().any(|x| name.ends_with(x)))
    })
}

/// Read the items notices named, as the scan would see them. A folder is only scanned again if
/// it arrived (or the system asks); items below a folder being scanned are left to that scan.
fn reread_all(root: &Path, todo: HashMap<PathBuf, Seen>, opts: &Options) -> Batch {
    let mut items: Vec<(PathBuf, Seen)> = todo.into_iter().collect();
    items.sort_by_key(|(p, _)| p.components().count());
    let mut scanning: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for (p, seen) in items {
        if scanning.iter().any(|d| p.starts_with(d)) {
            continue;
        }
        let Ok(rel) = p.strip_prefix(root) else { continue };
        let md = std::fs::symlink_metadata(&p);
        let Ok(md) = md else {
            // Gone (deleted, or renamed away). Taking out what isn't in the tree does nothing.
            if !rel.as_os_str().is_empty() {
                out.push((p, None));
            }
            continue;
        };
        if partial_download(rel) {
            continue;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !rel.as_os_str().is_empty() && opts.skip_hidden && scan::is_hidden(&name, &md) {
            continue;
        }
        if md.is_dir() {
            let root_again = rel.as_os_str().is_empty();
            if !(seen.rescan || (seen.arrived && !root_again)) || (!root_again && !admit_folder(root, &p, &md, opts)) {
                continue;
            }
            scanning.push(p.clone());
            let n = super::menus::reread(&p, opts);
            out.push((p, n));
        } else if md.is_file() {
            let modified = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            if opts.file_excluded(&name, md.len(), modified) {
                continue;
            }
            let n = super::menus::reread(&p, opts);
            out.push((p, n));
        }
    }
    out
}

/// Would the scan have walked into this folder? Not if it's skipped in Settings, on another
/// drive (with "Stay on the scanned drive"), or a mounted cloud drive the user didn't ask for.
fn admit_folder(root: &Path, p: &Path, md: &std::fs::Metadata, opts: &Options) -> bool {
    if opts.folder_excluded(p) {
        return false;
    }
    #[cfg(unix)]
    if opts.one_file_system {
        use std::os::unix::fs::MetadataExt;
        if std::fs::metadata(root).is_ok_and(|r| r.dev() != md.dev()) {
            return false;
        }
    }
    #[cfg(not(unix))]
    let _ = (root, md);
    let me = crate::cloud::norm(p);
    !crate::cloud::known().iter().any(|r| r.kind == crate::cloud::Kind::Mount && !opts.cloud_mounts && crate::cloud::norm(&r.path) == me)
}

impl App {
    /// Keep the watch on the analysis on screen running, and put what it found into the tree.
    /// Call once per frame.
    pub(super) fn live_poll(&mut self, ctx: &egui::Context) {
        let wanted = self.settings.watch_changes
            && self.tree.is_some()
            && !crate::fsys::remote::is_remote(&self.scanned_path)
            && !crate::fsys::archive::inside(&self.scanned_path);
        let key = (self.scanned_path.clone(), self.scanned_at);
        if !wanted {
            self.live = None;
            return;
        }
        if self.live.as_ref().is_none_or(|l| l.key != key) {
            let opts = self.options_for(self.scanned_with.as_ref().unwrap_or(&self.settings));
            self.live = Some(LiveScan::start(key.0, key.1, self.tree.clone().unwrap(), opts, ctx.clone()));
            return;
        }
        let Some(batch) = self.live.as_mut().and_then(LiveScan::take) else { return };
        // Not while a scan, a file operation or a duplicate search is using tree positions, nor
        // while duplicate results are up (putting changes in would clear them).
        if self.job.is_some() || self.ops.job.is_some() || self.dup_job.is_some() || !self.dup_groups.is_empty() {
            if let Some(l) = &mut self.live {
                l.hold(batch);
            }
            ctx.request_repaint_after(Duration::from_millis(500));
            return;
        }
        let t = Instant::now();
        // Read with the size shown when the watch began; the switch above the map may have moved.
        let mut batch = batch;
        for n in batch.iter_mut().filter_map(|(_, n)| n.as_mut()) {
            scan::set_metric(n, self.metric);
        }
        self.apply_updates(batch);
        self.dock.invalidate();
        if let Some(l) = &self.live {
            l.applied(t.elapsed());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_downloads_are_left_out() {
        assert!(partial_download(Path::new("Downloads/movie.mp4.crdownload")));
        assert!(partial_download(Path::new("big.iso.part")));
        assert!(partial_download(Path::new("Downloads/x.zip.download/x.zip")));
        assert!(!partial_download(Path::new("Downloads/movie.mp4")));
        assert!(!partial_download(Path::new("parts/notes.txt")));
    }

    #[test]
    fn maps_real_paths_back_to_the_scanned_one() {
        let (root, real) = (Path::new("/var/x"), Path::new("/private/var/x"));
        assert_eq!(inside(Path::new("/private/var/x/a.txt"), root, real), Some(PathBuf::from("/var/x/a.txt")));
        assert_eq!(inside(Path::new("/var/x/a.txt"), root, real), Some(PathBuf::from("/var/x/a.txt")));
        assert_eq!(inside(Path::new("/elsewhere/a.txt"), root, real), None);
    }

    /// A file written into a scanned folder shows up in the batch with its size; a partial
    /// download doesn't, and its final name does once it's renamed.
    #[test]
    fn notices_new_and_finished_downloads() {
        let dir = std::env::temp_dir().join(format!("filescape-livescan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let dir = dir.canonicalize().unwrap();
        let opts = Options::default();
        let tree = Arc::new(scan::scan(&dir, &opts, &Progress::default(), &scan::new_live(&dir)));
        let (tx, rx) = mpsc::channel();
        let (taken, gap, stop) = (Arc::new(AtomicBool::new(true)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
        let (d, t, g, s) = (dir.clone(), taken.clone(), gap.clone(), stop.clone());
        let th = std::thread::spawn(move || run(d, tree, opts, tx, t, g, s, || {}));
        std::thread::sleep(Duration::from_millis(400));

        std::fs::write(dir.join("sub").join("new.bin"), vec![0u8; 5000]).unwrap();
        std::fs::write(dir.join("film.mp4.crdownload"), vec![0u8; 100]).unwrap();
        let wait = |rx: &mpsc::Receiver<Batch>, want: &Path| -> Option<Node> {
            let until = Instant::now() + Duration::from_secs(8);
            while Instant::now() < until {
                if let Ok(b) = rx.recv_timeout(Duration::from_millis(200)) {
                    assert!(b.iter().all(|(p, _)| !p.to_string_lossy().ends_with(".crdownload") || !p.exists()));
                    taken.store(true, Ordering::Release);
                    if let Some((_, n)) = b.into_iter().find(|(p, _)| p == want) {
                        return n;
                    }
                }
            }
            None
        };
        let n = wait(&rx, &dir.join("sub").join("new.bin")).expect("new file noticed");
        assert_eq!(n.logical, 5000);

        std::fs::rename(dir.join("film.mp4.crdownload"), dir.join("film.mp4")).unwrap();
        let n = wait(&rx, &dir.join("film.mp4")).expect("finished download noticed");
        assert_eq!(n.logical, 100);

        std::fs::create_dir(dir.join("fresh")).unwrap();
        std::fs::write(dir.join("fresh").join("a.txt"), b"hello").unwrap();
        let n = wait(&rx, &dir.join("fresh")).expect("new folder noticed");
        assert!(n.is_dir);

        stop.store(true, Ordering::Relaxed);
        th.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
