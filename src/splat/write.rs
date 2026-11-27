//! Writing scenes in other formats, for the storage optimiser: SPZ (Niantic's, gzip, version
//! 3), PlayCanvas's compressed PLY, and the standard 3DGS PLY most tools read.
//!
//! Scenes are encoded in memory, so the size each format would take is measured, not guessed.
//! Writing goes to a new file next to the original (never over it), through a temporary name,
//! and the new file is read back and compared before the person is told it worked.
//!
//! What's in memory is already rounded a little (colour and opacity as bytes, harmonics as
//! half floats), which is as fine as SPZ and compressed PLY store them.

use super::{Frame, Limits, SH_C0, Scene, SceneKind, sh_rest};
use std::io::Write;
use std::path::{Path, PathBuf};

/// A format a scene can be written in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Spz,
    CompressedPly,
    Ply,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Spz, Target::CompressedPly, Target::Ply];

    pub fn name(self) -> &'static str {
        match self {
            Target::Spz => "SPZ",
            Target::CompressedPly => "Compressed PLY",
            Target::Ply => "PLY",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Target::Spz => "Niantic's compressed format: the smallest, read by most newer viewers and tools",
            Target::CompressedPly => "PlayCanvas's compressed PLY, as SuperSplat exports: small, read by PlayCanvas and SuperSplat",
            Target::Ply => "The standard 3DGS PLY: big, but read by every splat tool and trainer",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Target::Spz => "spz",
            Target::CompressedPly => "compressed.ply",
            Target::Ply => "ply",
        }
    }

    /// Whether `scene` is in this format already.
    pub fn is_format_of(self, scene: &Scene) -> bool {
        let f = scene.format.as_str();
        match self {
            Target::Spz => f.starts_with("SPZ"),
            Target::CompressedPly => f.starts_with("Compressed PLY"),
            Target::Ply => f == "PLY",
        }
    }
}

/// Why a scene can't be written whole.
pub fn unwritable(scene: &Scene) -> Option<String> {
    if scene.kind != SceneKind::Splat {
        return Some(match scene.kind {
            SceneKind::PointCloud => "This is a point cloud, not a splat scene.".into(),
            _ => "Scenes that change over time can't be converted yet.".into(),
        });
    }
    if scene.len() < scene.total {
        return Some(format!("Only {} of its {} splats could be read, so it can't be converted whole.", super::facts::group(scene.len()), super::facts::group(scene.total)));
    }
    if scene.scale.len() != scene.len() || scene.rot.len() != scene.len() {
        return Some("Some of its splats have no shape.".into());
    }
    if scene.sh.len() != scene.len() * sh_rest(scene.sh_degree) * 3 {
        return Some("Its view-dependent colour wasn't read.".into());
    }
    if scene.len() == 0 {
        return Some("It has no splats.".into());
    }
    None
}

/// Whether a scene already in memory (read for the preview) has everything a conversion
/// needs, so it needn't be read again.
pub fn complete(scene: &Scene) -> bool {
    unwritable(scene).is_none()
}

/// The scene in `target`'s bytes.
pub fn encode(scene: &Scene, target: Target) -> Result<Vec<u8>, String> {
    if let Some(why) = unwritable(scene) {
        return Err(why);
    }
    match target {
        Target::Spz => spz(scene),
        Target::CompressedPly => Ok(compressed_ply(scene)),
        Target::Ply => Ok(ply(scene)),
    }
}

/// The base harmonic (`f_dc`) from a byte colour.
fn dc(c: u8) -> f32 {
    (c as f32 / 255.0 - 0.5) / SH_C0
}

/// Opacity before the sigmoid, from a byte; kept finite at the ends.
fn logit(a: u8) -> f32 {
    let p = (a as f32 / 255.0).clamp(0.5 / 255.0, 254.5 / 255.0);
    (p / (1.0 - p)).ln()
}

/// Signs of each higher harmonic (in our order, coefficient by coefficient) when y and z are
/// flipped, as between PLY's axes (right, down, forward) and SPZ's (right, up, back): the
/// basis functions odd in y or z (but not both) change sign.
const FLIP_YZ: [f32; 24] = [
    -1.0, -1.0, 1.0, // degree 1: y, z, x
    -1.0, 1.0, 1.0, -1.0, 1.0, // degree 2: xy, yz, z², xz, x²-y²
    -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, // degree 3
    -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0, // degree 4
];

