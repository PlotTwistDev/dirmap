//! Ejecting external drives, removable media and disk images, through the operating system:
//! `diskutil` and `hdiutil` on macOS, GVFS or udisks on Linux, and the Windows storage and
//! Plug and Play calls that "Safely Remove Hardware" uses.
//!
//! Network shares aren't here: they are disconnected through `netdrives`. Every call can wait on
//! the disk (flushing what's still being written), so call them off the UI thread.

use crate::drives::{Drive, Kind};
use std::path::{Path, PathBuf};

/// A program with files open on a drive, which keeps it from being ejected.
#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub pid: u32,
    pub name: String,
}

/// Why a drive wasn't ejected.
#[derive(Clone, Debug)]
pub enum Failure {
    /// Something has files open on it. `holders` lists the programs, when the system says.
    Busy { holders: Vec<Holder>, detail: String },
    Other(String),
}

/// Whether Filescape offers Eject for a drive at all: external drives, removable media, disk
/// images and discs. Internal and system volumes never; network shares have Disconnect.
pub fn ejectable(d: &Drive) -> bool {
    !d.network && matches!(d.info.kind, Kind::External | Kind::Removable | Kind::Image | Kind::Optical)
}

/// Why an ejectable drive can't be ejected from here: it holds the system, the user's home
/// folder, or Filescape itself.
pub fn protected(d: &Drive) -> Option<&'static str> {
    let home = home_dir();
    let exe = std::env::current_exe().ok();
    protected_with(&d.root, home.as_deref(), exe.as_deref())
}

fn protected_with(root: &Path, home: Option<&Path>, exe: Option<&Path>) -> Option<&'static str> {
    if is_system_root(root) {
        return Some("It holds the operating system.");
    }
    if exe.is_some_and(|e| e.starts_with(root)) {
        return Some("Filescape is running from it. Quit Filescape first, then eject it from the system.");
    }
    if home.is_some_and(|h| h.starts_with(root)) {
        return Some("Your home folder is on it.");
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// The startup disk and the places the system itself lives.
fn is_system_root(root: &Path) -> bool {
    #[cfg(windows)]
    {
        let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let r = root.to_string_lossy().trim_end_matches('\\').to_ascii_uppercase();
        r == sys.trim_end_matches('\\').to_ascii_uppercase()
    }
    #[cfg(not(windows))]
    {
        const SYSTEM: &[&str] = &["/", "/boot", "/boot/efi", "/usr", "/var", "/etc", "/home", "/opt", "/srv", "/nix", "/System/Volumes/Data", "/System/Volumes/Preboot", "/System/Volumes/VM", "/System/Volumes/Update"];
        SYSTEM.iter().any(|s| root == Path::new(s))
    }
}

/// What ejecting `d` also takes with it: other volumes on the same physical disk (they go
/// together), and disk images stored on it (ejected first, or the drive stays busy). Images
/// come first in the order they have to go: an image inside an image before its host.
pub struct Plan {
    pub drive: Drive,
    /// Mounted images whose file is on the drive (or on one of its sibling volumes).
    pub images: Vec<Drive>,
    /// Other volumes of the same disk.
    pub siblings: Vec<Drive>,
}

pub fn plan(d: &Drive, all: &[Drive]) -> Plan {
    let siblings: Vec<Drive> = if d.info.disk.is_empty() || d.info.kind == Kind::Image {
        Vec::new()
    } else {
        all.iter().filter(|o| o.root != d.root && !o.network && o.info.disk == d.info.disk).cloned().collect()
    };
    let mut going: Vec<PathBuf> = std::iter::once(d.root.clone()).chain(siblings.iter().map(|s| s.root.clone())).collect();
    let mut images: Vec<Drive> = Vec::new();
    // Images on images: keep looking until nothing new turns up.
    loop {
        let found: Vec<Drive> = all
            .iter()
            .filter(|o| o.info.kind == Kind::Image && !going.contains(&o.root))
            .filter(|o| o.info.image.as_deref().and_then(|f| crate::drives::holding(f, all)).is_some_and(|h| going.contains(&h.root)))
            .cloned()
            .collect();
        if found.is_empty() {
            break;
        }
        going.extend(found.iter().map(|f| f.root.clone()));
        // Found later means stored deeper, so it has to go sooner.
        images.splice(0..0, found);
    }
    Plan { drive: d.clone(), images, siblings }
}

/// Eject a drive: its disk images first, then the drive (with the other volumes on its disk).
/// `force` unmounts even with files open, as the system's own Force Eject does: apps holding
/// them may lose unsaved work.
pub fn eject(p: &Plan, force: bool) -> Result<(), Failure> {
    for img in &p.images {
        imp::eject(img, force).map_err(|f| prefix(f, img))?;
    }
    imp::eject(&p.drive, force)
}

/// Says which drive a failure was about, when it was one of the images in front.
fn prefix(f: Failure, d: &Drive) -> Failure {
    let name = name_of(d);
    match f {
        Failure::Busy { holders, detail } => Failure::Busy { holders, detail: format!("{name} (a disk image on it) is in use. {detail}").trim().to_string() },
        Failure::Other(s) => Failure::Other(format!("{name} (a disk image on it): {s}")),
    }
}

pub fn name_of(d: &Drive) -> String {
    if !d.label.is_empty() && d.info.fs != d.label {
        return d.label.clone();
    }
    d.root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| d.root.display().to_string())
}

