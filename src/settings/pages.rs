//! The pages of the Settings window. Each is a list of sections built from [`crate::kit`];
//! nothing here draws its own widgets, so every page looks and behaves the same way.

use super::{LightLevel, PageCx, Settings, SizeUnits, Theme, TileStyle};
use crate::elevate::{self, AdminMode};
use crate::kit::{self, Icon, Kind, SliderRow, Tone};
use eframe::egui::{self, Id, RichText, vec2};

fn cpus() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

// ---------- General ----------

pub fn general(ui: &mut egui::Ui, cx: &mut PageCx, licence: &mut crate::licence::Licensing) {
    let f = cx.focuser();
    let d = &mut *cx.d;
    kit::section(ui, Icon::Palette, "Appearance", f.of("appearance"), |ui| {
        kit::stacked(ui, "Theme", Some("System follows your computer's light or dark mode."), |ui| {
            let gap = 12.0;
            let w = ((ui.available_width() - gap * 2.0) / 3.0).clamp(110.0, 190.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (th, name) in [(Theme::System, "System"), (Theme::Dark, "Dark"), (Theme::Light, "Light")] {
                    if kit::choice_card(ui, d.theme == th, vec2(w, w * 0.6), name, |p, r| super::preview::theme_picture(p, r, th)).clicked() {
                        d.theme = th;
                    }
                }
            });
        });
        kit::divider(ui);
        kit::stacked(ui, "Interface size", Some("Makes all text and controls bigger or smaller. Ctrl + and Ctrl − do the same anywhere in Filescape, and Ctrl 0 goes back to 100%."), |ui| {
            let mut pct = (d.ui_scale * 100.0).round() as i32;
            let mut opts: Vec<(i32, String)> = [80, 90, 100, 110, 125, 150].iter().map(|p| (*p, format!("{p}%"))).collect();
            if !opts.iter().any(|o| o.0 == pct) {
                opts.push((pct, format!("{pct}%")));
                opts.sort_by_key(|o| o.0);
            }
            let refs: Vec<(i32, &str)> = opts.iter().map(|(v, s)| (*v, s.as_str())).collect();
            if kit::segmented(ui, &mut pct, &refs, false).changed() {
                d.ui_scale = pct as f32 / 100.0;
            }
        });
        kit::divider(ui);
        kit::toggle_row(ui, "Striped rows", Some("Shade every other row so long lists are easier to follow."), &mut d.stripes);
        kit::divider(ui);
        kit::toggle_row(ui, "Explain system files", Some("Show an info icon after system files and folders that aren't obvious, like pagefile.sys, WinSxS, .Spotlight-V100 or /proc. Hover it to see what the item is and whether it's safe to delete. The preview pane and the right-click menu's What Is This? explain them too."), &mut d.system_notes);
        kit::divider(ui);
        kit::toggle_row(ui, "Backup reminder at start", Some("Each time Filescape starts, remind you to keep regular offline backups. Filescape can delete, move and overwrite files, and some of that can't be undone."), &mut d.backup_reminder);
    });

    kit::section(ui, Icon::Ruler, "File sizes", f.of("sizes"), |ui| {
        kit::stacked(ui, "Size units", None, |ui| {
            kit::segmented(ui, &mut d.units, &[(SizeUnits::Binary, "Binary  ·  1 KB = 1,024 bytes"), (SizeUnits::Decimal, "Decimal  ·  1 kB = 1,000 bytes")], true);
            ui.add_space(8.0);
            // `human` already follows the draft (the app sets the units from it each frame).
            let sample = crate::scan::human(1_500_000_000);
            kit::note(
                ui,
                match d.units {
                    SizeUnits::Binary => format!("A 1,500,000,000-byte file shows as {sample}. Binary counts in powers of 1,024, as Windows Explorer does."),
                    SizeUnits::Decimal => format!("A 1,500,000,000-byte file shows as {sample}. Decimal counts in thousands, as macOS, most Linux file managers and drive labels do."),
                },
            );
        });
    });

    kit::section(ui, Icon::Drive, "Scanning", f.of("scanning"), |ui| {
        kit::toggle_row(ui, "Live scan results", Some("Fill in the folder list while a scan runs. Off shows only the progress until it finishes."), &mut d.live_tree);
        kit::toggle_row(ui, "Keep up with changes", Some("Add, update and remove items in the analysis as files change after the scan (downloads, files deleted or moved in other apps), without scanning again. Only the changed items are read, and cloud files are never downloaded."), &mut d.watch_changes);
    });

    updates(ui, cx, licence);
}

/// Settings > Files.
pub fn files(ui: &mut egui::Ui, cx: &mut PageCx) {
    let f = cx.focuser();
    let d = &mut *cx.d;
    kit::section(ui, Icon::Folder, "Applications & Executables", f.of("apps"), |ui| {
        kit::toggle_row(ui, "Recognise application bundles", Some("Show applications (Safari.app and the like) as applications rather than folders, and name plug-ins, frameworks and other bundles in the Kind column. Off shows them all as plain folders."), &mut d.app_recognise);
        let on = d.app_recognise;
        ui.add_enabled_ui(on, |ui| {
            kit::divider(ui);
            kit::toggle_row(ui, "Show application icons", Some("Draw applications with their own icons, as Finder does. Icons are read once and kept while Filescape is open; apps in cloud folders keep a plain icon so nothing is downloaded."), &mut d.app_icons);
            kit::divider(ui);
            kit::toggle_row(ui, "Hide .app extensions", Some("Show “Safari” rather than “Safari.app”. Renaming still shows the whole name."), &mut d.app_hide_ext);
            kit::divider(ui);
            ui.add_enabled_ui(!d.app_as_folders, |ui| {
                let mut launch = d.app_double_click && !d.app_as_folders;
                if kit::toggle_row(ui, "Double-click to launch applications", Some("Double-click or Enter opens an application. Off, they show its contents instead, and Open Application in the right-click menu still launches it."), &mut launch).changed() {
                    d.app_double_click = launch;
                }
            });
            kit::divider(ui);
            kit::toggle_row(ui, "Treat application bundles as folders", Some("For developers and administrators: double-click and Enter go inside applications like any folder, and the right-click menu has the folder commands too. Applications keep their icons and names."), &mut d.app_as_folders);
        });
    });
}

