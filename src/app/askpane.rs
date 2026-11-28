//! Ask AI in the Files workspace: the preview pane's Ask AI tab. The selected file or folder is
//! what the conversation is about, without attaching anything. It shares Ask AI's model, settings
//! and engine with the AI Models workspace (see `ask.rs`) and has a conversation of its own,
//! which Expand carries over to AI Models. Like Ask AI everywhere, it only answers: Filescape never
//! acts on what the model writes.

use super::ask::{self, Phase, Which};
use super::*;
use crate::askabout;
use crate::asksuggest::{self, Mark, Needs, Preset};
use crate::kit::{self, Icon, Tone};
use crate::llm::Role;
use std::path::Path;

/// The standard disclaimer, under every Ask AI conversation.
pub(super) const DISCLAIMER: &str = "Local AI models can make mistakes. Always verify important information before acting on AI-generated responses.";

/// What the ⓘ beside the disclaimer explains.
const LIMITS: &str = "• AI models may give incorrect, incomplete or misleading answers.\n\
• Accuracy depends on the model you choose: its abilities, its size and how much context it has.\n\
• Smaller or heavily quantised models make more mistakes.\n\
• Check what it says about licences, legal documents, money and technical problems yourself.\n\
• Large files and folders may only be partly read, because a model can only take in so much at once. Filescape says when this happens.\n\
• Suggestions that files are redundant, duplicated or unneeded are guesses, not facts.\n\
• Filescape never runs commands, or changes, moves or deletes files, because of an answer.\n\n\
Private by design. Local by default. Always under your control.";

/// "Private · Local processing · Read-only" and the disclaimer, with the ⓘ that says more.
/// The same in AI Models, the Files workspace and Quick Look.
pub(super) fn ai_footer(ui: &mut egui::Ui) {
    let t = kit::tokens(ui);
    ui.vertical_centered(|ui| {
        ui.horizontal(|ui| {
            // Centre the first line by hand: horizontal layouts start at the left.
            let w = 230.0;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            crate::kit::icons::paint(ui.painter(), r, Icon::Lock, t.success);
            ui.label(egui::RichText::new("Private · Local processing · Read-only").size(11.5).color(t.weak));
            let (r, resp) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
            crate::kit::icons::paint(ui.painter(), r, Icon::Info, if resp.hovered() { t.accent } else { t.weak });
            resp.on_hover_ui(|ui| {
                ui.set_max_width(360.0);
                ui.strong("About Ask AI's answers");
                ui.add_space(4.0);
                ui.label(LIMITS);
            });
        });
        ui.add(egui::Label::new(egui::RichText::new(DISCLAIMER).size(11.0).color(t.weak)).wrap());
    });
}

/// The picture for a suggestion.
fn mark_icon(m: Mark) -> Icon {
    match m {
        Mark::Summary => Icon::File,
        Mark::Explain => Icon::Help,
        Mark::Search => Icon::Search,
        Mark::Warning => Icon::Warning,
        Mark::Check => Icon::Shield,
        Mark::Folder => Icon::Folder,
        Mark::Storage => Icon::Treemap,
        Mark::Chat => Icon::Chat,
    }
}

/// The file or folder's name, for the panel.
fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

impl App {
    /// Whether Ask AI shows in the Files workspace (Settings › Files › Ask AI).
    pub(super) fn ask_in_files(&self) -> bool {
        self.view().ask_in_files
    }

    /// Opens the preview pane on its Ask AI tab, about `path`.
    pub(super) fn open_files_ask(&mut self, path: Option<PathBuf>) {
        self.files.prefs.preview = true;
        self.files.prefs.ask_tab = true;
        if let Some(p) = path
            && self.ask.side.conversation.is_empty()
            && self.ask.side_ctx != [p.clone()]
        {
            self.side_about(vec![p]);
        }
    }

