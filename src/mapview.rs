//! The treemap view: the camera over a laid-out folder ("scene"), animated drill-in/out between
//! folders, Back/Forward history, hover and light animation, and building the visible tiles on
//! background threads.
//!
//! Layout is separate from the camera. A scene's world is the folder laid out once to the size of
//! the map; the camera only changes which world rectangle fills the viewport. Each scene keeps a
//! "base" tile set of the whole folder at the detail of the full view (always available as an
//! underlay) and, when zoomed in, a "detail" set covering the view plus a margin at the current
//! detail. A detail set is only rebuilt once the view has moved outside it or the scale has
//! changed by more than about 1.6x; in between, the GPU just draws the existing tiles through a
//! new transform. Mouse movement never rebuilds anything.

use crate::camera::{self, Camera, Feel};
use crate::scan::Node;
use crate::tilegpu::{self, Backend, Draw, Instance, Uniforms};
use crate::treemap::{self, Cushion, DRect, Detail, HitCache, Memo};
use eframe::egui::{self, pos2, Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

/// Colour for a file: (fill, colour used to pick a readable label), or None when it's filtered
/// out. Gets the node and its path from the scan root. Runs on builder threads.
pub type Colorizer = Arc<dyn Fn(&Node, &[u32]) -> Option<(Color32, Color32)> + Send + Sync>;

/// For a folder whose children from index `from` on are too small to draw (they are shown as one
/// blended patch): a colour for that patch instead of the blend, e.g. to point out filter
/// matches that are too small to see on their own. Gets the folder, its path from the scan root
/// and `from`. Runs on builder threads.
pub type HiddenFill = Arc<dyn Fn(&Node, &[u32], usize) -> Option<Color32> + Send + Sync>;

/// What decides how tiles are built, apart from the camera.
#[derive(Clone)]
pub struct Paint {
    /// Changes whenever anything that affects the tiles changes.
    pub key: u64,
    pub colorize: Colorizer,
    pub hidden: Option<HiddenFill>,
    pub cushion: Cushion,
    pub background: Color32,
    /// Shading for the CPU fallback's images.
    pub look: Look,
}

/// Cushion lighting and the interactive effects; applied per frame, no rebuild needed.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub ambient: f32,
    pub diffuse: f32,
    pub gain: f32,
    pub sun: [f32; 3],
    pub grid: Option<Color32>,
    /// Pointer light on/off and its strength (1 = normal).
    pub lighting: bool,
    pub light_level: f32,
    pub bevel: bool,
}

/// A file tile big enough that it may get a label.
pub struct Label {
    pub rect: DRect,
    /// Path from the scene's folder.
    pub path: Vec<u32>,
    pub color: Color32,
}

/// Tiles built for one scene over `cover`, at `ppu` physical pixels per world unit.
pub struct TileSet {
    id: u64,
    paint_key: u64,
    /// World the set was laid out in (a resize lays the scene out again).
    world: DRect,
    cover: DRect,
    clip: DRect,
    origin: (f64, f64),
    ppu: f64,
    instances: Arc<Vec<Instance>>,
    labels: Vec<Label>,
    /// CPU fallback: the tiles rendered to an image of `cover`.
    image: Option<ColorImage>,
    texture: Option<TextureHandle>,
}

impl TileSet {
    fn local_rect(&self, r: &DRect) -> [f32; 4] {
        let r = r.intersect(&self.clip);
        let (ox, oy) = self.origin;
        let p = self.ppu;
        [((r.x0 - ox) * p) as f32, ((r.y0 - oy) * p) as f32, ((r.x1 - ox) * p) as f32, ((r.y1 - oy) * p) as f32]
    }

    fn unit(&self) -> f64 {
        1.0 / self.ppu
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Base,
    Detail,
}

struct Job {
    id: u64,
    cancel: Arc<AtomicBool>,
    paint_key: u64,
    world: DRect,
    cover: DRect,
    ppu: f64,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Built {
    scene: u64,
    kind: Kind,
    set: TileSet,
    millis: f64,
}

/// A folder laid out to fill the map.
struct Scene {
    id: u64,
    root: Vec<u32>,
    world: DRect,
    base: Option<TileSet>,
    detail: Option<TileSet>,
    base_job: Option<Job>,
    detail_job: Option<Job>,
    hits: HitCache,
    /// Width of the map in physical pixels, as of the last frame.
    px_w: f64,
}

impl Scene {
    fn new(id: u64, root: Vec<u32>, world: DRect) -> Self {
        Self { id, root, world, base: None, detail: None, base_job: None, detail_job: None, hits: HitCache::default(), px_w: 1.0 }
    }

    /// The tile sets to draw for `view`, underlay first, and the one whose detail level is on
    /// screen (for labels and hit-testing).
    fn layers(&self, view: &DRect, key: u64) -> (Vec<&TileSet>, Option<&TileSet>) {
        let vis = view.intersect(&self.world);
        let mut out = Vec::new();
        let base = self.base.as_ref();
        // Current detail level (physical pixels per world unit).
        let ppu = self.px_w / view.w();
        let detail = self.detail.as_ref().filter(|d| {
            let fresh = d.paint_key == key || base.is_none_or(|b| b.paint_key != key);
            // Much finer than what's on screen now (zoomed back out): its sub-pixel tiles would
            // look different from the base around it, so leave it out until it's replaced.
            let sharp_enough = ppu >= d.ppu * 0.6;
            fresh && sharp_enough && d.world == self.world && d.cover.intersects(&vis) && base.is_none_or(|b| d.ppu > b.ppu * 1.1)
        });
        let covered = detail.is_some_and(|d| d.cover.contains_rect(&vis));
        if let Some(b) = base {
            if !covered {
                out.push(b);
            }
        }
        if let Some(d) = detail {
            out.push(d);
        }
        let main = if covered { detail } else { base.or(detail) };
        (out, main)
    }
}

/// Drill animation between a folder and one of its subfolders. The shallow scene's camera moves
/// from `from` to `to`; the deep scene is drawn exactly over its own rectangle `rf` in the shallow
/// scene, so it appears to grow out of (or shrink back into) its place, cross-fading with it.
struct Transition {
    old: Scene,
    drill_in: bool,
    rf: DRect,
    from: DRect,
    to: DRect,
    t: f64,
    secs: f64,
}

/// What to do once a change of folder has finished animating.
#[derive(Clone, Debug)]
enum After {
    /// Show this part of the new folder (fractions of it).
    View(DRect),
    /// Frame this entry (path from the scan root).
    Frame(Vec<u32>),
}

/// A place in the Back/Forward history.
#[derive(Clone, Debug)]
struct Entry {
    root: Vec<u32>,
    /// Visible part, as fractions of the folder's layout.
    view: DRect,
}

impl Entry {
    fn same(&self, o: &Entry) -> bool {
        if self.root != o.root {
            return false;
        }
        let (a, b) = (&self.view, &o.view);
        let scale = (a.w() / b.w()).ln().abs();
        let (ax, ay) = a.center();
        let (bx, by) = b.center();
        scale < 0.08 && (ax - bx).abs() < 0.03 * a.w().max(b.w()) && (ay - by).abs() < 0.03 * a.h().max(b.h())
    }
}

/// How much the pointer light brightens a tile's surface, and the soft highlight on top.
const LIGHT_DIFFUSE: f32 = 0.36;
const LIGHT_SPECULAR: f32 = 0.06;

/// Padding around something being framed, as a fraction of the viewport.
const FRAME_PAD: f64 = 0.08;

pub struct MapView {
    pub backend: Backend,
    pub camera: Camera,
    scene: Option<Scene>,
    trans: Option<Transition>,
    after: Option<After>,
    /// A change of folder the view wants (Back/Forward, zoom out, reset, zoom to selection);
    /// the app applies it to its zoom path.
    root_request: Option<Vec<u32>>,
    history: Vec<Entry>,
    cursor: usize,
    tree_key: usize,
    memo: Arc<Memo>,
    tx: mpsc::Sender<Built>,
    rx: mpsc::Receiver<Built>,
    next_id: u64,
    paint: Option<Paint>,
    ctx: Option<egui::Context>,
    last_frame: Option<Instant>,
    last_pass: Option<u64>,
    was_animating: bool,
    /// When the camera last moved; history records a place once it has been still a moment.
    last_motion: Instant,
    resize_since: Option<Instant>,
    frame_no: u64,
    pub dragging: bool,
    /// Hovered tile fading in (rect, strength) and the previous one fading out.
    hover: Option<(DRect, f32)>,
    hover_prev: Option<(DRect, f32)>,
    /// Pointer light strength (fades in and out) and its last world position.
    light: f32,
    light_at: (f64, f64),
    /// Labels fade in after a change of folder.
    label_fade: f32,
    /// Per-label fade (by path in the current folder): names fade in when a tile grows big
    /// enough for one and out when it shrinks, instead of popping.
    label_alpha: std::collections::HashMap<Vec<u32>, f32>,
    last_dt: f32,
    /// Timings for the status line / profiling.
    pub last_build_ms: f64,
    pub frame_cpu_ms: f64,
}

/// What `render` reports back.
pub struct Frame {
    /// Tile under the pointer: path from the scene's folder, and its world rectangle.
    pub hover: Option<(Vec<u32>, DRect)>,
    pub animating: bool,
}

impl MapView {
    pub fn new(backend: Backend) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            backend,
            camera: Camera::new(DRect::from_size(1.0, 1.0), (1.0, 1.0)),
            scene: None,
            trans: None,
            after: None,
            root_request: None,
            history: Vec::new(),
            cursor: 0,
            tree_key: 0,
            memo: Arc::new(Memo::default()),
            tx,
            rx,
            next_id: 1,
            paint: None,
            ctx: None,
            last_frame: None,
            last_pass: None,
            was_animating: false,
            last_motion: Instant::now(),
            resize_since: None,
            frame_no: 0,
            dragging: false,
            hover: None,
            hover_prev: None,
            light: 0.0,
            light_at: (0.0, 0.0),
            label_fade: 1.0,
            label_alpha: Default::default(),
            last_dt: 0.0,
            last_build_ms: 0.0,
            frame_cpu_ms: 0.0,
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Forget everything (a new scan).
    pub fn clear(&mut self) {
        self.scene = None;
        self.trans = None;
        self.after = None;
        self.root_request = None;
        self.history.clear();
        self.cursor = 0;
        self.memo = Arc::new(Memo::default());
        self.hover = None;
        self.hover_prev = None;
    }

