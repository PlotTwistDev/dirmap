//! Camera RAW files: the JPEG preview the camera stores inside every RAW (CR2, CR3, NEF, ARW,
//! DNG, RAF, ORF, RW2, PEF…), and the shot's details from its EXIF (kamadak-exif). The raw
//! sensor data isn't developed: the preview is what the camera showed on its screen.
//!
//! The previews are found by their JPEG signature, which works across makers without a
//! reader per format; each candidate's size is read from its header and the best one decoded.

use std::io::Cursor;
use std::path::Path;

/// Larger files aren't read.
pub const MAX_FILE: u64 = 400 << 20;

pub struct RawPreview {
    pub image: image::DynamicImage,
    /// Camera, lens, exposure, ISO, focal length, date.
    pub facts: Vec<(&'static str, String)>,
}

/// Each embedded JPEG: (offset, width, height).
fn candidates(data: &[u8]) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(k) = data[i..].windows(3).position(|w| w == [0xff, 0xd8, 0xff]) {
        let at = i + k;
        let dims = image::codecs::jpeg::JpegDecoder::new(Cursor::new(&data[at..])).ok().map(|d| image::ImageDecoder::dimensions(&d));
        if let Some((w, h)) = dims.filter(|(w, h)| *w >= 64 && *h >= 64 && *w <= 16_384 && *h <= 16_384) {
            out.push((at, w, h));
        }
        i = at + 3;
    }
    out
}

/// The preview `max` pixels across or larger (the largest when `max` is 0), turned the way the
/// camera was held. Runs on worker threads.
pub fn preview(path: &Path, max: u32) -> Result<RawPreview, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't open it: {e}"))?.len();
    if size > MAX_FILE {
        return Err(format!("Too big to preview ({}).", crate::scan::human(size)));
    }
    let data = std::fs::read(path).map_err(|e| format!("Couldn't open it: {e}"))?;
    preview_data(&data, max)
}

pub fn preview_data(data: &[u8], max: u32) -> Result<RawPreview, String> {
    let mut found = candidates(data);
    if found.is_empty() {
        return Err("No preview picture inside this RAW file.".into());
    }
    // Smallest that's big enough first, then larger, then smaller ones.
    let big = |c: &(usize, u32, u32)| c.1.max(c.2);
    found.sort_by_key(|c| big(c));
    if max > 0 {
        found.sort_by_key(|c| (big(c) < max, if big(c) >= max { big(c) } else { u32::MAX - big(c) }));
    } else {
        found.reverse();
    }
    // EXIF: TIFF-based RAWs carry it at the start; others in the preview JPEG itself.
    let tiff = data.starts_with(b"II") || data.starts_with(b"MM");
    let mut exif = if tiff { exif::Reader::new().read_raw(data.to_vec()).ok() } else { None };
    for (at, _, _) in found {
        // Lossless JPEG (the raw data of some cameras) doesn't decode: try the next.
        let Ok(img) = image::load_from_memory_with_format(&data[at..], image::ImageFormat::Jpeg) else { continue };
        if exif.is_none() {
            exif = exif::Reader::new().read_from_container(&mut Cursor::new(&data[at..])).ok();
        }
        let orient = exif.as_ref().and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)).and_then(|f| f.value.get_uint(0)).unwrap_or(1);
        let img = match orient {
            2 => img.fliph(),
            3 => img.rotate180(),
            4 => img.flipv(),
            5 => img.rotate90().fliph(),
            6 => img.rotate90(),
            7 => img.rotate270().fliph(),
            8 => img.rotate270(),
            _ => img,
        };
        return Ok(RawPreview { image: img, facts: exif.as_ref().map(facts).unwrap_or_default() });
    }
    Err("The preview inside this RAW file couldn't be read.".into())
}

/// The shot's details, in words.
pub fn facts(e: &exif::Exif) -> Vec<(&'static str, String)> {
    use exif::Tag;
    let get = |t: Tag| e.get_field(t, exif::In::PRIMARY).map(|f| f.display_value().with_unit(e).to_string().trim_matches('"').trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    let camera = match (get(Tag::Make), get(Tag::Model)) {
        (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase().split(' ').next().unwrap_or("").to_string()) => Some(model),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (a, b) => a.or(b),
    };
    if let Some(c) = camera {
        out.push(("Camera", c));
    }
    if let Some(l) = get(Tag::LensModel) {
        out.push(("Lens", l));
    }
    let exposure: Vec<String> = [Tag::ExposureTime, Tag::FNumber, Tag::PhotographicSensitivity, Tag::FocalLength]
        .into_iter()
        .filter_map(|t| get(t).map(|v| if t == Tag::PhotographicSensitivity { format!("ISO {v}") } else { v }))
        .collect();
    if !exposure.is_empty() {
        out.push(("Exposure", exposure.join(" · ")));
    }
    if let Some(d) = get(Tag::DateTimeOriginal) {
        out.push(("Taken", d));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg).unwrap();
        out
    }

    #[test]
    fn finds_the_right_preview() {
        // A made-up RAW: junk, a small thumbnail, more junk, a large preview, junk.
        let mut raw = b"FAKERAW\0".to_vec();
        raw.extend(vec![7u8; 1000]);
        raw.extend(jpeg(160, 120));
        raw.extend([0xff, 0xd8, 0xff, 0x00, 1, 2, 3]); // a false start
        raw.extend(jpeg(1200, 800));
        raw.extend(vec![9u8; 500]);
        assert_eq!(preview_data(&raw, 0).unwrap().image.width(), 1200, "the largest for the pane");
        assert_eq!(preview_data(&raw, 128).unwrap().image.width(), 160, "the smallest that's big enough for a tile");
        assert_eq!(preview_data(&raw, 2000).unwrap().image.width(), 1200, "else the largest");
        assert!(preview_data(b"no pictures here", 0).is_err());
    }
}
