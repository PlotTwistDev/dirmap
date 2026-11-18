//! Copying and moving with name clashes handled, and Undo.
//!
//! `transfer` asks what to do each time an item would land on one that's already there
//! (Replace, Keep Both, Skip, Merge for two folders, or Stop), through a callback, so the UI
//! can ask the user while the work waits on a worker thread. Every change it makes is recorded
//! as a `Step`; `undo` plays the steps back in reverse. Replaced items go to the Recycle Bin or
//! Trash first, and the bin is searched for where they went, so Undo can put them back.

use crate::actions;
use crate::fsys::{archive, remote};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Files at least this big are copied in pieces, so a pause or a stop takes effect within the
/// file and progress moves smoothly; smaller ones are copied by the system in one go.
const CHUNKED_FROM: u64 = 32 << 20;
const CHUNK: usize = 4 << 20;

/// Pause, stop and progress for a running transfer, shared with the UI.
#[derive(Default, Debug)]
pub struct Ctl {
    pub paused: AtomicBool,
    pub stop: AtomicBool,
    /// Bytes to copy and copied (copies, and moves between drives).
    pub bytes_total: AtomicU64,
    pub bytes_done: AtomicU64,
    /// Top-level items to place, and placed.
    pub items_total: AtomicU64,
    pub items_done: AtomicU64,
    /// Still adding up what's to be copied.
    pub counting: AtomicBool,
    /// The file being copied now.
    pub current: Mutex<String>,
}

impl Ctl {
    /// Wait while paused. False once stopped.
    pub fn go_on(&self) -> bool {
        while self.paused.load(Ordering::Relaxed) && !self.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
        !self.stop.load(Ordering::Relaxed)
    }

    fn stopped() -> io::Error {
        io::Error::new(io::ErrorKind::Interrupted, "stopped")
    }
}

/// Bytes in the files under `p`, stopping early if asked to.
fn bytes_in(p: &Path, ctl: &Ctl) -> u64 {
    if remote::is_remote(p) {
        return remote::bytes_in(p, ctl);
    }
    if let Some((_, size, _)) = archive::item(p) {
        return size;
    }
    let Ok(md) = fs::symlink_metadata(p) else { return 0 };
    if md.is_dir() && !md.file_type().is_symlink() {
        let Ok(rd) = fs::read_dir(p) else { return 0 };
        let mut sum = 0;
        for e in rd.flatten() {
            if ctl.stop.load(Ordering::Relaxed) {
                break;
            }
            sum += bytes_in(&e.path(), ctl);
        }
        sum
    } else {
        md.len()
    }
}

/// Copy a file or folder tree, honouring pause and stop and counting progress. The copy keeps
/// each file's permissions and modified time.
pub fn copy_tree(src: &Path, dest: &Path, ctl: &Ctl) -> io::Result<()> {
    if !ctl.go_on() {
        return Err(Ctl::stopped());
    }
    // To or from a server.
    if remote::is_remote(src) || remote::is_remote(dest) {
        return remote::copy_tree(src, dest, ctl);
    }
    // Out of an archive: extracted.
    if archive::inside(src) && archive::item(src).is_some() {
        return archive::extract_path(src, dest, ctl);
    }
    let md = fs::symlink_metadata(src)?;
    if md.is_dir() && !md.file_type().is_symlink() {
        fs::create_dir(dest)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dest.join(e.file_name()), ctl)?;
        }
        Ok(())
    } else if md.is_file() {
        if let Ok(mut c) = ctl.current.lock() {
            *c = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        }
        if md.len() < CHUNKED_FROM {
            fs::copy(src, dest)?;
            ctl.bytes_done.fetch_add(md.len(), Ordering::Relaxed);
            return Ok(());
        }
        let mut from = fs::File::open(src)?;
        let mut to = fs::File::create(dest)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if !ctl.go_on() {
                return Err(Ctl::stopped());
            }
            let n = from.read(&mut buf)?;
            if n == 0 {
                break;
            }
            to.write_all(&buf[..n])?;
            ctl.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
        }
        to.flush()?;
        if let Ok(m) = md.modified() {
            let _ = to.set_modified(m);
        }
        drop(to);
        let _ = fs::set_permissions(dest, md.permissions());
        Ok(())
    } else {
        actions::copy_all(src, dest)
    }
}