    pub fn set_paint(&mut self, paint: Paint) {
        self.paint = Some(paint);
    }

    pub fn paint_key(&self) -> Option<u64> {
        self.paint.as_ref().map(|p| p.key)
    }

    pub fn take_root_request(&mut self) -> Option<Vec<u32>> {
        self.root_request.take()
    }

    pub fn in_transition(&self) -> bool {
        self.trans.is_some()
    }

    pub fn can_go_back(&self) -> bool {
        self.cursor > 0 || self.current_entry().is_some_and(|e| self.history.get(self.cursor).is_some_and(|h| !h.same(&e)))
    }

    pub fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.history.len()
    }

    // ---------- coordinates ----------

    /// Screen position to world point of the current scene.
    pub fn to_world(&self, bounds: Rect, p: Pos2) -> (f64, f64) {
        let v = self.camera.view();
        let (sx, sy) = ((p.x - bounds.min.x) as f64, (p.y - bounds.min.y) as f64);
        camera::to_world(&v, (bounds.width() as f64, bounds.height() as f64), sx, sy)
    }

    /// World rectangle of the current scene to screen.
    pub fn to_screen(&self, bounds: Rect, r: &DRect) -> Rect {
        rect_on_screen(&self.camera.view(), bounds, r)
    }

    /// Screen point in the camera's viewport coordinates (they differ while a resize settles).
    fn cam_point(&self, bounds: Rect, p: Pos2) -> (f64, f64) {
        let (vw, vh) = self.camera.viewport();
        (((p.x - bounds.min.x) / bounds.width()) as f64 * vw, ((p.y - bounds.min.y) / bounds.height()) as f64 * vh)
    }

    // ---------- navigation ----------

    pub fn zoom_wheel(&mut self, amount: f64, at: Pos2, bounds: Rect, feel: &Feel) {
        if self.trans.is_some() {
            self.finish_transition(feel);
        }
        let p = self.cam_point(bounds, at);
        self.camera.zoom_by(amount, p, feel);
    }

    pub fn follow_pointer(&mut self, at: Pos2, bounds: Rect) {
        let p = self.cam_point(bounds, at);
        self.camera.follow_pointer(p);
    }

    pub fn pan(&mut self, delta: egui::Vec2, bounds: Rect) {
        if self.trans.is_some() {
            return;
        }
        let (vw, vh) = self.camera.viewport();
        let (dx, dy) = (delta.x as f64 * vw / bounds.width() as f64, delta.y as f64 * vh / bounds.height() as f64);
        self.camera.pan_by(dx, dy);
    }

    pub fn release(&mut self, velocity: egui::Vec2, feel: &Feel) {
        self.camera.release(velocity.x as f64, velocity.y as f64, feel);
    }

    /// Animate to frame an entry (path from the scan root), changing folder first if it isn't
    /// inside the one on screen.
    pub fn frame_path(&mut self, tree: &Node, abs: &[u32], feel: &Feel) {
        if self.scene.is_none() {
            return;
        }
        if self.trans.is_some() {
            self.finish_transition(feel);
        }
        let scene = self.scene.as_ref().unwrap();
        if abs.starts_with(&scene.root) {
            let Some(root) = tree.at(&scene.root) else { return };
            let cushion = self.cushion();
            if let Some((r, _)) = treemap::locate(root, scene.world, &abs[scene.root.len()..], cushion) {
                self.camera.frame(&r, FRAME_PAD, feel);
            }
        } else {
            let common = abs.iter().zip(&scene.root).take_while(|(a, b)| a == b).count();
            self.root_request = Some(abs[..common].to_vec());
            self.after = Some(After::Frame(abs.to_vec()));
        }
    }

    /// Zoom out one level: to the folder around the view, or to the parent folder when the
    /// whole folder is already in view.
    pub fn zoom_out(&mut self, tree: &Node, feel: &Feel) {
        if self.trans.is_some() {
            self.finish_transition(feel);
        }
        let Some(scene) = &mut self.scene else { return };
        let Some(root) = tree.at(&scene.root) else { return };
        let zoom = self.camera.zoom();
        if zoom > 1.02 {
            let view = self.camera.view();
            let (cx, cy) = view.center();
            let detail = Detail { unit: view.w() * 1e-4 };
            let path = scene.hits.hit(root, scene.world, cx, cy, detail, Some(&self.memo)).map(|h| h.0).unwrap_or_default();
            let rects = treemap::ancestors(root, scene.world, &path);
            for k in (0..rects.len()).rev() {
                if !root.at(&path[..k]).is_some_and(|n| n.is_dir) {
                    continue;
                }
                let to = self.camera.framing(&rects[k], FRAME_PAD * 0.5);
                if self.camera.world().w() / to.w() < zoom * 0.92 {
                    if k == 0 {
                        self.camera.reset(feel);
                    } else {
                        self.camera.fly_to(to, feel);
                    }
                    return;
                }
            }
            self.camera.reset(feel);
        } else if !scene.root.is_empty() {
            self.root_request = Some(scene.root[..scene.root.len() - 1].to_vec());
        }
    }

    /// Back to the whole scan.
    pub fn reset(&mut self, feel: &Feel) {
        if self.trans.is_some() {
            self.finish_transition(feel);
        }
        let Some(scene) = &self.scene else { return };
        if scene.root.is_empty() {
            self.camera.reset(feel);
        } else {
            self.root_request = Some(Vec::new());
        }
    }

    pub fn back(&mut self, feel: &Feel) {
        self.commit_now();
        if self.cursor > 0 {
            self.cursor -= 1;
            self.go(self.history[self.cursor].clone(), feel);
        }
    }

    pub fn forward(&mut self, feel: &Feel) {
        self.commit_now();
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            self.go(self.history[self.cursor].clone(), feel);
        }
    }

    fn go(&mut self, e: Entry, feel: &Feel) {
        if self.trans.is_some() {
            self.finish_transition(feel);
        }
        let Some(scene) = &self.scene else { return };
        if e.root == scene.root {
            let to = camera::clamp_view(&scene.world, camera::absolute(&scene.world, &e.view));
            self.camera.fly_to(to, feel);
        } else {
            self.root_request = Some(e.root.clone());
            self.after = Some(After::View(e.view));
        }
    }

    fn current_entry(&self) -> Option<Entry> {
        let s = self.scene.as_ref()?;
        Some(Entry { root: s.root.clone(), view: camera::relative(&s.world, &self.camera.view()) })
    }

    /// Record where we are if it's somewhere new.
    fn commit_now(&mut self) {
        if self.trans.is_some() || self.camera.is_moving() {
            return;
        }
        let Some(e) = self.current_entry() else { return };
        if self.history.get(self.cursor).is_some_and(|h| h.same(&e)) {
            return;
        }
        if !self.history.is_empty() {
            self.history.truncate(self.cursor + 1);
        }
        self.history.push(e);
        if self.history.len() > 200 {
            self.history.remove(0);
        }
        self.cursor = self.history.len() - 1;
    }

    fn cushion(&self) -> Cushion {
        self.paint.as_ref().map(|p| p.cushion).unwrap_or(Cushion { height: 0.38, scale: 0.91 })
    }

    // ---------- per frame ----------

    /// Keep the scene in step with the app's zoom path (starting a drill animation when it
    /// changed) and the map's size. Call first thing each frame.
    pub fn sync(&mut self, ctx: &egui::Context, tree: &Arc<Node>, root: &[u32], bounds: Rect, feel: &Feel) {
        self.ctx = Some(ctx.clone());
        let now = Instant::now();
        // Was the map on screen in the previous frame? (Not if the sunburst was showing.)
        let pass = ctx.cumulative_pass_nr();
        let recently_drawn = self.last_pass.is_some_and(|p| p + 3 >= pass);
        let key = Arc::as_ptr(tree) as usize;
        if key != self.tree_key {
            self.clear();
            self.tree_key = key;
        }
        let size = (bounds.width() as f64, bounds.height() as f64);
        if size.0 < 1.0 || size.1 < 1.0 {
            return;
        }

        // Window resized: once it settles, lay the folder out again for the new shape.
        if let Some(s) = &self.scene {
            let changed = (s.world.w() - size.0).abs() > 0.5 || (s.world.h() - size.1).abs() > 0.5;
            if changed {
                let since = *self.resize_since.get_or_insert(now);
                if now.duration_since(since).as_millis() >= 150 || !recently_drawn {
                    self.resize_since = None;
                    self.finish_transition(feel);
                    let world = DRect::from_size(size.0, size.1);
                    let s = self.scene.as_mut().unwrap();
                    s.world = world;
                    s.hits.clear();
                    s.base_job = None;
                    s.detail_job = None;
                    self.camera.rebase(world, size);
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(160));
                }
            } else {
                self.resize_since = None;
            }
        }

