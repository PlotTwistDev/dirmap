//! The Craft Apps' own files (getartcraft.com): PhotoCraft, VectorCraft, EffectCraft, FigCraft,
//! DesignCraft, TextureCraft, DeckCraft, AniCraft, MaterialCraft, FilmCraft and SoundCraft.
//! Filescape isn't connected with ArtCraft; it reads these files the way it reads other apps'.
//!
//! What's shown is the picture the app saved inside the file, when it saved one (a VectorCraft
//! file's PDF is drawn here, so it stays sharp at any size), and what the file says about itself:
//! the version that saved it, pages, artboards, layers, compositions, tracks and so on.
//! PhotoCraft, VectorCraft and EffectCraft files are read with `craft-doc` from the apps' own
//! libraries; the others are small JSON or ZIP readers here. Nothing in a file is run: no
//! expressions, node graphs, plugins or scripts, and no file a project mentions is opened.
//!
//! Every app refuses a file saved by a newer version of itself, and so does this: a newer file
//! shows its saved picture and the version that wrote it, never a half-read guess. The format
//! notes are in `docs/design/filescape-artcraft.md`.

use crate::savedpic::{Saved, decode, zip_bytes, zip_open};
use serde_json::Value;
use std::path::Path;

/// Files read whole (JSON documents) are no larger than this. The apps' own limits are 256 MB
/// (SoundCraft, EffectCraft) to 1 GB (AniCraft); documents that large are rare.
const FILE_MAX: u64 = 256 << 20;
/// The largest JSON member read out of a ZIP document.
const JSON_MAX: usize = 128 << 20;
/// Saved pictures larger than this aren't decoded.
const PIC_MAX: usize = 16 << 20;
/// How many names a list of pages, layers or tracks shows before "and N more".
const NAMES: usize = 12;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Craft {
    PhotoCraft,
    VectorCraft,
    EffectCraft,
    /// EffectCraft's Essential Graphics template: a ZIP with a poster and a project.
    EffectTemplate,
    FigCraft,
    DesignCraft,
    TextureCraft,
    DeckCraft,
    AniCraft,
    MaterialCraft,
    FilmCraft,
    SoundCraft,
    /// SoundCraft's bundle: the session and its sound files in one ZIP.
    SoundBundle,
}

impl Craft {
    pub fn name(self) -> &'static str {
        match self {
            Craft::PhotoCraft => "PhotoCraft",
            Craft::VectorCraft => "VectorCraft",
            Craft::EffectCraft | Craft::EffectTemplate => "EffectCraft",
            Craft::FigCraft => "FigCraft",
            Craft::DesignCraft => "DesignCraft",
            Craft::TextureCraft => "TextureCraft",
            Craft::DeckCraft => "DeckCraft",
            Craft::AniCraft => "AniCraft",
            Craft::MaterialCraft => "MaterialCraft",
            Craft::FilmCraft => "FilmCraft",
            Craft::SoundCraft | Craft::SoundBundle => "SoundCraft",
        }
    }

    /// What the file is, for the Kind column: "FilmCraft project".
    pub fn kind(self) -> &'static str {
        match self {
            Craft::PhotoCraft => "PhotoCraft picture",
            Craft::VectorCraft => "VectorCraft drawing",
            Craft::EffectCraft => "EffectCraft project",
            Craft::EffectTemplate => "EffectCraft template",
            Craft::FigCraft => "FigCraft design",
            Craft::DesignCraft => "DesignCraft layout",
            Craft::TextureCraft => "TextureCraft project",
            Craft::DeckCraft => "DeckCraft presentation",
            Craft::AniCraft => "AniCraft animation",
            Craft::MaterialCraft => "MaterialCraft material",
            Craft::FilmCraft => "FilmCraft project",
            Craft::SoundCraft => "SoundCraft session",
            Craft::SoundBundle => "SoundCraft bundle",
        }
    }

    /// What, if anything, makes the app save a picture inside the file, for the note shown when
    /// there's none.
    fn no_picture(self) -> &'static str {
        match self {
            Craft::PhotoCraft => "PhotoCraft saves a picture of the document with it only when asked to; this file has none.",
            Craft::VectorCraft => "VectorCraft saves a picture and a PDF of the artboards only when asked to (Create PDF-Compatible File); this file has neither.",
            Craft::TextureCraft => "TextureCraft saves a picture of the model with it only when asked to; this file has none.",
            _ => "This app doesn't save a picture inside its files, and Filescape doesn't draw them itself.",
        }
    }
}

/// The Craft app a file name belongs to, by its extension.
pub fn of(name: &str) -> Option<Craft> {
    Some(match crate::scan::ext_of(name).to_ascii_lowercase().as_str() {
        "pcraft" => Craft::PhotoCraft,
        "vectorcraft" | "drawcraft" | "vctemplate" => Craft::VectorCraft,
        "ecproj" => Craft::EffectCraft,
        "ectemplate" => Craft::EffectTemplate,
        "figcraft" => Craft::FigCraft,
        "designcraft" => Craft::DesignCraft,
        "tcraft" => Craft::TextureCraft,
        "deckcraft" => Craft::DeckCraft,
        "anicraft" => Craft::AniCraft,
        "mcraft" => Craft::MaterialCraft,
        "fcproj" => Craft::FilmCraft,
        "scraft" => Craft::SoundCraft,
        "scraftz" => Craft::SoundBundle,
        _ => return None,
    })
}

/// The newest format version of each app this reader knows, from the apps' sources at the
/// craft-libs commit in Cargo.toml (PhotoCraft, VectorCraft and EffectCraft are checked by
/// craft-doc itself).
mod newest {
    pub const FIGCRAFT: u64 = 1;
    pub const DESIGNCRAFT: u64 = 5;
    pub const TEXTURECRAFT: u64 = 3;
    pub const DECKCRAFT: u64 = 1;
    pub const ANICRAFT: u64 = 3;
    pub const FILMCRAFT: u64 = 17;
    pub const SOUNDCRAFT: u64 = 2;
}

pub fn read(path: &Path) -> Result<Saved, String> {
    let craft = path.file_name().and_then(|n| of(&n.to_string_lossy())).ok_or("Not a Craft Apps file")?;
    if path.is_dir() && craft != Craft::PhotoCraft {
        return Err(format!("A {} bundle (a folder); open it to see inside", craft.name()));
    }
    let mut out = Saved { picture: None, facts: vec![("App", craft.name().to_string())], note: None, refs: None };
    let base = path.parent();
    match craft {
        Craft::PhotoCraft => photo(path, &mut out)?,
        Craft::VectorCraft => vector(&whole(path)?, &mut out)?,
        Craft::EffectCraft => {
            let b = whole(path)?;
            effect(&b, &mut out)?;
            if let Ok(d) = json(&b) {
                uses(&mut out, "Footage", effect_files(&d), base);
            }
        }
        Craft::EffectTemplate => {
            let mut zip = zip_open(path).ok_or("Not an EffectCraft template")?;
            out.picture = zip_bytes(&mut zip, "poster.png", PIC_MAX).and_then(|b| decode(&b));
            let project = zip_bytes(&mut zip, "project.ecproj", JSON_MAX).ok_or("Not an EffectCraft template")?;
            out.facts.push(("Kind", "Essential Graphics template".into()));
            effect(&project, &mut out)?;
        }
        Craft::FigCraft => fig(path, &mut out)?,
        Craft::DesignCraft => design(path, &mut out)?,
        Craft::TextureCraft => texture(path, &mut out)?,
        Craft::DeckCraft => deck(path, &mut out)?,
        Craft::AniCraft => ani(&json(&whole(path)?)?, &mut out)?,
        Craft::MaterialCraft => material(path, &mut out)?,
        Craft::FilmCraft => {
            let d = json(&whole(path)?)?;
            film(&d, &mut out)?;
            uses(&mut out, "Media files", film_files(&d), base);
        }
        Craft::SoundCraft => {
            let d = json(&whole(path)?)?;
            sound(&d, &mut out)?;
            uses(&mut out, "Sound and video files", sound_files(&d, true), base);
        }
        Craft::SoundBundle => {
            let mut zip = zip_open(path).ok_or("Not a SoundCraft bundle")?;
            let name = entries(&zip).into_iter().find(|n| n.ends_with(".scraft") && !n.contains('/'));
            let session = name.and_then(|n| zip_bytes(&mut zip, &n, JSON_MAX)).ok_or("Not a SoundCraft bundle")?;
            let audio = entries(&zip).into_iter().filter(|n| !n.ends_with(".scraft") && !n.ends_with('/')).count();
            let d = json(&session)?;
            sound(&d, &mut out)?;
            out.facts.push(("Sound files inside", audio.to_string()));
            // Its sound files are inside; the movies it uses never are.
            uses(&mut out, "Video files", sound_files(&d, false), base);
        }
    }
    if let Some(p) = &out.picture {
        out.facts.push(("Saved picture", format!("{} × {}", p.width(), p.height())));
    } else {
        out.note = Some(craft.no_picture().into());
    }
    Ok(out)
}

