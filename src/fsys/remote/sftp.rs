//! SFTP, over SSH (russh). The server's key is checked against `~/.ssh/known_hosts` and
//! dirmap's own list; a new server is shown to the user before anything is sent, and a server
//! whose key changed is refused. Signing in tries, in turn: the SSH agent, the key files in
//! `~/.ssh` (or the one set for the site), then a password (from the keychain, or asked for).

use super::{Answer, Ask, Backend, Meta, Options, Site, ask, err, keychain_forget, keychain_get, keychain_set, password_for};
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::MethodKind;
use russh_sftp::client::SftpSession;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The runtime the SSH connections run on (two threads; every call from dirmap blocks on it
/// from a worker thread).
fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("dirmap-ssh").enable_all().build().expect("tokio runtime"))
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const PIECE: usize = 256 << 10;

/// dirmap's own list of server keys the user accepted ("Always"), beside `~/.ssh/known_hosts`
/// (which is only read).
pub fn own_known_hosts() -> Option<PathBuf> {
    crate::templates::own_folder().and_then(|t| t.parent().map(|d| d.join("known_hosts")))
}

fn user_known_hosts() -> Option<PathBuf> {
    crate::fsys::places::home().map(|h| h.join(".ssh").join("known_hosts"))
}

struct Client {
    site: Site,
    /// Why the server was refused, to say so instead of a bare "disconnected".
    refused: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let key = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(c) => keys::PublicKey::new(c.public_key().clone(), ""),
        };
        let site = self.site.clone();
        let refused = self.refused.clone();
        // Asking waits on the user, so off the runtime's threads.
        tokio::task::spawn_blocking(move || check_key(&site, &key, &refused)).await.map_err(|_| russh::Error::Disconnect)
    }
}

fn check_key(site: &Site, key: &keys::PublicKey, refused: &Mutex<Option<String>>) -> bool {
    let host = site.host.trim_start_matches('[').trim_end_matches(']');
    let algorithm = key.algorithm().to_string();
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    for file in [user_known_hosts(), own_known_hosts()].into_iter().flatten() {
        if !file.exists() {
            continue;
        }
        match keys::check_known_hosts_path(host, site.port, key, &file) {
            Ok(true) => return true,
            Ok(false) => {}
            Err(keys::Error::KeyChanged { line }) => {
                let known_hosts = format!("{}, line {line}", file.display());
                ask(Ask::ChangedHostKey { site: site.url(), algorithm, fingerprint: fingerprint.clone(), known_hosts: known_hosts.clone() });
                *refused.lock().unwrap() = Some(format!(
                    "{}'s key has changed since you last connected ({known_hosts}). This can mean someone is intercepting the connection, so dirmap didn't connect. If the server was reinstalled, remove the old key from that file and try again.",
                    site.host
                ));
                return false;
            }
            Err(_) => {}
        }
    }
    match ask(Ask::NewHostKey { site: site.url(), algorithm, fingerprint }) {
        Answer::Trust { always } => {
            if always && let Some(own) = own_known_hosts() {
                if let Some(d) = own.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = keys::known_hosts::learn_known_hosts_path(host, site.port, key, &own);
            }
            true
        }
        _ => {
            *refused.lock().unwrap() = Some("Not connected: the server's identity wasn't accepted.".into());
            false
        }
    }
}

pub struct Sftp {
    handle: Arc<Handle<Client>>,
    sftp: SftpSession,
}

fn ssh_err(e: russh::Error) -> io::Error {
    match e {
        russh::Error::IO(e) => e,
        e => io::Error::other(e.to_string()),
    }
}

