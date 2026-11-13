//! Every keyboard shortcut and mouse action dirmap supports, in one place. The tables in
//! `docs/src/reference/keyboard.md` and `mouse.md` are generated from these lists, and the tests
//! check each entry against the code that implements it (`source`), so the reference can't
//! drift from what the app really does. Change a binding in the code, then update it here and
//! run `DIRMAP_UPDATE_DOCS=1 cargo test help` to regenerate the tables.

/// One input and what it does.
pub struct Binding {
    /// As written on Windows and Linux, e.g. "Ctrl+F".
    pub keys: &'static str,
    /// The Mac spelling when it differs ("⌘F"), else "".
    pub mac: &'static str,
    pub action: &'static str,
    /// (file, text) pairs that must appear in the code implementing it.
    pub source: &'static [(&'static str, &'static str)],
}

pub struct Group {
    pub title: &'static str,
    /// Shown under the title: where these work.
    pub note: &'static str,
    pub bindings: &'static [Binding],
}

const APP: &str = "src/app.rs";
const MENUS: &str = "src/app/menus.rs";
const CUSTOM: &str = "src/app/custom.rs";
const HELP: &str = "src/help/mod.rs";
const BROWSER: &str = "src/app/browser.rs";

const fn b(keys: &'static str, mac: &'static str, action: &'static str, source: &'static [(&'static str, &'static str)]) -> Binding {
    Binding { keys, mac, action, source }
}

