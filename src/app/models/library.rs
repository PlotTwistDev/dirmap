//! The Library: the models on this computer as a list to search, sort, filter and group, with a
//! strip of totals above it, the places looked in on the left (folded away when not wanted) and
//! the chosen model's facts and actions on the right.

use super::*;
use crate::kit::Icon;
use crate::modelkind::{self, Kind};
use crate::models::{Footprint, Format, Source};
use std::collections::{BTreeMap, HashSet};

/// How the list is ordered.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum Sort {
    #[default]
    Size,
    Name,
    Kind,
    Source,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Sort::Size => "Size",
            Sort::Name => "Name",
            Sort::Kind => "Type",
            Sort::Source => "App",
        }
    }
}

/// The library's search, filters and layout. Kept when the models are looked for again.
#[derive(Default)]
pub(in crate::app) struct Library {
    search: String,
    kind: Option<Kind>,
    sort: Sort,
    source: Option<Source>,
    format: Option<Format>,
    /// Only models Filescape can run itself.
    runnable: bool,
    /// Only models with something to look at: extra copies, unused downloads.
    attention: bool,
    /// Only models that would fit this computer's memory in Filescape, at a short context.
    fits: bool,
    /// Versions and precisions of one model under one heading.
    pub grouped: bool,
    /// Group headings opened (by key).
    open: HashSet<String>,
    /// The places panel is folded away.
    pub side_hidden: bool,
}

impl Library {
    fn filtering(&self) -> bool {
        self.source.is_some() || self.format.is_some() || self.runnable || self.attention || self.fits
    }

    fn matches(&self, m: &Model, kind: Kind, hw: Option<&crate::fit::Hardware>) -> bool {
        if self.kind.is_some_and(|k| k != kind) || self.source.is_some_and(|s| s != m.source) || self.format.is_some_and(|f| f != m.format) {
            return false;
        }
        if self.runnable && !m.runnable() {
            return false;
        }
        if self.attention && m.extra == 0 && !m.orphan {
            return false;
        }
        if self.fits && !(m.runnable() && hw.is_some_and(|hw| crate::fit::fits_roughly(m.size, hw))) {
            return false;
        }
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let path = m.files.first().map(|f| f.path.shown().to_lowercase()).unwrap_or_default();
        q.split_whitespace().all(|w| {
            m.name.to_lowercase().contains(w) || m.details().to_lowercase().contains(w) || m.source.label().to_lowercase().contains(w) || kind.label().to_lowercase().contains(w) || path.contains(w)
        })
    }
}

/// One line of the list: a model, or the heading of a group of versions.
enum Row {
    Group { key: String, members: Vec<usize> },
    Model { i: usize, child: bool },
}

/// What the user asked to do with a model, from its menu or the details panel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Act {
    Inspect,
    Ask,
    Lab,
    Compare(usize),
    Fit,
    ShowFiles,
    ShowAnalysis,
    Shrink,
    Trash,
}

/// The colour that marks each kind of model: its icon and the tile behind it.
pub(super) fn kind_colour(k: Kind, dark: bool) -> Color32 {
    let (d, l) = match k {
        Kind::Language => ((0x5b, 0x9c, 0xf6), (0x1f, 0x63, 0xd1)),
        Kind::Image => ((0xb0, 0x8c, 0xf6), (0x77, 0x4b, 0xd6)),
        Kind::Video => ((0xec, 0x83, 0xb5), (0xb8, 0x3a, 0x7c)),
        Kind::Audio => ((0xf0, 0xa4, 0x5a), (0xb3, 0x62, 0x0c)),
        Kind::Embedding => ((0x4f, 0xc7, 0xb3), (0x14, 0x82, 0x72)),
        Kind::Vision => ((0x57, 0xc0, 0xe0), (0x13, 0x7c, 0x9c)),
        Kind::Multimodal => ((0x8d, 0x9b, 0xf8), (0x4b, 0x56, 0xc9)),
        Kind::Adapter => ((0x9c, 0xc8, 0x6a), (0x4f, 0x80, 0x1c)),
        Kind::Unknown => ((0x8c, 0x91, 0x9b), (0x6b, 0x70, 0x78)),
    };
    let c = if dark { d } else { l };
    Color32::from_rgb(c.0, c.1, c.2)
}

pub(super) fn kind_icon(k: Kind) -> Icon {
    match k {
        Kind::Language => Icon::Chat,
        Kind::Image => Icon::Image,
        Kind::Video => Icon::Film,
        Kind::Audio => Icon::Wave,
        Kind::Embedding => Icon::Points,
        Kind::Vision => Icon::Eye,
        Kind::Multimodal => Icon::Sparkle,
        Kind::Adapter => Icon::Plug,
        Kind::Unknown => Icon::Help,
    }
}

/// A rounded tile with the kind's icon. A guessed kind gets a dashed edge instead of a fill.
pub(super) fn kind_tile(p: &egui::Painter, rect: Rect, kind: Kind, sure: bool, dark: bool) {
    let c = kind_colour(kind, dark);
    let r = rect.height() * 0.24;
    if sure {
        p.rect_filled(rect, r, c.gamma_multiply(if dark { 0.18 } else { 0.13 }));
    } else {
        p.rect_filled(rect, r, c.gamma_multiply(0.06));
        let st = Stroke::new(1.0, c.gamma_multiply(0.7));
        let pts = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
        for w in pts.windows(2) {
            p.extend(egui::Shape::dashed_line(&[w[0], w[1]], st, 3.0, 2.5));
        }
    }
    let s = rect.height() * 0.56;
    crate::kit::icons::paint(p, Rect::from_center_size(rect.center(), vec2(s, s)), kind_icon(kind), c);
}

/// "Language", or "Language (probably)" when it's a guess.
pub(super) fn kind_words(kind: Kind, sure: bool) -> String {
    if sure || kind == Kind::Unknown { kind.label().to_string() } else { format!("{} (probably)", kind.label()) }
}

