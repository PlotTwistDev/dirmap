//! The address bar: breadcrumbs you can click, drop onto and right-click, and a path field
//! with suggestions as you type.
//!
//! Typing offers the folders inside what's typed so far (read on a worker thread, so a slow
//! share never stalls typing), then recent folders and favourites that match. Tab completes,
//! Up and Down choose, Enter goes. `~`, `%VAR%` and `$VAR` are expanded. Two optional prefixes:
//! `>` lists dirmap commands and `@` searches favourites. Nothing typed here is ever run as a
//! shell command.

use crate::fsys::PathShow;
use super::files::{self, FAction};
use super::*;
use crate::fsys::history::Location;
use crate::fsys::listing::Listing;
use crate::fsys::places;
use std::path::Path;

/// One suggestion under the path field.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Suggestion {
    pub label: String,
    /// Shown faint on the right: "Recent", "Favourite", a shortcut…
    pub note: String,
    pub pick: Pick,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Pick {
    /// Put this text in the field (a folder: ends with a separator, ready for more).
    Text(String),
    Command(Cmd),
}

/// dirmap commands reachable from `>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cmd {
    NewFolder,
    NewTab,
    SplitView,
    SwapPanes,
    IconsView,
    PreviewPane,
    HiddenItems,
    AnalysisDock,
    AnalyseFolder,
    StorageAnalysis,
    ConnectServer,
    TemplatesFolder,
    Settings,
    Help,
}

impl Cmd {
    const ALL: [Cmd; 14] = [
        Cmd::NewFolder,
        Cmd::NewTab,
        Cmd::SplitView,
        Cmd::SwapPanes,
        Cmd::IconsView,
        Cmd::PreviewPane,
        Cmd::HiddenItems,
        Cmd::AnalysisDock,
        Cmd::AnalyseFolder,
        Cmd::StorageAnalysis,
        Cmd::ConnectServer,
        Cmd::TemplatesFolder,
        Cmd::Settings,
        Cmd::Help,
    ];

    fn label(self) -> &'static str {
        match self {
            Cmd::NewFolder => "New Folder",
            Cmd::NewTab => "New Tab",
            Cmd::SplitView => "Split View: one or two panes",
            Cmd::SwapPanes => "Swap Panes",
            Cmd::IconsView => "Icons or Details View",
            Cmd::PreviewPane => "Preview Pane",
            Cmd::HiddenItems => "Show or Hide Hidden Items",
            Cmd::AnalysisDock => "Analysis Dock",
            Cmd::AnalyseFolder => "Analyse This Folder",
            Cmd::StorageAnalysis => "Storage Analysis Workspace",
            Cmd::ConnectServer => "Connect to Server…",
            Cmd::TemplatesFolder => "Open Templates Folder",
            Cmd::Settings => "Settings",
            Cmd::Help => "Help Centre",
        }
    }

    fn keys(self) -> &'static str {
        match (self, cfg!(target_os = "macos")) {
            (Cmd::NewFolder, false) => "Ctrl+Shift+N",
            (Cmd::NewFolder, true) => "⇧⌘N",
            (Cmd::NewTab, false) => "Ctrl+T",
            (Cmd::NewTab, true) => "⌘T",
            (Cmd::SplitView, false) => "Ctrl+\\",
            (Cmd::SplitView, true) => "⌘\\",
            (Cmd::SwapPanes, false) => "Ctrl+U",
            (Cmd::SwapPanes, true) => "⌘U",
            (Cmd::PreviewPane, false) => "Alt+P",
            (Cmd::PreviewPane, true) => "⌥P",
            (Cmd::AnalysisDock, false) => "Ctrl+J",
            (Cmd::AnalysisDock, true) => "⌘J",
            (Cmd::StorageAnalysis, false) => "Ctrl+3",
            (Cmd::StorageAnalysis, true) => "⌘3",
            (Cmd::Help, _) => "F1",
            _ => "",
        }
    }
}

/// Suggestions for what's typed, and the folder being read for them.
#[derive(Default)]
pub(super) struct Suggest {
    /// The text the list was made for.
    for_text: String,
    /// The folder whose sub-folders are offered, and its listing.
    dir: Option<PathBuf>,
    listing: Option<Listing>,
    pub items: Vec<Suggestion>,
    /// The highlighted suggestion.
    pub sel: Option<usize>,
}