/// The whole file, when it isn't too big to hold.
fn whole(path: &Path) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't read it: {e}"))?.len();
    if size > FILE_MAX {
        return Err(format!("Too big to read ({})", crate::scan::human(size)));
    }
    std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))
}

fn json(b: &[u8]) -> Result<Value, String> {
    // serde_json stops at 128 levels of nesting, so a hostile file can't exhaust the stack.
    serde_json::from_slice(b).map_err(|_| "The file couldn't be read (it may be damaged).".to_string())
}

/// "a, b, c and 4 more".
fn names<'a>(list: impl Iterator<Item = &'a str>) -> String {
    let all: Vec<&str> = list.map(|s| if s.trim().is_empty() { "(unnamed)" } else { s }).collect();
    let mut s = all.iter().take(NAMES).copied().collect::<Vec<_>>().join(", ");
    if all.len() > NAMES {
        s.push_str(&format!(" and {} more", all.len() - NAMES));
    }
    s
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

fn seconds(s: f64) -> String {
    if !s.is_finite() || s < 0.0 {
        return "unknown".into();
    }
    let t = s.round() as u64;
    if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
}

fn fps(num: f64, den: f64) -> String {
    if den <= 0.0 || num <= 0.0 {
        return "unknown".into();
    }
    let r = num / den;
    if (r - r.round()).abs() < 0.001 { format!("{} fps", r.round()) } else { format!("{r:.3} fps") }
}

/// The version a file declares, against the newest this reads: true when it can be read.
fn version(out: &mut Saved, found: Option<u64>, newest: u64) -> bool {
    let app = out.facts[0].1.clone();
    match found {
        Some(v) if v > newest => {
            out.facts.push(("Format", format!("version {v}, from a newer {app} than Filescape reads (up to {newest})")));
            false
        }
        Some(v) => {
            out.facts.push(("Format", format!("version {v}")));
            true
        }
        None => true,
    }
}

fn saved_by(out: &mut Saved, v: &Value, key: &str) {
    if let Some(g) = v.get(key).and_then(Value::as_str).filter(|g| !g.trim().is_empty()) {
        out.facts.push(("Saved by", g.trim().to_string()));
    }
}

// PhotoCraft, VectorCraft and EffectCraft, through craft-doc.

fn limits() -> craft_doc::Limits {
    let mut l = craft_doc::Limits::default();
    l.max_json = JSON_MAX as u64;
    l.max_entry = JSON_MAX as u64;
    l.max_total = 512 << 20;
    l
}

/// What craft-doc says about the file: the app's version and the format's. False when the file
/// is newer than craft-doc reads.
fn doc_info(b: &[u8], out: &mut Saved) -> Result<bool, String> {
    let info = craft_doc::info(b, &limits()).map_err(|e| doc_error(&e, &out.facts[0].1))?;
    if let Some(g) = info.generator.as_deref().filter(|g| !g.trim().is_empty()) {
        out.facts.push(("Saved by", g.trim().to_string()));
    }
    if info.supported {
        out.facts.push(("Format", format!("version {}", info.version)));
    } else {
        let app = out.facts[0].1.clone();
        out.facts.push(("Format", format!("version {}, from a newer {app} than Filescape reads", info.version)));
    }
    Ok(info.supported)
}

fn doc_error(e: &craft_doc::Error, app: &str) -> String {
    let s = e.to_string();
    if s.contains("limit") || s.contains("too large") || s.contains("budget") { format!("Too big to read: {s}") } else { format!("Not a {app} file Filescape can read ({s})") }
}

fn picture(b: &[u8], kinds: &[craft_doc::RenditionKind]) -> Option<image::DynamicImage> {
    kinds.iter().find_map(|&k| {
        let r = craft_doc::rendition(b, k, &limits()).ok().flatten()?;
        if k == craft_doc::RenditionKind::Pdf {
            let doc = crate::pdfview::open_bytes(r.bytes).ok()?;
            let (w, h, rgba) = doc.first(2048).ok()?;
            image::RgbaImage::from_raw(w as u32, h as u32, rgba).map(image::DynamicImage::ImageRgba8)
        } else if r.bytes.len() <= PIC_MAX {
            decode(&r.bytes)
        } else {
            None
        }
    })
}

fn layer_tree(layers: &[craft_doc::Layer]) -> (usize, usize) {
    let (mut n, mut hidden) = (0, 0);
    for l in layers {
        n += 1;
        if !l.visible {
            hidden += 1;
        }
        let (a, b) = layer_tree(&l.children);
        n += a;
        hidden += b;
    }
    (n, hidden)
}

fn layers_fact(out: &mut Saved, layers: &[craft_doc::Layer]) {
    if layers.is_empty() {
        return;
    }
    let (n, hidden) = layer_tree(layers);
    let mut s = count(n, "layer", "layers");
    if hidden > 0 {
        s.push_str(&format!(" ({hidden} hidden)"));
    }
    out.facts.push(("Layers", s));
    // Top of the stack first, as the apps' Layers panels list them.
    out.facts.push(("Top layers", names(layers.iter().rev().map(|l| l.name.as_str()))));
}

/// A PhotoCraft bundle: a ZIP of tiles (often gigabytes) around a small manifest. Only the
/// manifest and the saved pictures are taken out of it, into a small ZIP that craft-doc reads,
/// so a large document is never held in memory.
fn photo(path: &Path, out: &mut Saved) -> Result<(), String> {
    // A PhotoCraft document is a ZIP, or the same files in a folder (a bundle). Only the
    // manifest and the saved pictures are copied, never the tiles, which can be gigabytes.
    let mut get: Box<dyn FnMut(&str, usize) -> Option<Vec<u8>>> = if path.is_dir() {
        let dir = path.to_path_buf();
        Box::new(move |name: &str, max: usize| {
            let f = dir.join(name);
            let md = std::fs::symlink_metadata(&f).ok()?;
            (md.is_file() && md.len() <= max as u64).then(|| std::fs::read(&f).ok()).flatten()
        })
    } else {
        let mut zip = zip_open(path).ok_or("Not a PhotoCraft document")?;
        Box::new(move |name: &str, max: usize| zip_bytes(&mut zip, name, max))
    };
    let manifest = get("manifest.json", JSON_MAX).ok_or("Not a PhotoCraft document")?;
    let mut small = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut add = |name: &str, data: &[u8]| -> Option<()> {
        use std::io::Write;
        small.start_file(name, stored).ok()?;
        small.write_all(data).ok()
    };
    add("manifest.json", &manifest).ok_or("Couldn't read it")?;
    for name in ["composite/full.png", "composite/preview.png", "thumb.png"] {
        if let Some(b) = get(name, PIC_MAX) {
            add(name, &b).ok_or("Couldn't read it")?;
        }
    }
    let b = small.finish().map_err(|_| "Couldn't read it")?.into_inner();
    let current = doc_info(&b, out)?;
    out.picture = picture(&b, &[craft_doc::RenditionKind::Composite, craft_doc::RenditionKind::Thumbnail]);
    if !current {
        return Ok(());
    }
    let o = craft_doc::outline(&b, &limits()).map_err(|e| doc_error(&e, "PhotoCraft"))?;
    if let Some(c) = &o.canvas {
        out.facts.push(("Size", format!("{} × {} pixels", c.width, c.height)));
        if c.resolution_dpi > 0.0 {
            out.facts.push(("Resolution", format!("{} dpi", c.resolution_dpi.round())));
        }
        let mode = match c.mode.as_str() {
            "Rgb" => "RGB",
            "Cmyk" => "CMYK",
            "Grayscale" => "Greyscale",
            "Lab" => "Lab",
            other => other,
        };
        let depth = match c.depth.as_str() {
            "U8" => "8 bits a channel",
            "U16" => "16 bits a channel",
            "F32" => "32 bits a channel (floating point)",
            other => other,
        };
        out.facts.push(("Colour", format!("{mode}, {depth}")));
    }
    layers_fact(out, &o.layers);
    Ok(())
}

fn vector(b: &[u8], out: &mut Saved) -> Result<(), String> {
    let current = doc_info(b, out)?;
    out.picture = picture(b, &[craft_doc::RenditionKind::Pdf, craft_doc::RenditionKind::Thumbnail]);
    if !current {
        return Ok(());
    }
    let o = craft_doc::outline(b, &limits()).map_err(|e| doc_error(&e, "VectorCraft"))?;
    if !o.pages.is_empty() {
        out.facts.push(("Artboards", count(o.pages.len(), "artboard", "artboards")));
        let list = o.pages.iter().map(|p| format!("{} ({:.0} × {:.0} pt)", if p.name.trim().is_empty() { "(unnamed)" } else { p.name.trim() }, p.width.abs(), p.height.abs())).collect::<Vec<_>>();
        out.facts.push(("Artboard sizes", names(list.iter().map(String::as_str))));
    }
    layers_fact(out, &o.layers);
    Ok(())
}

fn effect(b: &[u8], out: &mut Saved) -> Result<(), String> {
    if craft_doc::sniff(&b[..b.len().min(64 << 10)]) != Some(craft_doc::Kind::EffectCraft) {
        return Err("Not an EffectCraft project".into());
    }
    if !doc_info(b, out)? {
        return Ok(());
    }
    let o = craft_doc::outline(b, &limits()).map_err(|e| doc_error(&e, "EffectCraft"))?;
    out.facts.push(("Compositions", count(o.pages.len(), "composition", "compositions")));
    for p in o.pages.iter().take(NAMES) {
        let mut s = format!("{:.0} × {:.0}", p.width, p.height);
        if let Some(t) = p.timing {
            s.push_str(&format!(", {}, {}", fps(t.frame_rate_num as f64, t.frame_rate_den as f64), seconds(t.duration_seconds)));
        }
        s.push_str(&format!(", {}", count(p.layers.len(), "layer", "layers")));
        let name = if p.name.trim().is_empty() { "(unnamed)".to_string() } else { p.name.trim().to_string() };
        out.facts.push(("Composition", format!("{name}: {s}")));
    }
    if o.pages.len() > NAMES {
        out.facts.push(("Composition", format!("and {} more", o.pages.len() - NAMES)));
    }
    Ok(())
}

// The other apps: their JSON, read for the few things worth showing.

fn fig(path: &Path, out: &mut Saved) -> Result<(), String> {
    let mut zip = zip_open(path).ok_or("Not a FigCraft file")?;
    let doc = json(&zip_bytes(&mut zip, "document.json", JSON_MAX).ok_or("Not a FigCraft file")?)?;
    if doc.get("format").and_then(Value::as_str) != Some("figcraft") {
        return Err("Not a FigCraft file".into());
    }
    out.picture = zip_bytes(&mut zip, "thumbnail.png", PIC_MAX).and_then(|b| decode(&b));
    if !version(out, doc.get("version").and_then(Value::as_u64), newest::FIGCRAFT) {
        return Ok(());
    }
    let nodes = doc.get("nodes").and_then(Value::as_object);
    let node = |id: &Value| nodes.and_then(|n| n.get(&id.to_string()));
    let pages: Vec<&Value> = doc.get("pages").and_then(Value::as_array).map(|p| p.iter().filter_map(node).collect()).unwrap_or_default();
    out.facts.push(("Pages", count(pages.len(), "page", "pages")));
    if !pages.is_empty() {
        out.facts.push(("Page names", names(pages.iter().map(|p| p.get("name").and_then(Value::as_str).unwrap_or("")))));
    }
    // Frames are what sit directly on a page; components can sit anywhere.
    let frames = pages.iter().flat_map(|p| p.get("children").and_then(Value::as_array).into_iter().flatten()).filter_map(node).filter(|n| n.get("type").and_then(Value::as_str) == Some("frame")).count();
    if frames > 0 {
        out.facts.push(("Frames", frames.to_string()));
    }
    if let Some(n) = nodes {
        let comps = n.values().filter(|v| v.get("type").and_then(Value::as_str) == Some("component")).count();
        if comps > 0 {
            out.facts.push(("Components", comps.to_string()));
        }
    }
    let images = entries(&zip).into_iter().filter(|n| n.starts_with("images/") && !n.ends_with('/')).count();
    if images > 0 {
        out.facts.push(("Images inside", images.to_string()));
    }
    Ok(())
}

fn design(path: &Path, out: &mut Saved) -> Result<(), String> {
    let mut zip = zip_open(path).ok_or("Not a DesignCraft document")?;
    let mime = zip_bytes(&mut zip, "mimetype", 256).unwrap_or_default();
    if mime.trim_ascii() != b"application/vnd.designcraft+zip" {
        return Err("Not a DesignCraft document".into());
    }
    out.picture = zip_bytes(&mut zip, "preview.png", PIC_MAX).and_then(|b| decode(&b));
    let meta = zip_bytes(&mut zip, "meta.json", 1 << 20).and_then(|b| json(&b).ok()).unwrap_or(Value::Null);
    saved_by(out, &meta, "generator");
    if !version(out, Some(meta.get("version").and_then(Value::as_u64).unwrap_or(1)), newest::DESIGNCRAFT) {
        return Ok(());
    }
    let Some(doc) = zip_bytes(&mut zip, "document.json", JSON_MAX).and_then(|b| json(&b).ok()) else { return Ok(()) };
    if let Some(t) = doc.get("title").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
        out.facts.push(("Title", t.trim().to_string()));
    }
    let spreads = doc.get("spreads").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let pages: Vec<&Value> = spreads.iter().flat_map(|s| s.get("pages").and_then(Value::as_array).into_iter().flatten()).collect();
    out.facts.push(("Pages", count(pages.len(), "page", "pages")));
    let settings = doc.get("settings");
    let (w, h) = (
        settings.and_then(|s| s.get("pageWidth")).and_then(Value::as_f64).or_else(|| pages.first().and_then(|p| p.get("width")).and_then(Value::as_f64)),
        settings.and_then(|s| s.get("pageHeight")).and_then(Value::as_f64).or_else(|| pages.first().and_then(|p| p.get("height")).and_then(Value::as_f64)),
    );
    if let (Some(w), Some(h)) = (w, h) {
        let orient = if w > h { "landscape" } else if h > w { "portrait" } else { "square" };
        out.facts.push(("Page size", format!("{:.0} × {:.0} mm, {orient}", w / 72.0 * 25.4, h / 72.0 * 25.4)));
    }
    if settings.and_then(|s| s.get("facingPages")).and_then(Value::as_bool) == Some(true) {
        out.facts.push(("Facing pages", "Yes".into()));
    }
    let names_of = |n: &str| entries(&zip).into_iter().filter(|x| x.starts_with(n) && !x.ends_with('/')).count();
    let assets = names_of("assets/");
    if assets > 0 {
        out.facts.push(("Pictures and fonts inside", assets.to_string()));
    }
    Ok(())
}

