//! Gaussian splat scenes (3D Gaussian Splatting): told apart from meshes and point clouds,
//! read with our own readers, described, and drawn as points for the preview.
//!
//! Splat files are untrusted and can be several gigabytes, so every reader checks counts
//! against the file's real length before allocating, never keeps more than [`Limits`] allows,
//! and reads big files by sampling blocks rather than all of it when only a picture is needed.
//! Nothing in a file is run. See `docs/design/filescape-splats.md`.

pub mod facts;
pub mod glb;
pub mod gpu;
pub mod html;
pub mod ply;
#[allow(clippy::module_inception)]
pub mod splat;
pub mod sog;
pub mod spz;

use std::path::Path;

/// What a PLY file holds, judged from its header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlyKind {
    /// Faces: a mesh, for the 3D model preview.
    Mesh,
    /// Points with at most a colour and normal.
    PointCloud,
    /// 3D Gaussian Splatting: colour as spherical harmonics, opacity, scale and rotation.
    Splat,
    /// A splat scene that changes over time (Space-Time Gaussians and similar).
    Splat4d,
    /// PlayCanvas's compressed PLY (packed chunks), read in a later stage.
    CompressedSplat,
    /// Something else (no vertex positions).
    Unknown,
}

/// Which way the scene's axes point. 3DGS trainers write PLYs in the cameras' frame (COLMAP:
/// x right, y down, z forward); SPZ is stored x right, y up, z back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Frame {
    RightDownForward,
    RightUpBack,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SceneKind {
    Splat,
    Splat4d,
    PointCloud,
}

/// How much of a scene to keep.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Points kept at most. Bigger scenes are thinned evenly and say so.
    pub max_points: usize,
    /// Keep the spherical harmonics (view-dependent colour) when there's room.
    pub keep_sh: bool,
}

impl Limits {
    /// For the preview pane.
    pub const PREVIEW: Limits = Limits { max_points: 4_000_000, keep_sh: true };
    /// For a thumbnail: a sample is plenty.
    pub const THUMB: Limits = Limits { max_points: 400_000, keep_sh: false };

    /// Memory the view-dependent colour may take (half precision); scenes needing more are
    /// shown with base colour only, and say so.
    pub const SH_BYTES: u64 = 256 << 20;

    /// Whether to keep harmonics for `points` splats with `rest` higher values per channel.
    pub fn keeps_sh(&self, points: u64, rest: usize) -> bool {
        self.keep_sh && rest > 0 && points.min(self.max_points as u64) * rest as u64 * 3 * 2 <= Self::SH_BYTES
    }
}

/// The note for a scene whose view-dependent colour was left out to save memory.
pub const SH_DROPPED: &str = "Too big to show its view-dependent colour, so base colour is shown";

/// Points kept by any reader, whatever the limits say.
pub const HARD_MAX_POINTS: usize = 50_000_000;
/// SPZ point counts above this many points per byte of file are lies (Niantic's own bound).
pub const SPZ_MAX_POINTS_PER_BYTE: u64 = 1024 / 9;

