//! Settings > Licensing, the "needs a licence" prompt and the expiry banner.

use super::{Feature, Licensing, PURCHASE_URL};
use filescape_licence as core;
use core::export::date_time;
use core::model::LicenceKind;
use core::standing::{Reason, Standing};
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
    match f {
        Feature::FileBrowser => "The file browser",
        Feature::DiskMap => "The disk map",
        Feature::Filters => "Filtering by type or size",
        Feature::CustomFilter => "The Custom filter",
        Feature::SequenceFilter => "Finding numbered sequences",
        Feature::Remote => "Connecting to servers",
        Feature::RemoteTransfers => "Server transfers",
        Feature::SequencePlayer => "The sequence player",
        Feature::BatchRename => "Batch rename",
        Feature::FolderCompare => "Folder compare",
        Feature::Duplicates => "The duplicate finder",
        Feature::Archives => "Archives",
        Feature::SavedSearches => "Saved searches",
        Feature::Snapshots => "Snapshots",
    }
}

fn row(ui: &mut egui::Ui, k: &str, v: impl Into<String>) {
    ui.label(RichText::new(k).weak());
    ui.label(v.into());
    ui.end_row();
}

impl Licensing {
    /// One line for the top of the window, when there's something to say.
    pub fn banner(&mut self, ui: &mut egui::Ui) -> bool {
        if self.ui.banner_closed {
            return false;
        }
        let (text, urgent) = match &self.standing {
            Standing::Temporary { days_left, .. } if *days_left <= 7 => (format!("Your temporary Filescape licence ends in {}.", days(*days_left)), false),
            Standing::Evaluation { days_left, .. } if *days_left <= 7 => (format!("Filescape evaluation: {} left.", days(*days_left)), false),
            Standing::Restricted(Reason::TemporaryExpired(_)) => ("Your temporary Filescape licence has expired. Your files, settings and saved data are intact.".into(), true),
            Standing::Restricted(Reason::EvaluationOver) => ("The Filescape evaluation has ended. The file browser, the disk map and basic filters stay free.".into(), true),
            Standing::Restricted(Reason::ClockRollback) => ("This computer's clock is behind a time Filescape has already seen, so time-limited licences are paused until it's right again.".into(), true),
            Standing::Restricted(Reason::UpdatesLapsed(_)) => ("Your Filescape licence's updates ended before this version was released.".into(), true),
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
                if urgent && ui.small_button("Purchase").clicked() {
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
                _ => "Enter a licence, or buy one. The file browser, the disk map and basic filters stay free.",
            });
            ui.label(RichText::new("Your files, settings and saved data are untouched, and anything already running carries on.").weak());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Enter Licence Key…").clicked() {
                    self.ui.open_settings = true;
                    self.ui.prompt = None;
                }
                if ui.button("Purchase Perpetual Licence").clicked() {
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

    /// Settings > Licensing.
    pub fn panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().max_height(390.0).auto_shrink([false, true]).show(ui, |ui| {
            self.status(ui);
            ui.add_space(8.0);
            self.details(ui);
            ui.add_space(8.0);
            self.entry(ui);
            #[cfg(feature = "licence-dev")]
            self.test_tools(ui);
        });
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        let now = self.now();
        let cert = self.shown_cert();
        match self.standing.clone() {
            Standing::Temporary { cert, days_left } => {
                ui.heading("Filescape Professional — Temporary Licence");
                ui.label("Your temporary licence is active, with full access to all Filescape features.");
                egui::Grid::new("lic-temp").num_columns(2).show(ui, |ui| {
                    row(ui, "Expires", cert.expires_at.map(date_time).unwrap_or_default());
                    row(ui, "Days remaining", days_left.to_string());
                    row(ui, "Activations used", format!("{} of {}", cert.activations_used, cert.max_activations));
                    row(ui, "Licence type", "Temporary (Non-renewable)");
                });
            }
            Standing::Perpetual(cert) => {
                ui.heading("Filescape Professional");
                ui.label("Your licence is active and doesn't expire.");
                egui::Grid::new("lic-perp").num_columns(2).show(ui, |ui| {
                    row(ui, "Updates until", cert.updates_until.map(date_time).unwrap_or_else(|| "always".into()));
                    row(ui, "Computers active", format!("{} of {}", cert.activations_used, cert.max_activations));
                    row(ui, "Licence type", "Perpetual");
                });
            }
            Standing::Evaluation { ends_at, days_left } => {
                ui.heading("Filescape evaluation");
                ui.label(format!("Every feature is unlocked until {} ({} left).", date_time(ends_at), days(days_left)));
                ui.label("Afterwards the file browser, the disk map and basic filters stay free.");
            }
            Standing::Restricted(Reason::TemporaryExpired(_)) => {
                ui.heading("Your temporary Filescape licence has expired");
                ui.label("Your temporary access period has ended. Your files, settings and saved data remain intact.");
                ui.label("To continue using Filescape's full functionality, enter another valid licence or purchase a perpetual licence.");
            }
            Standing::Restricted(r) => {
                ui.heading("Filescape is not licensed");
                ui.label(match r {
                    Reason::ClockRollback => "This computer's clock is behind a time Filescape has already seen, so time-limited licences are paused. Set the clock right, or go online so Filescape can check the time.",
                    Reason::UpdatesLapsed(_) => "Your licence covers versions released while its updates last. This version is newer; use an earlier one, or buy a new licence.",
                    Reason::Revoked => "Your licence has been revoked. If you think this is a mistake, write to us at filescape.app/contact (Help > Contact Filescape).",
                    _ => "The evaluation has ended. The file browser, the disk map and basic filters stay free.",
                });
            }
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if let Some(c) = &cert {
                if let Some(serial) = self.serial_of(&c.licence_id)
                    && ui.button("Copy Serial").clicked()
                {
                    ui.ctx().copy_text(serial);
                    self.ui.say("Serial number copied.".into(), false);
                }
                ui.menu_button("Export Licence ⏷", |ui| {
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
                if self.standing.is_licensed() && !matches!(self.standing, Standing::Evaluation { .. }) && ui.button("Deactivate This Computer…").on_hover_text(hint).clicked() {
                    self.ui.confirm_deactivate = true;
                }
            }
            // No Renew button for temporary licences: more time means a new serial.
            if !matches!(self.standing, Standing::Perpetual(_)) && ui.button("Purchase Perpetual Licence").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(PURCHASE_URL));
            }
        });
        if self.ui.confirm_deactivate {
            let perpetual = matches!(self.standing, Standing::Perpetual(_));
            ui.group(|ui| {
                ui.label(if perpetual {
                    "Deactivate Filescape on this computer? Professional features stop here, and its place is freed for another computer. (Release more than one computer in a week and the later ones keep their place for 7 days.)"
                } else {
                    "Deactivate Filescape on this computer? Professional features stop here, and the activation it used isn't given back."
                });
                ui.horizontal(|ui| {
                    if ui.button("Deactivate").clicked() {
                        self.ui.confirm_deactivate = false;
                        self.deactivate();
                    }
                    if ui.button("Cancel").clicked() {
                        self.ui.confirm_deactivate = false;
                    }
                });
            });
        }
    }

    fn details(&mut self, ui: &mut egui::Ui) {
        let Some(c) = self.shown_cert() else { return };
        let issued = self.issuance_of(&c.licence_id).map(|(_, i)| i);
        egui::CollapsingHeader::new("Licence Details").default_open(true).show(ui, |ui| {
            egui::Grid::new("lic-details").num_columns(2).show(ui, |ui| {
                row(ui, "Registered owner", c.owner.clone().unwrap_or_else(|| "(not assigned)".into()));
                row(ui, "Licence type", if c.kind == LicenceKind::Temporary { "Temporary (Non-renewable)" } else { "Perpetual" });
                row(ui, "Licence ID", c.licence_id.clone());
                row(ui, "Serial number", format!("ends in {}", c.serial_hint.trim_start_matches('…')));
                row(ui, "Date issued", date_time(c.issued_at));
                row(ui, "Date activated", date_time(c.first_activated_at));
                row(ui, "Expiration date", c.expires_at.map(date_time).unwrap_or_else(|| "never".into()));
                if c.kind == LicenceKind::Perpetual {
                    // Whichever the server signed most recently: computers come and go.
                    let n = issued.as_ref().filter(|i| i.as_of >= c.signed_at).map_or(c.activations_used, |i| i.activations_used);
                    row(ui, "Computers active", format!("{n} of {}", c.max_activations));
                } else {
                    row(ui, "Activations used", format!("{} of {}", issued.as_ref().map_or(c.activations_used, |i| i.activations_used.max(c.activations_used)), c.max_activations));
                }
                row(ui, "Status", self.status_label());
            });
        });
    }

    fn status_label(&self) -> String {
        match &self.standing {
            Standing::Temporary { .. } | Standing::Perpetual(_) => "Active".into(),
            Standing::Restricted(Reason::TemporaryExpired(_)) => "Expired".into(),
            Standing::Restricted(Reason::Revoked) => "Revoked".into(),
            Standing::Restricted(Reason::ClockRollback) => "Paused (clock behind)".into(),
            Standing::Restricted(Reason::UpdatesLapsed(_)) => "Updates ended before this version".into(),
            _ => "Not licensed".into(),
        }
    }

    fn entry(&mut self, ui: &mut egui::Ui) {
        let label = if self.shown_cert().is_some() { "Enter New Licence" } else { "Enter Licence Key" };
        ui.label(RichText::new(label).strong());
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.ui.entry).hint_text("FST-XXXXX-XXXXX-XXXXX-XXXXX").desired_width(280.0));
            let go = ui.add_enabled(!self.busy() && !self.ui.entry.trim().is_empty(), egui::Button::new("Activate")).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go && !self.busy() {
                let s = self.ui.entry.clone();
                self.activate(&s);
            }
        });
        if let Some((m, err)) = &self.ui.message {
            let c = if *err { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
            ui.label(RichText::new(m).color(c));
        }
        egui::CollapsingHeader::new("Offline activation").show(ui, |ui| {
            ui.label("For a computer without internet access: save a request file, upload it at account.filescape.app/activate from any computer that's online, and bring the response file back here. It counts towards the same activation limit.");
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.ui.entry.trim().is_empty(), egui::Button::new("Save Request…")).clicked() {
                    let s = self.ui.entry.clone();
                    match self.offline_request(&s) {
                        Ok(text) => {
                            if let Some(f) = rfd::FileDialog::new().set_file_name("filescape-activation-request.json").save_file() {
                                match std::fs::write(&f, text) {
                                    Ok(()) => self.ui.say(format!("Saved {}.", f.display()), false),
                                    Err(e) => self.ui.say(format!("Couldn't save it: {e}"), true),
                                }
                            }
                        }
                        Err(e) => self.ui.say(e, true),
                    }
                }
                if ui.button("Import Response…").clicked()
                    && let Some(f) = rfd::FileDialog::new().add_filter("Activation response", &["json"]).pick_file()
                {
                    match std::fs::read_to_string(&f) {
                        Ok(t) => {
                            if self.ui.serial_for_keychain.is_none()
                                && let Ok((_, s)) = core::serial::normalize(&self.ui.entry)
                            {
                                self.ui.serial_for_keychain = Some(s);
                            }
                            self.offline_response(&t)
                        }
                        Err(e) => self.ui.say(format!("Couldn't read it: {e}"), true),
                    }
                }
            });
        });
        egui::CollapsingHeader::new("Verify a licence certificate").show(ui, |ui| {
            ui.label("Checks a .filescape-licence file's signature, for proof of purchase.");
            if ui.button("Verify Certificate…").clicked()
                && let Some(f) = rfd::FileDialog::new().add_filter("Filescape licence certificate", &[core::export::CERTIFICATE_EXTENSION]).pick_file()
            {
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                match core::export::verify_certificate_file(&text, self.verifier()) {
                    Ok(c) => self.ui.say(format!("Genuine: licence {} ({}), issued {}.", c.licence_id, c.owner.unwrap_or_else(|| "no owner".into()), date_time(c.issued_at)), false),
                    Err(e) => self.ui.say(format!("Not a valid certificate: {e}."), true),
                }
            }
        });
    }

    fn export_certificate(&mut self, licence_id: &str) {
        let Some((env, _)) = self.issuance_of(licence_id) else {
            return self.ui.say("No certificate for this licence yet: it arrives with the next check.".into(), true);
        };
        match core::export::certificate_file(&env, self.verifier()) {
            Ok(text) => {
                if let Some(f) = rfd::FileDialog::new().set_file_name(format!("{licence_id}.{}", core::export::CERTIFICATE_EXTENSION)).save_file() {
                    match std::fs::write(&f, text) {
                        Ok(()) => self.ui.say(format!("Saved {}.", f.display()), false),
                        Err(e) => self.ui.say(format!("Couldn't save it: {e}"), true),
                    }
                }
            }
            Err(e) => self.ui.say(format!("Couldn't export it: {e}."), true),
        }
    }

    fn export_details(&mut self, licence_id: &str, _now: core::Ts) {
        let Some((_, c)) = self.issuance_of(licence_id) else {
            return self.ui.say("No details for this licence yet: they arrive with the next check.".into(), true);
        };
        let text = core::export::details_text(&c, &self.status_label());
        if let Some(f) = rfd::FileDialog::new().set_file_name(format!("{licence_id} details.txt")).save_file() {
            match std::fs::write(&f, text) {
                Ok(()) => self.ui.say(format!("Saved {}.", f.display()), false),
                Err(e) => self.ui.say(format!("Couldn't save it: {e}"), true),
            }
        }
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

fn days(n: i64) -> String {
    if n == 1 { "1 day".into() } else { format!("{n} days") }
}
