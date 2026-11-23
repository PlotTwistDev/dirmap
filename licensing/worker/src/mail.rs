//! Email through Resend (https://resend.com): the licence after a purchase, a temporary licence
//! from the admin, and "email me my licence".

use crate::db::LicenceRow;
use crate::web::{date, esc};
use crate::{Ctx, Db, Net};
use filescape_licence::model::{LicenceKind, Validity};

pub struct Message {
    pub to: String,
    pub subject: String,
    pub text: String,
}

/// Whether it was accepted. `Ok(false)`: no mail settings.
pub async fn send<D: Db, N: Net>(cx: &Ctx<'_, D, N>, m: &Message) -> Result<bool, String> {
    let Some(mc) = &cx.cfg.mail else { return Ok(false) };
    let html = format!(
        "<div style=\"font:15px/1.5 system-ui,sans-serif;max-width:560px\">{}</div>",
        m.text
            .split("\n\n")
            .map(|p| {
                let p = esc(p).replace('\n', "<br>");
                if p.starts_with("FST-") || p.starts_with("FSP-") { format!("<p style=\"font:18px ui-monospace,Consolas,monospace\"><b>{p}</b></p>") } else { format!("<p>{p}</p>") }
            })
            .collect::<String>()
    );
    let mut body = serde_json::json!({ "from": mc.from, "to": [m.to], "subject": m.subject, "text": m.text, "html": html });
    if let Some(r) = &mc.reply_to {
        body["reply_to"] = r.clone().into();
    }
    let (status, text) = cx
        .net
        .send("POST", "https://api.resend.com/emails", &[("authorization", format!("Bearer {}", mc.resend_key)), ("content-type", "application/json".into())], Some(body.to_string()))
        .await?;
    if (200..300).contains(&status) { Ok(true) } else { Err(format!("Resend said {status}: {}", text.chars().take(300).collect::<String>())) }
}

fn what(row: &LicenceRow) -> String {
    match row.lic.validity {
        Validity::Perpetual { updates_until } => format!(
            "This is a perpetual licence: it doesn't expire. It includes updates released until {}, and can be activated on up to {} computers.",
            date(updates_until).split(',').next().unwrap_or_default(),
            row.lic.max_activations
        ),
        Validity::FirstActivation { days } => format!(
            "This is a temporary licence for {days} days, counted from the first time it's activated. It can be activated on up to {} computers and can't be renewed.",
            row.lic.max_activations
        ),
        Validity::FixedExpiry { expires_at } => format!(
            "This is a temporary licence until {}. It can be activated on up to {} computers and can't be renewed.",
            date(expires_at),
            row.lic.max_activations
        ),
    }
}

const HOW: &str = "To activate: open Filescape, choose Help > Licence, paste the serial number and press Activate. A computer without internet access can use Offline activation on the same screen, with the page https://account.filescape.app/activate.";

pub fn licence_message(row: &LicenceRow, serial: &str, to: &str) -> Message {
    let paid = row.lic.kind() == LicenceKind::Perpetual;
    let hello = row.lic.owner.as_deref().map_or("Hello,".to_string(), |o| format!("Hello {o},"));
    let intro = if paid { "Thank you for buying Filescape Professional. Here is your serial number:" } else { "Here is your Filescape Professional serial number:" };
    Message {
        to: to.into(),
        subject: if paid { "Your Filescape licence".into() } else { "Your temporary Filescape licence".into() },
        text: format!(
            "{hello}\n\n{intro}\n\n{serial}\n\n{}\n\n{HOW}\n\nKeep this email: you can also get your serial again at https://account.filescape.app.\n\nFilescape (filescape.app)",
            what(row)
        ),
    }
}

pub fn reminder_message(rows: &[(LicenceRow, String)], to: &str) -> Message {
    let list = rows
        .iter()
        .map(|(r, s)| format!("{s}\n{}{}", what(r), if r.lic.revoked_at.is_some() { " (This licence has been revoked.)" } else { "" }))
        .collect::<Vec<_>>()
        .join("\n\n");
    Message {
        to: to.into(),
        subject: "Your Filescape serial numbers".into(),
        text: format!("Hello,\n\nYou asked for the Filescape serial numbers registered to this address:\n\n{list}\n\n{HOW}\n\nIf you didn't ask for this, you can ignore this email.\n\nFilescape (filescape.app)"),
    }
}
