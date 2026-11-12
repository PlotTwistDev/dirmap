//! The treemap camera: which part of the laid-out map is on screen, and every way it moves.
//!
//! All navigation (mouse wheel, trackpad pinch, drag to pan, double-click, right-click, zoom to
//! selection, Back/Forward, reset) goes through this one type, so there are no competing zoom
//! implementations. The layout is fixed in "world" units; the camera is just the world rectangle
//! shown in the viewport. Everything is f64 so deep zooms stay exact.
//!
//! Wheel zoom keeps the world point under the pointer fixed on every frame: each frame applies a
//! scale step about the pointer, so the anchor is exact by construction rather than approximately
//! right at the end of an animation. Animation timing uses elapsed seconds, never frame counts.

use crate::treemap::DRect;

/// How navigation feels; from the Settings window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feel {
    /// Animate zooming. Off: wheel steps and jumps apply at once.
    pub smooth: bool,
    /// Multiplies the zoom per wheel notch / trackpad distance. 1 = default.
    pub sensitivity: f64,
    /// Multiplies animation speed. 1 = default, 2 = twice as fast.
    pub speed: f64,
}

impl Default for Feel {
    fn default() -> Self {
        Self { smooth: true, sensitivity: 1.0, speed: 1.0 }
    }
}

/// Zoom (in natural-log units of scale) for one wheel notch at sensitivity 1: about ×1.3.
pub const NOTCH: f64 = 0.27;
/// Wheel distance in points that counts as one notch (trackpads report points).
pub const NOTCH_POINTS: f64 = 50.0;
/// Deepest zoom relative to the whole map.
pub const MAX_ZOOM: f64 = 1e7;

/// A framed animation from one view to another (zoom to selection, Back/Forward, reset...).
#[derive(Clone, Copy, Debug)]
struct Flight {
    from: DRect,
    to: DRect,
    t: f64,
    secs: f64,
}

#[derive(Clone, Debug)]
pub struct Camera {
    /// The whole laid-out map; the view never leaves it and never shows more than it.
    world: DRect,
    /// The world rectangle currently filling the viewport.
    view: DRect,
    /// Viewport size in points.
    vp: (f64, f64),
    /// Wheel zoom still to apply, as log(scale): the gap between where the zoom is heading and
    /// where it is now, and the rate it's closing at (critically damped spring).
    zoom_gap: f64,
    zoom_vel: f64,
    /// Screen point (relative to the viewport) the wheel zoom is anchored to.
    anchor: (f64, f64),
    flight: Option<Flight>,
    /// Pan inertia in world units per second, after a drag is released.
    glide: (f64, f64),
}

impl Camera {
    pub fn new(world: DRect, vp: (f64, f64)) -> Self {
        Self { world, view: world, vp, zoom_gap: 0.0, zoom_vel: 0.0, anchor: (0.0, 0.0), flight: None, glide: (0.0, 0.0) }
    }

    pub fn world(&self) -> DRect {
        self.world
    }

    pub fn view(&self) -> DRect {
        self.view
    }

    pub fn viewport(&self) -> (f64, f64) {
        self.vp
    }

    /// Screen points per world unit (horizontal).
    pub fn scale(&self) -> f64 {
        self.vp.0 / self.view.w()
    }

    /// How far in we are: 1 = whole map.
    pub fn zoom(&self) -> f64 {
        self.world.w() / self.view.w()
    }

    pub fn is_full(&self) -> bool {
        self.zoom() < 1.0005 && !self.is_moving()
    }

    pub fn is_moving(&self) -> bool {
        self.flight.is_some() || self.zoom_gap != 0.0 || self.zoom_vel != 0.0 || self.glide != (0.0, 0.0)
    }

    /// Replace the world (the map was laid out again, e.g. after a resize), keeping the same
    /// part of it on screen.
    pub fn rebase(&mut self, world: DRect, vp: (f64, f64)) {
        let map = |r: DRect| relative(&self.world, &r);
        let view = map(self.view);
        let flight = self.flight.map(|f| Flight { from: map(f.from), to: map(f.to), ..f });
        self.world = world;
        self.vp = vp;
        self.view = absolute(&world, &view);
        self.flight = flight.map(|f| Flight { from: absolute(&world, &f.from), to: absolute(&world, &f.to), ..f });
        if self.flight.is_none() {
            self.fit_aspect();
        }
    }

    /// Jump straight to `view` (no animation), stopping any motion.
    pub fn set_view(&mut self, view: DRect) {
        self.stop();
        self.view = view;
    }

