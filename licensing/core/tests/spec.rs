//! The temporary-licence spec's tests (section 10), numbered as in the spec. Tests 8, 12 and 20
//! also get app-level versions in phase 2 (relaunching the real app, a real transfer, CI on three
//! platforms); here they are covered at the level of the rules.

use filescape_licence::export::{certificate_file, date_time, details_text, verify_certificate_file};
use filescape_licence::features::{Feature, begin};
use filescape_licence::model::*;
use filescape_licence::rules::{MemoryLedger, TemporaryOptions};
use filescape_licence::sign::{Purpose, Signer, Verifier};
use filescape_licence::standing::{Context, LocalState, Reason, Standing, evaluate};
use filescape_licence::*;
use ring::rand::SystemRandom;
use std::sync::Arc;

const ISSUE: Ts = 1_803_891_600; // 1 Mar 2027 09:00 UTC
const FIRST: Ts = 1_804_687_200; // 10 Mar 2027 14:00 UTC
const FIRST_PLUS_30: Ts = 1_807_279_200; // 9 Apr 2027 14:00 UTC
const JUNE_30: Ts = 1_814_313_600; // 30 Jun 2027 00:00 UTC
const BUILD: Ts = 1_798_761_600; // 1 Jan 2027

fn ledger() -> (MemoryLedger, Verifier) {
    let pkcs8 = Signer::generate_pkcs8(&SystemRandom::new());
    let signer = Signer::from_pkcs8(&pkcs8).unwrap();
    let verifier = Verifier::new(&[signer.public_key()]);
    (MemoryLedger::new(signer), verifier)
}

fn temp(l: &MemoryLedger, opts: TemporaryOptions) -> (String, Licence) {
    l.issue_temporary(&opts, ISSUE).unwrap().remove(0)
}

fn req(serial: &str, n: u32) -> ActivationRequest {
    ActivationRequest {
        serial: serial.into(),
        activation_id: aid(serial, n),
        device_hash: device_hash(&format!("machine-{n}")),
        app_version: "2.4.0".into(),
    }
}

/// Activation IDs are random per serial in the app; here, readable.
fn aid(serial: &str, n: u32) -> String {
    format!("act-{n}-{}", &serial[serial.len() - 4..])
}

fn cert(v: &Verifier, r: &ActivationResponse) -> ActivationCert {
    v.verify(&r.activation, Purpose::Activation).unwrap()
}

/// The app's view after activating with `r` on device `n`.
fn app_standing(v: &Verifier, r: &ActivationResponse, n: u32, first_launch: Ts, now: Ts) -> Standing {
    let mut st = LocalState::default();
    st.observe(first_launch);
    st.store(v, r.activation.clone()).unwrap();
    st.observe(now);
    let dev = device_hash(&format!("machine-{n}"));
    evaluate(&Context { now, device_hash: &dev, build_date: BUILD, verifier: v }, &st)
}

#[test]
fn t01_thirty_day_serial_unlocks_every_feature() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    assert_eq!(lic.validity, Validity::FirstActivation { days: 30 });
    assert_eq!(lic.max_activations, 3);
    let r = l.activate(&req(&serial, 1), FIRST).unwrap();
    // First launch long ago, so the evaluation can't be what unlocks things.
    let s = app_standing(&v, &r, 1, FIRST - 100 * DAY, FIRST + DAY);
    assert!(matches!(s, Standing::Temporary { days_left: 29, .. }), "{s:?}");
    for f in Feature::ALL {
        assert!(begin(&s, f).is_some(), "{f:?}");
    }
}

