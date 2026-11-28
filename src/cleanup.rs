//! Space AI models take that nothing needs, for the Storage view of the AI Models workspace:
//! Ollama downloads no model uses, half-finished downloads, older Hugging Face versions and loose
//! copies of a model kept elsewhere. Also what removing one model properly means: for Ollama, its
//! own list entries and the files no other model shares.
//!
//! Finding only reads. Removing goes through [`remove`], which only touches the paths an item
//! lists, inside the folder it names, after one confirmation in the window.
//!
//! When an app's own records can't be read (Ollama's manifests, Hugging Face's refs), nothing in
//! that store is offered: what's in use can't be told, so nothing is called unused.

use crate::models::{Format, Model, Source};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Downloads no model uses: Ollama blobs no manifest names, Hugging Face blobs no version uses.
    Unused,
    /// Downloads that stopped part way.
    Partial,
    /// Older versions of a Hugging Face model, kept beside the current one.
    OldVersion,
    /// A loose model file that's byte-for-byte the same as one an app (or another folder) keeps.
    Copy,
    /// One model, chosen to be removed.
    Model,
}

impl Kind {
    pub fn heading(self) -> &'static str {
        match self {
            Kind::Unused => "Downloads no model uses",
            Kind::Partial => "Unfinished downloads",
            Kind::OldVersion => "Older Hugging Face versions",
            Kind::Copy => "Extra copies",
            Kind::Model => "Model",
        }
    }
}

/// Something that can be removed to free space.
#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Kind,
    pub title: String,
    /// Why it can go, in a sentence.
    pub detail: String,
    /// The app whose store it's in, or "Folder".
    pub app: &'static str,
    /// What removing it removes: files, links and folders.
    pub paths: Vec<PathBuf>,
    /// Bytes freed once removed (and, if moved to the Trash, the Trash emptied).
    pub bytes: u64,
    /// Every path is inside this folder; [`remove`] refuses anything else.
    pub within: PathBuf,
    /// For copies: the file each path is the same as, which is kept.
    pub keep: Vec<PathBuf>,
    /// Can be removed (a copy an app keeps can't: the app would lose it).
    pub can_trash: bool,
    /// Can be shared with what it's a copy of: one file on disk, under both names (hard links).
    pub can_share: bool,
    /// How sure Filescape is that nothing needs it.
    pub sure: Sure,
    /// The evidence, one plain sentence each, for the list and the confirmation.
    pub reasons: Vec<String>,
    /// For copies inside an app, an extension or another program's own folder: that program.
    /// Such a copy is listed but never removed or shared, since the program loads it from there.
    pub used_by: Option<String>,
}

/// How sure Filescape is that something can go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sure {
    /// The app's own records say nothing uses it.
    Certain,
    /// Nothing Filescape can see uses it, but something it can't see might (a program asking for
    /// an exact older version, a download that may still resume).
    Likely,
}

impl Sure {
    pub fn label(self) -> &'static str {
        match self {
            Sure::Certain => "Safe to remove",
            Sure::Likely => "Probably unused",
        }
    }
}

/// A download written to in the last few minutes may still be going: it's left alone.
const BUSY: Duration = Duration::from_secs(10 * 60);

/// Endings browsers and download tools give files they're still writing.
const PARTIAL_ENDINGS: [&str; 6] = [".part", ".partial", ".crdownload", ".download", ".incomplete", ".aria2"];

/// Everything that can be freed, in the places the library looked and the stores its models are
/// in, largest first within each kind.
pub fn find(places: &[PathBuf], models: &[Model]) -> Vec<Item> {
    let mut out = Vec::new();
    let mut ollama: Vec<PathBuf> = Vec::new();
    let mut hf: Vec<PathBuf> = Vec::new();
    let mut plain: Vec<PathBuf> = Vec::new();
    let add = |v: &mut Vec<PathBuf>, p: PathBuf| {
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        if !v.contains(&p) {
            v.push(p);
        }
    };
    for p in places {
        if is_ollama_store(p) {
            add(&mut ollama, p.clone());
        } else if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("models--")) {
            add(&mut hf, p.clone());
        } else if let Ok(rd) = std::fs::read_dir(p) {
            let repos: Vec<PathBuf> = rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("models--") && e.path().is_dir()).map(|e| e.path()).collect();
            if repos.is_empty() {
                add(&mut plain, p.clone());
            }
            for r in repos {
                add(&mut hf, r);
            }
        }
    }
    for m in models {
        let Some(f) = m.files.first() else { continue };
        match m.source {
            Source::Ollama => {
                if let Some(store) = f.path.parent().and_then(Path::parent).filter(|s| is_ollama_store(s)) {
                    add(&mut ollama, store.to_path_buf());
                }
            }
            Source::HuggingFace => {
                if let Some(repo) = f.path.ancestors().find(|a| a.file_name().is_some_and(|n| n.to_string_lossy().starts_with("models--"))) {
                    add(&mut hf, repo.to_path_buf());
                }
            }
            _ => {}
        }
    }
    for s in &ollama {
        ollama_items(s, &mut out);
    }
    for r in &hf {
        hf_items(r, &mut out);
    }
    for p in &plain {
        partial_files(p, p, 4, &mut out);
    }
    copies(models, &mut out);
    keep_linked(&mut out, places, models);
    // A loose partial can be inside a store too (looked at twice): keep the first.
    let mut seen = HashSet::new();
    out.retain(|i| i.paths.first().is_none_or(|p| seen.insert(p.clone())));
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then(b.bytes.cmp(&a.bytes)).then(a.title.cmp(&b.title)));
    out
}

/// Every path a link passes through on the way to its file: the link's own target, that
/// target's target and so on, then the file itself. Removing any of them breaks the link.
fn link_chain(link: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut at = link.to_path_buf();
    for _ in 0..16 {
        let Ok(t) = std::fs::read_link(&at) else { break };
        let t = if t.is_absolute() { t } else { at.parent().map(|d| d.join(&t)).unwrap_or(t) };
        out.push(t.clone());
        at = t;
    }
    if let Ok(c) = std::fs::canonicalize(link) {
        out.push(c);
    }
    out
}

