//! The contact form: filescape.app/contact posts here (`POST api.filescape.app/v1/contact`).
//!
//! Messages are stored in D1 and read and answered on admin.filescape.app/messages, so nobody's
//! personal email address is on the site or in a reply. When `CONTACT_NOTIFY_TO` is set (a
//! Worker secret), each new message is also emailed there through Resend, with a link to answer it
//! on the admin site.
//!
//! Spam: a hidden honeypot field, a minimum time on the page (sent by the page's script), limits
//! per sender address and per IP (kept only as a keyed hash), a daily cap, and Cloudflare
//! Turnstile when `TURNSTILE_SECRET` is set. Silent drops (honeypot, too fast) answer as if the
//! message was sent, so bots learn nothing.

use crate::db::{self, Row};
use crate::web::{Form, date, encode};
use crate::{Ctx, Db, Net, Req, Resp, mail};
use filescape_licence::{DAY, Ts};
use ring::rand::SecureRandom;

/// The topics the form offers; anything else is stored as "Other".
pub const TOPICS: &[&str] = &["Question", "Licence or purchase", "Bug report", "Feature request", "Refund", "Privacy request", "Other"];

pub const MAX_BODY: usize = 5000;
const MIN_BODY: usize = 10;
/// Messages a sender address or IP may send in an hour.
const PER_HOUR: i64 = 5;
/// All messages in a day, so a flood can't fill the database or the notification inbox.
const PER_DAY: i64 = 300;
/// Faster than this from page load to send is a bot (only when the page's script says).
const MIN_MILLIS: i64 = 3000;

/// A stored message.
#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: String,
    pub created_at: Ts,
    pub name: Option<String>,
    pub email: String,
    pub topic: String,
    pub body: String,
    pub status: String,
    pub notified_at: Option<Ts>,
}

#[derive(Debug, Clone)]
pub struct ReplyRow {
    pub at: Ts,
    pub actor: String,
    pub body: String,
}

const COLS: &str = "id, created_at, name, email, topic, body, status, notified_at";

fn row(r: &Row) -> Result<MessageRow, String> {
    Ok(MessageRow {
        id: db::req_text(r, "id")?,
        created_at: db::int(r, "created_at").unwrap_or(0),
        name: db::text(r, "name"),
        email: db::req_text(r, "email")?,
        topic: db::req_text(r, "topic")?,
        body: db::req_text(r, "body")?,
        status: db::text(r, "status").unwrap_or_else(|| "new".into()),
        notified_at: db::int(r, "notified_at"),
    })
}

/// Why a message wasn't accepted, as the form shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Email,
    Message,
    Challenge,
    TooMany,
    Busy,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::Email => "email",
            Refusal::Message => "message",
            Refusal::Challenge => "challenge",
            Refusal::TooMany => "too_many",
            Refusal::Busy => "busy",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Refusal::Email => "Please enter the email address we should reply to.",
            Refusal::Message => "Please write a message (between 10 and 5,000 characters).",
            Refusal::Challenge => "The spam check didn't pass. Please try again.",
            Refusal::TooMany => "You've sent several messages in the last hour. Please wait a while before sending another.",
            Refusal::Busy => "The contact form is very busy right now. Please try again tomorrow.",
        }
    }
}

fn one_line(s: &str, max: usize) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

fn valid_email(e: &str) -> bool {
    let Some((user, domain)) = e.split_once('@') else { return false };
    !user.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !domain.contains('@') && e.len() <= 200 && !e.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"'))
}

/// Origins allowed to post the form (the website; localhost in development).
fn origin_ok<D: Db, N: Net>(cx: &Ctx<'_, D, N>, origin: &str) -> bool {
    let o = origin.trim_end_matches('/');
    o == format!("https://{}", cx.cfg.hosts.site) || o == format!("https://www.{}", cx.cfg.hosts.site) || (cx.cfg.dev && (o.starts_with("http://localhost") || o.starts_with("http://127.0.0.1")))
}

