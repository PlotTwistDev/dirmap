//! Previews: Quick Look (Space) and the preview pane (Alt+P) show the item under the cursor.
//!
//! Each format has a provider: images (PNG, JPEG, GIF, WebP, TIFF, BMP, TGA, EXR, HDR…),
//! text and source code, and a fallback of facts. Files are read on a worker thread with hard
//! limits, so a huge or malformed file can't freeze or exhaust dirmap: images over 256 MB
//! aren't decoded, decoders are capped at 512 MB and 16k×16k, text reads stop at 256 KB, and
//! what's shown is scaled to at most 2048 px.

use crate::fsys::PathShow;
use super::*;
use eframe::egui::Vec2;
use crate::fsys::listing::Entry;
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};

const IMAGE_MAX_FILE: u64 = 256 << 20;
const TEXT_MAX: usize = 256 << 10;
const SHOW_MAX: u32 = 2048;
const CACHE: usize = 8;

/// What a file decoded to.
pub(super) enum Content {
    Image {
        pixels: Option<egui::ColorImage>,
        tex: Option<egui::TextureHandle>,
        /// Full size, and a line about the pixels ("RGBA, 16 bits per channel").
        size: [u32; 2],
        info: String,
        /// A picture the system made of a video or document (its size isn't the file's).
        poster: bool,
        /// A drawing's source (SVG), shown instead when asked; its size is the drawing's own
        /// and it can be shown larger than that.
        source: Option<(String, bool)>,
        /// More about it for the details (a photo's camera and exposure).
        facts: Vec<(&'static str, String)>,
        /// Its channels and layers, to look at one by one (pictures only).
        look: Option<Box<ImageLook>>,
    },
    /// Sound: played with the controls in the preview, from this file (a fetched copy for one
    /// on a server).
    Sound {
        play: PathBuf,
        /// Tags, format and cover, from the file.
        info: crate::soundinfo::SoundInfo,
        /// The waveform, if the system could decode it.
        peaks: Option<Arc<[f32]>>,
        cover: Option<egui::TextureHandle>,
    },
    Text {
        text: String,
        truncated: bool,
        /// Syntax colours, for code.
        colours: Option<Arc<crate::highlight::Colours>>,
    },
    /// A spreadsheet's cells.
    Sheet(Arc<crate::sheets::Book>),
    /// A Word, PowerPoint, OpenDocument or EPUB file, formatted.
    Doc { blocks: Arc<Vec<crate::help::doc::Block>>, facts: Vec<(&'static str, String)> },
    /// A PDF: every page, drawn as it comes into view.
    Pdf(Arc<PdfShown>),
    /// An archive: what's in it, as a tree with sizes.
    Archive(Arc<crate::archtree::Tree>),
    /// A 3D model, drawn from the angle it's turned to.
    Model { mesh: Arc<crate::model3d::Mesh>, tex: Option<egui::TextureHandle>, drawn: Option<(crate::model3d::Angle, [usize; 2], usize)> },
    /// A Gaussian splat scene or point cloud, drawn as points from the angle it's turned to.
    Splat { scene: Arc<crate::splat::Scene>, tex: Option<egui::TextureHandle>, drawn: SplatDrawn },
    Binary,
    /// Nothing to show beyond the facts.
    None,
    Error(String),
}

impl Content {
    /// More about the file for the details (a photo's camera, a PDF's title and pages).
    pub(super) fn facts(&self) -> &[(&'static str, String)] {
        match self {
            Content::Image { facts, .. } => facts,
            Content::Pdf(p) => &p.doc.facts,
            Content::Doc { facts, .. } => facts,
            Content::Sheet(b) => &b.facts,
            Content::Archive(t) => &t.facts,
            Content::Model { mesh, .. } => &mesh.facts,
            Content::Splat { scene, .. } => &scene.facts,
            _ => &[],
        }
    }
}

/// A PDF being shown, with its pages as they're drawn.
pub(super) struct PdfShown {
    doc: crate::pdfview::Doc,
    pages: std::sync::Mutex<HashMap<usize, PageSlot>>,
}

enum PageSlot {
    Drawing,
    /// Drawn this many pixels across; `again` while it's drawn again at a new size (it's shown
    /// as it was until then).
    Ready { width: u32, pixels: Option<egui::ColorImage>, tex: Option<egui::TextureHandle>, again: bool },
    Failed,
}

/// What Quick Look and the preview pane are showing.
#[derive(Default)]
pub(super) struct Look {
    /// Quick Look is open.
    pub open: bool,
    /// Quick Look shows the tile picked in Storage Analysis, not the File Manager's cursor.
    pub from_map: bool,
    /// Quick Look fills the window.
    pub full: bool,
    /// Recently decoded files: (path, modified, content).
    cache: VecDeque<(PathBuf, i64, Content)>,
    /// Being read: (path, modified, result).
    pending: Option<(PathBuf, i64, mpsc::Receiver<Content>)>,
    /// Image view: None fits the window; Some(scale) is 1.0 for actual pixels.
    pub zoom: Option<f32>,
    pub pan: Vec2,
    /// The file the zoom and pan are for.
    view_of: Option<PathBuf>,
    /// A sound playing (or paused) in the preview.
    pub player: Option<crate::media::Player>,
    /// Cloud files the user agreed to download to preview (others aren't read: see `preview_body`).
    pub download_ok: std::collections::HashSet<PathBuf>,
    /// A video playing in the preview.
    pub video: Option<VideoView>,
    /// Frames from across the video being previewed.
    strip: Option<StripView>,
}

/// A video's filmstrip: being read, then its frames.
struct StripView {
    path: PathBuf,
    rx: Option<mpsc::Receiver<Result<(crate::video::Info, Vec<crate::video::Frame>), String>>>,
    frames: Vec<(f64, egui::TextureHandle)>,
    /// The video's length, from the filmstrip's reading of it.
    length: Option<f64>,
    /// Its sound's shape and the seconds that covers, read after the frames (`None` while
    /// reading, `Some(None)` when it has no sound).
    wave_rx: Option<mpsc::Receiver<Option<(Vec<f32>, f64)>>>,
    wave: Option<Option<(Vec<f32>, f64)>>,
}

/// The filmstrip's height, and the sound's under it.
const STRIP_H: f32 = 56.0;
const STRIP_WAVE_H: f32 = 30.0;
/// Stretches in a video's sound on its timeline.
const STRIP_WAVE_BARS: usize = 600;

/// A video playing in a preview, and its picture.
pub(super) struct VideoView {
    pub video: crate::video::Video,
    tex: Option<egui::TextureHandle>,
    serial: u64,
    /// While the time slider is dragged: whether it was playing before, and where it's at.
    scrub: Option<(bool, f64)>,
}

impl Look {
    /// What a file decoded to, if it's in the cache.
    pub(super) fn cached(&self, path: &Path) -> Option<&Content> {
        self.cache.iter().find(|c| c.0 == path).map(|c| &c.2)
    }
}

/// The kind of preview a file gets, from its extension.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Kind {
    Image,
    /// Linear-light images, shown through an sRGB display transform.
    LinearImage,
    Text,
    /// SVG: drawn, with the source a click away.
    Drawing,
    /// A font: a specimen.
    Font,
    /// Camera RAW: the camera's own preview inside it.
    Raw,
    /// Photoshop: the flattened picture.
    Psd,
    /// A spreadsheet or CSV file: its cells.
    Sheet,
    /// A SQLite database: its tables.
    Database,
    /// A 3D model: drawn, to turn about.
    Model,
    /// A Gaussian splat scene: drawn as points, to turn about.
    Splat,
    /// Word, PowerPoint or OpenDocument: formatted.
    Doc,
    /// MIDI: a piano roll.
    Midi,
    /// GeoJSON, GPX or KML: the shapes drawn.
    Map,
    /// DXF drawn; DWG, STEP and IGES described.
    Cad,
    /// An app's own format: the picture it saved inside.
    Saved,
    /// DICOM or FITS: decoded here.
    Science,
    DiskImage,
    /// Video, sound or PDF: a picture or sound from the system.
    Media(crate::media::Media),
    Other,
}

pub(super) fn kind_of(name: &str) -> Kind {
    crate::formats::by_name(name).map_or(Kind::Other, kind_for)
}

/// The kind of preview a format gets.
pub(super) fn kind_for(f: &crate::formats::Format) -> Kind {
    use crate::formats::Category as C;
    use crate::media::Media;
    match f.category {
        C::Picture => Kind::Image,
        C::LinearPicture => Kind::LinearImage,
        C::Text => Kind::Text,
        C::Drawing => Kind::Drawing,
        C::Font => Kind::Font,
        C::Raw => Kind::Raw,
        C::Photoshop => Kind::Psd,
        C::Spreadsheet => Kind::Sheet,
        C::Video => Kind::Media(Media::Video),
        C::Audio => Kind::Media(Media::Audio),
        C::Pdf => Kind::Media(Media::Pdf),
        C::SystemPicture => Kind::Media(Media::Picture),
        C::Document => Kind::Doc,
        C::Midi => Kind::Midi,
        C::Map => Kind::Map,
        C::Cad => Kind::Cad,
        C::Saved => Kind::Saved,
        C::Science => Kind::Science,
        C::DiskImage => Kind::DiskImage,
        C::Database => Kind::Database,
        C::Model => Kind::Model,
        C::Splat => Kind::Splat,
    }
}

/// Files that may hold a splat scene though their kind says otherwise: PLY (mesh or
/// splats), GLB (model or splats), web pages (SuperSplat exports) and a loose SOG scene's
/// meta.json.
fn may_be_splat(path: &Path) -> bool {
    let ext = path.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "ply" | "glb" | "html" | "htm") || path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("meta.json"))
}

fn load_splat(path: &Path) -> Content {
    match crate::splat::read(path, crate::splat::Limits::PREVIEW) {
        Ok(scene) => Content::Splat { scene: Arc::new(scene), tex: None, drawn: SplatDrawn::default() },
        Err(e) => Content::Error(e),
    }
}

/// Read a file for its preview. Runs on a worker thread.
fn load(path: &Path, kind: Kind, size: u64) -> Content {
    // A LightCraft library: a folder, shown as one.
    if !crate::fsys::remote::is_remote(path) && path.is_dir() && crate::lightcraft::is_library(path) {
        return match crate::lightcraft::read(path) {
            Ok((blocks, facts)) => Content::Doc { blocks: Arc::new(blocks), facts },
            Err(e) => Content::Error(e),
        };
    }
    // A Gaussian splat training project or capture: a folder, shown as one.
    if !crate::fsys::remote::is_remote(path) && path.is_dir() {
        if let Some(p) = crate::splat::project::read(path) {
            let (blocks, facts) = crate::splat::project::describe(path, &p);
            return Content::Doc { blocks: Arc::new(blocks), facts };
        }
    }
    // On a server: fetched to a temporary copy first (not huge ones).
    if crate::fsys::remote::is_remote(path) {
        let archive = path.file_name().is_some_and(|n| crate::fsys::archive::is_archive(&n.to_string_lossy()));
        let limit = match kind {
            Kind::Other if !archive => 1 << 20,
            Kind::Media(crate::media::Media::Video) => 64 << 20,
            _ => IMAGE_MAX_FILE,
        };
        if size > limit {
            return Content::Error(format!("Too big to preview from the server ({})", crate::scan::human(size)));
        }
        return match crate::fsys::remote::fetch(path, limit) {
            Ok(tmp) => load(&tmp, kind, size),
            Err(e) => Content::Error(format!("Couldn't fetch it from the server: {}", crate::fsys::remote::describe(&e))),
        };
    }
    // Inside an archive: taken out to a temporary file first (not huge ones).
    if crate::fsys::archive::inside(path) {
        if size > IMAGE_MAX_FILE {
            return Content::Error("Too big to preview from inside the archive".into());
        }
        return match files::archive_temp(path) {
            Ok(tmp) => {
                let c = load(&tmp, kind, size);
                if let Some(d) = tmp.parent() {
                    let _ = std::fs::remove_dir_all(d);
                }
                c
            }
            Err(e) => Content::Error(format!("Couldn't read it from the archive: {e}")),
        };
    }
    // An archive: what's in it.
    if path.file_name().is_some_and(|n| crate::fsys::archive::is_archive(&n.to_string_lossy())) {
        return match crate::fsys::archive::index(path) {
            Ok(ix) => Content::Archive(Arc::new(crate::archtree::Tree::build(&ix, path, size))),
            Err(e) => Content::Error(e),
        };
    }
    if kind != Kind::Splat && may_be_splat(path) && crate::splat::claims(path) {
        return load_splat(path);
    }
    match kind {
        Kind::Image | Kind::LinearImage => load_image(path, kind == Kind::LinearImage, size),
        Kind::Text => match load_text(path) {
            Content::Text { text, truncated, .. } => coloured(path, text, truncated),
            c => c,
        },
        Kind::Doc => load_doc(path),
        Kind::Sheet => match crate::sheets::read(path, crate::sheets::ROWS, crate::sheets::COLS) {
            Ok(b) => Content::Sheet(Arc::new(b)),
            Err(e) => Content::Error(e),
        },
        // PLY files can hold meshes, point clouds or splat scenes: the header says which.
        Kind::Splat => load_splat(path),
        Kind::Model => match crate::model3d::read(path) {
            Ok(mesh) => Content::Model { mesh: Arc::new(mesh), tex: None, drawn: None },
            // A Windows object file (.obj) and the like: shown as any other file.
            Err(e) if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("obj")) && e.contains("object file") => Content::Binary,
            Err(e) => Content::Error(e),
        },
        Kind::Database => match crate::tables::read_sqlite(path, crate::sheets::ROWS, crate::sheets::COLS, true) {
            Ok(b) => Content::Sheet(Arc::new(b)),
            Err(e) => Content::Error(e),
        },
        Kind::Psd => match crate::psdview::preview(path) {
            Ok((p, layers)) => {
                let img = image::DynamicImage::ImageRgba8(p.image);
                let img = if img.width() > SHOW_MAX || img.height() > SHOW_MAX { img.thumbnail(SHOW_MAX, SHOW_MAX) } else { img };
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                Content::Image {
                    look: None,
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)),
                    tex: None,
                    size: p.size,
                    info: if p.thumbnail_only { "Photoshop's small thumbnail (the document is too large to show in full)".into() } else { "The flattened picture".into() },
                    poster: false,
                    source: None,
                    facts: layers,
                }
            }
            Err(e) => Content::Error(e),
        },
        Kind::Raw => match crate::camraw::preview(path, 0) {
            Ok(r) => {
                let img = if r.image.width() > SHOW_MAX || r.image.height() > SHOW_MAX { r.image.thumbnail(SHOW_MAX, SHOW_MAX) } else { r.image };
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                Content::Image {
                    look: None,
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)),
                    tex: None,
                    size: [w, h],
                    info: "The camera's preview".into(),
                    poster: true,
                    source: None,
                    facts: r.facts,
                }
            }
            Err(e) => Content::Error(e),
        },
        Kind::Midi => match crate::midiview::read(path) {
            Ok(song) => {
                let (w, h) = (1200, 400);
                Content::Image {
                    look: None,
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &song.roll(w, h))),
                    tex: None,
                    size: [w, h],
                    info: "Every note against time (across) and pitch (up), coloured by channel".into(),
                    poster: true,
                    source: None,
                    facts: song.facts,
                }
            }
            Err(e) => Content::Error(e),
        },
        Kind::Cad if crate::cadview::is_dxf(path) || crate::cadview::is_dwg(path) => match if crate::cadview::is_dwg(path) { crate::cadview::read_dwg(path) } else { crate::cadview::read_dxf(path) } {
            Ok(d) => {
                let (w, h) = d.fit(1400);
                Content::Image {
                    look: None,
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &d.draw(w, h))),
                    tex: None,
                    size: [w, h],
                    info: "Drawn from the file's lines and curves; text isn't drawn".into(),
                    poster: true,
                    source: None,
                    facts: d.facts,
                }
            }
            // A DWG that couldn't be converted still says what it is.
            Err(e) => match crate::cadview::is_dwg(path).then(|| crate::cadview::read_info(path)).and_then(Result::ok) {
                Some(i) => Content::Doc { blocks: Arc::new(i.blocks), facts: i.facts },
                None => Content::Error(e),
            },
        },
        Kind::Cad => match crate::cadview::read_info(path) {
            Ok(i) => Content::Doc { blocks: Arc::new(i.blocks), facts: i.facts },
            Err(e) => Content::Error(e),
        },
        Kind::DiskImage => match crate::diskimage::read(path) {
            Ok(crate::diskimage::Disk::Listing { index, facts }) => {
                let mut tree = crate::archtree::Tree::build(&index, path, size);
                // A disc isn't compressed: keep the count and the largest file, with the disc's own details first.
                let kept: Vec<_> = tree.facts.drain(..).filter(|f| matches!(f.0, "Contents" | "Largest")).collect();
                tree.facts = facts;
                tree.facts.extend(kept);
                Content::Archive(Arc::new(tree))
            }
            Ok(crate::diskimage::Disk::Details { blocks, facts }) => Content::Doc { blocks: Arc::new(blocks), facts },
            Err(e) => Content::Error(e),
        },
        Kind::Science => match crate::sciimage::read(path) {
            Ok(s) => Content::Image {
                look: None,
                pixels: Some(egui::ColorImage::from_rgba_unmultiplied([s.width as usize, s.height as usize], &s.rgba)),
                tex: None,
                size: [s.width, s.height],
                info: String::new(),
                poster: false,
                source: None,
                facts: s.facts,
            },
            Err(e) => Content::Error(e),
        },
        Kind::Saved => match crate::savedpic::read(path) {
            Ok(s) => match s.picture {
                Some(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    let app = s.facts.first().map(|f| f.1.clone()).unwrap_or_default();
                    Content::Image {
                        look: None,
                        pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)),
                        tex: None,
                        size: [w, h],
                        info: format!("The picture {app} saved inside the file"),
                        poster: true,
                        source: None,
                        facts: s.facts,
                    }
                }
                None => {
                    use crate::help::doc::{Block, Span};
                    let plain = |t: String| vec![Span { text: t, ..Default::default() }];
                    let app = s.facts.first().map(|f| f.1.clone()).unwrap_or_default();
                    let mut blocks = vec![Block::Heading { level: 1, spans: plain(format!("{app} file")), id: String::new() }];
                    for (k, v) in s.facts.iter().skip(1) {
                        blocks.push(Block::Para(vec![Span { text: format!("{k}: "), bold: true, ..Default::default() }, Span { text: v.clone(), ..Default::default() }]));
                    }
                    blocks.push(Block::Rule);
                    let note = s.note.clone().unwrap_or_else(|| format!("There's no picture saved inside it, and Filescape can't draw {app} files itself."));
                    blocks.push(Block::Para(plain(note)));
                    if let Some(r) = &s.refs {
                        blocks.extend(r.blocks());
                    }
                    Content::Doc { blocks: Arc::new(blocks), facts: s.facts }
                }
            },
            Err(e) => Content::Error(e),
        },
        Kind::Map => match crate::geoview::read(path) {
            Ok(g) => {
                let (w, h) = g.fit(1200);
                Content::Image {
                    look: None,
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &g.draw(w, h))),
                    tex: None,
                    size: [w, h],
                    info: "Drawn from the file: no map is fetched to go behind it".into(),
                    poster: true,
                    source: None,
                    facts: g.facts,
                }
            }
            Err(e) => Content::Error(e),
        },
        Kind::Font => match crate::fontview::specimen(path, 800) {
            Ok(s) => Content::Image {
                look: None,
                pixels: Some(egui::ColorImage::from_rgba_unmultiplied([s.width as usize, s.height as usize], &s.rgba)),
                tex: None,
                size: [s.width, s.height],
                info: s.about.line(),
                poster: true,
                source: None,
                facts: Vec::new(),
            },
            Err(e) => Content::Error(e),
        },
        Kind::Drawing => match crate::svg::render(path, SHOW_MAX.min(1600)) {
            Ok(d) => {
                let source = match load_text(path) {
                    Content::Text { text, truncated, .. } => Some((text, truncated)),
                    _ => None, // .svgz: compressed, no source to show
                };
                Content::Image {
                    look: None,
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([d.width as usize, d.height as usize], &d.rgba)),
                    tex: None,
                    size: [d.natural[0].round().max(1.0) as u32, d.natural[1].round().max(1.0) as u32],
                    info: "Vector drawing (sharp at any size)".into(),
                    poster: false,
                    source: source.or(Some((String::new(), false))),
                    facts: Vec::new(),
                }
            }
            // Not a drawing resvg can read: show the markup.
            Err(_) => load_text(path),
        },
        Kind::Media(crate::media::Media::Audio) => Content::Sound {
            play: path.to_path_buf(),
            info: crate::soundinfo::read(path),
            peaks: crate::wave::peaks(path, WAVE_BARS).ok().map(Into::into),
            cover: None,
        },
        // Every page, drawn here; the system's picture of the first page when it can't be.
        Kind::Media(crate::media::Media::Pdf) => match crate::pdfview::open(path) {
            Ok(doc) => Content::Pdf(Arc::new(PdfShown { doc, pages: Default::default() })),
            Err(e) => match crate::media::thumbnail(path, 1024) {
                Ok((w, h, rgba)) => Content::Image {
                    look: None,
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)),
                    tex: None,
                    size: [w as u32, h as u32],
                    info: "The first page, drawn by the system".into(),
                    poster: true,
                    source: None,
                    facts: Vec::new(),
                },
                Err(_) => Content::Error(e),
            },
        },
        Kind::Media(m) => match if m == crate::media::Media::Video { crate::video::poster(path, 1024) } else { crate::media::thumbnail(path, 1024) } {
            Ok((w, h, rgba)) => Content::Image {
                look: None,
                pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)),
                tex: None,
                size: [w as u32, h as u32],
                info: match m {
                    crate::media::Media::Pdf => "The first page".into(),
                    crate::media::Media::Picture => "Drawn by the system".into(),
                    _ => "A frame from the video".into(),
                },
                poster: true,
                source: None,
                facts: Vec::new(),
            },
            Err(_) => Content::Error(match m {
                crate::media::Media::Pdf => "No picture of this PDF: it couldn't be drawn (it may be damaged or password-protected, or the system can't draw PDFs)".into(),
                crate::media::Media::Picture => {
                    if cfg!(windows) {
                        "The system can't draw this picture. Windows needs the free HEIF and AV1 Video Extensions from the Microsoft Store for HEIC and AVIF.".into()
                    } else if cfg!(target_os = "macos") {
                        "The system couldn't draw this picture (it may be damaged).".into()
                    } else {
                        "No program here can draw this picture: install libheif's heif-thumbnailer (for HEIC) or ffmpeg (for AVIF).".into()
                    }
                }
                _ => "No picture of this video: the system has no thumbnailer for it".into(),
            }),
        },
        Kind::Other => {
            // Unknown or missing extension: its first bytes may say what it is.
            if let Some(k) = sniffed(path).filter(|k| *k != Kind::Other) {
                return load(path, k, size);
            }
            // Otherwise show it as text if it looks like text.
            match load_text(path) {
                Content::Text { text, truncated, .. } if size <= TEXT_MAX as u64 * 4 => coloured(path, text, truncated),
                Content::Error(e) => Content::Error(e),
                _ => Content::None,
            }
        }
    }
}