/// Every link to a file among the models and in the places, as (a path the link passes through,
/// who the link belongs to, the link itself).
fn links(places: &[PathBuf], models: &[Model]) -> Vec<(PathBuf, String, PathBuf)> {
    // (a path a link passes through, who the link belongs to, the link itself)
    let mut through: Vec<(PathBuf, String, PathBuf)> = Vec::new();
    let mut add = |link: &Path, who: String| {
        for hop in link_chain(link) {
            through.push((hop, who.clone(), link.to_path_buf()));
        }
    };
    let mut seen = HashSet::new();
    for m in models {
        for f in &m.files {
            if std::fs::symlink_metadata(&f.path).is_ok_and(|md| md.file_type().is_symlink()) && seen.insert(f.path.clone()) {
                add(&f.path, format!("{} ({})", m.name, m.source.label()));
            }
        }
    }
    // Links the library doesn't list as models (it doesn't follow links), in the places it looked.
    for place in places {
        let mut links = Vec::new();
        file_links(place, 8, &mut links);
        for l in links {
            if seen.insert(l.clone()) {
                add(&l, l.display().to_string());
            }
        }
    }
    through
}

/// Who, outside the item's own folder, links to `p` (or into it, for a folder).
fn users_of(it: &Item, p: &Path, through: &[(PathBuf, String, PathBuf)]) -> Vec<String> {
    let canon_within = std::fs::canonicalize(&it.within).unwrap_or(it.within.clone());
    let canon = std::fs::canonicalize(p).unwrap_or(p.to_path_buf());
    let mut who: Vec<String> = Vec::new();
    for (hop, w, link) in through {
        // A link inside the same store (a Hugging Face version's own links) isn't a user from elsewhere.
        if link.starts_with(&it.within) || link.starts_with(&canon_within) || it.paths.contains(link) {
            continue;
        }
        if (hop.starts_with(p) || hop.starts_with(&canon)) && !who.contains(w) {
            who.push(w.clone());
        }
    }
    who
}

/// Who links to anything removing `it` would remove, from the models and places given.
pub fn linked_from(it: &Item, places: &[PathBuf], models: &[Model]) -> Vec<String> {
    let through = links(places, models);
    let mut who: Vec<String> = Vec::new();
    for p in &it.paths {
        for w in users_of(it, p, &through) {
            if !who.contains(&w) {
                who.push(w);
            }
        }
    }
    who
}

/// Takes out of each item what a model elsewhere links to, such as an Ollama download a tool
/// linked into LM Studio's folder: removing it would break that model. An item left with
/// nothing is dropped; the others say what was kept and why.
fn keep_linked(out: &mut Vec<Item>, places: &[PathBuf], models: &[Model]) {
    let through = links(places, models);
    for it in out.iter_mut() {
        if !matches!(it.kind, Kind::Unused | Kind::Partial | Kind::OldVersion) {
            continue;
        }
        let mut kept: Vec<String> = Vec::new();
        let mut freed_less = 0u64;
        let snapshot = it.clone();
        it.paths.retain(|p| {
            let users = users_of(&snapshot, p, &through);
            if users.is_empty() {
                return true;
            }
            for who in users {
                if !kept.contains(&who) {
                    kept.push(who);
                }
            }
            freed_less += own_bytes(p);
            false
        });
        if kept.is_empty() {
            if matches!(it.kind, Kind::Unused | Kind::OldVersion) {
                it.reasons.push("No other model Filescape found links to it".into());
            }
            continue;
        }
        it.bytes = it.bytes.saturating_sub(freed_less);
        it.reasons.push(format!("Part of it is kept, because of a link to it from {}", kept.join(", ")));
    }
    out.retain(|it| !it.paths.is_empty() && it.bytes > 0);
}

/// Links to files (not folders) under `dir`, without following links to folders.
fn file_links(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            if std::fs::metadata(e.path()).is_ok_and(|m| m.is_file()) {
                out.push(e.path());
            }
        } else if t.is_dir() && depth > 0 {
            file_links(&e.path(), depth - 1, out);
        }
    }
}

/// The bytes removing a path frees: a file's size, or the files (not links) in a folder.
fn own_bytes(p: &Path) -> u64 {
    if std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) {
        let mut files = Vec::new();
        let _ = list_files(p, 8, &mut files);
        files.iter().filter(|f| std::fs::symlink_metadata(f).is_ok_and(|m| m.is_file())).map(|f| size(f)).sum()
    } else {
        size(p)
    }
}

fn is_ollama_store(p: &Path) -> bool {
    p.join("blobs").is_dir() && p.join("manifests").is_dir()
}

fn size(p: &Path) -> u64 {
    std::fs::symlink_metadata(p).map_or(0, |m| m.len())
}

/// How long ago a file was last written.
fn age(p: &Path) -> Option<Duration> {
    let m = std::fs::symlink_metadata(p).ok()?.modified().ok()?;
    SystemTime::now().duration_since(m).ok()
}

fn busy(p: &Path) -> bool {
    age(p).is_none_or(|a| a < BUSY)
}

/// "3 days ago", for when a download stopped.
fn ago(p: &Path) -> String {
    let Some(a) = age(p) else { return String::new() };
    let s = a.as_secs();
    let (n, unit) = if s >= 86_400 * 2 {
        (s / 86_400, "days")
    } else if s >= 7200 {
        (s / 3600, "hours")
    } else {
        (s / 60, "minutes")
    };
    format!("last written {n} {unit} ago")
}

/// How sure a stopped download can go, and why: one left a day or more is certainly stopped; a
/// newer one might be paused and resume from where it got to.
fn stopped(p: &Path, why: &str) -> (Sure, Vec<String>) {
    let mut reasons = vec![why.to_string(), format!("Nothing has written to it for a while: {}", ago(p))];
    if age(p).is_some_and(|a| a >= Duration::from_secs(86_400)) {
        (Sure::Certain, reasons)
    } else {
        reasons.push("Stopped less than a day ago: a paused download could still carry on from it".into());
        (Sure::Likely, reasons)
    }
}

fn list_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let t = e.file_type()?;
        if t.is_dir() && depth > 0 {
            list_files(&e.path(), depth - 1, out)?;
        } else if !t.is_dir() {
            out.push(e.path());
        }
    }
    Ok(())
}

// ---------- Ollama ----------

