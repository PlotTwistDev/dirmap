//! The Files view: a Finder / Explorer-style browser over the scan, as an alternative to the
//! treemap and sunburst. Its folder is the same `zoom` the maps use and its selection is the
//! app's selection, so switching views keeps your place.

use super::*;
use crate::actions;
use menus::{ItemInfo, TreeAction};
use std::cmp::Ordering as Cmp;

#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum Layout {
    #[default]
    List,
    Icons,
    Columns,
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum SortKey {
    Name,
    #[default]
    Size,
    Kind,
    Items,
    Modified,
}

impl SortKey {
    const ALL: [SortKey; 5] = [SortKey::Name, SortKey::Size, SortKey::Kind, SortKey::Items, SortKey::Modified];

    fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Size => "Size",
            SortKey::Kind => "Kind",
            SortKey::Items => "Items",
            SortKey::Modified => "Last change",
        }
    }

    /// Names and kinds sort A→Z; the rest largest, most or newest first.
    fn natural_desc(self) -> bool {
        !matches!(self, SortKey::Name | SortKey::Kind)
    }
}

/// A column of the List layout.
#[derive(Clone, Copy, PartialEq)]
enum Col {
    Name,
    Size,
    Share,
    Kind,
    Items,
    Modified,
}

impl Col {
    fn sort(self) -> Option<SortKey> {
        match self {
            Col::Name => Some(SortKey::Name),
            Col::Size => Some(SortKey::Size),
            Col::Share => None,
            Col::Kind => Some(SortKey::Kind),
            Col::Items => Some(SortKey::Items),
            Col::Modified => Some(SortKey::Modified),
        }
    }

    fn title(self) -> &'static str {
        match self {
            Col::Name => "Name",
            Col::Size => "Size",
            Col::Share => "% of folder",
            Col::Kind => "Kind",
            Col::Items => "Items",
            Col::Modified => "Last change",
        }
    }
}

#[derive(Default)]
pub(super) struct Browser {
    pub layout: Layout,
    sort: SortKey,
    /// Opposite of the sort's natural direction.
    reversed: bool,
    back: Vec<Vec<u32>>,
    fwd: Vec<Vec<u32>>,
    /// The folder and selection as of last frame, to notice changes made elsewhere.
    seen_zoom: Option<Vec<u32>>,
    seen_sel: Option<Vec<u32>>,
    /// The tree the history and sort orders belong to.
    tree_id: usize,
    orders: HashMap<Vec<u32>, Arc<Vec<u32>>>,
    order_key: (usize, SortKey, bool),
    /// Bring the selection into view next frame.
    scroll_to_sel: bool,
    /// Last scroll offset and height of the main list or grid, for keeping the selection in view.
    view: (f32, f32),
    /// Icons per row in the grid last frame.
    grid_cols: usize,
    /// Type-to-select: letters typed so far and when the last one came.
    typed: String,
    typed_at: f64,
}

const ROW: f32 = 22.0;
const CELL_W: f32 = 108.0;
const CELL_H: f32 = 104.0;
const COL_W: f32 = 250.0;

/// What a folder's entry is: "Folder", "PNG image", "File".
pub(super) fn kind_of(n: &Node) -> String {
    if n.is_dir {
        return "Folder".into();
    }
    let ext = n.ext();
    if ext.is_empty() {
        return "File".into();
    }
    let noun = match categories::of(&n.name) {
        Category::Audio => "audio",
        Category::Video => "video",
        Category::Images => "image",
        Category::Models => "AI model",
        Category::Archives => "archive",
        Category::Documents => "document",
        Category::Code => "source",
        Category::Programs => "program",
        Category::DiskImages => "disk image",
        Category::Other => "file",
    };
    format!("{} {noun}", ext.to_uppercase())
}

/// Case-insensitive, with runs of digits compared as numbers: "shot2" before "shot10".
pub(super) fn natural_cmp(a: &str, b: &str) -> Cmp {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Cmp::Equal,
            (None, _) => return Cmp::Less,
            (_, None) => return Cmp::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = a.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    a.next();
                }
                let mut nb = String::new();
                while let Some(c) = b.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    b.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
                if o != Cmp::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Cmp::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// The order to list a folder's children in.
pub(super) fn sorted(n: &Node, key: SortKey, reversed: bool) -> Vec<u32> {
    let c = &n.children;
    let mut o: Vec<u32> = (0..c.len() as u32).collect();
    let kinds: Vec<String> = if key == SortKey::Kind { c.iter().map(kind_of).collect() } else { Vec::new() };
    let folders_first = matches!(key, SortKey::Name | SortKey::Kind);
    o.sort_by(|&x, &y| {
        let (a, b) = (&c[x as usize], &c[y as usize]);
        let mut ord = match key {
            SortKey::Name => Cmp::Equal,
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Kind => kinds[x as usize].cmp(&kinds[y as usize]),
            SortKey::Items => (a.is_dir, a.files).cmp(&(b.is_dir, b.files)),
            SortKey::Modified => a.modified.cmp(&b.modified),
        };
        if key.natural_desc() {
            ord = ord.reverse();
        }
        let mut ord = ord.then_with(|| natural_cmp(&a.name, &b.name));
        if reversed {
            ord = ord.reverse();
        }
        if folders_first { b.is_dir.cmp(&a.is_dir).then(ord) } else { ord }
    });
    o
}

