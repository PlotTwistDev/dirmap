//! The time bar under a splat scene's preview, for scenes that change: a 4D scene (Space-Time
//! Gaussians Lite), scrubbed and played by working out each moment, and numbered splat files
//! in one folder (`take.0001.ply`, `take.0002.ply`, …), played frame by frame. Frames are
//! read in the background, a few ahead, and only frames already on this computer are used.

use super::*;
use crate::splat::{Limits, Scene};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The bar's height.
pub(super) const BAR_H: f32 = 30.0;
/// Frames kept read at once.
const KEEP: usize = 6;
/// Frames shown a second at most while playing numbered files.
const FPS: f64 = 12.0;
/// Seconds a 4D scene takes to play through.
const PLAY_SECS: f32 = 6.0;

#[derive(Default)]
struct State {
    checked: bool,
    /// Numbered files in order, when this file is one of them.
    frames: Vec<(u64, PathBuf)>,
    index: usize,
    /// The 4D scene as read, which every moment is worked out from.
    base: Option<Arc<Scene>>,
    t: f32,
    /// The moment or frame on show.
    shown: Option<Showing>,
    playing: bool,
    last: f64,
    /// Read frames, and those being read.
    cache: HashMap<usize, Arc<Scene>>,
    loading: Vec<usize>,
    failed: HashMap<usize, String>,
    /// A moment being worked out, and the latest one done.
    working: bool,
    moment: Option<(f32, Arc<Scene>)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Frame(usize),
    Moment(f32),
}

type Shared = Arc<Mutex<State>>;

fn shared(ui: &egui::Ui, path: &Path) -> Shared {
    ui.data_mut(|d| d.get_temp_mut_or_default::<Shared>(egui::Id::new(("splat-time", path))).clone())
}

/// The numbered files `path` belongs to, in order, when there are at least three.
fn sequence_of(path: &Path) -> Vec<(u64, PathBuf)> {
    if crate::fsys::remote::is_remote(path) {
        return Vec::new();
    }
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().map(|n| n.to_string_lossy().to_string())) else { return Vec::new() };
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let names: Vec<(String, u64)> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let md = e.metadata().ok()?;
            (md.is_file() && n.to_ascii_lowercase().ends_with(&format!(".{ext}"))).then_some((n, md.len()))
        })
        .take(100_000)
        .collect();
    let refs: Vec<(&str, u64)> = names.iter().map(|(n, s)| (n.as_str(), *s)).collect();
    let Some(me) = names.iter().position(|(n, _)| *n == name) else { return Vec::new() };
    for (seq, idx) in crate::sequences::in_names(&refs, 3) {
        if idx.contains(&me) {
            let mut frames: Vec<(u64, PathBuf)> = idx
                .iter()
                .filter_map(|&i| {
                    let n = &names[i].0;
                    let digits = n.strip_prefix(seq.prefix.as_str())?.strip_suffix(seq.suffix.as_str())?;
                    Some((digits.parse().ok()?, dir.join(n)))
                })
                .collect();
            frames.sort_by_key(|f| f.0);
            return frames;
        }
    }
    Vec::new()
}

/// Whether this scene's preview gets a time bar (looked into once per file).
pub(super) fn wants_bar(ui: &egui::Ui, path: &Path, scene: &Arc<Scene>) -> bool {
    let st = shared(ui, path);
    let mut s = st.lock().unwrap();
    if !s.checked {
        s.checked = true;
        if scene.timed.is_some() {
            s.base = Some(scene.clone());
            s.t = scene.timed.as_ref().map_or(0.0, |t| t.from);
            s.shown = Some(Showing::Moment(s.t));
        } else if scene.format.starts_with("PLY") || scene.format.starts_with("SPZ") || scene.format.starts_with("Compressed PLY") {
            s.frames = sequence_of(path);
            s.index = s.frames.iter().position(|(_, p)| p == path).unwrap_or(0);
            let i = s.index;
            s.cache.insert(i, scene.clone());
            s.shown = Some(Showing::Frame(s.index));
        }
    }
    s.base.is_some() || s.frames.len() >= 3
}

fn load(st: &Shared, i: usize, path: PathBuf, ctx: &egui::Context) {
    st.lock().unwrap().loading.push(i);
    let (st, ctx) = (st.clone(), ctx.clone());
    std::thread::spawn(move || {
        // A frame only in the cloud would download if read: it's skipped.
        let r = if crate::splat::lod::where_is(&path) == crate::splat::lod::Where::Cloud {
            Err("Only in the cloud (not downloaded for the preview)".to_string())
        } else {
            crate::splat::read(&path, Limits::PREVIEW)
        };
        let mut s = st.lock().unwrap();
        s.loading.retain(|&k| k != i);
        match r {
            Ok(scene) => {
                s.cache.insert(i, Arc::new(scene));
            }
            Err(e) => {
                s.failed.insert(i, e);
            }
        }
        // Keep the frames nearest the one on show.
        if s.cache.len() > KEEP {
            let at = s.index as isize;
            let mut keys: Vec<usize> = s.cache.keys().copied().collect();
            keys.sort_by_key(|&k| std::cmp::Reverse((k as isize - at).rem_euclid(s.frames.len().max(1) as isize)));
            for k in keys.into_iter().take(s.cache.len() - KEEP) {
                s.cache.remove(&k);
            }
        }
        ctx.request_repaint();
    });
}