/// A splat turned into `frame`: position, x y z w rotation, and whether its harmonics flip.
fn in_frame(scene: &Scene, frame: Frame, i: usize) -> ([f32; 3], [f32; 4], bool) {
    let (p, q) = (scene.pos[i], scene.rot[i]);
    if scene.frame == frame {
        (p, q, false)
    } else {
        // A half turn about x: (x, y, z) to (x, -y, -z), and the rotation with it.
        ([p[0], -p[1], -p[2]], [q[0], -q[1], -q[2], q[3]], true)
    }
}

/// SPZ version 3: a 16-byte header, then each attribute for every splat in turn, all in one
/// gzip stream. Positions are 24-bit fixed point (as many fraction bits as the scene's size
/// allows, up to 12), rotations the smallest three of the quaternion.
fn spz(scene: &Scene) -> Result<Vec<u8>, String> {
    let n = scene.len();
    if n > u32::MAX as usize {
        return Err("Too many splats for SPZ.".into());
    }
    let rest = sh_rest(scene.sh_degree);
    let degree = scene.sh_degree.min(3);
    let rest_out = sh_rest(degree);
    let biggest = scene.pos.iter().flat_map(|p| p.iter()).fold(0f32, |m, v| m.max(v.abs()));
    // 23 bits and a sign: the largest whole part decides how many bits are left for fractions.
    let whole = (biggest.max(1.0).log2().floor() as i32) + 1;
    let frac = (23 - whole).clamp(0, 12) as u8;
    let fixed = (1u32 << frac) as f32;
    let mut pos = Vec::with_capacity(n * 9);
    let mut alpha = Vec::with_capacity(n);
    let mut color = Vec::with_capacity(n * 3);
    let mut scale = Vec::with_capacity(n * 3);
    let mut rot = Vec::with_capacity(n * 4);
    let mut sh = Vec::with_capacity(n * rest_out * 3);
    for i in 0..n {
        let (p, q, flip) = in_frame(scene, Frame::RightUpBack, i);
        for v in p {
            let f = (v * fixed).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
            pos.extend_from_slice(&f.to_le_bytes()[..3]);
        }
        let c = scene.color[i];
        alpha.push(c[3]);
        for &ch in &c[..3] {
            color.push((dc(ch) * 0.15 * 255.0 + 127.5).round().clamp(0.0, 255.0) as u8);
        }
        for s in scene.scale[i] {
            scale.push(((s.max(1e-30).ln() + 10.0) * 16.0).round().clamp(0.0, 255.0) as u8);
        }
        rot.extend_from_slice(&smallest_three_spz(q));
        for k in 0..rest_out {
            // Niantic's defaults: 5 bits for the first degree, 4 for the rest.
            let bucket = if k < 3 { 1 << 3 } else { 1 << 4 };
            let sign = if flip { FLIP_YZ[k] } else { 1.0 };
            for ch in 0..3 {
                let v = scene.sh[(i * rest + k) * 3 + ch].to_f32() * sign;
                let q = (v * 128.0).round() as i32 + 128;
                sh.push(((q + bucket / 2) / bucket * bucket).clamp(0, 255) as u8);
            }
        }
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::with_capacity(n * 16), flate2::Compression::default());
    let mut head = Vec::with_capacity(16);
    head.extend_from_slice(&0x5053_474eu32.to_le_bytes());
    head.extend_from_slice(&3u32.to_le_bytes());
    head.extend_from_slice(&(n as u32).to_le_bytes());
    head.extend_from_slice(&[degree, frac, scene.antialiased as u8, 0]);
    for part in [&head, &pos, &alpha, &color, &scale, &rot, &sh] {
        gz.write_all(part).map_err(|e| e.to_string())?;
    }
    gz.finish().map_err(|e| e.to_string())
}

/// A rotation (x y z w) as SPZ's smallest three: the largest's index in the top two bits, then
/// each of the others as a sign and 9 bits, first to last from the top down.
fn smallest_three_spz(q: [f32; 4]) -> [u8; 4] {
    let n = (q.iter().map(|v| v * v).sum::<f32>()).sqrt().max(1e-12);
    let q = q.map(|v| v / n);
    let largest = (0..4).max_by(|&a, &b| q[a].abs().total_cmp(&q[b].abs())).unwrap();
    let negate = q[largest] < 0.0;
    const MASK: f32 = ((1u32 << 9) - 1) as f32;
    let mut comp = largest as u32;
    for (k, &v) in q.iter().enumerate() {
        if k == largest {
            continue;
        }
        let neg = ((v < 0.0) ^ negate) as u32;
        let mag = (MASK * (v.abs() * std::f32::consts::SQRT_2) + 0.5).min(MASK) as u32;
        comp = (comp << 10) | (neg << 9) | mag;
    }
    comp.to_le_bytes()
}

/// Splats per chunk in a compressed PLY.
const CHUNK: usize = 256;

