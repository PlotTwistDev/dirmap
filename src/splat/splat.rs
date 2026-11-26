//! Two web viewers' formats: Antimatter15's `.splat` (32-byte records, no header) and Mark
//! Kellogg's `.ksplat` (sections of spatial buckets, three levels of compression). Both keep
//! the 3DGS trainers' axes. Layouts follow PlayCanvas splat-transform's readers (MIT).

use super::{Frame, Limits, Scene, SceneKind};
use std::io::{BufReader, Read, Seek, SeekFrom};

fn open(path: &std::path::Path) -> Result<(std::fs::File, u64), String> {
    let f = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    Ok((f, len))
}

/// `.splat`: per splat, position (3 floats), linear scale (3 floats), colour and opacity
/// (4 bytes) and rotation (4 bytes, w x y z).
pub fn read_splat(path: &std::path::Path, limits: Limits) -> Result<Scene, String> {
    let (f, len) = open(path)?;
    if len == 0 || len % 32 != 0 {
        return Err("Not a .splat scene: its length isn't a whole number of 32-byte splats.".into());
    }
    let total = len / 32;
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut head = vec![0u8; (len.min(2048)) as usize];
    r.read_exact(&mut head).map_err(|e| format!("Couldn't read it: {e}"))?;
    if !looks_like_splat(&head) {
        return Err("This doesn't look like a .splat scene.".into());
    }
    r.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut scene = Scene::new(SceneKind::Splat, "SPLAT".into(), Frame::RightDownForward, total as usize);
    let every = super::every(total, limits);
    let mut rec = [0u8; 32];
    let mut i = 0u64;
    while i < total {
        r.read_exact(&mut rec).map_err(|e| format!("Couldn't read it: {e}"))?;
        let f = |k: usize| f32::from_le_bytes(rec[k * 4..k * 4 + 4].try_into().unwrap());
        let q = |k: usize| rec[28 + k] as f32 / 255.0 * 2.0 - 1.0;
        let c = |k: usize| rec[24 + k] as f32 / 255.0;
        scene.push_rgb([f(0), f(1), f(2)], [c(0), c(1), c(2)], c(3), [f(3), f(4), f(5)], [q(1), q(2), q(3), q(0)], &[]);
        i += every;
        if every > 1 && i < total {
            r.seek_relative(((every - 1) * 32) as i64).map_err(|e| e.to_string())?;
        }
    }
    Ok(scene)
}

/// Whether `.splat` data looks like splats rather than any file whose length happens to be a
/// multiple of 32: the first records have finite positions and positive scales.
pub fn looks_like_splat(head: &[u8]) -> bool {
    head.len() >= 32
        && head.chunks_exact(32).take(64).all(|r| {
            let f = |k: usize| f32::from_le_bytes(r[k * 4..k * 4 + 4].try_into().unwrap());
            (0..3).all(|k| f(k).is_finite() && f(k).abs() < 1e7) && (3..6).all(|k| f(k).is_finite() && f(k) >= 0.0 && f(k) < 1e7)
        })
}

struct Mode {
    centre: usize,
    scale: usize,
    rot: usize,
    sh: usize,
}

const MODES: [Mode; 3] = [Mode { centre: 12, scale: 12, rot: 16, sh: 4 }, Mode { centre: 6, scale: 6, rot: 8, sh: 2 }, Mode { centre: 6, scale: 6, rot: 8, sh: 1 }];

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn f16_at(b: &[u8], o: usize) -> f32 {
    half::f16::from_bits(u16_at(b, o)).to_f32()
}

