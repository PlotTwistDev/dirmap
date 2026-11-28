//! account.filescape.app: for customers. Sign in with an emailed link to see your licences and
//! release computers ([`crate::portal`]), offline activation, "email me my serial numbers", and
//! the page Paddle's checkout returns to.

use crate::portal::{self, Owned};
use crate::store::Fail;
use crate::web::{Form, date, esc, page};
use crate::{Ctx, Db, Net, Req, Resp, mail};
use filescape_licence::model::{LicenceKind, Validity};

fn nav(base: &str) -> String {
    format!("<b>Filescape</b><a href=\"{base}/\">Your licence</a><a href=\"{base}/activate\">Offline activation</a><a href=\"https://filescape.app/contact\">Contact</a><a href=\"https://filescape.app/\">filescape.app</a>")
}

/// A form posted from this site (not another one). Session cookies are SameSite=Lax as well.
fn same_site<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req) -> bool {
    let origin_ok = req.header("origin").is_none_or(|o| {
        let o = o.trim_end_matches('/');
        o == format!("https://{}", cx.cfg.hosts.account) || (cx.cfg.dev && (o.starts_with("http://localhost") || o.starts_with("http://127.0.0.1")))
    });
    origin_ok && req.header("sec-fetch-site").is_none_or(|s| s == "same-origin" || s == "none")
}

fn show(base: &str, title: &str, body: &str) -> Resp {
    Resp::html(200, page(&format!("{title} · Filescape"), &nav(base), body))
}

pub async fn route<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req, path: &str, base: &str) -> Result<Resp, String> {
    if req.method == "POST" && !same_site(cx, req) {
        return Ok(Resp::text(403, "That form was sent from another site."));
    }
    match (req.method.as_str(), path) {
        ("GET", "/") => match cx.session_email(req).await? {
            Some(email) => signed_in(cx, base, &email, req.query_param("msg").as_deref()).await,
            None => Ok(home(base, req.query_param("msg").as_deref())),
        },
        ("POST", "/login") => login(cx, base, &Form::parse(&req.text())).await,
        ("GET", "/signin") => Ok(signin_page(base, &req.query_param("t").unwrap_or_default())),
        ("POST", "/signin") => signin(cx, base, &Form::parse(&req.text())).await,
        ("POST", "/signout") => {
            cx.end_session(req).await?;
            let mut r = Resp::redirect(&format!("{base}/"));
            r.headers.push(("set-cookie".into(), portal::set_cookie("", 0, !cx.cfg.dev)));
            Ok(r)
        }
        ("POST", "/release") => {
            let Some(email) = cx.session_email(req).await? else { return Ok(Resp::redirect(&format!("{base}/?msg=Sign%20in%20again"))) };
            let form = Form::parse(&req.text());
            let done = cx.release_for(&email, &form.get("activation")).await?;
            let msg = if done { "Computer released. Filescape on that computer goes back to the free features at its next licence check." } else { "That computer was already released." };
            Ok(Resp::redirect(&format!("{base}/?msg={}", crate::web::encode(msg))))
        }
        ("POST", "/find") => find(cx, base, &Form::parse(&req.text())).await,
        // Extend maintenance: signed in (the licence must be registered to this email), then the
        // checkout page; the app's "Extend maintenance" goes straight to the checkout page.
        ("POST", "/renew") => {
            let Some(email) = cx.session_email(req).await? else { return Ok(Resp::redirect(&format!("{base}/?msg=Sign%20in%20again"))) };
            let id = Form::parse(&req.text()).get("licence");
            let owned = cx.licence(&id).await?.is_some_and(|r| r.lic.email.as_deref() == Some(email.as_str()));
            if !owned {
                return Ok(Resp::redirect(&format!("{base}/?msg=That%20licence%20isn%27t%20registered%20to%20you.")));
            }
            match cx.start_renewal(&id, "account", None).await? {
                Ok(t) => Ok(Resp::redirect(&format!("{base}/renew?t={t}"))),
                Err(e) => Ok(Resp::redirect(&format!("{base}/?msg={}", crate::web::encode(e.message())))),
            }
        }
        ("GET", "/renew") => renew_page(cx, base, &req.query_param("t").unwrap_or_default()).await,
        ("GET", "/renewed") => Ok(show(
            base,
            "Thank you",
            "<h1>Thank you for renewing</h1><div class=\"card\"><p>Your maintenance is extended as soon as Paddle confirms the payment, usually within a minute, and a confirmation email is on its way. Your serial number and your computers stay as they are.</p>\
             <p>Filescape picks up the new date at its next weekly licence check. To see it now, choose <b>Help &gt; Licence</b> and press <b>Check now</b>.</p></div>",
        )),
        ("GET", "/activate") => Ok(offline_page(base, None)),
        ("POST", "/activate") => offline(cx, base, req).await,
        ("GET", "/thanks") => Ok(show(
            base,
            "Thank you",
            "<h1>Thank you for buying Filescape</h1><div class=\"card\"><p>Your serial number is on its way to the email address you gave at checkout. It usually arrives within a minute; check your spam folder if it doesn't.</p>\
             <p>To activate: open Filescape, choose <b>Help &gt; Licence</b>, paste the serial number and press <b>Activate</b>.</p>\
             <p>On a Mac? The first launch needs one extra step because Filescape isn't notarised by Apple yet. <a href=\"https://filescape.app/mac\">Here's how to open it</a>.</p>\
             <p class=\"mute\">Paddle, our reseller, sends the receipt separately.</p></div><p>No email after a few minutes? <a href=\"./\">Send my serial numbers again</a>.</p>",
        )),
        _ => Ok(Resp::html(404, page("Not found", &nav(base), "<h1>Not found</h1>"))),
    }
}

