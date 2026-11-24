//! Bringing saved sites over from WinSCP and FileZilla, and sending them back out in
//! FileZilla's format (which WinSCP, Cyberduck and others can import too).
//!
//! Passwords are never imported or exported: they're asked for on the first connection, and
//! kept in the system keychain if the user ticks Remember.

use super::netui::SavedSite;
use crate::fsys::remote::{self, Scheme};
use std::path::{Path, PathBuf};

/// What an import found: the sites, and what was left out and why.
#[derive(Debug, Default)]
pub(super) struct Found {
    pub sites: Vec<SavedSite>,
    pub skipped: Vec<String>,
}

fn url(scheme: Scheme, user: &str, host: &str, port: u16, dir: &str) -> String {
    let user = if user.is_empty() { String::new() } else { format!("{user}@") };
    let port = if port == 0 || port == scheme.default_port() { String::new() } else { format!(":{port}") };
    let dir = if dir.is_empty() || dir == "/" { String::new() } else if dir.starts_with('/') { dir.to_string() } else { format!("/{dir}") };
    format!("{}://{user}{host}{port}{dir}", scheme.prefix())
}

fn site(name: &str, group: &str, url: String, key: Option<PathBuf>, local: &str, notes: &str) -> SavedSite {
    SavedSite {
        name: name.to_string(),
        url,
        key,
        group: group.to_string(),
        local_dir: (!local.is_empty()).then(|| PathBuf::from(local)),
        notes: notes.to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------
// FileZilla: sitemanager.xml

/// Where FileZilla keeps its sites on this computer.
pub(super) fn filezilla_file() -> Option<PathBuf> {
    let p = if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("FileZilla").join("sitemanager.xml"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("filezilla").join("sitemanager.xml"))
    }?;
    p.exists().then_some(p)
}

/// FileZilla's remote folder: "1 0 4 home 3 bob" (server type, prefix length, then each name
/// with its length first) is `/home/bob`.
fn filezilla_dir(s: &str) -> String {
    let mut rest = s.trim();
    let next = |rest: &mut &str| -> Option<String> {
        let t = rest.trim_start();
        let end = t.find(' ').unwrap_or(t.len());
        let tok = t[..end].to_string();
        *rest = t.get(end + 1..).unwrap_or("");
        (!tok.is_empty()).then_some(tok)
    };
    let (Some(_kind), Some(prefix)) = (next(&mut rest), next(&mut rest)) else { return String::new() };
    let mut out = String::new();
    if let Ok(n) = prefix.parse::<usize>()
        && n > 0
    {
        rest = rest.get(n.min(rest.len())..).unwrap_or("").trim_start();
    }
    while let Some(len) = next(&mut rest) {
        let Ok(n) = len.parse::<usize>() else { break };
        let name: String = rest.chars().take(n).collect();
        rest = rest.get(name.len()..).unwrap_or("");
        out.push('/');
        out.push_str(&name);
    }
    out
}

pub(super) fn from_filezilla(xml: &str) -> Result<Found, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("This isn't a FileZilla site file: {e}"))?;
    let mut found = Found::default();
    fn walk(node: roxmltree::Node, group: &str, found: &mut Found) {
        for n in node.children().filter(|n| n.is_element()) {
            match n.tag_name().name() {
                "Folder" => {
                    let name: String = n.children().filter(|c| c.is_text()).filter_map(|c| c.text()).collect::<String>().trim().to_string();
                    let g = if group.is_empty() { name } else { format!("{group}/{name}") };
                    walk(n, &g, found);
                }
                "Server" => {
                    let get = |tag: &str| n.children().find(|c| c.has_tag_name(tag)).and_then(|c| c.text()).unwrap_or("").trim().to_string();
                    let name = get("Name");
                    let host = get("Host");
                    let shown = if name.is_empty() { host.clone() } else { name.clone() };
                    let scheme = match get("Protocol").as_str() {
                        "1" => Some(Scheme::Sftp),
                        "3" | "4" => Some(Scheme::Ftps),
                        "0" | "6" | "" => Some(Scheme::Ftp),
                        _ => None,
                    };
                    let Some(scheme) = scheme else {
                        found.skipped.push(format!("{shown}: a kind of server Filescape doesn't connect to"));
                        continue;
                    };
                    if host.is_empty() {
                        found.skipped.push(format!("{shown}: no server address"));
                        continue;
                    }
                    // Implicit FTPS is FTPS on port 990.
                    let port = get("Port").parse().unwrap_or(0);
                    let port = if get("Protocol") == "3" && port == 0 { 990 } else { port };
                    let key = get("Keyfile");
                    let dir = filezilla_dir(&get("RemoteDir"));
                    let mut s = site(&name, group, url(scheme, &get("User"), &host, port, &dir), (!key.is_empty()).then(|| PathBuf::from(key)), &get("LocalDir"), &get("Comments"));
                    s.allow_insecure = false;
                    found.sites.push(s);
                }
                _ => walk(n, group, found),
            }
        }
    }
    walk(doc.root_element(), "", &mut found);
    Ok(found)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn filezilla_dir_out(dir: &str) -> String {
    let names: Vec<&str> = dir.split('/').filter(|n| !n.is_empty()).collect();
    if names.is_empty() {
        return String::new();
    }
    let mut s = "1 0".to_string();
    for n in names {
        s.push_str(&format!(" {} {n}", n.chars().count()));
    }
    s
}