fn texture(path: &Path, out: &mut Saved) -> Result<(), String> {
    let mut zip = zip_open(path).ok_or("Not a TextureCraft project")?;
    let p = json(&zip_bytes(&mut zip, "project.json", JSON_MAX).ok_or("Not a TextureCraft project")?)?;
    out.picture = zip_bytes(&mut zip, "thumbnail.png", PIC_MAX).and_then(|b| decode(&b));
    if !version(out, p.get("format_version").and_then(Value::as_u64), newest::TEXTURECRAFT) {
        return Ok(());
    }
    if let Some(m) = p.get("mesh").and_then(Value::as_str) {
        let mesh = match m.strip_prefix("builtin:") {
            Some(b) => format!("built in ({})", b.replace('_', " ")),
            None => {
                let kept = entries(&zip).into_iter().find(|n| n.starts_with("mesh.")).map(|n| n.trim_start_matches("mesh.").to_uppercase());
                match kept {
                    Some(ext) => format!("{m} (a copy of the {ext} file is inside)"),
                    None => m.to_string(),
                }
            }
        };
        out.facts.push(("Model", mesh));
    }
    let sets = p.get("texture_sets").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    out.facts.push(("Texture sets", count(sets.len(), "set", "sets")));
    for s in sets.iter().take(NAMES) {
        let name = s.get("name").and_then(Value::as_str).unwrap_or("(unnamed)");
        let res = s.get("resolution").and_then(Value::as_u64).map(|r| format!("{r} × {r}")).unwrap_or_default();
        let channels = s.get("channels").and_then(Value::as_array).map(|c| c.iter().filter_map(|c| c.get(0).and_then(Value::as_str)).map(channel).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        out.facts.push(("Texture set", format!("{name}: {res}; {channels}")));
    }
    let painted = entries(&zip).into_iter().filter(|n| n.starts_with("pixels/")).count();
    out.facts.push(("Painted maps", painted.to_string()));
    Ok(())
}

/// "BaseColor" → "base colour".
fn channel(c: &str) -> String {
    match c {
        "BaseColor" => "base colour".into(),
        "AmbientOcclusion" => "ambient occlusion".into(),
        _ => {
            let mut s = String::new();
            for (i, ch) in c.chars().enumerate() {
                if ch.is_uppercase() && i > 0 {
                    s.push(' ');
                }
                s.extend(ch.to_lowercase());
            }
            s
        }
    }
}

fn deck(path: &Path, out: &mut Saved) -> Result<(), String> {
    let mut zip = zip_open(path).ok_or("Not a DeckCraft presentation")?;
    let mime = zip_bytes(&mut zip, "mimetype", 256).unwrap_or_default();
    // Files from before DeckCraft's rename carry SlideCraft's type.
    if !mime.trim_ascii().starts_with(b"application/vnd.storyteller.") {
        return Err("Not a DeckCraft presentation".into());
    }
    let d = json(&zip_bytes(&mut zip, "presentation.json", JSON_MAX).ok_or("Not a DeckCraft presentation")?)?;
    saved_by(out, &d, "generator");
    if !version(out, d.get("version").and_then(Value::as_u64), newest::DECKCRAFT) {
        return Ok(());
    }
    let p = d.get("presentation").unwrap_or(&Value::Null);
    let slides = p.get("slides").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let hidden = slides.iter().filter(|s| s.get("hidden").and_then(Value::as_bool) == Some(true)).count();
    let mut s = count(slides.len(), "slide", "slides");
    if hidden > 0 {
        s.push_str(&format!(" ({hidden} hidden)"));
    }
    out.facts.push(("Slides", s));
    let size = p.get("slideSize");
    if let (Some(w), Some(h)) = (size.and_then(|s| s.get("width")).and_then(Value::as_f64), size.and_then(|s| s.get("height")).and_then(Value::as_f64)) {
        out.facts.push(("Slide shape", aspect(w, h)));
    }
    if let Some(t) = p.get("props").and_then(|p| p.get("title")).and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
        out.facts.push(("Title", t.trim().to_string()));
    }
    let media = entries(&zip).into_iter().filter(|n| n.starts_with("media/")).count();
    if media > 0 {
        out.facts.push(("Pictures and media inside", media.to_string()));
    }
    Ok(())
}

