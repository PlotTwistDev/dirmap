//! Photoshop documents (.psd, .psb): the flattened picture Photoshop saves after the layers
//! ("Maximize Compatibility", on by default), read here, or else the small thumbnail kept among
//! the image resources.
//!
//! The reader is our own and small: it walks the five sections, unpacks the composite's
//! channels (raw or PackBits) row by row and keeps only every n-th pixel when a smaller picture
//! is wanted, so a huge document never sits in memory whole. RGB, greyscale, CMYK, indexed and
//! bitmap documents at 1, 8, 16 and 32 bits.

use std::path::Path;

/// Pictures with more pixels than this aren't read (beyond their thumbnail).
const MAX_PIXELS: u64 = 1 << 30;

pub struct PsdPicture {
    pub image: image::RgbaImage,
    /// The document's own size (the picture may be smaller).
    pub size: [u32; 2],
    /// Only the thumbnail could be shown.
    pub thumbnail_only: bool,
}

struct Header {
    psb: bool,
    channels: usize,
    height: usize,
    width: usize,
    depth: usize,
    mode: u16,
}

fn be(data: &[u8], at: usize, n: usize) -> Option<u64> {
    data.get(at..at + n).map(|b| b.iter().fold(0u64, |v, x| v << 8 | *x as u64))
}

fn header(data: &[u8]) -> Option<Header> {
    if !data.starts_with(b"8BPS") {
        return None;
    }
    let version = be(data, 4, 2)?;
    Some(Header {
        psb: version == 2,
        channels: be(data, 12, 2)? as usize,
        height: be(data, 14, 4)? as usize,
        width: be(data, 18, 4)? as usize,
        depth: be(data, 22, 2)? as usize,
        mode: be(data, 24, 2)? as u16,
    })
}

/// Where the colour mode data, image resources and the composite image start.
fn sections(data: &[u8], h: &Header) -> Option<(usize, usize, usize)> {
    let cm = 26;
    let res = cm + 4 + be(data, cm, 4)? as usize;
    let layers = res + 4 + be(data, res, 4)? as usize;
    let len_size = if h.psb { 8 } else { 4 };
    let image = layers + len_size + be(data, layers, len_size)? as usize;
    Some((cm, res, image))
}

/// The thumbnail resource (1036, or 1033 in old files): a JPEG after a 28-byte header.
fn thumbnail(data: &[u8], res: usize) -> Option<image::RgbaImage> {
    let end = (res + 4 + be(data, res, 4)? as usize).min(data.len());
    let mut i = res + 4;
    while i + 12 <= end && &data[i..i + 4] == b"8BIM" {
        let id = be(data, i + 4, 2)?;
        let name_len = data[i + 6] as usize;
        // A Pascal name, padded to an even length (counting its length byte).
        let j = i + 6 + (1 + name_len).next_multiple_of(2);
        let size = be(data, j, 4)? as usize;
        let body = j + 4;
        if (id == 1036 || id == 1033) && body + 28 <= end {
            let jpeg = data.get(body + 28..(body + size).min(end))?;
            let mut img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?.to_rgba8();
            if id == 1033 {
                // Very old thumbnails are stored BGR.
                img.pixels_mut().for_each(|p| p.0.swap(0, 2));
            }
            return Some(img);
        }
        i = body + size.next_multiple_of(2);
    }
    None
}

/// Unpack one PackBits row into `out`.
fn packbits(src: &[u8], out: &mut Vec<u8>, want: usize) {
    out.clear();
    let mut i = 0;
    while i < src.len() && out.len() < want {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let k = n as usize + 1;
            out.extend_from_slice(&src[i..(i + k).min(src.len())]);
            i += k;
        } else if n != -128 {
            let k = (-(n as isize)) as usize + 1;
            if let Some(b) = src.get(i) {
                out.extend(std::iter::repeat_n(*b, k));
            }
            i += 1;
        }
    }
    out.resize(want, 0);
}

