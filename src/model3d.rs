//! 3D models: read (STL, OBJ, PLY, glTF/GLB, 3MF) into one list of triangles, and drawn here in
//! software as plain shaded clay, to turn about in the preview and for tiles and icons. No GPU,
//! no textures and nothing in the file is run. glTF is read with the gltf crate (MIT or
//! Apache-2.0); the others with small readers of our own.

use std::io::Read;
use std::path::Path;

/// Files larger than this aren't read.
pub const MAX_FILE: u64 = 256 << 20;
/// Triangles kept at most.
const MAX_TRIS: usize = 6_000_000;

pub struct Mesh {
    /// Corners, centred on the origin and scaled to fit a unit sphere.
    pub points: Vec<[f32; 3]>,
    pub tris: Vec<[u32; 3]>,
    /// The model's size in its own units (x, y, z), before scaling.
    #[allow(dead_code)] // shown through the facts; kept for the tests and later use
    pub extent: [f32; 3],
    pub facts: Vec<(&'static str, String)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Angle {
    /// Turn about the up axis, and tilt towards the viewer, in radians.
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for Angle {
    fn default() -> Self {
        Angle { yaw: -0.6, pitch: 0.45 }
    }
}

fn ext(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

pub fn read(path: &Path) -> Result<Mesh, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't read it: {e}"))?.len();
    if size > MAX_FILE {
        return Err(format!("Too big to show ({}).", crate::scan::human(size)));
    }
    let e = ext(path);
    let (points, tris, mut facts) = match e.as_str() {
        "gltf" | "glb" => gltf_mesh(path)?,
        "3mf" => threemf(path)?,
        _ => {
            let data = std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))?;
            match e.as_str() {
                "stl" => stl(&data)?,
                "obj" => obj(&data)?,
                "ply" => ply(&data)?,
                _ => return Err("Not a 3D model Filescape can read.".into()),
            }
        }
    };
    if tris.is_empty() {
        return Err("This model has no faces to show.".into());
    }
    let (points, extent) = normalise(points);
    facts.insert(0, ("Triangles", group(tris.len())));
    facts.insert(1, ("Points", group(points.len())));
    let [x, y, z] = extent;
    facts.push(("Size", format!("{} × {} × {} (in the model's units)", num(x), num(y), num(z))));
    Ok(Mesh { points, tris, extent, facts })
}

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn num(v: f32) -> String {
    if v >= 100.0 { format!("{v:.0}") } else if v >= 1.0 { format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string() } else { format!("{v:.3}") }
}

/// Centre the points and scale them into a unit sphere; the extent before scaling.
fn normalise(mut pts: Vec<[f32; 3]>) -> (Vec<[f32; 3]>, [f32; 3]) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in pts.iter().filter(|p| p.iter().all(|v| v.is_finite())) {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if lo[0] > hi[0] {
        return (pts, [0.0; 3]);
    }
    let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let r = (ext[0] * ext[0] + ext[1] * ext[1] + ext[2] * ext[2]).sqrt() / 2.0;
    let s = if r > 0.0 { 1.0 / r } else { 1.0 };
    for p in &mut pts {
        for k in 0..3 {
            p[k] = if p[k].is_finite() { (p[k] - c[k]) * s } else { 0.0 };
        }
    }
    (pts, ext)
}

