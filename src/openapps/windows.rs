//! Windows: the shell's own list of apps registered for an extension (`SHAssocEnumHandlers`,
//! what Explorer's Open with menu shows), opened through the handler itself, the same as
//! Explorer does.

use super::{Handler, Type};
use std::path::PathBuf;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, IBindCtx, IDataObject};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ASSOC_FILTER_RECOMMENDED, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_EXECUTABLE, AssocQueryStringW, BHID_DataObject, IAssocHandler, ILFree, IShellItemArray, SHAssocEnumHandlers,
    SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
};
use windows::core::{PCWSTR, PWSTR};

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(Some(0)).collect()
}

/// A string the shell allocated, read and freed.
fn take(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated string from the shell, freed once with the allocator it came from.
    unsafe {
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// The extension the shell is asked about.
fn ext_of(t: &Type) -> Option<String> {
    match t {
        Type::Ext(e) => Some(format!(".{e}")),
        Type::Text => Some(".txt".into()),
        Type::Folder | Type::Unknown => None,
    }
}

/// The handlers for an extension, as the shell gives them.
fn each(ext: &str, mut f: impl FnMut(IAssocHandler) -> bool) {
    let w = wide(ext.as_ref());
    // SAFETY: shell calls with a valid string; each handler is released when dropped.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(en) = SHAssocEnumHandlers(PCWSTR(w.as_ptr()), ASSOC_FILTER_RECOMMENDED) else { return };
        loop {
            let mut one = [None];
            let mut n = 0u32;
            if en.Next(&mut one, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let Some(h) = one[0].take() else { break };
            if !f(h) {
                break;
            }
        }
    }
}

/// The program the system opens the extension with.
fn default_exe(ext: &str) -> Option<String> {
    let w = wide(ext.as_ref());
    let mut buf = vec![0u16; 1024];
    let mut n = buf.len() as u32;
    // SAFETY: the buffer is as long as said.
    let r = unsafe { AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_EXECUTABLE, PCWSTR(w.as_ptr()), PCWSTR::null(), Some(PWSTR(buf.as_mut_ptr())), &mut n) };
    if r.is_err() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

pub fn handlers(t: &Type) -> Vec<Handler> {
    let Some(ext) = ext_of(t) else { return Vec::new() };
    let def = default_exe(&ext).map(|d| d.to_lowercase());
    let mut out = Vec::new();
    each(&ext, |h| {
        // SAFETY: calls on a live handler.
        let (id, name) = unsafe { (h.GetName().ok().and_then(take), h.GetUIName().ok().and_then(take)) };
        if let (Some(id), Some(name)) = (id, name) {
            let default = def.as_deref() == Some(id.to_lowercase().as_str());
            out.push(Handler { app: PathBuf::from(&id), id, name, default });
        }
        true
    });
    out
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

pub fn open(h: &Handler, files: &[PathBuf]) -> Result<(), String> {
    let ext = files.first().and_then(|f| f.extension()).map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_else(|| ".txt".into());
    let mut found = None;
    each(&ext, |x| {
        // SAFETY: a call on a live handler.
        let id = unsafe { x.GetName().ok().and_then(take) };
        if id.as_deref() == Some(h.id.as_str()) {
            found = Some(x);
            false
        } else {
            true
        }
    });
    let handler = found.ok_or_else(|| format!("{} can't open these any more: it may have been uninstalled.", h.name))?;
    let fail = |e: windows::core::Error| format!("Windows couldn't open it with {}: {}", h.name, e.message());
    // SAFETY: shell calls with ID lists that live until the handler has been invoked.
    unsafe {
        let mut pidls = Pidls(Vec::new());
        for f in files {
            let w = wide(f.as_os_str());
            let mut pidl = std::ptr::null_mut();
            SHParseDisplayName(PCWSTR(w.as_ptr()), None::<&IBindCtx>, &mut pidl, 0, None).map_err(fail)?;
            pidls.0.push(pidl);
        }
        let list: Vec<*const ITEMIDLIST> = pidls.0.iter().map(|p| *p as *const _).collect();
        let items: IShellItemArray = SHCreateShellItemArrayFromIDLists(&list).map_err(fail)?;
        let data: IDataObject = items.BindToHandler(None::<&IBindCtx>, &BHID_DataObject).map_err(fail)?;
        handler.Invoke(&data).map_err(fail)
    }
}
