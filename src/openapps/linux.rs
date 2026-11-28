//! Linux: the freedesktop.org way. A file's MIME type comes from the shared MIME database's
//! patterns (`mime/globs2`); the apps that open it are the desktop entries whose `MimeType=`
//! lists it (or a type it's a kind of), adjusted by `mimeapps.list` (Added, Removed and Default
//! Applications), as `xdg-mime` and the desktops read them.

use super::{Handler, Type};
use crate::apps::desktop;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The folders the system looks in for shared data: the user's own, then the system's.
fn data_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => out.push(PathBuf::from(d)),
        None => out.extend(std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))),
    }
    let sys = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    out.extend(sys.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    out
}

fn config_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(d) => out.push(PathBuf::from(d)),
        None => out.extend(std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))),
    }
    let sys = std::env::var("XDG_CONFIG_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    out.extend(sys.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    out
}

/// The shared MIME database: extension → type, and what each type is a kind of.
#[derive(Default)]
struct MimeDb {
    by_ext: HashMap<String, (u32, String)>,
    parents: HashMap<String, Vec<String>>,
    aliases: HashMap<String, String>,
}

impl MimeDb {
    fn load(dirs: &[PathBuf]) -> MimeDb {
        let mut db = MimeDb::default();
        // Later folders are less important: read them first so earlier ones win.
        for d in dirs.iter().rev() {
            let m = d.join("mime");
            if let Ok(t) = std::fs::read_to_string(m.join("globs2")) {
                for line in t.lines().filter(|l| !l.starts_with('#')) {
                    let mut f = line.splitn(4, ':');
                    let (Some(w), Some(mime), Some(glob)) = (f.next(), f.next(), f.next()) else { continue };
                    let Ok(w) = w.parse::<u32>() else { continue };
                    // Simple "*.ext" patterns only: the ones a file's extension can match.
                    let Some(ext) = glob.strip_prefix("*.").filter(|e| !e.contains(['*', '?', '[', '.'])) else { continue };
                    let ext = ext.to_lowercase();
                    if db.by_ext.get(&ext).is_none_or(|(old, _)| w >= *old) {
                        db.by_ext.insert(ext, (w, mime.to_string()));
                    }
                }
            }
            if let Ok(t) = std::fs::read_to_string(m.join("subclasses")) {
                for (a, b) in t.lines().filter_map(|l| l.split_once(' ')) {
                    db.parents.entry(a.to_string()).or_default().push(b.to_string());
                }
            }
            if let Ok(t) = std::fs::read_to_string(m.join("aliases")) {
                for (a, b) in t.lines().filter_map(|l| l.split_once(' ')) {
                    db.aliases.insert(a.to_string(), b.to_string());
                }
            }
        }
        db
    }

    /// A type and everything it's a kind of, most specific first: text/x-python, then
    /// text/x-script, then text/plain (every text/* type is plain text underneath).
    fn chain(&self, mime: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut todo = vec![self.aliases.get(mime).cloned().unwrap_or_else(|| mime.to_string())];
        while let Some(m) = todo.first().cloned() {
            todo.remove(0);
            if out.contains(&m) || out.len() > 16 {
                continue;
            }
            todo.extend(self.parents.get(&m).cloned().unwrap_or_default());
            out.push(m);
        }
        if out.iter().any(|m| m.starts_with("text/")) && !out.iter().any(|m| m == "text/plain") {
            out.push("text/plain".into());
        }
        out
    }
}

/// An installed launcher: its desktop id ("org.gnome.TextEditor.desktop") and file.
struct App {
    file: PathBuf,
    name: String,
    mimes: Vec<String>,
    /// Shown under All applications: it runs, isn't hidden, and takes files.
    listed: bool,
}

#[derive(Default)]
struct Index {
    db: MimeDb,
    apps: HashMap<String, App>,
    /// From `mimeapps.list`, most important first.
    defaults: HashMap<String, Vec<String>>,
    added: HashMap<String, Vec<String>>,
    removed: HashMap<String, Vec<String>>,
}