/// PlayCanvas's compressed PLY: splats sorted along a space-filling curve so neighbours share
/// a chunk, each chunk of 256 with its bounds, each splat in four 32-bit words, and the
/// harmonics as bytes.
fn compressed_ply(scene: &Scene) -> Vec<u8> {
    let n = scene.len();
    let rest = sh_rest(scene.sh_degree);
    let order = morton_order(scene);
    let chunks = n.div_ceil(CHUNK);
    let mut out = String::from("ply\nformat binary_little_endian 1.0\ncomment Written by Filescape\n");
    out += &format!("element chunk {chunks}\n");
    for p in ["min_x", "min_y", "min_z", "max_x", "max_y", "max_z", "min_scale_x", "min_scale_y", "min_scale_z", "max_scale_x", "max_scale_y", "max_scale_z", "min_r", "min_g", "min_b", "max_r", "max_g", "max_b"] {
        out += &format!("property float {p}\n");
    }
    out += &format!("element vertex {n}\n");
    for p in ["packed_position", "packed_rotation", "packed_scale", "packed_color"] {
        out += &format!("property uint {p}\n");
    }
    if rest > 0 {
        out += &format!("element sh {n}\n");
        for k in 0..rest * 3 {
            out += &format!("property uchar f_rest_{k}\n");
        }
    }
    out += "end_header\n";
    let mut bytes = out.into_bytes();
    bytes.reserve(chunks * 72 + n * (16 + rest * 3));
    let mut table = Vec::with_capacity(chunks * 18);
    let mut words = Vec::with_capacity(n * 4);
    let log_scale = |s: f32| s.max(1e-30).ln().clamp(-20.0, 20.0);
    for c in 0..chunks {
        let ids = &order[c * CHUNK..((c + 1) * CHUNK).min(n)];
        let mut lo = [f32::INFINITY; 9];
        let mut hi = [f32::NEG_INFINITY; 9];
        for &i in ids {
            let (p, _, _) = in_frame(scene, Frame::RightDownForward, i as usize);
            let s = scene.scale[i as usize].map(log_scale);
            let col = scene.color[i as usize];
            let v = [p[0], p[1], p[2], s[0], s[1], s[2], col[0] as f32 / 255.0, col[1] as f32 / 255.0, col[2] as f32 / 255.0];
            for k in 0..9 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        table.extend_from_slice(&[lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], lo[3], lo[4], lo[5], hi[3], hi[4], hi[5], lo[6], lo[7], lo[8], hi[6], hi[7], hi[8]]);
        let unorm = |v: f32, k: usize, bits: u32| -> u32 {
            let max = ((1u32 << bits) - 1) as f32;
            let span = hi[k] - lo[k];
            let t = if span > 0.0 { (v - lo[k]) / span } else { 0.0 };
            (t.clamp(0.0, 1.0) * max + 0.5) as u32
        };
        for &i in ids {
            let i = i as usize;
            let (p, q, _) = in_frame(scene, Frame::RightDownForward, i);
            let s = scene.scale[i].map(log_scale);
            let col = scene.color[i];
            let pos = unorm(p[0], 0, 11) << 21 | unorm(p[1], 1, 10) << 11 | unorm(p[2], 2, 11);
            let sc = unorm(s[0], 3, 11) << 21 | unorm(s[1], 4, 10) << 11 | unorm(s[2], 5, 11);
            let rgb = unorm(col[0] as f32 / 255.0, 6, 8) << 24 | unorm(col[1] as f32 / 255.0, 7, 8) << 16 | unorm(col[2] as f32 / 255.0, 8, 8) << 8 | col[3] as u32;
            words.extend_from_slice(&[pos, smallest_three_playcanvas(q), sc, rgb]);
        }
    }
    for v in table {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for w in words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    if rest > 0 {
        for &i in &order {
            let (_, _, flip) = in_frame(scene, Frame::RightDownForward, i as usize);
            // Channel by channel, as PLY keeps them.
            for ch in 0..3 {
                for k in 0..rest {
                    let sign = if flip { FLIP_YZ[k] } else { 1.0 };
                    let v = scene.sh[(i as usize * rest + k) * 3 + ch].to_f32() * sign;
                    bytes.push(((v / 8.0 + 0.5) * 256.0).floor().clamp(0.0, 255.0) as u8);
                }
            }
        }
    }
    bytes
}

/// A rotation (x y z w) as PlayCanvas packs it: which of w x y z is largest in the top two
/// bits, then the other three in that order, 10 bits each.
fn smallest_three_playcanvas(q: [f32; 4]) -> u32 {
    let n = (q.iter().map(|v| v * v).sum::<f32>()).sqrt().max(1e-12);
    let wxyz = [q[3] / n, q[0] / n, q[1] / n, q[2] / n];
    let largest = (0..4).max_by(|&a, &b| wxyz[a].abs().total_cmp(&wxyz[b].abs())).unwrap();
    let sign = if wxyz[largest] < 0.0 { -1.0 } else { 1.0 };
    let mut comp = largest as u32;
    for (k, &v) in wxyz.iter().enumerate() {
        if k == largest {
            continue;
        }
        let t = (v * sign * std::f32::consts::FRAC_1_SQRT_2 + 0.5).clamp(0.0, 1.0);
        comp = (comp << 10) | (t * 1023.0 + 0.5) as u32;
    }
    comp
}

/// Splats in Morton order, so each chunk of 256 is a small patch of space and its bounds are
/// tight.
fn morton_order(scene: &Scene) -> Vec<u32> {
    let (mid, r) = super::framing(scene);
    let spread = |v: u32| -> u64 {
        let mut x = v as u64 & 0x3ff;
        x = (x | x << 16) & 0x0300_00ff;
        x = (x | x << 8) & 0x0300_f00f;
        x = (x | x << 4) & 0x030c_30c3;
        (x | x << 2) & 0x0924_9249
    };
    // Distance from the middle on a log scale, so the far-out shell is still sorted finely
    // enough to give its chunks tight bounds too.
    let squash = |t: f32| t.signum() * t.abs().ln_1p();
    let reach = scene.pos.iter().flat_map(|p| (0..3).map(move |k| ((p[k] - mid[k]) / r).abs())).filter(|v| v.is_finite()).fold(1f32, f32::max);
    let span = squash(reach);
    let cell = |v: f32, m: f32| (((squash((v - m) / r) / span) * 0.5 + 0.5).clamp(0.0, 1.0) * 1023.0) as u32;
    let mut keyed: Vec<(u64, u32)> = scene.pos.iter().enumerate().map(|(i, p)| (spread(cell(p[0], mid[0])) | spread(cell(p[1], mid[1])) << 1 | spread(cell(p[2], mid[2])) << 2, i as u32)).collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, i)| i).collect()
}

