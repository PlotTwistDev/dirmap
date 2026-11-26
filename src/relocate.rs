//! Moving an app's whole model folder (Ollama's, LM Studio's, Hugging Face's) to another drive,
//! leaving a link where it was so the app keeps finding its models without being told.
//!
//! The order keeps the models safe at every step: copy everything to the new drive under a
//! temporary name, compare the copy with the original, name the copy, set the original aside
//! (a rename on its own drive), put the link in its place, check the link leads to the copy,
//! and only then delete the original. A failure or a stop at any step before the last puts
//! everything back as it was.

use crate::transfer::{self, Ctl};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

/// Room to leave on the new drive beyond the models themselves.
const SPARE: u64 = 512 << 20;

/// A move that has been checked and can start.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub app: &'static str,
    /// The folder the app uses now; a link once moved.
    pub from: PathBuf,
    /// The new folder, on the other drive.
    pub to: PathBuf,
    pub bytes: u64,
}

/// The new folder's name inside the chosen folder.
pub fn folder_name(app: &str) -> String {
    format!("{app} models")
}

/// Whether `from` is already a link to somewhere else (moved before, by us or by hand).
pub fn is_link(from: &Path) -> bool {
    fs::symlink_metadata(from).map(|m| m.file_type().is_symlink() || is_junction(&m)).unwrap_or(false)
}

#[cfg(windows)]
fn is_junction(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_REPARSE_POINT: a junction, which std doesn't call a symlink.
    m.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_junction(_: &fs::Metadata) -> bool {
    false
}

/// Whether Ollama's server is answering, so its files may be in use.
pub fn ollama_running() -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 11434));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(250)).is_ok()
}

/// Checks a move of `app`'s folder `from` into a new folder inside `dest`, and adds up its size.
/// The reason it can't happen, in a sentence, otherwise.
pub fn check(app: &'static str, from: &Path, dest: &Path) -> Result<Plan, String> {
    let md = fs::symlink_metadata(from).map_err(|e| format!("{} can't be read: {e}", from.display()))?;
    if md.file_type().is_symlink() || is_junction(&md) {
        return Err(format!("{app}'s folder is already a link to somewhere else."));
    }
    if !md.is_dir() {
        return Err(format!("{} isn't a folder.", from.display()));
    }
    if !dest.is_dir() {
        return Err(format!("{} isn't a folder.", dest.display()));
    }
    let to = dest.join(folder_name(app));
    if fs::symlink_metadata(&to).is_ok() {
        return Err(format!("{} already exists. Choose another folder, or rename that one.", to.display()));
    }
    let (cf, cd) = (fs::canonicalize(from).map_err(|e| e.to_string())?, fs::canonicalize(dest).map_err(|e| e.to_string())?);
    if cd.starts_with(&cf) {
        return Err("The new place is inside the folder being moved.".into());
    }
    if let (Some(a), Some(b)) = (crate::cleanup::file_id(from), crate::cleanup::file_id(dest))
        && a.0 == b.0
    {
        return Err("That's on the same drive, so moving there frees nothing. Choose a folder on another drive.".into());
    }
    if app == "Ollama" && ollama_running() {
        return Err("Ollama is running. Quit it first (from its menu-bar or tray icon, or stop the service), so no model is in use while it moves.".into());
    }
    let bytes = size(from);
    if let Some(free) = crate::drives::free_at(dest)
        && free < bytes + SPARE
    {
        return Err(format!("The new drive has {} free; the models need {}.", crate::scan::human(free), crate::scan::human(bytes + SPARE)));
    }
    Ok(Plan { app, from: from.to_path_buf(), to, bytes })
}

/// Bytes in the files under `p`, links not followed.
fn size(p: &Path) -> u64 {
    let Ok(md) = fs::symlink_metadata(p) else { return 0 };
    if md.is_dir() && !md.file_type().is_symlink() {
        fs::read_dir(p).map(|rd| rd.flatten().map(|e| size(&e.path())).sum()).unwrap_or(0)
    } else if md.is_file() {
        md.len()
    } else {
        0
    }
}

/// What happened, for the outcome line.
#[derive(Debug, PartialEq)]
pub struct Moved {
    /// The original couldn't be deleted at the end: it's still here (under another name) and can
    /// be removed by hand. The models are safely in the new place.
    pub left: Option<(PathBuf, String)>,
}

