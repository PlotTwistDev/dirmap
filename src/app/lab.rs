//! The Lab, in the AI Models workspace: see what a model would write next and how likely each
//! choice was, step through it a token at a time, pick a different token, and watch what each
//! sampling setting does. It shares Ask AI's loaded model and runs on this computer. Nothing here
//! changes the model or anything on disk.

use super::*;

mod inside;
mod steer;
use crate::kit::{self, Tone};
use crate::lab::{BenchResult, BenchSettings, Run, Settings, Step};
use crate::llm::Request;
use crate::vocab::visible_query as visible;

pub(super) struct LabState {
    text: String,
    /// Wrap the text as a chat message with the model's template, as Ask AI does.
    chat: bool,
    pub settings: Settings,
    /// Tokens chosen after the text so far (by "Use this" or "Keep"), as text.
    continuation: String,
    /// How many tokens "Write" asks for.
    steps: u32,
    pub run: Option<Run>,
    /// What the last run was asked: (text, chat, continuation, steps, settings).
    asked: Option<(String, bool, String, u32, Settings)>,
    /// The step whose choices are shown.
    pub shown: usize,
    /// Which of the Lab's pages is on screen.
    page: Page,
    /// The last look inside, and the token whose layers' guesses are shown.
    pub inside: Option<crate::lens::Inside>,
    pub lensed: Option<(usize, Vec<Vec<(String, f32)>>)>,
    /// Steering: the loaded model's layers, what to steer with, and the last result (without,
    /// with, and how long the push was).
    layers: usize,
    steer: steer::Steering,
    pub steered: Option<(Run, Run, f32)>,
    bench: BenchSettings,
    /// The speed test running: (run, of runs).
    pub bench_progress: Option<(u32, u32)>,
    /// What the running test was asked.
    bench_asked: BenchSettings,
    /// Speed test results kept so far, read on first use.
    results: Option<Vec<BenchResult>>,
}

