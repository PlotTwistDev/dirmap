//! Windows' own right-click menu for files and folders: the one Explorer shows under
//! "Show more options", with everything other apps add to it (7-Zip, Notepad++, PowerToys,
//! antivirus scanners, Send to, Give access to…).
//!
//! The shell builds the menu (IContextMenu on the items' folder) and Windows shows it
//! (TrackPopupMenuEx). While it's open, dirmap's window is subclassed so the messages that
//! draw icons and fill submenus reach IContextMenu2/3. The menu is modal: the frame waits
//! until it closes, as with any native menu.

use std::path::{Path, PathBuf};

/// What happened in the menu.
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Picked {
    /// Closed without a choice.
    Nothing,
    /// Rename: dirmap does it itself, in the list.
    Rename,
    /// The shell carried out the command (it may still be running, e.g. a copy).
    Done,
}

#[cfg(windows)]
pub fn show(hwnd: isize, paths: &[PathBuf], extended: bool) -> Result<Picked, String> {
    imp::show(hwnd, paths, extended)
}

#[cfg(not(windows))]
pub fn show(_hwnd: isize, _paths: &[PathBuf], _extended: bool) -> Result<Picked, String> {
    Err("The system menu is only on Windows.".into())
}

/// The folder the items are in, for the command's working directory.
#[cfg_attr(not(windows), allow(dead_code))]
fn folder_of(paths: &[PathBuf]) -> Option<&Path> {
    paths.first().and_then(|p| p.parent())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::cell::RefCell;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, IBindCtx};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        CMF_CANRENAME, CMF_EXPLORE, CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, DefSubclassProc, GCS_VERBW,
        IContextMenu, IContextMenu2, IContextMenu3, ILFindLastID, ILFree, IShellFolder, RemoveWindowSubclass, SHBindToParent, SHParseDisplayName, SetWindowSubclass,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreatePopupMenu, DestroyMenu, GetCursorPos, SW_SHOWNORMAL, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP,
        WM_MEASUREITEM, WM_MENUCHAR,
    };
    use windows::core::{Interface, PCSTR, PCWSTR, PSTR};

    /// The first command id the shell may use; 0 means "no choice".
    const FIRST: u32 = 1;
    const SUBCLASS_ID: usize = 0xd17a;
    /// CMIC_MASK_UNICODE (shobjidl.h): the W strings are used.
    const CMIC_MASK_UNICODE: u32 = 0x4000;

    enum Handler {
        Three(IContextMenu3),
        Two(IContextMenu2),
    }

    thread_local! {
        /// The open menu's handler, for the subclass procedure.
        static OPEN: RefCell<Option<Handler>> = const { RefCell::new(None) };
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(Some(0)).collect()
    }

    /// Item ID lists, freed when dropped.
    struct Pidls(Vec<*mut ITEMIDLIST>);
    impl Drop for Pidls {
        fn drop(&mut self) {
            for p in &self.0 {
                unsafe { ILFree(Some(*p)) };
            }
        }
    }

    pub fn show(hwnd: isize, paths: &[PathBuf], extended: bool) -> Result<Picked, String> {
        if paths.is_empty() || hwnd == 0 {
            return Ok(Picked::Nothing);
        }
        let hwnd = HWND(hwnd as *mut _);
        let fail = |what: &str, e: windows::core::Error| format!("Couldn't show the Windows menu ({what}: {}).", e.message());
        unsafe {
            // Usually already done by the window (drag and drop needs OLE); harmless if so.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mut pidls = Pidls(Vec::new());
            for p in paths {
                let w = wide(p.as_os_str());
                let mut pidl = std::ptr::null_mut();
                SHParseDisplayName(PCWSTR(w.as_ptr()), None::<&IBindCtx>, &mut pidl, 0, None).map_err(|e| fail(&p.display().to_string(), e))?;
                pidls.0.push(pidl);
            }
            // The items' folder, and each item relative to it. A selection is always in one folder.
            let folder: IShellFolder = SHBindToParent(pidls.0[0], None).map_err(|e| fail("folder", e))?;
            let children: Vec<*const ITEMIDLIST> = pidls.0.iter().map(|p| ILFindLastID(*p) as *const _).collect();
            let menu: IContextMenu = folder.GetUIObjectOf(hwnd, &children, None).map_err(|e| fail("menu", e))?;

            let hmenu = CreatePopupMenu().map_err(|e| fail("popup", e))?;
            let flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME | if extended { CMF_EXTENDEDVERBS } else { 0 };
            let hr = menu.QueryContextMenu(hmenu, 0, FIRST, 0x7fff, flags);
            if hr.is_err() {
                let _ = DestroyMenu(hmenu);
                return Err(fail("items", hr.into()));
            }

            // Submenus (Send to, Give access to) and icons come from the handler, through our window.
            let handler = if let Ok(m3) = menu.cast::<IContextMenu3>() {
                Some(Handler::Three(m3))
            } else {
                menu.cast::<IContextMenu2>().ok().map(Handler::Two)
            };
            OPEN.with(|o| *o.borrow_mut() = handler);
            let hooked = SetWindowSubclass(hwnd, Some(hook), SUBCLASS_ID, 0).as_bool();

            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;

            if hooked {
                let _ = RemoveWindowSubclass(hwnd, Some(hook), SUBCLASS_ID);
            }
            OPEN.with(|o| *o.borrow_mut() = None);

            let result = if cmd < FIRST {
                Ok(Picked::Nothing)
            } else {
                let id = (cmd - FIRST) as usize;
                if verb(&menu, id).is_some_and(|v| v.eq_ignore_ascii_case("rename")) {
                    Ok(Picked::Rename)
                } else {
                    let dir = folder_of(paths).map(|d| wide(d.as_os_str()));
                    let info = CMINVOKECOMMANDINFOEX {
                        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                        fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
                        hwnd,
                        // The command by number (MAKEINTRESOURCE), as the shell asks for.
                        lpVerb: PCSTR(id as *const u8),
                        lpVerbW: PCWSTR(id as *const u16),
                        lpDirectoryW: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
                        nShow: SW_SHOWNORMAL.0,
                        ptInvoke: pt,
                        ..Default::default()
                    };
                    menu.InvokeCommand(&info as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO)
                        .map(|_| Picked::Done)
                        .map_err(|e| format!("That command didn't work: {}", e.message()))
                }
            };
            let _ = DestroyMenu(hmenu);
            result
        }
    }

    /// The language-independent name of command `id` ("rename", "delete", "properties"…), if it has one.
    unsafe fn verb(menu: &IContextMenu, id: usize) -> Option<String> {
        let mut buf = [0u16; 128];
        unsafe { menu.GetCommandString(id, GCS_VERBW, None, PSTR(buf.as_mut_ptr().cast()), buf.len() as u32) }.ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }

    /// While the menu is open: pass the owner-drawn and submenu messages to the shell's handler.
    unsafe extern "system" fn hook(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
        if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
            // Take a reference out first: the handler may pump messages and land back here.
            let handler = OPEN.with(|o| match &*o.borrow() {
                Some(Handler::Three(m)) => Some(Handler::Three(m.clone())),
                Some(Handler::Two(m)) => Some(Handler::Two(m.clone())),
                None => None,
            });
            let handled = match &handler {
                Some(Handler::Three(m)) => {
                    let mut r = LRESULT(0);
                    unsafe { m.HandleMenuMsg2(msg, wp, lp, Some(&mut r)) }.ok().map(|_| r)
                }
                Some(Handler::Two(m)) if msg != WM_MENUCHAR => unsafe { m.HandleMenuMsg(msg, wp, lp) }.ok().map(|_| LRESULT(0)),
                _ => None,
            };
            if let Some(r) = handled {
                return match msg {
                    WM_MENUCHAR => r,
                    WM_INITMENUPOPUP => LRESULT(0),
                    // WM_DRAWITEM and WM_MEASUREITEM: handled.
                    _ => LRESULT(1),
                };
            }
        }
        unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
    }
}
