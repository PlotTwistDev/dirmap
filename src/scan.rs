//! Parallel directory scanner that builds an in-memory size tree.

use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// One file or directory in the scanned tree. Children are sorted by `size`, largest first.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: Box<str>,
    /// The size every view shows, sorts, lays out and totals by: `logical` or `alloc`, whichever
    /// the chosen [`Metric`] says (see [`set_metric`]). For a folder, the total of its children.
    pub size: u64,
    /// Bytes of data: what Explorer calls "Size" and `ls -l` shows. Never changes with the metric.
    pub logical: u64,
    /// Space allocated on the drive: "Size on disk" in Explorer, `du` on macOS and Linux. A cloud
    /// file that isn't downloaded takes (almost) none. Never changes with the metric.
    pub alloc: u64,
    /// Number of files at or below this node (1 for a file).
    pub files: u64,
    /// Last modification, seconds since the Unix epoch (newest file inside, for folders).
    pub modified: i64,
    pub is_dir: bool,
    /// [`store`] bits: a file's own storage state; for a folder, every state found inside it.
    pub flags: u8,
    pub children: Vec<Node>,
}

/// Which size the views show, sort and total by. Switching doesn't need a rescan: both sizes
/// are collected for every file (see [`set_metric`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Metric {
    /// Space actually used on this computer's drive (cloud files that aren't downloaded take
    /// none). What frees space, and what `du` reports.
    #[default]
    Disk,
    /// The files' own sizes, wherever the data is (what Explorer calls "Size").
    Logical,
}

impl Metric {
    pub fn pick(self, logical: u64, alloc: u64) -> u64 {
        match self {
            Metric::Disk => alloc,
            Metric::Logical => logical,
        }
    }

    /// Short name, as on the switch and column headers.
    pub fn label(self) -> &'static str {
        match self {
            Metric::Disk => "Size on disk",
            Metric::Logical => "Logical size",
        }
    }
}

/// Storage state bits in [`Node::flags`] and `fsys::listing::Entry::flags`.
pub mod store {
    /// A cloud file with none of its data on this computer (iCloud/Google Drive "dataless",
    /// OneDrive "online-only"). Opening it downloads it.
    pub const ONLINE_ONLY: u8 = 1;
    /// A cloud file with only part of its data downloaded (Windows).
    pub const PARTIAL: u8 = 2;
    /// The size on disk couldn't be read and is an estimate.
    pub const ESTIMATED: u8 = 4;
    /// No size-on-disk information at all: a server scan or a snapshot saved by an older
    /// version. `alloc` is a copy of `logical` here.
    pub const NO_ALLOC: u8 = 8;
    /// Reading this file's contents would download it.
    pub const NOT_LOCAL: u8 = ONLINE_ONLY | PARTIAL;
}

static METRIC: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The metric chosen in the app, for code that shows sizes but doesn't hold a tree (file lists).
/// Trees are converted explicitly with [`set_metric`].
pub fn metric() -> Metric {
    if METRIC.load(Ordering::Relaxed) == 1 { Metric::Logical } else { Metric::Disk }
}

pub fn set_shown_metric(m: Metric) {
    METRIC.store(m as u8, Ordering::Relaxed);
}

impl Node {
    /// A file with both its sizes; `size` follows `m`.
    pub fn file(name: Box<str>, logical: u64, alloc: u64, flags: u8, modified: i64, m: Metric) -> Node {
        Node { name, size: m.pick(logical, alloc), logical, alloc, files: 1, modified, is_dir: false, flags, children: Vec::new() }
    }

    /// A file whose size on disk isn't known (servers, old snapshots, tests): both sizes are `size`.
    pub fn plain(name: Box<str>, size: u64, modified: i64) -> Node {
        Node { name, size, logical: size, alloc: size, files: 1, modified, is_dir: false, flags: store::NO_ALLOC, children: Vec::new() }
    }

    /// A folder holding `children`, sorted and totalled as the scan does.
    pub fn folder(name: Box<str>, mut children: Vec<Node>) -> Node {
        sort_children(&mut children);
        children.shrink_to_fit();
        let mut n = Node { name, size: 0, logical: 0, alloc: 0, files: 0, modified: 0, is_dir: true, flags: 0, children };
        n.resum();
        n
    }

