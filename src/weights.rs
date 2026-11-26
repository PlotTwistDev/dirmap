//! Reading a model's numbers: turning GGUF's compressed blocks (and plain F32, F16, BF16) back
//! into values, a sample at a time, for statistics and histograms. Only reads; files only in the
//! cloud are never read.

use crate::inspect::Tensor;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Values per block and bytes per block for the types this module can read, by type name.
fn layout(kind: &str) -> Option<(usize, usize)> {
    Some(match kind {
        "F32" => (1, 4),
        "F16" | "BF16" => (1, 2),
        "Q8_0" => (32, 34),
        "Q4_0" => (32, 18),
        "Q4_1" => (32, 20),
        "Q5_0" => (32, 22),
        "Q5_1" => (32, 24),
        "Q4_K" => (256, 144),
        "Q5_K" => (256, 176),
        "Q6_K" => (256, 210),
        _ => return None,
    })
}

/// Whether this module can read a number type.
pub fn readable(kind: &str) -> bool {
    layout(kind).is_some()
}

fn f16(b: &[u8]) -> f32 {
    half::f16::from_bits(u16::from_le_bytes([b[0], b[1]])).to_f32()
}

/// Turns whole blocks of `kind` into values, appending to `out`.
pub fn decode(kind: &str, bytes: &[u8], out: &mut Vec<f32>) {
    let Some((_, size)) = layout(kind) else { return };
    for b in bytes.chunks_exact(size) {
        match kind {
            "F32" => out.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            "F16" => out.push(f16(b)),
            "BF16" => out.push(f32::from_bits((u16::from_le_bytes([b[0], b[1]]) as u32) << 16)),
            "Q8_0" => {
                let d = f16(b);
                out.extend(b[2..34].iter().map(|&q| q as i8 as f32 * d));
            }
            "Q4_0" => {
                let d = f16(b);
                let qs = &b[2..18];
                out.extend(qs.iter().map(|&q| ((q & 0xF) as i32 - 8) as f32 * d));
                out.extend(qs.iter().map(|&q| ((q >> 4) as i32 - 8) as f32 * d));
            }
            "Q4_1" => {
                let (d, m) = (f16(b), f16(&b[2..]));
                let qs = &b[4..20];
                out.extend(qs.iter().map(|&q| (q & 0xF) as f32 * d + m));
                out.extend(qs.iter().map(|&q| (q >> 4) as f32 * d + m));
            }
            "Q5_0" | "Q5_1" => {
                let one = kind == "Q5_1";
                let d = f16(b);
                let (m, at) = if one { (f16(&b[2..]), 4) } else { (0.0, 2) };
                let qh = u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
                let qs = &b[at + 4..at + 20];
                let bias = if one { 0 } else { 16 };
                let lo: Vec<f32> = (0..16).map(|j| (((qs[j] & 0xF) as u32 | (((qh >> j) << 4) & 0x10)) as i32 - bias) as f32 * d + m).collect();
                let hi: Vec<f32> = (0..16).map(|j| (((qs[j] >> 4) as u32 | ((qh >> (j + 12)) & 0x10)) as i32 - bias) as f32 * d + m).collect();
                out.extend(lo);
                out.extend(hi);
            }
            "Q4_K" | "Q5_K" => {
                let five = kind == "Q5_K";
                let (d, dmin) = (f16(b), f16(&b[2..]));
                let scales = &b[4..16];
                let (qh, mut qs) = if five { (&b[16..48], &b[48..176]) } else { (&b[0..0], &b[16..144]) };
                let (mut u1, mut u2) = (1u8, 2u8);
                for j in 0..4 {
                    let (s1, m1) = scale_min(2 * j, scales);
                    let (s2, m2) = scale_min(2 * j + 1, scales);
                    let (d1, m1, d2, m2) = (d * s1 as f32, dmin * m1 as f32, d * s2 as f32, dmin * m2 as f32);
                    for l in 0..32 {
                        let h = if five && qh[l] & u1 != 0 { 16 } else { 0 };
                        out.push(d1 * ((qs[l] & 0xF) + h) as f32 - m1);
                    }
                    for l in 0..32 {
                        let h = if five && qh[l] & u2 != 0 { 16 } else { 0 };
                        out.push(d2 * ((qs[l] >> 4) + h) as f32 - m2);
                    }
                    qs = &qs[32..];
                    u1 <<= 2;
                    u2 <<= 2;
                }
            }
            "Q6_K" => {
                let d = f16(&b[208..]);
                let (mut ql, mut qh, mut sc) = (&b[0..128], &b[128..192], &b[192..208]);
                for _ in 0..2 {
                    let mut y = [0f32; 128];
                    for l in 0..32 {
                        let is = l / 16;
                        let q1 = ((ql[l] & 0xF) | ((qh[l] & 3) << 4)) as i32 - 32;
                        let q2 = ((ql[l + 32] & 0xF) | (((qh[l] >> 2) & 3) << 4)) as i32 - 32;
                        let q3 = ((ql[l] >> 4) | (((qh[l] >> 4) & 3) << 4)) as i32 - 32;
                        let q4 = ((ql[l + 32] >> 4) | (((qh[l] >> 6) & 3) << 4)) as i32 - 32;
                        y[l] = d * (sc[is] as i8) as f32 * q1 as f32;
                        y[l + 32] = d * (sc[is + 2] as i8) as f32 * q2 as f32;
                        y[l + 64] = d * (sc[is + 4] as i8) as f32 * q3 as f32;
                        y[l + 96] = d * (sc[is + 6] as i8) as f32 * q4 as f32;
                    }
                    out.extend(y);
                    ql = &ql[64..];
                    qh = &qh[32..];
                    sc = &sc[8..];
                }
            }
            _ => {}
        }
    }
}