/// The programs with files open on the drive at `root`, as far as this user can see. Filescape
/// itself is listed too, as "Filescape", when it is one of them.
pub fn holders(root: &Path) -> Vec<Holder> {
    let mut v = imp::holders(root);
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.pid.cmp(&b.pid)));
    v.dedup_by(|a, b| a.pid == b.pid);
    let me = std::process::id();
    for h in &mut v {
        if h.pid == me {
            h.name = "Filescape".into();
        }
    }
    v
}

/// A tool's error text: is it the "something is using it" kind?
fn says_busy(s: &str) -> bool {
    let s = s.to_lowercase();
    ["busy", "in use", "dissented", "could not be unmounted", "couldn't be unmounted", "failed to unmount", "open files"].iter().any(|k| s.contains(k))
}

/// `diskutil`'s "dissented by PID 412 (/Applications/Preview.app/Contents/MacOS/Preview)".
#[cfg(any(test, target_os = "macos"))]
fn dissenters(s: &str) -> Vec<Holder> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(at) = rest.find("dissented by PID ") {
        rest = &rest[at + "dissented by PID ".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(pid) = digits.parse::<u32>() else { continue };
        let after = &rest[digits.len()..];
        let name = after
            .trim_start()
            .strip_prefix('(')
            .and_then(|r| r.split_once(')'))
            .map(|(path, _)| program_name(path))
            .unwrap_or_else(|| format!("process {pid}"));
        out.push(Holder { pid, name });
    }
    out
}

