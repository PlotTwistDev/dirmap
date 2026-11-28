//! Maintenance reminder emails, sent once a day by the Worker's Cron trigger.
//!
//! For each perpetual licence with an email: 30 days before software maintenance ends, 7 days
//! before, and on the day it ends (up to a week after, in case a run was missed). Only the latest
//! stage due is sent, each at most once per end date ([`reminders_sent`]), so a renewal starts
//! them again. Revoked licences, and addresses that turned reminders off on the account page or
//! with the link in the email, get none.

use crate::db::{self, LicenceRow};
use crate::{Ctx, Db, Net, mail};
use filescape_licence::{DAY, Ts};

/// At most this many emails a run; the rest go the next day.
pub const BATCH: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Days30,
    Days7,
    Ended,
}

impl Stage {
    fn key(self) -> &'static str {
        match self {
            Stage::Days30 => "30",
            Stage::Days7 => "7",
            Stage::Ended => "ended",
        }
    }
}

/// The reminder due for a maintenance end date `until` at `now`, if any.
pub fn due(until: Ts, now: Ts) -> Option<Stage> {
    if now >= until + 7 * DAY {
        None
    } else if now >= until {
        Some(Stage::Ended)
    } else if now >= until - 7 * DAY {
        Some(Stage::Days7)
    } else if now >= until - 30 * DAY {
        Some(Stage::Days30)
    } else {
        None
    }
}

/// The signature in an email's "turn reminders off" link, so it works without signing in.
pub fn unsubscribe_sig(key: &[u8; 32], email: &str) -> String {
    let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    data_encoding::BASE64URL_NOPAD.encode(&ring::hmac::sign(&k, format!("reminders-off:{email}").as_bytes()).as_ref()[..16])
}

