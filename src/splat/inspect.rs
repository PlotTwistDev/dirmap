//! The inspector: ways of looking at a scene other than as its picture (outlines, centres,
//! false colour by opacity, size, crowding or distance), and the quality checks, worked out
//! per splat so the splats a check counts can be picked out on screen.

use super::{Scene, SceneKind};

/// How the splats are drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Look {
    /// The scene as it was made.
    #[default]
    Splats,
    /// Each splat's outline, to see their shapes and how they overlap.
    Ellipses,
    /// Each splat's centre.
    Centres,
    /// False colour: how see-through each splat is.
    Opacity,
    /// False colour: how big each splat is next to the scene's typical one.
    Size,
    /// False colour: how crowded each splat's neighbourhood is.
    Density,
    /// False colour: how far each splat is from the middle of the scene.
    Distance,
    /// The splats the quality checks count, in colour; the rest grey.
    Problems,
}

impl Look {
    pub const ALL: [Look; 8] = [Look::Splats, Look::Ellipses, Look::Centres, Look::Opacity, Look::Size, Look::Density, Look::Distance, Look::Problems];

    pub fn name(self) -> &'static str {
        match self {
            Look::Splats => "Splats",
            Look::Ellipses => "Outlines",
            Look::Centres => "Centres",
            Look::Opacity => "Opacity",
            Look::Size => "Size",
            Look::Density => "Crowding",
            Look::Distance => "Distance",
            Look::Problems => "Problems",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Look::Splats => "The scene as it was made",
            Look::Ellipses => "Each splat's outline, to see their shapes and how they overlap",
            Look::Centres => "Each splat's centre as a dot",
            Look::Opacity => "How see-through each splat is: dark is nearly invisible, bright is solid",
            Look::Size => "How big each splat is next to the scene's typical splat: dark is small, bright is big",
            Look::Density => "How many splats are close by: dark is alone, bright is crowded",
            Look::Distance => "How far each splat is from the middle of the scene: dark is near, bright is far",
            Look::Problems => "The splats the checks found, in colour; the rest in grey",
        }
    }

    /// The viewer's drawing of each splat: 0 as a splat, 1 its outline, 2 its centre.
    pub fn display(self) -> u8 {
        match self {
            Look::Ellipses => 1,
            Look::Centres => 2,
            _ => 0,
        }
    }

    /// Whether the splats get colours of their own in this look.
    pub fn recolours(self) -> bool {
        matches!(self, Look::Opacity | Look::Size | Look::Density | Look::Distance | Look::Problems)
    }

    /// The ends of the colour scale, for the legend.
    pub fn scale_ends(self) -> Option<(&'static str, &'static str)> {
        match self {
            Look::Opacity => Some(("see-through", "solid")),
            Look::Size => Some(("small", "big")),
            Look::Density => Some(("alone", "crowded")),
            Look::Distance => Some(("near", "far")),
            _ => None,
        }
    }
}

/// What a check found, one bit each, per splat.
pub const FAINT: u8 = 1;
pub const HUGE: u8 = 2;
pub const FAR: u8 = 4;
pub const ALONE: u8 = 8;

/// Each check: its bit, its colour in the Problems look, and its name.
pub const PROBLEMS: [(u8, [u8; 3], &str); 4] = [(FAR, [255, 70, 70], "Far out"), (ALONE, [255, 210, 40], "Floaters"), (HUGE, [255, 130, 20], "Oversized"), (FAINT, [220, 90, 255], "Nearly invisible")];

/// The per-splat measures behind the checks and the false-colour looks.
#[derive(Debug, Default)]
pub struct Analysis {
    /// [`FAINT`], [`HUGE`], [`FAR`] and [`ALONE`] bits.
    pub flags: Vec<u8>,
    /// Splats in the same small cell of space (cells about twice the splats' typical spacing
    /// in the main body).
    pub crowd: Vec<u16>,
    /// The typical (median) size of a splat.
    pub typical: f32,
}

/// Splats with this many or fewer in their cell and the 26 around it (themselves included)
/// count as floaters.
const ALONE_MAX: u32 = 2;
/// Cells across the main body's radius at most.
const CELLS_MAX: f32 = 256.0;

