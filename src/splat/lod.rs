//! Big scenes kept as levels of detail in many chunks: PlayCanvas's streamed SOG
//! (`lod-meta.json` naming a folder per chunk), XGRIDS's LCC (`meta.lcc` with `index.bin`,
//! `data.bin` and `shcoef.bin` beside it) and LCC2 (a `.lcc2` file naming SOG or SPZ chunks).
//! Layouts follow PlayCanvas splat-transform's readers (MIT).
//!
//! One level is shown: the most detailed one that fits the limits and is on this computer. A
//! chunk whose data is only in the cloud is never opened, because opening it would download
//! it; such chunks are counted and named in the details instead.

use super::{Frame, Limits, Scene, SceneKind};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The JSON describing the levels is read at most this big.
const META_MAX: u64 = 64 << 20;
/// Chunks named at most.
const CHUNKS_MAX: usize = 200_000;

/// Where a file's data is, judged from its metadata alone (never opened).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    Here,
    Cloud,
    Missing,
}

#[cfg(test)]
thread_local! {
    /// Files the tests pretend are only in the cloud.
    pub(super) static CLOUD_FOR_TESTS: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn where_is(path: &Path) -> Where {
    #[cfg(test)]
    if CLOUD_FOR_TESTS.with(|c| c.borrow().iter().any(|p| p == path)) {
        return Where::Cloud;
    }
    match std::fs::metadata(path) {
        Ok(md) => {
            let (_, flags) = crate::scan::on_disk(&md, || path.to_path_buf(), crate::scan::cluster_of(path));
            if flags & crate::scan::store::NOT_LOCAL == 0 { Where::Here } else { Where::Cloud }
        }
        Err(_) => Where::Missing,
    }
}

/// A loose chunk's folder: here only when every file in it is.
fn folder_where(dir: &Path) -> Where {
    let Ok(rd) = std::fs::read_dir(dir) else { return Where::Missing };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_file() && where_is(&p) == Where::Cloud {
            return Where::Cloud;
        }
    }
    Where::Here
}

/// Whether this file name is one of these formats' entry files.
pub fn is_entry(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "lod-meta.json" || n.ends_with(".lcc") || n.ends_with(".lcc2")
}

/// A chunk's path inside the scene's folder, refusing names that climb out of it.
fn inside(dir: &Path, name: &str) -> Option<PathBuf> {
    let name = name.trim_start_matches("./");
    let p = Path::new(name);
    if name.is_empty() || p.is_absolute() || name.contains('\\') || p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    Some(dir.join(p))
}

fn read_meta(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    let mut out = Vec::new();
    f.take(META_MAX + 1).read_to_end(&mut out).map_err(|e| format!("Couldn't read it: {e}"))?;
    if out.len() as u64 > META_MAX {
        return Err("This scene's description is too big to be believable.".into());
    }
    Ok(out)
}

struct Chunk {
    path: PathBuf,
    /// Splats in it, when known without opening it.
    count: Option<u64>,
    at: Where,
}

/// One level of detail.
#[derive(Default)]
struct Level {
    chunks: Vec<Chunk>,
}

impl Level {
    fn here(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.iter().filter(|c| c.at == Where::Here)
    }
    fn here_splats(&self) -> u64 {
        self.here().map(|c| c.count.unwrap_or(0)).sum()
    }
    fn all_here(&self) -> bool {
        !self.chunks.is_empty() && self.chunks.iter().all(|c| c.at == Where::Here)
    }
    fn splats(&self) -> u64 {
        self.chunks.iter().map(|c| c.count.unwrap_or(0)).sum()
    }
}

/// The level to show: the most detailed one that's all here and fits the limits; failing
/// that the smallest one that's all here (shown from a sample); failing that the level with
/// the most splats here.
fn choose(levels: &[Level], limits: Limits) -> Option<usize> {
    let max = limits.max_points as u64;
    let whole: Vec<usize> = (0..levels.len()).filter(|&i| levels[i].all_here() && levels[i].here_splats() > 0).collect();
    if let Some(&i) = whole.iter().filter(|&&i| levels[i].here_splats() <= max).max_by_key(|&&i| levels[i].here_splats()) {
        return Some(i);
    }
    if let Some(&i) = whole.iter().min_by_key(|&&i| levels[i].here_splats()) {
        return Some(i);
    }
    let some: Vec<usize> = (0..levels.len()).filter(|&i| levels[i].here_splats() > 0).collect();
    some.iter().copied().filter(|&i| levels[i].here_splats() <= max).max_by_key(|&i| levels[i].here_splats()).or_else(|| some.iter().copied().min_by_key(|&i| levels[i].here_splats()))
}

