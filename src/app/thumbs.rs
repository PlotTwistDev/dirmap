//! Thumbnails for the Icons view. Images are decoded on a few worker threads, newest request
//! first, and only for what's on screen: scrolling past a folder of 10,000 renders drops the
//! requests for rows no longer visible. The textures are kept up to a memory budget, the least
//! recently drawn going first. The UI thread never reads a file.
//!
//! The treemap's tile previews use the same machinery with their own caches (bigger pictures,
//! local files only, text snippets and sound waveforms as well as pictures): see `tilepreview.rs`.

use super::preview::{self, Kind};
use eframe::egui::{self, TextureHandle};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};

/// Thumbnails are decoded no larger than this across; the Icons view's largest size.
pub(super) const THUMB_MAX: u32 = 256;
/// GPU memory the thumbnails may use (RGBA, 4 bytes a pixel).
/// Pictures on a server are fetched for thumbnails up to this size.
const REMOTE_MAX: u64 = 24 << 20;
const BUDGET: usize = 160 << 20;
/// Decoded thumbnails turned into textures per frame, so a burst doesn't stall a frame.
const UPLOADS_PER_FRAME: usize = 12;

/// Which version of a file a thumbnail is of: changes when the file is rewritten.
pub(super) fn stamp(size: u64, modified: i64) -> u64 {
    size.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ modified as u64
}

/// Whether a file of this name gets a thumbnail: images, drawings, fonts, Office documents (the
/// picture saved with them), and videos, PDFs and sound (album covers) where the system can make
/// a picture of them.
pub(super) fn wants_thumb(name: &str) -> bool {
    match preview::kind_of(name) {
        Kind::Image | Kind::LinearImage | Kind::Drawing | Kind::Font | Kind::Raw | Kind::Psd | Kind::Doc | Kind::Saved | Kind::Media(_) => true,
        Kind::Midi | Kind::Model | Kind::Map => crate::thumbcache::rich(),
        Kind::Cad => crate::thumbcache::rich() && name.to_ascii_lowercase().ends_with(".dxf"),
        _ => false,
    }
}

struct Job {
    path: PathBuf,
    stamp: u64,
    size: u64,
}

/// A finished thumbnail: a picture, or for text and documents the first lines of the text.
pub(super) enum Pic {
    Tex(TextureHandle),
    Text(Arc<str>),
    /// A sound's loudness along it (see `crate::wave`).
    Wave(Arc<[f32]>),
}

enum Decoded {
    Image(egui::ColorImage),
    Text(String),
    Wave(Vec<f32>),
}

/// Stretches in a sound's waveform.
const WAVE_BARS: usize = 240;

/// How a cache decodes: the Icons view's default, or the treemap's.
#[derive(Clone, Copy)]
pub(super) struct Config {
    /// Largest side of a picture, in pixels.
    pub max: u32,
    /// Never fetch from servers or take out of archives (zooming must never start a download).
    pub local_only: bool,
    /// Text files and documents give their first lines (`Pic::Text`).
    pub text: bool,
    /// GPU memory the pictures may use.
    pub budget: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config { max: THUMB_MAX, local_only: false, text: false, budget: BUDGET }
    }
}

/// Text kept for a text snippet: enough to fill a big tile.
const SNIPPET_LINES: usize = 80;
const SNIPPET_COLS: usize = 160;

enum Slot {
    Pending(u64),
    Ready { pic: Pic, stamp: u64, bytes: usize, used: Cell<u64> },
    /// Couldn't be read: the type icon is drawn instead, and it isn't tried again until the
    /// file changes.
    Failed(u64),
}

type Done = (PathBuf, u64, Option<Decoded>);

#[derive(Default)]
pub(super) struct Thumbs {
    config: Config,
    cache: HashMap<PathBuf, Slot>,
    queue: Option<Arc<(Mutex<Vec<Job>>, Condvar)>>,
    rx: Option<mpsc::Receiver<Done>>,
    /// Requests made this frame (drawn cells needing a thumbnail).
    wanted: Vec<(PathBuf, u64, u64)>,
    frame: u64,
    bytes: usize,
}

impl Thumbs {
    pub(super) fn with(config: Config) -> Self {
        Thumbs { config, ..Default::default() }
    }