fn home(base: &str, msg: Option<&str>) -> Resp {
    let m = msg.map(|m| format!("<div class=\"card\">{}</div>", esc(m))).unwrap_or_default();
    show(
        base,
        "Your licence",
        &format!(
            "<h1>Your Filescape licence</h1>{m}<div class=\"card\"><h2 style=\"margin-top:0\">See your licences and computers</h2><p>Enter the email address you bought with and we'll send you a sign-in link. Signed in, you can see which computers are using your licence and release one you no longer use.</p>\
             <form method=\"post\" action=\"{base}/login\" class=\"row\"><label>Email<input name=\"email\" type=\"email\" required size=\"32\" autocomplete=\"email\"></label><button class=\"primary\">Email me a sign-in link</button></form></div>\
             <div class=\"card\"><h2 style=\"margin-top:0\">Lost your serial number?</h2><p>Enter the email address you bought with, and we'll send every serial number registered to it.</p>\
             <form method=\"post\" action=\"{base}/find\" class=\"row\"><label>Email<input name=\"email\" type=\"email\" required size=\"32\"></label><button class=\"primary\">Email my serial numbers</button></form></div>\
             <div class=\"card\"><h2 style=\"margin-top:0\">Activating a computer without internet access?</h2><p>Use <a href=\"{base}/activate\">offline activation</a>.</p></div>\
             <div class=\"card\"><h2 style=\"margin-top:0\">Moved to a new computer?</h2><p>Deactivate Filescape on the old one (<b>Help &gt; Licence &gt; Deactivate</b>), or sign in above and release it. If the old computer is gone and you can't sign in, <a href=\"https://filescape.app/contact?topic=licence\">contact us</a> with your serial number.</p></div>"
        ),
    )
}

async fn find<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, form: &Form) -> Result<Resp, String> {
    let email = form.get("email").to_lowercase();
    let same_answer = "If that address has Filescape licences, their serial numbers are on their way. Check your spam folder if nothing arrives in a few minutes.";
    if !email.contains('@') || email.len() > 200 {
        return Ok(home(base, Some("Enter the email address you bought Filescape with.")));
    }
    // At most once every 10 minutes per address, so the form can't be used to flood someone.
    if !cx.mail_allowed(&email, "find", 600).await? {
        return Ok(home(base, Some(same_answer)));
    }
    let rows = cx.licences_by_email(&email).await?;
    let with_serials: Vec<_> = rows.into_iter().filter_map(|r| cx.serial_of(&r).map(|s| (r, s))).collect();
    if !with_serials.is_empty()
        && let Err(e) = mail::send(cx, &mail::reminder_message(&with_serials, &email)).await
    {
        eprintln!("find: {e}");
    }
    Ok(home(base, Some(same_answer)))
}