fn kind_tip(kind: Kind, sure: bool) -> String {
    let how = match (kind, sure) {
        (Kind::Unknown, _) => "",
        (_, true) => "\n\nFrom the model's own files.",
        (_, false) => "\n\nA guess from the model's name or folder: its files don't say.",
    };
    format!("{}{how}", kind.explain())
}

const ROW: f32 = 2.0 * ROW_H + 10.0;
const GROUP_ROW: f32 = ROW_H + 12.0;

impl App {
    /// Left: where models were looked for, folded to a thin rail when hidden.
    pub(super) fn models_places(&mut self, ui: &mut egui::Ui) {
        let t = kit::tokens(ui);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("LOCATIONS").size(10.5).color(t.weak).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::icon_button(ui, Icon::FoldLeft, "Hide the locations").clicked() {
                    self.models.lib.side_hidden = true;
                }
            });
        });
        ui.add_space(2.0);
        let list = self.models.list.as_deref().unwrap_or(&[]);
        // Models and space found in each place.
        let mut counts: Vec<(usize, u64)> = vec![(0, 0); self.models.places.len()];
        let mut in_analysis = (0usize, 0u64);
        for m in list {
            let Some(f) = m.files.first() else { continue };
            match self.models.places.iter().position(|(_, p)| f.path.starts_with(p)) {
                Some(k) => {
                    counts[k].0 += 1;
                    counts[k].1 += m.size;
                }
                None => {
                    in_analysis.0 += 1;
                    in_analysis.1 += m.size;
                }
            }
        }
        let mut remove = None;
        let mut reveal = None;
        for (k, (label, p)) in self.models.places.iter().enumerate() {
            let (n, bytes) = counts[k];
            let icon = match label.as_str() {
                "Downloads" => Icon::ArrowUp,
                "Added" => Icon::Folder,
                _ => Icon::Drive,
            };
            let r = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    crate::kit::icons::show(ui, icon, 14.0, if n > 0 { t.accent } else { t.weak });
                    ui.label(egui::RichText::new(label.as_str()).color(if n > 0 { t.strong } else { t.text }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if label == "Added"
                            && let Some(i) = self.files.prefs.model_folders.iter().position(|f| f == p)
                            && kit::icon_button(ui, Icon::Close, "Stop looking here").clicked()
                        {
                            remove = Some(i);
                        }
                        if n > 0 {
                            ui.label(egui::RichText::new(format!("{n} · {}", scan::human(bytes))).size(11.5).color(t.weak));
                        }
                    });
                })
                .response;
            let path = ui.add(egui::Label::new(egui::RichText::new(p.shown()).size(11.0).color(t.weak)).wrap().sense(Sense::click()));
            if r.union(path.clone()).on_hover_text("Click to show this folder in Files").clicked() || path.clicked() {
                reveal = Some(p.clone());
            }
            ui.add_space(5.0);
        }
        if self.tree.is_some() && self.job.is_none() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                crate::kit::icons::show(ui, Icon::Treemap, 14.0, if in_analysis.0 > 0 { t.accent } else { t.weak });
                ui.label("Storage Analysis scan");
                if in_analysis.0 > 0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("{} · {}", in_analysis.0, scan::human(in_analysis.1))).size(11.5).color(t.weak));
                    });
                }
            });
            ui.add(egui::Label::new(egui::RichText::new(self.scanned_path.shown()).size(11.0).color(t.weak)).wrap());
        }
        if self.models.places.is_empty() && self.tree.is_none() {
            ui.weak("No Ollama, LM Studio or Hugging Face folders on this computer.");
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new("Models elsewhere? Use Add folder, or scan a drive in Storage Analysis and look again.").size(11.5).color(t.weak));
        if let Some(i) = remove {
            self.files.prefs.model_folders.remove(i);
            self.start_models();
        }
        if let Some(p) = reveal {
            self.set_workspace(Workspace::Files);
            self.files.reveal(p, Vec::new());
        }
    }

    /// Middle: the totals, the controls and the list.
    pub(super) fn models_main(&mut self, ui: &mut egui::Ui) {
        if let Some(j) = &self.models.job {
            let (read, total) = (j.progress.read.load(Ordering::Relaxed), j.progress.to_read.load(Ordering::Relaxed));
            let status = if total > 0 {
                format!("Checking copies… {} of {}", scan::human(read), scan::human(total))
            } else {
                j.status.lock().unwrap().clone()
            };
            let mut cancel = false;
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(status);
                cancel = ui.small_button("Cancel").clicked();
            });
            if total > 0 {
                kit::progress(ui, read as f32 / total.max(1) as f32, Tone::Accent);
            }
            if cancel {
                self.cancel_models();
            }
            return;
        }
        let Some(list) = self.models.list.take() else {
            ui.label("The search was cancelled.");
            if ui.button("Look again").clicked() {
                self.start_models();
            }
            return;
        };
        self.models_list(ui, &list);
        self.models.list = Some(list);
    }

    fn models_list(&mut self, ui: &mut egui::Ui, list: &[Model]) {
        let t = kit::tokens(ui);
        if list.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                crate::kit::icons::show(ui, Icon::Chat, 40.0, t.weak);
                ui.add_space(8.0);
                ui.label(egui::RichText::new("No AI models found").size(kit::TITLE).color(t.strong));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Filescape looks where Ollama, LM Studio and Hugging Face keep their models, in Downloads, \
                             in folders you add and in the Storage Analysis scan, for model files (.gguf, .safetensors, \
                             .ckpt, .pt, .onnx …) of 16 MB or more.",
                        )
                        .color(t.text),
                    )
                    .wrap(),
                );
                ui.add_space(10.0);
                if kit::button(ui, kit::Kind::Primary, Some(Icon::Chat), "Get a small model to try").on_hover_text("Ask AI can download a small, free model (you see what and how big first)").clicked() {
                    self.ask.open = true;
                    self.ask.offer_starter();
                }
            });
            return;
        }
        let kinds: Vec<(Kind, bool)> = list.iter().map(modelkind::of).collect();

        // The totals.
        self.library_totals(ui, list);
        if let Some((tone, text)) = self.models.storage.outcome.clone() {
            ui.horizontal(|ui| {
                kit::callout(ui, tone, text);
                if kit::icon_button(ui, Icon::Close, "Hide").clicked() {
                    self.models.storage.outcome = None;
                }
            });
        }
        ui.add_space(6.0);

        // Type chips, then search, sort and filters.
        let mut present: BTreeMap<Kind, usize> = BTreeMap::new();
        for (m, (k, _)) in list.iter().zip(&kinds) {
            if !m.orphan {
                *present.entry(*k).or_default() += 1;
            }
        }
        let lib = &mut self.models.lib;
        if lib.kind.is_some_and(|k| !present.contains_key(&k)) {
            lib.kind = None;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // The chips wrap onto a second line rather than run under the controls on the right.
            let chips_w = ui.available_width() - 270.0;
            let all = list.iter().filter(|m| !m.orphan).count();
            let font = FontId::proportional(13.0);
            let width_of = |ui: &egui::Ui, s: &str| ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), t.text).size().x);
            let needed: f32 = width_of(ui, &format!("All  {all}")) + 24.0 + present.iter().map(|(k, n)| width_of(ui, &format!("{}  {n}", k.short())) + 44.0).sum::<f32>();
            if needed <= chips_w {
                // A chip for each kind found.
                if viz::pill(ui, None, &format!("All  {all}"), lib.kind.is_none(), false, false).clicked() {
                    lib.kind = None;
                }
                for (&k, &n) in &present {
                    let on = lib.kind == Some(k);
                    let r = viz::pill(ui, Some(kind_icon(k)), &format!("{}  {n}", k.short()), on, false, false).on_hover_text(k.explain());
                    if r.clicked() {
                        lib.kind = if on { None } else { Some(k) };
                    }
                }
            } else {
                // Too narrow for every chip: one that opens a list.
                let label = match lib.kind {
                    Some(k) => format!("{}  {}", k.short(), present.get(&k).copied().unwrap_or(0)),
                    None => format!("All types  {all}"),
                };
                let r = viz::pill(ui, lib.kind.map(kind_icon), &label, lib.kind.is_some(), false, true);
                egui::Popup::menu(&r).show(|ui| {
                    if ui.radio(lib.kind.is_none(), format!("All  {all}")).clicked() {
                        lib.kind = None;
                        ui.close();
                    }
                    for (&k, &n) in &present {
                        if ui.radio(lib.kind == Some(k), format!("{}  {n}", k.label())).on_hover_text(k.explain()).clicked() {
                            lib.kind = Some(k);
                            ui.close();
                        }
                    }
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let n = [lib.source.is_some(), lib.format.is_some(), lib.runnable, lib.attention, lib.fits].iter().filter(|b| **b).count();
                let label = if n > 0 { format!("Filters  {n}") } else { "Filters".to_string() };
                let r = viz::pill(ui, Some(Icon::Filter), &label, n > 0, false, true).on_hover_text("Show only some models: by app, format, or what Filescape can run");
                egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    ui.set_min_width(220.0);
                    ui.label(egui::RichText::new("App").size(11.0).color(t.weak));
                    for s in [None, Some(Source::Ollama), Some(Source::LmStudio), Some(Source::HuggingFace), Some(Source::Folder)] {
                        let name = s.map_or("Any", |s| s.label());
                        if ui.radio(lib.source == s, name).clicked() {
                            lib.source = s;
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Format").size(11.0).color(t.weak));
                    for f in [None, Some(Format::Gguf), Some(Format::Safetensors), Some(Format::PyTorch), Some(Format::Onnx), Some(Format::CoreMl), Some(Format::Other)] {
                        let name = f.map_or("Any", |f| f.label());
                        if ui.radio(lib.format == f, name).clicked() {
                            lib.format = f;
                        }
                    }
                    ui.separator();
                    ui.checkbox(&mut lib.runnable, "Runs in Filescape").on_hover_text("Whole GGUF models on this computer: Ask AI and the Lab can load them");
                    ui.checkbox(&mut lib.fits, "Fits this computer").on_hover_text(
                        "Models Filescape can run whose size, with room for a short conversation (a few thousand tokens), fits the memory it can use here. An estimate from the file size; Will it run? on a model's page gives the details.",
                    );
                    ui.checkbox(&mut lib.attention, "Copies and unused downloads").on_hover_text("Models with space that can be freed");
                    if lib.filtering() {
                        ui.separator();
                        if ui.button("Clear filters").clicked() {
                            lib.source = None;
                            lib.format = None;
                            lib.runnable = false;
                            lib.fits = false;
                            lib.attention = false;
                        }
                    }
                });
                let r = viz::pill(ui, None, &format!("Sort: {}", lib.sort.label()), false, false, true);
                egui::Popup::menu(&r).show(|ui| {
                    for s in [Sort::Size, Sort::Name, Sort::Kind, Sort::Source] {
                        if ui.radio(lib.sort == s, s.label()).clicked() {
                            lib.sort = s;
                            ui.close();
                        }
                    }
                });
                viz::switch(ui, &mut lib.grouped, &[(false, Some(Icon::List), "", "Every model file on its own line"), (true, Some(Icon::Layers), "", "Versions and precisions of one model together")]);
            });
        });
        ui.add_space(6.0);

        // The lines to show.
        let hw = super::hardware(ui.ctx());
        let shown: Vec<usize> = (0..list.len()).filter(|&i| lib.matches(&list[i], kinds[i].0, hw)).collect();
        let mut order = shown.clone();
        let by = |a: &usize, b: &usize| -> std::cmp::Ordering {
            let (ma, mb) = (&list[*a], &list[*b]);
            match lib.sort {
                Sort::Size => mb.size.cmp(&ma.size),
                Sort::Name => ma.name.to_lowercase().cmp(&mb.name.to_lowercase()),
                Sort::Kind => kinds[*a].0.cmp(&kinds[*b].0).then(mb.size.cmp(&ma.size)),
                Sort::Source => ma.source.label().cmp(mb.source.label()).then(mb.size.cmp(&ma.size)),
            }
        };
        order.sort_by(by);
        let mut rows = Vec::new();
        if lib.grouped {
            // Groups in the order of their first member; members in the chosen order.
            let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
            for &i in &order {
                let key = modelkind::family_key(&list[i]);
                match groups.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, v)) => v.push(i),
                    None => groups.push((key, vec![i])),
                }
            }
            for (key, members) in groups {
                if members.len() == 1 {
                    rows.push(Row::Model { i: members[0], child: false });
                    continue;
                }
                let open = lib.open.contains(&key);
                let kids = members.clone();
                rows.push(Row::Group { key, members });
                if open {
                    rows.extend(kids.into_iter().map(|i| Row::Model { i, child: true }));
                }
            }
        } else {
            rows.extend(order.iter().map(|&i| Row::Model { i, child: false }));
        }
        if rows.is_empty() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("No models match").color(t.strong));
                if ui.link("Show all models").clicked() {
                    lib.search.clear();
                    lib.kind = None;
                    lib.source = None;
                    lib.format = None;
                    lib.runnable = false;
                            lib.fits = false;
                    lib.attention = false;
                }
            });
            return;
        }

        let mut chosen = None;
        let mut act: Option<(usize, Act)> = None;
        let mut toggle = None;
        let heights: Vec<f32> = rows.iter().map(|r| if matches!(r, Row::Group { .. }) { GROUP_ROW } else { ROW }).collect();
        ui.spacing_mut().item_spacing.y = 0.0;
        let current = self.models.chosen;
        egui::ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, view| {
            let total: f32 = heights.iter().sum();
            ui.set_height(total);
            let top = ui.min_rect().top();
            let mut y = 0.0;
            for (r, h) in rows.iter().zip(&heights) {
                let (y0, y1) = (y, y + h);
                y = y1;
                if y1 < view.min.y || y0 > view.max.y {
                    continue;
                }
                let rect = Rect::from_min_size(pos2(ui.min_rect().left(), top + y0), vec2(ui.available_width(), *h));
                match r {
                    Row::Group { key, members } => {
                        let resp = ui.interact(rect, ui.id().with(("group", key)), Sense::click());
                        if group_row(ui, rect, &resp, list, members, &kinds, self.models.lib.open.contains(key)) {
                            toggle = Some(key.clone());
                        }
                    }
                    Row::Model { i, child } => {
                        let i = *i;
                        let m = &list[i];
                        let resp = ui.interact(rect, ui.id().with(("model", i)), Sense::click());
                        model_row(ui, rect, &resp, m, kinds[i], *child, current == Some(i));
                        let more = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(24.0, 24.0));
                        let more_r = ui.interact(more, ui.id().with(("more", i)), Sense::click()).on_hover_text("More actions");
                        if more_r.hovered() {
                            ui.painter().rect_filled(more, 5.0, t.pal.hover);
                        }
                        crate::kit::icons::paint(ui.painter(), more.shrink(5.0), Icon::More, if more_r.hovered() || resp.hovered() { t.strong } else { t.weak });
                        egui::Popup::menu(&more_r).show(|ui| {
                            if let Some(a) = actions_menu(ui, list, i) {
                                act = Some((i, a));
                            }
                        });
                        let resp = resp.on_hover_text(m.files.first().map(|f| f.path.shown()).unwrap_or_default());
                        if resp.clicked() {
                            chosen = Some(i);
                        }
                        if resp.double_clicked() {
                            act = Some((i, Act::Inspect));
                        }
                        resp.context_menu(|ui| {
                            if let Some(a) = actions_menu(ui, list, i) {
                                act = Some((i, a));
                            }
                        });
                    }
                }
            }
        });
        if let Some(k) = toggle {
            let open = &mut self.models.lib.open;
            if !open.remove(&k) {
                open.insert(k);
            }
        }
        if let Some(i) = chosen {
            self.models.chosen = Some(i);
        }
        if let Some((i, a)) = act {
            self.model_act(list, i, a);
        }
    }

    /// The strip of totals above the list.
    fn library_totals(&mut self, ui: &mut egui::Ui, list: &[Model]) {
        let t = kit::tokens(ui);
        let fp: Footprint = self.models.footprint;
        let count = list.iter().filter(|m| !m.orphan).count() as u64;
        // Extra copies aren't counted: Filescape doesn't remove them (an app may load either one).
        let unused: u64 = list.iter().filter(|m| m.orphan).map(|m| m.size).sum();
        let recoverable = unused;
        let runnable = list.iter().filter(|m| m.runnable() && m.extra == 0).count();
        let used_places = {
            let mut s = HashSet::new();
            for m in list {
                if let Some(f) = m.files.first() {
                    s.insert(self.models.places.iter().position(|(_, p)| f.path.starts_with(p)));
                }
            }
            s.len()
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            super::super::summary::stat(ui, "Models", &group(count), "Models found, not counting unused Ollama downloads.", None);
            super::super::summary::divider(ui);
            let disk_tip = if fp.shared > 0 {
                format!(
                    "Space the model files take on the drive. Files shared as one (hard links) are counted once: the files add up to {}, of which {} is shared.",
                    scan::human(fp.logical),
                    scan::human(fp.shared)
                )
            } else {
                format!("Space the model files take on the drive. Their sizes add up to {}.", scan::human(fp.logical))
            };
            super::super::summary::stat(ui, "On disk", &scan::human(fp.on_disk), &disk_tip, None);
            super::super::summary::stat(
                ui,
                "Recoverable",
                &scan::human(recoverable),
                &format!(
                    "{} in Ollama downloads no model uses. Storage also finds unfinished downloads and old versions. Extra copies are marked but not counted: an app may load either copy.",
                    scan::human(unused)
                ),
                (recoverable > 0).then_some(t.success),
            );
            super::super::summary::divider(ui);
            super::super::summary::stat(ui, "Locations", &used_places.to_string(), "Folders models were found in, including the Storage Analysis scan.", None);
            super::super::summary::stat(
                ui,
                "Runs in Filescape",
                &runnable.to_string(),
                "Whole GGUF models on this computer, which Ask AI and the Lab can load. Other formats are listed and inspected, not run.",
                (runnable > 0).then_some(t.accent),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let w = ui.available_width().clamp(140.0, 240.0);
                ui.allocate_ui(vec2(w, 32.0), |ui| kit::search_field(ui, &mut self.models.lib.search, "Search models"));
                if recoverable > 0
                    && ui.available_width() > 130.0
                    && viz::pill(ui, Some(Icon::Layers), "Free up space", false, false, false).on_hover_text("Open Storage: what can be freed, and why").clicked()
                {
                    self.models.storage.open = true;
                }
            });
        });
    }

    /// Right: the chosen model's facts and everything that can be done with it.
    pub(super) fn model_details(&mut self, ui: &mut egui::Ui) {
        let Some(list) = self.models.list.take() else { return };
        let Some(i) = self.models.chosen.filter(|&i| i < list.len()) else {
            self.models.list = Some(list);
            return;
        };
        let m = &list[i];
        let t = kit::tokens(ui);
        let (kind, sure) = modelkind::of(m);
        let mut act = None;
        let mut pick = None;
        let mut close = false;

        // Heading: the kind's tile, the name, what it is.
        ui.horizontal(|ui| {
            let (tile, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
            kind_tile(ui.painter(), tile, kind, sure, t.dark);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 30.0);
                ui.add(egui::Label::new(egui::RichText::new(&m.name).size(16.0).color(t.strong)).wrap());
                ui.label(egui::RichText::new(kind_words(kind, sure)).size(12.0).color(kind_colour(kind, t.dark))).on_hover_text(kind_tip(kind, sure));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                close = kit::icon_button(ui, Icon::Close, "Close").clicked();
            });
        });
        ui.add_space(8.0);
        for (tone, text) in statuses(m) {
            kit::badge(ui, &text, tone);
            ui.add_space(2.0);
        }
        ui.add_space(4.0);

        // Four facts as tiles.
        let info = &m.info;
        let tiles = [
            ("Parameters", info.params.clone().unwrap_or_else(|| "Unknown".into()), "How many numbers (weights) the model has. More usually means more capable and more memory."),
            ("Size", scan::human(m.size), "The model's files added up."),
            ("Precision", info.quant.clone().unwrap_or_else(|| "Unknown".into()), "How each weight is stored. Fewer bits make a smaller file that needs less memory, at some cost to quality."),
            (
                "Context",
                info.context.map(|c| format!("{} tokens", crate::fit::tokens(c))).unwrap_or_else(|| "Unknown".into()),
                "The longest text the model was trained to read at once, in tokens (a token is about three quarters of a word).",
            ),
        ];
        let w = ((ui.available_width() - 8.0) / 2.0).max(80.0);
        egui::Grid::new("model-tiles").num_columns(2).spacing([8.0, 8.0]).show(ui, |ui| {
            for (k, (label, value, tip)) in tiles.iter().enumerate() {
                fact_tile(ui, w, label, value, tip);
                if k % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        ui.add_space(10.0);

        // Whether Filescape can run it.
        let (tone, text) = if m.runnable() && !modelkind::chats(m) {
            (Tone::Neutral, "Filescape can load this model, but it doesn't write text, so Ask AI and the Lab can't use it.".to_string())
        } else if m.runnable() {
            (Tone::Success, "Filescape can run this model: chat with it in Ask AI, or look inside it in the Lab.".to_string())
        } else {
            (Tone::Neutral, why_not_runnable(m))
        };
        kit::callout(ui, tone, text);
        ui.add_space(10.0);

        // Actions.
        ui.label(egui::RichText::new("ACTIONS").size(10.5).color(t.weak).strong());
        ui.add_space(4.0);
        let compare_to: Vec<usize> = (0..list.len()).filter(|&o| o != i && comparable(&list[o])).collect();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if kit::button(ui, kit::Kind::Primary, Some(Icon::Search), "Inspect").on_hover_text("What's inside: its architecture, layers, tensors and metadata").clicked() {
                act = Some(Act::Inspect);
            }
            if m.runnable() && modelkind::chats(m) {
                if kit::button(ui, kit::Kind::Secondary, Some(Icon::Chat), "Ask AI").on_hover_text("Chat with this model, run by Filescape on this computer").clicked() {
                    act = Some(Act::Ask);
                }
                if kit::button(ui, kit::Kind::Secondary, Some(Icon::Flask), "Open in Lab").on_hover_text("See which word it would write next, and what happens inside it").clicked() {
                    act = Some(Act::Lab);
                }
            }
            if !m.info.companion && m.format == Format::Gguf && !m.files.iter().any(|f| f.not_local) {
                if kit::button(ui, kit::Kind::Secondary, Some(Icon::Gauge), "Will it run?").on_hover_text("How much memory it needs, and whether this computer has enough").clicked() {
                    act = Some(Act::Fit);
                }
            }
            if comparable(m) && !compare_to.is_empty() {
                let r = kit::button(ui, kit::Kind::Secondary, Some(Icon::Columns), "Compare…").on_hover_text("Side by side with another model: facts, layers, tensors and metadata");
                egui::Popup::menu(&r).show(|ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for &o in &compare_to {
                            if ui.button(format!("{}  ·  {}", list[o].name, list[o].details())).clicked() {
                                act = Some(Act::Compare(o));
                                ui.close();
                            }
                        }
                    });
                });
            }
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if kit::button(ui, kit::Kind::Ghost, Some(Icon::Folder), "Show in Files").clicked() {
                act = Some(Act::ShowFiles);
            }
            if kit::button(ui, kit::Kind::Ghost, Some(Icon::Treemap), "Show in Storage Analysis").on_hover_text("See it on the map with everything around it").clicked() {
                act = Some(Act::ShowAnalysis);
            }
            if m.runnable() && kit::button(ui, kit::Kind::Ghost, Some(Icon::Layers), "Optimise size…").on_hover_text("Make a smaller copy at a lower precision (quantise). The original is kept. Professional.").clicked() {
                act = Some(Act::Shrink);
            }
            if kit::button(ui, kit::Kind::Ghost, Some(Icon::Trash), if cfg!(windows) { "Move to Recycle Bin…" } else { "Move to Trash…" }).on_hover_text("Remove this model. You see everything it removes first.").clicked() {
                act = Some(Act::Trash);
            }
        });
        ui.add_space(12.0);

        // Files, copies.
        let files_label = if m.files.len() == 1 { "FILE".to_string() } else { format!("FILES ({})", m.files.len()) };
        ui.label(egui::RichText::new(files_label).size(10.5).color(t.weak).strong());
        ui.add_space(2.0);
        for (k, f) in m.files.iter().enumerate() {
            let r = ui.add(egui::Label::new(egui::RichText::new(f.path.shown()).size(12.0).color(t.text)).wrap().sense(Sense::click())).on_hover_text("Click to show it in Files");
            if r.clicked() {
                self.show_model_file_later(f.path.clone());
            }
            let mut note = scan::human(f.size);
            if f.not_local {
                note.push_str(" · in the cloud, not read");
            }
            if let Some((_, kept)) = m.same_as.iter().find(|(fi, _)| *fi == k) {
                note.push_str(&format!(" · same as {}", kept.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
            }
            ui.label(egui::RichText::new(note).size(11.0).color(t.weak));
            ui.add_space(3.0);
        }
        if !m.copies.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("THE SAME FILES ARE ALSO IN").size(10.5).color(t.weak).strong());
            for &c in &m.copies {
                if let Some(o) = list.get(c) {
                    let r = ui.add(egui::Label::new(format!("{} ({})", o.name, o.source.label())).sense(Sense::click()));
                    if r.on_hover_text(o.files.first().map(|f| f.path.shown()).unwrap_or_default()).clicked() {
                        pick = Some(c);
                    }
                }
            }
        }
        if close {
            self.models.chosen = None;
        }
        if let Some(a) = act {
            self.model_act(&list, i, a);
        }
        self.models.list = Some(list);
        if pick.is_some() {
            self.models.chosen = pick;
        }
        if let Some(p) = self.models.reveal_later.take() {
            self.show_model_file(&p);
        }
    }

    fn show_model_file_later(&mut self, p: PathBuf) {
        self.models.reveal_later = Some(p);
    }

    /// Does what was asked with model `i` of `list`.
    pub(super) fn model_act(&mut self, list: &[Model], i: usize, a: Act) {
        let Some(m) = list.get(i) else { return };
        match a {
            Act::Inspect => {
                self.models.chosen = Some(i);
                self.inspect_model(i, m);
            }
            Act::Fit => {
                self.models.chosen = Some(i);
                self.inspect_model(i, m);
                if let Some(p) = &mut self.models.page {
                    p.tab = PageTab::Fit;
                }
            }
            Act::Compare(o) => {
                let Some(other) = list.get(o) else { return };
                self.models.chosen = Some(i);
                self.inspect_model(i, m);
                let (tx, rx) = mpsc::channel();
                let other = other.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(inspect::inspect(&other));
                });
                if let Some(p) = &mut self.models.page {
                    p.compare_with = Some(o);
                    p.compare_rx = Some(rx);
                    p.tab = PageTab::Compare;
                }
            }
            Act::Ask => self.ask_about_model(m),
            Act::Lab => {
                self.ask_about_model(m);
                self.ask.lab = true;
            }
            Act::ShowFiles => {
                if let Some(f) = m.files.first() {
                    self.show_model_file(&f.path.clone());
                }
            }
            Act::ShowAnalysis => {
                if let Some(f) = m.files.first() {
                    let place = self.models.places.iter().find(|(_, p)| f.path.starts_with(p)).map(|(_, p)| p.clone());
                    self.show_model_in_analysis(&f.path.clone(), place);
                }
            }
            Act::Shrink => self.ask_shrink(m),
            Act::Trash => self.ask_remove_model(m),
        }
    }

    /// Storage Analysis with the model's file selected: in the current scan when it covers the
    /// file, otherwise a new scan of the place the model was found in (or its folder).
    fn show_model_in_analysis(&mut self, path: &Path, place: Option<PathBuf>) {
        if self.job.is_none()
            && let Some(tp) = self.tree_path_of(path)
        {
            self.set_workspace(Workspace::Analysis);
            if tp.len() > 1 {
                self.zoom = tp[..tp.len() - 1].to_vec();
            }
            self.select(tp);
            return;
        }
        let Some(dir) = place.or_else(|| path.parent().map(Path::to_path_buf)) else { return };
        self.set_workspace(Workspace::Analysis);
        self.path_input = dir.shown();
        self.start_scan_of(dir, false);
    }
}