/// Where an item went in the Recycle Bin or Trash, to put it back.
#[derive(Clone, Debug, PartialEq)]
pub struct Receipt {
    /// The item itself, inside the bin.
    pub in_bin: PathBuf,
    /// The bin's note of where it came from (removed when it's put back).
    pub info: Option<PathBuf>,
}

/// One change to the disk, recorded so it can be undone.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A new item (a copy, a new folder or file). Undo sends it to the bin.
    Created(PathBuf),
    /// Moved or renamed. Undo moves it back.
    Moved { from: PathBuf, to: PathBuf },
    /// Sent to the bin. Undo puts it back, if the bin could be searched.
    Trashed { original: PathBuf, receipt: Option<Receipt> },
    /// An empty folder removed after its contents were moved into another (Merge). Undo makes
    /// it again.
    RemovedDir(PathBuf),
    /// Deleted for good (replaced on a server, which has no bin). Can't be undone.
    Deleted(PathBuf),
}

/// What's at a path, on disk or on a server: (folder, bytes, modified). Links aren't followed.
fn stat(p: &Path) -> io::Result<(bool, u64, Option<SystemTime>)> {
    if remote::is_remote(p) {
        return remote::meta(p).map(|m| (m.is_dir, m.len, m.modified_time()));
    }
    fs::symlink_metadata(p).map(|md| (md.is_dir(), md.len(), md.modified().ok()))
}

fn exists(p: &Path) -> bool {
    stat(p).is_ok()
}

/// What to do when an item would land on one with the same name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    /// The one that's there goes to the bin, and this one takes its place.
    Replace,
    /// This one gets a free name ("photo 2.jpg").
    KeepBoth,
    /// Leave this one where it is.
    Skip,
    /// Two folders: put this one's contents into the other, asking again about clashes inside.
    Merge,
    /// Stop: nothing more is copied or moved.
    Stop,
}

/// A clash, with what's needed to decide.
#[derive(Clone, Debug)]
pub struct Clash {
    pub src: PathBuf,
    pub dest: PathBuf,
    pub src_dir: bool,
    pub dest_dir: bool,
    pub src_size: u64,
    pub dest_size: u64,
    pub src_modified: Option<SystemTime>,
    pub dest_modified: Option<SystemTime>,
}

/// What a transfer did.
#[derive(Default, Debug)]
pub struct Outcome {
    /// The items as they now are in the destination (top level only).
    pub made: Vec<PathBuf>,
    /// Paths that changed (sources moved away, destinations), to read again.
    pub touched: Vec<PathBuf>,
    pub steps: Vec<Step>,
    pub errors: Vec<String>,
    pub skipped: usize,
    pub stopped: bool,
}

/// Copy or move `sources` into `dest_dir`. `ask` decides each clash.
#[cfg(test)]
pub fn transfer(sources: &[PathBuf], dest_dir: &Path, copy: bool, ask: &mut dyn FnMut(&Clash) -> Choice) -> Outcome {
    transfer_with(sources, dest_dir, copy, ask, &Ctl::default())
}

