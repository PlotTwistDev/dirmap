//! "Show matches only": a copy of the scanned tree holding just the files that pass a filter
//! and the folders that contain them, plus a map back to the original tree's paths.
//!
//! Built on a background thread from the existing scan data (never the disk), in parallel, and
//! abandoned as soon as a newer filter supersedes it.

use crate::scan::Node;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// For one folder of the filtered tree: where each child came from in the original folder,
/// and the same map for each child that is a folder.
#[derive(Debug, Default)]
pub struct Back {
    /// `orig[i]` is the original index of filtered child `i`.
    pub orig: Vec<u32>,
    /// (filtered index, map) for each child folder, in filtered order.
    pub dirs: Vec<(u32, Back)>,
}

impl Back {
    fn sub(&self, filtered: u32) -> Option<&Back> {
        self.dirs.binary_search_by_key(&filtered, |d| d.0).ok().map(|i| &self.dirs[i].1)
    }

    /// The original tree's path for a path in the filtered tree. Stops early on a bad path.
    pub fn to_full(&self, path: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(path.len());
        let mut b = Some(self);
        for &i in path {
            let Some(m) = b else { break };
            let Some(&o) = m.orig.get(i as usize) else { break };
            out.push(o);
            b = m.sub(i);
        }
        out
    }

    /// The filtered tree's path for an original path: the deepest part of it that's still in
    /// the filtered tree, and whether that's all of it.
    pub fn to_view(&self, path: &[u32]) -> (Vec<u32>, bool) {
        let mut out = Vec::with_capacity(path.len());
        let mut b = Some(self);
        for &o in path {
            let Some(m) = b else { return (out, false) };
            let Some(i) = m.orig.iter().position(|&x| x == o) else { return (out, false) };
            out.push(i as u32);
            b = m.sub(i as u32);
        }
        (out, true)
    }
}

/// Copy the files of `root` that `keep` accepts, with the folders above them. Folders left
/// without matches are dropped; the root always stays. None if cancelled.
pub fn subset<F>(root: &Node, keep: &F, cancel: &AtomicBool) -> Option<(Node, Back)>
where
    F: Fn(&Node) -> bool + Sync,
{
    Some(walk(root, keep, cancel)?.unwrap_or_else(|| (empty_like(root), Back::default())))
}

fn empty_like(dir: &Node) -> Node {
    Node { name: dir.name.clone(), size: 0, files: 0, modified: 0, is_dir: true, children: Vec::new() }
}

/// The filtered folder and its map, or None inside if nothing in it matched. None if cancelled.
fn walk<F>(dir: &Node, keep: &F, cancel: &AtomicBool) -> Option<Option<(Node, Back)>>
where
    F: Fn(&Node) -> bool + Sync,
{
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let kids = if dir.children.len() > 64 || dir.children.iter().any(|c| c.is_dir) {
        dir.children.par_iter().enumerate().map(|(i, c)| child(i, c, keep, cancel)).collect::<Option<Vec<_>>>()?
    } else {
        dir.children.iter().enumerate().map(|(i, c)| child(i, c, keep, cancel)).collect::<Option<Vec<_>>>()?
    };
    let mut kids: Vec<(u32, Node, Option<Back>)> = kids.into_iter().flatten().collect();
    if kids.is_empty() {
        return Some(None);
    }
    // Same order the scanner uses: largest first, then by name.
    kids.sort_unstable_by(|a, b| b.1.size.cmp(&a.1.size).then_with(|| a.1.name.cmp(&b.1.name)));
    let mut back = Back { orig: Vec::with_capacity(kids.len()), dirs: Vec::new() };
    let mut children = Vec::with_capacity(kids.len());
    for (k, (orig, node, sub)) in kids.into_iter().enumerate() {
        back.orig.push(orig);
        if let Some(s) = sub {
            back.dirs.push((k as u32, s));
        }
        children.push(node);
    }
    let size = children.iter().map(|c| c.size).sum();
    let files = children.iter().map(|c| c.files).sum();
    let modified = children.iter().map(|c| c.modified).max().unwrap_or(0);
    Some(Some((Node { name: dir.name.clone(), size, files, modified, is_dir: true, children }, back)))
}

