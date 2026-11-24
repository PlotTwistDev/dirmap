//! Which licence applies on this machine now.
//!
//! Precedence, best first:
//! 1. a valid perpetual licence whose update entitlement covers this build
//! 2. a valid temporary licence (the latest expiry, if several)
//! 3. the evaluation, within 15 days of the first launch on this machine
//! 4. restricted
//!
//! An expired temporary licence is skipped, so it can never hide or cancel a perpetual one. The
//! evaluation is counted from the first launch, so a temporary licence expiring can't restart it.
//!
//! Clock checks: the app keeps the latest time it has seen ("high-water mark", in the licence
//! file and the keychain, larger wins). If the clock is more than [`ROLLBACK_TOLERANCE`] behind it,
//! or behind the newest certificate's server signing time, temporary licences and the evaluation
//! lock until the clock is right again or an online check supplies a trusted time. Perpetual
//! licences never depend on the clock.
//!
//! A machine that never goes online can't be fully protected from someone who controls both its
//! clock and every copy of these records; the Help Centre says so.

use crate::model::{ActivationCert, LicenceKind};
use crate::sign::{Envelope, Purpose, Verifier};
use crate::{DAY, EVALUATION_DAYS, ROLLBACK_TOLERANCE, Ts};
use serde::{Deserialize, Serialize};

/// What the app keeps (licence.json, mirrored in the keychain where noted).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalState {
    /// First launch on this machine (keychain too; the earliest copy wins).
    pub first_launch: Option<Ts>,
    /// Latest time seen (keychain too; the latest copy wins).
    pub high_water: Ts,
    /// Activation certificates, as signed by the server.
    pub certs: Vec<Envelope>,
    /// Licences the server reported revoked.
    pub revoked: Vec<String>,
}

impl LocalState {
    /// Merge the keychain's copy: earliest first launch, latest high-water mark.
    pub fn merge_keychain(&mut self, first_launch: Option<Ts>, high_water: Ts) {
        self.first_launch = match (self.first_launch, first_launch) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.high_water = self.high_water.max(high_water);
    }

    /// Call on every launch and periodically. Starts the evaluation clock on the first launch and
    /// moves the high-water mark forward (never back).
    pub fn observe(&mut self, now: Ts) {
        self.first_launch.get_or_insert(now);
        self.high_water = self.high_water.max(now);
    }

    /// A time the server vouched for. It replaces the high-water mark, so a clock that was wrong
    /// in the future and has been fixed unlocks again.
    pub fn trusted_time(&mut self, server_now: Ts) {
        self.high_water = server_now;
    }

    /// Keep this certificate, replacing any older one for the same activation.
    pub fn store(&mut self, verifier: &Verifier, env: Envelope) -> Result<ActivationCert, crate::sign::VerifyError> {
        let cert: ActivationCert = verifier.verify(&env, Purpose::Activation)?;
        self.certs.retain(|e| verifier.verify::<ActivationCert>(e, Purpose::Activation).map_or(true, |c| c.activation_id != cert.activation_id));
        self.certs.push(env);
        self.high_water = self.high_water.max(cert.signed_at);
        Ok(cert)
    }

    pub fn forget(&mut self, verifier: &Verifier, activation_id: &str) {
        self.certs.retain(|e| verifier.verify::<ActivationCert>(e, Purpose::Activation).map_or(true, |c| c.activation_id != activation_id));
    }
}

/// What the app knows about itself.
pub struct Context<'a> {
    pub now: Ts,
    pub device_hash: &'a str,
    /// Release date of this build, for perpetual update entitlement.
    pub build_date: Ts,
    pub verifier: &'a Verifier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Never licensed and the evaluation is over.
    EvaluationOver,
    /// The temporary licence ended (the newest one, for the expiry screen).
    TemporaryExpired(ActivationCert),
    /// The clock is behind a time already seen.
    ClockRollback,
    /// A perpetual licence whose update entitlement ended before this build was released.
    UpdatesLapsed(ActivationCert),
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    Perpetual(ActivationCert),
    Temporary { cert: ActivationCert, days_left: i64 },
    Evaluation { ends_at: Ts, days_left: i64 },
    Restricted(Reason),
}

impl Standing {
    pub fn is_licensed(&self) -> bool {
        !matches!(self, Standing::Restricted(_))
    }
}

fn days_left(end: Ts, now: Ts) -> i64 {
    ((end - now).max(0) + DAY - 1) / DAY
}

pub fn evaluate(ctx: &Context, state: &LocalState) -> Standing {
    // Only certificates signed by a trusted key, for this machine.
    let certs: Vec<ActivationCert> = state
        .certs
        .iter()
        .filter_map(|e| ctx.verifier.verify::<ActivationCert>(e, Purpose::Activation).ok())
        .filter(|c| c.device_hash == ctx.device_hash)
        .collect();
    let revoked = |c: &ActivationCert| state.revoked.contains(&c.licence_id);
    let newest_signed = certs.iter().map(|c| c.signed_at).max().unwrap_or(0);
    let rolled_back = ctx.now + ROLLBACK_TOLERANCE < state.high_water.max(newest_signed);

    // 1. Perpetual.
    let perpetual: Vec<&ActivationCert> = certs.iter().filter(|c| c.kind == LicenceKind::Perpetual && !revoked(c)).collect();
    if let Some(c) = perpetual.iter().find(|c| c.updates_until.is_none_or(|u| ctx.build_date <= u)) {
        return Standing::Perpetual((*c).clone());
    }
    // 2. Temporary.
    let temporary: Vec<&ActivationCert> = certs.iter().filter(|c| c.kind == LicenceKind::Temporary).collect();
    if !rolled_back {
        if let Some(c) = temporary
            .iter()
            .filter(|c| !revoked(c) && c.expires_at.is_some_and(|e| ctx.now < e))
            .max_by_key(|c| c.expires_at)
        {
            return Standing::Temporary { cert: (*c).clone(), days_left: days_left(c.expires_at.unwrap(), ctx.now) };
        }
    }
    // 3. Evaluation.
    let first = state.first_launch.unwrap_or(ctx.now);
    let eval_end = first + i64::from(EVALUATION_DAYS) * DAY;
    if !rolled_back && ctx.now < eval_end {
        return Standing::Evaluation { ends_at: eval_end, days_left: days_left(eval_end, ctx.now) };
    }
    // 4. Restricted, with the most useful reason.
    let time_limited_would_apply = ctx.now < eval_end || temporary.iter().any(|c| !revoked(c) && c.expires_at.is_some_and(|e| ctx.now < e));
    if rolled_back && time_limited_would_apply {
        return Standing::Restricted(Reason::ClockRollback);
    }
    if let Some(c) = temporary.iter().filter(|c| !revoked(c)).max_by_key(|c| c.expires_at) {
        return Standing::Restricted(Reason::TemporaryExpired((*c).clone()));
    }
    if let Some(c) = perpetual.first() {
        return Standing::Restricted(Reason::UpdatesLapsed((*c).clone()));
    }
    if certs.iter().any(revoked) {
        return Standing::Restricted(Reason::Revoked);
    }
    Standing::Restricted(Reason::EvaluationOver)
}
