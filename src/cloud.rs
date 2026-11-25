//! Which folders belong to a cloud storage service, found from where providers put them rather
//! than by asking each provider. Detection reads the mount table, a few well-known folders and
//! (on Windows) the list of sync roots Windows itself keeps; it never lists or opens anything
//! inside a cloud folder, so it can't make a provider download or enumerate files.
//!
//! What a file's own metadata says (online only, partly downloaded, pinned) is read by
//! `scan::on_disk`; this module only says which service a folder belongs to and how far its
//! "size on disk" can be trusted.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A cloud storage service, as named to the user.
#[cfg_attr(windows, allow(dead_code))]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Provider {
    GoogleDrive,
    OneDrive,
    SharePoint,
    ICloud,
    Dropbox,
    Box,
    Nextcloud,
    OwnCloud,
    PCloud,
    Mega,
    Synology,
    Rclone,
    AmazonS3,
    GoogleCloudStorage,
    AzureBlob,
    WebDav,
    /// Something registered as a sync root that isn't one of the above.
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    Other,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::GoogleDrive => "Google Drive",
            Provider::OneDrive => "OneDrive",
            Provider::SharePoint => "SharePoint",
            Provider::ICloud => "iCloud Drive",
            Provider::Dropbox => "Dropbox",
            Provider::Box => "Box",
            Provider::Nextcloud => "Nextcloud",
            Provider::OwnCloud => "ownCloud",
            Provider::PCloud => "pCloud",
            Provider::Mega => "MEGA",
            Provider::Synology => "Synology Drive",
            Provider::Rclone => "rclone",
            Provider::AmazonS3 => "Amazon S3",
            Provider::GoogleCloudStorage => "Google Cloud Storage",
            Provider::AzureBlob => "Azure Blob Storage",
            Provider::WebDav => "WebDAV",
            Provider::Other => "Cloud storage",
        }
    }

    /// The provider a folder, sync-root id or mount source is named after, if any. Matching is
    /// on the name alone; `Other` is never returned.
    pub fn from_name(name: &str) -> Option<Provider> {
        let n = name.to_lowercase();
        let has = |s: &str| n.contains(s);
        // A word that starts the name or follows a separator, so "box" doesn't match "Xbox".
        let word = |s: &str| n == s || n.starts_with(&format!("{s}-")) || n.starts_with(&format!("{s}!")) || n.starts_with(&format!("{s} ")) || n.starts_with(&format!("{s}_"));
        Some(if has("onedrive-sharedlibraries") || has("sharepoint") {
            Provider::SharePoint
        } else if has("googledrive") || has("google drive") || has("google-drive") || has("drivefs") || n == "my drive" {
            Provider::GoogleDrive
        } else if has("onedrive") {
            Provider::OneDrive
        } else if has("icloud") || has("mobile documents") || has("clouddocs") {
            Provider::ICloud
        } else if has("dropbox") {
            Provider::Dropbox
        } else if word("box") || n == "box sync" {
            Provider::Box
        } else if has("nextcloud") {
            Provider::Nextcloud
        } else if has("owncloud") {
            Provider::OwnCloud
        } else if has("pcloud") {
            Provider::PCloud
        } else if word("mega") || has("megasync") {
            Provider::Mega
        } else if has("synology") {
            Provider::Synology
        } else if has("rclone") {
            Provider::Rclone
        } else {
            return None;
        })
    }
}

/// How a provider keeps files on this computer, which says what "size on disk" means there.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    /// Files on demand (macOS File Provider, Windows Cloud Files): each file says whether its
    /// data is here, and size on disk is exact.
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    OnDemand,
    /// A plain folder kept fully in step with the cloud: everything in it is on this computer.
    Synced,
    /// A cloud drive mounted by a filesystem driver (FUSE, rclone, Google Drive's G: drive,
    /// WebDAV). Its files don't say how much of them is cached here, so the space they take on
    /// this computer is unknown.
    Mount,
}

impl Kind {
    pub fn describe(self) -> &'static str {
        match self {
            Kind::OnDemand => "files on demand",
            Kind::Synced => "synced folder",
            Kind::Mount => "mounted drive",
        }
    }
}

/// A folder that belongs to a cloud storage service.
#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub path: PathBuf,
    pub provider: Provider,
    pub kind: Kind,
}