/// A scene read into memory. Colours already include the base colour of the spherical
/// harmonics (`f_dc * C0 + 0.5`), and opacity is in alpha (0–255).
#[derive(Debug)]
pub struct Scene {
    pub kind: SceneKind,
    /// "PLY", "SPZ v2" and so on.
    pub format: String,
    pub frame: Frame,
    /// Points in the file. More than `pos.len()` when the scene was thinned or cut short.
    pub total: usize,
    pub pos: Vec<[f32; 3]>,
    pub color: Vec<[u8; 4]>,
    /// Linear scale (standard deviations) per axis; empty for point clouds.
    pub scale: Vec<[f32; 3]>,
    /// Unit rotation quaternions, x y z w; empty for point clouds.
    pub rot: Vec<[f32; 4]>,
    /// Degree of the spherical harmonics in the file (0–4).
    pub sh_degree: u8,
    /// Higher spherical-harmonic coefficients when kept: per point, coefficient by coefficient
    /// with r g b inside.
    pub sh: Vec<half::f16>,
    pub antialiased: bool,
    /// Points dropped for non-finite numbers.
    pub bad: usize,
    /// What the reader noticed: "The file ends early", and so on.
    pub notes: Vec<String>,
    /// What the file says about itself (header comments, the app that made it).
    pub said: Vec<(&'static str, String)>,
    /// Filled in by [`facts::describe`].
    pub facts: Vec<(&'static str, String)>,
}

impl Scene {
    fn new(kind: SceneKind, format: String, frame: Frame, total: usize) -> Scene {
        Scene {
            kind,
            format,
            frame,
            total,
            pos: Vec::new(),
            color: Vec::new(),
            scale: Vec::new(),
            rot: Vec::new(),
            sh_degree: 0,
            sh: Vec::new(),
            antialiased: false,
            bad: 0,
            notes: Vec::new(),
            said: Vec::new(),
            facts: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// Keeps a splat, or counts it as bad when any of its numbers aren't finite.
    #[allow(clippy::too_many_arguments)]
    fn push(&mut self, pos: [f32; 3], color: [u8; 4], scale: Option<[f32; 3]>, rot: Option<[f32; 4]>, sh: &[f32]) {
        let finite = pos.iter().all(|v| v.is_finite()) && scale.is_none_or(|s| s.iter().all(|v| v.is_finite())) && rot.is_none_or(|r| r.iter().all(|v| v.is_finite()));
        if !finite {
            self.bad += 1;
            return;
        }
        self.pos.push(pos);
        self.color.push(color);
        if let Some(s) = scale {
            self.scale.push(s);
        }
        if let Some(r) = rot {
            let n = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3]).sqrt();
            self.rot.push(if n > 1e-12 { [r[0] / n, r[1] / n, r[2] / n, r[3] / n] } else { [0.0, 0.0, 0.0, 1.0] });
        }
        if !sh.is_empty() {
            self.sh.extend(sh.iter().map(|v| half::f16::from_f32(*v)));
        }
    }
}

impl Scene {
    /// Keeps a splat given its base colour as display RGB (0–1), linear opacity, linear scale,
    /// an x y z w rotation and its higher harmonics in our order (may be empty).
    #[allow(clippy::too_many_arguments)]
    fn push_rgb(&mut self, pos: [f32; 3], rgb: [f32; 3], alpha: f32, scale: [f32; 3], rot: [f32; 4], sh: &[f32]) {
        let color = [to_u8(rgb[0]), to_u8(rgb[1]), to_u8(rgb[2]), to_u8(alpha)];
        self.push(pos, color, Some(scale), Some(rot), sh);
    }

    /// As [`Scene::push_rgb`], with the base colour as the 0th harmonic (`f_dc`).
    fn push_dc(&mut self, pos: [f32; 3], dc: [f32; 3], alpha: f32, scale: [f32; 3], rot: [f32; 4], sh: &[f32]) {
        self.push_rgb(pos, [dc[0] * SH_C0 + 0.5, dc[1] * SH_C0 + 0.5, dc[2] * SH_C0 + 0.5], alpha, scale, rot, sh);
    }

    /// Whether to keep this scene's harmonics (`rest` per channel), noting it when they're
    /// left out to save memory.
    fn plan_sh(&mut self, limits: Limits, rest: usize) -> bool {
        let keep = limits.keeps_sh(self.total as u64, rest);
        if limits.keep_sh && rest > 0 && !keep {
            self.notes.push(SH_DROPPED.into());
        }
        keep
    }
}

/// Keep every this-many splats of `total` to stay within `limits`.
fn every(total: u64, limits: Limits) -> u64 {
    total.div_ceil(limits.max_points.min(HARD_MAX_POINTS).max(1) as u64).max(1)
}

/// The degree of spherical harmonics with this many higher values per channel.
fn degree_for(rest_per_channel: usize) -> Option<u8> {
    match rest_per_channel {
        0 => Some(0),
        3 => Some(1),
        8 => Some(2),
        15 => Some(3),
        24 => Some(4),
        _ => None,
    }
}

/// Spherical-harmonic coefficients per colour channel beyond the base one, by degree.
pub fn sh_rest(degree: u8) -> usize {
    match degree {
        0 => 0,
        1 => 3,
        2 => 8,
        3 => 15,
        _ => 24,
    }
}

/// The base colour from the 0th spherical harmonic, as 3DGS defines it.
pub const SH_C0: f32 = 0.282_094_8;

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn to_u8(v: f32) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

/// What a PLY file holds, from its header alone (the first 64 KB at most).
pub fn classify_ply(path: &Path) -> Result<PlyKind, String> {
    let head = ply::Header::read(path)?;
    Ok(head.kind())
}

/// Whether a file that could be something else (a PLY mesh, a GLB model, a web page, a
/// `meta.json`) holds a splat scene. Files with splat-only extensions say so by their name.
pub fn claims(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "ply" => classify_ply(path).is_ok_and(|k| k != PlyKind::Mesh && k != PlyKind::Unknown),
        "glb" => glb::is_splat_glb(path),
        "html" | "htm" => html::has_scene(path),
        "json" if name == "meta.json" => std::fs::File::open(path).is_ok_and(|f| {
            use std::io::Read;
            let mut head = Vec::new();
            f.take(64 << 10).read_to_end(&mut head).is_ok() && sog::is_meta(&head)
        }),
        "splat" | "ksplat" | "sog" | "spz" => true,
        _ => false,
    }
}

/// A short label for a thumbnail or map tile, from the file's header alone (a few KB, never
/// the whole scene): the format and how many splats, such as "SPZ · 1.2M splats".
pub fn badge(path: &Path) -> Option<String> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    let (format, n, noun) = match ext {
        "ply" => {
            let h = ply::Header::read(path).ok()?;
            match h.kind() {
                PlyKind::PointCloud => ("PLY", h.vertices(), "points"),
                PlyKind::Splat4d => ("4D PLY", h.vertices(), "splats"),
                PlyKind::CompressedSplat => ("Compressed PLY", h.vertices(), "splats"),
                PlyKind::Splat => ("PLY", h.vertices(), "splats"),
                _ => return None,
            }
        }
        "spz" => ("SPZ", spz::count(path)?, "splats"),
        "splat" => ("SPLAT", std::fs::metadata(path).ok()?.len() / 32, "splats"),
        "ksplat" => ("KSPLAT", splat::ksplat_count(path)?, "splats"),
        "sog" | "json" => ("SOG", sog::count(path)?, "splats"),
        "glb" => ("GLB", glb::count(path)?, "splats"),
        // The scene is buried in the page's text: no count without decoding all of it.
        "html" | "htm" => return Some("Splat web page".into()),
        _ => return None,
    };
    Some(format!("{format} · {} {noun}", short_count(n)))
}

