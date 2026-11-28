//! The admin site's door. Cloudflare Access sits in front of admin.filescape.app and signs in
//! the admin; the Worker still checks Access's token itself (`Cf-Access-Jwt-Assertion`, RS256,
//! against the team's published keys, with the application's audience tag), so a request that
//! somehow bypasses Access is refused anyway. Form posts must also come from the admin site itself.

use crate::{Ctx, Db, Net, Req};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde_json::Value;

fn b64(s: &str) -> Result<Vec<u8>, &'static str> {
    data_encoding::BASE64URL_NOPAD.decode(s.trim_end_matches('=').as_bytes()).map_err(|_| "bad token encoding")
}

/// The signed-in admin's email, or why not.
pub async fn check<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req) -> Result<String, String> {
    if req.method != "GET" && req.method != "HEAD" {
        if !crate::web::same_origin_post(req, &cx.cfg.hosts.admin, cx.cfg.dev) {
            return Err("form posted from another site".into());
        }
    }
    let Some(ac) = &cx.cfg.access else {
        return if cx.cfg.dev { Ok("dev".into()) } else { Err("Cloudflare Access isn't set up (ACCESS_TEAM_DOMAIN, ACCESS_AUD)".into()) };
    };
    let token = req.header("cf-access-jwt-assertion").ok_or("no Cloudflare Access token")?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("malformed token".into());
    }
    let header: Value = serde_json::from_slice(&b64(parts[0])?).map_err(|_| "bad token header")?;
    let payload: Value = serde_json::from_slice(&b64(parts[1])?).map_err(|_| "bad token payload")?;
    let sig = b64(parts[2])?;
    if header["alg"] != "RS256" {
        return Err("unexpected token algorithm".into());
    }
    let kid = header["kid"].as_str().unwrap_or_default();
    let (status, body) = cx.net.send("GET", &format!("https://{}/cdn-cgi/access/certs", ac.team_domain), &[], None).await?;
    if status != 200 {
        return Err(format!("couldn't fetch Access keys ({status})"));
    }
    let keys: Value = serde_json::from_str(&body).map_err(|_| "bad Access keys")?;
    let key = keys["keys"].as_array().and_then(|ks| ks.iter().find(|k| k["kid"] == kid)).ok_or("token signed with an unknown key")?;
    let n = b64(key["n"].as_str().unwrap_or_default())?;
    let e = b64(key["e"].as_str().unwrap_or_default())?;
    let msg = format!("{}.{}", parts[0], parts[1]);
    RsaPublicKeyComponents { n: &n, e: &e }.verify(&RSA_PKCS1_2048_8192_SHA256, msg.as_bytes(), &sig).map_err(|_| "bad token signature")?;
    let aud_ok = match &payload["aud"] {
        Value::String(a) => *a == ac.audience,
        Value::Array(a) => a.iter().any(|x| *x == ac.audience.as_str()),
        _ => false,
    };
    if !aud_ok {
        return Err("token is for another application".into());
    }
    if payload["iss"].as_str() != Some(&format!("https://{}", ac.team_domain)) {
        return Err("token from another Access team".into());
    }
    if payload["exp"].as_i64().is_none_or(|exp| exp < cx.now) {
        return Err("token expired".into());
    }
    Ok(payload["email"].as_str().unwrap_or("admin").to_string())
}
