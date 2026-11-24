//! admin.filescape.app: licences, Create Temporary Licence, revoke, reset activations, limits,
//! the Activations page and settings. Signed in through Cloudflare Access ([`crate::access`]);
//! every change is recorded in the licence's history with the admin's email.

use crate::db::LicenceRow;
use crate::store::{ListFilter, describe};
use crate::web::{Form, date, encode, esc, iso_day, opt_date, page, parse_day};
use crate::{Ctx, Db, Net, Req, Resp, mail};
use filescape_licence::model::{LicenceKind, Status, Validity};
use filescape_licence::release::{self, Downloads, ReleaseManifest};
use filescape_licence::rules::{IssueError, TemporaryOptions};
use filescape_licence::{DEFAULT_MAX_ACTIVATIONS, DEFAULT_TEMPORARY_DAYS, SHARING_FLAG_MACHINES, SHARING_WINDOW_DAYS};

struct A<'a> {
    base: &'a str,
    actor: &'a str,
}

impl A<'_> {
    fn nav(&self) -> String {
        let b = self.base;
        format!(
            "<b>Filescape admin</b><a href=\"{b}/\">Licences</a><a href=\"{b}/issue\">Create temporary licence</a><a href=\"{b}/activations\">Activations</a><a href=\"{b}/releases\">Releases</a><a href=\"{b}/settings\">Settings</a><span class=\"mute\" style=\"margin-left:auto\">{}</span>",
            esc(self.actor)
        )
    }

    fn page(&self, title: &str, body: &str) -> Resp {
        Resp::html(200, page(&format!("{title} · Filescape admin"), &self.nav(), body))
    }
}

pub async fn route<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req, path: &str, base: &str, actor: &str) -> Result<Resp, String> {
    let a = A { base, actor };
    let segs: Vec<&str> = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    let form = Form::parse(&req.text());
    match (req.method.as_str(), segs.as_slice()) {
        ("GET", []) => home(cx, &a, req).await,
        ("GET", ["licence", id]) => licence(cx, &a, id, req.query_param("msg").as_deref()).await,
        ("POST", ["licence", id, action]) => licence_action(cx, &a, id, action, &form).await,
        ("GET", ["issue"]) => Ok(issue_form(&a, None)),
        ("POST", ["issue"]) => issue(cx, &a, &form).await,
        ("GET", ["activations"]) => activations(cx, &a).await,
        ("GET", ["releases"]) => releases(cx, &a, req.query_param("msg").as_deref()).await,
        ("POST", ["releases"]) => {
            let msg = if form.has("withdraw") {
                cx.set_latest_release(None, actor).await?;
                "Filescape no longer announces a new release.".to_string()
            } else {
                match release_from_form(&form, cx.now) {
                    Ok(m) => {
                        cx.set_latest_release(Some(&m), actor).await?;
                        format!("Filescape {} is now announced. Copies of Filescape see it the next time they start (within about 5 minutes).", m.version)
                    }
                    Err(e) => e,
                }
            };
            Ok(Resp::redirect(&format!("{base}/releases?msg={}", encode(&msg))))
        }
        ("GET", ["settings"]) => settings(cx, &a, req.query_param("msg").as_deref()).await,
        ("POST", ["settings"]) => {
            let max: u32 = form.get("perpetual_limit").parse().unwrap_or(0);
            let msg = match cx.set_default_perpetual_limit(max, actor).await? {
                Ok(()) => format!("New purchases now get {max} activations."),
                Err(_) => "The limit must be between 1 and 1000.".into(),
            };
            Ok(Resp::redirect(&format!("{base}/settings?msg={}", encode(&msg))))
        }
        _ => Ok(Resp::html(404, page("Not found", &a.nav(), "<h1>Not found</h1>"))),
    }
}

fn status_pill(row: &LicenceRow, now: i64) -> String {
    let s = row.lic.status(now);
    let class = match s {
        Status::Active => "good",
        Status::Issued => "mute",
        Status::Expired | Status::ActivationLimitReached => "warn",
        Status::Revoked => "bad",
    };
    let mut out = format!("<span class=\"pill {class}\">{}</span>", s.label());
    if let Some(f) = &row.refund_flag {
        out.push_str(&format!(" <span class=\"pill bad\">{}</span>", esc(f)));
    }
    out
}