/// `transfer`, which can be paused and stopped through `ctl` and reports its progress there.
pub fn transfer_with(sources: &[PathBuf], dest_dir: &Path, copy: bool, ask: &mut dyn FnMut(&Clash) -> Choice, ctl: &Ctl) -> Outcome {
    let mut out = Outcome::default();
    ctl.items_total.store(sources.len() as u64, Ordering::Relaxed);
    if copy {
        ctl.counting.store(true, Ordering::Relaxed);
        let total = sources.iter().map(|s| bytes_in(s, ctl)).sum();
        ctl.bytes_total.store(total, Ordering::Relaxed);
        ctl.counting.store(false, Ordering::Relaxed);
    }
    for s in sources {
        if out.stopped || !ctl.go_on() {
            out.stopped = true;
            break;
        }
        if let Some(p) = place(s, dest_dir, copy, ask, &mut out, ctl) {
            out.made.push(p);
        }
        ctl.items_done.fetch_add(1, Ordering::Relaxed);
    }
    out
}

/// Put one item into `dest_dir`. Returns where it ended up.
fn place(src: &Path, dest_dir: &Path, copy: bool, ask: &mut dyn FnMut(&Clash) -> Choice, out: &mut Outcome, ctl: &Ctl) -> Option<PathBuf> {
    let Some(name) = src.file_name().map(|n| n.to_string_lossy().to_string()) else {
        out.errors.push(format!("Can't {} a drive.", if copy { "copy" } else { "move" }));
        return None;
    };
    if dest_dir.starts_with(src) {
        out.errors.push(format!("Can't put {name} inside itself."));
        return None;
    }
    let in_place = src.parent() == Some(dest_dir);
    if in_place && !copy {
        // Moving something to where it already is does nothing.
        return None;
    }
    // A copy into the folder it's in is a duplicate: "name copy.ext", as in Finder and Explorer.
    let target = if in_place { actions::free_name(dest_dir, &name, "copy") } else { dest_dir.join(&name) };
    let Ok((existing_dir, existing_len, existing_modified)) = stat(&target) else {
        return put(src, &target, copy, out, ctl);
    };
    // What's being placed (in an archive, from its table of contents).
    let here = match archive::item(src) {
        Some((dir, size, m)) => Some((dir, size, (m > 0).then(|| std::time::UNIX_EPOCH + Duration::from_secs(m as u64)))),
        None => stat(src).ok(),
    };
    let Some((src_dir, src_size, src_modified)) = here else {
        out.errors.push(format!("{name} isn't there any more."));
        return None;
    };
    let clash = Clash {
        src: src.to_path_buf(),
        dest: target.clone(),
        src_dir,
        dest_dir: existing_dir,
        src_size,
        dest_size: existing_len,
        src_modified,
        dest_modified: existing_modified,
    };
    match ask(&clash) {
        Choice::Skip => {
            out.skipped += 1;
            None
        }
        Choice::Stop => {
            out.stopped = true;
            None
        }
        Choice::KeepBoth => {
            let free = actions::free_name(dest_dir, &name, "");
            put(src, &free, copy, out, ctl)
        }
        Choice::Replace if remote::is_remote(&target) => {
            // A server has no bin: what's there is deleted.
            if let Err(e) = remote::delete(&target) {
                out.errors.push(format!("Couldn't replace {name}: {}", remote::describe(&e)));
                return None;
            }
            out.steps.push(Step::Deleted(target.clone()));
            out.touched.push(target.clone());
            put(src, &target, copy, out, ctl)
        }
        Choice::Replace => {
            match actions::trash_keep(&target) {
                Ok(receipt) => out.steps.push(Step::Trashed { original: target.clone(), receipt }),
                Err(e) => {
                    out.errors.push(format!("Couldn't replace {name}: {e}"));
                    return None;
                }
            }
            out.touched.push(target.clone());
            put(src, &target, copy, out, ctl)
        }
        Choice::Merge if clash.src_dir && clash.dest_dir => {
            let listed = match archive::split(src).filter(|_| archive::inside(src)) {
                Some((a, inner)) => archive::list(&a, &inner).map(|es| es.iter().map(|e| src.join(&*e.name)).collect::<Vec<_>>()).map_err(io::Error::other),
                None if remote::is_remote(src) => remote::read_dir(src),
                None => fs::read_dir(src).map(|rd| rd.flatten().map(|e| e.path()).collect::<Vec<_>>()),
            };
            let entries = match listed {
                Ok(v) => v,
                Err(e) => {
                    out.errors.push(format!("Couldn't read {name}: {e}"));
                    return None;
                }
            };
            for e in entries {
                if out.stopped {
                    break;
                }
                if !ctl.go_on() {
                    out.stopped = true;
                    break;
                }
                place(&e, &target, copy, ask, out, ctl);
            }
            let emptied = if remote::is_remote(src) {
                remote::read_dir(src).is_ok_and(|d| d.is_empty()) && remote::remove_dir(src).is_ok()
            } else {
                fs::read_dir(src).is_ok_and(|mut d| d.next().is_none()) && fs::remove_dir(src).is_ok()
            };
            if !copy && emptied {
                out.steps.push(Step::RemovedDir(src.to_path_buf()));
                out.touched.push(src.to_path_buf());
            }
            out.touched.push(target.clone());
            Some(target)
        }
        Choice::Merge => {
            out.errors.push(format!("{name}: only two folders can be merged."));
            None
        }
    }
}

