//! admin.filescape.app: licences, Create Temporary Licence, revoke, reset activations, limits,
//! contact form messages (read, reply, close),
//! the Activations page and settings. Signed in through Cloudflare Access ([`crate::access`]);
//! every change is recorded in the licence's history with the admin's email.

use crate::db::LicenceRow;
use crate::maintenance::{GrantRow, NewGrant, Outcome, length};
use crate::renewal::{PriceChoice, RenewalSettings};
use crate::store::{ListFilter, describe};
use crate::web::{Form, date, encode, esc, iso_day, opt_date, page, parse_day};
use crate::{Ctx, Db, Net, Req, Resp, mail};
use filescape_licence::maintenance::{self as mt, Grant, GrantKind, MaintenanceStatus};
use filescape_licence::model::{Licence, LicenceKind, Status, Validity};
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
            "<b>Filescape admin</b><a href=\"{b}/\">Licences</a><a href=\"{b}/issue\">Create temporary licence</a><a href=\"{b}/activations\">Activations</a><a href=\"{b}/maintenance\">Maintenance</a><a href=\"{b}/renewals\">Renewals</a><a href=\"{b}/messages\">Messages</a><a href=\"{b}/releases\">Releases</a><a href=\"{b}/settings\">Settings</a><span class=\"mute\" style=\"margin-left:auto\">{}</span>",
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
        ("GET", ["renewals"]) => renewals(cx, &a, req.query_param("msg").as_deref(), None).await,
        ("POST", ["renewals"]) => match renewal_from_form(cx, &form) {
            Ok(s) => {
                cx.set_renewal_settings(s, actor).await?;
                Ok(Resp::redirect(&format!("{base}/renewals?msg={}", encode("Renewal settings saved."))))
            }
            // Shown again with what was typed, so nothing has to be entered twice.
            Err(e) => renewals(cx, &a, Some(&format!("Not saved: {e}")), Some(&form)).await,
        },
        ("GET", ["maintenance"]) => Ok(bulk_form(&a, req.query_param("msg").as_deref())),
        ("POST", ["maintenance"]) => bulk_extend(cx, &a, &form).await,
        ("GET", ["messages"]) => messages(cx, &a, req.query_param("f").as_deref() == Some("all")).await,
        ("GET", ["messages", id]) => message(cx, &a, id, req.query_param("msg").as_deref()).await,
        ("POST", ["messages", id, action]) => message_action(cx, &a, id, action, &form).await,
        ("GET", ["releases"]) => releases(cx, &a, req.query_param("msg").as_deref()).await,
        ("POST", ["releases"]) => {
            let msg = if form.has("withdraw") {
                cx.set_latest_release(None, actor).await?;
                "Filescape no longer announces a new release.".to_string()
            } else if form.has("hide") || form.has("show") {
                let (v, listed) = if form.has("show") { (form.get("show"), true) } else { (form.get("hide"), false) };
                if cx.set_listed(&v, listed, actor).await? {
                    format!("Filescape {v} is {} the download lists.", if listed { "back on" } else { "now hidden from" })
                } else {
                    format!("There's no version {v}.")
                }
            } else if form.has("list_only") {
                match release_from_form(&form, cx.now) {
                    Ok(m) => {
                        cx.add_release(&m, actor).await?;
                        format!("Filescape {} is on the download lists. It isn't announced to copies of Filescape.", m.version)
                    }
                    Err(e) => e,
                }
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
    let new_messages = cx.new_message_count().await?;
    let rows = cx.list(&q, filter, 200).await?;
    let opts = [("", "All"), ("perpetual", "Perpetual"), ("temporary", "Temporary"), ("expiring", "Ending in 7 days"), ("revoked", "Revoked"), ("refunds", "Refunds and chargebacks"), ("unmailed", "Paid, not emailed")]
        .iter()
        .map(|(v, l)| format!("<option value=\"{v}\"{}>{l}</option>", if *v == f { " selected" } else { "" }))
        .collect::<String>();
    let unread = match new_messages {
        0 => String::new(),
        1 => format!("<div class=\"card\"><a href=\"{}/messages\"><b>1 new message</b></a> from the contact form.</div>", a.base),
        n => format!("<div class=\"card\"><a href=\"{}/messages\"><b>{n} new messages</b></a> from the contact form.</div>", a.base),
    };
    let mut body = format!(
        "<h1>Licences</h1>{unread}<div class=\"stats\"><div><b>{total}</b>licences</div><div><b>{perpetual}</b>perpetual</div><div><b>{temp_live}</b>temporary, not expired</div><div><b>{live_acts}</b>computers activated</div></div>\
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
    body.push_str("</table></div>");
    if l.kind() == LicenceKind::Perpetual {
        body.push_str(&maintenance_section(cx, a, l).await?);
    }
    body.push_str("<h2>Actions</h2><div class=\"card\">");
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
    let needs_reason = matches!(action, "revoke" | "reinstate" | "reset" | "limit" | "void" | "reveal" | "extend" | "reverse-grant");
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
            "extend" => {
                let months: u32 = form.get("months").parse().unwrap_or(0);
                let days: u32 = form.get("days").parse().unwrap_or(0);
                if months == 0 && days == 0 || months > 60 || days > 3650 {
                    "Give a length: up to 60 months and 3650 days.".into()
                } else {
                    let g = NewGrant { grant: Grant::complimentary(cx.now, months, days), txn: None, amount: None, currency: None, actor: a.actor, reason: &reason };
                    outcome_text(&cx.add_grant(id, &g, None).await?)
                }
            }
            "reverse-grant" => match form.get("grant").parse::<i64>() {
                Ok(gid) => outcome_text(&cx.reverse_grant(id, gid, false, a.actor, &reason).await?),
                Err(_) => "No such grant.".into(),
            },
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
    let mut rows = String::new();
    for e in cx.catalogue(true).await? {
        let toggle = if e.listed {
            format!("<button name=\"hide\" value=\"{}\">Hide</button>", esc(&e.m.version))
        } else {
            format!("<span class=\"pill\">Hidden</span> <button name=\"show\" value=\"{}\">Show</button>", esc(&e.m.version))
        };
        let links = e.links().iter().map(|(l, u)| format!("<a href=\"{}\">{l}</a>", esc(u))).collect::<Vec<_>>().join(" · ");
        rows.push_str(&format!(
            "<tr><td><b>{}</b></td><td>{}</td><td style=\"font-size:13px\">{links}</td><td><form method=\"post\" action=\"{}/releases\">{toggle}</form></td></tr>",
            esc(&e.m.version),
            date(e.m.released_at),
            a.base
        ));
    }
    let catalogue = if rows.is_empty() {
        "<p class=\"mute\">No versions yet. Announcing a release adds it here.</p>".to_string()
    } else {
        format!("<p class=\"mute\">Customers see the listed versions on account.filescape.app/versions and, signed in, which ones their licence unlocks.</p><div class=\"scroll\"><table><tr><th>Version</th><th>Released</th><th>Downloads</th><th></th></tr>{rows}</table></div>")
    };
    let body = format!(
        "<h1>Releases</h1>{m}<h2>Announced now</h2>{current}\
         <h2>Announce a release</h2><p class=\"mute\">Upload the downloads to R2 first (SETUP.md step 9). Every copy of Filescape asks for this at launch and, when the version is newer than its own, shows a notice saying whether the person's licence covers it. The signature is made by this server; nothing else is needed.</p>\
         <form method=\"post\" action=\"{b}/releases\" class=\"card\" style=\"display:grid;gap:10px;max-width:640px\">\
         <label>Version<input name=\"version\" required placeholder=\"0.3.0\" value=\"\" size=\"12\"></label>\
         <label>Release date <span class=\"mute\">(date of the commit it was built from, YYYY-MM-DD; or exact Unix seconds from <span class=\"mono\">git log -1 --format=%ct v0.3.0</span>). Licences with updates until at least this date are told they're covered.</span><input name=\"released_at\" required value=\"{today}\" size=\"16\"></label>\
         <label>Summary <span class=\"mute\">(optional, one or two lines)</span><input name=\"summary\" maxlength=\"300\" value=\"{}\" style=\"width:100%\"></label>\
         <label>Release notes link <span class=\"mute\">(optional, https on filescape.app)</span><input name=\"notes_url\" value=\"{}\" style=\"width:100%\"></label>\
         <details><summary>Download links <span class=\"mute\">(leave empty for downloads.filescape.app/latest/&lt;os&gt;, or the website's download section on Linux)</span></summary>\
         <label>Windows<input name=\"windows\" value=\"{}\" style=\"width:100%\"></label><label>macOS<input name=\"macos\" value=\"{}\" style=\"width:100%\"></label><label>Linux<input name=\"linux\" value=\"{}\" style=\"width:100%\"></label></details>\
         <div class=\"row\"><button class=\"primary\">Announce</button><button name=\"list_only\" value=\"1\">Add to the download lists only</button></div>\
         <p class=\"mute\" style=\"margin:0\">Use <b>Add to the download lists only</b> for an older version, so customers whose maintenance ended can still download the newest version their licence unlocks. Each version's files go in R2 under <span class=\"mono\">&lt;version&gt;/&lt;os&gt;</span> as well as <span class=\"mono\">latest/&lt;os&gt;</span> (SETUP.md step 9).</p></form>\
         <h2>All versions</h2>{catalogue}",
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

// ---------- contact form messages ----------

fn message_pill(status: &str) -> &'static str {
    match status {
        "new" => "<span class=\"pill warn\">New</span>",
        "answered" => "<span class=\"pill good\">Answered</span>",
        _ => "<span class=\"pill mute\">Closed</span>",
    }
}

fn snippet(s: &str, n: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > n { format!("{}…", one.chars().take(n).collect::<String>()) } else { one }
}

async fn messages<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, all: bool) -> Result<Resp, String> {
    let rows = cx.messages(!all, 300).await?;
    let b = a.base;
    let mut body = format!(
        "<h1>Messages</h1><p class=\"mute\">From the contact form at filescape.app/contact. Replies go out from Filescape's address, so yours stays private.{}</p>\
         <p>{}</p>",
        if cx.cfg.contact.notify_to.is_some() { " New messages are also emailed to you." } else { " Set the CONTACT_NOTIFY_TO secret to be emailed when one arrives." },
        if all { format!("Showing all. <a href=\"{b}/messages\">Show open only</a>") } else { format!("Showing open messages. <a href=\"{b}/messages?f=all\">Show closed too</a>") }
    );
    if rows.is_empty() {
        body.push_str("<p class=\"mute\">No messages.</p>");
    } else {
        body.push_str("<div class=\"scroll\"><table><tr><th>Received</th><th>From</th><th>Topic</th><th>Message</th><th>Status</th></tr>");
        for m in &rows {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}<br><span class=\"mute\">{}</span></td><td>{}</td><td><a href=\"{b}/messages/{}\">{}</a></td><td>{}</td></tr>",
                date(m.created_at),
                esc(m.name.as_deref().unwrap_or("")),
                esc(&m.email),
                esc(&m.topic),
                esc(&m.id),
                esc(&snippet(&m.body, 120)),
                message_pill(&m.status),
            ));
        }
        body.push_str("</table></div>");
    }
    Ok(a.page("Messages", &body))
}

