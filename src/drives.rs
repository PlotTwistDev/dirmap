//! Lists the drives / mounted volumes that can be scanned, with their capacity.

use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Drive {
    /// Root to scan, e.g. `D:\` or `/home`.
    pub root: PathBuf,
    /// Volume label (or, on Linux, the filesystem type), may be empty.
    pub label: String,
    pub total: u64,
    /// Space this user can still write.
    pub free: u64,
    pub network: bool,
    /// What the drive is, and the details behind it.
    pub info: Info,
}

impl Drive {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// Roughly how much a scan of it will find: its own data on a disk image that can't fill up
    /// (see `held_back`), else everything that isn't free.
    pub fn content(&self) -> u64 {
        match (self.held_back(), self.info.data) {
            (Some(_), Some(data)) => data,
            _ => self.used(),
        }
    }

    /// Space that is neither taken by this volume's own data nor free to write: a sparse disk
    /// image whose host drive is fuller than the image. `None` when that isn't the case.
    pub fn held_back(&self) -> Option<u64> {
        let data = self.info.data?;
        let gap = self.total.saturating_sub(data).saturating_sub(self.free);
        // Small gaps are only the filesystem's reserve.
        (self.info.kind == Kind::Image && gap > self.total / 50).then_some(gap)
    }
}

/// What sort of drive a volume is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Unknown,
    Internal,
    External,
    /// Removable media: an SD card or a card reader.
    Removable,
    Network,
    /// A disk image (.dmg, .sparsebundle, .vhdx, a loop device): a file on another drive
    /// mounted as if it were a disk.
    Image,
    /// A RAM disk or a filesystem in user space (FUSE).
    Virtual,
    /// One of the operating system's own volumes, such as macOS Recovery.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    System,
    Optical,
    /// A phone or camera, over MTP, PTP or Apple's own protocol.
    Phone,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Unknown => "Drive",
            Kind::Internal => "Internal drive",
            Kind::External => "External drive",
            Kind::Removable => "Removable media",
            Kind::Network => "Network share",
            Kind::Image => "Disk image",
            Kind::Virtual => "Virtual drive",
            Kind::System => "System volume",
            Kind::Optical => "Optical disc",
            Kind::Phone => "Phone or camera",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Network => "🖧",
            Kind::Image => "💿",
            Kind::Optical => "📀",
            Kind::Phone => "📱",
            Kind::External | Kind::Removable => "⏏",
            Kind::Virtual => "⛭",
            _ => "🖴",
        }
    }

    /// One plain sentence on what this kind of drive is.
    pub fn about(self) -> &'static str {
        match self {
            Kind::Unknown => "",
            Kind::Internal => "Built into this computer.",
            Kind::External => "Plugged in over a cable. Eject it before unplugging.",
            Kind::Removable => "Removable media, such as an SD card.",
            Kind::Network => "A folder shared by another computer or a NAS. It's only as fast as the network.",
            Kind::Image => "A file on another drive that the system opens as if it were a disk.",
            Kind::Virtual => "Not a physical disk: a RAM disk or a filesystem run by an app (FUSE).",
            Kind::System => "One of the operating system's own volumes. Leave its contents alone.",
            Kind::Optical => "A CD, DVD or Blu-ray disc.",
            Kind::Phone => "A phone or camera. These don't mount as drives, so most systems can't browse them like a folder.",
        }
    }
}

/// Details about a drive, as far as the system tells us. Empty strings and `None` are unknown.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub kind: Kind,
    /// Filesystem, e.g. "APFS", "NTFS", "ext4".
    pub fs: String,
    /// The device or server behind it: `/dev/disk5s1`, `//nas/media`, `\\server\share`.
    pub device: String,
    pub read_only: bool,
    /// How it's connected: "USB", "Thunderbolt", "SATA", "NVMe", "Disk Image".
    pub bus: String,
    /// The disk's make and model.
    pub model: String,
    pub ssd: Option<bool>,
    pub encrypted: Option<bool>,
    /// Space taken by this volume's own data. Can differ from `total - free` on shared APFS
    /// containers and sparse disk images.
    pub data: Option<u64>,
    /// A disk image's file, and its format ("sparse bundle disk image").
    pub image: Option<PathBuf>,
    pub image_type: String,
    /// Volumes that share this one's space (an APFS container).
    pub container: String,
    /// Hidden from the system's own file manager (macOS `nobrowse`).
    pub hidden: bool,
}

impl Info {
    fn local(kind: Kind, fs: &str, device: &str) -> Info {
        Info { kind, fs: fs.to_string(), device: device.to_string(), ..Default::default() }
    }
}

/// The drive holding `path`, from a drive list: the one with the longest root that contains it.
pub fn holding<'a>(path: &std::path::Path, drives: &'a [Drive]) -> Option<&'a Drive> {
    drives.iter().filter(|d| path.starts_with(&d.root)).max_by_key(|d| d.root.as_os_str().len())
}