impl Default for LabState {
    fn default() -> Self {
        LabState {
            text: "The capital of France is".into(),
            chat: false,
            settings: Settings::default(),
            continuation: String::new(),
            steps: 30,
            run: None,
            asked: None,
            shown: 0,
            page: Page::Next,
            inside: None,
            lensed: None,
            layers: 0,
            steer: steer::Steering::default(),
            steered: None,
            bench: BenchSettings::default(),
            bench_progress: None,
            bench_asked: BenchSettings::default(),
            results: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Next,
    Inside,
    Steer,
    Speed,
}

impl LabState {
    /// A model was loaded: forget what was found inside the last one.
    pub fn loaded_layers(&mut self, layers: usize) {
        if layers != self.layers {
            self.steer.layer = steer::default_layer(layers);
        }
        self.layers = layers;
        self.inside = None;
        self.lensed = None;
        self.steered = None;
    }
}

/// Green for a confident choice, through amber, to red for an unlikely one.
fn confidence(p: f32) -> Color32 {
    let p = p.clamp(0.0, 1.0);
    if p > 0.5 {
        let t = (p - 0.5) * 2.0;
        Color32::from_rgb((230.0 - 150.0 * t) as u8, (180.0 + 20.0 * t) as u8, (60.0 + 40.0 * t) as u8)
    } else {
        let t = p * 2.0;
        Color32::from_rgb(230, (80.0 + 100.0 * t) as u8, 70)
    }
}

fn percent(p: f32) -> String {
    if p >= 0.995 {
        "100%".into()
    } else if p >= 0.1 {
        format!("{:.0}%", p * 100.0)
    } else if p >= 0.001 {
        format!("{:.1}%", p * 100.0)
    } else if p > 0.0 {
        "<0.1%".into()
    } else {
        "–".into()
    }
}

impl App {
    /// Asks the engine for `steps` tokens after the text and what's been chosen so far.
    fn lab_run(&mut self, steps: u32) {
        let l = &mut self.ask.lab_state;
        let Some(e) = &self.ask.engine else { return };
        if self.ask.loaded.is_none() || self.ask.phase != super::ask::Phase::Idle {
            return;
        }
        l.asked = Some((l.text.clone(), l.chat, l.continuation.clone(), steps, l.settings));
        self.ask.problem = None;
        self.ask.phase = super::ask::Phase::Answering;
        e.send(Request::Predict { text: l.text.clone(), chat: l.chat, continuation: l.continuation.clone(), steps, settings: l.settings });
    }

    /// The Lab, beside the model list it shares with Ask AI.
    pub(super) fn lab_chat(&mut self, ui: &mut egui::Ui) {
        let name = self.ask.loaded.as_ref().map(|(_, n, _)| n.clone());
        let t = kit::tokens(ui);
        ui.horizontal(|ui| {
            crate::kit::icons::show(ui, crate::kit::Icon::Flask, 20.0, t.accent);
            ui.label(egui::RichText::new("Lab").size(kit::TITLE - 2.0).color(t.strong));
            if let Some(n) = &name {
                ui.label(egui::RichText::new(format!("·  {n}")).size(14.0).color(t.weak));
            }
            ui.add_space(8.0);
            let busy = self.ask.phase != super::ask::Phase::Idle;
            ui.add_enabled_ui(!busy, |ui| {
                kit::segmented(ui, &mut self.ask.lab_state.page, &[(Page::Next, "Next token"), (Page::Inside, "Inside"), (Page::Steer, "Steer"), (Page::Speed, "Speed")], false);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                kit::badge(ui, "Free", Tone::Success).on_hover_text("Everything in the Lab is free: it's for learning how models work.");
            });
        });
        ui.add_space(2.0);
        about_page(ui, self.ask.lab_state.page);
        ui.add_space(2.0);
        match self.ask.lab_state.page {
            Page::Speed => return self.lab_speed(ui, name),
            Page::Inside => return self.lab_inside(ui, name),
            Page::Steer => return self.lab_steer(ui, name),
            Page::Next => {}
        }
        ui.weak("What the model would write next, and how sure it is.");
        ui.add_space(4.0);
        if let Some(p) = &self.ask.problem {
            kit::callout(ui, Tone::Warn, p.clone());
            ui.add_space(4.0);
        }
        if name.is_none() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label("Pick a model on the left and press Load."));
            return;
        }
        let busy = self.ask.phase != super::ask::Phase::Idle;
        let mut run: Option<u32> = None;
        // The text, and how it's read.
        {
            let l = &mut self.ask.lab_state;
            let r = ui.add(egui::TextEdit::multiline(&mut l.text).desired_rows(2).desired_width(f32::INFINITY).hint_text("Text for the model to carry on…"));
            if r.changed() {
                l.continuation.clear();
            }
            ui.horizontal(|ui| {
                if ui.checkbox(&mut l.chat, "As a chat message").on_hover_text("On: the text is wrapped in the model's chat template, as Ask AI sends a question, so you see how it starts an answer. Off: the model just carries on the text.").changed() {
                    l.continuation.clear();
                }
                ui.separator();
                // Buttons that look like buttons: the page's own tab is also called Next token.
                ui.add_enabled_ui(!busy, |ui| {
                    if kit::button(ui, kit::Kind::Primary, None, "▶ Show next token").on_hover_text("What the model would write next, and the alternatives").clicked() {
                        run = Some(1);
                    }
                    if kit::button(ui, kit::Kind::Secondary, None, &format!("Write {} tokens", l.steps)).on_hover_text("Write several tokens, each coloured by the probability the model gave it").clicked() {
                        run = Some(l.steps);
                    }
                });
                egui::ComboBox::from_id_salt("lab-steps").width(60.0).selected_text(format!("{}", l.steps)).show_ui(ui, |ui| {
                    for n in [10u32, 30, 100, 300] {
                        ui.selectable_value(&mut l.steps, n, format!("{n} tokens"));
                    }
                });
                if busy {
                    ui.spinner();
                }
                if !l.continuation.is_empty() && ui.add_enabled(!busy, egui::Button::new("Start again")).on_hover_text("Forget the tokens chosen so far").clicked() {
                    l.continuation.clear();
                    l.run = None;
                }
            });
        }
        ui.add_space(6.0);
        // Settings, with what they do to the next token.
        let before = self.ask.lab_state.settings;
        self.lab_settings(ui);
        let changed = self.ask.lab_state.settings != before;
        ui.add_space(6.0);
        ui.separator();
        // The text so far, then the run's tokens coloured by confidence.
        let Some(r) = self.ask.lab_state.run.clone() else {
            ui.weak("Press Show next token to see what the model would write.");
            if let Some(n) = run {
                self.lab_run(n);
            }
            return;
        };
        let mut use_token: Option<(usize, String)> = None;
        let mut keep = false;
        egui::ScrollArea::vertical().id_salt("lab-out").max_height((ui.available_height() * 0.42).max(80.0)).auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let l = &self.ask.lab_state;
                ui.weak(if l.chat { format!("[chat] {}", l.text) } else { l.text.clone() });
                if !l.continuation.is_empty() {
                    ui.label(&l.continuation);
                }
                for (i, s) in r.steps.iter().enumerate() {
                    let text = if s.end { "⏹".to_string() } else { visible(&s.chosen.text) };
                    let c = confidence(s.chosen.raw);
                    let sel = i == self.ask.lab_state.shown;
                    let rt = egui::RichText::new(text).color(Color32::BLACK).background_color(if sel { c } else { c.gamma_multiply(0.75) });
                    let resp = ui.add(egui::Label::new(rt).sense(Sense::click())).on_hover_ui(|ui| {
                        ui.label(format!("{} · {}", visible(&s.chosen.text), percent(s.chosen.raw)));
                        if s.end {
                            ui.weak("The model chose to stop here.");
                        }
                        ui.weak("Click to see the alternatives");
                    });
                    if resp.clicked() {
                        self.ask.lab_state.shown = i;
                    }
                }
            });
        });
        let shown = self.ask.lab_state.shown.min(r.steps.len().saturating_sub(1));
        ui.horizontal(|ui| {
            let avg = if r.steps.is_empty() { 0.0 } else { r.steps.iter().map(|s| s.chosen.raw).sum::<f32>() / r.steps.len() as f32 };
            ui.weak(format!("{} tokens in {:.1} s · read {} tokens · average confidence {}", r.steps.len(), r.seconds, r.prompt_tokens, percent(avg)));
            if r.steps.len() > 1 && ui.add_enabled(!busy, egui::Button::new("Keep these")).on_hover_text("Add the written tokens to the text, to carry on from there").clicked() {
                keep = true;
            }
        });
        ui.add_space(6.0);
        if let Some(s) = r.steps.get(shown) {
            if let Some(t) = choices(ui, s, shown, r.steps.len(), busy) {
                use_token = Some((shown, t));
            }
        }
        if keep {
            let l = &mut self.ask.lab_state;
            l.continuation.extend(r.steps.iter().filter(|s| !s.end).map(|s| s.chosen.text.clone()));
            l.run = None;
            l.shown = 0;
            run = Some(1);
        }
        if let Some((i, t)) = use_token {
            let l = &mut self.ask.lab_state;
            l.continuation.extend(r.steps[..i].iter().map(|s| s.chosen.text.clone()));
            l.continuation.push_str(&t);
            l.shown = 0;
            run = Some(1);
        }
        // Changing a setting shows its effect on the next token straight away.
        if changed && run.is_none() && r.steps.len() == 1 {
            run = Some(1);
        }
        if let Some(n) = run {
            self.lab_run(n);
        }
    }

    /// A speed test's result arrived: keep it with the others.
    pub(super) fn lab_benched(&mut self, read_speed: f32, write_speed: f32, first_token: f32, threads: u32) {
        let l = &mut self.ask.lab_state;
        l.bench_progress = None;
        let Some((path, model, _)) = self.ask.loaded.clone() else { return };
        let b = l.bench_asked;
        let r = BenchResult {
            model,
            path,
            when: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64),
            context: self.ask.actual_context,
            gpu: super::ask::use_gpu(),
            threads,
            read: b.read,
            write: b.write,
            runs: b.runs,
            read_speed,
            write_speed,
            first_token,
        };
        match crate::lab::results_file().map(|f| crate::lab::save_result(&f, r.clone())) {
            Some(Ok(all)) => l.results = Some(all),
            Some(Err(e)) => {
                l.results.get_or_insert_with(Vec::new).insert(0, r);
                self.ask.problem = Some(format!("The result couldn't be saved: {e}"));
            }
            None => l.results.get_or_insert_with(Vec::new).insert(0, r),
        }
    }

    /// The speed test: how fast the loaded model reads and writes on this computer, with the
    /// results kept so far.
    fn lab_speed(&mut self, ui: &mut egui::Ui, name: Option<String>) {
        ui.weak("How fast the model reads and writes on this computer. Results are kept, to compare models and settings.");
        ui.add_space(4.0);
        if let Some(p) = &self.ask.problem {
            kit::callout(ui, Tone::Warn, p.clone());
            ui.add_space(4.0);
        }
        if self.ask.lab_state.results.is_none() {
            self.ask.lab_state.results = Some(crate::lab::results_file().map(|f| crate::lab::load_results(&f)).unwrap_or_default());
        }
        let busy = self.ask.phase != super::ask::Phase::Idle;
        let mut start = false;
        let mut stop = false;
        if name.is_some() {
            let l = &mut self.ask.lab_state;
            ui.horizontal(|ui| {
                let b = &mut l.bench;
                ui.add_enabled_ui(!busy, |ui| {
                    ui.label("Read");
                    egui::ComboBox::from_id_salt("bench-read").width(70.0).selected_text(format!("{}", b.read)).show_ui(ui, |ui| {
                        for n in [128u32, 512, 2048] {
                            ui.selectable_value(&mut b.read, n, format!("{n} tokens"));
                        }
                    }).response.on_hover_text("How much text to read first, as when a long question or document is sent");
                    ui.label("Write");
                    egui::ComboBox::from_id_salt("bench-write").width(70.0).selected_text(format!("{}", b.write)).show_ui(ui, |ui| {
                        for n in [32u32, 128, 256] {
                            ui.selectable_value(&mut b.write, n, format!("{n} tokens"));
                        }
                    });
                    ui.label("Runs");
                    egui::ComboBox::from_id_salt("bench-runs").width(50.0).selected_text(format!("{}", b.runs)).show_ui(ui, |ui| {
                        for n in [1u32, 3, 5] {
                            ui.selectable_value(&mut b.runs, n, format!("{n}"));
                        }
                    }).response.on_hover_text("The test is run this many times and the middle result kept, so one slow run doesn't count");
                });
                if busy {
                    stop = ui.button("■ Stop").clicked();
                    ui.spinner();
                    if let Some((r, of)) = l.bench_progress {
                        ui.weak(format!("Run {r} of {of}…"));
                    }
                } else if kit::button(ui, kit::Kind::Primary, None, "Run speed test").clicked() {
                    start = true;
                }
            });
            ui.weak("The model is busy while it runs: other apps may be slower, and so is the test if they're busy too.");
        } else {
            ui.add_space(8.0);
            ui.label("Load a model on the left to test its speed.");
        }
        ui.add_space(8.0);
        let results = self.ask.lab_state.results.clone().unwrap_or_default();
        let current = self.ask.loaded.as_ref().map(|(p, _, _)| p.clone());
        if let Some(r) = results.iter().find(|r| Some(&r.path) == current.as_ref()) {
            speed_card(ui, r);
            ui.add_space(10.0);
        }
        if results.is_empty() {
            ui.weak("No results yet.");
        } else {
            ui.strong("All results");
            ui.add_space(2.0);
            results_table(ui, &results, current.as_deref());
        }
        if stop && let Some(e) = &self.ask.engine {
            e.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if start && let Some(e) = &self.ask.engine {
            let l = &mut self.ask.lab_state;
            l.bench_asked = l.bench;
            l.bench_progress = Some((1, l.bench.runs));
            self.ask.problem = None;
            self.ask.phase = super::ask::Phase::Answering;
            e.send(Request::Bench(l.bench));
        }
    }

    fn lab_settings(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.ask.lab_state.settings;
        egui::Grid::new("lab-settings").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
            ui.label("Temperature").on_hover_text("How adventurous the choice is. 0: always the likeliest token. 1: as likely as the model says. Above 1: unlikely tokens get more of a chance.");
            ui.add(egui::Slider::new(&mut s.temperature, 0.0..=2.0).fixed_decimals(2));
            ui.label("Top-k").on_hover_text("Only the k likeliest tokens can be picked. 0: no limit.");
            ui.add(egui::Slider::new(&mut s.top_k, 0..=100));
            ui.end_row();
            ui.label("Top-p").on_hover_text("Only the likeliest tokens that together make up this share can be picked. 1: no limit.");
            ui.add(egui::Slider::new(&mut s.top_p, 0.05..=1.0).fixed_decimals(2));
            ui.label("Min-p").on_hover_text("Tokens less likely than this share of the likeliest one can't be picked. 0: no limit.");
            ui.add(egui::Slider::new(&mut s.min_p, 0.0..=0.5).fixed_decimals(2));
            ui.end_row();
            ui.label("Seed").on_hover_text("The same seed and settings pick the same tokens again. It only matters above temperature 0.");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.seed).range(0..=u32::MAX as u64));
                if ui.small_button("🎲").on_hover_text("Another seed").clicked() {
                    s.seed = crate::lab::Rng::new(s.seed ^ 0x5DEE_CE66).next().to_bits() as u64;
                }
            });
            if ui.small_button("Ask AI's settings").on_hover_text("Temperature 0.6, top-k 40, top-p 0.95, min-p 0.05").clicked() {
                *s = Settings { seed: s.seed, ..Settings::default() };
            }
            ui.end_row();
        });
    }
}

