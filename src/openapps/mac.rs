//! macOS: Launch Services, which knows every app's declared document types. For a file on this
//! computer it's asked about the file itself (so a per-file choice in Finder's Get Info counts);
//! otherwise about its content type.

use super::{Handler, Type};
use std::ffi::c_void;
use std::path::{Path, PathBuf};

type CFRef = *const c_void;

#[repr(C, packed(2))]
struct LSLaunchURLSpec {
    app_url: CFRef,
    item_urls: CFRef,
    pass_thru_params: *const c_void,
    launch_flags: u32,
    async_ref_con: *mut c_void,
}

const ALL_ROLES: u32 = 0xFFFF_FFFF;
const LAUNCH_DEFAULTS: u32 = 0x1;
const UTF8: u32 = 0x0800_0100;

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn LSCopyApplicationURLsForURL(url: CFRef, roles: u32) -> CFRef;
    fn LSCopyDefaultApplicationURLForURL(url: CFRef, roles: u32, err: *mut CFRef) -> CFRef;
    fn LSCopyAllRoleHandlersForContentType(uti: CFRef, roles: u32) -> CFRef;
    fn LSCopyDefaultRoleHandlerForContentType(uti: CFRef, roles: u32) -> CFRef;
    fn LSCopyApplicationURLsForBundleIdentifier(id: CFRef, err: *mut CFRef) -> CFRef;
    fn UTTypeCreatePreferredIdentifierForTag(class: CFRef, tag: CFRef, conforming: CFRef) -> CFRef;
    fn LSOpenFromURLSpec(spec: *const LSLaunchURLSpec, launched: *mut CFRef) -> i32;
    fn LSSetDefaultRoleHandlerForContentType(uti: CFRef, roles: u32, bundle: CFRef) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFArrayGetCount(a: CFRef) -> isize;
    fn CFArrayGetValueAtIndex(a: CFRef, i: isize) -> CFRef;
    fn CFArrayCreate(alloc: CFRef, values: *const CFRef, n: isize, callbacks: *const c_void) -> CFRef;
    static kCFTypeArrayCallBacks: c_void;
    fn CFStringCreateWithBytes(alloc: CFRef, bytes: *const u8, n: isize, enc: u32, bom: u8) -> CFRef;
    fn CFStringGetCString(s: CFRef, buf: *mut u8, size: isize, enc: u32) -> u8;
    fn CFURLCreateFromFileSystemRepresentation(alloc: CFRef, buf: *const u8, len: isize, is_dir: u8) -> CFRef;
    fn CFURLGetFileSystemRepresentation(url: CFRef, resolve: u8, buf: *mut u8, max: isize) -> u8;
}

/// A Core Foundation object released when dropped.
struct Owned(CFRef);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: each was returned by a Create or Copy call, so it's ours to release once.
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cfstr(s: &str) -> Owned {
    // SAFETY: the bytes are valid for the call; the result is released by Owned.
    Owned(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) })
}

fn string(s: CFRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let mut buf = vec![0u8; 1024];
    // SAFETY: the buffer is as long as said; a string too long gives false.
    if unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) } == 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0)?;
    String::from_utf8(buf[..end].to_vec()).ok()
}

fn url_of(p: &Path) -> Owned {
    use std::os::unix::ffi::OsStrExt;
    let b = p.as_os_str().as_bytes();
    // SAFETY: the bytes are valid for the call; the result is released by Owned.
    Owned(unsafe { CFURLCreateFromFileSystemRepresentation(std::ptr::null(), b.as_ptr(), b.len() as isize, p.is_dir() as u8) })
}

fn path_of(url: CFRef) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    if url.is_null() {
        return None;
    }
    let mut buf = vec![0u8; 4096];
    // SAFETY: the buffer is as long as said.
    if unsafe { CFURLGetFileSystemRepresentation(url, 1, buf.as_mut_ptr(), buf.len() as isize) } == 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0)?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..end])))
}

/// The items of a CFArray (not retained: valid while the array is).
fn items(a: &Owned) -> Vec<CFRef> {
    if a.0.is_null() {
        return Vec::new();
    }
    // SAFETY: a is a CFArray; indexes are in range.
    unsafe { (0..CFArrayGetCount(a.0)).map(|i| CFArrayGetValueAtIndex(a.0, i)).collect() }
}

/// The content type a file of this kind has ("public.plain-text").
fn uti(t: &Type) -> Option<Owned> {
    let ext = match t {
        Type::Text => return Some(cfstr("public.plain-text")),
        Type::Folder => return Some(cfstr("public.folder")),
        Type::Ext(e) => e,
        Type::Unknown => return None,
    };
    let class = cfstr("public.filename-extension");
    let tag = cfstr(ext);
    // SAFETY: valid strings; the result is released by Owned.
    let u = Owned(unsafe { UTTypeCreatePreferredIdentifierForTag(class.0, tag.0, std::ptr::null()) });
    (!u.0.is_null()).then_some(u)
}

