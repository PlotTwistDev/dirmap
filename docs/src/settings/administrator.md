# Running as administrator

## Why it matters

Windows keeps some folders closed to ordinary programs: other users' profiles, parts of Windows
itself, and folders whose permissions have been restricted. Without administrator rights dirmap
can't look inside them, so their contents are counted as **unreadable** and left out of the
totals. On a system drive this can be many gigabytes.

Running as administrator lets dirmap read almost everything. It doesn't change what dirmap does:
it still only reads, unless you use a right-click command such as Delete. Keep in mind that, as
administrator, those commands can also delete files Windows would otherwise protect.

## The question at start

The first time dirmap starts on Windows without administrator rights, it asks:

| Answer | Effect |
|---|---|
| **🛡 Yes, always** | Restarts as administrator now and every time from now on. |
| **Just this once** | Restarts as administrator now; asks again next time. |
| **No, don't ask again** | Stays as it is and stops asking. |

Restarting shows Windows' usual permission prompt (User Account Control). If you decline it,
dirmap says **Windows didn't start dirmap as administrator, so it's carrying on as it is**, and
keeps running normally.

## Changing it later

**⚙ Settings > Advanced > Administrator** shows whether dirmap is running as administrator, and
has:

- **When dirmap starts:** **Always run as administrator**, **Ask if it isn't**, or **Never (don't
  ask)**.
- **🛡 Restart as administrator now**, shown when it isn't.

A restart keeps the command line it was started with, so a scan given on the command line starts
again in the new window.

## Linux and macOS

There's no restart button on Linux and macOS:

- **Linux:** start dirmap with `sudo` from a terminal to read every user's files.
- **macOS:** give dirmap **Full Disk Access** in **System Settings > Privacy & Security** instead.
  That's what macOS uses to protect other apps' data; running with `sudo` doesn't get around it.

## Related topics

- [Hidden, system and unreadable files](../disk-usage/hidden-and-unreadable.md)
- [Settings reference](reference.md)
- [A drive or folder can't be scanned](../troubleshooting/index.md#a-drive-or-folder-cant-be-scanned)
