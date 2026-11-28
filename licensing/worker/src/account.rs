//! account.filescape.app: for customers. Sign in with an emailed link to see your licences and
//! release computers ([`crate::portal`]), offline activation, "email me my serial numbers", and
//! the page Paddle's checkout returns to.

use crate::maintenance::GrantRow;
use crate::portal::{self, Owned};
use crate::profile::{ChangeDone, Prefs, purchase_txn};
use crate::store::Fail;
use crate::web::{Form, date, esc, page};
use crate::{Ctx, Db, Net, Req, Resp, mail};
use filescape_licence::maintenance::MaintenanceStatus;
use filescape_licence::model::{LicenceKind, Validity};
use filescape_licence::release::ReleaseManifest;

fn nav(base: &str) -> String {
    format!("<b>Filescape</b><a href=\"{base}/\">Your licence</a><a href=\"{base}/versions\">Downloads</a><a href=\"{base}/activate\">Offline activation</a><a href=\"https://filescape.app/contact\">Contact</a><a href=\"https://filescape.app/\">filescape.app</a>")
}

/// A form posted from this site (not another one). Session cookies are SameSite=Lax as well.
fn same_site<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req) -> bool {
    crate::web::same_origin_post(req, &cx.cfg.hosts.account, cx.cfg.dev)
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
        ("POST", "/login") => {
            if !cx.ip_allowed(req, "login", 10, 3600).await? {
                return Ok(home(base, Some("Too many sign-in requests from your network. Please try again in an hour.")));
            }
            login(cx, base, &Form::parse(&req.text())).await
        }
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
        ("POST", "/find") => {
            if !cx.ip_allowed(req, "find", 10, 3600).await? {
                return Ok(home(base, Some("Too many requests from your network. Please try again in an hour.")));
            }
            find(cx, base, &Form::parse(&req.text())).await
        }
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
        ("POST", "/prefs") => {
            let Some(email) = cx.session_email(req).await? else { return Ok(Resp::redirect(&format!("{base}/?msg=Sign%20in%20again"))) };
            let form = Form::parse(&req.text());
            cx.set_prefs(&email, Prefs { reminders: form.has("reminders") }).await?;
            Ok(Resp::redirect(&format!("{base}/?msg=Preferences%20saved.")))
        }
        ("POST", "/email/change") => email_change(cx, base, req).await,
        ("GET", "/email/confirm") => email_confirm_page(cx, base, &req.query_param("t").unwrap_or_default()).await,
        ("POST", "/email/confirm") => email_confirm(cx, base, req).await,
        ("GET", "/receipt") => {
            let Some(email) = cx.session_email(req).await? else { return Ok(Resp::redirect(&format!("{base}/?msg=Sign%20in%20again"))) };
            let (id, txn) = (req.query_param("licence").unwrap_or_default(), req.query_param("txn").unwrap_or_default());
            if !cx.owns_txn(&email, &id, &txn).await? {
                return Ok(Resp::redirect(&format!("{base}/?msg={}", crate::web::encode("That receipt isn't for a licence registered to you."))));
            }
            match cx.receipt_url(&txn).await? {
                Some(url) => Ok(Resp::redirect(&url)),
                None => Ok(Resp::redirect(&format!("{base}/?msg={}", crate::web::encode("That receipt isn't available right now. Paddle also emailed it to you when you paid.")))),
            }
        }
        ("GET", "/versions") => versions_page(cx, base).await,
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

fn day(t: filescape_licence::Ts) -> String {
    date(t).split(',').next().unwrap_or_default().to_string()
}

/// What the account page knows besides the licences themselves.
struct View<'a> {
    base: &'a str,
    now: filescape_licence::Ts,
    /// The renewal price label, when renewals can be bought.
    offer: Option<&'a str>,
    latest: Option<&'a ReleaseManifest>,
    catalogue: &'a [crate::catalogue::Entry],
    /// Receipts can be fetched from Paddle.
    receipts: bool,
}

fn tile(label: &str, value: &str, sub: &str, extra: &str) -> String {
    format!("<div><span>{label}</span><b>{value}</b><small>{sub}</small>{extra}</div>")
}

fn link_list(links: &[crate::catalogue::Link]) -> String {
    links.iter().map(|(l, u)| format!("<a href=\"{}\">{l}</a>", esc(u))).collect::<Vec<_>>().join(" · ")
}

