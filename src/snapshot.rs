//! Saving a scan to disk and comparing a later scan against it ("what changed since then?").

use crate::scan::Node;
use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"DMSNAP1\n";

/// A saved scan: where it was taken, when, and the whole tree.
pub struct Snapshot {
    pub root: PathBuf,
    pub taken: i64,
    pub tree: Node,
}

pub fn save(file: &Path, root: &Path, tree: &Node) -> io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, std::fs::File::create(file)?);
    w.write_all(MAGIC)?;
    write_str(&mut w, &root.to_string_lossy())?;
    w.write_all(&chrono::Utc::now().timestamp().to_le_bytes())?;
    write_node(&mut w, tree)?;
    w.flush()
}

pub fn load(file: &Path) -> io::Result<Snapshot> {
    let mut r = BufReader::with_capacity(1 << 20, std::fs::File::open(file)?);
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a dirmap snapshot"));
    }
    let root = PathBuf::from(read_str(&mut r)?);
    let taken = i64::from_le_bytes(read_arr(&mut r)?);
    let tree = read_node(&mut r)?;
    Ok(Snapshot { root, taken, tree })
}

fn write_str(w: &mut impl Write, s: &str) -> io::Result<()> {
    w.write_all(&(s.len() as u32).to_le_bytes())?;
    w.write_all(s.as_bytes())
}

fn write_node(w: &mut impl Write, n: &Node) -> io::Result<()> {
    write_str(w, &n.name)?;
    w.write_all(&n.size.to_le_bytes())?;
    w.write_all(&n.modified.to_le_bytes())?;
    if n.is_dir {
        w.write_all(&[1])?;
        w.write_all(&(n.children.len() as u32).to_le_bytes())?;
        for c in &n.children {
            write_node(w, c)?;
        }
        Ok(())
    } else {
        w.write_all(&[0])
    }
}

fn read_arr<const N: usize>(r: &mut impl Read) -> io::Result<[u8; N]> {
    let mut b = [0u8; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn read_str(r: &mut impl Read) -> io::Result<String> {
    let len = u32::from_le_bytes(read_arr(r)?) as usize;
    if len > 1 << 16 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "corrupt snapshot"));
    }
    let mut b = vec![0u8; len];
    r.read_exact(&mut b)?;
    String::from_utf8(b).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "corrupt snapshot"))
}

fn read_node(r: &mut impl Read) -> io::Result<Node> {
    let name = read_str(r)?.into_boxed_str();
    let size = u64::from_le_bytes(read_arr(r)?);
    let modified = i64::from_le_bytes(read_arr(r)?);
    let is_dir = read_arr::<1>(r)?[0] == 1;
    let mut children = Vec::new();
    let mut files = 1;
    if is_dir {
        let count = u32::from_le_bytes(read_arr(r)?) as usize;
        children.reserve(count.min(1 << 16));
        for _ in 0..count {
            children.push(read_node(r)?);
        }
        files = children.iter().map(|c| c.files).sum();
    }
    Ok(Node { name, size, files, modified, is_dir, children })
}

/// Size of each current entry in the old scan, laid out like the current tree (same child indices).
pub struct DiffNode {
    /// Size in the snapshot; `NEW` if it didn't exist then.
    pub old: u64,
    pub children: Vec<DiffNode>,
}

pub const NEW: u64 = u64::MAX;

impl DiffNode {
    pub fn at(&self, path: &[u32]) -> Option<&DiffNode> {
        let mut n = self;
        for &i in path {
            n = n.children.get(i as usize)?;
        }
        Some(n)
    }
}

/// One line of the "what changed" list.
pub struct Change {
    /// Path of child indices in the current tree; `None` for something that was deleted.
    pub path: Option<Vec<u32>>,
    /// Display path relative to the scanned folder.
    pub name: String,
    pub is_dir: bool,
    pub old: u64,
    pub new: u64,
}

impl Change {
    pub fn delta(&self) -> i64 {
        self.new as i64 - self.old as i64
    }
}

