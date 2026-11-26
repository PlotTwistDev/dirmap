//! Painting and design app files: Krita (.kra, .krz), OpenRaster (.ora), Figma local copies
//! (.fig), GIMP (.xcf, .xcf.gz) and Paint.NET (.pdn).
//!
//! Krita, OpenRaster and Figma files carry a picture of the whole document, shown as it is, with
//! the layers listed from their XML. GIMP and Paint.NET files carry only layers: Filescape draws
//! them, stacked in Normal mode with their opacity, visibility and masks, using `craft-xcf` and
//! `craft-pdn` from craft-libs (MIT or Apache-2.0) to read them. Other blend modes are drawn as
//! Normal, and the preview says so. Read-only; nothing is run.

use crate::savedpic::{Saved, decode, zip_bytes, zip_open};
use std::path::Path;

/// Files larger than this aren't read (GIMP and Paint.NET files are read whole).
const FILE_MAX: u64 = 512 << 20;
/// Pictures and XML read from inside a ZIP, at most.
const ENTRY_MAX: usize = 64 << 20;
/// The drawn picture is at most this many pixels across.
const DRAW_MAX: u32 = 2048;
/// Documents larger than this aren't drawn.
const CANVAS_MAX: u64 = 1 << 28;
/// Bytes of decoded layer samples at a time.
const BUDGET: usize = 1 << 30;
/// Layer names listed.
const NAMES: usize = 12;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Paint {
    Krita,
    OpenRaster,
    Figma,
    Gimp,
    PaintNet,
}

impl Paint {
    pub fn name(self) -> &'static str {
        match self {
            Paint::Krita => "Krita",
            Paint::OpenRaster => "OpenRaster",
            Paint::Figma => "Figma",
            Paint::Gimp => "GIMP",
            Paint::PaintNet => "Paint.NET",
        }
    }

    /// For the Kind column.
    pub fn kind(self) -> &'static str {
        match self {
            Paint::Krita => "Krita document",
            Paint::OpenRaster => "OpenRaster picture",
            Paint::Figma => "Figma file",
            Paint::Gimp => "GIMP picture",
            Paint::PaintNet => "Paint.NET picture",
        }
    }
}

pub fn of(name: &str) -> Option<Paint> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".xcf.gz") || lower.ends_with(".xcfgz") {
        return Some(Paint::Gimp);
    }
    Some(match crate::scan::ext_of(&lower) {
        "kra" | "krz" => Paint::Krita,
        "ora" => Paint::OpenRaster,
        "fig" => Paint::Figma,
        "xcf" => Paint::Gimp,
        "pdn" => Paint::PaintNet,
        _ => return None,
    })
}

pub fn read(path: &Path) -> Result<Saved, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let paint = of(&name).ok_or("Not a file Filescape knows")?;
    let mut out = Saved { picture: None, facts: vec![("App", paint.name().to_string())], note: None, refs: None };
    match paint {
        Paint::Krita => krita(path, &mut out)?,
        Paint::OpenRaster => openraster(path, &mut out)?,
        Paint::Figma => figma(path, &mut out)?,
        Paint::Gimp => gimp(&whole(path)?, &name, &mut out)?,
        Paint::PaintNet => paintnet(&whole(path)?, &mut out)?,
    }
    if out.picture.is_none() && out.note.is_none() {
        out.note = Some(format!("There's no picture saved inside it, and Filescape couldn't draw this {} file.", paint.name()));
    }
    Ok(out)
}

fn whole(path: &Path) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't read it: {e}"))?.len();
    if size > FILE_MAX {
        return Err(format!("Too big to read ({})", crate::scan::human(size)));
    }
    std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", crate::inspect::group(n as u64), if n == 1 { one } else { many })
}

