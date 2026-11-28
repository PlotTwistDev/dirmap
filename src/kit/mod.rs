//! Filescape's design kit: the shared spacing, type sizes, surface colours and the controls built
//! on them (toggle switches, segmented controls, setting rows, section cards, buttons, badges,
//! callouts, sliders with a value and reset). The Settings window is built entirely from these,
//! and they're meant for the rest of the app as it's brought into line.
//!
//! Everything takes its colours from [`crate::app::style`]'s palettes, so the light and dark
//! themes stay in step, and every interactive piece is a normal egui widget: it takes keyboard
//! focus with Tab, answers Space/Enter, shows a focus ring, and dims (and says why, where the
//! caller gives a reason) when disabled.

pub mod glossary;
pub mod icons;

use crate::app::style::{self, Pal};
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};
pub use glossary::glossary;
pub use icons::Icon;

// ---------- the scale ----------

/// Padding around a page's content.
pub const PAGE_PAD: i8 = 24;
/// Space above each section heading.
pub const SECTION_GAP: f32 = 22.0;
/// Inside a card.
pub const CARD_PAD_X: i8 = 16;
pub const CARD_PAD_Y: i8 = 6;
/// A setting row is at least this tall, so rows line up across pages.
pub const ROW_MIN_H: f32 = 44.0;
pub const RADIUS: u8 = 8;
pub const CONTROL_RADIUS: u8 = 6;

/// Type sizes: page title, section heading, setting label, supporting text, small print.
pub const TITLE: f32 = 21.0;
pub const HEADING: f32 = 14.5;
pub const LABEL: f32 = 13.5;
pub const DESC: f32 = 12.0;
pub const SMALL: f32 = 11.0;

/// The meaning a colour carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Neutral,
    Accent,
    Success,
    Warn,
    Danger,
}

/// Surface and status colours for the theme a `Ui` is drawn in.
#[derive(Clone, Copy)]
pub struct Tokens {
    pub dark: bool,
    pub pal: Pal,
    /// Behind a page's sections.
    pub content: Color32,
    /// The navigation column.
    pub sidebar: Color32,
    /// A section's card.
    pub card: Color32,
    pub card_border: Color32,
    /// Hairlines between rows in a card.
    pub divider: Color32,
    /// Sunken areas: segmented control tracks, wells.
    pub track: Color32,
    /// The chosen segment, raised from the track.
    pub raised: Color32,
    pub accent: Color32,
    /// Tinted backgrounds for the selected navigation item and accent callouts.
    pub accent_soft: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    pub success: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub text: Color32,
    pub strong: Color32,
    pub weak: Color32,
    /// Text of disabled controls: dimmer than weak, but still readable.
    pub disabled: Color32,
}

pub fn tokens(ui: &Ui) -> Tokens {
    tokens_for(ui.visuals().dark_mode)
}

pub fn tokens_for(dark: bool) -> Tokens {
    let p = if dark { style::DARK } else { style::LIGHT };
    if dark {
        Tokens {
            dark,
            pal: p,
            content: p.chrome,
            sidebar: p.side,
            card: p.paper,
            card_border: Color32::from_rgb(0x30, 0x32, 0x37),
            divider: Color32::from_rgb(0x33, 0x35, 0x3a),
            track: p.well,
            raised: p.hover,
            accent: p.accent,
            accent_soft: p.sel,
            on_accent: Color32::WHITE,
            success: Color32::from_rgb(0x4c, 0xc3, 0x8a),
            warn: Color32::from_rgb(0xe8, 0xaa, 0x4e),
            danger: Color32::from_rgb(0xf0, 0x74, 0x7a),
            text: p.text,
            strong: p.strong,
            weak: p.weak,
            disabled: Color32::from_rgb(0x6c, 0x6f, 0x77),
        }
    } else {
        Tokens {
            dark,
            pal: p,
            content: Color32::from_rgb(0xf4, 0xf5, 0xf7),
            sidebar: p.chrome,
            card: p.paper,
            card_border: Color32::from_rgb(0xdf, 0xe1, 0xe6),
            divider: Color32::from_rgb(0xe8, 0xe9, 0xec),
            track: Color32::from_rgb(0xe6, 0xe8, 0xec),
            raised: Color32::WHITE,
            accent: p.accent,
            accent_soft: p.sel,
            on_accent: Color32::WHITE,
            success: Color32::from_rgb(0x1a, 0x7f, 0x4b),
            warn: Color32::from_rgb(0x9a, 0x5d, 0x00),
            danger: Color32::from_rgb(0xc2, 0x36, 0x3a),
            text: p.text,
            strong: p.strong,
            weak: p.weak,
            disabled: Color32::from_rgb(0x8c, 0x90, 0x98),
        }
    }
}

