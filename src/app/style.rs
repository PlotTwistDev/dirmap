//! dirmap's look: one palette per theme, applied to egui's visuals, plus the few drawing
//! helpers the Files workspace and the analysis dock share (section titles, tool groups,
//! hairlines). Three surface levels (chrome, sidebar, content), one restrained accent, no
//! gradients or shadows except on popups.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// Colours for one theme.
#[derive(Clone, Copy)]
pub(crate) struct Pal {
    /// Toolbar, tab strip and status bar.
    pub chrome: Color32,
    /// The sidebar.
    pub side: Color32,
    /// The list and the dock: the "paper".
    pub paper: Color32,
    /// Every second row.
    pub stripe: Color32,
    /// Text fields and wells.
    pub well: Color32,
    pub hairline: Color32,
    /// A row or button under the pointer.
    pub hover: Color32,
    pub accent: Color32,
    /// Selected rows while the list has the keyboard, and when it doesn't.
    pub sel: Color32,
    pub sel_dim: Color32,
    pub text: Color32,
    pub strong: Color32,
    pub weak: Color32,
}

pub(crate) const DARK: Pal = Pal {
    chrome: Color32::from_rgb(0x1b, 0x1c, 0x1f),
    side: Color32::from_rgb(0x20, 0x21, 0x25),
    paper: Color32::from_rgb(0x26, 0x27, 0x2b),
    stripe: Color32::from_rgb(0x29, 0x2a, 0x2f),
    well: Color32::from_rgb(0x15, 0x16, 0x18),
    hairline: Color32::from_rgb(0x36, 0x38, 0x3d),
    hover: Color32::from_rgb(0x31, 0x33, 0x39),
    accent: Color32::from_rgb(0x4d, 0x8e, 0xf7),
    sel: Color32::from_rgb(0x2c, 0x45, 0x6e),
    sel_dim: Color32::from_rgb(0x3a, 0x3d, 0x44),
    text: Color32::from_rgb(0xdc, 0xdd, 0xe0),
    strong: Color32::from_rgb(0xf4, 0xf5, 0xf7),
    weak: Color32::from_rgb(0x96, 0x99, 0xa1),
};

pub(crate) const LIGHT: Pal = Pal {
    chrome: Color32::from_rgb(0xe9, 0xea, 0xed),
    side: Color32::from_rgb(0xf2, 0xf3, 0xf5),
    paper: Color32::from_rgb(0xff, 0xff, 0xff),
    stripe: Color32::from_rgb(0xf7, 0xf8, 0xfa),
    well: Color32::from_rgb(0xff, 0xff, 0xff),
    hairline: Color32::from_rgb(0xd6, 0xd8, 0xdd),
    hover: Color32::from_rgb(0xec, 0xee, 0xf2),
    accent: Color32::from_rgb(0x1f, 0x63, 0xd6),
    sel: Color32::from_rgb(0xd3, 0xe2, 0xfb),
    sel_dim: Color32::from_rgb(0xe2, 0xe4, 0xe9),
    text: Color32::from_rgb(0x1f, 0x22, 0x27),
    strong: Color32::from_rgb(0x0b, 0x0c, 0x0e),
    weak: Color32::from_rgb(0x5f, 0x64, 0x6d),
};

/// The palette for the theme `ui` is drawn in.
pub(crate) fn pal(ui: &egui::Ui) -> Pal {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

pub(crate) const RADIUS: u8 = 5;

/// egui's visuals built from a palette.
pub(crate) fn visuals(dark: bool) -> egui::Visuals {
    let p = if dark { DARK } else { LIGHT };
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = CornerRadius::same(RADIUS);
    v.panel_fill = p.chrome;
    v.window_fill = p.paper;
    v.window_stroke = Stroke::new(1.0, p.hairline);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.faint_bg_color = p.stripe;
    v.extreme_bg_color = p.well;
    v.code_bg_color = p.sel_dim;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.sel;
    v.selection.stroke = Stroke::new(1.0, p.strong);
    v.weak_text_color = Some(p.weak);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.paper;
    w.noninteractive.weak_bg_fill = p.paper;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.hairline);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    // Buttons rest flat on the chrome and gain a fill on hover, as in Explorer and Finder.
    w.inactive.bg_fill = p.well;
    w.inactive.weak_bg_fill = blend(p.chrome, p.hover, 0.65);
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.hairline);
    w.hovered.fg_stroke = Stroke::new(1.5, p.strong);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.sel;
    w.active.weak_bg_fill = p.sel;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.strong);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0, p.hairline);
    w.open.fg_stroke = Stroke::new(1.0, p.strong);
    w.open.corner_radius = r;
    v
}

/// Install both palettes; the theme setting then picks between them.
pub(crate) fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Dark, visuals(true));
    ctx.set_visuals_of(egui::Theme::Light, visuals(false));
    ctx.all_styles_mut(|s| s.spacing.menu_margin = egui::Margin::same(6));
}

/// Roomier controls for the File Manager's toolbar and dock (Storage Analysis keeps egui's
/// tighter defaults, as its toolbar is full).
pub(crate) fn roomy(ui: &mut egui::Ui) {
    let s = ui.spacing_mut();
    s.button_padding = egui::vec2(7.0, 3.0);
    s.interact_size.y = 24.0;
}

/// Mix two colours: `t` = 0 gives `a`, 1 gives `b`.
pub(crate) fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A sidebar or dock section title: small caps, letter-spaced, weak.
pub(crate) fn section_title(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let spaced: String = text.to_uppercase().chars().flat_map(|c| [c, '\u{200A}']).collect();
    ui.add(egui::Label::new(egui::RichText::new(spaced).size(10.5).color(pal(ui).weak).strong()).sense(egui::Sense::click()))
}

/// A row of tool buttons that reads as one group: a faint well behind them.
pub(crate) fn tool_group<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = pal(ui);
    egui::Frame::new()
        .fill(blend(p.chrome, p.hover, 0.35))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            ui.horizontal(add).inner
        })
        .inner
}

/// A flat icon button for toolbars: no frame until hovered.
pub(crate) fn icon_button(ui: &mut egui::Ui, enabled: bool, icon: &str, tip: impl Into<egui::WidgetText>) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(icon).size(14.0)).frame_when_inactive(false).min_size(egui::vec2(26.0, 22.0));
    ui.add_enabled(enabled, b).on_hover_text(tip)
}

/// A flat toggle button for toolbars: filled with the selection tint while on.
pub(crate) fn toggle_button(ui: &mut egui::Ui, on: bool, text: impl Into<egui::WidgetText>, tip: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Button::selectable(on, text)).on_hover_text(tip)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn lum(c: Color32) -> f64 {
        let f = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_is_readable_on_every_surface() {
        for p in [DARK, LIGHT] {
            for bg in [p.chrome, p.side, p.paper, p.stripe, p.hover, p.sel, p.sel_dim] {
                assert!(contrast(p.text, bg) >= 4.5, "text {:?} on {:?}: {:.2}", p.text, bg, contrast(p.text, bg));
            }
            for bg in [p.chrome, p.side, p.paper, p.stripe] {
                assert!(contrast(p.weak, bg) >= 3.0, "weak {:?} on {:?}: {:.2}", p.weak, bg, contrast(p.weak, bg));
            }
            assert!(contrast(p.accent, p.paper) >= 3.0, "accent on paper");
        }
    }

    #[test]
    fn surfaces_are_distinct() {
        for p in [DARK, LIGHT] {
            assert!(contrast(p.chrome, p.paper) > 1.1);
            assert!(p.hover != p.stripe);
        }
    }
}
