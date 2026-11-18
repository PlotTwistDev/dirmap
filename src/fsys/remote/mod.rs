//! Folders on servers: SFTP, FTP (and FTPS) and WebDAV, browsed and changed as if they were on
//! disk.
//!
//! A remote path is the server's address followed by the path there, for example
//! `sftp://rich@render01/home/rich/shots`. `PathBuf` keeps it (as `sftp:/rich@render01/…`, which
//! is relative on every system, so local file calls never touch it by accident); `split` reads it
//! back. Every function here can wait on the network, so call them from worker threads only.
//!
//! One connection per server is kept open and shared. Questions for the user (an unknown server
//! key, a password) go through `set_asker`, which the UI answers in a window while the worker
//! waits. Passwords are never written to dirmap's own files: they are kept only in the system
//! keychain, and only when the user ticks "Remember".

mod dav;
mod ftp;
mod sftp;

use super::listing::Entry;
use crate::transfer::Ctl;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The kinds of server dirmap can talk to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Scheme {
    Sftp,
    Ftp,
    Ftps,
    Dav,
    Davs,
}

impl Scheme {
    pub const ALL: [Scheme; 5] = [Scheme::Sftp, Scheme::Ftp, Scheme::Ftps, Scheme::Dav, Scheme::Davs];

    pub fn prefix(self) -> &'static str {
        match self {
            Scheme::Sftp => "sftp",
            Scheme::Ftp => "ftp",
            Scheme::Ftps => "ftps",
            Scheme::Dav => "dav",
            Scheme::Davs => "davs",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Scheme::Sftp => "SFTP (SSH)",
            Scheme::Ftp => "FTP (not encrypted)",
            Scheme::Ftps => "FTPS (FTP over TLS)",
            Scheme::Dav => "WebDAV (http, not encrypted)",
            Scheme::Davs => "WebDAV (https)",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Sftp => 22,
            Scheme::Ftp | Scheme::Ftps => 21,
            Scheme::Dav => 80,
            Scheme::Davs => 443,
        }
    }

    /// Sends passwords and files unencrypted.
    pub fn insecure(self) -> bool {
        matches!(self, Scheme::Ftp | Scheme::Dav)
    }

    fn from_prefix(s: &str) -> Option<Scheme> {
        Some(match s.to_ascii_lowercase().as_str() {
            "sftp" | "ssh" | "scp" => Scheme::Sftp,
            "ftp" => Scheme::Ftp,
            "ftps" | "ftpes" => Scheme::Ftps,
            "dav" | "webdav" | "http" => Scheme::Dav,
            "davs" | "webdavs" | "https" => Scheme::Davs,
            _ => return None,
        })
    }
}

/// A server and the account on it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Site {
    pub scheme: Scheme,
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl Site {
    /// `sftp://rich@render01` (the port only when it isn't the usual one).
    pub fn url(&self) -> String {
        let user = if self.user.is_empty() { String::new() } else { format!("{}@", self.user) };
        let port = if self.port == self.scheme.default_port() { String::new() } else { format!(":{}", self.port) };
        format!("{}://{user}{}{port}", self.scheme.prefix(), self.host)
    }

    /// `rich@render01`, for messages.
    pub fn short(&self) -> String {
        if self.user.is_empty() { self.host.clone() } else { format!("{}@{}", self.user, self.host) }
    }

    /// The keychain account a password for this site is kept under.
    fn account(&self) -> String {
        format!("{}://{}@{}:{}", self.scheme.prefix(), self.user, self.host, self.port)
    }
}

/// The user's name on this computer, used when an SFTP address names no user (as ssh does).
pub fn local_user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default()
}

/// Parse `user@host:port` for a scheme.
fn parse_authority(scheme: Scheme, auth: &str) -> Option<Site> {
    let (user, hostport) = match auth.rsplit_once('@') {
        Some((u, h)) => (u.to_string(), h),
        None => (if scheme == Scheme::Sftp { local_user() } else { String::new() }, auth),
    };
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        // [::1]:22
        let (h, after) = rest.split_once(']')?;
        let port = after.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(scheme.default_port()))?;
        (format!("[{h}]"), port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().ok()?),
            _ => (hostport.to_string(), scheme.default_port()),
        }
    };
    if host.is_empty() || host.contains(['/', '\\', ' ']) {
        return None;
    }
    Some(Site { scheme, user, host: host.to_ascii_lowercase(), port })
}