#[test]
fn t02_admin_sets_custom_duration_or_fixed_date() {
    let (l, _) = ledger();
    for days in [7, 14, 60, 90] {
        let (serial, lic) = temp(&l, TemporaryOptions { validity: Validity::FirstActivation { days }, ..Default::default() });
        assert_eq!(lic.expires_at, None, "expiry isn't set until first activation");
        l.activate(&req(&serial, 1), FIRST).unwrap();
        assert_eq!(l.licence(&lic.id).unwrap().expires_at, Some(FIRST + i64::from(days) * DAY));
    }
    let (_, fixed) = temp(&l, TemporaryOptions { validity: Validity::FixedExpiry { expires_at: JUNE_30 }, ..Default::default() });
    assert_eq!(fixed.expires_at, Some(JUNE_30));
    // A batch, with owner and purpose.
    let many = l
        .issue_temporary(&TemporaryOptions { count: 25, owner: Some("Acme Training".into()), purpose: Some("June course".into()), ..Default::default() }, ISSUE)
        .unwrap();
    assert_eq!(many.len(), 25);
    let serials: std::collections::HashSet<_> = many.iter().map(|(s, _)| s.clone()).collect();
    assert_eq!(serials.len(), 25);
    // Nonsense is refused.
    assert!(l.issue_temporary(&TemporaryOptions { validity: Validity::FixedExpiry { expires_at: ISSUE - 1 }, ..Default::default() }, ISSUE).is_err());
    assert!(l.issue_temporary(&TemporaryOptions { validity: Validity::FirstActivation { days: 0 }, ..Default::default() }, ISSUE).is_err());
}

#[test]
fn t03_first_activation_sets_expiry_from_that_moment() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    assert_eq!(lic.status(FIRST), Status::Issued);
    let r = l.activate(&req(&serial, 1), FIRST).unwrap();
    let c = cert(&v, &r);
    assert_eq!(c.first_activated_at, FIRST);
    assert_eq!(c.expires_at, Some(FIRST_PLUS_30));
    assert_eq!(date_time(FIRST_PLUS_30), "9 April 2027, 14:00 UTC");
    assert_eq!(l.licence(&lic.id).unwrap().status(FIRST), Status::Active);
}

#[test]
fn t04_later_activations_share_the_original_expiry() {
    let (l, v) = ledger();
    let (serial, _) = temp(&l, TemporaryOptions::default());
    let a = cert(&v, &l.activate(&req(&serial, 1), FIRST).unwrap());
    let b = cert(&v, &l.activate(&req(&serial, 2), FIRST + 20 * DAY).unwrap());
    let c = cert(&v, &l.activate(&req(&serial, 3), FIRST + 29 * DAY).unwrap());
    assert_eq!(a.expires_at, Some(FIRST_PLUS_30));
    assert_eq!(b.expires_at, Some(FIRST_PLUS_30));
    assert_eq!(c.expires_at, Some(FIRST_PLUS_30));
    assert_eq!(c.first_activated_at, FIRST);
}

#[test]
fn t05_three_new_activations_use_every_use() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    for n in 1..=3 {
        let r = l.activate(&req(&serial, n), FIRST + i64::from(n)).unwrap();
        assert!(r.consumed_use);
        assert_eq!(cert(&v, &r).activations_used, n);
    }
    let lic = l.licence(&lic.id).unwrap();
    assert_eq!(lic.uses_left(), 0);
    assert_eq!(lic.status(FIRST + DAY), Status::ActivationLimitReached);
}

#[test]
fn t06_fourth_new_activation_is_rejected_and_the_first_three_keep_working() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    for n in 1..=3 {
        l.activate(&req(&serial, n), FIRST).unwrap();
    }
    assert_eq!(l.activate(&req(&serial, 4), FIRST + DAY), Err(Rejection::ActivationLimitReached));
    assert_eq!(l.licence(&lic.id).unwrap().activations_used, 3);
    // Limit reached doesn't invalidate the machines already activated.
    let r = l.validate(&aid(&serial, 2), &device_hash("machine-2"), FIRST + 2 * DAY).unwrap();
    assert!(matches!(app_standing(&v, &r, 2, ISSUE - 100 * DAY, FIRST + 2 * DAY), Standing::Temporary { .. }));
}