/// The composite, keeping every `step`-th pixel across and down.
fn composite(data: &[u8], h: &Header, at: usize, cm: usize, step: usize) -> Option<image::RgbaImage> {
    let (w, ht, depth) = (h.width, h.height, h.depth);
    if !matches!(depth, 1 | 8 | 16 | 32) || w == 0 || ht == 0 {
        return None;
    }
    let base = match h.mode {
        0 | 1 | 2 => 1, // bitmap, greyscale, indexed
        3 => 3,         // RGB
        4 => 4,         // CMYK
        _ => return None,
    };
    let used = base.min(h.channels);
    if used < base {
        return None;
    }
    let row_bytes = (w * depth).div_ceil(8);
    let compression = be(data, at, 2)?;
    let mut pos = at + 2;
    // Each channel's rows: where each starts and how long it is.
    let rows = h.channels * ht;
    let mut spans = Vec::with_capacity(rows);
    match compression {
        0 => (0..rows).for_each(|r| spans.push((pos + r * row_bytes, row_bytes))),
        1 => {
            let n = if h.psb { 4 } else { 2 };
            let mut start = pos + rows * n;
            for r in 0..rows {
                let len = be(data, pos + r * n, n)? as usize;
                spans.push((start, len));
                start += len;
            }
            pos = start;
            let _ = pos;
        }
        _ => return None, // ZIP: only in layers, not in the composite
    }
    let (ow, oh) = (w.div_ceil(step), ht.div_ceil(step));
    let mut out = image::RgbaImage::new(ow as u32, oh as u32);
    let mut planes = vec![vec![0u8; ow]; used];
    let mut row = Vec::with_capacity(row_bytes);
    let palette = (h.mode == 2).then(|| data.get(cm + 4..cm + 4 + 768)).flatten();
    for (oy, y) in (0..ht).step_by(step).enumerate() {
        for (c, plane) in planes.iter_mut().enumerate() {
            let (s, len) = spans[c * ht + y];
            let src = data.get(s..s + len)?;
            if compression == 1 {
                packbits(src, &mut row, row_bytes);
            } else {
                row.clear();
                row.extend_from_slice(src);
            }
            for (ox, x) in (0..w).step_by(step).enumerate() {
                plane[ox] = match depth {
                    1 => if row[x / 8] >> (7 - x % 8) & 1 == 1 { 0 } else { 255 },
                    8 => row[x],
                    16 => row[x * 2],
                    _ => (f32::from_be_bytes([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]]).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8,
                };
            }
        }
        for ox in 0..ow {
            let px = match h.mode {
                3 => [planes[0][ox], planes[1][ox], planes[2][ox]],
                4 => {
                    // Stored inverted: 255 is no ink.
                    let k = planes[3][ox] as u32;
                    let ch = |v: u8| (v as u32 * k / 255) as u8;
                    [ch(planes[0][ox]), ch(planes[1][ox]), ch(planes[2][ox])]
                }
                2 => {
                    let i = planes[0][ox] as usize;
                    match palette {
                        Some(p) => [p[i], p[256 + i], p[512 + i]],
                        None => [planes[0][ox]; 3],
                    }
                }
                _ => [planes[0][ox]; 3],
            };
            out.put_pixel(ox as u32, oy as u32, image::Rgba([px[0], px[1], px[2], 255]));
        }
    }
    Some(out)
}

/// The picture in a document, about `max` across when `max` > 0. Runs on worker threads.
pub fn picture(path: &Path, max: u32) -> Result<PsdPicture, String> {
    let data = std::fs::read(path).map_err(|e| format!("Couldn't open it: {e}"))?;
    picture_data(&data, max)
}

