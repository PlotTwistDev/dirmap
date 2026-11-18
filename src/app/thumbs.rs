//! Thumbnails for the Icons view. Images are decoded on a few worker threads, newest request
//! first, and only for what's on screen: scrolling past a folder of 10,000 renders drops the
//! requests for rows no longer visible. The textures are kept up to a memory budget, the least
//! recently drawn going first. The UI thread never reads a file.

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

/// Whether a file of this name gets a thumbnail: images, and videos, PDFs and sound (album
/// covers) where the system can make a picture of them.
pub(super) fn wants_thumb(name: &str) -> bool {
    matches!(preview::kind_of(name), Kind::Image | Kind::LinearImage | Kind::Media(_))
}

struct Job {
    path: PathBuf,
    stamp: u64,
    size: u64,
}

enum Slot {
    Pending(u64),
    Ready { tex: TextureHandle, stamp: u64, bytes: usize, used: Cell<u64> },
    /// Couldn't be read: the type icon is drawn instead, and it isn't tried again until the
    /// file changes.
    Failed(u64),
}

type Done = (PathBuf, u64, Option<egui::ColorImage>);

#[derive(Default)]
pub(super) struct Thumbs {
    cache: HashMap<PathBuf, Slot>,
    queue: Option<Arc<(Mutex<Vec<Job>>, Condvar)>>,
    rx: Option<mpsc::Receiver<Done>>,
    /// Requests made this frame (drawn cells needing a thumbnail).
    wanted: Vec<(PathBuf, u64, u64)>,
    frame: u64,
    bytes: usize,
}

impl Thumbs {
    /// The thumbnail for a file, if it's ready. Asks for it otherwise (see `flush`).
    pub(super) fn get(&mut self, path: &Path, stamp: u64, size: u64) -> Option<&TextureHandle> {
        let fresh = match self.cache.get(path) {
            Some(Slot::Ready { stamp: s, .. }) | Some(Slot::Pending(s)) | Some(Slot::Failed(s)) => *s == stamp,
            None => false,
        };
        if !fresh {
            self.wanted.push((path.to_path_buf(), stamp, size));
            return None;
        }
        match self.cache.get(path) {
            Some(Slot::Ready { tex, used, .. }) => {
                used.set(self.frame);
                Some(tex)
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
                Some(img) => {
                    let bytes = img.pixels.len() * 4;
                    self.bytes += bytes;
                    let tex = ctx.load_texture(format!("thumb:{}", path.display()), img, egui::TextureOptions::LINEAR);
                    Slot::Ready { tex, stamp, bytes, used: Cell::new(self.frame) }
                }
                None => Slot::Failed(stamp),
            };
            if let Some(Slot::Ready { bytes, .. }) = self.cache.insert(path, slot) {
                self.bytes -= bytes;
            }
        }
        if self.bytes > BUDGET {
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
            if self.bytes <= BUDGET * 3 / 4 {
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
            let (q, tx) = (q.clone(), tx.clone());
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
                    let kind = preview::kind_of(&job.path.to_string_lossy());
                    // On a server: only pictures, fetched when they're small enough.
                    let local = if crate::fsys::remote::is_remote(&job.path) {
                        if matches!(kind, Kind::Image | Kind::LinearImage) && job.size <= REMOTE_MAX { crate::fsys::remote::fetch(&job.path, REMOTE_MAX).ok() } else { None }
                    } else {
                        Some(job.path.clone())
                    };
                    let img = local.and_then(|local| std::panic::catch_unwind(|| match kind {
                        Kind::Media(_) => crate::media::thumbnail(&local, THUMB_MAX).ok().map(|(w, h, rgba)| egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)),
                        _ => preview::decode_image(&local, kind == Kind::LinearImage, job.size, THUMB_MAX).ok().map(|r| r.0),
                    })
                    .ok()
                    .flatten());
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
