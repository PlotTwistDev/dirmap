//! File operations behind the right-click menus: open, show in the file manager, open a
//! terminal, Recycle Bin / Trash, permanent delete, move, hashing and Windows' Properties.
//! Everything that can take a while is meant to run off the UI thread.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What the file manager is called here, for menu labels.
pub const FILE_MANAGER: &str = if cfg!(windows) {
    "Explorer"
} else if cfg!(target_os = "macos") {
    "Finder"
} else {
    "File Manager"
};

/// What deleting to the bin is called here.
pub const BIN: &str = if cfg!(windows) { "Recycle Bin" } else { "Trash" };

fn spawn(cmd: &mut Command) -> Result<(), String> {
    cmd.spawn().map(drop).map_err(|e| format!("Couldn't start {}: {e}", cmd.get_program().to_string_lossy()))
}

#[cfg(windows)]
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Open with the default app (a folder opens in the file manager).
pub fn open(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let verb = wide("open".as_ref());
        let file = wide(path.as_os_str());
        let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        if r as usize > 32 { Ok(()) } else { Err(format!("Windows couldn't open {}.", path.display())) }
    }
    #[cfg(target_os = "macos")]
    {
        spawn(Command::new("open").arg(path))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        spawn(Command::new("xdg-open").arg(path))
    }
}