/// Lines describing a drive, as (label, value), for a tooltip or a details window. `all` is the
/// whole drive list, to say which drive a disk image lives on.
pub fn describe(d: &Drive, all: &[Drive]) -> Vec<(&'static str, String)> {
    use crate::scan::human;
    let i = &d.info;
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut kind = i.kind.name().to_string();
    if let Some(ssd) = i.ssd
        && matches!(i.kind, Kind::Internal | Kind::External)
    {
        kind.push_str(if ssd { " (SSD)" } else { " (hard disk)" });
    }
    out.push(("Type", kind));
    if i.kind == Kind::Image {
        if let Some(img) = &i.image {
            out.push(("Image file", img.display().to_string()));
            if let Some(host) = holding(img, all).filter(|h| h.root != d.root) {
                out.push(("Stored on", format!("{} ({} free there)", host_name(host), human(host.free))));
            }
        }
        if !i.image_type.is_empty() {
            out.push(("Format", i.image_type.clone()));
        }
    }
    if !i.fs.is_empty() {
        out.push(("Filesystem", i.fs.clone()));
    }
    out.push(("Capacity", human(d.total)));
    match i.data {
        Some(data) if d.held_back().is_some() => out.push(("Used", format!("{} by files", human(data)))),
        _ => out.push(("Used", format!("{} ({:.0}%)", human(d.used()), d.used() as f64 * 100.0 / d.total.max(1) as f64))),
    }
    out.push(("Free", human(d.free)));
    if let Some(gap) = d.held_back() {
        let host = i.image.as_deref().and_then(|p| holding(p, all)).map(host_name).unwrap_or_else(|| "its host drive".into());
        out.push(("Held back", format!("{}: the image can't use this until {host} has more room", human(gap))));
    }
    if !i.bus.is_empty() && i.kind != Kind::Image {
        out.push(("Connection", i.bus.clone()));
    }
    if !i.model.is_empty() {
        out.push(("Model", i.model.clone()));
    }
    if !i.device.is_empty() {
        out.push((if i.kind == Kind::Network { "Server" } else { "Device" }, i.device.clone()));
    }
    if !i.container.is_empty() {
        out.push(("Shares space", i.container.clone()));
    }
    let mut flags = Vec::new();
    flags.push(if i.read_only { "Read-only" } else { "Writable" });
    match i.encrypted {
        Some(true) => flags.push("encrypted"),
        Some(false) => flags.push("not encrypted"),
        None => {}
    }
    if i.hidden {
        flags.push("hidden in Finder");
    }
    out.push(("Access", flags.join(", ")));
    out
}

/// "DataHerdA" for a drive in a sentence: its label, or its root.
fn host_name(d: &Drive) -> String {
    let named = !d.label.is_empty() && d.info.fs != d.label;
    if named { d.label.clone() } else { d.root.display().to_string() }
}

/// A phone or camera plugged in. Most systems don't mount these as drives.
#[derive(Clone, Debug, PartialEq)]
pub struct Phone {
    pub name: String,
    /// Where it can be browsed as files (Linux GVFS), if anywhere.
    pub path: Option<PathBuf>,
    /// "MTP", "PTP", "Apple (AFC)".
    pub protocol: String,
}

impl Phone {
    /// What the user can do with it, in a sentence.
    pub fn hint(&self) -> &'static str {
        if self.path.is_some() {
            "Mounted by the desktop (GVFS), so it can be browsed here. Copying can be slow."
        } else if cfg!(target_os = "macos") {
            "macOS doesn't mount phones as drives. Click to open Image Capture for photos and videos; for other files use Finder (iPhone) or an MTP app such as OpenMTP (Android)."
        } else if cfg!(windows) {
            "Windows lists phones under This PC but not as a drive letter. Click to open This PC in File Explorer."
        } else {
            "Not mounted as a drive."
        }
    }
}

/// Phones and cameras plugged in right now. Runs system tools, so call it off the UI thread.
pub fn phones() -> Vec<Phone> {
    let mut out = phones_imp();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name && a.path == b.path);
    out
}

/// Open a phone the system's own way: Image Capture on macOS, This PC on Windows.
pub fn open_phone(p: &Phone) {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("open").args(["-a", "Image Capture"]).spawn();
    } else if cfg!(windows) {
        let _ = std::process::Command::new("explorer.exe").arg("shell:::{20D04FE0-3AEA-1069-A2D8-08002B30309D}").spawn();
    }
    let _ = p;
}

// ---------- Windows ----------

#[cfg(windows)]
pub fn list() -> Vec<Drive> {
    use windows_sys::Win32::NetworkManagement::WNet::WNetGetConnectionW;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    // GetDriveTypeW return values (winbase.h).
    const DRIVE_UNKNOWN: u32 = 0;
    const DRIVE_NO_ROOT_DIR: u32 = 1;
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;
    const FILE_READ_ONLY_VOLUME: u32 = 0x0008_0000;
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
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
        let mut fs = [0u16; 64];
        let mut flags = 0u32;
        let ok = unsafe {
            GetVolumeInformationW(
                wide.as_ptr(),
                name.as_mut_ptr(),
                name.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        let text = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(0)]);
        let (label, fs) = if ok != 0 { (text(&name), text(&fs)) } else { (String::new(), String::new()) };
        let mut info = Info::local(Kind::Internal, &fs, "");
        info.read_only = ok != 0 && flags & FILE_READ_ONLY_VOLUME != 0;
        match kind {
            DRIVE_REMOTE => {
                info.kind = Kind::Network;
                let local: Vec<u16> = format!("{letter}:").encode_utf16().chain(Some(0)).collect();
                let mut remote = [0u16; 512];
                let mut len = remote.len() as u32;
                if unsafe { WNetGetConnectionW(local.as_ptr(), remote.as_mut_ptr(), &mut len) } == 0 {
                    info.device = text(&remote);
                }
            }
            DRIVE_RAMDISK => info.kind = Kind::Virtual,
            _ => {
                if kind == DRIVE_REMOVABLE {
                    info.kind = Kind::Removable;
                }
                win_storage(letter, &mut info);
            }
        }
        let _ = avail;
        info.data = Some(total.saturating_sub(free));
        out.push(Drive { root: root.into(), label, total, free, network: kind == DRIVE_REMOTE, info });
    }
    out
}