/// The kind a file's first bytes say it is, for a file whose name doesn't say.
fn sniffed(path: &Path) -> Option<Kind> {
    use std::io::Read;
    let mut head = [0u8; 32];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    crate::formats::sniff(&head[..n]).map(kind_for)
}

fn load_image(path: &Path, linear: bool, size: u64) -> Content {
    use crate::imagelook::{Planes, View};
    let picture = |planes: Planes, full: [u32; 2], info: String, facts: Vec<(&'static str, String)>| {
        let (w, h, rgba) = planes.render(&View::default());
        Content::Image {
            pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)),
            tex: None,
            size: full,
            info,
            poster: false,
            source: None,
            facts,
            look: Some(Box::new(ImageLook { planes, view: View::default(), drawn: View::default() })),
        }
    };
    // OpenEXR: every layer (render passes), not only the main picture.
    let exr = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exr"));
    if exr && size <= IMAGE_MAX_FILE
        && let Ok(planes) = crate::imagelook::Planes::read_exr(path, SHOW_MAX as usize)
    {
        let full = planes.layers[0].full;
        let n = planes.layers.len();
        let mut facts = Vec::new();
        if n > 1 {
            facts.push(("Layers", planes.layers.iter().map(|l| l.label()).collect::<Vec<_>>().join(", ")));
        }
        let info = format!("{} · linear, shown as sRGB", if n == 1 { "1 layer".to_string() } else { format!("{n} layers") });
        return picture(planes, full, info, facts);
    }
    match open_image(path, size) {
        Ok(img) => {
            let full = [img.width(), img.height()];
            let info = describe(img.color());
            let planes = Planes::from_image(&img, linear, SHOW_MAX as usize);
            let info = if planes.linear { format!("{info} · linear, shown as sRGB") } else { info };
            picture(planes, full, info, Vec::new())
        }
        Err(e) => Content::Error(e),
    }
}

/// A picture's channels and layers, and which of them is showing.
pub(super) struct ImageLook {
    planes: crate::imagelook::Planes,
    view: crate::imagelook::View,
    /// The view the texture was drawn for.
    drawn: crate::imagelook::View,
}

/// The row over a picture: which layer, which channel, and (for high dynamic range pictures)
/// the exposure.
fn image_controls(ui: &mut egui::Ui, path: &Path, lk: &mut ImageLook) {
    let p = &lk.planes;
    let v = &mut lk.view;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if p.layers.len() > 1 {
            let mut layer = v.layer;
            egui::ComboBox::from_id_salt(("look-layer", path)).selected_text(p.layers[v.layer].label()).width(150.0).show_ui(ui, |ui| {
                for (i, l) in p.layers.iter().enumerate() {
                    ui.selectable_value(&mut layer, i, l.label());
                }
            });
            if layer != v.layer {
                v.layer = layer;
                v.channel = None;
            }
        }
        let l = &p.layers[v.layer];
        let colour = l.channels.len() > 1;
        if colour {
            ui.selectable_value(&mut v.channel, None, "Colour").on_hover_text("All channels together");
        }
        for (i, (name, _)) in l.channels.iter().enumerate() {
            let tip = match name.to_ascii_uppercase().as_str() {
                "R" => "Red only".to_string(),
                "G" => "Green only".to_string(),
                "B" => "Blue only".to_string(),
                "A" => "Transparency (alpha): white is solid".to_string(),
                "Z" => "Depth: near is dark, far is light".to_string(),
                _ => format!("The {name} channel only"),
            };
            if colour {
                ui.selectable_value(&mut v.channel, Some(i), name.as_str()).on_hover_text(tip);
            }
        }
        if p.linear {
            ui.add_space(6.0);
            ui.add(egui::Slider::new(&mut v.exposure, -8.0..=8.0).step_by(0.25).text("stops").fixed_decimals(2))
                .on_hover_text("Exposure: each stop doubles or halves the light, to see into the highlights and shadows");
            if v.exposure != 0.0 && ui.small_button("Reset").clicked() {
                v.exposure = 0.0;
            }
        }
    });
    ui.add_space(2.0);
}

/// Open an image file, refusing big files and huge dimensions rather than risk the memory.
fn open_image(path: &Path, size: u64) -> Result<image::DynamicImage, String> {
    if size > IMAGE_MAX_FILE {
        return Err(format!("Too big to preview ({}).", crate::scan::human(size)));
    }
    // JPEG XL is read through a hook (done once; also at start-up).
    jxl_oxide::integration::register_image_decoding_hook();
    // JPEG 2000 is decoded here (hayro-jpeg2000), its size checked before anything is decoded.
    if path.extension().is_some_and(|e| matches!(e.to_string_lossy().to_ascii_lowercase().as_str(), "jp2" | "j2k" | "j2c" | "jpc" | "jpf" | "jpg2"))
        || std::fs::File::open(path).ok().and_then(|mut f| {
            let mut head = [0u8; 12];
            std::io::Read::read_exact(&mut f, &mut head).ok()?;
            crate::formats::sniff(&head).filter(|f| f.id == "jp2")
        }).is_some()
    {
        let bytes = std::fs::read(path).map_err(|e| format!("Couldn't open it: {e}"))?;
        return crate::sciimage::jpeg2000(&bytes, 16_384).map(image::DynamicImage::ImageRgba8);
    }
    let mut reader = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).map_err(|e| format!("Couldn't open it: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(512 << 20);
    reader.limits(limits);
    let jpeg = reader.format() == Some(image::ImageFormat::Jpeg);
    match reader.decode() {
        Ok(i) => Ok(i),
        Err(image::ImageError::Limits(_)) => Err("Too large to preview safely.".into()),
        // 12-bit and arithmetic-coded JPEGs, which the usual decoder can't read.
        Err(e) => {
            if jpeg && let Ok(b) = std::fs::read(path) && crate::jpegx::needs(&b) {
                return crate::jpegx::decode(&b);
            }
            Err(format!("Couldn't read the image: {e}"))
        }
    }
}

/// Decode an image file for display, no larger than `max` pixels across: the pixels, the full
/// size and a description of the pixel format. Linear-light and float images go through the sRGB
/// curve. Runs on worker threads (thumbnails).
pub(super) fn decode_image(path: &Path, linear: bool, size: u64, max: u32) -> Result<(egui::ColorImage, [u32; 2], String), String> {
    let img = open_image(path, size)?;
    let (w, h) = (img.width(), img.height());
    let info = describe(img.color());
    let small = if w > max || h > max { img.thumbnail(max, max) } else { img };
    let pixels = if linear || matches!(small.color(), image::ColorType::Rgb32F | image::ColorType::Rgba32F) {
        // Scene-linear floats: through the sRGB transfer curve for display.
        let f = small.to_rgba32f();
        let px: Vec<Color32> = f
            .pixels()
            .map(|p| {
                let c = |v: f32| (srgb(v) * 255.0).round().clamp(0.0, 255.0) as u8;
                Color32::from_rgba_unmultiplied(c(p[0]), c(p[1]), c(p[2]), (p[3].clamp(0.0, 1.0) * 255.0).round() as u8)
            })
            .collect();
        egui::ColorImage::new([f.width() as usize, f.height() as usize], px)
    } else {
        let rgba = small.to_rgba8();
        egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
    };
    let info = if linear { format!("{info} · linear, shown as sRGB") } else { info };
    Ok((pixels, [w, h], info))
}

/// Linear light to the sRGB display curve.
fn srgb(v: f32) -> f32 {
    crate::imagelook::srgb(v)
}

fn describe(c: image::ColorType) -> String {
    use image::ColorType as C;
    let (ch, bits) = match c {
        C::L8 => ("Grey", "8-bit"),
        C::La8 => ("Grey + alpha", "8-bit"),
        C::Rgb8 => ("RGB", "8-bit"),
        C::Rgba8 => ("RGBA", "8-bit"),
        C::L16 => ("Grey", "16-bit"),
        C::La16 => ("Grey + alpha", "16-bit"),
        C::Rgb16 => ("RGB", "16-bit"),
        C::Rgba16 => ("RGBA", "16-bit"),
        C::Rgb32F => ("RGB", "32-bit float"),
        C::Rgba32F => ("RGBA", "32-bit float"),
        _ => ("Colour", "other"),
    };
    format!("{ch}, {bits}")
}

fn load_text(path: &Path) -> Content {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return Content::Error(crate::fsys::listing::describe(&e)),
    };
    let mut buf = Vec::with_capacity(16 << 10);
    if let Err(e) = (&mut f).take(TEXT_MAX as u64 + 1).read_to_end(&mut buf) {
        return Content::Error(crate::fsys::listing::describe(&e));
    }
    let truncated = buf.len() > TEXT_MAX;
    buf.truncate(TEXT_MAX);
    let Some(text) = decode_text(&buf) else {
        return Content::Binary;
    };
    Content::Text { text, truncated, colours: None }
}

/// Text, with syntax colours if it's code (not Markdown, which has its own view).
fn coloured(path: &Path, text: String, truncated: bool) -> Content {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let colours = if is_markdown(&name) { None } else { crate::highlight::colour(&name, &text).map(Arc::new) };
    Content::Text { text, truncated, colours }
}

