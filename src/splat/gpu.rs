//! Real Gaussian splatting on the graphics chip, with wgpu-3dgs-viewer (MIT or Apache-2.0).
//!
//! One worker thread owns its own wgpu device, separate from the window's: on Windows the
//! window is drawn with OpenGL, which can't run the compute shaders the depth sort needs, and
//! a device of our own can ask for the adapter's real buffer limits. Pictures are drawn
//! off-screen and copied back for the preview as an egui texture.
//!
//! The UI hands over the latest wanted picture ([`want`]); older wishes are dropped. Errors
//! from the GPU are caught (error scopes, an uncaptured-error handler, and a panic guard around
//! the whole job); after one, the preview falls back to drawing points on the processor. The
//! device is let go after a minute without work.

use super::{Frame as Axes, Scene, View};
use eframe::wgpu;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use wgpu_3dgs_viewer as gs;
use gs::core::glam::{Mat4, Quat, UVec2, U8Vec4, Vec3};

/// A picture of a scene.
pub struct Picture {
    /// Which wish it answers.
    pub id: u64,
    pub size: [usize; 2],
    /// Premultiplied RGBA.
    pub rgba: Vec<u8>,
    /// Time to draw and copy back.
    pub ms: f32,
    /// Splats drawn (fewer than the scene's when it had to be thinned to fit).
    pub drawn: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Not tried yet, or starting.
    Unknown,
    /// Working, on this adapter ("Apple M2 (Metal)").
    Ready(String),
    /// Can't be used here; the preview draws points instead.
    Unavailable(String),
}

/// Colours for the splats in place of their own (a false-colour look), one per splat.
pub type Paint = Arc<Vec<[u8; 4]>>;

struct Wish {
    id: u64,
    scene: Arc<Scene>,
    paint: Option<Paint>,
    view: View,
    size: [u32; 2],
    ctx: Option<eframe::egui::Context>,
}

struct Shared {
    wish: Mutex<Option<Wish>>,
    ready: Condvar,
    done: Mutex<Option<Picture>>,
    status: Mutex<Status>,
    /// When the job being drawn started; a job that runs past [`STUCK`] means the driver hung.
    busy_since: Mutex<Option<Instant>>,
    /// Pictures wanted by thumbnails, drawn when the preview wants nothing, each answered on
    /// its own channel. Guarded by `wish`'s lock too, so the worker never misses one.
    side: Mutex<std::collections::VecDeque<(Wish, std::sync::mpsc::SyncSender<Picture>)>>,
}

/// How long one picture (with opening the device and loading the scene) may take before the
/// graphics chip is given up on for this run, and the preview draws points instead.
const STUCK: Duration = Duration::from_secs(30);

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
/// What the worker is doing, for the message when the driver stops answering.
static STAGE: Mutex<&'static str> = Mutex::new("starting");

fn stage(what: &'static str) {
    *STAGE.lock().unwrap() = what;
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn shared() -> &'static Arc<Shared> {
    SHARED.get_or_init(|| {
        let s = Arc::new(Shared { wish: Mutex::new(None), ready: Condvar::new(), done: Mutex::new(None), status: Mutex::new(Status::Unknown), busy_since: Mutex::new(None), side: Mutex::new(Default::default()) });
        let w = s.clone();
        let _ = std::thread::Builder::new().name("splat-gpu".into()).spawn(move || worker(w));
        s
    })
}

pub fn status() -> Status {
    let s = shared();
    if s.busy_since.lock().unwrap().is_some_and(|t| t.elapsed() > STUCK) {
        let mut st = s.status.lock().unwrap();
        if !matches!(*st, Status::Unavailable(_)) {
            *st = Status::Unavailable(format!("The graphics chip didn't answer in time ({})", STAGE.lock().unwrap()));
        }
    }
    s.status.lock().unwrap().clone()
}