impl Tokens {
    pub fn tone(&self, t: Tone) -> Color32 {
        match t {
            Tone::Neutral => self.weak,
            Tone::Accent => self.accent,
            Tone::Success => self.success,
            Tone::Warn => self.warn,
            Tone::Danger => self.danger,
        }
    }

    /// A faint wash of a tone over the card colour, for badges and callouts.
    pub fn wash(&self, t: Tone) -> Color32 {
        style::blend(self.card, self.tone(t), if self.dark { 0.16 } else { 0.10 })
    }
}

/// The focus ring around a focused control.
pub fn focus_ring(ui: &Ui, rect: Rect, resp: &egui::Response, radius: f32) {
    if resp.has_focus() {
        let t = tokens(ui);
        ui.painter().rect_stroke(rect.expand(2.0), radius + 2.0, Stroke::new(2.0, t.accent), StrokeKind::Outside);
    }
}

// ---------- page structure ----------

/// A page's title and a sentence on what it's for.
pub fn page_header(ui: &mut Ui, title: &str, desc: &str) {
    let t = tokens(ui);
    ui.label(RichText::new(title).size(TITLE).strong().color(t.strong));
    ui.add_space(2.0);
    ui.label(RichText::new(desc).size(DESC + 0.5).color(t.weak));
    ui.add_space(6.0);
}

/// How a section is drawn when a search result points at it.
#[derive(Clone, Copy, Default)]
pub struct Focus {
    /// Outline the card in the accent colour.
    pub highlight: bool,
    /// Scroll it into view (once).
    pub scroll: bool,
}

/// A section: an icon and heading, then its rows in a card.
pub fn section<R>(ui: &mut Ui, icon: Icon, title: &str, focus: Focus, add: impl FnOnce(&mut Ui) -> R) -> R {
    section_with(ui, icon, title, None, focus, add)
}

/// [`section`] with a note (such as a badge) at the right of the heading.
pub fn section_with<R>(ui: &mut Ui, icon: Icon, title: &str, aside: Option<(&str, Tone)>, focus: Focus, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = tokens(ui);
    ui.add_space(SECTION_GAP - 8.0);
    let head = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        icons::show(ui, icon, 16.0, t.weak);
        ui.label(RichText::new(title).size(HEADING).strong().color(t.strong));
        if let Some((text, tone)) = aside {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| badge(ui, text, tone));
        }
    });
    ui.add_space(6.0);
    let stroke = if focus.highlight { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, t.card_border) };
    let r = card_frame(&t).stroke(stroke).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    if focus.scroll {
        head.response.union(r.response.clone()).scroll_to_me(Some(Align::TOP));
    }
    r.inner
}

pub fn card_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(CARD_PAD_X, CARD_PAD_Y))
}

/// A hairline between rows of a card.
pub fn divider(ui: &mut Ui) {
    let t = tokens(ui);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, t.divider));
}

fn label_block(ui: &mut Ui, label: &str, desc: Option<&str>, enabled: bool) {
    let t = tokens(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(egui::Label::new(RichText::new(label).size(LABEL).color(if enabled { t.strong } else { t.disabled })).wrap());
        if let Some(d) = desc {
            ui.add(egui::Label::new(RichText::new(d).size(DESC).color(if enabled { t.weak } else { t.disabled })).wrap());
        }
    });
}