fn kind_label(row: &LicenceRow) -> &'static str {
    match row.lic.kind() {
        LicenceKind::Perpetual => "Perpetual",
        LicenceKind::Temporary => "Temporary",
    }
}

fn ends(row: &LicenceRow) -> String {
    match row.lic.validity {
        Validity::Perpetual { updates_until } => format!("updates until {}", date(updates_until).split(',').next().unwrap_or_default()),
        Validity::FirstActivation { days } if row.lic.expires_at.is_none() => format!("{days} days from first activation"),
        _ => opt_date(row.lic.expires_at),
    }
}

async fn home<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, req: &Req) -> Result<Resp, String> {
    let q = req.query_param("q").unwrap_or_default();
    let f = req.query_param("f").unwrap_or_default();
    let filter = match f.as_str() {
        "temporary" => ListFilter::Temporary,
        "perpetual" => ListFilter::Perpetual,
        "expiring" => ListFilter::ExpiringSoon,
        "revoked" => ListFilter::Revoked,
        "refunds" => ListFilter::Refunds,
        "unmailed" => ListFilter::NotEmailed,
        _ => ListFilter::All,
    };
    let [total, perpetual, temp_live, live_acts] = cx.counts().await?;
    let rows = cx.list(&q, filter, 200).await?;
    let opts = [("", "All"), ("perpetual", "Perpetual"), ("temporary", "Temporary"), ("expiring", "Ending in 7 days"), ("revoked", "Revoked"), ("refunds", "Refunds and chargebacks"), ("unmailed", "Paid, not emailed")]
        .iter()
        .map(|(v, l)| format!("<option value=\"{v}\"{}>{l}</option>", if *v == f { " selected" } else { "" }))
        .collect::<String>();
    let mut body = format!(
        "<h1>Licences</h1><div class=\"stats\"><div><b>{total}</b>licences</div><div><b>{perpetual}</b>perpetual</div><div><b>{temp_live}</b>temporary, not expired</div><div><b>{live_acts}</b>computers activated</div></div>\
         <form class=\"card row\" method=\"get\" action=\"{b}/\"><label>Search<input name=\"q\" value=\"{}\" placeholder=\"Serial, licence ID, name, email or Paddle transaction\" size=\"44\"></label>\
         <label>Show<select name=\"f\">{opts}</select></label><button class=\"primary\">Search</button></form>",
        esc(&q),
        b = a.base
    );
    if rows.is_empty() {
        body.push_str("<p class=\"mute\">No licences match.</p>");
    } else {
        body.push_str("<div class=\"scroll\"><table><tr><th>Licence</th><th>Type</th><th>Owner</th><th>Status</th><th>Activations</th><th>Ends</th><th>Issued</th></tr>");
        for r in &rows {
            body.push_str(&format!(
                "<tr><td><a class=\"mono\" href=\"{b}/licence/{id}\">{id}</a><br><span class=\"mute mono\">{hint}</span></td><td>{}</td><td>{}<br><span class=\"mute\">{}</span></td><td>{}</td><td>{} of {}</td><td>{}</td><td>{}</td></tr>",
                kind_label(r),
                esc(r.lic.owner.as_deref().unwrap_or("")),
                esc(r.lic.email.as_deref().unwrap_or("")),
                status_pill(r, cx.now),
                r.lic.activations_used,
                r.lic.max_activations,
                ends(r),
                date(r.lic.issued_at).split(',').next().unwrap_or_default(),
                b = a.base,
                id = esc(&r.lic.id),
                hint = esc(&r.lic.serial_hint),
            ));
        }
        body.push_str("</table></div>");
        if rows.len() == 200 {
            body.push_str("<p class=\"mute\">Showing the newest 200. Search to narrow it down.</p>");
        }
    }
    Ok(a.page("Licences", &body))
}