fn offline_page(base: &str, error: Option<&str>) -> Resp {
    let err = error.map(|e| format!("<div class=\"card bad\">{}</div>", esc(e))).unwrap_or_default();
    show(
        base,
        "Offline activation",
        &format!(
            "<h1>Offline activation</h1>{err}<div class=\"card\"><ol>\
             <li>On the computer without internet access, open Filescape, choose <b>Help &gt; Licence</b>, enter the serial number, open <b>Offline activation</b> and choose <b>Save Request…</b>.</li>\
             <li>Bring the request file to this computer and choose it below. You'll get a response file back.</li>\
             <li>On the first computer, choose <b>Import Response…</b> and pick the response file.</li></ol>\
             <p class=\"mute\">It uses one of the licence's activations, as activating online does, and the response only works on the computer that made the request.</p>\
             <form method=\"post\" action=\"{base}/activate\" id=\"f\"><p><input type=\"file\" id=\"file\" accept=\".json,application/json\"></p>\
             <p><label>Or paste the request file's contents<textarea name=\"request\" id=\"req\" rows=\"6\" style=\"width:100%\" required></textarea></label></p>\
             <button class=\"primary\">Get response file</button></form></div>\
             <script>document.getElementById('file').addEventListener('change',e=>{{const f=e.target.files[0];if(!f)return;f.text().then(t=>{{document.getElementById('req').value=t;document.getElementById('f').submit();}});}});</script>"
        ),
    )
}

async fn offline<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, req: &Req) -> Result<Resp, String> {
    let form = Form::parse(&req.text());
    let Some(r) = crate::api::parse_offline(&form.get("request")) else {
        return Ok(offline_page(base, Some("That isn't a Filescape activation request file. In Filescape, use Offline activation > Save Request… and choose that file here.")));
    };
    match cx.activate(&r).await {
        Ok(resp) => {
            let mut out = Resp::json(200, &resp);
            out.headers.push(("content-disposition".into(), "attachment; filename=\"filescape-activation-response.json\"".into()));
            Ok(out)
        }
        Err(Fail::No(why) | Fail::Signed(why, _)) => Ok(offline_page(base, Some(why.message()))),
        Err(Fail::Err(e)) => Err(e),
    }
}

async fn login<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, form: &Form) -> Result<Resp, String> {
    let email = form.get("email").to_lowercase();
    let same_answer = "If that address has Filescape licences, a sign-in link is on its way. It works once, for 15 minutes. Check your spam folder if nothing arrives.";
    if !email.contains('@') || email.len() > 200 {
        return Ok(home(base, Some("Enter the email address you bought Filescape with.")));
    }
    // One link a minute per address, and only to addresses that have licences.
    if cx.licences_by_email(&email).await?.is_empty() || !cx.mail_allowed(&email, "login", 60).await? {
        return Ok(home(base, Some(same_answer)));
    }
    let t = cx.new_login(&email).await?;
    let link = if cx.cfg.dev { format!("http://localhost:8787{base}/signin?t={t}") } else { format!("https://{}/signin?t={t}", cx.cfg.hosts.account) };
    if let Err(e) = mail::send(cx, &mail::login_message(&link, &email)).await {
        eprintln!("login: {e}");
    }
    Ok(home(base, Some(same_answer)))
}

/// The link in the email lands here; signing in is a button press (a POST), so link checkers in
/// mail systems can't spend the link.
fn signin_page(base: &str, t: &str) -> Resp {
    show(
        base,
        "Sign in",
        &format!(
            "<h1>Sign in to your Filescape account</h1><div class=\"card\"><form method=\"post\" action=\"{base}/signin\"><input type=\"hidden\" name=\"t\" value=\"{}\"><button class=\"primary\">Sign in</button></form></div>",
            esc(t)
        ),
    )
}

async fn signin<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, form: &Form) -> Result<Resp, String> {
    match cx.use_login(&form.get("t")).await? {
        Some(session) => {
            let mut r = Resp::redirect(&format!("{base}/"));
            r.headers.push(("set-cookie".into(), portal::set_cookie(&session, portal::SESSION_LIFE, !cx.cfg.dev)));
            Ok(r)
        }
        None => Ok(home(base, Some("That sign-in link has expired or was already used. Ask for a new one below."))),
    }
}

fn os_label(os: Option<&str>) -> &str {
    os.unwrap_or("Computer")
}

