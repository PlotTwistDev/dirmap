//! The Custom filter: its input row and dropdown, autocomplete, recent filters and presets, the
//! background job that counts matches or builds the "Show matches only" tree, and the optional
//! sequence finder.

use super::*;
use crate::pattern::{self, Filter};
use crate::sequences::{self, Sequence};
use crate::subset::{self, Back};
use egui::containers::menu::{MenuButton, MenuConfig};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// How long typing must pause before the filter is applied to the maps.
const DEBOUNCE: Duration = Duration::from_millis(220);
const MAX_RECENT: usize = 12;

const EXAMPLES: [(&str, &str); 9] = [
    ("*.exr", "all EXR files"),
    ("*.exr, *.tx, *.tif", "any of several types"),
    ("*.exr, !*preview*", "EXR files, leaving out previews"),
    ("render_*", "names starting with render_"),
    ("*diffuse*", "names containing diffuse"),
    ("shot_####.exr", "four-digit frame numbers"),
    ("render.{frame}.exr", "frame numbers of any length"),
    ("render.{frame:1001-1200:04}.exr", "frames 1001–1200, four digits"),
    ("shot_[1001-1100].exr", "frames 1001–1100, any padding"),
];

/// Highlight keeps the layout and dims what doesn't match; Only rebuilds the maps from the
/// matches alone.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub(super) enum FilterMode {
    #[default]
    Highlight,
    Only,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Preset {
    pub name: String,
    pub pattern: String,
}

/// Remembered between runs, separately from the Settings window's values.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct CustomPrefs {
    pub recent: Vec<String>,
    pub presets: Vec<Preset>,
    pub case_sensitive: bool,
    pub match_all: bool,
    pub mode: FilterMode,
}

#[derive(Default)]
pub(super) struct CustomUi {
    /// The input row is showing.
    pub open: bool,
    pub text: String,
    /// When the text last changed and hasn't been applied yet.
    pub edited: Option<Instant>,
    pub error: Option<pattern::Error>,
    pub prefs: CustomPrefs,
    /// Focus the input next frame.
    pub focus: bool,
    /// Highlighted suggestion in the autocomplete list.
    pub pick: usize,
    pub preset_name: String,
}

impl CustomUi {
    pub fn options(&self) -> pattern::Options {
        pattern::Options { case_sensitive: self.prefs.case_sensitive, match_all: self.prefs.match_all }
    }
}

/// The filtered tree on screen in "Show matches only".
pub(super) struct Shown {
    pub filter: ViewFilter,
    pub tree: Arc<Node>,
    pub back: Arc<Back>,
    pub exts: Vec<ExtStat>,
}

/// Counting matches or building the filtered tree, on a background thread.
pub(super) struct FilterJob {
    filter: ViewFilter,
    only: bool,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<FilterDone>,
}

enum FilterDone {
    Count(u64, u64),
    Subset(Node, Back, Vec<ExtStat>),
}

#[derive(Default)]
pub(super) struct SeqState {
    job: Option<(Arc<AtomicBool>, mpsc::Receiver<Vec<Sequence>>)>,
    pub list: Option<Vec<Sequence>>,
    pub min_frames: usize,
}

impl App {
    /// The tree the maps and the folder list show: the matches only, or the whole scan.
    pub(super) fn shown_tree(&self) -> Option<Arc<Node>> {
        match &self.shown {
            Some(s) => Some(s.tree.clone()),
            None => self.tree.clone(),
        }
    }

    pub(super) fn shown_back(&self) -> Option<Arc<Back>> {
        self.shown.as_ref().map(|s| s.back.clone())
    }

    /// A path in the shown tree as a path in the whole scan (for duplicates and changes).
    pub(super) fn full_of(&self, view: &[u32]) -> Vec<u32> {
        match &self.shown {
            Some(s) => s.back.to_full(view),
            None => view.to_vec(),
        }
    }

    /// Select something given by its path in the whole scan.
    pub(super) fn select_full(&mut self, full: Vec<u32>) {
        let p = match &self.shown {
            Some(s) => s.back.to_view(&full).0,
            None => full,
        };
        self.select(p);
    }

