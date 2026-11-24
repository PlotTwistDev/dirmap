//! The Cloudflare Worker: environment, D1, outbound fetch, and the request/response conversion.

use crate::db::{Db, Row, Stmt, V};
use crate::{AccessConfig, Config, ContactConfig, Ctx, Hosts, MailConfig, Net, PaddleConfig, Req};
use filescape_licence::sign::Signer;
use wasm_bindgen::JsValue;
use worker::*;

struct D1(D1Database);

fn bind(v: &[V]) -> Vec<JsValue> {
    v.iter()
        .map(|x| match x {
            V::Null => JsValue::NULL,
            V::Int(i) => JsValue::from_f64(*i as f64),
            V::Text(s) => JsValue::from_str(s),
        })
        .collect()
}

impl Db for D1 {
    async fn all(&self, sql: &str, args: Vec<V>) -> std::result::Result<Vec<Row>, String> {
        let st = self.0.prepare(sql).bind(&bind(&args)).map_err(|e| e.to_string())?;
        let r = st.all().await.map_err(|e| e.to_string())?;
        r.results::<Row>().map_err(|e| e.to_string())
    }

    async fn batch(&self, stmts: Vec<Stmt>) -> std::result::Result<Vec<usize>, String> {
        let mut prepared = Vec::with_capacity(stmts.len());
        for (sql, args) in &stmts {
            prepared.push(self.0.prepare(*sql).bind(&bind(args)).map_err(|e| e.to_string())?);
        }
        let results = self.0.batch(prepared).await.map_err(|e| e.to_string())?;
        Ok(results.iter().map(|r| r.meta().ok().flatten().and_then(|m| m.changes).unwrap_or(0)).collect())
    }
}

struct WebNet;

impl Net for WebNet {
    async fn send(&self, method: &str, url: &str, headers: &[(&str, String)], body: Option<String>) -> std::result::Result<(u16, String), String> {
        let h = Headers::new();
        for (k, v) in headers {
            h.set(k, v).map_err(|e| e.to_string())?;
        }
        let mut init = RequestInit::new();
        init.with_method(Method::from(method.to_string())).with_headers(h).with_body(body.map(|b| JsValue::from_str(&b)));
        let req = Request::new_with_init(url, &init).map_err(|e| e.to_string())?;
        let mut resp = Fetch::Request(req).send().await.map_err(|e| e.to_string())?;
        let status = resp.status_code();
        Ok((status, resp.text().await.unwrap_or_default()))
    }
}

fn opt(env: &Env, name: &str) -> Option<String> {
    env.secret(name).map(|s| s.to_string()).or_else(|_| env.var(name).map(|v| v.to_string())).ok().filter(|s| !s.trim().is_empty())
}

fn config(env: &Env) -> std::result::Result<Config, String> {
    let signing = opt(env, "SIGNING_KEY").ok_or("SIGNING_KEY secret missing")?;
    let pkcs8 = data_encoding::BASE64URL_NOPAD.decode(signing.trim().trim_end_matches('=').as_bytes()).map_err(|_| "SIGNING_KEY isn't base64url")?;
    let signer = Signer::from_pkcs8(&pkcs8)?;
    let serial_key = opt(env, "SERIAL_KEY").and_then(|k| crate::secret::key32(&k)).ok_or("SERIAL_KEY secret missing or not 32 bytes")?;
    let sandbox = opt(env, "PADDLE_ENV").is_some_and(|e| e == "sandbox");
    let paddle = opt(env, "PADDLE_WEBHOOK_SECRET").map(|webhook_secret| PaddleConfig {
        webhook_secret,
        api_key: opt(env, "PADDLE_API_KEY"),
        api_base: if sandbox { "https://sandbox-api.paddle.com".into() } else { "https://api.paddle.com".into() },
        price_ids: opt(env, "PADDLE_PRICE_IDS").map(|s| s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()).unwrap_or_default(),
        update_months: opt(env, "UPDATE_MONTHS").and_then(|m| m.parse().ok()).unwrap_or(filescape_licence::DEFAULT_UPDATE_MONTHS),
    });
    let mail = match (opt(env, "RESEND_API_KEY"), opt(env, "MAIL_FROM")) {
        (Some(resend_key), Some(from)) => Some(MailConfig { resend_key, from, reply_to: opt(env, "MAIL_REPLY_TO") }),
        _ => None,
    };
    let access = match (opt(env, "ACCESS_TEAM_DOMAIN"), opt(env, "ACCESS_AUD")) {
        (Some(team_domain), Some(audience)) => Some(AccessConfig { team_domain, audience }),
        _ => None,
    };
    let mut hosts = Hosts::default();
    if let Some(h) = opt(env, "HOST_API") {
        hosts.api = h;
    }
    if let Some(h) = opt(env, "HOST_ACCOUNT") {
        hosts.account = h;
    }
    if let Some(h) = opt(env, "HOST_ADMIN") {
        hosts.admin = h;
    }
    if let Some(h) = opt(env, "HOST_SITE") {
        hosts.site = h;
    }
    let contact = ContactConfig { notify_to: opt(env, "CONTACT_NOTIFY_TO"), turnstile_secret: opt(env, "TURNSTILE_SECRET") };
    Ok(Config { signer, serial_key, paddle, mail, access, dev: opt(env, "DEV").is_some_and(|d| d == "1"), hosts, contact })
}

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let cfg = match config(&env) {
        Ok(c) => c,
        Err(e) => {
            console_error!("configuration: {e}");
            return Ok(Response::error("The licence server isn't configured.", 503)?);
        }
    };
    let url = req.url()?;
    let mut headers = Vec::new();
    for (k, v) in req.headers().entries() {
        headers.push((k.to_ascii_lowercase(), v));
    }
    let body = if matches!(req.method(), Method::Get | Method::Head) { Vec::new() } else { req.bytes().await.unwrap_or_default() };
    let r = Req {
        method: req.method().to_string(),
        host: url.host_str().unwrap_or_default().to_string(),
        path: url.path().to_string(),
        query: url.query().unwrap_or_default().to_string(),
        headers,
        body,
    };
    let db = D1(env.d1("DB")?);
    let net = WebNet;
    let now = (Date::now().as_millis() / 1000) as i64;
    let cx = Ctx::new(&db, &net, &cfg, now);
    let out = crate::handle(&cx, &r).await;
    let h = Headers::new();
    for (k, v) in &out.headers {
        h.set(k, v)?;
    }
    Ok(Response::from_bytes(out.body.into_bytes())?.with_status(out.status).with_headers(h))
}
