//! Customer sign-in for account.filescape.app: an emailed one-time link, then a cookie.
//!
//! No passwords. A link is a random token whose hash is kept for 15 minutes and works once; using
//! it (a POST, so mail scanners that fetch links don't spend it) starts a 30-day session, whose
//! token goes in an HttpOnly, Secure, SameSite=Lax cookie and is also only kept as a hash.
//! A signed-in customer sees the licences registered to their email and can release computers.

use crate::db::{self, ActivationRow, LicenceRow};
use crate::{Ctx, Db, Net, Req};
use filescape_licence::{DAY, Ts};
use ring::rand::SecureRandom;

pub const LINK_LIFE: Ts = 15 * 60;
pub const SESSION_LIFE: Ts = 30 * DAY;
pub const COOKIE: &str = "fs_session";

fn token(rng: &dyn SecureRandom) -> String {
    let mut b = [0u8; 32];
    rng.fill(&mut b).expect("random");
    data_encoding::BASE64URL_NOPAD.encode(&b)
}

pub fn token_hash(t: &str) -> String {
    data_encoding::HEXLOWER.encode(ring::digest::digest(&ring::digest::SHA256, t.as_bytes()).as_ref())
}

/// The session token in the request's cookie, if any.
pub fn cookie(req: &Req) -> Option<String> {
    req.header("cookie")?.split(';').filter_map(|p| p.trim().split_once('=')).find(|(k, _)| *k == COOKIE).map(|(_, v)| v.to_string()).filter(|v| v.len() == 43)
}

pub fn set_cookie(value: &str, max_age: Ts, secure: bool) -> String {
    format!("{COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{}", if secure { "; Secure" } else { "" })
}

/// One licence on the account page, with its computers.
pub struct Owned {
    pub row: LicenceRow,
    pub seats: u32,
    pub computers: Vec<ActivationRow>,
}

impl<D: Db, N: Net> Ctx<'_, D, N> {
    /// A new sign-in link token for `email`.
    pub async fn new_login(&self, email: &str) -> Result<String, String> {
        let t = token(&self.rng);
        self.db
            .batch(vec![
                // Old, unused links for this address stop working: only the newest one counts.
                ("UPDATE logins SET used_at = ?1 WHERE email = ?2 AND used_at IS NULL", vec![self.now.into(), email.into()]),
                ("INSERT INTO logins (token_hash, email, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)", vec![token_hash(&t).into(), email.into(), self.now.into(), (self.now + LINK_LIFE).into()]),
            ])
            .await?;
        Ok(t)
    }

    /// Spends a sign-in link: on success, a new session token.
    pub async fn use_login(&self, t: &str) -> Result<Option<String>, String> {
        let h = token_hash(t);
        let Some(r) = self.db.first("SELECT email FROM logins WHERE token_hash = ?1 AND used_at IS NULL AND expires_at > ?2", vec![(&h).into(), self.now.into()]).await? else {
            return Ok(None);
        };
        let email = db::req_text(&r, "email")?;
        let s = token(&self.rng);
        let changes = self
            .db
            .batch(vec![
                ("UPDATE logins SET used_at = ?1 WHERE token_hash = ?2 AND used_at IS NULL", vec![self.now.into(), (&h).into()]),
                ("INSERT INTO sessions (token_hash, email, created_at, expires_at) SELECT ?1, ?2, ?3, ?4 WHERE changes() = 1", vec![token_hash(&s).into(), (&email).into(), self.now.into(), (self.now + SESSION_LIFE).into()]),
                ("DELETE FROM sessions WHERE expires_at <= ?1", vec![self.now.into()]),
                ("DELETE FROM logins WHERE expires_at <= ?1", vec![(self.now - DAY).into()]),
            ])
            .await?;
        Ok((changes.get(1) == Some(&1)).then_some(s))
    }

    /// The email a session cookie belongs to.
    pub async fn session_email(&self, req: &Req) -> Result<Option<String>, String> {
        let Some(t) = cookie(req) else { return Ok(None) };
        let r = self.db.first("SELECT email FROM sessions WHERE token_hash = ?1 AND expires_at > ?2", vec![token_hash(&t).into(), self.now.into()]).await?;
        Ok(r.and_then(|r| db::text(&r, "email")))
    }

    pub async fn end_session(&self, req: &Req) -> Result<(), String> {
        if let Some(t) = cookie(req) {
            self.db.run("DELETE FROM sessions WHERE token_hash = ?1", vec![token_hash(&t).into()]).await?;
        }
        Ok(())
    }

    /// Everything registered to `email`, with each licence's computers (live ones first).
    pub async fn owned(&self, email: &str) -> Result<Vec<Owned>, String> {
        let mut out = Vec::new();
        for row in self.licences_by_email(email).await? {
            let mut computers = self.activations_of(&row.lic.id).await?;
            let acts: Vec<_> = computers.iter().map(|c| c.a.clone()).collect();
            let seats = filescape_licence::rules::seats_in_use(&row.lic, &acts, self.now);
            computers.sort_by_key(|c| (!c.a.is_live(), -c.a.created_at));
            out.push(Owned { row, seats, computers });
        }
        Ok(out)
    }

    /// Releases a computer from the account page, if it belongs to a licence registered to
    /// `email`. Returns whether anything was released.
    pub async fn release_for(&self, email: &str, activation_id: &str) -> Result<bool, String> {
        let Some(a) = self.activation(activation_id).await? else { return Ok(false) };
        let Some(l) = self.licence(&a.a.licence_id).await? else { return Ok(false) };
        if l.lic.email.as_deref() != Some(email) || !a.a.is_live() {
            return Ok(false);
        }
        self.release(&a.a, "account").await?;
        Ok(true)
    }
}