    /// Swap the tree on screen, carrying the zoom, selection and open folders across.
    fn set_shown(&mut self, new: Option<Shown>) {
        let old_back = self.shown_back();
        let new_back = new.as_ref().map(|s| s.back.clone());
        let to_full = |p: &[u32]| old_back.as_ref().map_or_else(|| p.to_vec(), |b| b.to_full(p));
        let to_view = |full: Vec<u32>| match &new_back {
            Some(b) => b.to_view(&full),
            None => (full, true),
        };
        self.zoom = to_view(to_full(&self.zoom)).0;
        self.selected = self.selected.take().and_then(|s| {
            let (p, exact) = to_view(to_full(&s));
            exact.then_some(p)
        });
        let mut expanded: HashSet<Box<[u32]>> = self
            .expanded
            .iter()
            .filter_map(|e| {
                let (p, exact) = to_view(to_full(e));
                exact.then(|| p.into())
            })
            .collect();
        expanded.insert(Box::new([]));
        self.expanded = expanded;
        self.rows = None;
        self.sun = None;
        self.hist = None;
        self.paint_gen += 1;
        if let Some(old) = std::mem::replace(&mut self.shown, new) {
            std::thread::spawn(move || drop(old));
        }
    }

    /// Go back to showing the whole scan and forget results computed from the current tree,
    /// before the tree itself changes. The filter stays and is reapplied to the new tree.
    pub(super) fn drop_filtered_view(&mut self) {
        self.set_shown(None);
        if let Some(j) = self.filter_job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.filter_stats = None;
        if let Some((c, _)) = self.seq.job.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.seq.list = None;
    }

    /// Forget everything tied to the previous scan.
    pub(super) fn reset_custom_for_new_tree(&mut self) {
        if let Some(j) = self.filter_job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(old) = self.shown.take() {
            std::thread::spawn(move || drop(old));
        }
        if let Some((c, _)) = self.seq.job.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.seq.list = None;
    }

