# About Filescape

Filescape is a disk usage visualiser for Windows, Linux and macOS, inspired by WinDirStat. It's
written in Rust, using egui and eframe for the interface, rayon for parallel scanning and wgpu or
OpenGL for drawing.

**❓ Help > About Filescape** shows the version you're running, how the map is being drawn (GPU or
OpenGL), whether Filescape is running as administrator and where its settings are kept. Include
these when reporting a problem.

## Source code

Filescape's source code, including this help, is at
<https://github.com/PlotTwistDev/dirmap>. The help pages are in the `docs` folder; every change to
Filescape is checked against them automatically, so the help describes the version it ships with.

## No warranty, and keep backups

Filescape can delete, move, rename, copy, overwrite, transfer and sync files, and some of those
can't be undone: **Delete Permanently**, anything deleted on a server, and a file replaced by
another. Keep a current backup of anything you can't afford to lose, and check what's selected
before you confirm. Filescape reminds you of this each time it starts, until you tick **Don't show
this again** on the reminder (turn it back on under **Settings > General > Backup reminder at
start**).

Filescape is provided "as is", without warranty. To the extent the law allows, we aren't liable for
lost, damaged or deleted files or data, however it happens. This doesn't affect your rights as a
consumer. The full wording is in the terms at <https://filescape.app/terms>; **Help > About
Filescape** links to them.

## Credits

- Squarified treemaps: Mark Bruls, Kees Huizing and Jarke J. van Wijk, 2000.
- Cushion treemaps: Jarke J. van Wijk and Huub van de Wetering, 1999.
- WinDirStat, for showing how useful a treemap of a disk can be.

## Related topics

- [What's new](whats-new.md)
- [Feature status](feature-status.md)