/// The limits for one chunk of `count` splats out of `of` being read, so that the chunks
/// together keep to `limits`.
fn share(limits: Limits, count: u64, of: u64) -> Limits {
    let f = (limits.max_points as f64 / of.max(1) as f64).min(1.0);
    Limits {
        max_points: ((count as f64 * f).ceil() as usize).max(1),
        keep_sh: limits.keep_sh,
        sh_bytes: if limits.sh_bytes == u64::MAX { u64::MAX } else { (limits.sh_bytes as f64 * count as f64 / of.max(1) as f64) as u64 },
    }
}

/// Adds a chunk's splats to the scene being built. Chunks with different colour detail lose
/// their view-dependent colour, all of them.
fn merge(into: &mut Option<Scene>, mut s: Scene) {
    let Some(base) = into else {
        s.said.clear();
        *into = Some(s);
        return;
    };
    if base.frame != s.frame {
        return;
    }
    if base.sh_degree != s.sh_degree || base.sh.is_empty() != s.sh.is_empty() {
        let mixed = "Its chunks keep different colour detail, so base colour is shown";
        base.sh.clear();
        base.sh_degree = base.sh_degree.min(s.sh_degree);
        s.sh.clear();
        if !base.notes.iter().any(|n| n == mixed) {
            base.notes.push(mixed.into());
        }
    }
    base.total += s.total;
    base.bad += s.bad;
    base.antialiased |= s.antialiased;
    base.pos.append(&mut s.pos);
    base.color.append(&mut s.color);
    base.scale.append(&mut s.scale);
    base.rot.append(&mut s.rot);
    base.sh.append(&mut s.sh);
    for n in s.notes {
        if !base.notes.contains(&n) {
            base.notes.push(n);
        }
    }
}

/// Reads the chosen level's chunks that are here, and says what was shown and what wasn't.
fn read_level(levels: &[Level], limits: Limits, format: &str, read_one: impl Fn(&Path, Limits) -> Result<Scene, String>) -> Result<Scene, String> {
    let cloud: usize = levels.iter().flat_map(|l| &l.chunks).filter(|c| c.at == Where::Cloud).count();
    let all: usize = levels.iter().map(|l| l.chunks.len()).sum();
    let Some(i) = choose(levels, limits) else {
        return Err(if cloud > 0 {
            "None of this scene's chunks are on this computer: they're only in the cloud, and Filescape doesn't download them for a preview.".into()
        } else {
            "None of this scene's chunks could be found next to it.".into()
        });
    };
    let level = &levels[i];
    let of = level.here_splats();
    let mut scene: Option<Scene> = None;
    let mut failed = 0usize;
    let mut first_err = None;
    for c in level.here() {
        match read_one(&c.path, share(limits, c.count.unwrap_or(0), of)) {
            Ok(s) => merge(&mut scene, s),
            Err(e) => {
                failed += 1;
                first_err.get_or_insert(e);
            }
        }
    }
    let mut scene = scene.ok_or_else(|| first_err.unwrap_or_else(|| "This scene's chunks are empty.".into()))?;
    scene.format = format.into();
    let here = level.here().count();
    scene = finish_levels(scene, levels, i, limits);
    if let Some(v) = scene.said.iter_mut().find(|(k, _)| *k == "Showing") {
        v.1 = format!("{} ({} chunk{})", v.1, here, if here == 1 { "" } else { "s" });
    }
    if cloud > 0 {
        scene.said.push(("Only in the cloud", format!("{cloud} of {all} chunks (not downloaded for the preview)")));
        let better = (0..i).any(|j| levels[j].splats() <= limits.max_points as u64);
        if better || !level.all_here() {
            scene.notes.push("Some of the scene is only in the cloud, so the preview shows what's on this computer".into());
        }
    }
    let missing = levels.iter().flat_map(|l| &l.chunks).filter(|c| c.at == Where::Missing).count();
    if missing > 0 {
        scene.notes.push(format!("{missing} of its chunks are missing"));
    }
    if failed > 0 {
        scene.notes.push(format!("{failed} of the chunks shown couldn't be read"));
    }
    Ok(scene)
}