/// "16:9 (960 × 540)".
fn aspect(w: f64, h: f64) -> String {
    let r = w / h.max(1e-9);
    let named = [("16:9", 16.0 / 9.0), ("4:3", 4.0 / 3.0), ("16:10", 1.6), ("1:1", 1.0), ("9:16", 9.0 / 16.0)].into_iter().find(|(_, x)| (r - x).abs() < 0.01).map(|(n, _)| n);
    match named {
        Some(n) => format!("{n} ({w:.0} × {h:.0})"),
        None => format!("{w:.0} × {h:.0}"),
    }
}

fn ani(d: &Value, out: &mut Saved) -> Result<(), String> {
    if d.get("format").and_then(Value::as_str) != Some("anicraft") {
        return Err("Not an AniCraft project".into());
    }
    saved_by(out, d, "generator");
    if !version(out, d.get("version").and_then(Value::as_u64), newest::ANICRAFT) {
        return Ok(());
    }
    let doc = d.get("document").unwrap_or(&Value::Null);
    let stage = doc.get("stage");
    let rate = stage.and_then(|s| s.get("fps")).and_then(Value::as_f64).filter(|f| *f > 0.0);
    if let (Some(w), Some(h)) = (stage.and_then(|s| s.get("width")).and_then(Value::as_f64), stage.and_then(|s| s.get("height")).and_then(Value::as_f64)) {
        out.facts.push(("Stage", format!("{w:.0} × {h:.0}")));
    }
    if let Some(r) = rate {
        out.facts.push(("Frame rate", fps(r, 1.0)));
    }
    let scenes = doc.get("scenes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    out.facts.push(("Scenes", count(scenes.len(), "scene", "scenes")));
    for s in scenes.iter().take(NAMES) {
        let layers = s.get("timeline").and_then(|t| t.get("layers")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        // A layer's keyframes each start at a frame and last a number of frames.
        let frames = layers
            .iter()
            .flat_map(|l| l.get("keyframes").and_then(Value::as_array).into_iter().flatten())
            .filter_map(|k| Some(k.get("start")?.as_u64()? + k.get("duration")?.as_u64()?))
            .max()
            .unwrap_or(0);
        let name = s.get("name").and_then(Value::as_str).unwrap_or("(unnamed)");
        let mut line = format!("{name}: {}, {}", count(frames as usize, "frame", "frames"), count(layers.len(), "layer", "layers"));
        if let Some(r) = rate {
            line.push_str(&format!(", {}", seconds(frames as f64 / r)));
        }
        out.facts.push(("Scene", line));
    }
    let symbols = doc.get("library").and_then(|l| l.get("items").or(Some(l))).and_then(|v| v.as_array().map(Vec::len).or_else(|| v.as_object().map(|o| o.len())));
    if let Some(n) = symbols.filter(|n| *n > 0) {
        out.facts.push(("Library items", n.to_string()));
    }
    Ok(())
}

fn material(path: &Path, out: &mut Saved) -> Result<(), String> {
    let mut zip = zip_open(path).ok_or("Not a MaterialCraft package")?;
    let sbs = zip_bytes(&mut zip, "package.sbs", JSON_MAX).ok_or("Not a MaterialCraft package")?;
    let text = String::from_utf8_lossy(&sbs);
    // The graph is only read for names; it is never evaluated.
    let opts = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 4_000_000, ..Default::default() };
    let x = roxmltree::Document::parse_with_options(&text, opts).map_err(|_| "The package couldn't be read (it may be damaged).")?;
    let id = |n: roxmltree::Node| n.children().find(|c| c.has_tag_name("identifier")).and_then(|i| i.attribute("v")).unwrap_or("").to_string();
    let graphs: Vec<roxmltree::Node> = x.descendants().filter(|n| n.has_tag_name("graph")).collect();
    out.facts.push(("Graphs", count(graphs.len(), "graph", "graphs")));
    for g in graphs.iter().take(NAMES) {
        let nodes = g.descendants().filter(|n| n.has_tag_name("compNode")).count();
        let params = g.descendants().filter(|n| n.has_tag_name("paraminput")).count();
        let outputs: Vec<String> = g.descendants().filter(|n| n.has_tag_name("graphoutput")).map(id).collect();
        let mut line = format!("{}: {}, {}", id(*g), count(nodes, "node", "nodes"), count(params, "parameter", "parameters"));
        if !outputs.is_empty() {
            line.push_str(&format!("; outputs {}", names(outputs.iter().map(String::as_str))));
        }
        out.facts.push(("Graph", line));
    }
    let resources = entries(&zip).into_iter().filter(|n| *n != "package.sbs" && !n.ends_with('/')).count();
    if resources > 0 {
        out.facts.push(("Resources inside", resources.to_string()));
    }
    Ok(())
}

/// FilmCraft and Premiere count time in ticks of 1/254,016,000,000 second.
const FILM_TICKS: f64 = 254_016_000_000.0;

fn film(d: &Value, out: &mut Saved) -> Result<(), String> {
    // Schema 1 is the bare project with "version": 1; later ones are an envelope.
    let (schema, project) = match d.get("format").and_then(Value::as_str) {
        Some("filmcraft.project") => (d.get("schema_version").and_then(Value::as_u64), d.get("project")),
        None if d.get("schema_version").is_some() && d.get("project").is_some() => (d.get("schema_version").and_then(Value::as_u64), d.get("project")),
        None if d.get("version").and_then(Value::as_u64) == Some(1) && d.get("items").is_some() => (Some(1), Some(d)),
        _ => return Err("Not a FilmCraft project".into()),
    };
    saved_by(out, d, "generator");
    if !version(out, schema, newest::FILMCRAFT) {
        return Ok(());
    }
    let project = project.unwrap_or(&Value::Null);
    if let Some(n) = project.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()) {
        out.facts.push(("Project", n.trim().to_string()));
    }
    let items: Vec<&Value> = project.get("items").and_then(Value::as_object).map(|o| o.values().collect()).unwrap_or_default();
    let kind = |i: &&Value, k: &str| i.get("kind").and_then(|v| v.get(k)).is_some();
    let media = items.iter().filter(|i| kind(i, "Media")).count();
    let sequences: Vec<&&Value> = items.iter().filter(|i| kind(i, "Sequence")).collect();
    out.facts.push(("Media", count(media, "clip", "clips")));
    out.facts.push(("Sequences", count(sequences.len(), "sequence", "sequences")));
    for s in sequences.iter().take(NAMES) {
        let seq = &s["kind"]["Sequence"];
        let set = &seq["settings"];
        let rate = fps(set["frame_rate"]["num"].as_f64().unwrap_or(0.0), set["frame_rate"]["den"].as_f64().unwrap_or(0.0));
        let tracks = |k: &str| seq.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let end = ["video_tracks", "audio_tracks"]
            .iter()
            .flat_map(|k| tracks(k).iter())
            .flat_map(|t| t.get("items").and_then(Value::as_array).into_iter().flatten())
            .filter_map(|c| Some(c.get("start")?.as_f64()? + c.get("duration")?.as_f64()?))
            .fold(0.0f64, f64::max);
        let name = s.get("name").and_then(Value::as_str).unwrap_or("(unnamed)");
        out.facts.push((
            "Sequence",
            format!(
                "{name}: {} × {}, {rate}, {}; {} and {}",
                set["width"].as_u64().unwrap_or(0),
                set["height"].as_u64().unwrap_or(0),
                seconds(end / FILM_TICKS),
                count(tracks("video_tracks").len(), "video track", "video tracks"),
                count(tracks("audio_tracks").len(), "audio track", "audio tracks"),
            ),
        ));
    }
    if sequences.len() > NAMES {
        out.facts.push(("Sequence", format!("and {} more", sequences.len() - NAMES)));
    }
    Ok(())
}