#[test]
fn t07_failed_activations_consume_nothing() {
    let (l, _) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    // A typo.
    let mut typo = serial.clone().into_bytes();
    let last = typo.len() - 1;
    typo[last] = if typo[last] == b'Z' { b'Y' } else { b'Z' };
    assert_eq!(l.activate(&req(std::str::from_utf8(&typo).unwrap(), 1), FIRST), Err(Rejection::BadSerial));
    // Well formed but never issued.
    let other = serial::generate(LicenceKind::Temporary, &SystemRandom::new());
    assert_eq!(l.activate(&req(&other, 1), FIRST), Err(Rejection::UnknownSerial));
    // An activation ID that belongs to another device.
    l.activate(&req(&serial, 1), FIRST).unwrap();
    let mut stolen = req(&serial, 2);
    stolen.activation_id = aid(&serial, 1);
    assert_eq!(l.activate(&stolen, FIRST), Err(Rejection::ActivationIdConflict));
    assert_eq!(l.licence(&lic.id).unwrap().activations_used, 1);
    // Revoked and expired serials.
    let (s2, l2) = temp(&l, TemporaryOptions::default());
    l.revoke(&l2.id, "test", FIRST).unwrap();
    assert_eq!(l.activate(&req(&s2, 1), FIRST), Err(Rejection::Revoked));
    assert_eq!(l.licence(&l2.id).unwrap().activations_used, 0);
    let (s3, l3) = temp(&l, TemporaryOptions { validity: Validity::FixedExpiry { expires_at: JUNE_30 }, ..Default::default() });
    assert_eq!(l.activate(&req(&s3, 1), JUNE_30), Err(Rejection::Expired));
    assert_eq!(l.licence(&l3.id).unwrap().activations_used, 0);
}

#[test]
fn t08_relaunching_and_reinstalling_consume_nothing() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    let r = l.activate(&req(&serial, 1), FIRST).unwrap();
    // Relaunch: the app evaluates its stored certificate; no server call at all.
    for day in 1..10 {
        assert!(matches!(app_standing(&v, &r, 1, FIRST, FIRST + day * DAY), Standing::Temporary { .. }));
    }
    // Reinstall with settings wiped: a new activation ID, same machine.
    let mut again = req(&serial, 1);
    again.activation_id = "act-after-reinstall".into();
    let r2 = l.activate(&again, FIRST + 5 * DAY).unwrap();
    assert!(!r2.consumed_use);
    assert_eq!(cert(&v, &r2).activation_id, aid(&serial, 1), "the existing activation is handed back");
    assert_eq!(l.licence(&lic.id).unwrap().activations_used, 1);
}

#[test]
fn t09_revalidation_and_retries_consume_nothing() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    l.activate(&req(&serial, 1), FIRST).unwrap();
    for i in 0..10 {
        // A retry after a dropped connection resends the same request.
        let r = l.activate(&req(&serial, 1), FIRST + i * 60).unwrap();
        assert!(!r.consumed_use);
        let r = l.validate(&aid(&serial, 1), &device_hash("machine-1"), FIRST + i * 7 * DAY / 10).unwrap();
        assert_eq!(cert(&v, &r).activations_used, 1);
    }
    assert_eq!(l.licence(&lic.id).unwrap().activations_used, 1);
}

#[test]
fn t10_deactivation_never_gives_a_use_back() {
    let (l, _) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    l.activate(&req(&serial, 1), FIRST).unwrap();
    l.deactivate(&aid(&serial, 1), &device_hash("machine-1"), FIRST + DAY).unwrap();
    assert_eq!(l.licence(&lic.id).unwrap().activations_used, 1);
    // The old activation is gone for good.
    assert_eq!(l.validate(&aid(&serial, 1), &device_hash("machine-1"), FIRST + DAY), Err(Rejection::Deactivated));
    assert_eq!(l.activate(&req(&serial, 1), FIRST + DAY), Err(Rejection::Deactivated));
    // Activating that machine again is a new activation, and consumes a use.
    let mut again = req(&serial, 1);
    again.activation_id = "act-1b".into();
    assert!(l.activate(&again, FIRST + DAY).unwrap().consumed_use);
    assert_eq!(l.licence(&lic.id).unwrap().activations_used, 2);
}

