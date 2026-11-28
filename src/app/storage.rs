//! Storage, in the AI Models workspace: how much space models take, by app, and what can be freed
//! (see `crate::cleanup`). Also the one confirmation every removal goes through, from here or
//! from the library's "Move to Trash…".
//!
//! Removing one thing at a time is free; removing several at once is part of managing models in
//! bulk (Professional), checked once when it starts.

use super::*;
use crate::cleanup::{self, Item, Kind, Removed};
use crate::fsys::PathShow;
use crate::kit::{self, Tone};
use crate::models::{Model, Source};
use std::collections::BTreeSet;

mod moving;
mod shrink;

#[derive(Default)]
pub(super) struct StorageState {
    /// The Storage view is on screen instead of the library.
    pub open: bool,
    rx: Option<mpsc::Receiver<Vec<Item>>>,
    /// What can be freed, for the current model list (`None` until looked for).
    items: Option<Vec<Item>>,
    picked: BTreeSet<usize>,
    /// Waiting for the one confirmation.
    confirm: Option<Confirm>,
    removing: Option<mpsc::Receiver<Vec<(Item, Vec<Removed>)>>>,
    pub outcome: Option<(Tone, String)>,
    /// Moving an app's folder to another drive.
    moving: Option<moving::Moving>,
    /// Shrinking one model into a new file.
    shrink: Option<shrink::Shrink>,
}

struct Confirm {
    items: Vec<Item>,
    trash: bool,
    /// Share copies as one file instead of removing them.
    share: bool,
}

impl StorageState {
    /// The model list changed: look again when next shown.
    pub fn forget(&mut self) {
        self.rx = None;
        self.items = None;
        self.picked.clear();
    }
}

fn app_order(app: &str) -> u8 {
    match app {
        "Ollama" => 0,
        "LM Studio" => 1,
        "Hugging Face" => 2,
        _ => 3,
    }
}