fn licence_card(base: &str, o: &Owned, now: filescape_licence::Ts, offer: Option<&str>) -> String {
    let l = &o.row.lic;
    let what = match l.validity {
        Validity::Perpetual { .. } => "Perpetual licence · never expires".to_string(),
        _ => match l.expires_at {
            Some(e) if e <= now => format!("Temporary licence · ended {}", date(e)),
            Some(e) => format!("Temporary licence · until {}", date(e)),
            None => "Temporary licence · not activated yet".into(),
        },
    };
    let state = if l.revoked_at.is_some() { " <span class=\"pill bad\">Revoked</span>" } else { "" };
    let maint = match l.validity {
        Validity::Perpetual { updates_until } => {
            let day = date(updates_until).split(',').next().unwrap_or_default().to_string();
            let status = match filescape_licence::maintenance::status(updates_until, now) {
                filescape_licence::maintenance::MaintenanceStatus::Active { days_left, .. } => {
                    format!("<b>Software maintenance:</b> <span class=\"good\">active</span>, {days_left} day{} left (until {day}). Every release published until then is included.", if days_left == 1 { "" } else { "s" })
                }
                filescape_licence::maintenance::MaintenanceStatus::Ended { .. } => format!(
                    "<b>Software maintenance:</b> ended {day}. Your licence is still valid: every release published until then keeps its Pro features for good. Extend maintenance to get the latest version and another year of updates."
                ),
            };
            let button = match (offer, l.revoked_at) {
                (Some(label), None) => format!(
                    "<form method=\"post\" action=\"{base}/renew\" style=\"margin-top:8px\"><input type=\"hidden\" name=\"licence\" value=\"{}\"><button class=\"primary\">Extend maintenance by 12 months · {}</button></form>",
                    esc(&l.id),
                    esc(label)
                ),
                _ => String::new(),
            };
            format!("<div class=\"card\" style=\"background:var(--bg)\">{status}{button}</div>")
        }
        _ => String::new(),
    };
    let count = if l.kind() == LicenceKind::Perpetual {
        format!("{} of {} computers active", o.seats, l.max_activations)
    } else {
        format!("{} of {} activations used (temporary licences don't get activations back)", l.activations_used, l.max_activations)
    };
    let mut rows = String::new();
    for c in &o.computers {
        let a = &c.a;
        let status = if a.is_live() {
            format!(
                "<form method=\"post\" action=\"{base}/release\" onsubmit=\"return confirm('Release this computer? Filescape on it goes back to the free features.')\"><input type=\"hidden\" name=\"activation\" value=\"{}\"><button>Release</button></form>",
                esc(&a.activation_id)
            )
        } else if let Some(e) = a.ended_at {
            format!("<span class=\"mute\">Released {}</span>", date(e))
        } else {
            "<span class=\"mute\">Removed by support</span>".into()
        };
        rows.push_str(&format!(
            "<tr><td>{}<div class=\"mute mono\" style=\"font-size:12px\">{}</div></td><td>{}</td><td>{}</td><td>{}</td><td>{status}</td></tr>",
            esc(os_label(a.os.as_deref())),
            esc(&a.device_hash[..8.min(a.device_hash.len())]),
            esc(&a.app_version),
            date(a.created_at),
            c.last_seen_at.map_or_else(|| "—".into(), date),
        ));
    }
    let table = if rows.is_empty() {
        "<p class=\"mute\">Not activated on any computer yet.</p>".to_string()
    } else {
        format!("<div class=\"scroll\"><table><tr><th>Computer</th><th>Version</th><th>Activated</th><th>Last check</th><th></th></tr>{rows}</table></div>")
    };
    format!("<div class=\"card\"><h2 style=\"margin-top:0\">Serial ending {}{state}</h2><p>{}<br>{}</p>{maint}{table}</div>", esc(&l.serial_hint), esc(&what), esc(&count))
}

async fn signed_in<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, email: &str, msg: Option<&str>) -> Result<Resp, String> {
    let m = msg.map(|m| format!("<div class=\"card\">{}</div>", esc(m))).unwrap_or_default();
    let owned = cx.owned(email).await?;
    let settings = cx.renewal_settings().await?;
    let offer = if cx.cfg.paddle.as_ref().is_some_and(|p| p.client_token.is_some()) { settings.offered().map(|p| p.label.clone()) } else { None };
    let cards: String = owned.iter().map(|o| licence_card(base, o, cx.now, offer.as_deref())).collect();
    let cards = if cards.is_empty() { "<div class=\"card\">No licences are registered to this address.</div>".into() } else { cards };
    Ok(show(
        base,
        "Your licences",
        &format!(
            "<h1>Your Filescape licences</h1><div class=\"row mute\" style=\"align-items:center\">Signed in as {}. <form method=\"post\" action=\"{base}/signout\"><button>Sign out</button></form></div>{m}{cards}\
             <p class=\"mute\">Releasing a computer frees its place for another one. The first computer you release in a week frees its place at once; after that, a released computer keeps its place for 7 days. Lost a serial number? Sign out and use <b>Email my serial numbers</b>.</p>",
            esc(email)
        ),
    ))
}

