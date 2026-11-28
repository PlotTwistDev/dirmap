# Setting up the Filescape licence server

Everything runs on Cloudflare: the licence server (a Worker, with a D1 database) at
api.filescape.app, account.filescape.app and admin.filescape.app; the website on Cloudflare Pages
at filescape.app; downloads on R2 at downloads.filescape.app. Payments go through Paddle, licence
emails through Resend.

This is done once. It takes about an hour. Commands run from the repository folder on your own
computer. You need Rust, Node.js 20 or later, and the filescape.app domain on Cloudflare (DNS
managed there). The Worker is compiled to WebAssembly, so also run this once:

```
rustup target add wasm32-unknown-unknown
rustup component add llvm-tools
cargo install worker-build
```

**Secrets never go in chat, email or the repository.** You paste them straight into Cloudflare
(`wrangler secret put …` asks for the value) and keep a copy in your password manager.

## 1. Make the keys

```
cargo run --manifest-path licensing/worker/Cargo.toml --example keygen
```

It prints three values. Save all three in your password manager now:

- `SIGNING_KEY`: signs every licence. A Worker secret.
- `SERIAL_KEY`: encrypts stored serials so they can be emailed again. A Worker secret.
- The public half. Not secret. The production one is built into the app (step 10).

Run it once only. A new signing key means app builds with the old public key stop accepting new
licences.

## 2. Wrangler and the database

```
cd licensing/worker
npx wrangler@4 login
npx wrangler@4 d1 create filescape-licences
```

Copy the `database_id` it prints into `wrangler.toml` (replacing the zeros), then create the tables:

```
npx wrangler@4 d1 migrations apply filescape-licences --remote
```

D1 keeps 30 days of point-in-time history automatically (Time Travel), so a mistake can be undone
with `wrangler d1 time-travel restore`.

## 3. Worker secrets

Each command asks for the value; paste it:

```
npx wrangler@4 secret put SIGNING_KEY
npx wrangler@4 secret put SERIAL_KEY
npx wrangler@4 secret put PADDLE_WEBHOOK_SECRET     # step 6
npx wrangler@4 secret put PADDLE_API_KEY            # "Filescape licence Worker" key in Paddle
npx wrangler@4 secret put RESEND_API_KEY            # step 5
npx wrangler@4 secret put CONTACT_NOTIFY_TO         # optional: your address, emailed about contact form messages
```

You can add the Paddle and Resend ones later; until then purchases aren't processed and nothing
is emailed (the admin site still works and shows serials on screen).

## 4. Cloudflare Access for the admin site

The admin site refuses everyone until this is set up.

1. Cloudflare dashboard > **Zero Trust** (the free plan covers up to 50 users). Note your team
   domain, such as `filescape.cloudflareaccess.com` (Settings > Custom Pages shows it).
2. **Access > Applications > Add an application > Self-hosted**. Domain `admin.filescape.app`.
   Add a policy: Action **Allow**, Include **Emails**, your address. One-time PIN login is fine.
3. Open the application's **Overview** and copy the **Application Audience (AUD) Tag**.
4. In `wrangler.toml`, set `ACCESS_TEAM_DOMAIN` to the team domain. Store the tag with
   `wrangler secret put ACCESS_AUD` (run in `licensing/worker`), so later deploys never blank it.

## 5. Email (Resend) and the contact form

1. Sign up at resend.com, **Domains > Add domain** `filescape.app`, and add the DNS records it shows
   in Cloudflare DNS. Wait for it to verify.
2. **API Keys > Create**, "Sending access", domain filescape.app. Put it in `RESEND_API_KEY` (step 3).
3. Licence emails come from `licences@filescape.app` (`MAIL_FROM` in `wrangler.toml`) with no
   Reply-To: there's no filescape.app mailbox. Every email and page points people at the contact
   form, https://filescape.app/contact, instead.
4. The contact form posts to `https://api.filescape.app/v1/contact`. Messages are kept in D1 and
   listed at https://admin.filescape.app/messages, where you reply: replies are emailed from
   `MAIL_FROM`, so your own address never appears. To be told when one arrives, store your address
   as a secret (it never goes in the repository or on the site):
   `npx wrangler@4 secret put CONTACT_NOTIFY_TO`. The notice has no Reply-To; answer from the
   admin site.