fn index() -> &'static Mutex<Option<std::sync::Arc<Index>>> {
    static I: OnceLock<Mutex<Option<std::sync::Arc<Index>>>> = OnceLock::new();
    I.get_or_init(Default::default)
}

pub fn forget() {
    *index().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn current() -> std::sync::Arc<Index> {
    let mut g = index().lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| std::sync::Arc::new(build(&data_dirs(), &config_dirs()))).clone()
}

/// Every launcher under each `applications` folder (subfolders give ids like "kde-kate.desktop").
fn scan_apps(dirs: &[PathBuf]) -> HashMap<String, App> {
    let mut apps = HashMap::new();
    fn walk(root: &Path, dir: &Path, depth: u32, apps: &mut HashMap<String, App>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 3 {
                    walk(root, &p, depth + 1, apps);
                }
                continue;
            }
            if p.extension().is_none_or(|x| x != "desktop") {
                continue;
            }
            let id = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('/', "-");
            // The first folder that has an id wins (the user's own over the system's).
            if apps.contains_key(&id) {
                continue;
            }
            let Some(entry) = std::fs::metadata(&p).ok().filter(|m| m.len() <= 1 << 20).and_then(|_| std::fs::read_to_string(&p).ok()).and_then(|t| desktop::entry(&t)) else {
                continue;
            };
            let hidden_file = std::fs::read_to_string(&p).is_ok_and(|t| t.lines().any(|l| l.trim() == "Hidden=true"));
            let runs = entry.kind == "Application"
                && entry.exec.as_deref().and_then(|x| desktop::command(x, &entry, &p)).is_some_and(|c| desktop::which(&c[0]).is_some())
                && entry.try_exec.as_deref().is_none_or(|t| desktop::which(t).is_some());
            // A Hidden entry is one the user deleted: it hides the system's copy too.
            let takes_files = entry.exec.as_deref().is_some_and(|x| ["%f", "%F", "%u", "%U"].iter().any(|c| x.contains(c)));
            let listed = runs && !hidden_file && !entry.hidden && takes_files;
            let app = App { file: p.clone(), name: entry.name.clone().unwrap_or_default(), mimes: if runs && !hidden_file { entry.mime_types } else { Vec::new() }, listed };
            apps.insert(id, app);
        }
    }
    for d in dirs {
        let root = d.join("applications");
        walk(&root, &root, 0, &mut apps);
    }
    apps
}

/// Read `mimeapps.list` files, most important first: (defaults, added, removed).
type Lists = (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>, HashMap<String, Vec<String>>);

fn read_lists(files: &[PathBuf]) -> Lists {
    let (mut defaults, mut added, mut removed): Lists = Default::default();
    for f in files {
        let Ok(t) = std::fs::read_to_string(f) else { continue };
        let mut group = "";
        for line in t.lines().map(str::trim) {
            if line.starts_with('[') {
                group = line;
                continue;
            }
            let Some((mime, ids)) = line.split_once('=') else { continue };
            let ids: Vec<String> = ids.split(';').map(str::trim).filter(|i| !i.is_empty()).map(String::from).collect();
            let into = match group {
                "[Default Applications]" => &mut defaults,
                "[Added Associations]" => &mut added,
                "[Removed Associations]" => &mut removed,
                _ => continue,
            };
            into.entry(mime.trim().to_string()).or_default().extend(ids);
        }
    }
    (defaults, added, removed)
}

fn build(data: &[PathBuf], config: &[PathBuf]) -> Index {
    let mut lists: Vec<PathBuf> = config.iter().map(|d| d.join("mimeapps.list")).collect();
    lists.extend(data.iter().map(|d| d.join("applications/mimeapps.list")));
    lists.extend(data.iter().map(|d| d.join("applications/defaults.list")));
    let (defaults, added, removed) = read_lists(&lists);
    Index { db: MimeDb::load(data), apps: scan_apps(data), defaults, added, removed }
}

