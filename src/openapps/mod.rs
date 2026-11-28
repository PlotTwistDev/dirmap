//! Which installed apps can open a file, asked of the system itself: Launch Services on a Mac,
//! the registered handlers on Windows, desktop entries and `mimeapps.list` on Linux. Nothing is
//! hardcoded per app. Answers are kept per file type, worked out on a worker thread the first
//! time a type is asked about, so menus stay quick.
//!
//! Opening goes through each system's own launcher with the files as separate arguments (never
//! a shell), so names with spaces, quotes or Unicode are passed whole.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub mod prefs;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

/// An app that can open a type of file.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Handler {
    /// What the system knows it by: an app bundle's or program's path, a desktop entry's id.
    pub id: String,
    /// Its name, as the system shows it ("Visual Studio Code").
    pub name: String,
    /// Where it is (an `.app`, an `.exe`, a `.desktop` file), for its icon. May not exist for
    /// apps the system keeps elsewhere (Windows Store apps).
    pub app: PathBuf,
    /// The system opens this type with it by default.
    pub default: bool,
}

/// What a file is, for choosing apps.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    /// By its extension (lower case, no dot).
    Ext(String),
    /// Plain text with no telling extension (LICENSE, README, Makefile, or text by its content).
    Text,
    Folder,
    /// Nothing known: no apps are suggested.
    Unknown,
}

impl Type {
    /// How it's written for people: ".txt files", "text files", "folders".
    pub fn label(&self) -> String {
        match self {
            Type::Ext(e) => format!(".{e} files"),
            Type::Text => "text files".into(),
            Type::Folder => "folders".into(),
            Type::Unknown => "this file".into(),
        }
    }

    /// The kind of file it is (Documents, Images…), for a default that covers the whole kind.
    /// None for folders, unknown files and kinds too mixed to share an app (Other, Programs…).
    pub fn category(&self) -> Option<crate::categories::Category> {
        use crate::categories::Category as C;
        let c = match self {
            Type::Ext(e) => crate::categories::of(&format!("x.{e}")),
            Type::Text => C::Documents,
            Type::Folder | Type::Unknown => return None,
        };
        matches!(c, C::Audio | C::Video | C::Images | C::Documents | C::Code | C::Archives | C::Models).then_some(c)
    }

    /// How it's kept in settings: "txt", or "(text)" and "(folder)", which no extension is.
    pub fn key(&self) -> Option<String> {
        match self {
            Type::Ext(e) => Some(e.clone()),
            Type::Text => Some("(text)".into()),
            Type::Folder => Some("(folder)".into()),
            Type::Unknown => None,
        }
    }
}

/// Bytes read from a file with no extension to tell what it is.
const SNIFF: usize = 8 << 10;

/// What a file is: by its extension, then a well-known name without one, then (for a file on
/// this computer) its first bytes: a known signature, or text when it's valid UTF-8 with no NUL
/// bytes. Anything else is Unknown, rather than guessed to be text.
pub fn identify(path: &Path, is_dir: bool) -> Type {
    if is_dir {
        return Type::Folder;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = crate::scan::ext_of(&name).to_ascii_lowercase();
    if !ext.is_empty() {
        return Type::Ext(ext);
    }
    if crate::formats::by_name(&name).is_some_and(|f| f.id == "text") {
        return Type::Text;
    }
    // Contents only for files here: reading one on a server or inside an archive is slow.
    if crate::fsys::remote::is_remote(path) || crate::fsys::archive::inside(path) {
        return Type::Unknown;
    }
    let mut head = vec![0u8; SNIFF];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    identify_head(&head[..n])
}

fn identify_head(head: &[u8]) -> Type {
    if head.is_empty() {
        return Type::Unknown;
    }
    if let Some(f) = crate::formats::sniff(head)
        && let Some(e) = f.exts.first()
    {
        return Type::Ext(e.to_string());
    }
    if looks_like_text(head) { Type::Text } else { Type::Unknown }
}

/// Valid UTF-8 (allowing a character cut off at the end), no NUL bytes, and few control
/// characters other than tabs and line breaks.
fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    let ok = match std::str::from_utf8(head) {
        Ok(s) => s,
        Err(e) if e.error_len().is_none() && head.len() - e.valid_up_to() < 4 => std::str::from_utf8(&head[..e.valid_up_to()]).unwrap_or(""),
        Err(_) => return false,
    };
    let controls = ok.chars().filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x0c')).count();
    !ok.is_empty() && controls * 100 <= ok.chars().count()
}

/// What's known about a type: the apps, or still being looked up.
#[derive(Clone)]
pub enum Apps {
    Looking,
    Found(Arc<Vec<Handler>>),
}

type Cache = Mutex<HashMap<Type, Option<Arc<Vec<Handler>>>>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The apps for a type, the default first. The first time a type is asked about the answer is
/// worked out on a worker thread, and `done` is called when it's ready (to redraw).
pub fn apps_for(t: &Type, done: impl FnOnce() + Send + 'static) -> Apps {
    if *t == Type::Unknown {
        return Apps::Found(Arc::default());
    }
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    match c.get(t) {
        Some(Some(found)) => return Apps::Found(found.clone()),
        Some(None) => return Apps::Looking,
        None => {}
    }
    c.insert(t.clone(), None);
    drop(c);
    let t = t.clone();
    let _ = std::thread::Builder::new().name("open-with".into()).spawn(move || {
        let found = Arc::new(find(&t));
        cache().lock().unwrap_or_else(|e| e.into_inner()).insert(t, Some(found));
        done();
    });
    Apps::Looking
}

