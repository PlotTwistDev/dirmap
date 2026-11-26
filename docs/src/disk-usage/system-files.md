# What system files and folders are

Every drive has files and folders you didn't make and whose names say little: `pagefile.sys`,
`WinSxS`, `.Spotlight-V100`, `/proc`. They're often among the biggest items on the map. Filescape
knows the common ones on Windows, macOS and Linux and tells you **what each one is, whether it's
safe to delete, and how to make it smaller safely** when it can be. The guide is built into the
app and works offline.

## Where you see it

- **An info icon after the name.** In the file manager, the Storage Analysis list and the folder
  tree, a small *i* in a circle follows the name of each item Filescape can explain. Its colour gives the verdict at a
  glance (see below). Hover it to read the explanation; click it to keep it open in a window.
- **The preview pane.** Select the item (or anything inside it) and the pane shows a
  *What is this?* box under the details.
- **The status bar over the map.** Hovering a tile adds a short note, for example
  *Hibernation file: let the system manage it*, or *in Windows component store: let the
  system manage it* for a file inside such a folder.
- **Right-click › What Is This?** in the file manager, the Storage Analysis list, the tree and
  on the map opens the explanation in a window, also for items inside a known folder.

Home folders (`/home`, `/root`) and the places other drives appear (`/media`, `/mnt`,
`/Volumes`) are explained only for themselves: your own files inside them get no note, unless
they're in a known folder such as `~/.cache`.

Turn the icons off with **⚙ Settings › General › Explain system files**. The preview box and the status
bar note follow the same setting; the menu item is always there.

## The verdicts

| Verdict | Colour | Meaning |
| --- | --- | --- |
| Don't delete | Red | Windows, macOS, Linux or your apps need it. Deleting it breaks things. |
| Let the system manage it | Amber | Don't delete it by hand, but there's a safe way to make it smaller, given in the explanation. |
| Safe to clear | Green | Temporary files or a cache that's rebuilt when needed. Clearing what's inside is safe. |
| Your call | Blue | Holds something you may want, like backups or your previous Windows. Use the tool named in the explanation to remove it. |
| Takes no disk space | Grey | Not really on the disk: the kernel's views (`/proc`), mount points, links. Sizes there aren't real. |

When the advice gives a command, it's shown in `this style`. Run it in Terminal, or on Windows in
a terminal opened with **Run as administrator** when it says so.

> **Tip:** the explanations cover the item where the system puts it (for example the page file at
> the root of a drive). A file with the same name somewhere else, like a copy of `pagefile.sys`
> in a backup folder, isn't explained, because it isn't the real thing.

## Everything Filescape explains

<!-- BEGIN GENERATED: do not edit by hand; see src/help/reference.rs -->

### Windows