// ---- Streamed SOG.

/// A streamed SOG's `lod-meta.json`, read as levels: each level the chunks its tree names.
fn sog_levels(meta_path: &Path) -> Result<(Vec<Level>, Value), String> {
    let meta: Value = serde_json::from_slice(&read_meta(meta_path)?).map_err(|e| format!("This lod-meta.json is damaged: {e}"))?;
    if let Some(v) = meta["version"].as_u64()
        && v != 1
    {
        return Err(format!("Streamed SOG version {v} is newer than Filescape knows."));
    }
    let n = meta["lodLevels"].as_u64().filter(|&n| n > 0 && n <= 64).ok_or("This lod-meta.json doesn't say how many levels it has.")? as usize;
    let names: Vec<String> = meta["filenames"].as_array().ok_or("This lod-meta.json names no chunks.")?.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect();
    if names.len() > CHUNKS_MAX {
        return Err("This lod-meta.json names too many chunks.".into());
    }
    let dir = meta_path.parent().ok_or("No folder.")?;
    // Splats per level per chunk, summed over the tree's nodes.
    let mut counts: Vec<std::collections::BTreeMap<usize, u64>> = vec![Default::default(); n];
    let mut stack = vec![&meta["tree"]];
    let mut nodes = 0usize;
    while let Some(node) = stack.pop() {
        nodes += 1;
        if nodes > 10 * CHUNKS_MAX + 1000 {
            return Err("This lod-meta.json's tree is too big.".into());
        }
        if let Some(lods) = node["lods"].as_object() {
            for (k, r) in lods {
                let (Ok(l), Some(file), Some(count)) = (k.parse::<usize>(), r["file"].as_u64(), r["count"].as_u64()) else { continue };
                if l < n && (file as usize) < names.len() {
                    *counts[l].entry(file as usize).or_default() += count;
                }
            }
        }
        if let Some(ch) = node["children"].as_array() {
            stack.extend(ch.iter());
        }
    }
    let levels = counts
        .into_iter()
        .map(|files| Level {
            chunks: files
                .into_iter()
                .filter_map(|(f, count)| {
                    let path = inside(dir, &names[f])?;
                    let at = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("sog")) {
                        where_is(&path)
                    } else {
                        match where_is(&path) {
                            Where::Here => folder_where(path.parent()?),
                            other => other,
                        }
                    };
                    Some(Chunk { path, count: Some(count), at })
                })
                .collect(),
        })
        .collect();
    Ok((levels, meta))
}

pub fn read_streamed_sog(path: &Path, limits: Limits) -> Result<Scene, String> {
    let (levels, meta) = sog_levels(path)?;
    let mut scene = read_level(&levels, limits, "Streamed SOG", |p, l| {
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sog")) { super::sog::read_bundle(p, l) } else { super::sog::read_folder(p, l) }
    })?;
    if meta["environment"].as_str().is_some() {
        scene.notes.push("Its separate sky (environment) isn't shown".into());
    }
    Ok(scene)
}

// ---- LCC (XGRIDS), version 1.

struct LccUnit {
    /// Per level: splats, byte offset in data.bin, bytes.
    lods: Vec<(u32, u64, u32)>,
}

struct Lcc {
    meta: Value,
    units: Vec<LccUnit>,
    levels: usize,
    has_sh: bool,
}

fn attr(meta: &Value, name: &str, which: &str) -> Option<[f32; 3]> {
    let a = meta["attributes"].as_array()?.iter().find(|a| a["name"].as_str() == Some(name))?;
    let v: Vec<f32> = a[which].as_array()?.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
    (v.len() >= 3).then(|| [v[0], v[1], v[2]])
}

