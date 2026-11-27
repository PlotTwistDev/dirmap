//! Looking inside archives (zip, 7z, tar, tar.gz, tar.xz, tar.bz2) as if they were folders, and
//! taking things out of them.
//!
//! A path inside an archive is the archive's path followed by the path within it, for example
//! `D:\shots\plates.zip\day1\A001.exr`. Archives are only read, never changed. Their table of
//! contents is read once (on a listing thread) and kept for the few archives used most recently;
//! extracting checks every name, so nothing in an archive can be written outside the folder it's
//! extracted to.

use super::listing::Entry;
use crate::transfer::Ctl;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Zip,
    SevenZ,
    Tar,
    TarGz,
    TarXz,
    TarBz2,
}

/// The archive format of a file, from its name.
pub fn format_of(name: &str) -> Option<Format> {
    let n = name.to_ascii_lowercase();
    Some(if n.ends_with(".zip") {
        Format::Zip
    } else if n.ends_with(".7z") {
        Format::SevenZ
    } else if n.ends_with(".tar") {
        Format::Tar
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        Format::TarGz
    } else if n.ends_with(".tar.xz") || n.ends_with(".txz") {
        Format::TarXz
    } else if n.ends_with(".tar.bz2") || n.ends_with(".tbz2") || n.ends_with(".tbz") {
        Format::TarBz2
    } else {
        return None;
    })
}

/// An archive's name without its archive extension ("plates.tar.gz" → "plates").
pub fn stem(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    for ext in [".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".txz", ".tbz2", ".tbz", ".tar", ".zip", ".7z"] {
        if n.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    name.to_string()
}

/// Whether a file of this name is an archive dirmap can open.
pub fn is_archive(name: &str) -> bool {
    format_of(name).is_some()
}

/// For a path into an archive: the archive file and the path inside it ("" for its top level,
/// otherwise names joined with '/'). Decided by the names alone; a real folder named like an
/// archive is told apart when it's read.
pub fn split(path: &Path) -> Option<(PathBuf, String)> {
    // An archive on a server is a file like any other there.
    if super::remote::is_remote(path) {
        return None;
    }
    let mut prefix = PathBuf::new();
    let mut comps = path.components();
    while let Some(c) = comps.next() {
        prefix.push(c);
        if let Component::Normal(n) = c
            && is_archive(&n.to_string_lossy())
        {
            let rest: Vec<String> = comps.map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            return Some((prefix, rest.join("/")));
        }
    }
    None
}

/// Whether a path is something inside an archive (not the archive itself).
pub fn inside(path: &Path) -> bool {
    split(path).is_some_and(|(_, inner)| !inner.is_empty())
}

/// One item in an archive.
#[derive(Clone, Debug)]
pub struct Item {
    /// Its path inside the archive, names joined with '/'.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch, 0 when unknown.
    pub modified: i64,
    /// Its compressed size, where the format keeps one per file (zip).
    pub packed: Option<u64>,
}

/// An archive's table of contents, folders included (also ones only implied by paths).
pub struct Index {
    pub items: Vec<Item>,
    /// Names left out because they would land outside the folder it's extracted to.
    pub unsafe_names: usize,
    /// Files that need a password.
    pub encrypted: usize,
}

impl Index {
    fn get(&self, inner: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.path == inner)
    }
}

type Cache = Vec<(PathBuf, i64, u64, Arc<Index>)>;
static CACHE: Mutex<Cache> = Mutex::new(Vec::new());
const CACHE_MAX: usize = 6;

/// The table of contents of an archive, read once and kept while the file is unchanged.
pub fn index(archive: &Path) -> Result<Arc<Index>, String> {
    let md = fs::metadata(archive).map_err(|e| super::listing::describe(&e))?;
    let stamp = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    if let Ok(c) = CACHE.lock()
        && let Some(hit) = c.iter().find(|x| x.0 == archive && x.1 == stamp && x.2 == md.len())
    {
        return Ok(hit.3.clone());
    }
    let fmt = archive.file_name().and_then(|n| format_of(&n.to_string_lossy())).ok_or("Not an archive")?;
    let raw = read_items(archive, fmt).map_err(|e| format!("This archive couldn't be read: {e}"))?;
    let ix = Arc::new(complete(raw));
    if let Ok(mut c) = CACHE.lock() {
        c.retain(|x| x.0 != archive);
        c.insert(0, (archive.to_path_buf(), stamp, md.len(), ix.clone()));
        c.truncate(CACHE_MAX);
    }
    Ok(ix)
}

