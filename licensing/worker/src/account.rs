//! account.filescape.app: for customers. Offline activation, "email me my serial numbers", and
//! the page Paddle's checkout returns to.

use crate::store::Fail;
use crate::web::{Form, esc, page};
use crate::{Ctx, Db, Net, Req, Resp, mail};

fn nav(base: &str) -> String {
    format!("<b>Filescape</b><a href=\"{base}/\">Your licence</a><a href=\"{base}/activate\">Offline activation</a><a href=\"https://filescape.app/\">filescape.app</a>")
}

fn show(base: &str, title: &str, body: &str) -> Resp {
    Resp::html(200, page(&format!("{title} · Filescape"), &nav(base), body))
}

pub async fn route<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req, path: &str, base: &str) -> Result<Resp, String> {
    match (req.method.as_str(), path) {
        ("GET", "/") => Ok(home(base, None)),
        ("POST", "/find") => find(cx, base, &Form::parse(&req.text())).await,
        ("GET", "/activate") => Ok(offline_page(base, None)),
        ("POST", "/activate") => offline(cx, base, req).await,
        ("GET", "/thanks") => Ok(show(
            base,
            "Thank you",
            "<h1>Thank you for buying Filescape</h1><div class=\"card\"><p>Your serial number is on its way to the email address you gave at checkout. It usually arrives within a minute; check your spam folder if it doesn't.</p>\
             <p>To activate: open Filescape, choose <b>Help &gt; Licence</b>, paste the serial number and press <b>Activate</b>.</p>\
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
            "<h1>Your Filescape licence</h1>{m}<div class=\"card\"><h2 style=\"margin-top:0\">Lost your serial number?</h2><p>Enter the email address you bought with, and we'll send every serial number registered to it.</p>\
             <form method=\"post\" action=\"{base}/find\" class=\"row\"><label>Email<input name=\"email\" type=\"email\" required size=\"32\"></label><button class=\"primary\">Email my serial numbers</button></form></div>\
             <div class=\"card\"><h2 style=\"margin-top:0\">Activating a computer without internet access?</h2><p>Use <a href=\"{base}/activate\">offline activation</a>.</p></div>\
             <div class=\"card\"><h2 style=\"margin-top:0\">Moved to a new computer and out of activations?</h2><p>Contact <a href=\"mailto:support@filescape.app\">support@filescape.app</a> with your serial number and we'll reset them.</p></div>"
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
        Err(Fail::No(why)) => Ok(offline_page(base, Some(why.message()))),
        Err(Fail::Err(e)) => Err(e),
    }
}
