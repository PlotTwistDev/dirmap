# Colour modes

**Color by:** in the filter bar chooses what the colours on the treemap and sunburst mean. It
doesn't change the layout, only the colours.

## Type

The default. Each file is coloured by its extension, matching the swatches in the **File types**
list. The 16 extensions using the most space get bright, distinct colours; the rest get softer
ones. See [File types and extensions](../disk-usage/file-types.md).

## Age

Colours each file by how long ago it was last modified, from hot to cold:

| Colour | Last changed |
|---|---|
| Red | today |
| Orange | about a week ago |
| Yellow | about a month ago |
| Green | about a year ago |
| Teal | about three years ago |
| Blue | ten or more years ago |

Colours blend smoothly between these points. A legend appears under the filter bar. Use it to
find old files nobody has touched for years, or to see what a program has been writing to today.

## Change

Available after you compare with a snapshot (see [Comparing with a snapshot](../analysis/snapshots.md));
until then the button is greyed out. Colours each file by how it changed since the snapshot:

| Colour | Meaning |
|---|---|
| Red | New since the snapshot. |
| Orange | Grew. The stronger the orange, the more it grew relative to its old size. |
| Blue | Shrank. |
| Grey | Unchanged. |

Files deleted since the snapshot aren't on the map, because they're no longer on disk; they're
listed in the **Changes** tab instead. **Stop comparing** in the Changes tab switches back to
**Type**.

## Related topics

- [The treemap](treemap.md)
- [Comparing with a snapshot](../analysis/snapshots.md)
- [Lighting and visual effects](visual-effects.md)
