# System requirements

| | Requirement |
|---|---|
| **Windows** | 64-bit Windows 10 or 11. No installer or runtime needed. |
| **Linux** | 64-bit x86 with X11 or Wayland, and `libxkbcommon-x11` (desktop installs already have it). |
| **macOS** | macOS 11 Big Sur or later, Apple silicon or Intel. |
| **Graphics** | Any GPU supported by Vulkan, Direct3D 12, Metal or OpenGL 3.3. dirmap uses the GPU first and falls back to OpenGL automatically, which also covers most virtual machines and remote desktops. |
| **Memory** | Depends on how many files you scan: see below. |
| **Disk space** | About 25 MB for the program. dirmap writes nothing else except its settings and any snapshots you save. |

## Memory and scan size

dirmap keeps the whole scan in memory so that browsing, filtering and zooming never have to
read the disk again. As a rough guide, allow **about 150 MB of memory per million files**
scanned, plus some extra while the scan finishes. A drive with 3.5 million files therefore needs
roughly 0.5 to 1 GB. Comparing with a snapshot holds a second copy of the scan, and
**Show matches only** builds a smaller filtered copy.

See [Scanning millions of files](../advanced/millions-of-files.md) for more detail.

## Related topics

- [Installation and first launch](installation.md)
- [Scanning millions of files](../advanced/millions-of-files.md)