fn open_lcc(path: &Path) -> Result<Lcc, String> {
    let meta: Value = serde_json::from_slice(&read_meta(path)?).map_err(|e| format!("This LCC scene's description is damaged: {e}"))?;
    let levels = meta["totalLevel"].as_u64().filter(|&n| n > 0 && n <= 64).ok_or("This LCC scene doesn't say how many levels it has.")? as usize;
    let dir = path.parent().ok_or("No folder.")?;
    let index_path = dir.join("index.bin");
    if where_is(&index_path) == Where::Cloud {
        return Err("This LCC scene's index is only in the cloud, and Filescape doesn't download it for a preview.".into());
    }
    let index = std::fs::read(&index_path).map_err(|_| "This LCC scene's index.bin isn't next to it.")?;
    let rec = 4 + 16 * levels;
    if index.len() % rec != 0 || index.len() / rec > CHUNKS_MAX {
        return Err("This LCC scene's index.bin doesn't match its description.".into());
    }
    let units = index
        .chunks_exact(rec)
        .map(|r| LccUnit {
            lods: (0..levels)
                .map(|l| {
                    let o = 4 + 16 * l;
                    let points = i32::from_le_bytes(r[o..o + 4].try_into().unwrap()).max(0) as u32;
                    let off = i64::from_le_bytes(r[o + 4..o + 12].try_into().unwrap()).max(0) as u64;
                    let size = i32::from_le_bytes(r[o + 12..o + 16].try_into().unwrap()).max(0) as u32;
                    (points, off, size)
                })
                .collect(),
        })
        .collect();
    let has_sh = match meta["fileType"].as_str() {
        Some("Portable") => false,
        Some("Quality") => true,
        _ => attr(&meta, "shcoef", "min").is_some(),
    };
    Ok(Lcc { meta, units, levels, has_sh })
}

/// LCC's rotation: three 10-bit components and which one (the largest) was left out, as
/// splat-transform decodes it, returned x y z w.
fn lcc_quat(v: u32) -> [f32; 4] {
    let s = std::f32::consts::SQRT_2;
    let c = |b: u32| ((v >> b) & 1023) as f32 / 1023.0 * s - 1.0 / s;
    let (a, b, cc) = (c(0), c(10), c(20));
    let w = (1.0 - (a * a + b * b + cc * cc).min(1.0)).sqrt();
    // As (w, x, y, z), by which component was left out.
    let wxyz = match v >> 30 {
        0 => [cc, w, a, b],
        1 => [cc, a, w, b],
        2 => [cc, a, b, w],
        _ => [w, a, b, cc],
    };
    [wxyz[1], wxyz[2], wxyz[3], wxyz[0]]
}

