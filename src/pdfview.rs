//! PDF pages drawn with hayro, a PDF renderer written in Rust (MIT or Apache-2.0), so every page
//! can be shown on every system without a PDF library to ship. Documents hayro can't open
//! (password-protected ones, damaged ones) fall back to the system's picture of the first page.

use hayro::hayro_interpret::{InterpreterSettings, TransformExt};
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use std::path::Path;

/// PDFs larger than this aren't read (the whole file is held in memory while it's shown).
pub const MAX_FILE: u64 = 256 << 20;
/// The largest page drawn, in pixels on either side.
const MAX_SIDE: f32 = 4000.0;

/// An open document. Pages are drawn from it on worker threads as they come into view.
pub struct Doc {
    pdf: Pdf,
    /// Each page's size in points (1/72 inch), as shown (rotation applied).
    pub sizes: Vec<[f32; 2]>,
    /// Title, author and so on, for the details.
    pub facts: Vec<(&'static str, String)>,
}

// Pages are drawn on worker threads while the pane holds the document.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Doc>();
};

fn text(b: &[u8]) -> String {
    let s = if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        // PDFDocEncoding: the same as Latin-1 for everything people put in titles.
        b.iter().map(|&c| c as char).collect()
    };
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

pub fn open(path: &Path) -> Result<Doc, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't read it: {e}"))?.len();
    if size > MAX_FILE {
        return Err(format!("Too big to show every page ({})", crate::scan::human(size)));
    }
    let data = std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    open_bytes(data)
}

/// A PDF already in memory (one saved inside another document, for instance).
pub fn open_bytes(data: Vec<u8>) -> Result<Doc, String> {
    let pdf = Pdf::new(data).map_err(|e| match e {
        hayro::hayro_syntax::LoadPdfError::Decryption(_) => "This PDF is password-protected.".to_string(),
        _ => "This PDF couldn't be read (it may be damaged).".to_string(),
    })?;
    let sizes: Vec<[f32; 2]> = pdf.pages().iter().map(|p| {
        let (w, h) = p.render_dimensions();
        [w.max(1.0), h.max(1.0)]
    }).collect();
    if sizes.is_empty() {
        return Err("This PDF has no pages.".into());
    }
    let m = pdf.metadata();
    let mut facts = Vec::new();
    for (k, v) in [("Title", &m.title), ("Author", &m.author), ("Subject", &m.subject)] {
        if let Some(t) = v.as_deref().map(text).filter(|t| !t.is_empty()) {
            facts.push((k, t));
        }
    }
    let n = sizes.len();
    facts.push(("Pages", if n == 1 { "1 page".into() } else { format!("{n} pages") }));
    let [w, h] = sizes[0];
    facts.push(("Page size", paper(w, h)));
    if let Some(t) = m.creator.as_deref().map(text).filter(|t| !t.is_empty()) {
        facts.push(("Made with", t));
    }
    Ok(Doc { pdf, sizes, facts })
}

/// "A4", "Letter", or the size in millimetres.
fn paper(w: f32, h: f32) -> String {
    let (a, b) = (w.min(h), w.max(h));
    let near = |x: f32, y: f32| (a - x).abs() < 3.0 && (b - y).abs() < 3.0;
    let named = [("A3", 842.0, 1191.0), ("A4", 595.0, 842.0), ("A5", 420.0, 595.0), ("Letter", 612.0, 792.0), ("Legal", 612.0, 1008.0), ("Tabloid", 792.0, 1224.0)]
        .iter()
        .find(|(_, x, y)| near(*x, *y))
        .map(|(n, ..)| *n);
    let mm = format!("{:.0} × {:.0} mm", w / 72.0 * 25.4, h / 72.0 * 25.4);
    match named {
        Some(n) => format!("{n}{}", if w > h { ", landscape" } else { "" }),
        None => mm,
    }
}

impl Doc {
    /// Page `i` (from 0) drawn `width` pixels across: RGBA, with its size.
    pub fn page(&self, i: usize, width: u32) -> Result<(usize, usize, Vec<u8>), String> {
        let page = self.pdf.pages().get(i).ok_or("No such page")?;
        let (w, h) = page.render_dimensions();
        let scale = (width as f32 / w).min(MAX_SIDE / w).min(MAX_SIDE / h).max(0.01);
        let px = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let cache = RenderCache::new();
            hayro::render(page, &cache, &InterpreterSettings::default(), &RenderSettings::default(), &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE })
        }))
        .map_err(|_| "This page couldn't be drawn.".to_string())?;
        let (pw, ph) = (px.width() as usize, px.height() as usize);
        // vello_cpu's pixels are premultiplied; the page is drawn on white, so they're opaque.
        let rgba = px.data_as_u8_slice().to_vec();
        Ok((pw, ph, rgba))
    }
}

