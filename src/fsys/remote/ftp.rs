//! FTP, and FTPS (explicit TLS, checked against the system's trusted certificates). Plain FTP
//! sends the password and files unencrypted; the connection manager says so.
//!
//! An FTP connection does one thing at a time, so a few are kept: a big download doesn't stop
//! the folder list from being read.

use super::{Backend, Meta, Scheme, Site, err, password_for};
use std::io::{self, Read, Write};
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use suppaftp::list::{File, ListParser};
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpStream, RustlsConnector, RustlsFtpStream, Status};

const TIMEOUT: Duration = Duration::from_secs(20);

enum Conn {
    Plain(FtpStream),
    Tls(RustlsFtpStream),
}

/// The same code for either kind of connection.
macro_rules! on {
    ($c:expr, $s:ident => $e:expr) => {
        match $c {
            Conn::Plain($s) => $e,
            Conn::Tls($s) => $e,
        }
    };
}

pub struct Ftp {
    site: Site,
    /// Kept in memory only, to open more connections to the same server.
    password: String,
    idle: Mutex<Vec<Conn>>,
    home: String,
    /// The server lists folders with MLSD (exact sizes and times); else LIST is read.
    mlsd: AtomicBool,
    dead: AtomicBool,
}

fn ftp_err(e: FtpError) -> io::Error {
    match e {
        FtpError::ConnectionError(e) => e,
        FtpError::UnexpectedResponse(r) => {
            let text = String::from_utf8_lossy(&r.body).trim().to_string();
            if r.status == Status::FileUnavailable && text.to_lowercase().contains("permission") {
                io::Error::new(io::ErrorKind::PermissionDenied, text)
            } else if r.status == Status::FileUnavailable {
                io::Error::new(io::ErrorKind::NotFound, text)
            } else {
                err(text)
            }
        }
        e => err(e),
    }
}

fn tls_config() -> io::Result<Arc<rustls::ClientConfig>> {
    use rustls_platform_verifier::ConfigVerifierExt;
    rustls::ClientConfig::with_platform_verifier().map(Arc::new).map_err(|e| io::Error::other(format!("Couldn't set up TLS: {e}")))
}

fn untrusted(site: &Site, e: &io::Error) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("{}'s certificate isn't one your system trusts ({e}). If it's your own server, add its certificate to the system's trusted certificates.", site.host))
}

fn open(site: &Site, password: &str) -> io::Result<Conn> {
    let host = site.host.trim_start_matches('[').trim_end_matches(']');
    let addr = (host, site.port).to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Couldn't find {}.", site.host)))?;
    let user = if site.user.is_empty() { "anonymous" } else { site.user.as_str() };
    let password = if site.user.is_empty() { "anonymous@" } else { password };
    let mut c = match site.scheme {
        // Port 990 is implicit FTPS: TLS from the first byte (older servers).
        Scheme::Ftps if site.port == 990 => {
            #[allow(deprecated)]
            let s = RustlsFtpStream::connect_secure_implicit(addr, RustlsConnector::from(tls_config()?), host).map_err(|e| {
                let e = ftp_err(e);
                if e.to_string().contains("certificate") { untrusted(site, &e) } else { e }
            })?;
            Conn::Tls(s)
        }
        Scheme::Ftps => {
            let s = RustlsFtpStream::connect_timeout(addr, TIMEOUT).map_err(ftp_err)?;
            let s = s.into_secure(RustlsConnector::from(tls_config()?), host).map_err(|e| {
                let e = ftp_err(e);
                if e.to_string().contains("certificate") { untrusted(site, &e) } else { e }
            })?;
            Conn::Tls(s)
        }
        _ => Conn::Plain(FtpStream::connect_timeout(addr, TIMEOUT).map_err(ftp_err)?),
    };
    let signed_in: io::Result<()> = on!(&mut c, s => {
        s.set_passive_nat_workaround(true);
        s.login(user, password).map_err(|e| match e {
            FtpError::UnexpectedResponse(r) if r.status == Status::NotLoggedIn => io::Error::new(io::ErrorKind::PermissionDenied, "login"),
            e => ftp_err(e),
        })?;
        s.transfer_type(FileType::Binary).map_err(ftp_err)
    });
    // The secure connection is checked on the first command, so its refusal shows up here.
    signed_in.map_err(|e| {
        if e.to_string().contains("certificate") { untrusted(site, &e) } else { e }
    })?;
    Ok(c)
}

