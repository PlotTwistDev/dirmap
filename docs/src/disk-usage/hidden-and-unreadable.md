# Hidden, system and unreadable files

## Hidden and system files

Filescape **includes hidden files by default**, because they use space like any other file. To
leave them out, turn on **⚙ Settings > Advanced > Skip hidden files and folders**, then rescan.

What counts as hidden:

- **Windows:** files and folders with the *Hidden* or *System* attribute (for example
  `pagefile.sys`, `hiberfil.sys`, `$Recycle.Bin` and `System Volume Information`).
- **Linux and macOS:** names starting with a dot, such as `.cache` or `.git`.

Skipping hidden files makes the totals smaller than the space really used, so it's best left off
when you're hunting for lost space.

## Unreadable folders and files

Some folders can't be read by ordinary programs: other users' profiles, parts of Windows,
folders whose permissions exclude you, and on macOS other apps' data unless Filescape has Full Disk
Access. Filescape can't see inside them, so their contents are **missing from the totals**.

- While scanning, the progress line shows **N unreadable**.
- When the scan finishes, the status bar says **N items couldn't be read**.
- The command-line report prints the count too.

Each folder that couldn't be opened counts once, however much is inside it.

## Getting access

| System | What to do |
|---|---|
| **Windows** | Run Filescape as administrator: **⚙ Settings > Advanced > Permissions > Restart now**, or answer **Yes** when Filescape asks at start. See [Running as administrator](../settings/administrator.md). |
| **macOS** | Give Filescape **Full Disk Access**, then quit and reopen it. See [Folder access on macOS](#folder-access-on-macos). |
| **Linux** | Start Filescape from a terminal with `sudo`, for example `sudo Filescape /`. |

Even as administrator, a few Windows folders may stay closed, and files in use by Windows are
still listed with their sizes because sizes come from the folder listing.

## Folder access on macOS

macOS asks before an app can read certain folders. The first time Filescape scans your Desktop,
Documents, Downloads, iCloud Drive, removable drives or network volumes, you'll see messages
such as **"Filescape" would like to access files in your Documents folder**. Click **Allow**. If
you click **Don't Allow**, that folder counts as unreadable; you can change your mind under
**System Settings > Privacy & Security > Files & Folders > Filescape**.

Even with every folder allowed, macOS keeps other apps' data (Mail, Messages, Safari, some of
`~/Library`) closed unless the app has **Full Disk Access**. For a complete picture of your disk,
and no more per-folder messages, give Filescape Full Disk Access:

1. Open **System Settings > Privacy & Security > Full Disk Access**.
2. If Filescape is in the list, turn it on. If it isn't, click **+**, choose **Filescape** in your
   Applications folder and click **Open**, then make sure it's turned on.
3. Enter your password or use Touch ID if asked.
4. Quit Filescape and open it again (macOS may offer **Quit & Reopen**), then scan again.

When a scan finishes with unreadable items and Filescape doesn't have Full Disk Access, the
status bar shows **Allow Full Disk Access…**, which opens the right page of System Settings. It's
also under **⚙ Settings > Advanced**. Running Filescape with `sudo` doesn't get around these
protections.

## Related topics

- [Running as administrator](../settings/administrator.md)
- [Troubleshooting](../troubleshooting/index.md)
- [Windows system storage](../advanced/windows-storage.md)