/// Match the current tree against the snapshot by name. Returns the per-entry old sizes and a
/// list of what changed: new and deleted files and folders (each as one entry), and files whose
/// size changed. Sorted biggest change first.
pub fn diff(cur: &Node, old: &Node) -> (DiffNode, Vec<Change>) {
    let mut changes = Vec::new();
    let mut path = Vec::new();
    let d = diff_into(cur, Some(old), &mut path, &mut String::new(), &mut changes);
    changes.sort_by_key(|c| std::cmp::Reverse(c.delta().unsigned_abs()));
    (d, changes)
}

fn diff_into(cur: &Node, old: Option<&Node>, path: &mut Vec<u32>, name: &mut String, out: &mut Vec<Change>) -> DiffNode {
    let Some(old) = old.filter(|o| o.is_dir == cur.is_dir) else {
        // New since the snapshot: one entry for it, nothing for what's inside.
        out.push(Change { path: Some(path.clone()), name: name.clone(), is_dir: cur.is_dir, old: 0, new: cur.size });
        return mark_new(cur);
    };
    if !cur.is_dir {
        if old.size != cur.size {
            out.push(Change { path: Some(path.clone()), name: name.clone(), is_dir: false, old: old.size, new: cur.size });
        }
        return DiffNode { old: old.size, children: Vec::new() };
    }
    let by_name: HashMap<&str, &Node> = old.children.iter().map(|c| (&*c.name, c)).collect();
    let mut seen = std::collections::HashSet::new();
    let children = cur
        .children
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let o = by_name.get(&*c.name).copied();
            if o.is_some() {
                seen.insert(&*c.name);
            }
            path.push(i as u32);
            let len = name.len();
            push_name(name, &c.name);
            let d = diff_into(c, o, path, name, out);
            name.truncate(len);
            path.pop();
            d
        })
        .collect();
    for o in old.children.iter().filter(|o| !seen.contains(&*o.name)) {
        let mut n = name.clone();
        push_name(&mut n, &o.name);
        out.push(Change { path: None, name: n, is_dir: o.is_dir, old: o.size, new: 0 });
    }
    DiffNode { old: old.size, children }
}

fn push_name(s: &mut String, name: &str) {
    if !s.is_empty() {
        s.push(std::path::MAIN_SEPARATOR);
    }
    s.push_str(name);
}

fn mark_new(n: &Node) -> DiffNode {
    DiffNode { old: NEW, children: n.children.iter().map(mark_new).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> Node {
        Node { name: name.into(), size, files: 1, modified: 0, is_dir: false, children: vec![] }
    }
    fn dir(name: &str, children: Vec<Node>) -> Node {
        let size = children.iter().map(|c| c.size).sum();
        Node { name: name.into(), size, files: 0, modified: 0, is_dir: true, children }
    }

    #[test]
    fn round_trip_and_diff() {
        let old = dir("r", vec![file("a", 10), file("gone", 5), dir("d", vec![file("x", 1)])]);
        let cur = dir("r", vec![file("a", 30), file("b", 7), dir("d", vec![file("x", 1)])]);
        let f = std::env::temp_dir().join(format!("dirmap-test-{}.snap", std::process::id()));
        save(&f, Path::new("/r"), &old).unwrap();
        let snap = load(&f).unwrap();
        std::fs::remove_file(&f).ok();
        assert_eq!(snap.tree.size, 16);
        assert_eq!(snap.tree.children[2].children[0].name.as_ref(), "x");

        let (d, changes) = diff(&cur, &snap.tree);
        assert_eq!(d.children[0].old, 10);
        assert_eq!(d.children[1].old, NEW);
        assert_eq!(d.children[2].children[0].old, 1);
        let summary: Vec<(&str, i64)> = changes.iter().map(|c| (c.name.as_str(), c.delta())).collect();
        assert_eq!(summary, vec![("a", 20), ("b", 7), ("gone", -5)]);
    }
}
