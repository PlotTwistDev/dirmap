#![allow(async_fn_in_trait)]
//! Filescape licence server.
//!
//! One Cloudflare Worker answers three hosts:
//!
//! - `api.filescape.app`: the JSON API the app uses ([`api`]) and the Paddle webhook ([`paddle`])
//! - `account.filescape.app`: customer pages: offline activation, "email me my licence", the
//!   page after a purchase ([`account`])
//! - `admin.filescape.app`: the web admin, behind Cloudflare Access ([`admin`]), including the
//!   contact form's messages ([`contact`], which also takes `POST api.filescape.app/v1/contact`)
//!
//! Everything but [`cf`] (the Worker entry point and D1) is plain Rust over two small traits,
//! [`Db`] and [`Net`], so the tests run natively against SQLite (which D1 is) with a fake network.
//! The activation rules themselves come from `filescape-licence` ([`filescape_licence::rules`]),
//! the same code the app's test mode runs.

pub mod access;
pub mod account;
pub mod admin;
pub mod api;
pub mod contact;
pub mod db;
pub mod mail;
pub mod maintenance;
pub mod paddle;
pub mod portal;
pub mod renewal;
pub mod secret;
pub mod store;
pub mod web;

#[cfg(target_arch = "wasm32")]
mod cf;

pub use db::{Db, Row, V};
use filescape_licence::Ts;
use filescape_licence::sign::Signer;

/// Outbound HTTP (Paddle's API, Resend, Cloudflare Access keys).
pub trait Net {
    /// Returns the status and body. `Err` only when nothing came back.
    async fn send(&self, method: &str, url: &str, headers: &[(&str, String)], body: Option<String>) -> Result<(u16, String), String>;
}

/// Settings and secrets, from the Worker's environment (see wrangler.toml and SETUP.md).
pub struct Config {
    pub signer: Signer,
    /// AES-256 key for serials kept to re-send (`SERIAL_KEY`).
    pub serial_key: [u8; 32],
    pub paddle: Option<PaddleConfig>,
    /// Resend API key and From address. Without them nothing is emailed (the admin site shows
    /// serials once instead).
    pub mail: Option<MailConfig>,
    /// Cloudflare Access team domain and the admin application's audience tag. `None` only in
    /// local development (`DEV = "1"`), where the admin site is open.
    pub access: Option<AccessConfig>,
    /// Local development: `/admin/…` and `/account/…` reach those sites on any host.
    pub dev: bool,
    pub hosts: Hosts,
    pub contact: ContactConfig,
}

/// The contact form ([`contact`]).
#[derive(Default)]
pub struct ContactConfig {
    /// Where new messages are announced by email (`CONTACT_NOTIFY_TO`, a secret so the address
    /// isn't in the repository). `None`: messages only appear on the admin site.
    pub notify_to: Option<String>,
    /// Cloudflare Turnstile secret key (`TURNSTILE_SECRET`). When set, the form must pass it.
    pub turnstile_secret: Option<String>,
}

pub struct PaddleConfig {
    pub webhook_secret: String,
    pub api_key: Option<String>,
    /// `https://api.paddle.com` or `https://sandbox-api.paddle.com`.
    pub api_base: String,
    /// Only purchases of these price IDs issue licences (empty: any).
    pub price_ids: Vec<String>,
    pub update_months: u32,
    /// Paddle.js client-side token (`PADDLE_CLIENT_TOKEN`, public: it's in every buyer's browser),
    /// for the renewal checkout on account.filescape.app. `None`: renewals can't be bought.
    pub client_token: Option<String>,
    /// A Paddle sandbox account (`PADDLE_ENV = "sandbox"`).
    pub sandbox: bool,
}

pub struct MailConfig {
    pub resend_key: String,
    pub from: String,
    pub reply_to: Option<String>,
}

pub struct AccessConfig {
    /// e.g. `filescape.cloudflareaccess.com`
    pub team_domain: String,
    pub audience: String,
}