/// `/Applications/Preview.app/Contents/MacOS/Preview` → `Preview`; a plain path's file name.
#[cfg(any(test, target_os = "macos"))]
fn program_name(path: &str) -> String {
    if let Some(app) = path.split('/').find(|c| c.ends_with(".app")) {
        return app.trim_end_matches(".app").to_string();
    }
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// `lsof -F pc` output: `p412` then `cPreview` for each process.
#[cfg(any(test, target_os = "macos"))]
fn parse_lsof(s: &str) -> Vec<Holder> {
    let mut out: Vec<Holder> = Vec::new();
    for line in s.lines() {
        if let Some(p) = line.strip_prefix('p') {
            if let Ok(pid) = p.parse() {
                out.push(Holder { pid, name: format!("process {pid}") });
            }
        } else if let (Some(c), Some(last)) = (line.strip_prefix('c'), out.last_mut()) {
            last.name = c.to_string();
        }
    }
    out
}

/// Run a command, giving up after `secs` (a tool waiting on a dead disk mustn't hang the
/// worker for ever). (success, stdout, stderr), or the error starting it.
#[cfg(unix)]
fn run(cmd: &str, args: &[&str], secs: u64) -> std::io::Result<(bool, String, String)> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let (mut o, mut e) = (String::new(), String::new());
        if let Some(s) = out.as_mut() {
            let _ = s.read_to_string(&mut o);
        }
        if let Some(s) = err.as_mut() {
            let _ = s.read_to_string(&mut e);
        }
        (o, e)
    });
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed().as_secs() >= secs {
            let _ = child.kill();
            let _ = child.wait();
            return Ok((false, String::new(), format!("{cmd} didn't answer within {secs} seconds.")));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let (o, e) = reader.join().unwrap_or_default();
    Ok((status.success(), o, e))
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub fn eject(d: &Drive, force: bool) -> Result<(), Failure> {
        let root = d.root.to_string_lossy().to_string();
        let attempt = if d.info.kind == Kind::Image {
            // Detaching takes the image's whole disk, and every volume on it, away.
            let mut args = vec!["detach", root.as_str()];
            if force {
                args.push("-force");
            }
            run("hdiutil", &args, 120)
        } else if !d.info.disk.is_empty() {
            // Unmount every volume on the disk, then eject the disk itself, as the Finder does.
            let disk = format!("/dev/{}", d.info.disk);
            let mut args = vec!["unmountDisk"];
            if force {
                args.push("force");
            }
            args.push(disk.as_str());
            match run("diskutil", &args, 120) {
                Ok((true, ..)) => run("diskutil", &["eject", disk.as_str()], 60),
                other => other,
            }
        } else {
            let mut args = vec!["eject"];
            if force {
                args.push("force");
            }
            args.push(root.as_str());
            run("diskutil", &args, 120)
        };
        match attempt {
            Err(e) => Err(Failure::Other(format!("Couldn't run the system's disk tool: {e}"))),
            Ok((true, ..)) => Ok(()),
            Ok((false, out, err)) => {
                let text = format!("{}\n{}", err.trim(), out.trim());
                if says_busy(&text) {
                    let mut holders = dissenters(&text);
                    if holders.is_empty() {
                        holders = super::holders(&d.root);
                    }
                    Err(Failure::Busy { holders, detail: String::new() })
                } else {
                    Err(Failure::Other(clean(&text)))
                }
            }
        }
    }

    /// The useful line of a tool's error: the last non-empty one, usually.
    fn clean(s: &str) -> String {
        let line = s.lines().map(str::trim).filter(|l| !l.is_empty()).next_back().unwrap_or("The disk couldn't be ejected.");
        line.to_string()
    }

    pub fn holders(root: &Path) -> Vec<Holder> {
        let root = root.to_string_lossy().to_string();
        // +f: everything open on the filesystem mounted there. -w: no warnings; -b: don't
        // block on stat() of other mounts.
        match run("lsof", &["-w", "-b", "-F", "pc", "+f", "--", root.as_str()], 10) {
            Ok((_, out, _)) => parse_lsof(&out),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;

    pub fn eject(d: &Drive, force: bool) -> Result<(), Failure> {
        let root = d.root.to_string_lossy().to_string();
        // GVFS unmounts, locks an encrypted volume, and powers the drive off, all as the user.
        let gio = if force { None } else { run("gio", &["mount", "-e", root.as_str()], 120).ok() };
        let err = match gio {
            Some((true, ..)) => return Ok(()),
            Some((false, out, err)) => format!("{} {}", err.trim(), out.trim()),
            None => String::new(),
        };
        if says_busy(&err) {
            return Err(busy(d));
        }
        // No GVFS, or it doesn't know this mount: udisks directly.
        let dev = d.info.device.clone();
        if !dev.starts_with("/dev/") {
            return Err(Failure::Other(if err.trim().is_empty() { "The system doesn't say which device it is.".into() } else { err.trim().to_string() }));
        }
        let mut args = vec!["unmount", "--no-user-interaction", "-b", dev.as_str()];
        if force {
            args.push("--force");
        }
        match run("udisksctl", &args, 120) {
            Err(_) => return umount(d, force),
            Ok((false, out, e)) => {
                let text = format!("{} {}", e.trim(), out.trim());
                if says_busy(&text) {
                    return Err(busy(d));
                }
                // Not something udisks manages (mounted by fstab): plain umount.
                if !text.contains("not mounted") {
                    return umount(d, force).map_err(|f| match f {
                        Failure::Other(s) => Failure::Other(format!("{s} ({})", text.trim())),
                        b => b,
                    });
                }
            }
            Ok((true, ..)) => {}
        }
        let whole = if d.info.disk.is_empty() { dev.clone() } else { format!("/dev/{}", d.info.disk) };
        let last = if d.info.kind == Kind::Image {
            run("udisksctl", &["loop-delete", "--no-user-interaction", "-b", whole.as_str()], 60)
        } else {
            run("udisksctl", &["power-off", "--no-user-interaction", "-b", whole.as_str()], 60)
        };
        // Unmounted is what matters: a drive that can't be powered off can still be unplugged.
        let _ = last;
        Ok(())
    }

    fn umount(d: &Drive, force: bool) -> Result<(), Failure> {
        let root = d.root.to_string_lossy().to_string();
        let args: Vec<&str> = if force { vec!["-l", root.as_str()] } else { vec![root.as_str()] };
        match run("umount", &args, 120) {
            Err(e) => Err(Failure::Other(format!("Couldn't run umount: {e}"))),
            Ok((true, ..)) => Ok(()),
            Ok((false, out, err)) => {
                let text = format!("{} {}", err.trim(), out.trim());
                if says_busy(&text) {
                    Err(busy(d))
                } else if text.contains("must be superuser") || text.contains("Operation not permitted") || text.contains("permission") {
                    Err(Failure::Other("Only an administrator can unmount it (it was mounted by the system, not the desktop). Use `sudo umount` in a terminal.".into()))
                } else {
                    Err(Failure::Other(text.trim().to_string()))
                }
            }
        }
    }

    fn busy(d: &Drive) -> Failure {
        Failure::Busy { holders: super::holders(&d.root), detail: String::new() }
    }

    /// Processes whose working folder or open files are on the drive, from /proc. Only this
    /// user's processes can be read without root, which are the ones the user can close.
    pub fn holders(root: &Path) -> Vec<Holder> {
        let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
            let base = e.path();
            let on = |p: PathBuf| std::fs::read_link(p).is_ok_and(|t| t.starts_with(root));
            let mut hit = on(base.join("cwd"));
            if !hit && let Ok(fds) = std::fs::read_dir(base.join("fd")) {
                hit = fds.flatten().any(|f| on(f.path()));
            }
            if hit {
                let name = std::fs::read_to_string(base.join("comm")).map(|s| s.trim().to_string()).unwrap_or_else(|_| format!("process {pid}"));
                out.push(Holder { pid, name });
            }
        }
        out
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::*;

    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(path: &str, access: u32) -> Option<Handle> {
        let w = wide(path);
        let h = unsafe { CreateFileW(w.as_ptr(), access, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
        (h != INVALID_HANDLE_VALUE).then_some(Handle(h))
    }

    fn ioctl(h: &Handle, code: u32, input: *const core::ffi::c_void, in_len: u32, output: *mut core::ffi::c_void, out_len: u32) -> bool {
        let mut got = 0u32;
        unsafe { DeviceIoControl(h.0, code, input, in_len, output, out_len, &mut got, std::ptr::null_mut()) != 0 }
    }

    fn device_number(h: &Handle) -> Option<(u32, u32)> {
        let mut n: STORAGE_DEVICE_NUMBER = unsafe { std::mem::zeroed() };
        ioctl(h, IOCTL_STORAGE_GET_DEVICE_NUMBER, std::ptr::null(), 0, (&mut n as *mut STORAGE_DEVICE_NUMBER).cast(), std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32).then_some((n.DeviceType, n.DeviceNumber))
    }

    fn letter(d: &Drive) -> Option<char> {
        let s = d.root.to_string_lossy();
        let mut c = s.chars();
        let l = c.next()?;
        (l.is_ascii_alphabetic() && c.next() == Some(':')).then_some(l.to_ascii_uppercase())
    }

    pub fn eject(d: &Drive, force: bool) -> Result<(), Failure> {
        let l = letter(d).ok_or_else(|| Failure::Other("Only drives with a letter can be ejected here.".into()))?;
        let vol = open(&format!(r"\\.\{l}:"), GENERIC_READ | GENERIC_WRITE).ok_or_else(|| Failure::Other(format!("Windows wouldn't open {l}: ({}).", unsafe { GetLastError() })))?;
        // Locking succeeds only when nothing else has a file open on it: that's the busy test.
        let mut locked = false;
        for _ in 0..10 {
            if ioctl(&vol, FSCTL_LOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0) {
                locked = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        if !locked && !force {
            return Err(Failure::Busy { holders: Vec::new(), detail: String::new() });
        }
        // Writes what's cached, then takes the filesystem away (open files become invalid when
        // forced without the lock).
        if !ioctl(&vol, FSCTL_DISMOUNT_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0) {
            return Err(Failure::Other(format!("Windows couldn't dismount {l}: ({}).", unsafe { GetLastError() })));
        }
        let allow = PREVENT_MEDIA_REMOVAL { PreventMediaRemoval: false };
        let _ = ioctl(&vol, IOCTL_STORAGE_MEDIA_REMOVAL, (&allow as *const PREVENT_MEDIA_REMOVAL).cast(), std::mem::size_of::<PREVENT_MEDIA_REMOVAL>() as u32, std::ptr::null_mut(), 0);
        let number = device_number(&vol);
        if d.info.kind == Kind::Image {
            drop(vol);
            return detach_image(number.map(|n| n.1));
        }
        if d.info.kind == Kind::Removable || d.info.kind == Kind::Optical {
            // A card in a reader, or a disc: eject the media, the reader stays.
            if ioctl(&vol, IOCTL_STORAGE_EJECT_MEDIA, std::ptr::null(), 0, std::ptr::null_mut(), 0) {
                return Ok(());
            }
        }
        // The handle has to be closed before Plug and Play is asked, or it's an open file.
        drop(vol);
        let Some((kind, number)) = number else { return Ok(()) };
        let Some(inst) = disk_instance(kind, number) else { return Ok(()) };
        request_eject(inst)
    }

    /// The device instance of the disk with this storage number.
    fn disk_instance(kind: u32, number: u32) -> Option<u32> {
        // GUID_DEVINTERFACE_DISK / _CDROM; FILE_DEVICE_CD_ROM and FILE_DEVICE_DVD (winioctl.h).
        const FILE_DEVICE_CD_ROM: u32 = 0x02;
        const FILE_DEVICE_DVD: u32 = 0x33;
        let guid = if kind == FILE_DEVICE_CD_ROM || kind == FILE_DEVICE_DVD {
            windows_sys::core::GUID { data1: 0x53F56308, data2: 0xB6BF, data3: 0x11D0, data4: [0x94, 0xF2, 0x00, 0xA0, 0xC9, 0x1E, 0xFB, 0x8B] }
        } else {
            windows_sys::core::GUID { data1: 0x53F56307, data2: 0xB6BF, data3: 0x11D0, data4: [0x94, 0xF2, 0x00, 0xA0, 0xC9, 0x1E, 0xFB, 0x8B] }
        };
        unsafe {
            let set = SetupDiGetClassDevsW(&guid, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
            if set == -1 {
                return None;
            }
            let mut found = None;
            for i in 0.. {
                let mut iface: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
                iface.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
                if SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &guid, i, &mut iface) == 0 {
                    break;
                }
                let mut need = 0u32;
                SetupDiGetDeviceInterfaceDetailW(set, &iface, std::ptr::null_mut(), 0, &mut need, std::ptr::null_mut());
                if need == 0 {
                    continue;
                }
                // u32s keep the buffer aligned for the struct's cbSize.
                let mut buf = vec![0u32; (need as usize).div_ceil(4) + 1];
                let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
                (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                let mut dev: SP_DEVINFO_DATA = std::mem::zeroed();
                dev.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
                if SetupDiGetDeviceInterfaceDetailW(set, &iface, detail, need, std::ptr::null_mut(), &mut dev) == 0 {
                    continue;
                }
                let p = (*detail).DevicePath.as_ptr();
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                let path = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                if open(&path, 0).and_then(|h| device_number(&h)).is_some_and(|(_, num)| num == number) {
                    found = Some(dev.DevInst);
                    break;
                }
            }
            SetupDiDestroyDeviceInfoList(set);
            found
        }
    }

    /// "Safely Remove": ask Plug and Play to eject the disk's parent (the USB device), then the
    /// disk itself if the parent won't.
    #[allow(non_upper_case_globals)] // the PNP_Veto* names are Windows' own
    fn request_eject(inst: u32) -> Result<(), Failure> {
        let mut parent = 0u32;
        let targets: Vec<u32> = if unsafe { CM_Get_Parent(&mut parent, inst, 0) } == CR_SUCCESS { vec![parent, inst] } else { vec![inst] };
        let mut last = Failure::Other("Windows wouldn't eject it.".into());
        for t in targets {
            let mut veto: PNP_VETO_TYPE = 0;
            let mut name = [0u16; 260];
            let r = unsafe { CM_Request_Device_EjectW(t, &mut veto, name.as_mut_ptr(), name.len() as u32, 0) };
            if r == CR_SUCCESS && veto == PNP_VetoTypeUnknown {
                return Ok(());
            }
            let who = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(0)]);
            last = match veto {
                PNP_VetoOutstandingOpen | PNP_VetoPendingClose => Failure::Busy { holders: Vec::new(), detail: String::new() },
                PNP_VetoWindowsApp => Failure::Busy { holders: if who.is_empty() { Vec::new() } else { vec![Holder { pid: 0, name: program_name_win(&who) }] }, detail: String::new() },
                PNP_VetoWindowsService => Failure::Busy { holders: Vec::new(), detail: format!("A Windows service is using it{}.", if who.is_empty() { String::new() } else { format!(" ({who})") }) },
                PNP_VetoNonDisableable => Failure::Other("Windows needs this drive and won't eject it (it may hold a page file or be the boot disk).".into()),
                PNP_VetoInsufficientRights => Failure::Other("Windows needs an administrator to eject it.".into()),
                PNP_VetoAlreadyRemoved => return Ok(()),
                _ => Failure::Other(format!("Windows wouldn't eject it (reason {veto}{}).", if who.is_empty() { String::new() } else { format!(", {who}") })),
            };
        }
        Err(last)
    }

    fn program_name_win(s: &str) -> String {
        s.rsplit('\\').next().unwrap_or(s).trim_end_matches(".exe").trim_end_matches(".EXE").to_string()
    }

    /// A mounted .vhd, .vhdx or .iso: dismounted through the Storage module, which does it
    /// without asking for administrator rights.
    fn detach_image(number: Option<u32>) -> Result<(), Failure> {
        use std::os::windows::process::CommandExt;
        let Some(n) = number else { return Err(Failure::Other("Windows doesn't say which disk the image is.".into())) };
        let script = format!(r"Dismount-DiskImage -DevicePath '\\.\PHYSICALDRIVE{n}' -ErrorAction Stop | Out-Null");
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .map_err(|e| Failure::Other(format!("Couldn't run PowerShell: {e}")))?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            let line = err.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Windows couldn't dismount the image.");
            Err(Failure::Other(line.to_string()))
        }
    }

    pub fn holders(_root: &Path) -> Vec<Holder> {
        Vec::new()
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use super::*;
    pub fn eject(_d: &Drive, _force: bool) -> Result<(), Failure> {
        Err(Failure::Other("Ejecting isn't supported on this system.".into()))
    }
    pub fn holders(_root: &Path) -> Vec<Holder> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drives::Info;

    fn drive(root: &str, label: &str, kind: Kind, disk: &str, image: Option<&str>) -> Drive {
        Drive { root: root.into(), label: label.into(), total: 1, free: 0, network: kind == Kind::Network, info: Info { kind, disk: disk.into(), image: image.map(PathBuf::from), ..Default::default() } }
    }

    #[test]
    fn what_can_be_ejected() {
        assert!(ejectable(&drive("/Volumes/DataHerdA", "DataHerdA", Kind::External, "disk4", None)));
        assert!(ejectable(&drive("/Volumes/MixDev", "MixDev", Kind::Image, "disk6", None)));
        assert!(ejectable(&drive("/media/rich/SD", "SD", Kind::Removable, "mmcblk0", None)));
        assert!(!ejectable(&drive("/", "Macintosh HD", Kind::Internal, "disk0", None)));
        assert!(!ejectable(&drive("/Volumes/Recovery", "Recovery", Kind::System, "disk0", None)));
        assert!(!ejectable(&drive("/Volumes/media", "media", Kind::Network, "", None)));
        assert!(!ejectable(&drive("/run/fuse", "x", Kind::Virtual, "", None)));
    }

    #[test]
    fn protects_the_system_home_and_itself() {
        let r = |s: &str| PathBuf::from(s);
        #[cfg(not(windows))]
        {
            assert!(protected_with(&r("/"), None, None).is_some());
            assert!(protected_with(&r("/home"), None, None).is_some());
            assert!(protected_with(&r("/Volumes/Ext"), Some(&r("/Volumes/Ext/rich")), None).unwrap().contains("home"));
            assert!(protected_with(&r("/Volumes/Ext"), None, Some(&r("/Volumes/Ext/Filescape.app/Contents/MacOS/filescape"))).unwrap().contains("running"));
            assert_eq!(protected_with(&r("/Volumes/Ext"), Some(&r("/Users/rich")), Some(&r("/Applications/Filescape.app/x"))), None);
        }
        #[cfg(windows)]
        {
            assert!(protected_with(&r(r"C:\"), None, None).is_some());
            assert_eq!(protected_with(&r(r"E:\"), Some(&r(r"C:\Users\rich")), None), None);
        }
    }

    /// MixDev, a sparse bundle on DataHerdA, goes first; DataHerdA's other volume goes along.
    #[test]
    fn plan_takes_images_and_siblings() {
        let all = vec![
            drive("/", "Macintosh HD", Kind::Internal, "disk0", None),
            drive("/Volumes/DataHerdA", "DataHerdA", Kind::External, "disk4", None),
            drive("/Volumes/DataHerdB", "DataHerdB", Kind::External, "disk4", None),
            drive("/Volumes/MixDev", "MixDev", Kind::Image, "disk6", Some("/Volumes/DataHerdA/MixDirectiveDev.sparsebundle")),
            drive("/Volumes/Inner", "Inner", Kind::Image, "disk7", Some("/Volumes/MixDev/inner.dmg")),
            drive("/Volumes/Other", "Other", Kind::External, "disk5", None),
        ];
        let p = plan(&all[1], &all);
        assert_eq!(p.siblings.iter().map(|d| d.label.as_str()).collect::<Vec<_>>(), ["DataHerdB"]);
        // The image inside MixDev has to go before MixDev.
        assert_eq!(p.images.iter().map(|d| d.label.as_str()).collect::<Vec<_>>(), ["Inner", "MixDev"]);
        // Ejecting the image alone takes nothing else.
        let p = plan(&all[3], &all);
        assert!(p.siblings.is_empty());
        assert_eq!(p.images.iter().map(|d| d.label.as_str()).collect::<Vec<_>>(), ["Inner"]);
        let p = plan(&all[5], &all);
        assert!(p.siblings.is_empty() && p.images.is_empty());
    }

    #[test]
    fn reads_who_is_using_it() {
        let msg = "Unmount of disk4 failed: at least one volume could not be unmounted\nUnmount failed for /Volumes/DataHerdA\nVolume DataHerdA on disk4s2 failed to unmount: dissented by PID 412 (/Applications/Preview.app/Contents/MacOS/Preview)\nDissenter parent PPID 1 (/sbin/launchd)";
        assert!(says_busy(msg));
        assert_eq!(dissenters(msg), vec![Holder { pid: 412, name: "Preview".into() }]);
        assert_eq!(parse_lsof("p412\ncPreview\np99\ncmdworker\n"), vec![Holder { pid: 412, name: "Preview".into() }, Holder { pid: 99, name: "mdworker".into() }]);
        assert_eq!(program_name("/usr/bin/vim"), "vim");
        assert!(says_busy("umount: /media/rich/USB: target is busy."));
        assert!(says_busy("Error ejecting mount: Device or resource busy"));
        assert!(!says_busy("Error ejecting mount: Not authorized to perform operation"));
    }
}
