//! glTF binary (GLB) files whose points are Gaussian splats, by Khronos's ratified
//! `KHR_gaussian_splatting` extension: position, rotation (x y z w), linear scale, linear
//! opacity and spherical harmonics as vertex attributes. glTF's axes: y up, z towards the
//! viewer.

use super::{Frame, Limits, Scene, SceneKind};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const EXT: &str = "KHR_gaussian_splatting";
/// The JSON part read at most this big.
const JSON_MAX: u32 = 64 << 20;

struct Glb {
    json: Value,
    /// Where the binary chunk starts, and its length.
    bin: Option<(u64, u64)>,
}

fn read_glb(f: &mut std::fs::File, len: u64) -> Result<Glb, String> {
    let mut h = [0u8; 20];
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    f.read_exact(&mut h).map_err(|_| "Too short to be a GLB file.".to_string())?;
    let u = |o: usize| u32::from_le_bytes(h[o..o + 4].try_into().unwrap());
    if &h[0..4] != b"glTF" || u(4) != 2 {
        return Err("Not a glTF 2 binary file.".into());
    }
    let (json_len, json_type) = (u(12), u(16));
    if json_type != 0x4E4F_534A || json_len > JSON_MAX || 20 + json_len as u64 > len {
        return Err("This GLB file's description is damaged.".into());
    }
    let mut js = vec![0u8; json_len as usize];
    f.read_exact(&mut js).map_err(|e| format!("Couldn't read it: {e}"))?;
    let json: Value = serde_json::from_slice(&js).map_err(|e| format!("This GLB file's description is damaged: {e}"))?;
    let at = 20 + json_len.div_ceil(4) as u64 * 4;
    let mut bin = None;
    if at + 8 <= len {
        let mut c = [0u8; 8];
        f.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        f.read_exact(&mut c).map_err(|e| e.to_string())?;
        let (blen, btype) = (u32::from_le_bytes(c[0..4].try_into().unwrap()) as u64, u32::from_le_bytes(c[4..8].try_into().unwrap()));
        if btype == 0x004E_4942 && at + 8 + blen <= len {
            bin = Some((at + 8, blen));
        }
    }
    Ok(Glb { json, bin })
}

/// Whether a GLB file holds Gaussian splats (its description names the extension).
pub fn is_splat_glb(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    read_glb(&mut f, len).is_ok_and(|g| g.json["extensionsUsed"].as_array().is_some_and(|a| a.iter().any(|e| e == EXT)))
}

/// One attribute's numbers, read whole from the binary chunk.
struct Accessor {
    data: Vec<u8>,
    stride: usize,
    comps: usize,
    ty: u64,
    normalized: bool,
}

impl Accessor {
    fn get(&self, row: usize, comp: usize) -> f32 {
        let size = match self.ty {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            _ => 4,
        };
        let b = &self.data[row * self.stride + comp * size..];
        let n = self.normalized;
        match self.ty {
            5126 => f32::from_le_bytes(b[..4].try_into().unwrap()),
            5121 => b[0] as f32 / if n { 255.0 } else { 1.0 },
            5120 => (b[0] as i8 as f32 / if n { 127.0 } else { 1.0 }).max(-1.0),
            5123 => u16::from_le_bytes([b[0], b[1]]) as f32 / if n { 65535.0 } else { 1.0 },
            5122 => (i16::from_le_bytes([b[0], b[1]]) as f32 / if n { 32767.0 } else { 1.0 }).max(-1.0),
            _ => f32::NAN,
        }
    }
}

fn accessor(f: &mut std::fs::File, g: &Glb, index: Option<u64>, comps: usize, count: usize) -> Result<Accessor, String> {
    let a = &g.json["accessors"][index.ok_or("A splat attribute is missing.")? as usize];
    let ty = a["componentType"].as_u64().unwrap_or(0);
    let size = match ty {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5126 => 4,
        _ => return Err("A splat attribute has a number type Filescape doesn't read.".into()),
    };
    if a.get("sparse").is_some() || a["count"].as_u64() != Some(count as u64) {
        return Err("A splat attribute's layout isn't supported.".into());
    }
    let bv = &g.json["bufferViews"][a["bufferView"].as_u64().ok_or("A splat attribute has no data.")? as usize];
    if bv["buffer"].as_u64() != Some(0) {
        return Err("This GLB file keeps its splats in another file.".into());
    }
    let (bin_at, bin_len) = g.bin.ok_or("This GLB file has no binary data.")?;
    let elem = size * comps;
    let stride = bv["byteStride"].as_u64().map_or(elem, |s| s as usize);
    let start = bv["byteOffset"].as_u64().unwrap_or(0) + a["byteOffset"].as_u64().unwrap_or(0);
    let bytes = (count.max(1) - 1) as u64 * stride as u64 + elem as u64;
    if stride < elem || start + bytes > bin_len || start + bytes > bv["byteOffset"].as_u64().unwrap_or(0) + bv["byteLength"].as_u64().unwrap_or(0) {
        return Err("A splat attribute runs outside its data.".into());
    }
    let mut data = vec![0u8; bytes as usize];
    f.seek(SeekFrom::Start(bin_at + start)).map_err(|e| e.to_string())?;
    f.read_exact(&mut data).map_err(|e| format!("Couldn't read it: {e}"))?;
    Ok(Accessor { data, stride, comps, ty, normalized: a["normalized"].as_bool().unwrap_or(false) })
}