#[test]
fn t11_expiry_locks_premium_features_and_keeps_free_ones() {
    let (l, v) = ledger();
    let (serial, _) = temp(&l, TemporaryOptions::default());
    let r = l.activate(&req(&serial, 1), FIRST).unwrap();
    let s = app_standing(&v, &r, 1, FIRST - 100 * DAY, FIRST_PLUS_30);
    assert!(matches!(&s, Standing::Restricted(Reason::TemporaryExpired(c)) if c.expires_at == Some(FIRST_PLUS_30)), "{s:?}");
    for f in Feature::ALL {
        assert_eq!(begin(&s, f).is_some(), f.is_free(), "{f:?}");
    }
    assert!(begin(&s, Feature::FileBrowser).is_some());
    assert!(begin(&s, Feature::DiskMap).is_some());
    assert!(begin(&s, Feature::RemoteTransfers).is_none());
}

#[test]
fn t12_expiry_does_not_interrupt_operations_already_started() {
    let (l, v) = ledger();
    let (serial, _) = temp(&l, TemporaryOptions::default());
    let r = l.activate(&req(&serial, 1), FIRST).unwrap();
    // A transfer and a batch rename start one minute before expiry...
    let before = app_standing(&v, &r, 1, FIRST - 100 * DAY, FIRST_PLUS_30 - 60);
    let transfer = begin(&before, Feature::RemoteTransfers).expect("allowed before expiry");
    let rename = begin(&before, Feature::BatchRename).expect("allowed before expiry");
    // ...and run for an hour. Each step needs only the permit, never the licence.
    let mut done = 0;
    for minute in 0..60 {
        let _now = FIRST_PLUS_30 - 60 + minute * 60;
        let step = |p: &filescape_licence::features::Permit| p.0;
        assert_eq!(step(&transfer), Feature::RemoteTransfers);
        assert_eq!(step(&rename), Feature::BatchRename);
        done += 1;
    }
    assert_eq!(done, 60);
    // New premium work can't start after expiry.
    let after = app_standing(&v, &r, 1, FIRST - 100 * DAY, FIRST_PLUS_30 + 60);
    assert!(begin(&after, Feature::RemoteTransfers).is_none());
}

#[test]
fn t13_expired_licence_cannot_be_renewed_or_reactivated() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    l.activate(&req(&serial, 1), FIRST).unwrap();
    // Not by the same machine, a new machine, a retry, or a revalidation.
    assert_eq!(l.activate(&req(&serial, 1), FIRST_PLUS_30), Err(Rejection::Expired));
    assert_eq!(l.activate(&req(&serial, 2), FIRST_PLUS_30 + DAY), Err(Rejection::Expired));
    assert_eq!(l.validate(&aid(&serial, 1), &device_hash("machine-1"), FIRST_PLUS_30), Err(Rejection::Expired));
    // Admin corrections never move the expiry.
    l.reset_activations(&lic.id, "support", FIRST_PLUS_30 + DAY).unwrap();
    l.void_activation(&lic.id, &aid(&serial, 1), true, "support", FIRST_PLUS_30 + DAY).unwrap();
    let after = l.licence(&lic.id).unwrap();
    assert_eq!(after.expires_at, Some(FIRST_PLUS_30));
    assert_eq!(after.first_activated_at, Some(FIRST));
    assert_eq!(after.status(FIRST_PLUS_30 + DAY), Status::Expired);
    assert_eq!(l.activate(&req(&serial, 3), FIRST_PLUS_30 + DAY), Err(Rejection::Expired));
    // The issuance certificate says it can't be renewed.
    let (s4, _) = temp(&l, TemporaryOptions::default());
    let c: IssuanceCert = v.verify(&l.activate(&req(&s4, 1), FIRST).unwrap().issuance, Purpose::Issuance).unwrap();
    assert!(c.non_renewable);
}