impl Root {
    /// `scan::store` bits every file in this folder gets.
    pub fn bits(&self) -> u8 {
        use crate::scan::store;
        match self.kind {
            Kind::Mount => store::CLOUD | store::MOUNT,
            _ => store::CLOUD,
        }
    }
}

/// A path as compared here: forward slashes, no trailing slash, and lower case where the
/// filesystem usually ignores case (Windows, macOS).
pub fn norm(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/');
    let s = if s.is_empty() { "/" } else { s };
    if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s.to_string() }
}

fn inside(p: &str, root: &str) -> bool {
    p == root || (p.starts_with(root) && (root.ends_with('/') || p.as_bytes().get(root.len()) == Some(&b'/')))
}

/// The cloud folder holding `path` (the deepest, if they nest), from `roots`.
pub fn root_of<'a>(roots: &'a [Root], path: &Path) -> Option<&'a Root> {
    let p = norm(path);
    roots.iter().filter(|r| inside(&p, &norm(&r.path))).max_by_key(|r| norm(&r.path).len())
}

/// Cloud folders strictly inside `path`.
pub fn roots_below<'a>(roots: &'a [Root], path: &Path) -> Vec<&'a Root> {
    let p = norm(path);
    roots.iter().filter(|r| { let n = norm(&r.path); n != p && inside(&n, &p) }).collect()
}

static CACHE: Mutex<Option<Arc<Vec<Root>>>> = Mutex::new(None);

/// The cloud folders on this computer, found once and then remembered. Blocks the first time
/// (a few milliseconds; reads the mount table and registry), so the app calls [`warm`] at start.
pub fn roots() -> Arc<Vec<Root>> {
    if let Some(r) = CACHE.lock().unwrap().as_ref() {
        return r.clone();
    }
    let r = Arc::new(detect());
    *CACHE.lock().unwrap() = Some(r.clone());
    r
}

/// What's known so far, without waiting: empty until detection has run.
pub fn known() -> Arc<Vec<Root>> {
    CACHE.lock().unwrap().clone().unwrap_or_default()
}

/// Find the cloud folders in the background, and again later if `again` (a drive was mounted).
pub fn warm(again: bool) {
    if again {
        *CACHE.lock().unwrap() = None;
    }
    std::thread::spawn(|| {
        roots();
    });
}

/// Look for cloud folders now. Never reads inside one.
pub fn detect() -> Vec<Root> {
    let mut out: Vec<Root> = Vec::new();
    let mut add = |r: Root| {
        let n = norm(&r.path);
        if !out.iter().any(|o| norm(&o.path) == n) {
            out.push(r);
        }
    };
    for r in mounts() {
        add(r);
    }
    for r in platform_roots() {
        add(r);
    }
    if let Some(home) = crate::fsys::places::home() {
        for (name, provider, kind) in HOME_FOLDERS {
            let p = home.join(name);
            // symlink_metadata: a link into another cloud folder is found there instead, and
            // the scan doesn't follow links anyway.
            if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
                add(Root { path: p, provider: *provider, kind: *kind });
            }
        }
    }
    out
}

/// Folders sync clients make in the home folder by default.
const HOME_FOLDERS: &[(&str, Provider, Kind)] = &[
    ("Dropbox", Provider::Dropbox, Kind::Synced),
    ("Nextcloud", Provider::Nextcloud, Kind::Synced),
    ("ownCloud", Provider::OwnCloud, Kind::Synced),
    ("MEGA", Provider::Mega, Kind::Synced),
    ("MEGAsync", Provider::Mega, Kind::Synced),
    ("SynologyDrive", Provider::Synology, Kind::Synced),
    ("Box Sync", Provider::Box, Kind::Synced),
    ("pCloudDrive", Provider::PCloud, Kind::Mount),
    ("pCloud Drive", Provider::PCloud, Kind::Mount),
    #[cfg(target_os = "linux")]
    ("OneDrive", Provider::OneDrive, Kind::Synced),
    #[cfg(target_os = "linux")]
    ("Google Drive", Provider::GoogleDrive, Kind::Synced),
    #[cfg(windows)]
    ("iCloudDrive", Provider::ICloud, Kind::OnDemand),
    #[cfg(windows)]
    ("Box", Provider::Box, Kind::OnDemand),
    #[cfg(windows)]
    ("My Drive", Provider::GoogleDrive, Kind::Synced),
];

