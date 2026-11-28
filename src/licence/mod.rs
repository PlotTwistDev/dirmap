//! Filescape licensing in the app: which licence applies on this machine, activating and
//! deactivating serials (online, or with request and response files), the weekly check, and the
//! start-only feature checks. The rules themselves live in `licensing/core` (crate
//! `filescape-licence`), shared with the licensing server.
//!
//! What is kept where:
//! - `licence.json` in the settings folder: activation and issuance certificates (signed by the
//!   server and bound to this machine, so safe in a file), activation IDs waiting for an answer,
//!   the first launch and the latest time seen.
//! - The system keychain (service `filescape-licence`): each original serial, for Copy Serial,
//!   and a second copy of the first launch and latest time seen, so deleting the file restarts
//!   neither the evaluation nor the clock checks.
//!
//! Expiry never deletes anything: settings, favourites, transfer history and scans stay.

pub mod ui;
pub mod update;

use filescape_licence as core;
use core::model::{ActivationCert, ActivationRequest, ActivationResponse, IssuanceCert, OFFLINE_REQUEST_FORMAT, OfflineRequest, Rejection, StatusStatement};
use core::sign::{Envelope, Purpose, Verifier};
use core::standing::{Context, LocalState, Standing};
use core::{DAY, Ts};
pub use core::features::{Feature, Permit};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The licensing server. `FILESCAPE_LICENCE_SERVER` overrides it (for a test Worker).
pub const SERVER: &str = "https://api.filescape.app";
pub const PURCHASE_URL: &str = "https://filescape.app/buy";
/// The contact form (there's no support email address).
pub const CONTACT_URL: &str = "https://filescape.app/contact";
/// The terms, including "as is" and no liability for lost files.
pub const TERMS_URL: &str = "https://filescape.app/terms";
/// Release date of this build, for perpetual update entitlement. build.rs takes it from the commit
/// being built (or `SOURCE_DATE_EPOCH`), so it can't be left stale at release time.
pub const RELEASE_DATE: Ts = parse_ts(env!("FILESCAPE_RELEASE_DATE"));

const fn parse_ts(s: &str) -> Ts {
    let b = s.as_bytes();
    let mut i = 0;
    let mut n: Ts = 0;
    while i < b.len() {
        assert!(b[i].is_ascii_digit(), "FILESCAPE_RELEASE_DATE must be Unix seconds");
        n = n * 10 + (b[i] - b'0') as Ts;
        i += 1;
    }
    n
}
const KEYCHAIN: &str = "filescape-licence";
/// How often to re-check the standing while running (also on regaining focus).
const RECHECK: Duration = Duration::from_secs(60);
/// The weekly online check, when the machine is online. A failed check never locks anything.
const REVALIDATE_EVERY: Ts = 7 * DAY;

/// Public keys of the production licensing server (base64url Ed25519), newest last. Not secret: they
/// only check signatures. To rotate, add the new key here and ship that build before the Worker
/// switches to it; keep the old one so existing certificates stay valid.
pub const PRODUCTION_KEYS: &[&str] = &[
    // key ID 204a936d1dae0b1d, made 23 Nov 2026 (api.filescape.app/v1/health)
    "ZEzWQGLrJ_Myeeihfp1vDd3PW-GD4m6zaJMMI5jOKiw",
];

/// The production keys, plus one more from `FILESCAPE_LICENCE_PUBKEY` (base64url) at build time,
/// for a build that talks to a test Worker.
fn trusted_keys() -> Vec<Vec<u8>> {
    let mut keys: Vec<Vec<u8>> = PRODUCTION_KEYS.iter().filter_map(|k| data_encoding::BASE64URL_NOPAD.decode(k.as_bytes()).ok()).collect();
    if let Some(k) = option_env!("FILESCAPE_LICENCE_PUBKEY")
        && let Ok(k) = data_encoding::BASE64URL_NOPAD.decode(k.trim().as_bytes())
    {
        keys.push(k);
    }
    keys
}

/// What the app keeps in licence.json.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    state: LocalState,
    /// Issuance certificates, for Licence Details and the exports.
    #[serde(default)]
    issuance: Vec<Envelope>,
    /// Activation IDs made before asking the server, by serial hash, so a retry after a dropped
    /// connection reuses the same one (and so can't consume a second use).
    #[serde(default)]
    pending: Vec<(String, String)>,
    #[serde(default)]
    last_validated: Ts,
    /// Serials by licence ID, only where the system keychain can't hold them (for Copy Serial).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    serials: Vec<(String, String)>,
}