/// The 6-bit scale and min of sub-block `j` in a K-quant's 12 scale bytes.
fn scale_min(j: usize, q: &[u8]) -> (u8, u8) {
    if j < 4 {
        (q[j] & 63, q[j + 4] & 63)
    } else {
        ((q[j + 4] & 0xF) | ((q[j - 4] >> 6) << 4), (q[j + 4] >> 4) | ((q[j] >> 6) << 4))
    }
}

/// Statistics of a sample of a tensor's values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Values looked at, and how many the tensor has.
    pub sampled: u64,
    pub total: u64,
    pub nan: u64,
    pub inf: u64,
    pub zeros: u64,
    pub min: f32,
    pub max: f32,
    pub mean: f64,
    pub std: f64,
    /// Histogram of the finite values between `range`.
    pub bins: Vec<u32>,
    pub range: (f32, f32),
    /// Values outside `range` (the extreme 0.1% at each end).
    pub outside: u64,
}

impl Stats {
    pub fn of(values: &[f32], total: u64, bins: usize) -> Stats {
        let mut s = Stats { sampled: values.len() as u64, total, min: f32::INFINITY, max: f32::NEG_INFINITY, ..Default::default() };
        let mut finite: Vec<f32> = Vec::with_capacity(values.len());
        let (mut sum, mut sq) = (0f64, 0f64);
        for &v in values {
            if v.is_nan() {
                s.nan += 1;
            } else if v.is_infinite() {
                s.inf += 1;
            } else {
                if v == 0.0 {
                    s.zeros += 1;
                }
                s.min = s.min.min(v);
                s.max = s.max.max(v);
                sum += v as f64;
                sq += v as f64 * v as f64;
                finite.push(v);
            }
        }
        if finite.is_empty() {
            s.min = 0.0;
            s.max = 0.0;
            return s;
        }
        let n = finite.len() as f64;
        s.mean = sum / n;
        s.std = (sq / n - s.mean * s.mean).max(0.0).sqrt();
        // The histogram covers all but the extreme 0.1% at each end, so a few outliers don't
        // squash it.
        finite.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f64| finite[((n - 1.0) * q) as usize];
        let (mut lo, mut hi) = (at(0.001), at(0.999));
        if lo >= hi {
            lo = s.min;
            hi = s.max;
        }
        if lo >= hi {
            hi = lo + 1e-6;
        }
        s.range = (lo, hi);
        s.bins = vec![0; bins.max(1)];
        let w = (hi - lo) / s.bins.len() as f32;
        for &v in &finite {
            if v < lo || v > hi {
                s.outside += 1;
                continue;
            }
            let i = (((v - lo) / w) as usize).min(s.bins.len() - 1);
            s.bins[i] += 1;
        }
        s
    }
}