/// The bus, make and model of a local drive, from the storage driver. Never spins anything up:
/// the device is opened without read or write access.
#[cfg(windows)]
fn win_storage(letter: char, info: &mut Info) {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY, StorageDeviceProperty};
    let path: Vec<u16> = format!(r"\\.\{letter}:").encode_utf16().chain(Some(0)).collect();
    let h = unsafe { CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return;
    }
    let mut q: STORAGE_PROPERTY_QUERY = unsafe { std::mem::zeroed() };
    q.PropertyId = StorageDeviceProperty;
    q.QueryType = PropertyStandardQuery;
    let mut buf = vec![0u8; 1024];
    let mut got = 0u32;
    let ok = unsafe {
        DeviceIoControl(h, IOCTL_STORAGE_QUERY_PROPERTY, (&q as *const STORAGE_PROPERTY_QUERY).cast(), std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32, buf.as_mut_ptr().cast(), buf.len() as u32, &mut got, std::ptr::null_mut())
    };
    unsafe { CloseHandle(h) };
    if ok == 0 || (got as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return;
    }
    let d: STORAGE_DEVICE_DESCRIPTOR = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast()) };
    let at = |off: u32| -> String {
        let off = off as usize;
        if off == 0 || off >= got as usize {
            return String::new();
        }
        let end = buf[off..got as usize].iter().position(|&b| b == 0).map_or(got as usize, |e| off + e);
        String::from_utf8_lossy(&buf[off..end]).trim().to_string()
    };
    let model = [at(d.VendorIdOffset), at(d.ProductIdOffset)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    info.model = model;
    // STORAGE_BUS_TYPE values (ntddstor.h).
    info.bus = match d.BusType {
        1 => "SCSI",
        3 => "ATA",
        4 => "FireWire",
        6 => "Fibre Channel",
        7 => "USB",
        8 => "RAID",
        9 => "iSCSI",
        10 => "SAS",
        11 => "SATA",
        12 => "SD card",
        13 => "MMC",
        14 => "Virtual",
        15 => "Virtual (file-backed)",
        16 => "Storage Spaces",
        17 => "NVMe",
        18 => "SCM",
        19 => "UFS",
        _ => "",
    }
    .to_string();
    match d.BusType {
        7 | 4 => {
            if info.kind != Kind::Removable {
                info.kind = Kind::External;
            }
        }
        12 | 13 => info.kind = Kind::Removable,
        // A mounted .vhd/.vhdx/.iso.
        15 => info.kind = Kind::Image,
        14 => info.kind = Kind::Virtual,
        _ => {
            if d.RemovableMedia && info.kind == Kind::Internal {
                info.kind = Kind::Removable;
            }
        }
    }
}

