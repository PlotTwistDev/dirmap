//! api.filescape.app: what the app calls, and the Paddle webhook.
//!
//! Errors the app should show are `{"error": "<rejection>"}` with a 4xx status (the app turns the
//! rejection into its own message).

use crate::store::Fail;
use crate::{Ctx, Db, Net, Req, Resp};
use filescape_licence::model::{ActivationRequest, OFFLINE_REQUEST_FORMAT, OfflineRequest, Rejection};
use filescape_licence::sign::Purpose;
use serde::Deserialize;

#[derive(Deserialize)]
struct ByActivation {
    activation_id: String,
    device_hash: String,
}

fn rejected(r: Rejection) -> Resp {
    let status = match r {
        Rejection::BadSerial => 400,
        Rejection::UnknownSerial | Rejection::NotFound => 404,
        Rejection::ActivationIdConflict => 409,
        _ => 403,
    };
    Resp::json(status, &serde_json::json!({ "error": r, "message": r.message() }))
}

fn answer<T: serde::Serialize>(r: Result<T, Fail>) -> Result<Resp, String> {
    match r {
        Ok(v) => Ok(Resp::json(200, &v)),
        Err(Fail::No(r)) => Ok(rejected(r)),
        Err(Fail::Signed(r, env)) => {
            let mut resp = rejected(r);
            resp.body = serde_json::to_string(&serde_json::json!({ "error": r, "message": r.message(), "status": env })).unwrap();
            Ok(resp)
        }
        Err(Fail::Err(e)) => Err(e),
    }
}

fn body<T: serde::de::DeserializeOwned>(req: &Req) -> Result<T, Resp> {
    if req.body.len() > 16 * 1024 {
        return Err(Resp::text(413, "Too large."));
    }
    serde_json::from_slice(&req.body).map_err(|_| rejected(Rejection::BadSerial))
}

/// An offline request file (as the app saves it), or a bare request.
pub fn parse_offline(text: &str) -> Option<ActivationRequest> {
    if let Ok(f) = serde_json::from_str::<OfflineRequest>(text) {
        return (f.format == OFFLINE_REQUEST_FORMAT).then_some(f.request);
    }
    serde_json::from_str::<ActivationRequest>(text).ok()
}

pub async fn route<D: Db, N: Net>(cx: &Ctx<'_, D, N>, req: &Req, path: &str) -> Result<Resp, String> {
    match (req.method.as_str(), path) {
        ("GET", "/" | "/v1" | "/v1/health") => Ok(Resp::json(200, &serde_json::json!({ "ok": true, "service": "filescape-licence", "time": cx.now, "key_id": cx.cfg.signer.key_id(), "public_key": filescape_licence::sign::b64(&cx.cfg.signer.public_key()) }))),
        ("POST", "/v1/activate") => match body::<ActivationRequest>(req) {
            Ok(r) => answer(cx.activate(&r).await),
            Err(resp) => Ok(resp),
        },
        ("POST", "/v1/offline") => match parse_offline(&req.text()) {
            Some(r) => answer(cx.activate(&r).await),
            None => Ok(rejected(Rejection::BadSerial)),
        },
        ("POST", "/v1/validate") => match body::<ByActivation>(req) {
            Ok(b) => answer(cx.validate(&b.activation_id, &b.device_hash).await),
            Err(resp) => Ok(resp),
        },
        ("POST", "/v1/deactivate") => match body::<ByActivation>(req) {
            Ok(b) => answer(cx.deactivate(&b.activation_id, &b.device_hash).await.map(|()| serde_json::json!({ "ok": true }))),
            Err(resp) => Ok(resp),
        },
        ("POST", "/v1/paddle") => crate::paddle::webhook(cx, req).await,
        // The app's update check at launch. A plain GET: nothing about the user or the computer
        // is sent, and the answer is the same for everyone, so caches may keep it a few minutes.
        ("GET", "/v1/release") => {
            let mut resp = match cx.latest_release().await? {
                Some(m) => Resp::json(200, &serde_json::json!({ "release": cx.cfg.signer.sign(Purpose::Release, &m) })),
                None => Resp::json(404, &serde_json::json!({ "error": "no_release" })),
            };
            resp.headers.retain(|(k, _)| k != "cache-control");
            resp.headers.push(("cache-control".into(), "public, max-age=300".into()));
            Ok(resp)
        }
        (_, "/v1/activate" | "/v1/offline" | "/v1/validate" | "/v1/deactivate" | "/v1/paddle" | "/v1/release") => Ok(Resp::text(405, "Method not allowed.")),
        _ => Ok(Resp::text(404, "Not found.")),
    }
}