pub struct Hosts {
    pub api: String,
    pub account: String,
    pub admin: String,
    pub site: String,
}

impl Default for Hosts {
    fn default() -> Self {
        Hosts {
            api: "api.filescape.app".into(),
            account: "account.filescape.app".into(),
            admin: "admin.filescape.app".into(),
            site: "filescape.app".into(),
        }
    }
}

/// An HTTP request, as the Worker received it.
#[derive(Debug, Clone, Default)]
pub struct Req {
    pub method: String,
    pub host: String,
    pub path: String,
    pub query: String,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn query_param(&self, name: &str) -> Option<String> {
        web::parse_form(&self.query).into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
}

#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Resp {
    pub fn new(status: u16, content_type: &str, body: String) -> Resp {
        let mut r = Resp { status, headers: vec![("content-type".into(), content_type.into())], body };
        r.headers.push(("cache-control".into(), "no-store".into()));
        r
    }

    pub fn json<T: serde::Serialize>(status: u16, v: &T) -> Resp {
        Resp::new(status, "application/json", serde_json::to_string(v).unwrap())
    }

    pub fn html(status: u16, body: String) -> Resp {
        let mut r = Resp::new(status, "text/html; charset=utf-8", body);
        r.headers.push(("content-security-policy".into(), "default-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; frame-ancestors 'none'".into()));
        r.headers.push(("x-content-type-options".into(), "nosniff".into()));
        r.headers.push(("referrer-policy".into(), "no-referrer".into()));
        r
    }

    pub fn text(status: u16, body: &str) -> Resp {
        Resp::new(status, "text/plain; charset=utf-8", body.into())
    }

    pub fn redirect(to: &str) -> Resp {
        let mut r = Resp::new(303, "text/plain", String::new());
        r.headers.push(("location".into(), to.into()));
        r
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// What a request needs: the database, the network, the settings and the time.
pub struct Ctx<'a, D: Db, N: Net> {
    pub db: &'a D,
    pub net: &'a N,
    pub cfg: &'a Config,
    pub now: Ts,
    pub rng: ring::rand::SystemRandom,
}

impl<'a, D: Db, N: Net> Ctx<'a, D, N> {
    pub fn new(db: &'a D, net: &'a N, cfg: &'a Config, now: Ts) -> Self {
        Ctx { db, net, cfg, now, rng: ring::rand::SystemRandom::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Site {
    Api,
    Account,
    Admin,
}

/// Which site a request is for, and its path within that site.
fn site_of(cfg: &Config, req: &Req) -> (Site, String, String) {
    let host = req.host.split(':').next().unwrap_or("").to_ascii_lowercase();
    if cfg.dev {
        for (prefix, site) in [("/admin", Site::Admin), ("/account", Site::Account)] {
            if req.path == prefix || req.path.starts_with(&format!("{prefix}/")) {
                let rest = req.path[prefix.len()..].to_string();
                return (site, if rest.is_empty() { "/".into() } else { rest }, prefix.into());
            }
        }
    }
    let site = if host == cfg.hosts.admin {
        Site::Admin
    } else if host == cfg.hosts.account {
        Site::Account
    } else {
        Site::Api
    };
    (site, req.path.clone(), String::new())
}

/// Handles one request.
pub async fn handle<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req) -> Resp {
    let (site, path, base) = site_of(cx.cfg, req);
    let r = match site {
        Site::Api => api::route(cx, req, &path).await,
        Site::Account => account::route(cx, req, &path, &base).await,
        Site::Admin => match access::check(cx, req).await {
            Ok(actor) => admin::route(cx, req, &path, &base, &actor).await,
            Err(why) => Ok(Resp::text(403, &format!("Not allowed: {why}"))),
        },
    };
    r.unwrap_or_else(|e| {
        eprintln!("error: {e}");
        Resp::text(500, "Something went wrong on the licence server. Please try again.")
    })
}
