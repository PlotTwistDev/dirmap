//! The files a project uses (footage, sound files, pictures), as the project names them:
//! whether each is on this computer, only in the cloud, on another drive or on a network drive,
//! and how much space they take together.
//!
//! Only each file's details are read (`stat`), never its contents, so a file that's only in the
//! cloud isn't downloaded. Nothing is offered for deletion: a project's own list is no proof a
//! file isn't used by something else.

use crate::help::doc::{Block, Span};
use crate::scan::store;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Files checked; a project naming more says how many weren't.
const CHECK_MAX: usize = 5000;
/// Rows listed.
const LIST_MAX: usize = 300;
/// What a link to a file starts with: following one shows the file in Files.
pub const SHOW: &str = "filescape://show/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// On the same drive as the project.
    Here,
    /// On another drive of this computer.
    OtherDrive,
    /// On a network drive.
    Network,
    /// In a cloud storage folder, downloaded.
    Cloud,
    /// In a cloud storage folder, not downloaded: opening it would download it.
    CloudOnly,
    /// Not found where the project says it is.
    Missing,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Here => "This drive",
            State::OtherDrive => "Another drive",
            State::Network => "Network drive",
            State::Cloud => "Cloud, downloaded",
            State::CloudOnly => "Only in the cloud",
            State::Missing => "Not found",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ref {
    /// As the project names it.
    pub named: String,
    /// Where it was looked for.
    pub path: PathBuf,
    pub state: State,
    /// Its size, 0 when it's missing.
    pub size: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Refs {
    /// What the files are called in this app: "Sound files", "Footage", "Media".
    pub what: &'static str,
    pub items: Vec<Ref>,
    /// Names the project lists beyond `CHECK_MAX`, not checked.
    pub unchecked: usize,
    /// Size of the files found, each physical file counted once.
    pub total: u64,
}

impl Refs {
    pub fn count(&self, s: State) -> usize {
        self.items.iter().filter(|r| r.state == s).count()
    }

    /// One line for the details: how many, how many aren't here, and their size.
    pub fn fact(&self) -> String {
        let n = self.items.len() + self.unchecked;
        let mut parts = Vec::new();
        let gone = self.count(State::Missing);
        if gone > 0 {
            parts.push(format!("{} not found", group(gone)));
        }
        let cloud = self.count(State::CloudOnly);
        if cloud > 0 {
            parts.push(format!("{} only in the cloud", group(cloud)));
        }
        let mut s = group(n);
        if !parts.is_empty() {
            s.push_str(&format!(" ({})", parts.join(", ")));
        }
        if self.total > 0 {
            s.push_str(&format!(", {}", crate::scan::human(self.total)));
        }
        s
    }

    /// A heading, a line saying what was found, and a table of the files with links that show
    /// each one in Files.
    pub fn blocks(&self) -> Vec<Block> {
        if self.items.is_empty() && self.unchecked == 0 {
            return Vec::new();
        }
        let mut out = vec![Block::Heading { level: 2, spans: plain(self.what), id: String::new() }];
        let mut line = format!("{} files, {} together", group(self.items.len() + self.unchecked), crate::scan::human(self.total));
        let gone = self.count(State::Missing);
        if gone > 0 {
            line.push_str(&format!(". {} {} where the project says", group(gone), if gone == 1 { "isn't" } else { "aren't" }));
        }
        let cloud = self.count(State::CloudOnly);
        if cloud > 0 {
            line.push_str(&format!(". {} {} only in the cloud and would be downloaded when the project opens {}", group(cloud), if cloud == 1 { "is" } else { "are" }, if cloud == 1 { "it" } else { "them" }));
        }
        if self.unchecked > 0 {
            line.push_str(&format!(". The last {} weren't checked", group(self.unchecked)));
        }
        line.push('.');
        out.push(Block::Para(plain(&line)));
        let rows = self
            .items
            .iter()
            .take(LIST_MAX)
            .map(|r| {
                // Found files by name, a link that shows them; missing ones as the project names
                // them, so it's clear where they were expected.
                let file = if r.state == State::Missing {
                    vec![Span { text: r.named.clone(), ..Default::default() }]
                } else {
                    let name = r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.named.clone());
                    vec![Span { text: name, link: Some(format!("{SHOW}{}", r.path.display())), ..Default::default() }]
                };
                let state = if r.state == State::Missing {
                    vec![Span { text: r.state.label().into(), italic: true, ..Default::default() }]
                } else {
                    plain(r.state.label())
                };
                let size = if r.state == State::Missing { plain("") } else { plain(&crate::scan::human(r.size)) };
                vec![file, state, size]
            })
            .collect();
        out.push(Block::Table { head: vec![plain("File"), plain("Where"), plain("Size")], rows });
        if self.items.len() > LIST_MAX {
            out.push(Block::Para(plain(&format!("And {} more.", group(self.items.len() - LIST_MAX)))));
        }
        out
    }
}

fn plain(t: &str) -> Vec<Span> {
    vec![Span { text: t.to_string(), ..Default::default() }]
}

fn group(n: usize) -> String {
    crate::inspect::group(n as u64)
}