/// What was clicked or picked while drawing, acted on afterwards.
#[derive(Default)]
struct Picks {
    select: Option<Vec<u32>>,
    /// Selected from a column: the folder that column shows becomes the current one.
    column_select: Option<(Vec<u32>, Vec<u32>)>,
    open: Option<Vec<u32>>,
    go: Option<Vec<u32>>,
    menu: Option<(TreeAction, Vec<u32>)>,
    sort: Option<SortKey>,
    hover: Option<Vec<u32>>,
}

impl App {
    fn browser_order(&mut self, tree: &Arc<Node>, folder: &[u32]) -> Arc<Vec<u32>> {
        let b = &mut self.browser;
        let key = (Arc::as_ptr(tree) as usize, b.sort, b.reversed);
        if key != b.order_key {
            b.orders.clear();
            b.order_key = key;
        }
        if let Some(o) = b.orders.get(folder) {
            return o.clone();
        }
        let o = Arc::new(tree.at(folder).map(|n| sorted(n, b.sort, b.reversed)).unwrap_or_default());
        b.orders.insert(folder.to_vec(), o.clone());
        o
    }

    /// Go to a folder, remembering where we were for Back.
    fn browse_to(&mut self, folder: Vec<u32>) {
        self.browser.typed.clear();
        if folder != self.zoom {
            self.browser.back.push(std::mem::replace(&mut self.zoom, folder));
            self.browser.fwd.clear();
        }
        self.browser.seen_zoom = Some(self.zoom.clone());
    }

    /// Open a folder in the browser (or a file with its app).
    pub(super) fn browse_into(&mut self, path: Vec<u32>) {
        let Some(tree) = self.shown_tree() else { return };
        let Some(n) = tree.at(&path) else { return };
        if !n.is_dir {
            self.tree_action(TreeAction::Open, path);
            return;
        }
        self.browse_to(path);
        self.browser.scroll_to_sel = true;
    }

    /// Up to the folder above, selecting the one we were in.
    fn browse_up(&mut self) {
        if self.zoom.is_empty() {
            return;
        }
        let from = self.zoom.clone();
        let mut up = from.clone();
        up.pop();
        self.browse_to(up);
        self.browser_select(from);
    }

    pub(super) fn browser_can_back(&self) -> bool {
        !self.browser.back.is_empty()
    }

    pub(super) fn browser_can_forward(&self) -> bool {
        !self.browser.fwd.is_empty()
    }

    pub(super) fn browser_back(&mut self) {
        if let Some(p) = self.browser.back.pop() {
            self.browser.fwd.push(std::mem::replace(&mut self.zoom, p));
            self.browser.seen_zoom = Some(self.zoom.clone());
            self.browser.scroll_to_sel = true;
        }
    }

    pub(super) fn browser_forward(&mut self) {
        if let Some(p) = self.browser.fwd.pop() {
            self.browser.back.push(std::mem::replace(&mut self.zoom, p));
            self.browser.seen_zoom = Some(self.zoom.clone());
            self.browser.scroll_to_sel = true;
        }
    }

    /// Select an item from the browser: shared with the folder list and the maps.
    fn browser_select(&mut self, path: Vec<u32>) {
        self.select(path.clone());
        self.browser.seen_sel = Some(path);
        self.browser.scroll_to_sel = true;
    }

    /// The selection, if it's an item of `folder`.
    fn selected_in(&self, folder: &[u32]) -> Option<u32> {
        let s = self.selected.as_ref()?;
        (s.len() == folder.len() + 1 && s.starts_with(folder)).then(|| *s.last().unwrap())
    }

    /// Keep history and folder in step with changes made elsewhere (the folder list, the
    /// breadcrumbs, a new scan or filter).
    fn browser_sync(&mut self, tree: &Arc<Node>) {
        let id = Arc::as_ptr(tree) as usize;
        if self.browser.tree_id != id {
            // Positions in a new tree mean nothing in the old one.
            self.browser.tree_id = id;
            self.browser.back.clear();
            self.browser.fwd.clear();
            self.browser.seen_zoom = None;
        }
        while !self.zoom.is_empty() && !tree.at(&self.zoom).is_some_and(|n| n.is_dir) {
            self.zoom.pop();
        }
        if let Some(seen) = self.browser.seen_zoom.take().filter(|s| *s != self.zoom) {
            self.browser.back.push(seen);
            self.browser.fwd.clear();
        }
        // Something selected elsewhere: show it among the items around it.
        if self.selected != self.browser.seen_sel {
            if let Some(sel) = self.selected.clone().filter(|s| tree.at(s).is_some()) {
                if sel != self.zoom && !sel.is_empty() && sel[..sel.len() - 1] != self.zoom[..] {
                    self.browse_to(sel[..sel.len() - 1].to_vec());
                }
                self.browser.scroll_to_sel = true;
            }
            self.browser.seen_sel = self.selected.clone();
        }
        self.browser.seen_zoom = Some(self.zoom.clone());
    }

    /// Faint (Highlight mode): a file that doesn't match, or a folder with no matches inside.
    fn browser_dim(&self, n: &Node, path: &[u32], hits: &Option<Arc<crate::subset::Hits>>) -> bool {
        if self.shown.is_some() || !self.filter.active() {
            return false;
        }
        if n.is_dir { hits.as_ref().is_some_and(|h| h.at(path).is_none()) } else { !self.filter.matches(&n.name, n.size) }
    }

    fn icon_tint(&self, n: &Node, path: &[u32]) -> Color32 {
        self.node_color(n, &self.full_of(path))
    }

    // ---------- the panel ----------