/// "Layers: 5 (1 hidden)" and the names, top first.
fn layer_facts(out: &mut Saved, layers: &[(String, bool)], groups: usize) {
    if layers.is_empty() {
        return;
    }
    let hidden = layers.iter().filter(|l| !l.1).count();
    let mut s = count(layers.len() - groups, "layer", "layers");
    let mut parts = Vec::new();
    if groups > 0 {
        parts.push(count(groups, "group", "groups"));
    }
    if hidden > 0 {
        parts.push(format!("{hidden} hidden"));
    }
    if !parts.is_empty() {
        s.push_str(&format!(" ({})", parts.join(", ")));
    }
    out.facts.push(("Layers", s));
    let names: Vec<String> = layers.iter().map(|l| if l.0.trim().is_empty() { "(unnamed)".to_string() } else { l.0.trim().to_string() }).collect();
    let mut list = names.iter().take(NAMES).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > NAMES {
        list.push_str(&format!(" and {} more", names.len() - NAMES));
    }
    out.facts.push(("Layer names", list));
}

fn xml(b: &[u8]) -> Option<roxmltree::Document<'_>> {
    let text = std::str::from_utf8(b).ok()?;
    roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 4_000_000, ..Default::default() }).ok()
}

// ---- Krita and OpenRaster: the merged picture they save, and the layer tree. ----

fn krita(path: &Path, out: &mut Saved) -> Result<(), String> {
    let mut zip = zip_open(path).ok_or("Not a Krita document")?;
    if zip_bytes(&mut zip, "mimetype", 256).is_none_or(|m| !m.starts_with(b"application/x-krita")) {
        return Err("Not a Krita document".into());
    }
    out.picture = zip_bytes(&mut zip, "mergedimage.png", ENTRY_MAX).and_then(|b| decode(&b)).or_else(|| zip_bytes(&mut zip, "preview.png", ENTRY_MAX).and_then(|b| decode(&b)));
    if let Some(doc) = zip_bytes(&mut zip, "maindoc.xml", ENTRY_MAX) {
        if let Some(d) = xml(&doc) {
            if let Some(v) = d.root_element().attribute("syntaxVersion").or_else(|| d.root_element().attribute("kritaVersion")) {
                out.facts.push(("Saved by", d.root_element().attribute("kritaVersion").map_or_else(|| format!("format {v}"), |k| format!("Krita {k}"))));
            }
            if let Some(img) = d.descendants().find(|n| n.has_tag_name("IMAGE")) {
                if let (Some(w), Some(h)) = (img.attribute("width"), img.attribute("height")) {
                    out.facts.push(("Size", format!("{w} × {h} pixels")));
                }
                if let Some(c) = img.attribute("colorspacename") {
                    out.facts.push(("Colour", c.to_string()));
                }
                // Layers sit in <layers><layer …/></layers>, groups holding their own <layers>.
                let layers: Vec<roxmltree::Node> = img.descendants().filter(|n| n.has_tag_name("layer")).collect();
                let groups = layers.iter().filter(|n| n.attribute("nodetype") == Some("grouplayer")).count();
                let list: Vec<(String, bool)> = layers.iter().map(|n| (n.attribute("name").unwrap_or("").to_string(), n.attribute("visible") != Some("0"))).collect();
                layer_facts(out, &list, groups);
            }
        }
    }
    Ok(())
}

fn openraster(path: &Path, out: &mut Saved) -> Result<(), String> {
    let mut zip = zip_open(path).ok_or("Not an OpenRaster picture")?;
    if zip_bytes(&mut zip, "mimetype", 256).is_none_or(|m| !m.starts_with(b"image/openraster")) {
        return Err("Not an OpenRaster picture".into());
    }
    out.picture = zip_bytes(&mut zip, "mergedimage.png", ENTRY_MAX)
        .and_then(|b| decode(&b))
        .or_else(|| zip_bytes(&mut zip, "Thumbnails/thumbnail.png", ENTRY_MAX).and_then(|b| decode(&b)));
    if let Some(d) = zip_bytes(&mut zip, "stack.xml", ENTRY_MAX).as_deref().and_then(xml) {
        let image = d.root_element();
        if let (Some(w), Some(h)) = (image.attribute("w"), image.attribute("h")) {
            out.facts.push(("Size", format!("{w} × {h} pixels")));
        }
        // The outermost <stack> is the image itself, not a group.
        let groups = image.descendants().filter(|n| n.has_tag_name("stack")).count().saturating_sub(1);
        let list: Vec<(String, bool)> = image
            .descendants()
            .filter(|n| n.has_tag_name("layer") || (n.has_tag_name("stack") && n.parent_element().is_some_and(|p| p.has_tag_name("stack"))))
            .map(|n| (n.attribute("name").unwrap_or("").to_string(), n.attribute("visibility") != Some("hidden")))
            .collect();
        layer_facts(out, &list, groups);
    }
    Ok(())
}

