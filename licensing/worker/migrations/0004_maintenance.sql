-- Software maintenance (28 Nov 2026). A perpetual licence never expires; its maintenance end date
-- ("updates included until") only decides which releases unlock Pro. That date is now the result
-- of a ledger of grants: the purchase, paid renewals and complimentary extensions. A refund marks a
-- grant reversed and the ledger is replayed, so nothing ever edits the date directly and the
-- purchased period can't be lost. See filescape_licence::maintenance.

-- The replayed end date, kept in step with validity.updates_until (which certificates carry).
ALTER TABLE licences ADD COLUMN maintenance_until INTEGER;
-- Which product a licence is for, so the server can serve more than one later.
ALTER TABLE licences ADD COLUMN product TEXT NOT NULL DEFAULT 'filescape';

CREATE TABLE maintenance_grants (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  licence_id TEXT NOT NULL REFERENCES licences(id),
  product TEXT NOT NULL DEFAULT 'filescape',
  kind TEXT NOT NULL,                  -- 'purchase', 'renewal' or 'complimentary'
  at INTEGER NOT NULL,                 -- when granted (payment time for a renewal)
  months INTEGER NOT NULL DEFAULT 0,
  days INTEGER NOT NULL DEFAULT 0,
  until INTEGER,                       -- purchase only: the end date it was issued with
  txn TEXT UNIQUE,                     -- payment provider transaction (renewals): one grant each
  amount TEXT,                         -- as the provider reported it, e.g. '2.99'
  currency TEXT,
  actor TEXT NOT NULL,
  reason TEXT,
  before_until INTEGER,                -- the licence's end date before and after this grant
  after_until INTEGER,
  reversed_at INTEGER,
  reversed_by TEXT,
  reverse_reason TEXT
);
CREATE INDEX maintenance_grants_licence ON maintenance_grants(licence_id, at);

-- Every perpetual licence gets its purchase grant with the date it already has: no date moves.
INSERT INTO maintenance_grants (licence_id, kind, at, until, txn, actor, reason, before_until, after_until)
  SELECT id, 'purchase', issued_at, json_extract(validity, '$.updates_until'), NULL, 'migration', 'existing licence', NULL, json_extract(validity, '$.updates_until')
  FROM licences WHERE kind = 'perpetual' AND json_extract(validity, '$.mode') = 'perpetual';
UPDATE licences SET maintenance_until = json_extract(validity, '$.updates_until')
  WHERE kind = 'perpetual' AND json_extract(validity, '$.mode') = 'perpetual';
CREATE INDEX licences_maintenance ON licences(maintenance_until);