fn sftp_err(e: russh_sftp::client::error::Error) -> io::Error {
    use russh_sftp::client::error::Error as E;
    use russh_sftp::protocol::StatusCode as S;
    match &e {
        E::Status(s) if s.status_code == S::NoSuchFile => io::Error::new(io::ErrorKind::NotFound, s.error_message.clone()),
        E::Status(s) if s.status_code == S::PermissionDenied => io::Error::new(io::ErrorKind::PermissionDenied, s.error_message.clone()),
        E::Status(s) if !s.error_message.is_empty() => err(&s.error_message),
        _ => err(e),
    }
}

fn meta_of(m: &russh_sftp::client::fs::Metadata) -> Meta {
    Meta { is_dir: m.is_dir(), is_link: m.is_symlink(), len: m.size.unwrap_or(0), modified: m.mtime.map(i64::from).unwrap_or(0) }
}

/// The key files ssh would try, in its order.
fn default_keys() -> Vec<PathBuf> {
    let Some(h) = crate::fsys::places::home() else { return Vec::new() };
    ["id_ed25519", "id_ecdsa", "id_rsa"].iter().map(|n| h.join(".ssh").join(n)).filter(|p| p.is_file()).collect()
}

/// Load a key file, asking for its passphrase (kept in the keychain if wanted) when it has one.
fn load_key(path: &PathBuf) -> Option<keys::PrivateKey> {
    match keys::load_secret_key(path, None) {
        Ok(k) => return Some(k),
        Err(keys::Error::KeyIsEncrypted) => {}
        Err(_) => return None,
    }
    let account = format!("ssh-key:{}", path.display());
    if let Some(p) = keychain_get(&account) {
        if let Ok(k) = keys::load_secret_key(path, Some(&p)) {
            return Some(k);
        }
        keychain_forget(&account);
    }
    let mut retry = false;
    loop {
        match ask(Ask::Passphrase { key: path.display().to_string(), retry }) {
            Answer::Secret { text, remember } => {
                if let Ok(k) = keys::load_secret_key(path, Some(&text)) {
                    if remember {
                        let _ = keychain_set(&account, &text);
                    }
                    return Some(k);
                }
                retry = true;
            }
            _ => return None,
        }
    }
}

impl Sftp {
    pub fn connect(site: &Site, opts: &Options) -> io::Result<Sftp> {
        rt().block_on(Self::connect_async(site, opts))
    }