/// The sites as a FileZilla sitemanager.xml (without passwords). Sites FileZilla can't hold
/// (WebDAV) are left out and named in the second value.
pub(super) fn to_filezilla(sites: &[SavedSite]) -> (String, Vec<String>) {
    use std::collections::BTreeMap;
    let mut left_out = Vec::new();
    // Group path -> servers' XML.
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in sites {
        let Some((site, dir)) = s.path().and_then(|p| remote::split(&p)) else {
            left_out.push(s.shown());
            continue;
        };
        let proto = match site.scheme {
            Scheme::Sftp => "1",
            Scheme::Ftp => "0",
            Scheme::Ftps if site.port == 990 => "3",
            Scheme::Ftps => "4",
            Scheme::Dav | Scheme::Davs => {
                left_out.push(s.shown());
                continue;
            }
        };
        let mut x = String::from("<Server>");
        x.push_str(&format!("<Host>{}</Host><Port>{}</Port><Protocol>{proto}</Protocol><Type>0</Type>", xml_escape(&site.host), site.port));
        x.push_str(&format!("<User>{}</User>", xml_escape(&site.user)));
        x.push_str(if site.user.is_empty() { "<Logontype>0</Logontype>" } else if s.key.is_some() { "<Logontype>5</Logontype>" } else { "<Logontype>2</Logontype>" });
        if let Some(k) = &s.key {
            x.push_str(&format!("<Keyfile>{}</Keyfile>", xml_escape(&k.to_string_lossy())));
        }
        x.push_str(&format!("<Name>{}</Name>", xml_escape(&s.shown())));
        x.push_str(&format!("<Comments>{}</Comments>", xml_escape(&s.notes)));
        if let Some(l) = &s.local_dir {
            x.push_str(&format!("<LocalDir>{}</LocalDir>", xml_escape(&l.to_string_lossy())));
        }
        x.push_str(&format!("<RemoteDir>{}</RemoteDir>", xml_escape(&filezilla_dir_out(&dir))));
        x.push_str("</Server>");
        groups.entry(s.group.clone()).or_default().push(x);
    }
    // Nested folders: write each group's path as nested <Folder>s.
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<FileZilla3 version=\"3.66.0\" platform=\"Filescape\">\n<Servers>\n");
    fn emit(out: &mut String, prefix: &str, groups: &BTreeMap<String, Vec<String>>, depth: usize) {
        if let Some(servers) = groups.get(prefix) {
            for x in servers {
                out.push_str(&"  ".repeat(depth));
                out.push_str(x);
                out.push('\n');
            }
        }
        let mut children: Vec<String> = groups
            .keys()
            .filter_map(|g| {
                let rest = if prefix.is_empty() { Some(g.as_str()) } else { g.strip_prefix(prefix).and_then(|r| r.strip_prefix('/')) }?;
                let first = rest.split('/').next()?;
                (!first.is_empty()).then(|| first.to_string())
            })
            .collect();
        children.dedup();
        for c in children {
            let full = if prefix.is_empty() { c.clone() } else { format!("{prefix}/{c}") };
            out.push_str(&"  ".repeat(depth));
            out.push_str(&format!("<Folder expanded=\"1\">{}\n", xml_escape(&c)));
            emit(out, &full, groups, depth + 1);
            out.push_str(&"  ".repeat(depth));
            out.push_str("</Folder>\n");
        }
    }
    // Groups without sites of their own still need their path's folders: emit walks every key.
    emit(&mut out, "", &groups, 1);
    out.push_str("</Servers>\n</FileZilla3>\n");
    (out, left_out)
}

