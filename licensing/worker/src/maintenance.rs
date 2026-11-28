//! Software maintenance in D1: the ledger behind each perpetual licence's "updates included until"
//! date ([`filescape_licence::maintenance`] holds the rules).
//!
//! Every change appends a grant (or marks one reversed) and replays the ledger. The licence's date
//! is written in the same batch, with a compare-and-set on its `validity` as read, so two changes
//! at once can't both apply to the same starting point: the loser re-reads and tries again. A paid
//! renewal carries its payment's transaction ID, which is unique in the ledger, so a notification
//! delivered twice (even at the same moment) extends maintenance once.
//!
//! The date lives in `validity.updates_until`, which every certificate the server signs carries, so
//! apps already installed pick up a change at their next weekly check, with no app update.

use crate::db::{self, Row, Stmt};
use crate::{Ctx, Db, Net};
use filescape_licence::Ts;
use filescape_licence::export::date_time;
use filescape_licence::maintenance::{self, Grant, GrantKind, RenewRefusal};
use filescape_licence::model::{LicenceKind, Validity};

const TRIES: usize = 5;

/// A ledger row as the admin and account pages show it.
#[derive(Debug, Clone)]
pub struct GrantRow {
    pub id: i64,
    pub grant: Grant,
    pub txn: Option<String>,
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub actor: String,
    pub reason: Option<String>,
    pub before_until: Option<Ts>,
    pub after_until: Option<Ts>,
    pub reversed_at: Option<Ts>,
    pub reversed_by: Option<String>,
    pub reverse_reason: Option<String>,
}

const GRANT_COLS: &str = "id, kind, at, months, days, until, txn, amount, currency, actor, reason, before_until, after_until, reversed_at, reversed_by, reverse_reason";

fn grant_row(r: &Row) -> Result<GrantRow, String> {
    let kind = GrantKind::parse(&db::req_text(r, "kind")?).ok_or("unknown grant kind")?;
    let reversed_at = db::int(r, "reversed_at");
    Ok(GrantRow {
        id: db::int(r, "id").ok_or("grant without id")?,
        grant: Grant {
            kind,
            at: db::int(r, "at").unwrap_or(0),
            months: db::int(r, "months").unwrap_or(0) as u32,
            days: db::int(r, "days").unwrap_or(0) as u32,
            until: db::int(r, "until"),
            reversed: reversed_at.is_some(),
        },
        txn: db::text(r, "txn"),
        amount: db::text(r, "amount"),
        currency: db::text(r, "currency"),
        actor: db::text(r, "actor").unwrap_or_default(),
        reason: db::text(r, "reason"),
        before_until: db::int(r, "before_until"),
        after_until: db::int(r, "after_until"),
        reversed_at,
        reversed_by: db::text(r, "reversed_by"),
        reverse_reason: db::text(r, "reverse_reason"),
    })
}

/// What a grant or a reversal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The end date moved (or stayed, for a reversal that changed nothing).
    Applied { before: Ts, after: Ts },
    /// That transaction was already applied (or that grant already reversed).
    AlreadyDone,
    Refused(RenewRefusal),
    /// No such licence or grant, or a temporary licence (which has no maintenance).
    NotFound,
    /// The grant can't be reversed by an admin (a purchase or a paid renewal).
    NotReversible,
}

/// What to record with a new grant.
pub struct NewGrant<'a> {
    pub grant: Grant,
    pub txn: Option<&'a str>,
    pub amount: Option<&'a str>,
    pub currency: Option<&'a str>,
    pub actor: &'a str,
    pub reason: &'a str,
}

fn day(t: Ts) -> String {
    date_time(t).split(',').next().unwrap_or_default().to_string()
}

/// "+12 months", "+30 days", "+1 month 15 days".
pub fn length(months: u32, days: u32) -> String {
    let mut parts = Vec::new();
    if months > 0 {
        parts.push(format!("{months} month{}", if months == 1 { "" } else { "s" }));
    }
    if days > 0 {
        parts.push(format!("{days} day{}", if days == 1 { "" } else { "s" }));
    }
    if parts.is_empty() { "nothing".into() } else { format!("+{}", parts.join(" ")) }
}

