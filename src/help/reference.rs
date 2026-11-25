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
const FILES: &str = "src/app/files.rs";
const REMOTE: &str = "src/app/remoteui.rs";
const DOCK: &str = "src/app/dock.rs";
const ADDR: &str = "src/app/addr.rs";
const FAVS: &str = "src/app/favs.rs";
const FILEOPS: &str = "src/app/fileops.rs";
const PREVIEW: &str = "src/app/preview.rs";
const VIEWS: &str = "src/app/views.rs";
const PLAYER: &str = "src/app/seqplayer.rs";

const fn b(keys: &'static str, mac: &'static str, action: &'static str, source: &'static [(&'static str, &'static str)]) -> Binding {
    Binding { keys, mac, action, source }
}

pub const KEYBOARD: &[Group] = &[
    Group {
        title: "Anywhere",
        note: "In the main window, unless the cursor is in a text box.",
        bindings: &[
            b("F1", "", "Open help for the part of the window under the pointer.", &[(APP, "Key::F1")]),
            b("Ctrl+1", "⌘1", "Switch to the Files workspace.", &[(FILES, "i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num1)")]),
            b("Ctrl+2", "⌘2", "Switch to the Remote workspace (this computer beside a server).", &[(FILES, "i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num2)")]),
            b("Ctrl+3", "⌘3", "Switch to the Storage Analysis workspace.", &[(FILES, "i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num3)")]),
            b("Ctrl+4", "⌘4", "Switch to the AI Models workspace.", &[(FILES, "i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num4)")]),
            b("Ctrl+F", "⌘F", "Files: put the cursor in the Filter box. Storage Analysis: open the Custom filter.", &[(FILES, "i.consume_key(M::COMMAND, Key::F)"), (CUSTOM, "egui::Modifiers::COMMAND, egui::Key::F")]),
        ],
    },
    Group {
        title: "Files workspace",
        note: "After you've clicked in the list or the sidebar, unless the cursor is in a text box.",
        bindings: &[
            b("↑ / ↓", "", "Select the item above or below. Hold Shift to extend the selection.", &[(FILES, "return Some(Do::Move(1));"), (FILES, "return Some(Do::Move(-1));")]),
            b("← / →, ↑ / ↓ (Icons view)", "", "Move along a row of icons, or to the row above or below.", &[(FILES, "if icons && i.consume_key(m, Key::ArrowDown) {"), (FILES, "((look || icons || strip) && i.consume_key(m, Key::ArrowRight))")]),
            b("← / → (Columns view)", "", "Up to the folder at the left, selecting the one you were in; into the selected folder.", &[(FILES, "if columns && !look && i.consume_key(M::NONE, Key::ArrowLeft) {"), (FILES, "if columns && !look && cursor_dir && i.consume_key(M::NONE, Key::ArrowRight) {")]),
            b("← / → (Gallery view)", "", "The previous or next item in the strip.", &[(FILES, "((look || icons || strip) && i.consume_key(m, Key::ArrowLeft))")]),
            b("Home / End", "", "Select the first or last item.", &[(FILES, "return Some(Do::To(0));"), (FILES, "return Some(Do::To(i64::MAX));")]),
            b("Page Up / Page Down", "", "Move a screenful.", &[(FILES, "return Some(Do::Page(1));"), (FILES, "return Some(Do::Page(-1));")]),
            b("Enter or Ctrl+↓", "Enter or ⌘↓", "Open the selected folder, or the file with its usual app.", &[(FILES, "i.consume_key(M::COMMAND, Key::ArrowDown) || i.consume_key(M::NONE, Key::Enter)")]),
            b("Backspace, Alt+↑ or Ctrl+↑", "Backspace, ⌥↑ or ⌘↑", "Up to the enclosing folder, selecting the one you were in. Above a drive: the Computer view.", &[(FILES, "i.consume_key(M::ALT, Key::ArrowUp) || i.consume_key(M::COMMAND, Key::ArrowUp) || i.consume_key(M::NONE, Key::Backspace)")]),
            b("Alt+← / Alt+→", "⌥← / ⌥→", "Back and Forward.", &[(FILES, "i.consume_key(M::ALT, Key::ArrowLeft)"), (FILES, "i.consume_key(M::ALT, Key::ArrowRight)")]),
            b("Ctrl+A", "⌘A", "Select everything shown.", &[(FILES, "return Some(Do::All);")]),
            b("Escape", "", "Select nothing.", &[(FILES, "i.consume_key(M::NONE, Key::Escape)")]),
            b("Type a name", "", "Jump to the first item whose name starts with what you type.", &[(FILES, "return Some(Do::Find(typed));")]),
            b("Ctrl+L, Alt+D or F4", "⌘L", "Type a path (works while typing elsewhere too). Start with > for commands or @ for favourites.", &[(FILES, "i.consume_key(M::COMMAND, Key::L) || (!cfg!(target_os = \"macos\") && (i.consume_key(M::ALT, Key::D) || i.consume_key(M::NONE, Key::F4)))")]),
            b("Ctrl+T", "⌘T", "New tab, showing the same folder.", &[(FILES, "return Some(Do::NewTab);")]),
            b("Ctrl+W", "⌘W", "Close the tab.", &[(FILES, "return Some(Do::CloseTab);")]),
            b("Ctrl+Shift+T", "⌘⇧T", "Reopen the tab you closed last.", &[(FILES, "i.consume_key(M::COMMAND | M::SHIFT, Key::T)")]),
            b("Ctrl+Tab / Ctrl+Shift+Tab", "⌃Tab / ⌃⇧Tab", "Next or previous tab.", &[(FILES, "return Some(Do::NextTab(true));"), (FILES, "return Some(Do::NextTab(false));")]),
            b("F5 or Ctrl+R", "F5 or ⌘R", "Read the folder again.", &[(FILES, "i.consume_key(M::NONE, Key::F5) || i.consume_key(M::COMMAND, Key::R)")]),
            b("Ctrl+Shift+.", "⌘⇧.", "Show or hide hidden files and folders.", &[(FILES, "i.consume_key(M::COMMAND | M::SHIFT, Key::Period)")]),
            b("Ctrl+C / Ctrl+X / Ctrl+V", "⌘C / ⌘X / ⌘V", "Copy or cut the selected items, then paste them into the folder you're in.", &[(FILES, "let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));"), (FILES, "let cut = i.events.iter().any(|e| matches!(e, egui::Event::Cut));"), (FILES, "return Some(Do::Act(FAction::Paste));")]),
            b("Ctrl+Shift+C", "⌘⇧C", "Copy the full paths of the selected items as text.", &[(FILES, "return Some(Do::Act(if i.modifiers.shift { FAction::CopyPath } else { FAction::Copy }));")]),
            b("F2", "", "Rename the selected item.", &[(FILES, "if i.consume_key(M::NONE, Key::F2) {")]),
            b("Delete", "Delete or ⌘Backspace", "Move the selected items to the Recycle Bin or Trash (it asks first).", &[(FILES, "if i.consume_key(M::NONE, Key::Delete) || (cfg!(target_os = \"macos\") && i.consume_key(M::COMMAND, Key::Backspace)) {")]),
            b("Shift+Delete", "⇧Delete", "Delete the selected items for good (it asks first).", &[(FILES, "if i.consume_key(M::SHIFT, Key::Delete) {")]),
            b("Ctrl+Shift+N", "⌘⇧N", "New folder here, with its name ready to type.", &[(FILES, "if i.consume_key(M::COMMAND | M::SHIFT, Key::N) {")]),
            b("Ctrl+Z", "⌘Z", "Undo the last copy, move, rename, new item or Move to Trash. Press again to undo the one before.", &[(FILES, "if i.consume_key(M::COMMAND, Key::Z) {"), (MENUS, "pub(super) fn undo_last(&mut self) {")]),
            b("Ctrl+Y or Ctrl+Shift+Z", "⌘Y or ⇧⌘Z", "Redo what was just undone.", &[(FILES, "if i.consume_key(M::COMMAND | M::SHIFT, Key::Z) || i.consume_key(M::COMMAND, Key::Y) {"), (MENUS, "pub(super) fn redo_last(&mut self) {")]),
            b("F6", "", "Move the keyboard to the sidebar, and back to the list. In the sidebar ↑ and ↓ move, Enter opens and Escape goes back.", &[(FILES, "if ctx.input_mut(|i| i.consume_key(M::NONE, Key::F6)) {")]),
            b("Alt+Enter", "⌥Enter", "Properties: size, contents and dates.", &[(FILES, "if i.consume_key(M::ALT, Key::Enter) {")]),
            b("Shift+F10", "", "Windows: Windows' own menu for the selected items, with what other apps add to it.", &[(FILES, "if cfg!(windows) && i.consume_key(M::SHIFT, Key::F10) {")]),
            b("Space", "", "Quick Look: a large preview of the selected file. ↑ / ↓ move to the next file while it's open; Space or Escape closes it.", &[(FILES, "if !look && i.consume_key(M::NONE, Key::Space) {"), (PREVIEW, "i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) || i.consume_key(egui::Modifiers::NONE, egui::Key::Space)")]),
            b("Alt+P", "⌥P", "Show or hide the preview pane (works while typing elsewhere too).", &[(FILES, "i.consume_key(M::ALT, Key::P)")]),
            b("Ctrl+\\", "⌘\\", "One pane or two side by side (works while typing elsewhere too).", &[(FILES, "i.consume_key(M::COMMAND, Key::Backslash)")]),
            b("Tab", "", "Two panes: work in the other pane.", &[(FILES, "if i.consume_key(M::NONE, Key::Tab) {")]),
            b("Shift+F5 / Shift+F6", "⇧F5 / ⇧F6", "Two panes: copy or move the selected items to the other pane's folder.", &[(FILES, "if i.consume_key(M::SHIFT, Key::F5) {"), (FILES, "if i.consume_key(M::SHIFT, Key::F6) {")]),
            b("Ctrl+U", "⌘U", "Two panes: swap them.", &[(FILES, "if i.consume_key(M::COMMAND, Key::U) {")]),
            b("Ctrl+J", "⌘J", "Show or hide the analysis dock (works while typing elsewhere too).", &[(FILES, "i.consume_key(M::COMMAND, Key::J)")]),
        ],
    },
    Group {
        title: "Remote workspace",
        note: "Everything in the Files workspace works here too. These are different or extra.",
        bindings: &[
            b("F5 / F6", "", "Two panes: copy or move the selected items to the other pane's folder, as in WinSCP. Ctrl+R reads the folder again.", &[(REMOTE, "two && i.consume_key(M::NONE, Key::F5)"), (REMOTE, "two && i.consume_key(M::NONE, Key::F6)")]),
            b("Ctrl+Alt+T", "", "Open an SSH terminal in the folder (SFTP servers).", &[(REMOTE, "i.consume_key(M::CTRL | M::ALT, Key::T)")]),
        ],
    },
    Group {
        title: "Sequence player",
        note: "While an image sequence is playing (Enter or double-click on a sequence in Files).",
        bindings: &[
            b("Space", "", "Play or pause.", &[(PLAYER, "if i.consume_key(M::NONE, Key::Space)")]),
            b("L / J / K", "", "Play forwards, play backwards, pause.", &[(PLAYER, "if i.consume_key(M::NONE, Key::L) {"), (PLAYER, "if i.consume_key(M::NONE, Key::J) {"), (PLAYER, "i.consume_key(M::NONE, Key::K)")]),
            b("← / →", "", "Back or on one frame (with Shift, ten).", &[(PLAYER, "if step(i, Key::ArrowLeft) {"), (PLAYER, "if step(i, Key::ArrowRight) {")]),
            b("Home / End", "", "First or last frame.", &[(PLAYER, "if i.consume_key(M::NONE, Key::Home) {"), (PLAYER, "if i.consume_key(M::NONE, Key::End) {")]),
            b("F", "", "Fit the frame to the window.", &[(PLAYER, "if i.consume_key(M::NONE, Key::F) {")]),
            b("Escape", "", "Close the player.", &[(PLAYER, "if i.consume_key(M::NONE, Key::Escape) {")]),
        ],
    },
    Group {
        title: "Treemap",
        note: "In Storage Analysis, when the treemap is showing (not the sunburst), the cursor isn't in a text box, and you last clicked the map (or the file manager is in its Tree layout).",
        bindings: &[
            b("+ or =", "", "Zoom in, around the pointer (or the middle of the map).", &[(APP, "egui::Key::Plus"), (APP, "egui::Key::Equals")]),
            b("−", "", "Zoom out, around the pointer (or the middle of the map).", &[(APP, "egui::Key::Minus")]),
            b("Home", "", "Whole map: back to the complete scan.", &[(APP, "egui::Key::Home")]),
            b("Alt+←", "⌥←", "Back to the previous place.", &[(APP, "alt && i.key_pressed(egui::Key::ArrowLeft)")]),
            b("Alt+→", "⌥→", "Forward again.", &[(APP, "alt && i.key_pressed(egui::Key::ArrowRight)")]),
        ],
    },
    Group {
        title: "File manager",
        note: "In Storage Analysis, after you've clicked in the file manager (not the map), in the List, Icons and Columns layouts, unless the cursor is in a text box.",
        bindings: &[
            b("↑ / ↓", "", "Select the item above or below (in Icons, the row above or below).", &[(BROWSER, "return Some(Do::Move(cols));"), (BROWSER, "return Some(Do::Move(-cols));")]),
            b("← / →", "", "Icons: the previous or next item. Columns: up to the enclosing folder, or into the selected folder.", &[(BROWSER, "return Some(Do::Move(-1));"), (BROWSER, "return Some(Do::Into);")]),
            b("Home / End", "", "Select the first or last item.", &[(BROWSER, "i.consume_key(M::NONE, Key::Home)"), (BROWSER, "i.consume_key(M::NONE, Key::End)")]),
            b("Page Up / Page Down", "", "Move 20 items (20 rows of icons).", &[(BROWSER, "i.consume_key(M::NONE, Key::PageDown)"), (BROWSER, "i.consume_key(M::NONE, Key::PageUp)")]),
            b("Enter", "", "Open the selected folder (the map follows). A selected file opens with its usual app.", &[(MENUS, "self.browser_has_keys() && info.is_dir && ctx.input_mut(|i| i.consume_key(M::NONE, Key::Enter))")]),
            b("Ctrl+↓", "⌘↓", "Open the selected folder or file.", &[(BROWSER, "i.consume_key(M::COMMAND, Key::ArrowDown)")]),
            b("Backspace or Ctrl+↑", "Backspace or ⌘↑", "Up to the enclosing folder, selecting the one you were in.", &[(BROWSER, "i.consume_key(M::COMMAND, Key::ArrowUp) || i.consume_key(M::NONE, Key::Backspace)")]),
            b("Alt+←", "⌥←", "Back to the previous folder.", &[(BROWSER, "i.consume_key(M::ALT, Key::ArrowLeft)")]),
            b("Alt+→", "⌥→", "Forward again.", &[(BROWSER, "i.consume_key(M::ALT, Key::ArrowRight)")]),
            b("Ctrl+A", "⌘A", "Select everything in the folder.", &[(BROWSER, "i.consume_key(M::COMMAND, Key::A)")]),
            b("Type a name", "", "Jump to the first item whose name starts with what you type.", &[(BROWSER, "(!typed.is_empty()).then_some(Do::Find(typed))")]),
            b("Ctrl+T", "⌘T", "New tab, showing the same folder.", &[(BROWSER, "i.consume_key(M::COMMAND, Key::T)")]),
            b("Ctrl+W", "⌘W", "Close the tab.", &[(BROWSER, "i.consume_key(M::COMMAND, Key::W)")]),
            b("Ctrl+Tab / Ctrl+Shift+Tab", "⌃Tab / ⌃⇧Tab", "Next or previous tab.", &[(BROWSER, "i.consume_key(M::CTRL, Key::Tab)"), (BROWSER, "i.consume_key(M::CTRL | M::SHIFT, Key::Tab)")]),
            b("Ctrl+Shift+N", "⌘⇧N", "New folder in the current folder, ready to name.", &[(BROWSER, "i.consume_key(M::COMMAND | M::SHIFT, Key::N)")]),
            b("Ctrl+Z", "⌘Z", "Undo the last file change, as in the Files workspace.", &[(BROWSER, "if i.consume_key(M::COMMAND, Key::Z) {")]),
            b("Ctrl+V", "⌘V", "Paste: copy or move what you copied or cut into the current folder. Paths copied as text in another app are copied in too.", &[(BROWSER, "egui::Event::Paste(t) => Some(t.clone()),")]),
        ],
    },
    Group {
        title: "Selected file or folder",
        note: "In Storage Analysis. Act on the selected item, wherever it was selected (file manager, map or a list), unless the cursor is in a text box. In the file manager they act on every selected item. The same commands are in the right-click menu.",
        bindings: &[
            b("Enter", "", "Open it with its usual app (a folder opens in the file manager).", &[(MENUS, "(M::NONE, Key::Enter, TreeAction::Open)")]),
            b("Ctrl+C or Ctrl+Insert", "⌘C", "Copy it, to paste in the file manager. Its path also goes on the clipboard as text.", &[(MENUS, "egui::Event::Copy"), (MENUS, "M::CTRL, Key::Insert"), (MENUS, "return Some(TreeAction::CopyItems);")]),
            b("Ctrl+X", "⌘X", "Cut it, to move it with Paste in the file manager.", &[(MENUS, "TreeAction::CutItems })")]),
            b("Ctrl+Shift+C", "⌘⇧C", "Copy its full path.", &[(MENUS, "if copy && i.modifiers.shift {")]),
            b("F2", "", "Rename it.", &[(MENUS, "i.consume_key(M::NONE, Key::F2)")]),
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
        title: "Address bar",
        note: "While typing a path in the Files workspace.",
        bindings: &[
            b("↑ / ↓", "", "Move through the suggestions.", &[(ADDR, "i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)"), (ADDR, "i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)")]),
            b("Tab", "", "Complete the name: the chosen suggestion, or as much as the matching folders share.", &[(ADDR, "i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)")]),
            b("Enter", "", "Go there, or run the chosen command.", &[(ADDR, "let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));")]),
            b("Escape", "", "Stop typing and show the path again.", &[(ADDR, "i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)")]),
        ],
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
        title: "Files workspace",
        note: "",
        bindings: &[
            b("Click", "", "Select the item.", &[(FILES, "picks.select = Some((i, ui.input(|inp| inp.modifiers)));")]),
            b("Ctrl+click (⌘-click on a Mac)", "", "Add the item to the selection, or take it out.", &[(FILES, "if m.command {")]),
            b("Shift+click", "", "Select everything from the last item clicked to this one.", &[(FILES, "} else if m.shift && t.anchor.is_some() {")]),
            b("Double-click", "", "Open a folder, or a file with its usual app.", &[(FILES, "picks.open = Some(i);")]),
            b("Middle-click a folder, place or tab", "", "Open the folder or place in a new tab; close the tab.", &[(FILES, "if resp.middle_clicked() && e.is_dir {"), (FILES, "picks.close_tab = Some(i);")]),
            b("Right-click", "", "The item's menu: Open, Open With, Copy, Cut, Paste, Rename (several items: Batch Rename), Move to Trash, Colour, Tags and Note, Copy Path, Add to Favourites, Show in Explorer or Finder, Properties, Analyse This Folder and, with two panes, Copy or Move to the other pane. On Windows, Show More Options opens Windows' own menu. On empty space: New, Paste, Undo, Redo, Refresh and more.", &[(FILES, "if let Some(a) = item_menu(ui, &ctx_menu, c.apps, e.ext()) {"), (FILES, "empty.context_menu(|ui| {")]),
            b("Shift+right-click", "", "Windows: straight to Windows' own menu for the item, with the extra commands Explorer shows with Shift.", &[(FILES, "if cfg!(windows) && !read_only && !server && resp.secondary_clicked() && ui.input(|i| i.modifiers.shift) {")]),
            b("Drag onto a folder, a tab, a folder in the path, the other pane or a sidebar place", "", "Move the items there when it's on the same drive, copy them when it's another drive. Hold Ctrl or Alt to copy (⌥ on a Mac), Shift to move (⌘ on a Mac). The label by the pointer says which. Hold over a tab to switch to it.", &[(FILES, "egui::DragAndDrop::set_payload(ui.ctx(), dnd::DragItems(items, dirs));"), (FILEOPS, "let op = dnd::op_for(&items.0, dest, ui.input(|i| i.modifiers), &self.drives);"), (FILES, "if now - since > 0.6 {")]),
            b("Ctrl+wheel over the icons (⌘+scroll on a Mac), or pinch", "", "Icons view: make the icons bigger or smaller.", &[(FILES, "let z = ui.input(|i| i.zoom_delta());")]),
            b("Click the view buttons at the bottom right of the list", "", "Show the folder as Details (rows), Icons, Columns or Gallery. Each folder remembers its own view, sort and grouping (View > Remember Each Folder's View).", &[(FILES, "self.files.panes[pane].view = v;"), (FILES, "pub(super) fn keep_looks(&mut self) {")]),
            b("Click an item in a column at the left (Columns view)", "", "Go back to that folder with the item selected; double-click to open it.", &[(VIEWS, "picks.reveal = Some((dir.to_path_buf(), e.name.clone()));")]),
            b("Drag a folder onto Favourites", "", "Bookmark it there; nothing is moved. A line shows where it will go. Drop onto the middle of a favourite to move or copy items into that folder instead.", &[(FAVS, "let n = add(&mut self.files.prefs.favourites, g, Some(at), folders);")]),
            b("Drag a favourite", "", "Reorder it, or move it to another group. Right-click a favourite to rename or remove it; right-click a group's title for New Group, Rename and Move.", &[(FAVS, "move_fav(&mut self.files.prefs.favourites, (fg, fi), g, at);")]),
            b("Drag the line between two panes", "", "Share the room between them. Double-click to make them even.", &[(FILES, "self.files.prefs.split_ratio = r.clamp(0.15, 0.85);"), (FILES, "self.files.prefs.split_ratio = 0.5;")]),
            b("Wheel, drag or double-click on a preview", "", "Zoom, pan, and switch between fitting the image and showing it 1:1.", &[(PREVIEW, "let wheel = ui.input(|i| i.smooth_scroll_delta.y);"), (PREVIEW, "if resp.double_clicked() {")]),
            b("Click a column heading", "", "Sort by that column; click again to reverse. Folders always come first.", &[(FILES, "picks.sort = Some(col);")]),
            b("Click a folder in the path", "", "Go there. Click the empty part of the path to type one. Click the arrow after a folder for the folders inside it.", &[(ADDR, "go = Some(loc.clone());"), (ADDR, "if bg.clicked() {")]),
            b("Click a place or drive in the sidebar", "", "Go there. Right-click for Open in New Tab, Open in Other Pane, Add to Favourites, Copy Path and Analyse.", &[(FILES, "picks.go = Some(loc.clone());"), (FILES, "resp.context_menu(|ui| place_menu(ui, path, pinned, other.is_some(), picks));")]),
            b("Back / forward mouse buttons", "", "Back and Forward.", &[(FILES, "egui::PointerButton::Extra1"), (FILES, "egui::PointerButton::Extra2")]),
            b("Drag the sidebar's right edge", "", "Make the sidebar wider or narrower.", &[(FILES, "egui::Panel::left(\"places\")")]),
            b("Drag a tab", "", "Move it along the tab strip. Right-click a tab for New Tab, Close Tab and Close Other Tabs.", &[(FILES, "picks.move_tab = Some((i, i - 1));"), (FILES, "picks.close_others = Some(i);")]),
            b("Drag a divider between column headings", "", "Resize the column to its right. Double-click the divider to reset it.", &[(FILES, "resized = Some((col, -g.drag_delta().x));"), (FILES, "resized = Some((col, f32::NAN));")]),
        ],
    },
    Group {
        title: "Sequence player",
        note: "",
        bindings: &[
            b("Double-click a sequence", "", "Play it. Select two sequences and press Enter to compare them (A/B).", &[(FILES, "if let Some(c) = t.clip_of(i) {"), (FILES, "if clips.len() > 1 {")]),
            b("Click or drag in the timeline", "", "Go to that frame.", &[(PLAYER, "pl.seek(k);")]),
            b("Mouse wheel", "", "Zoom in or out around the pointer. Double-click to fit.", &[(PLAYER, "let z = (pl.zoom * (scroll * 0.002).exp())"), (PLAYER, "if resp.double_clicked() {")]),
            b("Drag the picture", "", "Move it around when zoomed in.", &[(PLAYER, "pl.pan += resp.drag_delta();")]),
            b("Drag the wipe line", "", "A/B: move the line between the two sequences.", &[(PLAYER, "pl.wipe = ((p.x - img.left()) / img.width()).clamp(0.0, 1.0);")]),
        ],
    },
    Group {
        title: "Analysis dock",
        note: "The treemap or sunburst under the list in the Files workspace. Wheel, drag and the + − Home keys work as in Storage Analysis.",
        bindings: &[
            b("Click a tile", "", "Select it. The item in the list that holds it is selected too.", &[(DOCK, "self.files.select_name(&first.as_os_str().to_string_lossy());")]),
            b("Double-click a tile", "", "Open its folder in the list; the map follows.", &[(DOCK, "self.files.go(Location::Dir(disk));")]),
            b("Right-click a tile", "", "The same menu as in the list, plus Open in List or Show in List.", &[(DOCK, "self.tile_menu(ui, &disk, is_dir);")]),
            b("Click a file type under the map", "", "Highlight those files on the map; click again to stop.", &[(DOCK, "self.highlight_ext = if on { None } else { Some(ext.clone()) };")]),
            b("Drag the line above the dock", "", "Give the list or the map more room.", &[(FILES, "egui::Panel::bottom(\"files-dock\")")]),
        ],
    },
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
        title: "File manager",
        note: "",
        bindings: &[
            b("Hover", "", "Show the item's path, size and date in the status bar.", &[(BROWSER, "picks.hover = Some(path.to_vec());")]),
            b("Click", "", "Select the item (also on the maps).", &[(BROWSER, "picks.select = Some((path.to_vec(), ui.input(|i| i.modifiers)));")]),
            b("Ctrl+click (⌘-click on a Mac)", "", "Add the item to the selection, or take it out.", &[(BROWSER, "if m.command {")]),
            b("Shift+click", "", "Select everything from the last item clicked to this one.", &[(BROWSER, "(m.shift, self.browser.anchor)")]),
            b("Double-click", "", "Open a folder (the map follows), or a file with its usual app.", &[(BROWSER, "picks.open = Some(path.to_vec());")]),
            b("Middle-click a folder", "", "Open it in a new tab.", &[(BROWSER, "if resp.middle_clicked() && n.is_dir {")]),
            b("Right-click", "", "Select the item and open the right-click menu. On empty space: New Folder, Paste, Open in New Tab.", &[(BROWSER, "resp.context_menu(|ui| {"), (BROWSER, "empty.context_menu(|ui| {")]),
            b("Drag onto a folder, a folder in the path, a tab or empty space", "", "Move the selected items there when it's on the same drive, copy them when it's another drive. Hold Ctrl or Alt to copy (⌥ on a Mac), Shift to move (⌘ on a Mac).", &[(BROWSER, "resp.dnd_set_drag_payload(DragItems(items, dirs));"), (BROWSER, "let copy = dnd::op_for(&p.0, &dest, ui.input(|i| i.modifiers), &self.drives) == dnd::DropOp::Copy;")]),
            b("Click a column heading (List)", "", "Sort by that column; click again to reverse.", &[(BROWSER, "picks.sort = col.sort();")]),
            b("Click a folder in the path", "", "Go to that folder.", &[(BROWSER, "picks.go = Some(self.browser.folder[..depth].to_vec());")]),
            b("Click a tab, or middle-click it", "", "Switch to that tab, or close it.", &[(BROWSER, "picks.close_tab = Some(i);")]),
            b("Back / forward mouse buttons", "", "Back and Forward through folders you've opened.", &[(BROWSER, "egui::PointerButton::Extra1"), (BROWSER, "egui::PointerButton::Extra2")]),
            b("Drag the border below the file manager", "", "Give the file manager or the map more room.", &[(APP, "egui::Panel::top(\"tree\").default_size(340.0).resizable(true)")]),
        ],
    },
    Group {
        title: "Tree layout",
        note: "The file manager's Tree layout.",
        bindings: &[
            b("Click", "", "Select the item.", &[(APP, "clicked = Some(row.path.to_vec());")]),
            b("Click the arrow, or double-click a folder", "", "Open or close the folder.", &[(APP, "if n.is_dir && ((resp.clicked() && on_arrow) || resp.double_clicked()) {")]),
            b("Right-click", "", "Select the item and open the right-click menu.", &[(APP, "resp.context_menu(|ui| {")]),
            b("Click a column heading", "", "Sort by that column; click again to reverse.", &[(APP, "self.sort_desc = !self.sort_desc;")]),
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
