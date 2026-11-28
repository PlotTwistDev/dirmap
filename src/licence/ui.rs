//! Settings > Licensing, the "needs a licence" prompt and the expiry banner.

use super::{Feature, Licensing, PURCHASE_URL};
use filescape_licence as core;
use core::export::date_time;
use core::model::LicenceKind;
use core::standing::{Reason, Standing};
use crate::kit::{self, Icon, Kind, Tone};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct State {
    /// The serial being typed.
    pub entry: String,
    /// The last result, and whether it's an error.
    pub message: Option<(String, bool)>,
    /// A premium feature that was refused: shows the prompt.
    pub prompt: Option<Feature>,
    /// The prompt's "Enter Licence…" was pressed: the app opens Settings > Licensing.
    pub open_settings: bool,
    /// The banner was closed for this session.
    pub banner_closed: bool,
    pub(super) serial_for_keychain: Option<String>,
    /// Deactivate This Computer… was pressed: asking to confirm.
    confirm_deactivate: bool,
    /// Check Now was pressed: say what the check found.
    pub(super) checking: bool,
    /// A link to open in the browser once the server answered (the renewal checkout).
    pub(super) open_url: Option<String>,
    /// File pickers waiting for an answer (polled by the app each frame).
    pub asking: Vec<crate::asking::Asking<Licensing>>,
    #[cfg(feature = "licence-dev")]
    test: TestForm,
}

#[cfg(feature = "licence-dev")]
struct TestForm {
    days: u32,
    max: u32,
    fixed: bool,
    fixed_days: u32,
}

#[cfg(feature = "licence-dev")]
impl Default for TestForm {
    fn default() -> Self {
        TestForm { days: core::DEFAULT_TEMPORARY_DAYS, max: core::DEFAULT_MAX_ACTIVATIONS, fixed: false, fixed_days: 7 }
    }
}

impl State {
    pub fn say(&mut self, text: String, error: bool) {
        self.message = Some((text, error));
    }
}

pub fn feature_name(f: Feature) -> &'static str {
    f.name()
}

