# Filescape and the ArtCraft apps: Phase 1 audit

Status: research and compatibility audit, 26 November 2026. Written in answer to Rich's brief
"ArtCraft Ecosystem Integration and Native File Preview Support", Phase 1 and the deliverables
in its section 16. Nothing here is built yet. The roadmap in section 10 is for Rich to approve.

It complements [the preview engine roadmap](filescape-previews.md) (all three of its phases are
merged) and does not repeat it.

## How this was checked

- Every repository was cloned on 26 Nov 2026 at its `HEAD`: `storytold/craft-libs` (3b09a64),
  the 17 Craft Apps, `storytold/artcraft` (ArtCraft Studio, ac0e369) and
  `storytold/craft-launcher` (910beaf). All 20 are public.
- Formats were read from the code that opens and saves them (format crates, `const EXT`,
  `.desktop` `MimeType=` lines and freedesktop MIME XML with magic), and from the apps' own docs
  (`docs/project-files.md`, `docs/file-format-parity.md`, `docs/file-formats.md`).
- getartcraft.com/apps, getartcraft.com/apps/photocraft and craft.wiki were read on the web.
- **A spike** built 11 craft-libs crates (`craft-doc`, `-psd`, `-raw`, `-jpeg`, `-xcf`,
  `-sniff`, `-dxf`, `-aep`, `-scraft-format`, `-fcproj-format`, `-mesh`) as path dependencies
  of a scratch binary with Filescape's toolchain (rustc 1.97, edition 2024). They compiled
  unchanged, pulled about 100 third-party crates that are almost all already in Filescape's
  tree, and `craft-doc` read the VectorCraft and EffectCraft example files in under 1 ms.
- Not done: no ArtCraft app was installed or run, and no rendering through an app's CLI was
  tried. Claims about CLIs come from their source and packaging scripts.

## 1. Compatibility report: the ecosystem as it is

Eighteen products, not seventeen: the website lists 17 Craft Apps, plus ArtCraft Studio. The
launcher's catalogue also carries two community apps (SolidCraft, MailCraft) that are out of
scope.

| Fact | Evidence | What it means for Filescape |
|---|---|---|
| Every app is pure Rust on egui, MIT OR Apache-2.0 | each repo's `LICENSE-*` and `Cargo.toml` | Code can be reused in a commercial app with notices |
| All are **early alpha or "in development"** | getartcraft.com/apps; PhotoCraft is v0.23.1 | Formats change weekly. FilmCraft is on project schema 17. Pin and gate by version |
| Shared code lives in **craft-libs**: 241 crates, 145 documented | craft-libs `README.md`, `crates/` | The reusable part. Not on crates.io: git dependencies only, tag `v0.1.0` covers 5 UI crates, the rest are pinned by commit |
| Most apps clone an Adobe or Microsoft product and read its formats | READMEs ("clean-room reimplementation of Adobe Photoshop", etc.) | The libraries are as useful for PSD, AEP, IDML, DOCX, XLSX files as for the apps' own files |
| Every app ships a **headless CLI** inside its package | e.g. `deckcraft/packaging/macos/package.sh` puts `deckcraft-cli` in `Contents/MacOS/`; nfpm says "Includes deckcraft-cli, a headless renderer" | Installed apps can render previews without opening a window |
| The launcher detects installed apps on every OS | `craft-launcher/crates/native/src/detect.rs` | A small, well-documented detection recipe to follow |
| The ArtCraft name and logo are **trademarks, not open source** | `photocraft/NOTICE`, `docs/brand/LICENSE-brand.txt` | Filescape may name the apps in plain text; it must not use the marks or suggest endorsement |

### Per application

