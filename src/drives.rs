//! Lists the drives / mounted volumes that can be scanned, with their capacity.

use std::path::PathBuf;

pub struct Drive {
    /// Root to scan, e.g. `D:\` or `/home`.
    pub root: PathBuf,
    /// Volume label or filesystem type, may be empty.
    pub label: String,
    pub total: u64,
    pub free: u64,
    pub network: bool,
}

impl Drive {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}

#[cfg(windows)]
pub fn list() -> Vec<Drive> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    // GetDriveTypeW return values (winbase.h).
    const DRIVE_UNKNOWN: u32 = 0;
    const DRIVE_NO_ROOT_DIR: u32 = 1;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if kind == DRIVE_UNKNOWN || kind == DRIVE_NO_ROOT_DIR || kind == DRIVE_CDROM {
            continue;
        }
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
            continue; // not ready, e.g. an empty card reader or a disconnected share
        }
        let mut name = [0u16; 261];
        let ok = unsafe {
            GetVolumeInformationW(
                wide.as_ptr(),
                name.as_mut_ptr(),
                name.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        let label = if ok != 0 {
            let len = name.iter().position(|&c| c == 0).unwrap_or(0);
            String::from_utf16_lossy(&name[..len])
        } else {
            String::new()
        };
        out.push(Drive { root: root.into(), label, total, free, network: kind == DRIVE_REMOTE });
    }
    out
}

#[cfg(unix)]
pub fn list() -> Vec<Drive> {
    // Real filesystems from the mount table (Linux), or / plus /Volumes/* (macOS).
    let mut mounts: Vec<(PathBuf, String)> = Vec::new();
    if let Ok(s) = std::fs::read_to_string("/proc/self/mounts") {
        const SKIP: &[&str] = &[
            "proc", "sysfs", "devtmpfs", "devpts", "tmpfs", "cgroup", "cgroup2", "securityfs", "pstore", "debugfs",
            "tracefs", "configfs", "fusectl", "mqueue", "hugetlbfs", "bpf", "autofs", "binfmt_misc", "overlay",
            "squashfs", "nsfs", "efivarfs", "ramfs", "rpc_pipefs", "fuse.portal", "fuse.gvfsd-fuse",
        ];
        for line in s.lines() {
            let mut f = line.split_whitespace();
            let (Some(_dev), Some(dir), Some(fs)) = (f.next(), f.next(), f.next()) else { continue };
            if SKIP.contains(&fs) || dir.starts_with("/snap/") || dir.starts_with("/boot/efi") {
                continue;
            }
            mounts.push((PathBuf::from(dir.replace("\\040", " ")), fs.to_string()));
        }
    } else {
        mounts.push(("/".into(), String::new()));
        if let Ok(rd) = std::fs::read_dir("/Volumes") {
            for e in rd.flatten() {
                mounts.push((e.path(), String::new()));
            }
        }
    }
    let mut out: Vec<Drive> = Vec::new();
    for (root, fs) in mounts {
        let Some((total, free)) = statvfs(&root) else { continue };
        if total == 0 || out.iter().any(|d| d.root == root) {
            continue;
        }
        let network = matches!(fs.as_str(), "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "fuse.sshfs" | "9p");
        out.push(Drive { root, label: fs, total, free, network });
    }
    out
}

#[cfg(unix)]
fn statvfs(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = s.f_frsize as u64;
    Some((s.f_blocks as u64 * frsize, s.f_bavail as u64 * frsize))
}

#[cfg(not(any(windows, unix)))]
pub fn list() -> Vec<Drive> {
    Vec::new()
}
