//! Proof of ownership: the licence certificate file and the plain-text details.
//!
//! A serial *authorises* (it can activate, up to the limit); a certificate *proves issuance*
//! (it is signed by Filescape, says whose licence it is and when it runs, and can't activate
//! anything). Neither export contains the serial, the device hash, activation IDs, the email
//! address or any private key.

use crate::model::{IssuanceCert, LicenceKind, Validity};
use crate::sign::{Envelope, Purpose, Verifier, VerifyError, b64};
use crate::{ISSUER, Ts};
use serde::{Deserialize, Serialize};

pub const CERTIFICATE_FORMAT: &str = "filescape-licence-certificate/1";
/// File extension for Export Licence Certificate.
pub const CERTIFICATE_EXTENSION: &str = "filescape-licence";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateFile {
    pub format: String,
    pub issuer: String,
    /// The Ed25519 public key that signed it, so anyone can check it without Filescape.
    pub public_key: String,
    pub certificate: Envelope,
    pub how_to_verify: String,
}

/// Export Licence Certificate.
pub fn certificate_file(issuance: &Envelope, verifier: &Verifier) -> Result<String, VerifyError> {
    let _: IssuanceCert = verifier.verify(issuance, Purpose::Issuance)?;
    let key = verifier.public_key(&issuance.key_id).ok_or(VerifyError::UnknownKey)?;
    let file = CertificateFile {
        format: CERTIFICATE_FORMAT.into(),
        issuer: ISSUER.into(),
        public_key: b64(key),
        certificate: issuance.clone(),
        how_to_verify: "Open it in Filescape (Settings > Licensing > Verify certificate), or check the Ed25519 \
            signature in `certificate.signature` over the bytes `filescape-licence/v1/issuance\\n` followed by the \
            base64url-decoded `certificate.payload`, using `public_key`. Filescape's published keys are listed at \
            https://filescape.app/keys."
            .into(),
    };
    Ok(serde_json::to_string_pretty(&file).unwrap())
}

/// Checks a certificate file against the keys this build trusts (never the key inside the file).
pub fn verify_certificate_file(text: &str, verifier: &Verifier) -> Result<IssuanceCert, VerifyError> {
    let file: CertificateFile = serde_json::from_str(text).map_err(|_| VerifyError::Format)?;
    if file.format != CERTIFICATE_FORMAT {
        return Err(VerifyError::Format);
    }
    verifier.verify(&file.certificate, Purpose::Issuance)
}

/// "15 May 2027, 14:00 UTC".
pub fn date_time(ts: Ts) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // Days to civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    format!("{d} {} {y}, {:02}:{:02} UTC", MONTHS[(m - 1) as usize], secs / 3600, secs % 3600 / 60)
}

/// Export Licence Details: plain text for customer support and proof of issuance.
pub fn details_text(c: &IssuanceCert, status: &str) -> String {
    let kind = match c.kind {
        LicenceKind::Temporary => "Temporary (non-renewable)",
        LicenceKind::Perpetual => "Perpetual",
    };
    let mut s = String::new();
    let mut line = |k: &str, v: String| s.push_str(&format!("{k:<22}{v}\n"));
    line("Licence ID:", c.licence_id.clone());
    line("Registered owner:", c.owner.clone().unwrap_or_else(|| "(not assigned)".into()));
    line("Licence type:", kind.into());
    line("Serial number:", format!("ends in {}", c.serial_hint.trim_start_matches('…')));
    line("Issued:", date_time(c.issued_at));
    line("Activated:", c.first_activated_at.map(date_time).unwrap_or_else(|| "not yet".into()));
    match c.validity {
        Validity::FirstActivation { days } => line("Validity:", format!("{days} days from first activation")),
        Validity::FixedExpiry { .. } => line("Validity:", "until a fixed date".into()),
        Validity::Perpetual { .. } => line("Validity:", "does not expire".into()),
    }
    if let Some(e) = c.expires_at {
        line("Expires:", date_time(e));
    }
    if let Some(u) = c.updates_until {
        line("Updates until:", date_time(u));
    }
    line("Activations used:", format!("{} of {}", c.activations_used, c.max_activations));
    line("Status:", status.into());
    line("Issuer:", c.issuer.clone());
    line("Figures as of:", date_time(c.as_of));
    s.push_str("\nThis summary is for support. The signed licence certificate (.filescape-licence) is the\n");
    s.push_str("proof of issuance; it can't be used to activate Filescape.\n");
    s
}