// ---------------------------------------------------------------------------------------------
// WinSCP: WinSCP.ini, or the registry on Windows.

/// WinSCP's own %XX coding of names in session keys.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(h) = std::str::from_utf8(&b[i + 1..i + 3])
            && let Ok(v) = u8::from_str_radix(h, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One WinSCP session's settings, by name.
fn winscp_site(key: &str, v: &std::collections::HashMap<String, String>) -> Result<SavedSite, String> {
    let full = unescape(key);
    let (group, name) = match full.rsplit_once('/') {
        Some((g, n)) => (g.to_string(), n.to_string()),
        None => (String::new(), full.clone()),
    };
    let get = |k: &str| v.get(k).map(|s| unescape(s)).unwrap_or_default();
    let host = get("HostName");
    if host.is_empty() {
        return Err(format!("{name}: no server address"));
    }
    // FSProtocol: 0 SCP, 1 SFTP falling back to SCP, 2 SFTP, 5 FTP, 6 WebDAV, 7 S3. Ftps: 0 none,
    // 1 implicit, 2 or 3 explicit. Missing means SFTP.
    let ftps = get("Ftps");
    let secure = !ftps.is_empty() && ftps != "0";
    let scheme = match get("FSProtocol").as_str() {
        "" | "0" | "1" | "2" => Scheme::Sftp,
        "5" if secure => Scheme::Ftps,
        "5" => Scheme::Ftp,
        "6" if secure => Scheme::Davs,
        "6" => Scheme::Dav,
        _ => return Err(format!("{name}: a kind of server Filescape doesn't connect to (S3)")),
    };
    let port = get("PortNumber").parse().unwrap_or(0);
    let port = if scheme == Scheme::Ftps && ftps == "1" && port == 0 { 990 } else { port };
    let key = get("PublicKeyFile");
    let dir = get("RemoteDirectory");
    Ok(site(&name, &group, url(scheme, &get("UserName"), &host, port, &dir), (!key.is_empty()).then(|| PathBuf::from(key)), &get("LocalDirectory"), &get("Note")))
}

/// WinSCP.ini's `[Sessions\name]` sections.
pub(super) fn from_winscp_ini(text: &str) -> Found {
    let mut found = Found::default();
    let mut cur: Option<(String, std::collections::HashMap<String, String>)> = None;
    let done = |c: Option<(String, std::collections::HashMap<String, String>)>, found: &mut Found| {
        if let Some((k, v)) = c
            && k != "Default%20Settings"
        {
            match winscp_site(&k, &v) {
                Ok(s) => found.sites.push(s),
                Err(e) => found.skipped.push(e),
            }
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            done(cur.take(), &mut found);
            cur = sec.strip_prefix("Sessions\\").map(|k| (k.to_string(), Default::default()));
        } else if let Some((k, v)) = line.split_once('=')
            && let Some((_, map)) = &mut cur
        {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    done(cur.take(), &mut found);
    found
}

/// `reg query … /s` output for WinSCP's sessions key.
pub(super) fn from_winscp_reg(text: &str) -> Found {
    let mut ini = String::new();
    for line in text.lines() {
        let t = line.trim_end();
        if let Some(i) = t.find("\\Sessions\\") {
            ini.push_str(&format!("[Sessions\\{}]\n", &t[i + "\\Sessions\\".len()..]));
        } else if t.starts_with("    ") {
            let parts: Vec<&str> = t.trim().splitn(3, "    ").collect();
            if let [name, _kind, value] = parts.as_slice() {
                // DWORDs come as 0x16.
                let value = value.trim();
                let value = value.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()).map(|n| n.to_string()).unwrap_or_else(|| value.to_string());
                ini.push_str(&format!("{name}={value}\n"));
            } else if let [name, _kind] = parts.as_slice() {
                ini.push_str(&format!("{name}=\n"));
            }
        }
    }
    from_winscp_ini(&ini)
}

/// WinSCP's sites on this computer: its ini file if there is one, else the registry.
pub(super) fn winscp_here() -> Result<Found, String> {
    let ini = [std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("WinSCP.ini")), std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".winscp.ini"))]
        .into_iter()
        .flatten()
        .find(|p| p.exists());
    if let Some(p) = ini {
        return std::fs::read_to_string(&p).map(|t| from_winscp_ini(&t)).map_err(|e| format!("Couldn't read {}: {e}", p.display()));
    }
    if cfg!(windows) {
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Martin Prikryl\WinSCP 2\Sessions", "/s"])
            .output()
            .map_err(|e| format!("Couldn't read WinSCP's settings: {e}"))?;
        if !out.status.success() {
            return Err("WinSCP's sites weren't found on this computer. If WinSCP keeps them in a file, choose From a File… and pick WinSCP.ini.".into());
        }
        return Ok(from_winscp_reg(&String::from_utf8_lossy(&out.stdout)));
    }
    Err("WinSCP's sites weren't found. Choose From a File… and pick a WinSCP.ini (in WinSCP: Tools > Export/Backup Configuration).".into())
}