async fn licence<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, id: &str, msg: Option<&str>) -> Result<Resp, String> {
    let Some(r) = cx.licence(id).await? else { return Ok(Resp::html(404, page("Not found", &a.nav(), "<h1>No such licence</h1>"))) };
    let acts = cx.activations_of(id).await?;
    let events = cx.events_of(id).await?;
    let b = a.base;
    let l = &r.lic;
    let mut body = format!("<p><a href=\"{b}/\">← Licences</a></p><h1 class=\"mono\">{}</h1>", esc(&l.id));
    if let Some(m) = msg {
        body.push_str(&format!("<div class=\"card\">{}</div>", esc(m)));
    }
    let rows = [
        ("Status", status_pill(&r, cx.now)),
        ("Type", format!("{}{}", kind_label(&r), if l.kind() == LicenceKind::Temporary { " (non-renewable)" } else { "" })),
        ("Validity", esc(&describe(&l.validity, l.max_activations))),
        ("Serial", format!("<span class=\"mono\">{}</span>{}", esc(&l.serial_hint), if r.serial_enc.is_some() { " <span class=\"mute\">(kept encrypted; can be emailed again)</span>" } else { "" })),
        ("Owner", esc(l.owner.as_deref().unwrap_or("—"))),
        ("Email", esc(l.email.as_deref().unwrap_or("—"))),
        ("Purpose", esc(l.purpose.as_deref().unwrap_or("—"))),
        ("Issued", date(l.issued_at)),
        ("First activated", opt_date(l.first_activated_at)),
        ("Expires", opt_date(l.expires_at)),
        (
            if l.kind() == LicenceKind::Perpetual { "Computers active" } else { "Activations used" },
            if l.kind() == LicenceKind::Perpetual {
                let all: Vec<_> = acts.iter().map(|x| x.a.clone()).collect();
                format!("{} of {} <span class=\"mute\">({} activations ever)</span>", filescape_licence::rules::seats_in_use(l, &all, cx.now), l.max_activations, l.activations_used)
            } else {
                format!("{} of {}", l.activations_used, l.max_activations)
            },
        ),
        ("Paddle transaction", esc(l.order_ref.as_deref().unwrap_or("—"))),
        ("Emailed", opt_date(r.emailed_at)),
        ("Revoked", l.revoked_at.map_or("—".into(), |t| format!("{} ({})", date(t), esc(l.revoke_reason.as_deref().unwrap_or(""))))),
    ];
    body.push_str("<div class=\"card\"><table>");
    for (k, v) in rows {
        body.push_str(&format!("<tr><th style=\"width:180px\">{k}</th><td>{v}</td></tr>"));
    }
    body.push_str("</table></div><h2>Actions</h2><div class=\"card\">");
    let act = |name: &str, inner: &str, button: &str, class: &str| {
        format!("<form method=\"post\" action=\"{b}/licence/{}/{name}\" class=\"row\" style=\"margin:8px 0\">{inner}<button class=\"{class}\">{button}</button></form>", esc(&l.id))
    };
    if l.revoked_at.is_none() {
        body.push_str(&act("revoke", "<label>Reason<input name=\"reason\" required size=\"40\" placeholder=\"e.g. refunded, shared publicly\"></label>", "Revoke licence", "danger"));
    } else {
        body.push_str(&act("reinstate", "<label>Reason<input name=\"reason\" required size=\"40\"></label>", "Reinstate licence", ""));
    }
    body.push_str(&act(
        "reset",
        "<label>Reason<input name=\"reason\" required size=\"40\" placeholder=\"e.g. new computer, support request\"></label>",
        "Reset activations",
        "",
    ));
    body.push_str(if l.kind() == LicenceKind::Perpetual {
        "<p class=\"mute\" style=\"margin:-4px 0 10px\">Clears the 7-day release cooldown, so recently released computers stop holding seats. Computers still active keep their seats. To free one now, use Void on it below.</p>"
    } else {
        "<p class=\"mute\" style=\"margin:-4px 0 10px\">Gives every used activation back. Computers already activated keep working; the expiry doesn't change.</p>"
    });
    if l.kind() == LicenceKind::Perpetual {
        body.push_str(&act(
            "limit",
            &format!("<label>Computers at once<input name=\"max\" type=\"number\" min=\"1\" max=\"1000\" value=\"{}\" style=\"width:90px\"></label><label>Reason<input name=\"reason\" required size=\"30\"></label>", l.max_activations),
            "Set limit",
            "",
        ));
    }
    body.push_str(&act(
        "owner",
        &format!(
            "<label>Owner<input name=\"owner\" value=\"{}\"></label><label>Email<input name=\"email\" type=\"email\" value=\"{}\" size=\"28\"></label>",
            esc(l.owner.as_deref().unwrap_or("")),
            esc(l.email.as_deref().unwrap_or(""))
        ),
        "Save details",
        "",
    ));
    if r.serial_enc.is_some() && l.email.is_some() {
        body.push_str(&act("email", "", "Email the serial to the owner", ""));
    }
    if r.serial_enc.is_some() {
        body.push_str(&act("reveal", "<label>Reason<input name=\"reason\" required size=\"30\" placeholder=\"e.g. customer support\"></label>", "Show serial", ""));
    }
    if r.refund_flag.is_some() {
        body.push_str(&act("clear-refund", "", "Clear refund flag", ""));
    }
    body.push_str("</div><h2>Activations</h2>");
    if acts.is_empty() {
        body.push_str("<p class=\"mute\">Not activated yet.</p>");
    } else {
        body.push_str("<div class=\"scroll\"><table><tr><th>Activation</th><th>Computer</th><th>App</th><th>Activated</th><th>Last check</th><th>State</th><th></th></tr>");
        for x in &acts {
            let state = if let Some((t, why)) = &x.a.voided {
                format!("<span class=\"bad\">voided {}</span><br><span class=\"mute\">{}</span>", date(*t), esc(why))
            } else if let Some(t) = x.a.ended_at {
                format!("deactivated {}", date(t))
            } else {
                "<span class=\"good\">active</span>".into()
            };
            let counted = if x.a.counted { "" } else { "<br><span class=\"mute\">use given back</span>" };
            let void = if x.a.voided.is_none() {
                format!(
                    "<form method=\"post\" action=\"{b}/licence/{}/void\" class=\"row\"><input type=\"hidden\" name=\"activation\" value=\"{}\"><input name=\"reason\" required placeholder=\"Reason\" size=\"14\"><label style=\"flex-direction:row;align-items:center\"><input type=\"checkbox\" name=\"give_back\" value=\"1\">give use back</label><button class=\"danger\">Void</button></form>",
                    esc(&l.id),
                    esc(&x.a.activation_id)
                )
            } else {
                String::new()
            };
            body.push_str(&format!(
                "<tr><td class=\"mono\">{}</td><td class=\"mono\" title=\"{}\">{}…</td><td>{}</td><td>{}</td><td>{}</td><td>{state}{counted}</td><td>{void}</td></tr>",
                esc(&x.a.activation_id),
                esc(&x.a.device_hash),
                esc(&x.a.device_hash[..x.a.device_hash.len().min(10)]),
                esc(&x.a.app_version),
                date(x.a.created_at),
                opt_date(x.last_seen_at),
            ));
        }
        body.push_str("</table></div>");
    }
    body.push_str("<h2>History</h2><div class=\"scroll\"><table><tr><th>When</th><th>What</th><th>Details</th><th>By</th></tr>");
    for e in &events {
        body.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"mute\">{}</td></tr>", date(e.at), esc(&e.what), esc(&e.detail), esc(e.actor.as_deref().unwrap_or(""))));
    }
    body.push_str("</table></div>");
    Ok(a.page(&l.id, &body))
}

