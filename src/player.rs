//! The sequence player's engine: reading frames (every image type dirmap previews, plus DPX),
//! a cache of decoded frames held to a memory budget, read-ahead on worker threads, and the
//! display transform.
//!
//! Frames are kept as half-float RGBA, whatever the file held, so one display path serves 8-bit
//! PNGs, 10-bit log DPX and float EXR alike. The workers decode the frames nearest the playhead
//! in the direction of play first; when the budget is full, the frame furthest ahead (which, when
//! looping, is the one just played) makes room for a nearer one. Playback only ever shows frames
//! already in memory, so it runs at the chosen rate once they're cached, and slows (never skips)
//! while they aren't.

use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

/// Files bigger than this aren't read as frames (a 16K float EXR is about 1.5 GB).
const FRAME_FILE_MAX: u64 = 2 << 30;
/// Biggest frame read, in pixels across or down.
const SIDE_MAX: u32 = 16_384;

/// What a frame's values mean, which decides how they're shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Encoding {
    /// Scene-linear light (float EXR, HDR, linear DPX).
    Linear,
    /// Already encoded for a display (8 and 16-bit PNG, JPEG, TIFF, video DPX).
    Display,
    /// Cineon/DPX printing-density log.
    Log,
}

/// A decoded frame: half-float RGBA bits, row by row from the top.
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u16>,
    pub enc: Encoding,
    /// The file's pixel format, for the info line ("RGB, 10-bit log DPX").
    pub info: String,
}

impl Frame {
    pub fn bytes(&self) -> usize {
        self.px.len() * 2
    }
}

/// Read and decode one frame, shrunk by `scale` (1 is full size, 2 half, …).
pub fn decode(path: &Path, scale: u32) -> Result<Frame, String> {
    let bytes = if crate::fsys::remote::is_remote(path) {
        crate::fsys::remote::read_bytes(path, FRAME_FILE_MAX).map_err(|e| format!("Couldn't read it: {e}"))?
    } else {
        let len = std::fs::metadata(path).map_err(|e| format!("Couldn't read it: {e}"))?.len();
        if len > FRAME_FILE_MAX {
            return Err("Too big to play.".into());
        }
        std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))?
    };
    let dpx = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dpx"));
    let (w, h, rgba, enc, info) = if dpx { dpx::decode(&bytes)? } else { decode_image(&bytes, path)? };
    let (w, h, rgba) = shrink(w, h, rgba, scale.max(1));
    let px = rgba.par_iter().map(|&v| half::f16::from_f32(v).to_bits()).collect();
    Ok(Frame { w, h, px, enc, info })
}

type Decoded = (u32, u32, Vec<f32>, Encoding, String);

fn decode_image(bytes: &[u8], path: &Path) -> Result<Decoded, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    if let Some(f) = path.extension().and_then(image::ImageFormat::from_extension) {
        reader.set_format(f);
    } else {
        reader = reader.with_guessed_format().map_err(|e| format!("Couldn't read it: {e}"))?;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(SIDE_MAX);
    limits.max_image_height = Some(SIDE_MAX);
    limits.max_alloc = Some(4 << 30);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| format!("Couldn't read the image: {e}"))?;
    use image::ColorType as C;
    let float = matches!(img.color(), C::Rgb32F | C::Rgba32F);
    let info = format!("{}{}", describe(img.color()), if float { ", linear" } else { "" });
    let (w, h) = (img.width(), img.height());
    let rgba = img.into_rgba32f().into_raw();
    Ok((w, h, rgba, if float { Encoding::Linear } else { Encoding::Display }, info))
}

fn describe(c: image::ColorType) -> &'static str {
    use image::ColorType as C;
    match c {
        C::L8 | C::La8 => "Grey, 8-bit",
        C::L16 | C::La16 => "Grey, 16-bit",
        C::Rgb8 | C::Rgba8 => "RGB, 8-bit",
        C::Rgb16 | C::Rgba16 => "RGB, 16-bit",
        C::Rgb32F | C::Rgba32F => "RGB, float",
        _ => "Colour",
    }
}

/// Average `scale` × `scale` blocks.
fn shrink(w: u32, h: u32, rgba: Vec<f32>, scale: u32) -> (u32, u32, Vec<f32>) {
    if scale <= 1 || w < scale * 4 || h < scale * 4 {
        return (w, h, rgba);
    }
    let (nw, nh) = (w / scale, h / scale);
    let s = scale as usize;
    let w = w as usize;
    let k = 1.0 / (s * s) as f32;
    let mut out = vec![0f32; nw as usize * nh as usize * 4];
    out.par_chunks_mut(nw as usize * 4).enumerate().for_each(|(y, row)| {
        for x in 0..nw as usize {
            let mut acc = [0f32; 4];
            for dy in 0..s {
                let base = ((y * s + dy) * w + x * s) * 4;
                for dx in 0..s {
                    for c in 0..4 {
                        acc[c] += rgba[base + dx * 4 + c];
                    }
                }
            }
            for c in 0..4 {
                row[x * 4 + c] = acc[c] * k;
            }
        }
    });
    (nw, nh, out)
}

