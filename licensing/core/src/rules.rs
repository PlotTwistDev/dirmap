//! The activation rules, and the reference ledger that applies them.
//!
//! [`decide`] is pure: given a licence, its activations and a request, it says what should happen.
//! The Cloudflare Worker turns a [`Decision::NewUse`] into one D1 batch (a conditional
//! `UPDATE … WHERE used < max …` plus the activation `INSERT`), so the count can't be exceeded by
//! concurrent requests. [`MemoryLedger`] does the same under a mutex; it is the reference
//! implementation the tests run against, and what the app's tests use as a fake server.
//!
//! The spec's rules, by number:
//! 1–4. each new activation consumes one use; a fourth (past the limit) is rejected
//! 5. deactivating never gives a use back
//! 6, 10. reinstalling or relaunching on the same machine re-issues, it doesn't consume
//! 7. every activation shares the expiry fixed by the first
//! 8. failed attempts consume nothing
//! 9. revalidation (or a retry with the same activation ID) consumes nothing

use crate::model::*;
use crate::serial;
use crate::sign::{Purpose, Signer};
use crate::{DAY, ISSUER, Ts};
use ring::rand::SecureRandom;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Hand back a fresh certificate for this existing activation; nothing is consumed.
    Reissue { activation_id: String },
    /// A new activation: consume one use. On the first ever use, `first_activated_at` and
    /// `expires_at` are set; later uses leave them alone.
    NewUse { first_activation: Option<(Ts, Option<Ts>)> },
}

/// What should happen for `req` (whose serial has already been matched to `lic`).
pub fn decide(lic: &Licence, activations: &[Activation], req: &ActivationRequest, now: Ts) -> Result<Decision, Rejection> {
    if lic.revoked_at.is_some() {
        return Err(Rejection::Revoked);
    }
    let expired = lic.expires_at.is_some_and(|e| now >= e);
    // A retry, or revalidation by activation ID.
    if let Some(a) = activations.iter().find(|a| a.activation_id == req.activation_id) {
        if a.licence_id != lic.id || a.device_hash != req.device_hash {
            return Err(Rejection::ActivationIdConflict);
        }
        if !a.is_live() {
            return Err(Rejection::Deactivated);
        }
        if expired {
            return Err(Rejection::Expired);
        }
        return Ok(Decision::Reissue { activation_id: a.activation_id.clone() });
    }
    // The same machine again (reinstalled, settings wiped): re-issue its live activation.
    if let Some(a) = activations.iter().find(|a| a.licence_id == lic.id && a.device_hash == req.device_hash && a.is_live()) {
        if expired {
            return Err(Rejection::Expired);
        }
        return Ok(Decision::Reissue { activation_id: a.activation_id.clone() });
    }
    if expired {
        return Err(Rejection::Expired);
    }
    if lic.activations_used >= lic.max_activations {
        return Err(Rejection::ActivationLimitReached);
    }
    let first_activation = match lic.first_activated_at {
        Some(_) => None,
        None => Some((now, lic.validity.expiry_from(now))),
    };
    Ok(Decision::NewUse { first_activation })
}

/// Options for issuing temporary licences. `Default` is 30 days from first activation, 3 uses.
#[derive(Debug, Clone)]
pub struct TemporaryOptions {
    pub validity: Validity,
    pub max_activations: u32,
    pub count: u32,
    pub owner: Option<String>,
    pub email: Option<String>,
    pub purpose: Option<String>,
}

