//! Freeing up the space cloud files take on this computer, and keeping them here, using only the
//! methods each system documents for apps that aren't the cloud provider. Filescape asks the
//! system; the provider does the work, and refuses whatever isn't safe (files with changes not
//! yet uploaded, files open in an app). Nothing is ever deleted.
//!
//! - **Windows**: the Pinned and Unpinned file attributes of Cloud Files placeholders. They are
//!   what Explorer's "Free up space" and "Always keep on this device" set (and `attrib -P +U` /
//!   `attrib +P -U` on the command line); OneDrive, iCloud, Dropbox, Box and other Cloud Files
//!   providers follow them.
//! - **macOS**: `NSFileManager`'s `evictUbiquitousItemAtURL:error:` (Finder's "Remove Download")
//!   and `startDownloadingUbiquitousItemAtURL:error:` (Finder's "Download Now"), for iCloud Drive
//!   and File Provider folders.
//! - **Linux** and mounted cloud drives: no documented way, so nothing is offered.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// What to ask the provider for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Remove the local copy; the file stays in the cloud and downloads again when opened.
    FreeUp,
    /// Keep the files on this computer: pin them on Windows (which downloads them and stops them
    /// being freed up), download them on macOS.
    Keep,
}

impl Action {
    /// The menu command, as each system names it.
    pub fn label(self) -> &'static str {
        match self {
            Action::FreeUp if cfg!(target_os = "macos") => "Remove Download",
            Action::FreeUp => "Free Up Space",
            Action::Keep if cfg!(target_os = "macos") => "Download Now",
            Action::Keep => "Always Keep on This Device",
        }
    }
}

/// Whether this system has a documented way to free up and keep cloud files.
pub fn supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

/// What happened.
#[derive(Default, Debug)]
pub struct Outcome {
    /// Files the provider was asked about and accepted.
    pub done: u64,
    /// Files that were already as asked (already online only, already downloaded).
    pub already: u64,
    /// Files the provider refused, with its reason (the first few).
    pub failed: u64,
    pub reasons: Vec<(PathBuf, String)>,
    /// Freed-up files still on this computer when Filescape stopped waiting: the provider is
    /// still working on them.
    pub pending: u64,
    pub cancelled: bool,
}

impl Outcome {
    /// One line for the status bar.
    pub fn summary(&self, a: Action) -> String {
        let n = |k: u64| if k == 1 { "1 file".to_string() } else { format!("{k} files") };
        let mut s = match a {
            Action::FreeUp => format!("Asked your cloud app to free up {}", n(self.done)),
            Action::Keep if cfg!(target_os = "macos") => format!("Asked your cloud app to download {}", n(self.done)),
            Action::Keep => format!("Set {} to always stay on this device", n(self.done)),
        };
        if self.already > 0 {
            s.push_str(&format!("; {} already {}", n(self.already), if a == Action::FreeUp { "only in the cloud" } else { "here" }));
        }
        if self.pending > 0 {
            s.push_str(&format!("; {} still being freed up (Refresh later to see it)", n(self.pending)));
        }
        if a == Action::Keep && self.done > 0 {
            s.push_str(". Downloads continue in the background; Refresh to see them");
        }
        if self.failed > 0 {
            s.push_str(&format!(". {} couldn't be changed", n(self.failed)));
            if let Some((p, why)) = self.reasons.first() {
                s.push_str(&format!(" ({}: {why})", p.file_name().map(|f| f.to_string_lossy()).unwrap_or_default()));
            }
        }
        s.push('.');
        s.replace("..", ".")
    }
}

