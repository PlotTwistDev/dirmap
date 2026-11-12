//! The built-in Help Centre: a searchable window showing the manual in `docs/src`, which is
//! bundled into the program so it works offline and always matches this version.
//!
//! The app opens it from the Help menu, with F1 (context help) and from the small "?" links next
//! to controls; links use the page paths in `topics`.

pub mod doc;
#[cfg(test)]
pub mod reference;
mod render;
pub mod topics;
#[cfg(test)]
mod tests;

use doc::{Block, Docs, Page};
use eframe::egui::{self, vec2, Align, Key, Modifiers, RichText};
use render::{Nav, Out, Request};
use std::collections::HashMap;

/// A place in the help: a page and, optionally, a heading on it.
#[derive(Clone, PartialEq, Debug)]
struct Loc {
    page: String,
    anchor: Option<String>,
}

impl Loc {
    fn parse(topic: &str) -> Loc {
        match topic.split_once('#') {
            Some((p, a)) => Loc { page: p.to_string(), anchor: Some(a.to_string()) },
            None => Loc { page: topic.to_string(), anchor: None },
        }
    }
}

/// What the help window asks the app to do.
pub enum Action {
    /// "Use in Custom filter" on an example.
    ApplyFilter(String),
}

/// Facts for the About window, from the app.
pub struct AboutInfo {
    pub renderer: &'static str,
    pub elevated: bool,
}

#[derive(Default)]
pub struct Help {
    pub open: bool,
    pub about_open: bool,
    docs: Option<Docs>,
    at: Option<Loc>,
    back: Vec<Loc>,
    forward: Vec<Loc>,
    /// Scroll to the anchor (or the top) on the next frame.
    jump: bool,
    query: String,
    focus_search: bool,
    images: HashMap<String, Option<egui::TextureHandle>>,
    /// The pointer is over the help window (so the map ignores its Back/Forward buttons).
    pub hovered: bool,
}

impl Help {
    fn docs(&mut self) -> &Docs {
        self.docs.get_or_insert_with(Docs::load)
    }

    /// Open the help at `topic` ("views/treemap.md" or "views/treemap.md#heading").
    pub fn show_topic(&mut self, topic: &str) {
        let loc = Loc::parse(topic);
        let loc = if self.docs().page(&loc.page).is_some() { loc } else { Loc::parse(doc::HOME) };
        self.go(loc);
        self.open = true;
        self.query.clear();
    }

    fn go(&mut self, loc: Loc) {
        if let Some(cur) = self.at.take() {
            if cur != loc {
                self.back.push(cur);
                self.forward.clear();
            }
        }
        self.at = Some(loc);
        self.jump = true;
    }

    fn step(&mut self, back: bool) {
        let (from, to) = if back { (&mut self.back, &mut self.forward) } else { (&mut self.forward, &mut self.back) };
        if let Some(loc) = from.pop() {
            if let Some(cur) = self.at.take() {
                to.push(cur);
            }
            self.at = Some(loc);
            self.jump = true;
        }
    }