/// The menu of what can be done with a model, from its ⋯ button or a right-click.
fn actions_menu(ui: &mut egui::Ui, list: &[Model], i: usize) -> Option<Act> {
    let m = &list[i];
    let mut act = None;
    let item = |ui: &mut egui::Ui, icon: Icon, text: &str, a: Act, act: &mut Option<Act>| {
        if menu_item(ui, icon, text).clicked() {
            *act = Some(a);
            ui.close();
        }
    };
    item(ui, Icon::Search, "Inspect", Act::Inspect, &mut act);
    if m.runnable() && modelkind::chats(m) {
        item(ui, Icon::Chat, "Ask AI", Act::Ask, &mut act);
        item(ui, Icon::Flask, "Open in Lab", Act::Lab, &mut act);
    }
    if !m.info.companion && m.format == Format::Gguf && !m.files.iter().any(|f| f.not_local) {
        item(ui, Icon::Gauge, "Will it run?", Act::Fit, &mut act);
    }
    if comparable(m) {
        let others: Vec<usize> = (0..list.len()).filter(|&o| o != i && comparable(&list[o])).collect();
        if !others.is_empty() {
            ui.menu_button("Compare with", |ui| {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for o in others {
                        if ui.button(format!("{}  ·  {}", list[o].name, list[o].details())).clicked() {
                            act = Some(Act::Compare(o));
                            ui.close();
                        }
                    }
                });
            });
        }
    }
    ui.separator();
    item(ui, Icon::Folder, "Show in Files", Act::ShowFiles, &mut act);
    item(ui, Icon::Treemap, "Show in Storage Analysis", Act::ShowAnalysis, &mut act);
    if menu_item(ui, Icon::Copy, "Copy path").clicked() {
        if let Some(f) = m.files.first() {
            ui.ctx().copy_text(f.path.shown());
        }
        ui.close();
    }
    if m.runnable() {
        ui.separator();
        item(ui, Icon::Layers, "Optimise size (quantise)…", Act::Shrink, &mut act);
    }
    ui.separator();
    item(ui, Icon::Trash, if cfg!(windows) { "Move to Recycle Bin…" } else { "Move to Trash…" }, Act::Trash, &mut act);
    act
}