| App | Native format | Thumbnail inside? | Reusable library | Value to Filescape |
|---|---|---|---|---|
| PhotoCraft | `.pcraft` ZIP or folder bundle | Optional `thumb.png`, `composite/preview.png`, `composite/full.png` | `craft-doc`, `craft-pcraft-format`, `craft-psd`, `craft-xcf`, `craft-kra`, `craft-ora`, `craft-pdn`, `craft-cms` | High: PSD layers, GIMP/Paint.NET files |
| VectorCraft | `.vectorcraft` JSON, optionally gzip, blobs after `\n\0VCBLOBS\0` | Optional 256 px PNG and optional PDF of every artboard | `craft-doc`, `craft-vectorcraft-format`, `craft-affinity`, `craft-cdr`, `craft-eps`, `craft-metafile` | High when the PDF is saved (hayro draws it, sharp at any zoom) |
| FilmCraft | `.fcproj` JSON, `format: "filmcraft.project"`, schema 1-17 | No. Media referenced by path | `craft-fcproj-format`, `craft-aaf`, `craft-timeline` (interchange is egui-free), `craft-media` | High for dependency analysis |
| LightCraft | A library **folder** `LightCraft Library/` with `catalog.snap` + `catalog.log` JSON (`lightcraft-catalog` v13), `thumbs/`, `Originals/` | `thumbs/` cache (key-named) | `craft-raw`, `craft-develop`, `craft-develop-model`, `craft-metadata`, `craft-crs`, `craft-heif` | High: real RAW decoding; library storage breakdown |
| PdfCraft | None (edits PDF) | n/a | `craft-pdf` (vendors hayro 0.7) | None: Filescape already uses hayro 0.9 |
| EffectCraft | `.ecproj` JSON, `schema` 1-2; `.ectemplate` package with `poster.png` | Template only | `craft-doc` (compositions), `craft-ecproj-format`, `craft-aep` | High: comps and footage list; AEP too |
| AniCraft | `.anicraft` JSON, `format: "anicraft"`, v3; also opens `.fla`, `.xfl` | No | `craft-anicraft-format`, `craft-xfl`, `craft-lottie` | Medium: metadata; drawing needs the app |
| DesignCraft | `.designcraft` ZIP, MIME `application/vnd.designcraft+zip`, v5 | `preview.png` (≤ 4 MB) and `proxies/` | `craft-designcraft-format`, `craft-document-bundle`, `craft-idml` | Medium-high: preview and linked assets; IDML |
| FigCraft | `.figcraft` ZIP: `document.json` + `thumbnail.png` + `images/`, `versions/`, `branches/` | Yes, `thumbnail.png` | `craft-figcraft-format`, `craft-fig` + `craft-kiwi` (Figma `.fig`), `craft-sketch`, `craft-penpot` | High: thumbnails for .figcraft, .fig and .sketch |
| TextureCraft | `.tcraft` deflate ZIP: `project.json`, `mesh.<ext>`, `pixels/*.bin` (`TCTX`), `baked/*.bin` | Optional `thumbnail.png` | `craft-tcraft`, `craft-mesh` (adds FBX, DAE, Alembic), `craft-usd` | Medium: thumbnail, mesh, texture sets |
| MaterialCraft | `.mcraft` ZIP holding `package.sbs` (Substance Designer XML) and resources | No | `craft-sbs`, `craft-materialx`, `craft-mcmat`, `craft-material-graph` (26k lines) | Low-medium: graph summary and resources; rendering means evaluating the graph |
| SoundCraft | `.scraft` JSON, `format_version` 1-2; `.scraftz` ZIP with embedded audio | No | `craft-scraft-format` (typed reader, validates), `craft-midi`, `craft-sf2` | High for dependency analysis |
| WordCraft | None: DOCX is native; reads DOC, ODT, RTF, Markdown, LaTeX, HTML | n/a | `craft-docx`, `craft-doc-binary`, `craft-wordmodel`, `craft-odf`, `craft-rtf`. Layout and drawing are app crates (`wordcraft-layout`, `wordcraft-render`, CPU via vello_cpu) | Medium: real page pictures, but only by depending on app crates or the CLI |
| DeckCraft | `.deckcraft` ZIP, MIME `application/vnd.storyteller.deckcraft+zip` | No | `craft-deckcraft-format`, `craft-pptx`, `craft-ppt-binary`. Drawing is `deckcraft-render` (app crate, CPU) | Medium: slide pictures via the CLI `render --slide N` |
| CADCraft | None: DXF and DWG are native (`.sv$` autosave) | n/a | `craft-dxf`, `craft-dwg` (acadrust, MPL-2.0, child-process worker), `craft-shx` | Medium: draws DWG, which Filescape can't today |
| GridCraft | `.json` (older `.scjson`) workbook; natively XLSX, XLSM, XLSB, ODS, CSV | n/a | `craft-gridcraft-format`, `craft-xlsx`, `craft-xls`, `craft-ods`, `craft-csv` | Low: calamine already covers values |
| Craft Anything | `<name>.cany/` folder package (`project.toml`) and project folders with `craft.json` | No | `craft-pe`, `craft-elf`, `craft-macho`, `craft-dex`, `craft-java-class`, `craft-tar` | Low-medium: binary inspection, not previews |
| ArtCraft Studio | **No local project file.** Scenes save to ArtCraft's servers (`MediaUploadApi` `scene_snapshot`, a media token). Local data is `~/Artcraft/` (`assets/<date>/`, `downloads/`, `weights/`, `credentials/`, settings) | n/a | none (Tauri + TypeScript front end) | Low: label the folder; treat `credentials/` as sensitive |