type Parsed = (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<(&'static str, String)>);

fn too_many() -> String {
    "This model has too many triangles to show.".into()
}

fn stl(d: &[u8]) -> Result<Parsed, String> {
    // Binary: an 80-byte header, a count, then 50 bytes a triangle. ASCII files start "solid"
    // but so do some binary ones, so the size decides.
    if d.len() >= 84 {
        let n = u32::from_le_bytes([d[80], d[81], d[82], d[83]]) as usize;
        if 84 + n * 50 == d.len() {
            if n > MAX_TRIS {
                return Err(too_many());
            }
            let mut pts = Vec::with_capacity(n * 3);
            let mut tris = Vec::with_capacity(n);
            for t in 0..n {
                let b = &d[84 + t * 50 + 12..84 + t * 50 + 48];
                for v in 0..3 {
                    let f = |k: usize| f32::from_le_bytes(b[v * 12 + k * 4..v * 12 + k * 4 + 4].try_into().unwrap());
                    pts.push([f(0), f(1), f(2)]);
                }
                let i = (t * 3) as u32;
                tris.push([i, i + 1, i + 2]);
            }
            return Ok((pts, tris, vec![("Format", "STL (binary)".into())]));
        }
    }
    let text = String::from_utf8_lossy(d);
    if !text.trim_start().starts_with("solid") {
        return Err("This STL file couldn't be read.".into());
    }
    let mut pts = Vec::new();
    for line in text.lines() {
        let mut w = line.split_whitespace();
        if w.next() == Some("vertex") {
            let v: Vec<f32> = w.take(3).filter_map(|x| x.parse().ok()).collect();
            if v.len() == 3 {
                pts.push([v[0], v[1], v[2]]);
            }
        }
    }
    pts.truncate(pts.len() / 3 * 3);
    if pts.len() / 3 > MAX_TRIS {
        return Err(too_many());
    }
    let tris = (0..pts.len() as u32 / 3).map(|t| [t * 3, t * 3 + 1, t * 3 + 2]).collect();
    Ok((pts, tris, vec![("Format", "STL (text)".into())]))
}

fn obj(d: &[u8]) -> Result<Parsed, String> {
    // Windows object files share the .obj extension: those aren't text.
    if d.iter().take(4096).any(|&b| b == 0) {
        return Err("This .obj file is a compiled program's object file, not a 3D model.".into());
    }
    let text = String::from_utf8_lossy(d);
    let mut pts: Vec<[f32; 3]> = Vec::new();
    let mut tris = Vec::new();
    let (mut groups, mut materials) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for line in text.lines() {
        let mut w = line.split_whitespace();
        match w.next() {
            Some("v") => {
                let v: Vec<f32> = w.take(3).filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    pts.push([v[0], v[1], v[2]]);
                }
            }
            Some("f") => {
                // "f 1/2/3 4/5/6 …", negative indices counting back; fans for polygons.
                let n = pts.len() as i64;
                let idx: Vec<u32> = w
                    .filter_map(|c| c.split('/').next()?.parse::<i64>().ok())
                    .filter_map(|i| {
                        let i = if i < 0 { n + i } else { i - 1 };
                        (0..n).contains(&i).then_some(i as u32)
                    })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    tris.push([idx[0], idx[k], idx[k + 1]]);
                }
                if tris.len() > MAX_TRIS {
                    return Err(too_many());
                }
            }
            Some("o" | "g") => {
                groups.insert(w.collect::<Vec<_>>().join(" "));
            }
            Some("usemtl") => {
                materials.insert(w.collect::<Vec<_>>().join(" "));
            }
            _ => {}
        }
    }
    if pts.is_empty() {
        return Err("This .obj file has no 3D points in it.".into());
    }
    let mut facts = vec![("Format", "Wavefront OBJ".to_string())];
    if groups.len() > 1 {
        facts.push(("Parts", group(groups.len())));
    }
    if !materials.is_empty() {
        facts.push(("Materials", group(materials.len())));
    }
    Ok((pts, tris, facts))
}