    /// The thumbnail for a file, if it's ready and a picture. Asks for it otherwise (see `flush`).
    pub(super) fn get(&mut self, path: &Path, stamp: u64, size: u64) -> Option<&TextureHandle> {
        match self.pic(path, stamp, size) {
            Some(Pic::Tex(t)) => Some(t),
            _ => None,
        }
    }

    /// The thumbnail for a file (picture or text), if it's ready. Asks for it otherwise.
    pub(super) fn pic(&mut self, path: &Path, stamp: u64, size: u64) -> Option<&Pic> {
        let fresh = match self.cache.get(path) {
            Some(Slot::Ready { stamp: s, .. }) | Some(Slot::Pending(s)) | Some(Slot::Failed(s)) => *s == stamp,
            None => false,
        };
        if !fresh {
            self.wanted.push((path.to_path_buf(), stamp, size));
            return None;
        }
        match self.cache.get(path) {
            Some(Slot::Ready { pic, used, .. }) => {
                used.set(self.frame);
                Some(pic)
            }
            Some(Slot::Pending(_)) => {
                // Still wanted: keep it in the queue.
                self.wanted.push((path.to_path_buf(), stamp, size));
                None
            }
            _ => None,
        }
    }

    /// Once a frame, after drawing: queue what was asked for and drop queued work for files no
    /// longer on screen.
    pub(super) fn flush(&mut self) {
        let wanted = std::mem::take(&mut self.wanted);
        let Some(q) = self.queue.clone().or_else(|| (!wanted.is_empty()).then(|| self.start())) else { return };
        let (lock, cv) = &*q;
        let mut jobs = lock.lock().unwrap();
        let keep: HashSet<&Path> = wanted.iter().map(|w| w.0.as_path()).collect();
        let mut dropped = Vec::new();
        jobs.retain(|j| {
            let k = keep.contains(j.path.as_path());
            if !k {
                dropped.push(j.path.clone());
            }
            k
        });
        for p in dropped {
            if matches!(self.cache.get(&p), Some(Slot::Pending(_))) {
                self.cache.remove(&p);
            }
        }
        // Newest last: the workers take from the end, so the first cells drawn come first.
        for (path, stamp, size) in wanted.into_iter().rev() {
            if matches!(self.cache.get(&path), Some(Slot::Pending(s)) if *s == stamp) {
                continue;
            }
            self.cache.insert(path.clone(), Slot::Pending(stamp));
            jobs.push(Job { path, stamp, size });
        }
        if !jobs.is_empty() {
            cv.notify_all();
        }
    }

    /// Turn finished decodes into textures, and keep within the memory budget. Returns whether
    /// work is still going (so the caller keeps repainting).
    pub(super) fn poll(&mut self, ctx: &egui::Context) -> bool {
        self.frame += 1;
        let Some(rx) = &self.rx else { return false };
        for _ in 0..UPLOADS_PER_FRAME {
            let Ok((path, stamp, img)) = rx.try_recv() else { break };
            // Only if it's still wanted for this version of the file.
            if !matches!(self.cache.get(&path), Some(Slot::Pending(s)) if *s == stamp) {
                continue;
            }
            let slot = match img {
                Some(Decoded::Image(img)) => {
                    let bytes = img.pixels.len() * 4;
                    self.bytes += bytes;
                    let tex = ctx.load_texture(format!("thumb:{}", path.display()), img, egui::TextureOptions::LINEAR);
                    Slot::Ready { pic: Pic::Tex(tex), stamp, bytes, used: Cell::new(self.frame) }
                }
                Some(Decoded::Text(t)) => {
                    let bytes = t.len();
                    self.bytes += bytes;
                    Slot::Ready { pic: Pic::Text(t.into()), stamp, bytes, used: Cell::new(self.frame) }
                }
                Some(Decoded::Wave(w)) => {
                    let bytes = w.len() * 4;
                    self.bytes += bytes;
                    Slot::Ready { pic: Pic::Wave(w.into()), stamp, bytes, used: Cell::new(self.frame) }
                }
                None => Slot::Failed(stamp),
            };
            if let Some(Slot::Ready { bytes, .. }) = self.cache.insert(path, slot) {
                self.bytes -= bytes;
            }
        }
        if self.bytes > self.config.budget {
            self.evict();
        }
        let busy = self.cache.values().any(|s| matches!(s, Slot::Pending(_)));
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        busy
    }

