//! Thumbnails kept on disk between runs, so a folder of photos, videos or models shows its
//! pictures at once the second time. Each is a small QOI picture named by a hash of the file's
//! path, its size and date (so a changed file gets a new one) and the picture size asked for.
//!
//! The folder is the system's cache folder (~/Library/Caches, %LOCALAPPDATA%, ~/.cache), which
//! backup tools and cleaners know to skip. It's kept under a size limit, the least recently
//! used going first, and can be turned off or emptied in Settings.

use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(true);
/// The most the folder may hold, in bytes.
static LIMIT: AtomicU64 = AtomicU64::new(512 << 20);
/// Written since the folder was last tidied.
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// Thumbnails of 3D models and MIDI files (drawn here, so slower than most) are made.
static RICH: AtomicBool = AtomicBool::new(true);

pub fn set_rich(on: bool) {
    RICH.store(on, Ordering::Relaxed);
}

pub fn rich() -> bool {
    RICH.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn set_limit(bytes: u64) {
    LIMIT.store(bytes.max(16 << 20), Ordering::Relaxed);
}

/// The cache folder.
pub fn folder() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join(crate::APP_ID).join("Cache"))
    } else if cfg!(target_os = "macos") {
        crate::fsys::places::home().map(|h| h.join("Library/Caches").join(crate::APP_ID))
    } else {
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| crate::fsys::places::home().map(|h| h.join(".cache"))).map(|p| p.join(crate::APP_ID.to_lowercase()))
    };
    base.map(|b| b.join("thumbnails"))
}

fn key(path: &Path, stamp: u64, max: u32) -> String {
    // FNV-1a over the path's bytes, then the rest mixed in: stable between runs (unlike the
    // standard hasher's random keys).
    struct Fnv(u64);
    impl Hasher for Fnv {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            for b in bytes {
                self.0 = (self.0 ^ *b as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    let mut a = Fnv(0xcbf2_9ce4_8422_2325);
    path.as_os_str().hash(&mut a);
    let mut b = Fnv(0x8422_2325_cbf2_9ce4);
    path.as_os_str().hash(&mut b);
    stamp.hash(&mut b);
    max.hash(&mut b);
    format!("{:016x}{:016x}", a.finish(), b.finish())
}

fn file_for(dir: &Path, k: &str) -> PathBuf {
    dir.join(&k[..2]).join(format!("{k}.qoi"))
}

/// A kept thumbnail: RGBA pixels and their size.
pub fn get(path: &Path, stamp: u64, max: u32) -> Option<(usize, usize, Vec<u8>)> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    let f = file_for(&folder()?, &key(path, stamp, max));
    let mut data = Vec::new();
    std::fs::File::open(&f).ok()?.take(64 << 20).read_to_end(&mut data).ok()?;
    let img = image::load_from_memory_with_format(&data, image::ImageFormat::Qoi).ok()?.to_rgba8();
    // Used: newest first when the folder is tidied.
    if let Ok(file) = std::fs::File::options().append(true).open(&f) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
    Some((img.width() as usize, img.height() as usize, img.into_raw()))
}

/// Keep a thumbnail. Written beside its place and then moved there, so a reader never sees half.
pub fn put(path: &Path, stamp: u64, max: u32, w: usize, h: usize, rgba: &[u8]) {
    if !ENABLED.load(Ordering::Relaxed) || w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return;
    }
    let Some(dir) = folder() else { return };
    let f = file_for(&dir, &key(path, stamp, max));
    let Some(parent) = f.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let mut out = Vec::new();
    use image::ImageEncoder;
    if image::codecs::qoi::QoiEncoder::new(&mut out).write_image(rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8).is_err() {
        return;
    }
    let tmp = f.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, &out).is_ok() && std::fs::rename(&tmp, &f).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    // Tidy now and then, not on every write.
    if WRITTEN.fetch_add(out.len() as u64, Ordering::Relaxed) > LIMIT.load(Ordering::Relaxed) / 8 {
        WRITTEN.store(0, Ordering::Relaxed);
        std::thread::spawn(tidy);
    }
}

/// The folder's size in bytes, and how many thumbnails it holds.
pub fn usage() -> (u64, usize) {
    let Some(dir) = folder() else { return (0, 0) };
    let files = list(&dir);
    (files.iter().map(|f| f.1).sum(), files.len())
}

fn list(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    for sub in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        // Only our two-letter folders: nothing else is ever touched.
        let n = sub.file_name();
        let n = n.to_string_lossy();
        if n.len() != 2 || !n.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        for e in std::fs::read_dir(sub.path()).into_iter().flatten().flatten() {
            let name = e.file_name();
            if !name.to_string_lossy().ends_with(".qoi") {
                continue;
            }
            if let Ok(m) = e.metadata()
                && m.is_file()
            {
                out.push((e.path(), m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
            }
        }
    }
    out
}

/// Keep the folder under its limit: the least recently used go first, down to three quarters.
pub fn tidy() {
    let Some(dir) = folder() else { return };
    let mut files = list(&dir);
    let limit = LIMIT.load(Ordering::Relaxed);
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= limit {
        return;
    }
    files.sort_by_key(|f| f.2);
    for (p, len, _) in files {
        if total <= limit / 4 * 3 {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

/// Empty the folder (Settings' "Clear"). Only our thumbnails are removed.
pub fn clear() -> u64 {
    let Some(dir) = folder() else { return 0 };
    let mut freed = 0;
    for (p, len, _) in list(&dir) {
        if std::fs::remove_file(&p).is_ok() {
            freed += len;
        }
    }
    for sub in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let _ = std::fs::remove_dir(sub.path()); // only if empty
    }
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stable_and_distinct() {
        let p = Path::new("/photos/a.jpg");
        assert_eq!(key(p, 1, 256), key(p, 1, 256));
        assert_ne!(key(p, 1, 256), key(p, 2, 256));
        assert_ne!(key(p, 1, 256), key(p, 1, 512));
        assert_ne!(key(p, 1, 256), key(Path::new("/photos/b.jpg"), 1, 256));
        assert_eq!(key(p, 1, 256).len(), 32);
    }

    #[test]
    #[ignore] // uses the real cache folder: run by hand
    fn round_trip() {
        let p = Path::new("/no/such/test-picture.png");
        let px: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        put(p, 7, 99, 4, 3, &px);
        assert_eq!(get(p, 7, 99), Some((4, 3, px)));
        assert_eq!(get(p, 8, 99), None);
    }
}