    /// Draw the help and About windows. Call once per frame, before the panels.
    pub fn ui(&mut self, ctx: &egui::Context, about: &AboutInfo) -> Option<Action> {
        self.about_window(ctx, about);
        self.hovered = false;
        if !self.open {
            return None;
        }
        if self.at.is_none() {
            self.at = Some(Loc::parse(doc::HOME));
        }
        self.docs();
        let mut open = true;
        let mut action = None;
        let resp = egui::Window::new("dirmap Help")
            .id(egui::Id::new("help-window"))
            .open(&mut open)
            .default_size([1000.0, 720.0])
            .min_size([520.0, 360.0])
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| action = self.contents(ui));
        if let Some(r) = resp {
            self.hovered = r.response.contains_pointer();
        }
        // Keys for the help, while the pointer is over it: they then don't reach the map.
        if self.hovered {
            let (back, fwd, find) = ctx.input_mut(|i| {
                let mouse_back = i.pointer.button_pressed(egui::PointerButton::Extra1);
                let mouse_fwd = i.pointer.button_pressed(egui::PointerButton::Extra2);
                (
                    i.consume_key(Modifiers::ALT, Key::ArrowLeft) || mouse_back,
                    i.consume_key(Modifiers::ALT, Key::ArrowRight) || mouse_fwd,
                    i.consume_key(Modifiers::COMMAND, Key::F),
                )
            });
            if back {
                self.step(true);
            }
            if fwd {
                self.step(false);
            }
            if find {
                self.focus_search = true;
            }
        }
        if !open {
            self.open = false;
        }
        action
    }

    fn contents(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let docs = self.docs.take().unwrap();
        let mut nav: Option<Nav> = None;
        let mut request = None;

        // ---- top bar: back, forward, home, search ----
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.back.is_empty(), egui::Button::new("◀")).on_hover_text("Back (Alt+Left)").clicked() {
                self.step(true);
            }
            if ui.add_enabled(!self.forward.is_empty(), egui::Button::new("▶")).on_hover_text("Forward (Alt+Right)").clicked() {
                self.step(false);
            }
            if ui.button("🏠 Help Centre").clicked() {
                nav = Some(Nav::Page(doc::HOME.to_string(), None));
            }
            ui.separator();
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(egui::Id::new("help-search"))
                    .hint_text("🔍 Search help (Ctrl+F)")
                    .desired_width(260.0),
            );
            if std::mem::take(&mut self.focus_search) {
                search.request_focus();
            }
            if !self.query.is_empty() && ui.small_button("✖").on_hover_text("Clear the search").clicked() {
                self.query.clear();
            }
        });
        ui.separator();

        let at = self.at.clone().unwrap_or_else(|| Loc::parse(doc::HOME));

        // ---- sidebar: contents, or search results ----
        egui::Panel::left("help-nav").resizable(true).default_size(250.0).min_size(170.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                if self.query.trim().is_empty() {
                    contents_list(ui, &docs, &at.page, &mut nav);
                } else {
                    let hits = docs.search(&self.query);
                    ui.weak(match hits.len() {
                        0 => "No pages match. Try fewer or shorter words.".to_string(),
                        1 => "1 page".to_string(),
                        n => format!("{n} pages"),
                    });
                    for h in hits.iter().take(40) {
                        ui.add_space(4.0);
                        let p = docs.page(&h.path).unwrap();
                        let title = RichText::new(&p.title).strong();
                        if ui.selectable_label(at.page == h.path, title).clicked() {
                            nav = Some(Nav::Page(h.path.clone(), h.anchor.clone()));
                        }
                        if !p.section.is_empty() {
                            ui.label(RichText::new(&p.section).small().weak());
                        }
                        ui.label(RichText::new(&h.snippet).small());
                    }
                }
            });
        });

        // ---- the page ----
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(page) = docs.page(&at.page) else { return };
            let mut out = Out { page: &page.path, docs: &docs, images: &mut self.images, nav: None, request: None, anchors: Vec::new() };
            let mut area = egui::ScrollArea::vertical().auto_shrink(false).id_salt(("help-page", &page.path));
            if self.jump && at.anchor.is_none() {
                area = area.vertical_scroll_offset(0.0);
            }
            area.show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(860.0));
                ui.add_space(4.0);
                if !page.section.is_empty() {
                    ui.label(RichText::new(format!("{}  ›  {}", page.section, page.title)).small().weak());
                }
                let (title, rest) = split_title(&page.blocks);
                render::blocks(ui, &mut out, title);
                on_this_page(ui, page, &mut out);
                ui.add_space(6.0);
                render::blocks(ui, &mut out, rest);
                ui.add_space(16.0);
                ui.separator();
                prev_next(ui, &docs, &page.path, &mut out);
                ui.add_space(20.0);
                if self.jump {
                    if let Some(a) = &at.anchor {
                        if let Some((_, r)) = out.anchors.iter().find(|(id, _)| id == a) {
                            ui.scroll_to_rect(*r, Some(Align::TOP));
                        }
                    }
                }
            });
            self.jump = false;
            nav = nav.take().or(out.nav.take());
            request = out.request.take();
        });

        self.docs = Some(docs);
        match nav {
            Some(Nav::Page(p, a)) => {
                if self.docs().page(&p).is_some() {
                    self.go(Loc { page: p, anchor: a });
                }
            }
            Some(Nav::Url(u)) => ui.ctx().open_url(egui::OpenUrl::new_tab(u)),
            None => {}
        }
        request.map(|Request::ApplyFilter(p)| Action::ApplyFilter(p))
    }

    fn about_window(&mut self, ctx: &egui::Context, about: &AboutInfo) {
        if !self.about_open {
            return;
        }
        let mut open = true;
        let mut more = false;
        egui::Window::new("About dirmap")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.heading(format!("dirmap {}", env!("CARGO_PKG_VERSION")));
                ui.label("A disk usage visualiser for Windows, Linux and macOS.");
                ui.add_space(8.0);
                egui::Grid::new("about").num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
                    ui.label("Version");
                    ui.label(env!("CARGO_PKG_VERSION"));
                    ui.end_row();
                    ui.label("System");
                    ui.label(format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH));
                    ui.end_row();
                    ui.label("Drawing");
                    ui.label(about.renderer);
                    ui.end_row();
                    ui.label("Administrator");
                    ui.label(if about.elevated { "Yes" } else { "No" });
                    ui.end_row();
                    ui.label("Settings folder");
                    let dir = eframe::storage_dir("dirmap").map(|d| d.display().to_string()).unwrap_or_else(|| "(none)".into());
                    ui.add(egui::Label::new(RichText::new(dir).monospace().size(11.5)).wrap());
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy details").on_hover_text("For a problem report").clicked() {
                        let text = format!(
                            "dirmap {} · {} {} · drawing: {} · administrator: {}",
                            env!("CARGO_PKG_VERSION"),
                            std::env::consts::OS,
                            std::env::consts::ARCH,
                            about.renderer,
                            if about.elevated { "yes" } else { "no" }
                        );
                        ui.ctx().copy_text(text);
                    }
                    if ui.button("More about dirmap…").clicked() {
                        more = true;
                    }
                });
            });
        if more {
            self.show_topic(topics::ABOUT);
        }
        if !open {
            self.about_open = false;
        }
    }
}