/// The text in `b`: UTF-8, or UTF-16 (what PowerShell and many Windows tools write), with or
/// without a byte-order mark. None if it looks binary.
pub(super) fn decode_text(b: &[u8]) -> Option<String> {
    let utf16 = |le: bool, b: &[u8]| -> String {
        let units = b.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) });
        char::decode_utf16(units).map(|c| c.unwrap_or('\u{fffd}')).collect()
    };
    let text = if let Some(rest) = b.strip_prefix(&[0xff, 0xfe]) {
        utf16(true, rest)
    } else if let Some(rest) = b.strip_prefix(&[0xfe, 0xff]) {
        utf16(false, rest)
    } else if !is_binary(b) {
        let t = String::from_utf8_lossy(b);
        t.strip_prefix('\u{feff}').unwrap_or(&t).to_string()
    } else {
        // No mark: UTF-16 if nearly every other byte is zero and the rest aren't.
        let head = &b[..b.len().min(8192) & !1];
        let (even, odd) = head.chunks_exact(2).fold((0, 0), |(e, o), c| (e + (c[0] == 0) as usize, o + (c[1] == 0) as usize));
        let n = head.len() / 2;
        if n >= 2 && odd * 10 >= n * 9 && even * 10 < n {
            utf16(true, b)
        } else if n >= 2 && even * 10 >= n * 9 && odd * 10 < n {
            utf16(false, b)
        } else {
            return None;
        }
    };
    // Control characters other than tabs and line ends mean it wasn't text after all.
    (!text.chars().take(4096).any(|c| c == '\0')).then_some(text)
}

/// NUL bytes near the start, or UTF-16 without a BOM, mean it isn't text we can show.
pub(super) fn is_binary(b: &[u8]) -> bool {
    b.iter().take(8192).any(|&c| c == 0)
}

impl Look {
    /// The content for a file, starting to read it if needed. None while it's being read.
    fn get(&mut self, ctx: &egui::Context, path: &Path, e: &Entry) -> Option<&mut Content> {
        if let Some((p, m, rx)) = &self.pending {
            match rx.try_recv() {
                Ok(c) => {
                    let (p, m) = (p.clone(), *m);
                    self.pending = None;
                    self.cache.push_front((p, m, c));
                    self.cache.truncate(CACHE);
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(40)),
                Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
            }
        }
        let at = self.cache.iter().position(|(p, m, _)| p == path && *m == e.modified);
        match at {
            Some(i) => {
                let item = self.cache.remove(i).unwrap();
                self.cache.push_front(item);
                Some(&mut self.cache[0].2)
            }
            None => {
                if self.pending.as_ref().is_none_or(|(p, m, _)| p != path || *m != e.modified) {
                    let (tx, rx) = mpsc::channel();
                    let (p, kind, size) = (path.to_path_buf(), kind_of(&e.name), e.size);
                    std::thread::Builder::new()
                        .name("dirmap-preview".into())
                        .spawn(move || {
                            // A decoder bug shows as an error here, not a crash of the window.
                            let c = std::panic::catch_unwind(|| load(&p, kind, size)).unwrap_or_else(|_| Content::Error("This file couldn't be read.".into()));
                            let _ = tx.send(c);
                        })
                        .ok();
                    self.pending = Some((path.to_path_buf(), e.modified, rx));
                    ctx.request_repaint_after(Duration::from_millis(40));
                }
                None
            }
        }
    }
}

impl App {
    /// The item a preview shows: the cursor's item in the focused pane.
    fn look_target(&self) -> Option<(PathBuf, Entry)> {
        if self.files.look.from_map {
            return self.map_target().filter(|(_, e)| !e.is_dir);
        }
        let e = self.files.cursor_entry()?;
        Some((self.files.here().dir()?.join(&*e.name), e))
    }

