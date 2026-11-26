//! The Lab, in the AI Models workspace: see what a model would write next and how likely each
//! choice was, step through it a token at a time, pick a different token, and watch what each
//! sampling setting does. It shares Ask AI's loaded model and runs on this computer. Nothing here
//! changes the model or anything on disk.

use super::*;
use crate::kit::{self, Tone};
use crate::lab::{Run, Settings, Step};
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
        }
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
        ui.horizontal(|ui| {
            ui.heading(name.as_deref().map_or("Lab".to_string(), |n| format!("Lab · {n}")));
            ui.weak("What the model would write next, and how sure it is.");
        });
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
                if ui.add_enabled(!busy, egui::Button::new("Next token")).on_hover_text("What the model would write next, and the alternatives").clicked() {
                    run = Some(1);
                }
                if ui.add_enabled(!busy, egui::Button::new(format!("Write {}", l.steps))).on_hover_text("Write several tokens, each coloured by how likely the model thought it was").clicked() {
                    run = Some(l.steps);
                }
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
            ui.weak("Press Next token to see what the model would write.");
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
