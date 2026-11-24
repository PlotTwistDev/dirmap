//! Saving a scan to disk and comparing a later scan against it ("what changed since then?").

use crate::scan::{Metric, Node, store};
use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// Version 1 (earlier versions) stored one size per item, the logical size.
const MAGIC_V1: &[u8; 8] = b"DMSNAP1\n";
/// Version 2 stores both sizes and the storage state of each file.
const MAGIC: &[u8; 8] = b"DMSNAP2\n";

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
    let v2 = match &magic {
        m if m == MAGIC => true,
        m if m == MAGIC_V1 => false,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "not a Filescape snapshot")),
    };
    let root = PathBuf::from(read_str(&mut r)?);
    let taken = i64::from_le_bytes(read_arr(&mut r)?);
    let tree = read_node(&mut r, v2)?;
    Ok(Snapshot { root, taken, tree })
}

fn write_str(w: &mut impl Write, s: &str) -> io::Result<()> {
    w.write_all(&(s.len() as u32).to_le_bytes())?;
    w.write_all(s.as_bytes())
}

fn write_node(w: &mut impl Write, n: &Node) -> io::Result<()> {
    write_str(w, &n.name)?;
    if n.is_dir {
        w.write_all(&[1])?;
        w.write_all(&(n.children.len() as u32).to_le_bytes())?;
        for c in &n.children {
            write_node(w, c)?;
        }
        Ok(())
    } else {
        w.write_all(&[0, n.flags])?;
        w.write_all(&n.logical.to_le_bytes())?;
        w.write_all(&n.alloc.to_le_bytes())?;
        w.write_all(&n.modified.to_le_bytes())
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

fn read_node(r: &mut impl Read, v2: bool) -> io::Result<Node> {
    let name = read_str(r)?.into_boxed_str();
    if v2 {
        if read_arr::<1>(r)?[0] == 1 {
            let count = u32::from_le_bytes(read_arr(r)?) as usize;
            return read_children(r, name, count, v2);
        }
        let [flags] = read_arr::<1>(r)?;
        let logical = u64::from_le_bytes(read_arr(r)?);
        let alloc = u64::from_le_bytes(read_arr(r)?);
        let modified = i64::from_le_bytes(read_arr(r)?);
        return Ok(Node::file(name, logical, alloc, flags, modified, Metric::Disk));
    }
    // Version 1: one size (logical), and no size on disk.
    let size = u64::from_le_bytes(read_arr(r)?);
    let modified = i64::from_le_bytes(read_arr(r)?);
    if read_arr::<1>(r)?[0] == 1 {
        let count = u32::from_le_bytes(read_arr(r)?) as usize;
        let mut n = read_children(r, name, count, v2)?;
        n.flags |= store::NO_ALLOC;
        return Ok(n);
    }
    Ok(Node::plain(name, size, modified))
}

fn read_children(r: &mut impl Read, name: Box<str>, count: usize, v2: bool) -> io::Result<Node> {
    let mut children = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        children.push(read_node(r, v2)?);
    }
    Ok(Node::folder(name, children))
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

/// Whether sizes on disk can be compared with this snapshot (saved by an earlier version: no).
pub fn has_alloc(old: &Node) -> bool {
    old.flags & store::NO_ALLOC == 0
}

/// Match the current tree against the snapshot by name, comparing sizes by `m`. Returns the
/// per-entry old sizes and a list of what changed: new and deleted files and folders (each as
/// one entry), and files whose size changed. Sorted biggest change first.
pub fn diff(cur: &Node, old: &Node, m: Metric) -> (DiffNode, Vec<Change>) {
    let mut changes = Vec::new();
    let mut path = Vec::new();
    let d = diff_into(cur, Some(old), &mut path, &mut String::new(), &mut changes, m);
    changes.sort_by_key(|c| std::cmp::Reverse(c.delta().unsigned_abs()));
    (d, changes)
}

fn diff_into(cur: &Node, old: Option<&Node>, path: &mut Vec<u32>, name: &mut String, out: &mut Vec<Change>, m: Metric) -> DiffNode {
    let Some(old) = old.filter(|o| o.is_dir == cur.is_dir) else {
        // New since the snapshot: one entry for it, nothing for what's inside.
        out.push(Change { path: Some(path.clone()), name: name.clone(), is_dir: cur.is_dir, old: 0, new: cur.size_by(m) });
        return mark_new(cur);
    };
    if !cur.is_dir {
        if old.size_by(m) != cur.size_by(m) {
            out.push(Change { path: Some(path.clone()), name: name.clone(), is_dir: false, old: old.size_by(m), new: cur.size_by(m) });
        }
        return DiffNode { old: old.size_by(m), children: Vec::new() };
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
            let d = diff_into(c, o, path, name, out, m);
            name.truncate(len);
            path.pop();
            d
        })
        .collect();
    for o in old.children.iter().filter(|o| !seen.contains(&*o.name)) {
        let mut n = name.clone();
        push_name(&mut n, &o.name);
        out.push(Change { path: None, name: n, is_dir: o.is_dir, old: o.size_by(m), new: 0 });
    }
    DiffNode { old: old.size_by(m), children }
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
        Node::file(name.into(), size, size / 2, 0, 0, Metric::Logical)
    }
    fn dir(name: &str, children: Vec<Node>) -> Node {
        let mut d = Node::folder(name.into(), children);
        // Keep the order given, as the tests index children by position.
        d.children.sort_by_key(|c| c.name.clone());
        d
    }

    #[test]
    fn round_trip_and_diff() {
        let old = dir("r", vec![file("a", 10), file("gone", 5), dir("d", vec![file("x", 1)])]);
        let cur = dir("r", vec![file("a", 30), file("b", 7), dir("d", vec![file("x", 1)])]);
        let f = std::env::temp_dir().join(format!("dirmap-test-{}.snap", std::process::id()));
        save(&f, Path::new("/r"), &old).unwrap();
        let snap = load(&f).unwrap();
        std::fs::remove_file(&f).ok();
        assert_eq!((snap.tree.logical, snap.tree.alloc), (16, 7));
        assert!(has_alloc(&snap.tree));
        let x = snap.tree.children.iter().find(|c| &*c.name == "d").unwrap();
        assert_eq!(x.children[0].name.as_ref(), "x");

        let (d, changes) = diff(&cur, &snap.tree, Metric::Logical);
        assert_eq!(d.children[0].old, 10);
        assert_eq!(d.children[1].old, NEW);
        assert_eq!(d.children[2].children[0].old, 1);
        let summary: Vec<(&str, i64)> = changes.iter().map(|c| (c.name.as_str(), c.delta())).collect();
        assert_eq!(summary, vec![("a", 20), ("b", 7), ("gone", -5)]);
        let (_, changes) = diff(&cur, &snap.tree, Metric::Disk);
        let summary: Vec<(&str, i64)> = changes.iter().map(|c| (c.name.as_str(), c.delta())).collect();
        assert_eq!(summary, vec![("a", 10), ("b", 3), ("gone", -2)]);
    }

    #[test]
    fn version_1_snapshots_still_load_without_size_on_disk() {
        // name "r", size, modified, dir, 2 children: file "a" (10 bytes), file "b" (5 bytes)
        let mut b = MAGIC_V1.to_vec();
        let s = |b: &mut Vec<u8>, t: &str| {
            b.extend((t.len() as u32).to_le_bytes());
            b.extend(t.as_bytes());
        };
        s(&mut b, "/r");
        b.extend(7i64.to_le_bytes());
        s(&mut b, "r");
        b.extend(15u64.to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.push(1);
        b.extend(2u32.to_le_bytes());
        for (n, z) in [("a", 10u64), ("b", 5)] {
            s(&mut b, n);
            b.extend(z.to_le_bytes());
            b.extend(0i64.to_le_bytes());
            b.push(0);
        }
        let f = std::env::temp_dir().join(format!("dirmap-test-v1-{}.snap", std::process::id()));
        std::fs::write(&f, b).unwrap();
        let snap = load(&f).unwrap();
        std::fs::remove_file(&f).ok();
        assert_eq!((snap.taken, snap.tree.logical, snap.tree.files), (7, 15, 2));
        assert!(!has_alloc(&snap.tree));
        assert_eq!(&*snap.tree.children[0].name, "a");
    }
}