/// Expand `~`, `%VAR%` (Windows) and `$VAR` / `${VAR}` (elsewhere) in a typed path.
pub(super) fn expand(s: &str) -> String {
    let s = s.trim().trim_matches('"');
    let mut out = String::new();
    // ~ at the start: home.
    let rest = if let Some(r) = s.strip_prefix('~').filter(|r| r.is_empty() || r.starts_with(['/', '\\'])) {
        if let Some(h) = places::home() {
            out.push_str(&h.shown());
            r
        } else {
            s
        }
    } else {
        s
    };
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '%' && cfg!(windows) {
            if let Some(j) = chars[i + 1..].iter().position(|&x| x == '%') {
                let name: String = chars[i + 1..i + 1 + j].iter().collect();
                if let Some(v) = std::env::var_os(&name).filter(|_| !name.is_empty()) {
                    out.push_str(&v.to_string_lossy());
                    i += j + 2;
                    continue;
                }
            }
        } else if c == '$' && !cfg!(windows) {
            let (name, used) = if chars.get(i + 1) == Some(&'{') {
                match chars[i + 2..].iter().position(|&x| x == '}') {
                    Some(j) => (chars[i + 2..i + 2 + j].iter().collect::<String>(), j + 3),
                    None => (String::new(), 0),
                }
            } else {
                let n: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '_').collect();
                let len = n.chars().count();
                (n, len + 1)
            };
            if let Some(v) = std::env::var_os(&name).filter(|_| !name.is_empty()) {
                out.push_str(&v.to_string_lossy());
                i += used;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// What a typed location means: a folder, This PC / Computer, or a reason it can't be opened.
pub(super) fn resolve(s: &str) -> Result<Location, String> {
    let t = s.trim().trim_matches('"');
    if t.is_empty() {
        return Err(String::new());
    }
    if t.eq_ignore_ascii_case(places::computer_name()) {
        return Ok(Location::Computer);
    }
    let lower = t.to_lowercase();
    if let Some(p) = crate::fsys::remote::parse(t) {
        return Ok(Location::Dir(p));
    }
    if lower.contains("://") && !lower.starts_with("smb://") && !lower.starts_with("file://") {
        return Err("dirmap opens sftp://, ftp://, ftps://, dav:// and davs:// (or https://) addresses, smb:// shares and file:// paths.".into());
    }
    if lower.starts_with("smb://") {
        let Some((server, share, rest)) = crate::netdrives::parse_smb(t) else { return Err("Type smb://server/share.".into()) };
        if cfg!(windows) {
            let mut p = format!(r"\\{server}\{share}");
            if !rest.is_empty() {
                p.push('\\');
                p.push_str(&rest.replace('/', "\\"));
            }
            return Ok(Location::Dir(PathBuf::from(p)));
        }
        // Elsewhere an SMB share is opened through its mount.
        return Err(format!("Connect to smb://{server}/{share} first (Connect to Server in the sidebar's Network & Remote section), then open its folder."));
    }
    if lower.starts_with("file://") {
        let p = &t[7..];
        return Ok(Location::Dir(PathBuf::from(if cfg!(windows) { p.trim_start_matches('/') } else { p })));
    }
    let mut p = expand(t);
    if cfg!(windows) && p.len() == 2 && p.ends_with(':') {
        p.push('\\');
    }
    // Rebuilt from its parts, so "~/Projects/" and "~/Projects/." open as ~/Projects.
    Ok(Location::Dir(Path::new(&p).components().collect()))
}

/// Split typed text into (folder to list, start of the name being typed).
fn split_typed(text: &str) -> Option<(PathBuf, String)> {
    let e = expand(text);
    let i = e.rfind(['/', '\\'])?;
    let (dir, name) = (&e[..=i], &e[i + 1..]);
    let dir = if dir.len() > 1 { dir.trim_end_matches(['/', '\\']) } else { dir };
    let dir = if cfg!(windows) && dir.len() == 2 && dir.ends_with(':') { format!("{dir}\\") } else if dir.is_empty() { "/".into() } else { dir.to_string() };
    Some((PathBuf::from(dir), name.to_string()))
}

/// The longest start all of these share (ignoring case), for Tab.
fn common_prefix(names: &[&str]) -> String {
    let Some(first) = names.first() else { return String::new() };
    let mut n = first.chars().count();
    for s in &names[1..] {
        n = n.min(first.chars().zip(s.chars()).take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase())).count());
    }
    first.chars().take(n).collect()
}

