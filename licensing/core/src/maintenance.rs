//! Software maintenance: which releases a perpetual licence unlocks.
//!
//! A perpetual licence never expires. Its *maintenance* (shown as "updates included until") only
//! decides which builds unlock Pro: every build released on or before the end date does, for good.
//! Maintenance ending never locks a build that is already covered.
//!
//! The end date is never edited directly. It is the result of replaying a ledger of grants, oldest
//! first ([`replay`]):
//!
//! - **Purchase**: the 12 months that come with a new licence. Its end date is stored as it was
//!   issued, so licences sold before this ledger existed keep their exact date.
//! - **Renewal** (paid) and **Complimentary** (given by an admin): if the grant is dated on or before
//!   the current end date (compared by calendar day, UTC), it extends that end date; if maintenance
//!   had already ended, it starts a new period on the grant's date. Missed periods are never owed.
//!
//! A refund or a correction marks a grant reversed and the ledger is replayed without it, so a
//! later renewal is recomputed correctly and nothing else moves. The purchase grant can't be
//! reversed (refunding a purchase revokes the licence instead), so the paid-for period is a floor
//! no adjustment can go below.

use crate::model::{Licence, LicenceKind};
use crate::{DAY, Ts};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    Purchase,
    Renewal,
    Complimentary,
}

impl GrantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantKind::Purchase => "purchase",
            GrantKind::Renewal => "renewal",
            GrantKind::Complimentary => "complimentary",
        }
    }
    pub fn parse(s: &str) -> Option<GrantKind> {
        match s {
            "purchase" => Some(GrantKind::Purchase),
            "renewal" => Some(GrantKind::Renewal),
            "complimentary" => Some(GrantKind::Complimentary),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            GrantKind::Purchase => "Purchase",
            GrantKind::Renewal => "Renewal",
            GrantKind::Complimentary => "Complimentary extension",
        }
    }
}

/// One entry in a licence's maintenance ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub kind: GrantKind,
    /// When it was granted (the payment time for a renewal).
    pub at: Ts,
    /// Length, for renewals and complimentary grants.
    pub months: u32,
    pub days: u32,
    /// The end date a purchase grant set when the licence was issued.
    pub until: Option<Ts>,
    /// Refunded, charged back or withdrawn: left out of the replay.
    pub reversed: bool,
}

impl Grant {
    pub fn purchase(at: Ts, until: Ts) -> Grant {
        Grant { kind: GrantKind::Purchase, at, months: 0, days: 0, until: Some(until), reversed: false }
    }
    pub fn renewal(at: Ts, months: u32) -> Grant {
        Grant { kind: GrantKind::Renewal, at, months, days: 0, until: None, reversed: false }
    }
    pub fn complimentary(at: Ts, months: u32, days: u32) -> Grant {
        Grant { kind: GrantKind::Complimentary, at, months, days, until: None, reversed: false }
    }
    /// Whether an admin may reverse it. Paid renewals are reversed only by a refund or chargeback
    /// from the payment provider, and a purchase never.
    pub fn admin_reversible(&self) -> bool {
        self.kind == GrantKind::Complimentary && !self.reversed
    }
}

fn civil(ts: Ts) -> (i64, i64, i64, Ts) {
    let days = ts.div_euclid(DAY);
    let secs = ts.rem_euclid(DAY);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, secs)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: i64) -> i64 {
    let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
    next - days_from_civil(y, m, 1)
}

/// `months` calendar months after `ts` (UTC), keeping the time of day. A day that doesn't exist in
/// the target month becomes its last day: 31 January + 1 month = 28 or 29 February.
pub fn add_months(ts: Ts, months: u32) -> Ts {
    let (y, m, d, secs) = civil(ts);
    let total = y * 12 + (m - 1) + i64::from(months);
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let nd = d.min(days_in_month(ny, nm));
    days_from_civil(ny, nm, nd) * DAY + secs
}

/// The UTC calendar day a time falls on.
pub fn day_of(ts: Ts) -> i64 {
    ts.div_euclid(DAY)
}

/// Where a renewal or complimentary grant dated `at` starts: at the current end date if
/// maintenance is still running that day (renewing on the last day still counts as on time),
/// otherwise on the grant's own date.
pub fn extension_start(current_until: Ts, at: Ts) -> Ts {
    if day_of(at) <= day_of(current_until) { current_until } else { at }
}

/// The end date after one more grant.
pub fn apply(current_until: Ts, g: &Grant) -> Ts {
    if g.reversed {
        return current_until;
    }
    match g.kind {
        GrantKind::Purchase => g.until.unwrap_or(current_until).max(current_until),
        GrantKind::Renewal | GrantKind::Complimentary => add_months(extension_start(current_until, g.at), g.months) + i64::from(g.days) * DAY,
    }
}

