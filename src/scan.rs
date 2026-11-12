//! Parallel directory scanner that builds an in-memory size tree.

use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// One file or directory in the scanned tree. Children are sorted by size, largest first.
#[derive(Debug)]
pub struct Node {
    pub name: Box<str>,
    pub size: u64,
    /// Number of files at or below this node (1 for a file).
    pub files: u64,
    /// Last modification, seconds since the Unix epoch (newest file inside, for folders).
    pub modified: i64,
    pub is_dir: bool,
    pub children: Vec<Node>,
}

impl Node {
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
    pub bytes: AtomicU64,
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
    pub size: AtomicU64,
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
    pub size: u64,
    pub modified: i64,
}

impl LiveDir {
    fn new(name: Box<str>) -> Self {
        Self {
            name,
            size: AtomicU64::new(0),
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
    fn add(&self, bytes: u64, files: u64, modified: i64) {
        let mut c = Some(self);
        while let Some(link) = c {
            link.dir.size.fetch_add(bytes, Ordering::Relaxed);
            link.dir.files.fetch_add(files, Ordering::Relaxed);
            link.dir.modified.fetch_max(modified, Ordering::Relaxed);
            c = link.up;
        }
    }
}

/// Scan `root`, filling in `live` as it goes, and return the finished tree.
pub fn scan(root: &Path, opts: &Options, progress: &Progress, live: &LiveDir) -> Node {
    let dev = device_of(root);
    let run = || scan_dir(root.to_path_buf(), live, None, opts, dev, progress);
    match (opts.threads > 0).then(|| rayon::ThreadPoolBuilder::new().num_threads(opts.threads).build()) {
        Some(Ok(pool)) => pool.install(run),
        _ => run(),
    }
    progress.finishing.store(true, Ordering::Relaxed);
    freeze(live)
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

fn scan_dir(path: PathBuf, dir: &LiveDir, up: Option<&Chain>, opts: &Options, dev: Option<u64>, p: &Progress) {
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
                if opts.one_file_system && dev.is_some() && dev_of_md(&md) != dev {
                    return None;
                }
                let child = e.path();
                if opts.folder_excluded(&child) {
                    return None;
                }
                Some(Entry::Dir(child, name))
            } else if md.is_file() {
                let size = md.len();
                let modified = mtime(&md);
                if opts.file_excluded(&name, size, modified) {
                    return None;
                }
                Some(Entry::File(LiveFile { name, size, modified }))
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
    let newest = files.iter().map(|f| f.modified).max().unwrap_or(0);
    let count = files.len() as u64;
    p.files.fetch_add(count, Ordering::Relaxed);
    p.bytes.fetch_add(bytes, Ordering::Relaxed);
    {
        let mut k = dir.kids.lock().unwrap();
        k.files = files;
        k.dirs = subdirs.iter().map(|(_, d)| d.clone()).collect();
    }
    let chain = Chain { dir, up };
    chain.add(bytes, count, newest);

    subdirs.into_par_iter().for_each(|(path, d)| {
        scan_dir(path, &d, Some(&chain), opts, dev, p);
    });
    dir.done.store(true, Ordering::Release);
}

/// Turn the live tree into the compact, sorted tree the UI browses. Empties `live`.
fn freeze(live: &LiveDir) -> Node {
    let kids = std::mem::take(&mut *live.kids.lock().unwrap());
    let mut children: Vec<Node> = kids.dirs.par_iter().map(|d| freeze(d)).collect();
    children.extend(kids.files.into_iter().map(|f| Node {
        name: f.name,
        size: f.size,
        files: 1,
        modified: f.modified,
        is_dir: false,
        children: Vec::new(),
    }));
    children.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    children.shrink_to_fit();
    let size = children.iter().map(|c| c.size).sum();
    let files = children.iter().map(|c| c.files).sum();
    let modified = children.iter().map(|c| c.modified).max().unwrap_or(0);
    Node { name: live.name.clone(), size, files, modified, is_dir: true, children }
}

fn mtime(md: &fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(windows)]
fn is_hidden(_name: &str, md: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    md.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _md: &fs::Metadata) -> bool {
    name.starts_with('.')
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
    use super::{glob, parse_size};

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
