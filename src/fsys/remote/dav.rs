//! WebDAV (Nextcloud, ownCloud, a NAS, SharePoint-style servers), over https checked against
//! the system's trusted certificates, or plain http. Signing in uses HTTP Basic, which over
//! plain http is readable on the network; the connection manager says so.

use super::{Backend, Meta, Scheme, Site, err, password_for};
use std::io::{self, Read, Write};
use std::sync::Mutex;
use std::time::Duration;
use ureq::http::{Method, Request, Response};

pub struct Dav {
    agent: ureq::Agent,
    base: String,
    /// The Authorization header, once a password was needed. Kept in memory only.
    auth: Mutex<Option<String>>,
    site: Site,
}

/// Keep a path's characters that are safe in a URL; encode the rest.
fn encode(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What a request sends.
enum Body<'a> {
    None,
    /// Small, and sent again after signing in.
    Bytes(&'a [u8]),
    /// An upload, sent once.
    Stream(&'a mut dyn Read),
}

const PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

fn status_err(r: &Response<ureq::Body>, what: &str) -> io::Error {
    let s = r.status().as_u16();
    let kind = match s {
        404 | 410 => io::ErrorKind::NotFound,
        401 | 403 => io::ErrorKind::PermissionDenied,
        405 | 409 | 412 => io::ErrorKind::AlreadyExists,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("The server answered {s} {} to {what}.", r.status().canonical_reason().unwrap_or("")))
}

fn net_err(e: ureq::Error) -> io::Error {
    match e {
        ureq::Error::Io(e) => e,
        ureq::Error::Timeout(_) => io::Error::new(io::ErrorKind::TimedOut, "The server didn't answer in time."),
        e => err(e),
    }
}

/// Read a PROPFIND answer: (path on the server, decoded; meta) for each item.
fn parse_multistatus(xml: &str) -> io::Result<Vec<(String, Meta)>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| io::Error::other(format!("The server's answer couldn't be read: {e}")))?;
    let dav = |n: &roxmltree::Node, name: &str| n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some("DAV:");
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| dav(n, "response")) {
        let Some(href) = resp.children().find(|n| dav(n, "href")).and_then(|n| n.text()) else { continue };
        let mut m = Meta::default();
        for ps in resp.children().filter(|n| dav(n, "propstat")) {
            let ok = ps.children().find(|n| dav(n, "status")).and_then(|n| n.text()).is_none_or(|s| s.contains(" 200"));
            if !ok {
                continue;
            }
            for prop in ps.descendants().filter(|n| n.is_element()) {
                if dav(&prop, "collection") {
                    m.is_dir = true;
                } else if dav(&prop, "getcontentlength") {
                    m.len = prop.text().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
                } else if dav(&prop, "getlastmodified") {
                    m.modified = prop.text().and_then(|t| chrono::DateTime::parse_from_rfc2822(t.trim()).ok()).map(|t| t.timestamp()).unwrap_or(0);
                }
            }
        }
        // An absolute href: only its path.
        let path = match href.split_once("://") {
            Some((_, rest)) => rest.find('/').map(|i| &rest[i..]).unwrap_or("/"),
            None => href,
        };
        out.push((super::percent_decode(path), m));
    }
    Ok(out)
}

impl Dav {
    pub fn connect(site: &Site) -> io::Result<Dav> {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .allow_non_standard_methods(true)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .tls_config(tls)
            .build()
            .into();
        let scheme = if site.scheme == Scheme::Davs { "https" } else { "http" };
        let port = if site.port == site.scheme.default_port() { String::new() } else { format!(":{}", site.port) };
        let d = Dav { agent, base: format!("{scheme}://{}{port}", site.host), auth: Mutex::new(None), site: site.clone() };
        // Check the server answers (and sign in, if it wants that) before saying connected.
        d.stat("/").map(drop).or_else(|e| match e.kind() {
            // The top may be off limits while the folders below aren't (Nextcloud).
            io::ErrorKind::NotFound | io::ErrorKind::AlreadyExists => Ok(()),
            _ if e.to_string().contains("certificate") => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{}'s certificate isn't one your system trusts ({e}). If it's your own server, add its certificate to the system's trusted certificates.", site.host),
            )),
            _ => Err(e),
        })?;
        Ok(d)
    }

    fn url(&self, p: &str) -> String {
        format!("{}{}", self.base, encode(p))
    }

    /// Send a request, signing in when the server asks.
    fn send(&self, method: &str, p: &str, headers: &[(&str, String)], body: Body) -> io::Result<Response<ureq::Body>> {
        let method = Method::from_bytes(method.as_bytes()).map_err(io::Error::other)?;
        let mut body = body;
        let mut tried: Option<String> = None;
        loop {
            let mut b = Request::builder().method(method.clone()).uri(self.url(p));
            for (k, v) in headers {
                b = b.header(*k, v);
            }
            if let Some(a) = self.auth.lock().unwrap().as_ref() {
                b = b.header("Authorization", a);
            }
            let streamed = matches!(body, Body::Stream(_));
            let r = match std::mem::replace(&mut body, Body::None) {
                Body::Stream(r) => self.agent.run(b.body(ureq::SendBody::from_reader(r)).map_err(io::Error::other)?),
                Body::Bytes(bytes) => {
                    body = Body::Bytes(bytes);
                    self.agent.run(b.body(bytes).map_err(io::Error::other)?)
                }
                Body::None => self.agent.run(b.body(()).map_err(io::Error::other)?),
            }
            .map_err(net_err)?;
            // Anonymous, or an upload whose data is spent: the refusal is the answer.
            if r.status().as_u16() != 401 || self.site.user.is_empty() || streamed {
                return Ok(r);
            }
            let pw = password_for(&self.site, tried.as_deref()).ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Not connected: no password was given."))?;
            *self.auth.lock().unwrap() = Some(format!("Basic {}", base64(format!("{}:{pw}", self.site.user).as_bytes())));
            tried = Some(pw);
        }
    }

    fn propfind(&self, p: &str, depth: &str) -> io::Result<Vec<(String, Meta)>> {
        let mut r = self.send("PROPFIND", p, &[("Depth", depth.into()), ("Content-Type", "application/xml".into())], Body::Bytes(PROPFIND.as_bytes()))?;
        if r.status().as_u16() != 207 {
            return Err(status_err(&r, "reading a folder"));
        }
        let xml = r.body_mut().with_config().limit(256 << 20).read_to_string().map_err(net_err)?;
        parse_multistatus(&xml)
    }

    fn check(&self, r: Response<ureq::Body>, what: &str) -> io::Result<()> {
        if r.status().is_success() { Ok(()) } else { Err(status_err(&r, what)) }
    }
}