fn sep() -> char {
    std::path::MAIN_SEPARATOR
}

impl App {
    /// Work out the suggestions for the text in the path field (cheap; folder reads are async).
    fn update_suggestions(&mut self, text: &str) {
        let s = &mut self.files.suggest;
        let mut changed = s.for_text != text;
        if let Some(l) = s.listing.as_mut()
            && l.poll()
        {
            changed = true;
        }
        if !changed {
            return;
        }
        s.for_text = text.to_string();
        let mut items = Vec::new();
        let t = text.trim_start();
        if let Some(q) = t.strip_prefix('>') {
            let q = q.trim().to_lowercase();
            for c in Cmd::ALL {
                if q.is_empty() || c.label().to_lowercase().contains(&q) {
                    items.push(Suggestion { label: c.label().into(), note: c.keys().into(), pick: Pick::Command(c) });
                }
            }
        } else if let Some(q) = t.strip_prefix('@') {
            let q = q.trim().to_lowercase();
            for g in &self.files.prefs.favourites {
                for f in &g.items {
                    let shown = f.shown();
                    if q.is_empty() || shown.to_lowercase().contains(&q) || f.path.shown().to_lowercase().contains(&q) {
                        items.push(Suggestion { label: format!("★ {shown}"), note: f.path.shown(), pick: Pick::Text(f.path.shown()) });
                    }
                }
            }
        } else if let Some((dir, start)) = split_typed(t) {
            // Sub-folders of what's typed so far.
            if s.dir.as_ref() != Some(&dir) {
                s.dir = Some(dir.clone());
                s.listing = Some(Listing::start(dir.clone()));
            }
            let start_l = start.to_lowercase();
            let typed_dir = &t[..t.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0)];
            if let Some(l) = &s.listing {
                let mut names: Vec<&str> = l
                    .entries
                    .iter()
                    .filter(|e| e.is_dir && (self.files.prefs.show_hidden || !e.hidden || start.starts_with('.')))
                    .map(|e| &*e.name)
                    .filter(|n| n.to_lowercase().starts_with(&start_l) && *n != start)
                    .collect();
                names.sort_by(|a, b| browser::natural_cmp(a, b));
                for n in names.into_iter().take(40) {
                    items.push(Suggestion { label: n.to_string(), note: String::new(), pick: Pick::Text(format!("{typed_dir}{n}{}", sep())) });
                }
            }
        }
        if !t.starts_with(['>', '@']) && !t.is_empty() {
            // Recent folders and favourites containing what's typed.
            let q = expand(t).to_lowercase();
            let mut more = Vec::new();
            for g in &self.files.prefs.favourites {
                for f in &g.items {
                    more.push((f.path.clone(), "Favourite"));
                }
            }
            for p in &self.files.prefs.recent {
                more.push((p.clone(), "Recent"));
            }
            for (p, note) in more {
                let d = p.shown();
                if d.to_lowercase().contains(&q) && !items.iter().any(|i: &Suggestion| i.pick == Pick::Text(d.clone())) && items.len() < 60 {
                    items.push(Suggestion { label: d.clone(), note: note.into(), pick: Pick::Text(d) });
                }
            }
        }
        let s = &mut self.files.suggest;
        if s.items != items {
            s.sel = None;
        }
        s.items = items;
    }

    /// The address bar: breadcrumbs, or the path field while typing.
    pub(super) fn path_bar(&mut self, ui: &mut egui::Ui) -> Option<Location> {
        let id = egui::Id::new(("files-path", self.files.focus));
        let rect = ui.available_rect_before_wrap();
        let frame = Rect::from_min_size(rect.min, vec2(rect.width(), ui.spacing().interact_size.y));
        if self.files.path_edit.is_some() && self.files.path_pane == self.files.focus {
            return self.path_field(ui, id, frame);
        }
        // Breadcrumbs.
        let bg = ui.interact(frame, id.with("bg"), Sense::click());
        let pal = style::pal(ui);
        let border = if bg.hovered() { pal.weak } else { pal.hairline };
        ui.painter().rect(frame, style::RADIUS as f32, pal.well, Stroke::new(1.0, border), egui::StrokeKind::Inside);
        let mut go = None;
        let mut drop = None;
        let mut menu = None;
        let here = self.files.here().clone();
        let crumbs = files::crumbs_of(&here);
        let last = crumbs.len().saturating_sub(1);
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        // Keep the end of a long path visible: drop leading crumbs that don't fit.
        let font = egui::TextStyle::Button.resolve(ui.style());
        let width_of = |s: &str| ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x) + 16.0;
        let mut total: f32 = crumbs.iter().map(|c| width_of(&c.0)).sum();
        let mut skip = 0;
        while total > frame.width() - 40.0 && skip + 1 < crumbs.len() {
            total -= width_of(&crumbs[skip].0);
            skip += 1;
        }
        if skip > 0 {
            ui.weak("…");
        }
        let other = self.files.other();
        for (k, (label, loc)) in crumbs.into_iter().enumerate().skip(skip) {
            if k > skip {
                ui.weak("›");
            }
            let text = if k == last { egui::RichText::new(label).strong() } else { egui::RichText::new(label) };
            let resp = ui.add(egui::Button::new(text).frame(false));
            if resp.clicked() && k != last {
                go = Some(loc.clone());
            }
            if resp.middle_clicked() {
                self.files.new_tab(loc.clone());
            }
            if let Location::Dir(p) = &loc {
                if let Some(d) = self.folder_drop(ui, &resp, p) {
                    drop = Some(d);
                }
                let pinned = self.files.is_pinned(p);
                resp.context_menu(|ui| {
                    if ui.button("Open in New Tab").clicked() {
                        menu = Some((FAction::OpenInNewTab, p.clone()));
                    }
                    if other.is_some() && ui.button("Open in Other Pane").clicked() {
                        menu = Some((FAction::OpenInOtherPane, p.clone()));
                    }
                    ui.separator();
                    if ui.button("Copy Path").clicked() {
                        menu = Some((FAction::CopyPath, p.clone()));
                    }
                    if cfg!(windows) && ui.button("Copy Network (UNC) Path").on_hover_text("For a mapped drive: the \\\\server\\share form other computers can use").clicked() {
                        menu = Some((FAction::CopyUnc, p.clone()));
                    }
                    if !pinned && ui.button("Add to Favourites").clicked() {
                        menu = Some((FAction::Pin, p.clone()));
                    }
                    if ui.button("Analyse This Folder").clicked() {
                        menu = Some((FAction::Analyse, p.clone()));
                    }
                });
            }
        }
        if bg.clicked() {
            self.start_path_edit();
        }
        bg.on_hover_text(format!(
            "Click to type a path ({}). Type > for commands, @ for favourites",
            if cfg!(target_os = "macos") { "⌘L" } else { "Ctrl+L, Alt+D or F4" }
        ));
        if let Some((items, dest, op)) = drop {
            self.files_transfer(items, dest, op);
        }
        if let Some((a, p)) = menu {
            self.files_action(a, vec![p]);
        }
        go
    }

    /// The path field with its suggestion list.
    fn path_field(&mut self, ui: &mut egui::Ui, id: egui::Id, frame: Rect) -> Option<Location> {
        let has_focus = ui.memory(|m| m.has_focus(id));
        // Keys for the list, taken before the field sees them.
        let n = self.files.suggest.items.len();
        let (down, up, tab, esc) = if has_focus {
            ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                )
            })
        } else {
            (false, false, false, false)
        };
        let s = &mut self.files.suggest;
        if down && n > 0 {
            s.sel = Some(s.sel.map_or(0, |i| (i + 1) % n));
        }
        if up && n > 0 {
            s.sel = Some(s.sel.map_or(n - 1, |i| (i + n - 1) % n));
        }
        if esc {
            self.files.path_edit = None;
            self.files.suggest = Default::default();
            return None;
        }
        let mut set_text: Option<String> = None;
        if tab {
            // Complete: the chosen suggestion, else the part every folder suggestion shares.
            let s = &self.files.suggest;
            let text = self.files.path_edit.clone().unwrap_or_default();
            match s.sel.and_then(|i| s.items.get(i)).map(|x| x.pick.clone()) {
                Some(Pick::Text(t)) => set_text = Some(t),
                _ => {
                    let folders: Vec<&str> = s.items.iter().filter(|x| x.note.is_empty()).map(|x| x.label.as_str()).collect();
                    if let Some((_, start)) = split_typed(&text) {
                        let typed_dir = &text[..text.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0)];
                        if folders.len() == 1 {
                            set_text = Some(format!("{typed_dir}{}{}", folders[0], sep()));
                        } else {
                            let c = common_prefix(&folders);
                            if c.chars().count() > start.chars().count() {
                                set_text = Some(format!("{typed_dir}{c}"));
                            }
                        }
                    }
                }
            }
        }
        let text = self.files.path_edit.as_mut().unwrap();
        if let Some(t) = &set_text {
            *text = t.clone();
        }
        let resp = ui.add(
            egui::TextEdit::singleline(text)
                .id(id)
                .lock_focus(true)
                .desired_width(frame.width())
                .hint_text(if cfg!(windows) { "Type a folder: C:\\Projects, \\\\server\\share, %USERPROFILE%  ·  > commands  ·  @ favourites" } else { "Type a folder: ~/Projects, /mnt/nas, $HOME  ·  > commands  ·  @ favourites" }),
        );
        let fresh = std::mem::take(&mut self.files.focus_path);
        if fresh || set_text.is_some() {
            resp.request_focus();
            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
                let end = egui::text::CCursor::new(text.chars().count());
                let start = if fresh { egui::text::CCursor::new(0) } else { end };
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(start, end)));
                st.store(ui.ctx(), id);
            }
        }
        let text = text.clone();
        self.update_suggestions(&text);
        if self.files.suggest.listing.as_ref().is_some_and(|l| !l.done) {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        // The list under the field.
        let mut clicked = None;
        if resp.has_focus() || fresh {
            clicked = self.suggestion_list(ui, resp.rect);
        }
        let was_click = clicked.is_some();
        let chosen = clicked.or_else(|| if enter { self.files.suggest.sel.and_then(|i| self.files.suggest.items.get(i)).map(|s| s.pick.clone()) } else { None });
        match chosen {
            Some(Pick::Command(c)) => {
                self.files.path_edit = None;
                self.files.suggest = Default::default();
                self.run_command(c);
                return None;
            }
            Some(Pick::Text(t)) if was_click && t.ends_with(sep()) => {
                // A folder clicked in the list: fill it in and keep typing.
                *self.files.path_edit.as_mut().unwrap() = t;
                ctx_focus(ui, id);
                return None;
            }
            Some(Pick::Text(t)) => return self.finish_typed(&t),
            None => {}
        }
        if enter {
            if text.trim_start().starts_with(['>', '@']) {
                // Enter on a prefix with no choice: take the first match.
                if let Some(p) = self.files.suggest.items.first().map(|s| s.pick.clone()) {
                    self.files.path_edit = None;
                    return match p {
                        Pick::Command(c) => {
                            self.run_command(c);
                            None
                        }
                        Pick::Text(t) => self.finish_typed(&t),
                    };
                }
                return None;
            }
            return self.finish_typed(&text);
        }
        if resp.lost_focus() && !was_click && !ui.ctx().memory(|m| m.has_focus(id)) {
            // Clicked elsewhere (the list's own clicks are handled above).
            let over_list = ui.ctx().pointer_latest_pos().is_some_and(|p| self.files.suggest_rect.is_some_and(|r| r.contains(p)));
            if !over_list {
                self.files.path_edit = None;
                self.files.suggest = Default::default();
            }
        }
        None
    }

    fn finish_typed(&mut self, t: &str) -> Option<Location> {
        self.files.path_edit = None;
        self.files.suggest = Default::default();
        match resolve(t) {
            // A server address with no folder: connect, then open the account's own folder.
            Ok(Location::Dir(p)) if crate::fsys::remote::is_root(&p) && !t.trim().split_once("://").is_some_and(|(_, r)| r.trim_end_matches('/').contains('/')) => {
                self.open_server(p);
                None
            }
            Ok(l) => Some(l),
            Err(e) if e.is_empty() => None,
            Err(e) => {
                self.ops.notice = Some((e, true));
                None
            }
        }
    }

    /// Draw the suggestions under the field; returns one that was clicked.
    fn suggestion_list(&mut self, ui: &mut egui::Ui, field: Rect) -> Option<Pick> {
        let s = &self.files.suggest;
        if s.items.is_empty() {
            self.files.suggest_rect = None;
            return None;
        }
        let pal = style::pal(ui);
        let row_h = 22.0;
        let shown = s.items.len().min(12);
        let mut picked = None;
        let area = egui::Area::new(egui::Id::new("path-suggest")).order(egui::Order::Foreground).fixed_pos(field.left_bottom() + vec2(0.0, 2.0));
        let r = area.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                ui.set_width(field.width().max(320.0) - 8.0);
                egui::ScrollArea::vertical().max_height(row_h * shown as f32).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, item) in s.items.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                        let on = s.sel == Some(i) || resp.hovered();
                        if on {
                            ui.painter().rect_filled(r, style::RADIUS as f32, if s.sel == Some(i) { pal.sel } else { pal.hover });
                        }
                        let icon = match &item.pick {
                            Pick::Command(_) => "›",
                            Pick::Text(_) if item.note == "Recent" => "🕘",
                            Pick::Text(_) if item.note.is_empty() => "🗀",
                            _ => "",
                        };
                        ui.painter().text(pos2(r.left() + 8.0, r.center().y), Align2::LEFT_CENTER, icon, FontId::proportional(12.0), pal.weak);
                        let clip = Rect::from_min_max(pos2(r.left() + 26.0, r.top()), pos2(r.right() - 120.0, r.bottom()));
                        ui.painter().with_clip_rect(clip).text(pos2(clip.left(), r.center().y), Align2::LEFT_CENTER, &item.label, FontId::proportional(13.0), pal.text);
                        if !item.note.is_empty() {
                            let nclip = Rect::from_min_max(pos2(r.right() - 116.0, r.top()), pos2(r.right() - 6.0, r.bottom()));
                            ui.painter().with_clip_rect(nclip).text(pos2(nclip.right(), r.center().y), Align2::RIGHT_CENTER, &item.note, FontId::proportional(11.5), pal.weak);
                        }
                        if resp.clicked() {
                            picked = Some(item.pick.clone());
                        }
                    }
                });
            });
        });
        self.files.suggest_rect = Some(r.response.rect);
        picked
    }

    pub(super) fn start_path_edit(&mut self) {
        let text = match self.files.here() {
            Location::Dir(p) => p.shown(),
            Location::Computer => String::new(),
        };
        self.files.path_edit = Some(text);
        self.files.path_pane = self.files.focus;
        self.files.focus_path = true;
        self.files.suggest = Default::default();
    }

    /// Run a command picked with `>`.
    pub(super) fn run_command(&mut self, c: Cmd) {
        let dir = self.files.here().dir().map(Path::to_path_buf);
        match c {
            Cmd::NewFolder => {
                if let Some(d) = dir {
                    self.files_new_folder(d);
                }
            }
            Cmd::NewTab => {
                let h = self.files.here().clone();
                self.files.new_tab(h);
            }
            Cmd::SplitView => self.toggle_split(),
            Cmd::SwapPanes => self.files.swap_panes(),
            Cmd::IconsView => {
                let p = self.files.pane_mut();
                p.view = if p.view == files::FView::Icons { files::FView::Details } else { files::FView::Icons };
            }
            Cmd::PreviewPane => self.files.prefs.preview = !self.files.prefs.preview,
            Cmd::HiddenItems => self.files.prefs.show_hidden = !self.files.prefs.show_hidden,
            Cmd::AnalysisDock => self.files.prefs.dock.open = !self.files.prefs.dock.open,
            Cmd::AnalyseFolder => {
                if let Some(d) = dir {
                    self.analyse(d);
                }
            }
            Cmd::StorageAnalysis => self.set_workspace(files::Workspace::Analysis),
            Cmd::ConnectServer => self.files.connect = Some(super::netui::ConnectBox { remember: true, save: true, fresh: true, ..Default::default() }),
            Cmd::TemplatesFolder => self.open_templates_folder(),
            Cmd::Settings => self.dialog.open(&self.settings),
            Cmd::Help => self.help.show_topic(crate::help::topics::FILE_MANAGER),
        }
    }

    /// One pane or two (Ctrl+\): the second opens side by side, or as it was last.
    pub(super) fn toggle_split(&mut self) {
        let to = if self.files.prefs.split == files::Split::Single {
            if self.files.prefs.last_split == files::Split::Single { files::Split::Vertical } else { self.files.prefs.last_split }
        } else {
            self.files.prefs.last_split = self.files.prefs.split;
            files::Split::Single
        };
        self.files.set_split(to);
    }
}

