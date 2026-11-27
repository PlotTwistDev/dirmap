//! Network drives and server locations, through the operating system: mapped drives and UNC
//! paths on Windows, the system mounter on macOS, GVFS on Linux.
//!
//! dirmap never asks for or keeps a password here. Windows shows its own sign-in box (and can
//! save to Credential Manager); macOS uses the Keychain; GVFS uses the desktop's keyring.
//! Every function can block on the network, so call them off the UI thread.

use std::path::{Path, PathBuf};

/// A network location the system knows about.
#[derive(Clone, Debug, PartialEq)]
pub struct NetPlace {
    /// What to open: `Z:\`, `\\server\share`, or a mount point.
    pub path: PathBuf,
    /// `\\server\share` or `smb://server/share`, when known.
    pub remote: String,
    /// Mapped but not connected right now (Windows remembers mappings across restarts).
    pub disconnected: bool,
}

/// Mapped drives (Windows), or GVFS network mounts (Linux). macOS network volumes are already
/// in the drive list.
pub fn list() -> Vec<NetPlace> {
    imp::list()
}

/// Connect to `\\server\share` (or `smb://server/share`). On Windows `letter` maps it to a drive
/// letter (`None`: the first free one from Z: down) and `remember` keeps the mapping after a
/// restart; the system asks for a password if one is needed. Returns where it can be opened.
pub fn connect(target: &str, letter: Option<char>, remember: bool) -> Result<PathBuf, String> {
    let lower = target.trim().to_lowercase();
    if ["sftp://", "ftp://", "ftps://", "webdav://", "dav://", "davs://"].iter().any(|p| lower.starts_with(p)) {
        return Err("SFTP, FTP and WebDAV aren't built in yet. Connect to Server handles SMB shares for now.".into());
    }
    imp::connect(target, letter, remember)
}

/// Disconnect a mapped drive or unmount a network location.
pub fn disconnect(path: &Path) -> Result<(), String> {
    imp::disconnect(path)
}

/// The `\\server\share\…` form of a path on a mapped drive (Copy UNC Path).
pub fn unc_path(path: &Path) -> Option<PathBuf> {
    imp::unc_path(path)
}

