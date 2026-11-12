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
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // Through Finder, so "Put Back" works.
        let p = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("tell application \"Finder\" to delete POSIX file \"{p}\"");
        let out = Command::new("osascript").arg("-e").arg(script).output().map_err(|e| format!("Couldn't run osascript: {e}"))?;
        if out.status.success() {
            Ok(())
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
                    return Ok(());
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

/// Move into `dest_dir`, keeping the name. Across drives this copies, then deletes the original.
/// Returns the new path.
pub fn move_into(src: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    let name = src.file_name().ok_or("Can't move a drive.")?;
    let dest = dest_dir.join(name);
    if dest.starts_with(src) {
        return Err("Can't move a folder into itself.".into());
    }
    if fs::symlink_metadata(&dest).is_ok() {
        return Err(format!("{} already exists.", dest.display()));
    }
    match fs::rename(src, &dest) {
        Ok(()) => Ok(dest),
        Err(e) if is_cross_device(&e) => {
            if let Err(e) = copy_all(src, &dest) {
                // Leave the original alone and tidy up the partial copy.
                let _ = delete(&dest);
                return Err(format!("Copying to {} failed: {e}", dest_dir.display()));
            }
            delete(src).map_err(|e| format!("Copied, but couldn't remove the original: {e}"))?;
            Ok(dest)
        }
        Err(e) => Err(format!("Couldn't move: {e}")),
    }
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows.
    e.kind() == io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(if cfg!(windows) { 17 } else { 18 })
}

fn copy_all(src: &Path, dest: &Path) -> io::Result<()> {
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
        assert!(move_into(&d.join("src"), &d.join("src/inner")).is_err());
        let to = move_into(&d.join("src"), &d.join("dest")).unwrap();
        assert_eq!(fs::read_to_string(to.join("inner/f")).unwrap(), "abc");
        copy_all(&d.join("dest"), &d.join("copy")).unwrap();
        assert_eq!(fs::read_to_string(d.join("copy/src/inner/f")).unwrap(), "abc");
        delete(&d).unwrap();
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
