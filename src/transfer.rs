//! Copying and moving with name clashes handled, and Undo.
//!
//! `transfer` asks what to do each time an item would land on one that's already there
//! (Replace, Keep Both, Skip, Merge for two folders, or Stop), through a callback, so the UI
//! can ask the user while the work waits on a worker thread. Every change it makes is recorded
//! as a `Step`; `undo` plays the steps back in reverse. Replaced items go to the Recycle Bin or
//! Trash first, and the bin is searched for where they went, so Undo can put them back.

use crate::actions;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
pub fn transfer(sources: &[PathBuf], dest_dir: &Path, copy: bool, ask: &mut dyn FnMut(&Clash) -> Choice) -> Outcome {
    let mut out = Outcome::default();
    for s in sources {
        if out.stopped {
            break;
        }
        if let Some(p) = place(s, dest_dir, copy, ask, &mut out) {
            out.made.push(p);
        }
    }
    out
}

/// Put one item into `dest_dir`. Returns where it ended up.
fn place(src: &Path, dest_dir: &Path, copy: bool, ask: &mut dyn FnMut(&Clash) -> Choice, out: &mut Outcome) -> Option<PathBuf> {
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
    let Ok(existing) = fs::symlink_metadata(&target) else {
        return put(src, &target, copy, out);
    };
    let Ok(here) = fs::symlink_metadata(src) else {
        out.errors.push(format!("{name} isn't there any more."));
        return None;
    };
    let clash = Clash {
        src: src.to_path_buf(),
        dest: target.clone(),
        src_dir: here.is_dir(),
        dest_dir: existing.is_dir(),
        src_size: here.len(),
        dest_size: existing.len(),
        src_modified: here.modified().ok(),
        dest_modified: existing.modified().ok(),
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
            put(src, &free, copy, out)
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
            put(src, &target, copy, out)
        }
        Choice::Merge if clash.src_dir && clash.dest_dir => {
            let entries = match fs::read_dir(src) {
                Ok(rd) => rd.flatten().map(|e| e.path()).collect::<Vec<_>>(),
                Err(e) => {
                    out.errors.push(format!("Couldn't read {name}: {e}"));
                    return None;
                }
            };
            for e in entries {
                if out.stopped {
                    break;
                }
                place(&e, &target, copy, ask, out);
            }
            if !copy && fs::read_dir(src).is_ok_and(|mut d| d.next().is_none()) && fs::remove_dir(src).is_ok() {
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
fn put(src: &Path, to: &Path, copy: bool, out: &mut Outcome) -> Option<PathBuf> {
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let r = if copy {
        actions::copy_all(src, to).map_err(|e| {
            let _ = actions::delete(to);
            format!("Couldn't copy {name}: {e}")
        })
    } else {
        move_path(src, to)
    };
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
    let name = from.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if fs::symlink_metadata(to).is_ok() {
        return Err(format!("There's already something at {}.", to.display()));
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if actions::is_cross_device(&e) => {
            if let Err(e) = actions::copy_all(from, to) {
                let _ = actions::delete(to);
                return Err(format!("Couldn't move {name}: {e}"));
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
                if fs::symlink_metadata(p).is_err() {
                    Ok(())
                } else {
                    actions::trash(p).map_err(|e| format!("Couldn't remove {}: {e}", p.display()))
                }
            }
            Step::Moved { from, to } => {
                touched.push(from.clone());
                touched.push(to.clone());
                if fs::symlink_metadata(to).is_err() {
                    Err(format!("{} isn't there any more.", to.display()))
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
                if p.exists() { Ok(()) } else { fs::create_dir_all(p).map_err(|e| format!("Couldn't make {} again: {e}", p.display())) }
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