pub mod dpx {
    //! DPX (SMPTE 268M): 8, 10, 12 and 16-bit, RGB, RGBA, ABGR and luma, either byte order,
    //! 10-bit packed or filled (method A or B). The first image element is read.

    use super::{Decoded, Encoding, SIDE_MAX};

    struct Rd<'a> {
        b: &'a [u8],
        big: bool,
    }

    impl Rd<'_> {
        fn u32(&self, at: usize) -> Result<u32, String> {
            let s: [u8; 4] = self.b.get(at..at + 4).and_then(|s| s.try_into().ok()).ok_or("The DPX header is cut short.")?;
            Ok(if self.big { u32::from_be_bytes(s) } else { u32::from_le_bytes(s) })
        }
        fn u16(&self, at: usize) -> Result<u16, String> {
            let s: [u8; 2] = self.b.get(at..at + 2).and_then(|s| s.try_into().ok()).ok_or("The DPX header is cut short.")?;
            Ok(if self.big { u16::from_be_bytes(s) } else { u16::from_le_bytes(s) })
        }
        fn u8(&self, at: usize) -> Result<u8, String> {
            self.b.get(at).copied().ok_or_else(|| "The DPX header is cut short.".into())
        }
    }

    pub fn decode(b: &[u8]) -> Result<Decoded, String> {
        let big = match b.get(0..4) {
            Some(b"SDPX") => true,
            Some(b"XPDS") => false,
            _ => return Err("This isn't a DPX file.".into()),
        };
        let r = Rd { b, big };
        let orient = r.u16(768)?;
        let (w, h) = (r.u32(772)?, r.u32(776)?);
        if w == 0 || h == 0 || w > SIDE_MAX || h > SIDE_MAX {
            return Err(format!("A DPX of {w} × {h} can't be played."));
        }
        let desc = r.u8(800)?;
        let transfer = r.u8(801)?;
        let bits = r.u8(803)?;
        let packing = r.u16(804)?;
        let encoding = r.u16(806)?;
        if encoding != 0 {
            return Err("Run-length encoded DPX files aren't supported.".into());
        }
        let off = match r.u32(808)? {
            0 | u32::MAX => r.u32(4)?,
            o => o,
        } as usize;
        // Channels, and where each lands in RGBA.
        let (map, name): (&[usize], &str) = match desc {
            50 => (&[0, 1, 2], "RGB"),
            51 => (&[0, 1, 2, 3], "RGBA"),
            52 => (&[3, 2, 1, 0], "ABGR"),
            1..=3 | 6 | 8 => (&[0], "Grey"),
            _ => return Err(format!("DPX image type {desc} isn't supported.")),
        };
        let n = map.len();
        let (w, h) = (w as usize, h as usize);
        let samples = w * n;
        let data = b.get(off..).ok_or("The DPX image data is missing.")?;
        let max = ((1u32 << bits.min(16)) - 1) as f32;
        // Each line's samples, as 0..=max integers.
        let mut line = vec![0u32; samples];
        let mut out = vec![1f32; w * h * 4];
        let mut pos = 0usize;
        for y in 0..h {
            pos = read_line(&r, data, pos, bits, packing, &mut line)?;
            let row_y = if orient == 2 || orient == 3 { h - 1 - y } else { y };
            let flip = orient == 1 || orient == 3;
            for x in 0..w {
                let ox = if flip { w - 1 - x } else { x };
                let o = (row_y * w + ox) * 4;
                let s = &line[x * n..x * n + n];
                if n == 1 {
                    let v = s[0] as f32 / max;
                    out[o] = v;
                    out[o + 1] = v;
                    out[o + 2] = v;
                } else {
                    for (k, &c) in map.iter().enumerate() {
                        out[o + c] = s[k] as f32 / max;
                    }
                }
            }
        }
        let enc = match transfer {
            1 | 3 => Encoding::Log,
            2 => Encoding::Linear,
            _ => Encoding::Display,
        };
        let what = match enc {
            Encoding::Log => "log",
            Encoding::Linear => "linear",
            Encoding::Display => "video",
        };
        Ok((w as u32, h as u32, out, enc, format!("{name}, {bits}-bit {what} DPX")))
    }

    /// Read one line of samples starting at byte `pos`; returns where the next line starts
    /// (lines start on a 32-bit boundary).
    fn read_line(r: &Rd, data: &[u8], pos: usize, bits: u8, packing: u16, line: &mut [u32]) -> Result<usize, String> {
        let short = || "The DPX image data is cut short.".to_string();
        let n = line.len();
        let word = |i: usize| -> Result<u32, String> {
            let s: [u8; 4] = data.get(pos + i * 4..pos + i * 4 + 4).and_then(|s| s.try_into().ok()).ok_or_else(short)?;
            Ok(if r.big { u32::from_be_bytes(s) } else { u32::from_le_bytes(s) })
        };
        let pad4 = |bytes: usize| pos + bytes.div_ceil(4) * 4;
        match (bits, packing) {
            (8, _) => {
                let s = data.get(pos..pos + n).ok_or_else(short)?;
                for (o, &v) in line.iter_mut().zip(s) {
                    *o = v as u32;
                }
                Ok(pad4(n))
            }
            (16, _) => {
                let s = data.get(pos..pos + n * 2).ok_or_else(short)?;
                for (o, c) in line.iter_mut().zip(s.chunks_exact(2)) {
                    *o = if r.big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) } as u32;
                }
                Ok(pad4(n * 2))
            }
            // Three 10-bit samples to a 32-bit word: method A pads the low two bits, B the high.
            (10, 1 | 2) => {
                let shifts: [u32; 3] = if packing == 1 { [22, 12, 2] } else { [20, 10, 0] };
                let words = n.div_ceil(3);
                for i in 0..words {
                    let v = word(i)?;
                    for (k, s) in shifts.iter().enumerate() {
                        if let Some(o) = line.get_mut(i * 3 + k) {
                            *o = (v >> s) & 0x3ff;
                        }
                    }
                }
                Ok(pos + words * 4)
            }
            // 12-bit in 16: method A pads the low four bits, B the high.
            (12, 1 | 2) => {
                let s = data.get(pos..pos + n * 2).ok_or_else(short)?;
                for (o, c) in line.iter_mut().zip(s.chunks_exact(2)) {
                    let v = if r.big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) } as u32;
                    *o = if packing == 1 { v >> 4 } else { v & 0xfff };
                }
                Ok(pad4(n * 2))
            }
            // Packed: samples one after another across 32-bit words, from the top bit.
            (10 | 12, 0) => {
                let b = bits as u64;
                let words = (n as u64 * b).div_ceil(32) as usize;
                let mut acc: u64 = 0;
                let mut have: u64 = 0;
                let mut next = 0;
                for o in line.iter_mut() {
                    while have < b {
                        acc = (acc << 32) | word(next)? as u64;
                        next += 1;
                        have += 32;
                    }
                    *o = ((acc >> (have - b)) & ((1 << b) - 1)) as u32;
                    have -= b;
                    acc &= (1u64 << have) - 1;
                }
                Ok(pos + words * 4)
            }
            _ => Err(format!("{bits}-bit DPX (packing {packing}) isn't supported.")),
        }
    }
}