#[test]
fn t14_exports_reveal_nothing_sensitive() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions { owner: Some("Ada Lovelace".into()), email: Some("ada@example.com".into()), ..Default::default() });
    let r = l.activate(&req(&serial, 1), FIRST).unwrap();
    let file = certificate_file(&r.issuance, &v).unwrap();
    let c: IssuanceCert = v.verify(&r.issuance, Purpose::Issuance).unwrap();
    let text = details_text(&c, "Active");
    let payload = String::from_utf8(data_encoding::BASE64URL_NOPAD.decode(r.issuance.payload.as_bytes()).unwrap()).unwrap();
    let body: String = serial.replace('-', "");
    for out in [&file, &text, &payload] {
        assert!(!out.contains(&serial) && !out.contains(&body), "serial leaked");
        assert!(!out.contains(&device_hash("machine-1")), "device hash leaked");
        assert!(!out.contains(&aid(&serial, 1)), "activation ID leaked");
        assert!(!out.contains("ada@example.com"), "email leaked");
        assert!(!out.to_lowercase().contains("private"), "key material named");
    }
    assert!(text.contains(&lic.id) && text.contains("Ada Lovelace") && text.contains("1 of 3"));
    assert!(text.contains("9 April 2027, 14:00 UTC"));
    // The certificate can't activate or unlock anything.
    let env: CertificateFileShape = serde_json::from_str(&file).unwrap();
    assert!(v.verify::<ActivationCert>(&env.certificate, Purpose::Activation).is_err());
    let mut st = LocalState::default();
    assert!(st.store(&v, env.certificate.clone()).is_err());
}

#[derive(serde::Deserialize)]
struct CertificateFileShape {
    certificate: filescape_licence::sign::Envelope,
}

#[test]
fn t15_certificate_signatures_verify_and_tampering_fails() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    let r = l.activate(&req(&serial, 1), FIRST).unwrap();
    let file = certificate_file(&r.issuance, &v).unwrap();
    assert_eq!(verify_certificate_file(&file, &v).unwrap().licence_id, lic.id);
    // Tampered payload.
    let mut env = r.issuance.clone();
    let mut c: IssuanceCert = v.verify(&env, Purpose::Issuance).unwrap();
    c.max_activations = 99;
    env.payload = data_encoding::BASE64URL_NOPAD.encode(&serde_json::to_vec(&c).unwrap());
    assert_eq!(v.verify::<IssuanceCert>(&env, Purpose::Issuance), Err(filescape_licence::sign::VerifyError::BadSignature));
    // Wrong purpose, even with a valid signature.
    let mut as_activation = r.issuance.clone();
    as_activation.purpose = "activation".into();
    assert!(v.verify::<ActivationCert>(&as_activation, Purpose::Activation).is_err());
    // Signed by someone else's key.
    let (other, _) = ledger();
    let (s2, _) = temp(&other, TemporaryOptions::default());
    let forged = other.activate(&req(&s2, 1), FIRST).unwrap();
    assert_eq!(v.verify::<IssuanceCert>(&forged.issuance, Purpose::Issuance), Err(filescape_licence::sign::VerifyError::UnknownKey));
    // A third party can check it with only the public key in the file.
    let shown: serde_json::Value = serde_json::from_str(&file).unwrap();
    let pk = data_encoding::BASE64URL_NOPAD.decode(shown["public_key"].as_str().unwrap().as_bytes()).unwrap();
    assert!(Verifier::new(&[pk]).verify::<IssuanceCert>(&r.issuance, Purpose::Issuance).is_ok());
    // Activation certificates are bound to their device.
    let a = l.activate(&req(&serial, 1), FIRST).unwrap();
    let mut st = LocalState::default();
    st.observe(FIRST - 100 * DAY);
    st.store(&v, a.activation).unwrap();
    st.observe(FIRST + DAY);
    let other_machine = device_hash("someone-elses-pc");
    let s = evaluate(&Context { now: FIRST + DAY, device_hash: &other_machine, build_date: BUILD, verifier: &v }, &st);
    assert!(!s.is_licensed(), "a copied licence file must not work on another computer");
}

#[test]
fn t16_offline_activation_follows_the_same_global_limit() {
    let (l, v) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    let offline = |n: u32| serde_json::to_string(&OfflineRequest { format: OFFLINE_REQUEST_FORMAT.into(), request: req(&serial, n) }).unwrap();
    l.activate(&req(&serial, 1), FIRST).unwrap();
    let r2 = l.process_offline(&offline(2), FIRST + DAY).unwrap();
    assert_eq!(cert(&v, &r2).expires_at, Some(FIRST_PLUS_30));
    // The same offline file processed twice (carried to two computers) consumes once.
    assert!(!l.process_offline(&offline(2), FIRST + DAY).unwrap().consumed_use);
    l.activate(&req(&serial, 3), FIRST + 2 * DAY).unwrap();
    assert_eq!(l.process_offline(&offline(4), FIRST + 3 * DAY), Err(Rejection::ActivationLimitReached));
    assert_eq!(l.licence(&lic.id).unwrap().activations_used, 3);
}

