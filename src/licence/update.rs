//! The update check: at every launch, ask the licence server for the newest release and, when it's
//! newer than this build, show a notice saying whether this machine's licence covers it.
//!
//! The request is a plain GET with nothing about the person or the computer in it. The answer is
//! signed by the licence server ([`Purpose::Release`]); an answer that can't be fetched, read or
//! verified is ignored without a word. Nothing is ever locked or delayed by this check, and it's
//! off when Settings > General > "Check for a new version when Filescape starts" is cleared.

use super::{Licensing, PURCHASE_URL, RELEASE_DATE};
use eframe::egui::{self, RichText};
use filescape_licence::export::date_time;
use filescape_licence::release::{Coverage, ReleaseManifest, coverage};
use filescape_licence::sign::{Envelope, Purpose};
use filescape_licence::standing::Context;
use std::sync::mpsc;
use std::time::Duration;

/// This build's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
pub struct UpdateCheck {
    started: bool,
    rx: Option<mpsc::Receiver<Option<Envelope>>>,
    /// The verified newer release, once found.
    pub found: Option<ReleaseManifest>,
    /// The notice was closed for this session (it comes back at the next launch).
    pub closed: bool,
}

/// "windows", "macos" or "linux": the names of the downloads.
pub fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn fetch(url: &str) -> Option<Envelope> {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).tls_config(tls).build().new_agent();
    let mut resp = agent.get(url).header("Accept", "application/json").call().ok()?;
    // A manifest is a few hundred bytes; refuse anything silly.
    let text = resp.body_mut().with_config().limit(64 * 1024).read_to_string().ok()?;
    #[derive(serde::Deserialize)]
    struct Answer {
        release: Envelope,
    }
    serde_json::from_str::<Answer>(&text).ok().map(|a| a.release)
}

impl Licensing {
    /// Call once a frame. Starts the check on the first call when `enabled`, and collects the
    /// answer when it arrives.
    pub fn update_tick(&mut self, ctx: &egui::Context, enabled: bool) {
        if !self.update.started && enabled {
            self.update.started = true;
            let base = match &self.server {
                super::Server::Online(base) => base.clone(),
                #[cfg(feature = "licence-dev")]
                super::Server::Test(_) => std::env::var("FILESCAPE_LICENCE_SERVER").unwrap_or_else(|_| super::SERVER.into()),
            };
            let url = format!("{}/v1/release", base.trim_end_matches('/'));
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(fetch(&url));
                repaint.request_repaint();
            });
            self.update.rx = Some(rx);
        }
        if let Some(rx) = &self.update.rx {
            match rx.try_recv() {
                Ok(env) => {
                    self.update.rx = None;
                    self.update.found = env.and_then(|e| self.newer_release(&e));
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.update.rx = None,
            }
        }
    }

    /// The release in `env`, if it's signed by the licence server, recent, and newer than this build.
    pub fn newer_release(&self, env: &Envelope) -> Option<ReleaseManifest> {
        let m: ReleaseManifest = self.verifier.verify(env, Purpose::Release).ok()?;
        (m.is_fresh(RELEASE_DATE) && m.is_newer_than(VERSION)).then_some(m)
    }

    /// What the new release would mean for this machine's licence.
    pub fn release_coverage(&self, m: &ReleaseManifest) -> Coverage {
        let ctx = Context { now: self.now(), device_hash: &self.device, build_date: RELEASE_DATE, verifier: &self.verifier };
        coverage(&ctx, &self.stored.state, m.released_at)
    }

    /// The notice at the top of the window, when a newer version is out. Returns whether it showed.
    pub fn update_notice(&mut self, ui: &mut egui::Ui) -> bool {
        if self.update.closed {
            return false;
        }
        let Some(m) = self.update.found.clone() else { return false };
        let cov = self.release_coverage(&m);
        let mut shown = false;
        egui::Panel::top("update-notice").show(ui, |ui| {
            shown = true;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Filescape {} is available", m.version)).strong());
                ui.label(RichText::new(format!("(you have {VERSION}).")).weak());
                let (text, renew) = notice_text(&cov);
                let warn = ui.visuals().warn_fg_color;
                ui.label(if renew { RichText::new(text).color(warn) } else { RichText::new(text) });
                if let Some(s) = &m.summary {
                    ui.label(RichText::new(s).weak());
                }
                if ui.small_button("Download").on_hover_text(m.download_for(platform())).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(m.download_for(platform())));
                }
                if let Some(n) = m.notes()
                    && ui.small_button("What's new").on_hover_text(n).clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(n));
                }
                if renew && ui.small_button("Buy a licence").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(PURCHASE_URL));
                }
                if ui.small_button("✖").on_hover_text("Hide until Filescape restarts").clicked() {
                    self.update.closed = true;
                }
            });
        });
        shown
    }
}

