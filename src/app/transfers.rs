//! The transfer queue: copies and moves that involve a server. They run several at once in
//! the background, each on its own connection (a "lane", see `remote::set_lane`), so browsing
//! stays quick while files go up and down. Local copies keep using the one-at-a-time `Ops`
//! job; both share `transfer::transfer_with`, the clash question and Undo.
//!
//! Files are written under a part name and renamed into place when complete (see
//! `remote::download_file` / `upload_file`), so Retry, or the same copy asked for again after
//! dirmap was closed, continues from where it got to.

use super::menus::{ClashAsk, UNDO_MAX, UndoEntry};
use super::*;
use crate::fsys::PathShow;
use crate::fsys::remote;
use crate::transfer::{self, Choice, Clash, Ctl, Outcome};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;
use std::time::Instant;

/// What's remembered between runs (`DeskSave::transfers`).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct TransferPrefs {
    /// How many transfers run at once.
    pub at_once: u8,
    pub panel_height: f32,
}

impl Default for TransferPrefs {
    fn default() -> Self {
        Self { at_once: 2, panel_height: 190.0 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum State {
    Waiting,
    Running,
    Done,
    Failed,
    Stopped,
}

pub(super) struct Job {
    pub id: u64,
    pub sources: Vec<PathBuf>,
    pub dest: PathBuf,
    pub copy: bool,
    pub ctl: Arc<Ctl>,
    pub state: State,
    /// A second go after a failure: files already there at the same size are skipped and
    /// folders merged, instead of asking.
    pub retry: bool,
    pub started: Option<Instant>,
    pub took: Option<Duration>,
    /// What happened, once finished.
    pub note: String,
    pub errors: Vec<String>,
    /// A sync's plan between `sources[0]` (left) and `dest` (right), instead of a copy or move.
    pub sync: Option<Arc<Vec<(super::compare::Act, String)>>>,
    clash: Option<ClashAsk>,
    rx: Option<mpsc::Receiver<Outcome>>,
    lane: u8,
    /// (when, bytes done) over the last few seconds, for the speed.
    samples: VecDeque<(Instant, u64)>,
}

impl Job {
    pub fn label(&self) -> String {
        if let Some(plan) = &self.sync {
            return format!("Sync {} item{}: {} and {}", plan.len(), if plan.len() == 1 { "" } else { "s" }, self.sources[0].shown(), self.dest.shown());
        }
        let what = if self.sources.len() == 1 {
            self.sources[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.sources[0].shown())
        } else {
            format!("{} items", self.sources.len())
        };
        format!("{} {what} to {}", if self.copy { "Copy" } else { "Move" }, self.dest.shown())
    }

    pub fn finished(&self) -> bool {
        matches!(self.state, State::Done | State::Failed | State::Stopped)
    }

    /// Bytes a second over the last few seconds.
    pub fn speed(&self) -> Option<f64> {
        let (a, b) = (self.samples.front()?, self.samples.back()?);
        let dt = b.0.duration_since(a.0).as_secs_f64();
        (dt >= 0.5).then(|| b.1.saturating_sub(a.1) as f64 / dt)
    }

    /// Time left at the current speed.
    pub fn eta(&self) -> Option<Duration> {
        let s = self.speed().filter(|s| *s > 1.0)?;
        let (t, d) = (self.ctl.bytes_total.load(Ordering::Relaxed), self.ctl.bytes_done.load(Ordering::Relaxed));
        (t > d).then(|| Duration::from_secs_f64((t - d) as f64 / s))
    }

    fn sample(&mut self) {
        // Paused time doesn't count towards the speed.
        if self.ctl.paused.load(Ordering::Relaxed) {
            self.samples.clear();
            return;
        }
        let now = Instant::now();
        self.samples.push_back((now, self.ctl.bytes_done.load(Ordering::Relaxed)));
        while self.samples.len() > 2 && now.duration_since(self.samples[0].0) > Duration::from_secs(5) {
            self.samples.pop_front();
        }
    }
}

#[derive(Default)]
pub(super) struct Queue {
    pub jobs: Vec<Job>,
    next: u64,
    pub prefs: Option<TransferPrefs>,
    /// The panel is open.
    pub open: bool,
    /// The job whose clash question is in `Ops::clash` now.
    lent: Option<u64>,
    /// Server files open in apps here, whose saves are sent back.
    pub edits: Vec<remote::edit::Editing>,
}

/// How many finished transfers the list keeps.
const HISTORY: usize = 100;

impl Queue {
    pub fn active(&self) -> usize {
        self.jobs.iter().filter(|j| !j.finished()).count()
    }

    fn at_once(&self) -> usize {
        self.prefs.as_ref().map_or(2, |p| p.at_once.clamp(1, 8) as usize)
    }

    fn add(&mut self, sources: Vec<PathBuf>, dest: PathBuf, copy: bool, retry: bool) -> u64 {
        self.next += 1;
        self.jobs.push(Job {
            id: self.next,
            sources,
            dest,
            copy,
            ctl: Arc::new(Ctl::default()),
            state: State::Waiting,
            retry,
            sync: None,
            started: None,
            took: None,
            note: String::new(),
            errors: Vec::new(),
            clash: None,
            rx: None,
            lane: 0,
            samples: VecDeque::new(),
        });
        self.next
    }

    /// Start waiting jobs while there are free lanes.
    fn fill(&mut self) {
        let at_once = self.at_once();
        loop {
            let busy: Vec<u8> = self.jobs.iter().filter(|j| j.state == State::Running).map(|j| j.lane).collect();
            if busy.len() >= at_once {
                return;
            }
            let Some(j) = self.jobs.iter_mut().find(|j| j.state == State::Waiting && !j.ctl.paused.load(Ordering::Relaxed)) else { return };
            let lane = (1..=8u8).find(|l| !busy.contains(l)).unwrap_or(1);
            start(j, lane);
        }
    }
}

/// Run a job on its own thread and lane.
fn start(job: &mut Job, lane: u8) {
    let (ask_tx, ask_rx) = mpsc::channel::<(Clash, mpsc::Sender<(Choice, bool)>)>();
    job.clash = Some(ClashAsk { rx: ask_rx, current: None, all: false, copy: job.copy });
    let (tx, rx) = mpsc::channel();
    job.rx = Some(rx);
    job.lane = lane;
    job.state = State::Running;
    job.started = Some(Instant::now());
    job.samples.clear();
    let (ctl, sources, dest, copy, retry) = (job.ctl.clone(), job.sources.clone(), job.dest.clone(), job.copy, job.retry);
    if let Some(plan) = job.sync.clone() {
        std::thread::spawn(move || {
            remote::set_lane(lane);
            let out = super::compare::sync_items(&sources[0], &dest, &plan, retry, &ctl);
            remote::set_lane(0);
            let _ = tx.send(out);
        });
        return;
    }
    std::thread::spawn(move || {
        remote::set_lane(lane);
        // A retried move: what went across already is gone from where it was.
        let sources: Vec<PathBuf> = if retry && !copy { sources.into_iter().filter(|s| exists(s)).collect() } else { sources };
        let (mut for_files, mut for_dirs) = (None, if retry { Some(Choice::Merge) } else { None });
        let mut ask = |c: &Clash| {
            // On a retry, a file there at the same size and time arrived last time: part
            // files only take the real name once complete, and get the source's time then.
            if retry && !c.src_dir && !c.dest_dir && c.src_size == c.dest_size && same_time(c) {
                return Choice::Skip;
            }
            let slot = if c.src_dir && c.dest_dir { &mut for_dirs } else { &mut for_files };
            if let Some(ch) = *slot {
                return ch;
            }
            let (tx, rx) = mpsc::channel();
            if ask_tx.send((c.clone(), tx)).is_err() {
                return Choice::Stop;
            }
            match rx.recv() {
                Ok((ch, all)) => {
                    if all {
                        *slot = Some(ch);
                    }
                    ch
                }
                Err(_) => Choice::Stop,
            }
        };
        let out = transfer::transfer_with(&sources, &dest, copy, &mut ask, &ctl);
        remote::set_lane(0);
        let _ = tx.send(out);
    });
}

/// Modified within two seconds of each other (FTP and FAT keep times to the second or two).
fn same_time(c: &Clash) -> bool {
    match (c.src_modified, c.dest_modified) {
        (Some(a), Some(b)) => a.duration_since(b).or_else(|_| b.duration_since(a)).is_ok_and(|d| d <= Duration::from_secs(2)),
        _ => false,
    }
}

fn exists(p: &Path) -> bool {
    if remote::is_remote(p) { remote::meta(p).is_ok() } else { p.symlink_metadata().is_ok() }
}

fn short_time(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else {
        format!("{} h {:02} min", s / 3600, s / 60 % 60)
    }
}

/// "12.4 MB/s"
pub(super) fn rate(bps: f64) -> String {
    format!("{}/s", crate::scan::human(bps as u64))
}

impl App {
    /// A copy or move that involves a server joins the transfer queue instead of waiting for
    /// the one-at-a-time file operations. True when it did.
    pub(super) fn queue_transfer(&mut self, sources: &[PathBuf], dest: &Path, copy: bool) -> bool {
        if !remote::is_remote(dest) && !sources.iter().any(|s| remote::is_remote(s)) {
            return false;
        }
        // New server transfers need a licence; retries and resumes (Queue::add directly) never
        // check, so expiry can't interrupt them. Handled either way: no local copy instead.
        if !self.premium(crate::licence::Feature::RemoteTransfers) {
            return true;
        }
        let copy = copy || sources.iter().any(|s| crate::fsys::archive::inside(s));
        let sources: Vec<PathBuf> = sources.iter().filter(|s| copy || s.parent() != Some(dest)).cloned().collect();
        if sources.is_empty() {
            return true;
        }
        let q = &mut self.files.rws.queue;
        if q.prefs.is_none() {
            q.prefs = Some(self.files.prefs.remote_desk.transfers.clone());
        }
        q.add(sources, dest.to_path_buf(), copy, false);
        q.fill();
        let n = q.active();
        self.ops.notice = Some((
            if n == 1 { "Transfer started. It runs in the background; the Transfers chip below shows how it's going.".into() } else { format!("Added to the transfers ({n} in the queue).") },
            false,
        ));
        true
    }

    /// A sync with a server on either side: one job in the queue, the plan already confirmed.
    pub(super) fn queue_sync(&mut self, left: PathBuf, right: PathBuf, plan: Vec<(super::compare::Act, String)>) {
        let q = &mut self.files.rws.queue;
        if q.prefs.is_none() {
            q.prefs = Some(self.files.prefs.remote_desk.transfers.clone());
        }
        let id = q.add(vec![left], right, true, false);
        if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id) {
            j.sync = Some(Arc::new(plan));
        }
        q.fill();
        q.open = true;
        self.ops.notice = Some(("Sync started. It runs in the background, in the Transfers panel.".into(), false));
    }

    /// Collect finished transfers, start waiting ones, and pass clash questions to the clash
    /// window. Call once per frame, before `ops_ui`.
    pub(super) fn transfers_poll(&mut self, ctx: &egui::Context) {
        // Saves of server files being edited, sent back in the background.
        let sent = remote::edit::take_events();
        self.files.rws.queue.edits = remote::edit::editing();
        if !self.files.rws.queue.edits.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(1000));
        }
        if let Some((text, error)) = sent.last().cloned() {
            self.ops.notice = Some((text, error));
            self.files_after_op(&[], false);
        }
        let q = &mut self.files.rws.queue;
        if q.jobs.is_empty() {
            return;
        }
        // A question answered: its asker goes back to its job.
        if let Some(id) = q.lent
            && self.ops.clash.as_ref().is_none_or(|c| c.current.is_none())
        {
            q.lent = None;
            if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id) {
                j.clash = self.ops.clash.take();
            } else {
                self.ops.clash = None;
            }
        }
        let mut finished: Vec<(Outcome, String, bool)> = Vec::new();
        for j in q.jobs.iter_mut().filter(|j| j.state == State::Running) {
            j.sample();
            let Some(rx) = &j.rx else { continue };
            let out = match rx.try_recv() {
                Ok(out) => out,
                Err(mpsc::TryRecvError::Empty) => continue,
                Err(mpsc::TryRecvError::Disconnected) => Outcome { errors: vec!["The transfer stopped unexpectedly.".into()], ..Default::default() },
            };
            j.rx = None;
            j.clash = None;
            if q.lent == Some(j.id) {
                q.lent = None;
                self.ops.clash = None;
            }
            j.took = j.started.map(|s| s.elapsed());
            j.state = if !out.errors.is_empty() {
                State::Failed
            } else if out.stopped {
                State::Stopped
            } else {
                State::Done
            };
            j.errors = out.errors.clone();
            let n = out.made.len();
            let verb = if j.copy { "Copied" } else { "Moved" };
            let mut note = match n {
                0 => String::new(),
                n if j.sync.is_some() => format!("Synced {n} item{}", if n == 1 { "" } else { "s" }),
                1 => format!("{verb} to {}", j.dest.shown()),
                n => format!("{verb} {n} items to {}", j.dest.shown()),
            };
            if out.skipped > 0 {
                note.push_str(&format!("{}{} skipped", if note.is_empty() { "" } else { "; " }, if out.skipped == 1 { "1 item".into() } else { format!("{} items", out.skipped) }));
            }
            if out.stopped {
                note.push_str(if note.is_empty() { "Stopped" } else { "; stopped" });
            }
            if let Some(t) = j.took
                && j.ctl.bytes_done.load(Ordering::Relaxed) > 0
            {
                let b = j.ctl.bytes_done.load(Ordering::Relaxed);
                note.push_str(&format!(" ({} in {}, {})", crate::scan::human(b), short_time(t), rate(b as f64 / t.as_secs_f64().max(0.001))));
            }
            let failed = !out.errors.is_empty();
            let text = if failed { format!("{}: {}", j.label(), out.errors.join(" ")) } else { note.clone() };
            j.note = note;
            finished.push((out, text, failed));
        }
        // The next question, if nothing else is asking.
        if q.lent.is_none() && self.ops.clash.is_none() {
            for j in q.jobs.iter_mut().filter(|j| j.state == State::Running) {
                let Some(c) = &mut j.clash else { continue };
                if c.current.is_none()
                    && let Ok(x) = c.rx.try_recv()
                {
                    c.current = Some(x);
                }
                if c.current.is_some() {
                    q.lent = Some(j.id);
                    self.ops.clash = j.clash.take();
                    break;
                }
            }
        }
        q.fill();
        while q.jobs.iter().filter(|j| j.finished()).count() > HISTORY {
            let Some(i) = q.jobs.iter().position(|j| j.finished()) else { break };
            q.jobs.remove(i);
        }
        let busy = q.active() > 0;
        for (out, text, failed) in finished {
            if !out.steps.is_empty() {
                self.marks_follow(&out.steps);
                self.ops.redo.clear();
                self.ops.undo.push(UndoEntry { label: transfer::describe(&out.steps), steps: out.steps });
                if self.ops.undo.len() > UNDO_MAX {
                    self.ops.undo.remove(0);
                }
                self.ops.undo_fresh = !failed;
            }
            if !text.is_empty() {
                self.ops.notice = Some((text, failed));
            }
            self.files_after_op(&[], false);
        }
        if busy {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// The status bar's transfers chip: what's running and how fast. Opens the panel.
    pub(super) fn transfers_chip(&mut self, ui: &mut egui::Ui) {
        let q = &self.files.rws.queue;
        if q.jobs.is_empty() && q.edits.is_empty() {
            return;
        }
        let running: Vec<&Job> = q.jobs.iter().filter(|j| j.state == State::Running).collect();
        let waiting = q.jobs.iter().filter(|j| j.state == State::Waiting).count();
        let failed = q.jobs.iter().filter(|j| j.state == State::Failed).count();
        let speed: f64 = running.iter().filter(|j| !j.ctl.paused.load(Ordering::Relaxed)).filter_map(|j| j.speed()).sum();
        let mut text = if running.is_empty() && waiting == 0 {
            "Transfers".to_string()
        } else {
            let mut t = format!("Transfers: {}", running.len() + waiting);
            if speed > 0.0 {
                t.push_str(&format!(" · {}", rate(speed)));
            }
            t
        };
        if failed > 0 {
            text.push_str(&format!(" · {failed} failed"));
        }
        if !q.edits.is_empty() {
            text.push_str(&format!(" · editing {}", q.edits.len()));
        }
        let open = q.open;
        let mut b = egui::Button::new(egui::RichText::new(text).size(12.0)).small().selected(open);
        if failed > 0 {
            b = b.stroke(egui::Stroke::new(1.0, ui.visuals().error_fg_color));
        }
        if ui.add(b).on_hover_text("Transfers to and from servers: progress, speed, history").clicked() {
            self.files.rws.queue.open = !open;
        }
    }

    /// The Transfers panel along the bottom of the File Manager.
    pub(super) fn transfers_panel(&mut self, ui: &mut egui::Ui) {
        if !self.files.rws.queue.open {
            return;
        }
        let pal = style::pal(ui);
        let h = self.files.rws.queue.prefs.as_ref().map_or(190.0, |p| p.panel_height);
        let frame = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::symmetric(10, 6));
        let r = egui::Panel::bottom("transfers").frame(frame).resizable(true).default_size(h).min_size(90.0).show(ui, |ui| self.transfers_list(ui));
        let height = r.response.rect.height();
        let q = &mut self.files.rws.queue;
        q.prefs.get_or_insert_with(|| self.files.prefs.remote_desk.transfers.clone()).panel_height = height;
    }