/// A type's MIME type.
fn mime_of(ix: &Index, t: &Type) -> Option<String> {
    match t {
        Type::Ext(e) => ix.db.by_ext.get(e).map(|(_, m)| m.clone()),
        Type::Text => Some("text/plain".into()),
        Type::Folder => Some("inode/directory".into()),
        Type::Unknown => None,
    }
}

fn handlers_in(ix: &Index, t: &Type) -> Vec<Handler> {
    let Some(mime) = mime_of(ix, t) else { return Vec::new() };
    let chain = ix.db.chain(&mime);
    let removed: Vec<&String> = chain.iter().flat_map(|m| ix.removed.get(m).into_iter().flatten()).collect();
    let installed = |id: &String| ix.apps.get(id).filter(|a| !a.name.is_empty() && !removed.contains(&id));
    let default = chain.iter().flat_map(|m| ix.defaults.get(m).into_iter().flatten()).find(|id| installed(id).is_some()).cloned();
    let mut ids: Vec<&String> = chain.iter().flat_map(|m| ix.added.get(m).into_iter().flatten()).collect();
    for m in &chain {
        let mut more: Vec<&String> = ix.apps.iter().filter(|(_, a)| a.mimes.iter().any(|x| x == m)).map(|(id, _)| id).collect();
        more.sort();
        ids.extend(more);
    }
    ids.into_iter()
        .filter_map(|id| installed(id).map(|a| (id, a)))
        .map(|(id, a)| Handler { id: id.clone(), name: a.name.clone(), app: a.file.clone(), default: default.as_ref() == Some(id) })
        .collect()
}

pub fn handlers(t: &Type) -> Vec<Handler> {
    handlers_in(&current(), t)
}

pub fn open(h: &Handler, files: &[PathBuf]) -> Result<(), String> {
    crate::apps::linux::open_with_entry(&h.app, files)
}

/// Every app that takes files, whatever types it says it opens.
pub fn all_apps() -> Vec<Handler> {
    let ix = current();
    ix.apps
        .iter()
        .filter(|(_, a)| a.listed && !a.name.is_empty())
        .map(|(id, a)| Handler { id: id.clone(), name: a.name.clone(), app: a.file.clone(), default: false })
        .collect()
}

/// Make it the default for the type's MIME type, as the desktop's own settings do: through
/// `xdg-mime` where it's installed, else by writing the user's `mimeapps.list` directly.
pub fn set_default(h: &Handler, t: &Type) -> Result<(), String> {
    let mime = mime_of(&current(), t).ok_or("The system doesn't know what kind of file this is.")?;
    if desktop::which("xdg-mime").is_some() {
        let out = std::process::Command::new("xdg-mime")
            .args(["default", &h.id, &mime])
            .output()
            .map_err(|e| format!("Couldn't run xdg-mime, which changes the desktop's default apps: {e}"))?;
        return if out.status.success() { Ok(()) } else { Err(format!("xdg-mime couldn't change it: {}", String::from_utf8_lossy(&out.stderr).trim())) };
    }
    let file = config_dirs().into_iter().next().ok_or("There's no settings folder to write to.")?.join("mimeapps.list");
    let old = std::fs::read_to_string(&file).unwrap_or_default();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    }
    std::fs::write(&file, with_default(&old, &mime, &h.id)).map_err(|e| format!("Couldn't write {}: {e}", file.display()))
}