/// 312, 48K, 1.2M.
pub fn short_count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..999_500 => format!("{}K", (n as f64 / 1e3).round()),
        _ if n < 9_950_000 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{}M", (n as f64 / 1e6).round()),
    }
}

/// Read a scene, keeping what `limits` allows.
pub fn read(path: &Path, limits: Limits) -> Result<Scene, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    let mut scene = match ext {
        "spz" => spz::read(path, limits)?,
        "splat" => splat::read_splat(path, limits)?,
        "ksplat" => splat::read_ksplat(path, limits)?,
        "sog" => sog::read_bundle(path, limits)?,
        "json" => sog::read_folder(path, limits)?,
        "glb" => glb::read(path, limits)?,
        "html" | "htm" => html::read(path, limits)?,
        _ => ply::read(path, limits)?,
    };
    facts::describe(&mut scene, path);
    Ok(scene)
}

/// How the scene is looked at: turned (as the 3D model preview turns), zoomed and panned.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
    /// Distance from the framed middle, as a multiple of the starting distance.
    pub zoom: f32,
    /// Sideways and up, in units of the scene's framed radius.
    pub pan: [f32; 2],
}

impl Default for View {
    fn default() -> Self {
        let a = crate::model3d::Angle::default();
        View { yaw: a.yaw, pitch: a.pitch, zoom: 1.0, pan: [0.0, 0.0] }
    }
}

/// Vertical field of view of the preview camera.
pub const FOV: f32 = 50.0 * std::f32::consts::PI / 180.0;

impl View {
    /// Camera distance for a scene framed with radius `r`.
    pub fn distance(&self, r: f32) -> f32 {
        r / (FOV / 2.0).tan() * 1.35 * self.zoom
    }

    /// The rotation from the scene (y up, middle at the origin) to the camera's axes, as
    /// rows: x right, y up, z towards the viewer.
    pub fn rotation(&self) -> [[f32; 3]; 3] {
        let (sy, cy) = (self.yaw + std::f32::consts::PI).sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        // Turn about y, then tilt about x.
        [[cy, 0.0, sy], [-sp * -sy, cp, -sp * cy], [cp * -sy, sp, cp * cy]]
    }

    /// How far one pixel of a picture `h` pixels high moves the pan.
    pub fn pan_per_pixel(&self, h: f32) -> f32 {
        self.zoom * 2.7 / h.max(1.0)
    }
}

