//! Colours, tags and notes on files and folders. They're dirmap's own: kept in a small file in
//! dirmap's data folder, keyed by path, not written into the items or their folders. Renames
//! and moves made in dirmap carry them along; an item renamed or moved by another program loses
//! them (the manual says so).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The colours offered, in menu order: (name, RGB).
pub const COLOURS: [(&str, [u8; 3]); 7] = [
    ("Red", [230, 72, 72]),
    ("Orange", [240, 150, 50]),
    ("Yellow", [230, 200, 40]),
    ("Green", [80, 190, 90]),
    ("Blue", [70, 140, 240]),
    ("Purple", [170, 100, 220]),
    ("Grey", [150, 150, 150]),
];

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mark {
    /// Index into `COLOURS`.
    pub colour: Option<u8>,
    pub tags: Vec<String>,
    pub note: String,
}

impl Mark {
    fn is_empty(&self) -> bool {
        self.colour.is_none() && self.tags.is_empty() && self.note.trim().is_empty()
    }
}

#[derive(Default)]
pub struct Marks {
    map: HashMap<PathBuf, Mark>,
    file: Option<PathBuf>,
}

fn store() -> Option<PathBuf> {
    crate::templates::own_folder().and_then(|t| t.parent().map(|d| d.join("marks.ron")))
}

impl Marks {
    /// Read the saved marks (dirmap's own small file, never a user folder).
    pub fn load() -> Marks {
        let file = store();
        let map = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| ron::de::from_bytes::<Vec<(PathBuf, Mark)>>(&b).ok())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default();
        Marks { map, file }
    }

    #[cfg(test)]
    pub fn at(file: PathBuf) -> Marks {
        Marks {
            map: HashMap::new(),
            file: Some(file),
        }
    }

    pub fn get(&self, p: &Path) -> Option<&Mark> {
        self.map.get(p)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Change an item's mark (an empty mark removes it), and save.
    pub fn set(&mut self, p: &Path, m: Mark) {
        if m.is_empty() {
            self.map.remove(p);
        } else {
            self.map.insert(p.to_path_buf(), m);
        }
        self.save();
    }

    pub fn edit(&mut self, p: &Path, f: impl FnOnce(&mut Mark)) {
        let mut m = self.map.get(p).cloned().unwrap_or_default();
        f(&mut m);
        self.set(p, m);
    }

    /// An item moved or renamed: its mark, and those of everything inside it, follow.
    pub fn moved(&mut self, from: &Path, to: &Path) {
        let keys: Vec<PathBuf> = self
            .map
            .keys()
            .filter(|k| k.starts_with(from))
            .cloned()
            .collect();
        if keys.is_empty() {
            return;
        }
        for k in keys {
            if let (Some(m), Ok(rest)) = (self.map.remove(&k), k.strip_prefix(from)) {
                let new = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
                self.map.insert(new, m);
            }
        }
        self.save();
    }

    /// Every tag in use, sorted.
    pub fn tags(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .map
            .values()
            .flat_map(|m| m.tags.iter().cloned())
            .collect();
        set.into_iter().collect()
    }

    /// Items with this tag (case ignored).
    pub fn tagged(&self, tag: &str) -> Vec<PathBuf> {
        let tag = tag.to_lowercase();
        self.map
            .iter()
            .filter(|(_, m)| m.tags.iter().any(|t| t.to_lowercase() == tag))
            .map(|(p, _)| p.clone())
            .collect()
    }

    fn save(&self) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let mut v: Vec<(&PathBuf, &Mark)> = self.map.iter().collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        let Ok(bytes) = ron::ser::to_string_pretty(&v, ron::ser::PrettyConfig::default()) else {
            return;
        };
        // Written beside, then renamed over, so a crash never leaves half a file. Saves are
        // numbered so an older one finishing late never overwrites a newer one.
        static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let me = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::thread::spawn(move || {
            let mut last = LAST.lock().unwrap();
            if *last > me {
                return;
            }
            *last = me;
            if let Some(d) = file.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let part = file.with_extension("ron.part");
            if std::fs::write(&part, bytes).is_ok() {
                let _ = std::fs::rename(&part, &file);
            }
        });
    }
}

/// Tags typed as text: split on commas, `#` dropped, blanks and repeats removed.
pub fn parse_tags(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in s.split(',') {
        let t = t.trim().trim_start_matches('#').trim();
        if !t.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    }
    out
}

/// The common folder of some paths (where a tag's items are listed from).
pub fn common_folder(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?.parent()?.to_path_buf();
    let mut base = first;
    for p in &paths[1..] {
        while !p.starts_with(&base) {
            base = base.parent()?.to_path_buf();
        }
    }
    Some(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_follow_moves_and_save() {
        let d = std::env::temp_dir().join(format!("dirmap-marks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let mut m = Marks::at(d.join("marks.ron"));
        m.edit(Path::new("/a/shot"), |k| k.colour = Some(1));
        m.edit(Path::new("/a/shot/plate.exr"), |k| {
            k.tags = parse_tags("#Final, client ,final,")
        });
        assert_eq!(
            m.get(Path::new("/a/shot/plate.exr")).unwrap().tags,
            ["Final", "client"]
        );
        m.moved(Path::new("/a/shot"), Path::new("/b/shot2"));
        assert_eq!(m.get(Path::new("/b/shot2")).unwrap().colour, Some(1));
        assert!(
            m.get(Path::new("/b/shot2/plate.exr")).is_some()
                && m.get(Path::new("/a/shot")).is_none()
        );
        // A path that only starts with the same letters isn't inside.
        m.edit(Path::new("/b/shot22"), |k| k.note = "x".into());
        m.moved(Path::new("/b/shot2"), Path::new("/c"));
        assert!(m.get(Path::new("/b/shot22")).is_some());
        assert_eq!(m.tagged("FINAL"), [PathBuf::from("/c/plate.exr")]);
        assert_eq!(m.tags(), ["Final", "client"]);
        m.edit(Path::new("/b/shot22"), |k| k.note = " ".into());
        assert!(m.get(Path::new("/b/shot22")).is_none());
        std::thread::sleep(std::time::Duration::from_millis(200));
        let saved: Vec<(PathBuf, Mark)> =
            ron::de::from_bytes(&std::fs::read(d.join("marks.ron")).unwrap()).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(
            common_folder(&[PathBuf::from("/x/a/1"), PathBuf::from("/x/b/2")]),
            Some(PathBuf::from("/x"))
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