pub const KEYBOARD: &[Group] = &[
    Group {
        title: "Anywhere",
        note: "In the main window, unless the cursor is in a text box.",
        bindings: &[
            b("F1", "", "Open help for the part of the window under the pointer.", &[(APP, "Key::F1")]),
            b("Ctrl+F", "⌘F", "Open the Custom filter and put the cursor in it.", &[(CUSTOM, "egui::Modifiers::COMMAND, egui::Key::F")]),
        ],
    },
    Group {
        title: "Treemap",
        note: "When the treemap is showing (not the sunburst) and the cursor isn't in a text box.",
        bindings: &[
            b("+ or =", "", "Zoom in, around the pointer (or the middle of the map).", &[(APP, "egui::Key::Plus"), (APP, "egui::Key::Equals")]),
            b("−", "", "Zoom out, around the pointer (or the middle of the map).", &[(APP, "egui::Key::Minus")]),
            b("Home", "", "Whole map: back to the complete scan.", &[(APP, "egui::Key::Home")]),
            b("Alt+←", "⌥←", "Back to the previous place.", &[(APP, "alt && i.key_pressed(egui::Key::ArrowLeft)")]),
            b("Alt+→", "⌥→", "Forward again.", &[(APP, "alt && i.key_pressed(egui::Key::ArrowRight)")]),
        ],
    },
    Group {
        title: "Files view",
        note: "When the Files view is showing and the cursor isn't in a text box.",
        bindings: &[
            b("↑ / ↓", "", "Select the item above or below (in Icons, the row above or below).", &[(BROWSER, "return Some(Do::Move(cols));"), (BROWSER, "return Some(Do::Move(-cols));")]),
            b("← / →", "", "Icons: the previous or next item. Columns: up to the enclosing folder, or into the selected folder.", &[(BROWSER, "return Some(Do::Move(-1));"), (BROWSER, "return Some(Do::Into);")]),
            b("Home / End", "", "Select the first or last item.", &[(BROWSER, "i.consume_key(M::NONE, Key::Home)"), (BROWSER, "i.consume_key(M::NONE, Key::End)")]),
            b("Page Up / Page Down", "", "Move 20 items (20 rows of icons).", &[(BROWSER, "i.consume_key(M::NONE, Key::PageDown)"), (BROWSER, "i.consume_key(M::NONE, Key::PageUp)")]),
            b("Enter", "", "Open the selected folder in the Files view. A selected file opens with its usual app.", &[(MENUS, "self.map_kind == MapKind::Browser && info.is_dir && ctx.input_mut(|i| i.consume_key(M::NONE, Key::Enter))")]),
            b("Ctrl+↓", "⌘↓", "Open the selected folder or file.", &[(BROWSER, "i.consume_key(M::COMMAND, Key::ArrowDown)")]),
            b("Backspace or Ctrl+↑", "Backspace or ⌘↑", "Up to the enclosing folder, selecting the one you were in.", &[(BROWSER, "i.consume_key(M::COMMAND, Key::ArrowUp) || i.consume_key(M::NONE, Key::Backspace)")]),
            b("Alt+←", "⌥←", "Back to the previous folder.", &[(BROWSER, "i.consume_key(M::ALT, Key::ArrowLeft)")]),
            b("Alt+→", "⌥→", "Forward again.", &[(BROWSER, "i.consume_key(M::ALT, Key::ArrowRight)")]),
            b("Type a name", "", "Jump to the first item whose name starts with what you type.", &[(BROWSER, "(!typed.is_empty()).then_some(Do::Find(typed))")]),
        ],
    },
    Group {
        title: "Selected file or folder",
        note: "Act on the selected item, wherever it was selected (folder list, map or a list), unless the cursor is in a text box. The same commands are in the folder list's right-click menu.",
        bindings: &[
            b("Enter", "", "Open it with its usual app (a folder opens in the file manager).", &[(MENUS, "(M::NONE, Key::Enter, TreeAction::Open)")]),
            b("Ctrl+C or Ctrl+Insert", "⌘C", "Copy its full path.", &[(MENUS, "egui::Event::Copy"), (MENUS, "M::CTRL, Key::Insert")]),
            b("Ctrl+E", "⌘E", "Show it in Explorer, Finder or the file manager.", &[(MENUS, "(M::COMMAND, Key::E, TreeAction::Reveal)")]),
            b("Ctrl+P", "⌘P", "Open Command Prompt (Windows) or a terminal (macOS, Linux) in its folder.", &[(MENUS, "(M::COMMAND, Key::P, TreeAction::Terminal(terms[0].1))")]),
            b("Ctrl+S", "", "Windows: open PowerShell in its folder.", &[(MENUS, "(M::COMMAND, Key::S, TreeAction::Terminal(t.1))")]),
            b("F5", "", "Refresh: read it from disk again (the top item rescans everything).", &[(MENUS, "(M::NONE, Key::F5, TreeAction::Refresh)")]),
            b("Alt+Enter", "⌥Enter", "Properties.", &[(MENUS, "(M::ALT, Key::Enter, TreeAction::Properties)")]),
            b("Ctrl+H", "⌘H", "Compute its SHA-256 hash (files only).", &[(MENUS, "(M::COMMAND, Key::H, TreeAction::Hash)")]),
            b("Ctrl+M", "⌘M", "Move it to another folder.", &[(MENUS, "(M::COMMAND, Key::M, TreeAction::MoveTo)")]),
            b("Delete", "fn+Delete", "Delete to the Recycle Bin or Trash (asks first).", &[(MENUS, "(M::NONE, Key::Delete, TreeAction::Recycle)")]),
            b("Shift+Delete", "⇧fn+Delete", "Delete permanently (asks first).", &[(MENUS, "(M::SHIFT, Key::Delete, TreeAction::Delete)")]),
        ],
    },
    Group {
        title: "Folder box and Custom filter box",
        note: "While typing in them.",
        bindings: &[
            b("Enter (folder box)", "", "Scan the folder typed.", &[(APP, "let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));")]),
            b("Enter (Custom box)", "", "Apply the pattern now and add it to Recent.", &[(CUSTOM, "let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));")]),
            b("↓ / ↑", "", "Choose a suggested extension or example.", &[(CUSTOM, "egui::Key::ArrowDown"), (CUSTOM, "egui::Key::ArrowUp")]),
            b("Tab", "", "Complete the chosen suggestion.", &[(CUSTOM, "egui::Key::Tab")]),
        ],
    },
    Group {
        title: "Confirmation windows",
        note: "",
        bindings: &[b("Escape", "", "Cancel a Delete or Empty Folder confirmation.", &[(MENUS, "egui::Key::Escape")])],
    },
    Group {
        title: "Help window",
        note: "While the pointer is over the help window.",
        bindings: &[
            b("Ctrl+F", "⌘F", "Search the help.", &[(HELP, "Modifiers::COMMAND, Key::F")]),
            b("Alt+←", "⌥←", "Back to the previous help page.", &[(HELP, "Modifiers::ALT, Key::ArrowLeft")]),
            b("Alt+→", "⌥→", "Forward again.", &[(HELP, "Modifiers::ALT, Key::ArrowRight")]),
        ],
    },
];