impl Ftp {
    pub fn connect(site: &Site) -> io::Result<Ftp> {
        let mut tried: Option<String> = None;
        loop {
            let password = if site.user.is_empty() {
                String::new()
            } else {
                match password_for(site, tried.as_deref()) {
                    Some(p) => p,
                    None => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Not connected: no password was given.")),
                }
            };
            match open(site, &password) {
                Ok(mut c) => {
                    let home = on!(&mut c, s => s.pwd()).unwrap_or_else(|_| "/".into());
                    return Ok(Ftp { site: site.clone(), password, idle: Mutex::new(vec![c]), home, mlsd: AtomicBool::new(true), dead: AtomicBool::new(false) });
                }
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied && e.to_string() == "login" && !site.user.is_empty() => tried = Some(password),
                Err(e) if e.to_string() == "login" => return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} didn't accept the sign-in.", site.host))),
                Err(e) => return Err(e),
            }
        }
    }

    /// Run `f` on a free connection (opening another if all are busy). A connection that fails
    /// is closed, not reused.
    fn op<R>(&self, f: impl FnOnce(&mut Conn) -> io::Result<R>) -> io::Result<R> {
        let taken = self.idle.lock().unwrap().pop();
        let mut c = match taken {
            Some(c) => c,
            None => open(&self.site, &self.password).inspect_err(|_| self.dead.store(true, Ordering::Relaxed))?,
        };
        let r = f(&mut c);
        let keep = match &r {
            Ok(_) => true,
            // The server answered (no such file, refused): the connection is fine.
            Err(e) => matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied | io::ErrorKind::AlreadyExists),
        };
        if keep {
            let mut idle = self.idle.lock().unwrap();
            if idle.len() < 3 {
                idle.push(c);
            }
        }
        r
    }

    fn list_files(&self, p: &str) -> io::Result<Vec<File>> {
        if self.mlsd.load(Ordering::Relaxed) {
            match self.op(|c| on!(c, s => s.mlsd(Some(p))).map_err(ftp_err)) {
                Ok(lines) => return Ok(lines.iter().filter_map(|l| ListParser::parse_mlsd(l).ok()).collect()),
                Err(e) if e.kind() == io::ErrorKind::NotFound || e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
                Err(_) => self.mlsd.store(false, Ordering::Relaxed),
            }
        }
        let lines = self.op(|c| on!(c, s => s.list(Some(p))).map_err(ftp_err))?;
        Ok(lines.iter().filter_map(|l| ListParser::parse_posix(l).or_else(|_| ListParser::parse_dos(l)).ok()).collect())
    }
}

fn meta_of(f: &File) -> Meta {
    let modified = f.modified().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    use suppaftp::list::PosixPexQuery as Q;
    // LIST gives the rwx columns; MLSD rarely does, and then every bit reads as unset.
    let bits = |q: Q, shift: u32| ((f.can_read(q) as u32) << 2 | (f.can_write(q) as u32) << 1 | f.can_execute(q) as u32) << shift;
    let mode = bits(Q::Owner, 6) | bits(Q::Group, 3) | bits(Q::Others, 0);
    Meta { is_dir: f.is_directory(), is_link: f.is_symlink(), len: f.size() as u64, modified, mode: (mode != 0).then_some(mode), uid: f.uid(), gid: f.gid() }
}

fn name_of(f: &File) -> String {
    // Some servers list full paths.
    f.name().rsplit('/').next().unwrap_or(f.name()).to_string()
}

impl Backend for Ftp {
    fn stat(&self, p: &str) -> io::Result<Meta> {
        if p == "/" {
            return Ok(Meta { is_dir: true, ..Default::default() });
        }
        if let Ok(line) = self.op(|c| on!(c, s => s.mlst(Some(p))).map_err(ftp_err))
            && let Ok(f) = ListParser::parse_mlst(line.trim())
        {
            return Ok(meta_of(&f));
        }
        let (dir, name) = p.rsplit_once('/').unwrap_or(("", p));
        let dir = if dir.is_empty() { "/" } else { dir };
        if let Some(m) = self.list_files(dir)?.iter().find(|f| name_of(f) == name).map(meta_of) {
            return Ok(m);
        }
        // Many servers leave dot files out of listings (dirmap's own part files among them);
        // SIZE and MDTM still answer for a file.
        match self.op(|c| on!(c, s => s.size(p)).map_err(ftp_err)) {
            Ok(len) => {
                let modified = self.op(|c| on!(c, s => s.mdtm(p)).map_err(ftp_err)).map(|t| unix_secs(t.year() as i64, t.month() as i64, t.day() as i64, t.hour() as i64 * 3600 + t.minute() as i64 * 60 + t.second() as i64)).unwrap_or(0);
                Ok(Meta { len: len as u64, modified, ..Default::default() })
            }
            Err(_) => Err(io::Error::new(io::ErrorKind::NotFound, "no such file")),
        }
    }

