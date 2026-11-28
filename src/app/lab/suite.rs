//! Testing several models' speed in one go: each is loaded in turn, given the same speed test,
//! and its result kept with the others. Automated benchmark suites are Professional
//! (`Feature::ResearchAutomation`), checked when a suite starts.

use super::*;
use crate::models::Model;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// What the model being tested is doing.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Loading,
    Testing,
}

/// A suite under way, or the last one, kept to show its results.
#[derive(Default)]
pub(in crate::app) struct Suite {
    /// The models still to test, the next last.
    left: Vec<Model>,
    /// The model being tested: (file, stage, when its test was asked, in seconds).
    current: Option<(PathBuf, Stage, i64)>,
    /// How the model being tested is named in the results.
    label: String,
    /// What each model managed, in the order tested.
    done: Vec<(String, Result<BenchResult, String>)>,
    total: usize,
    stop: bool,
    /// The test each model gets.
    settings: BenchSettings,
}

/// The suite's picks and the suite itself.
#[derive(Default)]
pub(in crate::app) struct Suites {
    picked: BTreeSet<PathBuf>,
    pub suite: Option<Suite>,
    open: bool,
}

impl Suite {
    pub(in crate::app) fn running(&self) -> bool {
        self.current.is_some() || (!self.left.is_empty() && !self.stop)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl App {
    /// Moves the suite on when the engine is free: loads the next model, tests a loaded one, or
    /// keeps a finished test's result. Called after the engine's events each frame.
    pub(in crate::app) fn lab_suite_tick(&mut self) {
        if self.ask.phase != super::super::ask::Phase::Idle || self.ask.check.is_some() {
            return;
        }
        let Some(s) = self.ask.lab_state.suites.suite.as_mut() else { return };
        match s.current.clone() {
            None => {
                if s.stop {
                    s.left.clear();
                    return;
                }
                let Some(m) = s.left.pop() else { return };
                let Some(f) = m.files.first() else { return };
                s.current = Some((f.path.clone(), Stage::Loading, now()));
                // Two copies of one model at different precisions share a name.
                s.label = match &m.info.quant {
                    Some(q) => format!("{} · {q}", m.name),
                    None => m.name.clone(),
                };
                self.load_model(&m);
            }
            Some((path, Stage::Loading, _)) => {
                let loaded = self.ask.loaded.as_ref().is_some_and(|(p, _, _)| *p == path);
                if !loaded || s.stop {
                    let why = if s.stop { "Stopped".to_string() } else { self.ask.problem.clone().unwrap_or_else(|| "It couldn't be loaded.".into()) };
                    s.done.push((s.label.clone(), Err(why)));
                    s.current = None;
                    return;
                }
                let Some(e) = &self.ask.engine else { return };
                s.current = Some((path, Stage::Testing, now()));
                let b = s.settings;
                let l = &mut self.ask.lab_state;
                l.bench_asked = b;
                l.bench_progress = Some((1, b.runs));
                self.ask.problem = None;
                self.ask.phase = super::super::ask::Phase::Answering;
                e.send(Request::Bench(b));
            }
            Some((path, Stage::Testing, asked)) => {
                let name = s.label.clone();
                let found = self.ask.lab_state.results.as_ref().and_then(|all| all.iter().find(|r| r.path == path && r.when >= asked)).cloned();
                let r = match found {
                    Some(r) => Ok(r),
                    None if s.stop => Err("Stopped".into()),
                    None => Err(self.ask.problem.clone().unwrap_or_else(|| "The test didn't finish.".into())),
                };
                let s = self.ask.lab_state.suites.suite.as_mut().unwrap();
                s.done.push((name, r));
                s.current = None;
            }
        }
    }

    /// "Test several models": pick models, run the same test on each, compare.
    pub(super) fn lab_suite(&mut self, ui: &mut egui::Ui) {
        // Also moved on from here: a model that fails its memory check never starts the engine,
        // whose events move it on otherwise.
        if self.ask.lab_state.suites.suite.as_ref().is_some_and(Suite::running) {
            self.lab_suite_tick();
        }
        let list = self.models.list.clone().unwrap_or_default();
        let models: Vec<Model> = super::super::ask::runnable(&list).into_iter().map(|(_, m)| m.clone()).collect();
        let running = self.ask.lab_state.suites.suite.as_ref().is_some_and(Suite::running);
        let busy = self.ask.phase != super::super::ask::Phase::Idle;
        let su = &mut self.ask.lab_state.suites;
        let r = egui::CollapsingHeader::new(egui::RichText::new("Test several models").strong()).id_salt("lab-suite").open(running.then_some(true)).default_open(su.open).show(ui, |ui| {
            ui.weak("Each model you tick is loaded in turn and given the same test (the Read, Write and Runs above), so their speeds on this computer can be compared. Professional.");
            ui.add_space(4.0);
            if models.is_empty() {
                ui.label("No models Filescape can run were found.");
                return None;
            }
            ui.add_enabled_ui(!running, |ui| {
                egui::Grid::new("suite-models").num_columns(3).spacing([10.0, 2.0]).show(ui, |ui| {
                    for m in &models {
                        let Some(f) = m.files.first() else { continue };
                        let mut on = su.picked.contains(&f.path);
                        if ui.checkbox(&mut on, &m.name).changed() {
                            if on {
                                su.picked.insert(f.path.clone());
                            } else {
                                su.picked.remove(&f.path);
                            }
                        }
                        ui.weak(m.info.quant.clone().unwrap_or_default());
                        ui.weak(scan::human(m.size));
                        ui.end_row();
                    }
                });
            });
            let picked: Vec<Model> = models.iter().filter(|m| m.files.first().is_some_and(|f| su.picked.contains(&f.path))).cloned().collect();
            ui.add_space(4.0);
            let mut action = None;
            ui.horizontal(|ui| {
                if running {
                    if ui.button("■ Stop").clicked() {
                        action = Some(false);
                    }
                    ui.spinner();
                    if let Some(s) = &su.suite {
                        ui.weak(format!("Model {} of {}…", (s.done.len() + 1).min(s.total), s.total));
                    }
                } else {
                    let label = format!("Test {} model{}", picked.len(), if picked.len() == 1 { "" } else { "s" });
                    if ui.add_enabled(picked.len() > 1 && !busy, egui::Button::new(label)).on_hover_text("Tick two or more models").clicked() {
                        action = Some(true);
                    }
                    if ui.small_button("Tick all").clicked() {
                        su.picked = models.iter().filter_map(|m| m.files.first().map(|f| f.path.clone())).collect();
                    }
                }
            });
            if let Some(s) = &su.suite
                && !s.done.is_empty()
            {
                ui.add_space(6.0);
                suite_table(ui, &s.done);
            }
            action.map(|a| (a, picked))
        });
        su.open = r.fully_open();
        match r.body_returned.flatten() {
            Some((true, picked)) => {
                if !self.premium(crate::licence::Feature::ResearchAutomation) {
                    return;
                }
                let mut left = picked;
                let total = left.len();
                left.reverse();
                let settings = self.ask.lab_state.bench;
                self.ask.lab_state.suites.suite = Some(Suite { left, current: None, label: String::new(), done: Vec::new(), total, stop: false, settings });
                self.lab_suite_tick();
            }
            Some((false, _)) => {
                if let Some(s) = self.ask.lab_state.suites.suite.as_mut() {
                    s.stop = true;
                }
                if let Some(e) = &self.ask.engine {
                    e.stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            None => {}
        }
    }
}

/// The suite's results, fastest writer first, with the failures after.
fn suite_table(ui: &mut egui::Ui, done: &[(String, Result<BenchResult, String>)]) {
    let mut ok: Vec<&(String, Result<BenchResult, String>)> = done.iter().filter(|(_, r)| r.is_ok()).collect();
    ok.sort_by(|a, b| {
        let w = |x: &(String, Result<BenchResult, String>)| x.1.as_ref().map_or(0.0, |r| r.write_speed);
        w(b).total_cmp(&w(a))
    });
    let fastest = ok.first().and_then(|(_, r)| r.as_ref().ok()).map_or(1.0, |r| r.write_speed.max(1e-3));
    egui::Grid::new("suite-results").num_columns(5).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
        ui.strong("Model");
        ui.strong("Written").on_hover_text("Tokens a second written: how fast answers appear");
        ui.strong("Read").on_hover_text("Tokens a second read: how fast a question or document is taken in");
        ui.strong("To the first word");
        ui.strong("Where");
        ui.end_row();
        for (name, r) in ok {
            let Ok(r) = r else { continue };
            ui.label(name);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(60.0, 8.0), Sense::hover());
                let t = kit::tokens(ui);
                ui.painter().rect_filled(rect, 2.0, t.track);
                ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(60.0 * (r.write_speed / fastest).clamp(0.0, 1.0), 8.0)), 2.0, t.accent);
                ui.label(format!("{:.1}", r.write_speed));
            });
            ui.label(format!("{:.0}", r.read_speed));
            ui.label(format!("{:.2} s", r.first_token));
            ui.label(super::place(r));
            ui.end_row();
        }
        for (name, r) in done {
            if let Err(e) = r {
                ui.label(name);
                ui.weak(e);
                ui.label("");
                ui.label("");
                ui.label("");
                ui.end_row();
            }
        }
    });
    ui.weak("Each result is also kept in All results below, with every setting it ran with.");
}