/// Ask for a picture of `scene` from `view`, `size` pixels. Returns the wish's id; the
/// picture comes from [`take`] when it's ready, and `ctx` is asked to repaint then.
pub fn want(scene: Arc<Scene>, paint: Option<Paint>, view: View, size: [u32; 2], ctx: Option<eframe::egui::Context>) -> u64 {
    let s = shared();
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    *s.wish.lock().unwrap() = Some(Wish { id, scene, paint, view, size, ctx });
    s.ready.notify_one();
    id
}

/// The newest finished picture, if any.
pub fn take() -> Option<Picture> {
    shared().done.lock().unwrap().take()
}

/// Wait for the picture of wish `id` (for tests and thumbnails), up to `limit`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn wait_for(id: u64, limit: Duration) -> Option<Picture> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Some(p) = take() {
            if p.id >= id {
                return Some(p);
            }
        }
        if matches!(status(), Status::Unavailable(_)) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

/// Thumbnail pictures waiting at most; more are drawn with points instead.
const SIDE_MAX: usize = 8;

/// Draw a picture for a thumbnail or a saved picture, waiting up to `limit`. The preview's pictures go first, and
/// the preview's scene stays loaded; `None` when the graphics chip can't be used or is busy for
/// too long (the caller draws points instead).
pub fn picture_now(scene: Arc<Scene>, paint: Option<Paint>, view: View, size: [u32; 2], limit: Duration) -> Option<Picture> {
    if matches!(status(), Status::Unavailable(_)) {
        return None;
    }
    let s = shared();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    {
        let _w = s.wish.lock().unwrap();
        let mut side = s.side.lock().unwrap();
        if side.len() >= SIDE_MAX {
            return None;
        }
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        side.push_back((Wish { id, scene, paint, view, size, ctx: None }, tx));
        s.ready.notify_one();
    }
    rx.recv_timeout(limit).ok()
}

/// The two GPU layouts we use: with view-dependent colour (half precision, 128 bytes a splat)
/// and without (32 bytes).
type ShPod = gs::core::GaussianPodWithShHalfCov3dHalfConfigs;
type FlatPod = gs::core::GaussianPodWithShNoneCov3dHalfConfigs;

enum Loaded {
    Sh(gs::Viewer<ShPod>),
    Flat(gs::Viewer<FlatPod>),
}

impl Loaded {
    fn update_camera(&mut self, q: &wgpu::Queue, cam: &Orbit, size: UVec2) {
        match self {
            Loaded::Sh(v) => v.update_camera(q, cam, size),
            Loaded::Flat(v) => v.update_camera(q, cam, size),
        }
    }

    fn set_display(&mut self, q: &wgpu::Queue, display: u8, degree: gs::core::GaussianShDegree) {
        let std_dev = gs::core::GaussianMaxStdDev::new(3.0).unwrap();
        let mode = display_mode(display);
        match self {
            Loaded::Sh(v) => v.update_gaussian_transform(q, 1.0, mode, degree, false, std_dev),
            Loaded::Flat(v) => v.update_gaussian_transform(q, 1.0, mode, degree, false, std_dev),
        }
    }

    fn render(&self, enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        match self {
            Loaded::Sh(v) => v.render(enc, view),
            Loaded::Flat(v) => v.render(enc, view),
        }
    }
}

/// The camera, matching the processor drawing in `Scene::draw` exactly.
struct Orbit {
    view: Mat4,
    fov: f32,
    near: f32,
    far: f32,
}

impl gs::CameraTrait for Orbit {
    fn view(&self) -> Mat4 {
        self.view
    }

    fn projection(&self, aspect: f32) -> Mat4 {
        #[allow(deprecated)] // the same 0..1 depth projection under its new name in later glam
        Mat4::perspective_rh(self.fov, aspect, self.near, self.far)
    }
}

