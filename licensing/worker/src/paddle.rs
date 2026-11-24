//! The Paddle Billing webhook (POST api.filescape.app/v1/paddle).
//!
//! - `transaction.completed`: one perpetual licence per unit of a Filescape price bought, keyed on
//!   the Paddle transaction ID so a resent notification changes nothing, then emailed to the
//!   buyer. If the email fails the reply is 500, so Paddle sends the notification again later
//!   and the email is retried (the licence already exists and isn't issued twice).
//! - `adjustment.created` / `adjustment.updated`: a refund or chargeback flags the licence for the
//!   admin, who decides whether to revoke it.
//!
//! Every notification's `Paddle-Signature` is checked against the webhook secret first.

use crate::store::event_stmt;
use crate::{Ctx, Db, Net, Req, Resp, mail};
use filescape_licence::Ts;
use ring::hmac;
use serde_json::Value;

/// How old a signed notification may be.
const TOLERANCE: Ts = 300;

/// Checks `Paddle-Signature: ts=…;h1=…` (several `h1` while a secret is being rotated).
pub fn verify(secret: &str, header: &str, body: &[u8], now: Ts) -> Result<(), &'static str> {
    let mut ts = None;
    let mut sigs = Vec::new();
    for part in header.split(';') {
        match part.trim().split_once('=') {
            Some(("ts", v)) => ts = v.parse::<Ts>().ok(),
            Some(("h1", v)) => sigs.push(v.to_string()),
            _ => {}
        }
    }
    let ts = ts.ok_or("no timestamp")?;
    if (now - ts).abs() > TOLERANCE {
        return Err("timestamp too old");
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let mut msg = format!("{ts}:").into_bytes();
    msg.extend_from_slice(body);
    for s in sigs {
        if let Ok(sig) = data_encoding::HEXLOWER_PERMISSIVE.decode(s.as_bytes())
            && hmac::verify(&key, &msg, &sig).is_ok()
        {
            return Ok(());
        }
    }
    Err("bad signature")
}

/// For tests and the setup guide's sample request.
pub fn sign(secret: &str, body: &[u8], ts: Ts) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let mut msg = format!("{ts}:").into_bytes();
    msg.extend_from_slice(body);
    format!("ts={ts};h1={}", data_encoding::HEXLOWER.encode(hmac::sign(&key, &msg).as_ref()))
}

pub async fn webhook<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req) -> Result<Resp, String> {
    let Some(pc) = &cx.cfg.paddle else { return Ok(Resp::text(503, "Paddle isn't set up.")) };
    if let Err(why) = verify(&pc.webhook_secret, req.header("paddle-signature").unwrap_or(""), &req.body, cx.now) {
        return Ok(Resp::text(401, why));
    }
    let ev: Value = serde_json::from_slice(&req.body).map_err(|e| format!("Paddle sent unreadable JSON: {e}"))?;
    let event_id = ev["event_id"].as_str().unwrap_or_default().to_string();
    let kind = ev["event_type"].as_str().unwrap_or_default().to_string();
    if event_id.is_empty() {
        return Ok(Resp::text(400, "No event_id."));
    }
    if cx.db.first("SELECT outcome FROM webhooks WHERE event_id = ?1", vec![(&event_id).into()]).await?.is_some() {
        return Ok(Resp::text(200, "Already handled."));
    }
    let outcome = match kind.as_str() {
        "transaction.completed" => match completed(cx, &ev["data"]).await? {
            Ok(o) => o,
            // Not recorded, so Paddle's retry tries again.
            Err(e) => return Ok(Resp::text(500, &e)),
        },
        "adjustment.created" | "adjustment.updated" => adjustment(cx, &ev["data"]).await?,
        other => format!("ignored {other}"),
    };
    cx.db
        .run("INSERT OR IGNORE INTO webhooks (event_id, received_at, kind, outcome) VALUES (?1, ?2, ?3, ?4)", vec![event_id.into(), cx.now.into(), kind.into(), (&outcome).into()])
        .await?;
    Ok(Resp::text(200, &outcome))
}

/// How many Filescape licences a transaction paid for. The Paddle account also sells other
/// products, whose notifications arrive here too: only the configured Filescape prices count, and
/// with none configured nothing is issued.
pub fn units(data: &Value, price_ids: &[String]) -> u32 {
    data["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|i| i["price"]["id"].as_str().or(i["price_id"].as_str()).is_some_and(|p| price_ids.iter().any(|x| x == p)))
                .map(|i| i["quantity"].as_u64().unwrap_or(1) as u32)
                .sum()
        })
        .unwrap_or(0)
}

async fn customer<D: Db, N: Net>(cx: &Ctx<'_, D, N>, customer_id: &str) -> Result<(Option<String>, Option<String>), String> {
    let Some(pc) = &cx.cfg.paddle else { return Ok((None, None)) };
    let Some(key) = &pc.api_key else { return Ok((None, None)) };
    if customer_id.is_empty() || !customer_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Ok((None, None));
    }
    let (status, body) = cx.net.send("GET", &format!("{}/customers/{customer_id}", pc.api_base), &[("authorization", format!("Bearer {key}"))], None).await?;
    if status != 200 {
        return Err(format!("Paddle customer lookup said {status}"));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let name = v["data"]["name"].as_str().filter(|s| !s.trim().is_empty()).map(str::to_string);
    Ok((name, v["data"]["email"].as_str().map(str::to_string)))
}

