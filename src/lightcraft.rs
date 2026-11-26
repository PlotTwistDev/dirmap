//! LightCraft libraries: the folder LightCraft keeps its catalogue in (`catalog.snap` and
//! `catalog.log`), with its thumbnail cache (`thumbs/`) and the photos imported into it
//! (`Originals/`). Shows how many photos and albums it holds, what each part takes on disk, and
//! the photos it uses from elsewhere, marking any missing.
//!
//! Read-only. Only the catalogue's saved summary (`catalog.snap`) is read, for the few things
//! shown; changes LightCraft made since are in `catalog.log`, which isn't replayed. Photos are
//! looked up, never opened.

use crate::help::doc::{Block, Span};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The catalogue format version Filescape knows (LightCraft's `journal::VERSION`).
const NEWEST: u32 = 14;
/// A summary larger than this isn't read.
const SNAP_MAX: u64 = 1 << 30;
/// Files counted when adding up a folder.
const WALK_MAX: usize = 500_000;

pub const SNAPSHOT: &str = "catalog.snap";
pub const LOG: &str = "catalog.log";

/// A folder that holds a LightCraft catalogue. Only looks for the summary file, so it's cheap.
pub fn is_library(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir.join(SNAPSHOT)).is_ok_and(|m| m.is_file())
}

/// LightCraft's default name for a library folder.
pub fn is_library_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("LightCraft Library")
}

#[derive(Deserialize)]
struct Snap {
    format: String,
    version: u32,
    #[serde(default)]
    catalog: Catalog,
}

#[derive(Deserialize, Default)]
struct Catalog {
    #[serde(default)]
    photos: BTreeMap<String, Photo>,
    #[serde(default)]
    albums: BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct Photo {
    source: Source,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Source {
    File { path: String },
    #[serde(other)]
    Other,
}

/// Just the identification at the start of the summary, so a newer one isn't read whole.
#[derive(Deserialize)]
struct Head {
    format: String,
    version: u32,
}

/// The library's blocks and details for the preview.
pub fn read(dir: &Path) -> Result<(Vec<Block>, Vec<(&'static str, String)>), String> {
    let snap = dir.join(SNAPSHOT);
    let size = std::fs::metadata(&snap).map_err(|_| "Not a LightCraft library".to_string())?.len();
    let mut facts: Vec<(&'static str, String)> = vec![("App", "LightCraft".into())];
    let head = read_head(&snap)?;
    if head.format != "lightcraft-catalog" {
        return Err("Not a LightCraft library".into());
    }
    let newer = head.version > NEWEST;
    facts.push((
        "Catalogue",
        if newer { format!("version {}, from a newer LightCraft than Filescape reads (up to {NEWEST})", head.version) } else { format!("version {}", head.version) },
    ));
    let catalog = if newer || size > SNAP_MAX {
        None
    } else {
        let f = std::fs::File::open(&snap).map_err(|e| format!("Couldn't read it: {e}"))?;
        let s: Snap = serde_json::from_reader(std::io::BufReader::new(f).take(SNAP_MAX)).map_err(|_| "The catalogue couldn't be read (it may be damaged).".to_string())?;
        debug_assert!(s.format == head.format && s.version == head.version);
        Some(s.catalog)
    };
    let mut outside = Vec::new();
    let mut inside = 0usize;
    if let Some(c) = &catalog {
        let videos = c.photos.values().filter(|p| p.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("video"))).count();
        let mut s = group(c.photos.len());
        if videos > 0 {
            s = format!("{s}, including {}", count(videos, "video", "videos"));
        }
        facts.push(("Photos", s));
        facts.push(("Albums", group(c.albums.len())));
        let base = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        for p in c.photos.values() {
            if let Source::File { path } = &p.source {
                let full = crate::projrefs::resolve(path, Some(dir));
                if full.starts_with(dir) || full.starts_with(&base) {
                    inside += 1;
                } else {
                    outside.push(path.clone());
                }
            }
        }
    }
    // What each part takes.
    let log = std::fs::metadata(dir.join(LOG)).map(|m| m.len()).unwrap_or(0);
    let thumbs = folder_size(&dir.join("thumbs"));
    let originals = folder_size(&dir.join("Originals"));
    let refs = crate::projrefs::check("Photos elsewhere", outside, Some(dir));
    let mut rows = vec![
        ("Catalogue", size + log, "Photos, albums, ratings and edits".to_string()),
        ("Thumbnail cache", thumbs.0, "Pictures for quick display, redrawn by LightCraft if missing".to_string()),
        ("Imported photos", originals.0, format!("{} in Originals", count(originals.1, "file", "files"))),
    ];
    if catalog.is_some() {
        rows.push(("Photos elsewhere", refs.total, "Used from other folders (below)".to_string()));
    }
    for (k, v, _) in rows.iter().filter(|r| r.0 != "Photos elsewhere") {
        facts.push((k, crate::scan::human(*v)));
    }
    if catalog.is_some() && !refs.items.is_empty() {
        facts.push(("Photos elsewhere", refs.fact()));
    }
    let plain = |t: &str| vec![Span { text: t.to_string(), ..Default::default() }];
    let title = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "LightCraft library".into());
    let mut blocks = vec![Block::Heading { level: 1, spans: plain(&title), id: String::new() }];
    let mut line = "A LightCraft library.".to_string();
    if let Some(c) = &catalog {
        line.push_str(&format!(
            " {} in {}: {} imported into it and {} from other folders.",
            count(c.photos.len(), "photo", "photos"),
            count(c.albums.len(), "album", "albums"),
            group(inside),
            group(c.photos.len() - inside)
        ));
    } else if newer {
        line.push_str(" It was saved by a newer LightCraft than Filescape reads, so only its sizes are shown.");
    } else {
        line.push_str(" Its catalogue is too big to read here, so only its sizes are shown.");
    }
    if log > 0 {
        line.push_str(" Changes made since LightCraft last saved its summary aren't counted.");
    }
    blocks.push(Block::Para(plain(&line)));
    blocks.push(Block::Heading { level: 2, spans: plain("What it takes"), id: String::new() });
    let table = rows.iter().map(|(k, v, about)| vec![plain(k), plain(&crate::scan::human(*v)), plain(about)]).collect();
    blocks.push(Block::Table { head: vec![plain("Part"), plain("Size"), plain("What it is")], rows: table });
    blocks.extend(refs.blocks());
    Ok((blocks, facts))
}

fn read_head(snap: &Path) -> Result<Head, String> {
    // The identification comes first in the file (LightCraft writes it so); a few KB is plenty.
    let mut b = Vec::new();
    std::fs::File::open(snap).and_then(|f| f.take(4096).read_to_end(&mut b)).map_err(|e| format!("Couldn't read it: {e}"))?;
    let text = String::from_utf8_lossy(&b);
    let end = text.find("\"catalog\"").unwrap_or(text.len());
    let start = text[..end].trim_end().trim_end_matches(',');
    serde_json::from_str::<Head>(&format!("{start}}}")).or_else(|_| serde_json::from_str::<Head>(start)).map_err(|_| "Not a LightCraft library".to_string())
}

/// A folder's size and file count, without following links (capped).
fn folder_size(dir: &Path) -> (u64, usize) {
    let mut total = 0u64;
    let mut files = 0usize;
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                stack.push(e.path());
            } else if md.is_file() {
                total += md.len();
                files += 1;
                if files >= WALK_MAX {
                    return (total, files);
                }
            }
        }
    }
    (total, files)
}