/// The renewal checkout: what the renewal gives, then Paddle's overlay checkout with the renewal
/// token as custom data. This page alone allows Paddle's scripts and frames.
async fn renew_page<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, t: &str) -> Result<Resp, String> {
    let settings = cx.renewal_settings().await?;
    let (Some(price), Some(pc)) = (settings.offered(), cx.cfg.paddle.as_ref()) else { return Ok(show(base, "Extend maintenance", "<h1>Extend maintenance</h1><div class=\"card\">Maintenance renewals aren't available yet.</div>")) };
    let Some(token_client) = pc.client_token.as_deref() else { return Ok(show(base, "Extend maintenance", "<h1>Extend maintenance</h1><div class=\"card\">Maintenance renewals aren't available yet.</div>")) };
    let Some(p) = cx.renewal_token(t, false).await? else {
        return Ok(show(
            base,
            "Extend maintenance",
            &format!("<h1>Extend maintenance</h1><div class=\"card\"><p>This renewal link has expired or isn't valid. Start again from <b>Help &gt; Licence</b> in Filescape, or <a href=\"{base}/\">sign in to your account</a>.</p></div>"),
        ));
    };
    let day = |t: filescape_licence::Ts| date(t).split(',').next().unwrap_or_default().to_string();
    let after = crate::renewal::preview(p.until, cx.now, settings.months());
    let now_line = if p.until > cx.now {
        format!("Maintenance now runs until <b>{}</b>. Renewing adds 12 months to that date.", day(p.until))
    } else {
        format!("Maintenance ended on <b>{}</b>. Renewing starts a new 12 months today: nothing is charged for the time in between, and you get the latest version.", day(p.until))
    };
    let success = if cx.cfg.dev { format!("http://localhost:8787{base}/renewed") } else { format!("https://{}/renewed", cx.cfg.hosts.account) };
    let cfg = serde_json::json!({
        "sandbox": pc.sandbox,
        "token": token_client,
        "priceId": price.price_id,
        "renewal": t,
        "email": p.email,
        "successUrl": success,
    })
    .to_string()
    .replace('<', "\\u003c");
    let body = format!(
        "<h1>Extend software maintenance</h1><div class=\"card\"><p>Licence with serial ending <b>{}</b></p><p>{now_line}</p><p>After renewing: updates included until <b>{}</b>.</p>\
         <p>Your serial number, your computers and every version your licence already covers stay as they are. This is a one-time payment, not a subscription. Renewals are non-refundable.</p>\
         <p><button class=\"primary\" id=\"pay\">Extend maintenance · {}</button></p><p class=\"mute\" id=\"err\" hidden>The checkout couldn't open. Check that nothing is blocking cdn.paddle.com, or try again later.</p>\
         <p class=\"mute\">Checkout by Paddle, our reseller, who also sends the receipt. The price above is before any sales tax, which the checkout adds.</p></div>\
         <script type=\"application/json\" id=\"cfg\">{cfg}</script>\
         <script src=\"https://cdn.paddle.com/paddle/v2/paddle.js\"></script>\
         <script>(function(){{var c=JSON.parse(document.getElementById('cfg').textContent);var err=document.getElementById('err');\
         try{{if(c.sandbox)Paddle.Environment.set('sandbox');Paddle.Initialize({{token:c.token}});}}catch(e){{err.hidden=false}}\
         document.getElementById('pay').onclick=function(){{try{{var o={{items:[{{priceId:c.priceId,quantity:1}}],customData:{{renewal:c.renewal}},settings:{{successUrl:c.successUrl,allowQuantity:false}}}};if(c.email)o.customer={{email:c.email}};Paddle.Checkout.open(o)}}catch(e){{err.hidden=false}}}};}})();</script>",
        esc(&p.serial_hint),
        day(after),
        esc(&price.label),
    );
    let mut r = show(base, "Extend maintenance", &body);
    r.headers.retain(|(k, _)| k != "content-security-policy");
    r.headers.push((
        "content-security-policy".into(),
        "default-src 'self'; script-src 'unsafe-inline' https://cdn.paddle.com; style-src 'unsafe-inline' https://cdn.paddle.com; frame-src https://buy.paddle.com https://sandbox-buy.paddle.com; connect-src 'self' https://*.paddle.com; img-src 'self' data: https://*.paddle.com; frame-ancestors 'none'".into(),
    ));
    Ok(r)
}
