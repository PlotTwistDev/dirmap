//! The sequence player: plays an image sequence from memory, over the whole window. Two
//! sequences open together for an A/B compare with a wipe.
//!
//! Frames come from `crate::player::Engine`, which reads ahead of the playhead on worker
//! threads. Playback only shows frames already in memory, so the first pass may be slower than
//! the chosen rate while frames load; the rate actually reached is shown beside it. One frame per
//! track is turned into a texture each time the window is drawn.

use super::App;
use super::style;
use crate::player::{self, Clip, Engine, Slot, View};
use eframe::egui::{self, Align2, Color32, FontId, Key, Modifiers as M, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// What happens at the end.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(super) enum Ends {
    #[default]
    Loop,
    Once,
    PingPong,
}

impl Ends {
    const ALL: [Ends; 3] = [Ends::Loop, Ends::Once, Ends::PingPong];
    fn title(self) -> &'static str {
        match self {
            Ends::Loop => "Loop",
            Ends::Once => "Play Once",
            Ends::PingPong => "Ping-Pong",
        }
    }
}

const RATES: [f32; 9] = [23.976, 24.0, 25.0, 29.97, 30.0, 48.0, 50.0, 59.94, 60.0];

/// Remembered between runs.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct PlayerPrefs {
    /// Frame cache, in GB (0: a quarter of the memory).
    pub cache_gb: f32,
    pub fps: f32,
    pub ends: Ends,
    /// 1 full size, 2 half, 4 quarter.
    pub scale: u32,
    pub view: View,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        Self { cache_gb: 0.0, fps: 24.0, ends: Ends::Loop, scale: 1, view: View::Auto }
    }
}

