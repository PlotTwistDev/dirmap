//! The Lab's Steer page: push a model towards one word and away from another while it writes, and
//! see its text beside what it writes unpushed. The push is worked out from the two words alone
//! (the difference in what one layer makes of them) and only exists while the text is written:
//! nothing about the model or on disk changes.

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Steering {
    pub towards: String,
    pub away: String,
    /// Counted from 0, as the engine counts layers; shown from 1.
    pub layer: usize,
    pub strength: f32,
}

impl Default for Steering {
    fn default() -> Self {
        Steering { towards: "Love".into(), away: "Hate".into(), layer: 1, strength: 4.0 }
    }
}

/// Word pairs to start from: (towards, away).
const PRESETS: [(&str, &str); 6] = [("Love", "Hate"), ("Happy", "Sad"), ("Formal", "Casual"), ("Calm", "Angry"), ("Poetry", "Science"), ("Yes", "No")];

/// The strengths a sweep tries, in this order.
pub(super) const SWEEP: [f32; 5] = [2.0, 4.0, 6.0, 8.0, 10.0];

/// A strength sweep: the same push at several strengths, one after the other.
#[derive(Clone, Debug, Default)]
pub(super) struct Sweep {
    /// Strengths still to write.
    pub left: Vec<f32>,
    /// Written so far: (strength, text).
    pub done: Vec<(f32, Run)>,
    /// The unpushed text, from the first run.
    pub plain: Option<Run>,
}

/// About a third of the way in: early enough to change what comes after, late enough that the
/// layer knows what the words mean.
pub(super) fn default_layer(layers: usize) -> usize {
    (layers / 3).clamp(1, layers.saturating_sub(1).max(1))
}

/// A run's tokens, coloured by how sure the model was of each.
fn run_text(ui: &mut egui::Ui, r: &Run) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for s in &r.steps {
            let text = if s.end { "⏹".to_string() } else { visible(&s.chosen.text) };
            let rt = egui::RichText::new(text).color(Color32::BLACK).background_color(confidence(s.chosen.raw).gamma_multiply(0.85));
            ui.label(rt).on_hover_text(format!("{} · {}", visible(&s.chosen.text), percent(s.chosen.raw)));
        }
    });
}

fn st_eq(s: &Steering, t: &str, a: &str) -> bool {
    s.towards.trim().eq_ignore_ascii_case(t) && s.away.trim().eq_ignore_ascii_case(a)
}

/// A sweep's texts, one row per strength under the unpushed one.
fn sweep_table(ui: &mut egui::Ui, sw: &Sweep, busy: bool) {
    egui::ScrollArea::vertical().id_salt("steer-sweep").auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("steer-sweep-grid").num_columns(2).spacing([14.0, 8.0]).striped(true).show(ui, |ui| {
            if let Some(p) = &sw.plain {
                ui.strong("Without");
                ui.vertical(|ui| run_text(ui, p));
                ui.end_row();
            }
            for (s, r) in &sw.done {
                ui.strong(format!("{s:+.0}"));
                ui.vertical(|ui| run_text(ui, r));
                ui.end_row();
            }
        });
        if busy && !sw.left.is_empty() || busy && sw.done.len() < SWEEP.len() {
            ui.weak(format!("Writing {} of {}…", sw.done.len() + 1, SWEEP.len()));
        }
    });
    ui.weak("Each row is the same text and seed, pushed harder. Where it first changes, and where it stops making sense, differs by model, layer and words.");
}

impl App {
    /// Starts a sweep: the push at each of [`SWEEP`]'s strengths, in the direction chosen.
    fn lab_sweep(&mut self) {
        let sign = if self.ask.lab_state.steer.strength < 0.0 { -1.0 } else { 1.0 };
        let mut left: Vec<f32> = SWEEP.iter().map(|s| s * sign).collect();
        left.reverse();
        self.ask.lab_state.sweep = Some(Sweep { left, ..Default::default() });
        self.ask.lab_state.steered = None;
        self.lab_sweep_next();
    }

    /// Writes the sweep's next strength, if any is left.
    pub(in crate::app) fn lab_sweep_next(&mut self) {
        let Some(next) = self.ask.lab_state.sweep.as_mut().and_then(|s| s.left.pop()) else { return };
        self.lab_steer_at(next);
    }

    /// A steered run arrived while sweeping: keep it, then write the next strength.
    pub(in crate::app) fn lab_swept(&mut self, plain: Run, steered: Run) -> bool {
        let strength = self.ask.lab_state.sweep_strength;
        let Some(sw) = self.ask.lab_state.sweep.as_mut() else { return false };
        sw.plain.get_or_insert(plain);
        sw.done.push((strength, steered));
        self.ask.phase = super::super::ask::Phase::Idle;
        self.lab_sweep_next();
        true
    }

    fn lab_steer_run(&mut self) {
        self.ask.lab_state.sweep = None;
        let strength = self.ask.lab_state.steer.strength;
        self.lab_steer_at(strength);
    }

    fn lab_steer_at(&mut self, strength: f32) {
        let Some(e) = &self.ask.engine else { return };
        if self.ask.loaded.is_none() || self.ask.phase != super::super::ask::Phase::Idle {
            return;
        }
        let l = &mut self.ask.lab_state;
        l.sweep_strength = strength;
        self.ask.problem = None;
        self.ask.phase = super::super::ask::Phase::Answering;
        e.send(Request::Steer(crate::llm::Steer {
            text: l.text.clone(),
            chat: l.chat,
            towards: l.steer.towards.clone(),
            away: l.steer.away.clone(),
            layer: l.steer.layer,
            strength,
            steps: l.steps,
            settings: l.settings,
        }));
    }