5. Spam: the form has a hidden honeypot field, a minimum time on the page, 5 messages an hour per
   address and per IP (only a keyed hash of the IP is stored), and 300 a day in all. For more,
   add Cloudflare **Turnstile** (free): **Turnstile > Add widget**, hostname `filescape.app`,
   mode Managed. Put the site key in `turnstileSiteKey` in `licensing/site/config.js` and the
   secret key in `npx wrangler@4 secret put TURNSTILE_SECRET`, then deploy both. (Set the secret
   only once the site carries the site key, or the form refuses every message.)

## 6. Deploy the Worker

```
npx wrangler@4 deploy
```

Run it from `licensing/worker`. It builds the Rust code (`build.sh`; the first build takes a few
minutes) and creates the three hostnames. Check https://api.filescape.app/v1/health: `key_id` must match the
key ID keygen printed.

## 7. Paddle

The Filescape product and its prices are already on your live Paddle account: standard $9.99
(`pri_01m86kecyaqrsp7d702v4hc679`), launch $4.99 (`pri_01m86kf4h9r8tj5p6rtgez78v6`, the one the
website sells now) and the old $5.00 (`pri_01m84qwgg1a8858drzqk2rvzpz`, to be archived). All three
are in `wrangler.toml` as `PADDLE_PRICE_IDS`, so any of them issues a licence. Paddle sends
notifications for your other product too; the server ignores anything without these prices.
To end the launch price, change `licensing/site/config.js` (priceId, priceLabel, listPriceLabel)
and redeploy the website (step 8).

1. **Developer tools > Notifications > New destination**: URL `https://api.filescape.app/v1/paddle`,
   events `transaction.completed`, `adjustment.created`, `adjustment.updated`. Save, open it, copy
   the **secret key** into `PADDLE_WEBHOOK_SECRET` (step 3).
2. Use **Send test notification** for `transaction.completed`: the server answers "ignored … not a
   Filescape purchase" (the sample isn't the Filescape price), which shows the signature check works.
3. **Checkout > Checkout settings**: add `https://filescape.app/buy` as a default payment link if
   Paddle asks for one.
4. After the website is live (step 8), add filescape.app under **Checkout > Website approval**.

## 8. The website (Cloudflare Pages)

`licensing/site` is the whole site, ready to deploy: the homepage, the feature pages `/formats`,
`/ai-models` and `/vfx`, `/buy`, `/mac`, `/contact`, `/terms`, `/privacy` and `/refunds`. The homepage
and feature pages are generated from `licensing/site-src` (shared `head.html`, `nav.html` and
`foot.html`, the homepage in `body.html`, the others in `pages/`); after changing anything there,
or `docs/src/reference/formats.md`, run `python3 licensing/site-src/build.py`. It also writes
`sitemap.xml` and `robots.txt`: add any new public page to `PAGES` or `STATIC` in `build.py`. The
Paddle token and price for both checkout buttons are in `licensing/site/config.js`; rebuild after
changing the price, since the structured data search engines read takes it from there.

```
npx wrangler@4 pages deploy licensing/site --project-name filescape-site --branch main
```

The first run asks to create the project; give `main` as its production branch. `--branch main`
matters: without it wrangler uses your checkout's git branch, and anything other than the
production branch goes to a preview address instead of filescape.app.

In the dashboard, **Workers & Pages > filescape-site > Custom domains**: add `filescape.app` (and
`www.filescape.app` if you want it). Then submit the domain to Paddle (step 7.4).

### Google Search Console

filescape.app is a **Domain** property in Search Console (verified with a DNS record). Once the site
is deployed, open the property, choose **Sitemaps** in the left menu, enter `sitemap.xml` under
**Add a new sitemap** and press **Submit**. Google then reads `https://filescape.app/sitemap.xml`
again by itself whenever it changes. To ask for a changed page to be crawled sooner, paste its
address into **URL inspection** at the top and press **Request indexing**.

## 9. Downloads (R2)

1. **R2 > Create bucket** `filescape-downloads`. **Settings > Custom domains**: `downloads.filescape.app`.
2. Get the files from CI: a tag push (or **Actions > build > Run workflow** with a version) puts
   them on a GitHub release; any run also keeps them as artifacts named `Filescape-windows`,
   `Filescape-macos` and `Filescape-linux`. Linux has four files:

   | File | For | R2 object |
   |---|---|---|
   | `Filescape-windows.zip` | Windows 10 and 11 | `latest/windows` |
   | `Filescape-macos.dmg` | macOS 11 and later | `latest/macos` |
   | `Filescape-linux.AppImage` | any Linux, no install (the main Linux button) | `latest/linux-appimage` |
   | `Filescape-linux.deb` | Ubuntu, Debian, Mint, Pop!_OS | `latest/linux-deb` |
   | `Filescape-linux.rpm` | Fedora, RHEL, openSUSE | `latest/linux-rpm` |
   | `Filescape-linux.tar.gz` | any Linux, unpack and run | `latest/linux` |

   All Linux builds need glibc 2.35 or later (Ubuntu 22.04, Debian 12, Fedora 36 and newer).