/// One child: Some(None) = dropped, Some(Some(..)) = kept, None = cancelled.
fn child<F>(i: usize, c: &Node, keep: &F, cancel: &AtomicBool) -> Option<Option<(u32, Node, Option<Back>)>>
where
    F: Fn(&Node) -> bool + Sync,
{
    if c.is_dir {
        Some(walk(c, keep, cancel)?.map(|(n, b)| (i as u32, n, Some(b))))
    } else if keep(c) {
        let n = Node { name: c.name.clone(), size: c.size, files: 1, modified: c.modified, is_dir: false, children: Vec::new() };
        Some(Some((i as u32, n, None)))
    } else {
        Some(None)
    }
}

/// Where the matches are, for Highlight mode: per folder that holds any, how many and how
/// big, and the last child index holding one (children are sorted largest first, so "any match
/// among children `from..`" is just `last >= from`). Folders without matches have no entry.
#[derive(Debug, Default)]
pub struct Hits {
    pub files: u64,
    pub bytes: u64,
    /// Highest child index that is a match or holds one (meaningless when `files` is 0).
    pub last: u32,
    /// (child index, its hits) for child folders holding matches, by index.
    pub dirs: Vec<(u32, Hits)>,
    /// Matching bytes per lowercased extension (only on the root: the whole result).
    pub exts: Vec<(String, u64)>,
}

impl Hits {
    /// The entry for a folder by its path, None if nothing under it matches.
    pub fn at(&self, path: &[u32]) -> Option<&Hits> {
        let mut h = self;
        for &i in path {
            h = h.dirs.binary_search_by_key(&i, |d| d.0).ok().map(|k| &h.dirs[k].1)?;
        }
        (h.files > 0).then_some(h)
    }

    /// Does any child from index `from` on match or hold a match?
    pub fn any_from(&self, from: usize) -> bool {
        self.files > 0 && self.last as usize >= from
    }

    /// The extension with the most matching bytes.
    pub fn main_ext(&self) -> Option<&str> {
        self.exts.iter().max_by_key(|e| e.1).map(|e| e.0.as_str())
    }
}

/// Find where the files `keep` accepts are. None if cancelled.
pub fn hits<F>(root: &Node, keep: &F, cancel: &AtomicBool) -> Option<Hits>
where
    F: Fn(&Node) -> bool + Sync,
{
    let mut exts = Vec::new();
    let mut h = hits_in(root, keep, cancel, &mut exts)?;
    h.exts = exts;
    Some(h)
}

fn tally(exts: &mut Vec<(String, u64)>, ext: &str, bytes: u64) {
    match exts.iter_mut().find(|e| e.0.eq_ignore_ascii_case(ext)) {
        Some(e) => e.1 += bytes,
        None => exts.push((ext.to_ascii_lowercase(), bytes)),
    }
}

fn hits_in<F>(dir: &Node, keep: &F, cancel: &AtomicBool, exts: &mut Vec<(String, u64)>) -> Option<Hits>
where
    F: Fn(&Node) -> bool + Sync,
{
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let mut h = Hits::default();
    // Files here, on this thread.
    for (i, c) in dir.children.iter().enumerate() {
        if !c.is_dir && keep(c) {
            h.files += 1;
            h.bytes += c.size;
            h.last = h.last.max(i as u32);
            tally(exts, c.ext(), c.size);
        }
    }
    // Subfolders in parallel.
    let subs: Vec<(u32, &Node)> = dir.children.iter().enumerate().filter(|(_, c)| c.is_dir).map(|(i, c)| (i as u32, c)).collect();
    if subs.is_empty() {
        return Some(h);
    }
    let found: Vec<(u32, Hits, Vec<(String, u64)>)> = subs
        .par_iter()
        .map(|&(i, c)| {
            let mut e = Vec::new();
            hits_in(c, keep, cancel, &mut e).map(|sub| (i, sub, e))
        })
        .collect::<Option<Vec<_>>>()?;
    for (i, sub, e) in found {
        for (ext, b) in e {
            tally(exts, &ext, b);
        }
        if sub.files > 0 {
            h.files += sub.files;
            h.bytes += sub.bytes;
            h.last = h.last.max(i);
            h.dirs.push((i, sub));
        }
    }
    h.dirs.sort_unstable_by_key(|d| d.0);
    Some(h)
}

