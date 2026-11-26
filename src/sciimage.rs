//! Scientific pictures: DICOM (medical scans) and FITS (astronomy). Read here, without
//! outside libraries.
//!
//! DICOM: uncompressed pictures and baseline-JPEG ones, grey or colour, with the scan's window
//! (brightness and contrast) applied. Patient details (name, ID, birth date and the like) are
//! never shown or kept. FITS: the first picture in the file, stretched so faint detail shows.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Files larger than this aren't read whole (DICOM) or at all.
const FILE_MAX: u64 = 512 << 20;
/// Pictures with more pixels than this aren't decoded.
const PIXELS_MAX: usize = 64 << 20;

/// A picture to show, and what the file says about it.
pub struct Sci {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub facts: Vec<(&'static str, String)>,
}

pub fn is_fits(path: &Path) -> bool {
    path.extension().is_some_and(|e| matches!(e.to_string_lossy().to_ascii_lowercase().as_str(), "fits" | "fit" | "fts"))
}

pub fn read(path: &Path) -> Result<Sci, String> {
    if is_fits(path) { fits(path) } else { dicom(path) }
}

fn grey_to_rgba(v: &[u8]) -> Vec<u8> {
    v.iter().flat_map(|&g| [g, g, g, 255]).collect()
}

// ---- DICOM ----

/// One data element's tag and value bytes.
struct Elem<'a> {
    tag: (u16, u16),
    vr: [u8; 2],
    value: &'a [u8],
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    explicit: bool,
    big: bool,
}

impl<'a> Reader<'a> {
    fn u16(&self, at: usize) -> Option<u16> {
        let s = self.b.get(at..at + 2)?;
        Some(if self.big { u16::from_be_bytes([s[0], s[1]]) } else { u16::from_le_bytes([s[0], s[1]]) })
    }
    fn u32(&self, at: usize) -> Option<u32> {
        let s = self.b.get(at..at + 4)?;
        Some(if self.big { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) })
    }

    /// The next element at the top level. Sequences and items of undefined length are skipped
    /// over by their delimiters.
    fn next(&mut self) -> Option<Elem<'a>> {
        let p = self.pos;
        let tag = (self.u16(p)?, self.u16(p + 2)?);
        // Group 2 (the file meta information) is always explicit little-endian.
        let (explicit, big) = if tag.0 == 2 { (true, false) } else { (self.explicit, self.big) };
        let saved = (self.explicit, self.big);
        self.explicit = explicit;
        self.big = big;
        // Some files say explicit but write an element without its VR: read that one implicitly.
        let explicit = explicit && self.b.get(p + 4..p + 6).is_some_and(|v| v.iter().all(u8::is_ascii_uppercase));
        let (vr, len, head) = if explicit {
            let vr = [*self.b.get(p + 4)?, *self.b.get(p + 5)?];
            if matches!(&vr, b"OB" | b"OW" | b"OF" | b"SQ" | b"UT" | b"UN" | b"OD" | b"OL" | b"OV" | b"UC" | b"UR" | b"SV" | b"UV") {
                (vr, self.u32(p + 8)?, 12)
            } else {
                (vr, self.u16(p + 6)? as u32, 8)
            }
        } else {
            (*b"??", self.u32(p + 4)?, 8)
        };
        self.explicit = saved.0;
        self.big = saved.1;
        let start = p + head;
        if len == 0xFFFF_FFFF {
            // Undefined length: a sequence, or encapsulated (compressed) pixel data. Find the
            // matching delimiter, counting nested ones.
            let end = self.skip_undefined(start)?;
            self.pos = end;
            return Some(Elem { tag, vr, value: &self.b[start..end.min(self.b.len())] });
        }
        let end = start.checked_add(len as usize)?;
        if end > self.b.len() {
            // Cut short: take what there is.
            self.pos = self.b.len();
            return Some(Elem { tag, vr, value: &self.b[start.min(self.b.len())..] });
        }
        self.pos = end;
        Some(Elem { tag, vr, value: &self.b[start..end] })
    }

    /// Where an undefined-length value ends, after its sequence delimiter (FFFE,E0DD).
    fn skip_undefined(&self, mut at: usize) -> Option<usize> {
        let mut depth = 1;
        while at + 8 <= self.b.len() {
            let (g, e) = (self.u16(at)?, self.u16(at + 2)?);
            if g == 0xFFFE {
                let len = self.u32(at + 4)?;
                match e {
                    0xE0DD => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(at + 8);
                        }
                        at += 8;
                    }
                    0xE00D => at += 8,
                    _ if len == 0xFFFF_FFFF => at += 8, // an item of undefined length: walk into it
                    _ => at += 8 + len as usize,
                }
                continue;
            }
            // An element inside an item: step over it the same way as at the top level.
            let mut r = Reader { b: self.b, pos: at, explicit: self.explicit, big: self.big };
            let before = at;
            // A nested undefined-length sequence is handled by `next` recursively.
            r.next()?;
            at = r.pos;
            if at <= before {
                return None;
            }
        }
        Some(self.b.len())
    }
}

fn text(v: &[u8]) -> String {
    String::from_utf8_lossy(v).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string()
}

/// The first number in a value like `"40\\400"`.
fn number(v: &[u8]) -> Option<f64> {
    text(v).split('\\').next()?.trim().parse().ok()
}