/// Works out the moment `t` in the background. The caller, holding the lock, marks it as
/// working.
fn work_out(st: &Shared, base: Arc<Scene>, t: f32, ctx: &egui::Context) {
    let (st, ctx) = (st.clone(), ctx.clone());
    std::thread::spawn(move || {
        let m = base.at(t).map(Arc::new);
        let mut s = st.lock().unwrap();
        s.working = false;
        if let Some(m) = m {
            s.moment = Some((t, m));
        }
        ctx.request_repaint();
    });
}

/// The bar, in `r`. Puts the moment or frame to show into `scene`, and has it drawn again.
pub(super) fn bar(ui: &mut egui::Ui, r: Rect, path: &Path, scene: &mut Arc<Scene>, drawn: &mut preview::SplatDrawn) {
    let st = shared(ui, path);
    let now = ui.input(|i| i.time);
    let ctx = ui.ctx().clone();
    let mut s = st.lock().unwrap();
    let mut swap: Option<(Arc<Scene>, Showing)> = None;
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(4.0, 3.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let icon = if s.playing { "⏸" } else { "▶" };
    let play = ui.add(egui::Button::new(icon).min_size(vec2(28.0, 22.0))).on_hover_text(if s.playing { "Pause" } else { "Play" });
    if play.clicked() {
        s.playing = !s.playing;
        s.last = now;
    }
    if let Some(base) = s.base.clone() {
        let tm = base.timed.clone().unwrap();
        let (from, to) = (tm.from, tm.to);
        let mut t = s.t;
        if s.playing && !s.working && s.shown == Some(Showing::Moment(s.t)) && drawn.caught_up() {
            let dt = ((now - s.last) as f32).min(0.2);
            t += dt * (to - from) / PLAY_SECS;
            if t > to {
                t = from + (t - to);
            }
            s.last = now;
        } else if !s.playing {
            s.last = now;
        }
        let w = (ui.available_width() - 110.0).max(60.0);
        ui.spacing_mut().slider_width = w;
        if ui.add(egui::Slider::new(&mut t, from..=to).show_value(false)).on_hover_text("Drag to move through time").changed() {
            s.playing = false;
        }
        ui.label(egui::RichText::new(format!("time {}", short(t, to - from))).small());
        s.t = t;
        if let Some((mt, m)) = s.moment.take() {
            swap = Some((m, Showing::Moment(mt)));
        }
        if !s.working && s.shown != Some(Showing::Moment(s.t)) && swap.as_ref().is_none_or(|(_, w)| *w != Showing::Moment(s.t)) {
            work_out(&st, base, s.t, &ctx);
            s.working = true;
        }
        if s.playing {
            ctx.request_repaint();
        }
    } else {
        let n = s.frames.len();
        let mut i = s.index;
        // Playing: on to the next frame once it's read and its moment has come.
        // (and the last one has been drawn, so a slow graphics chip still shows every frame).
        if s.playing && now - s.last >= 1.0 / FPS && drawn.caught_up() {
            let next = (i + 1) % n;
            if s.cache.contains_key(&next) || s.failed.contains_key(&next) {
                i = next;
                s.last = now;
            }
        }
        let w = (ui.available_width() - 150.0).max(60.0);
        ui.spacing_mut().slider_width = w;
        let mut k = i as f64;
        if ui.add(egui::Slider::new(&mut k, 0.0..=(n - 1) as f64).step_by(1.0).show_value(false)).on_hover_text("Drag to go through the frames").changed() {
            s.playing = false;
            i = k as usize;
        }
        let (num, _) = &s.frames[i];
        let mut text = format!("frame {num} ({} of {n})", i + 1);
        if s.loading.contains(&i) {
            text.push_str(" · reading");
        }
        let label = ui.label(egui::RichText::new(text).small());
        if let Some(e) = s.failed.get(&i) {
            label.on_hover_text(e.as_str());
        }
        s.index = i;
        if s.shown != Some(Showing::Frame(i))
            && let Some(sc) = s.cache.get(&i)
        {
            swap = Some((sc.clone(), Showing::Frame(i)));
        }
        // Read the frame on show and a few after it.
        let ahead = if s.playing { 3 } else { 1 };
        for d in 0..=ahead {
            let j = (i + d) % n;
            if !s.cache.contains_key(&j) && !s.loading.contains(&j) && !s.failed.contains_key(&j) && s.loading.len() < 2 {
                let p = s.frames[j].1.clone();
                drop(s);
                load(&st, j, p, &ctx);
                s = st.lock().unwrap();
            }
        }
        if s.playing || !s.loading.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
    if let Some((sc, showing)) = swap {
        *scene = sc;
        s.shown = Some(showing);
        drawn.restart();
    }
}

/// A time to a sensible number of places for the span it's in.
fn short(t: f32, span: f32) -> String {
    if span >= 100.0 {
        format!("{t:.0}")
    } else if span >= 10.0 {
        format!("{t:.1}")
    } else {
        format!("{t:.2}")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn numbered_splat_files_play_in_order() {
        let dir = std::env::temp_dir().join(format!("filescape-splattime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["take.0010.ply", "take.0008.ply", "take.0009.ply", "take.0011.ply", "other.ply", "take.0009.spz", "shot_v2.0001.ply"] {
            std::fs::write(dir.join(n), b"ply").unwrap();
        }
        let f = super::sequence_of(&dir.join("take.0009.ply"));
        assert_eq!(f.iter().map(|(n, p)| (*n, p.file_name().unwrap().to_string_lossy().to_string())).collect::<Vec<_>>(), [(8, "take.0008.ply".into()), (9, "take.0009.ply".into()), (10, "take.0010.ply".into()), (11, "take.0011.ply".into())]);
        assert!(super::sequence_of(&dir.join("other.ply")).is_empty());
        assert!(super::sequence_of(&dir.join("take.0009.spz")).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