/// Show the item selected in its folder in the file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer parses its own command line: the path must be quoted as part of /select.
        spawn(Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path.display())))
    }
    #[cfg(target_os = "macos")]
    {
        spawn(Command::new("open").arg("-R").arg(path))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        // Most file managers (Nautilus, Dolphin, Nemo, Thunar…) can select a file over D-Bus;
        // otherwise just open the folder that holds it.
        let uri = file_uri(path);
        let ok = Command::new("dbus-send")
            .args(["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "--type=method_call"])
            .args(["/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems"])
            .arg(format!("array:string:{uri}"))
            .arg("string:")
            .output()
            .is_ok_and(|o| o.status.success());
        if ok {
            return Ok(());
        }
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        spawn(Command::new("xdg-open").arg(dir))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// The folder a terminal should start in: the item itself if it's a folder, else its parent.
fn folder_of(path: &Path) -> &Path {
    if path.is_dir() { path } else { path.parent().unwrap_or(path) }
}

/// Terminals offered in the menu: (label, which). Windows has two, like Explorer.
pub fn terminals() -> &'static [(&'static str, Terminal)] {
    if cfg!(windows) {
        &[("Open in Command Prompt…", Terminal::Cmd), ("Open in PowerShell…", Terminal::PowerShell)]
    } else {
        &[("Open in Terminal…", Terminal::Default)]
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Terminal {
    Default,
    Cmd,
    PowerShell,
}

pub fn terminal(path: &Path, which: Terminal) -> Result<(), String> {
    let dir = folder_of(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        let mut cmd = match which {
            Terminal::PowerShell => {
                let mut c = Command::new("powershell.exe");
                c.arg("-NoExit");
                c
            }
            _ => Command::new("cmd.exe"),
        };
        spawn(cmd.current_dir(dir).creation_flags(CREATE_NEW_CONSOLE))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = which;
        spawn(Command::new("open").args(["-a", "Terminal"]).arg(dir))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = which;
        let mut tried = Vec::new();
        if let Ok(t) = std::env::var("TERMINAL") {
            tried.push(t);
        }
        for t in ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "kitty", "alacritty", "wezterm", "foot", "xterm"] {
            tried.push(t.to_string());
        }
        for t in &tried {
            if Command::new(t).current_dir(dir).spawn().is_ok() {
                return Ok(());
            }
        }
        Err("Couldn't find a terminal to open (set $TERMINAL to choose one).".into())
    }
}

/// Windows' own Properties dialog. Returns false where there is none (the app shows its own).
pub fn shell_properties(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW, ShellExecuteExW};
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW;
        let verb = wide("properties".as_ref());
        let file = wide(path.as_os_str());
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_INVOKEIDLIST;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.nShow = SW_SHOW;
        unsafe { ShellExecuteExW(&mut info) != 0 }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Move to the Recycle Bin / Trash.
pub fn trash(path: &Path) -> Result<(), String> {
    trash_inner(path).map(drop)
}

/// Move to the Recycle Bin or Trash, then find where it went so Undo can put it back. `None`
/// when it's gone but can't be found there (a bin dirmap can't read, or none: on a network
/// share Windows deletes for good).
pub fn trash_keep(path: &Path) -> Result<Option<crate::transfer::Receipt>, String> {
    let original = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let since = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
    let told = trash_inner(path)?;
    if let Some(p) = told {
        return Ok(Some(crate::transfer::Receipt { in_bin: p, info: None }));
    }
    Ok(bin::find(&original, since))
}

/// Trash, returning where it went when the system says (Finder does).
fn trash_inner(path: &Path) -> Result<Option<PathBuf>, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW};
        // pFrom is a list of paths, each ending in a NUL, with one more NUL at the end.
        let mut from = wide(path.as_os_str());
        from.push(0);
        let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
        op.wFunc = FO_DELETE;
        op.pFrom = from.as_ptr();
        op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16;
        let r = unsafe { SHFileOperationW(&mut op) };
        if r != 0 {
            return Err(format!("Windows couldn't move it to the Recycle Bin (error {r:#x})."));
        }
        if op.fAnyOperationsAborted != 0 {
            return Err("Moving to the Recycle Bin was cancelled.".into());
        }
        if path.exists() {
            return Err("Some of it couldn't be moved to the Recycle Bin (in use, or no permission).".into());
        }
        Ok(None)
    }
    #[cfg(target_os = "macos")]
    {
        // Through Finder, so "Put Back" works.
        let p = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        // Finder answers with the item in the Trash, for Undo.
        let script = format!("tell application \"Finder\" to return POSIX path of ((delete POSIX file \"{p}\") as alias)");
        let out = Command::new("osascript").arg("-e").arg(script).output().map_err(|e| format!("Couldn't run osascript: {e}"))?;
        if out.status.success() {
            let at = String::from_utf8_lossy(&out.stdout).trim().trim_end_matches('/').to_string();
            Ok((!at.is_empty()).then(|| PathBuf::from(at)))
        } else {
            Err(format!("Finder couldn't move it to the Trash: {}", String::from_utf8_lossy(&out.stderr).trim()))
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let attempts: [&[&str]; 3] = [&["gio", "trash"], &["kioclient", "move"], &["trash-put"]];
        for a in attempts {
            let mut cmd = Command::new(a[0]);
            cmd.args(&a[1..]).arg(path);
            if a[0] == "kioclient" {
                cmd.arg("trash:/");
            }
            if let Ok(out) = cmd.output() {
                if out.status.success() && !path.exists() {
                    return Ok(None);
                }
                if a[0] == "gio" {
                    return Err(format!("Couldn't move it to the Trash: {}", String::from_utf8_lossy(&out.stderr).trim()));
                }
            }
        }
        Err("No Trash is available here (install gio or trash-cli), so nothing was deleted.".into())
    }
}

/// Delete for good, folder contents and all. Never follows links out of the folder.
pub fn delete(path: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(path)?;
    if md.is_dir() && !is_link(&md) {
        for e in fs::read_dir(path)? {
            delete(&e?.path())?;
        }
        remove(path, |p| fs::remove_dir(p))
    } else if md.is_dir() {
        // A directory symlink or junction: remove the link only.
        remove(path, |p| fs::remove_dir(p)).or_else(|_| remove(path, |p| fs::remove_file(p)))
    } else {
        remove(path, |p| fs::remove_file(p))
    }
}

fn is_link(md: &fs::Metadata) -> bool {
    if md.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        return md.file_attributes() & REPARSE_POINT != 0;
    }
    #[allow(unreachable_code)]
    false
}

/// Remove, clearing a read-only flag first if that's what's in the way (common on Windows).
fn remove(path: &Path, f: fn(&Path) -> io::Result<()>) -> io::Result<()> {
    match f(path) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            let mut perm = fs::symlink_metadata(path)?.permissions();
            if !perm.readonly() {
                return Err(e);
            }
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            fs::set_permissions(path, perm)?;
            f(path)
        }
        r => r,
    }
}