fn ply(d: &[u8]) -> Result<Parsed, String> {
    let end = d.windows(11).position(|w| w == b"end_header\n" || w == b"end_header\r").ok_or("This PLY file couldn't be read.")?;
    let head = String::from_utf8_lossy(&d[..end]);
    let mut body = &d[end + 10..];
    while let [b'\r' | b'\n', rest @ ..] = body {
        body = rest;
    }
    let mut format = "";
    // Elements, each with its count and properties: (name, type, list count type).
    let mut elems: Vec<(String, usize, Vec<(String, String, Option<String>)>)> = Vec::new();
    for line in head.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["format", f, ..] => format = if *f == "ascii" { "ascii" } else if *f == "binary_little_endian" { "le" } else { "be" },
            ["element", name, n] => elems.push((name.to_string(), n.parse().unwrap_or(0), Vec::new())),
            ["property", "list", ct, it, name] => {
                if let Some(e) = elems.last_mut() {
                    e.2.push((name.to_string(), it.to_string(), Some(ct.to_string())));
                }
            }
            ["property", t, name] => {
                if let Some(e) = elems.last_mut() {
                    e.2.push((name.to_string(), t.to_string(), None));
                }
            }
            _ => {}
        }
    }
    let size_of = |t: &str| match t {
        "char" | "uchar" | "int8" | "uint8" => 1,
        "short" | "ushort" | "int16" | "uint16" => 2,
        "int" | "uint" | "float" | "int32" | "uint32" | "float32" => 4,
        "double" | "float64" => 8,
        _ => 0,
    };
    let mut pts = Vec::new();
    let mut tris = Vec::new();
    let mut words = if format == "ascii" { Some(std::str::from_utf8(body).unwrap_or("").split_ascii_whitespace()) } else { None };
    let mut at = 0usize;
    let mut value = |t: &str| -> Option<f64> {
        if let Some(w) = words.as_mut() {
            return w.next()?.parse().ok();
        }
        let n = size_of(t);
        let b = body.get(at..at + n)?;
        at += n;
        let arr = |k: usize| -> [u8; 8] {
            let mut a = [0u8; 8];
            if format == "le" { a[..k].copy_from_slice(b) } else { a[..k].copy_from_slice(b); a[..k].reverse() }
            a
        };
        Some(match t {
            "char" | "int8" => b[0] as i8 as f64,
            "uchar" | "uint8" => b[0] as f64,
            "short" | "int16" => i16::from_le_bytes(arr(2)[..2].try_into().ok()?) as f64,
            "ushort" | "uint16" => u16::from_le_bytes(arr(2)[..2].try_into().ok()?) as f64,
            "int" | "int32" => i32::from_le_bytes(arr(4)[..4].try_into().ok()?) as f64,
            "uint" | "uint32" => u32::from_le_bytes(arr(4)[..4].try_into().ok()?) as f64,
            "float" | "float32" => f32::from_le_bytes(arr(4)[..4].try_into().ok()?) as f64,
            "double" | "float64" => f64::from_le_bytes(arr(8)),
            _ => return None,
        })
    };
    'elements: for (name, count, props) in &elems {
        for _ in 0..*count {
            let mut p = [0f32; 3];
            for (pname, t, list) in props {
                match list {
                    Some(ct) => {
                        let Some(n) = value(ct) else { break 'elements };
                        let n = n as usize;
                        if n > 64 {
                            break 'elements;
                        }
                        let mut idx = Vec::with_capacity(n);
                        for _ in 0..n {
                            let Some(i) = value(t) else { break 'elements };
                            idx.push(i as u32);
                        }
                        if name == "face" && (pname == "vertex_indices" || pname == "vertex_index") {
                            for k in 1..idx.len().saturating_sub(1) {
                                tris.push([idx[0], idx[k], idx[k + 1]]);
                            }
                            if tris.len() > MAX_TRIS {
                                return Err(too_many());
                            }
                        }
                    }
                    None => {
                        let Some(v) = value(t) else { break 'elements };
                        if name == "vertex" {
                            match pname.as_str() {
                                "x" => p[0] = v as f32,
                                "y" => p[1] = v as f32,
                                "z" => p[2] = v as f32,
                                _ => {}
                            }
                        }
                    }
                }
            }
            if name == "vertex" {
                pts.push(p);
            }
        }
    }
    let n = pts.len() as u32;
    tris.retain(|t: &[u32; 3]| t.iter().all(|&i| i < n));
    let what = match format {
        "ascii" => "PLY (text)",
        _ => "PLY (binary)",
    };
    let mut facts = vec![("Format", what.to_string())];
    if tris.is_empty() && !pts.is_empty() {
        facts.push(("Note", "A point cloud: points only, no faces".into()));
    }
    Ok((pts, tris, facts))
}

fn gltf_mesh(path: &Path) -> Result<Parsed, String> {
    let g = gltf::Gltf::open(path).map_err(|e| format!("This glTF file couldn't be read: {e}"))?;
    // Only buffers inside the file or in files beside it, never elsewhere on the computer.
    let base = path.parent().unwrap_or(Path::new("."));
    for b in g.document.buffers() {
        if let gltf::buffer::Source::Uri(uri) = b.source() {
            if uri.starts_with("data:") {
                continue;
            }
            let rel = Path::new(uri);
            let bad = uri.contains(':') || rel.is_absolute() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_)));
            let file = base.join(rel);
            if bad || !file.is_file() || std::fs::metadata(&file).map_or(true, |m| m.len() > MAX_FILE) {
                return Err("This glTF file's data is in files that aren't beside it.".into());
            }
        }
    }
    let buffers = gltf::import_buffers(&g.document, Some(base), g.blob.clone()).map_err(|e| format!("This glTF file's data couldn't be read: {e}"))?;
    let mut pts = Vec::new();
    let mut tris = Vec::new();
    // Every mesh where the scene puts it (its nodes' transforms), or as it is when there's no scene.
    fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
        let mut m = [[0.0; 4]; 4];
        for c in 0..4 {
            for r in 0..4 {
                m[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
            }
        }
        m
    }
    let mut placed: Vec<(gltf::Mesh, [[f32; 4]; 4])> = Vec::new();
    fn walk<'a>(n: gltf::Node<'a>, m: [[f32; 4]; 4], out: &mut Vec<(gltf::Mesh<'a>, [[f32; 4]; 4])>, depth: usize, mul: fn(&[[f32; 4]; 4], &[[f32; 4]; 4]) -> [[f32; 4]; 4]) {
        if depth > 64 {
            return;
        }
        let m = mul(&m, &n.transform().matrix());
        if let Some(mesh) = n.mesh() {
            out.push((mesh, m));
        }
        for c in n.children() {
            walk(c, m, out, depth + 1, mul);
        }
    }
    let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    if let Some(scene) = g.document.default_scene().or_else(|| g.document.scenes().next()) {
        for n in scene.nodes() {
            walk(n, id, &mut placed, 0, mul);
        }
    }
    if placed.is_empty() {
        placed = g.document.meshes().map(|m| (m, id)).collect();
    }
    for (mesh, m) in &placed {
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let r = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(pos) = r.read_positions() else { continue };
            let start = pts.len() as u32;
            for p in pos {
                let v = |r: usize| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r];
                pts.push([v(0), v(1), v(2)]);
            }
            let n = pts.len() as u32 - start;
            match r.read_indices() {
                Some(ix) => {
                    let ix: Vec<u32> = ix.into_u32().collect();
                    for t in ix.chunks_exact(3) {
                        if t.iter().all(|&i| i < n) {
                            tris.push([start + t[0], start + t[1], start + t[2]]);
                        }
                    }
                }
                None => tris.extend((0..n / 3).map(|t| [start + t * 3, start + t * 3 + 1, start + t * 3 + 2])),
            }
            if tris.len() > MAX_TRIS {
                return Err(too_many());
            }
        }
    }
    let d = &g.document;
    let mut facts = vec![("Format", if g.blob.is_some() { "glTF (binary, .glb)".to_string() } else { "glTF".to_string() })];
    let meshes = d.meshes().count();
    if meshes > 1 {
        facts.push(("Meshes", group(meshes)));
    }
    let (mats, texs, anims) = (d.materials().count(), d.textures().count(), d.animations().count());
    if mats > 0 {
        facts.push(("Materials", group(mats)));
    }
    if texs > 0 {
        facts.push(("Textures", format!("{} (not shown)", group(texs))));
    }
    if anims > 0 {
        facts.push(("Animations", group(anims)));
    }
    if let Some(maker) = d.as_json().asset.generator.as_deref().filter(|s| !s.is_empty()) {
        facts.push(("Made with", maker.chars().take(80).collect()));
    }
    Ok((pts, tris, facts))
}