/// A menu line with an icon before its text.
fn menu_item(ui: &mut egui::Ui, icon: Icon, text: &str) -> egui::Response {
    let t = kit::tokens(ui);
    let r = ui.add(egui::Button::new(format!("      {text}")).frame(false).min_size(vec2(ui.available_width().max(200.0), 22.0)));
    let c = Rect::from_center_size(pos2(r.rect.left() + 12.0, r.rect.center().y), vec2(14.0, 14.0));
    crate::kit::icons::paint(ui.painter(), c, icon, if r.hovered() { t.strong } else { t.weak });
    r
}

/// Whether a model can be compared: it can be read here and isn't an add-on.
fn comparable(m: &Model) -> bool {
    !m.info.companion && !m.files.iter().all(|f| f.not_local) && matches!(m.format, Format::Gguf | Format::Safetensors)
}

/// Why Filescape can't run a model itself, in a sentence.
fn why_not_runnable(m: &Model) -> String {
    if m.orphan {
        return "An Ollama download no installed model uses. Nothing runs it.".into();
    }
    if m.info.companion {
        return "An add-on that goes with another model (a vision projector or adapter). It doesn't run on its own.".into();
    }
    if m.files.iter().any(|f| f.not_local) {
        return "Some of its files are only in the cloud. Download them to run it.".into();
    }
    match m.format {
        Format::Gguf => "Filescape can't run this file.".into(),
        f => format!(
            "Filescape runs GGUF models. This one is {}: it can be inspected here and run by the app that uses it.",
            match f {
                Format::Safetensors => "in safetensors format",
                Format::PyTorch => "a PyTorch checkpoint",
                Format::Onnx => "an ONNX model",
                Format::CoreMl => "a Core ML model",
                _ => "in another format",
            }
        ),
    }
}

