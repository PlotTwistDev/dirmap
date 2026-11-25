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
use std::path::Path;

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
}

/// A video playing in a preview, and its picture.
pub(super) struct VideoView {
    pub video: crate::video::Video,
    tex: Option<egui::TextureHandle>,
    serial: u64,
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
    /// A spreadsheet: its cells.
    Sheet,
    /// Word, PowerPoint or OpenDocument: formatted.
    Doc,
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
    }
}

/// Read a file for its preview. Runs on a worker thread.
fn load(path: &Path, kind: Kind, size: u64) -> Content {
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
            Ok(ix) => {
                let files = ix.items.iter().filter(|i| !i.is_dir).count();
                let bytes: u64 = ix.items.iter().map(|i| i.size).sum();
                let mut text = format!("{} files, {} when extracted\n\n", group(files as u64), crate::scan::human(bytes));
                for it in ix.items.iter().take(2000) {
                    if it.is_dir {
                        text.push_str(&format!("{}/\n", it.path));
                    } else {
                        text.push_str(&format!("{}  ({})\n", it.path, crate::scan::human(it.size)));
                    }
                }
                let truncated = ix.items.len() > 2000;
                Content::Text { text, truncated, colours: None }
            }
            Err(e) => Content::Error(e),
        };
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
        Kind::Psd => match crate::psdview::picture(path, 0) {
            Ok(p) => {
                let img = image::DynamicImage::ImageRgba8(p.image);
                let img = if img.width() > SHOW_MAX || img.height() > SHOW_MAX { img.thumbnail(SHOW_MAX, SHOW_MAX) } else { img };
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                Content::Image {
                    pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)),
                    tex: None,
                    size: p.size,
                    info: if p.thumbnail_only { "Photoshop's small thumbnail (the document is too large to show in full)".into() } else { "The flattened picture".into() },
                    poster: false,
                    source: None,
                    facts: Vec::new(),
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
        Kind::Font => match crate::fontview::specimen(path, 800) {
            Ok(s) => Content::Image {
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
        Kind::Media(m) => match crate::media::thumbnail(path, 1024) {
            Ok((w, h, rgba)) => Content::Image {
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
    match decode_image(path, linear, size, SHOW_MAX) {
        Ok((pixels, size, info)) => Content::Image { pixels: Some(pixels), tex: None, size, info, poster: false, source: None, facts: Vec::new() },
        Err(e) => Content::Error(e),
    }
}

/// Decode an image file for display, no larger than `max` pixels across: the pixels, the full
/// size and a description of the pixel format. Linear-light and float images go through the sRGB
/// curve. Big files and huge dimensions are refused rather than risk the memory. Runs on worker
/// threads (previews and thumbnails).
pub(super) fn decode_image(path: &Path, linear: bool, size: u64, max: u32) -> Result<(egui::ColorImage, [u32; 2], String), String> {
    if size > IMAGE_MAX_FILE {
        return Err(format!("Too big to preview ({}).", crate::scan::human(size)));
    }
    let mut reader = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).map_err(|e| format!("Couldn't open it: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(512 << 20);
    reader.limits(limits);
    let img = match reader.decode() {
        Ok(i) => i,
        Err(image::ImageError::Limits(_)) => return Err("Too large to preview safely.".into()),
        Err(e) => return Err(format!("Couldn't read the image: {e}")),
    };
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
    let v = v.max(0.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.min(1.0).powf(1.0 / 2.4) - 0.055 }
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
        if e.is_dir {
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
    pub(super) fn preview_body(&mut self, ui: &mut egui::Ui, path: &Path, e: &Entry, mut size: Vec2) {
        let pal = style::pal(ui);
        if e.is_dir {
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
        match c {
            Content::Image { pixels, tex, size: full, source, .. } => {
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
                let label = format!("{} × {}  ·  {:.0}%", full[0], full[1], scale * 100.0);
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
            Content::Doc { blocks, .. } => {
                let blocks = blocks.clone();
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
                self.files.look.video = Some(VideoView { video, tex: None, serial: 0 });
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
        let mut at = st.position.min(l);
        ui.spacing_mut().slider_width = (ui.available_width() - tw - 34.0).max(40.0);
        let s = ui.add(egui::Slider::new(&mut at, 0.0..=l).show_value(false));
        if s.drag_stopped() || (s.clicked() && !s.dragged()) {
            v.video.seek(at);
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
            match colours {
                Some(c) => {
                    let spans = if ui.visuals().dark_mode { &c.dark } else { &c.light };
                    ui.add(egui::Label::new(coloured_job(text, spans, ui.visuals().text_color())).wrap());
                }
                None => {
                    ui.add(egui::Label::new(egui::RichText::new(text).monospace().size(12.0)).wrap());
                }
            }
            if truncated {
                ui.weak("… (only the first 256 KB is shown)");
            }
        });
    });
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
                        ui.selectable_value(&mut pick, i, &s.name);
                    }
                });
            });
            ui.data_mut(|d| d.insert_temp(pick_id, pick));
        }
        let sheet = &book.sheets[pick];
        let (h, w) = sheet.size;
        let mut note = format!("{} rows × {} columns", group(h as u64), group(w as u64));
        if h > sheet.rows.len() || w > crate::sheets::COLS {
            note.push_str(&format!(" · the first {} rows and {} columns", sheet.rows.len(), w.min(crate::sheets::COLS)));
        }
        if book.total > book.sheets.len() {
            note.push_str(&format!(" · {} of {} sheets", book.sheets.len(), book.total));
        }
        ui.label(egui::RichText::new(note).size(11.5).color(pal.weak));
        let font = FontId::proportional(12.0);
        let cw = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let cols = sheet.rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let widths: Vec<f32> = (0..cols).map(|c| (sheet.rows.iter().map(|r| r.get(c).map_or(0, |s| s.chars().count())).max().unwrap_or(0).clamp(3, 32) as f32 * cw + 12.0).max(44.0)).collect();
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
                    let (text, colour) = if head { (crate::sheets::column(c), pal.weak) } else { (sheet.rows[i - 1].get(c).cloned().unwrap_or_default(), pal.text) };
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
    if let Some(render::Nav::Url(u)) = out.nav.take()
        && (u.contains("://") || u.starts_with("mailto:"))
    {
        ui.ctx().open_url(egui::OpenUrl::new_tab(u));
    }
}

/// A Word, PowerPoint or OpenDocument file: formatted, or the picture saved with it when its
/// words can't be read.
fn load_doc(path: &Path) -> Content {
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