/// Does the move. On an error, nothing has changed (the error says so if putting things back
/// failed too).
pub fn run(plan: &Plan, ctl: &Ctl) -> Result<Moved, String> {
    let Plan { from, to, .. } = plan;
    let name = |p: &Path, tag: &str| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        p.with_file_name(format!(".{n}.{tag}"))
    };
    let partial = name(to, "filescape-partial");
    let old = name(from, "filescape-old");
    if fs::symlink_metadata(&old).is_ok() {
        return Err(format!("{} is in the way (left from an earlier move?). Remove it first.", old.display()));
    }
    ctl.bytes_total.store(plan.bytes, Ordering::Relaxed);
    ctl.bytes_done.store(0, Ordering::Relaxed);
    let undo_copy = |why: String| {
        let _ = fs::remove_dir_all(&partial);
        why
    };
    // 1. Copy, links made again as links.
    copy(from, &partial, from, to, ctl).map_err(|e| undo_copy(if e.kind() == io::ErrorKind::Interrupted { "Stopped. Nothing was moved.".into() } else { format!("Copying failed, so nothing was moved: {e}") }))?;
    // 2. Compare.
    same(from, &partial, from, to).map_err(|e| undo_copy(format!("The copy didn't match the original, so nothing was moved: {e}")))?;
    if !ctl.go_on() {
        return Err(undo_copy("Stopped. Nothing was moved.".into()));
    }
    // 3. Name the copy.
    fs::rename(&partial, to).map_err(|e| undo_copy(format!("The copy couldn't be named, so nothing was moved: {e}")))?;
    let undo_named = |why: String| {
        let _ = fs::remove_dir_all(to);
        why
    };
    // 4. Set the original aside.
    fs::rename(from, &old).map_err(|e| undo_named(format!("{} couldn't be set aside (is an app using it?), so nothing was moved: {e}", from.display())))?;
    let undo_aside = |why: String| {
        let _ = remove_link(from);
        match fs::rename(&old, from) {
            Ok(()) => undo_named(why),
            // Leave the copy too: the original is safe under its other name.
            Err(e) => format!("{why} Putting the original back failed too ({e}): it's at {}, and a full copy is at {}.", old.display(), to.display()),
        }
    };
    // 5. The link, and 6. that it leads to the copy.
    link(to, from).map_err(|e| undo_aside(format!("The link couldn't be made, so nothing was moved: {e}")))?;
    let leads = fs::canonicalize(from).ok().zip(fs::canonicalize(to).ok()).is_some_and(|(a, b)| a == b);
    if !leads {
        return Err(undo_aside("The link didn't lead to the new folder, so nothing was moved.".into()));
    }
    // 7. The original goes.
    let left = fs::remove_dir_all(&old).err().map(|e| (old.clone(), e.to_string()));
    Ok(Moved { left })
}

/// Copies the tree `src` to `dest`. Links are made again: one pointing inside the folder being
/// moved (`root`) by its full path is pointed at the same place in the new folder (`new_root`).
fn copy(src: &Path, dest: &Path, root: &Path, new_root: &Path, ctl: &Ctl) -> io::Result<()> {
    if !ctl.go_on() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
    }
    let md = fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        make_link(&retarget(&fs::read_link(src)?, root, new_root), dest, src)
    } else if md.is_dir() {
        fs::create_dir(dest)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy(&e.path(), &dest.join(e.file_name()), root, new_root, ctl)?;
        }
        Ok(())
    } else if md.is_file() {
        transfer::copy_tree(src, dest, ctl)
    } else {
        Err(io::Error::other(format!("{} isn't a file, folder or link", src.display())))
    }
}

fn retarget(target: &Path, root: &Path, new_root: &Path) -> PathBuf {
    match target.strip_prefix(root) {
        Ok(rest) if target.is_absolute() => new_root.join(rest),
        _ => target.to_path_buf(),
    }
}

#[cfg(unix)]
fn make_link(target: &Path, at: &Path, _like: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

#[cfg(windows)]
fn make_link(target: &Path, at: &Path, like: &Path) -> io::Result<()> {
    if fs::metadata(like).is_ok_and(|m| m.is_dir()) { std::os::windows::fs::symlink_dir(target, at) } else { std::os::windows::fs::symlink_file(target, at) }
}

#[cfg(not(any(unix, windows)))]
fn make_link(_: &Path, _: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::other("links aren't supported here"))
}

/// Every path in `a` is in `b` with the same kind, size and (for links) target.
fn same(a: &Path, b: &Path, root: &Path, new_root: &Path) -> Result<(), String> {
    let (ma, mb) = (fs::symlink_metadata(a).map_err(|e| e.to_string())?, fs::symlink_metadata(b).map_err(|e| format!("{} is missing: {e}", b.display()))?);
    let differs = || format!("{} differs from {}", b.display(), a.display());
    if ma.file_type().is_symlink() {
        let (ta, tb) = (fs::read_link(a).map_err(|e| e.to_string())?, fs::read_link(b).map_err(|e| e.to_string())?);
        return if mb.file_type().is_symlink() && retarget(&ta, root, new_root) == tb { Ok(()) } else { Err(differs()) };
    }
    if ma.is_dir() {
        if !mb.is_dir() {
            return Err(differs());
        }
        let names = |p: &Path| -> Result<Vec<_>, String> {
            let mut v: Vec<_> = fs::read_dir(p).map_err(|e| e.to_string())?.flatten().map(|e| e.file_name()).collect();
            v.sort();
            Ok(v)
        };
        let (na, nb) = (names(a)?, names(b)?);
        if na != nb {
            return Err(differs());
        }
        for n in na {
            same(&a.join(&n), &b.join(&n), root, new_root)?;
        }
        return Ok(());
    }
    if mb.is_file() && ma.len() == mb.len() { Ok(()) } else { Err(differs()) }
}