impl PlayerPrefs {
    pub fn budget(&self) -> u64 {
        if self.cache_gb > 0.0 { (self.cache_gb as f64 * (1u64 << 30) as f64) as u64 } else { player::default_budget() }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Compare {
    A,
    B,
    Wipe,
}

struct Track {
    clip: Clip,
    engine: Engine,
    tex: Option<TextureHandle>,
    /// What the texture shows: (slot, look generation).
    shown: Option<(usize, u64)>,
    size: [u32; 2],
    info: String,
}

pub(super) struct Player {
    tracks: Vec<Track>,
    pos: usize,
    playing: bool,
    dir: isize,
    last_t: Option<f64>,
    acc: f64,
    /// When frames were shown while playing, for the rate reached.
    shown_at: VecDeque<f64>,
    stops: f32,
    /// Bumped when the view or exposure changes, so the textures are redone.
    look: u64,
    luts: HashMap<(View, i32), Arc<Vec<u8>>>,
    /// 1.0 fits the frame to the window.
    zoom: f32,
    pan: Vec2,
    compare: Compare,
    wipe: f32,
    dragging_wipe: bool,
}

impl Player {
    pub fn new(clips: Vec<Clip>, prefs: &PlayerPrefs) -> Player {
        let each = prefs.budget() / clips.len().max(1) as u64;
        let tracks = clips
            .into_iter()
            .take(2)
            .map(|clip| {
                let engine = Engine::new(&clip, each, prefs.scale);
                Track { clip, engine, tex: None, shown: None, size: [0, 0], info: String::new() }
            })
            .collect::<Vec<_>>();
        let two = tracks.len() > 1;
        Player {
            tracks,
            pos: 0,
            playing: false,
            dir: 1,
            last_t: None,
            acc: 0.0,
            shown_at: VecDeque::new(),
            stops: 0.0,
            look: 0,
            luts: HashMap::new(),
            zoom: 1.0,
            pan: Vec2::ZERO,
            compare: if two { Compare::Wipe } else { Compare::A },
            wipe: 0.5,
            dragging_wipe: false,
        }
    }

    fn len(&self) -> usize {
        self.tracks[0].clip.slots.len()
    }

    /// Track `t`'s slot for the playhead (B holds its last frame when it's shorter).
    fn slot_of(&self, t: usize, k: usize) -> usize {
        k.min(self.tracks[t].clip.slots.len().saturating_sub(1))
    }

    /// The frame after `pos` in direction `dir`, and the direction then.
    fn next(&self, ends: Ends, pos: usize, dir: isize) -> Option<(usize, isize)> {
        let n = self.len() as isize;
        let k = pos as isize + dir;
        if (0..n).contains(&k) {
            return Some((k as usize, dir));
        }
        match ends {
            Ends::Loop => Some((k.rem_euclid(n) as usize, dir)),
            Ends::Once => None,
            Ends::PingPong if n > 1 => Some(((pos as isize - dir) as usize, -dir)),
            Ends::PingPong => None,
        }
    }

    /// Every track can show slot `k` now (a missing or unreadable frame counts as shown).
    fn ready(&self, k: usize) -> bool {
        (0..self.tracks.len()).all(|t| !matches!(self.tracks[t].engine.slot(self.slot_of(t, k)), Slot::Loading | Slot::Waiting))
    }

    fn seek(&mut self, k: usize) {
        self.pos = k.min(self.len().saturating_sub(1));
        self.acc = 0.0;
    }

    fn toggle(&mut self, ends: Ends, dir: isize) {
        if self.playing && self.dir == dir {
            self.playing = false;
            return;
        }
        // Played to the end once: start again.
        if ends == Ends::Once && self.next(ends, self.pos, dir).is_none() {
            self.pos = if dir > 0 { 0 } else { self.len() - 1 };
        }
        self.dir = dir;
        self.playing = true;
        self.acc = 0.0;
        self.shown_at.clear();
    }

    /// Move the playhead on by the time passed.
    fn tick(&mut self, now: f64, prefs: &PlayerPrefs) {
        let dt = self.last_t.map_or(0.0, |t| now - t);
        self.last_t = Some(now);
        if !self.playing {
            return;
        }
        let period = 1.0 / prefs.fps.max(1.0) as f64;
        // After a stall, don't rush to catch up.
        self.acc = (self.acc + dt).min(period * 3.0);
        while self.acc >= period {
            match self.next(prefs.ends, self.pos, self.dir) {
                None => {
                    self.playing = false;
                    break;
                }
                Some((k, d)) if self.ready(k) => {
                    self.pos = k;
                    self.dir = d;
                    self.acc -= period;
                    self.shown_at.push_back(now);
                }
                Some(_) => {
                    // Waiting for the frame: play from it as soon as it's there.
                    self.acc = period;
                    break;
                }
            }
        }
        while self.shown_at.front().is_some_and(|&t| now - t > 1.0) {
            self.shown_at.pop_front();
        }
    }

    /// The rate reached over the last second, while playing.
    fn rate(&self) -> Option<f32> {
        (self.playing && self.shown_at.len() > 2).then(|| {
            let span = self.shown_at.back().unwrap() - self.shown_at.front().unwrap();
            ((self.shown_at.len() - 1) as f64 / span.max(1e-3)) as f32
        })
    }

    fn lut(&mut self, view: View) -> Arc<Vec<u8>> {
        let key = (view, (self.stops * 1000.0).round() as i32);
        if self.luts.len() > 16 {
            self.luts.clear();
        }
        self.luts.entry(key).or_insert_with(|| Arc::new(player::lut(view, self.stops))).clone()
    }

    /// Turn each track's current frame into its texture, if it's not showing it already.
    fn upload(&mut self, ctx: &egui::Context, view: View) {
        for t in 0..self.tracks.len() {
            let k = self.slot_of(t, self.pos);
            if self.tracks[t].shown == Some((k, self.look)) {
                continue;
            }
            let Some(f) = self.tracks[t].engine.frame(k) else { continue };
            let lut = self.lut(view.resolve(f.enc));
            let image = egui::ColorImage::new([f.w as usize, f.h as usize], player::to_display(&f, &lut));
            let tr = &mut self.tracks[t];
            match &mut tr.tex {
                Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
                None => tr.tex = Some(ctx.load_texture(format!("seq-player-{t}"), image, egui::TextureOptions::LINEAR)),
            }
            tr.shown = Some((k, self.look));
            tr.size = [f.w, f.h];
            tr.info = f.info.clone();
        }
    }
}

impl App {
    /// Open the player on these sequences (the first two).
    pub(super) fn play_sequences(&mut self, clips: Vec<Clip>) {
        if clips.is_empty() {
            return;
        }
        self.files.player = Some(Player::new(clips, &self.files.prefs.player));
    }

    /// The player over the whole window, while it's open.
    pub(super) fn sequence_player(&mut self, ctx: &egui::Context) {
        let Some(mut pl) = self.files.player.take() else { return };
        let open = player_ui(ctx, &mut pl, &mut self.files.prefs.player);
        if open {
            self.files.player = Some(pl);
        }
    }
}

/// Draw the player and handle its keys; false once it's closed.
fn player_ui(ctx: &egui::Context, pl: &mut Player, prefs: &mut PlayerPrefs) -> bool {
    let n = pl.len();
    if n == 0 {
        return false;
    }
    let mut close = false;
    ctx.input_mut(|i| {
        if i.consume_key(M::NONE, Key::Escape) {
            close = true;
        }
        if i.consume_key(M::NONE, Key::Space) || i.consume_key(M::NONE, Key::K) && pl.playing {
            if pl.playing {
                pl.playing = false;
            } else {
                pl.toggle(prefs.ends, pl.dir);
            }
        }
        if i.consume_key(M::NONE, Key::L) {
            pl.toggle(prefs.ends, 1);
        }
        if i.consume_key(M::NONE, Key::J) {
            pl.toggle(prefs.ends, -1);
        }
        let step = |i: &mut egui::InputState, key| i.consume_key(M::NONE, key) || i.consume_key(M::SHIFT, key);
        if step(i, Key::ArrowRight) {
            pl.playing = false;
            let by = if i.modifiers.shift { 10 } else { 1 };
            pl.seek((pl.pos + by).min(n - 1));
        }
        if step(i, Key::ArrowLeft) {
            pl.playing = false;
            let by = if i.modifiers.shift { 10 } else { 1 };
            pl.seek(pl.pos.saturating_sub(by));
        }
        if i.consume_key(M::NONE, Key::Home) {
            pl.seek(0);
        }
        if i.consume_key(M::NONE, Key::End) {
            pl.seek(n - 1);
        }
        if i.consume_key(M::NONE, Key::F) {
            pl.zoom = 1.0;
            pl.pan = Vec2::ZERO;
        }
    });
    if close {
        return false;
    }
    let now = ctx.input(|i| i.time);
    pl.tick(now, prefs);
    for t in &pl.tracks {
        t.engine.seek(pl.slot_of(0, pl.pos).min(t.clip.slots.len().saturating_sub(1)), pl.dir);
    }
    pl.upload(ctx, prefs.view);
    if pl.playing {
        ctx.request_repaint();
    } else if pl.tracks.iter().any(|t| t.engine.busy()) {
        // The cache bar fills in as frames load.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    let screen = ctx.content_rect();
    let pal = style::DARK;
    let mut keep = true;
    egui::Area::new(egui::Id::new("seq-player")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        ui.set_min_size(screen.size());
        ui.painter().rect_filled(screen, 0.0, Color32::from_gray(10));
        let top = Rect::from_min_size(screen.min, vec2(screen.width(), 36.0));
        let bottom = Rect::from_min_max(pos2(screen.left(), screen.bottom() - 88.0), screen.max);
        let view = Rect::from_min_max(pos2(screen.left(), top.bottom()), pos2(screen.right(), bottom.top()));
        ui.painter().rect_filled(top, 0.0, pal.chrome);
        ui.painter().rect_filled(bottom, 0.0, pal.chrome);
        ui.scope_builder(egui::UiBuilder::new().max_rect(top.shrink2(vec2(10.0, 4.0))), |ui| {
            if !top_bar(ui, pl) {
                keep = false;
            }
        });
        viewer(ui, view, pl, prefs);
        ui.scope_builder(egui::UiBuilder::new().max_rect(bottom.shrink2(vec2(10.0, 6.0))), |ui| {
            timeline(ui, pl);
            ui.add_space(6.0);
            controls(ui, pl, prefs);
        });
    });
    keep
}

fn top_bar(ui: &mut egui::Ui, pl: &Player) -> bool {
    let mut keep = true;
    ui.horizontal_centered(|ui| {
        let names: Vec<String> = pl.tracks.iter().enumerate().map(|(i, t)| if pl.tracks.len() > 1 { format!("{}: {}", ["A", "B"][i], t.clip.label) } else { t.clip.label.clone() }).collect();
        ui.label(egui::RichText::new(names.join("    ")).strong().color(Color32::from_gray(230)));
        let t = &pl.tracks[0];
        if t.size[0] > 0 {
            ui.weak(format!("{} × {} · {}", t.size[0], t.size[1], t.info));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").on_hover_text("Esc").clicked() {
                keep = false;
            }
            ui.weak("Space plays · J K L · arrow keys step · F fits · Esc closes");
        });
    });
    keep
}