/// Roughly how many words a speed in tokens is: about ¾ of a word a token, in English.
/// A folded section under the Lab's tabs: what the page shows, how to read it, and what it
/// doesn't mean. Accurate rather than impressive: these are read by people learning.
fn about_page(ui: &mut egui::Ui, page: Page) {
    let (title, lines): (&str, &[&str]) = match page {
        Page::Next => (
            "About Next token",
            &[
                "A language model writes one token at a time. For each, it gives every token in its vocabulary a probability; then one is picked from them.",
                "The bars show those probabilities. Temperature, Top-K, Top-P and Min-P reshape them before the pick: low temperature favours the likeliest token, higher gives others more of a chance. With the same seed and settings, the same tokens are picked again.",
                "A high probability means the model expects that token there, not that what it writes is true. Models can be very sure of wrong facts.",
            ],
        ),
        Page::Inside => (
            "About Inside",
            &[
                "Each token is represented inside the model by a long list of numbers, which every layer changes a little. The map shows how much each layer changed each token's numbers.",
                "\"What each layer would predict\" passes a layer's output straight to the model's final step (a technique called the logit lens). It's a diagnostic view: early layers were never trained to be read this way, so their guesses are rough.",
                "Neither is a direct view of meaning or of anything like thought. They're measurements of numbers inside the model, from the model you loaded.",
            ],
        ),
        Page::Steer => (
            "About Steer",
            &[
                "Steering adds a direction to one layer's numbers while the model writes, worked out from the difference between how the model represents two words.",
                "It changes nothing in the model file, and only lasts for that answer. The effect is experimental: some directions change the tone clearly, others do little or make the text worse.",
            ],
        ),
        Page::Speed => (
            "About Speed",
            &[
                "Reading (prompt processing) is how fast the model takes in text; writing (generation) is how fast it produces new tokens, one at a time. They differ a lot, so they're measured apart.",
                "Speeds are in tokens a second, not words: a token is about three quarters of an English word. Results depend on the model, its precision, the context length, the threads used and what else the computer is doing.",
            ],
        ),
    };
    egui::CollapsingHeader::new(egui::RichText::new(title).size(12.0)).id_salt(("lab-about", title)).default_open(false).show(ui, |ui| {
        ui.set_max_width(760.0);
        for l in lines {
            ui.label(egui::RichText::new(*l).size(12.5));
            ui.add_space(3.0);
        }
    });
}