    /// Quick Look: a floating preview of the item under the cursor (or the tile picked in Storage
    /// Analysis). Arrow keys move through the list behind it; Space or Escape close it, F fills
    /// the window with it.
    pub(super) fn quick_look(&mut self, ctx: &egui::Context) {
        if !self.files.look.open {
            return;
        }
        let Some((path, e)) = self.look_target() else {
            self.files.look.open = false;
            return;
        };
        let typing = ctx.egui_wants_keyboard_input();
        let close = !typing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) || i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
        if close {
            self.files.look.open = false;
            self.files.look.full = false;
            return;
        }
        if !typing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F)) {
            self.files.look.full = !self.files.look.full;
        }
        let screen = ctx.content_rect();
        let size = vec2((screen.width() * 0.72).max(420.0), (screen.height() * 0.78).max(320.0));
        let full = self.files.look.full;
        let mut open = true;
        let mut step = 0i64;
        let mut win = egui::Window::new(egui::RichText::new(format!("{}", e.name)).strong())
            // A window of its own when full, so it comes back to its old size and place.
            .id(egui::Id::new(("quick-look", full)))
            .open(&mut open)
            .collapsible(false);
        win = if full {
            win.fixed_rect(screen.shrink(8.0)).resizable(false)
        } else {
            win.resizable(true).default_size(size).pivot(Align2::CENTER_CENTER).default_pos(screen.center())
        };
        win.show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.weak(facts_line(&e));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Open").clicked() {
                            let p = path.clone();
                            self.files.spawn(move || crate::actions::open(&p));
                        }
                        let (icon, tip) = if full { ("Window", "Back to a window (F)") } else { ("Full screen", "Fill the window (F)") };
                        if ui.button(icon).on_hover_text(tip).clicked() {
                            self.files.look.full = !full;
                        }
                        // Previous and next file, as the arrow keys do (not from the map,
                        // which has no order to walk).
                        if !self.files.look.from_map {
                            if ui.button("▶").on_hover_text("Next file (↓)").clicked() {
                                step = 1;
                            }
                            if ui.button("◀").on_hover_text("Previous file (↑)").clicked() {
                                step = -1;
                            }
                        }
                        let next = if self.files.look.from_map { "" } else { "⏶⏷ next · " };
                        ui.weak(if cfg!(target_os = "macos") { format!("{next}Space closes") } else { format!("{next}Space or Esc closes") });
                    });
                });
                ui.separator();
                let avail = ui.available_size();
                self.preview_body(ui, &path, &e, avail);
            });
        if !open {
            self.files.look.open = false;
            self.files.look.full = false;
        }
        if step != 0 {
            self.files_move_cursor(|pos, _| pos.map(|p| p as i64 + step).unwrap_or(0), false);
        }
    }

    /// The preview pane at the right of the list.
    /// With several items selected: how many of each kind and their size. Clicking a kind
    /// previews one of those items (the next one on each click).
    fn selection_tally(&mut self, ui: &mut egui::Ui) {
        let t = self.files.cur();
        if t.selected.len() < 2 {
            return;
        }
        let Some(l) = t.listing.as_ref() else { return };
        // kind -> (items, bytes, names in list order)
        let mut kinds: std::collections::BTreeMap<String, (usize, u64, Vec<Box<str>>)> = Default::default();
        let (mut total, mut files, mut folders) = (0u64, 0usize, 0usize);
        for &i in &t.order {
            let e = &l.entries[i as usize];
            if !t.selected.contains(&e.name) {
                continue;
            }
            // A sequence row stands for all its frames.
            let (kind, size) = match t.seqs.get(&i) {
                Some(s) => (format!("{} sequence", s.seq.extension().to_uppercase()), s.seq.bytes),
                // A folder's size is known only from an analysis that covers it.
                None if e.is_dir => (browser::kind_for(&e.name, true), t.sizes.as_ref().and_then(|s| s.1.get(&e.name)).map_or(0, |s| s.0)),
                None => (browser::kind_for(&e.name, false), e.size),
            };
            let k = kinds.entry(kind).or_default();
            k.0 += 1;
            k.1 += size;
            k.2.push(e.name.clone());
            total += size;
            if e.is_dir { folders += 1 } else { files += 1 }
        }
        let mut rows: Vec<_> = kinds.into_iter().collect();
        rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)));
        let cursor = t.cursor.clone();
        let pal = style::pal(ui);
        ui.add_space(4.0);
        let what = match (files, folders) {
            (_, 0) => format!("{files} files"),
            (0, _) => format!("{folders} folders"),
            _ => format!("{files} files, {folders} folders"),
        };
        ui.label(egui::RichText::new(format!("{} selected · {what} · {}", group(t.selected.len() as u64), crate::scan::human(total))).strong());
        ui.add_space(2.0);
        let mut show = None;
        egui::Grid::new("preview-tally").num_columns(3).spacing([10.0, 2.0]).striped(false).show(ui, |ui| {
            for (kind, (n, bytes, names)) in rows.iter().take(12) {
                let here = cursor.as_ref().is_some_and(|c| names.contains(c));
                let r = ui.add(egui::Button::selectable(here, kind.as_str())).on_hover_text("Preview one of these (click again for the next)");
                if r.clicked() {
                    // The next of this kind after the one on show, wrapping round.
                    let at = cursor.as_ref().and_then(|c| names.iter().position(|x| x == c));
                    show = Some(names[at.map_or(0, |i| (i + 1) % names.len())].clone());
                }
                ui.label(egui::RichText::new(group(*n as u64)).color(pal.weak));
                ui.label(egui::RichText::new(crate::scan::human(*bytes)).color(pal.weak));
                ui.end_row();
            }
        });
        if rows.len() > 12 {
            ui.weak(format!("and {} more kinds", rows.len() - 12));
        }
        if let Some(name) = show {
            // Move the cursor only: the selection stays as it is.
            let t = self.files.tab();
            t.cursor = Some(name);
            t.scroll_to_cursor = true;
        }
        ui.add_space(6.0);
        ui.separator();
    }

    pub(super) fn preview_pane(&mut self, ui: &mut egui::Ui) {
        let pal = style::pal(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            style::section_title(ui, "Preview");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::icon_button(ui, true, "✖", "Close the preview pane (Alt+P)").clicked() {
                    self.files.prefs.preview = false;
                }
            });
        });
        self.selection_tally(ui);
        let Some((path, e)) = self.look_target() else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("Select a file to preview it").color(pal.weak)));
            return;
        };
        ui.add_space(4.0);
        let w = ui.available_width();
        let h = (ui.available_height() * 0.55).clamp(120.0, w * 1.2);
        // Folders get an icon; a package (a Logic project, a LightCraft library…) gets a preview
        // like a file.
        if e.is_dir && !crate::packages::is_package(&path, &e.name) {
            let (r, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
            self.icons.paint(ui, Rect::from_center_size(r.center(), vec2(64.0, 64.0)), true, "", Color32::GRAY);
        } else {
            self.preview_body(ui, &path, &e, vec2(w, h));
        }
        ui.add_space(8.0);
        ui.add(egui::Label::new(egui::RichText::new(&*e.name).strong()).wrap());
        ui.add_space(4.0);
        egui::Grid::new("preview-facts").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            ui.weak("Kind");
            ui.label(browser::kind_for(&e.name, e.is_dir));
            ui.end_row();
            if !e.is_dir {
                ui.weak("Size");
                ui.label(format!("{} ({} bytes)", crate::scan::human(e.size), group(e.size)));
                ui.end_row();
            } else if let Some(&(b, f)) = self.files.cur().sizes.as_ref().and_then(|s| s.1.get(&e.name)) {
                ui.weak("Size");
                ui.label(format!("{} in {} files (analysed)", crate::scan::human(b), group(f)));
                ui.end_row();
            }
            if e.modified > 0 {
                ui.weak("Modified");
                ui.label(fmt_time(e.modified));
                ui.end_row();
            }
            if let Some(Content::Image { size, info, poster: false, .. }) = self.files.look.cache.iter().find(|c| c.0 == path).map(|c| &c.2) {
                ui.weak("Dimensions");
                ui.label(format!("{} × {}", size[0], size[1]));
                ui.end_row();
                ui.weak("Pixels");
                ui.label(info);
                ui.end_row();
            }
            if kind_of(&e.name) == Kind::Font && let Some(Content::Image { info, .. }) = self.files.look.cache.iter().find(|c| c.0 == path).map(|c| &c.2) {
                ui.weak("Font");
                ui.add(egui::Label::new(info.as_str()).wrap());
                ui.end_row();
            }
            if let Some(facts) = self.files.look.cache.iter().find(|c| c.0 == path).map(|c| c.2.facts()) {
                for (k, v) in facts {
                    ui.weak(*k);
                    ui.add(egui::Label::new(v.as_str()).wrap());
                    ui.end_row();
                }
            }
            if let Some(Content::Sound { info, .. }) = self.files.look.cache.iter().find(|c| c.0 == path).map(|c| &c.2) {
                for (k, v) in &info.facts {
                    ui.weak(*k);
                    ui.add(egui::Label::new(v.as_str()).wrap());
                    ui.end_row();
                }
            }
            if let Some(Content::Text { colours: Some(c), .. }) = self.files.look.cache.iter().find(|c| c.0 == path).map(|c| &c.2) {
                ui.weak("Language");
                ui.label(&c.language);
                ui.end_row();
            }
            ui.weak("Where");
            ui.add(egui::Label::new(path.parent().map(|p| p.shown()).unwrap_or_default()).wrap());
            ui.end_row();
        });
        if !e.is_dir && !e.not_local() && !crate::fsys::remote::is_remote(&path) && !crate::fsys::archive::inside(&path) {
            let scene = self.files.look.cache.iter().find(|c| c.0 == path).and_then(|c| if let Content::Splat { scene, .. } = &c.2 { Some(scene.clone()) } else { None });
            if let Some(scene) = scene {
                super::splatopt::card(ui, &path, &scene, e.size);
            }
        }
        if self.settings.system_notes && !crate::fsys::remote::is_remote(&path) {
            super::explain::preview_section(ui, &path, &e.name);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Open").clicked() {
                let p = path.clone();
                self.files.spawn(move || crate::actions::open(&p));
            }
            if !e.is_dir && ui.button("Quick Look").on_hover_text("Space").clicked() {
                self.files.look.open = true;
                self.files.look.from_map = false;
            }
        });
    }

    /// The preview itself: an image, text, or a large icon.
    /// The preview of a file, filling `size`. Videos get a filmstrip under the picture.
    pub(super) fn preview_body(&mut self, ui: &mut egui::Ui, path: &Path, e: &Entry, size: Vec2) {
        let strip = !e.is_dir
            && crate::media::media_of(&e.name) == Some(crate::media::Media::Video)
            && !e.not_local()
            && !crate::fsys::remote::is_remote(path)
            && !crate::fsys::archive::inside(path)
            && size.y > 240.0
            && self.settings.video_strip
            && crate::video::supported();
        if !strip {
            if self.files.look.strip.as_ref().is_some_and(|s| s.path != path) {
                self.files.look.strip = None;
            }
            return self.preview_main(ui, path, e, size);
        }
        // The sound under the frames, once it's known there is some.
        let wave = self.files.look.strip.as_ref().is_some_and(|s| s.path == path && matches!(s.wave, Some(Some(_))));
        let strip_h = STRIP_H + if wave { STRIP_WAVE_H } else { 0.0 };
        self.preview_main(ui, path, e, vec2(size.x, size.y - strip_h - ui.spacing().item_spacing.y));
        self.filmstrip(ui, path, vec2(size.x, strip_h));
    }

    /// The video as a timeline: frames from across it, and its sound under them when it has
    /// some. Click to play from that moment; drag to scrub, the picture following back and forth.
    fn filmstrip(&mut self, ui: &mut egui::Ui, path: &Path, size: Vec2) {
        const N: usize = 8;
        let pal = style::pal(ui);
        if self.files.look.strip.as_ref().is_none_or(|s| s.path != path) {
            let (tx, rx) = mpsc::channel();
            let (wtx, wrx) = mpsc::channel();
            let (p, ctx) = (path.to_path_buf(), ui.ctx().clone());
            let h = (STRIP_H * ui.ctx().pixels_per_point()).round() as u32;
            let _ = std::thread::Builder::new().name("filescape-filmstrip".into()).spawn(move || {
                let _ = tx.send(crate::video::filmstrip(&p, N, h));
                ctx.request_repaint();
                // The sound after the pictures, so they come first.
                let wave = crate::wave::timeline(&p, STRIP_WAVE_BARS).ok().filter(|(w, _)| w.iter().any(|v| *v > 0.01));
                let _ = wtx.send(wave);
                ctx.request_repaint();
            });
            self.files.look.strip = Some(StripView { path: path.to_path_buf(), rx: Some(rx), frames: Vec::new(), length: None, wave_rx: Some(wrx), wave: None });
        }
        let st = self.files.look.strip.as_mut().expect("just set");
        if let Some(rx) = &st.rx
            && let Ok(res) = rx.try_recv()
        {
            st.rx = None;
            if let Ok((info, frames)) = res {
                st.length = info.duration.filter(|d| *d > 0.0);
                for (i, f) in frames.into_iter().enumerate() {
                    let img = egui::ColorImage::from_rgba_unmultiplied(f.size, &f.rgba);
                    st.frames.push((f.pts, ui.ctx().load_texture(format!("strip:{}:{i}", path.display()), img, egui::TextureOptions::LINEAR)));
                }
            }
        }
        if let Some(rx) = &st.wave_rx
            && let Ok(w) = rx.try_recv()
        {
            st.wave_rx = None;
            st.wave = Some(w);
        }
        let (r, _) = ui.allocate_exact_size(size, Sense::hover());
        let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
        p.rect_filled(r, style::RADIUS, Color32::BLACK);
        let film = Rect::from_min_size(r.min, vec2(r.width(), STRIP_H.min(r.height())));
        if st.rx.is_some() {
            ui.put(Rect::from_center_size(film.center(), vec2(18.0, 18.0)), egui::Spinner::new());
            return;
        }
        if st.frames.is_empty() {
            return;
        }
        // As many as fit at a useful size: every other one in a narrow pane.
        let total = st.frames.len();
        let fit = ((film.width() / (STRIP_H * 1.5)).floor() as usize).clamp(3, total.max(3)).min(total);
        let shown: Vec<usize> = (0..fit).map(|k| (k * total + total / 2) / fit).map(|i| i.min(total - 1)).collect();
        let cell = film.width() / fit as f32;
        for (i, (_, tex)) in shown.iter().map(|&j| &st.frames[j]).enumerate() {
            let c = Rect::from_min_size(pos2(film.left() + i as f32 * cell, film.top()), vec2(cell, film.height())).shrink(1.0);
            let ts = tex.size_vec2();
            let s = (c.width() / ts.x).max(c.height() / ts.y);
            // Each frame fills its cell, cropped from the middle, so the strip reads as one band.
            let (fx, fy) = (c.width() / (ts.x * s), c.height() / (ts.y * s));
            p.image(tex.id(), c, Rect::from_center_size(pos2(0.5, 0.5), vec2(fx, fy)), Color32::WHITE);
        }
        // The sound, along the same time as the frames.
        if let Some(Some((w, secs))) = &st.wave
            && let Some(len) = st.length
            && r.height() > STRIP_H + 4.0
        {
            let band = Rect::from_min_max(pos2(r.left(), film.bottom() + 2.0), pos2(r.right(), r.bottom() - 2.0));
            let span = band.width() * (secs / len).min(1.0) as f32;
            let bars = ((span / 2.0) as usize).clamp(1, w.len());
            let bw = span / bars as f32;
            let mid = band.center().y;
            let ink = style::blend(pal.accent, Color32::WHITE, 0.25);
            for i in 0..bars {
                let (lo, hi) = (i * w.len() / bars, ((i + 1) * w.len() / bars).max(i * w.len() / bars + 1));
                let v = w[lo..hi.min(w.len())].iter().copied().fold(0.0, f32::max);
                let h = (v * band.height() * 0.5).max(0.5);
                let x = band.left() + i as f32 * bw;
                p.rect_filled(Rect::from_min_max(pos2(x, mid - h), pos2(x + (bw - 0.5).max(0.5), mid + h)), 0.0, ink);
            }
        }
        let Some(len) = st.length else { return };
        let resp = ui.interact(r, ui.id().with(("timeline", path)), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        let at_x = |x: f32| ((x - r.left()) / r.width()).clamp(0.0, 1.0) as f64 * len;
        // Where the video is, and where the pointer would take it.
        let playing_here = self.files.look.video.as_ref().filter(|v| v.video.path == path);
        let head = playing_here.map(|v| v.scrub.map_or(v.video.state().position, |s| s.1));
        if let Some(t) = head {
            let x = r.left() + (t / len).clamp(0.0, 1.0) as f32 * r.width();
            p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(2.0, Color32::WHITE));
            p.circle_filled(pos2(x, r.top() + 1.0), 4.0, Color32::WHITE);
        }
        if let Some(q) = resp.hover_pos() {
            let x = q.x.clamp(r.left(), r.right());
            p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0, pal.accent));
            let label = crate::media::clock(at_x(x));
            let g = p.layout_no_wrap(label, FontId::monospace(11.0), Color32::WHITE);
            let left = (x - g.size().x / 2.0 - 4.0).clamp(r.left(), (r.right() - g.size().x - 8.0).max(r.left()));
            let chip = Rect::from_min_size(pos2(left, r.bottom() - g.size().y - 6.0), g.size() + vec2(8.0, 4.0));
            p.rect_filled(chip, 3.0, Color32::from_black_alpha(190));
            p.galley(chip.min + vec2(4.0, 2.0), g, Color32::WHITE);
        }
        let pointer = resp.interact_pointer_pos().map(|q| at_x(q.x));
        if (resp.dragged() || resp.drag_started()) && let Some(t) = pointer {
            // Scrub: paused while dragging, the picture following the pointer.
            if self.files.look.video.as_ref().is_none_or(|v| v.video.path != path) {
                let ctx = ui.ctx().clone();
                let video = crate::video::Video::start(path, true, move || ctx.request_repaint());
                video.pause();
                self.files.look.video = Some(VideoView { video, tex: None, serial: 0, scrub: Some((false, f64::NAN)) });
            }
            if let Some(v) = self.files.look.video.as_mut() {
                let was = v.scrub.map_or_else(|| v.video.state().playing, |s| s.0);
                if v.scrub.is_none() && was {
                    v.video.pause();
                }
                if v.scrub.is_none_or(|s| s.1 != t) {
                    v.video.seek(t);
                }
                v.scrub = Some((was, t));
            }
        } else if resp.drag_stopped() {
            // Let go: carry on playing if it was before.
            if let Some(v) = self.files.look.video.as_mut().filter(|v| v.video.path == path)
                && v.scrub.take().is_some_and(|s| s.0)
            {
                v.video.play();
            }
        } else if resp.clicked() && let Some(t) = pointer {
            // A click plays from there.
            match self.files.look.video.as_mut().filter(|v| v.video.path == path) {
                Some(v) => {
                    v.scrub = None;
                    v.video.seek(t);
                    v.video.play();
                }
                None => {
                    let ctx = ui.ctx().clone();
                    let video = crate::video::Video::start(path, true, move || ctx.request_repaint());
                    video.seek(t);
                    self.files.look.video = Some(VideoView { video, tex: None, serial: 0, scrub: None });
                }
            }
        }
        if head.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    fn preview_main(&mut self, ui: &mut egui::Ui, path: &Path, e: &Entry, mut size: Vec2) {
        let pal = style::pal(ui);
        if e.is_dir && !crate::packages::is_package(path, &e.name) {
            let (r, _) = ui.allocate_exact_size(size, Sense::hover());
            self.icons.paint(ui, Rect::from_center_size(r.center(), vec2(96.0, 96.0)), true, "", Color32::GRAY);
            return;
        }
        let ctx = ui.ctx().clone();
        let tint = self.colors.get(e.ext()).copied().unwrap_or(Color32::GRAY);
        // A sound stops when the preview moves on to another file.
        if self.files.look.player.as_ref().is_some_and(|p| p.path != path) {
            self.files.look.player = None;
        }
        if self.files.look.video.as_ref().is_some_and(|v| v.video.path != path) {
            self.files.look.video = None;
        }
        if self.files.look.view_of.as_deref() != Some(path) {
            self.files.look.view_of = Some(path.to_path_buf());
            self.files.look.zoom = None;
            self.files.look.pan = Vec2::ZERO;
        }
        let (zoom, pan) = (self.files.look.zoom, self.files.look.pan);
        // Reading a file that's only in the cloud downloads it: only when asked to.
        if e.not_local() && self.settings.cloud_confirm && !self.files.look.download_ok.contains(path) {
            let (r, _) = ui.allocate_exact_size(size, Sense::hover());
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(12.0)).layout(egui::Layout::top_down(egui::Align::Center)));
            child.add_space((r.height() / 2.0 - 60.0).max(0.0));
            child.label(egui::RichText::new("☁").size(40.0).color(pal.weak));
            let what = if e.flags & crate::scan::store::ONLINE_ONLY != 0 { "This file is only in the cloud." } else { "This file is only partly downloaded." };
            child.label(egui::RichText::new(what).color(pal.text));
            child.label(egui::RichText::new(format!("Previewing it downloads it ({}).", crate::scan::human(e.size.saturating_sub(e.alloc)))).color(pal.weak).size(12.0));
            child.add_space(6.0);
            if child.button("Download and preview").on_hover_text("Fetch this file from the cloud to preview it. It then takes space on this computer until the cloud app frees it again.").clicked() {
                self.files.look.download_ok.insert(path.to_path_buf());
            }
            if child.link("About cloud files").clicked() {
                self.help.show_topic(topics::CLOUD_FILES);
            }
            return;
        }
        if let Some(v) = self.files.look.video.as_mut() {
            let open_outside = video_view(ui, v, size);
            if open_outside {
                if let Some(v) = &self.files.look.video {
                    v.video.pause();
                }
                let v = path.to_path_buf();
                self.files.spawn(move || crate::media::play_video(&v));
            }
            return;
        }
        let mut start_video = false;
        let Some(c) = self.files.look.get(&ctx, path, e) else {
            let (r, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.put(Rect::from_center_size(r.center(), vec2(24.0, 24.0)), egui::Spinner::new());
            return;
        };
        let mut new_view = None;
        // A drawing: the picture, or its source.
        if let Content::Image { source: Some((text, truncated)), .. } = c {
            let src_id = egui::Id::new("preview-drawing-source");
            let mut show_src = ui.data_mut(|d| *d.get_persisted_mut_or_default::<bool>(src_id));
            let has_text = !text.is_empty();
            let (text, truncated) = (text.clone(), *truncated);
            if has_text {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut show_src, false, "Drawing");
                    ui.selectable_value(&mut show_src, true, "Source");
                });
                ui.data_mut(|d| d.insert_persisted(src_id, show_src));
                size.y -= ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            }
            if has_text && show_src {
                text_view(ui, path, &text, truncated, size, None);
                return;
            }
        }
        // A picture: its layers, channels and exposure.
        if let Content::Image { look: Some(lk), pixels, .. } = c {
            let p = &lk.planes;
            let has_alpha = p.layers[0].channels.iter().any(|c| c.0 == "A");
            if p.linear || p.layers.len() > 1 || has_alpha || size.x > 520.0 {
                let top = ui.cursor().top();
                image_controls(ui, path, lk);
                size.y -= ui.cursor().top() - top;
            }
            if lk.view != lk.drawn {
                let (w, h, rgba) = lk.planes.render(&lk.view);
                *pixels = Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba));
                lk.drawn = lk.view;
            }
        }
        match c {
            Content::Image { pixels, tex, size: full, source, look, .. } => {
                if let Some(px) = pixels.take() {
                    *tex = Some(ctx.load_texture(format!("look:{}", path.display()), px, egui::TextureOptions::LINEAR));
                }
                let Some(tex) = tex else { return };
                let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let ts = tex.size_vec2();
                // Scale against the full image, so 1:1 means actual pixels even when shown smaller.
                // A drawing can be shown larger than its own size: it was drawn big enough.
                let fit = (r.width() / full[0] as f32).min(r.height() / full[1] as f32).min(if source.is_some() { 8.0 } else { 1.0 });
                let scale = zoom.unwrap_or(fit);
                let shown = vec2(full[0] as f32, full[1] as f32) * scale;
                let img_rect = Rect::from_center_size(r.center() + pan, shown);
                let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
                checkerboard(&p, img_rect.intersect(r), pal);
                p.image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                let _ = ts;
                // Wheel zooms about the pointer, drag pans, double-click toggles fit / 1:1.
                let mut z = scale;
                let mut pn = pan;
                if resp.hovered() {
                    let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                    if wheel != 0.0 {
                        let f = (wheel / 200.0).exp();
                        let nz = (z * f).clamp(0.02, 32.0);
                        if let Some(m) = ui.ctx().pointer_hover_pos() {
                            let rel = m - (r.center() + pn);
                            pn += rel - rel * (nz / z);
                        }
                        z = nz;
                        new_view = Some((Some(z), pn));
                    }
                }
                if resp.dragged() {
                    pn += resp.drag_delta();
                    new_view = Some((Some(z), pn));
                }
                if resp.double_clicked() {
                    new_view = Some(if zoom.is_some() { (None, Vec2::ZERO) } else { (Some(1.0), Vec2::ZERO) });
                }
                let mut label = format!("{} × {}  ·  {:.0}%", full[0], full[1], scale * 100.0);
                // The values under the pointer.
                if let Some(lk) = look
                    && let Some(m) = resp.hover_pos()
                    && img_rect.contains(m)
                {
                    let f = (m - img_rect.min) / img_rect.size();
                    let vals = lk.planes.sample(&lk.view, f.x, f.y);
                    let num = |v: f32| if lk.planes.linear { format!("{v:.2}") } else { format!("{}", (v * 255.0).round()) };
                    label.clear();
                    label.push_str(&vals.iter().map(|(n, v)| format!("{n} {}", num(*v))).collect::<Vec<_>>().join("  "));
                }
                let g = ui.painter().layout_no_wrap(label, FontId::proportional(11.0), Color32::WHITE);
                let lr = Rect::from_min_size(r.left_bottom() + vec2(6.0, -g.size().y - 10.0), g.size() + vec2(10.0, 4.0));
                ui.painter().rect_filled(lr, 3.0, Color32::from_black_alpha(150));
                ui.painter().galley(lr.min + vec2(5.0, 2.0), g, Color32::WHITE);
                // A video's frame: a play button over it.
                let video = crate::media::media_of(&e.name) == Some(crate::media::Media::Video) && !crate::fsys::remote::is_remote(path) && !crate::fsys::archive::inside(path);
                if !video {
                    resp.on_hover_text("Scroll to zoom, drag to move, double-click for actual pixels or fit");
                } else {
                    let br = Rect::from_center_size(r.center(), vec2(56.0, 56.0));
                    let b = ui.interact(br, ui.id().with(("play-video", path)), Sense::click());
                    let p = ui.painter();
                    p.circle_filled(br.center(), 28.0, Color32::from_black_alpha(if b.hovered() { 200 } else { 150 }));
                    let c = br.center() + vec2(3.0, 0.0);
                    p.add(egui::Shape::convex_polygon(vec![c + vec2(-9.0, -13.0), c + vec2(13.0, 0.0), c + vec2(-9.0, 13.0)], Color32::WHITE, Stroke::NONE));
                    let tip = if crate::video::supported() {
                        "Play it here"
                    } else if cfg!(target_os = "macos") {
                        "Play in Quick Look (Space or Esc closes it)"
                    } else {
                        "Play in your video app (playing here needs ffmpeg)"
                    };
                    if b.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                        start_video = true;
                    }
                }
            }
            Content::Text { text, truncated, colours } => {
                let truncated = *truncated;
                let text = text.clone();
                let colours = colours.clone();
                if is_markdown(&e.name) {
                    markdown_view(ui, path, &text, truncated, size);
                    return;
                }
                text_view(ui, path, &text, truncated, size, colours.as_deref());
            }
            Content::Sound { play, info, peaks, cover } => {
                let play = play.clone();
                let peaks = peaks.clone();
                let secs = info.secs;
                if cover.is_none() && let Some((w, h, rgba)) = info.cover.take() {
                    *cover = Some(ctx.load_texture(format!("cover:{}", path.display()), egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba), egui::TextureOptions::LINEAR));
                }
                let cover = cover.clone();
                let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                // Cover (or the icon), then the waveform, then the controls, as a block in the middle.
                let wide = (r.width() - 24.0).min(480.0);
                let wave_h = if peaks.is_some() { 52.0 } else { 0.0 };
                let art = (r.height() - 62.0 - wave_h - 36.0).clamp(80.0, wide.min(260.0));
                let block = art + 12.0 + wave_h + 8.0 + 62.0;
                let top = r.center().y - block / 2.0;
                let art_rect = Rect::from_center_size(pos2(r.center().x, top + art / 2.0), vec2(art, art));
                match &cover {
                    Some(t) => {
                        let ts = t.size_vec2();
                        let f = (art / ts.x).min(art / ts.y);
                        ui.painter().image(t.id(), Rect::from_center_size(art_rect.center(), ts * f), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => self.icons.paint_big(ui, Rect::from_center_size(art_rect.center(), vec2(80.0, 80.0)), false, e.ext(), tint),
                }
                let mut y = art_rect.bottom() + 12.0;
                if let Some(w) = &peaks {
                    let wr = Rect::from_center_size(pos2(r.center().x, y + wave_h / 2.0), vec2(wide, wave_h));
                    waveform(ui, &mut self.files.look.player, w, path, wr, secs);
                    y += wave_h + 8.0;
                }
                sound_controls(ui, &mut self.files.look.player, &play, path, Rect::from_center_size(pos2(r.center().x, y + 31.0), vec2(wide.min(360.0), 62.0)));
            }
            Content::Sheet(book) => {
                let book = book.clone();
                sheet_view(ui, path, &book, size);
            }
            Content::Pdf(shown) => {
                let shown = shown.clone();
                pdf_view(ui, path, &shown, size);
            }
            Content::Archive(tree) => {
                let tree = tree.clone();
                archive_view(ui, path, &tree, size);
            }
            Content::Model { mesh, tex, drawn } => model_view(ui, path, mesh, tex, drawn, size),
            Content::Splat { scene, tex, drawn } => splat_view(ui, path, scene, tex, drawn, size),
            Content::Doc { blocks, .. } => {
                let mut blocks = blocks.clone();
                // A splat training project's breakdown and timeline are Pro.
                let project = matches!(blocks.first(), Some(crate::help::doc::Block::Heading { id, .. }) if id == crate::splat::project::PREVIEW_ID);
                if project && !self.licence.allows(crate::licence::Feature::SplatProjects) {
                    blocks = Arc::new(crate::splat::project::free_blocks(&blocks));
                }
                let pal = style::pal(ui);
                egui::Frame::new().fill(pal.well).corner_radius(style::RADIUS).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_min_size(size - vec2(16.0, 16.0));
                    ui.set_max_size(size - vec2(16.0, 16.0));
                    egui::ScrollArea::vertical().id_salt(("look-doc", path)).auto_shrink(false).show(ui, |ui| {
                        ui.set_max_width(ui.available_width());
                        doc_blocks(ui, &blocks);
                    });
                });
            }
            Content::Binary | Content::None | Content::Error(_) => {
                let note = match c {
                    Content::Error(e) => e.clone(),
                    Content::Binary => "No preview for this kind of file".into(),
                    _ => String::new(),
                };
                let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                self.icons.paint(ui, Rect::from_center_size(r.center() - vec2(0.0, 14.0), vec2(80.0, 80.0)), false, e.ext(), tint);
                // Wrapped to the pane: some notes say what to install.
                let mut job = egui::text::LayoutJob::simple(note, FontId::proportional(12.0), pal.weak, (r.width() - 24.0).max(60.0));
                job.halign = egui::Align::Center;
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(r.center().x, r.center().y + 36.0), g, pal.weak);
            }
        }
        if let Some((z, p)) = new_view {
            self.files.look.zoom = z;
            self.files.look.pan = p;
        }
        if start_video {
            if crate::video::supported() {
                let ctx = ui.ctx().clone();
                let video = crate::video::Video::start(path, true, move || ctx.request_repaint());
                self.files.look.video = Some(VideoView { video, tex: None, serial: 0, scrub: None });
            } else {
                let v = path.to_path_buf();
                self.files.spawn(move || crate::media::play_video(&v));
            }
        }
    }
}

