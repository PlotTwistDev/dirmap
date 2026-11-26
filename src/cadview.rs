//! CAD files: DXF drawings drawn (lines, polylines, circles, arcs, ellipses, splines and blocks),
//! and what DWG, STEP and IGES files say about themselves: the program and version that wrote
//! them, units, author and parts. Those three can't be drawn without large, mostly proprietary
//! libraries, so only their details are shown.

use crate::help::doc::{Block, Span};
use resvg::tiny_skia;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// Files larger than this aren't read.
const FILE_MAX: u64 = 64 << 20;
/// Segments drawn at most.
const SEGS_MAX: usize = 3_000_000;
/// How deep block references inside blocks are followed.
const NEST_MAX: usize = 6;

type P = [f64; 2];

/// A DXF drawing: polylines in drawing units, each with a colour.
pub struct Drawing {
    paths: Vec<(Vec<P>, [u8; 3])>,
    pub facts: Vec<(&'static str, String)>,
    lo: P,
    hi: P,
}

/// What a DWG, STEP or IGES file says about itself.
pub struct Info {
    pub blocks: Vec<Block>,
    pub facts: Vec<(&'static str, String)>,
}

pub fn is_dxf(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
}

fn slurp(path: &Path) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > FILE_MAX {
        return Err(format!("Too big to read ({})", crate::scan::human(len)));
    }
    let mut b = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(FILE_MAX).read_to_end(&mut b)).map_err(|e| e.to_string())?;
    Ok(b)
}

fn group(n: usize) -> String {
    crate::inspect::group(n as u64)
}

/// AutoCAD's version codes, as people know them.
fn acad_version(code: &str) -> String {
    let name = match code.trim() {
        "AC1009" => "AutoCAD R11/R12",
        "AC1012" => "AutoCAD R13",
        "AC1014" => "AutoCAD R14",
        "AC1015" => "AutoCAD 2000",
        "AC1018" => "AutoCAD 2004",
        "AC1021" => "AutoCAD 2007",
        "AC1024" => "AutoCAD 2010",
        "AC1027" => "AutoCAD 2013",
        "AC1032" => "AutoCAD 2018",
        "AC1035" => "AutoCAD 2025",
        other => return format!("Unknown ({other})"),
    };
    format!("{name} format ({})", code.trim())
}

/// `$INSUNITS`.
fn units(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "Inches",
        2 => "Feet",
        3 => "Miles",
        4 => "Millimetres",
        5 => "Centimetres",
        6 => "Metres",
        7 => "Kilometres",
        8 => "Microinches",
        9 => "Mils",
        10 => "Yards",
        13 => "Microns",
        14 => "Decimetres",
        _ => return None,
    })
}

/// AutoCAD colour numbers 1 to 9 (others are drawn light grey).
fn aci(n: i64) -> [u8; 3] {
    match n {
        1 => [240, 80, 80],
        2 => [240, 220, 80],
        3 => [90, 210, 110],
        4 => [80, 210, 230],
        5 => [100, 130, 255],
        6 => [230, 90, 230],
        8 => [140, 140, 140],
        9 => [190, 190, 190],
        _ => [225, 228, 232],
    }
}

// ---- DXF ----

/// One entity: its type and its group codes in order.
struct Ent {
    kind: String,
    codes: Vec<(i32, String)>,
}

impl Ent {
    fn f(&self, code: i32) -> Option<f64> {
        self.codes.iter().find(|c| c.0 == code).and_then(|c| c.1.trim().parse().ok())
    }
    fn s(&self, code: i32) -> Option<&str> {
        self.codes.iter().find(|c| c.0 == code).map(|c| c.1.trim())
    }
    fn i(&self, code: i32) -> Option<i64> {
        self.s(code).and_then(|s| s.parse().ok())
    }
}

/// The group-code/value pairs of an ASCII DXF file.
fn pairs(text: &str) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        let Ok(code) = code.trim().parse::<i32>() else { break };
        out.push((code, value.trim_end_matches('\r').to_string()));
    }
    out
}

pub fn read_dxf(path: &Path) -> Result<Drawing, String> {
    let raw = slurp(path)?;
    if raw.starts_with(b"AutoCAD Binary DXF") {
        return Err("A binary DXF file, which Filescape doesn't draw yet".into());
    }
    dxf(&String::from_utf8_lossy(&raw))
}

