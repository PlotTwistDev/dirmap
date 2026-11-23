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
- Perpetual licences: also 3 activations ever (about $5, so buying again is fine); support can
  reset activations from the web admin.
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
| 2 | App: `src/licence/`, keychain store, device ID, gating at start points, Settings → Licensing, exports, offline files, Help Centre | ⏳ after review |
| 3 | `licensing/worker` on Cloudflare: D1, activation API, offline page, web admin, customer page, Paddle webhook, Resend, Pages site, R2 downloads | ⏳ |
| 4 | Hardening: end-to-end tests against a test Worker, device IDs on all three platforms, key rotation dry run, release | ⏳ |

## Running the tests

    cargo test --manifest-path licensing/core/Cargo.toml

`tests/spec.rs` has one test per item of the spec's section 10 (t01 … t20), plus a 20-thread
race on one serial and the admin tools. CI runs them on Windows, macOS and Linux.