fn downloads(v: &View, updates_until: Option<filescape_licence::Ts>) -> String {
    let Some(m) = v.latest else {
        return format!("<p class=\"mute\">Download Filescape from <a href=\"https://filescape.app/#download\">filescape.app</a>, or see <a href=\"{}/versions\">every version</a>.</p>", v.base);
    };
    let links = [("macOS", "macos"), ("Windows", "windows"), ("Linux", "linux")].iter().map(|(name, os)| format!("<a href=\"{}\">{name}</a>", esc(&m.download_for(os)))).collect::<Vec<_>>().join(" · ");
    let notes = m.notes().map(|u| format!(" · <a href=\"{}\">What's new</a>", esc(u))).unwrap_or_default();
    let head = format!("<b>Filescape {}</b> <span class=\"mute\">released {}</span>", esc(&m.version), day(m.released_at));
    let latest = match updates_until {
        Some(u) if m.released_at > u => {
            let unlocked = match crate::catalogue::newest_unlocked(v.catalogue, u) {
                Some(e) => format!(
                    "<p>The newest version your licence unlocks: <b>Filescape {}</b> <span class=\"mute\">released {}</span> <span class=\"pill good\">Included</span><br>Download: {}</p>",
                    esc(&e.m.version),
                    day(e.m.released_at),
                    link_list(&e.links())
                ),
                None => String::new(),
            };
            format!(
                "<p>{head} <span class=\"pill\">Not included</span><br>It came out after your maintenance ended, so it runs with the free features until you extend maintenance. Versions released up to {} keep Pro for good.<br>Download: {links}{notes}</p>{unlocked}",
                day(u)
            )
        }
        Some(_) => format!("<p>{head} <span class=\"pill good\">Included</span><br>Download: {links}{notes}</p>"),
        None => format!("<p>{head}<br>Download: {links}{notes}</p>"),
    };
    let older: Vec<_> = v.catalogue.iter().filter(|e| e.m.version != m.version).collect();
    if older.is_empty() {
        return latest;
    }
    let rows: String = older
        .iter()
        .map(|e| {
            let badge = match updates_until {
                Some(u) if e.m.released_at > u => " <span class=\"pill\">Not included</span>",
                Some(_) => " <span class=\"pill good\">Included</span>",
                None => "",
            };
            format!("<tr><td><b>{}</b>{badge}</td><td>{}</td><td style=\"font-size:13px\">{}</td></tr>", esc(&e.m.version), day(e.m.released_at), link_list(&e.links()))
        })
        .collect();
    format!("{latest}<details><summary>Earlier versions</summary><div class=\"scroll\"><table><tr><th>Version</th><th>Released</th><th>Download</th></tr>{rows}</table></div></details>")
}

/// account.filescape.app/versions: every listed version, for anyone.
async fn versions_page<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str) -> Result<Resp, String> {
    let rows: String = cx
        .catalogue(false)
        .await?
        .iter()
        .map(|e| {
            let notes = e.m.notes().map(|u| format!(" · <a href=\"{}\">What's new</a>", esc(u))).unwrap_or_default();
            format!("<tr><td><b>{}</b></td><td>{}</td><td style=\"font-size:13px\">{}{notes}</td></tr>", esc(&e.m.version), day(e.m.released_at), link_list(&e.links()))
        })
        .collect();
    let table = if rows.is_empty() {
        "<p>Get the current version from <a href=\"https://filescape.app/#download\">filescape.app</a>.</p>".to_string()
    } else {
        format!("<div class=\"scroll\"><table><tr><th>Version</th><th>Released</th><th>Download</th></tr>{rows}</table></div>")
    };
    Ok(show(
        base,
        "Every version",
        &format!(
            "<h1>Every Filescape version</h1><div class=\"card\"><p>A perpetual licence unlocks Filescape Professional in every version released before its software maintenance ends, for good. If yours has ended, install the newest version released before that date. <a href=\"{base}/\">Sign in</a> to see which versions your licence unlocks.</p>{table}</div>"
        ),
    ))
}