/// For a remote path: the server and the path on it (always starting with '/'). Decided by the
/// first two parts of the path, so it works with either kind of slash.
pub fn split(path: &Path) -> Option<(Site, String)> {
    let mut comps = path.components();
    let Some(Component::Normal(first)) = comps.next() else { return None };
    let first = first.to_str()?;
    let scheme = Scheme::from_prefix(first.strip_suffix(':')?)?;
    let Some(Component::Normal(auth)) = comps.next() else { return None };
    let site = parse_authority(scheme, auth.to_str()?)?;
    let mut rest = String::new();
    for c in comps {
        if let Component::Normal(n) = c {
            rest.push('/');
            rest.push_str(&n.to_string_lossy());
        }
    }
    if rest.is_empty() {
        rest.push('/');
    }
    Some((site, rest))
}

pub fn is_remote(path: &Path) -> bool {
    split(path).is_some()
}

/// The path for a place on a server.
pub fn path_of(site: &Site, remote: &str) -> PathBuf {
    let r = remote.trim_end_matches('/');
    let r = if r.is_empty() || r.starts_with('/') { r.to_string() } else { format!("/{r}") };
    PathBuf::from(format!("{}{}", site.url(), if r.is_empty() { "/" } else { &r }))
}

/// A remote path as an address to show and type: `sftp://rich@render01/home/rich`.
pub fn show(path: &Path) -> Option<String> {
    let (site, p) = split(path)?;
    Some(format!("{}{}", site.url(), p))
}

/// The server's top folder (`sftp://rich@render01/`): it has no parent.
pub fn is_root(path: &Path) -> bool {
    split(path).is_some_and(|(_, p)| p == "/")
}

/// The folder above, staying on the server.
pub fn parent(path: &Path) -> Option<PathBuf> {
    let (site, p) = split(path)?;
    if p == "/" {
        return None;
    }
    let up = p.rsplit_once('/').map(|(a, _)| a).unwrap_or("");
    Some(path_of(&site, up))
}

/// Read a typed address (`sftp://host/dir`, `ftp://user@host`, `https://host/dav`) as a remote
/// path. `None` if it isn't one.
pub fn parse(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    let (scheme, rest) = s.split_once("://")?;
    let scheme = Scheme::from_prefix(scheme)?;
    let (auth, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let site = parse_authority(scheme, auth)?;
    let path = percent_decode(path);
    Some(path_of(&site, &path))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
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

/// Items and what the server says about each: (name or relative path, meta).
pub type Listed = Vec<(String, Meta)>;

/// What a server says about one item.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Meta {
    pub is_dir: bool,
    pub is_link: bool,
    pub len: u64,
    /// Seconds since the Unix epoch, 0 when unknown.
    pub modified: i64,
}

impl Meta {
    pub fn modified_time(&self) -> Option<SystemTime> {
        (self.modified > 0).then(|| UNIX_EPOCH + Duration::from_secs(self.modified as u64))
    }
}

/// One way of talking to a server. Paths are absolute paths there, with '/'.
trait Backend: Send + Sync {
    fn stat(&self, p: &str) -> io::Result<Meta>;
    fn list(&self, p: &str) -> io::Result<Vec<(String, Meta)>>;
    fn mkdir(&self, p: &str) -> io::Result<()>;
    fn rmdir(&self, p: &str) -> io::Result<()>;
    fn rm(&self, p: &str) -> io::Result<()>;
    fn rename(&self, from: &str, to: &str) -> io::Result<()>;
    /// Download, calling `tick` with each piece's size; it returns false to stop.
    fn get(&self, p: &str, out: &mut dyn Write, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()>;
    /// Upload (replacing what's there).
    fn put(&self, p: &str, input: &mut dyn Read, len: u64, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()>;
    /// Set the modified time, where the server allows it.
    fn touch(&self, _p: &str, _modified: i64) -> io::Result<()> {
        Ok(())
    }
    /// The folder the account starts in.
    fn home(&self) -> io::Result<String>;
    /// Still connected.
    fn alive(&self) -> bool;
}

// ---------------------------------------------------------------------------------------------
// Questions for the user.

/// What a worker needs the user to decide.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// The first connection to an SSH server: is this its key?
    NewHostKey { site: String, algorithm: String, fingerprint: String },
    /// The server's key isn't the one seen before. Never connected; this only explains why.
    ChangedHostKey { site: String, algorithm: String, fingerprint: String, known_hosts: String },
    /// A password for the account (`retry` after a wrong one).
    Password { site: String, retry: bool },
    /// The passphrase for an SSH key file.
    Passphrase { key: String, retry: bool },
}

/// The user's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// Go ahead (a server's key): once, or remember it.
    Trust { always: bool },
    /// A password or passphrase, and whether to keep it in the system keychain.
    Secret { text: String, remember: bool },
    Cancel,
}

type Asker = Box<dyn Fn(Ask) -> Answer + Send + Sync>;

fn asker() -> &'static Mutex<Option<Arc<Asker>>> {
    static A: OnceLock<Mutex<Option<Arc<Asker>>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(None))
}