        if self.scene.as_ref().is_some_and(|s| s.root == root) {
            return;
        }
        let world = DRect::from_size(size.0, size.1);
        let id = self.id();
        let new = Scene::new(id, root.to_vec(), world);
        let after = self.after.take();
        self.finish_transition(feel);
        let cushion = self.cushion();
        let Some(old) = self.scene.take() else {
            self.scene = Some(new);
            self.camera = Camera::new(world, size);
            self.apply_after(tree, after, feel, false);
            return;
        };
        let animate = feel.smooth && recently_drawn && old.world == world;
        let mut trans = None;
        if animate && root.len() > old.root.len() && root.starts_with(&old.root) {
            // Drill in: the folder grows from where it is to fill the map.
            if let Some((rf, _)) = tree.at(&old.root).and_then(|n| treemap::locate(n, old.world, &root[old.root.len()..], cushion)) {
                let v = match &after {
                    Some(After::View(v)) => *v,
                    _ => DRect::new(0.0, 0.0, 1.0, 1.0),
                };
                let from = self.camera.view();
                let to = camera::absolute(&rf, &v);
                trans = Some((rf, from, to, true));
            }
        } else if animate && old.root.len() > root.len() && old.root.starts_with(root) {
            // Drill out: the folder we were in shrinks back into its place in the parent.
            if let Some((rf, _)) = tree.at(root).and_then(|n| treemap::locate(n, world, &old.root[root.len()..], cushion)) {
                let from = camera::absolute(&rf, &camera::relative(&old.world, &self.camera.view()));
                let mut cam = Camera::new(world, size);
                let to = match &after {
                    Some(After::View(v)) => camera::clamp_view(&world, camera::absolute(&world, v)),
                    Some(After::Frame(p)) if p.starts_with(root) => tree
                        .at(root)
                        .and_then(|n| treemap::locate(n, world, &p[root.len()..], cushion))
                        .map(|(r, _)| cam.framing(&r, FRAME_PAD))
                        .unwrap_or(world),
                    _ => world,
                };
                cam.set_view(from);
                self.camera = cam;
                trans = Some((rf, from, to, false));
            }
        }
        match trans {
            Some((rf, from, to, drill_in)) => {
                let zoom = (rf.w() / world.w()).ln().abs();
                let secs = (0.36 + 0.05 * zoom).min(0.8) / feel.speed.max(0.1);
                if drill_in {
                    self.camera = Camera::new(world, size);
                    if let Some(After::Frame(p)) = after {
                        self.after = Some(After::Frame(p));
                    }
                }
                self.trans = Some(Transition { old, drill_in, rf, from, to, t: 0.0, secs });
                self.scene = Some(new);
                self.hover = None;
                self.hover_prev = None;
            }
            None => {
                self.scene = Some(new);
                self.camera = Camera::new(world, size);
                self.apply_after(tree, after, feel, false);
            }
        }
        self.label_fade = 0.0;
    }

    fn apply_after(&mut self, tree: &Node, after: Option<After>, feel: &Feel, animate: bool) {
        let instant = Feel { smooth: false, ..*feel };
        let feel = if animate { feel } else { &instant };
        match after {
            Some(After::View(v)) => {
                let w = self.camera.world();
                self.camera.fly_to(camera::clamp_view(&w, camera::absolute(&w, &v)), feel);
            }
            Some(After::Frame(p)) => self.frame_path(tree, &p, feel),
            None => {}
        }
    }

    /// Jump to the end of a running drill animation.
    fn finish_transition(&mut self, _feel: &Feel) {
        let Some(t) = self.trans.take() else { return };
        if t.drill_in {
            let w = self.camera.world();
            let v = camera::relative(&t.rf, &t.to);
            self.camera.set_view(camera::clamp_view(&w, camera::absolute(&w, &v)));
        } else {
            self.camera.set_view(t.to);
        }
        drop(t.old);
    }

    /// Advance animations, pick up finished tile sets and request new ones, and draw.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        painter: &egui::Painter,
        tree: &Arc<Node>,
        bounds: Rect,
        pointer: Option<Pos2>,
        look: &Look,
        feel: &Feel,
    ) -> Frame {
        let t0 = Instant::now();
        let now = Instant::now();
        // After an idle spell (nothing was moving, so no frames were drawn), the time since the
        // last frame says nothing about animation; start new motion with a normal frame step
        // rather than jumping ahead by the idle time.
        let gap = self.last_frame.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        let dt = if self.was_animating { gap.min(0.1) } else { gap.min(1.0 / 60.0) };
        self.last_frame = Some(now);
        self.frame_no += 1;
        let ctx = painter.ctx().clone();
        self.last_pass = Some(ctx.cumulative_pass_nr());
        let ppp = ctx.pixels_per_point() as f64;
        let bounds_px = (bounds.width() as f64 * ppp, bounds.height() as f64 * ppp);

        // Animations.
        let mut animating = self.camera.update(dt, feel);
        if let Some(t) = &mut self.trans {
            t.t = (t.t + dt / t.secs).min(1.0);
            animating = true;
            if !t.drill_in {
                let v = camera::interpolate(&t.from, &t.to, camera::ease(t.t));
                self.camera.set_view(v);
            }
            if t.t >= 1.0 {
                self.finish_transition(feel);
                let after = self.after.take();
                self.apply_after(tree, after, feel, true);
            }
        }
        if animating || self.dragging {
            self.last_motion = now;
        } else if now.duration_since(self.last_motion).as_secs_f64() > 0.45 {
            self.commit_now();
        }
        self.label_fade = (self.label_fade + dt as f32 / 0.25).min(1.0);
        self.last_dt = dt as f32;

        // Builds.
        self.poll(&ctx);
        let draws = self.plan(bounds);
        let raster = !self.backend.is_gpu();
        for (which, view, _, _) in &draws {
            self.want(*which, view, bounds_px, ppp, tree, raster);
        }

        // Hover and the pointer light.
        let mut hover = None;
        if let (Some(p), None) = (pointer, &self.trans) {
            if bounds.contains(p) && !self.dragging {
                hover = self.hit(tree, bounds, p);
                self.light_at = self.to_world(bounds, p);
            }
        }
        let hover_rect = hover.as_ref().map(|h| h.1);
        if self.hover.map(|h| h.0) != hover_rect {
            self.hover_prev = self.hover.filter(|h| h.1 > 0.02);
            self.hover = hover_rect.map(|r| (r, 0.0));
        }
        let k = dt as f32;
        if let Some(h) = &mut self.hover {
            h.1 = (h.1 + k / 0.09).min(1.0);
            animating |= h.1 < 1.0;
        }
        if let Some(h) = &mut self.hover_prev {
            h.1 -= k / 0.14;
            animating = true;
        }
        if self.hover_prev.is_some_and(|h| h.1 <= 0.0) {
            self.hover_prev = None;
        }
        let light_target = if look.lighting && hover.is_some() { 1.0 } else { 0.0 };
        let before = self.light;
        self.light += (light_target - self.light) * (1.0 - (-k / 0.12).exp());
        if (self.light - light_target).abs() < 0.003 {
            self.light = light_target;
        }
        animating |= self.light != before;

        // Draw.
        let mut slot = 0;
        for (which, view, alpha, clip) in &draws {
            let lit = *which == 0 && self.trans.is_none();
            self.draw_scene(painter, *which, view, *alpha, *clip, bounds, bounds_px, look, lit, &mut slot);
        }
        self.frame_cpu_ms = t0.elapsed().as_secs_f64() * 1000.0;
        if std::env::var_os("DIRMAP_TRACE").is_some() {
            eprintln!(
                "frame dt={:.1}ms zoom={:.2} trans={:?} draws={} cpu={:.2}ms",
                dt * 1000.0,
                self.camera.zoom(),
                self.trans.as_ref().map(|t| (t.drill_in, (t.t * 100.0).round() / 100.0)),
                draws.len(),
                self.frame_cpu_ms
            );
        }
        let pending = self.scenes().any(|s| s.base_job.is_some() || s.detail_job.is_some());
        let animating = animating || self.label_fade < 1.0 || (pending && self.scene.as_ref().is_some_and(|s| s.base.is_none()));
        self.was_animating = animating || self.dragging;
        Frame { hover, animating }
    }

    fn scenes(&self) -> impl Iterator<Item = &Scene> {
        self.scene.iter().chain(self.trans.as_ref().map(|t| &t.old))
    }

    fn scene_mut(&mut self, which: usize) -> Option<&mut Scene> {
        if which == 0 { self.scene.as_mut() } else { self.trans.as_mut().map(|t| &mut t.old) }
    }

    fn scene_ref(&self, which: usize) -> Option<&Scene> {
        if which == 0 { self.scene.as_ref() } else { self.trans.as_ref().map(|t| &t.old) }
    }

    /// What to draw this frame: (scene: 0 current / 1 transition's old one, its view, alpha, clip).
    fn plan(&self, bounds: Rect) -> Vec<(usize, DRect, f32, Rect)> {
        let Some(scene) = &self.scene else { return Vec::new() };
        let Some(t) = &self.trans else {
            return vec![(0, self.camera.view(), 1.0, bounds)];
        };
        let e = camera::ease(t.t);
        let vs = camera::interpolate(&t.from, &t.to, e);
        let (shallow, deep, deep_world) = if t.drill_in { (1, 0, scene.world) } else { (0, 1, t.old.world) };
        // Where the deep folder sits on screen, and the view that puts its whole world there.
        let s = rect_on_screen(&vs, bounds, &t.rf);
        let (bw, bh) = (bounds.width() as f64, bounds.height() as f64);
        let (sw, sh) = (s.width() as f64, s.height() as f64);
        if sw <= 0.0 || sh <= 0.0 {
            return vec![(shallow, vs, 1.0, bounds)];
        }
        let w = DRect::new(
            deep_world.x0 + (bounds.min.x - s.min.x) as f64 * deep_world.w() / sw,
            deep_world.y0 + (bounds.min.y - s.min.y) as f64 * deep_world.h() / sh,
            0.0,
            0.0,
        );
        let vd = DRect::new(w.x0, w.y0, w.x0 + bw * deep_world.w() / sw, w.y0 + bh * deep_world.h() / sh);
        let a = if t.drill_in { smoothstep(0.0, 0.55, t.t) } else { 1.0 - smoothstep(0.1, 0.6, t.t) } as f32;
        vec![(shallow, vs, 1.0, bounds), (deep, vd, a, s.intersect(bounds))]
    }

    // ---------- tile building ----------

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(b) = self.rx.try_recv() {
            let Built { scene, kind, mut set, millis } = b;
            let mut wanted = false;
            for which in [0, 1] {
                let Some(s) = self.scene_mut(which) else { continue };
                if s.id != scene {
                    continue;
                }
                let job = if kind == Kind::Base { &mut s.base_job } else { &mut s.detail_job };
                if job.as_ref().is_some_and(|j| j.id == set.id) {
                    *job = None;
                    if let Some(img) = set.image.take() {
                        set.texture = Some(ctx.load_texture(format!("treemap-{}", set.id), img, TextureOptions::LINEAR));
                    }
                    if kind == Kind::Base {
                        s.base = Some(set);
                    } else {
                        s.detail = Some(set);
                    }
                    wanted = true;
                }
                break;
            }
            if wanted {
                self.last_build_ms = millis;
            }
        }
    }

    /// Request the tile sets scene `which` needs to show `view` sharply.
    fn want(&mut self, which: usize, view: &DRect, bounds_px: (f64, f64), ppp: f64, tree: &Arc<Node>, raster: bool) {
        let Some(paint) = self.paint.clone() else { return };
        let memo = self.memo.clone();
        let ctx = self.ctx.clone();
        let tx = self.tx.clone();
        let mut ids = [self.id(), self.id()];
        let Some(s) = self.scene_mut(which) else { return };
        s.px_w = bounds_px.0;
        let world = s.world;
        let base_ppu = (bounds_px.0 / world.w()).max(bounds_px.1 / world.h());
        let fits = |key: u64, w: &DRect, cover: &DRect, ppu: f64, vis: &DRect, want: f64, lo: f64, hi: f64| {
            key == paint.key && *w == world && cover.contains_rect(vis) && ppu / want > lo && ppu / want < hi
        };
        let spawn = |kind: Kind, id: u64, cover: DRect, ppu: f64, root: Vec<u32>, scene: u64| {
            let cancel = Arc::new(AtomicBool::new(false));
            let req = Req {
                scene,
                kind,
                id,
                tree: tree.clone(),
                root,
                world,
                cover,
                ppu,
                ppp,
                paint: paint.clone(),
                memo: memo.clone(),
                raster,
                cancel: cancel.clone(),
            };
            let (tx, ctx) = (tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                if let Some(b) = run(req) {
                    if tx.send(b).is_ok() {
                        if let Some(c) = ctx {
                            c.request_repaint();
                        }
                    }
                }
            });
            Job { id, cancel, paint_key: paint.key, world, cover, ppu }
        };
        let base_ok = s.base.as_ref().is_some_and(|b| fits(b.paint_key, &b.world, &b.cover, b.ppu, &world, base_ppu, 0.85, 1.18));
        let base_pending = s.base_job.as_ref().is_some_and(|j| fits(j.paint_key, &j.world, &j.cover, j.ppu, &world, base_ppu, 0.85, 1.18));
        if !base_ok && !base_pending {
            s.base_job = Some(spawn(Kind::Base, ids[0], world, base_ppu, s.root.clone(), s.id));
            ids[0] = 0;
        }
        let vis = view.intersect(&world);
        if vis.w() <= 0.0 || vis.h() <= 0.0 {
            return;
        }
        let ppu = (bounds_px.0 / view.w()).max(bounds_px.1 / view.h());
        if ppu <= base_ppu * 1.3 {
            s.detail_job = None;
            s.detail = None;
            return;
        }
        let ok = s.detail.as_ref().is_some_and(|d| fits(d.paint_key, &d.world, &d.cover, d.ppu, &vis, ppu, 0.6, 1.6));
        let pending = s.detail_job.as_ref().is_some_and(|j| fits(j.paint_key, &j.world, &j.cover, j.ppu, &vis, ppu, 0.6, 1.6));
        if !ok && !pending {
            let cover = vis.grow(0.3, 0.3).intersect(&world);
            s.detail_job = Some(spawn(Kind::Detail, ids[1], cover, ppu, s.root.clone(), s.id));
        }
    }

    // ---------- hit testing ----------

    /// The tile under screen point `p`: path from the scene's folder and its world rectangle.
    pub fn hit(&mut self, tree: &Node, bounds: Rect, p: Pos2) -> Option<(Vec<u32>, DRect)> {
        if self.trans.is_some() {
            return None;
        }
        let (x, y) = self.to_world(bounds, p);
        let view = self.camera.view();
        let key = self.paint_key().unwrap_or(0);
        let memo = self.memo.clone();
        let scene = self.scene.as_mut()?;
        let unit = scene.layers(&view, key).1.map(|s| s.unit()).unwrap_or(view.w() / bounds.width() as f64);
        let root = tree.at(&scene.root)?;
        scene.hits.hit(root, scene.world, x, y, Detail { unit }, Some(&memo))
    }

    /// Folder rectangles from the scene's folder down to `rel` (for outlines).
    pub fn locate(&self, tree: &Node, rel: &[u32]) -> Option<DRect> {
        let s = self.scene.as_ref()?;
        treemap::locate(tree.at(&s.root)?, s.world, rel, self.cushion()).map(|r| r.0)
    }

    pub fn root(&self) -> Option<&[u32]> {
        self.scene.as_ref().map(|s| &s.root[..])
    }

    /// Labels to draw, with their screen rectangles and opacity (0..1). A tile gets a name once
    /// it is at least 44 x 15 points; names fade in and out over a short time as tiles cross that.
    pub fn labels(&mut self, bounds: Rect) -> Vec<(Rect, &Label, f32)> {
        let step = self.last_dt / 0.16;
        let mut out = Vec::new();
        let Some(scene) = &self.scene else { return out };
        if self.trans.is_some() {
            self.label_alpha.clear();
            return out;
        }
        let view = self.camera.view();
        let key = self.paint.as_ref().map(|p| p.key).unwrap_or(0);
        let Some(set) = scene.layers(&view, key).1 else { return out };
        let sv = set_view(set, &scene.world, &view);
        let fade = self.label_fade;
        let alphas = &mut self.label_alpha;
        let mut seen = std::collections::HashSet::new();
        for l in &set.labels {
            let r = rect_on_screen(&sv, bounds, &l.rect);
            if !r.intersects(bounds) {
                continue;
            }
            let fits = r.width() >= 44.0 && r.height() >= 15.0;
            let a = match alphas.get_mut(&l.path) {
                Some(a) => {
                    *a = if fits { (*a + step).min(1.0) } else { *a - step };
                    *a
                }
                None if fits => {
                    // Already big enough when it first came into view (scrolled in, new folder):
                    // show at once, the folder-level fade covers it.
                    let a = if fade < 1.0 || step == 0.0 { 1.0 } else { step };
                    alphas.insert(l.path.clone(), a);
                    a
                }
                None => continue,
            };
            seen.insert(&l.path);
            if a > 0.0 {
                out.push((r, l, a.min(1.0) * fade));
            }
        }
        alphas.retain(|p, a| *a > 0.0 && seen.contains(p));
        out
    }

    /// True while any label is still fading.
    pub fn labels_fading(&self) -> bool {
        self.label_alpha.values().any(|a| *a < 1.0)
    }

    // ---------- drawing ----------

    #[allow(clippy::too_many_arguments)]
    fn draw_scene(
        &self,
        painter: &egui::Painter,
        which: usize,
        view: &DRect,
        alpha: f32,
        clip: Rect,
        bounds: Rect,
        bounds_px: (f64, f64),
        look: &Look,
        lit: bool,
        slot: &mut usize,
    ) {
        let Some(scene) = self.scene_ref(which) else { return };
        let key = self.paint_key().unwrap_or(0);
        let p = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
        let (layers, _) = scene.layers(view, key);
        if layers.is_empty() {
            // Nothing built yet: plain background.
            let bg = self.paint.as_ref().map(|p| p.background).unwrap_or(Color32::from_gray(45));
            p.rect_filled(rect_on_screen(view, bounds, &scene.world).intersect(bounds), 0.0, bg.gamma_multiply(alpha));
            return;
        }
        for set in layers {
            let sv = set_view(set, &scene.world, view);
            match &set.texture {
                Some(tex) if !self.backend.is_gpu() => {
                    let r = rect_on_screen(&sv, bounds, &set.cover);
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    p.image(tex.id(), r, uv, Color32::WHITE.gamma_multiply(alpha));
                }
                _ => {
                    let u = self.uniforms(set, &sv, &scene.world, view, bounds_px, alpha, look, lit);
                    let draw = Draw { id: set.id, instances: set.instances.clone(), uniforms: u };
                    self.backend.paint(&p, bounds, draw, *slot, self.frame_no);
                    *slot += 1;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn uniforms(&self, set: &TileSet, sv: &DRect, scene_world: &DRect, view: &DRect, bounds_px: (f64, f64), alpha: f32, look: &Look, lit: bool) -> Uniforms {
        let (ox, oy) = set.origin;
        let ppu = set.ppu;
        let mut u = Uniforms {
            xform: [
                (2.0 / (ppu * sv.w())) as f32,
                (-2.0 / (ppu * sv.h())) as f32,
                (2.0 * (ox - sv.x0) / sv.w() - 1.0) as f32,
                (1.0 - 2.0 * (oy - sv.y0) / sv.h()) as f32,
            ],
            ..Default::default()
        };
        let k = (bounds_px.0 / (ppu * sv.w()), bounds_px.1 / (ppu * sv.h()));
        let mut opts = 0;
        if look.bevel {
            opts |= tilegpu::BEVEL;
        }
        if look.grid.is_some() {
            opts |= tilegpu::GRID;
        }
        u.px = [k.0 as f32, k.1 as f32, alpha, opts as f32];
        u.shade = [look.ambient, look.diffuse, look.gain, 0.0];
        u.sun = [look.sun[0], look.sun[1], look.sun[2], 0.0];
        if let Some(g) = look.grid {
            u.grid = [g.r() as f32 / 255.0, g.g() as f32 / 255.0, g.b() as f32 / 255.0, 1.0];
        }
        let to_set = |x: f64, y: f64| {
            // Scene world to the set's world (identical unless a resize is pending).
            let r = camera::relative(scene_world, &DRect::new(x, y, x, y));
            let a = camera::absolute(&set.world, &r);
            (a.x0, a.y0)
        };
        if lit && self.light > 0.0 {
            let (lx, ly) = to_set(self.light_at.0, self.light_at.1);
            let radius = (0.45 * bounds_px.0.min(bounds_px.1)).clamp(220.0, 900.0);
            u.light = [((lx - ox) * ppu) as f32, ((ly - oy) * ppu) as f32, self.light * look.light_level, radius as f32];
            u.light2 = [(0.8 * radius) as f32, LIGHT_DIFFUSE, LIGHT_SPECULAR, 0.0];
        }
        if lit {
            let rect_in_set = |r: &DRect| {
                let a = to_set(r.x0, r.y0);
                let b = to_set(r.x1, r.y1);
                set.local_rect(&DRect::new(a.0, a.1, b.0, b.1))
            };
            if let Some((r, a)) = self.hover {
                u.hover_a = rect_in_set(&r);
                u.hover_w[0] = a;
            }
            if let Some((r, a)) = self.hover_prev {
                u.hover_b = rect_in_set(&r);
                u.hover_w[1] = a.max(0.0);
            }
        }
        let _ = view;
        u
    }
}

/// The view of a set's own world that matches `view` of the scene's world.
fn set_view(set: &TileSet, scene_world: &DRect, view: &DRect) -> DRect {
    if set.world == *scene_world {
        *view
    } else {
        camera::absolute(&set.world, &camera::relative(scene_world, view))
    }
}

/// World rectangle `r` on screen when `view` fills `bounds`.
pub fn rect_on_screen(view: &DRect, bounds: Rect, r: &DRect) -> Rect {
    let (bw, bh) = (bounds.width() as f64, bounds.height() as f64);
    let sx = |x: f64| bounds.min.x + ((x - view.x0) / view.w() * bw) as f32;
    let sy = |y: f64| bounds.min.y + ((y - view.y0) / view.h() * bh) as f32;
    // Keep far-off coordinates finite for the painter.
    let c = |v: f32| v.clamp(-1e7, 1e7);
    Rect::from_min_max(pos2(c(sx(r.x0)), c(sy(r.y0))), pos2(c(sx(r.x1)), c(sy(r.y1))))
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------- the builder (runs on a background thread) ----------

struct Req {
    scene: u64,
    kind: Kind,
    id: u64,
    tree: Arc<Node>,
    root: Vec<u32>,
    world: DRect,
    cover: DRect,
    ppu: f64,
    ppp: f64,
    paint: Paint,
    memo: Arc<Memo>,
    raster: bool,
    cancel: Arc<AtomicBool>,
}

/// Leaf data kept for the CPU fallback's image.
struct RasterTile {
    rect: DRect,
    surface: treemap::Surface,
    color: Color32,
    flat: bool,
}

fn run(req: Req) -> Option<Built> {
    let t0 = Instant::now();
    let set = build_set(&req)?;
    Some(Built { scene: req.scene, kind: req.kind, set, millis: t0.elapsed().as_secs_f64() * 1000.0 })
}

/// Smallest size, in points, of the marker over a patch of hidden matches (`Paint::hidden`).
const MARK_PT: f64 = 5.0;

/// Build the tiles for a request. None if it was cancelled.
fn build_set(req: &Req) -> Option<TileSet> {
    let root = req.tree.at(&req.root)?;
    let (ox, oy) = req.cover.center();
    let ppu = req.ppu;
    // Tiles are clipped to well beyond the cover so their clipped edges are never on screen.
    let clip = req.cover.grow(0.5, 0.5);
    let mut set = TileSet {
        id: req.id,
        paint_key: req.paint.key,
        world: req.world,
        cover: req.cover,
        clip,
        origin: (ox, oy),
        ppu,
        instances: Arc::new(Vec::new()),
        labels: Vec::new(),
        image: None,
        texture: None,
    };
    let mut inst = Vec::new();
    let bg = req.paint.background;
    // Background for gaps left by tiles too small to draw. Only over the cover: outside it, the
    // set has no tiles and whatever is underneath (the base set) must show through.
    inst.push(Instance { rect: set.local_rect(&req.cover), grad: [0.0; 4], color: bg.to_array(), flags: tilegpu::FLAT });
    let mut raster = Vec::new();
    let mut labels = Vec::new();
    // Drawn last, over their neighbours.
    let mut markers = Vec::new();
    let mut raster_markers = Vec::new();
    let mut abs = req.root.clone();
    let rl = abs.len();
    let pt_per_unit = ppu / req.ppp;
    let unit = 1.0 / ppu;
    let mut count = 0u32;
    let mut cancelled = false;
    let dark = Color32::from_gray(24);
    treemap::build(root, req.world, &req.cover, Detail { unit }, req.paint.cushion, Some(&req.memo), &mut |l| {
        count += 1;
        if count % 4096 == 0 && req.cancel.load(Ordering::Relaxed) {
            cancelled = true;
            return false;
        }
        abs.truncate(rl);
        abs.extend_from_slice(l.path);
        let rect = set.local_rect(&l.rect);
        if l.node.is_dir {
            // Files too small to draw at this zoom: fill their area with the colour they
            // would add up to, shaded as part of the folder, rather than leaving dark holes.
            let mut budget = 48;
            let marked = req.paint.hidden.as_ref().and_then(|h| h(l.node, &abs, l.hidden_from));
            if let Some(c) = marked {
                // A sliver of a pixel is easy to miss: also drop a small square marker on top,
                // MARK_PT across, centred on the patch.
                let min = MARK_PT * req.ppp / ppu;
                let mut mark_at = |x: f64, y: f64| {
                    let hw = min / 2.0;
                    let r = DRect { x0: x - hw, y0: y - hw, x1: x + hw, y1: y + hw };
                    markers.push(Instance { rect: set.local_rect(&r), grad: [0.0; 4], color: c.to_array(), flags: tilegpu::FLAT });
                    if req.raster {
                        raster_markers.push(RasterTile { rect: r, surface: l.surface, color: c, flat: true });
                    }
                };
                if l.rect.w().max(l.rect.h()) < min {
                    let (cx, cy) = l.rect.center();
                    mark_at(cx, cy);
                } else {
                    // A big folder whose small files hold matches: the patch is covered by the
                    // files drawn on top, leaving only slivers, so mark where the matches are.
                    // The same for a long thin folder too narrow to divide: one marker in its
                    // middle would be far from matches near its ends.
                    let hidden = req.paint.hidden.as_ref().unwrap();
                    for (x, y) in hidden_spots(l.node, &l.rect, l.hidden_from, &mut abs, &req.cover, min, &req.memo, &*req.paint.colorize, &**hidden) {
                        mark_at(x, y);
                    }
                }
            }
            let Some(fill) = marked.or_else(|| blend_of(l.node, l.hidden_from, &mut abs, &*req.paint.colorize, 0, &mut budget)) else {
                return true; // nothing in there passes the filter: background
            };
            let [s0, s1, s2, s3] = l.surface;
            let grad = [(2.0 * s0 / ppu) as f32, (2.0 * s0 * ox + s2) as f32, (2.0 * s1 / ppu) as f32, (2.0 * s1 * oy + s3) as f32];
            inst.push(Instance { rect, grad, color: fill.to_array(), flags: 0 });
            if req.raster {
                raster.push(RasterTile { rect: l.rect, surface: l.surface, color: fill, flat: false });
            }
            return true;
        }
        match (req.paint.colorize)(l.node, &abs) {
            None => {
                inst.push(Instance { rect, grad: [0.0; 4], color: dark.to_array(), flags: tilegpu::FLAT });
                if req.raster {
                    raster.push(RasterTile { rect: l.rect, surface: l.surface, color: dark, flat: true });
                }
            }
            Some((fill, label)) => {
                let [s0, s1, s2, s3] = l.surface;
                let grad = [(2.0 * s0 / ppu) as f32, (2.0 * s0 * ox + s2) as f32, (2.0 * s1 / ppu) as f32, (2.0 * s1 * oy + s3) as f32];
                inst.push(Instance { rect, grad, color: fill.to_array(), flags: 0 });
                if req.raster {
                    raster.push(RasterTile { rect: l.rect, surface: l.surface, color: fill, flat: false });
                }
                let (w, h) = (l.rect.w() * pt_per_unit, l.rect.h() * pt_per_unit);
                // A filter match only a pixel or two across is drawn, but is far harder to see
                // than the marker it had while it was too small to draw: keep the marker until
                // the tile is as big.
                if req.paint.hidden.is_some() && w.min(h) < MARK_PT {
                    let hw = MARK_PT * req.ppp / ppu / 2.0;
                    let (cx, cy) = l.rect.center();
                    let r = DRect { x0: cx - hw, y0: cy - hw, x1: cx + hw, y1: cy + hw };
                    let c = fill.lerp_to_gamma(Color32::WHITE, 0.12);
                    markers.push(Instance { rect: set.local_rect(&r), grad: [0.0; 4], color: c.to_array(), flags: tilegpu::FLAT });
                    if req.raster {
                        raster_markers.push(RasterTile { rect: r, surface: l.surface, color: c, flat: true });
                    }
                }
                if w >= 26.0 && h >= 9.0 {
                    labels.push(Label { rect: l.rect, path: l.path.to_vec(), color: label });
                }
            }
        }
        true
    });
    if cancelled || req.cancel.load(Ordering::Relaxed) {
        return None;
    }
    inst.extend(markers);
    raster.extend(raster_markers);
    set.instances = Arc::new(inst);
    set.labels = labels;
    if req.raster {
        set.image = Some(rasterize(&req.cover, ppu, bg, &raster, &req.paint));
    }
    Some(set)
}

/// Centres of a folder's children from index `from` on (those too small to draw) that are
/// filter matches, as `colorize` (files) and `hidden` (folders) tell. Folders bigger than a
/// `cell` are looked into, so a long thin one gets markers along it where its matches are. Only
/// places in `cover`, at most one per `cell`-sized square, and only so many, so a folder of a
/// million files stays cheap. `abs` is the folder's path from the scan root; it's restored.
#[allow(clippy::too_many_arguments)]
fn hidden_spots(
    node: &Node,
    rect: &DRect,
    from: usize,
    abs: &mut Vec<u32>,
    cover: &DRect,
    cell: f64,
    memo: &treemap::Memo,
    colorize: &(dyn Fn(&Node, &[u32]) -> Option<(Color32, Color32)> + Send + Sync),
    hidden: &(dyn Fn(&Node, &[u32], usize) -> Option<Color32> + Send + Sync),
) -> Vec<(f64, f64)> {
    struct Spots<'a> {
        cover: &'a DRect,
        cell: f64,
        memo: &'a treemap::Memo,
        colorize: &'a (dyn Fn(&Node, &[u32]) -> Option<(Color32, Color32)> + Send + Sync),
        hidden: &'a (dyn Fn(&Node, &[u32], usize) -> Option<Color32> + Send + Sync),
        seen: std::collections::HashSet<(i64, i64)>,
        out: Vec<(f64, f64)>,
        looked: usize,
    }
    const MAX_LOOK: usize = 200_000;
    const MAX_SPOTS: usize = 4096;
    impl Spots<'_> {
        fn key(&self, (x, y): (f64, f64)) -> (i64, i64) {
            ((x / self.cell).floor() as i64, (y / self.cell).floor() as i64)
        }
        fn walk(&mut self, node: &Node, rect: &DRect, from: usize, abs: &mut Vec<u32>) {
            let rects = treemap::child_rects(node, rect, 0.0, Some(self.memo));
            for (i, r) in rects.iter().enumerate().skip(from) {
                if self.looked >= MAX_LOOK || self.out.len() >= MAX_SPOTS {
                    return;
                }
                self.looked += 1;
                if !r.intersects(self.cover) {
                    continue;
                }
                let c = &node.children[i];
                let big = r.w().max(r.h()) >= self.cell;
                // A small child lands in one cell: skip it if that cell already has a marker.
                if !big && self.seen.contains(&self.key(r.center())) {
                    continue;
                }
                abs.push(i as u32);
                if c.is_dir {
                    if (self.hidden)(c, abs, 0).is_some() {
                        if big {
                            self.walk(c, r, 0, abs);
                        } else {
                            self.spot(r.center());
                        }
                    }
                } else if (self.colorize)(c, abs).is_some() {
                    self.spot(r.center());
                }
                abs.pop();
            }
        }
        fn spot(&mut self, p: (f64, f64)) {
            if self.seen.insert(self.key(p)) {
                self.out.push(p);
            }
        }
    }
    let mut s = Spots { cover, cell, memo, colorize, hidden, seen: Default::default(), out: Vec::new(), looked: 0 };
    s.walk(node, rect, from, abs);
    s.out
}

/// Size-weighted average colour of a folder's children from index `from` on, looking at the
/// largest few at each level (`budget` limits the total work). `abs` is the folder's path from
/// the scan root; it's restored before returning. None if nothing in there has a colour (all
/// filtered out).
fn blend_of(node: &Node, from: usize, abs: &mut Vec<u32>, colorize: &(dyn Fn(&Node, &[u32]) -> Option<(Color32, Color32)> + Send + Sync), depth: u32, budget: &mut i32) -> Option<Color32> {
    let mut acc = [0.0f64; 3];
    let mut weight = 0.0;
    for (i, c) in node.children.iter().enumerate().skip(from).take(8) {
        if *budget <= 0 || c.size == 0 {
            break;
        }
        *budget -= 1;
        abs.push(i as u32);
        let col = if c.is_dir {
            if depth < 4 { blend_of(c, 0, abs, colorize, depth + 1, budget) } else { None }
        } else {
            colorize(c, abs).map(|p| p.0)
        };
        abs.pop();
        if let Some(col) = col {
            let w = c.size as f64;
            acc[0] += col.r() as f64 * w;
            acc[1] += col.g() as f64 * w;
            acc[2] += col.b() as f64 * w;
            weight += w;
        }
    }
    (weight > 0.0).then(|| Color32::from_rgb((acc[0] / weight) as u8, (acc[1] / weight) as u8, (acc[2] / weight) as u8))
}

/// CPU fallback: render tiles into an image covering `cover` (the original per-pixel cushion
/// shading, without the interactive effects).
fn rasterize(cover: &DRect, ppu: f64, bg: Color32, tiles: &[RasterTile], paint: &Paint) -> ColorImage {
    let mut s = ppu;
    let max_px = 16.0e6;
    if cover.w() * cover.h() * s * s > max_px {
        s = (max_px / (cover.w() * cover.h())).sqrt();
    }
    let w = ((cover.w() * s).ceil() as usize).clamp(1, 8192);
    let h = ((cover.h() * s).ceil() as usize).clamp(1, 8192);
    let mut px = vec![bg; w * h];
    let look = &paint.look;
    let (ambient, diffuse, light) = (look.ambient, look.diffuse, look.sun);
    for t in tiles {
        let x0 = (((t.rect.x0 - cover.x0) * s).round().max(0.0) as usize).min(w);
        let x1 = (((t.rect.x1 - cover.x0) * s).round().max(0.0) as usize).min(w);
        let y0 = (((t.rect.y0 - cover.y0) * s).round().max(0.0) as usize).min(h);
        let y1 = (((t.rect.y1 - cover.y0) * s).round().max(0.0) as usize).min(h);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        if t.flat {
            for y in y0..y1 {
                px[y * w + x0..y * w + x1].fill(t.color);
            }
            continue;
        }
        let [s0, s1, s2, s3] = t.surface;
        // Gradient per pixel index: g = a * i + b.
        let (ax, bx) = ((2.0 * s0 / s) as f32, (2.0 * s0 * (cover.x0 + 0.5 / s) + s2) as f32);
        let (ay, by) = ((2.0 * s1 / s) as f32, (2.0 * s1 * (cover.y0 + 0.5 / s) + s3) as f32);
        let g = look.gain;
        let (cr, cg, cb) = (t.color.r() as f32 * g, t.color.g() as f32 * g, t.color.b() as f32 * g);
        for y in y0..y1 {
            let ny = -(ay * y as f32 + by);
            for x in x0..x1 {
                let nx = -(ax * x as f32 + bx);
                let cosa = ((nx * light[0] + ny * light[1] + light[2]) / (nx * nx + ny * ny + 1.0).sqrt()).clamp(0.0, 1.0);
                let f = ambient + diffuse * cosa;
                px[y * w + x] = Color32::from_rgb((cr * f).min(255.0) as u8, (cg * f).min(255.0) as u8, (cb * f).min(255.0) as u8);
            }
        }
        if let Some(gc) = look.grid {
            px[y0 * w + x0..y0 * w + x1].fill(gc);
            for y in y0..y1 {
                px[y * w + x0] = gc;
            }
        }
    }
    ColorImage::new([w, h], px)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint() -> Paint {
        Paint {
            key: 1,
            colorize: Arc::new(|_, _| Some((Color32::from_rgb(90, 140, 220), Color32::from_rgb(90, 140, 220)))),
            hidden: None,
            cushion: Cushion { height: 0.38, scale: 0.91 },
            background: Color32::from_gray(45),
            look: Look { ambient: 0.1, diffuse: 0.9, gain: 1.0, sun: [0.0, 0.0, 1.0], grid: None, lighting: false, light_level: 1.0, bevel: false },
        }
    }

    #[test]
    fn hidden_matches_are_marked() {
        // A big tree seen whole: most files are too small to draw. With a `hidden` hook every
        // blended patch takes its colour, and slivers get a marker at least MARK_PT across.
        let tree = Arc::new(treemap::synthetic(50_000, 4));
        let world = DRect { x0: 0.0, y0: 0.0, x1: 400.0, y1: 300.0 };
        let mark = Color32::from_rgb(250, 10, 10);
        let plain = build_set(&req(&tree, world, world, 1.0)).unwrap();
        assert!(!plain.instances.iter().any(|i| i.color == mark.to_array()));
        let mut r = req(&tree, world, world, 1.0);
        r.paint.hidden = Some(Arc::new(move |_: &Node, _: &[u32], _: usize| Some(mark)));
        let set = build_set(&r).unwrap();
        let marked: Vec<_> = set.instances.iter().filter(|i| i.color == mark.to_array()).collect();
        assert!(!marked.is_empty(), "patches of hidden files take the mark colour");
        let dots = marked.iter().filter(|i| i.flags == tilegpu::FLAT).count();
        assert!(dots > 0, "slivers get markers");
        for d in marked.iter().filter(|i| i.flags == tilegpu::FLAT) {
            let [x0, y0, x1, y1] = d.rect;
            assert!((x1 - x0).min(y1 - y0) >= MARK_PT as f32 - 0.01, "{:?}", d.rect);
        }
        // Markers come after every tile, so they're drawn on top.
        let first_dot = set.instances.iter().position(|i| i.color == mark.to_array() && i.flags == tilegpu::FLAT).unwrap();
        assert!(set.instances[first_dot..].iter().all(|i| i.flags == tilegpu::FLAT));
    }

    #[test]
    fn tiny_match_in_big_folder_keeps_its_marker() {
        // A folder drawn large, holding big files that don't match and one tiny file that does:
        // the tiny file can't be drawn, and its folder's patch is covered by the big files, so
        // the marker must sit where the tiny file is (not only when the folder itself is small).
        let file = |name: &str, size: u64| Node { name: name.into(), size, files: 1, modified: 0, is_dir: false, children: Vec::new() };
        let mut kids: Vec<Node> = (0..6).map(|i| file(&format!("big{i}.bin"), 1 << 30)).collect();
        kids.push(file("tiny.onnx", 10));
        let size = kids.iter().map(|k| k.size).sum();
        let dir = Node { name: "d".into(), size, files: kids.len() as u64, modified: 0, is_dir: true, children: kids };
        let tree = Arc::new(Node { name: "root".into(), size, files: dir.files, modified: 0, is_dir: true, children: vec![dir] });
        let world = DRect { x0: 0.0, y0: 0.0, x1: 400.0, y1: 300.0 };
        let mark = Color32::from_rgb(250, 10, 10);
        let mut r = req(&tree, world, world, 1.0);
        r.paint.colorize = Arc::new(|n: &Node, _: &[u32]| n.name.ends_with(".onnx").then_some((Color32::WHITE, Color32::WHITE)));
        r.paint.hidden = Some(Arc::new(move |n: &Node, _: &[u32], from: usize| n.children.iter().skip(from).any(|c| c.name.ends_with(".onnx")).then_some(mark)));
        let set = build_set(&r).unwrap();
        let dots: Vec<_> = set.instances.iter().filter(|i| i.color == mark.to_array() && i.flags == tilegpu::FLAT).collect();
        assert_eq!(dots.len(), 1, "one marker for the one tiny match");
        // It sits on the tiny file's place in the layout.
        let rects = treemap::child_rects(&tree.children[0], &world, 0.0, None);
        let (tx, ty) = rects[6].center();
        let [x0, y0, x1, y1] = dots[0].rect;
        let o = set.origin;
        let (cx, cy) = (((x0 + x1) / 2.0) as f64 + o.0, ((y0 + y1) / 2.0) as f64 + o.1);
        assert!((cx - tx).abs() < 1.0 && (cy - ty).abs() < 1.0, "marker at ({cx}, {cy}), file at ({tx}, {ty})");
    }

    #[test]
    fn small_drawn_match_keeps_a_marker_until_it_is_as_big() {
        // A match drawn only a couple of pixels across would look like it vanished after its
        // marker went, so it keeps one until its tile is MARK_PT across.
        let file = |name: &str, size: u64| Node { name: name.into(), size, files: 1, modified: 0, is_dir: false, children: Vec::new() };
        let kids = vec![file("big.bin", 120_000 * 1000 - 4000), file("small.onnx", 4000)];
        let size = kids.iter().map(|k| k.size).sum();
        let tree = Arc::new(Node { name: "root".into(), size, files: 2, modified: 0, is_dir: true, children: kids });
        let world = DRect { x0: 0.0, y0: 0.0, x1: 400.0, y1: 300.0 };
        let mark = Color32::from_rgb(250, 10, 10);
        let tile = Color32::from_rgb(10, 250, 10);
        let markers = |ppu: f64| {
            let small = treemap::child_rects(&tree, &world, 0.0, None)[1];
            let (cx, cy) = small.center();
            let cover = DRect { x0: cx - 200.0 / ppu, y0: cy - 150.0 / ppu, x1: cx + 200.0 / ppu, y1: cy + 150.0 / ppu };
            let mut r = req(&tree, world, cover, ppu);
            r.paint.colorize = Arc::new(move |n: &Node, _: &[u32]| n.name.ends_with(".onnx").then_some((tile, tile)));
            r.paint.hidden = Some(Arc::new(move |_: &Node, _: &[u32], _: usize| Some(mark)));
            let set = build_set(&r).unwrap();
            assert!(set.instances.iter().any(|i| i.color == tile.to_array()), "the small file is drawn at {ppu}");
            set.instances.iter().filter(|i| i.flags == tilegpu::FLAT && i.color != Color32::from_gray(24).to_array() && i.color != r.paint.background.to_array()).count()
        };
        // A strip 4 px² in area at 1:1 (thinner than a pixel), and about 7 px wide at 500:1.
        assert_eq!(markers(1.0), 1);
        assert_eq!(markers(500.0), 0);
    }

    #[test]
    fn every_match_in_view_is_drawn_or_marked() {
        // Zoomed in anywhere, each matching file in view is either drawn as its own tile or
        // has a marker on it, however small it is.
        let tree = Arc::new(treemap::synthetic(60_000, 7));
        let never = AtomicBool::new(false);
        let is = |n: &Node| n.name.ends_with(".txt");
        let hits = Arc::new(crate::subset::hits(&tree, &is, &never).unwrap());
        let world = DRect { x0: 0.0, y0: 0.0, x1: 1600.0, y1: 900.0 };
        let (tile, mark) = (Color32::from_rgb(10, 250, 10), Color32::from_rgb(250, 10, 10));
        // Every match and where it lies.
        let mut found = Vec::new();
        fn walk(n: &Node, r: DRect, is: &dyn Fn(&Node) -> bool, out: &mut Vec<DRect>) {
            for (c, cr) in n.children.iter().zip(treemap::child_rects(n, &r, 0.0, None).iter()) {
                if c.is_dir { walk(c, *cr, is, out) } else if is(c) { out.push(*cr) }
            }
        }
        walk(&tree, world, &is, &mut found);
        assert!(found.len() > 1000);
        let mut checked = 0;
        for (k, z) in [1.0, 4.0, 16.0, 64.0, 256.0, 1024.0].into_iter().enumerate() {
            // Centre the view on a few different matches.
            for f in found.iter().skip(k * 37).step_by(found.len() / 5) {
                let (cx, cy) = f.center();
                let (w, h) = (1600.0 / z, 900.0 / z);
                let cover = DRect { x0: cx - w / 2.0, y0: cy - h / 2.0, x1: cx + w / 2.0, y1: cy + h / 2.0 };
                let mut r = req(&tree, world, cover, z);
                r.paint.colorize = Arc::new(move |n: &Node, _: &[u32]| n.name.ends_with(".txt").then_some((tile, tile)));
                let hh = hits.clone();
                r.paint.hidden = Some(Arc::new(move |_: &Node, abs: &[u32], from: usize| hh.at(abs).filter(|x| x.any_from(from)).map(|_| mark)));
                let set = build_set(&r).unwrap();
                let o = set.origin;
                let shown: Vec<DRect> = set
                    .instances
                    .iter()
                    .filter(|i| i.color == tile.to_array() || (i.color == mark.to_array() && i.flags == tilegpu::FLAT))
                    .map(|i| DRect { x0: i.rect[0] as f64 / z + o.0, y0: i.rect[1] as f64 / z + o.1, x1: i.rect[2] as f64 / z + o.0, y1: i.rect[3] as f64 / z + o.1 })
                    .collect();
                let inner = DRect { x0: cover.x0 + w * 0.05, y0: cover.y0 + h * 0.05, x1: cover.x1 - w * 0.05, y1: cover.y1 - h * 0.05 };
                for m in found.iter().filter(|m| m.intersects(&inner)) {
                    let (mx, my) = m.center();
                    let slack = MARK_PT / z;
                    let near = DRect { x0: mx - slack, y0: my - slack, x1: mx + slack, y1: my + slack };
                    assert!(shown.iter().any(|s| s.intersects(&near) || s.intersects(m)), "zoom {z}: match at {m:?} neither drawn nor marked");
                    checked += 1;
                }
            }
        }
        assert!(checked > 100, "{checked}");
    }

    fn req(tree: &Arc<Node>, world: DRect, cover: DRect, ppu: f64) -> Req {
        Req {
            scene: 1,
            kind: Kind::Detail,
            id: 1,
            tree: tree.clone(),
            root: Vec::new(),
            world,
            cover,
            ppu,
            ppp: 1.0,
            paint: paint(),
            memo: Arc::new(Memo::default()),
            raster: false,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The GPU transform puts every tile exactly where the camera says it is, at any zoom.
    #[test]
    fn instance_transform_matches_camera() {
        let tree = Arc::new(treemap::synthetic(20_000, 9));
        let world = DRect::from_size(1000.0, 700.0);
        let feel = Feel::default();
        let mut cam = Camera::new(world, (1000.0, 700.0));
        let bounds = Rect::from_min_size(pos2(10.0, 20.0), egui::vec2(1000.0, 700.0));
        for (k, at) in [(3.0, (500.0, 350.0)), (40.0, (123.0, 456.0)), (900.0, (800.0, 100.0))] {
            cam.zoom_by((k as f64).ln(), at, &feel);
            while cam.update(1.0 / 60.0, &feel) {}
            let view = cam.view();
            let ppu = 1000.0 / view.w();
            let cover = view.grow(0.3, 0.3).intersect(&world);
            let set = build_set(&req(&tree, world, cover, ppu)).unwrap();
            assert!(set.instances.len() > 1);
            let mv = MapView::new(Backend::None);
            let look = Look { ambient: 0.1, diffuse: 0.9, gain: 1.0, sun: [0.0, 0.0, 1.0], grid: None, lighting: false, light_level: 1.0, bevel: false };
            let u = mv.uniforms(&set, &view, &world, &view, (1000.0, 700.0), 1.0, &look, false);
            let mut checked = 0;
            treemap::build(&tree, world, &view, Detail { unit: 1.0 / ppu }, paint().cushion, None, &mut |l| {
                if l.node.is_dir || checked > 200 {
                    return true;
                }
                let r = set.local_rect(&l.rect);
                // Instances are clipped to the set's clip box; compare that part.
                let screen = rect_on_screen(&view, bounds, &l.rect.intersect(&set.clip));
                // ndc -> screen
                let sx = |n: f32| bounds.min.x + (n + 1.0) / 2.0 * bounds.width();
                let sy = |n: f32| bounds.min.y + (1.0 - n) / 2.0 * bounds.height();
                let nx0 = r[0] * u.xform[0] + u.xform[2];
                let ny0 = r[1] * u.xform[1] + u.xform[3];
                if screen.min.x.abs() < 5000.0 && screen.min.y.abs() < 5000.0 {
                    assert!((sx(nx0) - screen.min.x).abs() < 0.05, "x {} vs {}", sx(nx0), screen.min.x);
                    assert!((sy(ny0) - screen.min.y).abs() < 0.05, "y {} vs {}", sy(ny0), screen.min.y);
                    checked += 1;
                }
                true
            });
            assert!(checked > 0);
        }
    }

    /// Cushion gradients survive the change to local coordinates even at deep zoom.
    #[test]
    fn local_gradients_match_world_gradients() {
        let tree = Arc::new(treemap::synthetic(5_000, 4));
        let world = DRect::from_size(1000.0, 700.0);
        let view = DRect::new(612.0, 333.0, 612.01, 333.007);
        let ppu = 1000.0 / view.w();
        let set = build_set(&req(&tree, world, view.grow(0.3, 0.3), ppu)).unwrap();
        let mut n = 0;
        treemap::build(&tree, world, &view.grow(0.3, 0.3), Detail { unit: 1.0 / ppu }, paint().cushion, None, &mut |l| {
            if l.node.is_dir {
                return true;
            }
            let (cx, cy) = l.rect.center();
            let [s0, s1, s2, s3] = l.surface;
            let gx = 2.0 * s0 * cx + s2;
            let gy = 2.0 * s1 * cy + s3;
            let i = set.instances.iter().find(|i| i.rect == set.local_rect(&l.rect)).unwrap();
            let u = ((cx - set.origin.0) * ppu) as f32;
            let v = ((cy - set.origin.1) * ppu) as f32;
            let lx = i.grad[0] * u + i.grad[1];
            let ly = i.grad[2] * v + i.grad[3];
            assert!((lx as f64 - gx).abs() < 1e-3 * (1.0 + gx.abs()), "{lx} vs {gx}");
            assert!((ly as f64 - gy).abs() < 1e-3 * (1.0 + gy.abs()), "{ly} vs {gy}");
            n += 1;
            true
        });
        assert!(n > 0);
    }

    #[test]
    fn cancelled_build_returns_nothing() {
        let tree = Arc::new(treemap::synthetic(200_000, 2));
        let world = DRect::from_size(1000.0, 700.0);
        let r = req(&tree, world, world, 1.0);
        r.cancel.store(true, Ordering::Relaxed);
        assert!(build_set(&r).is_none());
    }
}

/// Before/after timings on a synthetic scan of 3.5 million files. Run with
/// `cargo test --release benchmark -- --ignored --nocapture`.
#[cfg(test)]
mod bench {
    use super::*;
    use crate::app::Shading;
    use crate::settings::Settings;
    use std::time::Instant;

    fn ms(t: Instant) -> f64 {
        t.elapsed().as_secs_f64() * 1000.0
    }

    /// The original renderer: lay out the whole folder and shade every pixel on the UI thread,
    /// once per zoom step (as `render_treemap` did).
    fn old_render(root: &Node, w: usize, h: usize, shade: &Shading) -> (usize, Vec<Color32>) {
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(w as f32, h as f32));
        let tiles = treemap::layout(root, bounds, 0.38, 0.91);
        let mut px = vec![Color32::from_gray(45); w * h];
        for (i, t) in tiles.iter().enumerate() {
            if tiles.get(i + 1).is_some_and(|nx| nx.path.len() > t.path.len()) {
                continue;
            }
            let n = root.at(&t.path).unwrap();
            if n.is_dir {
                continue;
            }
            shade.paint(&mut px, w, t, Color32::from_rgb(90, 140, 220));
        }
        (tiles.len(), px)
    }

    #[test]
    #[ignore]
    fn benchmark() {
        let t = Instant::now();
        let tree = Arc::new(treemap::synthetic(3_500_000, 5));
        println!("synthetic tree: {} files, built in {:.0} ms", tree.files, ms(t));
        let (w, h) = (1600usize, 900usize);
        let world = DRect::from_size(w as f64, h as f64);
        let settings = Settings::default();
        let shade = Shading::new(&settings);
        let sub = tree.children.iter().position(|c| c.is_dir).unwrap() as u32;

        println!("\nBEFORE (CPU layout + per-pixel shading on the UI thread, {w}x{h} px)");
        for (label, path) in [("whole scan", vec![]), ("largest folder", vec![sub])] {
            let root = tree.at(&path).unwrap();
            let mut best = f64::MAX;
            let mut n = 0;
            for _ in 0..3 {
                let t = Instant::now();
                let (tiles, px) = old_render(root, w, h, &shade);
                best = best.min(ms(t));
                n = tiles;
                std::hint::black_box(px);
            }
            println!("  zoom step into {label:<15} {best:7.1} ms blocking ({n} tiles)");
        }
        println!("  hover / mouse move              ~0 ms (no lighting; cached image)");

        println!("\nAFTER");
        let paint = Paint {
            key: 1,
            colorize: Arc::new(|_, _| Some((Color32::from_rgb(90, 140, 220), Color32::from_rgb(90, 140, 220)))),
            hidden: None,
            cushion: Cushion { height: 0.38, scale: 0.91 },
            background: Color32::from_gray(45),
            look: Look { ambient: 0.13, diffuse: 0.87, gain: 1.0, sun: [0.0, 0.0, 1.0], grid: None, lighting: true, light_level: 1.0, bevel: true },
        };
        let memo = Arc::new(Memo::default());
        let req = |cover: DRect, ppu: f64| Req {
            scene: 1,
            kind: Kind::Detail,
            id: 1,
            tree: tree.clone(),
            root: Vec::new(),
            world,
            cover,
            ppu,
            ppp: 1.0,
            paint: paint.clone(),
            memo: memo.clone(),
            raster: false,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        println!("  Tile builds (background thread; the UI keeps drawing the previous tiles meanwhile):");
        for zoom in [1.0f64, 1.6, 4.0, 30.0, 1000.0, 100_000.0] {
            let view = DRect::new(700.0, 400.0, 700.0 + w as f64 / zoom, 400.0 + h as f64 / zoom).intersect(&world);
            let view = if zoom == 1.0 { world } else { view };
            let cover = view.grow(0.3, 0.3).intersect(&world);
            let mut best = f64::MAX;
            let mut n = 0;
            for _ in 0..3 {
                let t = Instant::now();
                let set = build_set(&req(cover, w as f64 / view.w())).unwrap();
                best = best.min(ms(t));
                n = set.instances.len();
            }
            println!("    at zoom x{zoom:<8} {best:7.1} ms  ({n} tiles)");
        }

        // Per-frame UI-thread work while zooming: camera step + transforms for the GPU.
        let feel = Feel::default();
        let mut cam = Camera::new(world, (w as f64, h as f64));
        let set = build_set(&req(world, 1.0)).unwrap();
        let mv = MapView::new(Backend::None);
        let look = paint.look;
        let frames = 100_000;
        let t = Instant::now();
        for k in 0..frames {
            if k % 10 == 0 {
                cam.zoom_by(if (k / 500) % 2 == 0 { camera::NOTCH } else { -camera::NOTCH }, (800.0, 450.0), &feel);
            }
            cam.update(1.0 / 120.0, &feel);
            let v = cam.view();
            std::hint::black_box(mv.uniforms(&set, &v, &world, &v, (w as f64, h as f64), 1.0, &look, true));
        }
        println!("  Per frame on the UI thread while zooming/panning: {:.2} µs", ms(t) * 1000.0 / frames as f64);

        // Mouse move: hit test the tile under the pointer.
        let mut hits = HitCache::default();
        let mut cold = 0.0;
        let t = Instant::now();
        let moves = 2000;
        for k in 0..moves {
            let (x, y) = (100.0 + (k as f64 * 0.7) % 1400.0, 100.0 + (k as f64 * 0.37) % 700.0);
            let tk = Instant::now();
            std::hint::black_box(hits.hit(&tree, world, x, y, Detail { unit: 1.0 }, Some(&memo)));
            if k == 0 {
                cold = ms(tk);
            }
        }
        println!("  Mouse move (hit test, lighting is on the GPU): first {:.2} ms, then {:.1} µs average", cold, ms(t) * 1000.0 / moves as f64);
    }
}

