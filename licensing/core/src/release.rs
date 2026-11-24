//! The newest release, as the licence server signs it, and what it means for this machine.
//!
//! The server answers `GET /v1/release` with `{"release": Envelope}`, signed with
//! [`Purpose::Release`](crate::sign::Purpose::Release) by the same key as the certificates, so the
//! app needs no new key and a manifest can't pass as any other document (or the other way round).
//!
//! `released_at` is the release build's own `FILESCAPE_RELEASE_DATE` (the time of the commit it was
//! built from), which is the date a perpetual licence's `updates_until` is compared with. So the
//! app can say, before anyone downloads anything, whether their licence covers the new version:
//! [`coverage`] runs the same precedence as [`crate::standing::evaluate`] with that date.
//!
//! Nothing here ever locks anything: an update notice that can't be fetched or checked is simply
//! not shown.

use crate::Ts;
use crate::model::LicenceKind;
use crate::standing::{Context, LocalState, Reason, Standing, evaluate};
use serde::{Deserialize, Serialize};

/// Signed with [`crate::sign::Purpose::Release`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseManifest {
    /// Shown to people, e.g. "0.3.0". Not used to decide what's newer: `released_at` does that, so
    /// a build that forgot to bump its version still compares correctly.
    pub version: String,
    /// The release build's `FILESCAPE_RELEASE_DATE` (Unix seconds).
    pub released_at: Ts,
    /// When the server signed this.
    pub signed_at: Ts,
    /// A line or two about what's new.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Release notes page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_url: Option<String>,
    /// Where each platform's download is. Missing ones fall back to [`default_download`].
    #[serde(default)]
    pub downloads: Downloads,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Downloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linux: Option<String>,
}

/// The website's download buttons use these (R2 bucket `filescape-downloads`).
pub fn default_download(os: &str) -> String {
    format!("https://downloads.filescape.app/latest/{os}")
}

/// Manifests signed longer than this before the build's own release date are ignored, so an old
/// one can't be replayed. The server signs a fresh one for every request.
pub const MAX_AGE: Ts = 30 * crate::DAY;

impl ReleaseManifest {
    /// The download link for `os` ("windows", "macos" or "linux"). Only links on Filescape's own
    /// sites are used, even with a valid signature, so the worst a leaked key could do is announce
    /// a wrong version.
    pub fn download_for(&self, os: &str) -> String {
        let given = match os {
            "windows" => self.downloads.windows.as_deref(),
            "macos" => self.downloads.macos.as_deref(),
            "linux" => self.downloads.linux.as_deref(),
            _ => None,
        };
        given.filter(|u| is_ours(u)).map(String::from).unwrap_or_else(|| default_download(os))
    }

    pub fn notes(&self) -> Option<&str> {
        self.notes_url.as_deref().filter(|u| is_ours(u))
    }

    /// Whether this is a newer version than `current` (both "major.minor.patch"; a pre-release
    /// suffix sorts before the plain version). Decided by version, not by date: the date on the
    /// manifest is only for the updates check, where a few hours either way doesn't matter.
    pub fn is_newer_than(&self, current: &str) -> bool {
        match (version_key(&self.version), version_key(current)) {
            (Some(a), Some(b)) => a > b,
            _ => false,
        }
    }

    /// Not an old manifest being replayed.
    pub fn is_fresh(&self, build_date: Ts) -> bool {
        self.signed_at >= build_date - MAX_AGE
    }
}

/// `(major, minor, patch, not a pre-release)`, from "1.2.3", "v1.2" or "1.2.3-beta.1".
pub fn version_key(v: &str) -> Option<(u64, u64, u64, bool)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match v.split_once(['-', '+']) {
        Some((c, rest)) => (c, v.as_bytes()[c.len()] == b'-' && !rest.is_empty()),
        None => (v, false),
    };
    let mut n = core.split('.').map(|p| p.parse::<u64>());
    let major = n.next()?.ok()?;
    let minor = n.next().unwrap_or(Ok(0)).ok()?;
    let patch = n.next().unwrap_or(Ok(0)).ok()?;
    if n.next().is_some() {
        return None;
    }
    Some((major, minor, patch, !pre))
}

/// An https link on filescape.app or one of its subdomains.
pub fn is_ours(u: &str) -> bool {
    let Some(rest) = u.strip_prefix("https://") else { return false };
    if u.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host == "filescape.app" || host.ends_with(".filescape.app")
}