/// A playing video: the picture (click to play or pause) and the controls under it. Returns
/// whether it was asked to open in the system's player.
fn video_view(ui: &mut egui::Ui, v: &mut VideoView, size: Vec2) -> bool {
    let pal = style::pal(ui);
    if let Some(f) = v.video.take_frame(&mut v.serial) {
        let img = egui::ColorImage::from_rgba_unmultiplied(f.size, &f.rgba);
        match &mut v.tex {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => v.tex = Some(ui.ctx().load_texture("preview-video", img, egui::TextureOptions::LINEAR)),
        }
    }
    let st = v.video.state();
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let pic = Rect::from_min_max(r.min, pos2(r.max.x, (r.max.y - 34.0).max(r.min.y + 20.0)));
    let p = ui.painter().with_clip_rect(pic.intersect(ui.clip_rect()));
    p.rect_filled(pic, style::RADIUS, Color32::BLACK);
    match &v.tex {
        Some(t) => {
            let ts = t.size_vec2();
            let s = (pic.width() / ts.x).min(pic.height() / ts.y);
            p.image(t.id(), Rect::from_center_size(pic.center(), ts * s), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None if st.error.is_none() => {
            ui.put(Rect::from_center_size(pic.center(), vec2(24.0, 24.0)), egui::Spinner::new());
        }
        None => {}
    }
    if let Some(e) = &st.error {
        p.text(pic.center(), Align2::CENTER_CENTER, e, FontId::proportional(12.0), Color32::from_gray(200));
    }
    let toggle = |v: &VideoView, st: &crate::video::State| if st.playing { v.video.pause() } else { v.video.play() };
    if resp.clicked() && resp.interact_pointer_pos().is_some_and(|q| pic.contains(q)) {
        toggle(v, &st);
    }
    let bar = Rect::from_min_max(pos2(r.min.x, pic.max.y + 4.0), r.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut child;
    if ui.button(if st.playing { "⏸" } else { "▶" }).on_hover_text(if st.playing { "Pause" } else { "Play" }).clicked() {
        toggle(v, &st);
    }
    let mut outside = false;
    let time = match st.info.and_then(|i| i.duration) {
        Some(l) => format!("{} / {}", crate::media::clock(st.position), crate::media::clock(l)),
        None => crate::media::clock(st.position),
    };
    let tw = 140.0;
    if let Some(l) = st.info.and_then(|i| i.duration).filter(|l| *l > 0.0) {
        let mut at = v.scrub.map_or(st.position, |s| s.1).min(l);
        ui.spacing_mut().slider_width = (ui.available_width() - tw - 34.0).max(40.0);
        let s = ui.add(egui::Slider::new(&mut at, 0.0..=l).show_value(false));
        // Dragging scrubs: the picture follows the handle back and forth, paused, and playing
        // carries on from there when it's let go if it was playing before.
        if s.dragged() {
            let was = v.scrub.map_or(st.playing, |s| s.0);
            if v.scrub.is_none() && st.playing {
                v.video.pause();
            }
            if v.scrub.is_none_or(|s| s.1 != at) {
                v.video.seek(at);
            }
            v.scrub = Some((was, at));
        }
        if s.drag_stopped() || (s.clicked() && !s.dragged()) {
            v.video.seek(at);
            if v.scrub.take().is_some_and(|s| s.0) {
                v.video.play();
            }
        }
    }
    ui.label(egui::RichText::new(time).monospace().size(12.0).color(pal.weak));
    let tip = if cfg!(target_os = "macos") { "Open in Quick Look" } else { "Open in your video app" };
    if ui.small_button("Open").on_hover_text(tip).clicked() {
        outside = true;
    }
    if st.playing {
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
    outside
}

fn is_markdown(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "md" | "markdown" | "mdown" | "mkd")
}

/// A Markdown file, formatted (as the Help Centre draws its pages) or as raw text, with a switch
/// between the two that's remembered.
/// Text in a scrolling well, wrapped, in syntax colours for code.
fn text_view(ui: &mut egui::Ui, path: &Path, text: &str, truncated: bool, size: Vec2, colours: Option<&crate::highlight::Colours>) {
    let pal = style::pal(ui);
    egui::Frame::new().fill(pal.well).corner_radius(style::RADIUS).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        ui.set_min_size(size - vec2(16.0, 16.0));
        ui.set_max_size(size - vec2(16.0, 16.0));
        egui::ScrollArea::vertical().id_salt(("look-text", path)).auto_shrink(false).show(ui, |ui| {
            let ink = ui.visuals().text_color();
            let job = match colours {
                Some(c) => coloured_job(text, if ui.visuals().dark_mode { &c.dark } else { &c.light }, ink),
                None => egui::text::LayoutJob::simple(text.to_string(), FontId::monospace(12.0), ink, f32::INFINITY),
            };
            numbered(ui, text, job);
            if truncated {
                ui.weak("… (only the first 256 KB is shown)");
            }
        });
    });
}

/// Text laid out with its line numbers in a gutter at the left: the number on the first row of
/// each line, so a wrapped line keeps one number.
fn numbered(ui: &mut egui::Ui, text: &str, mut job: egui::text::LayoutJob) {
    let font = FontId::monospace(11.0);
    let lines = text.lines().count().max(1);
    let digits = lines.to_string().len();
    let gutter = ui.fonts_mut(|f| f.glyph_width(&font, '0')) * digits as f32 + 12.0;
    job.wrap.max_width = (ui.available_width() - gutter).max(40.0);
    let galley = ui.painter().layout_job(job);
    let (r, _) = ui.allocate_exact_size(vec2(gutter + galley.size().x, galley.size().y), Sense::hover());
    let p = ui.painter();
    let weak = ui.visuals().weak_text_color().gamma_multiply(0.8);
    let view = ui.clip_rect();
    let mut line = 1;
    let mut starts = true;
    for row in &galley.rows {
        let y = r.top() + row.pos.y;
        if starts && y + row.rect().height() >= view.top() && y <= view.bottom() {
            p.text(pos2(r.left() + gutter - 8.0, y + row.rect().height() / 2.0), Align2::RIGHT_CENTER, line.to_string(), font.clone(), weak);
        }
        if row.ends_with_newline {
            line += 1;
        }
        starts = row.ends_with_newline;
    }
    p.galley(r.min + vec2(gutter, 0.0), galley, ui.visuals().text_color());
}

/// A workbook: a switch between its sheets, and the cells of the one picked, drawn only where
/// they're on screen.
fn sheet_view(ui: &mut egui::Ui, path: &Path, book: &crate::sheets::Book, size: Vec2) {
    let pal = style::pal(ui);
    let pick_id = egui::Id::new(("preview-sheet", path));
    let mut pick = ui.data(|d| d.get_temp::<usize>(pick_id)).unwrap_or(0).min(book.sheets.len() - 1);
    egui::Frame::new().fill(pal.well).corner_radius(style::RADIUS).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_min_size(size - vec2(12.0, 12.0));
        ui.set_max_size(size - vec2(12.0, 12.0));
        if book.sheets.len() > 1 {
            egui::ScrollArea::horizontal().id_salt(("sheet-tabs", path)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, s) in book.sheets.iter().enumerate() {
                        // A database's tables carry their size, quieter than the name.
                        let mut job = egui::text::LayoutJob::default();
                        let body = egui::TextStyle::Button.resolve(ui.style());
                        let colour = if pick == i { ui.visuals().selection.stroke.color } else { ui.visuals().widgets.inactive.fg_stroke.color };
                        job.append(&s.name, 0.0, egui::TextFormat::simple(body.clone(), colour));
                        if let Some(b) = s.bytes {
                            job.append(&crate::scan::human(b), 6.0, egui::TextFormat::simple(FontId::proportional(body.size * 0.85), colour.gamma_multiply(0.6)));
                        }
                        ui.selectable_value(&mut pick, i, job);
                    }
                });
            });
            ui.data_mut(|d| d.insert_temp(pick_id, pick));
        }
        let sheet = &book.sheets[pick];
        let (h, w) = sheet.size;
        let more = if sheet.rows_known { "" } else { "+" };
        let mut note = format!("{}{more} {} × {} columns", group(h as u64), if h == 1 && sheet.rows_known { "row" } else { "rows" }, group(w as u64));
        if let Some(b) = sheet.bytes {
            note.push_str(&format!(" · {} in the file", crate::scan::human(b)));
        }
        if h > sheet.rows.len() || w > crate::sheets::COLS {
            note.push_str(&format!(" · the first {} rows and {} columns", sheet.rows.len(), w.min(crate::sheets::COLS)));
        }
        if book.total > book.sheets.len() {
            note.push_str(&format!(" · {} of {} {}", book.sheets.len(), book.total, book.noun));
        }
        ui.label(egui::RichText::new(note).size(11.5).color(pal.weak));
        let font = FontId::proportional(12.0);
        let cw = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let cols = sheet.rows.iter().map(|r| r.len()).max().unwrap_or(0).max(sheet.heads.len());
        let widths: Vec<f32> = (0..cols)
            .map(|c| {
                let longest = sheet.rows.iter().chain(std::iter::once(&sheet.heads)).map(|r| r.get(c).map_or(0, |s| s.chars().count())).max().unwrap_or(0);
                (longest.clamp(3, 32) as f32 * cw + 12.0).max(44.0)
            })
            .collect();
        let num_w = (group(sheet.rows.len() as u64).len() as f32 * cw + 14.0).max(30.0);
        let row_h = 20.0;
        let total_w = num_w + widths.iter().sum::<f32>();
        egui::ScrollArea::both().id_salt(("sheet", path, pick)).auto_shrink(false).show_rows(ui, row_h, sheet.rows.len() + 1, |ui, range| {
            let (rect, _) = ui.allocate_exact_size(vec2(total_w, row_h * range.len() as f32), Sense::hover());
            let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            let line = Stroke::new(1.0, pal.weak.gamma_multiply(0.18));
            for (k, i) in range.clone().enumerate() {
                let y = rect.top() + k as f32 * row_h;
                let head = i == 0;
                if head {
                    p.rect_filled(Rect::from_min_size(pos2(rect.left(), y), vec2(total_w, row_h)), 0.0, pal.weak.gamma_multiply(0.12));
                }
                let num = if head { String::new() } else { group(i as u64) };
                p.text(pos2(rect.left() + num_w - 6.0, y + row_h / 2.0), Align2::RIGHT_CENTER, num, font.clone(), pal.weak);
                let mut x = rect.left() + num_w;
                for (c, cw) in widths.iter().enumerate() {
                    let cell = Rect::from_min_size(pos2(x, y), vec2(*cw, row_h));
                    let (text, colour) = if head { (sheet.heads.get(c).cloned().unwrap_or_else(|| crate::sheets::column(c)), if sheet.heads.is_empty() { pal.weak } else { pal.text }) } else { (sheet.rows[i - 1].get(c).cloned().unwrap_or_default(), pal.text) };
                    if !text.is_empty() {
                        // Numbers to the right, like a spreadsheet.
                        let numeric = !head && text.parse::<f64>().is_ok();
                        let (at, align) = if head { (cell.center(), Align2::CENTER_CENTER) } else if numeric { (pos2(cell.right() - 6.0, cell.center().y), Align2::RIGHT_CENTER) } else { (pos2(cell.left() + 6.0, cell.center().y), Align2::LEFT_CENTER) };
                        p.with_clip_rect(cell.intersect(p.clip_rect())).text(at, align, text.replace('\n', " "), font.clone(), colour);
                    }
                    p.line_segment([cell.left_top(), cell.left_bottom()], line);
                    x += cw;
                }
                p.line_segment([pos2(rect.left(), y + row_h), pos2(rect.left() + total_w, y + row_h)], line);
            }
        });
    });
}

