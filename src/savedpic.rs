//! Files from creative apps whose own format Filescape can't read: Apple iWork, Sketch,
//! CorelDRAW, Affinity, SketchUp, Blender, Cinema 4D, 3ds Max, Maya and Houdini. Most of them
//! save a small picture of the document inside the file; that picture is shown, with whatever
//! the file says about itself (the app and version that saved it, pages, artboards and so on).
//!
//! Only that saved picture is decoded. Nothing in the document is run.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How far into a file a saved picture is looked for.
const SCAN_MAX: u64 = 24 << 20;
/// Pictures larger than this aren't decoded.
const PIC_MAX: usize = 16 << 20;

/// The app a file belongs to.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum App {
    Pages,
    Numbers,
    Keynote,
    Sketch,
    CorelDraw,
    Affinity,
    SketchUp,
    Blender,
    Cinema4d,
    Max,
    Maya,
    Houdini,
}

impl App {
    pub fn name(self) -> &'static str {
        match self {
            App::Pages => "Pages",
            App::Numbers => "Numbers",
            App::Keynote => "Keynote",
            App::Sketch => "Sketch",
            App::CorelDraw => "CorelDRAW",
            App::Affinity => "Affinity",
            App::SketchUp => "SketchUp",
            App::Blender => "Blender",
            App::Cinema4d => "Cinema 4D",
            App::Max => "3ds Max",
            App::Maya => "Maya",
            App::Houdini => "Houdini",
        }
    }
}

pub fn app_of(name: &str) -> Option<App> {
    Some(match crate::scan::ext_of(name).to_ascii_lowercase().as_str() {
        "pages" => App::Pages,
        "numbers" => App::Numbers,
        "key" => App::Keynote,
        "sketch" => App::Sketch,
        "cdr" => App::CorelDraw,
        "afdesign" | "afphoto" | "afpub" | "aftemplate" => App::Affinity,
        "skp" => App::SketchUp,
        "blend" => App::Blender,
        "c4d" => App::Cinema4d,
        "max" => App::Max,
        "ma" | "mb" => App::Maya,
        "hip" | "hipnc" | "hiplc" => App::Houdini,
        _ => return None,
    })
}