/// Phones and cameras: Windows Portable Devices plugged in over USB. Removable drives also
/// show up as portable devices, so only USB-enumerated ones count.
#[cfg(windows)]
fn phones_imp() -> Vec<Phone> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVINFO_DATA, SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
        SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
    };
    // GUID_DEVINTERFACE_WPD {6AC27878-A6FA-4155-BA85-F98F491D4F33}
    let guid = windows_sys::core::GUID { data1: 0x6AC27878, data2: 0xA6FA, data3: 0x4155, data4: [0xBA, 0x85, 0xF9, 0x8F, 0x49, 0x1D, 0x4F, 0x33] };
    let mut out = Vec::new();
    unsafe {
        let set = SetupDiGetClassDevsW(&guid, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set == -1 {
            return out;
        }
        for i in 0.. {
            let mut dev: SP_DEVINFO_DATA = std::mem::zeroed();
            dev.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
            if SetupDiEnumDeviceInfo(set, i, &mut dev) == 0 {
                break;
            }
            let mut id = [0u16; 512];
            if SetupDiGetDeviceInstanceIdW(set, &dev, id.as_mut_ptr(), id.len() as u32, std::ptr::null_mut()) == 0 {
                continue;
            }
            let id = String::from_utf16_lossy(&id[..id.iter().position(|&c| c == 0).unwrap_or(0)]);
            if !id.to_ascii_uppercase().starts_with("USB\\") {
                continue;
            }
            let prop = |p| {
                let mut buf = [0u16; 256];
                let ok = SetupDiGetDeviceRegistryPropertyW(set, &dev, p, std::ptr::null_mut(), buf.as_mut_ptr().cast(), (buf.len() * 2) as u32, std::ptr::null_mut());
                (ok != 0).then(|| String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]))
            };
            let name = prop(SPDRP_FRIENDLYNAME).or_else(|| prop(SPDRP_DEVICEDESC)).filter(|s| !s.is_empty()).unwrap_or_else(|| "Portable device".into());
            out.push(Phone { name, path: None, protocol: "MTP/PTP".into() });
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

// ---------- Linux and other Unix ----------

#[cfg(all(unix, not(target_os = "macos")))]
pub fn list() -> Vec<Drive> {
    let Ok(s) = std::fs::read_to_string("/proc/self/mounts") else { return Vec::new() };
    let mut out = Vec::new();
    for m in parse_mounts(&s) {
        let Some((total, free, data)) = statvfs(&m.dir) else { continue };
        if total == 0 || out.iter().any(|d: &Drive| d.root == m.dir) {
            continue;
        }
        let mut info = linux_info(&m.dev, &m.fs);
        info.read_only = m.read_only;
        info.data = Some(data);
        let network = info.kind == Kind::Network;
        out.push(Drive { root: m.dir, label: m.fs, total, free, network, info });
    }
    out
}

/// A line of /proc/self/mounts we'd list.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
#[derive(Debug)]
struct Mount {
    dev: String,
    dir: PathBuf,
    fs: String,
    read_only: bool,
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn parse_mounts(s: &str) -> Vec<Mount> {
    const SKIP: &[&str] = &[
        "proc", "sysfs", "devtmpfs", "devpts", "tmpfs", "cgroup", "cgroup2", "securityfs", "pstore", "debugfs",
        "tracefs", "configfs", "fusectl", "mqueue", "hugetlbfs", "bpf", "autofs", "binfmt_misc", "overlay",
        "squashfs", "nsfs", "efivarfs", "ramfs", "rpc_pipefs", "fuse.portal", "fuse.gvfsd-fuse",
    ];
    let unescape = |s: &str| s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\");
    let mut out = Vec::new();
    for line in s.lines() {
        let mut f = line.split_whitespace();
        let (Some(dev), Some(dir), Some(fs)) = (f.next(), f.next(), f.next()) else { continue };
        if SKIP.contains(&fs) || dir.starts_with("/snap/") || dir.starts_with("/boot/efi") {
            continue;
        }
        let read_only = f.next().is_some_and(|o| o.split(',').any(|o| o == "ro"));
        out.push(Mount { dev: unescape(dev), dir: PathBuf::from(unescape(dir)), fs: fs.to_string(), read_only });
    }
    out
}

/// Filesystem types that live on another computer.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn network_fs(fs: &str) -> bool {
    matches!(fs, "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "fuse.sshfs" | "9p" | "afs" | "ceph" | "glusterfs" | "fuse.rclone" | "davfs" | "fuse.davfs2")
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn linux_info(dev: &str, fs: &str) -> Info {
    let mut info = Info::local(Kind::Unknown, fs, dev);
    if network_fs(fs) {
        info.kind = Kind::Network;
        return info;
    }
    if fs.starts_with("fuse") {
        info.kind = Kind::Virtual;
        info.fs = format!("FUSE ({})", fs.trim_start_matches("fuse.").trim_start_matches("fuse"));
        return info;
    }
    if fs == "iso9660" || fs == "udf" {
        info.kind = Kind::Optical;
    }
    #[cfg(not(test))]
    linux_block(dev, &mut info);
    info
}

/// What sysfs says about the block device behind a mount: USB or not, removable, the model,
/// SSD or spinning, a loop device's file, dm-crypt.
#[cfg(all(unix, not(target_os = "macos")))]
fn linux_block(dev: &str, info: &mut Info) {
    use std::path::Path;
    let read = |p: &Path| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let Ok(real) = std::fs::canonicalize(dev) else { return };
    let Some(name) = real.file_name().map(|n| n.to_string_lossy().to_string()) else { return };
    let Ok(mut sys) = std::fs::canonicalize(format!("/sys/class/block/{name}")) else { return };
    if sys.join("partition").exists() {
        sys.pop();
    }
    // Device mapper: encryption, then the disk underneath.
    for _ in 0..4 {
        if !sys.join("dm").exists() {
            break;
        }
        if read(&sys.join("dm/uuid")).is_some_and(|u| u.starts_with("CRYPT-")) {
            info.encrypted = Some(true);
        }
        let Some(lower) = std::fs::read_dir(sys.join("slaves")).ok().and_then(|mut r| r.next()).and_then(|e| e.ok()) else { return };
        let Ok(mut next) = std::fs::canonicalize(lower.path()) else { return };
        if next.join("partition").exists() {
            next.pop();
        }
        sys = next;
    }
    let disk = sys.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if disk.starts_with("loop") {
        info.kind = Kind::Image;
        info.image = read(&sys.join("loop/backing_file")).map(PathBuf::from);
        info.bus = "Loop device".into();
        return;
    }
    let path = sys.to_string_lossy().to_string();
    let usb = path.contains("/usb");
    info.ssd = read(&sys.join("queue/rotational")).map(|r| r == "0");
    let model = [read(&sys.join("device/vendor")), read(&sys.join("device/model")).or_else(|| read(&sys.join("device/name")))];
    info.model = model.into_iter().flatten().collect::<Vec<_>>().join(" ");
    info.bus = if usb {
        "USB"
    } else if disk.starts_with("nvme") {
        "NVMe"
    } else if disk.starts_with("mmcblk") {
        "SD/MMC"
    } else if path.contains("/ata") {
        "SATA"
    } else if path.contains("/virtio") {
        "Virtual (virtio)"
    } else {
        ""
    }
    .into();
    if info.kind == Kind::Optical {
        return;
    }
    let sd_card = disk.starts_with("mmcblk") && read(&sys.join("device/type")).is_some_and(|t| t == "SD");
    info.kind = if sd_card {
        Kind::Removable
    } else if usb || read(&sys.join("removable")).is_some_and(|r| r == "1") {
        Kind::External
    } else if path.contains("/virtio") || disk.starts_with("vd") || disk.starts_with("xvd") {
        Kind::Virtual
    } else {
        Kind::Internal
    };
}

/// Phones the desktop has mounted through GVFS (MTP, PTP cameras, iPhones over AFC).
#[cfg(all(unix, not(target_os = "macos")))]
fn phones_imp() -> Vec<Phone> {
    let uid = unsafe { libc::getuid() };
    let Ok(rd) = std::fs::read_dir(format!("/run/user/{uid}/gvfs")) else { return Vec::new() };
    rd.flatten().filter_map(|e| gvfs_phone(&e.file_name().to_string_lossy()).map(|mut p| {
        p.path = Some(e.path());
        p
    })).collect()
}

/// A GVFS mount name that is a phone or camera: `mtp:host=SAMSUNG_Android_R58M…`.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
pub fn gvfs_phone(name: &str) -> Option<Phone> {
    let (scheme, rest) = name.split_once(':')?;
    let protocol = match scheme {
        "mtp" => "MTP",
        "gphoto2" => "PTP",
        "afc" => "Apple (AFC)",
        _ => return None,
    };
    let host = rest.split(',').find_map(|kv| kv.strip_prefix("host=")).unwrap_or("");
    let host = host.replace("%20", " ");
    let name = if scheme == "afc" {
        "iPhone or iPad".to_string()
    } else {
        // "SAMSUNG_Android_R58M12345" → "SAMSUNG Android"; drop a trailing serial number.
        let mut words: Vec<&str> = host.split('_').filter(|w| !w.is_empty()).collect();
        if words.len() > 1 && words.last().is_some_and(|w| w.len() >= 6 && w.chars().any(|c| c.is_ascii_digit())) {
            words.pop();
        }
        if words.is_empty() { "Phone".to_string() } else { words.join(" ") }
    };
    Some(Phone { name, path: None, protocol: protocol.into() })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn statvfs(path: &std::path::Path) -> Option<(u64, u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = s.f_frsize as u64;
    let blocks = s.f_blocks as u64;
    Some((blocks * frsize, s.f_bavail as u64 * frsize, blocks.saturating_sub(s.f_bfree as u64) * frsize))
}

// ---------- macOS ----------

#[cfg(target_os = "macos")]
pub fn list() -> Vec<Drive> {
    // Every volume shows up in /Volumes, the startup disk as a link to /.
    let mut mounts: Vec<(PathBuf, String)> = vec![("/".into(), String::new())];
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            match std::fs::canonicalize(e.path()) {
                Ok(p) if p == std::path::Path::new("/") => mounts[0].1 = name,
                _ => mounts.push((e.path(), name)),
            }
        }
    }
    let mut out: Vec<Drive> = Vec::new();
    for (root, label) in mounts {
        let Some(m) = mac::statfs(&root) else { continue };
        if m.total == 0 || out.iter().any(|d| d.root == root) {
            continue;
        }
        let network = m.info.kind == Kind::Network;
        out.push(Drive { root, label, total: m.total, free: m.free, network, info: m.info });
    }
    mac::details(&mut out);
    out
}

#[cfg(target_os = "macos")]
fn phones_imp() -> Vec<Phone> {
    mac::usb_phones()
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::path::Path;

    pub struct Stat {
        pub total: u64,
        pub free: u64,
        pub info: Info,
    }

    fn cstr(b: &[libc::c_char]) -> String {
        let b: Vec<u8> = b.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&b).to_string()
    }

    /// What the kernel knows, without asking any daemon: sizes, filesystem, device, flags.
    pub fn statfs(path: &Path) -> Option<Stat> {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
            return None;
        }
        let bs = s.f_bsize as u64;
        let fs = cstr(&s.f_fstypename);
        let from = cstr(&s.f_mntfromname);
        let flags = s.f_flags as i32;
        let mut info = Info::local(Kind::Unknown, &fs, &from);
        info.data = Some(s.f_blocks.saturating_sub(s.f_bfree) * bs);
        // The startup disk's system volume is sealed read-only, but what's written there lands
        // on the Data volume beside it, so it isn't read-only to the user.
        info.read_only = flags & libc::MNT_RDONLY != 0 && path != Path::new("/");
        info.hidden = flags & libc::MNT_DONTBROWSE != 0;
        if flags & libc::MNT_LOCAL == 0 || matches!(fs.as_str(), "smbfs" | "nfs" | "afpfs" | "webdav" | "ftp") {
            info.kind = Kind::Network;
            // "//rich@nas._smb._tcp.local/media" → "//nas._smb._tcp.local/media"
            if let Some(rest) = from.strip_prefix("//")
                && let Some((_, host)) = rest.split_once('@')
            {
                info.device = format!("//{host}");
            }
        } else if matches!(fs.as_str(), "macfuse" | "osxfuse" | "fuse" | "fuse-t") || fs.starts_with("fuse") {
            info.kind = Kind::Virtual;
        } else if matches!(fs.as_str(), "cd9660" | "udf" | "cddafs") {
            info.kind = Kind::Optical;
        }
        Some(Stat { total: s.f_blocks * bs, free: s.f_bavail * bs, info })
    }

    fn run(cmd: &str, args: &[&str]) -> Option<Vec<u8>> {
        let out = std::process::Command::new(cmd).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
        out.status.success().then_some(out.stdout)
    }

    fn plist_dict(bytes: &[u8]) -> Option<plist::Dictionary> {
        plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()?.into_dictionary()
    }

    /// Ask diskutil about each local volume, and hdiutil about disk images.
    pub fn details(drives: &mut [Drive]) {
        let mut any_image = false;
        // APFS containers, to say which volumes share space: container → volume names.
        let mut containers: Vec<(String, String, PathBuf)> = Vec::new();
        let mut stores: Vec<(PathBuf, Vec<String>)> = Vec::new();
        // diskutil takes a moment per volume, so ask about them all at once.
        let answers: Vec<Option<plist::Dictionary>> = std::thread::scope(|sc| {
            let jobs: Vec<_> = drives
                .iter()
                .map(|d| {
                    let dev = d.info.device.clone();
                    sc.spawn(move || dev.starts_with("/dev/").then(|| run("diskutil", &["info", "-plist", &dev]).and_then(|b| plist_dict(&b))).flatten())
                })
                .collect();
            jobs.into_iter().map(|j| j.join().ok().flatten()).collect()
        });
        for (d, info) in drives.iter_mut().zip(answers) {
            let Some(info) = info else { continue };
            let s = |k: &str| info.get(k).and_then(|v| v.as_string()).unwrap_or("").to_string();
            let b = |k: &str| info.get(k).and_then(|v| v.as_boolean());
            let i = &mut d.info;
            let fs = s("FilesystemUserVisibleName");
            if !fs.is_empty() {
                i.fs = fs;
            }
            i.bus = s("BusProtocol");
            let media = s("MediaName");
            if !media.is_empty() && !media.starts_with("Apple") {
                i.model = media;
            }
            i.ssd = b("SolidState");
            i.encrypted = b("FileVault").filter(|&f| f).or(b("Encryption"));
            if b("WritableVolume") == Some(false) && d.root != Path::new("/") {
                i.read_only = true;
            }
            let container = s("APFSContainerReference");
            if !container.is_empty() {
                let name = if d.label.is_empty() { s("VolumeName") } else { d.label.clone() };
                containers.push((container, name, d.root.clone()));
            }
            let phys: Vec<String> = info
                .get("APFSPhysicalStores")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_dictionary()?.get("APFSPhysicalStore")?.as_string().map(str::to_string)).collect())
                .unwrap_or_default();
            let mut ids = phys;
            ids.push(s("ParentWholeDisk"));
            ids.push(s("DeviceIdentifier"));
            stores.push((d.root.clone(), ids));
            let recovery = s("RecoveryDeviceIdentifier");
            if i.bus == "Disk Image" {
                i.kind = Kind::Image;
                any_image = true;
            } else if !recovery.is_empty() && recovery == s("DeviceIdentifier") {
                i.kind = Kind::System;
            } else if b("Internal") == Some(true) {
                i.kind = if i.hidden && d.root != Path::new("/") { Kind::System } else { Kind::Internal };
            } else if i.kind == Kind::Unknown {
                i.kind = if b("RemovableMedia") == Some(true) && matches!(i.bus.as_str(), "Secure Digital" | "SD") { Kind::Removable } else { Kind::External };
            }
        }
        for d in drives.iter_mut() {
            let Some((c, _, _)) = containers.iter().find(|(_, _, r)| *r == d.root) else { continue };
            let others: Vec<&str> = containers.iter().filter(|(c2, _, r)| c2 == c && *r != d.root).map(|(_, n, _)| n.as_str()).collect();
            if !others.is_empty() {
                d.info.container = format!("APFS container {c}, with {}", others.join(", "));
            }
        }
        if !any_image {
            return;
        }
        let Some(info) = run("hdiutil", &["info", "-plist"]).and_then(|b| plist_dict(&b)) else { return };
        let Some(images) = info.get("images").and_then(|v| v.as_array()) else { return };
        for img in images.iter().filter_map(|v| v.as_dictionary()) {
            let path = img.get("image-path").and_then(|v| v.as_string()).unwrap_or("");
            let kind = img.get("image-type").and_then(|v| v.as_string()).unwrap_or("");
            let entities: Vec<(String, String)> = img
                .get("system-entities")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.as_dictionary())
                        .map(|e| {
                            let g = |k: &str| e.get(k).and_then(|v| v.as_string()).unwrap_or("").to_string();
                            (g("dev-entry").trim_start_matches("/dev/").to_string(), g("mount-point"))
                        })
                        .collect()
                })
                .unwrap_or_default();
            for d in drives.iter_mut().filter(|d| d.info.kind == Kind::Image) {
                let ids = stores.iter().find(|(r, _)| *r == d.root).map(|(_, v)| v.as_slice()).unwrap_or(&[]);
                let hit = entities.iter().any(|(dev, mp)| Path::new(mp) == d.root || (!dev.is_empty() && ids.iter().any(|i| i == dev)));
                if hit && !path.is_empty() {
                    d.info.image = Some(PathBuf::from(path));
                    d.info.image_type = kind.to_string();
                }
            }
        }
    }

    /// iPhones, iPads and phones or cameras speaking MTP/PTP on USB, from the I/O Registry.
    pub fn usb_phones() -> Vec<Phone> {
        let Some(out) = run("ioreg", &["-p", "IOUSB", "-l", "-w0"]) else { return Vec::new() };
        let mut phones = parse_ioreg(&String::from_utf8_lossy(&out));
        // MTP/PTP is announced by an interface, which IOUSB doesn't list: ask for those too.
        if let Some(out) = run("ioreg", &["-r", "-c", "IOUSBHostInterface", "-l", "-w0"]) {
            for (vendor, product, name) in ptp_interfaces(&String::from_utf8_lossy(&out)) {
                if !phones.iter().any(|p| p.name == name) {
                    let _ = (vendor, product);
                    phones.push(Phone { name, path: None, protocol: "MTP/PTP".into() });
                }
            }
        }
        phones
    }
}