fn updates(ui: &mut egui::Ui, cx: &mut PageCx, licence: &mut crate::licence::Licensing) {
    let f = cx.focuser();
    use crate::licence::update::{VERSION, platform};
    let st = licence.update_status();
    let aside = match &st.found {
        Some(m) => Some((format!("{} available", m.version), Tone::Accent)),
        None if st.checking => Some(("Checking…".to_string(), Tone::Neutral)),
        None if st.failed => Some(("Couldn't check".to_string(), Tone::Warn)),
        None if st.checked_at.is_some() => Some(("Up to date".to_string(), Tone::Success)),
        None => None,
    };
    let focus = f.of("updates");
    let d = &mut *cx.d;
    let help = &mut *cx.help;
    kit::section_with(ui, Icon::Refresh, "Updates", aside.as_ref().map(|(s, t)| (s.as_str(), *t)), focus, |ui| {
        kit::row(ui, "Installed version", None, |ui| {
            ui.label(RichText::new(format!("Filescape {VERSION}")).size(kit::LABEL).color(kit::tokens(ui).strong));
        });
        kit::divider(ui);
        let last = match st.checked_at {
            Some(ts) => {
                let when = chrono::DateTime::from_timestamp(ts, 0).map(|t| t.with_timezone(&chrono::Local));
                let today = chrono::Local::now().date_naive();
                when.map(|t| if t.date_naive() == today { format!("Today at {}", t.format("%H:%M")) } else { t.format("%-d %b %Y, %H:%M").to_string() }).unwrap_or_default()
            }
            None => "Not yet".into(),
        };
        let desc = if st.failed { Some("The last check couldn't reach filescape.app. Check your connection and try again.") } else { None };
        kit::row(ui, "Last successful check", desc, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let r = ui.add_enabled_ui(!st.checking, |ui| kit::button(ui, Kind::Secondary, Some(Icon::Refresh), if st.checking { "Checking…" } else { "Check now" })).inner;
            if r.on_hover_text("Ask filescape.app for the newest version now").clicked() {
                licence.check_updates_now();
            }
            ui.label(RichText::new(last).size(kit::LABEL - 0.5).color(kit::tokens(ui).text));
        });
        if let Some(m) = &st.found {
            kit::callout(ui, if st.renew { Tone::Warn } else { Tone::Accent }, format!("Filescape {} is available. {}", m.version, st.coverage_text));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if kit::button(ui, Kind::Primary, Some(Icon::External), "Download").on_hover_text(m.download_for(platform())).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(m.download_for(platform())));
                }
            });
            ui.add_space(6.0);
        }
        kit::divider(ui);
        kit::toggle_row(ui, "Check automatically", Some("At each launch, ask filescape.app for the newest version number. Nothing about you or this computer is sent."), &mut d.check_updates);
        kit::divider(ui);
        kit::row(ui, "Release notes", Some("What changed in each version."), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(url) = st.found.as_ref().and_then(|m| m.notes().map(String::from))
                && kit::button(ui, Kind::Secondary, Some(Icon::External), "What's new online").on_hover_text(url.clone()).clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
            if kit::button(ui, Kind::Secondary, Some(Icon::Help), "What's new in this version").clicked() {
                *help = Some(crate::help::topics::WHATS_NEW);
            }
        });
    });
}

// ---------- Filtering ----------

#[derive(Clone, Copy, PartialEq)]
enum RuleKind {
    Folder,
    Skip,
    Only,
}

/// Lines of a rule list, trimmed, without blanks.
fn rules(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// What's wrong with a rule, if anything.
fn rule_problem(kind: RuleKind, rule: &str, all: &[String]) -> Option<&'static str> {
    if all.iter().filter(|r| *r == rule).count() > 1 {
        return Some("Listed more than once.");
    }
    match kind {
        RuleKind::Folder => {
            let p = std::path::Path::new(rule);
            (!p.is_absolute() && !rule.starts_with('/') && !rule.starts_with('\\'))
                .then_some("Not a full path, so it never matches. Give the whole path, or use Browse.")
        }
        _ => (rule.contains('/') || rule.contains('\\')).then_some("Patterns match file names only, not folders or paths. Skip a folder in the list above instead."),
    }
}

/// An editable list of rules kept in `text` as one per line (the saved format). Returns
/// whether it changed.
fn rule_list(ui: &mut egui::Ui, text: &mut String, kind: RuleKind) -> bool {
    let t = kit::tokens(ui);
    let id = Id::new(("rule-list", kind as u8));
    let mut list = rules(text);
    let mut changed = false;
    let mut remove = None;
    let editing: Option<usize> = ui.data(|m| m.get_temp(id.with("edit")));
    if list.is_empty() {
        kit::note(
            ui,
            match kind {
                RuleKind::Folder => "No folders are skipped.",
                RuleKind::Skip => "No file names are skipped.",
                RuleKind::Only => "Every file name is counted.",
            },
        );
        ui.add_space(4.0);
    }
    for i in 0..list.len() {
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            kit::icons::show(ui, if kind == RuleKind::Folder { Icon::Folder } else { Icon::File }, 14.0, t.weak);
            if editing == Some(i) {
                let mut buf: String = ui.data(|m| m.get_temp(id.with("buf"))).unwrap_or_else(|| list[i].clone());
                let r = kit::field(ui, &mut buf, "", (ui.available_width() - 70.0).max(120.0));
                r.request_focus();
                let done = r.lost_focus();
                ui.data_mut(|m| m.insert_temp(id.with("buf"), buf.clone()));
                if done || kit::icon_button(ui, Icon::Check, "Done").clicked() {
                    let v = buf.trim().to_string();
                    if v.is_empty() {
                        remove = Some(i);
                    } else if v != list[i] {
                        list[i] = v;
                        changed = true;
                    }
                    ui.data_mut(|m| {
                        m.remove::<usize>(id.with("edit"));
                        m.remove::<String>(id.with("buf"));
                    });
                }
            } else {
                let problem = rule_problem(kind, &list[i], &list);
                let w = ui.available_width() - 70.0;
                ui.allocate_ui(vec2(w, 22.0), |ui| {
                    ui.add(egui::Label::new(RichText::new(&list[i]).monospace().size(12.5).color(if problem.is_some() { t.warn } else { t.strong })).truncate());
                    if let Some(p) = problem {
                        kit::icons::show(ui, Icon::Warning, 14.0, t.warn).on_hover_text(p);
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if kit::icon_button(ui, Icon::Trash, "Remove").clicked() {
                        remove = Some(i);
                    }
                    if kit::icon_button(ui, Icon::Pencil, "Edit").clicked() {
                        ui.data_mut(|m| {
                            m.insert_temp(id.with("edit"), i);
                            m.insert_temp(id.with("buf"), list[i].clone());
                        });
                    }
                });
            }
        });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    // Add a rule.
    ui.add_space(4.0);
    let mut new: String = ui.data(|m| m.get_temp(id.with("new"))).unwrap_or_default();
    let mut add = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let hint = match kind {
            RuleKind::Folder if cfg!(windows) => "Add a folder, e.g. C:\\Windows\\WinSxS",
            RuleKind::Folder => "Add a folder, e.g. /home/me/.cache",
            RuleKind::Skip => "Add a pattern, e.g. *.tmp",
            RuleKind::Only => "Add a pattern, e.g. *.mov",
        };
        let extra = if kind == RuleKind::Folder { 190.0 } else { 90.0 };
        let r = kit::field(ui, &mut new, hint, (ui.available_width() - extra).max(140.0));
        add = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        add |= ui.add_enabled_ui(!new.trim().is_empty(), |ui| kit::button(ui, Kind::Secondary, Some(Icon::Plus), "Add")).inner.clicked();
        let picked = ui.id().with("rule-browse");
        if kind == RuleKind::Folder && kit::button(ui, Kind::Secondary, Some(Icon::Folder), "Browse…").on_hover_text("Pick a folder to skip").clicked() {
            let mailbox = crate::asking::ask_into(crate::asking::Dialog::new(), crate::asking::Pick::Folder, ui.ctx());
            ui.data_mut(|d| d.insert_temp(picked, mailbox));
        }
        if let Some(f) = ui.data(|d| d.get_temp::<crate::asking::Mailbox>(picked)).and_then(|m| m.lock().ok()?.take()) {
            ui.data_mut(|d| d.remove::<crate::asking::Mailbox>(picked));
            new = f.display().to_string();
            add = true;
        }
    });
    if add && !new.trim().is_empty() {
        let v = new.trim().to_string();
        if !list.contains(&v) {
            list.push(v);
            changed = true;
        }
        new.clear();
    }
    ui.data_mut(|m| m.insert_temp(id.with("new"), new));
    if changed {
        *text = list.join("\n");
    }
    changed
}