/// One setting: its name and a line on what it does at the left, the control at the right,
/// both centred on the row.
pub fn row<R>(ui: &mut Ui, label: &str, desc: Option<&str>, control: impl FnOnce(&mut Ui) -> R) -> R {
    let enabled = ui.is_enabled();
    let w = ui.available_width();
    let top = ui.cursor().min;
    let left_w = (w * 0.6).max(160.0);
    // Measure the name and description, to centre them on the row.
    let lay = |text: &str, size: f32| ui.painter().layout(text.to_string(), egui::FontId::proportional(size), Color32::WHITE, left_w).size().y;
    let lh = lay(label, LABEL) + desc.map_or(0.0, |d| 2.0 + lay(d, DESC));
    let h = (lh + 20.0).max(ROW_MIN_H);
    ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_size(top + vec2(0.0, ((h - lh) / 2.0).floor()), vec2(left_w, lh + 4.0))).layout(Layout::top_down(Align::Min)), |ui| {
        label_block(ui, label, desc, enabled)
    });
    let right = Rect::from_min_max(egui::pos2(top.x + left_w + 12.0, top.y), egui::pos2(top.x + w, top.y + h));
    let r = ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)), control);
    let h = h.max(r.response.rect.height() + 12.0);
    ui.allocate_rect(Rect::from_min_size(top, vec2(w, h)), Sense::hover());
    r.inner
}

/// A setting whose control is too wide to sit beside its name (a segmented control, a slider,
/// a list): the name and description above, the control below.
pub fn stacked<R>(ui: &mut Ui, label: &str, desc: Option<&str>, control: impl FnOnce(&mut Ui) -> R) -> R {
    let enabled = ui.is_enabled();
    ui.add_space(8.0);
    label_block(ui, label, desc, enabled);
    ui.add_space(6.0);
    let r = control(ui);
    ui.add_space(10.0);
    r
}

/// A setting with an on/off switch.
pub fn toggle_row(ui: &mut Ui, label: &str, desc: Option<&str>, on: &mut bool) -> egui::Response {
    row(ui, label, desc, |ui| toggle(ui, on))
}

/// Small print below a control or at the end of a card.
pub fn note(ui: &mut Ui, text: impl Into<String>) {
    let t = tokens(ui);
    ui.add(egui::Label::new(RichText::new(text.into()).size(DESC).color(t.weak)).wrap());
}

/// Says why the controls just above are unavailable, so a dimmed control is never a mystery.
pub fn why_disabled(ui: &mut Ui, text: &str) {
    let t = tokens(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icons::show(ui, Icon::Info, 13.0, t.weak);
        ui.add(egui::Label::new(RichText::new(text).size(DESC).color(t.weak)).wrap());
    });
}

// ---------- controls ----------

/// An on/off switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = vec2(38.0, 21.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let t = tokens(ui);
        let k = ui.ctx().animate_bool_with_time(resp.id, *on, ui.style().animation_time.min(0.12));
        let enabled = ui.is_enabled();
        let off_fill = if t.dark { Color32::from_rgb(0x4a, 0x4d, 0x55) } else { Color32::from_rgb(0xc4, 0xc8, 0xcf) };
        let mut fill = style::blend(off_fill, t.accent, k);
        if resp.hovered() && enabled {
            fill = style::blend(fill, t.strong, 0.08);
        }
        if !enabled {
            fill = style::blend(fill, t.card, 0.55);
        }
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, radius, fill);
        let knob_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), k);
        let knob = if enabled { Color32::WHITE } else { style::blend(Color32::WHITE, t.card, 0.4) };
        ui.painter().circle_filled(egui::pos2(knob_x, rect.center().y), radius - 3.0, knob);
        focus_ring(ui, rect, &resp, radius);
    }
    resp
}