/// A mounted filesystem that is a cloud drive, from its type and source as the mount table
/// gives them. Network shares (SMB, NFS, SSH) aren't cloud storage and return `None`.
#[cfg_attr(windows, allow(dead_code))]
pub fn classify_mount(fstype: &str, source: &str) -> Option<Provider> {
    let t = fstype.to_lowercase();
    let src = source.to_lowercase();
    let by_src = Provider::from_name(&src);
    Some(match t.as_str() {
        "fuse.rclone" | "rclone" => Provider::Rclone,
        "fuse.google-drive-ocamlfuse" => Provider::GoogleDrive,
        "fuse.onedriver" => Provider::OneDrive,
        "fuse.s3fs" | "fuse.goofys" | "fuse.mountpoint-s3" => Provider::AmazonS3,
        "fuse.gcsfuse" => Provider::GoogleCloudStorage,
        "fuse.blobfuse" | "fuse.blobfuse2" => Provider::AzureBlob,
        "davfs" | "fuse.davfs2" | "davfs2" | "webdav" => match by_src {
            Some(p) => p,
            None if src.contains("remote.php") => Provider::Nextcloud,
            None => Provider::WebDav,
        },
        "fuse.sshfs" | "fuse.gvfsd-fuse" | "fuse.portal" | "fusectl" | "fuse.lxcfs" | "fuse.snapfuse" | "fuse.appimagefs" => return None,
        _ if t.starts_with("fuse") || t.contains("fuse") => by_src?,
        _ => return None,
    })
}