// ---- Figma: a local copy (.fig) carries a thumbnail and a little metadata. ----

fn figma(path: &Path, out: &mut Saved) -> Result<(), String> {
    let mut zip = zip_open(path).ok_or("Not a Figma file (only local copies saved as ZIPs are read)")?;
    let names: Vec<String> = zip.file_names().filter_map(Result::ok).map(|n| n.into_owned()).collect();
    if !names.iter().any(|n| n == "canvas.fig") {
        return Err("Not a Figma file".into());
    }
    out.picture = zip_bytes(&mut zip, "thumbnail.png", ENTRY_MAX).and_then(|b| decode(&b));
    if let Some(meta) = zip_bytes(&mut zip, "meta.json", 1 << 20).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
        if let Some(n) = meta.get("file_name").and_then(|v| v.as_str()).filter(|n| !n.trim().is_empty()) {
            out.facts.push(("Name", n.trim().to_string()));
        }
        if let Some(t) = meta.get("exported_at").and_then(|v| v.as_str()) {
            out.facts.push(("Saved", t.to_string()));
        }
    }
    let images = names.iter().filter(|n| n.starts_with("images/") && !n.ends_with('/')).count();
    if images > 0 {
        out.facts.push(("Pictures inside", images.to_string()));
    }
    if out.picture.is_none() {
        out.note = Some("This Figma file has no picture saved inside it, and Filescape doesn't draw Figma files itself.".into());
    }
    Ok(())
}

// ---- GIMP and Paint.NET: drawn from their layers. ----

/// Layers stacked in Normal mode, at most `DRAW_MAX` across (nearest pixel), premultiplied.
struct Canvas {
    w: u32,
    h: u32,
    step: u32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Option<Self> {
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > CANVAS_MAX {
            return None;
        }
        let step = w.max(h).div_ceil(DRAW_MAX).max(1);
        let (cw, ch) = (w.div_ceil(step), h.div_ceil(step));
        Some(Canvas { w: cw, h: ch, step, px: vec![[0.0; 4]; cw as usize * ch as usize] })
    }

    /// Lays a picture of `lw` × `lh` at (`x`, `y`) over what's there. `at` gives a straight RGBA
    /// pixel of it, 0 to 1.
    fn over(&mut self, x: i32, y: i32, lw: u32, lh: u32, opacity: f32, at: impl Fn(u32, u32) -> [f32; 4]) {
        if opacity <= 0.0 {
            return;
        }
        for cy in 0..self.h {
            let sy = i64::from(cy * self.step) - i64::from(y);
            if sy < 0 || sy >= i64::from(lh) {
                continue;
            }
            for cx in 0..self.w {
                let sx = i64::from(cx * self.step) - i64::from(x);
                if sx < 0 || sx >= i64::from(lw) {
                    continue;
                }
                let [r, g, b, a] = at(sx as u32, sy as u32);
                let a = (a * opacity).clamp(0.0, 1.0);
                let d = &mut self.px[(cy * self.w + cx) as usize];
                let k = 1.0 - a;
                *d = [r * a + d[0] * k, g * a + d[1] * k, b * a + d[2] * k, a + d[3] * k];
            }
        }
    }