/// Every blob Ollama's manifests name, or `None` if any manifest can't be read (then nothing is
/// called unused).
fn ollama_referenced(store: &Path) -> Option<(Vec<(PathBuf, HashSet<String>)>, HashSet<String>)> {
    let mut manifests = Vec::new();
    list_files(&store.join("manifests"), 6, &mut manifests).ok()?;
    let mut all = HashSet::new();
    let mut each = Vec::new();
    for m in manifests {
        let text = std::fs::read_to_string(&m).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        let digests: HashSet<String> = json["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .chain([&json["config"]])
            .filter_map(|l| l["digest"].as_str())
            .map(|d| d.replace(':', "-"))
            .collect();
        all.extend(digests.iter().cloned());
        each.push((m, digests));
    }
    Some((each, all))
}

fn ollama_items(store: &Path, out: &mut Vec<Item>) {
    let Some((manifests, referenced)) = ollama_referenced(store) else { return };
    // With no models listed, Ollama would clean up itself; and an empty list may be one that
    // couldn't be seen. Either way, nothing is called unused.
    if manifests.is_empty() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(store.join("blobs")) else { return };
    let mut partial: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("sha256") || e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = e.path();
        if let Some(i) = name.find("-partial") {
            partial.entry(name[..i].to_string()).or_default().push(path);
        } else if !referenced.contains(&name) && !busy(&path) {
            let info = crate::models::gguf_info(&path);
            let title = info.as_ref().and_then(|i| i.name.clone()).unwrap_or_else(|| short_digest(&name));
            out.push(Item {
                used_by: None,
                kind: Kind::Unused,
                sure: Sure::Certain,
                reasons: vec![
                    format!("None of the {} model{} in Ollama's own list names it", manifests.len(), if manifests.len() == 1 { "" } else { "s" }),
                    format!("Not a download in progress: {}", ago(&path)),
                ],
                keep: Vec::new(),
                can_trash: true,
                can_share: false,
                title,
                detail: format!("An Ollama download no model uses any more, left by a removed model or a stopped download ({}).", ago(&path)),
                app: "Ollama",
                bytes: size(&path),
                paths: vec![path],
                within: store.to_path_buf(),
            });
        }
    }
    for (digest, mut paths) in partial {
        if paths.iter().any(|p| busy(p)) {
            continue;
        }
        paths.sort();
        let (sure, reasons) = stopped(&paths[0], "Its name ends in \u{201C}-partial\u{201D}: Ollama hadn't finished it");
        out.push(Item {
            used_by: None,
            kind: Kind::Partial,
            sure,
            reasons,
            keep: Vec::new(),
            can_trash: true,
            can_share: false,
            title: short_digest(&digest),
            detail: format!("Part of an Ollama download that didn't finish ({}). Pulling the model again starts it afresh.", ago(&paths[0])),
            app: "Ollama",
            bytes: paths.iter().map(|p| size(p)).sum(),
            paths,
            within: store.to_path_buf(),
        });
    }
}

/// "sha256-6f3b98f5…" for a blob with no name of its own.
fn short_digest(name: &str) -> String {
    let n: String = name.chars().take(15).collect();
    if name.chars().count() > 15 { format!("{n}…") } else { n }
}

/// What removing one model means: its files, and for Ollama the manifests that name it and the
/// blobs no other manifest names; for Hugging Face also the version links to its files.
pub fn plan(m: &Model) -> Result<Item, String> {
    let first = m.files.first().ok_or("This model has no files.")?;
    let title = m.name.clone();
    match m.source {
        Source::Ollama if !m.orphan => {
            let store = first.path.parent().and_then(Path::parent).filter(|s| is_ollama_store(s)).ok_or("This model isn't in an Ollama folder Filescape recognises.")?;
            let (manifests, _) = ollama_referenced(store).ok_or("Ollama's list of models can't be read, so Filescape can't tell which files only this model uses.")?;
            let mine: HashSet<String> = m.files.iter().filter_map(|f| f.path.file_name()).map(|n| n.to_string_lossy().to_string()).collect();
            let (gone, kept): (Vec<_>, Vec<_>) = manifests.into_iter().partition(|(_, d)| d.iter().any(|x| mine.contains(x)));
            if gone.is_empty() {
                return Err("No Ollama model names these files.".into());
            }
            let still: HashSet<&String> = kept.iter().flat_map(|(_, d)| d.iter()).collect();
            let mut blobs: Vec<String> = gone.iter().flat_map(|(_, d)| d.iter()).filter(|d| !still.contains(d)).cloned().collect();
            blobs.sort();
            blobs.dedup();
            let mut paths: Vec<PathBuf> = gone.iter().map(|(p, _)| p.clone()).collect();
            paths.extend(blobs.iter().map(|b| store.join("blobs").join(b)).filter(|p| p.exists()));
            let bytes = paths.iter().map(|p| size(p)).sum();
            let tags = gone.len();
            Ok(Item {
                used_by: None,
                kind: Kind::Model,
                sure: Sure::Certain,
                reasons: Vec::new(),
                keep: Vec::new(),
                can_trash: true,
                can_share: false,
                title,
                detail: format!(
                    "Removes it from Ollama ({tags} name{}), as \u{201C}ollama rm\u{201D} would. Files other models share are kept.",
                    if tags == 1 { "" } else { "s" }
                ),
                app: "Ollama",
                bytes,
                paths,
                within: store.to_path_buf(),
            })
        }
        Source::HuggingFace => {
            let repo = first.path.ancestors().find(|a| a.file_name().is_some_and(|n| n.to_string_lossy().starts_with("models--"))).ok_or("This model isn't in a Hugging Face folder Filescape recognises.")?;
            let mine: HashSet<PathBuf> = m.files.iter().map(|f| std::fs::canonicalize(&f.path).unwrap_or(f.path.clone())).collect();
            let mut paths: Vec<PathBuf> = m.files.iter().map(|f| f.path.clone()).collect();
            // Links in every version that lead to these files.
            let mut links = Vec::new();
            let _ = list_files(&repo.join("snapshots"), 8, &mut links);
            for l in links {
                if std::fs::symlink_metadata(&l).is_ok_and(|md| md.file_type().is_symlink()) && std::fs::canonicalize(&l).is_ok_and(|t| mine.contains(&t)) {
                    paths.push(l);
                }
            }
            Ok(Item {
                used_by: None,
                kind: Kind::Model,
                sure: Sure::Certain,
                reasons: Vec::new(),
                keep: Vec::new(),
                can_trash: true,
                can_share: false,
                title,
                detail: "Removes its files from the Hugging Face cache, and the links to them. It downloads again if a program asks for it.".into(),
                app: "Hugging Face",
                bytes: m.files.iter().map(|f| f.size).sum(),
                paths,
                within: repo.to_path_buf(),
            })
        }
        _ => {
            let within = common_parent(m.files.iter().map(|f| f.path.as_path())).ok_or("These files have no common folder.")?;
            Ok(Item {
                used_by: None,
                kind: Kind::Model,
                sure: Sure::Certain,
                reasons: Vec::new(),
                keep: Vec::new(),
                can_trash: true,
                can_share: false,
                title,
                detail: match m.source {
                    Source::LmStudio => "Removes its files from LM Studio's models folder. LM Studio no longer lists it.".into(),
                    _ if m.orphan => "Removes this download, which no Ollama model uses.".into(),
                    _ => "Removes its files.".into(),
                },
                app: m.source.label(),
                bytes: m.files.iter().map(|f| f.size).sum(),
                paths: m.files.iter().map(|f| f.path.clone()).collect(),
                within,
            })
        }
    }
}

