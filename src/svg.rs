//! SVG drawings, drawn here with resvg (pure Rust: no scripting, no network).
//!
//! Only what's inside the file is drawn: pictures it links to by path or URL are left out
//! (pictures embedded as `data:` URLs are drawn), so previewing a file can't read others.
//! Text uses the system's fonts, loaded once.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use resvg::{tiny_skia, usvg};

/// Larger files aren't drawn (a preview shouldn't parse tens of megabytes of XML).
pub const MAX_FILE: u64 = 16 << 20;

/// A drawn SVG: RGBA pixels (not premultiplied), their size, and the drawing's own size in
/// CSS pixels.
pub struct Drawn {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub natural: [f32; 2],
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// Draw an SVG (or .svgz) file so its longer side is `max` pixels, larger or smaller than its
/// own size. Runs on worker threads.
pub fn render(path: &Path, max: u32) -> Result<Drawn, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't open it: {e}"))?.len();
    if size > MAX_FILE {
        return Err(format!("Too big to draw ({}).", crate::scan::human(size)));
    }
    let data = std::fs::read(path).map_err(|e| format!("Couldn't open it: {e}"))?;
    render_data(&data, max)
}

pub fn render_data(data: &[u8], max: u32) -> Result<Drawn, String> {
    let mut opt = usvg::Options { fontdb: fonts(), ..Default::default() };
    // Embedded pictures only: never files or URLs the drawing points at.
    opt.image_href_resolver = usvg::ImageHrefResolver { resolve_data: usvg::ImageHrefResolver::default_data_resolver(), resolve_string: Box::new(|_, _| None) };
    let tree = usvg::Tree::from_data(data, &opt).map_err(|e| format!("Couldn't read the drawing: {e}"))?;
    let natural = tree.size();
    let (w, h) = (natural.width(), natural.height());
    let scale = max as f32 / w.max(h);
    let (pw, ph) = (((w * scale).round() as u32).clamp(1, max), ((h * scale).round() as u32).clamp(1, max));
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("The drawing has no size")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    // tiny-skia's pixels are premultiplied.
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Ok(Drawn { width: pw, height: ph, rgba, natural: [w, h] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_and_keeps_to_itself() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20" height="10">
            <rect width="20" height="10" fill="#ff0000"/>
            <image href="/etc/passwd" width="20" height="10"/>
            <image xlink:href="https://example.com/x.png" width="20" height="10"/>
        </svg>"##;
        let d = render_data(svg, 200).unwrap();
        assert_eq!((d.width, d.height), (200, 100), "drawn larger than its own size");
        assert_eq!(d.natural, [20.0, 10.0]);
        assert_eq!(&d.rgba[..4], &[255, 0, 0, 255]);
        assert!(render_data(b"<svg", 100).is_err());
        assert!(render_data(b"not svg at all", 100).is_err());
    }
}