/// `.ksplat`: a 4 KB header, 1 KB per section header, then each section's bucket table and
/// splats.
pub fn read_ksplat(path: &std::path::Path, limits: Limits) -> Result<Scene, String> {
    const MAIN: u64 = 4096;
    const SECTION: u64 = 1024;
    let (f, len) = open(path)?;
    let mut r = BufReader::with_capacity(1 << 20, f);
    if len < MAIN {
        return Err("Too short to be a .ksplat scene.".into());
    }
    let mut head = vec![0u8; MAIN as usize];
    r.read_exact(&mut head).map_err(|e| format!("Couldn't read it: {e}"))?;
    let (major, minor) = (head[0], head[1]);
    if major != 0 || minor < 1 {
        return Err(format!("Unknown .ksplat version {major}.{minor}."));
    }
    let sections = u32_at(&head, 4) as u64;
    let total = u32_at(&head, 16) as u64;
    let mode_n = u16_at(&head, 20);
    let mode = MODES.get(mode_n as usize).ok_or_else(|| format!("Unknown .ksplat compression {mode_n}."))?;
    let nz = |v: f32, d: f32| if v == 0.0 || !v.is_finite() { d } else { v };
    let (sh_min, sh_max) = (nz(f32_at(&head, 36), -1.5), nz(f32_at(&head, 40), 1.5));
    if total == 0 || sections == 0 || MAIN + sections.saturating_mul(SECTION) > len || total > len {
        return Err("This .ksplat scene's header doesn't fit the file.".into());
    }
    let mut sh_heads = vec![0u8; (sections * SECTION) as usize];
    r.read_exact(&mut sh_heads).map_err(|e| format!("Couldn't read it: {e}"))?;
    let section = |s: u64| &sh_heads[(s * SECTION) as usize..((s + 1) * SECTION) as usize];
    let degree = (0..sections).filter(|&s| u32_at(section(s), 0) > 0).map(|s| u16_at(section(s), 40)).max().unwrap_or(0);
    if degree > 3 {
        return Err("This .ksplat scene has harmonics of an unknown degree.".into());
    }
    let mut scene = Scene::new(SceneKind::Splat, format!("KSPLAT (compression {mode_n})"), Frame::RightDownForward, total as usize);
    scene.sh_degree = degree as u8;
    let rest = super::sh_rest(degree as u8);
    let keep_sh = scene.plan_sh(limits, rest);
    let every = super::every(total, limits);
    let mut at = MAIN + sections * SECTION;
    let mut global = 0u64;
    let mut ours = vec![0f32; rest * 3];
    for s in 0..sections {
        let h = section(s);
        let count = u32_at(h, 0) as u64;
        let max_count = u32_at(h, 4) as u64;
        let capacity = u32_at(h, 8) as u64;
        let buckets = u32_at(h, 12) as u64;
        let block = f32_at(h, 16);
        let bucket_bytes = u16_at(h, 20) as u64;
        let quant = match u32_at(h, 24) {
            0 => if mode_n == 0 { 1 } else { 32767 },
            q => q,
        } as f32;
        let full = u32_at(h, 32) as u64;
        let partial = u32_at(h, 36) as u64;
        let sdeg = u16_at(h, 40);
        if sdeg > 3 || count > max_count {
            return Err("A .ksplat section header is damaged.".into());
        }
        let n_sh = [0usize, 9, 24, 45][sdeg as usize];
        let per = mode.centre + mode.scale + mode.rot + 4 + n_sh * mode.sh;
        let meta = partial * 4;
        let table = bucket_bytes.checked_mul(buckets).and_then(|b| b.checked_add(meta)).ok_or("Bad .ksplat section.")?;
        let data = max_count.checked_mul(per as u64).ok_or("Bad .ksplat section.")?;
        if at.checked_add(table).and_then(|v| v.checked_add(data)).is_none_or(|e| e > len) || buckets.saturating_mul(12) > table || partial > buckets {
            return Err("A .ksplat section runs past the end of the file.".into());
        }
        // Bucket table: sizes of the partly filled buckets, then every bucket's centre.
        r.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        let mut tb = vec![0u8; table as usize];
        r.read_exact(&mut tb).map_err(|e| format!("Couldn't read it: {e}"))?;
        let part_size = |k: u64| u32_at(&tb, (k * 4) as usize) as u64;
        let centre = |b: u64, k: usize| if (meta + b * 12 + 12) <= table { f32_at(&tb, (meta + b * 12) as usize + k * 4) } else { 0.0 };
        let pos_scale = block / 2.0 / quant;
        let full_splats = full * capacity;
        let (mut pb, mut pbase) = (full, full_splats);
        let data_at = at + table;
        let mut rec = vec![0u8; per];
        let mut last_read: Option<u64> = None;
        for i in 0..count {
            // Which bucket this splat is in, walked forward even for skipped splats.
            let bucket = if i < full_splats {
                i / capacity.max(1)
            } else {
                if pb - full < partial && i >= pbase + part_size(pb - full) {
                    pbase += part_size(pb - full);
                    pb += 1;
                }
                pb
            };
            if global % every != 0 {
                global += 1;
                continue;
            }
            global += 1;
            let want = data_at + i * per as u64;
            if last_read != Some(want) {
                r.seek(SeekFrom::Start(want)).map_err(|e| e.to_string())?;
            }
            r.read_exact(&mut rec).map_err(|e| format!("Couldn't read it: {e}"))?;
            last_read = Some(want + per as u64);
            let pos = if mode_n == 0 {
                [f32_at(&rec, 0), f32_at(&rec, 4), f32_at(&rec, 8)]
            } else {
                let p = |k: usize| (u16_at(&rec, k * 2) as f32 - quant) * pos_scale + centre(bucket, k);
                [p(0), p(1), p(2)]
            };
            let sc = mode.centre;
            let (scale, rot) = if mode_n == 0 {
                ([f32_at(&rec, sc), f32_at(&rec, sc + 4), f32_at(&rec, sc + 8)], [f32_at(&rec, sc + 12), f32_at(&rec, sc + 16), f32_at(&rec, sc + 20), f32_at(&rec, sc + 24)])
            } else {
                ([f16_at(&rec, sc), f16_at(&rec, sc + 2), f16_at(&rec, sc + 4)], [f16_at(&rec, sc + 6), f16_at(&rec, sc + 8), f16_at(&rec, sc + 10), f16_at(&rec, sc + 12)])
            };
            let co = mode.centre + mode.scale + mode.rot;
            let c = |k: usize| rec[co + k] as f32 / 255.0;
            if keep_sh {
                ours.iter_mut().for_each(|v| *v = 0.0);
                let ho = co + 4;
                for k in 0..n_sh {
                    // Stored band by band, each band red's then green's then blue's.
                    let (channel, coeff) = if k < 9 { (k / 3, k % 3) } else if k < 24 { ((k - 9) / 5, (k - 9) % 5 + 3) } else { ((k - 24) / 7, (k - 24) % 7 + 8) };
                    let v = match mode_n {
                        0 => f32_at(&rec, ho + k * 4),
                        1 => f16_at(&rec, ho + k * 2),
                        _ => sh_min + rec[ho + k] as f32 / 255.0 * (sh_max - sh_min),
                    };
                    if coeff < rest {
                        ours[coeff * 3 + channel] = v;
                    }
                }
            }
            scene.push_rgb(pos, [c(0), c(1), c(2)], c(3), scale.map(|v| v.max(1e-6)), [rot[1], rot[2], rot[3], rot[0]], if keep_sh { &ours } else { &[] });
        }
        at += table + data;
    }
    Ok(scene)
}