fn words(tokens_per_sec: f32) -> f32 {
    tokens_per_sec * 0.75
}

/// The latest result for the loaded model, in words people can picture.
fn speed_card(ui: &mut egui::Ui, r: &BenchResult) {
    let t = kit::tokens(ui);
    egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(6.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (big, what, tip) in [
                (format!("{:.1}", r.write_speed), "tokens a second written", format!("Roughly {:.0} English words a second, as a token is about ¾ of a word. People read about 4.", words(r.write_speed))),
                (format!("{:.0}", r.read_speed), "tokens a second read", "How fast it takes in a question or a document before answering".to_string()),
                (format!("{:.1}", r.first_token), "seconds to the first token", format!("After reading {} tokens", r.read)),
            ] {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(big).size(22.0).color(t.accent).strong());
                    ui.weak(what);
                })
                .response
                .on_hover_text(tip);
                ui.add_space(24.0);
            }
        });
        let pace = if words(r.write_speed) >= 6.0 {
            "Faster than you can read it."
        } else if words(r.write_speed) >= 3.0 {
            "About as fast as you read."
        } else {
            "Slower than you read: a smaller model, or a shorter context, would be quicker."
        };
        ui.weak(format!("{pace} {} · {} tokens of context.", place(r), crate::fit::tokens(r.context as u64)));
    });
}