async fn message<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, id: &str, msg: Option<&str>) -> Result<Resp, String> {
    let Some(m) = cx.message(id).await? else { return Ok(Resp::html(404, page("Not found", &a.nav(), "<h1>Message not found</h1>"))) };
    let b = a.base;
    let note = msg.map(|t| format!("<div class=\"card\">{}</div>", esc(t))).unwrap_or_default();
    let replies = cx.replies(&m.id).await?;
    let others: Vec<_> = cx.messages_from(&m.email).await?.into_iter().filter(|o| o.id != m.id).collect();
    let licences = cx.licences_by_email(&m.email).await?;
    let mut body = format!(
        "<p><a href=\"{b}/messages\">← Messages</a></p><h1>{} {}</h1>{note}\
         <div class=\"card\"><table><tr><th>From</th><td>{} &lt;{}&gt;</td></tr><tr><th>Received</th><td>{}</td></tr><tr><th>Notified</th><td>{}</td></tr><tr><th>Licences</th><td>{}</td></tr></table>\
         <div style=\"white-space:pre-wrap;margin-top:12px;overflow-wrap:anywhere\">{}</div></div>",
        esc(&m.topic),
        message_pill(&m.status),
        esc(m.name.as_deref().unwrap_or("")),
        esc(&m.email),
        date(m.created_at),
        opt_date(m.notified_at),
        if licences.is_empty() { "None for this address".to_string() } else { format!("<a href=\"{b}/?q={}\">{} licence{}</a>", encode(&m.email), licences.len(), if licences.len() == 1 { "" } else { "s" }) },
        esc(&m.body),
    );
    for r in &replies {
        body.push_str(&format!(
            "<div class=\"card\"><p class=\"mute\" style=\"margin-top:0\">Reply by {} on {}</p><div style=\"white-space:pre-wrap;overflow-wrap:anywhere\">{}</div></div>",
            esc(&r.actor),
            date(r.at),
            esc(&r.body)
        ));
    }
    let mail_ready = cx.cfg.mail.is_some();
    body.push_str(&format!(
        "<h2>Reply</h2><form method=\"post\" action=\"{b}/messages/{id}/reply\" class=\"card\" style=\"display:grid;gap:10px;max-width:760px\">\
         <p class=\"mute\" style=\"margin:0\">Emailed to {email} from Filescape's address, with their message quoted below it and a link back to the contact form. Your own address isn't shown.</p>\
         <textarea name=\"reply\" rows=\"10\" required style=\"width:100%\"{dis}></textarea><div class=\"row\"><button class=\"primary\"{dis}>Send reply</button>{why}</div></form>",
        id = esc(&m.id),
        email = esc(&m.email),
        dis = if mail_ready { "" } else { " disabled" },
        why = if mail_ready { "" } else { "<span class=\"warn\">Email isn't set up (RESEND_API_KEY, MAIL_FROM).</span>" },
    ));
    let toggle = if m.status == "closed" { ("reopen", "Reopen") } else { ("close", "Close") };
    body.push_str(&format!(
        "<div class=\"row\"><form method=\"post\" action=\"{b}/messages/{id}/{}\"><button>{}</button></form>\
         <form method=\"post\" action=\"{b}/messages/{id}/delete\" onsubmit=\"return confirm('Delete this message and its replies for good?')\"><button class=\"danger\">Delete</button></form></div>",
        toggle.0,
        toggle.1,
        id = esc(&m.id),
    ));
    if !others.is_empty() {
        body.push_str("<h2>Other messages from this address</h2><div class=\"scroll\"><table><tr><th>Received</th><th>Topic</th><th>Message</th><th>Status</th></tr>");
        for o in &others {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td><a href=\"{b}/messages/{}\">{}</a></td><td>{}</td></tr>",
                date(o.created_at),
                esc(&o.topic),
                esc(&o.id),
                esc(&snippet(&o.body, 100)),
                message_pill(&o.status)
            ));
        }
        body.push_str("</table></div>");
    }
    Ok(a.page("Message", &body))
}