fn history(v: &View, l: &filescape_licence::model::Licence, grants: &[GrantRow]) -> String {
    use filescape_licence::maintenance::GrantKind;
    let receipt = |txn: Option<&str>| match txn {
        Some(t) if v.receipts => format!("<a href=\"{}/receipt?licence={}&amp;txn={}\">Receipt</a>", v.base, crate::web::encode(&l.id), crate::web::encode(t)),
        Some(_) => "<span class=\"mute\">Emailed by Paddle</span>".into(),
        None => String::new(),
    };
    let mut rows = String::new();
    for g in grants {
        let (what, txn) = match g.grant.kind {
            GrantKind::Purchase => ("Purchase · perpetual licence with 12 months of maintenance".to_string(), purchase_txn(l.order_ref.as_deref())),
            GrantKind::Renewal => (format!("Maintenance renewal · 12 months{}", match (&g.amount, &g.currency) {
                (Some(a), Some(c)) => format!(" · {} {}", esc(a), esc(c)),
                _ => String::new(),
            }), g.txn.as_deref()),
            GrantKind::Complimentary => (format!("Maintenance extended by us · {}", crate::maintenance::length(g.grant.months, g.grant.days)), None),
        };
        let state = if g.reversed_at.is_some() { " <span class=\"pill bad\">Reversed</span>" } else { "" };
        let after = g.after_until.map(day).unwrap_or_default();
        rows.push_str(&format!("<tr><td>{}</td><td>{what}{state}</td><td>{after}</td><td>{}</td></tr>", day(g.grant.at), receipt(txn)));
    }
    if rows.is_empty() {
        rows = match purchase_txn(l.order_ref.as_deref()) {
            Some(t) => format!("<tr><td>{}</td><td>Purchase</td><td></td><td>{}</td></tr>", day(l.issued_at), receipt(Some(t))),
            None => format!("<tr><td>{}</td><td>Licence issued</td><td></td><td></td></tr>", day(l.issued_at)),
        };
    }
    format!("<div class=\"scroll\"><table><tr><th>Date</th><th>What</th><th>Maintenance until</th><th></th></tr>{rows}</table></div>")
}