    /// Drop the least recently drawn thumbnails until within budget (never ones drawn this or
    /// last frame).
    fn evict(&mut self) {
        let mut ready: Vec<(u64, PathBuf, usize)> = self
            .cache
            .iter()
            .filter_map(|(p, s)| match s {
                Slot::Ready { used, bytes, .. } if used.get() + 1 < self.frame => Some((used.get(), p.clone(), *bytes)),
                _ => None,
            })
            .collect();
        ready.sort_by_key(|r| r.0);
        for (_, p, b) in ready {
            if self.bytes <= self.config.budget * 3 / 4 {
                break;
            }
            self.cache.remove(&p);
            self.bytes -= b;
        }
    }

    fn start(&mut self) -> Arc<(Mutex<Vec<Job>>, Condvar)> {
        let q: Arc<(Mutex<Vec<Job>>, Condvar)> = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let (tx, rx) = mpsc::channel::<Done>();
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8) / 2;
        for _ in 0..n.max(1) {
            let (q, tx, config) = (q.clone(), tx.clone(), self.config);
            std::thread::spawn(move || {
                loop {
                    let job = {
                        let (lock, cv) = &*q;
                        let mut jobs = lock.lock().unwrap();
                        loop {
                            if let Some(j) = jobs.pop() {
                                break j;
                            }
                            jobs = cv.wait(jobs).unwrap();
                        }
                    };
                    let img = std::panic::catch_unwind(|| decode(&job, config)).ok().flatten();
                    if tx.send((job.path, job.stamp, img)).is_err() {
                        return;
                    }
                }
            });
        }
        self.queue = Some(q.clone());
        self.rx = Some(rx);
        q
    }
}

/// Make one thumbnail. Runs on a worker thread.
fn decode(job: &Job, config: Config) -> Option<Decoded> {
    let name = job.path.to_string_lossy();
    let kind = preview::kind_of(&name);
    // The map: a sound's waveform rather than a cover or an icon.
    if config.text && kind == Kind::Media(crate::media::Media::Audio) {
        if config.local_only && !is_local(&job.path) {
            return None;
        }
        return crate::wave::peaks(&job.path, WAVE_BARS).ok().map(Decoded::Wave);
    }
    if config.text && let Some(doc) = text_kind(&job.path.file_name()?.to_string_lossy()) {
        if config.local_only && !is_local(&job.path) {
            return None;
        }
        return snippet(&job.path, doc).map(Decoded::Text);
    }
    // Made before (in this run or an earlier one): from the disk cache, without reading the file.
    if let Some((w, h, rgba)) = crate::thumbcache::get(&job.path, job.stamp, config.max) {
        return Some(Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)));
    }
    let made = picture(job, config, kind, &name)?;
    if let Decoded::Image(img) = &made {
        let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        crate::thumbcache::put(&job.path, job.stamp, config.max, img.size[0], img.size[1], &rgba);
    }
    Some(made)
}

