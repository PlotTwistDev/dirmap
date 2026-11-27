# Gaussian Splat viewing and inspection: research and plan

Status: S1 (#96), S2 (#98), S3 (#101), S4 (#104) and S5 (#106) merged; S6 (#116) and S7 (#120) merged, S8 in review, 27 Nov 2026. Rich's M4: 16 ms for 150k splats, 39 ms for 786k. Free/Pro split decided (section 10). S4 leaves out hover orbit on tiles; S5 leaves out orthographic view and free-look fly mode (W A S D moves the orbit instead). S6 writes SPZ v3 (gzip, the most widely read version) rather than v4.

This answers section 28 of Rich's brief "Native Gaussian Splatting Viewer and Scene Inspection": items 1 to 6 and 9 are in this document; item 7 (prototype) is a headless spike described below; items 8, 10 and 11 belong to the build stages.

Everything here was checked against the real source code, cloned on 26 Nov 2026, unless it says otherwise.

## 1. The short answer

- **Use `wgpu-3dgs-viewer` 0.8 (MIT or Apache-2.0) for drawing, and write our own file readers.** It uses wgpu 30, the same version Filescape already ships, sorts on the GPU, and already has the splat, ellipse and point modes the brief asks for. In a spike it drew two real Niantic scenes (786k and 933k Gaussians) headless, with no window, even on a software GPU.
- **Run it on its own wgpu device, drawing off-screen, and show the picture as an egui texture.** Filescape draws its window with OpenGL on Windows, and OpenGL 3.3 has no compute shaders, so we can't draw inside eframe's own pipeline there. One path for every OS also gives thumbnails for free.
- **No WebView and no SuperSplat embedding.** SuperSplat is MIT and excellent, but it needs a browser engine (WebView2, WebKitGTK), which is a large new dependency and a new attack surface. It adds nothing the native path can't do for viewing.
- **Never use the original INRIA code or anything derived from it.** The Gaussian-Splatting License allows research use only ("may be used non-commercially"). None of the crates we'd adopt contain it (checked below).
- **Read formats ourselves, porting PlayCanvas `splat-transform` (MIT) as the reference.** It is the most complete and current open description of every format the brief lists (PLY, compressed PLY, SOG, streamed SOG, SPZ, SPLAT, KSPLAT, LCC, LCC2, GLB). We'd keep its notice.
- **Size cost: about 5 MB** on the stripped binary in the spike, as an upper bound. All 45 new crates are MIT, Apache or BSD.
- **Conversion is real and measurable.** The two Niantic SPZ samples hold 23 to 26 bytes per Gaussian, against 248 bytes for the same scene as a standard PLY: about 10 to 1. The storage optimiser card can show real numbers, not an assumed ratio.

## 2. What Filescape has today

| Piece | Where | What it means for splats |
|---|---|---|
| Window drawing | `main.rs` | eframe 0.36 with **both** renderers compiled in. Windows starts with OpenGL (glow) and falls back to wgpu; macOS and Linux start with wgpu. So wgpu 30.0.1 is already in the binary. |
| GPU tiles | `tilegpu.rs` | Treemap tiles are drawn by our own shaders in WGSL and GLSL through paint callbacks. A pattern to copy, but splats need compute, which the GLSL side can't do. |
| 3D models | `model3d.rs` | A software rasteriser that draws clay meshes (STL, OBJ, PLY, glTF, 3MF). No GPU. A splat PLY today shows "This model has no faces to show." |
| Format registry | `formats.rs` | `ply` and `glb` are both claimed by the `model3d` format. Splat detection needs a look inside the file, not only the extension. |
| Previews | `app/preview.rs`, `app/thumbs.rs`, `thumbcache.rs`, `app/tilepreview.rs` | A `Kind` per format, a thumbnail cache with a size limit, map tile pictures. Splats become one more `Kind` and one more thumbnail source. |
| Quick Look | `app/preview.rs`, `app/files.rs` | Space opens the enlarged preview for every kind. Splats plug in like any other kind. |
| Sequences | `sequences.rs`, `app/seqplayer.rs` | Image sequence detection and playback. PLY sequences reuse the detection. |
| Cloud safety | `cloud.rs`, scan flags `ONLINE_ONLY`, `NOT_LOCAL`, `MOUNT` | Tiles and previews already refuse to read files that aren't local. Splats follow the same rule. |

The ArtCraft thread may add `craft-mesh` (FBX, DAE, ABC). That is meshes, not splats, so the two don't overlap. Both should route through the same PLY and glTF sniffing described in section 5.

## 3. Renderer comparison

All checked on 26 Nov 2026 from the cloned repositories.

| | wgpu-3dgs-viewer | web-splat | Brush | wgpu-gs-viewer | SuperSplat viewer | PERF-GS |
|---|---|---|---|---|---|---|
| Who | Lio Qing | KeKsBoTer (TUM researchers) | Arthur Brussee | Abist Co., Ltd. | PlayCanvas | Electronic Arts SEED |
| Licence | MIT OR Apache-2.0 | Apache-2.0 | Apache-2.0 | MIT | MIT | BSD-3-Clause |
| What it is | A library, on crates.io | An app with a lib | A trainer and viewer app | An app | A web app (TypeScript, PlayCanvas engine) | A **trainer** (Python + Slang) |
| wgpu / egui | wgpu 30 (ours is 30.0.1) | wgpu 30, egui 0.37 (ours 0.36) | wgpu 30 from a fork, eframe 0.36, burn and CubeCL from git | wgpu 29 | WebGL2 / WebGPU | Slang via SlangPy, Vulkan or D3D12 |
| GPU sort | Radix sort in WGSL (BSD-2 from the `wgpu_sort` crate by the web-splat authors) | Same radix sort (they wrote it, ported from Fuchsia) | CubeCL radix sort | Global sort or tile mode | Web workers / GPU | GPUSorting (MIT) |
| Formats | PLY, SPZ v1–3 (v4 on git master only) | PLY, NPZ (c3dgs) | PLY, its own | PLY, SOG, Space-Time Gaussian Lite PLY | Everything splat-transform reads | Writes PLY |
| Display modes | Splat, ellipse, point; selection; per-Gaussian transforms; multi-model | Splat | Splat | Splat | Splat, rings, centres | n/a |
| Headless / off-screen | Yes: draws into any texture view (proved in the spike) | Possible | Possible | Possible | No (browser) | n/a |
| Activity | 10 releases from Feb 2025 to Aug 2026 (0.8.0, 23 Aug), master pushed 13 Oct, core 18 Nov; one maintainer | Pushed 23 Oct 2026 | Very active (22 Nov 2026), v1.0 | Pushed 18 Aug 2026 | Very active (26 Nov 2026) | Pushed 25 Nov 2026 |
| Fit for Filescape | **Best.** Same wgpu, small, modular, permissive | Good reference, but an app, and egui 0.37 | Too heavy (burn, CubeCL, forked wgpu); built for training | Useful reference for 4D (STG Lite) and SOG | Needs a WebView | Not a viewer. Its outputs are ordinary PLY files, which Filescape will recognise |

### Why not the others

- **Embedding SuperSplat.** Filescape has no browser engine. Adding one (wry: WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux) means a runtime dependency on Linux, different behaviour on each OS, and a JavaScript surface that reads local files. The brief says to use it only for capabilities native code can't reach; for viewing there are none. Editing is out of scope ("not a complete replacement for SuperSplat"), and "Open in SuperSplat" can stay an explicit external action.
- **Brush.** A great app, but its renderer lives inside a training stack (burn, CubeCL, a patched wgpu from git). Too much to carry for a preview.
- **web-splat.** Strong renderer from the authors of the sort that everyone reuses. It's an application with an egui 0.37 UI and isn't published as a library; wgpu-3dgs-viewer already packages the same sort as a library.
- **PERF-GS (Rich's link).** EA SEED's open trainer: "3D Gaussian Splatting in 10 seconds" on an RTX 4090, from COLMAP input, exporting `output.ply` under `outputs/train/<timestamp>/`. The code is BSD-3-Clause (LICENSE checked), and its sort is GPUSorting (MIT). Two caveats from its NOTICE.md: its optional benchmark patches carry the original Graphdeco (INRIA) non-commercial licence, and its "Ultrafast" preset uses NVIDIA research masks that are licensed separately. Filescape shouldn't train scenes, so we don't use it, but its output folder is one of the reconstruction layouts to recognise (stage S8).

### The INRIA question

The original `graphdeco-inria/gaussian-splatting` and its CUDA rasteriser are under the Gaussian-Splatting License: "Licensors grant non-exclusive rights to use the Software for research purposes … The Software may be used non-commercially". So no code, shaders or derived renderers from it can go into Filescape.

I checked the crates we'd adopt: no INRIA or Graphdeco code or notice in `wgpu-3dgs-viewer` or `wgpu-3dgs-core`. Their shaders are their own, except `radix_sort.wgsl`, which is BSD-2-Clause (Simon Niedermayr and Josef Stumpfegger, from `wgpu_sort`) and needs its notice kept. The PLY layout that INRIA's trainer writes is a file format, not code, and every tool reads it.

## 4. Recommended architecture

```
Files / Storage Analysis / Quick Look
        │ wants a picture of scene X at size W×H from camera C
        ▼
src/splat/mod.rs  ── the one door: open, describe, render, thumbnail, cancel
   ├─ detect.rs   PLY header classifier, multi-file scene manifests
   ├─ read/       our readers: ply, cply (compressed PLY), spz, splat, ksplat, sog …
   │              all stream into one compact Gaussian list, with hard limits
   ├─ facts.rs    counts, bounds, SH degree, bytes per Gaussian, quality checks
   ├─ frame.rs    automatic framing (see below)
   └─ gpu.rs      a worker thread that owns its own wgpu device:
                    wgpu-3dgs-viewer draws into an off-screen texture,
                    the pixels come back as an egui texture
```

**Its own device, off-screen.** On Windows the window is drawn with OpenGL, which can't run the compute shaders the sort needs, so splats can't be drawn inside eframe's pass there. A separate wgpu device (DX12 on Windows, Metal on macOS, Vulkan on Linux) can, whatever eframe uses. It also:
- lets us ask for the adapter's real limits. eframe's device uses wgpu's defaults, where one storage buffer can hold at most 128 MB, which is about one million Gaussians at half precision;
- keeps a GPU fault inside the splat worker: we wrap work in error scopes and recover from a lost device, and the file manager keeps running;
- is the same code for the preview pane, Quick Look and thumbnails.

The price is copying each frame back to the CPU: about 3.5 MB for a 1280×720 frame, and only while the camera moves. If that shows up in profiles, a later stage can share eframe's device on macOS and Linux (where eframe already uses wgpu) and skip the copy.

**Our own readers.** `wgpu-3dgs-core` 0.8 reads PLY and SPZ, but the spike found three reasons to feed it ourselves through its `Gaussians::Internal` list:
- SPZ v4 (the current version, "NGSP" with zstd) fails with "invalid gzip header" in 0.8.0; v4 support is only on its unreleased master.
- Two tiny SPZ v2/v3 test files from splat-transform made wgpu **panic** while making a bind group. In Filescape that would crash the app. Our layer has to validate before anything reaches the GPU, and catch errors.
- Neither its readers nor Niantic's reference reader put a ceiling on the point count ("There is no maximum point count enforced during loading", SPZ README). The brief rightly asks for checks before allocating.

**Automatic framing.** The first try framed the whole file and showed mostly blurry background: captures carry a shell of sky and distant floaters. Framing on the 10th to 90th percentile of positions fixed both test scenes. The real version will weight by opacity and size too.

**Memory per Gaussian on the GPU** (measured with `size_of` in the spike):

| Layout | Bytes | Gaussians in 128 MB | in 1 GB |
|---|---|---|---|
| Full precision, SH degree 3 | 224 | 0.6 M | 4.8 M |
| Half precision, SH 3 (our default) | 128 | 1.0 M | 8.4 M |
| 8-bit SH, half covariance | 80 | 1.7 M | 13.4 M |
| No SH (colour only), half | 32 | 4.2 M | 33.5 M |

Big scenes fall back down this list, then thin out (keeping the most visible Gaussians by opacity and size), and the viewer says so ("showing 3.0 M of 12.4 M Gaussians").

**Where splats appear.** A new preview `Kind::Splat`, a thumbnail source in `app/thumbs.rs` cached by `thumbcache.rs`, map tile pictures through `tilepreview.rs`, and Quick Look through the existing enlarged preview. One live viewer at a time; tiles only ever show cached pictures. Files that aren't local are never read (the existing `NOT_LOCAL`/`MOUNT` rule).

## 5. Format matrix (verified)

"Reference" is where the layout is defined. Nothing is marked supported until stage tests pass with real files.

| Format | Versions | How to recognise | Inside | Decoders needed | Thumbnail | Animation / streaming | Reference | Stage |
|---|---|---|---|---|---|---|---|---|
| PLY (3DGS) | ASCII and binary | Header has `f_dc_0..2`, `opacity`, `scale_*`, `rot_0..3`; optional `f_rest_*` (SH degree from their count); no `face` element | Floats per Gaussian (248 B at SH 3) | Our PLY reader | Render | No | INRIA trainer output; splat-transform `read-ply.ts` | S1 |
| PLY mesh / point cloud | | `face` element → mesh (stays with `model3d`); `x y z` + colour, no scale/rotation → point cloud | | Existing `model3d` reader (point clouds new) | Clay / points | No | PLY spec | S1 |
| Compressed PLY | PlayCanvas | `chunk` element (12 or 18 float properties) + `packed_position/rotation/scale/color` uint32 in `vertex`; optional `sh` element | 256-Gaussian chunks with min/max ranges | Our reader | Render | No | splat-transform `decompress-ply.ts` | S3 |
| SPZ | v1–3 gzip, v4 "NGSP" zstd | First bytes `1f 8b` (then "NGSP" magic inside) or `NGSP` | Separate streams per attribute; v3+ "smallest three" rotations; v4 has extensions (ILV records) | gzip (`flate2`, have it), zstd (`ruzstd`, pure Rust) | Render | No | nianticlabs/spz README (MIT) | S1 |
| SPLAT | antimatter15 | **No header.** Extension only, and size must be a multiple of 32 | 32 B: position 3×f32, scale 3×f32, RGBA u8, rotation u8×4; no SH | Our reader | Render | No | splat-transform `read-splat.ts` | S3 |
| KSPLAT | mkkellogg, major 0, minor ≥ 1 | 4096-byte main header, version bytes | Sections, compression levels 0–2, half floats, SH up to 2 | Our reader | Render | No | `read-ksplat.ts` | S3 |
| SOG (bundled `.sog`) | v1 (no version), v2, v3 | Zip with `meta.json` + WebP textures | Means split low/high, quantised quats and scales, SH palette (centroids + labels) | Our zip reader (have it) + lossless WebP (`image` crate, have it) | Render | No | `read-sog.ts`, PlayCanvas SOG spec | S3 |
| SOG unbundled | v2, v3 | `meta.json` with the SOG keys, next to the named `.webp` files | Same, as loose files | Same | Render | No | Same | S3 |
| Streamed SOG | | `lod-meta.json` + chunk folders | Multi-LOD chunks | Same + chunk loader | Render of a coarse LOD | Streaming | `read-lod.ts`, PlayCanvas spec | S9 |
| LCC (XGRIDS) | v1 | `meta.lcc` JSON + `index.bin`, `data.bin`, `shcoef.bin`, optional `environment.bin` | Quadtree units × LOD levels | Our reader | Coarse LOD | LOD streaming | XGRIDS LCC whitepaper (public; no licence file) + `read-lcc.ts` (MIT) | S9 |
| LCC2 | v2 | `meta.json` with N-ary tree | Tree of LOD nodes | Our reader | Coarse LOD | LOD streaming | `read-lcc2.ts` | S9 |
| GLB / glTF | `KHR_gaussian_splatting`, **ratified** by Khronos | `extensionsUsed` contains `KHR_gaussian_splatting` | Point primitive with Gaussian attributes | `gltf` crate (have it) + attribute mapping | Render | No | Khronos glTF repo (status: "Complete, Ratified"); splat-transform supports one primitive only | S3 |
| NPZ | c3dgs (KeKsBoTer) | Zip of `.npy` arrays with the c3dgs keys | Vector-quantised | `.npy` reader | Render | No | web-splat `io/` | Later, if users ask |
| PLY sequences | | Numbered PLYs in a folder, via `sequences.rs` | One scene per frame | As PLY | First frame | Frame playback | | S10 |
| 4D (Space-Time Gaussians Lite) | | PLY with time properties | Per-Gaussian motion and opacity over time | Our reader | One time | Yes | wgpu-gs-viewer (MIT) | S10, research |
| SuperSplat HTML export | | `.html` with embedded SOG | | **Never run its JS.** Find the embedded SOG with a safe parser | Render | No | splat-transform HTML writer | S3 |

## 6. Licences

| Component | Licence | Use | Notes |
|---|---|---|---|
| wgpu-3dgs-viewer, wgpu-3dgs-core 0.8 | MIT OR Apache-2.0 | Dependency | Keep notices; `radix_sort.wgsl` BSD-2-Clause notice too |
| 45 new transitive crates (glam, wesl, ply-rs, peg, logos, lexical …) | All MIT, Apache-2.0, BSD or Unlicense | Dependency | `packaging/licences.py` regenerates the notices page |
| splat-transform, SuperSplat, supersplat-viewer, PlayCanvas engine | MIT | Port readers and writers from splat-transform | Keep PlayCanvas's notice for ported code |
| Niantic spz | MIT | Reference for the SPZ reader; the two sample scenes for tests | Samples come with the MIT repo |
| web-splat | Apache-2.0 | Reference only | |
| Brush | Apache-2.0 | Reference; "Open in Brush" | |
| wgpu-gs-viewer | MIT | Reference for SOG and STG Lite | |
| PERF-GS | BSD-3-Clause | Not used | Optional patches carry INRIA's licence; NVIDIA masks licensed separately |
| graphdeco-inria/gaussian-splatting, diff-gaussian-rasterization | Research and non-commercial only | **Never** | Also check any future crate for copied INRIA code |
| XGRIDS LCC whitepaper | No licence stated | Read the spec; port from splat-transform (MIT) | "LCC" and "XGRIDS" as plain words only, no logos |
| Trademarks | | | SuperSplat, PlayCanvas, Niantic, XGRIDS, Postshot: plain-text names in "Open with" lists, no logos, no implied endorsement |

## 7. Footprint

Spike (release, stripped, Linux): a program with wgpu and PNG output was 4.8 MB; the same with wgpu-3dgs-viewer drawing a scene was 9.9 MB. So **about 5 MB at most**; Filescape already has some of those crates (memchr, regex-automata, aho-corasick, itertools), so the real increase should be less. For scale, the Windows download today is 23.8 MB zipped.

- Startup: nothing loads until a splat is shown. The device is created on first use and dropped after a minute with no splat on screen.
- Two dependencies we'd carry but not need: `ply-rs` (with the old `peg` 0.6) and the `wesl` shader linker that assembles shaders at run time. They are non-optional in `wgpu-3dgs-core`. If the 5 MB matters, stage S2 can measure the alternative: vendor the renderer (about 8,000 lines of Rust and WGSL across the two crates, before trimming what we don't use) with shaders linked at build time.
- Maintenance: one maintainer and about 10,000 downloads. The API changed between 0.8.0 and master (SH degree types). Pin `=0.8.0`, and if it stalls, vendoring is the fallback; the licence allows it.

## 8. GPU compatibility

| Platform | Backend | Expectation |
|---|---|---|
| Windows, NVIDIA / AMD / Intel | DX12 (own device; window stays on OpenGL) | Works; compute and storage buffers are standard on DX12 |
| Windows VM / Remote Desktop | DX12 WARP software adapter | Should work slowly; **to test** |
| macOS, Apple Silicon and Intel | Metal | Works; Metal allows large buffers |
| Linux | Vulkan; llvmpipe (software) in VMs | **Tested in the container on llvmpipe**: both scenes drew correctly |
| Old GPUs without compute | none | No interactive view. Show metadata and any embedded picture, and say why |

Before loading, check the adapter's `max_storage_buffer_binding_size` and `max_buffer_size` against the scene's bytes and pick a layout from the table in section 4.

## 9. The spike (prototype, item 7)

A 70-line program: read a scene, frame it, create a wgpu device with no window, draw into a texture with wgpu-3dgs-viewer, save a PNG. Run in the cloud container on llvmpipe (Mesa's software Vulkan, 4 CPU cores):

| Scene | Gaussians | File | Read | Upload + first frame (software GPU) |
|---|---|---|---|---|
| Niantic hornedlizard.spz (SPZ v2, SH 3) | 786,233 | 18.1 MB | 0.21 s | 1.85 s |
| Niantic racoonfamily.spz (SPZ v2, SH 3) | 932,560 | 24.2 MB | 0.27 s | 2.04 s |

Picture: `/mnt/project-files/gaussian-splats/spike-headless-render.png`. These are software-GPU numbers: they show the path works on a machine with no GPU, not how fast it will be. Real numbers (item 8) need Rich's Mac and Windows PC, in stage S2.

## 10. Free and Pro

The house rule is "make curiosity free, charge for productivity", and all previews are free. Decided with Rich on 26 Nov 2026: converting a single file is **free**, like freeing up one cloud file. Batch work and the productivity tools are Pro.

| Free | Pro |
|---|---|
| Recognising splats, map tile pictures, the preview pane, Quick Look, the full viewer | Batch conversion (many files at once) |
| All view modes (splat, point, ellipse, opacity, scale, density, depth) | Side-by-side comparison with synced cameras and image metrics |
| Scene facts and the single-scene quality report | Reconstruction project breakdowns and checkpoint timelines |
| The storage optimiser card with measured estimates for one file, and converting that one file | Storage reports across folders, optimiser recommendations in bulk |
| Open in external apps, camera viewpoints saved locally | 4D and animation inspection tools |

## 11. Roadmap

One PR per stage, each tested and merged on green CI, in this order:

| Stage | What | Size |
|---|---|---|
| **S1** | PLY classifier (mesh / point cloud / splat / 4D) so splat PLYs stop showing "no faces"; `src/splat` with our PLY and SPZ (v1–4) readers, hard limits, facts (count, SH degree, bounds, bytes per Gaussian, estimated GPU memory). Preview pane shows facts only. Fuzz tests on malformed files | Medium |
| **S2** | Renderer: own wgpu device worker, wgpu-3dgs-viewer pinned, off-screen render into the preview pane with orbit, pan, zoom, reset and framing; error scopes and device-lost recovery; budgets and fallback layouts. Benchmarks on Rich's Mac and Windows PC | Large |
| **S3** | More formats: compressed PLY, SPLAT, KSPLAT, SOG (bundled and loose), GLB with `KHR_gaussian_splatting`, SuperSplat HTML exports | Medium |
| **S4** | Thumbnails and map tiles: cached renders, format badge, count and size on larger tiles, hover orbit (off for big scenes). Quick Look with previous/next and screenshot | Medium |
| **S5** | Viewer modes and the inspector: point, ellipse, opacity, scale, density, depth, bounds; quality checks (floaters, huge or invisible Gaussians, bad numbers), highlight affected Gaussians; fly mode, FOV, ortho, full screen | Medium |
| **S6** | Storage optimiser card: measured size per target format (encode in memory, don't write), keep the original, write to a new file, reopen to verify. Writers for SPZ and compressed PLY first | Medium |
| **S7** | Comparison: two scenes side by side or split, synced cameras, facts diff | Medium |
| **S8** | Reconstruction projects: verified layouts (INRIA `point_cloud/iteration_N/point_cloud.ply` + `cameras.json` + `cfg_args`, COLMAP `sparse/0`, Nerfstudio `transforms.json`, PERF-GS `outputs/train/<time>/output.ply`, Postshot, Brush), checkpoint timeline. Never open `.pth`/`.pt` pickles (code can run); facts from file names and sizes only | Medium |
| **S9** | Large and streamed: streamed SOG, LCC, LCC2; LOD chunks with cloud-only chunks shown as such | Large |
| **S10** | Time: PLY sequences through `sequences.rs`, STG Lite 4D as a research spike | Medium |

Settings (a "Gaussian splats" section in Previews & Thumbnails) and Help Centre pages land with the stages that need them. The supported formats page (`formats.rs` → Help) is updated per stage.

**Test library.** The two Niantic SPZ scenes (MIT), the splat-transform fixtures (MIT), the wgpu-3dgs-core example PLY and SPZ, plus generated files: synthetic PLYs at 10k, 1M and 5M Gaussians, a mesh PLY and a point cloud PLY, truncated and corrupted copies, and a lying header that claims 2 billion Gaussians. Large real scenes (e.g. Mip-NeRF 360 trained outputs) have their own dataset licences; we'd use them only for local benchmarks, never bundled.

## 12. Decisions for Rich

1. Approve wgpu-3dgs-viewer with our own readers, on its own device (recommended), over vendoring a renderer from day one.
2. Confirm the free and Pro split in section 10, especially whether converting a single file is free.
3. Approve stages S1 and S2 to start; the rest wait for his look at S2 running on his Mac and PC.
