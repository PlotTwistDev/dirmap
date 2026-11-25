# Command line

Filescape can be started from a terminal (Command Prompt, PowerShell, Terminal) with options. On
Windows, run `filescape.exe`; elsewhere `filescape` (or the full path to it).

```text
Filescape [PATH]                                  open Filescape and scan PATH
filescape --cli [--top N] [--logical | --cloud-only | --reclaimable] [--save-snapshot FILE] [--all-fs] [--cloud-mounts] PATH
filescape --cli --free-up PATH
filescape --cli --keep PATH
filescape --cli --models PATH
                                               print a text report instead of opening a window
filescape --compare FILE [PATH]                   open Filescape comparing with a saved snapshot
filescape --help                                  show the usage line
```

## Options

| Option | Effect |
|---|---|
| `PATH` | The drive or folder to scan. In the window, scanning starts straight away. Without it, Filescape opens on the drive list. For `--cli` the default is the current folder. |
| `--cli` | Don't open a window: scan, print a report and exit. |
| `--top N` | With `--cli`: how many rows to print in each part of the report (default 15). |
| `--logical` | With `--cli`: rank and total by logical size instead of size on disk. See [Size on disk and logical size](../disk-usage/sizes.md#size-on-disk-and-logical-size). |
| `--cloud-only` | With `--cli`: rank and total by data that is in the cloud and not on this computer. See [Cloud files](../disk-usage/cloud-files.md). |
| `--reclaimable` | With `--cli`: rank and total by the space freeing up cloud files would give back. |
| `--free-up` | With `--cli`: ask your cloud app to free up the cloud files in PATH (a file or a folder), as **Free Up Space** does. Prints what happened; exits with 1 if any file couldn't be freed up, 2 on systems without a supported way, 3 without a Filescape Professional licence. |
| `--keep` | With `--cli`: keep the cloud files in PATH on this computer, downloading what isn't here, as **Always Keep on This Device** (Windows) or **Download Now** (macOS) does. Needs Filescape Professional, like `--free-up`. |
| `--models` | With `--cli`: list the AI models in PATH instead of the usual report: each model's size, name, details and where it's from, its files, and which are copies or unused Ollama downloads. See [AI models](../views/ai-models.md). |
| `--cloud-mounts` | Also walk into cloud drives mounted inside the folder (rclone, FUSE), which are left out by default. See [Mounted cloud drives](../disk-usage/cloud-files.md#mounted-cloud-drives). |
| `--save-snapshot FILE` | With `--cli`: also save the scan as a snapshot file for comparing later. |
| `--compare FILE` | Open the window with this snapshot loaded, so the scan is compared with it. See [Comparing with a snapshot](../analysis/snapshots.md). |
| `--all-fs` | Include other drives mounted inside the scanned folder (Linux and macOS), for this run only. Overrides **Stay on the scanned drive**. |
| `-h`, `--help` | Print the usage line and exit. |

## The text report

```text
$ filescape --cli --top 5 ~/Projects
/home/me/Projects: 182.4 GB on disk, 184.0 GB logical, in 1204311 files, 98770 folders (14.21s, 0 unreadable)
Sizes below: size on disk (bytes are 182.4 GB)

Largest entries:
      82.1 GB   45.0%  renders/
      40.3 GB   22.1%  sim_cache/
      ...

By extension:
      90.2 GB   49.5%    412000 files  .exr
      ...
```

The first line gives the total size on disk and logical size, number of files and folders, how
long the scan took and how many items couldn't be read. When the folder holds cloud files, a
`Cloud:` line names the services and says how many files are downloaded, online only (and how much
isn't downloaded), partly downloaded or unknown. The rest of the report uses sizes on disk, or
logical sizes with `--logical`. **Largest entries** lists what is directly inside the scanned folder,
largest first, with folders marked `/`. **By extension** lists the extensions using the most
space. The scan exclusions from the Settings window are not applied on the command line.

## Examples

Scan a drive in the window:

```sh
filescape D:\
```

Save a snapshot every night (for example from Task Scheduler or cron), then compare in the
morning:

```sh
filescape --cli --save-snapshot D:\Snapshots\projects.dmsnap D:\Projects
filescape --compare D:\Snapshots\projects.dmsnap D:\Projects
```

The 20 biggest items in your home folder, without opening a window:

```sh
filescape --cli --top 20 ~
```

## Environment variables

| Variable | Effect |
|---|---|
| `DIRMAP_RENDERER=glow` | Draw with OpenGL. This is already the default on Windows. Elsewhere, try it if the window is blank or garbled. |
| `DIRMAP_RENDERER=wgpu` | Draw with wgpu (DirectX 12 on Windows, Vulkan or Metal elsewhere). This is the default on Linux and macOS. On Windows the panels can shake while you resize the window. |
| `TERMINAL` | Linux: the terminal program used by **Open in Terminal…**. |

## Related topics

- [Comparing with a snapshot](../analysis/snapshots.md)
- [Scanning a drive or folder](../getting-started/scanning.md)
- [Troubleshooting](../troubleshooting/index.md)
