//! Running with administrator rights, so folders Windows hides from ordinary programs
//! (other users' profiles, System Volume Information, ...) can be read.
//!
//! The choice is kept in its own small file rather than with the other settings, because it has
//! to be read before the window exists and written before a restart.

use std::path::PathBuf;

/// What to do at start when dirmap isn't running as administrator.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum AdminMode {
    /// Ask once (the first-run prompt).
    #[default]
    Ask,
    /// Restart as administrator straight away.
    Always,
    /// Stay as we are and don't ask.
    Never,
}

/// Command-line marker on the restarted copy, so a declined or failed elevation can't loop.
pub const ELEVATED_FLAG: &str = "--elevated";

/// Whether this platform has a "restart as administrator" we can trigger (Windows' UAC prompt).
pub fn can_elevate() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

#[cfg(unix)]
pub fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(any(windows, unix)))]
pub fn is_elevated() -> bool {
    false
}

fn pref_file() -> Option<PathBuf> {
    eframe::storage_dir("dirmap").map(|d| d.join("admin-mode"))
}

pub fn load_mode() -> AdminMode {
    match pref_file().and_then(|f| std::fs::read_to_string(f).ok()).as_deref().map(str::trim) {
        Some("always") => AdminMode::Always,
        Some("never") => AdminMode::Never,
        _ => AdminMode::Ask,
    }
}

pub fn save_mode(mode: AdminMode) {
    let Some(f) = pref_file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = match mode {
        AdminMode::Ask => "ask",
        AdminMode::Always => "always",
        AdminMode::Never => "never",
    };
    let _ = std::fs::write(f, text);
}

/// Start a second copy of dirmap as administrator (Windows shows its UAC prompt), with the same
/// command line. Returns true if it started, in which case this copy should close; false if the
/// user said no or it isn't possible here.
#[cfg(windows)]
pub fn relaunch_elevated() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let Ok(exe) = std::env::current_exe() else { return false };
    let mut params: Vec<String> = std::env::args().skip(1).filter(|a| a != ELEVATED_FLAG).map(|a| quote_arg(&a)).collect();
    params.push(ELEVATED_FLAG.to_string());
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (verb, file, params) = (wide("runas".as_ref()), wide(exe.as_os_str()), wide(params.join(" ").as_ref()));
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), params.as_ptr(), std::ptr::null(), SW_SHOWNORMAL) };
    // ShellExecute reports success as a value above 32.
    r as isize > 32
}

#[cfg(not(windows))]
pub fn relaunch_elevated() -> bool {
    false
}

/// Quote one argument the way Windows programs parse their command line: wrap in quotes, and
/// double any backslashes that come right before a quote (so `C:\` doesn't swallow the closing one).
#[cfg_attr(not(windows), allow(dead_code))]
fn quote_arg(a: &str) -> String {
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in a.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes + 1));
                slashes = 0;
            }
            _ => slashes = 0,
        }
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\', slashes));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::quote_arg;

    #[test]
    fn quoting() {
        assert_eq!(quote_arg(r"C:\"), r#""C:\\""#);
        assert_eq!(quote_arg(r"D:\My Files"), r#""D:\My Files""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
    }
}