impl Licensing {
    /// One line for the top of the window, when there's something to say.
    pub fn banner(&mut self, ui: &mut egui::Ui) -> bool {
        if self.ui.banner_closed {
            return false;
        }
        let now = self.now();
        let ending = match &self.standing {
            Standing::Perpetual(c) => match c.updates_until.map(|u| core::maintenance::status(u, now)) {
                Some(core::maintenance::MaintenanceStatus::Active { days_left, .. }) if days_left <= 30 => Some(days_left),
                _ => None,
            },
            _ => None,
        };
        let (text, urgent) = match &self.standing {
            _ if ending.is_some() => (format!("Filescape software maintenance ends in {}. Extend it to keep getting new versions; this version keeps working either way.", days(ending.unwrap_or(0))), false),
            Standing::Temporary { days_left, .. } if *days_left <= 7 => (format!("Your temporary Filescape licence ends in {}.", days(*days_left)), false),
            Standing::Evaluation { days_left, .. } if *days_left <= 7 => (format!("Filescape evaluation: {} left.", days(*days_left)), false),
            Standing::Restricted(Reason::TemporaryExpired(_)) => ("Your temporary Filescape licence has expired. Your files, settings and saved data are intact.".into(), true),
            Standing::Restricted(Reason::EvaluationOver) => ("The Filescape evaluation has ended. The file browser, the disk map and the AI Models workspace stay free.".into(), true),
            Standing::Restricted(Reason::ClockRollback) => ("This computer's clock is behind a time Filescape has already seen, so time-limited licences are paused until it's right again.".into(), true),
            Standing::Restricted(Reason::UpdatesLapsed(_)) => ("Your Filescape software maintenance ended before this version was released, so it runs with the free features.".into(), true),
            Standing::Restricted(Reason::Revoked) => ("Your Filescape licence has been revoked.".into(), true),
            _ => return false,
        };
        let mut shown = false;
        egui::Panel::top("licence-banner").show(ui, |ui| {
            shown = true;
            ui.horizontal(|ui| {
                let c = if urgent { ui.visuals().warn_fg_color } else { ui.visuals().text_color() };
                ui.label(RichText::new(text).color(c));
                if ui.small_button("Licence…").clicked() {
                    self.ui.open_settings = true;
                }
                if self.maintained() {
                    if ui.add_enabled(!self.busy(), egui::Button::new("Extend Maintenance…").small()).on_hover_text("Opens the renewal checkout in your browser").clicked() {
                        self.renew();
                    }
                } else if urgent && ui.small_button("Purchase").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(PURCHASE_URL));
                }
                if ui.small_button("✖").on_hover_text("Hide until Filescape restarts").clicked() {
                    self.ui.banner_closed = true;
                }
            });
        });
        shown
    }

    /// Shown when a premium operation was refused.
    pub fn prompt(&mut self, ctx: &egui::Context) {
        let Some(f) = self.ui.prompt else { return };
        let mut open = true;
        egui::Window::new("Needs a Filescape licence").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
            ui.label(format!("{} is part of Filescape Professional.", feature_name(f)));
            ui.label(match &self.standing {
                Standing::Restricted(Reason::TemporaryExpired(_)) => "Your temporary licence has expired. Enter another licence, or buy a perpetual one.",
                Standing::Restricted(Reason::ClockRollback) => "Time-limited licences are paused because this computer's clock is behind. Set the clock right and try again.",
                Standing::Restricted(Reason::UpdatesLapsed(_)) => "Your licence's software maintenance ended before this version was released. Extend maintenance to unlock it, or use a version released before then.",
                _ => "Enter a licence, or buy one. The file browser, the disk map and the AI Models workspace stay free.",
            });
            ui.label(RichText::new("Your files, settings and saved data are untouched, and anything already running carries on.").weak());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Enter Licence Key…").clicked() {
                    self.ui.open_settings = true;
                    self.ui.prompt = None;
                }
                if matches!(self.standing, Standing::Restricted(Reason::UpdatesLapsed(_))) {
                    if ui.add_enabled(!self.busy(), egui::Button::new("Extend Maintenance…")).clicked() {
                        self.renew();
                        self.ui.prompt = None;
                    }
                } else if ui.button("Purchase Perpetual Licence").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(PURCHASE_URL));
                }
                if ui.button("Not now").clicked() {
                    self.ui.prompt = None;
                }
            });
        });
        if !open {
            self.ui.prompt = None;
        }
    }

    /// Settings > Licensing. `focus` says which section a settings search points at.
    pub fn panel(&mut self, ui: &mut egui::Ui, focus: &dyn Fn(&str) -> kit::Focus) {
        self.status(ui, focus("status"));
        self.details(ui, focus("details"));
        self.entry(ui, focus("activate"));
        #[cfg(feature = "licence-dev")]
        self.test_tools(ui);
    }

    /// The badge for the licence's state.
    fn status_badge(&self) -> (String, Tone) {
        let tone = match &self.standing {
            Standing::Temporary { .. } | Standing::Perpetual(_) => Tone::Success,
            Standing::Evaluation { .. } => Tone::Accent,
            Standing::Restricted(Reason::Revoked) => Tone::Danger,
            Standing::Restricted(_) => Tone::Warn,
        };
        let label = match &self.standing {
            Standing::Evaluation { .. } => "Evaluation".to_string(),
            Standing::Restricted(Reason::EvaluationOver) => "Free edition".to_string(),
            _ => self.status_label(),
        };
        (label, tone)
    }

    fn status(&mut self, ui: &mut egui::Ui, focus: kit::Focus) {
        let now = self.now();
        let cert = self.shown_cert();
        let (badge, tone) = self.status_badge();
        kit::section_with(ui, Icon::Key, "Licence", Some((&badge, tone)), focus, |ui| {
            let t = kit::tokens(ui);
            ui.add_space(12.0);
            let (edition, line): (&str, String) = match &self.standing {
                Standing::Temporary { .. } => ("Filescape Professional", "Temporary licence, with every Filescape feature unlocked until it ends.".into()),
                Standing::Perpetual(_) => ("Filescape Professional", "Your licence is active and never expires.".into()),
                Standing::Evaluation { ends_at, days_left } => ("Filescape Professional evaluation", format!("Every feature is unlocked until {} ({} left).", date_time(*ends_at), days(*days_left))),
                Standing::Restricted(Reason::TemporaryExpired(_)) => ("Filescape Free", "Your temporary licence has expired. Your files, settings and saved data are intact. Enter another licence, or buy a perpetual one, for the full feature set.".into()),
                Standing::Restricted(r) => (
                    "Filescape Free",
                    match r {
                        Reason::ClockRollback => "This computer's clock is behind a time Filescape has already seen, so time-limited licences are paused. Set the clock right, or go online so Filescape can check the time.",
                        Reason::UpdatesLapsed(_) => "Your licence is still valid, but its software maintenance ended before this version was released, so this version runs with the free features. Extend maintenance to unlock it and get 12 more months of new versions, or keep using a version released before maintenance ended.",
                        Reason::Revoked => "Your licence has been revoked. If you think this is a mistake, write to us at filescape.app/contact (Help > Contact Filescape).",
                        _ => "The evaluation has ended. The file browser, the disk map and the AI Models workspace stay free.",
                    }
                    .into(),
                ),
            };
            ui.label(RichText::new(edition).size(kit::HEADING + 3.0).strong().color(t.strong));
            ui.add_space(2.0);
            kit::note(ui, line);
            ui.add_space(10.0);
            match self.standing.clone() {
                Standing::Evaluation { ends_at, days_left } => {
                    let total = i64::from(core::EVALUATION_DAYS);
                    kit::progress(ui, days_left as f32 / total as f32, Tone::Accent);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} of {} left", days(days_left), days(total))).size(kit::DESC).color(t.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("Ends {}", date_time(ends_at).split(',').next().unwrap_or_default())).size(kit::DESC).color(t.weak));
                        });
                    });
                    ui.add_space(8.0);
                    kit::callout(ui, Tone::Neutral, "Free after the evaluation: the file browser, the disk map and the AI Models workspace. Your files, settings and saved data are never affected.");
                    ui.add_space(8.0);
                }
                Standing::Temporary { cert, days_left } => {
                    egui::Grid::new("lic-temp").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                        kit::kv(ui, "Licence type", "Temporary (non-renewable)");
                        kit::kv(ui, "Expires", cert.expires_at.map(date_time).unwrap_or_default());
                        kit::kv(ui, "Days remaining", days_left.to_string());
                        kit::kv(ui, "Activations used", format!("{} of {}", cert.activations_used, cert.max_activations));
                    });
                    ui.add_space(8.0);
                }
                Standing::Perpetual(cert) | Standing::Restricted(Reason::UpdatesLapsed(cert)) => {
                    egui::Grid::new("lic-perp").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                        kit::kv(ui, "Licence", "Perpetual: never expires");
                        kit::kv(ui, "Software maintenance", maintenance_line(cert.updates_until, now));
                        kit::kv(ui, "Computers active", format!("{} of {}", cert.activations_used, cert.max_activations));
                    });
                    ui.add_space(6.0);
                    if let Some(until) = cert.updates_until {
                        if let core::maintenance::MaintenanceStatus::Active { days_left, .. } = core::maintenance::status(until, now) {
                            kit::progress(ui, (days_left as f32 / 365.0).min(1.0), if days_left <= 30 { Tone::Warn } else { Tone::Success });
                            ui.add_space(6.0);
                        }
                        kit::note(ui, "Your licence never expires. Software maintenance decides which new versions it unlocks: every version released before its end date keeps Pro for good, even after maintenance ends. Extending adds 12 months to the end date, or starts 12 new months if it has already ended.");
                        ui.add_space(8.0);
                    }
                }
                _ => {}
            }
            kit::divider(ui);
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                // No Renew button for temporary licences: more time means a new serial.
                let maintained = self.maintained();
                if !maintained
                    && !matches!(self.standing, Standing::Perpetual(_))
                    && kit::button(ui, Kind::Primary, Some(Icon::External), "Buy Filescape Professional").on_hover_text("Opens filescape.app in your browser").clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(PURCHASE_URL));
                }
                if maintained {
                    let busy = self.busy();
                    let r = ui.add_enabled_ui(!busy, |ui| kit::button(ui, Kind::Primary, Some(Icon::External), "Extend Maintenance…")).inner;
                    if r.on_hover_text("Opens the renewal checkout in your browser: 12 more months of new versions. Your serial and computers stay as they are.").clicked() {
                        self.renew();
                    }
                }
                if cert.is_some() {
                    let busy = self.busy();
                    let r = ui.add_enabled_ui(!busy, |ui| kit::button(ui, Kind::Secondary, Some(Icon::Refresh), if self.ui.checking { "Checking…" } else { "Check Now" })).inner;
                    if r.on_hover_text("Ask the licence server for this licence's latest details, such as a new maintenance date").clicked() {
                        self.check_now();
                    }
                }
                if let Some(c) = &cert {
                    if let Some(serial) = self.serial_of(&c.licence_id)
                        && kit::button(ui, Kind::Secondary, Some(Icon::Copy), "Copy Serial").clicked()
                    {
                        ui.ctx().copy_text(serial);
                        self.ui.say("Serial number copied.".into(), false);
                    }
                    let r = kit::button(ui, Kind::Secondary, Some(Icon::File), "Export Licence ⏷");
                    egui::Popup::menu(&r).show(|ui| {
                        if ui.button("Licence Certificate (signed)…").clicked() {
                            self.export_certificate(&c.licence_id);
                            ui.close();
                        }
                        if ui.button("Licence Details (text)…").clicked() {
                            self.export_details(&c.licence_id, now);
                            ui.close();
                        }
                    });
                    let hint = if c.kind == LicenceKind::Perpetual { "Ends the activation here and frees this computer's place for another one." } else { "Ends the activation here. The activation it used isn't given back." };
                    if self.standing.is_licensed()
                        && !matches!(self.standing, Standing::Evaluation { .. })
                        && kit::button(ui, Kind::Danger, None, "Deactivate This Computer…").on_hover_text(hint).clicked()
                    {
                        self.ui.confirm_deactivate = true;
                    }
                }
            });
            ui.add_space(10.0);
            if self.ui.confirm_deactivate {
                let perpetual = matches!(self.standing, Standing::Perpetual(_));
                kit::callout(
                    ui,
                    Tone::Warn,
                    if perpetual {
                        "Deactivate Filescape on this computer? Professional features stop here, and its place is freed for another computer. (Release more than one computer in a week and the later ones keep their place for 7 days.)"
                    } else {
                        "Deactivate Filescape on this computer? Professional features stop here, and the activation it used isn't given back."
                    },
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if kit::button(ui, Kind::Danger, None, "Deactivate").clicked() {
                        self.ui.confirm_deactivate = false;
                        self.deactivate();
                    }
                    if kit::button(ui, Kind::Secondary, None, "Cancel").clicked() {
                        self.ui.confirm_deactivate = false;
                    }
                });
                ui.add_space(10.0);
            }
        });
    }

    fn details(&mut self, ui: &mut egui::Ui, focus: kit::Focus) {
        let Some(c) = self.shown_cert() else { return };
        let issued = self.issuance_of(&c.licence_id).map(|(_, i)| i);
        kit::section(ui, Icon::File, "Licence details", focus, |ui| {
            ui.add_space(10.0);
            egui::Grid::new("lic-details").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                kit::kv(ui, "Registered owner", c.owner.clone().unwrap_or_else(|| "(not assigned)".into()));
                kit::kv(ui, "Licence type", if c.kind == LicenceKind::Temporary { "Temporary (Non-renewable)" } else { "Perpetual" });
                kit::kv(ui, "Licence ID", RichText::new(c.licence_id.clone()).monospace());
                kit::kv(ui, "Serial number", format!("ends in {}", c.serial_hint.trim_start_matches('…')));
                kit::kv(ui, "Date issued", date_time(c.issued_at));
                kit::kv(ui, "Date activated", date_time(c.first_activated_at));
                kit::kv(ui, "Expiration date", c.expires_at.map(date_time).unwrap_or_else(|| "never".into()));
                if c.kind == LicenceKind::Perpetual {
                    // Whichever the server signed most recently: computers come and go.
                    let n = issued.as_ref().filter(|i| i.as_of >= c.signed_at).map_or(c.activations_used, |i| i.activations_used);
                    kit::kv(ui, "Computers active", format!("{n} of {}", c.max_activations));
                } else {
                    kit::kv(ui, "Activations used", format!("{} of {}", issued.as_ref().map_or(c.activations_used, |i| i.activations_used.max(c.activations_used)), c.max_activations));
                }
                kit::kv(ui, "Status", self.status_label());
            });
            ui.add_space(10.0);
        });
    }

    /// A perpetual licence whose maintenance can be extended (running, or ended before this
    /// version). Temporary licences can't be renewed.
    pub fn maintained(&self) -> bool {
        match &self.standing {
            Standing::Perpetual(c) => c.updates_until.is_some(),
            Standing::Restricted(Reason::UpdatesLapsed(_)) => true,
            _ => false,
        }
    }

    fn status_label(&self) -> String {
        match &self.standing {
            Standing::Temporary { .. } | Standing::Perpetual(_) => "Active".into(),
            Standing::Restricted(Reason::TemporaryExpired(_)) => "Expired".into(),
            Standing::Restricted(Reason::Revoked) => "Revoked".into(),
            Standing::Restricted(Reason::ClockRollback) => "Paused (clock behind)".into(),
            Standing::Restricted(Reason::UpdatesLapsed(_)) => "Maintenance ended before this version".into(),
            _ => "Not licensed".into(),
        }
    }

    fn entry(&mut self, ui: &mut egui::Ui, focus: kit::Focus) {
        let title = if self.shown_cert().is_some() { "Enter a new licence" } else { "Activate a licence" };
        kit::section(ui, Icon::Key, title, focus, |ui| {
            let t = kit::tokens(ui);
            kit::stacked(ui, "Licence key", Some("The serial from your purchase email, starting FSP- (perpetual) or FST- (temporary)."), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let r = kit::field(ui, &mut self.ui.entry, "FSP-XXXXX-XXXXX-XXXXX-XXXXX", (ui.available_width() - 200.0).clamp(200.0, 340.0));
                    if kit::button(ui, Kind::Secondary, Some(Icon::Clipboard), "Paste").on_hover_text("Paste the licence key from the clipboard").clicked() {
                        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                            Ok(text) => self.ui.entry = text.trim().to_string(),
                            Err(_) => self.ui.say("The clipboard has no text in it.".into(), true),
                        }
                    }
                    let go = ui.add_enabled_ui(!self.busy() && !self.ui.entry.trim().is_empty(), |ui| kit::button(ui, Kind::Primary, None, if self.busy() { "Activating…" } else { "Activate" })).inner.clicked()
                        || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if go && !self.busy() {
                        let s = self.ui.entry.clone();
                        self.activate(&s);
                    }
                });
            });
            if let Some((m, err)) = self.ui.message.clone() {
                kit::callout(ui, crate::settings::tone_of(err), m);
                ui.add_space(8.0);
            }
            kit::divider(ui);
            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new("Offline activation").size(kit::LABEL).color(t.strong)).id_salt("lic-offline").show(ui, |ui| {
                kit::note(ui, "For a computer without internet access: save a request file, upload it at account.filescape.app/activate from any computer that's online, and bring the response file back here. It counts towards the same activation limit.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if ui.add_enabled_ui(!self.ui.entry.trim().is_empty(), |ui| kit::button(ui, Kind::Secondary, Some(Icon::File), "Save Request…")).inner.on_disabled_hover_text("Enter the licence key above first").clicked() {
                        let s = self.ui.entry.clone();
                        match self.offline_request(&s) {
                            Ok(text) => {
                                let d = crate::asking::Dialog::new().set_file_name("filescape-activation-request.json");
                                crate::asking::ask(&mut self.ui.asking, d, crate::asking::Pick::Save, move |me, f| me.save_file(&f, text));
                            }
                            Err(e) => self.ui.say(e, true),
                        }
                    }
                    if kit::button(ui, Kind::Secondary, Some(Icon::Folder), "Import Response…").clicked() {
                        let d = crate::asking::Dialog::new().add_filter("Activation response", &["json"]);
                        crate::asking::ask(&mut self.ui.asking, d, crate::asking::Pick::File, |me, f| match std::fs::read_to_string(&f) {
                            Ok(t) => {
                                if me.ui.serial_for_keychain.is_none()
                                    && let Ok((_, s)) = core::serial::normalize(&me.ui.entry)
                                {
                                    me.ui.serial_for_keychain = Some(s);
                                }
                                me.offline_response(&t)
                            }
                            Err(e) => me.ui.say(format!("Couldn't read it: {e}"), true),
                        });
                    }
                });
                ui.add_space(6.0);
            });
            egui::CollapsingHeader::new(RichText::new("Verify a licence certificate").size(kit::LABEL).color(t.strong)).id_salt("lic-verify").show(ui, |ui| {
                kit::note(ui, "Checks a .filescape-licence file's signature, for proof of purchase.");
                ui.add_space(6.0);
                if kit::button(ui, Kind::Secondary, Some(Icon::Shield), "Verify Certificate…").clicked() {
                    let d = crate::asking::Dialog::new().add_filter("Filescape licence certificate", &[core::export::CERTIFICATE_EXTENSION]);
                    crate::asking::ask(&mut self.ui.asking, d, crate::asking::Pick::File, |me, f| {
                        let text = std::fs::read_to_string(&f).unwrap_or_default();
                        match core::export::verify_certificate_file(&text, me.verifier()) {
                            Ok(c) => me.ui.say(format!("Genuine: licence {} ({}), issued {}.", c.licence_id, c.owner.unwrap_or_else(|| "no owner".into()), date_time(c.issued_at)), false),
                            Err(e) => me.ui.say(format!("Not a valid certificate: {e}."), true),
                        }
                    });
                }
                ui.add_space(6.0);
            });
            ui.add_space(4.0);
        });
    }

    /// Write `text` to a file the user picked, and say how it went.
    fn save_file(&mut self, f: &std::path::Path, text: String) {
        match std::fs::write(f, text) {
            Ok(()) => self.ui.say(format!("Saved {}.", f.display()), false),
            Err(e) => self.ui.say(format!("Couldn't save it: {e}"), true),
        }
    }

    fn export_certificate(&mut self, licence_id: &str) {
        let Some((env, _)) = self.issuance_of(licence_id) else {
            return self.ui.say("No certificate for this licence yet: it arrives with the next check.".into(), true);
        };
        match core::export::certificate_file(&env, self.verifier()) {
            Ok(text) => {
                let d = crate::asking::Dialog::new().set_file_name(format!("{licence_id}.{}", core::export::CERTIFICATE_EXTENSION));
                crate::asking::ask(&mut self.ui.asking, d, crate::asking::Pick::Save, move |me, f| me.save_file(&f, text));
            }
            Err(e) => self.ui.say(format!("Couldn't export it: {e}."), true),
        }
    }

    fn export_details(&mut self, licence_id: &str, _now: core::Ts) {
        let Some((_, c)) = self.issuance_of(licence_id) else {
            return self.ui.say("No details for this licence yet: they arrive with the next check.".into(), true);
        };
        let text = core::export::details_text(&c, &self.status_label());
        let d = crate::asking::Dialog::new().set_file_name(format!("{licence_id} details.txt"));
        crate::asking::ask(&mut self.ui.asking, d, crate::asking::Pick::Save, move |me, f| me.save_file(&f, text));
    }

    #[cfg(feature = "licence-dev")]
    fn test_tools(&mut self, ui: &mut egui::Ui) {
        let now = self.now();
        let super::Server::Test(t) = &mut self.server else { return };
        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new("Test mode (FILESCAPE_LICENCE_DEV=1)").strong().color(ui.visuals().warn_fg_color));
        ui.label(RichText::new("Serials come from a test server inside this app that follows the real rules. Only this computer trusts them.").weak());
        let f = &mut self.ui.test;
        ui.horizontal(|ui| {
            ui.checkbox(&mut f.fixed, "Fixed date");
            if f.fixed {
                ui.add(egui::DragValue::new(&mut f.fixed_days).range(1..=365).suffix(" days from now"));
            } else {
                ui.add(egui::DragValue::new(&mut f.days).range(1..=365).suffix(" days"));
            }
            ui.add(egui::DragValue::new(&mut f.max).range(1..=20).prefix("limit "));
            if ui.button("Issue Temporary Serial").clicked() {
                let validity = if f.fixed { core::model::Validity::FixedExpiry { expires_at: now + i64::from(f.fixed_days) * core::DAY } } else { core::model::Validity::FirstActivation { days: f.days } };
                t.issue_temporary(&core::rules::TemporaryOptions { validity, max_activations: f.max, owner: Some("Test customer".into()), ..Default::default() }, now);
            }
            if ui.button("Issue Perpetual Serial").clicked() {
                t.issue_perpetual(now);
            }
        });
        let mut copy = None;
        for (s, what) in t.issued.iter().rev().take(6) {
            ui.horizontal(|ui| {
                ui.monospace(s);
                ui.label(RichText::new(what).weak());
                if ui.small_button("Copy").clicked() {
                    copy = Some(s.clone());
                }
                if ui.small_button("Use").clicked() {
                    self.ui.entry = s.clone();
                }
            });
        }
        if let Some(s) = copy {
            ui.ctx().copy_text(s);
        }
        let mut days = t.offset_days;
        ui.horizontal(|ui| {
            ui.label("Licence clock:");
            ui.add(egui::DragValue::new(&mut days).range(0..=800).suffix(" days ahead"));
            ui.label(RichText::new(format!("now {}", date_time(now))).weak());
        });
        if days != t.offset_days {
            t.offset_days = days;
            t.save();
            self.refresh();
        }
    }
}

/// "Active until 28 Nov 2027 (365 days left)" or "Ended 28 Nov 2027".
pub fn maintenance_line(until: Option<core::Ts>, now: core::Ts) -> String {
    let Some(u) = until else { return "Every future version".into() };
    let day = date_time(u).split(',').next().unwrap_or_default().to_string();
    match core::maintenance::status(u, now) {
        core::maintenance::MaintenanceStatus::Active { days_left, .. } => format!("Active until {day} ({} left)", days(days_left)),
        core::maintenance::MaintenanceStatus::Ended { .. } => format!("Ended {day}"),
    }
}

fn days(n: i64) -> String {
    if n == 1 { "1 day".into() } else { format!("{n} days") }
}