## 2. Native file format inventory (verified)

Only formats read from code are listed. "Signature" is what Filescape can check in the first
bytes; JSON formats are recognised by their first member, ZIPs by a named entry.

| Ext | App | Container | Signature | Version field | Thumbnail | Metadata Filescape can read | References other files |
|---|---|---|---|---|---|---|---|
| `.pcraft` | PhotoCraft | ZIP or folder | ZIP with `manifest.json` | manifest `version` 1 | Optional (3 kinds) | canvas, layer tree (`craft-doc`) | No (tiles are inside) |
| `.vectorcraft` (`.drawcraft` older, `.vctemplate`) | VectorCraft | JSON, maybe gzip | `{"format":"vectorcraft"` or gzip | `version` 1-4 | Optional PNG + PDF | artboards, layers, ICC profiles | Images embedded |
| `.fcproj` | FilmCraft | JSON | `"format":"filmcraft.project"` (v1 is a bare object with `"version":1`) | `schema_version` 1-17 | No | sequences, tracks, clips, frame rate, resolution | Yes: media paths, proxies, scratch disks, `Auto-Save/` |
| `LightCraft Library/` | LightCraft | Folder | `catalog.snap` `{"format":"lightcraft-catalog"` | `version` 1-13 | `thumbs/` | photos, albums, ratings, edits | Yes: each photo's original path |
| `.ecproj` | EffectCraft | JSON | `{"savedBy"` or `{"schema"` | `schema` 1-2 | No | compositions, layers (`craft-doc`) | Yes: footage paths |
| `.ectemplate` | EffectCraft | ZIP | `manifest.json` + `project.ecproj` | | `poster.png` | | |
| `.anicraft` | AniCraft | JSON | `{"format":"anicraft"` | `version` 1-3 | No | scenes, timelines | Mostly embedded paths table |
| `.designcraft` | DesignCraft | ZIP | first entry `mimetype` = `application/vnd.designcraft+zip` | 5 | `preview.png` | pages from `document.json` | `assets/`, `proxies/` inside; links possible |
| `.figcraft` | FigCraft | ZIP | `document.json` + `thumbnail.png` | `figcraft` v1 | Yes | pages, frames | Images inside |
| `.tcraft` | TextureCraft | ZIP | `project.json` + `TCTX` members | project `FORMAT_VERSION` | Optional | mesh, texture sets, channels | Original mesh kept inside |
| `.mcraft` | MaterialCraft | ZIP | `package.sbs` | | No | graphs, outputs, parameters | `sbs_package` paths to other packages |
| `.scraft` / `.scraftz` | SoundCraft | JSON / ZIP | JSON with `format_version` | 1-2 | No | tracks, tempo map, sample rate, MIDI | Yes: audio file paths (`.scraftz` embeds them) |
| `.deckcraft` | DeckCraft | ZIP | first entry `mimetype` = `application/vnd.storyteller.deckcraft+zip` | `presentation.json` v1 | No | slides | `media/` inside |
| `.json` / `.scjson` | GridCraft | JSON | GridCraft workbook shape only (no marker) | | No | sheets | |
| `.cany/` | Craft Anything | Folder | `project.toml` with `format_version` | 1 | No | project manifest | Yes |