/// Parse a Linux mount table (`/proc/self/mounts`) into cloud mounts.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_mounts(text: &str) -> Vec<Root> {
    let unescape = |s: &str| s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\");
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (src, dir, fs) = (f.next()?, f.next()?, f.next()?);
            let provider = classify_mount(fs, &unescape(src)).or_else(|| {
                // A FUSE mount named after its provider (pCloud mounts as plain "fuse").
                fs.starts_with("fuse").then(|| Provider::from_name(Path::new(&unescape(dir)).file_name()?.to_str()?)).flatten()
            })?;
            Some(Root { path: PathBuf::from(unescape(dir)), provider, kind: Kind::Mount })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn mounts() -> Vec<Root> {
    let mut out = std::fs::read_to_string("/proc/self/mounts").map(|s| parse_mounts(&s)).unwrap_or_default();
    // GNOME Online Accounts: each signed-in drive is a folder in the gvfs FUSE mount. Listing
    // the gvfs root only lists those connections, not their files.
    let uid = unsafe { libc::getuid() };
    if let Ok(rd) = std::fs::read_dir(format!("/run/user/{uid}/gvfs")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            let provider = if n.starts_with("google-drive:") {
                Provider::GoogleDrive
            } else if n.starts_with("onedrive:") {
                Provider::OneDrive
            } else if n.starts_with("dav:") || n.starts_with("davs:") {
                if n.contains("nextcloud") { Provider::Nextcloud } else { Provider::WebDav }
            } else {
                continue;
            };
            out.push(Root { path: e.path(), provider, kind: Kind::Mount });
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn mounts() -> Vec<Root> {
    use std::ffi::CStr;
    let mut out = Vec::new();
    let mut buf: *mut libc::statfs = std::ptr::null_mut();
    // SAFETY: getmntinfo fills `buf` with an array it owns of the returned length; MNT_NOWAIT
    // uses cached figures, so an unresponsive server can't stall this.
    let n = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
    for i in 0..n.max(0) as usize {
        // SAFETY: i < n, and the strings are NUL-terminated fixed arrays.
        let s = unsafe { &*buf.add(i) };
        let cs = |a: &[libc::c_char]| unsafe { CStr::from_ptr(a.as_ptr()) }.to_string_lossy().into_owned();
        let (fs, from, on) = (cs(&s.f_fstypename), cs(&s.f_mntfromname), cs(&s.f_mntonname));
        let t = fs.to_lowercase();
        let provider = if t.contains("fuse") {
            classify_mount("fuse", &from).or_else(|| Provider::from_name(Path::new(&on).file_name()?.to_str()?))
        } else if t == "webdav" {
            classify_mount("webdav", &from)
        } else {
            None
        };
        if let Some(provider) = provider {
            out.push(Root { path: PathBuf::from(on), provider, kind: Kind::Mount });
        }
    }
    out
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn mounts() -> Vec<Root> {
    Vec::new()
}

/// macOS: File Provider folders in ~/Library/CloudStorage (Google Drive, OneDrive, SharePoint,
/// Dropbox, Box and others) and iCloud Drive in ~/Library/Mobile Documents.
#[cfg(target_os = "macos")]
fn platform_roots() -> Vec<Root> {
    let mut out = Vec::new();
    let Some(home) = crate::fsys::places::home() else { return out };
    if let Ok(rd) = std::fs::read_dir(home.join("Library/CloudStorage")) {
        for e in rd.flatten() {
            if !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let provider = Provider::from_name(&name).unwrap_or(Provider::Other);
            out.push(Root { path: e.path(), provider, kind: Kind::OnDemand });
        }
    }
    let icloud = home.join("Library/Mobile Documents");
    if icloud.is_dir() {
        out.push(Root { path: icloud, provider: Provider::ICloud, kind: Kind::OnDemand });
    }
    out
}

/// Windows: the sync roots registered with Windows' Cloud Files (OneDrive, SharePoint, iCloud,
/// Dropbox, Box, Nextcloud, Google Drive in mirror mode…), OneDrive's own variables in case the
/// registry can't be read, and drive letters that are cloud drives (Google Drive's G:, pCloud).
#[cfg(windows)]
fn platform_roots() -> Vec<Root> {
    let mut out = sync_roots();
    for var in ["OneDriveCommercial", "OneDriveConsumer", "OneDrive"] {
        if let Some(p) = std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_dir()) {
            out.push(Root { path: p, provider: Provider::OneDrive, kind: Kind::OnDemand });
        }
    }
    // SharePoint libraries sync to %USERPROFILE%\<Organisation>, next to "OneDrive - <Organisation>".
    if let (Some(biz), Some(home)) = (std::env::var_os("OneDriveCommercial"), crate::fsys::places::home()) {
        let biz = PathBuf::from(biz);
        if let Some(org) = biz.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("OneDrive - ")) {
            let p = home.join(org);
            if p.is_dir() {
                out.push(Root { path: p, provider: Provider::SharePoint, kind: Kind::OnDemand });
            }
        }
    }
    out.extend(cloud_drive_letters());
    out
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
fn sync_roots() -> Vec<Root> {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ, REG_EXPAND_SZ};
    let mut out = Vec::new();
    // SAFETY: every buffer passed is valid for the length given with it; keys are closed.
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let base = wide(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager");
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, base.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return out;
        }
        for i in 0.. {
            let mut sub = [0u16; 512];
            let mut len = sub.len() as u32;
            if RegEnumKeyExW(key, i, sub.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) != 0 {
                break;
            }
            // e.g. "OneDrive!S-1-5-21-…!Business1|…", "Dropbox!S-1-5-21-…!…", "iCloudDrive!…".
            let id = String::from_utf16_lossy(&sub[..len as usize]);
            let mut roots: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(key, wide(&format!(r"{id}\UserSyncRoots")).as_ptr(), 0, KEY_READ, &mut roots) != 0 {
                continue;
            }
            for j in 0.. {
                let mut name = [0u16; 256];
                let mut nlen = name.len() as u32;
                let mut data = vec![0u16; 1024];
                let mut bytes = (data.len() * 2) as u32;
                let mut ty = 0u32;
                if RegEnumValueW(roots, j, name.as_mut_ptr(), &mut nlen, std::ptr::null(), &mut ty, data.as_mut_ptr().cast(), &mut bytes) != 0 {
                    break;
                }
                if ty != REG_SZ && ty != REG_EXPAND_SZ {
                    continue;
                }
                let end = data[..(bytes as usize / 2).min(data.len())].iter().position(|&c| c == 0).unwrap_or(bytes as usize / 2);
                let path = PathBuf::from(String::from_utf16_lossy(&data[..end]));
                if path.as_os_str().is_empty() || !path.is_dir() {
                    continue;
                }
                let folder = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut provider = Provider::from_name(id.split('!').next().unwrap_or(&id)).unwrap_or(Provider::Other);
                // A work OneDrive's SharePoint libraries are its other folders.
                if provider == Provider::OneDrive && id.contains("Business") && !folder.starts_with("OneDrive") {
                    provider = Provider::SharePoint;
                }
                out.push(Root { path, provider, kind: Kind::OnDemand });
            }
            RegCloseKey(roots);
        }
        RegCloseKey(key);
    }
    out
}