    /// Recompute a folder's totals from its children (which must already be sorted).
    pub fn resum(&mut self) {
        let (mut size, mut logical, mut alloc, mut files, mut modified, mut flags) = (0u64, 0u64, 0u64, 0u64, 0i64, 0u8);
        for c in &self.children {
            size += c.size;
            logical += c.logical;
            alloc += c.alloc;
            files += c.files;
            modified = modified.max(c.modified);
            flags |= c.flags;
        }
        (self.size, self.logical, self.alloc, self.files, self.modified, self.flags) = (size, logical, alloc, files, modified, flags);
    }

    /// The size under `m`, whatever the tree is sorted by.
    pub fn size_by(&self, m: Metric) -> u64 {
        m.pick(self.logical, self.alloc)
    }

    /// Extension for files, e.g. "rs". Empty if none.
    pub fn ext(&self) -> &str {
        if self.is_dir { "" } else { ext_of(&self.name) }
    }

    /// Walk a path of child indices from this node.
    pub fn at(&self, path: &[u32]) -> Option<&Node> {
        let mut n = self;
        for &i in path {
            n = n.children.get(i as usize)?;
        }
        Some(n)
    }
}

/// Largest first, then by name: the order every view relies on.
pub fn sort_children(children: &mut [Node]) {
    children.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
}

/// Make `n` (and everything below it) show, sort and total by `m`. Positions of children change,
/// so paths of child indices into the tree must be translated by the caller. Runs in parallel;
/// a few hundred milliseconds for millions of files.
pub fn set_metric(n: &mut Node, m: Metric) {
    if !n.is_dir {
        n.size = m.pick(n.logical, n.alloc);
        return;
    }
    if n.children.iter().filter(|c| c.is_dir).nth(1).is_some() {
        n.children.par_iter_mut().for_each(|c| set_metric(c, m));
    } else {
        n.children.iter_mut().for_each(|c| set_metric(c, m));
    }
    sort_children(&mut n.children);
    n.size = n.children.iter().map(|c| c.size).sum();
}

pub fn ext_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => &name[i + 1..],
        _ => "",
    }
}

/// Live counters shared between the scanning thread and the UI.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    /// Logical bytes found so far.
    pub bytes: AtomicU64,
    /// Bytes on disk found so far.
    pub alloc: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
    /// Set once the walk is done and the result is being assembled.
    pub finishing: AtomicBool,
    /// A folder currently being read, for display only.
    pub current: Mutex<PathBuf>,
}

/// A folder as the scan sees it while still running. The UI reads these concurrently to show
/// the tree filling in; sizes are totals of everything found below so far.
pub struct LiveDir {
    pub name: Box<str>,
    /// Logical bytes found below so far.
    pub size: AtomicU64,
    /// Bytes on disk found below so far.
    pub alloc: AtomicU64,
    pub files: AtomicU64,
    pub modified: AtomicI64,
    /// This folder and everything under it has been read.
    pub done: AtomicBool,
    pub kids: Mutex<LiveKids>,
}

#[derive(Default)]
pub struct LiveKids {
    pub dirs: Vec<Arc<LiveDir>>,
    pub files: Vec<LiveFile>,
}

pub struct LiveFile {
    pub name: Box<str>,
    /// Logical size.
    pub size: u64,
    pub alloc: u64,
    pub flags: u8,
    pub modified: i64,
}

impl LiveDir {
    /// Bytes found below so far, by `m`.
    pub fn size_by(&self, m: Metric) -> u64 {
        match m {
            Metric::Disk => self.alloc.load(Ordering::Relaxed),
            Metric::Logical => self.size.load(Ordering::Relaxed),
        }
    }
}

impl LiveDir {
    fn new(name: Box<str>) -> Self {
        Self {
            name,
            size: AtomicU64::new(0),
            alloc: AtomicU64::new(0),
            files: AtomicU64::new(0),
            modified: AtomicI64::new(0),
            done: AtomicBool::new(false),
            kids: Mutex::new(LiveKids::default()),
        }
    }
}