/// Clean a name from an archive: '/' separators, no leading or trailing slash, no "." parts.
/// None for names that would climb out ("..") or are empty.
fn clean(name: &str) -> Option<String> {
    let mut parts = Vec::new();
    let name = name.replace('\\', "/");
    for p in name.split('/') {
        match p {
            "" | "." => {}
            ".." => return None,
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Add the folders that are only implied by the paths of what's in them, and drop duplicates.
fn complete(raw: Raw) -> Index {
    let Raw { items: raw, unsafe_names, encrypted } = raw;
    let mut map: BTreeMap<String, Item> = BTreeMap::new();
    for it in raw {
        let mut p = it.path.as_str();
        while let Some((parent, _)) = p.rsplit_once('/') {
            map.entry(parent.to_string()).or_insert(Item { path: parent.to_string(), is_dir: true, size: 0, modified: 0, packed: None });
            p = parent;
        }
        map.insert(it.path.clone(), it);
    }
    Index { items: map.into_values().collect(), unsafe_names, encrypted }
}

fn zip_time(d: Option<zip::DateTime>) -> i64 {
    let Some(d) = d else { return 0 };
    chrono::NaiveDate::from_ymd_opt(d.year() as i32, d.month() as u32, d.day() as u32)
        .and_then(|day| day.and_hms_opt(d.hour() as u32, d.minute() as u32, d.second() as u32))
        .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
        .map_or(0, |t| t.timestamp())
}

/// A tar stream from a file, decompressed as its format needs.
fn tar_reader(archive: &Path, fmt: Format) -> io::Result<tar::Archive<Box<dyn Read>>> {
    let f = io::BufReader::new(fs::File::open(archive)?);
    let r: Box<dyn Read> = match fmt {
        Format::TarGz => Box::new(flate2::read::MultiGzDecoder::new(f)),
        Format::TarXz => Box::new(lzma_rust2::XzReader::new(f, true)),
        Format::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(f)),
        _ => Box::new(f),
    };
    Ok(tar::Archive::new(r))
}

#[derive(Default)]
struct Raw {
    items: Vec<Item>,
    unsafe_names: usize,
    encrypted: usize,
}

fn read_items(archive: &Path, fmt: Format) -> Result<Raw, String> {
    let mut raw = Raw::default();
    let out = &mut raw.items;
    match fmt {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(io::BufReader::new(fs::File::open(archive).map_err(|e| e.to_string())?)).map_err(|e| e.to_string())?;
            for i in 0..z.len() {
                let f = z.by_index_raw(i).map_err(|e| e.to_string())?;
                let Some(path) = f.name().ok().and_then(|n| clean(&n)) else {
                    raw.unsafe_names += 1;
                    continue;
                };
                raw.encrypted += usize::from(f.encrypted());
                out.push(Item { path, is_dir: f.is_dir(), size: f.size(), modified: zip_time(f.last_modified()), packed: Some(f.compressed_size()) });
            }
        }
        Format::SevenZ => {
            let r = sevenz_rust2::ArchiveReader::new(fs::File::open(archive).map_err(|e| e.to_string())?, sevenz_rust2::Password::empty()).map_err(|e| e.to_string())?;
            for e in &r.archive().files {
                let Some(path) = clean(e.name()) else {
                    raw.unsafe_names += 1;
                    continue;
                };
                let t: std::time::SystemTime = e.last_modified_date().into();
                let modified = t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                out.push(Item { path, is_dir: e.is_directory(), size: e.size(), modified, packed: None });
            }
        }
        _ => {
            let mut t = tar_reader(archive, fmt).map_err(|e| e.to_string())?;
            for e in t.entries().map_err(|e| e.to_string())? {
                let e = e.map_err(|e| e.to_string())?;
                let h = e.header();
                let kind = h.entry_type();
                if !(kind.is_dir() || kind.is_file()) {
                    continue;
                }
                let Some(path) = e.path().ok().and_then(|p| clean(&p.to_string_lossy())) else {
                    raw.unsafe_names += 1;
                    continue;
                };
                out.push(Item { path, is_dir: kind.is_dir(), size: h.size().unwrap_or(0), modified: h.mtime().unwrap_or(0) as i64, packed: None });
            }
        }
    }
    Ok(raw)
}

/// What's directly inside `inner` ("" for the top) of an archive, as listing entries.
pub fn list(archive: &Path, inner: &str) -> Result<Vec<Entry>, String> {
    let ix = index(archive)?;
    if !inner.is_empty() && !ix.get(inner).is_some_and(|i| i.is_dir) {
        return Err(format!("{inner} isn't a folder in this archive."));
    }
    let prefix = if inner.is_empty() { String::new() } else { format!("{inner}/") };
    Ok(ix
        .items
        .iter()
        .filter_map(|it| {
            let rest = it.path.strip_prefix(&prefix)?;
            (!rest.is_empty() && !rest.contains('/')).then(|| Entry {
                name: rest.into(),
                is_dir: it.is_dir,
                is_link: false,
                hidden: rest.starts_with('.'),
                size: it.size,
                alloc: it.size,
                flags: crate::scan::store::NO_ALLOC,
                modified: it.modified,
                bundle: 0,
            })
        })
        .collect())
}

/// An item inside an archive, by its full path: (is a folder, size of all it holds, modified).
pub fn item(path: &Path) -> Option<(bool, u64, i64)> {
    let (archive, inner) = split(path)?;
    if inner.is_empty() || !archive.is_file() {
        return None;
    }
    let ix = index(&archive).ok()?;
    let it = ix.get(&inner)?;
    let prefix = format!("{inner}/");
    let size = if it.is_dir { ix.items.iter().filter(|i| !i.is_dir && i.path.starts_with(&prefix)).map(|i| i.size).sum() } else { it.size };
    Some((it.is_dir, size, it.modified))
}

/// `path` + a cleaned relative name, refusing anything that isn't a plain name.
fn safe_join(dest: &Path, rel: &str) -> io::Result<PathBuf> {
    let mut out = dest.to_path_buf();
    for p in rel.split('/') {
        let c = Path::new(p);
        let ok = matches!(c.components().collect::<Vec<_>>().as_slice(), [Component::Normal(_)]);
        if !ok || p.contains(':') && cfg!(windows) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unsafe name in the archive: {rel}")));
        }
        out.push(p);
    }
    Ok(out)
}

