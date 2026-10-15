//! Parallel directory scanner that builds an in-memory size tree.

use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// One file or directory in the scanned tree. Children are sorted by size, largest first.
#[derive(Debug)]
pub struct Node {
    pub name: Box<str>,
    pub size: u64,
    /// Number of files at or below this node (1 for a file).
    pub files: u64,
    pub is_dir: bool,
    pub children: Vec<Node>,
}

impl Node {
    /// Lowercased extension for files, e.g. "rs". Empty if none.
    pub fn ext(&self) -> &str {
        if self.is_dir {
            return "";
        }
        match self.name.rfind('.') {
            Some(i) if i > 0 && i + 1 < self.name.len() => &self.name[i + 1..],
            _ => "",
        }
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

/// Live counters shared between the scanning thread and the UI.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
}

#[derive(Clone, Copy)]
pub struct Options {
    /// Don't descend into other mounted filesystems (e.g. /proc when scanning /).
    pub one_file_system: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { one_file_system: true }
    }
}

pub fn scan(root: &Path, opts: Options, progress: &Arc<Progress>) -> Node {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let dev = device_of(root);
    scan_dir(root.to_path_buf(), name.into(), opts, dev, progress)
}

fn scan_dir(path: PathBuf, name: Box<str>, opts: Options, dev: Option<u64>, p: &Progress) -> Node {
    p.dirs.fetch_add(1, Ordering::Relaxed);
    let entries: Vec<fs::DirEntry> = match fs::read_dir(&path) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => {
            p.errors.fetch_add(1, Ordering::Relaxed);
            Vec::new()
        }
    };

    let mut children: Vec<Node> = entries
        .into_par_iter()
        .filter_map(|e| {
            if p.cancel.load(Ordering::Relaxed) {
                return None;
            }
            // symlink_metadata: never follow symlinks, so no cycles or double counting.
            let md = match e.path().symlink_metadata() {
                Ok(m) => m,
                Err(_) => {
                    p.errors.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            };
            let name: Box<str> = e.file_name().to_string_lossy().into();
            if md.is_dir() {
                if opts.one_file_system && dev.is_some() && dev_of_md(&md) != dev {
                    return None;
                }
                Some(scan_dir(e.path(), name, opts, dev, p))
            } else if md.is_file() {
                let size = md.len();
                p.files.fetch_add(1, Ordering::Relaxed);
                p.bytes.fetch_add(size, Ordering::Relaxed);
                Some(Node { name, size, files: 1, is_dir: false, children: Vec::new() })
            } else {
                None // symlinks, sockets, devices
            }
        })
        .collect();

    children.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    children.shrink_to_fit();
    let size = children.iter().map(|c| c.size).sum();
    let files = children.iter().map(|c| c.files).sum();
    Node { name, size, files, is_dir: true, children }
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

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}