/// The maintenance end date the ledger adds up to, or `None` with no purchase grant (a licence
/// that never had maintenance). Grants are taken oldest first; ties keep the given order.
pub fn replay(grants: &[Grant]) -> Option<Ts> {
    let mut sorted: Vec<&Grant> = grants.iter().collect();
    sorted.sort_by_key(|g| g.at);
    let mut until = None;
    for g in sorted {
        if g.reversed {
            continue;
        }
        until = Some(match (until, g.kind) {
            (None, GrantKind::Purchase) => g.until?,
            // Extensions before the purchase is known can't be placed; skip them.
            (None, _) => continue,
            (Some(u), _) => apply(u, g),
        });
    }
    until
}

/// Maintenance as people see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceStatus {
    /// Running, with whole days left (counting today).
    Active { until: Ts, days_left: i64 },
    /// Ended: builds released up to `until` stay unlocked for good.
    Ended { until: Ts },
}

pub fn status(until: Ts, now: Ts) -> MaintenanceStatus {
    if now < until {
        MaintenanceStatus::Active { until, days_left: (until - now + DAY - 1) / DAY }
    } else {
        MaintenanceStatus::Ended { until }
    }
}

/// Why a licence can't take a renewal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewRefusal {
    /// Temporary licences are non-renewable.
    Temporary,
    Revoked,
    /// Maintenance already runs further ahead than the configured limit.
    TooFarAhead,
}

impl RenewRefusal {
    pub fn message(self) -> &'static str {
        match self {
            RenewRefusal::Temporary => "Temporary licences can't be renewed.",
            RenewRefusal::Revoked => "This licence has been revoked.",
            RenewRefusal::TooFarAhead => "Maintenance on this licence already runs well into the future. You can extend it again later.",
        }
    }
}

