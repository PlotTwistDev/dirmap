//! PDF pages drawn with hayro, a PDF renderer written in Rust (MIT or Apache-2.0), so every page
//! can be shown on every system without a PDF library to ship. Documents hayro can't open
//! (password-protected ones, damaged ones) fall back to the system's picture of the first page.

use hayro::hayro_interpret::InterpreterSettings;
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
