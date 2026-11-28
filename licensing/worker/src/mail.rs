//! Email through Resend (https://resend.com): the licence after a purchase, a temporary licence
//! from the admin, "email me my licence", and the contact form's notices and replies
//! ([`crate::contact`]).

use crate::db::LicenceRow;
use crate::web::{date, esc};
use crate::{Ctx, Db, Net};
use filescape_licence::model::{LicenceKind, Validity};

pub struct Message {
    pub to: String,
    pub subject: String,
    pub text: String,
    /// Overrides `MAIL_REPLY_TO` for this message; `Some("")` sends none.
    pub reply_to: Option<String>,
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
    if let Some(r) = m.reply_to.as_ref().or(mc.reply_to.as_ref()).filter(|r| !r.is_empty()) {
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
            "This is a perpetual licence: it doesn't expire. It includes updates released until {}, and can be active on up to {} computers at once. To move it to a new computer, deactivate it on an old one (in Filescape, or at https://account.filescape.app).",
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

const MAC: &str = "On a Mac? Filescape isn't notarised by Apple yet, so the first time you open it macOS blocks it. Click Done, then go to System Settings > Privacy & Security and click Open Anyway. Step-by-step help: https://filescape.app/mac";

pub fn licence_message(row: &LicenceRow, serial: &str, to: &str) -> Message {
    let paid = row.lic.kind() == LicenceKind::Perpetual;
    let hello = row.lic.owner.as_deref().map_or("Hello,".to_string(), |o| format!("Hello {o},"));
    let intro = if paid { "Thank you for buying Filescape Professional. Here is your serial number:" } else { "Here is your Filescape Professional serial number:" };
    Message {
        to: to.into(),
        subject: if paid { "Your Filescape licence".into() } else { "Your temporary Filescape licence".into() },
        text: format!(
            "{hello}\n\n{intro}\n\n{serial}\n\n{}\n\n{HOW}\n\n{MAC}\n\nKeep this email: you can also get your serial again at https://account.filescape.app.\n\nQuestions? Write to us at https://filescape.app/contact (replies to this email aren't read).\n\nFilescape (filescape.app)",
            what(row)
        ),
        reply_to: None,
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
        text: format!("Hello,\n\nYou asked for the Filescape serial numbers registered to this address:\n\n{list}\n\n{HOW}\n\nIf you didn't ask for this, you can ignore this email.\n\nQuestions? Write to us at https://filescape.app/contact (replies to this email aren't read).\n\nFilescape (filescape.app)"),
        reply_to: None,
    }
}

pub fn login_message(link: &str, to: &str) -> Message {
    Message {
        to: to.into(),
        subject: "Sign in to your Filescape account".into(),
        text: format!(
            "Hello,\n\nUse this link to sign in at account.filescape.app. It works once, for 15 minutes:\n\n{link}\n\nIf you didn't ask for this, you can ignore this email: nobody can sign in without the link.\n\nQuestions? Write to us at https://filescape.app/contact (replies to this email aren't read).\n\nFilescape (filescape.app)"
        ),
        reply_to: None,
    }
}

pub fn renewal_message(row: &LicenceRow, until: filescape_licence::Ts, to: &str) -> Message {
    let hello = row.lic.owner.as_deref().map_or("Hello,".to_string(), |o| format!("Hello {o},"));
    let day = date(until).split(',').next().unwrap_or_default().to_string();
    Message {
        to: to.into(),
        subject: "Your Filescape maintenance is extended".into(),
        text: format!(
            "{hello}\n\nThank you for renewing. Software maintenance on your Filescape licence (serial ending {}) now runs until {day}: every Filescape release published until then is included.\n\nYour serial number, your activated computers and everything your licence already covered stay as they are. Filescape picks up the new date at its next weekly licence check; to see it at once, choose Help > Licence and press Check now, or deactivate and activate again.\n\nYour receipt comes from Paddle, our reseller, in a separate email. You can see your licences and download eligible versions at https://account.filescape.app.\n\nQuestions? Write to us at https://filescape.app/contact (replies to this email aren't read).\n\nFilescape (filescape.app)",
            row.lic.serial_hint
        ),
        reply_to: None,
    }
}

/// Sent to the new address: confirm the move.
pub fn email_confirm_message(link: &str, old: &str, to: &str) -> Message {
    Message {
        to: to.into(),
        subject: "Confirm your new Filescape email address".into(),
        text: format!(
            "Hello,\n\nSomeone signed in to the Filescape account for {old} asked to register its licences to this address instead. To confirm, open this link and press Confirm. It works once, for 24 hours:\n\n{link}\n\nYour serial numbers and activated computers stay as they are. {old} gets a link to undo the change for 14 days.\n\nIf you didn't ask for this, ignore this email: nothing changes without the link.\n\nQuestions? Write to us at https://filescape.app/contact (replies to this email aren't read).\n\nFilescape (filescape.app)"
        ),
        reply_to: None,
    }
}

/// Sent to the old address once the licences have moved, with the undo link.
pub fn email_changed_message(undo_link: &str, new: &str, count: usize, to: &str) -> Message {
    let what = if count == 1 { "Your Filescape licence is".to_string() } else { format!("Your {count} Filescape licences are") };
    Message {
        to: to.into(),
        subject: "Your Filescape email address was changed".into(),
        text: format!(
            "Hello,\n\n{what} now registered to {new}, as asked from your account at account.filescape.app. Serial numbers and activated computers haven't changed, and future emails about your licence go to {new}.\n\nIf you didn't make this change, undo it with this link. It works for 14 days, and until then the address can't be changed again:\n\n{undo_link}\n\nQuestions? Write to us at https://filescape.app/contact (replies to this email aren't read).\n\nFilescape (filescape.app)"
        ),
        reply_to: None,
    }
}