fn licence_card(v: &View, o: &Owned, grants: &[GrantRow]) -> String {
    let (base, now) = (v.base, v.now);
    let l = &o.row.lic;
    let revoked = l.revoked_at.is_some();
    let state = if revoked { " <span class=\"pill bad\">Revoked</span>" } else { "" };
    let licence_tile = match l.validity {
        _ if revoked => tile("Licence", "<span class=\"bad\">Revoked</span>", "Contact us if you think this is a mistake", ""),
        Validity::Perpetual { .. } => tile("Licence", "<span class=\"good\">Perpetual</span>", "Never expires", ""),
        _ => match l.expires_at {
            Some(e) if e <= now => tile("Licence", "Temporary", &format!("Ended {}", day(e)), ""),
            Some(e) => tile("Licence", "Temporary", &format!("Until {}", day(e)), ""),
            None => tile("Licence", "Temporary", "Starts when first activated", ""),
        },
    };
    let mut maint_note = String::new();
    let maint_tile = match l.validity {
        Validity::Perpetual { updates_until } => {
            let button = match (v.offer, revoked) {
                (Some(label), false) => format!(
                    "<form method=\"post\" action=\"{base}/renew\" style=\"margin-top:8px\"><input type=\"hidden\" name=\"licence\" value=\"{}\"><button class=\"primary\">Extend maintenance by 12 months · {}</button></form>",
                    esc(&l.id),
                    esc(label)
                ),
                _ => String::new(),
            };
            match filescape_licence::maintenance::status(updates_until, now) {
                MaintenanceStatus::Active { days_left, .. } => {
                    let pct = (days_left * 100 / 365).clamp(2, 100);
                    maint_note = format!("<p>Every Filescape release published until {} is included. Renewing early loses nothing: it adds 12 months to that date.</p>{button}", day(updates_until));
                    let tone = if days_left <= 30 { "warn" } else { "good" };
                    tile(
                        "Software maintenance",
                        &format!("<span class=\"{tone}\">Active</span>"),
                        &format!("Until {} · {days_left} day{} left", day(updates_until), if days_left == 1 { "" } else { "s" }),
                        &format!("<div class=\"bar\"><i style=\"width:{pct}%\"></i></div>"),
                    )
                }
                MaintenanceStatus::Ended { .. } => {
                    maint_note = format!(
                        "<p>Your licence is still valid: every release published until {} keeps its Pro features for good. Extend maintenance to get the latest version and another 12 months of updates, counted from the day you renew.</p>{button}",
                        day(updates_until)
                    );
                    tile("Software maintenance", "Ended", &format!("On {}", day(updates_until)), "")
                }
            }
        }
        _ => tile("Software maintenance", "Included", "While the licence runs", ""),
    };
    let computers_tile = if l.kind() == LicenceKind::Perpetual {
        tile("Computers", &format!("{} of {}", o.seats, l.max_activations), "Active now", "")
    } else {
        tile("Activations", &format!("{} of {}", l.activations_used, l.max_activations), "Temporary licences don't get activations back", "")
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
    let until = match l.validity {
        Validity::Perpetual { updates_until } => Some(updates_until),
        _ => None,
    };
    format!(
        "<div class=\"card\"><h2 style=\"margin-top:0\">Filescape Pro · serial ending <span class=\"mono\">{}</span>{state}</h2>\
         <div class=\"tiles\">{licence_tile}{maint_tile}{computers_tile}</div>{maint_note}\
         <h3>Download</h3>{}<h3>Computers</h3>{table}<h3>Purchases and renewals</h3>{}</div>",
        esc(&l.serial_hint),
        downloads(v, until),
        history(v, l, grants),
    )
}

async fn signed_in<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, email: &str, msg: Option<&str>) -> Result<Resp, String> {
    let m = msg.map(|m| format!("<div class=\"card\">{}</div>", esc(m))).unwrap_or_default();
    let owned = cx.owned(email).await?;
    let settings = cx.renewal_settings().await?;
    let offer = if cx.cfg.paddle.as_ref().is_some_and(|p| p.client_token.is_some()) { settings.offered().map(|p| p.label.clone()) } else { None };
    let latest = cx.latest_release().await?;
    let catalogue = cx.catalogue(false).await?;
    let view = View { base, now: cx.now, offer: offer.as_deref(), latest: latest.as_ref(), catalogue: &catalogue, receipts: cx.cfg.paddle.as_ref().is_some_and(|p| p.api_key.is_some()) };
    let mut cards = String::new();
    for o in &owned {
        let grants = if o.row.lic.kind() == LicenceKind::Perpetual { cx.grants_of(&o.row.lic.id).await? } else { Vec::new() };
        cards.push_str(&licence_card(&view, o, &grants));
    }
    let cards = if cards.is_empty() { "<div class=\"card\">No licences are registered to this address.</div>".into() } else { cards };
    let prefs = cx.prefs(email).await?;
    Ok(show(
        base,
        "Your licences",
        &format!(
            "<h1>Your Filescape licences</h1><div class=\"row mute\" style=\"align-items:center\">Signed in as {}. <form method=\"post\" action=\"{base}/signout\"><button>Sign out</button></form></div>{m}{cards}\
             <p class=\"mute\">Releasing a computer frees its place for another one. The first computer you release in a week frees its place at once; after that, a released computer keeps its place for 7 days. Lost a serial number? Sign out and use <b>Email my serial numbers</b>.</p>\
             <div class=\"card\"><h2 style=\"margin-top:0\">Email preferences</h2><form method=\"post\" action=\"{base}/prefs\">\
             <p><label style=\"flex-direction:row;align-items:center;gap:8px;font-size:15px;color:var(--fg)\"><input type=\"checkbox\" name=\"reminders\" value=\"1\"{}> Remind me before software maintenance ends (30 days and 7 days before, and on the day)</label></p>\
             <p class=\"mute\">Emails about your purchases, renewals and sign-in links are always sent. We never send marketing email.</p><button>Save preferences</button></form></div>\
             <div class=\"card\"><h2 style=\"margin-top:0\">Email address</h2><p>Your licences are registered to <b>{}</b>. To move them to another address, enter it below. We'll email that address a link to confirm, and this address gets a link to undo the change for 14 days.</p>\
             <form method=\"post\" action=\"{base}/email/change\" class=\"row\"><label>New email<input name=\"email\" type=\"email\" required size=\"32\" autocomplete=\"email\"></label><button>Change email address</button></form></div>",
            esc(email),
            if prefs.reminders { " checked" } else { "" },
            esc(email),
        ),
    ))
}