#[test]
fn t16b_concurrent_activations_never_exceed_the_limit() {
    let (l, _) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions::default());
    let l = Arc::new(l);
    let handles: Vec<_> = (1..=20)
        .map(|n| {
            let (l, serial) = (l.clone(), serial.clone());
            std::thread::spawn(move || l.activate(&req(&serial, n), FIRST).is_ok())
        })
        .collect();
    let ok = handles.into_iter().map(|h| h.join().unwrap()).filter(|&x| x).count();
    assert_eq!(ok, 3);
    assert_eq!(l.licence(&lic.id).unwrap().activations_used, 3);
}

#[test]
fn t17_clock_rollback_does_not_bypass_expiry() {
    let (l, v) = ledger();
    let (serial, _) = temp(&l, TemporaryOptions::default());
    let r = l.activate(&req(&serial, 1), FIRST).unwrap();
    let dev = device_hash("machine-1");
    let mut st = LocalState::default();
    st.observe(FIRST - 100 * DAY);
    st.store(&v, r.activation.clone()).unwrap();
    // The app ran past expiry, then the clock was wound back three days.
    st.observe(FIRST_PLUS_30 + DAY);
    let ctx = |now| Context { now, device_hash: &dev, build_date: BUILD, verifier: &v };
    assert_eq!(evaluate(&ctx(FIRST_PLUS_30 - 2 * DAY), &st), Standing::Restricted(Reason::ClockRollback));
    // Winding back before the activation itself is caught by the server's signing time.
    let mut fresh = LocalState::default();
    fresh.store(&v, r.activation.clone()).unwrap();
    assert_eq!(evaluate(&ctx(FIRST - 3 * DAY), &fresh), Standing::Restricted(Reason::ClockRollback));
    // Small corrections (NTP, a dead battery fixed within minutes) are fine.
    let mut ok = LocalState::default();
    ok.observe(FIRST - 100 * DAY);
    ok.store(&v, r.activation.clone()).unwrap();
    ok.observe(FIRST + 5 * DAY);
    assert!(matches!(evaluate(&ctx(FIRST + 5 * DAY - 30 * 60), &ok), Standing::Temporary { .. }));
    // Deleting the licence file doesn't help: the keychain copy of the high-water mark wins.
    let mut wiped = LocalState::default();
    wiped.store(&v, r.activation.clone()).unwrap();
    wiped.merge_keychain(Some(FIRST - 100 * DAY), FIRST_PLUS_30 + DAY);
    assert_eq!(evaluate(&ctx(FIRST_PLUS_30 - 2 * DAY), &wiped), Standing::Restricted(Reason::ClockRollback));
    // A trusted time from the server clears a lock caused by a clock that had been in the future.
    let mut future = LocalState::default();
    future.observe(FIRST - 100 * DAY);
    future.store(&v, r.activation.clone()).unwrap();
    future.observe(FIRST + 365 * DAY);
    assert_eq!(evaluate(&ctx(FIRST + 2 * DAY), &future), Standing::Restricted(Reason::ClockRollback));
    future.trusted_time(FIRST + 2 * DAY);
    assert!(matches!(evaluate(&ctx(FIRST + 2 * DAY), &future), Standing::Temporary { .. }));
}