/// Number and total size of the files under `root` that `keep` accepts. None if cancelled.
#[cfg(test)]
pub fn count<F>(root: &Node, keep: &F, cancel: &AtomicBool) -> Option<(u64, u64)>
where
    F: Fn(&Node) -> bool + Sync,
{
    if !root.is_dir {
        return Some(if keep(root) { (1, root.size) } else { (0, 0) });
    }
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let add = |a: (u64, u64), b: (u64, u64)| (a.0 + b.0, a.1 + b.1);
    if root.children.iter().any(|c| c.is_dir) {
        root.children.par_iter().map(|c| count(c, keep, cancel)).try_reduce(|| (0, 0), |a, b| Some(add(a, b)))
    } else {
        Some(root.children.iter().filter(|c| keep(c)).fold((0, 0), |a, c| add(a, (1, c.size))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{Filter, Options};

    fn file(name: &str, size: u64) -> Node {
        Node { name: name.into(), size, files: 1, modified: size as i64, is_dir: false, children: Vec::new() }
    }
    fn dir(name: &str, mut children: Vec<Node>) -> Node {
        children.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        Node {
            name: name.into(),
            size: children.iter().map(|c| c.size).sum(),
            files: children.iter().map(|c| c.files).sum(),
            modified: children.iter().map(|c| c.modified).max().unwrap_or(0),
            is_dir: true,
            children,
        }
    }

    fn sample() -> Node {
        dir(
            "root",
            vec![
                dir(
                    "shots",
                    vec![
                        dir("sh010", (1001..=1010).map(|f| file(&format!("comp.{f}.exr"), 100 + f % 7)).collect()),
                        dir("sh020", vec![file("comp.1001.png", 5000), file("notes.txt", 10)]),
                    ],
                ),
                dir("empty", vec![]),
                file("big.iso", 1 << 20),
                file("preview.exr", 3),
            ],
        )
    }

    /// Every file in `n`, with its path.
    fn files(n: &Node, path: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, String, u64)>) {
        if !n.is_dir {
            out.push((path.clone(), n.name.to_string(), n.size));
            return;
        }
        for (i, c) in n.children.iter().enumerate() {
            path.push(i as u32);
            files(c, path, out);
            path.pop();
        }
    }

    fn check_consistent(full: &Node, f: &Filter) {
        let keep = |n: &Node| f.matches(&n.name);
        let never = AtomicBool::new(false);
        let (sub, back) = subset(full, &keep, &never).unwrap();
        let (count_files, count_bytes) = count(full, &keep, &never).unwrap();
        // Totals agree with a plain count over the original.
        assert_eq!((sub.files, sub.size), (count_files, count_bytes));
        // Every file in the subset matches, and maps back to the same file in the original.
        let mut got = Vec::new();
        files(&sub, &mut Vec::new(), &mut got);
        assert_eq!(got.len() as u64, count_files);
        for (p, name, size) in &got {
            assert!(f.matches(name));
            let orig = full.at(&back.to_full(p)).unwrap();
            assert_eq!((&*orig.name, orig.size), (name.as_str(), *size));
            assert_eq!(back.to_view(&back.to_full(p)), (p.clone(), true));
        }
        // Folder totals add up and children stay sorted largest-first, so the treemap,
        // sunburst and directory tree all see the same numbers.
        fn sums(n: &Node) {
            if n.is_dir {
                assert_eq!(n.size, n.children.iter().map(|c| c.size).sum::<u64>());
                assert_eq!(n.files, n.children.iter().map(|c| c.files).sum::<u64>());
                assert!(n.children.windows(2).all(|w| w[0].size >= w[1].size));
                assert!(n.children.iter().all(|c| !c.is_dir || c.files > 0), "no empty folders");
                n.children.iter().for_each(sums);
            }
        }
        sums(&sub);
    }

    #[test]
    fn keeps_matches_and_their_folders() {
        let t = sample();
        let f = Filter::compile("*.exr, !preview*", Options::default()).unwrap();
        check_consistent(&t, &f);
        let never = AtomicBool::new(false);
        let (sub, _) = subset(&t, &|n: &Node| f.matches(&n.name), &never).unwrap();
        assert_eq!(sub.files, 10);
        assert_eq!(sub.children.len(), 1, "only 'shots' survives");
        assert_eq!(&*sub.children[0].name, "shots");
        assert_eq!(sub.children[0].children.len(), 1, "only sh010 survives");
        for p in ["*", "comp.####.*", "comp.[1003-1005].exr", "*.png", "!*.exr"] {
            check_consistent(&t, &Filter::compile(p, Options::default()).unwrap());
        }
    }

    #[test]
    fn empty_result_keeps_the_root() {
        let t = sample();
        let never = AtomicBool::new(false);
        let (sub, back) = subset(&t, &|_: &Node| false, &never).unwrap();
        assert_eq!((sub.files, sub.size, sub.children.len()), (0, 0, 0));
        assert!(sub.is_dir);
        assert_eq!(&*sub.name, "root");
        assert_eq!(back.to_view(&[0, 0]), (vec![], false));
        assert_eq!(count(&t, &|_: &Node| false, &never), Some((0, 0)));
    }

    #[test]
    fn paths_translate_both_ways() {
        let t = sample();
        let f = Filter::compile("*.png, *.txt, *.iso", Options::default()).unwrap();
        let never = AtomicBool::new(false);
        let (sub, back) = subset(&t, &|n: &Node| f.matches(&n.name), &never).unwrap();
        // big.iso is the largest match in both trees.
        assert_eq!(&*sub.children[0].name, "big.iso");
        let full_iso = t.children.iter().position(|c| &*c.name == "big.iso").unwrap() as u32;
        assert_eq!(back.to_full(&[0]), vec![full_iso]);
        // A folder with some matches maps to its counterpart; a missing child maps to its
        // deepest surviving ancestor.
        let shots = t.children.iter().position(|c| &*c.name == "shots").unwrap() as u32;
        let (p, exact) = back.to_view(&[shots]);
        assert!(exact);
        assert_eq!(&*sub.at(&p).unwrap().name, "shots");
        let sh010 = t.children[shots as usize].children.iter().position(|c| &*c.name == "sh010").unwrap() as u32;
        assert_eq!(back.to_view(&[shots, sh010, 0]), (p.clone(), false));
    }

    #[test]
    fn switching_views_keeps_place() {
        // Highlight (whole tree) -> matches only (*.exr) -> another filter (*.png) -> Highlight:
        // a selected frame stays selected while it's shown, and falls back to its folder after.
        let t = sample();
        let never = AtomicBool::new(false);
        let shots = t.children.iter().position(|c| &*c.name == "shots").unwrap();
        let sh010 = t.children[shots].children.iter().position(|c| &*c.name == "sh010").unwrap();
        let frame = t.children[shots].children[sh010].children.iter().position(|c| &*c.name == "comp.1005.exr").unwrap();
        let full = vec![shots as u32, sh010 as u32, frame as u32];
        let exr = Filter::compile("*.exr", Options::default()).unwrap();
        let (a, back_a) = subset(&t, &|n: &Node| exr.matches(&n.name), &never).unwrap();
        let (in_a, exact) = back_a.to_view(&full);
        assert!(exact);
        assert_eq!(&*a.at(&in_a).unwrap().name, "comp.1005.exr");
        let png = Filter::compile("*.png", Options::default()).unwrap();
        let (b, back_b) = subset(&t, &|n: &Node| png.matches(&n.name), &never).unwrap();
        let (in_b, exact) = back_b.to_view(&back_a.to_full(&in_a));
        assert!(!exact);
        assert_eq!(&*b.at(&in_b).unwrap().name, "shots");
        assert_eq!(back_a.to_full(&in_a), full, "back to the whole tree");
    }

    #[test]
    fn hits_point_at_hidden_matches() {
        let t = sample();
        let never = AtomicBool::new(false);
        for p in ["*.exr", "*.txt", "!*.exr", "*.png, *.iso", "*nothing*"] {
            let f = Filter::compile(p, Options::default()).unwrap();
            let keep = |n: &Node| f.matches(&n.name);
            let h = hits(&t, &keep, &never).unwrap();
            assert_eq!((h.files, h.bytes), count(&t, &keep, &never).unwrap(), "{p}");
            // For every folder: `at` agrees with a direct count, and `any_from` with a scan
            // of the children from each index on.
            fn check(n: &Node, path: &mut Vec<u32>, h: &Hits, keep: &(dyn Fn(&Node) -> bool + Sync)) {
                let never = AtomicBool::new(false);
                let direct = count(n, &|c: &Node| keep(c), &never).unwrap();
                match h.at(path) {
                    Some(x) => assert_eq!((x.files, x.bytes), direct),
                    None => assert_eq!(direct.0, 0),
                }
                for from in 0..=n.children.len() {
                    let any = n.children[from..].iter().any(|c| count(c, &|c: &Node| keep(c), &never).unwrap().0 > 0);
                    assert_eq!(h.at(path).is_some_and(|x| x.any_from(from)), any);
                }
                for (i, c) in n.children.iter().enumerate().filter(|(_, c)| c.is_dir) {
                    path.push(i as u32);
                    check(c, path, h, keep);
                    path.pop();
                }
            }
            check(&t, &mut Vec::new(), &h, &keep);
        }
        let f = Filter::compile("*.exr, *.png", Options::default()).unwrap();
        let h = hits(&t, &|n: &Node| f.matches(&n.name), &never).unwrap();
        assert_eq!(h.main_ext(), Some("png"), "comp.1001.png (5000 B) outweighs the exr frames");
    }

    #[test]
    fn cancel_stops_work() {
        let t = sample();
        let stop = AtomicBool::new(true);
        assert!(subset(&t, &|_: &Node| true, &stop).is_none());
        assert!(count(&t, &|_: &Node| true, &stop).is_none());
        assert!(hits(&t, &|_: &Node| true, &stop).is_none());
    }

    #[test]
    fn large_tree() {
        // ~300k files in a deep synthetic tree: results agree with a plain count, quickly.
        let t = crate::treemap::synthetic(300_000, 7);
        let f = Filter::compile("*.r?, !*1*", Options::default()).unwrap();
        let start = std::time::Instant::now();
        check_consistent(&t, &f);
        let f = Filter::compile("*", Options::default()).unwrap();
        check_consistent(&t, &f);
        assert!(start.elapsed().as_secs() < 30);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::pattern::{Filter, Options};
    use std::time::Instant;

    /// Timings at the scale of a big drive. `cargo test --release -- --ignored --nocapture bench`
    #[test]
    #[ignore]
    fn three_and_a_half_million_files() {
        let t = crate::treemap::synthetic(3_500_000, 3);
        let never = AtomicBool::new(false);
        for p in ["*.png", "*.png, *.jpg, !*1*", "f{frame:1000-2000}.*", "*", "*no-such-thing*"] {
            let f = Filter::compile(p, Options::default()).unwrap();
            let keep = |n: &Node| f.matches(&n.name);
            let s = Instant::now();
            let (files, _) = count(&t, &keep, &never).unwrap();
            let c = s.elapsed();
            let s = Instant::now();
            assert_eq!(hits(&t, &keep, &never).unwrap().files, files);
            println!("  hits {:.1?}", s.elapsed());
            let s = Instant::now();
            let (sub, back) = subset(&t, &keep, &never).unwrap();
            let b = s.elapsed();
            assert_eq!(sub.files, files);
            let s = Instant::now();
            drop((sub, back));
            println!("{p:24} {files:>9} files  count {c:>9.1?}  subset {b:>9.1?}  free {:>8.1?}", s.elapsed());
        }
        let s = Instant::now();
        let seqs = crate::sequences::detect(&t, 3, &never).unwrap();
        println!("sequence detection: {} sequences in {:.1?}", seqs.len(), s.elapsed());
    }
}