    pub(super) fn lab_steer(&mut self, ui: &mut egui::Ui, name: Option<String>) {
        ui.weak("Push the model towards one idea and away from another while it writes, and compare. Only while it writes: the model doesn't change.");
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
        let mut sweep = false;
        let mut stop = false;
        {
            let l = &mut self.ask.lab_state;
            ui.add(egui::TextEdit::multiline(&mut l.text).desired_rows(2).desired_width(f32::INFINITY).hint_text("Text for the model to carry on…"));
            ui.checkbox(&mut l.chat, "As a chat message").on_hover_text("On: the text is wrapped in the model's chat template, as Ask AI sends a question.");
            ui.add_space(4.0);
            let layers = l.layers.max(2);
            ui.horizontal_wrapped(|ui| {
                ui.weak("Try:");
                for (t, a) in PRESETS {
                    let on = st_eq(&l.steer, t, a);
                    if ui.selectable_label(on, format!("{t} ↔ {a}")).on_hover_text(format!("Towards \u{201C}{t}\u{201D}, away from \u{201C}{a}\u{201D}")).clicked() {
                        l.steer.towards = t.into();
                        l.steer.away = a.into();
                    }
                }
            });
            ui.add_space(2.0);
            let st = &mut l.steer;
            egui::Grid::new("lab-steer").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                ui.label("Towards").on_hover_text("A word or phrase for what you want more of");
                ui.add(egui::TextEdit::singleline(&mut st.towards).desired_width(180.0).hint_text("such as Love"));
                ui.label("Away from").on_hover_text("A word or phrase for what you want less of. Either box may be left empty.");
                ui.add(egui::TextEdit::singleline(&mut st.away).desired_width(180.0).hint_text("such as Hate"));
                ui.end_row();
                ui.label("Strength").on_hover_text("How hard to push. Below 0 pushes the other way. Too strong and the text falls apart.");
                ui.add(egui::Slider::new(&mut st.strength, -10.0..=10.0).step_by(0.5).fixed_decimals(1));
                ui.label("At layer").on_hover_text("Which layer's output is pushed. Early layers change the words; middle ones the ideas; late ones little. About a third of the way in usually works best.");
                let mut shown = st.layer + 1;
                if ui.add(egui::Slider::new(&mut shown, 2..=layers).suffix(format!(" of {layers}"))).changed() {
                    st.layer = shown - 1;
                }
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if busy {
                    stop = ui.button("■ Stop").clicked();
                    ui.spinner();
                } else {
                    go = kit::button(ui, kit::Kind::Primary, None, &format!("Write {} both ways", l.steps)).on_hover_text("Write the same text without and with the push, with the same settings and seed").clicked();
                    sweep = kit::button(ui, kit::Kind::Secondary, None, "Sweep strengths")
                        .on_hover_text("Write the same text pushed at strengths 2, 4, 6, 8 and 10 (or minus those, if Strength is below 0), to see where the push starts to change it and where the text falls apart")
                        .clicked();
                }
                egui::ComboBox::from_id_salt("steer-steps").width(60.0).selected_text(format!("{}", l.steps)).show_ui(ui, |ui| {
                    for n in [10u32, 30, 100] {
                        ui.selectable_value(&mut l.steps, n, format!("{n} tokens"));
                    }
                });
                ui.weak("Sampling settings are the ones on Next token.");
            });
        }
        ui.add_space(6.0);
        ui.separator();
        if let Some(sw) = self.ask.lab_state.sweep.clone() {
            sweep_table(ui, &sw, busy);
        } else {
            match self.ask.lab_state.steered.clone() {
            Some((plain, steered, push)) => {
                let st = &self.ask.lab_state.steer;
                let what = match (st.towards.trim(), st.away.trim()) {
                    ("", a) => format!("away from \u{201C}{a}\u{201D}"),
                    (t, "") => format!("towards \u{201C}{t}\u{201D}"),
                    (t, a) => format!("towards \u{201C}{t}\u{201D}, away from \u{201C}{a}\u{201D}"),
                };
                let half = (ui.available_width() - 12.0) / 2.0;
                ui.columns(2, |c| {
                    c[0].strong("Without steering");
                    c[0].add_space(2.0);
                    c[0].set_max_width(half);
                    run_text(&mut c[0], &plain);
                    c[1].strong(format!("Steered {what}"));
                    c[1].add_space(2.0);
                    c[1].set_max_width(half);
                    run_text(&mut c[1], &steered);
                });
                ui.add_space(6.0);
                ui.weak(format!("The push was {:.0}% of the length of a typical output of that layer. Bigger pushes change more, until the text stops making sense.", push * 100.0));
                if plain.steps.iter().map(|s| s.chosen.id).eq(steered.steps.iter().map(|s| s.chosen.id)) {
                    ui.weak("The push didn't change the text. Try more strength or another layer.");
                }
            }
            None if !busy => {
                ui.weak("Choose what to steer towards and press Write both ways.");
            }
            None => {}
        }
        }
        if stop && let Some(e) = &self.ask.engine {
            if let Some(sw) = self.ask.lab_state.sweep.as_mut() {
                sw.left.clear();
            }
            e.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if go {
            self.lab_steer_run();
        }
        if sweep {
            self.lab_sweep();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layer_is_a_third_in_and_never_the_first() {
        assert_eq!(default_layer(36), 12);
        assert_eq!(default_layer(4), 1);
        assert_eq!(default_layer(2), 1);
        assert_eq!(default_layer(0), 1);
    }
}