async fn message_action<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, id: &str, action: &str, form: &Form) -> Result<Resp, String> {
    let Some(m) = cx.message(id).await? else { return Ok(Resp::html(404, page("Not found", &a.nav(), "<h1>Message not found</h1>"))) };
    let b = a.base;
    let back = |msg: &str| Ok(Resp::redirect(&format!("{b}/messages/{}?msg={}", encode(&m.id), encode(msg))));
    match action {
        "reply" => match cx.reply_to_message(&m, &form.get("reply"), a.actor).await? {
            Ok(()) => back(&format!("Reply sent to {}.", m.email)),
            Err(why) => back(&why),
        },
        "close" => {
            cx.set_message_status(&m.id, "closed").await?;
            Ok(Resp::redirect(&format!("{b}/messages")))
        }
        "reopen" => {
            let status = if cx.replies(&m.id).await?.is_empty() { "new" } else { "answered" };
            cx.set_message_status(&m.id, status).await?;
            back("Reopened.")
        }
        "delete" => {
            cx.delete_message(&m.id).await?;
            Ok(Resp::redirect(&format!("{b}/messages")))
        }
        _ => Ok(Resp::html(404, page("Not found", &a.nav(), "<h1>Not found</h1>"))),
    }
}

fn outcome_text(o: &Outcome) -> String {
    match o {
        Outcome::Applied { before, after } => format!("Maintenance changed: updates included until {} (was {}).", day(*after), day(*before)),
        Outcome::AlreadyDone => "Nothing to do: that was already done.".into(),
        Outcome::Refused(r) => r.message().into(),
        Outcome::NotFound => "No such licence or grant (temporary licences have no maintenance).".into(),
        Outcome::NotReversible => "Only complimentary extensions can be reversed here. Paid renewals are reversed by a refund in Paddle; the purchase never is.".into(),
    }
}

