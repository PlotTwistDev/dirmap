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

## 5. Email (Resend) and support mail

1. Sign up at resend.com, **Domains > Add domain** `filescape.app`, and add the DNS records it shows
   in Cloudflare DNS. Wait for it to verify.
2. **API Keys > Create**, "Sending access", domain filescape.app. Put it in `RESEND_API_KEY` (step 3).
3. Licence emails come from `licences@filescape.app` with replies to `support@filescape.app`
   (`MAIL_FROM`, `MAIL_REPLY_TO` in `wrangler.toml`). To receive support mail, turn on Cloudflare
   **Email Routing** for filescape.app and forward `support@` to your own address.

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

`licensing/site` is the whole site, ready to deploy: the homepage, `/buy`, `/terms`, `/privacy`
and `/refunds`. The homepage is generated from `licensing/site-src`; after changing anything
there, run `python3 licensing/site-src/build.py` to rebuild `site/index.html`. The Paddle token
and price for both checkout buttons are in `licensing/site/config.js`.

```
npx wrangler@4 pages deploy licensing/site --project-name filescape-site --branch main
```

The first run asks to create the project; give `main` as its production branch. `--branch main`
matters: without it wrangler uses your checkout's git branch, and anything other than the
production branch goes to a preview address instead of filescape.app.

In the dashboard, **Workers & Pages > filescape-site > Custom domains**: add `filescape.app` (and
`www.filescape.app` if you want it). Then submit the domain to Paddle (step 7.4).

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
4. Check each link downloads with the right name: `curl -sI https://downloads.filescape.app/latest/linux-deb`
   shows the `content-disposition` line.

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
   minutes). A licence whose updates ended before the release date is told so, with a link to buy;
   free and evaluation users are told it's a free update.

**Stop announcing it** on the same page withdraws the notice, for example if a release has to be
pulled.

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