/// The sentence about the licence, and whether it needs a new one.
pub fn notice_text(c: &Coverage) -> (String, bool) {
    match c {
        Coverage::Covered => ("Your licence covers it.".into(), false),
        Coverage::NotCovered { updates_until } => (
            format!(
                "Your licence's updates ended on {}, so in the new version only the free features would be unlocked. This version keeps working as it is.",
                date_time(*updates_until).split(',').next().unwrap_or_default()
            ),
            true,
        ),
        Coverage::NotAffected => ("It's a free update.".into(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filescape_licence::model::{ActivationCert, LicenceKind};
    use filescape_licence::sign::{Signer, Verifier};
    use filescape_licence::standing::{LocalState, Reason, Standing};
    use filescape_licence::{DAY, Ts};

    struct Fixture {
        signer: Signer,
        l: Licensing,
    }

    /// The app's licensing, trusting a fresh test key, with no files and no network.
    fn fixture(perpetual_updates_until: Option<Option<Ts>>) -> Fixture {
        let signer = Signer::from_pkcs8(&Signer::generate_pkcs8(&ring::rand::SystemRandom::new())).unwrap();
        let now = super::super::system_now();
        let mut state = LocalState { first_launch: Some(now - 60 * DAY), high_water: now, certs: vec![], revoked: vec![] };
        if let Some(updates_until) = perpetual_updates_until {
            let c = ActivationCert {
                licence_id: "lic_1".into(),
                kind: LicenceKind::Perpetual,
                owner: None,
                serial_hint: "FSP-…".into(),
                activation_id: "act_1".into(),
                device_hash: "dev".into(),
                issued_at: now - 30 * DAY,
                first_activated_at: now - 30 * DAY,
                expires_at: None,
                updates_until,
                activations_used: 1,
                max_activations: 3,
                signed_at: now - DAY,
            };
            state.certs.push(signer.sign(Purpose::Activation, &c));
        }
        let l = Licensing {
            stored: super::super::Stored { state, ..Default::default() },
            standing: Standing::Restricted(Reason::EvaluationOver),
            verifier: Verifier::new(&[signer.public_key()]),
            device: "dev".into(),
            dir: None,
            server: super::super::Server::Online("http://127.0.0.1:9".into()),
            checked: None,
            keychain_high_water: 0,
            rx: None,
            ui: Default::default(),
            update: Default::default(),
        };
        Fixture { signer, l }
    }

    fn manifest(version: &str, released_at: Ts) -> ReleaseManifest {
        ReleaseManifest { version: version.into(), released_at, signed_at: super::super::system_now(), summary: None, notes_url: None, downloads: Default::default() }
    }

    fn next_version() -> String {
        let mut p: Vec<u64> = VERSION.split('.').map(|x| x.parse().unwrap()).collect();
        p[1] += 1;
        format!("{}.{}.0", p[0], p[1])
    }

    #[test]
    fn a_newer_signed_release_is_found() {
        let f = fixture(None);
        let env = f.signer.sign(Purpose::Release, &manifest(&next_version(), RELEASE_DATE + DAY));
        assert!(f.l.newer_release(&env).is_some());
    }

    #[test]
    fn the_same_or_an_older_version_says_nothing() {
        let f = fixture(None);
        assert!(f.l.newer_release(&f.signer.sign(Purpose::Release, &manifest(VERSION, RELEASE_DATE + DAY))).is_none());
        assert!(f.l.newer_release(&f.signer.sign(Purpose::Release, &manifest("0.0.1", RELEASE_DATE + DAY))).is_none());
    }

    #[test]
    fn unsigned_wrongly_signed_or_stale_answers_are_ignored() {
        let f = fixture(None);
        // Signed by another key.
        let other = Signer::from_pkcs8(&Signer::generate_pkcs8(&ring::rand::SystemRandom::new())).unwrap();
        assert!(f.l.newer_release(&other.sign(Purpose::Release, &manifest(&next_version(), RELEASE_DATE + DAY))).is_none());
        // A certificate's signature can't be passed off as a release.
        let mut env = f.signer.sign(Purpose::Activation, &manifest(&next_version(), RELEASE_DATE + DAY));
        assert!(f.l.newer_release(&env).is_none());
        env.purpose = "release".into();
        assert!(f.l.newer_release(&env).is_none());
        // An old manifest replayed.
        let mut m = manifest(&next_version(), RELEASE_DATE + DAY);
        m.signed_at = RELEASE_DATE - 40 * DAY;
        assert!(f.l.newer_release(&f.signer.sign(Purpose::Release, &m)).is_none());
    }

    #[test]
    fn the_notice_says_whether_the_licence_covers_it() {
        let now = super::super::system_now();
        let m = manifest(&next_version(), now);
        // Free (evaluation over, no licence).
        let f = fixture(None);
        assert_eq!(f.l.release_coverage(&m), Coverage::NotAffected);
        assert_eq!(notice_text(&Coverage::NotAffected).1, false);
        // Perpetual with updates well past the release.
        let f = fixture(Some(Some(now + 300 * DAY)));
        assert_eq!(f.l.release_coverage(&m), Coverage::Covered);
        // Perpetual whose updates ended before the release (but after this build).
        let until = RELEASE_DATE.max(now - 2 * DAY) + 1;
        let f = fixture(Some(Some(until)));
        let late = manifest(&next_version(), until + DAY);
        assert_eq!(f.l.release_coverage(&late), Coverage::NotCovered { updates_until: until });
        let (text, renew) = notice_text(&f.l.release_coverage(&late));
        assert!(renew && text.contains("updates ended") && text.contains("keeps working"), "{text}");
    }
}