/// Delete everything inside a folder, keeping the folder.
pub fn empty_folder(dir: &Path) -> io::Result<()> {
    for e in fs::read_dir(dir)? {
        delete(&e?.path())?;
    }
    Ok(())
}

/// Remove every folder below `dir` that is empty on disk (including ones that become empty
/// when their empty subfolders go). `dir` itself stays. Returns how many were removed.
pub fn remove_empty_folders(dir: &Path) -> io::Result<u64> {
    fn walk(d: &Path, count: &mut u64) -> io::Result<bool> {
        let mut empty = true;
        for e in fs::read_dir(d)? {
            let e = e?;
            let md = fs::symlink_metadata(e.path())?;
            if md.is_dir() && !is_link(&md) {
                if walk(&e.path(), count)? && fs::remove_dir(e.path()).is_ok() {
                    *count += 1;
                } else {
                    empty = false;
                }
            } else {
                empty = false;
            }
        }
        Ok(empty)
    }
    let mut count = 0;
    walk(dir, &mut count)?;
    Ok(count)
}

/// A name in `dir` that isn't taken yet: `name`, else "stem copy.ext", "stem copy 2.ext", …
/// (`first` is the first suffix to try, e.g. "copy"; for new folders the plain name gets numbers).
pub fn free_name(dir: &Path, name: &str, first: &str) -> PathBuf {
    let taken = |p: &Path| fs::symlink_metadata(p).is_ok();
    let plain = dir.join(name);
    if !taken(&plain) {
        return plain;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let suffix = match (first.is_empty(), n) {
            (true, n) => format!(" {}", n + 1),
            (false, 1) => format!(" {first}"),
            (false, n) => format!(" {first} {n}"),
        };
        let p = dir.join(format!("{stem}{suffix}{ext}"));
        if !taken(&p) {
            return p;
        }
    }
    unreachable!()
}

/// Give a file or folder a new name in the same folder. Returns the new path.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name == "." || new_name == ".." {
        return Err("Type a name.".into());
    }
    if new_name.contains('/') || (cfg!(windows) && new_name.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])) {
        return Err(if cfg!(windows) { "A name can't contain any of \\ / : * ? \" < > |".into() } else { "A name can't contain /.".into() });
    }
    let parent = path.parent().ok_or("Can't rename a drive.")?;
    let dest = parent.join(new_name);
    // A change of case only is fine when the new name is the same file (case-insensitive disks).
    // On a case-sensitive disk `README` can be a different file from `readme`: never replace it.
    let case_only = dest.as_os_str().to_string_lossy().to_lowercase() == path.as_os_str().to_string_lossy().to_lowercase();
    let same_file = case_only && matches!((fs::canonicalize(path), fs::canonicalize(&dest)), (Ok(a), Ok(b)) if a == b);
    if !same_file && fs::symlink_metadata(&dest).is_ok() {
        return Err(format!("There's already something called {new_name} here."));
    }
    fs::rename(path, &dest).map_err(|e| format!("Couldn't rename: {e}"))?;
    Ok(dest)
}

/// Make a new, empty folder in `dir` with a free name. Returns its path.
pub fn new_folder(dir: &Path) -> Result<PathBuf, String> {
    let base = if cfg!(target_os = "macos") { "untitled folder" } else { "New folder" };
    let p = free_name(dir, base, "");
    fs::create_dir(&p).map_err(|e| format!("Couldn't make a folder: {e}"))?;
    Ok(p)
}

/// Make a new file in `dir` called `name` (or "name 2.ext"… if taken) holding `contents`.
/// `executable` sets the execute bits on Unix (scripts). Never replaces anything. Returns its path.
pub fn new_file(dir: &Path, name: &str, contents: &[u8], executable: bool) -> Result<PathBuf, String> {
    use std::io::Write;
    let p = free_name(dir, name, "");
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| format!("Couldn't make {name}: {e}"))?;
    f.write_all(contents).map_err(|e| format!("Couldn't write {name}: {e}"))?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        let mode = f.metadata().map(|m| m.permissions().mode()).unwrap_or(0o644);
        let _ = fs::set_permissions(&p, fs::Permissions::from_mode(mode | ((mode & 0o444) >> 2)));
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(p)
}

