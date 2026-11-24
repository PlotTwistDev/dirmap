//! Editing files that live on a server: a server file opened in an app is fetched into a
//! folder of its own, and every time the app saves it, the copy is sent back (through the same
//! part-file upload as the transfer queue, so the server never holds half a file). If the
//! server's copy changed since it was fetched, the user is asked first.
//!
//! One background thread checks the copies once a second. A save is sent once the file has
//! stayed the same for a second, so an app that writes in several steps is sent once.

use super::*;
use std::sync::OnceLock;

/// A server file being edited here.
#[derive(Clone, Debug)]
pub struct Editing {
    pub remote: PathBuf,
    pub local: PathBuf,
    /// Times a save was sent.
    pub uploads: u32,
    /// The last problem sending it, if the last try failed.
    pub error: Option<String>,
    /// Sending now.
    pub busy: bool,
}

struct Entry {
    info: Editing,
    /// The server's (size, modified) when last fetched or sent: changes to it mean someone
    /// else changed the file.
    server: (u64, i64),
    /// The copy's (size, modified) when last fetched or sent.
    seen: (u64, Option<SystemTime>),
    /// A change seen on the last check, sent once it's still the same on the next.
    pending: Option<(u64, Option<SystemTime>)>,
}

fn edits() -> &'static Mutex<Vec<Entry>> {
    static E: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(Vec::new()))
}

fn events() -> &'static Mutex<Vec<(String, bool)>> {
    static E: OnceLock<Mutex<Vec<(String, bool)>>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(Vec::new()))
}

/// The connection lane used for sending saves, apart from browsing and the queue's.
const LANE_EDIT: u8 = 200;

fn local_state(p: &Path) -> Option<(u64, Option<SystemTime>)> {
    fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()))
}

/// Fetch a server file to edit and start sending its saves back. Returns the local copy to
/// open. A file already being edited isn't fetched again (the copy may hold saved work).
pub fn open_copy(path: &Path) -> io::Result<PathBuf> {
    if let Some(e) = edits().lock().unwrap().iter().find(|e| e.info.remote == path) {
        if e.info.local.exists() {
            return Ok(e.info.local.clone());
        }
    }
    let (site, p) = site_path(path)?;
    let m = with(&site, |c| c.stat(&p))?;
    if m.is_dir {
        return Err(io::Error::other("it's a folder"));
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        site.hash(&mut h);
        p.hash(&mut h);
        h.finish()
    };
    let dir = temp_dir().join("edit").join(format!("{key:016x}"));
    fs::create_dir_all(&dir)?;
    let name = local_name(p.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("file"))?.to_string();
    let local = dir.join(&name);
    download_file(&site, &p, &local, m, &Ctl::default())?;
    let seen = local_state(&local).unwrap_or((0, None));
    let mut list = edits().lock().unwrap();
    list.retain(|e| e.info.remote != path);
    list.push(Entry {
        info: Editing { remote: path.to_path_buf(), local: local.clone(), uploads: 0, error: None, busy: false },
        server: (m.len, m.modified),
        seen,
        pending: None,
    });
    drop(list);
    start_watching();
    Ok(local)
}

/// Files being edited now.
pub fn editing() -> Vec<Editing> {
    edits().lock().unwrap().iter().map(|e| e.info.clone()).collect()
}

/// Stop sending a file's saves (the local copy stays until dirmap's temporary folder is
/// cleared).
pub fn stop(remote: &Path) {
    edits().lock().unwrap().retain(|e| e.info.remote != remote);
}

/// What happened since last asked: (message, is an error).
pub fn take_events() -> Vec<(String, bool)> {
    std::mem::take(&mut *events().lock().unwrap())
}

fn start_watching() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::spawn(|| {
            set_lane(LANE_EDIT);
            loop {
                std::thread::sleep(Duration::from_secs(1));
                check();
            }
        });
    });
}

/// One pass: note changed copies, and send those that stayed the same since the last pass.
fn check() {
    let due: Vec<PathBuf> = {
        let mut list = edits().lock().unwrap();
        let mut due = Vec::new();
        for e in list.iter_mut() {
            let Some(now) = local_state(&e.info.local) else { continue };
            if now == e.seen {
                e.pending = None;
            } else if e.pending == Some(now) {
                due.push(e.info.remote.clone());
            } else {
                e.pending = Some(now);
            }
        }
        due
    };
    for remote in due {
        send(&remote);
    }
}

/// Send one copy back, asking first if the server's copy changed meanwhile.
fn send(remote: &Path) {
    let Some((local, server, now)) = edits().lock().unwrap().iter_mut().find(|e| e.info.remote == remote).map(|e| {
        e.info.busy = true;
        (e.info.local.clone(), e.server, e.pending.unwrap_or(e.seen))
    }) else {
        return;
    };
    let name = local.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let r = (|| -> io::Result<Option<(u64, i64)>> {
        let (site, p) = site_path(remote)?;
        let there = with(&site, |c| c.stat(&p)).ok();
        if let Some(t) = there
            && (t.len, t.modified) != server
        {
            match ask(Ask::Overwrite { site: site.short(), name: name.clone() }) {
                Answer::Trust { .. } => {}
                _ => return Ok(None),
            }
        }
        upload_file(&local, &site, &p, &Ctl::default())?;
        let t = with(&site, |c| c.stat(&p))?;
        note(&site, false, format!("Sent the saved {name} back to {p}"));
        Ok(Some((t.len, t.modified)))
    })();
    let site_name = split(remote).map(|(s, _)| s.short()).unwrap_or_default();
    let mut list = edits().lock().unwrap();
    let Some(e) = list.iter_mut().find(|e| e.info.remote == remote) else { return };
    e.info.busy = false;
    e.pending = None;
    // Not sent again until the next save, whatever happened.
    e.seen = now;
    let msg = match r {
        Ok(Some(server)) => {
            e.server = server;
            e.info.uploads += 1;
            e.info.error = None;
            (format!("Saved {name} to {site_name}."), false)
        }
        Ok(None) => {
            e.info.error = Some("Not sent: the server's copy had changed".into());
            (format!("{name} wasn't sent to {site_name}: its copy there had changed. Save again to be asked again."), true)
        }
        Err(err) => {
            let d = describe(&err);
            e.info.error = Some(d.clone());
            (format!("Couldn't send {name} to {site_name}: {d}. Save again to try again."), true)
        }
    };
    events().lock().unwrap().push(msg);
}