Format versions are checked on read by every app: a newer file is refused with "update the
app", never half-read. Filescape should do the same: show "saved by a newer FilmCraft" plus
whatever the envelope says, never guess.

## 3. Reusable Rust crates

All are MIT OR Apache-2.0 unless noted. "Lines" is Rust source in `src/`.

### Worth adopting

| Crate | Lines | What it gives | Filescape today | Third-party deps | Verdict |
|---|---|---|---|---|---|
| `craft-doc` | 3.0k | Sniff, version, saved previews/PDF, pages and layers of .pcraft, .vectorcraft, .ecproj | Nothing | serde, flate2, blake3, ruzstd | **Adopt** (compiled, tested on samples) |
| `craft-scraft-format` | 4.4k | Typed, validated SoundCraft sessions | Nothing | serde (+ craft-audio types, craft-midi, craft-time, craft-zip) | **Adopt** for SoundCraft |
| `craft-fcproj-format` | 0.3k | FilmCraft envelope and all migrations | Nothing | serde | **Adopt** for version handling |
| `craft-jpeg` | 5.1k | 12-bit and arithmetic-coded JPEG | Gap noted in the preview roadmap | rayon | **Adopt** (quick win, also DICOM) |
| `craft-psd` | 31.2k | Layer tree, masks, ICC, per-layer pixels, every mode and depth | Own reader: merged image or thumbnail only | flate2, encoding_rs | **Adopt** for the layer list and files without a merged image |
| `craft-xcf`, `craft-pdn`, `craft-ora`, `craft-kra` | 0.8-2.1k each | GIMP, Paint.NET, OpenRaster, Krita layers and pixels | Nothing for any of the four | flate2 | **Adopt**. Krita and OpenRaster also carry `mergedimage.png`; XCF and PDN have no saved picture to fall back on |
| `craft-fig`, `craft-sketch`, `craft-penpot` | 1.5-7.5k | Figma `.fig`, Sketch, Penpot pages and frames | Sketch saved picture | kurbo, ruzstd | Adopt `.fig` thumbnail path; rest later |
| `craft-aep` | 9.8k | After Effects comps, layers, footage | Nothing | none | **Adopt** for dependency analysis |
| `craft-dwg` | 3.4k | DWG to DXF in a child process | DWG details only, no drawing | `acadrust` 0.6.6 (**MPL-2.0**) | Adopt after Rich's OK (MPL is on the allowed list) |
| `craft-mesh` | 12.8k | STL, OBJ, PLY, glTF, FBX, 3MF, DAE, Alembic | Own readers, no FBX/DAE/ABC | miniz_oxide | Adopt for the extra formats |
| `craft-raw` + `craft-develop-model` + `craft-develop` | 37k + 6k + 19k | Real RAW decode (clean room, permissive), the full CPU develop pipeline, Camera Raw `crs:` settings | Embedded JPEG only; LGPL decoders rejected | zune-jpeg, jxl-oxide, rayon | **Spike first**: large, but it is the only permissive RAW decoder found, and it can show LightCraft or Lightroom edits |

### Not worth adopting now

| Crate | Why not |
|---|---|
| `craft-pdf` | Vendors hayro 0.7; Filescape is on hayro 0.9. Duplicate engine |
| `craft-svg` | Wraps resvg/usvg, which Filescape already uses |
| `craft-audio` | Its decoders include AAC and AC-3 (patent questions) and fall back to symphonia (MPL); Filescape uses the system's decoders and should keep doing so |
| `craft-heif` | Pure-Rust HEVC via heic-rs 0.1 (single maintainer); bundling an HEVC decoder is a patent decision the preview roadmap already settled: use the system |
| `craft-midi`, `craft-csv`, `craft-xlsx` | midly, csv and calamine already cover previews; `craft-xlsx` would add cell styles only |
| `craft-docx`, `craft-pptx`, `craft-wordmodel` | Parse well, but they don't draw. Drawing is in app crates (see section 6) |
| UI crates (`craft-ui`, `craft-browse`, `craft-timeline` with `ui`, …) | Would bring a second design system into Filescape |
| `craft-gpu`, `craft-gpu-wgpu` | Filescape's default renderer is glow (OpenGL); its tile shader supports both. No new GPU context is needed for anything above |