/// How values are turned into what's shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum View {
    /// Linear frames through sRGB, log through the Cineon curve, the rest as stored.
    #[default]
    Auto,
    /// Linear light to an sRGB display.
    Srgb,
    /// Linear light to a Rec.709 (video) display.
    Rec709,
    /// Cineon log to linear, then sRGB.
    Log,
    /// The stored values, unchanged.
    Raw,
}

impl View {
    pub const ALL: [View; 5] = [View::Auto, View::Srgb, View::Rec709, View::Log, View::Raw];
    pub fn title(self) -> &'static str {
        match self {
            View::Auto => "Automatic",
            View::Srgb => "Linear to sRGB",
            View::Rec709 => "Linear to Rec.709",
            View::Log => "Cineon Log to sRGB",
            View::Raw => "Raw Values",
        }
    }
    /// What `Auto` means for frames of this encoding.
    pub fn resolve(self, enc: Encoding) -> View {
        match (self, enc) {
            (View::Auto, Encoding::Linear) => View::Srgb,
            (View::Auto, Encoding::Log) => View::Log,
            (View::Auto, Encoding::Display) => View::Raw,
            (v, _) => v,
        }
    }
}

pub fn srgb_curve(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

pub fn rec709_curve(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v < 0.018 { 4.5 * v } else { 1.099 * v.powf(0.45) - 0.099 }
}

/// Cineon printing density (0..1 of 10-bit code values) to linear, with black at 95 and white
/// at 685.
pub fn cineon_to_linear(v: f32) -> f32 {
    let black = 10f32.powf((95.0 - 685.0) * 0.002 / 0.6);
    (10f32.powf((v * 1023.0 - 685.0) * 0.002 / 0.6) - black) / (1.0 - black)
}

/// A table from every half-float value to a display byte, for one view and exposure (stops).
pub fn lut(view: View, stops: f32) -> Vec<u8> {
    let gain = 2f32.powf(stops);
    (0..=u16::MAX)
        .into_par_iter()
        .map(|bits| {
            let v = half::f16::from_bits(bits).to_f32();
            if !v.is_finite() {
                return if v == f32::INFINITY { 255 } else { 0 };
            }
            let d = match view {
                View::Raw | View::Auto => (v * gain).clamp(0.0, 1.0),
                View::Srgb => srgb_curve(v * gain),
                View::Rec709 => rec709_curve(v * gain),
                View::Log => srgb_curve(cineon_to_linear(v) * gain),
            };
            (d * 255.0).round() as u8
        })
        .collect()
}

/// A frame through a table, ready to show (alpha is ignored: frames are shown opaque).
pub fn to_display(f: &Frame, lut: &[u8]) -> Vec<eframe::egui::Color32> {
    f.px.par_chunks_exact(4).map(|p| eframe::egui::Color32::from_rgb(lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize])).collect()
}

