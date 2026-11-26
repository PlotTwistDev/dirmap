//! Ask AI, in the AI Models workspace: chat with a GGUF model found on this computer, run by
//! Filescape itself (see `crate::llm`). Free, and nothing leaves the computer. A model's answers
//! are only shown: Filescape never acts on them.

use super::*;
use crate::fsys::PathShow;
use crate::getmodel::{self, STARTER};
use crate::kit::{self, Tone};
use crate::llm::{self, Event, Message, Request, Role};
use crate::models::Model;

/// What the model is told before every conversation.
const SYSTEM: &str = "You are a helpful assistant inside Filescape, a file manager and disk space analyser, \
running entirely on the user's own computer. Answer clearly and briefly. If you aren't sure, say so.";

#[derive(Default)]
pub(super) struct AskState {
    /// Ask AI is on screen instead of the model library.
    pub open: bool,
    engine: Option<llm::Engine>,
    /// The model picked on the left: (file, name).
    picked: Option<(PathBuf, String)>,
    /// The model the engine has loaded, and its context length.
    loaded: Option<(PathBuf, String, u32)>,
    phase: Phase,
    /// Checking a model fits in memory before loading it.
    check: Option<mpsc::Receiver<Result<llm::LoadRequest, String>>>,
    conversation: Vec<Message>,
    /// The answer being written.
    answering: Option<String>,
    draft: String,
    context: u32,
    temperature: f32,
    last: Option<llm::Stats>,
    load_seconds: f32,
    /// The context the loaded model has: the one asked for, or less when the model is shorter.
    actual_context: u32,
    /// The last thing that went wrong.
    problem: Option<String>,
    /// "Get a small model" is on screen.
    offer: bool,
    fetch: Option<Fetch>,
    /// How the last download ended, when it didn't finish: stopped by the user, or went wrong.
    fetch_problem: Option<(Tone, String)>,
    /// A model just downloaded, to load once the library has found it.
    fetched: Option<PathBuf>,
}

/// The starter model downloading.
struct Fetch {
    progress: Arc<getmodel::Progress>,
    rx: mpsc::Receiver<Result<PathBuf, String>>,
}

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Loading,
    Answering,
}

impl AskState {
    fn context(&self) -> u32 {
        if self.context == 0 { 4096 } else { self.context }
    }

    fn temperature(&self) -> f32 {
        if self.temperature == 0.0 { 0.6 } else { self.temperature.max(0.0) }
    }
}

/// Models that Filescape can run, with the list's index.
fn runnable(list: &[Model]) -> Vec<(usize, &Model)> {
    let mut v: Vec<(usize, &Model)> = list.iter().enumerate().filter(|(_, m)| m.runnable() && m.extra == 0).collect();
    // Smallest first: they load and answer quickest.
    v.sort_by_key(|(_, m)| m.size);
    v
}

/// Whether to put the model on the graphics chip: on Apple silicon, where llama.cpp uses Metal.
fn use_gpu() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

impl App {
    /// Opens Ask AI with a model picked, from a model's page.
    pub(super) fn ask_about_model(&mut self, m: &Model) {
        if let Some(f) = m.files.first() {
            self.ask.picked = Some((f.path.clone(), m.name.clone()));
        }
        self.ask.open = true;
    }