/// How many splats a GLB file's splat scene has, from its description alone.
pub fn count(path: &Path) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let g = read_glb(&mut f, len).ok()?;
    let p = splat_prims(&g.json).into_iter().next()?;
    g.json["accessors"][p["attributes"]["POSITION"].as_u64()? as usize]["count"].as_u64()
}

fn splat_prims(json: &Value) -> Vec<&Value> {
    json["meshes"].as_array().into_iter().flatten().flat_map(|m| m["primitives"].as_array().into_iter().flatten()).filter(|p| p["extensions"].get(EXT).is_some()).collect()
}

pub fn read(path: &Path, limits: Limits) -> Result<Scene, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let g = read_glb(&mut f, len)?;
    let prims = splat_prims(&g.json);
    let p = match prims.as_slice() {
        [p] => *p,
        [] => return Err("This GLB file has no Gaussian splats.".into()),
        _ => return Err("This GLB file has several splat scenes; Filescape shows files with one.".into()),
    };
    if p.get("indices").is_some() {
        return Err("This GLB file's splats are indexed, which Filescape doesn't read yet.".into());
    }
    let attr = |n: &str| p["attributes"][n].as_u64();
    let name = |n: &str| format!("{EXT}:{n}");
    let count = g.json["accessors"][attr("POSITION").ok_or("No splat positions.")? as usize]["count"].as_u64().unwrap_or(0) as usize;
    if count == 0 || count > super::HARD_MAX_POINTS * 4 {
        return Err("This GLB file's splat count isn't believable.".into());
    }
    let pos = accessor(&mut f, &g, attr("POSITION"), 3, count)?;
    let rot = accessor(&mut f, &g, attr(&name("ROTATION")), 4, count)?;
    let scale = accessor(&mut f, &g, attr(&name("SCALE")), 3, count)?;
    let opacity = accessor(&mut f, &g, attr(&name("OPACITY")), 1, count)?;
    let dc = accessor(&mut f, &g, attr(&name("SH_DEGREE_0_COEF_0")), 3, count)?;
    let mut degree = 0u8;
    while degree < 3 && (0..(degree as usize + 1) * 2 + 1).all(|c| attr(&name(&format!("SH_DEGREE_{}_COEF_{c}", degree + 1))).is_some()) {
        degree += 1;
    }
    let mut scene = Scene::new(SceneKind::Splat, "GLB (KHR_gaussian_splatting)".into(), Frame::RightUpBack, count);
    scene.sh_degree = degree;
    if let Some(made) = g.json["asset"]["generator"].as_str() {
        scene.said.push(("Made by", made.chars().take(200).collect()));
    }
    let rest = super::sh_rest(degree);
    let keep_sh = scene.plan_sh(limits, rest);
    let mut sh = Vec::new();
    if keep_sh {
        for d in 1..=degree {
            for c in 0..(d as usize) * 2 + 1 {
                sh.push(accessor(&mut f, &g, attr(&name(&format!("SH_DEGREE_{d}_COEF_{c}"))), 3, count)?);
            }
        }
    }
    let _ = (pos.comps, rot.comps);
    let every = super::every(count as u64, limits) as usize;
    let mut ours = vec![0f32; rest * 3];
    for i in (0..count).step_by(every) {
        for (k, a) in sh.iter().enumerate() {
            for c in 0..3 {
                ours[k * 3 + c] = a.get(i, c);
            }
        }
        let p3 = |a: &Accessor| [a.get(i, 0), a.get(i, 1), a.get(i, 2)];
        scene.push_dc(p3(&pos), p3(&dc), opacity.get(i, 0).clamp(0.0, 1.0), p3(&scale).map(|v| v.max(1e-7)), [rot.get(i, 0), rot.get(i, 1), rot.get(i, 2), rot.get(i, 3)], if keep_sh { &ours } else { &[] });
    }
    Ok(scene)
}