/// Make a thumbnail picture from the file.
fn picture(job: &Job, config: Config, kind: Kind, name: &str) -> Option<Decoded> {
    let local = if crate::fsys::remote::is_remote(&job.path) {
        // On a server: only pictures, fetched when they're small enough (never for the map).
        if !config.local_only && matches!(kind, Kind::Image | Kind::LinearImage | Kind::Drawing) && job.size <= REMOTE_MAX { crate::fsys::remote::fetch(&job.path, REMOTE_MAX).ok()? } else { return None }
    } else if config.local_only && !is_local(&job.path) {
        return None;
    } else {
        job.path.clone()
    };
    let max = config.max;
    match kind {
        Kind::Media(crate::media::Media::Video) => crate::video::poster(&local, max).ok().map(|(w, h, rgba)| Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))),
        Kind::Media(_) => crate::media::thumbnail(&local, max).ok().map(|(w, h, rgba)| Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))),
        Kind::Image | Kind::LinearImage => preview::decode_image(&local, kind == Kind::LinearImage, job.size, max).ok().map(|r| Decoded::Image(r.0)),
        Kind::Psd => crate::psdview::picture(&local, max).ok().map(|p| {
            let img = image::DynamicImage::ImageRgba8(p.image);
            let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
            let rgba = img.to_rgba8();
            Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba))
        }),
        Kind::Raw => crate::camraw::preview(&local, max).ok().map(|r| {
            let img = if r.image.width() > max || r.image.height() > max { r.image.thumbnail(max, max) } else { r.image };
            let rgba = img.to_rgba8();
            Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba))
        }),
        Kind::Model => crate::model3d::read(&local).ok().map(|m| {
            let n = max.min(512) as usize;
            Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([n, n], &m.draw(n, n, crate::model3d::Angle::default(), 1)))
        }),
        Kind::Cad => crate::cadview::read_dxf(&local).ok().map(|d| {
            let (w, h) = d.fit(max.min(512));
            Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &d.draw(w, h)))
        }),
        Kind::Saved => crate::savedpic::read(&local).ok().and_then(|s| s.picture).map(|img| {
            let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
            let rgba = img.to_rgba8();
            Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba))
        }),
        Kind::Map => crate::geoview::read(&local).ok().map(|g| {
            let (w, h) = g.fit(max.min(512));
            Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &g.draw(w, h)))
        }),
        Kind::Midi => crate::midiview::read(&local).ok().map(|song| {
            let (w, h) = (max, (max * 2 / 3).max(1));
            Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &song.roll(w, h)))
        }),
        Kind::Font => crate::fontview::specimen(&local, max.min(512)).ok().map(|s| Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([s.width as usize, s.height as usize], &s.rgba))),
        Kind::Drawing => crate::svg::render(&local, max).ok().map(|d| Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([d.width as usize, d.height as usize], &d.rgba))),
        // The picture Office and LibreOffice save of the first page or slide.
        Kind::Doc => if name.to_ascii_lowercase().ends_with(".epub") { crate::epub::cover(&local) } else { crate::officedoc::saved_thumbnail(&local) }.map(|img| {
            let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
            let rgba = img.to_rgba8();
            Decoded::Image(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba))
        }),
        _ => None,
    }
}

/// A plain file on this computer (not on a server or inside an archive).
fn is_local(p: &Path) -> bool {
    !crate::fsys::remote::is_remote(p) && !crate::fsys::archive::inside(p)
}

/// Files whose text can be shown as a snippet.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum TextKind {
    Plain,
    /// Word (.docx), or OpenDocument text (.odt): the text from inside the zip.
    Docx,
    Odt,
    /// A spreadsheet: its first rows.
    Sheet,
    /// PowerPoint or OpenDocument slides: their titles and points.
    Slides,
    /// An EPUB book: its first lines.
    Book,
    /// A SQLite database: its first table's first rows.
    Database,
    /// An email, mailbox, contact card or calendar: who, what and when.
    Mail,
}

pub(super) fn text_kind(name: &str) -> Option<TextKind> {
    let f = crate::formats::by_name(name)?;
    match f.id {
        "docx" => Some(TextKind::Docx),
        "sheet" => Some(TextKind::Sheet),
        "csv" => Some(TextKind::Plain),
        "sqlite" => Some(TextKind::Database),
        "odt" => Some(TextKind::Odt),
        "pptx" | "odp" => Some(TextKind::Slides),
        "epub" => Some(TextKind::Book),
        "mail" | "mbox" | "vcard" | "ical" => Some(TextKind::Mail),
        _ => (f.category == crate::formats::Category::Text).then_some(TextKind::Plain),
    }
}

