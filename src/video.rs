//! Video playback in the preview panes: frames decoded by the system on a thread of their own,
//! scaled down for the pane, and shown in time; the sound plays through `media::Player`.
//!
//! - macOS: AVFoundation (AVAssetReader), through the Objective-C runtime.
//! - Windows: Media Foundation (IMFSourceReader).
//! - Linux and other systems: `ffmpeg`, when it's installed.
//!
//! Playback only ever shows the newest frame that's due; a frame that comes too late is dropped
//! rather than slowing the clock, so a slow machine shows fewer frames, not slower video.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// Frames are scaled to fit this many pixels across (the pane is rarely wider).
pub const MAX_W: u32 = 960;

/// What's known about a video once it's open.
#[derive(Clone, Copy, Debug, Default)]
pub struct Info {
    /// Shown size (after rotation), before scaling down.
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: Option<f64>,
}

/// One decoded frame: RGBA, `size` pixels.
pub struct Frame {
    pub pts: f64,
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

/// A decoder reading frames in order from where it was opened.
pub trait Decoder {
    fn next(&mut self) -> Option<Frame>;
}

/// Whether videos can play here (on Linux, only with ffmpeg installed).
pub fn supported() -> bool {
    imp::supported()
}

/// Open a video at `at` seconds.
fn open(path: &Path, at: f64) -> Result<(Box<dyn Decoder>, Info), String> {
    imp::open(path, at)
}

/// How a video is doing, for the UI.
#[derive(Clone, Default)]
pub struct State {
    pub playing: bool,
    pub ended: bool,
    pub position: f64,
    pub info: Option<Info>,
    pub error: Option<String>,
}

#[derive(Default)]
struct Shared {
    state: State,
    /// The newest frame due, and a count that changes with each new one.
    frame: Option<Frame>,
    serial: u64,
}

enum Cmd {
    Play,
    Pause,
    Seek(f64),
}

/// A video playing (or paused) on a thread of its own. Dropping it stops it.
pub struct Video {
    tx: mpsc::Sender<Cmd>,
    shared: Arc<Mutex<Shared>>,
    sound: Option<crate::media::Player>,
    pub path: PathBuf,
}

impl Video {
    /// Open a video and start playing it, with its sound or silently. `repaint` is called
    /// whenever a new frame is due.
    pub fn start(path: &Path, sound: bool, repaint: impl Fn() + Send + 'static) -> Video {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (p, s) = (path.to_path_buf(), shared.clone());
        let _ = std::thread::Builder::new().name("filescape-video".into()).spawn(move || run(&p, &rx, &s, &repaint));
        let _ = tx.send(Cmd::Play);
        // The sound: the system's player reads the same file (not on Windows yet, where the
        // only player at hand would open a window of its own).
        let sound = (sound && !cfg!(windows)).then(|| crate::media::Player::start(path));
        Video { tx, shared, sound, path: path.to_path_buf() }
    }

    pub fn state(&self) -> State {
        self.shared.lock().map(|s| s.state.clone()).unwrap_or_default()
    }

    /// The newest frame, if it's newer than `serial` (which is updated).
    pub fn take_frame(&self, serial: &mut u64) -> Option<Frame> {
        let mut s = self.shared.lock().ok()?;
        if s.serial == *serial {
            return None;
        }
        *serial = s.serial;
        s.frame.take()
    }

    pub fn play(&self) {
        let ended = self.state().ended;
        if let Some(s) = &self.sound {
            if ended {
                s.seek(0.0);
            }
            s.play();
        }
        let _ = self.tx.send(Cmd::Play);
    }

    pub fn pause(&self) {
        if let Some(s) = &self.sound {
            s.pause();
        }
        let _ = self.tx.send(Cmd::Pause);
    }