/// The standard 3DGS PLY: every value a float, in the trainers' order and axes.
fn ply(scene: &Scene) -> Vec<u8> {
    let n = scene.len();
    let rest = sh_rest(scene.sh_degree);
    let mut head = String::from("ply\nformat binary_little_endian 1.0\ncomment Written by Filescape\n");
    head += &format!("element vertex {n}\n");
    let mut names: Vec<String> = ["x", "y", "z", "nx", "ny", "nz", "f_dc_0", "f_dc_1", "f_dc_2"].map(String::from).to_vec();
    names.extend((0..rest * 3).map(|k| format!("f_rest_{k}")));
    names.extend(["opacity", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1", "rot_2", "rot_3"].map(String::from));
    for p in &names {
        head += &format!("property float {p}\n");
    }
    head += "end_header\n";
    let mut bytes = head.into_bytes();
    bytes.reserve(n * names.len() * 4);
    let mut put = |v: f32| bytes.extend_from_slice(&v.to_le_bytes());
    for i in 0..n {
        let (p, q, flip) = in_frame(scene, Frame::RightDownForward, i);
        let c = scene.color[i];
        for v in [p[0], p[1], p[2], 0.0, 0.0, 0.0, dc(c[0]), dc(c[1]), dc(c[2])] {
            put(v);
        }
        for ch in 0..3 {
            for k in 0..rest {
                let sign = if flip { FLIP_YZ[k] } else { 1.0 };
                put(scene.sh[(i * rest + k) * 3 + ch].to_f32() * sign);
            }
        }
        put(logit(c[3]));
        for s in scene.scale[i] {
            put(s.max(1e-30).ln());
        }
        // rot_0 is w.
        for v in [q[3], q[0], q[1], q[2]] {
            put(v);
        }
    }
    bytes
}

/// What reading a written file back found.
#[derive(Debug)]
pub struct Checked {
    pub splats: usize,
    /// How far splats moved at most, for all but the worst 1%, as a share of the scene's size
    /// (or of their distance from its middle, for those farther out).
    pub moved: f32,
    /// The average change in colour, in steps of 255.
    pub colour: f32,
}

impl Checked {
    pub fn summary(&self) -> String {
        let moved = if self.moved < 0.0001 { "less than 0.01%".to_string() } else { format!("{:.2}%", self.moved * 100.0) };
        format!("Read back and checked: all {} splats are there, moved by {moved} of the scene's size, colours off by {:.1} in 255 on average.", super::facts::group(self.splats), self.colour)
    }
}

/// Splats may move by at most this share of the scene's size, or of their distance from its
/// middle when farther (for all but the worst 1%).
const MOVED_MAX: f32 = 0.01;

/// Read `path` back and compare it with `scene`, which it was written from.
pub fn verify(scene: &Scene, path: &Path) -> Result<Checked, String> {
    let back = super::read(path, Limits::CONVERT).map_err(|e| format!("The new file couldn't be read back: {e}"))?;
    if back.len() != scene.len() || back.total != scene.total {
        return Err(format!("The new file has {} splats, not {}.", super::facts::group(back.len()), super::facts::group(scene.len())));
    }
    // Compressed PLY keeps splats in the order it was written in (deterministic, so worked
    // out again here); the others keep the scene's.
    let order: Vec<usize> = if back.format.starts_with("Compressed PLY") { morton_order(scene).into_iter().map(|i| i as usize).collect() } else { (0..scene.len()).collect() };
    let (oa, ob): (Vec<usize>, Vec<usize>) = (order, (0..back.len()).collect());
    let (mid, r) = super::framing(scene);
    let mut moved: Vec<f32> = Vec::with_capacity(scene.len());
    let mut colour = 0f64;
    for (&a, &b) in oa.iter().zip(&ob) {
        let pa = scene.pos[a];
        let pb = back.pos[b];
        let pb = if back.frame == scene.frame { pb } else { [pb[0], -pb[1], -pb[2]] };
        let d = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2) + (pa[2] - pb[2]).powi(2)).sqrt();
        // Far-out splats (sky) are stored more coarsely, as every format does: measured
        // against their distance from the middle.
        let far = ((pa[0] - mid[0]).powi(2) + (pa[1] - mid[1]).powi(2) + (pa[2] - mid[2]).powi(2)).sqrt();
        moved.push(if d.is_finite() { d / far.max(r) } else { f32::INFINITY });
        let (ca, cb) = (scene.color[a], back.color[b]);
        colour += (0..3).map(|k| (ca[k] as i32 - cb[k] as i32).abs()).sum::<i32>() as f64 / 3.0;
    }
    moved.sort_unstable_by(f32::total_cmp);
    let p99 = moved.get(moved.len() * 99 / 100).copied().unwrap_or(0.0);
    let checked = Checked { splats: back.len(), moved: p99, colour: (colour / scene.len().max(1) as f64) as f32 };
    if !(p99 <= MOVED_MAX) {
        return Err(format!("The new file's splats are out of place (moved by {:.1}% of the scene's size).", p99 * 100.0));
    }
    Ok(checked)
}

