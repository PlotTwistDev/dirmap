//! Small helpers for the HTML pages: escaping, forms, layout, dates.

use filescape_licence::Ts;

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// `application/x-www-form-urlencoded`, also used for query strings.
pub fn parse_form(s: &str) -> Vec<(String, String)> {
    s.split('&').filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (decode(k), decode(v))
    }).collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

pub struct Form(pub Vec<(String, String)>);

impl Form {
    pub fn parse(body: &str) -> Form {
        Form(parse_form(body))
    }

    pub fn get(&self, k: &str) -> String {
        self.0.iter().find(|(a, _)| a == k).map(|(_, v)| v.trim().to_string()).unwrap_or_default()
    }

    pub fn has(&self, k: &str) -> bool {
        self.0.iter().any(|(a, v)| a == k && !v.is_empty())
    }
}

pub fn date(t: Ts) -> String {
    filescape_licence::export::date_time(t)
}

pub fn opt_date(t: Option<Ts>) -> String {
    t.map_or_else(|| "—".into(), date)
}

/// The UTC day as `YYYY-MM-DD`, for a date input's value.
pub fn iso_day(t: Ts) -> String {
    let z = t.div_euclid(filescape_licence::DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` (from a date input) as midnight UTC.
pub fn parse_day(s: &str) -> Option<Ts> {
    let mut it = s.trim().split('-');
    let (y, m, d): (i64, i64, i64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * filescape_licence::DAY)
}

const CSS: &str = r#"
:root{--bg:#fff;--fg:#1d1f23;--mute:#5d6470;--line:#e3e6ea;--card:#f6f7f9;--acc:#2f6fdb;--warn:#b54708;--bad:#b42318;--good:#067647}
@media (prefers-color-scheme:dark){:root{--bg:#16181c;--fg:#e8eaed;--mute:#9aa1ab;--line:#2c3036;--card:#1e2126;--acc:#6ea0ff;--warn:#f79009;--bad:#f97066;--good:#47cd89}}
*{box-sizing:border-box}:root{--logo:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20' width='20' height='20'%3E%3Crect x='1' y='1' width='10' height='11' rx='2' fill='%23ff6363'/%3E%3Crect x='12' y='1' width='7' height='6' rx='1.5' fill='%23e6e6e6'/%3E%3Crect x='12' y='8' width='7' height='4' rx='1.2' fill='%2363a1ff'/%3E%3Crect x='1' y='13' width='6' height='6' rx='1.5' fill='%23e6e6e6'/%3E%3Crect x='8' y='13' width='5' height='6' rx='1.2' fill='%236a6b6c'/%3E%3Crect x='14' y='13' width='5' height='6' rx='1.2' fill='%23ff6363' opacity='.55'/%3E%3C/svg%3E")}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}
header{border-bottom:1px solid var(--line);padding:12px 20px;display:flex;gap:18px;align-items:center;flex-wrap:wrap}
header b{font-size:16px;display:inline-flex;align-items:center;gap:8px}header b::before{content:"";width:18px;height:18px;background:var(--logo) center/contain no-repeat}header a{color:var(--mute);text-decoration:none}header a:hover{color:var(--fg)}
main{max-width:1100px;margin:0 auto;padding:20px}
h1{font-size:22px;margin:4px 0 16px}h2{font-size:17px;margin:26px 0 10px}
a{color:var(--acc)}table{border-collapse:collapse;width:100%}th,td{text-align:left;padding:7px 8px;border-bottom:1px solid var(--line);vertical-align:top}
th{color:var(--mute);font-weight:500;font-size:13px}
.card{background:var(--card);border:1px solid var(--line);border-radius:8px;padding:14px 16px;margin:12px 0}
.row{display:flex;gap:12px;flex-wrap:wrap;align-items:flex-end}
label{display:flex;flex-direction:column;gap:4px;font-size:13px;color:var(--mute)}
input,select,textarea{max-width:100%;font:inherit;padding:6px 8px;border:1px solid var(--line);border-radius:6px;background:var(--bg);color:var(--fg)}
button{font:inherit;padding:6px 12px;border-radius:6px;border:1px solid var(--line);background:var(--card);color:var(--fg);cursor:pointer}
button.primary{background:var(--acc);border-color:var(--acc);color:#fff}button.danger{color:var(--bad)}
.mute{color:var(--mute)}.warn{color:var(--warn)}.bad{color:var(--bad)}.good{color:var(--good)}
.pill{display:inline-block;padding:1px 8px;border-radius:999px;font-size:12px;border:1px solid var(--line)}
code,.mono{font-family:ui-monospace,SFMono-Regular,Consolas,monospace}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:12px}.stats div{background:var(--card);border:1px solid var(--line);border-radius:8px;padding:10px 14px}.stats b{display:block;font-size:22px}
.scroll{overflow-x:auto}h3{font-size:15px;margin:20px 0 6px}
.tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:12px;margin:12px 0}.tiles>div{background:var(--bg);border:1px solid var(--line);border-radius:8px;padding:10px 14px}
.tiles>div>span{display:block;font-size:13px;color:var(--mute)}.tiles b{display:block;font-size:20px;margin:2px 0}.tiles small{color:var(--mute);font-size:13px}
.bar{height:6px;border-radius:3px;background:var(--line);margin-top:8px;overflow:hidden}.bar i{display:block;height:100%;background:var(--good)}
.pill.good{color:var(--good)}.pill.bad{color:var(--bad)}
@media (max-width:640px){main{padding:14px 16px}th,td{padding:6px 4px}}
"#;

pub fn page(title: &str, nav: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><link rel=\"icon\" href=\"https://filescape.app/favicon.svg\" type=\"image/svg+xml\"><title>{}</title><style>{CSS}</style></head><body><header>{nav}</header><main>{body}</main></body></html>",
        esc(title)
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn forms_and_days() {
        let f = super::Form::parse("a=1+2&b=%41%2Fx&c=&d=%zz");
        assert_eq!(f.get("a"), "1 2");
        assert_eq!(f.get("b"), "A/x");
        assert_eq!(f.get("d"), "%zz");
        assert!(!f.has("c"));
        assert_eq!(super::parse_day("1970-01-02"), Some(86_400));
        assert_eq!(super::parse_day("2026-11-23"), Some(1_795_392_000));
        assert_eq!(super::esc("<a href=\"x\">"), "&lt;a href=&quot;x&quot;&gt;");
    }
}

/// Whether a form POST came from `host` itself (cross-site request protection), from the browser's
/// own `Sec-Fetch-Site` and `Origin` headers, which pages can't set.
///
/// Browsers send `Origin: null` for a form posted from a page whose Referrer-Policy is
/// `no-referrer`, even to the same site, so `null` is accepted when `Sec-Fetch-Site` says
/// same-origin (a browser that sends one sends both).
pub fn same_origin_post(req: &crate::Req, host: &str, dev: bool) -> bool {
    let sec = req.header("sec-fetch-site");
    if sec.is_some_and(|s| s != "same-origin" && s != "none") {
        return false;
    }
    match req.header("origin").map(|o| o.trim_end_matches('/')) {
        None => true,
        Some("null") => sec.is_some(),
        Some(o) => o == format!("https://{host}") || (dev && (o.starts_with("http://localhost") || o.starts_with("http://127.0.0.1"))),
    }
}
