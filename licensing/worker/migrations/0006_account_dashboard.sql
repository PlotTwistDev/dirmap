-- Account dashboard (28 Nov 2026): reminder preferences, self-service email change with an undo
-- link, and a per-IP limit on the sign-in and "email my serial numbers" forms.

-- One row per customer email. No row: the defaults (maintenance reminders on).
CREATE TABLE email_prefs (
  email TEXT PRIMARY KEY,
  reminders INTEGER NOT NULL DEFAULT 1,  -- maintenance ending reminders (30 and 7 days, on the day)
  updated_at INTEGER NOT NULL
);

-- Email changes. 'confirm': sent to the new address, moves every licence on the old address.
-- 'undo': sent to the old address once moved, puts those licences back (licence_ids, JSON array).
-- Only token hashes are kept.
CREATE TABLE email_changes (
  token_hash TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  old_email TEXT NOT NULL,
  new_email TEXT NOT NULL,
  licence_ids TEXT,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  used_at INTEGER
);
CREATE INDEX email_changes_old ON email_changes(old_email, kind);
CREATE INDEX email_changes_new ON email_changes(new_email, kind);

-- Form posts per (keyed hash of) IP address, for rate limits. Rows older than a day are deleted.
CREATE TABLE ip_log (
  ip_hash TEXT NOT NULL,
  kind TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX ip_log_ip ON ip_log(ip_hash, kind, at);
CREATE INDEX ip_log_at ON ip_log(at);
