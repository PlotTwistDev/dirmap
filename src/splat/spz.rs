//! SPZ, Niantic's compressed splat format (MIT; github.com/nianticlabs/spz).
//!
//! Versions 1–3 are one gzip stream: a 16-byte header, then positions, alphas, colours,
//! scales, rotations and spherical harmonics, each attribute for every point in turn.
//! Version 4 starts with a 32-byte plain header ("NGSP"), optional extension records, a table
//! of contents, and each attribute as its own zstd frame.
//!
//! Point counts are checked against the file size (at most 1024 / 9 points per byte, the
//! reference reader's bound) and every stream is decompressed with a ceiling, so a small file
//! can't ask for gigabytes.

use super::{Frame, Limits, Scene, SceneKind, SPZ_MAX_POINTS_PER_BYTE, sh_rest, to_u8};
use std::io::Read;
use std::path::Path;

const MAGIC: u32 = 0x5053_474e; // "NGSP"
/// SPZ stores colour as `sh0 * 0.15 + 0.5`; 3DGS uses `sh0 * C0 + 0.5`.
const COLOR_SCALE: f32 = 0.15;

struct Header {
    version: u32,
    points: usize,
    sh_degree: u8,
    fractional_bits: u8,
    flags: u8,
}

/// Bytes of each attribute for `n` points.
struct Sizes {
    pos: usize,
    alpha: usize,
    color: usize,
    scale: usize,
    rot: usize,
    sh: usize,
}

impl Header {
    fn sizes(&self) -> Sizes {
        let n = self.points;
        Sizes {
            pos: n * if self.version == 1 { 6 } else { 9 },
            alpha: n,
            color: n * 3,
            scale: n * 3,
            rot: n * if self.version >= 3 { 4 } else { 3 },
            sh: n * sh_rest(self.sh_degree) * 3,
        }
    }