/// A `mimeapps.list` with `mime` opening in `id` by default, everything else kept as it was.
fn with_default(text: &str, mime: &str, id: &str) -> String {
    let mut out = Vec::new();
    let mut in_defaults = false;
    let mut done = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_defaults && !done {
                out.push(format!("{mime}={id};"));
                done = true;
            }
            in_defaults = t == "[Default Applications]";
        } else if in_defaults && t.split_once('=').is_some_and(|(m, _)| m.trim() == mime) {
            if !done {
                out.push(format!("{mime}={id};"));
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if !in_defaults {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push("[Default Applications]".into());
        }
        out.push(format!("{mime}={id};"));
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_default() {
        assert_eq!(with_default("", "text/plain", "kate.desktop"), "[Default Applications]\ntext/plain=kate.desktop;\n");
        let had = "[Added Associations]\nimage/png=gimp.desktop;\n[Default Applications]\ntext/plain=vim.desktop;\nimage/png=eog.desktop;\n";
        assert_eq!(
            with_default(had, "text/plain", "kate.desktop"),
            "[Added Associations]\nimage/png=gimp.desktop;\n[Default Applications]\ntext/plain=kate.desktop;\nimage/png=eog.desktop;\n"
        );
        let other = "[Default Applications]\nimage/png=eog.desktop;\n[Added Associations]\nimage/png=gimp.desktop;\n";
        assert_eq!(
            with_default(other, "text/plain", "kate.desktop"),
            "[Default Applications]\nimage/png=eog.desktop;\ntext/plain=kate.desktop;\n[Added Associations]\nimage/png=gimp.desktop;\n"
        );
        assert_eq!(with_default("[Added Associations]\nx=y;\n", "a/b", "c.desktop"), "[Added Associations]\nx=y;\n\n[Default Applications]\na/b=c.desktop;\n");
    }

    #[test]
    fn finds_apps_for_types() {
        let root = std::env::temp_dir().join(format!("fs-openapps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (data, sys, config) = (root.join("home"), root.join("usr"), root.join("config"));
        std::fs::create_dir_all(sys.join("mime")).unwrap();
        std::fs::write(sys.join("mime/globs2"), "# globs\n50:text/x-python:*.py\n50:text/markdown:*.md\n50:image/png:*.png\n10:text/x-readme:README*\n").unwrap();
        std::fs::write(sys.join("mime/subclasses"), "text/x-python text/x-script\n").unwrap();
        std::fs::create_dir_all(sys.join("applications/kde")).unwrap();
        std::fs::create_dir_all(data.join("applications")).unwrap();
        let entry = |name: &str, exec: &str, mimes: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nMimeType={mimes}\n");
        std::fs::write(sys.join("applications/gedit.desktop"), entry("Text Editor", "sh %U", "text/plain;")).unwrap();
        std::fs::write(sys.join("applications/kde/kate.desktop"), entry("Kate", "sh %U", "text/plain;text/markdown;")).unwrap();
        std::fs::write(sys.join("applications/idle.desktop"), entry("IDLE", "sh %f", "text/x-python;")).unwrap();
        std::fs::write(sys.join("applications/gone.desktop"), entry("Gone", "no-such-editor-here %U", "text/plain;")).unwrap();
        std::fs::write(sys.join("applications/viewer.desktop"), entry("Viewer", "sh %f", "image/png;")).unwrap();
        // The user hid the system's viewer.
        std::fs::write(data.join("applications/viewer.desktop"), "[Desktop Entry]\nType=Application\nName=Viewer\nExec=sh\nHidden=true\n").unwrap();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("mimeapps.list"), "[Default Applications]\ntext/plain=missing.desktop;kde-kate.desktop;\n\n[Removed Associations]\ntext/markdown=kde-kate.desktop;\n").unwrap();
        let ix = build(&[data.clone(), sys.clone()], &[config.clone()]);
        let names = |t: Type| handlers_in(&ix, &t).iter().map(|h| (h.name.clone(), h.default)).collect::<Vec<_>>();
        // Python is a kind of script, and of plain text: its own editor, then the text editors.
        let py = names(Type::Ext("py".into()));
        assert_eq!(py[0], ("IDLE".into(), false));
        assert!(py.contains(&("Kate".into(), true)) && py.contains(&("Text Editor".into(), false)));
        assert!(!py.iter().any(|(n, _)| n == "Gone"), "not installed: {py:?}");
        assert!(names(Type::Text).contains(&("Kate".into(), true)));
        // Kate was removed for Markdown, which is still text underneath.
        assert!(!names(Type::Ext("md".into())).iter().any(|(n, _)| n == "Kate"));
        assert!(names(Type::Ext("md".into())).iter().any(|(n, _)| n == "Text Editor"));
        assert!(names(Type::Ext("png".into())).is_empty());
        assert!(names(Type::Ext("zzz".into())).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