/// Work out the checks and measures for every splat.
pub fn analyse(scene: &Scene) -> Analysis {
    let n = scene.pos.len();
    let mut a = Analysis { flags: vec![0; n], crowd: vec![0; n], typical: 0.0 };
    if scene.kind == SceneKind::PointCloud || n == 0 {
        return a;
    }
    let (mid, r) = super::framing(scene);
    let mut sizes: Vec<f32> = scene.scale.iter().step_by((scene.scale.len() / 100_000).max(1)).map(|s| s[0].max(s[1]).max(s[2])).collect();
    sizes.sort_unstable_by(f32::total_cmp);
    a.typical = sizes.get(sizes.len() / 2).copied().unwrap_or(0.0);
    let far2 = (10.0 * r) * (10.0 * r);
    // Floaters are looked for near the subject only: farther out, captures thin out anyway.
    let near2 = (3.0 * r) * (3.0 * r);
    // The main body (a cube about 2 radii across) holds most splats, so their typical
    // spacing is 2r / cbrt(n); a cell is twice that.
    let cells_per_r = ((n as f32).cbrt() / 4.0).clamp(1.0, CELLS_MAX);
    let cell = r / cells_per_r;
    // Cells within 4 radii of the middle, keyed by their three indices.
    let reach = 4.0 * cells_per_r;
    let key = |p: [f32; 3]| -> Option<u64> {
        let mut k = 0u64;
        for (ax, v) in p.iter().enumerate() {
            let c = ((v - mid[ax]) / cell).floor();
            if !(-reach..reach).contains(&c) {
                return None;
            }
            k |= ((c + reach) as u64) << (ax * 20);
        }
        Some(k)
    };
    let mut cells: std::collections::HashMap<u64, u32> = std::collections::HashMap::with_capacity(n / 4);
    let keys: Vec<Option<u64>> = scene.pos.iter().map(|&p| key(p)).collect();
    for k in keys.iter().flatten() {
        *cells.entry(*k).or_default() += 1;
    }
    let around = |k: u64| -> u32 {
        let mut total = 0;
        for dz in [-1i64, 0, 1] {
            for dy in [-1i64, 0, 1] {
                for dx in [-1i64, 0, 1] {
                    let shift = dx + (dy << 20) + (dz << 40);
                    total += cells.get(&((k as i64 + shift) as u64)).copied().unwrap_or(0);
                }
            }
        }
        total
    };
    for i in 0..n {
        let mut f = 0;
        if scene.color[i][3] < 5 {
            f |= FAINT;
        }
        let p = scene.pos[i];
        let d2 = (p[0] - mid[0]).powi(2) + (p[1] - mid[1]).powi(2) + (p[2] - mid[2]).powi(2);
        let own = keys[i].map_or(1, |k| cells[&k]);
        a.crowd[i] = own.min(u16::MAX as u32) as u16;
        if d2 > far2 {
            f |= FAR;
        } else {
            // Far-out splats are big on purpose (sky, distant walls), so only those in the
            // main body count as oversized or alone.
            if let Some(s) = scene.scale.get(i) {
                if a.typical > 0.0 && s[0].max(s[1]).max(s[2]) > 200.0 * a.typical {
                    f |= HUGE;
                }
            }
            // Visible, and nothing else nearby: a floater.
            if d2 < near2 && own <= ALONE_MAX && f & FAINT == 0 && keys[i].is_some_and(|k| around(k) <= ALONE_MAX) {
                f |= ALONE;
            }
        }
        a.flags[i] = f;
    }
    a
}

/// How many splats each check found.
pub fn count(a: &Analysis, bit: u8) -> usize {
    a.flags.iter().filter(|f| *f & bit != 0).count()
}

/// A colour on the scale from dark blue through teal and green to yellow, for `t` from 0 to 1.
pub fn ramp(t: f32) -> [u8; 3] {
    const STOPS: [[f32; 3]; 5] = [[40.0, 30.0, 110.0], [40.0, 90.0, 160.0], [30.0, 150.0, 140.0], [110.0, 200.0, 80.0], [250.0, 230.0, 40.0]];
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 } * 4.0;
    let i = (t.floor() as usize).min(3);
    let f = t - i as f32;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    [(a[0] + (b[0] - a[0]) * f) as u8, (a[1] + (b[1] - a[1]) * f) as u8, (a[2] + (b[2] - a[2]) * f) as u8]
}

/// The splats' colours in a false-colour look (`None` for looks that keep the scene's own).
/// Opacity is raised so faint splats show; in Problems, the rest are grey and see-through.
pub fn paint(scene: &Scene, a: &Analysis, look: Look) -> Option<Vec<[u8; 4]>> {
    if !look.recolours() || a.flags.len() != scene.len() {
        return None;
    }
    let (mid, r) = super::framing(scene);
    let max_crowd = {
        let mut c: Vec<u16> = a.crowd.iter().step_by((a.crowd.len() / 100_000).max(1)).copied().collect();
        c.sort_unstable();
        c.get(c.len() * 99 / 100).copied().unwrap_or(1).max(2) as f32
    };
    let solid = |rgb: [u8; 3], alpha: u8| [rgb[0], rgb[1], rgb[2], alpha];
    Some(
        (0..scene.len())
            .map(|i| {
                let alpha = scene.color[i][3];
                let shown = alpha.max(150);
                match look {
                    Look::Opacity => solid(ramp(alpha as f32 / 255.0), 150),
                    Look::Size => {
                        let s = scene.scale.get(i).map_or(0.0, |s| s[0].max(s[1]).max(s[2]));
                        // A thousandth of the typical size to a thousand times it.
                        let t = ((s / a.typical.max(1e-12)).log10() + 3.0) / 6.0;
                        solid(ramp(t), shown)
                    }
                    Look::Density => solid(ramp((a.crowd[i] as f32).ln() / max_crowd.ln()), shown),
                    Look::Distance => {
                        let p = scene.pos[i];
                        let d = ((p[0] - mid[0]).powi(2) + (p[1] - mid[1]).powi(2) + (p[2] - mid[2]).powi(2)).sqrt();
                        // Out to the 10 radii past which splats count as far.
                        solid(ramp((1.0 + d / r).ln() / 11f32.ln()), shown)
                    }
                    _ => match PROBLEMS.iter().find(|(bit, _, _)| a.flags[i] & bit != 0) {
                        // Far-out splats are often big sky splats round the camera: tinted, so
                        // they don't hide everything else.
                        Some((FAR, rgb, _)) => solid(*rgb, alpha.min(35)),
                        Some((_, rgb, _)) => solid(*rgb, 255),
                        None => {
                            let c = scene.color[i];
                            let g = ((c[0] as u32 + c[1] as u32 + c[2] as u32) / 3 / 2 + 40) as u8;
                            [g, g, g, alpha / 4]
                        }
                    },
                }
            })
            .collect(),
    )
}
