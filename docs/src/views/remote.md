# The Remote workspace

The **🖧 Remote** workspace (**Ctrl+2**, **⌘2** on a Mac) is for working with servers: this
computer on the left, a server on the right, and your saved sites in the sidebar. It's the same
file manager as the [Files workspace](file-manager.md), with the same views, menus, shortcuts,
preview, drag and drop and file operations. Only the layout and the sidebar are its own.

![The Remote workspace: a local folder on the left, an SFTP server on the right, saved sites in the sidebar](../images/remote-workspace.png)

## Its own panes

Remote keeps its own panes, tabs and layout, separate from Files. Switching between Files and
Remote takes you back to where you were in each, and both are remembered when dirmap closes.
The first time, Remote opens with two panes: your Files folder on the left and the site you used
last on the right. Favourites, saved sites, tags and notes are shared by both workspaces.

Everything from the Files workspace works here, on either side: tabs, Back and Forward, the
address bar, Details, Icons and Columns views, filtering and grouping, sequences, Quick Look,
copy and move with clash handling and Undo, **Swap Panes** and **Compare Folders**. Either pane
can show this computer or any server, so local beside remote, remote beside remote and local
beside local all work.

## Working with two panes

With two panes, **F5** copies and **F6** moves the selected items to the other pane's folder, as
in WinSCP's Commander view (**Shift+F5** and **Shift+F6** do the same, as in Files). **Ctrl+R**
reads the folder again. Copies and moves to or from a server go to the
[transfer queue](#transfers); they ask about names already taken and can be undone, as in Files.

On a server, the item menu leaves out what only works on this computer (Show in Explorer, the
Windows menu, the Recycle Bin, the network path). **Delete Permanently** is the only delete,
because a server has no bin.

If a folder can't be read (the connection dropped, the server refused), the pane says why, with
**Try Again** and **Go up**.

## Transfers

Every copy or move to or from a server, however you start it (F5, drag and drop, paste, a drop on
a site), joins the transfer queue and runs in the background, so you can keep browsing, on the
same server too. Copies between folders on this computer work as in Files.

![The Transfers panel with two downloads running and one upload finished](../images/remote-transfers.png)

The **Transfers** chip in the status bar shows how many are running or waiting and how fast
they're going. Click it to open the **Transfers** panel along the bottom:

- Each running transfer shows how much has arrived, its speed and the time left. Hover over the
  bar to see the file it's on.
- **Pause** and **Resume** hold one transfer; **Pause All** holds them all. Pausing a waiting one
  keeps it from starting.
- **Stop** (or **Cancel** for one that hasn't started) ends it. Files that arrived complete
  stay, and **Undo** removes them; the file it was part-way through is removed.
- **At once** sets how many run at the same time (two at first, up to eight). Each uses its own
  connection to the server, so a long upload doesn't hold up browsing.
- Finished transfers stay in the list with what happened (the last 100). **Clear Finished**
  empties it; **Remove** takes one out.

When a name is already taken you're asked, one question at a time, as in Files: **Replace**,
**Keep Both**, **Skip** or **Stop**.

### When the connection drops

A file is written under a temporary name (`.name.dirmap-part`, beside where it's going) and only
takes its real name once all of it has arrived and its size matches. So a file with its real name
is always complete, and a cut-off transfer never replaces a good file.

If the connection drops, dirmap connects again and carries on from where it got to, up to three
times (after 1, 2 and 4 seconds). If it still can't, the transfer is marked failed and the part
file stays. **Retry** runs it again: files that already arrived (same size and time) are skipped,
folders are merged, and the part file is continued rather than started again. Asking for the same
copy later, even after closing dirmap, continues it the same way.

Continuing part-way works for downloads from SFTP, FTP and WebDAV servers that allow it, and for
uploads to SFTP and FTP. Elsewhere the file starts again from the beginning.

Every file's size is checked once it has arrived. dirmap doesn't compare checksums yet.

## SSH terminal

On an SFTP server, **Open SSH Terminal Here…** (in the item menu, the connection menu in the
toolbar, or **Ctrl+Alt+T**) opens your system's terminal signed in to the server, in that folder
(a file's own folder for a file). It runs the system's `ssh` with the server's address, port and
key file, so it uses your SSH agent, your keys and your `~/.ssh/config`. If the server needs a
password, `ssh` asks for it in the terminal.

- **Windows** opens a console window running `ssh.exe`. It's part of Windows 10 and 11; if it's
  missing, add **OpenSSH Client** in Settings > System > Optional features.
- **macOS** opens Terminal.
- **Linux** opens `$TERMINAL`, or the first of x-terminal-emulator, GNOME Terminal, Konsole and
  others that's installed.

`ssh` keeps its own list of known servers (`~/.ssh/known_hosts`), so the first time it may ask you
to accept the server's key, even if dirmap already knows it.

## Properties and permissions

**Properties** (**Alt+Enter**) on an item on a server shows what the server says about it: its
type, size, when it was modified, its owner and group (as numbers), where a link points, and its
permissions.

On SFTP and most FTP servers you can change the permissions: tick **Read**, **Write** and **Run**
for the owner, the group and others, or type the octal value (`755`), then **Apply**. For a
folder, **Also everything inside** applies them to every file and folder in it as well; folders
also get Run wherever they have Read, so they can still be opened. Links inside are left alone.
WebDAV servers have no permissions to change.

## Quick connect

Type an address in the **Quick connect** box in the toolbar and press **Enter**:

| You type | It connects to |
|---|---|
| `render01` or `rich@render01:2222/mnt/shots` | SFTP (a bare name is SFTP) |
| `sftp://rich@render01/mnt/shots` | SFTP |
| `ftps://ftp.example.com` | FTP over TLS (on port 990, TLS from the start, for older servers) |
| `ftp://nas.local` | FTP, after a warning that it isn't encrypted |
| `https://cloud.example.com/remote.php/dav` | WebDAV over https |
| `\\server\share` or `smb://server/share` | A network share, through your system ([Connect to Server](file-manager.md#network--remote)) |

The server opens in the pane that's already on a server, otherwise in the right-hand pane, so
this computer stays beside it. A quick connection isn't saved: use **Save as Site…** in the
connection menu (below) to keep it.

## The connection in the toolbar

Beside quick connect, the toolbar shows the focused pane's server: a closed padlock for an
encrypted protocol, an open one (in orange) for one that isn't, the server's name and the round
trip time of its last small requests. Click it for:

- **Connection Log…**: what dirmap asked the server and how long it took, and any errors.
  Passwords, passphrases and keys are never written to it.
- **Disconnect**.
- **Save as Site…**, for a server you connected to with quick connect.

## Sites

The **Sites** section of the sidebar lists your saved servers: favourites first, then by group.
A dot marks the ones connected now. Click a site to connect and open it; middle-click, or
right-click and choose **Open in New Tab**, to open it in a new tab. If the site has a local
folder, that folder opens in the other pane. Drop files on a site to copy them to its folder.
Right-click a site for **Add to Favourites**, **Edit…** and **Disconnect**. With more than six
sites a search box appears above them.

Servers you're connected to that aren't saved are listed under **Other Connections**.

## The Site Manager

**🗄 Sites** in the toolbar, or **✚ New Site…** in the sidebar, opens the Site Manager. The list
on the left has your sites by group, with a search box. The form on the right holds:

| Field | What it's for |
|---|---|
| **Name** | What the sidebar shows. Empty: the user and server. |
| **Group** | A folder for it, such as `Render Farm`. Use a slash for a subfolder: `Clients/Acme`. Pick an existing group from the menu beside it. |
| **Protocol** | SFTP, FTPS, FTP, WebDAV (https) or WebDAV (http). |
| **Server** and **Port** | The name or address. Paste a whole address and the rest fills in. |
| **User name** | Empty for SFTP: your user name on this computer. Empty for FTP: anonymous. |
| **Sign in with** | SFTP only: a key file. Empty: your SSH agent, then the keys in your `.ssh` folder, then a password. |
| **Remote folder** | Where it opens. Empty: your home folder (SFTP) or `/`. |
| **Local folder** | A folder on this computer to show beside it. |
| **Notes** | Anything you want to remember about it. Shown in the site's tooltip. |
| **★ Favourite** | Listed first. |

The buttons:

- **Save** keeps the form as a site (or updates the one selected).
- **Test** connects, lists the remote folder and disconnects, without saving, and says what
  happened. Any questions (the server's identity, a password) are asked as when connecting.
- **Connect** saves and opens it.
- **New Site**, **Duplicate** and **Delete** under the list. Deleting a site also removes any
  password kept for it in the keychain. Right-click a group's name to rename it.

Passwords are never saved in dirmap's own settings. When you tick **Remember** in the sign-in box
they go to Windows Credential Manager, your macOS Keychain or your desktop's keyring.

## Unencrypted protocols

FTP and WebDAV over http send your password and files as plain text that anyone on the network
can read or change. Before the first connection to such a server, dirmap asks: **Connect
Anyway** or **Cancel**. A saved site remembers the answer; a quick connection asks again the next
time dirmap starts. Use SFTP, FTPS or https when the server offers them.

## Safety

- The server's identity is checked for SFTP, and the certificate for FTPS and https, as described
  in [Servers](file-manager.md#servers-sftp-ftp-and-webdav).
- A name a server lists is always used as one name. Names such as `..`, or ones containing a
  slash, are left out of the list (the connection log says so), and a download never writes
  outside the folder you chose.
- Delete on a server is permanent and dirmap asks first.

## Related topics

- [The Files workspace](file-manager.md)
- [Servers: SFTP, FTP and WebDAV](file-manager.md#servers-sftp-ftp-and-webdav)
- [Keyboard shortcuts](../reference/keyboard.md)