    fn transfers_list(&mut self, ui: &mut egui::Ui) {
        if self.files.rws.queue.prefs.is_none() {
            self.files.rws.queue.prefs = Some(self.files.prefs.remote_desk.transfers.clone());
        }
        let q = &mut self.files.rws.queue;
        ui.horizontal(|ui| {
            ui.strong("Transfers");
            let (r, w) = (q.jobs.iter().filter(|j| j.state == State::Running).count(), q.jobs.iter().filter(|j| j.state == State::Waiting).count());
            ui.label(egui::RichText::new(format!("{r} running · {w} waiting")).size(12.0).weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✖").on_hover_text("Hide the panel; transfers carry on").clicked() {
                    q.open = false;
                }
                if ui.small_button("Clear Finished").on_hover_text("Remove finished, failed and stopped transfers from the list").clicked() {
                    q.jobs.retain(|j| !j.finished());
                }
                let all_paused = q.jobs.iter().filter(|j| !j.finished()).all(|j| j.ctl.paused.load(Ordering::Relaxed));
                let any = q.active() > 0;
                if ui.add_enabled(any, egui::Button::new(if all_paused && any { "Resume All" } else { "Pause All" }).small()).clicked() {
                    for j in q.jobs.iter().filter(|j| !j.finished()) {
                        j.ctl.paused.store(!all_paused, Ordering::Relaxed);
                    }
                }
                let p = q.prefs.as_mut().unwrap();
                ui.add(egui::DragValue::new(&mut p.at_once).range(1..=8)).on_hover_text("How many transfers run at once, each on its own connection");
                ui.label(egui::RichText::new("At once").size(12.0));
            });
        });
        ui.separator();
        let mut retry = None;
        let mut remove = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if !q.edits.is_empty() {
                ui.label(egui::RichText::new("Being edited: saves are sent back to the server").size(12.0).weak());
                let mut stop = None;
                for e in &q.edits {
                    ui.horizontal(|ui| {
                        ui.label("✏");
                        let name = e.local.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        ui.allocate_ui_with_layout(egui::vec2(280.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_min_width(280.0);
                            ui.add(egui::Label::new(egui::RichText::new(&name).size(12.0)).truncate()).on_hover_text(format!("{}\nCopy here: {}", e.remote.shown(), e.local.display()));
                        });
                        let (text, err) = match (&e.error, e.busy, e.uploads) {
                            (_, true, _) => ("Sending…".to_string(), false),
                            (Some(err), _, _) => (err.clone(), true),
                            (None, _, 0) => ("Open; nothing saved yet".into(), false),
                            (None, _, 1) => ("Saved to the server once".into(), false),
                            (None, _, n) => (format!("Saved to the server {n} times"), false),
                        };
                        let rt = egui::RichText::new(text).size(12.0);
                        ui.label(if err { rt.color(ui.visuals().error_fg_color) } else { rt.weak() });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Stop Sending").on_hover_text("Stop sending this file's saves to the server").clicked() {
                                stop = Some(e.remote.clone());
                            }
                        });
                    });
                }
                if let Some(r) = stop {
                    remote::edit::stop(&r);
                    q.edits.retain(|e| e.remote != r);
                }
                ui.separator();
            }
            if q.jobs.is_empty() && q.edits.is_empty() {
                ui.weak("Nothing here yet. Copies and moves to and from servers show up here.");
            }
            // Running and waiting first, then the finished ones, latest first.
            let mut order: Vec<usize> = (0..q.jobs.len()).filter(|&i| !q.jobs[i].finished()).collect();
            order.extend((0..q.jobs.len()).rev().filter(|&i| q.jobs[i].finished()));
            for i in order {
                let j = &q.jobs[i];
                let accent = ui.visuals().selection.bg_fill;
                ui.horizontal(|ui| {
                    let paused = j.ctl.paused.load(Ordering::Relaxed);
                    let stopping = j.ctl.stop.load(Ordering::Relaxed);
                    let (icon, color) = match j.state {
                        State::Waiting => ("…", ui.visuals().weak_text_color()),
                        State::Running if paused => ("⏸", ui.visuals().warn_fg_color),
                        State::Running => ("▶", accent),
                        State::Done => ("✔", ui.visuals().text_color()),
                        State::Failed => ("✖", ui.visuals().error_fg_color),
                        State::Stopped => ("■", ui.visuals().weak_text_color()),
                    };
                    ui.colored_label(color, icon);
                    let label = j.label();
                    ui.allocate_ui_with_layout(egui::vec2(280.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(280.0);
                        ui.add(egui::Label::new(egui::RichText::new(&label).size(12.0)).truncate()).on_hover_text(&label);
                    });
                    let (bt, bd) = (j.ctl.bytes_total.load(Ordering::Relaxed), j.ctl.bytes_done.load(Ordering::Relaxed));
                    match j.state {
                        State::Waiting => {
                            ui.label(egui::RichText::new(if paused { "Held" } else { "Waiting" }).size(12.0).weak());
                        }
                        State::Running => {
                            let frac = if bt > 0 { bd as f32 / bt as f32 } else { 0.0 };
                            let mut text = if j.ctl.counting.load(Ordering::Relaxed) {
                                "Counting…".to_string()
                            } else {
                                format!("{} of {}", crate::scan::human(bd.min(bt)), crate::scan::human(bt))
                            };
                            if let Some(s) = j.speed().filter(|_| !paused) {
                                text.push_str(&format!(" · {}", rate(s)));
                            }
                            if let Some(e) = j.eta().filter(|_| !paused) {
                                text.push_str(&format!(" · {} left", short_time(e)));
                            }
                            let text = if stopping { "Stopping…".into() } else if paused { format!("Paused · {text}") } else { text };
                            let current = j.ctl.current.lock().map(|c| c.clone()).unwrap_or_default();
                            ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width(300.0).text(egui::RichText::new(text).size(11.0)))
                                .on_hover_text(if current.is_empty() { String::new() } else { format!("Now: {current}") });
                        }
                        _ => {
                            let text = if j.errors.is_empty() { j.note.clone() } else { j.errors.join(" ") };
                            let rt = egui::RichText::new(&text).size(12.0);
                            let rt = if j.state == State::Failed { rt.color(ui.visuals().error_fg_color) } else { rt.weak() };
                            ui.add(egui::Label::new(rt).truncate()).on_hover_text(&text);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if j.finished() {
                            if ui.small_button("Remove").on_hover_text("Remove from the list").clicked() {
                                remove = Some(i);
                            }
                            if matches!(j.state, State::Failed | State::Stopped)
                                && ui
                                    .small_button("Retry")
                                    .on_hover_text("Run it again: files that arrived are skipped, and ones cut off part-way carry on from where they got to")
                                    .clicked()
                            {
                                retry = Some(i);
                            }
                        } else {
                            if ui.add_enabled(!stopping, egui::Button::new(if j.state == State::Waiting { "Cancel" } else { "Stop" }).small())
                                .on_hover_text("Stop now. Files that arrived complete stay (Undo removes them); the one cut off is removed")
                                .clicked()
                            {
                                j.ctl.stop.store(true, Ordering::Relaxed);
                                j.ctl.paused.store(false, Ordering::Relaxed);
                            }
                            let tip = if j.state == State::Waiting { "Hold it back, or let it start" } else { "Pause or carry on" };
                            if ui.add_enabled(!stopping, egui::Button::new(if paused { "Resume" } else { "Pause" }).small()).on_hover_text(tip).clicked() {
                                j.ctl.paused.store(!paused, Ordering::Relaxed);
                            }
                        }
                    });
                });
            }
        });
        // A waiting job that was cancelled never ran.
        for j in q.jobs.iter_mut().filter(|j| j.state == State::Waiting && j.ctl.stop.load(Ordering::Relaxed)) {
            j.state = State::Stopped;
            j.note = "Cancelled".into();
        }
        if let Some(i) = retry {
            let j = q.jobs.remove(i);
            let id = q.add(j.sources, j.dest, j.copy, true);
            if let Some(n) = q.jobs.iter_mut().find(|n| n.id == id) {
                n.sync = j.sync;
            }
        }
        if let Some(i) = remove {
            q.jobs.remove(i);
        }
        q.fill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_and_order() {
        let mut q = Queue { prefs: Some(TransferPrefs { at_once: 2, panel_height: 0.0 }), ..Default::default() };
        // Local paths never touch a server; the threads finish quickly with errors.
        let gone = std::env::temp_dir().join("dirmap-queue-test-missing");
        for _ in 0..3 {
            q.add(vec![gone.join("a")], gone.join("b"), true, false);
        }
        q.jobs[0].ctl.paused.store(true, Ordering::Relaxed);
        q.fill();
        let states: Vec<State> = q.jobs.iter().map(|j| j.state).collect();
        // The held one waits; the other two run on lanes 1 and 2.
        assert_eq!(states, vec![State::Waiting, State::Running, State::Running]);
        let mut lanes: Vec<u8> = q.jobs.iter().filter(|j| j.state == State::Running).map(|j| j.lane).collect();
        lanes.sort();
        assert_eq!(lanes, vec![1, 2]);
        assert_eq!(q.active(), 3);
    }

    #[test]
    fn times_read_well() {
        assert_eq!(short_time(Duration::from_secs(42)), "42 s");
        assert_eq!(short_time(Duration::from_secs(125)), "2 min 05 s");
        assert_eq!(short_time(Duration::from_secs(3720)), "1 h 02 min");
    }
}
