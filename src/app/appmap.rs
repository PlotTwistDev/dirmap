//! Applications in Storage Analysis: a Mac app is one thing to the person looking at the map,
//! so everything inside it takes the app's colour and counts as Applications, until the view
//! goes inside the app to see what it's made of.

use super::*;

impl App {
    /// The application the map is zoomed into (or into something inside), by its path in the
    /// tree the map shows.
    pub(super) fn zoom_app(&self) -> Option<Vec<u32>> {
        let t = self.shown_tree()?;
        crate::apps::app_on_path(&t, &self.zoom).map(|k| self.zoom[..=k].to_vec())
    }

    /// The colour every file of an application shown whole takes: an indigo that no file type
    /// is given first, so apps stand apart from folders of pictures or documents.
    pub(super) fn app_color(&self) -> Color32 {
        match self.view().tile_style {
            TileStyle::Modern => Color32::from_rgb(96, 100, 222),
            _ => Color32::from_rgb(82, 70, 204),
        }
    }

    /// Whether the item at this path in the whole scan is (inside) an application that is
    /// shown whole: the view isn't inside it.
    pub(super) fn whole_app_at(&self, full: &[u32]) -> bool {
        let Some(t) = self.tree.as_deref() else { return false };
        let Some(k) = crate::apps::app_on_path(t, full) else { return false };
        let zoom = self.full_of(&self.zoom);
        !(zoom.len() > k && zoom[..=k] == full[..=k])
    }

    /// A scanned item's icon in the analysis lists: an application's own, else by type.
    pub(super) fn paint_node_icon(&mut self, ui: &egui::Ui, rect: Rect, n: &Node, view: &[u32], tint: Color32) {
        if crate::apps::node_is_app(n) {
            let full = self.full_path(view);
            self.icons.paint_app(ui, rect, &full, n.modified);
        } else {
            self.icons.paint(ui, rect, n.is_dir, n.ext(), tint);
        }
    }

    /// Whether the item at this path in the tree the map shows is inside an application (for
    /// the Applications kind).
    pub(super) fn in_app(&self, view: &[u32]) -> bool {
        self.shown_tree().is_some_and(|t| crate::apps::app_on_path(&t, view).is_some())
    }
}

/// A part worth pointing out: most of the app, and big enough to matter.
fn notable(part: &str, bytes: u64, app: u64) -> bool {
    part != "Program" && part != "Other" && bytes >= 100 * 1024 * 1024 && bytes * 10 >= app * 4
}

impl App {
    /// At the top of Insights while the map is inside an application: what takes its space.
    pub(super) fn app_parts_panel(&mut self, ui: &mut egui::Ui) {
        let Some(path) = self.zoom_app() else { return };
        let Some(tree) = self.shown_tree() else { return };
        let Some(app) = tree.at(&path) else { return };
        let parts = crate::apps::parts(app);
        if parts.is_empty() {
            return;
        }
        let name = crate::apps::shown_name(&app.name, crate::apps::Bundle::App.code()).to_string();
        let t = crate::kit::tokens(ui);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            let full = self.full_path(&path);
            let ctx = ui.ctx().clone();
            self.icons.paint_app_on(&ctx, ui.painter(), r, &full, app.modified);
            ui.label(egui::RichText::new(format!("Inside {name}")).strong());
            ui.weak(scan::human(app.size));
        });
        let total = app.size.max(1);
        let font = FontId::proportional(12.5);
        for &(part, what, bytes) in &parts {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
            let share = bytes as f32 / total as f32;
            let bar = Rect::from_min_size(pos2(rect.left() + 110.0, rect.center().y - 4.0), vec2((rect.width() - 110.0 - 120.0).max(20.0) * share, 8.0));
            ui.painter().rect_filled(bar, 3.0, if notable(part, bytes, app.size) { t.warn } else { self.app_color() });
            ui.painter().text(pos2(rect.left() + 4.0, rect.center().y), Align2::LEFT_CENTER, part, font.clone(), t.text);
            ui.painter().text(pos2(rect.right() - 4.0, rect.center().y), Align2::RIGHT_CENTER, format!("{}  {:>3.0}%", scan::human(bytes), share * 100.0), font.clone(), t.weak);
            resp.on_hover_text(what);
        }
        for &(part, _, bytes) in parts.iter().filter(|p| notable(p.0, p.2, app.size)) {
            let why = match part {
                "Frameworks" => "it brings its own copies of libraries (often a whole web browser engine)",
                "Resources" => "its pictures, sounds, help and translations",
                "Shared support" => "extra content such as samples, templates or an installer",
                "Helpers" => "helper programs it carries for background work",
                _ => "this part",
            };
            ui.label(egui::RichText::new(format!("{} of {name} is {}: {why}.", viz::share(bytes, app.size), part.to_lowercase())).size(12.0).color(t.weak));
        }
        ui.add_space(4.0);
        ui.separator();
    }
}

#[cfg(test)]
mod tests {
    use super::notable;

    #[test]
    fn points_out_big_parts_only() {
        const MB: u64 = 1024 * 1024;
        assert!(notable("Frameworks", 400 * MB, 600 * MB));
        assert!(!notable("Frameworks", 50 * MB, 60 * MB), "too small to matter");
        assert!(!notable("Resources", 100 * MB, 1000 * MB), "a tenth isn't most of it");
        assert!(!notable("Program", 500 * MB, 600 * MB), "the program is the app");
    }
}