    fn list(&self, p: &str) -> io::Result<Vec<(String, Meta)>> {
        Ok(self.list_files(p)?.iter().map(|f| (name_of(f), meta_of(f))).collect())
    }

    fn mkdir(&self, p: &str) -> io::Result<()> {
        self.op(|c| on!(c, s => s.mkdir(p)).map_err(ftp_err))
    }

    fn rmdir(&self, p: &str) -> io::Result<()> {
        self.op(|c| on!(c, s => s.rmdir(p)).map_err(ftp_err))
    }

    fn rm(&self, p: &str) -> io::Result<()> {
        self.op(|c| on!(c, s => s.rm(p)).map_err(ftp_err))
    }

    fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        self.op(|c| on!(c, s => s.rename(from, to)).map_err(ftp_err))
    }

    fn chmod(&self, p: &str, mode: u32) -> io::Result<()> {
        self.op(|c| on!(c, s => s.site(format!("CHMOD {:o} {p}", mode & 0o7777))).map_err(ftp_err)).and_then(|r| {
            if r.status.code() / 100 == 2 { Ok(()) } else { Err(err(format!("The server refused to change permissions: {}", String::from_utf8_lossy(&r.body).trim()))) }
        })
    }

    fn readlink(&self, p: &str) -> io::Result<String> {
        let (dir, name) = p.rsplit_once('/').unwrap_or(("", p));
        let dir = if dir.is_empty() { "/" } else { dir };
        self.list_files(dir)?
            .iter()
            .find(|f| name_of(f) == name)
            .and_then(|f| f.symlink().map(|t| t.to_string_lossy().to_string()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "This server doesn't say where the link points."))
    }

    fn get(&self, p: &str, out: &mut dyn Write, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        self.get_from(p, 0, out, tick)
    }

    fn get_from(&self, p: &str, offset: u64, out: &mut dyn Write, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        let stopped = std::cell::Cell::new(false);
        let r = self.op(|c| {
            if offset > 0 {
                // REST: servers that can't are told apart by the error, and the copy starts over.
                on!(c, s => s.resume_transfer(offset as usize)).map_err(|e| io::Error::new(io::ErrorKind::Unsupported, ftp_err(e).to_string()))?;
            }
            on!(c, s => s.retr(p, |r: &mut dyn Read| {
                let mut buf = vec![0u8; 256 << 10];
                loop {
                    let n = r.read(&mut buf).map_err(FtpError::ConnectionError)?;
                    if n == 0 {
                        return Ok(());
                    }
                    out.write_all(&buf[..n]).map_err(FtpError::ConnectionError)?;
                    if !tick(n as u64) {
                        stopped.set(true);
                        return Err(FtpError::ConnectionError(super::stopped()));
                    }
                }
            }))
            .map_err(ftp_err)
        });
        if stopped.get() { Err(super::stopped()) } else { r }
    }

    fn put(&self, p: &str, input: &mut dyn Read, len: u64, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        self.put_at(p, 0, input, len, tick)
    }

    /// From `offset` on, APPE adds to the end of the part already there (the caller checked
    /// that it's `offset` long).
    fn put_at(&self, p: &str, offset: u64, input: &mut dyn Read, _len: u64, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        self.op(|c| {
            on!(c, s => {
                let mut w = if offset > 0 { s.append_with_stream(p) } else { s.put_with_stream(p) }.map_err(ftp_err)?;
                let mut buf = vec![0u8; 256 << 10];
                loop {
                    let n = input.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    w.write_all(&buf[..n])?;
                    if !tick(n as u64) {
                        // The connection is dropped with the half-sent file.
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
                    }
                }
                w.finish().map_err(ftp_err)
            })
        })
    }

    fn touch(&self, p: &str, modified: i64) -> io::Result<()> {
        let Some(t) = chrono::DateTime::from_timestamp(modified, 0) else { return Ok(()) };
        let stamp = t.format("%Y%m%d%H%M%S");
        self.op(|c| on!(c, s => s.custom_command(format!("MFMT {stamp} {p}"), &[Status::File])).map(drop).map_err(ftp_err))
    }

    fn home(&self) -> io::Result<String> {
        Ok(self.home.clone())
    }

    fn alive(&self) -> bool {
        !self.dead.load(Ordering::Relaxed)
    }
}

/// Seconds since the Unix epoch for a UTC date and time of day (MDTM's answer).
fn unix_secs(y: i64, m: i64, d: i64, secs: i64) -> i64 {
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe - 719468) * 86400 + secs
}
