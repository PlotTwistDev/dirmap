//! The licence server against SQLite (which D1 is) and a fake network, through the same
//! `handle` the Worker calls.

use filescape_licence::model::*;
use filescape_licence::sign::{Purpose, Signer, Verifier};
use filescape_licence::{DAY, device_hash};
use filescape_licence_server::db::{Row, Stmt, V};
use filescape_licence_server::{AccessConfig, Config, ContactConfig, Ctx, Db, Hosts, MailConfig, Net, PaddleConfig, Req, Resp, handle, paddle};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

const T0: i64 = 1_795_392_000; // 23 Nov 2026
const SCHEMA: &[&str] = &[include_str!("../migrations/0001_init.sql"), include_str!("../migrations/0002_seats_and_account.sql"), include_str!("../migrations/0003_contact_messages.sql"), include_str!("../migrations/0004_maintenance.sql"), include_str!("../migrations/0005_renewals.sql")];

/// Gives way once, so two requests run with `join` interleave like concurrent Workers.
struct YieldOnce(bool);
impl Future for YieldOnce {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

struct Sqlite(rusqlite::Connection);

fn params(args: &[V]) -> Vec<rusqlite::types::Value> {
    args.iter()
        .map(|v| match v {
            V::Null => rusqlite::types::Value::Null,
            V::Int(i) => rusqlite::types::Value::Integer(*i),
            V::Text(s) => rusqlite::types::Value::Text(s.clone()),
        })
        .collect()
}

impl Db for Sqlite {
    async fn all(&self, sql: &str, args: Vec<V>) -> Result<Vec<Row>, String> {
        YieldOnce(false).await;
        let mut st = self.0.prepare(sql).map_err(|e| e.to_string())?;
        let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let rows = st
            .query_map(rusqlite::params_from_iter(params(&args)), |r| {
                let mut m = Row::new();
                for (i, n) in names.iter().enumerate() {
                    let v = match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                        rusqlite::types::ValueRef::Integer(x) => x.into(),
                        rusqlite::types::ValueRef::Real(x) => x.into(),
                        rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned().into(),
                        rusqlite::types::ValueRef::Blob(_) => serde_json::Value::Null,
                    };
                    m.insert(n.clone(), v);
                }
                Ok(m)
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    async fn batch(&self, stmts: Vec<Stmt>) -> Result<Vec<usize>, String> {
        YieldOnce(false).await;
        self.0.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for (sql, args) in &stmts {
            match self.0.execute(sql, rusqlite::params_from_iter(params(args))) {
                Ok(n) => out.push(n),
                Err(e) => {
                    self.0.execute_batch("ROLLBACK").ok();
                    return Err(e.to_string());
                }
            }
        }
        self.0.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        Ok(out)
    }
}

/// Records what was sent; answers Paddle's customer lookups, Resend and Access keys.
#[derive(Default)]
struct FakeNet {
    sent: RefCell<Vec<(String, String, Option<String>)>>,
    resend_fails: Cell<bool>,
    lookup_fails: Cell<bool>,
    access_keys: RefCell<String>,
}

impl Net for FakeNet {
    async fn send(&self, method: &str, url: &str, _headers: &[(&str, String)], body: Option<String>) -> Result<(u16, String), String> {
        self.sent.borrow_mut().push((method.into(), url.into(), body.clone()));
        if url.starts_with("https://api.resend.com") {
            return Ok(if self.resend_fails.get() { (500, "down".into()) } else { (200, "{\"id\":\"m1\"}".into()) });
        }
        if url.contains("/customers/") {
            if self.lookup_fails.get() {
                return Ok((401, "expired key".into()));
            }
            return Ok((200, r#"{"data":{"id":"ctm_1","name":"Ada Buyer","email":"Ada@Example.com"}}"#.into()));
        }
        if url.ends_with("/cdn-cgi/access/certs") {
            return Ok((200, self.access_keys.borrow().clone()));
        }
        Ok((404, String::new()))
    }
}

impl FakeNet {
    fn emails(&self) -> Vec<String> {
        self.sent.borrow().iter().filter(|(_, u, _)| u.contains("resend")).map(|(_, _, b)| b.clone().unwrap_or_default()).collect()
    }
}

struct World {
    db: Sqlite,
    net: FakeNet,
    cfg: Config,
    now: Cell<i64>,
}

const WEBHOOK_SECRET: &str = "pdl_ntfset_test_secret";

fn world() -> World {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    for m in SCHEMA {
        conn.execute_batch(m).unwrap();
    }
    let pkcs8 = Signer::generate_pkcs8(&ring::rand::SystemRandom::new());
    World {
        db: Sqlite(conn),
        net: FakeNet::default(),
        cfg: Config {
            signer: Signer::from_pkcs8(&pkcs8).unwrap(),
            serial_key: [9u8; 32],
            paddle: Some(PaddleConfig {
                webhook_secret: WEBHOOK_SECRET.into(),
                api_key: Some("pdl_test_key".into()),
                api_base: "https://sandbox-api.paddle.com".into(),
                price_ids: vec!["pri_filescape".into()],
                update_months: 12,
                client_token: Some("test_client_token".into()),
                sandbox: true,
            }),
            mail: Some(MailConfig { resend_key: "re_test".into(), from: "Filescape <licences@filescape.app>".into(), reply_to: None }),
            access: None,
            dev: true,
            hosts: Hosts::default(),
            contact: ContactConfig { notify_to: Some("owner@example.net".into()), turnstile_secret: None },
        },
        now: Cell::new(T0),
    }
}

impl World {
    fn cx(&self) -> Ctx<'_, Sqlite, FakeNet> {
        Ctx::new(&self.db, &self.net, &self.cfg, self.now.get())
    }

    fn call(&self, method: &str, host: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Resp {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let req = Req {
            method: method.into(),
            host: host.into(),
            path: path.into(),
            query: query.into(),
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            body: body.as_bytes().to_vec(),
        };
        pollster::block_on(handle(&self.cx(), &req))
    }

    fn api(&self, path: &str, body: &serde_json::Value) -> Resp {
        self.call("POST", "api.filescape.app", path, &[("content-type", "application/json")], &body.to_string())
    }

    fn admin(&self, path: &str, form: &str) -> Resp {
        self.call("POST", "localhost", &format!("/admin{path}"), &[("origin", "http://localhost:8787")], form)
    }

    fn temp_serial(&self, days: u32, max: u32) -> String {
        let r = self.admin("/issue", &format!("mode=days&days={days}&max={max}&count=1&owner=Test+Owner"));
        assert_eq!(r.status, 200, "{}", r.body);
        let i = r.body.find("FST-").expect("serial shown");
        r.body[i..i + 27].to_string()
    }

    fn activate(&self, serial: &str, machine: &str, act: &str) -> Resp {
        self.api("/v1/activate", &serde_json::json!({ "serial": serial, "activation_id": act, "device_hash": device_hash(machine), "app_version": "2.3.0" }))
    }

    fn webhook(&self, body: &serde_json::Value) -> Resp {
        let b = body.to_string();
        let sig = paddle::sign(WEBHOOK_SECRET, b.as_bytes(), self.now.get());
        self.call("POST", "api.filescape.app", "/v1/paddle", &[("paddle-signature", &sig)], &b)
    }

    fn verifier(&self) -> Verifier {
        Verifier::new(&[self.cfg.signer.public_key()])
    }
}

fn ok(r: &Resp) -> ActivationResponse {
    assert_eq!(r.status, 200, "{}", r.body);
    serde_json::from_str(&r.body).unwrap()
}

fn rejection(r: &Resp) -> Rejection {
    assert!(r.status >= 400, "expected a refusal, got {}: {}", r.status, r.body);
    #[derive(serde::Deserialize)]
    struct E {
        error: Rejection,
    }
    serde_json::from_str::<E>(&r.body).unwrap().error
}

#[test]
fn activation_follows_the_three_use_rule_and_signs_for_the_app() {
    let w = world();
    let serial = w.temp_serial(30, 3);
    // Computer A: one use, signed certificate the app accepts.
    let a = ok(&w.activate(&serial, "machine-a", "act-a-000001"));
    assert!(a.consumed_use);
    let cert: ActivationCert = w.verifier().verify(&a.activation, Purpose::Activation).unwrap();
    assert_eq!((cert.activations_used, cert.max_activations), (1, 3));
    assert_eq!(cert.device_hash, device_hash("machine-a"));
    assert_eq!(cert.expires_at, Some(T0 + 30 * DAY));
    // Relaunch / retry with the same ID, or a new ID from the same machine: nothing consumed.
    assert!(!ok(&w.activate(&serial, "machine-a", "act-a-000001")).consumed_use);
    assert!(!ok(&w.activate(&serial.to_lowercase().replace('-', " "), "machine-a", "act-a-000002")).consumed_use);
    // Deactivate A: the use isn't given back.
    w.now.set(T0 + DAY);
    let r = w.api("/v1/deactivate", &serde_json::json!({ "activation_id": "act-a-000001", "device_hash": device_hash("machine-a") }));
    assert_eq!(r.status, 200, "{}", r.body);
    let b = ok(&w.activate(&serial, "machine-b", "act-b-000001"));
    let cb: ActivationCert = w.verifier().verify(&b.activation, Purpose::Activation).unwrap();
    assert_eq!(cb.activations_used, 2);
    // Expiry was fixed by the first activation.
    assert_eq!(cb.expires_at, Some(T0 + 30 * DAY));
    ok(&w.activate(&serial, "machine-c", "act-c-000001"));
    assert_eq!(rejection(&w.activate(&serial, "machine-d", "act-d-000001")), Rejection::ActivationLimitReached);
    // A's old activation is gone; activating A again would be a new (refused) use.
    assert_eq!(rejection(&w.activate(&serial, "machine-a", "act-a-000003")), Rejection::ActivationLimitReached);
    // Mistakes never reach the database.
    assert_eq!(rejection(&w.activate("FST-AAAAA-AAAAA-AAAAA-AAAAA", "machine-e", "act-e-000001")), Rejection::BadSerial);
    let rows = pollster::block_on(w.cx().licence_by_serial(&filescape_licence::serial::normalize(&serial).unwrap().1)).unwrap().unwrap();
    assert_eq!(rows.lic.activations_used, 3);
}

#[test]
fn weekly_check_and_expiry() {
    let w = world();
    let serial = w.temp_serial(30, 3);
    ok(&w.activate(&serial, "m1", "act-m1-00001"));
    let check = |id: &str, m: &str| w.api("/v1/validate", &serde_json::json!({ "activation_id": id, "device_hash": device_hash(m) }));
    w.now.set(T0 + 7 * DAY);
    let v = ok(&check("act-m1-00001", "m1"));
    assert!(!v.consumed_use);
    let c: ActivationCert = w.verifier().verify(&v.activation, Purpose::Activation).unwrap();
    assert_eq!(c.signed_at, T0 + 7 * DAY, "the check also gives the app a trusted time");
    assert_eq!(rejection(&check("act-m1-00001", "someone-else")), Rejection::ActivationIdConflict);
    assert_eq!(rejection(&check("act-nope-0001", "m1")), Rejection::NotFound);
    w.now.set(T0 + 30 * DAY);
    assert_eq!(rejection(&check("act-m1-00001", "m1")), Rejection::Expired);
    assert_eq!(rejection(&w.activate(&serial, "m2", "act-m2-00001")), Rejection::Expired);
}

#[test]
fn two_computers_racing_for_the_last_use_get_one_between_them() {
    let w = world();
    let serial = w.temp_serial(30, 2);
    ok(&w.activate(&serial, "m1", "act-m1-00001"));
    let req = |m: &str, id: &str| ActivationRequest { serial: serial.clone(), activation_id: id.into(), device_hash: device_hash(m), app_version: "2.3.0".into(), os: Some("macOS".into()) };
    let (r1, r2) = (req("m2", "act-m2-00001"), req("m3", "act-m3-00001"));
    let cx = w.cx();
    let (a, b) = pollster::block_on(futures::future::join(cx.activate(&r1), cx.activate(&r2)));
    let wins = [a.is_ok(), b.is_ok()].iter().filter(|x| **x).count();
    assert_eq!(wins, 1, "exactly one gets the last use");
    let lost = if a.is_ok() { b } else { a };
    assert!(matches!(lost, Err(filescape_licence_server::store::Fail::No(Rejection::ActivationLimitReached))), "{lost:?}");
    let lic = pollster::block_on(cx.licence_by_serial(&filescape_licence::serial::normalize(&serial).unwrap().1)).unwrap().unwrap();
    assert_eq!(lic.lic.activations_used, 2);
    assert_eq!(pollster::block_on(cx.activations_of(&lic.lic.id)).unwrap().len(), 2);
}

#[test]
fn fixed_date_licences_end_on_their_date_whenever_activated() {
    let w = world();
    let r = w.admin("/issue", "mode=fixed&until=2026-12-01&max=3&count=2&purpose=trade+show");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body.matches("FST-").count(), 2, "both serials shown");
    let i = r.body.find("FST-").unwrap();
    let serial = &r.body[i..i + 27];
    w.now.set(T0 + 3 * DAY);
    let a = ok(&w.activate(serial, "m1", "act-m1-00001"));
    let c: ActivationCert = w.verifier().verify(&a.activation, Purpose::Activation).unwrap();
    assert_eq!(c.expires_at, Some(1_796_083_200)); // 1 Dec 2026, midnight UTC
    // A date in the past is refused.
    let r = w.admin("/issue", "mode=fixed&until=2026-01-01&max=3&count=1");
    assert!(r.body.contains("must be in the future"), "{}", r.body);
}

#[test]
fn admin_reset_void_revoke_and_limits() {
    let w = world();
    let serial = w.temp_serial(30, 3);
    for (m, id) in [("m1", "act-m1-00001"), ("m2", "act-m2-00001"), ("m3", "act-m3-00001")] {
        ok(&w.activate(&serial, m, id));
    }
    let cx = w.cx();
    let id = pollster::block_on(cx.licence_by_serial(&filescape_licence::serial::normalize(&serial).unwrap().1)).unwrap().unwrap().lic.id;
    // Reset needs a reason, then gives every use back; machines keep working.
    let r = w.admin(&format!("/licence/{id}/reset"), "reason=");
    assert!(r.header("location").unwrap().contains("Give%20a%20reason"));
    w.admin(&format!("/licence/{id}/reset"), "reason=new+computers");
    ok(&w.activate(&serial, "m4", "act-m4-00001"));
    ok(&w.api("/v1/validate", &serde_json::json!({ "activation_id": "act-m1-00001", "device_hash": device_hash("m1") })));
    // Void with the use given back.
    w.admin(&format!("/licence/{id}/void"), "activation=act-m4-00001&reason=mistake&give_back=1");
    let l = pollster::block_on(cx.licence(&id)).unwrap().unwrap();
    assert_eq!(l.lic.activations_used, 0);
    assert_eq!(rejection(&w.api("/v1/validate", &serde_json::json!({ "activation_id": "act-m4-00001", "device_hash": device_hash("m4") }))), Rejection::Deactivated);
    // A temporary licence's limit can't be changed.
    let r = w.admin(&format!("/licence/{id}/limit"), "max=20&reason=x");
    assert!(r.header("location").unwrap().contains("Only%20perpetual"));
    // Revoke stops the weekly check; reinstate brings it back.
    w.admin(&format!("/licence/{id}/revoke"), "reason=shared+online");
    assert_eq!(rejection(&w.api("/v1/validate", &serde_json::json!({ "activation_id": "act-m1-00001", "device_hash": device_hash("m1") }))), Rejection::Revoked);
    w.admin(&format!("/licence/{id}/reinstate"), "reason=mistake");
    ok(&w.api("/v1/validate", &serde_json::json!({ "activation_id": "act-m1-00001", "device_hash": device_hash("m1") })));
    // The page shows it all, with who did it.
    let page = w.call("GET", "localhost", &format!("/admin/licence/{id}"), &[], "");
    for s in ["reset", "voided", "revoked", "reinstated", "new computers", "dev"] {
        assert!(page.body.contains(s), "history shows {s}");
    }
    // Show serial is recorded.
    let r = w.admin(&format!("/licence/{id}/reveal"), "reason=support");
    assert!(r.body.contains(&serial));
    assert!(w.call("GET", "localhost", &format!("/admin/licence/{id}"), &[], "").body.contains("serial shown"));
}

fn purchase(txn: &str, qty: u32) -> serde_json::Value {
    serde_json::json!({
        "event_id": format!("evt_{txn}"),
        "event_type": "transaction.completed",
        "occurred_at": "2026-11-23T10:00:00Z",
        "data": { "id": txn, "status": "completed", "customer_id": "ctm_1",
                  "items": [ { "price": { "id": "pri_filescape" }, "quantity": qty }, { "price": { "id": "pri_other" }, "quantity": 1 } ] }
    })
}

#[test]
fn paddle_purchase_issues_one_emailed_perpetual_licence() {
    let w = world();
    let r = w.webhook(&purchase("txn_01", 1));
    assert_eq!(r.status, 200, "{}", r.body);
    let mails = w.net.emails();
    assert_eq!(mails.len(), 1);
    let m: serde_json::Value = serde_json::from_str(&mails[0]).unwrap();
    assert_eq!(m["to"][0], "ada@example.com");
    let text = m["text"].as_str().unwrap();
    let i = text.find("FSP-").unwrap();
    let serial = &text[i..i + 27];
    assert!(text.contains("active on up to 3 computers at once"), "{text}");
    // A resent notification (same event, or same transaction in a new event) changes nothing.
    assert_eq!(w.webhook(&purchase("txn_01", 1)).status, 200);
    let mut again = purchase("txn_01", 1);
    again["event_id"] = "evt_other".into();
    assert_eq!(w.webhook(&again).status, 200);
    assert_eq!(w.net.emails().len(), 1, "not emailed twice");
    let cx = w.cx();
    assert_eq!(pollster::block_on(cx.list("", Default::default(), 100)).unwrap().len(), 1);
    // The serial works, on 3 computers at once.
    let a = ok(&w.activate(serial, "m1", "act-m1-00001"));
    let c: ActivationCert = w.verifier().verify(&a.activation, Purpose::Activation).unwrap();
    assert_eq!((c.kind, c.max_activations, c.owner.as_deref()), (LicenceKind::Perpetual, 3, Some("Ada Buyer")));
    assert_eq!(c.expires_at, None);
}

#[test]
fn paddle_signature_quantity_and_email_retry() {
    let w = world();
    // Wrong secret, or stale: refused.
    let b = purchase("txn_02", 2).to_string();
    let bad = paddle::sign("wrong", b.as_bytes(), T0);
    assert_eq!(w.call("POST", "api.filescape.app", "/v1/paddle", &[("paddle-signature", &bad)], &b).status, 401);
    let old = paddle::sign(WEBHOOK_SECRET, b.as_bytes(), T0 - 3600);
    assert_eq!(w.call("POST", "api.filescape.app", "/v1/paddle", &[("paddle-signature", &old)], &b).status, 401);
    // Email down: licences are made, Paddle is asked to retry.
    w.net.resend_fails.set(true);
    assert_eq!(w.webhook(&purchase("txn_02", 2)).status, 500);
    let cx = w.cx();
    assert_eq!(pollster::block_on(cx.list("", filescape_licence_server::store::ListFilter::NotEmailed, 100)).unwrap().len(), 2);
    // The retry emails the same two licences.
    w.net.resend_fails.set(false);
    assert_eq!(w.webhook(&purchase("txn_02", 2)).status, 200);
    assert_eq!(pollster::block_on(cx.list("", Default::default(), 100)).unwrap().len(), 2, "two units, two licences, none extra");
    assert_eq!(pollster::block_on(cx.list("", filescape_licence_server::store::ListFilter::NotEmailed, 100)).unwrap().len(), 0);
    // Other products sold through the same account don't issue anything (and are logged).
    let mut other = purchase("txn_03", 1);
    other["data"]["items"] = serde_json::json!([{ "price": { "id": "pri_other" }, "quantity": 1 }]);
    let r = w.webhook(&other);
    assert_eq!(r.status, 200);
    assert!(r.body.contains("not a Filescape purchase"), "{}", r.body);
    assert_eq!(pollster::block_on(cx.list("", Default::default(), 100)).unwrap().len(), 2);
    assert!(pollster::block_on(cx.db.first("SELECT outcome FROM webhooks WHERE event_id = 'evt_txn_03'", vec![])).unwrap().is_some(), "logged");
}

#[test]
fn refunds_are_flagged_for_the_admin() {
    let w = world();
    w.webhook(&purchase("txn_04", 1));
    let r = w.webhook(&serde_json::json!({ "event_id": "evt_adj", "event_type": "adjustment.created",
        "data": { "id": "adj_1", "action": "refund", "status": "pending_approval", "transaction_id": "txn_04" } }));
    assert!(r.body.contains("flagged 1"), "{}", r.body);
    let cx = w.cx();
    let rows = pollster::block_on(cx.list("", filescape_licence_server::store::ListFilter::Refunds, 100)).unwrap();
    assert_eq!(rows[0].refund_flag.as_deref(), Some("refund pending approval"));
    // A refund of the other product on the same Paddle account is logged and ignored.
    let r = w.webhook(&serde_json::json!({ "event_id": "evt_adj2", "event_type": "adjustment.created",
        "data": { "id": "adj_2", "action": "refund", "status": "approved", "transaction_id": "txn_other_product" } }));
    assert_eq!(r.status, 200);
    assert!(r.body.contains("not a Filescape purchase"), "{}", r.body);
    assert!(pollster::block_on(cx.db.first("SELECT outcome FROM webhooks WHERE event_id = 'evt_adj2'", vec![])).unwrap().is_some(), "logged");
    // Searching by transaction finds it.
    assert_eq!(pollster::block_on(cx.list("TXN_04", Default::default(), 100)).unwrap().len(), 1);
}

#[test]
fn perpetual_default_and_per_licence_limits_from_the_admin() {
    let w = world();
    w.admin("/settings", "perpetual_limit=25");
    w.webhook(&purchase("txn_05", 1));
    let cx = w.cx();
    let l = pollster::block_on(cx.licence_by_order("txn_05")).unwrap().unwrap();
    assert_eq!(l.lic.max_activations, 25);
    w.admin(&format!("/licence/{}/limit", l.lic.id), "max=40&reason=studio+licence");
    assert_eq!(pollster::block_on(cx.licence(&l.lic.id)).unwrap().unwrap().lic.max_activations, 40);
    assert!(w.admin("/settings", "perpetual_limit=0").header("location").unwrap().contains("between%201"));
}

#[test]
fn activations_page_flags_possible_sharing() {
    let w = world();
    w.webhook(&purchase("txn_06", 1));
    let text = serde_json::from_str::<serde_json::Value>(&w.net.emails()[0]).unwrap()["text"].as_str().unwrap().to_string();
    let i = text.find("FSP-").unwrap();
    let serial = text[i..i + 27].to_string();
    let id = pollster::block_on(w.cx().licence_by_serial(&filescape_licence::serial::normalize(&serial).unwrap().1)).unwrap().unwrap().lic.id;
    w.admin(&format!("/licence/{id}/limit"), "max=10&reason=office");
    for n in 0..6 {
        ok(&w.activate(&serial, &format!("pc-{n}"), &format!("act-pc-{n:05}")));
    }
    let page = w.call("GET", "localhost", "/admin/activations", &[], "");
    assert!(page.body.contains("possibly shared"), "{}", page.body);
}

#[test]
fn customers_get_serials_again_by_email_at_most_every_ten_minutes() {
    let w = world();
    w.webhook(&purchase("txn_07", 1));
    let before = w.net.emails().len();
    let ask = || w.call("POST", "account.filescape.app", "/find", &[], "email=ADA%40example.com");
    assert!(ask().body.contains("on their way"));
    assert_eq!(w.net.emails().len(), before + 1);
    assert!(w.net.emails().last().unwrap().contains("FSP-"));
    ask();
    assert_eq!(w.net.emails().len(), before + 1, "rate limited");
    // An unknown address gets the same answer and no email.
    assert!(w.call("POST", "account.filescape.app", "/find", &[], "email=nobody%40example.com").body.contains("on their way"));
    assert_eq!(w.net.emails().len(), before + 1);
}

#[test]
fn offline_activation_page_returns_a_response_file() {
    let w = world();
    let serial = w.temp_serial(30, 3);
    let file = serde_json::json!({ "format": OFFLINE_REQUEST_FORMAT, "request": { "serial": serial, "activation_id": "act-off-00001", "device_hash": device_hash("offline-pc"), "app_version": "2.3.0" } });
    let body = format!("request={}", filescape_licence_server::web::encode(&file.to_string()));
    let r = w.call("POST", "account.filescape.app", "/activate", &[], &body);
    assert!(r.header("content-disposition").unwrap().contains("filescape-activation-response.json"));
    let resp: ActivationResponse = serde_json::from_str(&r.body).unwrap();
    assert!(resp.consumed_use);
    let r = w.call("POST", "account.filescape.app", "/activate", &[], "request=nonsense");
    assert!(r.body.contains("a Filescape activation request file"));
}

#[test]
fn admin_needs_cloudflare_access_and_same_site_posts() {
    let mut w = world();
    // Posts from elsewhere are refused even in development.
    let r = w.call("POST", "localhost", "/admin/settings", &[("origin", "https://evil.example")], "perpetual_limit=1");
    assert_eq!(r.status, 403);
    let r = w.call("POST", "localhost", "/admin/settings", &[("origin", "http://localhost:8787"), ("sec-fetch-site", "cross-site")], "perpetual_limit=1");
    assert_eq!(r.status, 403);
    let r = w.call("POST", "localhost", "/admin/settings", &[("origin", "null")], "perpetual_limit=1");
    assert_eq!(r.status, 403);
    let r = w.call("POST", "localhost", "/admin/settings", &[("origin", "null"), ("sec-fetch-site", "same-origin")], "perpetual_limit=1");
    assert_ne!(r.status, 403, "{}", r.body);
    // Production: no token, no entry.
    w.cfg.dev = false;
    assert_eq!(w.call("GET", "admin.filescape.app", "/", &[], "").status, 403);
    w.cfg.access = Some(AccessConfig { team_domain: "filescape.cloudflareaccess.com".into(), audience: "aud123".into() });
    assert_eq!(w.call("GET", "admin.filescape.app", "/", &[], "").status, 403);
    // A real RS256 token from the team's key gets in.
    let kp = ring::signature::RsaKeyPair::from_der(include_bytes!("access-test-key.der")).unwrap();
    let pubk = ring::rsa::PublicKeyComponents::<Vec<u8>>::from(kp.public());
    let enc = |b: &[u8]| data_encoding::BASE64URL_NOPAD.encode(b);
    *w.net.access_keys.borrow_mut() = serde_json::json!({ "keys": [ { "kid": "k1", "kty": "RSA", "alg": "RS256", "n": enc(&pubk.n), "e": enc(&pubk.e) } ] }).to_string();
    let token = |aud: &str, exp: i64| {
        let h = enc(br#"{"alg":"RS256","kid":"k1","typ":"JWT"}"#);
        let p = enc(serde_json::json!({ "aud": [aud], "email": "rich@filescape.app", "exp": exp, "iss": "https://filescape.cloudflareaccess.com" }).to_string().as_bytes());
        let msg = format!("{h}.{p}");
        let mut sig = vec![0; kp.public().modulus_len()];
        kp.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), msg.as_bytes(), &mut sig).unwrap();
        format!("{msg}.{}", enc(&sig))
    };
    let good = token("aud123", T0 + 3600);
    let r = w.call("GET", "admin.filescape.app", "/", &[("cf-access-jwt-assertion", &good)], "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("rich@filescape.app"));
    for bad in [token("other-app", T0 + 3600), token("aud123", T0 - 1), format!("{}x", &good[..good.len() - 2])] {
        assert_eq!(w.call("GET", "admin.filescape.app", "/", &[("cf-access-jwt-assertion", &bad)], "").status, 403);
    }
    // The dev path prefixes don't exist in production.
    assert_ne!(w.call("GET", "api.filescape.app", "/admin/", &[], "").status, 200);
}

#[test]
fn health_and_unknown_routes() {
    let w = world();
    let r = w.call("GET", "api.filescape.app", "/v1/health", &[], "");
    assert!(r.body.contains(w.cfg.signer.key_id()));
    assert_eq!(w.call("GET", "api.filescape.app", "/v1/activate", &[], "").status, 405);
    assert_eq!(w.call("GET", "api.filescape.app", "/nope", &[], "").status, 404);
    assert_eq!(w.call("GET", "account.filescape.app", "/thanks", &[], "").status, 200);
    assert_eq!(rejection(&w.call("POST", "api.filescape.app", "/v1/activate", &[], "not json")), Rejection::BadSerial);
}

/// The signed status statement in an error reply, if any.
fn status_of(w: &World, r: &Resp) -> Option<filescape_licence::model::StatusStatement> {
    let v: serde_json::Value = serde_json::from_str(&r.body).ok()?;
    let env: filescape_licence::sign::Envelope = serde_json::from_value(v.get("status")?.clone()).ok()?;
    w.verifier().verify(&env, Purpose::Status).ok()
}

#[test]
fn only_revoked_and_deactivated_come_with_a_signed_statement() {
    let w = world();
    let serial = w.temp_serial(30, 3);
    ok(&w.activate(&serial, "m1", "act-m1-00001"));
    ok(&w.activate(&serial, "m2", "act-m2-00001"));
    let check = |id: &str, m: &str| w.api("/v1/validate", &serde_json::json!({ "activation_id": id, "device_hash": device_hash(m) }));
    // Unknown activations and other devices get a bare error, which the app ignores.
    assert!(status_of(&w, &check("act-nope-0001", "m1")).is_none());
    assert!(status_of(&w, &check("act-m1-00001", "someone-else")).is_none());
    // Deactivated: signed, for that activation and device.
    w.api("/v1/deactivate", &serde_json::json!({ "activation_id": "act-m2-00001", "device_hash": device_hash("m2") }));
    let st = status_of(&w, &check("act-m2-00001", "m2")).expect("signed");
    assert_eq!((st.status, st.activation_id.as_str(), st.device_hash), (Rejection::Deactivated, "act-m2-00001", device_hash("m2")));
    // Revoked: signed too.
    let cx = w.cx();
    let id = pollster::block_on(cx.licence_by_serial(&filescape_licence::serial::normalize(&serial).unwrap().1)).unwrap().unwrap().lic.id;
    w.admin(&format!("/licence/{id}/revoke"), "reason=refund");
    let st = status_of(&w, &check("act-m1-00001", "m1")).expect("signed");
    assert_eq!((st.status, st.licence_id), (Rejection::Revoked, id));
    // Expiry isn't a reason to take anything away: no statement.
    w.now.set(T0 + 31 * DAY);
    w.admin(&format!("/licence/{}/reinstate", st_licence(&w, &serial)), "reason=x");
    assert!(status_of(&w, &check("act-m1-00001", "m1")).is_none());
}

fn st_licence(w: &World, serial: &str) -> String {
    pollster::block_on(w.cx().licence_by_serial(&filescape_licence::serial::normalize(serial).unwrap().1)).unwrap().unwrap().lic.id
}

#[test]
fn a_failed_buyer_lookup_still_issues_the_licence_and_a_retry_emails_it() {
    let w = world();
    w.net.lookup_fails.set(true);
    let r = w.webhook(&purchase("txn_lookup", 1));
    assert_eq!(r.status, 500, "Paddle should retry: {}", r.body);
    let cx = w.cx();
    let row = pollster::block_on(cx.licence_by_order("txn_lookup")).unwrap().expect("issued before the lookup");
    assert!(row.lic.email.is_none() && row.emailed_at.is_none());
    assert_eq!(pollster::block_on(cx.list("", filescape_licence_server::store::ListFilter::NotEmailed, 100)).unwrap().len(), 1);
    // Paddle's retry: same licence, now with the buyer, emailed once.
    w.net.lookup_fails.set(false);
    let r = w.webhook(&purchase("txn_lookup", 1));
    assert_eq!(r.status, 200, "{}", r.body);
    let again = pollster::block_on(cx.licence_by_order("txn_lookup")).unwrap().unwrap();
    assert_eq!(again.lic.id, row.lic.id);
    assert_eq!(again.lic.email.as_deref(), Some("ada@example.com"));
    assert_eq!(again.lic.owner.as_deref(), Some("Ada Buyer"));
    assert!(again.emailed_at.is_some());
    assert_eq!(w.net.emails().len(), 1);
}

#[test]
fn approved_refunds_and_chargebacks_revoke_and_a_reversal_reinstates() {
    let w = world();
    let adj = |evt: &str, action: &str, status: &str, txn: &str, kind: &str| {
        w.webhook(&serde_json::json!({ "event_id": evt, "event_type": "adjustment.updated",
            "data": { "id": format!("adj_{evt}"), "action": action, "status": status, "type": kind, "transaction_id": txn } }))
    };
    let cx = w.cx();
    let revoked = |txn: &str| pollster::block_on(cx.licence_by_order(txn)).unwrap().unwrap().lic.revoked_at.is_some();
    w.webhook(&purchase("txn_r1", 1));
    assert!(adj("e1", "refund", "pending_approval", "txn_r1", "full").body.contains("flagged"));
    assert!(!revoked("txn_r1"));
    assert!(adj("e2", "refund", "approved", "txn_r1", "partial").body.contains("flagged"));
    assert!(!revoked("txn_r1"), "a partial refund only flags");
    assert!(adj("e3", "refund", "approved", "txn_r1", "full").body.contains("revoked"));
    assert!(revoked("txn_r1"));
    // A chargeback, then its reversal.
    w.webhook(&purchase("txn_r2", 1));
    assert!(adj("e4", "chargeback_warning", "approved", "txn_r2", "full").body.contains("flagged"));
    assert!(!revoked("txn_r2"));
    adj("e5", "chargeback", "approved", "txn_r2", "full");
    assert!(revoked("txn_r2"));
    assert!(adj("e6", "chargeback_reverse", "approved", "txn_r2", "full").body.contains("reinstated"));
    assert!(!revoked("txn_r2"));
    // A reversal never undoes a revoke made for another reason.
    adj("e7", "chargeback_reverse", "approved", "txn_r1", "full");
    assert!(revoked("txn_r1"));
}

#[test]
fn perpetual_seats_free_up_when_a_computer_is_released() {
    let w = world();
    w.webhook(&purchase("txn_10", 1));
    let text = serde_json::from_str::<serde_json::Value>(&w.net.emails()[0]).unwrap()["text"].as_str().unwrap().to_string();
    let i = text.find("FSP-").unwrap();
    let serial = text[i..i + 27].to_string();
    for n in 1..=3 {
        let a = ok(&w.activate(&serial, &format!("pc-{n}"), &format!("act-pc-{n:05}")));
        let c: ActivationCert = w.verifier().verify(&a.activation, Purpose::Activation).unwrap();
        assert_eq!((c.activations_used, c.max_activations), (n, 3));
    }
    assert_eq!(rejection(&w.activate(&serial, "pc-4", "act-pc-00004")), Rejection::ActivationLimitReached);
    // Deactivating in the app frees the seat.
    w.now.set(T0 + DAY);
    let r = w.api("/v1/deactivate", &serde_json::json!({ "activation_id": "act-pc-00001", "device_hash": device_hash("pc-1") }));
    assert_eq!(r.status, 200, "{}", r.body);
    ok(&w.activate(&serial, "pc-4", "act-pc-00004"));
    // A second release the same week keeps the seat for 7 days.
    let r = w.api("/v1/deactivate", &serde_json::json!({ "activation_id": "act-pc-00002", "device_hash": device_hash("pc-2") }));
    assert_eq!(r.status, 200);
    assert_eq!(rejection(&w.activate(&serial, "pc-5", "act-pc-00005")), Rejection::ActivationLimitReached);
    w.now.set(T0 + 8 * DAY + 1);
    let a = ok(&w.activate(&serial, "pc-5", "act-pc-00005"));
    let c: ActivationCert = w.verifier().verify(&a.activation, Purpose::Activation).unwrap();
    assert_eq!(c.activations_used, 3);
    // Two racing for the last seat after a release: one wins.
    let r = w.api("/v1/deactivate", &serde_json::json!({ "activation_id": "act-pc-00003", "device_hash": device_hash("pc-3") }));
    assert_eq!(r.status, 200);
    let req = |m: &str, id: &str| ActivationRequest { serial: serial.clone(), activation_id: id.into(), device_hash: device_hash(m), app_version: "2.3.0".into(), os: None };
    let cx = w.cx();
    let (a, b) = pollster::block_on(futures::future::join(cx.activate(&req("pc-6", "act-pc-00006")), cx.activate(&req("pc-7", "act-pc-00007"))));
    assert_eq!([a.is_ok(), b.is_ok()].iter().filter(|x| **x).count(), 1);
}

fn session_cookie(r: &Resp) -> String {
    let c = r.header("set-cookie").expect("cookie set");
    assert!(c.contains("HttpOnly") && c.contains("SameSite=Lax"), "{c}");
    c.split(';').next().unwrap().to_string()
}

#[test]
fn customers_sign_in_with_an_emailed_link_and_release_computers() {
    let w = world();
    w.webhook(&purchase("txn_11", 1));
    let text = serde_json::from_str::<serde_json::Value>(&w.net.emails()[0]).unwrap()["text"].as_str().unwrap().to_string();
    let i = text.find("FSP-").unwrap();
    let serial = text[i..i + 27].to_string();
    let act = |m: &str, id: &str, os: &str| {
        w.api("/v1/activate", &serde_json::json!({ "serial": serial, "activation_id": id, "device_hash": device_hash(m), "app_version": "2.4.0", "os": os }))
    };
    ok(&act("mac", "act-mac-0001", "macOS"));
    ok(&act("win", "act-win-0001", "Windows"));
    ok(&act("old", "act-old-0001", "Plan 9"));
    let acc = |method: &str, path: &str, cookie: &str, body: &str| w.call(method, "account.filescape.app", path, &[("cookie", cookie), ("origin", "https://account.filescape.app")], body);
    // Unknown addresses get the same answer and no email.
    let before = w.net.emails().len();
    assert!(acc("POST", "/login", "", "email=nobody%40example.com").body.contains("sign-in link is on its way"));
    assert_eq!(w.net.emails().len(), before);
    // The buyer gets a link.
    assert!(acc("POST", "/login", "", "email=ada%40example.com").body.contains("sign-in link is on its way"));
    let mail = w.net.emails().last().unwrap().clone();
    let m: serde_json::Value = serde_json::from_str(&mail).unwrap();
    let body = m["text"].as_str().unwrap();
    let i = body.find("/signin?t=").unwrap() + "/signin?t=".len();
    let t: String = body[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    // Opening the link doesn't spend it; pressing Sign in does, once.
    let page = acc("GET", &format!("/signin?t={t}"), "", "");
    assert!(page.body.contains("Sign in"));
    let r = acc("POST", "/signin", "", &format!("t={t}"));
    assert_eq!(r.status, 303, "{}", r.body);
    let cookie = session_cookie(&r);
    assert!(acc("POST", "/signin", "", &format!("t={t}")).body.contains("expired or was already used"));
    // Signed in: the licence, its computers, 3 of 3 active. Unknown OS names aren't shown.
    let home = acc("GET", "/", &cookie, "");
    assert!(home.body.contains("3 of 3 computers active"), "{}", home.body);
    assert!(home.body.contains("macOS") && home.body.contains("Windows") && !home.body.contains("Plan 9"));
    assert!(!home.body.contains(&serial), "the full serial isn't shown");
    // Another site can't post the form.
    let evil = w.call("POST", "account.filescape.app", "/release", &[("cookie", &cookie), ("origin", "https://evil.example")], "activation=act-mac-0001");
    assert_eq!(evil.status, 403);
    // Browsers that hide the origin send "null": allowed only when Sec-Fetch-Site vouches for it.
    let hidden = w.call("POST", "account.filescape.app", "/release", &[("cookie", &cookie), ("origin", "null")], "activation=act-none");
    assert_eq!(hidden.status, 403);
    let cross = w.call("POST", "account.filescape.app", "/release", &[("cookie", &cookie), ("origin", "null"), ("sec-fetch-site", "cross-site")], "activation=act-none");
    assert_eq!(cross.status, 403);
    let same = w.call("POST", "account.filescape.app", "/release", &[("cookie", &cookie), ("origin", "null"), ("sec-fetch-site", "same-origin")], "activation=act-none");
    assert_ne!(same.status, 403, "{}", same.body);
    // Release the Mac: the seat frees and the Mac's next check finds it ended.
    let r = acc("POST", "/release", &cookie, "activation=act-mac-0001");
    assert_eq!(r.status, 303);
    assert!(acc("GET", "/", &cookie, "").body.contains("2 of 3 computers active"));
    assert_eq!(rejection(&w.api("/v1/validate", &serde_json::json!({ "activation_id": "act-mac-0001", "device_hash": device_hash("mac") }))), Rejection::Deactivated);
    ok(&act("new", "act-new-0001", "Linux"));
    // Without the cookie (or after signing out) nothing can be released.
    assert_eq!(acc("POST", "/release", "", "activation=act-win-0001").status, 303);
    acc("POST", "/signout", &cookie, "");
    assert!(acc("GET", "/", &cookie, "").body.contains("Email me a sign-in link"));
    assert_eq!(acc("POST", "/release", &cookie, "activation=act-win-0001").status, 303);
    ok(&w.api("/v1/validate", &serde_json::json!({ "activation_id": "act-win-0001", "device_hash": device_hash("win") })));
    // Links expire after 15 minutes.
    w.now.set(T0 + 2 * DAY);
    acc("POST", "/login", "", "email=ada%40example.com");
    let m: serde_json::Value = serde_json::from_str(w.net.emails().last().unwrap()).unwrap();
    let body = m["text"].as_str().unwrap();
    let i = body.find("/signin?t=").unwrap() + "/signin?t=".len();
    let t: String = body[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    w.now.set(T0 + 2 * DAY + 16 * 60);
    assert!(acc("POST", "/signin", "", &format!("t={t}")).body.contains("expired"));
}

#[test]
fn releases_are_announced_signed_and_only_with_filescape_links() {
    use filescape_licence::release::ReleaseManifest;
    let w = world();
    // Nothing announced yet: the app shows no notice.
    let r = w.call("GET", "api.filescape.app", "/v1/release", &[], "");
    assert_eq!(r.status, 404);
    // Bad input is refused with a reason.
    for (form, why) in [
        ("version=soon&released_at=2026-11-20", "look%20like"),
        ("version=0.3.0&released_at=yesterday", "release%20date"),
        ("version=0.3.0&released_at=2030-01-01", "future"),
        ("version=0.3.0&released_at=2026-11-20&notes_url=https%3A%2F%2Fevil.example%2F", "filescape.app"),
    ] {
        let r = w.admin("/releases", form);
        assert!(r.header("location").unwrap().contains(why), "{form}: {:?}", r.header("location"));
    }
    assert_eq!(w.call("GET", "api.filescape.app", "/v1/release", &[], "").status, 404);
    // Announce one.
    let r = w.admin("/releases", "version=v0.3.0&released_at=2026-11-20&summary=Faster+scans&notes_url=https%3A%2F%2Ffilescape.app%2Fwhats-new&linux=https%3A%2F%2Ffilescape.app%2F%23download");
    assert!(r.header("location").unwrap().contains("now%20announced"));
    w.now.set(T0 + DAY);
    let r = w.call("GET", "api.filescape.app", "/v1/release", &[], "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("cache-control"), Some("public, max-age=300"));
    let env: filescape_licence::sign::Envelope = serde_json::from_value(serde_json::from_str::<serde_json::Value>(&r.body).unwrap()["release"].clone()).unwrap();
    let m: ReleaseManifest = w.verifier().verify(&env, Purpose::Release).unwrap();
    assert_eq!(m.version, "0.3.0");
    assert_eq!(m.released_at, T0 - 3 * DAY);
    assert_eq!(m.signed_at, T0 + DAY, "signed fresh for each request");
    assert_eq!(m.summary.as_deref(), Some("Faster scans"));
    assert_eq!(m.notes(), Some("https://filescape.app/whats-new"));
    assert_eq!(m.download_for("linux"), "https://filescape.app/#download");
    assert_eq!(m.download_for("windows"), "https://downloads.filescape.app/latest/windows");
    // It can't pass as anything that unlocks features.
    assert!(w.verifier().verify::<ActivationCert>(&env, Purpose::Activation).is_err());
    // The admin page shows it, and it can be withdrawn.
    assert!(w.call("GET", "localhost", "/admin/releases", &[], "").body.contains("0.3.0"));
    w.admin("/releases", "withdraw=1");
    assert_eq!(w.call("GET", "api.filescape.app", "/v1/release", &[], "").status, 404);
    // Exact Unix seconds work too.
    w.admin("/releases", &format!("version=0.3.1&released_at={}", T0 + 3600));
    let r = w.call("GET", "api.filescape.app", "/v1/release", &[], "");
    let env: filescape_licence::sign::Envelope = serde_json::from_value(serde_json::from_str::<serde_json::Value>(&r.body).unwrap()["release"].clone()).unwrap();
    assert_eq!(w.verifier().verify::<ReleaseManifest>(&env, Purpose::Release).unwrap().released_at, T0 + 3600);
    assert_eq!(w.call("POST", "api.filescape.app", "/v1/release", &[], "").status, 405);
}

// ---------- contact form ----------

impl World {
    fn contact(&self, form: &str, json: bool, ip: &str) -> Resp {
        let mut h = vec![("origin", "https://filescape.app"), ("content-type", "application/x-www-form-urlencoded"), ("cf-connecting-ip", ip)];
        if json {
            h.push(("accept", "application/json"));
        }
        self.call("POST", "api.filescape.app", "/v1/contact", &h, form)
    }

    fn message_ids(&self) -> Vec<String> {
        pollster::block_on(self.db.all("SELECT id FROM messages ORDER BY created_at", vec![])).unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
    }
}

const MSG: &str = "name=Ada+Buyer&email=Ada%40Example.com&topic=Bug+report&message=The+map+is+blank+on+my+NAS.&elapsed=9000";

#[test]
fn contact_form_stores_notifies_and_is_answered_from_the_admin_site() {
    let w = world();
    let r = w.contact(MSG, true, "203.0.113.9");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("access-control-allow-origin"), Some("https://filescape.app"));
    let ids = w.message_ids();
    assert_eq!(ids.len(), 1);
    let id = &ids[0];

    // The owner is told, with a link to the admin site and no Reply-To that would leak their address.
    let mails = w.net.emails();
    assert_eq!(mails.len(), 1);
    let n: serde_json::Value = serde_json::from_str(&mails[0]).unwrap();
    assert_eq!(n["to"][0], "owner@example.net");
    assert!(n.get("reply_to").is_none());
    assert!(n["text"].as_str().unwrap().contains(&format!("https://admin.filescape.app/messages/{id}")));
    assert!(n["text"].as_str().unwrap().contains("The map is blank on my NAS."));

    // The admin lists it as new, shows it, and replies.
    let list = w.call("GET", "localhost", "/admin/messages", &[], "");
    assert!(list.body.contains("ada@example.com") && list.body.contains("New"), "{}", list.body);
    assert!(w.call("GET", "localhost", "/admin/", &[], "").body.contains("1 new message"));
    let page = w.call("GET", "localhost", &format!("/admin/messages/{id}"), &[], "");
    assert!(page.body.contains("The map is blank on my NAS."));
    let r = w.admin(&format!("/messages/{id}/reply"), "reply=Thanks%2C+fixed+in+0.3.");
    assert_eq!(r.status, 303);
    let mails = w.net.emails();
    let reply: serde_json::Value = serde_json::from_str(mails.last().unwrap()).unwrap();
    assert_eq!(reply["to"][0], "ada@example.com");
    let text = reply["text"].as_str().unwrap();
    assert!(text.contains("Thanks, fixed in 0.3.") && text.contains("> The map is blank on my NAS.") && text.contains("https://filescape.app/contact"));
    assert!(!text.contains("owner@example.net"));
    let page = w.call("GET", "localhost", &format!("/admin/messages/{id}"), &[], "");
    assert!(page.body.contains("Answered") && page.body.contains("Thanks, fixed in 0.3."));

    // Close hides it from the open list; delete removes it and its replies.
    w.admin(&format!("/messages/{id}/close"), "");
    assert!(!w.call("GET", "localhost", "/admin/messages", &[], "").body.contains("ada@example.com"));
    assert!(w.call("GET", "localhost", "/admin/messages?f=all", &[], "").body.contains("ada@example.com"));
    w.admin(&format!("/messages/{id}/delete"), "");
    assert!(w.message_ids().is_empty());
    let left = pollster::block_on(w.db.all("SELECT count(*) AS n FROM message_replies", vec![])).unwrap();
    assert_eq!(left[0]["n"], 0);
}

#[test]
fn contact_form_refuses_bad_input_other_sites_and_floods() {
    let w = world();
    // Without the page's script the browser is sent back to the page with the outcome.
    let r = w.contact("email=nope&message=Hello+there+friend", false, "203.0.113.1");
    assert_eq!(r.status, 303);
    assert_eq!(r.header("location"), Some("https://filescape.app/contact?error=email"));
    let r = w.contact("email=a%40b.co&message=short", true, "203.0.113.1");
    assert_eq!(r.status, 400);
    assert!(r.body.contains("\"message\""));
    let r = w.contact("email=a%40b.co&message=A+long+enough+message.", false, "203.0.113.1");
    assert_eq!(r.header("location"), Some("https://filescape.app/contact?sent=1"));

    // Another site can't post it.
    let r = w.call("POST", "api.filescape.app", "/v1/contact", &[("origin", "https://evil.example")], MSG);
    assert_eq!(r.status, 403);

    // Honeypot and too-fast posts look sent but aren't kept.
    assert_eq!(w.contact(&format!("{MSG}&website=http%3A%2F%2Fspam"), true, "203.0.113.2").status, 200);
    assert_eq!(w.contact("email=b%40b.co&message=A+long+enough+message.&elapsed=400", true, "203.0.113.2").status, 200);
    assert_eq!(w.message_ids().len(), 1);

    // Five an hour from one address or one IP.
    for i in 0..5 {
        let r = w.contact(&format!("email=c{i}%40b.co&message=A+long+enough+message."), true, "198.51.100.7");
        assert_eq!(r.status, 200, "{i}: {}", r.body);
    }
    let r = w.contact("email=z%40b.co&message=A+long+enough+message.", true, "198.51.100.7");
    assert_eq!(r.status, 429);
    w.now.set(w.now.get() + 3601);
    assert_eq!(w.contact("email=z%40b.co&message=A+long+enough+message.", true, "198.51.100.7").status, 200);

    // Only a keyed hash of the IP is stored.
    let rows = pollster::block_on(w.db.all("SELECT ip_hash, topic FROM messages", vec![])).unwrap();
    assert!(rows.iter().all(|r| !r["ip_hash"].as_str().unwrap().contains("198.51")));
    assert!(rows.iter().all(|r| r["topic"] == "Other"));
}

#[test]
fn contact_form_checks_turnstile_when_configured() {
    let mut w = world();
    w.cfg.contact.turnstile_secret = Some("0x4AAA-secret".into());
    let r = w.contact(MSG, true, "203.0.113.9");
    assert_eq!(r.status, 400);
    assert!(r.body.contains("challenge"));
    // The fake network answers 404 to siteverify, which is an error, not a pass.
    let r = w.contact(&format!("{MSG}&cf-turnstile-response=tok"), true, "203.0.113.9");
    assert_eq!(r.status, 500);
    assert!(w.message_ids().is_empty());
}

// ---- Maintenance ----

fn bought(w: &World, txn: &str) -> (String, String) {
    assert_eq!(w.webhook(&purchase(txn, 1)).status, 200);
    let m: serde_json::Value = serde_json::from_str(w.net.emails().last().unwrap()).unwrap();
    let text = m["text"].as_str().unwrap();
    let i = text.find("FSP-").unwrap();
    let serial = text[i..i + 27].to_string();
    let id = pollster::block_on(w.cx().licence_by_order(txn)).unwrap().unwrap().lic.id;
    (serial, id)
}

fn until_of(w: &World, id: &str) -> i64 {
    let r = pollster::block_on(w.cx().licence(id)).unwrap().unwrap();
    let u = r.lic.validity.updates_until().unwrap();
    let col = w.db.0.query_row("SELECT maintenance_until FROM licences WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(u, col, "the cached column follows the certificate date");
    u
}

fn renew(w: &World, id: &str, txn: &str) -> filescape_licence_server::maintenance::Outcome {
    use filescape_licence::maintenance::Grant;
    use filescape_licence_server::maintenance::NewGrant;
    let g = NewGrant { grant: Grant::renewal(w.now.get(), 12), txn: Some(txn), amount: Some("2.99"), currency: Some("USD"), actor: "paddle", reason: "" };
    pollster::block_on(w.cx().add_grant(id, &g, Some(730))).unwrap()
}

#[test]
fn a_purchase_records_twelve_calendar_months_of_maintenance() {
    let w = world();
    let (_, id) = bought(&w, "txn_m1");
    assert_eq!(until_of(&w, &id), filescape_licence::maintenance::add_months(T0, 12));
    let g = pollster::block_on(w.cx().grants_of(&id)).unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].grant.kind, filescape_licence::maintenance::GrantKind::Purchase);
    assert_eq!(g[0].after_until, Some(until_of(&w, &id)));
}

#[test]
fn renewals_extend_the_same_licence_and_reach_installed_apps_at_the_weekly_check() {
    use filescape_licence::maintenance::add_months;
    use filescape_licence_server::maintenance::Outcome;
    let w = world();
    let (serial, id) = bought(&w, "txn_m2");
    let first = ok(&w.activate(&serial, "m1", "act-m1-00001"));
    let c: ActivationCert = w.verifier().verify(&first.activation, Purpose::Activation).unwrap();
    let end = add_months(T0, 12);
    assert_eq!(c.updates_until, Some(end));

    // Renew before expiry: +12 months on the end date.
    w.now.set(T0 + 100 * DAY);
    assert_eq!(renew(&w, &id, "txn_r1"), Outcome::Applied { before: end, after: add_months(end, 12) });
    // The same payment again (resent, or a second delivery) changes nothing.
    assert_eq!(renew(&w, &id, "txn_r1"), Outcome::AlreadyDone);
    // Stacked while active.
    assert_eq!(renew(&w, &id, "txn_r2"), Outcome::Applied { before: add_months(end, 12), after: add_months(end, 24) });
    // More than 2 years ahead: refused.
    assert!(matches!(renew(&w, &id, "txn_r3"), Outcome::Refused(filescape_licence::maintenance::RenewRefusal::TooFarAhead)));

    // Same serial, same activation: the weekly check returns the new date.
    let v = ok(&w.api("/v1/validate", &serde_json::json!({ "activation_id": "act-m1-00001", "device_hash": device_hash("m1") })));
    let c: ActivationCert = w.verifier().verify(&v.activation, Purpose::Activation).unwrap();
    assert_eq!((c.updates_until, c.activation_id.as_str(), c.activations_used), (Some(add_months(end, 24)), "act-m1-00001", 1));
    let lic = pollster::block_on(w.cx().licence(&id)).unwrap().unwrap();
    assert_eq!(lic.lic.serial_hint, filescape_licence::serial::hint(&serial));
    assert_eq!(pollster::block_on(w.cx().activations_of(&id)).unwrap().len(), 1, "devices untouched");
}

#[test]
fn renewing_after_a_gap_starts_from_the_payment_and_on_the_last_day_still_extends() {
    use filescape_licence::maintenance::add_months;
    use filescape_licence_server::maintenance::Outcome;
    let w = world();
    let (_, id) = bought(&w, "txn_m3");
    let end = add_months(T0, 12);
    // On the expiry date, a few hours after the end time.
    w.now.set(end + 3 * 3600);
    assert_eq!(renew(&w, &id, "txn_g1"), Outcome::Applied { before: end, after: add_months(end, 12) });
    // Two years after that ended: a new period from today, nothing owed for the gap.
    let later = add_months(end, 12) + 2 * 365 * DAY;
    w.now.set(later);
    assert_eq!(renew(&w, &id, "txn_g2"), Outcome::Applied { before: add_months(end, 12), after: add_months(later, 12) });
}

#[test]
fn temporary_and_revoked_licences_cannot_be_renewed() {
    use filescape_licence::maintenance::RenewRefusal;
    use filescape_licence_server::maintenance::Outcome;
    let w = world();
    let tmp = w.temp_serial(30, 3);
    let tid = st_licence(&w, &tmp);
    assert_eq!(renew(&w, &tid, "txn_t1"), Outcome::Refused(RenewRefusal::Temporary));
    let (_, id) = bought(&w, "txn_m4");
    w.admin(&format!("/licence/{id}/revoke"), "reason=refunded");
    assert_eq!(renew(&w, &id, "txn_t2"), Outcome::Refused(RenewRefusal::Revoked));
    assert!(pollster::block_on(w.cx().grant_by_txn("txn_t2")).unwrap().is_none());
}

#[test]
fn a_refunded_renewal_comes_out_of_the_ledger_and_never_the_purchase() {
    use filescape_licence::maintenance::add_months;
    use filescape_licence_server::maintenance::Outcome;
    let w = world();
    let (_, id) = bought(&w, "txn_m5");
    let end = add_months(T0, 12);
    w.now.set(T0 + 10 * DAY);
    renew(&w, &id, "txn_a");
    renew(&w, &id, "txn_b");
    assert_eq!(until_of(&w, &id), add_months(end, 24));
    let (lid, o) = pollster::block_on(w.cx().reverse_txn("txn_a", "paddle", "refund")).unwrap().unwrap();
    assert_eq!(lid, id);
    assert_eq!(o, Outcome::Applied { before: add_months(end, 24), after: add_months(end, 12) });
    assert_eq!(pollster::block_on(w.cx().reverse_txn("txn_a", "paddle", "refund")).unwrap().unwrap().1, Outcome::AlreadyDone);
    // An admin can't reverse the purchase or a paid renewal.
    let grants = pollster::block_on(w.cx().grants_of(&id)).unwrap();
    for g in &grants {
        if g.reversed_at.is_none() {
            let r = w.admin(&format!("/licence/{id}/reverse-grant"), &format!("grant={}&reason=oops", g.id));
            assert!(r.header("location").unwrap().contains("Only%20complimentary"), "{:?}", r.header("location"));
        }
    }
    assert_eq!(until_of(&w, &id), add_months(end, 12));
}

#[test]
fn admins_add_and_reverse_complimentary_maintenance_with_a_reason() {
    use filescape_licence::maintenance::add_months;
    let w = world();
    let (_, id) = bought(&w, "txn_m6");
    let (_, id2) = bought(&w, "txn_m7");
    let end = add_months(T0, 12);
    let r = w.admin(&format!("/licence/{id}/extend"), "months=0&days=30&reason=");
    assert!(r.header("location").unwrap().contains("Give%20a%20reason"));
    let r = w.admin(&format!("/licence/{id}/extend"), "months=0&days=30&reason=support+gesture");
    assert!(r.header("location").unwrap().contains("Maintenance%20changed"), "{:?}", r.header("location"));
    assert_eq!(until_of(&w, &id), end + 30 * DAY);
    let page = w.call("GET", "localhost", &format!("/admin/licence/{id}"), &[], "");
    assert!(page.body.contains("Complimentary extension +30 days") && page.body.contains("support gesture"), "{}", page.body);
    let ev = pollster::block_on(w.cx().events_of(&id)).unwrap();
    assert!(ev.iter().any(|e| e.what == "maintenance" && e.detail.contains("→") && e.actor.as_deref() == Some("dev")));
    // Reverse it: back to the purchased date, not below.
    let g = pollster::block_on(w.cx().grants_of(&id)).unwrap().into_iter().find(|g| g.grant.admin_reversible()).unwrap();
    w.admin(&format!("/licence/{id}/reverse-grant"), &format!("grant={}&reason=wrong+licence", g.id));
    assert_eq!(until_of(&w, &id), end);
    // Bulk: each licence from its own date; temporary and unknown IDs are reported, not extended.
    let tmp = st_licence(&w, &w.temp_serial(30, 3));
    let r = w.admin("/maintenance", &format!("ids={id}%0A{id2}%0A{id2}%0A{tmp}%0Anope&months=1&days=0&reason=outage"));
    let loc = r.header("location").unwrap().to_string();
    assert!(loc.contains("Extended%202%20of%204"), "{loc}");
    assert_eq!(until_of(&w, &id), add_months(end, 1));
    assert_eq!(until_of(&w, &id2), add_months(end, 1));
}

#[test]
fn the_migration_keeps_existing_licences_dates() {
    // A database as it was before 0004, with one perpetual licence issued on the old 30.44-day rule.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    for m in &SCHEMA[..3] {
        conn.execute_batch(m).unwrap();
    }
    let old_until = T0 + 12 * 2_629_746;
    conn.execute(
        "INSERT INTO licences (id, serial_hash, serial_hint, kind, validity, issued_at, max_activations) VALUES ('L1', 'h1', 'AAAA', 'perpetual', ?1, ?2, 3), ('L2', 'h2', 'BBBB', 'temporary', '{\"mode\":\"first_activation\",\"days\":30}', ?2, 3)",
        rusqlite::params![format!("{{\"mode\":\"perpetual\",\"updates_until\":{old_until}}}"), T0],
    )
    .unwrap();
    conn.execute_batch(SCHEMA[3]).unwrap();
    let (u, n): (i64, i64) = conn.query_row("SELECT maintenance_until, (SELECT count(*) FROM maintenance_grants) FROM licences WHERE id = 'L1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((u, n), (old_until, 1));
    let g: (String, i64, i64) = conn.query_row("SELECT kind, at, until FROM maintenance_grants WHERE licence_id = 'L1'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(g, ("purchase".to_string(), T0, old_until));
    let t: Option<i64> = conn.query_row("SELECT maintenance_until FROM licences WHERE id = 'L2'", [], |r| r.get(0)).unwrap();
    assert_eq!(t, None);
}

#[test]
fn two_deliveries_of_one_renewal_at_once_extend_once() {
    use filescape_licence::maintenance::{Grant, add_months};
    use filescape_licence_server::maintenance::NewGrant;
    let w = world();
    let (_, id) = bought(&w, "txn_m8");
    let cx = w.cx();
    let g = NewGrant { grant: Grant::renewal(T0, 12), txn: Some("txn_dup"), amount: None, currency: None, actor: "paddle", reason: "" };
    let (a, b) = pollster::block_on(futures::future::join(cx.add_grant(&id, &g, None), cx.add_grant(&id, &g, None)));
    let applied = [a.unwrap(), b.unwrap()].iter().filter(|o| matches!(o, filescape_licence_server::maintenance::Outcome::Applied { .. })).count();
    assert_eq!(applied, 1);
    assert_eq!(until_of(&w, &id), add_months(add_months(T0, 12), 12));
}

// ---- Renewals through Paddle ----

fn offer_renewals(w: &World) {
    let r = w.admin("/renewals", "enabled=1&standard_id=pri_renew&standard_label=%242.99&promo_id=&promo_label=&enterprise_id=&enterprise_label=");
    assert!(r.header("location").unwrap().contains("saved"), "{:?}", r.header("location"));
}

fn renewal_paid(txn: &str, token: &str, event: &str) -> serde_json::Value {
    serde_json::json!({
        "event_id": event, "event_type": "transaction.completed",
        "data": { "id": txn, "status": "completed", "customer_id": "ctm_1", "currency_code": "USD",
                  "custom_data": { "renewal": token },
                  "details": { "totals": { "grand_total": "299" } },
                  "items": [ { "price": { "id": "pri_renew" }, "quantity": 1 } ] }
    })
}

fn sign_in(w: &World) -> String {
    let acc = |path: &str, body: &str| w.call("POST", "account.filescape.app", path, &[("origin", "https://account.filescape.app")], body);
    acc("/login", "email=ada%40example.com");
    let m: serde_json::Value = serde_json::from_str(w.net.emails().last().unwrap()).unwrap();
    let body = m["text"].as_str().unwrap();
    let i = body.find("/signin?t=").unwrap() + "/signin?t=".len();
    let t: String = body[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    session_cookie(&acc("/signin", &format!("t={t}")))
}

fn token_from(location: &str) -> String {
    location.split("t=").nth(1).unwrap().to_string()
}

#[test]
fn renewals_are_admin_priced_and_never_use_a_purchase_price() {
    let w = world();
    let r = w.admin("/renewals", "enabled=1&standard_id=pri_filescape&standard_label=%242.99");
    assert!(r.body.contains("Not saved") && r.body.contains("purchase price"), "{}", r.body);
    // A failed save shows the form again with what was typed.
    let r = w.admin("/renewals", "enabled=1&standard_id=pri_renew&standard_label=");
    assert!(r.body.contains("short label") && r.body.contains("value=\"pri_renew\"") && r.body.contains("Not offered"), "{}", r.body);
    let r = w.admin("/renewals", "enabled=1");
    assert!(r.body.contains("set the standard price"));
    offer_renewals(&w);
    // Switching to a promotional price keeps the old one accepted for open checkouts.
    w.admin("/renewals", "enabled=1&standard_id=pri_renew2&standard_label=%242.99&promo_id=pri_promo&promo_label=%241.99&use_promo=1");
    let s = pollster::block_on(w.cx().renewal_settings()).unwrap();
    assert_eq!(s.offered().unwrap().price_id, "pri_promo");
    assert!(s.accepted().contains(&"pri_renew".to_string()) && s.accepted().contains(&"pri_renew2".to_string()));
    let page = w.call("GET", "localhost", "/admin/renewals", &[], "");
    assert!(page.body.contains("Offered") && page.body.contains("$1.99"), "{}", page.body);
}

#[test]
fn a_customer_renews_from_the_account_page_and_paddle_extends_the_same_licence() {
    use filescape_licence::maintenance::add_months;
    let w = world();
    let (serial, id) = bought(&w, "txn_buy1");
    ok(&w.activate(&serial, "m1", "act-m1-00001"));
    let cookie = sign_in(&w);
    let acc = |method: &str, path: &str, body: &str| w.call(method, "account.filescape.app", path, &[("cookie", &cookie), ("origin", "https://account.filescape.app")], body);
    // Not offered yet: no button, and asking anyway says so.
    let home = acc("GET", "/", "");
    assert!(home.body.contains("Software maintenance:") && !home.body.contains("Extend maintenance by"), "{}", home.body);
    assert!(acc("POST", "/renew", &format!("licence={id}")).header("location").unwrap().contains("aren%27t%20available"));
    offer_renewals(&w);
    assert!(acc("GET", "/", "").body.contains("Extend maintenance by 12 months · $2.99"));
    // Someone else's licence can't be renewed from this account.
    let (_, other) = bought(&w, "txn_buy2");
    pollster::block_on(w.cx().set_owner(&other, "Bob", "bob@example.com", "dev")).unwrap();
    assert!(acc("POST", "/renew", &format!("licence={other}")).header("location").unwrap().contains("isn%27t%20registered"));
    // Ada's: a checkout page with Paddle allowed on it only.
    let r = acc("POST", "/renew", &format!("licence={id}"));
    let loc = r.header("location").unwrap().to_string();
    assert!(loc.starts_with("/renew?t="), "{loc}");
    let t = token_from(&loc);
    let page = w.call("GET", "account.filescape.app", &loc, &[], "");
    assert!(page.body.contains("cdn.paddle.com/paddle/v2/paddle.js") && page.body.contains("pri_renew") && page.body.contains(&t));
    assert!(page.body.contains(&serial[serial.len() - 4..]) && page.body.contains("non-refundable"));
    assert!(page.header("content-security-policy").unwrap().contains("https://buy.paddle.com"));
    assert!(!acc("GET", "/", "").header("content-security-policy").unwrap().contains("paddle"), "other pages stay locked down");
    // Paddle confirms: 12 months on the same licence, recorded with the amount, emailed once.
    w.now.set(T0 + 20 * DAY);
    let before = w.net.emails().len();
    let r = w.webhook(&renewal_paid("txn_ren1", &t, "evt_ren1"));
    assert!(r.body.contains("renewed"), "{}", r.body);
    assert_eq!(until_of(&w, &id), add_months(add_months(T0, 12), 12));
    let g = pollster::block_on(w.cx().grant_by_txn("txn_ren1")).unwrap().unwrap().1;
    assert_eq!((g.amount.as_deref(), g.currency.as_deref()), (Some("2.99"), Some("USD")));
    assert_eq!(w.net.emails().len(), before + 1);
    assert!(w.net.emails().last().unwrap().contains("maintenance is extended"));
    // Resent as a new event: nothing changes, no second email. No new licence either.
    let r = w.webhook(&renewal_paid("txn_ren1", &t, "evt_ren1b"));
    assert!(r.body.contains("already applied"), "{}", r.body);
    assert_eq!(w.net.emails().len(), before + 1);
    assert_eq!(pollster::block_on(w.cx().list("", Default::default(), 100)).unwrap().len(), 2);
    // The same activation's weekly check now carries the new date.
    let v = ok(&w.api("/v1/validate", &serde_json::json!({ "activation_id": "act-m1-00001", "device_hash": device_hash("m1") })));
    let c: ActivationCert = w.verifier().verify(&v.activation, Purpose::Activation).unwrap();
    assert_eq!(c.updates_until, Some(add_months(add_months(T0, 12), 12)));
    // Admin sees it.
    assert!(w.call("GET", "localhost", "/admin/renewals", &[], "").body.contains("txn_ren1"));
    // The link stops opening the checkout after a day.
    w.now.set(T0 + 22 * DAY);
    assert!(w.call("GET", "account.filescape.app", &loc, &[], "").body.contains("expired"));
}

#[test]
fn the_app_gets_a_renewal_link_for_its_own_live_activation_only() {
    let w = world();
    let (serial, _) = bought(&w, "txn_buy3");
    ok(&w.activate(&serial, "m1", "act-m1-00001"));
    let link = |id: &str, m: &str| w.api("/v1/renew", &serde_json::json!({ "activation_id": id, "device_hash": device_hash(m) }));
    let r = link("act-m1-00001", "m1");
    assert_eq!(r.status, 403, "not offered yet: {}", r.body);
    offer_renewals(&w);
    let r = link("act-m1-00001", "m1");
    assert_eq!(r.status, 200, "{}", r.body);
    let url = serde_json::from_str::<serde_json::Value>(&r.body).unwrap()["url"].as_str().unwrap().to_string();
    assert!(url.starts_with("http://localhost:8787/account/renew?t="), "{url}");
    assert_eq!(rejection(&link("act-m1-00001", "someone-else")), Rejection::ActivationIdConflict);
    assert_eq!(rejection(&link("act-nope-0001", "m1")), Rejection::NotFound);
    // Temporary licences: refused.
    let tmp = w.temp_serial(30, 3);
    ok(&w.activate(&tmp, "m2", "act-m2-00001"));
    let r = link("act-m2-00001", "m2");
    assert!(r.status == 403 && r.body.contains("can't be renewed"), "{}", r.body);
    // A deactivated computer can't ask.
    w.api("/v1/deactivate", &serde_json::json!({ "activation_id": "act-m1-00001", "device_hash": device_hash("m1") }));
    assert_eq!(rejection(&link("act-m1-00001", "m1")), Rejection::Deactivated);
}

#[test]
fn a_renewal_payment_without_a_valid_token_extends_nothing_and_is_shown_to_the_admin() {
    let w = world();
    let (_, id) = bought(&w, "txn_buy4");
    offer_renewals(&w);
    let before = until_of(&w, &id);
    let r = w.webhook(&renewal_paid("txn_forged", "not-a-real-token", "evt_forged"));
    assert_eq!(r.status, 200);
    assert!(r.body.contains("renewal problem"), "{}", r.body);
    assert_eq!(until_of(&w, &id), before);
    assert_eq!(pollster::block_on(w.cx().list("", Default::default(), 100)).unwrap().len(), 1, "no licence issued for a renewal price");
    assert!(w.call("GET", "localhost", "/admin/renewals", &[], "").body.contains("txn_forged"));
    // A renewal of a licence revoked after the checkout opened: flagged for a refund.
    let t = pollster::block_on(w.cx().start_renewal(&id, "account", None)).unwrap().unwrap();
    w.admin(&format!("/licence/{id}/revoke"), "reason=refunded");
    let r = w.webhook(&renewal_paid("txn_late", &t, "evt_late"));
    assert!(r.body.contains("renewal problem") && r.body.contains("Refund it"), "{}", r.body);
    assert!(pollster::block_on(w.cx().events_of(&id)).unwrap().iter().any(|e| e.what == "renewal problem"));
}

#[test]
fn a_refunded_or_charged_back_renewal_gives_its_months_back_and_a_reversal_restores_them() {
    use filescape_licence::maintenance::add_months;
    let w = world();
    let (_, id) = bought(&w, "txn_buy5");
    offer_renewals(&w);
    let adj = |evt: &str, action: &str, status: &str, txn: &str| {
        w.webhook(&serde_json::json!({ "event_id": evt, "event_type": "adjustment.updated",
            "data": { "id": format!("adj_{evt}"), "action": action, "status": status, "type": "full", "transaction_id": txn } }))
    };
    let end = add_months(T0, 12);
    let t = pollster::block_on(w.cx().start_renewal(&id, "account", None)).unwrap().unwrap();
    w.webhook(&renewal_paid("txn_r_a", &t, "evt_a"));
    assert_eq!(until_of(&w, &id), add_months(end, 12));
    assert!(adj("ea1", "refund", "pending_approval", "txn_r_a").body.contains("flagged"));
    assert_eq!(until_of(&w, &id), add_months(end, 12));
    assert!(adj("ea2", "refund", "approved", "txn_r_a").body.contains("renewal reversed"));
    assert_eq!(until_of(&w, &id), end);
    let lic = pollster::block_on(w.cx().licence(&id)).unwrap().unwrap();
    assert!(lic.lic.revoked_at.is_none(), "the licence itself stays valid");
    // A chargeback, then its reversal.
    w.webhook(&renewal_paid("txn_r_b", &t, "evt_b"));
    adj("eb1", "chargeback", "approved", "txn_r_b");
    assert_eq!(until_of(&w, &id), end);
    assert!(adj("eb2", "chargeback_reverse", "approved", "txn_r_b").body.contains("restored"));
    assert_eq!(until_of(&w, &id), add_months(end, 12));
    // A refund of the purchase still revokes the licence, as before.
    assert!(adj("ec1", "refund", "approved", "txn_buy5").body.contains("revoked"));
}