    pub fn stop(&mut self) {
        self.flight = None;
        self.zoom_gap = 0.0;
        self.zoom_vel = 0.0;
        self.glide = (0.0, 0.0);
    }

    #[cfg(test)]
    pub fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        to_screen(&self.view, self.vp, x, y)
    }

    pub fn to_world(&self, sx: f64, sy: f64) -> (f64, f64) {
        to_world(&self.view, self.vp, sx, sy)
    }

    /// Settle any framed flight where it is now, so input continues from what's on screen.
    fn land(&mut self) {
        if self.flight.take().is_some() {
            self.fit_aspect();
        }
    }

    /// Zoom by `amount` (natural log of the scale factor; positive zooms in) about the screen
    /// point `at`. With `feel.smooth` this retargets the running zoom rather than starting
    /// another one, so fast wheel spins accumulate into one motion.
    pub fn zoom_by(&mut self, amount: f64, at: (f64, f64), feel: &Feel) {
        self.land();
        self.glide = (0.0, 0.0);
        self.anchor = at;
        let now = self.zoom().ln();
        let target = (now + self.zoom_gap + amount).clamp(0.0, MAX_ZOOM.ln());
        if feel.smooth {
            self.zoom_gap = target - now;
        } else {
            self.zoom_gap = 0.0;
            self.zoom_vel = 0.0;
            self.scale_about(target - now, at);
        }
    }

    /// Move the anchor of a zoom in progress (the pointer moved while zooming).
    pub fn follow_pointer(&mut self, at: (f64, f64)) {
        self.anchor = at;
    }

    /// Drag: move the map by a screen distance.
    pub fn pan_by(&mut self, dx: f64, dy: f64) {
        self.land();
        self.glide = (0.0, 0.0);
        let (sx, sy) = (self.vp.0 / self.view.w(), self.vp.1 / self.view.h());
        self.view = offset(&self.view, -dx / sx, -dy / sy);
        self.clamp();
    }

    /// Drag released with this screen velocity (points per second): coast to a stop.
    pub fn release(&mut self, vx: f64, vy: f64, feel: &Feel) {
        if !feel.smooth {
            return;
        }
        let speed = (vx * vx + vy * vy).sqrt();
        if speed < 60.0 {
            return;
        }
        let (sx, sy) = (self.vp.0 / self.view.w(), self.vp.1 / self.view.h());
        self.glide = (-vx / sx, -vy / sy);
    }

    /// Animate to show `target` (a world rectangle) as large as fits, centred, with `pad` of the
    /// viewport left around it on each side.
    pub fn frame(&mut self, target: &DRect, pad: f64, feel: &Feel) {
        let to = self.framing(target, pad);
        self.fly_to(to, feel);
    }

    /// The view that `frame` would end at.
    pub fn framing(&self, target: &DRect, pad: f64) -> DRect {
        let (vw, vh) = self.vp;
        let avail = ((1.0 - 2.0 * pad) * vw, (1.0 - 2.0 * pad) * vh);
        // World units per point so the target fits.
        let upp = (target.w() / avail.0).max(target.h() / avail.1).max(self.world.w() / vw / MAX_ZOOM);
        let (cx, cy) = target.center();
        let (w, h) = (vw * upp, vh * upp);
        clamp_view(&self.world, DRect::new(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0))
    }

    /// Animate to exactly `to`.
    pub fn fly_to(&mut self, to: DRect, feel: &Feel) {
        self.zoom_gap = 0.0;
        self.zoom_vel = 0.0;
        self.glide = (0.0, 0.0);
        if !feel.smooth {
            self.flight = None;
            self.view = to;
            return;
        }
        let from = self.view;
        let secs = flight_secs(&from, &to, feel.speed);
        self.flight = Some(Flight { from, to, t: 0.0, secs });
    }

    /// Whole map.
    pub fn reset(&mut self, feel: &Feel) {
        let to = self.world;
        self.fly_to(to, feel);
    }

    /// Advance animations by `dt` seconds. Returns true while something is still moving.
    pub fn update(&mut self, dt: f64, feel: &Feel) -> bool {
        let dt = dt.clamp(0.0, 0.25);
        if let Some(f) = &mut self.flight {
            f.t = (f.t + dt / f.secs).min(1.0);
            self.view = interpolate(&f.from, &f.to, ease(f.t));
            if f.t >= 1.0 {
                self.view = f.to;
                self.flight = None;
            }
        }
        if self.zoom_gap != 0.0 || self.zoom_vel != 0.0 {
            // Critically damped spring on log(scale): starts gently, never overshoots, and the
            // exact solution makes it independent of the frame rate.
            let w = 22.0 * feel.speed;
            let e0 = -self.zoom_gap;
            let k = (-w * dt).exp();
            let tmp = (self.zoom_vel + w * e0) * dt;
            let mut v = (self.zoom_vel - w * tmp) * k;
            let mut e = (e0 + tmp) * k;
            if e.abs() < 1e-4 && v.abs() < 1e-3 {
                e = 0.0;
                v = 0.0;
            }
            let step = e - e0;
            self.zoom_gap = -e;
            self.zoom_vel = v;
            let anchor = self.anchor;
            let applied = self.scale_about(step, anchor);
            if (applied - step).abs() > 1e-9 {
                // Hit the zoom limit: nothing more to do in that direction.
                self.zoom_gap = 0.0;
                self.zoom_vel = 0.0;
            }
        }
        if self.glide != (0.0, 0.0) {
            let (gx, gy) = self.glide;
            let before = self.view;
            self.view = offset(&self.view, gx * dt, gy * dt);
            self.clamp();
            let decay = (-5.0 * feel.speed * dt).exp();
            self.glide = (gx * decay, gy * decay);
            let moved = ((self.view.x0 - before.x0).abs() + (self.view.y0 - before.y0).abs()) * self.scale();
            let speed = (self.glide.0.hypot(self.glide.1)) * self.scale();
            if speed < 8.0 || (moved < 1e-6 && dt > 0.0) {
                self.glide = (0.0, 0.0);
            }
        }
        self.is_moving()
    }

    /// Scale by exp(`amount`) about screen point `at`, keeping the world point under it fixed
    /// (unless the view has to be pulled back inside the map when zooming out past the edge).
    /// Returns the amount actually applied after the zoom limits.
    fn scale_about(&mut self, amount: f64, at: (f64, f64)) -> f64 {
        let now = self.zoom().ln();
        let target = (now + amount).clamp(0.0, MAX_ZOOM.ln());
        let f = (target - now).exp();
        let (ax, ay) = self.to_world(at.0, at.1);
        let (w, h) = (self.view.w() / f, self.view.h() / f);
        let (u, v) = (at.0 / self.vp.0, at.1 / self.vp.1);
        self.view = DRect::new(ax - u * w, ay - v * h, ax - u * w + w, ay - v * h + h);
        if target <= 1e-9 {
            // Fully out: snap exactly to the whole map so rounding can't leave a sliver.
            self.view = self.world;
        }
        self.clamp();
        target - now
    }

    /// Keep the view inside the map.
    fn clamp(&mut self) {
        if self.flight.is_none() {
            self.view = clamp_view(&self.world, self.view);
        }
    }

    /// Make the view's aspect ratio match the viewport (after a stretched drill transition or a
    /// resize), keeping its centre and horizontal scale.
    fn fit_aspect(&mut self) {
        let (vw, vh) = self.vp;
        if vw <= 0.0 || vh <= 0.0 {
            return;
        }
        let upp = (self.view.w() / vw).max(self.view.h() / vh).min(self.world.w() / vw).min(self.world.h() / vh);
        let (cx, cy) = self.view.center();
        let (w, h) = (vw * upp, vh * upp);
        self.view = clamp_view(&self.world, DRect::new(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0));
    }
}

