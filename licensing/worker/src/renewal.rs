//! Maintenance renewals bought through Paddle.
//!
//! 1. A customer asks to renew: signed in on the account page (the licence must be registered to
//!    their email), or from the app (`POST /v1/renew` with its activation, which must be live on
//!    this licence and this computer). Temporary and revoked licences are refused.
//! 2. The server stores a random token for that licence (only its hash) and sends the browser to
//!    `account.filescape.app/renew?t=…`, which opens Paddle's checkout with the live renewal price
//!    and the token as custom data.
//! 3. Paddle's signed `transaction.completed` names a renewal price: the token says which licence,
//!    the price, quantity and totals come from Paddle, and the grant is keyed on the transaction ID
//!    ([`crate::maintenance`]), so a resent notification extends once.
//!
//! The browser never names a licence or an amount the server trusts: a forged token finds nothing,
//! and paying with someone else's token only extends their maintenance.
//!
//! Renewals are non-refundable (Rich, 28 Nov 2026). A refund Paddle approves anyway, or a
//! chargeback, reverses the renewal's months; a reversed chargeback restores them.

use crate::maintenance::{NewGrant, Outcome};
use crate::portal::token_hash;
use crate::{Ctx, Db, Net};
use filescape_licence::maintenance::{self, Grant, RenewRefusal};
use filescape_licence::model::Validity;
use filescape_licence::{DAY, Ts};
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};

/// How long a renewal link opens the checkout.
pub const TOKEN_LIFE: Ts = DAY;
/// How long after the link was made a payment can still name it (a checkout left open).
pub const TOKEN_PAYMENT_GRACE: Ts = 30 * DAY;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceChoice {
    pub price_id: String,
    /// What the account page and the app show before Paddle's checkout gives the exact amount with
    /// tax, e.g. "$2.99".
    pub label: String,
}

/// The admin Renewals page's settings (D1 `settings`, key `renewal`). Prices themselves live in
/// Paddle; this picks which one is offered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewalSettings {
    /// Renewals are offered at all.
    pub enabled: bool,
    pub standard: Option<PriceChoice>,
    pub promo: Option<PriceChoice>,
    /// Offer the promotional price instead of the standard one.
    pub use_promo: bool,
    /// For enterprise checkouts (organisations, later); always accepted from Paddle.
    pub enterprise: Option<PriceChoice>,
    /// Price IDs offered before, still accepted for checkouts that were already open.
    #[serde(default)]
    pub retired: Vec<String>,
    /// Months each renewal adds.
    #[serde(default = "twelve")]
    pub months: u32,
}

fn twelve() -> u32 {
    12
}

impl RenewalSettings {
    /// The price customers are offered now, if renewals are on.
    pub fn offered(&self) -> Option<&PriceChoice> {
        if !self.enabled {
            return None;
        }
        if self.use_promo
            && let Some(p) = &self.promo
        {
            return Some(p);
        }
        self.standard.as_ref()
    }

    /// Every price ID whose purchase is a renewal.
    pub fn accepted(&self) -> Vec<String> {
        let mut v: Vec<String> = [&self.standard, &self.promo, &self.enterprise].into_iter().flatten().map(|p| p.price_id.clone()).collect();
        v.extend(self.retired.iter().cloned());
        v.sort();
        v.dedup();
        v
    }

    pub fn months(&self) -> u32 {
        if self.months == 0 { 12 } else { self.months }
    }
}

/// Why a renewal can't start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefusal {
    /// Renewals aren't switched on, or no price is set.
    NotOffered,
    NotFound,
    Refused(RenewRefusal),
}

impl StartRefusal {
    pub fn message(self) -> &'static str {
        match self {
            StartRefusal::NotOffered => "Maintenance renewals aren't available yet.",
            StartRefusal::NotFound => "That licence wasn't found.",
            StartRefusal::Refused(r) => r.message(),
        }
    }
}

/// A token's licence, as the checkout page needs it.
pub struct Pending {
    pub licence_id: String,
    pub serial_hint: String,
    pub until: Ts,
    pub email: Option<String>,
    pub expires_at: Ts,
}

fn token(rng: &dyn SecureRandom) -> String {
    let mut b = [0u8; 24];
    rng.fill(&mut b).expect("random");
    data_encoding::BASE64URL_NOPAD.encode(&b)
}