    /// From the right-click menu's Ask AI submenu: a conversation about `paths` in the preview
    /// pane, asking `preset`'s question as soon as the model has what it needs. Choosing it from
    /// the menu starts a new chat when the current one is about something else.
    pub(super) fn files_ask_preset(&mut self, paths: Vec<PathBuf>, preset: Preset) {
        if paths.is_empty() {
            return;
        }
        self.files.prefs.preview = true;
        self.files.prefs.ask_tab = true;
        let same = self.ask.side_ctx == paths;
        if !same && !(self.ask.phase == Phase::Answering && self.ask.asker == Which::Side) {
            self.ask.side.clear();
        }
        self.side_gather(paths, preset.needs());
        match preset.question() {
            Some(q) => {
                self.ask.side.draft = q.to_string();
                self.ask.side_send = true;
            }
            None => self.ask.side_send = false,
        }
    }

    /// Makes `paths` what the Files chat's next question is about, and remembers them as its
    /// context. Files Filescape can read are read; folders only by names and sizes.
    fn side_about(&mut self, paths: Vec<PathBuf>) {
        self.side_gather(paths, Needs::Facts);
    }

    /// Gathers what the model is told about `paths` off the UI thread: from the Storage Analysis
    /// scan when it covers them all (sizes include everything inside folders), otherwise from
    /// the folder listings. `needs` says what else goes with it.
    fn side_gather(&mut self, paths: Vec<PathBuf>, needs: Needs) {
        let all_files_readable = paths.iter().all(|p| !p.is_dir()) && paths.iter().any(|p| askabout::can_read(p));
        let read = match needs {
            Needs::Facts => all_files_readable,
            Needs::Contents => paths.iter().any(|p| !p.is_dir()),
            Needs::FolderDocuments => true,
            Needs::Siblings => false,
        };
        let budget = self.ask.context() as usize * 3 / 4;
        let tree = self.tree.clone().filter(|_| self.job.is_none()).map(|t| (t, self.scanned_path.clone()));
        let mut asked = paths.clone();
        if needs == Needs::Siblings
            && let Some(parent) = paths.first().and_then(|p| p.parent())
        {
            asked.push(parent.to_path_buf());
        }
        self.gather_about(Which::Side, move || {
            let mut f = match &tree {
                Some((t, root)) => {
                    let found: Option<Vec<(PathBuf, Vec<u32>)>> = asked.iter().map(|p| askabout::tree_index(t, root, p).map(|i| (p.clone(), i))).collect();
                    match found {
                        Some(v) => {
                            let nodes: Vec<(PathBuf, &crate::scan::Node)> = v.iter().filter_map(|(p, i)| t.at(i).map(|n| (p.clone(), n))).collect();
                            askabout::from_tree(&nodes, budget)
                        }
                        None => askabout::from_disk(&asked, budget),
                    }
                }
                None => askabout::from_disk(&asked, budget),
            };
            if needs == Needs::FolderDocuments
                && let Some(d) = asked.first()
            {
                f.readable = askabout::documents_in(d);
                // The documents read are sources of the answer too.
                f.paths.extend(f.readable.iter().cloned());
            }
            f
        });
        if let Some(a) = &mut self.ask.side.about {
            a.read = read;
        }
        self.ask.side_skip = None;
        self.ask.side_line.clear();
        self.ask.side_ctx = paths;
    }