3. Upload each one under its object name, from the folder the files are in. `--cd` gives the
   downloaded file its proper name:
   ```
   npx wrangler@4 r2 object put filescape-downloads/latest/windows --file Filescape-windows.zip --ct application/zip --cd "attachment; filename=Filescape-windows.zip" --remote
   npx wrangler@4 r2 object put filescape-downloads/latest/macos --file Filescape-macos.dmg --ct application/x-apple-diskimage --cd "attachment; filename=Filescape-macos.dmg" --remote
   npx wrangler@4 r2 object put filescape-downloads/latest/linux-appimage --file Filescape-linux.AppImage --ct application/vnd.appimage --cd "attachment; filename=Filescape-linux.AppImage" --remote
   npx wrangler@4 r2 object put filescape-downloads/latest/linux-deb --file Filescape-linux.deb --ct application/vnd.debian.binary-package --cd "attachment; filename=filescape_amd64.deb" --remote
   npx wrangler@4 r2 object put filescape-downloads/latest/linux-rpm --file Filescape-linux.rpm --ct application/x-rpm --cd "attachment; filename=filescape.x86_64.rpm" --remote
   npx wrangler@4 r2 object put filescape-downloads/latest/linux --file Filescape-linux.tar.gz --ct application/gzip --cd "attachment; filename=Filescape-linux.tar.gz" --remote
   ```
   Uploading again replaces the file. The GitHub release keeps the same files as a mirror.

   Upload each release a second time under its version number as well, so customers whose
   maintenance has ended can still download the newest version their licence unlocks (the
   account site lists every version at account.filescape.app/versions). Same commands with
   `latest/` replaced by the version, for example:
   ```
   V=0.3.0
   npx wrangler@4 r2 object put filescape-downloads/$V/windows --file Filescape-windows.zip --ct application/zip --cd "attachment; filename=Filescape-$V-windows.zip" --remote
   npx wrangler@4 r2 object put filescape-downloads/$V/macos --file Filescape-macos.dmg --ct application/x-apple-diskimage --cd "attachment; filename=Filescape-$V-macos.dmg" --remote
   npx wrangler@4 r2 object put filescape-downloads/$V/linux-appimage --file Filescape-linux.AppImage --ct application/vnd.appimage --cd "attachment; filename=Filescape-$V-linux.AppImage" --remote
   npx wrangler@4 r2 object put filescape-downloads/$V/linux-deb --file Filescape-linux.deb --ct application/vnd.debian.binary-package --cd "attachment; filename=filescape_${V}_amd64.deb" --remote
   npx wrangler@4 r2 object put filescape-downloads/$V/linux-rpm --file Filescape-linux.rpm --ct application/x-rpm --cd "attachment; filename=filescape-$V.x86_64.rpm" --remote
   npx wrangler@4 r2 object put filescape-downloads/$V/linux --file Filescape-linux.tar.gz --ct application/gzip --cd "attachment; filename=Filescape-$V-linux.tar.gz" --remote
   ```
   These are never replaced: a version's files stay as they were released.
4. Check each link downloads with the right name: `curl -sI https://downloads.filescape.app/latest/linux-deb`
   shows the `content-disposition` line.
5. Publish checksums, so people can check their download. In the folder with the six files, name
   them as they arrive in a browser and write the sums into the site, then rebuild and deploy it:
   ```
   cp Filescape-linux.deb filescape_amd64.deb; cp Filescape-linux.rpm filescape.x86_64.rpm
   { echo "# Filescape $(grep -m1 '^version' ../Cargo.toml | cut -d'"' -f2), SHA-256"; shasum -a 256 Filescape-windows.zip Filescape-macos.dmg Filescape-linux.AppImage filescape_amd64.deb filescape.x86_64.rpm Filescape-linux.tar.gz; } > licensing/site/SHA256SUMS.txt
   ```
   (adjust the paths to where the files and the repo are). The homepage's Downloads section links
   to the file only while it exists; delete it if a later upload goes out without new sums.