fn place(r: &BenchResult) -> String {
    if r.gpu { "On the graphics chip".to_string() } else { format!("On the processor, {} threads", r.threads) }
}

fn results_table(ui: &mut egui::Ui, results: &[BenchResult], current: Option<&std::path::Path>) {
    let t = kit::tokens(ui);
    let fastest = results.iter().map(|r| r.write_speed).fold(0.0f32, f32::max).max(1e-3);
    egui::ScrollArea::vertical().id_salt("bench-results").auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("bench-table").num_columns(7).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
            for h in ["Model", "Written", "", "Read", "First word", "Where", "When"] {
                ui.weak(h);
            }
            ui.end_row();
            for r in results {
                let label = egui::RichText::new(&r.model);
                ui.label(if Some(r.path.as_path()) == current { label.strong() } else { label }).on_hover_text(r.path.display().to_string());
                ui.label(format!("{:.1}/s", r.write_speed));
                let (rect, _) = ui.allocate_exact_size(vec2(120.0, 10.0), Sense::hover());
                ui.painter().rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
                ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(rect.width() * r.write_speed / fastest, rect.height())), 3.0, t.accent);
                ui.label(format!("{:.0}/s", r.read_speed));
                ui.label(format!("{:.1}", r.first_token));
                ui.label(place(r)).on_hover_text(format!("{} tokens of context; read {}, wrote {}, middle of {} runs", r.context, r.read, r.write, r.runs));
                let when = chrono::DateTime::from_timestamp(r.when, 0).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                ui.weak(when);
                ui.end_row();
            }
        });
    });
}