/// Copy a template file into `dir` under `name` (numbered if taken). Returns its path.
pub fn new_from_template(dir: &Path, template: &Path, name: &str) -> Result<PathBuf, String> {
    let p = free_name(dir, name, "");
    copy_all(template, &p).map_err(|e| {
        let _ = delete(&p);
        format!("Couldn't make {name}: {e}")
    })?;
    Ok(p)
}

pub(crate) fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows.
    e.kind() == io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(if cfg!(windows) { 17 } else { 18 })
}

pub(crate) fn copy_all(src: &Path, dest: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.is_dir() && !is_link(&md) {
        fs::create_dir(dest)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_all(&e.path(), &dest.join(e.file_name()))?;
        }
        Ok(())
    } else if md.is_file() {
        fs::copy(src, dest).map(drop)
    } else {
        Err(io::Error::other(format!("{} is a link or special file; not copied", src.display())))
    }
}

/// SHA-256 of a file as lowercase hex. `progress` gets the bytes read so far.
pub fn sha256(path: &Path, progress: &std::sync::atomic::AtomicU64, cancel: &std::sync::atomic::AtomicBool) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::sync::atomic::Ordering;
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        progress.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dirmap-actions-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rename_never_replaces_a_file_differing_only_in_case() {
        let d = tmp("case");
        fs::write(d.join("readme"), "mine").unwrap();
        let case_sensitive = fs::symlink_metadata(d.join("README")).is_err();
        if case_sensitive {
            fs::write(d.join("README"), "theirs").unwrap();
            assert!(rename(&d.join("readme"), "README").is_err());
            assert_eq!(fs::read_to_string(d.join("README")).unwrap(), "theirs");
            assert_eq!(fs::read_to_string(d.join("readme")).unwrap(), "mine");
        } else {
            // Case-insensitive disk: a case change of the same file is allowed.
            let p = rename(&d.join("readme"), "README").unwrap();
            assert_eq!(fs::read_to_string(p).unwrap(), "mine");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_and_empty() {
        let d = tmp("del");
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::write(d.join("a/b/f"), "x").unwrap();
        fs::write(d.join("a/g"), "y").unwrap();
        let mut p = fs::metadata(d.join("a/g")).unwrap().permissions();
        p.set_readonly(true);
        fs::set_permissions(d.join("a/g"), p).unwrap();
        empty_folder(&d.join("a")).unwrap();
        assert!(d.join("a").exists() && fs::read_dir(d.join("a")).unwrap().next().is_none());
        delete(&d).unwrap();
        assert!(!d.exists());
    }

    #[test]
    fn empty_folders() {
        let d = tmp("empty");
        fs::create_dir_all(d.join("x/y/z")).unwrap();
        fs::create_dir_all(d.join("k/l")).unwrap();
        fs::write(d.join("k/file"), "1").unwrap();
        assert_eq!(remove_empty_folders(&d).unwrap(), 4);
        assert!(!d.join("x").exists() && d.join("k").exists() && !d.join("k/l").exists());
        delete(&d).unwrap();
    }

    #[test]
    fn moving() {
        let d = tmp("mv");
        fs::create_dir_all(d.join("src/inner")).unwrap();
        fs::write(d.join("src/inner/f"), "abc").unwrap();
        fs::create_dir_all(d.join("dest")).unwrap();
        let into = |src: &Path, dir: &Path| crate::transfer::transfer(&[src.to_path_buf()], dir, false, &mut |_| panic!("no clash"));
        assert!(!into(&d.join("src"), &d.join("src/inner")).errors.is_empty());
        let to = into(&d.join("src"), &d.join("dest")).made[0].clone();
        assert_eq!(fs::read_to_string(to.join("inner/f")).unwrap(), "abc");
        copy_all(&d.join("dest"), &d.join("copy")).unwrap();
        assert_eq!(fs::read_to_string(d.join("copy/src/inner/f")).unwrap(), "abc");
        delete(&d).unwrap();
    }

    #[test]
    fn copy_rename_new_folder() {
        let d = tmp("ops");
        fs::write(d.join("a.txt"), b"hi").unwrap();
        let copy_into = |src: &Path, dir: &Path| {
            let o = crate::transfer::transfer(&[src.to_path_buf()], dir, true, &mut |_| panic!("no clash"));
            if o.errors.is_empty() { Ok(o.made[0].clone()) } else { Err(o.errors.join(" ")) }
        };
        let c = copy_into(&d.join("a.txt"), &d).unwrap();
        assert_eq!(c.file_name().unwrap(), "a copy.txt");
        assert_eq!(copy_into(&d.join("a.txt"), &d).unwrap().file_name().unwrap(), "a copy 2.txt");
        let r = rename(&c, "b.txt").unwrap();
        assert_eq!(fs::read(&r).unwrap(), b"hi");
        assert!(rename(&r, "a.txt").is_err(), "name taken");
        assert!(rename(&r, "x/y").is_err());
        let f1 = new_folder(&d).unwrap();
        let f2 = new_folder(&d).unwrap();
        assert!(f1.is_dir() && f2.is_dir() && f1 != f2);
        assert!(f2.file_name().unwrap().to_string_lossy().ends_with(" 2"));
        fs::write(f1.join("in.txt"), b"x").unwrap();
        let sub = copy_into(&f1, &f2).unwrap();
        assert!(sub.join("in.txt").is_file());
        assert!(copy_into(&f1, &f1).is_err(), "into itself");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn new_files_never_replace() {
        let d = tmp("newfile");
        let a = new_file(&d, "notes.md", b"# Notes\n", false).unwrap();
        let b = new_file(&d, "notes.md", b"x", false).unwrap();
        assert_eq!(a.file_name().unwrap(), "notes.md");
        assert_eq!(b.file_name().unwrap(), "notes 2.md");
        assert_eq!(fs::read(&a).unwrap(), b"# Notes\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let s = new_file(&d, "run.sh", b"#!/bin/sh\n", true).unwrap();
            assert_ne!(fs::metadata(&s).unwrap().permissions().mode() & 0o100, 0);
        }
        let t = new_from_template(&d, &a, "copy.md").unwrap();
        assert_eq!(fs::read(&t).unwrap(), b"# Notes\n");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hash() {
        let d = tmp("hash");
        fs::write(d.join("f"), "abc").unwrap();
        let h = sha256(&d.join("f"), &Default::default(), &Default::default()).unwrap();
        assert_eq!(h, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        delete(&d).unwrap();
    }
}

/// Finding an item just sent to the bin, to put it back.
mod bin {
    use crate::transfer::Receipt;
    use std::path::Path;
    use std::time::SystemTime;

    /// Windows: each drive's `$Recycle.Bin\<user SID>` holds `$R…` (the item) and `$I…` (where it
    /// came from). Only the user's own folder can be read, so trying them all finds it.
    #[cfg(windows)]
    pub fn find(original: &Path, since: SystemTime) -> Option<Receipt> {
        use std::path::Component;
        let Some(Component::Prefix(pre)) = original.components().next() else { return None };
        let root = std::path::PathBuf::from(format!("{}\\$Recycle.Bin", pre.as_os_str().to_string_lossy()));
        let want = original.to_string_lossy().to_lowercase();
        let mut best: Option<(u64, Receipt)> = None;
        for user in std::fs::read_dir(&root).ok()?.flatten() {
            let Ok(items) = std::fs::read_dir(user.path()) else { continue };
            for e in items.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.starts_with("$I") || e.metadata().and_then(|m| m.modified()).is_ok_and(|m| m < since) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(e.path()) else { continue };
                let Some((when, path)) = parse_i(&bytes) else { continue };
                if path.to_lowercase() == want && best.as_ref().is_none_or(|b| when > b.0) {
                    let r = Receipt { in_bin: user.path().join(format!("$R{}", &name[2..])), info: Some(e.path()) };
                    best = Some((when, r));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// A `$I` file: version, size, deletion time, then the original path (fixed 260 characters
    /// in version 1, length-prefixed in version 2). Returns (deletion time, path).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn parse_i(b: &[u8]) -> Option<(u64, String)> {
        let u64_at = |i: usize| b.get(i..i + 8).map(|x| u64::from_le_bytes(x.try_into().unwrap()));
        let version = u64_at(0)?;
        let when = u64_at(16)?;
        let units: Vec<u16> = match version {
            1 => b.get(24..24 + 520)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect(),
            2 => {
                let n = u32::from_le_bytes(b.get(24..28)?.try_into().ok()?) as usize;
                b.get(28..28 + n * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
            }
            _ => return None,
        };
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        Some((when, String::from_utf16_lossy(&units[..end])))
    }

    /// Finder told us where it went.
    #[cfg(target_os = "macos")]
    pub fn find(_original: &Path, _since: SystemTime) -> Option<Receipt> {
        None
    }

    /// Linux and other Unixes (the freedesktop.org Trash): the home Trash, or `.Trash-<uid>` at
    /// the top of the drive. `info/<name>.trashinfo` says where `files/<name>` came from.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn find(original: &Path, since: SystemTime) -> Option<Receipt> {
        use std::os::unix::fs::MetadataExt;
        let uid = unsafe { libc::getuid() };
        let mut bins: Vec<(std::path::PathBuf, Option<std::path::PathBuf>)> = Vec::new();
        let data = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).or_else(|| crate::fsys::places::home().map(|h| h.join(".local/share")));
        if let Some(d) = data {
            bins.push((d.join("Trash"), None));
        }
        // The top of the drive the item was on: the last folder up on the same device.
        if let Some(parent) = original.parent()
            && let Ok(dev) = std::fs::metadata(parent).map(|m| m.dev())
        {
            let mut top = parent.to_path_buf();
            while let Some(up) = top.parent() {
                if std::fs::metadata(up).map(|m| m.dev()).ok() != Some(dev) {
                    break;
                }
                top = up.to_path_buf();
            }
            bins.push((top.join(format!(".Trash-{uid}")), Some(top.clone())));
            bins.push((top.join(".Trash").join(uid.to_string()), Some(top)));
        }
        let mut best: Option<(SystemTime, Receipt)> = None;
        for (bin, top) in bins {
            let Ok(rd) = std::fs::read_dir(bin.join("info")) else { continue };
            for e in rd.flatten() {
                let Ok(when) = e.metadata().and_then(|m| m.modified()) else { continue };
                if when < since {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
                let Some(path) = text.lines().find_map(|l| l.strip_prefix("Path=")).map(unescape) else { continue };
                let full = match &top {
                    Some(t) if !path.starts_with('/') => t.join(&path),
                    _ => std::path::PathBuf::from(&path),
                };
                if full != original || best.as_ref().is_some_and(|b| b.0 >= when) {
                    continue;
                }
                let name = e.file_name().to_string_lossy().trim_end_matches(".trashinfo").to_string();
                best = Some((when, Receipt { in_bin: bin.join("files").join(name), info: Some(e.path()) }));
            }
        }
        best.map(|b| b.1)
    }

    /// %-escapes in a .trashinfo path.
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    pub fn unescape(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%'
                && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    #[cfg(test)]
    #[test]
    fn bin_notes() {
        assert_eq!(unescape("/home/r/My%20Shots/a%25b.exr"), "/home/r/My Shots/a%b.exr");
        // A version 2 $I file for C:\x.txt.
        let mut b = Vec::new();
        b.extend(2u64.to_le_bytes());
        b.extend(5u64.to_le_bytes());
        b.extend(99u64.to_le_bytes());
        let p: Vec<u16> = "C:\\x.txt\0".encode_utf16().collect();
        b.extend((p.len() as u32).to_le_bytes());
        for u in p {
            b.extend(u.to_le_bytes());
        }
        assert_eq!(parse_i(&b), Some((99, "C:\\x.txt".to_string())));
    }
}