fn handler(app: PathBuf, default: Option<&PathBuf>) -> Handler {
    let name = crate::apps::info(&app)
        .and_then(|i| i.name)
        .unwrap_or_else(|| app.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    Handler { id: app.to_string_lossy().into_owned(), name, default: default == Some(&app), app }
}

pub fn handlers(t: &Type) -> Vec<Handler> {
    handlers_for(t, None)
}

/// The apps for a type; with a sample file on this computer, Launch Services is asked about it.
pub fn handlers_for(t: &Type, sample: Option<&Path>) -> Vec<Handler> {
    // SAFETY: Launch Services calls with valid objects; every returned object is released.
    unsafe {
        if let Some(p) = sample.filter(|p| p.exists() && matches!(t, Type::Ext(_))) {
            let url = url_of(p);
            let apps = Owned(LSCopyApplicationURLsForURL(url.0, ALL_ROLES));
            let def = Owned(LSCopyDefaultApplicationURLForURL(url.0, ALL_ROLES, std::ptr::null_mut()));
            let def = path_of(def.0);
            return items(&apps).into_iter().filter_map(|u| path_of(u)).map(|a| handler(a, def.as_ref())).collect();
        }
        let Some(u) = uti(t) else { return Vec::new() };
        let ids = Owned(LSCopyAllRoleHandlersForContentType(u.0, ALL_ROLES));
        let def_id = Owned(LSCopyDefaultRoleHandlerForContentType(u.0, ALL_ROLES));
        let def_id = string(def_id.0);
        let mut out = Vec::new();
        for id in items(&ids) {
            let urls = Owned(LSCopyApplicationURLsForBundleIdentifier(id, std::ptr::null_mut()));
            let Some(app) = items(&urls).into_iter().find_map(|u| path_of(u)) else { continue };
            let is_def = def_id.is_some() && string(id) == def_id;
            let mut h = handler(app, None);
            h.default = is_def;
            out.push(h);
        }
        out
    }
}

pub fn open(h: &Handler, files: &[PathBuf]) -> Result<(), String> {
    let app = url_of(&h.app);
    if app.0.is_null() {
        return Err(format!("{} isn't there any more.", h.name));
    }
    let urls: Vec<Owned> = files.iter().map(|f| url_of(f)).collect();
    let raw: Vec<CFRef> = urls.iter().map(|u| u.0).filter(|u| !u.is_null()).collect();
    // SAFETY: an array of valid URLs, retained by the array (kCFTypeArrayCallBacks) and released
    // by Owned; the spec lives through the call.
    let status = unsafe {
        let arr = Owned(CFArrayCreate(std::ptr::null(), raw.as_ptr(), raw.len() as isize, &kCFTypeArrayCallBacks as *const c_void));
        let spec = LSLaunchURLSpec { app_url: app.0, item_urls: if raw.is_empty() { std::ptr::null() } else { arr.0 }, pass_thru_params: std::ptr::null(), launch_flags: LAUNCH_DEFAULTS, async_ref_con: std::ptr::null_mut() };
        LSOpenFromURLSpec(&spec, std::ptr::null_mut())
    };
    if status == 0 { Ok(()) } else { Err(format!("macOS couldn't open it with {} (error {status}).", h.name)) }
}

/// Every app in the usual Applications folders.
pub fn all_apps() -> Vec<Handler> {
    let mut dirs = vec![PathBuf::from("/Applications"), PathBuf::from("/Applications/Utilities"), PathBuf::from("/System/Applications"), PathBuf::from("/System/Applications/Utilities")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Applications"));
    }
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "app") {
                out.push(handler(p, None));
            }
        }
    }
    out
}

/// Make it the default for the type, as Finder's Get Info "Change All…" does.
pub fn set_default(h: &Handler, t: &Type) -> Result<(), String> {
    let id = crate::apps::info(&h.app).and_then(|i| i.id).ok_or_else(|| format!("{} doesn't say what it's called to macOS, so it can't be made the default.", h.name))?;
    let u = uti(t).ok_or("macOS doesn't know what kind of file this is.")?;
    let b = cfstr(&id);
    // SAFETY: valid strings, kept alive through the call.
    let status = unsafe { LSSetDefaultRoleHandlerForContentType(u.0, ALL_ROLES, b.0) };
    if status == 0 { Ok(()) } else { Err(format!("macOS didn't change it (error {status}).")) }
}