fn ip_hash<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req) -> Option<String> {
    let ip = req.header("cf-connecting-ip")?.trim();
    if ip.is_empty() {
        return None;
    }
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &cx.cfg.serial_key);
    let tag = ring::hmac::sign(&key, format!("contact-ip:{ip}").as_bytes());
    Some(data_encoding::HEXLOWER.encode(&tag.as_ref()[..12]))
}

impl<D: Db, N: Net> Ctx<'_, D, N> {
    async fn count(&self, sql: &str, args: Vec<db::V>) -> Result<i64, String> {
        Ok(self.db.first(sql, args).await?.and_then(|r| db::int(&r, "n")).unwrap_or(0))
    }

    pub async fn message(&self, id: &str) -> Result<Option<MessageRow>, String> {
        self.db.first(&format!("SELECT {COLS} FROM messages WHERE id = ?1"), vec![id.into()]).await?.map(|r| row(&r)).transpose()
    }

    /// Newest first. `open`: everything not closed.
    pub async fn messages(&self, open: bool, limit: u32) -> Result<Vec<MessageRow>, String> {
        let sql = if open {
            format!("SELECT {COLS} FROM messages WHERE status != 'closed' ORDER BY created_at DESC LIMIT ?1")
        } else {
            format!("SELECT {COLS} FROM messages ORDER BY created_at DESC LIMIT ?1")
        };
        self.db.all(&sql, vec![limit.into()]).await?.iter().map(row).collect()
    }

    pub async fn messages_from(&self, email: &str) -> Result<Vec<MessageRow>, String> {
        self.db.all(&format!("SELECT {COLS} FROM messages WHERE email = ?1 ORDER BY created_at DESC LIMIT 50"), vec![email.into()]).await?.iter().map(row).collect()
    }

    pub async fn new_message_count(&self) -> Result<i64, String> {
        self.count("SELECT count(*) AS n FROM messages WHERE status = 'new'", vec![]).await
    }

    pub async fn replies(&self, message_id: &str) -> Result<Vec<ReplyRow>, String> {
        Ok(self
            .db
            .all("SELECT at, actor, body FROM message_replies WHERE message_id = ?1 ORDER BY at, id", vec![message_id.into()])
            .await?
            .iter()
            .map(|r| ReplyRow { at: db::int(r, "at").unwrap_or(0), actor: db::text(r, "actor").unwrap_or_default(), body: db::text(r, "body").unwrap_or_default() })
            .collect())
    }

    pub async fn set_message_status(&self, id: &str, status: &str) -> Result<bool, String> {
        Ok(self.db.run("UPDATE messages SET status = ?2, updated_at = ?3 WHERE id = ?1", vec![id.into(), status.into(), self.now.into()]).await? > 0)
    }

    pub async fn delete_message(&self, id: &str) -> Result<(), String> {
        self.db.batch(vec![("DELETE FROM message_replies WHERE message_id = ?1", vec![id.into()]), ("DELETE FROM messages WHERE id = ?1", vec![id.into()])]).await?;
        Ok(())
    }

    /// Emails a reply to the sender from `MAIL_FROM` and records it. `Ok(Err)`: not sent (why).
    pub async fn reply_to_message(&self, m: &MessageRow, text: &str, actor: &str) -> Result<Result<(), String>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Err("Write a reply first.".into()));
        }
        if text.chars().count() > 20_000 {
            return Ok(Err("That reply is too long.".into()));
        }
        match mail::send(self, &reply_message(m, text, &self.cfg.hosts.site)).await {
            Ok(true) => {}
            Ok(false) => return Ok(Err("Email isn't set up on the licence server (RESEND_API_KEY, MAIL_FROM), so the reply wasn't sent.".into())),
            Err(e) => return Ok(Err(format!("The reply wasn't sent: {e}"))),
        }
        self.db
            .batch(vec![
                ("INSERT INTO message_replies (message_id, at, actor, body) VALUES (?1, ?2, ?3, ?4)", vec![m.id.as_str().into(), self.now.into(), actor.into(), text.into()]),
                ("UPDATE messages SET status = 'answered', updated_at = ?2 WHERE id = ?1", vec![m.id.as_str().into(), self.now.into()]),
            ])
            .await?;
        Ok(Ok(()))
    }

    async fn turnstile_ok(&self, secret: &str, token: &str, req: &Req) -> Result<bool, String> {
        if token.is_empty() || token.len() > 4096 {
            return Ok(false);
        }
        let mut body = format!("secret={}&response={}", encode(secret), encode(token));
        if let Some(ip) = req.header("cf-connecting-ip") {
            body.push_str(&format!("&remoteip={}", encode(ip)));
        }
        let (status, text) = self
            .net
            .send("POST", "https://challenges.cloudflare.com/turnstile/v0/siteverify", &[("content-type", "application/x-www-form-urlencoded".into())], Some(body))
            .await?;
        if status != 200 {
            return Err(format!("Turnstile said {status}"));
        }
        Ok(serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["success"].as_bool()).unwrap_or(false))
    }

    /// Checks and stores a message from the form. `Ok(None)`: accepted (or silently dropped).
    pub async fn receive_message(&self, req: &Req, form: &Form) -> Result<Option<Refusal>, String> {
        // Bots fill every field, and post the moment the page loads.
        if form.has("website") {
            return Ok(None);
        }
        if let Ok(ms) = form.get("elapsed").parse::<i64>()
            && ms < MIN_MILLIS
        {
            return Ok(None);
        }
        let email = form.get("email").to_lowercase();
        if !valid_email(&email) {
            return Ok(Some(Refusal::Email));
        }
        let body = form.get("message").replace("\r\n", "\n");
        let n = body.chars().count();
        if !(MIN_BODY..=MAX_BODY).contains(&n) {
            return Ok(Some(Refusal::Message));
        }
        if let Some(secret) = &self.cfg.contact.turnstile_secret
            && !self.turnstile_ok(secret, &form.get("cf-turnstile-response"), req).await?
        {
            return Ok(Some(Refusal::Challenge));
        }
        let name = Some(one_line(&form.get("name"), 100)).filter(|n| !n.is_empty());
        let topic = form.get("topic");
        let topic = TOPICS.iter().find(|t| **t == topic).copied().unwrap_or("Other");
        let ip = ip_hash(self, req);
        let hour = self.now - 3600;
        if self.count("SELECT count(*) AS n FROM messages WHERE email = ?1 AND created_at > ?2", vec![email.as_str().into(), hour.into()]).await? >= PER_HOUR {
            return Ok(Some(Refusal::TooMany));
        }
        if let Some(ip) = &ip
            && self.count("SELECT count(*) AS n FROM messages WHERE ip_hash = ?1 AND created_at > ?2", vec![ip.as_str().into(), hour.into()]).await? >= PER_HOUR
        {
            return Ok(Some(Refusal::TooMany));
        }
        if self.count("SELECT count(*) AS n FROM messages WHERE created_at > ?1", vec![(self.now - DAY).into()]).await? >= PER_DAY {
            return Ok(Some(Refusal::Busy));
        }
        let mut b = [0u8; 8];
        self.rng.fill(&mut b).map_err(|_| "random")?;
        let id = format!("m_{}", data_encoding::HEXLOWER.encode(&b));
        self.db
            .run(
                "INSERT INTO messages (id, created_at, name, email, topic, body, ip_hash, status, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'new', ?2)",
                vec![id.as_str().into(), self.now.into(), name.clone().into(), email.as_str().into(), topic.into(), body.as_str().into(), ip.into()],
            )
            .await?;
        if let Some(to) = &self.cfg.contact.notify_to {
            let m = MessageRow { id: id.clone(), created_at: self.now, name, email, topic: topic.into(), body, status: "new".into(), notified_at: None };
            match mail::send(self, &notify_message(&m, to, &self.cfg.hosts.admin)).await {
                Ok(true) => {
                    self.db.run("UPDATE messages SET notified_at = ?2 WHERE id = ?1", vec![id.as_str().into(), self.now.into()]).await?;
                }
                Ok(false) => {}
                Err(e) => eprintln!("contact notify: {e}"),
            }
        }
        Ok(None)
    }
}

