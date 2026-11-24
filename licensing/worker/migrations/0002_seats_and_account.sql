-- Phase 2 (24 Nov 2026): perpetual licences count computers active now, customers sign in to
-- account.filescape.app with an emailed link to see and release their computers.

-- What the app reported, shown on the account page: 'Windows', 'macOS' or 'Linux'.
ALTER TABLE activations ADD COLUMN os TEXT;
-- Who ended it: 'app' (Deactivate in Filescape) or 'account' (Release on the account page).
-- Both count towards the release cooldown; an admin void doesn't.
ALTER TABLE activations ADD COLUMN released_via TEXT;
CREATE INDEX activations_device ON activations(licence_id, device_hash);

-- Sign-in links. Only a hash of the token is kept; a link works once, for 15 minutes.
CREATE TABLE logins (
  token_hash TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  used_at INTEGER
);
CREATE INDEX logins_email ON logins(email, created_at);

-- Signed-in browsers. The cookie carries the token; only its hash is kept.
CREATE TABLE sessions (
  token_hash TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_email ON sessions(email);
