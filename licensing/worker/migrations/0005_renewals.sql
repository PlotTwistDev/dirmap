-- Maintenance renewals (28 Nov 2026). A customer starts a renewal from the account page (signed
-- in) or from the app (proved by its activation); the server keeps a random token for that licence
-- and the Paddle checkout carries it as custom data. Only the token's hash is kept. When Paddle
-- reports the payment, the token says which licence to extend: the browser never names a licence
-- or an amount the server trusts. Renewal prices are chosen on the admin Renewals page
-- (settings key 'renewal').

CREATE TABLE renewal_tokens (
  token_hash TEXT PRIMARY KEY,
  licence_id TEXT NOT NULL REFERENCES licences(id),
  source TEXT NOT NULL,                -- 'account' or 'app'
  activation_id TEXT,                  -- the app's activation, when started from the app
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,         -- checkout can be opened until then
  last_txn TEXT                        -- the last payment made with it
);
CREATE INDEX renewal_tokens_licence ON renewal_tokens(licence_id, created_at);
