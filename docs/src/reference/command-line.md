# Command line

dirmap can be started from a terminal (Command Prompt, PowerShell, Terminal) with options. On
Windows, run `dirmap.exe`; elsewhere `dirmap` (or the full path to it).

```text
dirmap [PATH]                                  open dirmap and scan PATH
dirmap --cli [--top N] [--save-snapshot FILE] [--all-fs] PATH
                                               print a text report instead of opening a window
dirmap --compare FILE [PATH]                   open dirmap comparing with a saved snapshot
dirmap --help                                  show the usage line
```

## Options

| Option | Effect |
|---|---|
| `PATH` | The drive or folder to scan. In the window, scanning starts straight away. Without it, dirmap opens on the drive list. For `--cli` the default is the current folder. |
| `--cli` | Don't open a window: scan, print a report and exit. |
| `--top N` | With `--cli`: how many rows to print in each part of the report (default 15). |
| `--save-snapshot FILE` | With `--cli`: also save the scan as a snapshot file for comparing later. |
| `--compare FILE` | Open the window with this snapshot loaded, so the scan is compared with it. See [Comparing with a snapshot](../analysis/snapshots.md). |
| `--all-fs` | Include other drives mounted inside the scanned folder (Linux and macOS), for this run only. Overrides **Stay on the scanned drive**. |
| `-h`, `--help` | Print the usage line and exit. |

## The text report

```text
$ dirmap --cli --top 5 ~/Projects
/home/me/Projects: 182.4 GB in 1204311 files, 98770 folders (14.21s, 0 unreadable)

Largest entries:
      82.1 GB   45.0%  renders/
      40.3 GB   22.1%  sim_cache/
      ...

By extension:
      90.2 GB   49.5%    412000 files  .exr
      ...
```

The first line gives the total size, number of files and folders, how long the scan took and how
many items couldn't be read. **Largest entries** lists what is directly inside the scanned folder,
largest first, with folders marked `/`. **By extension** lists the extensions using the most
space. The scan exclusions from the Settings window are not applied on the command line.

## Examples

Scan a drive in the window:

```sh
dirmap D:\
```

Save a snapshot every night (for example from Task Scheduler or cron), then compare in the
morning:

```sh
dirmap --cli --save-snapshot D:\Snapshots\projects.dmsnap D:\Projects
dirmap --compare D:\Snapshots\projects.dmsnap D:\Projects
```

The 20 biggest items in your home folder, without opening a window:

```sh
dirmap --cli --top 20 ~
```

## Environment variables

| Variable | Effect |
|---|---|
| `DIRMAP_RENDERER=glow` | Draw with OpenGL instead of the default GPU renderer. Try it if the window is blank or garbled. |
| `TERMINAL` | Linux: the terminal program used by **Open in Terminal…**. |

## Related topics

- [Comparing with a snapshot](../analysis/snapshots.md)
- [Scanning a drive or folder](../getting-started/scanning.md)
- [Troubleshooting](../troubleshooting/index.md)