/// Where an archive item goes when `inner` is extracted to `dest`, or None if it isn't part of it.
fn target(item: &str, inner: &str, dest: &Path) -> Option<io::Result<PathBuf>> {
    if inner.is_empty() {
        return Some(safe_join(dest, item));
    }
    if item == inner {
        return Some(Ok(dest.to_path_buf()));
    }
    let rest = item.strip_prefix(inner)?.strip_prefix('/')?;
    Some(safe_join(dest, rest))
}

/// Write one file from an archive, honouring pause and stop.
fn write_file(r: &mut dyn Read, to: &Path, modified: i64, ctl: &Ctl) -> io::Result<()> {
    if let Some(p) = to.parent() {
        fs::create_dir_all(p)?;
    }
    if let Ok(mut c) = ctl.current.lock() {
        *c = to.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    }
    let mut f = fs::File::create(to)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if !ctl.go_on() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
        ctl.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
    }
    if modified > 0 {
        let _ = f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified as u64));
    }
    Ok(())
}

/// Extract `inner` (a file or folder in the archive; "" for everything) so that it becomes
/// `dest`. `dest` must not exist yet.
pub fn extract(archive: &Path, inner: &str, dest: &Path, ctl: &Ctl) -> io::Result<()> {
    let fmt = archive.file_name().and_then(|n| format_of(&n.to_string_lossy())).ok_or_else(|| io::Error::other("not an archive"))?;
    let ix = index(archive).map_err(io::Error::other)?;
    let whole = inner.is_empty() || ix.get(inner).is_some_and(|i| i.is_dir);
    if whole {
        fs::create_dir(dest)?;
        // Folders first, so empty ones come out too.
        for it in ix.items.iter().filter(|i| i.is_dir) {
            if let Some(t) = target(&it.path, inner, dest) {
                fs::create_dir_all(t?)?;
            }
        }
    } else if ix.get(inner).is_none() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{inner} isn't in the archive")));
    }
    match fmt {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(io::BufReader::new(fs::File::open(archive)?)).map_err(io::Error::other)?;
            for i in 0..z.len() {
                let mut f = z.by_index(i).map_err(io::Error::other)?;
                if f.is_dir() {
                    continue;
                }
                let Some(name) = f.name().ok().and_then(|n| clean(&n)) else { continue };
                let Some(to) = target(&name, inner, dest) else { continue };
                let modified = zip_time(f.last_modified());
                write_file(&mut f, &to?, modified, ctl)?;
            }
        }
        Format::SevenZ => {
            let mut r = sevenz_rust2::ArchiveReader::new(fs::File::open(archive)?, sevenz_rust2::Password::empty()).map_err(io::Error::other)?;
            let mut err: Option<io::Error> = None;
            let res = r.for_each_entries(|e, rd| {
                if e.is_directory() {
                    return Ok(true);
                }
                let Some(name) = clean(e.name()) else { return Ok(true) };
                let Some(to) = target(&name, inner, dest) else {
                    // Read past it.
                    io::copy(rd, &mut io::sink())?;
                    return Ok(true);
                };
                let t: std::time::SystemTime = e.last_modified_date().into();
                let modified = t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                match to.and_then(|to| write_file(rd, &to, modified, ctl)) {
                    Ok(()) => Ok(true),
                    Err(e) => {
                        err = Some(e);
                        Ok(false)
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
            res.map_err(io::Error::other)?;
        }
        _ => {
            let mut t = tar_reader(archive, fmt)?;
            for e in t.entries()? {
                let mut e = e?;
                if !e.header().entry_type().is_file() {
                    continue;
                }
                let Some(name) = e.path().ok().and_then(|p| clean(&p.to_string_lossy())) else { continue };
                let Some(to) = target(&name, inner, dest) else { continue };
                let modified = e.header().mtime().unwrap_or(0) as i64;
                write_file(&mut e, &to?, modified, ctl)?;
            }
        }
    }
    Ok(())
}

/// Copy something out of an archive given its full path (see `split`).
pub fn extract_path(src: &Path, dest: &Path, ctl: &Ctl) -> io::Result<()> {
    let (archive, inner) = split(src).ok_or_else(|| io::Error::other("not in an archive"))?;
    extract(&archive, &inner, dest, ctl)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dirmap-archive-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(format_of("Plates.ZIP"), Some(Format::Zip));
        assert_eq!(format_of("src.tar.gz"), Some(Format::TarGz));
        assert_eq!(format_of("x.tgz"), Some(Format::TarGz));
        assert_eq!(format_of("notes.txt"), None);
        assert_eq!(stem("plates.Tar.GZ"), "plates");
        assert_eq!(stem("a.b.zip"), "a.b");
        let (a, inner) = split(Path::new("/d/shots.zip/day1/a.exr")).unwrap();
        assert_eq!(a, Path::new("/d/shots.zip"));
        assert_eq!(inner, "day1/a.exr");
        assert_eq!(split(Path::new("/d/shots.zip")).unwrap().1, "");
        assert!(split(Path::new("/d/shots")).is_none());
        assert!(inside(Path::new("/d/a.7z/x")) && !inside(Path::new("/d/a.7z")));
        assert_eq!(clean("./a//b/"), Some("a/b".into()));
        assert_eq!(clean("a/../../etc"), None);
        assert!(safe_join(Path::new("/x"), "a/b").is_ok());
        assert!(safe_join(Path::new("/x"), "/etc/passwd").is_err() || safe_join(Path::new("/x"), "/etc/passwd").unwrap().starts_with("/x"));
    }

    #[test]
    fn zip_and_tar_list_and_extract() {
        let d = tmp("rw");
        // A zip with a folder, a nested file and a name trying to climb out.
        let zp = d.join("shots.zip");
        {
            let mut w = zip::ZipWriter::new(fs::File::create(&zp).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("day1/a.txt", o).unwrap();
            w.write_all(b"alpha").unwrap();
            w.start_file("day1/deep/b.txt", o).unwrap();
            w.write_all(b"beta!").unwrap();
            w.start_file("top.txt", o).unwrap();
            w.write_all(b"top").unwrap();
            w.start_file("../evil.txt", o).unwrap();
            w.write_all(b"no").unwrap();
            w.finish().unwrap();
        }
        let top = list(&zp, "").unwrap();
        let mut names: Vec<&str> = top.iter().map(|e| &*e.name).collect();
        names.sort();
        assert_eq!(names, ["day1", "top.txt"]);
        let day = list(&zp, "day1").unwrap();
        assert_eq!(day.len(), 2);
        assert!(day.iter().any(|e| &*e.name == "deep" && e.is_dir));
        assert_eq!(item(&zp.join("day1")).unwrap(), (true, 10, item(&zp.join("day1")).unwrap().2));
        // Extract a folder, a single file, and everything.
        let ctl = Ctl::default();
        extract(&zp, "day1", &d.join("out1"), &ctl).unwrap();
        assert_eq!(fs::read(d.join("out1/deep/b.txt")).unwrap(), b"beta!");
        extract_path(&zp.join("top.txt"), &d.join("one.txt"), &ctl).unwrap();
        assert_eq!(fs::read(d.join("one.txt")).unwrap(), b"top");
        extract(&zp, "", &d.join("all"), &ctl).unwrap();
        assert!(d.join("all/day1/a.txt").is_file());
        assert!(!d.join("evil.txt").exists());

        // A tar.gz of a folder.
        let src = d.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/c.txt"), b"gamma").unwrap();
        let tp = d.join("src.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(fs::File::create(&tp).unwrap(), flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            b.append_dir_all("src", &src).unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        assert_eq!(list(&tp, "src/sub").unwrap()[0].name.as_ref(), "c.txt");
        extract(&tp, "src/sub/c.txt", &d.join("c.txt"), &ctl).unwrap();
        assert_eq!(fs::read(d.join("c.txt")).unwrap(), b"gamma");
        fs::remove_dir_all(&d).ok();
    }
}