/// Drive letters whose volume label names a cloud provider ("Google Drive", "pCloud Drive").
/// Network drives are left alone: asking them for a label can stall.
#[cfg(windows)]
fn cloud_drive_letters() -> Vec<Root> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_RAMDISK: u32 = 6;
    let mut out = Vec::new();
    // SAFETY: plain calls with valid NUL-terminated paths and sized buffers.
    let mask = unsafe { GetLogicalDrives() };
    for i in 2..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let w = wide(&root);
        let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
        if kind != DRIVE_FIXED && kind != DRIVE_RAMDISK {
            continue;
        }
        let mut label = [0u16; 261];
        let ok = unsafe { GetVolumeInformationW(w.as_ptr(), label.as_mut_ptr(), label.len() as u32, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0) };
        if ok == 0 {
            continue;
        }
        let end = label.iter().position(|&c| c == 0).unwrap_or(0);
        let label = String::from_utf16_lossy(&label[..end]);
        if let Some(provider @ (Provider::GoogleDrive | Provider::PCloud)) = Provider::from_name(&label) {
            out.push(Root { path: PathBuf::from(root), provider, kind: Kind::Mount });
        }
    }
    out
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_roots() -> Vec<Root> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_providers_from_folder_and_root_names() {
        let p = |s| Provider::from_name(s);
        assert_eq!(p("GoogleDrive-someone@example.com"), Some(Provider::GoogleDrive));
        assert_eq!(p("OneDrive-Personal"), Some(Provider::OneDrive));
        assert_eq!(p("OneDrive-SharedLibraries-Contoso"), Some(Provider::SharePoint));
        assert_eq!(p("Dropbox"), Some(Provider::Dropbox));
        assert_eq!(p("Box-Box"), Some(Provider::Box));
        assert_eq!(p("iCloudDrive!S-1-5-21!x"), Some(Provider::ICloud));
        assert_eq!(p("Nextcloud-1"), Some(Provider::Nextcloud));
        assert_eq!(p("pCloudDrive"), Some(Provider::PCloud));
        assert_eq!(p("MEGA"), Some(Provider::Mega));
        assert_eq!(p("SynologyDrive"), Some(Provider::Synology));
        assert_eq!(p("Xbox"), None);
        assert_eq!(p("Megan's photos"), None);
        assert_eq!(p("Projects"), None);
    }

    #[test]
    fn finds_cloud_mounts_in_the_mount_table() {
        let table = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
gdrive: /home/rich/GDrive fuse.rclone rw,nosuid,nodev 0 0
pCloud.fs /home/rich/pCloudDrive fuse rw 0 0
https://cloud.example.org/remote.php/dav/files/rich /mnt/my\\040cloud davfs rw 0 0
nas:/export /mnt/nas nfs4 rw 0 0
//nas/share /mnt/share cifs rw 0 0
me@host:/srv /mnt/ssh fuse.sshfs rw 0 0
gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw 0 0
";
        let m = parse_mounts(table);
        let got: Vec<(&str, Provider)> = m.iter().map(|r| (r.path.to_str().unwrap(), r.provider)).collect();
        assert_eq!(got, vec![("/home/rich/GDrive", Provider::Rclone), ("/home/rich/pCloudDrive", Provider::PCloud), ("/mnt/my cloud", Provider::Nextcloud)]);
        assert!(m.iter().all(|r| r.kind == Kind::Mount));
    }

    #[test]
    fn matches_paths_to_the_deepest_root() {
        let r = |p: &str, provider| Root { path: PathBuf::from(p), provider, kind: Kind::OnDemand };
        let roots = vec![r("/home/a/Cloud", Provider::Dropbox), r("/home/a/Cloud/Inner", Provider::Box)];
        assert_eq!(root_of(&roots, Path::new("/home/a/Cloud/x/y")).map(|r| r.provider), Some(Provider::Dropbox));
        assert_eq!(root_of(&roots, Path::new("/home/a/Cloud/Inner/z")).map(|r| r.provider), Some(Provider::Box));
        assert_eq!(root_of(&roots, Path::new("/home/a/Cloud")).map(|r| r.provider), Some(Provider::Dropbox));
        assert!(root_of(&roots, Path::new("/home/a/CloudX")).is_none());
        assert_eq!(roots_below(&roots, Path::new("/home/a")).len(), 2);
        assert_eq!(roots_below(&roots, Path::new("/home/a/Cloud")).len(), 1);
        assert!(roots_below(&roots, Path::new("/")).len() == 2);
    }
}