fn day(t: i64) -> String {
    date(t).split(',').next().unwrap_or_default().to_string()
}

fn grant_paid(g: &GrantRow) -> String {
    match (&g.amount, &g.currency) {
        (Some(a), Some(c)) => format!("{} {}", esc(a), esc(c)),
        (Some(a), None) => esc(a),
        _ => "—".into(),
    }
}

/// The licence page's Maintenance card: status, the ledger, and the complimentary extension form.
async fn maintenance_section<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, l: &Licence) -> Result<String, String> {
    let b = a.base;
    let until = l.validity.updates_until().unwrap_or(0);
    let state = match mt::status(until, cx.now) {
        MaintenanceStatus::Active { days_left, .. } => format!("<span class=\"pill good\">Active</span> {days_left} days left"),
        MaintenanceStatus::Ended { .. } => "<span class=\"pill\">Ended</span> <span class=\"mute\">the licence stays valid; releases up to this date keep Pro</span>".into(),
    };
    let grants = cx.grants_of(&l.id).await?;
    let mut s = format!("<h2>Maintenance</h2><div class=\"card\"><p>Updates included until <b>{}</b> {state}</p>", day(until));
    if grants.is_empty() {
        s.push_str("<p class=\"mute\">No ledger yet: the purchase grant is added with the current date at the first change.</p>");
    } else {
        s.push_str("<div class=\"scroll\"><table><tr><th>When</th><th>Grant</th><th>Paid</th><th>Until before → after</th><th>By</th><th>Reason</th><th></th></tr>");
        for g in &grants {
            let what = match g.grant.kind {
                GrantKind::Purchase => "Purchase".to_string(),
                k => format!("{} {}", k.label(), length(g.grant.months, g.grant.days)),
            };
            let txn = g.txn.as_deref().map(|t| format!("<div class=\"mute mono\" style=\"font-size:12px\">{}</div>", esc(t))).unwrap_or_default();
            let span = match (g.before_until, g.after_until) {
                (Some(x), Some(y)) => format!("{} → {}", day(x), day(y)),
                (None, Some(y)) => day(y),
                _ => "—".into(),
            };
            let last = if let Some(t) = g.reversed_at {
                format!("<span class=\"bad\">reversed {}</span><br><span class=\"mute\">{} · {}</span>", date(t), esc(g.reversed_by.as_deref().unwrap_or("")), esc(g.reverse_reason.as_deref().unwrap_or("")))
            } else if g.grant.admin_reversible() {
                format!(
                    "<form method=\"post\" action=\"{b}/licence/{}/reverse-grant\" class=\"row\"><input type=\"hidden\" name=\"grant\" value=\"{}\"><input name=\"reason\" required placeholder=\"Reason\" size=\"14\"><button class=\"danger\">Reverse</button></form>",
                    esc(&l.id),
                    g.id
                )
            } else {
                String::new()
            };
            s.push_str(&format!(
                "<tr><td>{}</td><td>{what}{txn}</td><td>{}</td><td>{span}</td><td class=\"mute\">{}</td><td>{}</td><td>{last}</td></tr>",
                date(g.grant.at),
                grant_paid(g),
                esc(&g.actor),
                esc(g.reason.as_deref().unwrap_or("")),
            ));
        }
        s.push_str("</table></div>");
    }
    if l.revoked_at.is_none() {
        s.push_str(&format!(
            "<form method=\"post\" action=\"{b}/licence/{}/extend\" class=\"row\" style=\"margin:8px 0\"><label>Months<input name=\"months\" type=\"number\" min=\"0\" max=\"60\" value=\"0\" style=\"width:70px\"></label><label>Days<input name=\"days\" type=\"number\" min=\"0\" max=\"3650\" value=\"30\" style=\"width:80px\"></label><label>Reason<input name=\"reason\" required size=\"30\" placeholder=\"e.g. support gesture\"></label><button>Add complimentary maintenance</button></form>\
             <p class=\"mute\" style=\"margin:-4px 0 0\">Extends from the current end date, or from today if maintenance has ended. Recorded with your email, the reason and the dates before and after. Only complimentary grants can be reversed here, so paid time can't be removed by mistake.</p>",
            esc(&l.id)
        ));
    }
    s.push_str("</div>");
    Ok(s)
}