/// What to scan and what to leave out.
#[derive(Clone, Default)]
pub struct Options {
    /// Don't descend into other mounted filesystems (e.g. /proc when scanning /).
    pub one_file_system: bool,
    pub skip_hidden: bool,
    pub exclude_folders: Vec<String>,
    pub exclude_files: Vec<String>,
    pub include_files: Vec<String>,
    pub min_file_size: u64,
    /// Files modified before this (seconds since epoch) are skipped; 0 = off.
    pub min_modified: i64,
    /// 0 = one per CPU core.
    pub threads: usize,
    /// What the finished tree is sorted and totalled by.
    pub metric: Metric,
}

impl Options {
    fn folder_excluded(&self, path: &Path) -> bool {
        if self.exclude_folders.is_empty() {
            return false;
        }
        let p = norm_path(&path.to_string_lossy());
        self.exclude_folders.iter().any(|x| {
            let x = norm_path(x);
            !x.is_empty() && (p == x || p.starts_with(&format!("{x}/")))
        })
    }

    fn file_excluded(&self, name: &str, size: u64, modified: i64) -> bool {
        size < self.min_file_size
            || (self.min_modified != 0 && modified < self.min_modified)
            || self.exclude_files.iter().any(|g| glob(g, name))
            || (!self.include_files.is_empty() && !self.include_files.iter().any(|g| glob(g, name)))
    }
}

fn norm_path(s: &str) -> String {
    let s = s.trim().replace('\\', "/");
    let s = s.trim_end_matches('/');
    if cfg!(windows) { s.to_lowercase() } else { s.to_string() }
}

/// Case-insensitive glob match supporting `*` and `?`.
pub fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.trim().to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Ancestors of the folder being scanned, as a linked list on the stack. Sizes found in a
/// folder are added to every ancestor so the live tree always shows correct totals.
struct Chain<'a> {
    dir: &'a LiveDir,
    up: Option<&'a Chain<'a>>,
}

impl Chain<'_> {
    fn add(&self, bytes: u64, alloc: u64, files: u64, modified: i64) {
        let mut c = Some(self);
        while let Some(link) = c {
            link.dir.size.fetch_add(bytes, Ordering::Relaxed);
            link.dir.alloc.fetch_add(alloc, Ordering::Relaxed);
            link.dir.files.fetch_add(files, Ordering::Relaxed);
            link.dir.modified.fetch_max(modified, Ordering::Relaxed);
            c = link.up;
        }
    }
}

/// Scan `root`, filling in `live` as it goes, and return the finished tree.
pub fn scan(root: &Path, opts: &Options, progress: &Progress, live: &LiveDir) -> Node {
    let fs = SameFs::of(root);
    let run = || scan_dir(root.to_path_buf(), live, None, opts, &fs, progress);
    match (opts.threads > 0).then(|| rayon::ThreadPoolBuilder::new().num_threads(opts.threads).build()) {
        Some(Ok(pool)) => pool.install(run),
        _ => run(),
    }
    progress.finishing.store(true, Ordering::Relaxed);
    freeze(live, opts.metric)
}