fn sound(d: &Value, out: &mut Saved) -> Result<(), String> {
    let Some(v) = d.get("format_version").and_then(Value::as_u64) else { return Err("Not a SoundCraft session".into()) };
    if d.get("tracks").and_then(Value::as_array).is_none() || d.get("sample_rate").is_none() {
        return Err("Not a SoundCraft session".into());
    }
    if !version(out, Some(v), newest::SOUNDCRAFT) {
        return Ok(());
    }
    if let Some(n) = d.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()) {
        out.facts.push(("Session", n.trim().to_string()));
    }
    let rate = d.get("sample_rate").and_then(Value::as_f64).filter(|r| *r > 0.0);
    if let Some(r) = rate {
        let depth = match d.get("bit_depth").and_then(Value::as_str) {
            Some("Int16") => ", 16-bit",
            Some("Int24") => ", 24-bit",
            Some("Float32") => ", 32-bit float",
            _ => "",
        };
        out.facts.push(("Sample rate", format!("{:.1} kHz{depth}", r / 1000.0).replace(".0 kHz", " kHz")));
    }
    let tempo = d.get("tempo");
    if let Some(bpm) = tempo.and_then(|t| t.get("tempos")).and_then(Value::as_array).and_then(|t| t.first()).and_then(|t| t.get("bpm")).and_then(Value::as_f64) {
        let changes = tempo.and_then(|t| t.get("tempos")).and_then(Value::as_array).map_or(0, Vec::len);
        let mut s = format!("{} BPM", (bpm * 100.0).round() / 100.0);
        if changes > 1 {
            s.push_str(&format!(" ({} tempo changes)", changes - 1));
        }
        out.facts.push(("Tempo", s));
    }
    if let Some(m) = tempo.and_then(|t| t.get("meters")).and_then(Value::as_array).and_then(|m| m.first()) {
        if let (Some(n), Some(dn)) = (m.get("numerator").and_then(Value::as_u64), m.get("denominator").and_then(Value::as_u64)) {
            out.facts.push(("Time signature", format!("{n}/{dn}")));
        }
    }
    let tracks = d["tracks"].as_array().map(Vec::as_slice).unwrap_or_default();
    let of_kind = |k: &str| tracks.iter().filter(|t| t.get("kind").and_then(Value::as_str) == Some(k)).count();
    let mut kinds = Vec::new();
    for (k, one, many) in [("Audio", "audio", "audio"), ("Midi", "MIDI", "MIDI"), ("Instrument", "instrument", "instrument"), ("Aux", "aux", "aux"), ("Video", "video", "video")] {
        let n = of_kind(k);
        if n > 0 {
            kinds.push(format!("{n} {}", if n == 1 { one } else { many }));
        }
    }
    let mut line = count(tracks.len(), "track", "tracks");
    if !kinds.is_empty() {
        line.push_str(&format!(" ({})", kinds.join(", ")));
    }
    out.facts.push(("Tracks", line));
    if !tracks.is_empty() {
        out.facts.push(("Track names", names(tracks.iter().map(|t| t.get("name").and_then(Value::as_str).unwrap_or("")))));
    }
    // The session's length: where its last clip on each track's playlist in use ends.
    let end = tracks
        .iter()
        .filter_map(|t| {
            let lists = t.get("playlists")?.as_array()?;
            let i = t.get("active_playlist").and_then(Value::as_u64).unwrap_or(0) as usize;
            lists.get(i)?.get("clips")?.as_array()
        })
        .flatten()
        .filter_map(|c| Some(c.get("start")?.as_f64()? + c.get("length")?.as_f64()?))
        .fold(0.0f64, f64::max);
    if let Some(r) = rate.filter(|_| end > 0.0) {
        out.facts.push(("Length", seconds(end / r)));
    }
    Ok(())
}