async fn email_change<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, req: &Req) -> Result<Resp, String> {
    let back = |m: &str| Ok(Resp::redirect(&format!("{base}/?msg={}", crate::web::encode(m))));
    let Some(email) = cx.session_email(req).await? else { return back("Sign in again.") };
    let new = Form::parse(&req.text()).get("email").to_lowercase();
    if !crate::profile::valid_email(&new) {
        return back(&crate::profile::ChangeRefusal::Invalid.message());
    }
    if !cx.ip_allowed(req, "email-change", 5, 3600).await? || !cx.mail_allowed(&email, "email-change", 120).await? {
        return back("Please wait a few minutes before asking again.");
    }
    match cx.request_email_change(&email, &new).await? {
        Err(why) => back(&why.message()),
        Ok(t) => {
            let link = account_link(cx, base, &format!("/email/confirm?t={t}"));
            if let Err(e) = mail::send(cx, &mail::email_confirm_message(&link, &email, &new)).await {
                eprintln!("email change: {e}");
                return back("The confirmation email couldn't be sent. Please try again later.");
            }
            back(&format!("Almost done: open the link we sent to {new} to confirm the change. It works for 24 hours. Until then, your licences stay on {email}."))
        }
    }
}

fn account_link<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, path: &str) -> String {
    if cx.cfg.dev { format!("http://localhost:8787{base}{path}") } else { format!("https://{}{path}", cx.cfg.hosts.account) }
}

/// The confirm or undo link lands here; the change happens on a button press (a POST), so mail
/// scanners that fetch links don't spend it.
async fn email_confirm_page<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, t: &str) -> Result<Resp, String> {
    let Some((kind, old, new)) = cx.email_change(t).await? else {
        return Ok(show(base, "Email address", "<h1>Email address</h1><div class=\"card\">This link has expired or was already used.</div>"));
    };
    let (text, button) = if kind == "undo" {
        (format!("Move your Filescape licences back from <b>{}</b> to <b>{}</b>? Anyone signed in as {} is signed out.", esc(&new), esc(&old), esc(&new)), "Undo the change")
    } else {
        (format!("Register the Filescape licences of <b>{}</b> to <b>{}</b>? Serial numbers and activated computers stay as they are.", esc(&old), esc(&new)), "Confirm")
    };
    Ok(show(
        base,
        "Email address",
        &format!(
            "<h1>Email address</h1><div class=\"card\"><p>{text}</p><form method=\"post\" action=\"{base}/email/confirm\"><input type=\"hidden\" name=\"t\" value=\"{}\"><button class=\"primary\">{button}</button></form></div>",
            esc(t)
        ),
    ))
}

async fn email_confirm<D: Db, N: Net>(cx: &Ctx<'_, D, N>, base: &str, req: &Req) -> Result<Resp, String> {
    let t = Form::parse(&req.text()).get("t");
    let done = |body: String| Ok(show(base, "Email address", &format!("<h1>Email address</h1><div class=\"card\">{body}</div>")));
    match cx.use_email_change(&t).await? {
        ChangeDone::Expired => done("This link has expired or was already used.".into()),
        ChangeDone::Refused(why) => done(esc(&why.message())),
        ChangeDone::Undone { old, count, .. } => done(format!(
            "<p>Done: {} registered to <b>{}</b> again.</p><p>If you didn't make the original change, someone may have had access to your email. Change your email password, then <a href=\"{base}/\">sign in</a> to check your computers and release any you don't recognise.</p>",
            if count == 1 { "your licence is".to_string() } else { format!("your {count} licences are") },
            esc(&old)
        )),
        ChangeDone::Moved { old, new, count, undo } => {
            let link = account_link(cx, base, &format!("/email/confirm?t={undo}"));
            if let Err(e) = mail::send(cx, &mail::email_changed_message(&link, &new, count, &old)).await {
                eprintln!("email changed: {e}");
            }
            // Proving the new address is as good as a sign-in link.
            let session = cx.new_session(&new).await?;
            let mut r = Resp::redirect(&format!("{base}/?msg={}", crate::web::encode(&format!("Your licences are now registered to {new}. We've emailed {old} a link to undo this for 14 days."))));
            r.headers.push(("set-cookie".into(), portal::set_cookie(&session, portal::SESSION_LIFE, !cx.cfg.dev)));
            Ok(r)
        }
    }
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