/// A LUT from a `.cube` file (Adobe/Resolve format): 3D, or 1D per channel.
#[derive(Debug)]
pub struct Cube {
    pub title: String,
    /// Entries along each side (3D) or in all (1D).
    n: usize,
    three_d: bool,
    min: [f32; 3],
    max: [f32; 3],
    /// Red changing fastest, then green, then blue.
    data: Vec<[f32; 3]>,
}

impl Cube {
    pub fn parse(text: &str) -> Result<Cube, String> {
        let (mut n, mut three_d, mut title) = (0usize, true, String::new());
        let (mut min, mut max) = ([0.0f32; 3], [1.0f32; 3]);
        let mut data = Vec::new();
        for (no, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut w = line.split_whitespace();
            let first = w.next().unwrap_or("");
            let nums = |w: std::str::SplitWhitespace| -> Result<[f32; 3], String> {
                let v: Vec<f32> = w
                    .map(|x| x.parse::<f32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("Line {} isn't three numbers.", no + 1))?;
                v.try_into()
                    .map_err(|_| format!("Line {} isn't three numbers.", no + 1))
            };
            match first {
                "TITLE" => title = line[5..].trim().trim_matches('"').to_string(),
                "LUT_3D_SIZE" | "LUT_1D_SIZE" => {
                    three_d = first == "LUT_3D_SIZE";
                    n = w
                        .next()
                        .and_then(|x| x.parse().ok())
                        .filter(|&n| {
                            (2..=256).contains(&n) || (!three_d && (2..=65536).contains(&n))
                        })
                        .ok_or("The LUT size isn't one Filescape can use.")?;
                }
                "DOMAIN_MIN" => min = nums(w)?,
                "DOMAIN_MAX" => max = nums(w)?,
                "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                    let v: Vec<f32> = w.filter_map(|x| x.parse().ok()).collect();
                    if let [a, b] = v[..] {
                        min = [a; 3];
                        max = [b; 3];
                    }
                }
                _ if first.parse::<f32>().is_ok() => data.push(nums(line.split_whitespace())?),
                // Other keywords (from Resolve and others) don't change the table.
                _ => {}
            }
        }
        let want = if three_d { n * n * n } else { n };
        if n == 0 || data.len() != want {
            return Err(format!(
                "This isn't a complete .cube LUT ({} of {want} entries).",
                data.len()
            ));
        }
        Ok(Cube {
            title,
            n,
            three_d,
            min,
            max,
            data,
        })
    }

    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.n;
        let pos = |c: usize| {
            let v = ((rgb[c] - self.min[c]) / (self.max[c] - self.min[c]).max(1e-6))
                .clamp(0.0, 1.0)
                * (n - 1) as f32;
            let i = (v.floor() as usize).min(n - 2);
            (i, v - i as f32)
        };
        if !self.three_d {
            let mut out = [0.0; 3];
            for (c, o) in out.iter_mut().enumerate() {
                let (i, f) = pos(c);
                *o = self.data[i][c] * (1.0 - f) + self.data[i + 1][c] * f;
            }
            return out;
        }
        let ((r, fr), (g, fg), (b, fb)) = (pos(0), pos(1), pos(2));
        let at = |r: usize, g: usize, b: usize| self.data[r + g * n + b * n * n];
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        let c00 = lerp(at(r, g, b), at(r + 1, g, b), fr);
        let c10 = lerp(at(r, g + 1, b), at(r + 1, g + 1, b), fr);
        let c01 = lerp(at(r, g, b + 1), at(r + 1, g, b + 1), fr);
        let c11 = lerp(at(r, g + 1, b + 1), at(r + 1, g + 1, b + 1), fr);
        lerp(lerp(c00, c10, fg), lerp(c01, c11, fg), fb)
    }
}