fn ts_name(uid: &str) -> (&'static str, bool, bool, bool) {
    // name, explicit VR, big-endian, readable here
    match uid {
        "1.2.840.10008.1.2" => ("Implicit VR little-endian", false, false, true),
        "1.2.840.10008.1.2.1" => ("Explicit VR little-endian", true, false, true),
        "1.2.840.10008.1.2.2" => ("Explicit VR big-endian", true, true, true),
        "1.2.840.10008.1.2.1.99" => ("Deflated explicit VR", true, false, false),
        "1.2.840.10008.1.2.4.50" => ("JPEG baseline", true, false, true),
        "1.2.840.10008.1.2.4.51" => ("JPEG extended", true, false, true),
        "1.2.840.10008.1.2.4.57" | "1.2.840.10008.1.2.4.70" => ("JPEG lossless", true, false, true),
        "1.2.840.10008.1.2.4.80" | "1.2.840.10008.1.2.4.81" => ("JPEG-LS", true, false, true),
        "1.2.840.10008.1.2.4.90" | "1.2.840.10008.1.2.4.91" => ("JPEG 2000", true, false, true),
        "1.2.840.10008.1.2.4.201" | "1.2.840.10008.1.2.4.202" | "1.2.840.10008.1.2.4.203" => ("High-throughput JPEG 2000", true, false, false),
        "1.2.840.10008.1.2.5" => ("RLE", true, false, true),
        _ => ("Other", true, false, false),
    }
}

fn modality(code: &str) -> String {
    let name = match code {
        "CT" => "CT scan",
        "MR" => "MRI scan",
        "US" => "Ultrasound",
        "CR" | "DX" => "X-ray",
        "MG" => "Mammogram",
        "PT" => "PET scan",
        "NM" => "Nuclear medicine",
        "XA" => "X-ray angiography",
        "RF" => "Fluoroscopy",
        "OT" => "Other",
        "SC" => "Secondary capture",
        "IO" => "Dental X-ray",
        "OP" => "Eye photograph",
        "ES" => "Endoscopy",
        _ => return code.to_string(),
    };
    format!("{name} ({code})")
}

pub fn dicom(path: &Path) -> Result<Sci, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > FILE_MAX {
        return Err(format!("Too big to read ({})", crate::scan::human(len)));
    }
    let mut b = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(FILE_MAX).read_to_end(&mut b)).map_err(|e| e.to_string())?;
    dicom_bytes(&b)
}

fn dicom_bytes(b: &[u8]) -> Result<Sci, String> {
    dicom_parse(b, false)
}