/// A unit for sizes, as a power of 1,024 or 1,000.
const SIZE_UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];

fn size_input(ui: &mut egui::Ui, bytes: &mut u64, units: SizeUnits) {
    let base: u64 = if units == SizeUnits::Binary { 1024 } else { 1000 };
    let id = Id::new("min-size-unit");
    let mut unit: usize = ui.data(|m| m.get_temp(id)).unwrap_or_else(|| {
        // The largest unit the saved value is a whole number of.
        (0..SIZE_UNITS.len()).rev().find(|u| *bytes > 0 && *bytes % base.pow(*u as u32) == 0).unwrap_or(2)
    });
    let mult = base.pow(unit as u32) as f64;
    let mut v = *bytes as f64 / mult;
    ui.spacing_mut().item_spacing.x = 6.0;
    let before = unit;
    egui::ComboBox::from_id_salt(id).width(70.0).selected_text(SIZE_UNITS[unit]).show_ui(ui, |ui| {
        for (k, name) in SIZE_UNITS.iter().enumerate() {
            let name = if k == 1 && units == SizeUnits::Decimal { "kB" } else { name };
            ui.selectable_value(&mut unit, k, name);
        }
    });
    if ui.add(egui::DragValue::new(&mut v).range(0.0..=1.0e6).speed(0.5).max_decimals(if unit == 0 { 0 } else { 2 })).changed() || unit != before {
        *bytes = (v * base.pow(unit as u32) as f64).round() as u64;
    }
    ui.data_mut(|m| m.insert_temp(id, unit));
}

const AGE_UNITS: [(&str, u32); 4] = [("days", 1), ("weeks", 7), ("months", 30), ("years", 365)];

fn age_input(ui: &mut egui::Ui, days: &mut u32) {
    let id = Id::new("max-age-unit");
    let mut unit: usize = ui.data(|m| m.get_temp(id)).unwrap_or_else(|| (0..AGE_UNITS.len()).rev().find(|u| *days > 0 && *days % AGE_UNITS[*u].1 == 0).unwrap_or(0));
    let mut v = *days / AGE_UNITS[unit].1;
    ui.spacing_mut().item_spacing.x = 6.0;
    let before = unit;
    egui::ComboBox::from_id_salt(id).width(80.0).selected_text(AGE_UNITS[unit].0).show_ui(ui, |ui| {
        for (k, (name, _)) in AGE_UNITS.iter().enumerate() {
            ui.selectable_value(&mut unit, k, *name);
        }
    });
    if ui.add(egui::DragValue::new(&mut v).range(1..=36500 / AGE_UNITS[unit].1).speed(0.2)).changed() || unit != before {
        *days = (v.max(1) * AGE_UNITS[unit].1).min(36500);
    }
    ui.data_mut(|m| m.insert_temp(id, unit));
}