fn bulk_form(a: &A<'_>, msg: Option<&str>) -> Resp {
    let m = msg.map(|m| format!("<div class=\"card\" style=\"white-space:pre-wrap\">{}</div>", esc(m))).unwrap_or_default();
    a.page(
        "Maintenance",
        &format!(
            "<h1>Complimentary maintenance for several licences</h1>{m}<div class=\"card\"><form method=\"post\" action=\"{}/maintenance\">\
             <label>Licence IDs, one per line<textarea name=\"ids\" rows=\"8\" style=\"width:100%\" required></textarea></label>\
             <div class=\"row\"><label>Months<input name=\"months\" type=\"number\" min=\"0\" max=\"60\" value=\"0\" style=\"width:70px\"></label><label>Days<input name=\"days\" type=\"number\" min=\"0\" max=\"3650\" value=\"30\" style=\"width:80px\"></label>\
             <label>Reason<input name=\"reason\" required size=\"40\"></label><button>Extend all</button></div></form>\
             <p class=\"mute\">Each licence gets its own complimentary grant, extending from its own end date (or from today if it has ended). Temporary licences are skipped. At most 500 at once.</p></div>",
            a.base
        ),
    )
}

async fn bulk_extend<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, form: &Form) -> Result<Resp, String> {
    let reason = form.get("reason");
    let months: u32 = form.get("months").parse().unwrap_or(0);
    let days: u32 = form.get("days").parse().unwrap_or(0);
    let mut ids: Vec<String> = form.get("ids").split(|c: char| c.is_whitespace() || c == ',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|i| seen.insert(i.clone()));
    let msg = if reason.is_empty() {
        "Give a reason: it goes in each licence's history.".to_string()
    } else if months == 0 && days == 0 || months > 60 || days > 3650 {
        "Give a length: up to 60 months and 3650 days.".into()
    } else if ids.is_empty() || ids.len() > 500 {
        "List between 1 and 500 licence IDs.".into()
    } else {
        let done = cx.extend_many(&ids, months, days, a.actor, &reason).await?;
        let ok = done.iter().filter(|(_, o)| matches!(o, Outcome::Applied { .. })).count();
        let mut m = format!("Extended {ok} of {} licences ({}).", done.len(), length(months, days));
        for (id, o) in done.iter().filter(|(_, o)| !matches!(o, Outcome::Applied { .. })) {
            m.push_str(&format!("\n{id}: {}", outcome_text(o)));
        }
        m
    };
    Ok(Resp::redirect(&format!("{}/maintenance?msg={}", a.base, encode(&msg))))
}

fn renewal_from_form<D: Db, N: Net>(cx: &Ctx<'_, D, N>, form: &Form) -> Result<RenewalSettings, String> {
    let purchase = cx.cfg.paddle.as_ref().map(|p| p.price_ids.clone()).unwrap_or_default();
    let choice = |key: &str| -> Result<Option<PriceChoice>, String> {
        let id = form.get(&format!("{key}_id"));
        let label = form.get(&format!("{key}_label"));
        if id.is_empty() {
            return Ok(None);
        }
        if !id.starts_with("pri_") || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("{id} isn't a Paddle price ID (pri_…)."));
        }
        if purchase.contains(&id) {
            return Err(format!("{id} is a licence purchase price (PADDLE_PRICE_IDS). A renewal needs its own price, or renewals would issue new licences."));
        }
        if label.is_empty() || label.len() > 40 {
            return Err(format!("Give {id} a short label, such as $2.99."));
        }
        Ok(Some(PriceChoice { price_id: id, label }))
    };
    let s = RenewalSettings {
        enabled: form.has("enabled"),
        standard: choice("standard")?,
        promo: choice("promo")?,
        use_promo: form.has("use_promo"),
        enterprise: choice("enterprise")?,
        retired: Vec::new(),
        months: 12,
    };
    if s.enabled && s.offered().is_none() {
        return Err("To offer renewals, set the standard price (or the promotional one with \"Offer the promotional price\" ticked).".into());
    }
    Ok(s)
}