pub const MOUSE: &[Group] = &[
    Group {
        title: "Treemap",
        note: "",
        bindings: &[
            b("Hover", "", "Show the file's path, size and date in the status bar; a faint outline marks its folder.", &[(APP, "self.hover_status = Some(self.hover_text(&abs));")]),
            b("Wheel forward / back", "", "Zoom in / out towards the pointer. The spot under the pointer stays put.", &[(APP, "self.map.zoom_wheel(amount, p, bounds, &feel);")]),
            b("Ctrl+wheel (⌘+scroll on a Mac), or pinch", "", "Zoom too.", &[(APP, "i.zoom_delta()")]),
            b("Drag (left or middle button)", "", "Pan when zoomed in. Let go while moving to coast.", &[(APP, "resp.dragged_by(egui::PointerButton::Primary) || resp.dragged_by(egui::PointerButton::Middle)")]),
            b("Click", "", "Select the file (the folder list opens to show it).", &[(APP, "select = Some(abs);")]),
            b("Double-click", "", "Open the folder that holds the file (or frame the file if it's already in the folder on screen).", &[(APP, "if resp.double_clicked() {")]),
            b("Right-click", "", "Zoom out one level: to the folder around the view, then to the parent folder.", &[(APP, "self.map.zoom_out(&tree, &feel);")]),
            b("Back / forward mouse buttons", "", "Back and Forward through places you've looked at.", &[(APP, "egui::PointerButton::Extra1"), (APP, "egui::PointerButton::Extra2")]),
        ],
    },
    Group {
        title: "Sunburst",
        note: "",
        bindings: &[
            b("Hover", "", "Show the item's path, size and date in the status bar.", &[(APP, "self.hover_status = Some(self.hover_text(&abs));")]),
            b("Click a segment", "", "Select it.", &[(APP, "select = Some(abs);")]),
            b("Double-click a segment", "", "Make that folder (or the file's folder) the centre.", &[(APP, "zoom_to = Some(abs);")]),
            b("Wheel forward", "", "Go into the top-level folder under the pointer.", &[(APP, "fn wheel_zoom(")]),
            b("Wheel back", "", "Go up one level.", &[(APP, "if *wheel <= -40.0 {")]),
            b("Click the middle, or right-click", "", "Go up one level.", &[(APP, "} else if in_hole {"), (APP, "if resp.secondary_clicked() && !self.zoom.is_empty() {")]),
        ],
    },
    Group {
        title: "Files view",
        note: "",
        bindings: &[
            b("Hover", "", "Show the item's path, size and date in the status bar.", &[(BROWSER, "picks.hover = Some(path.to_vec());")]),
            b("Click", "", "Select the item (also in the folder list and on the maps).", &[(BROWSER, "picks.select = Some(path.to_vec());")]),
            b("Double-click", "", "Open a folder in the Files view, or a file with its usual app.", &[(BROWSER, "picks.open = Some(path.to_vec());")]),
            b("Right-click", "", "Select the item and open the right-click menu.", &[(BROWSER, "resp.context_menu(|ui| {")]),
            b("Click a column heading (List)", "", "Sort by that column; click again to reverse.", &[(BROWSER, "picks.sort = col.sort();")]),
            b("Click a folder in the path", "", "Go to that folder.", &[(BROWSER, "picks.go = Some(self.zoom[..depth].to_vec());")]),
            b("Back / forward mouse buttons", "", "Back and Forward through folders you've opened.", &[(BROWSER, "egui::PointerButton::Extra1"), (BROWSER, "egui::PointerButton::Extra2")]),
        ],
    },
    Group {
        title: "Folder list",
        note: "",
        bindings: &[
            b("Click", "", "Select the item.", &[(APP, "clicked = Some(row.path.to_vec());")]),
            b("Click the arrow, or double-click a folder", "", "Open or close the folder.", &[(APP, "if n.is_dir && ((resp.clicked() && on_arrow) || resp.double_clicked()) {")]),
            b("Right-click", "", "Select the item and open the right-click menu.", &[(APP, "resp.context_menu(|ui| {")]),
            b("Click a column heading", "", "Sort by that column; click again to reverse.", &[(APP, "self.sort_desc = !self.sort_desc;")]),
            b("Drag the border below the list", "", "Give the list or the map more room.", &[(APP, "egui::Panel::top(\"tree\").default_size(320.0).resizable(true)")]),
        ],
    },
    Group {
        title: "Side panel and other lists",
        note: "",
        bindings: &[
            b("Click a file type", "", "Highlight that type on the map; click again to stop.", &[(APP, "toggle = Some(e.ext.clone());")]),
            b("Right-click a file type", "", "Find all files of that type, highlight, copy or exclude it.", &[(APP, "Self::ext_menu(ui, &e.ext, on)")]),
            b("Click a size range (Sizes tab)", "", "Show only files in that range; click again to stop.", &[(APP, "pick = Some(if on { (0, 0) } else { (lo, hi) });")]),
            b("Click a change, duplicate or listed file", "", "Select it in the folder list and on the map.", &[(APP, "self.select_full(p);")]),
            b("Click a sequence", "", "Filter the map to that sequence and select its folder.", &[(CUSTOM, "self.set_custom(&pattern);")]),
            b("Drag the panel's left edge", "", "Make the side panel wider or narrower.", &[(APP, "egui::Panel::right(\"exts\").default_size(330.0).resizable(true)")]),
        ],
    },
    Group {
        title: "Size boxes",
        note: "Larger than, Smaller than, and the Duplicates minimum.",
        bindings: &[
            b("Drag left or right", "", "Change the size.", &[(APP, "egui::DragValue::new(&mut f)")]),
            b("Click, then type", "", "Enter a size such as 500 MB or 2 GB (a plain number means MB; 0 turns it off).", &[(APP, "scan::parse_size(t)")]),
        ],
    },
];

/// The Markdown tables for a reference, as they appear in the docs.
pub fn markdown(groups: &[Group]) -> String {
    let mut out = String::new();
    for g in groups {
        out.push_str(&format!("### {}\n\n", g.title));
        if !g.note.is_empty() {
            out.push_str(&format!("{}\n\n", g.note));
        }
        let mac = g.bindings.iter().any(|b| !b.mac.is_empty());
        if mac {
            out.push_str("| Windows and Linux | macOS | Action |\n|---|---|---|\n");
        } else {
            out.push_str("| Input | Action |\n|---|---|\n");
        }
        for b in g.bindings {
            if mac {
                let m = if b.mac.is_empty() { b.keys } else { b.mac };
                out.push_str(&format!("| **{}** | **{}** | {} |\n", b.keys, m, b.action));
            } else {
                out.push_str(&format!("| **{}** | {} |\n", b.keys, b.action));
            }
        }
        out.push('\n');
    }
    out.trim_end().to_string() + "\n"
}

pub const BEGIN: &str = "<!-- BEGIN GENERATED: do not edit by hand; see src/help/reference.rs -->";
pub const END: &str = "<!-- END GENERATED -->";