/// `smb://server/share/dir` or `\\server\share\dir` as (server, share, rest), for either form.
pub fn parse_smb(s: &str) -> Option<(String, String, String)> {
    let s = s.trim();
    let rest = if let Some(r) = s.strip_prefix("smb://").or_else(|| s.strip_prefix("SMB://")) {
        r.to_string()
    } else {
        s.strip_prefix(r"\\").or_else(|| s.strip_prefix("//"))?.replace('\\', "/")
    };
    // smb://user@server/share: the user goes to the system's sign-in, not to us.
    let rest = rest.rsplit_once('@').map(|(_, r)| r.to_string()).unwrap_or(rest);
    let mut parts = rest.splitn(3, '/');
    let server = parts.next().filter(|s| !s.is_empty())?.to_string();
    let share = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").trim_end_matches('/').to_string();
    Some((server, share, path))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::NetworkManagement::WNet::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn from_wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut n = 0;
        unsafe {
            while *p.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        }
    }

    fn message(code: u32) -> String {
        match code {
            5 => "Access denied.".into(),
            53 | 67 => "The server or share wasn't found.".into(),
            85 => "That drive letter is already in use.".into(),
            86 | 1326 => "The user name or password is wrong.".into(),
            1219 => "You're already connected to this server as another user. Disconnect that first.".into(),
            1223 => "Cancelled.".into(),
            2250 => "That isn't a network connection.".into(),
            2401 | 2404 => "Files are open on it. Close them first.".into(),
            c => format!("Windows error {c}."),
        }
    }

    pub fn list() -> Vec<NetPlace> {
        let mut out = Vec::new();
        for scope in [RESOURCE_CONNECTED, RESOURCE_REMEMBERED] {
            let mut h = std::ptr::null_mut();
            if unsafe { WNetOpenEnumW(scope, RESOURCETYPE_DISK, 0, std::ptr::null(), &mut h) } != 0 {
                continue;
            }
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                let mut count = u32::MAX;
                let mut size = buf.len() as u32;
                let r = unsafe { WNetEnumResourceW(h, &mut count, buf.as_mut_ptr().cast(), &mut size) };
                if r != 0 {
                    break;
                }
                let items = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const NETRESOURCEW, count as usize) };
                for r in items {
                    let local = from_wide(r.lpLocalName);
                    let remote = from_wide(r.lpRemoteName);
                    let path = if local.is_empty() { PathBuf::from(&remote) } else { PathBuf::from(format!("{local}\\")) };
                    if remote.is_empty() || out.iter().any(|p: &NetPlace| p.path == path) {
                        continue;
                    }
                    out.push(NetPlace { path, remote, disconnected: scope == RESOURCE_REMEMBERED });
                }
            }
            unsafe { WNetCloseEnum(h) };
        }
        out
    }

    pub fn connect(target: &str, letter: Option<char>, remember: bool) -> Result<PathBuf, String> {
        let (server, share, _) = parse_smb(target).ok_or("Type a location like \\\\server\\share.")?;
        if share.is_empty() {
            return Err("Name a share too, like \\\\server\\share.".into());
        }
        let remote = format!(r"\\{server}\{share}");
        let letter = match letter {
            Some(l) => Some(l.to_ascii_uppercase()),
            None => {
                let used = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
                (b'D'..=b'Z').rev().map(char::from).find(|c| used & (1 << (*c as u8 - b'A')) == 0)
            }
        };
        let local = letter.map(|l| format!("{l}:"));
        let (wr, wl) = (wide(&remote), local.as_deref().map(wide));
        let nr = NETRESOURCEW {
            dwScope: 0,
            dwType: RESOURCETYPE_DISK,
            dwDisplayType: 0,
            dwUsage: 0,
            lpLocalName: wl.as_ref().map_or(std::ptr::null_mut(), |w| w.as_ptr() as *mut u16),
            lpRemoteName: wr.as_ptr() as *mut u16,
            lpComment: std::ptr::null_mut(),
            lpProvider: std::ptr::null_mut(),
        };
        let mut flags = CONNECT_INTERACTIVE;
        if remember {
            flags |= CONNECT_UPDATE_PROFILE;
        }
        let r = unsafe { WNetAddConnection2W(&nr, std::ptr::null(), std::ptr::null(), flags) };
        if r != 0 {
            return Err(format!("Couldn't connect to {remote}: {}", message(r)));
        }
        Ok(match local {
            Some(l) => PathBuf::from(format!("{l}\\")),
            None => PathBuf::from(remote),
        })
    }

    pub fn disconnect(path: &Path) -> Result<(), String> {
        let s = path.display().to_string();
        let name = if s.len() >= 2 && s.as_bytes()[1] == b':' { s[..2].to_string() } else { s.trim_end_matches('\\').to_string() };
        let r = unsafe { WNetCancelConnection2W(wide(&name).as_ptr(), CONNECT_UPDATE_PROFILE, 0) };
        if r != 0 { Err(format!("Couldn't disconnect {name}: {}", message(r))) } else { Ok(()) }
    }

    pub fn unc_path(path: &Path) -> Option<PathBuf> {
        let s = path.display().to_string();
        if s.starts_with(r"\\") {
            return Some(path.to_path_buf());
        }
        let w = wide(&s);
        let mut buf = vec![0u8; 2048];
        let mut size = buf.len() as u32;
        let r = unsafe { WNetGetUniversalNameW(w.as_ptr(), UNIVERSAL_NAME_INFO_LEVEL, buf.as_mut_ptr().cast(), &mut size) };
        if r != 0 {
            return None;
        }
        let info = unsafe { &*(buf.as_ptr() as *const UNIVERSAL_NAME_INFOW) };
        Some(PathBuf::from(from_wide(info.lpUniversalName)))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub fn list() -> Vec<NetPlace> {
        Vec::new()
    }

    pub fn connect(target: &str, _letter: Option<char>, _remember: bool) -> Result<PathBuf, String> {
        let (server, share, _) = parse_smb(target).ok_or("Type a location like smb://server/share.")?;
        let url = format!("smb://{server}/{share}");
        // The system mounter asks for a password (offering the Keychain) and mounts in /Volumes.
        let ok = std::process::Command::new("open").arg(&url).status().map_err(|e| e.to_string())?.success();
        if !ok {
            return Err(format!("Couldn't open {url}"));
        }
        Ok(PathBuf::from("/Volumes").join(if share.is_empty() { server } else { share }))
    }

    pub fn disconnect(path: &Path) -> Result<(), String> {
        let out = std::process::Command::new("diskutil").arg("unmount").arg(path).output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    }

    pub fn unc_path(_path: &Path) -> Option<PathBuf> {
        None
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;

    /// GVFS mounts its network locations under /run/user/<uid>/gvfs.
    fn gvfs() -> PathBuf {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/run/user/{uid}/gvfs"))
    }

    pub fn list() -> Vec<NetPlace> {
        let Ok(rd) = std::fs::read_dir(gvfs()) else { return Vec::new() };
        let mut out: Vec<NetPlace> = rd
            .flatten()
            // Phones and cameras are listed with the drives.
            .filter(|e| crate::drives::gvfs_phone(&e.file_name().to_string_lossy()).is_none())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                NetPlace { remote: gvfs_remote(&name).unwrap_or_else(|| name.clone()), path: e.path(), disconnected: false }
            })
            .collect();
        out.sort_by(|a, b| a.remote.cmp(&b.remote));
        out
    }

    pub fn connect(target: &str, _letter: Option<char>, _remember: bool) -> Result<PathBuf, String> {
        let (server, share, _) = parse_smb(target).ok_or("Type a location like smb://server/share.")?;
        let url = format!("smb://{server}/{share}");
        let out = std::process::Command::new("gio").args(["mount", &url]).stdin(std::process::Stdio::null()).output();
        match out {
            Err(_) => Err("Connecting needs GVFS (the gio command). Mount the share with your system instead, then open its folder.".into()),
            Ok(o) if !o.status.success() && !String::from_utf8_lossy(&o.stderr).contains("already mounted") => Err(format!(
                "Couldn't mount {url}: {}. If it needs a password, connect once from your desktop's file manager; it then appears here.",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Ok(_) => Ok(gvfs().join(format!("smb-share:server={},share={}", server.to_lowercase(), share.to_lowercase()))),
        }
    }

    pub fn disconnect(path: &Path) -> Result<(), String> {
        let out = std::process::Command::new("gio").arg("mount").arg("-u").arg(path).output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    }

    pub fn unc_path(_path: &Path) -> Option<PathBuf> {
        None
    }

    /// `smb-share:server=nas,share=media` → `smb://nas/media`.
    pub fn gvfs_remote(name: &str) -> Option<String> {
        let (kind, rest) = name.split_once(':')?;
        let mut server = None;
        let mut share = None;
        let mut host = None;
        for kv in rest.split(',') {
            match kv.split_once('=')? {
                ("server", v) => server = Some(v),
                ("share", v) => share = Some(v),
                ("host", v) => host = Some(v),
                _ => {}
            }
        }
        Some(match kind {
            "smb-share" => format!("smb://{}/{}", server?, share?),
            "sftp" => format!("sftp://{}", host?),
            k => format!("{k}://{}", host.or(server)?),
        })
    }

    #[cfg(test)]
    #[test]
    fn gvfs_names() {
        assert_eq!(gvfs_remote("smb-share:server=nas,share=media").as_deref(), Some("smb://nas/media"));
        assert_eq!(gvfs_remote("sftp:host=box.local,user=rich").as_deref(), Some("sftp://box.local"));
        assert_eq!(gvfs_remote("junk"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smb_locations() {
        assert_eq!(parse_smb(r"\\nas\media\shows"), Some(("nas".into(), "media".into(), "shows".into())));
        assert_eq!(parse_smb("smb://rich@nas/media/"), Some(("nas".into(), "media".into(), "".into())));
        assert_eq!(parse_smb("smb://nas"), Some(("nas".into(), "".into(), "".into())));
        assert_eq!(parse_smb("/mnt/nas"), None);
        assert_eq!(parse_smb("smb://"), None);
    }
}