fn common_parent<'a>(mut paths: impl Iterator<Item = &'a Path>) -> Option<PathBuf> {
    let mut c = paths.next()?.parent()?.to_path_buf();
    for p in paths {
        while !p.starts_with(&c) {
            c = c.parent()?.to_path_buf();
        }
    }
    Some(c)
}

// ---------- Hugging Face ----------

fn hf_items(repo: &Path, out: &mut Vec<Item>) {
    let name = repo.file_name().map(|n| n.to_string_lossy()["models--".len()..].replacen("--", "/", 1)).unwrap_or_default();
    // The versions in use: the ones a ref (main, a branch, a pull request) points at.
    let mut refs = Vec::new();
    if list_files(&repo.join("refs"), 4, &mut refs).is_err() {
        return;
    }
    let revs: HashSet<String> = refs.iter().filter_map(|r| std::fs::read_to_string(r).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let Ok(rd) = std::fs::read_dir(repo.join("snapshots")) else { return };
    let snaps: Vec<PathBuf> = rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
    let (current, old): (Vec<_>, Vec<_>) = snaps.into_iter().partition(|s| s.file_name().is_some_and(|n| revs.contains(&*n.to_string_lossy())));
    let blobs = repo.join("blobs");
    let blob_files: Vec<PathBuf> = std::fs::read_dir(&blobs).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    // Unfinished downloads are worth offering even when the versions can't be told apart.
    let partial: Vec<PathBuf> = blob_files.iter().filter(|p| p.extension().is_some_and(|e| e == "incomplete") && !busy(p)).cloned().collect();
    if !partial.is_empty() {
        let (sure, reasons) = stopped(&partial[0], "Hugging Face names files \u{201C}.incomplete\u{201D} until they finish");
        out.push(Item {
            used_by: None,
            kind: Kind::Partial,
            sure,
            reasons,
            keep: Vec::new(),
            can_trash: true,
            can_share: false,
            title: name.clone(),
            detail: format!("Hugging Face files that didn't finish downloading ({}). They download again when needed.", ago(&partial[0])),
            app: "Hugging Face",
            bytes: partial.iter().map(|p| size(p)).sum(),
            paths: partial,
            within: repo.to_path_buf(),
        });
    }
    if current.is_empty() {
        return;
    }
    // What the current versions use: link targets, or the files themselves where links aren't
    // possible (Windows without developer mode copies them in).
    let uses = |dir: &Path| -> Option<(HashSet<PathBuf>, Vec<PathBuf>)> {
        let mut files = Vec::new();
        list_files(dir, 8, &mut files).ok()?;
        let mut targets = HashSet::new();
        let mut own = Vec::new();
        for f in files {
            let md = std::fs::symlink_metadata(&f).ok()?;
            if md.file_type().is_symlink() {
                if let Ok(t) = std::fs::canonicalize(&f) {
                    targets.insert(t);
                }
            } else {
                own.push(f);
            }
        }
        Some((targets, own))
    };
    let mut used = HashSet::new();
    for c in &current {
        let Some((t, _)) = uses(c) else { return };
        used.extend(t);
    }
    let mut paths = Vec::new();
    let mut bytes = 0;
    let mut freed = HashSet::new();
    for o in &old {
        let Some((targets, own)) = uses(o) else { return };
        paths.push(o.clone());
        bytes += own.iter().map(|f| size(f)).sum::<u64>();
        for t in targets {
            if !used.contains(&t) && t.starts_with(std::fs::canonicalize(&blobs).unwrap_or(blobs.clone())) && freed.insert(t.clone()) {
                bytes += size(&t);
                paths.push(t);
            }
        }
    }
    // Blobs no version links to at all.
    let canon_blobs = std::fs::canonicalize(&blobs).unwrap_or(blobs.clone());
    // Only when the current versions are links: with copied-in files, blobs aren't used anyway.
    let linked = current.iter().all(|cur| uses(cur).is_some_and(|(t, own)| own.is_empty() && !t.is_empty()));
    let mut loose = 0u64;
    for b in &blob_files {
        let c = std::fs::canonicalize(b).unwrap_or(b.clone());
        if !linked || b.extension().is_some_and(|e| e == "incomplete") || used.contains(&c) || freed.contains(&c) || busy(b) || !c.starts_with(&canon_blobs) {
            continue;
        }
        loose += size(b);
        bytes += size(b);
        paths.push(b.clone());
    }
    if paths.is_empty() || bytes == 0 {
        return;
    }
    let n = old.len();
    let mut reasons = Vec::new();
    if n > 0 {
        reasons.push(format!("No branch or tag (such as main) points at {}; the current version is kept", if n == 1 { "this older version".to_string() } else { format!("these {n} older versions") }));
        reasons.push("A program that asks for an exact version by its code would still use it; most ask for main".into());
        reasons.push("Files the current version also uses are kept".into());
    }
    if loose > 0 {
        reasons.push(format!("{} of files no version links to at all", crate::scan::human(loose)));
    }
    out.push(Item {
        used_by: None,
        kind: Kind::OldVersion,
        // Unreferenced versions are only probably unused (see above); stray blobs certainly are.
        sure: if n > 0 { Sure::Likely } else { Sure::Certain },
        reasons,
        keep: Vec::new(),
        can_trash: true,
        can_share: false,
        title: name,
        detail: if n > 0 {
            format!(
                "{n} older version{} kept beside the current one{}. Programs use the current version.",
                if n == 1 { "" } else { "s" },
                if loose > 0 { ", and files no version uses" } else { "" }
            )
        } else {
            "Files no version of this model uses.".into()
        },
        app: "Hugging Face",
        bytes,
        paths,
        within: repo.to_path_buf(),
    });
}

// ---------- loose files ----------

/// "Qwen3-8B.Q4_K_M.gguf.crdownload" and the like: a model file a browser or tool didn't finish.
pub fn is_partial_model(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PARTIAL_ENDINGS.iter().any(|end| {
        lower.strip_suffix(end).is_some_and(|stem| stem.rsplit_once('.').is_some_and(|(_, ext)| Format::of_ext(ext).is_some_and(|f| f != Format::Other)))
    })
}

fn partial_files(root: &Path, dir: &Path, depth: u32, out: &mut Vec<Item>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        if t.is_dir() && depth > 0 && !name.starts_with('.') {
            partial_files(root, &e.path(), depth - 1, out);
        } else if t.is_file() && is_partial_model(&name) && !busy(&e.path()) {
            let p = e.path();
            let (sure, reasons) = stopped(&p, "Its name ends the way unfinished downloads do");
            out.push(Item {
                used_by: None,
                kind: Kind::Partial,
                sure,
                reasons,
                keep: Vec::new(),
                can_trash: true,
                can_share: false,
                title: name.clone(),
                detail: format!("A model download that didn't finish ({}).", ago(&p)),
                app: "Folder",
                bytes: size(&p),
                paths: vec![p],
                within: root.to_path_buf(),
            });
        }
    }
}