async fn licence_action<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, id: &str, action: &str, form: &Form) -> Result<Resp, String> {
    let Some(row) = cx.licence(id).await? else { return Ok(Resp::text(404, "No such licence.")) };
    let reason = form.get("reason");
    let needs_reason = matches!(action, "revoke" | "reinstate" | "reset" | "limit" | "void" | "reveal");
    let msg: String = if needs_reason && reason.is_empty() {
        "Give a reason: it goes in the licence's history.".into()
    } else {
        match action {
            "revoke" => {
                cx.revoke(id, &reason, a.actor).await?;
                "Revoked. Activated computers lose access at their next check (within a week).".into()
            }
            "reinstate" => {
                cx.unrevoke(id, &reason, a.actor).await?;
                "Reinstated.".into()
            }
            "reset" => {
                cx.reset_activations(id, &reason, a.actor).await?;
                "Activations reset.".into()
            }
            "limit" => match cx.set_limit(id, form.get("max").parse().unwrap_or(0), &reason, a.actor).await? {
                Ok(()) => "Limit changed.".into(),
                Err(IssueError::NotPerpetual) => "Only perpetual licences have an adjustable limit.".into(),
                Err(_) => "The limit must be between 1 and 1000.".into(),
            },
            "owner" => {
                cx.set_owner(id, &form.get("owner"), &form.get("email"), a.actor).await?;
                "Details saved.".into()
            }
            "void" => {
                cx.void_activation(id, &form.get("activation"), form.has("give_back"), &reason, a.actor).await?;
                "Activation voided.".into()
            }
            "clear-refund" => {
                cx.clear_refund_flag(id, a.actor).await?;
                "Refund flag cleared.".into()
            }
            "email" => match (cx.serial_of(&row), &row.lic.email) {
                (Some(serial), Some(to)) => match mail::send(cx, &mail::licence_message(&row, &serial, to)).await {
                    Ok(true) => {
                        cx.mark_emailed(id, to, a.actor).await?;
                        format!("Emailed to {to}.")
                    }
                    Ok(false) => "Email isn't set up (RESEND_API_KEY, MAIL_FROM).".into(),
                    Err(e) => format!("The email failed: {e}"),
                },
                _ => "No serial or email on this licence.".into(),
            },
            "reveal" => {
                let serial = cx.serial_of(&row);
                cx.db.run("INSERT INTO events (licence_id, at, what, detail, actor) VALUES (?1, ?2, 'serial shown', ?3, ?4)", vec![id.into(), cx.now.into(), (&reason).into(), a.actor.into()]).await?;
                let body = format!(
                    "<p><a href=\"{b}/licence/{i}\">← {i}</a></p><h1>Serial</h1><div class=\"card\"><p class=\"mono\" style=\"font-size:20px\">{}</p><p class=\"mute\">Shown to {} and recorded in the history.</p></div>",
                    esc(serial.as_deref().unwrap_or("Can't be decrypted (was SERIAL_KEY changed?)")),
                    esc(a.actor),
                    b = a.base,
                    i = esc(id)
                );
                return Ok(a.page("Serial", &body));
            }
            _ => return Ok(Resp::text(404, "Unknown action.")),
        }
    };
    Ok(Resp::redirect(&format!("{}/licence/{}?msg={}", a.base, encode(id), encode(&msg))))
}