/// What's found: the saved picture, if there is one, and the details.
pub struct Saved {
    pub picture: Option<image::DynamicImage>,
    pub facts: Vec<(&'static str, String)>,
    /// Said instead of the usual line when there's no picture (why there isn't one).
    pub note: Option<String>,
    /// The files a project uses, checked (Craft Apps projects).
    pub refs: Option<crate::projrefs::Refs>,
}

pub fn read(path: &Path) -> Result<Saved, String> {
    if path.file_name().is_some_and(|n| crate::craftfile::of(&n.to_string_lossy()).is_some()) {
        return crate::craftfile::read(path);
    }
    if path.file_name().is_some_and(|n| crate::paintfile::of(&n.to_string_lossy()).is_some()) {
        return crate::paintfile::read(path);
    }
    if path.file_name().is_some_and(|n| crate::aepfile::is_project(&n.to_string_lossy())) {
        return crate::aepfile::read(path);
    }
    let app = path.file_name().and_then(|n| app_of(&n.to_string_lossy())).ok_or("Not a file Filescape knows")?;
    if path.is_dir() {
        return Err(format!("A {} package (a folder); open it to see inside", app.name()));
    }
    if matches!(app, App::Pages | App::Numbers | App::Keynote | App::Sketch) && zip_open(path).is_none() {
        return Err(format!("Not a {} document", app.name()));
    }
    let mut facts = vec![("App", app.name().to_string())];
    let picture = match app {
        App::Pages | App::Numbers | App::Keynote => iwork(path, &mut facts),
        App::Sketch => sketch(path, &mut facts),
        App::CorelDraw => corel(path, &mut facts),
        App::Blender => blender(path, &mut facts)?,
        App::Maya => {
            maya(path, &mut facts);
            None
        }
        App::Houdini => {
            houdini(path, &mut facts);
            None
        }
        App::SketchUp => {
            sketchup_version(path, &mut facts);
            embedded(path)
        }
        App::Affinity | App::Cinema4d | App::Max => embedded(path),
    };
    if let Some(p) = &picture {
        facts.push(("Saved picture", format!("{} × {}", p.width(), p.height())));
    }
    Ok(Saved { picture, facts, note: None, refs: None })
}

pub(crate) fn zip_open(path: &Path) -> Option<zip::ZipArchive<std::fs::File>> {
    zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()
}

pub(crate) fn zip_bytes(zip: &mut zip::ZipArchive<std::fs::File>, name: &str, max: usize) -> Option<Vec<u8>> {
    let f = zip.by_name(name).ok()?;
    if f.size() as usize > max {
        return None;
    }
    let mut b = Vec::new();
    f.take(max as u64).read_to_end(&mut b).ok()?;
    Some(b)
}

pub(crate) fn decode(b: &[u8]) -> Option<image::DynamicImage> {
    image::load_from_memory(b).ok().filter(|i| i.width() >= 16 && i.height() >= 16)
}

/// iWork (2013 and later, and '09): a zip with the document's first page saved as a picture.
fn iwork(path: &Path, facts: &mut Vec<(&'static str, String)>) -> Option<image::DynamicImage> {
    let mut zip = zip_open(path)?;
    // The app versions that have saved it, oldest first.
    if let Some(b) = zip_bytes(&mut zip, "Metadata/BuildVersionHistory.plist", 1 << 20) {
        let text = String::from_utf8_lossy(&b);
        let versions: Vec<&str> = text.split("<string>").skip(1).filter_map(|s| s.split("</string>").next()).collect();
        if let Some(last) = versions.last() {
            facts.push(("Last saved by", last.to_string()));
        }
        if let Some(t) = versions.iter().find(|v| v.starts_with("Template: ")) {
            facts.push(("Template", t.trim_start_matches("Template: ").to_string()));
        }
    }
    if zip.by_name("Index/Document.iwa").is_ok() || zip.by_name("Index.zip").is_ok() {
        facts.push(("Format", "iWork 2013 or later".into()));
    } else if zip.by_name("index.xml").is_ok() || zip.by_name("index.apxl").is_ok() {
        facts.push(("Format", "iWork '09".into()));
    }
    ["preview.jpg", "QuickLook/Thumbnail.jpg", "preview-web.jpg", "preview-micro.jpg"].iter().find_map(|n| zip_bytes(&mut zip, n, PIC_MAX).and_then(|b| decode(&b)))
}

/// Sketch: a zip with `previews/preview.png` and `meta.json`.
fn sketch(path: &Path, facts: &mut Vec<(&'static str, String)>) -> Option<image::DynamicImage> {
    let mut zip = zip_open(path)?;
    if let Some(meta) = zip_bytes(&mut zip, "meta.json", 8 << 20).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
        if let Some(v) = meta.get("appVersion").and_then(|v| v.as_str()) {
            facts.push(("Saved with", format!("Sketch {v}")));
        }
        if let Some(pages) = meta.get("pagesAndArtboards").and_then(|p| p.as_object()) {
            let boards: usize = pages.values().filter_map(|p| p.get("artboards").and_then(|a| a.as_object()).map(|a| a.len())).sum();
            facts.push(("Pages", pages.len().to_string()));
            facts.push(("Artboards", boards.to_string()));
        }
        if let Some(fonts) = meta.get("fonts").and_then(|f| f.as_array()).filter(|f| !f.is_empty()) {
            facts.push(("Fonts", fonts.iter().filter_map(|f| f.as_str()).take(6).collect::<Vec<_>>().join(", ")));
        }
    }
    zip_bytes(&mut zip, "previews/preview.png", PIC_MAX).and_then(|b| decode(&b))
}

/// CorelDRAW X4 and later: a zip with `previews/thumbnail.png`. Older files are RIFF, with a
/// picture in a `DISP` chunk that isn't read here.
fn corel(path: &Path, facts: &mut Vec<(&'static str, String)>) -> Option<image::DynamicImage> {
    let mut head = [0u8; 12];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).ok()?;
    if head.starts_with(b"RIFF") {
        // "CDRx": the letter is the version (X3 is 'D').
        facts.push(("Format", format!("CorelDRAW (RIFF, {})", String::from_utf8_lossy(&head[8..12]).trim())));
        return embedded(path);
    }
    let mut zip = zip_open(path)?;
    facts.push(("Format", "CorelDRAW X4 or later".into()));
    let names: Vec<String> = (0..zip.len()).filter_map(|i| zip.by_index(i).ok().and_then(|f| f.name().ok().map(|n| n.to_string()))).collect();
    let pages = names.iter().filter(|n| n.starts_with("content/data/page") || n.starts_with("content/page")).count();
    if pages > 0 {
        facts.push(("Pages", pages.to_string()));
    }
    zip_bytes(&mut zip, "previews/thumbnail.png", PIC_MAX).and_then(|b| decode(&b))
}

/// The first PNG or JPEG picture inside a file's first few megabytes: the thumbnail most
/// apps save near the start.
fn embedded(path: &Path) -> Option<image::DynamicImage> {
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(SCAN_MAX).read_to_end(&mut buf).ok()?;
    let mut at = 0;
    for _ in 0..64 {
        let png = find(&buf[at..], b"\x89PNG\r\n\x1a\n").map(|i| (at + i, true));
        let jpg = find(&buf[at..], b"\xff\xd8\xff").map(|i| (at + i, false));
        let (start, is_png) = match (png, jpg) {
            (Some(p), Some(j)) => if p.0 < j.0 { p } else { j },
            (Some(p), None) => p,
            (None, Some(j)) => j,
            (None, None) => return None,
        };
        let end = if is_png {
            find(&buf[start..], b"IEND").map(|i| start + i + 8)
        } else {
            find(&buf[start..], b"\xff\xd9").map(|i| start + i + 2)
        }
        .unwrap_or(buf.len())
        .min(buf.len())
        .min(start + PIC_MAX);
        // A JPEG's end marker can come early (a thumbnail inside it); try to the end of the
        // scanned part too.
        if let Some(img) = decode(&buf[start..end]).or_else(|| (!is_png).then(|| decode(&buf[start..(start + PIC_MAX).min(buf.len())])).flatten()) {
            return Some(img);
        }
        at = start + 3;
    }
    None
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// SketchUp's version, written in UTF-16 near the start: `{24.0.594}`.
fn sketchup_version(path: &Path, facts: &mut Vec<(&'static str, String)>) {
    let mut head = vec![0u8; 256];
    let Ok(n) = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)) else { return };
    let units: Vec<u16> = head[..n & !1].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16_lossy(&units);
    if let Some(v) = text.split('{').nth(1).and_then(|s| s.split('}').next()).filter(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.')) {
        let major = v.split('.').next().unwrap_or("");
        let year = major.parse::<u32>().ok().filter(|m| (17..60).contains(m)).map(|m| format!(" (SketchUp 20{m})")).unwrap_or_default();
        facts.push(("Saved with", format!("Version {v}{year}")));
    }
}

/// Blender: a header, then blocks; the `TEST` block is the thumbnail (width, height, then RGBA
/// rows bottom up). Compressed .blend files (gzip before 3.0, zstd since) aren't read.
fn blender(path: &Path, facts: &mut Vec<(&'static str, String)>) -> Result<Option<image::DynamicImage>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut head = [0u8; 17];
    let n = f.read(&mut head).map_err(|e| e.to_string())?;
    if head.starts_with(&[0x1f, 0x8b]) || head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        facts.push(("Compressed", "Yes, so its saved picture isn't shown".into()));
        return Ok(None);
    }
    if n < 12 || !head.starts_with(b"BLENDER") {
        return Err("Not a Blender file".into());
    }
    // BLENDER, then '_' (4-byte pointers) or '-' (8), 'v' (little end) or 'V', then the version.
    // Blender 5 adds a longer header: "BLENDER17-01v0500".
    let (ptr, little, ver, hdr) = if head[7].is_ascii_digit() {
        (8usize, head[12] == b'v', String::from_utf8_lossy(&head[13..17]).to_string(), 17u64)
    } else {
        (if head[7] == b'-' { 8 } else { 4 }, head[8] == b'v', String::from_utf8_lossy(&head[9..12]).to_string(), 12u64)
    };
    // "402" is 4.2, "293" is 2.93; Blender 5 writes "0500".
    let digits = ver.trim_start_matches('0');
    let pretty = match (digits.get(..digits.len().saturating_sub(2)), digits.get(digits.len().saturating_sub(2)..).and_then(|m| m.parse::<u32>().ok())) {
        (Some(major), Some(minor)) if !major.is_empty() => format!("{major}.{minor}"),
        _ => ver.clone(),
    };
    facts.push(("Saved with", format!("Blender {pretty}")));
    let rd = |b: &[u8]| if little { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) };
    f.seek(SeekFrom::Start(hdr)).map_err(|e| e.to_string())?;
    // Block header: code (4), size (4), old pointer (ptr), SDNA index (4), count (4). Blender 5
    // uses 8-byte sizes and counts.
    let big = hdr == 17;
    let bh = if big { 4 + 8 + ptr + 4 + 8 } else { 4 + 4 + ptr + 4 + 4 };
    let mut b = vec![0u8; bh];
    let mut blocks = 0usize;
    let mut scenes = 0;
    let mut objects = 0;
    let mut picture = None;
    for _ in 0..2_000_000 {
        if f.read_exact(&mut b).is_err() {
            break;
        }
        let code = &b[..4];
        let size = if big {
            let lo = rd(&b[4..8]) as u64;
            let hi = rd(&b[8..12]) as u64;
            if little { lo | hi << 32 } else { hi | lo << 32 }
        } else {
            rd(&b[4..8]) as u64
        };
        blocks += 1;
        match code {
            b"ENDB" => break,
            b"SC\0\0" => scenes += 1,
            b"OB\0\0" => objects += 1,
            b"TEST" if picture.is_none() && size >= 8 && size as usize <= PIC_MAX => {
                let mut data = vec![0u8; size as usize];
                f.read_exact(&mut data).map_err(|e| e.to_string())?;
                let (w, h) = (rd(&data[..4]) as usize, rd(&data[4..8]) as usize);
                if w > 0 && h > 0 && w <= 4096 && h <= 4096 && data.len() >= 8 + w * h * 4 {
                    let mut rgba = vec![0u8; w * h * 4];
                    // Rows are stored bottom up.
                    for y in 0..h {
                        let src = 8 + (h - 1 - y) * w * 4;
                        rgba[y * w * 4..(y + 1) * w * 4].copy_from_slice(&data[src..src + w * 4]);
                    }
                    picture = image::RgbaImage::from_raw(w as u32, h as u32, rgba).map(image::DynamicImage::ImageRgba8);
                }
                continue;
            }
            _ => {}
        }
        f.seek(SeekFrom::Current(size as i64)).map_err(|e| e.to_string())?;
    }
    if scenes > 0 {
        facts.push(("Scenes", scenes.to_string()));
    }
    if objects > 0 {
        facts.push(("Objects", crate::inspect::group(objects)));
    }
    let _ = blocks;
    Ok(picture)
}

/// Maya ASCII files say what made them and what's in them in plain text; binary ones are IFF
/// with a few readable strings near the start.
fn maya(path: &Path, facts: &mut Vec<(&'static str, String)>) {
    let mut buf = Vec::new();
    if std::fs::File::open(path).and_then(|f| f.take(SCAN_MAX).read_to_end(&mut buf)).is_err() {
        return;
    }
    let text = String::from_utf8_lossy(&buf);
    if let Some(first) = text.lines().next().filter(|l| l.starts_with("//Maya")) {
        facts.push(("Format", first.trim_start_matches("//").trim().to_string()));
        let mut nodes: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for l in text.lines() {
            if let Some(rest) = l.strip_prefix("createNode ") {
                *nodes.entry(rest.split_whitespace().next().unwrap_or("")).or_default() += 1;
            } else if let Some(rest) = l.strip_prefix("currentUnit ") {
                if let Some(u) = rest.split_whitespace().skip_while(|w| *w != "-l").nth(1) {
                    facts.push(("Units", u.trim_end_matches(';').to_string()));
                }
            } else if let Some(rest) = l.strip_prefix("fileInfo \"product\" ") {
                facts.push(("Saved with", rest.trim_end_matches(';').trim_matches('"').to_string()));
            }
        }
        let total: usize = nodes.values().sum();
        facts.push(("Nodes", crate::inspect::group(total as u64)));
        for (kind, label) in [("mesh", "Meshes"), ("camera", "Cameras"), ("joint", "Joints")] {
            if let Some(n) = nodes.get(kind).filter(|n| **n > 0) {
                facts.push((label, crate::inspect::group(*n as u64)));
            }
        }
        let refs = text.lines().filter(|l| l.starts_with("file -r")).count();
        if refs > 0 {
            facts.push(("References", refs.to_string()));
        }
    } else if buf.starts_with(b"FOR4") || buf.starts_with(b"FOR8") {
        facts.push(("Format", "Maya binary".into()));
        if let Some(i) = find(&buf[..buf.len().min(4096)], b"product\0") {
            let s: String = buf[i + 8..].iter().take(64).take_while(|&&c| c != 0).map(|&c| c as char).collect();
            if !s.is_empty() {
                facts.push(("Saved with", s));
            }
        }
    }
}

/// Houdini .hip files are an archive of text; the first few name the version.
fn houdini(path: &Path, facts: &mut Vec<(&'static str, String)>) {
    let mut buf = Vec::new();
    if std::fs::File::open(path).and_then(|f| f.take(1 << 20).read_to_end(&mut buf)).is_err() {
        return;
    }
    let text = String::from_utf8_lossy(&buf);
    if let Some(v) = text.lines().find_map(|l| l.trim().strip_prefix("set -g _HIP_SAVEVERSION = ")) {
        facts.push(("Saved with", format!("Houdini {}", v.trim_matches('\''))));
    }
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("hipnc")) {
        facts.push(("Licence", "Non-commercial (Apprentice)".into()));
    }
    let nodes = text.matches(".init\0").count() + text.matches(".init\n").count();
    if nodes > 0 {
        facts.push(("Nodes", format!("{} or more", crate::inspect::group(nodes as u64))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("filescape-saved-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn fact<'a>(s: &'a Saved, k: &str) -> Option<&'a str> {
        s.facts.iter().find(|f| f.0 == k).map(|f| f.1.as_str())
    }

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, b) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn iwork_and_sketch_previews() {
        let p = tmp("Report.pages");
        let mut jpg = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(60, 80, image::Rgb([10, 20, 30])).write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
        zip_with(&p, &[("Index/Document.iwa", b"x"), ("preview.jpg", &jpg.into_inner()), ("Metadata/BuildVersionHistory.plist", b"<array><string>Template: Blank (14.0)</string><string>M14.2-7041.0.109-4</string></array>")]);
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "App"), Some("Pages"));
        assert_eq!(fact(&s, "Format"), Some("iWork 2013 or later"));
        assert_eq!(fact(&s, "Last saved by"), Some("M14.2-7041.0.109-4"));
        assert_eq!(fact(&s, "Template"), Some("Blank (14.0)"));
        assert_eq!(s.picture.map(|p| (p.width(), p.height())), Some((60, 80)));
        let p = tmp("Design.sketch");
        zip_with(&p, &[("previews/preview.png", &png(40, 30)), ("meta.json", br#"{"appVersion":"101","pagesAndArtboards":{"a":{"artboards":{"x":{},"y":{}}},"b":{"artboards":{}}},"fonts":["Inter"]}"#)]);
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Saved with"), Some("Sketch 101"));
        assert_eq!(fact(&s, "Pages"), Some("2"));
        assert_eq!(fact(&s, "Artboards"), Some("2"));
        assert!(s.picture.is_some());
    }

    #[test]
    fn picture_found_inside_a_binary_file() {
        let p = tmp("poster.afdesign");
        let mut b = b"\0\xffKInAjunk\xff\xd8 not a jpeg".to_vec();
        b.extend(png(32, 32));
        b.extend(b"more data after");
        std::fs::write(&p, &b).unwrap();
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "App"), Some("Affinity"));
        assert_eq!(s.picture.map(|p| p.width()), Some(32));
    }

    #[test]
    fn blender_thumbnail_block() {
        let p = tmp("scene.blend");
        let mut b = b"BLENDER-v402".to_vec();
        let block = |code: &[u8; 4], data: &[u8]| {
            let mut out = code.to_vec();
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(0u64.to_le_bytes());
            out.extend(0u32.to_le_bytes());
            out.extend(1u32.to_le_bytes());
            out.extend(data);
            out
        };
        b.extend(block(b"REND", &[0; 8]));
        // 2×2: bottom row red, top row blue.
        let mut t = Vec::new();
        t.extend(2u32.to_le_bytes());
        t.extend(2u32.to_le_bytes());
        t.extend([255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255]);
        b.extend(block(b"TEST", &t));
        b.extend(block(b"SC\0\0", &[0; 4]));
        b.extend(block(b"OB\0\0", &[0; 4]));
        b.extend(block(b"OB\0\0", &[0; 4]));
        b.extend(block(b"ENDB", &[]));
        std::fs::write(&p, &b).unwrap();
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Saved with"), Some("Blender 4.2"));
        assert_eq!(fact(&s, "Scenes"), Some("1"));
        assert_eq!(fact(&s, "Objects"), Some("2"));
        // Turned the right way up: blue on top.
        let pic = s.picture.expect("thumbnail").to_rgba8();
        assert_eq!(pic.dimensions(), (2, 2));
        assert_eq!(pic.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(pic.get_pixel(1, 1).0, [255, 0, 0, 255]);
        std::fs::write(&p, [0x28, 0xb5, 0x2f, 0xfd, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert!(fact(&read(&p).unwrap(), "Compressed").is_some());
    }

    #[test]
    fn maya_ascii_and_houdini_details() {
        let p = tmp("rig.ma");
        std::fs::write(&p, "//Maya ASCII 2024 scene\n//Name: rig.ma\nrequires maya \"2024\";\ncurrentUnit -l centimeter -a degree -t film;\nfileInfo \"product\" \"Maya 2024\";\ncreateNode transform -n \"a\";\ncreateNode mesh -n \"aShape\";\ncreateNode camera -n \"cam\";\nfile -rdi 1 -ns \"x\" \"x.ma\";\nfile -r -ns \"x\" \"x.ma\";\n").unwrap();
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Format"), Some("Maya ASCII 2024 scene"));
        assert_eq!(fact(&s, "Units"), Some("centimeter"));
        assert_eq!(fact(&s, "Saved with"), Some("Maya 2024"));
        assert_eq!(fact(&s, "Nodes"), Some("3"));
        assert_eq!(fact(&s, "Meshes"), Some("1"));
        assert_eq!(fact(&s, "References"), Some("2"));
        let p = tmp("shot.hipnc");
        std::fs::write(&p, "070707...\nset -g _HIP_SAVEVERSION = '20.5.370'\n").unwrap();
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Saved with"), Some("Houdini 20.5.370"));
        assert_eq!(fact(&s, "Licence"), Some("Non-commercial (Apprentice)"));
    }

    #[test]
    fn sketchup_version_and_broken_files() {
        let p = tmp("house.skp");
        let mut b = vec![0xff, 0xfe, 0xff, 0x0e];
        for c in "SketchUp Model{24.0.594}".encode_utf16() {
            b.extend(c.to_le_bytes());
        }
        std::fs::write(&p, &b).unwrap();
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Saved with"), Some("Version 24.0.594 (SketchUp 2024)"));
        for name in ["x.blend", "x.pages", "x.cdr", "x.max", "x.mb"] {
            let p = tmp(name);
            std::fs::write(&p, b"BLENDER").unwrap();
            let _ = read(&p);
        }
    }
}
