# Filescape: the expanded preview engine

Status: review and roadmap, 25 November 2026. Written in answer to Rich's brief "Advanced File
Preview Engine and Expanded File Format Support" (sections 1 to 14). This document is section
14's deliverable: a review of what exists, a licence review of candidate libraries, and a
prioritised roadmap. Phase 1 is built in small PRs after it, each listed in the tracker at the
end.

A format is only called supported once its preview has been built and checked, never because
its extension was added to a list.

## 1. What exists today (main at caa4ecd)

### 1.1 The pieces

| Piece | File | What it does |
|---|---|---|
| Kind of file | `app/preview.rs` `kind_of` | Extension → Image, LinearImage, Text, Media(Video/Audio/Pdf), Other |
| Text kind | `app/thumbs.rs` `text_kind` | Extension → Plain, Docx, Odt (SVG excluded on tiles) |
| Media kind | `media.rs` `media_of` | Extension → Video, Audio, Pdf |
| Full preview | `app/preview.rs` `load`, `preview_body` | Worker thread loads `Content` (Image, Text, Sound, Error…); the same body draws the File Manager pane, Quick Look and the Storage Analysis pane |
| Quick Look | `app/preview.rs` `quick_look` | Space in the File Manager: a large window, ⏶⏷ for the next item, Open |
| Thumbnails | `app/thumbs.rs` | Worker pool, newest first, memory budget with LRU eviction, `stamp(size, mtime)` invalidation; used by icon views, gallery and tiles |
| Tile previews | `app/tilepreview.rs` | Fades previews in on zoomed treemap tiles; two thumbnail pools (320 px, 1024 px from 400 px across); hover-to-play video; waveforms; cloud files never read |
| System pictures | `media.rs` `thumbnail` | Video frame / PDF page / album art: Windows shell thumbnail handlers (+ Windows.Data.Pdf), macOS `qlmanage -t`, Linux `pdftoppm`, `ffmpegthumbnailer`, `ffmpeg` |
| Video playback | `video.rs` | AVFoundation, Media Foundation, ffmpeg; frames into a texture |
| Sound | `media.rs`, `macaudio.rs`, `wave.rs` | Play/pause/seek; waveform peaks via AVFoundation, Media Foundation, ffmpeg |
| Documents | `app/thumbs.rs` `doc_text` | Words of .docx/.odt (zip + roxmltree), no formatting |
| Markdown | `help::render` | Formatted .md through the Help Centre renderer, with a Raw switch |
| Archives | `fsys/archive.rs` | zip, 7z, tar(.gz/.bz2/.xz), browsable as folders; the pane lists the contents |

### 1.2 Formats covered now

- **Pictures** (pure Rust, `image` crate): png, jpg/jpeg/jpe, gif, webp, tif/tiff, bmp, tga, ico,
  qoi, pbm/pgm/ppm/pnm; exr and hdr through the sRGB curve. Limits: 256 MB file, 16 384 px a
  side, 512 MB allocation.
- **Video** (system decoders): mp4, m4v, mov, mkv, avi, wmv, webm, mpg/mpeg, m2ts/mts, 3gp, flv,
  mxf. A poster frame, plus playback in the pane and on hover. What actually decodes depends on
  the system's codecs.
- **Sound** (system decoders): mp3, wav, flac, m4a, aac, ogg/oga, opus, wma, aif/aiff, alac.
- **PDF**: first page only, as a picture.
- **Text and code**: about 110 extensions plus README, LICENSE and the like; unknown files that
  look like text. Monospace, wrapped, first 256 KB, no colour.
- **Documents**: docx/docm/dotx, odt/ott as plain words.
- **Markdown**: formatted, or raw.
- **SVG**: shown as markup text only.
- **Archives**: a flat list of contents in the pane; browsing as folders in the file manager.

### 1.3 Findings

1. **Classification is spread over three functions** (`kind_of`, `text_kind`, `media_of`) and
   is by extension only. A renamed PNG is "Other"; a `.txt` that's binary is caught only by the
   text decoder. → a single **format registry** with signatures (section 3).