/// Models whose files are copies of another model's. A loose copy (in Downloads, say) can be
/// removed; any copy on the same drive as the one kept can be shared with it as one file.
fn copies(models: &[Model], out: &mut Vec<Item>) {
    for m in models {
        if m.orphan || m.same_as.is_empty() {
            continue;
        }
        let pairs: Vec<(&crate::models::File, &PathBuf)> = m.same_as.iter().filter_map(|(fi, k)| m.files.get(*fi).map(|f| (f, k))).collect();
        // Shown for information only: an app or extension may load either copy from where it is,
        // and an update may rewrite one in place, so neither removing nor sharing is offered.
        let used_by = pairs.iter().find_map(|(f, _)| app_owner(&f.path));
        let kept_by = pairs.iter().find_map(|(_, k)| app_owner(k));
        let (can_trash, can_share) = (false, false);
        let Some(within) = common_parent(pairs.iter().map(|(f, _)| f.path.as_path())) else { continue };
        let other = m.copies.iter().filter_map(|&i| models.get(i)).next();
        let bytes: u64 = pairs.iter().map(|(f, _)| f.size).sum();
        let whole = bytes >= m.size;
        let mut reasons = vec!["Same size and the same contents, compared in full".to_string()];
        if let Some(k) = pairs.first().map(|(_, k)| k) {
            reasons.push(format!("The other copy: {}", k.display()));
        }
        if let Some(who) = &used_by {
            reasons.push(format!("This copy is inside {who}'s own files, and {who} may load it from there"));
        }
        if let Some(who) = kept_by.as_ref().filter(|w| Some(*w) != used_by.as_ref()) {
            reasons.push(format!("The other copy is inside {who}'s own files"));
        }
        reasons.push("Listed for information: Filescape doesn't remove or share copies, since the app that loads either one may stop working".into());
        out.push(Item {
            kind: Kind::Copy,
            sure: Sure::Certain,
            reasons,
            title: m.name.clone(),
            detail: match other {
                Some(o) => format!(
                    "{} the same, byte for byte, as {} in {}.",
                    if whole { "All of it is" } else { "Part of it is" },
                    o.name,
                    o.source.label()
                ),
                None => "The same, byte for byte, as another model's files.".into(),
            },
            app: m.source.label(),
            bytes,
            paths: pairs.iter().map(|(f, _)| f.path.clone()).collect(),
            keep: pairs.iter().map(|(_, k)| (*k).clone()).collect(),
            can_trash,
            can_share,
            used_by,
            within,
        });
    }
}

/// Bundle endings: a folder with one of these is an app, or a part an app loads.
const BUNDLES: [&str; 8] = [".app", ".appex", ".framework", ".bundle", ".plugin", ".kext", ".xpc", ".mlmodelc"];

/// The app, extension or program whose own files this path is inside, if it's one: an app bundle,
/// an editor's extensions, a program's settings or data folder (a dot folder at home,
/// Application Support, AppData, .config, .local/share), or a package folder. Folders that only
/// store models (Ollama, LM Studio, Hugging Face's cache) aren't owners: models there are the
/// point of them.
pub fn app_owner(p: &Path) -> Option<String> {
    let parts: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let lower: Vec<String> = parts.iter().map(|c| c.to_ascii_lowercase()).collect();
    let stem = |s: &str| s.rsplit_once('.').map_or(s, |(a, _)| a).to_string();
    // The outermost app bundle names the app; .mlmodelc alone is a compiled model, not an owner.
    if let Some(i) = lower.iter().position(|c| BUNDLES[..7].iter().any(|e| c.ends_with(e) && c.len() > e.len())) {
        let outer = lower.iter().position(|c| c.ends_with(".app") && c.len() > 4).unwrap_or(i);
        return Some(stem(&parts[outer]));
    }
    let next = |i: usize| parts.get(i + 1).filter(|n| i + 2 < parts.len() || !n.contains('.')).cloned();
    for (i, c) in lower.iter().enumerate() {
        match c.as_str() {
            "extensions" | "node_modules" | "site-packages" | "dist-packages" => {
                if let Some(n) = parts.get(i + 1) {
                    if i + 1 < parts.len() - 1 {
                        return Some(n.clone());
                    }
                }
            }
            "application support" | "containers" | "group containers" if i > 0 && lower[i - 1] == "library" => return next(i),
            "appdata" => {
                if matches!(lower.get(i + 1).map(String::as_str), Some("roaming" | "local" | "locallow")) {
                    let n = next(i + 1)?;
                    return (!is_store(&n)).then_some(n);
                }
            }
            ".config" | ".local" | ".cache" => {
                let at = if c == ".local" && lower.get(i + 1).map(String::as_str) == Some("share") { i + 1 } else { i };
                let n = next(at)?;
                return (!is_store(&n)).then_some(n);
            }
            "program files" | "program files (x86)" if i <= 1 => return next(i),
            _ if c.starts_with('.') && c.len() > 1 && i + 1 < parts.len() => {
                let n = parts[i].trim_start_matches('.').to_string();
                return (!is_store(&n)).then_some(n);
            }
            _ => {}
        }
    }
    None
}