/// An archive's contents as a tree: folders open and close with a click, biggest first, with
/// each item's size, a bar for its share of the whole, and how much compression saved.
fn archive_view(ui: &mut egui::Ui, path: &Path, tree: &crate::archtree::Tree, size: Vec2) {
    use std::collections::HashSet;
    let pal = style::pal(ui);
    let open_id = egui::Id::new(("preview-archive-open", path));
    let mut open: HashSet<usize> = ui.data(|d| d.get_temp(open_id)).unwrap_or_else(|| {
        // A single folder at the top (the usual way archives are made) starts open.
        let root = &tree.nodes[0];
        let mut o = HashSet::new();
        if let [only] = root.children[..]
            && tree.nodes[only].is_dir
        {
            o.insert(only);
        }
        o
    });
    // The rows showing: depth-first through the open folders.
    let mut rows = Vec::new();
    let mut stack: Vec<usize> = tree.nodes[0].children.iter().rev().copied().collect();
    while let Some(i) = stack.pop() {
        rows.push(i);
        if open.contains(&i) {
            stack.extend(tree.nodes[i].children.iter().rev().copied());
        }
    }
    let total = tree.nodes[0].size.max(1);
    let packed_col = tree.nodes[0].packed.is_some() && tree.nodes[0].files > 0;
    let mut toggle = None;
    egui::Frame::new().fill(pal.well).corner_radius(style::RADIUS).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_min_size(size - vec2(12.0, 12.0));
        ui.set_max_size(size - vec2(12.0, 12.0));
        let font = FontId::proportional(12.0);
        let row_h = 20.0;
        let size_w = 66.0;
        let saved_w = if packed_col { 46.0 } else { 0.0 };
        let bar_w = 56.0;
        // Column heads.
        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
        let small = FontId::proportional(11.0);
        let mut x = hr.right() - 4.0;
        if packed_col {
            ui.painter().text(pos2(x, hr.center().y), Align2::RIGHT_CENTER, "Saved", small.clone(), pal.weak);
            x -= saved_w;
        }
        ui.painter().text(pos2(x, hr.center().y), Align2::RIGHT_CENTER, "Size", small.clone(), pal.weak);
        ui.painter().text(pos2(hr.left() + 4.0, hr.center().y), Align2::LEFT_CENTER, "Name", small, pal.weak);
        egui::ScrollArea::vertical().id_salt(("archive", path)).auto_shrink(false).show_rows(ui, row_h, rows.len(), |ui, range| {
            for &i in &rows[range] {
                let n = &tree.nodes[i];
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), if n.is_dir { Sense::click() } else { Sense::hover() });
                let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
                if resp.hovered() {
                    p.rect_filled(r, 3.0, pal.hover);
                }
                let mut right = r.right() - 4.0;
                if packed_col {
                    if let Some(pk) = n.packed
                        && n.size > 0
                    {
                        p.text(pos2(right, r.center().y), Align2::RIGHT_CENTER, crate::archtree::saved_short(pk, n.size), font.clone(), pal.weak);
                    }
                    right -= saved_w;
                }
                p.text(pos2(right, r.center().y), Align2::RIGHT_CENTER, crate::scan::human(n.size), font.clone(), pal.text);
                right -= size_w;
                // Share of the whole archive.
                let bar = Rect::from_min_size(pos2(right - bar_w, r.center().y - 3.0), vec2(bar_w, 6.0));
                if r.width() > 260.0 {
                    p.rect_filled(bar, 3.0, pal.weak.gamma_multiply(0.15));
                    let f = (n.size as f32 / total as f32).clamp(0.0, 1.0);
                    if f > 0.0 {
                        p.rect_filled(Rect::from_min_size(bar.min, vec2((bar_w * f).max(2.0), 6.0)), 3.0, pal.accent);
                    }
                    right = bar.left() - 8.0;
                }
                let indent = r.left() + 4.0 + (n.depth.saturating_sub(1) as f32) * 14.0;
                if n.is_dir {
                    // A small triangle: pointing down when open.
                    let c = pos2(indent + 5.0, r.center().y);
                    let pts = if open.contains(&i) { vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)] } else { vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)] };
                    p.add(egui::Shape::convex_polygon(pts, pal.weak, Stroke::NONE));
                }
                let label = if n.is_dir { format!("{}/", n.name) } else { n.name.clone() };
                let name_rect = Rect::from_min_max(pos2(indent + 14.0, r.top()), pos2(right, r.bottom()));
                p.with_clip_rect(name_rect.intersect(p.clip_rect())).text(pos2(name_rect.left(), r.center().y), Align2::LEFT_CENTER, label, font.clone(), if n.is_dir { pal.strong } else { pal.text });
                if n.is_dir {
                    let files = n.files;
                    let resp = resp.on_hover_text(format!("{} {} inside", group(files), if files == 1 { "file" } else { "files" }));
                    if resp.clicked() {
                        toggle = Some(i);
                    }
                }
            }
        });
    });
    if let Some(i) = toggle
        && !open.remove(&i)
    {
        open.insert(i);
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
}