    pub(super) fn browser_panel(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.shown_tree() else { return };
        self.browser_sync(&tree);
        let prev_status = self.hover_status.take();
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill);
        let mut picks = Picks::default();

        let bar = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| self.browser_bar(ui, &tree, &mut picks));
        let bar_rect = bar.response.rect;
        ui.painter().hline(bar_rect.x_range(), bar_rect.bottom(), ui.visuals().widgets.noninteractive.bg_stroke);

        let order = self.browser_order(&tree, &self.zoom.clone());
        let pointer_in = ui.rect_contains_pointer(rect);
        self.browser_keys(ui.ctx(), &tree, &order, pointer_in);

        let footer_h = 22.0;
        let mut body = ui.available_rect_before_wrap();
        body.max.y -= footer_h;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        match self.browser.layout {
            Layout::List => self.browser_list(&mut child, &tree, &order, &mut picks),
            Layout::Icons => self.browser_icons(&mut child, &tree, &order, &mut picks),
            Layout::Columns => self.browser_columns(&mut child, &tree, &mut picks),
        }
        let footer = Rect::from_min_max(pos2(rect.left(), body.bottom()), pos2(rect.right(), body.bottom() + footer_h));
        self.browser_footer(ui, footer, &tree, &order);
        self.browser.scroll_to_sel = false;

        // Act on what was clicked.
        if let Some(p) = picks.hover {
            self.hover_status = Some(self.hover_text(&p));
        }
        if let Some(k) = picks.sort {
            if self.browser.sort == k {
                self.browser.reversed = !self.browser.reversed;
            } else {
                self.browser.sort = k;
                self.browser.reversed = false;
            }
        }
        if let Some((folder, p)) = picks.column_select {
            // Moving along the columns isn't a place to go Back to.
            self.zoom = folder;
            self.browser.seen_zoom = Some(self.zoom.clone());
            self.browser_select(p);
        }
        if let Some(p) = picks.select {
            self.browser_select(p);
        }
        if let Some(p) = picks.go {
            self.browse_to(p);
            self.browser.scroll_to_sel = true;
        }
        if let Some(p) = picks.open {
            self.browser_select(p.clone());
            self.browse_into(p);
        }
        if let Some((a, p)) = picks.menu {
            self.browser.seen_sel = Some(p.clone());
            match a {
                TreeAction::Toggle => self.browse_into(p),
                a => self.tree_action(a, p),
            }
            self.browser.seen_sel = self.selected.clone();
        }
        if self.hover_status != prev_status {
            ui.ctx().request_repaint();
        }
    }

    /// Back, Forward, Up, the path, and the layout and sort controls.
    fn browser_bar(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, picks: &mut Picks) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let ctx = ui.ctx().clone();
            let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
            if ui.add_enabled(self.browser_can_back(), egui::Button::new("◀")).on_hover_text(format!("Back ({})", k(egui::Modifiers::ALT, egui::Key::ArrowLeft))).clicked() {
                self.browser_back();
            }
            if ui.add_enabled(self.browser_can_forward(), egui::Button::new("▶")).on_hover_text(format!("Forward ({})", k(egui::Modifiers::ALT, egui::Key::ArrowRight))).clicked() {
                self.browser_forward();
            }
            if ui.add_enabled(!self.zoom.is_empty(), egui::Button::new("⏶")).on_hover_text(format!("Up to the enclosing folder (Backspace or {})", k(egui::Modifiers::COMMAND, egui::Key::ArrowUp))).clicked() {
                self.browse_up();
            }
            ui.add_space(6.0);
            // The path, each part clickable.
            ui.spacing_mut().item_spacing.x = 3.0;
            let mut n: &Node = tree;
            let crumbs: Vec<(usize, String)> = std::iter::once((0, format!("🖴 {}", tree.name)))
                .chain(self.zoom.iter().enumerate().filter_map(|(k, &i)| {
                    n = n.children.get(i as usize)?;
                    Some((k + 1, n.name.to_string()))
                }))
                .collect();
            let last = crumbs.len() - 1;
            for (k, (depth, label)) in crumbs.into_iter().enumerate() {
                if k > 0 {
                    ui.weak("›");
                }
                let text = if k == last { egui::RichText::new(label).strong() } else { egui::RichText::new(label) };
                if ui.add(egui::Button::new(text).frame(false)).clicked() && k != last {
                    picks.go = Some(self.zoom[..depth].to_vec());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let b = &mut self.browser;
                let arrow = if b.reversed != b.sort.natural_desc() { "⏷" } else { "⏶" };
                egui::ComboBox::from_id_salt("browser-sort")
                    .selected_text(format!("Sort: {} {arrow}", b.sort.label()))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for key in SortKey::ALL {
                            if ui.selectable_label(b.sort == key, key.label()).clicked() {
                                picks.sort = Some(key);
                            }
                        }
                    })
                    .response
                    .on_hover_text("Choose the order; pick the same one again to reverse it");
                ui.add_space(8.0);
                for (layout, label, tip) in [
                    (Layout::Columns, "◫ Columns", "A column per folder, from the top of the scan down to here"),
                    (Layout::Icons, "⊞ Icons", "Large icons in a grid"),
                    (Layout::List, "☰ List", "One line per item, with sizes and dates"),
                ] {
                    ui.selectable_value(&mut b.layout, layout, label).on_hover_text(tip);
                }
            });
        });
    }

    /// Count and total of the folder, and the selected item.
    fn browser_footer(&mut self, ui: &mut egui::Ui, rect: Rect, tree: &Arc<Node>, order: &[u32]) {
        let p = ui.painter();
        p.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
        let Some(folder) = tree.at(&self.zoom) else { return };
        let items = order.len();
        let mut text = format!("{} item{} · {}", group(items as u64), if items == 1 { "" } else { "s" }, scan::human(folder.size));
        if let Some(i) = self.selected_in(&self.zoom) {
            if let Some(n) = folder.children.get(i as usize) {
                let share = if folder.size > 0 { n.size as f64 * 100.0 / folder.size as f64 } else { 0.0 };
                text.push_str(&format!("  ·  “{}” selected: {} ({share:.1}% of this folder)", n.name, scan::human(n.size)));
            }
        }
        p.text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), ui.visuals().weak_text_color());
    }

    /// Arrow keys, Enter, Backspace, Back/Forward and type-to-select.
    fn browser_keys(&mut self, ctx: &egui::Context, tree: &Arc<Node>, order: &[u32], pointer_in: bool) {
        use egui::{Key, Modifiers as M};
        if self.ops.modal() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let help_hovered = self.help.hovered;
        let cols = match self.browser.layout {
            Layout::Icons => self.browser.grid_cols.max(1) as i64,
            _ => 1,
        };
        let pos = self.selected_in(&self.zoom).and_then(|i| order.iter().position(|&o| o == i)).map(|p| p as i64);
        let now = ctx.input(|i| i.time);
        enum Do {
            Move(i64),
            To(i64),
            Up,
            Into,
            Open,
            Back,
            Forward,
            Find(String),
        }
        let act = ctx.input_mut(|i| {
            let mouse = pointer_in && !help_hovered;
            if (mouse && i.pointer.button_pressed(egui::PointerButton::Extra1)) || i.consume_key(M::ALT, Key::ArrowLeft) {
                return Some(Do::Back);
            }
            if (mouse && i.pointer.button_pressed(egui::PointerButton::Extra2)) || i.consume_key(M::ALT, Key::ArrowRight) {
                return Some(Do::Forward);
            }
            if i.consume_key(M::COMMAND, Key::ArrowUp) || i.consume_key(M::NONE, Key::Backspace) {
                return Some(Do::Up);
            }
            if i.consume_key(M::COMMAND, Key::ArrowDown) {
                return Some(Do::Open);
            }
            if i.consume_key(M::NONE, Key::ArrowDown) {
                return Some(Do::Move(cols));
            }
            if i.consume_key(M::NONE, Key::ArrowUp) {
                return Some(Do::Move(-cols));
            }
            if i.consume_key(M::NONE, Key::Home) {
                return Some(Do::To(0));
            }
            if i.consume_key(M::NONE, Key::End) {
                return Some(Do::To(i64::MAX));
            }
            if i.consume_key(M::NONE, Key::PageDown) {
                return Some(Do::Move(20 * cols));
            }
            if i.consume_key(M::NONE, Key::PageUp) {
                return Some(Do::Move(-20 * cols));
            }
            match self.browser.layout {
                Layout::Icons => {
                    if i.consume_key(M::NONE, Key::ArrowRight) {
                        return Some(Do::Move(1));
                    }
                    if i.consume_key(M::NONE, Key::ArrowLeft) {
                        return Some(Do::Move(-1));
                    }
                }
                Layout::Columns => {
                    if i.consume_key(M::NONE, Key::ArrowRight) {
                        return Some(Do::Into);
                    }
                    if i.consume_key(M::NONE, Key::ArrowLeft) {
                        return Some(Do::Up);
                    }
                }
                Layout::List => {}
            }
            if i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt {
                return None;
            }
            let typed: String = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect();
            (!typed.is_empty()).then_some(Do::Find(typed))
        });
        let Some(act) = act else { return };
        let len = order.len() as i64;
        let pick = |app: &mut Self, p: i64| {
            if len > 0 {
                let mut path = app.zoom.clone();
                path.push(order[p.clamp(0, len - 1) as usize]);
                app.browser_select(path);
            }
        };
        match act {
            Do::Move(d) => pick(self, pos.map_or(if d > 0 { 0 } else { len - 1 }, |p| p + d)),
            Do::To(p) => pick(self, p),
            Do::Up => self.browse_up(),
            Do::Back => self.browser_back(),
            Do::Forward => self.browser_forward(),
            Do::Open => {
                if let Some(s) = self.selected.clone().filter(|_| pos.is_some()) {
                    self.browse_into(s);
                }
            }
            Do::Into => {
                // Columns: into the selected folder, selecting its first item.
                if let Some(s) = self.selected.clone().filter(|_| pos.is_some()) {
                    if tree.at(&s).is_some_and(|n| n.is_dir && !n.children.is_empty()) {
                        let first = self.browser_order(tree, &s).first().copied();
                        self.zoom = s.clone();
                        self.browser.seen_zoom = Some(self.zoom.clone());
                        self.browser.typed.clear();
                        if let Some(f) = first {
                            let mut p = s;
                            p.push(f);
                            self.browser_select(p);
                        }
                    }
                }
            }
            Do::Find(t) => {
                if now - self.browser.typed_at > 1.0 {
                    self.browser.typed.clear();
                }
                self.browser.typed_at = now;
                self.browser.typed.push_str(&t.to_lowercase());
                let want = self.browser.typed.clone();
                if let Some(folder) = tree.at(&self.zoom) {
                    let hit = order.iter().position(|&i| folder.children[i as usize].name.to_lowercase().starts_with(&want));
                    if let Some(p) = hit {
                        pick(self, p as i64);
                    }
                }
            }
        }
    }

    // ---------- List ----------

    fn list_columns(width: f32) -> Vec<(Col, f32)> {
        let mut cols = vec![(Col::Size, 84.0), (Col::Share, 112.0), (Col::Kind, 130.0), (Col::Items, 72.0), (Col::Modified, 132.0)];
        // Narrow windows lose the less important columns first.
        for drop in [Col::Items, Col::Kind, Col::Share, Col::Modified] {
            if width - cols.iter().map(|c| c.1).sum::<f32>() >= 220.0 {
                break;
            }
            cols.retain(|c| c.0 != drop);
        }
        let fixed: f32 = cols.iter().map(|c| c.1).sum();
        cols.insert(0, (Col::Name, (width - fixed).max(160.0)));
        cols
    }

    fn browser_list(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, order: &Arc<Vec<u32>>, picks: &mut Picks) {
        let folder_path = self.zoom.clone();
        let Some(folder) = tree.at(&folder_path) else { return };
        let width = ui.available_width();
        let cols = Self::list_columns(width);

        // Headings.
        let (hdr, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
        ui.painter().rect_filled(hdr, 0.0, ui.visuals().faint_bg_color);
        let mut x = hdr.left();
        for (i, (col, w)) in cols.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(x, hdr.top()), vec2(*w, ROW));
            x += w;
            let sense = if col.sort().is_some() { Sense::click() } else { Sense::hover() };
            let resp = ui.interact(cell, ui.id().with(("bhdr", i)), sense);
            let on = col.sort() == Some(self.browser.sort);
            let arrow = if !on { "" } else if self.browser.reversed != self.browser.sort.natural_desc() { " ⏷" } else { " ⏶" };
            let color = if resp.hovered() || on { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
            let (at, align) = match col {
                Col::Name | Col::Kind | Col::Share => (pos2(cell.left() + 8.0, cell.center().y), Align2::LEFT_CENTER),
                _ => (pos2(cell.right() - 8.0, cell.center().y), Align2::RIGHT_CENTER),
            };
            ui.painter().text(at, align, format!("{}{arrow}", col.title()), FontId::proportional(13.0), color);
            if resp.on_hover_text(if col.sort().is_some() { "Click to sort; again to reverse" } else { "Share of this folder's size" }).clicked() {
                picks.sort = col.sort();
            }
        }

        if order.is_empty() {
            Self::empty_note(ui, self.shown.is_some());
            return;
        }
        let hits = self.current_hits();
        let sel = self.selected_in(&folder_path);
        let sel_row = sel.and_then(|s| order.iter().position(|&o| o == s));
        let mut area = egui::ScrollArea::vertical().id_salt(("blist", &folder_path)).auto_shrink(false);
        if let Some(r) = sel_row.filter(|_| self.browser.scroll_to_sel) {
            area = area.vertical_scroll_offset(keep_in_view(self.browser.view, r as f32 * ROW, ROW));
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let out = area.show_rows(ui, ROW, order.len(), |ui, range| {
            for r in range {
                let i = order[r];
                let n = &folder.children[i as usize];
                let mut path = folder_path.clone();
                path.push(i);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                let selected = sel == Some(i);
                let dim = self.browser_dim(n, &path, &hits);
                let vis = ui.visuals().clone();
                let p = ui.painter().clone();
                if selected {
                    p.rect_filled(rect, 3.0, vis.selection.bg_fill);
                } else if resp.hovered() {
                    p.rect_filled(rect, 3.0, vis.widgets.hovered.weak_bg_fill);
                } else if r % 2 == 1 && self.view().stripes {
                    p.rect_filled(rect, 0.0, vis.faint_bg_color);
                }
                let tc = if selected {
                    vis.selection.stroke.color
                } else if dim {
                    vis.weak_text_color().gamma_multiply(0.6)
                } else {
                    vis.text_color()
                };
                let weak = if selected || dim { tc } else { vis.weak_text_color() };
                let font = FontId::proportional(13.0);
                let cy = rect.center().y;
                let tint = self.icon_tint(n, &path);
                let tint = if dim { custom::dim(tint) } else { tint };
                let mut x = rect.left();
                for (col, w) in &cols {
                    let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, ROW));
                    x += w;
                    let right = pos2(cell.right() - 8.0, cy);
                    match col {
                        Col::Name => {
                            let icon = Rect::from_min_size(pos2(cell.left() + 8.0, cy - 8.0), vec2(16.0, 16.0));
                            self.icons.paint(ui, icon, n.is_dir, n.ext(), tint);
                            let clip = Rect::from_min_max(pos2(cell.left() + 30.0, rect.top()), pos2(cell.right() - 6.0, rect.bottom()));
                            let np = p.with_clip_rect(clip.intersect(p.clip_rect()));
                            np.text(pos2(clip.left(), cy), Align2::LEFT_CENTER, &*n.name, font.clone(), tc);
                        }
                        Col::Size => {
                            p.text(right, Align2::RIGHT_CENTER, scan::human(n.size), font.clone(), tc);
                        }
                        Col::Share => {
                            let frac = if folder.size > 0 { n.size as f32 / folder.size as f32 } else { 0.0 };
                            let bar = Rect::from_min_size(pos2(cell.left() + 8.0, cy - 4.0), vec2(54.0, 8.0));
                            p.rect_filled(bar, 2.0, vis.extreme_bg_color);
                            let mut fill = bar;
                            fill.set_width(bar.width() * frac.clamp(0.0, 1.0));
                            let c = if n.is_dir { Color32::from_rgb(90, 140, 220) } else { tint };
                            p.rect_filled(fill, 2.0, if dim { custom::dim(c) } else { c });
                            p.text(right, Align2::RIGHT_CENTER, format!("{:.1}%", frac * 100.0), font.clone(), weak);
                        }
                        Col::Kind => {
                            let clip = cell.shrink2(vec2(6.0, 0.0));
                            p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(cell.left() + 8.0, cy), Align2::LEFT_CENTER, kind_of(n), font.clone(), weak);
                        }
                        Col::Items => {
                            let t = if n.is_dir { group(n.files) } else { "—".into() };
                            p.text(right, Align2::RIGHT_CENTER, t, font.clone(), weak);
                        }
                        Col::Modified => {
                            if n.modified > 0 {
                                p.text(right, Align2::RIGHT_CENTER, fmt_time(n.modified), font.clone(), weak);
                            }
                        }
                    }
                }
                self.item_input(&resp, &path, n, picks);
            }
        });
        self.browser.view = (out.state.offset.y, out.inner_rect.height());
    }

    /// Clicks, double-clicks, hover and the right-click menu, the same in every layout.
    fn item_input(&self, resp: &egui::Response, path: &[u32], n: &Node, picks: &mut Picks) {
        if resp.double_clicked() {
            picks.open = Some(path.to_vec());
        } else if resp.clicked() || resp.secondary_clicked() {
            picks.select = Some(path.to_vec());
        }
        if resp.hovered() {
            picks.hover = Some(path.to_vec());
        }
        let info = ItemInfo {
            is_dir: n.is_dir,
            open: false,
            is_root: path.is_empty(),
            is_drive: path.is_empty() && self.scanned_path.parent().is_none(),
            busy: self.ops.job.is_some(),
            in_browser: true,
        };
        resp.context_menu(|ui| {
            if let Some(a) = Self::tree_menu(ui, &info) {
                picks.menu = Some((a, path.to_vec()));
            }
        });
    }

    fn empty_note(ui: &mut egui::Ui, filtered: bool) {
        let msg = if filtered { "Nothing in this folder matches the filter" } else { "This folder is empty" };
        ui.add_space(30.0);
        ui.vertical_centered(|ui| ui.weak(msg));
    }

    // ---------- Icons ----------

    fn browser_icons(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, order: &Arc<Vec<u32>>, picks: &mut Picks) {
        let folder_path = self.zoom.clone();
        let Some(folder) = tree.at(&folder_path) else { return };
        if order.is_empty() {
            Self::empty_note(ui, self.shown.is_some());
            return;
        }
        let width = ui.available_width() - 12.0;
        let per_row = ((width / CELL_W).floor() as usize).max(1);
        self.browser.grid_cols = per_row;
        let rows = order.len().div_ceil(per_row);
        let hits = self.current_hits();
        let sel = self.selected_in(&folder_path);
        let sel_row = sel.and_then(|s| order.iter().position(|&o| o == s)).map(|p| p / per_row);
        let mut area = egui::ScrollArea::vertical().id_salt(("bicons", &folder_path)).auto_shrink(false);
        if let Some(r) = sel_row.filter(|_| self.browser.scroll_to_sel) {
            area = area.vertical_scroll_offset(keep_in_view(self.browser.view, r as f32 * CELL_H, CELL_H));
        }
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let out = area.show_rows(ui, CELL_H, rows, |ui, range| {
            for row in range {
                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), CELL_H), Sense::hover());
                for c in 0..per_row {
                    let Some(&i) = order.get(row * per_row + c) else { break };
                    let n = &folder.children[i as usize];
                    let mut path = folder_path.clone();
                    path.push(i);
                    let cell = Rect::from_min_size(pos2(line.left() + 6.0 + c as f32 * CELL_W, line.top()), vec2(CELL_W, CELL_H)).shrink(3.0);
                    let resp = ui.interact(cell, ui.id().with(("bicon", i)), Sense::click());
                    let selected = sel == Some(i);
                    let dim = self.browser_dim(n, &path, &hits);
                    let vis = ui.visuals().clone();
                    let p = ui.painter().clone();
                    if selected {
                        p.rect_filled(cell, 6.0, vis.selection.bg_fill);
                    } else if resp.hovered() {
                        p.rect_filled(cell, 6.0, vis.widgets.hovered.weak_bg_fill);
                    }
                    let tint = self.icon_tint(n, &path);
                    let tint = if dim { custom::dim(tint) } else { tint };
                    let icon = Rect::from_center_size(pos2(cell.center().x, cell.top() + 28.0), vec2(42.0, 42.0));
                    self.icons.paint(ui, icon, n.is_dir, n.ext(), tint);
                    if dim {
                        p.rect_filled(icon, 0.0, vis.panel_fill.gamma_multiply(0.6));
                    }
                    // Share of the folder, as a thin bar under the icon.
                    let frac = if folder.size > 0 { n.size as f32 / folder.size as f32 } else { 0.0 };
                    let bar = Rect::from_center_size(pos2(cell.center().x, cell.top() + 55.0), vec2(56.0, 3.0));
                    p.rect_filled(bar, 1.5, vis.extreme_bg_color);
                    let mut fill = bar;
                    fill.set_width((bar.width() * frac).max(if frac > 0.0 { 1.0 } else { 0.0 }));
                    p.rect_filled(fill, 1.5, if n.is_dir { Color32::from_rgb(90, 140, 220) } else { tint });
                    let tc = if selected {
                        vis.selection.stroke.color
                    } else if dim {
                        vis.weak_text_color().gamma_multiply(0.6)
                    } else {
                        vis.text_color()
                    };
                    let mut job = egui::text::LayoutJob::simple(n.name.to_string(), FontId::proportional(12.0), tc, cell.width() - 8.0);
                    job.wrap.max_rows = 2;
                    job.wrap.break_anywhere = true;
                    job.halign = egui::Align::Center;
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let gh = galley.size().y;
                    p.galley(pos2(cell.center().x, cell.top() + 61.0), galley, tc);
                    let weak = if selected || dim { tc } else { vis.weak_text_color() };
                    p.text(pos2(cell.center().x, cell.top() + 63.0 + gh), Align2::CENTER_TOP, scan::human(n.size), FontId::proportional(11.0), weak);
                    self.item_input(&resp, &path, n, picks);
                }
            }
        });
        self.browser.view = (out.state.offset.y, out.inner_rect.height());
    }

    // ---------- Columns ----------

    fn browser_columns(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, picks: &mut Picks) {
        // One column per folder from the top down to the current one, then the selected
        // folder's contents or the selected file's details.
        let mut folders: Vec<Vec<u32>> = (0..=self.zoom.len()).map(|k| self.zoom[..k].to_vec()).collect();
        let sel_here = self.selected_in(&self.zoom).map(|i| {
            let mut p = self.zoom.clone();
            p.push(i);
            p
        });
        let mut preview = None;
        if let Some(s) = sel_here {
            match tree.at(&s) {
                Some(n) if n.is_dir => folders.push(s),
                Some(_) => preview = Some(s),
                None => {}
            }
        }
        let height = ui.available_height();
        let hits = self.current_hits();
        egui::ScrollArea::horizontal().id_salt("bcols").auto_shrink(false).stick_to_right(true).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for (k, folder_path) in folders.iter().enumerate() {
                    let order = self.browser_order(tree, folder_path);
                    let Some(folder) = tree.at(folder_path) else { continue };
                    // The item that leads to the next column, or the selection in the last folder.
                    let mark = if k < self.zoom.len() { Some(self.zoom[k]) } else { self.selected_in(folder_path) };
                    let active = k == self.zoom.len();
                    let (col_rect, _) = ui.allocate_exact_size(vec2(COL_W, height), Sense::hover());
                    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(col_rect).layout(egui::Layout::top_down(egui::Align::Min)));
                    cui.set_clip_rect(col_rect.intersect(ui.clip_rect()));
                    ui.painter().vline(col_rect.right(), col_rect.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);
                    if order.is_empty() {
                        Self::empty_note(&mut cui, self.shown.is_some());
                        continue;
                    }
                    let mark_row = mark.and_then(|m| order.iter().position(|&o| o == m));
                    let mut area = egui::ScrollArea::vertical().id_salt(("bcol", folder_path)).auto_shrink(false);
                    if let Some(r) = mark_row.filter(|_| self.browser.scroll_to_sel) {
                        let y = r as f32 * ROW;
                        area = area.vertical_scroll_offset((y - height / 2.0).max(0.0));
                    }
                    cui.spacing_mut().item_spacing.y = 0.0;
                    area.show_rows(&mut cui, ROW, order.len(), |ui, range| {
                        for r in range {
                            let i = order[r];
                            let n = &folder.children[i as usize];
                            let mut path = folder_path.clone();
                            path.push(i);
                            let (rect, resp) = ui.allocate_exact_size(vec2(COL_W - 1.0, ROW), Sense::click());
                            let on = mark == Some(i);
                            let vis = ui.visuals().clone();
                            let p = ui.painter().clone();
                            let dim = self.browser_dim(n, &path, &hits);
                            if on {
                                // The trail to the current folder is drawn quieter than the selection.
                                let fill = if active { vis.selection.bg_fill } else { vis.widgets.inactive.weak_bg_fill };
                                p.rect_filled(rect.shrink2(vec2(3.0, 0.0)), 3.0, fill);
                            } else if resp.hovered() {
                                p.rect_filled(rect.shrink2(vec2(3.0, 0.0)), 3.0, vis.widgets.hovered.weak_bg_fill);
                            }
                            let tc = if on && active {
                                vis.selection.stroke.color
                            } else if dim {
                                vis.weak_text_color().gamma_multiply(0.6)
                            } else {
                                vis.text_color()
                            };
                            let weak = if (on && active) || dim { tc } else { vis.weak_text_color() };
                            let cy = rect.center().y;
                            let tint = self.icon_tint(n, &path);
                            let tint = if dim { custom::dim(tint) } else { tint };
                            self.icons.paint(ui, Rect::from_min_size(pos2(rect.left() + 8.0, cy - 8.0), vec2(16.0, 16.0)), n.is_dir, n.ext(), tint);
                            let size = scan::human(n.size);
                            let size_w = 74.0;
                            p.text(pos2(rect.right() - 20.0, cy), Align2::RIGHT_CENTER, size, FontId::proportional(11.5), weak);
                            if n.is_dir {
                                p.text(pos2(rect.right() - 8.0, cy), Align2::CENTER_CENTER, "›", FontId::proportional(14.0), weak);
                            }
                            let clip = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(rect.right() - 20.0 - size_w, rect.bottom()));
                            p.with_clip_rect(clip.intersect(p.clip_rect())).text(pos2(clip.left(), cy), Align2::LEFT_CENTER, &*n.name, FontId::proportional(13.0), tc);
                            // A click in any column makes that column's folder the current one.
                            let before = (picks.select.clone(), picks.open.clone());
                            self.item_input(&resp, &path, n, picks);
                            if picks.select != before.0 {
                                if let Some(s) = picks.select.take() {
                                    picks.column_select = Some((folder_path.clone(), s));
                                }
                            }
                        }
                    });
                }
                if let Some(p) = preview {
                    self.file_preview(ui, tree, &p, height);
                }
            });
        });
    }

    /// The last column when a file is selected: a big icon and what's known about it.
    fn file_preview(&mut self, ui: &mut egui::Ui, tree: &Arc<Node>, path: &[u32], height: f32) {
        let Some(n) = tree.at(path) else { return };
        let (rect, _) = ui.allocate_exact_size(vec2(COL_W + 40.0, height), Sense::hover());
        let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(14.0)).layout(egui::Layout::top_down(egui::Align::Center)));
        cui.add_space(10.0);
        let (icon, _) = cui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
        let tint = self.icon_tint(n, path);
        self.icons.paint(&cui, icon, false, n.ext(), tint);
        cui.add_space(8.0);
        cui.add(egui::Label::new(egui::RichText::new(&*n.name).strong()).wrap());
        cui.weak(format!("{} · {}", kind_of(n), scan::human(n.size)));
        cui.add_space(8.0);
        egui::Grid::new("bpreview").num_columns(2).spacing([10.0, 4.0]).show(&mut cui, |ui| {
            ui.weak("Size");
            ui.label(format!("{} ({} bytes)", scan::human(n.size), group(n.size)));
            ui.end_row();
            if n.modified > 0 {
                ui.weak("Last change");
                ui.label(fmt_time(n.modified));
                ui.end_row();
            }
            if let Some(parent) = tree.at(&path[..path.len() - 1]).filter(|p| p.size > 0) {
                ui.weak("Of folder");
                ui.label(format!("{:.1}%", n.size as f64 * 100.0 / parent.size as f64));
                ui.end_row();
            }
        });
        cui.add_space(10.0);
        let mut act = None;
        cui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui.button("Open").clicked() {
                act = Some(TreeAction::Open);
            }
            if ui.button(format!("Show in {}", actions::FILE_MANAGER)).clicked() {
                act = Some(TreeAction::Reveal);
            }
        });
        if let Some(a) = act {
            self.tree_action(a, path.to_vec());
        }
    }
}

