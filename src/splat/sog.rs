//! SOG ("Spatially Ordered Gaussians", PlayCanvas): a scene as a handful of lossless WebP
//! pictures plus `meta.json`, either loose in a folder or bundled in one `.sog` zip. Versions
//! 1 to 3 are read; layouts follow PlayCanvas splat-transform's readers (MIT). Trainer axes.

use super::{Frame, Limits, Scene, SceneKind};
use serde_json::Value;
use std::io::{Read, Seek};
use std::path::Path;

/// Each picture or `meta.json` read at most this big.
const PART_MAX: u64 = 1 << 30;

/// Where the parts come from.
enum Parts<R: Read + Seek> {
    Zip(zip::ZipArchive<R>),
    Folder(std::path::PathBuf),
}

impl<R: Read + Seek> Parts<R> {
    fn get(&mut self, name: &str) -> Result<Vec<u8>, String> {
        if name.contains("..") || name.starts_with('/') || name.contains('\\') {
            return Err(format!("This SOG scene names a part outside itself ({name})."));
        }
        let mut out = Vec::new();
        match self {
            Parts::Zip(z) => {
                let f = z.by_name(name).map_err(|_| format!("This SOG scene is missing its part {name}."))?;
                if f.size() > PART_MAX {
                    return Err(format!("A part of this SOG scene is too big ({name})."));
                }
                f.take(PART_MAX).read_to_end(&mut out).map_err(|e| format!("Couldn't read {name}: {e}"))?;
            }
            Parts::Folder(dir) => {
                let p = dir.join(name);
                let f = std::fs::File::open(&p).map_err(|_| format!("The SOG scene's part {name} isn't next to its meta.json."))?;
                f.take(PART_MAX).read_to_end(&mut out).map_err(|e| format!("Couldn't read {name}: {e}"))?;
            }
        }
        Ok(out)
    }

    fn picture(&mut self, name: &str, count: usize) -> Result<(Vec<u8>, usize, usize), String> {
        let bytes = self.get(name)?;
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::WebP).map_err(|e| format!("Couldn't read the SOG picture {name}: {e}"))?;
        let (w, h) = (img.width() as usize, img.height() as usize);
        if w * h < count {
            return Err(format!("The SOG picture {name} is too small for the scene."));
        }
        Ok((img.to_rgba8().into_raw(), w, h))
    }
}

/// A bundled `.sog` file.
pub fn read_bundle(path: &Path, limits: Limits) -> Result<Scene, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    read_zip(f, limits)
}

/// A `.sog` zip from any source (a file, or one embedded in a page).
pub fn read_zip<R: Read + Seek>(src: R, limits: Limits) -> Result<Scene, String> {
    let z = zip::ZipArchive::new(src).map_err(|e| format!("Not a SOG bundle: {e}"))?;
    read_parts(Parts::Zip(z), limits)
}

/// A loose scene: its `meta.json`, with the pictures beside it.
pub fn read_folder(meta: &Path, limits: Limits) -> Result<Scene, String> {
    let dir = meta.parent().ok_or("No folder.")?.to_path_buf();
    read_parts::<std::fs::File>(Parts::Folder(dir), limits)
}

/// Whether a `meta.json` is a SOG scene's: it has `means` and `quats`.
pub fn is_meta(text: &[u8]) -> bool {
    let s = String::from_utf8_lossy(text);
    s.contains("\"means\"") && s.contains("\"quats\"") && s.contains("\"sh0\"")
}

fn nums(v: &Value) -> Vec<f32> {
    v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default()
}

