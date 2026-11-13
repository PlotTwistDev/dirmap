# Scan exclusions

**⚙ Settings > Filtering** decides what a scan reads in the first place. Unlike the filters in the
filter bar, these **change the totals**: anything excluded here isn't counted anywhere, and you
have to scan again for a change to take effect. When you change them, the status bar shows
**Scan settings changed. Rescan**.

Use them to leave out things you never want to see, such as a backup folder you know about or
temporary files, not to search: for searching, the [filter bar](overview.md) is faster and
doesn't need a rescan.

## The settings

| Setting | Effect |
|---|---|
| **Folders to skip** | One full path per line. The folder and everything under it is skipped. Matching ignores case on Windows, and `\` and `/` are treated the same. Example: `C:\Windows\WinSxS`. |
| **File names to skip** | One pattern per line, matched against file names. `*` matches any characters and `?` one character, ignoring case. Example: `*.tmp` or `~$*`. |
| **Only count file names matching** | One pattern per line. If anything is entered here, **only** matching files are counted. Leave it empty to count all files. |
| **Skip files smaller than** | In KB. 0 turns it off. |
| **Skip files not changed for** | In days. Files last modified longer ago than this are skipped. 0 turns it off. |

Folders can also be added straight from the file manager: right-click a folder and choose **Add to
Exclusion Filter**. On a file, that adds its name to **File names to skip**; in the File types list
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