    pub fn seek(&self, secs: f64) {
        if let Some(s) = &self.sound {
            s.seek(secs);
        }
        let _ = self.tx.send(Cmd::Seek(secs));
    }
}

/// The playback thread: decode, wait until each frame is due, hand it over.
fn run(path: &Path, rx: &mpsc::Receiver<Cmd>, shared: &Mutex<Shared>, repaint: &dyn Fn()) {
    let set = |f: &mut dyn FnMut(&mut State)| {
        if let Ok(mut s) = shared.lock() {
            f(&mut s.state);
        }
    };
    let show = |f: Frame| {
        if let Ok(mut s) = shared.lock() {
            s.state.position = f.pts;
            s.frame = Some(f);
            s.serial += 1;
        }
        repaint();
    };
    let mut dec: Option<Box<dyn Decoder>> = None;
    let mut info: Option<Info> = None;
    let mut playing = false;
    let mut pos = 0.0;
    // When playback (re)started: the moment, and the time in the video then.
    let mut clock: Option<(Instant, f64)> = None;
    let mut pending: Option<Cmd> = None;
    loop {
        let cmd = match pending.take() {
            Some(c) => Some(c),
            None if playing => match rx.try_recv() {
                Ok(c) => Some(c),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            },
        };
        match cmd {
            Some(Cmd::Play) => {
                let ended = shared.lock().map(|s| s.state.ended).unwrap_or(false);
                if ended {
                    pos = 0.0;
                    dec = None;
                }
                playing = true;
                clock = None;
                set(&mut |s| {
                    s.playing = true;
                    s.ended = false;
                });
            }
            Some(Cmd::Pause) => {
                playing = false;
                set(&mut |s| s.playing = false);
                continue;
            }
            Some(Cmd::Seek(t)) => {
                pos = t.max(0.0);
                clock = None;
                match open(path, pos) {
                    Ok((mut d, i)) => {
                        info = Some(i);
                        set(&mut |s| {
                            s.info = Some(i);
                            s.ended = false;
                        });
                        // Paused: show the frame there at once.
                        if !playing && let Some(f) = d.next() {
                            pos = f.pts;
                            show(f);
                        }
                        dec = Some(d);
                    }
                    Err(e) => set(&mut |s| s.error = Some(e.clone())),
                }
                if !playing {
                    continue;
                }
            }
            None => {}
        }
        if !playing {
            continue;
        }
        if dec.is_none() {
            match open(path, pos) {
                Ok((d, i)) => {
                    dec = Some(d);
                    info = Some(i);
                    set(&mut |s| s.info = Some(i));
                }
                Err(e) => {
                    playing = false;
                    set(&mut |s| {
                        s.error = Some(e.clone());
                        s.playing = false;
                    });
                    continue;
                }
            }
        }
        let Some(f) = dec.as_mut().and_then(|d| d.next()) else {
            playing = false;
            dec = None;
            let end = info.and_then(|i| i.duration);
            set(&mut |s| {
                s.playing = false;
                s.ended = true;
                if let Some(d) = end {
                    s.position = d;
                }
            });
            repaint();
            continue;
        };
        let (t0, p0) = *clock.get_or_insert((Instant::now(), f.pts));
        let due = t0 + Duration::from_secs_f64((f.pts - p0).max(0.0));
        let now = Instant::now();
        if due > now {
            // Wait, still listening: a pause or seek drops this frame.
            match rx.recv_timeout(due - now) {
                Ok(c) => {
                    pending = Some(c);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        } else {
            // Late by more than two frames: skip it and catch up.
            let frame = 1.0 / info.map_or(30.0, |i| i.fps.max(1.0));
            if (now - due).as_secs_f64() > 2.0 * frame {
                pos = f.pts;
                continue;
            }
        }
        pos = f.pts;
        show(f);
    }
}

/// Fit `w`×`h` inside `MAX_W` across, keeping the shape, in even numbers.
pub fn scaled(w: u32, h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (2, 2);
    }
    let sw = w.min(MAX_W);
    let sh = ((h as f64 * sw as f64 / w as f64).round() as u32).max(2);
    (sw & !1, sh & !1)
}

/// Scale a frame of 4-byte pixels down to `dw`×`dh` (nearest pixel), swapping blue and red when
/// `bgra`. `row` is the source's bytes per row.
#[allow(clippy::too_many_arguments, dead_code)]
pub fn resample(src: &[u8], sw: usize, sh: usize, row: isize, start: isize, dw: usize, dh: usize, bgra: bool) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let sy = (y * sh / dh).min(sh - 1) as isize;
        let line = start + sy * row;
        for x in 0..dw {
            let sx = (x * sw / dw).min(sw - 1);
            let i = (line + sx as isize * 4) as usize;
            let o = (y * dw + x) * 4;
            let p = &src[i..i + 4];
            if bgra {
                out[o] = p[2];
                out[o + 1] = p[1];
                out[o + 2] = p[0];
            } else {
                out[o..o + 3].copy_from_slice(&p[..3]);
            }
            out[o + 3] = 255;
        }
    }
    out
}