/// A row of mutually exclusive choices. `fill` stretches it across the available width.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)], fill: bool) -> egui::Response {
    let t = tokens(ui);
    let enabled = ui.is_enabled();
    let font = egui::FontId::proportional(LABEL - 0.5);
    let pad = 14.0;
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, s)| ui.painter().layout_no_wrap(s.to_string(), font.clone(), t.text).size().x + pad * 2.0)
        .collect();
    let natural: f32 = widths.iter().sum::<f32>() + 6.0;
    let total = if fill { ui.available_width().max(natural) } else { natural };
    let h = 30.0;
    let (rect, mut resp) = ui.allocate_exact_size(vec2(total, h), Sense::hover());
    ui.painter().rect_filled(rect, CONTROL_RADIUS, t.track);
    let scale = (total - 6.0) / (natural - 6.0);
    let mut x = rect.left() + 3.0;
    for (i, (v, text)) in options.iter().enumerate() {
        let w = widths[i] * scale;
        let seg = Rect::from_min_size(egui::pos2(x, rect.top() + 3.0), vec2(w, h - 6.0));
        x += w;
        let r = ui.interact(seg, resp.id.with(i), Sense::click());
        let selected = *value == *v;
        if r.clicked() && !selected {
            *value = *v;
            resp.mark_changed();
        }
        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, selected, *text));
        if selected {
            ui.painter().rect_filled(seg, CONTROL_RADIUS - 1, t.raised);
            if !t.dark {
                ui.painter().rect_stroke(seg, CONTROL_RADIUS - 1, Stroke::new(1.0, t.card_border), StrokeKind::Inside);
            }
        } else if r.hovered() && enabled {
            ui.painter().rect_filled(seg, CONTROL_RADIUS - 1, style::blend(t.track, t.raised, 0.5));
        }
        let color = if !enabled {
            t.disabled
        } else if selected {
            t.strong
        } else {
            t.weak
        };
        ui.painter().text(seg.center(), egui::Align2::CENTER_CENTER, *text, font.clone(), color);
        focus_ring(ui, seg, &r, (CONTROL_RADIUS - 1) as f32);
    }
    resp
}

/// How a button looks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The main action: accent fill.
    Primary,
    /// An ordinary action: outlined.
    Secondary,
    /// A quiet action: text only until hovered.
    Ghost,
    /// A destructive action: outlined in red.
    Danger,
}

/// A button with an optional icon.
pub fn button(ui: &mut Ui, kind: Kind, icon: Option<Icon>, text: &str) -> egui::Response {
    let t = tokens(ui);
    let enabled = ui.is_enabled();
    let font = egui::FontId::proportional(LABEL - 0.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
    let icon_w = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let pad = if kind == Kind::Ghost { 8.0 } else { 14.0 };
    let size = vec2((galley.size().x + icon_w + pad * 2.0).max(if kind == Kind::Primary { 72.0 } else { 0.0 }), 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered() && enabled;
        let pressed = resp.is_pointer_button_down_on() && enabled;
        let (fill, stroke, fg) = match kind {
            Kind::Primary => {
                let mut f = t.accent;
                if hovered {
                    f = style::blend(f, Color32::WHITE, 0.10);
                }
                if pressed {
                    f = style::blend(t.accent, Color32::BLACK, 0.12);
                }
                if !enabled {
                    f = style::blend(t.accent, t.card, 0.6);
                }
                (f, Stroke::NONE, if enabled { t.on_accent } else { style::blend(t.on_accent, t.card, 0.35) })
            }
            Kind::Secondary | Kind::Danger => {
                let edge = if kind == Kind::Danger { style::blend(t.card_border, t.danger, 0.7) } else if t.dark { Color32::from_rgb(0x45, 0x48, 0x50) } else { Color32::from_rgb(0xc9, 0xcd, 0xd4) };
                let fill = if pressed { t.track } else if hovered { t.pal.hover } else { t.card };
                let fg = if !enabled { t.disabled } else if kind == Kind::Danger { t.danger } else { t.strong };
                (fill, Stroke::new(1.0, if enabled { edge } else { t.divider }), fg)
            }
            Kind::Ghost => {
                let fill = if hovered { t.pal.hover } else { Color32::TRANSPARENT };
                (fill, Stroke::NONE, if !enabled { t.disabled } else if hovered { t.strong } else { t.text })
            }
        };
        ui.painter().rect(rect, CONTROL_RADIUS, fill, stroke, StrokeKind::Inside);
        let mut x = rect.center().x - (galley.size().x + icon_w) / 2.0;
        if let Some(i) = icon {
            icons::paint(ui.painter(), Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0)), i, fg);
            x += icon_w;
        }
        ui.painter().galley_with_override_text_color(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
        focus_ring(ui, rect, &resp, CONTROL_RADIUS as f32);
    }
    resp
}

