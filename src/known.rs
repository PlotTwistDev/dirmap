//! What system files and folders are: a built-in, offline guide to the items that aren't obvious
//! (pagefile.sys, WinSxS, .Spotlight-V100, /proc, swap files…), whether they're safe to delete,
//! and how to shrink them safely when they can be.
//!
//! Each entry names where the item lives with a small pattern:
//! - `/a/b` at the root of a file system (on Windows, the root of any drive or share),
//! - `~/a/b` in any user's home folder,
//! - `a/b` anywhere (the last parts of a path).
//!
//! A part may use `*` and `?` wildcards. Matching ignores case and works on the path text only,
//! so it never touches the disk and costs next to nothing per row.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
    /// Found on any system: removable drives, project folders.
    Any,
}

impl Os {
    pub fn here() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

/// What to do about an item, in one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The system or apps need it. Deleting it breaks things.
    Keep,
    /// Don't delete it by hand, but the system has a safe way to make it smaller.
    Managed,
    /// Temporary or rebuilt on demand: clearing its contents is safe.
    Clean,
    /// Holds something you might want (backups, an old Windows): up to you.
    Yours,
    /// Not really on the disk (kernel views, mount points, links): takes no space here.
    Virtual,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Keep => "Don't delete",
            Verdict::Managed => "Let the system manage it",
            Verdict::Clean => "Safe to clear",
            Verdict::Yours => "Your call",
            Verdict::Virtual => "Takes no disk space",
        }
    }

    /// The colour of its icon and label, readable on light and dark backgrounds.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Verdict::Keep => [214, 72, 64],
            Verdict::Managed => [212, 140, 32],
            Verdict::Clean => [56, 160, 88],
            Verdict::Yours => [74, 134, 222],
            Verdict::Virtual => [140, 140, 150],
        }
    }
}

#[derive(Debug)]
pub struct Known {
    pub os: Os,
    pub at: &'static str,
    /// A short name: "Windows page file".
    pub title: &'static str,
    /// What it is and why it's there.
    pub what: &'static str,
    pub verdict: Verdict,
    /// What to do instead of deleting it by hand, or how to clear it safely.
    pub advice: &'static str,
}

impl Known {
    /// Everything about it as plain text, for tooltips.
    pub fn text(&self) -> String {
        format!("{} · {}\n\n{}\n\n{}", self.title, self.verdict.label(), self.what, self.advice)
    }
}

/// The entry for exactly this item, if it's one Filescape knows about.
pub fn explain(path: &Path) -> Option<&'static Known> {
    index().explain(&path.to_string_lossy())
}

/// `explain`, or failing that `inside` (with `true`).
pub fn about(path: &Path) -> Option<(&'static Known, bool)> {
    let i = index();
    let s = path.to_string_lossy();
    i.explain(&s).map(|k| (k, false)).or_else(|| i.inside(&s).map(|k| (k, true)))
}

#[cfg(test)]
/// Where an entry lives, as people write it on its system: `C:\\pagefile.sys`, `~/Library/Caches`.
pub fn shown_at(k: &Known) -> String {
    match k.os {
        Os::Windows => {
            let p = if let Some(r) = k.at.strip_prefix("~/") { format!("%USERPROFILE%\\{r}") } else if k.at.starts_with('/') { format!("C:{}", k.at) } else { k.at.to_string() };
            p.replace('/', "\\")
        }
        _ => k.at.to_string(),
    }
}