/// Checks the files a project uses (see `projrefs`), when it names any, and says how many in the
/// details. A project from a newer version than Filescape knows isn't read for them.
fn uses(out: &mut Saved, what: &'static str, named: Vec<String>, base: Option<&Path>) {
    if named.is_empty() || out.facts.iter().any(|(k, v)| *k == "Format" && v.contains("newer")) {
        return;
    }
    let refs = crate::projrefs::check(what, named, base);
    out.facts.push((what, refs.fact()));
    out.refs = Some(refs);
}

/// FilmCraft: each media item's file and its proxy (`items.*.kind.Media.{media,proxy}.File.path`).
fn film_files(d: &Value) -> Vec<String> {
    let project = if d.get("project").is_some() { &d["project"] } else { d };
    let items = project.get("items").and_then(Value::as_object).map(|o| o.values().collect::<Vec<_>>()).unwrap_or_default();
    items
        .iter()
        .filter_map(|i| i.get("kind")?.get("Media"))
        .flat_map(|m| ["media", "proxy"].map(|k| m.get(k).and_then(|r| r.get("File")).and_then(|f| f.get("path").unwrap_or(f).as_str())))
        .flatten()
        .map(str::to_string)
        .collect()
}

/// SoundCraft: the sound files (relative to the session's folder, or absolute when referenced)
/// that have been saved, and the movies video tracks use.
fn sound_files(d: &Value, audio: bool) -> Vec<String> {
    let list = |k: &str| d.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default().to_vec();
    let mut out = Vec::new();
    if audio {
        out.extend(
            list("sources")
                .iter()
                .filter(|s| s.get("unsaved").and_then(Value::as_bool) != Some(true))
                .filter_map(|s| s.get("path").and_then(Value::as_str).map(str::to_string)),
        );
    }
    out.extend(list("videos").iter().filter_map(|v| v.get("path").and_then(Value::as_str).map(str::to_string)));
    out
}

/// EffectCraft: footage items' files and their proxies (`items.*.kind` tagged `"type": "Footage"`, its `path`;
/// `items.*.proxy.footage.path`).
fn effect_files(d: &Value) -> Vec<String> {
    let items = d.get("items").or_else(|| d.get("project").and_then(|p| p.get("items")));
    let items = items.and_then(Value::as_object).map(|o| o.values().collect::<Vec<_>>()).unwrap_or_default();
    let mut out = Vec::new();
    for i in items {
        let kind = i.get("kind");
        if let Some(p) = kind.filter(|k| k.get("type").and_then(Value::as_str) == Some("Footage")).and_then(|f| f.get("path")).and_then(Value::as_str) {
            out.push(p.to_string());
        }
        if let Some(p) = i.get("proxy").and_then(|x| x.get("footage")).and_then(|f| f.get("path")).and_then(Value::as_str) {
            out.push(p.to_string());
        }
    }
    out
}

/// The names of the entries in a ZIP (any that can't be read are left out).
fn entries<R: std::io::Read + std::io::Seek>(zip: &zip::ZipArchive<R>) -> Vec<String> {
    zip.file_names().filter_map(Result::ok).map(|n| n.into_owned()).collect()
}