/// A name for the converted file next to `source` that isn't taken: "Lizard.spz", then
/// "Lizard 2.spz" and so on.
pub fn suggested_name(source: &Path, target: Target) -> PathBuf {
    let dir = source.parent().unwrap_or(Path::new("."));
    let name = source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let stem = [".compressed.ply", ".ply", ".spz", ".splat", ".ksplat", ".sog", ".glb", ".html", ".htm", ".json"].iter().find(|e| lower.ends_with(*e)).map_or(name.as_str(), |e| &name[..name.len() - e.len()]);
    let stem = if stem.eq_ignore_ascii_case("meta") { source.parent().and_then(|p| p.file_name()).map_or("Scene".to_string(), |n| n.to_string_lossy().to_string()) } else { stem.to_string() };
    (1..1000)
        .map(|k| dir.join(if k == 1 { format!("{stem}.{}", target.extension()) } else { format!("{stem} {k}.{}", target.extension()) }))
        .find(|p| !p.exists() && p != source)
        .unwrap_or_else(|| dir.join(format!("{stem} converted.{}", target.extension())))
}

/// Write `scene` as `target` to `dest`, never over `source`: to a temporary name first, then
/// moved into place, then read back and checked. A file that fails the check is removed.
pub fn convert(scene: &Scene, source: &Path, target: Target, dest: &Path) -> Result<Checked, String> {
    let same = |a: &Path, b: &Path| a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y);
    if same(source, dest) {
        return Err("The converted scene can't replace the original. Pick another name.".into());
    }
    let bytes = encode(scene, target)?;
    let dir = dest.parent().ok_or("There's no folder to write to.")?;
    let tmp = dir.join(format!(".{}.filescape-part", dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    let written = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, dest)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Couldn't write it: {e}"));
    }
    match verify(scene, dest) {
        Ok(c) => Ok(c),
        Err(e) => {
            let _ = std::fs::remove_file(dest);
            Err(format!("{e} It has been removed; the original is untouched."))
        }
    }
}