/// Scroll offset that brings a row at `y` (height `h`) into a view last at `(offset, height)`,
/// moving as little as possible.
fn keep_in_view(view: (f32, f32), y: f32, h: f32) -> f32 {
    let (off, height) = view;
    if y < off {
        y
    } else if y + h > off + height {
        (y + h - height).max(0.0)
    } else {
        off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, modified: i64) -> Node {
        Node { name: name.into(), size, files: 1, modified, is_dir: false, children: Vec::new() }
    }

    fn dir(name: &str, children: Vec<Node>) -> Node {
        let size = children.iter().map(|c| c.size).sum();
        let files = children.iter().map(|c| c.files).sum();
        Node { name: name.into(), size, files, modified: 0, is_dir: true, children }
    }

    fn names(n: &Node, key: SortKey, reversed: bool) -> Vec<&str> {
        sorted(n, key, reversed).into_iter().map(|i| &*n.children[i as usize].name).collect()
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("shot2.exr", "shot10.exr"), Cmp::Less);
        assert_eq!(natural_cmp("Apple", "banana"), Cmp::Less);
        assert_eq!(natural_cmp("a01", "a1"), Cmp::Greater);
        assert_eq!(natural_cmp("a", "a"), Cmp::Equal);
    }

    #[test]
    fn sorting() {
        let root = dir(
            "r",
            vec![
                file("b10.png", 50, 3),
                dir("zeta", vec![file("x", 500, 1)]),
                file("b2.png", 10, 9),
                dir("Alpha", vec![file("y", 1, 2), file("z", 1, 2)]),
                file("notes.txt", 100, 5),
            ],
        );
        // Names: folders first, then files in natural order; reversed keeps folders first.
        assert_eq!(names(&root, SortKey::Name, false), ["Alpha", "zeta", "b2.png", "b10.png", "notes.txt"]);
        assert_eq!(names(&root, SortKey::Name, true), ["zeta", "Alpha", "notes.txt", "b10.png", "b2.png"]);
        assert_eq!(names(&root, SortKey::Size, false), ["zeta", "notes.txt", "b10.png", "b2.png", "Alpha"]);
        assert_eq!(names(&root, SortKey::Size, true)[0], "Alpha");
        assert_eq!(names(&root, SortKey::Modified, false)[0], "b2.png");
        assert_eq!(names(&root, SortKey::Items, false)[..2], ["Alpha", "zeta"]);
        assert_eq!(names(&root, SortKey::Kind, false), ["Alpha", "zeta", "b2.png", "b10.png", "notes.txt"]);
        assert_eq!(kind_of(&root.children[0]), "PNG image");
        assert_eq!(kind_of(&root.children[1]), "Folder");
        assert_eq!(kind_of(&file("README", 1, 0)), "File");
    }

    #[test]
    fn scrolling_moves_as_little_as_possible() {
        assert_eq!(keep_in_view((100.0, 200.0), 150.0, 20.0), 100.0);
        assert_eq!(keep_in_view((100.0, 200.0), 40.0, 20.0), 40.0);
        assert_eq!(keep_in_view((100.0, 200.0), 300.0, 20.0), 120.0);
    }
}
