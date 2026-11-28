//! The Lab's Compare page: write the same text twice, side by side, with two sets of sampling
//! settings. Keeping one side's result while another model is loaded compares two models. Only
//! one model is loaded at a time, so each side is written in turn and kept; nothing is changed.

use super::*;

/// One side's result: what wrote it and what it wrote.
#[derive(Clone, Debug)]
pub(in crate::app) struct Shot {
    pub model: String,
    pub settings: Settings,
    pub text: String,
    pub chat: bool,
    pub run: Run,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct Side {
    pub settings: Settings,
    pub shot: Option<Shot>,
    /// Keep this side's result when writing again, such as after loading another model.
    pub keep: bool,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct Compare {
    pub sides: [Side; 2],
    /// Sides still to write, last first.
    pub(in crate::app) left: Vec<usize>,
    /// The side being written.
    pub(in crate::app) waiting: Option<usize>,
}

impl Default for Compare {
    fn default() -> Self {
        let side = |settings| Side { settings, shot: None, keep: false };
        Compare { sides: [side(LIKELIEST), side(Settings::default())], left: Vec::new(), waiting: None }
    }
}

const LIKELIEST: Settings = Settings { temperature: 0.0, top_k: 0, top_p: 1.0, min_p: 0.0, seed: 1 };
const ADVENTUROUS: Settings = Settings { temperature: 1.2, top_k: 0, top_p: 1.0, min_p: 0.0, seed: 1 };

/// Settings in a few words, for a side's heading and the hover.
pub(super) fn describe(s: &Settings) -> String {
    if s.temperature <= 0.0 {
        return "always the likeliest".into();
    }
    let mut parts = vec![format!("temperature {:.2}", s.temperature)];
    if s.top_k > 0 {
        parts.push(format!("top-k {}", s.top_k));
    }
    if s.top_p < 1.0 {
        parts.push(format!("top-p {:.2}", s.top_p));
    }
    if s.min_p > 0.0 {
        parts.push(format!("min-p {:.2}", s.min_p));
    }
    parts.join(", ")
}

/// Where two runs first pick a different token, counted from 1; None if they never do.
pub(super) fn parting(a: &Run, b: &Run) -> Option<usize> {
    let n = a.steps.len().min(b.steps.len());
    (0..n).find(|&i| a.steps[i].chosen.id != b.steps[i].chosen.id).map(|i| i + 1).or((a.steps.len() != b.steps.len()).then_some(n + 1))
}

fn averages(r: &Run) -> (f32, f32) {
    let n = r.steps.len().max(1) as f32;
    (r.steps.iter().map(|s| s.chosen.raw).sum::<f32>() / n, r.steps.iter().map(|s| s.entropy).sum::<f32>() / n)
}

impl App {
    fn lab_compare_run(&mut self) {
        let c = &mut self.ask.lab_state.compare;
        c.left = (0..2).rev().filter(|&i| !(c.sides[i].keep && c.sides[i].shot.is_some())).collect();
        self.lab_compare_next();
    }

    fn lab_compare_next(&mut self) {
        let Some(e) = &self.ask.engine else { return };
        if self.ask.loaded.is_none() || self.ask.phase != super::super::ask::Phase::Idle {
            return;
        }
        let l = &mut self.ask.lab_state;
        let Some(i) = l.compare.left.pop() else { return };
        l.compare.waiting = Some(i);
        // The same seed for both sides, so only the settings (or the model) differ.
        let settings = Settings { seed: l.settings.seed, ..l.compare.sides[i].settings };
        self.ask.problem = None;
        self.ask.phase = super::super::ask::Phase::Answering;
        e.send(Request::Predict { text: l.text.clone(), chat: l.chat, continuation: String::new(), steps: l.steps, settings });
    }

    /// A run for one side arrived: keep it, and write the other side if it's still to do.
    pub(in crate::app) fn lab_compared(&mut self, side: usize, run: Run) {
        let model = self.ask.loaded.as_ref().map(|(_, n, _)| n.clone()).unwrap_or_default();
        let l = &mut self.ask.lab_state;
        let settings = Settings { seed: l.settings.seed, ..l.compare.sides[side].settings };
        l.compare.sides[side].shot = Some(Shot { model, settings, text: l.text.clone(), chat: l.chat, run });
        self.lab_compare_next();
    }

    pub(super) fn lab_compare(&mut self, ui: &mut egui::Ui, name: Option<String>) {
        ui.weak("The same text written twice, side by side. Change the settings on either side, or keep one side and load another model to compare two models.");
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
        let busy = self.ask.phase != super::super::ask::Phase::Idle;
        let mut go = false;
        let mut stop = false;
        {
            let l = &mut self.ask.lab_state;
            ui.add(egui::TextEdit::multiline(&mut l.text).desired_rows(2).desired_width(f32::INFINITY).hint_text("Text for the model to carry on…"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut l.chat, "As a chat message").on_hover_text("On: the text is wrapped in the model's chat template, as Ask AI sends a question.");
                ui.separator();
                let todo = (0..2).filter(|&i| !(l.compare.sides[i].keep && l.compare.sides[i].shot.is_some())).count();
                if busy {
                    stop = ui.button("■ Stop").clicked();
                    ui.spinner();
                } else {
                    let label = match todo {
                        2 => format!("Write {} tokens both ways", l.steps),
                        _ => format!("Write {} tokens", l.steps),
                    };
                    go = ui.add_enabled_ui(todo > 0, |ui| kit::button(ui, kit::Kind::Primary, None, &label))
                        .inner
                        .on_hover_text("Write the text with each side's settings and the same seed")
                        .on_disabled_hover_text("Both sides are kept. Untick Keep on one to write it again.")
                        .clicked();
                }
                egui::ComboBox::from_id_salt("compare-steps").width(60.0).selected_text(format!("{}", l.steps)).show_ui(ui, |ui| {
                    for n in [10u32, 30, 100] {
                        ui.selectable_value(&mut l.steps, n, format!("{n} tokens"));
                    }
                });
                ui.weak(format!("Seed {} for both (set on Next token)", l.settings.seed));
            });
        }
        ui.add_space(6.0);
        let text = self.ask.lab_state.text.clone();
        let chat = self.ask.lab_state.chat;
        let shots: Vec<Option<Shot>> = self.ask.lab_state.compare.sides.iter().map(|s| s.shot.clone()).collect();
        ui.columns(2, |c| {
            for (i, ui) in c.iter_mut().enumerate() {
                let side = &mut self.ask.lab_state.compare.sides[i];
                kit::card_frame(&kit::tokens(ui)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.strong(["A", "B"][i]);
                        ui.weak(describe(&side.settings));
                    });
                    side_settings(ui, i, &mut side.settings, busy);
                    ui.add_enabled_ui(side.shot.is_some(), |ui| {
                        ui.checkbox(&mut side.keep, "Keep this result")
                            .on_hover_text("Writing again leaves this side as it is. Load another model and write, to compare two models.");
                    });
                    ui.separator();
                    match &side.shot {
                        Some(s) => {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(&s.model).strong());
                                ui.weak(format!("· {}", describe(&s.settings)));
                            });
                            if s.text != text || s.chat != chat {
                                ui.label(egui::RichText::new("Written from different text").color(kit::tokens(ui).warn)).on_hover_text(format!("This side carried on: {}", s.text));
                            }
                            steer::run_text(ui, &s.run);
                            let (conf, bits) = averages(&s.run);
                            let speed = if s.run.seconds > 0.0 { s.run.steps.len() as f32 / s.run.seconds } else { 0.0 };
                            ui.weak(format!("Average confidence {} · uncertainty {:.1} bits · {:.0} tokens a second", percent(conf), bits, speed)).on_hover_text(UNCERTAINTY);
                        }
                        None if busy && self.ask.lab_state.compare.waiting == Some(i) => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.weak("Writing…");
                            });
                        }
                        None => {
                            ui.weak("Not written yet.");
                        }
                    }
                });
            }
        });
        if let (Some(a), Some(b)) = (&shots[0], &shots[1]) {
            ui.add_space(4.0);
            let same_model = a.model == b.model;
            let what = if same_model { "settings" } else { "models" };
            ui.weak(match parting(&a.run, &b.run) {
                None => format!("Both sides wrote the same tokens: these {what} made no difference here."),
                Some(1) => "The two sides differ from the first token.".to_string(),
                Some(n) => format!("The two sides agree for {} and part at token {n}.", if n == 2 { "1 token".into() } else { format!("{} tokens", n - 1) }),
            });
            if !same_model {
                ui.weak("Speeds from different models are only comparable if nothing else was running. The Speed page measures them properly.");
            }
        }
        if stop && let Some(e) = &self.ask.engine {
            self.ask.lab_state.compare.left.clear();
            e.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if go {
            self.lab_compare_run();
        }
    }
}