    pub(super) fn poll_ask(&mut self, ctx: &egui::Context) {
        self.poll_fetch(ctx);
        if let Some(rx) = &self.ask.check {
            match rx.try_recv() {
                Ok(Ok(req)) => {
                    self.ask.check = None;
                    self.ask.engine.get_or_insert_with(llm::Engine::start).send(Request::Load(req));
                }
                Ok(Err(e)) => {
                    self.ask.check = None;
                    self.ask.phase = Phase::Idle;
                    self.ask.problem = Some(e);
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(50)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.ask.check = None;
                    self.ask.phase = Phase::Idle;
                }
            }
        }
        let Some(e) = &self.ask.engine else { return };
        let mut got = false;
        while let Ok(ev) = e.rx.try_recv() {
            got = true;
            match ev {
                Event::Loading => self.ask.phase = Phase::Loading,
                Event::Loaded { context, seconds } => {
                    self.ask.phase = Phase::Idle;
                    self.ask.load_seconds = seconds;
                    self.ask.actual_context = context;
                    self.ask.loaded = self.ask.picked.clone().map(|(p, n)| (p, n, self.ask.context()));
                    self.ask.problem = None;
                }
                Event::Text(t) => self.ask.answering.get_or_insert_with(String::new).push_str(&t),
                Event::Done(s) => {
                    let text = self.ask.answering.take().unwrap_or_default();
                    self.ask.conversation.push(Message { role: Role::Assistant, text });
                    self.ask.last = Some(s);
                    self.ask.phase = Phase::Idle;
                }
                Event::Unloaded => {
                    self.ask.loaded = None;
                    self.ask.phase = Phase::Idle;
                }
                Event::Error(err) => {
                    // A failed load leaves nothing loaded; a failed answer keeps what was written.
                    if self.ask.phase == Phase::Loading {
                        self.ask.loaded = None;
                    }
                    if let Some(t) = self.ask.answering.take().filter(|t| !t.is_empty()) {
                        self.ask.conversation.push(Message { role: Role::Assistant, text: t });
                    }
                    self.ask.phase = Phase::Idle;
                    self.ask.problem = Some(err);
                }
            }
        }
        if got || self.ask.phase != Phase::Idle {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }

    fn poll_fetch(&mut self, ctx: &egui::Context) {
        if let Some(f) = &self.ask.fetch {
            match f.rx.try_recv() {
                Ok(Ok(path)) => {
                    self.ask.fetch = None;
                    self.ask.offer = false;
                    self.ask.fetched = Some(path);
                    // Look again so the library lists it, then load it (below).
                    self.start_models();
                }
                Ok(Err(e)) => {
                    let tone = if f.progress.cancel.load(Ordering::Relaxed) { Tone::Neutral } else { Tone::Warn };
                    self.ask.fetch = None;
                    self.ask.fetch_problem = Some((tone, e));
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => self.ask.fetch = None,
            }
        }
        if let Some(path) = self.ask.fetched.clone()
            && let Some(list) = &self.models.list
        {
            self.ask.fetched = None;
            let found = runnable(list).into_iter().find(|(_, m)| m.files.first().is_some_and(|f| f.path == path)).map(|(_, m)| m.clone());
            match found {
                Some(m) if self.ask.phase == Phase::Idle && self.ask.check.is_none() => self.load_model(&m),
                Some(m) => self.ask.picked = m.files.first().map(|f| (f.path.clone(), m.name.clone())),
                None => self.ask.problem = Some(format!("{} was downloaded to {}, but the model list doesn't show it.", STARTER.name, path.shown())),
            }
        }
    }

    fn start_fetch(&mut self) {
        let Some(dir) = getmodel::folder() else {
            self.ask.fetch_problem = Some((Tone::Warn, "Filescape can't tell where to save it on this computer.".into()));
            return;
        };
        self.ask.fetch_problem = None;
        let progress = Arc::new(getmodel::Progress::default());
        let (tx, rx) = mpsc::channel();
        let p = progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(getmodel::download(&STARTER, &dir, &p));
        });
        self.ask.fetch = Some(Fetch { progress, rx });
    }

    /// Checks the picked model fits in memory, then loads it.
    fn load_model(&mut self, m: &Model) {
        let Some(f) = m.files.first() else { return };
        self.ask.picked = Some((f.path.clone(), m.name.clone()));
        self.ask.problem = None;
        self.ask.phase = Phase::Loading;
        self.ask.conversation.clear();
        self.ask.answering = None;
        self.ask.last = None;
        let (tx, rx) = mpsc::channel();
        let (m, context, gpu) = (m.clone(), self.ask.context(), use_gpu());
        std::thread::spawn(move || {
            let _ = tx.send(check_fit(&m, context, gpu));
        });
        self.ask.check = Some(rx);
    }