/// Every app that can be asked to open a file of this type, not only those that say they can
/// (for Choose Another Application): every installed app on a Mac and Linux, and on Windows
/// every program registered for the extension. Worked out on a worker thread, as `apps_for`.
pub fn all_apps(t: &Type, done: impl FnOnce() + Send + 'static) -> Apps {
    static ALL: OnceLock<Cache> = OnceLock::new();
    let all = ALL.get_or_init(Default::default);
    // The list is the same for every type except on Windows.
    let key = if cfg!(windows) { t.clone() } else { Type::Unknown };
    let mut c = all.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = c.get(&key) {
        return found.clone().map_or(Apps::Looking, Apps::Found);
    }
    c.insert(key.clone(), None);
    drop(c);
    let t = t.clone();
    let _ = std::thread::Builder::new().name("open-with-all".into()).spawn(move || {
        #[cfg(target_os = "linux")]
        let mut found = linux::all_apps();
        #[cfg(target_os = "macos")]
        let mut found = mac::all_apps();
        #[cfg(windows)]
        let mut found = windows::all_apps(&t);
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        let mut found: Vec<Handler> = Vec::new();
        let _ = &t;
        for h in &mut found {
            h.default = false;
        }
        tidy(&mut found);
        all.lock().unwrap_or_else(|e| e.into_inner()).insert(key, Some(Arc::new(found)));
        done();
    });
    Apps::Looking
}

/// Make an app the system's default for a type: for every app on this computer, not just
/// Filescape. Only ever done when the person asks for it and confirms. On Windows the system
/// keeps that choice to itself, so its own Open with box is shown for `sample` instead.
pub fn set_system_default(h: &Handler, t: &Type, sample: &Path) -> Result<(), String> {
    let _ = (h, t, sample);
    #[cfg(target_os = "linux")]
    let r = linux::set_default(h, t);
    #[cfg(target_os = "macos")]
    let r = mac::set_default(h, t);
    #[cfg(windows)]
    let r = windows::set_default(sample);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let r = Err("Changing the system's default app isn't supported here.".to_string());
    forget();
    r
}

/// The system lets Filescape set its default itself (elsewhere, its own box is shown).
pub const SETS_SYSTEM_DEFAULT: bool = cfg!(any(target_os = "linux", target_os = "macos"));

/// Forget what was found, so apps installed or removed since show up (asked again next time).
pub fn forget() {
    cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
    #[cfg(target_os = "linux")]
    linux::forget();
}

/// Ask the system, then tidy: one entry per app, the default first, then by name.
fn find(t: &Type) -> Vec<Handler> {
    if *t == Type::Unknown {
        return Vec::new();
    }
    #[cfg(target_os = "linux")]
    let mut found = linux::handlers(t);
    #[cfg(target_os = "macos")]
    let mut found = mac::handlers(t);
    #[cfg(windows)]
    let mut found = windows::handlers(t);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let mut found: Vec<Handler> = Vec::new();
    tidy(&mut found);
    found
}

fn tidy(found: &mut Vec<Handler>) {
    let mut seen = std::collections::HashSet::new();
    found.retain(|h| !h.name.trim().is_empty() && seen.insert(h.id.clone()));
    // Two copies of one app (an old version left behind) keep their places but say where.
    found.sort_by(|a, b| b.default.cmp(&a.default).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
}

/// Open files with an app, all in one go where the app takes several. Files on a server or
/// inside an archive must be copied out first (the caller does that, as Open does).
pub fn open(h: &Handler, files: &[PathBuf]) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    return linux::open(h, files);
    #[cfg(target_os = "macos")]
    return mac::open(h, files);
    #[cfg(windows)]
    return windows::open(h, files);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (h, files);
        Err("Opening with another app isn't supported here.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_files() {
        assert_eq!(identify(Path::new("/x/notes.TXT"), false), Type::Ext("txt".into()));
        assert_eq!(identify(Path::new("/x/src"), true), Type::Folder);
        assert_eq!(identify(Path::new("/x/LICENSE"), false), Type::Text);
        assert_eq!(identify(Path::new("/x/Makefile"), false), Type::Text);
        assert_eq!(identify_head(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Type::Ext("png".into()));
        assert_eq!(identify_head("Copyright (c) 2026\n\tSome caf\u{e9} text".as_bytes()), Type::Text);
        // A character cut off at the end of what was read is still text.
        assert_eq!(identify_head(&"caf\u{e9}".as_bytes()[..4]), Type::Text);
        assert_eq!(identify_head(b"\x7fELF\x02\x01\x01\0\0\0"), Type::Unknown);
        assert_eq!(identify_head(&[0xff, 0xfe, 0x41, 0x42]), Type::Unknown);
        assert_eq!(identify_head(b""), Type::Unknown);
        assert_eq!(identify_head(b"\x01\x02\x03\x04\x05abc"), Type::Unknown);
        let dir = std::env::temp_dir().join(format!("fs-identify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("NOTES"), "Things to do\n- one\n").unwrap();
        std::fs::write(dir.join("blob"), [0u8, 1, 2, 3, 200, 201]).unwrap();
        assert_eq!(identify(&dir.join("NOTES"), false), Type::Text);
        assert_eq!(identify(&dir.join("blob"), false), Type::Unknown);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Type::Ext("md".into()).label(), ".md files");
        assert_eq!(Type::Text.key().as_deref(), Some("(text)"));
        assert_eq!(Type::Unknown.key(), None);
    }

    #[test]
    fn tidies_the_list() {
        let h = |id: &str, name: &str, default: bool| Handler { id: id.into(), name: name.into(), app: PathBuf::new(), default };
        let mut v = vec![h("b", "zed", false), h("a", "Atom", false), h("c", "Kate", true), h("a", "Atom again", false), h("d", " ", false)];
        tidy(&mut v);
        assert_eq!(v.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["Kate", "Atom", "zed"]);
    }
}