/// Where the camera looks: the middle of the scene and how far out it reaches, from the 10th
/// to the 90th percentile of positions on each axis (captures carry a shell of sky and
/// floaters far out that would otherwise shrink the subject to a dot).
pub fn framing(scene: &Scene) -> ([f32; 3], f32) {
    if scene.pos.is_empty() {
        return ([0.0; 3], 1.0);
    }
    let step = (scene.pos.len() / 200_000).max(1);
    let mut centre = [0.0f32; 3];
    let mut extent = [0.0f32; 3];
    for a in 0..3 {
        let mut v: Vec<f32> = scene.pos.iter().step_by(step).map(|p| p[a]).collect();
        v.sort_unstable_by(f32::total_cmp);
        let lo = v[v.len() / 10];
        let hi = v[(v.len() * 9 / 10).min(v.len() - 1)];
        centre[a] = (lo + hi) / 2.0;
        extent[a] = hi - lo;
    }
    let r = 0.5 * (extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt();
    (centre, if r > 0.0 && r.is_finite() { r } else { 1.0 })
}

impl Scene {
    /// The scene drawn `w` × `h` as soft round points, back to front, on transparent (RGBA,
    /// not premultiplied). A stand-in for real splatting: it shows what's there and where.
    /// `step` > 1 draws only every `step`th point (quicker while turning).
    pub fn draw(&self, w: usize, h: usize, view: &View, step: usize) -> Vec<u8> {
        let mut out = vec![0u8; w * h * 4];
        if w == 0 || h == 0 || self.pos.is_empty() {
            return out;
        }
        let (c, r) = framing(self);
        // Up is -y for trainer PLYs, +y for SPZ.
        let flip = if self.frame == Frame::RightDownForward { -1.0 } else { 1.0 };
        let m = view.rotation();
        let dist = view.distance(r);
        let focal = 0.5 * h as f32 / (FOV / 2.0).tan();
        let (cx, cyc) = (w as f32 / 2.0, h as f32 / 2.0);
        let near = dist * 0.02;
        let (px0, py0) = (view.pan[0] * r, view.pan[1] * r);
        let step = step.max(1);
        // Project: (screen x, screen y, depth, radius in pixels, index).
        let mut pts: Vec<(f32, f32, f32, f32, u32)> = Vec::with_capacity(self.pos.len() / step + 1);
        for i in (0..self.pos.len()).step_by(step) {
            let p = self.pos[i];
            let v = [p[0] - c[0], flip * (p[1] - c[1]), flip * (p[2] - c[2])];
            let x = m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2] - px0;
            let y = m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2] - py0;
            let z = m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2];
            let depth = dist - z;
            if depth < near {
                continue;
            }
            let sx = cx + x * focal / depth;
            let syy = cyc - y * focal / depth;
            if sx < -8.0 || syy < -8.0 || sx > w as f32 + 8.0 || syy > h as f32 + 8.0 {
                continue;
            }
            let size = self.scale.get(i).map_or(0.0, |s| s[0].max(s[1]).max(s[2]) * 1.6);
            let rad = (size * focal / depth).clamp(0.75, if step > 1 { 2.5 } else { 4.0 }) * (step as f32).sqrt().min(2.0);
            pts.push((sx, syy, depth, rad, i as u32));
        }
        pts.sort_unstable_by(|a, b| b.2.total_cmp(&a.2));
        // Premultiplied colour and coverage, composited back to front.
        let mut acc = vec![[0f32; 4]; w * h];
        for (sx, syy, _, rad, i) in pts {
            let col = self.color[i as usize];
            let alpha = if self.kind == SceneKind::PointCloud { 1.0 } else { (col[3] as f32 / 255.0).min(0.99) };
            if alpha < 0.02 {
                continue;
            }
            let rgb = [col[0] as f32 / 255.0, col[1] as f32 / 255.0, col[2] as f32 / 255.0];
            let x0 = (sx - rad).floor().max(0.0) as usize;
            let x1 = ((sx + rad).ceil() as isize).min(w as isize - 1);
            let y0 = (syy - rad).floor().max(0.0) as usize;
            let y1 = ((syy + rad).ceil() as isize).min(h as isize - 1);
            if x1 < 0 || y1 < 0 {
                continue;
            }
            let inv = 1.0 / (rad * rad);
            for y in y0..=y1 as usize {
                let dy = y as f32 + 0.5 - syy;
                for x in x0..=x1 as usize {
                    let dx = x as f32 + 0.5 - sx;
                    let d2 = (dx * dx + dy * dy) * inv;
                    if d2 > 1.0 {
                        continue;
                    }
                    let a = alpha * (-2.0 * d2).exp();
                    let px = &mut acc[y * w + x];
                    for k in 0..3 {
                        px[k] = rgb[k] * a + px[k] * (1.0 - a);
                    }
                    px[3] = a + px[3] * (1.0 - a);
                }
            }
        }
        for (i, px) in acc.iter().enumerate() {
            if px[3] > 0.0 {
                let o = &mut out[i * 4..i * 4 + 4];
                for k in 0..3 {
                    o[k] = to_u8(px[k] / px[3]);
                }
                o[3] = to_u8(px[3]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