/// Analyse a folder on a server (SFTP, FTP, WebDAV): read every folder below it there, then
/// build the tree. Nothing shows until it's done, but the counts move as it goes.
pub fn scan_remote(root: &Path, progress: &Progress) -> Node {
    use crate::fsys::remote;
    use std::collections::HashMap;
    let mut by_dir: HashMap<String, Vec<(String, remote::Meta)>> = HashMap::new();
    let walked = remote::walk(root, &progress.cancel, &mut |rel, items| {
        progress.dirs.fetch_add(1, Ordering::Relaxed);
        for (_, m) in items {
            if !m.is_dir {
                progress.files.fetch_add(1, Ordering::Relaxed);
                progress.bytes.fetch_add(m.len, Ordering::Relaxed);
                progress.alloc.fetch_add(m.len, Ordering::Relaxed);
            }
        }
        if let Ok(mut c) = progress.current.lock() {
            *c = if rel.is_empty() { root.to_path_buf() } else { root.join(rel) };
        }
        by_dir.insert(rel.to_string(), items.to_vec());
    });
    if walked.is_err() {
        progress.errors.fetch_add(1, Ordering::Relaxed);
    }
    progress.finishing.store(true, Ordering::Relaxed);
    fn build(name: Box<str>, rel: &str, by_dir: &HashMap<String, Vec<(String, remote::Meta)>>) -> Node {
        // Servers don't say how much space a file takes on their disks: both sizes are its length.
        let children: Vec<Node> = by_dir
            .get(rel)
            .map(|items| {
                items
                    .iter()
                    .map(|(n, m)| {
                        if m.is_dir && !m.is_link {
                            let r = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
                            build(n.as_str().into(), &r, by_dir)
                        } else {
                            let mut f = Node::plain(n.as_str().into(), if m.is_dir { 0 } else { m.len }, m.modified);
                            if m.is_dir {
                                f.is_dir = true;
                                f.files = 0;
                            }
                            f
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut n = Node::folder(name, children);
        n.flags |= store::NO_ALLOC;
        n
    }
    build(root_name(root), "", &by_dir)
}

pub fn root_name(root: &Path) -> Box<str> {
    root.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
        .into()
}

pub fn new_live(root: &Path) -> Arc<LiveDir> {
    Arc::new(LiveDir::new(root_name(root)))
}

enum Entry {
    Dir(PathBuf, Box<str>),
    File(LiveFile),
}

fn scan_dir(path: PathBuf, dir: &LiveDir, up: Option<&Chain>, opts: &Options, sfs: &SameFs, p: &Progress) {
    if p.cancel.load(Ordering::Relaxed) {
        dir.done.store(true, Ordering::Release);
        return;
    }
    p.dirs.fetch_add(1, Ordering::Relaxed);
    // try_lock: never make scanner threads wait on each other just to update a label.
    if let Ok(mut cur) = p.current.try_lock() {
        cur.clone_from(&path);
    }
    let entries: Vec<fs::DirEntry> = match fs::read_dir(&path) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => {
            p.errors.fetch_add(1, Ordering::Relaxed);
            Vec::new()
        }
    };

    let found: Vec<Entry> = entries
        .into_par_iter()
        .filter_map(|e| {
            if p.cancel.load(Ordering::Relaxed) {
                return None;
            }
            // DirEntry::metadata never follows symlinks (no cycles or double counting), and on
            // Windows it comes from the directory listing itself, so it costs no extra syscall.
            // Junctions report as symlinks and are skipped too.
            let md = match e.metadata() {
                Ok(m) => m,
                Err(_) => {
                    p.errors.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            };
            let name: Box<str> = e.file_name().to_string_lossy().into();
            if opts.skip_hidden && is_hidden(&name, &md) {
                return None;
            }
            if md.is_dir() {
                if opts.one_file_system && !sfs.contains(&md) {
                    return None;
                }
                let child = e.path();
                if sfs.skip.as_deref() == Some(child.as_path()) || opts.folder_excluded(&child) {
                    return None;
                }
                Some(Entry::Dir(child, name))
            } else if md.is_file() {
                let size = md.len();
                let modified = mtime(&md);
                if opts.file_excluded(&name, size, modified) {
                    return None;
                }
                // Metadata only: nothing here opens the file, so cloud files stay in the cloud.
                let (mut alloc, flags) = on_disk(&md, || e.path(), sfs.cluster);
                if sfs.linked_again(&md) {
                    alloc = 0; // another name for a file already counted: the disk holds it once
                }
                Some(Entry::File(LiveFile { name, size, alloc, flags, modified }))
            } else {
                None // symlinks, sockets, devices
            }
        })
        .collect();

    let mut subdirs = Vec::new();
    let mut files = Vec::new();
    for e in found {
        match e {
            Entry::Dir(path, name) => subdirs.push((path, Arc::new(LiveDir::new(name)))),
            Entry::File(f) => files.push(f),
        }
    }
    let bytes: u64 = files.iter().map(|f| f.size).sum();
    let alloc: u64 = files.iter().map(|f| f.alloc).sum();
    let newest = files.iter().map(|f| f.modified).max().unwrap_or(0);
    let count = files.len() as u64;
    p.files.fetch_add(count, Ordering::Relaxed);
    p.bytes.fetch_add(bytes, Ordering::Relaxed);
    p.alloc.fetch_add(alloc, Ordering::Relaxed);
    {
        let mut k = dir.kids.lock().unwrap();
        k.files = files;
        k.dirs = subdirs.iter().map(|(_, d)| d.clone()).collect();
    }
    let chain = Chain { dir, up };
    chain.add(bytes, alloc, count, newest);

    subdirs.into_par_iter().for_each(|(path, d)| {
        scan_dir(path, &d, Some(&chain), opts, sfs, p);
    });
    dir.done.store(true, Ordering::Release);
}

/// Turn the live tree into the compact, sorted tree the UI browses. Empties `live`.
fn freeze(live: &LiveDir, m: Metric) -> Node {
    let kids = std::mem::take(&mut *live.kids.lock().unwrap());
    let mut children: Vec<Node> = kids.dirs.par_iter().map(|d| freeze(d, m)).collect();
    children.extend(kids.files.into_iter().map(|f| Node::file(f.name, f.size, f.alloc, f.flags, f.modified, m)));
    Node::folder(live.name.clone(), children)
}

fn mtime(md: &fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(windows)]
pub(crate) fn is_hidden(_name: &str, md: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    md.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
pub(crate) fn is_hidden(name: &str, _md: &fs::Metadata) -> bool {
    name.starts_with('.')
}

/// A file's space on disk and storage state (`store` bits), from metadata already in hand.
/// Never opens the file or reads its data, so it can't make a cloud provider download it.
/// `path` is only asked for on Windows, for compressed, sparse and cloud files.
#[cfg(unix)]
pub fn on_disk(md: &fs::Metadata, _path: impl FnOnce() -> PathBuf, _cluster: u64) -> (u64, u8) {
    use std::os::unix::fs::MetadataExt;
    // st_blocks counts 512-byte units on every Unix, whatever the filesystem's block size.
    let alloc = md.blocks().saturating_mul(512);
    #[allow(unused_mut)]
    let mut flags = 0;
    #[cfg(target_os = "macos")]
    {
        // SF_DATALESS: a File Provider file (iCloud Drive, Google Drive, OneDrive, Dropbox…) whose
        // contents are only in the cloud. Reading it would download it.
        const SF_DATALESS: u32 = 0x4000_0000;
        if std::os::macos::fs::MetadataExt::st_flags(md) & SF_DATALESS != 0 {
            flags |= store::ONLINE_ONLY;
        }
    }
    (alloc, flags)
}

#[cfg(windows)]
pub fn on_disk(md: &fs::Metadata, path: impl FnOnce() -> PathBuf, cluster: u64) -> (u64, u8) {
    use std::os::windows::fs::MetadataExt;
    const SPARSE: u32 = 0x200;
    const COMPRESSED: u32 = 0x800;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    let a = md.file_attributes();
    let len = md.len();
    let round = |n: u64| n.div_ceil(cluster.max(1)) * cluster.max(1);
    // Cloud Files placeholders (OneDrive, Dropbox, Box, iCloud for Windows…) say whether their
    // data is here in the attributes the folder listing already returned.
    let cloud = a & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0;
    if a & RECALL_ON_OPEN != 0 {
        return (0, store::ONLINE_ONLY); // not even opened for its size: that alone fetches it
    }
    if !cloud && a & (SPARSE | COMPRESSED) == 0 {
        return (round(len), 0);
    }
    // GetCompressedFileSizeW opens for attributes only, which doesn't download anything.
    match win::compressed_size(&path()) {
        Some(n) => {
            let flags = match (cloud, n) {
                (false, _) => 0,
                (true, 0) => store::ONLINE_ONLY,
                (true, n) if n < len => store::PARTIAL,
                _ => 0,
            };
            (round(n), flags)
        }
        None if cloud => (0, store::ONLINE_ONLY | store::ESTIMATED),
        None => (round(len), store::ESTIMATED),
    }
}

#[cfg(not(any(unix, windows)))]
pub fn on_disk(md: &fs::Metadata, _path: impl FnOnce() -> PathBuf, _cluster: u64) -> (u64, u8) {
    (md.len(), store::ESTIMATED)
}

/// The allocation unit of the drive holding `path` (Windows rounds every file up to it).
#[cfg(windows)]
pub fn cluster_of(path: &Path) -> u64 {
    win::cluster(path).unwrap_or(4096)
}

#[cfg(not(windows))]
pub fn cluster_of(_path: &Path) -> u64 {
    512
}

#[cfg(windows)]
mod win {
    use std::collections::HashMap;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use windows_sys::Win32::Storage::FileSystem::{GetCompressedFileSizeW, GetDiskFreeSpaceW, GetVolumePathNameW, INVALID_FILE_SIZE};

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    pub fn compressed_size(path: &Path) -> Option<u64> {
        let w = wide(path);
        let mut high = 0u32;
        // SAFETY: `w` is NUL-terminated and outlives the call; `high` is a valid out pointer.
        let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
        if low == INVALID_FILE_SIZE && std::io::Error::last_os_error().raw_os_error() != Some(0) {
            return None;
        }
        Some(((high as u64) << 32) | low as u64)
    }

    /// Cluster size of the volume holding `path`, cached per volume.
    pub fn cluster(path: &Path) -> Option<u64> {
        static CACHE: Mutex<Option<HashMap<PathBuf, u64>>> = Mutex::new(None);
        let w = wide(path);
        let mut vol = vec![0u16; 1024];
        // SAFETY: buffers are valid for the lengths given.
        if unsafe { GetVolumePathNameW(w.as_ptr(), vol.as_mut_ptr(), vol.len() as u32) } == 0 {
            return None;
        }
        let end = vol.iter().position(|&c| c == 0).unwrap_or(vol.len());
        let key = PathBuf::from(String::from_utf16_lossy(&vol[..end]));
        if let Some(&c) = CACHE.lock().ok()?.get_or_insert_with(HashMap::new).get(&key) {
            return Some(c);
        }
        let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
        // SAFETY: `vol` is NUL-terminated; the out pointers are valid.
        if unsafe { GetDiskFreeSpaceW(vol.as_ptr(), &mut spc, &mut bps, &mut free, &mut total) } == 0 {
            return None;
        }
        let c = (spc as u64 * bps as u64).max(512);
        CACHE.lock().ok()?.get_or_insert_with(HashMap::new).insert(key, c);
        Some(c)
    }
}

/// Which filesystems count as "the scanned drive" when staying on one drive.
struct SameFs {
    /// Device ids; empty when the platform doesn't have them (then nothing is skipped).
    devs: Vec<u64>,
    /// A folder never to walk into, because its contents are already reached another way.
    skip: Option<PathBuf>,
    /// Allocation unit of the scanned drive (Windows).
    cluster: u64,
    /// Files with more than one name (hard links) already counted, by (device, inode).
    #[cfg_attr(not(unix), allow(dead_code))]
    links: Mutex<std::collections::HashSet<(u64, u64)>>,
}

impl SameFs {
    /// True for the second and later names of a hard-linked file, whose space is already counted.
    #[cfg(unix)]
    fn linked_again(&self, md: &fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        md.nlink() > 1 && !self.links.lock().unwrap().insert((md.dev(), md.ino()))
    }

    #[cfg(not(unix))]
    fn linked_again(&self, _md: &fs::Metadata) -> bool {
        false
    }

    fn of(root: &Path) -> Self {
        #[allow(unused_mut)]
        let mut s = SameFs { devs: device_of(root).into_iter().collect(), skip: None, cluster: cluster_of(root), links: Default::default() };
        // On macOS the read-only system volume (/) and the data volume (/System/Volumes/Data)
        // are joined by firmlinks: /Users, /Applications and so on really live on the data
        // volume. Count both as the startup drive, and don't walk the data volume a second time
        // through its own mount point.
        #[cfg(target_os = "macos")]
        {
            let data = Path::new("/System/Volumes/Data");
            if s.devs.first().is_some_and(|&d| Some(d) == device_of(Path::new("/"))) {
                s.devs.extend(device_of(data));
                s.skip = Some(data.to_path_buf());
            }
        }
        s
    }

    fn contains(&self, md: &fs::Metadata) -> bool {
        self.devs.is_empty() || dev_of_md(md).is_some_and(|d| self.devs.contains(&d))
    }
}

fn device_of(path: &Path) -> Option<u64> {
    fs::symlink_metadata(path).ok().and_then(|m| dev_of_md(&m))
}

#[cfg(unix)]
fn dev_of_md(md: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(md.dev())
}

#[cfg(not(unix))]
fn dev_of_md(_md: &fs::Metadata) -> Option<u64> {
    None
}

/// How many cloud files a tree holds that aren't (fully) on this computer.
#[derive(Default, Debug, PartialEq)]
pub struct CloudTally {
    pub online_only: u64,
    pub partial: u64,
    /// Logical bytes of the online-only and partly downloaded files: what opening them all
    /// could download.
    pub online_bytes: u64,
}

pub fn cloud_tally(n: &Node) -> CloudTally {
    let mut t = CloudTally::default();
    fn walk(n: &Node, t: &mut CloudTally) {
        if n.flags & store::NOT_LOCAL == 0 {
            return; // nothing below: skip the whole folder
        }
        if n.is_dir {
            n.children.iter().for_each(|c| walk(c, t));
        } else {
            if n.flags & store::ONLINE_ONLY != 0 {
                t.online_only += 1;
            } else {
                t.partial += 1;
            }
            t.online_bytes += n.logical.saturating_sub(n.alloc);
        }
    }
    walk(n, &mut t);
    t
}

/// Per-extension totals, sorted by bytes descending.
pub struct ExtStat {
    pub ext: String,
    pub bytes: u64,
    pub files: u64,
}

pub fn ext_stats(root: &Node) -> Vec<ExtStat> {
    let mut map: std::collections::HashMap<&str, (u64, u64)> = Default::default();
    fn walk<'a>(n: &'a Node, map: &mut std::collections::HashMap<&'a str, (u64, u64)>) {
        if n.is_dir {
            for c in &n.children {
                walk(c, map);
            }
        } else {
            let e = map.entry(n.ext()).or_default();
            e.0 += n.size;
            e.1 += 1;
        }
    }
    walk(root, &mut map);
    let mut v: Vec<ExtStat> = map
        .into_iter()
        .map(|(ext, (bytes, files))| ExtStat { ext: ext.to_lowercase(), bytes, files })
        .collect();
    // Merge entries that differ only by case (e.g. JPG and jpg).
    v.sort_by(|a, b| a.ext.cmp(&b.ext));
    v.dedup_by(|b, a| {
        if a.ext == b.ext {
            a.bytes += b.bytes;
            a.files += b.files;
            true
        } else {
            false
        }
    });
    v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    v
}

static DECIMAL: AtomicBool = AtomicBool::new(false);

/// Choose between 1024-based (default) and 1000-based size display.
pub fn set_decimal_units(on: bool) {
    DECIMAL.store(on, Ordering::Relaxed);
}

pub fn human(bytes: u64) -> String {
    let (base, units) = if DECIMAL.load(Ordering::Relaxed) {
        (1000.0, ["B", "kB", "MB", "GB", "TB", "PB"])
    } else {
        (1024.0, ["B", "KB", "MB", "GB", "TB", "PB"])
    };
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= base && u < units.len() - 1 {
        v /= base;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", units[u])
    }
}

/// Parse a size typed by the user: "500" (MB), "1.5 GB", "200k", "10 MB". Units follow the
/// binary/decimal setting.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let num: f64 = s[..split].trim().parse().ok()?;
    let base: f64 = if DECIMAL.load(Ordering::Relaxed) { 1000.0 } else { 1024.0 };
    let unit = s[split..].trim().trim_end_matches("ib").trim_end_matches('b');
    let mult = match unit {
        "" if split == s.len() => base * base, // a bare number means MB
        "" => 1.0,
        "k" => base,
        "m" => base * base,
        "g" => base.powi(3),
        "t" => base.powi(4),
        _ => return None,
    };
    Some((num * mult) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("filescape-scan-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn scan_of(dir: &Path, m: Metric) -> Node {
        let opts = Options { metric: m, ..Default::default() };
        scan(dir, &opts, &Progress::default(), &new_live(dir))
    }

    /// A sparse file stands in for a cloud placeholder: big logical size, (almost) nothing on
    /// disk. The scan keeps both, and the metric decides which the tree is sorted and totalled by.
    #[cfg(unix)]
    #[test]
    fn keeps_logical_and_on_disk_sizes() {
        let d = temp("sizes");
        fs::File::create(d.join("placeholder.bin")).unwrap().set_len(1 << 30).unwrap();
        fs::write(d.join("real.dat"), vec![7u8; 256 << 10]).unwrap();
        let disk = scan_of(&d, Metric::Disk);
        let logical = scan_of(&d, Metric::Logical);
        fs::remove_dir_all(&d).ok();

        for t in [&disk, &logical] {
            assert_eq!(t.logical, (1 << 30) + (256 << 10));
            assert!(t.alloc >= 256 << 10 && t.alloc < 1 << 20, "on disk: {}", t.alloc);
        }
        assert_eq!(disk.size, disk.alloc);
        assert_eq!(&*disk.children[0].name, "real.dat");
        assert_eq!(logical.size, logical.logical);
        assert_eq!(&*logical.children[0].name, "placeholder.bin");

        // Switching re-sorts and re-totals without rescanning, and back again.
        let mut t = disk.clone();
        set_metric(&mut t, Metric::Logical);
        assert_eq!((t.size, &*t.children[0].name), (logical.size, "placeholder.bin"));
        set_metric(&mut t, Metric::Disk);
        assert_eq!((t.size, &*t.children[0].name), (disk.size, "real.dat"));
    }

    /// Like `du`, a file with several names takes its space once.
    #[cfg(unix)]
    #[test]
    fn hard_links_take_space_once() {
        let d = temp("links");
        fs::write(d.join("a"), vec![1u8; 64 << 10]).unwrap();
        fs::hard_link(d.join("a"), d.join("b")).unwrap();
        let t = scan_of(&d, Metric::Disk);
        fs::remove_dir_all(&d).ok();
        assert_eq!(t.logical, 128 << 10);
        assert!(t.alloc >= 64 << 10 && t.alloc < 128 << 10, "on disk: {}", t.alloc);
    }

    #[test]
    fn folders_total_and_flag_what_is_inside() {
        let m = Metric::Disk;
        let cloud = Node::file("c.mov".into(), 900, 0, store::ONLINE_ONLY, 5, m);
        let local = Node::file("l.txt".into(), 100, 4096, 0, 9, m);
        let sub = Node::folder("sub".into(), vec![cloud]);
        let mut root = Node::folder("r".into(), vec![sub, local]);
        assert_eq!((root.logical, root.alloc, root.size, root.files, root.modified), (1000, 4096, 4096, 2, 9));
        assert_eq!(root.flags & store::ONLINE_ONLY, store::ONLINE_ONLY);
        assert_eq!(&*root.children[0].name, "l.txt");
        assert_eq!(cloud_tally(&root), CloudTally { online_only: 1, partial: 0, online_bytes: 900 });
        set_metric(&mut root, Metric::Logical);
        assert_eq!((root.size, &*root.children[0].name), (1000, "sub"));
        // Unknown sizes on disk are marked, never silently zero.
        let p = Node::plain("x".into(), 50, 0);
        assert_eq!((p.alloc, p.flags & store::NO_ALLOC), (50, store::NO_ALLOC));
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("2"), Some(2 << 20));
        assert_eq!(parse_size("1.5 GB"), Some(3 << 29));
        assert_eq!(parse_size("200k"), Some(200 << 10));
        assert_eq!(parse_size("10 B"), Some(10));
        assert_eq!(parse_size("lots"), None);
    }

    #[test]
    fn globs() {
        assert!(glob("*.tmp", "a.TMP"));
        assert!(glob("~$*", "~$doc.docx"));
        assert!(glob("a?c", "abc"));
        assert!(!glob("*.tmp", "a.tmp.txt"));
        assert!(glob("*", ""));
    }
}