/// Reads up to `windows` evenly spread pieces of about `window` bytes of a tensor and decodes
/// them. Small tensors are read whole.
pub fn sample(path: &Path, t: &Tensor, windows: u64, window: u64) -> Result<Vec<f32>, String> {
    let (per, size) = layout(&t.kind).ok_or_else(|| format!("{} numbers can't be read yet", t.kind))?;
    let size = size as u64;
    let blocks = t.bytes / size;
    if blocks == 0 {
        return Ok(Vec::new());
    }
    let per_window = (window / size).max(1);
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let mut read = |first: u64, count: u64, out: &mut Vec<f32>| -> Result<(), String> {
        buf.resize((count * size) as usize, 0);
        f.seek(SeekFrom::Start(t.offset + first * size)).map_err(|e| e.to_string())?;
        f.read_exact(&mut buf).map_err(|_| "the file ends before this tensor does".to_string())?;
        decode(&t.kind, &buf, out);
        Ok(())
    };
    if blocks <= per_window * windows {
        // Read it all, a window at a time.
        let mut b = 0;
        while b < blocks {
            let n = per_window.min(blocks - b);
            read(b, n, &mut out)?;
            b += n;
        }
    } else {
        let step = blocks / windows;
        for w in 0..windows {
            read(w * step, per_window.min(blocks - w * step), &mut out)?;
        }
    }
    let _ = per;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(v: f32) -> [u8; 2] {
        half::f16::from_f32(v).to_bits().to_le_bytes()
    }

    #[test]
    fn plain_and_q8_0() {
        let mut out = Vec::new();
        decode("F16", &[h(1.5), h(-2.0)].concat(), &mut out);
        decode("BF16", &((1.0f32.to_bits() >> 16) as u16).to_le_bytes(), &mut out);
        let mut q8 = h(0.5).to_vec();
        q8.extend((0..32).map(|i| (i as i8 - 16) as u8));
        decode("Q8_0", &q8, &mut out);
        assert_eq!(&out[..3], &[1.5, -2.0, 1.0]);
        assert_eq!(out[3], -8.0);
        assert_eq!(out[3 + 31], 7.5);
    }

    #[test]
    fn q4_0_and_q4_k() {
        let mut b = h(2.0).to_vec();
        b.extend([0x9Fu8; 16]); // low nibble 15 → 7, high nibble 9 → 1
        let mut out = Vec::new();
        decode("Q4_0", &b, &mut out);
        assert_eq!(out.len(), 32);
        assert_eq!(out[0], 14.0);
        assert_eq!(out[16], 2.0);
        // Q4_K: d = 1, dmin = 0.5, every scale 1 and min 2, every nibble 3: 1*3 - 0.5*2 = 2.
        let mut k = [h(1.0), h(0.5)].concat();
        let mut scales = [0u8; 12];
        scales[..4].fill(1);
        scales[4..8].fill(2);
        scales[8..].fill(0x21); // sub-blocks 4-7: scale low 1, min high 2
        k.extend(scales);
        k.extend([0x33u8; 128]);
        out.clear();
        decode("Q4_K", &k, &mut out);
        assert_eq!(out.len(), 256);
        assert!(out.iter().all(|&v| v == 2.0), "{:?}", &out[..8]);
    }

    #[test]
    fn q6_k_layout() {
        // All quants 32 (zero after the -32), so every value is 0 whatever the scales.
        let mut b = vec![0u8; 210];
        b[128..192].fill(0b1010_1010); // high bits 2 → 32
        b[192..208].fill(3);
        b[208..210].copy_from_slice(&h(1.0));
        let mut out = Vec::new();
        decode("Q6_K", &b, &mut out);
        assert_eq!(out.len(), 256);
        assert!(out.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn stats_and_histogram() {
        let mut v: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0 - 0.5).collect();
        v.push(f32::NAN);
        v.push(0.0);
        let s = Stats::of(&v, 2000, 10);
        assert_eq!(s.nan, 1);
        assert_eq!(s.zeros, 2);
        assert_eq!(s.sampled, 1002);
        assert!(s.mean.abs() < 0.01);
        assert_eq!(s.bins.iter().map(|&b| b as u64).sum::<u64>() + s.outside, 1001);
        assert!(s.range.0 > -0.5 && s.range.1 < 0.5);
    }
}

/// Compares with the gguf Python package's own dequantization, from blocks of random bytes it
/// wrote: `FILESCAPE_DEQUANT_FIXTURES=dir cargo test dequant_matches -- --ignored`.
#[cfg(test)]
#[test]
#[ignore]
fn dequant_matches_reference() {
    let dir = std::path::PathBuf::from(std::env::var("FILESCAPE_DEQUANT_FIXTURES").unwrap());
    for kind in ["Q4_0", "Q4_1", "Q5_0", "Q5_1", "Q8_0", "Q4_K", "Q5_K", "Q6_K"] {
        let raw = std::fs::read(dir.join(format!("{kind}.bin"))).unwrap();
        let want: Vec<f32> = std::fs::read(dir.join(format!("{kind}.f32"))).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let mut got = Vec::new();
        decode(kind, &raw, &mut got);
        assert_eq!(got.len(), want.len(), "{kind}");
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!(g == w || (g.is_nan() && w.is_nan()) || (g - w).abs() <= 1e-3 * w.abs().max(1.0), "{kind}[{i}]: {g} vs {w}");
        }
    }
}