/// The frames, fitted to `r`, zoomed and panned; the wipe between A and B.
fn viewer(ui: &mut egui::Ui, r: Rect, pl: &mut Player, prefs: &PlayerPrefs) {
    let resp = ui.interact(r, egui::Id::new("seq-player-view"), Sense::click_and_drag());
    let [w, h] = pl.tracks.iter().find(|t| t.size[0] > 0).map_or([16, 9], |t| t.size);
    let fit = (r.width() / w as f32).min(r.height() / h as f32);
    let s = fit * pl.zoom;
    let center = r.center() + pl.pan;
    let img = Rect::from_center_size(center, vec2(w as f32 * s, h as f32 * s));
    // Zoom about the pointer.
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0
            && let Some(p) = ui.input(|i| i.pointer.hover_pos())
        {
            let z = (pl.zoom * (scroll * 0.002).exp()).clamp(0.05, 64.0);
            let at = (p - center) / s;
            pl.zoom = z;
            pl.pan = p - at * fit * z - r.center();
        }
    }
    let wipe_x = img.left() + img.width() * pl.wipe;
    if resp.drag_started() {
        let from = ui.input(|i| i.pointer.press_origin());
        pl.dragging_wipe = pl.compare == Compare::Wipe && from.is_some_and(|p| (p.x - wipe_x).abs() < 10.0);
    }
    if resp.dragged() {
        if pl.dragging_wipe {
            if let Some(p) = resp.interact_pointer_pos() {
                pl.wipe = ((p.x - img.left()) / img.width()).clamp(0.0, 1.0);
            }
        } else {
            pl.pan += resp.drag_delta();
        }
    }
    if resp.double_clicked() {
        pl.zoom = 1.0;
        pl.pan = Vec2::ZERO;
    }
    if pl.compare == Compare::Wipe && resp.hover_pos().is_some_and(|p| (p.x - wipe_x).abs() < 10.0) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let p = ui.painter().with_clip_rect(r);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    let draw = |t: usize, clip: Rect| {
        let tr = &pl.tracks[t];
        let k = pl.slot_of(t, pl.pos);
        let slot = tr.engine.slot(k);
        let pc = p.with_clip_rect(clip.intersect(r));
        if slot == Slot::Missing {
            pc.rect_filled(img, 0.0, Color32::BLACK);
            pc.text(img.center(), Align2::CENTER_CENTER, format!("Frame {} is missing", tr.clip.frame_number(k)), FontId::proportional(16.0), Color32::from_rgb(230, 120, 100));
            return;
        }
        if let Slot::Failed(e) = &slot {
            pc.rect_filled(img, 0.0, Color32::BLACK);
            pc.text(img.center(), Align2::CENTER_CENTER, format!("Frame {}: {e}", tr.clip.frame_number(k)), FontId::proportional(14.0), Color32::from_rgb(230, 120, 100));
            return;
        }
        if let Some(tex) = &tr.tex {
            pc.image(tex.id(), img, uv, Color32::WHITE);
        }
        if tr.shown.is_none_or(|(s, _)| s != k) {
            pc.text(img.left_top() + vec2(8.0, 8.0), Align2::LEFT_TOP, format!("Loading frame {}…", tr.clip.frame_number(k)), FontId::proportional(13.0), Color32::from_gray(200));
        }
    };
    match pl.compare {
        Compare::A => draw(0, r),
        Compare::B => draw(1, r),
        Compare::Wipe => {
            draw(0, Rect::from_min_max(r.min, pos2(wipe_x, r.bottom())));
            draw(1, Rect::from_min_max(pos2(wipe_x, r.top()), r.max));
            let line = Color32::from_gray(235);
            p.line_segment([pos2(wipe_x, img.top()), pos2(wipe_x, img.bottom())], Stroke::new(1.5, line));
            p.circle_filled(pos2(wipe_x, img.center().y), 6.0, line);
            let tag = |at: Pos2, a: Align2, s: &str| {
                p.text(at, a, s, FontId::proportional(14.0), line);
            };
            tag(pos2(wipe_x - 8.0, img.top() + 8.0), Align2::RIGHT_TOP, "A");
            tag(pos2(wipe_x + 8.0, img.top() + 8.0), Align2::LEFT_TOP, "B");
        }
    }
    let _ = prefs;
}

