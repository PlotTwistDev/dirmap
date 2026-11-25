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
    },
    /// Sound: played with the controls in the preview, from this file (a fetched copy for one
    /// on a server).
    Sound(PathBuf),
    Text {
        text: String,
        truncated: bool,
    },
    Binary,
    /// Nothing to show beyond the facts.
    None,
    Error(String),
}

/// What Quick Look and the preview pane are showing.
#[derive(Default)]
pub(super) struct Look {
    /// Quick Look is open.
    pub open: bool,
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
    /// Video, sound or PDF: a picture or sound from the system.
    Media(crate::media::Media),
    Other,
}

pub(super) fn kind_of(name: &str) -> Kind {
    let ext = crate::scan::ext_of(name).to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "jpe" | "gif" | "webp" | "tif" | "tiff" | "bmp" | "tga" | "ico" | "qoi" | "pbm" | "pgm" | "ppm" | "pnm" => Kind::Image,
        "exr" | "hdr" => Kind::LinearImage,
        "txt" | "md" | "markdown" | "rst" | "log" | "csv" | "tsv" | "json" | "jsonl" | "xml" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "rs" | "py" | "js"
        | "mjs" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "hpp" | "cc" | "cs" | "java" | "kt" | "go" | "rb" | "php" | "swift" | "m" | "mm" | "sh" | "bash" | "zsh"
        | "fish" | "ps1" | "psm1" | "bat" | "cmd" | "lua" | "sql" | "html" | "htm" | "css" | "scss" | "less" | "vue" | "svelte" | "glsl" | "hlsl" | "wgsl" | "osl"
        | "vex" | "mel" | "nk" | "usda" | "obj" | "mtl" | "gitignore" | "dockerfile" | "makefile" | "cmake" | "gradle" | "properties" | "env" | "srt" | "vtt"
        | "tex" | "bib" | "r" | "jl" | "dart" | "zig" | "nim" | "ex" | "exs" | "erl" | "hs" | "ml" | "clj" | "scala" | "pl" | "pm" | "tcl" | "diff" | "patch"
        | "lock" | "svg" => Kind::Text,
        "" if matches!(name.to_ascii_lowercase().as_str(), "readme" | "license" | "makefile" | "dockerfile" | "changelog" | "authors") => Kind::Text,
        _ => match crate::media::media_of(name) {
            Some(m) => Kind::Media(m),
            None => Kind::Other,
        },
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
                Content::Text { text, truncated }
            }
            Err(e) => Content::Error(e),
        };
    }
    // Word and OpenDocument text: the words in them.
    if let Some(k @ (thumbs::TextKind::Docx | thumbs::TextKind::Odt)) = path.file_name().and_then(|n| thumbs::text_kind(&n.to_string_lossy())) {
        return match thumbs::doc_text(path, k) {
            Some(text) => {
                let truncated = text.len() > TEXT_MAX;
                let mut text = text;
                text.truncate(text.floor_char_boundary(TEXT_MAX));
                Content::Text { text, truncated }
            }
            None => Content::Error("Couldn't read the text in this document".into()),
        };
    }
    match kind {
        Kind::Image | Kind::LinearImage => load_image(path, kind == Kind::LinearImage, size),
        Kind::Text => load_text(path),
        Kind::Media(crate::media::Media::Audio) => Content::Sound(path.to_path_buf()),
        Kind::Media(m) => match crate::media::thumbnail(path, 1024) {
            Ok((w, h, rgba)) => Content::Image {
                pixels: Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)),
                tex: None,
                size: [w as u32, h as u32],
                info: if m == crate::media::Media::Pdf { "The first page".into() } else { "A frame from the video".into() },
                poster: true,
            },
            Err(_) => Content::Error(if m == crate::media::Media::Pdf { "No picture of this PDF: it couldn't be drawn (it may be damaged or password-protected, or the system can't draw PDFs)".into() } else { "No picture of this video: the system has no thumbnailer for it".into() }),
        },
        Kind::Other => {
            // Unknown extension: show it as text if it looks like text.
            match load_text(path) {
                Content::Text { text, truncated } if size <= TEXT_MAX as u64 * 4 => Content::Text { text, truncated },
                Content::Error(e) => Content::Error(e),
                _ => Content::None,
            }
        }
    }
}