/// Copy or move to a path that's free.
fn put(src: &Path, to: &Path, copy: bool, out: &mut Outcome, ctl: &Ctl) -> Option<PathBuf> {
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let r = if copy {
        copy_tree(src, to, ctl).map_err(|e| {
            // A stopped copy leaves nothing half-made behind.
            let _ = actions::delete(to);
            if e.kind() == io::ErrorKind::Interrupted { String::new() } else { format!("Couldn't copy {name}: {e}") }
        })
    } else {
        move_with(src, to, ctl)
    };
    if r.as_ref().is_err_and(|e| e.is_empty()) {
        out.stopped = true;
        return None;
    }
    match r {
        Ok(()) => {
            out.steps.push(if copy { Step::Created(to.to_path_buf()) } else { Step::Moved { from: src.to_path_buf(), to: to.to_path_buf() } });
            if !copy {
                out.touched.push(src.to_path_buf());
            }
            out.touched.push(to.to_path_buf());
            Some(to.to_path_buf())
        }
        Err(e) => {
            out.errors.push(e);
            None
        }
    }
}

/// Move a file or folder to a free path, copying then removing the original across drives.
pub fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    move_with(from, to, &Ctl::default())
}

/// `move_path` with pause and stop (an empty error means it was stopped; nothing moved).
fn move_with(from: &Path, to: &Path, ctl: &Ctl) -> Result<(), String> {
    let name = from.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if exists(to) {
        return Err(format!("There's already something at {}.", crate::fsys::PathShow::shown(to)));
    }
    let renamed = if remote::same_site(from, to) {
        remote::rename(from, to)
    } else if remote::is_remote(from) || remote::is_remote(to) {
        // Between a server and here, or two servers: copied, then the original removed.
        Err(io::Error::new(io::ErrorKind::CrossesDevices, "between servers"))
    } else {
        fs::rename(from, to)
    };
    match renamed {
        Ok(()) => Ok(()),
        Err(e) if actions::is_cross_device(&e) => {
            ctl.bytes_total.fetch_add(bytes_in(from, ctl), Ordering::Relaxed);
            if let Err(e) = copy_tree(from, to, ctl) {
                let _ = actions::delete(to);
                return Err(if e.kind() == io::ErrorKind::Interrupted { String::new() } else { format!("Couldn't move {name}: {e}") });
            }
            actions::delete(from).map_err(|e| format!("Moved {name}, but couldn't remove the original: {e}"))
        }
        Err(e) => Err(format!("Couldn't move {name}: {e}")),
    }
}

