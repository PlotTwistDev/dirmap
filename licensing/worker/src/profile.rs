//! The signed-in customer's own settings on account.filescape.app: reminder preferences, changing
//! the email address their licences are registered to, receipts, and a per-IP limit on the forms
//! that send email.
//!
//! An email change is confirmed from the new address, then the old address gets an undo link that
//! works for [`UNDO_LIFE`]. While it works, the licences can't be moved again, so someone who got
//! into a customer's account can't move them out of reach of the undo link. Only hashes of the
//! tokens are kept.

use crate::db::{self, Stmt};
use crate::portal::token_hash;
use crate::store::event_stmt;
use crate::{Ctx, Db, Net, Req};
use filescape_licence::{DAY, Ts};
use ring::rand::SecureRandom;

pub const CONFIRM_LIFE: Ts = DAY;
pub const UNDO_LIFE: Ts = 14 * DAY;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefs {
    /// Emails before maintenance ends (30 and 7 days, and on the day).
    pub reminders: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { reminders: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeRefusal {
    Invalid,
    Same,
    /// The new address already has licences of its own. Merging accounts is a support job.
    Taken,
    /// An undo link for an earlier change is still open, until this time.
    Locked(Ts),
    NoLicences,
}

impl ChangeRefusal {
    pub fn message(&self) -> String {
        match self {
            ChangeRefusal::Invalid => "Enter a valid email address.".into(),
            ChangeRefusal::Same => "That's already the address your licences are registered to.".into(),
            ChangeRefusal::Taken => "That address already has Filescape licences. To put all your licences on one address, contact us at https://filescape.app/contact.".into(),
            ChangeRefusal::Locked(until) => format!(
                "Your email address was changed recently, so it can't be changed again until {}. Contact us at https://filescape.app/contact if you need it sooner.",
                crate::web::date(*until).split(',').next().unwrap_or_default()
            ),
            ChangeRefusal::NoLicences => "No licences are registered to this address.".into(),
        }
    }
}

/// What spending a confirm or undo link did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeDone {
    /// The licences moved from `old` to `new`; `undo` is the token for the old address.
    Moved { old: String, new: String, count: usize, undo: String },
    /// The licences went back to `old`.
    Undone { old: String, new: String, count: usize },
    Expired,
    Refused(ChangeRefusal),
}

pub fn valid_email(e: &str) -> bool {
    let Some((user, domain)) = e.split_once('@') else { return false };
    !user.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
        && e.len() <= 200
        && !e.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"' | '\''))
}

fn token(rng: &dyn SecureRandom) -> String {
    let mut b = [0u8; 32];
    rng.fill(&mut b).expect("random");
    data_encoding::BASE64URL_NOPAD.encode(&b)
}

impl<D: Db, N: Net> Ctx<'_, D, N> {
    pub async fn prefs(&self, email: &str) -> Result<Prefs, String> {
        let r = self.db.first("SELECT reminders FROM email_prefs WHERE email = ?1", vec![email.into()]).await?;
        Ok(r.map_or_else(Prefs::default, |r| Prefs { reminders: db::int(&r, "reminders").unwrap_or(1) != 0 }))
    }

    pub async fn set_prefs(&self, email: &str, p: Prefs) -> Result<(), String> {
        self.db
            .run(
                "INSERT INTO email_prefs (email, reminders, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(email) DO UPDATE SET reminders = excluded.reminders, updated_at = excluded.updated_at",
                vec![email.into(), (p.reminders as i64).into(), self.now.into()],
            )
            .await?;
        Ok(())
    }

    /// Whether this IP address may post `kind` again: at most `max` in `within` seconds. Posts
    /// without an address (development, tests) are always allowed.
    pub async fn ip_allowed(&self, req: &Req, kind: &str, max: i64, within: Ts) -> Result<bool, String> {
        let Some(ip) = req.header("cf-connecting-ip").map(str::trim).filter(|s| !s.is_empty()) else { return Ok(true) };
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &self.cfg.serial_key);
        let h = data_encoding::HEXLOWER.encode(&ring::hmac::sign(&key, format!("ip:{ip}").as_bytes()).as_ref()[..12]);
        let n = self
            .db
            .first("SELECT count(*) AS n FROM ip_log WHERE ip_hash = ?1 AND kind = ?2 AND at > ?3", vec![(&h).into(), kind.into(), (self.now - within).into()])
            .await?
            .and_then(|r| db::int(&r, "n"))
            .unwrap_or(0);
        if n >= max {
            return Ok(false);
        }
        self.db
            .batch(vec![
                ("INSERT INTO ip_log (ip_hash, kind, at) VALUES (?1, ?2, ?3)", vec![(&h).into(), kind.into(), self.now.into()]),
                ("DELETE FROM ip_log WHERE at < ?1", vec![(self.now - DAY).into()]),
            ])
            .await?;
        Ok(true)
    }

    /// The end of an open undo window that blocks changing `email`, if any.
    async fn change_lock(&self, email: &str) -> Result<Option<Ts>, String> {
        let r = self
            .db
            .first("SELECT max(expires_at) AS until FROM email_changes WHERE kind = 'undo' AND new_email = ?1 AND used_at IS NULL AND expires_at > ?2", vec![email.into(), self.now.into()])
            .await?;
        Ok(r.and_then(|r| db::int(&r, "until")))
    }

    async fn change_refusal(&self, old: &str, new: &str) -> Result<Option<ChangeRefusal>, String> {
        if !valid_email(new) {
            return Ok(Some(ChangeRefusal::Invalid));
        }
        if new == old {
            return Ok(Some(ChangeRefusal::Same));
        }
        if let Some(until) = self.change_lock(old).await? {
            return Ok(Some(ChangeRefusal::Locked(until)));
        }
        if self.licences_by_email(old).await?.is_empty() {
            return Ok(Some(ChangeRefusal::NoLicences));
        }
        if !self.licences_by_email(new).await?.is_empty() {
            return Ok(Some(ChangeRefusal::Taken));
        }
        Ok(None)
    }

    /// Starts moving `old`'s licences to `new`: the token for the link emailed to `new`.
    pub async fn request_email_change(&self, old: &str, new: &str) -> Result<Result<String, ChangeRefusal>, String> {
        let new = new.trim().to_lowercase();
        if let Some(why) = self.change_refusal(old, &new).await? {
            return Ok(Err(why));
        }
        let t = token(&self.rng);
        self.db
            .batch(vec![
                // Only the newest request counts.
                ("UPDATE email_changes SET used_at = ?1 WHERE kind = 'confirm' AND old_email = ?2 AND used_at IS NULL", vec![self.now.into(), old.into()]),
                (
                    "INSERT INTO email_changes (token_hash, kind, old_email, new_email, created_at, expires_at) VALUES (?1, 'confirm', ?2, ?3, ?4, ?5)",
                    vec![token_hash(&t).into(), old.into(), (&new).into(), self.now.into(), (self.now + CONFIRM_LIFE).into()],
                ),
            ])
            .await?;
        Ok(Ok(t))
    }

    /// The pending change a link is for (without spending it): `(kind, old, new)`.
    pub async fn email_change(&self, t: &str) -> Result<Option<(String, String, String)>, String> {
        let r = self
            .db
            .first("SELECT kind, old_email, new_email FROM email_changes WHERE token_hash = ?1 AND used_at IS NULL AND expires_at > ?2", vec![token_hash(t).into(), self.now.into()])
            .await?;
        Ok(r.map(|r| (db::text(&r, "kind").unwrap_or_default(), db::text(&r, "old_email").unwrap_or_default(), db::text(&r, "new_email").unwrap_or_default())))
    }

    /// Spends a confirm or undo link.
    pub async fn use_email_change(&self, t: &str) -> Result<ChangeDone, String> {
        let h = token_hash(t);
        let Some(r) = self
            .db
            .first("SELECT kind, old_email, new_email, licence_ids FROM email_changes WHERE token_hash = ?1 AND used_at IS NULL AND expires_at > ?2", vec![(&h).into(), self.now.into()])
            .await?
        else {
            return Ok(ChangeDone::Expired);
        };
        let kind = db::req_text(&r, "kind")?;
        let old = db::req_text(&r, "old_email")?;
        let new = db::req_text(&r, "new_email")?;
        let spend: Stmt = ("UPDATE email_changes SET used_at = ?1 WHERE token_hash = ?2 AND used_at IS NULL", vec![self.now.into(), (&h).into()]);
        if kind == "undo" {
            let ids: Vec<String> = db::text(&r, "licence_ids").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            let mut stmts = vec![spend];
            for id in &ids {
                stmts.push(("UPDATE licences SET email = ?1 WHERE id = ?2 AND email = ?3 AND (SELECT used_at FROM email_changes WHERE token_hash = ?4) = ?5", vec![(&old).into(), id.into(), (&new).into(), (&h).into(), self.now.into()]));
                stmts.push(event_stmt(id, self.now, "email", &format!("Email change undone: back from {new} to {old}"), "account"));
            }
            // The new address loses access: its sessions end and its own pending changes are void.
            stmts.push(("DELETE FROM sessions WHERE email = ?1", vec![(&new).into()]));
            stmts.push(("UPDATE email_changes SET used_at = ?1 WHERE kind = 'confirm' AND old_email = ?2 AND used_at IS NULL", vec![self.now.into(), (&new).into()]));
            let changes = self.db.batch(stmts).await?;
            if changes.first() != Some(&1) {
                return Ok(ChangeDone::Expired);
            }
            let count = (0..ids.len()).filter(|i| changes.get(1 + i * 2) == Some(&1)).count();
            return Ok(ChangeDone::Undone { old, new, count });
        }
        // Confirm: checked again now, as the new address may have bought a licence meanwhile.
        if let Some(why) = self.change_refusal(&old, &new).await? {
            self.db.run(spend.0, spend.1).await?;
            return Ok(ChangeDone::Refused(why));
        }
        let ids: Vec<String> = self.licences_by_email(&old).await?.into_iter().map(|r| r.lic.id).collect();
        let undo = token(&self.rng);
        let mut stmts = vec![spend];
        for id in &ids {
            stmts.push(("UPDATE licences SET email = ?1 WHERE id = ?2 AND email = ?3", vec![(&new).into(), id.into(), (&old).into()]));
            stmts.push(event_stmt(id, self.now, "email", &format!("Email changed by the customer from {old} to {new}"), "account"));
        }
        stmts.push((
            "INSERT INTO email_changes (token_hash, kind, old_email, new_email, licence_ids, created_at, expires_at) VALUES (?1, 'undo', ?2, ?3, ?4, ?5, ?6)",
            vec![token_hash(&undo).into(), (&old).into(), (&new).into(), serde_json::to_string(&ids).unwrap().into(), self.now.into(), (self.now + UNDO_LIFE).into()],
        ));
        // Signed-in browsers on the old address are signed out; preferences come along.
        stmts.push(("DELETE FROM sessions WHERE email = ?1", vec![(&old).into()]));
        stmts.push((
            "INSERT INTO email_prefs (email, reminders, updated_at) SELECT ?1, reminders, ?3 FROM email_prefs WHERE email = ?2 ON CONFLICT(email) DO UPDATE SET reminders = excluded.reminders, updated_at = excluded.updated_at",
            vec![(&new).into(), (&old).into(), self.now.into()],
        ));
        let changes = self.db.batch(stmts).await?;
        if changes.first() != Some(&1) {
            return Ok(ChangeDone::Expired);
        }
        Ok(ChangeDone::Moved { old, new, count: ids.len(), undo })
    }

    /// The Paddle transactions shown on the account page for a licence registered to `email`:
    /// its purchase and any renewals. `txn` must be one of them.
    pub async fn owns_txn(&self, email: &str, licence_id: &str, txn: &str) -> Result<bool, String> {
        let Some(l) = self.licence(licence_id).await? else { return Ok(false) };
        if l.lic.email.as_deref() != Some(email) || !txn.starts_with("txn_") {
            return Ok(false);
        }
        if purchase_txn(l.lic.order_ref.as_deref()) == Some(txn) {
            return Ok(true);
        }
        Ok(self.grant_by_txn(txn).await?.is_some_and(|(id, _)| id == licence_id))
    }

    /// A link to Paddle's PDF invoice for a transaction (Paddle's API makes one that works for a
    /// short while). `None` without an API key or when Paddle has none (e.g. not yet completed).
    pub async fn receipt_url(&self, txn: &str) -> Result<Option<String>, String> {
        let Some(pc) = &self.cfg.paddle else { return Ok(None) };
        let Some(key) = &pc.api_key else { return Ok(None) };
        if !txn.starts_with("txn_") || !txn.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Ok(None);
        }
        let (status, body) = self.net.send("GET", &format!("{}/transactions/{txn}/invoice", pc.api_base), &[("authorization", format!("Bearer {key}"))], None).await?;
        if status != 200 {
            eprintln!("receipt {txn}: Paddle said {status}");
            return Ok(None);
        }
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        Ok(v["data"]["url"].as_str().filter(|u| u.starts_with("https://")).map(str::to_string))
    }
}

/// The Paddle transaction a licence was bought in (its order reference without the `#n` that
/// numbers extra units).
pub fn purchase_txn(order_ref: Option<&str>) -> Option<&str> {
    order_ref.map(|o| o.split('#').next().unwrap_or(o)).filter(|t| t.starts_with("txn_"))
}