fn dxf(text: &str) -> Result<Drawing, String> {
    let pairs = pairs(text);
    if pairs.is_empty() {
        return Err("Not a DXF drawing".into());
    }
    // Split into sections, and each section into entities (a 0 code starts one).
    let mut header: HashMap<String, String> = HashMap::new();
    let mut entities: Vec<Ent> = Vec::new();
    let mut blocks: HashMap<String, Vec<Ent>> = HashMap::new();
    let mut layers = 0usize;
    let mut section = String::new();
    let mut i = 0;
    let mut cur_block: Option<String> = None;
    while i < pairs.len() {
        let (code, ref v) = pairs[i];
        if code == 0 && v == "SECTION" {
            section = pairs.get(i + 1).map(|p| p.1.trim().to_string()).unwrap_or_default();
            i += 2;
            continue;
        }
        if code == 0 && v == "ENDSEC" {
            section.clear();
            i += 1;
            continue;
        }
        match section.as_str() {
            "HEADER" => {
                if code == 9
                    && let Some(val) = pairs.get(i + 1)
                {
                    header.insert(v.trim().to_string(), val.1.trim().to_string());
                }
                i += 1;
            }
            "TABLES" => {
                if code == 0 && v == "LAYER" {
                    layers += 1;
                }
                i += 1;
            }
            "ENTITIES" | "BLOCKS" if code == 0 => {
                let mut e = Ent { kind: v.trim().to_string(), codes: Vec::new() };
                i += 1;
                while i < pairs.len() && pairs[i].0 != 0 {
                    e.codes.push(pairs[i].clone());
                    i += 1;
                }
                if section == "BLOCKS" {
                    match e.kind.as_str() {
                        "BLOCK" => {
                            let name = e.s(2).unwrap_or("").to_string();
                            blocks.insert(name.clone(), Vec::new());
                            cur_block = Some(name);
                        }
                        "ENDBLK" => cur_block = None,
                        _ => {
                            if let Some(b) = cur_block.as_ref().and_then(|n| blocks.get_mut(n)) {
                                b.push(e);
                            }
                        }
                    }
                } else {
                    entities.push(e);
                }
            }
            _ => i += 1,
        }
    }
    // A block's own base point: its geometry is placed relative to it.
    let mut d = Drawing { paths: Vec::new(), facts: Vec::new(), lo: [f64::MAX; 2], hi: [f64::MIN; 2] };
    let mut segs = 0usize;
    let mut texts = 0usize;
    let mut kinds: HashMap<String, usize> = HashMap::new();
    for e in &entities {
        *kinds.entry(e.kind.clone()).or_default() += 1;
    }
    let ident = Xf { scale: [1.0, 1.0], rot: 0.0, off: [0.0, 0.0] };
    draw_ents(&entities, &blocks, ident, 0, 7, &mut d, &mut segs, &mut texts);
    if let Some(v) = header.get("$ACADVER") {
        d.facts.push(("Format", acad_version(v)));
    }
    let unit = header.get("$INSUNITS").and_then(|u| u.parse().ok()).and_then(units);
    if let Some(u) = unit {
        d.facts.push(("Units", u.into()));
    }
    d.facts.push(("Objects", group(entities.len())));
    let mut top: Vec<(String, usize)> = kinds.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if !top.is_empty() {
        d.facts.push(("Most common", top.iter().take(4).map(|(k, n)| format!("{} {}", group(*n), k.to_ascii_lowercase())).collect::<Vec<_>>().join(", ")));
    }
    if layers > 0 {
        d.facts.push(("Layers", group(layers)));
    }
    let named = blocks.keys().filter(|k| !k.starts_with('*')).count();
    if named > 0 {
        d.facts.push(("Blocks", group(named)));
    }
    if texts > 0 {
        d.facts.push(("Text labels", format!("{} (not drawn)", group(texts))));
    }
    if d.paths.is_empty() {
        return Err("Nothing in it that Filescape can draw".into());
    }
    let (w, h) = (d.hi[0] - d.lo[0], d.hi[1] - d.lo[1]);
    let u = match unit {
        Some("Millimetres") => " mm",
        Some("Centimetres") => " cm",
        Some("Metres") => " m",
        Some("Inches") => " in",
        Some("Feet") => " ft",
        _ => "",
    };
    d.facts.push(("Size", format!("{} × {}{u}", short(w), short(h))));
    if segs >= SEGS_MAX {
        d.facts.push(("Drawn", "Only part: it's very large".into()));
    }
    Ok(d)
}