/// The first lines of a text file or document, tabs expanded and long lines cut.
fn snippet(path: &Path, kind: TextKind) -> Option<String> {
    use std::io::Read;
    let raw = match kind {
        TextKind::Plain => {
            let mut buf = Vec::with_capacity(8192);
            std::fs::File::open(path).ok()?.take(12 << 10).read_to_end(&mut buf).ok()?;
            preview::decode_text(&buf)?
        }
        TextKind::Docx | TextKind::Odt => doc_text(path, kind)?,
        TextKind::Sheet => crate::sheets::snippet(&crate::sheets::read(path, 40, 12).ok()?),
        TextKind::Slides => {
            let k = crate::officedoc::kind_of(&path.file_name()?.to_string_lossy())?;
            crate::officedoc::plain_text(&crate::officedoc::read(path, k).ok()?)
        }
        TextKind::Database => crate::sheets::snippet(&crate::tables::read_sqlite(path, 40, 12).ok()?),
        TextKind::Book => crate::officedoc::plain_text(&crate::epub::open(path, 40).ok()?.blocks),
        TextKind::Mail => crate::mailview::snippet(path)?,
    };
    let mut out = String::new();
    for line in raw.lines().take(SNIPPET_LINES) {
        let line: String = line.replace('\t', "    ").chars().filter(|c| !c.is_control()).take(SNIPPET_COLS).collect();
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let out = out.trim_end().to_string();
    (!out.trim().is_empty()).then_some(out)
}

/// The paragraphs of a Word or OpenDocument file, from the XML inside it (limited, so a huge
/// document can't hold up a worker).
pub(super) fn doc_text(path: &Path, kind: TextKind) -> Option<String> {
    use std::io::Read;
    const XML_MAX: u64 = 16 << 20;
    let entry = if kind == TextKind::Docx { "word/document.xml" } else { "content.xml" };
    // w:p in Word; text:p and text:h (headings) in OpenDocument.
    let para = |n: &roxmltree::Node| n.is_element() && (n.tag_name().name() == "p" || kind == TextKind::Odt && n.tag_name().name() == "h");
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let f = zip.by_name(entry).ok()?;
    if f.size() > XML_MAX {
        return None;
    }
    let mut xml = String::new();
    f.take(XML_MAX).read_to_string(&mut xml).ok()?;
    let doc = roxmltree::Document::parse_with_options(&xml, roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() }).ok()?;
    let mut out = String::new();
    for p in doc.descendants().filter(para) {
        // Paragraphs inside paragraphs (text boxes) are read with the one around them.
        if p.ancestors().skip(1).any(|a| para(&a)) {
            continue;
        }
        let mut line = String::new();
        for t in p.descendants() {
            match (kind, t.tag_name().name()) {
                (TextKind::Docx, "t") if t.is_element() => line.push_str(t.text().unwrap_or("")),
                (TextKind::Docx, "tab") if t.is_element() => line.push('\t'),
                (TextKind::Odt, _) if t.is_text() => line.push_str(t.text().unwrap_or("")),
                (TextKind::Odt, "tab") if t.is_element() => line.push('\t'),
                _ => {}
            }
        }
        out.push_str(&line);
        out.push('\n');
        if out.len() > 256 << 10 {
            break;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_files_get_thumbnails() {
        assert!(wants_thumb("shot.0001.exr"));
        assert!(wants_thumb("Photo.JPG"));
        assert!(!wants_thumb("notes.txt"));
        assert!(wants_thumb("clip.mp4"));
        assert!(!wants_thumb("folder"));
        assert_ne!(stamp(10, 5), stamp(10, 6));
        assert_ne!(stamp(10, 5), stamp(11, 5));
    }

    #[test]
    fn thumbnails_load_in_the_background() {
        let dir = std::env::temp_dir().join(format!("dirmap-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        image::RgbaImage::from_pixel(600, 300, image::Rgba([200, 30, 30, 255])).save(&png).unwrap();
        let bad = dir.join("b.png");
        std::fs::write(&bad, b"not a png").unwrap();
        let ctx = egui::Context::default();
        let mut t = Thumbs::default();
        let size = std::fs::metadata(&png).unwrap().len();
        let mut got = None;
        for _ in 0..400 {
            t.poll(&ctx);
            let a = t.get(&png, stamp(size, 1), size).map(|x| x.size());
            let _ = t.get(&bad, stamp(9, 1), 9);
            t.flush();
            if let Some(s) = a
                && matches!(t.cache.get(&bad), Some(Slot::Failed(_)))
            {
                got = Some(s);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Fitted inside THUMB_MAX, keeping the shape.
        assert_eq!(got, Some([256, 128]));
        // A changed file is asked for again.
        assert!(t.get(&png, stamp(size, 2), size).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
