# About dirmap

dirmap is a disk usage visualiser for Windows, Linux and macOS, inspired by WinDirStat. It's
written in Rust, using egui and eframe for the interface, rayon for parallel scanning and wgpu or
OpenGL for drawing.

**❓ Help > About dirmap** shows the version you're running, how the map is being drawn (GPU or
OpenGL), whether dirmap is running as administrator and where its settings are kept. Include
these when reporting a problem.

## Source code

dirmap's source code, including this help, is at
<https://github.com/PlotTwistDev/dirmap>. The help pages are in the `docs` folder; every change to
dirmap is checked against them automatically, so the help describes the version it ships with.

## Credits

- Squarified treemaps: Mark Bruls, Kees Huizing and Jarke J. van Wijk, 2000.
- Cushion treemaps: Jarke J. van Wijk and Huub van de Wetering, 1999.
- WinDirStat, for showing how useful a treemap of a disk can be.

## Related topics

- [What's new](whats-new.md)
- [Feature status](feature-status.md)
