# Running as administrator

## Why it matters

Windows keeps some folders closed to ordinary programs: other users' profiles, parts of Windows
itself, and folders whose permissions have been restricted. Without administrator rights Filescape
can't look inside them, so their contents are counted as **unreadable** and left out of the
totals. On a system drive this can be many gigabytes.

Running as administrator lets Filescape read almost everything. It doesn't change what Filescape does:
it still only reads, unless you use a right-click command such as Delete. Keep in mind that, as
administrator, those commands can also delete files Windows would otherwise protect.

## The question at start

The first time Filescape starts on Windows without administrator rights, it asks:

| Answer | Effect |
|---|---|
| **🛡 Yes, always** | Restarts as administrator now and every time from now on. |
| **Just this once** | Restarts as administrator now; asks again next time. |
| **No, don't ask again** | Stays as it is and stops asking. |

Restarting shows Windows' usual permission prompt (User Account Control). If you decline it,
Filescape says **Windows didn't start Filescape as administrator, so it's carrying on as it is**, and
keeps running normally.

## Changing it later

**⚙ Settings > Advanced > Permissions** shows whether Filescape is running as administrator, and
has:

- **Run as administrator at start:** **Always**, **Ask** (if it isn't), or **Never** (don't ask).
- **Restart as administrator > Restart now**, shown when it isn't.

Filescape doesn't need administrator rights for your own files, so there's no need to run it that
way all the time: use it to measure protected system folders, then restart normally.

A restart keeps the command line it was started with, so a scan given on the command line starts
again in the new window.

## Linux and macOS

There's no restart button on Linux and macOS:

- **Linux:** start Filescape with `sudo` from a terminal to read every user's files.
- **macOS:** give Filescape **Full Disk Access** instead. **⚙ Settings > Advanced > Permissions > Open System
  Settings…** takes you there. That's what macOS uses to protect other apps' data; running
  with `sudo` doesn't get around it. See [Folder access on macOS](../disk-usage/hidden-and-unreadable.md#folder-access-on-macos).

## Related topics

- [Hidden, system and unreadable files](../disk-usage/hidden-and-unreadable.md)
- [Settings reference](reference.md)
- [A drive or folder can't be scanned](../troubleshooting/index.md#a-drive-or-folder-cant-be-scanned)