pub fn filtering(ui: &mut egui::Ui, cx: &mut PageCx) {
    let f = cx.focuser();
    let stale = cx.env.scanned.is_some_and(|s| s.scan_key() != cx.d.scan_key());
    let d = &mut *cx.d;
    let (folders, skip, only) = (rules(&d.exclude_folders), rules(&d.exclude_files), rules(&d.include_files));
    let aside = if stale { ("Rescan needed", Tone::Warn) } else { ("Applies to the next scan", Tone::Neutral) };
    kit::section_with(ui, Icon::Filter, "Active rules", Some(aside), f.of("summary"), |ui| {
        let mut parts = Vec::new();
        let n = |k: usize, one: &str, many: &str| if k == 1 { format!("1 {one}") } else { format!("{k} {many}") };
        if !folders.is_empty() {
            parts.push(n(folders.len(), "folder skipped", "folders skipped"));
        }
        if !skip.is_empty() {
            parts.push(n(skip.len(), "name pattern skipped", "name patterns skipped"));
        }
        if !only.is_empty() {
            parts.push(format!("only {} counted", n(only.len(), "name pattern", "name patterns")));
        }
        if d.min_file_size > 0 {
            parts.push(format!("files under {} skipped", crate::scan::human(d.min_file_size)));
        }
        if d.max_age_days > 0 {
            parts.push(format!("files unchanged for {} days skipped", d.max_age_days));
        }
        ui.add_space(8.0);
        if parts.is_empty() {
            kit::note(ui, "No filters: every file and folder is counted.");
        } else {
            let mut s = parts.join(", ");
            s[..1].make_ascii_uppercase();
            ui.add(egui::Label::new(RichText::new(format!("{s}.")).size(kit::LABEL).color(kit::tokens(ui).strong)).wrap());
        }
        ui.add_space(6.0);
        kit::note(
            ui,
            if stale {
                "The scan on screen was made with different rules. Rescan (F5) to see these applied; until then its totals don't reflect them."
            } else {
                "Skipped items aren't counted, so totals leave them out. Patterns use * for any run of characters and ? for one character."
            },
        );
        ui.add_space(8.0);
    });

    kit::section(ui, Icon::Folder, "Skipped folders", f.of("folders"), |ui| {
        kit::stacked(ui, "Folders to skip", Some("Each folder and everything in it is left out of scans."), |ui| rule_list(ui, &mut d.exclude_folders, RuleKind::Folder));
    });

    kit::section(ui, Icon::File, "File names", f.of("names"), |ui| {
        kit::stacked(ui, "Skip files named", Some("Files whose names match any of these patterns aren't counted."), |ui| {
            rule_list(ui, &mut d.exclude_files, RuleKind::Skip);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                ui.label(RichText::new("Common:").size(kit::DESC).color(kit::tokens(ui).weak));
                let mut list = rules(&d.exclude_files);
                for ex in ["*.tmp", "*.bak", "~$*", "*.log", "Thumbs.db", ".DS_Store"] {
                    let has = list.iter().any(|r| r == ex);
                    if ui.add_enabled_ui(!has, |ui| kit::button(ui, Kind::Ghost, Some(Icon::Plus), ex)).inner.on_hover_text(format!("Skip files named {ex}")).clicked() {
                        list.push(ex.into());
                        d.exclude_files = list.join("\n");
                        break;
                    }
                }
            });
        });
        kit::divider(ui);
        kit::stacked(ui, "Only count files named", Some("Leave empty to count every file. With any patterns here, only files matching one of them are counted."), |ui| {
            rule_list(ui, &mut d.include_files, RuleKind::Only);
        });
        kit::divider(ui);
        kit::stacked(ui, "Try a file name", Some("Check what these rules do with a name."), |ui| {
            let id = Id::new("filter-try");
            let mut name: String = ui.data(|m| m.get_temp(id)).unwrap_or_default();
            ui.horizontal(|ui| {
                kit::field(ui, &mut name, "A file name, e.g. report.tmp", 240.0);
                let name = name.trim();
                if !name.is_empty() {
                    let skip = rules(&d.exclude_files);
                    let only = rules(&d.include_files);
                    if let Some(g) = skip.iter().find(|g| crate::scan::glob(g, name)) {
                        kit::badge(ui, &format!("Skipped: matches {g}"), Tone::Warn);
                    } else if !only.is_empty() && !only.iter().any(|g| crate::scan::glob(g, name)) {
                        kit::badge(ui, "Skipped: matches no \"only count\" pattern", Tone::Warn);
                    } else {
                        kit::badge(ui, "Counted", Tone::Success);
                    }
                }
            });
            ui.data_mut(|m| m.insert_temp(id, name));
        });
    });

    kit::section(ui, Icon::Clock, "Size and age", f.of("sizeage"), |ui| {
        let mut small = d.min_file_size > 0;
        kit::row(ui, "Skip small files", Some("Leave out files smaller than this size."), |ui| {
            if kit::toggle(ui, &mut small).changed() {
                d.min_file_size = if small { 1024 } else { 0 };
            }
            if small {
                size_input(ui, &mut d.min_file_size, d.units);
            }
        });
        kit::divider(ui);
        let mut old = d.max_age_days > 0;
        kit::row(ui, "Skip old files", Some("Count only files changed within this time."), |ui| {
            if kit::toggle(ui, &mut old).changed() {
                d.max_age_days = if old { 365 } else { 0 };
            }
            if old {
                age_input(ui, &mut d.max_age_days);
            }
        });
    });
}

// ---------- Folder list ----------

pub fn folder_list(ui: &mut egui::Ui, cx: &mut PageCx) {
    let f = cx.focuser();
    let d = &mut *cx.d;
    kit::section(ui, Icon::Columns, "Preview", f.of("preview"), |ui| {
        ui.add_space(10.0);
        super::preview::folder_list(ui, d);
        ui.add_space(10.0);
    });
    kit::section(ui, Icon::List, "Columns", f.of("columns"), |ui| {
        let fixed = |ui: &mut egui::Ui, name: &str, desc: &str| {
            kit::row(ui, name, Some(desc), |ui| {
                kit::badge(ui, "Always shown", Tone::Neutral);
                kit::icons::show(ui, Icon::Lock, 14.0, kit::tokens(ui).weak);
            });
        };
        fixed(ui, "Name", "The file or folder name, with its icon.");
        kit::divider(ui);
        fixed(ui, "Size", "Its size on disk, counting everything inside a folder.");
        kit::divider(ui);
        kit::toggle_row(ui, "% of parent", Some("Each item's share of the folder it's in, with a bar."), &mut d.col_percent);
        kit::divider(ui);
        kit::toggle_row(ui, "Files", Some("How many files each folder holds, counting subfolders."), &mut d.col_files);
        kit::divider(ui);
        kit::toggle_row(ui, "Last change", Some("The newest change to anything inside."), &mut d.col_modified);
    });
    kit::section(ui, Icon::Sliders, "Rows", f.of("rows"), |ui| {
        kit::toggle_row(ui, "Striped rows", Some("Shade every other row. The same setting as on the General page, and it applies to every list."), &mut d.stripes);
    });
}

// ---------- Treemap ----------

/// The Classic look presets: (name, brightness, cushion, height, scale, grid).
const PRESETS: [(&str, &str, f32, f32, f32, f32, bool); 3] = [
    ("classic", "Classic", 0.88, 0.87, 0.38, 0.91, false),
    ("soft", "Soft", 0.9, 0.5, 0.25, 0.85, false),
    ("flat", "Flat", 0.75, 0.0, 0.0, 0.0, true),
];

fn apply_preset(d: &mut Settings, p: &(&str, &str, f32, f32, f32, f32, bool)) {
    let def = Settings::default();
    (d.brightness, d.cushion, d.height, d.scale) = (p.2, p.3, p.4, p.5);
    d.show_grid = p.6;
    if p.0 == "classic" {
        (d.light_x, d.light_y) = (def.light_x, def.light_y);
    }
}

fn is_preset(d: &Settings, p: &(&str, &str, f32, f32, f32, f32, bool)) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() < 0.005;
    near(d.brightness, p.2) && near(d.cushion, p.3) && near(d.height, p.4) && near(d.scale, p.5) && d.show_grid == p.6
}

/// A square pad for the light's direction: drag the dot to where the light comes from.
fn light_pad(ui: &mut egui::Ui, x: &mut f32, y: &mut f32) -> egui::Response {
    let t = kit::tokens(ui);
    let size = 92.0;
    let (rect, mut resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::click_and_drag());
    let enabled = ui.is_enabled();
    let p = ui.painter();
    p.rect_filled(rect, kit::CONTROL_RADIUS, t.track);
    p.hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, t.divider));
    p.vline(rect.center().x, rect.y_range(), egui::Stroke::new(1.0, t.divider));
    let to = |v: f32, lo: f32, hi: f32| egui::lerp(lo..=hi, (v + 2.0) / 4.0);
    if (resp.dragged() || resp.clicked()) && enabled
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let r = rect.shrink(8.0);
        *x = (((pos.x - r.left()) / r.width()) * 4.0 - 2.0).clamp(-2.0, 2.0);
        *y = (((pos.y - r.top()) / r.height()) * 4.0 - 2.0).clamp(-2.0, 2.0);
        *x = (*x * 10.0).round() / 10.0;
        *y = (*y * 10.0).round() / 10.0;
        resp.mark_changed();
    }
    let r = rect.shrink(8.0);
    let dot = egui::pos2(to(*x, r.left(), r.right()), to(*y, r.top(), r.bottom()));
    p.line_segment([dot, rect.center()], egui::Stroke::new(1.0, t.weak));
    p.circle_filled(dot, 6.0, if enabled { t.warn } else { t.disabled });
    p.rect_stroke(egui::Rect::from_center_size(rect.center(), vec2(14.0, 10.0)), 2.0, egui::Stroke::new(1.0, t.weak), egui::StrokeKind::Middle);
    kit::focus_ring(ui, rect, &resp, kit::CONTROL_RADIUS as f32);
    resp.on_hover_text("Drag the dot to where the light comes from")
}