To build the Linux packages yourself on Ubuntu (22.04 for the widest reach):
`sudo apt install rpm`, `cargo build --release`, then
`packaging/linux/package.sh target/release/filescape dist`.

## 10. App builds for sale

A plain release build is the build for sale:

```
cargo build --release
```

The production public key (key ID `204a936d1dae0b1d`) is built into the app
(`src/licence/mod.rs`, `PRODUCTION_KEYS`), and test mode is off unless asked for with
`--features licence-dev`, which a release build refuses. The release date that update entitlement
is checked against comes from the commit being built (or `SOURCE_DATE_EPOCH`), so there's nothing
to edit at release time. If you ever make a new signing key, add its public half to
`PRODUCTION_KEYS` and ship that build *before* switching the Worker to the new key.

## 10b. Releasing a new version (the in-app update notice)

Every copy of Filescape asks `https://api.filescape.app/v1/release` at launch. The Worker answers
with the release set on the admin site's **Releases** page, signed with the same key as the
licences, so there's no extra key or file to look after. Until a release is set there, the app
shows nothing.

1. Bump `version` in `Cargo.toml` (for example `0.2.0` to `0.3.0`). The notice compares version
   numbers: a build with the same version as the announced one never shows it.
2. Commit, then tag and push: `git tag v0.3.0 && git push origin v0.3.0`. CI builds the three
   downloads and attaches them to a GitHub release.
3. Upload the downloads to R2 (step 9), so `downloads.filescape.app/latest/...` is the new version
   before anyone is told about it.
4. Get the release date the build uses for update entitlement: `git log -1 --format=%ct v0.3.0`.
5. https://admin.filescape.app/releases: version `0.3.0`, release date (paste the number from
   step 4, or type the date), an optional one-line summary and release notes link (on
   filescape.app), then **Announce**. Download links can stay empty: they default to
   `downloads.filescape.app/latest/windows` and `/latest/macos`, and on Linux to the website's
   download section, where people pick the AppImage, .deb, .rpm or .tar.gz.
6. Copies of Filescape show the notice the next time they start (answers are cached for up to 5
   minutes). A licence whose maintenance ended before the release date is told so, with Extend
   Maintenance;
   free and evaluation users are told it's a free update.

**Stop announcing it** on the same page withdraws the notice, for example if a release has to be
pulled.