/// A file picked by the user: FileZilla XML or WinSCP ini, told apart by what's in it.
pub(super) fn from_file(p: &Path) -> Result<Found, String> {
    let text = std::fs::read_to_string(p).map_err(|e| format!("Couldn't read {}: {e}", p.display()))?;
    if text.trim_start().starts_with('<') {
        from_filezilla(&text)
    } else if text.contains("[Sessions\\") {
        Ok(from_winscp_ini(&text))
    } else {
        Err(format!("{} isn't a FileZilla site file or a WinSCP.ini.", p.display()))
    }
}

/// Add what was found, leaving out sites already saved (same address). Returns how many were
/// added.
pub(super) fn merge(into: &mut Vec<SavedSite>, found: Vec<SavedSite>) -> usize {
    let mut n = 0;
    for s in found {
        if into.iter().any(|o| o.url == s.url && o.name == s.name) {
            continue;
        }
        into.push(s);
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_filezilla_sites_and_writes_them_back() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<FileZilla3 version="3.66.0" platform="windows">
  <Servers>
    <Server><Host>render01</Host><Port>2222</Port><Protocol>1</Protocol><Type>0</Type><User>rich</User>
      <Pass encoding="base64">c2VjcmV0</Pass><Logontype>1</Logontype><Name>Render head</Name>
      <RemoteDir>1 0 3 mnt 5 shots</RemoteDir><Comments>Machine room</Comments></Server>
    <Folder expanded="1">Clients
      <Folder expanded="1">Acme
        <Server><Host>ftp.acme.test</Host><Port>990</Port><Protocol>3</Protocol><User>up</User><Name>Acme drop</Name><RemoteDir>1 0 8 my files</RemoteDir></Server>
      </Folder>
      <Server><Host>s3.test</Host><Protocol>5</Protocol><Name>Bucket</Name></Server>
    </Folder>
  </Servers>
</FileZilla3>"#;
        let f = from_filezilla(xml).unwrap();
        assert_eq!(f.sites.len(), 2, "{f:?}");
        assert_eq!(f.sites[0].url, "sftp://rich@render01:2222/mnt/shots");
        assert_eq!(f.sites[0].name, "Render head");
        assert_eq!(f.sites[0].notes, "Machine room");
        assert_eq!(f.sites[1].group, "Clients/Acme");
        assert_eq!(f.sites[1].url, "ftps://up@ftp.acme.test:990/my files");
        assert_eq!(f.skipped.len(), 1);
        // No password comes across.
        assert!(!format!("{:?}", f.sites).contains("secret") && !format!("{:?}", f.sites).contains("c2VjcmV0"));
        for s in &f.sites {
            assert!(s.path().is_some(), "{} doesn't parse", s.url);
        }
        let (out, left_out) = to_filezilla(&f.sites);
        assert!(left_out.is_empty());
        let back = from_filezilla(&out).unwrap();
        assert_eq!(back.sites.iter().map(|s| (&s.url, &s.group, &s.name)).collect::<Vec<_>>(), f.sites.iter().map(|s| (&s.url, &s.group, &s.name)).collect::<Vec<_>>());
    }

    #[test]
    fn reads_winscp_ini_and_registry() {
        let ini = "[Configuration\\Interface]\nTheme=1\n[Sessions\\Default%20Settings]\nHostName=x\n[Sessions\\Render%20Farm/head%20node]\nHostName=render01\nUserName=rich\nPortNumber=2222\nRemoteDirectory=/mnt/shots\nPublicKeyFile=C:%5CUsers%5Crich%5Cid.ppk\n[Sessions\\nas]\nHostName=nas.local\nFSProtocol=5\nFtps=1\n[Sessions\\dav]\nHostName=cloud.test\nFSProtocol=6\nFtps=1\nRemoteDirectory=/remote.php/dav\n[Sessions\\bucket]\nHostName=s3.amazonaws.com\nFSProtocol=7\n";
        let f = from_winscp_ini(ini);
        assert_eq!(f.sites.len(), 3, "{f:?}");
        assert_eq!(f.sites[0].name, "head node");
        assert_eq!(f.sites[0].group, "Render Farm");
        assert_eq!(f.sites[0].url, "sftp://rich@render01:2222/mnt/shots");
        assert_eq!(f.sites[0].key.as_deref(), Some(Path::new("C:\\Users\\rich\\id.ppk")));
        assert_eq!(f.sites[1].url, "ftps://nas.local:990");
        assert!(f.sites[2].url.starts_with("davs://cloud.test"), "{}", f.sites[2].url);
        assert_eq!(f.skipped.len(), 1);
        for s in &f.sites {
            assert!(s.path().is_some(), "{} doesn't parse", s.url);
        }
        let reg = "\r\nHKEY_CURRENT_USER\\Software\\Martin Prikryl\\WinSCP 2\\Sessions\\Render%20Farm/head%20node\r\n    HostName    REG_SZ    render01\r\n    PortNumber    REG_DWORD    0x8ae\r\n    UserName    REG_SZ    rich\r\n\r\n";
        let r = from_winscp_reg(reg);
        assert_eq!(r.sites.len(), 1);
        assert_eq!(r.sites[0].url, "sftp://rich@render01:2222");
        let mut all = f.sites.clone();
        assert_eq!(merge(&mut all, r.sites.clone()), 1);
        assert_eq!(merge(&mut all, r.sites), 0);
    }

    #[test]
    fn filezilla_folders() {
        assert_eq!(filezilla_dir("1 0 4 home 3 bob"), "/home/bob");
        assert_eq!(filezilla_dir("1 0 8 my files"), "/my files");
        assert_eq!(filezilla_dir(""), "");
        assert_eq!(filezilla_dir_out("/home/bob"), "1 0 4 home 3 bob");
    }
}