#[cfg(test)]
/// The list of every entry, for the manual (kept current by a test in `help`).
pub fn markdown() -> String {
    let mut out = String::new();
    for (os, title) in [(Os::Windows, "Windows"), (Os::Mac, "macOS"), (Os::Linux, "Linux"), (Os::Any, "On any system (removable drives, projects)")] {
        out.push_str(&format!("### {title}\n\n| Item | What it is | Verdict | What to do |\n| --- | --- | --- | --- |\n"));
        for k in ALL.iter().filter(|k| k.os == os) {
            out.push_str(&format!("| `{}` | **{}.** {} | {} | {} |\n", shown_at(k), k.title, k.what, k.verdict.label(), k.advice));
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn index() -> &'static Index {
    static I: OnceLock<Index> = OnceLock::new();
    I.get_or_init(|| {
        let home = crate::fsys::places::home().map(|h| h.to_string_lossy().into_owned());
        Index::new(Os::here(), home.as_deref())
    })
}

enum Anchor {
    Root,
    Home,
    Anywhere,
}

struct Rule {
    anchor: Anchor,
    parts: Vec<String>,
    k: &'static Known,
}

pub struct Index {
    os: Os,
    /// Each home folder pattern, as parts: `users/*`, `home/*`, `root`, and the real home.
    homes: Vec<Vec<String>>,
    /// Rules by their last part, when it has no wildcard.
    by_name: HashMap<String, Vec<Rule>>,
    /// Rules whose last part has a wildcard.
    wild: Vec<Rule>,
}

impl Index {
    pub fn new(os: Os, home: Option<&str>) -> Index {
        let mut homes: Vec<Vec<String>> = match os {
            Os::Windows | Os::Mac => vec![vec!["users".into(), "*".into()]],
            _ => vec![vec!["home".into(), "*".into()], vec!["root".into()]],
        };
        if let Some(h) = home {
            let p = parts(os, h);
            if !p.is_empty() && !homes.iter().any(|m| matches(m, &p)) {
                homes.push(p);
            }
        }
        let mut by_name: HashMap<String, Vec<Rule>> = HashMap::new();
        let mut wild = Vec::new();
        for k in ALL.iter().filter(|k| k.os == os || k.os == Os::Any) {
            let (anchor, rest) = if let Some(r) = k.at.strip_prefix("~/") {
                (Anchor::Home, r)
            } else if let Some(r) = k.at.strip_prefix('/') {
                (Anchor::Root, r)
            } else {
                (Anchor::Anywhere, k.at)
            };
            let parts: Vec<String> = rest.split('/').map(|s| s.to_lowercase()).collect();
            let last = parts.last().unwrap().clone();
            let rule = Rule { anchor, parts, k };
            if last.contains(['*', '?']) {
                wild.push(rule);
            } else {
                by_name.entry(last).or_default().push(rule);
            }
        }
        Index { os, homes, by_name, wild }
    }

    pub fn explain(&self, path: &str) -> Option<&'static Known> {
        self.at(&parts(self.os, path))
    }

    pub fn inside(&self, path: &str) -> Option<&'static Known> {
        let p = parts(self.os, path);
        (1..p.len()).rev().find_map(|n| self.at(&p[..n]))
    }

    /// The best rule for a path given as lowercase parts: the most specific one.
    fn at(&self, p: &[String]) -> Option<&'static Known> {
        let last = p.last()?;
        let named = self.by_name.get(last).into_iter().flatten();
        let mut best: Option<(usize, &'static Known)> = None;
        for r in named.chain(self.wild.iter()) {
            if let Some(score) = self.fits(r, p) {
                if best.is_none_or(|b| score > b.0) {
                    best = Some((score, r.k));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// How well `r` fits the path, or `None`. Longer patterns win; anchored ones beat
    /// "anywhere" ones of the same length; this system's own entries beat general ones.
    fn fits(&self, r: &Rule, p: &[String]) -> Option<usize> {
        let n = r.parts.len();
        let own = usize::from(r.k.os != Os::Any);
        match r.anchor {
            Anchor::Root => (p.len() == n && matches(&r.parts, p)).then_some(n * 4 + 2 + own),
            Anchor::Anywhere => (p.len() >= n && matches(&r.parts, &p[p.len() - n..])).then_some(n * 4 + own),
            Anchor::Home => self
                .homes
                .iter()
                .any(|h| p.len() == h.len() + n && matches(h, &p[..h.len()]) && matches(&r.parts, &p[h.len()..]))
                .then_some(n * 4 + 1 + own),
        }
    }
}

/// A path as lowercase parts below its root: drive letters and network share names are left
/// out (so `C:\Windows` and `\\pc\c$\Windows` are both `windows`), and on macOS the Data
/// volume and `/private` aliases are folded into the names people know.
fn parts(os: Os, path: &str) -> Vec<String> {
    let mut s = path.replace('\\', "/");
    if os == Os::Windows || os == Os::Any {
        if let Some(r) = s.strip_prefix("//?/UNC/").map(str::to_string) {
            s = format!("//{r}");
        } else if let Some(r) = s.strip_prefix("//?/").map(str::to_string) {
            s = r;
        }
        if let Some(r) = s.strip_prefix("//") {
            // \\server\share\rest: the share is the root.
            s = r.splitn(3, '/').nth(2).unwrap_or("").to_string();
        } else if s.len() >= 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic() {
            s = s[2..].to_string();
        }
    }
    let mut v: Vec<String> = s.split('/').filter(|p| !p.is_empty() && *p != ".").map(|p| p.to_lowercase()).collect();
    if os == Os::Mac {
        if v.len() > 3 && v[0] == "system" && v[1] == "volumes" && v[2] == "data" {
            v.drain(..3);
        }
        if matches!(v.first().map(String::as_str), Some("var" | "tmp" | "etc")) {
            v.insert(0, "private".into());
        }
    }
    v
}

/// Whether each pattern part matches the path part at the same place (same lengths).
fn matches(pat: &[String], p: &[String]) -> bool {
    pat.len() == p.len() && pat.iter().zip(p).all(|(a, b)| glob(a.as_bytes(), b.as_bytes()))
}

/// `*` and `?` wildcards within one part.
fn glob(pat: &[u8], s: &[u8]) -> bool {
    match pat.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) => (0..=s.len()).any(|i| glob(rest, &s[i..])),
        Some((&c, rest)) => s.split_first().is_some_and(|(&d, srest)| (c == b'?' || c == d) && glob(rest, srest)),
    }
}

use Os::{Any, Linux, Mac, Windows};
use Verdict::{Clean, Keep, Managed, Virtual, Yours};

const fn k(os: Os, at: &'static str, title: &'static str, verdict: Verdict, what: &'static str, advice: &'static str) -> Known {
    Known { os, at, title, what, verdict, advice }
}

/// Everything Filescape can explain. Keep each text short: it's shown in a tooltip.
pub static ALL: &[Known] = &[
    // ---------------------------------------------------------------- Windows
    k(Windows, "/pagefile.sys", "Windows page file", Managed,
        "Virtual memory. Windows moves memory that isn't being used right now into this file, so RAM is free for what you're doing. Its size follows how much RAM you have and what you run.",
        "Don't delete it. To make it smaller or move it to another drive: System Properties › Advanced › Performance › Settings › Advanced › Virtual memory (run sysdm.cpl). Letting Windows manage the size is usually best."),
    k(Windows, "/hiberfil.sys", "Hibernation file", Managed,
        "When the PC hibernates, and for Fast Startup, Windows saves what's in memory here so it can pick up where it left off. Often 40% of the size of your RAM.",
        "If you never hibernate, run `powercfg /hibernate off` in an administrator terminal: the file goes away (Fast Startup turns off too). `powercfg /h /type reduced` shrinks it and keeps Fast Startup."),
    k(Windows, "/swapfile.sys", "Swap file for Store apps", Managed,
        "Windows uses it to put Store apps to sleep and wake them quickly. Usually 256 MB or less.",
        "Don't delete it. It's managed with the page file and takes little space."),
    k(Windows, "/DumpStack.log*", "Crash dump log", Keep,
        "A tiny log written at start-up by the part of Windows that records crash dumps.",
        "Leave it: it's in use while Windows runs and takes almost nothing."),
    k(Windows, "/Windows", "Windows", Keep,
        "Windows itself: the system, its drivers, fonts and settings.",
        "Never delete things here by hand. Use Settings › System › Storage › Temporary files, or Disk Cleanup (cleanmgr) › Clean up system files, which know what's safe."),
    k(Windows, "/Windows/WinSxS", "Windows component store", Managed,
        "Every version of the system components Windows has installed, so updates can be undone and features turned on without the install disc. Most files in it are hard links shared with System32, so it looks bigger than the space it really takes.",
        "Never delete files in it. To remove versions updates have replaced: Disk Cleanup › Clean up system files › Windows Update Cleanup, or `Dism.exe /Online /Cleanup-Image /StartComponentCleanup` in an administrator terminal."),
    k(Windows, "/Windows/Installer", "Installer cache", Keep,
        "Copies of the installers (.msi, .msp) of programs and updates, which Windows needs to repair, update or uninstall them.",
        "Don't delete files here: programs may then fail to update or uninstall. Uninstall programs you don't use in Settings › Apps instead."),
    k(Windows, "/Windows/SoftwareDistribution", "Windows Update folder", Managed,
        "Windows Update's working folder: downloaded updates and the update history.",
        "Disk Cleanup › Clean up system files › Windows Update Cleanup removes what's no longer needed. Don't delete the folder while updates are installing."),
    k(Windows, "/Windows/SoftwareDistribution/Download", "Downloaded updates", Clean,
        "Updates Windows downloaded. Once they're installed they aren't needed.",
        "Safe to empty when no update is installing (Windows downloads again anything it still needs). Disk Cleanup or Settings › Storage › Temporary files does it for you."),
    k(Windows, "/Windows/Temp", "System temporary files", Clean,
        "Temporary files from Windows, installers and services.",
        "Settings › System › Storage › Temporary files clears it. Deleting by hand is fine too; files still in use are skipped."),
    k(Windows, "/Windows/Prefetch", "Prefetch", Clean,
        "Notes on which files programs read when they start, so Windows can load them faster.",
        "Harmless to clear, but there's little point: it's small, Windows trims it itself, and programs start slower until it's rebuilt."),
    k(Windows, "/Windows/System32", "System programs", Keep,
        "The core of Windows: system programs, libraries and drivers (64-bit, despite the name).",
        "Never delete anything here."),
    k(Windows, "/Windows/SysWOW64", "32-bit system files", Keep,
        "The 32-bit versions of Windows' programs and libraries, used by 32-bit apps.",
        "Never delete anything here."),
    k(Windows, "/Windows/System32/DriverStore", "Driver store", Keep,
        "Every driver package installed on this PC, so devices work again when they're plugged back in. Old graphics drivers pile up here.",
        "Don't delete by hand. Disk Cleanup › Clean up system files › Device driver packages removes old versions; or list them with `pnputil /enum-drivers` and remove one with `pnputil /delete-driver oemNN.inf`."),
    k(Windows, "/Windows/Logs", "Windows logs", Clean,
        "Logs from updates, servicing (CBS) and setup. CBS logs can grow large when something keeps failing.",
        "Old .log and .cab files are safe to delete. Disk Cleanup › Setup log files does it too."),
    k(Windows, "/Windows/LiveKernelReports", "Hang reports", Clean,
        "Memory dumps Windows writes when a driver stops responding, often the graphics driver. Can reach several GB.",
        "Safe to delete the .dmp files unless you're troubleshooting the problem."),
    k(Windows, "/Windows/Minidump", "Crash dumps", Clean,
        "Small memory dumps from blue-screen crashes.",
        "Safe to delete once you're not troubleshooting a crash. Disk Cleanup › System error minidump files."),
    k(Windows, "/Windows/MEMORY.DMP", "Crash dump", Clean,
        "The full memory dump of the last blue-screen crash. Can be as big as your RAM.",
        "Safe to delete unless someone is investigating the crash. Disk Cleanup › System error memory dump files."),
    k(Windows, "/Windows/assembly", "Global assembly cache", Keep,
        "Shared .NET Framework libraries that programs rely on.",
        "Never delete anything here."),
    k(Windows, "/Windows/servicing", "Servicing stack", Keep,
        "The part of Windows that installs updates and features.",
        "Never delete anything here."),
    k(Windows, "/Windows/CSC", "Offline Files cache", Managed,
        "Copies of network files kept for use offline (the Offline Files feature).",
        "Don't delete it by hand. Turn off Offline Files, or delete its copies, in Control Panel › Sync Center."),
    k(Windows, "/Windows.old", "Previous Windows", Yours,
        "Your previous Windows installation, kept after an upgrade so you can go back (for about 10 days).",
        "When you're happy with the new version: Settings › System › Storage › Temporary files › Previous Windows installation(s). Deleting it by hand usually fails on permissions."),
    k(Windows, "/$WINDOWS.~BT", "Windows upgrade files", Clean,
        "Windows Setup's working folder from an upgrade: downloads and what it needs to roll back.",
        "Disk Cleanup › Clean up system files › Temporary Windows installation files removes it."),
    k(Windows, "/$WINDOWS.~WS", "Windows upgrade files", Clean,
        "Left behind by the Media Creation Tool or an upgrade.",
        "Safe to delete once the upgrade is finished. Disk Cleanup › Temporary Windows installation files removes it."),
    k(Windows, "/$WinREAgent", "Recovery update files", Clean,
        "A working folder for updates to the Windows Recovery Environment.",
        "Usually small, and Windows removes it itself once the update is done."),
    k(Windows, "/$SysReset", "Reset logs", Clean,
        "Logs from a \"Reset this PC\" that finished or failed.",
        "Safe to delete unless you're troubleshooting the reset."),
    k(Windows, "/$GetCurrent", "Upgrade logs", Clean,
        "Logs and files from a Windows upgrade.",
        "Safe to delete once the upgrade is finished."),
    k(Windows, "/Recovery", "Recovery tools", Keep,
        "Files Windows uses to repair or reset itself.",
        "Don't delete it: without it, Reset this PC and start-up repair may not work."),
    k(Windows, "/ProgramData", "Shared program data", Keep,
        "Data and settings that programs keep for all users. Hidden.",
        "Big folders inside belong to specific programs: clean them up through that program, or uninstall it."),
    k(Windows, "/ProgramData/Package Cache", "Installer package cache", Keep,
        "Installers kept by Visual Studio, .NET and other programs, used to repair, update or uninstall them.",
        "Don't delete it: those programs may then fail to update or uninstall. Uninstalling the program removes its packages."),
    k(Windows, "/ProgramData/Microsoft/Windows/WER", "Error reports", Clean,
        "Windows Error Reporting: crash reports waiting to be sent, and ones already sent.",
        "Safe to clear. Disk Cleanup › Windows error reports, or Settings › Storage › Temporary files."),
    k(Windows, "/Program Files", "Programs", Keep,
        "Installed 64-bit programs.",
        "Remove programs with Settings › Apps › Installed apps, not by deleting their folders."),
    k(Windows, "/Program Files (x86)", "32-bit programs", Keep,
        "Installed 32-bit programs.",
        "Remove programs with Settings › Apps › Installed apps, not by deleting their folders."),
    k(Windows, "/PerfLogs", "Performance logs", Clean,
        "Reports from Performance Monitor. Usually empty.",
        "Safe to delete what's in it."),
    k(Windows, "/Config.Msi", "Install rollback files", Clean,
        "Windows Installer keeps these while a program installs, to undo a failed install.",
        "Windows normally removes it. Safe to delete when nothing is installing."),
    k(Windows, "/MSOCache", "Office install cache", Yours,
        "Installation files from an older Microsoft Office, used to repair or add features without the disc.",
        "Can be deleted if you can download Office again; repairs will then ask for the installer."),
    k(Windows, "/Documents and Settings", "Link to Users", Virtual,
        "A link to C:\\Users kept for old programs. It holds nothing itself.",
        "Leave it: it takes no space."),
    k(Windows, "/bootmgr", "Boot manager", Keep,
        "Starts Windows.",
        "Never delete it: the PC may not start."),
    k(Windows, "/BOOTNXT", "Boot file", Keep,
        "Used while Windows starts.",
        "Never delete it."),
    k(Windows, "/inetpub", "Web server folder", Keep,
        "The folder of Internet Information Services (IIS), Windows' web server. Windows creates it even when IIS is off.",
        "Leave it: recent Windows updates need it, and it's tiny."),
    k(Windows, "~/AppData", "App data", Keep,
        "Settings and data that programs keep for you. Hidden. Local stays on this PC, Roaming can follow you to other PCs, LocalLow is for sandboxed apps.",
        "Big folders inside belong to specific programs: clear their caches in the program, or uninstall it."),
    k(Windows, "~/AppData/Local/Temp", "Your temporary files", Clean,
        "Temporary files from programs and installers. They often stay behind.",
        "Settings › System › Storage › Temporary files clears it. Deleting by hand is fine too; files in use are skipped."),
    k(Windows, "~/AppData/Local/Microsoft/Windows/INetCache", "Internet cache", Clean,
        "Web pages and files cached by Windows and apps that use its web engine.",
        "Safe to clear: Settings › Storage › Temporary files › Temporary Internet Files."),
    k(Windows, "~/AppData/Local/CrashDumps", "App crash dumps", Clean,
        "Memory dumps of programs that crashed.",
        "Safe to delete unless you're sending them to a developer."),
    k(Windows, "~/AppData/Local/Microsoft/Windows/WER", "Error reports", Clean,
        "Your Windows Error Reporting files: crash reports waiting to be sent, and ones already sent.",
        "Safe to clear."),
    k(Windows, "~/AppData/Local/D3DSCache", "Shader cache", Clean,
        "Compiled graphics shaders, so games and apps start faster.",
        "Safe to clear (Settings › Storage › Temporary files › DirectX Shader Cache); it's rebuilt as needed."),
    k(Windows, "~/AppData/Local/Packages", "Store app data", Keep,
        "Settings and data of apps from the Microsoft Store, one folder per app.",
        "Don't delete by hand. Settings › Apps › the app › Advanced options › Reset clears an app's data."),
    k(Windows, "~/AppData/Local/Microsoft/WindowsApps", "App shortcuts", Virtual,
        "Tiny launchers for Store apps (winget, python and others), so they can be started by name.",
        "Leave it: these take next to no space."),
    k(Windows, "~/NTUSER.DAT*", "Your registry", Keep,
        "Your part of the Windows registry: your settings for Windows and programs, and its logs.",
        "Never delete it: your account would stop working."),
    k(Windows, "~/ntuser.ini", "Profile settings", Keep,
        "Part of your Windows profile.",
        "Leave it: it's tiny."),
    // ---------------------------------------------------------------- macOS
    k(Mac, "/System", "macOS", Keep,
        "macOS itself, on a sealed, read-only system volume.",
        "Nothing here can or should be deleted. Space on the system volume isn't yours to reclaim."),
    k(Mac, "/System/Volumes/Data", "Data volume", Keep,
        "The volume with everything you and your apps write. /Users, /Applications and others are shown both here and in their usual places, so tools that follow both count them twice. Time Machine's local snapshots also live on this volume and appear in no folder.",
        "Clean up the folders inside rather than this one. `tmutil listlocalsnapshots /` lists local snapshots; macOS removes them when space runs low."),
    k(Mac, "/System/Volumes/VM", "Virtual memory volume", Managed,
        "Holds the swap files macOS writes when memory is short, and the sleep image.",
        "macOS manages it. Quitting memory-hungry apps or restarting shrinks swap."),
    k(Mac, "/System/Volumes/Preboot", "Preboot volume", Keep,
        "Files macOS needs to start up and unlock FileVault.",
        "Never delete anything here."),
    k(Mac, "/System/Volumes/Update", "Update volume", Keep,
        "Used while macOS installs updates.",
        "macOS clears it itself."),
    k(Mac, "/private/var/vm", "Virtual memory", Managed,
        "Swap files macOS writes when memory is short, and the sleep image (older macOS; newer versions use the VM volume).",
        "macOS manages it. Quitting memory-hungry apps or restarting shrinks swap."),
    k(Mac, "sleepimage", "Sleep image", Managed,
        "When the Mac sleeps, it can save memory here so nothing is lost if the battery runs out. As big as your RAM.",
        "Leave it on laptops. On a desktop Mac, `sudo pmset -a hibernatemode 0` stops writing it, and the file can then be deleted."),
    k(Mac, "vm/swapfile*", "Swap file", Managed,
        "Memory macOS moved to disk because RAM was full.",
        "macOS deletes swap files itself. Quit memory-hungry apps or restart to free them."),
    k(Mac, "/private/var/folders", "Temporary and cache folders", Managed,
        "Each user's private temporary files and caches (where $TMPDIR points). Can grow to several GB.",
        "Don't delete it by hand while you're logged in: running apps can break. A restart clears its temporary part."),
    k(Mac, "/private/var/log", "System logs", Managed,
        "Logs from macOS and services.",
        "macOS rotates and trims them. Old .gz files are safe to delete."),
    k(Mac, "/private/var/db/diagnostics", "Unified logs", Managed,
        "The system log that Console shows.",
        "macOS trims it itself. Leave it."),
    k(Mac, "/private/var/db/uuidtext", "Log support files", Managed,
        "Used to read the unified system log.",
        "macOS trims it itself. Leave it."),
    k(Mac, "/private/var/db/oah", "Rosetta cache", Managed,
        "Intel apps translated by Rosetta 2 to run on Apple silicon.",
        "Leave it: macOS rebuilds it, and Intel apps start slower until then."),
    k(Mac, "/private/var/db", "System databases", Keep,
        "Databases macOS keeps about the system: logins, receipts, Spotlight settings and more.",
        "Don't delete anything here by hand."),
    k(Mac, "/private/tmp", "Temporary files", Clean,
        "Temporary files. Cleared when the Mac restarts.",
        "Restart to clear it. Don't delete files apps are using."),
    k(Mac, "/cores", "Core dumps", Clean,
        "Memory dumps of programs that crashed, written only when core dumps are turned on.",
        "Safe to delete."),
    k(Mac, "/Volumes", "Other drives", Virtual,
        "Where other drives, disk images and network shares appear, plus a link to the startup disk.",
        "Takes no space here: each item is another drive."),
    k(Mac, "/dev", "Devices", Virtual,
        "The kernel's view of disks and other devices. Not files on the disk.",
        "Takes no space."),
    k(Mac, "/Library", "Shared library", Keep,
        "Settings and support files of apps, for all users.",
        "Big folders inside belong to specific apps: clean up through the app, or uninstall it."),
    k(Mac, "/Library/Caches", "Shared caches", Clean,
        "Caches apps keep for all users. Apps rebuild them as needed.",
        "Safe to clear what's inside, best after quitting apps."),
    k(Mac, "/Library/Updates", "Downloaded updates", Managed,
        "macOS updates downloaded and waiting to install.",
        "Install or skip the update in System Settings › General › Software Update; macOS removes the download."),
    k(Mac, "/Applications", "Applications", Yours,
        "Your apps.",
        "Delete an app by moving it to the Trash. Its settings stay in ~/Library until you remove them too."),
    k(Mac, "/usr", "Unix programs", Keep,
        "Command-line programs and libraries that are part of macOS (on the sealed system volume).",
        "Nothing here can or should be deleted, except /usr/local, which holds things you installed."),
    k(Mac, "/opt/homebrew", "Homebrew", Yours,
        "Programs installed with Homebrew (on Apple silicon).",
        "`brew autoremove` removes packages nothing needs; `brew cleanup` removes old versions."),
    k(Mac, "~/Library", "Your library", Keep,
        "Your app settings and data: mail, messages, caches, app support files. Hidden by Finder.",
        "Big folders inside belong to specific apps: clean up through the app, or uninstall it."),
    k(Mac, "~/Library/Caches", "Your caches", Clean,
        "Caches apps keep for you. Apps rebuild them as needed.",
        "Safe to clear what's inside, best after quitting apps."),
    k(Mac, "~/Library/Caches/Homebrew", "Homebrew downloads", Clean,
        "Downloads Homebrew kept after installing.",
        "`brew cleanup --prune=all` clears it."),
    k(Mac, "~/Library/Logs", "Your logs", Clean,
        "Logs and crash reports from your apps.",
        "Safe to delete."),
    k(Mac, "~/Library/Application Support", "App support files", Keep,
        "Data apps keep for you: libraries, databases, downloads, plug-ins.",
        "Big folders inside belong to specific apps. Remove them only for apps you've uninstalled."),
    k(Mac, "~/Library/Application Support/MobileSync/Backup", "iPhone and iPad backups", Yours,
        "Backups of your iPhone or iPad made by Finder or iTunes. Each one can take tens of GB.",
        "Delete old ones in Finder: select the device in the sidebar › Manage Backups."),
    k(Mac, "~/Library/Containers", "App containers", Keep,
        "Data of sandboxed apps, one folder per app.",
        "Don't delete by hand: clear an app's data in the app, or uninstall it."),
    k(Mac, "~/Library/Containers/com.docker.docker", "Docker Desktop", Yours,
        "Docker Desktop's virtual disk: images, containers and volumes.",
        "`docker system prune` removes what nothing uses. Docker Desktop › Settings › Resources sets the disk limit."),
    k(Mac, "~/Library/Group Containers", "Shared app containers", Keep,
        "Data shared between related apps (Office, Messages and others).",
        "Don't delete by hand."),
    k(Mac, "~/Library/Mail", "Mail", Keep,
        "Apple Mail's messages and attachments.",
        "Delete mail in the Mail app, or remove accounts you no longer use."),
    k(Mac, "~/Library/Messages/Attachments", "Messages attachments", Yours,
        "Photos, videos and files sent and received in Messages.",
        "Delete big attachments in System Settings › General › Storage › Messages, or set Keep messages to 1 year in Messages › Settings."),
    k(Mac, "~/Library/CloudStorage", "Cloud folders", Keep,
        "Folders of cloud services (Google Drive, OneDrive, Dropbox…).",
        "Free up space through the cloud app, or with Filescape's Free up space on files you don't need locally."),
    k(Mac, "~/Library/Mobile Documents", "iCloud Drive", Keep,
        "Your iCloud Drive folder on this Mac.",
        "Turn on Optimize Mac Storage in iCloud settings, or remove downloads (Filescape's Free up space)."),
    k(Mac, "~/Library/Developer/Xcode/DerivedData", "Xcode build files", Clean,
        "Build products and indexes Xcode makes for each project.",
        "Safe to delete; Xcode rebuilds what it needs (the next build is slower)."),
    k(Mac, "~/Library/Developer/Xcode/iOS DeviceSupport", "iOS device symbols", Clean,
        "Debug symbols for each iOS version of the devices you've connected to Xcode.",
        "Safe to delete versions you no longer debug on; Xcode fetches them again when needed."),
    k(Mac, "~/Library/Developer/Xcode/Archives", "Xcode archives", Yours,
        "Builds you archived for release. Needed to read crash reports from those versions.",
        "Delete old ones in Xcode › Window › Organizer."),
    k(Mac, "~/Library/Developer/CoreSimulator", "iOS simulators", Yours,
        "Simulated devices and their data.",
        "`xcrun simctl delete unavailable` removes simulators for runtimes you no longer have; remove runtimes in Xcode › Settings › Platforms."),
    k(Mac, "~/.Trash", "Your Trash", Clean,
        "Things you moved to the Trash.",
        "Empty the Trash in Finder."),
    k(Mac, "*.photoslibrary", "Photos library", Keep,
        "The Photos app's library: your photos and videos with their edits and database.",
        "Never delete or move files inside it. Delete photos in Photos, or with iCloud Photos turn on Optimize Mac Storage."),
    // ---------------------------------------------------------------- Linux
    k(Linux, "/proc", "Processes (virtual)", Virtual,
        "The kernel's live view of processes and system state. Not files on the disk.",
        "Takes no space. Sizes shown here aren't real."),
    k(Linux, "/proc/kcore", "Kernel memory (virtual)", Virtual,
        "A view of the computer's memory. It looks enormous but isn't stored anywhere.",
        "Takes no space. Ignore its size."),
    k(Linux, "/sys", "Devices and drivers (virtual)", Virtual,
        "The kernel's view of devices, drivers and settings. Not files on the disk.",
        "Takes no space."),
    k(Linux, "/dev", "Devices", Virtual,
        "Disks, terminals and other devices as files. Kept in memory.",
        "Takes no space on the disk."),
    k(Linux, "/run", "Runtime files", Virtual,
        "Information about running services, kept in memory and cleared at start-up.",
        "Takes no disk space."),
    k(Linux, "/tmp", "Temporary files", Clean,
        "Temporary files. Often kept in memory and cleared at start-up.",
        "Restarting clears it. Don't delete files programs are using."),
    k(Linux, "/var/tmp", "Longer-lived temporary files", Clean,
        "Temporary files that survive a restart. systemd usually removes ones older than 30 days.",
        "Files nothing is using are safe to delete."),
    k(Linux, "/var/log", "System logs", Managed,
        "Logs from the system and services. logrotate rotates and compresses them.",
        "Old rotated files (.1, .gz) are safe to delete. Empty a big log with `sudo truncate -s 0 FILE` rather than deleting it."),
    k(Linux, "/var/log/journal", "systemd journal", Managed,
        "The systemd log. Can grow to several GB.",
        "`sudo journalctl --vacuum-size=200M` shrinks it. Set SystemMaxUse= in /etc/systemd/journald.conf to keep it small."),
    k(Linux, "/var/cache", "System caches", Clean,
        "Caches of system tools and package managers.",
        "Use each package manager's own clean command (`sudo apt clean`, `sudo dnf clean all`, `sudo paccache -rk2`) rather than deleting by hand."),
    k(Linux, "/var/cache/apt/archives", "Downloaded packages (apt)", Clean,
        "Packages apt downloaded to install or update software.",
        "`sudo apt clean` clears it."),
    k(Linux, "/var/cache/pacman/pkg", "Downloaded packages (pacman)", Clean,
        "Every package version pacman downloaded.",
        "`sudo paccache -rk2` keeps the last two versions of each; `sudo pacman -Sc` keeps only installed ones."),
    k(Linux, "/var/cache/dnf", "Downloaded packages (dnf)", Clean,
        "Package metadata and downloads from dnf.",
        "`sudo dnf clean all` clears it."),
    k(Linux, "/var/lib/apt/lists", "Package lists", Clean,
        "The lists of available packages apt downloads.",
        "Safe to delete; `sudo apt update` fetches them again."),
    k(Linux, "/var/lib/snapd", "Snap packages", Managed,
        "Snap packages and their older versions (several are kept per app).",
        "Keep fewer old versions: `sudo snap set system refresh.retain=2`. `snap list --all` shows disabled ones; remove one with `sudo snap remove NAME --revision=N`."),
    k(Linux, "/snap", "Snap apps (mounted)", Virtual,
        "Each snap app mounted from a compressed file in /var/lib/snapd/snaps. Sizes here count the unpacked contents, which aren't on the disk like that.",
        "Remove apps with `sudo snap remove NAME`. The real space is in /var/lib/snapd."),
    k(Linux, "~/snap", "Snap app data", Yours,
        "Your data for snap apps, one folder per app.",
        "Remove an app's folder only after uninstalling the app."),
    k(Linux, "/var/lib/flatpak", "Flatpak apps", Managed,
        "Flatpak apps and the runtimes they need.",
        "`flatpak uninstall --unused` removes runtimes no app uses."),
    k(Linux, "~/.local/share/flatpak", "Your Flatpak apps", Managed,
        "Flatpak apps installed for you only, and their runtimes.",
        "`flatpak uninstall --user --unused` removes runtimes no app uses."),
    k(Linux, "~/.var/app", "Flatpak app data", Yours,
        "Your data and caches for Flatpak apps, one folder per app.",
        "Each app's cache folder is safe to clear; remove an app's folder only after uninstalling it."),
    k(Linux, "/var/lib/docker", "Docker", Yours,
        "Docker images, containers, volumes and build cache.",
        "`docker system prune` removes stopped containers and unused data; add `-a` for unused images. Volumes hold data: prune them only on purpose."),
    k(Linux, "/var/lib/containers", "Podman containers", Yours,
        "Podman images and containers.",
        "`podman system prune` removes what nothing uses."),
    k(Linux, "~/.local/share/containers", "Your Podman containers", Yours,
        "Rootless Podman images and containers.",
        "`podman system prune` removes what nothing uses."),
    k(Linux, "/var/lib/libvirt/images", "Virtual machine disks", Yours,
        "Disks of virtual machines made with virt-manager or libvirt.",
        "Delete a machine and its storage in virt-manager."),
    k(Linux, "/var/lib/systemd/coredump", "Core dumps", Clean,
        "Memory dumps of programs that crashed. systemd caps their size.",
        "Safe to delete. `coredumpctl list` shows what they're from."),
    k(Linux, "/var/crash", "Crash reports", Clean,
        "Crash reports, mostly from Ubuntu's apport.",
        "Safe to delete."),
    k(Linux, "/var/lib", "Program state", Keep,
        "Databases and state kept by system services and package managers.",
        "Don't delete by hand: big folders belong to specific services (docker, snapd, databases)."),
    k(Linux, "/var/spool", "Queues", Keep,
        "Print jobs, mail and scheduled jobs waiting to be handled.",
        "Leave it."),
    k(Linux, "/swapfile", "Swap file", Managed,
        "Swap space: memory the system moved to disk.",
        "To shrink it: `sudo swapoff /swapfile`, `sudo fallocate -l 2G /swapfile`, `sudo mkswap /swapfile`, `sudo swapon /swapfile`. To remove it, also take its line out of /etc/fstab."),
    k(Linux, "/swap.img", "Swap file", Managed,
        "Swap space (Ubuntu's name for it): memory the system moved to disk.",
        "To shrink it: `sudo swapoff /swap.img`, `sudo fallocate -l 2G /swap.img`, `sudo mkswap /swap.img`, `sudo swapon /swap.img`. To remove it, also take its line out of /etc/fstab."),
    k(Linux, "/boot", "Boot files", Keep,
        "The kernels and boot loader the system starts from.",
        "If it's full, remove old kernels with the package manager: `sudo apt autoremove --purge` on Ubuntu and Debian."),
    k(Linux, "/usr/lib/modules", "Kernel modules", Managed,
        "Drivers for each installed kernel version.",
        "Old kernels and their modules go with `sudo apt autoremove --purge` (Ubuntu, Debian) or your package manager."),
    k(Linux, "/lib/modules", "Kernel modules", Managed,
        "Drivers for each installed kernel version.",
        "Old kernels and their modules go with `sudo apt autoremove --purge` (Ubuntu, Debian) or your package manager."),
    k(Linux, "/usr", "Installed software", Keep,
        "Programs, libraries and data installed by the package manager.",
        "Remove software with the package manager, never by deleting files here."),
    k(Linux, "/etc", "System settings", Keep,
        "Configuration of the system and services.",
        "Never delete things here by hand."),
    k(Linux, "/bin", "Programs", Keep,
        "Essential programs. On most current systems a link to /usr/bin.",
        "Never delete it."),
    k(Linux, "/sbin", "System programs", Keep,
        "Programs for administrators. On most current systems a link to /usr/sbin.",
        "Never delete it."),
    k(Linux, "/lib", "Libraries", Keep,
        "Essential libraries. On most current systems a link to /usr/lib.",
        "Never delete it."),
    k(Linux, "/lib64", "64-bit libraries", Keep,
        "Essential 64-bit libraries. On most current systems a link to /usr/lib64 or /usr/lib.",
        "Never delete it."),
    k(Linux, "/opt", "Add-on software", Yours,
        "Software installed outside the package manager (Chrome, vendor tools…).",
        "Uninstall programs the way they were installed."),
    k(Linux, "/mnt", "Mount points", Virtual,
        "Where other drives and shares are mounted by hand.",
        "Takes no space here: each item is another drive."),
    k(Linux, "/media", "Removable drives", Virtual,
        "Where USB drives and discs appear.",
        "Takes no space here: each item is another drive."),
    k(Linux, "/var", "Variable data", Keep,
        "Data that changes while the system runs: logs, caches, databases, spools, containers.",
        "Look inside for what's big: logs, caches and package downloads can be cleaned safely, the rest belongs to services."),
    k(Linux, "/home", "Home folders", Keep,
        "Each user's own files and settings.",
        "Your files are yours to sort out; leave other users' folders alone."),
    k(Linux, "/srv", "Served data", Yours,
        "Data served by this computer, such as websites or FTP files. Often empty.",
        "Belongs to whatever service you set up here."),
    k(Linux, "/nix", "Nix store", Managed,
        "Packages installed with the Nix package manager, including old generations.",
        "`nix-collect-garbage -d` removes old generations and packages nothing uses."),
    k(Linux, "/root", "Administrator's home", Keep,
        "The root user's home folder.",
        "Leave it unless you know what's in it."),
    k(Linux, "/.snapshots", "Snapper snapshots", Yours,
        "Btrfs snapshots made by Snapper, often before every update. Snapshots share data, so their sizes add up to more than they really use.",
        "`sudo snapper list` and `sudo snapper delete N`; set limits in /etc/snapper/configs/root."),
    k(Linux, "/timeshift", "Timeshift snapshots", Yours,
        "System snapshots made by Timeshift.",
        "Delete old snapshots in Timeshift."),
    k(Linux, "~/.cache", "Your caches", Clean,
        "Caches of your apps: browsers, thumbnails, pip, fonts and more.",
        "Safe to clear, best after closing apps; they rebuild what they need."),
    k(Linux, "~/.cache/thumbnails", "Thumbnails", Clean,
        "Picture previews made by the file manager.",
        "Safe to clear; they're made again when needed."),
    k(Linux, "~/.local/share/Trash", "Your Trash", Clean,
        "Things you moved to the Trash.",
        "Empty the Trash in your file manager."),
    k(Linux, "~/.local/share/baloo", "KDE file index", Managed,
        "KDE's search index of your files.",
        "`balooctl purge` rebuilds it; `balooctl disable` turns indexing off."),
    k(Linux, "~/.xsession-errors*", "Session log", Clean,
        "Messages from programs in your desktop session. Can grow large.",
        "Safe to delete; it's recreated when you log in."),
    // ---------------------------------------------------------------- Any system
    k(Any, "$Recycle.Bin", "Recycle Bin", Clean,
        "Things deleted on this drive in Windows, one folder per user.",
        "Empty the Recycle Bin from the desktop, or Settings › System › Storage › Temporary files. Don't delete the folder itself."),
    k(Any, "System Volume Information", "System Volume Information", Managed,
        "Windows' data for this drive: System Restore points, shadow copies (Previous Versions) and the search index. Can grow to many GB.",
        "Windows won't let you delete it. Shrink restore points in System Protection (SystemPropertiesProtection.exe) › Configure › Max Usage. `vssadmin list shadowstorage` shows the space."),
    k(Any, "Thumbs.db", "Thumbnail cache", Clean,
        "Picture previews older Windows (and Windows on network folders) saves in a folder.",
        "Safe to delete; it's made again when needed."),
    k(Any, "ehthumbs*.db", "Media thumbnails", Clean,
        "Thumbnails from Windows Media Center.",
        "Safe to delete."),
    k(Any, "desktop.ini", "Folder settings", Keep,
        "Tiny Windows file that sets a folder's icon or the name Explorer shows for it (Documents, Pictures…).",
        "Leave it: deleting it only resets how the folder looks, and it takes no real space."),
    k(Any, ".DS_Store", "Finder view settings", Clean,
        "macOS Finder's settings for a folder: icon positions, view and sort.",
        "Harmless to delete; Finder makes it again. To stop Macs writing them on network shares: `defaults write com.apple.desktopservices DSDontWriteNetworkStores true`."),
    k(Any, "._*", "Mac file extras", Clean,
        "A Mac saved extra information (tags, icons, resource forks) next to a file on a drive that can't store it, like FAT, exFAT or some network shares.",
        "Safe to delete on drives only Windows or Linux use; a Mac then loses those extras. On a Mac, `dot_clean` merges them back."),
    k(Any, "__MACOSX", "Mac zip extras", Clean,
        "Mac file extras that came out of a zip made on a Mac.",
        "Safe to delete."),
    k(Any, ".Spotlight-V100", "Spotlight index", Managed,
        "macOS's search index for this drive.",
        "On a Mac, add the drive to System Settings › Spotlight › Search Privacy to stop indexing it, or rebuild the index with `sudo mdutil -E /Volumes/NAME`. Deleting it on another system is harmless; the Mac rebuilds it."),
    k(Any, ".fseventsd", "File change log", Keep,
        "macOS's record of changes on this drive, used by Time Machine and Spotlight.",
        "Leave it. On an external drive deleting it is harmless, but the next Time Machine backup may take longer."),
    k(Any, ".DocumentRevisions-V100", "Document versions", Keep,
        "Saved versions of documents for File › Revert To on a Mac.",
        "Don't delete it by hand. Versions of a document go away when the document is deleted."),
    k(Any, ".MobileBackups*", "Local Time Machine snapshots", Managed,
        "Time Machine's local snapshots on older macOS.",
        "macOS removes them when space runs low; `tmutil` manages them."),
    k(Any, "Backups.backupdb", "Time Machine backups", Yours,
        "Time Machine backups on this drive.",
        "Delete old backups from Time Machine itself (or `tmutil delete`), never in Finder or here."),
    k(Any, ".Trashes", "Trash for this drive", Clean,
        "Things a Mac moved to the Trash from this drive.",
        "Empty the Trash while the drive is connected to the Mac."),
    k(Any, ".TemporaryItems", "Mac temporary items", Clean,
        "Temporary files a Mac left on this drive.",
        "Safe to delete when no Mac is using the drive."),
    k(Any, ".Trash-*", "Linux Trash for this drive", Clean,
        "Things moved to the Trash on Linux from this drive.",
        "Empty the Trash, or delete it while nothing is using the drive."),
    k(Any, "lost+found", "Recovered fragments", Keep,
        "Where a file system check puts pieces of files it found without a name. Normally empty.",
        "Look inside for anything you lost after a crash; delete what's there if not. Keep the folder itself."),
    k(Any, "found.0*", "Recovered fragments", Yours,
        "Pieces of files Windows' disk check (chkdsk) recovered.",
        "Look for anything you need (they're named FILE0000.CHK); otherwise safe to delete."),
    k(Any, "node_modules", "JavaScript packages", Clean,
        "Packages a JavaScript project uses. Often huge and full of tiny files.",
        "Safe to delete for projects you aren't working on; `npm install` brings it back."),
    k(Any, "__pycache__", "Python bytecode cache", Clean,
        "Compiled Python files, so scripts start faster.",
        "Safe to delete; Python makes them again."),
    k(Any, ".git", "Git repository", Keep,
        "The whole history of a project under Git.",
        "Don't delete it unless you mean to lose the history. `git gc` packs it tighter."),
    k(Any, "~/.npm", "npm cache", Clean,
        "Packages npm downloaded.",
        "`npm cache clean --force` clears it."),
    k(Any, "~/.cargo/registry", "Rust package cache", Clean,
        "Crates Cargo downloaded.",
        "Safe to delete; Cargo downloads what a build needs again."),
    k(Any, "~/.gradle/caches", "Gradle cache", Clean,
        "Downloaded libraries and build caches of Gradle.",
        "Safe to delete while no build is running."),
    k(Any, "~/.m2/repository", "Maven repository", Clean,
        "Libraries Maven downloaded.",
        "Safe to delete; Maven downloads them again."),
    k(Any, "~/.nuget/packages", "NuGet packages", Clean,
        "Packages .NET projects downloaded.",
        "`dotnet nuget locals all --clear` clears it."),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> Index {
        Index::new(Os::Windows, Some("C:\\Users\\rich"))
    }

    fn title(i: &Index, p: &str) -> Option<&'static str> {
        i.explain(p).map(|k| k.title)
    }

    #[test]
    fn windows_items() {
        let w = win();
        assert_eq!(title(&w, "C:\\pagefile.sys"), Some("Windows page file"));
        assert_eq!(title(&w, "D:\\PAGEFILE.SYS"), Some("Windows page file"));
        assert_eq!(title(&w, "C:\\Windows\\WinSxS"), Some("Windows component store"));
        assert_eq!(title(&w, "\\\\?\\C:\\Windows\\WinSxS"), Some("Windows component store"));
        assert_eq!(title(&w, "\\\\pc\\c$\\Windows\\WinSxS"), Some("Windows component store"));
        assert_eq!(title(&w, "E:\\$RECYCLE.BIN"), Some("Recycle Bin"));
        assert_eq!(title(&w, "C:\\Users\\anna\\AppData\\Local\\Temp"), Some("Your temporary files"));
        assert_eq!(title(&w, "C:\\Users\\rich\\NTUSER.DAT{53b39e88}.TM.blf"), Some("Your registry"));
        // Only at the root.
        assert_eq!(title(&w, "C:\\Backups\\pagefile.sys"), None);
        assert_eq!(title(&w, "C:\\Data\\Windows"), None);
        // Not other systems' items.
        assert_eq!(title(&w, "C:\\proc"), None);
        assert_eq!(title(&w, "C:\\pagefile"), None);
    }

    #[test]
    fn most_specific_wins() {
        let w = win();
        assert_eq!(title(&w, "C:\\Windows\\SoftwareDistribution\\Download"), Some("Downloaded updates"));
        assert_eq!(title(&w, "C:\\Windows\\SoftwareDistribution"), Some("Windows Update folder"));
        // The Windows-only entry for the user's WER folder, not a general one.
        assert_eq!(w.explain("C:\\ProgramData\\Microsoft\\Windows\\WER").unwrap().verdict, Clean);
    }

    #[test]
    fn inside_finds_the_nearest_folder() {
        let w = win();
        assert_eq!(w.inside("C:\\Windows\\WinSxS\\amd64_x\\a.dll").map(|k| k.title), Some("Windows component store"));
        assert_eq!(w.inside("C:\\Windows\\notepad.exe").map(|k| k.title), Some("Windows"));
        assert_eq!(w.inside("C:\\Windows").map(|k| k.title), None);
        assert_eq!(w.inside("D:\\work\\app\\node_modules\\react\\index.js").map(|k| k.title), Some("JavaScript packages"));
        assert!(w.inside("D:\\work\\notes.txt").is_none());
    }

    #[test]
    fn mac_items() {
        let m = Index::new(Os::Mac, Some("/Users/rich"));
        assert_eq!(title(&m, "/var/vm"), Some("Virtual memory"));
        assert_eq!(title(&m, "/private/var/vm/sleepimage"), Some("Sleep image"));
        assert_eq!(title(&m, "/System/Volumes/VM/swapfile3"), Some("Swap file"));
        assert_eq!(title(&m, "/System/Volumes/Data/Users/rich/Library/Caches"), Some("Your caches"));
        assert_eq!(title(&m, "/System/Volumes/Data"), Some("Data volume"));
        assert_eq!(title(&m, "/Volumes/Backup/.Spotlight-V100"), Some("Spotlight index"));
        assert_eq!(title(&m, "/Users/rich/Pictures/Photos Library.photoslibrary"), Some("Photos library"));
        assert_eq!(title(&m, "/Users/rich/Library/Application Support/MobileSync/Backup"), Some("iPhone and iPad backups"));
        assert_eq!(title(&m, "/Users/rich/Documents/.DS_Store"), Some("Finder view settings"));
        assert_eq!(title(&m, "/Users/rich/Documents/._report.pdf"), Some("Mac file extras"));
        assert_eq!(title(&m, "/Users/rich/Documents/report.pdf"), None);
    }

    #[test]
    fn linux_items() {
        let l = Index::new(Os::Linux, Some("/srv/people/rich"));
        assert_eq!(l.explain("/proc").unwrap().verdict, Virtual);
        assert_eq!(title(&l, "/proc/kcore"), Some("Kernel memory (virtual)"));
        assert_eq!(title(&l, "/var/log/journal"), Some("systemd journal"));
        assert_eq!(title(&l, "/home/anna/.cache"), Some("Your caches"));
        assert_eq!(title(&l, "/root/.cache"), Some("Your caches"));
        // A home folder outside /home is found through the real home.
        assert_eq!(title(&l, "/srv/people/rich/.cache"), Some("Your caches"));
        assert_eq!(title(&l, "/media/rich/USB/.Trash-1000"), Some("Linux Trash for this drive"));
        assert_eq!(title(&l, "/media/rich/USB/System Volume Information"), Some("System Volume Information"));
        assert_eq!(title(&l, "/home/anna/project/target"), None);
        // macOS-only and Windows-only entries don't show up on Linux.
        assert_eq!(title(&l, "/private/var/vm"), None);
        assert_eq!(title(&l, "/pagefile.sys"), None);
    }

    #[test]
    fn every_entry_is_reachable_and_well_formed() {
        for os in [Os::Windows, Os::Mac, Os::Linux] {
            let i = Index::new(os, None);
            for k in ALL.iter().filter(|k| k.os == os) {
                assert!(!k.title.is_empty() && k.what.ends_with('.') && k.advice.ends_with(['.', ')']), "{}", k.at);
                assert!(k.text().len() < 700, "{} is too long for a tooltip", k.at);
                // Build a real path for the pattern and check this entry is the one found.
                let mut p = k.at.replace('*', "x").replace('?', "x");
                if let Some(r) = p.strip_prefix("~/") {
                    p = format!("{}/{r}", if os == Os::Linux { "/home/u" } else { "/Users/u" });
                } else if !p.starts_with('/') {
                    p = format!("/data/{p}");
                }
                assert_eq!(i.explain(&p).map(|f| f.at), Some(k.at), "{os:?} {p}");
            }
        }
        // Patterns are unique per system.
        let mut seen = std::collections::HashSet::new();
        for k in ALL {
            assert!(seen.insert((k.os as u8, k.at.to_lowercase())), "duplicate {}", k.at);
        }
    }

    #[test]
    fn globs() {
        assert!(glob(b"ntuser.dat*", b"ntuser.dat"));
        assert!(glob(b"ntuser.dat*", b"ntuser.dat.log1"));
        assert!(glob(b"._*", b"._a"));
        assert!(!glob(b"._*", b"_a"));
        assert!(glob(b"found.0??", b"found.000"));
    }
}