fn short(v: f64) -> String {
    let s = if v.abs() >= 100.0 { format!("{v:.0}") } else if v.abs() >= 1.0 { format!("{v:.2}") } else { format!("{v:.4}") };
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Placing a block: scale, then rotate, then move.
#[derive(Clone, Copy)]
struct Xf {
    scale: P,
    rot: f64,
    off: P,
}

impl Xf {
    fn apply(&self, p: P) -> P {
        let (x, y) = (p[0] * self.scale[0], p[1] * self.scale[1]);
        let (s, c) = self.rot.sin_cos();
        [x * c - y * s + self.off[0], x * s + y * c + self.off[1]]
    }
    /// This placement inside another.
    fn then(&self, outer: &Xf) -> Xf {
        let off = outer.apply(self.off);
        Xf { scale: [self.scale[0] * outer.scale[0], self.scale[1] * outer.scale[1]], rot: self.rot + outer.rot, off }
    }
}

/// Points around an arc, `a0` to `a1` in radians, counter-clockwise.
fn arc(c: P, r: f64, a0: f64, mut a1: f64) -> Vec<P> {
    while a1 <= a0 {
        a1 += std::f64::consts::TAU;
    }
    let n = (((a1 - a0) / std::f64::consts::TAU) * 96.0).ceil().clamp(4.0, 96.0) as usize;
    (0..=n).map(|k| a0 + (a1 - a0) * k as f64 / n as f64).map(|a| [c[0] + r * a.cos(), c[1] + r * a.sin()]).collect()
}

/// A polyline vertex list with bulges (arcs between vertices) turned into points.
fn bulged(pts: &[(P, f64)], closed: bool) -> Vec<P> {
    let mut out = Vec::new();
    let n = pts.len();
    for k in 0..n {
        let (a, bulge) = pts[k];
        out.push(a);
        let next = if k + 1 < n { Some(pts[k + 1].0) } else if closed { Some(pts[0].0) } else { None };
        let Some(b) = next else { continue };
        if bulge.abs() > 1e-9 {
            // The arc's sweep is 4·atan(bulge).
            let sweep = 4.0 * bulge.atan();
            let chord = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            if chord < 1e-12 {
                continue;
            }
            let r = chord / (2.0 * (sweep / 2.0).sin()).abs();
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let d = (r * r - (chord / 2.0).powi(2)).max(0.0).sqrt();
            let (ux, uy) = ((b[0] - a[0]) / chord, (b[1] - a[1]) / chord);
            // The centre is to the left for a positive bulge under 180°.
            let side = if (bulge > 0.0) == (sweep.abs() < std::f64::consts::PI) { 1.0 } else { -1.0 };
            let c = [mid[0] - uy * d * side, mid[1] + ux * d * side];
            let a0 = (a[1] - c[1]).atan2(a[0] - c[0]);
            let steps = ((sweep.abs() / std::f64::consts::TAU) * 64.0).ceil().clamp(2.0, 64.0) as usize;
            for s in 1..steps {
                let t = a0 + sweep * s as f64 / steps as f64;
                out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
            }
        }
    }
    if closed && let Some(f) = out.first().copied() {
        out.push(f);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_ents(ents: &[Ent], blocks: &HashMap<String, Vec<Ent>>, xf: Xf, depth: usize, layer_colour: i64, d: &mut Drawing, segs: &mut usize, texts: &mut usize) {
    let mut i = 0;
    while i < ents.len() {
        if *segs >= SEGS_MAX {
            return;
        }
        let e = &ents[i];
        i += 1;
        // Colour 0 is "by block", 256 "by layer": the block's colour, or plain.
        let colour = match e.i(62) {
            Some(0) => layer_colour,
            Some(c) if c > 0 && c < 256 => c,
            _ => 7,
        };
        let mut push = |pts: Vec<P>, d: &mut Drawing| {
            if pts.len() < 2 {
                return;
            }
            *segs += pts.len() - 1;
            let pts: Vec<P> = pts.into_iter().map(|p| xf.apply(p)).filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
            for p in &pts {
                d.lo = [d.lo[0].min(p[0]), d.lo[1].min(p[1])];
                d.hi = [d.hi[0].max(p[0]), d.hi[1].max(p[1])];
            }
            d.paths.push((pts, aci(colour)));
        };
        let at = |x: i32, y: i32| -> Option<P> { Some([e.f(x)?, e.f(y)?]) };
        match e.kind.as_str() {
            "LINE" => {
                if let (Some(a), Some(b)) = (at(10, 20), at(11, 21)) {
                    push(vec![a, b], d);
                }
            }
            "LWPOLYLINE" => {
                // Vertices: each 10 starts one; a 42 after it is its bulge.
                let mut pts: Vec<(P, f64)> = Vec::new();
                let mut x = None;
                for (c, v) in &e.codes {
                    let v: f64 = v.trim().parse().unwrap_or(0.0);
                    match c {
                        10 => x = Some(v),
                        20 => {
                            if let Some(x) = x.take() {
                                pts.push(([x, v], 0.0));
                            }
                        }
                        42 => {
                            if let Some(last) = pts.last_mut() {
                                last.1 = v;
                            }
                        }
                        _ => {}
                    }
                }
                let closed = e.i(70).unwrap_or(0) & 1 == 1;
                push(bulged(&pts, closed), d);
            }
            "POLYLINE" => {
                // VERTEX entities follow, up to SEQEND.
                let closed = e.i(70).unwrap_or(0) & 1 == 1;
                let mut pts = Vec::new();
                while i < ents.len() && ents[i].kind == "VERTEX" {
                    if let Some(p) = [ents[i].f(10), ents[i].f(20)].into_iter().collect::<Option<Vec<_>>>() {
                        pts.push(([p[0], p[1]], ents[i].f(42).unwrap_or(0.0)));
                    }
                    i += 1;
                }
                if i < ents.len() && ents[i].kind == "SEQEND" {
                    i += 1;
                }
                push(bulged(&pts, closed), d);
            }
            "CIRCLE" => {
                if let (Some(c), Some(r)) = (at(10, 20), e.f(40)) {
                    push(arc(c, r, 0.0, std::f64::consts::TAU), d);
                }
            }
            "ARC" => {
                if let (Some(c), Some(r), Some(a0), Some(a1)) = (at(10, 20), e.f(40), e.f(50), e.f(51)) {
                    push(arc(c, r, a0.to_radians(), a1.to_radians()), d);
                }
            }
            "ELLIPSE" => {
                if let (Some(c), Some(m)) = (at(10, 20), at(11, 21)) {
                    let ratio = e.f(40).unwrap_or(1.0);
                    let (t0, mut t1) = (e.f(41).unwrap_or(0.0), e.f(42).unwrap_or(std::f64::consts::TAU));
                    while t1 <= t0 {
                        t1 += std::f64::consts::TAU;
                    }
                    let minor = [-m[1] * ratio, m[0] * ratio];
                    let pts = (0..=72).map(|k| t0 + (t1 - t0) * k as f64 / 72.0).map(|t| [c[0] + m[0] * t.cos() + minor[0] * t.sin(), c[1] + m[1] * t.cos() + minor[1] * t.sin()]).collect();
                    push(pts, d);
                }
            }
            "SPLINE" => {
                // Fit points (11/21) when there are any, else the control points: close enough
                // for a preview.
                let take = |xc: i32, yc: i32| -> Vec<P> {
                    let xs: Vec<f64> = e.codes.iter().filter(|c| c.0 == xc).filter_map(|c| c.1.trim().parse().ok()).collect();
                    let ys: Vec<f64> = e.codes.iter().filter(|c| c.0 == yc).filter_map(|c| c.1.trim().parse().ok()).collect();
                    xs.into_iter().zip(ys).map(|(x, y)| [x, y]).collect()
                };
                let fit = take(11, 21);
                push(if fit.len() >= 2 { fit } else { take(10, 20) }, d);
            }
            "SOLID" | "TRACE" | "3DFACE" => {
                let c: Vec<P> = [(10, 20), (11, 21), (13, 23), (12, 22)].iter().filter_map(|&(x, y)| at(x, y)).collect();
                if c.len() >= 3 {
                    let mut c = c;
                    c.push(c[0]);
                    push(c, d);
                }
            }
            "POINT" => {
                if let Some(p) = at(10, 20) {
                    push(vec![p, [p[0] + 1e-9, p[1]]], d);
                }
            }
            "TEXT" | "MTEXT" | "ATTRIB" | "ATTDEF" => *texts += 1,
            "INSERT" if depth < NEST_MAX => {
                let Some(b) = e.s(2).and_then(|n| blocks.get(n)) else { continue };
                let inner = Xf {
                    scale: [e.f(41).unwrap_or(1.0), e.f(42).unwrap_or(1.0)],
                    rot: e.f(50).unwrap_or(0.0).to_radians(),
                    off: at(10, 20).unwrap_or([0.0, 0.0]),
                };
                draw_ents(b, blocks, inner.then(&xf), depth + 1, colour, d, segs, texts);
            }
            _ => {}
        }
    }
}

impl Drawing {
    /// A size for the picture, no wider or taller than `max`.
    pub fn fit(&self, max: u32) -> (u32, u32) {
        let (w, h) = (self.hi[0] - self.lo[0], self.hi[1] - self.lo[1]);
        let a = if w > 0.0 && h > 0.0 { (w / h).clamp(0.4, 3.0) } else { 1.4 };
        if a >= 1.0 { (max, ((max as f64 / a) as u32).max(16)) } else { (((max as f64 * a) as u32).max(16), max) }
    }

    /// The drawing, light lines on dark, `w` by `h`, as RGBA.
    pub fn draw(&self, w: u32, h: u32) -> Vec<u8> {
        let Some(mut pix) = tiny_skia::Pixmap::new(w.max(1), h.max(1)) else { return vec![0; (w * h * 4) as usize] };
        pix.fill(tiny_skia::Color::from_rgba8(22, 26, 34, 255));
        let margin = (w.min(h) as f64 * 0.05).max(3.0);
        let (sw, sh) = ((self.hi[0] - self.lo[0]).max(1e-9), (self.hi[1] - self.lo[1]).max(1e-9));
        let scale = ((w as f64 - 2.0 * margin) / sw).min((h as f64 - 2.0 * margin) / sh);
        let (ox, oy) = ((w as f64 - sw * scale) / 2.0, (h as f64 - sh * scale) / 2.0);
        let to = |p: P| ((ox + (p[0] - self.lo[0]) * scale) as f32, (h as f64 - oy - (p[1] - self.lo[1]) * scale) as f32);
        let stroke = tiny_skia::Stroke { width: (w.min(h) as f32 / 500.0).clamp(1.0, 3.0), line_cap: tiny_skia::LineCap::Round, ..Default::default() };
        // One path per colour, so thousands of lines draw quickly.
        let mut by_colour: HashMap<[u8; 3], tiny_skia::PathBuilder> = HashMap::new();
        for (pts, c) in &self.paths {
            let pb = by_colour.entry(*c).or_default();
            let mut last = (f32::NAN, f32::NAN);
            for (k, p) in pts.iter().enumerate() {
                let (x, y) = to(*p);
                if k == 0 {
                    pb.move_to(x, y);
                } else if (x - last.0).abs() >= 0.25 || (y - last.1).abs() >= 0.25 || k + 1 == pts.len() {
                    pb.line_to(x, y);
                } else {
                    continue;
                }
                last = (x, y);
            }
        }
        for (c, pb) in by_colour {
            if let Some(path) = pb.finish() {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(c[0], c[1], c[2], 235);
                paint.anti_alias = true;
                pix.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
            }
        }
        pix.take()
    }
}

// ---- DWG, STEP and IGES ----

pub fn read_info(path: &Path) -> Result<Info, String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "dwg" => {
            let mut head = [0u8; 6];
            std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).map_err(|e| e.to_string())?;
            if !head.starts_with(b"AC") {
                return Err("Not a DWG drawing".into());
            }
            let v = String::from_utf8_lossy(&head).to_string();
            Ok(info("AutoCAD drawing (DWG)", vec![("Format", acad_version(&v))], "DWG is AutoCAD's own format. Filescape can't draw it; save a DXF copy from your CAD program to see the drawing here."))
        }
        "step" | "stp" | "p21" => step(&String::from_utf8_lossy(&slurp(path)?)),
        "iges" | "igs" => iges(&String::from_utf8_lossy(&slurp(path)?)),
        _ => Err("Not a CAD file Filescape knows".into()),
    }
}