fn issue_form(a: &A<'_>, error: Option<&str>) -> Resp {
    let err = error.map(|e| format!("<div class=\"card bad\">{}</div>", esc(e))).unwrap_or_default();
    let body = format!(
        "<h1>Create temporary licence</h1>{err}<form method=\"post\" action=\"{}/issue\" class=\"card\">\
         <p><label style=\"flex-direction:row;gap:8px;align-items:center;color:inherit\"><input type=\"radio\" name=\"mode\" value=\"days\" checked> Runs for <input name=\"days\" type=\"number\" min=\"1\" max=\"3650\" value=\"{DEFAULT_TEMPORARY_DAYS}\" style=\"width:80px\"> days from the first activation</label></p>\
         <p><label style=\"flex-direction:row;gap:8px;align-items:center;color:inherit\"><input type=\"radio\" name=\"mode\" value=\"fixed\"> Ends on <input name=\"until\" type=\"date\"> (midnight UTC), whenever it's activated</label></p>\
         <div class=\"row\"><label>Activation limit<input name=\"max\" type=\"number\" min=\"1\" max=\"100\" value=\"{DEFAULT_MAX_ACTIVATIONS}\" style=\"width:90px\"></label>\
         <label>How many serials<input name=\"count\" type=\"number\" min=\"1\" max=\"100\" value=\"1\" style=\"width:90px\"></label></div>\
         <div class=\"row\" style=\"margin-top:10px\"><label>Registered owner<input name=\"owner\" size=\"28\"></label><label>Email (optional: the serial is sent there)<input name=\"email\" type=\"email\" size=\"30\"></label></div>\
         <div class=\"row\" style=\"margin-top:10px\"><label>Purpose (internal note)<input name=\"purpose\" size=\"60\" placeholder=\"e.g. review copy, trade show, customer trial\"></label></div>\
         <p class=\"mute\">Temporary licences can't be renewed. Each computer it's activated on uses one activation, and deactivating doesn't give it back.</p>\
         <button class=\"primary\">Create</button></form>",
        a.base
    );
    a.page("Create temporary licence", &body)
}

