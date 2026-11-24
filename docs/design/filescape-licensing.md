# Filescape licensing

The full assessment and plan (decisions, architecture, Cloudflare setup, Paddle, test plan) is
the shared doc "dirmap temporary licences: assessment and plan" (23 Nov 2026). This file tracks
progress in the repo.

Before 23 Nov 2026 dirmap had no licensing at all: no evaluation, no perpetual licences, no
server. Everything here is new.

## Decisions (Rich, 23 Nov 2026)

- Product name **Filescape**, at filescape.app. The app itself is not renamed yet.
- Free: the file browser, the disk map and basic filtering (category buttons, size limits,
  Highlight / Show matches only, scan exclusions). Everything else is premium
  (`licensing/core/src/features.rs` is the one table).
- Temporary licences: 30 days by default, or any admin-set length or fixed date; 3 activations
  ever; non-renewable.
- Perpetual licences: 10 activations by default (changed from 3 on 23 Nov). The admin sets the
  default for new purchases and can change any perpetual licence's limit; an Activations page
  flags possible sharing (more than 5 new machines in 30 days). Support can reset activations.
- Hosting: Cloudflare. Worker in Rust (`workers-rs`, sharing `licensing/core`) with D1 at
  api.filescape.app, admin.filescape.app (behind Cloudflare Access) and account.filescape.app;
  website on Pages; installers on R2 at downloads.filescape.app.
- Payments: Paddle (`transaction.completed` webhook issues the licence). Email: Resend.
- Premium checks happen only where an operation starts. Queued and running work (transfers,
  retries, resumes, moves, renames, archive writes) always finishes.

## Tracker