fn info(title: &str, facts: Vec<(&'static str, String)>, note: &str) -> Info {
    let plain = |t: &str| vec![Span { text: t.to_string(), ..Default::default() }];
    let mut blocks = vec![Block::Heading { level: 1, spans: plain(title), id: String::new() }];
    for (k, v) in &facts {
        blocks.push(Block::Para(vec![Span { text: format!("{k}: "), bold: true, ..Default::default() }, Span { text: v.clone(), ..Default::default() }]));
    }
    blocks.push(Block::Rule);
    blocks.push(Block::Para(plain(note)));
    Info { blocks, facts }
}

/// The quoted strings in a STEP header record: `FILE_NAME('a','b',('c'),...)`.
fn step_strings(rec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut inside = false;
    let mut chars = rec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if inside && chars.peek() == Some(&'\'') {
                cur.push('\'');
                chars.next();
            } else if inside {
                out.push(std::mem::take(&mut cur));
                inside = false;
            } else {
                inside = true;
            }
        } else if inside {
            cur.push(c);
        }
    }
    out
}

fn step(text: &str) -> Result<Info, String> {
    if !text.trim_start().starts_with("ISO-10303-21") {
        return Err("Not a STEP file".into());
    }
    let header = text.split("ENDSEC;").next().unwrap_or("");
    let record = |name: &str| header.find(name).map(|i| &header[i..]).map(|r| &r[..r.find(";").unwrap_or(r.len())]);
    let mut facts = Vec::new();
    if let Some(r) = record("FILE_NAME") {
        let s = step_strings(r);
        // name, time, authors…, organisations…, preprocessor, originating system, authorisation
        let get = |i: usize| s.get(i).filter(|v| !v.trim().is_empty()).cloned();
        if let Some(v) = get(0) {
            facts.push(("Name", v));
        }
        if let Some(v) = get(1) {
            facts.push(("Saved", v.replace('T', " ")));
        }
        let n = s.len();
        if n >= 3 {
            if let Some(v) = s.get(n.saturating_sub(2)).filter(|v| !v.trim().is_empty()) {
                facts.push(("Made with", v.clone()));
            }
            if let Some(v) = s.get(n.saturating_sub(3)).filter(|v| !v.trim().is_empty()) {
                facts.push(("Translator", v.clone()));
            }
        }
    }
    if let Some(r) = record("FILE_SCHEMA") {
        let s = step_strings(r).join(", ");
        let ap = if s.contains("AP242") || s.contains("AP242_") {
            "AP242 (managed model-based 3D engineering)"
        } else if s.contains("AUTOMOTIVE_DESIGN") || s.contains("AP214") {
            "AP214 (automotive design)"
        } else if s.contains("CONFIG_CONTROL_DESIGN") || s.contains("AP203") {
            "AP203 (configuration-controlled design)"
        } else {
            ""
        };
        facts.push(("Schema", if ap.is_empty() { s } else { ap.into() }));
    }
    let data = text.split_once("DATA;").map(|x| x.1).unwrap_or("");
    let ents = data.matches("\n#").count() + data.starts_with('#') as usize;
    facts.push(("Entities", group(ents)));
    let products: Vec<String> = data
        .match_indices("PRODUCT(")
        .filter(|(i, _)| !data[..*i].ends_with('_'))
        .filter_map(|(i, _)| step_strings(&data[i..i + data[i..].find(';').unwrap_or(0)]).into_iter().next())
        .filter(|n| !n.trim().is_empty())
        .take(50)
        .collect();
    if !products.is_empty() {
        facts.push(("Parts", format!("{}: {}", group(products.len()), products.iter().take(8).cloned().collect::<Vec<_>>().join(", "))));
    }
    if data.contains("MANIFOLD_SOLID_BREP") || data.contains("ADVANCED_BREP") {
        facts.push(("Contains", "Solid models".into()));
    }
    Ok(info("STEP 3D model", facts, "STEP models aren't drawn yet: they need a full geometry kernel. These are the details saved in the file."))
}