async fn issue<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, form: &Form) -> Result<Resp, String> {
    let validity = if form.get("mode") == "fixed" {
        match parse_day(&form.get("until")) {
            Some(t) => Validity::FixedExpiry { expires_at: t },
            None => return Ok(issue_form(a, Some("Choose the end date."))),
        }
    } else {
        Validity::FirstActivation { days: form.get("days").parse().unwrap_or(0) }
    };
    let s = |k: &str| Some(form.get(k)).filter(|v| !v.is_empty());
    let opts = TemporaryOptions {
        validity,
        max_activations: form.get("max").parse().unwrap_or(0).min(100),
        count: form.get("count").parse().unwrap_or(1).clamp(1, 100),
        owner: s("owner"),
        email: s("email"),
        purpose: s("purpose"),
    };
    let made = match cx.issue_temporary(&opts, a.actor).await? {
        Ok(v) => v,
        Err(IssueError::BadValidity) => return Ok(issue_form(a, Some("The length must be at least a day, and an end date must be in the future."))),
        Err(_) => return Ok(issue_form(a, Some("The activation limit must be at least 1."))),
    };
    let mut body = format!("<h1>Created {} temporary licence{}</h1><div class=\"card\"><p><b>Copy the serials now:</b> this page is the only time they're shown in full (unless you use Show serial on a licence, which is recorded).</p><table><tr><th>Serial</th><th>Licence</th><th>Email</th></tr>", made.len(), if made.len() == 1 { "" } else { "s" });
    for (serial, lic) in &made {
        let mut emailed = "—".to_string();
        if let Some(to) = &lic.email {
            let row = cx.licence(&lic.id).await?.ok_or("licence vanished")?;
            emailed = match mail::send(cx, &mail::licence_message(&row, serial, to)).await {
                Ok(true) => {
                    cx.mark_emailed(&lic.id, to, a.actor).await?;
                    format!("sent to {}", esc(to))
                }
                Ok(false) => "email isn't set up".into(),
                Err(e) => format!("<span class=\"bad\">failed: {}</span>", esc(&e)),
            };
        }
        body.push_str(&format!("<tr><td class=\"mono\" style=\"font-size:17px\">{}</td><td><a class=\"mono\" href=\"{}/licence/{id}\">{id}</a></td><td>{emailed}</td></tr>", esc(serial), a.base, id = esc(&lic.id)));
    }
    body.push_str(&format!("</table></div><p>{}</p>", esc(&describe(&opts.validity, opts.max_activations))));
    Ok(a.page("Created", &body))
}