/// The frames from first to last: what's in memory, what's missing, and the playhead. Click or
/// drag to move to a frame.
fn timeline(ui: &mut egui::Ui, pl: &mut Player) {
    let pal = style::DARK;
    let n = pl.len();
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(r, 3.0, pal.well);
    let x_of = |k: usize| r.left() + r.width() * k as f32 / n as f32;
    // Runs of frames in memory, and missing or unreadable ones, along the bottom.
    let bar = Rect::from_min_max(pos2(r.left(), r.bottom() - 5.0), r.max);
    let map = pl.tracks[0].engine.map();
    let mut k = 0;
    while k < n {
        let (cached, bad) = map[k];
        let mut e = k + 1;
        while e < n && map[e] == (cached, bad) {
            e += 1;
        }
        let colour = if bad {
            Some(Color32::from_rgb(200, 70, 60))
        } else if cached {
            Some(pal.accent)
        } else {
            None
        };
        if let Some(c) = colour {
            p.rect_filled(Rect::from_min_max(pos2(x_of(k), bar.top()), pos2(x_of(e).max(x_of(k) + 1.0), bar.bottom())), 0.0, c);
        }
        k = e;
    }
    let clip = &pl.tracks[0].clip;
    let font = FontId::proportional(11.0);
    p.text(pos2(r.left() + 4.0, r.top() + 3.0), Align2::LEFT_TOP, clip.frame_number(0).to_string(), font.clone(), pal.weak);
    p.text(pos2(r.right() - 4.0, r.top() + 3.0), Align2::RIGHT_TOP, clip.frame_number(n - 1).to_string(), font.clone(), pal.weak);
    let x = x_of(pl.pos) + r.width() / n as f32 / 2.0;
    p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(2.0, Color32::from_gray(240)));
    if (resp.dragged() || resp.clicked())
        && let Some(at) = resp.interact_pointer_pos()
    {
        pl.playing = false;
        let k = (((at.x - r.left()) / r.width()) * n as f32).floor().clamp(0.0, (n - 1) as f32) as usize;
        pl.seek(k);
    }
    resp.on_hover_text("The bar along the bottom shows the frames in memory; red ones are missing or couldn't be read");
}