/// Who answers questions (the UI). Without one, every question is answered Cancel.
pub fn set_asker(f: impl Fn(Ask) -> Answer + Send + Sync + 'static) {
    *asker().lock().unwrap() = Some(Arc::new(Box::new(f)));
}

/// Ask the user, waiting for the answer. Only one question at a time.
fn ask(q: Ask) -> Answer {
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let f = asker().lock().unwrap().clone();
    match f {
        Some(f) => f(q),
        None => Answer::Cancel,
    }
}

// ---------------------------------------------------------------------------------------------
// The system keychain (Windows Credential Manager, macOS Keychain, the Secret Service on Linux).

const KEYCHAIN_SERVICE: &str = "dirmap";

/// Run off any async runtime: the keychain libraries may start their own.
fn off_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("keychain thread"))
}

pub fn keychain_get(account: &str) -> Option<String> {
    let account = account.to_string();
    off_runtime(move || keyring::Entry::new(KEYCHAIN_SERVICE, &account).ok()?.get_password().ok())
}

pub fn keychain_set(account: &str, secret: &str) -> Result<(), String> {
    let (account, secret) = (account.to_string(), secret.to_string());
    off_runtime(move || {
        keyring::Entry::new(KEYCHAIN_SERVICE, &account).and_then(|e| e.set_password(&secret)).map_err(|e| format!("The system keychain didn't take it: {e}"))
    })
}

pub fn keychain_forget(account: &str) {
    let account = account.to_string();
    off_runtime(move || {
        if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, &account) {
            let _ = e.delete_credential();
        }
    })
}

/// Forget the saved password for a server address, if there is one.
pub fn forget_password(url: &str) {
    if let Some((site, _)) = parse(url).as_deref().and_then(split) {
        keychain_forget(&site.account());
    }
}