impl Default for TemporaryOptions {
    fn default() -> Self {
        TemporaryOptions {
            validity: Validity::FirstActivation { days: crate::DEFAULT_TEMPORARY_DAYS },
            max_activations: crate::DEFAULT_MAX_ACTIVATIONS,
            count: 1,
            owner: None,
            email: None,
            purpose: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueError {
    /// A temporary licence needs a temporary validity, and a fixed expiry must be in the future.
    BadValidity,
    BadLimit,
    /// This Paddle order already has a licence.
    DuplicateOrder(String),
    /// Only perpetual licences have adjustable limits; a temporary licence keeps what it was
    /// issued with.
    NotPerpetual,
    NotFound,
}

/// One row of the admin Activations page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActivationReportRow {
    pub licence_id: String,
    pub kind: LicenceKind,
    pub owner: Option<String>,
    pub activations_used: u32,
    pub max_activations: u32,
    /// Machines activated in the last [`crate::SHARING_WINDOW_DAYS`] days.
    pub recent_machines: usize,
    /// Machines with a live activation now.
    pub live_machines: usize,
    /// More than [`crate::SHARING_FLAG_MACHINES`] new machines in the window: maybe shared.
    pub flagged: bool,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct State {
    licences: HashMap<String, Licence>,
    by_serial: HashMap<String, String>,
    by_order: HashMap<String, String>,
    activations: Vec<Activation>,
    #[serde(default = "default_perpetual")]
    perpetual_limit: u32,
}

fn default_perpetual() -> u32 {
    crate::DEFAULT_PERPETUAL_ACTIVATIONS
}

/// The reference licensing server, in memory.
pub struct MemoryLedger {
    state: Mutex<State>,
    signer: Signer,
    rng: ring::rand::SystemRandom,
}

fn random_id(rng: &dyn SecureRandom, prefix: &str) -> String {
    let mut b = [0u8; 8];
    rng.fill(&mut b).expect("system random number generator failed");
    format!("{prefix}-{}", &data_encoding::BASE32_NOPAD.encode(&b)[..12])
}

fn event(lic: &mut Licence, at: Ts, what: &str, detail: impl Into<String>) {
    lic.events.push(Event { at, what: what.into(), detail: detail.into() });
}

impl MemoryLedger {
    pub fn new(signer: Signer) -> MemoryLedger {
        let state = State { perpetual_limit: default_perpetual(), ..State::default() };
        MemoryLedger { state: Mutex::new(state), signer, rng: ring::rand::SystemRandom::new() }
    }

    /// Restores a ledger saved with [`MemoryLedger::to_json`] (the app's test-serial mode keeps
    /// one on disk).
    pub fn from_json(signer: Signer, json: &str) -> Result<MemoryLedger, String> {
        let state: State = serde_json::from_str(json).map_err(|e| e.to_string())?;
        Ok(MemoryLedger { state: Mutex::new(state), signer, rng: ring::rand::SystemRandom::new() })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&*self.state.lock().unwrap()).unwrap()
    }

    /// Admin: the activation limit new perpetual licences get.
    pub fn set_default_perpetual_limit(&self, max: u32) -> Result<(), IssueError> {
        if max == 0 {
            return Err(IssueError::BadLimit);
        }
        self.state.lock().unwrap().perpetual_limit = max;
        Ok(())
    }

    pub fn default_perpetual_limit(&self) -> u32 {
        self.state.lock().unwrap().perpetual_limit
    }

    /// Admin: change one perpetual licence's limit. Lowering it below the uses already consumed
    /// blocks new activations only; machines already activated keep working.
    pub fn set_activation_limit(&self, licence_id: &str, max: u32, reason: &str, now: Ts) -> Result<(), IssueError> {
        if max == 0 {
            return Err(IssueError::BadLimit);
        }
        let mut st = self.state.lock().unwrap();
        let lic = st.licences.get_mut(licence_id).ok_or(IssueError::NotFound)?;
        if lic.kind() != LicenceKind::Perpetual {
            return Err(IssueError::NotPerpetual);
        }
        let before = lic.max_activations;
        lic.max_activations = max;
        event(lic, now, "limit changed", format!("{before} to {max}: {reason}"));
        Ok(())
    }

    /// Admin Activations page: every licence that has been activated, busiest first.
    pub fn activation_report(&self, now: Ts) -> Vec<ActivationReportRow> {
        let st = self.state.lock().unwrap();
        let since = now - crate::SHARING_WINDOW_DAYS * DAY;
        let mut rows: Vec<ActivationReportRow> = st
            .licences
            .values()
            .filter(|l| l.first_activated_at.is_some())
            .map(|l| {
                let acts = st.activations.iter().filter(|a| a.licence_id == l.id);
                let mut recent: Vec<&str> = acts.clone().filter(|a| a.created_at >= since).map(|a| a.device_hash.as_str()).collect();
                recent.sort_unstable();
                recent.dedup();
                let live = acts.filter(|a| a.is_live()).count();
                ActivationReportRow {
                    licence_id: l.id.clone(),
                    kind: l.kind(),
                    owner: l.owner.clone(),
                    activations_used: l.activations_used,
                    max_activations: l.max_activations,
                    recent_machines: recent.len(),
                    live_machines: live,
                    flagged: recent.len() > crate::SHARING_FLAG_MACHINES,
                }
            })
            .collect();
        rows.sort_by(|a, b| b.flagged.cmp(&a.flagged).then(b.recent_machines.cmp(&a.recent_machines)).then(a.licence_id.cmp(&b.licence_id)));
        rows
    }

    pub fn public_key(&self) -> Vec<u8> {
        self.signer.public_key()
    }

    fn insert(&self, st: &mut State, serial: &str, mut lic: Licence, now: Ts) -> Licence {
        lic.serial_hash = serial::hash(serial);
        lic.serial_hint = serial::hint(serial);
        let what = format!("{:?}", lic.validity);
        event(&mut lic, now, "issued", what);
        st.by_serial.insert(lic.serial_hash.clone(), lic.id.clone());
        if let Some(o) = &lic.order_ref {
            st.by_order.insert(o.clone(), lic.id.clone());
        }
        st.licences.insert(lic.id.clone(), lic.clone());
        lic
    }

    fn blank(&self, validity: Validity, max: u32, now: Ts) -> Licence {
        Licence {
            id: random_id(&self.rng, "LIC"),
            serial_hash: String::new(),
            serial_hint: String::new(),
            validity,
            owner: None,
            email: None,
            purpose: None,
            issued_at: now,
            first_activated_at: None,
            expires_at: match validity {
                Validity::FixedExpiry { expires_at } => Some(expires_at),
                _ => None,
            },
            max_activations: max,
            activations_used: 0,
            revoked_at: None,
            revoke_reason: None,
            order_ref: None,
            events: Vec::new(),
        }
    }

    /// Create Temporary Licence: one or more serials. The serials are returned once and never
    /// stored.
    pub fn issue_temporary(&self, opts: &TemporaryOptions, now: Ts) -> Result<Vec<(String, Licence)>, IssueError> {
        match opts.validity {
            Validity::FirstActivation { days } if days >= 1 => {}
            Validity::FixedExpiry { expires_at } if expires_at > now => {}
            _ => return Err(IssueError::BadValidity),
        }
        if opts.max_activations == 0 {
            return Err(IssueError::BadLimit);
        }
        let mut st = self.state.lock().unwrap();
        let mut out = Vec::new();
        for _ in 0..opts.count.max(1) {
            let serial = serial::generate(LicenceKind::Temporary, &self.rng);
            let mut lic = self.blank(opts.validity, opts.max_activations, now);
            lic.owner = opts.owner.clone();
            lic.email = opts.email.clone();
            lic.purpose = opts.purpose.clone();
            out.push((serial.clone(), self.insert(&mut st, &serial, lic, now)));
        }
        Ok(out)
    }

    /// A perpetual licence, as a Paddle purchase creates. `order_ref` (the Paddle transaction ID)
    /// makes a resent webhook harmless.
    pub fn issue_perpetual(&self, owner: Option<String>, email: Option<String>, order_ref: Option<String>, update_months: u32, now: Ts) -> Result<(String, Licence), IssueError> {
        let mut st = self.state.lock().unwrap();
        if let Some(o) = &order_ref {
            if let Some(id) = st.by_order.get(o) {
                return Err(IssueError::DuplicateOrder(id.clone()));
            }
        }
        let serial = serial::generate(LicenceKind::Perpetual, &self.rng);
        // Months as 30.44 days: entitlement is a date, not a calendar computation.
        let updates_until = now + i64::from(update_months) * 2_629_746;
        let limit = st.perpetual_limit;
        let mut lic = self.blank(Validity::Perpetual { updates_until }, limit, now);
        lic.owner = owner;
        lic.email = email;
        lic.order_ref = order_ref;
        let lic = self.insert(&mut st, &serial, lic, now);
        Ok((serial, lic))
    }

    fn certs(&self, lic: &Licence, a: &Activation, now: Ts) -> (crate::sign::Envelope, crate::sign::Envelope) {
        let act = ActivationCert {
            licence_id: lic.id.clone(),
            kind: lic.kind(),
            owner: lic.owner.clone(),
            serial_hint: lic.serial_hint.clone(),
            activation_id: a.activation_id.clone(),
            device_hash: a.device_hash.clone(),
            issued_at: lic.issued_at,
            first_activated_at: lic.first_activated_at.unwrap_or(a.created_at),
            expires_at: lic.expires_at,
            updates_until: lic.validity.updates_until(),
            activations_used: lic.activations_used,
            max_activations: lic.max_activations,
            signed_at: now,
        };
        (self.signer.sign(Purpose::Activation, &act), self.signer.sign(Purpose::Issuance, &issuance(lic, now)))
    }

    /// POST /v1/activate (and the offline upload).
    pub fn activate(&self, req: &ActivationRequest, now: Ts) -> Result<ActivationResponse, Rejection> {
        let (_, canonical) = serial::normalize(&req.serial).map_err(|_| Rejection::BadSerial)?;
        let mut st = self.state.lock().unwrap();
        let id = st.by_serial.get(&serial::hash(&canonical)).cloned().ok_or(Rejection::UnknownSerial)?;
        let State { licences, activations, .. } = &mut *st;
        let lic = licences.get_mut(&id).unwrap();
        let decision = decide(lic, activations, req, now)?;
        let (activation_id, consumed) = match decision {
            Decision::Reissue { activation_id } => {
                event(lic, now, "revalidated", activation_id.clone());
                (activation_id, false)
            }
            Decision::NewUse { first_activation } => {
                if let Some((first, expires)) = first_activation {
                    lic.first_activated_at = Some(first);
                    lic.expires_at = expires;
                }
                lic.activations_used += 1;
                activations.push(Activation {
                    activation_id: req.activation_id.clone(),
                    licence_id: lic.id.clone(),
                    device_hash: req.device_hash.clone(),
                    app_version: req.app_version.clone(),
                    created_at: now,
                    counted: true,
                    ended_at: None,
                    voided: None,
                });
                event(lic, now, "activated", format!("{} ({} of {})", req.activation_id, lic.activations_used, lic.max_activations));
                (req.activation_id.clone(), true)
            }
        };
        let a = activations.iter().find(|a| a.activation_id == activation_id).unwrap();
        let (activation, issuance) = self.certs(lic, a, now);
        Ok(ActivationResponse { activation, issuance, status: lic.status(now), consumed_use: consumed })
    }

    /// The offline route: same rules, same limit.
    pub fn process_offline(&self, file: &str, now: Ts) -> Result<ActivationResponse, Rejection> {
        let req: OfflineRequest = serde_json::from_str(file).map_err(|_| Rejection::BadSerial)?;
        if req.format != OFFLINE_REQUEST_FORMAT {
            return Err(Rejection::BadSerial);
        }
        self.activate(&req.request, now)
    }

    /// POST /v1/validate: the weekly check. Never consumes a use.
    pub fn validate(&self, activation_id: &str, device_hash: &str, now: Ts) -> Result<ActivationResponse, Rejection> {
        let mut st = self.state.lock().unwrap();
        let State { licences, activations, .. } = &mut *st;
        let a = activations.iter().find(|a| a.activation_id == activation_id).ok_or(Rejection::NotFound)?;
        if a.device_hash != device_hash {
            return Err(Rejection::ActivationIdConflict);
        }
        if !a.is_live() {
            return Err(Rejection::Deactivated);
        }
        let lic = licences.get_mut(&a.licence_id).unwrap();
        if lic.revoked_at.is_some() {
            return Err(Rejection::Revoked);
        }
        if lic.expires_at.is_some_and(|e| now >= e) {
            return Err(Rejection::Expired);
        }
        let (activation, issuance) = self.certs(lic, a, now);
        Ok(ActivationResponse { activation, issuance, status: lic.status(now), consumed_use: false })
    }

    /// POST /v1/deactivate: ends this device's activation. The use is not given back.
    pub fn deactivate(&self, activation_id: &str, device_hash: &str, now: Ts) -> Result<(), Rejection> {
        let mut st = self.state.lock().unwrap();
        let State { licences, activations, .. } = &mut *st;
        let a = activations.iter_mut().find(|a| a.activation_id == activation_id).ok_or(Rejection::NotFound)?;
        if a.device_hash != device_hash {
            return Err(Rejection::ActivationIdConflict);
        }
        if a.ended_at.is_none() {
            a.ended_at = Some(now);
            let lic = licences.get_mut(&a.licence_id).unwrap();
            event(lic, now, "deactivated", activation_id);
        }
        Ok(())
    }

    /// Admin: revoke. Existing activations stop at their next check.
    pub fn revoke(&self, licence_id: &str, reason: &str, now: Ts) -> Result<(), Rejection> {
        let mut st = self.state.lock().unwrap();
        let lic = st.licences.get_mut(licence_id).ok_or(Rejection::NotFound)?;
        if lic.revoked_at.is_none() {
            lic.revoked_at = Some(now);
            lic.revoke_reason = Some(reason.into());
            event(lic, now, "revoked", reason);
        }
        Ok(())
    }

    /// Admin: cancel an activation made in error, optionally giving its use back. Never touches
    /// the expiry or the first activation.
    pub fn void_activation(&self, licence_id: &str, activation_id: &str, give_back_use: bool, reason: &str, now: Ts) -> Result<(), Rejection> {
        let mut st = self.state.lock().unwrap();
        let State { licences, activations, .. } = &mut *st;
        let lic = licences.get_mut(licence_id).ok_or(Rejection::NotFound)?;
        let a = activations.iter_mut().find(|a| a.activation_id == activation_id && a.licence_id == licence_id).ok_or(Rejection::NotFound)?;
        if a.voided.is_none() {
            a.voided = Some((now, reason.into()));
        }
        if give_back_use && a.counted {
            a.counted = false;
            lic.activations_used = lic.activations_used.saturating_sub(1);
        }
        event(lic, now, "voided", format!("{activation_id}: {reason}{}", if give_back_use { " (use given back)" } else { "" }));
        Ok(())
    }

    /// Admin: Reset activations. Gives every used activation back; machines already activated
    /// keep working. Expiry, first activation and history are kept.
    pub fn reset_activations(&self, licence_id: &str, reason: &str, now: Ts) -> Result<(), Rejection> {
        let mut st = self.state.lock().unwrap();
        let State { licences, activations, .. } = &mut *st;
        let lic = licences.get_mut(licence_id).ok_or(Rejection::NotFound)?;
        for a in activations.iter_mut().filter(|a| a.licence_id == licence_id) {
            a.counted = false;
        }
        let before = lic.activations_used;
        lic.activations_used = 0;
        event(lic, now, "reset", format!("{before} use(s) given back: {reason}"));
        Ok(())
    }

    pub fn licence(&self, id: &str) -> Option<Licence> {
        self.state.lock().unwrap().licences.get(id).cloned()
    }

    pub fn activations(&self, licence_id: &str) -> Vec<Activation> {
        self.state.lock().unwrap().activations.iter().filter(|a| a.licence_id == licence_id).cloned().collect()
    }

    /// Admin search: licence ID, serial (exact), owner or email (part), or expiring in [from, to).
    pub fn search(&self, text: &str, expiring: Option<(Ts, Ts)>) -> Vec<Licence> {
        let st = self.state.lock().unwrap();
        let needle = text.trim().to_lowercase();
        let by_serial = serial::normalize(text).ok().and_then(|(_, s)| st.by_serial.get(&serial::hash(&s)).cloned());
        let mut out: Vec<Licence> = st
            .licences
            .values()
            .filter(|l| {
                let text_ok = needle.is_empty()
                    || by_serial.as_deref() == Some(l.id.as_str())
                    || l.id.to_lowercase() == needle
                    || l.owner.as_deref().is_some_and(|o| o.to_lowercase().contains(&needle))
                    || l.email.as_deref().is_some_and(|o| o.to_lowercase().contains(&needle));
                let date_ok = expiring.is_none_or(|(a, b)| l.expires_at.is_some_and(|e| e >= a && e < b));
                text_ok && date_ok
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.issued_at.cmp(&a.issued_at).then(a.id.cmp(&b.id)));
        out
    }

    /// Days of validity left, for admin lists.
    pub fn days_left(lic: &Licence, now: Ts) -> Option<i64> {
        lic.expires_at.map(|e| ((e - now).max(0) + DAY - 1) / DAY)
    }
}

/// The proof-of-issuance contents for a licence as it stands.
pub fn issuance(lic: &Licence, now: Ts) -> IssuanceCert {
    IssuanceCert {
        licence_id: lic.id.clone(),
        kind: lic.kind(),
        owner: lic.owner.clone(),
        serial_hint: lic.serial_hint.clone(),
        issued_at: lic.issued_at,
        first_activated_at: lic.first_activated_at,
        expires_at: lic.expires_at,
        updates_until: lic.validity.updates_until(),
        validity: lic.validity,
        max_activations: lic.max_activations,
        activations_used: lic.activations_used,
        non_renewable: lic.kind() == LicenceKind::Temporary,
        issuer: ISSUER.into(),
        as_of: now,
    }
}