/// Move `view` back inside `world` (and shrink it to fit if it is bigger).
pub fn clamp_view(world: &DRect, view: DRect) -> DRect {
    let axis = |a0: f64, a1: f64, lo: f64, hi: f64| {
        let len = (a1 - a0).min(hi - lo);
        let start = a0.clamp(lo, hi - len);
        (start, start + len)
    };
    let (x0, x1) = axis(view.x0, view.x1, world.x0, world.x1);
    let (y0, y1) = axis(view.y0, view.y1, world.y0, world.y1);
    DRect::new(x0, y0, x1, y1)
}

fn offset(r: &DRect, dx: f64, dy: f64) -> DRect {
    DRect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy)
}

/// World point to screen (relative to the viewport's top-left) for a view filling `vp`.
#[cfg(test)]
pub fn to_screen(view: &DRect, vp: (f64, f64), x: f64, y: f64) -> (f64, f64) {
    ((x - view.x0) * vp.0 / view.w(), (y - view.y0) * vp.1 / view.h())
}

pub fn to_world(view: &DRect, vp: (f64, f64), sx: f64, sy: f64) -> (f64, f64) {
    (view.x0 + sx * view.w() / vp.0, view.y0 + sy * view.h() / vp.1)
}

/// `r` as fractions of `of` (0..1 when inside it).
pub fn relative(of: &DRect, r: &DRect) -> DRect {
    let (w, h) = (of.w(), of.h());
    DRect::new((r.x0 - of.x0) / w, (r.y0 - of.y0) / h, (r.x1 - of.x0) / w, (r.y1 - of.y0) / h)
}