fn controls(ui: &mut egui::Ui, pl: &mut Player, prefs: &mut PlayerPrefs) {
    let n = pl.len();
    ui.horizontal(|ui| {
        if ui.button("⏮").on_hover_text("First frame (Home)").clicked() {
            pl.seek(0);
        }
        if ui.button("⏴").on_hover_text("Back a frame (←)").clicked() {
            pl.playing = false;
            pl.seek(pl.pos.saturating_sub(1));
        }
        let label = if pl.playing { "⏸" } else { "▶" };
        if ui.button(label).on_hover_text("Play or pause (Space)").clicked() {
            if pl.playing {
                pl.playing = false;
            } else {
                pl.toggle(prefs.ends, 1);
            }
        }
        if ui.button("⏵").on_hover_text("On a frame (→)").clicked() {
            pl.playing = false;
            pl.seek((pl.pos + 1).min(n - 1));
        }
        if ui.button("⏭").on_hover_text("Last frame (End)").clicked() {
            pl.seek(n - 1);
        }
        let clip = &pl.tracks[0].clip;
        ui.label(egui::RichText::new(format!("{}", clip.frame_number(pl.pos))).monospace().strong());
        ui.weak(format!("{} of {n}", pl.pos + 1));
        ui.separator();
        egui::ComboBox::from_id_salt("seq-ends").selected_text(prefs.ends.title()).show_ui(ui, |ui| {
            for e in Ends::ALL {
                ui.selectable_value(&mut prefs.ends, e, e.title());
            }
        });
        egui::ComboBox::from_id_salt("seq-fps").selected_text(format!("{} fps", fmt_rate(prefs.fps))).show_ui(ui, |ui| {
            for r in RATES {
                ui.selectable_value(&mut prefs.fps, r, format!("{} fps", fmt_rate(r)));
            }
        });
        match pl.rate() {
            Some(r) => {
                let slow = r < prefs.fps * 0.97;
                let text = egui::RichText::new(format!("playing at {r:.1}")).small();
                ui.label(if slow { text.color(Color32::from_rgb(230, 170, 80)) } else { text.weak() })
                    .on_hover_text(if slow { "Slower than the chosen rate: frames are still being read. Once they're all in memory it plays at full speed." } else { "The rate reached over the last second" });
            }
            None => {
                ui.weak(egui::RichText::new("").small());
            }
        }
        ui.separator();
        let before = (prefs.view, pl.stops);
        egui::ComboBox::from_id_salt("seq-view").selected_text(prefs.view.title()).show_ui(ui, |ui| {
            for v in View::ALL {
                ui.selectable_value(&mut prefs.view, v, v.title());
            }
        })
        .response
        .on_hover_text("How the values are shown. Automatic: float frames as linear light on an sRGB display, log DPX through the Cineon curve, the rest as stored");
        ui.add(egui::DragValue::new(&mut pl.stops).speed(0.05).range(-12.0..=12.0).fixed_decimals(1).prefix("Exposure ").suffix(" stops"));
        if pl.stops != 0.0 && ui.small_button("0").on_hover_text("Reset the exposure").clicked() {
            pl.stops = 0.0;
        }
        if (prefs.view, pl.stops) != before {
            pl.look += 1;
        }
        ui.separator();
        let scale = prefs.scale;
        egui::ComboBox::from_id_salt("seq-scale").selected_text(scale_title(scale)).show_ui(ui, |ui| {
            for s in [1, 2, 4] {
                ui.selectable_value(&mut prefs.scale, s, scale_title(s));
            }
        })
        .response
        .on_hover_text("Smaller frames fit more of the sequence in memory and play more smoothly");
        if prefs.scale != scale {
            for t in &mut pl.tracks {
                t.engine.set_scale(prefs.scale);
                t.shown = None;
            }
        }
        if pl.tracks.len() > 1 {
            ui.separator();
            for (c, label) in [(Compare::A, "A"), (Compare::Wipe, "Wipe"), (Compare::B, "B")] {
                if ui.add(egui::Button::selectable(pl.compare == c, label)).clicked() {
                    pl.compare = c;
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (used, budget) = pl.tracks.iter().fold((0, 0), |(u, b), t| {
                let (x, y) = t.engine.memory();
                (u + x, b + y)
            });
            let before = prefs.cache_gb;
            ui.menu_button(egui::RichText::new(format!("In memory: {} of {}", crate::scan::human(used), crate::scan::human(budget))).small(), |ui| {
                let ram = player::total_ram();
                let mut auto = prefs.cache_gb <= 0.0;
                if ui.checkbox(&mut auto, "Automatic: a quarter of the computer's memory").changed() {
                    prefs.cache_gb = if auto { 0.0 } else { (player::default_budget() >> 30).max(1) as f32 };
                }
                if !auto {
                    let max = if ram > 0 { ((ram as f64 * 0.75) / (1u64 << 30) as f64).floor().max(1.0) as f32 } else { 64.0 };
                    ui.add(egui::Slider::new(&mut prefs.cache_gb, 0.5..=max).step_by(0.5).suffix(" GB").text("Frame memory"));
                }
                ui.weak("More memory holds more of a sequence, so more of it plays at full speed.");
            })
            .response
            .on_hover_text("How much memory the frames may use");
            if prefs.cache_gb != before {
                let each = prefs.budget() / pl.tracks.len() as u64;
                for t in &pl.tracks {
                    t.engine.set_budget(each);
                }
            }
        });
    });
}

fn fmt_rate(r: f32) -> String {
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r}") }
}