    fn send_question(&mut self) {
        let q = self.ask.draft.trim().to_string();
        if q.is_empty() || self.ask.phase != Phase::Idle || self.ask.loaded.is_none() {
            return;
        }
        self.ask.draft.clear();
        self.ask.problem = None;
        self.ask.conversation.push(Message { role: Role::User, text: q });
        let mut conv = vec![Message { role: Role::System, text: SYSTEM.into() }];
        conv.extend(self.ask.conversation.iter().cloned().map(|mut m| {
            // Earlier thinking isn't sent back: models are trained without it.
            if m.role == Role::Assistant {
                m.text = llm::split_thinking(&m.text).1.to_string();
            }
            m
        }));
        self.ask.answering = Some(String::new());
        self.ask.phase = Phase::Answering;
        let sampling = llm::Sampling { temperature: self.ask.temperature(), ..Default::default() };
        if let Some(e) = &self.ask.engine {
            e.send(Request::Ask { conversation: conv, sampling });
        }
    }

    /// Ask AI, below the toolbar of the AI Models workspace.
    pub(super) fn ask_workspace(&mut self, ui: &mut egui::Ui) {
        let pal = style::pal(ui);
        let side = egui::Frame::new().fill(pal.side).inner_margin(egui::Margin { left: 8, right: 6, top: 8, bottom: 6 });
        let r = egui::Panel::left("ask-models").default_size(280.0).min_size(200.0).resizable(true).frame(side).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.ask_side(ui));
        });
        self.help_regions.push((r.response.rect, topics::ASK_AI));
        let frame = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::same(10));
        let r = egui::CentralPanel::default().frame(frame).show(ui, |ui| self.ask_chat(ui));
        self.help_regions.push((r.response.rect, topics::ASK_AI));
    }

    /// Left: the models Filescape can run, and the settings.
    fn ask_side(&mut self, ui: &mut egui::Ui) {
        let Some(list) = self.models.list.take() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Looking for models…");
            });
            return;
        };
        let models = runnable(&list);
        ui.strong("Model");
        ui.add_space(4.0);
        let mut load = None;
        let has_starter = models.iter().any(|(_, m)| m.files.first().is_some_and(|f| f.path.file_name().is_some_and(|n| n == STARTER.file)));
        if models.is_empty() {
            ui.label("No model Filescape can run was found.");
            ui.add_space(4.0);
            ui.weak(
                "Ask AI runs GGUF models, the kind Ollama and LM Studio download. Filescape can download a small \
                 one for you, or install one with Ollama or LM Studio, or put a .gguf file in Downloads, then press Look again.",
            );
            ui.add_space(8.0);
            if kit::button(ui, kit::Kind::Primary, None, "Get a small model…").clicked() {
                self.ask.offer = true;
            }
            ui.add_space(4.0);
        }
        for (_, m) in &models {
            let path = m.files.first().map(|f| f.path.clone());
            let picked = self.ask.picked.as_ref().is_some_and(|(p, _)| Some(p) == path.as_ref());
            let loaded = self.ask.loaded.as_ref().is_some_and(|(p, _, _)| Some(p) == path.as_ref());
            let w = ui.available_width();
            let (rect, r) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
            let vis = ui.visuals().clone();
            if picked {
                ui.painter().rect_filled(rect, 4.0, vis.selection.bg_fill);
            } else if r.hovered() {
                ui.painter().rect_filled(rect, 4.0, vis.widgets.hovered.weak_bg_fill);
            }
            let p = ui.painter().with_clip_rect(rect.shrink(2.0));
            let strong = if picked { vis.selection.stroke.color } else { vis.strong_text_color() };
            let mut x = 6.0;
            if loaded {
                p.circle_filled(rect.left_top() + vec2(10.0, 11.0), 4.0, kit::tokens(ui).success);
                x = 19.0;
            }
            p.text(rect.left_top() + vec2(x, 11.0), Align2::LEFT_CENTER, &m.name, FontId::proportional(13.5), strong);
            let sub = format!("{} · {}", crate::scan::human(m.size), m.details().trim_start_matches("GGUF · "));
            p.text(rect.left_top() + vec2(6.0, 29.0), Align2::LEFT_CENTER, sub, FontId::proportional(12.0), vis.weak_text_color());
            let r = r.on_hover_text(format!(
                "{}\n{}\n{}{}",
                m.name,
                m.details(),
                path.as_ref().map(|p| p.shown()).unwrap_or_default(),
                if loaded { "\nLoaded" } else { "\nDouble-click to load" }
            ));
            if r.clicked() {
                self.ask.picked = path.clone().map(|p| (p, m.name.clone()));
            }
            if r.double_clicked() && self.ask.phase == Phase::Idle {
                load = Some((*m).clone());
            }
        }
        if !models.is_empty() && !has_starter {
            ui.add_space(2.0);
            if kit::button(ui, kit::Kind::Ghost, None, "Get a small model…").on_hover_text(format!("Download {} ({})", STARTER.name, crate::scan::human(STARTER.size))).clicked() {
                self.ask.offer = true;
            }
        }
        ui.add_space(10.0);
        ui.strong("Settings");
        ui.add_space(4.0);
        let mut ctx_len = self.ask.context();
        ui.horizontal(|ui| {
            ui.label("Context");
            egui::ComboBox::from_id_salt("ask-context").selected_text(crate::fit::tokens(ctx_len as u64)).show_ui(ui, |ui| {
                for c in [2048u32, 4096, 8192, 16384, 32768] {
                    ui.selectable_value(&mut ctx_len, c, format!("{} tokens", crate::fit::tokens(c as u64)));
                }
            });
        })
        .response
        .on_hover_text("How much of the conversation the model keeps in mind. Longer needs more memory. Applies when a model is loaded.");
        self.ask.context = ctx_len;
        let mut t = self.ask.temperature();
        ui.horizontal(|ui| {
            ui.label("Creativity");
            ui.add(egui::Slider::new(&mut t, 0.01..=1.2).show_value(false));
        })
        .response
        .on_hover_text("Low: the likeliest words, the same answer each time. High: more varied, and more likely to wander.");
        self.ask.temperature = t;
        ui.add_space(10.0);
        let picked = self.ask.picked.clone().and_then(|(p, _)| models.iter().find(|(_, m)| m.files.first().is_some_and(|f| f.path == p)).map(|(_, m)| (*m).clone()));
        let busy = self.ask.phase != Phase::Idle || self.ask.check.is_some();
        ui.horizontal(|ui| {
            if let Some(m) = &picked {
                let is_loaded = self.ask.loaded.as_ref().is_some_and(|(p, _, c)| m.files.first().is_some_and(|f| &f.path == p) && *c == ctx_len);
                let label = if is_loaded { "Loaded" } else { "Load" };
                if ui.add_enabled(!busy && !is_loaded, egui::Button::new(label)).on_hover_text("Read the model into memory so it can answer").clicked() {
                    load = Some(m.clone());
                }
            }
            if self.ask.loaded.is_some() && ui.add_enabled(!busy, egui::Button::new("Unload")).on_hover_text("Free the memory the model uses").clicked()
                && let Some(e) = &self.ask.engine
            {
                e.send(Request::Unload);
            }
        });
        ui.add_space(10.0);
        ui.weak(if use_gpu() {
            "Models run on this Mac's graphics chip."
        } else {
            "Models run on the processor. Small models (1 to 4B) answer quickest."
        });
        self.models.list = Some(list);
        if let Some(m) = load {
            self.load_model(&m);
        }
    }

    /// Middle: the conversation and the question box.
    fn ask_chat(&mut self, ui: &mut egui::Ui) {
        if self.ask.offer || self.ask.fetch.is_some() {
            self.get_model(ui);
            return;
        }
        let name = self.ask.loaded.as_ref().map(|(_, n, _)| n.clone());
        ui.horizontal(|ui| {
            match (&self.ask.phase, &name) {
                (Phase::Loading, _) => {
                    ui.spinner();
                    ui.label(format!("Loading {}…", self.ask.picked.as_ref().map_or("the model", |(_, n)| n.as_str())));
                }
                (_, Some(n)) => {
                    ui.heading(n);
                    ui.weak(format!("{} tokens of context", crate::fit::tokens(self.ask.actual_context as u64)))
                        .on_hover_text("How much of the conversation the model keeps in mind. It can't be longer than the model was trained for.");
                }
                (_, None) => {
                    ui.heading("Ask AI");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!self.ask.conversation.is_empty() && self.ask.phase == Phase::Idle, egui::Button::new("New chat")).clicked() {
                    self.ask.conversation.clear();
                    self.ask.last = None;
                    self.ask.problem = None;
                }
            });
        });
        ui.weak("Runs on this computer: nothing you ask leaves it. Answers can be wrong, and Filescape never acts on them.");
        ui.add_space(6.0);
        if let Some(p) = &self.ask.problem {
            kit::callout(ui, Tone::Warn, p.clone());
            ui.add_space(6.0);
        }
        // The question box, at the bottom.
        let input_h = 92.0;
        let avail = ui.available_height();
        let mut send = false;
        let mut stop = false;
        egui::ScrollArea::vertical().id_salt("ask-chat").auto_shrink(false).stick_to_bottom(true).max_height((avail - input_h).max(60.0)).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(820.0));
            if self.ask.conversation.is_empty() && self.ask.answering.is_none() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    if name.is_some() {
                        ui.label("Ask anything. Press Enter to send, Shift+Enter for a new line.");
                    } else if self.ask.phase == Phase::Idle && self.models.list.as_ref().is_some_and(|l| runnable(l).is_empty()) {
                        ui.label("To start, get a small model on the left, or install one with Ollama or LM Studio.");
                    } else if self.ask.phase == Phase::Idle {
                        ui.label("Pick a model on the left and press Load.");
                    }
                });
            }
            let model = name.clone().unwrap_or_else(|| "Model".into());
            for (i, m) in self.ask.conversation.iter().enumerate() {
                message(ui, i, m.role, &m.text, &model, false);
                ui.add_space(10.0);
            }
            if let Some(t) = &self.ask.answering {
                message(ui, self.ask.conversation.len(), Role::Assistant, t, &model, true);
            }
            if let (Some(s), None) = (&self.ask.last, &self.ask.answering) {
                let speed = if s.seconds > 0.0 { format!(" · {:.1} tokens a second", s.tokens as f32 / s.seconds) } else { String::new() };
                let mut line = format!("{} tokens{speed}", s.tokens);
                if s.cut_short {
                    line.push_str(" · stopped before the end");
                }
                if s.trimmed {
                    line.push_str(" · the oldest messages no longer fit and were left out");
                }
                let prompt = if s.prompt_seconds > 0.0 { format!(", {:.0} tokens a second", (s.prompt_tokens - s.reused) as f32 / s.prompt_seconds) } else { String::new() };
                ui.weak(line).on_hover_text(format!(
                    "Read {} tokens of conversation{prompt} ({} kept from the previous answer).\nModel loaded in {:.1} s.",
                    s.prompt_tokens, s.reused, self.ask.load_seconds
                ));
            }
        });
        ui.add_space(4.0);
        let ready = name.is_some();
        let answering = self.ask.phase == Phase::Answering;
        ui.horizontal(|ui| {
            let w = ui.available_width() - 90.0;
            let hint = if ready { "Ask a question…" } else { "Load a model to ask a question" };
            let r = ui.add_enabled(
                ready,
                egui::TextEdit::multiline(&mut self.ask.draft).hint_text(hint).desired_rows(3).desired_width(w).return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))),
            );
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) {
                send = true;
            }
            ui.vertical(|ui| {
                if answering {
                    stop = ui.button("■ Stop").on_hover_text("Stop the answer here").clicked();
                } else if ui.add_enabled(ready && !self.ask.draft.trim().is_empty() && self.ask.phase == Phase::Idle, egui::Button::new("Send")).clicked() {
                    send = true;
                }
            });
        });
        if stop && let Some(e) = &self.ask.engine {
            e.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if send {
            self.send_question();
        }
    }
}