/// The password for a site: the keychain's, else the user's (kept if they say so). `tried`
/// is the one that just failed, so it isn't offered again.
fn password_for(site: &Site, tried: Option<&str>) -> Option<String> {
    if tried.is_none()
        && let Some(p) = keychain_get(&site.account())
    {
        return Some(p);
    }
    if tried.is_some() {
        // A wrong saved password is forgotten.
        if keychain_get(&site.account()).as_deref() == tried {
            keychain_forget(&site.account());
        }
    }
    match ask(Ask::Password { site: site.url(), retry: tried.is_some() }) {
        Answer::Secret { text, remember } => {
            if remember {
                let _ = keychain_set(&site.account(), &text);
            }
            Some(text)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Connections.

/// Settings for a site that aren't in its address.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    /// An SSH key file to try first.
    pub key: Option<PathBuf>,
}

fn options() -> &'static Mutex<HashMap<String, Options>> {
    static O: OnceLock<Mutex<HashMap<String, Options>>> = OnceLock::new();
    O.get_or_init(Default::default)
}

/// Tell connections to this address (`sftp://rich@render01`) what else to use.
pub fn set_options(url: &str, o: Options) {
    if let Some((site, _)) = parse(url).as_deref().and_then(split) {
        options().lock().unwrap().insert(site.url(), o);
    }
}

type Slot = Arc<Mutex<Option<Arc<dyn Backend>>>>;

fn pool() -> &'static Mutex<HashMap<Site, Slot>> {
    static P: OnceLock<Mutex<HashMap<Site, Slot>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// The open connection to a site, connecting (and asking for what's needed) first if there
/// isn't one. Connections to one site are made one at a time.
fn conn(site: &Site) -> io::Result<Arc<dyn Backend>> {
    let slot = pool().lock().unwrap().entry(site.clone()).or_default().clone();
    let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = s.as_ref()
        && c.alive()
    {
        return Ok(c.clone());
    }
    *s = None;
    let opts = options().lock().unwrap().get(&site.url()).cloned().unwrap_or_default();
    let c: Arc<dyn Backend> = match site.scheme {
        Scheme::Sftp => Arc::new(sftp::Sftp::connect(site, &opts)?),
        Scheme::Ftp | Scheme::Ftps => Arc::new(ftp::Ftp::connect(site)?),
        Scheme::Dav | Scheme::Davs => Arc::new(dav::Dav::connect(site)?),
    };
    *s = Some(c.clone());
    Ok(c)
}

/// Run `f` on a site's connection, connecting again once if the connection had dropped.
fn with<T>(site: &Site, mut f: impl FnMut(&dyn Backend) -> io::Result<T>) -> io::Result<T> {
    let c = conn(site)?;
    match f(&*c) {
        Err(e) if !c.alive() => {
            let _ = e;
            let c = conn(site)?;
            f(&*c)
        }
        r => r,
    }
}

/// Connect (asking for what's needed) and return the folder the account starts in.
pub fn connect(path: &Path) -> Result<PathBuf, String> {
    let (site, p) = split(path).ok_or("That isn't a server address.")?;
    let c = conn(&site).map_err(|e| e.to_string())?;
    if p != "/" {
        return Ok(path.to_path_buf());
    }
    let home = c.home().unwrap_or_else(|_| "/".into());
    Ok(path_of(&site, &home))
}

/// Close the connection to a site (open windows on it connect again when used).
pub fn disconnect(path: &Path) {
    if let Some((site, _)) = split(path) {
        pool().lock().unwrap().remove(&site);
    }
}

/// The sites with a connection open now.
pub fn connected() -> Vec<String> {
    let p = pool().lock().unwrap();
    let mut out: Vec<String> = p
        .iter()
        .filter(|(_, slot)| slot.try_lock().ok().is_some_and(|s| s.as_ref().is_some_and(|c| c.alive())))
        .map(|(site, _)| site.url())
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------------------------
// File operations, by path.

fn site_path(path: &Path) -> io::Result<(Site, String)> {
    split(path).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a server path"))
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// What's at a path (an error of kind NotFound when nothing is).
pub fn meta(path: &Path) -> io::Result<Meta> {
    let (site, p) = site_path(path)?;
    with(&site, |c| c.stat(&p))
}

/// A folder's items, for the file list.
pub fn list(path: &Path) -> Result<Vec<Entry>, String> {
    let (site, p) = site_path(path).map_err(|e| e.to_string())?;
    let items = with(&site, |c| c.list(&p)).map_err(|e| describe(&e))?;
    Ok(items
        .into_iter()
        .filter(|(n, _)| n != "." && n != "..")
        .map(|(name, m)| Entry { hidden: name.starts_with('.'), name: name.into(), is_dir: m.is_dir, is_link: m.is_link, size: if m.is_dir { 0 } else { m.len }, modified: m.modified })
        .collect())
}

/// The paths of a folder's items.
pub fn read_dir(path: &Path) -> io::Result<Vec<PathBuf>> {
    let (site, p) = site_path(path)?;
    let items = with(&site, |c| c.list(&p))?;
    Ok(items.into_iter().filter(|(n, _)| n != "." && n != "..").map(|(n, _)| path_of(&site, &join(&p, &n))).collect())
}

pub fn create_dir(path: &Path) -> io::Result<()> {
    let (site, p) = site_path(path)?;
    with(&site, |c| c.mkdir(&p))
}

/// Make a folder and any missing above it.
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    match meta(path) {
        Ok(m) if m.is_dir => return Ok(()),
        Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, "a file is in the way")),
        Err(_) => {}
    }
    if let Some(up) = parent(path) {
        create_dir_all(&up)?;
    }
    create_dir(path)
}

/// Remove an empty folder.
pub fn remove_dir(path: &Path) -> io::Result<()> {
    let (site, p) = site_path(path)?;
    with(&site, |c| c.rmdir(&p))
}

/// Rename or move within one server.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let (a, pa) = site_path(from)?;
    let (b, pb) = site_path(to)?;
    if a != b {
        return Err(io::Error::new(io::ErrorKind::CrossesDevices, "different servers"));
    }
    with(&a, |c| c.rename(&pa, &pb))
}

/// Whether two paths are on the same server (so a move is a rename there).
pub fn same_site(a: &Path, b: &Path) -> bool {
    matches!((split(a), split(b)), (Some((x, _)), Some((y, _))) if x == y)
}

/// Delete for good, folder contents and all. Links are removed, not followed.
pub fn delete(path: &Path) -> io::Result<()> {
    let (site, p) = site_path(path)?;
    let c = conn(&site)?;
    fn walk(c: &dyn Backend, p: &str) -> io::Result<()> {
        let m = c.stat(p)?;
        if m.is_dir && !m.is_link {
            for (n, _) in c.list(p)? {
                if n != "." && n != ".." {
                    walk(c, &join(p, &n))?;
                }
            }
            c.rmdir(p)
        } else {
            c.rm(p)
        }
    }
    walk(&*c, &p)
}