/// `Ok(Ok(outcome))`, or `Ok(Err(why))` to have Paddle retry.
///
/// The licences are issued first, before anything that can fail (the Paddle customer lookup, the
/// email), so a paid order always has its licences. A failure after that returns 500: Paddle sends
/// the notification again, which finds the same licences (keyed on the transaction) and retries
/// the lookup and the email. If Paddle gives up, the licences sit in the admin's "Not emailed" list.
async fn completed<D: Db, N: Net>(cx: &Ctx<'_, D, N>, data: &Value) -> Result<Result<String, String>, String> {
    let pc = cx.cfg.paddle.as_ref().unwrap();
    let txn = data["id"].as_str().unwrap_or_default();
    if txn.is_empty() {
        return Ok(Ok("no transaction id".into()));
    }
    if pc.price_ids.is_empty() {
        return Ok(Ok(format!("ignored {txn}: PADDLE_PRICE_IDS isn't set")));
    }
    let n = units(data, &pc.price_ids).min(50);
    if n == 0 {
        return Ok(Ok(format!("ignored {txn}: not a Filescape purchase")));
    }
    let mut issued = Vec::new();
    for i in 0..n {
        let order = if i == 0 { txn.to_string() } else { format!("{txn}#{}", i + 1) };
        issued.push(cx.issue_perpetual(None, None, &order, "paddle").await?);
    }
    let ids: Vec<String> = issued.iter().map(|(_, r)| r.lic.id.clone()).collect();
    if issued.iter().all(|(_, r)| r.emailed_at.is_some()) {
        return Ok(Ok(format!("{txn}: issued {}", ids.join(", "))));
    }
    let (name, email) = match customer(cx, data["customer_id"].as_str().unwrap_or_default()).await {
        Ok(c) => c,
        Err(e) => return Ok(Err(format!("licence issued ({}) but the buyer lookup failed: {e}", ids.join(", ")))),
    };
    let mut mail_errors = Vec::new();
    for (serial, row) in &issued {
        if row.emailed_at.is_some() {
            continue;
        }
        cx.fill_buyer(&row.lic.id, name.as_deref(), email.as_deref()).await?;
        let row = cx.licence(&row.lic.id).await?.ok_or("licence vanished")?;
        if let Some(to) = &row.lic.email {
            match mail::send(cx, &mail::licence_message(&row, serial, to)).await {
                Ok(true) => cx.mark_emailed(&row.lic.id, to, "paddle").await?,
                Ok(false) => {}
                Err(e) => mail_errors.push(e),
            }
        }
    }
    if let Some(e) = mail_errors.first() {
        return Ok(Err(format!("licence issued but not emailed: {e}")));
    }
    Ok(Ok(format!("{txn}: issued {}", ids.join(", "))))
}

/// Refunds and chargebacks. A full refund or chargeback that Paddle has approved revokes the
/// licences it paid for (the refunds page promises this); one still pending, a chargeback warning
/// or a partial refund only flags them for the admin. A reversed chargeback reinstates licences
/// that a chargeback revoked.
async fn adjustment<D: Db, N: Net>(cx: &Ctx<'_, D, N>, data: &Value) -> Result<String, String> {
    let action = data["action"].as_str().unwrap_or_default();
    let status = data["status"].as_str().unwrap_or_default();
    let txn = data["transaction_id"].as_str().unwrap_or_default();
    let adj = data["id"].as_str().unwrap_or_default();
    if txn.is_empty() || !matches!(action, "refund" | "chargeback" | "chargeback_warning" | "chargeback_reverse") {
        return Ok(format!("ignored adjustment {action}"));
    }
    let full = data["type"].as_str().is_none_or(|t| t == "full");
    let flag = format!("{action} {status}{}", if full { "" } else { " (partial)" }).replace('_', " ");
    let rows = cx.db.all("SELECT id, revoke_reason FROM licences WHERE order_ref = ?1 OR order_ref LIKE ?2 ESCAPE '\\'", vec![txn.into(), format!("{}#%", txn.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")).into()]).await?;
    if rows.is_empty() {
        // A refund of another product sold through the same Paddle account.
        return Ok(format!("ignored {action} on {txn}: not a Filescape purchase"));
    }
    let revoke = full && status == "approved" && matches!(action, "refund" | "chargeback");
    let reinstate = status == "approved" && action == "chargeback_reverse";
    let reason = format!("Paddle {} {adj}", action.replace('_', " "));
    let mut stmts = Vec::new();
    for r in &rows {
        let id = crate::db::text(r, "id").unwrap_or_default();
        stmts.push(("UPDATE licences SET refund_flag = ?1 WHERE id = ?2", vec![(&flag).into(), (&id).into()]));
        stmts.push(event_stmt(&id, cx.now, "paddle", &format!("{flag} ({adj})"), "paddle"));
        if revoke {
            stmts.push(("UPDATE licences SET revoked_at = ?1, revoke_reason = ?2 WHERE id = ?3 AND revoked_at IS NULL", vec![cx.now.into(), (&reason).into(), (&id).into()]));
            stmts.push(event_stmt(&id, cx.now, "revoked", &reason, "paddle"));
        }
        if reinstate && crate::db::text(r, "revoke_reason").is_some_and(|w| w.starts_with("Paddle chargeback")) {
            stmts.push(("UPDATE licences SET revoked_at = NULL, revoke_reason = NULL WHERE id = ?1", vec![(&id).into()]));
            stmts.push(event_stmt(&id, cx.now, "reinstated", &reason, "paddle"));
        }
    }
    cx.db.batch(stmts).await?;
    let what = if revoke { "revoked" } else if reinstate { "reinstated" } else { "flagged" };
    Ok(format!("{txn}: {what} {} licence(s): {flag}", rows.len()))
}