#[test]
fn t18_temporary_expiry_does_not_restart_the_evaluation() {
    let (l, v) = ledger();
    let dev = device_hash("machine-1");
    let ctx = |now| Context { now, device_hash: &dev, build_date: BUILD, verifier: &v };
    let day0 = FIRST;
    let mut st = LocalState::default();
    st.observe(day0);
    assert!(matches!(evaluate(&ctx(day0 + DAY), &st), Standing::Evaluation { days_left: 14, .. }));
    // Temporary licence activated on day 10, expires on day 40.
    let (serial, _) = temp(&l, TemporaryOptions::default());
    let r = l.activate(&req(&serial, 1), day0 + 10 * DAY).unwrap();
    st.store(&v, r.activation).unwrap();
    st.observe(day0 + 20 * DAY);
    assert!(matches!(evaluate(&ctx(day0 + 20 * DAY), &st), Standing::Temporary { .. }));
    st.observe(day0 + 41 * DAY);
    assert!(matches!(evaluate(&ctx(day0 + 41 * DAY), &st), Standing::Restricted(Reason::TemporaryExpired(_))));
    // Deleting the licence file: the keychain still has the first launch.
    let mut wiped = LocalState::default();
    wiped.merge_keychain(Some(day0), day0 + 41 * DAY);
    wiped.observe(day0 + 41 * DAY);
    assert_eq!(evaluate(&ctx(day0 + 41 * DAY), &wiped), Standing::Restricted(Reason::EvaluationOver));
}

#[test]
fn t19_perpetual_licences_are_unaffected_and_win() {
    let (l, v) = ledger();
    let dev = device_hash("machine-1");
    let (pserial, plic) = l.issue_perpetual(Some("Rich".into()), None, Some("txn_01".into()), DEFAULT_UPDATE_MONTHS, ISSUE).unwrap();
    assert_eq!(plic.kind(), LicenceKind::Perpetual);
    assert_eq!(plic.max_activations, DEFAULT_PERPETUAL_ACTIVATIONS);
    assert_eq!(plic.max_activations, 10);
    // A resent Paddle webhook doesn't make a second licence.
    assert!(l.issue_perpetual(None, None, Some("txn_01".into()), 12, ISSUE).is_err());
    let (tserial, _) = temp(&l, TemporaryOptions::default());
    let mut st = LocalState::default();
    st.observe(ISSUE);
    st.store(&v, l.activate(&req(&tserial, 1), FIRST).unwrap().activation).unwrap();
    st.store(&v, l.activate(&req(&pserial, 1), FIRST).unwrap().activation).unwrap();
    let build = ISSUE + 30 * DAY;
    let ctx = |now| Context { now, device_hash: &dev, build_date: build, verifier: &v };
    // Perpetual wins while the temporary is still running, and after it expires.
    assert!(matches!(evaluate(&ctx(FIRST + DAY), &st), Standing::Perpetual(_)));
    st.observe(FIRST_PLUS_30 + 400 * DAY);
    assert!(matches!(evaluate(&ctx(FIRST_PLUS_30 + 400 * DAY), &st), Standing::Perpetual(_)));
    // Perpetual licences don't depend on the clock.
    assert!(matches!(evaluate(&ctx(FIRST), &st), Standing::Perpetual(_)));
    // Update entitlement: a build released after it lapses isn't covered.
    let late = Context { now: ISSUE + 500 * DAY, device_hash: &dev, build_date: ISSUE + 400 * DAY, verifier: &v };
    assert!(matches!(evaluate(&late, &st), Standing::Restricted(Reason::TemporaryExpired(_)) | Standing::Restricted(Reason::UpdatesLapsed(_))));
    // Perpetual licences stop at their limit (10 by default, Rich 23 Nov), which the admin can
    // change, and support can reset them.
    l.set_activation_limit(&plic.id, 3, "test", FIRST).unwrap();
    l.activate(&req(&pserial, 2), FIRST).unwrap();
    l.activate(&req(&pserial, 3), FIRST).unwrap();
    assert_eq!(l.activate(&req(&pserial, 4), FIRST), Err(Rejection::ActivationLimitReached));
    l.reset_activations(&plic.id, "new PC, ticket 42", FIRST + DAY).unwrap();
    assert!(l.activate(&req(&pserial, 4), FIRST + DAY).unwrap().consumed_use);
    assert!(l.licence(&plic.id).unwrap().events.iter().any(|e| e.what == "reset" && e.detail.contains("ticket 42")));
}

#[test]
fn t20_rules_do_not_depend_on_the_platform() {
    // Every rule above is pure Rust over UTC seconds, run in CI on Windows, macOS and Linux.
    // Device IDs (the one platform-specific input) are hashed the same way everywhere.
    assert_eq!(device_hash(" abc \n"), device_hash("abc"));
    assert_eq!(device_hash("abc").len(), 64);
}