| Item | What it is | Verdict | What to do |
| --- | --- | --- | --- |
| `C:\pagefile.sys` | **Windows page file.** Virtual memory. Windows moves memory that isn't being used right now into this file, so RAM is free for what you're doing. Its size follows how much RAM you have and what you run. | Let the system manage it | Don't delete it. To make it smaller or move it to another drive: System Properties › Advanced › Performance › Settings › Advanced › Virtual memory (run sysdm.cpl). Letting Windows manage the size is usually best. |
| `C:\hiberfil.sys` | **Hibernation file.** When the PC hibernates, and for Fast Startup, Windows saves what's in memory here so it can pick up where it left off. Often 40% of the size of your RAM. | Let the system manage it | If you never hibernate, run `powercfg /hibernate off` in an administrator terminal: the file goes away (Fast Startup turns off too). `powercfg /h /type reduced` shrinks it and keeps Fast Startup. |
| `C:\swapfile.sys` | **Swap file for Store apps.** Windows uses it to put Store apps to sleep and wake them quickly. Usually 256 MB or less. | Let the system manage it | Don't delete it. It's managed with the page file and takes little space. |
| `C:\DumpStack.log*` | **Crash dump log.** A tiny log written at start-up by the part of Windows that records crash dumps. | Don't delete | Leave it: it's in use while Windows runs and takes almost nothing. |
| `C:\Windows` | **Windows.** Windows itself: the system, its drivers, fonts and settings. | Don't delete | Never delete things here by hand. Use Settings › System › Storage › Temporary files, or Disk Cleanup (cleanmgr) › Clean up system files, which know what's safe. |
| `C:\Windows\WinSxS` | **Windows component store.** Every version of the system components Windows has installed, so updates can be undone and features turned on without the install disc. Most files in it are hard links shared with System32, so it looks bigger than the space it really takes. | Let the system manage it | Never delete files in it. To remove versions updates have replaced: Disk Cleanup › Clean up system files › Windows Update Cleanup, or `Dism.exe /Online /Cleanup-Image /StartComponentCleanup` in an administrator terminal. |
| `C:\Windows\Installer` | **Installer cache.** Copies of the installers (.msi, .msp) of programs and updates, which Windows needs to repair, update or uninstall them. | Don't delete | Don't delete files here: programs may then fail to update or uninstall. Uninstall programs you don't use in Settings › Apps instead. |
| `C:\Windows\SoftwareDistribution` | **Windows Update folder.** Windows Update's working folder: downloaded updates and the update history. | Let the system manage it | Disk Cleanup › Clean up system files › Windows Update Cleanup removes what's no longer needed. Don't delete the folder while updates are installing. |
| `C:\Windows\SoftwareDistribution\Download` | **Downloaded updates.** Updates Windows downloaded. Once they're installed they aren't needed. | Safe to clear | Safe to empty when no update is installing (Windows downloads again anything it still needs). Disk Cleanup or Settings › Storage › Temporary files does it for you. |
| `C:\Windows\Temp` | **System temporary files.** Temporary files from Windows, installers and services. | Safe to clear | Settings › System › Storage › Temporary files clears it. Deleting by hand is fine too; files still in use are skipped. |
| `C:\Windows\Prefetch` | **Prefetch.** Notes on which files programs read when they start, so Windows can load them faster. | Safe to clear | Harmless to clear, but there's little point: it's small, Windows trims it itself, and programs start slower until it's rebuilt. |
| `C:\Windows\System32` | **System programs.** The core of Windows: system programs, libraries and drivers (64-bit, despite the name). | Don't delete | Never delete anything here. |
| `C:\Windows\SysWOW64` | **32-bit system files.** The 32-bit versions of Windows' programs and libraries, used by 32-bit apps. | Don't delete | Never delete anything here. |
| `C:\Windows\System32\DriverStore` | **Driver store.** Every driver package installed on this PC, so devices work again when they're plugged back in. Old graphics drivers pile up here. | Don't delete | Don't delete by hand. Disk Cleanup › Clean up system files › Device driver packages removes old versions; or list them with `pnputil /enum-drivers` and remove one with `pnputil /delete-driver oemNN.inf`. |
| `C:\Windows\Logs` | **Windows logs.** Logs from updates, servicing (CBS) and setup. CBS logs can grow large when something keeps failing. | Safe to clear | Old .log and .cab files are safe to delete. Disk Cleanup › Setup log files does it too. |
| `C:\Windows\LiveKernelReports` | **Hang reports.** Memory dumps Windows writes when a driver stops responding, often the graphics driver. Can reach several GB. | Safe to clear | Safe to delete the .dmp files unless you're troubleshooting the problem. |
| `C:\Windows\Minidump` | **Crash dumps.** Small memory dumps from blue-screen crashes. | Safe to clear | Safe to delete once you're not troubleshooting a crash. Disk Cleanup › System error minidump files. |
| `C:\Windows\MEMORY.DMP` | **Crash dump.** The full memory dump of the last blue-screen crash. Can be as big as your RAM. | Safe to clear | Safe to delete unless someone is investigating the crash. Disk Cleanup › System error memory dump files. |
| `C:\Windows\assembly` | **Global assembly cache.** Shared .NET Framework libraries that programs rely on. | Don't delete | Never delete anything here. |
| `C:\Windows\servicing` | **Servicing stack.** The part of Windows that installs updates and features. | Don't delete | Never delete anything here. |
| `C:\Windows\CSC` | **Offline Files cache.** Copies of network files kept for use offline (the Offline Files feature). | Let the system manage it | Don't delete it by hand. Turn off Offline Files, or delete its copies, in Control Panel › Sync Center. |
| `C:\Windows.old` | **Previous Windows.** Your previous Windows installation, kept after an upgrade so you can go back (for about 10 days). | Your call | When you're happy with the new version: Settings › System › Storage › Temporary files › Previous Windows installation(s). Deleting it by hand usually fails on permissions. |
| `C:\$WINDOWS.~BT` | **Windows upgrade files.** Windows Setup's working folder from an upgrade: downloads and what it needs to roll back. | Safe to clear | Disk Cleanup › Clean up system files › Temporary Windows installation files removes it. |
| `C:\$WINDOWS.~WS` | **Windows upgrade files.** Left behind by the Media Creation Tool or an upgrade. | Safe to clear | Safe to delete once the upgrade is finished. Disk Cleanup › Temporary Windows installation files removes it. |
| `C:\$WinREAgent` | **Recovery update files.** A working folder for updates to the Windows Recovery Environment. | Safe to clear | Usually small, and Windows removes it itself once the update is done. |
| `C:\$SysReset` | **Reset logs.** Logs from a "Reset this PC" that finished or failed. | Safe to clear | Safe to delete unless you're troubleshooting the reset. |
| `C:\$GetCurrent` | **Upgrade logs.** Logs and files from a Windows upgrade. | Safe to clear | Safe to delete once the upgrade is finished. |
| `C:\Recovery` | **Recovery tools.** Files Windows uses to repair or reset itself. | Don't delete | Don't delete it: without it, Reset this PC and start-up repair may not work. |
| `C:\ProgramData` | **Shared program data.** Data and settings that programs keep for all users. Hidden. | Don't delete | Big folders inside belong to specific programs: clean them up through that program, or uninstall it. |
| `C:\ProgramData\Package Cache` | **Installer package cache.** Installers kept by Visual Studio, .NET and other programs, used to repair, update or uninstall them. | Don't delete | Don't delete it: those programs may then fail to update or uninstall. Uninstalling the program removes its packages. |
| `C:\ProgramData\Microsoft\Windows\WER` | **Error reports.** Windows Error Reporting: crash reports waiting to be sent, and ones already sent. | Safe to clear | Safe to clear. Disk Cleanup › Windows error reports, or Settings › Storage › Temporary files. |
| `C:\Program Files` | **Programs.** Installed 64-bit programs. | Don't delete | Remove programs with Settings › Apps › Installed apps, not by deleting their folders. |
| `C:\Program Files (x86)` | **32-bit programs.** Installed 32-bit programs. | Don't delete | Remove programs with Settings › Apps › Installed apps, not by deleting their folders. |
| `C:\PerfLogs` | **Performance logs.** Reports from Performance Monitor. Usually empty. | Safe to clear | Safe to delete what's in it. |
| `C:\Config.Msi` | **Install rollback files.** Windows Installer keeps these while a program installs, to undo a failed install. | Safe to clear | Windows normally removes it. Safe to delete when nothing is installing. |
| `C:\MSOCache` | **Office install cache.** Installation files from an older Microsoft Office, used to repair or add features without the disc. | Your call | Can be deleted if you can download Office again; repairs will then ask for the installer. |
| `C:\Documents and Settings` | **Link to Users.** A link to C:\Users kept for old programs. It holds nothing itself. | Takes no disk space | Leave it: it takes no space. |
| `C:\bootmgr` | **Boot manager.** Starts Windows. | Don't delete | Never delete it: the PC may not start. |
| `C:\BOOTNXT` | **Boot file.** Used while Windows starts. | Don't delete | Never delete it. |
| `C:\inetpub` | **Web server folder.** The folder of Internet Information Services (IIS), Windows' web server. Windows creates it even when IIS is off. | Don't delete | Leave it: recent Windows updates need it, and it's tiny. |
| `%USERPROFILE%\AppData` | **App data.** Settings and data that programs keep for you. Hidden. Local stays on this PC, Roaming can follow you to other PCs, LocalLow is for sandboxed apps. | Don't delete | Big folders inside belong to specific programs: clear their caches in the program, or uninstall it. |
| `%USERPROFILE%\AppData\Local\Temp` | **Your temporary files.** Temporary files from programs and installers. They often stay behind. | Safe to clear | Settings › System › Storage › Temporary files clears it. Deleting by hand is fine too; files in use are skipped. |
| `%USERPROFILE%\AppData\Local\Microsoft\Windows\INetCache` | **Internet cache.** Web pages and files cached by Windows and apps that use its web engine. | Safe to clear | Safe to clear: Settings › Storage › Temporary files › Temporary Internet Files. |
| `%USERPROFILE%\AppData\Local\CrashDumps` | **App crash dumps.** Memory dumps of programs that crashed. | Safe to clear | Safe to delete unless you're sending them to a developer. |
| `%USERPROFILE%\AppData\Local\Microsoft\Windows\WER` | **Error reports.** Your Windows Error Reporting files: crash reports waiting to be sent, and ones already sent. | Safe to clear | Safe to clear. |
| `%USERPROFILE%\AppData\Local\D3DSCache` | **Shader cache.** Compiled graphics shaders, so games and apps start faster. | Safe to clear | Safe to clear (Settings › Storage › Temporary files › DirectX Shader Cache); it's rebuilt as needed. |
| `%USERPROFILE%\AppData\Local\Packages` | **Store app data.** Settings and data of apps from the Microsoft Store, one folder per app. | Don't delete | Don't delete by hand. Settings › Apps › the app › Advanced options › Reset clears an app's data. |
| `%USERPROFILE%\AppData\Local\Microsoft\WindowsApps` | **App shortcuts.** Tiny launchers for Store apps (winget, python and others), so they can be started by name. | Takes no disk space | Leave it: these take next to no space. |
| `%USERPROFILE%\NTUSER.DAT*` | **Your registry.** Your part of the Windows registry: your settings for Windows and programs, and its logs. | Don't delete | Never delete it: your account would stop working. |
| `%USERPROFILE%\ntuser.ini` | **Profile settings.** Part of your Windows profile. | Don't delete | Leave it: it's tiny. |

