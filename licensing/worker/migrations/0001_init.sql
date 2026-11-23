-- Filescape licence server, D1 schema.
-- Serials are never stored in clear: serial_hash finds a licence, serial_enc (AES-256-GCM under
-- the SERIAL_KEY secret) lets the server email a customer their serial again.

CREATE TABLE licences (
  id TEXT PRIMARY KEY,
  serial_hash TEXT NOT NULL UNIQUE,
  serial_hint TEXT NOT NULL,
  serial_enc TEXT,
  kind TEXT NOT NULL,                 -- 'temporary' or 'perpetual'
  validity TEXT NOT NULL,             -- JSON of filescape_licence::model::Validity
  owner TEXT,
  email TEXT,                         -- lower case
  purpose TEXT,
  issued_at INTEGER NOT NULL,
  first_activated_at INTEGER,
  expires_at INTEGER,
  max_activations INTEGER NOT NULL,
  activations_used INTEGER NOT NULL DEFAULT 0,
  revoked_at INTEGER,
  revoke_reason TEXT,
  order_ref TEXT UNIQUE,              -- Paddle transaction ID (with #n for extra quantity)
  refund_flag TEXT,                   -- set when Paddle reports a refund or chargeback
  emailed_at INTEGER
);
CREATE INDEX licences_email ON licences(email);
CREATE INDEX licences_expires ON licences(expires_at);
CREATE INDEX licences_issued ON licences(issued_at);

CREATE TABLE activations (
  activation_id TEXT PRIMARY KEY,
  licence_id TEXT NOT NULL REFERENCES licences(id),
  device_hash TEXT NOT NULL,
  app_version TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  counted INTEGER NOT NULL DEFAULT 1,  -- 0 after an admin void (use given back) or reset
  ended_at INTEGER,                    -- deactivated by the user
  voided_at INTEGER,                   -- cancelled by an admin
  void_reason TEXT,
  last_seen_at INTEGER
);
CREATE INDEX activations_licence ON activations(licence_id);

CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  licence_id TEXT,
  at INTEGER NOT NULL,
  what TEXT NOT NULL,
  detail TEXT NOT NULL,
  actor TEXT
);
CREATE INDEX events_licence ON events(licence_id);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Paddle notifications already handled, so a resend does nothing.
CREATE TABLE webhooks (
  event_id TEXT PRIMARY KEY,
  received_at INTEGER NOT NULL,
  kind TEXT NOT NULL,
  outcome TEXT NOT NULL
);

-- "Email me my licence" requests, to limit how often one address is mailed.
CREATE TABLE mail_log (
  email TEXT NOT NULL,
  at INTEGER NOT NULL,
  kind TEXT NOT NULL
);
CREATE INDEX mail_log_email ON mail_log(email, at);