pub fn treemap(ui: &mut egui::Ui, cx: &mut PageCx) {
    let f = cx.focuser();
    let focus_preview = f.of("preview");
    let d = &mut *cx.d;
    let cache = &mut *cx.preview;
    kit::section(ui, Icon::Grid, "Preview", focus_preview, |ui| {
        ui.add_space(10.0);
        let h = (ui.available_width() * 0.36).clamp(150.0, 230.0);
        cache.treemap(ui, d, h);
        ui.add_space(6.0);
        kit::note(ui, "A sample folder drawn with these settings. The map in the main window previews them too, with pointer lighting and zoom.");
        ui.add_space(8.0);
    });

    kit::section(ui, Icon::Palette, "Tile style", f.of("style"), |ui| {
        ui.add_space(10.0);
        let gap = 14.0;
        let w = ((ui.available_width() - gap) / 2.0).min(230.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (st, name, slot, tip) in [
                (TileStyle::Modern, "Modern", "style-modern", "Soft pastel tiles with folder name tags, as on filescape.app"),
                (TileStyle::Classic, "Classic cushion", "style-classic", "Shaded cushion tiles in strong colours, tuned with the shading settings below"),
            ] {
                let mut look = d.clone();
                look.tile_style = st;
                let ctx = ui.ctx().clone();
                if kit::choice_card(ui, d.tile_style == st, vec2(w, w * 0.45), name, |p, r| cache.thumb(&ctx, p, r, slot, &look)).on_hover_text(tip).clicked() {
                    d.tile_style = st;
                }
            }
        });
        ui.add_space(8.0);
    });

    kit::section(ui, Icon::Pencil, "Tile details", f.of("details"), |ui| {
        kit::toggle_row(ui, "File labels", Some("Write names (and sizes) on tiles big enough to hold them, on the treemap and the sunburst."), &mut d.show_labels);
        kit::divider(ui);
        kit::toggle_row(ui, "Previews on tiles", Some("When you zoom in, tiles big enough show what's in the file: pictures, video frames, PDF pages and the first lines of text and documents. Files only in the cloud are never downloaded for this."), &mut d.tile_previews);
        kit::divider(ui);
        kit::row(ui, "Selection border", Some("The outline around the selected item."), |ui| {
            let def = Settings::default().selection_color;
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.color_edit_button_srgba(&mut d.selection_color);
            if kit::reset_button(ui, d.selection_color != def, "white") {
                d.selection_color = def;
            }
        });
    });

    let classic = d.tile_style == TileStyle::Classic;
    kit::section_with(ui, Icon::Sun, "Classic cushion shading", (!classic).then_some(("Classic style only", Tone::Neutral)), f.of("shading"), |ui| {
        if !classic {
            ui.add_space(10.0);
            kit::why_disabled(ui, "Presets, brightness, the cushion shape, light direction, grid and block depth shape the Classic cushion style. The Modern style has its own fixed look, so they're hidden while it's chosen.");
            ui.add_space(8.0);
            if kit::button(ui, Kind::Secondary, Some(Icon::Sun), "Switch to Classic cushion").clicked() {
                d.tile_style = TileStyle::Classic;
            }
            ui.add_space(10.0);
            return;
        }
        ui.add_enabled_ui(classic, |ui| {
            kit::stacked(ui, "Presets", None, |ui| {
                let gap = 12.0;
                let w = ((ui.available_width() - gap * 2.0) / 3.0).min(150.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for p in &PRESETS {
                        let mut look = d.clone();
                        look.tile_style = TileStyle::Classic;
                        apply_preset(&mut look, p);
                        let ctx = ui.ctx().clone();
                        let slot: &'static str = match p.0 {
                            "classic" => "preset-classic",
                            "soft" => "preset-soft",
                            _ => "preset-flat",
                        };
                        if kit::choice_card(ui, classic && is_preset(d, p), vec2(w, w * 0.5), p.1, |pt, r| cache.thumb(&ctx, pt, r, slot, &look)).clicked() {
                            apply_preset(d, p);
                        }
                    }
                });
            });
            kit::divider(ui);
            let def = Settings::default();
            let pct = (kit::pct as fn(f32) -> f64, kit::pct_back as fn(f64) -> f32);
            SliderRow { label: "Brightness", desc: Some("Overall brightness of the tiles."), value: &mut d.brightness, range: 0.0..=1.0, default: def.brightness, scale: pct, suffix: "", decimals: 0, logarithmic: false }.show(ui);
            SliderRow { label: "Cushion shading", desc: Some("How strongly shading shows folder boundaries (0 is flat)."), value: &mut d.cushion, range: 0.0..=1.0, default: def.cushion, scale: pct, suffix: "", decimals: 0, logarithmic: false }.show(ui);
            SliderRow { label: "Height", desc: Some("How rounded each folder's cushion is."), value: &mut d.height, range: 0.0..=1.0, default: def.height, scale: pct, suffix: "", decimals: 0, logarithmic: false }.show(ui);
            SliderRow { label: "Scale factor", desc: Some("How much of the cushion shape carries down to deeper folders."), value: &mut d.scale, range: 0.0..=1.0, default: def.scale, scale: pct, suffix: "", decimals: 0, logarithmic: false }.show(ui);
            kit::divider(ui);
            kit::stacked(ui, "Light direction", Some("Where the light falls from. Negative x is from the left, negative y from the top."), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    light_pad(ui, &mut d.light_x, &mut d.light_y);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("x").color(kit::tokens(ui).weak));
                            ui.add(egui::DragValue::new(&mut d.light_x).range(-2.0..=2.0).speed(0.02).fixed_decimals(1));
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("y").color(kit::tokens(ui).weak));
                            ui.add(egui::DragValue::new(&mut d.light_y).range(-2.0..=2.0).speed(0.02).fixed_decimals(1));
                        });
                        if kit::reset_button(ui, (d.light_x, d.light_y) != (def.light_x, def.light_y), "from the top left") {
                            (d.light_x, d.light_y) = (def.light_x, def.light_y);
                        }
                    });
                });
            });
            kit::divider(ui);
            kit::row(ui, "Grid", Some("Draw a line around every tile."), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                kit::toggle(ui, &mut d.show_grid);
                ui.add_enabled_ui(d.show_grid && classic, |ui| ui.color_edit_button_srgba(&mut d.grid_color)).response.on_hover_text("Grid colour");
            });
            kit::divider(ui);
            kit::toggle_row(ui, "Block depth", Some("A very slight bevel so neighbouring tiles stand apart."), &mut d.depth_effect);
        });
    });

    kit::section(ui, Icon::Sun, "Lighting", f.of("lighting"), |ui| {
        kit::toggle_row(ui, "Interactive lighting", Some("A soft light follows the pointer, and the tile under it lifts slightly."), &mut d.lighting);
        kit::divider(ui);
        ui.add_enabled_ui(d.lighting, |ui| {
            kit::row(ui, "Intensity", None, |ui| {
                kit::segmented(ui, &mut d.light_level, &[(LightLevel::Subtle, "Subtle"), (LightLevel::Normal, "Normal"), (LightLevel::Strong, "Strong")], false);
            });
        });
        if !d.lighting {
            kit::why_disabled(ui, "Turn on Interactive lighting to choose its intensity.");
            ui.add_space(8.0);
        }
    });

    kit::section(ui, Icon::Compass, "Navigation", f.of("navigation"), |ui| {
        kit::toggle_row(ui, "Smooth zoom", Some("Animate zooming, panning, opening folders and Back/Forward. Off makes every change happen at once, for less motion."), &mut d.smooth_zoom);
        kit::divider(ui);
        let def = Settings::default();
        let same = (kit::same as fn(f32) -> f64, kit::same_back as fn(f64) -> f32);
        SliderRow { label: "Zoom sensitivity", desc: Some("How far each wheel notch, pinch or trackpad movement zooms."), value: &mut d.zoom_sensitivity, range: 0.25..=3.0, default: def.zoom_sensitivity, scale: same, suffix: "×", decimals: 2, logarithmic: false }.show(ui);
        kit::divider(ui);
        ui.add_enabled_ui(d.smooth_zoom, |ui| {
            SliderRow { label: "Animation speed", desc: Some("How fast animations play."), value: &mut d.anim_speed, range: 0.5..=2.5, default: def.anim_speed, scale: same, suffix: "×", decimals: 2, logarithmic: true }.show(ui);
        });
        if !d.smooth_zoom {
            kit::why_disabled(ui, "Animation speed applies only with Smooth zoom on.");
            ui.add_space(8.0);
        }
    });
}