/// Apple mobile devices in `ioreg -p IOUSB -l` output.
#[cfg(any(test, target_os = "macos"))]
fn parse_ioreg(s: &str) -> Vec<Phone> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut vendor = 0u32;
    let flush = |name: &str, vendor: u32, out: &mut Vec<Phone>| {
        if vendor == 0x05AC && ["iPhone", "iPad", "iPod"].iter().any(|k| name.contains(k)) {
            out.push(Phone { name: name.to_string(), path: None, protocol: "Apple".into() });
        }
    };
    for line in s.lines() {
        let t = line.trim_start_matches(|c: char| c == ' ' || c == '|');
        if t.starts_with("+-o ") {
            flush(&name, vendor, &mut out);
            name.clear();
            vendor = 0;
        } else if let Some(v) = ioreg_value(t, "USB Product Name").or_else(|| ioreg_value(t, "kUSBProductString")) {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = ioreg_value(t, "idVendor") {
            vendor = v.parse().unwrap_or(0);
        }
    }
    flush(&name, vendor, &mut out);
    out
}

/// USB interfaces of class 6 (still image: PTP, and most MTP phones) or named MTP, as
/// (vendor, product id, product name).
#[cfg(any(test, target_os = "macos"))]
fn ptp_interfaces(s: &str) -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    let (mut class, mut vendor, mut product, mut name, mut iname) = (0u32, 0u32, 0u32, String::new(), String::new());
    let flush = |class: u32, vendor: u32, product: u32, name: &str, iname: &str, out: &mut Vec<(u32, u32, String)>| {
        if vendor != 0x05AC && (class == 6 || iname.contains("MTP")) && !out.iter().any(|o: &(u32, u32, String)| o.0 == vendor && o.1 == product) {
            out.push((vendor, product, if name.is_empty() { "Phone or camera".to_string() } else { name.to_string() }));
        }
    };
    for line in s.lines() {
        let t = line.trim_start_matches(|c: char| c == ' ' || c == '|');
        if t.starts_with("+-o ") {
            flush(class, vendor, product, &name, &iname, &mut out);
            (class, vendor, product) = (0, 0, 0);
            name.clear();
            iname.clear();
        } else if let Some(v) = ioreg_value(t, "bInterfaceClass") {
            class = v.parse().unwrap_or(0);
        } else if let Some(v) = ioreg_value(t, "idVendor") {
            vendor = v.parse().unwrap_or(0);
        } else if let Some(v) = ioreg_value(t, "idProduct") {
            product = v.parse().unwrap_or(0);
        } else if let Some(v) = ioreg_value(t, "kUSBProductString").or_else(|| ioreg_value(t, "USB Product Name")) {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = ioreg_value(t, "kUSBString").or_else(|| ioreg_value(t, "USB Interface Name")) {
            iname = v.trim_matches('"').to_string();
        }
    }
    flush(class, vendor, product, &name, &iname, &mut out);
    out
}

