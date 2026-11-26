//! The Lab's Inside page: read a text and see what happens to each token on its way through the
//! model. A map shows how much each layer changed each token, and for a chosen token, what each
//! layer would predict if the model stopped there. Reading only: nothing about the model changes.

use super::*;

/// How strongly a layer's change shows: a share of the vector's length, on a log scale from a
/// barely-there 2% to a complete rewrite (200%).
fn heat(change: f32) -> f32 {
    if !change.is_finite() {
        return 0.0;
    }
    ((change.max(1e-4).ln() - 0.02f32.ln()) / (2.0f32.ln() - 0.02f32.ln())).clamp(0.0, 1.0)
}

fn heat_colour(base: Color32, t: f32) -> Color32 {
    let hot = Color32::from_rgb(245, 150, 40);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    Color32::from_rgb(mix(base.r(), hot.r()), mix(base.g(), hot.g()), mix(base.b(), hot.b()))
}

/// The first layer from which the top guess stays the same as the last layer's, if any before it.
pub(super) fn settles(layers: &[Vec<(String, f32)>]) -> Option<usize> {
    let last = &layers.last()?.first()?.0;
    let mut at = layers.len() - 1;
    while at > 0 && layers[at - 1].first().map(|g| &g.0) == Some(last) {
        at -= 1;
    }
    Some(at)
}

impl App {
    fn lab_look(&mut self) {
        let Some(e) = &self.ask.engine else { return };
        if self.ask.loaded.is_none() || self.ask.phase != super::super::ask::Phase::Idle {
            return;
        }
        let l = &mut self.ask.lab_state;
        l.inside = None;
        l.lensed = None;
        self.ask.problem = None;
        self.ask.phase = super::super::ask::Phase::Answering;
        e.send(Request::Look { text: l.text.clone(), chat: l.chat });
    }

    fn lab_lens(&mut self, token: usize) {
        let Some(e) = &self.ask.engine else { return };
        if self.ask.phase != super::super::ask::Phase::Idle {
            return;
        }
        self.ask.problem = None;
        self.ask.phase = super::super::ask::Phase::Answering;
        e.send(Request::Lens(token));
    }

    pub(super) fn lab_inside(&mut self, ui: &mut egui::Ui, name: Option<String>) {
        ui.weak("What happens to each token on its way through the model's layers, and what each layer would predict.");
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
        let mut look = false;
        {
            let l = &mut self.ask.lab_state;
            ui.add(egui::TextEdit::multiline(&mut l.text).desired_rows(2).desired_width(f32::INFINITY).hint_text("Text for the model to read…"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut l.chat, "As a chat message").on_hover_text("On: the text is wrapped in the model's chat template, as Ask AI sends a question.");
                ui.separator();
                look = ui.add_enabled(!busy, egui::Button::new("Look inside")).on_hover_text(format!("Read the text and catch every layer's output (at most {} tokens)", crate::probe::MAX_TOKENS)).clicked();
                if busy {
                    ui.spinner();
                }
            });
        }
        ui.add_space(6.0);
        ui.separator();
        let Some(inside) = self.ask.lab_state.inside.clone() else {
            ui.weak("Press Look inside to see the model read the text.");
            if look {
                self.lab_look();
            }
            return;
        };
        let picked = self.ask.lab_state.lensed.as_ref().map(|(t, _)| *t);
        let mut pick: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.strong("How much each layer changed each token");
            ui.weak(format!("· {} tokens, {} layers, read in {:.2} s", inside.tokens.len(), inside.change.len(), inside.seconds));
        });
        ui.weak("Brighter: the layer changed the token's meaning more. Click a token to see what each layer would predict after it.");
        ui.add_space(4.0);
        let map_h = (ui.available_height() * 0.45).clamp(120.0, 420.0);
        egui::ScrollArea::horizontal().id_salt("inside-map").show(ui, |ui| {
            if let Some(t) = change_map(ui, &inside, picked, map_h, busy) {
                pick = Some(t);
            }
        });
        ui.add_space(8.0);
        match &self.ask.lab_state.lensed {
            Some((t, layers)) => lens_table(ui, inside.tokens.get(*t).map_or("", |s| s.as_str()), layers),
            None if busy => {
                ui.weak("Working out what each layer would predict…");
            }
            None => {}
        }
        if look {
            self.lab_look();
        } else if let Some(t) = pick.filter(|t| Some(*t) != picked) {
            self.lab_lens(t);
        }
    }
}

