//! macOS: application icons from the system (NSWorkspace, as Finder shows them) and launching
//! through LaunchServices, which runs Gatekeeper's checks and asks the user where it needs to.

use std::ffi::{CString, c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

type Id = *mut c_void;
type Sel = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn LSOpenCFURLRef(url: *const c_void, launched: *mut *const c_void) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(alloc: *const c_void, buf: *const u8, len: isize, is_dir: u8) -> *const c_void;
    fn CFRelease(cf: *const c_void);
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

/// objc_msgSend cast to a method's signature (`C-unwind`, so `macaudio::guard` can catch an
/// Objective-C exception).
macro_rules! send {
    ($ret:ty; $obj:expr, $sel:expr $(, $arg:expr => $t:ty)*) => {{
        let f: extern "C-unwind" fn(Id, Sel $(, $t)*) -> $ret = std::mem::transmute(objc_msgSend as *const ());
        f($obj, sel($sel) $(, $arg)*)
    }};
}

/// The icon Finder shows for the item, `size` pixels across, as RGBA.
pub fn icon(path: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: messages with matching signatures to objects that are checked for nil, all inside
    // an autorelease pool popped at the end; the bitmap rep we alloc is released.
    let png = unsafe {
        let pool = objc_autoreleasePoolPush();
        let r = crate::macaudio::guard(|| {
            let ws = send!(Id; class("NSWorkspace"), "sharedWorkspace");
            let s = send!(Id; class("NSString"), "stringWithUTF8String:", c.as_ptr() => *const c_char);
            if ws.is_null() || s.is_null() {
                return Ok(None);
            }
            let img = send!(Id; ws, "iconForFile:", s => Id);
            if img.is_null() {
                return Ok(None);
            }
            let mut rect = Rect { x: 0.0, y: 0.0, w: size as f64, h: size as f64 };
            let cg = send!(Id; img, "CGImageForProposedRect:context:hints:", &mut rect as *mut Rect => *mut Rect, std::ptr::null_mut() => Id, std::ptr::null_mut() => Id);
            if cg.is_null() {
                return Ok(None);
            }
            let rep = send!(Id; class("NSBitmapImageRep"), "alloc");
            let rep = send!(Id; rep, "initWithCGImage:", cg => Id);
            if rep.is_null() {
                return Ok(None);
            }
            let props = send!(Id; class("NSDictionary"), "dictionary");
            // NSBitmapImageFileTypePNG
            let data = send!(Id; rep, "representationUsingType:properties:", 4usize => usize, props => Id);
            let out = if data.is_null() {
                None
            } else {
                let len = send!(usize; data, "length");
                let bytes = send!(*const u8; data, "bytes");
                (!bytes.is_null() && len > 0).then(|| std::slice::from_raw_parts(bytes, len).to_vec())
            };
            send!((); rep, "release");
            Ok(out)
        });
        objc_autoreleasePoolPop(pool);
        r.ok().flatten()?
    };
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()?.into_rgba8();
    Some((img.width(), img.height(), img.into_raw()))
}

/// Open the application as Finder would.
pub fn launch(app: &Path) -> Result<(), String> {
    let bytes = app.as_os_str().as_bytes();
    let name = app.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // SAFETY: the URL is created from a byte buffer that outlives the call and released after.
    let status = unsafe {
        let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, 1);
        if url.is_null() {
            return Err(format!("macOS couldn't open “{name}”."));
        }
        let s = LSOpenCFURLRef(url, std::ptr::null_mut());
        CFRelease(url);
        s
    };
    match status {
        0 => Ok(()),
        // The user said no in a system dialog (Gatekeeper's "Cancel", for one).
        -128 => Ok(()),
        s => Err(explain(s, &name)),
    }
}

/// A plain sentence for a LaunchServices error code.
fn explain(status: i32, name: &str) -> String {
    match status {
        -10660 => format!("“{name}” is in the Trash. Move it out of the Trash to open it."),
        -10661 => format!("“{name}” can't be opened: its program isn't one this Mac can run (it may be damaged, or made for another kind of computer)."),
        -10664 | -10825 => format!("“{name}” needs a newer version of macOS."),
        -10665 => format!("“{name}” is made for Intel Macs and needs Rosetta. macOS will offer to install it when you open the app in Finder."),
        -10811 => format!("“{name}” isn't an application macOS can open."),
        -10826 => format!("You don't have permission to open “{name}”."),
        -10827 => format!("“{name}” is damaged or incomplete: its program is missing. Try installing it again."),
        -10818 => format!("“{name}” is already starting."),
        _ => format!("macOS couldn't open “{name}” (LaunchServices error {status})."),
    }
}