    async fn connect_async(site: &Site, opts: &Options) -> io::Result<Sftp> {
        let config = Arc::new(client::Config { inactivity_timeout: None, keepalive_interval: Some(Duration::from_secs(30)), keepalive_max: 4, nodelay: true, ..Default::default() });
        let refused = Arc::new(Mutex::new(None));
        let handler = Client { site: site.clone(), refused: refused.clone() };
        let host = site.host.trim_start_matches('[').trim_end_matches(']').to_string();
        let mut h = match tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, (host.as_str(), site.port), handler)).await {
            Err(_) => return Err(io::Error::new(io::ErrorKind::TimedOut, format!("{} didn't answer within {} seconds.", site.host, CONNECT_TIMEOUT.as_secs()))),
            Ok(Err(e)) => {
                if let Some(why) = refused.lock().unwrap().take() {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
                }
                return Err(match ssh_err(e) {
                    e if e.kind() == io::ErrorKind::ConnectionRefused => io::Error::new(e.kind(), format!("{} refused the connection on port {} (is SSH running there?).", site.host, site.port)),
                    e => io::Error::new(e.kind(), format!("Couldn't connect to {}: {e}", site.host)),
                });
            }
            Ok(Ok(h)) => h,
        };
        if !Self::sign_in(&mut h, site, opts).await? {
            let _ = h.disconnect(russh::Disconnect::ByApplication, "", "en").await;
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} didn't accept the sign-in for {}.", site.host, site.user)));
        }
        let ch = h.channel_open_session().await.map_err(ssh_err)?;
        ch.request_subsystem(true, "sftp").await.map_err(ssh_err)?;
        let sftp = SftpSession::new(ch.into_stream()).await.map_err(|e| io::Error::other(format!("{} has no SFTP service: {e}", site.host)))?;
        sftp.set_timeout(60);
        Ok(Sftp { handle: Arc::new(h), sftp })
    }

    /// Try each way of signing in. False when none worked (or the user cancelled).
    async fn sign_in(h: &mut Handle<Client>, site: &Site, opts: &Options) -> io::Result<bool> {
        let user = site.user.clone();
        let mut methods: Option<Vec<MethodKind>> = None;
        let note = |r: &russh::client::AuthResult, methods: &mut Option<Vec<MethodKind>>| {
            if let russh::client::AuthResult::Failure { remaining_methods, .. } = r {
                *methods = Some(remaining_methods.to_vec());
            }
        };
        let allowed = |methods: &Option<Vec<MethodKind>>, k: MethodKind| methods.as_ref().is_none_or(|m| m.contains(&k));

        // 1. The SSH agent.
        if allowed(&methods, MethodKind::PublicKey)
            && let Some(mut agent) = agent().await
            && let Ok(ids) = agent.request_identities().await
        {
            for id in ids {
                let key = id.public_key().into_owned();
                let hash = if key.algorithm().is_rsa() { h.best_supported_rsa_hash().await.ok().flatten().flatten() } else { None };
                match h.authenticate_publickey_with(&user, key, hash, &mut agent).await {
                    Ok(r) if r.success() => return Ok(true),
                    Ok(r) => note(&r, &mut methods),
                    Err(_) => {}
                }
            }
        }

        // 2. Key files.
        let files: Vec<PathBuf> = opts.key.iter().cloned().chain(if opts.key.is_some() { Vec::new() } else { default_keys() }).collect();
        for f in files {
            if !allowed(&methods, MethodKind::PublicKey) {
                break;
            }
            let path = f.clone();
            let Some(key) = tokio::task::spawn_blocking(move || load_key(&path)).await.ok().flatten() else { continue };
            let hash = if key.algorithm().is_rsa() { h.best_supported_rsa_hash().await.ok().flatten().flatten() } else { None };
            let r = h.authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await.map_err(ssh_err)?;
            if r.success() {
                return Ok(true);
            }
            note(&r, &mut methods);
        }

        // 3. A password (or keyboard-interactive, which many servers use for passwords).
        let mut tried: Option<String> = None;
        for _ in 0..3 {
            let use_password = allowed(&methods, MethodKind::Password);
            let use_kbd = allowed(&methods, MethodKind::KeyboardInteractive);
            if !use_password && !use_kbd {
                break;
            }
            let s = site.clone();
            let t = tried.clone();
            let Some(pw) = tokio::task::spawn_blocking(move || password_for(&s, t.as_deref())).await.ok().flatten() else { return Ok(false) };
            if use_password {
                let r = h.authenticate_password(&user, &pw).await.map_err(ssh_err)?;
                if r.success() {
                    return Ok(true);
                }
                note(&r, &mut methods);
            }
            if use_kbd && allowed(&methods, MethodKind::KeyboardInteractive) {
                let mut r = h.authenticate_keyboard_interactive_start(&user, None).await.map_err(ssh_err)?;
                for _ in 0..4 {
                    match r {
                        KeyboardInteractiveAuthResponse::Success => return Ok(true),
                        KeyboardInteractiveAuthResponse::Failure { remaining_methods, .. } => {
                            methods = Some(remaining_methods.to_vec());
                            break;
                        }
                        KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                            let answers = prompts.iter().map(|_| pw.clone()).collect();
                            r = h.authenticate_keyboard_interactive_respond(answers).await.map_err(ssh_err)?;
                        }
                    }
                }
            }
            tried = Some(pw);
        }
        Ok(false)
    }
}

#[cfg(unix)]
async fn agent() -> Option<keys::agent::client::AgentClient<Box<dyn keys::agent::client::AgentStream + Send + Unpin>>> {
    keys::agent::client::AgentClient::connect_env().await.ok().map(|a| a.dynamic())
}