/// Folders whose whole job is keeping models, whatever app writes them.
fn is_store(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "ollama" | "lmstudio" | "lm-studio" | "lm studio" | "huggingface" | "models")
}

/// Identifies a file on its drive: two names with the same id are one file (hard links).
#[cfg(unix)]
pub fn file_id(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(p).ok()?;
    Some((m.dev(), m.ino()))
}

#[cfg(windows)]
pub fn file_id(p: &Path) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    let f = std::fs::File::open(p).ok()?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a handle open for the call, and a structure to fill.
    let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } != 0;
    ok.then(|| (info.dwVolumeSerialNumber as u64, ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64))
}

#[cfg(not(any(unix, windows)))]
pub fn file_id(_: &Path) -> Option<(u64, u64)> {
    None
}

/// Both on one drive, so one can be a hard link to the other. Unused while sharing copies is
/// turned off (Rich, 28 Nov: too risky inside apps and extensions).
#[allow(dead_code)]
fn same_drive(a: &Path, b: &Path) -> bool {
    matches!((file_id(a), file_id(b)), (Some(x), Some(y)) if x.0 == y.0 && x != y)
}

fn same_contents(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let (mut fa, mut fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let n = fa.read(&mut ba)?;
        if n == 0 {
            return Ok(true);
        }
        fb.read_exact(&mut bb[..n])?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

/// Makes each copy a second name of the file it's the same as (a hard link), checking first that
/// they're still identical. Both names keep working; the space of the copy is freed at once.
pub fn share(item: &Item) -> Vec<Removed> {
    let within = std::fs::canonicalize(&item.within).unwrap_or(item.within.clone());
    item.paths
        .iter()
        .zip(&item.keep)
        .map(|(copy, keep)| {
            let result = (|| -> Result<(), String> {
                let real = copy.parent().and_then(|d| std::fs::canonicalize(d).ok()).map(|d| d.join(copy.file_name().unwrap_or_default()));
                if real.is_none_or(|r| !r.starts_with(&within)) {
                    return Err("It isn't where it was listed, so it was left alone.".into());
                }
                if file_id(copy).is_some() && file_id(copy) == file_id(keep) {
                    return Ok(());
                }
                if !same_contents(copy, keep).map_err(|e| e.to_string())? {
                    return Err("The two files are no longer the same, so both were kept.".into());
                }
                let name = copy.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let tmp = copy.with_file_name(format!(".{name}.filescape-link"));
                let _ = std::fs::remove_file(&tmp);
                std::fs::hard_link(keep, &tmp).map_err(|e| format!("This drive can't share files ({e})."))?;
                std::fs::rename(&tmp, copy).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    format!("The copy couldn't be replaced (is a program using it?): {e}")
                })
            })();
            Removed { path: copy.clone(), result }
        })
        .collect()
}

// ---------- removing ----------

/// What became of one path.
#[derive(Debug)]
pub struct Removed {
    pub path: PathBuf,
    pub result: Result<(), String>,
}