/// 3MF: a zip with the model as XML (3D/3dmodel.model), as 3D printers' software saves it.
fn threemf(path: &Path) -> Result<Parsed, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|_| "This 3MF file couldn't be read.".to_string())?;
    let name = z.file_names().filter_map(|n| n.ok()).map(|n| n.to_string()).find(|n| n.to_ascii_lowercase().ends_with(".model")).ok_or("This 3MF file has no model in it.")?;
    let mut xml = String::new();
    z.by_name(&name).map_err(|e| e.to_string())?.take(512 << 20).read_to_string(&mut xml).map_err(|_| "This 3MF file couldn't be read.".to_string())?;
    let doc = roxmltree::Document::parse_with_options(&xml, roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() }).map_err(|_| "This 3MF file couldn't be read.".to_string())?;
    let mut pts = Vec::new();
    let mut tris = Vec::new();
    let mut objects = 0;
    for mesh in doc.descendants().filter(|n| n.has_tag_name("mesh")) {
        objects += 1;
        let start = pts.len() as u32;
        for v in mesh.descendants().filter(|n| n.has_tag_name("vertex")) {
            let a = |k: &str| v.attribute(k).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
            pts.push([a("x"), a("y"), a("z")]);
        }
        let n = pts.len() as u32 - start;
        for t in mesh.descendants().filter(|n| n.has_tag_name("triangle")) {
            let a = |k: &str| t.attribute(k).and_then(|s| s.parse::<u32>().ok());
            if let (Some(i), Some(j), Some(k)) = (a("v1"), a("v2"), a("v3"))
                && i < n
                && j < n
                && k < n
            {
                tris.push([start + i, start + j, start + k]);
            }
        }
        if tris.len() > MAX_TRIS {
            return Err(too_many());
        }
    }
    let mut facts = vec![("Format", "3MF (3D printing)".to_string())];
    if objects > 1 {
        facts.push(("Objects", group(objects)));
    }
    if let Some(unit) = doc.root_element().attribute("unit") {
        facts.push(("Units", unit.to_string()));
    }
    Ok((pts, tris, facts))
}