/// Ask the provider to free up or keep every file in `paths` (folders: everything in them).
/// Folders are walked by their listing only. Freed-up files are watched for a few seconds so the
/// tree shows the result.
pub fn apply(paths: &[PathBuf], a: Action, cancel: &AtomicBool) -> Outcome {
    let mut out = Outcome::default();
    let mut watch = Vec::new();
    for p in paths {
        walk(p, a, cancel, &mut out, &mut watch);
    }
    out.cancelled = cancel.load(Ordering::Relaxed);
    if a == Action::FreeUp && !watch.is_empty() && !out.cancelled {
        // Providers free files up on their own schedule, usually within a second or two.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !watch.is_empty() && std::time::Instant::now() < until && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(400));
            watch.retain(|f| std::fs::symlink_metadata(f).is_ok_and(|md| !away(f, &md)));
        }
        out.pending = watch.len() as u64;
    }
    out
}

const WATCH_MAX: usize = 2000;

fn walk(p: &Path, a: Action, cancel: &AtomicBool, out: &mut Outcome, watch: &mut Vec<PathBuf>) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    // symlink_metadata: links are never followed, so nothing outside the selection changes.
    let Ok(md) = std::fs::symlink_metadata(p) else { return };
    if md.file_type().is_symlink() {
        return;
    }
    if md.is_dir() {
        // Windows keeps the folder's own state too, which new files in it inherit.
        #[cfg(windows)]
        let _ = sys::set(p, &md, a);
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                walk(&e.path(), a, cancel, out, watch);
            }
        }
        return;
    }
    if !md.is_file() {
        return;
    }
    let here = !away(p, &md);
    if (a == Action::FreeUp && !here && !pinned(&md)) || (a == Action::Keep && here && !needs_pin(&md)) {
        out.already += 1;
        return;
    }
    match sys::set(p, &md, a) {
        Ok(()) => {
            out.done += 1;
            if a == Action::FreeUp && here && watch.len() < WATCH_MAX {
                watch.push(p.to_path_buf());
            }
        }
        Err(e) => {
            out.failed += 1;
            if out.reasons.len() < 5 {
                out.reasons.push((p.to_path_buf(), e));
            }
        }
    }
}

/// None of the file's data is on this computer.
fn away(p: &Path, md: &std::fs::Metadata) -> bool {
    let (alloc, flags) = crate::scan::on_disk(md, || p.to_path_buf(), 4096);
    flags & crate::scan::store::ONLINE_ONLY != 0 || (alloc == 0 && md.len() > 0 && flags & crate::scan::store::CLOUD != 0)
}

#[cfg(windows)]
fn pinned(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & sys::PINNED != 0
}

#[cfg(not(windows))]
fn pinned(_md: &std::fs::Metadata) -> bool {
    false
}

/// On Windows, "keep" also means pinned, so a downloaded but unpinned file still needs it.
fn needs_pin(md: &std::fs::Metadata) -> bool {
    cfg!(windows) && !pinned(md)
}

#[cfg(windows)]
mod sys {
    use super::Action;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;
    use windows_sys::Win32::Storage::FileSystem::SetFileAttributesW;

    pub const PINNED: u32 = 0x8_0000;
    pub const UNPINNED: u32 = 0x10_0000;
    /// The attributes SetFileAttributesW takes, besides the two above.
    const SETTABLE: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x100 | 0x1000 | 0x2000;
    const NORMAL: u32 = 0x80;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    const OFFLINE: u32 = 0x1000;
    const REPARSE_POINT: u32 = 0x400;