fn quote(s: &str) -> String {
    s.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")
}

/// The email to the site owner about a new message. It has no Reply-To: answering from the admin
/// site keeps the owner's own address private.
pub fn notify_message(m: &MessageRow, to: &str, admin_host: &str) -> mail::Message {
    let from = match &m.name {
        Some(n) => format!("{n} ({})", m.email),
        None => m.email.clone(),
    };
    mail::Message {
        to: to.into(),
        subject: format!("Filescape contact form: {} from {}", m.topic, one_line(m.name.as_deref().unwrap_or(&m.email), 60)),
        text: format!(
            "New message from the Filescape contact form.\n\nFrom: {from}\nTopic: {}\nReceived: {}\n\n{}\n\nAnswer it at https://{admin_host}/messages/{} so the reply comes from Filescape and your own address stays private. Replying to this email doesn't reach the sender.",
            m.topic,
            date(m.created_at),
            m.body,
            m.id
        ),
        reply_to: None,
    }
}

/// The reply the sender gets.
pub fn reply_message(m: &MessageRow, text: &str, site_host: &str) -> mail::Message {
    let hello = m.name.as_deref().map_or("Hello,".to_string(), |n| format!("Hello {n},"));
    mail::Message {
        to: m.email.clone(),
        subject: format!("Re: your message to Filescape ({})", m.topic),
        text: format!(
            "{hello}\n\n{text}\n\nFilescape (filescape.app)\n\nTo write back, please use https://{site_host}/contact (replies to this email aren't read).\n\nYou wrote on {}:\n\n{}",
            date(m.created_at),
            quote(&m.body)
        ),
        reply_to: None,
    }
}