/// A 3D model: drag to turn it, double-click to put it back.
fn model_view(ui: &mut egui::Ui, path: &Path, mesh: &crate::model3d::Mesh, tex: &mut Option<egui::TextureHandle>, drawn: &mut Option<(crate::model3d::Angle, [usize; 2], usize)>, size: Vec2) {
    use crate::model3d::Angle;
    let pal = style::pal(ui);
    let angle_id = egui::Id::new(("model-angle", path));
    let mut angle: Angle = ui.data(|d| d.get_temp(angle_id)).unwrap_or_default();
    let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let turning = resp.dragged();
    if turning {
        let d = resp.drag_delta();
        angle.yaw += d.x * 0.01;
        angle.pitch = (angle.pitch + d.y * 0.01).clamp(-1.5, 1.5);
    }
    if resp.double_clicked() {
        angle = Angle::default();
    }
    ui.data_mut(|d| d.insert_temp(angle_id, angle));
    let ppp = ui.ctx().pixels_per_point();
    let px = [((r.width() * ppp) as usize).clamp(16, 1600), ((r.height() * ppp) as usize).clamp(16, 1600)];
    // While it's turning, fewer triangles in big models so it keeps up.
    let step = if turning { (mesh.tris.len() / 150_000).max(1) } else { 1 };
    if *drawn != Some((angle, px, step)) || tex.is_none() {
        let rgba = mesh.draw(px[0], px[1], angle, step);
        let img = egui::ColorImage::from_rgba_unmultiplied(px, &rgba);
        match tex {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => *tex = Some(ui.ctx().load_texture(format!("model:{}", path.display()), img, egui::TextureOptions::LINEAR)),
        }
        *drawn = Some((angle, px, step));
        if step > 1 {
            ui.ctx().request_repaint(); // draw it whole once it stops
        }
    }
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    p.rect_filled(r, style::RADIUS, pal.well);
    if let Some(t) = tex {
        p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text("Drag to turn it, double-click to put it back");
}

/// What the splat preview's texture shows, and the picture it's waiting for from the GPU.
#[derive(Default)]
pub struct SplatDrawn {
    /// The view and size in the texture, and whether it's the real splatting.
    shown: Option<(crate::splat::View, [usize; 2], bool)>,
    /// The GPU wish being waited for: its id, view and size.
    pending: Option<(u64, crate::splat::View, [usize; 2])>,
    /// Recent wishes, newest last. While the scene is being turned a new one is made every
    /// frame, so a finished picture is usually for one a frame or two old: it's shown all the
    /// same (it's the newest there is), or turning would show nothing until the drag stopped.
    asked: VecDeque<(u64, crate::splat::View, [usize; 2])>,
    /// Time the last GPU picture took, and the splats in it.
    gpu: Option<(f32, usize)>,
    /// The false colours of the look on show, made when it was picked.
    paint: Option<(crate::splat::inspect::Look, crate::splat::gpu::Paint)>,
}

/// A splat scene or point cloud: drag to turn, right-drag (or shift-drag) to move, scroll to
/// zoom, double-click to put it back. Splats are drawn for real on the graphics chip; until
/// the first picture comes, or where the chip can't, their centres are drawn as points.
fn splat_view(ui: &mut egui::Ui, path: &Path, scene: &Arc<crate::splat::Scene>, tex: &mut Option<egui::TextureHandle>, drawn: &mut SplatDrawn, size: Vec2) {
    use crate::splat::{SceneKind, View, gpu};
    let pal = style::pal(ui);
    let view_id = egui::Id::new(("splat-view", path));
    let mut view: View = ui.data(|d| d.get_temp(view_id)).unwrap_or_default();
    let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let px = [((r.width() * ppp) as usize).clamp(16, 1600), ((r.height() * ppp) as usize).clamp(16, 1600)];
    let busy = steer_splat_view(ui, &resp, r, &mut view);
    let bounds_id = egui::Id::new(("splat-bounds", path));
    let mut show_bounds: bool = ui.data(|d| d.get_temp(bounds_id)).unwrap_or(false);
    ui.data_mut(|d| d.insert_temp(view_id, view));
    // A false-colour look's colours, made once when it's picked.
    if !view.look.recolours() {
        drawn.paint = None;
    } else if drawn.paint.as_ref().is_none_or(|(l, _)| *l != view.look) {
        drawn.paint = crate::splat::inspect::paint(scene, &scene.analysis, view.look).map(|p| (view.look, Arc::new(p)));
    }
    let paint = drawn.paint.as_ref().map(|(_, p)| p.clone());
    let set = |tex: &mut Option<egui::TextureHandle>, img: egui::ColorImage| match tex {
        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
        None => *tex = Some(ui.ctx().load_texture(format!("splat:{}", path.display()), img, egui::TextureOptions::LINEAR)),
    };
    let gpu_ok = !matches!(gpu::status(), gpu::Status::Unavailable(_)) && scene.kind != SceneKind::PointCloud;
    // The same scene can be on screen twice (Quick Look over the preview pane), sharing one
    // picture. Only the bigger view draws; the other shows its picture stretched, so the two
    // don't take turns asking for their own size for ever.
    let claim_id = egui::Id::new(("splat-claim", path));
    let pass = ui.ctx().cumulative_pass_nr();
    let claim: Option<(u64, usize)> = ui.data(|d| d.get_temp(claim_id));
    let follower = claim.is_some_and(|(p, area)| p + 1 >= pass && area > px[0] * px[1]);
    if !follower {
        ui.data_mut(|d| d.insert_temp(claim_id, (pass, px[0] * px[1])));
    }
    // A finished GPU picture.
    if gpu_ok && !follower {
        if let Some(pic) = gpu::take() {
            if let Some(&(_, v, s)) = drawn.asked.iter().find(|(id, _, _)| *id == pic.id) {
                set(tex, egui::ColorImage::from_rgba_premultiplied(pic.size, &pic.rgba));
                drawn.shown = Some((v, s, true));
                drawn.gpu = Some((pic.ms, pic.drawn));
                drawn.asked.retain(|(id, _, _)| *id > pic.id);
                if drawn.pending.is_some_and(|(id, _, _)| id <= pic.id) {
                    drawn.pending = None;
                }
            }
        }
    }
    // While it's moving, pictures at half size: a quarter of the pixels to draw, copy back
    // and upload, so it keeps up; the full size follows when it stops.
    let gpu_px = if busy && gpu_ok { [(px[0] / 2).max(16), (px[1] / 2).max(16)] } else { px };
    let want = (view, gpu_px);
    if !follower && (drawn.shown.map(|(v, s, _)| (v, s)) != Some(want) || tex.is_none()) {
        if gpu_ok {
            if drawn.pending.map(|(_, v, s)| (v, s)) != Some(want) {
                let id = gpu::want(scene.clone(), paint.clone(), view, [gpu_px[0] as u32, gpu_px[1] as u32], Some(ui.ctx().clone()));
                drawn.pending = Some((id, view, gpu_px));
                drawn.asked.push_back((id, view, gpu_px));
                while drawn.asked.len() > 32 {
                    drawn.asked.pop_front();
                }
            }
            // Something to look at until the first picture comes.
            if tex.is_none() {
                set(tex, egui::ColorImage::from_rgba_unmultiplied(px, &scene.draw_painted(px[0], px[1], &view, (scene.len() / 300_000).max(1), paint.as_deref().map(|p| p.as_slice()))));
                drawn.shown = Some((view, px, false));
            }
        } else {
            // While it's moving, a sample of big scenes so it keeps up.
            let step = if busy { (scene.len() / 150_000).max(1) } else { 1 };
            set(tex, egui::ColorImage::from_rgba_unmultiplied(px, &scene.draw_painted(px[0], px[1], &view, step, paint.as_deref().map(|p| p.as_slice()))));
            drawn.shown = Some((view, px, false));
            drawn.pending = None;
            if step > 1 {
                drawn.shown = None; // draw it whole once it stops
                ui.ctx().request_repaint();
            }
        }
    }
    if drawn.pending.is_some() {
        // Look again now and then, in case the graphics chip has stopped answering.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    }
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    p.rect_filled(r, style::RADIUS, pal.well);
    if let Some(t) = tex {
        p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    let real = drawn.shown.is_some_and(|s| s.2);
    let what = if scene.kind == SceneKind::PointCloud {
        "Points".to_string()
    } else if real {
        match (gpu::status(), drawn.gpu) {
            (gpu::Status::Ready(name), Some((ms, n))) if n < scene.len() => format!("{name} · {ms:.0} ms · {} of the splats, to fit graphics memory", crate::splat::facts::group(n)),
            (gpu::Status::Ready(name), Some((ms, _))) => format!("{name} · {ms:.0} ms"),
            _ => "Splats".to_string(),
        }
    } else if let gpu::Status::Unavailable(why) = gpu::status() {
        format!("Splat centres, drawn as points: {why}")
    } else {
        "Splat centres, drawn as points (the graphics chip is starting)".to_string()
    };
    // On a dark strip, so it reads over a busy scene.
    let galley = p.layout_no_wrap(what, FontId::proportional(11.0), Color32::from_gray(220));
    let at = r.left_bottom() + vec2(8.0, -6.0 - galley.size().y);
    p.rect_filled(Rect::from_min_size(at - vec2(4.0, 2.0), galley.size() + vec2(8.0, 4.0)), 3.0, Color32::from_black_alpha(150));
    p.galley(at, galley, Color32::from_gray(220));
    let on_button = ui.rect_contains_pointer(save_button_rect(r));
    let resp = resp.on_hover_cursor(egui::CursorIcon::Grab);
    let resp = if on_button { resp } else { resp.on_hover_text("Drag to turn it, right-drag or shift-drag to move it, scroll to zoom, double-click to put it back") };
    if show_bounds {
        draw_bounds(&p, scene, &view, r);
    }
    splat_legend(&p, scene, view.look, r);
    // The tools along the top (look, bounds, field of view), when there's room; a change
    // shows from the next frame.
    if r.height() >= 150.0 && r.width() >= 300.0 && scene.kind != SceneKind::PointCloud {
        let mut v = view;
        splat_tools(ui, r, &mut v, &mut show_bounds, resp.hovered() || resp.dragged());
        ui.data_mut(|d| d.insert_temp(bounds_id, show_bounds));
        if v != view {
            ui.data_mut(|d| d.insert_temp(view_id, v));
            ui.ctx().request_repaint();
        }
    }
    save_view_button(ui, path, scene, paint, view, r, px, resp.hovered());
}

/// Drag to turn, right-drag (or shift-drag) to move, scroll or pinch to zoom, W A S D Q E to
/// move about while the pointer is over it, double-click to put it back. Whether it's moving.
pub(super) fn steer_splat_view(ui: &mut egui::Ui, resp: &egui::Response, r: Rect, view: &mut crate::splat::View) -> bool {
    use crate::splat::View;
    let shift = ui.input(|i| i.modifiers.shift);
    let moving = resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) || (shift && resp.dragged_by(egui::PointerButton::Primary));
    let turning = !moving && resp.dragged_by(egui::PointerButton::Primary);
    let d = resp.drag_delta();
    if turning {
        view.yaw += d.x * 0.01;
        view.pitch = (view.pitch + d.y * 0.01).clamp(-1.5, 1.5);
    } else if moving {
        let k = view.pan_per_pixel(r.height());
        view.pan[0] -= d.x * k;
        view.pan[1] += d.y * k;
    }
    if resp.hovered() {
        // Take the scroll for zooming, so the pane around it doesn't scroll as well.
        let (scroll, pinch) = ui.input_mut(|i| {
            let s = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta = Vec2::ZERO;
            (s, i.zoom_delta())
        });
        if scroll != 0.0 || pinch != 1.0 {
            view.zoom = (view.zoom * (-scroll * 0.002).exp() / pinch).clamp(0.02, 20.0);
        }
    }
    // W A S D to move about (and Q and E down and up) while the pointer is over it.
    let mut keys_moving = false;
    if resp.hovered() && !ui.ctx().egui_wants_keyboard_input() {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let held = |k: egui::Key| ui.input(|i| i.key_down(k) && i.modifiers.is_none());
        let (fwd, side, up) = (held(egui::Key::W) as i8 - held(egui::Key::S) as i8, held(egui::Key::D) as i8 - held(egui::Key::A) as i8, held(egui::Key::E) as i8 - held(egui::Key::Q) as i8);
        if fwd != 0 || side != 0 || up != 0 {
            keys_moving = true;
            view.zoom = (view.zoom * (-(fwd as f32) * dt * 0.8).exp()).clamp(0.02, 20.0);
            view.pan[0] += side as f32 * dt * 0.6 * view.zoom;
            view.pan[1] += up as f32 * dt * 0.6 * view.zoom;
            ui.ctx().request_repaint();
        }
    }
    if resp.double_clicked() {
        // Back to the start, keeping the look and field of view.
        *view = View { look: view.look, fov: view.fov, ..View::default() };
    }
    turning || moving || keys_moving
}

/// The look picker, bounds switch and field of view along the top of a splat view: faint
/// until the pointer comes over the view, and always shown while a look other than the
/// scene's own is on.
fn splat_tools(ui: &mut egui::Ui, r: Rect, view: &mut crate::splat::View, bounds: &mut bool, hovered: bool) {
    use crate::splat::inspect::Look;
    let bar = Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2((r.width() - 60.0).min(430.0), 26.0));
    // The pointer anywhere over the view (or over the look list, which hangs below the bar
    // and is on a layer of its own).
    let pointer_in = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| r.contains(p));
    let near = hovered || pointer_in || ui.ctx().any_popup_open();
    if !near && view.look == Look::Splats && !*bounds {
        return;
    }
    ui.painter().rect_filled(bar.expand(2.0), 6.0, Color32::from_black_alpha(if near { 175 } else { 110 }));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(6.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    // Dark controls on the dark bar, whatever the app's theme: in the light theme the boxes
    // would be light, and light text on them unreadable.
    child.set_visuals(egui::Visuals::dark());
    egui::ComboBox::from_id_salt(("splat-look", r.min.x as i32, r.min.y as i32))
        .selected_text(view.look.name())
        .width(92.0)
        .show_ui(&mut child, |ui| {
            for l in Look::ALL {
                ui.selectable_value(&mut view.look, l, l.name()).on_hover_text(l.about());
            }
        })
        .response
        .on_hover_text(view.look.about());
    child.checkbox(bounds, "Bounds").on_hover_text("The box round the scene's main body (the middle 80% of its splats on each axis)");
    child.label("FOV");
    child
        .add(egui::Slider::new(&mut view.fov, crate::splat::FOV_RANGE).suffix("°").show_value(true).max_decimals(0))
        .on_hover_text("Field of view: the camera moves in or out as it changes, so only the perspective changes");
}

/// The main body's box, as lines over the picture.
fn draw_bounds(p: &egui::Painter, scene: &crate::splat::Scene, view: &crate::splat::View, r: Rect) {
    if scene.pos.is_empty() {
        return;
    }
    let (lo, hi) = crate::splat::facts::bounds(scene);
    let proj = scene.projector(view, r.width(), r.height());
    let corner = |i: usize| [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
    let stroke = egui::Stroke::new(1.5, Color32::from_rgb(120, 220, 255));
    for a in 0..8usize {
        for bit in [1, 2, 4] {
            let b = a | bit;
            if b == a {
                continue;
            }
            // Along the edge in steps, so an edge passing behind the camera is cut, not wrapped.
            let pts: Vec<egui::Pos2> = (0..=16)
                .filter_map(|k| {
                    let t = k as f32 / 16.0;
                    let (pa, pb) = (corner(a), corner(b));
                    proj([pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t, pa[2] + (pb[2] - pa[2]) * t]).map(|q| r.min + vec2(q[0], q[1]))
                })
                .collect();
            if pts.len() >= 2 {
                p.add(egui::Shape::line(pts, stroke));
            }
        }
    }
}

/// What a false-colour look's colours mean, or what the Problems look found, in the bottom
/// right corner.
fn splat_legend(p: &egui::Painter, scene: &crate::splat::Scene, look: crate::splat::inspect::Look, r: Rect) {
    use crate::splat::inspect::{self, Look};
    let font = FontId::proportional(11.0);
    let ink = Color32::from_gray(225);
    if let Some((lo, hi)) = look.scale_ends() {
        let w = 140.0;
        let bar = Rect::from_min_size(pos2(r.right() - w - 14.0, r.bottom() - 34.0), vec2(w, 8.0));
        p.rect_filled(bar.expand2(vec2(8.0, 20.0)).translate(vec2(0.0, 6.0)), 4.0, Color32::from_black_alpha(160));
        let steps = 28;
        for k in 0..steps {
            let c = inspect::ramp(k as f32 / (steps - 1) as f32);
            let x0 = bar.left() + bar.width() * k as f32 / steps as f32;
            p.rect_filled(Rect::from_min_max(pos2(x0, bar.top()), pos2(x0 + bar.width() / steps as f32 + 0.5, bar.bottom())), 0.0, Color32::from_rgb(c[0], c[1], c[2]));
        }
        p.text(pos2(bar.left(), bar.bottom() + 3.0), Align2::LEFT_TOP, lo, font.clone(), ink);
        p.text(pos2(bar.right(), bar.bottom() + 3.0), Align2::RIGHT_TOP, hi, font, ink);
    } else if look == Look::Problems {
        let a = &scene.analysis;
        let rows: Vec<(String, [u8; 3])> = inspect::PROBLEMS.iter().map(|(bit, rgb, name)| (format!("{name}: {}", crate::splat::facts::group(inspect::count(a, *bit))), *rgb)).collect();
        let h = rows.len() as f32 * 16.0 + 8.0;
        let bx = Rect::from_min_size(pos2(r.right() - 190.0, r.bottom() - h - 8.0), vec2(182.0, h));
        p.rect_filled(bx, 4.0, Color32::from_black_alpha(170));
        for (i, (text, c)) in rows.into_iter().enumerate() {
            let y = bx.top() + 12.0 + i as f32 * 16.0;
            p.circle_filled(pos2(bx.left() + 12.0, y), 4.5, Color32::from_rgb(c[0], c[1], c[2]));
            p.text(pos2(bx.left() + 22.0, y), Align2::LEFT_CENTER, text, font.clone(), ink);
        }
    }
}

/// Where a saved picture of a splat view is up to: the picker's answer, then the drawing.
#[derive(Clone)]
struct SavingView {
    picked: crate::asking::Mailbox,
    view: crate::splat::View,
    px: [usize; 2],
    /// What happened, once the picture has been drawn and written.
    said: Arc<std::sync::Mutex<Option<(String, std::time::Instant)>>>,
    started: bool,
}

fn save_button_rect(r: Rect) -> Rect {
    Rect::from_min_size(pos2(r.right() - 34.0, r.top() + 8.0), vec2(26.0, 26.0))
}

/// The biggest side of a saved picture of a splat view.
const SAVED_VIEW_MAX: f32 = 3840.0;

/// A "Save picture" button in the top right corner of a splat view: the view as it is, drawn
/// again at up to twice its size (up to 4K) and saved as a PNG where the person picks.
fn save_view_button(ui: &mut egui::Ui, path: &Path, scene: &Arc<crate::splat::Scene>, paint: Option<crate::splat::gpu::Paint>, view: crate::splat::View, r: Rect, px: [usize; 2], hovered: bool) {
    let id = egui::Id::new(("splat-save", path));
    let mut st: Option<SavingView> = ui.data(|d| d.get_temp(id));
    let said = st.as_ref().and_then(|s| s.said.lock().unwrap().clone());
    let fresh_said = said.as_ref().filter(|(_, t)| t.elapsed().as_secs() < 6);
    let busy = st.as_ref().is_some_and(|s| s.started && s.said.lock().unwrap().is_none());
    let button = save_button_rect(r);
    let over = ui.rect_contains_pointer(button);
    if hovered || over || busy || fresh_said.is_some() {
        let p = ui.painter();
        p.rect_filled(button, 6.0, Color32::from_black_alpha(if over { 200 } else { 140 }));
        p.text(button.center(), Align2::CENTER_CENTER, if busy { "…" } else { "📷" }, FontId::proportional(14.0), Color32::from_gray(230));
        let b = ui.interact(button, id.with("button"), Sense::click()).on_hover_text("Save this view as a picture (PNG, up to 4K)");
        if b.clicked() && !busy {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Scene".into());
            let mut d = crate::asking::Dialog::new().set_file_name(format!("{stem} view.png")).add_filter("PNG picture", &["png"]);
            if let Some(dir) = path.parent() {
                d = d.set_directory(dir);
            }
            st = Some(SavingView { picked: crate::asking::ask_into(d, crate::asking::Pick::Save, ui.ctx()), view, px, said: Default::default(), started: false });
        }
    }
    if let Some(s) = st.as_mut().filter(|s| !s.started) {
        let picked = s.picked.lock().unwrap().take();
        if let Some(dest) = picked {
            s.started = true;
            let (scene, view, said, ctx, paint) = (scene.clone(), s.view, s.said.clone(), ui.ctx().clone(), paint.clone());
            let k = (SAVED_VIEW_MAX / s.px[0].max(s.px[1]) as f32).min(2.0);
            let size = [(s.px[0] as f32 * k) as usize, (s.px[1] as f32 * k) as usize];
            std::thread::spawn(move || {
                let msg = match save_view(&scene, paint, view, size, &dest) {
                    Ok(()) => format!("Saved {}", dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                    Err(e) => format!("Couldn't save the picture: {e}"),
                };
                *said.lock().unwrap() = Some((msg, std::time::Instant::now()));
                ctx.request_repaint();
            });
        }
    }
    if let Some((msg, _)) = fresh_said {
        let p = ui.painter();
        let galley = p.layout_no_wrap(msg.clone(), FontId::proportional(11.0), Color32::from_gray(230));
        let at = pos2(button.left() - 8.0 - galley.size().x, button.center().y - galley.size().y / 2.0);
        p.rect_filled(Rect::from_min_size(at - vec2(5.0, 3.0), galley.size() + vec2(10.0, 6.0)), 4.0, Color32::from_black_alpha(170));
        p.galley(at, galley, Color32::from_gray(230));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    if busy || st.as_ref().is_some_and(|s| !s.started) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    if let Some(s) = st {
        ui.data_mut(|d| d.insert_temp(id, s));
    }
}

/// Draw a splat view at `size` (on the graphics chip, or as points where it can't be used) and
/// write it as a PNG.
pub(super) fn save_view(scene: &Arc<crate::splat::Scene>, paint: Option<crate::splat::gpu::Paint>, view: crate::splat::View, size: [usize; 2], dest: &Path) -> Result<(), String> {
    use crate::splat::{SceneKind, gpu};
    let gpu_pic = if scene.kind == SceneKind::Splat { gpu::picture_now(scene.clone(), paint.clone(), view, [size[0] as u32, size[1] as u32], std::time::Duration::from_secs(60)) } else { None };
    let (w, h, rgba) = match gpu_pic {
        Some(p) => {
            // Premultiplied to straight alpha.
            let mut rgba = p.rgba;
            for c in rgba.chunks_exact_mut(4) {
                if c[3] > 0 && c[3] < 255 {
                    for k in 0..3 {
                        c[k] = ((c[k] as u32 * 255 + c[3] as u32 / 2) / c[3] as u32).min(255) as u8;
                    }
                }
            }
            (p.size[0], p.size[1], rgba)
        }
        None => (size[0], size[1], scene.draw_painted(size[0], size[1], &view, 1, paint.as_deref().map(|p| p.as_slice()))),
    };
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("the picture came back the wrong size")?;
    img.save_with_format(dest, image::ImageFormat::Png).map_err(|e| e.to_string())
}

/// Text laid out in its syntax colours; what the spans don't cover is in `plain`.
fn coloured_job(text: &str, spans: &[crate::highlight::Span], plain: Color32) -> egui::text::LayoutJob {
    let font = FontId::monospace(12.0);
    let mut job = egui::text::LayoutJob::default();
    let fmt = |color: Color32, italics: bool| egui::TextFormat { font_id: font.clone(), color, italics, ..Default::default() };
    let mut at = 0;
    for s in spans {
        if s.start >= text.len() || !text.is_char_boundary(s.start) || !text.is_char_boundary(s.end.min(text.len())) {
            break;
        }
        let end = s.end.min(text.len());
        job.append(&text[s.start..end], 0.0, fmt(Color32::from_rgb(s.rgb[0], s.rgb[1], s.rgb[2]), s.italic));
        at = end;
    }
    if at < text.len() {
        job.append(&text[at..], 0.0, fmt(plain, false));
    }
    job
}

/// Every page of a PDF in a column to scroll through. Pages are drawn on worker threads as they
/// come into view, at the width shown, and let go of again once well out of view.
fn pdf_view(ui: &mut egui::Ui, path: &Path, shown: &Arc<PdfShown>, size: Vec2) {
    const GAP: f32 = 10.0;
    let pal = style::pal(ui);
    let ppp = ui.ctx().pixels_per_point();
    let n = shown.doc.sizes.len();
    egui::Frame::new().fill(pal.well).corner_radius(style::RADIUS).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        ui.set_min_size(size - vec2(16.0, 16.0));
        ui.set_max_size(size - vec2(16.0, 16.0));
        let width = ui.available_width() - 4.0;
        let heights: Vec<f32> = shown.doc.sizes.iter().map(|[w, h]| width * h / w).collect();
        let total = heights.iter().sum::<f32>() + GAP * n.saturating_sub(1) as f32;
        let out = egui::ScrollArea::vertical().id_salt(("look-pdf", path)).auto_shrink(false).show_viewport(ui, |ui, view| {
            let (r, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
            let want = ((width * ppp).round() as u32).clamp(64, 2400);
            let near = view.expand2(vec2(0.0, view.height()));
            let keep = view.expand2(vec2(0.0, view.height() * 3.0));
            let mut pages = shown.pages.lock().unwrap();
            let mut started = 0;
            let mut current = 0;
            let mut y = 0.0;
            for (i, h) in heights.iter().enumerate() {
                let local = Rect::from_min_size(pos2(0.0, y), vec2(width, *h));
                let pr = Rect::from_min_size(r.min + vec2(0.0, y), vec2(width, *h));
                y += h + GAP;
                if local.center().y <= view.center().y || i == 0 {
                    current = i;
                }
                if !local.intersects(keep) {
                    if matches!(pages.get(&i), Some(PageSlot::Ready { .. } | PageSlot::Failed)) {
                        pages.remove(&i);
                    }
                    continue;
                }
                // Drawn again when the pane has grown or shrunk a lot since.
                let stale = match pages.get(&i) {
                    None => true,
                    Some(PageSlot::Ready { width: w, again: false, .. }) => (*w as f32 - want as f32).abs() > want as f32 * 0.3,
                    _ => false,
                };
                if stale && local.intersects(near) && started < 4 {
                    started += 1;
                    match pages.get_mut(&i) {
                        Some(PageSlot::Ready { again, .. }) => *again = true,
                        _ => {
                            pages.insert(i, PageSlot::Drawing);
                        }
                    }
                    let s = shown.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let slot = match s.doc.page(i, want) {
                            Ok((w, h, rgba)) => PageSlot::Ready { width: want, pixels: Some(egui::ColorImage::from_rgba_premultiplied([w, h], &rgba)), tex: None, again: false },
                            Err(_) => PageSlot::Failed,
                        };
                        s.pages.lock().unwrap().insert(i, slot);
                        ctx.request_repaint();
                    });
                }
                if !local.intersects(view) {
                    continue;
                }
                let p = ui.painter();
                p.rect_filled(pr, 0.0, Color32::WHITE);
                match pages.get_mut(&i) {
                    Some(PageSlot::Ready { pixels, tex, .. }) => {
                        if let Some(px) = pixels.take() {
                            *tex = Some(ui.ctx().load_texture(format!("pdf:{}:{i}", path.display()), px, egui::TextureOptions::LINEAR));
                        }
                        if let Some(t) = tex {
                            p.image(t.id(), pr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                    }
                    Some(PageSlot::Failed) => {
                        p.text(pr.center(), egui::Align2::CENTER_CENTER, "This page couldn't be drawn", FontId::proportional(12.0), Color32::GRAY);
                    }
                    _ => {
                        ui.put(Rect::from_center_size(pr.center(), vec2(20.0, 20.0)), egui::Spinner::new().color(Color32::GRAY));
                    }
                }
            }
            current
        });
        // Which page is in the middle of the view, over the bottom left corner.
        let label = format!("Page {} of {}", out.inner + 1, group(n as u64));
        let g = ui.painter().layout_no_wrap(label, FontId::proportional(11.0), Color32::WHITE);
        let r = out.inner_rect;
        let lr = Rect::from_min_size(r.left_bottom() + vec2(6.0, -g.size().y - 10.0), g.size() + vec2(10.0, 4.0));
        ui.painter().rect_filled(lr, 3.0, Color32::from_black_alpha(150));
        ui.painter().galley(lr.min + vec2(5.0, 2.0), g, Color32::WHITE);
    });
}

const SHOW_ID: &str = "filescape-show-file";

/// A file a preview's link asked to be shown in Files, once.
pub(super) fn take_show(ctx: &egui::Context) -> Option<PathBuf> {
    ctx.data_mut(|d| d.remove_temp::<PathBuf>(egui::Id::new(SHOW_ID))).filter(|p| !p.as_os_str().is_empty())
}

/// Blocks drawn with the Help Centre's renderer. Web links open in the browser; links to other
/// files or headings do nothing, and pictures aren't fetched.
fn doc_blocks(ui: &mut egui::Ui, blocks: &[crate::help::doc::Block]) {
    use crate::help::{doc, render};
    static NO_DOCS: std::sync::OnceLock<doc::Docs> = std::sync::OnceLock::new();
    let mut images = HashMap::new();
    let mut out = render::Out {
        page: "",
        docs: NO_DOCS.get_or_init(|| doc::Docs::from_sources(&[])),
        images: &mut images,
        nav: None,
        request: None,
        anchors: Vec::new(),
    };
    render::blocks(ui, &mut out, blocks);
    // A file a project uses: shown in Files by the app (see `take_show`).
    if let Some(render::Nav::Url(u)) = &out.nav
        && let Some(p) = crate::projrefs::shown_path(u)
    {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(SHOW_ID), p));
        out.nav = None;
    }
    if let Some(render::Nav::Url(u)) = out.nav.take()
        && (u.contains("://") || u.starts_with("mailto:"))
    {
        ui.ctx().open_url(egui::OpenUrl::new_tab(u));
    }
}

/// A Word, PowerPoint or OpenDocument file: formatted, or the picture saved with it when its
/// words can't be read.
fn load_doc(path: &Path) -> Content {
    if path.file_name().is_some_and(|n| crate::daw::is_project(&n.to_string_lossy())) {
        // Logic saves a picture of the project's window: shown with the project's details.
        if path.file_name().is_some_and(|n| crate::daw::is_logic(&n.to_string_lossy()))
            && let Some(img) = crate::daw::logic_window(path)
        {
            return match crate::daw::read(path) {
                Ok((_, facts)) => Content::Image {
                    look: None,
                    size: [img.width(), img.height()],
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())),
                    tex: None,
                    info: "Logic's picture of the project window, saved with the project".into(),
                    poster: true,
                    source: None,
                    facts,
                },
                Err(e) => Content::Error(e),
            };
        }
        return match crate::daw::read(path) {
            Ok((blocks, facts)) => Content::Doc { blocks: Arc::new(blocks), facts },
            Err(e) => Content::Error(e),
        };
    }
    if path.file_name().is_some_and(|n| crate::mailview::kind_of(&n.to_string_lossy()).is_some()) {
        return match crate::mailview::read(path) {
            Ok(m) => Content::Doc { blocks: Arc::new(m.blocks), facts: m.facts },
            Err(e) => Content::Error(e),
        };
    }
    if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("epub")) {
        return match crate::epub::read(path) {
            Ok(b) => Content::Doc { blocks: Arc::new(b.blocks), facts: b.facts },
            Err(e) => Content::Error(e),
        };
    }
    let Some(kind) = path.file_name().and_then(|n| crate::officedoc::kind_of(&n.to_string_lossy())) else {
        return Content::Binary;
    };
    match crate::officedoc::read(path, kind) {
        Ok(b) => Content::Doc { blocks: Arc::new(b), facts: Vec::new() },
        Err(e) => match crate::officedoc::saved_thumbnail(path) {
            Some(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                Content::Image {
                    look: None,
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)),
                    tex: None,
                    size: [w, h],
                    info: "The picture saved with it".into(),
                    poster: true,
                    source: None,
                    facts: Vec::new(),
                }
            }
            None => Content::Error(e),
        },
    }
}