/// Badges for what's notable about a model: copies, unused, add-on, in the cloud.
fn statuses(m: &Model) -> Vec<(Tone, String)> {
    let mut v = Vec::new();
    if m.orphan {
        v.push((Tone::Warn, "Unused download".to_string()));
    }
    if m.extra > 0 {
        v.push((Tone::Neutral, format!("Extra copy · {}", scan::human(m.extra))));
    } else if !m.copies.is_empty() {
        v.push((Tone::Neutral, "Also kept elsewhere".to_string()));
    }
    if m.info.companion {
        v.push((Tone::Neutral, "Add-on".to_string()));
    }
    if m.files.iter().any(|f| f.not_local) {
        v.push((Tone::Neutral, "In the cloud".to_string()));
    }
    v
}

/// A fact as a small tile: label over value.
pub(super) fn fact_tile(ui: &mut egui::Ui, w: f32, label: &str, value: &str, tip: &str) {
    let t = kit::tokens(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 46.0), Sense::hover());
    ui.painter().rect_filled(rect, 6.0, t.pal.well);
    ui.painter().text(rect.left_top() + vec2(10.0, 7.0), Align2::LEFT_TOP, label.to_uppercase(), FontId::proportional(10.0), t.weak);
    let unknown = value == "Unknown";
    let font = FontId::proportional(if unknown { 13.0 } else { 15.0 });
    let clip = ui.painter().with_clip_rect(rect.shrink(2.0));
    clip.text(rect.left_top() + vec2(10.0, 21.0), Align2::LEFT_TOP, value, font, if unknown { t.weak } else { t.strong });
    resp.on_hover_text(if unknown { format!("{tip}\n\nThe model's files don't say.") } else { tip.to_string() });
}