/// Undo recorded steps, last first. Returns the problems (empty when all went back) and the
/// paths that changed.
pub fn undo(steps: &[Step]) -> (Vec<String>, Vec<PathBuf>) {
    let mut errors = Vec::new();
    let mut touched = Vec::new();
    for s in steps.iter().rev() {
        let r = match s {
            Step::Created(p) => {
                touched.push(p.clone());
                if !exists(p) {
                    Ok(())
                } else if remote::is_remote(p) {
                    remote::delete(p).map_err(|e| format!("Couldn't remove {}: {}", crate::fsys::PathShow::shown(p.as_path()), remote::describe(&e)))
                } else {
                    actions::trash(p).map_err(|e| format!("Couldn't remove {}: {e}", p.display()))
                }
            }
            Step::Moved { from, to } => {
                touched.push(from.clone());
                touched.push(to.clone());
                if !exists(to) {
                    Err(format!("{} isn't there any more.", crate::fsys::PathShow::shown(to.as_path())))
                } else {
                    move_path(to, from)
                }
            }
            Step::Trashed { original, receipt } => {
                touched.push(original.clone());
                match receipt {
                    Some(r) => restore(r, original),
                    None => Err(format!(
                        "{} couldn't be found in the {}. Put it back from there yourself.",
                        original.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                        actions::BIN
                    )),
                }
            }
            Step::RemovedDir(p) => {
                touched.push(p.clone());
                if exists(p) {
                    Ok(())
                } else if remote::is_remote(p) {
                    remote::create_dir_all(p).map_err(|e| format!("Couldn't make {} again: {e}", crate::fsys::PathShow::shown(p.as_path())))
                } else {
                    fs::create_dir_all(p).map_err(|e| format!("Couldn't make {} again: {e}", p.display()))
                }
            }
            Step::Deleted(p) => {
                touched.push(p.clone());
                Err(format!("{} was deleted on the server when it was replaced, and can't be brought back.", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
            }
        };
        if let Err(e) = r {
            errors.push(e);
        }
    }
    (errors, touched)
}

/// Put an item back from the bin.
pub fn restore(r: &Receipt, original: &Path) -> Result<(), String> {
    if fs::symlink_metadata(&r.in_bin).is_err() {
        return Err(format!("{} isn't in the {} any more.", original.display(), actions::BIN));
    }
    if let Some(parent) = original.parent() {
        let _ = fs::create_dir_all(parent);
    }
    move_path(&r.in_bin, original)?;
    if let Some(info) = &r.info {
        let _ = fs::remove_file(info);
    }
    Ok(())
}

/// Describe the steps for a menu: "Undo Move of 3 items".
pub fn describe(steps: &[Step]) -> String {
    let n = |k: &dyn Fn(&Step) -> bool| steps.iter().filter(|s| k(s)).count();
    let moved = n(&|s| matches!(s, Step::Moved { .. }));
    let made = n(&|s| matches!(s, Step::Created(_)));
    let binned = n(&|s| matches!(s, Step::Trashed { .. }));
    let items = |c: usize| if c == 1 { "1 item".to_string() } else { format!("{c} items") };
    if moved > 0 && steps.iter().all(|s| matches!(s, Step::Moved { from, to } if from.parent() == to.parent())) {
        "Rename".into()
    } else if moved > 0 {
        format!("Move of {}", items(moved))
    } else if made > 0 && binned == 0 && steps.len() == 1 {
        "New Item".into()
    } else if made > 0 {
        format!("Copy of {}", items(made))
    } else if binned > 0 {
        format!("Delete of {}", items(binned))
    } else {
        "Last Change".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dirmap-transfer-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn pause_and_stop() {
        let d = tmp("pause");
        let src = d.join("big.bin");
        let f = fs::File::create(&src).unwrap();
        f.set_len(40 << 20).unwrap();
        drop(f);
        let dest = d.join("to");
        fs::create_dir(&dest).unwrap();
        // Paused: nothing is copied until it's resumed.
        let ctl = std::sync::Arc::new(Ctl::default());
        ctl.paused.store(true, Ordering::Relaxed);
        let (c, s2, d2) = (ctl.clone(), src.clone(), dest.clone());
        let h = std::thread::spawn(move || transfer_with(&[s2], &d2, true, &mut |_| Choice::Stop, &c));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(ctl.bytes_done.load(Ordering::Relaxed), 0);
        assert_eq!(ctl.bytes_total.load(Ordering::Relaxed), 40 << 20);
        ctl.paused.store(false, Ordering::Relaxed);
        let out = h.join().unwrap();
        assert!(out.errors.is_empty() && !out.stopped, "{out:?}");
        assert_eq!(fs::metadata(dest.join("big.bin")).unwrap().len(), 40 << 20);
        assert_eq!(fs::metadata(dest.join("big.bin")).unwrap().modified().unwrap(), fs::metadata(&src).unwrap().modified().unwrap());
        // Stopped: nothing half-made is left, and it isn't an error.
        let ctl = Ctl::default();
        ctl.stop.store(true, Ordering::Relaxed);
        let out = transfer_with(&[src.clone()], &d.join("to"), true, &mut |_| Choice::KeepBoth, &ctl);
        assert!(out.stopped && out.errors.is_empty() && out.made.is_empty(), "{out:?}");
        let stopped_dir = d.join("to2");
        fs::create_dir(&stopped_dir).unwrap();
        let ctl = Ctl::default();
        ctl.stop.store(true, Ordering::Relaxed);
        assert!(copy_tree(&src, &stopped_dir.join("x"), &ctl).is_err());
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn clashes_are_asked_and_undone() {
        let d = tmp("clash");
        let (a, b) = (d.join("a"), d.join("b"));
        fs::create_dir_all(a.join("sub")).unwrap();
        fs::create_dir_all(b.join("sub")).unwrap();
        fs::write(a.join("x.txt"), "new").unwrap();
        fs::write(b.join("x.txt"), "old").unwrap();
        fs::write(a.join("y.txt"), "y").unwrap();
        fs::write(a.join("sub/z.txt"), "z").unwrap();
        fs::write(b.join("sub/keep.txt"), "k").unwrap();

        // Keep Both for the file, Merge for the folder; y.txt has no clash.
        let mut asked = Vec::new();
        let srcs = vec![a.join("x.txt"), a.join("y.txt"), a.join("sub")];
        let out = transfer(&srcs, &b, true, &mut |c| {
            asked.push(c.dest.file_name().unwrap().to_string_lossy().to_string());
            if c.src_dir { Choice::Merge } else { Choice::KeepBoth }
        });
        assert_eq!(asked, ["x.txt", "sub"]);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(fs::read_to_string(b.join("x 2.txt")).unwrap(), "new");
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "old");
        assert!(b.join("y.txt").exists() && b.join("sub/z.txt").exists() && b.join("sub/keep.txt").exists());
        // Undo removes the copies only.
        let (errs, _) = undo(&out.steps);
        assert!(errs.is_empty() || errs.iter().all(|e| e.contains("Trash")), "{errs:?}");
        if errs.is_empty() {
            assert!(!b.join("x 2.txt").exists() && !b.join("y.txt").exists() && !b.join("sub/z.txt").exists());
            assert!(b.join("x.txt").exists() && b.join("sub/keep.txt").exists());
        }
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn moves_skip_stop_and_undo() {
        let d = tmp("move");
        let (a, b) = (d.join("a"), d.join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        for n in ["1.txt", "2.txt", "3.txt"] {
            fs::write(a.join(n), n).unwrap();
        }
        fs::write(b.join("2.txt"), "there").unwrap();
        fs::write(b.join("3.txt"), "there").unwrap();
        let srcs: Vec<PathBuf> = ["1.txt", "2.txt", "3.txt"].iter().map(|n| a.join(n)).collect();
        let mut n = 0;
        let out = transfer(&srcs, &b, false, &mut |_| {
            n += 1;
            if n == 1 { Choice::Skip } else { Choice::Stop }
        });
        assert_eq!((out.skipped, out.stopped), (1, true));
        assert!(b.join("1.txt").exists() && !a.join("1.txt").exists());
        assert!(a.join("2.txt").exists() && a.join("3.txt").exists());
        assert_eq!(fs::read_to_string(b.join("2.txt")).unwrap(), "there");
        // Undo moves 1.txt back.
        let (errs, _) = undo(&out.steps);
        assert!(errs.is_empty(), "{errs:?}");
        assert!(a.join("1.txt").exists() && !b.join("1.txt").exists());
        // Moving to where it is does nothing; into itself is refused.
        let out = transfer(&[a.join("1.txt")], &a, false, &mut |_| Choice::Replace);
        assert!(out.steps.is_empty() && out.errors.is_empty());
        let out = transfer(std::slice::from_ref(&a), &a, false, &mut |_| Choice::Replace);
        assert_eq!(out.errors.len(), 1);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn copy_in_place_makes_a_duplicate() {
        let d = tmp("dup");
        fs::write(d.join("f.txt"), "f").unwrap();
        let out = transfer(&[d.join("f.txt")], &d, true, &mut |_| panic!("no clash expected"));
        assert_eq!(out.made, vec![d.join("f copy.txt")]);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn rename_undo_and_descriptions() {
        let d = tmp("ren");
        let steps = vec![Step::Moved { from: d.join("a"), to: d.join("b") }];
        assert_eq!(describe(&steps), "Rename");
        assert_eq!(describe(&[Step::Moved { from: d.join("a/x"), to: d.join("b/x") }, Step::Moved { from: d.join("a/y"), to: d.join("b/y") }]), "Move of 2 items");
        assert_eq!(describe(&[Step::Created(d.join("n"))]), "New Item");
        fs::write(d.join("b"), "b").unwrap();
        let (errs, _) = undo(&steps);
        assert!(errs.is_empty());
        assert!(d.join("a").exists() && !d.join("b").exists());
        // Gone since: says so, and changes nothing.
        let (errs, _) = undo(&steps);
        assert_eq!(errs.len(), 1);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn restore_puts_back_and_tidies_the_note() {
        let d = tmp("restore");
        let bin = d.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("f.txt"), "f").unwrap();
        fs::write(bin.join("f.txt.info"), "x").unwrap();
        let r = Receipt { in_bin: bin.join("f.txt"), info: Some(bin.join("f.txt.info")) };
        restore(&r, &d.join("back/f.txt")).unwrap();
        assert!(d.join("back/f.txt").exists() && !bin.join("f.txt.info").exists());
        assert!(restore(&r, &d.join("back/f.txt")).is_err());
        fs::remove_dir_all(&d).ok();
    }
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod bin_tests {
    use super::*;

    /// With a real Trash (gio): replacing sends the old file there, and Undo puts it back.
    #[test]
    fn replace_then_undo_restores_from_the_trash() {
        if std::process::Command::new("gio").arg("version").output().is_err() {
            return;
        }
        let d = std::env::temp_dir().join(format!("dirmap-bin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let (a, b) = (d.join("a"), d.join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("shot.exr"), "new").unwrap();
        fs::write(b.join("shot.exr"), "old").unwrap();
        let out = transfer(&[a.join("shot.exr")], &b, false, &mut |_| Choice::Replace);
        if !out.errors.is_empty() {
            // No usable Trash here (gio without a session): nothing to check.
            eprintln!("skipped: {:?}", out.errors);
            fs::remove_dir_all(&d).ok();
            return;
        }
        assert_eq!(fs::read_to_string(b.join("shot.exr")).unwrap(), "new");
        assert!(matches!(&out.steps[0], Step::Trashed { receipt: Some(_), .. }), "{:?}", out.steps);
        let (errs, _) = undo(&out.steps);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(fs::read_to_string(b.join("shot.exr")).unwrap(), "old");
        assert_eq!(fs::read_to_string(a.join("shot.exr")).unwrap(), "new");
        fs::remove_dir_all(&d).ok();
    }
}