    pub fn set(p: &Path, md: &std::fs::Metadata, a: Action) -> Result<(), String> {
        let attrs = md.file_attributes();
        // Only Cloud Files placeholders (reparse points, or carrying a cloud state) are changed.
        if attrs & (REPARSE_POINT | PINNED | UNPINNED | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS | OFFLINE) == 0 {
            return Err("not a cloud file".into());
        }
        let keep = attrs & SETTABLE;
        let new = match a {
            Action::FreeUp => keep | UNPINNED,
            Action::Keep => keep | PINNED,
        };
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `w` is a NUL-terminated path that outlives the call.
        if unsafe { SetFileAttributesW(w.as_ptr(), if new == 0 { NORMAL } else { new }) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use super::Action;
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    type Id = *mut c_void;
    type Sel = *mut c_void;

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    fn sel(s: &str) -> Sel {
        let c = CString::new(s).unwrap();
        // SAFETY: a valid NUL-terminated selector name.
        unsafe { sel_registerName(c.as_ptr()) }
    }

    fn class(s: &str) -> Id {
        let c = CString::new(s).unwrap();
        // SAFETY: a valid NUL-terminated class name.
        unsafe { objc_getClass(c.as_ptr()) }
    }

    /// `[NSFileManager.defaultManager evictUbiquitousItemAtURL:… error:…]` or
    /// `startDownloadingUbiquitousItemAtURL:error:`. Both return at once; the provider works in
    /// the background, and refuses to evict anything not yet uploaded.
    pub fn set(p: &Path, _md: &std::fs::Metadata, a: Action) -> Result<(), String> {
        let path = CString::new(p.as_os_str().as_bytes()).map_err(|_| "bad path".to_string())?;
        // SAFETY: objc_msgSend is called through function pointer types matching each method's
        // signature; every object is checked for nil before use, and autoreleased objects live
        // until the pool is popped after the last use.
        unsafe {
            let msg = objc_msgSend as *const ();
            let send0: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(msg);
            let send_str: extern "C" fn(Id, Sel, *const c_char) -> Id = std::mem::transmute(msg);
            let send1: extern "C" fn(Id, Sel, Id) -> Id = std::mem::transmute(msg);
            let send_ok: extern "C" fn(Id, Sel, Id, *mut Id) -> u8 = std::mem::transmute(msg);
            let utf8: extern "C" fn(Id, Sel) -> *const c_char = std::mem::transmute(msg);
            let pool = objc_autoreleasePoolPush();
            let result = (|| {
                let s = send_str(class("NSString"), sel("stringWithUTF8String:"), path.as_ptr());
                if s.is_null() {
                    return Err("bad path".to_string());
                }
                let url = send1(class("NSURL"), sel("fileURLWithPath:"), s);
                let fm = send0(class("NSFileManager"), sel("defaultManager"));
                if url.is_null() || fm.is_null() {
                    return Err("the system didn't answer".to_string());
                }
                let mut err: Id = std::ptr::null_mut();
                let method = match a {
                    Action::FreeUp => "evictUbiquitousItemAtURL:error:",
                    Action::Keep => "startDownloadingUbiquitousItemAtURL:error:",
                };
                if send_ok(fm, sel(method), url, &mut err) & 1 != 0 {
                    return Ok(());
                }
                if err.is_null() {
                    return Err("refused".to_string());
                }
                let desc = send0(err, sel("localizedDescription"));
                let c = if desc.is_null() { std::ptr::null() } else { utf8(desc, sel("UTF8String")) };
                Err(if c.is_null() { "refused".to_string() } else { CStr::from_ptr(c).to_string_lossy().into_owned() })
            })();
            objc_autoreleasePoolPop(pool);
            result
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod sys {
    use super::Action;
    use std::path::Path;

    pub fn set(_p: &Path, _md: &std::fs::Metadata, _a: Action) -> Result<(), String> {
        Err("not supported on this system".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_what_happened() {
        let o = Outcome { done: 3, already: 1, failed: 1, reasons: vec![(PathBuf::from("/x/a.txt"), "has changes not yet uploaded".into())], ..Default::default() };
        let s = o.summary(Action::FreeUp);
        assert!(s.starts_with("Asked your cloud app to free up 3 files; 1 file already only in the cloud"), "{s}");
        assert!(s.contains("1 file couldn't be changed (a.txt: has changes not yet uploaded)"), "{s}");
        assert!(!s.contains(".."));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn changes_nothing_where_unsupported() {
        let d = std::env::temp_dir().join(format!("dirmap-cloudops-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("f"), b"data").unwrap();
        let o = apply(&[d.clone()], Action::FreeUp, &AtomicBool::new(false));
        assert!(std::fs::read(d.join("f")).unwrap() == b"data", "never deleted");
        std::fs::remove_dir_all(&d).ok();
        assert_eq!((o.done, o.failed), (0, 1));
    }
}