| Phase | What | State |
|---|---|---|
| 1 | `licensing/core`: model, serials, Ed25519 envelopes, activation rules, reference ledger, precedence, evaluation, clock checks, exports, spec tests | ✅ 23 Nov |
| 2 | App: `src/licence/`, keychain store, device ID, gating at start points, Settings → Licensing, exports, offline files, Help Centre; core: adjustable perpetual limits and the activation report | ✅ 23 Nov (test mode until the server exists) |
| 3 | `licensing/worker` on Cloudflare: D1, activation API, offline page, web admin, customer page, Paddle webhook, Resend, Pages site, R2 downloads | ✅ 23 Nov (code and tests; deploying is Rich's, see `licensing/SETUP.md`) |
| 4 | Hardening: end-to-end tests against a test Worker, device IDs on all three platforms, key rotation dry run, release | ⏳ |

## Running the tests

    cargo test --manifest-path licensing/core/Cargo.toml

`tests/spec.rs` has one test per item of the spec's section 10 (t01 … t20), plus a 20-thread
race on one serial and the admin tools. CI runs them on Windows, macOS and Linux.

## Phase 2 notes

- `src/licence/mod.rs`: state in `licence.json` (settings folder) with the clock record mirrored in
  the keychain (`filescape-licence` service); serials in the keychain, or in `licence.json` where
  there's no keychain. Device ID: Windows MachineGuid, macOS IOPlatformUUID, Linux machine-id.
- Gates (`App::premium` / `Licensing::begin`): snapshots save/load, duplicates, batch rename,
  compare, sequence player, connect to a server, `queue_transfer` (new server transfers only, never
  retries), archive open/extract, saved searches, the Custom filter and sequence filter.
- Test mode: cargo feature `licence-dev` (on by default for now; drop for release in phase 4) plus
  `FILESCAPE_LICENCE_DEV=1`. The in-app server is `MemoryLedger` saved to
  `licence-test-server.json`, with its own key and a licence clock offset.
- `FILESCAPE_LICENCE_PUBKEY` (build time) adds the production public key; `FILESCAPE_LICENCE_SERVER`
  overrides the server URL.

## Phase 3 notes

- `licensing/worker`: one Rust Worker (workers-rs 0.9) for api., account. and admin.filescape.app,
  with D1 (`migrations/0001_init.sql`). Logic is plain Rust over two traits (`Db`, `Net`); the tests
  (`tests/server.rs`) run it against SQLite, the engine D1 uses, with a fake network. It was also run
  in `wrangler dev` with a local D1, and the app activated and deactivated against it.
- Activation takes the last use atomically: one D1 batch with `UPDATE ... WHERE activations_used = ?`
  and `INSERT ... WHERE changes() = 1`; a lost race decides again from fresh rows.
- Serials: looked up by hash; also kept AES-256-GCM encrypted (`SERIAL_KEY`) so customers and the
  admin can have them emailed again. A database copy alone reveals none.
- Paddle (live account, shared with another product): only `PADDLE_PRICE_IDS` issue licences;
  other products' transactions and refunds are logged and ignored. Licences are keyed on the
  transaction ID; a failed email returns 500 so Paddle retries and the email is resent.
- Admin: Cloudflare Access in front, and the Worker verifies Access's RS256 token and audience
  itself. Every change goes in the licence history with the admin's email.
- Website drafts in `licensing/site` (legal name and refund period to fill in). Setup steps:
  `licensing/SETUP.md`.
- App: Deactivate This Computer now asks to confirm.

## Phase 1b (launch fixes, 24 Nov 2026)

From the audit in the shared doc "Filescape licensing audit and enterprise proposal".

- Test mode (`licence-dev`) is no longer a default feature, and `build.rs` refuses a release build
  with it (override with `FILESCAPE_ALLOW_LICENCE_DEV=1` for a local test build only). Run its
  tests with `cargo test --features licence-dev`. It was removed from the public Help Centre;
  usage: start a `--features licence-dev` build with `FILESCAPE_LICENCE_DEV=1`, then Settings >
  Licensing > Test mode issues serials and moves the licence clock.
- The production public key is in `src/licence/mod.rs` (`PRODUCTION_KEYS`, key ID
  `204a936d1dae0b1d`). `FILESCAPE_LICENCE_PUBKEY` still adds a test Worker's key at build time.
- `RELEASE_DATE` comes from build.rs: `SOURCE_DATE_EPOCH`, else the commit date.
- The weekly check removes or locks a licence only on a server-signed `StatusStatement`
  (purpose `status`); unsigned errors, including "not found", change nothing. A successful check
  clears a licence from the local revoked list (an un-revoke now unlocks the app).
- Paddle: licences are issued before the customer lookup, which fills in owner and email
  afterwards. An approved full refund or chargeback revokes; pending, partial and chargeback
  warnings only flag; an approved chargeback reversal reinstates what a chargeback revoked.
- `ACCESS_AUD` is a Worker secret, not a `wrangler.toml` variable; `keep_vars = true`.

## Phase 2 (individual licences, 24 Nov 2026)

Rich's answers in the audit doc: no sales yet, personal Pro = 3 active computers, price $9.99
standard / $4.99 launch (Paddle price unchanged at $5 until he approves the change).

- Perpetual licences limit computers active now: `rules::seats_in_use` = live activations +
  max(0, self-releases in the last 7 days − 1) (`RELEASE_COOLDOWN_DAYS`). Deactivating in the app
  or Release on the account page frees the seat; admin Void frees it at once and doesn't count;
  Reset clears the cooldown. `activations_used` stays as history. Certificates carry seats in use
  for perpetual licences, uses ever for temporary ones.
- Temporary licences keep the original spec's 3-uses-ever rule (spec tests t05, t06, t10 and t16
  unchanged).
- `ActivationRequest.os` (optional, serde default): Windows, macOS or Linux, shown on the account
  page. Migration `0002_seats_and_account.sql`: `activations.os`, `activations.released_via`,
  `logins`, `sessions`.
- The Worker's allocation stays one batch: `UPDATE licences … WHERE activations_used = <read>`
  (a version check) then `INSERT … WHERE changes() = 1`; a lost race decides again. A concurrent
  release only frees seats, so it can't over-allocate.
- account.filescape.app sign-in: emailed one-time link (15 minutes, hash stored, used by a POST
  so mail scanners can't spend it), 30-day HttpOnly Secure SameSite=Lax cookie (hash stored),
  forms checked for Origin. Signed in: licences for that email, computers, Release.
- Deferred: update renewals (needs a Paddle price, so Rich's approval), Resend idempotency keys,
  a D1 export job (D1 Time Travel already keeps 30 days).