fn load_image(path: &Path, linear: bool, size: u64) -> Content {
    match decode_image(path, linear, size, SHOW_MAX) {
        Ok((pixels, size, info)) => Content::Image { pixels: Some(pixels), tex: None, size, info, poster: false },
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
    Content::Text { text, truncated }
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
        let e = self.files.cursor_entry()?;
        Some((self.files.here().dir()?.join(&*e.name), e))
    }

    /// Quick Look: a floating preview of the item under the cursor. Arrow keys move through the
    /// list behind it; Space or Escape close it.
    pub(super) fn quick_look(&mut self, ctx: &egui::Context) {
        if !self.files.look.open {
            return;
        }
        let Some((path, e)) = self.look_target() else {
            self.files.look.open = false;
            return;
        };
        let close = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) || i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
        if close {
            self.files.look.open = false;
            return;
        }
        let screen = ctx.content_rect();
        let size = vec2((screen.width() * 0.72).max(420.0), (screen.height() * 0.78).max(320.0));
        let mut open = true;
        egui::Window::new(egui::RichText::new(format!("{}", e.name)).strong())
            .id(egui::Id::new("quick-look"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(size)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.weak(facts_line(&e));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Open").clicked() {
                            let p = path.clone();
                            self.files.spawn(move || crate::actions::open(&p));
                        }
                        ui.weak(if cfg!(target_os = "macos") { "⏶⏷ next · Space closes" } else { "⏶⏷ next · Space or Esc closes" });
                    });
                });
                ui.separator();
                let avail = ui.available_size();
                self.preview_body(ui, &path, &e, avail);
            });
        if !open {
            self.files.look.open = false;
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
            }
        });
    }

    /// The preview itself: an image, text, or a large icon.
    pub(super) fn preview_body(&mut self, ui: &mut egui::Ui, path: &Path, e: &Entry, size: Vec2) {
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
        match c {
            Content::Image { pixels, tex, size: full, .. } => {
                if let Some(px) = pixels.take() {
                    *tex = Some(ctx.load_texture(format!("look:{}", path.display()), px, egui::TextureOptions::LINEAR));
                }
                let Some(tex) = tex else { return };
                let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let ts = tex.size_vec2();
                // Scale against the full image, so 1:1 means actual pixels even when shown smaller.
                let fit = (r.width() / full[0] as f32).min(r.height() / full[1] as f32).min(1.0);
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
            Content::Text { text, truncated } => {
                let truncated = *truncated;
                let text = text.clone();
                if is_markdown(&e.name) {
                    markdown_view(ui, path, &text, truncated, size);
                    return;
                }
                egui::Frame::new().fill(pal.well).corner_radius(style::RADIUS).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_min_size(size - vec2(16.0, 16.0));
                    ui.set_max_size(size - vec2(16.0, 16.0));
                    egui::ScrollArea::both().id_salt(("look-text", path)).auto_shrink(false).show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&text).monospace().size(12.0)).extend());
                        if truncated {
                            ui.weak("… (only the first 256 KB is shown)");
                        }
                    });
                });
            }
            Content::Sound(play) => {
                let play = play.clone();
                let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                let icon = Rect::from_center_size(r.center() - vec2(0.0, 44.0), vec2(80.0, 80.0));
                self.icons.paint_big(ui, icon, false, e.ext(), tint);
                sound_controls(ui, &mut self.files.look.player, &play, path, Rect::from_center_size(r.center() + vec2(0.0, 47.0), vec2((r.width() - 24.0).min(360.0), 62.0)));
            }
            Content::Binary | Content::None | Content::Error(_) => {
                let note = match c {
                    Content::Error(e) => e.clone(),
                    Content::Binary => "No preview for this kind of file".into(),
                    _ => String::new(),
                };
                let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                self.icons.paint(ui, Rect::from_center_size(r.center() - vec2(0.0, 14.0), vec2(80.0, 80.0)), false, e.ext(), tint);
                ui.painter().text(r.center() + vec2(0.0, 44.0), Align2::CENTER_CENTER, note, FontId::proportional(12.0), pal.weak);
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
fn markdown_view(ui: &mut egui::Ui, path: &Path, text: &str, truncated: bool, size: Vec2) {
    use crate::help::{doc, render};
    static NO_DOCS: std::sync::OnceLock<doc::Docs> = std::sync::OnceLock::new();
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
        egui::ScrollArea::both().id_salt(("look-md", path, raw)).auto_shrink(false).show(ui, |ui| {
            if raw {
                ui.add(egui::Label::new(egui::RichText::new(text).monospace().size(12.0)).extend());
            } else {
                // Parsed once per file shown, not every frame.
                let key = egui::Id::new(("preview-markdown", path, text.len()));
                let blocks = ui.data(|d| d.get_temp::<Arc<Vec<doc::Block>>>(key)).unwrap_or_else(|| {
                    let b = Arc::new(doc::parse(text));
                    ui.data_mut(|d| d.insert_temp(key, b.clone()));
                    b
                });
                ui.set_max_width(ui.available_width());
                let mut images = HashMap::new();
                let mut out = render::Out {
                    page: "",
                    docs: NO_DOCS.get_or_init(|| doc::Docs::from_sources(&[])),
                    images: &mut images,
                    nav: None,
                    request: None,
                    anchors: Vec::new(),
                };
                render::blocks(ui, &mut out, &blocks);
                // Web links open in the browser; links to other files or headings do nothing.
                if let Some(render::Nav::Url(u)) = out.nav.take() {
                    if u.contains("://") || u.starts_with("mailto:") {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(u));
                    }
                }
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
    }

    #[test]
    fn previews_read_safely() {
        let d = std::env::temp_dir().join(format!("dirmap-look-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        // Text, cut at the limit.
        let t = d.join("big.txt");
        std::fs::write(&t, "x".repeat(TEXT_MAX + 10)).unwrap();
        match load(&t, Kind::Text, 0) {
            Content::Text { text, truncated } => assert!(truncated && text.len() == TEXT_MAX),
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