/// A Secondary button with a dropdown arrow joined to it, divided by a line: the main part does
/// the usual thing, the arrow offers others. Returns (main, arrow).
pub fn split_button(ui: &mut Ui, icon: Option<Icon>, text: &str, arrow_tip: &str) -> (egui::Response, egui::Response) {
    let t = tokens(ui);
    let enabled = ui.is_enabled();
    let font = egui::FontId::proportional(LABEL - 0.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
    let icon_w = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let main_w = galley.size().x + icon_w + 14.0 * 2.0 - 4.0;
    let arrow_w = 28.0;
    let (rect, _) = ui.allocate_exact_size(vec2(main_w + arrow_w, 30.0), Sense::hover());
    let main_rect = Rect::from_min_size(rect.min, vec2(main_w, rect.height()));
    let arrow_rect = Rect::from_min_max(egui::pos2(main_rect.max.x, rect.min.y), rect.max);
    let main = ui.interact(main_rect, ui.id().with(("split-main", text)), Sense::click());
    let arrow = ui.interact(arrow_rect, ui.id().with(("split-arrow", text)), Sense::click());
    main.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    arrow.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, arrow_tip));
    if ui.is_rect_visible(rect) {
        let edge = if t.dark { Color32::from_rgb(0x45, 0x48, 0x50) } else { Color32::from_rgb(0xc9, 0xcd, 0xd4) };
        let fg = if enabled { t.strong } else { t.disabled };
        let p = ui.painter();
        p.rect(rect, CONTROL_RADIUS, t.card, Stroke::NONE, StrokeKind::Inside);
        for (r, resp, round) in [
            (main_rect, &main, egui::CornerRadius { nw: CONTROL_RADIUS, sw: CONTROL_RADIUS, ne: 0, se: 0 }),
            (arrow_rect, &arrow, egui::CornerRadius { nw: 0, sw: 0, ne: CONTROL_RADIUS, se: CONTROL_RADIUS }),
        ] {
            if enabled && resp.is_pointer_button_down_on() {
                p.rect_filled(r, round, t.track);
            } else if enabled && resp.hovered() {
                p.rect_filled(r, round, t.pal.hover);
            }
        }
        p.rect(rect, CONTROL_RADIUS, Color32::TRANSPARENT, Stroke::new(1.0, if enabled { edge } else { t.divider }), StrokeKind::Inside);
        p.line_segment([egui::pos2(arrow_rect.min.x, rect.min.y + 6.0), egui::pos2(arrow_rect.min.x, rect.max.y - 6.0)], Stroke::new(1.0, edge));
        let mut x = main_rect.center().x - (galley.size().x + icon_w) / 2.0;
        if let Some(i) = icon {
            icons::paint(p, Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0)), i, fg);
            x += icon_w;
        }
        p.galley_with_override_text_color(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
        icons::paint(p, Rect::from_center_size(arrow_rect.center(), vec2(12.0, 12.0)), Icon::ChevronDown, fg);
        focus_ring(ui, main_rect, &main, CONTROL_RADIUS as f32);
        focus_ring(ui, arrow_rect, &arrow, CONTROL_RADIUS as f32);
    }
    (main, arrow.on_hover_text(arrow_tip))
}

/// A square button showing only an icon (give it a tooltip).
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> egui::Response {
    let t = tokens(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let enabled = ui.is_enabled();
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, CONTROL_RADIUS, t.pal.hover);
    }
    let fg = if !enabled { t.disabled } else if resp.hovered() { t.strong } else { t.weak };
    icons::paint(ui.painter(), rect.shrink(6.0), icon, fg);
    focus_ring(ui, rect, &resp, CONTROL_RADIUS as f32);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tip));
    resp.on_hover_text(tip)
}

/// A small icon button that puts one setting back to its default; it only shows when the
/// setting differs from it, and keeps its space otherwise so nothing shifts.
pub fn reset_button(ui: &mut Ui, changed: bool, default_text: &str) -> bool {
    if changed {
        icon_button(ui, Icon::Reset, &format!("Reset to the default ({default_text})")).clicked()
    } else {
        ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
        false
    }
}

