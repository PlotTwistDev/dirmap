//! Fonts: a specimen drawn in the font itself (ab_glyph), and its names from the font's own
//! tables (ttf-parser). TrueType, OpenType and collections of them.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use std::path::Path;

/// Larger files aren't read.
pub const MAX_FILE: u64 = 64 << 20;

/// What a font says about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct About {
    pub family: String,
    pub style: String,
    pub glyphs: u16,
    /// Fonts in the file (a .ttc collection holds several; the first is shown).
    pub faces: u32,
    pub variable: bool,
}

impl About {
    /// "Inter Bold · 2,548 glyphs · variable · 1 of 4 in the collection"
    pub fn line(&self) -> String {
        let mut s = format!("{} {}", self.family, self.style).trim().to_string();
        s.push_str(&format!(" · {} glyphs", self.glyphs));
        if self.variable {
            s.push_str(" · variable");
        }
        if self.faces > 1 {
            s.push_str(&format!(" · the first of {} in the collection", self.faces));
        }
        s
    }
}

/// A picture of the font: RGBA, width, height.
pub struct Specimen {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub about: About,
}

/// Draw a specimen `max` pixels wide: the name and sample lines for previews, a large "Aa"
/// for small pictures (tiles and icons). Runs on worker threads.
pub fn specimen(path: &Path, max: u32) -> Result<Specimen, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't open it: {e}"))?.len();
    if size > MAX_FILE {
        return Err(format!("Too big to preview ({}).", crate::scan::human(size)));
    }
    let data = std::fs::read(path).map_err(|e| format!("Couldn't open it: {e}"))?;
    specimen_data(&data, max)
}