fn orbit(scene: &Scene, v: &View) -> Orbit {
    let (_, r) = super::framing(scene);
    let m = v.rotation();
    let dist = v.distance(r);
    // Rows of the rotation are the camera's axes; glam is column-major.
    let rot = Mat4::from_cols_array(&[m[0][0], m[1][0], m[2][0], 0.0, m[0][1], m[1][1], m[2][1], 0.0, m[0][2], m[1][2], m[2][2], 0.0, 0.0, 0.0, 0.0, 1.0]);
    let view = Mat4::from_translation(Vec3::new(-v.pan[0] * r, -v.pan[1] * r, -dist)) * rot;
    Orbit { view, fov: v.fov_radians(), near: (dist * 0.005).max(1e-4), far: dist * 400.0 + r * 50.0 }
}

/// Splats as the viewer's own type, made one at a time from the scene (no second copy).
struct Splats {
    scene: Arc<Scene>,
    paint: Option<Paint>,
    step: usize,
}

impl FromIterator<gs::core::Gaussian> for Splats {
    fn from_iter<T: IntoIterator<Item = gs::core::Gaussian>>(_: T) -> Self {
        // The viewer never builds one; only reads.
        unreachable!("Splats are only read")
    }
}

impl gs::core::IterGaussian for Splats {
    fn iter_gaussian(&self) -> impl ExactSizeIterator<Item = gs::core::Gaussian> + '_ {
        let s = &*self.scene;
        let rest = super::sh_rest(s.sh_degree);
        let has_sh = !s.sh.is_empty() && self.paint.is_none();
        let colours: &[[u8; 4]] = match &self.paint {
            Some(p) if p.len() == s.len() => p,
            _ => &s.color,
        };
        (0..s.len()).step_by(self.step).map(move |i| {
            let mut sh = [Vec3::ZERO; 15];
            if has_sh {
                let base = i * rest * 3;
                for (k, slot) in sh.iter_mut().enumerate().take(rest.min(15)) {
                    let at = base + k * 3;
                    *slot = Vec3::new(s.sh[at].to_f32(), s.sh[at + 1].to_f32(), s.sh[at + 2].to_f32());
                }
            }
            let r = s.rot[i];
            let c = colours[i];
            gs::core::Gaussian {
                rot: Quat::from_xyzw(r[0], r[1], r[2], r[3]),
                pos: Vec3::from_array(s.pos[i]),
                color: U8Vec4::new(c[0], c[1], c[2], c[3]),
                sh,
                scale: Vec3::from_array(s.scale[i]),
            }
        })
    }
}

struct Device {
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
    max_binding: u64,
    lost: Arc<AtomicBool>,
    failed: Arc<Mutex<Option<String>>>,
}

fn open_device() -> Result<Device, String> {
    let inst = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))
        .map_err(|e| format!("No graphics adapter that can draw splats ({e})"))?;
    let info = adapter.get_info();
    let limits = adapter.limits();
    if !adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS) {
        return Err(format!("{} can't run compute shaders", info.name));
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("splats"), required_limits: limits.clone(), ..Default::default() }))
        .map_err(|e| format!("The graphics adapter refused ({e})"))?;
    let failed = Arc::new(Mutex::new(None));
    let f = failed.clone();
    device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
        *f.lock().unwrap() = Some(e.to_string());
    }));
    let lost = Arc::new(AtomicBool::new(false));
    let l = lost.clone();
    device.set_device_lost_callback(move |_, _| l.store(true, Ordering::Relaxed));
    let name = format!("{} ({:?})", info.name.trim(), info.backend);
    Ok(Device { device, queue, name, max_binding: limits.max_storage_buffer_binding_size.min(limits.max_buffer_size), lost, failed })
}

struct Current {
    scene: Arc<Scene>,
    paint: Option<Paint>,
    /// How the splats are drawn now (see [`super::inspect::Look::display`]).
    display: u8,
    degree: gs::core::GaussianShDegree,
    loaded: Loaded,
    drawn: usize,
}