impl<D: Db, N: Net> Ctx<'_, D, N> {
    /// A licence's ledger, oldest first.
    pub async fn grants_of(&self, licence_id: &str) -> Result<Vec<GrantRow>, String> {
        let sql = format!("SELECT {GRANT_COLS} FROM maintenance_grants WHERE licence_id = ?1 ORDER BY at, id");
        self.db.all(&sql, vec![licence_id.into()]).await?.iter().map(grant_row).collect()
    }

    pub async fn grant_by_txn(&self, txn: &str) -> Result<Option<(String, GrantRow)>, String> {
        let sql = format!("SELECT licence_id, {GRANT_COLS} FROM maintenance_grants WHERE txn = ?1");
        match self.db.first(&sql, vec![txn.into()]).await? {
            Some(r) => Ok(Some((db::req_text(&r, "licence_id")?, grant_row(&r)?))),
            None => Ok(None),
        }
    }

    /// The purchase grant statement for a perpetual licence being issued.
    pub fn purchase_grant_stmt(&self, licence_id: &str, until: Ts, txn_note: Option<&str>, actor: &str) -> Stmt {
        (
            "INSERT INTO maintenance_grants (licence_id, kind, at, until, actor, reason, after_until) VALUES (?1, 'purchase', ?2, ?3, ?4, ?5, ?3)",
            vec![licence_id.into(), self.now.into(), until.into(), actor.into(), txn_note.map(|o| format!("order {o}")).into()],
        )
    }

    /// Makes sure a perpetual licence has its purchase grant (licences issued between the
    /// migration and this code being deployed might not), using the date it already has.
    async fn ensure_purchase(&self, licence_id: &str, issued_at: Ts, until: Ts) -> Result<(), String> {
        self.db
            .run(
                "INSERT INTO maintenance_grants (licence_id, kind, at, until, actor, reason, after_until) \
                 SELECT ?1, 'purchase', ?2, ?3, 'system', 'existing licence', ?3 WHERE NOT EXISTS (SELECT 1 FROM maintenance_grants WHERE licence_id = ?1 AND kind = 'purchase')",
                vec![licence_id.into(), issued_at.into(), until.into()],
            )
            .await
            .map(|_| ())
    }

    /// Adds a renewal or complimentary grant and moves the licence's end date. `max_ahead_days`
    /// (renewals bought by customers) refuses a renewal once maintenance runs that far ahead.
    pub async fn add_grant(&self, licence_id: &str, g: &NewGrant<'_>, max_ahead_days: Option<u32>) -> Result<Outcome, String> {
        if g.grant.kind == GrantKind::Purchase {
            return Err("purchase grants are made when a licence is issued".into());
        }
        if let Some(t) = g.txn
            && self.grant_by_txn(t).await?.is_some()
        {
            return Ok(Outcome::AlreadyDone);
        }
        for _ in 0..TRIES {
            let Some(row) = self.licence(licence_id).await? else { return Ok(Outcome::NotFound) };
            let lic = &row.lic;
            let Validity::Perpetual { updates_until } = lic.validity else { return Ok(Outcome::Refused(RenewRefusal::Temporary)) };
            if let Err(r) = maintenance::can_renew(lic, updates_until, self.now, if g.grant.kind == GrantKind::Renewal { max_ahead_days } else { None }) {
                return Ok(Outcome::Refused(r));
            }
            self.ensure_purchase(licence_id, lic.issued_at, updates_until).await?;
            let mut ledger: Vec<Grant> = self.grants_of(licence_id).await?.into_iter().map(|r| r.grant).collect();
            let before = maintenance::replay(&ledger).unwrap_or(updates_until);
            ledger.push(g.grant.clone());
            let after = maintenance::replay(&ledger).unwrap_or(before);
            let old_validity = serde_json::to_string(&lic.validity).unwrap();
            let new_validity = serde_json::to_string(&Validity::Perpetual { updates_until: after }).unwrap();
            let detail = format!(
                "{} {}: updates until {} → {}{}{}",
                g.grant.kind.label(),
                length(g.grant.months, g.grant.days),
                day(before),
                day(after),
                g.txn.map(|t| format!(", transaction {t}")).unwrap_or_default(),
                if g.reason.is_empty() { String::new() } else { format!(" ({})", g.reason) }
            );
            let stmts: Vec<Stmt> = vec![
                ("UPDATE licences SET validity = ?1, maintenance_until = ?2 WHERE id = ?3 AND validity = ?4", vec![new_validity.into(), after.into(), licence_id.into(), old_validity.into()]),
                (
                    "INSERT INTO maintenance_grants (licence_id, kind, at, months, days, txn, amount, currency, actor, reason, before_until, after_until) \
                     SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12 WHERE changes() = 1",
                    vec![
                        licence_id.into(),
                        g.grant.kind.as_str().into(),
                        g.grant.at.into(),
                        g.grant.months.into(),
                        g.grant.days.into(),
                        g.txn.into(),
                        g.amount.into(),
                        g.currency.into(),
                        g.actor.into(),
                        Some(g.reason).filter(|r| !r.is_empty()).into(),
                        before.into(),
                        after.into(),
                    ],
                ),
                ("INSERT INTO events (licence_id, at, what, detail, actor) SELECT ?1, ?2, 'maintenance', ?3, ?4 WHERE changes() = 1", vec![licence_id.into(), self.now.into(), detail.into(), g.actor.into()]),
            ];
            match self.db.batch(stmts).await {
                Ok(n) if n.first() == Some(&1) => return Ok(Outcome::Applied { before, after }),
                // The licence changed under us: decide again from fresh rows.
                Ok(_) => continue,
                // The same transaction applied by a parallel delivery.
                Err(e) if e.contains("UNIQUE") => return Ok(Outcome::AlreadyDone),
                Err(e) => return Err(e),
            }
        }
        Err("the licence kept changing; try again".into())
    }

    /// Marks a grant reversed and replays the ledger. `by_provider`: a refund or chargeback, which
    /// may reverse a paid renewal; an admin may only reverse complimentary grants. A purchase is
    /// never reversed here (refunding a purchase revokes the licence instead).
    pub async fn reverse_grant(&self, licence_id: &str, grant_id: i64, by_provider: bool, actor: &str, reason: &str) -> Result<Outcome, String> {
        for _ in 0..TRIES {
            let Some(row) = self.licence(licence_id).await? else { return Ok(Outcome::NotFound) };
            let Validity::Perpetual { updates_until } = row.lic.validity else { return Ok(Outcome::NotFound) };
            let rows = self.grants_of(licence_id).await?;
            let Some(target) = rows.iter().find(|r| r.id == grant_id) else { return Ok(Outcome::NotFound) };
            if target.grant.reversed {
                return Ok(Outcome::AlreadyDone);
            }
            let allowed = match target.grant.kind {
                GrantKind::Purchase => false,
                GrantKind::Renewal => by_provider,
                GrantKind::Complimentary => true,
            };
            if !allowed {
                return Ok(Outcome::NotReversible);
            }
            let mut ledger: Vec<Grant> = rows.iter().map(|r| r.grant.clone()).collect();
            let before = maintenance::replay(&ledger).unwrap_or(updates_until);
            for (g, r) in ledger.iter_mut().zip(&rows) {
                if r.id == grant_id {
                    g.reversed = true;
                }
            }
            let after = maintenance::replay(&ledger).unwrap_or(before);
            let old_validity = serde_json::to_string(&row.lic.validity).unwrap();
            let new_validity = serde_json::to_string(&Validity::Perpetual { updates_until: after }).unwrap();
            let detail = format!("{} reversed: updates until {} → {} ({reason})", target.grant.kind.label(), day(before), day(after));
            let stmts: Vec<Stmt> = vec![
                ("UPDATE licences SET validity = ?1, maintenance_until = ?2 WHERE id = ?3 AND validity = ?4", vec![new_validity.into(), after.into(), licence_id.into(), old_validity.into()]),
                (
                    "UPDATE maintenance_grants SET reversed_at = ?1, reversed_by = ?2, reverse_reason = ?3 WHERE id = ?4 AND reversed_at IS NULL AND changes() = 1",
                    vec![self.now.into(), actor.into(), reason.into(), grant_id.into()],
                ),
                ("INSERT INTO events (licence_id, at, what, detail, actor) SELECT ?1, ?2, 'maintenance', ?3, ?4 WHERE changes() = 1", vec![licence_id.into(), self.now.into(), detail.into(), actor.into()]),
            ];
            let n = self.db.batch(stmts).await?;
            match (n.first(), n.get(1)) {
                (Some(1), Some(1)) => return Ok(Outcome::Applied { before, after }),
                // The licence moved, or the grant was reversed by someone else in between (the
                // date update then needs undoing, which the retry does by replaying again).
                (Some(1), _) => {
                    self.resync(licence_id).await?;
                    return Ok(Outcome::AlreadyDone);
                }
                _ => continue,
            }
        }
        Err("the licence kept changing; try again".into())
    }

    /// A refund or chargeback of the renewal paid by `txn`.
    pub async fn reverse_txn(&self, txn: &str, actor: &str, reason: &str) -> Result<Option<(String, Outcome)>, String> {
        let Some((licence_id, g)) = self.grant_by_txn(txn).await? else { return Ok(None) };
        let o = self.reverse_grant(&licence_id, g.id, true, actor, reason).await?;
        Ok(Some((licence_id, o)))
    }

    /// A reversed chargeback: the renewal paid by `txn` counts again, if a chargeback took it away.
    pub async fn unreverse_txn(&self, txn: &str, actor: &str, reason: &str) -> Result<Option<String>, String> {
        let Some((licence_id, g)) = self.grant_by_txn(txn).await? else { return Ok(None) };
        if g.reversed_at.is_none() || !g.reverse_reason.as_deref().is_some_and(|r| r.starts_with("Paddle chargeback")) {
            return Ok(Some(licence_id));
        }
        self.db
            .batch(vec![
                ("UPDATE maintenance_grants SET reversed_at = NULL, reversed_by = NULL, reverse_reason = NULL WHERE id = ?1", vec![g.id.into()]),
                ("INSERT INTO events (licence_id, at, what, detail, actor) VALUES (?1, ?2, 'maintenance', ?3, ?4)", vec![(&licence_id).into(), self.now.into(), format!("Renewal restored ({reason})").into(), actor.into()]),
            ])
            .await?;
        self.resync(&licence_id).await?;
        Ok(Some(licence_id))
    }

    /// Writes the licence's date from its ledger again (after a race, or as a repair).
    pub async fn resync(&self, licence_id: &str) -> Result<(), String> {
        let Some(row) = self.licence(licence_id).await? else { return Ok(()) };
        let Validity::Perpetual { updates_until } = row.lic.validity else { return Ok(()) };
        let ledger: Vec<Grant> = self.grants_of(licence_id).await?.into_iter().map(|r| r.grant).collect();
        let until = maintenance::replay(&ledger).unwrap_or(updates_until);
        let v = serde_json::to_string(&Validity::Perpetual { updates_until: until }).unwrap();
        self.db.run("UPDATE licences SET validity = ?1, maintenance_until = ?2 WHERE id = ?3", vec![v.into(), until.into(), licence_id.into()]).await.map(|_| ())
    }

    /// The same complimentary extension on several licences (a support gesture, an outage, a
    /// studio's co-terminated renewal). Returns one line per licence.
    pub async fn extend_many(&self, ids: &[String], months: u32, days: u32, actor: &str, reason: &str) -> Result<Vec<(String, Outcome)>, String> {
        let mut out = Vec::new();
        for id in ids {
            let g = NewGrant { grant: Grant::complimentary(self.now, months, days), txn: None, amount: None, currency: None, actor, reason };
            let o = match self.licence(id).await? {
                Some(r) if r.lic.kind() == LicenceKind::Perpetual => self.add_grant(id, &g, None).await?,
                Some(_) => Outcome::Refused(RenewRefusal::Temporary),
                None => Outcome::NotFound,
            };
            out.push((id.clone(), o));
        }
        Ok(out)
    }
}