fn markdown_view(ui: &mut egui::Ui, path: &Path, text: &str, truncated: bool, size: Vec2) {
    use crate::help::doc;
    let pal = style::pal(ui);
    let raw_id = egui::Id::new("preview-markdown-raw");
    let mut raw = ui.data_mut(|d| *d.get_persisted_mut_or_default::<bool>(raw_id));
    egui::Frame::new().fill(pal.well).corner_radius(style::RADIUS).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        ui.set_min_size(size - vec2(16.0, 16.0));
        ui.set_max_size(size - vec2(16.0, 16.0));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut raw, false, "Formatted");
            ui.selectable_value(&mut raw, true, "Raw");
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt(("look-md", path, raw)).auto_shrink(false).show(ui, |ui| {
            if raw {
                ui.add(egui::Label::new(egui::RichText::new(text).monospace().size(12.0)).wrap());
            } else {
                // Parsed once per file shown, not every frame.
                let key = egui::Id::new(("preview-markdown", path, text.len()));
                let blocks = ui.data(|d| d.get_temp::<Arc<Vec<doc::Block>>>(key)).unwrap_or_else(|| {
                    let b = Arc::new(doc::parse(text));
                    ui.data_mut(|d| d.insert_temp(key, b.clone()));
                    b
                });
                ui.set_max_width(ui.available_width());
                doc_blocks(ui, &blocks);
            }
            if truncated {
                ui.weak("… (only the first 256 KB is shown)");
            }
        });
    });
    ui.data_mut(|d| d.insert_persisted(raw_id, raw));
}

/// Play or pause on its own line (a fixed width, so it stays put as its label changes), and
/// under it how far through, with a bar to jump about where the player can.
/// Bars in the waveform shown in the preview.
const WAVE_BARS: usize = 240;

/// A sound's waveform: what has played in the accent colour, and a click jumps there.
fn waveform(ui: &mut egui::Ui, player: &mut Option<crate::media::Player>, w: &[f32], path: &Path, r: Rect, secs: Option<f64>) {
    let pal = style::pal(ui);
    let st = player.as_ref().filter(|p| p.path == path).map(|p| p.state()).unwrap_or_default();
    // The player's length, or the file's own when the player doesn't say (ffplay).
    let done = match st.length.or(secs).filter(|l| *l > 0.0) {
        Some(l) => (st.position / l).clamp(0.0, 1.0) as f32,
        None => 0.0,
    };
    let seekable = player.as_ref().is_some_and(|p| p.path == path && p.can_seek()) && st.length.is_some_and(|l| l > 0.0);
    let resp = ui.interact(r, ui.id().with(("wave", path)), if seekable { Sense::click() } else { Sense::hover() });
    let p = ui.painter().with_clip_rect(r);
    let bars = w.len().max(1);
    let bw = r.width() / bars as f32;
    let mid = r.center().y;
    for (i, v) in w.iter().enumerate() {
        let x = r.left() + i as f32 * bw;
        let h = (v * r.height() * 0.5).max(0.75);
        let c = if (i as f32 + 0.5) / bars as f32 <= done { pal.accent } else { pal.weak.gamma_multiply(0.6) };
        p.rect_filled(Rect::from_min_max(pos2(x + bw * 0.15, mid - h), pos2(x + bw * 0.85, mid + h)), 0.5, c);
    }
    if seekable {
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Click to jump there");
        if resp.clicked() && let (Some(pos), Some(l), Some(pl)) = (resp.interact_pointer_pos(), st.length, player.as_ref()) {
            pl.seek(((pos.x - r.left()) / r.width()).clamp(0.0, 1.0) as f64 * l);
        }
    }
}

fn sound_controls(ui: &mut egui::Ui, player: &mut Option<crate::media::Player>, play: &Path, path: &Path, r: Rect) {
    let pal = style::pal(ui);
    let st = player.as_ref().map(|p| p.state()).unwrap_or_default();
    let top = Rect::from_min_size(r.min, vec2(r.width(), 28.0));
    let bottom = Rect::from_min_max(pos2(r.min.x, r.min.y + 34.0), r.max);
    let label = if st.playing { "⏸ Pause" } else { "▶ Play" };
    let button = egui::Button::new(label).min_size(vec2(96.0, 26.0));
    if ui.put(Rect::from_center_size(top.center(), vec2(96.0, 26.0)), button).on_hover_text("Play the sound here").clicked() {
        match player {
            Some(p) if st.playing => p.pause(),
            Some(p) => p.play(),
            None => {
                let mut p = crate::media::Player::start(play);
                // Known by the file shown, even when a fetched copy plays.
                p.path = path.to_path_buf();
                *player = Some(p);
            }
        }
    }
    let line = |ui: &mut egui::Ui, text: egui::RichText| {
        ui.put(bottom, egui::Label::new(text).wrap());
    };
    if let Some(e) = &st.error {
        line(ui, egui::RichText::new(e).color(pal.weak).size(12.0));
        return;
    }
    let time = match st.length {
        Some(l) => format!("{} / {}", crate::media::clock(st.position), crate::media::clock(l)),
        None => crate::media::clock(st.position),
    };
    let time = egui::RichText::new(time).monospace().size(12.0).color(pal.weak);
    match (st.length.filter(|l| *l > 0.0), player.as_ref().filter(|p| p.can_seek())) {
        (Some(l), Some(p)) => {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bottom).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let ui = &mut child;
            let mut at = st.position.min(l);
            ui.spacing_mut().slider_width = (ui.available_width() - 90.0).max(40.0);
            let s = ui.add(egui::Slider::new(&mut at, 0.0..=l).show_value(false));
            if s.drag_stopped() || (s.clicked() && !s.dragged()) {
                p.seek(at);
            }
            ui.label(time);
        }
        // No bar to show: just the time, centred under the button.
        _ => line(ui, time),
    }
    if st.playing {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}

/// "12.4 MB · PNG image · 3 Nov 2026"
pub(super) fn facts_line(e: &Entry) -> String {
    let mut s = browser::kind_for(&e.name, e.is_dir);
    if !e.is_dir {
        s = format!("{} · {s}", crate::scan::human(e.size));
        if e.flags & crate::scan::store::ONLINE_ONLY != 0 {
            s.push_str(" · online only");
        } else if e.flags & crate::scan::store::PARTIAL != 0 {
            s.push_str(" · partly downloaded");
        }
    }
    if e.modified > 0 {
        s.push_str(&format!(" · {}", fmt_time(e.modified)));
    }
    s
}

/// Grey squares behind an image, so transparency shows.
fn checkerboard(p: &egui::Painter, r: Rect, pal: style::Pal) {
    if !r.is_positive() {
        return;
    }
    let a = style::blend(pal.paper, pal.weak, 0.12);
    let b = style::blend(pal.paper, pal.weak, 0.26);
    p.rect_filled(r, 0.0, a);
    let s = 10.0;
    let (nx, ny) = ((r.width() / s).ceil() as i32, (r.height() / s).ceil() as i32);
    if nx * ny > 40_000 {
        return;
    }
    for y in 0..ny {
        for x in 0..nx {
            if (x + y) % 2 == 0 {
                let c = Rect::from_min_size(r.min + vec2(x as f32 * s, y as f32 * s), vec2(s, s)).intersect(r);
                p.rect_filled(c, 0.0, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_xl_decodes() {
        // An 8 × 6 red picture, lossless, from libjxl.
        const RED: &[u8] = b"\xff\x0a\x28\x30\x10\x09\x08\x02\x01\x00\x54\x00\x4b\x18\x8b\x15\xc2\x49\x02\xe6\x05\x22\x00\x58\xff\xfc\xcc\x00\x00\x00\x00\x00\x00";
        let dir = std::env::temp_dir().join(format!("filescape-jxl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("red");
        std::fs::write(&p, RED).unwrap();
        assert_eq!(crate::formats::sniff(RED).map(|f| f.id), Some("jxl"));
        let (px, size, _) = decode_image(&p, false, RED.len() as u64, 64).unwrap();
        assert_eq!(size, [8, 6]);
        // Red, give or take JPEG XL's colour maths.
        let c = px.pixels[0];
        assert!(c.r() > 245 && c.g() < 10 && c.b() < 10, "{c:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of("a.PNG"), Kind::Image);
        assert_eq!(kind_of("beauty.0001.exr"), Kind::LinearImage);
        assert_eq!(kind_of("main.rs"), Kind::Text);
        assert_eq!(kind_of("README"), Kind::Text);
        assert_eq!(kind_of("movie.mov"), Kind::Media(crate::media::Media::Video));
        assert_eq!(kind_of("data.xyz"), Kind::Other);
        assert_eq!(kind_of("report.docx"), Kind::Doc);
        assert_eq!(kind_of("Talk.PPTX"), Kind::Doc);
        assert_eq!(kind_of("notes.odp"), Kind::Doc);
        assert_eq!(kind_of("song.FLAC"), Kind::Media(crate::media::Media::Audio));
        assert_eq!(kind_of("logo.svg"), Kind::Drawing);
        assert_eq!(kind_of("Inter.otf"), Kind::Font);
        assert_eq!(kind_of("tune.mid"), Kind::Midi);
        assert_eq!(kind_of("data.TSV"), Kind::Sheet);
        assert_eq!(kind_of("History.sqlite"), Kind::Database);
        assert_eq!(kind_of("IMG_0001.CR3"), Kind::Raw);
        assert_eq!(kind_of("poster.psd"), Kind::Psd);
        assert_eq!(kind_of("Budget.XLSX"), Kind::Sheet);
        // A file with no extension is known by its first bytes.
        let dir = std::env::temp_dir().join(format!("fs-sniff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("download");
        std::fs::write(&p, b"%PDF-1.7\n").unwrap();
        assert_eq!(sniffed(&p), Some(Kind::Media(crate::media::Media::Pdf)));
        std::fs::write(&p, b"just some words").unwrap();
        assert_eq!(sniffed(&p), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn previews_read_safely() {
        let d = std::env::temp_dir().join(format!("dirmap-look-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        // Text, cut at the limit.
        let t = d.join("big.txt");
        std::fs::write(&t, "x".repeat(TEXT_MAX + 10)).unwrap();
        match load(&t, Kind::Text, 0) {
            Content::Text { text, truncated, .. } => assert!(truncated && text.len() == TEXT_MAX),
            _ => panic!("text"),
        }
        // Binary is recognised.
        let b = d.join("blob.dat");
        std::fs::write(&b, [1u8, 0, 2, 3]).unwrap();
        assert!(matches!(load(&b, Kind::Text, 4), Content::Binary));
        // UTF-16 text, as PowerShell writes it, with and without a byte-order mark.
        let le: Vec<u8> = "Handle  Name\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_text(&[&[0xff, 0xfe][..], &le].concat()).as_deref(), Some("Handle  Name\r\n"));
        assert_eq!(decode_text(&le).as_deref(), Some("Handle  Name\r\n"));
        let be: Vec<u8> = "abc".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode_text(&[&[0xfe, 0xff][..], &be].concat()).as_deref(), Some("abc"));
        // A real PNG decodes with its size; a broken one is an error, not a panic.
        let png = d.join("p.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 128])).save(&png).unwrap();
        match load(&png, Kind::Image, 100) {
            Content::Image { size, info, pixels, .. } => {
                assert_eq!(size, [3, 2]);
                assert!(info.starts_with("RGBA"));
                assert_eq!(pixels.unwrap().size, [3, 2]);
            }
            _ => panic!("png"),
        }
        // A drawing: drawn larger than its own size, with its source; broken markup is text.
        let svg = d.join("icon.svg");
        std::fs::write(&svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><rect width="24" height="12" fill="#00f"/></svg>"##).unwrap();
        match load(&svg, Kind::Drawing, 100) {
            Content::Image { size, pixels, source: Some((text, _)), .. } => {
                assert_eq!(size, [24, 12]);
                assert!(pixels.unwrap().size[0] > 24);
                assert!(text.starts_with("<svg"));
            }
            _ => panic!("svg"),
        }
        std::fs::write(&svg, "<svg oops").unwrap();
        assert!(matches!(load(&svg, Kind::Drawing, 9), Content::Text { .. }));
        let bad = d.join("bad.png");
        std::fs::write(&bad, b"\x89PNG\r\n\x1a\nnonsense").unwrap();
        assert!(matches!(load(&bad, Kind::Image, 20), Content::Error(_)));
        assert!(matches!(load(&png, Kind::Image, IMAGE_MAX_FILE + 1), Content::Error(_)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn display_curve() {
        assert_eq!(srgb(0.0), 0.0);
        assert!((srgb(1.0) - 1.0).abs() < 1e-4);
        assert!((srgb(0.18) - 0.46).abs() < 0.01);
        assert_eq!(srgb(-1.0), 0.0);
    }
}

#[cfg(test)]
mod splat_save_tests {
    #[test]
    fn a_splat_view_saves_as_a_png() {
        let dir = std::env::temp_dir().join(format!("filescape-save-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut ply = b"ply\nformat binary_little_endian 1.0\nelement vertex 200\n".to_vec();
        let props = ["x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1", "rot_2", "rot_3"];
        for p in props {
            ply.extend_from_slice(format!("property float {p}\n").as_bytes());
        }
        ply.extend_from_slice(b"end_header\n");
        for i in 0..200u32 {
            let v = [(i % 20) as f32 * 0.1, (i / 20) as f32 * 0.1, 0.0, 1.0, 0.5, 0.0, 4.0, -3.0, -3.0, -3.0, 1.0, 0.0, 0.0, 0.0];
            for x in v {
                ply.extend_from_slice(&x.to_le_bytes());
            }
        }
        let src = dir.join("scene.ply");
        std::fs::write(&src, ply).unwrap();
        let scene = std::sync::Arc::new(crate::splat::read(&src, crate::splat::Limits::PREVIEW).unwrap());
        let dest = dir.join("scene view.png");
        super::save_view(&scene, None, crate::splat::View::default(), [160, 100], &dest).unwrap();
        let img = image::open(&dest).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (160, 100));
        // Something was drawn.
        assert!(img.pixels().filter(|p| p[3] > 100 && p[0] > 60).count() > 200);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