// ---------- Linux and others: ffmpeg ----------

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::*;
    use std::io::Read;
    use std::process::{Child, Command, Stdio};

    fn find(prog: &str) -> Option<PathBuf> {
        std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(prog)).find(|p| p.is_file()))
    }

    pub fn supported() -> bool {
        find("ffmpeg").is_some() && find("ffprobe").is_some()
    }

    fn probe(path: &Path) -> Result<Info, String> {
        let probe = find("ffprobe").ok_or("Playing videos needs ffmpeg, which isn't installed")?;
        let out = Command::new(probe)
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,avg_frame_rate,r_frame_rate:stream_side_data=rotation:stream_tags=rotate:format=duration", "-of", "json"])
            .arg(path)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| "Couldn't read the video".to_string())?;
        let s = &v["streams"][0];
        let (w, h) = (s["width"].as_u64().unwrap_or(0) as u32, s["height"].as_u64().unwrap_or(0) as u32);
        if w == 0 || h == 0 {
            return Err("No picture in this file".into());
        }
        let rate = |k: &str| {
            let r = s[k].as_str().unwrap_or("0/1");
            let (n, d) = r.split_once('/').unwrap_or((r, "1"));
            let (n, d) = (n.parse::<f64>().unwrap_or(0.0), d.parse::<f64>().unwrap_or(1.0));
            if d > 0.0 { n / d } else { 0.0 }
        };
        let fps = [rate("avg_frame_rate"), rate("r_frame_rate")].into_iter().find(|f| (1.0..=240.0).contains(f)).unwrap_or(30.0);
        let rot = s["side_data_list"][0]["rotation"].as_f64().or_else(|| s["tags"]["rotate"].as_str().and_then(|r| r.parse().ok())).unwrap_or(0.0);
        let turned = (rot.abs() as i64 % 180) == 90;
        let duration = v["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok());
        Ok(Info { width: if turned { h } else { w }, height: if turned { w } else { h }, fps, duration })
    }

    struct Ffmpeg {
        child: Child,
        size: (u32, u32),
        at: f64,
        fps: f64,
        n: u64,
    }

    impl Decoder for Ffmpeg {
        fn next(&mut self) -> Option<Frame> {
            let (w, h) = self.size;
            let mut buf = vec![0u8; w as usize * h as usize * 4];
            self.child.stdout.as_mut()?.read_exact(&mut buf).ok()?;
            let pts = self.at + self.n as f64 / self.fps;
            self.n += 1;
            Some(Frame { pts, size: [w as usize, h as usize], rgba: buf })
        }
    }

    impl Drop for Ffmpeg {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    pub fn open(path: &Path, at: f64) -> Result<(Box<dyn Decoder>, Info), String> {
        let info = probe(path)?;
        let (w, h) = scaled(info.width, info.height);
        let ffmpeg = find("ffmpeg").ok_or("Playing videos needs ffmpeg, which isn't installed")?;
        let child = Command::new(ffmpeg)
            .args(["-nostdin", "-loglevel", "error", "-ss", &format!("{at:.3}"), "-i"])
            .arg(path)
            .args(["-an", "-sn", "-vf", &format!("scale={w}:{h}"), "-pix_fmt", "rgba", "-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok((Box::new(Ffmpeg { child, size: (w, h), at, fps: info.fps, n: 0 }), info))
    }
}

// ---------- macOS: AVFoundation ----------

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::{CString, c_char, c_void};
    use std::os::unix::ffi::OsStrExt;

    type Id = *mut c_void;
    type Sel = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CMTimeRange {
        start: CMTime,
        duration: CMTime,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "CoreMedia", kind = "framework")]
    unsafe extern "C" {
        fn CMSampleBufferGetPresentationTimeStamp(sbuf: Id) -> CMTime;
        fn CMSampleBufferGetImageBuffer(sbuf: Id) -> Id;
        fn CMTimeGetSeconds(t: CMTime) -> f64;
    }

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        static kCVPixelBufferPixelFormatTypeKey: Id;
        fn CVPixelBufferLockBaseAddress(buf: Id, flags: u64) -> i32;
        fn CVPixelBufferUnlockBaseAddress(buf: Id, flags: u64) -> i32;
        fn CVPixelBufferGetBaseAddress(buf: Id) -> *const u8;
        fn CVPixelBufferGetBytesPerRow(buf: Id) -> usize;
        fn CVPixelBufferGetWidth(buf: Id) -> usize;
        fn CVPixelBufferGetHeight(buf: Id) -> usize;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: Id);
    }

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
        #[cfg(target_arch = "x86_64")]
        fn objc_msgSend_stret();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    fn sel(s: &str) -> Sel {
        let c = CString::new(s).unwrap();
        // SAFETY: a valid NUL-terminated selector name.
        unsafe { sel_registerName(c.as_ptr()) }
    }

    fn class(s: &str) -> Id {
        let c = CString::new(s).unwrap();
        // SAFETY: a valid NUL-terminated class name.
        unsafe { objc_getClass(c.as_ptr()) }
    }

    /// objc_msgSend cast to a method's signature.
    macro_rules! send {
        ($ret:ty; $obj:expr, $sel:expr $(, $arg:expr => $t:ty)*) => {{
            let f: extern "C" fn(Id, Sel $(, $t)*) -> $ret = std::mem::transmute(objc_msgSend as *const ());
            f($obj, sel($sel) $(, $arg)*)
        }};
    }

    /// A method returning a struct bigger than 16 bytes (CMTime): through objc_msgSend_stret on
    /// Intel Macs; objc_msgSend handles it on Apple silicon.
    unsafe fn send_time(obj: Id, name: &str) -> CMTime {
        #[cfg(target_arch = "x86_64")]
        let f: extern "C" fn(Id, Sel) -> CMTime = unsafe { std::mem::transmute(objc_msgSend_stret as *const ()) };
        #[cfg(not(target_arch = "x86_64"))]
        let f: extern "C" fn(Id, Sel) -> CMTime = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        f(obj, sel(name))
    }

    pub fn supported() -> bool {
        true
    }

    struct Av {
        reader: Id,
        output: Id,
        size: (u32, u32),
    }

    // SAFETY: the decoder is made and used on the playback thread only; the objects are only
    // touched there.
    unsafe impl Send for Av {}

    impl Decoder for Av {
        fn next(&mut self) -> Option<Frame> {
            // SAFETY: Core Media and Core Video calls on a sample buffer we own (copy… returns it
            // retained) and release; the pixel buffer is locked while read.
            unsafe {
                let pool = objc_autoreleasePoolPush();
                let sb = send!(Id; self.output, "copyNextSampleBuffer");
                let frame = (|| {
                    if sb.is_null() {
                        return None;
                    }
                    let pts = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sb));
                    let img = CMSampleBufferGetImageBuffer(sb);
                    if img.is_null() {
                        return None;
                    }
                    CVPixelBufferLockBaseAddress(img, 1);
                    let base = CVPixelBufferGetBaseAddress(img);
                    let (row, w, h) = (CVPixelBufferGetBytesPerRow(img), CVPixelBufferGetWidth(img), CVPixelBufferGetHeight(img));
                    let out = (!base.is_null() && w > 0 && h > 0).then(|| {
                        let src = std::slice::from_raw_parts(base, row * h);
                        let (dw, dh) = scaled(w as u32, h as u32);
                        let rgba = resample(src, w, h, row as isize, 0, dw as usize, dh as usize, true);
                        Frame { pts: if pts.is_finite() { pts } else { 0.0 }, size: [dw as usize, dh as usize], rgba }
                    });
                    CVPixelBufferUnlockBaseAddress(img, 1);
                    out
                })();
                if !sb.is_null() {
                    CFRelease(sb);
                }
                objc_autoreleasePoolPop(pool);
                frame
            }
        }
    }

    impl Drop for Av {
        fn drop(&mut self) {
            // SAFETY: both were retained in `open`.
            unsafe {
                send!(Id; self.reader, "cancelReading");
                send!(Id; self.output, "release");
                send!(Id; self.reader, "release");
            }
        }
    }

    pub fn open(path: &Path, at: f64) -> Result<(Box<dyn Decoder>, Info), String> {
        let cpath = CString::new(path.as_os_str().as_bytes()).map_err(|_| "bad path".to_string())?;
        // SAFETY: objc_msgSend is called through function pointer types matching each method's
        // signature; objects are checked for nil before use; the ones kept are retained before
        // the autorelease pool is popped.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let r = (|| {
                let s = send!(Id; class("NSString"), "stringWithUTF8String:", cpath.as_ptr() => *const c_char);
                let url = send!(Id; class("NSURL"), "fileURLWithPath:", s => Id);
                let asset = send!(Id; class("AVURLAsset"), "URLAssetWithURL:options:", url => Id, std::ptr::null_mut() => Id);
                if asset.is_null() {
                    return Err("Couldn't open the video".to_string());
                }
                let vide = send!(Id; class("NSString"), "stringWithUTF8String:", c"vide".as_ptr() => *const c_char);
                let tracks = send!(Id; asset, "tracksWithMediaType:", vide => Id);
                let track = send!(Id; tracks, "firstObject");
                if track.is_null() {
                    return Err("No picture in this file".to_string());
                }
                let fps = send!(f32; track, "nominalFrameRate") as f64;
                let duration = CMTimeGetSeconds(send_time(asset, "duration"));
                // The video composition applies the track's rotation.
                let comp = send!(Id; class("AVVideoComposition"), "videoCompositionWithPropertiesOfAsset:", asset => Id);
                let size: CGSize = if comp.is_null() { send!(CGSize; track, "naturalSize") } else { send!(CGSize; comp, "renderSize") };
                let key = kCVPixelBufferPixelFormatTypeKey;
                let bgra = send!(Id; class("NSNumber"), "numberWithUnsignedInt:", 0x4247_5241u32 => u32);
                let settings = send!(Id; class("NSDictionary"), "dictionaryWithObject:forKey:", bgra => Id, key => Id);
                let output = if comp.is_null() {
                    send!(Id; class("AVAssetReaderTrackOutput"), "assetReaderTrackOutputWithTrack:outputSettings:", track => Id, settings => Id)
                } else {
                    let o = send!(Id; class("AVAssetReaderVideoCompositionOutput"), "assetReaderVideoCompositionOutputWithVideoTracks:videoSettings:", tracks => Id, settings => Id);
                    if !o.is_null() {
                        send!(Id; o, "setVideoComposition:", comp => Id);
                    }
                    o
                };
                let mut err: Id = std::ptr::null_mut();
                let reader = send!(Id; class("AVAssetReader"), "assetReaderWithAsset:error:", asset => Id, &mut err as *mut Id => *mut Id);
                if reader.is_null() || output.is_null() {
                    return Err("The system can't play this video".to_string());
                }
                send!(Id; output, "setAlwaysCopiesSampleData:", 0u8 => u8);
                let start = CMTime { value: (at.max(0.0) * 600.0) as i64, timescale: 600, flags: 1, epoch: 0 };
                // kCMTimePositiveInfinity: valid | positive infinity.
                let forever = CMTime { value: 0, timescale: 0, flags: 1 | 4, epoch: 0 };
                send!(Id; reader, "setTimeRange:", CMTimeRange { start, duration: forever } => CMTimeRange);
                if send!(u8; reader, "canAddOutput:", output => Id) == 0 {
                    return Err("The system can't play this video".to_string());
                }
                send!(Id; reader, "addOutput:", output => Id);
                if send!(u8; reader, "startReading") == 0 {
                    return Err("The system couldn't start reading this video".to_string());
                }
                send!(Id; reader, "retain");
                send!(Id; output, "retain");
                let (w, h) = (size.width.abs().round() as u32, size.height.abs().round() as u32);
                let info = Info { width: w, height: h, fps: if fps > 0.0 { fps } else { 30.0 }, duration: duration.is_finite().then_some(duration) };
                Ok((Box::new(Av { reader, output, size: (w, h) }) as Box<dyn Decoder>, info))
            })();
            objc_autoreleasePoolPop(pool);
            r
        }
    }
}

