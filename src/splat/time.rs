//! Scenes that change over time.
//!
//! Space-Time Gaussians "Lite" (STG Lite) PLYs: each splat has a moment it's centred on
//! (`trbf_center`), how long it lasts around it (`trbf_scale`, a log), a cubic path
//! (`motion_0`–`motion_8`: velocity, then the second and third terms) and a turn rate
//! (`omega_0`–`omega_3`, w x y z). At time t, with d = t − centre:
//!
//! * position = centre position + m₀d + m₁d² + m₂d³
//! * rotation = normalise(rotation + ω·d)
//! * opacity = sigmoid(opacity) · exp(−(d / e^scale)²)
//!
//! and the colour is plain RGB (`f_dc_*`), not spherical harmonics. This is the
//! representation described in "Spacetime Gaussian Feature Splatting for Real-Time Dynamic
//! View Synthesis" (Li et al., CVPR 2024), as read by abist-co-ltd's wgpu-gs-viewer (MIT).
//! No code from the paper's own implementation is used.

use super::Scene;

/// What each kept splat does over time, alongside the scene's own arrays (same order).
#[derive(Debug, Default)]
pub struct Timed {
    /// Position at its centre moment.
    pub pos: Vec<[f32; 3]>,
    pub motion: Vec<[f32; 9]>,
    /// Rotation as stored (w x y z, not normalised).
    pub rot: Vec<[f32; 4]>,
    pub omega: Vec<[f32; 4]>,
    pub centre: Vec<f32>,
    /// Log of how long it lasts.
    pub span: Vec<f32>,
    /// Opacity before the sigmoid.
    pub logit: Vec<f32>,
    /// The times the scene covers, start and end.
    pub from: f32,
    pub to: f32,
}

/// The properties an STG Lite PLY has.
pub const STG_LITE: &[&str] = &[
    "trbf_center",
    "trbf_scale",
    "motion_0",
    "motion_1",
    "motion_2",
    "motion_3",
    "motion_4",
    "motion_5",
    "motion_6",
    "motion_7",
    "motion_8",
    "omega_0",
    "omega_1",
    "omega_2",
    "omega_3",
];

impl Timed {
    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// Works out the times covered from the splats' centre moments: 0 to 1 as the trainer
    /// writes them, or wider when the centres say so.
    pub fn settle(&mut self) {
        let mut c: Vec<f32> = self.centre.iter().copied().filter(|v| v.is_finite()).collect();
        if c.is_empty() {
            (self.from, self.to) = (0.0, 1.0);
            return;
        }
        c.sort_unstable_by(f32::total_cmp);
        let lo = c[c.len() / 100];
        let hi = c[(c.len() * 99 / 100).min(c.len() - 1)];
        (self.from, self.to) = (lo.min(0.0), hi.max(1.0));
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// One splat at time `t`: position, rotation (x y z w) and opacity (0–1).
pub fn splat_at(tm: &Timed, i: usize, t: f32) -> ([f32; 3], [f32; 4], f32) {
    let d = t - tm.centre[i];
    let (d2, d3) = (d * d, d * d * d);
    let (p, m) = (tm.pos[i], tm.motion[i]);
    let pos = [p[0] + m[0] * d + m[3] * d2 + m[6] * d3, p[1] + m[1] * d + m[4] * d2 + m[7] * d3, p[2] + m[2] * d + m[5] * d2 + m[8] * d3];
    let (r, w) = (tm.rot[i], tm.omega[i]);
    let q = [r[0] + w[0] * d, r[1] + w[1] * d, r[2] + w[2] * d, r[3] + w[3] * d];
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let rot = if n > 1e-12 { [q[1] / n, q[2] / n, q[3] / n, q[0] / n] } else { [0.0, 0.0, 0.0, 1.0] };
    let life = tm.span[i].exp().max(1e-6);
    let alpha = sigmoid(tm.logit[i]) * (-(d / life).powi(2)).exp();
    (pos, rot, alpha)
}

impl Scene {
    /// The scene as it is at time `t`, for drawing. Its details and checks are the whole
    /// scene's, kept from this one.
    pub fn at(&self, t: f32) -> Option<Scene> {
        let tm = self.timed.as_ref()?;
        if tm.len() != self.len() {
            return None;
        }
        let mut s = Scene::new(self.kind, self.format.clone(), self.frame, self.total);
        s.pos = Vec::with_capacity(self.len());
        s.rot = Vec::with_capacity(self.len());
        s.color = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            let (p, r, a) = splat_at(tm, i, t);
            s.pos.push(p);
            s.rot.push(r);
            let c = self.color[i];
            s.color.push([c[0], c[1], c[2], (a * 255.0).round().clamp(0.0, 255.0) as u8]);
        }
        s.scale = self.scale.clone();
        s.sh_degree = self.sh_degree;
        s.sh = self.sh.clone();
        s.antialiased = self.antialiased;
        s.bad = self.bad;
        s.notes = self.notes.clone();
        s.said = self.said.clone();
        s.facts = self.facts.clone();
        s.analysis = self.analysis.clone();
        s.timed = self.timed.clone();
        Some(s)
    }
}