### macOS

| Item | What it is | Verdict | What to do |
| --- | --- | --- | --- |
| `/System` | **macOS.** macOS itself, on a sealed, read-only system volume. | Don't delete | Nothing here can or should be deleted. Space on the system volume isn't yours to reclaim. |
| `/System/Volumes/Data` | **Data volume.** The volume with everything you and your apps write. /Users, /Applications and others are shown both here and in their usual places, so tools that follow both count them twice. Time Machine's local snapshots also live on this volume and appear in no folder. | Don't delete | Clean up the folders inside rather than this one. `tmutil listlocalsnapshots /` lists local snapshots; macOS removes them when space runs low. |
| `/System/Volumes/VM` | **Virtual memory volume.** Holds the swap files macOS writes when memory is short, and the sleep image. | Let the system manage it | macOS manages it. Quitting memory-hungry apps or restarting shrinks swap. |
| `/System/Volumes/Preboot` | **Preboot volume.** Files macOS needs to start up and unlock FileVault. | Don't delete | Never delete anything here. |
| `/System/Volumes/Update` | **Update volume.** Used while macOS installs updates. | Don't delete | macOS clears it itself. |
| `/private/var/vm` | **Virtual memory.** Swap files macOS writes when memory is short, and the sleep image (older macOS; newer versions use the VM volume). | Let the system manage it | macOS manages it. Quitting memory-hungry apps or restarting shrinks swap. |
| `sleepimage` | **Sleep image.** When the Mac sleeps, it can save memory here so nothing is lost if the battery runs out. As big as your RAM. | Let the system manage it | Leave it on laptops. On a desktop Mac, `sudo pmset -a hibernatemode 0` stops writing it, and the file can then be deleted. |
| `vm/swapfile*` | **Swap file.** Memory macOS moved to disk because RAM was full. | Let the system manage it | macOS deletes swap files itself. Quit memory-hungry apps or restart to free them. |
| `/private/var/folders` | **Temporary and cache folders.** Each user's private temporary files and caches (where $TMPDIR points). Can grow to several GB. | Let the system manage it | Don't delete it by hand while you're logged in: running apps can break. A restart clears its temporary part. |
| `/private/var/log` | **System logs.** Logs from macOS and services. | Let the system manage it | macOS rotates and trims them. Old .gz files are safe to delete. |
| `/private/var/db/diagnostics` | **Unified logs.** The system log that Console shows. | Let the system manage it | macOS trims it itself. Leave it. |
| `/private/var/db/uuidtext` | **Log support files.** Used to read the unified system log. | Let the system manage it | macOS trims it itself. Leave it. |
| `/private/var/db/oah` | **Rosetta cache.** Intel apps translated by Rosetta 2 to run on Apple silicon. | Let the system manage it | Leave it: macOS rebuilds it, and Intel apps start slower until then. |
| `/private/var/db` | **System databases.** Databases macOS keeps about the system: logins, receipts, Spotlight settings and more. | Don't delete | Don't delete anything here by hand. |
| `/private/tmp` | **Temporary files.** Temporary files. Cleared when the Mac restarts. | Safe to clear | Restart to clear it. Don't delete files apps are using. |
| `/cores` | **Core dumps.** Memory dumps of programs that crashed, written only when core dumps are turned on. | Safe to clear | Safe to delete. |
| `/Volumes` | **Other drives.** Where other drives, disk images and network shares appear, plus a link to the startup disk. | Takes no disk space | Takes no space here: each item is another drive. |
| `/dev` | **Devices.** The kernel's view of disks and other devices. Not files on the disk. | Takes no disk space | Takes no space. |
| `/Library` | **Shared library.** Settings and support files of apps, for all users. | Don't delete | Big folders inside belong to specific apps: clean up through the app, or uninstall it. |
| `/Library/Caches` | **Shared caches.** Caches apps keep for all users. Apps rebuild them as needed. | Safe to clear | Safe to clear what's inside, best after quitting apps. |
| `/Library/Updates` | **Downloaded updates.** macOS updates downloaded and waiting to install. | Let the system manage it | Install or skip the update in System Settings › General › Software Update; macOS removes the download. |
| `/Applications` | **Applications.** Your apps. | Your call | Delete an app by moving it to the Trash. Its settings stay in ~/Library until you remove them too. |
| `/usr` | **Unix programs.** Command-line programs and libraries that are part of macOS (on the sealed system volume). | Don't delete | Nothing here can or should be deleted, except /usr/local, which holds things you installed. |
| `/opt/homebrew` | **Homebrew.** Programs installed with Homebrew (on Apple silicon). | Your call | `brew autoremove` removes packages nothing needs; `brew cleanup` removes old versions. |
| `~/Library` | **Your library.** Your app settings and data: mail, messages, caches, app support files. Hidden by Finder. | Don't delete | Big folders inside belong to specific apps: clean up through the app, or uninstall it. |
| `~/Library/Caches` | **Your caches.** Caches apps keep for you. Apps rebuild them as needed. | Safe to clear | Safe to clear what's inside, best after quitting apps. |
| `~/Library/Caches/Homebrew` | **Homebrew downloads.** Downloads Homebrew kept after installing. | Safe to clear | `brew cleanup --prune=all` clears it. |
| `~/Library/Logs` | **Your logs.** Logs and crash reports from your apps. | Safe to clear | Safe to delete. |
| `~/Library/Application Support` | **App support files.** Data apps keep for you: libraries, databases, downloads, plug-ins. | Don't delete | Big folders inside belong to specific apps. Remove them only for apps you've uninstalled. |
| `~/Library/Application Support/MobileSync/Backup` | **iPhone and iPad backups.** Backups of your iPhone or iPad made by Finder or iTunes. Each one can take tens of GB. | Your call | Delete old ones in Finder: select the device in the sidebar › Manage Backups. |
| `~/Library/Containers` | **App containers.** Data of sandboxed apps, one folder per app. | Don't delete | Don't delete by hand: clear an app's data in the app, or uninstall it. |
| `~/Library/Containers/com.docker.docker` | **Docker Desktop.** Docker Desktop's virtual disk: images, containers and volumes. | Your call | `docker system prune` removes what nothing uses. Docker Desktop › Settings › Resources sets the disk limit. |
| `~/Library/Group Containers` | **Shared app containers.** Data shared between related apps (Office, Messages and others). | Don't delete | Don't delete by hand. |
| `~/Library/Mail` | **Mail.** Apple Mail's messages and attachments. | Don't delete | Delete mail in the Mail app, or remove accounts you no longer use. |
| `~/Library/Messages/Attachments` | **Messages attachments.** Photos, videos and files sent and received in Messages. | Your call | Delete big attachments in System Settings › General › Storage › Messages, or set Keep messages to 1 year in Messages › Settings. |
| `~/Library/CloudStorage` | **Cloud folders.** Folders of cloud services (Google Drive, OneDrive, Dropbox…). | Don't delete | Free up space through the cloud app, or with Filescape's Free up space on files you don't need locally. |
| `~/Library/Mobile Documents` | **iCloud Drive.** Your iCloud Drive folder on this Mac. | Don't delete | Turn on Optimize Mac Storage in iCloud settings, or remove downloads (Filescape's Free up space). |
| `~/Library/Developer/Xcode/DerivedData` | **Xcode build files.** Build products and indexes Xcode makes for each project. | Safe to clear | Safe to delete; Xcode rebuilds what it needs (the next build is slower). |
| `~/Library/Developer/Xcode/iOS DeviceSupport` | **iOS device symbols.** Debug symbols for each iOS version of the devices you've connected to Xcode. | Safe to clear | Safe to delete versions you no longer debug on; Xcode fetches them again when needed. |
| `~/Library/Developer/Xcode/Archives` | **Xcode archives.** Builds you archived for release. Needed to read crash reports from those versions. | Your call | Delete old ones in Xcode › Window › Organizer. |
| `~/Library/Developer/CoreSimulator` | **iOS simulators.** Simulated devices and their data. | Your call | `xcrun simctl delete unavailable` removes simulators for runtimes you no longer have; remove runtimes in Xcode › Settings › Platforms. |
| `~/.Trash` | **Your Trash.** Things you moved to the Trash. | Safe to clear | Empty the Trash in Finder. |
| `*.photoslibrary` | **Photos library.** The Photos app's library: your photos and videos with their edits and database. | Don't delete | Never delete or move files inside it. Delete photos in Photos, or with iCloud Photos turn on Optimize Mac Storage. |