2. **No duplicated preview drawing.** The File Manager pane, Quick Look and the Storage
   Analysis pane share `preview_body`; icons, gallery and tiles share `thumbs.rs`. Good: new
   formats are added once.
3. **Quick Look exists but only in the File Manager.** Not from Storage Analysis tiles, no
   full screen, no pin, no zoom/pan on pictures.
4. **Thumbnails are memory-only.** Each launch re-decodes. No disk cache, no size setting.
5. **Jobs aren't cancelled** when a tile leaves the screen; newest-first ordering hides most of
   it, but a burst of zooming queues work that's no longer needed.
6. **System thumbnailers run as subprocesses on macOS and Linux** (good isolation), but
   in-process on Windows (shell handlers) and for the pure-Rust decoders. A decoder panic is
   caught (`catch_unwind`); an abort (as with the AVFoundation exception, fixed in #38) is not.
7. **Security**: no scripts run anywhere today. SVG is shown as text. Archives are listed, not
   extracted, except one item to a temp file for previewing (path checked). No size cap on
   zip *uncompressed* totals for the single-item extraction (bounded by `IMAGE_MAX_FILE` only
   for pictures).
8. **Privacy**: no previews leave the computer. Sensitive files (keys, `.env`) are shown as text
   like any other: the brief asks for tile thumbnails of these to be off by default.
9. **Cloud**: tiles never read NOT_LOCAL or MOUNT files; the pane asks before downloading. That
   stays the rule for every new format.
10. **No third-party licence notices ship with the app.** Most Rust crates are MIT/Apache, which
    require their notices in binary distributions. This is already owed for today's
    dependencies, before anything new is added. → generate a notices file at build time
    (`cargo about`) and show it in About. Independent of the preview work, but should land soon.

## 2. Licence review of candidate libraries

Checked on crates.io, 25 November 2026.

| Need | Candidate | Licence | Notes | Verdict |
|---|---|---|---|---|
| SVG | `resvg` / `usvg` 0.50 | MIT or Apache-2.0 | Pure Rust, no scripting; external hrefs can be refused | **Use** |
| Syntax colour | `syntect` 5.3 | MIT | Use `default-fancy` (pure-Rust regex) to avoid Oniguruma (C) | **Use** |
|  | `two-face` 0.5 | MIT or Apache-2.0 | bat's extra grammars (TOML, Dockerfile, etc.); grammar files carry their own (permissive) licences, to list in notices | Use with syntect |
| Fonts | `ttf-parser` 0.25 | MIT or Apache-2.0 | Names, axes, glyph outlines | **Use** |
|  | `ab_glyph` 0.2 | Apache-2.0 | Rasterise sample text into a picture | **Use** (already in the tree via egui) |
| Spreadsheets | `calamine` 0.38 | MIT | xlsx, xlsm, xlsb, xls, ods; cached values only, never runs formulas or macros | **Use** |
| PDF pages | `hayro` 0.9 | MIT or Apache-2.0 | Pure-Rust renderer, young; would give every page on every system | **Use** (chosen 25 Nov) |
|  | `pdfium-render` 0.9 | MIT or Apache-2.0 | Needs the PDFium library (BSD-3) shipped beside the app, ~5 MB a platform | Not needed |
|  | System | n/a | Windows.Data.Pdf and `pdftoppm` already render any page; macOS needs CGPDFDocument | Use where there today |
| Photoshop | `psd` 0.3 | MIT or Apache-2.0 | Flattened composite; plus the embedded JPEG thumbnail (resource 1036) as a cheap path | **Use**, guarded by limits |
| Camera RAW | own reader + `kamadak-exif` 0.6 | BSD-2-Clause | Embedded JPEG previews from TIFF-based RAWs (CR2, NEF, ARW, DNG, PEF, ORF, RW2…) and CR3/ISO boxes; EXIF metadata | **Use** |
|  | `rawler` / `rawloader` | LGPL-2.1 | Full RAW decode. LGPL in a statically linked Rust binary obliges relinkable objects for users: avoid | Avoid |
| HEIC / HEIF | System | n/a | macOS ImageIO/Quick Look (built in); Windows shell (needs the free HEIF extension); Linux `heif-thumbnailer` if installed | **Use** |
|  | `libheif-rs` 3 | MIT (binding) | libheif itself is LGPL-3 with HEVC patents: avoid bundling | Avoid |
| AVIF | `image` `avif-native` | MIT/Apache + libdav1d BSD-2 | C dependency, cross-builds need care | Phase 1 via system first, then this |
| JPEG XL | `jxl-oxide` 0.12 | MIT or Apache-2.0 | Pure Rust, through the `image` crate's decoding hook | **Use** |
| Audio tags, cover art | `lofty` 0.25 | MIT or Apache-2.0 | Title/artist/album, embedded pictures, duration, sample rate, bit depth | **Use** |
| Audio decode | `symphonia` 0.6 | MPL-2.0 | File-level copyleft (changes to its own files must be published), fine to ship. System decoders already cover playback | Not needed yet |
| MIDI | `midly` 0.5 | Unlicense | Piano roll | Phase 2 |
| SQLite | `rusqlite` 0.43 (bundled) | MIT; SQLite public domain | Open read-only, `immutable=1` | Phase 2 |
| Parquet | `parquet` 60 | Apache-2.0 | Large dependency tree | Phase 2/3 |
| DICOM | `dicom-object` 0.10 | MIT or Apache-2.0 | Strip patient fields from tiles | Phase 3 |
| EPUB | own (zip + roxmltree) | n/a | The `epub` crate is GPL-3: write our own small reader | **Own** |
| PowerPoint, formatted Word | own (zip + roxmltree), system | n/a | No permissive Rust renderer exists. Word → headings, paragraphs, tables, bold/italic, pictures drawn with the Help renderer's blocks; PowerPoint → slide text outline plus `docProps/thumbnail.jpeg` when saved; macOS Quick Look and Windows shell handlers render real first slides/pages | **Own + system** |

Codec licensing: Filescape ships no video or HEVC decoders of its own; it uses the system's.
Keep it that way (H.264/HEVC patent pools). ProRes, DNx, BRAW and R3D are decodable only where
the system or the vendor's free SDK provides it.

## 3. Architecture

Rather than rewriting, grow what's there:

1. **Format registry** (`src/formats.rs`): one table of `Format { name, exts, category,
   signatures, mime, tile, pane, metadata, platforms, limits }`. `kind_of`, `text_kind` and
   `media_of` become lookups into it. Signatures (magic bytes) are checked when the extension
   is unknown or doesn't match. The Supported Formats directory (brief §11) is generated from
   this table, so it can't drift from the code.
2. **Providers stay functions, not plugins**, keyed by the registry: a tile picture
   (`thumbs::decode`), a pane `Content` (`preview::load`), and metadata rows. A provider
   returns "not handled" so the next one (system thumbnailer, embedded thumbnail) can try.
   Proper trait objects only if a third party ever needs to add one.
3. **New `Content` variants** as formats need them: `Svg { pic, source }`, `Code { spans }`,
   `Sheet { sheets }`, `Slides`, `Pages { count, current }`, `Font { … }`, `Book { … }`.
4. **Thumbnail pipeline**: keep `thumbs.rs`; add cancel-by-generation (drop jobs for tiles no
   longer visible), a per-job time limit, and later an on-disk cache keyed by path + stamp.
5. **Isolation**: system thumbnailers stay out of process. New pure-Rust decoders run in the
   worker with `catch_unwind`, input-size limits and allocation caps. A subprocess sandbox for
   higher-risk decoders is Phase 2.

## 4. Roadmap

### Phase 1: everyday formats (now)

In this order, each its own PR with tests and screenshots:

1. **Format registry** (no visible change) and signature sniffing.
2. **SVG and SVGZ rendered** on tiles and in panes; Rendered/Source switch. No scripts, no
   external files or network.
3. **Syntax highlighting** in the panes (and coloured text on tiles), line numbers, wrap toggle.
4. **Fonts** (ttf, otf, ttc, woff where uncompressed tables allow; woff2 later): a sample on
   tiles; family, style, axes, alphabet, sizes and custom text in the pane.
5. **Sound pane**: waveform with playhead and click-to-seek, tags and cover art (lofty),
   sample rate, bit depth, channels; more extensions (aifc, caf, wv, ape where the system
   decodes them). Tile choice of waveform or cover art.
6. **Camera RAW**: embedded previews and EXIF (camera, lens, exposure, ISO, date, GPS); system
   fallback.
7. **HEIC/HEIF/AVIF** through the system; AVIF through dav1d after.
8. **Photoshop** (psd/psb): embedded thumbnail, then the composite.
9. **Spreadsheets**: grid with sheet tabs, virtualised rows, cached values; a mini grid on tiles.
10. **Word, formatted** (headings, lists, tables, emphasis, pictures); **PowerPoint**: slide
    outline, slide count, the saved thumbnail; system renderers where present.
11. **PDF, every page**: page count, next/previous, scrolling; text search later.
12. **EPUB**: cover on tiles; title, author, chapters and readable text in the pane.
13. **Quick Look everywhere**: from Storage Analysis too, full screen, zoom and pan, pin.

### Phase 2: creative and professional

3D viewer (OBJ, STL, PLY, glTF/GLB, USDA, 3MF; offscreen thumbnail on tiles), MIDI piano roll,
SQLite/structured data, archive tree with sizes and ratios, image channels and HDR exposure,
multilayer EXR, filmstrips and representative frames, JPEG XL, disk thumbnail cache, Previews &
Thumbnails settings page with presets, Supported Formats directory.

### Phase 3: specialist and proprietary

Embedded thumbnails or metadata only for Blender, Maya, 3ds Max, Cinema 4D, Houdini, SketchUp,
Affinity, Sketch, CorelDRAW, Ableton, Logic, Cubase, FL Studio, REAPER; Apple iWork (their
embedded preview JPEG/PDF); CAD (DXF drawing; DWG/STEP/IGES research); geographic (GeoJSON,
GPX, KML outlines); DICOM and FITS; disk images read-only; email (eml, mbox, vcf, ics).

## 5. Open decisions for Rich

1. **Free or Professional**: decided 25 Nov, every preview is free for now.
2. **Third-party notices** (finding 10): decided 25 Nov, yes, a generated page in Help > About.
3. **PDF renderer**: decided 25 Nov, hayro. In the spike it drew a 14-page TeX paper and pdf.js
   test files correctly in 1-20 ms a page at 1000 px wide, adds about 4 MB to the app and
   needs nothing shipped beside it. PDFs it can't open (password-protected, damaged) fall back
   to the system's first-page picture.

## 6. Tracker

| Step | PR | State |
|---|---|---|
| Review and roadmap (this doc) | #43 | Merged |
| Format registry and file signatures | #44 | Merged |
| Third-party licence notices in About | #45 | Merged |
| SVG drawings (resvg) | #46 | Merged |
| Syntax colouring (syntect + two-face) | #47 | Merged |
| Font specimens | #48 | Merged |
| Sound pane: tags, cover, waveform | #49 | Merged |
| Camera RAW (embedded JPEG + EXIF) | #50 | Merged |
| HEIC and AVIF through the system | #51 | Merged |
| Photoshop PSD/PSB (own reader) | #52 | Merged |
| Spreadsheets (calamine) | #53 | Merged |
| Word, PowerPoint and OpenDocument formatted | #54 | Merged |
| Every PDF page (hayro) | #55 | Merged |
| EPUB books (own reader) | #56 | Merged |
| Quick Look from Storage Analysis, full screen | #57 | Merged |
| **Phase 2** | | |
| JPEG XL (jxl-oxide) | #66 | Merged |
| MIDI piano roll (midly) | #67 | Merged |
| SQLite (rusqlite) and CSV tables | #68 | Merged |
| Archive tree with sizes and ratios | #69 | Merged |
| Image channels, HDR exposure, multilayer EXR | #70 | Merged |
| Video filmstrips and representative frames | #71 | Merged |
| 3D model viewer (STL, OBJ, PLY, glTF, 3MF) | #72 | Merged |
| Disk thumbnail cache, Previews & thumbnails settings | #73 | Merged |
| Supported formats directory | #74 | Merged |
| **Phase 3** | | |
| Email, contacts and calendars (eml, mbox, vcf, ics) | | In review |