pub fn unsubscribe_ok(key: &[u8; 32], email: &str, sig: &str) -> bool {
    let want = unsubscribe_sig(key, email);
    want.len() == sig.len() && want.bytes().zip(sig.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

pub fn message(row: &LicenceRow, until: Ts, stage: Stage, now: Ts, off_link: &str, account: &str) -> mail::Message {
    let hello = row.lic.owner.as_deref().map_or("Hello,".to_string(), |o| format!("Hello {o},"));
    let day = crate::web::date(until).split(',').next().unwrap_or_default().to_string();
    let serial = &row.lic.serial_hint;
    let left = ((until - now + DAY - 1) / DAY).max(1);
    let (subject, lead) = match stage {
        Stage::Days30 | Stage::Days7 => (
            format!("Filescape maintenance ends in {left} day{}", if left == 1 { "" } else { "s" }),
            format!("Software maintenance on your Filescape licence (serial ending {serial}) ends on {day}. Until then, every new version of Filescape is included."),
        ),
        Stage::Ended => (
            "Your Filescape maintenance has ended".to_string(),
            format!("Software maintenance on your Filescape licence (serial ending {serial}) ended on {day}."),
        ),
    };
    let after = match stage {
        Stage::Ended => "Your licence is still valid and never expires. Filescape keeps working with every Professional feature, and every version released up to that date stays unlocked for good. Versions released from now on run with the free features unless you extend maintenance.\n\nExtending maintenance adds 12 months of new versions, counted from the day you renew. It's a one-time payment, not a subscription.",
        _ => "Your licence never expires, and nothing stops working when maintenance ends: every version released until then keeps its Professional features for good. Extending maintenance adds 12 months to the end date, so renewing early loses nothing. It's a one-time payment, not a subscription.",
    };
    mail::Message {
        to: row.lic.email.clone().unwrap_or_default(),
        subject,
        text: format!(
            "{hello}\n\n{lead}\n\n{after}\n\nTo extend it, choose Help > Licence > Extend Maintenance in Filescape, or sign in at {account} and press Extend maintenance.\n\nDon't want these reminders? Turn them off: {off_link}\n\nQuestions? Write to us at https://filescape.app/contact (replies to this email aren't read).\n\nFilescape (filescape.app)"
        ),
        reply_to: None,
    }
}

impl<D: Db, N: Net> Ctx<'_, D, N> {
    /// One run: sends the reminders due now. Returns how many were sent.
    pub async fn send_reminders(&self) -> Result<usize, String> {
        // Perpetual licences ending within 30 days or ended within 7, with an address, not revoked,
        // whose owner hasn't turned reminders off.
        let sql = format!(
            "SELECT {} FROM licences l WHERE l.kind = 'perpetual' AND l.revoked_at IS NULL AND l.email IS NOT NULL AND l.maintenance_until IS NOT NULL \
             AND l.maintenance_until > ?1 AND l.maintenance_until <= ?2 \
             AND NOT EXISTS (SELECT 1 FROM email_prefs p WHERE p.email = l.email AND p.reminders = 0) ORDER BY l.maintenance_until LIMIT 500",
            crate::db::LICENCE_COLS.split(", ").map(|c| format!("l.{c}")).collect::<Vec<_>>().join(", ")
        );
        let rows = self.db.all(&sql, vec![(self.now - 7 * DAY).into(), (self.now + 30 * DAY).into()]).await?;
        let base = if self.cfg.dev { "http://localhost:8787/account".to_string() } else { format!("https://{}", self.cfg.hosts.account) };
        let mut sent = 0;
        for r in rows {
            if sent >= BATCH {
                break;
            }
            let row = db::licence(&r)?;
            let (Some(until), Some(email)) = (row.lic.validity.updates_until(), row.lic.email.clone()) else { continue };
            let Some(stage) = due(until, self.now) else { continue };
            // Claim it first, so two runs can't both send it. A later stage was already sent: skip.
            let claim_sql: &'static str = match stage {
                Stage::Days30 => "INSERT INTO reminders_sent (licence_id, until, stage, email, sent_at) SELECT ?1, ?2, ?3, ?4, ?5 WHERE NOT EXISTS (SELECT 1 FROM reminders_sent WHERE licence_id = ?1 AND until = ?2 AND stage IN ('7', 'ended')) ON CONFLICT DO NOTHING",
                Stage::Days7 => "INSERT INTO reminders_sent (licence_id, until, stage, email, sent_at) SELECT ?1, ?2, ?3, ?4, ?5 WHERE NOT EXISTS (SELECT 1 FROM reminders_sent WHERE licence_id = ?1 AND until = ?2 AND stage = 'ended') ON CONFLICT DO NOTHING",
                Stage::Ended => "INSERT INTO reminders_sent (licence_id, until, stage, email, sent_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
            };
            let claimed = self.db.batch(vec![(claim_sql, vec![(&row.lic.id).into(), until.into(), stage.key().into(), (&email).into(), self.now.into()])]).await?;
            if claimed.first() != Some(&1) {
                continue;
            }
            let off = format!("{base}/reminders/off?e={}&s={}", crate::web::encode(&email), unsubscribe_sig(&self.cfg.serial_key, &email));
            match mail::send(self, &message(&row, until, stage, self.now, &off, &base)).await {
                Ok(_) => {
                    sent += 1;
                    self.db.batch(vec![crate::store::event_stmt(&row.lic.id, self.now, "reminder", &format!("Maintenance reminder ({}) sent to {email}", stage.key()), "cron")]).await?;
                }
                Err(e) => {
                    // Give the claim back so tomorrow's run tries again.
                    eprintln!("reminder {}: {e}", row.lic.id);
                    self.db
                        .batch(vec![("DELETE FROM reminders_sent WHERE licence_id = ?1 AND until = ?2 AND stage = ?3", vec![(&row.lic.id).into(), until.into(), stage.key().into()])])
                        .await?;
                }
            }
        }
        Ok(sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_stage_due_is_the_one_sent() {
        let u = 1_000 * DAY;
        assert_eq!(due(u, u - 31 * DAY), None);
        assert_eq!(due(u, u - 30 * DAY), Some(Stage::Days30));
        assert_eq!(due(u, u - 8 * DAY), Some(Stage::Days30));
        assert_eq!(due(u, u - 7 * DAY), Some(Stage::Days7));
        assert_eq!(due(u, u - 1), Some(Stage::Days7));
        assert_eq!(due(u, u), Some(Stage::Ended));
        assert_eq!(due(u, u + 6 * DAY), Some(Stage::Ended));
        assert_eq!(due(u, u + 7 * DAY), None);
    }

    #[test]
    fn unsubscribe_links_are_signed_per_address() {
        let k = [3u8; 32];
        let s = unsubscribe_sig(&k, "a@example.com");
        assert!(unsubscribe_ok(&k, "a@example.com", &s));
        assert!(!unsubscribe_ok(&k, "b@example.com", &s));
        assert!(!unsubscribe_ok(&[4u8; 32], "a@example.com", &s));
    }
}