/// The page's own title, then the rest.
fn split_title(blocks: &[Block]) -> (&[Block], &[Block]) {
    match blocks.first() {
        Some(Block::Heading { level: 1, .. }) => blocks.split_at(1),
        _ => (&[], blocks),
    }
}

/// The table of contents in the sidebar, the section holding `current` opened.
fn contents_list(ui: &mut egui::Ui, docs: &Docs, current: &str, nav: &mut Option<Nav>) {
    if ui.selectable_label(current == doc::HOME, RichText::new("Help Centre").strong()).clicked() {
        *nav = Some(Nav::Page(doc::HOME.to_string(), None));
    }
    for s in &docs.sections {
        let here = s.pages.iter().any(|p| p == current);
        egui::CollapsingHeader::new(RichText::new(&s.title).strong())
            .id_salt(("help-sec", &s.title))
            .open(here.then_some(true))
            .default_open(false)
            .show(ui, |ui| {
                for p in &s.pages {
                    if ui.selectable_label(p == current, docs.title(p)).clicked() {
                        *nav = Some(Nav::Page(p.clone(), None));
                    }
                }
            });
    }
}

/// "On this page" links to the page's sections, for longer pages.
fn on_this_page(ui: &mut egui::Ui, page: &Page, out: &mut Out) {
    let secs: Vec<_> = page.headings.iter().filter(|h| h.0 == 2 && h.1 != "Related topics").collect();
    if secs.len() < 3 {
        return;
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("On this page:").weak());
        for (_, text, id) in secs {
            if ui.link(text).clicked() {
                out.nav = Some(Nav::Page(page.path.clone(), Some(id.clone())));
            }
        }
    });
}

fn prev_next(ui: &mut egui::Ui, docs: &Docs, path: &str, out: &mut Out) {
    let (prev, next) = docs.neighbours(path);
    ui.horizontal(|ui| {
        if let Some(p) = prev {
            if ui.link(format!("◀ {}", docs.title(p))).clicked() {
                out.nav = Some(Nav::Page(p.to_string(), None));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if let Some(n) = next {
                if ui.link(format!("{} ▶", docs.title(n))).clicked() {
                    out.nav = Some(Nav::Page(n.to_string(), None));
                }
            }
        });
    });
    let _ = vec2(0.0, 0.0);
}

/// A small "?" button that opens a help topic. Returns true when clicked.
pub fn link_button(ui: &mut egui::Ui, tip: &str) -> bool {
    ui.add(egui::Button::new(RichText::new("?").strong()).small().corner_radius(8.0))
        .on_hover_text(format!("{tip} (F1)"))
        .clicked()
}