/// What the new release would mean for this machine's licence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// A perpetual licence's updates include it: Pro stays unlocked after updating.
    Covered,
    /// A perpetual licence's updates ended before it: updating would leave only the free
    /// features (this version keeps working as it is).
    NotCovered { updates_until: Ts },
    /// No perpetual licence is involved (free, evaluation or temporary): updating changes nothing
    /// about what's unlocked.
    NotAffected,
}

/// Whether the release dated `released_at` is covered, by running the same rules as the app's
/// own standing with that release date in place of this build's.
pub fn coverage(ctx: &Context, state: &LocalState, released_at: Ts) -> Coverage {
    let now = evaluate(ctx, state);
    let then = evaluate(&Context { build_date: released_at, ..*ctx }, state);
    let lapsed_until = |s: &Standing| match s {
        Standing::Perpetual(c) | Standing::Restricted(Reason::UpdatesLapsed(c)) if c.kind == LicenceKind::Perpetual => c.updates_until,
        _ => None,
    };
    match (&now, &then) {
        (_, Standing::Perpetual(_)) => Coverage::Covered,
        // Licensed by a perpetual licence now (or one that has already lapsed for this build),
        // and not perpetual-licensed in the new release: its updates ended.
        (Standing::Perpetual(_) | Standing::Restricted(Reason::UpdatesLapsed(_)), _) => match lapsed_until(&now) {
            Some(u) => Coverage::NotCovered { updates_until: u },
            None => Coverage::NotAffected,
        },
        _ => Coverage::NotAffected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ActivationCert;
    use crate::sign::{Purpose, Signer, Verifier};
    use crate::{DAY, sign::Envelope};

    const NOW: Ts = 1_800_000_000;

    fn signer() -> Signer {
        let rng = ring::rand::SystemRandom::new();
        Signer::from_pkcs8(&Signer::generate_pkcs8(&rng)).unwrap()
    }

    fn cert(s: &Signer, kind: LicenceKind, updates_until: Option<Ts>, expires_at: Option<Ts>) -> Envelope {
        let c = ActivationCert {
            licence_id: format!("lic_{kind:?}"),
            kind,
            owner: None,
            serial_hint: "FSP-…".into(),
            activation_id: "act_1".into(),
            device_hash: "dev".into(),
            issued_at: NOW - 30 * DAY,
            first_activated_at: NOW - 30 * DAY,
            expires_at,
            updates_until,
            activations_used: 1,
            max_activations: 3,
            signed_at: NOW - DAY,
        };
        s.sign(Purpose::Activation, &c)
    }

    fn check(certs: Vec<Envelope>, s: &Signer, build: Ts, release: Ts, first_launch: Ts) -> Coverage {
        let v = Verifier::new(&[s.public_key()]);
        let ctx = Context { now: NOW, device_hash: "dev", build_date: build, verifier: &v };
        let state = LocalState { first_launch: Some(first_launch), high_water: NOW, certs, revoked: vec![] };
        coverage(&ctx, &state, release)
    }

    #[test]
    fn perpetual_within_updates_is_covered() {
        let s = signer();
        let c = cert(&s, LicenceKind::Perpetual, Some(NOW + 100 * DAY), None);
        assert_eq!(check(vec![c], &s, NOW - 10 * DAY, NOW, NOW - 60 * DAY), Coverage::Covered);
    }

    #[test]
    fn perpetual_past_updates_is_not_covered_but_this_version_still_is() {
        let s = signer();
        let until = NOW - 5 * DAY;
        let c = cert(&s, LicenceKind::Perpetual, Some(until), None);
        assert_eq!(check(vec![c], &s, NOW - 10 * DAY, NOW, NOW - 60 * DAY), Coverage::NotCovered { updates_until: until });
    }

    #[test]
    fn perpetual_already_lapsed_for_this_build_is_still_not_covered() {
        let s = signer();
        let until = NOW - 50 * DAY;
        let c = cert(&s, LicenceKind::Perpetual, Some(until), None);
        assert_eq!(check(vec![c], &s, NOW - 10 * DAY, NOW, NOW - 60 * DAY), Coverage::NotCovered { updates_until: until });
    }

    #[test]
    fn perpetual_without_an_updates_date_is_covered() {
        let s = signer();
        let c = cert(&s, LicenceKind::Perpetual, None, None);
        assert_eq!(check(vec![c], &s, NOW - 10 * DAY, NOW + 900 * DAY, NOW - 60 * DAY), Coverage::Covered);
    }

    #[test]
    fn free_evaluation_and_temporary_are_not_affected() {
        let s = signer();
        // Free (evaluation over).
        assert_eq!(check(vec![], &s, NOW - 10 * DAY, NOW, NOW - 60 * DAY), Coverage::NotAffected);
        // Evaluation.
        assert_eq!(check(vec![], &s, NOW - 10 * DAY, NOW, NOW - DAY), Coverage::NotAffected);
        // Temporary.
        let t = cert(&s, LicenceKind::Temporary, None, Some(NOW + 10 * DAY));
        assert_eq!(check(vec![t], &s, NOW - 10 * DAY, NOW, NOW - 60 * DAY), Coverage::NotAffected);
    }

    #[test]
    fn a_second_perpetual_licence_with_later_updates_covers_it() {
        let s = signer();
        let old = cert(&s, LicenceKind::Perpetual, Some(NOW - 5 * DAY), None);
        let new = cert(&s, LicenceKind::Perpetual, Some(NOW + 300 * DAY), None);
        assert_eq!(check(vec![old, new], &s, NOW - 10 * DAY, NOW, NOW - 60 * DAY), Coverage::Covered);
    }

    #[test]
    fn manifest_is_signed_for_its_purpose_only() {
        let s = signer();
        let v = Verifier::new(&[s.public_key()]);
        let m = ReleaseManifest { version: "0.3.0".into(), released_at: NOW, signed_at: NOW, summary: None, notes_url: None, downloads: Downloads::default() };
        let env = s.sign(Purpose::Release, &m);
        assert_eq!(v.verify::<ReleaseManifest>(&env, Purpose::Release).unwrap(), m);
        assert!(v.verify::<ActivationCert>(&env, Purpose::Activation).is_err());
        let mut tampered = env.clone();
        tampered.payload = crate::sign::b64(br#"{"version":"9.9.9","released_at":1,"signed_at":1}"#);
        assert!(v.verify::<ReleaseManifest>(&tampered, Purpose::Release).is_err());
    }

    #[test]
    fn newer_by_version() {
        let m = |v: &str| ReleaseManifest { version: v.into(), released_at: NOW, signed_at: NOW, summary: None, notes_url: None, downloads: Downloads::default() };
        assert!(m("0.3.0").is_newer_than("0.2.0"));
        assert!(m("v0.2.1").is_newer_than("0.2.0"));
        assert!(m("1.0").is_newer_than("0.9.9"));
        assert!(m("0.10.0").is_newer_than("0.9.0"));
        assert!(m("0.3.0").is_newer_than("0.3.0-beta.2"));
        assert!(!m("0.3.0-beta.1").is_newer_than("0.3.0"));
        assert!(!m("0.2.0").is_newer_than("0.2.0"));
        assert!(!m("0.1.9").is_newer_than("0.2.0"));
        assert!(!m("banana").is_newer_than("0.2.0"));
        assert!(!m("1.2.3.4").is_newer_than("0.2.0"));
        assert!(m("0.3.0").is_fresh(NOW + 29 * DAY));
        assert!(!m("0.3.0").is_fresh(NOW + 31 * DAY));
    }

    #[test]
    fn only_filescape_links_are_used() {
        let mut m = ReleaseManifest { version: "0.3.0".into(), released_at: NOW, signed_at: NOW, summary: None, notes_url: Some("javascript:alert(1)".into()), downloads: Downloads::default() };
        assert_eq!(m.notes(), None);
        assert_eq!(m.download_for("linux"), "https://downloads.filescape.app/latest/linux");
        m.notes_url = Some("https://filescape.app/whats-new".into());
        assert_eq!(m.notes(), Some("https://filescape.app/whats-new"));
        m.downloads.linux = Some("https://filescape.app/#download".into());
        m.downloads.windows = Some("https://evil.example/filescape.app".into());
        m.downloads.macos = Some("https://filescape.app.evil.example/x".into());
        assert_eq!(m.download_for("linux"), "https://filescape.app/#download");
        assert_eq!(m.download_for("windows"), "https://downloads.filescape.app/latest/windows");
        assert_eq!(m.download_for("macos"), "https://downloads.filescape.app/latest/macos");
        m.downloads.macos = Some("https://user@evil.example/".into());
        assert_eq!(m.download_for("macos"), "https://downloads.filescape.app/latest/macos");
    }
}
