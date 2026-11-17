//! What the New menu can make: dirmap's built-in starting files, whose contents are shown in
//! the menu's tooltips, and the user's own templates (any file or folder in a templates folder,
//! copied as it is).
//!
//! A template is always a real file. dirmap never makes an empty `.psd` or `.blend` and calls
//! it a document: for those, put a real one in the templates folder.

use std::path::{Path, PathBuf};

/// A starting file built into dirmap.
#[derive(Clone, Copy, Debug)]
pub struct Builtin {
    /// Menu label.
    pub label: &'static str,
    /// The new file's name.
    pub name: &'static str,
    pub contents: &'static str,
    /// Mark it executable (Unix scripts).
    pub executable: bool,
}

const TEXT: Builtin = Builtin { label: "Text Document", name: "New Text Document.txt", contents: "", executable: false };
const MARKDOWN: Builtin = Builtin { label: "Markdown", name: "README.md", contents: "# Title\n\n", executable: false };
const JSON: Builtin = Builtin { label: "JSON", name: "data.json", contents: "{\n}\n", executable: false };
const XML: Builtin = Builtin { label: "XML", name: "data.xml", contents: "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<root>\n</root>\n", executable: false };
const CSV: Builtin = Builtin { label: "CSV", name: "data.csv", contents: "name,value\n", executable: false };
const SH: Builtin = Builtin { label: "Shell Script", name: "script.sh", contents: "#!/bin/sh\nset -eu\n\n", executable: true };
const PY: Builtin = Builtin { label: "Python Script", name: "script.py", contents: "#!/usr/bin/env python3\n\n\ndef main():\n    pass\n\n\nif __name__ == \"__main__\":\n    main()\n", executable: true };
const PS1: Builtin = Builtin { label: "PowerShell Script", name: "script.ps1", contents: "#Requires -Version 5.1\nSet-StrictMode -Version Latest\n\n", executable: false };
const BAT: Builtin = Builtin { label: "Batch File", name: "script.cmd", contents: "@echo off\r\nsetlocal\r\n\r\n", executable: false };

/// Documents, then scripts for this system.
pub fn builtins() -> (&'static [Builtin], &'static [Builtin]) {
    const DOCS: &[Builtin] = &[TEXT, MARKDOWN, JSON, XML, CSV];
    const WIN: &[Builtin] = &[PS1, BAT, PY];
    const UNIX: &[Builtin] = &[SH, PY];
    (DOCS, if cfg!(windows) { WIN } else { UNIX })
}

/// A file or folder in a templates folder.
#[derive(Clone, Debug, PartialEq)]
pub struct UserTemplate {
    pub path: PathBuf,
    /// What the menu shows: the name without its extension.
    pub label: String,
    pub is_dir: bool,
}

/// dirmap's own templates folder (made on first use by "Open Templates Folder").
pub fn own_folder() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        crate::fsys::places::home().map(|h| h.join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| crate::fsys::places::home().map(|h| h.join(".local/share")))
    }?;
    Some(base.join("dirmap").join("Templates"))
}

/// Folders to look in: dirmap's own, and on Linux the desktop's Templates folder (which GNOME
/// Files and Dolphin use too).
pub fn folders() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = own_folder().into_iter().collect();
    if cfg!(all(unix, not(target_os = "macos")))
        && let Some(h) = crate::fsys::places::home()
    {
        let dirs = std::fs::read_to_string(h.join(".config/user-dirs.dirs")).unwrap_or_default();
        let t = crate::fsys::places::parse_user_dirs(&dirs, &h).into_iter().find(|(k, _)| k == "TEMPLATES").map(|(_, p)| p).unwrap_or_else(|| h.join("Templates"));
        if t != h {
            out.push(t);
        }
    }
    out
}

/// The user's templates, sorted by label. Reads the disk: call it off the UI thread.
pub fn user_templates(folders: &[PathBuf]) -> Vec<UserTemplate> {
    let mut out = Vec::new();
    for f in folders {
        let Ok(rd) = std::fs::read_dir(f) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            out.push(UserTemplate { label: label_of(&name, is_dir), path: e.path(), is_dir });
        }
    }
    out.sort_by_key(|t| t.label.to_lowercase());
    out.dedup_by(|a, b| a.label == b.label);
    out
}

fn label_of(name: &str, is_dir: bool) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => name[..i].to_string(),
        _ => name.to_string(),
    }
}

/// The name a new copy of a template gets.
pub fn new_name(t: &Path) -> String {
    t.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "New file".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid_for_their_type() {
        let (docs, scripts) = builtins();
        for b in docs.iter().chain(scripts) {
            assert!(b.name.contains('.'), "{} has an extension", b.name);
            if b.name.ends_with(".xml") {
                assert!(b.contents.starts_with("<?xml") && b.contents.contains("<root>") && b.contents.contains("</root>"));
            }
            if b.name.ends_with(".sh") || b.name.ends_with(".py") {
                assert!(b.contents.starts_with("#!"));
            }
        }
        // JSON parses as an empty object.
        let j: String = JSON.contents.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(j, "{}");
        assert_eq!(CSV.contents.lines().next().unwrap().split(',').count(), 2);
    }

    #[test]
    fn user_templates_listed_by_label() {
        let d = std::env::temp_dir().join(format!("dirmap-tpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Project")).unwrap();
        std::fs::write(d.join("Shot notes.md"), "x").unwrap();
        std::fs::write(d.join(".hidden"), "x").unwrap();
        let t = user_templates(std::slice::from_ref(&d));
        let labels: Vec<&str> = t.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["Project", "Shot notes"]);
        assert!(t[0].is_dir);
        assert_eq!(new_name(&t[1].path), "Shot notes.md");
        let _ = std::fs::remove_dir_all(&d);
    }
}