/// Removes an item's paths: to the Trash (`trash`) or for good. Paths already gone are skipped;
/// paths outside the item's folder are refused.
pub fn remove(item: &Item, trash: bool) -> Vec<Removed> {
    let within = std::fs::canonicalize(&item.within).unwrap_or(item.within.clone());
    item.paths
        .iter()
        .filter(|p| std::fs::symlink_metadata(p).is_ok())
        .map(|p| {
            // The folder a path is in, resolved: links inside it are removed, not followed.
            let real = p.parent().and_then(|d| std::fs::canonicalize(d).ok()).map(|d| d.join(p.file_name().unwrap_or_default()));
            let result = if real.is_none_or(|r| !r.starts_with(&within)) {
                Err("It isn't where it was listed, so it was left alone.".into())
            } else if trash {
                crate::actions::trash_keep(p).map(|_| ())
            } else {
                crate::actions::delete(p).map_err(|e| e.to_string())
            };
            Removed { path: p.clone(), result }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_inside_apps_are_theirs() {
        let o = |p: &str| app_owner(Path::new(p));
        assert_eq!(o("/Users/r/.antigravity/extensions/devsense.intelli-php-0.12/out/models/intelliphp_v3/model.onnx").as_deref(), Some("antigravity"));
        assert_eq!(o("/Applications/Logic Pro.app/Contents/Resources/mBERT.bundle/model.bin").as_deref(), Some("Logic Pro"));
        assert_eq!(o("/Users/r/Music/Plug-ins/mBERT.bundle/weights.bin").as_deref(), Some("mBERT"));
        assert_eq!(o("/Users/r/Library/Application Support/Topaz/models/a.onnx").as_deref(), Some("Topaz"));
        assert_eq!(o("C:/Users/r/AppData/Roaming/Krita/models/a.onnx").as_deref(), Some("Krita"));
        assert_eq!(o("/home/r/.local/share/audacity/models/a.onnx").as_deref(), Some("audacity"));
        assert_eq!(o("/home/r/.cache/torch/hub/a.pt").as_deref(), Some("torch"));
        assert_eq!(o("/home/r/venv/lib/python3.12/site-packages/rembg/u2net.onnx").as_deref(), Some("rembg"));
        // Model stores, and the user's own folders, belong to no app.
        assert_eq!(o("/home/r/.ollama/models/blobs/sha256-ab"), None);
        assert_eq!(o("/home/r/.cache/huggingface/hub/models--a--b/blobs/x"), None);
        assert_eq!(o("/home/r/.lmstudio/models/a/b.gguf"), None);
        assert_eq!(o("/home/r/Downloads/qwen.gguf"), None);
        assert_eq!(o("/home/r/Models/sd/x.mlmodelc/weights.bin"), None);
    }

    use std::fs;

    /// A fresh folder for one test, removed when dropped.
    struct Temp(PathBuf);
    impl Temp {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir(name: &str) -> Temp {
        let d = std::env::temp_dir().join(format!("filescape-cleanup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Temp(fs::canonicalize(&d).unwrap())
    }

    fn old(p: &Path) {
        let t = fs::File::options().write(true).open(p).unwrap();
        t.set_modified(SystemTime::now() - Duration::from_secs(3 * 86_400)).unwrap();
    }

    fn write(p: &Path, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![7u8; bytes]).unwrap();
        old(p);
    }

    fn manifest(store: &Path, name: &str, digests: &[&str]) {
        let layers: Vec<serde_json::Value> =
            digests.iter().map(|d| serde_json::json!({"mediaType": "application/vnd.ollama.image.model", "digest": d.replace("sha256-", "sha256:")})).collect();
        let p = store.join("manifests/registry.ollama.ai/library").join(name).join("latest");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, serde_json::json!({"config": {"digest": "sha256:cfg"}, "layers": layers}).to_string()).unwrap();
    }

    #[test]
    fn ollama_unused_and_partial() {
        let d = tempdir("ollama");
        let s = d.path().join("models");
        manifest(&s, "a", &["sha256-aaa", "sha256-shared"]);
        manifest(&s, "b", &["sha256-bbb", "sha256-shared"]);
        for b in ["sha256-aaa", "sha256-bbb", "sha256-shared", "sha256-cfg", "sha256-gone", "sha256-dl-partial", "sha256-dl-partial-0"] {
            write(&s.join("blobs").join(b), 100);
        }
        write(&s.join("blobs/sha256-new-partial"), 100);
        // Still being written: left alone.
        fs::File::options().write(true).open(s.join("blobs/sha256-new-partial")).unwrap().set_modified(SystemTime::now()).unwrap();
        let items = find(std::slice::from_ref(&s), &[]);
        let unused: Vec<_> = items.iter().filter(|i| i.kind == Kind::Unused).collect();
        assert_eq!(unused.len(), 1, "{items:?}");
        assert_eq!(unused[0].paths, [s.join("blobs/sha256-gone")]);
        let partial: Vec<_> = items.iter().filter(|i| i.kind == Kind::Partial).collect();
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].paths.len(), 2);
        assert_eq!(partial[0].bytes, 200);
        // An unreadable manifest: nothing is called unused.
        fs::write(s.join("manifests/registry.ollama.ai/library/b/latest"), "not json").unwrap();
        assert!(find(std::slice::from_ref(&s), &[]).iter().all(|i| i.kind != Kind::Unused));
    }

    #[test]
    #[cfg(unix)]
    fn what_another_model_links_to_is_kept() {
        let d = tempdir("linked");
        let s = d.path().join("models");
        manifest(&s, "a", &["sha256-aaa"]);
        for b in ["sha256-aaa", "sha256-gone", "sha256-lost"] {
            write(&s.join("blobs").join(b), 100);
        }
        // A tool linked an Ollama download into LM Studio's folder; then the Ollama model was removed.
        let lms = d.path().join("lmstudio/pub/m/m.gguf");
        fs::create_dir_all(lms.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(s.join("blobs/sha256-gone"), &lms).unwrap();
        let m = Model {
            name: "Linked".into(),
            source: Source::LmStudio,
            format: Format::Gguf,
            info: Default::default(),
            files: vec![crate::models::File { path: lms, size: 100, not_local: false }],
            size: 100,
            copies: vec![],
            extra: 0,
            same_as: vec![],
            orphan: false,
        };
        let items = find(std::slice::from_ref(&s), std::slice::from_ref(&m));
        let unused: Vec<_> = items.iter().filter(|i| i.kind == Kind::Unused).collect();
        assert_eq!(unused.len(), 1, "{items:?}");
        assert_eq!(unused[0].paths, [s.join("blobs/sha256-lost")]);
        assert_eq!(unused[0].sure, Sure::Certain);
        assert!(unused[0].reasons[0].contains("None of the 1 model in Ollama's own list"), "{:?}", unused[0].reasons);
        assert!(unused[0].reasons.iter().any(|r| r.contains("No other model")));
        // The same link found by looking in LM Studio's folder, not in the list of models.
        let items = find(&[s.clone(), d.path().join("lmstudio")], &[]);
        let gone: Vec<_> = items.iter().filter(|i| i.kind == Kind::Unused).flat_map(|i| i.paths.iter()).collect();
        assert_eq!(gone, [&s.join("blobs/sha256-lost")]);
        // Removing the download as a model from the list is refused for the same reason.
        let orphan = Model { name: "gone".into(), source: Source::Ollama, orphan: true, files: vec![crate::models::File { path: s.join("blobs/sha256-gone"), size: 100, not_local: false }], ..m };
        let p = plan(&orphan).unwrap();
        let who = linked_from(&p, &[d.path().join("lmstudio")], &[]);
        assert_eq!(who.len(), 1);
        assert!(who[0].ends_with("m.gguf"), "{who:?}");
        assert!(linked_from(&p, &[s.clone()], &[]).is_empty());
    }

    #[test]
    fn newer_stopped_downloads_are_only_probably_unused() {
        let d = tempdir("stopped");
        let old_one = d.path().join("old.gguf.crdownload");
        let new_one = d.path().join("new.gguf.part");
        write(&old_one, 100);
        write(&new_one, 100);
        fs::File::options().write(true).open(&new_one).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        let items = find(&[d.path().to_path_buf()], &[]);
        let sure = |n: &str| items.iter().find(|i| i.title == n).unwrap().sure;
        assert_eq!((sure("old.gguf.crdownload"), sure("new.gguf.part")), (Sure::Certain, Sure::Likely));
        assert!(items.iter().find(|i| i.title == "new.gguf.part").unwrap().reasons.iter().any(|r| r.contains("carry on")));
    }

    #[test]
    fn removing_an_ollama_model_keeps_shared_files() {
        let d = tempdir("rmollama");
        let s = d.path().join("models");
        manifest(&s, "a", &["sha256-aaa", "sha256-shared"]);
        manifest(&s, "b", &["sha256-bbb", "sha256-shared"]);
        for b in ["sha256-aaa", "sha256-bbb", "sha256-shared", "sha256-cfg"] {
            write(&s.join("blobs").join(b), 100);
        }
        let m = Model {
            name: "a".into(),
            source: Source::Ollama,
            format: Format::Gguf,
            info: Default::default(),
            files: vec![crate::models::File { path: s.join("blobs/sha256-aaa"), size: 100, not_local: false }],
            size: 100,
            copies: vec![],
            extra: 0,
            same_as: vec![],
            orphan: false,
        };
        let p = plan(&m).unwrap();
        let names: HashSet<String> = p.paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(names.contains("sha256-aaa") && names.contains("latest"), "{names:?}");
        assert!(!names.contains("sha256-shared") && !names.contains("sha256-cfg") && !names.contains("sha256-bbb"));
        let r = remove(&p, false);
        assert!(r.iter().all(|r| r.result.is_ok()), "{r:?}");
        assert!(!s.join("blobs/sha256-aaa").exists() && s.join("blobs/sha256-shared").exists() && s.join("manifests/registry.ollama.ai/library/b/latest").exists());
        assert!(!s.join("manifests/registry.ollama.ai/library/a/latest").exists());
    }

    #[cfg(unix)]
    #[test]
    fn hugging_face_old_versions() {
        let d = tempdir("hf");
        let r = d.path().join("models--org--m");
        for b in ["new", "old", "same", "stray"] {
            write(&r.join("blobs").join(b), 1000);
        }
        write(&r.join("blobs/half.incomplete"), 500);
        fs::create_dir_all(r.join("refs")).unwrap();
        fs::write(r.join("refs/main"), "rev2").unwrap();
        for (rev, files) in [("rev1", [("model.safetensors", "old"), ("config.json", "same")]), ("rev2", [("model.safetensors", "new"), ("config.json", "same")])] {
            fs::create_dir_all(r.join("snapshots").join(rev)).unwrap();
            for (n, b) in files {
                std::os::unix::fs::symlink(format!("../../blobs/{b}"), r.join("snapshots").join(rev).join(n)).unwrap();
            }
        }
        let items = find(&[d.path().to_path_buf()], &[]);
        let old: Vec<_> = items.iter().filter(|i| i.kind == Kind::OldVersion).collect();
        assert_eq!(old.len(), 1, "{items:?}");
        assert_eq!(old[0].title, "org/m");
        // The old snapshot, its own blob and the stray one; not what the current one uses.
        assert_eq!(old[0].bytes, 2000);
        let names: Vec<String> = old[0].paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(names.contains(&"rev1".into()) && names.contains(&"old".into()) && names.contains(&"stray".into()), "{names:?}");
        assert!(!names.contains(&"same".into()) && !names.contains(&"new".into()));
        assert!(items.iter().any(|i| i.kind == Kind::Partial && i.bytes == 500));
        assert_eq!(old[0].sure, Sure::Likely);
        // A folder elsewhere links to the old version's file: that version and its file stay.
        let link = d.path().join("comfy/model.safetensors");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(r.join("snapshots/rev1/model.safetensors"), &link).unwrap();
        let m = Model {
            name: "Comfy".into(),
            source: Source::Folder,
            format: Format::Safetensors,
            info: Default::default(),
            files: vec![crate::models::File { path: link, size: 1000, not_local: false }],
            size: 1000,
            copies: vec![],
            extra: 0,
            same_as: vec![],
            orphan: false,
        };
        let items = find(&[d.path().to_path_buf()], std::slice::from_ref(&m));
        let old = items.iter().find(|i| i.kind == Kind::OldVersion).unwrap();
        let names: Vec<String> = old.paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["stray"]);
        assert_eq!(old.bytes, 1000);
        assert!(old.reasons.iter().any(|r| r.contains("because of a link to it from Comfy (")), "{:?}", old.reasons);
        // No refs: the versions can't be told apart, so none is offered.
        fs::remove_file(r.join("refs/main")).unwrap();
        assert!(find(&[d.path().to_path_buf()], &[]).iter().all(|i| i.kind != Kind::OldVersion));
    }

    #[test]
    fn sharing_copies_as_one_file() {
        let d = tempdir("share");
        let keep = d.path().join("app/model.gguf");
        let copy = d.path().join("Downloads/model.gguf");
        write(&keep, 5000);
        write(&copy, 5000);
        assert!(same_drive(&copy, &keep));
        let item = Item {
            used_by: None,
            kind: Kind::Copy,
            sure: Sure::Certain,
            reasons: Vec::new(),
            title: "m".into(),
            detail: String::new(),
            app: "Folder",
            paths: vec![copy.clone()],
            bytes: 5000,
            within: d.path().join("Downloads"),
            keep: vec![keep.clone()],
            can_trash: true,
            can_share: true,
        };
        let r = share(&item);
        assert!(r[0].result.is_ok(), "{r:?}");
        assert_eq!(file_id(&copy), file_id(&keep));
        assert_eq!(fs::read(&copy).unwrap().len(), 5000);
        // Already one file: nothing to do, and no longer "the same drive, another file".
        assert!(!same_drive(&copy, &keep));
        assert!(share(&item)[0].result.is_ok());
        // Files that differ now are both kept.
        let other = d.path().join("Downloads/other.gguf");
        write(&other, 5000);
        fs::write(&other, vec![8u8; 5000]).unwrap();
        let item2 = Item { paths: vec![other.clone()], ..item };
        assert!(share(&item2)[0].result.is_err());
        assert_ne!(file_id(&other), file_id(&keep));
    }

    #[test]
    fn partial_names() {
        assert!(is_partial_model("Qwen3-8B.Q4_K_M.gguf.crdownload"));
        assert!(is_partial_model("model.safetensors.part"));
        assert!(!is_partial_model("notes.txt.part"));
        assert!(!is_partial_model("model.gguf"));
        assert!(!is_partial_model("film.mp4.crdownload"));
    }

    #[test]
    fn remove_refuses_paths_elsewhere() {
        let d = tempdir("refuse");
        let inside = d.path().join("in/x.gguf");
        let outside = d.path().join("out/y.gguf");
        write(&inside, 10);
        write(&outside, 10);
        let item = Item { kind: Kind::Copy, sure: Sure::Certain, reasons: vec![], used_by: None, keep: vec![], can_trash: true, can_share: false, title: "x".into(), detail: String::new(), app: "Folder", paths: vec![inside.clone(), outside.clone()], bytes: 20, within: d.path().join("in") };
        let r = remove(&item, false);
        assert!(r[0].result.is_ok() && r[1].result.is_err());
        assert!(!inside.exists() && outside.exists());
    }
}