impl App {
    /// "Get a small model": everything about the download, for the user to approve, then its
    /// progress.
    fn get_model(&mut self, ui: &mut egui::Ui) {
        let s = &STARTER;
        let folder = getmodel::folder();
        let part = folder.as_ref().and_then(|f| std::fs::metadata(f.join(format!("{}.part", s.file))).ok()).map_or(0, |m| m.len());
        ui.heading("Get a small model");
        ui.add_space(6.0);
        egui::ScrollArea::vertical().id_salt("ask-get").auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(640.0));
            ui.label("Filescape can download one small model, so Ask AI works without installing anything else. This is what it would download:");
            ui.add_space(10.0);
            egui::Grid::new("ask-starter").num_columns(2).spacing([18.0, 7.0]).show(ui, |ui| {
                ui.weak("Model");
                ui.strong(s.name);
                ui.end_row();
                ui.weak("Made by");
                ui.label(s.publisher);
                ui.end_row();
                ui.weak("Download");
                ui.label(format!("{} ({})", crate::scan::human(s.size), s.file));
                ui.end_row();
                ui.weak("Memory to run it");
                ui.label(format!("About {}", crate::scan::human(s.memory)));
                ui.end_row();
                ui.weak("Licence");
                ui.hyperlink_to(s.licence, s.licence_url).on_hover_text(s.licence_url);
                ui.end_row();
                ui.weak("From");
                ui.hyperlink_to(s.page.trim_start_matches("https://"), s.page).on_hover_text("The publisher's own page on Hugging Face");
                ui.end_row();
                ui.weak("Saved in");
                ui.label(folder.as_ref().map_or_else(|| "Unknown".into(), |f| f.shown()));
                ui.end_row();
            });
            ui.add_space(10.0);
            ui.label(s.about);
            ui.add_space(10.0);
            ui.weak(
                "This is the only time Ask AI goes online. The file comes from the publisher's own page, at a fixed \
                 version, and Filescape checks its checksum before using it. Nothing about you or your files is sent.",
            );
            ui.add_space(12.0);
            if let Some((tone, e)) = &self.ask.fetch_problem {
                kit::callout(ui, *tone, e.clone());
                ui.add_space(8.0);
            }
            if let Some(f) = &self.ask.fetch {
                let done = f.progress.done.load(Ordering::Relaxed);
                let stage = *f.progress.stage.lock().unwrap();
                let frac = done as f32 / s.size as f32;
                ui.add(egui::ProgressBar::new(frac).desired_width(ui.available_width().min(480.0)).text(format!(
                    "{} of {}",
                    crate::scan::human(done),
                    crate::scan::human(s.size)
                )));
                ui.add_space(4.0);
                ui.weak(if stage.is_empty() { "Starting…" } else { stage });
                ui.add_space(8.0);
                let stopping = f.progress.cancel.load(Ordering::Relaxed);
                let r = ui.add_enabled_ui(!stopping, |ui| kit::button(ui, kit::Kind::Secondary, None, if stopping { "Stopping…" } else { "Stop" })).inner;
                if r.on_hover_text("What has downloaded is kept, so it carries on from there next time").clicked() {
                    f.progress.cancel.store(true, Ordering::Relaxed);
                }
            } else {
                let label = if part > 0 {
                    format!("Carry on downloading ({} left)", crate::scan::human(s.size.saturating_sub(part)))
                } else {
                    format!("Download {}", crate::scan::human(s.size))
                };
                ui.horizontal(|ui| {
                    if kit::button(ui, kit::Kind::Primary, None, &label).clicked() {
                        self.start_fetch();
                    }
                    if kit::button(ui, kit::Kind::Secondary, None, "Not now").clicked() {
                        self.ask.offer = false;
                        self.ask.fetch_problem = None;
                    }
                });
                ui.add_space(6.0);
                ui.weak("When it has downloaded, Filescape loads it, ready for your first question.");
            }
        });
    }
}