pub fn specimen_data(data: &[u8], max: u32) -> Result<Specimen, String> {
    let faces = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    let face = ttf_parser::Face::parse(data, 0).map_err(|e| format!("Couldn't read the font: {e}"))?;
    let name = |ids: &[u16]| {
        ids.iter().find_map(|id| face.names().into_iter().filter(|n| n.name_id == *id && n.is_unicode()).find_map(|n| n.to_string())).unwrap_or_default()
    };
    use ttf_parser::name_id;
    let about = About {
        family: name(&[name_id::TYPOGRAPHIC_FAMILY, name_id::FAMILY]),
        style: name(&[name_id::TYPOGRAPHIC_SUBFAMILY, name_id::SUBFAMILY]),
        glyphs: face.number_of_glyphs(),
        faces,
        variable: face.is_variable(),
    };
    let font = FontRef::try_from_slice_and_index(data, 0).map_err(|_| "Couldn't read the font's outlines".to_string())?;
    // A font with no Latin letters (symbols, other scripts): show the characters it has.
    let latin = font.glyph_id('a').0 != 0 && font.glyph_id('A').0 != 0;
    let own: String = if latin {
        String::new()
    } else {
        let mut chars = Vec::new();
        if let Some(cmap) = face.tables().cmap {
            for t in cmap.subtables.into_iter().filter(|t| t.is_unicode()) {
                t.codepoints(|c| {
                    if chars.len() < 400 && let Some(ch) = char::from_u32(c) && !ch.is_control() && !ch.is_whitespace() {
                        chars.push(ch);
                    }
                });
            }
        }
        chars.sort_unstable();
        chars.dedup();
        chars.into_iter().filter(|c| font.glyph_id(*c).0 != 0).take(60).collect()
    };
    let w = max.max(64);
    let small = w < 600;
    let lines: Vec<(String, f32)> = if small {
        let aa = if latin { "Aa".to_string() } else { own.chars().take(2).collect() };
        vec![(aa, w as f32 * 0.42), (about.family.clone(), w as f32 * 0.09)]
    } else if latin {
        // Roughly square, so it reads in a narrow pane too.
        let u = w as f32 / 800.0;
        [
            (format!("{} {}", about.family, about.style).trim().to_string(), 44.0),
            ("AaBbCcDdEeFf".into(), 80.0),
            ("ABCDEFGHIJKLM".into(), 44.0),
            ("NOPQRSTUVWXYZ".into(), 44.0),
            ("abcdefghijklm".into(), 44.0),
            ("nopqrstuvwxyz".into(), 44.0),
            ("0123456789 &@?!".into(), 44.0),
            ("The quick brown fox".into(), 30.0),
            ("jumps over the lazy dog.".into(), 30.0),
            ("The quick brown fox jumps over the lazy dog.".into(), 16.0),
        ]
        .into_iter()
        .map(|(t, px)| (t, px * u))
        .collect()
    } else {
        let u = w as f32 / 800.0;
        let chars: Vec<char> = own.chars().collect();
        chars.chunks(10).map(|c| (c.iter().map(|ch| format!("{ch} ")).collect::<String>(), 44.0 * u)).collect()
    };
    let pad = (w as f32 * 0.04).max(4.0);
    let heights: Vec<f32> = lines.iter().map(|(_, px)| font.as_scaled(PxScale::from(*px)).height() * 1.15).collect();
    let h = if small { w } else { (pad * 2.0 + heights.iter().sum::<f32>()).ceil() as u32 };
    // Narrow fonts don't leave a wide empty margin.
    let widest = lines.iter().map(|(t, px)| {
        let sc = font.as_scaled(PxScale::from(*px));
        t.chars().map(|c| sc.h_advance(font.glyph_id(c))).sum::<f32>()
    });
    let w = if small { w } else { ((widest.fold(0.0, f32::max) + pad * 2.0).ceil() as u32).clamp(w / 3, w) };
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    let mut y = pad;
    for ((text, px), lh) in lines.iter().zip(&heights) {
        let scaled = font.as_scaled(PxScale::from(*px));
        let line_w: f32 = text.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum();
        // Small pictures centre their lines, down the middle; previews start at the left.
        let mut x = if small { ((w as f32 - line_w) / 2.0).max(pad / 2.0) } else { pad };
        if small && text == &lines[0].0 {
            y = h as f32 * 0.12;
        }
        let baseline = y + scaled.ascent();
        let mut prev = None;
        for c in text.chars() {
            let id = font.glyph_id(c);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            prev = Some(id);
            if let Some(g) = font.outline_glyph(id.with_scale_and_position(*px, point(x, baseline))) {
                let b = g.px_bounds();
                g.draw(|gx, gy, cov| {
                    let (px_x, px_y) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if px_x >= 0 && px_y >= 0 && (px_x as u32) < w && (px_y as u32) < h {
                        let i = ((px_y as u32 * w + px_x as u32) * 4) as usize;
                        let ink = 30.0;
                        for ch in &mut rgba[i..i + 3] {
                            *ch = (*ch as f32 * (1.0 - cov) + ink * cov).round() as u8;
                        }
                    }
                });
            }
            x += scaled.h_advance(id);
        }
        y += lh;
    }
    Ok(Specimen { width: w, height: h, rgba, about })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font that's sure to be there: the one egui is built with.
    fn some_font() -> Vec<u8> {
        let fonts = eframe::egui::FontDefinitions::default();
        let (_, data) = fonts.font_data.iter().find(|(n, _)| n.contains("Ubuntu")).or(fonts.font_data.iter().next()).unwrap();
        data.font.to_vec()
    }

    #[test]
    fn specimen_and_names() {
        let data = some_font();
        let s = specimen_data(&data, 800).unwrap();
        assert!(!s.about.family.is_empty() && s.about.glyphs > 100);
        assert!(s.width <= 800 && s.width > 300);
        assert_eq!(s.rgba.len(), (s.width * s.height * 4) as usize);
        assert!(s.rgba.chunks_exact(4).any(|p| p[0] < 100), "some ink");
        let t = specimen_data(&data, 256).unwrap();
        assert_eq!((t.width, t.height), (256, 256));
        assert!(specimen_data(b"not a font", 256).is_err());
        assert!(s.about.line().contains("glyphs"));
    }
}