fn files(v: &Value) -> Vec<String> {
    v["files"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Inverse of SOG's log transform of positions.
fn unlog(v: f32) -> f32 {
    let e = v.abs().exp() - 1.0;
    if v < 0.0 { -e } else { e }
}

/// The smallest-three rotation of versions 1 and 2 (tag 252–255 says which is largest), as
/// x y z w.
fn quat_tag(px: u8, py: u8, pz: u8, tag: u8) -> [f32; 4] {
    if !(252..=255).contains(&tag) {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let big = (tag - 252) as usize;
    let s = std::f32::consts::SQRT_2;
    let v = [(px as f32 / 255.0 * 2.0 - 1.0) / s, (py as f32 / 255.0 * 2.0 - 1.0) / s, (pz as f32 / 255.0 * 2.0 - 1.0) / s];
    let mut wxyz = [0f32; 4];
    let mut k = 0;
    for (slot, out) in wxyz.iter_mut().enumerate() {
        if slot != big {
            *out = v[k];
            k += 1;
        }
    }
    wxyz[big] = (1.0 - (v[0] * v[0] + v[1] * v[1] + v[2] * v[2])).max(0.0).sqrt();
    [wxyz[1], wxyz[2], wxyz[3], wxyz[0]]
}

/// Version 3's rotation: an exponential map in three bytes, as x y z w.
fn quat_expmap(a: u8, b: u8, c: u8) -> [f32; 4] {
    let p = [(a as f32 - 128.0) / 127.0, (b as f32 - 128.0) / 127.0, (c as f32 - 128.0) / 127.0];
    let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    let t = std::f32::consts::FRAC_PI_2 * r;
    let k = if r > 1e-12 { t.sin() / r } else { std::f32::consts::FRAC_PI_2 };
    [p[0] * k, p[1] * k, p[2] * k, t.cos()]
}

fn read_parts<R: Read + Seek>(mut parts: Parts<R>, limits: Limits) -> Result<Scene, String> {
    let meta_bytes = parts.get("meta.json")?;
    let meta: Value = serde_json::from_slice(&meta_bytes).map_err(|e| format!("This SOG scene's meta.json is damaged: {e}"))?;
    let version = meta["version"].as_u64().unwrap_or(1);
    let count = if version >= 2 { meta["count"].as_u64() } else { meta["means"]["shape"][0].as_u64() }.ok_or("This SOG scene doesn't say how many splats it has.")? as usize;
    if count == 0 || count > 1 << 28 {
        return Err("This SOG scene's splat count isn't believable.".into());
    }
    if version > 3 {
        return Err(format!("SOG version {version} is newer than Filescape knows."));
    }
    let mut scene = Scene::new(SceneKind::Splat, format!("SOG v{version}"), Frame::RightDownForward, count);
    if let Some(c) = meta["comments"].as_array().and_then(|a| a.first()).and_then(|c| c.as_str()) {
        scene.said.push(("Comment", c.chars().take(200).collect()));
    }
    let mf = files(&meta["means"]);
    let (lo, _, _) = parts.picture(mf.first().ok_or("No positions.")?, count)?;
    let (hi, _, _) = parts.picture(mf.get(1).ok_or("No positions.")?, count)?;
    let (mins, maxs) = (nums(&meta["means"]["mins"]), nums(&meta["means"]["maxs"]));
    if mins.len() < 3 || maxs.len() < 3 {
        return Err("This SOG scene's bounds are missing.".into());
    }
    let (q, _, _) = parts.picture(files(&meta["quats"]).first().ok_or("No rotations.")?, count)?;
    let (sc, _, _) = parts.picture(files(&meta["scales"]).first().ok_or("No scales.")?, count)?;
    let (c0, _, _) = parts.picture(files(&meta["sh0"]).first().ok_or("No colours.")?, count)?;
    // Scales and colours: codebooks of 256 (v2, v3) or ranges (v1).
    let lut = |part: &str, ch: usize| -> Vec<f32> {
        if version >= 2 {
            let mut v = nums(&meta[part]["codebook"]);
            v.resize(256, 0.0);
            v
        } else {
            let (a, b) = (nums(&meta[part]["mins"]), nums(&meta[part]["maxs"]));
            let (a, b) = (a.get(ch).copied().unwrap_or(0.0), b.get(ch).copied().unwrap_or(0.0));
            (0..256).map(|i| a + (b - a) * i as f32 / 255.0).collect()
        }
    };
    let scale_lut: Vec<Vec<f32>> = (0..3).map(|c| lut("scales", c)).collect();
    let col_lut: Vec<Vec<f32>> = (0..4).map(|c| lut("sh0", c)).collect();
    // Higher harmonics (v2, v3): a palette of coefficient sets, each splat naming one.
    let bands = if version >= 2 { meta["shN"]["bands"].as_u64().unwrap_or(0) as u8 } else { 0 };
    if version == 1 && meta.get("shN").is_some() {
        scene.notes.push("Its view-dependent colour (an older SOG layout) isn't shown".into());
    }
    let bands = bands.min(3);
    let rest = super::sh_rest(bands);
    scene.sh_degree = bands;
    let keep_sh = rest > 0 && scene.plan_sh(limits, rest);
    let mut palette = None;
    if keep_sh {
        let sf = files(&meta["shN"]);
        let (cen, cw, ch) = parts.picture(sf.first().ok_or("No harmonics.")?, 0)?;
        let (lab, _, _) = parts.picture(sf.get(1).ok_or("No harmonics.")?, count)?;
        if cw != 64 * rest {
            return Err("This SOG scene's harmonics palette has the wrong width.".into());
        }
        let mut book = nums(&meta["shN"]["codebook"]);
        book.resize(256, 0.0);
        let n = meta["shN"]["count"].as_u64().unwrap_or(0) as usize;
        palette = Some((cen, cw, ch, lab, book, n));
    }
    let opacity = |b: u8| -> f32 {
        match version {
            3 => ((b as f32 / 204.0).powi(2)).min(1.0),
            2 => b as f32 / 255.0,
            _ => super::sigmoid(col_lut[3][b as usize]),
        }
    };
    let every = super::every(count as u64, limits) as usize;
    let mut ours = vec![0f32; rest * 3];
    for i in (0..count).step_by(every) {
        let o = i * 4;
        let m = |k: usize| unlog(mins[k] + (maxs[k] - mins[k]).max(f32::MIN_POSITIVE) * ((lo[o + k] as u32 | (hi[o + k] as u32) << 8) as f32 / 65535.0));
        let pos = [m(0), m(1), m(2)];
        let rot = if version == 3 { quat_expmap(q[o], q[o + 1], q[o + 2]) } else { quat_tag(q[o], q[o + 1], q[o + 2], q[o + 3]) };
        let scale = [scale_lut[0][sc[o] as usize].exp(), scale_lut[1][sc[o + 1] as usize].exp(), scale_lut[2][sc[o + 2] as usize].exp()];
        let dc = [col_lut[0][c0[o] as usize], col_lut[1][c0[o + 1] as usize], col_lut[2][c0[o + 2] as usize]];
        if let Some((cen, cw, ch, lab, book, n)) = &palette {
            ours.iter_mut().for_each(|v| *v = 0.0);
            let label = lab[o] as usize | (lab[o + 1] as usize) << 8 | if version == 3 { (lab[o + 2] as usize) << 16 } else { 0 };
            if label < *n {
                let (cy, cx0) = (label / 64, (label % 64) * rest);
                for j in 0..rest {
                    let cx = cx0 + j;
                    if cy < *ch && cx < *cw {
                        let at = (cy * cw + cx) * 4;
                        for c in 0..3 {
                            ours[j * 3 + c] = book[cen[at + c] as usize];
                        }
                    }
                }
            }
        }
        scene.push_dc(pos, dc, opacity(c0[o + 3]), scale, rot, if keep_sh { &ours } else { &[] });
    }
    Ok(scene)
}