async fn activations<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>) -> Result<Resp, String> {
    let rows = cx.report().await?;
    let flagged = rows.iter().filter(|r| r.flagged).count();
    let mut body = format!(
        "<h1>Activations</h1><p class=\"mute\">Every activated licence, busiest first. A licence is flagged when more than {SHARING_FLAG_MACHINES} new computers activated it in the last {SHARING_WINDOW_DAYS} days: it may be shared. Flagged now: <b>{flagged}</b>.</p>"
    );
    if rows.is_empty() {
        body.push_str("<p class=\"mute\">Nothing activated yet.</p>");
    } else {
        body.push_str(&format!("<div class=\"scroll\"><table><tr><th>Licence</th><th>Type</th><th>Owner</th><th>Used</th><th>New computers ({SHARING_WINDOW_DAYS} days)</th><th>Active now</th><th></th></tr>"));
        for r in &rows {
            body.push_str(&format!(
                "<tr><td><a class=\"mono\" href=\"{}/licence/{id}\">{id}</a></td><td>{}</td><td>{}</td><td>{} of {}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                a.base,
                if r.kind == LicenceKind::Perpetual { "Perpetual" } else { "Temporary" },
                esc(r.owner.as_deref().unwrap_or("")),
                r.activations_used,
                r.max_activations,
                r.recent_machines,
                r.live_machines,
                if r.flagged { "<span class=\"pill bad\">possibly shared</span>" } else { "" },
                id = esc(&r.licence_id)
            ));
        }
        body.push_str("</table></div>");
    }
    Ok(a.page("Activations", &body))
}

/// The release from the Releases form, checked.
fn release_from_form(form: &Form, now: i64) -> Result<ReleaseManifest, String> {
    let version = form.get("version").trim_start_matches(['v', 'V']).to_string();
    if release::version_key(&version).is_none() {
        return Err("The version must look like 0.3.0.".into());
    }
    let when = form.get("released_at");
    let released_at = match when.parse::<i64>() {
        Ok(t) if t > 1_000_000_000 => t,
        _ => parse_day(&when).ok_or("The release date must be a date (YYYY-MM-DD) or Unix seconds.")?,
    };
    if released_at > now + filescape_licence::DAY {
        return Err("The release date can't be in the future.".into());
    }
    let link = |k: &str| -> Result<Option<String>, String> {
        let v = form.get(k);
        if v.is_empty() {
            Ok(None)
        } else if release::is_ours(&v) {
            Ok(Some(v))
        } else {
            Err(format!("{v} isn't a https link on filescape.app; the app only opens links there."))
        }
    };
    let summary = form.get("summary");
    if summary.chars().count() > 300 {
        return Err("Keep the summary to 300 characters; link to release notes for more.".into());
    }
    Ok(ReleaseManifest {
        version,
        released_at,
        signed_at: now,
        summary: (!summary.is_empty()).then_some(summary),
        notes_url: link("notes_url")?,
        downloads: Downloads { windows: link("windows")?, macos: link("macos")?, linux: link("linux")? },
    })
}

