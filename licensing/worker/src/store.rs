//! Licences and activations in D1, applying the core's rules.
//!
//! Activation is the one place two requests can race for the last seat. The core decides from the
//! licence and its activations ([`rules::decide`], which counts seats in use); the result is one
//! batch (one transaction): an `UPDATE licences … WHERE activations_used = <as read>` (every new
//! activation bumps that history counter, so it works as a version number) and an
//! `INSERT … WHERE changes() = 1`. If the update changed nothing, another activation got in first
//! (or the licence changed), so the request is decided again from fresh rows. A deactivation in
//! between only frees a seat, so deciding on slightly old rows can never over-allocate.

use crate::db::{self, ACTIVATION_COLS, ActivationRow, Db, LICENCE_COLS, LicenceRow, Stmt, V};
use crate::{Ctx, Net, secret};
use filescape_licence::model::*;
use filescape_licence::rules::{self, ActivationReportRow, Decision, IssueError, TemporaryOptions};
use filescape_licence::{Ts, serial};

/// Why a request failed: a rule said no (sent to the app), or something broke.
#[derive(Debug)]
pub enum Fail {
    No(Rejection),
    /// A rejection the app acts on (revoked, deactivated), with the signed statement it needs to.
    Signed(Rejection, filescape_licence::sign::Envelope),
    Err(String),
}

impl From<String> for Fail {
    fn from(e: String) -> Fail {
        Fail::Err(e)
    }
}

impl From<&str> for Fail {
    fn from(e: &str) -> Fail {
        Fail::Err(e.into())
    }
}

impl From<Rejection> for Fail {
    fn from(r: Rejection) -> Fail {
        Fail::No(r)
    }
}

pub fn event_stmt(licence_id: &str, at: Ts, what: &str, detail: &str, actor: &str) -> Stmt {
    ("INSERT INTO events (licence_id, at, what, detail, actor) VALUES (?1, ?2, ?3, ?4, ?5)", vec![licence_id.into(), at.into(), what.into(), detail.into(), actor.into()])
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub at: Ts,
    pub what: String,
    pub detail: String,
    pub actor: Option<String>,
}

/// Admin list filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListFilter {
    #[default]
    All,
    Temporary,
    Perpetual,
    /// Temporary licences ending in the next 7 days.
    ExpiringSoon,
    Revoked,
    /// Paddle reported a refund or chargeback.
    Refunds,
    /// Paid but the email didn't go out.
    NotEmailed,
}