/// Where a name in the project points: absolute as it is, relative from `base` (the project's
/// folder). Windows paths in a project saved on Windows are kept as written elsewhere.
pub fn resolve(named: &str, base: Option<&Path>) -> PathBuf {
    let n = named.trim();
    let n = n.strip_prefix("file://").unwrap_or(n);
    let looks_windows = n.len() > 2 && n.as_bytes()[1] == b':' && n.as_bytes()[0].is_ascii_alphabetic() || n.starts_with("\\\\");
    let p = if cfg!(windows) || looks_windows { PathBuf::from(n) } else { PathBuf::from(n.replace('\\', "/")) };
    if p.is_absolute() || looks_windows {
        return p;
    }
    match base {
        Some(b) => b.join(p),
        None => p,
    }
}

/// Checks every file the project names: `named` as written in the project, relative ones from
/// `base`. Repeats are listed once.
pub fn check(what: &'static str, named: impl IntoIterator<Item = String>, base: Option<&Path>) -> Refs {
    let mut seen = HashSet::new();
    let mut names: Vec<String> = named.into_iter().filter(|n| !n.trim().is_empty() && seen.insert(n.clone())).collect();
    let unchecked = names.len().saturating_sub(CHECK_MAX);
    names.truncate(CHECK_MAX);
    let home = base.and_then(device_of);
    let net = network_roots();
    let clouds = crate::cloud::roots();
    let mut physical = HashSet::new();
    let mut total = 0u64;
    let items = names
        .into_iter()
        .map(|named| {
            let path = resolve(&named, base);
            let Some(md) = std::fs::metadata(&path).ok() else {
                return Ref { named, path, state: State::Missing, size: 0 };
            };
            let size = md.len();
            let (_, flags) = crate::scan::on_disk(&md, || path.clone(), 4096);
            let state = if flags & store::ONLINE_ONLY != 0 {
                State::CloudOnly
            } else if flags & store::CLOUD != 0 || crate::cloud::root_of(&clouds, &path).is_some() {
                State::Cloud
            } else if net.iter().any(|r| path.starts_with(r)) {
                State::Network
            } else if home.is_some() && device_of(&path) != home {
                State::OtherDrive
            } else {
                State::Here
            };
            if physical.insert(identity(&path, &md)) {
                total += size;
            }
            Ref { named, path, state, size }
        })
        .collect();
    Refs { what, items, unchecked, total }
}

/// Which drive a path is on.
#[cfg(unix)]
fn device_of(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device_of(p: &Path) -> Option<u64> {
    // The drive letter (or the server of a UNC path).
    let c = p.components().next()?;
    let s = c.as_os_str().to_string_lossy().to_ascii_lowercase();
    Some(s.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211)))
}

/// The same file reached by two names (a link, `..`) is counted once.
#[cfg(unix)]
fn identity(_p: &Path, md: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (md.dev(), md.ino())
}

#[cfg(not(unix))]
fn identity(p: &Path, _md: &std::fs::Metadata) -> String {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase()
}

/// Roots of the network drives mounted here.
fn network_roots() -> Vec<PathBuf> {
    crate::drives::list().into_iter().filter(|d| d.network).map(|d| d.root).collect()
}

/// The path a "show" link points to, if `link` is one.
pub fn shown_path(link: &str) -> Option<PathBuf> {
    link.strip_prefix(SHOW).filter(|p| !p.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("filescape-refs-{}-{}", std::process::id(), rand_tag()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rand_tag() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    #[test]
    fn found_missing_and_counted_once() {
        let d = temp();
        std::fs::create_dir_all(d.join("Audio")).unwrap();
        std::fs::write(d.join("Audio/kick.wav"), vec![0u8; 1000]).unwrap();
        std::fs::write(d.join("snare.wav"), vec![0u8; 500]).unwrap();
        let abs = d.join("snare.wav").display().to_string();
        let r = check(
            "Sound files",
            ["Audio/kick.wav".to_string(), abs.clone(), "./snare.wav".into(), "gone.wav".into(), "Audio/kick.wav".into(), "".into()],
            Some(&d),
        );
        // The repeat and the empty name are dropped; snare.wav named two ways is one file.
        assert_eq!(r.items.len(), 4);
        assert_eq!(r.total, 1500);
        assert_eq!(r.count(State::Missing), 1);
        assert_eq!(r.count(State::Here), 3);
        assert_eq!(r.fact(), "4 (1 not found), 1.5 KB");
        let blocks = r.blocks();
        assert!(matches!(&blocks[0], Block::Heading { .. }));
        let Block::Table { rows, .. } = &blocks[2] else { panic!("no table") };
        assert_eq!(rows.len(), 4);
        let link = rows[0][0][0].link.as_deref().unwrap();
        assert_eq!(shown_path(link), Some(d.join("Audio/kick.wav")));
        assert!(rows[3][0][0].link.is_none(), "missing files have no link");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn windows_paths_stay_as_written() {
        let p = resolve("C:\\Media\\a.mov", Some(Path::new("/projects")));
        assert_eq!(p, PathBuf::from("C:\\Media\\a.mov"));
        assert_eq!(resolve("file:///media/a.mov", None), PathBuf::from("/media/a.mov"));
        #[cfg(unix)]
        assert_eq!(resolve("Audio Files\\take.wav", Some(Path::new("/p"))), PathBuf::from("/p/Audio Files/take.wav"));
        let r = check("Media", ["Z:\\nowhere\\clip.mov".to_string()], Some(Path::new("/")));
        assert_eq!(r.items[0].state, State::Missing);
    }

    #[test]
    fn nothing_named_makes_no_section() {
        assert!(check("Media", Vec::<String>::new(), None).blocks().is_empty());
    }
}
