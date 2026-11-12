# Hidden, system and unreadable files

## Hidden and system files

dirmap **includes hidden files by default**, because they use space like any other file. To
leave them out, turn on **⚙ Settings > Advanced > Skip hidden files and folders**, then rescan.

What counts as hidden:

- **Windows:** files and folders with the *Hidden* or *System* attribute (for example
  `pagefile.sys`, `hiberfil.sys`, `$Recycle.Bin` and `System Volume Information`).
- **Linux and macOS:** names starting with a dot, such as `.cache` or `.git`.

Skipping hidden files makes the totals smaller than the space really used, so it's best left off
when you're hunting for lost space.

## Unreadable folders and files

Some folders can't be read by ordinary programs: other users' profiles, parts of Windows,
folders whose permissions exclude you, and on macOS other apps' data unless dirmap has Full Disk
Access. dirmap can't see inside them, so their contents are **missing from the totals**.

- While scanning, the progress line shows **N unreadable**.
- When the scan finishes, the status bar says **N items couldn't be read**.
- The command-line report prints the count too.

Each folder that couldn't be opened counts once, however much is inside it.

## Getting access

| System | What to do |
|---|---|
| **Windows** | Run dirmap as administrator: **⚙ Settings > Advanced > 🛡 Restart as administrator now**, or answer **Yes** when dirmap asks at start. See [Running as administrator](../settings/administrator.md). |
| **macOS** | Give dirmap **Full Disk Access** in **System Settings > Privacy & Security**, then restart it. |
| **Linux** | Start dirmap from a terminal with `sudo`, for example `sudo dirmap /`. |

Even as administrator, a few Windows folders may stay closed, and files in use by Windows are
still listed with their sizes because sizes come from the folder listing.

## Related topics

- [Running as administrator](../settings/administrator.md)
- [Troubleshooting](../troubleshooting/index.md)
- [Windows system storage](../advanced/windows-storage.md)