/// `"key" = value` from an ioreg property line.
#[cfg(any(test, target_os = "macos"))]
fn ioreg_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix('"')?.strip_prefix(key)?.strip_prefix("\" = ")?;
    Some(rest.trim())
}

#[cfg(not(any(windows, unix)))]
pub fn list() -> Vec<Drive> {
    Vec::new()
}

#[cfg(not(any(windows, unix)))]
fn phones_imp() -> Vec<Phone> {
    Vec::new()
}

/// The WSL distributions installed for this user, as `\\wsl.localhost\<name>` roots, the way
/// Explorer lists them under Linux. Read from the registry, so nothing starts WSL.
#[cfg(windows)]
pub fn wsl() -> Vec<PathBuf> {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let mut out = Vec::new();
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, wide(r"Software\Microsoft\Windows\CurrentVersion\Lxss").as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return out;
        }
        let value = wide("DistributionName");
        for i in 0.. {
            let mut sub = [0u16; 256];
            let mut len = sub.len() as u32;
            let r = RegEnumKeyExW(key, i, sub.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            if r != 0 {
                break;
            }
            let mut name = [0u16; 256];
            let mut bytes = (name.len() * 2) as u32;
            if RegGetValueW(key, sub.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), name.as_mut_ptr().cast(), &mut bytes) == 0 {
                let n = String::from_utf16_lossy(&name[..(bytes as usize / 2).saturating_sub(1)]);
                // docker-desktop's own distributions aren't for browsing.
                if !n.is_empty() && !n.starts_with("docker-desktop") {
                    out.push(PathBuf::from(format!(r"\\wsl.localhost\{n}")));
                }
            }
        }
        RegCloseKey(key);
    }
    out.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    out
}

