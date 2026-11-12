//! Finding duplicate files: same size, then same first 64 KB, then same full contents.

use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::Hasher;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A file to check: where it is in the tree, where it is on disk, and how big it is.
#[derive(Clone)]
pub struct Candidate {
    pub tree_path: Vec<u32>,
    pub path: PathBuf,
    pub size: u64,
}

/// Files with identical contents.
pub struct Group {
    pub size: u64,
    pub files: Vec<Candidate>,
}

impl Group {
    /// Space that deleting all but one copy would free.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

#[derive(Default)]
pub struct Progress {
    pub to_read: AtomicU64,
    pub read: AtomicU64,
    pub cancel: AtomicBool,
}

pub fn find(candidates: Vec<Candidate>, progress: &Progress) -> Vec<Group> {
    // 1. Only files that share their size with another file can be duplicates.
    let mut by_size: HashMap<u64, Vec<Candidate>> = HashMap::new();
    for c in candidates {
        by_size.entry(c.size).or_default().push(c);
    }
    let same_size: Vec<Vec<Candidate>> = by_size.into_values().filter(|g| g.len() > 1).collect();

    // 2. Then the first 64 KB, which tells most same-size files apart cheaply.
    const HEAD: u64 = 64 * 1024;
    let head_bytes: u64 = same_size.iter().flatten().map(|c| c.size.min(HEAD)).sum();
    progress.to_read.store(head_bytes, Ordering::Relaxed);
    let same_head: Vec<Vec<Candidate>> = same_size
        .into_par_iter()
        .flat_map_iter(|g| split_by(g, |c| hash_file(c, HEAD, progress)))
        .collect();

    // 3. Then the whole file, for groups where a 64 KB head isn't the whole file.
    let rest: u64 = same_head.iter().flatten().filter(|c| c.size > HEAD).map(|c| c.size).sum();
    progress.to_read.fetch_add(rest, Ordering::Relaxed);
    let mut groups: Vec<Group> = same_head
        .into_par_iter()
        .flat_map_iter(|g| {
            if g[0].size <= HEAD { vec![g] } else { split_by(g, |c| hash_file(c, u64::MAX, progress)) }
        })
        .map(|files| Group { size: files[0].size, files })
        .collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.wasted()));
    groups
}

/// Split a group by a key computed per file (in parallel), keeping sub-groups of 2 or more.
/// Files that can't be read (key `None`) are dropped.
fn split_by(g: Vec<Candidate>, key: impl Fn(&Candidate) -> Option<u64> + Sync) -> Vec<Vec<Candidate>> {
    let keys: Vec<Option<u64>> = g.par_iter().map(&key).collect();
    let mut out: HashMap<u64, Vec<Candidate>> = HashMap::new();
    for (c, k) in g.into_iter().zip(keys) {
        if let Some(k) = k {
            out.entry(k).or_default().push(c);
        }
    }
    out.into_values().filter(|g| g.len() > 1).collect()
}

fn hash_file(c: &Candidate, limit: u64, progress: &Progress) -> Option<u64> {
    if progress.cancel.load(Ordering::Relaxed) {
        return None;
    }
    let mut f = std::fs::File::open(&c.path).ok()?.take(limit);
    let mut h = std::hash::DefaultHasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.write(&buf[..n]);
        progress.read.fetch_add(n as u64, Ordering::Relaxed);
        if progress.cancel.load(Ordering::Relaxed) {
            return None;
        }
    }
    Some(h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_identical_files() {
        let dir = std::env::temp_dir().join(format!("dirmap-dupes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let mut other = big.clone();
        *other.last_mut().unwrap() ^= 1; // same size and head, different tail
        let files = [("a", &big), ("b", &big), ("c", &other), ("d", &big[..1000].to_vec())];
        let mut cands = Vec::new();
        for (i, (name, data)) in files.iter().enumerate() {
            let p = dir.join(name);
            std::fs::write(&p, data).unwrap();
            cands.push(Candidate { tree_path: vec![i as u32], path: p, size: data.len() as u64 });
        }
        let groups = find(cands, &Progress::default());
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(groups.len(), 1);
        let mut names: Vec<u32> = groups[0].files.iter().map(|c| c.tree_path[0]).collect();
        names.sort();
        assert_eq!(names, vec![0, 1]);
        assert_eq!(groups[0].wasted(), 200_000);
    }
}