/// One model line: its kind tile, name, facts, badges and size.
fn model_row(ui: &egui::Ui, rect: Rect, resp: &egui::Response, m: &Model, (kind, sure): (Kind, bool), child: bool, selected: bool) {
    let t = kit::tokens(ui);
    let vis = ui.visuals();
    let p = ui.painter();
    let inner = rect.shrink2(vec2(2.0, 2.0));
    if selected {
        p.rect_filled(inner, 6.0, vis.selection.bg_fill);
    } else if resp.hovered() {
        p.rect_filled(inner, 6.0, vis.widgets.hovered.weak_bg_fill);
    }
    let left = rect.left() + if child { 30.0 } else { 8.0 };
    let tile = Rect::from_min_size(pos2(left, rect.center().y - 15.0), vec2(30.0, 30.0));
    kind_tile(p, tile, kind, sure, t.dark);
    if m.orphan || m.extra > 0 {
        // A warning dot on the tile's corner.
        p.circle_filled(tile.right_top() + vec2(-2.0, 2.0), 4.0, t.warn);
    }
    let x = tile.right() + 10.0;
    let right = rect.right() - 36.0;
    let size_g = p.layout_no_wrap(scan::human(m.size), FontId::proportional(13.5), t.strong);
    let size_w = size_g.size().x;
    let clip = p.with_clip_rect(Rect::from_min_max(pos2(x, rect.top()), pos2(right - size_w - 12.0, rect.bottom())).intersect(ui.clip_rect()));
    let name_c = if selected { vis.selection.stroke.color } else { t.strong };
    let top = rect.top() + rect.height() * 0.33;
    let bottom = rect.top() + rect.height() * 0.68;
    clip.text(pos2(x, top), Align2::LEFT_CENTER, &m.name, FontId::proportional(13.5), name_c);
    // The second line: kind · family · size · precision · context · app, then notes.
    let small = FontId::proportional(12.0);
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c: Color32| egui::TextFormat { font_id: small.clone(), color: c, ..Default::default() };
    job.append(&kind_words(kind, sure), 0.0, fmt(kind_colour(kind, t.dark)));
    let mut rest = Vec::new();
    rest.push(m.format.label().to_string());
    rest.extend(m.info.arch.clone());
    rest.extend(m.info.params.clone());
    rest.extend(m.info.quant.clone());
    if let Some(c) = m.info.context.filter(|&c| c > 0) {
        rest.push(format!("{} context", crate::fit::tokens(c)));
    }
    if m.files.len() > 1 {
        rest.push(format!("{} files", m.files.len()));
    }
    job.append(&format!("  ·  {}", rest.join(" · ")), 0.0, fmt(t.weak));
    if let Some(n) = note(m) {
        job.append(&format!("   {n}"), 0.0, fmt(t.warn));
    }
    let g = ui.painter().layout_job(job);
    clip.galley(pos2(x, bottom - g.size().y / 2.0), g, t.text);
    p.galley(pos2(right - size_w, top - size_g.size().y / 2.0), size_g, t.strong);
    let src = p.layout_no_wrap(m.source.label().to_string(), small.clone(), t.weak);
    let sw = src.size().x;
    p.galley(pos2(right - sw, bottom - src.size().y / 2.0), src, t.weak);
    if m.runnable() {
        // A small "runs here" mark beside the app.
        let c = pos2(right - sw - 9.0, bottom);
        p.circle_filled(c, 3.0, t.success);
    }
    p.hline(rect.x_range().shrink(6.0), rect.bottom() - 0.5, Stroke::new(1.0, t.divider.gamma_multiply(0.6)));
}