    fn image(&self) -> image::DynamicImage {
        let mut out = image::RgbaImage::new(self.w, self.h);
        for (p, o) in self.px.iter().zip(out.pixels_mut()) {
            let a = p[3];
            let c = |v: f32| if a > 0.0 { ((v / a).clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
            *o = image::Rgba([c(p[0]), c(p[1]), c(p[2]), (a.clamp(0.0, 1.0) * 255.0).round() as u8]);
        }
        image::DynamicImage::ImageRgba8(out)
    }
}

const GO: &dyn Fn() -> bool = &|| true;

fn gimp(bytes: &[u8], name: &str, out: &mut Saved) -> Result<(), String> {
    let unzipped;
    let bytes = if craft_xcf::is_gzipped_xcf(name, bytes) {
        unzipped = craft_xcf::gunzip(bytes).map_err(|_| "Not a GIMP picture, or a damaged one".to_string())?;
        &unzipped[..]
    } else {
        bytes
    };
    if !craft_xcf::is_xcf(bytes) {
        return Err("Not a GIMP picture".into());
    }
    let xcf = craft_xcf::Xcf::parse(bytes).map_err(|_| "This GIMP picture is damaged".to_string())?;
    let img = xcf.image();
    out.facts.push(("Format", format!("XCF version {}", img.version)));
    out.facts.push(("Size", format!("{} × {} pixels", img.width, img.height)));
    let base = match img.base_type {
        craft_xcf::BaseType::Rgb => "RGB",
        craft_xcf::BaseType::Gray => "Greyscale",
        craft_xcf::BaseType::Indexed => "Indexed",
    };
    out.facts.push(("Colour", base.to_string()));
    let tree = img.tree();
    // Names top first, as GIMP lists them.
    let list: Vec<(String, bool)> = img.layers.iter().filter(|l| !l.floating_selection).map(|l| (l.name.clone(), l.visible)).collect();
    let groups = img.layers.iter().filter(|l| l.group).count();
    layer_facts(out, &list, groups);
    let Some(mut canvas) = Canvas::new(img.width, img.height) else {
        out.note = Some("This picture is too large for Filescape to draw.".into());
        return Ok(());
    };
    // GIMP's Normal is 28 (0 in old files); a group's Pass Through is 61.
    let other_modes = img.layers.iter().filter(|l| l.visible && !l.group && !matches!(l.mode, 0 | 28)).count();
    let mut budget = BUDGET;
    fn draw(xcf: &craft_xcf::Xcf, nodes: &[craft_xcf::TreeNode], opacity: f32, canvas: &mut Canvas, budget: &mut usize) {
        // Topmost first: draw from the bottom up.
        for node in nodes.iter().rev() {
            let Some(l) = xcf.image().layers.get(node.layer) else { continue };
            if !l.visible {
                continue;
            }
            let o = opacity * l.opacity;
            if l.group {
                draw(xcf, &node.children, o, canvas, budget);
                continue;
            }
            let mut b = *budget;
            let Ok(Some(px)) = xcf.layer_pixels(node.layer, &mut b, GO) else { continue };
            let mask = if l.has_mask() && l.apply_mask { xcf.layer_mask(node.layer, &mut b, GO).ok().flatten().map(|m| m.1) } else { None };
            let c = px.channels;
            canvas.over(l.x, l.y, px.width, px.height, o, |x, y| {
                let i = (y * px.width + x) as usize * c;
                let s = &px.data[i..i + c];
                let mut p = match c {
                    1 => [s[0], s[0], s[0], 1.0],
                    2 => [s[0], s[0], s[0], s[1]],
                    3 => [s[0], s[1], s[2], 1.0],
                    _ => [s[0], s[1], s[2], s[3]],
                };
                if let Some(m) = &mask
                    && let Some(v) = m.data.get((y * m.width + x) as usize)
                {
                    p[3] *= v;
                }
                p
            });
        }
    }
    draw(&xcf, &tree.roots, 1.0, &mut canvas, &mut budget);
    out.picture = Some(canvas.image());
    out.facts.push(("Shown", if other_modes > 0 { format!("Drawn by Filescape from the layers; {} drawn as Normal", count(other_modes, "layer with another blend mode is", "layers with other blend modes are")) } else { "Drawn by Filescape from the layers".into() }));
    Ok(())
}

fn paintnet(bytes: &[u8], out: &mut Saved) -> Result<(), String> {
    if !craft_pdn::is_pdn(bytes) {
        return Err("Not a Paint.NET picture".into());
    }
    let doc = craft_pdn::Document::parse(bytes, GO).map_err(|_| "This Paint.NET picture is damaged, or from a version Filescape can't read".to_string())?;
    let (w, h) = (doc.width as u32, doc.height as u32);
    out.facts.push(("Size", format!("{w} × {h} pixels")));
    // Bottom first in the file; listed top first.
    let list: Vec<(String, bool)> = doc.layers.iter().rev().map(|l| (l.name.clone(), l.visible)).collect();
    layer_facts(out, &list, 0);
    let Some(mut canvas) = Canvas::new(w, h) else {
        out.note = Some("This picture is too large for Filescape to draw.".into());
        return Ok(());
    };
    let wanted: std::collections::HashSet<usize> = doc.layers.iter().filter(|l| l.visible && l.opacity > 0).map(|l| l.surface).collect();
    let mut surfaces: std::collections::HashMap<usize, craft_pdn::Surface> = std::collections::HashMap::new();
    doc.read_surfaces(GO, &mut |i, s| {
        if wanted.contains(&i) {
            surfaces.insert(i, s);
        }
    })
    .map_err(|_| "This Paint.NET picture is damaged".to_string())?;
    let other_modes = doc.layers.iter().filter(|l| l.visible && l.blend != craft_pdn::BlendMode::Normal).count();
    for l in doc.layers.iter().filter(|l| l.visible) {
        let Some(s) = surfaces.get(&l.surface) else { continue };
        let stride = s.stride.max(1);
        canvas.over(0, 0, s.width as u32, (s.data.len() / stride) as u32, f32::from(l.opacity) / 255.0, |x, y| {
            let i = y as usize * stride + x as usize * 4;
            match s.data.get(i..i + 4) {
                Some(p) => [f32::from(p[2]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[0]) / 255.0, f32::from(p[3]) / 255.0],
                None => [0.0; 4],
            }
        });
    }
    out.picture = Some(canvas.image());
    out.facts.push(("Shown", if other_modes > 0 { format!("Drawn by Filescape from the layers; {} drawn as Normal", count(other_modes, "layer with another blend mode is", "layers with other blend modes are")) } else { "Drawn by Filescape from the layers".into() }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("filescape-paint-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([10, 200, 30, 255])).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (n, b) in entries {
            z.start_file(*n, o).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    fn fact<'a>(s: &'a Saved, k: &str) -> Option<&'a str> {
        s.facts.iter().find(|f| f.0 == k).map(|f| f.1.as_str())
    }

    fn file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn krita_openraster_and_figma_show_their_saved_picture() {
        let maindoc = br#"<?xml version="1.0"?><DOC syntaxVersion="2.0" kritaVersion="5.2.2"><IMAGE width="640" height="480" colorspacename="RGBA"><layers>
            <layer name="Ink" nodetype="paintlayer" visible="1"/><layer name="Group" nodetype="grouplayer" visible="1"><layers><layer name="Sketch" nodetype="paintlayer" visible="0"/></layers></layer>
            </layers></IMAGE></DOC>"#;
        let p = file("Art.kra", &zip(&[("mimetype", b"application/x-krita"), ("maindoc.xml", maindoc), ("mergedimage.png", &png(64, 48))]));
        let s = read(&p).unwrap();
        assert_eq!(s.picture.as_ref().map(|i| i.width()), Some(64));
        assert_eq!(fact(&s, "Saved by"), Some("Krita 5.2.2"));
        assert_eq!(fact(&s, "Size"), Some("640 × 480 pixels"));
        assert_eq!(fact(&s, "Layers"), Some("2 layers (1 group, 1 hidden)"));
        assert_eq!(fact(&s, "Layer names"), Some("Ink, Group, Sketch"));

        let stack = br#"<image w="300" h="200"><stack><layer name="Top" src="data/1.png"/><stack name="Folder"><layer name="Under" visibility="hidden" src="data/2.png"/></stack></stack></image>"#;
        let p = file("Pic.ora", &zip(&[("mimetype", b"image/openraster"), ("stack.xml", stack), ("Thumbnails/thumbnail.png", &png(30, 20))]));
        let s = read(&p).unwrap();
        assert_eq!(s.picture.as_ref().map(|i| i.width()), Some(30));
        assert_eq!(fact(&s, "Layers"), Some("2 layers (1 group, 1 hidden)"));
        assert_eq!(fact(&s, "Layer names"), Some("Top, Folder, Under"));

        let p = file("Design.fig", &zip(&[("canvas.fig", b"fig-kiwi"), ("meta.json", br#"{"file_name":"Onboarding","exported_at":"2026-11-02T10:00:00Z"}"#), ("thumbnail.png", &png(40, 30)), ("images/abc", b"x")]));
        let s = read(&p).unwrap();
        assert_eq!(s.picture.as_ref().map(|i| i.height()), Some(30));
        assert_eq!(fact(&s, "Name"), Some("Onboarding"));
        assert_eq!(fact(&s, "Pictures inside"), Some("1"));
        // A Figma file of another kind, a renamed file: refused.
        assert!(read(&file("x.fig", b"fig-kiwi....")).is_err());
        assert!(read(&file("x.kra", &zip(&[("mimetype", b"image/png")]))).is_err());
    }

    // GIMP: a small XCF writer after the one in craft-xcf's tests (MIT or Apache-2.0):
    // version 11, uncompressed 8-bit RGBA layers.
    fn xcf(w: u32, h: u32, layers: &[(&str, i32, i32, u32, u32, f32, bool, u32, [u8; 4])]) -> Vec<u8> {
        let mut o: Vec<u8> = b"gimp xcf v011\0".to_vec();
        let u32b = |o: &mut Vec<u8>, v: u32| o.extend(v.to_be_bytes());
        let ptr = |o: &mut Vec<u8>, v: u64| o.extend(v.to_be_bytes());
        let prop = |o: &mut Vec<u8>, k: u32, p: &[u8]| {
            o.extend(k.to_be_bytes());
            o.extend((p.len() as u32).to_be_bytes());
            o.extend(p);
        };
        u32b(&mut o, w);
        u32b(&mut o, h);
        u32b(&mut o, 0); // RGB
        u32b(&mut o, 150); // 8-bit, perceptual
        prop(&mut o, 17, &[0]); // no compression
        o.extend([0u8; 8]);
        let slots: Vec<usize> = layers
            .iter()
            .map(|_| {
                let at = o.len();
                ptr(&mut o, 0);
                at
            })
            .collect();
        ptr(&mut o, 0);
        ptr(&mut o, 0); // no channels
        for (l, slot) in layers.iter().zip(slots) {
            let (name, x, y, lw, lh, opacity, visible, mode, rgba) = *l;
            let at = o.len() as u64;
            o[slot..slot + 8].copy_from_slice(&at.to_be_bytes());
            u32b(&mut o, lw);
            u32b(&mut o, lh);
            u32b(&mut o, 1); // RGBA
            u32b(&mut o, name.len() as u32 + 1);
            o.extend(name.as_bytes());
            o.push(0);
            prop(&mut o, 6, &((opacity * 255.0).round() as u32).to_be_bytes());
            prop(&mut o, 33, &opacity.to_be_bytes());
            prop(&mut o, 8, &u32::from(visible).to_be_bytes());
            prop(&mut o, 7, &mode.to_be_bytes());
            let mut off = x.to_be_bytes().to_vec();
            off.extend(y.to_be_bytes());
            prop(&mut o, 15, &off);
            o.extend([0u8; 8]);
            let hslot = o.len();
            ptr(&mut o, 0);
            ptr(&mut o, 0); // no mask
            let hier = o.len() as u64;
            o[hslot..hslot + 8].copy_from_slice(&hier.to_be_bytes());
            u32b(&mut o, lw);
            u32b(&mut o, lh);
            u32b(&mut o, 4);
            let lslot = o.len();
            ptr(&mut o, 0);
            ptr(&mut o, 0);
            let level = o.len() as u64;
            o[lslot..lslot + 8].copy_from_slice(&level.to_be_bytes());
            u32b(&mut o, lw);
            u32b(&mut o, lh);
            // One tile (the layers here are under 64 × 64).
            let tslot = o.len();
            ptr(&mut o, 0);
            ptr(&mut o, 0);
            let tile = o.len() as u64;
            o[tslot..tslot + 8].copy_from_slice(&tile.to_be_bytes());
            for _ in 0..lw * lh {
                o.extend(rgba);
            }
        }
        o
    }

    #[test]
    fn gimp_layers_are_drawn() {
        let b = xcf(
            8,
            8,
            &[
                // Top first: a hidden layer, a half-opaque red square in the corner, a blue background.
                ("Hidden", 0, 0, 8, 8, 1.0, false, 28, [0, 255, 0, 255]),
                ("Red", 4, 4, 4, 4, 0.5, true, 28, [255, 0, 0, 255]),
                ("Back", 0, 0, 8, 8, 1.0, true, 28, [0, 0, 255, 255]),
            ],
        );
        let p = file("Pic.xcf", &b);
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Size"), Some("8 × 8 pixels"));
        assert_eq!(fact(&s, "Layers"), Some("3 layers (1 hidden)"));
        assert_eq!(fact(&s, "Layer names"), Some("Hidden, Red, Back"));
        let img = s.picture.unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255, 255]);
        let p = img.get_pixel(6, 6).0;
        assert!((p[0] as i32 - 128).abs() <= 1 && (p[2] as i32 - 127).abs() <= 1 && p[1] == 0, "{p:?}");
        // Gzipped the same.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&b).unwrap();
        let p = file("Pic.xcf.gz", &gz.finish().unwrap());
        assert!(read(&p).unwrap().picture.is_some());
        // Damaged: an error, never a panic.
        for n in [10, 40, b.len() / 2, b.len() - 3] {
            let _ = read(&file("Cut.xcf", &b[..n]));
        }
    }

    // Paint.NET: the synthetic writer from craft-pdn's tests (MIT or Apache-2.0), 2 × 2 layers.
    enum Field {
        Int(i32),
        Long(i64),
        Byte(u8),
        Bool(bool),
        Ref(i32),
    }

    struct Writer(Vec<u8>);

    impl Writer {
        fn int(&mut self, n: i32) {
            self.0.extend(n.to_le_bytes());
        }
        fn string(&mut self, s: &str) {
            let mut len = s.len();
            while len >= 128 {
                self.0.push((len as u8 & 127) | 128);
                len >>= 7;
            }
            self.0.push(len as u8);
            self.0.extend(s.as_bytes());
        }
        fn object(&mut self, id: i32, class: &str, fields: &[(&str, Field)]) {
            self.0.push(5);
            self.int(id);
            self.string(class);
            self.int(fields.len() as i32);
            for (name, _) in fields {
                self.string(name);
            }
            for (_, f) in fields {
                self.0.push(if matches!(f, Field::Ref(_)) { 2 } else { 0 });
            }
            for (_, f) in fields {
                match f {
                    Field::Int(_) => self.0.push(8),
                    Field::Long(_) => self.0.push(9),
                    Field::Byte(_) => self.0.push(2),
                    Field::Bool(_) => self.0.push(1),
                    Field::Ref(_) => {}
                }
            }
            self.int(1);
            for (_, f) in fields {
                match f {
                    Field::Int(n) => self.int(*n),
                    Field::Long(n) => self.0.extend(n.to_le_bytes()),
                    Field::Byte(n) => self.0.push(*n),
                    Field::Bool(b) => self.0.push(u8::from(*b)),
                    Field::Ref(id) => {
                        self.0.push(9);
                        self.int(*id);
                    }
                }
            }
        }
    }

    /// Layer 0 (the background, bottom) is opaque, layer 1 is hidden. Pixels are B, G, R, A.
    fn pdn(layers: usize) -> Vec<u8> {
        use Field::*;
        let mut w = Writer(b"PDN3\0\0\0\0\x01".to_vec());
        w.0.push(0);
        for n in [1, -1, 1, 0] {
            w.int(n);
        }
        w.0.push(12);
        w.int(1);
        w.string("PaintDotNet.Data");
        w.object(1, "PaintDotNet.Document", &[("width", Int(2)), ("height", Int(2)), ("layers", Ref(2))]);
        w.object(2, "PaintDotNet.LayerList", &[("parent", Ref(1)), ("ArrayList+_size", Int(layers as i32)), ("ArrayList+_items", Ref(3))]);
        w.0.push(16);
        w.int(3);
        w.int(layers as i32 + 2);
        for i in 0..layers {
            w.0.push(9);
            w.int(16 + i as i32 * 10);
        }
        w.0.extend([13, 2]);
        for i in 0..layers {
            let id = 16 + i as i32 * 10;
            w.object(
                id,
                "PaintDotNet.BitmapLayer",
                &[("Layer+width", Int(2)), ("Layer+height", Int(2)), ("surface", Ref(id + 1)), ("Layer+properties", Ref(id + 2)), ("properties", Ref(id + 3))],
            );
            w.object(id + 1, "PaintDotNet.Surface", &[("width", Int(2)), ("height", Int(2)), ("stride", Int(8)), ("scan0", Ref(id + 7))]);
            let fields = vec![("name", Ref(id + 4)), ("opacity", Byte(255)), ("visible", Bool(i == 0)), ("isBackground", Bool(i == 0)), ("blendMode", Ref(id + 5))];
            w.object(id + 2, "PaintDotNet.Layer+LayerProperties", &fields);
            w.object(id + 3, "PaintDotNet.BitmapLayer+BitmapLayerProperties", &[("blendOp", Ref(id + 6))]);
            w.0.push(6);
            w.int(id + 4);
            w.string(&format!("Layer {i}"));
            w.object(id + 5, "PaintDotNet.LayerBlendMode", &[("value__", Int(0))]);
            w.object(id + 6, "PaintDotNet.UserBlendOps+NormalBlendOp", &[]);
            w.object(id + 7, "PaintDotNet.MemoryBlock", &[("length64", Long(16)), ("hasParent", Bool(false)), ("deferred", Bool(true))]);
        }
        w.0.push(11);
        for i in 0..layers {
            w.0.push(1);
            w.0.extend(16u32.to_be_bytes());
            let px: [u8; 4] = if i == 0 { [255, 0, 0, 255] } else { [0, 255, 0, 255] };
            w.0.extend(0u32.to_be_bytes());
            w.0.extend(16u32.to_be_bytes());
            for _ in 0..4 {
                w.0.extend(px);
            }
        }
        w.0
    }

    #[test]
    fn paintnet_layers_are_drawn() {
        let b = pdn(2);
        let p = file("Pic.pdn", &b);
        let s = read(&p).unwrap();
        assert_eq!(fact(&s, "Size"), Some("2 × 2 pixels"));
        assert_eq!(fact(&s, "Layers"), Some("2 layers (1 hidden)"));
        assert_eq!(fact(&s, "Layer names"), Some("Layer 1, Layer 0"));
        // The background is blue (B = 255); the hidden green layer isn't drawn.
        assert_eq!(s.picture.unwrap().to_rgba8().get_pixel(1, 1).0, [0, 0, 255, 255]);
        for n in [5, 30, b.len() / 2, b.len() - 1] {
            let _ = read(&file("Cut.pdn", &b[..n]));
        }
        assert!(read(&file("No.pdn", b"PDN3 but not really")).is_err());
    }
}