/// Bytes in the files under a path.
pub fn bytes_in(path: &Path, ctl: &Ctl) -> u64 {
    let Ok((site, p)) = site_path(path) else { return 0 };
    let Ok(c) = conn(&site) else { return 0 };
    fn walk(c: &dyn Backend, p: &str, m: Meta, ctl: &Ctl) -> u64 {
        if !m.is_dir || m.is_link {
            return m.len;
        }
        let Ok(items) = c.list(p) else { return 0 };
        let mut sum = 0;
        for (n, m) in items {
            if ctl.stop.load(Ordering::Relaxed) {
                break;
            }
            if n != "." && n != ".." {
                sum += walk(c, &join(p, &n), m, ctl);
            }
        }
        sum
    }
    match c.stat(&p) {
        Ok(m) => walk(&*c, &p, m, ctl),
        Err(_) => 0,
    }
}

fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "stopped")
}

fn set_current(ctl: &Ctl, name: &str) {
    if let Ok(mut c) = ctl.current.lock() {
        *c = name.to_string();
    }
}

/// Copy a file or folder tree where either end (or both) is on a server. Archive items are
/// extracted to a temporary folder first.
pub fn copy_tree(src: &Path, dest: &Path, ctl: &Ctl) -> io::Result<()> {
    if !ctl.go_on() {
        return Err(stopped());
    }
    match (split(src), split(dest)) {
        (Some((ss, sp)), Some((ds, dp))) => {
            let m = with(&ss, |c| c.stat(&sp))?;
            if m.is_dir && !m.is_link {
                create_dir(dest)?;
                for (n, _) in with(&ss, |c| c.list(&sp))? {
                    if n != "." && n != ".." {
                        copy_tree(&path_of(&ss, &join(&sp, &n)), &path_of(&ds, &join(&dp, &n)), ctl)?;
                    }
                }
                Ok(())
            } else {
                // Server to server, through a temporary file here.
                let tmp = temp_file(src);
                let r = download_file(&ss, &sp, &tmp, m, ctl).and_then(|_| {
                    // The bytes are counted once.
                    ctl.bytes_done.fetch_sub(m.len.min(ctl.bytes_done.load(Ordering::Relaxed)), Ordering::Relaxed);
                    upload_file(&tmp, &ds, &dp, ctl)
                });
                let _ = fs::remove_file(&tmp);
                r
            }
        }
        (Some((ss, sp)), None) => {
            let m = with(&ss, |c| c.stat(&sp))?;
            if m.is_dir && !m.is_link {
                fs::create_dir(dest)?;
                for (n, _) in with(&ss, |c| c.list(&sp))? {
                    if n != "." && n != ".." {
                        copy_tree(&path_of(&ss, &join(&sp, &n)), &dest.join(&n), ctl)?;
                    }
                }
                Ok(())
            } else {
                download_file(&ss, &sp, dest, m, ctl)
            }
        }
        (None, Some((ds, dp))) => {
            if super::archive::inside(src) {
                // Out of an archive: extracted here first.
                let dir = temp_dir().join(format!("x{}", unique()));
                fs::create_dir_all(&dir)?;
                let name = src.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                let local = dir.join(&name);
                let r = super::archive::extract_path(src, &local, ctl).and_then(|_| copy_tree(&local, dest, ctl));
                let _ = fs::remove_dir_all(&dir);
                return r;
            }
            let md = fs::symlink_metadata(src)?;
            if md.is_dir() {
                create_dir(dest)?;
                for e in fs::read_dir(src)? {
                    let e = e?;
                    let n = e.file_name().to_string_lossy().to_string();
                    copy_tree(&e.path(), &path_of(&ds, &join(&dp, &n)), ctl)?;
                }
                Ok(())
            } else if md.file_type().is_symlink() {
                // Links can't be made on most servers: the file it points to is copied.
                upload_file(src, &ds, &dp, ctl)
            } else {
                upload_file(src, &ds, &dp, ctl)
            }
        }
        (None, None) => Err(io::Error::new(io::ErrorKind::InvalidInput, "neither path is on a server")),
    }
}

fn download_file(site: &Site, p: &str, dest: &Path, m: Meta, ctl: &Ctl) -> io::Result<()> {
    set_current(ctl, p.rsplit('/').next().unwrap_or(p));
    let mut f = fs::File::create(dest)?;
    let r = with(site, |c| {
        let mut tick = |n: u64| {
            ctl.bytes_done.fetch_add(n, Ordering::Relaxed);
            ctl.go_on()
        };
        c.get(p, &mut f, &mut tick)
    });
    if let Err(e) = r {
        drop(f);
        let _ = fs::remove_file(dest);
        return Err(e);
    }
    f.flush()?;
    if let Some(t) = m.modified_time() {
        let _ = f.set_modified(t);
    }
    Ok(())
}