fn scale_title(s: u32) -> &'static str {
    match s {
        2 => "Half Size",
        4 => "Quarter Size",
        _ => "Full Size",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(n: usize) -> Clip {
        Clip { label: "x".into(), first: 1001, step: 1, slots: vec![None; n] }
    }

    #[test]
    fn ends_of_the_sequence() {
        let pl = Player::new(vec![clip(3)], &PlayerPrefs { cache_gb: 0.01, ..Default::default() });
        assert_eq!(pl.next(Ends::Loop, 2, 1), Some((0, 1)));
        assert_eq!(pl.next(Ends::Loop, 0, -1), Some((2, -1)));
        assert_eq!(pl.next(Ends::Once, 2, 1), None);
        assert_eq!(pl.next(Ends::PingPong, 2, 1), Some((1, -1)));
        assert_eq!(pl.next(Ends::PingPong, 0, -1), Some((1, 1)));
        // Missing frames don't hold playback up.
        assert!(pl.ready(1));
    }

    #[test]
    fn plays_at_the_rate_and_stops_once() {
        let prefs = PlayerPrefs { cache_gb: 0.01, fps: 10.0, ends: Ends::Once, ..Default::default() };
        let mut pl = Player::new(vec![clip(5)], &prefs);
        pl.toggle(prefs.ends, 1);
        pl.tick(0.0, &prefs);
        for i in 1..=3 {
            pl.tick(i as f64 * 0.1 + 1e-6, &prefs);
        }
        assert_eq!(pl.pos, 3);
        pl.tick(1.0, &prefs);
        assert_eq!(pl.pos, 4);
        assert!(!pl.playing);
        // Space at the end plays from the start.
        pl.toggle(prefs.ends, 1);
        assert_eq!((pl.pos, pl.playing), (0, true));
    }
}