    fn check(&self, file_len: u64) -> Result<(), String> {
        if !(1..=4).contains(&self.version) {
            return Err(format!("SPZ version {} isn't one Filescape knows.", self.version));
        }
        if self.points == 0 {
            return Err("This SPZ file has no splats.".into());
        }
        if self.sh_degree > 4 {
            return Err(format!("Spherical harmonics of degree {} aren't valid.", self.sh_degree));
        }
        if self.points as u64 > file_len.saturating_mul(SPZ_MAX_POINTS_PER_BYTE) || self.points > i32::MAX as usize {
            return Err(format!("This SPZ file claims {} splats, more than a file of its size can hold.", self.points));
        }
        if self.points > super::HARD_MAX_POINTS * 4 {
            return Err(format!("This SPZ file has {} splats, more than Filescape reads.", self.points));
        }
        Ok(())
    }
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Read at most `max` bytes from a decoder, and fail if it has more or fewer than `want`.
fn exact(mut r: impl Read, want: usize, what: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(want.min(64 << 20));
    (&mut r).take(want as u64).read_to_end(&mut out).map_err(|e| format!("The {what} couldn't be decompressed: {e}"))?;
    if out.len() < want {
        return Err(format!("The {what} stop early: the file is cut short or damaged."));
    }
    Ok(out)
}

pub fn read(path: &Path, limits: Limits) -> Result<Scene, String> {
    let data = std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    parse(&data, limits)
}

pub(super) fn parse(data: &[u8], limits: Limits) -> Result<Scene, String> {
    let file_len = data.len() as u64;
    // Uncompressed legacy files (versions 1–3 without the gzip) are read too, as PlayCanvas's
    // tools do.
    let raw_legacy = data.starts_with(b"NGSP") && data.len() >= 8 && u32le(data, 4) < 4;
    let (head, streams) = if data.starts_with(b"NGSP") && !raw_legacy {
        if data.len() < 32 {
            return Err("This SPZ file is cut short.".into());
        }
        let head = Header { version: u32le(data, 4), points: u32le(data, 8) as usize, sh_degree: data[12], fractional_bits: data[13], flags: data[14] };
        head.check(file_len)?;
        if head.version < 4 {
            return Err("This SPZ file's header doesn't match its version.".into());
        }
        let count = data[15] as usize;
        let toc = u32le(data, 16) as usize;
        let s = head.sizes();
        let wanted: Vec<(usize, &str)> =
            [(s.pos, "positions"), (s.alpha, "opacities"), (s.color, "colours"), (s.scale, "scales"), (s.rot, "rotations"), (s.sh, "spherical harmonics")].into_iter().filter(|(n, _)| *n > 0).collect();
        if count != wanted.len() {
            return Err("This SPZ file's table of contents doesn't match its header.".into());
        }
        let toc_end = toc.checked_add(count * 16).filter(|&e| toc >= 32 && e <= data.len()).ok_or("This SPZ file's table of contents is outside the file.")?;
        let mut at = toc_end;
        let mut out = Vec::new();
        for (i, (want, what)) in wanted.iter().enumerate() {
            let comp = u64le(data, toc + i * 16) as usize;
            let raw = u64le(data, toc + i * 16 + 8) as usize;
            if raw != *want {
                return Err("This SPZ file's table of contents doesn't match its header.".into());
            }
            let end = at.checked_add(comp).filter(|&e| e <= data.len()).ok_or("This SPZ file is cut short.")?;
            let dec = ruzstd::decoding::StreamingDecoder::new(&data[at..end]).map_err(|e| format!("The {what} couldn't be decompressed: {e}"))?;
            out.push(exact(dec, *want, what)?);
            at = end;
        }
        // Back in the fixed order, with an empty one for missing harmonics.
        if s.sh == 0 {
            out.push(Vec::new());
        }
        (head, out)
    } else if data.starts_with(&[0x1f, 0x8b]) || raw_legacy {
        let mut gz: Box<dyn Read> = if raw_legacy { Box::new(data) } else { Box::new(flate2::read::GzDecoder::new(data)) };
        let mut h = [0u8; 16];
        gz.read_exact(&mut h).map_err(|_| "Not an SPZ file (no header inside).".to_string())?;
        if u32le(&h, 0) != MAGIC {
            return Err("Not an SPZ file.".into());
        }
        let head = Header { version: u32le(&h, 4), points: u32le(&h, 8) as usize, sh_degree: h[12], fractional_bits: h[13], flags: h[14] };
        head.check(file_len)?;
        if head.version >= 4 {
            return Err("This SPZ file's header doesn't match its version.".into());
        }
        let s = head.sizes();
        let mut out = Vec::new();
        for (n, what) in [(s.pos, "positions"), (s.alpha, "opacities"), (s.color, "colours"), (s.scale, "scales"), (s.rot, "rotations"), (s.sh, "spherical harmonics")] {
            out.push(exact(&mut gz, n, what)?);
        }
        (head, out)
    } else {
        return Err("Not an SPZ file.".into());
    };
    let [pos, alpha, color, scale, rot, sh]: [Vec<u8>; 6] = streams.try_into().map_err(|_| "Bad SPZ data.".to_string())?;

    let n = head.points;
    let mut scene = Scene::new(SceneKind::Splat, format!("SPZ v{}", head.version), Frame::RightUpBack, n);
    scene.sh_degree = head.sh_degree;
    scene.antialiased = head.flags & 1 != 0;
    let rest = sh_rest(head.sh_degree);
    let keep_sh = limits.keeps_sh(n as u64, rest);
    if limits.keep_sh && rest > 0 && !keep_sh {
        scene.notes.push(super::SH_DROPPED.into());
    }
    let cap = limits.max_points.min(super::HARD_MAX_POINTS);
    let every = n.div_ceil(cap.max(1)).max(1);
    let frac = 1.0 / (1u32 << head.fractional_bits.min(24)) as f32;
    let mut shv = vec![0f32; if keep_sh { rest * 3 } else { 0 }];
    for i in (0..n).step_by(every) {
        let p = if head.version == 1 {
            let h = |k: usize| half::f16::from_le_bytes([pos[i * 6 + k * 2], pos[i * 6 + k * 2 + 1]]).to_f32();
            [h(0), h(1), h(2)]
        } else {
            let f = |k: usize| {
                let b = &pos[i * 9 + k * 3..];
                let mut v = b[0] as i32 | (b[1] as i32) << 8 | (b[2] as i32) << 16;
                if v & 0x80_0000 != 0 {
                    v |= !0xff_ffff;
                }
                v as f32 * frac
            };
            [f(0), f(1), f(2)]
        };
        let a = alpha[i];
        let c = |k: usize| {
            // Back to the base harmonic, then to 3DGS's colour.
            let sh0 = (color[i * 3 + k] as f32 / 255.0 - 0.5) / COLOR_SCALE;
            to_u8(sh0 * super::SH_C0 + 0.5)
        };
        let s = |k: usize| (scale[i * 3 + k] as f32 / 16.0 - 10.0).exp();
        let r = if head.version >= 3 {
            let mut comp = u32::from_le_bytes(rot[i * 4..i * 4 + 4].try_into().unwrap());
            let largest = (comp >> 30) as usize;
            const MASK: u32 = (1 << 9) - 1;
            let mut q = [0f32; 4];
            let mut sum = 0.0;
            for (k, slot) in q.iter_mut().enumerate() {
                if k == largest {
                    continue;
                }
                let mag = comp & MASK;
                let neg = (comp >> 9) & 1 != 0;
                comp >>= 10;
                let v = std::f32::consts::FRAC_1_SQRT_2 * (mag as f32 / MASK as f32) * if neg { -1.0 } else { 1.0 };
                sum += v * v;
                *slot = v;
            }
            q[largest] = (1.0 - sum).max(0.0).sqrt();
            q
        } else {
            let xyz = [0, 1, 2].map(|k| rot[i * 3 + k] as f32 / 127.5 - 1.0);
            let w = (1.0 - xyz.iter().map(|v| v * v).sum::<f32>()).max(0.0).sqrt();
            [xyz[0], xyz[1], xyz[2], w]
        };
        if keep_sh {
            for (k, v) in shv.iter_mut().enumerate() {
                *v = (sh[i * rest * 3 + k] as f32 - 128.0) / 128.0;
            }
        }
        scene.push(p, [c(0), c(1), c(2), a], Some([s(0), s(1), s(2)]), Some(r), &shv);
    }
    Ok(scene)
}