/// The licensing server, or the in-process test server.
enum Server {
    Online(String),
    #[cfg(feature = "licence-dev")]
    Test(Box<dev::TestServer>),
}

enum Answer {
    Activated(Result<ActivationResponse, String>),
    /// Per activation: a fresh certificate, or the server's signed statement that it ended or was
    /// revoked. Unsigned errors (offline, server faults, "not found") aren't passed on at all.
    Validated(Vec<(String, Result<ActivationResponse, Envelope>)>),
    Deactivated(String, Result<(), String>),
    /// The renewal checkout link for this computer's licence, or why there isn't one.
    Renew(Result<String, String>),
}

pub struct Licensing {
    stored: Stored,
    pub standing: Standing,
    verifier: Verifier,
    device: String,
    dir: Option<PathBuf>,
    server: Server,
    checked: Option<Instant>,
    keychain_high_water: Ts,
    rx: Option<mpsc::Receiver<Answer>>,
    pub ui: ui::State,
    /// The update check at launch ([`update`]).
    pub update: update::UpdateCheck,
}

fn system_now() -> Ts {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as Ts).unwrap_or(0)
}

fn keychain_get(account: &str) -> Option<String> {
    if cfg!(test) {
        return None;
    }
    let account = account.to_string();
    std::thread::scope(|s| s.spawn(move || keyring::Entry::new(KEYCHAIN, &account).ok()?.get_password().ok()).join().ok().flatten())
}

/// Whether it was saved.
fn keychain_set(account: &str, value: &str) -> bool {
    if cfg!(test) {
        return false;
    }
    let (account, value) = (account.to_string(), value.to_string());
    std::thread::scope(|s| s.spawn(move || keyring::Entry::new(KEYCHAIN, &account).is_ok_and(|e| e.set_password(&value).is_ok())).join().unwrap_or(false))
}

/// The settings folder (beside Templates).
fn settings_dir() -> Option<PathBuf> {
    crate::templates::own_folder().and_then(|t| t.parent().map(PathBuf::from))
}

