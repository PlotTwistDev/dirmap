//! macOS sound through AVFoundation: playing (AVAudioPlayer, which pauses at once and can jump
//! to any point, unlike `afplay`, which keeps playing what it has buffered when stopped) and the
//! levels for a waveform (AVAssetReader). Called through the Objective-C runtime.
//!
//! AVFoundation reports some mistakes by raising an Objective-C exception. Rust can't unwind
//! through one (the app aborts), so every call into it goes through [`guard`], which catches it
//! and turns it into an error.

#![cfg(target_os = "macos")]

use std::ffi::{CStr, CString, c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

type Id = *mut c_void;
type Sel = *mut c_void;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVFormatIDKey: Id;
    static AVSampleRateKey: Id;
    static AVNumberOfChannelsKey: Id;
    static AVLinearPCMBitDepthKey: Id;
    static AVLinearPCMIsFloatKey: Id;
    static AVLinearPCMIsBigEndianKey: Id;
    static AVLinearPCMIsNonInterleaved: Id;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetDataBuffer(sbuf: Id) -> Id;
    fn CMBlockBufferGetDataLength(buf: Id) -> usize;
    fn CMBlockBufferCopyDataBytes(buf: Id, offset: usize, len: usize, dest: *mut c_void) -> i32;
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

/// Run `f`, turning an Objective-C exception raised inside it into an error. Messages sent in `f`
/// must go through a `C-unwind` function pointer (as `send!` does) so the exception can reach
/// the catch.
pub fn guard<R>(f: impl FnOnce() -> Result<R, String>) -> Result<R, String> {
    match objc2::exception::catch(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(Some(e)) => Err(format!("The system couldn't read this file ({e})")),
        Err(None) => Err("The system couldn't read this file".into()),
    }
}

/// objc_msgSend cast to a method's signature.
macro_rules! send {
    ($ret:ty; $obj:expr, $sel:expr $(, $arg:expr => $t:ty)*) => {{
        let f: extern "C-unwind" fn(Id, Sel $(, $t)*) -> $ret = std::mem::transmute(objc_msgSend as *const ());
        f($obj, sel($sel) $(, $arg)*)
    }};
}

/// An NSURL for a file. Autoreleased: call inside a pool.
unsafe fn file_url(path: &Path) -> Option<Id> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: messages with matching signatures; nil is checked.
    unsafe {
        let s = send!(Id; class("NSString"), "stringWithUTF8String:", c.as_ptr() => *const c_char);
        if s.is_null() {
            return None;
        }
        let url = send!(Id; class("NSURL"), "fileURLWithPath:", s => Id);
        (!url.is_null()).then_some(url)
    }
}

unsafe fn describe(err: Id) -> String {
    if err.is_null() {
        return "The system can't play this file".into();
    }
    // SAFETY: an NSError's description, read while the pool holds it.
    unsafe {
        let d = send!(Id; err, "localizedDescription");
        let c = if d.is_null() { std::ptr::null() } else { send!(*const c_char; d, "UTF8String") };
        if c.is_null() { "The system can't play this file".into() } else { CStr::from_ptr(c).to_string_lossy().into_owned() }
    }
}

/// Play a file's sound until the channel closes, following its commands.
pub fn play(path: &Path, rx: &mpsc::Receiver<crate::media::Cmd>, state: &Mutex<crate::media::PlayState>) {
    use crate::media::Cmd;
    let set = |f: &mut dyn FnMut(&mut crate::media::PlayState)| {
        if let Ok(mut s) = state.lock() {
            f(&mut s);
        }
    };
    // SAFETY: messages to an AVAudioPlayer we created (alloc/init, so owned) and release at the
    // end, all on this thread; autoreleased objects live until the pool is popped.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let r = guard(|| {
            let url = file_url(path).ok_or("bad path")?;
            let mut err: Id = std::ptr::null_mut();
            let alloc = send!(Id; class("AVAudioPlayer"), "alloc");
            let player = send!(Id; alloc, "initWithContentsOfURL:error:", url => Id, &mut err as *mut Id => *mut Id);
            if player.is_null() {
                return Err(describe(err));
            }
            send!(u8; player, "prepareToPlay");
            let length = send!(f64; player, "duration");
            set(&mut |s| s.length = (length > 0.0).then_some(length));
            // Whether we asked it to play (it stops by itself at the end).
            let mut wanted = false;
            loop {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(Cmd::Play) => {
                        if state.lock().map(|s| s.ended).unwrap_or(false) {
                            send!(Id; player, "setCurrentTime:", 0.0f64 => f64);
                        }
                        send!(u8; player, "play");
                        wanted = true;
                    }
                    Ok(Cmd::Pause) => {
                        send!(Id; player, "pause");
                        wanted = false;
                    }
                    Ok(Cmd::Seek(t)) => {
                        send!(Id; player, "setCurrentTime:", t.max(0.0) => f64);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                let playing = send!(u8; player, "isPlaying") != 0;
                let pos = send!(f64; player, "currentTime");
                let ended = wanted && !playing;
                if ended {
                    wanted = false;
                }
                set(&mut |s| {
                    s.playing = playing;
                    s.position = pos;
                    if ended {
                        s.ended = true;
                        s.position = 0.0;
                    } else if playing {
                        s.ended = false;
                    }
                });
            }
            send!(Id; player, "stop");
            send!(Id; player, "release");
            Ok(())
        });
        if let Err(e) = r {
            set(&mut |s| {
                s.error = Some(e.clone());
                s.playing = false;
            });
        }
        objc_autoreleasePoolPop(pool);
    }
}