### Linux

| Item | What it is | Verdict | What to do |
| --- | --- | --- | --- |
| `/proc` | **Processes (virtual).** The kernel's live view of processes and system state. Not files on the disk. | Takes no disk space | Takes no space. Sizes shown here aren't real. |
| `/proc/kcore` | **Kernel memory (virtual).** A view of the computer's memory. It looks enormous but isn't stored anywhere. | Takes no disk space | Takes no space. Ignore its size. |
| `/sys` | **Devices and drivers (virtual).** The kernel's view of devices, drivers and settings. Not files on the disk. | Takes no disk space | Takes no space. |
| `/dev` | **Devices.** Disks, terminals and other devices as files. Kept in memory. | Takes no disk space | Takes no space on the disk. |
| `/run` | **Runtime files.** Information about running services, kept in memory and cleared at start-up. | Takes no disk space | Takes no disk space. |
| `/tmp` | **Temporary files.** Temporary files. Often kept in memory and cleared at start-up. | Safe to clear | Restarting clears it. Don't delete files programs are using. |
| `/var/tmp` | **Longer-lived temporary files.** Temporary files that survive a restart. systemd usually removes ones older than 30 days. | Safe to clear | Files nothing is using are safe to delete. |
| `/var/log` | **System logs.** Logs from the system and services. logrotate rotates and compresses them. | Let the system manage it | Old rotated files (.1, .gz) are safe to delete. Empty a big log with `sudo truncate -s 0 FILE` rather than deleting it. |
| `/var/log/journal` | **systemd journal.** The systemd log. Can grow to several GB. | Let the system manage it | `sudo journalctl --vacuum-size=200M` shrinks it. Set SystemMaxUse= in /etc/systemd/journald.conf to keep it small. |
| `/var/cache` | **System caches.** Caches of system tools and package managers. | Safe to clear | Use each package manager's own clean command (`sudo apt clean`, `sudo dnf clean all`, `sudo paccache -rk2`) rather than deleting by hand. |
| `/var/cache/apt/archives` | **Downloaded packages (apt).** Packages apt downloaded to install or update software. | Safe to clear | `sudo apt clean` clears it. |
| `/var/cache/pacman/pkg` | **Downloaded packages (pacman).** Every package version pacman downloaded. | Safe to clear | `sudo paccache -rk2` keeps the last two versions of each; `sudo pacman -Sc` keeps only installed ones. |
| `/var/cache/dnf` | **Downloaded packages (dnf).** Package metadata and downloads from dnf. | Safe to clear | `sudo dnf clean all` clears it. |
| `/var/lib/apt/lists` | **Package lists.** The lists of available packages apt downloads. | Safe to clear | Safe to delete; `sudo apt update` fetches them again. |
| `/var/lib/snapd` | **Snap packages.** Snap packages and their older versions (several are kept per app). | Let the system manage it | Keep fewer old versions: `sudo snap set system refresh.retain=2`. `snap list --all` shows disabled ones; remove one with `sudo snap remove NAME --revision=N`. |
| `/snap` | **Snap apps (mounted).** Each snap app mounted from a compressed file in /var/lib/snapd/snaps. Sizes here count the unpacked contents, which aren't on the disk like that. | Takes no disk space | Remove apps with `sudo snap remove NAME`. The real space is in /var/lib/snapd. |
| `~/snap` | **Snap app data.** Your data for snap apps, one folder per app. | Your call | Remove an app's folder only after uninstalling the app. |
| `/var/lib/flatpak` | **Flatpak apps.** Flatpak apps and the runtimes they need. | Let the system manage it | `flatpak uninstall --unused` removes runtimes no app uses. |
| `~/.local/share/flatpak` | **Your Flatpak apps.** Flatpak apps installed for you only, and their runtimes. | Let the system manage it | `flatpak uninstall --user --unused` removes runtimes no app uses. |
| `~/.var/app` | **Flatpak app data.** Your data and caches for Flatpak apps, one folder per app. | Your call | Each app's cache folder is safe to clear; remove an app's folder only after uninstalling it. |
| `/var/lib/docker` | **Docker.** Docker images, containers, volumes and build cache. | Your call | `docker system prune` removes stopped containers and unused data; add `-a` for unused images. Volumes hold data: prune them only on purpose. |
| `/var/lib/containers` | **Podman containers.** Podman images and containers. | Your call | `podman system prune` removes what nothing uses. |
| `~/.local/share/containers` | **Your Podman containers.** Rootless Podman images and containers. | Your call | `podman system prune` removes what nothing uses. |
| `/var/lib/libvirt/images` | **Virtual machine disks.** Disks of virtual machines made with virt-manager or libvirt. | Your call | Delete a machine and its storage in virt-manager. |
| `/var/lib/systemd/coredump` | **Core dumps.** Memory dumps of programs that crashed. systemd caps their size. | Safe to clear | Safe to delete. `coredumpctl list` shows what they're from. |
| `/var/crash` | **Crash reports.** Crash reports, mostly from Ubuntu's apport. | Safe to clear | Safe to delete. |
| `/var/lib` | **Program state.** Databases and state kept by system services and package managers. | Don't delete | Don't delete by hand: big folders belong to specific services (docker, snapd, databases). |
| `/var/spool` | **Queues.** Print jobs, mail and scheduled jobs waiting to be handled. | Don't delete | Leave it. |
| `/swapfile` | **Swap file.** Swap space: memory the system moved to disk. | Let the system manage it | To shrink it: `sudo swapoff /swapfile`, `sudo fallocate -l 2G /swapfile`, `sudo mkswap /swapfile`, `sudo swapon /swapfile`. To remove it, also take its line out of /etc/fstab. |
| `/swap.img` | **Swap file.** Swap space (Ubuntu's name for it): memory the system moved to disk. | Let the system manage it | To shrink it: `sudo swapoff /swap.img`, `sudo fallocate -l 2G /swap.img`, `sudo mkswap /swap.img`, `sudo swapon /swap.img`. To remove it, also take its line out of /etc/fstab. |
| `/boot` | **Boot files.** The kernels and boot loader the system starts from. | Don't delete | If it's full, remove old kernels with the package manager: `sudo apt autoremove --purge` on Ubuntu and Debian. |
| `/usr/lib/modules` | **Kernel modules.** Drivers for each installed kernel version. | Let the system manage it | Old kernels and their modules go with `sudo apt autoremove --purge` (Ubuntu, Debian) or your package manager. |
| `/lib/modules` | **Kernel modules.** Drivers for each installed kernel version. | Let the system manage it | Old kernels and their modules go with `sudo apt autoremove --purge` (Ubuntu, Debian) or your package manager. |
| `/usr` | **Installed software.** Programs, libraries and data installed by the package manager. | Don't delete | Remove software with the package manager, never by deleting files here. |
| `/etc` | **System settings.** Configuration of the system and services. | Don't delete | Never delete things here by hand. |
| `/bin` | **Programs.** Essential programs. On most current systems a link to /usr/bin. | Don't delete | Never delete it. |
| `/sbin` | **System programs.** Programs for administrators. On most current systems a link to /usr/sbin. | Don't delete | Never delete it. |
| `/lib` | **Libraries.** Essential libraries. On most current systems a link to /usr/lib. | Don't delete | Never delete it. |
| `/lib64` | **64-bit libraries.** Essential 64-bit libraries. On most current systems a link to /usr/lib64 or /usr/lib. | Don't delete | Never delete it. |
| `/opt` | **Add-on software.** Software installed outside the package manager (Chrome, vendor tools…). | Your call | Uninstall programs the way they were installed. |
| `/mnt` | **Mount points.** Where other drives and shares are mounted by hand. | Takes no disk space | Takes no space here: each item is another drive. |
| `/media` | **Removable drives.** Where USB drives and discs appear. | Takes no disk space | Takes no space here: each item is another drive. |
| `/var` | **Variable data.** Data that changes while the system runs: logs, caches, databases, spools, containers. | Don't delete | Look inside for what's big: logs, caches and package downloads can be cleaned safely, the rest belongs to services. |
| `/home` | **Home folders.** Each user's own files and settings. | Don't delete | Your files are yours to sort out; leave other users' folders alone. |
| `/srv` | **Served data.** Data served by this computer, such as websites or FTP files. Often empty. | Your call | Belongs to whatever service you set up here. |
| `/nix` | **Nix store.** Packages installed with the Nix package manager, including old generations. | Let the system manage it | `nix-collect-garbage -d` removes old generations and packages nothing uses. |
| `/root` | **Administrator's home.** The root user's home folder. | Don't delete | Leave it unless you know what's in it. |
| `/.snapshots` | **Snapper snapshots.** Btrfs snapshots made by Snapper, often before every update. Snapshots share data, so their sizes add up to more than they really use. | Your call | `sudo snapper list` and `sudo snapper delete N`; set limits in /etc/snapper/configs/root. |
| `/timeshift` | **Timeshift snapshots.** System snapshots made by Timeshift. | Your call | Delete old snapshots in Timeshift. |
| `~/.cache` | **Your caches.** Caches of your apps: browsers, thumbnails, pip, fonts and more. | Safe to clear | Safe to clear, best after closing apps; they rebuild what they need. |
| `~/.cache/thumbnails` | **Thumbnails.** Picture previews made by the file manager. | Safe to clear | Safe to clear; they're made again when needed. |
| `~/.local/share/Trash` | **Your Trash.** Things you moved to the Trash. | Safe to clear | Empty the Trash in your file manager. |
| `~/.local/share/baloo` | **KDE file index.** KDE's search index of your files. | Let the system manage it | `balooctl purge` rebuilds it; `balooctl disable` turns indexing off. |
| `~/.xsession-errors*` | **Session log.** Messages from programs in your desktop session. Can grow large. | Safe to clear | Safe to delete; it's recreated when you log in. |

### On any system (removable drives, projects)

| Item | What it is | Verdict | What to do |
| --- | --- | --- | --- |
| `$Recycle.Bin` | **Recycle Bin.** Things deleted on this drive in Windows, one folder per user. | Safe to clear | Empty the Recycle Bin from the desktop, or Settings › System › Storage › Temporary files. Don't delete the folder itself. |
| `System Volume Information` | **System Volume Information.** Windows' data for this drive: System Restore points, shadow copies (Previous Versions) and the search index. Can grow to many GB. | Let the system manage it | Windows won't let you delete it. Shrink restore points in System Protection (SystemPropertiesProtection.exe) › Configure › Max Usage. `vssadmin list shadowstorage` shows the space. |
| `Thumbs.db` | **Thumbnail cache.** Picture previews older Windows (and Windows on network folders) saves in a folder. | Safe to clear | Safe to delete; it's made again when needed. |
| `ehthumbs*.db` | **Media thumbnails.** Thumbnails from Windows Media Center. | Safe to clear | Safe to delete. |
| `desktop.ini` | **Folder settings.** Tiny Windows file that sets a folder's icon or the name Explorer shows for it (Documents, Pictures…). | Don't delete | Leave it: deleting it only resets how the folder looks, and it takes no real space. |
| `.DS_Store` | **Finder view settings.** macOS Finder's settings for a folder: icon positions, view and sort. | Safe to clear | Harmless to delete; Finder makes it again. To stop Macs writing them on network shares: `defaults write com.apple.desktopservices DSDontWriteNetworkStores true`. |
| `._*` | **Mac file extras.** A Mac saved extra information (tags, icons, resource forks) next to a file on a drive that can't store it, like FAT, exFAT or some network shares. | Safe to clear | Safe to delete on drives only Windows or Linux use; a Mac then loses those extras. On a Mac, `dot_clean` merges them back. |
| `__MACOSX` | **Mac zip extras.** Mac file extras that came out of a zip made on a Mac. | Safe to clear | Safe to delete. |
| `.Spotlight-V100` | **Spotlight index.** macOS's search index for this drive. | Let the system manage it | On a Mac, add the drive to System Settings › Spotlight › Search Privacy to stop indexing it, or rebuild the index with `sudo mdutil -E /Volumes/NAME`. Deleting it on another system is harmless; the Mac rebuilds it. |
| `.fseventsd` | **File change log.** macOS's record of changes on this drive, used by Time Machine and Spotlight. | Don't delete | Leave it. On an external drive deleting it is harmless, but the next Time Machine backup may take longer. |
| `.DocumentRevisions-V100` | **Document versions.** Saved versions of documents for File › Revert To on a Mac. | Don't delete | Don't delete it by hand. Versions of a document go away when the document is deleted. |
| `.MobileBackups*` | **Local Time Machine snapshots.** Time Machine's local snapshots on older macOS. | Let the system manage it | macOS removes them when space runs low; `tmutil` manages them. |
| `Backups.backupdb` | **Time Machine backups.** Time Machine backups on this drive. | Your call | Delete old backups from Time Machine itself (or `tmutil delete`), never in Finder or here. |
| `.Trashes` | **Trash for this drive.** Things a Mac moved to the Trash from this drive. | Safe to clear | Empty the Trash while the drive is connected to the Mac. |
| `.TemporaryItems` | **Mac temporary items.** Temporary files a Mac left on this drive. | Safe to clear | Safe to delete when no Mac is using the drive. |
| `.Trash-*` | **Linux Trash for this drive.** Things moved to the Trash on Linux from this drive. | Safe to clear | Empty the Trash, or delete it while nothing is using the drive. |
| `lost+found` | **Recovered fragments.** Where a file system check puts pieces of files it found without a name. Normally empty. | Don't delete | Look inside for anything you lost after a crash; delete what's there if not. Keep the folder itself. |
| `found.0*` | **Recovered fragments.** Pieces of files Windows' disk check (chkdsk) recovered. | Your call | Look for anything you need (they're named FILE0000.CHK); otherwise safe to delete. |
| `node_modules` | **JavaScript packages.** Packages a JavaScript project uses. Often huge and full of tiny files. | Safe to clear | Safe to delete for projects you aren't working on; `npm install` brings it back. |
| `__pycache__` | **Python bytecode cache.** Compiled Python files, so scripts start faster. | Safe to clear | Safe to delete; Python makes them again. |
| `.git` | **Git repository.** The whole history of a project under Git. | Don't delete | Don't delete it unless you mean to lose the history. `git gc` packs it tighter. |
| `~/.npm` | **npm cache.** Packages npm downloaded. | Safe to clear | `npm cache clean --force` clears it. |
| `~/.cargo/registry` | **Rust package cache.** Crates Cargo downloaded. | Safe to clear | Safe to delete; Cargo downloads what a build needs again. |
| `~/.gradle/caches` | **Gradle cache.** Downloaded libraries and build caches of Gradle. | Safe to clear | Safe to delete while no build is running. |
| `~/.m2/repository` | **Maven repository.** Libraries Maven downloaded. | Safe to clear | Safe to delete; Maven downloads them again. |
| `~/.nuget/packages` | **NuGet packages.** Packages .NET projects downloaded. | Safe to clear | `dotnet nuget locals all --clear` clears it. |

<!-- END GENERATED -->