/// `inflated`: the data set after the meta group has been inflated already (deflated files).
fn dicom_parse(b: &[u8], inflated: bool) -> Result<Sci, String> {
    // A 128-byte preamble then "DICM"; very old files start straight with elements, explicit
    // VR when the bytes after the first tag are letters, big-endian when its group reads so.
    let start = if b.get(128..132) == Some(b"DICM") { 132 } else { 0 };
    let (explicit, big) = if start == 132 {
        (true, false)
    } else {
        (b.get(4..6).is_some_and(|v| v.iter().all(u8::is_ascii_uppercase)), b.first() == Some(&0) && b.get(1).is_some_and(|&g| g != 0))
    };
    let mut r = Reader { b, pos: start, explicit, big };
    let mut get: std::collections::HashMap<(u16, u16), (&[u8], [u8; 2])> = std::collections::HashMap::new();
    let mut pixel: Option<(&[u8], [u8; 2])> = None;
    let mut switched = false;
    loop {
        let before = r.pos;
        let Some(e) = r.next() else { break };
        // After the meta group, the transfer syntax says how the rest is written: if it differs,
        // this element is read again the new way.
        if e.tag.0 > 2 && !switched {
            switched = true;
            if let Some((v, _)) = get.get(&(0x0002, 0x0010)) {
                let ts = text(v);
                // Deflated: the rest is one raw deflate stream. Inflate it and read again.
                if ts == "1.2.840.10008.1.2.1.99" && !inflated {
                    let mut out = b[..before].to_vec();
                    std::io::Read::take(flate2::read::DeflateDecoder::new(&b[before..]), FILE_MAX).read_to_end(&mut out).map_err(|e| format!("Couldn't inflate it: {e}"))?;
                    return dicom_parse(&out, true);
                }
                let (_, explicit, big, _) = ts_name(&ts);
                if explicit != r.explicit || big != r.big {
                    r.explicit = explicit;
                    r.big = big;
                    r.pos = before;
                    continue;
                }
            }
        }
        if e.tag == (0x7FE0, 0x0010) {
            pixel = Some((e.value, e.vr));
            break;
        }
        // Patient details (group 0010) are never kept.
        if e.tag.0 != 0x0010 {
            get.insert(e.tag, (e.value, e.vr));
        }
        if get.len() > 100_000 {
            break;
        }
    }
    if start == 0 && get.is_empty() {
        return Err("Not a DICOM file".into());
    }
    let s = |g: u16, e: u16| get.get(&(g, e)).map(|v| text(v.0)).filter(|t| !t.is_empty());
    let us = |g: u16, e: u16| get.get(&(g, e)).and_then(|v| v.0.get(..2)).map(|x| if r.big { u16::from_be_bytes([x[0], x[1]]) } else { u16::from_le_bytes([x[0], x[1]]) });
    let num = |g: u16, e: u16| get.get(&(g, e)).and_then(|v| number(v.0));
    let mut facts = Vec::new();
    if let Some(m) = s(0x0008, 0x0060) {
        facts.push(("Kind of scan", modality(&m)));
    }
    for (g, e, label) in [(0x0008, 0x1030, "Study"), (0x0008, 0x103E, "Series"), (0x0018, 0x0015, "Body part")] {
        if let Some(v) = s(g, e) {
            facts.push((label, v));
        }
    }
    if let Some(d) = s(0x0008, 0x0020).filter(|d| d.len() == 8) {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&d, "%Y%m%d") {
            facts.push(("Date", d.format("%-d %b %Y").to_string()));
        }
    }
    let maker = [s(0x0008, 0x0070), s(0x0008, 0x1090)].into_iter().flatten().collect::<Vec<_>>().join(" ");
    if !maker.is_empty() {
        facts.push(("Scanner", maker));
    }
    let ts = s(0x0002, 0x0010).unwrap_or_else(|| "1.2.840.10008.1.2".into());
    let (ts_label, _, _, readable) = ts_name(&ts);
    let readable = readable || (inflated && ts == "1.2.840.10008.1.2.1.99");
    facts.push(("Encoding", ts_label.to_string()));
    let (rows, cols) = (us(0x0028, 0x0010).unwrap_or(0) as usize, us(0x0028, 0x0011).unwrap_or(0) as usize);
    let frames = s(0x0028, 0x0008).and_then(|f| f.parse::<usize>().ok()).unwrap_or(1).max(1);
    if rows > 0 && cols > 0 {
        facts.push(("Size", format!("{cols} × {rows}")));
    }
    if frames > 1 {
        facts.push(("Frames", format!("{frames} (the first is shown)")));
    }
    if let Some(t) = num(0x0018, 0x0050) {
        facts.push(("Slice thickness", format!("{t} mm")));
    }
    facts.push(("Patient details", "Not shown".into()));
    let Some((data, _vr)) = pixel else { return Err("No picture in it".into()) };
    if rows == 0 || cols == 0 || rows * cols > PIXELS_MAX {
        return Err("The picture's size is missing or too big".into());
    }
    if !readable {
        return Err(format!("Its picture is compressed as {ts_label}, which Filescape can't decode yet"));
    }
    let spp = us(0x0028, 0x0002).unwrap_or(1).max(1) as usize;
    let photo = s(0x0028, 0x0004).unwrap_or_else(|| "MONOCHROME2".into());
    // JPEG baseline: the first fragment after the offset table is the first frame.
    if ts == "1.2.840.10008.1.2.4.50" {
        let frag = fragments(data, r.big).into_iter().find(|f| f.starts_with(&[0xFF, 0xD8])).ok_or("No picture in it")?;
        let img = image::load_from_memory_with_format(frag, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
        let mut rgba = img.to_rgba8();
        if photo == "MONOCHROME1" {
            rgba.pixels_mut().for_each(|p| p.0 = [255 - p.0[0], 255 - p.0[1], 255 - p.0[2], 255]);
        }
        let (w, h) = rgba.dimensions();
        return Ok(Sci { width: w, height: h, rgba: rgba.into_raw(), facts });
    }
    let bits = us(0x0028, 0x0100).unwrap_or(16) as usize;
    let signed = us(0x0028, 0x0103).unwrap_or(0) == 1;
    let stored = us(0x0028, 0x0101).map_or(bits, |b| b as usize).clamp(1, 32);
    let n = rows * cols;
    // JPEG extended and lossless, JPEG-LS and JPEG 2000: the first frame decoded.
    let mut decoded: Option<Vec<f64>> = None;
    if matches!(ts.as_str(), "1.2.840.10008.1.2.4.51" | "1.2.840.10008.1.2.4.57" | "1.2.840.10008.1.2.4.70" | "1.2.840.10008.1.2.4.80" | "1.2.840.10008.1.2.4.81" | "1.2.840.10008.1.2.4.90" | "1.2.840.10008.1.2.4.91") {
        let frame = first_frame(data, r.big).ok_or("No picture in it")?;
        let (w, h, chans, mut vals) = decode_frame(&ts, &frame, n)?;
        if w as usize * h as usize != n {
            return Err("The picture's size doesn't match the file's".into());
        }
        if chans == 3 {
            let max = vals.iter().cloned().fold(1.0f64, f64::max);
            let k = if max > 255.0 { 255.0 / max } else { 1.0 };
            let rgba = vals.chunks_exact(3).flat_map(|c| [(c[0] * k) as u8, (c[1] * k) as u8, (c[2] * k) as u8, 255]).collect();
            return Ok(Sci { width: w, height: h, rgba, facts });
        }
        if signed {
            let half = (1u64 << (stored - 1)) as f64;
            let jp2k = ts.ends_with(".90") || ts.ends_with(".91");
            if jp2k {
                // The JPEG 2000 decoder returns signed samples shifted up by half the range.
                vals.iter_mut().for_each(|v| *v -= half);
            } else {
                // The others return the stored bits: two's complement.
                vals.iter_mut().for_each(|v| if *v >= half { *v -= 2.0 * half });
            }
        }
        decoded = Some(vals);
    }
    // RLE: the first frame's segments unpacked into planes, then read as uncompressed planar
    // little-endian data.
    let unpacked;
    let (data, planar_rle) = if ts == "1.2.840.10008.1.2.5" {
        let frag = fragments(data, r.big).into_iter().skip(1).find(|f| f.len() >= 64).ok_or("No picture in it")?;
        unpacked = rle_frame(frag, n, bits / 8, spp)?;
        (&unpacked[..], true)
    } else {
        (data, false)
    };
    let big_px = r.big && !planar_rle;
    if decoded.is_none() && spp == 3 && bits == 8 {
        // Colour: interleaved, or planar when (0028,0006) is 1.
        if data.len() < n * 3 {
            return Err("The picture is cut short".into());
        }
        let planar = planar_rle || us(0x0028, 0x0006).unwrap_or(0) == 1;
        let rgba = (0..n)
            .flat_map(|i| if planar { [data[i], data[n + i], data[2 * n + i], 255] } else { [data[i * 3], data[i * 3 + 1], data[i * 3 + 2], 255] })
            .collect();
        return Ok(Sci { width: cols as u32, height: rows as u32, rgba, facts });
    }
    if decoded.is_none() && (spp != 1 || !(bits == 8 || bits == 16 || bits == 32)) {
        return Err(format!("{spp} samples of {bits} bits a pixel isn't supported yet"));
    }
    let need = n * bits / 8;
    if decoded.is_none() && data.len() < need {
        return Err("The picture is cut short".into());
    }
    let raw: Vec<f64> = if let Some(d) = decoded { d } else { (0..n)
        .map(|i| {
            if bits == 8 {
                data[i] as f64
            } else if bits == 16 {
                let x = [data[i * 2], data[i * 2 + 1]];
                let u = if big_px { u16::from_be_bytes(x) } else { u16::from_le_bytes(x) };
                if signed { u as i16 as f64 } else { u as f64 }
            } else {
                let x = [data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]];
                let u = if big_px { u32::from_be_bytes(x) } else { u32::from_le_bytes(x) };
                if signed { u as i32 as f64 } else { u as f64 }
            }
        })
        .collect() };
    let (slope, icpt) = (num(0x0028, 0x1053).unwrap_or(1.0), num(0x0028, 0x1052).unwrap_or(0.0));
    let vals: Vec<f64> = raw.iter().map(|v| v * slope + icpt).collect();
    // The scan's own window, or the range of its values.
    let (lo, hi) = match (num(0x0028, 0x1050), num(0x0028, 0x1051)) {
        (Some(c), Some(w)) if w > 1.0 => (c - w / 2.0, c + w / 2.0),
        _ => percentiles(&vals, 0.005, 0.995),
    };
    let span = (hi - lo).max(1e-9);
    let inv = photo == "MONOCHROME1";
    let grey: Vec<u8> = vals
        .iter()
        .map(|v| {
            let g = (((v - lo) / span).clamp(0.0, 1.0) * 255.0) as u8;
            if inv { 255 - g } else { g }
        })
        .collect();
    Ok(Sci { width: cols as u32, height: rows as u32, rgba: grey_to_rgba(&grey), facts })
}