/// The rate sound is read at for a waveform: a common rate every decoder converts to (asking
/// AVFoundation for a very low one, such as 2000, can raise an exception).
const READ_RATE: u32 = 8000;

/// The sound as mono 16-bit levels at `rate` per second (each the loudest of its stretch), at
/// most `max_secs` of it.
pub fn samples(path: &Path, rate: u32, max_secs: f64) -> Result<Vec<i16>, String> {
    // SAFETY: messages with matching signatures; objects checked for nil; sample buffers are
    // copied (so owned) and released; everything else is autoreleased in the pool.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let r = guard(|| {
            let url = file_url(path).ok_or("bad path")?;
            let asset = send!(Id; class("AVURLAsset"), "URLAssetWithURL:options:", url => Id, std::ptr::null_mut() => Id);
            if asset.is_null() {
                return Err("Couldn't open it".to_string());
            }
            let soun = send!(Id; class("NSString"), "stringWithUTF8String:", c"soun".as_ptr() => *const c_char);
            let tracks = send!(Id; asset, "tracksWithMediaType:", soun => Id);
            let track = send!(Id; tracks, "firstObject");
            if track.is_null() {
                return Err("No sound in this file".to_string());
            }
            let num_u = |v: u32| send!(Id; class("NSNumber"), "numberWithUnsignedInt:", v => u32);
            let num_b = |v: bool| send!(Id; class("NSNumber"), "numberWithBool:", v as u8 => u8);
            let dict = send!(Id; class("NSMutableDictionary"), "dictionary");
            for (k, v) in [
                (AVFormatIDKey, num_u(0x6C70_636D)), // 'lpcm'
                (AVSampleRateKey, num_u(READ_RATE)),
                (AVNumberOfChannelsKey, num_u(1)),
                (AVLinearPCMBitDepthKey, num_u(16)),
                (AVLinearPCMIsFloatKey, num_b(false)),
                (AVLinearPCMIsBigEndianKey, num_b(false)),
                (AVLinearPCMIsNonInterleaved, num_b(false)),
            ] {
                send!(Id; dict, "setObject:forKey:", v => Id, k => Id);
            }
            let output = send!(Id; class("AVAssetReaderTrackOutput"), "assetReaderTrackOutputWithTrack:outputSettings:", track => Id, dict => Id);
            let mut err: Id = std::ptr::null_mut();
            let reader = send!(Id; class("AVAssetReader"), "assetReaderWithAsset:error:", asset => Id, &mut err as *mut Id => *mut Id);
            if reader.is_null() || output.is_null() || send!(u8; reader, "canAddOutput:", output => Id) == 0 {
                return Err(describe(err));
            }
            send!(Id; reader, "addOutput:", output => Id);
            if send!(u8; reader, "startReading") == 0 {
                return Err("The system couldn't read the sound".to_string());
            }
            let cap = (rate as f64 * max_secs) as usize;
            // Each output sample is the loudest of this many read ones.
            let step = (READ_RATE / rate.max(1)).max(1) as usize;
            let (mut out, mut buf): (Vec<i16>, Vec<i16>) = (Vec::new(), Vec::new());
            let (mut peak, mut k) = (0i16, 0usize);
            while out.len() < cap {
                let inner = objc_autoreleasePoolPush();
                let sb = send!(Id; output, "copyNextSampleBuffer");
                if sb.is_null() {
                    objc_autoreleasePoolPop(inner);
                    break;
                }
                let block = CMSampleBufferGetDataBuffer(sb);
                if !block.is_null() {
                    let len = CMBlockBufferGetDataLength(block) / 2;
                    buf.resize(len, 0);
                    if CMBlockBufferCopyDataBytes(block, 0, len * 2, buf.as_mut_ptr() as *mut c_void) == 0 {
                        for &v in &buf {
                            peak = peak.max(v.unsigned_abs().min(i16::MAX as u16) as i16);
                            k += 1;
                            if k == step {
                                out.push(peak);
                                (peak, k) = (0, 0);
                            }
                        }
                    }
                }
                CFRelease(sb);
                objc_autoreleasePoolPop(inner);
            }
            send!(Id; reader, "cancelReading");
            out.truncate(cap);
            Ok(out)
        });
        objc_autoreleasePoolPop(pool);
        r
    }
}