/// A link to the folder `target` at `at`: a symbolic link, or on Windows a junction (which needs
/// no special rights).
#[cfg(unix)]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

#[cfg(windows)]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(at).arg(target).creation_flags(0x0800_0000).output()?;
    if out.status.success() && fs::symlink_metadata(at).is_ok() {
        Ok(())
    } else {
        Err(io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

#[cfg(not(any(unix, windows)))]
fn link(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::other("links aren't supported here"))
}

/// Removes the link at `at` (never what it leads to).
fn remove_link(at: &Path) -> io::Result<()> {
    if !is_link(at) {
        return Ok(());
    }
    // A junction or a folder link is removed as a folder on Windows; a symlink as a file on Unix.
    fs::remove_file(at).or_else(|_| fs::remove_dir(at))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn tempdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("filescape-relocate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A Hugging Face–style store: a blob, a relative link to it, and an absolute one.
    fn store(root: &Path) {
        let repo = root.join("models--org--m");
        fs::create_dir_all(repo.join("blobs")).unwrap();
        fs::create_dir_all(repo.join("snapshots/abc")).unwrap();
        fs::write(repo.join("blobs/123"), vec![7u8; 5000]).unwrap();
        std::os::unix::fs::symlink("../../blobs/123", repo.join("snapshots/abc/model.gguf")).unwrap();
        std::os::unix::fs::symlink(repo.join("blobs/123"), repo.join("snapshots/abc/abs.gguf")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
    }

    #[test]
    fn moves_and_leaves_a_link() {
        let d = tempdir("moves");
        let from = d.join("hub");
        store(&from);
        fs::create_dir(d.join("other")).unwrap();
        let plan = Plan { app: "Hugging Face", from: from.clone(), to: d.join("other").join(folder_name("Hugging Face")), bytes: size(&from) };
        assert_eq!(plan.bytes, 5000);
        let r = run(&plan, &Ctl::default()).unwrap();
        assert_eq!(r.left, None);
        assert!(is_link(&from));
        // Both links still reach the model, through the old path and the new.
        for p in [&from, &plan.to] {
            assert_eq!(fs::read(p.join("models--org--m/snapshots/abc/model.gguf")).unwrap().len(), 5000);
            assert_eq!(fs::read(p.join("models--org--m/snapshots/abc/abs.gguf")).unwrap().len(), 5000);
        }
        assert_eq!(fs::read_link(plan.to.join("models--org--m/snapshots/abc/abs.gguf")).unwrap(), plan.to.join("models--org--m/blobs/123"));
        assert!(plan.to.join("empty").is_dir());
        assert!(!d.join(".hub.filescape-old").exists());
        // Moving again is refused: it's a link now.
        assert!(check("Hugging Face", &from, &d).unwrap_err().contains("already a link"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_failure_puts_everything_back() {
        let d = tempdir("fails");
        let from = d.join("models");
        store(&from);
        // The copy can't be named: something is already where it would go.
        let to = d.join("new");
        fs::write(&to, b"in the way").unwrap();
        let plan = Plan { app: "Ollama", from: from.clone(), to: to.clone(), bytes: size(&from) };
        assert!(run(&plan, &Ctl::default()).is_err());
        assert!(!is_link(&from));
        assert_eq!(fs::read(from.join("models--org--m/blobs/123")).unwrap().len(), 5000);
        assert_eq!(fs::read(&to).unwrap(), b"in the way");
        assert!(!d.join(".new.filescape-partial").exists());
        // Stopped part way: the same.
        fs::remove_file(&to).unwrap();
        let ctl = Ctl::default();
        ctl.stop.store(true, Ordering::Relaxed);
        assert!(run(&plan, &ctl).unwrap_err().starts_with("Stopped"));
        assert!(!to.exists() && !is_link(&from) && from.join("empty").is_dir());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn checks_refuse_what_cant_work() {
        let d = tempdir("checks");
        let from = d.join("models");
        store(&from);
        // Inside itself.
        assert!(check("LM Studio", &from, &from.join("empty")).unwrap_err().contains("inside"));
        // Same drive (the temp folder is one drive).
        assert!(check("LM Studio", &from, &d).unwrap_err().contains("same drive"));
        // Already there.
        fs::create_dir(d.join(folder_name("LM Studio"))).unwrap();
        assert!(check("LM Studio", &from, &d).unwrap_err().contains("already exists"));
        fs::remove_dir_all(&d).unwrap();
    }
}