/// The first page, no larger than `max` across or down: for thumbnails and tiles.
pub fn first_page(path: &Path, max: u32) -> Result<(usize, usize, Vec<u8>), String> {
    open(path)?.first(max)
}

impl Doc {
    /// The first page, no larger than `max` across or down.
    pub fn first(&self, max: u32) -> Result<(usize, usize, Vec<u8>), String> {
        let [w, h] = self.sizes[0];
        let width = if h > w { (max as f32 * w / h).round().max(1.0) as u32 } else { max };
        self.page(0, width)
    }
}

/// The words on a PDF's pages, as Ask AI reads them.
pub struct Text {
    pub text: String,
    /// Pages read, from the first, and pages in the document.
    pub read: usize,
    pub pages: usize,
}

/// Collects the characters of each glyph drawn, with where it was drawn, to put them back
/// into words and lines. Nothing is drawn.
#[derive(Default)]
struct Glyphs {
    out: String,
    /// Where the last glyph's ink ended, its baseline and its height, in page points.
    last: Option<(f64, f64, f64)>,
}

impl<'a> hayro::hayro_interpret::Device<'a> for Glyphs {
    fn draw_path(&mut self, _: &hayro::hayro_interpret::kurbo::BezPath, _: hayro::hayro_interpret::DrawProps<'a>, _: &hayro::hayro_interpret::DrawMode) {}
    fn push_clip_path(&mut self, _: &hayro::hayro_interpret::ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<hayro::hayro_interpret::SoftMask<'a>>, _: hayro::hayro_interpret::BlendMode) {}
    fn draw_image(&mut self, _: hayro::hayro_interpret::Image<'a, '_>, _: hayro::hayro_interpret::ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_glyph_run(&mut self, run: &hayro::hayro_interpret::font::GlyphRun<'_, 'a>, props: hayro::hayro_interpret::DrawProps<'a>, _: &hayro::hayro_interpret::DrawMode) {
        use hayro::hayro_interpret::kurbo::Point;
        if run.writing_mode() != hayro::hayro_interpret::hayro_cmap::WritingMode::Horizontal {
            // Vertical writing: the characters in order, without guessing at lines.
            self.out.extend(run.glyphs().iter().filter_map(|g| g.as_unicode()).map(|u| match u {
                hayro::hayro_interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                hayro::hayro_interpret::hayro_cmap::BfString::String(s) => s,
            }));
            return;
        }
        for g in run.glyphs() {
            let t = props.transform * g.transform();
            let at = t * Point::ZERO;
            // A glyph is 1000 units high; how tall that is on the page.
            let size = (t * Point::new(0.0, 1000.0) - at).hypot().max(0.5);
            let ch = match g.as_unicode() {
                Some(hayro::hayro_interpret::hayro_cmap::BfString::Char(c)) => c.to_string(),
                Some(hayro::hayro_interpret::hayro_cmap::BfString::String(s)) => s,
                None => continue,
            };
            if let Some((x, y, h)) = self.last {
                let h = h.max(size);
                if (at.y - y).abs() > h * 0.5 {
                    if !self.out.ends_with('\n') {
                        self.out.push('\n');
                    }
                } else if (at.x - x > h * 0.2 || at.x < x - h) && !self.out.ends_with([' ', '\n']) && !ch.starts_with(' ') {
                    self.out.push(' ');
                }
            }
            self.out.extend(ch.chars().filter(|c| !c.is_control()));
            // Where the glyph's ink ends (glyphs don't carry their widths here). A gap of a
            // fifth of an em or more before the next one is a space between words.
            let right = match &**g {
                hayro::hayro_interpret::font::Glyph::Outline(o) => {
                    use hayro::hayro_interpret::kurbo::Shape;
                    let b = o.outline().bounding_box();
                    if b.width() > 0.0 { b.x1 } else { 250.0 }
                }
                hayro::hayro_interpret::font::Glyph::Type3(_) => 600.0,
            };
            self.last = Some(((t * Point::new(right, 0.0)).x, at.y, size));
        }
    }
}

/// The text of the PDF at `path`, page by page from the first, until there are `chars`
/// characters. Scanned pages (pictures of text) have none.
pub fn read_text(path: &Path, chars: usize) -> Result<Text, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't read it: {e}"))?.len();
    if size > MAX_FILE {
        return Err(format!("too large to read ({})", crate::scan::human(size)));
    }
    let data = std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    let pdf = Pdf::new(data).map_err(|e| match e {
        hayro::hayro_syntax::LoadPdfError::Decryption(_) => "password-protected".to_string(),
        _ => "couldn't be read (it may be damaged)".to_string(),
    })?;
    let pages = pdf.pages();
    let mut out = Text { text: String::new(), read: 0, pages: pages.len() };
    let cache = hayro::hayro_interpret::InterpreterCache::new();
    for page in pages.iter() {
        if out.text.chars().count() >= chars {
            break;
        }
        let mut dev = Glyphs::default();
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (w, h) = page.render_dimensions();
            let mut ctx = hayro::hayro_interpret::Context::new(
                page.initial_transform(true).to_kurbo(),
                hayro::hayro_interpret::kurbo::Rect::new(0.0, 0.0, w as f64, h as f64),
                &cache,
                page.xref(),
                InterpreterSettings::default(),
            );
            hayro::hayro_interpret::interpret_page(page, &mut ctx, &mut dev);
        }))
        .is_ok();
        out.read += 1;
        if ok {
            let t = dev.out.trim();
            if !t.is_empty() {
                if !out.text.is_empty() {
                    out.text.push_str("\n\n");
                }
                out.text.push_str(&format!("[Page {}]\n{t}", out.read));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A one-page PDF with a filled square, written by hand (offsets worked out below).
    pub(crate) fn tiny() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>",
            "<< /Length 30 >>\nstream\n1 0 0 rg 0 0 100 100 re f\n\nendstream",
            "<< /Title (Test page) >>",
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let x = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).bytes());
        out
    }

    /// A PDF with one line of Helvetica text on each page, written by hand.
    pub(crate) fn with_text(pages: &[&str]) -> Vec<u8> {
        let n = pages.len();
        // 1 catalog, 2 pages, 3 font, then a page and its contents for each.
        let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + i * 2)).collect();
        let mut objs = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        for (i, t) in pages.iter().enumerate() {
            objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 5 + i * 2));
            let body = format!("BT /F1 14 Tf 20 150 Td ({t}) Tj 0 -20 Td (Line two here) Tj ET");
            objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let x = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).bytes());
        out
    }

    #[test]
    fn reads_text_in_lines_and_words() {
        let dir = std::env::temp_dir().join(format!("filescape-pdftext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.pdf");
        std::fs::write(&p, with_text(&["Hello brave world", "Page two"])).unwrap();
        let t = read_text(&p, 10_000).unwrap();
        assert_eq!((t.read, t.pages), (2, 2));
        assert_eq!(t.text, "[Page 1]\nHello brave world\nLine two here\n\n[Page 2]\nPage two\nLine two here");
        // Stops after the page that reaches the budget.
        let t = read_text(&p, 5).unwrap();
        assert_eq!((t.read, t.pages), (1, 2));
        // No text: a page that only draws.
        std::fs::write(&p, tiny()).unwrap();
        assert_eq!(read_text(&p, 100).unwrap().text, "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn draws_pages() {
        let dir = std::env::temp_dir().join(format!("filescape-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.pdf");
        std::fs::write(&p, tiny()).unwrap();
        let doc = open(&p).unwrap();
        assert_eq!(doc.sizes, vec![[200.0, 100.0]]);
        assert!(doc.facts.contains(&("Title", "Test page".into())));
        let (w, h, px) = doc.page(0, 400).unwrap();
        assert_eq!((w, h), (400, 200));
        // Left half red, right half the white page.
        assert_eq!(&px[(100 * w + 100) * 4..][..4], &[255, 0, 0, 255]);
        assert_eq!(&px[(100 * w + 300) * 4..][..4], &[255, 255, 255, 255]);
        let (w, h, _) = first_page(&p, 100).unwrap();
        assert_eq!((w, h), (100, 50));
        std::fs::write(&p, b"%PDF-1.4 nothing").unwrap();
        assert!(open(&p).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn paper_sizes() {
        assert_eq!(paper(595.0, 842.0), "A4");
        assert_eq!(paper(792.0, 612.0), "Letter, landscape");
        assert_eq!(paper(200.0, 100.0), "71 × 35 mm");
    }
}