pub fn read_lcc(path: &Path, limits: Limits) -> Result<Scene, String> {
    use std::io::{Read, Seek, SeekFrom};
    let lcc = open_lcc(path)?;
    let dir = path.parent().ok_or("No folder.")?;
    let data_path = dir.join("data.bin");
    let at = where_is(&data_path);
    // One file holds every level, so a level is "here" when data.bin is.
    let levels: Vec<Level> = (0..lcc.levels)
        .map(|l| Level { chunks: vec![Chunk { path: data_path.clone(), count: Some(lcc.units.iter().map(|u| u.lods[l].0 as u64).sum()), at }] })
        .collect();
    let Some(level) = choose(&levels, limits) else {
        return Err(match at {
            Where::Cloud => "This LCC scene's data is only in the cloud, and Filescape doesn't download it for a preview.".into(),
            _ => "This LCC scene's data.bin isn't next to it.".into(),
        });
    };
    let total = levels[level].splats();
    let step = super::every(total, limits);
    let mut scene = Scene::new(SceneKind::Splat, "LCC (XGRIDS)".into(), Frame::RightDownForward, total as usize);
    let sh_path = dir.join("shcoef.bin");
    let mut sh_file = None;
    if lcc.has_sh {
        if where_is(&sh_path) == Where::Here {
            if scene.plan_sh(Limits { max_points: (total / step).max(1) as usize, ..limits }, 15) {
                sh_file = std::fs::File::open(&sh_path).ok();
            }
        } else {
            scene.notes.push("Its view-dependent colour is only in the cloud, so base colour is shown".into());
        }
    }
    scene.sh_degree = if sh_file.is_some() { 3 } else { 0 };
    let (smin, smax) = (attr(&lcc.meta, "scale", "min").ok_or("This LCC scene's scales are missing.")?, attr(&lcc.meta, "scale", "max").ok_or("This LCC scene's scales are missing.")?);
    let (hmin, hmax) = (attr(&lcc.meta, "shcoef", "min").unwrap_or([0.0; 3]), attr(&lcc.meta, "shcoef", "max").unwrap_or([0.0; 3]));
    let mut data = std::fs::File::open(&data_path).map_err(|e| format!("Couldn't read data.bin: {e}"))?;
    let data_len = data.metadata().map(|m| m.len()).unwrap_or(0);
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let mut seen = 0u64;
    let mut buf = Vec::new();
    let mut sh_buf = Vec::new();
    let mut sh = [0f32; 45];
    for u in &lcc.units {
        let (points, off, _) = u.lods[level];
        if points == 0 {
            continue;
        }
        let bytes = points as u64 * 32;
        if off.saturating_add(bytes) > data_len {
            scene.notes.push("The file ends early".into());
            break;
        }
        buf.resize(bytes as usize, 0);
        data.seek(SeekFrom::Start(off)).and_then(|_| data.read_exact(&mut buf)).map_err(|e| format!("Couldn't read data.bin: {e}"))?;
        let has_sh = match sh_file.as_mut() {
            Some(f) => {
                sh_buf.resize(points as usize * 64, 0);
                f.seek(SeekFrom::Start(off * 2)).and_then(|_| f.read_exact(&mut sh_buf)).is_ok()
            }
            None => false,
        };
        if sh_file.is_some() && !has_sh {
            return Err("This LCC scene's shcoef.bin doesn't match its data.".into());
        }
        for i in 0..points as usize {
            let take = seen % step == 0;
            seen += 1;
            if !take {
                continue;
            }
            let r = &buf[i * 32..i * 32 + 32];
            let f = |o: usize| f32::from_le_bytes(r[o..o + 4].try_into().unwrap());
            let h = |o: usize| u16::from_le_bytes(r[o..o + 2].try_into().unwrap()) as f32 / 65535.0;
            let pos = [f(0), f(4), f(8)];
            let color = [r[12], r[13], r[14], r[15]];
            let scale = [mix(smin[0], smax[0], h(16)), mix(smin[1], smax[1], h(18)), mix(smin[2], smax[2], h(20))];
            let rot = lcc_quat(u32::from_le_bytes(r[22..26].try_into().unwrap()));
            let shv: &[f32] = if has_sh {
                let s = &sh_buf[i * 64..i * 64 + 64];
                for j in 0..15 {
                    let e = u32::from_le_bytes(s[j * 4..j * 4 + 4].try_into().unwrap());
                    sh[j * 3] = mix(hmin[0], hmax[0], (e & 0x7ff) as f32 / 2047.0);
                    sh[j * 3 + 1] = mix(hmin[1], hmax[1], ((e >> 11) & 0x3ff) as f32 / 1023.0);
                    sh[j * 3 + 2] = mix(hmin[2], hmax[2], ((e >> 21) & 0x7ff) as f32 / 2047.0);
                }
                &sh
            } else {
                &[]
            };
            scene.push(pos, color, Some(scale), Some(rot), shv);
        }
    }
    if lcc.has_sh && sh_file.is_none() {
        // Degree 3 in the file, even when not kept.
        scene.sh_degree = 3;
        scene.sh.clear();
    }
    if dir.join("environment.bin").exists() {
        scene.notes.push("Its separate sky (environment) isn't shown".into());
    }
    // The figures for what's shown.
    let mut out = finish_levels(scene, &levels, level, limits);
    up_is_z(&mut out);
    Ok(out)
}

/// Says which level of a one-file scene was shown.
fn finish_levels(mut scene: Scene, levels: &[Level], level: usize, limits: Limits) -> Scene {
    let n = levels.len();
    let finest = levels.iter().map(Level::splats).max().unwrap_or(0);
    scene.said.push(("Detail levels", if n == 1 { "One".into() } else { format!("{n}, the most detailed with {} splats", super::facts::group(finest as usize)) }));
    {
        let which = if n == 1 {
            "its only level".to_string()
        } else if level == 0 { "the most detailed".to_string() } else if level + 1 == n { "the coarsest".to_string() } else { format!("level {} of {n}", level + 1) };
        let why = if level > 0 && levels[0].splats() > limits.max_points as u64 { " (the more detailed ones are too big to preview)" } else { "" };
        scene.said.push(("Showing", format!("{which}{why}")));
    }
    scene
}