/// One message of the conversation. The model's thinking, if it shows any, is folded away.
fn message(ui: &mut egui::Ui, i: usize, role: Role, text: &str, model: &str, writing: bool) {
    let t = kit::tokens(ui);
    match role {
        Role::User => {
            ui.label(egui::RichText::new("You").strong().color(t.accent));
            ui.add(egui::Label::new(text).wrap().selectable(true));
        }
        _ => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(model).strong());
                if writing {
                    ui.spinner();
                }
            });
            let (thinking, reply) = llm::split_thinking(text);
            if let Some(th) = thinking.filter(|t| !t.is_empty()) {
                let title = if writing && reply.is_empty() { "Thinking…" } else { "Thought about it" };
                egui::CollapsingHeader::new(egui::RichText::new(title).weak()).id_salt(("ask-think", i)).show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(th).weak()).wrap().selectable(true));
                });
            }
            if !reply.is_empty() {
                formatted(ui, i, reply);
                if !writing {
                    ui.horizontal(|ui| {
                        if ui.small_button("Copy").on_hover_text("Copy this answer").clicked() {
                            ui.ctx().copy_text(reply.to_string());
                        }
                    });
                }
            }
        }
    }
}

/// An answer, with its Markdown (lists, bold, code) formatted. Links are shown but not opened.
fn formatted(ui: &mut egui::Ui, i: usize, text: &str) {
    use crate::help::{doc, render};
    let key = egui::Id::new(("ask-md", i, text.len()));
    let blocks = ui.data(|d| d.get_temp::<Arc<Vec<doc::Block>>>(key)).unwrap_or_else(|| {
        let b = Arc::new(doc::parse(text));
        ui.data_mut(|d| d.insert_temp(key, b.clone()));
        b
    });
    static NO_DOCS: std::sync::OnceLock<doc::Docs> = std::sync::OnceLock::new();
    let mut images = std::collections::HashMap::new();
    let mut out = render::Out {
        page: "",
        docs: NO_DOCS.get_or_init(|| doc::Docs::from_sources(&[])),
        images: &mut images,
        nav: None,
        request: None,
        anchors: Vec::new(),
    };
    render::blocks(ui, &mut out, &blocks);
}

/// Checks the model fits in this computer's memory at `context` tokens, and returns what to load.
fn check_fit(m: &Model, context: u32, gpu: bool) -> Result<llm::LoadRequest, String> {
    let f = m.files.first().ok_or("The model has no file.")?;
    let d = crate::inspect::inspect(m);
    if let Some(why) = &d.unread {
        return Err(format!("The model can't be read: {why}"));
    }
    let weights: u64 = d.tensors.iter().map(|t| t.bytes).sum::<u64>().max(m.size);
    let hw = crate::fit::detect();
    let vram = if gpu { hw.usable_vram() } else { 0 };
    if let Some(n) = crate::fit::need(&d, weights, context as u64, 2.0)
        && crate::fit::verdict(&n, &hw, vram) == crate::fit::Verdict::TooBig
    {
        return Err(format!(
            "{} needs about {} of memory at {} tokens of context, more than this computer has free ({}). Choose a shorter context or a smaller model.",
            m.name,
            crate::scan::human(n.total()),
            crate::fit::tokens(context as u64),
            crate::scan::human(hw.usable_ram().max(vram)),
        ));
    }
    Ok(llm::LoadRequest { path: f.path.clone(), context, gpu })
}