/// The map: a column per token, a row per layer (first at the top). Returns a clicked token.
fn change_map(ui: &mut egui::Ui, inside: &crate::lens::Inside, picked: Option<usize>, height: f32, busy: bool) -> Option<usize> {
    let n = inside.tokens.len();
    let layers = inside.change.len();
    if n == 0 || layers == 0 {
        return None;
    }
    let label_w = 34.0;
    let head_h = 18.0;
    let cell_w = ((ui.available_width() - label_w) / n as f32).clamp(14.0, 56.0);
    let cell_h = ((height - head_h) / layers as f32).clamp(3.0, 14.0);
    let size = egui::vec2(label_w + cell_w * n as f32, head_h + cell_h * layers as f32);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter_at(rect);
    let base = ui.visuals().extreme_bg_color;
    let text = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let font = egui::FontId::monospace(10.0);
    let x0 = rect.left() + label_w;
    let y0 = rect.top() + head_h;
    for (t, tok) in inside.tokens.iter().enumerate() {
        let x = x0 + t as f32 * cell_w;
        let shown = visible(tok);
        let chars = ((cell_w - 2.0) / 6.0).floor().max(1.0) as usize;
        let short: String = if shown.chars().count() > chars { shown.chars().take(chars.saturating_sub(1).max(1)).chain(['…']).collect() } else { shown };
        let colour = if Some(t) == picked { text } else { weak };
        p.text(egui::pos2(x + cell_w / 2.0, rect.top() + head_h / 2.0), egui::Align2::CENTER_CENTER, short, font.clone(), colour);
    }
    for l in 0..layers {
        let y = y0 + l as f32 * cell_h;
        // Label every few layers, so they stay readable.
        let every = (12.0 / cell_h).ceil().max(1.0) as usize;
        if l % every == 0 || l + 1 == layers {
            p.text(egui::pos2(rect.left() + label_w - 4.0, y + cell_h / 2.0), egui::Align2::RIGHT_CENTER, format!("{}", l + 1), font.clone(), weak);
        }
        for t in 0..n {
            let c = inside.change[l].get(t).copied().unwrap_or(f32::NAN);
            let r = egui::Rect::from_min_size(egui::pos2(x0 + t as f32 * cell_w, y), egui::vec2(cell_w - 1.0, cell_h - (cell_h > 4.0) as u8 as f32));
            p.rect_filled(r, 0.0, if c.is_finite() { heat_colour(base, heat(c)) } else { base });
        }
    }
    if let Some(t) = picked.filter(|t| *t < n) {
        let r = egui::Rect::from_min_size(egui::pos2(x0 + t as f32 * cell_w, rect.top()), egui::vec2(cell_w - 1.0, size.y));
        p.rect_stroke(r, 2.0, egui::Stroke::new(1.5, ui.visuals().selection.stroke.color), egui::StrokeKind::Inside);
    }
    let at = |pos: egui::Pos2| {
        let t = ((pos.x - x0) / cell_w).floor();
        let l = ((pos.y - y0) / cell_h).floor();
        (t >= 0.0 && (t as usize) < n).then_some((t as usize, (l >= 0.0 && (l as usize) < layers).then_some(l as usize)))
    };
    let hover = resp.hover_pos().and_then(at);
    let resp = resp.on_hover_ui(|ui| {
        if let Some((t, l)) = hover {
            ui.label(format!("Token {}: {}", t + 1, visible(&inside.tokens[t])));
            if let Some(l) = l {
                let c = inside.change[l][t];
                if c.is_finite() {
                    ui.weak(format!("Layer {} changed it by {:.0}% of its length", l + 1, c * 100.0));
                } else {
                    ui.weak(format!("Layer {}: the first layer, which makes the token's vector", l + 1));
                }
            }
            ui.weak("Click to see what each layer would predict after it");
        }
    });
    if busy {
        return None;
    }
    if resp.clicked() { resp.interact_pointer_pos().and_then(at).map(|(t, _)| t) } else { None }
}

/// What each layer would predict after the token, with where the final answer settles.
fn lens_table(ui: &mut egui::Ui, token: &str, layers: &[Vec<(String, f32)>]) {
    ui.horizontal(|ui| {
        ui.strong(format!("What each layer would predict after “{}”", visible(token)));
    });
    if let Some(at) = settles(layers) {
        let last = layers.last().and_then(|l| l.first()).map_or(String::new(), |g| visible(&g.0));
        ui.weak(if at + 1 == layers.len() {
            format!("Only the last layer predicts “{last}”.")
        } else {
            format!("From layer {} on, the likeliest next token is “{last}”.", at + 1)
        });
    }
    ui.add_space(2.0);
    let settle = settles(layers);
    egui::ScrollArea::vertical().id_salt("inside-lens").auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("inside-lens-grid").striped(true).num_columns(1 + layers.first().map_or(0, |l| l.len())).spacing([12.0, 3.0]).show(ui, |ui| {
            ui.weak("Layer");
            ui.weak("Likeliest");
            for _ in 1..layers.first().map_or(1, |l| l.len()) {
                ui.weak("");
            }
            ui.end_row();
            for (i, guesses) in layers.iter().enumerate() {
                let label = egui::RichText::new(format!("{}", i + 1)).monospace();
                ui.label(if Some(i) == settle { label.strong() } else { label });
                for (j, (text, p)) in guesses.iter().enumerate() {
                    let c = confidence(*p);
                    let rt = egui::RichText::new(format!("{} {}", visible(text), percent(*p)));
                    if j == 0 {
                        ui.label(rt.color(Color32::BLACK).background_color(c));
                    } else {
                        ui.label(rt.weak());
                    }
                }
                ui.end_row();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_scale() {
        assert_eq!(heat(f32::NAN), 0.0);
        assert_eq!(heat(0.0), 0.0);
        assert_eq!(heat(5.0), 1.0);
        assert!(heat(0.1) > heat(0.05) && heat(0.1) < heat(1.0));
    }

    #[test]
    fn where_the_answer_settles() {
        let g = |s: &str| vec![(s.to_string(), 0.5)];
        assert_eq!(settles(&[g("a"), g("b"), g("c")]), Some(2));
        assert_eq!(settles(&[g("a"), g("c"), g("c")]), Some(1));
        assert_eq!(settles(&[g("c"), g("c")]), Some(0));
        assert_eq!(settles(&[]), None);
    }
}