// ---------- Advanced ----------

/// Performance profiles: preset scanner thread counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Profile {
    Auto,
    Low,
    Balanced,
    High,
    Network,
    Custom,
}

impl Profile {
    fn threads(self, cpus: usize) -> Option<usize> {
        match self {
            Profile::Auto => Some(0),
            Profile::Low => Some((cpus / 4).max(1)),
            Profile::Balanced => Some((cpus / 2).max(1)),
            Profile::High => Some((cpus * 2).min(256)),
            Profile::Network => Some(32),
            Profile::Custom => None,
        }
    }
}

pub fn cloud(ui: &mut egui::Ui, cx: &mut PageCx) {
    use crate::scan::Metric;
    let f = cx.focuser();
    let help = &mut *cx.help;
    let d = &mut *cx.d;
    kit::section(ui, Icon::Ruler, "Sizes", f.of("sizes"), |ui| {
        kit::stacked(ui, "Size shown", Some("What Storage Analysis sizes, sorts and totals by. The Size switch above the map changes it too."), |ui| {
            kit::segmented(ui, &mut d.size_metric, &[(Metric::Disk, "On disk"), (Metric::Logical, "Logical"), (Metric::CloudOnly, "Only in the cloud")], true);
            ui.add_space(8.0);
            kit::note(ui, match d.size_metric {
                Metric::Disk => "Space files take on this computer, as du and Explorer's \"Size on disk\" count it. Cloud files that are only online take none. Recommended.",
                Metric::Logical => "Each file's full size, wherever its data is. Cloud files count in full even when none of them is on this computer.",
                Metric::CloudOnly => "Only data that's in the cloud and not on this computer, to see where it is. Everything else counts as nothing.",
                Metric::Reclaimable => "Space freeing up downloaded cloud files would give back (Professional). Choose it with the Size switch above the map.",
            });
        });
        kit::divider(ui);
        kit::toggle_row(ui, "Stored column", Some("In the Storage Analysis list, add the other size and where each item's data is, when the scan has cloud files."), &mut d.cloud_column);
    });

    kit::section(ui, Icon::Cloud, "Status", f.of("status"), |ui| {
        kit::toggle_row(ui, "Status icons", Some("Show an icon after the name of cloud files: online only, partly downloaded, downloaded, always kept on this device, or unknown. Hover one to see what it means. On the treemap, tiles of files not on this computer get a cloud in the corner."), &mut d.cloud_icons);
        kit::divider(ui);
        kit::toggle_row(ui, "Fade online-only tiles", Some("At Size: Logical, draw the map tiles of online-only files faded (partly downloaded ones half as much), so the files actually on this computer stand out. At Size: On disk they take no space and don't appear."), &mut d.cloud_fade);
        kit::toggle_row(ui, "Storage breakdown", Some("Under the map, a bar splitting the scan into files not in the cloud, downloaded cloud files and data only in the cloud."), &mut d.cloud_breakdown);
    });

    kit::section(ui, Icon::Shield, "Keeping files in the cloud", f.of("downloads"), |ui| {
        kit::toggle_row(ui, "Ask before downloading", Some("Scans, folder listings, thumbnails and duplicate searches never read online-only files. When you ask for something that has to (a preview, playing a sequence, comparing contents), Filescape says how much it would download and asks first."), &mut d.cloud_confirm);
        if !d.cloud_confirm {
            kit::callout(ui, Tone::Warn, "Previews and players will download online-only files without asking. Scans and thumbnails still never do.");
            ui.add_space(8.0);
        }
        kit::divider(ui);
        kit::toggle_row(ui, "Scan mounted cloud drives", Some("Walk into cloud drives mounted inside a scanned folder (rclone, other FUSE drives, Google Drive's G: drive on Windows). Off by default: reading one asks the service about every folder in it, and how much is stored here can't be told. Scanning the drive itself always works."), &mut d.cloud_mounts);
    });

    kit::section(ui, Icon::Folder, "Cloud folders on this computer", f.of("found"), |ui| {
        let roots = crate::cloud::known();
        if roots.is_empty() {
            kit::note(ui, "None found. Filescape looks in the places cloud services use (on macOS ~/Library/CloudStorage and iCloud Drive; on Windows the sync roots Windows keeps and OneDrive's folders; on Linux the mount table and the usual folders in your home folder).");
        } else {
            for r in roots.iter() {
                kit::row(ui, r.provider.name(), Some(&format!("{} · {}", r.path.display(), r.kind.describe())), |_| {});
            }
        }
        ui.add_space(8.0);
        if ui.link("How Filescape tells where cloud files are").clicked() {
            *help = Some(crate::help::topics::CLOUD_FILES);
        }
        ui.add_space(8.0);
    });
}

