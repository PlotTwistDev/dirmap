//! PLY: the header classified (mesh, point cloud, splat scene) and splat scenes or point clouds
//! read, streaming, with counts checked against the file's length first.
//!
//! A 3DGS PLY has one `vertex` element with `x y z`, `f_dc_0..2` (base colour), `f_rest_*`
//! (higher spherical harmonics, red's first, then green's, then blue's), `opacity` (a logit),
//! `scale_0..2` (logs) and `rot_0..3` (w x y z). 2D Gaussian scenes leave out `scale_2`.

use super::{Frame, Limits, PlyKind, Scene, SceneKind, SH_C0, sigmoid, to_u8};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Headers longer than this aren't PLY headers.
const MAX_HEADER: usize = 256 << 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Encoding {
    Ascii,
    Little,
    Big,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Type {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Type {
    fn parse(s: &str) -> Option<Type> {
        Some(match s {
            "char" | "int8" => Type::I8,
            "uchar" | "uint8" => Type::U8,
            "short" | "int16" => Type::I16,
            "ushort" | "uint16" => Type::U16,
            "int" | "int32" => Type::I32,
            "uint" | "uint32" => Type::U32,
            "float" | "float32" => Type::F32,
            "double" | "float64" => Type::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Type::I8 | Type::U8 => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::F64 => 8,
        }
    }

    fn get(self, b: &[u8], big: bool) -> f32 {
        macro_rules! num {
            ($t:ty, $n:expr) => {{
                let a: [u8; $n] = b[..$n].try_into().unwrap();
                if big { <$t>::from_be_bytes(a) } else { <$t>::from_le_bytes(a) }
            }};
        }
        match self {
            Type::I8 => b[0] as i8 as f32,
            Type::U8 => b[0] as f32,
            Type::I16 => num!(i16, 2) as f32,
            Type::U16 => num!(u16, 2) as f32,
            Type::I32 => num!(i32, 4) as f32,
            Type::U32 => num!(u32, 4) as f32,
            Type::F32 => num!(f32, 4),
            Type::F64 => num!(f64, 8) as f32,
        }
    }
}

#[derive(Debug)]
struct Property {
    name: String,
    ty: Type,
    /// For list properties: the count's type.
    list: Option<Type>,
}

#[derive(Debug)]
struct Element {
    name: String,
    count: u64,
    props: Vec<Property>,
}

impl Element {
    fn has(&self, name: &str) -> bool {
        self.props.iter().any(|p| p.name == name)
    }

    /// Bytes per record, when every property has a fixed size.
    fn stride(&self) -> Option<usize> {
        self.props.iter().map(|p| if p.list.is_some() { None } else { Some(p.ty.size()) }).sum()
    }
}

/// A PLY header.
#[derive(Debug)]
pub struct Header {
    encoding: Encoding,
    elements: Vec<Element>,
    comments: Vec<String>,
    /// Where the data starts.
    data_at: u64,
    file_len: u64,
}

impl Header {
    /// Read and parse the header at the start of a file.
    pub fn read(path: &Path) -> Result<Header, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut head = Vec::new();
        file.take(MAX_HEADER as u64).read_to_end(&mut head).map_err(|e| format!("Couldn't read it: {e}"))?;
        Header::parse(&head, file_len)
    }

    fn parse(head: &[u8], file_len: u64) -> Result<Header, String> {
        if !head.starts_with(b"ply") {
            return Err("Not a PLY file.".into());
        }
        let end = head.windows(10).position(|w| w == b"end_header").ok_or("This PLY file's header is missing or too long.")?;
        let mut data_at = end + 10;
        // The header ends at the line break after end_header (\n, or \r\n from some writers).
        if head.get(data_at) == Some(&b'\r') {
            data_at += 1;
        }
        if head.get(data_at) == Some(&b'\n') {
            data_at += 1;
        }
        let text = String::from_utf8_lossy(&head[..end]);
        let mut encoding = None;
        let mut elements: Vec<Element> = Vec::new();
        let mut comments = Vec::new();
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.as_slice() {
                ["format", f, ..] => {
                    encoding = Some(match *f {
                        "ascii" => Encoding::Ascii,
                        "binary_little_endian" => Encoding::Little,
                        "binary_big_endian" => Encoding::Big,
                        _ => return Err(format!("Unknown PLY encoding \"{f}\".")),
                    })
                }
                ["comment", ..] | ["obj_info", ..] => {
                    let c = line.trim_start().splitn(2, char::is_whitespace).nth(1).unwrap_or("").trim();
                    if !c.is_empty() && comments.len() < 32 {
                        comments.push(c.chars().take(200).collect());
                    }
                }
                ["element", name, n] => {
                    let count = n.parse::<u64>().map_err(|_| format!("Bad element count \"{n}\" in the header."))?;
                    elements.push(Element { name: name.to_string(), count, props: Vec::new() });
                }
                ["property", "list", ct, it, name] => {
                    let e = elements.last_mut().ok_or("A property before any element.")?;
                    let (Some(ct), Some(it)) = (Type::parse(ct), Type::parse(it)) else { return Err("Unknown property type in the header.".into()) };
                    e.props.push(Property { name: name.to_string(), ty: it, list: Some(ct) });
                }
                ["property", t, name] => {
                    let e = elements.last_mut().ok_or("A property before any element.")?;
                    let ty = Type::parse(t).ok_or_else(|| format!("Unknown property type \"{t}\"."))?;
                    e.props.push(Property { name: name.to_string(), ty, list: None });
                }
                _ => {}
            }
        }
        let encoding = encoding.ok_or("This PLY file doesn't say how it's stored.")?;
        Ok(Header { encoding, elements, comments, data_at: data_at as u64, file_len })
    }

    fn vertex(&self) -> Option<&Element> {
        self.elements.iter().find(|e| e.name == "vertex")
    }

    pub fn kind(&self) -> PlyKind {
        if self.elements.iter().any(|e| e.name == "face" && e.count > 0) {
            return PlyKind::Mesh;
        }
        if self.elements.iter().any(|e| e.name == "chunk") && self.vertex().is_some_and(|v| v.has("packed_position")) {
            return PlyKind::CompressedSplat;
        }
        let Some(v) = self.vertex() else { return PlyKind::Unknown };
        if !(v.has("x") && v.has("y") && v.has("z")) {
            return PlyKind::Unknown;
        }
        let splat = ["f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "rot_0", "rot_1", "rot_2", "rot_3"].iter().all(|p| v.has(p));
        if !splat {
            return PlyKind::PointCloud;
        }
        // Space-Time Gaussians: a temporal centre and polynomial motion per splat.
        if v.has("trbf_center") || v.has("motion_0") || v.has("t") {
            return PlyKind::Splat4d;
        }
        PlyKind::Splat
    }
}

/// What to pull out of each vertex, as property indices.
struct Wanted {
    xyz: [usize; 3],
    dc: Option<[usize; 3]>,
    rgb: Option<([usize; 3], f32)>,
    opacity: Option<usize>,
    scale: Option<[Option<usize>; 3]>,
    rot: Option<[usize; 4]>,
    rest: Vec<usize>,
}

/// Read a splat scene or point cloud from a PLY file.
pub fn read(path: &Path, limits: Limits) -> Result<Scene, String> {
    let head = Header::read(path)?;
    let kind = head.kind();
    let scene_kind = match kind {
        PlyKind::Splat => SceneKind::Splat,
        PlyKind::Splat4d => SceneKind::Splat4d,
        PlyKind::PointCloud => SceneKind::PointCloud,
        PlyKind::Mesh => return Err("This PLY file is a mesh.".into()),
        PlyKind::CompressedSplat => return Err("This is a compressed PLY splat scene (PlayCanvas's format). Filescape will show these in a later version.".into()),
        PlyKind::Unknown => return Err("This PLY file has no points to show.".into()),
    };
    // Elements before the vertices are skipped, so they need a fixed size in binary files.
    let mut skip = 0u64;
    let mut vertex = None;
    for e in &head.elements {
        if e.name == "vertex" {
            vertex = Some(e);
            break;
        }
        if head.encoding != Encoding::Ascii {
            let stride = e.stride().ok_or("This PLY file has a list before its points, which Filescape can't skip.")?;
            skip = skip.checked_add(e.count.checked_mul(stride as u64).ok_or("Bad PLY header.")?).ok_or("Bad PLY header.")?;
        } else if e.count > 0 {
            return Err("This text PLY file has other data before its points.".into());
        }
    }
    let v = vertex.ok_or("No points in this PLY file.")?;
    if v.props.iter().any(|p| p.list.is_some()) {
        return Err("This PLY file's points have lists, which Filescape doesn't read.".into());
    }
    let idx = |n: &str| v.props.iter().position(|p| p.name == n);
    let want = Wanted {
        xyz: [idx("x").unwrap(), idx("y").unwrap(), idx("z").unwrap()],
        dc: match (idx("f_dc_0"), idx("f_dc_1"), idx("f_dc_2")) {
            (Some(a), Some(b), Some(c)) => Some([a, b, c]),
            _ => None,
        },
        rgb: match (idx("red").or(idx("diffuse_red")), idx("green").or(idx("diffuse_green")), idx("blue").or(idx("diffuse_blue"))) {
            (Some(a), Some(b), Some(c)) => {
                // Byte colours are 0–255; float ones 0–1.
                let full = match v.props[a].ty {
                    Type::U8 | Type::I8 => 255.0,
                    Type::U16 | Type::I16 => 65535.0,
                    _ => 1.0,
                };
                Some(([a, b, c], full))
            }
            _ => None,
        },
        opacity: idx("opacity"),
        scale: idx("scale_0").map(|a| [Some(a), idx("scale_1"), idx("scale_2")]),
        rot: match (idx("rot_0"), idx("rot_1"), idx("rot_2"), idx("rot_3")) {
            (Some(a), Some(b), Some(c), Some(d)) => Some([a, b, c, d]),
            _ => None,
        },
        rest: {
            let mut r = Vec::new();
            while let Some(i) = idx(&format!("f_rest_{}", r.len())) {
                r.push(i);
            }
            r
        },
    };
    let is_splat = scene_kind != SceneKind::PointCloud;
    // f_rest holds 3 × (coefficients beyond the first) values.
    let degree = match want.rest.len() {
        0..9 => if want.rest.is_empty() { 0 } else { 1 },
        9..24 => 1,
        24..45 => 2,
        45..72 => 3,
        _ => 4,
    };
    let rest_per_channel = super::sh_rest(degree);
    let total = v.count;
    let mut scene = Scene::new(scene_kind, "PLY".into(), Frame::RightDownForward, total.min(usize::MAX as u64) as usize);
    scene.sh_degree = if is_splat { degree } else { 0 };
    scene.antialiased = head.comments.iter().any(|c| c.contains("SplatRenderMode: mip"));
    for c in &head.comments {
        if let Some(v) = c.strip_prefix("Generated by ") {
            scene.said.push(("Made by", v.to_string()));
        } else if c.starts_with("Postshot") {
            scene.said.push(("Made by", c.to_string()));
        }
    }
    if scene.said.is_empty() && !head.comments.is_empty() {
        scene.said.push(("Comment", head.comments[0].clone()));
    }
    if v.has("scale_0") && !v.has("scale_2") {
        scene.notes.push("Flat (2D Gaussian) splats".into());
    }
    let keep_sh = is_splat && limits.keeps_sh(scene.total as u64, rest_per_channel);
    if is_splat && limits.keep_sh && rest_per_channel > 0 && !keep_sh {
        scene.notes.push(super::SH_DROPPED.into());
    }
    let cap = limits.max_points.min(super::HARD_MAX_POINTS) as u64;
    let mut rest_vals = vec![0f32; if keep_sh { rest_per_channel * 3 } else { 0 }];

    let mut take = |scene: &mut Scene, vals: &dyn Fn(usize) -> f32| {
        let pos = [vals(want.xyz[0]), vals(want.xyz[1]), vals(want.xyz[2])];
        let alpha = want.opacity.map_or(1.0, |i| sigmoid(vals(i)));
        let rgb = if let Some(dc) = want.dc {
            [vals(dc[0]) * SH_C0 + 0.5, vals(dc[1]) * SH_C0 + 0.5, vals(dc[2]) * SH_C0 + 0.5]
        } else if let Some((c, full)) = want.rgb {
            [vals(c[0]) / full, vals(c[1]) / full, vals(c[2]) / full]
        } else {
            [0.8, 0.8, 0.8]
        };
        let color = [to_u8(rgb[0]), to_u8(rgb[1]), to_u8(rgb[2]), to_u8(alpha)];
        let (scale, rot) = if is_splat {
            let s = want.scale.unwrap();
            let get = |o: Option<usize>| o.map_or(1e-7, |i| vals(i).exp());
            let r = want.rot.unwrap();
            // rot_0 is w.
            (Some([get(s[0]), get(s[1]), get(s[2])]), Some([vals(r[1]), vals(r[2]), vals(r[3]), vals(r[0])]))
        } else {
            (None, None)
        };
        if keep_sh {
            // PLY order: all of red's, then green's, then blue's. Ours: coefficient by
            // coefficient, r g b inside.
            for k in 0..rest_per_channel {
                for ch in 0..3 {
                    rest_vals[k * 3 + ch] = want.rest.get(ch * (want.rest.len() / 3) + k).map_or(0.0, |&i| vals(i));
                }
            }
        }
        scene.push(pos, color, scale, rot, &rest_vals);
    };

    match head.encoding {
        Encoding::Ascii => {
            let file = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
            let mut r = BufReader::new(file);
            r.seek(SeekFrom::Start(head.data_at)).map_err(|e| e.to_string())?;
            // Text PLYs are rarely big; still keep to the limit, evenly.
            let every = total.div_ceil(cap.max(1)).max(1);
            let mut line = String::new();
            let mut vals = vec![0f32; v.props.len()];
            let mut n = 0u64;
            while n < total {
                line.clear();
                if r.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                    scene.notes.push(format!("The file ends early: {n} of {total} points"));
                    break;
                }
                if line.trim().is_empty() {
                    continue;
                }
                if n % every == 0 {
                    for (slot, w) in vals.iter_mut().zip(line.split_ascii_whitespace()) {
                        *slot = w.parse().unwrap_or(f32::NAN);
                    }
                    take(&mut scene, &|i| vals[i]);
                }
                n += 1;
            }
        }
        Encoding::Little | Encoding::Big => {
            let big = head.encoding == Encoding::Big;
            let stride = v.stride().unwrap();
            let offsets: Vec<usize> = v.props.iter().scan(0, |at, p| {
                let o = *at;
                *at += p.ty.size();
                Some(o)
            }).collect();
            let start = head.data_at.checked_add(skip).ok_or("Bad PLY header.")?;
            let room = head.file_len.saturating_sub(start) / stride.max(1) as u64;
            let present = room.min(total);
            if present < total {
                scene.notes.push(format!("The file ends early: {present} of {total} points are there"));
            }
            let file = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
            let mut r = BufReader::with_capacity(1 << 20, file);
            r.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
            // Read in blocks; when there are more points than the limit, read evenly spaced
            // blocks and skip the rest, so a sample of a huge file is quick.
            const BLOCK: u64 = 4096;
            let blocks = present.div_ceil(BLOCK);
            let keep_blocks = cap.div_ceil(BLOCK).max(1);
            let mut buf = vec![0u8; BLOCK as usize * stride];
            let mut b = 0u64;
            let mut at = start;
            let mut kept_blocks = 0u64;
            while b < blocks {
                // Block b is read when it's the next of keep_blocks spread over all of them.
                let want_b = if blocks <= keep_blocks { b } else { (2 * kept_blocks + 1) * blocks / (2 * keep_blocks) };
                if want_b >= blocks {
                    break;
                }
                let pos = start + want_b * BLOCK * stride as u64;
                if pos != at {
                    r.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
                }
                let n = (present - want_b * BLOCK).min(BLOCK) as usize;
                let bytes = &mut buf[..n * stride];
                r.read_exact(bytes).map_err(|e| format!("Couldn't read it: {e}"))?;
                at = pos + bytes.len() as u64;
                for rec in bytes.chunks_exact(stride) {
                    take(&mut scene, &|i| v.props[i].ty.get(&rec[offsets[i]..], big));
                }
                kept_blocks += 1;
                b = want_b + 1;
                if scene.len() as u64 >= cap {
                    break;
                }
            }
        }
    }
    Ok(scene)
}

#[cfg(test)]
pub(super) fn parse_for_test(head: &[u8]) -> Result<PlyKind, String> {
    Header::parse(head, 0).map(|h| h.kind())
}
