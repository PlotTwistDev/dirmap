# Setting up the Filescape licence server

Everything runs on Cloudflare: the licence server (a Worker, with a D1 database) at
api.filescape.app, account.filescape.app and admin.filescape.app; the website on Cloudflare Pages
at filescape.app; downloads on R2 at downloads.filescape.app. Payments go through Paddle, licence
emails through Resend.

This is done once. It takes about an hour. Commands run from the repository folder on your own
computer. You need Rust, Node.js 20 or later, and the filescape.app domain on Cloudflare (DNS
managed there).

**Secrets never go in chat, email or the repository.** You paste them straight into Cloudflare
(`wrangler secret put …` asks for the value) and keep a copy in your password manager.

## 1. Make the keys

```
cargo run --manifest-path licensing/worker/Cargo.toml --example keygen
```

It prints three values. Save all three in your password manager now:

- `SIGNING_KEY`: signs every licence. A Worker secret.
- `SERIAL_KEY`: encrypts stored serials so they can be emailed again. A Worker secret.
- `FILESCAPE_LICENCE_PUBKEY`: the public half, built into the app (step 10). Not secret.

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
4. In `wrangler.toml`, set `ACCESS_TEAM_DOMAIN` to the team domain and `ACCESS_AUD` to the tag.

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

It builds the Rust code (the first build installs `worker-build` and takes a few minutes) and
creates the three hostnames. Check https://api.filescape.app/v1/health: `key_id` must match the
key ID keygen printed.

## 7. Paddle

The Filescape product and price are already on your live Paddle account
(`pri_01m84qwgg1a8858drzqk2rvzpz`, set in `wrangler.toml` as `PADDLE_PRICE_IDS`). Paddle sends
notifications for your other product too; the server ignores anything without this price.

1. **Developer tools > Notifications > New destination**: URL `https://api.filescape.app/v1/paddle`,
   events `transaction.completed`, `adjustment.created`, `adjustment.updated`. Save, open it, copy
   the **secret key** into `PADDLE_WEBHOOK_SECRET` (step 3).
2. Use **Send test notification** for `transaction.completed`: the server answers "ignored … not a
   Filescape purchase" (the sample isn't the Filescape price), which shows the signature check works.
3. **Checkout > Checkout settings**: add `https://filescape.app/buy` as a default payment link if
   Paddle asks for one.
4. After the website is live (step 8), add filescape.app under **Checkout > Website approval**.

## 8. The website (Cloudflare Pages)

Fill in the drafts first: `[SELLER]` (your trading name; no address is needed, since Paddle is
the seller of record and the pages give support@filescape.app as the contact) in
`licensing/site/terms.html` and `privacy.html`, and the refund period in `refunds.html`. Then:

```
npx wrangler@4 pages deploy licensing/site --project-name filescape-site
```

In the dashboard, **Workers & Pages > filescape-site > Custom domains**: add `filescape.app` (and
`www.filescape.app` if you want it). Then submit the domain to Paddle (step 7.4).

## 9. Downloads (R2)

1. **R2 > Create bucket** `filescape-downloads`. **Settings > Custom domains**: `downloads.filescape.app`.
2. Upload each release:
   ```
   npx wrangler@4 r2 object put filescape-downloads/latest/Filescape-windows.zip --file Filescape-windows.zip --remote
   ```
   The website's download buttons point at `latest/Filescape-windows.zip` and
   `latest/Filescape-macos.dmg` (`licensing/site/config.js`). Attach the same files to a GitHub
   release as a mirror.

## 10. App builds for sale

Build with the public key from step 1, and without test mode:

```
set FILESCAPE_LICENCE_PUBKEY=<public key>          (Windows; export … on macOS)
cargo build --release --no-default-features
```

`--no-default-features` removes the test-serial mode (`FILESCAPE_LICENCE_DEV`). Phase 4 makes
that the default.

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
be guessed (80 random bits), so this is only against floods.

## Local development

```
cd licensing/worker
cp dev.vars.example .dev.vars                      # then put keygen output in it
npx wrangler@4 d1 migrations apply filescape-licences --local
npx wrangler@4 dev
```

With `DEV=1` the admin site is at http://localhost:8787/admin/ and the customer pages at
http://localhost:8787/account/. Run the app against it with
`FILESCAPE_LICENCE_SERVER=http://localhost:8787` and `FILESCAPE_LICENCE_PUBKEY` set to the
`public_key` shown at http://localhost:8787/v1/health.

Tests: `cargo test --manifest-path licensing/worker/Cargo.toml` (runs against SQLite, the engine
D1 uses).
