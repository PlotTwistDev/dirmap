# Scan exclusions

**⚙ Settings > Filtering** decides what a scan reads in the first place. Unlike the filters in the
**Filters** panel, these **change the totals**: anything excluded here isn't counted anywhere, and you
have to scan again for a change to take effect. When you change them, the status bar shows
**Scan settings changed. Rescan**.

Use them to leave out things you never want to see, such as a backup folder you know about or
temporary files, not to search: for searching, the [Filters panel](overview.md) is faster and
doesn't need a rescan.

## The settings

| Setting | Effect |
|---|---|
| **Folders to skip** | Full paths, added one at a time with **Add** or **Browse…**. The folder and everything under it is skipped. Matching ignores case on Windows, and `\` and `/` are treated the same. Example: `C:\Windows\WinSxS`. |
| **Skip files named** | Patterns matched against file names. `*` matches any characters and `?` one character, ignoring case. Example: `*.tmp` or `~$*`. |
| **Only count files named** | Patterns. If there are any, **only** matching files are counted. Leave it empty to count all files. |
| **Skip small files** | Turn it on and give a size in bytes, KB, MB, GB or TB. |
| **Skip old files** | Turn it on and give a time in days, weeks, months or years. Files last modified longer ago than this are skipped. |

Each rule can be edited (**✎**) or removed (**🗑**), and one that can't work (a folder that isn't a
full path, or a name pattern containing a path) gets a warning sign. **Try a file name** checks
what the rules do with a name you type.

Folders can also be added straight from the file manager: right-click a folder and choose **Add to
Exclusion Filter**. On a file, that adds its name to **Skip files named**; in the File types list
it adds `*.ext`. A message in the status bar confirms it, and the change applies to the next scan.

## Pattern syntax here is simpler

Scan exclusions use **simple wildcards only**: `*` and `?`, one pattern per line, always ignoring
case. The Custom filter's extras (commas, `!`, `#`, `{frame}`, ranges and bare extensions such as
`exr`) don't apply here, so write `*.exr`, not `exr`.

## Other settings that change what's scanned

These are under **⚙ Settings > Advanced**:

- **Stay on the scanned drive** (on by default): don't scan other drives mounted inside the
  folder. See [Links, junctions and mounted drives](../disk-usage/links-and-mounts.md).
- **Skip hidden files and folders** (off by default). See
  [Hidden, system and unreadable files](../disk-usage/hidden-and-unreadable.md).

## Related topics

- [Settings reference](../settings/reference.md)
- [Filtering overview](overview.md)
- [Scan results appear incomplete](../troubleshooting/index.md#scan-results-appear-incomplete)