impl App {
    /// Starts looking for what can be freed, once the model list is in.
    fn storage_look(&mut self) {
        let Some(list) = self.models.list.clone() else { return };
        let places: Vec<PathBuf> = self.models.places.iter().map(|(_, p)| p.clone()).collect();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(cleanup::find(&places, &list));
        });
        self.models.storage.rx = Some(rx);
    }

    pub(super) fn poll_storage(&mut self, ctx: &egui::Context) {
        self.poll_moving(ctx);
        let s = &mut self.models.storage;
        if let Some(rx) = &s.rx {
            match rx.try_recv() {
                Ok(items) => {
                    s.items = Some(items);
                    s.rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => s.rx = None,
            }
        }
        if let Some(rx) = &s.removing {
            match rx.try_recv() {
                Ok(done) => {
                    s.removing = None;
                    s.outcome = Some(outcome(&done));
                    // Look again: the list and what can be freed have changed.
                    self.start_models();
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => s.removing = None,
            }
        }
    }

    /// Asks to remove one model from the library: what that means is worked out first.
    pub(super) fn ask_remove_model(&mut self, m: &Model) {
        let places: Vec<PathBuf> = self.models.places.iter().map(|(_, p)| p.clone()).collect();
        let list = self.models.list.clone().unwrap_or_default();
        let plan = cleanup::plan(m).and_then(|item| {
            let who = cleanup::linked_from(&item, &places, &list);
            if who.is_empty() { Ok(item) } else { Err(format!("{} links to its files and would stop working. Remove that first.", who.join(", "))) }
        });
        match plan {
            Ok(item) => self.models.storage.confirm = Some(Confirm { items: vec![item], trash: true, share: false }),
            Err(e) => self.models.storage.outcome = Some((Tone::Warn, format!("{} can't be removed: {e}", m.name))),
        }
    }

    /// Asks to remove these (or share them as one file), after the Professional check for more
    /// than one.
    fn ask_remove(&mut self, items: Vec<Item>, share: bool) {
        let items: Vec<Item> = items.into_iter().filter(|i| if share { i.can_share } else { i.can_trash }).collect();
        if items.is_empty() || (items.len() > 1 && !self.premium(crate::licence::Feature::ModelAutomation)) {
            return;
        }
        self.models.storage.confirm = Some(Confirm { items, trash: true, share });
    }

    /// The Storage view.
    pub(super) fn storage_view(&mut self, ui: &mut egui::Ui) {
        let Some(list) = self.models.list.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Looking for AI models…");
            });
            return;
        };
        if self.models.storage.items.is_none() && self.models.storage.rx.is_none() {
            self.storage_look();
        }
        self.storage_header(ui, &list);
        ui.add_space(8.0);
        if let Some((tone, text)) = self.models.storage.outcome.clone() {
            kit::callout(ui, tone, text);
            if ui.small_button("Hide").clicked() {
                self.models.storage.outcome = None;
            }
            ui.add_space(6.0);
        }
        by_app(ui, &list);
        self.storage_places(ui, &list);
        ui.add_space(12.0);
        let busy = self.models.storage.removing.is_some();
        let Some(items) = self.models.storage.items.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Looking for space to free…");
            });
            return;
        };
        let total: u64 = items.iter().map(|i| i.bytes).sum();
        if items.is_empty() {
            ui.strong("Nothing to free");
            ui.weak("No unused or unfinished downloads, older versions or loose copies were found where the library looked.");
            return;
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} CAN BE FREED", scan::human(total))).size(11.0).strong().color(kit::tokens(ui).weak));
            if busy {
                ui.spinner();
                ui.label("Removing…");
            }
        });
        ui.weak("Nothing here is used by a model Filescape can see. Each item says how sure that is and why, and lists what it removes before anything happens.");
        ui.add_space(4.0);
        let mut remove_one = None;
        let mut share_one = None;
        egui::ScrollArea::vertical().id_salt("storage-items").max_height((ui.available_height() - 40.0).max(120.0)).auto_shrink([false, true]).show(ui, |ui| {
            let mut last = None;
            for (i, it) in items.iter().enumerate() {
                if last != Some(it.kind) {
                    ui.add_space(8.0);
                    let n = items.iter().filter(|x| x.kind == it.kind).count();
                    let bytes: u64 = items.iter().filter(|x| x.kind == it.kind).map(|x| x.bytes).sum();
                    ui.horizontal(|ui| {
                        ui.strong(it.kind.heading());
                        ui.weak(format!("· {n} · {}", scan::human(bytes)));
                    });
                    ui.weak(kind_words(it.kind));
                    ui.add_space(2.0);
                    last = Some(it.kind);
                }
                ui.horizontal(|ui| {
                    let mut on = self.models.storage.picked.contains(&i);
                    if ui.add_enabled(!busy, egui::Checkbox::without_text(&mut on)).changed() {
                        if on {
                            self.models.storage.picked.insert(i);
                        } else {
                            self.models.storage.picked.remove(&i);
                        }
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&it.title).strong());
                            ui.weak(format!("· {} · {}", it.app, scan::human(it.bytes)));
                            sure_badge(ui, it);
                        });
                        ui.add(egui::Label::new(egui::RichText::new(&it.detail).small().weak()).wrap());
                        if !it.reasons.is_empty() {
                            egui::CollapsingHeader::new(egui::RichText::new("Why").small()).id_salt(("why", i)).show(ui, |ui| reasons(ui, it));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if it.can_trash && ui.add_enabled(!busy, egui::Button::new(format!("{}…", bin_verb()))).on_hover_text("See what this removes, then confirm").clicked() {
                            remove_one = Some(i);
                        }
                        if it.can_share
                            && ui
                                .add_enabled(!busy, egui::Button::new("Share as one file…"))
                                .on_hover_text("Keep both names but one file on disk (a hard link). Every app keeps working; the space is freed at once.")
                                .clicked()
                        {
                            share_one = Some(i);
                        }
                    });
                })
                .response
                .on_hover_text(it.paths.first().map(|p| p.shown()).unwrap_or_default());
                ui.add_space(3.0);
            }
        });
        ui.separator();
        let picked: Vec<usize> = self.models.storage.picked.iter().copied().filter(|&i| i < items.len()).collect();
        let mut remove_picked = false;
        let mut share_picked = false;
        ui.horizontal(|ui| {
            let trash: Vec<usize> = picked.iter().copied().filter(|&i| items[i].can_trash).collect();
            let bytes: u64 = trash.iter().map(|&i| items[i].bytes).sum();
            let label = if trash.len() > 1 { format!("Remove {} items ({})…", trash.len(), scan::human(bytes)) } else { "Remove selected…".into() };
            let r = ui.add_enabled(!busy && !trash.is_empty(), egui::Button::new(label));
            remove_picked = r.on_hover_text("Several at once is part of managing models in bulk (Professional). One at a time is free.").clicked();
            let shared: Vec<usize> = picked.iter().copied().filter(|&i| items[i].can_share).collect();
            if !shared.is_empty() {
                let label = if shared.len() > 1 { format!("Share {} as one file…", shared.len()) } else { "Share selected as one file…".into() };
                share_picked = ui.add_enabled(!busy, egui::Button::new(label)).clicked();
            }
            if !self.licence.allows(crate::licence::Feature::ModelAutomation) {
                ui.weak("Several at once: Professional");
            }
            if ui.add_enabled(!busy, egui::Button::new("Select all")).clicked() {
                self.models.storage.picked = (0..items.len()).collect();
            }
            if !picked.is_empty() && ui.add_enabled(!busy, egui::Button::new("Select none")).clicked() {
                self.models.storage.picked.clear();
            }
        });
        if let Some(i) = remove_one {
            self.ask_remove(vec![items[i].clone()], false);
        } else if let Some(i) = share_one {
            self.ask_remove(vec![items[i].clone()], true);
        } else if remove_picked || share_picked {
            self.ask_remove(picked.iter().map(|&i| items[i].clone()).collect(), share_picked);
        }
    }

    /// The one confirmation, listing everything that goes.
    pub(super) fn storage_confirm(&mut self, ctx: &egui::Context) {
        let Some(c) = &mut self.models.storage.confirm else { return };
        let mut go = false;
        let mut cancel = false;
        let bytes: u64 = c.items.iter().map(|i| i.bytes).sum();
        let paths: usize = c.items.iter().map(|i| i.paths.len()).sum();
        egui::Modal::new(egui::Id::new("storage-confirm")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            let what = match &c.items[..] {
                [one] => one.title.clone(),
                many => format!("{} items", many.len()),
            };
            ui.heading(if c.share { format!("Share {what} as one file?") } else { format!("Remove {what}?") });
            ui.add_space(4.0);
            if c.share {
                ui.label(format!("This frees {}. Each copy becomes a second name of the file it's the same as, so every app keeps finding its model, with one file on disk. Each pair is compared once more first.", scan::human(bytes)));
            } else {
                ui.label(format!("This frees {}, removing {}:", scan::human(bytes), if paths == 1 { "this".to_string() } else { format!("these {paths}") }));
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                for it in &c.items {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{} · {}", it.title, scan::human(it.bytes))).strong());
                        sure_badge(ui, it);
                    });
                    ui.add(egui::Label::new(egui::RichText::new(&it.detail).small().weak()).wrap());
                    if it.sure == cleanup::Sure::Likely {
                        reasons(ui, it);
                    }
                    for (k, p) in it.paths.iter().enumerate() {
                        let line = match it.keep.get(k).filter(|_| c.share) {
                            Some(keep) => format!("{}\n    becomes the same file as {}", p.shown(), keep.shown()),
                            None => p.shown(),
                        };
                        ui.add(egui::Label::new(egui::RichText::new(line).small().monospace()).wrap());
                    }
                    ui.add_space(4.0);
                }
            });
            ui.add_space(6.0);
            if c.share {
                ui.weak("Removing either name later keeps the other. A program that changes one in place would change both; model files aren't changed that way.");
            } else {
                ui.radio_value(&mut c.trash, true, format!("{} (the space comes back when it's emptied)", bin_verb()));
                ui.radio_value(&mut c.trash, false, "Delete for good (the space comes back now)");
            }
            if c.items.iter().any(|i| i.app == "Ollama") {
                ui.weak("If Ollama is downloading a model right now, let it finish first.");
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let verb = if c.share { "Share as one file" } else if c.trash { bin_verb() } else { "Delete" };
                go = kit::button(ui, if c.trash || c.share { kit::Kind::Primary } else { kit::Kind::Danger }, None, verb).clicked();
                cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if cancel || go {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
        if cancel {
            self.models.storage.confirm = None;
        } else if go {
            let Confirm { items, trash, share } = self.models.storage.confirm.take().unwrap();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let done = items.into_iter().map(|it| {
                    let r = if share { cleanup::share(&it) } else { cleanup::remove(&it, trash) };
                    (it, r)
                });
                let _ = tx.send(done.collect());
            });
            self.models.storage.removing = Some(rx);
            self.models.storage.picked.clear();
        }
    }
}