    /// The Ask AI tab of the Files preview pane. `selection` is what's selected in the list (or
    /// the item under the cursor).
    pub(super) fn files_ask_pane(&mut self, ui: &mut egui::Ui, selection: Vec<PathBuf>) {
        let t = kit::tokens(ui);
        let pal = style::pal(ui);
        self.ensure_models();
        // Before the first question the context follows the selection; after it, a change is
        // offered, never made silently.
        let fresh = self.ask.side.conversation.is_empty() && self.ask.side.answering.is_none();
        let mut changed: Vec<PathBuf> = Vec::new();
        let differs = !selection.is_empty() && selection != self.ask.side_ctx && !(fresh && self.ask.side_send);
        if differs {
            if fresh {
                self.side_about(selection.clone());
            } else if self.ask.side_skip.as_ref() != selection.first() && !selection.iter().all(|p| self.ask.side_ctx.contains(p)) {
                changed = selection.clone();
            }
        }
        if self.ask.offering() {
            egui::ScrollArea::vertical().id_salt("files-ask-get").auto_shrink(false).show(ui, |ui| self.get_model(ui));
            return;
        }

        // The context: what the conversation is about.
        let ctx = self.ask.side_ctx.clone();
        if let Some(f) = self.ask.side.about.as_ref().and_then(|a| a.facts.as_ref())
            && f.paths.starts_with(&ctx)
        {
            self.ask.side_line = f.summary.clone();
        }
        let facts = (!self.ask.side_line.is_empty()).then(|| self.ask.side_line.clone());
        let mut new_chat = false;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            let dir = ctx.first().is_some_and(|p| p.is_dir());
            crate::kit::icons::paint(ui.painter(), r, if dir { Icon::Folder } else { Icon::File }, t.accent);
            ui.vertical(|ui| {
                let title = match ctx.as_slice() {
                    [] => "Nothing selected".to_string(),
                    [one] => name_of(one),
                    many => format!("{} items", many.len()),
                };
                ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate());
                let n = ctx.len();
                let sub = match (&facts, n) {
                    (_, 0) => "Select a file or folder to ask about it".to_string(),
                    (Some(f), _) => f.clone(),
                    (None, 1) => String::new(),
                    (None, n) => format!("{n} items in context"),
                };
                ui.label(egui::RichText::new(sub).size(11.5).color(t.weak)).on_hover_text(ctx.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !fresh && kit::icon_button(ui, Icon::Close, "New chat: forget this conversation").clicked() {
                    new_chat = true;
                }
            });
        });
        if ctx.len() > 1 {
            // Exactly what the conversation covers.
            egui::CollapsingHeader::new(egui::RichText::new(format!("Included ({})", ctx.len())).size(12.0)).id_salt("files-ask-included").show(ui, |ui| {
                for p in ctx.iter().take(40) {
                    let dir = p.is_dir();
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                        crate::kit::icons::paint(ui.painter(), r, if dir { Icon::Folder } else { Icon::File }, t.weak);
                        ui.add(egui::Label::new(egui::RichText::new(name_of(p)).size(12.0)).truncate()).on_hover_text(p.display().to_string());
                    });
                }
                if ctx.len() > 40 {
                    ui.label(egui::RichText::new(format!("and {} more", ctx.len() - 40)).size(12.0).color(t.weak));
                }
            });
        }
        ui.add_space(4.0);

        // A different item selected during a conversation.
        let mut choice = None;
        if !changed.is_empty() {
            let what = match changed.as_slice() {
                [one] => name_of(one),
                many => format!("{} items", many.len()),
            };
            egui::Frame::new().fill(t.accent_soft).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.add(egui::Label::new(format!("Now selected: {what}")).truncate());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if ui.small_button("New chat").on_hover_text("Start again, about this item").clicked() {
                        choice = Some(0);
                    }
                    if ui.small_button("Replace").on_hover_text("Keep the conversation; ask about this item from now on").clicked() {
                        choice = Some(1);
                    }
                    if ui.small_button("Add").on_hover_text("Ask about this item too, with what's already here").clicked() {
                        choice = Some(2);
                    }
                    if ui.small_button("Keep this chat").on_hover_text("Leave the conversation as it is").clicked() {
                        choice = Some(3);
                    }
                });
            });
            ui.add_space(4.0);
        }

        // The model.
        let loaded = self.ask.loaded.as_ref().map(|(_, n, _)| n.clone());
        let mut load = None;
        egui::Frame::new().fill(pal.chrome).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| match (&self.ask.phase, &loaded) {
                (Phase::Loading, _) => {
                    ui.spinner();
                    ui.label(format!("Loading {}…", self.ask.picked.as_ref().map_or("the model", |(_, n)| n.as_str())));
                }
                (_, Some(n)) => {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, t.success);
                    ui.add(egui::Label::new(n.as_str()).truncate()).on_hover_text("Loaded, and shared with Ask AI in the AI Models workspace");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.add_enabled(self.ask.phase == Phase::Idle, egui::Button::new("Change").small()).on_hover_text("Load another model");
                        self.model_menu(&r, &mut load);
                    });
                }
                (_, None) => {
                    let models = self.models.list.as_ref().map(|l| ask::runnable(l).into_iter().map(|(_, m)| m.clone()).collect::<Vec<_>>());
                    match models {
                        None => {
                            ui.spinner();
                            ui.weak("Looking for models…");
                        }
                        Some(v) if v.is_empty() => {
                            ui.weak("No model yet");
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Get a small model…").on_hover_text("See what Filescape would download, and how big it is, before anything is fetched").clicked() {
                                    self.ask.offer_starter();
                                }
                            });
                        }
                        Some(v) => {
                            // The picked model, or the smallest.
                            let pick = self.ask.picked.as_ref().and_then(|(p, _)| v.iter().find(|m| m.files.first().is_some_and(|f| &f.path == p))).unwrap_or(&v[0]).clone();
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                            ui.painter().circle_stroke(r.center(), 4.0, egui::Stroke::new(1.2, t.weak));
                            ui.add(egui::Label::new(egui::RichText::new(&pick.name).color(t.weak)).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let r = ui.small_button("⏷").on_hover_text("Pick another model");
                                self.model_menu(&r, &mut load);
                                if ui.small_button("Load").on_hover_text(format!("Read {} into memory ({})", pick.name, crate::scan::human(pick.size))).clicked() {
                                    load = Some(pick.clone());
                                }
                            });
                        }
                    }
                }
            });
        });
        if let Some(p) = self.ask.problem.clone() {
            ui.add_space(4.0);
            kit::callout(ui, Tone::Warn, p);
        }
        ui.add_space(6.0);

        // The conversation, then the question box and the disclaimer at the bottom.
        let ready = loaded.is_some();
        // A question picked from the right-click menu goes as soon as the model has what it needs.
        if self.ask.side_send && ready && self.ask.phase == Phase::Idle && self.ask.side.about.as_ref().is_some_and(|a| a.facts.is_some()) {
            self.ask.side_send = false;
            self.send_question(Which::Side);
        }
        let here = self.ask.asker == Which::Side;
        let busy_elsewhere = self.ask.phase == Phase::Answering && !here;
        let answering = self.ask.phase == Phase::Answering && here;
        // The question box and footer's height as last drawn: they're drawn after the conversation.
        let bottom_id = egui::Id::new("files-ask-bottom");
        let footer_h = ui.data(|d| d.get_temp::<f32>(bottom_id)).unwrap_or(150.0) + 6.0;
        let avail = ui.available_height();
        let mut ask_now: Option<(String, bool)> = None;
        let mut reveal: Option<PathBuf> = None;
        egui::ScrollArea::vertical().id_salt("files-ask-chat").auto_shrink(false).stick_to_bottom(true).max_height((avail - footer_h).max(80.0)).show(ui, |ui| {
            let c = &self.ask.side;
            if c.conversation.is_empty() && c.answering.is_none() {
                if let Some(p) = ctx.first() {
                    let is_dir = ctx.len() == 1 && p.is_dir();
                    ui.label(match (ctx.len(), is_dir) {
                        (1, true) => "What would you like to know about this folder?",
                        (1, false) => "What would you like to know about this file?",
                        _ => "What would you like to know about these?",
                    });
                    ui.add_space(4.0);
                    let sugg = if ctx.len() == 1 { asksuggest::for_path(p, is_dir) } else { asksuggest::for_many(ctx.iter().all(|p| !p.is_dir())) };
                    for s in sugg {
                        let w = ui.available_width();
                        let (rect, r) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                        let fill = if r.hovered() && ready { t.raised } else { pal.chrome };
                        ui.painter().rect(rect, 6.0, fill, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
                        let ic = Rect::from_center_size(rect.left_center() + vec2(16.0, 0.0), vec2(14.0, 14.0));
                        crate::kit::icons::paint(ui.painter(), ic, mark_icon(s.mark), if ready { t.accent } else { t.weak });
                        let col = if ready { t.text } else { t.weak };
                        ui.painter().with_clip_rect(rect.shrink(2.0)).text(rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, s.label, FontId::proportional(13.0), col);
                        let r = r.on_hover_text(if ready { s.question } else { "Load a model first" });
                        if r.clicked() && ready && self.ask.phase == Phase::Idle {
                            ask_now = Some((s.question.to_string(), s.read));
                        }
                        ui.add_space(4.0);
                    }
                    if ctx.len() == 1 && asksuggest::sort_of(&name_of(p), is_dir) == asksuggest::Sort::Image {
                        let note = match (self.ask.vision, crate::askabout::can_show(p)) {
                            (true, true) => None,
                            (true, false) => Some("The model can see pictures, but Filescape can't show it this kind of file. Questions use its name and details."),
                            (false, _) => Some("To ask about what's in a picture, load a model that can see images: one with a vision projector (mmproj) file beside it. Until then, questions use its name and details."),
                        };
                        if let Some(note) = note {
                            ui.add(egui::Label::new(egui::RichText::new(note).size(11.5).color(t.weak)).wrap());
                        }
                    }
                } else {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.weak("Select a file or folder, then ask about it here."));
                }
            }
            let model = loaded.clone().unwrap_or_else(|| "Model".into());
            for (i, m) in c.conversation.iter().enumerate() {
                let about = c.attached.get(&i).map(|a| (a.label.as_str(), a.text.as_str()));
                ask::message(ui, i, m.role, &m.text, &model, false, about);
                if m.role == Role::Assistant
                    && let Some(a) = (0..i).rev().find_map(|q| c.attached.get(&q))
                    && !a.paths.is_empty()
                {
                    ask::sources(ui, i, a, &m.text, &mut reveal);
                }
                ui.add_space(18.0);
            }
            if here && let Some(a) = &c.answering {
                ask::message(ui, c.conversation.len(), Role::Assistant, a, &model, true, None);
            }
            if here && c.reading.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Reading the file…");
                });
            }
        });

        // The question box.
        let bottom_top = ui.cursor().top();
        ui.add_space(4.0);
        if busy_elsewhere {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("The model is answering in AI Models…");
            });
        }
        let vision = self.ask.vision;
        let about = self.ask.side.about.as_ref().and_then(|a| a.facts.as_ref().map(|f| (f.readable.len(), if vision { f.paths.iter().filter(|p| crate::askabout::can_show(p)).count() } else { 0 }, a.read)));
        if let Some((n, pics, mut read)) = about
            && n + pics > 0
        {
            let (what, tip) = match (n, pics) {
                (0, 1) => ("Show the picture to the model", "Off: the model sees only its name, size and dates.\nOn: the model looks at the picture for this question, on this computer."),
                (0, _) => ("Show the pictures to the model", "Off: the model sees only names, sizes and dates.\nOn: the model looks at the pictures for this question, on this computer."),
                (1, 0) => ("Include the file's contents", "Off: the model sees only names, sizes and dates.\nOn: Filescape reads the file's text for this question. Files only in the cloud aren't read."),
                _ => ("Include the files' contents", "Off: the model sees only names, sizes and dates.\nOn: Filescape reads the files' text (and shows pictures to a model that can see) for this question. Files only in the cloud aren't read."),
            };
            if ui.checkbox(&mut read, egui::RichText::new(what).size(12.0)).on_hover_text(tip).changed()
                && let Some(a) = &mut self.ask.side.about
            {
                a.read = read;
            }
        }
        let mut send = false;
        let mut stop = false;
        let mut expand = false;
        let sort = ctx.first().map(|p| asksuggest::sort_of(&name_of(p), ctx.len() == 1 && p.is_dir()));
        egui::Frame::new().fill(pal.chrome).corner_radius(8.0).stroke(egui::Stroke::new(1.0, t.card_border)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            let hint = match (ready, sort) {
                (false, _) => "Load a model to ask a question",
                (true, Some(_)) if ctx.len() > 1 => "Ask anything about these items…",
                (true, Some(s)) => asksuggest::hint(s),
                (true, None) => "Ask a question…",
            };
            let r = ui.add_enabled(
                ready,
                egui::TextEdit::multiline(&mut self.ask.side.draft)
                    .hint_text(hint)
                    .desired_rows(2)
                    .desired_width(ui.available_width())
                    .frame(egui::Frame::NONE)
                    .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))),
            );
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) {
                send = true;
            }
            ui.horizontal(|ui| {
                let n = self.ask.side_ctx.len();
                if n > 0 {
                    ui.label(egui::RichText::new(format!("{n} {} in context", if n == 1 { "item" } else { "items" })).size(11.5).color(t.weak));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if answering {
                        stop = ui.button("■ Stop").on_hover_text("Stop the answer here").clicked();
                    } else if ui.add_enabled(ready && !self.ask.side.draft.trim().is_empty() && self.ask.phase == Phase::Idle, egui::Button::new("Send")).clicked() {
                        send = true;
                    }
                    let r = kit::icon_button(ui, Icon::More, "More");
                    egui::Popup::menu(&r).show(|ui| {
                        if ui.add_enabled(!fresh && !answering, egui::Button::new("New chat")).clicked() {
                            new_chat = true;
                        }
                        if ui.button("Open in AI Models").on_hover_text("Carry on this conversation in the AI Models workspace, with more room. It replaces the conversation there.").clicked() {
                            expand = true;
                        }
                    });
                });
            });
        });
        ui.add_space(6.0);
        ai_footer(ui);
        let h = ui.cursor().top() - bottom_top;
        ui.data_mut(|d| d.insert_temp(bottom_id, h));

        if let Some((q, read)) = ask_now {
            self.ask.side.draft = q;
            if let Some(a) = &mut self.ask.side.about {
                a.read = read && a.facts.as_ref().is_some_and(|f| !f.readable.is_empty() || (vision && f.paths.iter().any(|p| crate::askabout::can_show(p))));
            }
            send = true;
        }
        if stop {
            self.stop_answer(Which::Side);
        }
        if send {
            // Follow-ups after the first question keep the context the conversation already has.
            self.send_question(Which::Side);
        }
        if let Some(m) = load {
            self.load_model(&m);
        }
        match choice {
            Some(0) => {
                self.ask.side.clear();
                self.side_about(changed);
            }
            Some(1) => self.side_about(changed),
            Some(2) => {
                // The new items go with the next question; the chat is about all of them.
                let mut all = self.ask.side_ctx.clone();
                let new: Vec<PathBuf> = changed.into_iter().filter(|p| !all.contains(p)).collect();
                self.side_about(new.clone());
                all.extend(new);
                self.ask.side_ctx = all;
            }
            Some(_) => self.ask.side_skip = changed.first().cloned(),
            None => {}
        }
        if new_chat && !answering {
            self.ask.side.clear();
            self.ask.side.about = None;
            self.ask.side_ctx.clear();
            self.ask.side_send = false;
            if !selection.is_empty() {
                self.side_about(selection);
            }
        }
        if expand {
            self.ask.main = std::mem::take(&mut self.ask.side);
            if self.ask.asker == Which::Side {
                self.ask.asker = Which::Main;
            }
            self.ask.side_ctx.clear();
            self.ask.open = true;
            self.ask.lab = false;
            self.set_workspace(Workspace::Models);
        }
        if let Some(p) = reveal {
            self.show_model_file(&p);
        }
    }

    /// The models Ask AI can load, as a menu under `r`.
    fn model_menu(&mut self, r: &egui::Response, load: &mut Option<crate::models::Model>) {
        let models = self.models.list.as_ref().map(|l| ask::runnable(l).into_iter().map(|(_, m)| m.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let loaded = self.ask.loaded.as_ref().map(|(p, _, _)| p.clone());
        egui::Popup::menu(r).show(|ui| {
            for m in &models {
                let is = m.files.first().is_some_and(|f| Some(&f.path) == loaded.as_ref());
                if ui.add(egui::Button::selectable(is, format!("{} · {}", m.name, crate::scan::human(m.size)))).clicked() && !is {
                    *load = Some(m.clone());
                }
            }
            ui.separator();
            if ui.button("Get a small model…").clicked() {
                self.ask.offer_starter();
            }
        });
    }
}