/// Whether `lic`, whose maintenance ends at `until`, can be renewed at `now`. `max_ahead_days`
/// caps how far ahead stacked renewals may reach (none: no cap).
pub fn can_renew(lic: &Licence, until: Ts, now: Ts, max_ahead_days: Option<u32>) -> Result<(), RenewRefusal> {
    if lic.kind() != LicenceKind::Perpetual {
        return Err(RenewRefusal::Temporary);
    }
    if lic.revoked_at.is_some() {
        return Err(RenewRefusal::Revoked);
    }
    if let Some(cap) = max_ahead_days
        && until - now > i64::from(cap) * DAY
    {
        return Err(RenewRefusal::TooFarAhead);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::date_time;

    fn ts(y: i64, m: i64, d: i64) -> Ts {
        days_from_civil(y, m, d) * DAY + 12 * 3600
    }
    fn day(t: Ts) -> String {
        date_time(t).split(',').next().unwrap().to_string()
    }

    #[test]
    fn calendar_months() {
        assert_eq!(day(add_months(ts(2026, 11, 28), 12)), "28 November 2027");
        assert_eq!(day(add_months(ts(2027, 1, 31), 1)), "28 February 2027");
        assert_eq!(day(add_months(ts(2028, 1, 31), 1)), "29 February 2028");
        assert_eq!(day(add_months(ts(2028, 2, 29), 12)), "28 February 2029");
        assert_eq!(day(add_months(ts(2026, 12, 15), 1)), "15 January 2027");
        assert_eq!(add_months(ts(2026, 11, 28), 0), ts(2026, 11, 28));
        for t in [0, ts(1999, 3, 1), ts(2100, 2, 28)] {
            assert_eq!(days_from_civil(civil(t).0, civil(t).1, civil(t).2), day_of(t));
        }
    }

    fn bought() -> Vec<Grant> {
        vec![Grant::purchase(ts(2026, 11, 28), ts(2027, 11, 28))]
    }

    #[test]
    fn renew_before_expiry_adds_twelve_months_to_the_end() {
        let mut g = bought();
        g.push(Grant::renewal(ts(2027, 6, 1), 12));
        assert_eq!(day(replay(&g).unwrap()), "28 November 2028");
    }

    #[test]
    fn renew_on_the_expiry_date_extends() {
        let mut g = bought();
        // Later in the day than the end time: still the same calendar day.
        g.push(Grant::renewal(ts(2027, 11, 28) + 6 * 3600, 12));
        assert_eq!(replay(&g).unwrap(), ts(2028, 11, 28));
    }

    #[test]
    fn renew_after_expiry_starts_from_the_payment() {
        let mut g = bought();
        let paid = ts(2027, 11, 29) + 3600;
        g.push(Grant::renewal(paid, 12));
        assert_eq!(replay(&g).unwrap(), add_months(paid, 12));
        assert_eq!(day(replay(&g).unwrap()), "29 November 2028");
    }

    #[test]
    fn renew_after_several_years_charges_nothing_for_the_gap() {
        // Rich's example: buy Nov 2026, lapse Nov 2027, return Nov 2028.
        let mut g = bought();
        let paid = ts(2028, 11, 20);
        g.push(Grant::renewal(paid, 12));
        assert_eq!(day(replay(&g).unwrap()), "20 November 2029");
        let mut g = bought();
        g.push(Grant::renewal(ts(2031, 3, 5), 12));
        assert_eq!(day(replay(&g).unwrap()), "5 March 2032");
    }

    #[test]
    fn several_renewals_while_active_stack() {
        let mut g = bought();
        g.push(Grant::renewal(ts(2027, 1, 1), 12));
        g.push(Grant::renewal(ts(2027, 2, 1), 12));
        g.push(Grant::renewal(ts(2027, 3, 1), 12));
        assert_eq!(day(replay(&g).unwrap()), "28 November 2030");
    }

    #[test]
    fn a_refunded_renewal_is_replayed_out_and_later_ones_recomputed() {
        let mut g = bought();
        g.push(Grant::renewal(ts(2027, 1, 1), 12));
        g.push(Grant::renewal(ts(2027, 2, 1), 12));
        assert_eq!(day(replay(&g).unwrap()), "28 November 2029");
        g[1].reversed = true;
        assert_eq!(day(replay(&g).unwrap()), "28 November 2028");

        // A refund that makes a later renewal land after expiry starts that one from its own date.
        let mut g = bought();
        g.push(Grant::renewal(ts(2027, 10, 1), 12));
        g.push(Grant::renewal(ts(2028, 3, 1), 12));
        assert_eq!(day(replay(&g).unwrap()), "28 November 2029");
        g[1].reversed = true;
        assert_eq!(day(replay(&g).unwrap()), "1 March 2029");
    }

    #[test]
    fn nothing_goes_below_the_purchase() {
        let mut g = bought();
        g.push(Grant::complimentary(ts(2027, 5, 1), 0, 30));
        assert_eq!(day(replay(&g).unwrap()), "28 December 2027");
        g[1].reversed = true;
        assert_eq!(replay(&g).unwrap(), ts(2027, 11, 28));
        // Even a reversed purchase flag is never produced by the server, but replay still needs
        // the purchase to place anything.
        assert_eq!(replay(&[Grant::renewal(ts(2027, 1, 1), 12)]), None);
        assert!(!g[0].admin_reversible());
        assert!(!Grant::renewal(0, 12).admin_reversible());
        assert!(Grant::complimentary(0, 1, 0).admin_reversible());
    }

    #[test]
    fn grant_order_is_by_date_not_by_insertion() {
        let mut g = vec![Grant::renewal(ts(2028, 3, 1), 12)];
        g.extend(bought());
        assert_eq!(day(replay(&g).unwrap()), "1 March 2029");
    }

    #[test]
    fn status_counts_days_left() {
        let until = ts(2027, 11, 28);
        assert_eq!(status(until, until - 10 * DAY), MaintenanceStatus::Active { until, days_left: 10 });
        assert_eq!(status(until, until - 10 * DAY + 1), MaintenanceStatus::Active { until, days_left: 10 });
        assert_eq!(status(until, until), MaintenanceStatus::Ended { until });
    }

    #[test]
    fn only_live_perpetual_licences_renew() {
        let now = ts(2027, 1, 1);
        let mut lic = crate::model::Licence {
            id: "L".into(),
            serial_hash: String::new(),
            serial_hint: String::new(),
            validity: crate::model::Validity::Perpetual { updates_until: ts(2027, 11, 28) },
            owner: None,
            email: None,
            purpose: None,
            issued_at: now,
            first_activated_at: None,
            expires_at: None,
            max_activations: 3,
            activations_used: 0,
            revoked_at: None,
            revoke_reason: None,
            order_ref: None,
            events: vec![],
        };
        let until = ts(2027, 11, 28);
        assert_eq!(can_renew(&lic, until, now, None), Ok(()));
        assert_eq!(can_renew(&lic, until, now, Some(365)), Ok(()));
        assert_eq!(can_renew(&lic, until + 2 * 365 * DAY, now, Some(730)), Err(RenewRefusal::TooFarAhead));
        lic.revoked_at = Some(now);
        assert_eq!(can_renew(&lic, until, now, None), Err(RenewRefusal::Revoked));
        lic.revoked_at = None;
        lic.validity = crate::model::Validity::FirstActivation { days: 30 };
        assert_eq!(can_renew(&lic, until, now, None), Err(RenewRefusal::Temporary));
    }
}