fn ctx_focus(ui: &egui::Ui, id: egui::Id) {
    ui.ctx().memory_mut(|m| m.request_focus(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home_and_variables() {
        if let Some(h) = places::home() {
            assert_eq!(expand("~"), h.shown());
            assert!(expand("~/x").ends_with("x"));
            assert!(expand("~/x").starts_with(&h.shown()));
        }
        // SAFETY: tests in this module don't read DIRMAP_T_VAR concurrently.
        unsafe { std::env::set_var("DIRMAP_T_VAR", "/data") };
        if cfg!(windows) {
            assert_eq!(expand(r"%DIRMAP_T_VAR%\x"), r"/data\x");
            assert_eq!(expand("%NOPE_NOT_SET%"), "%NOPE_NOT_SET%");
        } else {
            assert_eq!(expand("$DIRMAP_T_VAR/x"), "/data/x");
            assert_eq!(expand("${DIRMAP_T_VAR}x"), "/datax");
            assert_eq!(expand("$NOPE_NOT_SET/x"), "$NOPE_NOT_SET/x");
            assert_eq!(expand("/a/b"), "/a/b");
        }
    }

    #[test]
    fn typed_locations() {
        assert_eq!(resolve("  "), Err(String::new()));
        assert_eq!(resolve(places::computer_name()), Ok(Location::Computer));
        assert_eq!(resolve("\"/tmp/x\""), Ok(Location::Dir(PathBuf::from("/tmp/x"))));
        assert_eq!(resolve("/tmp/x/"), Ok(Location::Dir(PathBuf::from("/tmp/x"))));
        assert_eq!(resolve("/"), Ok(Location::Dir(PathBuf::from("/"))));
        assert_eq!(resolve("sftp://rich@box/home"), Ok(Location::Dir(PathBuf::from("sftp://rich@box/home"))));
        assert!(resolve("gopher://box").is_err());
        if cfg!(windows) {
            assert_eq!(resolve("d:"), Ok(Location::Dir(PathBuf::from("d:\\"))));
            assert_eq!(resolve("smb://nas/media/tv"), Ok(Location::Dir(PathBuf::from(r"\\nas\media\tv"))));
        } else {
            assert!(resolve("smb://nas/media").is_err());
            assert_eq!(resolve("file:///tmp"), Ok(Location::Dir(PathBuf::from("/tmp"))));
        }
    }

    #[test]
    fn completing_names() {
        assert_eq!(common_prefix(&["Projects", "Proposals"]), "Pro");
        assert_eq!(common_prefix(&["render", "Renders"]), "render");
        assert_eq!(common_prefix(&[]), "");
        if !cfg!(windows) {
            assert_eq!(split_typed("/home/ri"), Some((PathBuf::from("/home"), "ri".into())));
            assert_eq!(split_typed("/ho"), Some((PathBuf::from("/"), "ho".into())));
            assert_eq!(split_typed("/home/"), Some((PathBuf::from("/home"), "".into())));
            assert_eq!(split_typed("nothing"), None);
        }
    }
}