/// One side's settings: starting points, then the four that shape the pick.
fn side_settings(ui: &mut egui::Ui, i: usize, s: &mut Settings, busy: bool) {
    ui.add_enabled_ui(!busy, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (label, preset, tip) in [
                ("Likeliest", LIKELIEST, "Always pick the likeliest token (temperature 0)"),
                ("Ask AI's", Settings::default(), "The settings Ask AI uses: temperature 0.6, top-k 40, top-p 0.95, min-p 0.05"),
                ("Adventurous", ADVENTUROUS, "Temperature 1.2 and no limits: unlikely tokens get more of a chance"),
            ] {
                let on = Settings { seed: s.seed, ..preset } == *s;
                if ui.selectable_label(on, label).on_hover_text(tip).clicked() {
                    *s = Settings { seed: s.seed, ..preset };
                }
            }
        });
        egui::Grid::new(("compare-settings", i)).num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
            ui.label("Temperature");
            ui.add(egui::Slider::new(&mut s.temperature, 0.0..=2.0).fixed_decimals(2));
            ui.end_row();
            ui.label("Top-k");
            ui.add(egui::Slider::new(&mut s.top_k, 0..=100));
            ui.end_row();
            ui.label("Top-p");
            ui.add(egui::Slider::new(&mut s.top_p, 0.05..=1.0).fixed_decimals(2));
            ui.end_row();
            ui.label("Min-p");
            ui.add(egui::Slider::new(&mut s.min_p, 0.0..=0.5).fixed_decimals(2));
            ui.end_row();
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::{Choice, Step};

    fn run(ids: &[i32]) -> Run {
        let steps = ids
            .iter()
            .map(|&id| Step { chosen: Choice { id, text: String::new(), raw: 1.0, adjusted: 1.0 }, top: Vec::new(), kept: 1, end: false, entropy: 0.0 })
            .collect();
        Run { steps, prompt_tokens: 0, seconds: 1.0 }
    }

    #[test]
    fn runs_part_where_they_first_differ() {
        assert_eq!(parting(&run(&[1, 2, 3]), &run(&[1, 2, 3])), None);
        assert_eq!(parting(&run(&[1, 2, 3]), &run(&[1, 5, 3])), Some(2));
        assert_eq!(parting(&run(&[4]), &run(&[1])), Some(1));
        assert_eq!(parting(&run(&[1, 2]), &run(&[1, 2, 3])), Some(3));
    }

    #[test]
    fn settings_read_in_a_few_words() {
        assert_eq!(describe(&LIKELIEST), "always the likeliest");
        assert_eq!(describe(&Settings::default()), "temperature 0.60, top-k 40, top-p 0.95, min-p 0.05");
        assert_eq!(describe(&ADVENTUROUS), "temperature 1.20");
    }
}