/// A frame through a `.cube` LUT, after an exposure gain on the stored values.
pub fn to_display_cube(f: &Frame, cube: &Cube, stops: f32) -> Vec<eframe::egui::Color32> {
    let gain = 2f32.powf(stops);
    let h = |b: u16| half::f16::from_bits(b).to_f32() * gain;
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    f.px.par_chunks_exact(4)
        .map(|p| {
            let o = cube.apply([h(p[0]), h(p[1]), h(p[2])]);
            eframe::egui::Color32::from_rgb(byte(o[0]), byte(o[1]), byte(o[2]))
        })
        .collect()
}

/// The machine's memory, in bytes (0 when it can't be found).
pub fn total_ram() -> u64 {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("MemTotal:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()))
            .map_or(0, |kb| kb * 1024)
    }
    #[cfg(target_os = "macos")]
    {
        let mut v: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        let name = c"hw.memsize";
        // SAFETY: hw.memsize is a u64 and `len` says how much room there is.
        let r = unsafe { libc::sysctlbyname(name.as_ptr(), (&mut v as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0) };
        if r == 0 { v } else { 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: the struct is plain data with its length set, as the call needs.
        unsafe {
            let mut m: MEMORYSTATUSEX = std::mem::zeroed();
            m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut m) != 0 { m.ullTotalPhys } else { 0 }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        0
    }
}

/// The default cache budget: a quarter of the memory, at least 1 GB.
pub fn default_budget() -> u64 {
    match total_ram() {
        0 => 4 << 30,
        r => (r / 4).max(1 << 30),
    }
}

/// The frames of a sequence, one slot per frame number from first to last on the step; a
/// missing frame's slot is empty.
#[derive(Clone)]
pub struct Clip {
    pub label: String,
    pub first: u64,
    pub step: u64,
    pub slots: Vec<Option<PathBuf>>,
}

impl Clip {
    /// A clip from a sequence's file names (as listed in `dir`).
    pub fn from_names(label: String, dir: &Path, seq: &crate::sequences::Sequence, names: &[&str]) -> Clip {
        let step = seq.step.max(1);
        let n = ((seq.last.saturating_sub(seq.first)) / step + 1) as usize;
        let mut slots = vec![None; n.min(1_000_000)];
        for name in names {
            let Some(num) = name.strip_prefix(&*seq.prefix).and_then(|r| r.strip_suffix(&*seq.suffix)).and_then(|d| d.parse::<u64>().ok()) else { continue };
            if num < seq.first || !(num - seq.first).is_multiple_of(step) {
                continue;
            }
            if let Some(s) = slots.get_mut(((num - seq.first) / step) as usize) {
                *s = Some(dir.join(name));
            }
        }
        Clip { label, first: seq.first, step, slots }
    }

    pub fn frame_number(&self, k: usize) -> u64 {
        self.first + k as u64 * self.step
    }
}

/// Where a slot is.
#[derive(Clone, PartialEq, Debug)]
pub enum Slot {
    /// No file for this frame number.
    Missing,
    Ready,
    Loading,
    Waiting,
    Failed(String),
}

struct State {
    head: usize,
    dir: isize,
    budget: usize,
    bytes: usize,
    /// Bytes of the last frame decoded, to judge whether the next will fit.
    est: usize,
    frames: HashMap<usize, Arc<Frame>>,
    failed: HashMap<usize, String>,
    busy: HashSet<usize>,
    scale: u32,
    /// Bumped when the scale changes, so frames decoded at the old one are dropped.
    generation: u64,
    quit: bool,
}

struct Shared {
    st: Mutex<State>,
    cv: Condvar,
    slots: Vec<Option<PathBuf>>,
}

impl Shared {
    /// Distance of slot `k` ahead of the playhead in the direction of play, wrapping.
    fn ahead(st: &State, n: usize, k: usize) -> usize {
        let d = (k as isize - st.head as isize) * st.dir;
        d.rem_euclid(n as isize) as usize
    }

    /// The next frame to decode, making room for it if a frame further ahead can go.
    fn pick(&self, st: &mut State) -> Option<usize> {
        let n = self.slots.len();
        let mut held = 0usize;
        for d in 0..n {
            let k = (st.head as isize + d as isize * st.dir).rem_euclid(n as isize) as usize;
            if self.slots[k].is_none() || st.failed.contains_key(&k) {
                continue;
            }
            if let Some(f) = st.frames.get(&k) {
                held += f.bytes();
                continue;
            }
            if st.busy.contains(&k) {
                held += st.est;
                continue;
            }
            // Room for this frame: everything nearer is held, so make room from further ahead.
            if held + st.est > st.budget && d > 0 {
                return None;
            }
            let busy = st.busy.len() * st.est;
            while st.bytes + busy + st.est > st.budget {
                let far = st.frames.keys().copied().filter(|&j| Self::ahead(st, n, j) > d).max_by_key(|&j| Self::ahead(st, n, j));
                match far {
                    Some(j) => {
                        if let Some(f) = st.frames.remove(&j) {
                            st.bytes -= f.bytes();
                        }
                    }
                    None if d == 0 => break,
                    None => return None,
                }
            }
            return Some(k);
        }
        None
    }
}

/// Decodes a clip's frames on worker threads into a cache held to a budget.
pub struct Engine {
    sh: Arc<Shared>,
}

impl Engine {
    pub fn new(clip: &Clip, budget: u64, scale: u32) -> Engine {
        let sh = Arc::new(Shared {
            st: Mutex::new(State {
                head: 0,
                dir: 1,
                budget: budget as usize,
                bytes: 0,
                est: 0,
                frames: HashMap::new(),
                failed: HashMap::new(),
                busy: HashSet::new(),
                scale,
                generation: 0,
                quit: false,
            }),
            cv: Condvar::new(),
            slots: clip.slots.clone(),
        });
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(1).clamp(1, 8);
        for _ in 0..threads {
            let sh = sh.clone();
            std::thread::spawn(move || work(&sh));
        }
        Engine { sh }
    }

    /// Move the playhead (and the direction read-ahead goes).
    pub fn seek(&self, k: usize, dir: isize) {
        let mut st = self.sh.st.lock().unwrap();
        if st.head != k || st.dir != dir {
            st.head = k;
            st.dir = if dir < 0 { -1 } else { 1 };
            self.sh.cv.notify_all();
        }
    }

    pub fn frame(&self, k: usize) -> Option<Arc<Frame>> {
        self.sh.st.lock().unwrap().frames.get(&k).cloned()
    }

    pub fn slot(&self, k: usize) -> Slot {
        let st = self.sh.st.lock().unwrap();
        match self.sh.slots.get(k) {
            None | Some(None) => Slot::Missing,
            Some(Some(_)) if st.frames.contains_key(&k) => Slot::Ready,
            Some(Some(_)) => match st.failed.get(&k) {
                Some(e) => Slot::Failed(e.clone()),
                None if st.busy.contains(&k) => Slot::Loading,
                None => Slot::Waiting,
            },
        }
    }

    /// For the timeline: each slot's state, as (cached, missing or failed).
    pub fn map(&self) -> Vec<(bool, bool)> {
        let st = self.sh.st.lock().unwrap();
        self.sh.slots.iter().enumerate().map(|(k, s)| (st.frames.contains_key(&k), s.is_none() || st.failed.contains_key(&k))).collect()
    }

    /// Bytes held, and the budget.
    pub fn memory(&self) -> (u64, u64) {
        let st = self.sh.st.lock().unwrap();
        (st.bytes as u64, st.budget as u64)
    }

    /// Change the memory budget; frames beyond it go as the workers next make room.
    pub fn set_budget(&self, budget: u64) {
        let mut st = self.sh.st.lock().unwrap();
        st.budget = budget as usize;
        // Drop the frames furthest ahead until it fits.
        let n = self.sh.slots.len();
        while st.bytes > st.budget {
            let Some(j) = st.frames.keys().copied().max_by_key(|&j| Shared::ahead(&st, n, j)) else { break };
            if let Some(f) = st.frames.remove(&j) {
                st.bytes -= f.bytes();
            }
        }
        self.sh.cv.notify_all();
    }

    /// Decode at a different size, dropping what's cached.
    pub fn set_scale(&self, scale: u32) {
        let mut st = self.sh.st.lock().unwrap();
        if st.scale != scale {
            st.scale = scale;
            st.generation += 1;
            st.frames.clear();
            st.failed.clear();
            st.bytes = 0;
            st.est = 0;
            self.sh.cv.notify_all();
        }
    }

    /// Whether anything is still to be read into the cache.
    pub fn busy(&self) -> bool {
        !self.sh.st.lock().unwrap().busy.is_empty()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.sh.st.lock().unwrap().quit = true;
        self.sh.cv.notify_all();
    }
}

fn work(sh: &Shared) {
    loop {
        let (k, scale, generation) = {
            let mut st = sh.st.lock().unwrap();
            loop {
                if st.quit {
                    return;
                }
                if let Some(k) = sh.pick(&mut st) {
                    st.busy.insert(k);
                    break (k, st.scale, st.generation);
                }
                st = sh.cv.wait(st).unwrap();
            }
        };
        let Some(path) = sh.slots[k].as_ref() else { continue };
        let r = decode(path, scale);
        let mut st = sh.st.lock().unwrap();
        st.busy.remove(&k);
        if st.generation == generation {
            match r {
                Ok(f) => {
                    st.est = st.est.max(f.bytes());
                    st.bytes += f.bytes();
                    st.frames.insert(k, Arc::new(f));
                }
                Err(e) => {
                    st.failed.insert(k, e);
                }
            }
        }
        sh.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn dpx_file(big: bool, bits: u8, packing: u16, desc: u8, transfer: u8, w: u32, h: u32, data: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 2048];
        let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
        let put16 = |b: &mut Vec<u8>, at: usize, v: u16| b[at..at + 2].copy_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
        b[0..4].copy_from_slice(if big { b"SDPX" } else { b"XPDS" });
        put32(&mut b, 4, 2048);
        put16(&mut b, 768, 0);
        put16(&mut b, 770, 1);
        put32(&mut b, 772, w);
        put32(&mut b, 776, h);
        b[800] = desc;
        b[801] = transfer;
        b[803] = bits;
        put16(&mut b, 804, packing);
        put32(&mut b, 808, 2048);
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn dpx_10_bit_method_a_both_byte_orders() {
        // One pixel: R=1023, G=512, B=0; then R=0, G=0, B=1023.
        let words = [(1023u32 << 22) | (512 << 12), 1023 << 2];
        for big in [true, false] {
            let data: Vec<u8> = words.iter().flat_map(|w| if big { w.to_be_bytes() } else { w.to_le_bytes() }).collect();
            let f = dpx_file(big, 10, 1, 50, 3, 2, 1, &data);
            let (w, h, px, enc, info) = dpx::decode(&f).unwrap();
            assert_eq!((w, h, enc), (2, 1, Encoding::Log));
            assert!(info.contains("10-bit log"));
            assert_eq!(px[0], 1.0);
            assert!((px[1] - 512.0 / 1023.0).abs() < 1e-6);
            assert_eq!(&px[2..8], &[0.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
        }
    }

    #[test]
    fn dpx_10_bit_method_b_and_packed() {
        let w = (1023u32 << 20) | (1 << 10) | 7;
        let f = dpx_file(true, 10, 2, 50, 2, 1, 1, &w.to_be_bytes());
        let (_, _, px, enc, _) = dpx::decode(&f).unwrap();
        assert_eq!(enc, Encoding::Linear);
        assert_eq!(px[0], 1.0);
        assert!((px[1] - 1.0 / 1023.0).abs() < 1e-6 && (px[2] - 7.0 / 1023.0).abs() < 1e-6);
        // Packed: 2 RGB pixels = 60 bits across two words, from the top bit.
        let vals = [1023u64, 0, 512, 1, 2, 3];
        let mut bits: u64 = 0;
        for v in vals {
            bits = (bits << 10) | v;
        }
        bits <<= 4;
        let f = dpx_file(true, 10, 0, 50, 0, 2, 1, &bits.to_be_bytes());
        let (_, _, px, _, _) = dpx::decode(&f).unwrap();
        let got: Vec<u32> = [0, 1, 2, 4, 5, 6].iter().map(|&i| (px[i] * 1023.0).round() as u32).collect();
        assert_eq!(got, vec![1023, 0, 512, 1, 2, 3]);
    }

    #[test]
    fn dpx_8_12_16_bit_and_flipped() {
        // 8-bit grey, 2×2, stored bottom to top: rows padded to 4 bytes.
        let f = dpx_file(true, 8, 0, 6, 0, 2, 2, &[0, 255, 0, 0, 255, 0, 0, 0]);
        let mut f2 = f.clone();
        f2[768..770].copy_from_slice(&2u16.to_be_bytes());
        let (_, _, px, _, _) = dpx::decode(&f).unwrap();
        assert_eq!((px[4], px[8], px[12]), (1.0, 1.0, 0.0));
        let (_, _, px, _, _) = dpx::decode(&f2).unwrap();
        assert_eq!((px[0], px[4], px[8]), (1.0, 0.0, 0.0));
        // 12-bit method A, little-endian RGB.
        let data: Vec<u8> = [4095u16 << 4, 0, 2048 << 4, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let (_, _, px, _, _) = dpx::decode(&dpx_file(false, 12, 1, 50, 0, 1, 1, &data)).unwrap();
        assert_eq!(px[0], 1.0);
        assert!((px[2] - 2048.0 / 4095.0).abs() < 1e-6);
        // 16-bit RGBA.
        let data: Vec<u8> = [65535u16, 0, 65535, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let (_, _, px, _, _) = dpx::decode(&dpx_file(true, 16, 0, 51, 0, 1, 1, &data)).unwrap();
        assert_eq!(&px[..4], &[1.0, 0.0, 1.0, 0.0]);
        // Not a DPX, and a cut-short one.
        assert!(dpx::decode(b"PNG.........").is_err());
        assert!(dpx::decode(&dpx_file(true, 10, 1, 50, 0, 100, 100, &[0; 8])).is_err());
    }

    #[test]
    fn display_tables() {
        let raw = lut(View::Raw, 0.0);
        let one = half::f16::from_f32(1.0).to_bits() as usize;
        let mid = half::f16::from_f32(0.5).to_bits() as usize;
        assert_eq!((raw[0], raw[one], raw[mid]), (0, 255, 128));
        let s = lut(View::Srgb, 0.0);
        assert_eq!(s[half::f16::from_f32(0.18).to_bits() as usize], 118);
        // One stop up shows 0.5 as white.
        assert_eq!(lut(View::Srgb, 1.0)[mid], 255);
        // Cineon: code 685 is white, 95 black.
        assert!((cineon_to_linear(685.0 / 1023.0) - 1.0).abs() < 1e-4);
        assert!(cineon_to_linear(95.0 / 1023.0).abs() < 1e-4);
        assert_eq!(View::Auto.resolve(Encoding::Log), View::Log);
        assert_eq!(View::Auto.resolve(Encoding::Display), View::Raw);
        assert!(rec709_curve(1.0) > 0.999);
    }

    #[test]
    fn engine_caches_ahead_within_budget() {
        let dir = std::env::temp_dir().join(format!("dirmap-player-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut slots = Vec::new();
        for i in 0..10 {
            if i == 4 {
                slots.push(None);
                continue;
            }
            let p = dir.join(format!("f.{i:04}.png"));
            image::RgbImage::from_pixel(16, 16, image::Rgb([i as u8 * 20, 0, 0])).save(&p).unwrap();
            slots.push(Some(p));
        }
        let clip = Clip { label: "f".into(), first: 0, step: 1, slots };
        // Room for 4 frames of 16×16 half RGBA (2 KB each).
        let e = Engine::new(&clip, 4 * 2048, 1);
        let wait = |e: &Engine| {
            for _ in 0..200 {
                if !e.busy() {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    if !e.busy() {
                        return;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        wait(&e);
        let m = e.map();
        assert!(m[0].0 && m[1].0 && m[2].0 && m[3].0, "{m:?}");
        assert!(m[4].1 && !m[4].0);
        assert!(e.memory().0 <= 4 * 2048);
        assert_eq!(e.slot(4), Slot::Missing);
        // Moving on drops the frames behind for the ones ahead.
        e.seek(6, 1);
        wait(&e);
        let m = e.map();
        assert!(m[6].0 && m[7].0 && m[8].0 && m[9].0, "{m:?}");
        assert!(!m[0].0);
        let f = e.frame(6).unwrap();
        assert_eq!((f.w, f.h, f.enc), (16, 16, Encoding::Display));
        let shown = to_display(&f, &lut(View::Raw, 0.0));
        assert_eq!(shown[0], eframe::egui::Color32::from_rgb(120, 0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cube_luts() {
        // An identity 3D LUT of size 2, and one that swaps red and blue.
        let id =
            "TITLE \"id\"\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let c = Cube::parse(id).unwrap();
        assert_eq!(c.title, "id");
        let v = c.apply([0.25, 0.5, 0.75]);
        assert!(
            (v[0] - 0.25).abs() < 1e-6 && (v[1] - 0.5).abs() < 1e-6 && (v[2] - 0.75).abs() < 1e-6
        );
        assert_eq!(c.apply([2.0, -1.0, 0.0]), [1.0, 0.0, 0.0]);
        let swap = "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        let v = Cube::parse(swap).unwrap().apply([1.0, 0.0, 0.25]);
        assert!((v[0] - 0.25).abs() < 1e-6 && v[2] == 1.0);
        // 1D, with a domain.
        let one = "LUT_1D_SIZE 3\nDOMAIN_MAX 2 2 2\n0 0 0\n0.5 0.5 0.5\n1 1 1\n";
        assert_eq!(
            Cube::parse(one).unwrap().apply([1.0, 2.0, 0.5]),
            [0.5, 1.0, 0.25]
        );
        assert!(Cube::parse("LUT_3D_SIZE 2\n0 0 0\n").is_err());
        assert!(Cube::parse("LUT_3D_SIZE 2\n0 0 x\n").is_err());
    }

    #[test]
    fn clip_slots_from_names() {
        let names = ["a.0001.exr", "a.0002.exr", "a.0004.exr"];
        let seq = crate::sequences::in_names(&names.iter().map(|n| (*n, 1u64)).collect::<Vec<_>>(), 3).remove(0).0;
        let c = Clip::from_names("a".into(), Path::new("/x"), &seq, &names);
        assert_eq!(c.slots.len(), 4);
        assert!(c.slots[2].is_none());
        assert_eq!(c.slots[3].as_deref(), Some(Path::new("/x/a.0004.exr")));
        assert_eq!(c.frame_number(3), 4);
    }
}
