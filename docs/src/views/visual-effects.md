# Lighting and visual effects

The treemap is drawn on your graphics card, which lets it add a few effects that make it easier
to read. All of them can be adjusted or turned off under **⚙ Settings > Treemap**, and changes
show on the map straight away while the Settings window is open.

## Tile style

**Tile style** picks one of two looks:

- **Modern** (the default): each file is a soft pastel tile with a gentle glow, names and sizes
  are written straight on tiles with room for them, and the folders on screen are outlined with a
  dark tag giving their name and size. The glow's light follows the pointer. This is the same
  look as the map on filescape.app.
- **Classic cushion**: the cushion treemap described below, in stronger colours. The shading
  settings, presets, grid and block depth effect apply to this style only.

## Cushion shading

In the Classic cushion style, each folder's area is shaded like a cushion lit from one side, so the edges between folders
show as creases. This is what lets you see folder boundaries without drawing boxes.

| Setting | Effect |
|---|---|
| **Brightness** | Overall brightness of the tiles. |
| **Cushion shading** | How strong the shading is. 0 makes flat tiles of solid colour. |
| **Height** | How rounded each cushion is. Higher makes the creases between folders deeper. |
| **Scale factor** | How much deeper folders' cushions are flattened compared with their parents. Lower values make the top-level folders dominate the shading. |
| **Light from x / y** | The direction the light comes from. Negative x is from the left, negative y from the top. |
| **Grid** | Draws a thin line (in the colour you pick) around every tile. |

**Presets** set these in one go: **Classic** (the defaults, close to WinDirStat), **Soft**
(gentler shading) and **Flat** (no shading, with a grid).

## Interactive lighting

With **Interactive lighting** on, a soft light follows the pointer across the map (in the Modern
style, the glow on every tile leans towards the pointer), the tile under
the pointer lifts slightly, and the light fades away when the pointer leaves. **Intensity** sets
how strong it is: **Subtle**, **Normal** or **Strong**. It's purely visual; turn it off if you
find it distracting.

## Block depth effect

A very slight bevel on each tile's edges so that neighbouring tiles of the same colour stand
apart.

## Labels

**File names on tiles** (in the filter bar, and under Settings > Treemap > File labels) writes the name
on any tile at least about 44 × 15 points, and the size as well on taller tiles. Names fade in as
tiles grow while you zoom, and get larger on large tiles. They're written in white on a dark
backing so they read on any colour of tile.

## Selection border

The colour of the outline around the selected item, white by default. Pick a different one if
white is hard to see against your files' colours.

## Related topics

- [Settings reference](../settings/reference.md)
- [The treemap](treemap.md)
- [Zooming or rendering performance problems](../troubleshooting/index.md#zooming-or-rendering-is-slow)