fn worker(s: Arc<Shared>) {
    let mut dev: Option<Device> = None;
    let mut cur: Option<Current> = None;
    loop {
        let mut answer = None;
        let wish = {
            let mut w = s.wish.lock().unwrap();
            loop {
                if let Some(x) = w.take() {
                    break Some(x);
                }
                if let Some((x, tx)) = s.side.lock().unwrap().pop_front() {
                    answer = Some(tx);
                    break Some(x);
                }
                let (g, t) = s.ready.wait_timeout(w, Duration::from_secs(60)).unwrap();
                w = g;
                if t.timed_out() && w.is_none() {
                    break None;
                }
            }
        };
        let Some(wish) = wish else {
            // A minute idle: let the graphics memory go.
            cur = None;
            dev = None;
            continue;
        };
        if matches!(*s.status.lock().unwrap(), Status::Unavailable(_)) {
            continue;
        }
        let ctx = wish.ctx.clone();
        *s.busy_since.lock().unwrap() = Some(Instant::now());
        // A thumbnail's scene is loaded beside the preview's and let go straight after (unless
        // it is the preview's, as for a saved picture of the view).
        let mut aside = None;
        let same = cur.as_ref().is_some_and(|c| c.holds(&wish.scene, &wish.paint));
        let slot = if answer.is_some() && !same { &mut aside } else { &mut cur };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| draw(&mut dev, slot, &wish)));
        drop(aside);
        *s.busy_since.lock().unwrap() = None;
        if matches!(*s.status.lock().unwrap(), Status::Unavailable(_)) {
            // Given up on while it was stuck; stay with points.
            continue;
        }
        let outcome = match result {
            Ok(r) => r,
            Err(_) => Err("The splat renderer stopped unexpectedly".to_string()),
        };
        match outcome {
            Ok(pic) => {
                if let Some(d) = &dev {
                    *s.status.lock().unwrap() = Status::Ready(d.name.clone());
                }
                match answer {
                    Some(tx) => {
                        let _ = tx.try_send(pic);
                    }
                    None => *s.done.lock().unwrap() = Some(pic),
                }
            }
            Err(e) => {
                cur = None;
                dev = None;
                *s.status.lock().unwrap() = Status::Unavailable(e);
            }
        }
        if let Some(c) = ctx {
            c.request_repaint();
        }
    }
}

fn draw(dev: &mut Option<Device>, cur: &mut Option<Current>, wish: &Wish) -> Result<Picture, String> {
    let t = Instant::now();
    if dev.as_ref().is_some_and(|d| d.lost.load(Ordering::Relaxed)) {
        *dev = None;
        *cur = None;
    }
    if dev.is_none() {
        stage("opening the device");
        *dev = Some(open_device()?);
    }
    let d = dev.as_ref().unwrap();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    if cur.as_ref().is_none_or(|c| !c.holds(&wish.scene, &wish.paint)) {
        *cur = None; // free the old scene's buffers first
        stage("loading the scene");
        *cur = Some(load(d, &wish.scene, wish.paint.clone(), format)?);
    }
    let c = cur.as_mut().unwrap();
    let [w, h] = [wish.size[0].clamp(16, 4096), wish.size[1].clamp(16, 4096)];
    stage("drawing");
    let scope = d.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let oom = d.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    c.loaded.update_camera(&d.queue, &orbit(&wish.scene, &wish.view), UVec2::new(w, h));
    let display = wish.view.look.display();
    if display != c.display {
        c.display = display;
        c.loaded.set_display(&d.queue, display, c.degree);
    }
    let tex = d.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("splat picture"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let tv = tex.create_view(&Default::default());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = d.device.create_buffer(&wgpu::BufferDescriptor { label: Some("splat readback"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = d.device.create_command_encoder(&Default::default());
    {
        let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &tv, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            ..Default::default()
        });
    }
    c.loaded.render(&mut enc, &tv);
    enc.copy_texture_to_buffer(tex.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } }, tex.size());
    d.queue.submit([enc.finish()]);
    stage("checking for errors");
    if let Some(e) = pollster::block_on(oom.pop()) {
        return Err(format!("Out of graphics memory ({e})"));
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        return Err(format!("The graphics driver refused the scene ({e})"));
    }
    stage("reading the picture back");
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    d.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("The graphics adapter stopped ({e})"))?;
    if let Some(e) = d.failed.lock().unwrap().take() {
        return Err(e);
    }
    let mapped = buf.slice(..).get_mapped_range().map_err(|e| format!("Couldn't read the picture back ({e:?})"))?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        rgba.extend_from_slice(&mapped[y * row as usize..y * row as usize + w as usize * 4]);
    }
    drop(mapped);
    buf.unmap();
    Ok(Picture { id: wish.id, size: [w as usize, h as usize], rgba, ms: t.elapsed().as_secs_f32() * 1000.0, drawn: c.drawn })
}