/// A group heading: the family, how many versions, their sizes; returns true when clicked.
fn group_row(ui: &egui::Ui, rect: Rect, resp: &egui::Response, list: &[Model], members: &[usize], kinds: &[(Kind, bool)], open: bool) -> bool {
    let t = kit::tokens(ui);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect.shrink2(vec2(2.0, 2.0)), 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let first = &list[members[0]];
    let chevron = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(12.0, 12.0));
    let ch = if open { Icon::ChevronDown } else { Icon::ArrowRight };
    crate::kit::icons::paint(p, chevron, ch, t.weak);
    let (kind, sure) = kinds[members[0]];
    let tile = Rect::from_min_size(pos2(rect.left() + 28.0, rect.center().y - 11.0), vec2(22.0, 22.0));
    kind_tile(p, tile, kind, sure, t.dark);
    let total: u64 = members.iter().map(|&i| list[i].size).sum();
    let quants: Vec<String> = {
        let mut v: Vec<String> = members.iter().filter_map(|&i| list[i].info.quant.clone()).collect();
        v.sort();
        v.dedup();
        v
    };
    let mut apps: Vec<&str> = members.iter().map(|&i| list[i].source.label()).collect();
    apps.sort();
    apps.dedup();
    let name = group_name(first);
    let x = tile.right() + 10.0;
    let mut job = egui::text::LayoutJob::default();
    job.append(&name, 0.0, egui::TextFormat { font_id: FontId::proportional(13.5), color: t.strong, ..Default::default() });
    let mut desc = format!("   {} versions", members.len());
    if !quants.is_empty() {
        desc.push_str(&format!(" · {}", quants.join(", ")));
    }
    desc.push_str(&format!(" · {}", apps.join(", ")));
    job.append(&desc, 0.0, egui::TextFormat { font_id: FontId::proportional(12.0), color: t.weak, ..Default::default() });
    let g = ui.painter().layout_job(job);
    let size_g = p.layout_no_wrap(scan::human(total), FontId::proportional(13.5), t.text);
    let right = rect.right() - 36.0;
    let clip = p.with_clip_rect(Rect::from_min_max(pos2(x, rect.top()), pos2(right - size_g.size().x - 12.0, rect.bottom())).intersect(ui.clip_rect()));
    clip.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, t.text);
    p.galley(pos2(right - size_g.size().x, rect.center().y - size_g.size().y / 2.0), size_g, t.text);
    p.hline(rect.x_range().shrink(6.0), rect.bottom() - 0.5, Stroke::new(1.0, t.divider.gamma_multiply(0.6)));
    resp.clicked()
}

/// A readable name for a group: the first model's name without its publisher and precision.
fn group_name(m: &Model) -> String {
    let mut n = m.name.clone();
    if let Some(i) = n.find(" (") {
        n.truncate(i);
    }
    if let Some(q) = &m.info.quant {
        for sep in ["-", "_", ".", ":"] {
            let suffix = format!("{sep}{q}");
            if n.to_lowercase().ends_with(&suffix.to_lowercase()) {
                n.truncate(n.len() - suffix.len());
                break;
            }
        }
    }
    n
}