impl Licensing {
    pub fn load() -> Licensing {
        let dir = settings_dir();
        let mut stored: Stored = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join("licence.json")).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // The keychain's copy of the clock: earliest first launch, latest time seen.
        let (kc_first, kc_high) = keychain_get("clock")
            .and_then(|s| {
                let (a, b) = s.split_once(',')?;
                Some((a.parse::<Ts>().ok().filter(|&t| t > 0), b.parse::<Ts>().ok()?))
            })
            .unwrap_or((None, 0));
        stored.state.merge_keychain(kc_first, kc_high);
        let mut keys = trusted_keys();
        let server = Self::pick_server(dir.as_deref(), &mut keys);
        let mut me = Licensing {
            stored,
            standing: Standing::Restricted(core::standing::Reason::EvaluationOver),
            verifier: Verifier::new(&keys),
            device: device_id::hash(),
            dir,
            server,
            checked: None,
            keychain_high_water: kc_high,
            rx: None,
            ui: ui::State::default(),
            update: update::UpdateCheck::default(),
        };
        me.refresh();
        me
    }

    #[allow(unused_variables)]
    fn pick_server(dir: Option<&std::path::Path>, keys: &mut Vec<Vec<u8>>) -> Server {
        #[cfg(feature = "licence-dev")]
        if dev::enabled()
            && let Some(t) = dev::TestServer::open(dir)
        {
            keys.push(t.public_key());
            return Server::Test(Box::new(t));
        }
        Server::Online(std::env::var("FILESCAPE_LICENCE_SERVER").unwrap_or_else(|_| SERVER.into()))
    }

    /// The clock the licence goes by (the test server can move it forward).
    pub fn now(&self) -> Ts {
        match &self.server {
            #[cfg(feature = "licence-dev")]
            Server::Test(t) => system_now() + t.offset_days * DAY,
            _ => system_now(),
        }
    }

    /// Work out the standing again, move the clock record forward and save it.
    pub fn refresh(&mut self) {
        let now = self.now();
        self.stored.state.observe(now);
        let ctx = Context { now, device_hash: &self.device, build_date: RELEASE_DATE, verifier: &self.verifier };
        self.standing = core::standing::evaluate(&ctx, &self.stored.state);
        self.checked = Some(Instant::now());
        self.save();
    }

    fn save(&mut self) {
        if let Some(d) = &self.dir {
            let _ = std::fs::create_dir_all(d);
            if let Ok(b) = serde_json::to_vec_pretty(&self.stored) {
                let tmp = d.join("licence.json.tmp");
                if std::fs::write(&tmp, b).is_ok() {
                    let _ = std::fs::rename(&tmp, d.join("licence.json"));
                }
            }
        }
        // The keychain copy only every few minutes: it can be slow.
        let st = &self.stored.state;
        if st.high_water > self.keychain_high_water + 600 || (self.keychain_high_water == 0 && st.first_launch.is_some()) {
            keychain_set("clock", &format!("{},{}", st.first_launch.unwrap_or(0), st.high_water));
            self.keychain_high_water = st.high_water;
        }
    }

    /// Call once a frame: re-checks every minute and on regaining focus, collects answers from
    /// the server, and starts the weekly check.
    pub fn tick(&mut self, ctx: &eframe::egui::Context) {
        let focus_gained = ctx.input(|i| i.events.iter().any(|e| matches!(e, eframe::egui::Event::WindowFocused(true))));
        if focus_gained || self.checked.is_none_or(|c| c.elapsed() >= RECHECK) {
            self.refresh();
        }
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(a) => {
                    self.rx = None;
                    self.answer(a);
                    if let Some(url) = self.ui.open_url.take() {
                        ctx.open_url(eframe::egui::OpenUrl::new_tab(url));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(200)),
                Err(mpsc::TryRecvError::Disconnected) => self.rx = None,
            }
        } else if !self.stored.state.certs.is_empty() && self.now() - self.stored.last_validated >= REVALIDATE_EVERY {
            self.revalidate();
        }
        ctx.request_repaint_after(RECHECK);
    }

    pub fn allows(&self, f: Feature) -> bool {
        core::features::allows(&self.standing, f)
    }

    /// Where a premium operation starts. `None` shows the licence prompt instead. Anything already
    /// running is never checked again.
    pub fn begin(&mut self, f: Feature) -> Option<Permit> {
        let p = core::features::begin(&self.standing, f);
        if p.is_none() {
            self.ui.prompt = Some(f);
        }
        p
    }

    fn cert(&self, env: &Envelope) -> Option<ActivationCert> {
        self.verifier.verify(env, Purpose::Activation).ok()
    }

    /// The licence the screens talk about: the one in use, or the expired temporary one.
    pub fn shown_cert(&self) -> Option<ActivationCert> {
        use core::standing::Reason;
        match &self.standing {
            Standing::Perpetual(c) | Standing::Temporary { cert: c, .. } => Some(c.clone()),
            Standing::Restricted(Reason::TemporaryExpired(c) | Reason::UpdatesLapsed(c)) => Some(c.clone()),
            _ => None,
        }
    }

    pub fn issuance_of(&self, licence_id: &str) -> Option<(Envelope, IssuanceCert)> {
        self.stored
            .issuance
            .iter()
            .rev()
            .filter_map(|e| self.verifier.verify::<IssuanceCert>(e, Purpose::Issuance).ok().map(|c| (e.clone(), c)))
            .find(|(_, c)| c.licence_id == licence_id)
    }

    pub fn serial_of(&self, licence_id: &str) -> Option<String> {
        keychain_get(&format!("serial:{licence_id}")).or_else(|| self.stored.serials.iter().find(|(id, _)| id == licence_id).map(|(_, s)| s.clone()))
    }

    pub fn verifier(&self) -> &Verifier {
        &self.verifier
    }

    /// The request for a serial, reusing the activation ID saved for it if there is one.
    fn request_for(&mut self, serial_input: &str) -> Result<ActivationRequest, String> {
        let (_, serial) = core::serial::normalize(serial_input).map_err(|e| e.to_string())?;
        let h = core::serial::hash(&serial);
        self.ui.serial_for_keychain = Some(serial.clone());
        let id = match self.stored.pending.iter().find(|(s, _)| *s == h) {
            Some((_, id)) => id.clone(),
            None => {
                let id = new_activation_id();
                self.stored.pending.push((h, id.clone()));
                // Saved before the first attempt, so a retry can't consume a second use.
                self.save();
                id
            }
        };
        Ok(ActivationRequest { serial, activation_id: id, device_hash: self.device.clone(), app_version: env!("CARGO_PKG_VERSION").into(), os: Some(os_name().into()) })
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Activate a serial. Problems with what was typed are reported at once; nothing reaches the
    /// server (or consumes a use) for a mistyped serial.
    pub fn activate(&mut self, serial_input: &str) {
        let req = match self.request_for(serial_input) {
            Ok(r) => r,
            Err(e) => return self.ui.say(e, true),
        };
        #[cfg(feature = "licence-dev")]
        let now = self.now();
        match &mut self.server {
            #[cfg(feature = "licence-dev")]
            Server::Test(t) => {
                let a = t.activate(&req, now);
                self.answer(Answer::Activated(a));
            }
            Server::Online(base) => {
                let url = format!("{base}/v1/activate");
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(Answer::Activated(post::<_, ActivationResponse>(&url, &req)));
                });
                self.rx = Some(rx);
                self.ui.say("Activating…".into(), false);
            }
        }
    }

    /// Offline activation, step 1: the request file to carry to a computer with internet access.
    pub fn offline_request(&mut self, serial_input: &str) -> Result<String, String> {
        let req = self.request_for(serial_input)?;
        Ok(serde_json::to_string_pretty(&OfflineRequest { format: OFFLINE_REQUEST_FORMAT.into(), request: req }).unwrap())
    }

    /// Offline activation, step 2: the response file from account.filescape.app/activate.
    pub fn offline_response(&mut self, text: &str) {
        let a = serde_json::from_str::<ActivationResponse>(text).map_err(|_| "That isn't a Filescape activation response file.".to_string());
        self.answer(Answer::Activated(a));
    }

    fn answer(&mut self, a: Answer) {
        match a {
            Answer::Activated(Ok(r)) => {
                let cert = match self.stored.state.store(&self.verifier, r.activation.clone()) {
                    Ok(c) => c,
                    Err(e) => return self.ui.say(format!("The licence server's answer couldn't be checked: {e}."), true),
                };
                if cert.device_hash != self.device {
                    self.stored.state.forget(&self.verifier, &cert.activation_id);
                    return self.ui.say("That activation is for a different computer.".into(), true);
                }
                self.stored.state.trusted_time(cert.signed_at);
                self.stored.issuance.retain(|e| self.verifier.verify::<IssuanceCert>(e, Purpose::Issuance).map_or(true, |c| c.licence_id != cert.licence_id));
                self.stored.issuance.push(r.issuance);
                self.stored.state.revoked.retain(|l| *l != cert.licence_id);
                if let Some(serial) = self.ui.serial_for_keychain.take() {
                    self.stored.serials.retain(|(id, _)| *id != cert.licence_id);
                    if !keychain_set(&format!("serial:{}", cert.licence_id), &serial) {
                        self.stored.serials.push((cert.licence_id.clone(), serial.clone()));
                    }
                    let h = core::serial::hash(&serial);
                    self.stored.pending.retain(|(s, _)| *s != h);
                }
                self.stored.last_validated = self.now();
                let used = format!("{} of {}", cert.activations_used, cert.max_activations);
                let perpetual = cert.kind == core::model::LicenceKind::Perpetual;
                self.refresh();
                self.ui.entry.clear();
                self.ui.say(
                    match (r.consumed_use, perpetual) {
                        (true, true) => format!("Activated. Computers active: {used}."),
                        (true, false) => format!("Activated. Activations used: {used}."),
                        (false, _) => "Activated again on this computer. No activation was used.".into(),
                    },
                    false,
                );
            }
            Answer::Activated(Err(e)) => self.ui.say(e, true),
            Answer::Validated(results) => {
                for (activation_id, r) in results {
                    match r {
                        Ok(r) => {
                            if let Ok(c) = self.stored.state.store(&self.verifier, r.activation) {
                                self.stored.state.trusted_time(c.signed_at);
                                // Valid again (an admin undid a revoke): unlock it.
                                self.stored.state.revoked.retain(|l| *l != c.licence_id);
                            }
                        }
                        Err(env) => {
                            // Only a statement signed by the licence server, about this activation
                            // on this computer, takes a licence away.
                            let Ok(st) = self.verifier.verify::<StatusStatement>(&env, Purpose::Status) else { continue };
                            if st.activation_id != activation_id || st.device_hash != self.device {
                                continue;
                            }
                            match st.status {
                                Rejection::Revoked => {
                                    if !self.stored.state.revoked.contains(&st.licence_id) {
                                        self.stored.state.revoked.push(st.licence_id);
                                    }
                                }
                                Rejection::Deactivated => self.stored.state.forget(&self.verifier, &activation_id),
                                _ => {}
                            }
                        }
                    }
                }
                self.stored.last_validated = self.now();
                self.refresh();
                if std::mem::take(&mut self.ui.checking) {
                    let msg = match &self.standing {
                        Standing::Perpetual(c) | Standing::Restricted(core::standing::Reason::UpdatesLapsed(c)) => match c.updates_until {
                            Some(u) => format!("Checked. Software maintenance runs until {}.", core::export::date_time(u).split(',').next().unwrap_or_default()),
                            None => "Checked. Your licence is up to date.".into(),
                        },
                        _ => "Checked with the licence server.".into(),
                    };
                    self.ui.say(msg, false);
                }
            }
            Answer::Renew(Ok(url)) => {
                self.ui.open_url = Some(url);
                self.ui.say("The renewal checkout is open in your browser. After paying, press Check Now here to pick up the new date.".into(), false);
            }
            Answer::Renew(Err(e)) => self.ui.say(e, true),
            Answer::Deactivated(activation_id, Ok(())) => {
                self.stored.state.forget(&self.verifier, &activation_id);
                self.refresh();
                self.ui.say("This computer is deactivated. The activation it used isn't given back.".into(), false);
            }
            Answer::Deactivated(_, Err(e)) => self.ui.say(e, true),
        }
    }

    /// The weekly check: learns about revocations and refreshes the trusted time. Never locks
    /// anything when the server can't be reached.
    fn revalidate(&mut self) {
        let ids: Vec<String> = self.stored.state.certs.iter().filter_map(|e| self.cert(e)).map(|c| c.activation_id).collect();
        let device = self.device.clone();
        let now = self.now();
        match &mut self.server {
            #[cfg(feature = "licence-dev")]
            Server::Test(t) => {
                let r = ids.into_iter().filter_map(|id| t.validate(&id, &device, now).map(|r| (id, r))).collect();
                self.answer(Answer::Validated(r));
            }
            Server::Online(base) => {
                let url = format!("{base}/v1/validate");
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let mut out = Vec::new();
                    for id in ids {
                        let body = serde_json::json!({ "activation_id": id, "device_hash": device });
                        match post_full::<_, ActivationResponse>(&url, &body) {
                            Ok(r) => out.push((id, Ok(r))),
                            Err(PostError { status: Some(env), .. }) => out.push((id, Err(env))),
                            // Offline, the server is down, or an unsigned error: change nothing,
                            // try again next week.
                            Err(_) => {}
                        }
                    }
                    let _ = tx.send(Answer::Validated(out));
                });
                self.rx = Some(rx);
                // Don't retry every frame while offline.
                self.stored.last_validated = now;
            }
        }
    }

    /// Settings > Licensing > Check Now: the weekly check, at once, saying what it found.
    pub fn check_now(&mut self) {
        if self.rx.is_some() || self.stored.state.certs.is_empty() {
            return;
        }
        self.ui.checking = true;
        self.revalidate();
    }

    /// Extend Maintenance: asks the server for a renewal checkout link for this computer's
    /// licence (proved by its activation), then opens it in the browser. The serial isn't sent.
    pub fn renew(&mut self) {
        let Some(c) = self.shown_cert() else { return };
        if self.rx.is_some() {
            return;
        }
        let (id, device) = (c.activation_id.clone(), self.device.clone());
        match &mut self.server {
            #[cfg(feature = "licence-dev")]
            Server::Test(_) => self.answer(Answer::Renew(Err("Maintenance renewals need the real licence server.".into()))),
            Server::Online(base) => {
                let url = format!("{base}/v1/renew");
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let body = serde_json::json!({ "activation_id": id, "device_hash": device });
                    let r = post::<_, serde_json::Value>(&url, &body).and_then(|v| {
                        v["url"].as_str().filter(|u| u.starts_with("https://") || u.starts_with("http://localhost")).map(str::to_string).ok_or_else(|| "The licence server's answer couldn't be read.".to_string())
                    });
                    let _ = tx.send(Answer::Renew(r));
                });
                self.rx = Some(rx);
            }
        }
    }

    /// Deactivate this computer for the licence shown. The use isn't given back.
    pub fn deactivate(&mut self) {
        let Some(c) = self.shown_cert() else { return };
        let (id, device) = (c.activation_id.clone(), self.device.clone());
        #[cfg(feature = "licence-dev")]
        let now = self.now();
        match &mut self.server {
            #[cfg(feature = "licence-dev")]
            Server::Test(t) => {
                let r = t.deactivate(&id, &device, now);
                self.answer(Answer::Deactivated(id, r));
            }
            Server::Online(base) => {
                let url = format!("{base}/v1/deactivate");
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let body = serde_json::json!({ "activation_id": id, "device_hash": device });
                    let r = post::<_, serde_json::Value>(&url, &body).map(|_| ());
                    let _ = tx.send(Answer::Deactivated(id, r));
                });
                self.rx = Some(rx);
            }
        }
    }
}