fn cors(mut r: Resp, origin: Option<&str>) -> Resp {
    if let Some(o) = origin {
        r.headers.push(("access-control-allow-origin".into(), o.trim_end_matches('/').into()));
        r.headers.push(("vary".into(), "origin".into()));
    }
    r
}

/// `/v1/contact` on the API host.
pub async fn route<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req) -> Result<Resp, String> {
    let origin = req.header("origin");
    let allowed = origin.is_none_or(|o| origin_ok(cx, o));
    let site = if cx.cfg.dev && origin.is_some_and(|o| o.starts_with("http://")) { origin.unwrap_or_default().trim_end_matches('/').to_string() } else { format!("https://{}", cx.cfg.hosts.site) };
    match req.method.as_str() {
        "OPTIONS" if allowed => {
            let mut r = cors(Resp::new(204, "text/plain", String::new()), origin);
            r.headers.push(("access-control-allow-methods".into(), "POST".into()));
            r.headers.push(("access-control-allow-headers".into(), "content-type, accept".into()));
            r.headers.push(("access-control-max-age".into(), "86400".into()));
            Ok(r)
        }
        "POST" => {
            if !allowed {
                return Ok(Resp::text(403, "The contact form only accepts messages from filescape.app."));
            }
            let json = req.header("accept").is_some_and(|a| a.contains("application/json"));
            if req.body.len() > 64 * 1024 {
                return Ok(cors(Resp::json(413, &serde_json::json!({ "error": "message", "message": Refusal::Message.message() })), origin));
            }
            let form = Form::parse(&req.text());
            let refusal = cx.receive_message(req, &form).await?;
            Ok(match (refusal, json) {
                (None, true) => cors(Resp::json(200, &serde_json::json!({ "ok": true })), origin),
                (Some(r), true) => cors(Resp::json(if matches!(r, Refusal::TooMany | Refusal::Busy) { 429 } else { 400 }, &serde_json::json!({ "error": r.code(), "message": r.message() })), origin),
                // Without the page's script: back to the page, which says what happened.
                (None, false) => Resp::redirect(&format!("{site}/contact?sent=1")),
                (Some(r), false) => Resp::redirect(&format!("{site}/contact?error={}", r.code())),
            })
        }
        _ => Ok(Resp::text(405, "Method not allowed.")),
    }
}
