//! An archive's contents as a tree for the preview: each folder with the total size of what's in
//! it, biggest first, and how well each file was compressed where the format says.

use crate::fsys::archive::Index;
use std::collections::HashMap;
use std::path::Path;

pub struct Node {
    pub name: String,
    pub is_dir: bool,
    /// Unpacked size, folders counting everything in them.
    pub size: u64,
    /// Compressed size, where every file under it has one.
    pub packed: Option<u64>,
    /// Files under it (1 for a file).
    pub files: u64,
    pub children: Vec<usize>,
    pub parent: usize,
    pub depth: u16,
}

pub struct Tree {
    /// Node 0 is the archive itself.
    pub nodes: Vec<Node>,
    pub facts: Vec<(&'static str, String)>,
}

impl Tree {
    pub fn build(ix: &Index, archive: &Path, archive_size: u64) -> Tree {
        let mut nodes = vec![Node { name: String::new(), is_dir: true, size: 0, packed: Some(0), files: 0, children: Vec::new(), parent: 0, depth: 0 }];
        let mut by_path: HashMap<&str, usize> = HashMap::new();
        // Items come sorted by path, so a folder always comes before what's in it.
        for it in &ix.items {
            let (parent, name) = match it.path.rsplit_once('/') {
                Some((p, n)) => (by_path.get(p).copied().unwrap_or(0), n),
                None => (0, it.path.as_str()),
            };
            let i = nodes.len();
            let depth = nodes[parent].depth + 1;
            nodes.push(Node {
                name: name.to_string(),
                is_dir: it.is_dir,
                size: if it.is_dir { 0 } else { it.size },
                packed: if it.is_dir { Some(0) } else { it.packed },
                files: u64::from(!it.is_dir),
                children: Vec::new(),
                parent,
                depth,
            });
            nodes[parent].children.push(i);
            by_path.insert(&it.path, i);
        }
        // Totals, deepest first (children always come after their parents).
        for i in (1..nodes.len()).rev() {
            let (size, packed, files) = (nodes[i].size, nodes[i].packed, nodes[i].files);
            let p = nodes[i].parent;
            let n = &mut nodes[p];
            n.size += size;
            n.files += files;
            n.packed = n.packed.zip(packed).map(|(a, b)| a + b);
        }
        // Biggest first, folders and files mixed, the way the map sizes things.
        let sizes: Vec<(u64, String)> = nodes.iter().map(|n| (n.size, n.name.to_lowercase())).collect();
        for n in &mut nodes {
            n.children.sort_by(|&a, &b| sizes[b].0.cmp(&sizes[a].0).then_with(|| sizes[a].1.cmp(&sizes[b].1)));
        }
        let root = &nodes[0];
        let folders = ix.items.iter().filter(|i| i.is_dir).count();
        let mut facts = vec![
            ("Contents", format!("{} {}, {} {}", root.files, if root.files == 1 { "file" } else { "files" }, folders, if folders == 1 { "folder" } else { "folders" })),
            ("Unpacked", crate::scan::human(root.size)),
        ];
        if root.size > 0 && archive_size > 0 {
            facts.push(("Compression", ratio_words(archive_size, root.size)));
        }
        let fmt = archive.file_name().and_then(|n| crate::fsys::archive::format_of(&n.to_string_lossy()));
        if let Some(f) = fmt {
            use crate::fsys::archive::Format as F;
            let name = match f {
                F::Zip => "Zip",
                F::SevenZ => "7-Zip",
                F::Tar => "Tar (not compressed)",
                F::TarGz => "Tar, gzip compressed",
                F::TarXz => "Tar, xz compressed",
                F::TarBz2 => "Tar, bzip2 compressed",
            };
            facts.push(("Format", name.into()));
        }
        if let Some(big) = ix.items.iter().filter(|i| !i.is_dir).max_by_key(|i| i.size) {
            facts.push(("Largest", format!("{} ({})", big.path.rsplit('/').next().unwrap_or(&big.path), crate::scan::human(big.size))));
        }
        if ix.encrypted > 0 {
            facts.push(("Password", format!("{} {} need a password", ix.encrypted, if ix.encrypted == 1 { "file" } else { "files" })));
        }
        if ix.unsafe_names > 0 {
            facts.push(("Warning", format!("{} {} would land outside the folder it's extracted to; Filescape leaves them out", ix.unsafe_names, if ix.unsafe_names == 1 { "name" } else { "names" })));
        }
        if root.files > 0 && root.size > archive_size.saturating_mul(1000) && root.size > 1 << 30 {
            facts.push(("Warning", "Unpacks to over 1,000 times its size: check where it came from before extracting it".into()));
        }
        Tree { nodes, facts }
    }
}

/// "Saves 63%" for packed beside unpacked; "Stored" when nothing was saved.
pub fn ratio_words(packed: u64, size: u64) -> String {
    if size == 0 {
        return String::new();
    }
    let saved = 1.0 - packed as f64 / size as f64;
    if saved < 0.01 { "Stored, not compressed".into() } else { format!("Saves {:.0}%", (saved * 100.0).floor()) }
}

/// Just the percentage saved, for a column: "63%", or "–" when nothing was.
pub fn saved_short(packed: u64, size: u64) -> String {
    if size == 0 {
        return String::new();
    }
    let saved = 1.0 - packed as f64 / size as f64;
    if saved < 0.01 { "–".into() } else { format!("{:.0}%", (saved * 100.0).floor()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sizes_and_ratios() {
        let dir = std::env::temp_dir().join(format!("filescape-archtree-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            z.start_file("docs/a.txt", deflate).unwrap();
            z.write_all(&[b'a'; 10_000]).unwrap();
            z.start_file("docs/deep/b.bin", stored).unwrap();
            z.write_all(&[7; 3000]).unwrap();
            z.start_file("top.txt", stored).unwrap();
            z.write_all(b"hello").unwrap();
            z.start_file("../evil.txt", stored).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let ix = crate::fsys::archive::index(&p).unwrap();
        let t = Tree::build(&ix, &p, std::fs::metadata(&p).unwrap().len());
        let root = &t.nodes[0];
        assert_eq!((root.size, root.files), (13_005, 3));
        let docs = &t.nodes[root.children[0]];
        assert_eq!((docs.name.as_str(), docs.size, docs.files), ("docs", 13_000, 2));
        let a = docs.children.iter().map(|&i| &t.nodes[i]).find(|n| n.name == "a.txt").unwrap();
        assert!(a.packed.unwrap() < 200);
        assert_eq!(saved_short(a.packed.unwrap(), a.size).ends_with('%'), true);
        assert_eq!(saved_short(3000, 3000), "–");
        assert!(t.facts.iter().any(|f| f.0 == "Warning" && f.1.starts_with("1 name")));
        assert!(t.facts.contains(&("Format", "Zip".into())));
        assert!(t.facts.contains(&("Largest", "a.txt (9.8 KB)".into())) || t.facts.iter().any(|f| f.0 == "Largest" && f.1.starts_with("a.txt")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