    /// Each frame: apply typed text after a pause, then keep the match count and (in "Show
    /// matches only") the filtered tree up to date in the background. A newer filter cancels
    /// the work for an older one.
    pub(super) fn drive_filter(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.custom.edited {
            let left = DEBOUNCE.saturating_sub(t.elapsed());
            if left.is_zero() {
                self.apply_custom();
            } else {
                ctx.request_repaint_after(left);
            }
        }
        let Some(tree) = self.tree.clone() else { return };
        let active = self.filter.active();
        let only = active && self.custom.prefs.mode == FilterMode::Only;

        // Collect a finished job.
        if let Some(job) = &self.filter_job {
            match job.rx.try_recv() {
                Ok(done) => {
                    let job = self.filter_job.take().unwrap();
                    match done {
                        FilterDone::Count(files, bytes) => self.filter_stats = Some((job.filter, files, bytes)),
                        FilterDone::Subset(node, back, exts) => {
                            self.filter_stats = Some((job.filter.clone(), node.files, node.size));
                            if only && job.filter == self.filter {
                                let tree = Arc::new(node);
                                self.set_shown(Some(Shown { filter: job.filter, tree, back: Arc::new(back), exts }));
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(50)),
                Err(mpsc::TryRecvError::Disconnected) => self.filter_job = None,
            }
        }

        if !only && self.shown.is_some() {
            self.set_shown(None);
        }
        if !active {
            if let Some(j) = self.filter_job.take() {
                j.cancel.store(true, Ordering::Relaxed);
            }
            return;
        }
        let have_stats = self.filter_stats.as_ref().is_some_and(|(f, _, _)| *f == self.filter);
        let have_tree = !only || self.shown.as_ref().is_some_and(|s| s.filter == self.filter);
        if have_stats && have_tree {
            return;
        }
        let want_tree = only && !have_tree;
        if self.filter_job.as_ref().is_some_and(|j| j.filter == self.filter && j.only == want_tree) {
            return;
        }
        if let Some(j) = self.filter_job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let filter = self.filter.clone();
        let (f, c) = (filter.clone(), cancel.clone());
        std::thread::spawn(move || {
            let keep = |n: &Node| f.matches(&n.name, n.size);
            let done = if want_tree {
                subset::subset(&tree, &keep, &c).map(|(node, back)| {
                    let exts = scan::ext_stats(&node);
                    FilterDone::Subset(node, back, exts)
                })
            } else {
                subset::count(&tree, &keep, &c).map(|(files, bytes)| FilterDone::Count(files, bytes))
            };
            if let Some(d) = done.filter(|_| !c.load(Ordering::Relaxed)) {
                let _ = tx.send(d);
            }
        });
        self.filter_job = Some(FilterJob { filter, only: want_tree, cancel, rx });
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    /// The match count is being worked out, or the filtered view is being built.
    pub(super) fn filtering_busy(&self) -> bool {
        self.filter_job.is_some()
    }

    /// Compile the typed text and, if it's valid, put it into effect now.
    pub(super) fn apply_custom(&mut self) {
        self.custom.edited = None;
        match Filter::compile(&self.custom.text, self.custom.options()) {
            Ok(f) => {
                self.custom.error = None;
                self.filter.custom = (!f.is_empty()).then(|| Arc::new(f));
            }
            Err(e) => self.custom.error = Some(e),
        }
    }

    /// Set the text (from the dropdown, a sequence, …) and apply it at once.
    pub(super) fn set_custom(&mut self, text: &str) {
        self.custom.text = text.to_string();
        self.custom.open = true;
        self.apply_custom();
        self.remember_recent();
    }

    fn remember_recent(&mut self) {
        let t = self.custom.text.trim().to_string();
        if t.is_empty() || self.custom.error.is_some() {
            return;
        }
        let r = &mut self.custom.prefs.recent;
        r.retain(|x| *x != t);
        r.insert(0, t);
        r.truncate(MAX_RECENT);
    }

    pub(super) fn clear_custom(&mut self) {
        self.custom.text.clear();
        self.custom.error = None;
        self.custom.edited = None;
        self.filter.custom = None;
    }

    /// Ctrl+F (⌘F on a Mac) opens the Custom filter and puts the cursor in it.
    pub(super) fn custom_shortcut(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.custom.open = true;
            self.custom.focus = true;
        }
    }

    /// The "Custom" toggle at the end of the Show: row. Shows the active pattern when the row
    /// is closed, so a filter in effect is never hidden.
    pub(super) fn custom_button(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let active = self.filter.custom.is_some();
        let label = match &self.filter.custom {
            Some(f) if !self.custom.open => {
                let s = f.source().trim();
                let short: String = s.chars().take(28).collect();
                format!("🔍 Custom: {short}{}", if s.chars().count() > 28 { "…" } else { "" })
            }
            _ => "🔍 Custom".to_string(),
        };
        let shortcut = ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F));
        let resp = ui
            .selectable_label(self.custom.open || active, label)
            .on_hover_text(format!("Filter by file name, extension or frame number ({shortcut})"));
        if resp.clicked() {
            self.custom.open = !self.custom.open;
            self.custom.focus = self.custom.open;
        }
    }

    /// The input row under the Show: row, when Custom is open.
    pub(super) fn custom_row(&mut self, ui: &mut egui::Ui) {
        if !self.custom.open {
            return;
        }
        let id = egui::Id::new("custom-filter-input");
        let had_focus = ui.memory(|m| m.has_focus(id));
        let suggestions = if had_focus { self.suggestions() } else { Vec::new() };
        if suggestions.is_empty() {
            self.custom.pick = 0;
        } else {
            self.custom.pick = self.custom.pick.min(suggestions.len() - 1);
        }
        // Keys for the suggestion list, taken before the text box sees them.
        let mut accept = None;
        if !suggestions.is_empty() {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    self.custom.pick = (self.custom.pick + 1) % suggestions.len();
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    self.custom.pick = (self.custom.pick + suggestions.len() - 1) % suggestions.len();
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                    accept = Some(self.custom.pick);
                }
            });
        }

        let mut input_rect = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Custom:");
            let edit = egui::TextEdit::singleline(&mut self.custom.text)
                .id(id)
                .desired_width(300.0)
                .hint_text("*.exr, shot_####.exr, !*preview*");
            let resp = ui.add(edit);
            input_rect = Some(resp.rect);
            if std::mem::take(&mut self.custom.focus) {
                resp.request_focus();
            }
            if resp.changed() {
                self.custom.edited = Some(Instant::now());
                // Validate right away so the warning keeps up with typing; apply after a pause.
                self.custom.error = Filter::compile(&self.custom.text, self.custom.options()).err();
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter {
                self.apply_custom();
                self.remember_recent();
            } else if resp.lost_focus() && self.custom.edited.is_none() {
                self.remember_recent();
            }
            if ui.add_enabled(!self.custom.text.is_empty(), egui::Button::new("✖").small()).on_hover_text("Clear the custom filter").clicked() {
                self.clear_custom();
            }
            let mut case = self.custom.prefs.case_sensitive;
            if ui.toggle_value(&mut case, "Aa").on_hover_text("Match upper/lower case exactly").changed() {
                self.custom.prefs.case_sensitive = case;
                self.apply_custom();
            }
            self.custom_menu(ui);
            if let Some(e) = &self.custom.error {
                ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {e}"));
            }
        });

        // Suggestions (or, with an empty box, examples) right under the input.
        if let (Some(rect), false) = (input_rect, suggestions.is_empty()) {
            let mut clicked = None;
            egui::Area::new(id.with("suggest"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.left_bottom() + vec2(0.0, 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(rect.width());
                        for (k, s) in suggestions.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.selectable_label(k == self.custom.pick, egui::RichText::new(&s.text).monospace()).clicked() {
                                    clicked = Some(k);
                                }
                                ui.weak(&s.note);
                            });
                        }
                        ui.weak(if self.custom.text.trim().is_empty() {
                            "Examples. Ctrl+F jumps here; the ⏷ menu has syntax help."
                        } else {
                            "Tab or click to complete"
                        });
                    });
                });
            if let Some(k) = clicked.or(accept) {
                let s = &suggestions[k];
                self.custom.text = s.replace.clone();
                self.custom.focus = true;
                self.apply_custom();
                // Put the cursor at the end of the completed text.
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                    let end = egui::text::CCursor::new(self.custom.text.chars().count());
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                    state.store(ui.ctx(), id);
                }
            }
        }
    }

    /// Completions for the extension being typed, or examples while the box is empty.
    fn suggestions(&self) -> Vec<Suggestion> {
        let text = &self.custom.text;
        if text.trim().is_empty() {
            return EXAMPLES
                .iter()
                .map(|(p, note)| Suggestion { text: p.to_string(), note: note.to_string(), replace: p.to_string() })
                .collect();
        }
        let Some((at, word)) = Filter::typing_ext(text) else { return Vec::new() };
        if word.is_empty() {
            return Vec::new();
        }
        let lower = word.to_ascii_lowercase();
        self.exts
            .iter()
            .filter(|e| !e.ext.is_empty() && e.ext.starts_with(&lower) && e.ext != lower)
            .take(8)
            .map(|e| Suggestion {
                text: format!(".{}", e.ext),
                note: format!("{} · {} files", scan::human(e.bytes), group(e.files)),
                replace: format!("{}{}", &text[..at], e.ext),
            })
            .collect()
    }

    /// The ▾ dropdown: examples, recent filters, presets, options and syntax help.
    fn custom_menu(&mut self, ui: &mut egui::Ui) {
        let config = MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let mut pick: Option<String> = None;
        MenuButton::new("⏷").config(config).ui(ui, |ui| {
            ui.set_max_width(420.0);
            if !self.custom.prefs.presets.is_empty() {
                ui.strong("Presets");
                let mut remove = None;
                for (k, p) in self.custom.prefs.presets.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("🗑").on_hover_text("Delete this preset").clicked() {
                            remove = Some(k);
                        }
                        if ui.button(&p.name).on_hover_text(&p.pattern).clicked() {
                            pick = Some(p.pattern.clone());
                        }
                    });
                }
                if let Some(k) = remove {
                    self.custom.prefs.presets.remove(k);
                }
                ui.separator();
            }
            let can_save = !self.custom.text.trim().is_empty() && self.custom.error.is_none();
            ui.horizontal(|ui| {
                ui.add_enabled(
                    can_save,
                    egui::TextEdit::singleline(&mut self.custom.preset_name).desired_width(150.0).hint_text("Preset name"),
                );
                if ui.add_enabled(can_save, egui::Button::new("Save as preset")).clicked() {
                    let pattern = self.custom.text.trim().to_string();
                    let name = match self.custom.preset_name.trim() {
                        "" => pattern.clone(),
                        n => n.to_string(),
                    };
                    let presets = &mut self.custom.prefs.presets;
                    presets.retain(|p| p.name != name);
                    presets.push(Preset { name, pattern });
                    self.custom.preset_name.clear();
                }
            });
            ui.separator();
            if !self.custom.prefs.recent.is_empty() {
                ui.horizontal(|ui| {
                    ui.strong("Recent");
                    if ui.small_button("Clear").clicked() {
                        self.custom.prefs.recent.clear();
                    }
                });
                for r in &self.custom.prefs.recent {
                    if ui.button(egui::RichText::new(r).monospace()).clicked() {
                        pick = Some(r.clone());
                    }
                }
                ui.separator();
            }
            ui.strong("Examples");
            for (p, note) in EXAMPLES {
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new(p).monospace()).clicked() {
                        pick = Some(p.to_string());
                    }
                    ui.weak(note);
                });
            }
            ui.separator();
            ui.strong("Options");
            let mut changed = ui.checkbox(&mut self.custom.prefs.case_sensitive, "Case-sensitive").changed();
            changed |= ui.radio_value(&mut self.custom.prefs.match_all, false, "Match any term (OR)").changed();
            changed |= ui
                .radio_value(&mut self.custom.prefs.match_all, true, "Match all terms (AND)")
                .on_hover_text("A file must match every term; ! terms still exclude")
                .changed();
            if changed {
                self.apply_custom();
            }
            if ui.button("Find image sequences…").on_hover_text("List numbered file sequences in the Sequences tab").clicked() {
                self.side_tab = SideTab::Sequences;
                self.start_sequences();
                ui.close();
            }
            ui.separator();
            ui.collapsing("Syntax", |ui| {
                ui.label(egui::RichText::new(pattern::HELP).monospace().size(11.5));
            });
            if pick.is_some() {
                ui.close();
            }
        });
        if let Some(p) = pick {
            self.set_custom(&p);
        }
    }

    /// Below the size boxes when a filter is on: Highlight / Show matches only, and the count.
    pub(super) fn filter_summary(&mut self, ui: &mut egui::Ui, total: u64) {
        ui.separator();
        let mut mode = self.custom.prefs.mode;
        ui.selectable_value(&mut mode, FilterMode::Highlight, "Highlight")
            .on_hover_text("Keep the layout; dim everything that doesn't match");
        ui.selectable_value(&mut mode, FilterMode::Only, "Show matches only")
            .on_hover_text("Rebuild the maps and folder list from the matching files alone");
        self.custom.prefs.mode = mode;
        ui.separator();
        let current = self.filter_stats.as_ref().filter(|(f, _, _)| *f == self.filter);
        match current {
            Some((_, 0, _)) => {
                ui.colored_label(ui.visuals().warn_fg_color, "No files match");
            }
            Some((_, files, bytes)) => {
                let pct = *bytes as f64 * 100.0 / total.max(1) as f64;
                ui.strong(format!("{} files match · {} ({pct:.1}% of the scan)", group(*files), scan::human(*bytes)));
            }
            None => {}
        }
        if self.filtering_busy() || self.custom.edited.is_some() {
            ui.spinner();
            ui.weak(if mode == FilterMode::Only { "Filtering…" } else { "Counting…" });
        }
    }

    // ---------- sequences ----------

    pub(super) fn start_sequences(&mut self) {
        let Some(tree) = self.tree.clone() else { return };
        if let Some((c, _)) = self.seq.job.take() {
            c.store(true, Ordering::Relaxed);
        }
        if self.seq.min_frames == 0 {
            self.seq.min_frames = 3;
        }
        let min = self.seq.min_frames;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let c = cancel.clone();
        std::thread::spawn(move || {
            if let Some(list) = sequences::detect(&tree, min, &c) {
                let _ = tx.send(list);
            }
        });
        self.seq.job = Some((cancel, rx));
        self.seq.list = None;
    }

    pub(super) fn poll_sequences(&mut self, ctx: &egui::Context) {
        if let Some((_, rx)) = &self.seq.job {
            match rx.try_recv() {
                Ok(list) => {
                    self.seq.list = Some(list);
                    self.seq.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => self.seq.job = None,
            }
        }
    }

    /// The Sequences tab: numbered file sequences found on request.
    pub(super) fn sequences_panel(&mut self, ui: &mut egui::Ui) {
        if self.seq.min_frames == 0 {
            self.seq.min_frames = 3;
        }
        ui.horizontal_wrapped(|ui| {
            let busy = self.seq.job.is_some();
            if ui.add_enabled(!busy, egui::Button::new("Find sequences")).clicked() {
                self.start_sequences();
            }
            ui.label("at least");
            ui.add(egui::DragValue::new(&mut self.seq.min_frames).range(2..=1000));
            ui.label("frames");
            if busy {
                ui.spinner();
                if ui.small_button("Cancel").clicked()
                    && let Some((c, _)) = self.seq.job.take()
                {
                    c.store(true, Ordering::Relaxed);
                }
            }
        });
        let Some(list) = self.seq.list.take() else {
            ui.add_space(6.0);
            ui.weak(
                "Groups numbered files such as render.1001.exr, render.1002.exr … into one entry, \
                 with frame range, missing frames and total size. Click one to filter the map to it.",
            );
            return;
        };
        let total: u64 = list.iter().map(|s| s.bytes).sum();
        ui.label(format!("{} sequences · {}", group(list.len() as u64), scan::human(total)));
        if list.is_empty() {
            ui.weak("No numbered sequences found.");
        }
        let mut chosen = None;
        let h = 2.0 * ROW_H + 4.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, h, list.len(), |ui, range| {
            for (k, s) in list[range.clone()].iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
                let vis = ui.visuals().clone();
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 2.0, vis.widgets.hovered.weak_bg_fill);
                } else if (range.start + k) % 2 == 1 && self.view().stripes {
                    ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
                }
                let font = FontId::proportional(13.0);
                let top = rect.top() + ROW_H / 2.0 + 2.0;
                let bottom = top + ROW_H;
                let clip = Rect::from_min_max(rect.left_top(), pos2(rect.right() - 80.0, rect.bottom()));
                let p = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
                p.text(pos2(rect.left() + 4.0, top), Align2::LEFT_CENTER, s.display(), font.clone(), vis.strong_text_color());
                let mut info = format!("{} frames · {} · avg {}", group(s.frames), s.range_text(), scan::human(s.average()));
                if s.missing > 0 {
                    info.push_str(&format!(" · {} missing", group(s.missing)));
                }
                ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).text(
                    pos2(rect.left() + 4.0, bottom),
                    Align2::LEFT_CENTER,
                    info,
                    font.clone(),
                    vis.weak_text_color(),
                );
                ui.painter().text(pos2(rect.right() - 4.0, top), Align2::RIGHT_CENTER, scan::human(s.bytes), font.clone(), vis.text_color());
                let resp = resp.on_hover_ui(|ui| {
                    ui.label(self.full_path_of_full(&s.folder).display().to_string());
                    let pad = s.padding.map_or("not padded".to_string(), |w| format!("{w} digits"));
                    ui.label(format!(".{} · {pad}", s.extension()));
                    if s.missing > 0 {
                        ui.label(format!("Missing: {}", s.gaps_text()));
                    }
                    ui.weak("Click to filter the map to this sequence");
                });
                if resp.clicked() {
                    chosen = Some(range.start + k);
                }
            }
        });
        if let Some(i) = chosen {
            let s = &list[i];
            let (pattern, folder) = (s.pattern(), s.folder.clone());
            self.set_custom(&pattern);
            self.select_full(folder);
        }
        self.seq.list = Some(list);
    }

    /// Disk path of an entry given by its path in the whole scan.
    pub(super) fn full_path_of_full(&self, full: &[u32]) -> PathBuf {
        let mut p = self.scanned_path.clone();
        let Some(mut n) = self.tree.as_deref() else { return p };
        for &i in full {
            let Some(c) = n.children.get(i as usize) else { break };
            n = c;
            p.push(&*n.name);
        }
        p
    }
}

struct Suggestion {
    text: String,
    note: String,
    replace: String,
}

/// A non-matching tile in Highlight mode: a dark grey that keeps a hint of its brightness, so
/// the layout stays readable while the matches stand out.
pub(super) fn dim(c: Color32) -> Color32 {
    Color32::from_gray((26.0 + luma(c) * 0.16) as u8)
}