async fn releases<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, msg: Option<&str>) -> Result<Resp, String> {
    let m = msg.map(|m| format!("<div class=\"card\">{}</div>", esc(m))).unwrap_or_default();
    let cur = cx.latest_release().await?;
    let current = match &cur {
        Some(r) => format!(
            "<div class=\"card\"><table><tr><th>Version</th><td><b>{}</b></td></tr><tr><th>Released</th><td>{}</td></tr><tr><th>Summary</th><td>{}</td></tr><tr><th>Release notes</th><td>{}</td></tr>\
             <tr><th>Windows</th><td class=\"mono\">{}</td></tr><tr><th>macOS</th><td class=\"mono\">{}</td></tr><tr><th>Linux</th><td class=\"mono\">{}</td></tr></table>\
             <form method=\"post\" action=\"{}/releases\" style=\"margin-top:10px\"><button name=\"withdraw\" value=\"1\">Stop announcing it</button></form></div>",
            esc(&r.version),
            date(r.released_at),
            esc(r.summary.as_deref().unwrap_or("—")),
            esc(r.notes().unwrap_or("—")),
            esc(&r.download_for("windows")),
            esc(&r.download_for("macos")),
            esc(&r.download_for("linux")),
            a.base,
        ),
        None => "<p class=\"mute\">No release is announced. Filescape shows no update notice.</p>".into(),
    };
    let v = |f: fn(&ReleaseManifest) -> Option<&str>| esc(cur.as_ref().and_then(f).unwrap_or(""));
    let today = iso_day(cx.now);
    let body = format!(
        "<h1>Releases</h1>{m}<h2>Announced now</h2>{current}\
         <h2>Announce a release</h2><p class=\"mute\">Upload the downloads to R2 first (SETUP.md step 9). Every copy of Filescape asks for this at launch and, when the version is newer than its own, shows a notice saying whether the person's licence covers it. The signature is made by this server; nothing else is needed.</p>\
         <form method=\"post\" action=\"{b}/releases\" class=\"card\" style=\"display:grid;gap:10px;max-width:640px\">\
         <label>Version<input name=\"version\" required placeholder=\"0.3.0\" value=\"\" size=\"12\"></label>\
         <label>Release date <span class=\"mute\">(date of the commit it was built from, YYYY-MM-DD; or exact Unix seconds from <span class=\"mono\">git log -1 --format=%ct v0.3.0</span>). Licences with updates until at least this date are told they're covered.</span><input name=\"released_at\" required value=\"{today}\" size=\"16\"></label>\
         <label>Summary <span class=\"mute\">(optional, one or two lines)</span><input name=\"summary\" maxlength=\"300\" value=\"{}\" style=\"width:100%\"></label>\
         <label>Release notes link <span class=\"mute\">(optional, https on filescape.app)</span><input name=\"notes_url\" value=\"{}\" style=\"width:100%\"></label>\
         <details><summary>Download links <span class=\"mute\">(leave empty for downloads.filescape.app/latest/&lt;os&gt;)</span></summary>\
         <label>Windows<input name=\"windows\" value=\"{}\" style=\"width:100%\"></label><label>macOS<input name=\"macos\" value=\"{}\" style=\"width:100%\"></label><label>Linux<input name=\"linux\" value=\"{}\" style=\"width:100%\"></label></details>\
         <div><button class=\"primary\">Announce</button></div></form>",
        v(|r| r.summary.as_deref()),
        v(|r| r.notes()),
        v(|r| r.downloads.windows.as_deref()),
        v(|r| r.downloads.macos.as_deref()),
        v(|r| r.downloads.linux.as_deref()),
        b = a.base,
    );
    Ok(a.page("Releases", &body))
}

async fn settings<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, msg: Option<&str>) -> Result<Resp, String> {
    let limit = cx.default_perpetual_limit().await?;
    let m = msg.map(|m| format!("<div class=\"card\">{}</div>", esc(m))).unwrap_or_default();
    let cfg = cx.cfg;
    let yes_no = |b: bool| if b { "<span class=\"good\">set up</span>" } else { "<span class=\"warn\">not set up</span>" };
    let body = format!(
        "<h1>Settings</h1>{m}<form method=\"post\" action=\"{}/settings\" class=\"card row\"><label>Computers at once, new perpetual licences<input name=\"perpetual_limit\" type=\"number\" min=\"1\" max=\"1000\" value=\"{limit}\" style=\"width:100px\"></label><button class=\"primary\">Save</button></form>\
         <p class=\"mute\">Licences already sold keep their limit; change one on its own page. Temporary licences get their limit when they're created.</p>\
         <h2>Server</h2><div class=\"card\"><table><tr><th>Signing key ID</th><td class=\"mono\">{}</td></tr><tr><th>Paddle</th><td>{}{}</td></tr><tr><th>Email (Resend)</th><td>{}</td></tr><tr><th>Cloudflare Access check</th><td>{}</td></tr><tr><th>Server time</th><td>{}</td></tr></table></div>",
        a.base,
        esc(cfg.signer.key_id()),
        yes_no(cfg.paddle.is_some()),
        cfg.paddle.as_ref().map_or(String::new(), |p| format!(" <span class=\"mute\">({}, {})</span>", esc(&p.api_base), if p.api_key.is_some() { "API key set" } else { "<span class=\"warn\">no API key: buyers' emails can't be looked up</span>" })),
        yes_no(cfg.mail.is_some()),
        if cfg.access.is_some() { yes_no(true) } else { "<span class=\"warn\">off (local development)</span>" },
        date(cx.now),
    );
    Ok(a.page("Settings", &body))
}