/// Inverse of `relative`.
pub fn absolute(of: &DRect, r: &DRect) -> DRect {
    let (w, h) = (of.w(), of.h());
    DRect::new(of.x0 + r.x0 * w, of.y0 + r.y0 * h, of.x0 + r.x1 * w, of.y0 + r.y1 * h)
}

/// Ease-in-out (cubic): gentle start, gentle landing.
pub fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Length of a framed animation: longer for bigger changes of scale, scaled by the speed setting.
fn flight_secs(from: &DRect, to: &DRect, speed: f64) -> f64 {
    let zoom = (to.w() / from.w()).ln().abs().max((to.h() / from.h()).ln().abs());
    let (fx, fy) = from.center();
    let (tx, ty) = to.center();
    let pan = ((tx - fx) / from.w().min(to.w())).hypot((ty - fy) / from.h().min(to.h()));
    let secs = 0.28 + 0.045 * zoom + 0.04 * pan.min(4.0);
    secs.min(0.85) / speed.max(0.1)
}

/// Point `t` (0..1) of the way from view `a` to view `b`. Each axis zooms about the point that
/// stays put between the two views, with the scale changing geometrically, so the motion reads
/// as one steady zoom towards (or away from) the target rather than a pan plus a zoom. Pure pans
/// interpolate linearly.
pub fn interpolate(a: &DRect, b: &DRect, t: f64) -> DRect {
    let axis = |a0: f64, a1: f64, b0: f64, b1: f64| {
        let (wa, wb) = (a1 - a0, b1 - b0);
        let ratio = (wb / wa).ln();
        if ratio.abs() < 1e-3 {
            let s = a0 + (b0 - a0) * t;
            let w = wa + (wb - wa) * t;
            return (s, s + w);
        }
        let w = wa * (ratio * t).exp();
        // Fixed point in view fractions: a0 + u*wa == b0 + u*wb.
        let u = (b0 - a0) / (wa - wb);
        let fixed = a0 + u * wa;
        let s = fixed - u * w;
        (s, s + w)
    };
    let (x0, x1) = axis(a.x0, a.x1, b.x0, b.x1);
    let (y0, y1) = axis(a.y0, a.y1, b.y0, b.y1);
    DRect::new(x0, y0, x1, y1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera::new(DRect::from_size(1200.0, 800.0), (1200.0, 800.0))
    }

    fn run(c: &mut Camera, secs: f64, dt: f64, feel: &Feel) {
        let mut t = 0.0;
        while t < secs {
            c.update(dt, feel);
            t += dt;
        }
    }

    /// The world point under the pointer stays under the pointer on every frame of a smooth zoom.
    #[test]
    fn wheel_zoom_keeps_point_under_cursor_every_frame() {
        let feel = Feel::default();
        let mut c = cam();
        let at = (913.7, 211.3);
        let anchor = c.to_world(at.0, at.1);
        for _ in 0..40 {
            c.zoom_by(NOTCH, at, &feel);
            for _ in 0..3 {
                c.update(1.0 / 144.0, &feel);
                let (sx, sy) = c.to_screen(anchor.0, anchor.1);
                assert!((sx - at.0).abs() < 1e-6 && (sy - at.1).abs() < 1e-6, "drifted to {sx},{sy}");
            }
        }
        run(&mut c, 2.0, 1.0 / 60.0, &feel);
        assert!(c.zoom() > 1e4, "zoom {}", c.zoom());
        let (sx, sy) = c.to_screen(anchor.0, anchor.1);
        assert!((sx - at.0).abs() < 1e-6 && (sy - at.1).abs() < 1e-6);
    }

    /// Point at a tiny tile and zoom until it fills the screen: it never leaves the pointer.
    #[test]
    fn tiny_tile_stays_under_cursor_while_enlarged() {
        let feel = Feel::default();
        let mut c = cam();
        let tile = DRect::new(700.001, 300.002, 700.0013, 300.0022);
        let (wx, wy) = tile.center();
        let at = c.to_screen(wx, wy);
        while (tile.w() * c.scale()) < 200.0 {
            c.zoom_by(NOTCH, at, &feel);
            c.update(1.0 / 60.0, &feel);
            let (sx, sy) = c.to_screen(wx, wy);
            assert!((sx - at.0).abs() < 1e-5 && (sy - at.1).abs() < 1e-5);
            assert!(c.view().contains(wx, wy));
        }
    }

    /// Rapid in/out spins with a stationary pointer: one smooth motion, anchor fixed, no drift,
    /// and ending where the sum of the notches says.
    #[test]
    fn rapid_in_and_out_is_stable() {
        let feel = Feel::default();
        let mut c = cam();
        let at = (300.0, 500.0);
        c.zoom_by(NOTCH * 10.0, at, &feel);
        run(&mut c, 1.0, 1.0 / 60.0, &feel);
        let anchor = c.to_world(at.0, at.1);
        let start = c.zoom();
        let mut prev_gap = f64::NAN;
        for k in 0..60 {
            let dir = if k % 3 == 0 { -1.0 } else { 1.0 };
            c.zoom_by(dir * NOTCH, at, &feel);
            c.update(1.0 / 120.0, &feel);
            let (sx, sy) = c.to_screen(anchor.0, anchor.1);
            assert!((sx - at.0).abs() < 1e-6 && (sy - at.1).abs() < 1e-6);
            prev_gap = c.zoom_gap;
        }
        assert!(prev_gap.is_finite());
        run(&mut c, 2.0, 1.0 / 120.0, &feel);
        let expected = start * (NOTCH * 20.0).exp();
        assert!((c.zoom() / expected - 1.0).abs() < 1e-3, "{} vs {expected}", c.zoom());
        assert!(!c.is_moving());
    }

    /// Zooming while the pointer moves: at every frame the point under the *current* pointer is
    /// the one that stays still.
    #[test]
    fn zoom_while_moving_pointer() {
        let feel = Feel::default();
        let mut c = cam();
        c.zoom_by(NOTCH * 6.0, (600.0, 400.0), &feel);
        for k in 0..120 {
            let at = (200.0 + k as f64 * 5.0, 300.0 + (k as f64 * 0.1).sin() * 100.0);
            c.follow_pointer(at);
            let w = c.to_world(at.0, at.1);
            c.update(1.0 / 60.0, &feel);
            let (sx, sy) = c.to_screen(w.0, w.1);
            assert!((sx - at.0).abs() < 1e-6 && (sy - at.1).abs() < 1e-6);
            if k % 10 == 0 {
                c.zoom_by(NOTCH, at, &feel);
            }
        }
    }

    /// Zooming out symmetric with zooming in, and back to exactly the whole map.
    #[test]
    fn zoom_out_returns_to_whole_map() {
        let feel = Feel::default();
        let mut c = cam();
        let at = (1100.0, 90.0);
        for _ in 0..12 {
            c.zoom_by(NOTCH, at, &feel);
        }
        run(&mut c, 1.5, 1.0 / 60.0, &feel);
        let anchor = c.to_world(at.0, at.1);
        for _ in 0..6 {
            c.zoom_by(-NOTCH, at, &feel);
        }
        run(&mut c, 1.5, 1.0 / 60.0, &feel);
        assert!((c.zoom() - (NOTCH * 6.0).exp()).abs() < 1e-2);
        let (sx, sy) = c.to_screen(anchor.0, anchor.1);
        assert!((sx - at.0).abs() < 1e-6 && (sy - at.1).abs() < 1e-6, "zoom-out anchor moved to {sx},{sy}");
        for _ in 0..30 {
            c.zoom_by(-NOTCH, at, &feel);
        }
        run(&mut c, 2.0, 1.0 / 60.0, &feel);
        assert_eq!(c.view(), c.world());
        assert!(c.is_full());
    }

    /// The same input gives the same motion at 30, 60 and 240 frames per second.
    #[test]
    fn timing_is_frame_rate_independent() {
        let feel = Feel::default();
        let at = (400.0, 300.0);
        let mut views = Vec::new();
        for fps in [30.0, 60.0, 240.0] {
            let mut c = cam();
            c.zoom_by(NOTCH * 5.0, at, &feel);
            run(&mut c, 0.1 - 1e-9, 1.0 / fps, &feel);
            views.push(c.zoom());
        }
        for z in &views {
            assert!((z / views[0] - 1.0).abs() < 1e-9, "{views:?}");
        }
        assert!(views[0] > 1.0 && views[0] < (NOTCH * 5.0).exp(), "mid-animation zoom {views:?}");
    }

    #[test]
    fn instant_mode_applies_at_once() {
        let feel = Feel { smooth: false, ..Feel::default() };
        let mut c = cam();
        let at = (50.0, 700.0);
        let anchor = c.to_world(at.0, at.1);
        c.zoom_by(NOTCH * 3.0, at, &feel);
        assert!(!c.is_moving());
        assert!((c.zoom() - (NOTCH * 3.0).exp()).abs() < 1e-9);
        let (sx, sy) = c.to_screen(anchor.0, anchor.1);
        assert!((sx - at.0).abs() < 1e-9 && (sy - at.1).abs() < 1e-9);
    }

    #[test]
    fn framing_fits_target_with_padding_and_lands_exactly() {
        let feel = Feel::default();
        let mut c = cam();
        let target = DRect::new(100.0, 100.0, 104.0, 101.0);
        c.frame(&target, 0.1, &feel);
        let mut frames = 0;
        while c.update(1.0 / 60.0, &feel) {
            frames += 1;
            assert!(frames < 200);
        }
        let v = c.view();
        assert!(v.contains_rect(&target));
        let (sx0, _) = c.to_screen(target.x0, target.y0);
        let (sx1, _) = c.to_screen(target.x1, target.y1);
        assert!((sx0 - 120.0).abs() < 1e-6 && (sx1 - 1080.0).abs() < 1e-6, "{sx0} {sx1}");
        assert!((v.w() / v.h() - 1.5).abs() < 1e-9);
        c.reset(&feel);
        while c.update(1.0 / 60.0, &feel) {}
        assert_eq!(c.view(), c.world());
    }

    #[test]
    fn interpolation_hits_endpoints_and_zooms_about_fixed_point() {
        let a = DRect::new(0.0, 0.0, 1200.0, 800.0);
        let b = DRect::new(600.0, 200.0, 612.0, 208.0);
        assert_eq!(interpolate(&a, &b, 0.0), a);
        let e = interpolate(&a, &b, 1.0);
        assert!((e.x0 - b.x0).abs() < 1e-9 && (e.y1 - b.y1).abs() < 1e-9);
        // The fixed point is at the same screen fraction all the way.
        let u = (b.x0 - a.x0) / (a.w() - b.w());
        let fixed = a.x0 + u * a.w();
        for k in 1..10 {
            let m = interpolate(&a, &b, k as f64 / 10.0);
            assert!(((fixed - m.x0) / m.w() - u).abs() < 1e-9);
        }
        // Pure pans are linear.
        let c = DRect::new(100.0, 0.0, 1300.0, 800.0);
        let m = interpolate(&a, &c, 0.25);
        assert!((m.x0 - 25.0).abs() < 1e-9);
    }

    #[test]
    fn pan_is_clamped_and_glides_to_a_stop() {
        let feel = Feel::default();
        let mut c = cam();
        c.pan_by(100.0, 0.0);
        assert_eq!(c.view(), c.world(), "can't pan the whole map");
        c.zoom_by(NOTCH * 8.0, (600.0, 400.0), &feel);
        run(&mut c, 1.0, 1.0 / 60.0, &feel);
        let before = c.view();
        c.pan_by(-30.0, 12.0);
        let after = c.view();
        assert!(((after.x0 - before.x0) * c.scale() - 30.0).abs() < 1e-6);
        c.release(-900.0, 0.0, &feel);
        let mut frames = 0;
        while c.update(1.0 / 60.0, &feel) {
            frames += 1;
            assert!(frames < 400);
        }
        assert!(c.view().x0 > after.x0);
        assert!(c.world().contains_rect(&c.view()));
    }

    #[test]
    fn rebase_keeps_the_same_part_on_screen() {
        let feel = Feel::default();
        let mut c = cam();
        c.zoom_by(NOTCH * 4.0, (300.0, 300.0), &feel);
        run(&mut c, 1.0, 1.0 / 60.0, &feel);
        let rel = relative(&c.world(), &c.view());
        c.rebase(DRect::from_size(1500.0, 1000.0), (1500.0, 1000.0));
        let rel2 = relative(&c.world(), &c.view());
        assert!((rel.x0 - rel2.x0).abs() < 1e-9 && (rel.y1 - rel2.y1).abs() < 1e-9);
    }
}