/// "Safe to remove" or "Probably unused", with what it rests on when hovered.
fn sure_badge(ui: &mut egui::Ui, it: &Item) {
    let tone = match it.sure {
        cleanup::Sure::Certain => Tone::Success,
        cleanup::Sure::Likely => Tone::Warn,
    };
    // A copy an app keeps can't be removed, only shared: say that, not "remove".
    let text = if it.kind == Kind::Copy && !it.can_trash { "Safe to share" } else { it.sure.label() };
    let r = kit::badge(ui, text, tone);
    if !it.reasons.is_empty() {
        r.on_hover_text(it.reasons.iter().map(|r| format!("• {r}")).collect::<Vec<_>>().join("\n"));
    }
}

fn reasons(ui: &mut egui::Ui, it: &Item) {
    for r in &it.reasons {
        ui.add(egui::Label::new(egui::RichText::new(format!("• {r}")).small()).wrap());
    }
}

fn s(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn bin_verb() -> &'static str {
    if cfg!(windows) { "Move to Recycle Bin" } else { "Move to Trash" }
}

/// What removals came to, in a sentence.
fn outcome(done: &[(Item, Vec<Removed>)]) -> (Tone, String) {
    let failed: Vec<&Removed> = done.iter().flat_map(|(_, r)| r.iter()).filter(|r| r.result.is_err()).collect();
    let freed: u64 = done.iter().filter(|(_, r)| r.iter().all(|r| r.result.is_ok())).map(|(i, _)| i.bytes).sum();
    let names = match done {
        [(one, _)] => one.title.clone(),
        many => format!("{} items", many.len()),
    };
    if failed.is_empty() {
        (Tone::Success, format!("Done: {names}, {} freed.", scan::human(freed)))
    } else {
        let first = &failed[0];
        let why = first.result.as_ref().err().cloned().unwrap_or_default();
        (Tone::Warn, format!("{} of {} couldn't be removed. {}: {why}", failed.len(), done.iter().map(|(_, r)| r.len()).sum::<usize>(), first.path.shown()))
    }
}