fn new_activation_id() -> String {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut b).expect("random");
    format!("act_{}", data_encoding::HEXLOWER.encode(&b))
}

fn post<B: Serialize, R: serde::de::DeserializeOwned>(url: &str, body: &B) -> Result<R, String> {
    post_full(url, body).map_err(|e| e.message)
}

struct PostError {
    message: String,
    /// The signed status statement the server sends with a revoked or deactivated reply.
    status: Option<Envelope>,
}

fn post_full<B: Serialize, R: serde::de::DeserializeOwned>(url: &str, body: &B) -> Result<R, PostError> {
    let fail = |message: String| PostError { message, status: None };
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .tls_config(tls)
        .build()
        .new_agent();
    let mut resp = agent
        .post(url)
        .header("Content-Type", "application/json")
        .send(serde_json::to_string(body).unwrap())
        .map_err(|_| fail("Couldn't reach the Filescape licence server. Check the connection, or use offline activation below.".to_string()))?;
    let text = resp.body_mut().read_to_string().map_err(|e| fail(e.to_string()))?;
    if resp.status().is_success() {
        serde_json::from_str(&text).map_err(|_| fail("The licence server's answer couldn't be read.".to_string()))
    } else {
        #[derive(Deserialize)]
        struct E {
            error: Rejection,
            #[serde(default)]
            status: Option<Envelope>,
        }
        Err(match serde_json::from_str::<E>(&text) {
            Ok(e) => PostError { message: e.error.message().to_string(), status: e.status },
            // Answers outside the licence rules (e.g. renewals) carry their own sentence.
            Err(_) => match serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["message"].as_str().map(str::to_string)) {
                Some(m) => fail(m),
                None => fail(format!("The licence server said no ({}).", resp.status())),
            },
        })
    }
}