#[test]
fn admin_tools_history_void_and_search() {
    let (l, _) = ledger();
    let (serial, lic) = temp(&l, TemporaryOptions { owner: Some("Acme Ltd".into()), ..Default::default() });
    l.activate(&req(&serial, 1), FIRST).unwrap();
    l.activate(&req(&serial, 2), FIRST).unwrap();
    // A mistaken activation is voided with its use given back; the expiry stays.
    l.void_activation(&lic.id, &aid(&serial, 2), true, "activated on the wrong PC", FIRST + DAY).unwrap();
    let after = l.licence(&lic.id).unwrap();
    assert_eq!(after.activations_used, 1);
    assert_eq!(after.expires_at, Some(FIRST_PLUS_30));
    assert_eq!(l.validate(&aid(&serial, 2), &device_hash("machine-2"), FIRST + DAY), Err(Rejection::Deactivated));
    let what: Vec<&str> = after.events.iter().map(|e| e.what.as_str()).collect();
    assert_eq!(what, ["issued", "activated", "activated", "voided"]);
    // Search by serial (as typed), licence ID, owner and expiry.
    assert_eq!(l.search(&serial.to_lowercase(), None)[0].id, lic.id);
    assert_eq!(l.search(&lic.id, None)[0].id, lic.id);
    assert_eq!(l.search("acme", None)[0].id, lic.id);
    assert_eq!(l.search("", Some((FIRST_PLUS_30 - DAY, FIRST_PLUS_30 + DAY)))[0].id, lic.id);
    assert!(l.search("nobody", None).is_empty());
    // Revocation reaches activated machines at their next check.
    l.revoke(&lic.id, "chargeback", FIRST + 2 * DAY).unwrap();
    assert_eq!(l.validate(&aid(&serial, 1), &device_hash("machine-1"), FIRST + 2 * DAY), Err(Rejection::Revoked));
}

#[test]
fn perpetual_limits_are_adjustable_and_activations_are_monitored() {
    let (l, _) = ledger();
    let (serial, lic) = l.issue_perpetual(None, None, None, 12, ISSUE).unwrap();
    for n in 1..=7 {
        l.activate(&req(&serial, n), FIRST + i64::from(n) * DAY).unwrap();
    }
    // Seven machines in a week: flagged as possibly shared.
    let report = l.activation_report(FIRST + 8 * DAY);
    assert_eq!(report[0].licence_id, lic.id);
    assert!(report[0].flagged);
    assert_eq!((report[0].activations_used, report[0].max_activations, report[0].recent_machines), (7, 10, 7));
    // Months later the same licence is no longer flagged.
    assert!(!l.activation_report(FIRST + 100 * DAY)[0].flagged);
    // Lowering the limit blocks new machines but keeps the seven working.
    l.set_activation_limit(&lic.id, 5, "sharing", FIRST + 9 * DAY).unwrap();
    assert_eq!(l.activate(&req(&serial, 8), FIRST + 9 * DAY), Err(Rejection::ActivationLimitReached));
    assert!(l.validate(&aid(&serial, 7), &device_hash("machine-7"), FIRST + 9 * DAY).is_ok());
    // New purchases get the new default; temporary licences keep theirs.
    l.set_default_perpetual_limit(20).unwrap();
    assert_eq!(l.issue_perpetual(None, None, None, 12, ISSUE).unwrap().1.max_activations, 20);
    let (_, t) = temp(&l, TemporaryOptions::default());
    assert!(l.set_activation_limit(&t.id, 10, "no", FIRST).is_err());
    assert!(l.set_activation_limit(&lic.id, 0, "no", FIRST).is_err());
    // The ledger survives a save and reload (the app's test-serial mode keeps one on disk).
    let json = l.to_json();
    let pkcs8 = Signer::generate_pkcs8(&SystemRandom::new());
    let again = MemoryLedger::from_json(Signer::from_pkcs8(&pkcs8).unwrap(), &json).unwrap();
    assert_eq!(again.licence(&lic.id).unwrap().max_activations, 5);
    assert_eq!(again.default_perpetual_limit(), 20);
}