async fn renewals<D: Db, N: Net>(cx: &Ctx<'_, D, N>, a: &A<'_>, msg: Option<&str>, draft: Option<&Form>) -> Result<Resp, String> {
    let saved = cx.renewal_settings().await?;
    // A form that didn't validate: show its values (unchecked) instead of the saved ones.
    let s = match draft {
        None => saved.clone(),
        Some(f) => {
            let raw = |k: &str| {
                let (id, label) = (f.get(&format!("{k}_id")), f.get(&format!("{k}_label")));
                (!id.is_empty() || !label.is_empty()).then_some(PriceChoice { price_id: id, label })
            };
            RenewalSettings { enabled: f.has("enabled"), standard: raw("standard"), promo: raw("promo"), use_promo: f.has("use_promo"), enterprise: raw("enterprise"), ..saved.clone() }
        }
    };
    let client = cx.cfg.paddle.as_ref().is_some_and(|p| p.client_token.is_some());
    let m = msg.map(|m| format!("<div class=\"card\">{}</div>", esc(m))).unwrap_or_default();
    let field = |key: &str, title: &str, c: &Option<PriceChoice>| {
        let (id, label) = c.as_ref().map_or((String::new(), String::new()), |c| (c.price_id.clone(), c.label.clone()));
        format!(
            "<div class=\"row\" style=\"margin:8px 0\"><label>{title} price ID<input name=\"{key}_id\" value=\"{}\" size=\"34\" placeholder=\"pri_…\"></label><label>Shown as<input name=\"{key}_label\" value=\"{}\" size=\"10\" placeholder=\"e.g. $2.99\"></label></div>",
            esc(&id),
            esc(&label)
        )
    };
    let offered = match saved.offered() {
        Some(p) => format!("<span class=\"pill good\">Offered</span> at {} ({})", esc(&p.label), esc(&p.price_id)),
        None => "<span class=\"pill\">Not offered</span>".into(),
    };
    let warn = if client { "" } else { "<p class=\"warn\">PADDLE_CLIENT_TOKEN isn't set on the Worker, so the checkout can't open even when renewals are on.</p>" };
    let mut body = format!(
        "<h1>Maintenance renewals</h1>{m}<div class=\"card\"><p>{offered}</p>{warn}<form method=\"post\" action=\"{b}/renewals\">\
         <label style=\"flex-direction:row;align-items:center;gap:8px\"><input type=\"checkbox\" name=\"enabled\" value=\"1\"{}>Offer renewals on the account page and in the app</label>\
         {}{}<label style=\"flex-direction:row;align-items:center;gap:8px\"><input type=\"checkbox\" name=\"use_promo\" value=\"1\"{}>Offer the promotional price instead of the standard one</label>{}\
         <p><button class=\"primary\">Save</button></p></form>\
         <p class=\"mute\">Create each price in Paddle as a one-time price (not recurring) on the renewal product; the amount, tax and currencies come from Paddle. \"Shown as\" is only the label before checkout. Discount codes set up in Paddle work at checkout. Each renewal adds 12 months: to the end date if maintenance is running (or ends that day), otherwise from the payment. Renewals are non-refundable; a refund made in Paddle anyway, or a chargeback, takes the 12 months back. Price IDs you replace stay accepted for checkouts already open{}.</p></div>",
        if s.enabled { " checked" } else { "" },
        field("standard", "Standard", &s.standard),
        field("promo", "Promotional", &s.promo),
        if s.use_promo { " checked" } else { "" },
        field("enterprise", "Enterprise", &s.enterprise),
        if s.retired.is_empty() { String::new() } else { format!(" (now: {})", esc(&s.retired.join(", "))) },
        b = a.base
    );
    let problems = cx.renewal_problems(50).await?;
    if !problems.is_empty() {
        body.push_str("<h2>Payments that need attention</h2><div class=\"scroll\"><table><tr><th>When</th><th>What happened</th></tr>");
        for r in &problems {
            body.push_str(&format!("<tr><td>{}</td><td>{}</td></tr>", date(crate::db::int(r, "received_at").unwrap_or(0)), esc(&crate::db::text(r, "outcome").unwrap_or_default())));
        }
        body.push_str("</table></div>");
    }
    let rows = cx.recent_renewals(100).await?;
    body.push_str("<h2>Recent renewals</h2>");
    if rows.is_empty() {
        body.push_str("<p class=\"mute\">None yet.</p>");
    } else {
        body.push_str("<div class=\"scroll\"><table><tr><th>When</th><th>Licence</th><th>Paid</th><th>Until before → after</th><th>Transaction</th><th></th></tr>");
        for r in &rows {
            let t = |k: &str| crate::db::text(r, k).unwrap_or_default();
            let i = |k: &str| crate::db::int(r, k);
            let paid = if t("amount").is_empty() { "—".into() } else { format!("{} {}", esc(&t("amount")), esc(&t("currency"))) };
            let span = match (i("before_until"), i("after_until")) {
                (Some(x), Some(y)) => format!("{} → {}", day(x), day(y)),
                _ => "—".into(),
            };
            let state = if i("reversed_at").is_some() { format!("<span class=\"bad\">reversed</span> <span class=\"mute\">{}</span>", esc(&t("reverse_reason"))) } else { String::new() };
            body.push_str(&format!(
                "<tr><td>{}</td><td><a href=\"{}/licence/{}\">…{}</a><div class=\"mute\">{}</div></td><td>{paid}</td><td>{span}</td><td class=\"mono\">{}</td><td>{state}</td></tr>",
                date(i("at").unwrap_or(0)),
                a.base,
                encode(&t("licence_id")),
                esc(&t("serial_hint")),
                esc(&t("email")),
                esc(&t("txn")),
            ));
        }
        body.push_str("</table></div>");
    }
    Ok(a.page("Renewals", &body))
}