fn kind_words(k: Kind) -> &'static str {
    match k {
        Kind::Unused => "Left behind by removed models or stopped downloads. No model needs them.",
        Kind::Partial => "Downloads that stopped part way. Starting the download again starts afresh.",
        Kind::OldVersion => "Hugging Face keeps each version a program downloaded. Programs use the current one.",
        Kind::Copy => "Files that are byte-for-byte the same as a model kept elsewhere. Share them as one file so every app keeps working, or remove a loose copy.",
        Kind::Model => "",
    }
}

/// Space by app, as bars.
impl App {
    /// The title row and the totals strip, styled like the Library's.
    fn storage_header(&mut self, ui: &mut egui::Ui, list: &[Model]) {
        let t = kit::tokens(ui);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            crate::kit::icons::paint(ui.painter(), r, crate::kit::icons::Icon::Layers, t.accent);
            ui.label(egui::RichText::new("Storage").size(18.0).strong());
            ui.weak("· how much space AI models take, and what can be freed");
        });
        ui.add_space(6.0);
        let fp = self.models.footprint;
        let apps = {
            let mut a: Vec<&str> = list.iter().map(|m| m.source.label()).collect();
            a.sort();
            a.dedup();
            a.len()
        };
        let freeable = self.models.storage.items.as_ref().map(|i| i.iter().map(|x| x.bytes).sum::<u64>());
        kit::card_frame(&t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let tip = if fp.shared > 0 {
                    format!("Space the model files take on the drive, counting files shared as one (hard links) once. Their sizes add up to {}.", scan::human(fp.logical))
                } else {
                    "Space the model files take on the drive.".to_string()
                };
                super::summary::stat(ui, "On disk", &scan::human(fp.on_disk), &tip, None);
                super::summary::divider(ui);
                match freeable {
                    Some(b) => super::summary::stat(ui, "Can be freed", &scan::human(b), "Unused and unfinished downloads, older versions and loose copies listed below.", (b > 0).then_some(t.success)),
                    None => super::summary::stat(ui, "Can be freed", "…", "Still looking.", None),
                };
                super::summary::divider(ui);
                super::summary::stat(ui, "Apps", &apps.to_string(), "Apps and folders that keep models.", None);
            });
        });
    }
}

fn by_app(ui: &mut egui::Ui, list: &[Model]) {
    let mut apps: Vec<(&'static str, u64, usize)> = Vec::new();
    for m in list {
        let app = match m.source {
            Source::Folder => "Other folders",
            s => s.label(),
        };
        match apps.iter_mut().find(|a| a.0 == app) {
            Some(a) => {
                a.1 += m.size;
                a.2 += usize::from(!m.orphan);
            }
            None => apps.push((app, m.size, usize::from(!m.orphan))),
        }
    }
    apps.sort_by_key(|a| (app_order(a.0), std::cmp::Reverse(a.1)));
    let total: u64 = apps.iter().map(|a| a.1).sum::<u64>().max(1);
    let t = kit::tokens(ui);
    egui::Grid::new("storage-apps").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
        for (app, bytes, n) in &apps {
            ui.label(*app);
            let (rect, _) = ui.allocate_exact_size(vec2(220.0, 12.0), Sense::hover());
            ui.painter().rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
            ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(rect.width() * (*bytes as f32 / total as f32), rect.height())), 3.0, t.accent);
            ui.label(scan::human(*bytes));
            ui.weak(format!("{n} model{}", s(*n)));
            ui.end_row();
        }
    });
}