fn upload_file(src: &Path, site: &Site, p: &str, ctl: &Ctl) -> io::Result<()> {
    set_current(ctl, &src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let md = fs::metadata(src)?;
    let r = with(site, |c| {
        let mut f = fs::File::open(src)?;
        let mut tick = |n: u64| {
            ctl.bytes_done.fetch_add(n, Ordering::Relaxed);
            ctl.go_on()
        };
        c.put(p, &mut f, md.len(), &mut tick)
    });
    if let Err(e) = r {
        let _ = with(site, |c| c.rm(p));
        return Err(e);
    }
    if let Ok(t) = md.modified()
        && let Ok(d) = t.duration_since(UNIX_EPOCH)
    {
        let _ = with(site, |c| c.touch(p, d.as_secs() as i64));
    }
    Ok(())
}

/// Write bytes to a new file on a server.
pub fn write_new(path: &Path, contents: &[u8]) -> io::Result<()> {
    let (site, p) = site_path(path)?;
    with(&site, |c| c.put(&p, &mut io::Cursor::new(contents), contents.len() as u64, &mut |_| true))
}

/// dirmap's folder for files fetched from servers to preview or open.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("dirmap-remote")
}

fn unique() -> u64 {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    t ^ (N.fetch_add(1, Ordering::Relaxed) << 48) ^ std::process::id() as u64
}

fn temp_file(src: &Path) -> PathBuf {
    let dir = temp_dir();
    let _ = fs::create_dir_all(&dir);
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    dir.join(format!("{}-{name}", unique()))
}

/// Fetch a server file into a temporary copy (to preview or open it), refusing ones over `max`
/// bytes. The copy keeps the file's name, in a folder of its own, so apps show the right name.
pub fn fetch(path: &Path, max: u64) -> io::Result<PathBuf> {
    let (site, p) = site_path(path)?;
    let m = with(&site, |c| c.stat(&p))?;
    if m.is_dir {
        return Err(io::Error::other("it's a folder"));
    }
    if m.len > max {
        return Err(io::Error::other(format!("it's {}, too big to fetch for this", crate::scan::human(m.len))));
    }
    // The same file, unchanged since it was fetched, isn't fetched again.
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        site.hash(&mut h);
        p.hash(&mut h);
        m.len.hash(&mut h);
        m.modified.hash(&mut h);
        h.finish()
    };
    let dir = temp_dir().join(format!("{key:016x}"));
    let name = p.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("file");
    let dest = dir.join(name);
    if fs::metadata(&dest).is_ok_and(|d| d.len() == m.len) {
        return Ok(dest);
    }
    fs::create_dir_all(&dir)?;
    let part = dir.join(format!(".{name}.part"));
    download_file(&site, &p, &part, m, &Ctl::default())?;
    fs::rename(&part, &dest)?;
    Ok(dest)
}

/// Read a whole server file into memory (frames for the sequence player), refusing ones over
/// `max` bytes.
pub fn read_bytes(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let (site, p) = site_path(path)?;
    let m = with(&site, |c| c.stat(&p))?;
    if m.len > max {
        return Err(io::Error::other(format!("it's {}, too big", crate::scan::human(m.len))));
    }
    with(&site, |c| {
        let mut out = Vec::with_capacity(m.len as usize);
        c.get(&p, &mut out, &mut |_| true)?;
        Ok(out)
    })
}

/// Every item under a folder on a server, for analysing it: (path relative to `path`, meta),
/// folders before what's in them. `seen` is told each folder's items as it's read.
pub fn walk(path: &Path, stop: &std::sync::atomic::AtomicBool, seen: &mut dyn FnMut(&str, &Listed)) -> io::Result<Listed> {
    let (site, p) = site_path(path)?;
    let c = conn(&site)?;
    let mut out = Vec::new();
    let mut todo = vec![String::new()];
    while let Some(rel) = todo.pop() {
        if stop.load(Ordering::Relaxed) {
            return Err(stopped());
        }
        let abs = if rel.is_empty() { p.clone() } else { join(&p, &rel) };
        let items = match c.list(&abs) {
            Ok(v) => v,
            Err(e) if rel.is_empty() => return Err(e),
            Err(_) => continue,
        };
        let items: Vec<(String, Meta)> = items.into_iter().filter(|(n, _)| n != "." && n != "..").collect();
        seen(&rel, &items);
        for (n, m) in items {
            let r = if rel.is_empty() { n } else { format!("{rel}/{n}") };
            if m.is_dir && !m.is_link {
                todo.push(r.clone());
            }
            out.push((r, m));
        }
    }
    Ok(out)
}

/// A plain message for an error from a server.
pub fn describe(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "It isn't there (the server says no such file or folder).".into(),
        io::ErrorKind::PermissionDenied => format!("The server refused: {e}"),
        _ => e.to_string(),
    }
}