impl Backend for Dav {
    fn stat(&self, p: &str) -> io::Result<Meta> {
        let items = self.propfind(p, "0")?;
        let mut m = items.into_iter().next().map(|(_, m)| m).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))?;
        if p == "/" {
            m.is_dir = true;
        }
        Ok(m)
    }

    fn list(&self, p: &str) -> io::Result<Vec<(String, Meta)>> {
        let items = self.propfind(p, "1")?;
        let me = p.trim_end_matches('/');
        Ok(items
            .into_iter()
            .filter_map(|(href, m)| {
                let h = href.trim_end_matches('/');
                if h == me || h.is_empty() {
                    return None;
                }
                let name = h.rsplit('/').next()?.to_string();
                Some((name, m))
            })
            .collect())
    }

    fn mkdir(&self, p: &str) -> io::Result<()> {
        let r = self.send("MKCOL", p, &[], Body::None)?;
        self.check(r, "making a folder")
    }

    fn rmdir(&self, p: &str) -> io::Result<()> {
        let r = self.send("DELETE", &format!("{}/", p.trim_end_matches('/')), &[], Body::None)?;
        self.check(r, "removing a folder")
    }

    fn rm(&self, p: &str) -> io::Result<()> {
        let r = self.send("DELETE", p, &[], Body::None)?;
        self.check(r, "deleting")
    }

    fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        let r = self.send("MOVE", from, &[("Destination", self.url(to)), ("Overwrite", "F".into())], Body::None)?;
        self.check(r, "moving")
    }

    fn get(&self, p: &str, out: &mut dyn Write, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        self.get_from(p, 0, out, tick)
    }

    fn get_from(&self, p: &str, offset: u64, out: &mut dyn Write, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        let range = if offset > 0 { vec![("Range", format!("bytes={offset}-"))] } else { Vec::new() };
        let mut r = self.send("GET", p, &range, Body::None)?;
        if !r.status().is_success() {
            return Err(status_err(&r, "downloading"));
        }
        if offset > 0 && r.status().as_u16() != 206 {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "The server can't continue a download part-way."));
        }
        let mut rd = r.body_mut().with_config().limit(u64::MAX).reader();
        let mut buf = vec![0u8; 256 << 10];
        loop {
            let n = rd.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            out.write_all(&buf[..n])?;
            if !tick(n as u64) {
                return Err(super::stopped());
            }
        }
    }

    fn put(&self, p: &str, input: &mut dyn Read, len: u64, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        // Sign in first with a small request, so the upload isn't spent on a refusal.
        if self.auth.lock().unwrap().is_none() {
            let parent = p.rsplit_once('/').map(|(a, _)| if a.is_empty() { "/" } else { a }).unwrap_or("/");
            let _ = self.propfind(parent, "0");
        }
        struct Counting<'a> {
            inner: &'a mut dyn Read,
            tick: &'a mut dyn FnMut(u64) -> bool,
        }
        impl Read for Counting<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.inner.read(buf)?;
                if n > 0 && !(self.tick)(n as u64) {
                    return Err(super::stopped());
                }
                Ok(n)
            }
        }
        let mut c = Counting { inner: input, tick };
        let r = self.send("PUT", p, &[("Content-Length", len.to_string())], Body::Stream(&mut c))?;
        self.check(r, "uploading")
    }

    fn home(&self) -> io::Result<String> {
        Ok("/".into())
    }

    fn alive(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_folder_listing() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
 <d:response><d:href>/dav/shots/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
 <d:response><d:href>https://h/dav/shots/A%20001.exr</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>1234</d:getcontentlength><d:getlastmodified>Tue, 17 Nov 2026 10:00:00 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
</d:multistatus>"#;
        let items = parse_multistatus(xml).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].1.is_dir);
        assert_eq!(items[1].0, "/dav/shots/A 001.exr");
        assert_eq!(items[1].1.len, 1234);
        assert!(items[1].1.modified > 0);
        assert_eq!(base64(b"rich:pw"), "cmljaDpwdw==");
        assert_eq!(encode("/a b/é"), "/a%20b/%C3%A9");
    }
}