#[cfg(not(windows))]
pub fn wsl() -> Vec<PathBuf> {
    Vec::new()
}

/// Free space available to this user on the volume holding `path` (which must exist).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn free_at(path: &std::path::Path) -> Option<u64> {
    statvfs(path).map(|(_, free, _)| free)
}

#[cfg(target_os = "macos")]
pub fn free_at(path: &std::path::Path) -> Option<u64> {
    mac::statfs(path).map(|s| s.free)
}

#[cfg(windows)]
pub fn free_at(path: &std::path::Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    (unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) } != 0).then_some(avail)
}

#[cfg(not(any(windows, unix)))]
pub fn free_at(_path: &std::path::Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    fn drive(root: &str, label: &str, total: u64, free: u64, info: Info) -> Drive {
        Drive { root: root.into(), label: label.into(), total, free, network: info.kind == Kind::Network, info }
    }

    /// MixDev on Rich's Mac: a 250 GB sparse bundle holding 111 GB, on an exFAT drive with
    /// 1.7 GB left, so only 1.7 GB can be written to it.
    fn mixdev() -> Vec<Drive> {
        let host = drive("/Volumes/DataHerdA", "DataHerdA", 931 * GB, 1_700_000_000, Info { kind: Kind::External, fs: "ExFAT".into(), bus: "USB".into(), ..Default::default() });
        let img = drive(
            "/Volumes/MixDev",
            "MixDev",
            250 * GB,
            1_700_000_000,
            Info {
                kind: Kind::Image,
                fs: "APFS".into(),
                device: "/dev/disk5s1".into(),
                data: Some(111 * GB),
                image: Some("/Volumes/DataHerdA/MixDirectiveDev.sparsebundle".into()),
                image_type: "sparse bundle disk image".into(),
                hidden: true,
                ..Default::default()
            },
        );
        vec![drive("/", "Macintosh HD", 228 * GB, 22 * GB, Info { kind: Kind::Internal, fs: "APFS".into(), ..Default::default() }), img, host]
    }

    #[test]
    fn sparse_image_explained() {
        let all = mixdev();
        let d = &all[1];
        assert_eq!(d.held_back(), Some(250 * GB - 111 * GB - 1_700_000_000));
        assert_eq!(d.content(), 111 * GB);
        let lines = describe(d, &all);
        let get = |k: &str| lines.iter().find(|l| l.0 == k).map(|l| l.1.clone()).unwrap_or_default();
        assert_eq!(get("Type"), "Disk image");
        assert!(get("Stored on").starts_with("DataHerdA ("), "{}", get("Stored on"));
        assert!(get("Held back").contains("until DataHerdA has more room"));
        assert_eq!(get("Used"), format!("{} by files", crate::scan::human(111 * GB)));
        assert!(get("Access").contains("hidden in Finder"));
        // An ordinary drive has no held-back part, and its used is everything not free.
        assert_eq!(all[0].held_back(), None);
        assert_eq!(all[0].content(), 206 * GB);
    }

    #[test]
    fn holding_picks_longest_root() {
        let all = mixdev();
        let h = holding(std::path::Path::new("/Volumes/DataHerdA/x.sparsebundle"), &all).unwrap();
        assert_eq!(h.label, "DataHerdA");
        assert_eq!(holding(std::path::Path::new("/Users/rich"), &all).unwrap().root, PathBuf::from("/"));
    }

    #[test]
    fn linux_mounts() {
        let m = parse_mounts(
            "/dev/sda1 / ext4 rw,relatime 0 0\nproc /proc proc rw 0 0\n/dev/sdb1 /media/rich/My\\040Disk vfat ro,nosuid 0 0\nnas:/vol /mnt/nas nfs4 rw 0 0\nrclone: /mnt/gd fuse.rclone rw 0 0\nsshfs#box: /mnt/box fuse.sshfs rw 0 0\n/dev/sr0 /media/cd iso9660 ro 0 0\n",
        );
        let dirs: Vec<String> = m.iter().map(|m| m.dir.display().to_string()).collect();
        assert_eq!(dirs, ["/", "/media/rich/My Disk", "/mnt/nas", "/mnt/gd", "/mnt/box", "/media/cd"]);
        assert!(!m[0].read_only && m[1].read_only);
        assert_eq!(linux_info(&m[2].dev, &m[2].fs).kind, Kind::Network);
        assert_eq!(linux_info(&m[2].dev, &m[2].fs).device, "nas:/vol");
        assert_eq!(linux_info(&m[3].dev, &m[3].fs).kind, Kind::Network);
        assert_eq!(linux_info(&m[4].dev, &m[4].fs).kind, Kind::Network);
        assert_eq!(linux_info("/dev/fuse", "fuse.mergerfs").kind, Kind::Virtual);
        assert_eq!(linux_info(&m[5].dev, &m[5].fs).kind, Kind::Optical);
    }

    #[test]
    fn gvfs_phones() {
        let p = gvfs_phone("mtp:host=SAMSUNG_Android_R58M12345XY").unwrap();
        assert_eq!((p.name.as_str(), p.protocol.as_str()), ("SAMSUNG Android", "MTP"));
        assert_eq!(gvfs_phone("gphoto2:host=Canon_Inc._EOS_R6").unwrap().name, "Canon Inc. EOS R6");
        assert_eq!(gvfs_phone("afc:host=00008110-001A2B3C4D5E6F,port=3").unwrap().name, "iPhone or iPad");
        assert!(gvfs_phone("smb-share:server=nas,share=media").is_none());
        assert!(gvfs_phone("sftp:host=box").is_none());
    }

    #[test]
    fn ioreg_phones() {
        let usb = r#"+-o Root  <class IORegistryEntry, id 0x100000100, retain 31>
  +-o iPhone@00100000  <class IOUSBHostDevice, id 0x100001234, registered, matched, active, busy 0 (2 ms), retain 30>
  | {
  |   "USB Product Name" = "iPhone"
  |   "idVendor" = 1452
  | }
  +-o T7@00200000  <class IOUSBHostDevice, id 0x100001235>
  | {
  |   "USB Product Name" = "PSSD T7"
  |   "idVendor" = 1256
  | }
"#;
        let p = parse_ioreg(usb);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "iPhone");
        let ifs = r#"+-o IOUSBHostInterface@0  <class IOUSBHostInterface>
    {
      "idProduct" = 26720
      "bInterfaceClass" = 6
      "kUSBProductString" = "Pixel 8"
      "idVendor" = 6353
    }
+-o IOUSBHostInterface@0  <class IOUSBHostInterface>
    {
      "idProduct" = 1
      "bInterfaceClass" = 8
      "kUSBProductString" = "PSSD T7"
      "idVendor" = 1256
    }
+-o IOUSBHostInterface@1  <class IOUSBHostInterface>
    {
      "idProduct" = 26720
      "bInterfaceClass" = 6
      "kUSBProductString" = "Pixel 8"
      "idVendor" = 6353
    }
"#;
        let found = ptp_interfaces(ifs);
        assert_eq!(found, vec![(6353, 26720, "Pixel 8".to_string())]);
    }
}