#[cfg(test)]
mod tests {
    //! Fixtures follow the shapes in the apps' own tests and examples (MIT or Apache-2.0):
    //! craft-libs `crates/doc/tests/common`, FilmCraft `crates/format/tests/fixtures`, AniCraft
    //! `crates/engine/tests/fixtures`, and documents written by `craft-scraft-format`.
    use super::*;
    use std::io::Write;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 40, 255])).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (n, d) in entries {
            z.start_file(*n, o).unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    fn file(name: &str, bytes: &[u8]) -> (tempdir::Dir, std::path::PathBuf) {
        let d = tempdir::Dir::new();
        let p = d.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        (d, p)
    }

    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let p = std::env::temp_dir().join(format!("filescape-craft-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn fact<'a>(s: &'a Saved, k: &str) -> Option<&'a str> {
        s.facts.iter().find(|f| f.0 == k).map(|f| f.1.as_str())
    }

    fn pcraft_layer(name: &str, visible: bool, content: &str) -> String {
        format!(
            r#"{{"id":1,"name":"{name}","visible":{visible},"locks":{{}},"blend":"Normal","opacity":1.0,"fill_opacity":1.0,"clipped":false,"mask":null,
            "effects":{{"enabled":true,"items":[],"psd_raw":null}},"label":"None","content":{content},"psd_blocks":[],"psd_id":null,"fill_cache":null}}"#
        )
    }

    const RASTER: &str = r#"{"kind":"raster","surface":{"format":{"mode":"Rgb","sample":"U8","alpha":true},"default":"00000000","tiles":[]}}"#;

    fn pcraft(version: u32) -> Vec<u8> {
        let layers = format!("[{},{}]", pcraft_layer("Background", true, RASTER), pcraft_layer("Group", false, &format!(r#"{{"kind":"group","expanded":true,"children":[{}]}}"#, pcraft_layer("Inner", true, RASTER))));
        let manifest = format!(
            r#"{{"format_version":{version},"generator":"PhotoCraft 0.23.1",
            "document":{{"id":1,"name":"Untitled","size":{{"width":640,"height":480}},"resolution_dpi":300.0,"mode":"Rgb","depth":"U16",
              "icc_profile":null,"layers":{layers},"channels":[],"guides":{{}},"selection":null,"metadata":{{}},"global_light":{{}}}},"thumbnail":"thumb.png","composite":"composite/preview.png"}}"#
        );
        zip(&[("tiles/0123.zst", b"tile"), ("thumb.png", &png(32, 24)), ("composite/preview.png", &png(64, 48)), ("manifest.json", manifest.as_bytes())])
    }

    #[test]
    fn photocraft() {
        let (_d, p) = file("Poster.pcraft", &pcraft(1));
        let s = read(&p).unwrap();
        assert_eq!(s.picture.as_ref().map(|i| (i.width(), i.height())), Some((64, 48)));
        assert_eq!(fact(&s, "App"), Some("PhotoCraft"));
        assert_eq!(fact(&s, "Size"), Some("640 × 480 pixels"));
        assert_eq!(fact(&s, "Colour"), Some("RGB, 16 bits a channel"));
        assert_eq!(fact(&s, "Layers"), Some("3 layers (1 hidden)"));
        assert_eq!(fact(&s, "Top layers"), Some("Group, Background"));
        // A newer format: the picture and the version, nothing guessed.
        let (_d, p) = file("Newer.pcraft", &pcraft(99));
        let s = read(&p).unwrap();
        assert!(fact(&s, "Format").unwrap().contains("newer PhotoCraft"));
        assert!(fact(&s, "Size").is_none());
        // The same document saved as a bundle (a folder): read from the folder's own files.
        let zipped = pcraft(1);
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(zipped)).unwrap();
        let (_d, p) = file("placeholder", b"");
        let bundle = p.parent().unwrap().join("Bundle.pcraft");
        for i in 0..z.len() {
            let mut e = z.by_index(i).unwrap();
            if e.is_dir() {
                continue;
            }
            let out = bundle.join(e.name().unwrap().to_string());
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            std::io::copy(&mut e, &mut std::fs::File::create(&out).unwrap()).unwrap();
        }
        let s = read(&bundle).unwrap();
        assert_eq!(s.picture.as_ref().map(|i| (i.width(), i.height())), Some((64, 48)));
        assert_eq!(fact(&s, "Layers"), Some("3 layers (1 hidden)"));
        assert!(crate::packages::is_package(&bundle, "Bundle.pcraft"));
    }

    fn vectorcraft(members: &str) -> String {
        let document = r#"{"version":1,"title":"","units":"Points","color_mode":"Rgb",
            "artboards":[{"id":1,"name":"Front","rect":{"x0":0.0,"y0":0.0,"x1":612.0,"y1":792.0},"show_center_mark":false,"show_cross_hairs":false}],
            "layers":[{"id":1,"name":"Art","kind":{"type":"layer","color":{"Preset":0},"template":false,"printable":true,"children":[]}}],
            "next_id":5}"#;
        let sep = if members.is_empty() { "" } else { "," };
        format!(r#"{{"format":"vectorcraft","version":4,"generator":"VectorCraft 0.11.0"{sep}{members},"document":{document},"images":{{}}}}"#)
    }

    fn b64(b: &[u8]) -> String {
        data_encoding::BASE64.encode(b)
    }

    #[test]
    fn vectorcraft_preview_and_artboards() {
        let json = vectorcraft(&format!(r#""preview":{{"mime":"image/png","data":"{}"}}"#, b64(&png(40, 30))));
        let (_d, p) = file("Logo.vectorcraft", json.as_bytes());
        let s = read(&p).unwrap();
        assert_eq!(s.picture.as_ref().map(|i| i.width()), Some(40));
        assert_eq!(fact(&s, "Saved by"), Some("VectorCraft 0.11.0"));
        assert_eq!(fact(&s, "Artboard sizes"), Some("Front (612 × 792 pt)"));
        // Gzip-compressed (Use Compression), with no picture.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(vectorcraft("").as_bytes()).unwrap();
        let (_d, p) = file("Plain.vectorcraft", &gz.finish().unwrap());
        let s = read(&p).unwrap();
        assert!(s.picture.is_none() && s.note.as_deref().unwrap().contains("PDF-Compatible"));
        assert_eq!(fact(&s, "Artboards"), Some("1 artboard"));
    }

    #[test]
    fn vectorcraft_pdf_is_drawn() {
        let pdf = crate::pdfview::tests::tiny();
        let json = vectorcraft(&format!(r#""pdf":{{"mime":"application/pdf","data":"{}"}}"#, b64(&pdf)));
        let (_d, p) = file("Sheet.vectorcraft", json.as_bytes());
        let s = read(&p).unwrap();
        let pic = s.picture.expect("the PDF drawn");
        assert!(pic.width() > 100);
    }

    const ECPROJ: &str = r#"{"savedBy":"0.12.0","schema":2,"settings":{},"items":{
        "2":{"id":2,"name":"clip.mov","label":0,"kind":{"type":"Footage","path":"clip.mov","kind":"Video","width":640,"height":360,"pixel_aspect":1.0,"frame_rate":{"num":30,"den":1},"duration":2540160000000}},
        "10":{"id":10,"name":"Main","label":0,"kind":{"type":"Comp","width":1920,"height":1080,"pixel_aspect":1.0,"frame_rate":{"num":24000,"den":1001},"duration":1270080000000,"layers":[
            {"id":1,"name":"Title","source":{"type":"Text"},"switches":{"video":true,"audio":false},"blend_mode":"Add"}]}}},"next_id":11}"#;

    #[test]
    fn effectcraft_and_template() {
        let (_d, p) = file("Promo.ecproj", ECPROJ.as_bytes());
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Composition"), Some("Main: 1920 × 1080, 23.976 fps, 0:05, 1 layer"));
        let r = s.refs.as_ref().expect("footage checked");
        assert_eq!(r.items.iter().map(|i| i.named.as_str()).collect::<Vec<_>>(), ["clip.mov"]);
        assert_eq!(r.items[0].path, p.parent().unwrap().join("clip.mov"));
        let (_d, p) = file("Lower third.ectemplate", &zip(&[("manifest.json", b"{}"), ("project.ecproj", ECPROJ.as_bytes()), ("poster.png", &png(48, 27))]));
        let s = read(&p).unwrap();
        assert_eq!(s.picture.as_ref().map(|i| i.width()), Some(48));
        assert_eq!(fact(&s, "Compositions"), Some("1 composition"));
        // Any other JSON isn't taken for a project.
        let (_d, p) = file("x.ecproj", br#"{"name":"x"}"#);
        assert!(read(&p).is_err());
    }

    #[test]
    fn figcraft() {
        let doc = r#"{"format":"figcraft","version":1,"imageIndex":[],"name":"App","pages":[1],"nodes":{
            "1":{"id":1,"name":"Screens","type":"page","children":[2,3]},
            "2":{"id":2,"name":"Home","type":"frame","parent":1},
            "3":{"id":3,"name":"Button","type":"component","parent":1}}}"#;
        let (_d, p) = file("App.figcraft", &zip(&[("document.json", doc.as_bytes()), ("thumbnail.png", &png(50, 40)), ("images/abc", b"x")]));
        let s = read(&p).unwrap();
        assert_eq!(s.picture.as_ref().map(|i| i.height()), Some(40));
        assert_eq!(fact(&s, "Page names"), Some("Screens"));
        assert_eq!(fact(&s, "Frames"), Some("1"));
        assert_eq!(fact(&s, "Components"), Some("1"));
        assert_eq!(fact(&s, "Images inside"), Some("1"));
    }

    #[test]
    fn designcraft() {
        let doc = r#"{"title":"Spring issue","settings":{"pageWidth":595.0,"pageHeight":842.0,"facingPages":true},"spreads":[{"id":1,"pages":[{"id":1,"width":595.0,"height":842.0}]},{"id":2,"pages":[{"id":2},{"id":3}]}],"parents":[],"layers":[]}"#;
        let bytes = zip(&[
            ("mimetype", b"application/vnd.designcraft+zip"),
            ("meta.json", br#"{"format":"designcraft","version":5,"generator":"DesignCraft 0.9.0"}"#),
            ("document.json", doc.as_bytes()),
            ("preview.png", &png(42, 60)),
            ("assets/1/photo.jpg", b"x"),
        ]);
        let (_d, p) = file("Magazine.designcraft", &bytes);
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Pages"), Some("3 pages"));
        assert_eq!(fact(&s, "Page size"), Some("210 × 297 mm, portrait"));
        assert_eq!(fact(&s, "Title"), Some("Spring issue"));
        assert_eq!(fact(&s, "Pictures and fonts inside"), Some("1"));
        let (_d, p) = file("Fake.designcraft", &zip(&[("mimetype", b"application/zip")]));
        assert!(read(&p).is_err());
    }

    #[test]
    fn texturecraft() {
        let project = r#"{"format_version":3,"name":"Ball","mesh":"builtin:material_ball","texture_sets":[{"name":"Ball","resolution":1024,"channels":[["BaseColor","Srgb8"],["Roughness","L8"],["AmbientOcclusion","L8"]]}],"layers":{}}"#;
        let (_d, p) = file("Ball.tcraft", &zip(&[("project.json", project.as_bytes()), ("pixels/layer_1_0.bin", b"TCTX"), ("thumbnail.png", &png(64, 64))]));
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Model"), Some("built in (material ball)"));
        assert_eq!(fact(&s, "Texture set"), Some("Ball: 1024 × 1024; base colour, roughness, ambient occlusion"));
        assert_eq!(fact(&s, "Painted maps"), Some("1"));
    }

    #[test]
    fn deckcraft() {
        let pres = r#"{"format":"deckcraft","version":1,"generator":"DeckCraft 0.5.0","presentation":{"slideSize":{"width":960.0,"height":540.0},"slides":[{"id":1},{"id":2,"hidden":true}],"props":{"title":"Kickoff"}}}"#;
        let (_d, p) = file("Kickoff.deckcraft", &zip(&[("mimetype", b"application/vnd.storyteller.deckcraft+zip"), ("presentation.json", pres.as_bytes()), ("media/1", b"x")]));
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Slides"), Some("2 slides (1 hidden)"));
        assert_eq!(fact(&s, "Slide shape"), Some("16:9 (960 × 540)"));
        assert!(s.picture.is_none() && s.note.is_some());
    }

    #[test]
    fn anicraft() {
        let doc = r#"{"format":"anicraft","version":3,"generator":"AniCraft 0.3.0","paths":[],"document":{"stage":{"width":400,"height":300,"fps":24.0},
            "scenes":[{"name":"Scene 1","timeline":{"layers":[{"id":1,"name":"Ball","keyframes":[{"start":0,"duration":10},{"start":10,"duration":38}]}]}}],"library":{"items":[]},"next_id":2}}"#;
        let (_d, p) = file("Bounce.anicraft", doc.as_bytes());
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Stage"), Some("400 × 300"));
        assert_eq!(fact(&s, "Scene"), Some("Scene 1: 48 frames, 1 layer, 0:02"));
        let (_d, p) = file("New.anicraft", doc.replace(r#""version":3"#, r#""version":4"#).as_bytes());
        assert!(fact(&read(&p).unwrap(), "Format").unwrap().contains("newer AniCraft"));
    }

    #[test]
    fn materialcraft() {
        let sbs = r#"<?xml version="1.0"?><package><identifier v="Bricks"/><content><graph><identifier v="bricks"/>
            <paraminputs><paraminput><identifier v="scale"/></paraminput></paraminputs>
            <graphOutputs><graphoutput><identifier v="basecolor"/></graphoutput><graphoutput><identifier v="normal"/></graphoutput></graphOutputs>
            <compNodes><compNode/><compNode/></compNodes></graph></content></package>"#;
        let (_d, p) = file("Bricks.mcraft", &zip(&[("package.sbs", sbs.as_bytes()), ("resources/noise.png", b"x")]));
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Graph"), Some("bricks: 2 nodes, 1 parameter; outputs basecolor, normal"));
        assert_eq!(fact(&s, "Resources inside"), Some("1"));
    }

    #[test]
    fn filmcraft() {
        let d = r#"{"format":"filmcraft.project","schema_version":17,"generator":"FilmCraft 0.12.0","project":{"name":"Trailer","items":{
            "5":{"id":5,"name":"Ocean.mp4","kind":{"Media":{"media":{"File":{"path":"Footage/Ocean.mp4"}},"proxy":{"File":{"path":"/nowhere/Ocean proxy.mov"}}}}},
            "21":{"id":21,"name":"Assembly","kind":{"Sequence":{"settings":{"width":1920,"height":1080,"frame_rate":{"num":24000,"den":1001}},
              "video_tracks":[{"id":15,"items":[{"id":29,"item":5,"start":0,"duration":1270080000000}]}],
              "audio_tracks":[{"id":18,"items":[{"id":30,"item":5,"start":254016000000,"duration":1270080000000}]},{"id":19,"items":[]}]}}}}}}"#;
        let (_d, p) = file("Trailer.fcproj", d.as_bytes());
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Format"), Some("version 17"));
        assert_eq!(fact(&s, "Media"), Some("1 clip"));
        assert_eq!(fact(&s, "Sequence"), Some("Assembly: 1920 × 1080, 23.976 fps, 0:06; 1 video track and 2 audio tracks"));
        // Its media: relative to the project's folder, checked without being opened.
        std::fs::create_dir_all(p.parent().unwrap().join("Footage")).unwrap();
        std::fs::write(p.parent().unwrap().join("Footage/Ocean.mp4"), [0u8; 2048]).unwrap();
        let s = read(&p).unwrap();
        let r = s.refs.as_ref().unwrap();
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.count(crate::projrefs::State::Missing), 1);
        assert_eq!(r.total, 2048);
        assert_eq!(fact(&s, "Media"), Some("1 clip"));
        assert!(fact(&s, "Media files").unwrap().starts_with("2 (1 not found)"));
        let (_d, p) = file("Next.fcproj", d.replace(":17,", ":18,").as_bytes());
        assert!(fact(&read(&p).unwrap(), "Format").unwrap().contains("newer FilmCraft"));
    }

    const SCRAFT: &str = r#"{"format_version":1,"name":"Demo song","sample_rate":48000,"bit_depth":"Int24","frame_rate":"Fps30","timecode_start":0,
        "tempo":{"tempos":[{"tick":0,"bpm":120.0}],"meters":[{"tick":0,"numerator":3,"denominator":4}]},"key_signatures":[],
        "tracks":[{"id":1,"name":"Vocals","kind":"Audio","playlists":[{"name":"Vocals","clips":[{"id":1,"name":"take","start":48000,"length":480000}]}],"active_playlist":0},
                  {"id":2,"name":"Keys","kind":"Midi","playlists":[{"name":"Keys","clips":[]}],"active_playlist":0}],
        "sources":[{"id":1,"name":"take.wav","path":"Audio Files/take.wav","channels":2,"frames":480000,"sample_rate":48000,"format":"Wav"}]}"#;

    #[test]
    fn soundcraft_session_and_bundle() {
        let (_d, p) = file("Demo.scraft", SCRAFT.as_bytes());
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Sample rate"), Some("48 kHz, 24-bit"));
        assert_eq!(fact(&s, "Tempo"), Some("120 BPM"));
        assert_eq!(fact(&s, "Time signature"), Some("3/4"));
        assert_eq!(fact(&s, "Tracks"), Some("2 tracks (1 audio, 1 MIDI)"));
        assert_eq!(fact(&s, "Length"), Some("0:11"));
        let r = s.refs.as_ref().unwrap();
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].path, p.parent().unwrap().join("Audio Files/take.wav"));
        assert_eq!(r.count(crate::projrefs::State::Missing), 1);
        let (_d, p) = file("Demo.scraftz", &zip(&[("Demo.scraft", SCRAFT.as_bytes()), ("Audio Files/take.wav", b"RIFF")]));
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Sound files inside"), Some("1"));
    }

    #[test]
    fn damaged_files_are_refused_not_panicked_on() {
        for (name, bytes) in [
            ("a.pcraft", b"PK\x03\x04 truncated".to_vec()),
            ("a.vectorcraft", b"{\"format\":\"vectorcraft\",".to_vec()),
            ("a.fcproj", vec![b'['; 100_000]),
            ("a.scraft", b"\x1f\x8b\x08".to_vec()),
            ("a.figcraft", zip(&[("document.json", b"{\"format\":\"figma\"}")])),
            ("a.mcraft", zip(&[("package.sbs", b"<!DOCTYPE x [<!ENTITY a \"b\">]><x>&a;</x>")])),
        ] {
            let (_d, p) = file(name, &bytes);
            assert!(read(&p).is_err(), "{name}");
        }
    }
}
