-- Maintenance reminder emails (28 Nov 2026), sent by the daily Cron trigger: 30 and 7 days before
-- software maintenance ends, and on the day it ends. One row per licence, end date and stage, so a
-- renewal (a new end date) starts the reminders again and a reminder is never sent twice.
CREATE TABLE reminders_sent (
  licence_id TEXT NOT NULL,
  until INTEGER NOT NULL,
  stage TEXT NOT NULL,                  -- '30', '7' or 'ended'
  email TEXT NOT NULL,
  sent_at INTEGER NOT NULL,
  PRIMARY KEY (licence_id, until, stage)
);