fn display_mode(display: u8) -> gs::core::GaussianDisplayMode {
    match display {
        1 => gs::core::GaussianDisplayMode::Ellipse,
        2 => gs::core::GaussianDisplayMode::Point,
        _ => gs::core::GaussianDisplayMode::Splat,
    }
}

impl Current {
    /// Whether this is the scene, with these colours, already on the graphics chip.
    fn holds(&self, scene: &Arc<Scene>, paint: &Option<Paint>) -> bool {
        Arc::ptr_eq(&self.scene, scene)
            && match (&self.paint, paint) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

fn load(d: &Device, scene: &Arc<Scene>, paint: Option<Paint>, format: wgpu::TextureFormat) -> Result<Current, String> {
    let n = scene.len();
    if n == 0 || scene.rot.len() != n || scene.scale.len() != n {
        return Err("Nothing to draw".into());
    }
    let has_sh = !scene.sh.is_empty() && scene.sh_degree > 0 && paint.is_none();
    let per = if has_sh { std::mem::size_of::<ShPod>() } else { std::mem::size_of::<FlatPod>() } as u64;
    // Thin evenly until the splats fit one storage buffer.
    let step = (n as u64 * per).div_ceil(d.max_binding.max(1)).max(1) as usize;
    let splats = Splats { scene: scene.clone(), paint: paint.clone(), step };
    let drawn = n.div_ceil(step);
    let scope = d.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let oom = d.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    stage("loading the scene");
    let mut loaded = if has_sh {
        Loaded::Sh(gs::Viewer::<ShPod>::new(&d.device, format, &splats).map_err(|e| e.to_string())?)
    } else {
        Loaded::Flat(gs::Viewer::<FlatPod>::new(&d.device, format, &splats).map_err(|e| e.to_string())?)
    };
    // Trainer PLYs have y down and z forward; turn them to y up like SPZ, then move the
    // framed middle to the origin.
    let (c, _) = super::framing(scene);
    let rot = if scene.frame == Axes::RightDownForward { Quat::from_rotation_x(std::f32::consts::PI) } else { Quat::IDENTITY };
    let pos = -(rot * Vec3::from_array(c));
    let degree = gs::core::GaussianShDegree::new(if has_sh { scene.sh_degree.min(3) } else { 0 }).unwrap();
    let std_dev = gs::core::GaussianMaxStdDev::new(3.0).unwrap();
    match &mut loaded {
        Loaded::Sh(v) => {
            v.update_model_transform(&d.queue, pos, rot, Vec3::ONE);
            v.update_gaussian_transform(&d.queue, 1.0, gs::core::GaussianDisplayMode::Splat, degree, false, std_dev);
        }
        Loaded::Flat(v) => {
            v.update_model_transform(&d.queue, pos, rot, Vec3::ONE);
            v.update_gaussian_transform(&d.queue, 1.0, gs::core::GaussianDisplayMode::Splat, degree, false, std_dev);
        }
    }
    stage("checking the scene loaded");
    if let Some(e) = pollster::block_on(oom.pop()) {
        return Err(format!("Out of graphics memory for {} splats ({e})", drawn));
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        return Err(format!("The graphics driver refused the scene ({e})"));
    }
    Ok(Current { scene: scene.clone(), paint, display: 0, degree, loaded, drawn })
}