/// A rounded label for status ("Active", "Needs a rescan").
pub fn badge(ui: &mut Ui, text: &str, tone: Tone) -> egui::Response {
    let t = tokens(ui);
    let font = egui::FontId::proportional(SMALL + 0.5);
    let fg = t.tone(tone);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let size = galley.size() + vec2(16.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, size.y / 2.0, t.wash(tone));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

/// A tinted box with an icon and a message: information, a warning, or a result.
pub fn callout(ui: &mut Ui, tone: Tone, text: impl Into<String>) {
    let t = tokens(ui);
    let icon = match tone {
        Tone::Warn | Tone::Danger => Icon::Warning,
        Tone::Success => Icon::Check,
        _ => Icon::Info,
    };
    egui::Frame::new()
        .fill(t.wash(tone))
        .corner_radius(CornerRadius::same(CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::show(ui, icon, 15.0, t.tone(tone));
                ui.add(egui::Label::new(RichText::new(text.into()).size(DESC + 0.5).color(t.text)).wrap());
            });
        });
}

/// A thin progress bar.
pub fn progress(ui: &mut Ui, frac: f32, tone: Tone) {
    let t = tokens(ui);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, t.track);
    let mut done = rect;
    done.set_width(rect.width() * frac.clamp(0.0, 1.0));
    ui.painter().rect_filled(done, 3.0, t.tone(tone));
}

/// A text field in the kit's size: a sunken well with a hairline border that turns accent while
/// typing.
pub fn field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    let t = tokens(ui);
    let id = ui.next_auto_id();
    let focused = ui.memory(|m| m.has_focus(id));
    let well = if t.dark { t.pal.well } else { Color32::from_rgb(0xf7, 0xf8, 0xfa) };
    let edge = if focused {
        Stroke::new(1.5, t.accent)
    } else {
        Stroke::new(1.0, if t.dark { Color32::from_rgb(0x3a, 0x3c, 0x42) } else { Color32::from_rgb(0xcf, 0xd3, 0xd9) })
    };
    ui.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(hint)
            .desired_width(width)
            .frame(egui::Frame::new().fill(well).stroke(edge).corner_radius(CONTROL_RADIUS).inner_margin(Margin::symmetric(8, 6)))
            .font(egui::FontId::proportional(LABEL - 0.5)),
    )
}

/// A key and its value, for detail lists such as the licence's.
pub fn kv(ui: &mut Ui, key: &str, value: impl Into<RichText>) {
    let t = tokens(ui);
    ui.label(RichText::new(key).size(DESC + 0.5).color(t.weak));
    ui.add(egui::Label::new(value.into().size(LABEL - 0.5).color(t.strong)).wrap());
    ui.end_row();
}

/// A navigation item in a sidebar.
pub fn nav_item(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let t = tokens(ui);
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label));
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CONTROL_RADIUS, t.accent_soft);
        let bar = Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(3.0, rect.height() - 16.0));
        p.rect_filled(bar, 1.5, t.accent);
    } else if resp.hovered() {
        p.rect_filled(rect, CONTROL_RADIUS, t.pal.hover);
    }
    let fg = if selected { t.strong } else { t.text };
    let ic = Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(p, ic, icon, if selected { t.accent } else { t.weak });
    p.text(egui::pos2(rect.left() + 38.0, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(LABEL), fg);
    focus_ring(ui, rect, &resp, CONTROL_RADIUS as f32);
    resp
}

/// A search box with a magnifier and a clear button.
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str) -> egui::Response {
    let t = tokens(ui);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
    let id = ui.id().with("search-edit");
    let focused = ui.memory(|m| m.has_focus(id));
    let stroke = if focused { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, t.card_border) };
    ui.painter().rect(rect, CONTROL_RADIUS, t.pal.well, stroke, StrokeKind::Inside);
    icons::paint(ui.painter(), Rect::from_center_size(egui::pos2(rect.left() + 16.0, rect.center().y), vec2(14.0, 14.0)), Icon::Search, t.weak);
    let inner = Rect::from_min_max(rect.min + vec2(30.0, 7.0), rect.max - vec2(26.0, 6.0));
    let resp = ui.put(
        inner,
        egui::TextEdit::singleline(text).id(id).hint_text(hint).frame(egui::Frame::NONE).desired_width(w - 58.0).font(egui::FontId::proportional(LABEL - 0.5)),
    );
    if !text.is_empty() {
        let c = Rect::from_center_size(egui::pos2(rect.right() - 15.0, rect.center().y), vec2(20.0, 20.0));
        let r = ui.interact(c, id.with("clear"), Sense::click()).on_hover_text("Clear the search");
        icons::paint(ui.painter(), c.shrink(5.0), Icon::Close, if r.hovered() { t.strong } else { t.weak });
        if r.clicked() {
            text.clear();
            resp.request_focus();
        }
    }
    resp
}