pub fn previews(ui: &mut egui::Ui, cx: &mut PageCx) {
    use super::PreviewPreset as P;
    let f = cx.focuser();
    let d = &mut *cx.d;
    let def = Settings::default();
    kit::section(ui, Icon::Sliders, "Presets", f.of("presets"), |ui| {
        kit::stacked(ui, "Preview presets", Some("A starting point; the settings below can then be changed one by one."), |ui| {
            let mut p = P::of(d);
            let before = p;
            kit::segmented(ui, &mut p, &[(P::Light, "Light"), (P::Balanced, "Balanced"), (P::Everything, "Everything")], true);
            if p != before {
                p.apply(d);
            }
            ui.add_space(8.0);
            kit::note(ui, match P::of(d) {
                P::Light => "Nothing is kept on disk, and only pictures that are quick to make: no filmstrips, no searching videos for a better frame, no drawing 3D models or music for icons.",
                P::Balanced => "Everything shown, with up to 512 MB of thumbnails kept on disk so folders you've seen before show their pictures at once. Recommended.",
                P::Everything => "Everything shown, with room for 2 GB of thumbnails on disk, for big photo, video and model libraries.",
                P::Custom => "Your own mix of the settings below.",
            });
        });
    });

    kit::section(ui, Icon::Drive, "Thumbnails", f.of("thumbnails"), |ui| {
        kit::toggle_row(ui, "Keep thumbnails on disk", Some("Keep the pictures made for Icons view and map tiles, so they show at once next time, even for files on servers or in the cloud. They're kept in the system's cache folder, which backups skip. Off keeps them only while Filescape is open."), &mut d.thumb_cache);
        ui.add_enabled_ui(d.thumb_cache, |ui| {
            SliderRow {
                label: "Thumbnail cache size",
                desc: Some("The most they may take. The least recently seen are removed first when it's full."),
                value: &mut d.thumb_cache_mb,
                range: 64.0..=8192.0,
                default: def.thumb_cache_mb,
                scale: (|v| v as f64, |v| v as f32),
                suffix: " MB",
                decimals: 0,
                logarithmic: true,
            }
            .show(ui);
        });
        kit::divider(ui);
        // The folder's size, looked at now and then rather than every frame.
        let id = Id::new("settings-thumb-usage");
        let now = ui.input(|i| i.time);
        let (when, bytes, count) = ui.data(|m| m.get_temp::<(f64, u64, usize)>(id)).unwrap_or((-10.0, 0, 0));
        let (bytes, count) = if now - when > 3.0 {
            let (b, c) = crate::thumbcache::usage();
            ui.data_mut(|m| m.insert_temp(id, (now, b, c)));
            (b, c)
        } else {
            (bytes, count)
        };
        let folder = crate::thumbcache::folder().map(|p| p.display().to_string()).unwrap_or_default();
        let what = if count == 0 { "No thumbnails kept.".to_string() } else { format!("{} in {} thumbnails.", crate::scan::human(bytes), count) };
        kit::row(ui, "Kept now", Some(&format!("{what} {folder}")), |ui| {
            if kit::button(ui, Kind::Secondary, Some(Icon::Trash), "Clear").on_hover_text("Remove every kept thumbnail. They're made again as they're needed.").clicked() {
                crate::thumbcache::clear();
                ui.data_mut(|m| m.remove::<(f64, u64, usize)>(id));
            }
        });
    });

    kit::section(ui, Icon::File, "Videos", f.of("videos"), |ui| {
        kit::toggle_row(ui, "Filmstrip", Some("Under a video's preview, frames from across it; click one to play from there. Needs a system that can play videos here (on Linux, ffmpeg)."), &mut d.video_strip);
        kit::toggle_row(ui, "Better video pictures", Some("When a video's first frame is black or a fade, look at a few frames from across it and show the most telling one instead, in the preview and in Icons view."), &mut d.video_better_poster);
    });

    kit::section(ui, Icon::Grid, "3D models and music", f.of("models"), |ui| {
        kit::toggle_row(ui, "3D and MIDI pictures", Some("Draw 3D models (STL, OBJ, PLY, glTF, 3MF) and MIDI files for Icons view and map tiles. Big models take a moment each. The preview pane always shows them."), &mut d.rich_thumbs);
    });
}