/// This machine's ID, hashed. Only the hash leaves the machine.
mod device_id {
    pub fn hash() -> String {
        let raw = machine_id().or_else(fallback).unwrap_or_else(|| "unknown".into());
        filescape_licence::device_hash(&raw)
    }

    #[cfg(windows)]
    fn machine_id() -> Option<String> {
        use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW};
        let key: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography\0".encode_utf16().collect();
        let name: Vec<u16> = "MachineGuid\0".encode_utf16().collect();
        let mut buf = [0u16; 128];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
        if r != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.trim().is_empty())
    }

    #[cfg(target_os = "macos")]
    fn machine_id() -> Option<String> {
        let out = std::process::Command::new("/usr/sbin/ioreg").args(["-rd1", "-c", "IOPlatformExpertDevice"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().find(|l| l.contains("IOPlatformUUID"))?;
        line.split('"').nth(3).map(str::to_string)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn machine_id() -> Option<String> {
        ["/etc/machine-id", "/var/lib/dbus/machine-id"].iter().find_map(|p| std::fs::read_to_string(p).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// No machine ID: a random one, kept in the keychain.
    fn fallback() -> Option<String> {
        if let Some(id) = super::keychain_get("device") {
            return Some(id);
        }
        let id = super::new_activation_id();
        super::keychain_set("device", &id).then_some(id)
    }
}

/// Test-serial mode, until the licensing server is live: run with `FILESCAPE_LICENCE_DEV=1` and
/// Settings > Licensing can issue test serials from an in-process server that follows exactly the
/// same rules (it *is* the reference server from licensing/core), and move the licence clock
/// forward to try expiry. Its signing key is made on this machine and trusted only in this mode.
/// The `licence-dev` feature is to be dropped from release builds before launch (phase 4).
#[cfg(feature = "licence-dev")]
pub mod dev {
    use filescape_licence as core;
    use core::model::{ActivationRequest, ActivationResponse};
    use core::rules::{MemoryLedger, TemporaryOptions};
    use core::sign::Signer;
    use core::Ts;
    use std::path::{Path, PathBuf};

    pub fn enabled() -> bool {
        std::env::var("FILESCAPE_LICENCE_DEV").is_ok_and(|v| v == "1")
    }

    pub struct TestServer {
        pub ledger: MemoryLedger,
        dir: PathBuf,
        pub offset_days: i64,
        /// Serials issued this session, for copying.
        pub issued: Vec<(String, String)>,
    }

    impl TestServer {
        pub fn open(dir: Option<&Path>) -> Option<TestServer> {
            let dir = dir?.to_path_buf();
            std::fs::create_dir_all(&dir).ok()?;
            let key_path = dir.join("licence-test-key.pk8");
            let pkcs8 = match std::fs::read(&key_path) {
                Ok(k) => k,
                Err(_) => {
                    let k = Signer::generate_pkcs8(&ring::rand::SystemRandom::new());
                    std::fs::write(&key_path, &k).ok()?;
                    k
                }
            };
            let signer = Signer::from_pkcs8(&pkcs8).ok()?;
            let ledger = match std::fs::read_to_string(dir.join("licence-test-server.json")) {
                Ok(j) => MemoryLedger::from_json(signer, &j).ok()?,
                Err(_) => MemoryLedger::new(signer),
            };
            let offset_days = std::fs::read_to_string(dir.join("licence-test-clock.txt")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            Some(TestServer { ledger, dir, offset_days, issued: Vec::new() })
        }

        pub fn public_key(&self) -> Vec<u8> {
            self.ledger.public_key()
        }

        pub fn save(&self) {
            let _ = std::fs::write(self.dir.join("licence-test-server.json"), self.ledger.to_json());
            let _ = std::fs::write(self.dir.join("licence-test-clock.txt"), self.offset_days.to_string());
        }

        pub fn activate(&mut self, req: &ActivationRequest, now: Ts) -> Result<ActivationResponse, String> {
            let r = self.ledger.activate(req, now).map_err(|e| e.message().to_string());
            self.save();
            r
        }

        /// `None` where the real server would send only an unsigned error.
        pub fn validate(&mut self, id: &str, device: &str, now: Ts) -> Option<Result<ActivationResponse, core::sign::Envelope>> {
            match self.ledger.validate(id, device, now) {
                Ok(r) => Some(Ok(r)),
                Err(why) => self.ledger.status(id, why, now).map(Err),
            }
        }

        pub fn deactivate(&mut self, id: &str, device: &str, now: Ts) -> Result<(), String> {
            let r = self.ledger.deactivate(id, device, now).map_err(|e| e.message().to_string());
            self.save();
            r
        }

        pub fn issue_temporary(&mut self, opts: &TemporaryOptions, now: Ts) {
            if let Ok(v) = self.ledger.issue_temporary(opts, now) {
                for (s, l) in v {
                    self.issued.push((s, format!("Temporary, {}", describe(&l.validity))));
                }
            }
            self.save();
        }

        pub fn issue_perpetual(&mut self, now: Ts) {
            if let Ok((s, l)) = self.ledger.issue_perpetual(Some("Test buyer".into()), None, None, core::DEFAULT_UPDATE_MONTHS, now) {
                self.issued.push((s, format!("Perpetual, {} activations", l.max_activations)));
            }
            self.save();
        }
    }

    fn describe(v: &core::model::Validity) -> String {
        match v {
            core::model::Validity::FirstActivation { days } => format!("{days} days from first activation"),
            core::model::Validity::FixedExpiry { expires_at } => format!("until {}", core::export::date_time(*expires_at)),
            core::model::Validity::Perpetual { .. } => "perpetual".into(),
        }
    }
}

#[cfg(test)]
mod build_tests {
    use super::*;

    #[test]
    fn the_production_key_is_built_in() {
        let keys = trusted_keys();
        assert!(keys.iter().any(|k| core::sign::key_id(k) == "204a936d1dae0b1d"), "production key missing");
    }

    #[test]
    fn the_release_date_comes_from_the_build() {
        assert!(RELEASE_DATE >= 1_795_392_000, "older than the first release: {RELEASE_DATE}");
    }
}

#[cfg(all(test, feature = "licence-dev"))]
mod tests {
    use super::*;
    use filescape_licence::standing::Reason;

    /// The app's licensing with the test server, kept in `dir` as on a real computer.
    fn open(dir: &std::path::Path) -> Licensing {
        let stored = std::fs::read(dir.join("licence.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let t = dev::TestServer::open(Some(dir)).unwrap();
        let verifier = Verifier::new(&[t.public_key()]);
        let mut me = Licensing {
            stored,
            standing: Standing::Restricted(Reason::EvaluationOver),
            verifier,
            device: "test-device".into(),
            dir: Some(dir.to_path_buf()),
            server: Server::Test(Box::new(t)),
            checked: None,
            keychain_high_water: 0,
            rx: None,
            ui: ui::State::default(),
            update: update::UpdateCheck::default(),
        };
        me.refresh();
        me
    }

    fn test_server(l: &mut Licensing) -> &mut dev::TestServer {
        match &mut l.server {
            Server::Test(t) => t,
            _ => unreachable!(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("filescape-licence-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_perpetual_licence_shows_maintenance_and_can_ask_to_extend_it() {
        let dir = temp_dir("maintenance");
        let mut l = open(&dir);
        let now = l.now();
        test_server(&mut l).issue_perpetual(now);
        let serial = test_server(&mut l).issued.last().unwrap().0.clone();
        l.activate(&serial);
        assert!(matches!(l.standing, Standing::Perpetual(_)), "{:?}", l.standing);
        assert!(l.maintained());
        let Standing::Perpetual(c) = l.standing.clone() else { unreachable!() };
        assert!(ui::maintenance_line(c.updates_until, now).starts_with("Active until"));
        assert!(ui::maintenance_line(c.updates_until, c.updates_until.unwrap()).starts_with("Ended"));
        // Check Now asks the server and says the date.
        l.check_now();
        let (msg, err) = l.ui.message.clone().unwrap();
        assert!(!err && msg.contains("Software maintenance runs until"), "{msg}");
        // Extend Maintenance: the test server can't sell renewals and says so.
        l.renew();
        let (msg, err) = l.ui.message.clone().unwrap();
        assert!(err && msg.contains("real licence server"), "{msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relaunching_or_activating_again_uses_no_more_activations() {
        let dir = temp_dir("relaunch");
        let mut l = open(&dir);
        let now = l.now();
        test_server(&mut l).issue_temporary(&Default::default(), now);
        let serial = test_server(&mut l).issued[0].0.clone();
        l.activate(&serial);
        let Standing::Temporary { cert, .. } = &l.standing else { panic!("{:?}", l.standing) };
        assert_eq!((cert.activations_used, cert.max_activations), (1, 3));
        let id = cert.licence_id.clone();

        // Start the app again: the licence is still in use and no activation was used.
        drop(l);
        let mut l = open(&dir);
        assert!(matches!(&l.standing, Standing::Temporary { cert, .. } if cert.licence_id == id));
        // Typing the serial again on the same computer doesn't use another activation either.
        l.activate(&serial);
        assert_eq!(test_server(&mut l).ledger.licence(&id).unwrap().activations_used, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expiry_stops_new_work_but_not_work_already_started() {
        let dir = temp_dir("expiry");
        let mut l = open(&dir);
        let now = l.now();
        test_server(&mut l).issue_temporary(&Default::default(), now);
        let serial = test_server(&mut l).issued[0].0.clone();
        l.activate(&serial);
        let running = l.begin(Feature::BatchRename).expect("licensed");

        // 31 days on: past the licence (30 days) and the evaluation (15).
        test_server(&mut l).offset_days = 31;
        l.refresh();
        assert!(matches!(l.standing, Standing::Restricted(Reason::TemporaryExpired(_))), "{:?}", l.standing);
        // The rename that already started keeps its permit; nothing checks it again.
        assert_eq!(running.0, Feature::BatchRename);
        // A new one asks for a licence, and the free features carry on.
        assert!(l.begin(Feature::BatchRename).is_none());
        assert_eq!(l.ui.prompt, Some(Feature::BatchRename));
        assert!(l.begin(Feature::FileBrowser).is_some() && l.begin(Feature::DiskMap).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_signed_statement_takes_a_licence_away_and_reinstating_unlocks() {
        let dir = temp_dir("signed");
        let mut l = open(&dir);
        let now = l.now();
        test_server(&mut l).issue_perpetual(now);
        let serial = test_server(&mut l).issued[0].0.clone();
        l.activate(&serial);
        let Standing::Perpetual(cert) = l.standing.clone() else { panic!("{:?}", l.standing) };
        // An unsigned or forged answer changes nothing.
        l.answer(Answer::Validated(vec![(cert.activation_id.clone(), Err(Envelope { format: "x".into(), purpose: "status".into(), key_id: "x".into(), payload: String::new(), signature: String::new() }))]));
        assert!(matches!(l.standing, Standing::Perpetual(_)));
        // Past the 15-day evaluation, so only the licence counts.
        test_server(&mut l).offset_days = 20;
        let now = l.now();
        // The server revokes it: the signed statement locks it.
        test_server(&mut l).ledger.revoke(&cert.licence_id, "refund", now).unwrap();
        l.revalidate();
        assert!(matches!(l.standing, Standing::Restricted(Reason::Revoked)), "{:?}", l.standing);
        // Reinstated by mistake-fix: the next check unlocks it without typing the serial again.
        test_server(&mut l).ledger.unrevoke(&cert.licence_id, "mistake", now).unwrap();
        l.revalidate();
        assert!(matches!(l.standing, Standing::Perpetual(_)), "{:?}", l.standing);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Shown next to this computer on the customer's account page.
fn os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    }
}