impl Mesh {
    /// The model drawn `w` × `h` from an angle: shaded clay on transparent, RGBA. `step` > 1
    /// draws only every `step`th triangle (quicker, while it's being turned).
    pub fn draw(&self, w: usize, h: usize, a: Angle, step: usize) -> Vec<u8> {
        let mut px = vec![0u8; w * h * 4];
        if w == 0 || h == 0 {
            return px;
        }
        let mut depth = vec![f32::INFINITY; w * h];
        let (sy, cy) = a.yaw.sin_cos();
        let (sp, cp) = a.pitch.sin_cos();
        // Model y is up (glTF, OBJ); STL and 3MF are usually z-up, which reads fine turned.
        let view: Vec<[f32; 3]> = self
            .points
            .iter()
            .map(|p| {
                let x = p[0] * cy + p[2] * sy;
                let z = -p[0] * sy + p[2] * cy;
                let y = p[1] * cp - z * sp;
                let z = p[1] * sp + z * cp;
                [x, y, z]
            })
            .collect();
        let scale = 0.48 * w.min(h) as f32;
        let (cx, cyc) = (w as f32 / 2.0, h as f32 / 2.0);
        let screen = |v: [f32; 3]| [cx + v[0] * scale, cyc - v[1] * scale, -v[2]];
        let light = {
            let l = [-0.4f32, 0.6, 0.7];
            let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
            [l[0] / n, l[1] / n, l[2] / n]
        };
        for t in self.tris.iter().step_by(step.max(1)) {
            let [a, b, c] = [view[t[0] as usize], view[t[1] as usize], view[t[2] as usize]];
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let mut n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len == 0.0 || !len.is_finite() {
                continue;
            }
            n = [n[0] / len, n[1] / len, n[2] / len];
            // Two-sided: models' windings are often inconsistent.
            if n[2] < 0.0 {
                n = [-n[0], -n[1], -n[2]];
            }
            let diffuse = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
            let shade = 0.28 + 0.62 * diffuse + 0.12 * n[2];
            let col = [(214.0 * shade).min(255.0) as u8, (204.0 * shade).min(255.0) as u8, (190.0 * shade).min(255.0) as u8];
            let [p0, p1, p2] = [screen(a), screen(b), screen(c)];
            let area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
            if area.abs() < 1e-9 {
                continue;
            }
            let minx = p0[0].min(p1[0]).min(p2[0]).floor().max(0.0) as usize;
            let maxx = (p0[0].max(p1[0]).max(p2[0]).ceil() as isize).min(w as isize - 1);
            let miny = p0[1].min(p1[1]).min(p2[1]).floor().max(0.0) as usize;
            let maxy = (p0[1].max(p1[1]).max(p2[1]).ceil() as isize).min(h as isize - 1);
            if maxx < 0 || maxy < 0 {
                continue;
            }
            for y in miny..=maxy as usize {
                let fy = y as f32 + 0.5;
                for x in minx..=maxx as usize {
                    let fx = x as f32 + 0.5;
                    let w0 = ((p1[0] - fx) * (p2[1] - fy) - (p2[0] - fx) * (p1[1] - fy)) / area;
                    let w1 = ((p2[0] - fx) * (p0[1] - fy) - (p0[0] - fx) * (p2[1] - fy)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * p0[2] + w1 * p1[2] + w2 * p2[2];
                    let i = y * w + x;
                    if z < depth[i] {
                        depth[i] = z;
                        px[i * 4..i * 4 + 4].copy_from_slice(&[col[0], col[1], col[2], 255]);
                    }
                }
            }
        }
        // Points only (a point cloud): dots.
        if self.tris.is_empty() {
            for v in &view {
                let s = screen(*v);
                if s[0] >= 0.0 && s[1] >= 0.0 && (s[0] as usize) < w && (s[1] as usize) < h {
                    let i = s[1] as usize * w + s[0] as usize;
                    px[i * 4..i * 4 + 4].copy_from_slice(&[214, 204, 190, 255]);
                }
            }
        }
        px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_obj() -> String {
        let mut s = String::from("o cube\nv -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n");
        s.push_str("f 1 2 3 4\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n");
        s
    }

    #[test]
    fn reads_every_format_and_draws() {
        let dir = std::env::temp_dir().join(format!("filescape-3d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // OBJ: six quads → twelve triangles.
        let p = dir.join("cube.obj");
        std::fs::write(&p, cube_obj()).unwrap();
        let m = read(&p).unwrap();
        assert_eq!((m.points.len(), m.tris.len()), (8, 12));
        assert_eq!(m.extent, [2.0, 2.0, 2.0]);
        let px = m.draw(64, 64, Angle::default(), 1);
        assert_eq!(&px[(32 * 64 + 32) * 4 + 3..][..1], &[255]); // the middle is cube
        assert_eq!(px[3], 0); // a corner is empty
        // A Windows object file with the same extension.
        std::fs::write(&p, b"L\x01\x03\0\0\0\0\0binary").unwrap();
        assert!(read(&p).err().unwrap().contains("object file"));
        // Binary STL: one triangle.
        let mut stl = vec![0u8; 80];
        stl.extend(1u32.to_le_bytes());
        stl.extend([0u8; 12]);
        for v in [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]] {
            for c in v {
                stl.extend(c.to_le_bytes());
            }
        }
        stl.extend([0u8; 2]);
        let p = dir.join("t.stl");
        std::fs::write(&p, &stl).unwrap();
        let m = read(&p).unwrap();
        assert_eq!(m.tris.len(), 1);
        assert_eq!(m.extent, [1.0, 2.0, 0.0]);
        // ASCII PLY with a quad.
        let p = dir.join("q.ply");
        std::fs::write(&p, "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n").unwrap();
        assert_eq!(read(&p).unwrap().tris.len(), 2);
        // Binary PLY, the same quad.
        let mut b = b"ply\nformat binary_little_endian 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n".to_vec();
        for v in [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in v {
                b.extend(c.to_le_bytes());
            }
        }
        b.push(4);
        for i in [0i32, 1, 2, 3] {
            b.extend(i.to_le_bytes());
        }
        std::fs::write(&p, &b).unwrap();
        let m = read(&p).unwrap();
        assert_eq!((m.points.len(), m.tris.len()), (4, 2));
        // 3MF.
        let p = dir.join("t.3mf");
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            z.start_file("3D/3dmodel.model", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(br#"<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="0" y="10" z="5"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="1" v3="9"/></triangles></mesh></object></resources></model>"#).unwrap();
            z.finish().unwrap();
        }
        let m = read(&p).unwrap();
        assert_eq!(m.tris.len(), 1); // the one pointing past the end is dropped
        assert!(m.facts.contains(&("Units", "millimeter".into())));
        // glTF with its data inline, and one pointing outside its folder.
        let p = dir.join("t.gltf");
        let mut bin = Vec::new();
        for v in [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in v {
                bin.extend(c.to_le_bytes());
            }
        }
        let data = {
            const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut s = String::new();
            for c in bin.chunks(3) {
                let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
                for k in 0..4 {
                    s.push(if k <= c.len() { A[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
                }
            }
            s
        };
        let doc = |uri: &str| format!(r#"{{"asset":{{"version":"2.0","generator":"test"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0,"translation":[5,0,0]}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}}],"bufferViews":[{{"buffer":0,"byteLength":36}}],"buffers":[{{"byteLength":36,"uri":"{uri}"}}]}}"#);
        std::fs::write(&p, doc(&format!("data:application/octet-stream;base64,{data}"))).unwrap();
        let m = read(&p).unwrap();
        assert_eq!(m.tris.len(), 1);
        assert!(m.facts.contains(&("Made with", "test".into())));
        std::fs::write(&p, doc("../../etc/passwd")).unwrap();
        assert!(read(&p).err().unwrap().contains("aren't beside it"));
        std::fs::write(&p, doc("file:///dev/zero")).unwrap();
        assert!(read(&p).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