pub fn advanced(ui: &mut egui::Ui, cx: &mut PageCx) {
    let f = cx.focuser();
    let env = cx.env;
    let fda_missing = cx.fda_missing;
    let d = &mut *cx.d;
    kit::section(ui, Icon::Drive, "Scanning behaviour", f.of("scanning"), |ui| {
        kit::toggle_row(ui, "Skip hidden files and folders", Some("Leave out hidden items (and system files on Windows, dot-files on Linux and macOS). Needs a rescan."), &mut d.skip_hidden);
        if d.skip_hidden {
            kit::callout(ui, Tone::Warn, "Totals leave out the space hidden items use, so they'll be lower than your system reports.");
            ui.add_space(8.0);
        }
    });

    kit::section(ui, Icon::Folder, "Filesystem", f.of("filesystem"), |ui| {
        let desc = if cfg!(windows) {
            "Don't scan other drives mounted inside the scanned folder. Windows never follows those either way; this matters on Linux and macOS. Needs a rescan."
        } else {
            "Don't scan other drives mounted inside the scanned folder, such as network shares or external disks. The --all-fs option turns it off for one run. Needs a rescan."
        };
        kit::toggle_row(ui, "Stay on the scanned drive", Some(desc), &mut d.one_file_system);
        kit::divider(ui);
        ui.add_space(8.0);
        kit::callout(ui, Tone::Neutral, "Symbolic links and junctions are never followed, so nothing is counted twice.");
        ui.add_space(8.0);
    });

    let profiles = [
        (Profile::Auto, "Automatic", "One thread per CPU thread. Recommended for most drives."),
        (Profile::Low, "Low", "A quarter of the CPU threads, so other work stays responsive while a scan runs."),
        (Profile::Balanced, "Balanced", "Half the CPU threads."),
        (Profile::High, "High", "Twice the CPU threads. Can help fast SSDs and RAID arrays; on a single hard disk it mostly adds seeking."),
        (Profile::Network, "Network", "32 threads. Each read on a network share waits for the server, so reading many folders at once hides that delay."),
        (Profile::Custom, "Custom", "Choose the number of threads yourself."),
    ];
    let n = cpus();
    kit::section(ui, Icon::Cpu, "Performance", f.of("performance"), |ui| {
        let custom_id = Id::new("threads-custom");
        let custom: bool = ui.data(|m| m.get_temp(custom_id)).unwrap_or(false);
        let mut prof = if custom {
            Profile::Custom
        } else {
            profiles.iter().map(|p| p.0).find(|p| p.threads(n) == Some(d.threads)).unwrap_or(Profile::Custom)
        };
        kit::stacked(ui, "Scanner threads", Some(&format!("How many folders are read at once. This computer has {n} CPU thread{}.", if n == 1 { "" } else { "s" })), |ui| {
            let opts: Vec<(Profile, &str)> = profiles.iter().map(|p| (p.0, p.1)).collect();
            if kit::segmented(ui, &mut prof, &opts, true).changed() {
                ui.data_mut(|m| m.insert_temp(custom_id, prof == Profile::Custom));
                if let Some(k) = prof.threads(n) {
                    d.threads = k;
                } else if d.threads == 0 {
                    d.threads = n;
                }
            }
            ui.add_space(8.0);
            let what = profiles.iter().find(|p| p.0 == prof).map(|p| p.2).unwrap_or_default();
            let shown = if d.threads == 0 { n } else { d.threads };
            kit::note(ui, format!("{what} ({shown} thread{} here.)", if shown == 1 { "" } else { "s" }));
        });
        if prof == Profile::Custom {
            kit::divider(ui);
            kit::row(ui, "Threads", Some("1 to 256. Applies from the next scan."), |ui| {
                ui.add(egui::DragValue::new(&mut d.threads).range(1..=256).speed(0.2));
            });
        }
        if d.threads > n * 2 && prof != Profile::Network {
            kit::callout(ui, Tone::Warn, "Many more threads than CPU threads rarely speeds up a local disk and can slow the rest of the computer during a scan. It suits network drives.");
            ui.add_space(8.0);
        }
    });

    permissions(ui, d, cx.restart, fda_missing, f.of("permissions"));

    kit::section(ui, Icon::Wrench, "Diagnostics", f.of("diagnostics"), |ui| {
        let dir = eframe::storage_dir(crate::APP_ID);
        kit::row(ui, "Settings folder", Some(&dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "Not available on this system".into())), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if let Some(dir) = &dir {
                if kit::icon_button(ui, Icon::Copy, "Copy the folder's path").clicked() {
                    ui.ctx().copy_text(dir.display().to_string());
                }
                if kit::button(ui, Kind::Secondary, Some(Icon::Folder), "Open").clicked() {
                    let _ = crate::actions::open(dir);
                }
            }
        });
        kit::divider(ui);
        kit::row(ui, "System details", Some(&format!("Filescape {} · {} {} · drawing with {} · {} CPU threads", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, env.renderer, n)), |ui| {
            if kit::button(ui, Kind::Secondary, Some(Icon::Copy), "Copy").on_hover_text("Copy these details for a problem report").clicked() {
                ui.ctx().copy_text(format!(
                    "Filescape {} · {} {} · drawing: {} · CPU threads: {} · scanner threads: {} · administrator: {}",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    env.renderer,
                    n,
                    if d.threads == 0 { "auto".to_string() } else { d.threads.to_string() },
                    if elevate::is_elevated() { "yes" } else { "no" }
                ));
            }
        });
    });
}

fn admin_label(m: AdminMode) -> &'static str {
    match m {
        AdminMode::Always => "Always",
        AdminMode::Ask => "Ask",
        AdminMode::Never => "Never",
    }
}

fn permissions(ui: &mut egui::Ui, d: &mut Settings, restart: &mut bool, fda_missing: bool, focus: kit::Focus) {
    let elevated = elevate::is_elevated();
    let (badge, tone) = if cfg!(windows) {
        if elevated { ("Running as administrator", Tone::Warn) } else { ("Standard user", Tone::Neutral) }
    } else if elevated {
        ("Running as root", Tone::Warn)
    } else {
        ("Your user account", Tone::Neutral)
    };
    kit::section_with(ui, Icon::Shield, "Permissions", Some((badge, tone)), focus, |ui| {
        ui.add_space(8.0);
        kit::note(ui, "Filescape doesn't need extra rights for your own files. Folders it can't read are listed as unreadable rather than stopping the scan.");
        ui.add_space(6.0);
        if elevate::can_elevate() {
            // Windows: the UAC choice.
            kit::divider(ui);
            kit::row(ui, "Run as administrator at start", Some("Only needed to measure protected system folders such as C:\\Windows or other users' profiles."), |ui| {
                kit::segmented(ui, &mut d.admin, &[(AdminMode::Always, admin_label(AdminMode::Always)), (AdminMode::Ask, admin_label(AdminMode::Ask)), (AdminMode::Never, admin_label(AdminMode::Never))], false)
                    .on_hover_text("Always: restart as administrator at every start. Ask: offer it when Filescape isn't. Never: don't ask.");
            });
            if !elevated {
                kit::divider(ui);
                kit::row(ui, "Restart as administrator", Some("For this session only. Windows asks you to confirm."), |ui| {
                    if kit::button(ui, Kind::Secondary, Some(Icon::Shield), "Restart now").clicked() {
                        *restart = true;
                    }
                });
            } else {
                kit::callout(ui, Tone::Warn, "While running as administrator, files you create or move here belong to the administrator. Use it for measuring, then restart normally.");
                ui.add_space(8.0);
            }
        } else if cfg!(target_os = "macos") {
            kit::divider(ui);
            kit::row(
                ui,
                "Full Disk Access",
                Some("macOS keeps other apps' data and some folders (Mail, Messages, Safari, Time Machine) private unless an app has Full Disk Access. Turn it on in System Settings, then quit and reopen Filescape."),
                |ui| {
                    if kit::button(ui, Kind::Secondary, Some(Icon::External), "Open System Settings…").clicked() {
                        elevate::open_full_disk_access_settings();
                    }
                },
            );
            if fda_missing {
                kit::callout(ui, Tone::Warn, "Filescape doesn't have Full Disk Access yet, so protected folders are counted as unreadable.");
                ui.add_space(8.0);
            } else {
                kit::note(ui, "There's no need to run Filescape with sudo on macOS.");
                ui.add_space(8.0);
            }
        } else {
            kit::divider(ui);
            ui.add_space(8.0);
            kit::callout(
                ui,
                if elevated { Tone::Warn } else { Tone::Neutral },
                if elevated {
                    "Filescape is running as root. Use that for a one-off scan of system folders; files you create or move while root belong to root."
                } else {
                    "To measure other users' folders or system areas, start Filescape from a terminal with sudo for that one scan. Avoid running it as root day to day."
                },
            );
            ui.add_space(8.0);
        }
    });
}