/// Map a message to an io::Error, guessing its kind from the words servers use.
fn err(msg: impl std::fmt::Display) -> io::Error {
    let m = msg.to_string();
    let l = m.to_lowercase();
    let kind = if l.contains("no such file") || l.contains("not found") || l.contains("550") && l.contains("no such") {
        io::ErrorKind::NotFound
    } else if l.contains("permission denied") || l.contains("forbidden") {
        io::ErrorKind::PermissionDenied
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(kind, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip() {
        let p = parse("sftp://rich@Render01:2222/home/rich/shots").unwrap();
        let (site, rest) = split(&p).unwrap();
        assert_eq!(site, Site { scheme: Scheme::Sftp, user: "rich".into(), host: "render01".into(), port: 2222 });
        assert_eq!(rest, "/home/rich/shots");
        assert_eq!(show(&p).unwrap(), "sftp://rich@render01:2222/home/rich/shots");
        // Joining works, with either slash.
        let j = p.join("A001.exr");
        assert_eq!(split(&j).unwrap().1, "/home/rich/shots/A001.exr");
        assert_eq!(parent(&j).unwrap(), p);
        let top = parse("ftp://host").unwrap();
        assert!(is_root(&top));
        assert_eq!(parent(&top), None);
        assert_eq!(show(&top).unwrap(), "ftp://host/");
        assert_eq!(split(&parse("ftp://host/a%20b").unwrap()).unwrap().1, "/a b");
        assert_eq!(split(&parse("https://u@dav.example.com/remote.php/dav").unwrap()).unwrap().0.scheme, Scheme::Davs);
        assert!(parse("C:/Users").is_none());
        assert!(!is_remote(Path::new("/home/rich/sftp:/x")));
        assert!(split(&parse("sftp://[::1]:2200/x").unwrap()).is_some_and(|(s, _)| s.host == "[::1]" && s.port == 2200));
        assert!(!Path::new("sftp:/h/x").is_absolute());
    }
}

/// Against real servers: `DIRMAP_TEST_SERVERS="sftp://tester@127.0.0.1:2222/home/tester ftp://…"`
/// and `DIRMAP_TEST_PASSWORD`, then `cargo test live_servers -- --ignored`.
#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn live_servers() {
        let urls = std::env::var("DIRMAP_TEST_SERVERS").expect("DIRMAP_TEST_SERVERS");
        let pw = std::env::var("DIRMAP_TEST_PASSWORD").unwrap_or_default();
        set_asker(move |q| match q {
            Ask::NewHostKey { .. } => Answer::Trust { always: false },
            Ask::Password { retry: false, .. } => Answer::Secret { text: pw.clone(), remember: false },
            _ => Answer::Cancel,
        });
        let local = std::env::temp_dir().join(format!("dirmap-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&local);
        fs::create_dir_all(local.join("tree/sub")).unwrap();
        fs::write(local.join("tree/one.txt"), b"one").unwrap();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
        fs::write(local.join("tree/sub/big.bin"), &big).unwrap();
        for url in urls.split_whitespace() {
            eprintln!("== {url}");
            let start = parse(url).unwrap();
            let home = connect(&start).unwrap();
            let names: Vec<String> = list(&home).unwrap().into_iter().map(|e| e.name.to_string()).collect();
            eprintln!("listed {names:?}");
            let dir = home.join(format!("dirmap-test-{}", std::process::id()));
            create_dir(&dir).unwrap();
            assert!(meta(&dir).unwrap().is_dir);
            let ctl = Ctl::default();
            copy_tree(&local.join("tree"), &dir.join("tree"), &ctl).unwrap();
            assert_eq!(ctl.bytes_done.load(Ordering::Relaxed), 3_000_003);
            assert_eq!(bytes_in(&dir.join("tree"), &ctl), 3_000_003);
            let m = meta(&dir.join("tree/sub/big.bin")).unwrap();
            assert_eq!(m.len, 3_000_000);
            rename(&dir.join("tree/one.txt"), &dir.join("tree/two.txt")).unwrap();
            assert_eq!(meta(&dir.join("tree/one.txt")).unwrap_err().kind(), io::ErrorKind::NotFound);
            // Back down again, and server to server.
            let back = local.join(format!("back-{}", url.split(':').next().unwrap()));
            copy_tree(&dir.join("tree"), &back, &ctl).unwrap();
            assert_eq!(fs::read(back.join("sub/big.bin")).unwrap(), big);
            assert_eq!(fs::read(back.join("two.txt")).unwrap(), b"one");
            copy_tree(&dir.join("tree"), &dir.join("copy"), &ctl).unwrap();
            assert_eq!(meta(&dir.join("copy/sub/big.bin")).unwrap().len, 3_000_000);
            let f = fetch(&dir.join("copy/two.txt"), 1 << 20).unwrap();
            assert_eq!(fs::read(&f).unwrap(), b"one");
            assert_eq!(read_bytes(&dir.join("copy/sub/big.bin"), 4 << 20).unwrap(), big);
            assert!(read_bytes(&dir.join("copy/sub/big.bin"), 1 << 20).is_err());
            let mut seen = 0;
            let all = walk(&dir, &std::sync::atomic::AtomicBool::new(false), &mut |_, items| seen += items.len()).unwrap();
            assert_eq!(seen, 8);
            assert_eq!(all.len(), 8, "{all:?}");
            // Through the file manager's copy and move, with a name clash and Undo.
            let t = crate::transfer::transfer_with(&[local.join("tree")], &dir, true, &mut |_| crate::transfer::Choice::KeepBoth, &ctl);
            assert!(t.errors.is_empty(), "{:?}", t.errors);
            assert_eq!(t.made, vec![dir.join("tree 2")]);
            let m = crate::transfer::transfer_with(&[dir.join("tree 2")], &dir.join("copy"), false, &mut |_| crate::transfer::Choice::Stop, &ctl);
            assert!(m.errors.is_empty(), "{:?}", m.errors);
            assert!(meta(&dir.join("copy/tree 2/sub/big.bin")).is_ok());
            let (errs, _, _) = crate::transfer::undo(&m.steps);
            assert!(errs.is_empty(), "{errs:?}");
            assert!(meta(&dir.join("tree 2")).unwrap().is_dir);
            crate::transfer::transfer_with(&[local.join("tree/one.txt")], &dir.join("tree"), true, &mut |_| crate::transfer::Choice::Stop, &ctl);
            let r = crate::transfer::transfer_with(&[local.join("tree/one.txt")], &dir.join("tree"), true, &mut |_| crate::transfer::Choice::Replace, &ctl);
            assert!(r.errors.is_empty(), "{:?}", r.errors);
            assert!(r.steps.iter().any(|s| matches!(s, crate::transfer::Step::Deleted(_))), "{:?}", r.steps);
            let p = crate::actions::rename(&dir.join("tree/one.txt"), "renamed.txt").unwrap();
            assert_eq!(p, dir.join("tree/renamed.txt"));
            let nf = crate::actions::new_folder(&dir).unwrap();
            assert!(meta(&nf).unwrap().is_dir);
            write_new(&dir.join("new.txt"), b"made").unwrap();
            assert_eq!(meta(&dir.join("new.txt")).unwrap().len, 4);
            delete(&dir).unwrap();
            assert!(meta(&dir).is_err());
        }
        let _ = fs::remove_dir_all(&local);
    }
}

/// Signing in with key files: `DIRMAP_TEST_KEY_SERVER=sftp://tester@127.0.0.1:2222/`, with
/// `DIRMAP_TEST_KEY` (no passphrase) and `DIRMAP_TEST_KEY_LOCKED` (passphrase "phrase1").
#[cfg(test)]
mod live_keys {
    use super::*;

    #[test]
    #[ignore]
    fn live_key_sign_in() {
        let url = std::env::var("DIRMAP_TEST_KEY_SERVER").expect("DIRMAP_TEST_KEY_SERVER");
        let asked = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_asker(move |q| {
            a2.lock().unwrap().push(format!("{q:?}"));
            match q {
                Ask::NewHostKey { .. } => Answer::Trust { always: false },
                Ask::Passphrase { retry: false, .. } => Answer::Secret { text: "wrong".into(), remember: false },
                Ask::Passphrase { retry: true, .. } => Answer::Secret { text: "phrase1".into(), remember: false },
                _ => Answer::Cancel,
            }
        });
        for (var, port_user) in [("DIRMAP_TEST_KEY", "tester"), ("DIRMAP_TEST_KEY_LOCKED", "tester")] {
            let key = PathBuf::from(std::env::var(var).unwrap());
            set_options(&url, Options { key: Some(key) });
            disconnect(&parse(&url).unwrap());
            let home = connect(&parse(&url).unwrap()).unwrap_or_else(|e| panic!("{var}: {e} {:?}", asked.lock().unwrap()));
            assert!(show(&home).unwrap().ends_with(&format!("/home/{port_user}")));
        }
        let asked = asked.lock().unwrap();
        assert!(asked.iter().all(|a| !a.starts_with("Password")), "{asked:?}");
        assert_eq!(asked.iter().filter(|a| a.starts_with("Passphrase")).count(), 2, "{asked:?}");
    }
}