pub fn picture_data(data: &[u8], max: u32) -> Result<PsdPicture, String> {
    let h = header(data).ok_or(if data.starts_with(b"8BPS") { "This document is damaged." } else { "Not a Photoshop document." })?;
    let size = [h.width as u32, h.height as u32];
    let (cm, res, at) = sections(data, &h).ok_or("This document is damaged.")?;
    let big = h.width.max(h.height);
    // A small picture: the thumbnail if it's big enough.
    if max > 0 && let Some(t) = thumbnail(data, res).filter(|t| t.width().max(t.height()) >= max.min(big as u32)) {
        return Ok(PsdPicture { image: t, size, thumbnail_only: false });
    }
    let target = if max > 0 { max as usize } else { 2048 };
    let step = (big / target).max(1);
    let full = if (h.width as u64) * (h.height as u64) <= MAX_PIXELS { composite(data, &h, at, cm, step) } else { None };
    match full {
        Some(img) => Ok(PsdPicture { image: img, size, thumbnail_only: false }),
        None => thumbnail(data, res)
            .map(|t| PsdPicture { image: t, size, thumbnail_only: true })
            .ok_or_else(|| "This document has no picture Filescape can show (save it with Maximize Compatibility).".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×1 RGB, 8-bit PSD: red, blue; raw or PackBits; with an optional thumbnail.
    fn tiny_psd(rle: bool, with_thumb: Option<&[u8]>) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(b"8BPS");
        d.extend(1u16.to_be_bytes());
        d.extend([0u8; 6]);
        d.extend(3u16.to_be_bytes());
        d.extend(1u32.to_be_bytes());
        d.extend(2u32.to_be_bytes());
        d.extend(8u16.to_be_bytes());
        d.extend(3u16.to_be_bytes());
        d.extend(0u32.to_be_bytes());
        let mut res = Vec::new();
        if let Some(jpeg) = with_thumb {
            res.extend(b"8BIM");
            res.extend(1036u16.to_be_bytes());
            res.extend([0u8, 0]);
            let body_len = 28 + jpeg.len();
            res.extend((body_len as u32).to_be_bytes());
            res.extend([0u8; 28]);
            res.extend(jpeg);
            if body_len % 2 == 1 {
                res.push(0);
            }
        }
        d.extend((res.len() as u32).to_be_bytes());
        d.extend(res);
        // A layer section with something in it, skipped by its length.
        d.extend(6u32.to_be_bytes());
        d.extend([1, 2, 3, 4, 5, 6]);
        let planes: [[u8; 2]; 3] = [[255, 0], [0, 0], [0, 255]];
        if rle {
            d.extend(1u16.to_be_bytes());
            // Each row: "copy 2 bytes" (1, a, b) = 3 bytes.
            for _ in 0..3 {
                d.extend(3u16.to_be_bytes());
            }
            for p in planes {
                d.extend([1, p[0], p[1]]);
            }
        } else {
            d.extend(0u16.to_be_bytes());
            planes.iter().for_each(|p| d.extend(p));
        }
        d
    }

    #[test]
    fn composite_and_thumbnail() {
        for rle in [false, true] {
            let d = tiny_psd(rle, None);
            let p = picture_data(&d, 0).unwrap();
            assert_eq!(p.size, [2, 1]);
            assert_eq!(p.image.get_pixel(0, 0).0, [255, 0, 0, 255], "rle {rle}");
            assert_eq!(p.image.get_pixel(1, 0).0, [0, 0, 255, 255], "rle {rle}");
        }
        let mut out = Vec::new();
        packbits(&[0xfd, 7, 1, 9], &mut out, 5);
        assert_eq!(out, [7, 7, 7, 7, 9], "a run, then one literal");
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 2, image::Rgb([0, 200, 0]))).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        let d = tiny_psd(true, Some(&jpeg));
        let (_, res, _) = sections(&d, &header(&d).unwrap()).unwrap();
        assert_eq!(thumbnail(&d, res).unwrap().width(), 4);
        assert!(picture_data(b"GIF89a", 0).is_err());
    }
}