impl<D: Db, N: Net> Ctx<'_, D, N> {
    // ---- Reading ----

    pub async fn licence(&self, id: &str) -> Result<Option<LicenceRow>, String> {
        let sql = format!("SELECT {LICENCE_COLS} FROM licences WHERE id = ?1");
        self.db.first(&sql, vec![id.into()]).await?.map(|r| db::licence(&r)).transpose()
    }

    pub async fn licence_by_serial(&self, canonical: &str) -> Result<Option<LicenceRow>, String> {
        let sql = format!("SELECT {LICENCE_COLS} FROM licences WHERE serial_hash = ?1");
        self.db.first(&sql, vec![serial::hash(canonical).into()]).await?.map(|r| db::licence(&r)).transpose()
    }

    pub async fn licence_by_order(&self, order_ref: &str) -> Result<Option<LicenceRow>, String> {
        let sql = format!("SELECT {LICENCE_COLS} FROM licences WHERE order_ref = ?1");
        self.db.first(&sql, vec![order_ref.into()]).await?.map(|r| db::licence(&r)).transpose()
    }

    pub async fn licences_by_email(&self, email: &str) -> Result<Vec<LicenceRow>, String> {
        let sql = format!("SELECT {LICENCE_COLS} FROM licences WHERE email = ?1 ORDER BY issued_at");
        self.db.all(&sql, vec![email.trim().to_lowercase().into()]).await?.iter().map(db::licence).collect()
    }

    pub async fn activations_of(&self, licence_id: &str) -> Result<Vec<ActivationRow>, String> {
        let sql = format!("SELECT {ACTIVATION_COLS} FROM activations WHERE licence_id = ?1 ORDER BY created_at");
        self.db.all(&sql, vec![licence_id.into()]).await?.iter().map(db::activation).collect()
    }

    pub async fn activation(&self, activation_id: &str) -> Result<Option<ActivationRow>, String> {
        let sql = format!("SELECT {ACTIVATION_COLS} FROM activations WHERE activation_id = ?1");
        self.db.first(&sql, vec![activation_id.into()]).await?.map(|r| db::activation(&r)).transpose()
    }

    pub async fn events_of(&self, licence_id: &str) -> Result<Vec<EventRow>, String> {
        let rows = self.db.all("SELECT at, what, detail, actor FROM events WHERE licence_id = ?1 ORDER BY id DESC", vec![licence_id.into()]).await?;
        Ok(rows
            .iter()
            .map(|r| EventRow { at: db::int(r, "at").unwrap_or(0), what: db::text(r, "what").unwrap_or_default(), detail: db::text(r, "detail").unwrap_or_default(), actor: db::text(r, "actor") })
            .collect())
    }

    pub async fn list(&self, text: &str, filter: ListFilter, limit: u32) -> Result<Vec<LicenceRow>, String> {
        let t = text.trim();
        let mut wh: Vec<String> = Vec::new();
        let mut args: Vec<V> = Vec::new();
        if !t.is_empty() {
            let by_serial = serial::normalize(t).ok().map(|(_, s)| serial::hash(&s));
            args.push(t.to_lowercase().into());
            args.push(format!("%{}%", t.to_lowercase().replace('%', "")).into());
            args.push(by_serial.into());
            wh.push("(lower(id) = ?1 OR lower(coalesce(owner, '')) LIKE ?2 OR coalesce(email, '') LIKE ?2 OR lower(coalesce(order_ref, '')) = ?1 OR serial_hash = ?3)".into());
        }
        let n = args.len();
        match filter {
            ListFilter::All => {}
            ListFilter::Temporary => wh.push("kind = 'temporary'".into()),
            ListFilter::Perpetual => wh.push("kind = 'perpetual'".into()),
            ListFilter::ExpiringSoon => {
                args.push(self.now.into());
                args.push((self.now + 7 * filescape_licence::DAY).into());
                wh.push(format!("revoked_at IS NULL AND expires_at >= ?{} AND expires_at < ?{}", n + 1, n + 2));
            }
            ListFilter::Revoked => wh.push("revoked_at IS NOT NULL".into()),
            ListFilter::Refunds => wh.push("refund_flag IS NOT NULL".into()),
            ListFilter::NotEmailed => wh.push("order_ref IS NOT NULL AND emailed_at IS NULL".into()),
        }
        let order = if filter == ListFilter::ExpiringSoon { "expires_at" } else { "issued_at DESC" };
        let sql = format!(
            "SELECT {LICENCE_COLS} FROM licences {} ORDER BY {order} LIMIT {}",
            if wh.is_empty() { String::new() } else { format!("WHERE {}", wh.join(" AND ")) },
            limit.min(500)
        );
        self.db.all(&sql, args).await?.iter().map(db::licence).collect()
    }

    /// Counts for the admin home page: (licences, perpetual, temporary active, activations).
    pub async fn counts(&self) -> Result<[i64; 4], String> {
        let r = self
            .db
            .first(
                "SELECT (SELECT count(*) FROM licences) AS a, (SELECT count(*) FROM licences WHERE kind = 'perpetual') AS b, \
                 (SELECT count(*) FROM licences WHERE kind = 'temporary' AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?1)) AS c, \
                 (SELECT count(*) FROM activations WHERE ended_at IS NULL AND voided_at IS NULL) AS d",
                vec![self.now.into()],
            )
            .await?
            .unwrap_or_default();
        Ok(["a", "b", "c", "d"].map(|k| db::int(&r, k).unwrap_or(0)))
    }

    // ---- Settings ----

    pub async fn default_perpetual_limit(&self) -> Result<u32, String> {
        let r = self.db.first("SELECT value FROM settings WHERE key = 'perpetual_limit'", vec![]).await?;
        Ok(r.and_then(|r| db::text(&r, "value")).and_then(|v| v.parse().ok()).unwrap_or(filescape_licence::DEFAULT_PERPETUAL_ACTIVATIONS))
    }

    pub async fn set_default_perpetual_limit(&self, max: u32, actor: &str) -> Result<Result<(), IssueError>, String> {
        if max == 0 || max > 1000 {
            return Ok(Err(IssueError::BadLimit));
        }
        self.db
            .batch(vec![
                ("INSERT INTO settings (key, value) VALUES ('perpetual_limit', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", vec![max.to_string().into()]),
                ("INSERT INTO events (licence_id, at, what, detail, actor) VALUES (NULL, ?1, 'setting', ?2, ?3)", vec![self.now.into(), format!("perpetual limit for new purchases: {max}").into(), actor.into()]),
            ])
            .await?;
        Ok(Ok(()))
    }

    // ---- Issuing ----

    fn insert_stmt(&self, lic: &Licence, serial: &str) -> Stmt {
        (
            "INSERT INTO licences (id, serial_hash, serial_hint, serial_enc, kind, validity, owner, email, purpose, issued_at, first_activated_at, expires_at, max_activations, activations_used, order_ref) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?12, 0, ?13)",
            vec![
                (&lic.id).into(),
                serial::hash(serial).into(),
                serial::hint(serial).into(),
                secret::seal(&self.cfg.serial_key, &self.rng, serial).into(),
                kind_str(lic.kind()).into(),
                serde_json::to_string(&lic.validity).unwrap().into(),
                lic.owner.clone().into(),
                lic.email.as_ref().map(|e| e.trim().to_lowercase()).into(),
                lic.purpose.clone().into(),
                lic.issued_at.into(),
                lic.expires_at.into(),
                lic.max_activations.into(),
                lic.order_ref.clone().into(),
            ],
        )
    }

    /// Create Temporary Licence. The serials are shown once (and emailed if an address is given).
    pub async fn issue_temporary(&self, opts: &TemporaryOptions, actor: &str) -> Result<Result<Vec<(String, Licence)>, IssueError>, String> {
        if let Err(e) = rules::check_temporary(opts, self.now) {
            return Ok(Err(e));
        }
        let mut out = Vec::new();
        let mut stmts = Vec::new();
        for _ in 0..opts.count.clamp(1, 100) {
            let serial = serial::generate(LicenceKind::Temporary, &self.rng);
            let mut lic = rules::new_licence(&self.rng, opts.validity, opts.max_activations, self.now);
            lic.serial_hash = serial::hash(&serial);
            lic.serial_hint = serial::hint(&serial);
            lic.owner = opts.owner.clone();
            lic.email = opts.email.as_ref().map(|e| e.trim().to_lowercase());
            lic.purpose = opts.purpose.clone();
            stmts.push(self.insert_stmt(&lic, &serial));
            stmts.push(event_stmt(&lic.id, self.now, "issued", &describe(&lic.validity, lic.max_activations), actor));
            out.push((serial, lic));
        }
        self.db.batch(stmts).await?;
        Ok(Ok(out))
    }

    /// A paid licence. `order_ref` (the Paddle transaction) makes this idempotent: a second call
    /// returns the licence the first one made. Returns the serial and the row.
    pub async fn issue_perpetual(&self, owner: Option<String>, email: Option<String>, order_ref: &str, actor: &str) -> Result<(String, LicenceRow), String> {
        if let Some(row) = self.licence_by_order(order_ref).await? {
            let serial = row.serial_enc.as_deref().and_then(|s| secret::open(&self.cfg.serial_key, s)).ok_or("serial can't be decrypted (SERIAL_KEY changed?)")?;
            return Ok((serial, row));
        }
        let months = self.cfg.paddle.as_ref().map_or(filescape_licence::DEFAULT_UPDATE_MONTHS, |p| p.update_months);
        let limit = self.default_perpetual_limit().await?;
        let serial = serial::generate(LicenceKind::Perpetual, &self.rng);
        let mut lic = rules::new_licence(&self.rng, Validity::Perpetual { updates_until: rules::updates_until(self.now, months) }, limit, self.now);
        lic.serial_hash = serial::hash(&serial);
        lic.serial_hint = serial::hint(&serial);
        lic.owner = owner;
        lic.email = email.map(|e| e.trim().to_lowercase());
        lic.order_ref = Some(order_ref.into());
        let ins = self.insert_stmt(&lic, &serial);
        let ev = event_stmt(&lic.id, self.now, "issued", &format!("perpetual, {limit} computers at once, order {order_ref}"), actor);
        match self.db.batch(vec![ins, ev]).await {
            Ok(_) => {}
            // Two deliveries of the same order at once: the other one won.
            Err(e) if e.contains("UNIQUE") => {
                let row = self.licence_by_order(order_ref).await?.ok_or(e)?;
                let serial = row.serial_enc.as_deref().and_then(|s| secret::open(&self.cfg.serial_key, s)).ok_or("serial can't be decrypted")?;
                return Ok((serial, row));
            }
            Err(e) => return Err(e),
        }
        let row = self.licence(&lic.id).await?.ok_or("licence vanished")?;
        Ok((serial, row))
    }

    /// Fill in the buyer's name and email where the licence has none yet (a paid licence is issued
    /// before the Paddle customer lookup).
    pub async fn fill_buyer(&self, licence_id: &str, owner: Option<&str>, email: Option<&str>) -> Result<(), String> {
        let o = owner.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let e = email.map(|e| e.trim().to_lowercase()).filter(|s| !s.is_empty());
        self.db
            .run("UPDATE licences SET owner = coalesce(owner, ?1), email = coalesce(email, ?2) WHERE id = ?3", vec![o.into(), e.into(), licence_id.into()])
            .await
            .map(|_| ())
    }

    pub fn serial_of(&self, row: &LicenceRow) -> Option<String> {
        row.serial_enc.as_deref().and_then(|s| secret::open(&self.cfg.serial_key, s))
    }

    pub async fn mark_emailed(&self, licence_id: &str, to: &str, actor: &str) -> Result<(), String> {
        self.db
            .batch(vec![("UPDATE licences SET emailed_at = ?1 WHERE id = ?2", vec![self.now.into(), licence_id.into()]), event_stmt(licence_id, self.now, "emailed", to, actor)])
            .await
            .map(|_| ())
    }

    // ---- The app's API ----

    /// POST /v1/activate, and offline activation.
    pub async fn activate(&self, req: &ActivationRequest) -> Result<ActivationResponse, Fail> {
        let (_, canonical) = serial::normalize(&req.serial).map_err(|_| Rejection::BadSerial)?;
        if req.activation_id.len() < 8 || req.activation_id.len() > 100 || req.device_hash.len() != 64 {
            return Err(Fail::No(Rejection::BadSerial));
        }
        for _ in 0..4 {
            let row = self.licence_by_serial(&canonical).await?.ok_or(Rejection::UnknownSerial)?;
            let lic = row.lic;
            let sql = format!("SELECT {ACTIVATION_COLS} FROM activations WHERE licence_id = ?1 OR activation_id = ?2");
            let acts: Vec<Activation> = self.db.all(&sql, vec![(&lic.id).into(), (&req.activation_id).into()]).await?.iter().map(|r| db::activation(r).map(|a| a.a)).collect::<Result<_, _>>()?;
            match rules::decide(&lic, &acts, req, self.now)? {
                Decision::Reissue { activation_id } => {
                    self.db
                        .run("UPDATE activations SET last_seen_at = ?1, app_version = ?2 WHERE activation_id = ?3", vec![self.now.into(), (&req.app_version).into(), (&activation_id).into()])
                        .await?;
                    let a = acts.iter().find(|a| a.activation_id == activation_id).ok_or("activation vanished")?;
                    return Ok(self.respond(&lic, &acts, a, false));
                }
                Decision::NewUse { first_activation } => {
                    let first_expiry = first_activation.and_then(|(_, e)| e);
                    let changes = self
                        .db
                        .batch(vec![
                            (
                                "UPDATE licences SET activations_used = activations_used + 1, \
                                 first_activated_at = coalesce(first_activated_at, ?1), \
                                 expires_at = CASE WHEN first_activated_at IS NULL AND ?2 IS NOT NULL THEN ?2 ELSE expires_at END \
                                 WHERE id = ?3 AND activations_used = ?4 AND max_activations = ?5 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?1)",
                                vec![self.now.into(), first_expiry.into(), (&lic.id).into(), lic.activations_used.into(), lic.max_activations.into()],
                            ),
                            (
                                "INSERT INTO activations (activation_id, licence_id, device_hash, app_version, created_at, counted, last_seen_at, os) \
                                 SELECT ?1, ?2, ?3, ?4, ?5, 1, ?5, ?6 WHERE changes() = 1",
                                vec![(&req.activation_id).into(), (&lic.id).into(), (&req.device_hash).into(), (&req.app_version).into(), self.now.into(), clean_os(req.os.as_deref()).into()],
                            ),
                            (
                                "INSERT INTO events (licence_id, at, what, detail, actor) SELECT ?1, ?2, 'activated', ?3, 'app' WHERE changes() = 1",
                                vec![(&lic.id).into(), self.now.into(), format!("{} ({} of {})", req.activation_id, rules::seats_in_use(&lic, &acts, self.now) + 1, lic.max_activations).into()],
                            ),
                        ])
                        .await;
                    match changes {
                        Ok(c) if c.first() == Some(&1) && c.get(1) == Some(&1) => {
                            let row = self.licence(&lic.id).await?.ok_or("licence vanished")?;
                            let acts = self.acts_of(&lic.id).await?;
                            let a = acts.iter().find(|a| a.activation_id == req.activation_id).ok_or("activation vanished")?;
                            return Ok(self.respond(&row.lic, &acts, a, true));
                        }
                        // The licence changed under us (another activation took a use): decide again.
                        Ok(_) => continue,
                        // Same activation ID inserted at the same moment: decide again (re-issue).
                        Err(e) if e.contains("UNIQUE") || e.contains("constraint") => continue,
                        Err(e) => return Err(Fail::Err(e)),
                    }
                }
            }
        }
        Err(Fail::Err("too much contention activating this licence".into()))
    }

    async fn acts_of(&self, licence_id: &str) -> Result<Vec<Activation>, String> {
        Ok(self.activations_of(licence_id).await?.into_iter().map(|r| r.a).collect())
    }

    /// Seats the licence is using now ([`rules::seats_in_use`]).
    pub async fn seats(&self, lic: &Licence) -> Result<u32, String> {
        Ok(rules::seats_in_use(lic, &self.acts_of(&lic.id).await?, self.now))
    }

    fn respond(&self, lic: &Licence, acts: &[Activation], a: &Activation, consumed: bool) -> ActivationResponse {
        let used = rules::seats_in_use(lic, acts, self.now);
        let (activation, issuance) = rules::certificates(&self.cfg.signer, lic, a, used, self.now);
        ActivationResponse { activation, issuance, status: lic.status(self.now), consumed_use: consumed }
    }

    /// POST /v1/validate: the weekly check.
    pub async fn validate(&self, activation_id: &str, device_hash: &str) -> Result<ActivationResponse, Fail> {
        let a = self.activation(activation_id).await?.ok_or(Rejection::NotFound)?.a;
        let lic = self.licence(&a.licence_id).await?.ok_or(Rejection::NotFound)?.lic;
        if let Err(why) = rules::check_validate(&lic, &a, device_hash, self.now) {
            // Only for the device that owns the activation: anyone else gets the bare rejection.
            return Err(match rules::status_statement(&self.cfg.signer, &lic, &a, why, self.now) {
                Some(env) if a.device_hash == device_hash => Fail::Signed(why, env),
                _ => Fail::No(why),
            });
        }
        self.db.run("UPDATE activations SET last_seen_at = ?1 WHERE activation_id = ?2", vec![self.now.into(), activation_id.into()]).await?;
        let acts = self.acts_of(&lic.id).await?;
        Ok(self.respond(&lic, &acts, &a, false))
    }

    /// POST /v1/deactivate. A perpetual licence's seat is freed (subject to the release cooldown);
    /// a temporary licence's use isn't given back.
    pub async fn deactivate(&self, activation_id: &str, device_hash: &str) -> Result<(), Fail> {
        let a = self.activation(activation_id).await?.ok_or(Rejection::NotFound)?.a;
        if a.device_hash != device_hash {
            return Err(Fail::No(Rejection::ActivationIdConflict));
        }
        self.release(&a, "app").await?;
        Ok(())
    }

    /// Ends an activation by its owner's choice (`via`: "app" or "account").
    pub async fn release(&self, a: &Activation, via: &str) -> Result<(), String> {
        if a.ended_at.is_none() && a.voided.is_none() {
            self.db
                .batch(vec![
                    ("UPDATE activations SET ended_at = ?1, released_via = ?2 WHERE activation_id = ?3 AND ended_at IS NULL", vec![self.now.into(), via.into(), (&a.activation_id).into()]),
                    event_stmt(&a.licence_id, self.now, "deactivated", &a.activation_id, via),
                ])
                .await?;
        }
        Ok(())
    }

    // ---- Admin ----

    pub async fn revoke(&self, licence_id: &str, reason: &str, actor: &str) -> Result<(), String> {
        self.db
            .batch(vec![
                ("UPDATE licences SET revoked_at = ?1, revoke_reason = ?2 WHERE id = ?3 AND revoked_at IS NULL", vec![self.now.into(), reason.into(), licence_id.into()]),
                event_stmt(licence_id, self.now, "revoked", reason, actor),
            ])
            .await
            .map(|_| ())
    }

    /// Undo a revoke (for a mistake). Existing activations work again at their next check.
    pub async fn unrevoke(&self, licence_id: &str, reason: &str, actor: &str) -> Result<(), String> {
        self.db
            .batch(vec![
                ("UPDATE licences SET revoked_at = NULL, revoke_reason = NULL WHERE id = ?1", vec![licence_id.into()]),
                event_stmt(licence_id, self.now, "reinstated", reason, actor),
            ])
            .await
            .map(|_| ())
    }

    /// Reset activations: a temporary licence gets every use back; a perpetual licence's release
    /// cooldown is cleared. Machines already activated keep working (and keep their seats).
    pub async fn reset_activations(&self, licence_id: &str, reason: &str, actor: &str) -> Result<(), String> {
        let before = self.licence(licence_id).await?.map_or(0, |r| r.lic.activations_used);
        self.db
            .batch(vec![
                ("UPDATE activations SET counted = 0 WHERE licence_id = ?1", vec![licence_id.into()]),
                ("UPDATE licences SET activations_used = 0 WHERE id = ?1", vec![licence_id.into()]),
                event_stmt(licence_id, self.now, "reset", &format!("{before} use(s) given back: {reason}"), actor),
            ])
            .await
            .map(|_| ())
    }

    /// Cancel one activation (made in error), optionally giving its use back.
    pub async fn void_activation(&self, licence_id: &str, activation_id: &str, give_back: bool, reason: &str, actor: &str) -> Result<(), String> {
        let Some(a) = self.activation(activation_id).await? else { return Ok(()) };
        if a.a.licence_id != licence_id {
            return Err("that activation belongs to another licence".into());
        }
        let mut stmts: Vec<Stmt> = vec![(
            "UPDATE activations SET voided_at = coalesce(voided_at, ?1), void_reason = coalesce(void_reason, ?2) WHERE activation_id = ?3",
            vec![self.now.into(), reason.into(), activation_id.into()],
        )];
        if give_back && a.a.counted {
            stmts.push(("UPDATE activations SET counted = 0 WHERE activation_id = ?1", vec![activation_id.into()]));
            stmts.push(("UPDATE licences SET activations_used = max(activations_used - 1, 0) WHERE id = ?1", vec![licence_id.into()]));
        }
        stmts.push(event_stmt(licence_id, self.now, "voided", &format!("{activation_id}: {reason}{}", if give_back { " (use given back)" } else { "" }), actor));
        self.db.batch(stmts).await.map(|_| ())
    }

    /// Change a perpetual licence's limit.
    pub async fn set_limit(&self, licence_id: &str, max: u32, reason: &str, actor: &str) -> Result<Result<(), IssueError>, String> {
        if max == 0 || max > 1000 {
            return Ok(Err(IssueError::BadLimit));
        }
        let Some(row) = self.licence(licence_id).await? else { return Ok(Err(IssueError::NotFound)) };
        if row.lic.kind() != LicenceKind::Perpetual {
            return Ok(Err(IssueError::NotPerpetual));
        }
        self.db
            .batch(vec![
                ("UPDATE licences SET max_activations = ?1 WHERE id = ?2", vec![max.into(), licence_id.into()]),
                event_stmt(licence_id, self.now, "limit changed", &format!("{} to {max}: {reason}", row.lic.max_activations), actor),
            ])
            .await?;
        Ok(Ok(()))
    }

    pub async fn set_owner(&self, licence_id: &str, owner: &str, email: &str, actor: &str) -> Result<(), String> {
        let o = Some(owner.trim()).filter(|s| !s.is_empty());
        let e = Some(email.trim().to_lowercase()).filter(|s| !s.is_empty());
        self.db
            .batch(vec![
                ("UPDATE licences SET owner = ?1, email = ?2 WHERE id = ?3", vec![o.into(), e.clone().into(), licence_id.into()]),
                event_stmt(licence_id, self.now, "details changed", &format!("owner {}, email {}", o.unwrap_or("(none)"), e.as_deref().unwrap_or("(none)")), actor),
            ])
            .await
            .map(|_| ())
    }

    pub async fn clear_refund_flag(&self, licence_id: &str, actor: &str) -> Result<(), String> {
        self.db
            .batch(vec![("UPDATE licences SET refund_flag = NULL WHERE id = ?1", vec![licence_id.into()]), event_stmt(licence_id, self.now, "refund flag cleared", "", actor)])
            .await
            .map(|_| ())
    }

    /// The Activations page: every activated licence, flagged ones first.
    pub async fn report(&self) -> Result<Vec<ActivationReportRow>, String> {
        let lics: Vec<LicenceRow> = self.db.all(&format!("SELECT {LICENCE_COLS} FROM licences WHERE first_activated_at IS NOT NULL"), vec![]).await?.iter().map(db::licence).collect::<Result<_, _>>()?;
        let acts: Vec<ActivationRow> = self.db.all(&format!("SELECT {ACTIVATION_COLS} FROM activations"), vec![]).await?.iter().map(db::activation).collect::<Result<_, _>>()?;
        let since = rules::report_since(self.now);
        let mut rows: Vec<ActivationReportRow> = lics
            .iter()
            .map(|l| {
                let mine: Vec<&Activation> = acts.iter().filter(|a| a.a.licence_id == l.lic.id).map(|a| &a.a).collect();
                rules::report_row(&l.lic, &mine, since)
            })
            .collect();
        rules::sort_report(&mut rows);
        Ok(rows)
    }

    /// Whether this address was mailed (for `kind`) in the last `within` seconds; records it if not.
    pub async fn mail_allowed(&self, email: &str, kind: &str, within: Ts) -> Result<bool, String> {
        let r = self.db.first("SELECT count(*) AS n FROM mail_log WHERE email = ?1 AND kind = ?2 AND at > ?3", vec![email.into(), kind.into(), (self.now - within).into()]).await?;
        if r.and_then(|r| db::int(&r, "n")).unwrap_or(0) > 0 {
            return Ok(false);
        }
        self.db.run("INSERT INTO mail_log (email, at, kind) VALUES (?1, ?2, ?3)", vec![email.into(), self.now.into(), kind.into()]).await?;
        Ok(true)
    }
}

pub fn kind_str(k: LicenceKind) -> &'static str {
    match k {
        LicenceKind::Temporary => "temporary",
        LicenceKind::Perpetual => "perpetual",
    }
}

pub fn describe(v: &Validity, max: u32) -> String {
    match v {
        Validity::FirstActivation { days } => format!("temporary, {days} days from first activation, {max} activations"),
        Validity::FixedExpiry { expires_at } => format!("temporary, until {}, {max} activations", filescape_licence::export::date_time(*expires_at)),
        Validity::Perpetual { updates_until } => format!("perpetual, updates until {}, {max} computers at once", filescape_licence::export::date_time(*updates_until)),
    }
}

/// The operating system name the app sent, if it's one we know (it's shown on the account page).
pub fn clean_os(os: Option<&str>) -> Option<String> {
    os.filter(|o| matches!(*o, "Windows" | "macOS" | "Linux")).map(str::to_string)
}
