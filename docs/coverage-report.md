# Documentation coverage report

This report covers the dirmap help and documentation work: what was audited, what is
documented, what the app does differently from what you might expect, and what is still
missing. It describes the code on this branch.

## What was delivered

| Requirement | Where it is |
|---|---|
| **User manual**: 57 pages, about 26,000 words | `docs/src`, with the contents in `docs/src/SUMMARY.md`. |
| **Quick Start** | `getting-started/quick-start.md`, also under **❓ Help > Getting Started**. |
| **In-app Help Centre** | **❓ Help** menu and **F1**. It has a searchable index with snippets, a contents sidebar, an "On this page" list, breadcrumbs, Back/Forward (buttons, Alt+←/→, mouse buttons), Previous/Next, and copyable code blocks with **Use in Custom filter**. It follows the light/dark theme and works offline. The code is in `src/help/`. |
| **Context-sensitive help (F1)** | Opens the page for the part of the window under the pointer. That covers the toolbar, the folder list, the filter bar (the syntax page when the Custom box is focused), the treemap or sunburst, the current side-panel tab, the status bar (the unreadable-items page when there are some), the Settings window (that tab's section), the scan screen and the start screen. |
| **Tooltips and help links** | Tooltips on: the column headings, side-panel tabs, View and Color by, Settings, every Settings control, and the unreadable count. "?" links next to the Custom filter, the side tabs and the map hint. Also **📖 Full syntax reference…** and **📖 More examples…** in the Custom menu, a **❓ Help** button in Settings, "How duplicates are found", and "New to dirmap? Read the Quick Start". |
| **Keyboard and mouse reference** | Generated from `src/help/reference.rs`. Each entry names the code that implements it, and the tests check that code still exists. The tables are rewritten with `DIRMAP_UPDATE_DOCS=1 cargo test help`. |
| **Filter syntax reference** | `filtering/syntax.md` (every table is executed by the tests), `filtering/examples.md` (every example compiles), `filtering/categories.md` (checked against `categories::of`). |
| **Ten tutorials** | `tutorials/`: largest files, largest folders, files by extension, a render sequence, WSL/VM disks, comparing types, duplicates, dense treemaps, filtering while keeping the layout, and finding missing space. |
| **Troubleshooting and FAQ** | `troubleshooting/index.md`, `troubleshooting/faq.md`. |
| **Settings reference** | Every setting, with its purpose, default and effect, in `settings/reference.md`. The tests fail if a field of `Settings` or `CustomPrefs` isn't documented. |
| **Screenshots** | 13 real captures of this build in `docs/src/images`. |
| **Automated validation in CI** | 21 tests in `src/help/tests.rs`, run by the existing `cargo test --release` step on Windows, Linux and macOS. A new **docs** job builds the site with mdBook and uploads it as the **dirmap-manual** artifact. |
| **Architecture for future updates** | Described in `advanced/architecture.md#the-help-system` and in the README. |

### Small code changes made along the way

- **Unreadable count in the status bar.** The scanner already counted folders and files it couldn't open, but the window never showed the number. The status bar now says "N items couldn't be read." in amber, with a tooltip. I checked this by running as an unprivileged user on a folder with a locked subfolder.
- **Treemap Back/Forward with the help open.** The treemap's mouse Back/Forward buttons used to fire even while the pointer was over another window. They now leave the help window alone, so its own Back/Forward works.

## Feature audit

The user-facing version of this is `about/feature-status.md`. In summary:

**Implemented and documented:**
- scanning, with live progress and the live tree;
- the drive list;
- the folder list and its columns;
- the treemap, with its navigation model;
- the sunburst;
- the colour modes;
- the File types list;
- category, size and Custom filters, in Highlight or Show matches only;
- sequences;
- the size histogram;
- snapshots and comparison;
- duplicates;
- the folder-list right-click menu, with its shortcuts;
- settings;
- run as administrator;
- the command-line report.

**Partly implemented:**
- **Right-click menus** exist only on the folder list and the File types list. The map has none, because right-click zooms out there.
- **Keyboard use** has no arrow-key movement in the folder list and no way to move between panels.
- **Duplicate verification** uses a 64-bit non-cryptographic hash, not a byte-by-byte comparison, and there is no bulk "keep one" action.
- **The sunburst** has no Back/Forward.
- **Sizes** are logical sizes only, and hard links are counted once per name.
- **The administrator restart** button exists only on Windows.

**Not implemented.** These were checked against the code because the request mentioned them. None is described as available anywhere in the docs:
- Compress;
- Sparsify;
- Optimize VHD;
- Custom Cleanup;
- the full Windows Explorer context submenu;
- Group Unregistered Types;
- a size-on-disk mode;
- multi-select;
- export;
- watching for changes;
- reading the NTFS MFT directly;
- multi-drive maps;
- translations.

**Duplicate detection does exist.** It matches by size, then by a hash of the first 64 KB, then by a hash of the whole file.

## Inconsistencies and surprises found

None of these were changed, apart from the two fixes above. Each is documented where users will meet it.

1. **Ctrl+S opens PowerShell on Windows.** Most people expect Ctrl+S to mean Save, so this is easy to press by accident. Consider another key.
2. **Remove Empty Folders runs without asking,** unlike Delete and Empty Folder. It only removes empty folders, but it is the one file-changing command with no confirmation.
3. **Back and Forward only work in the treemap.** In the sunburst the ◀ ▶ buttons are greyed out, and Alt+← and Alt+→ do nothing.
4. **Right-click means different things.** It opens a menu in the folder list and zooms out on the map. People coming from Explorer or WinDirStat will expect a menu on the map.
5. **Some extensions are in surprising categories.** `.ts` is counted as video, not TypeScript; `.key` as a Keynote document; `.obj` as an object file, not a 3D model. A `.bin` file only counts as an AI model when its name looks like one.
6. **The sequence finder also groups code and documents.** In a source tree, `mod_1.py`, `mod_2.py` and so on show up as "sequences". This is correct by its rules but noisy. A filter for image extensions, or a higher minimum frame count, would help.
7. **"Stay on the scanned drive" has no effect on Windows.** It only acts on Linux and macOS, where it compares device IDs. On Windows, mount points are already skipped because they're reparse points. The setting's tooltip now says so.
8. **The mouse Back/Forward buttons fired from anywhere,** even over other windows. Fixed for the help window, as described above.
9. **Scan errors were counted but never shown.** Fixed, as described above.

## Proposed shortcuts (not implemented)

These are listed only here and on the keyboard page's "Not available yet" note. They are not in the reference.

| Proposal | Why |
|---|---|
| ↑ / ↓ / ← / → in the folder list | Standard tree navigation, and the most-requested keyboard gap. |
| Backspace: zoom out one level | Matches Explorer's "up". |
| Ctrl+0: whole map | A common "reset zoom" key, alongside Home. |
| Escape: close the help, About and Settings windows | Today they close only with ✖, Cancel or OK. |
| Back/Forward in the sunburst | Consistency with the treemap. |
| Shift+F10 or the Menu key: open the right-click menu | Keyboard access to the menu. |
| Another key for PowerShell | Frees Ctrl+S, which people expect to mean Save. |

## What was not verified

- **Windows-only behaviour** was read from the code, not run. This build was tested on Linux. That covers:
  - Explorer icons;
  - the Command Prompt and PowerShell entries;
  - the Recycle Bin;
  - Properties;
  - the administrator prompt and restart;
  - the Windows drive list.

  The administrator screenshot comes from an earlier capture of the same dialog, and its text matches the current code. On Windows, the right-click menu has entries the Linux screenshot doesn't show.
- **The mdBook site build** was not run locally, because this environment couldn't download mdBook. The new **docs** CI job runs it on every push. Everything else about the pages (parsing, links, anchors, images) is checked by the tests.
- **Memory and speed figures** in `advanced/millions-of-files.md` come from:
  - a 190,000-file command-line scan here, which peaked at about 150 MB per million files;
  - a 600,000-file GUI scan, at about 470,000 files per second on a warm cache.

  Real drives, especially network drives, will be slower.

## Screenshots

The screenshots were taken from this build on Linux under a virtual display at 1600×1000 (some at 1280×820), using a generated sample folder. That folder held sparse files with realistic sizes, three render sequences with missing frames, AI models, VM disks and three sets of real duplicates. To retake one, follow the procedure in `advanced/architecture.md#the-help-system`. The file names are what the pages reference, and the tests check that each image exists, is a valid PNG and is used by a page.
