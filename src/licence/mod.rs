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

use filescape_licence as core;
use core::model::{ActivationCert, ActivationRequest, ActivationResponse, IssuanceCert, OFFLINE_REQUEST_FORMAT, OfflineRequest, Rejection};
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
/// Release date of this build, for perpetual update entitlement. Set at each release.
pub const RELEASE_DATE: Ts = 1_795_392_000; // 23 Nov 2026
const KEYCHAIN: &str = "filescape-licence";
/// How often to re-check the standing while running (also on regaining focus).
const RECHECK: Duration = Duration::from_secs(60);
/// The weekly online check, when the machine is online. A failed check never locks anything.
const REVALIDATE_EVERY: Ts = 7 * DAY;

/// Public keys of the licensing server. Empty until the production key is made (phase 3);
/// `FILESCAPE_LICENCE_PUBKEY` (base64url) adds one at build time.
fn trusted_keys() -> Vec<Vec<u8>> {
    let mut keys = Vec::new();
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
    Validated(Vec<(String, Result<ActivationResponse, Rejection>)>),
    Deactivated(String, Result<(), String>),
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
        Ok(ActivationRequest { serial, activation_id: id, device_hash: self.device.clone(), app_version: env!("CARGO_PKG_VERSION").into() })
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

    /// Offline activation, step 2: the response file from api.filescape.app/activate.
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
                self.refresh();
                self.ui.entry.clear();
                self.ui.say(if r.consumed_use { format!("Activated. Activations used: {used}.") } else { "Activated again on this computer. No activation was used.".into() }, false);
            }
            Answer::Activated(Err(e)) => self.ui.say(e, true),
            Answer::Validated(results) => {
                for (activation_id, r) in results {
                    match r {
                        Ok(r) => {
                            if let Ok(c) = self.stored.state.store(&self.verifier, r.activation) {
                                self.stored.state.trusted_time(c.signed_at);
                            }
                        }
                        Err(Rejection::Revoked) => {
                            if let Some(c) = self.stored.state.certs.iter().filter_map(|e| self.cert(e)).find(|c| c.activation_id == activation_id) {
                                self.stored.state.revoked.push(c.licence_id);
                            }
                        }
                        Err(Rejection::Deactivated | Rejection::NotFound) => self.stored.state.forget(&self.verifier, &activation_id),
                        Err(_) => {}
                    }
                }
                self.stored.last_validated = self.now();
                self.refresh();
            }
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
                let r = ids.into_iter().map(|id| (id.clone(), t.validate(&id, &device, now))).collect();
                self.answer(Answer::Validated(r));
            }
            Server::Online(base) => {
                let url = format!("{base}/v1/validate");
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let mut out = Vec::new();
                    for id in ids {
                        let body = serde_json::json!({ "activation_id": id, "device_hash": device });
                        match post::<_, ActivationResponse>(&url, &body) {
                            Ok(r) => out.push((id, Ok(r))),
                            Err(e) => {
                                // Offline or the server is down: try again next week.
                                if let Some(r) = rejection_of(&e) {
                                    out.push((id, Err(r)));
                                }
                            }
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

/// The server's error replies are `{"error": "<rejection>"}`; the message is shown as is.
fn rejection_of(message: &str) -> Option<Rejection> {
    [
        Rejection::BadSerial,
        Rejection::UnknownSerial,
        Rejection::Revoked,
        Rejection::Expired,
        Rejection::ActivationLimitReached,
        Rejection::ActivationIdConflict,
        Rejection::Deactivated,
        Rejection::NotFound,
    ]
    .into_iter()
    .find(|r| r.message() == message)
}

fn post<B: Serialize, R: serde::de::DeserializeOwned>(url: &str, body: &B) -> Result<R, String> {
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
        .map_err(|_| "Couldn't reach the Filescape licence server. Check the connection, or use offline activation below.".to_string())?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        serde_json::from_str(&text).map_err(|_| "The licence server's answer couldn't be read.".to_string())
    } else {
        #[derive(Deserialize)]
        struct E {
            error: Rejection,
        }
        Err(serde_json::from_str::<E>(&text).map(|e| e.error.message().to_string()).unwrap_or_else(|_| format!("The licence server said no ({}).", resp.status())))
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
    use core::model::{ActivationRequest, ActivationResponse, Rejection};
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

        pub fn validate(&mut self, id: &str, device: &str, now: Ts) -> Result<ActivationResponse, Rejection> {
            self.ledger.validate(id, device, now)
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
}
