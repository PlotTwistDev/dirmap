//! Filescape licence core.
//!
//! Everything the app and the licensing server must agree on lives here, with no I/O, so that
//! every rule can be tested with a fake clock and compiled into both the app and the Cloudflare
//! Worker:
//!
//! - [`serial`]: the serial number format (typo-checked before anything reaches the server)
//! - [`sign`]: Ed25519 signed envelopes with a purpose, so one kind of document can't pass as another
//! - [`model`]: licences, activations, certificates, requests and responses
//! - [`rules`]: the activation rules, and [`rules::MemoryLedger`], the reference server
//! - [`standing`]: which licence applies on this machine now (precedence, evaluation, clock checks)
//! - [`features`]: what is free and what needs a licence, and the start-only [`features::Permit`]
//! - [`export`]: the licence certificate file and the human-readable details
//! - [`release`]: the signed newest-release manifest, and whether a licence covers that release
//! - [`maintenance`]: the maintenance ledger (purchase, renewals, complimentary grants) and its end date
//!
//! Times are Unix seconds, UTC.

pub mod export;
pub mod features;
pub mod maintenance;
pub mod model;
pub mod release;
pub mod rules;
pub mod serial;
pub mod sign;
pub mod standing;

/// Unix seconds, UTC.
pub type Ts = i64;
pub const DAY: Ts = 86_400;
pub const HOUR: Ts = 3_600;

/// Default length of a temporary licence.
pub const DEFAULT_TEMPORARY_DAYS: u32 = 30;
/// Activation limit of a temporary licence (the spec: three uses, ever).
pub const DEFAULT_MAX_ACTIVATIONS: u32 = 3;
/// Default number of computers a perpetual licence can have active at once (Rich, 24 Nov 2026:
/// three active computers; deactivating one frees its seat). Adjustable from the admin site.
pub const DEFAULT_PERPETUAL_ACTIVATIONS: u32 = 3;
/// A perpetual licence's first self-service release in this many days frees its seat at once;
/// each further release in the window keeps the seat taken until the window passes, so a serial
/// can't be rotated round many computers.
pub const RELEASE_COOLDOWN_DAYS: i64 = 7;
/// The admin Activations page flags a licence with more new machines than this in
/// [`SHARING_WINDOW_DAYS`] as possibly shared.
pub const SHARING_FLAG_MACHINES: usize = 5;
pub const SHARING_WINDOW_DAYS: i64 = 30;
/// Length of the evaluation, counted from the first launch on a machine.
pub const EVALUATION_DAYS: u32 = 15;
/// How far behind the latest time already seen the clock may be before time-limited licences lock.
pub const ROLLBACK_TOLERANCE: Ts = 2 * HOUR;
/// Default update entitlement of a perpetual licence.
pub const DEFAULT_UPDATE_MONTHS: u32 = 12;

/// Shown as the issuer on certificates and exports.
pub const ISSUER: &str = "Filescape (filescape.app)";

pub(crate) fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    for p in parts {
        ctx.update(p);
    }
    ctx.finish().as_ref().try_into().unwrap()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// A stable, salted hash of the machine's own ID (Windows MachineGuid, macOS IOPlatformUUID,
/// Linux /etc/machine-id). Only this hash ever leaves the machine.
pub fn device_hash(machine_id: &str) -> String {
    hex(&sha256(&[b"filescape-device/v1\0", machine_id.trim().as_bytes()]))
}
