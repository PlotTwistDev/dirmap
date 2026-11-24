//! Licences, activations, certificates, requests and responses.

use crate::sign::Envelope;
use crate::{DAY, Ts};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LicenceKind {
    Temporary,
    Perpetual,
}

/// How long a licence lasts. Set once when it is issued; nothing can change it afterwards
/// (temporary licences are non-renewable: more time means a new serial).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Validity {
    /// Temporary: runs for `days` from the first successful activation.
    FirstActivation { days: u32 },
    /// Temporary: ends at a fixed time, whenever it is activated (a training event, say).
    FixedExpiry { expires_at: Ts },
    /// Perpetual: never expires; covers builds released up to `updates_until`.
    Perpetual { updates_until: Ts },
}

impl Validity {
    pub fn kind(&self) -> LicenceKind {
        match self {
            Validity::Perpetual { .. } => LicenceKind::Perpetual,
            _ => LicenceKind::Temporary,
        }
    }
    pub fn updates_until(&self) -> Option<Ts> {
        match *self {
            Validity::Perpetual { updates_until } => Some(updates_until),
            _ => None,
        }
    }
    /// The expiry that follows from a first activation at `first`.
    pub fn expiry_from(&self, first: Ts) -> Option<Ts> {
        match *self {
            Validity::FirstActivation { days } => Some(first + i64::from(days) * DAY),
            Validity::FixedExpiry { expires_at } => Some(expires_at),
            Validity::Perpetual { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Generated but not yet activated.
    Issued,
    /// Within its validity period.
    Active,
    /// The validity period has ended.
    Expired,
    /// Every use consumed. Machines already activated keep working.
    ActivationLimitReached,
    /// Administratively disabled.
    Revoked,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Issued => "Issued",
            Status::Active => "Active",
            Status::Expired => "Expired",
            Status::ActivationLimitReached => "Activation limit reached",
            Status::Revoked => "Revoked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub at: Ts,
    pub what: String,
    pub detail: String,
}

/// A licence as the server keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Licence {
    pub id: String,
    pub serial_hash: String,
    pub serial_hint: String,
    pub validity: Validity,
    pub owner: Option<String>,
    /// Kept on the server for sign-in links and support; never put in certificates or exports.
    pub email: Option<String>,
    /// Internal note: what the licence was issued for.
    pub purpose: Option<String>,
    pub issued_at: Ts,
    pub first_activated_at: Option<Ts>,
    pub expires_at: Option<Ts>,
    pub max_activations: u32,
    pub activations_used: u32,
    pub revoked_at: Option<Ts>,
    pub revoke_reason: Option<String>,
    /// Paddle transaction that paid for it, if any.
    pub order_ref: Option<String>,
    pub events: Vec<Event>,
}

impl Licence {
    pub fn kind(&self) -> LicenceKind {
        self.validity.kind()
    }

    pub fn status(&self, now: Ts) -> Status {
        if self.revoked_at.is_some() {
            Status::Revoked
        } else if self.expires_at.is_some_and(|e| now >= e) {
            Status::Expired
        } else if self.first_activated_at.is_none() {
            Status::Issued
        } else if self.activations_used >= self.max_activations {
            Status::ActivationLimitReached
        } else {
            Status::Active
        }
    }

    pub fn uses_left(&self) -> u32 {
        self.max_activations.saturating_sub(self.activations_used)
    }
}

/// One activation of one licence on one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activation {
    /// Made by the app and saved before the first attempt, so retries reuse it.
    pub activation_id: String,
    pub licence_id: String,
    pub device_hash: String,
    pub app_version: String,
    pub created_at: Ts,
    /// Whether this activation still counts against the limit (false after an admin void or reset).
    pub counted: bool,
    /// Deactivated by the user. A deactivated activation never comes back; activating that
    /// machine again is a new activation.
    pub ended_at: Option<Ts>,
    /// Cancelled by an admin, with the reason.
    pub voided: Option<(Ts, String)>,
}

impl Activation {
    pub fn is_live(&self) -> bool {
        self.ended_at.is_none() && self.voided.is_none()
    }
}

/// Signed with [`crate::sign::Purpose::Activation`]. Unlocks features on the one device whose
/// hash it carries, until `expires_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationCert {
    pub licence_id: String,
    pub kind: LicenceKind,
    pub owner: Option<String>,
    pub serial_hint: String,
    pub activation_id: String,
    pub device_hash: String,
    pub issued_at: Ts,
    pub first_activated_at: Ts,
    pub expires_at: Option<Ts>,
    pub updates_until: Option<Ts>,
    pub activations_used: u32,
    pub max_activations: u32,
    /// The server's clock when it signed: a trusted floor for the app's clock checks.
    pub signed_at: Ts,
}

/// Signed with [`crate::sign::Purpose::Issuance`]. Proves the licence was issued; carries no
/// device or activation data, so it can't unlock anything anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuanceCert {
    pub licence_id: String,
    pub kind: LicenceKind,
    pub owner: Option<String>,
    pub serial_hint: String,
    pub issued_at: Ts,
    pub first_activated_at: Option<Ts>,
    pub expires_at: Option<Ts>,
    pub updates_until: Option<Ts>,
    pub validity: Validity,
    pub max_activations: u32,
    pub activations_used: u32,
    pub non_renewable: bool,
    pub issuer: String,
    /// When these figures were true.
    pub as_of: Ts,
}

/// Signed with [`crate::sign::Purpose::Status`]. The app removes or locks a licence only on one
/// of these, never on an unsigned error reply, so a server fault, a database restored to the wrong
/// point or a proxy's error page can't take anyone's licence away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusStatement {
    pub licence_id: String,
    pub activation_id: String,
    pub device_hash: String,
    /// `Revoked` or `Deactivated`; the app ignores anything else.
    pub status: Rejection,
    pub signed_at: Ts,
}

/// What the app sends to activate (online, or saved as a file for offline activation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationRequest {
    pub serial: String,
    pub activation_id: String,
    pub device_hash: String,
    pub app_version: String,
}

/// The offline request file: an [`ActivationRequest`] carried to any computer with internet
/// access and uploaded at account.filescape.app/activate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineRequest {
    pub format: String,
    pub request: ActivationRequest,
}

pub const OFFLINE_REQUEST_FORMAT: &str = "filescape-activation-request/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationResponse {
    pub activation: Envelope,
    pub issuance: Envelope,
    pub status: Status,
    /// Whether this request consumed a use (false for retries and revalidations).
    pub consumed_use: bool,
}

/// Why the server said no. None of these consume a use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    BadSerial,
    UnknownSerial,
    Revoked,
    Expired,
    ActivationLimitReached,
    /// The activation ID belongs to another device.
    ActivationIdConflict,
    /// That activation was deactivated or voided; a new activation is needed.
    Deactivated,
    NotFound,
}

impl Rejection {
    pub fn message(self) -> &'static str {
        match self {
            Rejection::BadSerial => "That serial number isn't valid.",
            Rejection::UnknownSerial => "That serial number wasn't issued by Filescape.",
            Rejection::Revoked => "This licence has been revoked.",
            Rejection::Expired => "This licence has expired. Temporary licences can't be renewed or reactivated.",
            Rejection::ActivationLimitReached => "This serial has already been activated the maximum number of times.",
            Rejection::ActivationIdConflict => "This activation belongs to another computer.",
            Rejection::Deactivated => "This activation was ended. Activate again to continue.",
            Rejection::NotFound => "No such licence or activation.",
        }
    }
}