// ---- LCC2 (XGRIDS).

/// Drops commas before a closing bracket outside strings: some LCC2 exporters write them.
fn strip_trailing_commas(text: &str) -> String {
    let b: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut in_str, mut esc) = (false, false);
    for i in 0..b.len() {
        let c = b[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == ',' {
            let next = b[i + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn children(node: &Value) -> Vec<&Value> {
    match &node["child"] {
        Value::Array(a) => a.iter().collect(),
        Value::Object(m) => m.values().collect(),
        _ => Vec::new(),
    }
}

/// An LCC2 file as levels of chunk files, and whether they're SPZ (otherwise SOG).
fn lcc2_levels(path: &Path) -> Result<(Vec<Level>, bool, Value), String> {
    let text = String::from_utf8_lossy(&read_meta(path)?).to_string();
    let mut meta: Value = serde_json::from_str(&text).or_else(|_| serde_json::from_str(&strip_trailing_commas(&text))).map_err(|e| format!("This LCC2 scene's description is damaged: {e}"))?;
    let old = meta.get("total_splats").is_some() && meta.get("lod_3dgs_info").is_some() && meta.get("lod_level").is_some();
    let levels = if old { meta["lod_level"].as_u64() } else { meta["totalLevels"].as_u64() };
    let n = levels.filter(|&n| n > 0 && n <= 64).ok_or("This LCC2 scene doesn't say how many levels it has.")? as usize;
    let files: Vec<String> = if old {
        meta["root"]["files"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|f| {
                        let f = f.as_str().unwrap_or("").trim_start_matches('/');
                        if f.ends_with(".sog") { f.to_string() } else { format!("{f}.sog") }
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        meta["root"]["splatFiles"].as_array().map(|a| a.iter().map(|f| f.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
    };
    if files.is_empty() {
        return Err("This LCC2 scene names no chunks.".into());
    }
    if files.len() > CHUNKS_MAX {
        return Err("This LCC2 scene names too many chunks.".into());
    }
    let spz = meta["splatType"].as_str() == Some(".spz");
    let env = meta["root"]["data"]["env"]["name"].as_u64();
    let dir = path.parent().ok_or("No folder.")?;
    // Level = levels - depth, so the deepest nodes are level 0, the most detailed.
    let mut by_level: Vec<std::collections::BTreeMap<usize, Option<u64>>> = vec![Default::default(); n];
    let mut stack: Vec<(&Value, usize)> = children(&meta["root"]).into_iter().map(|c| (c, 1)).collect();
    let mut nodes = 0usize;
    while let Some((node, depth)) = stack.pop() {
        nodes += 1;
        if nodes > 10 * CHUNKS_MAX + 1000 {
            return Err("This LCC2 scene's tree is too big.".into());
        }
        let d = &node["data"]["3dgs"];
        if let Some(f) = d["name"].as_u64()
            && Some(f) != env
            && (f as usize) < files.len()
            && depth <= n
        {
            let e = by_level[n - depth].entry(f as usize).or_insert(Some(0));
            *e = match (*e, d["count"].as_u64()) {
                (Some(a), Some(c)) => Some(a + c),
                _ => None,
            };
        }
        if depth < 64 {
            stack.extend(children(node).into_iter().map(|c| (c, depth + 1)));
        }
    }
    let levels = by_level
        .into_iter()
        .map(|m| Level {
            chunks: m
                .into_iter()
                .filter_map(|(f, count)| {
                    let mut p = inside(dir, &files[f])?;
                    // Some exporters name chunks with folders that aren't there: the bare name.
                    if !p.exists()
                        && let Some(base) = p.file_name().map(|b| dir.join(b))
                        && base.exists()
                    {
                        p = base;
                    }
                    let at = where_is(&p);
                    let count = count.or_else(|| if at == Where::Here { if spz { super::spz::count(&p) } else { super::sog::count(&p) } } else { None });
                    Some(Chunk { path: p, count, at })
                })
                .collect(),
        })
        .collect();
    meta["_env"] = Value::Bool(env.is_some());
    Ok((levels, spz, meta))
}

pub fn read_lcc2(path: &Path, limits: Limits) -> Result<Scene, String> {
    let (levels, spz, meta) = lcc2_levels(path)?;
    let mut scene = read_level(&levels, limits, if spz { "LCC2 (XGRIDS, SPZ chunks)" } else { "LCC2 (XGRIDS, SOG chunks)" }, |p, l| {
        if spz {
            // splat-transform reads SPZ chunks with no change of axes: their numbers are in
            // the LCC2 scene's own axes, like the SOG chunks'.
            super::spz::read(p, l).map(|mut s| {
                s.frame = Frame::RightDownForward;
                s
            })
        } else {
            super::sog::read_bundle(p, l)
        }
    })?;
    if meta["_env"].as_bool() == Some(true) {
        scene.notes.push("Its separate sky (environment) isn't shown".into());
    }
    up_is_z(&mut scene);
    Ok(scene)
}

// ---- Turning XGRIDS scenes the right way up.

/// XGRIDS scenes stand on z; the rest of Filescape expects the trainers' axes. A quarter turn
/// about x, as splat-transform's fixed LCC transform works out to against its PLY one.
pub(super) fn up_is_z(scene: &mut Scene) {
    let r = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]];
    let h = std::f32::consts::FRAC_1_SQRT_2;
    rotate(scene, r, [h, 0.0, 0.0, h]);
}

pub(super) fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let ([ax, ay, az, aw], [bx, by, bz, bw]) = (a, b);
    [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]
}

/// Turns every splat by `r` (the same turn as the quaternion `q`, x y z w), with its
/// view-dependent colour.
pub(super) fn rotate(scene: &mut Scene, r: [[f32; 3]; 3], q: [f32; 4]) {
    let m = |p: [f32; 3]| [r[0][0] * p[0] + r[0][1] * p[1] + r[0][2] * p[2], r[1][0] * p[0] + r[1][1] * p[1] + r[1][2] * p[2], r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2]];
    for p in &mut scene.pos {
        *p = m(*p);
    }
    for s in &mut scene.rot {
        *s = qmul(q, *s);
    }
    let degree = scene.sh_degree.min(3) as usize;
    let rest = super::sh_rest(scene.sh_degree);
    if scene.sh.is_empty() || rest == 0 || scene.sh.len() % (rest * 3) != 0 {
        return;
    }
    let mats: Vec<Vec<f64>> = (1..=degree).map(|l| sh_turn(l, r)).collect();
    let mut tmp = vec![0f32; rest * 3];
    for point in scene.sh.chunks_exact_mut(rest * 3) {
        for (k, v) in point.iter().enumerate() {
            tmp[k] = v.to_f32();
        }
        let mut start = 0;
        for (li, mat) in mats.iter().enumerate() {
            let n = 2 * (li + 1) + 1;
            for c in 0..3 {
                for j in 0..n {
                    let mut acc = 0f64;
                    for i in 0..n {
                        acc += tmp[(start + i) * 3 + c] as f64 * mat[i * n + j];
                    }
                    point[(start + j) * 3 + c] = half::f16::from_f64(acc);
                }
            }
            start += n;
        }
    }
}

/// The 3DGS real spherical harmonics of degree `l` (1–3), in its order and with its signs.
pub(super) fn sh_basis(l: usize, d: [f64; 3]) -> Vec<f64> {
    let [x, y, z] = d;
    let (xx, yy, zz) = (x * x, y * y, z * z);
    match l {
        1 => {
            let c = 0.4886025119029199;
            vec![-c * y, c * z, -c * x]
        }
        2 => vec![1.0925484305920792 * x * y, -1.0925484305920792 * y * z, 0.31539156525252005 * (2.0 * zz - xx - yy), -1.0925484305920792 * x * z, 0.5462742152960396 * (xx - yy)],
        _ => vec![
            -0.5900435899266435 * y * (3.0 * xx - yy),
            2.890611442640554 * x * y * z,
            -0.4570457994644658 * y * (4.0 * zz - xx - yy),
            0.3731763325901154 * z * (2.0 * zz - 3.0 * xx - 3.0 * yy),
            -0.4570457994644658 * x * (4.0 * zz - xx - yy),
            1.445305721320277 * z * (xx - yy),
            -0.5900435899266435 * x * (xx - 3.0 * yy),
        ],
    }
}

/// The matrix M (row-major, n × n) that turns degree-`l` coefficients with the scene:
/// new[j] = Σ old[i] · M[i][j], so that the colour seen along a turned direction is the colour
/// that was seen along the original one. Found by least squares over many directions.
fn sh_turn(l: usize, r: [[f32; 3]; 3]) -> Vec<f64> {
    let n = 2 * l + 1;
    // Directions spread over the sphere (a Fibonacci lattice).
    let k = 96;
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    let dirs: Vec<[f64; 3]> = (0..k)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f64 + 0.5) / k as f64;
            let rad = (1.0 - y * y).sqrt();
            let t = golden * i as f64;
            [rad * t.cos(), y, rad * t.sin()]
        })
        .collect();
    // The inverse turn is the transpose.
    let back = |d: [f64; 3]| {
        let c = |i: usize| r[0][i] as f64 * d[0] + r[1][i] as f64 * d[1] + r[2][i] as f64 * d[2];
        [c(0), c(1), c(2)]
    };
    // Y(back(d)) = Σ_j M[i][j] Y_j(d): solve (AᵀA) Mᵀ = AᵀB.
    let a: Vec<Vec<f64>> = dirs.iter().map(|&d| sh_basis(l, d)).collect();
    let b: Vec<Vec<f64>> = dirs.iter().map(|&d| sh_basis(l, back(d))).collect();
    let mut ata = vec![0f64; n * n];
    let mut atb = vec![0f64; n * n];
    for s in 0..k {
        for p in 0..n {
            for q in 0..n {
                ata[p * n + q] += a[s][p] * a[s][q];
                atb[p * n + q] += a[s][p] * b[s][q];
            }
        }
    }
    // Gaussian elimination with partial pivoting; atb becomes Mᵀ.
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| ata[i * n + col].abs().total_cmp(&ata[j * n + col].abs())).unwrap();
        for c in 0..n {
            ata.swap(col * n + c, piv * n + c);
            atb.swap(col * n + c, piv * n + c);
        }
        let d = ata[col * n + col];
        for c in 0..n {
            ata[col * n + c] /= d;
            atb[col * n + c] /= d;
        }
        for row in 0..n {
            if row != col {
                let f = ata[row * n + col];
                if f != 0.0 {
                    for c in 0..n {
                        ata[row * n + c] -= f * ata[col * n + c];
                        atb[row * n + c] -= f * atb[col * n + c];
                    }
                }
            }
        }
    }
    // atb[j][i] = M[i][j]: return M row-major.
    let mut m = vec![0f64; n * n];
    for i in 0..n {
        for j in 0..n {
            m[i * n + j] = atb[j * n + i];
        }
    }
    m
}

// ---- Counts for badges, from the descriptions alone.

pub fn count(path: &Path) -> Option<(&'static str, u64)> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let meta: Value = {
        let bytes = read_meta(path).ok()?;
        let text = String::from_utf8_lossy(&bytes).to_string();
        serde_json::from_str(&text).or_else(|_| serde_json::from_str(&strip_trailing_commas(&text))).ok()?
    };
    if name == "lod-meta.json" {
        let n = meta["counts"].as_array().and_then(|a| a.first()).and_then(Value::as_u64).or_else(|| meta["count"].as_u64())?;
        Some(("Streamed SOG", n))
    } else if name.ends_with(".lcc2") {
        let n = meta["totalSplats"].as_u64().or_else(|| meta["total_splats"].as_u64())?;
        let finest = meta["lodSplats"].as_array().and_then(|a| a.first()).and_then(Value::as_u64).or_else(|| meta["lod_3dgs_info"].as_array().and_then(|a| a.last()).and_then(Value::as_u64));
        Some(("LCC2", finest.unwrap_or(n)))
    } else {
        Some(("LCC", meta["splats"].as_array()?.iter().filter_map(Value::as_u64).max()?))
    }
}