#[cfg(windows)]
async fn agent() -> Option<keys::agent::client::AgentClient<Box<dyn keys::agent::client::AgentStream + Send + Unpin>>> {
    // Windows' own OpenSSH agent, else Pageant.
    if let Ok(a) = keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        return Some(a.dynamic());
    }
    keys::agent::client::AgentClient::connect_pageant().await.ok().map(|a| a.dynamic())
}

impl Backend for Sftp {
    fn stat(&self, p: &str) -> io::Result<Meta> {
        let m = rt().block_on(async { self.sftp.symlink_metadata(p).await.map(|m| meta_of(&m)).map_err(sftp_err) })?;
        // A link to a folder opens like a folder.
        if m.is_link && rt().block_on(self.sftp.metadata(p)).is_ok_and(|t| t.is_dir()) {
            return Ok(Meta { is_dir: true, ..m });
        }
        Ok(m)
    }

    fn list(&self, p: &str) -> io::Result<Vec<(String, Meta)>> {
        rt().block_on(async {
            let rd = self.sftp.read_dir(p).await.map_err(sftp_err)?;
            let mut out = Vec::new();
            let mut links = Vec::new();
            for e in rd {
                let m = meta_of(&e.metadata());
                if m.is_link {
                    links.push(out.len());
                }
                out.push((e.file_name(), m));
            }
            // Links to folders open like folders.
            for i in links {
                let full = super::join(p, &out[i].0);
                if self.sftp.metadata(full).await.is_ok_and(|m| m.is_dir()) {
                    out[i].1.is_dir = true;
                }
            }
            Ok(out)
        })
    }

    fn mkdir(&self, p: &str) -> io::Result<()> {
        rt().block_on(self.sftp.create_dir(p)).map_err(sftp_err)
    }

    fn rmdir(&self, p: &str) -> io::Result<()> {
        rt().block_on(self.sftp.remove_dir(p)).map_err(sftp_err)
    }

    fn rm(&self, p: &str) -> io::Result<()> {
        rt().block_on(self.sftp.remove_file(p)).map_err(sftp_err)
    }

    fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        rt().block_on(self.sftp.rename(from, to)).map_err(sftp_err)
    }

    fn get(&self, p: &str, out: &mut dyn Write, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        rt().block_on(async {
            let mut f = self.sftp.open(p).await.map_err(sftp_err)?;
            let mut buf = vec![0u8; PIECE];
            loop {
                let n = f.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])?;
                if !tick(n as u64) {
                    return Err(super::stopped());
                }
            }
            Ok(())
        })
    }

    fn put(&self, p: &str, input: &mut dyn Read, _len: u64, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        rt().block_on(async {
            let mut f = self.sftp.create(p).await.map_err(sftp_err)?;
            let mut buf = vec![0u8; PIECE];
            loop {
                let n = input.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n]).await?;
                if !tick(n as u64) {
                    return Err(super::stopped());
                }
            }
            f.shutdown().await?;
            Ok(())
        })
    }

    fn touch(&self, p: &str, modified: i64) -> io::Result<()> {
        let mut m = russh_sftp::protocol::FileAttributes::empty();
        m.mtime = Some(modified as u32);
        m.atime = Some(modified as u32);
        rt().block_on(self.sftp.set_metadata(p, m)).map_err(sftp_err)
    }

    fn home(&self) -> io::Result<String> {
        rt().block_on(self.sftp.canonicalize(".")).map_err(sftp_err)
    }

    fn alive(&self) -> bool {
        !self.handle.is_closed()
    }
}

impl Drop for Sftp {
    fn drop(&mut self) {
        // Close politely, without waiting on a server that's gone.
        let h = self.handle.clone();
        rt().spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(2), h.disconnect(russh::Disconnect::ByApplication, "", "en")).await;
        });
    }
}