/// A selectable card with a picture above its name (themes, tile styles, presets).
pub fn choice_card(ui: &mut Ui, selected: bool, size: Vec2, label: &str, picture: impl FnOnce(&egui::Painter, Rect)) -> egui::Response {
    let t = tokens(ui);
    let label_h = 22.0;
    let (rect, resp) = ui.allocate_exact_size(size + vec2(0.0, label_h), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), selected, label));
    let pic = Rect::from_min_size(rect.min, size);
    let p = ui.painter();
    picture(&p.with_clip_rect(pic.shrink(1.0)), pic.shrink(3.0));
    let enabled = ui.is_enabled();
    let stroke = if selected {
        Stroke::new(2.0, t.accent)
    } else if resp.hovered() && enabled {
        Stroke::new(1.0, t.weak)
    } else {
        Stroke::new(1.0, t.card_border)
    };
    p.rect_stroke(pic, RADIUS, stroke, StrokeKind::Inside);
    if !enabled {
        p.rect_filled(pic, RADIUS, t.card.gamma_multiply(0.55));
    }
    let fg = if !enabled { t.disabled } else if selected { t.accent } else { t.text };
    p.text(egui::pos2(pic.center().x, pic.bottom() + label_h / 2.0 + 1.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(LABEL - 0.5), fg);
    focus_ring(ui, pic, &resp, RADIUS as f32);
    resp
}

/// A slider with its value (editable for precise input) and a reset button.
pub struct SliderRow<'a> {
    pub label: &'a str,
    pub desc: Option<&'a str>,
    pub value: &'a mut f32,
    pub range: std::ops::RangeInclusive<f32>,
    pub default: f32,
    /// Shows and reads the value: `to` turns the stored value into the shown one, `from` back.
    pub scale: (fn(f32) -> f64, fn(f64) -> f32),
    pub suffix: &'a str,
    pub decimals: usize,
    pub logarithmic: bool,
}

impl SliderRow<'_> {
    pub fn show(self, ui: &mut Ui) -> egui::Response {
        let SliderRow { label, desc, value, range, default, scale, suffix, decimals, logarithmic } = self;
        let enabled = ui.is_enabled();
        let t = tokens(ui);
        ui.add_space(8.0);
        let mut changed = false;
        ui.horizontal(|ui| {
            label_block(ui, label, desc, enabled);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let def_text = format!("{:.*}{}", decimals, (scale.0)(default), suffix);
                if reset_button(ui, (*value - default).abs() > 1e-4, &def_text) {
                    *value = default;
                    changed = true;
                }
                let (lo, hi) = ((scale.0)(*range.start()), (scale.0)(*range.end()));
                let mut shown = (scale.0)(*value);
                let step = 10f64.powi(-(decimals as i32));
                let r = ui.add(
                    egui::DragValue::new(&mut shown)
                        .range(lo..=hi)
                        .speed(step.max((hi - lo) / 300.0))
                        .max_decimals(decimals)
                        .min_decimals(decimals)
                        .suffix(suffix),
                );
                if r.changed() {
                    *value = (scale.1)(shown).clamp(*range.start(), *range.end());
                    changed = true;
                }
            });
        });
        ui.add_space(2.0);
        let w = ui.available_width();
        ui.spacing_mut().slider_width = w - 4.0;
        ui.visuals_mut().selection.bg_fill = if enabled { t.accent } else { t.disabled };
        let mut s = egui::Slider::new(value, range).show_value(false).trailing_fill(true);
        if logarithmic {
            s = s.logarithmic(true);
        }
        let mut resp = ui.add(s);
        ui.add_space(8.0);
        if changed {
            resp.mark_changed();
        }
        resp
    }
}

/// Identity mapping for [`SliderRow::scale`].
pub fn same(v: f32) -> f64 {
    v as f64
}
pub fn same_back(v: f64) -> f32 {
    v as f32
}
/// 0–1 shown as 0–100.
pub fn pct(v: f32) -> f64 {
    (v as f64 * 100.0).round()
}
pub fn pct_back(v: f64) -> f32 {
    (v / 100.0) as f32
}
