//! Dragging items out of Filescape into other apps: Finder, Explorer, the Desktop, Mail, the
//! Dock's Trash or the Recycle Bin. Inside the window egui runs the drag; once the pointer leaves
//! it, the system's own drag takes over, so every app (and the Trash) treats the items exactly as
//! if they had come from Finder or Explorer.
//!
//! - **macOS**: an `NSDraggingSession` on the window's view, with file URLs. The drop target
//!   decides what happens (Finder moves on the same drive and copies across drives, the Trash
//!   trashes). If the Trash reports a delete but the items are still there, Filescape trashes
//!   them itself, through Finder, so Put Back works either way.
//! - **Windows**: `SHDoDragDrop` with the shell's own data object for the items, so Explorer, the
//!   Recycle Bin and other apps do what they do for Explorer's drags, drag picture included.
//! - **Linux**: not available. The window layer has no way to start a drag that leaves the
//!   window, so dragging stops at the window's edge as before.

use std::path::PathBuf;
use std::sync::Mutex;

/// Whether this system can drag items out of the window.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

/// How a drag out ended, as the drop target reported it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
pub enum Outcome {
    /// Dropped nowhere, or cancelled.
    Nothing,
    Copied,
    Moved,
    /// Dropped on the Trash (macOS).
    Trashed,
}

/// The last drag out to end: its items and outcome. Read by [`take_ended`].
static ENDED: Mutex<Option<(Vec<PathBuf>, Outcome)>> = Mutex::new(None);

/// A drag out that has ended since the last call (the app refreshes, and finishes a drop on the
/// Trash if needed).
pub fn take_ended() -> Option<(Vec<PathBuf>, Outcome)> {
    ENDED.lock().ok()?.take()
}

#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
fn ended(paths: Vec<PathBuf>, how: Outcome) {
    if let Ok(mut e) = ENDED.lock() {
        *e = Some((paths, how));
    }
}