fn group(n: usize) -> String {
    crate::inspect::group(n as u64)
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", group(n), if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(snap: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("filescape-lc-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))).join("LightCraft Library");
        std::fs::create_dir_all(d.join("thumbs/ab")).unwrap();
        std::fs::create_dir_all(d.join("Originals/2026")).unwrap();
        std::fs::write(d.join(SNAPSHOT), snap).unwrap();
        std::fs::write(d.join(LOG), "{\"seq\":8,\"crc\":1,\"op\":{}}\n").unwrap();
        std::fs::write(d.join("thumbs/ab/abcd.jpg"), [0u8; 300]).unwrap();
        std::fs::write(d.join("Originals/2026/a.jpg"), [0u8; 1000]).unwrap();
        d
    }

    #[test]
    fn photos_albums_and_where_they_are() {
        let outside = std::env::temp_dir().join(format!("filescape-lc-photo-{}.jpg", std::process::id()));
        std::fs::write(&outside, [0u8; 500]).unwrap();
        let snap = format!(
            r#"{{"format":"lightcraft-catalog","version":14,"seq":7,"catalog":{{"photos":{{
                "1":{{"id":1,"source":{{"type":"file","path":"Originals/2026/a.jpg"}},"file_name":"a.jpg","kind":"photo","develop":{{"exposure":0.5}}}},
                "2":{{"id":2,"source":{{"type":"file","path":"{}"}},"file_name":"b.jpg"}},
                "3":{{"id":3,"source":{{"type":"file","path":"/nowhere/c.mov"}},"kind":"video"}},
                "4":{{"id":4,"source":{{"type":"demo","scene":2}}}}}},
              "albums":{{"1":{{"name":"Trip"}}}},"next_photo":5}}}}"#,
            outside.display().to_string().replace('\\', "/")
        );
        let d = library(&snap);
        assert!(is_library(&d));
        let (blocks, facts) = read(&d).unwrap();
        let f = |k: &str| facts.iter().find(|x| x.0 == k).map(|x| x.1.clone());
        assert_eq!(f("Catalogue").as_deref(), Some("version 14"));
        assert_eq!(f("Photos").as_deref(), Some("4, including 1 video"));
        assert_eq!(f("Albums").as_deref(), Some("1"));
        assert_eq!(f("Imported photos"), Some(crate::scan::human(1000)));
        assert_eq!(f("Thumbnail cache"), Some(crate::scan::human(300)));
        assert_eq!(f("Photos elsewhere"), Some(format!("2 (1 not found), {}", crate::scan::human(500))));
        assert!(blocks.iter().any(|b| matches!(b, Block::Table { .. })));
        std::fs::remove_dir_all(d.parent().unwrap()).ok();
        std::fs::remove_file(&outside).ok();
    }

    #[test]
    fn a_newer_or_damaged_catalogue() {
        let d = library(r#"{"format":"lightcraft-catalog","version":99,"seq":1,"catalog":{"photos":{}}}"#);
        let (_, facts) = read(&d).unwrap();
        assert!(facts.iter().any(|(k, v)| *k == "Catalogue" && v.contains("newer LightCraft")));
        assert!(!facts.iter().any(|(k, _)| *k == "Photos"));
        std::fs::remove_dir_all(d.parent().unwrap()).ok();
        let d = library(r#"{"format":"lightcraft-catalog","version":14,"seq":1,"catalog":{"photos":{"1":"#);
        assert!(read(&d).is_err());
        std::fs::remove_dir_all(d.parent().unwrap()).ok();
        let d = library(r#"{"format":"something-else","version":1}"#);
        assert!(read(&d).is_err());
        std::fs::remove_dir_all(d.parent().unwrap()).ok();
    }
}