/// The likeliest tokens at one step, as bars: the model's own probability, and what's left after
/// the settings. Returns a token the user chose to use instead.
fn choices(ui: &mut egui::Ui, s: &Step, i: usize, n: usize, busy: bool) -> Option<String> {
    let t = kit::tokens(ui);
    let mut out = None;
    ui.horizontal(|ui| {
        ui.strong(if n > 1 { format!("Token {} of {n}: the alternatives", i + 1) } else { "The next token: the alternatives".to_string() });
        ui.weak(format!("· the settings leave {} to pick from", s.kept));
    });
    ui.add_space(2.0);
    let bar_w = (ui.available_width() - 330.0).clamp(80.0, 420.0);
    egui::Grid::new(("lab-choices", i)).num_columns(5).spacing([10.0, 3.0]).show(ui, |ui| {
        ui.weak("Token");
        ui.weak("Model");
        ui.weak("");
        ui.weak("After settings");
        ui.end_row();
        for c in &s.top {
            let chosen = c.id == s.chosen.id;
            let label = egui::RichText::new(format!("\u{201C}{}\u{201D}", visible(&c.text))).monospace();
            ui.label(if chosen { label.strong() } else { label }).on_hover_text(format!("Token {}", c.id));
            ui.label(percent(c.raw));
            let (rect, _) = ui.allocate_exact_size(vec2(bar_w, 14.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
            p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(rect.width() * c.raw, rect.height() * 0.5)), 2.0, ui.visuals().weak_text_color());
            p.rect_filled(egui::Rect::from_min_size(rect.min + vec2(0.0, rect.height() * 0.5), vec2(rect.width() * c.adjusted, rect.height() * 0.5)), 2.0, t.accent);
            ui.label(percent(c.adjusted));
            ui.horizontal(|ui| {
                if chosen {
                    ui.weak("chosen");
                } else if ui.add_enabled(!busy, egui::Button::new("Use this").small()).on_hover_text("Write this token here instead, and see what comes next").clicked() {
                    out = Some(c.text.clone());
                }
            });
            ui.end_row();
        }
    });
    if !s.top.iter().any(|c| c.id == s.chosen.id) {
        ui.weak(format!("Chosen: \u{201C}{}\u{201D}, {} (not among the likeliest ten)", visible(&s.chosen.text), percent(s.chosen.raw)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_read_well() {
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(0.4567), "46%");
        assert_eq!(percent(0.0123), "1.2%");
        assert_eq!(percent(0.00001), "<0.1%");
        assert_eq!(percent(0.0), "–");
    }

    #[test]
    fn confidence_goes_red_to_green() {
        let (lo, hi) = (confidence(0.01), confidence(0.99));
        assert!(lo.r() > lo.g() && hi.g() > hi.r());
    }
}
