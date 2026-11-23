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
| 3 | `licensing/worker` on Cloudflare: D1, activation API, offline page, web admin, customer page, Paddle webhook, Resend, Pages site, R2 downloads | ⏳ |
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