/// Start the system's drag for `paths`, from the window (`window`: the NSView on macOS, the
/// HWND on Windows). On macOS it returns at once and the drag goes on in the background; on
/// Windows it returns when the items are dropped.
pub fn start(window: isize, paths: Vec<PathBuf>) -> Result<(), String> {
    if paths.is_empty() || window == 0 {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        mac::start(window, paths)
    }
    #[cfg(windows)]
    {
        win::start(window, paths)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = paths;
        Err("Dragging out of Filescape isn't available on Linux yet.".into())
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::ffi::{CString, c_char, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::sync::OnceLock;

    type Id = *mut c_void;
    type Sel = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point {
        x: f64,
        y: f64,
    }

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

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn objc_getProtocol(name: *const c_char) -> Id;
        fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
        fn objc_registerClassPair(cls: Id);
        fn class_addMethod(cls: Id, name: Sel, imp: *const c_void, types: *const c_char) -> u8;
        fn class_addProtocol(cls: Id, protocol: Id) -> u8;
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

    macro_rules! send {
        ($ret:ty; $obj:expr, $sel:expr $(, $arg:expr => $t:ty)*) => {{
            let f: extern "C-unwind" fn(Id, Sel $(, $t)*) -> $ret = std::mem::transmute(objc_msgSend as *const ());
            f($obj, sel($sel) $(, $arg)*)
        }};
    }

    /// The drag under way: its items, the view it started from, and its source object (kept
    /// until the drag ends, then released).
    struct Live {
        paths: Vec<PathBuf>,
        view: usize,
        source: usize,
    }
    static LIVE: Mutex<Option<Live>> = Mutex::new(None);

    // NSDragOperation bits.
    const COPY: usize = 1;
    const LINK: usize = 2;
    const GENERIC: usize = 4;
    const MOVE: usize = 16;
    const DELETE: usize = 32;

    /// What the items allow: anything Finder allows for files.
    extern "C" fn mask(_this: Id, _cmd: Sel, _session: Id, _context: isize) -> usize {
        COPY | LINK | GENERIC | MOVE | DELETE
    }

    /// The drag ended: note how, and give the window its mouse-up back (AppKit kept it for the
    /// drag, so the window would otherwise think the button is still held).
    extern "C" fn session_ended(_this: Id, _cmd: Sel, _session: Id, _at: Point, op: usize) {
        let Some(live) = LIVE.lock().ok().and_then(|mut l| l.take()) else { return };
        let how = if op & DELETE != 0 {
            Outcome::Trashed
        } else if op & MOVE != 0 || op & GENERIC != 0 {
            Outcome::Moved
        } else if op & (COPY | LINK) != 0 {
            Outcome::Copied
        } else {
            Outcome::Nothing
        };
        // SAFETY: messages with matching signatures to the window's view and its window (alive:
        // the app is running), and releasing the source object we allocated.
        unsafe {
            let view = live.view as Id;
            let window = send!(Id; view, "window");
            if !window.is_null() {
                let at = send!(Point; window, "mouseLocationOutsideOfEventStream");
                let number = send!(isize; window, "windowNumber");
                // NSEventTypeLeftMouseUp
                let up = send!(Id; class("NSEvent"), "mouseEventWithType:location:modifierFlags:timestamp:windowNumber:context:eventNumber:clickCount:pressure:",
                    2usize => usize, at => Point, 0usize => usize, 0.0f64 => f64, number => isize, std::ptr::null_mut() => Id, 0isize => isize, 1isize => isize, 0.0f32 => f32);
                let app = send!(Id; class("NSApplication"), "sharedApplication");
                if !up.is_null() && !app.is_null() {
                    send!((); app, "postEvent:atStart:", up => Id, 0u8 => u8);
                }
            }
            send!((); live.source as Id, "release");
        }
        ended(live.paths, how);
    }

    /// The dragging-source class, made once.
    fn source_class() -> Id {
        static CLASS: OnceLock<usize> = OnceLock::new();
        *CLASS.get_or_init(|| {
            let name = CString::new("FilescapeDragSource").unwrap();
            // SAFETY: a new NSObject subclass with two methods whose type strings match the
            // functions (NSUInteger, id, SEL, id, NSInteger / void, id, SEL, id, NSPoint, NSUInteger).
            unsafe {
                let existing = objc_getClass(name.as_ptr());
                if !existing.is_null() {
                    return existing as usize;
                }
                let cls = objc_allocateClassPair(class("NSObject"), name.as_ptr(), 0);
                let t1 = CString::new("Q@:@q").unwrap();
                let t2 = CString::new("v@:@{CGPoint=dd}Q").unwrap();
                class_addMethod(cls, sel("draggingSession:sourceOperationMaskForDraggingContext:"), mask as *const c_void, t1.as_ptr());
                class_addMethod(cls, sel("draggingSession:endedAtPoint:operation:"), session_ended as *const c_void, t2.as_ptr());
                let proto = CString::new("NSDraggingSource").unwrap();
                let p = objc_getProtocol(proto.as_ptr());
                if !p.is_null() {
                    class_addProtocol(cls, p);
                }
                objc_registerClassPair(cls);
                cls as usize
            }
        }) as Id
    }

    unsafe fn nsstring(p: &std::path::Path) -> Option<Id> {
        let c = CString::new(p.as_os_str().as_bytes()).ok()?;
        // SAFETY: an autoreleased NSString from a NUL-terminated UTF-8 buffer.
        let s = unsafe { send!(Id; class("NSString"), "stringWithUTF8String:", c.as_ptr() => *const c_char) };
        (!s.is_null()).then_some(s)
    }

    pub fn start(view: isize, paths: Vec<PathBuf>) -> Result<(), String> {
        if LIVE.lock().map(|l| l.is_some()).unwrap_or(true) {
            return Ok(());
        }
        let view = view as Id;
        // SAFETY: messages with matching signatures to AppKit objects checked for nil, inside an
        // autorelease pool; objects we alloc are released once the array (or the drag) holds them.
        let r = unsafe {
            let pool = objc_autoreleasePoolPush();
            let r = crate::macaudio::guard(|| {
                let app = send!(Id; class("NSApplication"), "sharedApplication");
                let event = if app.is_null() { std::ptr::null_mut() } else { send!(Id; app, "currentEvent") };
                if event.is_null() {
                    return Err("macOS didn't say where the drag is.".to_string());
                }
                let in_window = send!(Point; event, "locationInWindow");
                let at = send!(Point; view, "convertPoint:fromView:", in_window => Point, std::ptr::null_mut() => Id);
                let ws = send!(Id; class("NSWorkspace"), "sharedWorkspace");
                let items = send!(Id; class("NSMutableArray"), "array");
                for (i, p) in paths.iter().enumerate() {
                    let Some(s) = nsstring(p) else { continue };
                    let url = send!(Id; class("NSURL"), "fileURLWithPath:", s => Id);
                    if url.is_null() {
                        continue;
                    }
                    let item = send!(Id; class("NSDraggingItem"), "alloc");
                    let item = send!(Id; item, "initWithPasteboardWriter:", url => Id);
                    if item.is_null() {
                        continue;
                    }
                    // The items' icons, fanned out a little under the pointer.
                    let icon = send!(Id; ws, "iconForFile:", s => Id);
                    let off = (i.min(4) as f64) * 6.0;
                    let frame = Rect { x: at.x - 24.0 + off, y: at.y - 24.0 - off, w: 48.0, h: 48.0 };
                    send!((); item, "setDraggingFrame:contents:", frame => Rect, icon => Id);
                    send!((); items, "addObject:", item => Id);
                    send!((); item, "release");
                }
                if send!(usize; items, "count") == 0 {
                    return Err("Those items can't be dragged out.".to_string());
                }
                let source = send!(Id; source_class(), "alloc");
                let source = send!(Id; source, "init");
                *LIVE.lock().map_err(|_| "busy".to_string())? = Some(Live { paths: paths.clone(), view: view as usize, source: source as usize });
                let session = send!(Id; view, "beginDraggingSessionWithItems:event:source:", items => Id, event => Id, source => Id);
                if session.is_null() {
                    if let Ok(mut l) = LIVE.lock() {
                        *l = None;
                    }
                    send!((); source, "release");
                    return Err("macOS couldn't start the drag.".to_string());
                }
                send!((); session, "setAnimatesToStartingPositionsOnCancelOrFail:", 1u8 => u8);
                Ok(())
            });
            objc_autoreleasePoolPop(pool);
            r
        };
        r
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, IBindCtx, IDataObject};
    use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{ILFindLastID, ILFree, IShellFolder, SHBindToParent, SHDoDragDrop, SHParseDisplayName};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_LBUTTONUP};
    use windows::core::PCWSTR;

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(Some(0)).collect()
    }

    struct Pidls(Vec<*mut ITEMIDLIST>);
    impl Drop for Pidls {
        fn drop(&mut self) {
            for p in &self.0 {
                // SAFETY: each was allocated by SHParseDisplayName and is freed once.
                unsafe { ILFree(Some(*p)) };
            }
        }
    }

    pub fn start(hwnd: isize, paths: Vec<PathBuf>) -> Result<(), String> {
        let hwnd = HWND(hwnd as *mut _);
        let fail = |what: &str, e: windows::core::Error| format!("Couldn't start dragging ({what}: {}).", e.message());
        // Items from one folder go in one data object (a selection always is).
        let parent = paths[0].parent().map(|p| p.to_path_buf());
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p.parent().map(|q| q.to_path_buf()) == parent).collect();
        // SAFETY: shell calls on this (OLE-initialised) UI thread, with ID lists that live until
        // the drag is over.
        let effect = unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mut pidls = Pidls(Vec::new());
            for p in &paths {
                let w = wide(p.as_os_str());
                let mut pidl = std::ptr::null_mut();
                SHParseDisplayName(PCWSTR(w.as_ptr()), None::<&IBindCtx>, &mut pidl, 0, None).map_err(|e| fail(&p.display().to_string(), e))?;
                pidls.0.push(pidl);
            }
            let folder: IShellFolder = SHBindToParent(pidls.0[0], None).map_err(|e| fail("folder", e))?;
            let children: Vec<*const ITEMIDLIST> = pidls.0.iter().map(|p| ILFindLastID(*p) as *const _).collect();
            let data: IDataObject = folder.GetUIObjectOf(hwnd, &children, None).map_err(|e| fail("items", e))?;
            // The shell supplies the drag picture and the drop source; this returns at the drop.
            let r = SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK);
            // The drag kept the mouse-up: give it back so the window doesn't think it's held.
            let _ = PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), LPARAM(0));
            r.map_err(|e| fail("drag", e))?
        };
        let how = if effect.contains(DROPEFFECT_MOVE) {
            Outcome::Moved
        } else if effect == DROPEFFECT(0) {
            // A move the target did itself (Explorer on the same drive, the Recycle Bin) can
            // report "none"; the app looks at whether the items are still there.
            Outcome::Nothing
        } else {
            Outcome::Copied
        };
        ended(paths, how);
        Ok(())
    }
}