## 4. Commercial licensing assessment

- **Code**: MIT OR Apache-2.0 everywhere checked. Filescape must ship the licence texts and
  `NOTICE` contents. The existing generated notices page (Help > About, PR #45) picks up
  crates from `Cargo.lock`; git dependencies need a check that they appear too.
- **MPL-2.0**: `acadrust` (via `craft-dwg`) and `symphonia` (via `craft-audio`). MPL is
  file-level copyleft: fine to ship unmodified with source availability noted. It does not
  touch Filescape's own code.
- **GPL trap**: `craft-text`'s optional `slovak-gpl` hyphenation feature. Not needed by
  anything recommended; keep it off (a `cargo deny` licence check should enforce this).
- **Data files**: FOGRA39 data in `craft-cms` (redistribution allowed unmodified, with
  attribution); OFL test fonts only in tests.
- **Patents**: HEVC (craft-heif), AAC, AC-3/E-AC-3 (craft-audio). Not recommended, so no new
  exposure.
- **Trademarks**: "ArtCraft" and its wordmark and logos are trademarks, explicitly not open
  source, and the brand licence forbids using them to suggest endorsement or in the marketing of
  other products. Per-app icons ship in each repo under `assets/app-icon/`; the NOTICE files
  don't clearly put them under MIT/Apache. **Recommendation**: name apps in plain text only
  ("Open in PhotoCraft"), draw Filescape's own generic file-type glyphs, say nowhere that
  Filescape is affiliated, and ask the ArtCraft team before using any app icon. A settings
  heading "Craft Apps" reads as description rather than branding; Rich's call.
- **No source-disclosure risk** found in anything recommended.

## 5. Comparison with Filescape's preview architecture

Filescape already has what the brief asks the integration to plug into: the format registry
(`src/formats.rs`), shared pane/Quick Look/tile previews (`app/preview.rs`, `app/thumbs.rs`,
`app/tilepreview.rs`), the disk thumbnail cache, cancellation by generation, `catch_unwind`
around decoders, cloud-file rules, and subprocess isolation for system thumbnailers. Creative
project formats already follow one pattern (`daw.rs`, `savedpic.rs`, `cadview.rs`): read
details, list referenced files, mark missing ones, never load plugins.

So ArtCraft support is **more providers in the same registry**, not a new subsystem:

- One module per family, e.g. `src/craftdoc.rs` (PhotoCraft, VectorCraft, EffectCraft via
  `craft-doc`), `src/craftproj.rs` (FilmCraft, SoundCraft, AniCraft, DesignCraft, FigCraft,
  TextureCraft, MaterialCraft, DeckCraft), each registering formats with signatures.
- A shared **project inspection** result (section 7 of the brief) is a generalisation of what
  `daw.rs` already returns: overview rows, structure tree, references with found/missing/cloud
  status, embedded assets with sizes. Ableton, REAPER, Logic and the Craft Apps would all fill
  it.
- The brief says Filescape uses wgpu. It uses glow by default and supports wgpu; nothing
  recommended here needs either: all decoders above are CPU, and pictures go through the
  existing texture path.

## 6. Immediate integration opportunities

In order of value for effort:

1. **Saved previews of Craft documents**: `.pcraft`, `.vectorcraft` (PNG, and the PDF drawn by
   hayro so tiles stay sharp), `.figcraft`, `.fig`, `.designcraft`, `.tcraft`, `.ectemplate`.
   Cheap, safe, and the same shape as `savedpic.rs`.
2. **Project details and references** for `.fcproj`, `.scraft`/`.scraftz`, `.ecproj`, `.aep`
   and LightCraft libraries: duration, frame rate, resolution, tracks, tempo, and every
   referenced file with found / missing / cloud-only, without opening any of them.
3. **12-bit JPEG** (`craft-jpeg`): closes a gap the preview roadmap left open.
4. **PSD layer list** and composites for files saved without "Maximize Compatibility"
   (`craft-psd`), plus **GIMP, Paint.NET, Krita and OpenRaster** pictures (none shown today).
5. **DWG drawings** through `craft-dwg` in a child process (Filescape already has the DXF
   drawer).

## 7. Formats that need more research

| Format | Open question |
|---|---|
| `.vectorcraft` without saved PDF/PNG (all 7 shipped examples) | Drawing needs `vectorcraft-render` (app crate) or the CLI. Measure how often users' files carry the PDF |
| `.deckcraft`, `.anicraft`, `.mcraft`, `.docx` pages, `.pptx` slides | No saved picture. The renderers are CPU and egui-free (`deckcraft-render`, `wordcraft-render` on vello_cpu) but live in app repos that change daily. Spike: depend on one at a pinned commit and measure size, build time, fonts |
| RAW develop | `craft-raw` camera coverage against Filescape users' cameras; speed of a 1000 px develop on CPU |
| LightCraft catalog | Snapshot can be large; read photo paths with a streaming JSON reader under a byte cap |
| GridCraft `.json` | No marker member; must be told apart from any JSON by shape, which risks false matches. Leave as plain JSON unless the app adds a marker |
| `.cany` and Craft Anything projects | Contents are AI build sessions; value is low. Recognise and label only |
| App CLIs as preview providers | Exact flags and output per app, start-up time, whether they open a window or touch the network on some platforms. Needs the apps installed on Rich's Mac |

## 8. Cross-platform assessment

- Every crate recommended builds on Windows, macOS and Linux (and wasm); none has `unsafe`
  or C code, except that `craft-dwg` is unavailable on FreeBSD/aarch64 (not a Filescape
  target).
- **Detection of installed apps**, following the launcher's own logic: macOS
  `/Applications` and `~/Applications` `<App>.app`, read from `Info.plist`, bundle ids
  `ai.storyteller.<app>`; Windows uninstall entries from publisher "Learning Machines LLC" and
  portable copies under `%LOCALAPPDATA%\ArtCraft Launcher\apps\`; Linux AppImages in
  `~/Applications`, `~/AppImages`, `~/.local/bin`, deb/rpm packages, Flatpaks
  (`ai.storyteller.<app>`) and tarballs. The launcher's record is `installed.json` in its data
  folder ("ArtCraft Launcher" on Mac/Windows, `artcraft-launcher` on Linux). Each app's CLI sits
  beside its main binary.
- Opening a file in an app needs no new mechanism: Filescape's Open With already launches
  programs; the work is offering the right app per format.

## 9. Performance and security assessment

- The craft-libs readers are written for hostile input: no `unsafe`, no panics, explicit byte,
  entry and nesting budgets, ZIP bomb checks (`craft-zip`). This is better than several of
  Filescape's own small readers.
- Filescape still wraps each call: worker thread, `catch_unwind`, a time limit, and the input
  size caps it uses today. `craft-dwg` must use its child-process worker (in-process is refused
  by design).
- No project file is executed: expressions (EffectCraft), node graphs (MaterialCraft),
  plugins (SoundCraft VST3/CLAP/AU), macros and scripts are never evaluated. Reading names and
  parameters only.
- Referenced files: paths come from untrusted JSON. Use `craft-docpath`'s rule (local, or on
  the document's own share), resolve without following into other volumes unexpectedly, never
  open a referenced file to check it (metadata only), and never read cloud placeholders
  (existing NOT_LOCAL rule).
- Headless CLIs (optional, later) run as subprocesses with a timeout, a temp output folder, and
  are invoked only for the specific file and only if the user turned external providers on.
- Costs measured: the 11 spike crates add about 1 MB to a stripped binary when used; most of
  their dependencies are already in Filescape's tree.

## 10. Prioritised roadmap (for approval)

Each stage is one or two PRs, independently testable, with fixtures and screenshots, in the
style of the preview roadmap.

**Stage A: Craft documents with saved pictures** (≈ 1 PR)
`craft-doc` at a pinned commit; register `.pcraft`, `.vectorcraft`, `.ecproj`, `.figcraft`,
`.designcraft`, `.tcraft`, `.deckcraft`, `.anicraft`, `.mcraft`, `.fcproj`, `.scraft`,
`.scraftz` by signature. Tiles and pane show the saved picture or VectorCraft PDF; pane shows
app, format version ("newer than Filescape reads" when so), pages/artboards/layers. Files
without a picture get the category tile and details.

**Stage B: project inspection and references** (≈ 2 PRs)
A shared `Project` result used by the pane (Overview, Structure, References, Storage) for
FilmCraft, SoundCraft, EffectCraft, After Effects (`craft-aep`) and the existing DAW readers.
Referenced files: found, missing, cloud-only, other drive, network; total size counted once
per physical file. "Reveal in Storage Analysis" for each. Nothing is ever offered for deletion
on the basis of references. LightCraft Library folders get the same view (catalogue, thumbs
cache, Originals, photos outside the library).

**Stage C: wider professional formats** (≈ 3 PRs)
12-bit JPEG (`craft-jpeg`); PSD layer list and layer composite fallback (`craft-psd`); GIMP
XCF, Paint.NET PDN, Krita and OpenRaster pictures; Figma `.fig` thumbnails; FBX/DAE/Alembic in the 3D viewer
(`craft-mesh`); DWG drawn via `craft-dwg` in a child process.

**Stage D: spikes, then decide** (research PRs, no user-visible change until approved)
RAW develop with `craft-raw` + `craft-develop` (Original/Adjusted switch for LightCraft and
Lightroom XMP edits); page and slide pictures via `wordcraft-render` / `deckcraft-render` as
pinned git dependencies.

**Stage E: installed apps** (≈ 2 PRs)
Detect installed Craft Apps and versions; "Open in <App>" for the formats each app opens
(from its `.desktop` MIME list), preferred app per format, never changing OS file associations;
a link to getartcraft.com when an app isn't installed. Settings section with the detected apps
and what each provides. Optional external preview providers through the apps' CLIs, off by
default, with time and memory limits.

Settings from section 11 of the brief land with the stage that needs them, inside the existing
Previews & thumbnails page plus one "Craft Apps" page for detection and preferences.

## 11. Representative test suite

Fixtures, all MIT OR Apache-2.0 from the apps' repos (provenance to go in
`tests/fixtures/artcraft/README.md` with commit and path):

| Fixture | Source |
|---|---|
| 7 `.vectorcraft` | `vectorcraft/examples/` |
| 5 `.figcraft` with thumbnails | `figcraft/examples/` |
| `Material Ball.tcraft` | `texturecraft/assets/samples/` |
| `deck.deckcraft` | `deckcraft/crates/format/tests/golden/` |
| 6 `.fcproj` across schema versions | `filmcraft/crates/format/tests/fixtures/` |
| 3 `.ecproj` | `effectcraft/crates/engine/src/tests/fixtures/` |
| 2 `.anicraft` (v3) | `anicraft/crates/engine/tests/fixtures/` |
| `.pcraft`, `.scraft`, `.designcraft`, `.mcraft`, LightCraft library | None committed upstream: generate small ones with the format crates' writers in a test helper |

Tests per stage: sniffing (right kind, and no false match on ordinary ZIP/JSON), version
gating (a file with version + 1 reports "newer"), saved-picture extraction, reference listing
with missing and cloud-only paths, hostile inputs (truncated, ZIP bomb, deep JSON, path
traversal in references), and a time budget per preview.

## 12. Supported formats documentation

Nothing is added to `docs/src/reference/formats.md` until a stage ships: that page is
generated from the registry and only lists formats with a working preview. Each stage adds its
rows there and to the tracker below.

## Tracker

| Stage | PR | State |
|---|---|---|
| Phase 1 audit (this doc) | #89 | Merged |
| Stage A: Craft documents with saved pictures | #91 | In review |
| Stage B: files a project uses, After Effects projects, LightCraft libraries, PhotoCraft bundles | #94 | In review |
| Stage C1: 12-bit and arithmetic JPEG, Photoshop layers | (this PR) | In review |
| Stage C2: Krita, OpenRaster, Figma, GIMP and Paint.NET | (this PR) | In review |
| Stage C3: FBX, COLLADA, Alembic models; DWG drawings | (this PR) | In review |