/// The first frame's compressed bytes: fragments after the offset table, up to the end of
/// the first JPEG-family stream.
fn first_frame(data: &[u8], big: bool) -> Option<Vec<u8>> {
    let frags = fragments(data, big);
    let mut out = Vec::new();
    for f in frags.iter().skip(1) {
        out.extend_from_slice(f);
        let end = out.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        if out[..end].ends_with(&[0xFF, 0xD9]) {
            out.truncate(end);
            break;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// A JPEG 2000 file (.jp2 or a bare codestream) as a picture, refusing sides over `max`.
pub fn jpeg2000(bytes: &[u8], max: u32) -> Result<image::RgbaImage, String> {
    let img = hayro_jpeg2000::Image::new(bytes, &hayro_jpeg2000::DecodeSettings::default()).map_err(|e| format!("Couldn't read the image: {e:?}"))?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 || w > max || h > max {
        return Err("Too large to preview safely.".into());
    }
    let mut ctx = hayro_jpeg2000::DecoderContext::default();
    let dec = img.decode(&mut ctx).map_err(|e| format!("Couldn't read the image: {e:?}"))?;
    let comps = dec.components();
    let n = w as usize * h as usize;
    let get = |c: usize, i: usize| -> u8 {
        let comp = &comps[c.min(comps.len() - 1)];
        let max = ((1u32 << comp.bit_depth().clamp(1, 16)) - 1) as f32;
        (comp.samples().get(i).copied().unwrap_or(0.0) / max * 255.0).round().clamp(0.0, 255.0) as u8
    };
    if comps.is_empty() {
        return Err("Couldn't read the image".into());
    }
    let colour = comps.len() >= 3;
    let alpha = comps.len() == 2 || comps.len() >= 4;
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let (r, g, b) = if colour { (get(0, i), get(1, i), get(2, i)) } else { let v = get(0, i); (v, v, v) };
        let a = if alpha { get(comps.len() - 1, i) } else { 255 };
        out.extend_from_slice(&[r, g, b, a]);
    }
    image::RgbaImage::from_raw(w, h, out).ok_or_else(|| "Couldn't read the image".into())
}

/// A compressed frame decoded: width, height, channels (1 or 3) and samples, interleaved.
fn decode_frame(ts: &str, frame: &[u8], n: usize) -> Result<(u32, u32, usize, Vec<f64>), String> {
    let wrong = || "The picture's size doesn't match the file's".to_string();
    match ts.rsplit('.').next().unwrap_or("") {
        "90" | "91" => {
            let img = hayro_jpeg2000::Image::new(frame, &hayro_jpeg2000::DecodeSettings::default()).map_err(|e| format!("Couldn't decode its JPEG 2000 picture: {e:?}"))?;
            let (w, h) = (img.width(), img.height());
            // Checked before decoding: the decoder sets aside room for the size it's told.
            if w as usize * h as usize != n {
                return Err(wrong());
            }
            let mut ctx = hayro_jpeg2000::DecoderContext::default();
            let dec = img.decode(&mut ctx).map_err(|e| format!("Couldn't decode its JPEG 2000 picture: {e:?}"))?;
            let comps = dec.components();
            let n = w as usize * h as usize;
            let chans = if comps.len() >= 3 { 3 } else { 1 };
            let mut out = vec![0.0; n * chans];
            for (c, comp) in comps.iter().take(chans).enumerate() {
                for (i, v) in comp.samples().iter().take(n).enumerate() {
                    out[i * chans + c] = *v as f64;
                }
            }
            Ok((w, h, chans, out))
        }
        "80" | "81" => {
            // The frame header (SOF55) gives the size before anything is decoded.
            let sof = frame.windows(2).position(|w| w == [0xFF, 0xF7]).and_then(|i| frame.get(i + 5..i + 9));
            if sof.is_none_or(|s| u16::from_be_bytes([s[0], s[1]]) as usize * u16::from_be_bytes([s[2], s[3]]) as usize != n) {
                return Err(wrong());
            }
            let img = jpegls::decode_image(frame).map_err(|e| format!("Couldn't decode its JPEG-LS picture: {e}"))?;
            let n = img.width as usize * img.height as usize;
            let chans = if img.components >= 3 { 3 } else { 1 };
            let mut out = vec![0.0; n * chans];
            for c in 0..chans {
                for i in 0..n {
                    out[i * chans + c] = img.samples.get(c * n + i).copied().unwrap_or(0) as f64;
                }
            }
            Ok((img.width, img.height, chans, out))
        }
        // 12-bit and arithmetic-coded frames (JPEG extended): craft-jpeg.
        _ if crate::jpegx::needs(frame) => {
            let (w, h, c, v) = crate::jpegx::samples(frame)?;
            if w as usize * h as usize != n {
                return Err(wrong());
            }
            Ok((w, h, c, v))
        }
        _ => {
            let mut d = jpeg_decoder::Decoder::new(frame);
            let jpeg_err = |e: jpeg_decoder::Error| match e {
                jpeg_decoder::Error::Unsupported(jpeg_decoder::UnsupportedFeature::SamplePrecision(b)) => format!("Its picture is {b}-bit JPEG, which Filescape can't decode yet"),
                e => format!("Couldn't decode its JPEG picture: {e}"),
            };
            d.read_info().map_err(jpeg_err)?;
            if d.info().is_none_or(|i| i.width as usize * i.height as usize != n) {
                return Err(wrong());
            }
            let px = d.decode().map_err(jpeg_err)?;
            let info = d.info().ok_or("Couldn't decode its JPEG picture")?;
            let (w, h) = (info.width as u32, info.height as u32);
            Ok(match info.pixel_format {
                jpeg_decoder::PixelFormat::L8 => (w, h, 1, px.iter().map(|&v| v as f64).collect()),
                jpeg_decoder::PixelFormat::L16 => (w, h, 1, px.chunks_exact(2).map(|c| u16::from_ne_bytes([c[0], c[1]]) as f64).collect()),
                jpeg_decoder::PixelFormat::RGB24 => (w, h, 3, px.iter().map(|&v| v as f64).collect()),
                jpeg_decoder::PixelFormat::CMYK32 => return Err("Its picture is in CMYK, which isn't supported".into()),
            })
        }
    }
}

/// One RLE frame: a 64-byte header of segment offsets, then PackBits segments, one per byte of
/// each sample, most significant first. Returned as little-endian samples, plane by plane.
fn rle_frame(frag: &[u8], n: usize, bytes: usize, spp: usize) -> Result<Vec<u8>, String> {
    let le = |a: usize| frag.get(a..a + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let segs = le(0).ok_or("Damaged RLE data")?;
    if segs != bytes * spp || segs == 0 || segs > 15 {
        return Err("RLE data with an unexpected layout".into());
    }
    let offs: Vec<usize> = (0..segs).map(|i| le(4 + i * 4).unwrap_or(0)).collect();
    let mut out = vec![0u8; n * bytes * spp];
    for (k, &o) in offs.iter().enumerate() {
        let end = offs.get(k + 1).copied().unwrap_or(frag.len()).min(frag.len());
        let src = frag.get(o..end).ok_or("Damaged RLE data")?;
        // PackBits.
        let mut plane = Vec::with_capacity(n);
        let mut i = 0;
        while i < src.len() && plane.len() < n {
            let c = src[i] as i8;
            i += 1;
            if c >= 0 {
                let m = c as usize + 1;
                plane.extend_from_slice(&src[i..(i + m).min(src.len())]);
                i += m;
            } else if c != -128 {
                let m = (-(c as isize)) as usize + 1;
                if let Some(&v) = src.get(i) {
                    plane.extend(std::iter::repeat_n(v, m));
                }
                i += 1;
            }
        }
        plane.resize(n, 0);
        // Segment k is sample k / bytes, byte (bytes - 1 - k % bytes) of it (most significant
        // first); written little-endian, plane after plane.
        let (sample, byte) = (k / bytes, bytes - 1 - k % bytes);
        for (p, v) in plane.into_iter().enumerate() {
            out[sample * n * bytes + p * bytes + byte] = v;
        }
    }
    Ok(out)
}

/// The fragments of encapsulated pixel data: items (FFFE,E000) up to the delimiter.
fn fragments(data: &[u8], big: bool) -> Vec<&[u8]> {
    let rd = |a: usize| -> Option<u32> {
        let s = data.get(a..a + 4)?;
        Some(if big { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) })
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= data.len() {
        let (g, e) = (u16::from_le_bytes([data[at], data[at + 1]]), u16::from_le_bytes([data[at + 2], data[at + 3]]));
        let Some(len) = rd(at + 4) else { break };
        if g != 0xFFFE || e != 0xE000 {
            break;
        }
        let s = at + 8;
        let end = (s + len as usize).min(data.len());
        out.push(&data[s..end]);
        at = end;
    }
    out
}

/// The values at two fractions of the way through, sorted (on a sample for big pictures).
fn percentiles(v: &[f64], a: f64, b: f64) -> (f64, f64) {
    let step = (v.len() / 200_000).max(1);
    let mut s: Vec<f64> = v.iter().step_by(step).copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return (0.0, 1.0);
    }
    s.sort_by(|x, y| x.total_cmp(y));
    let at = |f: f64| s[((s.len() - 1) as f64 * f) as usize];
    (at(a), at(b))
}

// ---- FITS ----

/// One header: its cards as keyword → value (strings unquoted).
fn fits_header(f: &mut std::fs::File) -> Option<Vec<(String, String)>> {
    let mut cards = Vec::new();
    let mut block = [0u8; 2880];
    for _ in 0..1000 {
        f.read_exact(&mut block).ok()?;
        for card in block.chunks(80) {
            let card = String::from_utf8_lossy(card);
            let key = card.get(..8).unwrap_or("").trim().to_string();
            if key == "END" {
                return Some(cards);
            }
            if card.get(8..10) == Some("= ") {
                let raw = card[10..].trim();
                let value = if let Some(q) = raw.strip_prefix('\'') {
                    q.split('\'').next().unwrap_or("").trim().to_string()
                } else {
                    raw.split('/').next().unwrap_or("").trim().to_string()
                };
                cards.push((key, value));
            }
        }
    }
    None
}

pub fn fits(path: &Path) -> Result<Sci, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut facts = Vec::new();
    for hdu in 0..64 {
        let h = fits_header(&mut f).ok_or(if hdu == 0 { "Not a FITS file" } else { "No picture in it" })?;
        let get = |k: &str| h.iter().find(|c| c.0 == k).map(|c| c.1.as_str());
        let int = |k: &str| get(k).and_then(|v| v.parse::<i64>().ok());
        if hdu == 0 {
            if get("SIMPLE") != Some("T") {
                return Err("Not a FITS file".into());
            }
            for (k, label) in [("OBJECT", "Object"), ("TELESCOP", "Telescope"), ("INSTRUME", "Instrument"), ("FILTER", "Filter"), ("DATE-OBS", "Observed"), ("OBSERVER", "Observer")] {
                if let Some(v) = get(k).filter(|v| !v.is_empty()) {
                    facts.push((label, v.replace('T', " ")));
                }
            }
            if let Some(e) = get("EXPTIME").or(get("EXPOSURE")).and_then(|v| v.parse::<f64>().ok()) {
                facts.push(("Exposure", format!("{e} s")));
            }
        }
        let bitpix = int("BITPIX").ok_or("Damaged header")?;
        let naxis = int("NAXIS").unwrap_or(0).clamp(0, 999) as usize;
        let dims: Vec<usize> = (1..=naxis).map(|i| int(&format!("NAXIS{i}")).unwrap_or(0).max(0) as usize).collect();
        let bytes = (bitpix.unsigned_abs() / 8) as usize;
        let (pcount, gcount) = (int("PCOUNT").unwrap_or(0).max(0) as usize, int("GCOUNT").unwrap_or(1).max(1) as usize);
        let data_len = if naxis == 0 { 0 } else { bytes * gcount * (pcount + dims.iter().product::<usize>()) };
        let is_image = (hdu == 0 || get("XTENSION") == Some("IMAGE")) && naxis >= 2 && dims[0] > 0 && dims[1] > 0;
        if !is_image {
            let skip = data_len.div_ceil(2880) * 2880;
            f.seek(SeekFrom::Current(skip as i64)).map_err(|e| e.to_string())?;
            continue;
        }
        let (w, hgt) = (dims[0], dims[1]);
        if w * hgt > PIXELS_MAX {
            return Err("The picture is too big to show".into());
        }
        let planes: usize = dims[2..].iter().product::<usize>().max(1);
        facts.push(("Size", format!("{w} × {hgt}{}", if planes > 1 { format!(" × {planes}") } else { String::new() })));
        facts.push((
            "Values",
            match bitpix {
                8 => "8-bit whole numbers",
                16 => "16-bit whole numbers",
                32 => "32-bit whole numbers",
                64 => "64-bit whole numbers",
                -32 => "32-bit floating point",
                -64 => "64-bit floating point",
                _ => "Unknown",
            }
            .to_string(),
        ));
        let mut buf = vec![0u8; w * hgt * bytes];
        f.read_exact(&mut buf).map_err(|_| "The picture is cut short".to_string())?;
        let (bzero, bscale) = (get("BZERO").and_then(|v| v.parse().ok()).unwrap_or(0.0), get("BSCALE").and_then(|v| v.parse().ok()).unwrap_or(1.0));
        let vals: Vec<f64> = buf
            .chunks_exact(bytes)
            .map(|c| {
                let v = match bitpix {
                    8 => c[0] as f64,
                    16 => i16::from_be_bytes([c[0], c[1]]) as f64,
                    32 => i32::from_be_bytes([c[0], c[1], c[2], c[3]]) as f64,
                    64 => i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f64,
                    -32 => f32::from_be_bytes([c[0], c[1], c[2], c[3]]) as f64,
                    _ => f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]),
                };
                v * bscale + bzero
            })
            .collect();
        // Faint things show: clip the extremes, then an asinh stretch.
        let (lo, hi) = percentiles(&vals, 0.0025, 0.9985);
        let span = (hi - lo).max(1e-12);
        let k = 10.0f64;
        let norm = k.asinh();
        // FITS rows run bottom up.
        let mut grey = vec![0u8; w * hgt];
        for y in 0..hgt {
            for x in 0..w {
                let v = vals[y * w + x];
                let t = if v.is_finite() { ((v - lo) / span).clamp(0.0, 1.0) } else { 0.0 };
                grey[(hgt - 1 - y) * w + x] = ((k * t).asinh() / norm * 255.0) as u8;
            }
        }
        facts.push(("Shown", "Stretched so faint detail shows".into()));
        return Ok(Sci { width: w as u32, height: hgt as u32, rgba: grey_to_rgba(&grey), facts });
    }
    Err("No picture in it".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(tag: (u16, u16), vr: &[u8; 2], v: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(tag.0.to_le_bytes());
        out.extend(tag.1.to_le_bytes());
        out.extend(vr);
        if matches!(vr, b"OB" | b"OW" | b"SQ" | b"UN" | b"UT") {
            out.extend([0, 0]);
            out.extend((v.len() as u32).to_le_bytes());
        } else {
            out.extend((v.len() as u16).to_le_bytes());
        }
        out.extend(v);
        out
    }

    fn fact<'a>(s: &'a Sci, k: &str) -> Option<&'a str> {
        s.facts.iter().find(|f| f.0 == k).map(|f| f.1.as_str())
    }

    #[test]
    fn dicom_explicit_little_endian() {
        let mut b = vec![0u8; 128];
        b.extend(b"DICM");
        b.extend(el((0x0002, 0x0010), b"UI", b"1.2.840.10008.1.2.1\0"));
        b.extend(el((0x0008, 0x0020), b"DA", b"20261126"));
        b.extend(el((0x0008, 0x0060), b"CS", b"CT"));
        b.extend(el((0x0008, 0x0070), b"LO", b"Acme"));
        // A sequence of undefined length with one item, to be skipped.
        b.extend([0x08, 0x00, 0x15, 0x11, b'S', b'Q', 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        b.extend([0xFE, 0xFF, 0x00, 0xE0, 0xFF, 0xFF, 0xFF, 0xFF]);
        b.extend(el((0x0008, 0x1150), b"UI", b"1.2\0"));
        b.extend([0xFE, 0xFF, 0x0D, 0xE0, 0, 0, 0, 0]);
        b.extend([0xFE, 0xFF, 0xDD, 0xE0, 0, 0, 0, 0]);
        b.extend(el((0x0010, 0x0010), b"PN", b"Doe^Jane"));
        b.extend(el((0x0028, 0x0002), b"US", &1u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0004), b"CS", b"MONOCHROME2 "));
        b.extend(el((0x0028, 0x0010), b"US", &2u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0011), b"US", &2u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0100), b"US", &16u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0103), b"US", &1u16.to_le_bytes()));
        b.extend(el((0x0028, 0x1050), b"DS", b"0 "));
        b.extend(el((0x0028, 0x1051), b"DS", b"200 "));
        b.extend(el((0x0028, 0x1052), b"DS", b"-100"));
        b.extend(el((0x0028, 0x1053), b"DS", b"1 "));
        let px: Vec<u8> = [0i16, 100, 200, 300].iter().flat_map(|v| v.to_le_bytes()).collect();
        b.extend(el((0x7FE0, 0x0010), b"OW", &px));
        let s = dicom_bytes(&b).unwrap();
        assert_eq!(fact(&s, "Kind of scan"), Some("CT scan (CT)"));
        assert_eq!(fact(&s, "Date"), Some("26 Nov 2026"));
        assert_eq!(fact(&s, "Scanner"), Some("Acme"));
        assert_eq!(fact(&s, "Size"), Some("2 × 2"));
        assert!(!s.facts.iter().any(|f| f.1.contains("Doe")));
        // -100 → black, 0 → middle, 100 and up → white (window 0 ± 100).
        let g: Vec<u8> = s.rgba.chunks(4).map(|c| c[0]).collect();
        assert_eq!(g[0], 0);
        assert!((g[1] as i32 - 127).abs() <= 1, "{g:?}");
        assert_eq!(&g[2..], &[255, 255]);
    }

    #[test]
    fn dicom_compressed_says_so_and_junk_is_refused() {
        let mut b = vec![0u8; 128];
        b.extend(b"DICM");
        b.extend(el((0x0002, 0x0010), b"UI", b"1.2.840.10008.1.2.4.201\0"));
        b.extend(el((0x0028, 0x0010), b"US", &2u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0011), b"US", &2u16.to_le_bytes()));
        b.extend([0xE0, 0x7F, 0x10, 0x00, b'O', b'B', 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        b.extend([0xFE, 0xFF, 0x00, 0xE0, 0, 0, 0, 0]);
        b.extend([0xFE, 0xFF, 0xDD, 0xE0, 0, 0, 0, 0]);
        let e = dicom_bytes(&b).err().unwrap();
        assert!(e.contains("JPEG 2000"), "{e}");
        assert!(dicom_bytes(b"hello").is_err());
        for n in 0..b.len() {
            let _ = dicom_bytes(&b[..n]);
        }
    }

    #[test]
    fn dicom_twelve_bit_jpeg() {
        let jpeg = crate::jpegx::tests::twelve_bit();
        let mut b = vec![0u8; 128];
        b.extend(b"DICM");
        b.extend(el((0x0002, 0x0010), b"UI", b"1.2.840.10008.1.2.4.51\0"));
        b.extend(el((0x0028, 0x0002), b"US", &1u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0004), b"CS", b"MONOCHROME2 "));
        b.extend(el((0x0028, 0x0010), b"US", &8u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0011), b"US", &8u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0100), b"US", &16u16.to_le_bytes()));
        b.extend(el((0x0028, 0x0101), b"US", &12u16.to_le_bytes()));
        // Encapsulated: an empty offset table, then the frame.
        b.extend([0xE0, 0x7F, 0x10, 0x00, b'O', b'B', 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        b.extend([0xFE, 0xFF, 0x00, 0xE0, 0, 0, 0, 0]);
        let mut frag = jpeg.clone();
        if frag.len() % 2 == 1 {
            frag.push(0);
        }
        b.extend([0xFE, 0xFF, 0x00, 0xE0]);
        b.extend((frag.len() as u32).to_le_bytes());
        b.extend(&frag);
        b.extend([0xFE, 0xFF, 0xDD, 0xE0, 0, 0, 0, 0]);
        let s = dicom_bytes(&b).unwrap();
        assert_eq!((s.width, s.height), (8, 8));
    }

    #[test]
    fn fits_image_in_an_extension() {
        let card = |s: &str| format!("{s:<80}");
        let block = |cards: &[String]| {
            let mut h: String = cards.concat();
            h.push_str(&card("END"));
            while h.len() % 2880 != 0 {
                h.push(' ');
            }
            h.into_bytes()
        };
        let mut b = block(&[card("SIMPLE  =                    T"), card("BITPIX  =                    8"), card("NAXIS   =                    0"), card("OBJECT  = 'M31     '"), card("EXPTIME =                 30.0 / seconds")]);
        b.extend(block(&[card("XTENSION= 'IMAGE   '"), card("BITPIX  =                  -32"), card("NAXIS   =                    2"), card("NAXIS1  =                    3"), card("NAXIS2  =                    2")]));
        let mut data: Vec<u8> = [0.0f32, 1.0, 2.0, 3.0, 4.0, 100.0].iter().flat_map(|v| v.to_be_bytes()).collect();
        data.resize(2880, 0);
        b.extend(data);
        let dir = std::env::temp_dir().join(format!("filescape-fits-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("m31.fits");
        std::fs::write(&p, &b).unwrap();
        let s = read(&p).unwrap();
        assert_eq!((s.width, s.height), (3, 2));
        assert_eq!(fact(&s, "Object"), Some("M31"));
        assert_eq!(fact(&s, "Exposure"), Some("30 s"));
        assert_eq!(fact(&s, "Values"), Some("32-bit floating point"));
        // Bottom-up: the first row stored (0, 1, 2) is shown at the bottom, darker.
        let g: Vec<u8> = s.rgba.chunks(4).map(|c| c[0]).collect();
        assert!(g[3] < g[0], "{g:?}");
        std::fs::write(&p, b"SIMPLE  = F").unwrap();
        assert!(read(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Reads every file in `$FILESCAPE_SCI_DIR` and prints what happened: for checking against
/// real-world samples (pydicom's test files, for one).
#[cfg(test)]
#[test]
#[ignore]
fn read_sample_folder() {
    let Ok(dir) = std::env::var("FILESCAPE_SCI_DIR") else { return };
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    names.sort();
    for p in names {
        match read(&p) {
            Ok(s) => println!(
                "ok   {:50} {}x{} {:?} sum={}",
                p.file_name().unwrap().to_string_lossy(),
                s.width,
                s.height,
                s.facts.iter().find(|f| f.0 == "Encoding").map(|f| &f.1),
                s.rgba.iter().map(|&v| v as u64).sum::<u64>()
            ),
            Err(e) => println!("err  {:50} {e}", p.file_name().unwrap().to_string_lossy()),
        }
    }
}
