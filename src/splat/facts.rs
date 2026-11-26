//! What a scene is: counts, colour detail, size, and what looks odd. Values worked out from
//! the points are labelled as such, and memory figures are estimates.

use super::{Scene, SceneKind, sh_rest};
use crate::scan::human;
use std::path::Path;

/// GPU bytes per splat at half precision, by spherical-harmonic degree (the viewer's layouts:
/// 32 bytes without harmonics, 128 with them up to degree 3).
pub fn gpu_bytes_per_splat(degree: u8) -> u64 {
    if degree == 0 { 32 } else { 128 }
}

/// Bytes per splat in a standard 3DGS PLY with these harmonics: position, normal, 3 base
/// colour values, the higher harmonics, opacity, 3 scales and 4 rotation values, as floats.
pub fn ply_bytes_per_splat(degree: u8) -> u64 {
    (3 + 3 + 3 + sh_rest(degree) as u64 * 3 + 1 + 3 + 4) * 4
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
    let a = v.abs();
    if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.3}")
    }
}

/// Quality checks, as counts of points.
#[derive(Default, Debug, PartialEq)]
pub struct Checks {
    /// Nearly transparent (opacity below 2%).
    pub faint: usize,
    /// Much bigger than the scene's typical splat (over 200 times the median), in the main body.
    pub huge: usize,
    /// Far outside the scene's main body (over 10 times its radius from the middle).
    pub far: usize,
}

pub fn checks(scene: &Scene) -> Checks {
    let mut c = Checks::default();
    if scene.kind == SceneKind::PointCloud || scene.pos.is_empty() {
        return c;
    }
    let (mid, r) = super::framing(scene);
    let mut sizes: Vec<f32> = scene.scale.iter().step_by((scene.scale.len() / 100_000).max(1)).map(|s| s[0].max(s[1]).max(s[2])).collect();
    sizes.sort_unstable_by(f32::total_cmp);
    let median = sizes.get(sizes.len() / 2).copied().unwrap_or(0.0);
    let far2 = (10.0 * r) * (10.0 * r);
    for i in 0..scene.pos.len() {
        if scene.color[i][3] < 5 {
            c.faint += 1;
        }
        let p = scene.pos[i];
        let d2 = (p[0] - mid[0]).powi(2) + (p[1] - mid[1]).powi(2) + (p[2] - mid[2]).powi(2);
        if d2 > far2 {
            c.far += 1;
        } else if let Some(s) = scene.scale.get(i) {
            // Far-out splats are big on purpose (sky, distant walls), so only those in the main
            // body count.
            if median > 0.0 && s[0].max(s[1]).max(s[2]) > 200.0 * median {
                c.huge += 1;
            }
        }
    }
    c
}

/// Fill `scene.facts`.
pub fn describe(scene: &mut Scene, path: &Path) {
    let file_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut f: Vec<(&'static str, String)> = Vec::new();
    let what = match scene.kind {
        SceneKind::Splat => "Gaussian splat scene",
        SceneKind::Splat4d => "Gaussian splat scene that changes over time (shown at its start)",
        SceneKind::PointCloud => "Point cloud",
    };
    f.push(("Holds", what.into()));
    f.push(("Format", scene.format.clone()));
    let noun = if scene.kind == SceneKind::PointCloud { "Points" } else { "Splats" };
    f.push((noun, group(scene.total)));
    if scene.len() + scene.bad < scene.total {
        f.push(("Shown", format!("{} (a sample: there are too many to show all)", group(scene.len()))));
    }
    if scene.kind != SceneKind::PointCloud {
        let detail = match scene.sh_degree {
            0 => "Base colour only (no view-dependent colour)".to_string(),
            d => format!("Degree {d} spherical harmonics ({} values per splat)", (sh_rest(d) + 1) * 3),
        };
        f.push(("Colour", detail));
        if scene.antialiased {
            f.push(("Trained", "With antialiasing (Mip-Splatting)".into()));
        }
        if scene.total > 0 && file_len > 0 {
            let per = file_len as f64 / scene.total as f64;
            f.push(("Bytes per splat", format!("{per:.1}")));
            let ply = scene.total as u64 * ply_bytes_per_splat(scene.sh_degree);
            if !scene.format.starts_with("PLY") && ply > file_len {
                f.push(("As a standard PLY", format!("about {} ({:.0} times bigger, calculated)", human(ply), ply as f64 / file_len as f64)));
            }
        }
        let gpu = scene.total as u64 * gpu_bytes_per_splat(scene.sh_degree);
        f.push(("Graphics memory", format!("about {} to view (estimate)", human(gpu))));
    }
    if !scene.pos.is_empty() {
        let (lo, hi) = bounds(scene);
        f.push(("Size (main body)", format!("{} × {} × {} in the scene's units (calculated)", num(hi[0] - lo[0]), num(hi[1] - lo[1]), num(hi[2] - lo[2]))));
    }
    let c = checks(scene);
    let pct = |n: usize| 100.0 * n as f64 / scene.len().max(1) as f64;
    if scene.bad > 0 {
        f.push(("Unreadable", format!("{} splats have numbers that aren't valid (skipped)", group(scene.bad))));
    }
    if c.faint > 0 {
        f.push(("Nearly invisible", format!("{} ({:.1}%) under 2% opacity", group(c.faint), pct(c.faint))));
    }
    if c.huge > 0 {
        f.push(("Oversized", format!("{} ({:.2}%) in the main body, over 200 times the typical size", group(c.huge), pct(c.huge))));
    }
    if c.far > 0 {
        f.push(("Far out", format!("{} ({:.2}%) far from the main body: often sky or floaters", group(c.far), pct(c.far))));
    }
    for n in &scene.notes {
        f.push(("Note", n.clone()));
    }
    f.extend(scene.said.iter().cloned());
    scene.facts = f;
}

/// The 10th to 90th percentile box of positions: the scene's main body, without the shell of
/// sky and stray splats that captures carry.
pub fn bounds(scene: &Scene) -> ([f32; 3], [f32; 3]) {
    let step = (scene.pos.len() / 200_000).max(1);
    let mut lo = [0f32; 3];
    let mut hi = [0f32; 3];
    for a in 0..3 {
        let mut v: Vec<f32> = scene.pos.iter().step_by(step).map(|p| p[a]).collect();
        v.sort_unstable_by(f32::total_cmp);
        lo[a] = v[v.len() / 10];
        hi[a] = v[(v.len() * 9 / 10).min(v.len() - 1)];
    }
    (lo, hi)
}
