//! Drag and drop rules shared by every view: whether a drop moves or copies, and the label
//! by the pointer that says so before the button is let go.
//!
//! As in Explorer and Finder: on the same drive a drop moves, onto another drive (or a
//! network location) it copies. Ctrl (⌥ on macOS) forces a copy, Shift (⌘ on macOS) a move.
//! Alt still copies on Windows and Linux, as it did in dirmap before.

use super::*;
use crate::drives::Drive;
use std::path::Path;

/// Items being dragged (files and folders from a list, a tile or a favourite's folder).
pub(super) use super::browser::DragItems;

/// A favourite being dragged within the sidebar (group, index).
#[derive(Clone, Copy, Debug)]
pub(super) struct DragFav(pub usize, pub usize);

/// What a drop will do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DropOp {
    Move,
    Copy,
}

/// The root of the drive or volume a path is on: the drive letter or `\\server\share` on
/// Windows, or the longest mount point that contains it elsewhere. Worked out from the path
/// and the drive list only, so it never touches the disk (or a slow share) while dragging.
pub(super) fn volume_of(path: &Path, drives: &[Drive]) -> PathBuf {
    #[cfg(windows)]
    {
        let _ = drives;
        use std::path::Component;
        if let Some(Component::Prefix(p)) = path.components().next() {
            return PathBuf::from(p.as_os_str().to_ascii_uppercase());
        }
        PathBuf::new()
    }
    #[cfg(not(windows))]
    {
        drives
            .iter()
            .map(|d| d.root.as_path())
            .filter(|r| path.starts_with(r))
            .max_by_key(|r| r.as_os_str().len())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"))
    }
}

/// Move or copy, for dropping `sources` into `dest` with these keys held.
pub(super) fn op_for(sources: &[PathBuf], dest: &Path, m: egui::Modifiers, drives: &[Drive]) -> DropOp {
    let (copy_key, move_key) = if cfg!(target_os = "macos") { (m.alt, m.mac_cmd) } else { (m.ctrl || m.alt, m.shift) };
    if copy_key {
        return DropOp::Copy;
    }
    if move_key {
        return DropOp::Move;
    }
    let to = volume_of(dest, drives);
    if sources.iter().all(|s| volume_of(s, drives) == to) { DropOp::Move } else { DropOp::Copy }
}

/// Dropping these here would put something inside itself, or where it already is.
pub(super) fn pointless(sources: &[PathBuf], dest: &Path, op: DropOp) -> bool {
    sources.iter().any(|s| dest.starts_with(s)) || (op == DropOp::Move && sources.iter().all(|s| s.parent() == Some(dest)))
}

/// "report.pdf" or "3 items".
pub(super) fn what(items: &[PathBuf]) -> String {
    match items {
        [one] => one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| one.display().to_string()),
        many => format!("{} items", many.len()),
    }
}

/// A folder's name for labels: "Projects", or "D:\" for a drive.
pub(super) fn folder_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

/// The keys that change a drop, for the label.
pub(super) fn keys_hint(op: DropOp) -> &'static str {
    match (op, cfg!(target_os = "macos")) {
        (DropOp::Move, true) => "hold ⌥ to copy",
        (DropOp::Move, false) => "hold Ctrl to copy",
        (DropOp::Copy, true) => "hold ⌘ to move",
        (DropOp::Copy, false) => "hold Shift to move",
    }
}

/// A label next to the pointer while dragging.
pub(super) fn pointer_label(ctx: &egui::Context, text: &str, warn: bool) {
    let Some(p) = ctx.pointer_latest_pos() else { return };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dnd-label")));
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(13.0), Color32::WHITE);
    let r = Rect::from_min_size(p + vec2(14.0, 10.0), galley.size() + vec2(12.0, 6.0));
    let bg = if warn { Color32::from_rgba_unmultiplied(120, 70, 30, 235) } else { Color32::from_rgba_unmultiplied(30, 90, 170, 235) };
    painter.rect_filled(r, 4.0, bg);
    painter.galley(r.min + vec2(6.0, 3.0), galley, Color32::WHITE);
    ctx.request_repaint();
}

/// The label for dropping items into a folder.
pub(super) fn drop_text(items: &[PathBuf], dest: &Path, op: DropOp) -> (String, bool) {
    if pointless(items, dest, op) {
        return (format!("Can't {} {} here", if op == DropOp::Copy { "copy" } else { "move" }, what(items)), true);
    }
    let verb = if op == DropOp::Copy { "Copy" } else { "Move" };
    (format!("{verb} {} to {}  ({})", what(items), folder_name(dest), keys_hint(op)), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(root: &str) -> Drive {
        Drive { root: root.into(), label: String::new(), total: 1, free: 1, network: false }
    }

    #[test]
    fn same_drive_moves_other_drive_copies() {
        let none = egui::Modifiers::NONE;
        if cfg!(windows) {
            let d: Vec<Drive> = Vec::new();
            assert_eq!(op_for(&[PathBuf::from(r"C:\a\x.txt")], Path::new(r"c:\b"), none, &d), DropOp::Move);
            assert_eq!(op_for(&[PathBuf::from(r"C:\a\x.txt")], Path::new(r"D:\b"), none, &d), DropOp::Copy);
            assert_eq!(op_for(&[PathBuf::from(r"\\nas\share\x")], Path::new(r"C:\b"), none, &d), DropOp::Copy);
        } else {
            let d = vec![drive("/"), drive("/mnt/nas")];
            assert_eq!(op_for(&[PathBuf::from("/home/a/x.txt")], Path::new("/tmp"), none, &d), DropOp::Move);
            assert_eq!(op_for(&[PathBuf::from("/home/a/x.txt")], Path::new("/mnt/nas/b"), none, &d), DropOp::Copy);
            assert_eq!(op_for(&[PathBuf::from("/mnt/nas/x")], Path::new("/mnt/nas/y"), none, &d), DropOp::Move);
        }
    }

    #[test]
    fn keys_force_the_action() {
        let d = vec![drive("/")];
        let copy = if cfg!(target_os = "macos") { egui::Modifiers { alt: true, ..Default::default() } } else { egui::Modifiers { ctrl: true, ..Default::default() } };
        let mv = if cfg!(target_os = "macos") { egui::Modifiers { mac_cmd: true, ..Default::default() } } else { egui::Modifiers { shift: true, ..Default::default() } };
        let src = if cfg!(windows) { PathBuf::from(r"C:\a") } else { PathBuf::from("/a") };
        let other = if cfg!(windows) { PathBuf::from(r"D:\b") } else { PathBuf::from("/b") };
        assert_eq!(op_for(std::slice::from_ref(&src), &other, copy, &d), DropOp::Copy);
        assert_eq!(op_for(&[src], &other, mv, &d), DropOp::Move);
    }

    #[test]
    fn no_drops_into_themselves() {
        let a = PathBuf::from("/x/a");
        assert!(pointless(std::slice::from_ref(&a), Path::new("/x/a/b"), DropOp::Copy));
        assert!(pointless(std::slice::from_ref(&a), Path::new("/x"), DropOp::Move));
        assert!(!pointless(std::slice::from_ref(&a), Path::new("/x"), DropOp::Copy));
        assert!(!pointless(&[a], Path::new("/y"), DropOp::Move));
    }
}