/// An IGES Global section parameter: a Hollerith string (`5HHello`) or a plain value.
fn iges_params(g: &str) -> Vec<String> {
    // The first two may change the delimiters; almost every file keeps , and ;.
    let mut out = Vec::new();
    let b = g.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i > start && i < b.len() && b[i] == b'H' {
            let n: usize = g[start..i].parse().unwrap_or(0);
            let s = (i + 1).min(b.len());
            let e = (s + n).min(b.len());
            out.push(g[s..e].to_string());
            i = e;
            while i < b.len() && b[i] != b',' && b[i] != b';' {
                i += 1;
            }
        } else {
            i = start;
            while i < b.len() && b[i] != b',' && b[i] != b';' {
                i += 1;
            }
            out.push(g[start..i].trim().to_string());
        }
        if i < b.len() && b[i] == b';' {
            break;
        }
        i += 1;
    }
    out
}

fn iges(text: &str) -> Result<Info, String> {
    // 80-column records; column 73 says which section a line is in.
    let section = |l: &str| l.as_bytes().get(72).copied();
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    if !lines.iter().any(|l| section(l) == Some(b'G')) {
        return Err("Not an IGES file".into());
    }
    let global: String = lines.iter().filter(|l| section(l) == Some(b'G')).map(|l| &l[..72.min(l.len())]).collect();
    let start: String = lines.iter().filter(|l| section(l) == Some(b'S')).map(|l| l[..72.min(l.len())].trim()).collect::<Vec<_>>().join(" ");
    let p = iges_params(&global);
    let get = |i: usize| p.get(i).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut facts = Vec::new();
    // 3 product id, 4 file name, 5 system id, 6 preprocessor version, 14 unit name,
    // 18 date, 21 author, 22 organisation.
    if let Some(v) = get(2) {
        facts.push(("Product", v));
    }
    if let Some(v) = get(3) {
        facts.push(("Name", v));
    }
    if let Some(v) = get(4) {
        facts.push(("Made with", v));
    }
    if let Some(v) = get(14) {
        facts.push(("Units", v));
    }
    if let Some(v) = get(17) {
        facts.push(("Saved", v));
    }
    if let Some(v) = get(20) {
        facts.push(("Author", v));
    }
    if let Some(v) = get(21) {
        facts.push(("Organisation", v));
    }
    let d = lines.iter().filter(|l| section(l) == Some(b'D')).count();
    facts.push(("Entities", group(d / 2)));
    if !start.is_empty() {
        facts.push(("Note", start.chars().take(200).collect()));
    }
    Ok(info("IGES model", facts, "IGES models aren't drawn yet. These are the details saved in the file."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact<'a>(f: &'a [(&'static str, String)], k: &str) -> Option<&'a str> {
        f.iter().find(|x| x.0 == k).map(|x| x.1.as_str())
    }

    fn dxf_text(body: &[(&str, &str)]) -> String {
        body.iter().map(|(c, v)| format!("{c}\n{v}\n")).collect()
    }

    #[test]
    fn dxf_entities_blocks_and_header() {
        let t = dxf_text(&[
            ("0", "SECTION"), ("2", "HEADER"), ("9", "$ACADVER"), ("1", "AC1015"), ("9", "$INSUNITS"), ("70", "4"), ("0", "ENDSEC"),
            ("0", "SECTION"), ("2", "TABLES"), ("0", "TABLE"), ("2", "LAYER"), ("0", "LAYER"), ("2", "0"), ("0", "LAYER"), ("2", "Walls"), ("0", "ENDTAB"), ("0", "ENDSEC"),
            ("0", "SECTION"), ("2", "BLOCKS"), ("0", "BLOCK"), ("2", "Door"), ("0", "LINE"), ("10", "0"), ("20", "0"), ("11", "1"), ("21", "0"), ("0", "ENDBLK"), ("0", "ENDSEC"),
            ("0", "SECTION"), ("2", "ENTITIES"),
            ("0", "LINE"), ("8", "Walls"), ("62", "1"), ("10", "0"), ("20", "0"), ("11", "100"), ("21", "0"),
            ("0", "LWPOLYLINE"), ("90", "3"), ("70", "1"), ("10", "0"), ("20", "0"), ("10", "100"), ("20", "0"), ("42", "1"), ("10", "100"), ("20", "50"),
            ("0", "CIRCLE"), ("10", "50"), ("20", "25"), ("40", "10"),
            ("0", "ARC"), ("10", "0"), ("20", "0"), ("40", "5"), ("50", "0"), ("51", "90"),
            ("0", "INSERT"), ("2", "Door"), ("10", "200"), ("20", "10"), ("41", "2"), ("50", "90"),
            ("0", "TEXT"), ("1", "Kitchen"),
            ("0", "ENDSEC"), ("0", "EOF"),
        ]);
        let d = dxf(&t).unwrap();
        assert_eq!(fact(&d.facts, "Format"), Some("AutoCAD 2000 format (AC1015)"));
        assert_eq!(fact(&d.facts, "Units"), Some("Millimetres"));
        assert_eq!(fact(&d.facts, "Objects"), Some("6"));
        assert_eq!(fact(&d.facts, "Layers"), Some("2"));
        assert_eq!(fact(&d.facts, "Blocks"), Some("1"));
        assert_eq!(fact(&d.facts, "Text labels"), Some("1 (not drawn)"));
        // The door, rotated 90° and doubled, runs from (200, 10) up to (200, 12).
        assert!((d.hi[0] - 200.0).abs() < 1e-6, "{:?}", d.hi);
        assert!(d.paths.iter().any(|(p, _)| p.iter().any(|q| (q[0] - 200.0).abs() < 1e-6 && (q[1] - 12.0).abs() < 1e-6)));
        let (w, h) = d.fit(300);
        assert_eq!(d.draw(w, h).len(), (w * h * 4) as usize);
    }

    #[test]
    fn bulge_makes_a_half_circle() {
        let pts = bulged(&[([0.0, 0.0], 1.0), ([2.0, 0.0], 0.0)], false);
        // A bulge of 1 is a half circle of radius 1 centred on (1, 0): it passes (1, -1) or (1, 1).
        assert!(pts.iter().any(|p| (p[0] - 1.0).abs() < 0.05 && (p[1].abs() - 1.0).abs() < 0.05), "{pts:?}");
        assert!(pts.iter().all(|p| ((p[0] - 1.0).powi(2) + p[1].powi(2)).sqrt() < 1.0001));
    }

    #[test]
    fn step_and_iges_details() {
        let s = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('A bracket'),'2;1');\nFILE_NAME('bracket.step','2026-11-20T10:11:12',('Ann'),('Acme'),'ST-DEVELOPER v18','SolidWorks 2024','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));\nENDSEC;\nDATA;\n#1=PRODUCT('Bracket','Bracket','',(#2));\n#2=PRODUCT_CONTEXT('',#3,'mechanical');\n#3=MANIFOLD_SOLID_BREP('',#4);\nENDSEC;\nEND-ISO-10303-21;\n";
        let i = step(s).unwrap();
        assert_eq!(fact(&i.facts, "Name"), Some("bracket.step"));
        assert_eq!(fact(&i.facts, "Made with"), Some("SolidWorks 2024"));
        assert_eq!(fact(&i.facts, "Schema"), Some("AP214 (automotive design)"));
        assert_eq!(fact(&i.facts, "Parts"), Some("1: Bracket"));
        assert_eq!(fact(&i.facts, "Entities"), Some("3"));
        assert_eq!(fact(&i.facts, "Contains"), Some("Solid models"));
        let pad = |s: &str, sec: char, n: usize| format!("{s:<72}{sec}{n:>7}");
        let g = "1H,,1H;,7HBracket,11Hbracket.igs,8HCATIA V5,3H1.0,32,38,6,308,15,7HBracket,1.,2,2HMM,1,0.1,13H261120.101112,0.001,100.,3HAnn,4HAcme,11,0,;";
        let text = [pad("Exported part", 'S', 1), pad(&g[..72], 'G', 1), pad(&g[72..], 'G', 2), pad("     110       1", 'D', 1), pad("     110       0", 'D', 2)].join("\n");
        let i = iges(&text).unwrap();
        assert_eq!(fact(&i.facts, "Product"), Some("Bracket"));
        assert_eq!(fact(&i.facts, "Made with"), Some("CATIA V5"));
        assert_eq!(fact(&i.facts, "Units"), Some("MM"));
        assert_eq!(fact(&i.facts, "Author"), Some("Ann"));
        assert_eq!(fact(&i.facts, "Entities"), Some("1"));
        assert_eq!(fact(&i.facts, "Note"), Some("Exported part"));
    }

    #[test]
    fn broken_input_does_not_panic() {
        for t in ["", "0\nSECTION\n", "0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n42\n5\n0\nINSERT\n2\nX\n0\nPOLYLINE\n0\nVERTEX\n", "x\ny"] {
            let _ = dxf(t);
        }
        assert!(step("nope").is_err());
        assert!(iges("short").is_err());
        let _ = iges_params("12Hshort");
        let _ = step_strings("('unterminated");
    }
}