/// A Paddle total in the currency's smallest unit ("299") as "2.99".
pub fn money(minor: &str, currency: &str) -> String {
    let zero_decimal = matches!(currency, "JPY" | "KRW" | "VND" | "CLP" | "ISK" | "HUF" | "TWD");
    let Ok(n) = minor.parse::<i64>() else { return minor.to_string() };
    if zero_decimal { n.to_string() } else { format!("{}.{:02}", n / 100, (n % 100).abs()) }
}

impl<D: Db, N: Net> Ctx<'_, D, N> {
    pub async fn renewal_settings(&self) -> Result<RenewalSettings, String> {
        let r = self.db.first("SELECT value FROM settings WHERE key = 'renewal'", vec![]).await?;
        Ok(r.and_then(|r| crate::db::text(&r, "value")).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
    }

    /// Saves the settings. A price ID that stops being offered moves to `retired`, so checkouts
    /// already open still count.
    pub async fn set_renewal_settings(&self, mut s: RenewalSettings, actor: &str) -> Result<(), String> {
        let old = self.renewal_settings().await?;
        let now_ids = s.accepted();
        let mut retired = old.retired.clone();
        retired.extend(old.accepted().into_iter().filter(|p| !now_ids.contains(p)));
        retired.retain(|p| ![&s.standard, &s.promo, &s.enterprise].into_iter().flatten().any(|c| &c.price_id == p));
        retired.sort();
        retired.dedup();
        s.retired = retired;
        let offered = s.offered().map_or("not offered".to_string(), |p| format!("offered at {} ({})", p.label, p.price_id));
        self.db
            .batch(vec![
                ("INSERT INTO settings (key, value) VALUES ('renewal', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", vec![serde_json::to_string(&s).unwrap().into()]),
                ("INSERT INTO events (licence_id, at, what, detail, actor) VALUES (NULL, ?1, 'setting', ?2, ?3)", vec![self.now.into(), format!("renewals: {offered}").into(), actor.into()]),
            ])
            .await
            .map(|_| ())
    }

    /// Starts a renewal of `licence_id`: returns the token for `/renew?t=`. The caller has already
    /// checked who is asking (signed-in owner, or the app's live activation).
    pub async fn start_renewal(&self, licence_id: &str, source: &str, activation_id: Option<&str>) -> Result<Result<String, StartRefusal>, String> {
        if self.renewal_settings().await?.offered().is_none() {
            return Ok(Err(StartRefusal::NotOffered));
        }
        let Some(row) = self.licence(licence_id).await? else { return Ok(Err(StartRefusal::NotFound)) };
        let until = match row.lic.validity {
            Validity::Perpetual { updates_until } => updates_until,
            _ => return Ok(Err(StartRefusal::Refused(RenewRefusal::Temporary))),
        };
        if let Err(r) = maintenance::can_renew(&row.lic, until, self.now, None) {
            return Ok(Err(StartRefusal::Refused(r)));
        }
        let t = token(&self.rng);
        self.db
            .run(
                "INSERT INTO renewal_tokens (token_hash, licence_id, source, activation_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                vec![token_hash(&t).into(), licence_id.into(), source.into(), activation_id.into(), self.now.into(), (self.now + TOKEN_LIFE).into()],
            )
            .await?;
        Ok(Ok(t))
    }

    /// The licence a renewal token is for. `for_payment`: a payment may arrive up to
    /// [`TOKEN_PAYMENT_GRACE`] after the link was made; opening the checkout needs a live link.
    pub async fn renewal_token(&self, t: &str, for_payment: bool) -> Result<Option<Pending>, String> {
        if t.is_empty() || t.len() > 64 {
            return Ok(None);
        }
        let Some(r) = self.db.first("SELECT licence_id, created_at, expires_at FROM renewal_tokens WHERE token_hash = ?1", vec![token_hash(t).into()]).await? else { return Ok(None) };
        let created = crate::db::int(&r, "created_at").unwrap_or(0);
        let expires_at = crate::db::int(&r, "expires_at").unwrap_or(0);
        let live = if for_payment { self.now < created + TOKEN_PAYMENT_GRACE } else { self.now < expires_at };
        if !live {
            return Ok(None);
        }
        let id = crate::db::req_text(&r, "licence_id")?;
        let Some(row) = self.licence(&id).await? else { return Ok(None) };
        let Validity::Perpetual { updates_until } = row.lic.validity else { return Ok(None) };
        Ok(Some(Pending { licence_id: id, serial_hint: row.lic.serial_hint.clone(), until: updates_until, email: row.lic.email.clone(), expires_at }))
    }

    /// A completed Paddle transaction that bought `qty` renewals. Returns the outcome text the
    /// webhook records; a problem starts with "renewal problem" and shows on the admin Renewals
    /// page (and, when the licence is known, in its history) for a manual fix or refund.
    pub async fn paid_renewal(&self, data: &serde_json::Value, qty: u32) -> Result<String, String> {
        let txn = data["id"].as_str().unwrap_or_default();
        let settings = self.renewal_settings().await?;
        let t = data["custom_data"]["renewal"].as_str().unwrap_or_default();
        let Some(p) = self.renewal_token(t, true).await? else {
            return Ok(format!("renewal problem: {txn} has no valid renewal token, so no licence was extended"));
        };
        let currency = data["currency_code"].as_str().unwrap_or_default().to_string();
        let total = data["details"]["totals"]["grand_total"].as_str().map(|m| money(m, &currency));
        let months = settings.months() * qty.clamp(1, 10);
        let g = NewGrant { grant: Grant::renewal(self.now, months), txn: Some(txn), amount: total.as_deref(), currency: Some(currency.as_str()).filter(|c| !c.is_empty()), actor: "paddle", reason: "" };
        let o = self.add_grant(&p.licence_id, &g, None).await?;
        self.db.run("UPDATE renewal_tokens SET last_txn = ?1 WHERE token_hash = ?2", vec![txn.into(), token_hash(t).into()]).await?;
        Ok(match o {
            Outcome::Applied { after, .. } => {
                self.renewal_confirmation(&p.licence_id, after).await;
                format!("{txn}: renewed {} until {}", p.licence_id, crate::web::date(after))
            }
            Outcome::AlreadyDone => format!("{txn}: renewal already applied"),
            Outcome::Refused(r) => {
                let why = format!("renewal problem: {txn} paid for {} but it can't be renewed ({}). Refund it in Paddle.", p.licence_id, r.message());
                self.db.run("INSERT INTO events (licence_id, at, what, detail, actor) VALUES (?1, ?2, 'renewal problem', ?3, 'paddle')", vec![(&p.licence_id).into(), self.now.into(), (&why).into()]).await?;
                why
            }
            Outcome::NotFound | Outcome::NotReversible => format!("renewal problem: {txn}: licence {} not found", p.licence_id),
        })
    }

    async fn renewal_confirmation(&self, licence_id: &str, until: Ts) {
        let Ok(Some(row)) = self.licence(licence_id).await else { return };
        let Some(to) = row.lic.email.clone() else { return };
        if let Err(e) = crate::mail::send(self, &crate::mail::renewal_message(&row, until, &to)).await {
            eprintln!("renewal confirmation: {e}");
        }
    }

    /// Renewals for the admin Renewals page: newest first.
    pub async fn recent_renewals(&self, limit: u32) -> Result<Vec<crate::db::Row>, String> {
        self.db
            .all(
                &format!(
                    "SELECT g.licence_id, g.at, g.months, g.txn, g.amount, g.currency, g.before_until, g.after_until, g.reversed_at, g.reverse_reason, l.serial_hint, l.email \
                     FROM maintenance_grants g JOIN licences l ON l.id = g.licence_id WHERE g.kind = 'renewal' ORDER BY g.at DESC LIMIT {}",
                    limit.min(500)
                ),
                vec![],
            )
            .await
    }

    /// Paid renewals the server couldn't apply.
    pub async fn renewal_problems(&self, limit: u32) -> Result<Vec<crate::db::Row>, String> {
        self.db
            .all(&format!("SELECT event_id, received_at, outcome FROM webhooks WHERE outcome LIKE 'renewal problem%' ORDER BY received_at DESC LIMIT {}", limit.min(200)), vec![])
            .await
    }
}

/// What a renewal would make the end date, for the checkout page.
pub fn preview(until: Ts, now: Ts, months: u32) -> Ts {
    maintenance::apply(until, &Grant::renewal(now, months))
}