Announcing also adds the version to **All versions** on the same page, which is the list customers
download from at account.filescape.app/versions. **Hide** takes a pulled version off that list.
To list a version released before the catalogue existed, fill in the form and press **Add to the
download lists only** (it isn't announced), after uploading its files under `<version>/` in R2.

## 11. Try a real purchase

1. In Paddle, create a 100% discount code limited to one use.
2. Buy at https://filescape.app/buy with it. The serial should arrive by email within a minute.
3. Activate it in a release build. https://admin.filescape.app shows the licence, its activation
   and history.
4. Revoke it in the admin site, or refund the transaction in Paddle and watch the refund flag
   appear on the licence.

## Optional: rate limiting

Cloudflare's free plan includes one rate-limiting rule. **Security > WAF > Rate limiting rules**:
`http.host eq "api.filescape.app"`, 30 requests per 10 seconds per IP, Block. Serial numbers can't
be guessed (80 random bits), so this is only against floods. If you'd rather protect the account
page's sign-in form instead, use `http.host eq "account.filescape.app" and http.request.method eq
"POST"`, 10 requests per minute per IP; the server already sends at most one sign-in email a minute
per address.

## Updating a server set up before Phase 2

Phase 2 (24 Nov 2026) adds computer seats and customer sign-in. On a server that's already running:

1. `npx wrangler@4 d1 migrations apply filescape-licences --remote` (applies
   `0002_seats_and_account.sql`: two activation columns and the sign-in tables).
2. Deploy the Worker (step 6).
3. In the admin site, **Settings**: "Computers at once, new perpetual licences" should say 3. If it
   was saved as 10 earlier, set it to 3. Existing test licences keep their own limit; change one
   on its page if needed.
4. Deploy the website (step 8) for the new "3 computers at once" copy, terms and privacy text.

Sign-in links are emailed through Resend (step 5), so the account page needs `RESEND_API_KEY`.

## Updating a server for the contact form

The contact form (24 Nov 2026) replaces the support@ address. On a server that's already running:

1. `npx wrangler@4 d1 migrations apply filescape-licences --remote` (applies
   `0003_contact_messages.sql`: the `messages` and `message_replies` tables).
2. Optionally `npx wrangler@4 secret put CONTACT_NOTIFY_TO` (step 5.4).
3. Deploy the Worker (step 6), then the website (step 8).
4. Send a test message at https://filescape.app/contact and answer it at
   https://admin.filescape.app/messages.

## Updating a server for software maintenance

Maintenance stage 1 (28 Nov 2026) keeps a ledger behind each perpetual licence's "updates
included until" date. Nothing customers see changes, and no licence's date moves. On a server
that's already running:

1. `npx wrangler@4 d1 migrations apply filescape-licences --remote` (applies
   `0004_maintenance.sql`: the `maintenance_grants` ledger, a purchase grant for every existing
   perpetual licence with the date it already has, and two licence columns).
2. Deploy the Worker (step 6).
3. Open any perpetual licence in the admin site: its new **Maintenance** card should show one
   Purchase row with the same date as before. **Maintenance** in the top bar extends several
   licences at once.

## Turning on maintenance renewals

Stage 2 (28 Nov 2026) lets customers extend maintenance for 12 months through Paddle, from the
account page or from the app. Renewals are off until you switch them on in admin.

1. In Paddle (live account): **Catalog > Products > New product**, name "Filescape Pro maintenance
   renewal", tax category Standard digital goods. Add prices, each **One-time** (not recurring),
   quantity fixed at 1:
   - Launch: $2.99 (in use while Pro sells at $4.99)
   - Standard: $5.99 (for when Pro is $9.99)

   Copy each price ID (`pri_…`). Don't add them to `PADDLE_PRICE_IDS`: that list issues new
   licences.
2. If Paddle asks for website approval of account.filescape.app, add it under **Checkout > Website
   approval** (the checkout opens there).
3. `npx wrangler@4 d1 migrations apply filescape-licences --remote` (applies `0005_renewals.sql`).
4. Deploy the Worker (step 6). `PADDLE_CLIENT_TOKEN` is already in `wrangler.toml`.
5. On https://admin.filescape.app/renewals: put the launch price ID under **Standard**, shown as
   `$2.99`, tick **Offer renewals**, Save. For a promotion, add a promotional price and tick
   **Offer the promotional price**; Paddle discount codes also work at checkout.
6. Test: sign in at https://account.filescape.app, press **Extend maintenance**, pay (a 100%
   discount code works), and check the licence's Maintenance card in admin shows the renewal.

Paid renewals that couldn't be applied (a revoked licence, a checkout without a valid renewal
link) are listed under **Payments that need attention** on the Renewals page; refund those in
Paddle.

## Updating a server for the account dashboard

Stage 3 (28 Nov 2026) turns account.filescape.app into a dashboard: licence and maintenance
status, downloads, purchase and renewal history with Paddle receipts, reminder preferences, and
changing the email address (confirmed from the new address, with a 14-day undo link sent to the
old one). Sign-in and "email my serial numbers" are limited to 10 an hour per network.

1. `npx wrangler@4 d1 migrations apply filescape-licences --remote` (applies
   `0006_account_dashboard.sql`).
2. Deploy the Worker (step 6).

Receipt links need `PADDLE_API_KEY` with permission to read transactions; without it the
history says Paddle emailed the receipt.

## Updating a server for the release catalogue

Stage 5 (28 Nov 2026) keeps every version, not only the newest, so customers can download the
newest version their licence unlocks.

1. `npx wrangler@4 d1 migrations apply filescape-licences --remote` (applies
   `0007_release_catalogue.sql`; the release announced now becomes the first entry).
2. Deploy the Worker (step 6).
3. Upload the current release's files again under its version number (step 9), so the version
   links work.

## Local development

```
cd licensing/worker
cp dev.vars.example .dev.vars                      # then put keygen output in it
npx wrangler@4 d1 migrations apply filescape-licences --local
npx wrangler@4 dev
```

With `DEV=1` the admin site is at http://localhost:8787/admin/ and the customer pages at
http://localhost:8787/account/. Run the app against it with
`FILESCAPE_LICENCE_SERVER=http://localhost:8787`, built with `FILESCAPE_LICENCE_PUBKEY` set to the
`public_key` shown at http://localhost:8787/v1/health.

Tests: `cargo test --manifest-path licensing/worker/Cargo.toml` (runs against SQLite, the engine
D1 uses).