// ---------- Windows: Media Foundation ----------

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::core::{GUID, HSTRING};

    const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

    pub fn supported() -> bool {
        true
    }

    struct Mf {
        reader: IMFSourceReader,
        size: (u32, u32),
    }

    impl Decoder for Mf {
        fn next(&mut self) -> Option<Frame> {
            // SAFETY: Media Foundation calls on a reader made in `open`; the buffer is locked
            // while read and the reported pitch and size bound every access.
            unsafe {
                loop {
                    let (mut flags, mut ts) = (0u32, 0i64);
                    let mut sample: Option<IMFSample> = None;
                    self.reader.ReadSample(FIRST_VIDEO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample)).ok()?;
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        return None;
                    }
                    let Some(sample) = sample else { continue };
                    let buf = sample.GetBufferByIndex(0).ok()?;
                    let (w, h) = self.size;
                    let (dw, dh) = scaled(w, h);
                    let pts = ts as f64 / 10_000_000.0;
                    if let Ok(b2) = windows::core::Interface::cast::<IMF2DBuffer>(&buf) {
                        let (mut p, mut pitch) = (std::ptr::null_mut(), 0i32);
                        b2.Lock2D(&mut p, &mut pitch).ok()?;
                        // The first row may sit at the end (bottom-up, negative pitch).
                        let rows = h as isize;
                        let (lo, len) = if pitch < 0 { (p.offset((rows - 1) * pitch as isize), (rows * -(pitch as isize)) as usize) } else { (p, (rows * pitch as isize) as usize) };
                        let src = std::slice::from_raw_parts(lo as *const u8, len);
                        let start = if pitch < 0 { (rows - 1) * -(pitch as isize) } else { 0 };
                        let rgba = resample(src, w as usize, h as usize, pitch as isize, start, dw as usize, dh as usize, true);
                        let _ = b2.Unlock2D();
                        return Some(Frame { pts, size: [dw as usize, dh as usize], rgba });
                    }
                    let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                    buf.Lock(&mut p, None, Some(&mut len)).ok()?;
                    let row = w as usize * 4;
                    let out = (len as usize >= row * h as usize).then(|| {
                        let src = std::slice::from_raw_parts(p as *const u8, len as usize);
                        resample(src, w as usize, h as usize, row as isize, 0, dw as usize, dh as usize, true)
                    });
                    let _ = buf.Unlock();
                    return out.map(|rgba| Frame { pts, size: [dw as usize, dh as usize], rgba });
                }
            }
        }
    }

    pub fn open(path: &Path, at: f64) -> Result<(Box<dyn Decoder>, Info), String> {
        let e = |x: windows::core::Error| x.message().to_string();
        // SAFETY: Media Foundation and COM calls with valid arguments; every result is checked.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(e)?;
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 1).map_err(e)?;
            let attrs = attrs.ok_or("no attributes")?;
            attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1).map_err(e)?;
            let url = HSTRING::from(path.as_os_str());
            let reader = MFCreateSourceReaderFromURL(&url, &attrs).map_err(e)?;
            let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
            reader.SetStreamSelection(FIRST_VIDEO, true).map_err(|_| "No picture in this file".to_string())?;
            let mt = MFCreateMediaType().map_err(e)?;
            mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(e)?;
            mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(e)?;
            reader.SetCurrentMediaType(FIRST_VIDEO, None, &mt).map_err(e)?;
            let cur = reader.GetCurrentMediaType(FIRST_VIDEO).map_err(e)?;
            let fs = cur.GetUINT64(&MF_MT_FRAME_SIZE).map_err(e)?;
            let (w, h) = ((fs >> 32) as u32, fs as u32);
            let fps = cur.GetUINT64(&MF_MT_FRAME_RATE).ok().map(|r| ((r >> 32) as f64, (r as u32) as f64)).filter(|r| r.1 > 0.0).map_or(30.0, |r| r.0 / r.1);
            let duration = reader
                .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
                .ok()
                .and_then(|v| u64::try_from(&v).ok())
                .map(|d| d as f64 / 10_000_000.0);
            if at > 0.0 {
                let pos = PROPVARIANT::from((at * 10_000_000.0) as i64);
                let _ = reader.SetCurrentPosition(&GUID::zeroed(), &pos);
            }
            Ok((Box::new(Mf { reader, size: (w, h) }), Info { width: w, height: h, fps, duration }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fit_the_pane() {
        assert_eq!(scaled(1920, 1080), (960, 540));
        assert_eq!(scaled(640, 360), (640, 360));
        assert_eq!(scaled(1081, 1921), (960, 1706));
    }

    #[test]
    fn resample_swaps_and_flips() {
        // 2x2 BGRA, bottom-up (negative row, starting at the last row).
        let src = [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0, 10, 11, 12, 0];
        let out = resample(&src, 2, 2, -8, 8, 2, 2, true);
        assert_eq!(&out[..8], &[9, 8, 7, 255, 12, 11, 10, 255]);
    }

    #[test]
    #[cfg(not(any(windows, target_os = "macos")))]
    fn plays_a_video_with_ffmpeg() {
        if !supported() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("filescape-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=10:duration=1", "-pix_fmt", "yuv420p", "-y"])
            .arg(&p)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return;
        }
        let (mut d, info) = open(&p, 0.5).unwrap();
        assert_eq!((info.width, info.height), (320, 240));
        assert!((info.fps - 10.0).abs() < 0.01);
        let f = d.next().unwrap();
        assert_eq!(f.size, [320, 240]);
        assert!((f.pts - 0.5).abs() < 1e-6);
        let n = std::iter::from_fn(|| d.next()).count();
        assert!((3..=6).contains(&n), "{n} frames after the first");
        std::fs::remove_dir_all(&dir).ok();
    }
}
